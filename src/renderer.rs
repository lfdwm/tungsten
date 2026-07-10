use std::{error::Error, mem::size_of};

use glam::Vec3;
use sdl3::{
    gpu::{
        BlendFactor, BlendOp, BufferBinding, ColorTargetBlendState, ColorTargetDescription,
        ColorTargetInfo, CompareOp, CullMode, DepthStencilState, DepthStencilTargetInfo, Device,
        FillMode, Filter, GraphicsPipeline, GraphicsPipelineTargetInfo, IndexElementSize, LoadOp,
        PrimitiveType, RasterizerState, Sampler, SamplerAddressMode, SamplerCreateInfo,
        SamplerMipmapMode, ShaderFormat, ShaderStage, StoreOp, Texture, TextureCreateInfo,
        TextureFormat, TextureSamplerBinding, TextureType, TextureUsage, VertexAttribute,
        VertexBufferDescription, VertexElementFormat, VertexInputRate, VertexInputState,
    },
    pixels::Color,
    video::Window,
};

use crate::{
    camera::Camera,
    config::AppConfig,
    raster_model::{RasterModelCpu, RasterModelGpu, RasterVertex, create_raster_model_sampler},
    terrain::TerrainMaps,
    water::WaterVertex,
};

const RAYMARCH_START_DISTANCE: f32 = 0.05;
const DEPTH_TARGET_FORMAT: TextureFormat = TextureFormat::R32Float;
const CUBE_DEPTH_TARGET_FORMAT: TextureFormat = TextureFormat::D32Float;

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
    debug: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RasterParams {
    camera: [f32; 4],
    render: [f32; 4],
    model: [f32; 4],
    rotation: [f32; 4],
    material_diffuse: [f32; 4],
    material_specular: [f32; 4],
    material_flags: [f32; 4],
    ray_forward: [f32; 4],
    ray_right: [f32; 4],
    ray_up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WaterParams {
    camera: [f32; 4],
    render: [f32; 4],
    ray_forward: [f32; 4],
    ray_right: [f32; 4],
    ray_up: [f32; 4],
}

struct RenderTarget {
    color_texture: Texture<'static>,
    terrain_depth_texture: Texture<'static>,
    scene_depth_texture: Texture<'static>,
    cube_depth_texture: Texture<'static>,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DebugVisualMode {
    None,
    HeightSources,
    HitMethods,
    NormalLighting,
    Depth,
}

impl DebugVisualMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::None => Self::HeightSources,
            Self::HeightSources => Self::HitMethods,
            Self::HitMethods => Self::NormalLighting,
            Self::NormalLighting => Self::Depth,
            Self::Depth => Self::None,
        }
    }

    pub(crate) fn as_shader_value(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::HeightSources => 1.0,
            Self::HitMethods => 2.0,
            Self::NormalLighting => 3.0,
            Self::Depth => 4.0,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::HeightSources => "height sources",
            Self::HitMethods => "ray/hit methods",
            Self::NormalLighting => "normal lighting",
            Self::Depth => "depth",
        }
    }

    pub(crate) fn color_key(self) -> &'static str {
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
            Self::Depth => "  white: near terrain\n  black: far terrain and sky",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct OverlayStats {
    pub(crate) fps: f32,
    pub(crate) frame_ms: f32,
}

pub(crate) struct Renderer {
    terrain_pipeline: GraphicsPipeline,
    water_pipeline: GraphicsPipeline,
    raster_pipeline: GraphicsPipeline,
    upscale_pipeline: GraphicsPipeline,
    upscale_sampler: Sampler,
    raster_sampler: Sampler,
    cube_model: RasterModelGpu,
    raster_model: Option<RasterModelGpu>,
    render_target: Option<RenderTarget>,
    target_format: TextureFormat,
}

