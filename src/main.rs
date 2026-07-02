use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sdl3::{
    event::Event,
    gpu::{
        ColorTargetDescription, ColorTargetInfo, Device, FillMode, Filter, GraphicsPipeline,
        GraphicsPipelineTargetInfo, LoadOp, PresentMode, PrimitiveType, Sampler,
        SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode, ShaderFormat, ShaderStage,
        StoreOp, SwapchainComposition, Texture, TextureCreateInfo, TextureFormat, TextureRegion,
        TextureSamplerBinding, TextureTransferInfo, TextureType, TextureUsage, TransferBufferUsage,
    },
    keyboard::{Keycode, Scancode},
    pixels::Color,
};
use tungsten::worldmap::{WorldmapManifest, manifest_dir};

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;
const CONFIG_PATH: &str = "config.toml";
const CAMERA_RECORDING_DIR: &str = "recordings";
const CAMERA_RECORDING_INTERVAL_FRAMES: u64 = 10;
const REPLAY_STATS_BUCKET_FRAMES: u64 = 10;
const REPLAY_SUMMARY_WARMUP_FRAMES: u64 = 10;
const DEFAULT_WORLDMAP_PATH: &str = "assets/worldmaps/continent/manifest.toml";
const RAYMARCH_START_DISTANCE: f32 = 0.05;
const DEFAULT_START_X: f32 = 250.0;
const DEFAULT_START_Y: f32 = 330.0;
const DEFAULT_START_HEIGHT: f32 = 150.0;
const DEFAULT_TILE_CACHE_RADIUS: u32 = 1;
const MAX_TILE_CACHE_RADIUS: u32 = 2;
const MAX_SHADER_TILE_SLOTS: usize = 25;
const R16_BYTES_PER_PIXEL: usize = 2;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const DEFAULT_RAY_ITERATION_COUNT: u32 = 700;
const MAX_RAY_ITERATION_COUNT: u32 = 4096;
const DEFAULT_NEAR_DDA_DISTANCE: f32 = 512.0;
const DEFAULT_NEAR_DDA_MAX_STEPS: u32 = 1024;
const MAX_NEAR_DDA_MAX_STEPS: u32 = 4096;
const DEFAULT_HEIGHT_LOD_BLEND_START: f32 = 125.0;
const DEFAULT_HEIGHT_LOD_BLEND_END: f32 = 300.0;
const DEFAULT_NORMAL_DETAIL_BLEND_START: f32 = 500.0;
const DEFAULT_NORMAL_DETAIL_BLEND_END: f32 = 1000.0;
const DEFAULT_PERFORMANCE_RENDER_SCALE: f32 = 0.5;
const DEFAULT_PRESENT_MODE: AppPresentMode = AppPresentMode::Vsync;
const DEFAULT_MAX_FRAMERATE: f32 = 0.0;
const DEFAULT_RENDER_DEBUG_VISUALS: bool = false;
const PLAYER_EYE_HEIGHT: f32 = 1.0;
const PLAYER_MOVE_SPEED: f32 = 5.0;
const PLAYER_MIN_EYE_HEIGHT: f32 = 1.0;
const PLAYER_MAX_EYE_HEIGHT: f32 = 120.0;
const PLAYER_EYE_HEIGHT_SCROLL_STEP: f32 = 0.5;
const PLAYER_MIN_MOVE_SPEED: f32 = 5.0;
const PLAYER_MAX_MOVE_SPEED: f32 = 500.0;
const PLAYER_MOVE_SPEED_SCROLL_STEP: f32 = 1.0;
const PLAYER_GRAVITY: f32 = 240.0;
const PLAYER_JUMP_SPEED: f32 = 105.0;
const PLAYER_MAX_FALL_SPEED: f32 = 260.0;
const PLAYER_GROUND_SNAP: f32 = 8.0;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct ShaderParams {
    camera: [f32; 4],
    render: [f32; 4],
    terrain: [f32; 4],
    height_maps: [f32; 4],
    source_maps: [f32; 4],
    tile_info: [f32; 4],
    tile_window: [f32; 4],
    lod_distances: [f32; 4],
    raymarch: [f32; 4],
    near_dda: [f32; 4],
    debug: [f32; 4],
    ray_forward: [f32; 4],
    ray_right: [f32; 4],
    ray_up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct UpscaleParams {
    overlay: [f32; 4],
}

struct Camera {
    x: f32,
    y: f32,
    height: f32,
    yaw: f32,
    pitch: f32,
    vertical_fov: f32,
    max_distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraSample {
    frame: u64,
    x: f32,
    y: f32,
    height: f32,
    yaw: f32,
    pitch: f32,
}

impl CameraSample {
    fn from_camera(frame: u64, camera: &Camera) -> Self {
        Self {
            frame,
            x: camera.x,
            y: camera.y,
            height: camera.height,
            yaw: camera.yaw,
            pitch: camera.pitch,
        }
    }

    fn apply_to_camera(self, camera: &mut Camera) {
        camera.x = self.x;
        camera.y = self.y;
        camera.height = self.height;
        camera.yaw = self.yaw;
        camera.pitch = self.pitch;
    }

    fn interpolate(a: Self, b: Self, frame: u64) -> Self {
        let frame_delta = (b.frame - a.frame) as f32;
        let t = if frame_delta > 0.0 {
            (frame - a.frame) as f32 / frame_delta
        } else {
            0.0
        };

        Self {
            frame,
            x: lerp(a.x, b.x, t),
            y: lerp(a.y, b.y, t),
            height: lerp(a.height, b.height, t),
            yaw: lerp(a.yaw, b.yaw, t),
            pitch: lerp(a.pitch, b.pitch, t),
        }
    }
}

#[derive(Debug)]
struct CameraTrace {
    samples: Vec<CameraSample>,
}

impl CameraTrace {
    fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        Self::parse(&contents).map_err(|error| format!("{}: {error}", path.display()).into())
    }

    fn parse(contents: &str) -> Result<Self, String> {
        let mut samples: Vec<CameraSample> = Vec::new();

        for (line_index, raw_line) in contents.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line
                .split_once('#')
                .map_or(raw_line, |(before_comment, _)| before_comment)
                .trim();

            if line.is_empty() {
                continue;
            }

            let sample = parse_camera_sample(line, line_number)?;
            if let Some(previous) = samples.last() {
                if sample.frame <= previous.frame {
                    return Err(format!(
                        "line {line_number}: frame values must be strictly increasing"
                    ));
                }
            }
            samples.push(sample);
        }

        if samples.len() < 2 {
            return Err("camera trace must contain at least two samples".to_owned());
        }

        Ok(Self { samples })
    }

    fn first_frame(&self) -> u64 {
        self.samples[0].frame
    }

    fn last_frame(&self) -> u64 {
        self.samples[self.samples.len() - 1].frame
    }

    fn sample_at_frame(&self, frame: u64) -> Option<CameraSample> {
        if frame > self.last_frame() {
            return None;
        }
        if frame <= self.first_frame() {
            return Some(self.samples[0]);
        }

        self.samples.windows(2).find_map(|samples| {
            let a = samples[0];
            let b = samples[1];
            if frame <= b.frame {
                Some(CameraSample::interpolate(a, b, frame))
            } else {
                None
            }
        })
    }
}

struct CameraReplay {
    trace: CameraTrace,
    frame: u64,
}

impl CameraReplay {
    fn new(trace: CameraTrace) -> Self {
        let frame = trace.first_frame();
        Self { trace, frame }
    }

    fn current_frame(&self) -> u64 {
        self.frame
    }

    fn apply_to_camera(&self, camera: &mut Camera) -> bool {
        if let Some(sample) = self.trace.sample_at_frame(self.frame) {
            sample.apply_to_camera(camera);
            true
        } else {
            false
        }
    }

    fn advance_after_submitted_frame(&mut self) -> bool {
        if self.frame >= self.trace.last_frame() {
            false
        } else {
            self.frame += 1;
            true
        }
    }
}

struct CameraRecordingSummary {
    path: PathBuf,
    sample_count: u64,
}

struct CameraRecorder {
    writer: BufWriter<File>,
    path: PathBuf,
    frame: u64,
    sample_count: u64,
}

impl CameraRecorder {
    fn start(camera: &Camera) -> Result<Self, Box<dyn Error>> {
        let (path, file) = create_camera_recording_file()?;
        let mut recorder = Self {
            writer: BufWriter::new(file),
            path,
            frame: 0,
            sample_count: 0,
        };

        writeln!(recorder.writer, "# tungsten camera trace v1")?;
        writeln!(recorder.writer, "# frame\tx\ty\theight\tyaw\tpitch")?;
        recorder.write_sample(camera)?;

        println!("camera recording started: {}", recorder.path.display());
        Ok(recorder)
    }

    fn update_after_submitted_frame(&mut self, camera: &Camera) -> Result<(), Box<dyn Error>> {
        self.frame += 1;
        if self.frame % CAMERA_RECORDING_INTERVAL_FRAMES == 0 {
            self.write_sample(camera)?;
        }

        Ok(())
    }

    fn finish(mut self) -> Result<CameraRecordingSummary, Box<dyn Error>> {
        self.writer.flush()?;
        Ok(CameraRecordingSummary {
            path: self.path,
            sample_count: self.sample_count,
        })
    }

    fn write_sample(&mut self, camera: &Camera) -> Result<(), Box<dyn Error>> {
        let sample = CameraSample::from_camera(self.frame, camera);
        writeln!(
            self.writer,
            "{}\t{:.9}\t{:.9}\t{:.9}\t{:.9}\t{:.9}",
            sample.frame, sample.x, sample.y, sample.height, sample.yaw, sample.pitch
        )?;
        self.sample_count += 1;

        Ok(())
    }
}

struct ReplayStats {
    submitted_frame_count: u64,
    frame_count: u64,
    total: Duration,
    min: Option<Duration>,
    max: Option<Duration>,
    buckets: Vec<ReplayStatsBucket>,
}

