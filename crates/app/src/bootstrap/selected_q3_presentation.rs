//! Supplemental selected-source Q3 cgame scenes, one per seat.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/selected-q3-presentation.ts`
//! (`ApplicationSelectedQ3Presentation`, `ApplicationSelectedQ3Presentations`).
//! Ownership, generation tracking, audio flushing, the frame gate, seat
//! retention, and aggregate cleanup errors are concrete here. The Q3
//! transport, scene presentation, services, renderer, and media live behind
//! [`SelectedQ3SceneBackend`] because their construction needs
//! `./q3-client/services.ts` (`createApplicationQ3Services`) and
//! `./q3-client/visibility.ts` (`selectApplicationQ3Snapshot`), which have
//! no lane port. Likewise the selected source arrives as
//! [`SelectedQ3Source`] (`./simulation/arsenal/q3-source.ts` has no lane
//! port), the viewing seat as [`SelectedQ3Seat`] (`./presentation.ts` has
//! no lane port), and the cgame audio sink as [`SelectedQ3AudioSink`]
//! (`./audio.ts` has no lane port). The component-effects frame
//! (`ApplicationEffectFrame` in `./effects.ts`, unported) stays
//! backend-opaque: the seat invokes a boolean render effect. Sync port:
//! the donor's async initialize/prepare become sync calls.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_content::contract::ContentId;
use qa_core::identity::{ActorId, ClientId, ProviderId, SeatId};
use thiserror::Error;

use super::audio::q3::Q3SeatAudioOperation;
use super::simulation::q3::types::Q3SourcePresentationState;
use super::ui::SeatPresentationEvent;

/// Selected-source Q3 presentation (donor `SelectedQ3Presentation`).
#[derive(Clone)]
pub struct SelectedQ3Presentation {
    /// Presenting content.
    pub content: ContentId,
    /// Selected source.
    pub source: Rc<dyn SelectedQ3Source>,
    /// Copied source state.
    pub state: Q3SourcePresentationState,
}

/// Selected Q3 source surface read by this module (donor `Q3SelectedSource`
/// in `./simulation/arsenal/q3-source.ts`).
pub trait SelectedQ3Source {
    /// Source revision (donor `generation`).
    fn generation(&self) -> u64;
    /// Restored presentation baseline, if any.
    fn presentation_baseline(&self) -> Option<Q3SourcePresentationState>;
    /// Whether the source is still open (donor `active`).
    fn is_active(&self) -> bool;
    /// Whether an actor is live on the source (donor `live`).
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Source actor for a sound slot, if any (donor `actor`).
    fn source_actor(&self, number: i32) -> Option<ActorId>;
    /// Whether a client slot is occupied (donor `pool.at(number).client`).
    fn pool_client_present(&self, number: i32) -> bool;
    /// Client slot count (donor `pool.maxClients`).
    fn max_clients(&self) -> i32;
    /// Hosting provider (donor `source.host.provider`).
    fn host_provider(&self) -> ProviderId;
}

/// Viewing seat surface read by this module (donor `WorldSeatPresentation`
/// in `./presentation.ts`).
pub trait SelectedQ3Seat {
    /// Local player actor.
    fn actor(&self) -> ActorId;
    /// Local seat.
    fn seat(&self) -> SeatId;
    /// Local client.
    fn client(&self) -> ClientId;
    /// Show center print (donor `presentation.ui.centerPrint`).
    fn center_print(&self, text: &str, time_ms: f64, duration_ms: f64);
    /// Bind the component render effect, returning its unbind closure.
    fn bind_component_effects(&self, render: SelectedQ3Effect) -> Box<dyn FnOnce()>;
}

/// Component render effect (donor `bindComponentEffects` callback): `true`
/// when the backend produced a frame.
pub type SelectedQ3Effect = Rc<dyn Fn() -> bool>;

/// Q3 hardware profile (donor `hardware()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectedQ3Hardware {
    /// Generic hardware.
    Generic,
    /// Rage Pro hardware.
    RagePro,
}

/// Cgame audio frame (donor `receiveCgameFrame` argument).
#[derive(Debug, Clone, PartialEq)]
pub struct SelectedQ3AudioFrame {
    /// Presenting content.
    pub content: ContentId,
    /// Owning provider.
    pub owner: ProviderId,
    /// Owning seat.
    pub seat: SeatId,
    /// Audio operations.
    pub operations: Vec<Q3SeatAudioOperation>,
}

/// Cgame audio sink (donor `options.audio.receiveCgameFrame`).
pub trait SelectedQ3AudioSink {
    /// Receive one cgame audio frame.
    fn receive_cgame_frame(&self, frame: SelectedQ3AudioFrame);
}

