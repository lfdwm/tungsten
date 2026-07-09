use std::{
    cell::RefCell,
    collections::HashMap,
    error::Error,
    fs::{self, File},
    io::BufReader,
    path::{Path, PathBuf},
};

use glam::{Vec2, Vec3, Vec4};
use sdl3::gpu::{
    Buffer, BufferUsageFlags, CopyPass, Device, Filter, Sampler, SamplerAddressMode,
    SamplerCreateInfo, SamplerMipmapMode, Texture, TextureFormat, TextureUsage,
};

use crate::gpu_upload::{create_buffer_with_data, create_texture_2d_with_pixels};

const RGBA_BYTES_PER_PIXEL: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RasterVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) texcoord: [f32; 2],
    pub(crate) tangent: [f32; 4],
}

#[derive(Clone, Debug)]
pub(crate) struct RasterModelCpu {
    pub(crate) batches: Vec<RasterMeshCpu>,
    pub(crate) materials: Vec<RasterMaterialCpu>,
    pub(crate) bounds_min: [f32; 3],
    pub(crate) bounds_max: [f32; 3],
}

#[derive(Clone, Debug)]
pub(crate) struct RasterMeshCpu {
    pub(crate) vertices: Vec<RasterVertex>,
    pub(crate) indices: Vec<u32>,
    pub(crate) material_index: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct RasterMaterialCpu {
    pub(crate) diffuse_color: [f32; 3],
    pub(crate) specular_color: [f32; 3],
    pub(crate) shininess: f32,
    pub(crate) diffuse_texture: Option<RasterTextureCpu>,
    pub(crate) specular_texture: Option<RasterTextureCpu>,
    pub(crate) normal_texture: Option<RasterTextureCpu>,
}

#[derive(Clone, Debug)]
pub(crate) struct RasterTextureCpu {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Vec<u8>,
}

pub(crate) struct RasterModelGpu {
    pub(crate) batches: Vec<RasterMeshGpu>,
    pub(crate) materials: Vec<RasterMaterialGpu>,
}

pub(crate) struct RasterMeshGpu {
    pub(crate) vertex_buffer: Buffer,
    pub(crate) index_buffer: Buffer,
    pub(crate) index_count: u32,
    pub(crate) material_index: usize,
}

pub(crate) struct RasterMaterialGpu {
    pub(crate) diffuse_texture: Texture<'static>,
    pub(crate) specular_texture: Texture<'static>,
    pub(crate) normal_texture: Texture<'static>,
    pub(crate) diffuse: [f32; 4],
    pub(crate) specular: [f32; 4],
    pub(crate) flags: [f32; 4],
}

impl RasterModelCpu {
    pub(crate) fn load_obj(path: &Path) -> Result<Self, Box<dyn Error>> {
        let obj_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let material_dirs = RefCell::new(HashMap::new());
        let load_options = tobj::LoadOptions {
            single_index: true,
            triangulate: true,
            ignore_points: true,
            ignore_lines: true,
            ..Default::default()
        };
        let file = File::open(path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        let mut reader = BufReader::new(file);
        let (models, loaded_materials) =
            tobj::load_obj_buf(&mut reader, &load_options, |mtl_path| {
                let resolved_path = resolve_relative_path(&obj_dir, mtl_path);
                let mtl_dir = resolved_path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(&obj_dir)
                    .to_path_buf();
                let loaded = tobj::load_mtl(&resolved_path);

                if let Ok((materials, _names)) = &loaded {
                    let mut dirs = material_dirs.borrow_mut();
                    for material in materials {
                        dirs.insert(material.name.clone(), mtl_dir.clone());
                    }
                }

                loaded
            })
            .map_err(|error| format!("failed to load OBJ {}: {error}", path.display()))?;
        let loaded_materials = loaded_materials.map_err(|error| {
            format!(
                "failed to load OBJ materials for {}: {error}",
                path.display()
            )
        })?;
        let material_dirs = material_dirs.into_inner();
        let mut materials = Vec::with_capacity(loaded_materials.len() + 1);
        materials.push(RasterMaterialCpu::fallback());
        for material in &loaded_materials {
            let material_dir = material_dirs
                .get(&material.name)
                .map(PathBuf::as_path)
                .unwrap_or(&obj_dir);
            materials.push(RasterMaterialCpu::from_tobj(material, material_dir)?);
        }

        let mut batches = Vec::new();
        let mut bounds = ModelBounds::empty();
        for model in &models {
            let mesh = &model.mesh;
            if mesh.indices.is_empty() {
                continue;
            }

            let material_index = mesh
                .material_id
                .and_then(|index| (index + 1 < materials.len()).then_some(index + 1))
                .unwrap_or(0);
            let batch = RasterMeshCpu::from_tobj_mesh(mesh, material_index)
                .map_err(|error| format!("{} in {}: {error}", model.name, path.display()))?;
            bounds.include_vertices(&batch.vertices);
            batches.push(batch);
        }

        if batches.is_empty() {
            return Err(format!("{} did not contain any triangle meshes", path.display()).into());
        }

        Ok(Self {
            batches,
            materials,
            bounds_min: bounds.min.to_array(),
            bounds_max: bounds.max.to_array(),
        })
    }

    pub(crate) fn cube() -> Self {
        let vertices = CUBE_VERTICES.to_vec();
        let indices = CUBE_INDICES.to_vec();
        let mut bounds = ModelBounds::empty();
        bounds.include_vertices(&vertices);

        Self {
            batches: vec![RasterMeshCpu {
                vertices,
                indices,
                material_index: 0,
            }],
            materials: vec![RasterMaterialCpu::cube()],
            bounds_min: bounds.min.to_array(),
            bounds_max: bounds.max.to_array(),
        }
    }
}

impl RasterMeshCpu {
    fn from_tobj_mesh(mesh: &tobj::Mesh, material_index: usize) -> Result<Self, String> {
        if mesh.positions.len() % 3 != 0 {
            return Err("position buffer length is not divisible by 3".to_owned());
        }
        if mesh.indices.len() % 3 != 0 {
            return Err("index buffer length is not divisible by 3 after triangulation".to_owned());
        }

        let vertex_count = mesh.positions.len() / 3;
        let has_normals = mesh.normals.len() >= vertex_count * 3;
        let has_texcoords = mesh.texcoords.len() >= vertex_count * 2;
        let mut vertices = Vec::with_capacity(vertex_count);

        for index in 0..vertex_count {
            let position = [
                mesh.positions[index * 3] as f32,
                mesh.positions[index * 3 + 1] as f32,
                mesh.positions[index * 3 + 2] as f32,
            ];
            let normal = if has_normals {
                normalize_or(
                    Vec3::new(
                        mesh.normals[index * 3] as f32,
                        mesh.normals[index * 3 + 1] as f32,
                        mesh.normals[index * 3 + 2] as f32,
                    ),
                    Vec3::Y,
                )
                .to_array()
            } else {
                [0.0, 0.0, 0.0]
            };
            let texcoord = if has_texcoords {
                [
                    mesh.texcoords[index * 2] as f32,
                    1.0 - mesh.texcoords[index * 2 + 1] as f32,
                ]
            } else {
                [0.0, 0.0]
            };

            vertices.push(RasterVertex {
                position,
                normal,
                texcoord,
                tangent: [1.0, 0.0, 0.0, 1.0],
            });
        }

        for &index in &mesh.indices {
            if index as usize >= vertex_count {
                return Err(format!(
                    "mesh index {index} is out of range for {vertex_count} vertices"
                ));
            }
        }

        let indices = mesh.indices.clone();
        if !has_normals {
            generate_normals(&mut vertices, &indices);
        }
        generate_tangents(&mut vertices, &indices, has_texcoords);

        Ok(Self {
            vertices,
            indices,
            material_index,
        })
    }
}

impl RasterMaterialCpu {
    fn fallback() -> Self {
        Self {
            diffuse_color: [1.0, 1.0, 1.0],
            specular_color: [0.0, 0.0, 0.0],
            shininess: 28.0,
            diffuse_texture: None,
            specular_texture: None,
            normal_texture: None,
        }
    }

