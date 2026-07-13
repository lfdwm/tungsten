use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    error::Error,
    fs,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use sdl3::gpu::{
    Buffer, BufferUsageFlags, CopyPass, Device, Filter, Sampler, SamplerAddressMode,
    SamplerCreateInfo, SamplerMipmapMode, Texture, TextureFormat, TextureUsage,
};
use serde::{Deserialize, Serialize};

use crate::{
    camera::CameraRay,
    gpu_upload::{create_buffer_with_data, create_texture_2d_with_pixels},
    terrain::{HeightField, TerrainMaps},
    worldmap::WorldmapManifest,
};

const RGBA_BYTES_PER_PIXEL: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PropVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub texcoord: [f32; 2],
    pub tangent: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropInstanceGpu {
    pub model: [f32; 4],
    pub rotation: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropTransform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: f32,
}

pub struct PropScene {
    catalog: PropCatalog,
    worldmap_dir: PathBuf,
    active_window_min: [u32; 2],
    active_window_max: [u32; 2],
    models: HashMap<PathBuf, PropModelState>,
    loader: PropModelLoader,
    pending_uploads: VecDeque<PendingPropModelUpload>,
    draw_groups: Vec<PropDrawGroup>,
}

pub struct PropDrawGroup {
    pub model_path: PathBuf,
    pub instance_buffer: Buffer,
    pub instance_count: u32,
}

enum PropModelState {
    Ready(PropModelGpu),
    Loading,
    Failed,
}

struct PropModelLoader {
    request_sender: mpsc::Sender<PathBuf>,
    result_receiver: mpsc::Receiver<PropModelLoadResult>,
    _worker_thread: thread::JoinHandle<()>,
}

struct PropModelLoadResult {
    path: PathBuf,
    result: Result<PropModelCpu, String>,
}

struct PendingPropModelUpload {
    path: PathBuf,
    cpu_model: PropModelCpu,
}

impl PropModelLoader {
    fn spawn() -> Result<Self, Box<dyn Error>> {
        let (request_sender, request_receiver) = mpsc::channel::<PathBuf>();
        let (result_sender, result_receiver) = mpsc::channel::<PropModelLoadResult>();
        let worker_thread = thread::Builder::new()
            .name("tungsten-prop-loader".to_owned())
            .spawn(move || {
                while let Ok(path) = request_receiver.recv() {
                    let result = PropModelCpu::load_gltf(&path).map_err(|error| error.to_string());
                    if result_sender
                        .send(PropModelLoadResult { path, result })
                        .is_err()
                    {
                        break;
                    }
                }
            })?;

        Ok(Self {
            request_sender,
            result_receiver,
            _worker_thread: worker_thread,
        })
    }

    fn request(&self, path: PathBuf) -> Result<(), mpsc::SendError<PathBuf>> {
        self.request_sender.send(path)
    }
}

#[derive(Clone, Debug)]
pub struct PropCatalog {
    definitions: HashMap<String, PropDefinition>,
}

#[derive(Clone, Debug)]
pub struct PropDefinition {
    pub id: String,
    pub model_path: PathBuf,
    pub metadata: toml::Table,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PropDefinitionToml {
    id: String,
    model_path: PathBuf,
    #[serde(default)]
    metadata: toml::Table,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PropTileToml {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instance: Vec<PropInstanceToml>,
}

pub type PropTile = PropTileToml;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PropInstanceToml {
    pub prop: String,
    pub source_x: f32,
    pub source_y: f32,
    pub height_mode: PropHeightMode,
    pub height: f32,
    pub height_offset: f32,
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub scale: f32,
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    pub metadata: toml::Table,
}

pub type PropInstance = PropInstanceToml;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum PropHeightMode {
    #[serde(rename = "terrain")]
    Terrain,
    #[serde(rename = "absolute")]
    Absolute,
}

impl PropBounds {
    pub fn corners(self) -> [Vec3; 8] {
        let min = Vec3::from_array(self.min);
        let max = Vec3::from_array(self.max);
        [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, max.z),
            Vec3::new(min.x, max.y, max.z),
        ]
    }
}

impl PropTransform {
    pub fn local_to_world(self, local: Vec3) -> Vec3 {
        self.position + self.rotation * (local * self.scale)
    }
}

#[derive(Clone, Copy, Debug)]
struct PropBoundsBuilder {
    min: Vec3,
    max: Vec3,
    has_points: bool,
}

impl PropBoundsBuilder {
    fn empty() -> Self {
        Self {
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
            has_points: false,
        }
    }

    fn include_point(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
        self.has_points = true;
    }

    fn include_bounds(&mut self, bounds: PropBounds) {
        self.include_point(Vec3::from_array(bounds.min));
        self.include_point(Vec3::from_array(bounds.max));
    }

