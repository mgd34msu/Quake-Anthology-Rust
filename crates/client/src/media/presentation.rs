//! Cinematic presentation: dimensions, upload images, fullscreen.
//!
//! Donor provenance: `src/media/presentation.ts`
//! (`cinematicDimensions`, `CinematicImage`, `FullscreenCinematic`,
//! `cinematicPixelRect`).
//!
//! Headless: image handles are `u32` and uploads record
//! [`ImageOperation`] values instead of touching a renderer.

use qa_core::binary::BinaryReader;
use qa_core::identity::SeatId;

use super::containers::decode_ogg_movie;
use super::playback::CinematicSource;
use super::roq::{RoqChunkEvent, RoqChunkHooks, RoqDecoder, RoqDecoderOptions};
use super::source::{read_media, MemMedia};
use super::theora::TheoraDecoder;
use super::types::{
    CinematicEndReason, CinematicFrame, CinematicHost, CinematicStatus, CinematicTarget, CinematicTimeline,
};
use crate::ClientError;

/// Cinematic dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CinematicDimensions {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
}

/// Read cinematic dimensions without retaining a decoder
/// (`cinematicDimensions`).
pub fn cinematic_dimensions(source: &CinematicSource) -> Result<CinematicDimensions, ClientError> {
    match source {
        CinematicSource::Image { width, height, .. } => Ok(CinematicDimensions {
            width: *width,
            height: *height,
        }),
        CinematicSource::Cin { source, bytes } => {
            let mut input = MemMedia::new(bytes.clone(), source);
            let header = read_media(&mut input, 0, 8)?;
            let mut reader = BinaryReader::new(&header, source);
            let width = reader.i32().map_err(|error| ClientError::BadMedia(error.to_string()))?;
            let height = reader.i32().map_err(|error| ClientError::BadMedia(error.to_string()))?;
            if width <= 0 || height <= 0 || i64::from(width) * i64::from(height) > 0x1000000 {
                return Err(ClientError::BadMedia("Invalid CIN dimensions".to_string()));
            }
            Ok(CinematicDimensions {
                width: width as usize,
                height: height as usize,
            })
        }
        CinematicSource::Roq { source, bytes } => {
            if bytes.is_empty() {
                return Err(ClientError::BadMedia(format!("Empty cinematic: {source}")));
            }
            let mut decoder = RoqDecoder::from_bytes(
                bytes.clone(),
                source,
                RoqDecoderOptions {
                    end_policy: super::containers::RoqEndPolicy::CinematicLookahead,
                    silent: true,
                    scratch: None,
                },
            )?;
            loop {
                match decoder.next_chunk(RoqChunkHooks::default())? {
                    RoqChunkEvent::Info { width, height } => {
                        return Ok(CinematicDimensions { width, height });
                    }
                    RoqChunkEvent::End => {
                        return Err(ClientError::BadMedia(format!(
                            "Cinematic contains no video info: {source}"
                        )));
                    }
                    _ => {}
                }
            }
        }
        CinematicSource::Ogv { bytes, .. } => {
            let movie = decode_ogg_movie(bytes)?;
            let mut decoder = TheoraDecoder::new(&movie.video[..3])?;
            let dimensions = CinematicDimensions {
                width: decoder.width(),
                height: decoder.height(),
            };
            decoder.close();
            Ok(dimensions)
        }
    }
}

