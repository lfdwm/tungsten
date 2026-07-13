use std::{error::Error, mem::size_of};

use imgui::{Context, DrawCmd, DrawData, DrawIdx, DrawVert, TextureId};
use sdl3::{
    gpu::{
        BlendFactor, BlendOp, Buffer, BufferBinding, BufferUsageFlags, ColorTargetBlendState,
        ColorTargetDescription, ColorTargetInfo, CommandBuffer, Device, FillMode, GraphicsPipeline,
        GraphicsPipelineTargetInfo, IndexElementSize, LoadOp, PrimitiveType, RasterizerState,
        Sampler, SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode, ShaderFormat,
        ShaderStage, StoreOp, Texture, TextureFormat, TextureSamplerBinding, TextureUsage,
        VertexAttribute, VertexBufferDescription, VertexElementFormat, VertexInputRate,
        VertexInputState,
    },
    pixels::Color,
    rect::Rect,
};

use crate::gpu_upload::{create_buffer_with_data, create_texture_2d_with_pixels};

const FONT_TEXTURE_ID: TextureId = TextureId::new(0);

#[repr(C)]
#[derive(Clone, Copy)]
struct ImguiParams {
    transform: [f32; 4],
}

struct DrawListBuffers {
    vertex_buffer: Buffer,
    index_buffer: Buffer,
}

pub struct ImguiSdlGpuRenderer {
    pipeline: GraphicsPipeline,
    sampler: Sampler,
    font_texture: Texture<'static>,
}

impl ImguiSdlGpuRenderer {
    pub fn new(
        gpu: &Device,
        target_format: TextureFormat,
        imgui: &mut Context,
    ) -> Result<Self, Box<dyn Error>> {
        imgui.set_renderer_name(Some("tungsten-sdl-gpu-imgui".to_owned()));

        let font_texture = upload_font_texture(gpu, imgui)?;
        let pipeline = create_imgui_pipeline(gpu, target_format)?;
        let sampler = gpu.create_sampler(
            SamplerCreateInfo::new()
                .with_min_filter(sdl3::gpu::Filter::Linear)
                .with_mag_filter(sdl3::gpu::Filter::Linear)
                .with_mipmap_mode(SamplerMipmapMode::Nearest)
                .with_address_mode_u(SamplerAddressMode::ClampToEdge)
                .with_address_mode_v(SamplerAddressMode::ClampToEdge)
                .with_address_mode_w(SamplerAddressMode::ClampToEdge),
        )?;

        Ok(Self {
            pipeline,
            sampler,
            font_texture,
        })
    }

