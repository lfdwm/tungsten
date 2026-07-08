use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use image::RgbaImage;
use png::{BitDepth, ColorType, Compression, Encoder};

const PNG_STREAM_CHUNK_BYTES: usize = 1024 * 1024;
const RGBA_CHANNELS: usize = 4;

const BAYER_4X4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

#[derive(Debug, PartialEq, Eq)]
struct Size {
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SampleAxis {
    lower: usize,
    upper: usize,
    weight: f32,
}

struct Args {
    input: PathBuf,
    output: PathBuf,
    output_size: Size,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args(env::args_os().skip(1))?;
    upsample_colormap(&args)?;

    println!(
        "wrote {} as {}x{} Bayer 4x4 dithered RGBA PNG upsample",
        args.output.display(),
        args.output_size.width,
        args.output_size.height
    );

    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, Box<dyn Error>> {
    let mut input = None;
    let mut output = None;
    let mut output_size = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--input") => input = Some(PathBuf::from(next_value(&mut args, "--input")?)),
            Some("--output") => output = Some(PathBuf::from(next_value(&mut args, "--output")?)),
            Some("--output-size") => {
                output_size = Some(parse_size(next_value(&mut args, "--output-size")?)?)
            }
            Some("--help" | "-h") => return Err(usage().into()),
            Some(flag) if flag.starts_with('-') => {
                return Err(format!("unknown flag: {flag}").into());
            }
            _ => return Err(usage().into()),
        }
    }

