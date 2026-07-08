mod camera;
mod config;
mod raster_model;
mod renderer;
mod terrain;
mod water;

use std::{
    env,
    error::Error,
    ffi::OsString,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use camera::{
    Camera, CameraMode, CameraRecorder, CameraReplay, CameraTrace, PlayerPhysics, ReplayStats,
    apply_gravity_wheel_adjustment, enable_gravity_mode, print_camera_recording_summary,
    print_replay_stats, terrain_full_map_distance, toggle_camera_recording, update_freecam,
    update_gravity_camera, write_replay_stats_csv,
};
use config::{AppConfig, CONFIG_PATH};
use renderer::{DebugVisualMode, OverlayStats, Renderer};
use sdl3::{
    event::Event,
    gpu::{Device, ShaderFormat, SwapchainComposition},
    keyboard::{Keycode, Scancode},
};

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

#[derive(Debug, PartialEq, Eq)]
struct AppArgs {
    replay_camera: Option<PathBuf>,
}

impl AppArgs {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, Box<dyn Error>> {
        let mut replay_camera = None;
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.to_str() {
                Some("--replay-camera") => {
                    replay_camera =
                        Some(PathBuf::from(next_arg_value(&mut args, "--replay-camera")?))
                }
                Some("--help" | "-h") => return Err(app_usage().into()),
                Some(flag) if flag.starts_with('-') => {
                    return Err(format!("unknown flag: {flag}").into());
                }
                _ => return Err(app_usage().into()),
            }
        }

        Ok(Self { replay_camera })
    }
}

fn next_arg_value(
    args: &mut impl Iterator<Item = OsString>,
    flag: &'static str,
) -> Result<OsString, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn app_usage() -> &'static str {
    "usage: tungsten [--replay-camera <camera-trace.tsv>]"
}

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

