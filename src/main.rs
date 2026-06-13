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
        GraphicsPipelineTargetInfo, LoadOp, PrimitiveType, Sampler, SamplerAddressMode,
        SamplerCreateInfo, SamplerMipmapMode, ShaderFormat, ShaderStage, StoreOp, Texture,
        TextureCreateInfo, TextureFormat, TextureRegion, TextureSamplerBinding,
        TextureTransferInfo, TextureType, TextureUsage, TransferBufferUsage,
    },
    keyboard::{Keycode, Scancode},
    pixels::Color,
};

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;
const CONFIG_PATH: &str = "config.toml";
const COLOR_MAP_PATH: &str = "assets/untracked/continent Material Output 4096_diffuse.png";
const HEIGHT_MAP_NEAR_PATH: &str = "assets/untracked/continent Height Output 8192.r16";
const HEIGHT_MAP_FAR_PATH: &str = "assets/untracked/continent Height Max 1024.r16";
const HEIGHT_MAP_NEAR_WIDTH: u32 = 8192;
const HEIGHT_MAP_NEAR_HEIGHT: u32 = 8192;
const HEIGHT_MAP_FAR_WIDTH: u32 = 1024;
const HEIGHT_MAP_FAR_HEIGHT: u32 = 1024;
const TERRAIN_HORIZONTAL_SCALE: f32 = 0.5;
const HEIGHT_SCALE: f32 = 255.0 * 1.7;
const RAYMARCH_START_DISTANCE: f32 = 1.0;
const DEFAULT_RAY_ITERATION_COUNT: u32 = 700;
const MAX_RAY_ITERATION_COUNT: u32 = 4096;
const DEFAULT_HEIGHT_LOD_BLEND_START: f32 = 125.0;
const DEFAULT_HEIGHT_LOD_BLEND_END: f32 = 300.0;
const DEFAULT_NORMAL_DETAIL_BLEND_START: f32 = 500.0;
const DEFAULT_NORMAL_DETAIL_BLEND_END: f32 = 1000.0;
const DEFAULT_PERFORMANCE_RENDER_SCALE: f32 = 0.5;

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
}

struct RenderTarget {
    texture: Texture<'static>,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AppConfig {
    ray_iteration_count: u32,
    performance_render_scale: f32,
    normal_detail_blend_start: f32,
    normal_detail_blend_end: f32,
    height_lod_blend_start: f32,
    height_lod_blend_end: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            ray_iteration_count: DEFAULT_RAY_ITERATION_COUNT,
            performance_render_scale: DEFAULT_PERFORMANCE_RENDER_SCALE,
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
        x: 250.0,
        y: 330.0,
        height: 150.0,
        yaw: 0.4,
        pitch: -0.08,
        vertical_fov: 1.05,
        max_distance: 3000.0,
    };

    let mut events = sdl.event_pump()?;
    let mut previous_frame = Instant::now();

    'running: loop {
        let mut mouse_delta = [0.0, 0.0];
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
                _ => {}
            }
        }

        let now = Instant::now();
        let dt = (now - previous_frame).min(Duration::from_millis(50));
        previous_frame = now;
        update_camera(&events, &mut camera, dt.as_secs_f32(), mouse_delta);

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
            upscale_pass.draw_primitives(3, 1, 0, 0);
            gpu.end_render_pass(upscale_pass);

            command_buffer.submit()?;
        } else {
            command_buffer.cancel();
        }
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
    let height_near = create_texture_from_r16(
        gpu,
        &copy_pass,
        HEIGHT_MAP_NEAR_PATH,
        HEIGHT_MAP_NEAR_WIDTH,
        HEIGHT_MAP_NEAR_HEIGHT,
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
    let pixels = fs::read(path)?;
    let expected_size = width as usize * height as usize * 2;
    if pixels.len() != expected_size {
        return Err(format!(
            "{path} has {} bytes, expected {expected_size} for a {width}x{height} R16 heightmap",
            pixels.len()
        )
        .into());
    }

    create_texture_from_bytes(
        gpu,
        copy_pass,
        width,
        height,
        TextureFormat::R16Unorm,
        &pixels,
    )
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

fn update_camera(events: &sdl3::EventPump, camera: &mut Camera, dt: f32, mouse_delta: [f32; 2]) {
    let keyboard = events.keyboard_state();
    let turn_speed = 1.85;
    let pitch_speed = 1.35;
    let mouse_sensitivity = 0.0024;
    let move_speed = 135.0;
    let height_speed = 80.0;

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
    camera.pitch = camera.pitch.clamp(-1.45, 1.45);
    camera.vertical_fov = camera.vertical_fov.clamp(0.5, 1.4);
    camera.max_distance = camera.max_distance.clamp(120.0, 4096.0);
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
            HEIGHT_SCALE,
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
            height_lod_blend_start = 175.0
            # normal detail values intentionally omitted
            "#,
        )
        .unwrap();

        assert_eq!(config.ray_iteration_count, 200);
        assert_eq!(config.performance_render_scale, 0.4);
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
    }
}
