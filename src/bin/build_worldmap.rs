use std::{
    collections::HashMap,
    env,
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use tungsten::worldmap::{
    COLOR_FORMAT_RGBA8, COLOR_NEAR_DIR, HEIGHT_FORMAT_R16LE, HEIGHT_NEAR_DIR, MANIFEST_FILE_NAME,
    WATER_FLOW_DIR, WATER_FLOW_FORMAT_RG8, WATER_MESH_DIR, WATER_MESH_FORMAT_WMESH1, WaterManifest,
    WorldmapManifest, color_tile_file_name, height_tile_file_name, water_flow_tile_file_name,
    water_mesh_tile_file_name,
};

const R16_BYTES_PER_PIXEL: usize = 2;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const RGB_BYTES_PER_PIXEL: usize = 3;
const RG_BYTES_PER_PIXEL: usize = 2;
const DEFAULT_TILE_SIZE: usize = 1024;
const DEFAULT_TILE_PADDING: usize = 2;
const DEFAULT_HORIZONTAL_SCALE: f32 = 0.5;
const DEFAULT_HEIGHT_SCALE: f32 = 255.0 * 2.1;
const OCEAN_HEIGHT_TOLERANCE_WORLD: f32 = 0.1;
const WATER_SKIRT_DROP: f32 = 8.0;
const WATER_SKIRT_TERRAIN_CLEARANCE: f32 = 2.0;
const WATER_MESH_MAGIC: &[u8; 8] = b"TWMESH1\0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    width: usize,
    height: usize,
}

#[derive(Debug, PartialEq)]
struct Args {
    height_input: PathBuf,
    height_size: Size,
    color_input: PathBuf,
    water_height_input: PathBuf,
    water_flow_input: PathBuf,
    output: PathBuf,
    tile_size: usize,
    tile_padding: usize,
    far_height_size: Size,
    far_color_size: Size,
    horizontal_scale: f32,
    height_scale: f32,
    name: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args(env::args_os().skip(1))?;
    let manifest = build_worldmap(&args)?;

    println!(
        "wrote worldmap `{}` to {} ({}x{} source, {}x{} tiles)",
        manifest.name,
        args.output.display(),
        manifest.source_width,
        manifest.source_height,
        manifest.tile_count_x,
        manifest.tile_count_y
    );

    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, Box<dyn Error>> {
    let mut height_input = None;
    let mut height_size = None;
    let mut color_input = None;
    let mut water_height_input = None;
    let mut water_flow_input = None;
    let mut output = None;
    let mut tile_size = None;
    let mut tile_padding = None;
    let mut far_height_size = None;
    let mut far_color_size = None;
    let mut horizontal_scale = None;
    let mut height_scale = None;
    let mut name = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--height-input") => {
                height_input = Some(PathBuf::from(next_value(&mut args, "--height-input")?))
            }
            Some("--height-size") => {
                height_size = Some(parse_size(next_value(&mut args, "--height-size")?)?)
            }
            Some("--color-input") => {
                color_input = Some(PathBuf::from(next_value(&mut args, "--color-input")?))
            }
            Some("--water-height-input") => {
                water_height_input = Some(PathBuf::from(next_value(
                    &mut args,
                    "--water-height-input",
                )?))
            }
            Some("--water-flow-input") => {
                water_flow_input = Some(PathBuf::from(next_value(&mut args, "--water-flow-input")?))
            }
            Some("--output") => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            Some("--tile-size") => {
                tile_size = Some(parse_usize(next_value(&mut args, "--tile-size")?)?)
            }
            Some("--tile-padding") => {
                tile_padding = Some(parse_usize(next_value(&mut args, "--tile-padding")?)?)
            }
            Some("--far-height-size") => {
                far_height_size = Some(parse_size(next_value(&mut args, "--far-height-size")?)?)
            }
            Some("--far-color-size") => {
                far_color_size = Some(parse_size(next_value(&mut args, "--far-color-size")?)?)
            }
            Some("--horizontal-scale") => {
                horizontal_scale = Some(parse_f32(next_value(&mut args, "--horizontal-scale")?)?)
            }
            Some("--height-scale") => {
                height_scale = Some(parse_f32(next_value(&mut args, "--height-scale")?)?)
            }
            Some("--name") => {
                name = Some(parse_string(next_value(&mut args, "--name")?, "--name")?)
            }
            Some("--help" | "-h") => return Err(usage().into()),
            Some(flag) if flag.starts_with('-') => {
                return Err(format!("unknown flag: {flag}").into());
            }
            _ => return Err(usage().into()),
        }
    }

    Ok(Args {
        height_input: height_input.ok_or("--height-input is required")?,
        height_size: height_size.ok_or("--height-size is required")?,
        color_input: color_input.ok_or("--color-input is required")?,
        water_height_input: water_height_input.ok_or("--water-height-input is required")?,
        water_flow_input: water_flow_input.ok_or("--water-flow-input is required")?,
        output: output.ok_or("--output is required")?,
        tile_size: tile_size.unwrap_or(DEFAULT_TILE_SIZE),
        tile_padding: tile_padding.unwrap_or(DEFAULT_TILE_PADDING),
        far_height_size: far_height_size.ok_or("--far-height-size is required")?,
        far_color_size: far_color_size.ok_or("--far-color-size is required")?,
        horizontal_scale: horizontal_scale.unwrap_or(DEFAULT_HORIZONTAL_SCALE),
        height_scale: height_scale.unwrap_or(DEFAULT_HEIGHT_SCALE),
        name,
    })
}

fn next_value(
    args: &mut impl Iterator<Item = OsString>,
    flag: &'static str,
) -> Result<OsString, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("{flag} requires a value").into())
}