struct ReplayStatsBucket {
    replay_frame_start: u64,
    replay_frame_end: u64,
    frame_count: u64,
    total: Duration,
    min: Duration,
    max: Duration,
}

impl ReplayStatsBucket {
    fn new(replay_frame: u64, duration: Duration) -> Self {
        Self {
            replay_frame_start: replay_frame,
            replay_frame_end: replay_frame,
            frame_count: 1,
            total: duration,
            min: duration,
            max: duration,
        }
    }

    fn record_frame(&mut self, replay_frame: u64, duration: Duration) {
        self.replay_frame_end = replay_frame;
        self.frame_count += 1;
        self.total += duration;
        self.min = self.min.min(duration);
        self.max = self.max.max(duration);
    }

    fn average_fps(&self) -> f64 {
        let elapsed_seconds = self.total.as_secs_f64();
        if elapsed_seconds > 0.0 {
            self.frame_count as f64 / elapsed_seconds
        } else {
            0.0
        }
    }

    fn average_frame_ms(&self) -> f64 {
        let elapsed_seconds = self.total.as_secs_f64();
        if self.frame_count > 0 {
            elapsed_seconds * 1000.0 / self.frame_count as f64
        } else {
            0.0
        }
    }
}

impl ReplayStats {
    fn new() -> Self {
        Self {
            submitted_frame_count: 0,
            frame_count: 0,
            total: Duration::ZERO,
            min: None,
            max: None,
            buckets: Vec::new(),
        }
    }

    fn record_frame(&mut self, replay_frame: u64, duration: Duration) {
        self.submitted_frame_count += 1;
        if self.submitted_frame_count > REPLAY_SUMMARY_WARMUP_FRAMES {
            self.frame_count += 1;
            self.total += duration;
            self.min = Some(self.min.map_or(duration, |min| min.min(duration)));
            self.max = Some(self.max.map_or(duration, |max| max.max(duration)));
        }

        if let Some(bucket) = self.buckets.last_mut() {
            if bucket.frame_count < REPLAY_STATS_BUCKET_FRAMES {
                bucket.record_frame(replay_frame, duration);
                return;
            }
        }

        self.buckets
            .push(ReplayStatsBucket::new(replay_frame, duration));
    }

    fn ignored_summary_frame_count(&self) -> u64 {
        self.submitted_frame_count.min(REPLAY_SUMMARY_WARMUP_FRAMES)
    }
}

fn parse_camera_sample(line: &str, line_number: usize) -> Result<CameraSample, String> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() != 6 {
        return Err(format!(
            "line {line_number}: expected 6 fields: frame x y height yaw pitch"
        ));
    }

    let frame = fields[0]
        .parse()
        .map_err(|_| format!("line {line_number}: `frame` must be an unsigned integer"))?;

    Ok(CameraSample {
        frame,
        x: parse_camera_sample_f32(fields[1], "x", line_number)?,
        y: parse_camera_sample_f32(fields[2], "y", line_number)?,
        height: parse_camera_sample_f32(fields[3], "height", line_number)?,
        yaw: parse_camera_sample_f32(fields[4], "yaw", line_number)?,
        pitch: parse_camera_sample_f32(fields[5], "pitch", line_number)?,
    })
}

fn parse_camera_sample_f32(value: &str, name: &str, line_number: usize) -> Result<f32, String> {
    let parsed: f32 = value
        .parse()
        .map_err(|_| format!("line {line_number}: `{name}` must be a number"))?;

    if !parsed.is_finite() {
        return Err(format!("line {line_number}: `{name}` must be finite"));
    }

    Ok(parsed)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn create_camera_recording_file() -> Result<(PathBuf, File), Box<dyn Error>> {
    fs::create_dir_all(CAMERA_RECORDING_DIR)?;
    let timestamp_millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    for attempt in 0..1000 {
        let file_name = if attempt == 0 {
            format!("camera-{timestamp_millis}.tsv")
        } else {
            format!("camera-{timestamp_millis}-{attempt}.tsv")
        };
        let path = Path::new(CAMERA_RECORDING_DIR).join(file_name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("failed to create {}: {error}", path.display()).into());
            }
        }
    }

    Err("failed to create a unique camera recording path".into())
}

fn toggle_camera_recording(
    recorder: &mut Option<CameraRecorder>,
    camera: &Camera,
) -> Result<(), Box<dyn Error>> {
    if let Some(active_recorder) = recorder.take() {
        print_camera_recording_summary(active_recorder.finish()?);
    } else {
        *recorder = Some(CameraRecorder::start(camera)?);
    }

    Ok(())
}

fn print_camera_recording_summary(summary: CameraRecordingSummary) {
    println!(
        "camera recording saved: {} ({} samples)",
        summary.path.display(),
        summary.sample_count
    );
}

fn print_replay_stats(stats: &ReplayStats) {
    let elapsed_seconds = stats.total.as_secs_f64();
    let average_fps = if elapsed_seconds > 0.0 {
        stats.frame_count as f64 / elapsed_seconds
    } else {
        0.0
    };
    let min_frame = stats.min.unwrap_or(Duration::ZERO);
    let max_frame = stats.max.unwrap_or(Duration::ZERO);
    let average_frame_ms = if stats.frame_count > 0 {
        elapsed_seconds * 1000.0 / stats.frame_count as f64
    } else {
        0.0
    };

    println!("replay complete");
    println!("frames: {}", stats.frame_count);
    println!(
        "warmup_frames_ignored: {}",
        stats.ignored_summary_frame_count()
    );
    println!("elapsed_seconds: {:.6}", elapsed_seconds);
    println!("average_fps: {:.3}", average_fps);
    println!("min_fps: {:.3}", fps_from_frame_duration(max_frame));
    println!("max_fps: {:.3}", fps_from_frame_duration(min_frame));
    println!("frame_ms_min: {:.3}", frame_duration_ms(min_frame));
    println!("frame_ms_avg: {:.3}", average_frame_ms);
    println!("frame_ms_max: {:.3}", frame_duration_ms(max_frame));
}

fn write_replay_stats_csv(stats: &ReplayStats) -> Result<PathBuf, Box<dyn Error>> {
    let (path, file) = create_replay_stats_file()?;
    let mut writer = BufWriter::new(file);

    writeln!(
        writer,
        "bucket_index,replay_frame_start,replay_frame_end,frames,elapsed_ms,average_fps,min_fps,max_fps,frame_ms_min,frame_ms_avg,frame_ms_max"
    )?;

    for (bucket_index, bucket) in stats.buckets.iter().enumerate() {
        writeln!(
            writer,
            "{},{},{},{},{:.6},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3}",
            bucket_index,
            bucket.replay_frame_start,
            bucket.replay_frame_end,
            bucket.frame_count,
            frame_duration_ms(bucket.total),
            bucket.average_fps(),
            fps_from_frame_duration(bucket.max),
            fps_from_frame_duration(bucket.min),
            frame_duration_ms(bucket.min),
            bucket.average_frame_ms(),
            frame_duration_ms(bucket.max)
        )?;
    }

    writer.flush()?;
    Ok(path)
}

fn create_replay_stats_file() -> Result<(PathBuf, File), Box<dyn Error>> {
    let timestamp_millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();

    for attempt in 0..1000 {
        let file_name = if attempt == 0 {
            format!("tungsten-replay-fps-{timestamp_millis}.csv")
        } else {
            format!("tungsten-replay-fps-{timestamp_millis}-{attempt}.csv")
        };
        let path = Path::new("/tmp").join(file_name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!("failed to create {}: {error}", path.display()).into());
            }
        }
    }

    Err("failed to create a unique replay FPS stats path".into())
}

fn fps_from_frame_duration(duration: Duration) -> f64 {
    let seconds = duration.as_secs_f64();
    if seconds > 0.0 { 1.0 / seconds } else { 0.0 }
}

fn frame_duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

struct TerrainMaps {
    color_near: Texture<'static>,
    color_far: Texture<'static>,
    height_near_atlas: Texture<'static>,
    height_far: Texture<'static>,
    color_sampler: Sampler,
    height_sampler: Sampler,
    terrain_size: [f32; 2],
    source_size: [f32; 2],
    height_near_atlas_size: [f32; 2],
    height_far_size: [f32; 2],
    color_far_size: [f32; 2],
    tile_size: u32,
    tile_padding: u32,
    stored_tile_size: u32,
    tile_cache_width: u32,
    current_window_min: [u32; 2],
    current_window_max: [u32; 2],
    height_scale: f32,
    manifest: WorldmapManifest,
    worldmap_dir: PathBuf,
    tile_cache: TerrainTileCache,
}

impl TerrainMaps {
    fn collision_height(&self) -> &HeightField {
        &self.tile_cache.collision_height
    }

    fn update_tile_cache_for_position(
        &mut self,
        gpu: &Device,
        world_x: f32,
        world_y: f32,
    ) -> Result<(), Box<dyn Error>> {
        let uploads = self.update_tile_window_for_position(gpu, world_x, world_y)?;
        if uploads == 0 {
            return Ok(());
        }

        Ok(())
    }

    fn update_tile_window_for_position(
        &mut self,
        gpu: &Device,
        world_x: f32,
        world_y: f32,
    ) -> Result<usize, Box<dyn Error>> {
        let [center_x, center_y] = self.tile_for_world_pos(world_x, world_y);
        let (window_min, window_max) = self.tile_window_bounds_for_center(center_x, center_y);

        if self.current_window_min != window_min || self.current_window_max != window_max {
            self.tile_cache
                .collision_height
                .invalidate_tiles_outside(window_min, window_max);
            self.current_window_min = window_min;
            self.current_window_max = window_max;
        }

        let missing_tiles =
            self.missing_tiles_by_priority(center_x, center_y, window_min, window_max);

        if missing_tiles.is_empty() {
            return Ok(0);
        }

        let upload_count = missing_tiles.len();
        let command_buffer = gpu.acquire_command_buffer()?;
        let copy_pass = gpu.begin_copy_pass(&command_buffer)?;
        for [tile_x, tile_y] in missing_tiles {
            self.upload_tile_with_copy_pass(gpu, &copy_pass, tile_x, tile_y)?;
        }
        gpu.end_copy_pass(copy_pass);
        command_buffer.submit()?;

        Ok(upload_count)
    }

