use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    error::Error,
    thread,
    time::{Duration, Instant},
};

use glam::{Vec2, Vec3};
use imgui::{Condition, Context, DrawListMut, ImColor32, Ui};
use sdl3::{
    gpu::{Device, ShaderFormat, SwapchainComposition},
    keyboard::{Keycode, Scancode},
    mouse::MouseButton,
    video::Window,
};
use tungsten::{
    camera::{
        Camera, camera_project_world_to_screen, camera_screen_ray, terrain_full_map_distance,
    },
    config::{AppConfig, CONFIG_PATH},
    imgui_sdl_gpu::ImguiSdlGpuRenderer,
    input::{FrameInput, InputState},
    player::PlayerController,
    props::{
        PropBounds, PropHeightMode, PropInstanceToml, PropScene, PropTileToml, PropTransform,
        prop_bounds_world_corners, prop_tile_coords_for_source, prop_transform,
        raycast_prop_bounds, source_position_from_world,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PropSelection {
    tile_coords: [u32; 2],
    instance_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GizmoHandle {
    TranslateX,
    TranslateY,
    TranslateZ,
    RotatePitch,
    RotateYaw,
    RotateRoll,
    Scale,
}

#[derive(Clone, Debug)]
struct GizmoDrag {
    handle: GizmoHandle,
    selection: PropSelection,
    last_mouse: [f32; 2],
}

#[derive(Clone, Debug)]
struct ActiveProp {
    selection: PropSelection,
    transform: PropTransform,
    bounds: PropBounds,
}

struct EditorState {
    prop_ids: Vec<String>,
    selected_prop: Option<String>,
    selected_instance: Option<PropSelection>,
    gizmo_mode: GizmoMode,
    active_drag: Option<GizmoDrag>,
    metadata_buffer: String,
    metadata_error: Option<String>,
    metadata_selection: Option<PropSelection>,
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
            selected_instance: None,
            gizmo_mode: GizmoMode::Translate,
            active_drag: None,
            metadata_buffer: String::new(),
            metadata_error: None,
            metadata_selection: None,
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
        let imgui_wants_keyboard = imgui.io().want_capture_keyboard;

        if input.quit_requested() {
            if editor.dirty_tiles.is_empty() {
                break 'running;
            }
            editor.pending_quit = true;
            editor.mouse_captured = false;
            set_mouse_capture(&mouse, &window, false);
        }
        if !imgui_wants_keyboard && input.key_pressed(Keycode::Tab) {
            editor.mouse_captured = !editor.mouse_captured;
            set_mouse_capture(&mouse, &window, editor.mouse_captured);
        }
        if !imgui_wants_keyboard && input.key_pressed(Keycode::M) {
            editor.picker_open = true;
            editor.mouse_captured = false;
            set_mouse_capture(&mouse, &window, false);
        }
        if !imgui_wants_keyboard && input.key_pressed(Keycode::W) {
            editor.gizmo_mode = GizmoMode::Translate;
        }
        if !imgui_wants_keyboard && input.key_pressed(Keycode::E) {
            editor.gizmo_mode = GizmoMode::Rotate;
        }
        if !imgui_wants_keyboard && input.key_pressed(Keycode::R) {
            editor.gizmo_mode = GizmoMode::Scale;
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

        if prop_refresh_requested {
            prop_scene.refresh_for_editor_tiles(&gpu, &terrain_maps, &editor.edited_tiles)?;
            prop_refresh_requested = false;
        } else {
            prop_scene.update_model_loads(&gpu);
        }

        let active_props = build_active_props(&editor, &terrain_maps, &prop_scene)?;
        let ui = imgui.frame();
        if draw_editor_ui(ui, &mut editor, &camera, &terrain_maps)? {
            prop_refresh_requested = true;
        }
        if update_and_draw_gizmo(
            ui,
            &input,
            &mut editor,
            &terrain_maps,
            &active_props,
            &camera,
            &window,
        )? {
            prop_refresh_requested = true;
        }
        let imgui_wants_mouse = ui.io().want_capture_mouse;

        if should_place_prop(&input, &editor, imgui_wants_mouse) {
            if place_prop(&input, &mut editor, &terrain_maps, &camera, &window)?.is_some() {
                prop_refresh_requested = true;
            }
        } else if should_pick_prop(&input, &editor, imgui_wants_mouse) {
            let selection = pick_prop(&input, &active_props, &camera, &window);
            set_selected_instance(&mut editor, &terrain_maps, selection)?;
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

fn draw_editor_ui(
    ui: &Ui,
    editor: &mut EditorState,
    camera: &Camera,
    terrain: &TerrainMaps,
) -> Result<bool, Box<dyn Error>> {
    sync_metadata_buffer(editor, terrain)?;
    let mut changed = false;

    ui.window("Editor")
        .position([12.0, 12.0], Condition::FirstUseEver)
        .size([330.0, 190.0], Condition::FirstUseEver)
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
                "placement {}",
                editor.selected_prop.as_deref().unwrap_or("<none>")
            ));
            if let Some(selection) = editor.selected_instance {
                ui.text(format!(
                    "selected tile {},{} instance {}",
                    selection.tile_coords[0], selection.tile_coords[1], selection.instance_index
                ));
            } else {
                ui.text("selected <none>");
            }
            ui.text(format!("dirty tiles {}", editor.dirty_tiles.len()));
            ui.separator();
            ui.radio_button("Move", &mut editor.gizmo_mode, GizmoMode::Translate);
            ui.same_line();
            ui.radio_button("Rotate", &mut editor.gizmo_mode, GizmoMode::Rotate);
            ui.same_line();
            ui.radio_button("Scale", &mut editor.gizmo_mode, GizmoMode::Scale);
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

    changed |= draw_detail_pane(ui, editor, terrain)?;

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

    Ok(changed)
}

fn should_place_prop(input: &FrameInput, editor: &EditorState, imgui_wants_mouse: bool) -> bool {
    !editor.mouse_captured
        && editor.selected_prop.is_some()
        && editor.active_drag.is_none()
        && input.mouse_pressed(MouseButton::Left)
        && shift_held(input)
        && !imgui_wants_mouse
        && !editor.pending_quit
}

fn should_pick_prop(input: &FrameInput, editor: &EditorState, imgui_wants_mouse: bool) -> bool {
    !editor.mouse_captured
        && editor.active_drag.is_none()
        && input.mouse_pressed(MouseButton::Left)
        && !shift_held(input)
        && !imgui_wants_mouse
        && !editor.pending_quit
}

fn place_prop(
    input: &FrameInput,
    editor: &mut EditorState,
    terrain: &TerrainMaps,
    camera: &Camera,
    window: &Window,
) -> Result<Option<PropSelection>, Box<dyn Error>> {
    let Some(prop) = editor.selected_prop.clone() else {
        return Ok(None);
    };
    let Some(mouse_position) = input.mouse_position() else {
        return Ok(None);
    };

    let (width, height) = window.size();
    let ray = camera_screen_ray(camera, width, height, mouse_position);
    let Some(hit) = raycast_terrain(terrain.collision_height(), ray, camera.max_distance) else {
        editor.save_status = Some("No loaded terrain hit under cursor".to_owned());
        return Ok(None);
    };

    let source = source_position_from_world(&terrain.manifest, hit.world_x, hit.world_y);
    let tile_coords = prop_tile_coords_for_source(&terrain.manifest, source[0], source[1]);
    let instance_index = {
        let tile = editable_tile_mut(editor, terrain, tile_coords)?;
        tile.instance
            .push(PropInstanceToml::terrain(prop, source[0], source[1]));
        tile.instance.len() - 1
    };
    let selection = PropSelection {
        tile_coords,
        instance_index,
    };
    editor.dirty_tiles.insert(tile_coords);
    editor.last_hit = Some(hit);
    editor.save_status = Some(format!(
        "Placed prop in tile {},{}",
        tile_coords[0], tile_coords[1]
    ));
    set_selected_instance(editor, terrain, Some(selection))?;

    Ok(Some(selection))
}

fn pick_prop(
    input: &FrameInput,
    active_props: &[ActiveProp],
    camera: &Camera,
    window: &Window,
) -> Option<PropSelection> {
    let mouse_position = input.mouse_position()?;
    let (width, height) = window.size();
    let ray = camera_screen_ray(camera, width, height, mouse_position);
    let mut best_hit = None;

    for prop in active_props {
        let Some(distance) = raycast_prop_bounds(ray, prop.transform, prop.bounds) else {
            continue;
        };
        if distance > camera.max_distance {
            continue;
        }
        if best_hit.is_none_or(|(best_distance, _)| distance < best_distance) {
            best_hit = Some((distance, prop.selection));
        }
    }

    best_hit.map(|(_, selection)| selection)
}

fn build_active_props(
    editor: &EditorState,
    terrain: &TerrainMaps,
    prop_scene: &PropScene,
) -> Result<Vec<ActiveProp>, Box<dyn Error>> {
    let mut active_props = Vec::new();

    for tile_y in terrain.current_window_min[1]..=terrain.current_window_max[1] {
        for tile_x in terrain.current_window_min[0]..=terrain.current_window_max[0] {
            let tile_coords = [tile_x, tile_y];
            let tile = prop_tile_for_read(editor, terrain, tile_coords)?;
            for (instance_index, instance) in tile.instance.iter().enumerate() {
                let Some(definition) = prop_scene.catalog().definition(&instance.prop) else {
                    continue;
                };
                let Some(model) = prop_scene.model(&definition.model_path) else {
                    continue;
                };
                active_props.push(ActiveProp {
                    selection: PropSelection {
                        tile_coords,
                        instance_index,
                    },
                    transform: prop_transform(
                        instance,
                        &terrain.manifest,
                        terrain.collision_height(),
                    ),
                    bounds: model.bounds,
                });
            }
        }
    }

    Ok(active_props)
}

fn draw_detail_pane(
    ui: &Ui,
    editor: &mut EditorState,
    terrain: &TerrainMaps,
) -> Result<bool, Box<dyn Error>> {
    let Some(selection) = editor.selected_instance else {
        return Ok(false);
    };
    let Some(mut edited_instance) = selected_instance_clone(editor, terrain)? else {
        ui.window("Selected Prop")
            .position([360.0, 12.0], Condition::FirstUseEver)
            .size([390.0, 120.0], Condition::FirstUseEver)
            .build(|| {
                ui.text("Selection no longer exists");
            });
        return Ok(false);
    };

    let mut instance_changed = false;
    let mut apply_metadata = false;
    let height_modes = ["terrain", "absolute"];
    let mut height_mode_index = match edited_instance.height_mode {
        PropHeightMode::Terrain => 0,
        PropHeightMode::Absolute => 1,
    };

    ui.window("Selected Prop")
        .position([360.0, 12.0], Condition::FirstUseEver)
        .size([410.0, 470.0], Condition::FirstUseEver)
        .build(|| {
            ui.text(format!(
                "tile {},{} instance {}",
                selection.tile_coords[0], selection.tile_coords[1], selection.instance_index
            ));
            ui.separator();

            if let Some(_combo) = ui.begin_combo("prop", &edited_instance.prop) {
                for prop_id in &editor.prop_ids {
                    let selected = edited_instance.prop == *prop_id;
                    if ui.selectable_config(prop_id).selected(selected).build() {
                        edited_instance.prop = prop_id.clone();
                        instance_changed = true;
                    }
                    if selected {
                        ui.set_item_default_focus();
                    }
                }
            }

            if ui.combo_simple_string("height mode", &mut height_mode_index, &height_modes) {
                edited_instance.height_mode = if height_mode_index == 0 {
                    PropHeightMode::Terrain
                } else {
                    PropHeightMode::Absolute
                };
                instance_changed = true;
            }

            ui.separator();
            if edit_float(ui, "source x", &mut edited_instance.source_x, 0.25) {
                edited_instance.source_x = clamp_source_x(terrain, edited_instance.source_x);
                instance_changed = true;
            }
            if edit_float(ui, "source y", &mut edited_instance.source_y, 0.25) {
                edited_instance.source_y = clamp_source_y(terrain, edited_instance.source_y);
                instance_changed = true;
            }
            if edit_float(ui, "height", &mut edited_instance.height, 0.25) {
                instance_changed = true;
            }
            if edit_float(
                ui,
                "height offset",
                &mut edited_instance.height_offset,
                0.25,
            ) {
                instance_changed = true;
            }

            ui.separator();
            if edit_float(ui, "pitch", &mut edited_instance.pitch, 0.01) {
                instance_changed = true;
            }
            if edit_float(ui, "yaw", &mut edited_instance.yaw, 0.01) {
                instance_changed = true;
            }
            if edit_float(ui, "roll", &mut edited_instance.roll, 0.01) {
                instance_changed = true;
            }
            if edit_float(ui, "scale", &mut edited_instance.scale, 0.05) {
                edited_instance.scale = edited_instance.scale.max(0.01);
                instance_changed = true;
            }

            ui.separator();
            ui.input_text_multiline("metadata", &mut editor.metadata_buffer, [380.0, 120.0])
                .build();
            if ui.button("Apply metadata") {
                apply_metadata = true;
            }
            if let Some(error) = editor.metadata_error.as_deref() {
                ui.text_colored([1.0, 0.35, 0.35, 1.0], error);
            }
        });

    let mut changed = false;
    if instance_changed {
        update_selected_instance(editor, terrain, edited_instance)?;
        changed = true;
    }
    if apply_metadata {
        changed |= apply_metadata_buffer(editor, terrain)?;
    }

    Ok(changed)
}

fn edit_float(ui: &Ui, label: &str, value: &mut f32, step: f32) -> bool {
    let previous = *value;
    if !ui
        .input_float(label, value)
        .step(step)
        .step_fast(step * 10.0)
        .display_format("%.3f")
        .build()
    {
        return false;
    }

    if value.is_finite() {
        true
    } else {
        *value = previous;
        false
    }
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

fn prop_tile_for_read(
    editor: &EditorState,
    terrain: &TerrainMaps,
    tile_coords: [u32; 2],
) -> Result<PropTileToml, Box<dyn Error>> {
    if let Some(tile) = editor.edited_tiles.get(&tile_coords) {
        return Ok(tile.clone());
    }

    let path =
        terrain
            .manifest
            .props_tile_path(&terrain.worldmap_dir, tile_coords[0], tile_coords[1]);
    PropTileToml::load_or_empty(&path)
}

fn selected_instance_clone(
    editor: &EditorState,
    terrain: &TerrainMaps,
) -> Result<Option<PropInstanceToml>, Box<dyn Error>> {
    let Some(selection) = editor.selected_instance else {
        return Ok(None);
    };
    let tile = prop_tile_for_read(editor, terrain, selection.tile_coords)?;

    Ok(tile.instance.get(selection.instance_index).cloned())
}

fn set_selected_instance(
    editor: &mut EditorState,
    terrain: &TerrainMaps,
    selection: Option<PropSelection>,
) -> Result<(), Box<dyn Error>> {
    editor.selected_instance = selection;
    editor.active_drag = None;
    sync_metadata_buffer(editor, terrain)
}

fn sync_metadata_buffer(
    editor: &mut EditorState,
    terrain: &TerrainMaps,
) -> Result<(), Box<dyn Error>> {
    if editor.metadata_selection == editor.selected_instance {
        return Ok(());
    }

    let Some(selection) = editor.selected_instance else {
        editor.metadata_buffer.clear();
        editor.metadata_error = None;
        editor.metadata_selection = None;
        return Ok(());
    };

    let tile = prop_tile_for_read(editor, terrain, selection.tile_coords)?;
    if let Some(instance) = tile.instance.get(selection.instance_index) {
        editor.metadata_buffer = toml::to_string_pretty(&instance.metadata)
            .map_err(|error| format!("failed to serialize prop metadata: {error}"))?;
        editor.metadata_error = None;
        editor.metadata_selection = Some(selection);
    } else {
        editor.selected_instance = None;
        editor.metadata_buffer.clear();
        editor.metadata_error = None;
        editor.metadata_selection = None;
    }

    Ok(())
}

fn apply_metadata_buffer(
    editor: &mut EditorState,
    terrain: &TerrainMaps,
) -> Result<bool, Box<dyn Error>> {
    let Some(mut instance) = selected_instance_clone(editor, terrain)? else {
        return Ok(false);
    };

    let metadata = if editor.metadata_buffer.trim().is_empty() {
        Ok(toml::Table::new())
    } else {
        toml::from_str::<toml::Table>(&editor.metadata_buffer)
    };

    match metadata {
        Ok(metadata) => {
            instance.metadata = metadata;
            update_selected_instance(editor, terrain, instance)?;
            editor.metadata_error = None;
            Ok(true)
        }
        Err(error) => {
            editor.metadata_error = Some(error.to_string());
            Ok(false)
        }
    }
}

fn update_selected_instance(
    editor: &mut EditorState,
    terrain: &TerrainMaps,
    mut updated: PropInstanceToml,
) -> Result<(), Box<dyn Error>> {
    let Some(selection) = editor.selected_instance else {
        return Ok(());
    };

    updated.source_x = clamp_source_x(terrain, updated.source_x);
    updated.source_y = clamp_source_y(terrain, updated.source_y);
    updated.scale = updated.scale.max(0.01);

    let new_tile_coords =
        prop_tile_coords_for_source(&terrain.manifest, updated.source_x, updated.source_y);
    if new_tile_coords == selection.tile_coords {
        let mut replaced = false;
        {
            let tile = editable_tile_mut(editor, terrain, selection.tile_coords)?;
            if let Some(instance) = tile.instance.get_mut(selection.instance_index) {
                *instance = updated;
                replaced = true;
            }
        }
        if replaced {
            editor.dirty_tiles.insert(selection.tile_coords);
        } else {
            editor.selected_instance = None;
            editor.active_drag = None;
        }
        return sync_metadata_buffer(editor, terrain);
    }

    let mut removed = false;
    {
        let old_tile = editable_tile_mut(editor, terrain, selection.tile_coords)?;
        if selection.instance_index < old_tile.instance.len() {
            old_tile.instance.remove(selection.instance_index);
            removed = true;
        }
    }
    if !removed {
        editor.selected_instance = None;
        editor.active_drag = None;
        return sync_metadata_buffer(editor, terrain);
    }
    editor.dirty_tiles.insert(selection.tile_coords);

    let new_index = {
        let new_tile = editable_tile_mut(editor, terrain, new_tile_coords)?;
        new_tile.instance.push(updated);
        new_tile.instance.len() - 1
    };
    editor.dirty_tiles.insert(new_tile_coords);
    editor.selected_instance = Some(PropSelection {
        tile_coords: new_tile_coords,
        instance_index: new_index,
    });

    sync_metadata_buffer(editor, terrain)
}

fn update_and_draw_gizmo(
    ui: &Ui,
    input: &FrameInput,
    editor: &mut EditorState,
    terrain: &TerrainMaps,
    active_props: &[ActiveProp],
    camera: &Camera,
    window: &Window,
) -> Result<bool, Box<dyn Error>> {
    let Some(selection) = editor.selected_instance else {
        editor.active_drag = None;
        return Ok(false);
    };
    let Some(active_prop) = active_props.iter().find(|prop| prop.selection == selection) else {
        editor.active_drag = None;
        return Ok(false);
    };

    let (width, height) = window.size();
    let Some(center) = project_screen(camera, width, height, active_prop.transform.position) else {
        return Ok(false);
    };

    let draw_list = ui.get_foreground_draw_list();
    let active_handle = editor
        .active_drag
        .as_ref()
        .filter(|drag| drag.selection == selection)
        .map(|drag| drag.handle);
    let hovered_handle = if active_handle.is_none()
        && !ui.io().want_capture_mouse
        && !shift_held(input)
        && !editor.pending_quit
    {
        input.mouse_position().and_then(|mouse| {
            hit_gizmo_handle(
                editor.gizmo_mode,
                Vec2::from_array(mouse),
                center,
                active_prop.transform,
                camera,
                width,
                height,
            )
        })
    } else {
        None
    };

    draw_selected_bounds(
        &draw_list,
        active_prop.transform,
        active_prop.bounds,
        camera,
        width,
        height,
    );
    match editor.gizmo_mode {
        GizmoMode::Translate => draw_translate_gizmo(
            &draw_list,
            active_prop.transform,
            camera,
            width,
            height,
            hovered_handle,
            active_handle,
        ),
        GizmoMode::Rotate => draw_rotate_gizmo(&draw_list, center, hovered_handle, active_handle),
        GizmoMode::Scale => draw_scale_gizmo(&draw_list, center, hovered_handle, active_handle),
    }

    if active_handle.is_none()
        && input.mouse_pressed(MouseButton::Left)
        && !shift_held(input)
        && !ui.io().want_capture_mouse
    {
        if let (Some(handle), Some(mouse)) = (hovered_handle, input.mouse_position()) {
            editor.active_drag = Some(GizmoDrag {
                handle,
                selection,
                last_mouse: mouse,
            });
        }
    }

    let Some(drag) = editor.active_drag.clone() else {
        return Ok(false);
    };
    if drag.selection != selection || input.mouse_released(MouseButton::Left) {
        editor.active_drag = None;
        return Ok(false);
    }
    if !input.mouse_held(MouseButton::Left) {
        return Ok(false);
    }
    let Some(mouse) = input.mouse_position() else {
        return Ok(false);
    };
    if Vec2::from_array(mouse).distance_squared(Vec2::from_array(drag.last_mouse)) <= 0.0001 {
        return Ok(false);
    }

    let changed = apply_gizmo_drag(
        editor,
        terrain,
        active_prop,
        camera,
        width,
        height,
        drag.handle,
        drag.last_mouse,
        mouse,
    )?;
    if let Some(active_drag) = editor.active_drag.as_mut() {
        active_drag.last_mouse = mouse;
    }

    Ok(changed)
}

fn draw_selected_bounds(
    draw_list: &DrawListMut<'_>,
    transform: PropTransform,
    bounds: PropBounds,
    camera: &Camera,
    width: u32,
    height: u32,
) {
    const EDGES: [(usize, usize); 12] = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];

    let corners = prop_bounds_world_corners(transform, bounds);
    let projected = corners.map(|corner| project_screen(camera, width, height, corner));
    for (start, end) in EDGES {
        let (Some(start), Some(end)) = (projected[start], projected[end]) else {
            continue;
        };
        draw_list
            .add_line(
                start.to_array(),
                end.to_array(),
                ImColor32::from_rgba(255, 214, 92, 220),
            )
            .thickness(1.5)
            .build();
    }
}

fn draw_translate_gizmo(
    draw_list: &DrawListMut<'_>,
    transform: PropTransform,
    camera: &Camera,
    width: u32,
    height: u32,
    hovered: Option<GizmoHandle>,
    active: Option<GizmoHandle>,
) {
    for (handle, axis) in [
        (GizmoHandle::TranslateX, Vec3::X),
        (GizmoHandle::TranslateY, Vec3::Y),
        (GizmoHandle::TranslateZ, Vec3::Z),
    ] {
        let Some((start, end)) =
            translate_axis_screen(camera, width, height, transform.position, axis)
        else {
            continue;
        };
        let color = gizmo_handle_color(handle);
        let thickness = gizmo_handle_thickness(handle, hovered, active);
        draw_list
            .add_line(start.to_array(), end.to_array(), color)
            .thickness(thickness)
            .build();
        draw_list
            .add_circle(end.to_array(), 5.0 + thickness, color)
            .filled(true)
            .build();
    }
}

fn draw_rotate_gizmo(
    draw_list: &DrawListMut<'_>,
    center: Vec2,
    hovered: Option<GizmoHandle>,
    active: Option<GizmoHandle>,
) {
    for (handle, radius) in [
        (GizmoHandle::RotateYaw, 34.0),
        (GizmoHandle::RotatePitch, 44.0),
        (GizmoHandle::RotateRoll, 54.0),
    ] {
        draw_list
            .add_circle(center.to_array(), radius, gizmo_handle_color(handle))
            .num_segments(72)
            .thickness(gizmo_handle_thickness(handle, hovered, active))
            .build();
    }
}

fn draw_scale_gizmo(
    draw_list: &DrawListMut<'_>,
    center: Vec2,
    hovered: Option<GizmoHandle>,
    active: Option<GizmoHandle>,
) {
    let handle = GizmoHandle::Scale;
    let end = center + Vec2::new(52.0, -52.0);
    let color = gizmo_handle_color(handle);
    let thickness = gizmo_handle_thickness(handle, hovered, active);
    draw_list
        .add_line(center.to_array(), end.to_array(), color)
        .thickness(thickness)
        .build();
    draw_list
        .add_rect(
            (end - Vec2::splat(6.0)).to_array(),
            (end + Vec2::splat(6.0)).to_array(),
            color,
        )
        .filled(true)
        .build();
}

fn hit_gizmo_handle(
    mode: GizmoMode,
    mouse: Vec2,
    center: Vec2,
    transform: PropTransform,
    camera: &Camera,
    width: u32,
    height: u32,
) -> Option<GizmoHandle> {
    match mode {
        GizmoMode::Translate => {
            let mut best = None;
            for (handle, axis) in [
                (GizmoHandle::TranslateX, Vec3::X),
                (GizmoHandle::TranslateY, Vec3::Y),
                (GizmoHandle::TranslateZ, Vec3::Z),
            ] {
                let Some((start, end)) =
                    translate_axis_screen(camera, width, height, transform.position, axis)
                else {
                    continue;
                };
                let distance = distance_to_segment(mouse, start, end).min(mouse.distance(end));
                if distance <= 10.0
                    && best.is_none_or(|(best_distance, _)| distance < best_distance)
                {
                    best = Some((distance, handle));
                }
            }
            best.map(|(_, handle)| handle)
        }
        GizmoMode::Rotate => {
            let radius = mouse.distance(center);
            let mut best = None;
            for (handle, ring_radius) in [
                (GizmoHandle::RotateYaw, 34.0),
                (GizmoHandle::RotatePitch, 44.0),
                (GizmoHandle::RotateRoll, 54.0),
            ] {
                let distance = (radius - ring_radius).abs();
                if distance <= 7.0 && best.is_none_or(|(best_distance, _)| distance < best_distance)
                {
                    best = Some((distance, handle));
                }
            }
            best.map(|(_, handle)| handle)
        }
        GizmoMode::Scale => {
            let end = center + Vec2::new(52.0, -52.0);
            let distance = distance_to_segment(mouse, center, end).min(mouse.distance(end));
            (distance <= 10.0).then_some(GizmoHandle::Scale)
        }
    }
}

fn apply_gizmo_drag(
    editor: &mut EditorState,
    terrain: &TerrainMaps,
    active_prop: &ActiveProp,
    camera: &Camera,
    width: u32,
    height: u32,
    handle: GizmoHandle,
    previous_mouse: [f32; 2],
    current_mouse: [f32; 2],
) -> Result<bool, Box<dyn Error>> {
    let Some(mut instance) = selected_instance_clone(editor, terrain)? else {
        return Ok(false);
    };
    let delta = Vec2::from_array(current_mouse) - Vec2::from_array(previous_mouse);

    let changed = match handle {
        GizmoHandle::TranslateX | GizmoHandle::TranslateY | GizmoHandle::TranslateZ => {
            let axis = match handle {
                GizmoHandle::TranslateX => Vec3::X,
                GizmoHandle::TranslateY => Vec3::Y,
                GizmoHandle::TranslateZ => Vec3::Z,
                _ => unreachable!(),
            };
            let Some(amount) = screen_axis_drag_amount(
                camera,
                width,
                height,
                active_prop.transform.position,
                axis,
                delta,
            ) else {
                return Ok(false);
            };
            if amount.abs() <= 0.00001 {
                return Ok(false);
            }
            match handle {
                GizmoHandle::TranslateX => {
                    translate_instance_world(&mut instance, terrain, amount, 0.0);
                }
                GizmoHandle::TranslateY => match instance.height_mode {
                    PropHeightMode::Terrain => instance.height_offset += amount,
                    PropHeightMode::Absolute => instance.height += amount,
                },
                GizmoHandle::TranslateZ => {
                    translate_instance_world(&mut instance, terrain, 0.0, amount);
                }
                _ => unreachable!(),
            }
            true
        }
        GizmoHandle::RotatePitch => {
            instance.pitch -= delta.y * 0.01;
            true
        }
        GizmoHandle::RotateYaw => {
            instance.yaw += delta.x * 0.01;
            true
        }
        GizmoHandle::RotateRoll => {
            instance.roll += delta.x * 0.01;
            true
        }
        GizmoHandle::Scale => {
            let previous_scale = instance.scale;
            let factor = (1.0 + (delta.x - delta.y) * 0.01).max(0.01);
            instance.scale = (instance.scale * factor).max(0.01);
            (instance.scale - previous_scale).abs() > 0.00001
        }
    };

    if changed {
        update_selected_instance(editor, terrain, instance)?;
    }
    Ok(changed)
}

fn translate_axis_screen(
    camera: &Camera,
    width: u32,
    height: u32,
    center: Vec3,
    axis: Vec3,
) -> Option<(Vec2, Vec2)> {
    let start = project_screen(camera, width, height, center)?;
    let end = project_screen(
        camera,
        width,
        height,
        center + axis * gizmo_handle_world_len(camera, center),
    )?;

    Some((start, end))
}

fn screen_axis_drag_amount(
    camera: &Camera,
    width: u32,
    height: u32,
    center: Vec3,
    axis: Vec3,
    screen_delta: Vec2,
) -> Option<f32> {
    let (start, end) = translate_axis_screen(camera, width, height, center, axis)?;
    let screen_axis = end - start;
    let screen_len = screen_axis.length();
    if screen_len <= 1.0 {
        return None;
    }

    let world_len = gizmo_handle_world_len(camera, center);
    Some(screen_delta.dot(screen_axis / screen_len) * (world_len / screen_len))
}

fn gizmo_handle_world_len(camera: &Camera, center: Vec3) -> f32 {
    let camera_pos = Vec3::new(camera.x, camera.height, camera.y);
    camera_pos
        .distance(center)
        .mul_add(0.12, 0.0)
        .clamp(2.0, 80.0)
}

fn translate_instance_world(
    instance: &mut PropInstanceToml,
    terrain: &TerrainMaps,
    world_delta_x: f32,
    world_delta_y: f32,
) {
    let source = source_position_from_world(
        &terrain.manifest,
        instance.source_x * terrain.manifest.horizontal_scale + world_delta_x,
        instance.source_y * terrain.manifest.horizontal_scale + world_delta_y,
    );
    instance.source_x = source[0];
    instance.source_y = source[1];
}

fn project_screen(camera: &Camera, width: u32, height: u32, world: Vec3) -> Option<Vec2> {
    camera_project_world_to_screen(camera, width, height, world).map(Vec2::from_array)
}

fn distance_to_segment(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    let length_sq = segment.length_squared();
    if length_sq <= 0.0001 {
        return point.distance(start);
    }

    let t = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

fn gizmo_handle_color(handle: GizmoHandle) -> ImColor32 {
    match handle {
        GizmoHandle::TranslateX | GizmoHandle::RotatePitch => {
            ImColor32::from_rgba(236, 88, 88, 230)
        }
        GizmoHandle::TranslateY | GizmoHandle::RotateYaw => ImColor32::from_rgba(89, 197, 112, 230),
        GizmoHandle::TranslateZ | GizmoHandle::RotateRoll => {
            ImColor32::from_rgba(83, 139, 240, 230)
        }
        GizmoHandle::Scale => ImColor32::from_rgba(244, 205, 75, 230),
    }
}

fn gizmo_handle_thickness(
    handle: GizmoHandle,
    hovered: Option<GizmoHandle>,
    active: Option<GizmoHandle>,
) -> f32 {
    if active == Some(handle) {
        5.0
    } else if hovered == Some(handle) {
        4.0
    } else {
        2.5
    }
}

fn clamp_source_x(terrain: &TerrainMaps, value: f32) -> f32 {
    value.clamp(0.0, terrain.manifest.source_width as f32 - 0.001)
}

fn clamp_source_y(terrain: &TerrainMaps, value: f32) -> f32 {
    value.clamp(0.0, terrain.manifest.source_height as f32 - 0.001)
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

fn shift_held(input: &FrameInput) -> bool {
    input.scancode_held(Scancode::LShift) || input.scancode_held(Scancode::RShift)
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
