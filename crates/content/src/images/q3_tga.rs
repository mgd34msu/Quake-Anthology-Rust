//! Quake III TGA decoder (owned pixels).
//!
//! Donor: `decodeTga` in `src/formats/images/q3-tga.ts`
//! (ported from id Software `tr_image.c` LoadTGA, GPL-2.0-or-later).

use qa_core::binary::BinaryReader;

use super::{fail, ContentError, ImageLevel};

const HEADER_SIZE: usize = 18;

// Donor arity: mirrors LoadTGA's pixel-write operands one-to-one.
#[allow(clippy::too_many_arguments)]
fn write_pixel(
    output: &mut [u8],
    source_pixel: usize,
    width: usize,
    height: usize,
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
) {
    let source_y = source_pixel / width;
    let source_x = source_pixel - source_y * width;
    let destination = ((height - source_y - 1) * width + source_x) * 4;
    output[destination] = red;
    output[destination + 1] = green;
    output[destination + 2] = blue;
    output[destination + 3] = alpha;
}

fn read_and_write_pixel(
    reader: &mut BinaryReader<'_>,
    output: &mut [u8],
    source_pixel: usize,
    width: usize,
    height: usize,
    pixel_depth: u8,
) -> Result<(), ContentError> {
    if pixel_depth == 8 {
        let intensity = reader.u8()?;
        write_pixel(
            output,
            source_pixel,
            width,
            height,
            intensity,
            intensity,
            intensity,
            255,
        );
        return Ok(());
    }
    let blue = reader.u8()?;
    let green = reader.u8()?;
    let red = reader.u8()?;
    let alpha = if pixel_depth == 32 { reader.u8()? } else { 255 };
    write_pixel(output, source_pixel, width, height, red, green, blue, alpha);
    Ok(())
}

/// Decode a Q3 TGA (`decodeTga`, exported as `decodeQ3Tga`).
pub fn decode_q3_tga(bytes: &[u8], source: &str) -> Result<ImageLevel, ContentError> {
    decode_q3_tga_with_warning(bytes, source, &mut |_| {})
}

