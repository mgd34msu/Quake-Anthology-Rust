//! Native Q2 mod host: classic and rerelease sources behind one host.
//!
//! Port of donor `src/app/bootstrap/simulation/native-mod-host.ts`
//! (`createNativeModHost`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use qa_compat::q2::classic::records::RawEntityView;
use qa_content::contract::{ModCommandSource, ProviderReference};
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_world::spatial::ActorCollision;

use super::classic_guest_services::GuestCommandLine;
use super::source_hosts::ActorHostScene;

/// Native mod host error.
#[derive(Debug, thiserror::Error)]
pub enum NativeModHostError {
    /// Invalid host state or input.
    #[error("invalid native mod host: {0}")]
    Invalid(String),
}

impl NativeModHostError {
    /// Invalid-data error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

fn mapped(error: impl ToString) -> NativeModHostError {
    NativeModHostError::invalid(error.to_string())
}

/// Mirror of the `ModHostServices.actors` surface from donor
/// `src/world/session/mods.ts` (canonical home: session lane; unify
/// post-merge).
#[derive(Clone)]
pub struct ModActorServices {
    /// Actor liveness probe.
    pub is_live: Rc<dyn Fn(&ActorId) -> bool>,
    /// Resolve a saved actor.
    pub reference_saved: Rc<dyn Fn(SavedActorId) -> ActorId>,
}

/// Body-read callback for mod hosts.
pub type ModBodyReadFn = Rc<dyn Fn(&ActorId) -> Option<qa_content::q2::support::contracts::BodyState>>;
/// Engine command-text callback for mod hosts.
pub type ModCommandMessageFn = Rc<dyn Fn(ModCommandText, Option<ActorId>)>;

/// Mirror of the `ModHostServices.bodies` surface from donor
/// `src/world/session/mods.ts` (canonical home: session lane; unify
/// post-merge).
#[derive(Clone)]
pub struct ModBodyServices {
    /// Read an actor body.
    pub read: ModBodyReadFn,
}

/// Mirror of the `ModHostServices.engine` surface from donor
/// `src/world/session/mods.ts` (canonical home: session lane; unify
/// post-merge).
#[derive(Clone)]
pub struct ModServiceEngine {
    /// Engine print sink.
    pub print: Rc<dyn Fn(&str)>,
    /// Engine command-text sink, when the engine accepts messages.
    pub message: Option<ModCommandMessageFn>,
    /// Engine event sink.
    pub emit: Rc<dyn Fn(ModEngineEvent, f64, Option<ActorId>)>,
    /// Engine player list, when the engine tracks presentation.
    pub presentation_players: Option<Rc<dyn Fn() -> Vec<ActorId>>>,
}

/// Engine command text.
#[derive(Debug, Clone)]
pub struct ModCommandText {
    /// Text payload.
    pub text: String,
}

/// One debug-shape segment.
#[derive(Debug, Clone)]
pub struct DebugSegment {
    /// Segment start.
    pub start: Vec3,
    /// Segment end.
    pub end: Vec3,
    /// Segment color bytes.
    pub color: [u8; 3],
}

/// Looped sound payload for [`ModEngineEvent::Sound`].
#[derive(Debug, Clone)]
pub struct ModSoundEvent {
    /// Owning actor.
    pub actor: ActorId,
    /// Sound path.
    pub path: String,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Loop origin.
    pub origin: Vec3,
    /// Reliable delivery.
    pub reliable: bool,
    /// Loop state.
    pub loop_state: SoundLoopState,
    /// Loop owner.
    pub loop_owner: qa_core::identity::ProviderId,
}

/// Engine presentation event.
#[derive(Debug, Clone)]
pub enum ModEngineEvent {
    /// Debug graph sample.
    DebugGraph {
        /// Sample value.
        value: f64,
        /// Sample color.
        color: i64,
    },
    /// Entity event.
    EntityEvent {
        /// Owning actor.
        actor: ActorId,
        /// Event id.
        event: u32,
    },
    /// Looped sound.
    Sound(Box<ModSoundEvent>),
    /// Rerelease debug shapes.
    DebugShapes {
        /// Shape segments.
        segments: Vec<DebugSegment>,
        /// Lifetime in milliseconds.
        lifetime_ms: u32,
    },
    /// Rerelease world text.
    WorldText {
        /// Glyph text.
        text: String,
        /// Lifetime in seconds.
        lifetime: f32,
    },
    /// Translated service event passthrough.
    Service(Box<super::types::SimulationPresentationEvent>),
}

/// Looped sound state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundLoopState {
    /// Loop starts.
    Start,
    /// Loop stops.
    Stop,
}

/// Mirror of the `ModHostServices.commands` surface from donor
/// `src/world/session/mods.ts` (canonical home: session lane; unify
/// post-merge).
#[derive(Clone)]
pub struct ModCommandServices {
    /// Bind a console command.
    pub bind: Rc<dyn Fn(String, ModCommandHandler)>,
}

/// Mod console command handler.
pub type ModCommandHandler = Rc<dyn Fn(&ModCommandInvocation)>;

/// Mirror of the command invocation `commands.bind` handlers receive
/// (donor `src/world/session/mod-commands.ts`, canonical home: session
/// lane; unify post-merge).
#[derive(Debug, Clone)]
pub struct ModCommandInvocation {
    /// Arguments.
    pub arguments: Vec<String>,
    /// Tail text.
    pub args: String,
}

/// Mirror of the `ModHostServices.resources` surface from donor
/// `src/world/session/mods.ts` (canonical home: session lane; unify
/// post-merge).
#[derive(Clone)]
pub struct ModResourceServices {
    /// Own a native resource handle.
    pub own: Rc<dyn Fn(String)>,
}

/// Mirror of `ModHostServices` from donor `src/world/session/mods.ts`
/// (canonical home: session lane; unify post-merge).
#[derive(Clone)]
pub struct ModHostServices {
    /// Actor services.
    pub actors: ModActorServices,
    /// Body services.
    pub bodies: ModBodyServices,
    /// Engine services, when the host presents output.
    pub engine: Option<ModServiceEngine>,
    /// Source time.
    pub time: Rc<dyn Fn() -> qa_core::time::SourceTime>,
    /// Session seed.
    pub seed: u32,
    /// Command services.
    pub commands: ModCommandServices,
    /// Resource services.
    pub resources: ModResourceServices,
    /// Saved-actor resolver override.
    pub reference_saved: Option<Rc<dyn Fn(SavedActorId) -> ActorId>>,
}

/// Native mod projection: the provider-owned view between native slots
/// and primary actors (donor `NativeModProjection`).
pub trait NativeModProjection {
    /// Project a source record to its actor.
    fn project(&self, record: &RawEntityView) -> Option<OwnedActor>;
    /// Actor bound to a native slot.
    fn actor_at(&self, slot: u32) -> Option<ActorId>;
    /// Native slot bound to an actor.
    fn slot_of(&self, actor: &ActorId) -> Option<u32>;
    /// Whether a client slot may connect.
    fn accepts_client(&self, slot: u32) -> bool;
    /// Native address bound to an actor.
    fn address(&self, actor: &ActorId) -> Option<GuestAddress>;
    /// Run an import call inside the projection boundary.
    fn import_boundary(
        &self,
        name: &str,
        values: &[GuestCallValue],
        invoke: &dyn Fn() -> GuestCallResult,
    ) -> GuestCallResult;
}

/// Settable projection delegate for the provider/host bind dance: the
/// host is created before the provider exists, so creation installs an
/// empty projection that the provider replaces once it binds.
pub struct SharedProjection {
    inner: Rc<RefCell<Rc<dyn NativeModProjection>>>,
}

impl SharedProjection {
    /// Empty shared projection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(Rc::new(EmptyProjection))),
        }
    }

    /// Handle to the shared slot.
    #[must_use]
    pub fn handle(&self) -> Rc<dyn NativeModProjection> {
        Rc::new(Self {
            inner: Rc::clone(&self.inner),
        })
    }

    /// Install the bound projection.
    pub fn set(&self, projection: Rc<dyn NativeModProjection>) {
        *self.inner.borrow_mut() = projection;
    }
}

impl Default for SharedProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeModProjection for SharedProjection {
    fn project(&self, record: &RawEntityView) -> Option<OwnedActor> {
        self.inner.borrow().project(record)
    }

    fn actor_at(&self, slot: u32) -> Option<ActorId> {
        self.inner.borrow().actor_at(slot)
    }

    fn slot_of(&self, actor: &ActorId) -> Option<u32> {
        self.inner.borrow().slot_of(actor)
    }

    fn accepts_client(&self, slot: u32) -> bool {
        self.inner.borrow().accepts_client(slot)
    }

    fn address(&self, actor: &ActorId) -> Option<GuestAddress> {
        self.inner.borrow().address(actor)
    }

    fn import_boundary(
        &self,
        name: &str,
        values: &[GuestCallValue],
        invoke: &dyn Fn() -> GuestCallResult,
    ) -> GuestCallResult {
        self.inner.borrow().import_boundary(name, values, invoke)
    }
}

/// Empty slot projection before the provider binds.
pub struct EmptyProjection;

impl NativeModProjection for EmptyProjection {
    fn project(&self, _record: &RawEntityView) -> Option<OwnedActor> {
        None
    }

    fn actor_at(&self, _slot: u32) -> Option<ActorId> {
        None
    }

    fn slot_of(&self, _actor: &ActorId) -> Option<u32> {
        None
    }

    fn accepts_client(&self, _slot: u32) -> bool {
        false
    }

    fn address(&self, _actor: &ActorId) -> Option<GuestAddress> {
        None
    }

    fn import_boundary(
        &self,
        _name: &str,
        _values: &[GuestCallValue],
        invoke: &dyn Fn() -> GuestCallResult,
    ) -> GuestCallResult {
        invoke()
    }
}

/// Engine factory (donor `NativeModHostContext.engine`).
pub type EngineFactory = Rc<
    dyn Fn(
        &ProviderReference,
        &super::source_hosts::ActorHostRuntime,
    ) -> Box<dyn qa_content::q2::foundation::host::Q2FoundationHost>,
>;

/// Collision sink (donor `NativeModHostContext.collision`).
pub type ModCollisionSink = Rc<dyn Fn(&OwnedActor, &ActorCollision)>;

