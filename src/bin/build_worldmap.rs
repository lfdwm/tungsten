use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use tungsten::worldmap::{
    COLOR_FORMAT_RGBA8, COLOR_NEAR_DIR, HEIGHT_FORMAT_R16LE, HEIGHT_NEAR_DIR, MANIFEST_FILE_NAME,
    WorldmapManifest, color_tile_file_name, height_tile_file_name,
};

const R16_BYTES_PER_PIXEL: usize = 2;
const RGBA_BYTES_PER_PIXEL: usize = 4;
const DEFAULT_TILE_SIZE: usize = 1024;
const DEFAULT_TILE_PADDING: usize = 2;
const DEFAULT_HORIZONTAL_SCALE: f32 = 0.5;
const DEFAULT_HEIGHT_SCALE: f32 = 255.0 * 2.1;

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
    let height_far_path = far_height_relative_path(&args.far_height_size);
    let color_far_path = far_color_relative_path(&args.far_color_size);
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
    }
    .validate()?;

    fs::create_dir_all(&args.output)?;
    write_height_tiles(&height_bytes, &args.height_size, args, &manifest)?;
    write_color_tiles(color.as_raw(), &args.height_size, args, &manifest)?;

    let far_height = max_pool_r16(&height_bytes, &args.height_size, &args.far_height_size);
    write_output(args.output.join(&manifest.height_far_path), &far_height)?;

    let far_color = box_downsample_rgba(color.as_raw(), &args.height_size, &args.far_color_size);
    write_output(args.output.join(&manifest.color_far_path), &far_color)?;

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
        [--tile-size 1024] [--tile-padding 2] \\
        [--horizontal-scale 0.5] [--height-scale 535.5] [--name <name>]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;
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

    #[test]
    fn parses_args_with_defaults() {
        let args = parse_args(os_args(&[
            "--height-input",
            "height.r16",
            "--height-size",
            "16384x16384",
            "--color-input",
            "color.png",
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

        let args = Args {
            height_input: height_path,
            height_size: Size {
                width: 4,
                height: 4,
            },
            color_input: color_path,
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

        let _ = fs::remove_dir_all(dir);
    }
}
