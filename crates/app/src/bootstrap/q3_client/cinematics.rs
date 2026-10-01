//! Quake III UI cinematics over retained movie sources.
//!
//! Port of `src/app/bootstrap/q3-client/cinematics.ts`
//! (`ApplicationQ3Cinematics`). The owner prepares retained movie
//! sources, plays them into sixteen slots shared with system
//! cinematics, and checkpoints both. Source I/O, image allocation, and
//! system transitions are injected seams; playback, upload images, and
//! mixer lanes reuse `qa-client` media. Loading is synchronous, so the
//! donor's pending-load guard has no counterpart. Movies draw through
//! [`PictureAsset::Image`] with the uploaded frame handle instead of
//! the donor's compiled video material.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

use qa_client::media::audio::{cinematic_audio, AudioStreamTarget, CinematicAudience, CinematicMixer};
use qa_client::media::playback::{cinematic_bytes, CinematicPlayback, CinematicSource, PlaybackCheckpoint};
use qa_client::media::presentation::{cinematic_dimensions, CinematicImage, ImageOperation};
use qa_client::media::still::cinematic_pcx;
use qa_client::media::types::{CinematicAudio, CinematicHost, CinematicTarget, DecoderStatus};
use qa_client::text::draw2d::{Draw2D, ImagePicture, PictureAsset, Rect};
use qa_client::ClientError;
use qa_core::identity::SeatId;
use thiserror::Error;

/// Maximum movie/system slots.
const MAX_SLOTS: i32 = 16;
/// Checkpoint version.
const CHECKPOINT_VERSION: u32 = 1;

static NEXT_CONSUMER: AtomicU32 = AtomicU32::new(0);

/// UI cinematic asset handle (donor `UiCinematicAsset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiCinematicAsset {
    /// Prepared source path.
    pub path: String,
}

/// Open movie handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CinematicHandle {
    /// Slot index.
    pub index: i32,
}

/// Fullscreen system-cinematic request (donor `ScreenCinematicRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCinematicRequest {
    /// Movie name.
    pub name: String,
    /// Loop playback.
    pub looping: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Suppress audio.
    pub silent: bool,
}

/// System-cinematic playback status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCinematicStatus {
    /// Holding a frame.
    Held,
    /// Playing.
    Playing,
    /// Paused.
    Paused,
    /// Ended or stopped.
    Finished,
}

/// Open system cinematic (donor `SystemCinematicHandle`).
pub trait SystemCinematicHandle {
    /// Current status.
    fn status(&mut self) -> SystemCinematicStatus;
    /// Skip the cinematic.
    fn skip(&mut self);
    /// Stop the cinematic.
    fn stop(&mut self);
    /// Capture a checkpoint, when the transition owns one.
    fn capture_checkpoint(&self) -> Option<Vec<u8>>;
    /// Publish a restored cinematic.
    fn publish_restored(&mut self) {}
}

/// System-cinematic owner (donor `SystemCinematicHost`).
pub trait SystemCinematicHost {
    /// Open a fullscreen cinematic guarded by `current`.
    fn open(&mut self, request: ScreenCinematicRequest, current: &dyn Fn() -> bool) -> Box<dyn SystemCinematicHandle>;
    /// Restore a cinematic checkpoint, or `None` without a transition owner.
    fn restore(&mut self, checkpoint: &[u8], current: &dyn Fn() -> bool) -> Option<Box<dyn SystemCinematicHandle>>;
}

/// Retained movie bytes with their resource identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicBytes {
    /// Movie bytes.
    pub bytes: Vec<u8>,
    /// Resource identity.
    pub resource: String,
}

/// Movie source mounts.
pub trait CinematicMounts {
    /// Open a movie by resolved name.
    fn open(&mut self, name: &str) -> Option<CinematicBytes>;
}

/// Movie image allocation.
pub trait CinematicImages {
    /// Allocate an image.
    fn allocate(&mut self, width: usize, height: usize, resource: &str) -> u32;
    /// Commit an upload operation.
    fn commit(&mut self, operation: ImageOperation);
    /// Release an image.
    fn release(&mut self, image: u32);
}