impl Renderer {
    pub(crate) fn new(
        gpu: &Device,
        target_format: TextureFormat,
        config: &AppConfig,
    ) -> Result<Self, Box<dyn Error>> {
        let cube_model = RasterModelGpu::upload(gpu, &RasterModelCpu::cube())?;
        let raster_model =
            if config.raster_model_enabled && !config.raster_model_path.as_os_str().is_empty() {
                let cpu_model = RasterModelCpu::load_obj(&config.raster_model_path)?;
                let _model_bounds = (cpu_model.bounds_min, cpu_model.bounds_max);
                Some(RasterModelGpu::upload(gpu, &cpu_model)?)
            } else {
                None
            };

        Ok(Self {
            terrain_pipeline: create_terrain_pipeline(gpu, target_format)?,
            water_pipeline: create_water_pipeline(gpu, target_format)?,
            raster_pipeline: create_raster_pipeline(gpu, target_format)?,
            upscale_pipeline: create_upscale_pipeline(gpu, target_format)?,
            upscale_sampler: create_upscale_sampler(gpu)?,
            raster_sampler: create_raster_model_sampler(gpu)?,
            cube_model,
            raster_model,
            render_target: None,
            target_format,
        })
    }

    pub(crate) fn render_frame(
        &mut self,
        gpu: &Device,
        window: &Window,
        terrain_maps: &TerrainMaps,
        camera: &Camera,
        config: &AppConfig,
        debug_visual_mode: DebugVisualMode,
        overlay: OverlayStats,
    ) -> Result<bool, Box<dyn Error>> {
        let (window_width, window_height) = window.size();
        ensure_render_target(
            gpu,
            &mut self.render_target,
            self.target_format,
            window_width,
            window_height,
            config.performance_render_scale,
        )?;
        let render_target = self
            .render_target
            .as_mut()
            .expect("render target should be initialized before drawing");

        let mut command_buffer = gpu.acquire_command_buffer()?;
        let params = shader_params(
            camera,
            terrain_maps,
            render_target.width,
            render_target.height,
            config,
            debug_visual_mode,
        );

        let color_targets = [
            ColorTargetInfo::default()
                .with_texture(&render_target.color_texture)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(105, 136, 157)),
            ColorTargetInfo::default()
                .with_texture(&render_target.terrain_depth_texture)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(255, 255, 255)),
            ColorTargetInfo::default()
                .with_texture(&render_target.scene_depth_texture)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(255, 255, 255)),
        ];

        let render_pass = gpu.begin_render_pass(&command_buffer, &color_targets, None)?;
        render_pass.bind_graphics_pipeline(&self.terrain_pipeline);
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

        let water = &terrain_maps.water;
        let color_targets = [
            ColorTargetInfo::default()
                .with_texture(&render_target.color_texture)
                .with_load_op(LoadOp::LOAD)
                .with_store_op(StoreOp::STORE),
            ColorTargetInfo::default()
                .with_texture(&render_target.scene_depth_texture)
                .with_load_op(LoadOp::LOAD)
                .with_store_op(StoreOp::STORE),
        ];
        let depth_target = DepthStencilTargetInfo::new()
            .with_texture(&mut render_target.cube_depth_texture)
            .with_cycle(true)
            .with_clear_depth(1.0)
            .with_load_op(LoadOp::CLEAR)
            .with_store_op(StoreOp::STORE);
        let water_params = water_params(camera, render_target.width, render_target.height);
        let water_pass =
            gpu.begin_render_pass(&command_buffer, &color_targets, Some(&depth_target))?;
        water_pass.bind_graphics_pipeline(&self.water_pipeline);
        water_pass.bind_fragment_samplers(
            0,
            &[TextureSamplerBinding::new()
                .with_texture(&render_target.terrain_depth_texture)
                .with_sampler(&self.upscale_sampler)],
        );
        command_buffer.push_vertex_uniform_data(0, &water_params);
        command_buffer.push_fragment_uniform_data(0, &water_params);

        water_pass.bind_vertex_buffers(
            0,
            &[BufferBinding::new()
                .with_buffer(&water.ocean.vertex_buffer)
                .with_offset(0)],
        );
        water_pass.bind_index_buffer(
            &BufferBinding::new()
                .with_buffer(&water.ocean.index_buffer)
                .with_offset(0),
            IndexElementSize::_32BIT,
        );
        water_pass.draw_indexed_primitives(water.ocean.index_count, 1, 0, 0, 0);

        for tile in &water.tiles {
            let mesh = &tile.mesh;
            water_pass.bind_vertex_buffers(
                0,
                &[BufferBinding::new()
                    .with_buffer(&mesh.vertex_buffer)
                    .with_offset(0)],
            );
            water_pass.bind_index_buffer(
                &BufferBinding::new()
                    .with_buffer(&mesh.index_buffer)
                    .with_offset(0),
                IndexElementSize::_32BIT,
            );
            water_pass.draw_indexed_primitives(mesh.index_count, 1, 0, 0, 0);
        }
        gpu.end_render_pass(water_pass);

        let raster_draw = if config.raster_model_enabled {
            self.raster_model.as_ref().map(|model| {
                let yaw = config.raster_model_yaw_degrees.to_radians();
                (
                    model,
                    [
                        config.raster_model_x,
                        config.raster_model_y,
                        config.raster_model_height,
                        config.raster_model_scale,
                    ],
                    [yaw.cos(), yaw.sin(), 0.0, 0.0],
                )
            })
        } else {
            None
        }
        .or_else(|| {
            config.raster_cube_enabled.then_some((
                &self.cube_model,
                [
                    config.raster_cube_x,
                    config.raster_cube_y,
                    config.raster_cube_height,
                    config.raster_cube_size,
                ],
                [1.0, 0.0, 0.0, 0.0],
            ))
        });

        if let Some((model, model_transform, rotation)) = raster_draw {
            let color_targets = [
                ColorTargetInfo::default()
                    .with_texture(&render_target.color_texture)
                    .with_load_op(LoadOp::LOAD)
                    .with_store_op(StoreOp::STORE),
                ColorTargetInfo::default()
                    .with_texture(&render_target.scene_depth_texture)
                    .with_load_op(LoadOp::LOAD)
                    .with_store_op(StoreOp::STORE),
            ];

            let depth_target = DepthStencilTargetInfo::new()
                .with_texture(&mut render_target.cube_depth_texture)
                .with_cycle(true)
                .with_clear_depth(1.0)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE);

            let raster_pass =
                gpu.begin_render_pass(&command_buffer, &color_targets, Some(&depth_target))?;
            raster_pass.bind_graphics_pipeline(&self.raster_pipeline);
            for batch in &model.batches {
                let material = model
                    .materials
                    .get(batch.material_index)
                    .unwrap_or(&model.materials[0]);
                let raster_params = raster_params(
                    camera,
                    render_target.width,
                    render_target.height,
                    model_transform,
                    rotation,
                    material.diffuse,
                    material.specular,
                    material.flags,
                );

                raster_pass.bind_vertex_buffers(
                    0,
                    &[BufferBinding::new()
                        .with_buffer(&batch.vertex_buffer)
                        .with_offset(0)],
                );
                raster_pass.bind_index_buffer(
                    &BufferBinding::new()
                        .with_buffer(&batch.index_buffer)
                        .with_offset(0),
                    IndexElementSize::_32BIT,
                );
                raster_pass.bind_fragment_samplers(
                    0,
                    &[
                        TextureSamplerBinding::new()
                            .with_texture(&render_target.terrain_depth_texture)
                            .with_sampler(&self.upscale_sampler),
                        TextureSamplerBinding::new()
                            .with_texture(&material.diffuse_texture)
                            .with_sampler(&self.raster_sampler),
                        TextureSamplerBinding::new()
                            .with_texture(&material.specular_texture)
                            .with_sampler(&self.raster_sampler),
                        TextureSamplerBinding::new()
                            .with_texture(&material.normal_texture)
                            .with_sampler(&self.raster_sampler),
                    ],
                );
                command_buffer.push_vertex_uniform_data(0, &raster_params);
                command_buffer.push_fragment_uniform_data(0, &raster_params);
                raster_pass.draw_indexed_primitives(batch.index_count, 1, 0, 0, 0);
            }
            gpu.end_render_pass(raster_pass);
        }

        if let Ok(swapchain) = command_buffer.wait_and_acquire_swapchain_texture(window) {
            let upscale_params = upscale_params(
                overlay,
                swapchain.width(),
                swapchain.height(),
                debug_visual_mode,
            );
            let color_targets = [ColorTargetInfo::default()
                .with_texture(&swapchain)
                .with_load_op(LoadOp::CLEAR)
                .with_store_op(StoreOp::STORE)
                .with_clear_color(Color::RGB(105, 136, 157))];

            let upscale_pass = gpu.begin_render_pass(&command_buffer, &color_targets, None)?;
            upscale_pass.bind_graphics_pipeline(&self.upscale_pipeline);
            upscale_pass.bind_fragment_samplers(
                0,
                &[
                    TextureSamplerBinding::new()
                        .with_texture(&render_target.color_texture)
                        .with_sampler(&self.upscale_sampler),
                    TextureSamplerBinding::new()
                        .with_texture(&render_target.scene_depth_texture)
                        .with_sampler(&self.upscale_sampler),
                ],
            );
            command_buffer.push_fragment_uniform_data(0, &upscale_params);
            upscale_pass.draw_primitives(3, 1, 0, 0);
            gpu.end_render_pass(upscale_pass);

            command_buffer.submit()?;
            Ok(true)
        } else {
            command_buffer.cancel();
            Ok(false)
        }
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
                ColorTargetDescription::new().with_format(DEPTH_TARGET_FORMAT),
                ColorTargetDescription::new().with_format(DEPTH_TARGET_FORMAT),
            ]),
        )
        .build()?;

    Ok(pipeline)
}

