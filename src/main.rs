use std::{
    env,
    error::Error,
    ffi::OsString,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use sdl3::gpu::{Device, ShaderFormat, SwapchainComposition};
use tungsten::{
    camera::{Camera, terrain_full_map_distance},
    camera_trace::{
        CameraReplay, CameraTrace, ReplayStats, print_replay_stats, write_replay_stats_csv,
    },
    config::{AppConfig, CONFIG_PATH},
    game_controls::GameControls,
    input::InputState,
    props::PropScene,
    renderer::{OverlayStats, Renderer},
    terrain,
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
    let mut prop_scene = PropScene::load(&gpu, &terrain_maps)?;
    let mut renderer = Renderer::new(&gpu, target_format)?;
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
    let mut game_controls = GameControls::new(config.render_debug_visuals);
    let mut fps_counter = FpsCounter::new();

    let mut events = sdl.event_pump()?;
    let mut input_state = InputState::new();
    let mut previous_frame = Instant::now();

    'running: loop {
        let input = input_state.poll(&mut events, !replay_enabled);
        if input.quit_requested() {
            break 'running;
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
            game_controls.update(
                &input,
                &gpu,
                &mut terrain_maps,
                &mut camera,
                dt.as_secs_f32(),
            )?;
        }

        terrain_maps.update_tile_cache_for_position(&gpu, camera.x, camera.y)?;
        prop_scene.update_for_terrain(&gpu, &terrain_maps)?;

        let frame_submitted = renderer.render_frame(
            &gpu,
            &window,
            &terrain_maps,
            &prop_scene,
            &camera,
            &config,
            game_controls.debug_visual_mode(),
            fps_counter.overlay_stats(),
        )?;

        fps_counter.update(frame_duration);
        limit_framerate(now, config.max_framerate);

        if frame_submitted {
            game_controls.update_after_submitted_frame(&camera)?;
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

    game_controls.finish_recording()?;
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