/// Decode a Q3 TGA, reporting the top-down header quirk through `warning`.
pub fn decode_q3_tga_with_warning(
    bytes: &[u8],
    source: &str,
    warning: &mut dyn FnMut(&str),
) -> Result<ImageLevel, ContentError> {
    let mut reader = BinaryReader::new(bytes, source);
    if reader.length() < HEADER_SIZE {
        return Err(fail(source, 0, "truncated TGA header".to_string()));
    }

    let id_length = reader.u8()?;
    let color_map_type = reader.u8()?;
    let image_type = reader.u8()?;
    reader.skip(5)?;
    reader.skip(4)?;
    let width = reader.u16()?;
    let height = reader.u16()?;
    let pixel_depth = reader.u8()?;
    let descriptor = reader.u8()?;

    if image_type != 2 && image_type != 3 && image_type != 10 {
        return Err(fail(source, 2, format!("unsupported TGA image type {image_type}")));
    }
    if color_map_type != 0 {
        return Err(fail(source, 1, "color-mapped TGA images are unsupported".to_string()));
    }
    if pixel_depth != 24 && pixel_depth != 32 && !(image_type == 3 && pixel_depth == 8) {
        return Err(fail(
            source,
            16,
            format!("unsupported {pixel_depth}-bit depth for TGA image type {image_type}"),
        ));
    }
    if width == 0 || height == 0 {
        return Err(fail(source, 12, format!("invalid TGA dimensions {width}x{height}")));
    }

    reader.skip(usize::from(id_length))?;
    let width_usize = usize::from(width);
    let height_usize = usize::from(height);
    let pixel_count = width_usize as u64 * height_usize as u64;
    let output_length = pixel_count * 4;
    let bytes_per_pixel = u64::from(pixel_depth / 8);
    if image_type == 10 {
        let minimum = pixel_count.div_ceil(128) * (bytes_per_pixel + 1);
        if (reader.remaining() as u64) < minimum {
            return Err(fail(
                source,
                reader.offset(),
                "truncated TGA RLE pixel data".to_string(),
            ));
        }
    } else {
        let required = pixel_count * bytes_per_pixel;
        if (reader.remaining() as u64) < required {
            return Err(fail(source, reader.offset(), "truncated TGA pixel data".to_string()));
        }
    }

    if output_length > 0x7fff_ffff {
        return Err(fail(
            source,
            12,
            format!("decoded TGA size {output_length} overflows the source signed-int allocation"),
        ));
    }
    let pixel_total = pixel_count as usize;
    let mut output = vec![0u8; output_length as usize];

    if image_type != 10 {
        for pixel in 0..pixel_total {
            read_and_write_pixel(&mut reader, &mut output, pixel, width_usize, height_usize, pixel_depth)?;
        }
    } else {
        let mut pixel = 0usize;
        while pixel < pixel_total {
            let packet_header = reader.u8()?;
            let packet_length = usize::from(packet_header & 0x7f) + 1;
            let packet_end = (pixel + packet_length).min(pixel_total);
            if packet_header & 0x80 != 0 {
                let blue = reader.u8()?;
                let green = reader.u8()?;
                let red = reader.u8()?;
                let alpha = if pixel_depth == 32 { reader.u8()? } else { 255 };
                while pixel < packet_end {
                    write_pixel(&mut output, pixel, width_usize, height_usize, red, green, blue, alpha);
                    pixel += 1;
                }
            } else {
                while pixel < packet_end {
                    read_and_write_pixel(&mut reader, &mut output, pixel, width_usize, height_usize, pixel_depth)?;
                    pixel += 1;
                }
            }
        }
    }

    if descriptor & 0x20 != 0 {
        warning(&format!(
            "WARNING: '{source}' TGA file header declares top-down image, ignoring\n"
        ));
    }
    Ok(ImageLevel {
        width: u32::from(width),
        height: u32::from(height),
        pixels: output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(image_type: u8, depth: u8, descriptor: u8, width: u16, height: u16) -> Vec<u8> {
        let mut out = vec![0, 0, image_type];
        out.extend_from_slice(&[0u8; 5]);
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.push(depth);
        out.push(descriptor);
        out
    }

    #[test]
    fn decodes_truecolor_bottom_up() {
        let mut bytes = header(2, 24, 0, 2, 1);
        bytes.extend_from_slice(&[0, 0, 255, 0, 255, 0]);
        let image = decode_q3_tga(&bytes, "<test>").unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn ignores_top_down_descriptor_with_warning() {
        let mut bytes = header(2, 24, 32, 1, 2);
        bytes.extend_from_slice(&[3, 2, 1, 6, 5, 4]);
        let mut warnings = Vec::new();
        let image = decode_q3_tga_with_warning(&bytes, "<test>", &mut |text| {
            warnings.push(text.to_string());
        })
        .unwrap();
        assert_eq!(
            warnings,
            vec!["WARNING: '<test>' TGA file header declares top-down image, ignoring\n"]
        );
        // Still flipped bottom-up: first stored pixel lands on the last row.
        assert_eq!(image.pixels, vec![4, 5, 6, 255, 1, 2, 3, 255]);
    }

    #[test]
    fn rle_packet_end_clamps_to_image() {
        // 2x1 image with a run packet claiming 3 pixels: only 2 are written.
        let mut bytes = header(10, 24, 0, 2, 1);
        bytes.extend_from_slice(&[0x82, 9, 8, 7]);
        let image = decode_q3_tga(&bytes, "<test>").unwrap();
        assert_eq!(image.pixels, vec![7, 8, 9, 255, 7, 8, 9, 255]);
    }

    #[test]
    fn rejects_bad_type_and_truncation() {
        let bytes = header(1, 24, 0, 1, 1);
        let error = decode_q3_tga(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "unsupported TGA image type 1");
        assert_eq!(error.offset, 2);

        let bytes = header(2, 24, 0, 2, 1);
        let error = decode_q3_tga(&bytes, "<test>").unwrap_err();
        assert_eq!(error.message, "truncated TGA pixel data");
    }
}