fn create_water_pipeline(
    gpu: &Device,
    target_format: TextureFormat,
) -> Result<GraphicsPipeline, Box<dyn Error>> {
    let vertex_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/water.vert.spv")),
            ShaderStage::Vertex,
        )
        .with_uniform_buffers(1)
        .with_entrypoint(c"main")
        .build()?;

    let fragment_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/water.frag.spv")),
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
        .with_vertex_input_state(
            VertexInputState::new()
                .with_vertex_buffer_descriptions(&[VertexBufferDescription::new()
                    .with_slot(0)
                    .with_pitch(size_of::<WaterVertex>() as u32)
                    .with_input_rate(VertexInputRate::Vertex)
                    .with_instance_step_rate(0)])
                .with_vertex_attributes(&[
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float3)
                        .with_location(0)
                        .with_buffer_slot(0)
                        .with_offset(0),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float3)
                        .with_location(1)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 3]>() as u32),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float2)
                        .with_location(2)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 6]>() as u32),
                ]),
        )
        .with_rasterizer_state(
            RasterizerState::new()
                .with_fill_mode(FillMode::Fill)
                .with_cull_mode(CullMode::None)
                .with_enable_depth_clip(true),
        )
        .with_depth_stencil_state(
            DepthStencilState::new()
                .with_enable_depth_test(true)
                .with_enable_depth_write(true)
                .with_compare_op(CompareOp::Less),
        )
        .with_target_info(
            GraphicsPipelineTargetInfo::new()
                .with_color_target_descriptions(&[
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
                    ColorTargetDescription::new().with_format(DEPTH_TARGET_FORMAT),
                ])
                .with_has_depth_stencil_target(true)
                .with_depth_stencil_format(CUBE_DEPTH_TARGET_FORMAT),
        )
        .build()?;

    Ok(pipeline)
}

