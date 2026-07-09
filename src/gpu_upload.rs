use std::error::Error;

use sdl3::gpu::{
    Buffer, BufferRegion, BufferUsageFlags, CopyPass, Device, Texture, TextureCreateInfo,
    TextureFormat, TextureRegion, TextureTransferInfo, TextureType, TextureUsage,
    TransferBufferLocation, TransferBufferUsage,
};

pub(crate) fn create_buffer_with_data<T: Copy>(
    gpu: &Device,
    copy_pass: &CopyPass,
    usage: BufferUsageFlags,
    data: &[T],
    context: &str,
) -> Result<Buffer, Box<dyn Error>> {
    if data.is_empty() {
        return Err(format!("cannot create an empty {context} GPU buffer").into());
    }

    let len_bytes = transfer_size_bytes(std::mem::size_of_val(data), context)?;
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

pub(crate) fn create_texture_2d(
    gpu: &Device,
    width: u32,
    height: u32,
    format: TextureFormat,
    usage: TextureUsage,
) -> Result<Texture<'static>, Box<dyn Error>> {
    Ok(gpu.create_texture(
        TextureCreateInfo::new()
            .with_format(format)
            .with_type(TextureType::_2D)
            .with_width(width)
            .with_height(height)
            .with_layer_count_or_depth(1)
            .with_num_levels(1)
            .with_usage(usage),
    )?)
}

pub(crate) fn create_texture_2d_with_pixels(
    gpu: &Device,
    copy_pass: &CopyPass,
    width: u32,
    height: u32,
    format: TextureFormat,
    usage: TextureUsage,
    pixels: &[u8],
    context: &str,
) -> Result<Texture<'static>, Box<dyn Error>> {
    let texture = create_texture_2d(gpu, width, height, format, usage)?;
    upload_bytes_to_texture_region(
        gpu, copy_pass, &texture, pixels, 0, width, height, 0, 0, width, height, context,
    )?;

    Ok(texture)
}

pub(crate) fn upload_bytes_to_texture_region(
    gpu: &Device,
    copy_pass: &CopyPass,
    texture: &Texture,
    pixels: &[u8],
    transfer_offset: u32,
    pixels_per_row: u32,
    rows_per_layer: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    context: &str,
) -> Result<(), Box<dyn Error>> {
    let size_bytes = transfer_size_bytes(pixels.len(), context)?;
    let transfer_buffer = gpu
        .create_transfer_buffer()
        .with_size(size_bytes)
        .with_usage(TransferBufferUsage::UPLOAD)
        .build()?;

    let mut map = transfer_buffer.map::<u8>(gpu, false);
    map.mem_mut().copy_from_slice(pixels);
    map.unmap();

    copy_pass.upload_to_gpu_texture(
        TextureTransferInfo::new()
            .with_transfer_buffer(&transfer_buffer)
            .with_offset(transfer_offset)
            .with_pixels_per_row(pixels_per_row)
            .with_rows_per_layer(rows_per_layer),
        TextureRegion::new()
            .with_texture(texture)
            .with_layer(0)
            .with_x(x)
            .with_y(y)
            .with_width(width)
            .with_height(height)
            .with_depth(1),
        false,
    );

    Ok(())
}

fn transfer_size_bytes(size: usize, context: &str) -> Result<u32, Box<dyn Error>> {
    u32::try_from(size)
        .map_err(|_| format!("{context} upload is too large for SDL transfer buffer size").into())
}