    fn build(self) -> PropBounds {
        if self.has_points {
            PropBounds {
                min: self.min.to_array(),
                max: self.max.to_array(),
            }
        } else {
            PropBounds {
                min: Vec3::ZERO.to_array(),
                max: Vec3::ZERO.to_array(),
            }
        }
    }
}

#[derive(Clone, Debug)]
struct PropModelCpu {
    meshes: Vec<PropMeshCpu>,
    materials: Vec<PropMaterialCpu>,
    bounds: PropBounds,
}

#[derive(Clone, Debug)]
struct PropMeshCpu {
    vertices: Vec<PropVertex>,
    indices: Vec<u32>,
    material_index: usize,
    bounds: PropBounds,
}

#[derive(Clone, Debug)]
struct PropMaterialCpu {
    base_color_factor: [f32; 4],
    metallic_factor: f32,
    roughness_factor: f32,
    base_color_texture: Option<PropTextureCpu>,
    normal_texture: Option<PropTextureCpu>,
}

#[derive(Clone, Debug)]
struct PropTextureCpu {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

pub struct PropModelGpu {
    pub meshes: Vec<PropMeshGpu>,
    pub materials: Vec<PropMaterialGpu>,
    pub bounds: PropBounds,
}

pub struct PropMeshGpu {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    pub index_count: u32,
    pub material_index: usize,
}

pub struct PropMaterialGpu {
    pub base_color_texture: Texture<'static>,
    pub normal_texture: Texture<'static>,
    pub base_color: [f32; 4],
    pub specular: [f32; 4],
    pub flags: [f32; 4],
}

impl PropScene {
    pub fn load(gpu: &Device, terrain: &TerrainMaps) -> Result<Self, Box<dyn Error>> {
        let catalog = PropCatalog::load(
            &terrain.manifest.props_catalog_dir(&terrain.worldmap_dir),
            &terrain.worldmap_dir,
        )?;
        let loader = PropModelLoader::spawn()?;
        let mut scene = Self {
            catalog,
            worldmap_dir: terrain.worldmap_dir.clone(),
            active_window_min: [u32::MAX, u32::MAX],
            active_window_max: [u32::MAX, u32::MAX],
            models: HashMap::new(),
            loader,
            pending_uploads: VecDeque::new(),
            draw_groups: Vec::new(),
        };
        scene.load_initial_window(gpu, terrain)?;

        Ok(scene)
    }

    pub fn update_for_terrain(
        &mut self,
        gpu: &Device,
        terrain: &TerrainMaps,
    ) -> Result<(), Box<dyn Error>> {
        self.update_model_loads(gpu);

        if self.active_window_min == terrain.current_window_min
            && self.active_window_max == terrain.current_window_max
        {
            return Ok(());
        }

        let grouped_instances = self.active_instances(terrain)?;
        for model_path in grouped_instances.keys() {
            self.request_model_if_needed(model_path)?;
        }

        self.draw_groups = upload_instance_groups(gpu, grouped_instances)?;
        self.active_window_min = terrain.current_window_min;
        self.active_window_max = terrain.current_window_max;

        Ok(())
    }

    pub fn refresh_for_editor_tiles(
        &mut self,
        gpu: &Device,
        terrain: &TerrainMaps,
        edited_tiles: &BTreeMap<[u32; 2], PropTileToml>,
    ) -> Result<(), Box<dyn Error>> {
        self.update_model_loads(gpu);

        let grouped_instances = self.active_instances_with_overrides(terrain, edited_tiles)?;
        for model_path in grouped_instances.keys() {
            self.request_model_if_needed(model_path)?;
        }

        self.draw_groups = upload_instance_groups(gpu, grouped_instances)?;
        self.active_window_min = terrain.current_window_min;
        self.active_window_max = terrain.current_window_max;

        Ok(())
    }

    pub fn update_model_loads(&mut self, gpu: &Device) {
        self.drain_completed_model_loads();
        self.upload_one_pending_model(gpu);
    }

    pub fn catalog(&self) -> &PropCatalog {
        &self.catalog
    }

    pub fn draw_groups(&self) -> &[PropDrawGroup] {
        &self.draw_groups
    }

    pub fn model(&self, path: &Path) -> Option<&PropModelGpu> {
        match self.models.get(path) {
            Some(PropModelState::Ready(model)) => Some(model),
            Some(PropModelState::Loading | PropModelState::Failed) | None => None,
        }
    }

    fn load_initial_window(
        &mut self,
        gpu: &Device,
        terrain: &TerrainMaps,
    ) -> Result<(), Box<dyn Error>> {
        let grouped_instances = self.active_instances(terrain)?;
        for model_path in grouped_instances.keys() {
            self.load_model_synchronously(gpu, model_path);
        }

        self.draw_groups = upload_instance_groups(gpu, grouped_instances)?;
        self.active_window_min = terrain.current_window_min;
        self.active_window_max = terrain.current_window_max;

        Ok(())
    }

    fn active_instances(
        &self,
        terrain: &TerrainMaps,
    ) -> Result<BTreeMap<PathBuf, Vec<PropInstanceGpu>>, Box<dyn Error>> {
        let mut grouped_instances = BTreeMap::<PathBuf, Vec<PropInstanceGpu>>::new();
        for tile_y in terrain.current_window_min[1]..=terrain.current_window_max[1] {
            for tile_x in terrain.current_window_min[0]..=terrain.current_window_max[0] {
                self.load_tile_instances(terrain, tile_x, tile_y, &mut grouped_instances)?;
            }
        }

        Ok(grouped_instances)
    }

    fn active_instances_with_overrides(
        &self,
        terrain: &TerrainMaps,
        edited_tiles: &BTreeMap<[u32; 2], PropTileToml>,
    ) -> Result<BTreeMap<PathBuf, Vec<PropInstanceGpu>>, Box<dyn Error>> {
        let mut grouped_instances = BTreeMap::<PathBuf, Vec<PropInstanceGpu>>::new();
        for tile_y in terrain.current_window_min[1]..=terrain.current_window_max[1] {
            for tile_x in terrain.current_window_min[0]..=terrain.current_window_max[0] {
                if let Some(tile) = edited_tiles.get(&[tile_x, tile_y]) {
                    let context = format!("edited prop tile {tile_x},{tile_y}");
                    self.push_tile_instances(
                        terrain,
                        tile_x,
                        tile_y,
                        tile,
                        &context,
                        &mut grouped_instances,
                    )?;
                } else {
                    self.load_tile_instances(terrain, tile_x, tile_y, &mut grouped_instances)?;
                }
            }
        }

        Ok(grouped_instances)
    }

