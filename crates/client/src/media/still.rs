//! Stills: PCX-backed poster sources.
//!
//! Donor provenance: `src/media/still.ts` (`cinematicPcx`). Stills play
//! through [`crate::media::playback::CinematicPlayback`] image sources.

use qa_content::images::decode_pcx;

use super::containers::cin_rgba;
use super::playback::CinematicSource;
use crate::ClientError;

/// A still image (validated poster pixels).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicStill {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// RGBA pixels.
    pub rgba: Vec<u8>,
}

/// Build a still from RGBA pixels.
pub fn cinematic_still(width: usize, height: usize, rgba: Vec<u8>) -> Result<CinematicStill, ClientError> {
    if width == 0 || height == 0 || rgba.len() != width * height * 4 {
        return Err(ClientError::BadMedia("Invalid still frame pixels".to_string()));
    }
    Ok(CinematicStill { width, height, rgba })
}

/// Build a still from indexed pixels plus a palette.
pub fn cinematic_indexed_still(
    width: usize,
    height: usize,
    pixels: &[u8],
    palette: &[u8],
) -> Result<CinematicStill, ClientError> {
    if width == 0 || height == 0 || pixels.len() != width * height {
        return Err(ClientError::BadMedia("Invalid still frame pixels".to_string()));
    }
    Ok(CinematicStill {
        width,
        height,
        rgba: cin_rgba(pixels, palette)?,
    })
}

/// Build an image source from PCX bytes (`cinematicPcx`).
pub fn cinematic_pcx(bytes: &[u8], source: &str) -> Result<CinematicSource, ClientError> {
    let image = decode_pcx(bytes, source).map_err(|error| ClientError::BadMedia(error.to_string()))?;
    let Some(palette) = image.palette else {
        return Err(ClientError::BadMedia(
            "A cinematic PCX needs its own palette".to_string(),
        ));
    };
    Ok(CinematicSource::Image {
        source: source.to_string(),
        width: image.width,
        height: image.height,
        rgba: cin_rgba(&image.indices, &palette)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcx(width: u16, height: u16, scanline: &[u8], palette: Option<&[u8]>) -> Vec<u8> {
        // Minimal version 5, RLE, 8-bit, single-plane PCX.
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(&[10, 5, 1, 8]);
        bytes[4..6].copy_from_slice(&0u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&0u16.to_le_bytes());
        bytes[8..10].copy_from_slice(&(width - 1).to_le_bytes());
        bytes[10..12].copy_from_slice(&(height - 1).to_le_bytes());
        bytes[65] = 1;
        bytes[66..68].copy_from_slice(&width.to_le_bytes());
        for _ in 0..height {
            bytes.extend_from_slice(scanline);
        }
        if let Some(palette) = palette {
            bytes.push(12);
            bytes.extend_from_slice(palette);
        }
        bytes
    }

    fn palette() -> Vec<u8> {
        (0..768).map(|index| (index % 256) as u8).collect()
    }

    #[test]
    fn still_validates_pixels() {
        assert!(cinematic_still(2, 1, vec![0u8; 8]).is_ok());
        assert!(cinematic_still(2, 2, vec![0u8; 8]).is_err());
        assert!(cinematic_still(0, 1, vec![]).is_err());
    }

    #[test]
    fn indexed_still_expands_palette() {
        let still = cinematic_indexed_still(2, 1, &[0, 1], &palette()).unwrap();
        assert_eq!(still.rgba.len(), 8);
        assert!(cinematic_indexed_still(2, 1, &[0], &palette()).is_err());
    }

    #[test]
    fn pcx_decodes_through_its_own_palette() {
        let bytes = pcx(2, 1, &[3, 4], Some(&palette()));
        match cinematic_pcx(&bytes, "<test>").unwrap() {
            CinematicSource::Image {
                width, height, rgba, ..
            } => {
                assert_eq!((width, height), (2, 1));
                assert_eq!(rgba, vec![9, 10, 11, 255, 12, 13, 14, 255]);
            }
            source => panic!("expected image, got {source:?}"),
        }
        // RLE runs expand before palette lookup.
        let bytes = pcx(2, 1, &[0xC2, 7], Some(&palette()));
        match cinematic_pcx(&bytes, "<test>").unwrap() {
            CinematicSource::Image { rgba, .. } => {
                assert_eq!(rgba, vec![21, 22, 23, 255, 21, 22, 23, 255]);
            }
            source => panic!("expected image, got {source:?}"),
        }
    }

    #[test]
    fn pcx_without_palette_is_an_error() {
        let bytes = pcx(2, 1, &[3, 4], None);
        let error = cinematic_pcx(&bytes, "<test>").unwrap_err();
        assert_eq!(error.to_string(), "A cinematic PCX needs its own palette");
    }
}