    fn upload_tile_with_copy_pass(
        &mut self,
        gpu: &Device,
        copy_pass: &sdl3::gpu::CopyPass,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<(), Box<dyn Error>> {
        let expected_height_bytes =
            checked_square_texture_bytes(self.stored_tile_size, R16_BYTES_PER_PIXEL)?;
        let expected_color_bytes =
            checked_square_texture_bytes(self.stored_tile_size, RGBA_BYTES_PER_PIXEL)?;
        let height_path = self
            .manifest
            .height_tile_path(&self.worldmap_dir, tile_x, tile_y);
        let color_path = self
            .manifest
            .color_tile_path(&self.worldmap_dir, tile_x, tile_y);
        let height_bytes = read_exact_bytes(&height_path, expected_height_bytes)?;
        let color_bytes = read_exact_bytes(&color_path, expected_color_bytes)?;
        let slot_index = slot_index_for_ring_tile(tile_x, tile_y, self.tile_cache_width);
        let [slot_x, slot_y] = slot_xy_for_ring_tile(tile_x, tile_y, self.tile_cache_width);
        let atlas_x = slot_x * self.tile_size;
        let atlas_y = slot_y * self.tile_size;

        upload_padded_tile_payload(
            gpu,
            copy_pass,
            &self.height_near_atlas,
            atlas_x,
            atlas_y,
            self.tile_size,
            self.stored_tile_size,
            self.tile_padding,
            R16_BYTES_PER_PIXEL,
            &height_bytes,
        )?;
        upload_padded_tile_payload(
            gpu,
            copy_pass,
            &self.color_near,
            atlas_x,
            atlas_y,
            self.tile_size,
            self.stored_tile_size,
            self.tile_padding,
            RGBA_BYTES_PER_PIXEL,
            &color_bytes,
        )?;
        self.tile_cache
            .collision_height
            .update_slot(slot_index, tile_x, tile_y, &height_bytes)?;

        Ok(())
    }

    fn tile_window_bounds_for_center(&self, center_x: u32, center_y: u32) -> ([u32; 2], [u32; 2]) {
        let radius = (self.tile_cache_width - 1) / 2;
        let min_x = center_x.saturating_sub(radius);
        let min_y = center_y.saturating_sub(radius);
        let max_x = (center_x + radius).min(self.manifest.tile_count_x - 1);
        let max_y = (center_y + radius).min(self.manifest.tile_count_y - 1);

        ([min_x, min_y], [max_x, max_y])
    }

    fn missing_tiles_by_priority(
        &self,
        center_x: u32,
        center_y: u32,
        window_min: [u32; 2],
        window_max: [u32; 2],
    ) -> Vec<[u32; 2]> {
        let mut missing_tiles = Vec::new();
        for tile_y in window_min[1]..=window_max[1] {
            for tile_x in window_min[0]..=window_max[0] {
                let slot_index = slot_index_for_ring_tile(tile_x, tile_y, self.tile_cache_width);
                if !self.tile_cache.collision_height.slots[slot_index]
                    .is_loaded_tile(tile_x, tile_y)
                {
                    missing_tiles.push([tile_x, tile_y]);
                }
            }
        }

        missing_tiles.sort_by_key(|[tile_x, tile_y]| {
            center_x.abs_diff(*tile_x) + center_y.abs_diff(*tile_y)
        });
        missing_tiles
    }

    fn tile_for_world_pos(&self, world_x: f32, world_y: f32) -> [u32; 2] {
        let source_x = (world_x / self.terrain_size[0] * self.manifest.source_width as f32)
            .clamp(0.0, (self.manifest.source_width - 1) as f32);
        let source_y = (world_y / self.terrain_size[1] * self.manifest.source_height as f32)
            .clamp(0.0, (self.manifest.source_height - 1) as f32);

        [
            (source_x as u32 / self.tile_size).min(self.manifest.tile_count_x - 1),
            (source_y as u32 / self.tile_size).min(self.manifest.tile_count_y - 1),
        ]
    }
}

fn slot_xy_for_ring_tile(tile_x: u32, tile_y: u32, tile_cache_width: u32) -> [u32; 2] {
    [tile_x % tile_cache_width, tile_y % tile_cache_width]
}

fn slot_index_for_ring_tile(tile_x: u32, tile_y: u32, tile_cache_width: u32) -> usize {
    let [slot_x, slot_y] = slot_xy_for_ring_tile(tile_x, tile_y, tile_cache_width);
    (slot_y * tile_cache_width + slot_x) as usize
}

struct RenderTarget {
    texture: Texture<'static>,
    width: u32,
    height: u32,
}

struct HeightField {
    slots: Vec<TerrainTileSlot>,
    tile_size: u32,
    tile_padding: u32,
    stored_tile_size: u32,
    source_size: [u32; 2],
    terrain_size: [f32; 2],
    height_scale: f32,
}

impl HeightField {
    fn new(manifest: &WorldmapManifest, tile_cache_width: u32) -> Result<Self, Box<dyn Error>> {
        let stored_tile_size = manifest.stored_tile_size()?;
        let slot_count = checked_tile_slot_count(tile_cache_width)?;

        Ok(Self {
            slots: vec![TerrainTileSlot::empty(); slot_count],
            tile_size: manifest.tile_size,
            tile_padding: manifest.tile_padding,
            stored_tile_size,
            source_size: [manifest.source_width, manifest.source_height],
            terrain_size: manifest.terrain_size(),
            height_scale: manifest.height_scale,
        })
    }

    fn height_at(&self, world_x: f32, world_y: f32) -> f32 {
        let sample_x = (world_x / self.terrain_size[0] * self.source_size[0] as f32)
            .clamp(0.0, (self.source_size[0] - 1) as f32);
        let sample_y = (world_y / self.terrain_size[1] * self.source_size[1] as f32)
            .clamp(0.0, (self.source_size[1] - 1) as f32);
        let x0 = sample_x.floor() as u32;
        let y0 = sample_y.floor() as u32;
        let x1 = (x0 + 1).min(self.source_size[0] - 1);
        let y1 = (y0 + 1).min(self.source_size[1] - 1);
        let tx = sample_x - x0 as f32;
        let ty = sample_y - y0 as f32;

        let h00 = self.sample_height(x0, y0);
        let h10 = self.sample_height(x1, y0);
        let h01 = self.sample_height(x0, y1);
        let h11 = self.sample_height(x1, y1);
        let h0 = h00 + (h10 - h00) * tx;
        let h1 = h01 + (h11 - h01) * tx;

        h0 + (h1 - h0) * ty
    }

    fn sample_height(&self, x: u32, y: u32) -> f32 {
        self.sample_raw_height(x, y)
            .map(|height| height as f32 / u16::MAX as f32 * self.height_scale)
            .unwrap_or(0.0)
    }

    fn sample_raw_height(&self, x: u32, y: u32) -> Option<u16> {
        let tile_x = x / self.tile_size;
        let tile_y = y / self.tile_size;
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.is_loaded_tile(tile_x, tile_y))?;

        let local_x = x - tile_x * self.tile_size + self.tile_padding;
        let local_y = y - tile_y * self.tile_size + self.tile_padding;
        let index = local_y as usize * self.stored_tile_size as usize + local_x as usize;
        slot.height_samples.get(index).copied()
    }

    fn update_slot(
        &mut self,
        slot_index: usize,
        tile_x: u32,
        tile_y: u32,
        height_bytes: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        let expected_bytes =
            self.stored_tile_size as usize * self.stored_tile_size as usize * R16_BYTES_PER_PIXEL;
        if height_bytes.len() != expected_bytes {
            return Err(format!(
                "height tile {tile_x},{tile_y} has {} bytes, expected {expected_bytes}",
                height_bytes.len()
            )
            .into());
        }

        let slot = self
            .slots
            .get_mut(slot_index)
            .ok_or_else(|| format!("tile slot {slot_index} is out of range"))?;
        slot.world_tile_x = tile_x as i32;
        slot.world_tile_y = tile_y as i32;
        slot.loaded = true;
        slot.height_samples.clear();
        slot.height_samples.extend(
            height_bytes
                .chunks_exact(2)
                .map(|sample| u16::from_le_bytes([sample[0], sample[1]])),
        );

        Ok(())
    }

    fn invalidate_tiles_outside(&mut self, window_min: [u32; 2], window_max: [u32; 2]) {
        for slot in &mut self.slots {
            if !slot.loaded {
                continue;
            }
            if slot.world_tile_x < window_min[0] as i32
                || slot.world_tile_y < window_min[1] as i32
                || slot.world_tile_x > window_max[0] as i32
                || slot.world_tile_y > window_max[1] as i32
            {
                slot.loaded = false;
            }
        }
    }
}

#[derive(Clone)]
struct TerrainTileSlot {
    world_tile_x: i32,
    world_tile_y: i32,
    loaded: bool,
    height_samples: Vec<u16>,
}

impl TerrainTileSlot {
    fn empty() -> Self {
        Self {
            world_tile_x: -1,
            world_tile_y: -1,
            loaded: false,
            height_samples: Vec::new(),
        }
    }

    fn is_loaded_tile(&self, tile_x: u32, tile_y: u32) -> bool {
        self.loaded && self.world_tile_x == tile_x as i32 && self.world_tile_y == tile_y as i32
    }
}