    fn load_model_synchronously(&mut self, gpu: &Device, model_path: &Path) {
        if self.models.contains_key(model_path) {
            return;
        }

        match PropModelCpu::load_gltf(model_path) {
            Ok(cpu_model) => match PropModelGpu::upload(gpu, &cpu_model) {
                Ok(gpu_model) => {
                    self.models
                        .insert(model_path.to_path_buf(), PropModelState::Ready(gpu_model));
                }
                Err(error) => {
                    eprintln!(
                        "failed to upload prop model {}: {error}",
                        model_path.display()
                    );
                    self.models
                        .insert(model_path.to_path_buf(), PropModelState::Failed);
                }
            },
            Err(error) => {
                eprintln!(
                    "failed to load prop model {}: {error}",
                    model_path.display()
                );
                self.models
                    .insert(model_path.to_path_buf(), PropModelState::Failed);
            }
        }
    }

    fn request_model_if_needed(&mut self, model_path: &Path) -> Result<(), Box<dyn Error>> {
        if self.models.contains_key(model_path) {
            return Ok(());
        }

        self.loader
            .request(model_path.to_path_buf())
            .map_err(|_| "prop loader worker stopped unexpectedly")?;
        self.models
            .insert(model_path.to_path_buf(), PropModelState::Loading);

        Ok(())
    }

    fn drain_completed_model_loads(&mut self) {
        while let Ok(result) = self.loader.result_receiver.try_recv() {
            match result.result {
                Ok(cpu_model) => self.pending_uploads.push_back(PendingPropModelUpload {
                    path: result.path,
                    cpu_model,
                }),
                Err(error) => {
                    eprintln!(
                        "failed to load prop model {}: {error}",
                        result.path.display()
                    );
                    self.models.insert(result.path, PropModelState::Failed);
                }
            }
        }
    }

    fn upload_one_pending_model(&mut self, gpu: &Device) {
        let Some(pending) = self.pending_uploads.pop_front() else {
            return;
        };

        match PropModelGpu::upload(gpu, &pending.cpu_model) {
            Ok(gpu_model) => {
                self.models
                    .insert(pending.path, PropModelState::Ready(gpu_model));
            }
            Err(error) => {
                eprintln!(
                    "failed to upload prop model {}: {error}",
                    pending.path.display()
                );
                self.models.insert(pending.path, PropModelState::Failed);
            }
        }
    }

    fn load_tile_instances(
        &self,
        terrain: &TerrainMaps,
        tile_x: u32,
        tile_y: u32,
        grouped_instances: &mut BTreeMap<PathBuf, Vec<PropInstanceGpu>>,
    ) -> Result<(), Box<dyn Error>> {
        let path = terrain
            .manifest
            .props_tile_path(&self.worldmap_dir, tile_x, tile_y);
        if !path.exists() {
            return Ok(());
        }

        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read prop tile {}: {error}", path.display()))?;
        let tile: PropTileToml =
            toml::from_str(&contents).map_err(|error| format!("{}: {error}", path.display()))?;

        self.push_tile_instances(
            terrain,
            tile_x,
            tile_y,
            &tile,
            &path.display().to_string(),
            grouped_instances,
        )?;

        Ok(())
    }

    fn push_tile_instances(
        &self,
        terrain: &TerrainMaps,
        tile_x: u32,
        tile_y: u32,
        tile: &PropTileToml,
        context: &str,
        grouped_instances: &mut BTreeMap<PathBuf, Vec<PropInstanceGpu>>,
    ) -> Result<(), Box<dyn Error>> {
        for (index, instance) in tile.instance.iter().enumerate() {
            let context = format!("{context} instance {index}");
            validate_instance(instance, &terrain.manifest, tile_x, tile_y, &context)?;
            let definition = self.catalog.definition(&instance.prop).ok_or_else(|| {
                format!(
                    "{context} references unknown prop id `{}`; add it to the prop catalog",
                    instance.prop
                )
            })?;
            let gpu_instance =
                prop_instance_gpu(instance, &terrain.manifest, terrain.collision_height());

            grouped_instances
                .entry(definition.model_path.clone())
                .or_default()
                .push(gpu_instance);
        }

        Ok(())
    }
}

impl PropCatalog {
    pub fn load(catalog_dir: &Path, worldmap_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let entries = fs::read_dir(catalog_dir).map_err(|error| {
            format!(
                "failed to read prop catalog directory {}: {error}",
                catalog_dir.display()
            )
        })?;
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
            {
                paths.push(path);
            }
        }
        paths.sort();

        let mut definitions = HashMap::new();
        for path in paths {
            let contents = fs::read_to_string(&path).map_err(|error| {
                format!(
                    "failed to read prop catalog file {}: {error}",
                    path.display()
                )
            })?;
            let parsed: PropDefinitionToml = toml::from_str(&contents)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let definition = PropDefinition::from_toml(parsed, worldmap_dir, &path)?;
            let id = definition.id.clone();
            let previous = definitions.insert(id.clone(), definition);
            if previous.is_some() {
                return Err(
                    format!("{} defines duplicate prop id `{}`", path.display(), id).into(),
                );
            }
        }