    fn cube() -> Self {
        Self {
            diffuse_color: [0.82, 0.48, 0.22],
            specular_color: [0.24, 0.24, 0.24],
            shininess: 28.0,
            diffuse_texture: None,
            specular_texture: None,
            normal_texture: None,
        }
    }

    fn from_tobj(material: &tobj::Material, material_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let diffuse_texture =
            load_texture(material.diffuse_texture.as_deref(), material_dir, "diffuse")?;
        let specular_texture = load_texture(
            material.specular_texture.as_deref(),
            material_dir,
            "specular",
        )?;
        let normal_texture_value = material
            .normal_texture
            .as_deref()
            .or_else(|| unknown_texture_value(material, &["norm", "bump", "map_Bump"]));
        let normal_texture = load_texture(normal_texture_value, material_dir, "normal")?;

        Ok(Self {
            diffuse_color: material
                .diffuse
                .map(|color| [color[0] as f32, color[1] as f32, color[2] as f32])
                .unwrap_or([1.0, 1.0, 1.0]),
            specular_color: material
                .specular
                .map(|color| [color[0] as f32, color[1] as f32, color[2] as f32])
                .unwrap_or([0.0, 0.0, 0.0]),
            shininess: material.shininess.map(|value| value as f32).unwrap_or(28.0),
            diffuse_texture,
            specular_texture,
            normal_texture,
        })
    }
}

impl RasterModelGpu {
    pub(crate) fn upload(gpu: &Device, cpu: &RasterModelCpu) -> Result<Self, Box<dyn Error>> {
        let copy_commands = gpu.acquire_command_buffer()?;
        let copy_pass = gpu.begin_copy_pass(&copy_commands)?;

        let mut materials = Vec::with_capacity(cpu.materials.len());
        for material in &cpu.materials {
            materials.push(RasterMaterialGpu::upload(gpu, &copy_pass, material)?);
        }

        let mut batches = Vec::with_capacity(cpu.batches.len());
        for batch in &cpu.batches {
            batches.push(RasterMeshGpu::upload(gpu, &copy_pass, batch)?);
        }

        gpu.end_copy_pass(copy_pass);
        copy_commands.submit()?;

        Ok(Self { batches, materials })
    }
}

impl RasterMeshGpu {
    fn upload(
        gpu: &Device,
        copy_pass: &CopyPass,
        cpu: &RasterMeshCpu,
    ) -> Result<Self, Box<dyn Error>> {
        let vertex_buffer = create_buffer_with_data(
            gpu,
            copy_pass,
            BufferUsageFlags::VERTEX,
            &cpu.vertices,
            "raster model vertex",
        )?;
        let index_buffer = create_buffer_with_data(
            gpu,
            copy_pass,
            BufferUsageFlags::INDEX,
            &cpu.indices,
            "raster model index",
        )?;
        let index_count =
            u32::try_from(cpu.indices.len()).map_err(|_| "raster model index count exceeds u32")?;

        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count,
            material_index: cpu.material_index,
        })
    }
}

