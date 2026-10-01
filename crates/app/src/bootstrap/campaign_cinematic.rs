//! Campaign fullscreen cinematic (port of donor
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/campaign-cinematic.ts`).
//!
//! [`CampaignCinematic`] plays one fullscreen movie for campaign, remote
//! server, and standalone requests. Sync adaptations: preparation and restore
//! are synchronous, checkpoints are the typed
//! [`CampaignCinematicCheckpoint`], and the internal millisecond clock stands
//! in for the donor's clock object.
//!
//! The media engine shapes differ from the donor: playback is driven through
//! [`CinematicPlayback`](qa_client::media::playback::CinematicPlayback) with a
//! transient host per call (following `q3_client/cinematics.rs`) instead of
//! holding the donor's callback-wired `FullscreenCinematic`, and texture
//! uploads apply through [`CampaignCinematicImages::commit`] while the
//! executed frame carries only draw commands. Mounts, images, renderer,
//! mixer, and captions are seams because `assets.ts`, `audio.ts`, and
//! `renderer.ts` are out-of-scope siblings.

use qa_client::media::audio::{cinematic_audio, AudioStreamTarget, CinematicAudience, CinematicMixer};
use qa_client::media::playback::{cinematic_bytes, CinematicPlayback, PlaybackCheckpoint};
use qa_client::media::presentation::{cinematic_dimensions, CinematicImage, ImageOperation};
use qa_client::media::still::cinematic_pcx;
use qa_client::media::types::{CinematicAudio, CinematicHost, CinematicStatus, CinematicTarget, CinematicTimeline};
use qa_client::render::types::{
    DrawBuffer, ImageSource, Rect, RenderCommand, RenderFrame, RendererImage, ResourceOwner, TextureRect,
};
use qa_client::ui::types::{SeatInputEvent, SeatInputEventKind};
use qa_content::contract::ResolvedResourceReference;
use qa_core::identity::SeatId;
use qa_core::math::vec4;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Checkpoint version (donor literal `1`).
pub const CINEMATIC_CHECKPOINT_VERSION: u32 = 1;

/// Next mixer lane number (donor `nextCinematic`).
static NEXT_CINEMATIC: AtomicU64 = AtomicU64::new(0);

/// Fullscreen cinematic request (donor `ScreenCinematicRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCinematicRequest {
    /// Movie name, with or without an extension.
    pub name: String,
    /// Loop playback.
    pub loop_playback: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Suppress audio.
    pub silent: bool,
}

/// Cinematic captions (donor `ScreenCinematicCaptions`).
pub trait ScreenCinematicCaptions {
    /// Prepare captions for a movie path.
    fn prepare(&mut self, path: &str);
    /// Caption commands for a timeline and viewport.
    fn commands(&self, timeline: &CinematicTimeline, viewport: &Rect) -> Vec<RenderCommand>;
}

/// Cinematic mount bytes with their resource reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicBytes {
    /// File bytes.
    pub bytes: Vec<u8>,
    /// Resource reference.
    pub reference: ResolvedResourceReference,
}

/// Content mounts for cinematic bytes (donor `content.mounts`).
pub trait CampaignCinematicMounts {
    /// Open a movie path.
    fn open(&mut self, path: &str) -> Option<CinematicBytes>;
}

/// Cinematic images (donor `assets.images`).
pub trait CampaignCinematicImages {
    /// Owning renderer lifetime.
    fn owner(&self) -> ResourceOwner;
    /// Allocate an image handle.
    fn allocate(&mut self, width: usize, height: usize, resource: &ResolvedResourceReference) -> u32;
    /// Apply a texture upload operation.
    fn commit(&mut self, operation: ImageOperation);
}

/// Cinematic renderer (donor `renderer`).
pub trait CampaignCinematicRenderer {
    /// Drawable size in pixels.
    fn drawable_size(&self) -> (u32, u32);
    /// Execute a frame.
    fn execute(&mut self, frame: RenderFrame);
}

/// Cinematic mixer (donor `CinematicOutput["engine"]`).
pub trait CampaignCinematicMixer: CinematicMixer {
    /// Stop all audio.
    fn stop_all(&mut self);
    /// Pump the mixer.
    fn pump(&mut self);
    /// Whether the mixer restores owned PCM checkpoints.
    fn supports_stream_restore(&self) -> bool;
}