fn parse_size(size: OsString) -> Result<Size, Box<dyn Error>> {
    let size = size
        .to_str()
        .ok_or("size must be valid UTF-8 formatted as WIDTHxHEIGHT")?;
    let (width, height) = size
        .split_once('x')
        .or_else(|| size.split_once('X'))
        .ok_or("size must be formatted as WIDTHxHEIGHT")?;
    let width = width.parse::<usize>()?;
    let height = height.parse::<usize>()?;

    if width == 0 || height == 0 {
        return Err("width and height must be greater than zero".into());
    }

    Ok(Size { width, height })
}

fn parse_usize(value: OsString) -> Result<usize, Box<dyn Error>> {
    let value = value
        .to_str()
        .ok_or("integer values must be valid UTF-8")?
        .replace('_', "");
    let parsed = value.parse::<usize>()?;
    if parsed == 0 {
        return Err("integer values must be greater than zero".into());
    }

    Ok(parsed)
}

fn parse_f32(value: OsString) -> Result<f32, Box<dyn Error>> {
    let value = value
        .to_str()
        .ok_or("floating point values must be valid UTF-8")?
        .replace('_', "");
    let parsed = value.parse::<f32>()?;
    if !parsed.is_finite() || parsed <= 0.0 {
        return Err("scale values must be finite and greater than zero".into());
    }

    Ok(parsed)
}

fn parse_string(value: OsString, flag: &'static str) -> Result<String, Box<dyn Error>> {
    let value = value
        .to_str()
        .ok_or_else(|| format!("{flag} must be valid UTF-8"))?
        .to_owned();
    if value.is_empty() {
        return Err(format!("{flag} must not be empty").into());
    }

    Ok(value)
}

fn build_worldmap(args: &Args) -> Result<WorldmapManifest, Box<dyn Error>> {
    validate_args(args)?;

    let height_bytes = fs::read(&args.height_input)?;
    let expected_height_bytes = checked_r16_bytes(&args.height_size)?;
    if height_bytes.len() != expected_height_bytes {
        return Err(format!(
            "{} has {} bytes, expected {expected_height_bytes} for {}x{} R16",
            args.height_input.display(),
            height_bytes.len(),
            args.height_size.width,
            args.height_size.height
        )
        .into());
    }

    let mut color_reader = image::ImageReader::open(&args.color_input)?;
    color_reader.no_limits();
    let color = color_reader.decode()?.to_rgba8();
    let color_size = Size {
        width: color.width() as usize,
        height: color.height() as usize,
    };
    if color_size != args.height_size {
        return Err(format!(
            "{} is {}x{}, expected {}x{} to match height input",
            args.color_input.display(),
            color_size.width,
            color_size.height,
            args.height_size.width,
            args.height_size.height
        )
        .into());
    }

    let tile_count_x = args.height_size.width / args.tile_size;
    let tile_count_y = args.height_size.height / args.tile_size;
    let water_source = load_water_source(args, tile_count_x, tile_count_y)?;
    let height_far_path = far_height_relative_path(&args.far_height_size);
    let color_far_path = far_color_relative_path(&args.far_color_size);
    let water_manifest = WaterManifest {
        source_width: water_source.size.width as u32,
        source_height: water_source.size.height as u32,
        tile_size_x: (water_source.size.width / tile_count_x) as u32,
        tile_size_y: (water_source.size.height / tile_count_y) as u32,
        mesh_format: WATER_MESH_FORMAT_WMESH1.to_owned(),
        mesh_path: WATER_MESH_DIR.to_owned(),
        flow_format: WATER_FLOW_FORMAT_RG8.to_owned(),
        flow_path: WATER_FLOW_DIR.to_owned(),
        ocean_raw_height: water_source.ocean_raw_height as u32,
        ocean_height: water_height_to_world(water_source.ocean_raw_height, args.height_scale),
    };
    let manifest = WorldmapManifest {
        name: args
            .name
            .clone()
            .unwrap_or_else(|| default_worldmap_name(&args.output)),
        source_width: u32::try_from(args.height_size.width)?,
        source_height: u32::try_from(args.height_size.height)?,
        horizontal_scale: args.horizontal_scale,
        height_scale: args.height_scale,
        tile_size: u32::try_from(args.tile_size)?,
        tile_padding: u32::try_from(args.tile_padding)?,
        tile_count_x: u32::try_from(tile_count_x)?,
        tile_count_y: u32::try_from(tile_count_y)?,
        height_format: HEIGHT_FORMAT_R16LE.to_owned(),
        height_near_path: HEIGHT_NEAR_DIR.to_owned(),
        height_far_path,
        height_far_width: u32::try_from(args.far_height_size.width)?,
        height_far_height: u32::try_from(args.far_height_size.height)?,
        color_format: COLOR_FORMAT_RGBA8.to_owned(),
        color_near_path: COLOR_NEAR_DIR.to_owned(),
        color_far_path,
        color_far_width: u32::try_from(args.far_color_size.width)?,
        color_far_height: u32::try_from(args.far_color_size.height)?,
        water: water_manifest,
    }
    .validate()?;

    fs::create_dir_all(&args.output)?;
    write_height_tiles(&height_bytes, &args.height_size, args, &manifest)?;
    write_color_tiles(color.as_raw(), &args.height_size, args, &manifest)?;

    let far_height = max_pool_r16(&height_bytes, &args.height_size, &args.far_height_size);
    write_output(args.output.join(&manifest.height_far_path), &far_height)?;

    let far_color = box_downsample_rgba(color.as_raw(), &args.height_size, &args.far_color_size);
    write_output(args.output.join(&manifest.color_far_path), &far_color)?;

    write_water_tiles(
        &height_bytes,
        &args.height_size,
        &water_source,
        args,
        &manifest,
    )?;

    manifest.write_to(args.output.join(MANIFEST_FILE_NAME))?;

    Ok(manifest)
}

