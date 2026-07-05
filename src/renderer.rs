use std::{error::Error, mem::size_of};

use sdl3::{
    gpu::{
        Buffer, BufferBinding, BufferRegion, BufferUsageFlags, ColorTargetDescription,
        ColorTargetInfo, CompareOp, CopyPass, CullMode, DepthStencilState, DepthStencilTargetInfo,
        Device, FillMode, Filter, GraphicsPipeline, GraphicsPipelineTargetInfo, IndexElementSize,
        LoadOp, PrimitiveType, RasterizerState, Sampler, SamplerAddressMode, SamplerCreateInfo,
        SamplerMipmapMode, ShaderFormat, ShaderStage, StoreOp, Texture, TextureCreateInfo,
        TextureFormat, TextureSamplerBinding, TextureType, TextureUsage, TransferBuffer,
        TransferBufferLocation, TransferBufferUsage, VertexAttribute, VertexBufferDescription,
        VertexElementFormat, VertexInputRate, VertexInputState,
    },
    pixels::Color,
    video::Window,
};

use crate::{camera::Camera, config::AppConfig, terrain::TerrainMaps};

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
struct RasterCubeParams {
    camera: [f32; 4],
    render: [f32; 4],
    cube: [f32; 4],
    ray_forward: [f32; 4],
    ray_right: [f32; 4],
    ray_up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CubeVertex {
    position: [f32; 3],
    normal: [f32; 3],
}

struct RenderTarget {
    color_texture: Texture<'static>,
    terrain_depth_texture: Texture<'static>,
    scene_depth_texture: Texture<'static>,
    cube_depth_texture: Texture<'static>,
    width: u32,
    height: u32,
}

struct CubeMesh {
    vertex_buffer: Buffer,
    index_buffer: Buffer,
    index_count: u32,
}

const CUBE_VERTICES: &[CubeVertex] = &[
    // +Z face
    CubeVertex {
        position: [-0.5, -0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
    },
    CubeVertex {
        position: [0.5, -0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
    },
    CubeVertex {
        position: [0.5, 0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
    },
    CubeVertex {
        position: [-0.5, 0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
    },
    // -Z face
    CubeVertex {
        position: [0.5, -0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
    },
    CubeVertex {
        position: [-0.5, -0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
    },
    CubeVertex {
        position: [-0.5, 0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
    },
    CubeVertex {
        position: [0.5, 0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
    },
    // +X face
    CubeVertex {
        position: [0.5, -0.5, 0.5],
        normal: [1.0, 0.0, 0.0],
    },
    CubeVertex {
        position: [0.5, -0.5, -0.5],
        normal: [1.0, 0.0, 0.0],
    },
    CubeVertex {
        position: [0.5, 0.5, -0.5],
        normal: [1.0, 0.0, 0.0],
    },
    CubeVertex {
        position: [0.5, 0.5, 0.5],
        normal: [1.0, 0.0, 0.0],
    },
    // -X face
    CubeVertex {
        position: [-0.5, -0.5, -0.5],
        normal: [-1.0, 0.0, 0.0],
    },
    CubeVertex {
        position: [-0.5, -0.5, 0.5],
        normal: [-1.0, 0.0, 0.0],
    },
    CubeVertex {
        position: [-0.5, 0.5, 0.5],
        normal: [-1.0, 0.0, 0.0],
    },
    CubeVertex {
        position: [-0.5, 0.5, -0.5],
        normal: [-1.0, 0.0, 0.0],
    },
    // +Y face
    CubeVertex {
        position: [-0.5, 0.5, 0.5],
        normal: [0.0, 1.0, 0.0],
    },
    CubeVertex {
        position: [0.5, 0.5, 0.5],
        normal: [0.0, 1.0, 0.0],
    },
    CubeVertex {
        position: [0.5, 0.5, -0.5],
        normal: [0.0, 1.0, 0.0],
    },
    CubeVertex {
        position: [-0.5, 0.5, -0.5],
        normal: [0.0, 1.0, 0.0],
    },
    // -Y face
    CubeVertex {
        position: [-0.5, -0.5, -0.5],
        normal: [0.0, -1.0, 0.0],
    },
    CubeVertex {
        position: [0.5, -0.5, -0.5],
        normal: [0.0, -1.0, 0.0],
    },
    CubeVertex {
        position: [0.5, -0.5, 0.5],
        normal: [0.0, -1.0, 0.0],
    },
    CubeVertex {
        position: [-0.5, -0.5, 0.5],
        normal: [0.0, -1.0, 0.0],
    },
];

const CUBE_INDICES: &[u16] = &[
    0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17, 18,
    16, 18, 19, 20, 21, 22, 20, 22, 23,
];

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
    raster_cube_pipeline: GraphicsPipeline,
    upscale_pipeline: GraphicsPipeline,
    upscale_sampler: Sampler,
    cube_mesh: CubeMesh,
    render_target: Option<RenderTarget>,
    target_format: TextureFormat,
}

impl Renderer {
    pub(crate) fn new(gpu: &Device, target_format: TextureFormat) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            terrain_pipeline: create_terrain_pipeline(gpu, target_format)?,
            raster_cube_pipeline: create_raster_cube_pipeline(gpu, target_format)?,
            upscale_pipeline: create_upscale_pipeline(gpu, target_format)?,
            upscale_sampler: create_upscale_sampler(gpu)?,
            cube_mesh: create_cube_mesh(gpu)?,
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

        if config.raster_cube_enabled {
            let cube_params =
                raster_cube_params(camera, render_target.width, render_target.height, config);
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
            raster_pass.bind_graphics_pipeline(&self.raster_cube_pipeline);
            raster_pass.bind_vertex_buffers(
                0,
                &[BufferBinding::new()
                    .with_buffer(&self.cube_mesh.vertex_buffer)
                    .with_offset(0)],
            );
            raster_pass.bind_index_buffer(
                &BufferBinding::new()
                    .with_buffer(&self.cube_mesh.index_buffer)
                    .with_offset(0),
                IndexElementSize::_16BIT,
            );
            raster_pass.bind_fragment_samplers(
                0,
                &[TextureSamplerBinding::new()
                    .with_texture(&render_target.terrain_depth_texture)
                    .with_sampler(&self.upscale_sampler)],
            );
            command_buffer.push_vertex_uniform_data(0, &cube_params);
            command_buffer.push_fragment_uniform_data(0, &cube_params);
            raster_pass.draw_indexed_primitives(self.cube_mesh.index_count, 1, 0, 0, 0);
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

fn create_raster_cube_pipeline(
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
                    .with_pitch(size_of::<CubeVertex>() as u32)
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

fn raster_cube_params(
    camera: &Camera,
    width: u32,
    height: u32,
    config: &AppConfig,
) -> RasterCubeParams {
    let ray_basis = camera_ray_basis(camera, width, height);

    RasterCubeParams {
        camera: [camera.x, camera.y, camera.height, 0.0],
        render: [
            width as f32,
            height as f32,
            RAYMARCH_START_DISTANCE,
            camera.max_distance,
        ],
        cube: [
            config.raster_cube_x,
            config.raster_cube_y,
            config.raster_cube_height,
            config.raster_cube_size,
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

fn create_cube_mesh(gpu: &Device) -> Result<CubeMesh, Box<dyn Error>> {
    let vertices_len_bytes = std::mem::size_of_val(CUBE_VERTICES);
    let indices_len_bytes = std::mem::size_of_val(CUBE_INDICES);
    let transfer_buffer = gpu
        .create_transfer_buffer()
        .with_size(vertices_len_bytes.max(indices_len_bytes) as u32)
        .with_usage(TransferBufferUsage::UPLOAD)
        .build()?;

    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;
    let vertex_buffer = create_buffer_with_data(
        gpu,
        &transfer_buffer,
        &copy_pass,
        BufferUsageFlags::VERTEX,
        CUBE_VERTICES,
    )?;
    let index_buffer = create_buffer_with_data(
        gpu,
        &transfer_buffer,
        &copy_pass,
        BufferUsageFlags::INDEX,
        CUBE_INDICES,
    )?;

    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;

    Ok(CubeMesh {
        vertex_buffer,
        index_buffer,
        index_count: CUBE_INDICES.len() as u32,
    })
}

fn create_buffer_with_data<T: Copy>(
    gpu: &Device,
    transfer_buffer: &TransferBuffer,
    copy_pass: &CopyPass,
    usage: BufferUsageFlags,
    data: &[T],
) -> Result<Buffer, Box<dyn Error>> {
    let len_bytes = std::mem::size_of_val(data);
    let buffer = gpu
        .create_buffer()
        .with_size(len_bytes as u32)
        .with_usage(usage)
        .build()?;

    let mut map = transfer_buffer.map::<T>(gpu, true);
    let mem = map.mem_mut();
    for (index, &value) in data.iter().enumerate() {
        mem[index] = value;
    }
    map.unmap();

    copy_pass.upload_to_gpu_buffer(
        TransferBufferLocation::new()
            .with_offset(0)
            .with_transfer_buffer(transfer_buffer),
        BufferRegion::new()
            .with_offset(0)
            .with_size(len_bytes as u32)
            .with_buffer(&buffer),
        true,
    );

    Ok(buffer)
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
    fn raster_cube_params_use_config_world_coordinates() {
        let mut config = AppConfig::default();
        config.raster_cube_x = 12.0;
        config.raster_cube_y = 34.0;
        config.raster_cube_height = 56.0;
        config.raster_cube_size = 7.0;
        let camera = Camera {
            x: 1.0,
            y: 2.0,
            height: 3.0,
            yaw: 0.0,
            pitch: 0.0,
            vertical_fov: 1.0,
            max_distance: 1000.0,
        };

        let params = raster_cube_params(&camera, 800, 400, &config);

        assert_eq!(params.camera, [1.0, 2.0, 3.0, 0.0]);
        assert_eq!(
            params.render,
            [800.0, 400.0, RAYMARCH_START_DISTANCE, 1000.0]
        );
        assert_eq!(params.cube, [12.0, 34.0, 56.0, 7.0]);
    }
}
