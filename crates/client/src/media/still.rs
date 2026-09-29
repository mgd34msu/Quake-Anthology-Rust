//! Stills: PCX-backed poster sources.
//!
//! Donor provenance: `src/media/still.ts` (`cinematicPcx`). Stills play
//! through [`crate::media::playback::CinematicPlayback`] image sources.
//!
//! PCX container decode needs image formats (deferred in `qa-content`);
//! this module validates still inputs and expands palette pixels with
//! [`crate::media::containers::cin_rgba`].

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
///
/// PCX decode needs image formats; the palette check
/// (`A cinematic PCX needs its own palette`) runs after decode.
pub fn cinematic_pcx(bytes: &[u8], source: &str) -> Result<CinematicSource, ClientError> {
    let _ = (bytes, source);
    Err(ClientError::DeferredEngine("PCX still decode"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn still_validates_pixels() {
        assert!(cinematic_still(2, 1, vec![0u8; 8]).is_ok());
        assert!(cinematic_still(2, 2, vec![0u8; 8]).is_err());
        assert!(cinematic_still(0, 1, vec![]).is_err());
    }

    #[test]
    fn indexed_still_expands_palette() {
        let palette: Vec<u8> = (0..768).map(|index| (index % 256) as u8).collect();
        let still = cinematic_indexed_still(2, 1, &[0, 1], &palette).unwrap();
        assert_eq!(still.rgba.len(), 8);
        assert!(cinematic_indexed_still(2, 1, &[0], &palette).is_err());
    }

    #[test]
    fn pcx_decode_is_deferred() {
        let err = cinematic_pcx(&[1, 2, 3], "<test>").unwrap_err();
        assert!(matches!(err, ClientError::DeferredEngine(_)));
    }
}
