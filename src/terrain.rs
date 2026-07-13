use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use sdl3::gpu::{
    Device, Filter, Sampler, SamplerAddressMode, SamplerCreateInfo, SamplerMipmapMode, Texture,
    TextureFormat, TextureUsage,
};

use crate::{
    camera::CameraRay,
    config::AppConfig,
    gpu_upload::{
        create_texture_2d, create_texture_2d_with_pixels, upload_bytes_to_texture_region,
    },
    water::WaterMaps,
    worldmap::{WorldmapManifest, manifest_dir},
};

const MAX_SHADER_TILE_SLOTS: usize = 25;
const R16_BYTES_PER_PIXEL: usize = 2;
const RGBA_BYTES_PER_PIXEL: usize = 4;

pub struct TerrainMaps {
    pub color_near: Texture<'static>,
    pub color_far: Texture<'static>,
    pub height_near_atlas: Texture<'static>,
    pub height_far: Texture<'static>,
    pub color_sampler: Sampler,
    pub height_sampler: Sampler,
    pub terrain_size: [f32; 2],
    pub source_size: [f32; 2],
    pub height_near_atlas_size: [f32; 2],
    pub height_far_size: [f32; 2],
    pub color_far_size: [f32; 2],
    pub tile_size: u32,
    pub tile_padding: u32,
    pub stored_tile_size: u32,
    pub tile_cache_width: u32,
    pub current_window_min: [u32; 2],
    pub current_window_max: [u32; 2],
    pub height_scale: f32,
    pub manifest: WorldmapManifest,
    pub worldmap_dir: PathBuf,
    pub water: WaterMaps,
    tile_cache: TerrainTileCache,
}

impl TerrainMaps {
    pub fn collision_height(&self) -> &HeightField {
        &self.tile_cache.collision_height
    }

    pub fn update_tile_cache_for_position(
        &mut self,
        gpu: &Device,
        world_x: f32,
        world_y: f32,
    ) -> Result<(), Box<dyn Error>> {
        let uploads = self.update_tile_window_for_position(gpu, world_x, world_y)?;
        if uploads == 0 {
            return Ok(());
        }

        Ok(())
    }

    fn update_tile_window_for_position(
        &mut self,
        gpu: &Device,
        world_x: f32,
        world_y: f32,
    ) -> Result<usize, Box<dyn Error>> {
        let [center_x, center_y] = self.tile_for_world_pos(world_x, world_y);
        let (window_min, window_max) = self.tile_window_bounds_for_center(center_x, center_y);

        if self.current_window_min != window_min || self.current_window_max != window_max {
            self.tile_cache
                .collision_height
                .invalidate_tiles_outside(window_min, window_max);
            self.current_window_min = window_min;
            self.current_window_max = window_max;
        }

        let missing_tiles =
            self.missing_tiles_by_priority(center_x, center_y, window_min, window_max);

        if missing_tiles.is_empty() {
            return Ok(0);
        }

        let upload_count = missing_tiles.len();
        let command_buffer = gpu.acquire_command_buffer()?;
        let copy_pass = gpu.begin_copy_pass(&command_buffer)?;
        for [tile_x, tile_y] in missing_tiles {
            self.upload_tile_with_copy_pass(gpu, &copy_pass, tile_x, tile_y)?;
        }
        gpu.end_copy_pass(copy_pass);
        command_buffer.submit()?;

        Ok(upload_count)
    }