fn validate_args(args: &Args) -> Result<(), Box<dyn Error>> {
    checked_r16_bytes(&args.height_size)?;
    checked_rgba_bytes(&args.height_size)?;
    checked_r16_bytes(&args.far_height_size)?;
    checked_rgba_bytes(&args.far_color_size)?;

    if args.height_size.width % args.tile_size != 0 || args.height_size.height % args.tile_size != 0
    {
        return Err(format!(
            "height size {}x{} must be evenly divisible by tile size {}",
            args.height_size.width, args.height_size.height, args.tile_size
        )
        .into());
    }
    if args.far_height_size.width > args.height_size.width
        || args.far_height_size.height > args.height_size.height
    {
        return Err("far height size must be no larger than the source size".into());
    }
    if args.far_color_size.width > args.height_size.width
        || args.far_color_size.height > args.height_size.height
    {
        return Err("far color size must be no larger than the source size".into());
    }
    if args.height_size.width % args.far_height_size.width != 0
        || args.height_size.height % args.far_height_size.height != 0
    {
        return Err("source height size must be evenly divisible by far height size".into());
    }
    if args.height_size.width % args.far_color_size.width != 0
        || args.height_size.height % args.far_color_size.height != 0
    {
        return Err("source color size must be evenly divisible by far color size".into());
    }
    Ok(())
}

struct WaterSource {
    height: Vec<u16>,
    flow_rgb: Vec<u8>,
    size: Size,
    ocean_raw_height: u16,
    ocean_tolerance_raw: u16,
}

#[derive(Clone, Copy)]
struct WaterMeshVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

struct WaterMesh {
    vertices: Vec<WaterMeshVertex>,
    indices: Vec<u32>,
}

fn load_water_source(
    args: &Args,
    tile_count_x: usize,
    tile_count_y: usize,
) -> Result<WaterSource, Box<dyn Error>> {
    let height_path = &args.water_height_input;
    let flow_path = &args.water_flow_input;
    let (height, size) = read_water_height_png(height_path)?;
    let (flow_rgb, flow_size) = read_water_flow_png(flow_path)?;

    if flow_size != size {
        return Err(format!(
            "{} is {}x{}, expected {}x{} to match water height input",
            flow_path.display(),
            flow_size.width,
            flow_size.height,
            size.width,
            size.height
        )
        .into());
    }
    if size.width % tile_count_x != 0 || size.height % tile_count_y != 0 {
        return Err("water dimensions must be evenly divisible by terrain tile counts".into());
    }

    let ocean_raw_height = detect_ocean_raw_height(&height, &size)?;
    let ocean_tolerance_raw = ocean_tolerance_raw(args.height_scale);

    Ok(WaterSource {
        height,
        flow_rgb,
        size,
        ocean_raw_height,
        ocean_tolerance_raw,
    })
}

fn read_water_height_png(path: &Path) -> Result<(Vec<u16>, Size), Box<dyn Error>> {
    let mut reader = image::ImageReader::open(path).map_err(|error| {
        format!(
            "failed to open water height PNG {}: {error}",
            path.display()
        )
    })?;
    reader.no_limits();
    let image = reader
        .decode()
        .map_err(|error| {
            format!(
                "failed to decode water height PNG {}: {error}",
                path.display()
            )
        })?
        .to_luma16();
    let size = Size {
        width: image.width() as usize,
        height: image.height() as usize,
    };

    Ok((image.into_raw(), size))
}

fn read_water_flow_png(path: &Path) -> Result<(Vec<u8>, Size), Box<dyn Error>> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| format!("failed to open water flow PNG {}: {error}", path.display()))?;
    reader.no_limits();
    let image = reader
        .decode()
        .map_err(|error| {
            format!(
                "failed to decode water flow PNG {}: {error}",
                path.display()
            )
        })?
        .to_rgb8();
    let size = Size {
        width: image.width() as usize,
        height: image.height() as usize,
    };

    Ok((image.into_raw(), size))
}

fn detect_ocean_raw_height(water_height: &[u16], size: &Size) -> Result<u16, Box<dyn Error>> {
    let mut counts = HashMap::<u16, usize>::new();

    for x in 0..size.width {
        count_border_water(water_height, size.width, x, 0, &mut counts);
        count_border_water(water_height, size.width, x, size.height - 1, &mut counts);
    }
    for y in 1..size.height.saturating_sub(1) {
        count_border_water(water_height, size.width, 0, y, &mut counts);
        count_border_water(water_height, size.width, size.width - 1, y, &mut counts);
    }

    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(height, _)| height)
        .ok_or_else(|| "water height map has no non-zero border water for ocean detection".into())
}

fn count_border_water(
    water_height: &[u16],
    width: usize,
    x: usize,
    y: usize,
    counts: &mut HashMap<u16, usize>,
) {
    let height = water_height[y * width + x];
    if height > 0 {
        *counts.entry(height).or_insert(0) += 1;
    }
}

fn ocean_tolerance_raw(height_scale: f32) -> u16 {
    ((OCEAN_HEIGHT_TOLERANCE_WORLD / height_scale) * u16::MAX as f32)
        .ceil()
        .max(1.0) as u16
}

