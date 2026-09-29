//! Cinematic presentation: dimensions, upload images, fullscreen.
//!
//! Donor provenance: `src/media/presentation.ts`
//! (`cinematicDimensions`, `CinematicImage`, `FullscreenCinematic`).
//!
//! Headless: image handles are `u32` and uploads record
//! [`ImageOperation`] values instead of touching a renderer.

use qa_core::identity::SeatId;

use super::containers::{decode_ogg_movie, parse_cin_header, parse_roq_header, parse_theora_ident};
use super::playback::CinematicSource;
use super::source::MemMedia;
use super::types::{
    CinematicEndReason, CinematicFrame, CinematicHost, CinematicStatus, CinematicTarget, CinematicTimeline,
};
use crate::ClientError;

/// Cinematic dimensions (`CinematicDimensions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CinematicDimensions {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
}

/// Read cinematic dimensions without decoding (`cinematicDimensions`).
pub fn cinematic_dimensions(source: &CinematicSource) -> Result<CinematicDimensions, ClientError> {
    match source {
        CinematicSource::Image { width, height, .. } => Ok(CinematicDimensions {
            width: *width,
            height: *height,
        }),
        CinematicSource::Cin { source, bytes } => {
            let mut input = MemMedia::new(bytes.clone(), source);
            match parse_cin_header(&mut input) {
                Ok(header) => Ok(CinematicDimensions {
                    width: header.width as usize,
                    height: header.height as usize,
                }),
                Err(_) => Err(ClientError::BadMedia("Invalid CIN dimensions".to_string())),
            }
        }
        CinematicSource::Roq { bytes, .. } => {
            use super::containers::{
                ROQ_AUDIO_MONO, ROQ_AUDIO_STEREO, ROQ_CODEBOOK, ROQ_FRAME, ROQ_HANG, ROQ_INFO, ROQ_MAGIC, ROQ_PACKET,
                ROQ_QUAD_JPEG,
            };
            let no_video = || ClientError::BadMedia("Cinematic contains no video info".to_string());
            let mut offset = parse_roq_header(bytes, "<cinematic>").map(|(_, offset)| offset)?;
            loop {
                if offset + 8 > bytes.len() {
                    return Err(no_video());
                }
                let header = super::containers::parse_roq_chunk_header(bytes, offset, "<cinematic>")?;
                if header.size > 65536 || header.id == ROQ_MAGIC {
                    return Err(no_video());
                }
                if header.id == ROQ_INFO {
                    if offset + 8 + 4 > bytes.len() {
                        return Err(no_video());
                    }
                    let width = u16::from_le_bytes([bytes[offset + 8], bytes[offset + 9]]) as usize;
                    let height = u16::from_le_bytes([bytes[offset + 10], bytes[offset + 11]]) as usize;
                    return Ok(CinematicDimensions { width, height });
                }
                if !matches!(
                    header.id,
                    ROQ_CODEBOOK
                        | ROQ_FRAME
                        | ROQ_QUAD_JPEG
                        | ROQ_HANG
                        | ROQ_AUDIO_MONO
                        | ROQ_AUDIO_STEREO
                        | ROQ_PACKET
                ) {
                    return Err(no_video());
                }
                offset += 8 + if matches!(header.id, ROQ_HANG | ROQ_PACKET) {
                    0
                } else {
                    header.size
                };
            }
        }
        CinematicSource::Ogv { bytes, .. } => {
            let movie = decode_ogg_movie(bytes)?;
            let Some(first) = movie.video.first() else {
                return Err(ClientError::BadMedia(
                    "Ogg movie has no complete Theora video".to_string(),
                ));
            };
            let ident = parse_theora_ident(first)?;
            Ok(CinematicDimensions {
                width: ident.width,
                height: ident.height,
            })
        }
    }
}

/// An image resource operation (headless `ImageResourceOperation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageOperation {
    /// Upload pixels.
    Upload {
        /// Image handle.
        image: u32,
        /// Width.
        width: usize,
        /// Height.
        height: usize,
        /// RGBA pixels.
        rgba: Vec<u8>,
    },
    /// Release the image.
    Release {
        /// Image handle.
        image: u32,
    },
}

/// A cinematic upload image (`CinematicImage`).
pub struct CinematicImage {
    image: u32,
    width: usize,
    height: usize,
    revision: i32,
    closed: bool,
    allocate: Box<dyn Fn(usize, usize) -> u32>,
}

