//! Material cinematics: shader video sources.
//!
//! Donor provenance: `src/media/material.ts` (`MaterialCinematic`).

use super::playback::CinematicPlayback;
use super::presentation::{CinematicImage, ImageOperation};
use super::types::CinematicTarget;
use crate::materials::cinematic::ShaderCinematicSource;
use crate::ClientError;

/// A material cinematic (`MaterialCinematic`).
pub struct MaterialCinematic {
    /// Playback.
    pub playback: CinematicPlayback,
    texture: CinematicImage,
    enabled: bool,
    uploaded: Box<dyn FnMut(ImageOperation)>,
}

impl MaterialCinematic {
    /// New material cinematic.
    pub fn new(
        playback: CinematicPlayback,
        image: u32,
        width: usize,
        height: usize,
        allocate: Box<dyn Fn(usize, usize) -> u32>,
        uploaded: Box<dyn FnMut(ImageOperation)>,
    ) -> Result<Self, ClientError> {
        if !matches!(playback.target(), CinematicTarget::Material(_)) {
            return Err(ClientError::BadMedia(
                "Material cinematic requires a material target".to_string(),
            ));
        }
        Ok(Self {
            playback,
            texture: CinematicImage::new(image, width, height, allocate),
            enabled: true,
            uploaded,
        })
    }

    /// Enable or disable.
    pub fn set_enabled(
        &mut self,
        enabled: bool,
        wall_now: f64,
        host: &mut dyn super::types::CinematicHost,
    ) -> Result<(), ClientError> {
        self.enabled = enabled;
        self.playback.pause(!enabled, wall_now, host)
    }

    /// Resolve the current texture (`resolve`).
    pub fn resolve(
        &mut self,
        wall_now: f64,
        host: &mut dyn super::types::CinematicHost,
        apply: &mut dyn FnMut(ImageOperation),
    ) -> Result<u32, ClientError> {
        let tick = if self.enabled {
            Some(self.playback.tick(wall_now, host)?)
        } else {
            None
        };
        let frame = tick
            .as_ref()
            .and_then(|tick| tick.frame.clone())
            .or_else(|| self.playback.current_frame());
        // Source RoQ may publish INFO before its first image. Keep the
        // texture clear until a frame exists.
        let (width, height) = self.texture.dimensions();
        let clear = super::types::CinematicFrame {
            rgba: vec![0u8; width * height * 4],
            width,
            height,
            index: 0,
            pass: 0,
            source_time: 0.0,
            time: 0.0,
            decoded: true,
        };
        let picture = frame.clone().unwrap_or(clear);
        let revision = match &tick {
            None => -2,
            Some(_) => self.playback.revision() as i32,
        };
        let uploaded = &mut self.uploaded;
        self.texture.resolve(&picture, revision, &mut |operation| {
            apply(operation.clone());
            uploaded(operation);
        })
    }

    /// Close the cinematic.
    pub fn close(&mut self, host: &mut dyn super::types::CinematicHost) -> Result<Option<ImageOperation>, ClientError> {
        let release = self.texture.release();
        self.playback.close(host)?;
        Ok(release)
    }
}

impl ShaderCinematicSource for MaterialCinematic {
    fn image(&self) -> u32 {
        self.texture.image()
    }

    fn enabled(&self) -> bool {
        self.enabled
    }
}