fn main() -> Result<(), Box<dyn Error>> {
    let args = AppArgs::parse(env::args_os().skip(1))?;
    let replay_trace = args
        .replay_camera
        .as_deref()
        .map(CameraTrace::load)
        .transpose()?;
    let replay_enabled = replay_trace.is_some();
    let mut replay = replay_trace.map(CameraReplay::new);
    let mut replay_stats = if replay_enabled {
        Some(ReplayStats::new())
    } else {
        None
    };
    let mut replay_completed = false;

    let config = AppConfig::load(CONFIG_PATH)?;
    let sdl = sdl3::init()?;
    let video = sdl.video()?;

    let mut window_builder =
        video.window("tungsten - SDL_GPU VoxelSpace", WINDOW_WIDTH, WINDOW_HEIGHT);
    window_builder.position_centered().resizable();
    if replay_enabled {
        window_builder.fullscreen();
    }
    let window = window_builder.build()?;

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
    let mut renderer = Renderer::new(&gpu, target_format, &config)?;
    let mouse = sdl.mouse();
    mouse.set_relative_mouse_mode(&window, true);
    mouse.show_cursor(false);

    let mut camera = Camera {
        x: config.start_x,
        y: config.start_y,
        height: config.start_height,
        yaw: 0.4,
        pitch: -0.08,
        vertical_fov: 1.05,
        max_distance: terrain_full_map_distance(terrain_maps.terrain_size),
    };
    let mut camera_mode = CameraMode::Freecam;
    let mut player_physics = PlayerPhysics::new();
    let mut fps_counter = FpsCounter::new();
    let mut debug_visual_mode = DebugVisualMode::None;
    let mut camera_recorder: Option<CameraRecorder> = None;

    let mut events = sdl.event_pump()?;
    let mut previous_frame = Instant::now();

    'running: loop {
        let mut mouse_delta = [0.0, 0.0];
        let mut wheel_delta = 0.0;
        let mut jump_requested = false;
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::MouseMotion { xrel, yrel, .. } if !replay_enabled => {
                    mouse_delta[0] += xrel;
                    mouse_delta[1] += yrel;
                }
                Event::MouseWheel { y, .. } if !replay_enabled => {
                    wheel_delta += y;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::G),
                    repeat: false,
                    ..
                } if !replay_enabled => {
                    camera_mode = match camera_mode {
                        CameraMode::Freecam => {
                            terrain_maps
                                .update_tile_cache_for_position(&gpu, camera.x, camera.y)?;
                            enable_gravity_mode(
                                &mut camera,
                                &mut player_physics,
                                terrain_maps.collision_height(),
                            );
                            CameraMode::Gravity
                        }
                        CameraMode::Gravity => CameraMode::Freecam,
                    };
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Space),
                    repeat: false,
                    ..
                } if !replay_enabled => {
                    jump_requested = true;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F3),
                    repeat: false,
                    ..
                } if !replay_enabled => {
                    if config.render_debug_visuals {
                        debug_visual_mode = debug_visual_mode.next();
                        println!(
                            "debug visuals: {}\n{}",
                            debug_visual_mode.label(),
                            debug_visual_mode.color_key()
                        );
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F11),
                    repeat: false,
                    ..
                } if !replay_enabled => {
                    toggle_camera_recording(&mut camera_recorder, &camera)?;
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let frame_duration = now - previous_frame;
        let dt = frame_duration.min(Duration::from_millis(50));
        previous_frame = now;

        if let Some(active_replay) = replay.as_ref() {
            if !active_replay.apply_to_camera(&mut camera) {
                replay_completed = true;
                break 'running;
            }
        } else {
            if camera_mode == CameraMode::Gravity {
                terrain_maps.update_tile_cache_for_position(&gpu, camera.x, camera.y)?;
            }
            if camera_mode == CameraMode::Gravity && wheel_delta != 0.0 {
                let keyboard = events.keyboard_state();
                let adjust_move_speed = keyboard.is_scancode_pressed(Scancode::LShift)
                    || keyboard.is_scancode_pressed(Scancode::RShift);
                apply_gravity_wheel_adjustment(
                    &mut camera,
                    &mut player_physics,
                    terrain_maps.collision_height(),
                    wheel_delta,
                    adjust_move_speed,
                );
            }
            match camera_mode {
                CameraMode::Freecam => {
                    update_freecam(&events, &mut camera, dt.as_secs_f32(), mouse_delta)
                }
                CameraMode::Gravity => update_gravity_camera(
                    &events,
                    &mut camera,
                    &mut player_physics,
                    terrain_maps.collision_height(),
                    dt.as_secs_f32(),
                    mouse_delta,
                    jump_requested,
                ),
            }
        }

        terrain_maps.update_tile_cache_for_position(&gpu, camera.x, camera.y)?;

        let frame_submitted = renderer.render_frame(
            &gpu,
            &window,
            &terrain_maps,
            &camera,
            &config,
            debug_visual_mode,
            fps_counter.overlay_stats(),
        )?;

        fps_counter.update(frame_duration);
        limit_framerate(now, config.max_framerate);

        if frame_submitted {
            if let Some(active_recorder) = camera_recorder.as_mut() {
                active_recorder.update_after_submitted_frame(&camera)?;
            }
            if let Some(active_replay) = replay.as_mut() {
                if let Some(stats) = replay_stats.as_mut() {
                    stats.record_frame(active_replay.current_frame(), now.elapsed());
                }
                if !active_replay.advance_after_submitted_frame() {
                    replay_completed = true;
                    break 'running;
                }
            }
        }
    }

    if let Some(active_recorder) = camera_recorder.take() {
        print_camera_recording_summary(active_recorder.finish()?);
    }
    if replay_completed {
        if let Some(stats) = replay_stats.as_ref() {
            print_replay_stats(stats);
            let fps_csv_path = write_replay_stats_csv(stats)?;
            println!("fps_csv: {}", fps_csv_path.display());
        }
    }

    Ok(())
}

fn limit_framerate(frame_start: Instant, max_framerate: f32) {
    if max_framerate <= 0.0 {
        return;
    }

    let target_frame_duration = Duration::from_secs_f32(1.0 / max_framerate);
    if let Some(remaining) = target_frame_duration.checked_sub(frame_start.elapsed()) {
        thread::sleep(remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(args: &[&str]) -> Vec<std::ffi::OsString> {
        args.iter().map(std::ffi::OsString::from).collect()
    }

    #[test]
    fn parses_main_args() {
        assert_eq!(
            AppArgs::parse(Vec::<std::ffi::OsString>::new()).unwrap(),
            AppArgs {
                replay_camera: None
            }
        );
        assert_eq!(
            AppArgs::parse(os_args(&["--replay-camera", "trace.tsv"])).unwrap(),
            AppArgs {
                replay_camera: Some(PathBuf::from("trace.tsv"))
            }
        );

        let error = AppArgs::parse(os_args(&["--replay-camera"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("--replay-camera requires a value"));

        let error = AppArgs::parse(os_args(&["--unknown"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown flag"));

        let error = AppArgs::parse(os_args(&["--help"]))
            .unwrap_err()
            .to_string();
        assert!(error.contains("usage: tungsten"));
    }
}