/// Mirror of the host clock (donor `NativeModHostContext.clock`).
#[derive(Clone)]
pub struct NativeModHostClock {
    /// Milliseconds reader.
    pub now_milliseconds: Rc<dyn Fn() -> i64>,
    /// Performance counter reader.
    pub performance_counter: Rc<dyn Fn() -> u64>,
    /// Performance counter frequency.
    pub performance_frequency: u64,
}

/// Mirror of `NativeModHostContext` from donor
/// `src/app/bootstrap/simulation/native-mod-host.ts` (canonical home:
/// this module).
#[derive(Clone)]
pub struct NativeModHostContext {
    /// Host scene queries.
    pub scene: Rc<dyn ActorHostScene>,
    /// Client capacity.
    pub max_clients: u32,
    /// Frame milliseconds.
    pub frame_ms: f64,
    /// Skill level.
    pub skill: i64,
    /// Session mode.
    pub mode: String,
    /// Gravity.
    pub gravity: f64,
    /// Host clock.
    pub clock: NativeModHostClock,
    /// Navigation services.
    pub navigation: Rc<RefCell<dyn qa_compat::q2::rerelease::navigation::NavigationServices>>,
    /// Collision sink.
    pub collision: ModCollisionSink,
    /// Map path.
    pub map_path: String,
    /// Entity text.
    pub entities: String,
    /// Spawn point.
    pub spawn_point: String,
    /// Engine factory.
    pub engine: EngineFactory,
}

/// Shared navigation adapter: services take ownership, so the context
/// lends its table through interior mutability. The borrowed runtime
/// handle cannot cross the shared borrow, so `runtime` reports none;
/// movement calls stay live.
struct SharedNavigation {
    inner: Rc<RefCell<dyn qa_compat::q2::rerelease::navigation::NavigationServices>>,
}

impl qa_compat::q2::rerelease::navigation::NavigationServices for SharedNavigation {
    fn runtime(&self) -> Option<&qa_compat::q2::rerelease::navigation::NavRuntime> {
        None
    }

    fn move_to_point(
        &mut self,
        actor: u32,
        point: Vec3,
        tolerance: f32,
    ) -> qa_compat::q2::rerelease::navigation::GoalStatus {
        self.inner.borrow_mut().move_to_point(actor, point, tolerance)
    }

    fn follow_actor(&mut self, actor: u32, target: u32) -> qa_compat::q2::rerelease::navigation::GoalStatus {
        self.inner.borrow_mut().follow_actor(actor, target)
    }
}

/// Guest command invocation (donor `CommandInvocation` from
/// `src/core/commands/index.ts`; canonical home: core lane, no Rust
/// counterpart — unify post-merge).
#[derive(Debug, Clone)]
pub struct GuestCommandInvocation {
    /// Arguments.
    pub argv: Vec<String>,
    /// Tail text.
    pub args_text: String,
    /// Invocation source.
    pub source: Option<ModCommandSource>,
    active: Rc<Cell<bool>>,
}

impl GuestCommandInvocation {
    /// Active invocation.
    #[must_use]
    pub fn new(argv: Vec<String>, args_text: String, source: Option<ModCommandSource>) -> Self {
        Self {
            argv,
            args_text,
            source,
            active: Rc::new(Cell::new(true)),
        }
    }

    /// Deactivate the invocation.
    pub fn deactivate(&self) {
        self.active.set(false);
    }

    /// Assert the invocation is still active.
    pub fn assert_active(&self) -> Result<(), NativeModHostError> {
        if self.active.get() {
            Ok(())
        } else {
            Err(NativeModHostError::invalid(
                "Native command invocation is no longer active",
            ))
        }
    }
}

/// Shared mutable command line with nesting (donor `withCommand`).
#[derive(Debug, Clone)]
pub struct NativeModCommandCell {
    line: Rc<RefCell<GuestCommandLine>>,
    source: Rc<RefCell<Option<ModCommandSource>>>,
}

impl NativeModCommandCell {
    /// Empty command cell.
    #[must_use]
    pub fn new() -> Self {
        Self {
            line: Rc::new(RefCell::new(GuestCommandLine {
                arguments: Vec::new(),
                args: String::new(),
            })),
            source: Rc::new(RefCell::new(None)),
        }
    }

    /// Reader closure for services options.
    #[must_use]
    pub fn reader(&self) -> Rc<dyn Fn() -> GuestCommandLine> {
        let line = Rc::clone(&self.line);
        Rc::new(move || line.borrow().clone())
    }

    /// Current invocation source, if any.
    #[must_use]
    pub fn source(&self) -> Option<ModCommandSource> {
        self.source.borrow().clone()
    }

    /// Run with an invocation installed, restoring the parent after.
    pub fn with<T>(
        &self,
        invocation: &GuestCommandInvocation,
        run: impl FnOnce() -> T,
    ) -> Result<T, NativeModHostError> {
        invocation.assert_active()?;
        let parent = self.line.borrow().clone();
        let parent_source = self.source.borrow().clone();
        *self.line.borrow_mut() = GuestCommandLine {
            arguments: invocation.argv.clone(),
            args: invocation.args_text.clone(),
        };
        *self.source.borrow_mut() = invocation.source.clone();
        let result = run();
        *self.line.borrow_mut() = parent;
        *self.source.borrow_mut() = parent_source;
        Ok(result)
    }
}

impl Default for NativeModCommandCell {
    fn default() -> Self {
        Self::new()
    }
}

use qa_compat::q2::classic::host::ClassicTraceResult;
use qa_compat::q2::classic::layout::classic_q2_exports;
use qa_compat::q2::rerelease::api::game_exports;
use qa_compat::q2::rerelease::host::{Q2Trace, RereleaseQ2GuestHost};
use qa_compat::q2::rerelease::layouts::{client_layout, edict_layout, field_offset};
use qa_compat::q2::rerelease::public_state::RereleasePublicEdict;
use qa_content::catalog::native_provider_timing;
use qa_content::contract::{GameFamily, ModCommandPort, NativeModDeclaration, SourceWeaponModel};
use qa_content::paths::normalize_resource_path;
use qa_content::q2::support::contracts::{TraceFamily, TraceResult};
use qa_core::cmd::{ascii_fold, Dialect};
use qa_core::cvar::CvarRegistry;
use qa_core::numeric::NumericOps;
use qa_guest::core::contracts::GuestCallSignature;
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::runtime::windows::contracts::WindowsCapabilities;
use qa_world::save::ownership::ProviderCheckpoint;
use qa_world::save::value::{decode_checkpoint_value, encode_checkpoint_value, SaveJson};

use super::classic_guest_services::{
    ClassicGuestServices, ClassicGuestServicesOptions, ResourceKind as ClassicResourceKind,
};
use super::classic_guest_source::{ClassicGuestSource, ClassicGuestSourceOptions, PreparedClassicGuest};
use super::native_mod_presentation::{ModEdition, NativeModPresentation, NativeModPresentationOptions};
use super::random::SourceRandom;
use super::rerelease_guest_services::RereleaseGuestServices;
use super::rerelease_guest_services_contract::{RereleaseGuestServicesOptions, RereleaseGuestServicesPort};
use super::rerelease_guest_source::{
    GuestClock, PreparedRereleaseGuest, RereleaseGuestSource, RereleaseGuestSourceOptions,
};
use super::source_hosts::ActorHostRuntime;
use super::types::GuestLocalize;
use crate::persistence::q2::classic_guest::{
    decode_q2_classic_original_save, encode_q2_classic_original_save, ClassicOriginalSaveFiles, Q2ClassicLevelState,
    Q2ClassicOriginalIdentity, Q2ClassicOriginalServerState,
};
use crate::persistence::PersistenceError;

/// Prepared guest for a native mod.
pub enum NativeModPrepared {
    /// Classic guest.
    Classic(PreparedClassicGuest),
    /// Rerelease guest.
    Rerelease(PreparedRereleaseGuest),
}

/// Cvar capture seam: donor `CvarRegistry.captureWorldTransferState`
/// has no Rust counterpart (canonical home: core lane; unify
/// post-merge).
pub type NativeModCvarCaptureFn = Rc<dyn Fn(&CvarRegistry) -> SaveJson>;

/// Cvar restore seam: donor `CvarRegistry.restoreSaveState` has no Rust
/// counterpart (canonical home: core lane; unify post-merge).
pub type NativeModCvarRestoreFn = Rc<dyn Fn(&mut CvarRegistry, &SaveJson) -> Result<(), NativeModHostError>>;

/// Command-port binder seam (donor `bindCommands`).
pub type BindCommandsFn = Rc<dyn Fn(&mut CvarRegistry) -> ModCommandPort>;

/// Native mod host options.
pub struct NativeModHostOptions {
    /// Prepared guest.
    pub prepared: NativeModPrepared,
    /// Mod declaration.
    pub declaration: NativeModDeclaration,
    /// Mod source identity.
    pub source: ProviderReference,
    /// Host context.
    pub context: NativeModHostContext,
    /// Mod services.
    pub services: ModHostServices,
    /// Slot projection.
    pub projection: Rc<dyn NativeModProjection>,
    /// Command-port binder.
    pub bind_commands: Option<BindCommandsFn>,
    /// Localizer.
    pub localize: GuestLocalize,
    /// Frame pump.
    pub next_frame: Box<dyn FnMut()>,
    /// Cvar capture seam.
    pub capture_cvars: NativeModCvarCaptureFn,
    /// Cvar restore seam.
    pub restore_cvars: NativeModCvarRestoreFn,
    /// Service translation seam.
    pub translate: super::native_mod_presentation::TranslateQ2ServiceRecordsFn,
    /// Wire fog seam.
    pub fog_from_wire: super::native_mod_presentation::Q2FogFromWireFn,
}

/// Native game export entry.
#[derive(Debug, Clone)]
pub struct NativeGameEntry {
    /// Export address.
    pub address: GuestAddress,
    /// Call signature.
    pub signature: GuestCallSignature,
}

