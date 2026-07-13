use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    error::Error,
    thread,
    time::{Duration, Instant},
};

use imgui::{Condition, Context, Ui};
use sdl3::{
    gpu::{Device, ShaderFormat, SwapchainComposition},
    keyboard::{Keycode, Scancode},
    mouse::MouseButton,
    video::Window,
};
use tungsten::{
    camera::{Camera, camera_screen_ray, terrain_full_map_distance},
    config::{AppConfig, CONFIG_PATH},
    imgui_sdl_gpu::ImguiSdlGpuRenderer,
    input::{FrameInput, InputState},
    player::PlayerController,
    props::{
        PropInstanceToml, PropScene, PropTileToml, prop_tile_coords_for_source,
        source_position_from_world,
    },
    renderer::{DebugVisualMode, OverlayStats, Renderer},
    terrain::{self, TerrainHit, TerrainMaps, raycast_terrain},
};

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

struct FpsCounter {
    accumulated: Duration,
    frame_count: u32,
    displayed_fps: f32,
    displayed_frame_ms: f32,
}

impl FpsCounter {
    fn new() -> Self {
        Self {
            accumulated: Duration::ZERO,
            frame_count: 0,
            displayed_fps: 0.0,
            displayed_frame_ms: 0.0,
        }
    }

    fn update(&mut self, frame_duration: Duration) {
        self.accumulated += frame_duration;
        self.frame_count += 1;

        if self.accumulated >= Duration::from_millis(250) {
            self.displayed_fps = self.frame_count as f32 / self.accumulated.as_secs_f32();
            self.displayed_frame_ms =
                self.accumulated.as_secs_f32() * 1000.0 / self.frame_count as f32;
            self.accumulated = Duration::ZERO;
            self.frame_count = 0;
        }
    }

    fn overlay_stats(&self) -> OverlayStats {
        OverlayStats {
            fps: self.displayed_fps,
            frame_ms: self.displayed_frame_ms,
        }
    }
}

struct EditorState {
    prop_ids: Vec<String>,
    selected_prop: Option<String>,
    picker_open: bool,
    mouse_captured: bool,
    edited_tiles: BTreeMap<[u32; 2], PropTileToml>,
    dirty_tiles: BTreeSet<[u32; 2]>,
    pending_quit: bool,
    save_and_quit_requested: bool,
    quit_confirmed: bool,
    save_status: Option<String>,
    last_hit: Option<TerrainHit>,
}