struct TerrainTileCache {
    collision_height: HeightField,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CameraMode {
    Freecam,
    Gravity,
}

struct PlayerPhysics {
    vertical_velocity: f32,
    on_ground: bool,
    eye_height: f32,
    move_speed: f32,
}

impl PlayerPhysics {
    fn new() -> Self {
        Self {
            vertical_velocity: 0.0,
            on_ground: false,
            eye_height: PLAYER_EYE_HEIGHT,
            move_speed: PLAYER_MOVE_SPEED,
        }
    }
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
}

#[derive(Clone, Debug, PartialEq)]
struct AppConfig {
    worldmap: PathBuf,
    tile_cache_radius: u32,
    ray_iteration_count: u32,
    performance_render_scale: f32,
    present_mode: AppPresentMode,
    max_framerate: f32,
    render_debug_visuals: bool,
    near_dda_distance: f32,
    near_dda_max_steps: u32,
    start_x: f32,
    start_y: f32,
    start_height: f32,
    normal_detail_blend_start: f32,
    normal_detail_blend_end: f32,
    height_lod_blend_start: f32,
    height_lod_blend_end: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppPresentMode {
    Vsync,
    Immediate,
    Mailbox,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DebugVisualMode {
    None,
    HeightSources,
    HitMethods,
    NormalLighting,
}

impl DebugVisualMode {
    fn next(self) -> Self {
        match self {
            Self::None => Self::HeightSources,
            Self::HeightSources => Self::HitMethods,
            Self::HitMethods => Self::NormalLighting,
            Self::NormalLighting => Self::None,
        }
    }

    fn as_shader_value(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::HeightSources => 1.0,
            Self::HitMethods => 2.0,
            Self::NormalLighting => 3.0,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::HeightSources => "height sources",
            Self::HitMethods => "ray/hit methods",
            Self::NormalLighting => "normal lighting",
        }
    }

    fn color_key(self) -> &'static str {
        match self {
            Self::None => "  no debug colors",
            Self::HeightSources => {
                "  blue: resident near height tile\n  purple: near/far height blend\n  orange: far max-height map\n  red/orange: far 2D backdrop"
            }
            Self::HitMethods => {
                "  green: resident near-tile DDA hit\n  cyan: main raymarch hit\n  yellow: large-step probe hit\n  magenta: far 2D backdrop hit"
            }
            Self::NormalLighting => {
                "  green: detailed sampled normals\n  yellow: detailed-to-flat lighting blend\n  red: flat far terrain light"
            }
        }
    }
}

impl AppPresentMode {
    fn to_sdl(self) -> PresentMode {
        match self {
            Self::Vsync => PresentMode::Vsync,
            Self::Immediate => PresentMode::Immediate,
            Self::Mailbox => PresentMode::Mailbox,
        }
    }

    fn as_config_value(self) -> &'static str {
        match self {
            Self::Vsync => "vsync",
            Self::Immediate => "immediate",
            Self::Mailbox => "mailbox",
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            worldmap: PathBuf::from(DEFAULT_WORLDMAP_PATH),
            tile_cache_radius: DEFAULT_TILE_CACHE_RADIUS,
            ray_iteration_count: DEFAULT_RAY_ITERATION_COUNT,
            performance_render_scale: DEFAULT_PERFORMANCE_RENDER_SCALE,
            present_mode: DEFAULT_PRESENT_MODE,
            max_framerate: DEFAULT_MAX_FRAMERATE,
            render_debug_visuals: DEFAULT_RENDER_DEBUG_VISUALS,
            near_dda_distance: DEFAULT_NEAR_DDA_DISTANCE,
            near_dda_max_steps: DEFAULT_NEAR_DDA_MAX_STEPS,
            start_x: DEFAULT_START_X,
            start_y: DEFAULT_START_Y,
            start_height: DEFAULT_START_HEIGHT,
            normal_detail_blend_start: DEFAULT_NORMAL_DETAIL_BLEND_START,
            normal_detail_blend_end: DEFAULT_NORMAL_DETAIL_BLEND_END,
            height_lod_blend_start: DEFAULT_HEIGHT_LOD_BLEND_START,
            height_lod_blend_end: DEFAULT_HEIGHT_LOD_BLEND_END,
        }
    }
}

impl AppConfig {
    fn load(path: &str) -> Result<Self, Box<dyn Error>> {
        match fs::read_to_string(path) {
            Ok(contents) => {
                Self::parse(&contents).map_err(|error| format!("{path}: {error}").into())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("failed to read {path}: {error}").into()),
        }
    }

    fn parse(contents: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut seen_keys = Vec::new();

        for (line_index, raw_line) in contents.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line
                .split_once('#')
                .map_or(raw_line, |(before_comment, _)| before_comment)
                .trim();

            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                return Err(format!(
                    "line {line_number}: tables are not supported; use flat `key = value` entries"
                ));
            }

            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {line_number}: expected `key = value`"))?;
            let key = key.trim();
            let value = value.trim();

            if key.is_empty() {
                return Err(format!("line {line_number}: config key is empty"));
            }
            if value.is_empty() {
                return Err(format!("line {line_number}: value for `{key}` is empty"));
            }
            if seen_keys.iter().any(|seen_key| seen_key == key) {
                return Err(format!("line {line_number}: duplicate config key `{key}`"));
            }
            seen_keys.push(key.to_owned());

            match key {
                "worldmap" => {
                    config.worldmap = PathBuf::from(parse_config_string(key, value, line_number)?)
                }
                "tile_cache_radius" => {
                    config.tile_cache_radius = parse_config_u32(key, value, line_number)?
                }
                "ray_iteration_count" => {
                    config.ray_iteration_count = parse_config_u32(key, value, line_number)?
                }
                "performance_render_scale" => {
                    config.performance_render_scale = parse_config_f32(key, value, line_number)?
                }
                "present_mode" => {
                    config.present_mode = parse_present_mode_config(value, line_number)?
                }
                "max_framerate" => {
                    config.max_framerate = parse_config_f32(key, value, line_number)?
                }
                "render_debug_visuals" => {
                    config.render_debug_visuals = parse_config_bool(key, value, line_number)?
                }
                "near_dda_distance" => {
                    config.near_dda_distance = parse_config_f32(key, value, line_number)?
                }
                "near_dda_max_steps" => {
                    config.near_dda_max_steps = parse_config_u32(key, value, line_number)?
                }
                "start_x" => config.start_x = parse_config_f32(key, value, line_number)?,
                "start_y" => config.start_y = parse_config_f32(key, value, line_number)?,
                "start_height" => config.start_height = parse_config_f32(key, value, line_number)?,
                "normal_detail_blend_start" => {
                    config.normal_detail_blend_start = parse_config_f32(key, value, line_number)?
                }
                "normal_detail_blend_end" => {
                    config.normal_detail_blend_end = parse_config_f32(key, value, line_number)?
                }
                "height_lod_blend_start" => {
                    config.height_lod_blend_start = parse_config_f32(key, value, line_number)?
                }
                "height_lod_blend_end" => {
                    config.height_lod_blend_end = parse_config_f32(key, value, line_number)?
                }
                _ => return Err(format!("line {line_number}: unknown config key `{key}`")),
            }
        }

        config.validate()
    }

    fn validate(self) -> Result<Self, String> {
        if self.worldmap.as_os_str().is_empty() {
            return Err("`worldmap` must not be empty".to_owned());
        }
        if self.tile_cache_radius == 0 || self.tile_cache_radius > MAX_TILE_CACHE_RADIUS {
            return Err(format!(
                "`tile_cache_radius` must be between 1 and {MAX_TILE_CACHE_RADIUS}"
            ));
        }
        if !(1..=MAX_RAY_ITERATION_COUNT).contains(&self.ray_iteration_count) {
            return Err(format!(
                "`ray_iteration_count` must be between 1 and {MAX_RAY_ITERATION_COUNT}"
            ));
        }
        if !(self.performance_render_scale > 0.0 && self.performance_render_scale <= 1.0) {
            return Err(
                "`performance_render_scale` must be greater than 0.0 and no more than 1.0"
                    .to_owned(),
            );
        }
        if self.max_framerate < 0.0 {
            return Err("`max_framerate` must be non-negative; use 0.0 for unlimited".to_owned());
        }
        if self.near_dda_distance <= 0.0 {
            return Err("`near_dda_distance` must be greater than 0.0".to_owned());
        }
        if !(1..=MAX_NEAR_DDA_MAX_STEPS).contains(&self.near_dda_max_steps) {
            return Err(format!(
                "`near_dda_max_steps` must be between 1 and {MAX_NEAR_DDA_MAX_STEPS}"
            ));
        }
        if self.start_x < 0.0 || self.start_y < 0.0 || self.start_height < 0.0 {
            return Err("`start_x`, `start_y`, and `start_height` must be non-negative".to_owned());
        }
        validate_blend_range(
            "height_lod",
            self.height_lod_blend_start,
            self.height_lod_blend_end,
        )?;
        validate_blend_range(
            "normal_detail",
            self.normal_detail_blend_start,
            self.normal_detail_blend_end,
        )?;

        Ok(self)
    }
}

fn parse_config_u32(key: &str, value: &str, line_number: usize) -> Result<u32, String> {
    let normalized = value.replace('_', "");
    normalized
        .parse()
        .map_err(|_| format!("line {line_number}: `{key}` must be an unsigned integer"))
}

fn parse_config_f32(key: &str, value: &str, line_number: usize) -> Result<f32, String> {
    let normalized = value.replace('_', "");
    let parsed: f32 = normalized
        .parse()
        .map_err(|_| format!("line {line_number}: `{key}` must be a number"))?;

    if !parsed.is_finite() {
        return Err(format!("line {line_number}: `{key}` must be finite"));
    }

    Ok(parsed)
}

fn parse_config_bool(key: &str, value: &str, line_number: usize) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "line {line_number}: `{key}` must be `true` or `false`"
        )),
    }
}