/// Backend construction inputs (donor `initialize` captures).
pub struct SelectedQ3Init<'a> {
    /// Local player actor.
    pub actor: &'a ActorId,
    /// Baseline state (restored baseline or current state).
    pub baseline: &'a Q3SourcePresentationState,
    /// Viewing seat.
    pub seat: &'a SeatId,
    /// Viewing client.
    pub client: &'a ClientId,
    /// Presenting content.
    pub content: &'a ContentId,
    /// Hosting provider.
    pub host_provider: &'a ProviderId,
    /// Hardware profile.
    pub hardware: SelectedQ3Hardware,
    /// Audio operation sink.
    pub audio: &'a dyn Fn(Q3SeatAudioOperation),
    /// Center-print sink.
    pub center_print: &'a dyn Fn(&str, f64, f64),
    /// Sound-slot actor resolver.
    pub source_actor: &'a dyn Fn(i32) -> Option<ActorId>,
    /// Diagnostic print sink.
    pub print: &'a dyn Fn(&str),
}

/// Supplemental Q3 scene backend (donor transport, game, services,
/// renderer, and media).
pub trait SelectedQ3SceneBackend {
    /// Build transport, services, game, and renderer (donor `initialize`).
    fn initialize(&mut self, init: &SelectedQ3Init<'_>) -> Result<(), SelectedQ3Error>;
    /// Whether a captured scene is ready for effects.
    fn has_scene(&self) -> bool;
    /// Render one component-effects frame; `true` when produced.
    fn render_effects(&self) -> bool;
    /// Reset one entity whose actor changed, refreshing client info when
    /// `client_info` is `Some` (donor generation-change branch).
    fn handle_generation_change(&mut self, number: i32, client_info: Option<&str>) -> Result<(), SelectedQ3Error>;
    /// Receive source state plus its content-filtered events (donor
    /// `transport.receive`).
    fn receive_state(&mut self, state: &Q3SourcePresentationState, events: &[SeatPresentationEvent]);
    /// Draw one scene frame (donor `game.frames.drawSceneFrame`).
    fn draw_frame(&mut self, server_time_ms: i32, engine_frame: i32) -> Result<(), SelectedQ3Error>;
    /// Capture, clear, and preload the scene (donor post-draw block).
    fn capture_scene(&mut self) -> Result<(), SelectedQ3Error>;
    /// Release backend resources (donor game/renderer/services/media
    /// closes).
    fn close(&mut self) -> Result<(), String>;
}

/// Backend factory (donor constructors behind `initialize`).
pub trait SelectedQ3BackendFactory {
    /// Backend type.
    type Backend: SelectedQ3SceneBackend;
    /// Create and initialize one backend.
    fn create(&self, init: &SelectedQ3Init<'_>) -> Result<Self::Backend, SelectedQ3Error>;
}

impl<F: SelectedQ3BackendFactory> SelectedQ3BackendFactory for &F {
    type Backend = F::Backend;