        Ok(Self { definitions })
    }

    pub fn definition(&self, id: &str) -> Option<&PropDefinition> {
        self.definitions.get(id)
    }

    pub fn sorted_ids(&self) -> Vec<String> {
        let mut ids = self.definitions.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    }
}

impl PropDefinition {
    fn from_toml(
        toml: PropDefinitionToml,
        worldmap_dir: &Path,
        path: &Path,
    ) -> Result<Self, Box<dyn Error>> {
        if toml.id.is_empty() {
            return Err(format!("{}: prop `id` must not be empty", path.display()).into());
        }
        if toml.model_path.as_os_str().is_empty() {
            return Err(format!("{}: `model_path` must not be empty", path.display()).into());
        }

        Ok(Self {
            id: toml.id,
            model_path: resolve_worldmap_path(worldmap_dir, &toml.model_path),
            metadata: toml.metadata,
        })
    }
}

impl PropTileToml {
    pub fn load_or_empty(path: &Path) -> Result<Self, Box<dyn Error>> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(path)
            .map_err(|error| format!("failed to read prop tile {}: {error}", path.display()))?;
        toml::from_str(&contents).map_err(|error| format!("{}: {error}", path.display()).into())
    }

    pub fn save_to_path(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create prop tile directory {}: {error}",
                    parent.display()
                )
            })?;
        }

        let contents = toml::to_string_pretty(self).map_err(|error| {
            format!("failed to serialize prop tile {}: {error}", path.display())
        })?;
        fs::write(path, contents)
            .map_err(|error| format!("failed to write prop tile {}: {error}", path.display()))?;

        Ok(())
    }
}

impl PropInstanceToml {
    pub fn terrain(prop: String, source_x: f32, source_y: f32) -> Self {
        Self {
            prop,
            source_x,
            source_y,
            height_mode: PropHeightMode::Terrain,
            height: 0.0,
            height_offset: 0.0,
            pitch: 0.0,
            yaw: 0.0,
            roll: 0.0,
            scale: 1.0,
            metadata: toml::Table::new(),
        }
    }
}

impl PropModelCpu {
    fn load_gltf(path: &Path) -> Result<Self, Box<dyn Error>> {
        let (document, buffers, images) = gltf::import(path)
            .map_err(|error| format!("failed to import {}: {error}", path.display()))?;
        let materials = load_materials(&document, &images)?;
        let scene = document
            .default_scene()
            .or_else(|| document.scenes().next())
            .ok_or_else(|| format!("{} does not contain a scene", path.display()))?;
        let mut meshes = Vec::new();

        for node in scene.nodes() {
            load_node_meshes(node, Mat4::IDENTITY, &buffers, &mut meshes)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }

        if meshes.is_empty() {
            return Err(format!("{} did not contain any triangle meshes", path.display()).into());
        }
        let mut bounds = PropBoundsBuilder::empty();
        for mesh in &meshes {
            bounds.include_bounds(mesh.bounds);
        }

        Ok(Self {
            meshes,
            materials,
            bounds: bounds.build(),
        })
    }
}

impl PropMaterialCpu {
    fn fallback() -> Self {
        Self {
            base_color_factor: [1.0, 1.0, 1.0, 1.0],
            metallic_factor: 0.0,
            roughness_factor: 0.5,
            base_color_texture: None,
            normal_texture: None,
        }
    }

    fn from_gltf(
        material: gltf::Material<'_>,
        images: &[gltf::image::Data],
    ) -> Result<Self, Box<dyn Error>> {
        let pbr = material.pbr_metallic_roughness();
        let base_color_texture = if let Some(info) = pbr.base_color_texture() {
            Some(texture_from_gltf_image(
                images,
                info.texture().source().index(),
                "base color",
            )?)
        } else {
            None
        };
        let normal_texture = if let Some(info) = material.normal_texture() {
            Some(texture_from_gltf_image(
                images,
                info.texture().source().index(),
                "normal",
            )?)
        } else {
            None
        };

        Ok(Self {
            base_color_factor: pbr.base_color_factor(),
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
            base_color_texture,
            normal_texture,
        })
    }
}

impl PropModelGpu {
    fn upload(gpu: &Device, cpu: &PropModelCpu) -> Result<Self, Box<dyn Error>> {
        let copy_commands = gpu.acquire_command_buffer()?;
        let copy_pass = gpu.begin_copy_pass(&copy_commands)?;

        let mut materials = Vec::with_capacity(cpu.materials.len());
        for material in &cpu.materials {
            materials.push(PropMaterialGpu::upload(gpu, &copy_pass, material)?);
        }

        let mut meshes = Vec::with_capacity(cpu.meshes.len());
        for mesh in &cpu.meshes {
            meshes.push(PropMeshGpu::upload(gpu, &copy_pass, mesh)?);
        }

        gpu.end_copy_pass(copy_pass);
        copy_commands.submit()?;

        Ok(Self {
            meshes,
            materials,
            bounds: cpu.bounds,
        })
    }
}

impl PropMeshGpu {
    fn upload(
        gpu: &Device,
        copy_pass: &CopyPass,
        cpu: &PropMeshCpu,
    ) -> Result<Self, Box<dyn Error>> {
        let vertex_buffer = create_buffer_with_data(
            gpu,
            copy_pass,
            BufferUsageFlags::VERTEX,
            &cpu.vertices,
            "prop vertex",
        )?;
        let index_buffer = create_buffer_with_data(
            gpu,
            copy_pass,
            BufferUsageFlags::INDEX,
            &cpu.indices,
            "prop index",
        )?;
        let index_count =
            u32::try_from(cpu.indices.len()).map_err(|_| "prop index count exceeds u32")?;

        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count,
            material_index: cpu.material_index,
        })
    }
}