fn write_water_tiles(
    terrain_height: &[u8],
    terrain_size: &Size,
    water: &WaterSource,
    args: &Args,
    manifest: &WorldmapManifest,
) -> Result<(), Box<dyn Error>> {
    let mesh_dir = args.output.join(WATER_MESH_DIR);
    let flow_dir = args.output.join(WATER_FLOW_DIR);
    fs::create_dir_all(&mesh_dir)?;
    fs::create_dir_all(&flow_dir)?;

    for tile_y in 0..manifest.tile_count_y as usize {
        for tile_x in 0..manifest.tile_count_x as usize {
            let mesh =
                build_water_tile_mesh(terrain_height, terrain_size, water, args, tile_x, tile_y);
            write_water_mesh(
                mesh_dir.join(water_mesh_tile_file_name(
                    u32::try_from(tile_x)?,
                    u32::try_from(tile_y)?,
                )),
                &mesh,
            )?;
            let flow = water_flow_tile(water, manifest, tile_x, tile_y);
            write_output(
                flow_dir.join(water_flow_tile_file_name(
                    u32::try_from(tile_x)?,
                    u32::try_from(tile_y)?,
                )),
                &flow,
            )?;
        }
    }

    Ok(())
}

fn build_water_tile_mesh(
    terrain_height: &[u8],
    terrain_size: &Size,
    water: &WaterSource,
    args: &Args,
    tile_x: usize,
    tile_y: usize,
) -> WaterMesh {
    let water_tile_width = water.size.width / (args.height_size.width / args.tile_size);
    let water_tile_height = water.size.height / (args.height_size.height / args.tile_size);
    let start_x = tile_x * water_tile_width;
    let start_y = tile_y * water_tile_height;
    let end_x = start_x + water_tile_width;
    let end_y = start_y + water_tile_height;
    let mut mesh = WaterMesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };

    for y in start_y..end_y {
        for x in start_x..end_x {
            if !is_mesh_water_cell(water, x, y) {
                continue;
            }

            add_water_cell(&mut mesh, terrain_height, terrain_size, water, args, x, y);
        }
    }

    mesh
}

fn add_water_cell(
    mesh: &mut WaterMesh,
    terrain_height: &[u8],
    terrain_size: &Size,
    water: &WaterSource,
    args: &Args,
    x: usize,
    y: usize,
) {
    let h00 = water_height_at_or_cell(water, x, y, x, y);
    let h10 = water_height_at_or_cell(water, x + 1, y, x, y);
    let h11 = water_height_at_or_cell(water, x + 1, y + 1, x, y);
    let h01 = water_height_at_or_cell(water, x, y + 1, x, y);
    let p00 = water_top_position(water, args, x, y, h00);
    let p10 = water_top_position(water, args, x + 1, y, h10);
    let p11 = water_top_position(water, args, x + 1, y + 1, h11);
    let p01 = water_top_position(water, args, x, y + 1, h01);

    add_quad(mesh, p00, p10, p11, p01);

    if x == 0 || !is_mesh_water_cell(water, x - 1, y) {
        add_water_skirt(mesh, terrain_height, terrain_size, args, p01, p00);
    }
    if !is_mesh_water_cell(water, x + 1, y) {
        add_water_skirt(mesh, terrain_height, terrain_size, args, p10, p11);
    }
    if y == 0 || !is_mesh_water_cell(water, x, y - 1) {
        add_water_skirt(mesh, terrain_height, terrain_size, args, p00, p10);
    }
    if !is_mesh_water_cell(water, x, y + 1) {
        add_water_skirt(mesh, terrain_height, terrain_size, args, p11, p01);
    }
}

fn add_water_skirt(
    mesh: &mut WaterMesh,
    terrain_height: &[u8],
    terrain_size: &Size,
    args: &Args,
    a: [f32; 3],
    b: [f32; 3],
) {
    let bottom_a = water_skirt_bottom(terrain_height, terrain_size, args, a);
    let bottom_b = water_skirt_bottom(terrain_height, terrain_size, args, b);
    add_quad(mesh, a, b, [b[0], bottom_b, b[2]], [a[0], bottom_a, a[2]]);
}

fn water_skirt_bottom(
    terrain_height: &[u8],
    terrain_size: &Size,
    args: &Args,
    point: [f32; 3],
) -> f32 {
    let terrain =
        sample_terrain_height_world(terrain_height, terrain_size, args, point[0], point[2])
            - WATER_SKIRT_TERRAIN_CLEARANCE;
    let water = point[1] - WATER_SKIRT_DROP;

    terrain.min(water).max(0.0)
}

fn add_quad(mesh: &mut WaterMesh, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) {
    let start = mesh.vertices.len() as u32;
    for position in [a, b, c, d] {
        mesh.vertices.push(WaterMeshVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [position[0], position[2]],
        });
    }
    mesh.indices
        .extend_from_slice(&[start, start + 1, start + 2, start, start + 2, start + 3]);
}

fn is_mesh_water_cell(water: &WaterSource, x: usize, y: usize) -> bool {
    if x >= water.size.width || y >= water.size.height {
        return false;
    }

    let raw = water.height[y * water.size.width + x];
    raw > 0 && !is_ocean_raw(water, raw)
}

fn is_ocean_raw(water: &WaterSource, raw: u16) -> bool {
    raw.abs_diff(water.ocean_raw_height) <= water.ocean_tolerance_raw
}

fn water_height_at_or_cell(
    water: &WaterSource,
    x: usize,
    y: usize,
    cell_x: usize,
    cell_y: usize,
) -> u16 {
    let sample_x = x.min(water.size.width - 1);
    let sample_y = y.min(water.size.height - 1);
    let raw = water.height[sample_y * water.size.width + sample_x];
    if raw > 0 {
        raw
    } else {
        water.height[cell_y * water.size.width + cell_x]
    }
}