fn parse_present_mode_config(value: &str, line_number: usize) -> Result<AppPresentMode, String> {
    let value = parse_config_string("present_mode", value, line_number)?;

    match value.to_ascii_lowercase().as_str() {
        "vsync" | "v-sync" => Ok(AppPresentMode::Vsync),
        "immediate" => Ok(AppPresentMode::Immediate),
        "mailbox" => Ok(AppPresentMode::Mailbox),
        _ => Err(format!(
            "line {line_number}: `present_mode` must be one of `vsync`, `immediate`, or `mailbox`"
        )),
    }
}

fn parse_config_string<'a>(
    key: &str,
    value: &'a str,
    line_number: usize,
) -> Result<&'a str, String> {
    let value = value.trim();
    let unquoted = if value.len() >= 2 {
        let bytes = value.as_bytes();
        if (bytes[0] == b'"' && bytes[value.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[value.len() - 1] == b'\'')
        {
            &value[1..value.len() - 1]
        } else {
            value
        }
    } else {
        value
    };

    if unquoted.is_empty() {
        return Err(format!("line {line_number}: `{key}` must not be empty"));
    }

    Ok(unquoted)
}

fn validate_blend_range(name: &str, start: f32, end: f32) -> Result<(), String> {
    if start < 0.0 || end < 0.0 {
        return Err(format!(
            "`{name}_blend_start` and `{name}_blend_end` must be non-negative"
        ));
    }
    if start >= end {
        return Err(format!(
            "`{name}_blend_start` must be less than `{name}_blend_end`"
        ));
    }

    Ok(())
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
    let terrain_pipeline = create_terrain_pipeline(&gpu, target_format)?;
    let upscale_pipeline = create_upscale_pipeline(&gpu, target_format)?;
    let mut terrain_maps = load_terrain_maps(&gpu, &config)?;
    let upscale_sampler = create_upscale_sampler(&gpu)?;
    let mut render_target = None;
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
    let mut camera_recorder = None;

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

        let (window_width, window_height) = window.size();
        ensure_render_target(
            &gpu,
            &mut render_target,
            target_format,
            window_width,
            window_height,
            config.performance_render_scale,
        )?;
        let render_target = render_target
            .as_ref()
            .expect("render target should be initialized before drawing");

        let mut command_buffer = gpu.acquire_command_buffer()?;
        let mut frame_submitted = false;
        let params = shader_params(
            &camera,
            &terrain_maps,
            render_target.width,
            render_target.height,
            &config,
            debug_visual_mode,
        );

        let color_targets = [ColorTargetInfo::default()
            .with_texture(&render_target.texture)
            .with_load_op(LoadOp::CLEAR)
            .with_store_op(StoreOp::STORE)
            .with_clear_color(Color::RGB(105, 136, 157))];

        let render_pass = gpu.begin_render_pass(&command_buffer, &color_targets, None)?;
        render_pass.bind_graphics_pipeline(&terrain_pipeline);
        render_pass.bind_fragment_samplers(
            0,
            &[
                TextureSamplerBinding::new()
                    .with_texture(&terrain_maps.color_near)
                    .with_sampler(&terrain_maps.color_sampler),
                TextureSamplerBinding::new()
                    .with_texture(&terrain_maps.height_near_atlas)
                    .with_sampler(&terrain_maps.height_sampler),
                TextureSamplerBinding::new()
                    .with_texture(&terrain_maps.height_far)
                    .with_sampler(&terrain_maps.height_sampler),
                TextureSamplerBinding::new()
                    .with_texture(&terrain_maps.color_far)
                    .with_sampler(&terrain_maps.color_sampler),
            ],
        );
        command_buffer.push_fragment_uniform_data(0, &params);
        render_pass.draw_primitives(3, 1, 0, 0);
        gpu.end_render_pass(render_pass);

        if let Ok(swapchain) = command_buffer.wait_and_acquire_swapchain_texture(&window) {
            let upscale_params =
                upscale_params(&fps_counter, swapchain.width(), swapchain.height());
            let color_targets = [ColorTargetInfo::default()
                .with_texture(&swapchain)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(105, 136, 157))];

            let upscale_pass = gpu.begin_render_pass(&command_buffer, &color_targets, None)?;
            upscale_pass.bind_graphics_pipeline(&upscale_pipeline);
            upscale_pass.bind_fragment_samplers(
                0,
                &[TextureSamplerBinding::new()
                    .with_texture(&render_target.texture)
                    .with_sampler(&upscale_sampler)],
            );
            command_buffer.push_fragment_uniform_data(0, &upscale_params);
            upscale_pass.draw_primitives(3, 1, 0, 0);
            gpu.end_render_pass(upscale_pass);

            command_buffer.submit()?;
            frame_submitted = true;
        } else {
            command_buffer.cancel();
        }

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

fn create_terrain_pipeline(
    gpu: &Device,
    target_format: TextureFormat,
) -> Result<GraphicsPipeline, Box<dyn Error>> {
    let vertex_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/fullscreen.vert.spv")),
            ShaderStage::Vertex,
        )
        .with_entrypoint(c"main")
        .build()?;

    let fragment_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/voxelspace.frag.spv")),
            ShaderStage::Fragment,
        )
        .with_samplers(4)
        .with_uniform_buffers(1)
        .with_entrypoint(c"main")
        .build()?;

    let pipeline = gpu
        .create_graphics_pipeline()
        .with_fragment_shader(&fragment_shader)
        .with_vertex_shader(&vertex_shader)
        .with_primitive_type(PrimitiveType::TriangleList)
        .with_fill_mode(FillMode::Fill)
        .with_target_info(
            GraphicsPipelineTargetInfo::new().with_color_target_descriptions(&[
                ColorTargetDescription::new().with_format(target_format),
            ]),
        )
        .build()?;

    Ok(pipeline)
}

fn create_upscale_pipeline(
    gpu: &Device,
    target_format: TextureFormat,
) -> Result<GraphicsPipeline, Box<dyn Error>> {
    let vertex_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/fullscreen.vert.spv")),
            ShaderStage::Vertex,
        )
        .with_entrypoint(c"main")
        .build()?;

    let fragment_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/upscale.frag.spv")),
            ShaderStage::Fragment,
        )
        .with_samplers(1)
        .with_uniform_buffers(1)
        .with_entrypoint(c"main")
        .build()?;

    let pipeline = gpu
        .create_graphics_pipeline()
        .with_fragment_shader(&fragment_shader)
        .with_vertex_shader(&vertex_shader)
        .with_primitive_type(PrimitiveType::TriangleList)
        .with_fill_mode(FillMode::Fill)
        .with_target_info(
            GraphicsPipelineTargetInfo::new().with_color_target_descriptions(&[
                ColorTargetDescription::new().with_format(target_format),
            ]),
        )
        .build()?;

    Ok(pipeline)
}

fn upscale_params(fps_counter: &FpsCounter, width: u32, height: u32) -> UpscaleParams {
    UpscaleParams {
        overlay: [
            fps_counter.displayed_fps,
            width as f32,
            height as f32,
            fps_counter.displayed_frame_ms,
        ],
    }
}

fn scaled_render_dimension(window_dimension: u32, render_scale: f32) -> u32 {
    ((window_dimension as f32 * render_scale).round() as u32).max(1)
}

fn ensure_render_target(
    gpu: &Device,
    render_target: &mut Option<RenderTarget>,
    format: TextureFormat,
    window_width: u32,
    window_height: u32,
    render_scale: f32,
) -> Result<(), Box<dyn Error>> {
    let width = scaled_render_dimension(window_width, render_scale);
    let height = scaled_render_dimension(window_height, render_scale);
    let needs_recreate = match render_target {
        Some(target) => target.width != width || target.height != height,
        None => true,
    };

    if needs_recreate {
        *render_target = Some(RenderTarget {
            texture: create_color_target_texture(gpu, width, height, format)?,
            width,
            height,
        });
    }

    Ok(())
}

fn create_color_target_texture(
    gpu: &Device,
    width: u32,
    height: u32,
    format: TextureFormat,
) -> Result<Texture<'static>, Box<dyn Error>> {
    Ok(gpu.create_texture(
        TextureCreateInfo::new()
            .with_format(format)
            .with_type(TextureType::_2D)
            .with_width(width)
            .with_height(height)
            .with_layer_count_or_depth(1)
            .with_num_levels(1)
            .with_usage(TextureUsage::COLOR_TARGET | TextureUsage::SAMPLER),
    )?)
}

fn create_upscale_sampler(gpu: &Device) -> Result<Sampler, Box<dyn Error>> {
    Ok(gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::ClampToEdge)
            .with_address_mode_v(SamplerAddressMode::ClampToEdge)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?)
}