    fn upload_tile_with_copy_pass(
        &mut self,
        gpu: &Device,
        copy_pass: &sdl3::gpu::CopyPass,
        tile_x: u32,
        tile_y: u32,
    ) -> Result<(), Box<dyn Error>> {
        let expected_height_bytes =
            checked_square_texture_bytes(self.stored_tile_size, R16_BYTES_PER_PIXEL)?;
        let expected_color_bytes =
            checked_square_texture_bytes(self.stored_tile_size, RGBA_BYTES_PER_PIXEL)?;
        let height_path = self
            .manifest
            .height_tile_path(&self.worldmap_dir, tile_x, tile_y);
        let color_path = self
            .manifest
            .color_tile_path(&self.worldmap_dir, tile_x, tile_y);
        let height_bytes = read_exact_bytes(&height_path, expected_height_bytes)?;
        let color_bytes = read_exact_bytes(&color_path, expected_color_bytes)?;
        let slot_index = slot_index_for_ring_tile(tile_x, tile_y, self.tile_cache_width);
        let [slot_x, slot_y] = slot_xy_for_ring_tile(tile_x, tile_y, self.tile_cache_width);
        let atlas_x = slot_x * self.tile_size;
        let atlas_y = slot_y * self.tile_size;

        upload_padded_tile_payload(
            gpu,
            copy_pass,
            &self.height_near_atlas,
            atlas_x,
            atlas_y,
            self.tile_size,
            self.stored_tile_size,
            self.tile_padding,
            R16_BYTES_PER_PIXEL,
            &height_bytes,
            "height tile",
        )?;
        upload_padded_tile_payload(
            gpu,
            copy_pass,
            &self.color_near,
            atlas_x,
            atlas_y,
            self.tile_size,
            self.stored_tile_size,
            self.tile_padding,
            RGBA_BYTES_PER_PIXEL,
            &color_bytes,
            "color tile",
        )?;
        self.tile_cache
            .collision_height
            .update_slot(slot_index, tile_x, tile_y, &height_bytes)?;

        Ok(())
    }

    fn tile_window_bounds_for_center(&self, center_x: u32, center_y: u32) -> ([u32; 2], [u32; 2]) {
        let radius = (self.tile_cache_width - 1) / 2;
        let min_x = center_x.saturating_sub(radius);
        let min_y = center_y.saturating_sub(radius);
        let max_x = (center_x + radius).min(self.manifest.tile_count_x - 1);
        let max_y = (center_y + radius).min(self.manifest.tile_count_y - 1);

        ([min_x, min_y], [max_x, max_y])
    }

    fn missing_tiles_by_priority(
        &self,
        center_x: u32,
        center_y: u32,
        window_min: [u32; 2],
        window_max: [u32; 2],
    ) -> Vec<[u32; 2]> {
        let mut missing_tiles = Vec::new();
        for tile_y in window_min[1]..=window_max[1] {
            for tile_x in window_min[0]..=window_max[0] {
                let slot_index = slot_index_for_ring_tile(tile_x, tile_y, self.tile_cache_width);
                if !self.tile_cache.collision_height.slots[slot_index]
                    .is_loaded_tile(tile_x, tile_y)
                {
                    missing_tiles.push([tile_x, tile_y]);
                }
            }
        }

        missing_tiles.sort_by_key(|[tile_x, tile_y]| {
            center_x.abs_diff(*tile_x) + center_y.abs_diff(*tile_y)
        });
        missing_tiles
    }

    fn tile_for_world_pos(&self, world_x: f32, world_y: f32) -> [u32; 2] {
        let source_x = (world_x / self.terrain_size[0] * self.manifest.source_width as f32)
            .clamp(0.0, (self.manifest.source_width - 1) as f32);
        let source_y = (world_y / self.terrain_size[1] * self.manifest.source_height as f32)
            .clamp(0.0, (self.manifest.source_height - 1) as f32);

        [
            (source_x as u32 / self.tile_size).min(self.manifest.tile_count_x - 1),
            (source_y as u32 / self.tile_size).min(self.manifest.tile_count_y - 1),
        ]
    }
}

fn slot_xy_for_ring_tile(tile_x: u32, tile_y: u32, tile_cache_width: u32) -> [u32; 2] {
    [tile_x % tile_cache_width, tile_y % tile_cache_width]
}

fn slot_index_for_ring_tile(tile_x: u32, tile_y: u32, tile_cache_width: u32) -> usize {
    let [slot_x, slot_y] = slot_xy_for_ring_tile(tile_x, tile_y, tile_cache_width);
    (slot_y * tile_cache_width + slot_x) as usize
}

pub struct HeightField {
    slots: Vec<TerrainTileSlot>,
    pub tile_size: u32,
    pub tile_padding: u32,
    pub stored_tile_size: u32,
    pub source_size: [u32; 2],
    pub terrain_size: [f32; 2],
    pub height_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainHit {
    pub world_x: f32,
    pub world_y: f32,
    pub height: f32,
    pub distance: f32,
}

impl HeightField {
    fn new(manifest: &WorldmapManifest, tile_cache_width: u32) -> Result<Self, Box<dyn Error>> {
        let stored_tile_size = manifest.stored_tile_size()?;
        let slot_count = checked_tile_slot_count(tile_cache_width)?;

        Ok(Self {
            slots: vec![TerrainTileSlot::empty(); slot_count],
            tile_size: manifest.tile_size,
            tile_padding: manifest.tile_padding,
            stored_tile_size,
            source_size: [manifest.source_width, manifest.source_height],
            terrain_size: manifest.terrain_size(),
            height_scale: manifest.height_scale,
        })
    }

