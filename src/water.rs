use std::{error::Error, fs, mem::size_of, path::Path};

use sdl3::gpu::{
    Buffer, BufferRegion, BufferUsageFlags, CopyPass, Device, TransferBufferLocation,
    TransferBufferUsage,
};
use tungsten::worldmap::WorldmapManifest;

const WATER_MESH_MAGIC: &[u8; 8] = b"TWMESH1\0";

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct WaterVertex {
    pub(crate) position: [f32; 3],
    pub(crate) normal: [f32; 3],
    pub(crate) uv: [f32; 2],
}

pub(crate) struct WaterMaps {
    pub(crate) ocean: WaterMeshGpu,
    pub(crate) tiles: Vec<WaterTileGpu>,
}

pub(crate) struct WaterTileGpu {
    tile_x: u32,
    tile_y: u32,
    pub(crate) mesh: Option<WaterMeshGpu>,
}

pub(crate) struct WaterMeshGpu {
    pub(crate) vertex_buffer: Buffer,
    pub(crate) index_buffer: Buffer,
    pub(crate) index_count: u32,
}

#[derive(Debug)]
struct WaterMeshCpu {
    vertices: Vec<WaterVertex>,
    indices: Vec<u32>,
}

impl WaterMaps {
    pub(crate) fn load(gpu: &Device, manifest: &WorldmapManifest) -> Result<Self, Box<dyn Error>> {
        let ocean_cpu = WaterMeshCpu::ocean_plane(manifest, manifest.water.ocean_height);
        let copy_commands = gpu.acquire_command_buffer()?;
        let copy_pass = gpu.begin_copy_pass(&copy_commands)?;
        let ocean = WaterMeshGpu::upload(gpu, &copy_pass, &ocean_cpu)?;
        gpu.end_copy_pass(copy_pass);
        copy_commands.submit()?;

        Ok(Self {
            ocean,
            tiles: Vec::new(),
        })
    }

    pub(crate) fn update_tile_cache(
        &mut self,
        gpu: &Device,
        manifest: &WorldmapManifest,
        worldmap_dir: &Path,
        window_min: [u32; 2],
        window_max: [u32; 2],
    ) -> Result<(), Box<dyn Error>> {
        self.tiles.retain(|tile| {
            tile.tile_x >= window_min[0]
                && tile.tile_y >= window_min[1]
                && tile.tile_x <= window_max[0]
                && tile.tile_y <= window_max[1]
        });

        let mut missing_tiles = Vec::new();
        for tile_y in window_min[1]..=window_max[1] {
            for tile_x in window_min[0]..=window_max[0] {
                if !self
                    .tiles
                    .iter()
                    .any(|tile| tile.tile_x == tile_x && tile.tile_y == tile_y)
                {
                    missing_tiles.push([tile_x, tile_y]);
                }
            }
        }

        if missing_tiles.is_empty() {
            return Ok(());
        }

        let copy_commands = gpu.acquire_command_buffer()?;
        let copy_pass = gpu.begin_copy_pass(&copy_commands)?;
        for [tile_x, tile_y] in missing_tiles {
            let path = manifest.water_mesh_tile_path(worldmap_dir, tile_x, tile_y);
            let cpu_mesh = WaterMeshCpu::load(&path)?;
            let mesh = if cpu_mesh.indices.is_empty() {
                None
            } else {
                Some(WaterMeshGpu::upload(gpu, &copy_pass, &cpu_mesh)?)
            };
            self.tiles.push(WaterTileGpu {
                tile_x,
                tile_y,
                mesh,
            });
        }
        gpu.end_copy_pass(copy_pass);
        copy_commands.submit()?;

        Ok(())
    }
}

impl WaterMeshCpu {
    fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let bytes = fs::read(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        parse_water_mesh(&bytes).map_err(|error| format!("{}: {error}", path.display()).into())
    }

    fn ocean_plane(manifest: &WorldmapManifest, ocean_height: f32) -> Self {
        let [width, depth] = manifest.terrain_size();
        let vertices = vec![
            WaterVertex {
                position: [0.0, ocean_height, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, 0.0],
            },
            WaterVertex {
                position: [width, ocean_height, 0.0],
                normal: [0.0, 1.0, 0.0],
                uv: [width, 0.0],
            },
            WaterVertex {
                position: [width, ocean_height, depth],
                normal: [0.0, 1.0, 0.0],
                uv: [width, depth],
            },
            WaterVertex {
                position: [0.0, ocean_height, depth],
                normal: [0.0, 1.0, 0.0],
                uv: [0.0, depth],
            },
        ];
        let indices = vec![0, 1, 2, 0, 2, 3];

        Self { vertices, indices }
    }
}

impl WaterMeshGpu {
    fn upload(
        gpu: &Device,
        copy_pass: &CopyPass,
        cpu: &WaterMeshCpu,
    ) -> Result<Self, Box<dyn Error>> {
        let vertex_buffer =
            create_buffer_with_data(gpu, copy_pass, BufferUsageFlags::VERTEX, &cpu.vertices)?;
        let index_buffer =
            create_buffer_with_data(gpu, copy_pass, BufferUsageFlags::INDEX, &cpu.indices)?;
        let index_count =
            u32::try_from(cpu.indices.len()).map_err(|_| "water mesh index count exceeds u32")?;

        Ok(Self {
            vertex_buffer,
            index_buffer,
            index_count,
        })
    }
}

