use std::{
    error::Error,
    fs, io,
    time::{Duration, Instant},
};

use image::GenericImageView;
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

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;
const CONFIG_PATH: &str = "config.toml";
//const COLOR_MAP_PATH: &str = "assets/untracked/continent Material Output 4096_diffuse.png";
const COLOR_MAP_PATH: &str = "assets/untracked/continent Material Output 8192_diffuse.png";
//const HEIGHT_MAP_NEAR_PATH: &str = "assets/untracked/continent Height Output 8192.r16";
const HEIGHT_MAP_NEAR_PATH: &str = "assets/untracked/continent Height Output 16384.r16";
const HEIGHT_MAP_NEAR_WIDTH: u32 = 16384;
const HEIGHT_MAP_NEAR_HEIGHT: u32 = 16384;
const HEIGHT_MAP_FAR_PATH: &str = "assets/untracked/continent Height Max 2048.r16";
const HEIGHT_MAP_FAR_WIDTH: u32 = 2048;
const HEIGHT_MAP_FAR_HEIGHT: u32 = 2048;
const RAYMARCH_START_DISTANCE: f32 = 0.05;
const TERRAIN_HEIGHT_SCALE: f32 = 255.0 * 2.1;
const TERRAIN_HORIZONTAL_SCALE: f32 = 0.5;
const DEFAULT_START_X: f32 = 250.0;
const DEFAULT_START_Y: f32 = 330.0;
const DEFAULT_START_HEIGHT: f32 = 150.0;
const DEFAULT_RAY_ITERATION_COUNT: u32 = 700;
const MAX_RAY_ITERATION_COUNT: u32 = 4096;
const DEFAULT_HEIGHT_LOD_BLEND_START: f32 = 125.0;
const DEFAULT_HEIGHT_LOD_BLEND_END: f32 = 300.0;
const DEFAULT_NORMAL_DETAIL_BLEND_START: f32 = 500.0;
const DEFAULT_NORMAL_DETAIL_BLEND_END: f32 = 1000.0;
const DEFAULT_PERFORMANCE_RENDER_SCALE: f32 = 0.5;
const DEFAULT_PRESENT_MODE: AppPresentMode = AppPresentMode::Vsync;
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

#[repr(C)]
#[derive(Clone, Copy)]
struct ShaderParams {
    camera: [f32; 4],
    render: [f32; 4],
    terrain: [f32; 4],
    height_maps: [f32; 4],
    lod_distances: [f32; 4],
    raymarch: [f32; 4],
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

struct TerrainMaps {
    color: Texture<'static>,
    height_near: Texture<'static>,
    height_far: Texture<'static>,
    color_sampler: Sampler,
    height_sampler: Sampler,
    terrain_size: [f32; 2],
    color_size: [f32; 2],
    height_near_size: [f32; 2],
    height_far_size: [f32; 2],
    collision_height: HeightField,
}

struct RenderTarget {
    texture: Texture<'static>,
    width: u32,
    height: u32,
}

struct HeightField {
    samples: Vec<u16>,
    width: u32,
    height: u32,
    terrain_size: [f32; 2],
}

impl HeightField {
    fn from_r16_bytes(
        bytes: &[u8],
        width: u32,
        height: u32,
        terrain_size: [f32; 2],
    ) -> Result<Self, Box<dyn Error>> {
        let expected_size = width as usize * height as usize * 2;
        if bytes.len() != expected_size {
            return Err(format!(
                "R16 height field has {} bytes, expected {expected_size} for {width}x{height}",
                bytes.len()
            )
            .into());
        }

        let samples = bytes
            .chunks_exact(2)
            .map(|sample| u16::from_le_bytes([sample[0], sample[1]]))
            .collect();

        Ok(Self {
            samples,
            width,
            height,
            terrain_size,
        })
    }

