use super::*;
use std::{io::Cursor, num::NonZeroU64};

pub(super) fn png(bytes: &[u8]) -> Result<RgbaImage, FormatError> {
    let mut decoder =
        png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: MAX_BYTES });
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| FormatError::InvalidValue)?;
    let width = reader.info().width;
    let height = reader.info().height;
    let count = pixel_count(width, height)?;
    let size = reader
        .output_buffer_size()
        .ok_or(FormatError::InvalidRange)?;
    if size > MAX_BYTES {
        return Err(FormatError::InvalidRange);
    }
    let mut output = vec![0; size];
    let info = reader
        .next_frame(&mut output)
        .map_err(|_| FormatError::InvalidValue)?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(FormatError::Unsupported);
    }
    output.truncate(info.buffer_size());
    let pixels = match info.color_type {
        png::ColorType::Rgba => output,
        kind => {
            let channels = match kind {
                png::ColorType::Grayscale => 1,
                png::ColorType::GrayscaleAlpha => 2,
                png::ColorType::Rgb => 3,
                _ => return Err(FormatError::Unsupported),
            };
            let mut rgba = Vec::with_capacity(count * 4);
            for pixel in output.chunks_exact(channels) {
                rgba.extend_from_slice(&match channels {
                    1 => [pixel[0], pixel[0], pixel[0], 255],
                    2 => [pixel[0], pixel[0], pixel[0], pixel[1]],
                    _ => [pixel[0], pixel[1], pixel[2], 255],
                });
            }
            rgba
        }
    };
    reader.finish().map_err(|_| FormatError::InvalidValue)?;
    if pixels.len() != count * 4 {
        return Err(FormatError::InvalidRecordSize);
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
pub(super) fn jpeg(bytes: &[u8]) -> Result<RgbaImage, FormatError> {
    use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let options = DecoderOptions::default()
        .set_max_width(16384)
        .set_max_height(16384)
        .jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|_| FormatError::InvalidValue)?;
    let info = decoder.info().ok_or(FormatError::InvalidValue)?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    let count = pixel_count(width, height)?;
    let cmyk = matches!(
        decoder.input_colorspace(),
        Some(ColorSpace::CMYK | ColorSpace::YCCK)
    );
    if cmyk {
        // Original Quake JPEG loading treats decoded C/M/Y as RGB and drops K.
        // Ask the codec for CMYK explicitly instead of its RGB conversion.
        decoder.set_options(decoder.options().jpeg_set_out_colorspace(ColorSpace::CMYK));
    }
    let mut pixels = decoder.decode().map_err(|_| FormatError::InvalidValue)?;
    if pixels.len() != count * 4 {
        return Err(FormatError::InvalidRecordSize);
    }
    if cmyk {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[3] = 255;
        }
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
pub(super) fn gif(bytes: &[u8]) -> Result<RgbaImage, FormatError> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        NonZeroU64::new(MAX_BYTES as u64).ok_or(FormatError::InvalidRange)?,
    ));
    let mut reader = options
        .read_info(Cursor::new(bytes))
        .map_err(|_| FormatError::InvalidValue)?;
    let width = u32::from(reader.width());
    let height = u32::from(reader.height());
    let count = pixel_count(width, height)?;
    let frame = reader
        .read_next_frame()
        .map_err(|_| FormatError::InvalidValue)?
        .ok_or(FormatError::Truncated)?;
    if u32::from(frame.left) + u32::from(frame.width) > width
        || u32::from(frame.top) + u32::from(frame.height) > height
    {
        return Err(FormatError::InvalidRange);
    }
    let mut pixels = vec![0; count * 4];
    for y in 0..frame.height as usize {
        let dest = ((frame.top as usize + y) * width as usize + frame.left as usize) * 4;
        let first = y * frame.width as usize * 4;
        let end = first + frame.width as usize * 4;
        let row = frame.buffer.get(first..end).ok_or(FormatError::Truncated)?;
        pixels[dest..dest + row.len()].copy_from_slice(row);
    }
    Ok(RgbaImage {
        width,
        height,
        pixels,
    })
}