fn parse_water_mesh(bytes: &[u8]) -> Result<WaterMeshCpu, String> {
    let header_len = WATER_MESH_MAGIC.len() + 8;
    if bytes.len() < header_len {
        return Err("water mesh is shorter than its header".to_owned());
    }
    if &bytes[0..WATER_MESH_MAGIC.len()] != WATER_MESH_MAGIC {
        return Err("water mesh has invalid magic".to_owned());
    }

    let vertex_count_offset = WATER_MESH_MAGIC.len();
    let vertex_count = u32::from_le_bytes([
        bytes[vertex_count_offset],
        bytes[vertex_count_offset + 1],
        bytes[vertex_count_offset + 2],
        bytes[vertex_count_offset + 3],
    ]) as usize;
    let index_count_offset = vertex_count_offset + 4;
    let index_count = u32::from_le_bytes([
        bytes[index_count_offset],
        bytes[index_count_offset + 1],
        bytes[index_count_offset + 2],
        bytes[index_count_offset + 3],
    ]) as usize;
    let vertex_bytes = vertex_count
        .checked_mul(size_of::<WaterVertex>())
        .ok_or("water mesh vertex byte size overflows usize")?;
    let index_bytes = index_count
        .checked_mul(size_of::<u32>())
        .ok_or("water mesh index byte size overflows usize")?;
    let expected_len = header_len
        .checked_add(vertex_bytes)
        .and_then(|len| len.checked_add(index_bytes))
        .ok_or("water mesh byte size overflows usize")?;
    if bytes.len() != expected_len {
        return Err(format!(
            "water mesh has {} bytes, expected {expected_len}",
            bytes.len()
        ));
    }

    let mut cursor = header_len;
    let mut vertices = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        let mut values = [0.0; 8];
        for value in &mut values {
            *value = read_f32(bytes, &mut cursor);
        }
        vertices.push(WaterVertex {
            position: [values[0], values[1], values[2]],
            normal: [values[3], values[4], values[5]],
            uv: [values[6], values[7]],
        });
    }

    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        indices.push(read_u32(bytes, &mut cursor));
    }

    Ok(WaterMeshCpu { vertices, indices })
}

fn read_f32(bytes: &[u8], cursor: &mut usize) -> f32 {
    let value = f32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    value
}

fn read_u32(bytes: &[u8], cursor: &mut usize) -> u32 {
    let value = u32::from_le_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    value
}

fn create_buffer_with_data<T: Copy>(
    gpu: &Device,
    copy_pass: &CopyPass,
    usage: BufferUsageFlags,
    data: &[T],
) -> Result<Buffer, Box<dyn Error>> {
    if data.is_empty() {
        return Err("cannot create an empty water mesh GPU buffer".into());
    }

    let len_bytes = std::mem::size_of_val(data);
    let len_bytes = u32::try_from(len_bytes)
        .map_err(|_| "water mesh buffer is too large for SDL buffer size")?;
    let buffer = gpu
        .create_buffer()
        .with_size(len_bytes)
        .with_usage(usage)
        .build()?;
    let transfer_buffer = gpu
        .create_transfer_buffer()
        .with_size(len_bytes)
        .with_usage(TransferBufferUsage::UPLOAD)
        .build()?;

    let mut map = transfer_buffer.map::<T>(gpu, false);
    map.mem_mut().copy_from_slice(data);
    map.unmap();

    copy_pass.upload_to_gpu_buffer(
        TransferBufferLocation::new()
            .with_offset(0)
            .with_transfer_buffer(&transfer_buffer),
        BufferRegion::new()
            .with_offset(0)
            .with_size(len_bytes)
            .with_buffer(&buffer),
        false,
    );

    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mesh_bytes(vertices: &[WaterVertex], indices: &[u32]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(WATER_MESH_MAGIC);
        bytes.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(indices.len() as u32).to_le_bytes());
        for vertex in vertices {
            for value in vertex
                .position
                .iter()
                .chain(vertex.normal.iter())
                .chain(vertex.uv.iter())
            {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }

        bytes
    }

    #[test]
    fn parses_water_mesh_binary() {
        let vertices = vec![WaterVertex {
            position: [1.0, 2.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            uv: [4.0, 5.0],
        }];
        let indices = vec![0];
        let parsed = parse_water_mesh(&mesh_bytes(&vertices, &indices)).unwrap();

        assert_eq!(parsed.vertices.len(), 1);
        assert_eq!(parsed.indices, indices);
        assert_eq!(parsed.vertices[0].position, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn rejects_invalid_water_mesh_magic() {
        let mut bytes = mesh_bytes(&[], &[]);
        bytes[0] = b'X';

        let error = parse_water_mesh(&bytes).unwrap_err();

        assert!(error.contains("invalid magic"));
    }
}