    fn height_at(&self, world_x: f32, world_y: f32) -> f32 {
        let sample_x = (world_x / self.terrain_size[0] * self.width as f32)
            .clamp(0.0, (self.width - 1) as f32);
        let sample_y = (world_y / self.terrain_size[1] * self.height as f32)
            .clamp(0.0, (self.height - 1) as f32);
        let x0 = sample_x.floor() as u32;
        let y0 = sample_y.floor() as u32;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
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
        let index = y as usize * self.width as usize + x as usize;
        self.samples[index] as f32 / u16::MAX as f32 * TERRAIN_HEIGHT_SCALE
    }
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct AppConfig {
    ray_iteration_count: u32,
    performance_render_scale: f32,
    present_mode: AppPresentMode,
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
            ray_iteration_count: DEFAULT_RAY_ITERATION_COUNT,
            performance_render_scale: DEFAULT_PERFORMANCE_RENDER_SCALE,
            present_mode: DEFAULT_PRESENT_MODE,
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
                "ray_iteration_count" => {
                    config.ray_iteration_count = parse_config_u32(key, value, line_number)?
                }
                "performance_render_scale" => {
                    config.performance_render_scale = parse_config_f32(key, value, line_number)?
                }
                "present_mode" => {
                    config.present_mode = parse_present_mode_config(value, line_number)?
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
    let config = AppConfig::load(CONFIG_PATH)?;
    let sdl = sdl3::init()?;
    let video = sdl.video()?;

    let window = video
        .window("tungsten - SDL_GPU VoxelSpace", WINDOW_WIDTH, WINDOW_HEIGHT)
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
    let terrain_pipeline = create_terrain_pipeline(&gpu, target_format)?;
    let upscale_pipeline = create_upscale_pipeline(&gpu, target_format)?;
    let terrain_maps = load_terrain_maps(&gpu)?;
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
        max_distance: 3000.0,
    };
    let mut camera_mode = CameraMode::Freecam;
    let mut player_physics = PlayerPhysics::new();
    let mut fps_counter = FpsCounter::new();

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
                Event::MouseMotion { xrel, yrel, .. } => {
                    mouse_delta[0] += xrel;
                    mouse_delta[1] += yrel;
                }
                Event::MouseWheel { y, .. } => {
                    wheel_delta += y;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::G),
                    repeat: false,
                    ..
                } => {
                    camera_mode = match camera_mode {
                        CameraMode::Freecam => {
                            enable_gravity_mode(
                                &mut camera,
                                &mut player_physics,
                                &terrain_maps.collision_height,
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
                } => {
                    jump_requested = true;
                }
                _ => {}
            }
        }

        let now = Instant::now();
        let frame_duration = now - previous_frame;
        let dt = frame_duration.min(Duration::from_millis(50));
        previous_frame = now;
        if camera_mode == CameraMode::Gravity && wheel_delta != 0.0 {
            let keyboard = events.keyboard_state();
            let adjust_move_speed = keyboard.is_scancode_pressed(Scancode::LShift)
                || keyboard.is_scancode_pressed(Scancode::RShift);
            apply_gravity_wheel_adjustment(
                &mut camera,
                &mut player_physics,
                &terrain_maps.collision_height,
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
                &terrain_maps.collision_height,
                dt.as_secs_f32(),
                mouse_delta,
                jump_requested,
            ),
        }

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
        let params = shader_params(
            &camera,
            &terrain_maps,
            render_target.width,
            render_target.height,
            &config,
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
                    .with_texture(&terrain_maps.color)
                    .with_sampler(&terrain_maps.color_sampler),
                TextureSamplerBinding::new()
                    .with_texture(&terrain_maps.height_near)
                    .with_sampler(&terrain_maps.height_sampler),
                TextureSamplerBinding::new()
                    .with_texture(&terrain_maps.height_far)
                    .with_sampler(&terrain_maps.height_sampler),
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
        } else {
            command_buffer.cancel();
        }

        fps_counter.update(frame_duration);
    }

    Ok(())
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
        .with_samplers(3)
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

fn load_terrain_maps(gpu: &Device) -> Result<TerrainMaps, Box<dyn Error>> {
    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;

    let color_image = image::open(COLOR_MAP_PATH)?;
    let color_size = [color_image.width() as f32, color_image.height() as f32];
    let terrain_size = [
        HEIGHT_MAP_NEAR_WIDTH as f32 * TERRAIN_HORIZONTAL_SCALE,
        HEIGHT_MAP_NEAR_HEIGHT as f32 * TERRAIN_HORIZONTAL_SCALE,
    ];
    let height_near_size = [HEIGHT_MAP_NEAR_WIDTH as f32, HEIGHT_MAP_NEAR_HEIGHT as f32];
    let height_far_size = [HEIGHT_MAP_FAR_WIDTH as f32, HEIGHT_MAP_FAR_HEIGHT as f32];

    let color =
        create_texture_from_rgba8(gpu, &copy_pass, color_image, TextureFormat::R8g8b8a8Unorm)?;
    let height_near_pixels = read_r16_pixels(
        HEIGHT_MAP_NEAR_PATH,
        HEIGHT_MAP_NEAR_WIDTH,
        HEIGHT_MAP_NEAR_HEIGHT,
    )?;
    let collision_height = HeightField::from_r16_bytes(
        &height_near_pixels,
        HEIGHT_MAP_NEAR_WIDTH,
        HEIGHT_MAP_NEAR_HEIGHT,
        terrain_size,
    )?;
    let height_near = create_texture_from_bytes(
        gpu,
        &copy_pass,
        HEIGHT_MAP_NEAR_WIDTH,
        HEIGHT_MAP_NEAR_HEIGHT,
        TextureFormat::R16Unorm,
        &height_near_pixels,
    )?;
    let height_far = create_texture_from_r16(
        gpu,
        &copy_pass,
        HEIGHT_MAP_FAR_PATH,
        HEIGHT_MAP_FAR_WIDTH,
        HEIGHT_MAP_FAR_HEIGHT,
    )?;
    let color_sampler = gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::Repeat)
            .with_address_mode_v(SamplerAddressMode::Repeat)
            .with_address_mode_w(SamplerAddressMode::Repeat),
    )?;
    let height_sampler = gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::Repeat)
            .with_address_mode_v(SamplerAddressMode::Repeat)
            .with_address_mode_w(SamplerAddressMode::Repeat),
    )?;

    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;

    Ok(TerrainMaps {
        color,
        height_near,
        height_far,
        color_sampler,
        height_sampler,
        terrain_size,
        color_size,
        height_near_size,
        height_far_size,
        collision_height,
    })
}