fn create_raster_pipeline(
    gpu: &Device,
    target_format: TextureFormat,
) -> Result<GraphicsPipeline, Box<dyn Error>> {
    let vertex_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/raster_cube.vert.spv")),
            ShaderStage::Vertex,
        )
        .with_uniform_buffers(1)
        .with_entrypoint(c"main")
        .build()?;

    let fragment_shader = gpu
        .create_shader()
        .with_code(
            ShaderFormat::SPIRV,
            include_bytes!(concat!(env!("OUT_DIR"), "/raster_cube.frag.spv")),
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
        .with_vertex_input_state(
            VertexInputState::new()
                .with_vertex_buffer_descriptions(&[VertexBufferDescription::new()
                    .with_slot(0)
                    .with_pitch(size_of::<RasterVertex>() as u32)
                    .with_input_rate(VertexInputRate::Vertex)
                    .with_instance_step_rate(0)])
                .with_vertex_attributes(&[
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float3)
                        .with_location(0)
                        .with_buffer_slot(0)
                        .with_offset(0),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float3)
                        .with_location(1)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 3]>() as u32),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float2)
                        .with_location(2)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 6]>() as u32),
                    VertexAttribute::new()
                        .with_format(VertexElementFormat::Float4)
                        .with_location(3)
                        .with_buffer_slot(0)
                        .with_offset(size_of::<[f32; 8]>() as u32),
                ]),
        )
        .with_rasterizer_state(
            RasterizerState::new()
                .with_fill_mode(FillMode::Fill)
                .with_cull_mode(CullMode::None)
                .with_enable_depth_clip(true),
        )
        .with_depth_stencil_state(
            DepthStencilState::new()
                .with_enable_depth_test(true)
                .with_enable_depth_write(true)
                .with_compare_op(CompareOp::Less),
        )
        .with_target_info(
            GraphicsPipelineTargetInfo::new()
                .with_color_target_descriptions(&[
                    ColorTargetDescription::new().with_format(target_format),
                    ColorTargetDescription::new().with_format(DEPTH_TARGET_FORMAT),
                ])
                .with_has_depth_stencil_target(true)
                .with_depth_stencil_format(CUBE_DEPTH_TARGET_FORMAT),
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
        .with_samplers(2)
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

fn upscale_params(
    overlay: OverlayStats,
    width: u32,
    height: u32,
    debug_visual_mode: DebugVisualMode,
) -> UpscaleParams {
    UpscaleParams {
        overlay: [overlay.fps, width as f32, height as f32, overlay.frame_ms],
        debug: [debug_visual_mode.as_shader_value(), 0.0, 0.0, 0.0],
    }
}

fn raster_params(
    camera: &Camera,
    width: u32,
    height: u32,
    model: [f32; 4],
    rotation: [f32; 4],
    material_diffuse: [f32; 4],
    material_specular: [f32; 4],
    material_flags: [f32; 4],
) -> RasterParams {
    let ray_basis = camera_ray_basis(camera, width, height);

    RasterParams {
        camera: [camera.x, camera.y, camera.height, 0.0],
        render: [
            width as f32,
            height as f32,
            RAYMARCH_START_DISTANCE,
            camera.max_distance,
        ],
        model,
        rotation,
        material_diffuse,
        material_specular,
        material_flags,
        ray_forward: vec3_param(ray_basis.forward),
        ray_right: vec3_param(ray_basis.right_scaled),
        ray_up: vec3_param(ray_basis.up_scaled),
    }
}

fn water_params(camera: &Camera, width: u32, height: u32) -> WaterParams {
    let ray_basis = camera_ray_basis(camera, width, height);

    WaterParams {
        camera: [camera.x, camera.y, camera.height, 0.0],
        render: [
            width as f32,
            height as f32,
            RAYMARCH_START_DISTANCE,
            camera.max_distance,
        ],
        ray_forward: vec3_param(ray_basis.forward),
        ray_right: vec3_param(ray_basis.right_scaled),
        ray_up: vec3_param(ray_basis.up_scaled),
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
            color_texture: create_color_target_texture(gpu, width, height, format)?,
            terrain_depth_texture: create_color_target_texture(
                gpu,
                width,
                height,
                DEPTH_TARGET_FORMAT,
            )?,
            scene_depth_texture: create_color_target_texture(
                gpu,
                width,
                height,
                DEPTH_TARGET_FORMAT,
            )?,
            cube_depth_texture: create_depth_target_texture(gpu, width, height)?,
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

fn create_depth_target_texture(
    gpu: &Device,
    width: u32,
    height: u32,
) -> Result<Texture<'static>, Box<dyn Error>> {
    Ok(gpu.create_texture(
        TextureCreateInfo::new()
            .with_format(CUBE_DEPTH_TARGET_FORMAT)
            .with_type(TextureType::_2D)
            .with_width(width)
            .with_height(height)
            .with_layer_count_or_depth(1)
            .with_num_levels(1)
            .with_usage(TextureUsage::DEPTH_STENCIL_TARGET),
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
        ray_forward: vec3_param(ray_basis.forward),
        ray_right: vec3_param(ray_basis.right_scaled),
        ray_up: vec3_param(ray_basis.up_scaled),
    }
}

struct RayBasis {
    forward: Vec3,
    right_scaled: Vec3,
    up_scaled: Vec3,
}

fn camera_ray_basis(camera: &Camera, width: u32, height: u32) -> RayBasis {
    let (sin_yaw, cos_yaw) = camera.yaw.sin_cos();
    let (sin_pitch, cos_pitch) = camera.pitch.sin_cos();
    let forward_flat = Vec3::new(sin_yaw, 0.0, -cos_yaw);
    let right = Vec3::new(cos_yaw, 0.0, sin_yaw);
    let forward = (forward_flat * cos_pitch + Vec3::Y * sin_pitch).normalize();
    let up = (Vec3::Y * cos_pitch - forward_flat * sin_pitch).normalize();
    let aspect = width as f32 / (height as f32).max(1.0);
    let tan_half_fov = (camera.vertical_fov * 0.5).tan();

    RayBasis {
        forward,
        right_scaled: right * aspect * tan_half_fov,
        up_scaled: up * tan_half_fov,
    }
}

fn vec3_param(value: Vec3) -> [f32; 4] {
    [value.x, value.y, value.z, 0.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_debug_visual_modes_including_depth() {
        assert_eq!(DebugVisualMode::None.next(), DebugVisualMode::HeightSources);
        assert_eq!(
            DebugVisualMode::HeightSources.next(),
            DebugVisualMode::HitMethods
        );
        assert_eq!(
            DebugVisualMode::HitMethods.next(),
            DebugVisualMode::NormalLighting
        );
        assert_eq!(
            DebugVisualMode::NormalLighting.next(),
            DebugVisualMode::Depth
        );
        assert_eq!(DebugVisualMode::Depth.next(), DebugVisualMode::None);
        assert_eq!(DebugVisualMode::Depth.as_shader_value(), 4.0);
    }

    #[test]
    fn raster_params_use_config_world_coordinates() {
        let camera = Camera {
            x: 1.0,
            y: 2.0,
            height: 3.0,
            yaw: 0.0,
            pitch: 0.0,
            vertical_fov: 1.0,
            max_distance: 1000.0,
        };

        let params = raster_params(
            &camera,
            800,
            400,
            [12.0, 34.0, 56.0, 7.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.8, 0.7, 0.6, 28.0],
            [0.1, 0.2, 0.3, 0.0],
            [1.0, 0.0, 0.0, 0.0],
        );

        assert_eq!(params.camera, [1.0, 2.0, 3.0, 0.0]);
        assert_eq!(
            params.render,
            [800.0, 400.0, RAYMARCH_START_DISTANCE, 1000.0]
        );
        assert_eq!(params.model, [12.0, 34.0, 56.0, 7.0]);
        assert_eq!(params.rotation, [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(params.material_diffuse, [0.8, 0.7, 0.6, 28.0]);
    }
}