/// Native source save image (donor `NativeModSourceSave`).
#[derive(Debug, Clone)]
pub enum NativeModSourceSave {
    /// Classic original save record.
    Classic {
        /// Encoded original save.
        original: ProviderCheckpoint,
    },
    /// Rerelease save parts.
    Rerelease {
        /// Game save bytes.
        game: Vec<u8>,
        /// Level save bytes.
        level: Vec<u8>,
        /// Encoded cvars.
        cvars: Vec<u8>,
        /// Configstrings.
        configstrings: Vec<(i32, String)>,
    },
}

impl NativeModSourceSave {
    /// Save edition tag.
    #[must_use]
    pub fn edition(&self) -> &'static str {
        match self {
            Self::Classic { .. } => "classic",
            Self::Rerelease { .. } => "rerelease",
        }
    }
}

/// Classic host parts.
pub struct ClassicHostParts {
    /// Guest source.
    pub source: ClassicGuestSource,
    /// Guest services.
    pub services: ClassicGuestServices,
    /// Original save files.
    pub files: ClassicOriginalSaveFiles,
    /// Command cell.
    pub command: NativeModCommandCell,
}

/// Rerelease host parts.
pub struct RereleaseHostParts {
    /// Guest source.
    pub source: RereleaseGuestSource,
    /// Guest services.
    pub services: RereleaseGuestServices,
    /// Command cell.
    pub command: NativeModCommandCell,
}

/// Native mod host.
pub struct NativeModHost {
    context: NativeModHostContext,
    source: ProviderReference,
    edition: ModEdition,
    parts: HostParts,
    presentation: NativeModPresentation,
    projection: Rc<dyn NativeModProjection>,
    commands: Option<Rc<ModCommandPort>>,
    capture_cvars: NativeModCvarCaptureFn,
    restore_cvars: NativeModCvarRestoreFn,
    source_time: Rc<Cell<Option<f64>>>,
    source_frame: Rc<Cell<u32>>,
    source_clock: Rc<dyn Fn() -> qa_core::time::SourceTime>,
    frame_seconds: f64,
    short_map: String,
    spawn_entities: Option<String>,
    entity_table: RefCell<qa_compat::q2::native_mod_provider::EntityTable>,
    guns: RefCell<std::collections::HashMap<u32, u32>>,
    next_frame: Box<dyn FnMut()>,
}

enum HostParts {
    Classic(Box<ClassicHostParts>),
    Rerelease(Box<RereleaseHostParts>),
}

/// Create a native mod host over a prepared guest.
pub fn create_native_mod_host(options: NativeModHostOptions) -> Result<NativeModHost, NativeModHostError> {
    let rerelease = matches!(options.prepared, NativeModPrepared::Rerelease(_));
    if rerelease && options.services.engine.is_none() {
        return Err(NativeModHostError::invalid(
            "Native component presentation requires destination engine services",
        ));
    }
    let timing = native_provider_timing(&options.source, GameFamily::Q2, rerelease);
    let mut cvars = CvarRegistry::new(if rerelease {
        Dialect::Q2Rerelease
    } else {
        Dialect::Q2Classic
    });
    let max_clients = resolve_max_clients(
        options.declaration.clients.as_ref().map(|clients| clients.maximum),
        options.context.max_clients,
    )?;
    for (name, value) in [
        ("maxclients", max_clients.to_string()),
        ("skill", options.context.skill.to_string()),
        ("deathmatch", u8::from(options.context.mode == "deathmatch").to_string()),
        ("coop", u8::from(options.context.mode == "coop").to_string()),
        ("sv_gravity", options.context.gravity.to_string()),
    ] {
        cvars.register(name, &value, 0).map_err(mapped)?;
    }
    for cvar in &options.declaration.cvars {
        if cvars.get(&cvar.name).is_none() {
            cvars.register(&cvar.name, &cvar.value, 0).map_err(mapped)?;
        } else {
            cvars.set(&cvar.name, &cvar.value, true).map_err(mapped)?;
        }
    }
    if options.declaration.clients.is_some()
        && cvars
            .get("maxclients")
            .is_none_or(|current| current.value.parse::<u32>().ok() != Some(max_clients))
    {
        return Err(NativeModHostError::invalid(
            "Native component client capacity differs from maxclients",
        ));
    }
    let commands = options.bind_commands.as_ref().map(|bind| Rc::new(bind(&mut cvars)));
    let frame_seconds = resolve_frame_seconds(
        options
            .declaration
            .source_actors
            .as_ref()
            .map(|actors| actors.frame_seconds),
        rerelease,
        options.context.frame_ms,
    );
    let source_time: Rc<Cell<Option<f64>>> = Rc::new(Cell::new(None));
    let source_frame: Rc<Cell<u32>> = Rc::new(Cell::new(0));
    let services_time = Rc::clone(&options.services.time);
    let runtime_time = Rc::clone(&services_time);
    let read_time = Rc::clone(&source_time);
    let runtime = ActorHostRuntime {
        numeric: timing.numeric,
        random: SourceRandom::new(options.services.seed),
        now: Box::new(move || read_time.get().unwrap_or_else(|| seconds(&(runtime_time)()))),
        frame_seconds: Box::new(move || frame_seconds),
        schedule: Box::new(|_, _| panic!("Native mod scheduling requires a declared source lifecycle callback")),
    };
    let engine = (options.context.engine)(&options.source, &runtime);
    let short_map = short_map_name(&options.context.map_path);
    let NativeModHostOptions {
        prepared,
        declaration,
        source,
        context,
        services,
        projection,
        bind_commands: _,
        localize,
        next_frame,
        capture_cvars,
        restore_cvars,
        translate,
        fog_from_wire,
    } = options;
    let mut host = match prepared {
        NativeModPrepared::Classic(prepared) => create_classic_host(
            &source,
            &context,
            &services,
            prepared,
            engine,
            cvars,
            max_clients,
            &projection,
            &commands,
        )?,
        NativeModPrepared::Rerelease(prepared) => create_rerelease_host(
            &source,
            &context,
            &services,
            &localize,
            prepared,
            engine,
            cvars,
            max_clients,
            &projection,
            &commands,
            frame_seconds,
        )?,
    };
    let presentation = NativeModPresentation::new(
        host.edition,
        NativeModPresentationOptions {
            admission: declaration.client_presentation,
            content: source.content.clone(),
            projection: Rc::clone(&projection),
            services: services.clone(),
            context: context.clone(),
            owner: source.provider.clone(),
            translate,
            fog_from_wire,
        },
    )
    .map_err(mapped)?;
    host.presentation = Some(presentation);
    Ok(NativeModHost {
        context,
        source,
        edition: host.edition,
        parts: host.parts,
        presentation: host.presentation.take().expect("presentation"),
        projection,
        commands,
        capture_cvars,
        restore_cvars,
        source_time,
        source_frame,
        source_clock: services_time,
        frame_seconds,
        short_map,
        spawn_entities: declaration.spawn_entities.clone(),
        entity_table: RefCell::new(qa_compat::q2::native_mod_provider::EntityTable {
            base: GuestAddress { space: 0, offset: 0 },
            stride: 0,
            count: 0,
            capacity: 0,
        }),
        guns: RefCell::new(std::collections::HashMap::new()),
        next_frame,
    })
}

fn seconds(time: &qa_core::time::SourceTime) -> f64 {
    match time {
        qa_core::time::SourceTime::Seconds(value) => f64::from(*value),
        qa_core::time::SourceTime::Milliseconds(value) => f64::from(*value) / 1000.0,
    }
}

struct PartialHost {
    edition: ModEdition,
    parts: HostParts,
    presentation: Option<NativeModPresentation>,
}

fn classic_print(services: &ModHostServices) -> super::classic_guest_services::GuestPrintFn {
    let engine = services.engine.clone();
    Box::new(move |text| {
        if let Some(engine) = engine.as_ref() {
            (engine.print)(text);
        }
    })
}

fn classic_add_command(
    commands: &Option<Rc<ModCommandPort>>,
    cell: &NativeModCommandCell,
) -> super::classic_guest_services::GuestAddCommandFn {
    let commands = commands.clone();
    let cell = cell.clone();
    Box::new(move |text| {
        let Some(commands) = commands.as_ref() else {
            panic!("Native gameplay mod requires a declared command buffer binding");
        };
        commands
            .append(text, cell.source().as_ref())
            .expect("native mod command buffer");
    })
}

fn classic_debug_graph(services: &ModHostServices) -> super::classic_guest_services::GuestDebugGraphFn {
    let services = services.clone();
    Box::new(move |value, color| {
        let Some(engine) = services.engine.as_ref() else {
            panic!("Native gameplay mod requires a declared destination presentation binding");
        };
        (engine.emit)(
            ModEngineEvent::DebugGraph {
                value,
                color: i64::from(color),
            },
            seconds(&(services.time)()),
            None,
        );
    })
}

#[allow(clippy::too_many_arguments)]
fn classic_base_options(
    source: &ProviderReference,
    context: &NativeModHostContext,
    services: &ModHostServices,
    rerelease: bool,
    engine: Box<dyn qa_content::q2::foundation::host::Q2FoundationHost>,
    cvars: CvarRegistry,
    max_clients: u32,
    command: &NativeModCommandCell,
    projection: &Rc<dyn NativeModProjection>,
    commands: &Option<Rc<ModCommandPort>>,
) -> Result<ClassicGuestServicesOptions, NativeModHostError> {
    let numeric = native_provider_timing(source, GameFamily::Q2, rerelease).numeric;
    let projection_accepts = Rc::clone(projection);
    let collision_sink = Rc::clone(&context.collision);
    Ok(ClassicGuestServicesOptions {
        primary_world: None,
        pickup_profile: None,
        pickups: None,
        damage_provenance: None,
        engine,
        scene: Rc::clone(&context.scene),
        cvars,
        numeric: NumericOps::select(numeric).map_err(mapped)?,
        map_path: context.map_path.clone(),
        max_clients,
        accepts_client: Some(Rc::new(move |slot| projection_accepts.accepts_client(slot))),
        admit: Box::new(|_, _| panic!("Native gameplay mod requires a declared owned actor binding")),
        collision: Box::new(move |actor, collision| collision_sink(actor, collision)),
        print: classic_print(services),
        command: command.reader(),
        add_command: classic_add_command(commands, command),
        debug_graph: classic_debug_graph(services),
    })
}