impl PropMaterialGpu {
    fn upload(
        gpu: &Device,
        copy_pass: &CopyPass,
        cpu: &PropMaterialCpu,
    ) -> Result<Self, Box<dyn Error>> {
        let base_color_texture = upload_texture_or_fallback(
            gpu,
            copy_pass,
            cpu.base_color_texture.as_ref(),
            [255, 255, 255, 255],
        )?;
        let normal_texture = upload_texture_or_fallback(
            gpu,
            copy_pass,
            cpu.normal_texture.as_ref(),
            [128, 128, 255, 255],
        )?;
        let roughness = cpu.roughness_factor.clamp(0.04, 1.0);
        let shininess = ((1.0 - roughness) * 96.0 + 4.0).max(1.0);
        let dielectric_specular = 0.04 * (1.0 - cpu.metallic_factor.clamp(0.0, 1.0));

        Ok(Self {
            base_color_texture,
            normal_texture,
            base_color: [
                cpu.base_color_factor[0],
                cpu.base_color_factor[1],
                cpu.base_color_factor[2],
                shininess,
            ],
            specular: [
                dielectric_specular,
                dielectric_specular,
                dielectric_specular,
                0.0,
            ],
            flags: [
                cpu.base_color_texture.is_some() as u32 as f32,
                cpu.normal_texture.is_some() as u32 as f32,
                0.0,
                0.0,
            ],
        })
    }
}

pub fn create_prop_sampler(gpu: &Device) -> Result<Sampler, Box<dyn Error>> {
    Ok(gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::Repeat)
            .with_address_mode_v(SamplerAddressMode::Repeat)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?)
}

pub fn prop_tile_coords_for_source(
    manifest: &WorldmapManifest,
    source_x: f32,
    source_y: f32,
) -> [u32; 2] {
    [
        ((source_x.floor().max(0.0) as u32) / manifest.tile_size).min(manifest.tile_count_x - 1),
        ((source_y.floor().max(0.0) as u32) / manifest.tile_size).min(manifest.tile_count_y - 1),
    ]
}

pub fn source_position_from_world(
    manifest: &WorldmapManifest,
    world_x: f32,
    world_y: f32,
) -> [f32; 2] {
    [
        (world_x / manifest.horizontal_scale).clamp(0.0, manifest.source_width as f32 - 0.001),
        (world_y / manifest.horizontal_scale).clamp(0.0, manifest.source_height as f32 - 0.001),
    ]
}

pub fn prop_transform(
    instance: &PropInstanceToml,
    manifest: &WorldmapManifest,
    height_field: &HeightField,
) -> PropTransform {
    let world_x = instance.source_x * manifest.horizontal_scale;
    let world_y = instance.source_y * manifest.horizontal_scale;
    let base_height = match instance.height_mode {
        PropHeightMode::Terrain => height_field.height_at(world_x, world_y),
        PropHeightMode::Absolute => instance.height,
    };
    let rotation = (Quat::from_rotation_y(instance.yaw)
        * Quat::from_rotation_x(instance.pitch)
        * Quat::from_rotation_z(instance.roll))
    .normalize();

    PropTransform {
        position: Vec3::new(world_x, base_height + instance.height_offset, world_y),
        rotation,
        scale: instance.scale,
    }
}

pub fn prop_bounds_world_corners(transform: PropTransform, bounds: PropBounds) -> [Vec3; 8] {
    bounds
        .corners()
        .map(|corner| transform.local_to_world(corner))
}

pub fn raycast_prop_bounds(
    ray: CameraRay,
    transform: PropTransform,
    bounds: PropBounds,
) -> Option<f32> {
    let inverse_rotation = transform.rotation.conjugate();
    let inverse_scale = transform.scale.max(0.000001).recip();
    let origin = inverse_rotation * (ray.origin - transform.position) * inverse_scale;
    let direction = inverse_rotation * ray.direction * inverse_scale;
    let min = Vec3::from_array(bounds.min);
    let max = Vec3::from_array(bounds.max);
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;

    for axis in 0..3 {
        let origin_axis = origin[axis];
        let direction_axis = direction[axis];
        let min_axis = min[axis];
        let max_axis = max[axis];

        if direction_axis.abs() <= 0.000001 {
            if origin_axis < min_axis || origin_axis > max_axis {
                return None;
            }
            continue;
        }

        let mut t0 = (min_axis - origin_axis) / direction_axis;
        let mut t1 = (max_axis - origin_axis) / direction_axis;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_min = t_min.max(t0);
        t_max = t_max.min(t1);
        if t_min > t_max {
            return None;
        }
    }

    if t_min >= 0.0 {
        Some(t_min)
    } else if t_max >= 0.0 {
        Some(t_max)
    } else {
        None
    }
}

fn upload_instance_groups(
    gpu: &Device,
    grouped_instances: BTreeMap<PathBuf, Vec<PropInstanceGpu>>,
) -> Result<Vec<PropDrawGroup>, Box<dyn Error>> {
    if grouped_instances.is_empty() {
        return Ok(Vec::new());
    }

    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;
    let mut draw_groups = Vec::with_capacity(grouped_instances.len());

    for (model_path, instances) in grouped_instances {
        let instance_buffer = create_buffer_with_data(
            gpu,
            &copy_pass,
            BufferUsageFlags::VERTEX,
            &instances,
            "prop instance",
        )?;
        let instance_count =
            u32::try_from(instances.len()).map_err(|_| "prop instance count exceeds u32")?;
        draw_groups.push(PropDrawGroup {
            model_path,
            instance_buffer,
            instance_count,
        });
    }

    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;

    Ok(draw_groups)
}