    fn create(&self, init: &SelectedQ3Init<'_>) -> Result<Self::Backend, SelectedQ3Error> {
        (*self).create(init)
    }
}

/// Selected Q3 presentation failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SelectedQ3Error {
    /// Operation failed.
    #[error("{0}")]
    Failed(String),
    /// Cleanup aggregated failures (donor `AggregateError`).
    #[error("{context}: {failures:?}")]
    Aggregate {
        /// Operation context.
        context: String,
        /// Failure messages.
        failures: Vec<String>,
    },
}

/// Selected Q3 presentation options (donor
/// `SelectedQ3PresentationOptions`).
pub struct SelectedQ3PresentationOptions<F> {
    /// Hardware profile.
    pub hardware: SelectedQ3Hardware,
    /// Cgame audio sink.
    pub audio: Rc<dyn SelectedQ3AudioSink>,
    /// Diagnostic print sink.
    pub print: Rc<dyn Fn(&str)>,
    /// Backend factory.
    pub factory: F,
}

/// One original selected-source cgame scene (donor
/// `ApplicationSelectedQ3Presentation`).
pub struct ApplicationSelectedQ3Presentation<B> {
    backend: Rc<RefCell<B>>,
    audio: Rc<RefCell<Vec<Q3SeatAudioOperation>>>,
    closed: Rc<Cell<bool>>,
    actor: ActorId,
    generation: u64,
    content: ContentId,
    source: Rc<dyn SelectedQ3Source>,
    seat: Rc<dyn SelectedQ3Seat>,
    audio_sink: Rc<dyn SelectedQ3AudioSink>,
    generations: HashMap<i32, ActorId>,
    frame: i32,
    busy: bool,
    unbind: Option<Box<dyn FnOnce()>>,
}

impl<B: SelectedQ3SceneBackend + 'static> ApplicationSelectedQ3Presentation<B> {
    /// Create and initialize one presentation (donor `create`).
    pub fn create<F>(
        options: &SelectedQ3PresentationOptions<F>,
        source: &SelectedQ3Presentation,
        seat: Rc<dyn SelectedQ3Seat>,
    ) -> Result<Self, SelectedQ3Error>
    where
        F: SelectedQ3BackendFactory<Backend = B>,
    {
        let actor = seat.actor();
        let audio: Rc<RefCell<Vec<Q3SeatAudioOperation>>> = Rc::new(RefCell::new(Vec::new()));
        let closed = Rc::new(Cell::new(false));
        let baseline = source
            .source
            .presentation_baseline()
            .unwrap_or_else(|| source.state.clone());
        let print = Rc::clone(&options.print);
        let audio_sink = Rc::clone(&options.audio);
        let push_audio = {
            let audio = Rc::clone(&audio);
            let closed = Rc::clone(&closed);
            Rc::new(move |operation: Q3SeatAudioOperation| {
                if closed.get() && matches!(operation, Q3SeatAudioOperation::ClearLoops { .. }) {
                    return;
                }
                audio.borrow_mut().push(operation);
            })
        };
        let center_print = {
            let seat = Rc::clone(&seat);
            Rc::new(move |text: &str, time: f64, duration: f64| seat.center_print(text, time, duration))
        };
        let source_actor = {
            let source = Rc::clone(&source.source);
            Rc::new(move |number: i32| source.source_actor(number))
        };
        let init = SelectedQ3Init {
            actor: &actor,
            baseline: &baseline,
            seat: &seat.seat(),
            client: &seat.client(),
            content: &source.content,
            host_provider: &source.source.host_provider(),
            hardware: options.hardware,
            audio: &|operation| push_audio(operation),
            center_print: &|text, time, duration| center_print(text, time, duration),
            source_actor: &|number| source_actor(number),
            print: &|text| print(text),
        };
        let backend = match options.factory.create(&init) {
            Ok(backend) => Rc::new(RefCell::new(backend)),
            Err(error) => {
                let audio_sink = Rc::clone(&audio_sink);
                audio_sink.receive_cgame_frame(SelectedQ3AudioFrame {
                    content: source.content.clone(),
                    owner: source.source.host_provider(),
                    seat: seat.seat(),
                    operations: vec![Q3SeatAudioOperation::ReleaseOwner],
                });
                return Err(error);
            }
        };
        let mut owner = Self {
            backend: Rc::clone(&backend),
            audio,
            closed: Rc::clone(&closed),
            actor,
            generation: source.source.generation(),
            content: source.content.clone(),
            source: Rc::clone(&source.source),
            seat: Rc::clone(&seat),
            audio_sink,
            generations: HashMap::new(),
            frame: -1,
            busy: false,
            unbind: None,
        };
        for row in baseline.entities.iter() {
            owner.generations.insert(row.state.number, row.actor.clone());
        }
        let render_closed = Rc::clone(&closed);
        let render_actor = owner.actor.clone();
        let render_generation = owner.generation;
        let render_source = Rc::clone(&owner.source);
        let render_backend = Rc::clone(&backend);
        let effect: SelectedQ3Effect = Rc::new(move || {
            if render_closed.get() || !render_backend.borrow().has_scene() {
                return false;
            }
            let live = render_source.is_active() && render_source.is_live(&render_actor);
            if !live || render_source.generation() != render_generation {
                return false;
            }
            render_backend.borrow().render_effects()
        });
        owner.unbind = Some(seat.bind_component_effects(effect));
        if let Err(error) = owner.assert_current(&seat) {
            owner.close_quiet();
            return Err(error);
        }
        owner.flush_audio();
        Ok(owner)
    }

    /// Whether a source/seat pair is still this presentation's own (donor
    /// `owns`).
    pub fn owns(&self, source: &SelectedQ3Presentation, seat: &Rc<dyn SelectedQ3Seat>) -> bool {
        owns_selected(
            self.closed.get(),
            self.generation,
            &self.actor,
            &self.content,
            &self.source,
            &self.seat,
            &source.content,
            &source.source,
            seat,
        )
    }

    /// Fail when this presentation no longer owns its source/seat (donor
    /// `assertCurrent`).
    fn assert_current(&self, seat: &Rc<dyn SelectedQ3Seat>) -> Result<(), SelectedQ3Error> {
        if self.closed.get() || !self.owns_self(seat) {
            return Err(SelectedQ3Error::Failed(
                "Selected Q3 presentation belongs to a retired source or viewer".to_string(),
            ));
        }
        Ok(())
    }

    /// Ownership of the stored source/seat pair.
    fn owns_self(&self, seat: &Rc<dyn SelectedQ3Seat>) -> bool {
        owns_selected(
            self.closed.get(),
            self.generation,
            &self.actor,
            &self.content,
            &self.source,
            &self.seat,
            &self.content,
            &self.source,
            seat,
        )
    }

    /// Flush queued audio to the sink (donor `flushAudio`).
    fn flush_audio(&self) {
        let operations = std::mem::take(&mut *self.audio.borrow_mut());
        if operations.is_empty() {
            return;
        }
        self.audio_sink.receive_cgame_frame(SelectedQ3AudioFrame {
            content: self.content.clone(),
            owner: self.source.host_provider(),
            seat: self.seat.seat(),
            operations,
        });
    }