impl EditorState {
    fn new(prop_ids: Vec<String>) -> Self {
        Self {
            selected_prop: prop_ids.first().cloned(),
            prop_ids,
            picker_open: false,
            mouse_captured: true,
            edited_tiles: BTreeMap::new(),
            dirty_tiles: BTreeSet::new(),
            pending_quit: false,
            save_and_quit_requested: false,
            quit_confirmed: false,
            save_status: None,
            last_hit: None,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let config = AppConfig::load(CONFIG_PATH)?;
    let sdl = sdl3::init()?;
    let video = sdl.video()?;

    let window = video
        .window("tungsten editor", WINDOW_WIDTH, WINDOW_HEIGHT)
        .position_centered()
        .resizable()
        .build()?;

    let gpu = Device::new(ShaderFormat::SPIRV, cfg!(debug_assertions))?.with_window(&window)?;
    gpu.set_swapchain_parameters(
        &window,
        config.present_mode.to_sdl(),
        SwapchainComposition::Sdr,
    )
    .map_err(|error| {
        format!(
            "failed to set `{}` present mode: {error}",
            config.present_mode.as_config_value()
        )
    })?;
    let target_format = gpu.get_swapchain_texture_format(&window);
    let mut terrain_maps = terrain::load_terrain_maps(&gpu, &config)?;
    let mut prop_scene = PropScene::load(&gpu, &terrain_maps)?;
    let mut renderer = Renderer::new(&gpu, target_format)?;
    let mut imgui = Context::create();
    imgui.set_ini_filename(None);
    let mut imgui_renderer = ImguiSdlGpuRenderer::new(&gpu, target_format, &mut imgui)?;

    let mouse = sdl.mouse();
    set_mouse_capture(&mouse, &window, true);

    let mut camera = Camera {
        x: config.start_x,
        y: config.start_y,
        height: config.start_height,
        yaw: 0.4,
        pitch: -0.08,
        vertical_fov: 1.05,
        max_distance: terrain_full_map_distance(terrain_maps.terrain_size),
    };
    let mut player = PlayerController::new();
    let mut editor = EditorState::new(prop_scene.catalog().sorted_ids());
    let mut fps_counter = FpsCounter::new();

    let mut events = sdl.event_pump()?;
    let mut input_state = InputState::new();
    let mut previous_frame = Instant::now();
    let mut prop_refresh_requested = true;

    'running: loop {
        let input = input_state.poll(&mut events, true);

        if input.quit_requested() {
            if editor.dirty_tiles.is_empty() {
                break 'running;
            }
            editor.pending_quit = true;
            editor.mouse_captured = false;
            set_mouse_capture(&mouse, &window, false);
        }
        if input.key_pressed(Keycode::Tab) {
            editor.mouse_captured = !editor.mouse_captured;
            set_mouse_capture(&mouse, &window, editor.mouse_captured);
        }
        if input.key_pressed(Keycode::M) {
            editor.picker_open = true;
            editor.mouse_captured = false;
            set_mouse_capture(&mouse, &window, false);
        }
        if ctrl_held(&input) && input.key_pressed(Keycode::S) {
            save_dirty_tiles(&terrain_maps, &mut editor)?;
        }

        let now = Instant::now();
        let frame_duration = now - previous_frame;
        let dt = frame_duration.min(Duration::from_millis(50));
        previous_frame = now;

        update_imgui_io(&mut imgui, &input, &window, dt);
        if editor.mouse_captured {
            player.tick(
                &input,
                &mut camera,
                terrain_maps.collision_height(),
                dt.as_secs_f32(),
            );
        }

        let previous_window_min = terrain_maps.current_window_min;
        let previous_window_max = terrain_maps.current_window_max;
        terrain_maps.update_tile_cache_for_position(&gpu, camera.x, camera.y)?;
        if terrain_maps.current_window_min != previous_window_min
            || terrain_maps.current_window_max != previous_window_max
        {
            prop_refresh_requested = true;
        }

        let ui = imgui.frame();
        draw_editor_ui(ui, &mut editor, &camera);
        let imgui_wants_mouse = ui.io().want_capture_mouse;

        if should_place_prop(&input, &editor, imgui_wants_mouse) {
            if place_prop(&input, &mut editor, &terrain_maps, &camera, &window)? {
                prop_refresh_requested = true;
            }
        }

        if editor.save_and_quit_requested {
            save_dirty_tiles(&terrain_maps, &mut editor)?;
            editor.quit_confirmed = true;
        }
        if editor.quit_confirmed {
            break 'running;
        }

        if prop_refresh_requested {
            prop_scene.refresh_for_editor_tiles(&gpu, &terrain_maps, &editor.edited_tiles)?;
            prop_refresh_requested = false;
        } else {
            prop_scene.update_model_loads(&gpu);
        }

        let draw_data = imgui.render();
        let frame_submitted = renderer.render_frame_with_overlay(
            &gpu,
            &window,
            &terrain_maps,
            &prop_scene,
            &camera,
            &config,
            DebugVisualMode::None,
            fps_counter.overlay_stats(),
            |gpu, command_buffer, swapchain| {
                imgui_renderer.render(gpu, command_buffer, swapchain, draw_data)
            },
        )?;

        fps_counter.update(frame_duration);
        limit_framerate(now, config.max_framerate);
        if !frame_submitted {
            thread::sleep(Duration::from_millis(1));
        }
    }

    Ok(())
}

fn draw_editor_ui(ui: &Ui, editor: &mut EditorState, camera: &Camera) {
    ui.window("Editor")
        .position([12.0, 12.0], Condition::FirstUseEver)
        .size([330.0, 150.0], Condition::FirstUseEver)
        .build(|| {
            ui.text(format!(
                "camera {:.1}, {:.1}, {:.1}",
                camera.x, camera.y, camera.height
            ));
            ui.text(format!(
                "mouse {}",
                if editor.mouse_captured {
                    "captured"
                } else {
                    "free"
                }
            ));
            ui.text(format!(
                "selected {}",
                editor.selected_prop.as_deref().unwrap_or("<none>")
            ));
            ui.text(format!("dirty tiles {}", editor.dirty_tiles.len()));
            if let Some(hit) = editor.last_hit {
                ui.text(format!(
                    "last hit {:.1}, {:.1}, height {:.1}",
                    hit.world_x, hit.world_y, hit.height
                ));
            }
            if let Some(status) = editor.save_status.as_deref() {
                ui.separator();
                ui.text(status);
            }
        });

    if editor.picker_open {
        let mut picker_open = true;
        ui.window("Props")
            .opened(&mut picker_open)
            .position([12.0, 174.0], Condition::FirstUseEver)
            .size([330.0, 420.0], Condition::FirstUseEver)
            .build(|| {
                if editor.prop_ids.is_empty() {
                    ui.text("No props in catalog");
                }
                for prop_id in &editor.prop_ids {
                    let selected = editor.selected_prop.as_deref() == Some(prop_id.as_str());
                    if ui.selectable_config(prop_id).selected(selected).build() {
                        editor.selected_prop = Some(prop_id.clone());
                    }
                }
            });
        editor.picker_open = picker_open;
    }

    if editor.pending_quit {
        ui.window("Unsaved changes")
            .always_auto_resize(true)
            .position([420.0, 240.0], Condition::Appearing)
            .build(|| {
                ui.text(format!("{} dirty prop tile(s)", editor.dirty_tiles.len()));
                if ui.button("Save and Quit") {
                    editor.save_and_quit_requested = true;
                }
                ui.same_line();
                if ui.button("Discard and Quit") {
                    editor.quit_confirmed = true;
                    editor.dirty_tiles.clear();
                }
                ui.same_line();
                if ui.button("Cancel") {
                    editor.pending_quit = false;
                }
            });
    }
}

fn should_place_prop(input: &FrameInput, editor: &EditorState, imgui_wants_mouse: bool) -> bool {
    !editor.mouse_captured
        && editor.selected_prop.is_some()
        && input.mouse_pressed(MouseButton::Left)
        && !imgui_wants_mouse
        && !editor.pending_quit
}

fn place_prop(
    input: &FrameInput,
    editor: &mut EditorState,
    terrain: &TerrainMaps,
    camera: &Camera,
    window: &Window,
) -> Result<bool, Box<dyn Error>> {
    let Some(prop) = editor.selected_prop.clone() else {
        return Ok(false);
    };
    let Some(mouse_position) = input.mouse_position() else {
        return Ok(false);
    };

    let (width, height) = window.size();
    let ray = camera_screen_ray(camera, width, height, mouse_position);
    let Some(hit) = raycast_terrain(terrain.collision_height(), ray, camera.max_distance) else {
        editor.save_status = Some("No loaded terrain hit under cursor".to_owned());
        return Ok(false);
    };

    let source = source_position_from_world(&terrain.manifest, hit.world_x, hit.world_y);
    let tile_coords = prop_tile_coords_for_source(&terrain.manifest, source[0], source[1]);
    let tile = editable_tile_mut(editor, terrain, tile_coords)?;
    tile.instance
        .push(PropInstanceToml::terrain(prop, source[0], source[1]));
    editor.dirty_tiles.insert(tile_coords);
    editor.last_hit = Some(hit);
    editor.save_status = Some(format!(
        "Placed prop in tile {},{}",
        tile_coords[0], tile_coords[1]
    ));

    Ok(true)
}

fn editable_tile_mut<'a>(
    editor: &'a mut EditorState,
    terrain: &TerrainMaps,
    tile_coords: [u32; 2],
) -> Result<&'a mut PropTileToml, Box<dyn Error>> {
    match editor.edited_tiles.entry(tile_coords) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => {
            let path = terrain.manifest.props_tile_path(
                &terrain.worldmap_dir,
                tile_coords[0],
                tile_coords[1],
            );
            Ok(entry.insert(PropTileToml::load_or_empty(&path)?))
        }
    }
}