/// Cinematic failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3CinematicError {
    /// UI cinematics are closed.
    #[error("UI cinematics are closed")]
    Closed,
    /// Cinematic restore has not completed.
    #[error("Cinematic restore has not completed")]
    Restoring,
    /// Cinematic checkpoint requires an idle live owner.
    #[error("Cinematic checkpoint requires an idle live owner")]
    CheckpointNotIdle,
    /// Cinematic mixer cannot checkpoint its owned PCM lanes.
    #[error("Cinematic mixer cannot checkpoint its owned PCM lanes")]
    NoPcmCheckpoint,
    /// System cinematic checkpoint requires its attached client transition checkpoint.
    #[error("System cinematic checkpoint requires its attached client transition checkpoint")]
    NoSystemCheckpoint,
    /// System cinematic restore requires its attached client transition checkpoint.
    #[error("System cinematic restore requires its attached client transition checkpoint")]
    NoSystemRestore,
    /// System cinematic requires an attached client transition owner.
    #[error("System cinematic requires an attached client transition owner")]
    NoSystemHost,
    /// Cinematic restore requires an unpublished empty owner.
    #[error("Cinematic restore requires an unpublished empty owner")]
    RestoreRequiresEmpty,
    /// Cinematic owner cannot publish before restore completes.
    #[error("Cinematic owner cannot publish before restore completes")]
    PublishTooEarly,
    /// Cinematic resource changed under a checkpoint.
    #[error("cinematic resource changed: {0}")]
    ResourceChanged(String),
    /// Invalid cinematic checkpoint.
    #[error("Invalid cinematic checkpoint: {0}")]
    BadCheckpoint(String),
    /// Missing cinematic.
    #[error("Missing cinematic {0}")]
    Missing(String),
    /// Unsupported cinematic format.
    #[error("Unsupported cinematic format: {0}")]
    UnsupportedFormat(String),
    /// Unprepared cinematic.
    #[error("Unprepared cinematic {0}")]
    Unprepared(String),
    /// No free cinematic handle.
    #[error("CIN_HandleForVideo: none free")]
    NoneFree,
    /// Media failure.
    #[error("{0}")]
    Media(#[from] ClientError),
}

/// Movie play mode.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Q3CinematicMode {
    loop_playback: bool,
    hold: bool,
    silent: bool,
    shader: bool,
}

/// One open movie.
struct Movie {
    playback: CinematicPlayback,
    image: CinematicImage,
    path: String,
    asset: UiCinematicAsset,
    mode: Q3CinematicMode,
    rect: Rect,
}

/// Retained movie source.
struct PreparedSource {
    source: CinematicSource,
    resource: String,
}

/// Checkpointed movie mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3CinematicModeCheckpoint {
    /// Loop playback.
    pub loop_playback: bool,
    /// Hold the last frame.
    pub hold: bool,
    /// Suppress audio.
    pub silent: bool,
    /// Shader target.
    pub shader: bool,
    /// Draw rectangle.
    pub rect: Rect,
}

/// Checkpointed source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CinematicSourceCheckpoint {
    /// Source path.
    pub path: String,
    /// Resource identity.
    pub resource: String,
}

/// Checkpointed movie.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MovieCheckpoint<P> {
    /// Slot index.
    pub index: i32,
    /// Asset path.
    pub asset: String,
    /// Play mode.
    pub mode: Q3CinematicModeCheckpoint,
    /// Playback checkpoint.
    pub playback: PlaybackCheckpoint,
    /// Mixer PCM checkpoint.
    pub pcm: P,
}

/// Checkpointed system cinematic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SystemCheckpoint {
    /// Slot index.
    pub index: i32,
    /// System checkpoint bytes.
    pub checkpoint: Vec<u8>,
}

/// Cinematic owner checkpoint (donor `captureCheckpoint`, version 1).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CinematicCheckpoint<P> {
    /// Checkpoint version (1).
    pub version: u32,
    /// Retained sources.
    pub sources: Vec<Q3CinematicSourceCheckpoint>,
    /// Open movies.
    pub movies: Vec<Q3MovieCheckpoint<P>>,
    /// Open system cinematics.
    pub systems: Vec<Q3SystemCheckpoint>,
}

/// Per-tick playback host bridging one mixer lane and print.
struct TickHost<'a, M: CinematicMixer> {
    mixer: &'a mut M,
    lane: String,
    seat: SeatId,
    print: &'a mut dyn FnMut(&str),
}

impl<M: CinematicMixer> CinematicHost for TickHost<'_, M> {
    fn on_audio(&mut self, audio: &CinematicAudio, _target: &CinematicTarget) {
        let seat = CinematicTarget::Seat(self.seat.clone());
        cinematic_audio(&mut *self.mixer, &self.lane, 1.0).on_audio(audio, &seat);
    }

    fn on_audio_reset(&mut self, _target: &CinematicTarget) {
        cinematic_audio(&mut *self.mixer, &self.lane, 1.0).on_audio_reset();
    }

    fn on_audio_pause(&mut self, paused: bool, _target: &CinematicTarget) {
        cinematic_audio(&mut *self.mixer, &self.lane, 1.0).on_audio_pause(paused);
    }

    fn on_complete(&mut self, _reason: qa_client::media::types::CinematicEndReason, _target: &CinematicTarget) {}

    fn developer_print(&mut self, message: &str) {
        (self.print)(message);
    }
}