#[allow(clippy::too_many_arguments)]
fn create_classic_host(
    source: &ProviderReference,
    context: &NativeModHostContext,
    services: &ModHostServices,
    prepared: PreparedClassicGuest,
    engine: Box<dyn qa_content::q2::foundation::host::Q2FoundationHost>,
    cvars: CvarRegistry,
    max_clients: u32,
    projection: &Rc<dyn NativeModProjection>,
    commands: &Option<Rc<ModCommandPort>>,
) -> Result<PartialHost, NativeModHostError> {
    let command = NativeModCommandCell::new();
    let base = classic_base_options(
        source,
        context,
        services,
        false,
        engine,
        cvars,
        max_clients,
        &command,
        projection,
        commands,
    )?;
    let mut guest_services = ClassicGuestServices::new(base).map_err(mapped)?;
    let clock = context.clock.clone();
    let counter = context.clock.clone();
    let frequency = context.clock.performance_frequency;
    let mut source = ClassicGuestSource::create(
        prepared,
        ClassicGuestSourceOptions {
            capabilities: WindowsCapabilities {
                now_milliseconds: Some(Rc::new(move || (clock.now_milliseconds)())),
                performance_counter: Some(Rc::new(move || (counter.performance_counter)())),
                performance_frequency: Some(frequency),
                command_line: None,
                environment: None,
                open_file: None,
                standard_output: None,
                standard_input: None,
            },
            instruction_budget: None,
        },
    )
    .map_err(mapped)?;
    let image_base = source.image_base();
    guest_services
        .bind_host(&mut source.host, Some(image_base))
        .map_err(mapped)?;
    Ok(PartialHost {
        edition: ModEdition::Classic,
        parts: HostParts::Classic(Box::new(ClassicHostParts {
            source,
            services: guest_services,
            files: ClassicOriginalSaveFiles::new(None),
            command,
        })),
        presentation: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_rerelease_host(
    source: &ProviderReference,
    context: &NativeModHostContext,
    services: &ModHostServices,
    localize: &GuestLocalize,
    prepared: PreparedRereleaseGuest,
    engine: Box<dyn qa_content::q2::foundation::host::Q2FoundationHost>,
    cvars: CvarRegistry,
    max_clients: u32,
    projection: &Rc<dyn NativeModProjection>,
    commands: &Option<Rc<ModCommandPort>>,
    frame_seconds: f64,
) -> Result<PartialHost, NativeModHostError> {
    let command = NativeModCommandCell::new();
    let base = classic_base_options(
        source,
        context,
        services,
        true,
        engine,
        cvars,
        max_clients,
        &command,
        projection,
        commands,
    )?;
    let shapes_services = services.clone();
    let text_services = services.clone();
    let navigation: Box<dyn qa_compat::q2::rerelease::navigation::NavigationServices> = Box::new(SharedNavigation {
        inner: Rc::clone(&context.navigation),
    });
    let mut guest_services = RereleaseGuestServices::new(RereleaseGuestServicesOptions {
        base,
        frame_milliseconds: (frame_seconds * 1000.0).round() as i32,
        localize: localize.clone(),
        clipboard: super::rerelease_guest_services_contract::RereleaseGuestClipboard::Dedicated,
        debug_shapes: Box::new(move |event| {
            let Some(engine) = shapes_services.engine.as_ref() else {
                panic!("Native gameplay mod requires a declared destination presentation binding");
            };
            (engine.emit)(
                ModEngineEvent::DebugShapes {
                    segments: event
                        .lines
                        .iter()
                        .map(|line| DebugSegment {
                            start: line.start,
                            end: line.end,
                            color: [
                                (line.color.x.clamp(0.0, 1.0) * 255.0) as u8,
                                (line.color.y.clamp(0.0, 1.0) * 255.0) as u8,
                                (line.color.z.clamp(0.0, 1.0) * 255.0) as u8,
                            ],
                        })
                        .collect(),
                    lifetime_ms: event.lifetime_milliseconds,
                },
                seconds(&(shapes_services.time)()),
                None,
            );
        }),
        world_text: Box::new(move |event| {
            let Some(engine) = text_services.engine.as_ref() else {
                panic!("Native gameplay mod requires a declared destination presentation binding");
            };
            (engine.emit)(
                ModEngineEvent::WorldText {
                    text: event.text.text.clone(),
                    lifetime: event.lifetime,
                },
                seconds(&(text_services.time)()),
                None,
            );
        }),
        navigation,
        semantic_bindings: None,
        foreign_damage: None,
    })
    .map_err(mapped)?;
    let clock = context.clock.clone();
    let counter = context.clock.clone();
    let frequency = context.clock.performance_frequency;
    let mut source = RereleaseGuestSource::create(
        prepared,
        RereleaseGuestSourceOptions {
            clock: GuestClock {
                now_milliseconds: Rc::new(move || (clock.now_milliseconds)()),
                performance_counter: Rc::new(move || (counter.performance_counter)()),
                performance_frequency: frequency,
            },
            instruction_budget: None,
            foreign_damage: None,
            pickups: None,
            intercept_import: None,
        },
    )
    .map_err(mapped)?;
    guest_services.bind_host(&mut source.host).map_err(mapped)?;
    Ok(PartialHost {
        edition: ModEdition::Rerelease,
        parts: HostParts::Rerelease(Box::new(RereleaseHostParts {
            source,
            services: guest_services,
            command,
        })),
        presentation: None,
    })
}

use qa_compat::q2::classic::host::{ClassicSurface, TraceHitSlot};
use qa_compat::q2::classic::layout::CLASSIC_Q2_ABI;
use qa_compat::q2::native_mod_provider::EntityTable;
use qa_compat::q2::rerelease::host::{TraceHit as RereleaseTraceHit, TraceSurface};
use qa_content::q2::support::contracts::{Q2SurfaceInfo, TraceHit};

fn persist(error: impl ToString) -> PersistenceError {
    PersistenceError::BadSave(error.to_string())
}

/// Strip the maps prefix and bsp suffix for spawn calls.
fn short_map_name(map_path: &str) -> String {
    let without_prefix = map_path.strip_prefix("maps/").unwrap_or(map_path);
    without_prefix
        .strip_suffix(".bsp")
        .unwrap_or(without_prefix)
        .to_string()
}

/// Resolve the source frame cadence.
fn resolve_frame_seconds(declared: Option<f64>, rerelease: bool, frame_ms: f64) -> f64 {
    declared.unwrap_or(if rerelease { frame_ms / 1000.0 } else { 0.1 })
}

/// Resolve the client capacity.
fn resolve_max_clients(declared: Option<u64>, context_max: u32) -> Result<u32, NativeModHostError> {
    declared
        .map(|maximum| {
            u32::try_from(maximum).map_err(|_| NativeModHostError::invalid("Native mod client capacity exceeds u32"))
        })
        .transpose()
        .map(|resolved| resolved.unwrap_or(context_max))
}

/// Whether a command line is a server command.
fn is_sv_command(argv: &[String]) -> bool {
    argv.first().is_some_and(|head| ascii_fold(head) == "sv")
}

/// Validate an API2023 stance height.
fn validate_stance_height(view_height: f64) -> Result<i8, NativeModHostError> {
    let height = view_height.trunc();
    if !view_height.is_finite() || height < -128.0 || height > 127.0 {
        return Err(NativeModHostError::invalid(
            "Selected stance height exceeds API2023 pmove range",
        ));
    }
    Ok(height as i8)
}

/// Resolve a classic game export signature.
fn classic_game_signature(name: &str) -> Result<GuestCallSignature, NativeModHostError> {
    classic_q2_exports()
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.signature.clone())
        .ok_or_else(|| NativeModHostError::invalid(format!("Unknown API3 game export {name}")))
}

/// Resolve a rerelease game export signature.
fn rerelease_game_signature(name: &str) -> Result<GuestCallSignature, NativeModHostError> {
    game_exports()
        .iter()
        .find(|entry| entry.name == name)
        .map(|entry| entry.signature.clone())
        .ok_or_else(|| NativeModHostError::invalid(format!("Unknown API2023 game export {name}")))
}

fn q2_fields(trace: &TraceResult) -> Result<&qa_content::q2::support::contracts::Q2TraceFields, NativeModHostError> {
    match &trace.family {
        TraceFamily::Q2(fields) => Ok(fields),
        _ => Err(NativeModHostError::invalid("Native trace must be a Q2 trace")),
    }
}

fn trace_surface(surface: &Q2SurfaceInfo) -> TraceSurface {
    TraceSurface {
        name: surface.name.clone(),
        flags: surface.flags as u32,
        value: surface.value,
        material: surface.material.clone(),
    }
}

fn classic_surface(surface: &Q2SurfaceInfo) -> ClassicSurface {
    ClassicSurface {
        name: surface.name.clone(),
        flags: surface.flags,
        value: surface.value,
    }
}

/// Lower a scene trace to the classic trace record.
fn classic_trace_input(
    trace: &TraceResult,
    slot_of: &dyn Fn(&ActorId) -> Option<u32>,
) -> Result<ClassicTraceResult, NativeModHostError> {
    let fields = q2_fields(trace)?;
    let hit = match &trace.hit {
        TraceHit::None => TraceHitSlot::None,
        TraceHit::World { .. } => TraceHitSlot::World,
        TraceHit::Actor { actor } => slot_of(actor)
            .map(TraceHitSlot::Slot)
            .ok_or_else(|| NativeModHostError::invalid("Native trace hit an unprojected actor"))?,
    };
    Ok(ClassicTraceResult {
        all_solid: trace.all_solid,
        start_solid: trace.start_solid,
        fraction: trace.fraction as f32,
        end: trace.end,
        plane_normal: fields.source_plane.normal,
        plane_dist: fields.source_plane.distance,
        plane_type: fields.source_plane.plane_type as u8,
        plane_signbits: fields.source_plane.signbits as u8,
        contents: fields.contents,
        surface: fields.surface.as_ref().map(classic_surface),
        hit,
    })
}

/// Lower a scene trace to the rerelease trace record.
fn rerelease_trace_input(
    trace: &TraceResult,
    slot_of: &dyn Fn(&ActorId) -> Option<u32>,
) -> Result<Q2Trace, NativeModHostError> {
    let fields = q2_fields(trace)?;
    let hit = match &trace.hit {
        TraceHit::None | TraceHit::World { .. } => RereleaseTraceHit::World,
        TraceHit::Actor { actor } => slot_of(actor)
            .map(RereleaseTraceHit::Actor)
            .ok_or_else(|| NativeModHostError::invalid("Native trace hit an unprojected actor"))?,
    };
    Ok(Q2Trace {
        all_solid: trace.all_solid,
        start_solid: trace.start_solid,
        fraction: trace.fraction as f32,
        end: trace.end,
        plane_normal: fields.source_plane.normal,
        plane_distance: fields.source_plane.distance,
        plane_type: fields.source_plane.plane_type as u8,
        plane_signbits: fields.source_plane.signbits as u8,
        surface: fields.surface.as_ref().map(trace_surface),
        contents: fields.contents as u32,
        hit,
        secondary: fields.secondary.as_ref().map(|plane| {
            (
                plane.plane.normal,
                plane.plane.distance,
                plane.plane.plane_type as u8,
                plane.plane.signbits as u8,
                plane.surface.as_ref().map(trace_surface),
            )
        }),
    })
}

impl NativeModHost {
    /// Host edition.
    #[must_use]
    pub fn edition(&self) -> ModEdition {
        self.edition
    }

    /// Mod content identity.
    #[must_use]
    pub fn content(&self) -> &qa_content::contract::ContentId {
        &self.source.content
    }

    /// Guest console variables.
    #[must_use]
    pub fn cvars(&self) -> &CvarRegistry {
        match &self.parts {
            HostParts::Classic(parts) => parts.services.cvars(),
            HostParts::Rerelease(parts) => parts.services.cvars(),
        }
    }

    /// Mutable guest console variables.
    pub fn cvars_mut(&mut self) -> &mut CvarRegistry {
        match &mut self.parts {
            HostParts::Classic(parts) => parts.services.cvars_mut(),
            HostParts::Rerelease(parts) => parts.services.cvars_mut(),
        }
    }

    /// Client presentation.
    #[must_use]
    pub fn presentation(&self) -> &NativeModPresentation {
        &self.presentation
    }

    /// Mutable client presentation.
    pub fn presentation_mut(&mut self) -> &mut NativeModPresentation {
        &mut self.presentation
    }

    /// Slot projection.
    #[must_use]
    pub fn projection(&self) -> &Rc<dyn NativeModProjection> {
        &self.projection
    }

    /// Bound command port, if any.
    #[must_use]
    pub fn commands(&self) -> Option<&ModCommandPort> {
        self.commands.as_ref().map(Rc::as_ref)
    }

    /// Guest address space.
    pub fn memory(&mut self) -> &mut SparseGuestMemory {
        match &mut self.parts {
            HostParts::Classic(parts) => &mut parts.source.host.memory,
            HostParts::Rerelease(parts) => &mut parts.source.host.memory,
        }
    }

    /// Current source time in seconds.
    #[must_use]
    pub fn source_time(&self) -> f64 {
        self.source_time
            .get()
            .unwrap_or_else(|| seconds(&(self.source_clock)()))
    }

    /// Image base address.
    #[must_use]
    pub fn image_base(&self) -> GuestAddress {
        match &self.parts {
            HostParts::Classic(parts) => parts.source.image_base(),
            HostParts::Rerelease(parts) => parts.source.image_base(),
        }
    }

    fn classic_parts_mut(&mut self) -> Result<&mut ClassicHostParts, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => Ok(parts.as_mut()),
            HostParts::Rerelease(_) => Err(NativeModHostError::invalid(
                "Native classic call reached a rerelease host",
            )),
        }
    }

    fn command_cell(&self) -> &NativeModCommandCell {
        match &self.parts {
            HostParts::Classic(parts) => &parts.command,
            HostParts::Rerelease(parts) => &parts.command,
        }
    }

    fn classic_identity(&mut self) -> Result<Q2ClassicOriginalIdentity, NativeModHostError> {
        let module = match &mut self.parts {
            HostParts::Classic(parts) => {
                let module = parts.source.memory().module().clone();
                qa_guest::checkpoint::ModuleIdentity {
                    id: format!("{}:{}", module.id.namespace, module.id.name),
                    artifact_path: module.artifact_path.clone(),
                    digest: format!("{}:{}", module.digest.algorithm, module.digest.value),
                    revision: module.revision.clone(),
                }
            }
            HostParts::Rerelease(_) => {
                return Err(NativeModHostError::invalid(
                    "Native classic call reached a rerelease host",
                ));
            }
        };
        Ok(Q2ClassicOriginalIdentity {
            module,
            map: self.context.map_path.clone(),
        })
    }

    fn snapshot_entity_table(&mut self) -> Result<EntityTable, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let host = &mut parts.source.host;
                let descriptor = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| NativeModHostError::invalid("Classic native entities are not bound"))?
                    .descriptor(&mut host.memory)
                    .map_err(mapped)?;
                Ok(EntityTable {
                    base: descriptor.base,
                    stride: descriptor.stride,
                    count: descriptor.count,
                    capacity: descriptor.capacity,
                })
            }
            HostParts::Rerelease(parts) => {
                let host = &parts.source.host;
                Ok(EntityTable {
                    base: host.entity_base,
                    stride: host.entity_stride,
                    count: host.entity_count as usize,
                    capacity: host.entity_capacity as usize,
                })
            }
        }
    }

    /// Synchronize the source frame clock.
    pub fn synchronize_frame(&mut self, seconds: f64, frame: u32) {
        self.source_time.set(Some(seconds));
        self.source_frame.set(frame);
        if let HostParts::Rerelease(parts) = &mut self.parts {
            parts.services.begin_frame(frame);
        }
        self.refresh_guns();
    }

    fn refresh_guns(&mut self) {
        let mut guns = std::collections::HashMap::new();
        match &mut self.parts {
            HostParts::Classic(parts) => {
                for slot in 1..=parts.services.max_clients() {
                    if let Ok(state) = ClassicGuestServices::player_state(&mut parts.source.host, slot) {
                        if state.view.gun_index != 0 {
                            guns.insert(slot, state.view.gun_index as u32);
                        }
                    }
                }
            }
            HostParts::Rerelease(parts) => {
                for slot in 1..=parts.services.max_clients() {
                    if let Ok(gun) = rerelease_gun_index(&mut parts.source.host, slot) {
                        if gun != 0 {
                            guns.insert(slot, gun as u32);
                        }
                    }
                }
            }
        }
        *self.guns.borrow_mut() = guns;
    }

    /// Read the source record for a slot.
    pub fn entity(&mut self, slot: u32) -> Result<RawEntityView, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let host = &mut parts.source.host;
                host.edicts
                    .as_mut()
                    .ok_or_else(|| NativeModHostError::invalid("Classic native entities are not bound"))?
                    .at(&mut host.memory, slot)
                    .map_err(mapped)
            }
            HostParts::Rerelease(parts) => {
                let host = &parts.source.host;
                Ok(RawEntityView {
                    slot,
                    address: host.record_at(slot).map_err(mapped)?,
                    stride_bytes: host.entity_stride,
                })
            }
        }
    }

    /// Read the client pointer for a slot.
    pub fn client(&mut self, slot: u32) -> Result<Option<GuestAddress>, NativeModHostError> {
        if matches!(self.parts, HostParts::Classic(_)) {
            let record = self.entity(slot)?;
            let HostParts::Classic(parts) = &mut self.parts else {
                unreachable!()
            };
            let host = &mut parts.source.host;
            return host
                .memory
                .read_pointer(host.memory.offset(record.address, 84).map_err(mapped)?)
                .map_err(mapped);
        }
        match &mut self.parts {
            HostParts::Classic(_) => unreachable!(),
            HostParts::Rerelease(parts) => {
                let host = &mut parts.source.host;
                let address = host.record_at(slot).map_err(mapped)?;
                let mut view = RereleasePublicEdict::new(&mut host.memory, address).map_err(mapped)?;
                view.pointer("client").map_err(mapped)
            }
        }
    }

    /// Whether a slot is active.
    pub fn active(&mut self, slot: u32) -> Result<bool, NativeModHostError> {
        if matches!(self.parts, HostParts::Classic(_)) {
            let record = self.entity(slot)?;
            let HostParts::Classic(parts) = &mut self.parts else {
                unreachable!()
            };
            let host = &mut parts.source.host;
            return Ok(host
                .memory
                .read_i32(host.memory.offset(record.address, 88).map_err(mapped)?)
                .map_err(mapped)?
                != 0);
        }
        match &mut self.parts {
            HostParts::Classic(_) => unreachable!(),
            HostParts::Rerelease(parts) => Ok(parts
                .services
                .entity_info(&mut parts.source.host, slot)
                .map_err(mapped)?
                .active),
        }
    }

    /// Clear the entity event for a slot.
    pub fn clear_entity_event(&mut self, slot: u32) -> Result<(), NativeModHostError> {
        if matches!(self.parts, HostParts::Classic(_)) {
            let record = self.entity(slot)?;
            let HostParts::Classic(parts) = &mut self.parts else {
                unreachable!()
            };
            let host = &mut parts.source.host;
            return host
                .memory
                .write_i32(host.memory.offset(record.address, 80).map_err(mapped)?, 0)
                .map_err(mapped);
        }
        match &mut self.parts {
            HostParts::Classic(_) => unreachable!(),
            HostParts::Rerelease(parts) => {
                let host = &mut parts.source.host;
                let record = host.record_at(slot).map_err(mapped)?;
                let event = field_offset(&edict_layout(), "s.event").map_err(mapped)? as i64;
                host.memory
                    .write_u8(host.memory.offset(record, event).map_err(mapped)?, 0)
                    .map_err(mapped)
            }
        }
    }

    /// Resolve the view weapon model for a slot.
    pub fn weapon_model(&mut self, slot: u32) -> Result<Option<SourceWeaponModel>, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let state = ClassicGuestServices::player_state(&mut parts.source.host, slot).map_err(mapped)?;
                if state.view.gun_index == 0 {
                    return Ok(None);
                }
                let path = parts
                    .services
                    .resource(ClassicResourceKind::Model, state.view.gun_index);
                if path.is_empty() {
                    return Err(NativeModHostError::invalid(
                        "Original native viewmodel has no model configstring",
                    ));
                }
                Ok(Some(SourceWeaponModel::SourcePath {
                    path: normalize_resource_path(&path).map_err(mapped)?,
                    frame: f64::from(state.view.gun_frame),
                }))
            }
            HostParts::Rerelease(parts) => {
                let (gun_index, gun_frame) = rerelease_gun(&mut parts.source.host, slot).map_err(mapped)?;
                if gun_index == 0 {
                    return Ok(None);
                }
                let path = parts
                    .services
                    .resource(super::rerelease_guest_services::ResourceKind::Model, gun_index);
                if path.is_empty() {
                    return Err(NativeModHostError::invalid(
                        "Original native viewmodel has no model configstring",
                    ));
                }
                Ok(Some(SourceWeaponModel::SourcePath {
                    path: normalize_resource_path(&path).map_err(mapped)?,
                    frame: f64::from(gun_frame),
                }))
            }
        }
    }

    /// Project the player view for a stance height (rerelease only).
    pub fn project_player_view(&mut self, slot: u32, view_height: f64) -> Result<(), NativeModHostError> {
        let HostParts::Rerelease(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Classic native mods have no stance-height projection",
            ));
        };
        let height = validate_stance_height(view_height)?;
        let host = &mut parts.source.host;
        let record = host.record_at(slot).map_err(mapped)?;
        let mut view = RereleasePublicEdict::new(&mut host.memory, record).map_err(mapped)?;
        let client = view.client().map_err(mapped)?;
        host.memory
            .write_u8(host.memory.offset(client, 48).map_err(mapped)?, height as u8)
            .map_err(mapped)
    }

    /// Encode a scene trace to guest bytes.
    pub fn encode_trace(&mut self, trace: &TraceResult) -> Result<Vec<u8>, NativeModHostError> {
        let projection = Rc::clone(&self.projection);
        let slot_of = |actor: &ActorId| projection.slot_of(actor);
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let input = classic_trace_input(trace, &slot_of)?;
                parts.source.host.trace_bytes(&input).map_err(mapped)
            }
            HostParts::Rerelease(parts) => {
                let input = rerelease_trace_input(trace, &slot_of)?;
                match parts.source.host.encode_trace(&input, None).map_err(mapped)? {
                    GuestCallResult::Value(qa_guest::core::contracts::GuestCallValue::Aggregate { bytes, .. }) => {
                        Ok(bytes)
                    }
                    _ => Err(NativeModHostError::invalid("Native trace must be an aggregate")),
                }
            }
        }
    }

    /// Run a nested command invocation.
    pub fn with_command<T>(
        &self,
        invocation: &GuestCommandInvocation,
        run: impl FnOnce() -> T,
    ) -> Result<T, NativeModHostError> {
        self.command_cell().with(invocation, run)
    }

    /// Run a server command through the guest.
    pub fn invoke_command(&mut self, invocation: &GuestCommandInvocation) -> Result<bool, NativeModHostError> {
        if !is_sv_command(&invocation.argv) {
            return Ok(false);
        }
        let cell = self.command_cell().clone();
        let rerelease = matches!(self.parts, HostParts::Rerelease(_));
        let run = cell.with(invocation, || {
            if rerelease {
                // The DLL `ServerCommand` dispatch is skipped headless: no
                // guest module layer exists to receive the call. The
                // invocation still installs so `command`/`addCommand`
                // observe it.
                Ok(())
            } else {
                self.classic_parts_mut()?
                    .source
                    .host
                    .call("ServerCommand", &[])
                    .map(|_| ())
                    .map_err(mapped)
            }
        })?;
        run?;
        Ok(true)
    }

    /// Resolve a named guest export.
    pub fn entry(&self, name: &str) -> Result<GuestAddress, NativeModHostError> {
        match &self.parts {
            HostParts::Classic(parts) => parts.source.entry(name).map_err(mapped),
            HostParts::Rerelease(parts) => parts.source.entry(name).map_err(mapped),
        }
    }

    /// Resolve a named game export entry.
    pub fn game_entry(&mut self, name: &str) -> Result<NativeGameEntry, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(_) => {
                let signature = classic_game_signature(name)?;
                Err(NativeModHostError::invalid(format!(
                    "Classic game export table is not exposed to native hosts ({name} has signature {signature:?})"
                )))
            }
            HostParts::Rerelease(_) => {
                let signature = rerelease_game_signature(name)?;
                Err(NativeModHostError::invalid(format!(
                    "Rerelease game export calls require the guest module layer ({name} has signature {signature:?})"
                )))
            }
        }
    }

    /// Read the live entity table.
    pub fn entities(&mut self) -> Result<EntityTable, NativeModHostError> {
        self.snapshot_entity_table()
    }

    /// Invoke a guest entry.
    pub fn invoke(
        &mut self,
        entry: GuestAddress,
        signature: &GuestCallSignature,
        values: &[GuestCallValue],
    ) -> Result<GuestCallResult, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let budget = parts.source.budget();
                let host = &mut parts.source.host;
                host.cvars.refresh(&mut host.memory).map_err(mapped)?;
                host.invoke(entry, signature, values, budget).map_err(mapped)
            }
            HostParts::Rerelease(_) => Err(NativeModHostError::invalid(
                "Rerelease export calls require the guest module layer",
            )),
        }
    }

    fn spawn_classic(&mut self) -> Result<(), NativeModHostError> {
        let short_map = self.short_map.clone();
        let spawn_entities = self.spawn_entities.clone();
        let HostParts::Classic(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Native classic call reached a rerelease host",
            ));
        };
        if let Some(entities) = spawn_entities.as_ref() {
            parts
                .source
                .host
                .spawn_entities(&short_map, entities, "")
                .map_err(mapped)?;
        }
        parts.services.complete_spawn();
        Ok(())
    }

    fn spawn_rerelease(&mut self) -> Result<(), NativeModHostError> {
        let HostParts::Rerelease(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Native rerelease call reached a classic host",
            ));
        };
        // Entity spawn runs inside the guest module layer, which has no
        // Rust counterpart; the prebuilt entity table stands in.
        let _ = &parts.source.host.entity_base;
        parts.services.complete_spawn();
        Ok(())
    }

    fn cache_entity_table(&mut self) {
        if let Ok(table) = self.snapshot_entity_table() {
            *self.entity_table.borrow_mut() = table;
        }
    }

    /// Initialize the guest, spawning unless restoring.
    pub fn initialize(&mut self, restoring: bool) -> Result<(), NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                parts.source.init_loading(&mut *self.next_frame).map_err(mapped)?;
            }
            HostParts::Rerelease(parts) => {
                parts.source.init_loading(&mut *self.next_frame).map_err(mapped)?;
            }
        }
        if !restoring {
            match &mut self.parts {
                HostParts::Classic(_) => self.spawn_classic()?,
                HostParts::Rerelease(_) => self.spawn_rerelease()?,
            }
        }
        self.cache_entity_table();
        Ok(())
    }

    /// Capture the source save image.
    pub fn checkpoint(&mut self) -> Result<NativeModSourceSave, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(_) => self.checkpoint_classic(),
            HostParts::Rerelease(_) => self.checkpoint_rerelease(),
        }
    }

    fn checkpoint_classic(&mut self) -> Result<NativeModSourceSave, NativeModHostError> {
        let identity = self.classic_identity()?;
        let HostParts::Classic(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Native classic call reached a rerelease host",
            ));
        };
        let mut configstrings: Vec<(i64, String)> = parts
            .services
            .configstrings()
            .into_iter()
            .map(|(index, value)| (i64::from(index), value))
            .collect();
        configstrings.sort_by_key(|(index, _)| *index);
        let capture_cvars = Rc::clone(&self.capture_cvars);
        let capture = capture_cvars(parts.services.cvars());
        let server = Q2ClassicOriginalServerState {
            state: Q2ClassicLevelState {
                configstrings,
                portals: Vec::new(),
            },
            cvars: encode_checkpoint_value(&capture),
        };
        let ClassicHostParts {
            source,
            services: _,
            files,
            command: _,
        } = parts.as_mut();
        let host = &mut source.host;
        let save = files
            .capture(
                &identity,
                server,
                |game, level, autosave| {
                    host.save("WriteGame", game, autosave).map_err(persist)?;
                    host.save("WriteLevel", level, false).map_err(persist)?;
                    Ok(())
                },
                false,
            )
            .map_err(mapped)?;
        Ok(NativeModSourceSave::Classic {
            original: encode_q2_classic_original_save(&save).map_err(mapped)?,
        })
    }

    fn checkpoint_rerelease(&mut self) -> Result<NativeModSourceSave, NativeModHostError> {
        let HostParts::Rerelease(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Native rerelease call reached a classic host",
            ));
        };
        let game = parts.source.host.write_save("game", false).map_err(mapped)?;
        let level = parts.source.host.write_save("level", false).map_err(mapped)?;
        if !game.deferred.is_empty()
            || !game.projections.is_empty()
            || !level.deferred.is_empty()
            || !level.projections.is_empty()
        {
            return Err(NativeModHostError::invalid(
                "Unexpected whole-world native mod save state",
            ));
        }
        let mut configstrings: Vec<(i32, String)> = parts.services.configstrings().into_iter().collect();
        configstrings.sort_by_key(|(index, _)| *index);
        Ok(NativeModSourceSave::Rerelease {
            game: game.native,
            level: level.native,
            cvars: encode_checkpoint_value(&(self.capture_cvars)(parts.services.cvars())),
            configstrings,
        })
    }

    /// Restore the source save image.
    pub fn restore(&mut self, save: &NativeModSourceSave) -> Result<(), NativeModHostError> {
        match (&mut self.parts, save) {
            (HostParts::Classic(_), NativeModSourceSave::Classic { .. }) => self.restore_classic(save),
            (HostParts::Rerelease(_), NativeModSourceSave::Rerelease { .. }) => self.restore_rerelease(save),
            _ => Err(NativeModHostError::invalid("Native mod save ABI differs")),
        }
    }

    fn restore_classic(&mut self, save: &NativeModSourceSave) -> Result<(), NativeModHostError> {
        let NativeModSourceSave::Classic { original } = save else {
            return Err(NativeModHostError::invalid("Native mod save ABI differs"));
        };
        let identity = self.classic_identity()?;
        let HostParts::Classic(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Native classic call reached a rerelease host",
            ));
        };
        let decoded = decode_q2_classic_original_save(original, &identity).map_err(mapped)?;
        let cvars = decode_checkpoint_value(&decoded.server.cvars).map_err(mapped)?;
        let restore_cvars = Rc::clone(&self.restore_cvars);
        restore_cvars(parts.services.cvars_mut(), &cvars)?;
        let short_map = self.short_map.clone();
        let spawn_entities = self.spawn_entities.clone();
        let ClassicHostParts {
            source,
            services,
            files,
            command: _,
        } = parts.as_mut();
        let host = &mut source.host;
        files
            .restore(&decoded, &identity, |game, level| {
                host.save("ReadGame", game, false).map_err(persist)?;
                if let Some(entities) = spawn_entities.as_ref() {
                    host.spawn_entities(&short_map, entities, "").map_err(persist)?;
                }
                services.complete_spawn();
                for (index, value) in &decoded.server.state.configstrings {
                    let index = i32::try_from(*index).map_err(|_| persist("Classic configstring index exceeds i32"))?;
                    services.set_configstring(host, index, value).map_err(persist)?;
                }
                host.save("ReadLevel", level, false).map_err(persist)?;
                Ok(())
            })
            .map_err(mapped)?;
        self.cache_entity_table();
        Ok(())
    }

    fn restore_rerelease(&mut self, save: &NativeModSourceSave) -> Result<(), NativeModHostError> {
        let NativeModSourceSave::Rerelease {
            game,
            level,
            cvars,
            configstrings,
        } = save
        else {
            return Err(NativeModHostError::invalid("Native mod save ABI differs"));
        };
        let HostParts::Rerelease(parts) = &mut self.parts else {
            return Err(NativeModHostError::invalid(
                "Native rerelease call reached a classic host",
            ));
        };
        let decoded = decode_checkpoint_value(cvars).map_err(mapped)?;
        (self.restore_cvars)(parts.services.cvars_mut(), &decoded)?;
        parts
            .source
            .host
            .read_save(&qa_compat::q2::rerelease::host::SourceSave {
                native: game.clone(),
                deferred: Vec::new(),
                projections: Vec::new(),
            })
            .map_err(mapped)?;
        parts.services.complete_spawn();
        let values: std::collections::HashMap<i32, String> = configstrings.iter().cloned().collect();
        parts.services.restore_configstrings(&values).map_err(mapped)?;
        parts
            .source
            .host
            .read_save(&qa_compat::q2::rerelease::host::SourceSave {
                native: level.clone(),
                deferred: Vec::new(),
                projections: Vec::new(),
            })
            .map_err(mapped)?;
        self.cache_entity_table();
        Ok(())
    }

    /// Close the guest.
    pub fn close(&mut self) -> Result<(), NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => parts.source.close().map_err(mapped),
            HostParts::Rerelease(parts) => parts.source.close().map_err(mapped),
        }
    }
}

