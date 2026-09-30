//! Menu artwork decoding.
//!
//! Donor provenance: `src/app/bootstrap/menu-art.ts` (`loadMenuArtImage`).
//!
//! Sync adaptation: the donor reads Bun-embedded PNG files with caching and
//! per-call pixel copies. The sync port is a pure function over injected
//! bytes through [`qa_content::images::png::decode_png`] with no cache —
//! callers that need the donor's memoization keep the returned image. The
//! four known asset paths become [`MenuArtName`]; anything else is the
//! donor's "Unknown menu artwork" error.

use qa_content::images::png::decode_png;
use qa_content::images::ContentError;
use thiserror::Error;

/// Known menu artwork assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MenuArtName {
    /// Main menu background.
    MainBackground,
    /// Menu background.
    Background,
    /// Menu panel.
    Panel,
    /// Menu focus highlight.
    Focus,
}

impl MenuArtName {
    /// Asset resource path.
    #[must_use]
    pub fn path(self) -> &'static str {
        match self {
            Self::MainBackground => "assets/ui/main-menu-background.png",
            Self::Background => "assets/ui/menu-background.png",
            Self::Panel => "assets/ui/menu-panel.png",
            Self::Focus => "assets/ui/menu-focus.png",
        }
    }

    /// Parse an asset path, rejecting unknown artwork.
    pub fn parse(path: &str) -> Result<Self, MenuArtError> {
        match path {
            "assets/ui/main-menu-background.png" => Ok(Self::MainBackground),
            "assets/ui/menu-background.png" => Ok(Self::Background),
            "assets/ui/menu-panel.png" => Ok(Self::Panel),
            "assets/ui/menu-focus.png" => Ok(Self::Focus),
            _ => Err(MenuArtError::Unknown(path.to_string())),
        }
    }
}

/// Decoded menu artwork (donor `ImageLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuArtImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
}

/// Failure decoding menu artwork.
#[derive(Debug, Error)]
pub enum MenuArtError {
    /// Unknown asset path.
    #[error("Unknown menu artwork: {0}")]
    Unknown(String),
    /// PNG decoding failed.
    #[error(transparent)]
    Decode(#[from] ContentError),
}

/// Decode artwork bytes for a known asset.
pub fn decode_menu_art(name: MenuArtName, bytes: &[u8]) -> Result<MenuArtImage, MenuArtError> {
    let image = decode_png(bytes, name.path())?;
    Ok(MenuArtImage {
        width: image.width,
        height: image.height,
        pixels: image.pixels,
    })
}

/// Decode artwork bytes addressed by asset path.
pub fn load_menu_art_image(path: &str, bytes: &[u8]) -> Result<MenuArtImage, MenuArtError> {
    let name = MenuArtName::parse(path)?;
    decode_menu_art(name, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::images::png::encode_png;

    #[test]
    fn known_paths_roundtrip() {
        for name in [
            MenuArtName::MainBackground,
            MenuArtName::Background,
            MenuArtName::Panel,
            MenuArtName::Focus,
        ] {
            assert_eq!(MenuArtName::parse(name.path()).unwrap(), name);
        }
    }

    #[test]
    fn unknown_path_errors() {
        let error = MenuArtName::parse("assets/ui/other.png").unwrap_err();
        assert_eq!(error.to_string(), "Unknown menu artwork: assets/ui/other.png");
        assert!(matches!(error, MenuArtError::Unknown(_)));
    }

    #[test]
    fn decode_roundtrip_and_invalid_bytes() {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0];
        let bytes = encode_png(2, 2, &rgba).unwrap();
        let image = load_menu_art_image("assets/ui/menu-panel.png", &bytes).unwrap();
        assert_eq!(image.width, 2);
        assert_eq!(image.height, 2);
        assert_eq!(image.pixels, rgba);
        assert!(matches!(
            load_menu_art_image("assets/ui/menu-panel.png", b"nope").unwrap_err(),
            MenuArtError::Decode(_)
        ));
        assert!(matches!(
            load_menu_art_image("assets/ui/other.png", &bytes).unwrap_err(),
            MenuArtError::Unknown(_)
        ));
    }
}