impl RasterMaterialGpu {
    fn upload(
        gpu: &Device,
        copy_pass: &CopyPass,
        cpu: &RasterMaterialCpu,
    ) -> Result<Self, Box<dyn Error>> {
        let diffuse_texture = upload_texture_or_fallback(
            gpu,
            copy_pass,
            cpu.diffuse_texture.as_ref(),
            [255, 255, 255, 255],
        )?;
        let specular_texture = upload_texture_or_fallback(
            gpu,
            copy_pass,
            cpu.specular_texture.as_ref(),
            [0, 0, 0, 255],
        )?;
        let normal_texture = upload_texture_or_fallback(
            gpu,
            copy_pass,
            cpu.normal_texture.as_ref(),
            [128, 128, 255, 255],
        )?;

        Ok(Self {
            diffuse_texture,
            specular_texture,
            normal_texture,
            diffuse: [
                cpu.diffuse_color[0],
                cpu.diffuse_color[1],
                cpu.diffuse_color[2],
                cpu.shininess.max(1.0),
            ],
            specular: [
                cpu.specular_color[0],
                cpu.specular_color[1],
                cpu.specular_color[2],
                0.0,
            ],
            flags: [
                cpu.diffuse_texture.is_some() as u32 as f32,
                cpu.specular_texture.is_some() as u32 as f32,
                cpu.normal_texture.is_some() as u32 as f32,
                0.0,
            ],
        })
    }
}

pub(crate) fn create_raster_model_sampler(gpu: &Device) -> Result<Sampler, Box<dyn Error>> {
    Ok(gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Linear)
            .with_mag_filter(Filter::Linear)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::Repeat)
            .with_address_mode_v(SamplerAddressMode::Repeat)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?)
}