fn rerelease_client(
    host: &mut RereleaseQ2GuestHost,
    slot: u32,
) -> Result<GuestAddress, qa_compat::q2::rerelease::public_state::PublicStateError> {
    let record = host
        .record_at(slot)
        .map_err(|_| qa_compat::q2::rerelease::public_state::PublicStateError::NoClient)?;
    let mut view = RereleasePublicEdict::new(&mut host.memory, record)?;
    view.client()
}

fn rerelease_gun(host: &mut RereleaseQ2GuestHost, slot: u32) -> Result<(i32, i32), NativeModHostError> {
    let client = rerelease_client(host, slot).map_err(mapped)?;
    let layout = client_layout();
    let gun_index = field_offset(&layout, "ps.gunindex").map_err(mapped)? as i64;
    let gun_frame = field_offset(&layout, "ps.gunframe").map_err(mapped)? as i64;
    let index = host
        .memory
        .read_i32(host.memory.offset(client, gun_index).map_err(mapped)?)
        .map_err(mapped)?;
    let frame = host
        .memory
        .read_i32(host.memory.offset(client, gun_frame).map_err(mapped)?)
        .map_err(mapped)?;
    Ok((index, frame))
}

fn rerelease_gun_index(host: &mut RereleaseQ2GuestHost, slot: u32) -> Result<i32, NativeModHostError> {
    rerelease_gun(host, slot).map(|(index, _)| index)
}

