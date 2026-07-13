use std::{
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::camera::Camera;

const CAMERA_RECORDING_DIR: &str = "recordings";
const CAMERA_RECORDING_INTERVAL_FRAMES: u64 = 10;
const REPLAY_STATS_BUCKET_FRAMES: u64 = 10;
const REPLAY_SUMMARY_WARMUP_FRAMES: u64 = 10;

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
pub struct CameraTrace {
    samples: Vec<CameraSample>,
}

impl CameraTrace {
    pub fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
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

pub struct CameraReplay {
    trace: CameraTrace,
    frame: u64,
}

impl CameraReplay {
    pub fn new(trace: CameraTrace) -> Self {
        let frame = trace.first_frame();
        Self { trace, frame }
    }

    pub fn current_frame(&self) -> u64 {
        self.frame
    }

    pub fn apply_to_camera(&self, camera: &mut Camera) -> bool {
        if let Some(sample) = self.trace.sample_at_frame(self.frame) {
            sample.apply_to_camera(camera);
            true
        } else {
            false
        }
    }

    pub fn advance_after_submitted_frame(&mut self) -> bool {
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

pub struct CameraRecordingController {
    recorder: Option<CameraRecorder>,
}

impl CameraRecordingController {
    pub fn new() -> Self {
        Self { recorder: None }
    }

    pub fn toggle(&mut self, camera: &Camera) -> Result<(), Box<dyn Error>> {
        if let Some(active_recorder) = self.recorder.take() {
            print_camera_recording_summary(active_recorder.finish()?);
        } else {
            self.recorder = Some(CameraRecorder::start(camera)?);
        }

        Ok(())
    }

    pub fn update_after_submitted_frame(&mut self, camera: &Camera) -> Result<(), Box<dyn Error>> {
        if let Some(active_recorder) = self.recorder.as_mut() {
            active_recorder.update_after_submitted_frame(camera)?;
        }

        Ok(())
    }

    pub fn finish_active(&mut self) -> Result<(), Box<dyn Error>> {
        if let Some(active_recorder) = self.recorder.take() {
            print_camera_recording_summary(active_recorder.finish()?);
        }

        Ok(())
    }
}

pub struct ReplayStats {
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
    pub fn new() -> Self {
        Self {
            submitted_frame_count: 0,
            frame_count: 0,
            total: Duration::ZERO,
            min: None,
            max: None,
            buckets: Vec::new(),
        }
    }

    pub fn record_frame(&mut self, replay_frame: u64, duration: Duration) {
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

fn print_camera_recording_summary(summary: CameraRecordingSummary) {
    println!(
        "camera recording saved: {} ({} samples)",
        summary.path.display(),
        summary.sample_count
    );
}

pub fn print_replay_stats(stats: &ReplayStats) {
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

pub fn write_replay_stats_csv(stats: &ReplayStats) -> Result<PathBuf, Box<dyn Error>> {
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