fn load_texture(
    texture_value: Option<&str>,
    material_dir: &Path,
    texture_kind: &str,
) -> Result<Option<RasterTextureCpu>, Box<dyn Error>> {
    let Some(texture_value) = texture_value else {
        return Ok(None);
    };
    let texture_value = texture_path_value(texture_value);
    if texture_value.is_empty() {
        return Ok(None);
    }

    let path = resolve_relative_path(material_dir, Path::new(texture_value));
    if !has_png_extension(&path) {
        return Err(format!(
            "{} is a {texture_kind} texture, but only PNG textures are supported",
            path.display()
        )
        .into());
    }

    let pixels = fs::read(&path)
        .map_err(|error| format!("failed to read PNG texture {}: {error}", path.display()))?;
    let image = image::load_from_memory_with_format(&pixels, image::ImageFormat::Png)
        .map_err(|error| format!("failed to decode PNG texture {}: {error}", path.display()))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(format!("{} decoded to an empty texture", path.display()).into());
    }

    Ok(Some(RasterTextureCpu {
        width,
        height,
        pixels: image.into_raw(),
    }))
}

fn upload_texture_or_fallback(
    gpu: &Device,
    copy_pass: &CopyPass,
    texture: Option<&RasterTextureCpu>,
    fallback_rgba: [u8; 4],
) -> Result<Texture<'static>, Box<dyn Error>> {
    if let Some(texture) = texture {
        return create_texture_from_rgba_pixels(
            gpu,
            copy_pass,
            texture.width,
            texture.height,
            &texture.pixels,
        );
    }

    create_texture_from_rgba_pixels(gpu, copy_pass, 1, 1, &fallback_rgba)
}

fn create_texture_from_rgba_pixels(
    gpu: &Device,
    copy_pass: &CopyPass,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<Texture<'static>, Box<dyn Error>> {
    let expected_size = width as usize * height as usize * RGBA_BYTES_PER_PIXEL;
    if pixels.len() != expected_size {
        return Err(format!(
            "RGBA texture upload has {} bytes, expected {expected_size} for {width}x{height}",
            pixels.len()
        )
        .into());
    }
    create_texture_2d_with_pixels(
        gpu,
        copy_pass,
        width,
        height,
        TextureFormat::R8g8b8a8Unorm,
        TextureUsage::SAMPLER,
        pixels,
        "texture",
    )
}

fn unknown_texture_value<'a>(material: &'a tobj::Material, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(value) = material.unknown_param.get(*key) {
            return Some(value.as_str());
        }
    }

    for (key, value) in &material.unknown_param {
        if keys.iter().any(|wanted| key.eq_ignore_ascii_case(wanted)) {
            return Some(value.as_str());
        }
    }

    None
}

fn texture_path_value(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.starts_with('-') {
        trimmed.split_whitespace().last().unwrap_or("")
    } else {
        trimmed
    }
}

fn resolve_relative_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn has_png_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

fn generate_normals(vertices: &mut [RasterVertex], indices: &[u32]) {
    for vertex in vertices.iter_mut() {
        vertex.normal = [0.0, 0.0, 0.0];
    }

    for triangle in indices.chunks_exact(3) {
        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;
        let p0 = Vec3::from_array(vertices[i0].position);
        let p1 = Vec3::from_array(vertices[i1].position);
        let p2 = Vec3::from_array(vertices[i2].position);
        let normal = (p1 - p0).cross(p2 - p0);

        for index in [i0, i1, i2] {
            vertices[index].normal = (Vec3::from_array(vertices[index].normal) + normal).to_array();
        }
    }

    for vertex in vertices.iter_mut() {
        vertex.normal = normalize_or(Vec3::from_array(vertex.normal), Vec3::Y).to_array();
    }
}