impl qa_compat::q2::native_mod_provider::ProviderHost for NativeModHost {
    fn memory(&mut self) -> &mut SparseGuestMemory {
        self.memory()
    }

    fn image_base(&self) -> GuestAddress {
        self.image_base()
    }

    fn entry(&self, name: &str) -> Option<GuestAddress> {
        self.entry(name).ok()
    }

    fn entities(&self) -> EntityTable {
        *self.entity_table.borrow()
    }

    fn weapon_model(&self, slot: usize) -> u32 {
        u32::try_from(slot)
            .ok()
            .and_then(|slot| self.guns.borrow().get(&slot).copied())
            .unwrap_or(0)
    }

    fn invoke_entry(&mut self, address: GuestAddress, values: &[GuestCallValue]) -> GuestCallResult {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let signature = GuestCallSignature {
                    abi: CLASSIC_Q2_ABI,
                    parameters: Vec::new(),
                    result: None,
                    variadic: true,
                };
                let budget = parts.source.budget();
                match parts.source.host.invoke(address, &signature, values, budget) {
                    Ok(result) => result,
                    Err(error) => panic!("Native classic entry call failed: {error}"),
                }
            }
            HostParts::Rerelease(_) => {
                panic!("Rerelease export calls require the guest module layer")
            }
        }
    }
}