fn validate_instance(
    instance: &PropInstanceToml,
    manifest: &WorldmapManifest,
    tile_x: u32,
    tile_y: u32,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    if instance.prop.is_empty() {
        return Err(format!("{context}: `prop` must not be empty").into());
    }
    for (key, value) in [
        ("source_x", instance.source_x),
        ("source_y", instance.source_y),
        ("height", instance.height),
        ("height_offset", instance.height_offset),
        ("pitch", instance.pitch),
        ("yaw", instance.yaw),
        ("roll", instance.roll),
        ("scale", instance.scale),
    ] {
        if !value.is_finite() {
            return Err(format!("{context}: `{key}` must be finite").into());
        }
    }
    if instance.source_x < 0.0
        || instance.source_x >= manifest.source_width as f32
        || instance.source_y < 0.0
        || instance.source_y >= manifest.source_height as f32
    {
        return Err(format!("{context}: source position must be inside the worldmap").into());
    }
    if instance.scale <= 0.0 {
        return Err(format!("{context}: `scale` must be greater than 0.0").into());
    }

    let instance_tile_x =
        ((instance.source_x.floor() as u32) / manifest.tile_size).min(manifest.tile_count_x - 1);
    let instance_tile_y =
        ((instance.source_y.floor() as u32) / manifest.tile_size).min(manifest.tile_count_y - 1);
    if instance_tile_x != tile_x || instance_tile_y != tile_y {
        return Err(format!(
            "{context}: source position belongs to tile {instance_tile_x},{instance_tile_y}, not {tile_x},{tile_y}"
        )
        .into());
    }

    Ok(())
}

fn prop_instance_gpu(
    instance: &PropInstanceToml,
    manifest: &WorldmapManifest,
    height_field: &HeightField,
) -> PropInstanceGpu {
    let transform = prop_transform(instance, manifest, height_field);

    PropInstanceGpu {
        model: [
            transform.position.x,
            transform.position.z,
            transform.position.y,
            transform.scale,
        ],
        rotation: transform.rotation.to_array(),
    }
}

fn load_materials(
    document: &gltf::Document,
    images: &[gltf::image::Data],
) -> Result<Vec<PropMaterialCpu>, Box<dyn Error>> {
    let mut materials = Vec::new();
    materials.push(PropMaterialCpu::fallback());
    for material in document.materials() {
        materials.push(PropMaterialCpu::from_gltf(material, images)?);
    }

    Ok(materials)
}

fn load_node_meshes(
    node: gltf::Node<'_>,
    parent_transform: Mat4,
    buffers: &[gltf::buffer::Data],
    meshes: &mut Vec<PropMeshCpu>,
) -> Result<(), Box<dyn Error>> {
    let transform = parent_transform * node_transform(node.transform());
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            meshes.push(load_primitive(primitive, transform, buffers)?);
        }
    }
    for child in node.children() {
        load_node_meshes(child, transform, buffers, meshes)?;
    }

    Ok(())
}

fn node_transform(transform: gltf::scene::Transform) -> Mat4 {
    let (translation, rotation, scale) = transform.decomposed();
    Mat4::from_scale_rotation_translation(
        Vec3::from_array(scale),
        Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]).normalize(),
        Vec3::from_array(translation),
    )
}

fn load_primitive(
    primitive: gltf::Primitive<'_>,
    transform: Mat4,
    buffers: &[gltf::buffer::Data],
) -> Result<PropMeshCpu, Box<dyn Error>> {
    if primitive.mode() != gltf::mesh::Mode::Triangles {
        return Err("only triangle glTF primitives are supported".into());
    }

    let reader =
        primitive.reader(|buffer| buffers.get(buffer.index()).map(|data| data.0.as_slice()));
    let positions = reader
        .read_positions()
        .ok_or("glTF primitive is missing POSITION data")?
        .collect::<Vec<_>>();
    if positions.is_empty() {
        return Err("glTF primitive contains no vertices".into());
    }

    let vertex_count = positions.len();
    let normals = reader
        .read_normals()
        .map(|normals| normals.collect::<Vec<_>>())
        .transpose_len(vertex_count, "NORMAL")?;
    let texcoords = reader
        .read_tex_coords(0)
        .map(|texcoords| texcoords.into_f32().collect::<Vec<_>>())
        .transpose_len(vertex_count, "TEXCOORD_0")?;
    let tangents = reader
        .read_tangents()
        .map(|tangents| tangents.collect::<Vec<_>>())
        .transpose_len(vertex_count, "TANGENT")?;
    let mut indices = if let Some(indices) = reader.read_indices() {
        indices.into_u32().collect::<Vec<_>>()
    } else {
        (0..u32::try_from(vertex_count).map_err(|_| "glTF vertex count exceeds u32")?).collect()
    };
    if indices.len() % 3 != 0 {
        return Err("glTF triangle index count is not divisible by 3".into());
    }
    for &index in &indices {
        if index as usize >= vertex_count {
            return Err(format!("glTF mesh index {index} is out of range").into());
        }
    }

    let mut vertices = Vec::with_capacity(vertex_count);
    let mut bounds = PropBoundsBuilder::empty();
    for index in 0..vertex_count {
        let position = transform.transform_point3(Vec3::from_array(positions[index]));
        bounds.include_point(position);
        let normal = normals
            .as_ref()
            .map(|values| {
                normalize_or(
                    transform.transform_vector3(Vec3::from_array(values[index])),
                    Vec3::Y,
                )
                .to_array()
            })
            .unwrap_or([0.0, 0.0, 0.0]);
        let texcoord = texcoords
            .as_ref()
            .map(|values| values[index])
            .unwrap_or([0.0, 0.0]);
        let tangent = tangents
            .as_ref()
            .map(|values| {
                let direction = normalize_or(
                    transform.transform_vector3(Vec3::new(
                        values[index][0],
                        values[index][1],
                        values[index][2],
                    )),
                    Vec3::X,
                );
                [direction.x, direction.y, direction.z, values[index][3]]
            })
            .unwrap_or([1.0, 0.0, 0.0, 1.0]);

        vertices.push(PropVertex {
            position: position.to_array(),
            normal,
            texcoord,
            tangent,
        });
    }

    if normals.is_none() {
        generate_normals(&mut vertices, &indices);
    }
    if tangents.is_none() {
        generate_tangents(&mut vertices, &indices, texcoords.is_some());
    }

    indices.shrink_to_fit();

    Ok(PropMeshCpu {
        vertices,
        indices,
        material_index: primitive
            .material()
            .index()
            .map(|index| index + 1)
            .unwrap_or(0),
        bounds: bounds.build(),
    })
}