/// Campaign cinematic failure.
#[derive(Debug, Clone, PartialEq)]
pub enum CampaignCinematicError {
    /// The request was retired.
    Retired(String),
    /// The movie path is invalid.
    InvalidPath(String),
    /// The movie is missing from content.
    Missing(String),
    /// The movie format is unsupported.
    Unsupported(String),
    /// The restored resource changed.
    ResourceChanged,
    /// The restored clock is negative.
    NegativeClock,
    /// The checkpoint version is unsupported.
    BadCheckpoint(String),
    /// The mixer cannot checkpoint or restore its PCM lane.
    Mixer(String),
    /// Frame ran while the cinematic was not active.
    FrameNotActive,
    /// The frame interval is invalid.
    FrameInterval,
    /// Media failure.
    Media(String),
}

impl std::fmt::Display for CampaignCinematicError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retired(message)
            | Self::InvalidPath(message)
            | Self::Missing(message)
            | Self::Unsupported(message)
            | Self::BadCheckpoint(message)
            | Self::Mixer(message)
            | Self::Media(message) => write!(f, "{message}"),
            Self::ResourceChanged => write!(f, "cinematic resource changed"),
            Self::NegativeClock => write!(f, "Negative cinematic clock"),
            Self::FrameNotActive => write!(f, "Fullscreen cinematic is not the active request"),
            Self::FrameInterval => write!(f, "Invalid cinematic frame interval"),
        }
    }
}

impl std::error::Error for CampaignCinematicError {}

impl From<qa_client::ClientError> for CampaignCinematicError {
    fn from(error: qa_client::ClientError) -> Self {
        Self::Media(error.to_string())
    }
}

/// Owner state (donor `state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CinematicState {
    Prepared,
    Active,
    Closed,
}

/// Campaign cinematic checkpoint (donor `captureCheckpoint`, version 1).
#[derive(Debug, Clone, PartialEq)]
pub struct CampaignCinematicCheckpoint<P> {
    /// Checkpoint version (1).
    pub version: u32,
    /// Original request.
    pub request: ScreenCinematicRequest,
    /// Opened resource reference.
    pub resource: ResolvedResourceReference,
    /// Clock in milliseconds.
    pub clock: f64,
    /// Whether audio started.
    pub audio_started: bool,
    /// Whether focus pause was active.
    pub focus_paused: bool,
    /// Playback checkpoint.
    pub playback: PlaybackCheckpoint,
    /// Audio buffered before activation.
    pub initial_audio: Vec<CinematicAudio>,
    /// Owned PCM lane checkpoint.
    pub pcm: Option<P>,
}

/// Saved preparation state.
enum SavedState<'a, P> {
    Fresh,
    Restored(&'a CampaignCinematicCheckpoint<P>),
}

/// Select and validate the movie path (donor path selection).
pub fn select_cinematic_path(name: &str) -> Result<String, CampaignCinematicError> {
    let stem = name.rsplit('/').next().unwrap_or(name);
    let selected = if stem.contains('.') {
        name.to_string()
    } else {
        format!("{name}.roq")
    };
    if selected.starts_with('/')
        || selected.contains('\\')
        || selected.contains('\0')
        || selected
            .split('/')
            .any(|part| part == ".." || part == "." || part.is_empty())
    {
        return Err(CampaignCinematicError::InvalidPath(
            "Invalid cinematic resource path".to_string(),
        ));
    }
    if selected.starts_with("video/") || selected.starts_with("pics/") {
        return Ok(selected);
    }
    let folder = if selected.to_lowercase().ends_with(".pcx") {
        "pics"
    } else {
        "video"
    };
    Ok(format!("{folder}/{selected}"))
}

/// Campaign fullscreen cinematic (donor `CampaignCinematic`).
pub struct CampaignCinematic<M: CinematicMixer, I, R, C> {
    state: CinematicState,
    playback: CinematicPlayback,
    image: CinematicImage,
    renderer: R,
    images: Rc<RefCell<I>>,
    mixer: M,
    clock_ms: f64,
    lane: String,
    path: String,
    request: ScreenCinematicRequest,
    resource: ResolvedResourceReference,
    captions: Option<C>,
    current: Box<dyn Fn() -> bool>,
    audio_ready: bool,
    audio_started: bool,
    focus_paused: bool,
    pending_audio: Vec<CinematicAudio>,
    pending_pcm: Option<M::StreamCheckpoint>,
}