    pub fn render(
        &mut self,
        gpu: &Device,
        command_buffer: &CommandBuffer,
        swapchain: &Texture<'_>,
        draw_data: &DrawData,
    ) -> Result<(), Box<dyn Error>> {
        let framebuffer_width =
            (draw_data.display_size[0] * draw_data.framebuffer_scale[0]).round() as u32;
        let framebuffer_height =
            (draw_data.display_size[1] * draw_data.framebuffer_scale[1]).round() as u32;
        if framebuffer_width == 0
            || framebuffer_height == 0
            || draw_data.total_vtx_count == 0
            || draw_data.total_idx_count == 0
        {
            return Ok(());
        }

        let copy_pass = gpu.begin_copy_pass(command_buffer)?;
        let mut buffers = Vec::with_capacity(draw_data.draw_lists_count());
        for draw_list in draw_data.draw_lists() {
            if draw_list.vtx_buffer().is_empty() || draw_list.idx_buffer().is_empty() {
                buffers.push(None);
                continue;
            }

            let vertex_buffer = create_buffer_with_data(
                gpu,
                &copy_pass,
                BufferUsageFlags::VERTEX,
                draw_list.vtx_buffer(),
                "imgui vertex",
            )?;
            let index_buffer = create_buffer_with_data(
                gpu,
                &copy_pass,
                BufferUsageFlags::INDEX,
                draw_list.idx_buffer(),
                "imgui index",
            )?;
            buffers.push(Some(DrawListBuffers {
                vertex_buffer,
                index_buffer,
            }));
        }
        gpu.end_copy_pass(copy_pass);

        let color_targets = [ColorTargetInfo::default()
            .with_texture(swapchain)
            .with_load_op(LoadOp::LOAD)
            .with_store_op(StoreOp::STORE)
            .with_clear_color(Color::RGB(0, 0, 0))];
        let render_pass = gpu.begin_render_pass(command_buffer, &color_targets, None)?;
        render_pass.bind_graphics_pipeline(&self.pipeline);
        render_pass.bind_fragment_samplers(
            0,
            &[TextureSamplerBinding::new()
                .with_texture(&self.font_texture)
                .with_sampler(&self.sampler)],
        );
        command_buffer.push_vertex_uniform_data(0, &imgui_params(draw_data));

        let index_element_size = imgui_index_element_size()?;
        for (draw_list, buffers) in draw_data.draw_lists().zip(buffers.iter()) {
            let Some(buffers) = buffers else {
                continue;
            };

            render_pass.bind_vertex_buffers(
                0,
                &[BufferBinding::new()
                    .with_buffer(&buffers.vertex_buffer)
                    .with_offset(0)],
            );
            render_pass.bind_index_buffer(
                &BufferBinding::new()
                    .with_buffer(&buffers.index_buffer)
                    .with_offset(0),
                index_element_size,
            );

            for command in draw_list.commands() {
                match command {
                    DrawCmd::Elements { count, cmd_params } => {
                        if cmd_params.texture_id != FONT_TEXTURE_ID {
                            continue;
                        }
                        let Some(scissor) = imgui_scissor(
                            draw_data,
                            cmd_params.clip_rect,
                            framebuffer_width,
                            framebuffer_height,
                        ) else {
                            continue;
                        };

                        render_pass.set_scissor(scissor);
                        render_pass.draw_indexed_primitives(
                            u32::try_from(count).map_err(|_| "imgui index command exceeds u32")?,
                            1,
                            u32::try_from(cmd_params.idx_offset)
                                .map_err(|_| "imgui index offset exceeds u32")?,
                            i32::try_from(cmd_params.vtx_offset)
                                .map_err(|_| "imgui vertex offset exceeds i32")?,
                            0,
                        );
                    }
                    DrawCmd::ResetRenderState => {
                        render_pass.bind_graphics_pipeline(&self.pipeline);
                        render_pass.bind_fragment_samplers(
                            0,
                            &[TextureSamplerBinding::new()
                                .with_texture(&self.font_texture)
                                .with_sampler(&self.sampler)],
                        );
                        command_buffer.push_vertex_uniform_data(0, &imgui_params(draw_data));
                    }
                    DrawCmd::RawCallback { .. } => {}
                }
            }
        }

        gpu.end_render_pass(render_pass);

        Ok(())
    }
}

fn upload_font_texture(
    gpu: &Device,
    imgui: &mut Context,
) -> Result<Texture<'static>, Box<dyn Error>> {
    let fonts = imgui.fonts();
    let atlas = fonts.build_rgba32_texture();
    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;
    let texture = create_texture_2d_with_pixels(
        gpu,
        &copy_pass,
        atlas.width,
        atlas.height,
        TextureFormat::R8g8b8a8Unorm,
        TextureUsage::SAMPLER,
        atlas.data,
        "imgui font atlas",
    )?;
    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;

    fonts.tex_id = FONT_TEXTURE_ID;

    Ok(texture)
}

