use std::{
    error::Error,
    time::{Duration, Instant},
};

use image::GenericImageView;
use sdl3::{
    event::Event,
    gpu::{
        ColorTargetDescription, ColorTargetInfo, Device, FillMode, Filter,
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
const COLOR_MAP_PATH: &str = "assets/untracked/C11W.png";
const HEIGHT_MAP_PATH: &str = "assets/untracked/D11.png";

#[repr(C)]
#[derive(Clone, Copy)]
struct ShaderParams {
    camera: [f32; 4],
    render: [f32; 4],
    maps: [f32; 4],
    tuning: [f32; 4],
}

struct Camera {
    x: f32,
    y: f32,
    angle: f32,
    height: f32,
    horizon_offset: f32,
    projection_scale: f32,
}

struct TerrainMaps {
    color: Texture<'static>,
    height: Texture<'static>,
    color_sampler: Sampler,
    height_sampler: Sampler,
    color_size: [f32; 2],
    height_size: [f32; 2],
}

fn main() -> Result<(), Box<dyn Error>> {
    let sdl = sdl3::init()?;
    let video = sdl.video()?;

    let window = video
        .window("tungsten - SDL_GPU VoxelSpace", WINDOW_WIDTH, WINDOW_HEIGHT)
        .position_centered()
        .resizable()
        .build()?;

    let gpu = Device::new(ShaderFormat::SPIRV, cfg!(debug_assertions))?.with_window(&window)?;
    let pipeline = create_pipeline(&gpu, &window)?;
    let terrain_maps = load_terrain_maps(&gpu)?;

    let mut camera = Camera {
        x: 250.0,
        y: 330.0,
        angle: 0.4,
        height: 150.0,
        horizon_offset: 0.0,
        projection_scale: 910.0,
    };

    let mut events = sdl.event_pump()?;
    let mut previous_frame = Instant::now();

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                _ => {}
            }
        }

        let now = Instant::now();
        let dt = (now - previous_frame).min(Duration::from_millis(50));
        previous_frame = now;
        update_camera(&events, &mut camera, dt.as_secs_f32());

        let mut command_buffer = gpu.acquire_command_buffer()?;
        if let Ok(swapchain) = command_buffer.wait_and_acquire_swapchain_texture(&window) {
            let (width, height) = window.size();
            let params = shader_params(&camera, &terrain_maps, width, height);
            let color_targets = [ColorTargetInfo::default()
                .with_texture(&swapchain)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(105, 136, 157))];

            let render_pass = gpu.begin_render_pass(&command_buffer, &color_targets, None)?;
            render_pass.bind_graphics_pipeline(&pipeline);
            render_pass.bind_fragment_samplers(
                0,
                &[
                    TextureSamplerBinding::new()
                        .with_texture(&terrain_maps.color)
                        .with_sampler(&terrain_maps.color_sampler),
                    TextureSamplerBinding::new()
                        .with_texture(&terrain_maps.height)
                        .with_sampler(&terrain_maps.height_sampler),
                ],
            );
            command_buffer.push_fragment_uniform_data(0, &params);
            render_pass.draw_primitives(3, 1, 0, 0);
            gpu.end_render_pass(render_pass);
            command_buffer.submit()?;
        } else {
            command_buffer.cancel();
        }
    }

    Ok(())
}

fn create_pipeline(
    gpu: &Device,
    window: &sdl3::video::Window,
) -> Result<sdl3::gpu::GraphicsPipeline, Box<dyn Error>> {
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
        .with_samplers(2)
        .with_uniform_buffers(1)
        .with_entrypoint(c"main")
        .build()?;

    let swapchain_format = gpu.get_swapchain_texture_format(window);
    let pipeline = gpu
        .create_graphics_pipeline()
        .with_fragment_shader(&fragment_shader)
        .with_vertex_shader(&vertex_shader)
        .with_primitive_type(PrimitiveType::TriangleList)
        .with_fill_mode(FillMode::Fill)
        .with_target_info(
            GraphicsPipelineTargetInfo::new().with_color_target_descriptions(&[
                ColorTargetDescription::new().with_format(swapchain_format),
            ]),
        )
        .build()?;

    Ok(pipeline)
}

fn load_terrain_maps(gpu: &Device) -> Result<TerrainMaps, Box<dyn Error>> {
    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;

    let color_image = image::open(COLOR_MAP_PATH)?;
    let height_image = image::open(HEIGHT_MAP_PATH)?;
    let color_size = [color_image.width() as f32, color_image.height() as f32];
    let height_size = [height_image.width() as f32, height_image.height() as f32];

    let color =
        create_texture_from_rgba8(gpu, &copy_pass, color_image, TextureFormat::R8g8b8a8Unorm)?;
    let height =
        create_texture_from_rgba8(gpu, &copy_pass, height_image, TextureFormat::R8g8b8a8Unorm)?;
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
            .with_min_filter(Filter::Linear)
            .with_mag_filter(Filter::Linear)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::Repeat)
            .with_address_mode_v(SamplerAddressMode::Repeat)
            .with_address_mode_w(SamplerAddressMode::Repeat),
    )?;

    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;

    Ok(TerrainMaps {
        color,
        height,
        color_sampler,
        height_sampler,
        color_size,
        height_size,
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
    map.mem_mut().copy_from_slice(pixels.as_raw());
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

fn update_camera(events: &sdl3::EventPump, camera: &mut Camera, dt: f32) {
    let keyboard = events.keyboard_state();
    let turn_speed = 1.85;
    let move_speed = 135.0;
    let height_speed = 80.0;

    if keyboard.is_scancode_pressed(Scancode::Q) || keyboard.is_scancode_pressed(Scancode::Left) {
        camera.angle -= turn_speed * dt;
    }
    if keyboard.is_scancode_pressed(Scancode::E) || keyboard.is_scancode_pressed(Scancode::Right) {
        camera.angle += turn_speed * dt;
    }

    let forward = [camera.angle.sin(), -camera.angle.cos()];
    let right = [camera.angle.cos(), camera.angle.sin()];
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
    if keyboard.is_scancode_pressed(Scancode::Up) {
        camera.horizon_offset -= 110.0 * dt;
    }
    if keyboard.is_scancode_pressed(Scancode::Down) {
        camera.horizon_offset += 110.0 * dt;
    }

    camera.height = camera.height.clamp(20.0, 420.0);
    camera.projection_scale = camera.projection_scale.clamp(320.0, 1800.0);
    camera.horizon_offset = camera.horizon_offset.clamp(-220.0, 220.0);
}

fn shader_params(
    camera: &Camera,
    terrain_maps: &TerrainMaps,
    width: u32,
    height: u32,
) -> ShaderParams {
    ShaderParams {
        camera: [camera.x, camera.y, camera.angle, camera.height],
        render: [width as f32, height as f32, camera.projection_scale, 255.0],
        maps: [
            terrain_maps.color_size[0],
            terrain_maps.color_size[1],
            terrain_maps.height_size[0],
            terrain_maps.height_size[1],
        ],
        tuning: [0.43, camera.horizon_offset, 0.72, 850.0],
    }
}