    pub fn height_at(&self, world_x: f32, world_y: f32) -> f32 {
        let sample_x = (world_x / self.terrain_size[0] * self.source_size[0] as f32)
            .clamp(0.0, (self.source_size[0] - 1) as f32);
        let sample_y = (world_y / self.terrain_size[1] * self.source_size[1] as f32)
            .clamp(0.0, (self.source_size[1] - 1) as f32);
        let x0 = sample_x.floor() as u32;
        let y0 = sample_y.floor() as u32;
        let x1 = (x0 + 1).min(self.source_size[0] - 1);
        let y1 = (y0 + 1).min(self.source_size[1] - 1);
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

    pub fn height_at_loaded(&self, world_x: f32, world_y: f32) -> Option<f32> {
        let [sample_x, sample_y] = self.sample_coords(world_x, world_y)?;
        let x0 = sample_x.floor() as u32;
        let y0 = sample_y.floor() as u32;
        let x1 = (x0 + 1).min(self.source_size[0] - 1);
        let y1 = (y0 + 1).min(self.source_size[1] - 1);
        let tx = sample_x - x0 as f32;
        let ty = sample_y - y0 as f32;

        let h00 = self.sample_height_loaded(x0, y0)?;
        let h10 = self.sample_height_loaded(x1, y0)?;
        let h01 = self.sample_height_loaded(x0, y1)?;
        let h11 = self.sample_height_loaded(x1, y1)?;
        let h0 = h00 + (h10 - h00) * tx;
        let h1 = h01 + (h11 - h01) * tx;

        Some(h0 + (h1 - h0) * ty)
    }

    fn sample_coords(&self, world_x: f32, world_y: f32) -> Option<[f32; 2]> {
        if !world_x.is_finite()
            || !world_y.is_finite()
            || world_x < 0.0
            || world_y < 0.0
            || world_x > self.terrain_size[0]
            || world_y > self.terrain_size[1]
        {
            return None;
        }

        Some([
            (world_x / self.terrain_size[0] * self.source_size[0] as f32)
                .clamp(0.0, (self.source_size[0] - 1) as f32),
            (world_y / self.terrain_size[1] * self.source_size[1] as f32)
                .clamp(0.0, (self.source_size[1] - 1) as f32),
        ])
    }

    fn sample_height(&self, x: u32, y: u32) -> f32 {
        self.sample_raw_height(x, y)
            .map(|height| height as f32 / u16::MAX as f32 * self.height_scale)
            .unwrap_or(0.0)
    }

    fn sample_height_loaded(&self, x: u32, y: u32) -> Option<f32> {
        self.sample_raw_height(x, y)
            .map(|height| height as f32 / u16::MAX as f32 * self.height_scale)
    }

    fn sample_raw_height(&self, x: u32, y: u32) -> Option<u16> {
        let tile_x = x / self.tile_size;
        let tile_y = y / self.tile_size;
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.is_loaded_tile(tile_x, tile_y))?;

        let local_x = x - tile_x * self.tile_size + self.tile_padding;
        let local_y = y - tile_y * self.tile_size + self.tile_padding;
        let index = local_y as usize * self.stored_tile_size as usize + local_x as usize;
        slot.height_samples.get(index).copied()
    }

    fn update_slot(
        &mut self,
        slot_index: usize,
        tile_x: u32,
        tile_y: u32,
        height_bytes: &[u8],
    ) -> Result<(), Box<dyn Error>> {
        let expected_bytes =
            self.stored_tile_size as usize * self.stored_tile_size as usize * R16_BYTES_PER_PIXEL;
        if height_bytes.len() != expected_bytes {
            return Err(format!(
                "height tile {tile_x},{tile_y} has {} bytes, expected {expected_bytes}",
                height_bytes.len()
            )
            .into());
        }

        let slot = self
            .slots
            .get_mut(slot_index)
            .ok_or_else(|| format!("tile slot {slot_index} is out of range"))?;
        slot.world_tile_x = tile_x as i32;
        slot.world_tile_y = tile_y as i32;
        slot.loaded = true;
        slot.height_samples.clear();
        slot.height_samples.extend(
            height_bytes
                .chunks_exact(2)
                .map(|sample| u16::from_le_bytes([sample[0], sample[1]])),
        );

        Ok(())
    }

