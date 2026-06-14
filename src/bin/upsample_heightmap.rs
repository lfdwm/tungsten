use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::{self, File},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

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
    input_size: Size,
    output: PathBuf,
    output_size: Size,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args(env::args_os().skip(1))?;
    upsample_r16(&args)?;

    println!(
        "wrote {} as {}x{} bilinear R16 upsample from {}x{} source",
        args.output.display(),
        args.output_size.width,
        args.output_size.height,
        args.input_size.width,
        args.input_size.height
    );

    Ok(())
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Args, Box<dyn Error>> {
    let mut input = None;
    let mut input_size = None;
    let mut output = None;
    let mut output_size = None;
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--input") => input = Some(PathBuf::from(next_value(&mut args, "--input")?)),
            Some("--input-size") => {
                input_size = Some(parse_size(next_value(&mut args, "--input-size")?)?)
            }
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
        input_size: input_size.ok_or("--input-size is required")?,
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

fn upsample_r16(args: &Args) -> Result<(), Box<dyn Error>> {
    validate_sizes(&args.input_size, &args.output_size)?;

    let input = fs::read(&args.input)?;
    let expected_input_bytes = checked_len_bytes(&args.input_size)?;
    if input.len() != expected_input_bytes {
        return Err(format!(
            "{} has {} bytes, expected {expected_input_bytes} for {}x{} R16",
            args.input.display(),
            input.len(),
            args.input_size.width,
            args.input_size.height
        )
        .into());
    }

    write_upsampled_r16(&input, &args.input_size, &args.output, &args.output_size)
}

fn validate_sizes(input_size: &Size, output_size: &Size) -> Result<(), Box<dyn Error>> {
    if output_size.width < input_size.width || output_size.height < input_size.height {
        return Err("output size must be no smaller than input size".into());
    }

    checked_len_bytes(input_size)?;
    checked_len_bytes(output_size)?;

    Ok(())
}

fn checked_len_bytes(size: &Size) -> Result<usize, Box<dyn Error>> {
    size.width
        .checked_mul(size.height)
        .and_then(|pixels| pixels.checked_mul(2))
        .ok_or_else(|| "image dimensions overflow usize".into())
}

fn write_upsampled_r16(
    input: &[u8],
    input_size: &Size,
    output: &Path,
    output_size: &Size,
) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(output)?;
    let mut writer = BufWriter::new(file);
    let x_samples = sample_axis_table(input_size.width, output_size.width);
    let mut row = vec![0; output_size.width * 2];

    for out_y in 0..output_size.height {
        let y_sample = sample_axis(input_size.height, output_size.height, out_y);

        for (out_x, x_sample) in x_samples.iter().enumerate() {
            let height = sample_bilinear_r16(input, input_size.width, *x_sample, y_sample);
            let output_index = out_x * 2;
            row[output_index..output_index + 2].copy_from_slice(&height.to_le_bytes());
        }

        writer.write_all(&row)?;
    }

    writer.flush()?;
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

fn sample_bilinear_r16(input: &[u8], width: usize, x: SampleAxis, y: SampleAxis) -> u16 {
    let h00 = read_r16(input, width, x.lower, y.lower) as f32;
    let h10 = read_r16(input, width, x.upper, y.lower) as f32;
    let h01 = read_r16(input, width, x.lower, y.upper) as f32;
    let h11 = read_r16(input, width, x.upper, y.upper) as f32;

    let h0 = h00 + (h10 - h00) * x.weight;
    let h1 = h01 + (h11 - h01) * x.weight;
    let height = h0 + (h1 - h0) * y.weight;

    height.round().clamp(0.0, u16::MAX as f32) as u16
}

fn read_r16(input: &[u8], width: usize, x: usize, y: usize) -> u16 {
    let index = (y * width + x) * 2;
    u16::from_le_bytes([input[index], input[index + 1]])
}

fn usage() -> &'static str {
    "usage: cargo run --release --bin upsample_heightmap -- \\
        --input <source.r16> --input-size <WIDTHxHEIGHT> \\
        --output <dest.r16> --output-size <WIDTHxHEIGHT>"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size() {
        assert_eq!(
            parse_size(OsString::from("8192x4096")).unwrap(),
            Size {
                width: 8192,
                height: 4096
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
    fn bilinear_samples_r16() {
        let input_size = Size {
            width: 2,
            height: 2,
        };
        let heights: [u16; 4] = [0, 100, 200, 300];
        let input = heights
            .iter()
            .flat_map(|height| height.to_le_bytes())
            .collect::<Vec<_>>();

        let height = sample_bilinear_r16(
            &input,
            input_size.width,
            SampleAxis {
                lower: 0,
                upper: 1,
                weight: 0.5,
            },
            SampleAxis {
                lower: 0,
                upper: 1,
                weight: 0.5,
            },
        );

        assert_eq!(height, 150);
    }
}
