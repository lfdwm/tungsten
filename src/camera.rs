use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use glam::Vec2;
use sdl3::keyboard::Scancode;

use crate::terrain::HeightField;

const CAMERA_RECORDING_DIR: &str = "recordings";
const CAMERA_RECORDING_INTERVAL_FRAMES: u64 = 10;
const REPLAY_STATS_BUCKET_FRAMES: u64 = 10;
const REPLAY_SUMMARY_WARMUP_FRAMES: u64 = 10;
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

pub(crate) struct Camera {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) height: f32,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) vertical_fov: f32,
    pub(crate) max_distance: f32,
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
pub(crate) struct CameraTrace {
    samples: Vec<CameraSample>,
}

impl CameraTrace {
    pub(crate) fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
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

pub(crate) struct CameraReplay {
    trace: CameraTrace,
    frame: u64,
}

impl CameraReplay {
    pub(crate) fn new(trace: CameraTrace) -> Self {
        let frame = trace.first_frame();
        Self { trace, frame }
    }

    pub(crate) fn current_frame(&self) -> u64 {
        self.frame
    }

    pub(crate) fn apply_to_camera(&self, camera: &mut Camera) -> bool {
        if let Some(sample) = self.trace.sample_at_frame(self.frame) {
            sample.apply_to_camera(camera);
            true
        } else {
            false
        }
    }

    pub(crate) fn advance_after_submitted_frame(&mut self) -> bool {
        if self.frame >= self.trace.last_frame() {
            false
        } else {
            self.frame += 1;
            true
        }
    }
}

pub(crate) struct CameraRecordingSummary {
    path: PathBuf,
    sample_count: u64,
}

pub(crate) struct CameraRecorder {
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

    pub(crate) fn update_after_submitted_frame(
        &mut self,
        camera: &Camera,
    ) -> Result<(), Box<dyn Error>> {
        self.frame += 1;
        if self.frame % CAMERA_RECORDING_INTERVAL_FRAMES == 0 {
            self.write_sample(camera)?;
        }

        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<CameraRecordingSummary, Box<dyn Error>> {
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

pub(crate) struct ReplayStats {
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

    pub(crate) fn record_frame(&mut self, replay_frame: u64, duration: Duration) {
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
    pub(crate) fn new() -> Self {
        Self {
            submitted_frame_count: 0,
            frame_count: 0,
            total: Duration::ZERO,
            min: None,
            max: None,
            buckets: Vec::new(),
        }
    }

    pub(crate) fn record_frame(&mut self, replay_frame: u64, duration: Duration) {
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

pub(crate) fn toggle_camera_recording(
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

pub(crate) fn print_camera_recording_summary(summary: CameraRecordingSummary) {
    println!(
        "camera recording saved: {} ({} samples)",
        summary.path.display(),
        summary.sample_count
    );
}

pub(crate) fn print_replay_stats(stats: &ReplayStats) {
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

pub(crate) fn write_replay_stats_csv(stats: &ReplayStats) -> Result<PathBuf, Box<dyn Error>> {
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CameraMode {
    Freecam,
    Gravity,
}

pub(crate) struct PlayerPhysics {
    vertical_velocity: f32,
    on_ground: bool,
    eye_height: f32,
    move_speed: f32,
}

impl PlayerPhysics {
    pub(crate) fn new() -> Self {
        Self {
            vertical_velocity: 0.0,
            on_ground: false,
            eye_height: PLAYER_EYE_HEIGHT,
            move_speed: PLAYER_MOVE_SPEED,
        }
    }
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

pub(crate) fn update_freecam(
    events: &sdl3::EventPump,
    camera: &mut Camera,
    dt: f32,
    mouse_delta: [f32; 2],
) {
    update_camera_look(events, camera, dt, mouse_delta);

    let keyboard = events.keyboard_state();
    let move_speed = 135.0;
    let height_speed = 80.0;

    let (forward, right) = horizontal_camera_axes(camera.yaw);
    let mut movement = Vec2::ZERO;

    if keyboard.is_scancode_pressed(Scancode::W) {
        movement += forward;
    }
    if keyboard.is_scancode_pressed(Scancode::S) {
        movement -= forward;
    }
    if keyboard.is_scancode_pressed(Scancode::D) {
        movement += right;
    }
    if keyboard.is_scancode_pressed(Scancode::A) {
        movement -= right;
    }

    if movement.length_squared() > 0.0 {
        let movement = movement.normalize() * move_speed * dt;
        camera.x += movement.x;
        camera.y += movement.y;
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

pub(crate) fn enable_gravity_mode(
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

pub(crate) fn update_gravity_camera(
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

pub(crate) fn terrain_full_map_distance(terrain_size: [f32; 2]) -> f32 {
    Vec2::from_array(terrain_size).length()
}

pub(crate) fn apply_gravity_wheel_adjustment(
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
    let (forward, right) = horizontal_camera_axes(camera.yaw);
    let mut movement = Vec2::ZERO;

    if keyboard.is_scancode_pressed(Scancode::W) {
        movement += forward;
    }
    if keyboard.is_scancode_pressed(Scancode::S) {
        movement -= forward;
    }
    if keyboard.is_scancode_pressed(Scancode::D) {
        movement += right;
    }
    if keyboard.is_scancode_pressed(Scancode::A) {
        movement -= right;
    }

    if movement.length_squared() > 0.0 {
        let movement = movement.normalize() * move_speed * dt;
        camera.x += movement.x;
        camera.y += movement.y;
        camera.x = camera.x.clamp(0.0, collision_height.terrain_size[0]);
        camera.y = camera.y.clamp(0.0, collision_height.terrain_size[1]);
    }
}

fn horizontal_camera_axes(yaw: f32) -> (Vec2, Vec2) {
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    (Vec2::new(sin_yaw, -cos_yaw), Vec2::new(cos_yaw, sin_yaw))
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
