use std::error::Error;

use sdl3::{
    event::Event,
    gpu::{ColorTargetInfo, Device, LoadOp, ShaderFormat, StoreOp},
    keyboard::Keycode,
    pixels::Color,
};

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

fn main() -> Result<(), Box<dyn Error>> {
    let sdl = sdl3::init()?;
    let video = sdl.video()?;

    let window = video
        .window("tungsten", WINDOW_WIDTH, WINDOW_HEIGHT)
        .position_centered()
        .resizable()
        .build()?;

    let gpu =
        Device::new(supported_shader_formats(), cfg!(debug_assertions))?.with_window(&window)?;
    let mut events = sdl.event_pump()?;

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

        let mut command_buffer = gpu.acquire_command_buffer()?;

        if let Ok(swapchain) = command_buffer.wait_and_acquire_swapchain_texture(&window) {
            let color_targets = [ColorTargetInfo::default()
                .with_texture(&swapchain)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(26, 28, 34))];

            let render_pass = gpu.begin_render_pass(&command_buffer, &color_targets, None)?;
            gpu.end_render_pass(render_pass);
        }

        command_buffer.submit()?;
    }

    Ok(())
}

fn supported_shader_formats() -> ShaderFormat {
    ShaderFormat::SPIRV
        | ShaderFormat::DXIL
        | ShaderFormat::DXBC
        | ShaderFormat::MSL
        | ShaderFormat::METALLIB
}