/// Per-call playback host bridging one mixer lane.
struct CampaignHost<'a, M: CinematicMixer> {
    mixer: &'a mut M,
    lane: &'a str,
    ready: bool,
    pending: &'a mut Vec<CinematicAudio>,
}

impl<M: CinematicMixer> CinematicHost for CampaignHost<'_, M> {
    fn on_audio(&mut self, audio: &CinematicAudio, target: &CinematicTarget) {
        if self.ready {
            cinematic_audio(&mut *self.mixer, self.lane, 1.0).on_audio(audio, target);
        } else {
            self.pending.push(audio.clone());
        }
    }

    fn on_audio_reset(&mut self, _target: &CinematicTarget) {
        if self.ready {
            cinematic_audio(&mut *self.mixer, self.lane, 1.0).on_audio_reset();
        }
    }

    fn on_audio_pause(&mut self, paused: bool, _target: &CinematicTarget) {
        if self.ready {
            cinematic_audio(&mut *self.mixer, self.lane, 1.0).on_audio_pause(paused);
        }
    }

    fn on_complete(&mut self, _reason: qa_client::media::types::CinematicEndReason, _target: &CinematicTarget) {}
}

impl<M, I, R, C> CampaignCinematic<M, I, R, C>
where
    M: CampaignCinematicMixer,
    I: CampaignCinematicImages + 'static,
    R: CampaignCinematicRenderer,
    C: ScreenCinematicCaptions,
{
    /// Prepare a cinematic (donor `prepare`).
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        request: ScreenCinematicRequest,
        mounts: &mut dyn CampaignCinematicMounts,
        images: Rc<RefCell<I>>,
        mixer: M,
        renderer: R,
        seat: SeatId,
        current: Box<dyn Fn() -> bool>,
        captions: Option<C>,
    ) -> Result<Self, CampaignCinematicError> {
        Self::prepare_owned(
            request,
            SavedState::Fresh,
            mounts,
            images,
            mixer,
            renderer,
            seat,
            current,
            captions,
        )
    }

    /// Restore a cinematic (donor `restore`).
    #[allow(clippy::too_many_arguments)]
    pub fn restore(
        checkpoint: &CampaignCinematicCheckpoint<M::StreamCheckpoint>,
        mounts: &mut dyn CampaignCinematicMounts,
        images: Rc<RefCell<I>>,
        mixer: M,
        renderer: R,
        seat: SeatId,
        current: Box<dyn Fn() -> bool>,
        captions: Option<C>,
    ) -> Result<Self, CampaignCinematicError> {
        if checkpoint.version != CINEMATIC_CHECKPOINT_VERSION {
            return Err(CampaignCinematicError::BadCheckpoint(format!(
                "unsupported cinematic checkpoint {}",
                checkpoint.version
            )));
        }
        if !mixer.supports_stream_restore() {
            return Err(CampaignCinematicError::Mixer(
                "Cinematic mixer cannot restore its owned PCM lane".to_string(),
            ));
        }
        if !checkpoint.clock.is_finite() || checkpoint.clock < 0.0 {
            return Err(CampaignCinematicError::NegativeClock);
        }
        Self::prepare_owned(
            checkpoint.request.clone(),
            SavedState::Restored(checkpoint),
            mounts,
            images,
            mixer,
            renderer,
            seat,
            current,
            captions,
        )
    }

    /// Prepare or restore a cinematic (donor `prepareOwned`).
    #[allow(clippy::too_many_arguments)]
    fn prepare_owned(
        request: ScreenCinematicRequest,
        saved: SavedState<'_, M::StreamCheckpoint>,
        mounts: &mut dyn CampaignCinematicMounts,
        images: Rc<RefCell<I>>,
        mixer: M,
        renderer: R,
        seat: SeatId,
        current: Box<dyn Fn() -> bool>,
        mut captions: Option<C>,
    ) -> Result<Self, CampaignCinematicError> {
        let assert_current = || -> Result<(), CampaignCinematicError> {
            if !current() {
                return Err(CampaignCinematicError::Retired(
                    "Cinematic preparation belongs to a retired request".to_string(),
                ));
            }
            Ok(())
        };
        assert_current()?;
        let mut path = select_cinematic_path(&request.name)?;
        let mut resource = mounts.open(&path);
        assert_current()?;
        if resource.is_none() && path.to_lowercase().ends_with(".cin") {
            path = format!("{}.ogv", &path[..path.len() - 4]);
            resource = mounts.open(&path);
            assert_current()?;
        }
        let Some(resource) = resource else {
            return Err(CampaignCinematicError::Missing(format!(
                "Missing campaign cinematic: {path}"
            )));
        };
        let saved = match saved {
            SavedState::Fresh => None,
            SavedState::Restored(checkpoint) => {
                if checkpoint.resource != resource.reference {
                    return Err(CampaignCinematicError::ResourceChanged);
                }
                Some(checkpoint)
            }
        };
        let extension = path.rsplit('.').next().unwrap_or_default().to_lowercase();
        if extension != "cin" && extension != "roq" && extension != "ogv" && extension != "pcx" {
            return Err(CampaignCinematicError::Unsupported(format!(
                "Unsupported cinematic format: {extension}"
            )));
        }
        let source = if extension == "pcx" {
            cinematic_pcx(&resource.bytes, &path)?
        } else {
            cinematic_bytes(&extension, resource.bytes.clone(), &path).ok_or_else(|| {
                CampaignCinematicError::Unsupported(format!("Unsupported cinematic format: {extension}"))
            })?
        };
        let dimensions = cinematic_dimensions(&source)?;
        let lane = format!(
            "campaign-cinematic:{}",
            NEXT_CINEMATIC.fetch_add(1, Ordering::SeqCst) + 1
        );
        let clock_ms = saved.map_or(0.0, |saved| saved.clock);
        let audio_started = saved.is_some_and(|saved| saved.audio_started);
        let focus_paused = saved.is_some_and(|saved| saved.focus_paused);
        let pending_audio = saved.map_or_else(Vec::new, |saved| saved.initial_audio.clone());
        let pending_pcm = saved.and_then(|saved| saved.pcm.clone());
        let target = CinematicTarget::Seat(seat.clone());
        let playback = match saved {
            Some(saved) => CinematicPlayback::open_restored(
                &source,
                target,
                clock_ms,
                request.loop_playback,
                request.hold,
                request.silent,
                &saved.playback,
            )?,
            None => CinematicPlayback::open(
                &source,
                target,
                clock_ms,
                request.loop_playback,
                request.hold,
                request.silent,
            )?,
        };
        if let Some(captions) = captions.as_mut() {
            captions.prepare(&path);
        }
        assert_current()?;
        let reference = resource.reference.clone();
        let handle = images
            .borrow_mut()
            .allocate(dimensions.width, dimensions.height, &reference);
        let lane_images = Rc::clone(&images);
        let image = CinematicImage::new(
            handle,
            dimensions.width,
            dimensions.height,
            Box::new(move |width, height| lane_images.borrow_mut().allocate(width, height, &reference)),
        );
        Ok(Self {
            state: CinematicState::Prepared,
            playback,
            image,
            renderer,
            images,
            mixer,
            clock_ms,
            lane,
            path,
            request,
            resource: resource.reference.clone(),
            captions,
            current,
            audio_ready: false,
            audio_started,
            focus_paused,
            pending_audio,
            pending_pcm,
        })
    }

    /// Capture the owner checkpoint (donor `captureCheckpoint`).
    pub fn capture_checkpoint(
        &self,
    ) -> Result<CampaignCinematicCheckpoint<M::StreamCheckpoint>, CampaignCinematicError> {
        if self.state == CinematicState::Closed || !(self.current)() {
            return Err(CampaignCinematicError::Retired(
                "Cinematic checkpoint belongs to a retired request".to_string(),
            ));
        }
        let playback = self.playback.capture(self.clock_ms)?;
        let pcm = if self.audio_ready {
            Some(self.mixer.capture_stream_checkpoint(&self.lane).ok_or_else(|| {
                CampaignCinematicError::Mixer("Cinematic mixer cannot checkpoint its owned PCM lane".to_string())
            })?)
        } else {
            self.pending_pcm.clone()
        };
        Ok(CampaignCinematicCheckpoint {
            version: CINEMATIC_CHECKPOINT_VERSION,
            request: self.request.clone(),
            resource: self.resource.clone(),
            clock: self.clock_ms,
            audio_started: self.audio_ready || self.audio_started,
            focus_paused: self.focus_paused,
            playback,
            initial_audio: self.pending_audio.clone(),
            pcm,
        })
    }

    /// Activate the cinematic (donor `activate`).
    pub fn activate(&mut self) -> Result<(), CampaignCinematicError> {
        if self.state == CinematicState::Closed || !(self.current)() {
            return Err(CampaignCinematicError::Retired(
                "Cinematic activation belongs to a retired request".to_string(),
            ));
        }
        if self.state == CinematicState::Active {
            return Ok(());
        }
        self.state = CinematicState::Active;
        let status = self.playback.status();
        if status != CinematicStatus::Ended && status != CinematicStatus::Stopped {
            self.start_audio();
        }
        Ok(())
    }

    /// Start mixer audio (donor `startAudio`).
    fn start_audio(&mut self) {
        if !self.audio_started {
            self.mixer.stop_all();
        }
        if let Some(pcm) = self.pending_pcm.take() {
            self.mixer.restore_stream_checkpoint(
                &AudioStreamTarget {
                    id: self.lane.clone(),
                    audience: CinematicAudience::World,
                    gain: 1.0,
                },
                &pcm,
            );
        }
        self.audio_ready = true;
        let pending = std::mem::take(&mut self.pending_audio);
        for audio in &pending {
            cinematic_audio(&mut self.mixer, &self.lane, 1.0)
                .on_audio(audio, &CinematicTarget::Material(self.lane.clone()));
        }
        if self.playback.status() == CinematicStatus::Paused {
            cinematic_audio(&mut self.mixer, &self.lane, 1.0).on_audio_pause(true);
        }
    }

    /// Current timeline (donor `timeline`).
    pub fn timeline(&self) -> Result<CinematicTimeline, CampaignCinematicError> {
        Ok(self.playback.timeline(self.clock_ms)?)
    }

    /// Current status (donor `status`).
    pub fn status(&self) -> CinematicStatus {
        if self.state == CinematicState::Closed || !(self.current)() {
            return CinematicStatus::Stopped;
        }
        self.playback.status()
    }

    /// Pause or resume (donor `pause`).
    pub fn pause(&mut self, paused: bool) {
        if self.state == CinematicState::Closed || !(self.current)() {
            return;
        }
        let mut host = CampaignHost {
            mixer: &mut self.mixer,
            lane: &self.lane,
            ready: self.audio_ready,
            pending: &mut self.pending_audio,
        };
        let _ = self.playback.pause(paused, self.clock_ms, &mut host);
    }

    /// Skip to the end (donor `skip`).
    pub fn skip(&mut self) {
        if self.state == CinematicState::Closed || !(self.current)() {
            return;
        }
        let mut host = CampaignHost {
            mixer: &mut self.mixer,
            lane: &self.lane,
            ready: self.audio_ready,
            pending: &mut self.pending_audio,
        };
        let _ = self.playback.skip(&mut host);
    }

    /// Handle seat input (donor `input`).
    pub fn input(&mut self, event: &SeatInputEvent) -> bool {
        let press = matches!(
            event.kind,
            SeatInputEventKind::Key {
                down: true,
                repeat: false,
                ..
            } | SeatInputEventKind::MouseButton { down: true, .. }
                | SeatInputEventKind::ControllerButton { down: true, .. }
        );
        if press {
            let time = self.playback.playback_time(self.clock_ms).unwrap_or(0.0);
            // A still image has a held zero clock; source PCX pictures still
            // accept a deliberate skip.
            if time > 1000.0 || (self.status() == CinematicStatus::Held && self.clock_ms > 1000.0) {
                self.skip();
            }
        }
        true
    }

    /// Run one frame (donor `frame`).
    pub fn frame(
        &mut self,
        elapsed_ms: f64,
        sequence: u64,
        console_open: bool,
        menu_open: bool,
    ) -> Result<bool, CampaignCinematicError> {
        if self.state != CinematicState::Active || !(self.current)() {
            return Err(CampaignCinematicError::FrameNotActive);
        }
        if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
            return Err(CampaignCinematicError::FrameInterval);
        }
        self.clock_ms += elapsed_ms;
        let wall = self.clock_ms;
        let focus = if console_open {
            "console"
        } else if menu_open {
            "menu"
        } else {
            "game"
        };
        if focus != "game" && self.playback.status() == CinematicStatus::Playing {
            self.focus_paused = true;
            let mut host = CampaignHost {
                mixer: &mut self.mixer,
                lane: &self.lane,
                ready: self.audio_ready,
                pending: &mut self.pending_audio,
            };
            self.playback.pause(true, wall, &mut host)?;
        } else if focus == "game" && self.focus_paused {
            self.focus_paused = false;
            let mut host = CampaignHost {
                mixer: &mut self.mixer,
                lane: &self.lane,
                ready: self.audio_ready,
                pending: &mut self.pending_audio,
            };
            self.playback.pause(false, wall, &mut host)?;
        }
        let tick = {
            let mut host = CampaignHost {
                mixer: &mut self.mixer,
                lane: &self.lane,
                ready: self.audio_ready,
                pending: &mut self.pending_audio,
            };
            self.playback.tick(wall, &mut host)?
        };
        let visible = focus != "menu"
            && tick.status != CinematicStatus::Ended
            && tick.status != CinematicStatus::Stopped
            && tick.frame.is_some();
        let upload = if visible {
            match &tick.frame {
                Some(frame) => self.image.prepare(frame, self.playback.revision() as i32)?,
                None => None,
            }
        } else {
            None
        };
        if !console_open && !menu_open {
            if let Some(operations) = &upload {
                for operation in operations {
                    self.images.borrow_mut().commit(operation.clone());
                }
            }
            let (width, height) = self.renderer.drawable_size();
            let viewport = Rect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
            };
            let mut commands = vec![RenderCommand::DrawBuffer {
                buffer: DrawBuffer::Back,
                clear: true,
            }];
            if visible {
                let owner = self.images.borrow().owner();
                let (image_width, image_height) = self.image.dimensions();
                commands.push(RenderCommand::SetColor(vec4(1.0, 1.0, 1.0, 1.0)));
                commands.push(RenderCommand::StretchPic {
                    rect: viewport,
                    uv: TextureRect {
                        s1: 0.0,
                        t1: 0.0,
                        s2: 1.0,
                        t2: 1.0,
                    },
                    image: RendererImage {
                        owner: owner.clone(),
                        ordinal: self.image.image(),
                        source: ImageSource::Generated {
                            name: self.path.clone(),
                        },
                        width: image_width as u32,
                        height: image_height as u32,
                    },
                });
                if let Some(captions) = &self.captions {
                    commands.extend(captions.commands(&self.playback.timeline(wall)?, &viewport));
                }
            }
            commands.push(RenderCommand::SwapBuffers);
            self.renderer.execute(RenderFrame {
                owner: self.images.borrow().owner(),
                sequence,
                commands,
            });
            if upload.is_some() {
                self.image.complete()?;
            }
        }
        self.mixer.pump();
        Ok(self.playback.status() == CinematicStatus::Ended)
    }

    /// Close the cinematic (donor `close`).
    pub fn close(&mut self) {
        if self.state == CinematicState::Closed {
            return;
        }
        self.state = CinematicState::Closed;
        let mut host = CampaignHost {
            mixer: &mut self.mixer,
            lane: &self.lane,
            ready: self.audio_ready,
            pending: &mut self.pending_audio,
        };
        let _ = self.playback.close(&mut host);
        if let Some(operation) = self.image.release() {
            self.images.borrow_mut().commit(operation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{
        ContentDigest, ContentId, MountId, MountIdentity, ResourceId, ResourceProvenance, ResourceResolution,
    };
    use qa_core::identity::IdentityOwner;

    fn reference(path: &str) -> ResolvedResourceReference {
        use qa_content::contract::{LooseMount, MountPlanId};
        ResolvedResourceReference {
            id: ResourceId(format!("resource:test:{path}")),
            requested_path: path.to_string(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_string()),
                        content: ContentId("cinematic-test".to_string()),
                        generation: 0,
                    },
                    root_path: "corpus".to_string(),
                },
                member_path: path.to_string(),
            },
            digest: ContentDigest("sha256:00".to_string()),
            byte_length: 0,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:cinematic".to_string()),
                rank: 0,
            },
        }
    }

    fn pcx() -> Vec<u8> {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(&[10, 5, 1, 8]);
        bytes[8..10].copy_from_slice(&0u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&0u16.to_le_bytes());
        bytes[65] = 1;
        bytes[66..68].copy_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&[0xC1, 0x00]);
        bytes.push(12);
        bytes.extend((0..768).map(|index| (index % 256) as u8));
        bytes
    }

    struct StubMounts {
        files: std::collections::HashMap<String, Vec<u8>>,
    }

    impl CampaignCinematicMounts for StubMounts {
        fn open(&mut self, path: &str) -> Option<CinematicBytes> {
            self.files.get(path).map(|bytes| CinematicBytes {
                bytes: bytes.clone(),
                reference: reference(path),
            })
        }
    }

    struct StubImages {
        next: u32,
        committed: Vec<ImageOperation>,
        owner: ResourceOwner,
    }

    impl CampaignCinematicImages for StubImages {
        fn owner(&self) -> ResourceOwner {
            self.owner.clone()
        }
        fn allocate(&mut self, _width: usize, _height: usize, _resource: &ResolvedResourceReference) -> u32 {
            self.next += 1;
            self.next
        }
        fn commit(&mut self, operation: ImageOperation) {
            self.committed.push(operation);
        }
    }

    struct StubRenderer {
        frames: Vec<RenderFrame>,
    }

    impl CampaignCinematicRenderer for StubRenderer {
        fn drawable_size(&self) -> (u32, u32) {
            (640, 480)
        }
        fn execute(&mut self, frame: RenderFrame) {
            self.frames.push(frame);
        }
    }

    #[derive(Default)]
    struct StubMixer {
        stopped: u32,
        pumps: u32,
        restore: bool,
        pcm: Option<Vec<u8>>,
    }

    impl CinematicMixer for StubMixer {
        type StreamCheckpoint = Vec<u8>;
        fn queue_stream(&mut self, _target: &AudioStreamTarget, _pcm: &qa_client::media::audio::StreamPcm) {}
        fn stop_stream(&mut self, _id: &str) {}
        fn pause_stream(&mut self, _id: &str, _paused: bool) {}
        fn capture_stream_checkpoint(&self, _id: &str) -> Option<Vec<u8>> {
            Some(vec![7, 7])
        }
        fn restore_stream_checkpoint(&mut self, _target: &AudioStreamTarget, checkpoint: &Vec<u8>) {
            self.restore = true;
            self.pcm = Some(checkpoint.clone());
        }
    }

    impl CampaignCinematicMixer for StubMixer {
        fn stop_all(&mut self) {
            self.stopped += 1;
        }
        fn pump(&mut self) {
            self.pumps += 1;
        }
        fn supports_stream_restore(&self) -> bool {
            true
        }
    }

    struct StubCaptions {
        prepared: Vec<String>,
    }

    impl ScreenCinematicCaptions for StubCaptions {
        fn prepare(&mut self, path: &str) {
            self.prepared.push(path.to_string());
        }
        fn commands(&self, _timeline: &CinematicTimeline, _viewport: &Rect) -> Vec<RenderCommand> {
            vec![RenderCommand::SwapBuffers]
        }
    }

    fn seat() -> SeatId {
        IdentityOwner::create("cinematic-test").unwrap().seat(0)
    }

    fn owner() -> ResourceOwner {
        ResourceOwner {
            identity: 1,
            session: IdentityOwner::create("cinematic-owner").unwrap().session().clone(),
            generation: 0,
        }
    }

    fn request(name: &str) -> ScreenCinematicRequest {
        ScreenCinematicRequest {
            name: name.to_string(),
            loop_playback: false,
            hold: true,
            silent: true,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn open(
        name: &str,
        files: std::collections::HashMap<String, Vec<u8>>,
    ) -> CampaignCinematic<StubMixer, StubImages, StubRenderer, StubCaptions> {
        let mut mounts = StubMounts { files };
        CampaignCinematic::prepare(
            request(name),
            &mut mounts,
            Rc::new(RefCell::new(StubImages {
                next: 41,
                committed: Vec::new(),
                owner: owner(),
            })),
            StubMixer::default(),
            StubRenderer { frames: Vec::new() },
            seat(),
            Box::new(|| true),
            Some(StubCaptions { prepared: Vec::new() }),
        )
        .unwrap()
    }

    #[test]
    fn select_cinematic_path_validates_and_prefixes() {
        assert_eq!(select_cinematic_path("intro").unwrap(), "video/intro.roq");
        assert_eq!(select_cinematic_path("intro.cin").unwrap(), "video/intro.cin");
        assert_eq!(select_cinematic_path("still.pcx").unwrap(), "pics/still.pcx");
        assert_eq!(select_cinematic_path("video/custom.ogv").unwrap(), "video/custom.ogv");
        assert!(select_cinematic_path("/abs.roq").is_err());
        assert!(select_cinematic_path("a/../b.roq").is_err());
        assert!(select_cinematic_path("a\\.roq").is_err());
    }

    #[test]
    fn prepare_rejects_missing_and_unsupported() {
        let mut mounts = StubMounts {
            files: std::collections::HashMap::new(),
        };
        let result = CampaignCinematic::prepare(
            request("missing"),
            &mut mounts,
            Rc::new(RefCell::new(StubImages {
                next: 0,
                committed: Vec::new(),
                owner: owner(),
            })),
            StubMixer::default(),
            StubRenderer { frames: Vec::new() },
            seat(),
            Box::new(|| true),
            None::<StubCaptions>,
        );
        let Err(error) = result else {
            panic!("missing movie must fail")
        };
        assert!(matches!(error, CampaignCinematicError::Missing(_)));
        let mut files = std::collections::HashMap::new();
        files.insert("video/clip.mp4".to_string(), vec![1, 2, 3]);
        let mut mounts = StubMounts { files };
        let result = CampaignCinematic::prepare(
            request("clip.mp4"),
            &mut mounts,
            Rc::new(RefCell::new(StubImages {
                next: 0,
                committed: Vec::new(),
                owner: owner(),
            })),
            StubMixer::default(),
            StubRenderer { frames: Vec::new() },
            seat(),
            Box::new(|| true),
            None::<StubCaptions>,
        );
        let Err(error) = result else {
            panic!("unsupported movie must fail")
        };
        assert!(matches!(error, CampaignCinematicError::Unsupported(_)));
    }

    #[test]
    fn still_frame_draws_and_pumps() {
        let mut files = std::collections::HashMap::new();
        files.insert("pics/still.pcx".to_string(), pcx());
        let mut movie = open("still.pcx", files);
        assert_eq!(movie.status(), CinematicStatus::Held);
        assert!(movie.frame(16.0, 1, false, false).is_err());
        movie.activate().unwrap();
        let ended = movie.frame(16.0, 1, false, false).unwrap();
        assert!(!ended);
        assert_eq!(movie.mixer.pumps, 1);
        assert_eq!(movie.renderer.frames.len(), 1);
        let frame = &movie.renderer.frames[0];
        assert!(matches!(
            frame.commands[0],
            RenderCommand::DrawBuffer { clear: true, .. }
        ));
        assert!(matches!(frame.commands.last(), Some(RenderCommand::SwapBuffers)));
        assert!(frame
            .commands
            .iter()
            .any(|command| matches!(command, RenderCommand::StretchPic { .. })));
        assert!(!movie.images.borrow().committed.is_empty());
    }

    #[test]
    fn checkpoint_round_trip_restores_audio_lane() {
        let mut files = std::collections::HashMap::new();
        files.insert("pics/still.pcx".to_string(), pcx());
        let mut movie = open("still.pcx", files.clone());
        movie.activate().unwrap();
        movie.frame(16.0, 1, false, false).unwrap();
        let checkpoint = movie.capture_checkpoint().unwrap();
        assert_eq!(checkpoint.version, 1);
        assert!(checkpoint.audio_started);
        let mut mounts = StubMounts { files };
        let mut restored = CampaignCinematic::restore(
            &checkpoint,
            &mut mounts,
            Rc::new(RefCell::new(StubImages {
                next: 90,
                committed: Vec::new(),
                owner: owner(),
            })),
            StubMixer::default(),
            StubRenderer { frames: Vec::new() },
            seat(),
            Box::new(|| true),
            None::<StubCaptions>,
        )
        .unwrap();
        restored.activate().unwrap();
        assert!(restored.mixer.restore);
        assert_eq!(restored.status(), CinematicStatus::Held);
        restored.close();
        assert_eq!(restored.status(), CinematicStatus::Stopped);
    }

    #[test]
    fn retired_request_rejects_lifecycle_calls() {
        let mut files = std::collections::HashMap::new();
        files.insert("pics/still.pcx".to_string(), pcx());
        let mut mounts = StubMounts { files };
        let result = CampaignCinematic::prepare(
            request("still.pcx"),
            &mut mounts,
            Rc::new(RefCell::new(StubImages {
                next: 0,
                committed: Vec::new(),
                owner: owner(),
            })),
            StubMixer::default(),
            StubRenderer { frames: Vec::new() },
            seat(),
            Box::new(|| false),
            None::<StubCaptions>,
        );
        let Err(error) = result else {
            panic!("retired request must fail")
        };
        assert!(matches!(error, CampaignCinematicError::Retired(_)));
    }
}
