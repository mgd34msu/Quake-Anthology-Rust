//! Shader cinematic sources (`ShaderCinematicSource`).
//!
//! Donor provenance: `src/materials/cinematic.ts`. A video stage keeps a
//! dynamic image source; the media crate owns playback.

/// A shader video source: a dynamic image plus its handle.
pub trait ShaderCinematicSource {
    /// Current image handle.
    fn image(&self) -> u32;
    /// Whether the source is enabled.
    fn enabled(&self) -> bool {
        true
    }
}

/// A headless fixed cinematic source for synthetic fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedCinematicSource {
    /// Image handle.
    pub image: u32,
    /// Enabled flag.
    pub enabled: bool,
}

impl ShaderCinematicSource for FixedCinematicSource {
    fn image(&self) -> u32 {
        self.image
    }

    fn enabled(&self) -> bool {
        self.enabled
    }
}