/// An image resource operation (headless `ImageResourceOperation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageOperation {
    /// Create the image.
    Create {
        /// Image handle.
        image: u32,
        /// Width.
        width: usize,
        /// Height.
        height: usize,
        /// RGBA pixels.
        rgba: Vec<u8>,
    },
    /// Update the image in place.
    Update {
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

struct PendingUpload {
    operations: Vec<ImageOperation>,
    target: (u32, usize, usize),
    next: usize,
    revision: i32,
}

/// A cinematic upload image (`CinematicImage`).
pub struct CinematicImage {
    current: (u32, usize, usize),
    allocate: Box<dyn Fn(usize, usize) -> u32>,
    uploaded: bool,
    uploaded_revision: i32,
    closed: bool,
    executing: bool,
    pending: Option<PendingUpload>,
    completed: bool,
}

impl std::fmt::Debug for CinematicImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CinematicImage")
            .field("image", &self.current.0)
            .field("width", &self.current.1)
            .field("height", &self.current.2)
            .field("revision", &self.uploaded_revision)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl CinematicImage {
    /// New image.
    #[must_use]
    pub fn new(image: u32, width: usize, height: usize, allocate: Box<dyn Fn(usize, usize) -> u32>) -> Self {
        Self {
            current: (image, width, height),
            allocate,
            uploaded: false,
            uploaded_revision: -1,
            closed: false,
            executing: false,
            pending: None,
            completed: false,
        }
    }

    /// Current image handle (the pending target while an upload is
    /// in flight).
    #[must_use]
    pub fn image(&self) -> u32 {
        self.pending.as_ref().map_or(self.current.0, |pending| pending.target.0)
    }

    /// Current dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> (usize, usize) {
        (self.current.1, self.current.2)
    }

    /// Prepare an upload (`prepare`); returns the operations to apply,
    /// or `None` when the revision is already uploaded.
    pub fn prepare(
        &mut self,
        frame: &CinematicFrame,
        revision: i32,
    ) -> Result<Option<Vec<ImageOperation>>, ClientError> {
        if self.closed {
            return Err(ClientError::BadMedia("Cinematic image is closed".to_string()));
        }
        if let Some(pending) = &self.pending {
            return Ok(Some(pending.operations.clone()));
        }
        if revision == self.uploaded_revision {
            return Ok(None);
        }
        let resized = frame.width != self.current.1 || frame.height != self.current.2;
        let target = if resized {
            ((self.allocate)(frame.width, frame.height), frame.width, frame.height)
        } else {
            self.current
        };
        let mut operations = Vec::new();
        if resized && self.uploaded {
            operations.push(ImageOperation::Release { image: self.current.0 });
        }
        let content = frame.rgba.clone();
        if self.uploaded && !resized {
            operations.push(ImageOperation::Update {
                image: target.0,
                width: frame.width,
                height: frame.height,
                rgba: content,
            });
        } else {
            operations.push(ImageOperation::Create {
                image: target.0,
                width: frame.width,
                height: frame.height,
                rgba: content,
            });
        }
        self.pending = Some(PendingUpload {
            operations: operations.clone(),
            target,
            next: 0,
            revision,
        });
        self.completed = false;
        Ok(Some(operations))
    }

    /// Mark the pending upload complete (`upload.complete`).
    pub fn complete(&mut self) -> Result<(), ClientError> {
        if self.completed || self.pending.is_none() {
            return Err(ClientError::BadMedia(
                "Cinematic upload completion was already used".to_string(),
            ));
        }
        if self.closed {
            return Err(ClientError::BadMedia(
                "Cinematic upload completed after image release".to_string(),
            ));
        }
        let Some(pending) = self.pending.take() else {
            return Err(ClientError::BadMedia(
                "Cinematic upload completion was already used".to_string(),
            ));
        };
        self.current = pending.target;
        self.uploaded = true;
        self.uploaded_revision = pending.revision;
        self.completed = true;
        Ok(())
    }

    /// Resolve an upload (`resolve`): apply pending operations, then
    /// complete.
    pub fn resolve(
        &mut self,
        frame: &CinematicFrame,
        revision: i32,
        apply: &mut dyn FnMut(ImageOperation),
    ) -> Result<u32, ClientError> {
        if self.executing {
            return Err(ClientError::BadMedia("Cinematic upload cannot reenter".to_string()));
        }
        self.executing = true;
        let result = self.resolve_inner(frame, revision, apply);
        self.executing = false;
        result
    }

    fn resolve_inner(
        &mut self,
        frame: &CinematicFrame,
        revision: i32,
        apply: &mut dyn FnMut(ImageOperation),
    ) -> Result<u32, ClientError> {
        self.prepare(frame, revision)?;
        if let Some(pending) = self.pending.as_mut() {
            while pending.next < pending.operations.len() {
                let operation = pending.operations[pending.next].clone();
                pending.next += 1;
                if matches!(operation, ImageOperation::Release { .. }) {
                    self.uploaded = false;
                }
                apply(operation);
            }
        }
        if self.pending.is_some() {
            self.complete()?;
        }
        Ok(self.current.0)
    }

    /// Release the image (`release`).
    pub fn release(&mut self) -> Option<ImageOperation> {
        if self.closed {
            return None;
        }
        self.closed = true;
        // The pending upload handle survives so `complete` reports
        // release instead of reuse; `prepare` and `resolve` refuse
        // closed images before reaching it.
        self.uploaded
            .then_some(ImageOperation::Release { image: self.current.0 })
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
        if focus != "game" && self.playback.status() == CinematicStatus::Playing {
            self.focus_paused = true;
            self.playback.pause(true, wall_now, self.host)?;
        } else if focus == "game" && self.focus_paused {
            self.focus_paused = false;
            self.playback.pause(false, wall_now, self.host)?;
        }
        let tick = self.playback.tick(wall_now, self.host)?;
        let visible = focus != "menu"
            && tick.status != CinematicStatus::Ended
            && tick.status != CinematicStatus::Stopped
            && tick.frame.is_some();
        if visible {
            if let Some(frame) = &tick.frame {
                let revision = self.playback.revision() as i32;
                self.texture.resolve(frame, revision, apply)?;
            }
        }
        Ok(FullscreenFrame {
            status: tick.status,
            frame: tick.frame,
            texture: self.texture.image(),
            blank: !visible,
        })
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

    /// Restore focus-pause state.
    pub fn restore_focus_paused(&mut self, focus_paused: bool) {
        self.focus_paused = focus_paused;
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
    /// Whether callers clear only this viewport for blank frames.
    pub blank: bool,
}

/// A pixel rectangle (`Rect`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CinematicRect {
    /// X.
    pub x: f64,
    /// Y.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Q3's source `SCR_AdjustFrom640` stretches within the selected
/// seat's viewport (`cinematicPixelRect`).
#[must_use]
pub fn cinematic_pixel_rect(rect: &CinematicRect, viewport: &CinematicRect) -> CinematicRect {
    CinematicRect {
        x: viewport.x + f64::from((rect.x as f32 * ((viewport.width / 640.0) as f32)).trunc()),
        y: viewport.y + f64::from((rect.y as f32 * ((viewport.height / 480.0) as f32)).trunc()),
        width: f64::from((rect.width as f32 * ((viewport.width / 640.0) as f32)).trunc()),
        height: f64::from((rect.height as f32 * ((viewport.height / 480.0) as f32)).trunc()),
    }
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

    fn frame(width: usize, height: usize) -> CinematicFrame {
        CinematicFrame {
            rgba: vec![1u8; width * height * 4],
            width,
            height,
            index: 0,
            pass: 0,
            source_time: 0.0,
            time: 0.0,
            decoded: true,
        }
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
        let mut cin = vec![0u8; 8];
        cin[..4].copy_from_slice(&64i32.to_le_bytes());
        cin[4..8].copy_from_slice(&32i32.to_le_bytes());
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: cin,
        };
        assert_eq!(
            cinematic_dimensions(&source).unwrap(),
            CinematicDimensions { width: 64, height: 32 }
        );
        let source = CinematicSource::Cin {
            source: "test.cin".to_string(),
            bytes: vec![0u8; 8],
        };
        assert!(cinematic_dimensions(&source).is_err());

        // The lookahead scan needs input past the INFO chunk.
        let mut roq = vec![0u8; 8 + 8 + 8 + 8 + 2];
        roq[0..2].copy_from_slice(&super::super::containers::ROQ_MAGIC.to_le_bytes());
        roq[6..8].copy_from_slice(&30u16.to_le_bytes());
        roq[8..10].copy_from_slice(&super::super::containers::ROQ_INFO.to_le_bytes());
        roq[10..14].copy_from_slice(&8u32.to_le_bytes());
        roq[16..18].copy_from_slice(&64u16.to_le_bytes());
        roq[18..20].copy_from_slice(&32u16.to_le_bytes());
        roq[24..26].copy_from_slice(&super::super::containers::ROQ_FRAME.to_le_bytes());
        roq[26..30].copy_from_slice(&2u32.to_le_bytes());
        let source = CinematicSource::Roq {
            source: "test.roq".to_string(),
            bytes: roq,
        };
        assert_eq!(
            cinematic_dimensions(&source).unwrap(),
            CinematicDimensions { width: 64, height: 32 }
        );
        let source = CinematicSource::Roq {
            source: "empty.roq".to_string(),
            bytes: Vec::new(),
        };
        assert!(cinematic_dimensions(&source).is_err());
        // Fake OGV framing reaches the native Theora decoder, which
        // rejects it.
        let source = CinematicSource::Ogv {
            source: "fake.ogv".to_string(),
            bytes: vec![0u8; 64],
        };
        assert!(cinematic_dimensions(&source).is_err());
    }

    #[test]
    fn uploads_create_update_and_release() {
        let mut image = CinematicImage::new(1, 2, 2, Box::new(|_, _| 7));
        let picture = frame(2, 2);
        let operations = image.prepare(&picture, 3).unwrap().unwrap();
        assert_eq!(operations.len(), 1);
        assert!(matches!(operations[0], ImageOperation::Create { image: 1, .. }));
        // A pending upload is returned again until completed.
        assert!(image.prepare(&picture, 4).unwrap().is_some());
        image.complete().unwrap();
        assert!(image.complete().is_err());
        // Same-size uploads update in place.
        let operations = image.prepare(&picture, 4).unwrap().unwrap();
        assert!(matches!(operations[0], ImageOperation::Update { .. }));
        image.complete().unwrap();
        // Resizes release before recreating.
        let wide = frame(4, 2);
        let operations = image.prepare(&wide, 5).unwrap().unwrap();
        assert_eq!(operations.len(), 2);
        assert!(matches!(operations[0], ImageOperation::Release { .. }));
        assert!(matches!(operations[1], ImageOperation::Create { image: 7, .. }));
        image.complete().unwrap();
        assert_eq!(image.image(), 7);
        assert!(image.release().is_some());
        assert!(image.release().is_none());
        assert!(image.prepare(&picture, 6).is_err());
    }

    #[test]
    fn resolve_applies_and_completes() {
        let mut image = CinematicImage::new(1, 2, 2, Box::new(|_, _| 7));
        let picture = frame(2, 2);
        let mut operations = Vec::new();
        let handle = image
            .resolve(&picture, 3, &mut |operation| operations.push(operation))
            .unwrap();
        assert_eq!(handle, 1);
        assert_eq!(operations.len(), 1);
        // The uploaded revision resolves without new operations.
        let handle = image
            .resolve(&picture, 3, &mut |operation| operations.push(operation))
            .unwrap();
        assert_eq!(handle, 1);
        assert_eq!(operations.len(), 1);
    }

    #[test]
    fn pixel_rect_stretches_from_640() {
        let rect = CinematicRect {
            x: 320.0,
            y: 240.0,
            width: 32.0,
            height: 32.0,
        };
        let viewport = CinematicRect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        };
        assert_eq!(
            cinematic_pixel_rect(&rect, &viewport),
            CinematicRect {
                x: 400.0,
                y: 300.0,
                width: 40.0,
                height: 40.0,
            }
        );
        let full = CinematicRect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        assert_eq!(cinematic_pixel_rect(&full, &viewport).width, 800.0);
        let _ = CIN_FPS;
    }

    #[test]
    fn fullscreen_handles_focus_and_blank() {
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
        assert!(!frame.blank);
        assert_eq!(operations.len(), 1);
        // Menus blank without uploading.
        let frame = fullscreen
            .prepare(0.0, "menu", &mut |operation| operations.push(operation))
            .unwrap();
        assert!(frame.blank);
        assert_eq!(operations.len(), 1);
        fullscreen.restore_focus_paused(true);
        assert!(fullscreen.focus_paused());
        assert!(fullscreen.close().unwrap().is_some());
    }
}