fn water_top_position(
    water: &WaterSource,
    args: &Args,
    x: usize,
    y: usize,
    raw_height: u16,
) -> [f32; 3] {
    let terrain_width = args.height_size.width as f32 * args.horizontal_scale;
    let terrain_depth = args.height_size.height as f32 * args.horizontal_scale;

    [
        x as f32 / water.size.width as f32 * terrain_width,
        water_height_to_world(raw_height, args.height_scale),
        y as f32 / water.size.height as f32 * terrain_depth,
    ]
}

fn water_height_to_world(raw_height: u16, height_scale: f32) -> f32 {
    raw_height as f32 / u16::MAX as f32 * height_scale
}

fn sample_terrain_height_world(
    terrain_height: &[u8],
    terrain_size: &Size,
    args: &Args,
    world_x: f32,
    world_y: f32,
) -> f32 {
    let terrain_width = args.height_size.width as f32 * args.horizontal_scale;
    let terrain_depth = args.height_size.height as f32 * args.horizontal_scale;
    let sample_x = (world_x / terrain_width * terrain_size.width as f32)
        .clamp(0.0, (terrain_size.width - 1) as f32);
    let sample_y = (world_y / terrain_depth * terrain_size.height as f32)
        .clamp(0.0, (terrain_size.height - 1) as f32);
    let x0 = sample_x.floor() as usize;
    let y0 = sample_y.floor() as usize;
    let x1 = (x0 + 1).min(terrain_size.width - 1);
    let y1 = (y0 + 1).min(terrain_size.height - 1);
    let tx = sample_x - x0 as f32;
    let ty = sample_y - y0 as f32;
    let h00 = read_r16(terrain_height, terrain_size.width, x0, y0) as f32;
    let h10 = read_r16(terrain_height, terrain_size.width, x1, y0) as f32;
    let h01 = read_r16(terrain_height, terrain_size.width, x0, y1) as f32;
    let h11 = read_r16(terrain_height, terrain_size.width, x1, y1) as f32;
    let h0 = h00 + (h10 - h00) * tx;
    let h1 = h01 + (h11 - h01) * tx;

    (h0 + (h1 - h0) * ty) / u16::MAX as f32 * args.height_scale
}

fn water_flow_tile(
    water: &WaterSource,
    manifest: &WorldmapManifest,
    tile_x: usize,
    tile_y: usize,
) -> Vec<u8> {
    let water_tile_width = water.size.width / manifest.tile_count_x as usize;
    let water_tile_height = water.size.height / manifest.tile_count_y as usize;
    let start_x = tile_x * water_tile_width;
    let start_y = tile_y * water_tile_height;
    let mut output = vec![0; water_tile_width * water_tile_height * RG_BYTES_PER_PIXEL];

    for y in 0..water_tile_height {
        for x in 0..water_tile_width {
            let source_index =
                ((start_y + y) * water.size.width + start_x + x) * RGB_BYTES_PER_PIXEL;
            let output_index = (y * water_tile_width + x) * RG_BYTES_PER_PIXEL;
            output[output_index] = water.flow_rgb[source_index];
            output[output_index + 1] = water.flow_rgb[source_index + 1];
        }
    }

    output
}