fn create_imgui_pipeline(
    gpu: &Device,
    target_format: TextureFormat,
) -> Result<GraphicsPipeline, Box<dyn Error>> {
    let vertex_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/imgui.vert.spv")),
            ShaderStage::Vertex,
        )
        .with_uniform_buffers(1)
        .with_entrypoint(c"main")
        .build()?;

    let fragment_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/imgui.frag.spv")),
            ShaderStage::Fragment,
        )
        .with_samplers(1)
        .with_entrypoint(c"main")
        .build()?;

    Ok(gpu
        .create_graphics_pipeline()
        .with_fragment_shader(&fragment_shader)
        .with_vertex_shader(&vertex_shader)
        .with_primitive_type(PrimitiveType::TriangleList)
        .with_fill_mode(FillMode::Fill)
        .with_vertex_input_state(
            VertexInputState::new()
                .with_vertex_buffer_descriptions(&[VertexBufferDescription::new()
                    .with_slot(0)
                    .with_pitch(size_of::<DrawVert>() as u32)
                    .with_input_rate(VertexInputRate::Vertex)
                    .with_instance_step_rate(0)])
                .with_vertex_attributes(&[
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float2)
                        .with_location(0)
                        .with_buffer_slot(0)
                        .with_offset(0),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float2)
                        .with_location(1)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 2]>() as u32),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Ubyte4Norm)
                        .with_location(2)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 4]>() as u32),
                ]),
        )
        .with_rasterizer_state(
            RasterizerState::new()
                .with_fill_mode(FillMode::Fill)
                .with_cull_mode(sdl3::gpu::CullMode::None),
        )
        .with_target_info(
            GraphicsPipelineTargetInfo::new().with_color_target_descriptions(&[
                ColorTargetDescription::new()
                    .with_format(target_format)
                    .with_blend_state(
                        ColorTargetBlendState::new()
                            .with_enable_blend(true)
                            .with_src_color_blendfactor(BlendFactor::SrcAlpha)
                            .with_dst_color_blendfactor(BlendFactor::OneMinusSrcAlpha)
                            .with_color_blend_op(BlendOp::Add)
                            .with_src_alpha_blendfactor(BlendFactor::One)
                            .with_dst_alpha_blendfactor(BlendFactor::OneMinusSrcAlpha)
                            .with_alpha_blend_op(BlendOp::Add),
                    ),
            ]),
        )
        .build()?)
}

fn imgui_params(draw_data: &DrawData) -> ImguiParams {
    let scale_x = 2.0 / draw_data.display_size[0].max(1.0);
    let scale_y = 2.0 / draw_data.display_size[1].max(1.0);
    let translate_x = -1.0 - draw_data.display_pos[0] * scale_x;
    let translate_y = -1.0 - draw_data.display_pos[1] * scale_y;

    ImguiParams {
        transform: [scale_x, scale_y, translate_x, translate_y],
    }
}

fn imgui_scissor(
    draw_data: &DrawData,
    clip_rect: [f32; 4],
    framebuffer_width: u32,
    framebuffer_height: u32,
) -> Option<Rect> {
    let min_x =
        ((clip_rect[0] - draw_data.display_pos[0]) * draw_data.framebuffer_scale[0]).floor();
    let min_y =
        ((clip_rect[1] - draw_data.display_pos[1]) * draw_data.framebuffer_scale[1]).floor();
    let max_x = ((clip_rect[2] - draw_data.display_pos[0]) * draw_data.framebuffer_scale[0]).ceil();
    let max_y = ((clip_rect[3] - draw_data.display_pos[1]) * draw_data.framebuffer_scale[1]).ceil();

    let min_x = min_x.clamp(0.0, framebuffer_width as f32);
    let min_y = min_y.clamp(0.0, framebuffer_height as f32);
    let max_x = max_x.clamp(0.0, framebuffer_width as f32);
    let max_y = max_y.clamp(0.0, framebuffer_height as f32);
    if max_x <= min_x || max_y <= min_y {
        return None;
    }

    Some(Rect::new(
        min_x as i32,
        min_y as i32,
        (max_x - min_x).max(1.0) as u32,
        (max_y - min_y).max(1.0) as u32,
    ))
}

fn imgui_index_element_size() -> Result<IndexElementSize, Box<dyn Error>> {
    match size_of::<DrawIdx>() {
        2 => Ok(IndexElementSize::_16BIT),
        4 => Ok(IndexElementSize::_32BIT),
        size => Err(format!("unsupported ImGui index size {size}").into()),
    }
}