    fn invalidate_tiles_outside(&mut self, window_min: [u32; 2], window_max: [u32; 2]) {
        for slot in &mut self.slots {
            if !slot.loaded {
                continue;
            }
            if slot.world_tile_x < window_min[0] as i32
                || slot.world_tile_y < window_min[1] as i32
                || slot.world_tile_x > window_max[0] as i32
                || slot.world_tile_y > window_max[1] as i32
            {
                slot.loaded = false;
            }
        }
    }
}

pub fn raycast_terrain(
    height_field: &HeightField,
    ray: CameraRay,
    max_distance: f32,
) -> Option<TerrainHit> {
    let direction = ray.direction.normalize_or_zero();
    if direction.length_squared() == 0.0 || max_distance <= 0.0 {
        return None;
    }

    let sample_world_size = (height_field.terrain_size[0] / height_field.source_size[0] as f32)
        .max(height_field.terrain_size[1] / height_field.source_size[1] as f32)
        .max(0.25);
    let max_steps = 8192.0;
    let step = sample_world_size.max(max_distance / max_steps);
    let mut previous = None::<(f32, f32)>;
    let mut distance = 0.0;

    while distance <= max_distance {
        if let Some(delta) = terrain_ray_delta(height_field, ray.origin + direction * distance) {
            if delta <= 0.0 {
                let hit_distance = previous
                    .map(|(previous_distance, _)| {
                        refine_terrain_hit(
                            height_field,
                            ray.origin,
                            direction,
                            previous_distance,
                            distance,
                        )
                    })
                    .unwrap_or(distance);
                let hit_pos = ray.origin + direction * hit_distance;
                let height = height_field.height_at_loaded(hit_pos.x, hit_pos.z)?;
                return Some(TerrainHit {
                    world_x: hit_pos.x,
                    world_y: hit_pos.z,
                    height,
                    distance: hit_distance,
                });
            }
            previous = Some((distance, delta));
        }

        distance += step;
    }

    None
}

fn terrain_ray_delta(height_field: &HeightField, pos: glam::Vec3) -> Option<f32> {
    height_field
        .height_at_loaded(pos.x, pos.z)
        .map(|height| pos.y - height)
}

fn refine_terrain_hit(
    height_field: &HeightField,
    origin: glam::Vec3,
    direction: glam::Vec3,
    mut low: f32,
    mut high: f32,
) -> f32 {
    for _ in 0..12 {
        let mid = (low + high) * 0.5;
        match terrain_ray_delta(height_field, origin + direction * mid) {
            Some(delta) if delta > 0.0 => low = mid,
            Some(_) => high = mid,
            None => break,
        }
    }

    high
}

#[derive(Clone)]
struct TerrainTileSlot {
    world_tile_x: i32,
    world_tile_y: i32,
    loaded: bool,
    height_samples: Vec<u16>,
}

impl TerrainTileSlot {
    fn empty() -> Self {
        Self {
            world_tile_x: -1,
            world_tile_y: -1,
            loaded: false,
            height_samples: Vec::new(),
        }
    }