fn load_terrain_maps(gpu: &Device, config: &AppConfig) -> Result<TerrainMaps, Box<dyn Error>> {
    let manifest = WorldmapManifest::load(&config.worldmap)?;
    let worldmap_dir = manifest_dir(&config.worldmap)?;
    let stored_tile_size = manifest.stored_tile_size()?;
    let tile_cache_width = config.tile_cache_radius * 2 + 1;
    let atlas_size = checked_atlas_size(tile_cache_width, manifest.tile_size)?;
    let terrain_size = manifest.terrain_size();
    let source_size = [manifest.source_width as f32, manifest.source_height as f32];
    let height_far_size = [
        manifest.height_far_width as f32,
        manifest.height_far_height as f32,
    ];
    let color_far_size = [
        manifest.color_far_width as f32,
        manifest.color_far_height as f32,
    ];

    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;

    let color_near = create_empty_texture(
        gpu,
        atlas_size,
        atlas_size,
        TextureFormat::R8g8b8a8Unorm,
        TextureUsage::SAMPLER,
    )?;
    let height_near_atlas = create_empty_texture(
        gpu,
        atlas_size,
        atlas_size,
        TextureFormat::R16Unorm,
        TextureUsage::SAMPLER,
    )?;
    let height_far = create_texture_from_r16(
        gpu,
        &copy_pass,
        manifest.height_far_path(&worldmap_dir),
        manifest.height_far_width,
        manifest.height_far_height,
    )?;
    let color_far_pixels = read_rgba_pixels(
        &manifest.color_far_path(&worldmap_dir),
        manifest.color_far_width,
        manifest.color_far_height,
    )?;
    let color_far = create_texture_from_bytes(
        gpu,
        &copy_pass,
        manifest.color_far_width,
        manifest.color_far_height,
        TextureFormat::R8g8b8a8Unorm,
        &color_far_pixels,
    )?;
    let color_sampler = gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::ClampToEdge)
            .with_address_mode_v(SamplerAddressMode::ClampToEdge)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?;
    let height_sampler = gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::ClampToEdge)
            .with_address_mode_v(SamplerAddressMode::ClampToEdge)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?;
    let tile_cache = TerrainTileCache {
        collision_height: HeightField::new(&manifest, tile_cache_width)?,
    };

    let mut terrain_maps = TerrainMaps {
        color_near,
        color_far,
        height_near_atlas,
        height_far,
        color_sampler,
        height_sampler,
        terrain_size,
        source_size,
        height_near_atlas_size: [atlas_size as f32, atlas_size as f32],
        height_far_size,
        color_far_size,
        tile_size: manifest.tile_size,
        tile_padding: manifest.tile_padding,
        stored_tile_size,
        tile_cache_width,
        current_window_min: [u32::MAX, u32::MAX],
        current_window_max: [u32::MAX, u32::MAX],
        height_scale: manifest.height_scale,
        manifest,
        worldmap_dir,
        tile_cache,
    };

    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;
    terrain_maps.update_tile_window_for_position(gpu, config.start_x, config.start_y)?;

    Ok(terrain_maps)
}

fn create_empty_texture(
    gpu: &Device,
    width: u32,
    height: u32,
    format: TextureFormat,
    usage: TextureUsage,
) -> Result<Texture<'static>, Box<dyn Error>> {
    Ok(gpu.create_texture(
        TextureCreateInfo::new()
            .with_format(format)
            .with_type(TextureType::_2D)
            .with_width(width)
            .with_height(height)
            .with_layer_count_or_depth(1)
            .with_num_levels(1)
            .with_usage(usage),
    )?)
}

fn create_texture_from_r16(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
) -> Result<Texture<'static>, Box<dyn Error>> {
    let pixels = read_r16_pixels(path.as_ref(), width, height)?;
    create_texture_from_bytes(
        gpu,
        copy_pass,
        width,
        height,
        TextureFormat::R16Unorm,
        &pixels,
    )
}

fn read_r16_pixels(path: &Path, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = fs::read(path)?;
    let expected_size = width as usize * height as usize * R16_BYTES_PER_PIXEL;
    if pixels.len() != expected_size {
        return Err(format!(
            "{} has {} bytes, expected {expected_size} for a {width}x{height} R16 heightmap",
            path.display(),
            pixels.len()
        )
        .into());
    }

    Ok(pixels)
}

fn read_rgba_pixels(path: &Path, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = fs::read(path)?;
    let expected_size = width as usize * height as usize * RGBA_BYTES_PER_PIXEL;
    if pixels.len() != expected_size {
        return Err(format!(
            "{} has {} bytes, expected {expected_size} for a {width}x{height} RGBA8 map",
            path.display(),
            pixels.len()
        )
        .into());
    }

    Ok(pixels)
}

fn read_exact_bytes(path: &Path, expected_size: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    if bytes.len() != expected_size {
        return Err(format!(
            "{} has {} bytes, expected {expected_size}",
            path.display(),
            bytes.len()
        )
        .into());
    }

    Ok(bytes)
}

fn create_texture_from_bytes(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    width: u32,
    height: u32,
    format: TextureFormat,
    pixels: &[u8],
) -> Result<Texture<'static>, Box<dyn Error>> {
    let size_bytes = pixels.len() as u32;

    let texture = gpu.create_texture(
        TextureCreateInfo::new()
            .with_format(format)
            .with_type(TextureType::_2D)
            .with_width(width)
            .with_height(height)
            .with_layer_count_or_depth(1)
            .with_num_levels(1)
            .with_usage(TextureUsage::SAMPLER),
    )?;

    let transfer_buffer = gpu
        .create_transfer_buffer()
        .with_size(size_bytes)
        .with_usage(TransferBufferUsage::UPLOAD)
        .build()?;

    let mut map = transfer_buffer.map::<u8>(gpu, false);
    map.mem_mut().copy_from_slice(pixels);
    map.unmap();

    copy_pass.upload_to_gpu_texture(
        TextureTransferInfo::new()
            .with_transfer_buffer(&transfer_buffer)
            .with_offset(0)
            .with_pixels_per_row(width)
            .with_rows_per_layer(height),
        TextureRegion::new()
            .with_texture(&texture)
            .with_layer(0)
            .with_width(width)
            .with_height(height)
            .with_depth(1),
        false,
    );

    Ok(texture)
}

fn upload_padded_tile_payload(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    texture: &Texture,
    x: u32,
    y: u32,
    tile_size: u32,
    stored_tile_size: u32,
    tile_padding: u32,
    bytes_per_pixel: usize,
    pixels: &[u8],
) -> Result<(), Box<dyn Error>> {
    let size_bytes = u32::try_from(pixels.len())
        .map_err(|_| "tile upload is too large for SDL transfer buffer size")?;
    let row_offset_pixels = tile_padding
        .checked_mul(stored_tile_size)
        .and_then(|offset| offset.checked_add(tile_padding))
        .ok_or("tile payload offset overflows u32")?;
    let offset = row_offset_pixels as usize * bytes_per_pixel;
    let offset = u32::try_from(offset).map_err(|_| "tile payload offset overflows u32")?;
    let transfer_buffer = gpu
        .create_transfer_buffer()
        .with_size(size_bytes)
        .with_usage(TransferBufferUsage::UPLOAD)
        .build()?;

    let mut map = transfer_buffer.map::<u8>(gpu, false);
    map.mem_mut().copy_from_slice(pixels);
    map.unmap();

    copy_pass.upload_to_gpu_texture(
        TextureTransferInfo::new()
            .with_transfer_buffer(&transfer_buffer)
            .with_offset(offset)
            .with_pixels_per_row(stored_tile_size)
            .with_rows_per_layer(stored_tile_size),
        TextureRegion::new()
            .with_texture(texture)
            .with_layer(0)
            .with_x(x)
            .with_y(y)
            .with_width(tile_size)
            .with_height(tile_size)
            .with_depth(1),
        false,
    );

    Ok(())
}

fn checked_atlas_size(tile_cache_width: u32, stored_tile_size: u32) -> Result<u32, Box<dyn Error>> {
    tile_cache_width
        .checked_mul(stored_tile_size)
        .ok_or_else(|| "tile atlas dimensions overflow u32".into())
}

fn checked_tile_slot_count(tile_cache_width: u32) -> Result<usize, Box<dyn Error>> {
    let count = tile_cache_width
        .checked_mul(tile_cache_width)
        .ok_or("tile cache slot count overflows u32")?;
    let count = usize::try_from(count)?;

    if count > MAX_SHADER_TILE_SLOTS {
        return Err(
            format!("tile cache needs {count} slots; max is {MAX_SHADER_TILE_SLOTS}").into(),
        );
    }

    Ok(count)
}

fn checked_square_texture_bytes(
    size: u32,
    bytes_per_pixel: usize,
) -> Result<usize, Box<dyn Error>> {
    (size as usize)
        .checked_mul(size as usize)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| "square texture byte size overflows usize".into())
}

fn update_camera_look(
    events: &sdl3::EventPump,
    camera: &mut Camera,
    dt: f32,
    mouse_delta: [f32; 2],
) {
    let keyboard = events.keyboard_state();
    let turn_speed = 1.85;
    let pitch_speed = 1.35;
    let mouse_sensitivity = 0.0024;

    camera.yaw += mouse_delta[0] * mouse_sensitivity;
    camera.pitch -= mouse_delta[1] * mouse_sensitivity;

    if keyboard.is_scancode_pressed(Scancode::Q) || keyboard.is_scancode_pressed(Scancode::Left) {
        camera.yaw -= turn_speed * dt;
    }
    if keyboard.is_scancode_pressed(Scancode::E) || keyboard.is_scancode_pressed(Scancode::Right) {
        camera.yaw += turn_speed * dt;
    }
    if keyboard.is_scancode_pressed(Scancode::Up) {
        camera.pitch += pitch_speed * dt;
    }
    if keyboard.is_scancode_pressed(Scancode::Down) {
        camera.pitch -= pitch_speed * dt;
    }

    camera.pitch = camera.pitch.clamp(-1.45, 1.45);
}

fn update_freecam(events: &sdl3::EventPump, camera: &mut Camera, dt: f32, mouse_delta: [f32; 2]) {
    update_camera_look(events, camera, dt, mouse_delta);

    let keyboard = events.keyboard_state();
    let move_speed = 135.0;
    let height_speed = 80.0;

    let forward = [camera.yaw.sin(), -camera.yaw.cos()];
    let right = [camera.yaw.cos(), camera.yaw.sin()];
    let mut movement = [0.0, 0.0];

    if keyboard.is_scancode_pressed(Scancode::W) {
        movement[0] += forward[0];
        movement[1] += forward[1];
    }
    if keyboard.is_scancode_pressed(Scancode::S) {
        movement[0] -= forward[0];
        movement[1] -= forward[1];
    }
    if keyboard.is_scancode_pressed(Scancode::D) {
        movement[0] += right[0];
        movement[1] += right[1];
    }
    if keyboard.is_scancode_pressed(Scancode::A) {
        movement[0] -= right[0];
        movement[1] -= right[1];
    }

    let length = (movement[0] * movement[0] + movement[1] * movement[1]).sqrt();
    if length > 0.0 {
        camera.x += movement[0] / length * move_speed * dt;
        camera.y += movement[1] / length * move_speed * dt;
    }

    if keyboard.is_scancode_pressed(Scancode::R) {
        camera.height += height_speed * dt;
    }
    if keyboard.is_scancode_pressed(Scancode::F) {
        camera.height -= height_speed * dt;
    }

    camera.height = camera.height.clamp(20.0, 520.0);
    camera.vertical_fov = camera.vertical_fov.clamp(0.5, 1.4);
    camera.max_distance = camera.max_distance.max(120.0);
}