impl std::fmt::Debug for CinematicImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CinematicImage")
            .field("image", &self.image)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("revision", &self.revision)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl CinematicImage {
    /// New image.
    #[must_use]
    pub fn new(image: u32, width: usize, height: usize, allocate: Box<dyn Fn(usize, usize) -> u32>) -> Self {
        Self {
            image,
            width,
            height,
            revision: -1,
            closed: false,
            allocate,
        }
    }

    /// Current image handle.
    #[must_use]
    pub const fn image(&self) -> u32 {
        self.image
    }

    /// Current dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Prepare an upload (`prepare`).
    pub fn prepare(
        &mut self,
        frame: &CinematicFrame,
        revision: i32,
        apply: &mut dyn FnMut(ImageOperation),
    ) -> Result<(), ClientError> {
        if self.closed {
            return Err(ClientError::BadMedia("Cinematic image is closed".to_string()));
        }
        if revision == self.revision {
            return Ok(());
        }
        if frame.width == 0 || frame.height == 0 {
            return Err(ClientError::BadMedia("Invalid cinematic frame pixels".to_string()));
        }
        self.image = (self.allocate)(frame.width, frame.height);
        self.width = frame.width;
        self.height = frame.height;
        self.revision = revision;
        apply(ImageOperation::Upload {
            image: self.image,
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba.clone(),
        });
        Ok(())
    }

    /// Resolve an upload (`resolve`).
    pub fn resolve(
        &mut self,
        frame: &CinematicFrame,
        revision: i32,
        apply: &mut dyn FnMut(ImageOperation),
    ) -> Result<u32, ClientError> {
        self.prepare(frame, revision, apply)?;
        Ok(self.image)
    }

    /// Release the image (`release`).
    pub fn release(&mut self) -> Option<ImageOperation> {
        if self.closed {
            return None;
        }
        self.closed = true;
        Some(ImageOperation::Release { image: self.image })
    }
}

/// A fullscreen cinematic (`FullscreenCinematic`).
pub struct FullscreenCinematic<'a> {
    playback: super::playback::CinematicPlayback,
    texture: CinematicImage,
    host: &'a mut dyn CinematicHost,
    focus_paused: bool,
}

impl<'a> FullscreenCinematic<'a> {
    /// Open a fullscreen cinematic.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        seat: SeatId,
        source: &CinematicSource,
        wall_now: f64,
        loop_playback: bool,
        hold: bool,
        silent: bool,
        image: u32,
        width: usize,
        height: usize,
        allocate: Box<dyn Fn(usize, usize) -> u32>,
        host: &'a mut dyn CinematicHost,
    ) -> Result<Self, ClientError> {
        let playback = super::playback::CinematicPlayback::open(
            source,
            CinematicTarget::Seat(seat),
            wall_now,
            loop_playback,
            hold,
            silent,
        )?;
        Ok(Self {
            playback,
            texture: CinematicImage::new(image, width, height, allocate),
            host,
            focus_paused: false,
        })
    }

    /// Prepare a frame (`prepare`).
    pub fn prepare(
        &mut self,
        wall_now: f64,
        focus: &str,
        apply: &mut dyn FnMut(ImageOperation),
    ) -> Result<FullscreenFrame, ClientError> {
        if self.playback.status() == CinematicStatus::Stopped {
            return Ok(FullscreenFrame {
                status: CinematicStatus::Stopped,
                frame: None,
                texture: self.texture.image(),
            });
        }
        let paused = focus != "game";
        if paused != self.focus_paused {
            self.playback.pause(paused, wall_now, self.host)?;
            self.focus_paused = paused;
        }
        let tick = self.playback.tick(wall_now, self.host)?;
        if tick.status == CinematicStatus::Stopped {
            return Ok(FullscreenFrame {
                status: CinematicStatus::Stopped,
                frame: None,
                texture: self.texture.image(),
            });
        }
        // Source RoQ may publish INFO before its first image.
        match &tick.frame {
            None => Ok(FullscreenFrame {
                status: tick.status,
                frame: None,
                texture: self.texture.image(),
            }),
            Some(frame) => {
                let blank = frame.rgba.iter().all(|byte| *byte == 0);
                let texture = if blank {
                    None
                } else {
                    Some(self.texture.resolve(frame, self.playback.revision() as i32, apply)?)
                };
                Ok(FullscreenFrame {
                    status: tick.status,
                    frame: Some(frame.clone()),
                    texture: texture.unwrap_or_else(|| self.texture.image()),
                })
            }
        }
    }

    /// Complete the cinematic.
    pub fn complete(&mut self, reason: CinematicEndReason) -> Result<(), ClientError> {
        match reason {
            CinematicEndReason::Finished | CinematicEndReason::Skipped => self.playback.skip(self.host),
            CinematicEndReason::Stopped => self.playback.stop(self.host),
        }
    }

    /// Timeline.
    pub fn timeline(&self, wall_now: f64) -> Result<CinematicTimeline, ClientError> {
        self.playback.timeline(wall_now)
    }

    /// Capture focus-pause state.
    #[must_use]
    pub const fn focus_paused(&self) -> bool {
        self.focus_paused
    }

    /// Close the cinematic.
    pub fn close(&mut self) -> Result<Option<ImageOperation>, ClientError> {
        self.playback.close(self.host)?;
        Ok(self.texture.release())
    }
}