fn generate_tangents(vertices: &mut [RasterVertex], indices: &[u32], has_texcoords: bool) {
    if !has_texcoords {
        for vertex in vertices.iter_mut() {
            vertex.tangent = fallback_tangent(Vec3::from_array(vertex.normal)).to_array();
        }
        return;
    }

    let mut tangent_sums = vec![Vec3::ZERO; vertices.len()];
    let mut bitangent_sums = vec![Vec3::ZERO; vertices.len()];

    for triangle in indices.chunks_exact(3) {
        let i0 = triangle[0] as usize;
        let i1 = triangle[1] as usize;
        let i2 = triangle[2] as usize;

        let p0 = Vec3::from_array(vertices[i0].position);
        let p1 = Vec3::from_array(vertices[i1].position);
        let p2 = Vec3::from_array(vertices[i2].position);
        let uv0 = Vec2::from_array(vertices[i0].texcoord);
        let uv1 = Vec2::from_array(vertices[i1].texcoord);
        let uv2 = Vec2::from_array(vertices[i2].texcoord);
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let delta_uv1 = uv1 - uv0;
        let delta_uv2 = uv2 - uv0;
        let determinant = delta_uv1.x * delta_uv2.y - delta_uv2.x * delta_uv1.y;
        if determinant.abs() <= 0.000001 {
            continue;
        }

        let inverse = 1.0 / determinant;
        let tangent = (edge1 * delta_uv2.y - edge2 * delta_uv1.y) * inverse;
        let bitangent = (edge2 * delta_uv1.x - edge1 * delta_uv2.x) * inverse;

        for &index in &[i0, i1, i2] {
            tangent_sums[index] += tangent;
            bitangent_sums[index] += bitangent;
        }
    }

    for (index, vertex) in vertices.iter_mut().enumerate() {
        let normal = normalize_or(Vec3::from_array(vertex.normal), Vec3::Y);
        let tangent_sum = tangent_sums[index];
        let orthogonal_tangent = tangent_sum - normal * normal.dot(tangent_sum);
        let fallback = fallback_tangent(normal);
        let fallback = Vec3::new(fallback.x, fallback.y, fallback.z);
        let tangent = normalize_or(orthogonal_tangent, fallback);
        let handedness = if normal.cross(tangent).dot(bitangent_sums[index]) < 0.0 {
            -1.0
        } else {
            1.0
        };

        vertex.normal = normal.to_array();
        vertex.tangent = Vec4::new(tangent.x, tangent.y, tangent.z, handedness).to_array();
    }
}

fn fallback_tangent(normal: Vec3) -> Vec4 {
    let axis = if normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let tangent = normalize_or(axis.cross(normal), Vec3::X);

    Vec4::new(tangent.x, tangent.y, tangent.z, 1.0)
}

#[derive(Clone, Copy)]
struct ModelBounds {
    min: Vec3,
    max: Vec3,
}

impl ModelBounds {
    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }

    fn include_vertices(&mut self, vertices: &[RasterVertex]) {
        for vertex in vertices {
            let position = Vec3::from_array(vertex.position);
            self.min = self.min.min(position);
            self.max = self.max.max(position);
        }
    }
}

fn normalize_or(value: Vec3, fallback: Vec3) -> Vec3 {
    let length_sq = value.length_squared();
    if length_sq <= 0.00000001 {
        fallback
    } else {
        value * length_sq.sqrt().recip()
    }
}