/// Owned per-slot presentation inputs (donor presentation-constructor
/// closures in `native-mod-host.ts`).
pub struct NativeModPresentationSlot {
    /// Entity appearance.
    pub appearance: super::native_mod_presentation::NativeModAppearance,
    /// Entity signature.
    pub signature: String,
    /// Client player state.
    pub player: qa_net::q2_adapters::Q2Player,
    /// Entity presentation state.
    pub state: super::native_mod_presentation::NativeModEntityState,
}

/// Drained presentation inputs (donor presentation-constructor closures).
pub struct NativeModPresentationDrain {
    /// Current configstrings.
    pub configstrings: std::collections::HashMap<i32, String>,
    /// Drained messages.
    pub messages: Vec<super::classic_guest_services::ClassicGuestMessage>,
    /// Presentation clock.
    pub clock: super::native_mod_presentation::NativeQ2PresentationClock,
}

fn vec3(value: &qa_net::q2_adapters::Q2Vec3) -> Vec3 {
    Vec3 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
    }
}

/// Signature over the model indexes and skin (donor `JSON.stringify`
/// of the pair; numbers only, so the manual rendering matches).
fn entity_signature(model_indexes: [u16; 4], skin: i32) -> String {
    format!(
        "[[{},{},{},{}],{}]",
        model_indexes[0], model_indexes[1], model_indexes[2], model_indexes[3], skin
    )
}