    fn is_loaded_tile(&self, tile_x: u32, tile_y: u32) -> bool {
        self.loaded && self.world_tile_x == tile_x as i32 && self.world_tile_y == tile_y as i32
    }
}

struct TerrainTileCache {
    collision_height: HeightField,
}

pub fn load_terrain_maps(gpu: &Device, config: &AppConfig) -> Result<TerrainMaps, Box<dyn Error>> {
    let manifest = WorldmapManifest::load(&config.worldmap)?;
    let worldmap_dir = manifest_dir(&config.worldmap)?;
    let stored_tile_size = manifest.stored_tile_size()?;
    let tile_cache_width = config.tile_cache_radius * 2 + 1;
    let atlas_size = checked_atlas_size(tile_cache_width, manifest.tile_size)?;
    let terrain_size = manifest.terrain_size();
    let source_size = [manifest.source_width as f32, manifest.source_height as f32];
    let height_far_size = [
        manifest.height_far_width as f32,
        manifest.height_far_height as f32,
    ];
    let color_far_size = [
        manifest.color_far_width as f32,
        manifest.color_far_height as f32,
    ];

    let copy_commands = gpu.acquire_command_buffer()?;
    let copy_pass = gpu.begin_copy_pass(&copy_commands)?;

    let color_near = create_texture_2d(
        gpu,
        atlas_size,
        atlas_size,
        TextureFormat::R8g8b8a8Unorm,
        TextureUsage::SAMPLER,
    )?;
    let height_near_atlas = create_texture_2d(
        gpu,
        atlas_size,
        atlas_size,
        TextureFormat::R16Unorm,
        TextureUsage::SAMPLER,
    )?;
    let height_far = create_texture_from_r16(
        gpu,
        &copy_pass,
        manifest.height_far_path(&worldmap_dir),
        manifest.height_far_width,
        manifest.height_far_height,
    )?;
    let color_far_pixels = read_rgba_pixels(
        &manifest.color_far_path(&worldmap_dir),
        manifest.color_far_width,
        manifest.color_far_height,
    )?;
    let color_far = create_texture_2d_with_pixels(
        gpu,
        &copy_pass,
        manifest.color_far_width,
        manifest.color_far_height,
        TextureFormat::R8g8b8a8Unorm,
        TextureUsage::SAMPLER,
        &color_far_pixels,
        "far color texture",
    )?;
    let color_sampler = gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::ClampToEdge)
            .with_address_mode_v(SamplerAddressMode::ClampToEdge)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?;
    let height_sampler = gpu.create_sampler(
        SamplerCreateInfo::new()
            .with_min_filter(Filter::Nearest)
            .with_mag_filter(Filter::Nearest)
            .with_mipmap_mode(SamplerMipmapMode::Nearest)
            .with_address_mode_u(SamplerAddressMode::ClampToEdge)
            .with_address_mode_v(SamplerAddressMode::ClampToEdge)
            .with_address_mode_w(SamplerAddressMode::ClampToEdge),
    )?;
    let tile_cache = TerrainTileCache {
        collision_height: HeightField::new(&manifest, tile_cache_width)?,
    };

    gpu.end_copy_pass(copy_pass);
    copy_commands.submit()?;

    let water = WaterMaps::load(gpu, &manifest, &worldmap_dir)?;

    let mut terrain_maps = TerrainMaps {
        color_near,
        color_far,
        height_near_atlas,
        height_far,
        color_sampler,
        height_sampler,
        terrain_size,
        source_size,
        height_near_atlas_size: [atlas_size as f32, atlas_size as f32],
        height_far_size,
        color_far_size,
        tile_size: manifest.tile_size,
        tile_padding: manifest.tile_padding,
        stored_tile_size,
        tile_cache_width,
        current_window_min: [u32::MAX, u32::MAX],
        current_window_max: [u32::MAX, u32::MAX],
        height_scale: manifest.height_scale,
        manifest,
        worldmap_dir,
        water,
        tile_cache,
    };

    terrain_maps.update_tile_window_for_position(gpu, config.start_x, config.start_y)?;

    Ok(terrain_maps)
}

fn create_texture_from_r16(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
) -> Result<Texture<'static>, Box<dyn Error>> {
    let pixels = read_r16_pixels(path.as_ref(), width, height)?;
    create_texture_2d_with_pixels(
        gpu,
        copy_pass,
        width,
        height,
        TextureFormat::R16Unorm,
        TextureUsage::SAMPLER,
        &pixels,
        "far height texture",
    )
}

fn read_r16_pixels(path: &Path, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = fs::read(path)?;
    let expected_size = width as usize * height as usize * R16_BYTES_PER_PIXEL;
    if pixels.len() != expected_size {
        return Err(format!(
            "{} has {} bytes, expected {expected_size} for a {width}x{height} R16 heightmap",
            path.display(),
            pixels.len()
        )
        .into());
    }

    Ok(pixels)
}

fn read_rgba_pixels(path: &Path, width: u32, height: u32) -> Result<Vec<u8>, Box<dyn Error>> {
    let pixels = fs::read(path)?;
    let expected_size = width as usize * height as usize * RGBA_BYTES_PER_PIXEL;
    if pixels.len() != expected_size {
        return Err(format!(
            "{} has {} bytes, expected {expected_size} for a {width}x{height} RGBA8 map",
            path.display(),
            pixels.len()
        )
        .into());
    }

    Ok(pixels)
}

