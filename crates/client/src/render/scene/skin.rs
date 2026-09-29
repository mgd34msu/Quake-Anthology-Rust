//! Skin background flood fill.
//!
//! Donor provenance: `src/render/scene/skin.ts` (`floodSkin`).
//! `GL_FloodFillSkin` replaces the connected skin background before
//! mipmapping.

use crate::render::error::RenderError;
use crate::render::types::Palette;

/// Replace the connected background of an indexed skin.
///
/// The fill starts at index 0: every connected pixel holding that index is
/// rewritten to the neighboring non-background color (or the first
/// all-black palette entry when no such neighbor exists), using 255 as the
/// visited sentinel. Skins already filled with black or 255 are returned
/// unchanged.
pub fn flood_skin(indices: &[u8], width: u32, height: u32, palette: &Palette) -> Result<Vec<u8>, RenderError> {
    if width == 0 || height == 0 {
        return Err(RenderError::BadDimensions {
            width,
            height,
            detail: "skin dimensions must be positive".to_string(),
        });
    }
    let expected = width as usize * height as usize;
    if indices.len() != expected {
        return Err(RenderError::BadDimensions {
            width,
            height,
            detail: format!("skin holds {} indices, expected {expected}", indices.len()),
        });
    }
    if palette.colors.len() < 768 {
        return Err(RenderError::BadWire(format!(
            "skin palette holds {} color bytes, expected at least 768",
            palette.colors.len()
        )));
    }

    let mut result = indices.to_vec();
    let fill = result[0];
    let mut black = 0u8;
    for index in 0..256usize {
        if palette.colors[index * 3] == 0 && palette.colors[index * 3 + 1] == 0 && palette.colors[index * 3 + 2] == 0 {
            black = index as u8;
            break;
        }
    }
    if fill == black || fill == 255 {
        return Ok(result);
    }

    let stride = width as usize;
    let rows = height as usize;
    let mut queue = vec![0usize];
    result[0] = 255;
    while let Some(pixel) = queue.pop() {
        let x = pixel % stride;
        let y = pixel / stride;
        let mut color = black;
        let neighbors = [
            if x > 0 { Some(pixel - 1) } else { None },
            if x + 1 < stride { Some(pixel + 1) } else { None },
            if y > 0 { Some(pixel - stride) } else { None },
            if y + 1 < rows { Some(pixel + stride) } else { None },
        ];
        for next in neighbors.into_iter().flatten() {
            if result[next] == 255 {
                queue.push(next);
            } else {
                color = result[next];
            }
        }
        result[pixel] = color;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette_with_black_at(entry: u8) -> Palette {
        let mut colors = vec![9u8; 768];
        colors[entry as usize * 3] = 0;
        colors[entry as usize * 3 + 1] = 0;
        colors[entry as usize * 3 + 2] = 0;
        Palette {
            colors,
            source: "test".to_string(),
        }
    }

    #[test]
    fn black_fill_returns_copy() {
        let palette = palette_with_black_at(0);
        let indices = vec![0u8; 16];
        assert_eq!(flood_skin(&indices, 4, 4, &palette).unwrap(), indices);
    }

    #[test]
    fn sentinel_fill_returns_copy() {
        let palette = palette_with_black_at(0);
        let indices = vec![255u8; 6];
        assert_eq!(flood_skin(&indices, 3, 2, &palette).unwrap(), indices);
    }

    #[test]
    fn flood_replaces_background_with_neighbor_color() {
        let palette = palette_with_black_at(0);
        let indices = vec![5, 255, 255, 255, 7, 255, 255, 255, 255];
        assert_eq!(
            flood_skin(&indices, 3, 3, &palette).unwrap(),
            vec![0, 7, 7, 0, 7, 7, 0, 7, 7]
        );
    }

    #[test]
    fn flood_without_background_leaves_pixels_untouched() {
        let palette = palette_with_black_at(0);
        let indices = vec![5, 5, 5, 5, 7, 5, 5, 5, 5];
        assert_eq!(flood_skin(&indices, 3, 3, &palette).unwrap(), indices);
    }

    #[test]
    fn flood_stops_at_unconnected_background() {
        let palette = palette_with_black_at(0);
        let indices = vec![5, 7, 5, 7, 7, 7, 5, 7, 5];
        assert_eq!(
            flood_skin(&indices, 3, 3, &palette).unwrap(),
            vec![7, 7, 5, 7, 7, 7, 5, 7, 5]
        );
    }

    #[test]
    fn dimension_mismatches_fail() {
        let palette = palette_with_black_at(0);
        assert!(matches!(
            flood_skin(&[1, 2], 0, 2, &palette),
            Err(RenderError::BadDimensions { .. })
        ));
        assert!(matches!(
            flood_skin(&[1, 2], 1, 0, &palette),
            Err(RenderError::BadDimensions { .. })
        ));
        assert!(matches!(
            flood_skin(&[1, 2, 3], 2, 2, &palette),
            Err(RenderError::BadDimensions { .. })
        ));
    }

    #[test]
    fn short_palette_fails() {
        let palette = Palette {
            colors: vec![0u8; 100],
            source: "test".to_string(),
        };
        assert!(matches!(flood_skin(&[1], 1, 1, &palette), Err(RenderError::BadWire(_))));
    }
}