const CUBE_VERTICES: &[RasterVertex] = &[
    RasterVertex {
        position: [-0.5, -0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
        texcoord: [0.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, -0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
        texcoord: [1.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, 0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
        texcoord: [1.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, 0.5, 0.5],
        normal: [0.0, 0.0, 1.0],
        texcoord: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, -0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
        texcoord: [0.0, 1.0],
        tangent: [-1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, -0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
        texcoord: [1.0, 1.0],
        tangent: [-1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, 0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
        texcoord: [1.0, 0.0],
        tangent: [-1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, 0.5, -0.5],
        normal: [0.0, 0.0, -1.0],
        texcoord: [0.0, 0.0],
        tangent: [-1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, -0.5, 0.5],
        normal: [1.0, 0.0, 0.0],
        texcoord: [0.0, 1.0],
        tangent: [0.0, 0.0, -1.0, 1.0],
    },
    RasterVertex {
        position: [0.5, -0.5, -0.5],
        normal: [1.0, 0.0, 0.0],
        texcoord: [1.0, 1.0],
        tangent: [0.0, 0.0, -1.0, 1.0],
    },
    RasterVertex {
        position: [0.5, 0.5, -0.5],
        normal: [1.0, 0.0, 0.0],
        texcoord: [1.0, 0.0],
        tangent: [0.0, 0.0, -1.0, 1.0],
    },
    RasterVertex {
        position: [0.5, 0.5, 0.5],
        normal: [1.0, 0.0, 0.0],
        texcoord: [0.0, 0.0],
        tangent: [0.0, 0.0, -1.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, -0.5, -0.5],
        normal: [-1.0, 0.0, 0.0],
        texcoord: [0.0, 1.0],
        tangent: [0.0, 0.0, 1.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, -0.5, 0.5],
        normal: [-1.0, 0.0, 0.0],
        texcoord: [1.0, 1.0],
        tangent: [0.0, 0.0, 1.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, 0.5, 0.5],
        normal: [-1.0, 0.0, 0.0],
        texcoord: [1.0, 0.0],
        tangent: [0.0, 0.0, 1.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, 0.5, -0.5],
        normal: [-1.0, 0.0, 0.0],
        texcoord: [0.0, 0.0],
        tangent: [0.0, 0.0, 1.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, 0.5, 0.5],
        normal: [0.0, 1.0, 0.0],
        texcoord: [0.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, 0.5, 0.5],
        normal: [0.0, 1.0, 0.0],
        texcoord: [1.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, 0.5, -0.5],
        normal: [0.0, 1.0, 0.0],
        texcoord: [1.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, 0.5, -0.5],
        normal: [0.0, 1.0, 0.0],
        texcoord: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, -0.5, -0.5],
        normal: [0.0, -1.0, 0.0],
        texcoord: [0.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, -0.5, -0.5],
        normal: [0.0, -1.0, 0.0],
        texcoord: [1.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [0.5, -0.5, 0.5],
        normal: [0.0, -1.0, 0.0],
        texcoord: [1.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
    RasterVertex {
        position: [-0.5, -0.5, 0.5],
        normal: [0.0, -1.0, 0.0],
        texcoord: [0.0, 0.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    },
];

const CUBE_INDICES: &[u32] = &[
    0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7, 8, 9, 10, 8, 10, 11, 12, 13, 14, 12, 14, 15, 16, 17, 18,
    16, 18, 19, 20, 21, 22, 20, 22, 23,
];

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn loads_obj_with_png_material_maps() {
        let dir = test_dir("loads_obj_with_png_material_maps");
        write_png(&dir.join("diffuse.png"), [200, 20, 30, 255]);
        write_png(&dir.join("specular.png"), [40, 50, 60, 255]);
        write_png(&dir.join("normal.png"), [128, 128, 255, 255]);
        fs::write(
            dir.join("model.mtl"),
            r#"newmtl material0
Kd 0.25 0.5 0.75
Ks 0.1 0.2 0.3
Ns 48
map_Kd diffuse.png
map_Ks specular.png
norm normal.png
"#,
        )
        .unwrap();
        fs::write(
            dir.join("model.obj"),
            r#"mtllib model.mtl
o tri
v 0 0 0
v 1 0 0
v 0 1 0
vt 0 0
vt 1 0
vt 0 1
vn 0 0 1
usemtl material0
f 1/1/1 2/2/1 3/3/1
"#,
        )
        .unwrap();

        let model = RasterModelCpu::load_obj(&dir.join("model.obj")).unwrap();

        assert_eq!(model.batches.len(), 1);
        assert_eq!(model.batches[0].indices.len(), 3);
        assert_eq!(model.batches[0].material_index, 1);
        assert_eq!(model.materials[1].diffuse_color, [0.25, 0.5, 0.75]);
        assert_eq!(model.materials[1].specular_color, [0.1, 0.2, 0.3]);
        assert_eq!(model.materials[1].shininess, 48.0);
        assert_eq!(
            model.materials[1].diffuse_texture.as_ref().unwrap().width,
            1
        );
        assert_eq!(
            model.materials[1].specular_texture.as_ref().unwrap().height,
            1
        );
        assert!(model.materials[1].normal_texture.is_some());
    }

    #[test]
    fn rejects_non_png_material_maps() {
        let dir = test_dir("rejects_non_png_material_maps");
        fs::write(
            dir.join("model.mtl"),
            r#"newmtl material0
map_Kd diffuse.jpg
"#,
        )
        .unwrap();
        fs::write(
            dir.join("model.obj"),
            r#"mtllib model.mtl
v 0 0 0
v 1 0 0
v 0 1 0
usemtl material0
f 1 2 3
"#,
        )
        .unwrap();

        let error = RasterModelCpu::load_obj(&dir.join("model.obj")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("only PNG textures are supported")
        );
    }

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tungsten-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path, rgba: [u8; 4]) {
        let file = File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&rgba).unwrap();
    }
}