trait OptionalAttributeLength<T> {
    fn transpose_len(
        self,
        expected_len: usize,
        label: &str,
    ) -> Result<Option<Vec<T>>, Box<dyn Error>>;
}

impl<T> OptionalAttributeLength<T> for Option<Vec<T>> {
    fn transpose_len(
        self,
        expected_len: usize,
        label: &str,
    ) -> Result<Option<Vec<T>>, Box<dyn Error>> {
        if let Some(values) = &self {
            if values.len() != expected_len {
                return Err(format!(
                    "glTF attribute {label} has {} values, expected {expected_len}",
                    values.len()
                )
                .into());
            }
        }

        Ok(self)
    }
}

fn texture_from_gltf_image(
    images: &[gltf::image::Data],
    image_index: usize,
    texture_kind: &str,
) -> Result<PropTextureCpu, Box<dyn Error>> {
    let image = images.get(image_index).ok_or_else(|| {
        format!("glTF {texture_kind} texture references missing image {image_index}")
    })?;
    let pixels = rgba8_from_gltf_image(image, texture_kind)?;
    if image.width == 0 || image.height == 0 {
        return Err(format!("glTF {texture_kind} texture decoded to an empty image").into());
    }

    Ok(PropTextureCpu {
        width: image.width,
        height: image.height,
        pixels,
    })
}

fn rgba8_from_gltf_image(
    image: &gltf::image::Data,
    texture_kind: &str,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixel_count = image.width as usize * image.height as usize;
    let mut rgba = Vec::with_capacity(pixel_count * RGBA_BYTES_PER_PIXEL);

    match image.format {
        gltf::image::Format::R8 => {
            require_image_bytes(image, pixel_count, texture_kind)?;
            for &r in &image.pixels {
                rgba.extend_from_slice(&[r, r, r, 255]);
            }
        }
        gltf::image::Format::R8G8 => {
            require_image_bytes(image, pixel_count * 2, texture_kind)?;
            for pixel in image.pixels.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], 0, 255]);
            }
        }
        gltf::image::Format::R8G8B8 => {
            require_image_bytes(image, pixel_count * 3, texture_kind)?;
            for pixel in image.pixels.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        gltf::image::Format::R8G8B8A8 => {
            require_image_bytes(image, pixel_count * 4, texture_kind)?;
            rgba.extend_from_slice(&image.pixels);
        }
        _ => {
            return Err(format!(
                "glTF {texture_kind} texture format {:?} is not supported",
                image.format
            )
            .into());
        }
    }

    Ok(rgba)
}

fn require_image_bytes(
    image: &gltf::image::Data,
    expected_len: usize,
    texture_kind: &str,
) -> Result<(), Box<dyn Error>> {
    if image.pixels.len() != expected_len {
        return Err(format!(
            "glTF {texture_kind} texture has {} bytes, expected {expected_len}",
            image.pixels.len()
        )
        .into());
    }

    Ok(())
}

fn upload_texture_or_fallback(
    gpu: &Device,
    copy_pass: &CopyPass,
    texture: Option<&PropTextureCpu>,
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
        "prop texture",
    )
}

fn resolve_worldmap_path(worldmap_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        worldmap_dir.join(path)
    }
}