/// Quake III UI cinematics.
pub struct ApplicationQ3Cinematics<M: CinematicMixer, S> {
    mounts: Box<dyn CinematicMounts>,
    images: Rc<RefCell<S>>,
    mixer: M,
    seat: SeatId,
    now: Box<dyn FnMut() -> f64>,
    print: Box<dyn FnMut(&str)>,
    system: Option<Box<dyn SystemCinematicHost>>,
    sources: HashMap<String, PreparedSource>,
    movies: HashMap<i32, Movie>,
    systems: HashMap<i32, Box<dyn SystemCinematicHandle>>,
    namespace: String,
    closed: bool,
    restoring: bool,
}

impl<M: CinematicMixer, S: CinematicImages + 'static> ApplicationQ3Cinematics<M, S> {
    /// Build a cinematic owner.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        mounts: Box<dyn CinematicMounts>,
        images: S,
        mixer: M,
        seat: SeatId,
        now: Box<dyn FnMut() -> f64>,
        print: Box<dyn FnMut(&str)>,
        system: Option<Box<dyn SystemCinematicHost>>,
    ) -> Self {
        let namespace = format!("q3-cinematic:{}", NEXT_CONSUMER.fetch_add(1, Ordering::Relaxed) + 1);
        Self {
            mounts,
            images: Rc::new(RefCell::new(images)),
            mixer,
            seat,
            now,
            print,
            system,
            sources: HashMap::new(),
            movies: HashMap::new(),
            systems: HashMap::new(),
            namespace,
            closed: false,
            restoring: false,
        }
    }

    fn lane(&self, index: i32) -> String {
        format!("{}:{index}", self.namespace)
    }

    fn host(&mut self, index: i32) -> TickHost<'_, M> {
        let lane = self.lane(index);
        let seat = self.seat.clone();
        TickHost {
            mixer: &mut self.mixer,
            lane,
            seat,
            print: &mut *self.print,
        }
    }

    fn free_slot(&self) -> Option<i32> {
        let mut index = 0;
        while self.movies.contains_key(&index) || self.systems.contains_key(&index) {
            index += 1;
        }
        (index < MAX_SLOTS).then_some(index)
    }

    fn resolve_name(path: &str) -> String {
        let selected = if path.contains('/') {
            path.to_string()
        } else {
            format!("video/{path}")
        };
        let base = selected.rsplit('/').next().unwrap_or_default();
        let has_extension = base.contains('.') && base.rsplit('.').next().is_some_and(|part| !part.is_empty());
        if has_extension {
            selected
        } else {
            format!("{selected}.roq")
        }
    }

    /// Prepare a retained movie source.
    pub fn prepare_asset(&mut self, path: &str) -> Result<UiCinematicAsset, Q3CinematicError> {
        if self.restoring {
            return Err(Q3CinematicError::Restoring);
        }
        self.prepare_inner(path)
    }

    fn prepare_inner(&mut self, path: &str) -> Result<UiCinematicAsset, Q3CinematicError> {
        if self.closed {
            return Err(Q3CinematicError::Closed);
        }
        if !self.sources.contains_key(path) {
            let name = Self::resolve_name(path);
            let opened = self.mounts.open(&name);
            if self.closed {
                return Err(Q3CinematicError::Closed);
            }
            let Some(opened) = opened else {
                return Err(Q3CinematicError::Missing(name));
            };
            if opened.bytes.is_empty() {
                return Err(Q3CinematicError::Missing(name));
            }
            let extension = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
            if !matches!(extension.as_str(), "roq" | "cin" | "ogv" | "pcx") {
                return Err(Q3CinematicError::UnsupportedFormat(extension));
            }
            let source = if extension == "pcx" {
                cinematic_pcx(&opened.bytes, &name)?
            } else {
                cinematic_bytes(&extension, opened.bytes.clone(), &name)
                    .ok_or_else(|| Q3CinematicError::UnsupportedFormat(extension.clone()))?
            };
            self.sources.insert(
                path.to_string(),
                PreparedSource {
                    source,
                    resource: opened.resource,
                },
            );
        }
        Ok(UiCinematicAsset { path: path.to_string() })
    }

    /// Play a looping silent movie.
    pub fn play(&mut self, asset: &UiCinematicAsset, rect: Rect) -> Result<Option<CinematicHandle>, Q3CinematicError> {
        self.play_mode(
            asset,
            Q3CinematicMode {
                loop_playback: true,
                hold: false,
                silent: true,
                shader: false,
            },
            rect,
        )
    }

    fn play_mode(
        &mut self,
        asset: &UiCinematicAsset,
        mode: Q3CinematicMode,
        rect: Rect,
    ) -> Result<Option<CinematicHandle>, Q3CinematicError> {
        if self.restoring {
            return Err(Q3CinematicError::Restoring);
        }
        if self.closed {
            return Err(Q3CinematicError::Closed);
        }
        if !self.sources.contains_key(&asset.path) {
            return Err(Q3CinematicError::Unprepared(asset.path.clone()));
        }
        let Some(index) = self.free_slot() else {
            return Ok(None);
        };
        self.create_movie(index, asset, mode, rect, None).map(Some)
    }

    fn create_movie(
        &mut self,
        index: i32,
        asset: &UiCinematicAsset,
        mode: Q3CinematicMode,
        rect: Rect,
        checkpoint: Option<&PlaybackCheckpoint>,
    ) -> Result<CinematicHandle, Q3CinematicError> {
        let prepared = self
            .sources
            .get(&asset.path)
            .ok_or_else(|| Q3CinematicError::Unprepared(asset.path.clone()))?;
        let dimensions = cinematic_dimensions(&prepared.source)?;
        let lane = self.lane(index);
        let target = if mode.shader {
            CinematicTarget::Material(lane)
        } else {
            CinematicTarget::Seat(self.seat.clone())
        };
        let wall = (self.now)();
        let playback = match checkpoint {
            Some(checkpoint) => CinematicPlayback::open_restored(
                &prepared.source,
                target,
                wall,
                mode.loop_playback,
                mode.hold,
                mode.silent,
                checkpoint,
            )?,
            None => CinematicPlayback::open(
                &prepared.source,
                target,
                wall,
                mode.loop_playback,
                mode.hold,
                mode.silent,
            )?,
        };
        let resource = prepared.resource.clone();
        let path = prepared.source.source().to_string();
        let images = Rc::clone(&self.images);
        let handle = images
            .borrow_mut()
            .allocate(dimensions.width, dimensions.height, &resource);
        let image = CinematicImage::new(
            handle,
            dimensions.width,
            dimensions.height,
            Box::new(move |width, height| images.borrow_mut().allocate(width, height, &resource)),
        );
        self.movies.insert(
            index,
            Movie {
                playback,
                image,
                path,
                asset: asset.clone(),
                mode,
                rect,
            },
        );
        Ok(CinematicHandle { index })
    }

    /// Capture the owner checkpoint.
    pub fn capture_checkpoint(&mut self) -> Result<Q3CinematicCheckpoint<M::StreamCheckpoint>, Q3CinematicError> {
        if self.closed || self.restoring {
            return Err(Q3CinematicError::CheckpointNotIdle);
        }
        let mut systems = Vec::new();
        for (index, system) in self.systems.iter() {
            let Some(checkpoint) = system.capture_checkpoint() else {
                return Err(Q3CinematicError::NoSystemCheckpoint);
            };
            systems.push(Q3SystemCheckpoint {
                index: *index,
                checkpoint,
            });
        }
        systems.sort_by_key(|system| system.index);
        let mut sources: Vec<Q3CinematicSourceCheckpoint> = self
            .sources
            .iter()
            .map(|(path, source)| Q3CinematicSourceCheckpoint {
                path: path.clone(),
                resource: source.resource.clone(),
            })
            .collect();
        sources.sort_by(|left, right| left.path.cmp(&right.path));
        let mut movies = Vec::new();
        let mut indices: Vec<i32> = self.movies.keys().copied().collect();
        indices.sort();
        for index in indices {
            let wall = (self.now)();
            let movie = self.movies.get(&index).expect("movie slot");
            let playback = movie.playback.capture(wall)?;
            let mode = movie.mode;
            let rect = movie.rect;
            let asset = movie.asset.path.clone();
            let lane = self.lane(index);
            let Some(pcm) = self.mixer.capture_stream_checkpoint(&lane) else {
                return Err(Q3CinematicError::NoPcmCheckpoint);
            };
            movies.push(Q3MovieCheckpoint {
                index,
                asset,
                mode: Q3CinematicModeCheckpoint {
                    loop_playback: mode.loop_playback,
                    hold: mode.hold,
                    silent: mode.silent,
                    shader: mode.shader,
                    rect,
                },
                playback,
                pcm,
            });
        }
        Ok(Q3CinematicCheckpoint {
            version: CHECKPOINT_VERSION,
            sources,
            movies,
            systems,
        })
    }

    /// Restore an unpublished empty owner.
    pub fn restore_checkpoint(
        &mut self,
        checkpoint: &Q3CinematicCheckpoint<M::StreamCheckpoint>,
    ) -> Result<(), Q3CinematicError> {
        if self.closed
            || self.restoring
            || !self.movies.is_empty()
            || !self.sources.is_empty()
            || !self.systems.is_empty()
        {
            return Err(Q3CinematicError::RestoreRequiresEmpty);
        }
        if checkpoint.version != CHECKPOINT_VERSION {
            return Err(Q3CinematicError::BadCheckpoint(format!(
                "unsupported version {}",
                checkpoint.version
            )));
        }
        let mut paths = std::collections::HashSet::new();
        for source in &checkpoint.sources {
            if !paths.insert(source.path.clone()) {
                return Err(Q3CinematicError::BadCheckpoint(
                    "duplicate cinematic source".to_string(),
                ));
            }
        }
        let mut slots = std::collections::HashSet::new();
        for movie in &checkpoint.movies {
            if movie.index < 0
                || movie.index >= MAX_SLOTS
                || !slots.insert(movie.index)
                || !paths.contains(&movie.asset)
            {
                return Err(Q3CinematicError::BadCheckpoint(
                    "invalid cinematic slot or source".to_string(),
                ));
            }
            for value in [
                movie.mode.rect.x,
                movie.mode.rect.y,
                movie.mode.rect.width,
                movie.mode.rect.height,
            ] {
                if !value.is_finite() {
                    return Err(Q3CinematicError::BadCheckpoint(
                        "cinematic rect is not finite".to_string(),
                    ));
                }
            }
        }
        for system in &checkpoint.systems {
            if system.index < 0 || system.index >= MAX_SLOTS || !slots.insert(system.index) {
                return Err(Q3CinematicError::BadCheckpoint(
                    "duplicate or invalid system cinematic slot".to_string(),
                ));
            }
        }
        if !checkpoint.systems.is_empty() && self.system.is_none() {
            return Err(Q3CinematicError::NoSystemRestore);
        }
        self.restoring = true;
        let result = self.restore_inner(checkpoint);
        self.restoring = false;
        result
    }

    fn restore_inner(
        &mut self,
        checkpoint: &Q3CinematicCheckpoint<M::StreamCheckpoint>,
    ) -> Result<(), Q3CinematicError> {
        for source in &checkpoint.sources {
            self.restoring_prepare(&source.path)?;
            if self
                .sources
                .get(&source.path)
                .is_some_and(|prepared| prepared.resource != source.resource)
            {
                self.abort_restore();
                return Err(Q3CinematicError::ResourceChanged(source.path.clone()));
            }
        }
        if self.closed {
            self.abort_restore();
            return Err(Q3CinematicError::Closed);
        }
        for system in &checkpoint.systems {
            if self.system.is_none() {
                self.abort_restore();
                return Err(Q3CinematicError::NoSystemRestore);
            }
            let current = !self.closed;
            let restored = self
                .system
                .as_mut()
                .expect("system host")
                .restore(&system.checkpoint, &|| current);
            let Some(restored) = restored else {
                self.abort_restore();
                return Err(Q3CinematicError::NoSystemRestore);
            };
            self.systems.insert(system.index, restored);
        }
        for movie in &checkpoint.movies {
            let asset = UiCinematicAsset {
                path: movie.asset.clone(),
            };
            let mode = Q3CinematicMode {
                loop_playback: movie.mode.loop_playback,
                hold: movie.mode.hold,
                silent: movie.mode.silent,
                shader: movie.mode.shader,
            };
            if let Err(error) = self.create_movie(movie.index, &asset, mode, movie.mode.rect, Some(&movie.playback)) {
                self.abort_restore();
                return Err(error);
            }
            let target = AudioStreamTarget {
                id: self.lane(movie.index),
                audience: CinematicAudience::Seat(self.seat.clone()),
                gain: 1.0,
            };
            self.mixer.restore_stream_checkpoint(&target, &movie.pcm);
        }
        Ok(())
    }

    fn restoring_prepare(&mut self, path: &str) -> Result<(), Q3CinematicError> {
        self.prepare_inner(path).map(|_| ())
    }

    fn abort_restore(&mut self) {
        for index in self.movies.keys().copied().collect::<Vec<_>>() {
            let _ = self.stop_inner(index);
        }
        for index in self.systems.keys().copied().collect::<Vec<_>>() {
            let _ = self.stop_inner(index);
        }
        self.sources.clear();
    }

    /// Publish restored system cinematics.
    pub fn publish_restored(&mut self) -> Result<(), Q3CinematicError> {
        if self.closed || self.restoring {
            return Err(Q3CinematicError::PublishTooEarly);
        }
        for system in self.systems.values_mut() {
            system.publish_restored();
        }
        Ok(())
    }

    /// Open a guest movie, returning its handle or -1 when missing.
    pub fn play_guest(&mut self, path: &str, rect: Rect, bits: u32) -> Result<i32, Q3CinematicError> {
        if self.restoring {
            return Err(Q3CinematicError::Restoring);
        }
        if bits & 1 != 0 {
            if self.closed {
                return Err(Q3CinematicError::Closed);
            }
            if self.system.is_none() {
                return Err(Q3CinematicError::NoSystemHost);
            }
            let Some(index) = self.free_slot() else {
                return Err(Q3CinematicError::NoneFree);
            };
            let request = ScreenCinematicRequest {
                name: path.to_string(),
                looping: bits & 2 != 0,
                hold: bits & 4 != 0,
                silent: bits & 8 != 0,
            };
            let current = !self.closed;
            // Checked above.
            let system = self.system.as_mut().expect("system host").open(request, &|| current);
            if self.closed {
                return Err(Q3CinematicError::Closed);
            }
            self.systems.insert(index, system);
            return Ok(index);
        }
        let asset = match self.prepare_asset(path) {
            Ok(asset) => asset,
            Err(Q3CinematicError::Missing(_)) => return Ok(-1),
            Err(error) => return Err(error),
        };
        if self.closed {
            return Err(Q3CinematicError::Closed);
        }
        let source = self
            .sources
            .get(&asset.path)
            .expect("prepared source")
            .source
            .source()
            .to_string();
        for (index, movie) in self.movies.iter() {
            if movie.path == source {
                return Ok(*index);
            }
        }
        let mode = Q3CinematicMode {
            loop_playback: bits & 2 != 0,
            hold: bits & 4 != 0,
            silent: bits & 8 != 0,
            shader: bits & 16 != 0,
        };
        match self.play_mode(&asset, mode, rect)? {
            Some(handle) => Ok(handle.index),
            None => Err(Q3CinematicError::NoneFree),
        }
    }

    /// Tick a guest movie, returning its status code.
    pub fn run_guest(&mut self, handle: i32) -> Result<i32, Q3CinematicError> {
        if self.systems.contains_key(&handle) {
            let status = self.systems.get_mut(&handle).expect("system slot").status();
            return Ok(match status {
                SystemCinematicStatus::Held => 0,
                SystemCinematicStatus::Playing | SystemCinematicStatus::Paused => 1,
                SystemCinematicStatus::Finished => {
                    self.stop_inner(handle)?;
                    2
                }
            });
        }
        if !self.movies.contains_key(&handle) {
            return Ok(2);
        }
        let wall = (self.now)();
        let lane = self.lane(handle);
        let Self {
            movies,
            mixer,
            seat,
            print,
            ..
        } = self;
        let movie = movies.get_mut(&handle).expect("movie slot");
        let mut host = TickHost {
            mixer,
            lane,
            seat: seat.clone(),
            print: &mut **print,
        };
        let _ = movie.playback.tick(wall, &mut host)?;
        let status = movie.playback.source_status();
        drop(host);
        match status {
            DecoderStatus::Playing | DecoderStatus::Paused => Ok(1),
            DecoderStatus::Looped => Ok(5),
            DecoderStatus::Held => Ok(0),
            DecoderStatus::Ended | DecoderStatus::Stopped => {
                self.stop_inner(handle)?;
                Ok(2)
            }
        }
    }

    /// Stop a guest movie.
    pub fn stop_guest(&mut self, handle: i32) -> Result<i32, Q3CinematicError> {
        if let Some(mut system) = self.systems.remove(&handle) {
            system.skip();
            return Ok(2);
        }
        self.stop_inner(handle)?;
        Ok(2)
    }

    /// Draw a guest movie.
    pub fn draw_guest(&mut self, handle: i32, draw: &mut Draw2D<'_>) -> Result<(), Q3CinematicError> {
        let Some(rect) = self.movies.get(&handle).map(|movie| movie.rect) else {
            return Ok(());
        };
        self.draw(handle, rect, draw)
    }

    /// Update a movie's draw rectangle.
    pub fn set_extents(&mut self, handle: i32, rect: Rect) {
        if let Some(movie) = self.movies.get_mut(&handle) {
            movie.rect = rect;
        }
    }

    /// Tick a UI movie.
    pub fn run(&mut self, handle: i32) -> Result<(), Q3CinematicError> {
        if !self.movies.contains_key(&handle) {
            return Ok(());
        }
        let wall = (self.now)();
        let lane = self.lane(handle);
        let Self {
            movies,
            mixer,
            seat,
            print,
            ..
        } = self;
        let movie = movies.get_mut(&handle).expect("movie slot");
        let mut host = TickHost {
            mixer,
            lane,
            seat: seat.clone(),
            print: &mut **print,
        };
        let _ = movie.playback.tick(wall, &mut host)?;
        Ok(())
    }

    /// Draw a UI movie.
    pub fn draw(&mut self, handle: i32, rect: Rect, draw: &mut Draw2D<'_>) -> Result<(), Q3CinematicError> {
        let Some(movie) = self.movies.get_mut(&handle) else {
            return Ok(());
        };
        let Some(frame) = movie.playback.current_frame() else {
            return Ok(());
        };
        let revision = movie.playback.revision() as i32;
        let images = Rc::clone(&self.images);
        let image = movie.image.resolve(&frame, revision, &mut |operation| {
            images.borrow_mut().commit(operation);
        })?;
        let (width, height) = movie.image.dimensions();
        draw.draw_pic(
            rect,
            PictureAsset::Image(ImagePicture {
                image,
                width: width as u32,
                height: height as u32,
            }),
        );
        Ok(())
    }

    /// Stop a movie slot.
    pub fn stop_slot(&mut self, index: i32) -> Result<(), Q3CinematicError> {
        self.stop_inner(index)
    }

    /// Stop a movie or system slot.
    pub fn stop(&mut self, handle: i32) -> Result<(), Q3CinematicError> {
        self.stop_inner(handle)
    }

    fn stop_inner(&mut self, handle: i32) -> Result<(), Q3CinematicError> {
        if let Some(mut system) = self.systems.remove(&handle) {
            system.stop();
            return Ok(());
        }
        let Some(mut movie) = self.movies.remove(&handle) else {
            return Ok(());
        };
        let release = movie.image.release();
        let mut host = self.host(handle);
        let closed = movie.playback.close(&mut host);
        drop(host);
        if let Some(ImageOperation::Release { image }) = release {
            self.images.borrow_mut().release(image);
        }
        closed?;
        Ok(())
    }

    /// Close the owner, releasing every movie and source.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        for index in self.movies.keys().copied().collect::<Vec<_>>() {
            let _ = self.stop_inner(index);
        }
        for index in self.systems.keys().copied().collect::<Vec<_>>() {
            let _ = self.stop_inner(index);
        }
        self.sources.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct StubMounts {
        movies: HashMap<String, CinematicBytes>,
    }

    impl StubMounts {
        fn new() -> Self {
            // Minimal version 5, RLE, 8-bit, single-plane PCX (see still.rs).
            let mut bytes = vec![0u8; 128];
            bytes[..4].copy_from_slice(&[10, 5, 1, 8]);
            bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
            bytes[10..12].copy_from_slice(&0u16.to_le_bytes());
            bytes[65] = 1;
            bytes[66..68].copy_from_slice(&2u16.to_le_bytes());
            bytes.extend_from_slice(&[3, 4]);
            bytes.push(12);
            bytes.extend_from_slice(&(0..768).map(|index| (index % 256) as u8).collect::<Vec<_>>());
            Self {
                movies: HashMap::from([(
                    "video/idlog.pcx".to_string(),
                    CinematicBytes {
                        bytes,
                        resource: "pak0:video/idlog.pcx".to_string(),
                    },
                )]),
            }
        }
    }

    impl CinematicMounts for StubMounts {
        fn open(&mut self, name: &str) -> Option<CinematicBytes> {
            self.movies.get(name).cloned()
        }
    }

    struct StubImages {
        next: u32,
        committed: Vec<ImageOperation>,
        released: Vec<u32>,
    }

    impl StubImages {
        fn new() -> Self {
            Self {
                next: 7,
                committed: Vec::new(),
                released: Vec::new(),
            }
        }
    }

    impl CinematicImages for StubImages {
        fn allocate(&mut self, _width: usize, _height: usize, _resource: &str) -> u32 {
            let handle = self.next;
            self.next += 1;
            handle
        }

        fn commit(&mut self, operation: ImageOperation) {
            self.committed.push(operation);
        }

        fn release(&mut self, image: u32) {
            self.released.push(image);
        }
    }

    struct StubMixer {
        checkpoints: HashMap<String, Vec<u8>>,
    }

    impl StubMixer {
        fn new() -> Self {
            Self {
                checkpoints: HashMap::new(),
            }
        }
    }

    impl CinematicMixer for StubMixer {
        type StreamCheckpoint = Vec<u8>;

        fn queue_stream(&mut self, _target: &AudioStreamTarget, _pcm: &qa_client::media::audio::StreamPcm) {}
        fn stop_stream(&mut self, _id: &str) {}
        fn pause_stream(&mut self, _id: &str, _paused: bool) {}

        fn capture_stream_checkpoint(&self, id: &str) -> Option<Vec<u8>> {
            Some(self.checkpoints.get(id).cloned().unwrap_or_else(|| vec![9]))
        }

        fn restore_stream_checkpoint(&mut self, target: &AudioStreamTarget, checkpoint: &Vec<u8>) {
            self.checkpoints.insert(target.id.clone(), checkpoint.clone());
        }
    }

    struct StubSystem {
        status: SystemCinematicStatus,
        skipped: bool,
        stopped: bool,
        published: bool,
    }

    impl SystemCinematicHandle for StubSystem {
        fn status(&mut self) -> SystemCinematicStatus {
            self.status
        }
        fn skip(&mut self) {
            self.skipped = true;
        }
        fn stop(&mut self) {
            self.stopped = true;
        }
        fn capture_checkpoint(&self) -> Option<Vec<u8>> {
            Some(vec![1, 2])
        }
        fn publish_restored(&mut self) {
            self.published = true;
        }
    }

    struct StubSystemHost {
        opened: Vec<ScreenCinematicRequest>,
    }

    impl SystemCinematicHost for StubSystemHost {
        fn open(
            &mut self,
            request: ScreenCinematicRequest,
            current: &dyn Fn() -> bool,
        ) -> Box<dyn SystemCinematicHandle> {
            assert!(current());
            self.opened.push(request);
            Box::new(StubSystem {
                status: SystemCinematicStatus::Playing,
                skipped: false,
                stopped: false,
                published: false,
            })
        }

        fn restore(&mut self, checkpoint: &[u8], current: &dyn Fn() -> bool) -> Option<Box<dyn SystemCinematicHandle>> {
            assert!(current());
            assert_eq!(checkpoint, &[1, 2]);
            Some(Box::new(StubSystem {
                status: SystemCinematicStatus::Playing,
                skipped: false,
                stopped: false,
                published: false,
            }))
        }
    }

    type Owner = ApplicationQ3Cinematics<StubMixer, StubImages>;

    fn owner(system: bool) -> Owner {
        let registry = IdentityOwner::create("q3-cinematic-test").expect("owner");
        ApplicationQ3Cinematics::new(
            Box::new(StubMounts::new()),
            StubImages::new(),
            StubMixer::new(),
            registry.seat(0),
            Box::new(|| 1.0),
            Box::new(|_| {}),
            system.then(|| Box::new(StubSystemHost { opened: Vec::new() }) as Box<dyn SystemCinematicHost>),
        )
    }

    fn rect() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        }
    }

    #[test]
    fn guest_play_dedups_and_runs_held_still() {
        let mut cinematics = owner(false);
        let handle = cinematics.play_guest("idlog.pcx", rect(), 0).expect("play");
        assert!(handle >= 0);
        assert_eq!(cinematics.play_guest("idlog.pcx", rect(), 0).expect("replay"), handle);
        assert_eq!(cinematics.run_guest(handle).expect("run"), 0);
        assert_eq!(cinematics.run_guest(99).expect("missing"), 2);
    }

    #[test]
    fn missing_movie_returns_negative_one() {
        let mut cinematics = owner(false);
        assert_eq!(cinematics.play_guest("nope", rect(), 0).expect("play"), -1);
    }

    #[test]
    fn system_play_requires_host_and_reports_status() {
        let mut bare = owner(false);
        assert_eq!(
            bare.play_guest("idlog.pcx", rect(), 1).unwrap_err(),
            Q3CinematicError::NoSystemHost
        );
        let mut cinematics = owner(true);
        let handle = cinematics.play_guest("idlog.pcx", rect(), 1 | 2).expect("play");
        assert_eq!(cinematics.run_guest(handle).expect("run"), 1);
        assert_eq!(cinematics.stop_guest(handle).expect("stop"), 2);
        assert_eq!(cinematics.run_guest(handle).expect("run"), 2);
    }

    #[test]
    fn checkpoint_round_trips_movies_and_systems() {
        let mut cinematics = owner(true);
        let movie = cinematics.play_guest("idlog.pcx", rect(), 0).expect("movie");
        let system = cinematics.play_guest("idlog.pcx", rect(), 1).expect("system");
        assert_ne!(movie, system);
        let checkpoint = cinematics.capture_checkpoint().expect("capture");
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.movies.len(), 1);
        assert_eq!(checkpoint.systems.len(), 1);
        let mut restored = owner(true);
        restored.restore_checkpoint(&checkpoint).expect("restore");
        restored.publish_restored().expect("publish");
        let again = restored.capture_checkpoint().expect("recapture");
        assert_eq!(again.movies.len(), 1);
        assert_eq!(again.systems.len(), 1);
        assert_eq!(again.sources, checkpoint.sources);
    }

    #[test]
    fn restore_validates_owner_and_slots() {
        let mut cinematics = owner(false);
        let checkpoint = cinematics.capture_checkpoint().expect("empty capture");
        let mut busy = owner(false);
        busy.play_guest("idlog.pcx", rect(), 0).expect("play");
        assert_eq!(
            busy.restore_checkpoint(&checkpoint).unwrap_err(),
            Q3CinematicError::RestoreRequiresEmpty
        );
        let mut bad = checkpoint.clone();
        bad.version = 2;
        assert!(matches!(
            cinematics.restore_checkpoint(&bad).unwrap_err(),
            Q3CinematicError::BadCheckpoint(_)
        ));
        let mut systems = checkpoint;
        systems.systems.push(Q3SystemCheckpoint {
            index: 0,
            checkpoint: vec![1],
        });
        assert_eq!(
            cinematics.restore_checkpoint(&systems).unwrap_err(),
            Q3CinematicError::NoSystemRestore
        );
    }

    #[test]
    fn extents_and_close_release_slots() {
        let mut cinematics = owner(false);
        let handle = cinematics.play_guest("idlog.pcx", rect(), 0).expect("play");
        cinematics.set_extents(
            handle,
            Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
            },
        );
        cinematics.run(handle).expect("run");
        cinematics.stop(handle).expect("stop");
        assert_eq!(cinematics.run_guest(handle).expect("run"), 2);
        cinematics.close();
        assert_eq!(
            cinematics.play_guest("idlog.pcx", rect(), 0).unwrap_err(),
            Q3CinematicError::Closed
        );
    }
}