    /// Prepare one frame (donor `prepare`).
    pub fn prepare(
        &mut self,
        source: &SelectedQ3Presentation,
        events: &[SeatPresentationEvent],
        frame: i32,
    ) -> Result<(), SelectedQ3Error> {
        let seat = Rc::clone(&self.seat);
        self.assert_current(&seat)?;
        if !self.owns(source, &seat) || self.busy {
            return Err(SelectedQ3Error::Failed(
                "Selected Q3 scene source changed or is already preparing".to_string(),
            ));
        }
        if frame <= self.frame {
            return Ok(());
        }
        self.busy = true;
        let result = self.prepare_frame(source, events, frame);
        self.busy = false;
        result
    }

    /// Frame preparation with cleanup on failure.
    fn prepare_frame(
        &mut self,
        source: &SelectedQ3Presentation,
        events: &[SeatPresentationEvent],
        frame: i32,
    ) -> Result<(), SelectedQ3Error> {
        let result = self.draw_source(source, events, frame);
        if let Err(error) = &result {
            let cleanup = self.close().err().map(|error| error.to_string());
            let mut failures = vec![error.to_string()];
            failures.extend(cleanup);
            return Err(SelectedQ3Error::Aggregate {
                context: "Selected Q3 frame and cleanup failed".to_string(),
                failures,
            });
        }
        Ok(())
    }

    /// Draw one source frame.
    fn draw_source(
        &mut self,
        source: &SelectedQ3Presentation,
        events: &[SeatPresentationEvent],
        frame: i32,
    ) -> Result<(), SelectedQ3Error> {
        for row in source.state.entities.iter() {
            let number = row.state.number;
            let changed = self
                .generations
                .get(&number)
                .is_some_and(|previous| previous != &row.actor);
            if changed {
                let client_info = if number < source.source.max_clients() {
                    Some(
                        source
                            .state
                            .configstrings
                            .iter()
                            .find(|row| row.index == 544 + number)
                            .map_or(String::new(), |row| row.value.clone()),
                    )
                } else {
                    None
                };
                self.backend
                    .borrow_mut()
                    .handle_generation_change(number, client_info.as_deref())?;
                let seat = Rc::clone(&self.seat);
                self.assert_current(&seat)?;
            }
        }
        for row in source.state.entities.iter() {
            self.generations.insert(row.state.number, row.actor.clone());
        }
        let filtered: Vec<SeatPresentationEvent> = events
            .iter()
            .filter(|event| event.content == source.content)
            .cloned()
            .collect();
        self.backend.borrow_mut().receive_state(&source.state, &filtered);
        self.backend.borrow_mut().draw_frame(source.state.time, frame)?;
        let seat = Rc::clone(&self.seat);
        self.assert_current(&seat)?;
        self.backend.borrow_mut().capture_scene()?;
        self.assert_current(&seat)?;
        self.flush_audio();
        self.frame = frame;
        Ok(())
    }

    /// Release the presentation (donor `close`).
    pub fn close(&mut self) -> Result<(), SelectedQ3Error> {
        if self.closed.get() {
            return Ok(());
        }
        self.closed.set(true);
        self.audio.borrow_mut().clear();
        let mut failures = Vec::new();
        if let Some(unbind) = self.unbind.take() {
            unbind();
        }
        if let Err(error) = self.backend.borrow_mut().close() {
            failures.push(error);
        }
        self.audio_sink.receive_cgame_frame(SelectedQ3AudioFrame {
            content: self.content.clone(),
            owner: self.source.host_provider(),
            seat: self.seat.seat(),
            operations: vec![Q3SeatAudioOperation::ReleaseOwner],
        });
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SelectedQ3Error::Aggregate {
                context: "Selected Q3 presentation cleanup failed".to_string(),
                failures,
            })
        }
    }

    /// Best-effort close during failed creation.
    fn close_quiet(&mut self) {
        self.closed.set(true);
        if let Some(unbind) = self.unbind.take() {
            unbind();
        }
        let _ = self.backend.borrow_mut().close();
    }

    /// Whether the presentation is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }

    /// Last prepared engine frame.
    #[must_use]
    pub fn frame(&self) -> i32 {
        self.frame
    }
}

/// Shared ownership check (donor `owns`).
#[allow(clippy::too_many_arguments)]
fn owns_selected(
    closed: bool,
    generation: u64,
    actor: &ActorId,
    content: &ContentId,
    owned_source: &Rc<dyn SelectedQ3Source>,
    owned_seat: &Rc<dyn SelectedQ3Seat>,
    source_content: &ContentId,
    source: &Rc<dyn SelectedQ3Source>,
    seat: &Rc<dyn SelectedQ3Seat>,
) -> bool {
    !closed
        && Rc::ptr_eq(source, owned_source)
        && source_content == content
        && source.generation() == generation
        && Rc::ptr_eq(seat, owned_seat)
        && seat.actor() == *actor
        && source.is_active()
        && source.is_live(actor)
}

/// SeatMap key by seat identity.
#[derive(Clone)]
struct SeatKey {
    seat: Rc<dyn SelectedQ3Seat>,
}

impl PartialEq for SeatKey {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.seat, &other.seat)
    }
}