fn enable_gravity_mode(
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
) {
    let ground_height = player_ground_height(camera, physics, collision_height);
    if camera.height < ground_height {
        camera.height = ground_height;
    }
    physics.vertical_velocity = 0.0;
    physics.on_ground = camera.height <= ground_height + PLAYER_GROUND_SNAP;
}

fn update_gravity_camera(
    events: &sdl3::EventPump,
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
    dt: f32,
    mouse_delta: [f32; 2],
    jump_requested: bool,
) {
    update_camera_look(events, camera, dt, mouse_delta);
    update_player_horizontal_movement(events, camera, collision_height, physics.move_speed, dt);

    let ground_height = player_ground_height(camera, physics, collision_height);
    if physics.on_ground && !jump_requested {
        if camera.height < ground_height || camera.height - ground_height <= PLAYER_GROUND_SNAP {
            camera.height = ground_height;
            physics.vertical_velocity = 0.0;
            physics.on_ground = true;
        } else {
            physics.on_ground = false;
        }
    }

    if jump_requested && physics.on_ground {
        physics.vertical_velocity = PLAYER_JUMP_SPEED;
        physics.on_ground = false;
    }

    if !physics.on_ground {
        physics.vertical_velocity =
            (physics.vertical_velocity - PLAYER_GRAVITY * dt).max(-PLAYER_MAX_FALL_SPEED);
        camera.height += physics.vertical_velocity * dt;
    }

    collide_player_with_terrain(camera, physics, collision_height);
    camera.vertical_fov = camera.vertical_fov.clamp(0.5, 1.4);
    camera.max_distance = camera.max_distance.max(120.0);
}

fn terrain_full_map_distance(terrain_size: [f32; 2]) -> f32 {
    terrain_size[0].hypot(terrain_size[1])
}

fn apply_gravity_wheel_adjustment(
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
    wheel_delta: f32,
    adjust_move_speed: bool,
) {
    let changed = if adjust_move_speed {
        let previous = physics.move_speed;
        physics.move_speed = (physics.move_speed + wheel_delta * PLAYER_MOVE_SPEED_SCROLL_STEP)
            .clamp(PLAYER_MIN_MOVE_SPEED, PLAYER_MAX_MOVE_SPEED);
        physics.move_speed != previous
    } else {
        let previous = physics.eye_height;
        physics.eye_height = (physics.eye_height + wheel_delta * PLAYER_EYE_HEIGHT_SCROLL_STEP)
            .clamp(PLAYER_MIN_EYE_HEIGHT, PLAYER_MAX_EYE_HEIGHT);
        let delta = physics.eye_height - previous;
        if delta != 0.0 {
            camera.height += delta;
            let ground_height = player_ground_height(camera, physics, collision_height);
            if camera.height < ground_height || physics.on_ground {
                camera.height = ground_height;
                physics.vertical_velocity = 0.0;
                physics.on_ground = true;
            }
        }
        physics.eye_height != previous
    };

    if changed {
        println!(
            "gravity camera height: {:.1}, movement speed: {:.1}",
            physics.eye_height, physics.move_speed
        );
    }
}

fn update_player_horizontal_movement(
    events: &sdl3::EventPump,
    camera: &mut Camera,
    collision_height: &HeightField,
    move_speed: f32,
    dt: f32,
) {
    let keyboard = events.keyboard_state();
    let forward = [camera.yaw.sin(), -camera.yaw.cos()];
    let right = [camera.yaw.cos(), camera.yaw.sin()];
    let mut movement = [0.0, 0.0];

    if keyboard.is_scancode_pressed(Scancode::W) {
        movement[0] += forward[0];
        movement[1] += forward[1];
    }
    if keyboard.is_scancode_pressed(Scancode::S) {
        movement[0] -= forward[0];
        movement[1] -= forward[1];
    }
    if keyboard.is_scancode_pressed(Scancode::D) {
        movement[0] += right[0];
        movement[1] += right[1];
    }
    if keyboard.is_scancode_pressed(Scancode::A) {
        movement[0] -= right[0];
        movement[1] -= right[1];
    }

    let length = (movement[0] * movement[0] + movement[1] * movement[1]).sqrt();
    if length > 0.0 {
        camera.x += movement[0] / length * move_speed * dt;
        camera.y += movement[1] / length * move_speed * dt;
        camera.x = camera.x.clamp(0.0, collision_height.terrain_size[0]);
        camera.y = camera.y.clamp(0.0, collision_height.terrain_size[1]);
    }
}

fn collide_player_with_terrain(
    camera: &mut Camera,
    physics: &mut PlayerPhysics,
    collision_height: &HeightField,
) {
    let ground_height = player_ground_height(camera, physics, collision_height);
    if camera.height <= ground_height {
        camera.height = ground_height;
        if physics.vertical_velocity < 0.0 {
            physics.vertical_velocity = 0.0;
        }
        physics.on_ground = true;
    } else {
        physics.on_ground = false;
    }
}

fn player_ground_height(
    camera: &Camera,
    physics: &PlayerPhysics,
    collision_height: &HeightField,
) -> f32 {
    collision_height.height_at(camera.x, camera.y) + physics.eye_height
}

fn shader_params(
    camera: &Camera,
    terrain_maps: &TerrainMaps,
    width: u32,
    height: u32,
    config: &AppConfig,
    debug_visual_mode: DebugVisualMode,
) -> ShaderParams {
    let ray_basis = camera_ray_basis(camera, width, height);

    ShaderParams {
        camera: [camera.x, camera.y, camera.height, 0.0],
        render: [
            width as f32,
            height as f32,
            camera.vertical_fov,
            terrain_maps.height_scale,
        ],
        terrain: [
            terrain_maps.terrain_size[0],
            terrain_maps.terrain_size[1],
            terrain_maps.manifest.tile_count_x as f32,
            terrain_maps.manifest.tile_count_y as f32,
        ],
        height_maps: [
            terrain_maps.height_near_atlas_size[0],
            terrain_maps.height_near_atlas_size[1],
            terrain_maps.height_far_size[0],
            terrain_maps.height_far_size[1],
        ],
        source_maps: [
            terrain_maps.source_size[0],
            terrain_maps.source_size[1],
            terrain_maps.color_far_size[0],
            terrain_maps.color_far_size[1],
        ],
        tile_info: [
            terrain_maps.tile_size as f32,
            terrain_maps.tile_cache_width as f32,
            (terrain_maps.current_window_min[0] % terrain_maps.tile_cache_width) as f32,
            (terrain_maps.current_window_min[1] % terrain_maps.tile_cache_width) as f32,
        ],
        tile_window: [
            terrain_maps.current_window_min[0] as f32,
            terrain_maps.current_window_min[1] as f32,
            terrain_maps.current_window_max[0] as f32,
            terrain_maps.current_window_max[1] as f32,
        ],
        lod_distances: [
            config.height_lod_blend_start,
            config.height_lod_blend_end,
            config.normal_detail_blend_start,
            config.normal_detail_blend_end,
        ],
        raymarch: [
            camera.pitch,
            RAYMARCH_START_DISTANCE,
            camera.max_distance,
            config.ray_iteration_count as f32,
        ],
        near_dda: [
            config.near_dda_distance,
            config.near_dda_max_steps as f32,
            0.0,
            0.0,
        ],
        debug: [debug_visual_mode.as_shader_value(), 0.0, 0.0, 0.0],
        ray_forward: [
            ray_basis.forward[0],
            ray_basis.forward[1],
            ray_basis.forward[2],
            0.0,
        ],
        ray_right: [
            ray_basis.right_scaled[0],
            ray_basis.right_scaled[1],
            ray_basis.right_scaled[2],
            0.0,
        ],
        ray_up: [
            ray_basis.up_scaled[0],
            ray_basis.up_scaled[1],
            ray_basis.up_scaled[2],
            0.0,
        ],
    }
}

struct RayBasis {
    forward: [f32; 3],
    right_scaled: [f32; 3],
    up_scaled: [f32; 3],
}

fn camera_ray_basis(camera: &Camera, width: u32, height: u32) -> RayBasis {
    let sin_yaw = camera.yaw.sin();
    let cos_yaw = camera.yaw.cos();
    let sin_pitch = camera.pitch.sin();
    let cos_pitch = camera.pitch.cos();
    let forward_flat = [sin_yaw, 0.0, -cos_yaw];
    let right = [cos_yaw, 0.0, sin_yaw];
    let world_up = [0.0, 1.0, 0.0];
    let forward = normalize3(add3(
        scale3(forward_flat, cos_pitch),
        scale3(world_up, sin_pitch),
    ));
    let up = normalize3(add3(
        scale3(world_up, cos_pitch),
        scale3(forward_flat, -sin_pitch),
    ));
    let aspect = width as f32 / (height as f32).max(1.0);
    let tan_half_fov = (camera.vertical_fov * 0.5).tan();

    RayBasis {
        forward,
        right_scaled: scale3(right, aspect * tan_half_fov),
        up_scaled: scale3(up, tan_half_fov),
    }
}

fn add3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale3(v: [f32; 3], scale: f32) -> [f32; 3] {
    [v[0] * scale, v[1] * scale, v[2] * scale]
}