fn read_exact_bytes(path: &Path, expected_size: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    if bytes.len() != expected_size {
        return Err(format!(
            "{} has {} bytes, expected {expected_size}",
            path.display(),
            bytes.len()
        )
        .into());
    }

    Ok(bytes)
}

fn upload_padded_tile_payload(
    gpu: &Device,
    copy_pass: &sdl3::gpu::CopyPass,
    texture: &Texture,
    x: u32,
    y: u32,
    tile_size: u32,
    stored_tile_size: u32,
    tile_padding: u32,
    bytes_per_pixel: usize,
    pixels: &[u8],
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let row_offset_pixels = tile_padding
        .checked_mul(stored_tile_size)
        .and_then(|offset| offset.checked_add(tile_padding))
        .ok_or("tile payload offset overflows u32")?;
    let offset = row_offset_pixels as usize * bytes_per_pixel;
    let offset = u32::try_from(offset).map_err(|_| "tile payload offset overflows u32")?;

    upload_bytes_to_texture_region(
        gpu,
        copy_pass,
        texture,
        pixels,
        offset,
        stored_tile_size,
        stored_tile_size,
        x,
        y,
        tile_size,
        tile_size,
        context,
    )
}

fn checked_atlas_size(tile_cache_width: u32, stored_tile_size: u32) -> Result<u32, Box<dyn Error>> {
    tile_cache_width
        .checked_mul(stored_tile_size)
        .ok_or_else(|| "tile atlas dimensions overflow u32".into())
}

fn checked_tile_slot_count(tile_cache_width: u32) -> Result<usize, Box<dyn Error>> {
    let count = tile_cache_width
        .checked_mul(tile_cache_width)
        .ok_or("tile cache slot count overflows u32")?;
    let count = usize::try_from(count)?;

    if count > MAX_SHADER_TILE_SLOTS {
        return Err(
            format!("tile cache needs {count} slots; max is {MAX_SHADER_TILE_SLOTS}").into(),
        );
    }

    Ok(count)
}

fn checked_square_texture_bytes(
    size: u32,
    bytes_per_pixel: usize,
) -> Result<usize, Box<dyn Error>> {
    (size as usize)
        .checked_mul(size as usize)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| "square texture byte size overflows usize".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_height_field_with_bilinear_filtering() {
        let manifest = WorldmapManifest {
            name: "test".to_owned(),
            source_width: 2,
            source_height: 2,
            horizontal_scale: 1.0,
            height_scale: 100.0,
            tile_size: 2,
            tile_padding: 0,
            tile_count_x: 1,
            tile_count_y: 1,
            height_format: "r16le".to_owned(),
            height_near_path: "height/near".to_owned(),
            height_far_path: "height/far/max_1.r16".to_owned(),
            height_far_width: 1,
            height_far_height: 1,
            color_format: "rgba8".to_owned(),
            color_near_path: "color/near".to_owned(),
            color_far_path: "color/far/overview_1.rgba".to_owned(),
            color_far_width: 1,
            color_far_height: 1,
            props_catalog_path: "props/catalog".to_owned(),
            props_tiles_path: "props/tiles".to_owned(),
            water: crate::worldmap::WaterManifest {
                source_width: 2,
                source_height: 2,
                tile_size_x: 2,
                tile_size_y: 2,
                mesh_format: "wmesh1".to_owned(),
                mesh_path: "water/mesh".to_owned(),
                flow_format: "rg8".to_owned(),
                flow_path: "water/flow".to_owned(),
                ocean_raw_height: 1,
                ocean_height: 1.0,
            },
        };
        let bytes = [
            0_u16.to_le_bytes(),
            u16::MAX.to_le_bytes(),
            u16::MAX.to_le_bytes(),
            0_u16.to_le_bytes(),
        ]
        .concat();
        let mut height_field = HeightField::new(&manifest, 1).unwrap();
        height_field.update_slot(0, 0, 0, &bytes).unwrap();

        assert_eq!(height_field.height_at(0.0, 0.0), 0.0);
        assert!((height_field.height_at(0.5, 0.5) - 50.0).abs() < 0.001);
        assert!((height_field.height_at(2.0, 0.0) - 100.0).abs() < 0.001);
    }
}