fn write_water_mesh(path: impl AsRef<Path>, mesh: &WaterMesh) -> Result<(), Box<dyn Error>> {
    let vertex_count =
        u32::try_from(mesh.vertices.len()).map_err(|_| "water mesh vertex count exceeds u32")?;
    let index_count =
        u32::try_from(mesh.indices.len()).map_err(|_| "water mesh index count exceeds u32")?;
    let mut bytes = Vec::with_capacity(
        WATER_MESH_MAGIC.len()
            + 8
            + mesh.vertices.len() * 32
            + mesh.indices.len() * std::mem::size_of::<u32>(),
    );
    bytes.extend_from_slice(WATER_MESH_MAGIC);
    bytes.extend_from_slice(&vertex_count.to_le_bytes());
    bytes.extend_from_slice(&index_count.to_le_bytes());
    for vertex in &mesh.vertices {
        for value in vertex
            .position
            .iter()
            .chain(vertex.normal.iter())
            .chain(vertex.uv.iter())
        {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for index in &mesh.indices {
        bytes.extend_from_slice(&index.to_le_bytes());
    }

    write_output(path, &bytes)
}

fn write_height_tiles(
    height_bytes: &[u8],
    source_size: &Size,
    args: &Args,
    manifest: &WorldmapManifest,
) -> Result<(), Box<dyn Error>> {
    let dir = args.output.join(&manifest.height_near_path);
    fs::create_dir_all(&dir)?;

    for tile_y in 0..manifest.tile_count_y as usize {
        for tile_x in 0..manifest.tile_count_x as usize {
            let tile = padded_r16_tile(
                height_bytes,
                source_size,
                tile_x,
                tile_y,
                args.tile_size,
                args.tile_padding,
            );
            write_output(
                dir.join(height_tile_file_name(
                    u32::try_from(tile_x)?,
                    u32::try_from(tile_y)?,
                )),
                &tile,
            )?;
        }
    }

    Ok(())
}

fn write_color_tiles(
    color_bytes: &[u8],
    source_size: &Size,
    args: &Args,
    manifest: &WorldmapManifest,
) -> Result<(), Box<dyn Error>> {
    let dir = args.output.join(&manifest.color_near_path);
    fs::create_dir_all(&dir)?;

    for tile_y in 0..manifest.tile_count_y as usize {
        for tile_x in 0..manifest.tile_count_x as usize {
            let tile = padded_rgba_tile(
                color_bytes,
                source_size,
                tile_x,
                tile_y,
                args.tile_size,
                args.tile_padding,
            );
            write_output(
                dir.join(color_tile_file_name(
                    u32::try_from(tile_x)?,
                    u32::try_from(tile_y)?,
                )),
                &tile,
            )?;
        }
    }

    Ok(())
}

fn padded_r16_tile(
    input: &[u8],
    source_size: &Size,
    tile_x: usize,
    tile_y: usize,
    tile_size: usize,
    padding: usize,
) -> Vec<u8> {
    let stored_size = tile_size + padding * 2;
    let mut output = vec![0; stored_size * stored_size * R16_BYTES_PER_PIXEL];

    for out_y in 0..stored_size {
        let source_y = padded_source_coord(tile_y, tile_size, padding, out_y, source_size.height);
        for out_x in 0..stored_size {
            let source_x =
                padded_source_coord(tile_x, tile_size, padding, out_x, source_size.width);
            let source_index = r16_index(source_size.width, source_x, source_y);
            let output_index = r16_index(stored_size, out_x, out_y);
            output[output_index..output_index + R16_BYTES_PER_PIXEL]
                .copy_from_slice(&input[source_index..source_index + R16_BYTES_PER_PIXEL]);
        }
    }

    output
}

fn padded_rgba_tile(
    input: &[u8],
    source_size: &Size,
    tile_x: usize,
    tile_y: usize,
    tile_size: usize,
    padding: usize,
) -> Vec<u8> {
    let stored_size = tile_size + padding * 2;
    let mut output = vec![0; stored_size * stored_size * RGBA_BYTES_PER_PIXEL];

    for out_y in 0..stored_size {
        let source_y = padded_source_coord(tile_y, tile_size, padding, out_y, source_size.height);
        for out_x in 0..stored_size {
            let source_x =
                padded_source_coord(tile_x, tile_size, padding, out_x, source_size.width);
            let source_index = rgba_index(source_size.width, source_x, source_y);
            let output_index = rgba_index(stored_size, out_x, out_y);
            output[output_index..output_index + RGBA_BYTES_PER_PIXEL]
                .copy_from_slice(&input[source_index..source_index + RGBA_BYTES_PER_PIXEL]);
        }
    }

    output
}

fn padded_source_coord(
    tile_index: usize,
    tile_size: usize,
    padding: usize,
    output_coord: usize,
    source_limit: usize,
) -> usize {
    let coord = tile_index as i64 * tile_size as i64 + output_coord as i64 - padding as i64;
    coord.clamp(0, source_limit as i64 - 1) as usize
}

fn max_pool_r16(input: &[u8], source_size: &Size, output_size: &Size) -> Vec<u8> {
    let scale_x = source_size.width / output_size.width;
    let scale_y = source_size.height / output_size.height;
    let mut output = vec![0; output_size.width * output_size.height * R16_BYTES_PER_PIXEL];

    for out_y in 0..output_size.height {
        for out_x in 0..output_size.width {
            let mut max_height = 0;
            for dy in 0..scale_y {
                let source_y = out_y * scale_y + dy;
                for dx in 0..scale_x {
                    let source_x = out_x * scale_x + dx;
                    max_height =
                        max_height.max(read_r16(input, source_size.width, source_x, source_y));
                }
            }

            let output_index = r16_index(output_size.width, out_x, out_y);
            output[output_index..output_index + R16_BYTES_PER_PIXEL]
                .copy_from_slice(&max_height.to_le_bytes());
        }
    }

    output
}

fn box_downsample_rgba(input: &[u8], source_size: &Size, output_size: &Size) -> Vec<u8> {
    let scale_x = source_size.width / output_size.width;
    let scale_y = source_size.height / output_size.height;
    let divisor = (scale_x * scale_y) as u32;
    let mut output = vec![0; output_size.width * output_size.height * RGBA_BYTES_PER_PIXEL];

    for out_y in 0..output_size.height {
        for out_x in 0..output_size.width {
            let mut sums = [0_u32; RGBA_BYTES_PER_PIXEL];
            for dy in 0..scale_y {
                let source_y = out_y * scale_y + dy;
                for dx in 0..scale_x {
                    let source_x = out_x * scale_x + dx;
                    let source_index = rgba_index(source_size.width, source_x, source_y);
                    for channel in 0..RGBA_BYTES_PER_PIXEL {
                        sums[channel] += input[source_index + channel] as u32;
                    }
                }
            }

            let output_index = rgba_index(output_size.width, out_x, out_y);
            for channel in 0..RGBA_BYTES_PER_PIXEL {
                output[output_index + channel] = ((sums[channel] + divisor / 2) / divisor) as u8;
            }
        }
    }

    output
}

fn read_r16(input: &[u8], width: usize, x: usize, y: usize) -> u16 {
    let index = r16_index(width, x, y);
    u16::from_le_bytes([input[index], input[index + 1]])
}

fn r16_index(width: usize, x: usize, y: usize) -> usize {
    (y * width + x) * R16_BYTES_PER_PIXEL
}

fn rgba_index(width: usize, x: usize, y: usize) -> usize {
    (y * width + x) * RGBA_BYTES_PER_PIXEL
}

fn checked_r16_bytes(size: &Size) -> Result<usize, Box<dyn Error>> {
    size.width
        .checked_mul(size.height)
        .and_then(|pixels| pixels.checked_mul(R16_BYTES_PER_PIXEL))
        .ok_or_else(|| "R16 dimensions overflow usize".into())
}

fn checked_rgba_bytes(size: &Size) -> Result<usize, Box<dyn Error>> {
    size.width
        .checked_mul(size.height)
        .and_then(|pixels| pixels.checked_mul(RGBA_BYTES_PER_PIXEL))
        .ok_or_else(|| "RGBA dimensions overflow usize".into())
}

fn write_output(path: impl AsRef<Path>, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(path, bytes)?;
    Ok(())
}

fn far_height_relative_path(size: &Size) -> String {
    if size.width == size.height {
        format!("height/far/max_{}.r16", size.width)
    } else {
        format!("height/far/max_{}x{}.r16", size.width, size.height)
    }
}

fn far_color_relative_path(size: &Size) -> String {
    if size.width == size.height {
        format!("color/far/overview_{}.rgba", size.width)
    } else {
        format!("color/far/overview_{}x{}.rgba", size.width, size.height)
    }
}

fn default_worldmap_name(output: &Path) -> String {
    output
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("worldmap")
        .to_owned()
}

fn usage() -> &'static str {
    "usage: cargo run --release --bin build_worldmap -- \\
        --height-input <source.r16> --height-size <WIDTHxHEIGHT> \\
        --color-input <source.png> --output <assets/worldmaps/name> \\
        --far-height-size <WIDTHxHEIGHT> --far-color-size <WIDTHxHEIGHT> \\
        --water-height-input <water.png> --water-flow-input <flow.png> \\
        [--tile-size 1024] [--tile-padding 2] \\
        [--horizontal-scale 0.5] [--height-scale 535.5] [--name <name>]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma, RgbImage, RgbaImage};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn os_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        env::temp_dir().join(format!("tungsten_{name}_{}_{}", std::process::id(), millis))
    }

    fn r16_bytes(values: &[u16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn write_luma16_png(path: &Path, width: u32, height: u32, values: Vec<u16>) {
        let image = ImageBuffer::<Luma<u16>, Vec<u16>>::from_raw(width, height, values).unwrap();
        image.save(path).unwrap();
    }

    fn water_mesh_counts(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[0..WATER_MESH_MAGIC.len()], WATER_MESH_MAGIC);
        let vertex_count = u32::from_le_bytes([
            bytes[WATER_MESH_MAGIC.len()],
            bytes[WATER_MESH_MAGIC.len() + 1],
            bytes[WATER_MESH_MAGIC.len() + 2],
            bytes[WATER_MESH_MAGIC.len() + 3],
        ]);
        let index_count = u32::from_le_bytes([
            bytes[WATER_MESH_MAGIC.len() + 4],
            bytes[WATER_MESH_MAGIC.len() + 5],
            bytes[WATER_MESH_MAGIC.len() + 6],
            bytes[WATER_MESH_MAGIC.len() + 7],
        ]);

        (vertex_count, index_count)
    }

    #[test]
    fn parses_args_with_defaults() {
        let args = parse_args(os_args(&[
            "--height-input",
            "height.r16",
            "--height-size",
            "16384x16384",
            "--color-input",
            "color.png",
            "--water-height-input",
            "water.png",
            "--water-flow-input",
            "flow.png",
            "--output",
            "assets/worldmaps/continent",
            "--far-height-size",
            "2048x2048",
            "--far-color-size",
            "4096x4096",
        ]))
        .unwrap();

        assert_eq!(args.tile_size, DEFAULT_TILE_SIZE);
        assert_eq!(args.tile_padding, DEFAULT_TILE_PADDING);
        assert_eq!(args.horizontal_scale, DEFAULT_HORIZONTAL_SCALE);
        assert_eq!(args.height_scale, DEFAULT_HEIGHT_SCALE);
        assert_eq!(args.water_height_input, PathBuf::from("water.png"));
        assert_eq!(args.water_flow_input, PathBuf::from("flow.png"));
    }

    #[test]
    fn rejects_missing_water_flow_input() {
        let error = parse_args(os_args(&[
            "--height-input",
            "height.r16",
            "--height-size",
            "4x4",
            "--color-input",
            "color.png",
            "--water-height-input",
            "water.png",
            "--output",
            "world",
            "--far-height-size",
            "2x2",
            "--far-color-size",
            "2x2",
        ]))
        .unwrap_err();

        assert!(error.to_string().contains("--water-flow-input is required"));
    }

    #[test]
    fn padded_r16_tile_clamps_world_edges() {
        let input = r16_bytes(&[
            1, 2, 3, 4, //
            5, 6, 7, 8, //
            9, 10, 11, 12, //
            13, 14, 15, 16,
        ]);
        let tile = padded_r16_tile(
            &input,
            &Size {
                width: 4,
                height: 4,
            },
            0,
            0,
            2,
            1,
        );
        let heights = tile
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();

        assert_eq!(
            heights,
            vec![
                1, 1, 2, 3, //
                1, 1, 2, 3, //
                5, 5, 6, 7, //
                9, 9, 10, 11,
            ]
        );
    }

    #[test]
    fn builds_small_worldmap_package() {
        let dir = unique_temp_dir("build_worldmap");
        let height_path = dir.join("source.r16");
        let color_path = dir.join("source.png");
        let water_height_path = dir.join("water.png");
        let flow_path = dir.join("flow.png");
        let output = dir.join("world");
        fs::create_dir_all(&dir).unwrap();

        fs::write(
            &height_path,
            r16_bytes(&[
                1, 2, 3, 4, //
                5, 6, 7, 8, //
                9, 10, 11, 12, //
                13, 14, 15, 16,
            ]),
        )
        .unwrap();
        let color = RgbaImage::from_raw(
            4,
            4,
            vec![
                0, 1, 2, 255, 4, 5, 6, 255, 8, 9, 10, 255, 12, 13, 14, 255, //
                16, 17, 18, 255, 20, 21, 22, 255, 24, 25, 26, 255, 28, 29, 30, 255, //
                32, 33, 34, 255, 36, 37, 38, 255, 40, 41, 42, 255, 44, 45, 46, 255, //
                48, 49, 50, 255, 52, 53, 54, 255, 56, 57, 58, 255, 60, 61, 62, 255,
            ],
        )
        .unwrap();
        color.save(&color_path).unwrap();
        write_luma16_png(&water_height_path, 4, 4, vec![100; 4 * 4]);
        RgbImage::from_raw(4, 4, vec![127; 4 * 4 * RGB_BYTES_PER_PIXEL])
            .unwrap()
            .save(&flow_path)
            .unwrap();

        let args = Args {
            height_input: height_path,
            height_size: Size {
                width: 4,
                height: 4,
            },
            color_input: color_path,
            water_height_input: water_height_path,
            water_flow_input: flow_path,
            output: output.clone(),
            tile_size: 2,
            tile_padding: 1,
            far_height_size: Size {
                width: 2,
                height: 2,
            },
            far_color_size: Size {
                width: 2,
                height: 2,
            },
            horizontal_scale: 0.5,
            height_scale: 10.0,
            name: Some("test-world".to_owned()),
        };

        let manifest = build_worldmap(&args).unwrap();
        let parsed = WorldmapManifest::load(output.join(MANIFEST_FILE_NAME)).unwrap();
        let far_height = fs::read(output.join(&manifest.height_far_path)).unwrap();
        let far_heights = far_height
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();
        let far_color = fs::read(output.join(&manifest.color_far_path)).unwrap();

        assert_eq!(parsed, manifest);
        assert_eq!(manifest.tile_count_x, 2);
        assert_eq!(manifest.tile_count_y, 2);
        assert_eq!(manifest.water.source_width, 4);
        assert_eq!(manifest.water.source_height, 4);
        assert_eq!(far_heights, vec![6, 8, 14, 16]);
        assert_eq!(far_color.len(), 2 * 2 * RGBA_BYTES_PER_PIXEL);
        assert_eq!(
            fs::metadata(output.join("height/near/tile_0000_0000.r16"))
                .unwrap()
                .len(),
            4 * 4 * R16_BYTES_PER_PIXEL as u64
        );
        assert_eq!(
            fs::metadata(output.join("color/near/tile_0001_0001.rgba"))
                .unwrap()
                .len(),
            4 * 4 * RGBA_BYTES_PER_PIXEL as u64
        );
        assert_eq!(
            fs::metadata(output.join("water/mesh/tile_0000_0000.wmesh"))
                .unwrap()
                .len(),
            (WATER_MESH_MAGIC.len() + 8) as u64
        );
        assert_eq!(
            fs::metadata(output.join("water/flow/tile_0001_0001.rg8"))
                .unwrap()
                .len(),
            2 * 2 * RG_BYTES_PER_PIXEL as u64
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn builds_worldmap_package_with_water() {
        let dir = unique_temp_dir("build_worldmap_water");
        let height_path = dir.join("source.r16");
        let color_path = dir.join("source.png");
        let water_height_path = dir.join("water.png");
        let flow_path = dir.join("flow.png");
        let output = dir.join("world");
        fs::create_dir_all(&dir).unwrap();

        fs::write(
            &height_path,
            r16_bytes(&[
                1, 2, 3, 4, //
                5, 6, 7, 8, //
                9, 10, 11, 12, //
                13, 14, 15, 16,
            ]),
        )
        .unwrap();
        RgbaImage::from_raw(4, 4, vec![64; 4 * 4 * RGBA_BYTES_PER_PIXEL])
            .unwrap()
            .save(&color_path)
            .unwrap();
        write_luma16_png(
            &water_height_path,
            4,
            4,
            vec![
                100, 100, 100, 100, //
                100, 1000, 0, 100, //
                100, 0, 1200, 100, //
                100, 100, 100, 100,
            ],
        );
        RgbImage::from_raw(4, 4, vec![127; 4 * 4 * RGB_BYTES_PER_PIXEL])
            .unwrap()
            .save(&flow_path)
            .unwrap();

        let args = Args {
            height_input: height_path,
            height_size: Size {
                width: 4,
                height: 4,
            },
            color_input: color_path,
            water_height_input: water_height_path,
            water_flow_input: flow_path,
            output: output.clone(),
            tile_size: 2,
            tile_padding: 1,
            far_height_size: Size {
                width: 2,
                height: 2,
            },
            far_color_size: Size {
                width: 2,
                height: 2,
            },
            horizontal_scale: 0.5,
            height_scale: 10.0,
            name: Some("test-water".to_owned()),
        };

        let manifest = build_worldmap(&args).unwrap();
        let parsed = WorldmapManifest::load(output.join(MANIFEST_FILE_NAME)).unwrap();
        let water = &parsed.water;
        let mesh_bytes = fs::read(output.join("water/mesh/tile_0000_0000.wmesh")).unwrap();
        let flow_bytes = fs::read(output.join("water/flow/tile_0000_0000.rg8")).unwrap();
        let (vertex_count, index_count) = water_mesh_counts(&mesh_bytes);

        assert_eq!(parsed, manifest);
        assert_eq!(water.source_width, 4);
        assert_eq!(water.source_height, 4);
        assert_eq!(water.tile_size_x, 2);
        assert_eq!(water.tile_size_y, 2);
        assert_eq!(water.ocean_raw_height, 100);
        assert!(water.ocean_height > 0.0);
        assert_eq!(flow_bytes.len(), 2 * 2 * RG_BYTES_PER_PIXEL);
        assert!(vertex_count > 0);
        assert!(index_count > 0);

        let _ = fs::remove_dir_all(dir);
    }
}