/// A prepared fullscreen frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FullscreenFrame {
    /// Status.
    pub status: CinematicStatus,
    /// Frame.
    pub frame: Option<CinematicFrame>,
    /// Texture handle.
    pub texture: u32,
}

/// Letterboxed pixel rect (`cinematicPixelRect`).
#[must_use]
pub fn cinematic_pixel_rect(video_width: f32, video_height: f32, target_width: f32, target_height: f32) -> [f32; 4] {
    let scale = (target_width / video_width).min(target_height / video_height);
    let width = (video_width * scale).trunc();
    let height = (video_height * scale).trunc();
    [
        ((target_width - width) / 2.0).trunc(),
        ((target_height - height) / 2.0).trunc(),
        width,
        height,
    ]
}

/// CIN frame rate in frames per second.
pub use super::containers::CIN_FRAME_RATE as CIN_FPS;

#[cfg(test)]
mod tests {
    use super::*;

    struct NullHost;

    impl CinematicHost for NullHost {
        fn on_audio(&mut self, _audio: &super::super::types::CinematicAudio, _target: &CinematicTarget) {}

        fn on_audio_reset(&mut self, _target: &CinematicTarget) {}

        fn on_audio_pause(&mut self, _paused: bool, _target: &CinematicTarget) {}

        fn on_complete(&mut self, _reason: CinematicEndReason, _target: &CinematicTarget) {}
    }

    #[test]
    fn dimensions_read_containers() {
        let image = CinematicSource::Image {
            source: "poster".to_string(),
            width: 4,
            height: 2,
            rgba: vec![0u8; 32],
        };
        assert_eq!(
            cinematic_dimensions(&image).unwrap(),
            CinematicDimensions { width: 4, height: 2 }
        );
        let mut roq = vec![0u8; 8 + 8 + 8];
        roq[0..2].copy_from_slice(&super::super::containers::ROQ_MAGIC.to_le_bytes());
        roq[6..8].copy_from_slice(&30u16.to_le_bytes());
        roq[8..10].copy_from_slice(&super::super::containers::ROQ_INFO.to_le_bytes());
        roq[10..14].copy_from_slice(&8u32.to_le_bytes());
        roq[16..18].copy_from_slice(&64u16.to_le_bytes());
        roq[18..20].copy_from_slice(&32u16.to_le_bytes());
        let source = CinematicSource::Roq {
            source: "test.roq".to_string(),
            bytes: roq,
        };
        assert_eq!(
            cinematic_dimensions(&source).unwrap(),
            CinematicDimensions { width: 64, height: 32 }
        );
    }

    #[test]
    fn image_uploads_once_per_revision() {
        let mut image = CinematicImage::new(1, 2, 2, Box::new(|_, _| 7));
        let frame = CinematicFrame {
            rgba: vec![1u8; 16],
            width: 2,
            height: 2,
            index: 0,
            pass: 0,
            source_time: 0.0,
            time: 0.0,
            decoded: true,
        };
        let mut operations = Vec::new();
        image
            .prepare(&frame, 3, &mut |operation| operations.push(operation))
            .unwrap();
        image
            .prepare(&frame, 3, &mut |operation| operations.push(operation))
            .unwrap();
        assert_eq!(operations.len(), 1);
        assert!(image.release().is_some());
        assert!(image.release().is_none());
    }

    #[test]
    fn pixel_rect_letterboxes() {
        assert_eq!(
            cinematic_pixel_rect(320.0, 240.0, 640.0, 480.0),
            [0.0, 0.0, 640.0, 480.0]
        );
        assert_eq!(
            cinematic_pixel_rect(320.0, 240.0, 640.0, 400.0),
            [53.0, 0.0, 533.0, 400.0]
        );
        let _ = CIN_FPS;
    }

    #[test]
    fn fullscreen_prepares_still() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = NullHost;
        let source = CinematicSource::Image {
            source: "poster".to_string(),
            width: 2,
            height: 2,
            rgba: vec![9u8; 16],
        };
        let mut fullscreen = FullscreenCinematic::open(
            owner.seat(0),
            &source,
            0.0,
            false,
            false,
            true,
            1,
            2,
            2,
            Box::new(|_, _| 7),
            &mut host,
        )
        .unwrap();
        let mut operations = Vec::new();
        let frame = fullscreen
            .prepare(0.0, "game", &mut |operation| operations.push(operation))
            .unwrap();
        assert_eq!(frame.status, CinematicStatus::Held);
        assert_eq!(operations.len(), 1);
    }
}