fn create_texture_from_rgba8(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    image: image::DynamicImage,
    format: TextureFormat,
) -> Result<Texture<'static>, Box<dyn Error>> {
    let (width, height) = image.dimensions();
    let pixels = image.to_rgba8();

    create_texture_from_bytes(gpu, copy_pass, width, height, format, pixels.as_raw())
}

fn create_texture_from_r16(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    path: &str,
    width: u32,
    height: u32,
) -> Result<Texture<'static>, Box<dyn Error>> {
    let pixels = read_r16_pixels(path, width, height)?;
    create_texture_from_bytes(
        gpu,
        copy_pass,
        width,
        height,
        TextureFormat::R16Unorm,
        &pixels,
    )
}

fn read_r16_pixels(path: &str, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = fs::read(path)?;
    let expected_size = width as usize * height as usize * 2;
    if pixels.len() != expected_size {
        return Err(format!(
            "{path} has {} bytes, expected {expected_size} for a {width}x{height} R16 heightmap",
            pixels.len()
        )
        .into());
    }

    Ok(pixels)
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
    camera.max_distance = camera.max_distance.clamp(120.0, 4096.0);
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
    camera.max_distance = camera.max_distance.clamp(120.0, 4096.0);
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
) -> ShaderParams {
    let ray_basis = camera_ray_basis(camera, width, height);

    ShaderParams {
        camera: [camera.x, camera.y, camera.height, 0.0],
        render: [
            width as f32,
            height as f32,
            camera.vertical_fov,
            TERRAIN_HEIGHT_SCALE,
        ],
        terrain: [
            terrain_maps.terrain_size[0],
            terrain_maps.terrain_size[1],
            terrain_maps.color_size[0],
            terrain_maps.color_size[1],
        ],
        height_maps: [
            terrain_maps.height_near_size[0],
            terrain_maps.height_near_size[1],
            terrain_maps.height_far_size[0],
            terrain_maps.height_far_size[1],
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

    #[test]
    fn parses_config_overrides_and_keeps_defaults() {
        let config = AppConfig::parse(
            r#"
            ray_iteration_count = 200
            performance_render_scale = 0.4
            present_mode = "mailbox"
            start_x = 123.0
            start_y = 456.0
            start_height = 78.0
            height_lod_blend_start = 175.0
            # normal detail values intentionally omitted
            "#,
        )
        .unwrap();

        assert_eq!(config.ray_iteration_count, 200);
        assert_eq!(config.performance_render_scale, 0.4);
        assert_eq!(config.present_mode, AppPresentMode::Mailbox);
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
    }

    #[test]
    fn samples_height_field_with_bilinear_filtering() {
        let bytes = [
            0_u16.to_le_bytes(),
            u16::MAX.to_le_bytes(),
            u16::MAX.to_le_bytes(),
            0_u16.to_le_bytes(),
        ]
        .concat();
        let height_field = HeightField::from_r16_bytes(&bytes, 2, 2, [2.0, 2.0]).unwrap();

        assert_eq!(height_field.height_at(0.0, 0.0), 0.0);
        assert!((height_field.height_at(0.5, 0.5) - TERRAIN_HEIGHT_SCALE * 0.5).abs() < 0.001);
        assert!((height_field.height_at(2.0, 0.0) - TERRAIN_HEIGHT_SCALE).abs() < 0.001);
    }
}