    Ok(Args {
        input: input.ok_or("--input is required")?,
        output: output.ok_or("--output is required")?,
        output_size: output_size.ok_or("--output-size is required")?,
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

fn upsample_colormap(args: &Args) -> Result<(), Box<dyn Error>> {
    let input = read_input_rgba(&args.input)?;
    let input_size = Size {
        width: input.width() as usize,
        height: input.height() as usize,
    };

    validate_sizes(&input_size, &args.output_size)?;
    write_upsampled_png(&input, &input_size, &args.output, &args.output_size)
}

fn read_input_rgba(path: &Path) -> Result<RgbaImage, Box<dyn Error>> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    reader.no_limits();

    Ok(reader
        .decode()
        .map_err(|error| format!("failed to decode {}: {error}", path.display()))?
        .to_rgba8())
}

fn validate_sizes(input_size: &Size, output_size: &Size) -> Result<(), Box<dyn Error>> {
    if output_size.width < input_size.width || output_size.height < input_size.height {
        return Err("output size must be no smaller than input size".into());
    }

    checked_pixel_bytes(input_size)?;
    checked_pixel_bytes(output_size)?;
    checked_png_dimension(output_size.width, "output width")?;
    checked_png_dimension(output_size.height, "output height")?;

    Ok(())
}

fn checked_pixel_bytes(size: &Size) -> Result<usize, Box<dyn Error>> {
    size.width
        .checked_mul(size.height)
        .and_then(|pixels| pixels.checked_mul(RGBA_CHANNELS))
        .ok_or_else(|| "image dimensions overflow usize".into())
}

fn checked_row_bytes(width: usize) -> Result<usize, Box<dyn Error>> {
    width
        .checked_mul(RGBA_CHANNELS)
        .ok_or_else(|| "image row width overflows usize".into())
}

fn checked_png_dimension(value: usize, label: &'static str) -> Result<u32, Box<dyn Error>> {
    u32::try_from(value).map_err(|_| format!("{label} exceeds u32::MAX").into())
}

fn write_upsampled_png(
    input: &RgbaImage,
    input_size: &Size,
    output: &Path,
    output_size: &Size,
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(output)?;
    let writer = BufWriter::new(file);
    let mut encoder = Encoder::new(
        writer,
        checked_png_dimension(output_size.width, "output width")?,
        checked_png_dimension(output_size.height, "output height")?,
    );
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::Fast);

    let mut png_writer = encoder.write_header()?;
    {
        let mut stream = png_writer.stream_writer_with_size(PNG_STREAM_CHUNK_BYTES)?;
        let x_samples = sample_axis_table(input_size.width, output_size.width);
        let mut row = vec![0; checked_row_bytes(output_size.width)?];

        for out_y in 0..output_size.height {
            let y_sample = sample_axis(input_size.height, output_size.height, out_y);

            for (out_x, x_sample) in x_samples.iter().enumerate() {
                let threshold = dither_threshold(out_x, out_y);
                let color = sample_bayer_rgba(
                    input.as_raw(),
                    input_size.width,
                    *x_sample,
                    y_sample,
                    threshold,
                );
                let output_index = out_x * RGBA_CHANNELS;

                row[output_index..output_index + RGBA_CHANNELS].copy_from_slice(&color);
            }

            stream.write_all(&row)?;
        }

        stream.finish()?;
    }

    png_writer.finish()?;
    Ok(())
}

fn sample_axis_table(input_len: usize, output_len: usize) -> Vec<SampleAxis> {
    (0..output_len)
        .map(|out_index| sample_axis(input_len, output_len, out_index))
        .collect()
}

fn sample_axis(input_len: usize, output_len: usize, out_index: usize) -> SampleAxis {
    if input_len == 1 || output_len == 1 {
        return SampleAxis {
            lower: 0,
            upper: 0,
            weight: 0.0,
        };
    }

    let source_pos = out_index as f32 * (input_len - 1) as f32 / (output_len - 1) as f32;
    let lower = source_pos.floor() as usize;
    let upper = (lower + 1).min(input_len - 1);

    SampleAxis {
        lower,
        upper,
        weight: source_pos - lower as f32,
    }
}

fn sample_bayer_rgba(
    input: &[u8],
    width: usize,
    x: SampleAxis,
    y: SampleAxis,
    threshold: f32,
) -> [u8; 4] {
    let top_left_weight = (1.0 - x.weight) * (1.0 - y.weight);
    let top_right_weight = x.weight * (1.0 - y.weight);
    let bottom_left_weight = (1.0 - x.weight) * y.weight;

    let (source_x, source_y) = if threshold < top_left_weight {
        (x.lower, y.lower)
    } else if threshold < top_left_weight + top_right_weight {
        (x.upper, y.lower)
    } else if threshold < top_left_weight + top_right_weight + bottom_left_weight {
        (x.lower, y.upper)
    } else {
        (x.upper, y.upper)
    };

    read_rgba(input, width, source_x, source_y)
}

fn read_rgba(input: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let index = (y * width + x) * RGBA_CHANNELS;
    [
        input[index],
        input[index + 1],
        input[index + 2],
        input[index + 3],
    ]
}

fn dither_threshold(x: usize, y: usize) -> f32 {
    (BAYER_4X4[y & 3][x & 3] as f32 + 0.5) / 16.0
}

fn usage() -> &'static str {
    "usage: cargo run --release --bin upsample_colormap -- \\
        --input <source.png> --output <dest.png> --output-size <WIDTHxHEIGHT>"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size() {
        assert_eq!(
            parse_size(OsString::from("16384x8192")).unwrap(),
            Size {
                width: 16384,
                height: 8192
            }
        );
    }

    #[test]
    fn samples_axis_endpoint_aligned() {
        assert_eq!(
            sample_axis(2, 3, 1),
            SampleAxis {
                lower: 0,
                upper: 1,
                weight: 0.5
            }
        );
        assert_eq!(
            sample_axis(4, 7, 6),
            SampleAxis {
                lower: 3,
                upper: 3,
                weight: 0.0
            }
        );
    }

    #[test]
    fn bayer_selects_source_rgba_without_interpolation() {
        let input: [u8; 16] = [
            0, 10, 20, 255, 100, 110, 120, 255, 200, 210, 220, 255, 255, 250, 245, 255,
        ];
        let x = SampleAxis {
            lower: 0,
            upper: 1,
            weight: 0.5,
        };
        let y = SampleAxis {
            lower: 0,
            upper: 1,
            weight: 0.5,
        };

        assert_eq!(sample_bayer_rgba(&input, 2, x, y, 0.10), [0, 10, 20, 255]);
        assert_eq!(
            sample_bayer_rgba(&input, 2, x, y, 0.30),
            [100, 110, 120, 255]
        );
        assert_eq!(
            sample_bayer_rgba(&input, 2, x, y, 0.55),
            [200, 210, 220, 255]
        );
        assert_eq!(
            sample_bayer_rgba(&input, 2, x, y, 0.80),
            [255, 250, 245, 255]
        );
    }

    #[test]
    fn dither_threshold_uses_4x4_bayer_pattern() {
        assert_eq!(dither_threshold(0, 0), 0.5 / 16.0);
        assert_eq!(dither_threshold(1, 0), 8.5 / 16.0);
        assert_eq!(dither_threshold(3, 3), 5.5 / 16.0);
        assert_eq!(dither_threshold(4, 0), dither_threshold(0, 0));
        assert_eq!(dither_threshold(0, 4), dither_threshold(0, 0));
    }

    #[test]
    fn writes_streamed_png() {
        let input = RgbaImage::from_raw(
            2,
            2,
            vec![
                0, 10, 20, 255, 100, 110, 120, 255, 200, 210, 220, 255, 255, 250, 245, 255,
            ],
        )
        .unwrap();
        let path = env::temp_dir().join(format!(
            "tungsten_upsample_colormap_test_{}.png",
            std::process::id()
        ));

        write_upsampled_png(
            &input,
            &Size {
                width: 2,
                height: 2,
            },
            &path,
            &Size {
                width: 3,
                height: 3,
            },
        )
        .unwrap();

        let output = image::open(&path).unwrap().to_rgba8();
        let _ = fs::remove_file(path);

        assert_eq!(output.dimensions(), (3, 3));
        assert_eq!(output.get_pixel(0, 0).0, [0, 10, 20, 255]);
        assert_eq!(output.get_pixel(1, 1).0, [100, 110, 120, 255]);
        assert_eq!(output.get_pixel(2, 2).0, [255, 250, 245, 255]);
    }
}