fn save_dirty_tiles(terrain: &TerrainMaps, editor: &mut EditorState) -> Result<(), Box<dyn Error>> {
    let dirty_tiles = editor.dirty_tiles.iter().copied().collect::<Vec<_>>();
    for tile_coords in &dirty_tiles {
        let Some(tile) = editor.edited_tiles.get(tile_coords) else {
            continue;
        };
        let path =
            terrain
                .manifest
                .props_tile_path(&terrain.worldmap_dir, tile_coords[0], tile_coords[1]);
        tile.save_to_path(&path)?;
    }

    editor.dirty_tiles.clear();
    editor.save_status = Some(format!("Saved {} prop tile(s)", dirty_tiles.len()));

    Ok(())
}

fn update_imgui_io(
    imgui: &mut Context,
    input: &FrameInput,
    window: &Window,
    frame_duration: Duration,
) {
    let io = imgui.io_mut();
    let (width, height) = window.size();
    io.display_size = [width as f32, height as f32];
    io.delta_time = frame_duration.as_secs_f32().max(1.0 / 240.0);
    if let Some(mouse_position) = input.mouse_position() {
        io.mouse_pos = mouse_position;
    }
    io.mouse_down = [
        input.mouse_held(MouseButton::Left),
        input.mouse_held(MouseButton::Right),
        input.mouse_held(MouseButton::Middle),
        input.mouse_held(MouseButton::X1),
        input.mouse_held(MouseButton::X2),
    ];
    io.mouse_wheel = input.wheel_delta();
    io.key_ctrl = ctrl_held(input);
    io.key_shift = input.scancode_held(Scancode::LShift) || input.scancode_held(Scancode::RShift);
    io.key_alt = input.scancode_held(Scancode::LAlt) || input.scancode_held(Scancode::RAlt);
    io.key_super = input.scancode_held(Scancode::LGui) || input.scancode_held(Scancode::RGui);

    for text in input.text_input() {
        for character in text.chars() {
            io.add_input_character(character);
        }
    }
}

fn set_mouse_capture(mouse: &sdl3::mouse::MouseUtil, window: &Window, captured: bool) {
    mouse.set_relative_mouse_mode(window, captured);
    mouse.show_cursor(!captured);
}

fn ctrl_held(input: &FrameInput) -> bool {
    input.scancode_held(Scancode::LCtrl) || input.scancode_held(Scancode::RCtrl)
}

fn limit_framerate(frame_start: Instant, max_framerate: f32) {
    if max_framerate <= 0.0 {
        return;
    }

    let target_frame_time = Duration::from_secs_f32(1.0 / max_framerate);
    let elapsed = frame_start.elapsed();
    if elapsed < target_frame_time {
        thread::sleep(target_frame_time - elapsed);
    }
}