fn normalize3(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 0.0 {
        [v[0] / length, v[1] / length, v[2] / length]
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os_args(args: &[&str]) -> Vec<std::ffi::OsString> {
        args.iter().map(std::ffi::OsString::from).collect()
    }

    fn sample(frame: u64, x: f32, y: f32, height: f32, yaw: f32, pitch: f32) -> CameraSample {
        CameraSample {
            frame,
            x,
            y,
            height,
            yaw,
            pitch,
        }
    }

    fn assert_f32_near(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.0001,
            "expected {actual} to be near {expected}"
        );
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

    #[test]
    fn parses_camera_trace_comments_and_samples() {
        let trace = CameraTrace::parse(
            r#"
            # tungsten camera trace v1
            # frame x y height yaw pitch
            0   1.0 2.0 3.0 4.0 5.0

            10  11.0 12.0 13.0 14.0 15.0 # inline comment
            "#,
        )
        .unwrap();

        assert_eq!(trace.samples.len(), 2);
        assert_eq!(trace.samples[0], sample(0, 1.0, 2.0, 3.0, 4.0, 5.0));
        assert_eq!(trace.samples[1], sample(10, 11.0, 12.0, 13.0, 14.0, 15.0));
    }

    #[test]
    fn rejects_invalid_camera_traces() {
        let error = CameraTrace::parse("0 1.0 2.0 3.0 4.0")
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected 6 fields"));

        let error = CameraTrace::parse(
            r#"
            0 1.0 NaN 3.0 4.0 5.0
            10 1.0 2.0 3.0 4.0 5.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("finite"));

        let error = CameraTrace::parse(
            r#"
            10 1.0 2.0 3.0 4.0 5.0
            10 1.0 2.0 3.0 4.0 5.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("strictly increasing"));

        let error = CameraTrace::parse("0 1.0 2.0 3.0 4.0 5.0").unwrap_err();
        assert!(error.contains("at least two samples"));
    }

    #[test]
    fn interpolates_camera_trace_by_frame() {
        let trace = CameraTrace {
            samples: vec![
                sample(0, 10.0, 20.0, 30.0, 0.0, -1.0),
                sample(10, 20.0, 40.0, 70.0, 1.0, 0.0),
            ],
        };

        assert_eq!(
            trace.sample_at_frame(0).unwrap(),
            sample(0, 10.0, 20.0, 30.0, 0.0, -1.0)
        );

        let midpoint = trace.sample_at_frame(5).unwrap();
        assert_eq!(midpoint.frame, 5);
        assert_f32_near(midpoint.x, 15.0);
        assert_f32_near(midpoint.y, 30.0);
        assert_f32_near(midpoint.height, 50.0);
        assert_f32_near(midpoint.yaw, 0.5);
        assert_f32_near(midpoint.pitch, -0.5);

        assert_eq!(
            trace.sample_at_frame(10).unwrap(),
            sample(10, 20.0, 40.0, 70.0, 1.0, 0.0)
        );
        assert!(trace.sample_at_frame(11).is_none());
    }

    #[test]
    fn buckets_replay_stats_every_ten_frames() {
        let mut stats = ReplayStats::new();
        let total_frames = REPLAY_SUMMARY_WARMUP_FRAMES + REPLAY_STATS_BUCKET_FRAMES + 1;
        for frame in 0..total_frames {
            stats.record_frame(frame, Duration::from_millis(frame + 1));
        }

        assert_eq!(stats.submitted_frame_count, total_frames);
        assert_eq!(stats.frame_count, REPLAY_STATS_BUCKET_FRAMES + 1);
        assert_eq!(
            stats.ignored_summary_frame_count(),
            REPLAY_SUMMARY_WARMUP_FRAMES
        );
        assert_eq!(
            stats.min,
            Some(Duration::from_millis(REPLAY_SUMMARY_WARMUP_FRAMES + 1))
        );
        assert_eq!(stats.max, Some(Duration::from_millis(total_frames)));
        assert_eq!(stats.buckets.len(), 3);
        assert_eq!(stats.buckets[0].replay_frame_start, 0);
        assert_eq!(stats.buckets[0].replay_frame_end, 9);
        assert_eq!(stats.buckets[0].frame_count, 10);
        assert_eq!(stats.buckets[0].min, Duration::from_millis(1));
        assert_eq!(stats.buckets[0].max, Duration::from_millis(10));
        assert_eq!(stats.buckets[1].replay_frame_start, 10);
        assert_eq!(stats.buckets[1].replay_frame_end, 19);
        assert_eq!(stats.buckets[1].frame_count, 10);
        assert_eq!(stats.buckets[1].min, Duration::from_millis(11));
        assert_eq!(stats.buckets[1].max, Duration::from_millis(20));
        assert_eq!(stats.buckets[2].replay_frame_start, 20);
        assert_eq!(stats.buckets[2].replay_frame_end, 20);
        assert_eq!(stats.buckets[2].frame_count, 1);
    }

    #[test]
    fn parses_config_overrides_and_keeps_defaults() {
        let config = AppConfig::parse(
            r#"
            worldmap = "assets/worldmaps/test/manifest.toml"
            tile_cache_radius = 1
            ray_iteration_count = 200
            performance_render_scale = 0.4
            present_mode = "mailbox"
            max_framerate = 120.0
            render_debug_visuals = true
            near_dda_distance = 96.0
            near_dda_max_steps = 128
            start_x = 123.0
            start_y = 456.0
            start_height = 78.0
            height_lod_blend_start = 175.0
            # normal detail values intentionally omitted
            "#,
        )
        .unwrap();

        assert_eq!(
            config.worldmap,
            PathBuf::from("assets/worldmaps/test/manifest.toml")
        );
        assert_eq!(config.tile_cache_radius, 1);
        assert_eq!(config.ray_iteration_count, 200);
        assert_eq!(config.performance_render_scale, 0.4);
        assert_eq!(config.present_mode, AppPresentMode::Mailbox);
        assert_eq!(config.max_framerate, 120.0);
        assert!(config.render_debug_visuals);
        assert_eq!(config.near_dda_distance, 96.0);
        assert_eq!(config.near_dda_max_steps, 128);
        assert_eq!(config.start_x, 123.0);
        assert_eq!(config.start_y, 456.0);
        assert_eq!(config.start_height, 78.0);
        assert_eq!(config.height_lod_blend_start, 175.0);
        assert_eq!(
            config.normal_detail_blend_start,
            DEFAULT_NORMAL_DETAIL_BLEND_START
        );
        assert_eq!(
            config.normal_detail_blend_end,
            DEFAULT_NORMAL_DETAIL_BLEND_END
        );
    }

    #[test]
    fn parses_present_mode_values() {
        assert_eq!(
            AppConfig::parse("present_mode = vsync")
                .unwrap()
                .present_mode,
            AppPresentMode::Vsync
        );
        assert_eq!(
            AppConfig::parse("present_mode = 'immediate'")
                .unwrap()
                .present_mode,
            AppPresentMode::Immediate
        );
        assert_eq!(
            AppConfig::parse("present_mode = \"mailbox\"")
                .unwrap()
                .present_mode,
            AppPresentMode::Mailbox
        );
    }

    #[test]
    fn rejects_invalid_config_ranges() {
        let error = AppConfig::parse(
            r#"
            ray_iteration_count = 0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("ray_iteration_count"));

        let error = AppConfig::parse(
            r#"
            height_lod_blend_start = 300.0
            height_lod_blend_end = 125.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("height_lod_blend_start"));

        let error = AppConfig::parse(
            r#"
            start_height = -1.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("start_height"));

        let error = AppConfig::parse(
            r#"
            present_mode = "triple"
            "#,
        )
        .unwrap_err();
        assert!(error.contains("present_mode"));

        let error = AppConfig::parse(
            r#"
            near_dda_distance = 0.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("near_dda_distance"));

        let error = AppConfig::parse(
            r#"
            max_framerate = -1.0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("max_framerate"));

        let error = AppConfig::parse(
            r#"
            near_dda_max_steps = 0
            "#,
        )
        .unwrap_err();
        assert!(error.contains("near_dda_max_steps"));

        let error = AppConfig::parse(
            r#"
            render_debug_visuals = maybe
            "#,
        )
        .unwrap_err();
        assert!(error.contains("render_debug_visuals"));

        let error = AppConfig::parse(
            r#"
            tile_cache_radius = 99
            "#,
        )
        .unwrap_err();
        assert!(error.contains("tile_cache_radius"));

        let error = AppConfig::parse(
            r#"
            worldmap = ""
            "#,
        )
        .unwrap_err();
        assert!(error.contains("worldmap"));
    }

    #[test]
    fn samples_height_field_with_bilinear_filtering() {
        let manifest = WorldmapManifest {
            name: "test".to_owned(),
            source_width: 2,
            source_height: 2,
            horizontal_scale: 1.0,
            height_scale: 100.0,
            tile_size: 2,
            tile_padding: 0,
            tile_count_x: 1,
            tile_count_y: 1,
            height_format: "r16le".to_owned(),
            height_near_path: "height/near".to_owned(),
            height_far_path: "height/far/max_1.r16".to_owned(),
            height_far_width: 1,
            height_far_height: 1,
            color_format: "rgba8".to_owned(),
            color_near_path: "color/near".to_owned(),
            color_far_path: "color/far/overview_1.rgba".to_owned(),
            color_far_width: 1,
            color_far_height: 1,
        };
        let bytes = [
            0_u16.to_le_bytes(),
            u16::MAX.to_le_bytes(),
            u16::MAX.to_le_bytes(),
            0_u16.to_le_bytes(),
        ]
        .concat();
        let mut height_field = HeightField::new(&manifest, 1).unwrap();
        height_field.update_slot(0, 0, 0, &bytes).unwrap();

        assert_eq!(height_field.height_at(0.0, 0.0), 0.0);
        assert!((height_field.height_at(0.5, 0.5) - 50.0).abs() < 0.001);
        assert!((height_field.height_at(2.0, 0.0) - 100.0).abs() < 0.001);
    }
}
