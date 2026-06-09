use std::{
    env,
    error::Error,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
struct Size {
    width: usize,
    height: usize,
}

struct Args {
    input: PathBuf,
    input_size: Size,
    output: PathBuf,
    output_size: Size,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = parse_args(env::args_os().skip(1))?;
    generate_max_height_mip(&args)?;

    println!(
        "wrote {} as {}x{} max-height R16 from {}x{} source",
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

fn generate_max_height_mip(args: &Args) -> Result<(), Box<dyn Error>> {
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

    let output = max_pool_r16(
        &input,
        &args.input_size,
        &args.output_size,
        args.input_size.width / args.output_size.width,
        args.input_size.height / args.output_size.height,
    );

    write_output(&args.output, &output)?;

    Ok(())
}

fn validate_sizes(input_size: &Size, output_size: &Size) -> Result<(), Box<dyn Error>> {
    if output_size.width > input_size.width || output_size.height > input_size.height {
        return Err("output size must be no larger than input size".into());
    }
    if input_size.width % output_size.width != 0 || input_size.height % output_size.height != 0 {
        return Err(format!(
            "input size {}x{} must be evenly divisible by output size {}x{}",
            input_size.width, input_size.height, output_size.width, output_size.height
        )
        .into());
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

fn max_pool_r16(
    input: &[u8],
    input_size: &Size,
    output_size: &Size,
    scale_x: usize,
    scale_y: usize,
) -> Vec<u8> {
    let mut output = vec![0; output_size.width * output_size.height * 2];

    for out_y in 0..output_size.height {
        for out_x in 0..output_size.width {
            let mut max_height = 0;

            for dy in 0..scale_y {
                let in_y = out_y * scale_y + dy;
                for dx in 0..scale_x {
                    let in_x = out_x * scale_x + dx;
                    max_height = max_height.max(read_r16(input, input_size.width, in_x, in_y));
                }
            }

            let output_index = (out_y * output_size.width + out_x) * 2;
            output[output_index..output_index + 2].copy_from_slice(&max_height.to_le_bytes());
        }
    }

    output
}

fn read_r16(input: &[u8], width: usize, x: usize, y: usize) -> u16 {
    let index = (y * width + x) * 2;
    u16::from_le_bytes([input[index], input[index + 1]])
}

fn write_output(output: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(output, bytes)?;
    Ok(())
}

fn usage() -> &'static str {
    "usage: cargo run --bin max_height_mip -- \\
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
    fn max_pools_r16() {
        let input_size = Size {
            width: 4,
            height: 4,
        };
        let output_size = Size {
            width: 2,
            height: 2,
        };
        let heights: [u16; 16] = [1, 2, 3, 4, 5, 9, 7, 8, 10, 11, 12, 13, 14, 15, 6, 16];
        let input = heights
            .iter()
            .flat_map(|height| height.to_le_bytes())
            .collect::<Vec<_>>();

        let output = max_pool_r16(&input, &input_size, &output_size, 2, 2);
        let pooled = output
            .chunks_exact(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect::<Vec<_>>();

        assert_eq!(pooled, vec![9, 8, 15, 16]);
    }
}