fn generate_normals(vertices: &mut [PropVertex], indices: &[u32]) {
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

fn generate_tangents(vertices: &mut [PropVertex], indices: &[u32], has_texcoords: bool) {
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

fn normalize_or(value: Vec3, fallback: Vec3) -> Vec3 {
    let length_sq = value.length_squared();
    if length_sq <= 0.00000001 {
        fallback
    } else {
        value * length_sq.sqrt().recip()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn loads_catalog_directory() {
        let dir = test_dir("loads_catalog_directory");
        let catalog_dir = dir.join("props/catalog");
        fs::create_dir_all(&catalog_dir).unwrap();
        fs::write(
            catalog_dir.join("pine.toml"),
            r#"
id = "pine_a"
model_path = "props/assets/pine_a.glb"
metadata = { kind = "tree" }
"#,
        )
        .unwrap();

        let catalog = PropCatalog::load(&catalog_dir, &dir).unwrap();
        let definition = catalog.definition("pine_a").unwrap();

        assert_eq!(definition.id, "pine_a");
        assert_eq!(definition.model_path, dir.join("props/assets/pine_a.glb"));
        assert_eq!(definition.metadata["kind"].as_str().unwrap(), "tree");
    }

    #[test]
    fn parses_tile_instances_with_three_rotation_axes() {
        let tile: PropTileToml = toml::from_str(
            r#"
[[instance]]
prop = "pine_a"
source_x = 12.0
source_y = 34.0
height_mode = "terrain"
height = 0.0
height_offset = 1.5
pitch = 0.1
yaw = 0.2
roll = 0.3
scale = 2.0
metadata = { spawn_id = "pine_1" }
"#,
        )
        .unwrap();

        assert_eq!(tile.instance.len(), 1);
        assert_eq!(tile.instance[0].height_mode, PropHeightMode::Terrain);
        assert_eq!(tile.instance[0].pitch, 0.1);
        assert_eq!(tile.instance[0].yaw, 0.2);
        assert_eq!(tile.instance[0].roll, 0.3);
        assert_eq!(
            tile.instance[0].metadata["spawn_id"].as_str().unwrap(),
            "pine_1"
        );
    }

    #[test]
    fn loads_minimal_gltf_mesh() {
        let dir = test_dir("loads_minimal_gltf_mesh");
        let path = write_minimal_gltf(&dir);

        let model = PropModelCpu::load_gltf(&path).unwrap();

        assert_eq!(model.meshes.len(), 1);
        assert_eq!(model.meshes[0].vertices.len(), 3);
        assert_eq!(model.meshes[0].indices, vec![0, 1, 2]);
        assert_eq!(model.materials.len(), 1);
        assert_eq!(model.bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(model.bounds.max, [1.0, 1.0, 0.0]);
        assert_eq!(model.meshes[0].bounds, model.bounds);
        assert_ne!(model.meshes[0].vertices[0].normal, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn raycasts_transformed_prop_bounds() {
        let bounds = PropBounds {
            min: [-1.0, -1.0, -1.0],
            max: [1.0, 1.0, 1.0],
        };
        let transform = PropTransform {
            position: Vec3::new(2.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
            scale: 2.0,
        };
        let hit_ray = CameraRay {
            origin: Vec3::new(2.0, 0.0, -8.0),
            direction: Vec3::Z,
        };
        let miss_ray = CameraRay {
            origin: Vec3::new(8.0, 0.0, -8.0),
            direction: Vec3::Z,
        };

        assert_eq!(
            raycast_prop_bounds(hit_ray, transform, bounds).unwrap(),
            6.0
        );
        assert_eq!(raycast_prop_bounds(miss_ray, transform, bounds), None);
    }

    #[test]
    fn worker_loads_requested_model() {
        let dir = test_dir("worker_loads_requested_model");
        let path = write_minimal_gltf(&dir);
        let loader = PropModelLoader::spawn().unwrap();

        loader.request(path.clone()).unwrap();
        let result = loader
            .result_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        assert_eq!(result.path, path);
        assert_eq!(result.result.unwrap().meshes.len(), 1);
    }

    #[test]
    fn worker_reports_load_errors() {
        let dir = test_dir("worker_reports_load_errors");
        let path = dir.join("missing.gltf");
        let loader = PropModelLoader::spawn().unwrap();

        loader.request(path.clone()).unwrap();
        let result = loader
            .result_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();

        assert_eq!(result.path, path);
        assert!(result.result.unwrap_err().contains("failed to import"));
    }

    #[test]
    fn scheduling_marks_model_loading_once() {
        let dir = test_dir("scheduling_marks_model_loading_once");
        let model_path = dir.join("missing.gltf");
        let loader = PropModelLoader::spawn().unwrap();
        let mut scene = PropScene {
            catalog: PropCatalog {
                definitions: HashMap::new(),
            },
            worldmap_dir: dir,
            active_window_min: [u32::MAX, u32::MAX],
            active_window_max: [u32::MAX, u32::MAX],
            models: HashMap::new(),
            loader,
            pending_uploads: VecDeque::new(),
            draw_groups: Vec::new(),
        };

        scene.request_model_if_needed(&model_path).unwrap();
        scene.request_model_if_needed(&model_path).unwrap();

        assert!(matches!(
            scene.models.get(&model_path),
            Some(PropModelState::Loading)
        ));
    }

    fn write_minimal_gltf(dir: &Path) -> PathBuf {
        let mut buffer = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0_u16, 1, 2] {
            buffer.extend_from_slice(&index.to_le_bytes());
        }
        fs::write(dir.join("tri.bin"), &buffer).unwrap();
        fs::write(
            dir.join("tri.gltf"),
            format!(
                r#"{{
  "asset": {{ "version": "2.0" }},
  "scene": 0,
  "scenes": [{{ "nodes": [0] }}],
  "nodes": [{{ "mesh": 0 }}],
  "buffers": [{{ "uri": "tri.bin", "byteLength": {} }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 36, "target": 34962 }},
    {{ "buffer": 0, "byteOffset": 36, "byteLength": 6, "target": 34963 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }},
    {{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "meshes": [{{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "mode": 4 }}] }}]
}}"#,
                buffer.len()
            ),
        )
        .unwrap();

        dir.join("tri.gltf")
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
}