impl NativeModHost {
    /// Read the owned presentation inputs for a slot.
    pub fn presentation_slot(&mut self, slot: u32) -> Result<NativeModPresentationSlot, NativeModHostError> {
        match &mut self.parts {
            HostParts::Classic(parts) => {
                let host = &mut parts.source.host;
                let state = ClassicGuestServices::entity_state(host, slot).map_err(mapped)?;
                let record = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| NativeModHostError::invalid("Classic native entities are not bound"))?
                    .at(&mut host.memory, slot)
                    .map_err(mapped)?;
                let inuse = host
                    .memory
                    .read_i32(host.memory.offset(record.address, 88).map_err(mapped)?)
                    .map_err(mapped)?
                    != 0;
                let server_flags = host
                    .memory
                    .read_i32(host.memory.offset(record.address, 184).map_err(mapped)?)
                    .map_err(mapped)?;
                let model = parts.services.model_appearance(host, slot).map_err(mapped)?;
                let player = ClassicGuestServices::player_state(host, slot).map_err(mapped)?;
                Ok(NativeModPresentationSlot {
                    appearance: super::native_mod_presentation::NativeModAppearance {
                        path: model.path,
                        skin: model.skin,
                        skin_path: model.skin_path,
                        attached_models: model.attached_models,
                        frame: state.frame,
                        old_frame: state.frame,
                        effects: state.effects as i32,
                        render_flags: state.render_effects as i32,
                        scale: 1.0,
                        alpha: if state.render_effects & 32 != 0 { 0.3 } else { 1.0 },
                        visible: inuse && server_flags & 1 == 0,
                        origin: vec3(&state.origin),
                        angles: vec3(&state.angles),
                    },
                    signature: entity_signature(state.model_indexes, state.skin),
                    player: qa_net::q2_adapters::Q2Player::Classic(player),
                    state: super::native_mod_presentation::NativeModEntityState {
                        active: inuse,
                        visible: server_flags & 1 == 0,
                        sound: u32::from(state.sound),
                        event: u32::from(state.event),
                        origin: vec3(&state.origin),
                        volume: 1.0,
                        attenuation: 1.0,
                    },
                })
            }
            HostParts::Rerelease(parts) => {
                let state = parts
                    .services
                    .entity_state(&mut parts.source.host, slot)
                    .map_err(mapped)?;
                let info = parts
                    .services
                    .entity_info(&mut parts.source.host, slot)
                    .map_err(mapped)?;
                let model = parts
                    .services
                    .model_appearance(&mut parts.source.host, slot)
                    .map_err(mapped)?;
                let player = parts
                    .services
                    .player_state(&mut parts.source.host, slot)
                    .map_err(mapped)?;
                let effects = i32::try_from(state.effects).map_err(|_| {
                    NativeModHostError::invalid("Native mod effects exceed lossless shared presentation")
                })?;
                let base = &state.base;
                Ok(NativeModPresentationSlot {
                    appearance: super::native_mod_presentation::NativeModAppearance {
                        path: model.path,
                        skin: model.skin,
                        skin_path: model.skin_path,
                        attached_models: model.attached_models,
                        frame: base.frame,
                        old_frame: i32::from(state.old_frame),
                        effects,
                        render_flags: base.render_effects as i32,
                        scale: if state.scale == 0.0 { 1.0 } else { state.scale },
                        alpha: if state.alpha == 0.0 {
                            if base.render_effects & 32 != 0 {
                                0.3
                            } else {
                                1.0
                            }
                        } else {
                            state.alpha
                        },
                        visible: info.active && info.server_flags & 1 == 0,
                        origin: vec3(&base.origin),
                        angles: vec3(&base.angles),
                    },
                    signature: entity_signature(base.model_indexes, base.skin),
                    player: qa_net::q2_adapters::Q2Player::Rerelease(player),
                    state: super::native_mod_presentation::NativeModEntityState {
                        active: info.active,
                        visible: info.server_flags & 1 == 0,
                        sound: u32::from(base.sound),
                        event: u32::from(base.event),
                        origin: vec3(&base.origin),
                        volume: state.loop_volume,
                        attenuation: state.loop_attenuation,
                    },
                })
            }
        }
    }

    /// Drain the shared presentation inputs.
    pub fn presentation_drain(&mut self) -> Result<NativeModPresentationDrain, NativeModHostError> {
        let time = self
            .source_time
            .get()
            .unwrap_or_else(|| seconds(&(self.source_clock)()));
        let clock = super::native_mod_presentation::NativeQ2PresentationClock {
            server_frame: self.source_frame.get() as i32,
            time_milliseconds: (time * 1000.0) as i64,
            frame_time_milliseconds: Some((self.frame_seconds * 1000.0) as i64),
        };
        match &mut self.parts {
            HostParts::Classic(parts) => Ok(NativeModPresentationDrain {
                configstrings: parts.services.configstrings(),
                messages: parts.services.drain_messages(),
                clock,
            }),
            HostParts::Rerelease(parts) => Ok(NativeModPresentationDrain {
                configstrings: parts.services.configstrings(),
                messages: parts
                    .services
                    .drain_messages()
                    .into_iter()
                    .map(|message| message.base)
                    .collect(),
                clock,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_content::q2::support::contracts::{
        Q2BspPlane, Q2SecondaryPlane, Q2SurfaceInfo, Q2TraceFields, TraceContact,
    };
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Vec3;

    use super::*;

    fn trace_fixture(hit: TraceHit) -> TraceResult {
        TraceResult {
            fraction: 0.5,
            end: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            start_solid: false,
            all_solid: true,
            contact: TraceContact::None,
            hit,
            family: TraceFamily::Q2(Q2TraceFields {
                contents: 7,
                surface: Some(Q2SurfaceInfo {
                    name: "rock".to_string(),
                    flags: 9,
                    value: 3,
                    material: "stone".to_string(),
                }),
                source_plane: Q2BspPlane {
                    normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                    distance: 4.0,
                    plane_type: 2,
                    signbits: 1,
                },
                secondary: Some(Q2SecondaryPlane {
                    plane: Q2BspPlane {
                        normal: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                        distance: 5.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                    surface: None,
                }),
            }),
        }
    }

    #[test]
    fn short_map_strips_prefix_and_suffix() {
        assert_eq!(short_map_name("maps/q2dm1.bsp"), "q2dm1");
        assert_eq!(short_map_name("q2dm1.bsp"), "q2dm1");
        assert_eq!(short_map_name("maps/q2dm1"), "q2dm1");
        assert_eq!(short_map_name("q2dm1"), "q2dm1");
    }

    #[test]
    fn frame_seconds_prefers_declaration() {
        assert_eq!(resolve_frame_seconds(Some(0.05), false, 100.0), 0.05);
        assert_eq!(resolve_frame_seconds(None, false, 100.0), 0.1);
        assert_eq!(resolve_frame_seconds(None, true, 50.0), 0.05);
    }

    #[test]
    fn max_clients_prefers_declaration() {
        assert_eq!(resolve_max_clients(Some(8), 4).expect("clients"), 8);
        assert_eq!(resolve_max_clients(None, 4).expect("clients"), 4);
        assert!(resolve_max_clients(Some(u64::from(u32::MAX) + 1), 4).is_err());
    }

    #[test]
    fn sv_detection_folds_case() {
        assert!(is_sv_command(&["sv".to_string()]));
        assert!(is_sv_command(&["SV".to_string(), "x".to_string()]));
        assert!(!is_sv_command(&[]));
        assert!(!is_sv_command(&["say".to_string()]));
    }

    #[test]
    fn stance_height_validates_range() {
        assert_eq!(validate_stance_height(24.9).expect("height"), 24);
        assert_eq!(validate_stance_height(-128.0).expect("height"), -128);
        assert!(validate_stance_height(128.0).is_err());
        assert!(validate_stance_height(f64::NAN).is_err());
        assert!(validate_stance_height(f64::INFINITY).is_err());
    }

    #[test]
    fn game_signatures_resolve_or_reject() {
        assert!(classic_game_signature("Init").is_ok());
        assert!(classic_game_signature("Nope").is_err());
        assert!(rerelease_game_signature("SpawnEntities").is_ok());
        assert!(rerelease_game_signature("Nope").is_err());
    }

    #[test]
    fn classic_trace_lowers_hit_and_surface() {
        let owner = IdentityOwner::create("trace").expect("owner");
        let actor = owner.actor(3, 1);
        let trace = trace_fixture(TraceHit::Actor { actor: actor.clone() });
        let input = classic_trace_input(&trace, &|hit| (hit == &actor).then_some(9)).expect("trace");
        assert!(input.all_solid);
        assert_eq!(input.fraction, 0.5);
        assert_eq!(input.hit, TraceHitSlot::Slot(9));
        assert_eq!(input.contents, 7);
        assert_eq!(input.plane_type, 2);
        let surface = input.surface.expect("surface");
        assert_eq!(surface.name, "rock");
        assert_eq!(surface.flags, 9);
        let missing = classic_trace_input(&trace, &|_| None);
        assert!(missing.is_err());
        let world = classic_trace_input(&trace_fixture(TraceHit::World { model: 0 }), &|_| None).expect("world");
        assert_eq!(world.hit, TraceHitSlot::World);
    }

    #[test]
    fn trace_lowering_rejects_non_q2() {
        let mut trace = trace_fixture(TraceHit::None);
        trace.family = TraceFamily::Q1 {
            in_open: true,
            in_water: false,
            source_plane: qa_core::math::Plane {
                normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                distance: 0.0,
            },
            surface_flags: None,
        };
        assert!(classic_trace_input(&trace, &|_| None).is_err());
        assert!(rerelease_trace_input(&trace, &|_| None).is_err());
    }

    #[test]
    fn rerelease_trace_lowers_secondary() {
        let trace = trace_fixture(TraceHit::None);
        let input = rerelease_trace_input(&trace, &|_| None).expect("trace");
        assert!(matches!(input.hit, RereleaseTraceHit::World));
        assert_eq!(input.contents, 7);
        let surface = input.surface.expect("surface");
        assert_eq!(surface.material, "stone");
        let (normal, distance, _, _, surface) = input.secondary.expect("secondary");
        assert_eq!(normal.x, 1.0);
        assert_eq!(distance, 5.0);
        assert!(surface.is_none());
    }

    #[test]
    fn command_cell_nests_and_restores() {
        let cell = NativeModCommandCell::new();
        let outer = GuestCommandInvocation::new(vec!["sv".to_string(), "a".to_string()], "a".to_string(), None);
        let inner = GuestCommandInvocation::new(vec!["sv".to_string()], String::new(), None);
        cell.with(&outer, || {
            assert_eq!(cell.reader()().arguments, vec!["sv".to_string(), "a".to_string()]);
            cell.with(&inner, || {
                assert_eq!(cell.reader()().arguments, vec!["sv".to_string()]);
            })
            .expect("inner");
            assert_eq!(cell.reader()().args, "a");
        })
        .expect("outer");
        assert!(cell.reader()().arguments.is_empty());
        let stale = GuestCommandInvocation::new(Vec::new(), String::new(), None);
        stale.deactivate();
        assert!(cell.with(&stale, || {}).is_err());
    }

    #[test]
    fn shared_projection_installs_binding() {
        let shared = SharedProjection::default();
        assert!(!shared.accepts_client(1));
        let payer = IdentityOwner::create("projection").expect("owner");
        let actor = payer.actor(1, 1);
        struct Bound(ActorId);
        impl NativeModProjection for Bound {
            fn project(&self, _record: &RawEntityView) -> Option<OwnedActor> {
                None
            }

            fn actor_at(&self, _slot: u32) -> Option<ActorId> {
                Some(self.0.clone())
            }

            fn slot_of(&self, _actor: &ActorId) -> Option<u32> {
                Some(1)
            }

            fn accepts_client(&self, _slot: u32) -> bool {
                true
            }

            fn address(&self, _actor: &ActorId) -> Option<GuestAddress> {
                None
            }

            fn import_boundary(
                &self,
                _name: &str,
                _values: &[GuestCallValue],
                invoke: &dyn Fn() -> GuestCallResult,
            ) -> GuestCallResult {
                invoke()
            }
        }
        let handle = shared.handle();
        shared.set(Rc::new(Bound(actor.clone())));
        assert!(handle.accepts_client(1));
        assert_eq!(handle.actor_at(7), Some(actor));
    }

    #[test]
    fn empty_projection_calls_through() {
        let empty = EmptyProjection;
        let record = RawEntityView {
            slot: 1,
            address: GuestAddress { space: 0, offset: 0 },
            stride_bytes: 0,
        };
        assert!(empty.project(&record).is_none());
        let result = empty.import_boundary("trace", &[], &|| GuestCallResult::Void);
        assert!(matches!(result, GuestCallResult::Void));
    }

    #[test]
    fn entity_signature_renders_pair() {
        assert_eq!(entity_signature([1, 2, 3, 4], 7), "[[1,2,3,4],7]");
    }

    #[test]
    fn save_edition_tags_match() {
        let classic = NativeModSourceSave::Classic {
            original: ProviderCheckpoint {
                provider: "p".to_string(),
                schema: "s".to_string(),
                version: 1,
                bytes: Vec::new(),
            },
        };
        assert_eq!(classic.edition(), "classic");
        let rerelease = NativeModSourceSave::Rerelease {
            game: Vec::new(),
            level: Vec::new(),
            cvars: Vec::new(),
            configstrings: Vec::new(),
        };
        assert_eq!(rerelease.edition(), "rerelease");
    }
}