impl Eq for SeatKey {}

impl std::hash::Hash for SeatKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Rc::as_ptr(&self.seat).addr().hash(state);
    }
}

/// Per-seat selected Q3 scenes (donor
/// `ApplicationSelectedQ3Presentations`).
pub struct ApplicationSelectedQ3Presentations<B, F> {
    options: SelectedQ3PresentationOptions<F>,
    seats: HashMap<SeatKey, ApplicationSelectedQ3Presentation<B>>,
    closed: bool,
    busy: bool,
}

impl<B: SelectedQ3SceneBackend + 'static, F: SelectedQ3BackendFactory<Backend = B>>
    ApplicationSelectedQ3Presentations<B, F>
{
    /// Create the collection (donor constructor).
    pub fn new(options: SelectedQ3PresentationOptions<F>) -> Self {
        Self {
            options,
            seats: HashMap::new(),
            closed: false,
            busy: false,
        }
    }

    /// Retire seats outside `presentations` (donor `retainPresentations`).
    pub fn retain_presentations(&mut self, presentations: &[Rc<dyn SelectedQ3Seat>]) -> Result<(), SelectedQ3Error> {
        if self.closed {
            return Err(SelectedQ3Error::Failed(
                "Selected Q3 presentation collection is closed".to_string(),
            ));
        }
        let mut failures = Vec::new();
        let retired: Vec<SeatKey> = self
            .seats
            .keys()
            .filter(|key| !presentations.iter().any(|seat| Rc::ptr_eq(seat, &key.seat)))
            .cloned()
            .collect();
        for key in retired {
            if let Some(mut consumer) = self.seats.remove(&key) {
                if let Err(error) = consumer.close() {
                    failures.push(error.to_string());
                }
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SelectedQ3Error::Aggregate {
                context: "Selected Q3 seat retirement failed".to_string(),
                failures,
            })
        }
    }

    /// Prepare one frame for every seat (donor `prepare`).
    pub fn prepare(
        &mut self,
        presentations: &[Rc<dyn SelectedQ3Seat>],
        source: Option<&SelectedQ3Presentation>,
        events: &[SeatPresentationEvent],
        frame: i32,
    ) -> Result<(), SelectedQ3Error> {
        if self.closed || self.busy {
            return Err(SelectedQ3Error::Failed(
                "Selected Q3 presentation collection is closed or already preparing".to_string(),
            ));
        }
        self.busy = true;
        let result = self.prepare_guarded(presentations, source, events, frame);
        self.busy = false;
        result
    }

    /// Guarded preparation.
    fn prepare_guarded(
        &mut self,
        presentations: &[Rc<dyn SelectedQ3Seat>],
        source: Option<&SelectedQ3Presentation>,
        events: &[SeatPresentationEvent],
        frame: i32,
    ) -> Result<(), SelectedQ3Error> {
        self.retain_presentations(presentations)?;
        let stale: Vec<SeatKey> = self
            .seats
            .iter()
            .filter(|(key, consumer)| source.is_none_or(|source| !consumer.owns(source, &key.seat)))
            .map(|(key, _)| key.clone())
            .collect();
        for key in stale {
            if let Some(mut consumer) = self.seats.remove(&key) {
                consumer.close()?;
            }
        }
        let Some(source) = source else {
            return Ok(());
        };
        for seat in presentations {
            let viewing = source.state.clients.iter().any(|row| row.actor == seat.actor());
            if !viewing {
                continue;
            }
            let key = SeatKey { seat: Rc::clone(seat) };
            if !self.seats.contains_key(&key) {
                let consumer = ApplicationSelectedQ3Presentation::create(&self.options, source, Rc::clone(seat))?;
                if self.closed {
                    let mut consumer = consumer;
                    let _ = consumer.close();
                    return Err(SelectedQ3Error::Failed(
                        "Selected Q3 presentation collection retired during initialization".to_string(),
                    ));
                }
                self.seats.insert(key.clone(), consumer);
            }
            if let Some(consumer) = self.seats.get_mut(&key) {
                consumer.prepare(source, events, frame)?;
            }
        }
        Ok(())
    }

    /// Release every seat (donor `close`).
    pub fn close(&mut self) -> Result<(), SelectedQ3Error> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut failures = Vec::new();
        for (_, mut consumer) in std::mem::take(&mut self.seats) {
            if let Err(error) = consumer.close() {
                failures.push(error.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SelectedQ3Error::Aggregate {
                context: "Selected Q3 presentation collection cleanup failed".to_string(),
                failures,
            })
        }
    }

    /// Whether the collection is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Seat count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seats.len()
    }

    /// Whether the collection holds no seats.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seats.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use qa_content::q3::base::shared::definitions::Product;
    use qa_content::q3::base::shared::entity_state::EntityState;
    use qa_content::q3::base::shared::player_state::create_player_state;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::simulation::q3::types::{
        Q3SourcePresentationClient, Q3SourcePresentationEntity, Q3SourcePresentationString,
    };

    use super::*;

    struct FakeSource {
        generation: Cell<u64>,
        active: Cell<bool>,
        live: RefCell<Vec<ActorId>>,
        actors: RefCell<HashMap<i32, ActorId>>,
        max_clients: i32,
        provider: ProviderId,
    }

    impl SelectedQ3Source for FakeSource {
        fn generation(&self) -> u64 {
            self.generation.get()
        }
        fn presentation_baseline(&self) -> Option<Q3SourcePresentationState> {
            None
        }
        fn is_active(&self) -> bool {
            self.active.get()
        }
        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.borrow().contains(actor)
        }
        fn source_actor(&self, number: i32) -> Option<ActorId> {
            self.actors.borrow().get(&number).cloned()
        }
        fn pool_client_present(&self, number: i32) -> bool {
            self.actors.borrow().contains_key(&number)
        }
        fn max_clients(&self) -> i32 {
            self.max_clients
        }
        fn host_provider(&self) -> ProviderId {
            self.provider.clone()
        }
    }

    struct FakeSeat {
        actor: ActorId,
        seat: SeatId,
        client: ClientId,
        effects: RefCell<Vec<SelectedQ3Effect>>,
    }

    impl SelectedQ3Seat for FakeSeat {
        fn actor(&self) -> ActorId {
            self.actor.clone()
        }
        fn seat(&self) -> SeatId {
            self.seat.clone()
        }
        fn client(&self) -> ClientId {
            self.client.clone()
        }
        fn center_print(&self, _text: &str, _time_ms: f64, _duration_ms: f64) {}
        fn bind_component_effects(&self, render: SelectedQ3Effect) -> Box<dyn FnOnce()> {
            self.effects.borrow_mut().push(render);
            Box::new(|| {})
        }
    }

    struct FakeBackend {
        scenes: usize,
        changes: RefCell<Vec<(i32, Option<String>)>>,
        draws: RefCell<Vec<(i32, i32)>>,
        received: RefCell<usize>,
        close_error: Option<String>,
    }

    impl SelectedQ3SceneBackend for FakeBackend {
        fn initialize(&mut self, _init: &SelectedQ3Init<'_>) -> Result<(), SelectedQ3Error> {
            Ok(())
        }
        fn has_scene(&self) -> bool {
            self.scenes > 0
        }
        fn render_effects(&self) -> bool {
            self.scenes > 0
        }
        fn handle_generation_change(&mut self, number: i32, client_info: Option<&str>) -> Result<(), SelectedQ3Error> {
            self.changes
                .borrow_mut()
                .push((number, client_info.map(str::to_string)));
            Ok(())
        }
        fn receive_state(&mut self, _state: &Q3SourcePresentationState, events: &[SeatPresentationEvent]) {
            *self.received.borrow_mut() += events.len();
        }
        fn draw_frame(&mut self, server_time_ms: i32, engine_frame: i32) -> Result<(), SelectedQ3Error> {
            self.draws.borrow_mut().push((server_time_ms, engine_frame));
            Ok(())
        }
        fn capture_scene(&mut self) -> Result<(), SelectedQ3Error> {
            self.scenes += 1;
            Ok(())
        }
        fn close(&mut self) -> Result<(), String> {
            if let Some(error) = self.close_error.clone() {
                return Err(error);
            }
            Ok(())
        }
    }

    struct FakeFactory {
        backends: RefCell<Vec<Rc<RefCell<FakeBackend>>>>,
        fail_create: bool,
    }

    impl SelectedQ3BackendFactory for FakeFactory {
        type Backend = SharedBackend;

        fn create(&self, init: &SelectedQ3Init<'_>) -> Result<Self::Backend, SelectedQ3Error> {
            if self.fail_create {
                return Err(SelectedQ3Error::Failed("factory failed".to_string()));
            }
            let backend = Rc::new(RefCell::new(FakeBackend {
                scenes: 0,
                changes: RefCell::new(Vec::new()),
                draws: RefCell::new(Vec::new()),
                received: RefCell::new(0),
                close_error: None,
            }));
            backend.borrow_mut().initialize(init)?;
            self.backends.borrow_mut().push(Rc::clone(&backend));
            Ok(SharedBackend { backend })
        }
    }

    struct SharedBackend {
        backend: Rc<RefCell<FakeBackend>>,
    }

    impl SelectedQ3SceneBackend for SharedBackend {
        fn initialize(&mut self, _init: &SelectedQ3Init<'_>) -> Result<(), SelectedQ3Error> {
            Ok(())
        }
        fn has_scene(&self) -> bool {
            self.backend.borrow().has_scene()
        }
        fn render_effects(&self) -> bool {
            self.backend.borrow().render_effects()
        }
        fn handle_generation_change(&mut self, number: i32, client_info: Option<&str>) -> Result<(), SelectedQ3Error> {
            self.backend.borrow_mut().handle_generation_change(number, client_info)
        }
        fn receive_state(&mut self, state: &Q3SourcePresentationState, events: &[SeatPresentationEvent]) {
            self.backend.borrow_mut().receive_state(state, events);
        }
        fn draw_frame(&mut self, server_time_ms: i32, engine_frame: i32) -> Result<(), SelectedQ3Error> {
            self.backend.borrow_mut().draw_frame(server_time_ms, engine_frame)
        }
        fn capture_scene(&mut self) -> Result<(), SelectedQ3Error> {
            self.backend.borrow_mut().capture_scene()
        }
        fn close(&mut self) -> Result<(), String> {
            self.backend.borrow_mut().close()
        }
    }

    struct FakeSink {
        frames: RefCell<Vec<SelectedQ3AudioFrame>>,
    }

    impl SelectedQ3AudioSink for FakeSink {
        fn receive_cgame_frame(&self, frame: SelectedQ3AudioFrame) {
            self.frames.borrow_mut().push(frame);
        }
    }

    struct Fixture {
        authority: IdentityOwner,
        source: Rc<FakeSource>,
        seat: Rc<FakeSeat>,
        sink: Rc<FakeSink>,
        factory: FakeFactory,
    }

    fn entity(number: i32, actor: ActorId) -> Q3SourcePresentationEntity {
        Q3SourcePresentationEntity {
            actor,
            state: EntityState {
                number,
                ..EntityState::default()
            },
            origin: vec3(0.0, 0.0, 0.0),
            linked: true,
            server_flags: 0,
            single_client: -1,
        }
    }

    fn state(time: i32, entities: Vec<Q3SourcePresentationEntity>, actor: ActorId) -> Q3SourcePresentationState {
        Q3SourcePresentationState {
            product: Product::Baseq3,
            time,
            entities,
            clients: vec![Q3SourcePresentationClient {
                actor,
                slot: 0,
                state: create_player_state(Product::Baseq3, None),
            }],
            configstrings: vec![Q3SourcePresentationString {
                index: 544,
                value: "ranger".to_string(),
            }],
        }
    }

    fn fixture() -> Fixture {
        let authority = IdentityOwner::create("selected-q3-test").unwrap();
        let actor = authority.actor(0, 0);
        let seat = authority.seat(0);
        let client = authority.client(0, 0);
        Fixture {
            authority,
            source: Rc::new(FakeSource {
                generation: Cell::new(7),
                active: Cell::new(true),
                live: RefCell::new(vec![actor.clone()]),
                actors: RefCell::new(HashMap::from([(0, actor.clone())])),
                max_clients: 32,
                provider: ProviderId {
                    namespace: "test".to_string(),
                    name: "host".to_string(),
                },
            }),
            seat: Rc::new(FakeSeat {
                actor: actor.clone(),
                seat,
                client,
                effects: RefCell::new(Vec::new()),
            }),
            sink: Rc::new(FakeSink {
                frames: RefCell::new(Vec::new()),
            }),
            factory: FakeFactory {
                backends: RefCell::new(Vec::new()),
                fail_create: false,
            },
        }
    }

    fn presentation(fixture: &Fixture, time: i32) -> SelectedQ3Presentation {
        let _ = &fixture.authority;
        SelectedQ3Presentation {
            content: ContentId("q3:test".to_string()),
            source: Rc::clone(&fixture.source) as Rc<dyn SelectedQ3Source>,
            state: state(
                time,
                vec![entity(0, fixture.seat.actor.clone())],
                fixture.seat.actor.clone(),
            ),
        }
    }

    fn options(fixture: &Fixture) -> SelectedQ3PresentationOptions<&FakeFactory> {
        SelectedQ3PresentationOptions {
            hardware: SelectedQ3Hardware::Generic,
            audio: Rc::clone(&fixture.sink) as Rc<dyn SelectedQ3AudioSink>,
            print: Rc::new(|_| {}),
            factory: &fixture.factory,
        }
    }

    #[test]
    fn create_owns_and_binds_effect() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let owner = ApplicationSelectedQ3Presentation::create(&options(&fixture), &source, Rc::clone(&seat)).unwrap();
        assert!(owner.owns(&source, &seat));
        assert_eq!(owner.frame(), -1);
        assert!(!owner.is_closed());
        assert_eq!(fixture.seat.effects.borrow().len(), 1);
        assert!(!fixture.seat.effects.borrow()[0]());
    }

    #[test]
    fn owns_rejects_retired_pairs() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let owner = ApplicationSelectedQ3Presentation::create(&options(&fixture), &source, Rc::clone(&seat)).unwrap();
        let other_source = Rc::new(FakeSource {
            generation: Cell::new(7),
            active: Cell::new(true),
            live: RefCell::new(vec![fixture.seat.actor.clone()]),
            actors: RefCell::new(HashMap::new()),
            max_clients: 32,
            provider: ProviderId {
                namespace: "test".to_string(),
                name: "other".to_string(),
            },
        });
        let foreign = SelectedQ3Presentation {
            content: ContentId("q3:test".to_string()),
            source: other_source as Rc<dyn SelectedQ3Source>,
            state: source.state.clone(),
        };
        assert!(!owner.owns(&foreign, &seat));
        fixture.source.generation.set(8);
        assert!(!owner.owns(&source, &seat));
        fixture.source.generation.set(7);
        fixture.source.active.set(false);
        assert!(!owner.owns(&source, &seat));
        fixture.source.active.set(true);
        fixture.source.live.borrow_mut().clear();
        assert!(!owner.owns(&source, &seat));
    }

    #[test]
    fn prepare_gates_frames_and_tracks_generations() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let mut owner =
            ApplicationSelectedQ3Presentation::create(&options(&fixture), &source, Rc::clone(&seat)).unwrap();
        owner.prepare(&source, &[], 3).unwrap();
        assert_eq!(owner.frame(), 3);
        assert_eq!(fixture.factory.backends.borrow().len(), 1);
        let backend = Rc::clone(&fixture.factory.backends.borrow()[0]);
        assert_eq!(*backend.borrow().draws.borrow(), vec![(100, 3)]);
        owner.prepare(&source, &[], 3).unwrap();
        assert_eq!(backend.borrow().draws.borrow().len(), 1);
        let other = fixture.authority.actor(1, 0);
        let mut changed = source.clone();
        changed.state.entities = vec![entity(0, other)];
        changed.state.time = 200;
        owner.prepare(&changed, &[], 4).unwrap();
        assert_eq!(
            *backend.borrow().changes.borrow(),
            vec![(0, Some("ranger".to_string()))]
        );
        assert_eq!(owner.frame(), 4);
    }

    #[test]
    fn prepare_rejects_foreign_source() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let mut owner =
            ApplicationSelectedQ3Presentation::create(&options(&fixture), &source, Rc::clone(&seat)).unwrap();
        let mut foreign = source.clone();
        foreign.content = ContentId("q3:other".to_string());
        let error = owner.prepare(&foreign, &[], 1).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Selected Q3 scene source changed or is already preparing"
        );
    }

    #[test]
    fn close_releases_owner_and_aggregates() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let mut owner =
            ApplicationSelectedQ3Presentation::create(&options(&fixture), &source, Rc::clone(&seat)).unwrap();
        owner.close().unwrap();
        assert!(owner.is_closed());
        assert!(!owner.owns(&source, &seat));
        let frames = fixture.sink.frames.borrow();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].operations, vec![Q3SeatAudioOperation::ReleaseOwner]);
        owner.close().unwrap();
    }

    #[test]
    fn factory_failure_returns_error() {
        let mut fixture = fixture();
        fixture.factory.fail_create = true;
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let error = ApplicationSelectedQ3Presentation::create(&options(&fixture), &source, seat)
            .err()
            .unwrap();
        assert_eq!(error.to_string(), "factory failed");
        assert_eq!(fixture.sink.frames.borrow().len(), 1);
    }

    #[test]
    fn collection_retains_prepares_and_closes() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let seat = Rc::clone(&fixture.seat) as Rc<dyn SelectedQ3Seat>;
        let factory = FakeFactory {
            backends: RefCell::new(Vec::new()),
            fail_create: false,
        };
        let options = SelectedQ3PresentationOptions {
            hardware: SelectedQ3Hardware::RagePro,
            audio: Rc::clone(&fixture.sink) as Rc<dyn SelectedQ3AudioSink>,
            print: Rc::new(|_| {}),
            factory,
        };
        let mut collection = ApplicationSelectedQ3Presentations::new(options);
        assert!(collection.is_empty());
        collection.prepare(&[Rc::clone(&seat)], Some(&source), &[], 1).unwrap();
        assert_eq!(collection.len(), 1);
        collection.prepare(&[], Some(&source), &[], 2).unwrap();
        assert!(collection.is_empty());
        collection.prepare(&[Rc::clone(&seat)], None, &[], 3).unwrap();
        assert!(collection.is_empty());
        collection.close().unwrap();
        assert!(collection.is_closed());
        assert!(collection.prepare(&[seat], Some(&source), &[], 4).is_err());
    }

    #[test]
    fn collection_skips_non_viewing_seats() {
        let fixture = fixture();
        let source = presentation(&fixture, 100);
        let stranger = Rc::new(FakeSeat {
            actor: fixture.authority.actor(9, 0),
            seat: fixture.authority.seat(1),
            client: fixture.authority.client(1, 0),
            effects: RefCell::new(Vec::new()),
        });
        let stranger = stranger as Rc<dyn SelectedQ3Seat>;
        let factory = FakeFactory {
            backends: RefCell::new(Vec::new()),
            fail_create: false,
        };
        let options = SelectedQ3PresentationOptions {
            hardware: SelectedQ3Hardware::Generic,
            audio: Rc::clone(&fixture.sink) as Rc<dyn SelectedQ3AudioSink>,
            print: Rc::new(|_| {}),
            factory,
        };
        let mut collection = ApplicationSelectedQ3Presentations::new(options);
        collection.prepare(&[stranger], Some(&source), &[], 1).unwrap();
        assert!(collection.is_empty());
    }
}
