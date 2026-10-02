//! Q3 application server host binding.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/network-q3.ts`
//! (`Q3ApplicationServerBindingOptions`, `Q3ApplicationServerAuthority`,
//! `createQ3ApplicationServerHost`).
//!
//! The donor is synchronous except for guest/package awaits; this port is
//! fully synchronous (guest and package siblings expose sync host seams).
//!
//! # Missing siblings
//!
//! - `simulation/runtime.ts` (`SharedSimulation`): [`Q3HostSimulation`],
//!   [`Q3HostScene`].
//! - `simulation/q3/runtime.ts` (`Q3SourceRuntime`, `Q3ClientAdmissionDenied`):
//!   [`Q3HostNativeSource`]; the denial maps to
//!   [`Q3HostError::AdmissionDenied`].
//! - `app/bootstrap/network/q3-types.ts`: the host shape is implemented as
//!   inherent methods here; [`Q3ApplicationPlayer`] and
//!   [`Q3ApplicationAdmission`] are reused from `q3::guest_runtime` (unify
//!   post-merge); `administration` stays an opaque generic.
//! - `app/bootstrap/network/q3-downloads.ts` (`Q3ApplicationPackages`):
//!   [`Q3HostPackages`], opened through [`Q3OpenPackagesFn`].
//! - `app/bootstrap/content.ts` (`LoadedApplicationContent`):
//!   [`Q3HostContent`].
//! - `world/session/session.ts` (`EngineSession`): [`Q3HostSession`].
//! - `network/q3/adapters.ts` (`toQ3UserCommand`, `fromQ3PlayerState`,
//!   `Q3_PROTOCOL`): [`to_q3_user_command`], [`from_qvm_player_state`],
//!   [`Q3_PROTOCOL_VERSION`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, set_info_value, CvarRegistry, InfoOptions, InfoTarget};
use qa_core::identity::{ActorId, ClientId};
use qa_core::math::{Bounds, Vec3};
use qa_guest::qvm::entity_record::{QvmEntityState, QvmTrajectory};
use qa_guest::qvm::player_record::QvmPlayerState;
use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand};
use qa_net::common::endpoint::NetworkAddress;
use qa_net::q3_net::{
    Gamestate, GamestateEntry, Q3EntityState, Q3NetError, Q3PlayerSlots, Q3PlayerState, Q3Product, Q3Trajectory,
};
use qa_net::q3_visibility::{select_q3_snapshot_entities, Q3VisibilityBindings, Q3VisibilityEntity};
use qa_net::q3_visibility::{Q3VisibilityLink, Q3VisibilityWorld};
use thiserror::Error;

use super::q3::guest_runtime::{Q3ApplicationAdmission, Q3ApplicationPlayer, Q3QvmServerGame};
use super::q3::server_state::{Q3ServerState, Q3StoredUserCommand};

/// Q3 wire protocol version (donor `Q3_PROTOCOL.version`).
pub const Q3_PROTOCOL_VERSION: u32 = 68;

/// Maximum Q3 configstrings scanned for gamestate entries.
const Q3_CONFIGSTRING_COUNT: i32 = 1024;

/// Maximum native entity leaves resolved per snapshot link.
const Q3_LINK_LEAF_CAP: usize = 128;

/// Maximum status response player rows in bytes.
const Q3_STATUS_ROW_BUDGET: usize = 16384;

/// Print sink, mirroring donor `print`.
pub type Q3PrintFn = Rc<dyn Fn(&str)>;

/// Package-set factory, mirroring donor `Q3ApplicationPackages.open`.
pub type Q3OpenPackagesFn<C, P> = Rc<dyn Fn(&C, u32) -> Result<P, Q3HostError>>;

/// Configstring broadcast callback (donor `prepare` `configstring?` argument).
pub type Q3ConfigstringFn<'a> = &'a mut dyn FnMut(i32, &str);

/// Q3 server bring-up mode (donor `mode?: 'new' | 'restore'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q3ServerMode {
    /// Fresh server id and configstrings.
    #[default]
    New,
    /// Retain the saved server id.
    Restore,
}

/// Q3 source round binding.
#[derive(Clone)]
enum Q3SourceBinding {
    /// Saved QVM guest source.
    Guest(Rc<Q3QvmServerGame>),
    /// Native source round (rebindable).
    Native(Rc<dyn Q3HostNativeSource>),
}

/// Host failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3HostError {
    /// No Q3 provider is bound (donor `Q3 network requires a Q3 game provider`).
    #[error("Q3 network requires a Q3 game provider")]
    MissingProvider,
    /// Native round binding is unavailable.
    #[error("Native source round binding is unavailable")]
    NativeRoundUnavailable,
    /// Restore mode requires a saved QVM source.
    #[error("Restored Q3 client authority requires a saved QVM source")]
    RestoreRequiresGuest,
    /// Package metadata has not been prepared.
    #[error("Q3 package metadata has not been prepared")]
    PackagesNotPrepared,
    /// World changed the checksum feed without replacing the content host.
    #[error("Q3 world changed checksum feed without replacing content host")]
    ChecksumFeedChanged,
    /// Saved server id mismatch in restore mode.
    #[error("Restored Q3 client authority must retain the saved server id")]
    SavedServerIdMismatch,
    /// Client admission denied (donor `Q3ClientAdmissionDenied`).
    #[error("{0}")]
    AdmissionDenied(String),
    /// Client was never admitted.
    #[error("Application has not admitted the Q3 client")]
    ClientNotAdmitted,
    /// Native player record disappeared.
    #[error("Q3 snapshot player disappeared")]
    SnapshotPlayerGone,
    /// Fast restart plan is incompatible.
    #[error("Q3 network fast restart is incompatible: {0}")]
    RestartIncompatible(String),
    /// Source round is already retired.
    #[error("Q3 network source round is already retired")]
    SourceRoundRetired,
    /// Rebind target is not a fresh round with retained state and product.
    #[error("Q3 network requires a new round with retained server state and product")]
    RebindRejected,
    /// Guest-only operation on a native source.
    #[error("Q3 guest-only operation is unavailable: {0}")]
    GuestOnly(&'static str),
    /// Native-only operation on a guest source.
    #[error("Q3 native-only operation is unavailable: {0}")]
    NativeOnly(&'static str),
    /// Session failure.
    #[error("Q3 session: {0}")]
    Session(String),
    /// Content failure.
    #[error("Q3 content: {0}")]
    Content(String),
    /// Package failure.
    #[error("Q3 packages: {0}")]
    Packages(String),
    /// Guest failure.
    #[error("Q3 guest: {0}")]
    Guest(String),
    /// Cvar failure.
    #[error("Q3 cvar: {0}")]
    Cvar(String),
    /// Snapshot failure.
    #[error("Q3 snapshot: {0}")]
    Snapshot(String),
}

impl From<Q3NetError> for Q3HostError {
    fn from(value: Q3NetError) -> Self {
        Self::Snapshot(format!("{value:?}"))
    }
}

/// Native entity link flags (donor `pool.at(number).r` surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3NativeEntityFlags {
    /// Whether the entity is linked.
    pub linked: bool,
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
}

/// Native player client record (donor `records.byActor(actor).client` surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3NativeClient {
    /// Wire player state (the `Q3SourceRuntime` sibling owns the native
    /// `ps` to wire-record conversion).
    pub player_state: Q3PlayerState,
    /// Persistent net name (donor `client.pers.netname`).
    pub netname: String,
}

/// Native record binding (donor `records.byActor(actor)` surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3NativeRecord {
    /// Client slot.
    pub slot: i32,
    /// Player client, when spawned.
    pub client: Option<Q3NativeClient>,
}

/// Native Q3 source round seam (donor `Q3SourceRuntime` used surface).
pub trait Q3HostNativeSource: std::fmt::Debug {
    /// Donor `source.options.product`.
    fn product(&self) -> String;
    /// Donor `source.host.serverState`.
    fn server_state(&self) -> Rc<Q3ServerState>;
    /// Donor `source.host.now()`.
    fn now_ms(&self) -> i32;
    /// Donor `source.host.engine.getUserinfo(slot)`.
    fn engine_userinfo(&self, slot: i32) -> String;
    /// Donor `source.pool.numEntities`.
    fn num_entities(&self) -> i32;
    /// Wire copy of `source.pool.at(number).s`.
    fn entity_wire_state(&self, number: i32) -> Q3EntityState;
    /// Donor `source.pool.at(number)` link flags (`inuse && r.linked` folds
    /// into [`Q3NativeEntityFlags::linked`]).
    fn entity_flags(&self, number: i32) -> Q3NativeEntityFlags;
    /// Donor `source.pool.at(number).actor.id`.
    fn entity_actor(&self, number: i32) -> Option<ActorId>;
    /// Donor `source.records.byActor(actor)`.
    fn record_for_actor(&self, actor: &ActorId) -> Option<Q3NativeRecord>;
    /// Donor `source.playerCommand(actor, name, args)`.
    fn player_command(&self, actor: &ActorId, name: &str, args: &[String]);
    /// Donor `source.admission.userinfoChanged(slot)`.
    fn userinfo_changed(&self, slot: i32);
}

/// Collision scene seam (donor `simulation.scene` used surface).
pub trait Q3HostScene: std::fmt::Debug {
    /// Donor `scene.boxLeaves(bounds, cap).leaves`.
    fn box_leaves(&self, bounds: &Bounds, cap: usize) -> Vec<i32>;
    /// Donor `scene.leafArea(leaf)`.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Donor `scene.leafCluster(leaf)`.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Donor `scene.pointLeaf(point)`.
    fn point_leaf(&self, point: Vec3) -> i32;
    /// Donor `scene.areasConnected(a, b)`.
    fn areas_connected(&self, first: i32, second: i32) -> bool;
    /// Donor `scene.areaBits(area)`.
    fn area_bits(&self, area: i32) -> Vec<u8>;
    /// Donor `scene.clusterVisible(cluster, leaf, 'pvs')`.
    fn cluster_visible(&self, cluster: i32, leaf: i32) -> bool;
}

/// Source restart plan (donor `simulation.sourceRestartPlan()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3SourceRestartPlan {
    /// Fast restart is compatible.
    SourceReset,
    /// Fast restart is incompatible with a reason.
    Other {
        /// Plan reason.
        reason: String,
    },
}

/// Shared simulation seam (donor `SharedSimulation` used surface).
pub trait Q3HostSimulation: std::fmt::Debug {
    /// Donor `simulation.q3Guest()`.
    fn q3_guest(&self) -> Option<Rc<Q3QvmServerGame>>;
    /// Donor `simulation.q3Source()`.
    fn q3_source(&self) -> Option<Rc<dyn Q3HostNativeSource>>;
    /// Donor `simulation.options.maxClients`.
    fn max_clients(&self) -> i32;
    /// Donor `simulation.players()`.
    fn players(&self) -> Vec<ActorId>;
    /// Donor `simulation.movementPlayer(actor)?.client`.
    fn movement_client(&self, actor: &ActorId) -> Option<ClientId>;
    /// Donor `simulation.admitPlayer(client)`; denials surface as
    /// [`Q3HostError::AdmissionDenied`].
    fn admit_player(&mut self, client: &ClientId) -> Result<(), Q3HostError>;
    /// Donor `simulation.disconnectPlayer(actor)`.
    fn disconnect_player(&mut self, actor: &ActorId);
    /// Donor `simulation.sourceRestartPlan()`.
    fn source_restart_plan(&self) -> Q3SourceRestartPlan;
    /// Donor `simulation.timeSeconds`.
    fn time_seconds(&self) -> f64;
    /// Donor `simulation.recipe.movement.provider`.
    fn movement_provider(&self) -> String;
    /// Donor `simulation.recipe.character.definition.provider`.
    fn character_provider(&self) -> String;
    /// Donor `simulation.observeClientCommand(command)`.
    fn observe_client_command(&mut self, command: ActorCommand);
    /// Donor `simulation.notifyClientEvent(kind, actor)`.
    fn notify_client_event(&mut self, kind: &str, actor: &ActorId);
    /// Donor `simulation.scene`.
    fn scene(&self) -> Rc<dyn Q3HostScene>;
    /// Donor `simulation.bodies.linked(actor)?.absoluteBounds`.
    fn body_bounds(&self, actor: &ActorId) -> Option<Bounds>;
}

/// Client connection origin (donor `client.connect('loopback' | 'remote')`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ClientOrigin {
    /// Loopback connection.
    Loopback,
    /// Remote connection.
    Remote,
}

/// Engine session seam (donor `EngineSession` used surface).
pub trait Q3HostSession: std::fmt::Debug {
    /// Donor `session.createClient(slot)`.
    fn create_client(&mut self, slot: u32) -> Result<ClientId, Q3HostError>;
    /// Donor `client.connect(origin)`.
    fn connect_client(&mut self, client: &ClientId, origin: Q3ClientOrigin);
    /// Donor `session.closeClient(client)`.
    fn close_client(&mut self, client: &ClientId);
}

/// Loaded content seam (donor `LoadedApplicationContent` used surface).
pub trait Q3HostContent: std::fmt::Debug {
    /// Donor `options.content.q3Product?.restriction.kind === 'demo'`.
    fn q3_demo_restricted(&self) -> bool;
    /// Donor `options.content.mounts.resolve(path)`.
    fn resolve_mount(&self, path: &str) -> Result<(), Q3HostError>;
}

/// Package metadata seam (donor `Q3ApplicationPackages` used surface).
pub trait Q3HostPackages: std::fmt::Debug {
    /// Download handle (donor `Q3DownloadReadFile`).
    type Download;
    /// Donor `packages.references.references.checksumFeed`.
    fn checksum_feed(&self) -> u32;
    /// Donor `references.loadedPakChecksums()`.
    fn loaded_pak_checksums(&self) -> String;
    /// Donor `references.loadedPakNames()`.
    fn loaded_pak_names(&self) -> String;
    /// Donor `references.referencedPakChecksums()`.
    fn referenced_pak_checksums(&self) -> String;
    /// Donor `references.referencedPakNames()`.
    fn referenced_pak_names(&self) -> String;
    /// Donor `packages.collect(content)`.
    fn collect(&mut self, content: &dyn Q3HostContent);
    /// Donor `packages.pureChecksum(path)`.
    fn pure_checksum(&self, path: &str) -> i32;
    /// Donor `packages.packs.map(pack => pack.pack.pureChecksum)`.
    fn pack_pure_checksums(&self) -> Vec<i32>;
    /// Donor `packages.openDownload(name)`.
    fn open_download(&self, name: &str) -> Option<Self::Download>;
}

/// Client admission request (donor `admit` request surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3AdmitRequest {
    /// Client slot.
    pub slot: u32,
    /// Remote address.
    pub address: NetworkAddress,
    /// Client userinfo string.
    pub userinfo: String,
}

/// Client rate contract (donor `rate(player)` surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3RateInfo {
    /// Requested rate bytes per second.
    pub rate: i32,
    /// Maximum rate bytes per second (`sv_maxRate`).
    pub max_rate: f32,
    /// Snapshot period in milliseconds.
    pub snapshot_msec: i32,
    /// Whether the client connects over loopback.
    pub local: bool,
    /// Forced LAN flag (donor always `false`).
    pub force_lan: bool,
    /// LAN flag (donor always `false`).
    pub lan: bool,
}

/// Pure-server contract (donor `pure(serverId)` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PureInfo {
    /// Whether pure mode is enabled (`sv_pure`).
    pub enabled: bool,
    /// Checksum feed.
    pub checksum_feed: i32,
    /// Checksum feed server id.
    pub checksum_feed_server_id: i32,
    /// `vm/cgame.qvm` pure checksum.
    pub cgame_checksum: i32,
    /// `vm/ui.qvm` pure checksum.
    pub ui_checksum: i32,
    /// Loaded pak pure checksums.
    pub loaded_pure_checksums: Vec<i32>,
}

/// Player snapshot (donor `snapshot(player)` surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3HostSnapshot {
    /// Snapshot player state.
    pub player: Q3PlayerState,
    /// Area visibility mask.
    pub area_mask: Vec<u8>,
    /// Visible entity states.
    pub entities: Vec<Q3EntityState>,
}

/// Native wire support (donor `supportsSourceWire()` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3WireSupport {
    /// Native wire is supported.
    Supported,
    /// Native wire is unsupported with reasons.
    Unsupported {
        /// Unsupported reasons.
        reasons: Vec<String>,
    },
}

/// Map a stored wire command to a movement command (donor `toQ3UserCommand`).
#[must_use]
pub fn to_q3_user_command(value: &Q3StoredUserCommand) -> UserCommand {
    UserCommand::Q3 {
        server_time_milliseconds: f64::from(value.server_time),
        angle_words: [
            f64::from(value.angles[0]),
            f64::from(value.angles[1]),
            f64::from(value.angles[2]),
        ],
        buttons: f64::from(value.buttons),
        weapon: f64::from(value.weapon),
        forward_move: f64::from(value.forwardmove),
        right_move: f64::from(value.rightmove),
        up_move: f64::from(value.upmove),
    }
}

/// Map a product tag to the wire product.
fn q3_product_from_str(product: &str) -> Q3Product {
    if product == "missionpack" {
        Q3Product::MissionPack
    } else {
        Q3Product::Base
    }
}

fn vec3_to_array(value: Vec3) -> [f32; 3] {
    [value.x, value.y, value.z]
}

fn qvm_trajectory_to_wire(value: &QvmTrajectory) -> Q3Trajectory {
    Q3Trajectory {
        trajectory_type: value.trajectory_type,
        time: value.time,
        duration: value.duration,
        base: vec3_to_array(value.base),
        delta: vec3_to_array(value.delta),
    }
}

/// Copy a guest entity state to a wire record (donor `wireEntity`).
#[must_use]
pub fn qvm_entity_state_to_wire(value: &QvmEntityState) -> Q3EntityState {
    Q3EntityState {
        number: value.number,
        e_type: value.e_type,
        e_flags: value.e_flags,
        pos: qvm_trajectory_to_wire(&value.pos),
        apos: qvm_trajectory_to_wire(&value.apos),
        time: value.time,
        time2: value.time2,
        origin: vec3_to_array(value.origin),
        origin2: vec3_to_array(value.origin2),
        angles: vec3_to_array(value.angles),
        angles2: vec3_to_array(value.angles2),
        other_entity_num: value.other_entity_num,
        other_entity_num2: value.other_entity_num2,
        ground_entity_num: value.ground_entity_num,
        constant_light: value.constant_light,
        loop_sound: value.loop_sound,
        modelindex: value.modelindex,
        modelindex2: value.modelindex2,
        client_num: value.client_num,
        frame: value.frame,
        solid: value.solid,
        event: value.event,
        event_parm: value.event_parm,
        powerups: value.powerups,
        weapon: value.weapon,
        legs_anim: value.legs_anim,
        torso_anim: value.torso_anim,
        generic1: value.generic1,
    }
}

fn qvm_slots_to_wire(values: &[i32; 16]) -> Q3PlayerSlots {
    let mut slots = Q3PlayerSlots::new();
    for (index, value) in values.iter().enumerate() {
        let _ = slots.set(index, *value);
    }
    slots
}

/// Map a guest player state to a wire record (donor `fromQ3PlayerState`).
#[must_use]
pub fn from_qvm_player_state(value: &QvmPlayerState, product: &str) -> Q3PlayerState {
    Q3PlayerState {
        product: q3_product_from_str(product),
        command_time: value.command_time_ms,
        pm_type: value.movement_type,
        bob_cycle: value.bob_cycle,
        pm_flags: value.movement_flags,
        pm_time: value.movement_time_ms,
        origin: vec3_to_array(value.origin),
        velocity: vec3_to_array(value.velocity),
        weapon_time: value.weapon_time_ms,
        gravity: value.gravity,
        speed: value.speed,
        delta_angles: [
            value.delta_angle_words[0] as f32,
            value.delta_angle_words[1] as f32,
            value.delta_angle_words[2] as f32,
        ],
        ground_entity_num: value.ground_entity_number,
        legs_timer: value.legs_timer_ms,
        legs_anim: value.legs_animation,
        torso_timer: value.torso_timer_ms,
        torso_anim: value.torso_animation,
        movement_dir: value.movement_direction,
        grapple_point: vec3_to_array(value.grapple_point),
        e_flags: value.flags,
        event_sequence: value.event_sequence,
        events: [value.events[0], value.events[1]],
        event_parms: [value.event_parameters[0], value.event_parameters[1]],
        external_event: value.external_event,
        external_event_parm: value.external_event_parameter,
        external_event_time: value.external_event_time_ms,
        client_num: value.client_number,
        weapon: value.weapon,
        weapon_state: value.weapon_state,
        viewangles: vec3_to_array(value.view_angles),
        viewheight: value.view_height,
        damage_event: value.damage_event,
        damage_yaw: value.damage_yaw,
        damage_pitch: value.damage_pitch,
        damage_count: value.damage_count,
        stats: qvm_slots_to_wire(&value.stats),
        persistant: qvm_slots_to_wire(&value.persistent),
        powerups: qvm_slots_to_wire(&value.powerups),
        ammo: qvm_slots_to_wire(&value.ammo),
        generic1: value.generic1,
        loop_sound: value.loop_sound,
        jumppad_ent: value.jump_pad_entity,
        ping: value.ping_ms,
        pmove_framecount: value.movement_frame_count,
        jumppad_frame: value.jump_pad_frame,
        entity_event_sequence: value.entity_event_sequence,
    }
}

/// Parse a `Number.parseInt(value, 10)`-style integer prefix: leading ASCII
/// whitespace, an optional sign, then a maximal digit run.
fn parse_info_int(value: &str) -> Option<i32> {
    let trimmed = value.trim_start();
    let (sign, digits) = match trimmed.strip_prefix(['+', '-']) {
        Some(rest) => (trimmed.starts_with('-'), rest),
        None => (false, trimmed),
    };
    let run: String = digits.chars().take_while(char::is_ascii_digit).collect();
    if run.is_empty() {
        return None;
    }
    let mut parsed: i32 = run.parse().ok()?;
    if sign {
        parsed = parsed.saturating_neg();
    }
    Some(parsed)
}

/// Server host binding options (donor `Q3ApplicationServerBindingOptions`).
#[derive(Clone)]
pub struct Q3ApplicationServerBindingOptions<S, E, C, P, A> {
    /// Engine session.
    pub session: E,
    /// Shared simulation.
    pub simulation: S,
    /// Loaded content.
    pub content: C,
    /// Bring-up mode.
    pub mode: Q3ServerMode,
    /// Network-lane administration handle (opaque passthrough).
    pub administration: Option<A>,
    /// Package-set factory.
    pub open_packages: Q3OpenPackagesFn<C, P>,
    /// Print sink.
    pub print: Q3PrintFn,
}

/// Q3 application server host (donor `Q3ApplicationServerAuthority`).
pub struct Q3ApplicationServerHost<S, E, C, P, A> {
    session: E,
    simulation: S,
    content: C,
    mode: Q3ServerMode,
    administration: Option<A>,
    open_packages: Q3OpenPackagesFn<C, P>,
    print: Q3PrintFn,
    binding: Q3SourceBinding,
    state: Rc<Q3ServerState>,
    product: String,
    max_clients: i32,
    packages: Option<P>,
    touched_cgame: bool,
}

/// Create the Q3 application server host (donor
/// `createQ3ApplicationServerHost`).
pub fn create_q3_application_server_host<S, E, C, P, A>(
    options: Q3ApplicationServerBindingOptions<S, E, C, P, A>,
) -> Result<Q3ApplicationServerHost<S, E, C, P, A>, Q3HostError>
where
    S: Q3HostSimulation,
    E: Q3HostSession,
    C: Q3HostContent,
    P: Q3HostPackages,
{
    let simulation = options.simulation;
    let guest = simulation.q3_guest();
    let native = simulation.q3_source();
    let binding = match (guest, native) {
        (Some(guest), _) => Q3SourceBinding::Guest(guest),
        (None, Some(native)) => Q3SourceBinding::Native(native),
        (None, None) => return Err(Q3HostError::MissingProvider),
    };
    let is_guest = matches!(binding, Q3SourceBinding::Guest(_));
    if options.mode == Q3ServerMode::Restore && !is_guest {
        return Err(Q3HostError::RestoreRequiresGuest);
    }
    let state = match &binding {
        Q3SourceBinding::Guest(guest) => Rc::clone(&guest.state),
        Q3SourceBinding::Native(native) => native.server_state(),
    };
    let product = match &binding {
        Q3SourceBinding::Guest(_) => "baseq3".to_string(),
        Q3SourceBinding::Native(native) => native.product(),
    };
    if options.mode == Q3ServerMode::New {
        let fs_game = if is_guest {
            state.cvars.borrow().variable_string("fs_game")
        } else if product == "missionpack" {
            "missionpack".to_string()
        } else {
            String::new()
        };
        state
            .cvars
            .borrow_mut()
            .register("fs_game", &fs_game, flags::SYSTEM_INFO)
            .map_err(|error| Q3HostError::Cvar(format!("{error:?}")))?;
    }
    let max_clients = simulation.max_clients();
    Ok(Q3ApplicationServerHost {
        session: options.session,
        simulation,
        content: options.content,
        mode: options.mode,
        administration: options.administration,
        open_packages: options.open_packages,
        print: options.print,
        binding,
        state,
        product,
        max_clients,
        packages: None,
        touched_cgame: false,
    })
}

impl<S, E, C, P, A> Q3ApplicationServerHost<S, E, C, P, A>
where
    S: Q3HostSimulation,
    E: Q3HostSession,
    C: Q3HostContent,
    P: Q3HostPackages,
{
    /// Whether the host drives a saved guest source.
    fn is_guest(&self) -> bool {
        matches!(self.binding, Q3SourceBinding::Guest(_))
    }

    fn guest(&self) -> Result<Rc<Q3QvmServerGame>, Q3HostError> {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => Ok(Rc::clone(guest)),
            Q3SourceBinding::Native(_) => Err(Q3HostError::GuestOnly("guest source")),
        }
    }

    fn native(&self) -> Result<Rc<dyn Q3HostNativeSource>, Q3HostError> {
        match &self.binding {
            Q3SourceBinding::Guest(_) => Err(Q3HostError::NativeRoundUnavailable),
            Q3SourceBinding::Native(native) => Ok(Rc::clone(native)),
        }
    }

    fn cvars(&self) -> std::cell::Ref<'_, CvarRegistry> {
        self.state.cvars.borrow()
    }

    fn set_cvar(&self, name: &str, value: &str) -> Result<(), Q3HostError> {
        self.state
            .cvars
            .borrow_mut()
            .set(name, value, true)
            .map_err(|error| Q3HostError::Cvar(format!("{error:?}")))?;
        Ok(())
    }

    /// Donor `product`.
    #[must_use]
    pub fn product(&self) -> &str {
        &self.product
    }

    /// Donor `maxClients`.
    #[must_use]
    pub fn max_clients(&self) -> i32 {
        self.max_clients
    }

    /// Donor `administration` passthrough.
    #[must_use]
    pub fn administration(&self) -> Option<&A> {
        self.administration.as_ref()
    }

    /// Donor `print`.
    pub fn print(&self, text: &str) {
        (self.print)(text);
    }

    /// Donor `admission.privateClients`.
    #[must_use]
    pub fn admission_private_clients(&self) -> i32 {
        self.cvars()
            .get("sv_privateClients")
            .map_or(0, |snapshot| snapshot.integer_value)
    }

    /// Donor `admission.privatePassword`.
    #[must_use]
    pub fn admission_private_password(&self) -> String {
        self.cvars().variable_string("sv_privatePassword")
    }

    /// Donor `admission.reconnectLimitSeconds`.
    #[must_use]
    pub fn admission_reconnect_limit_seconds(&self) -> i32 {
        self.cvars()
            .get("sv_reconnectlimit")
            .map_or(3, |snapshot| snapshot.integer_value)
    }

    /// Donor `admission.minimumPing`.
    #[must_use]
    pub fn admission_minimum_ping(&self) -> f32 {
        self.cvars().variable_value("sv_minPing")
    }

    /// Donor `admission.maximumPing`.
    #[must_use]
    pub fn admission_maximum_ping(&self) -> f32 {
        self.cvars().variable_value("sv_maxPing")
    }

    /// Donor `admission.demoRestricted`.
    #[must_use]
    pub fn admission_demo_restricted(&self) -> bool {
        self.content.q3_demo_restricted()
    }

    /// Donor `admission.enabled`.
    #[must_use]
    pub fn admission_enabled(&self) -> bool {
        self.cvars().variable_value("g_gametype") != 2.0 && self.cvars().variable_value("ui_singlePlayerActive") == 0.0
    }

    /// Donor `admission.gameDirectory`.
    #[must_use]
    pub fn admission_game_directory(&self) -> String {
        self.cvars().variable_string("fs_game")
    }

    /// Donor `admission.strictAuth`.
    #[must_use]
    pub fn admission_strict_auth(&self) -> String {
        self.cvars().variable_string("sv_strictAuth")
    }

    /// Donor `admission.floodProtect`.
    #[must_use]
    pub fn admission_flood_protect(&self) -> bool {
        self.cvars().variable_value("sv_floodProtect") != 0.0
    }

    fn require_saved_server_id(&self, server_id: i32) -> Result<(), Q3HostError> {
        if self.mode == Q3ServerMode::Restore
            && f64::from(server_id) != f64::from(self.cvars().variable_value("sv_serverid"))
        {
            return Err(Q3HostError::SavedServerIdMismatch);
        }
        Ok(())
    }

    fn actor_for_client(&self, client: &ClientId) -> Option<ActorId> {
        self.simulation
            .players()
            .into_iter()
            .find(|actor| self.simulation.movement_client(actor).as_ref() == Some(client))
    }

    /// Donor `playerFor(client)`.
    pub fn carried_player(&self, client: &ClientId) -> Result<Q3ApplicationPlayer, Q3HostError> {
        if let Q3SourceBinding::Guest(guest) = &self.binding {
            return guest.player(client).ok_or(Q3HostError::ClientNotAdmitted);
        }
        let native = self.native()?;
        let actor = self.actor_for_client(client).ok_or(Q3HostError::ClientNotAdmitted)?;
        let entity = native.record_for_actor(&actor).ok_or(Q3HostError::ClientNotAdmitted)?;
        Ok(Q3ApplicationPlayer::for_native(client.clone(), actor, entity.slot))
    }

    /// Donor `connect(client, userinfo)`.
    pub fn connect(&mut self, client: &ClientId, userinfo: &str) -> Result<Q3ApplicationAdmission, Q3HostError> {
        if let Q3SourceBinding::Guest(guest) = &self.binding {
            return guest
                .connect(client, userinfo)
                .map_err(|error| Q3HostError::Guest(format!("{error:?}")));
        }
        let slot = i32::try_from(client.slot()).unwrap_or(i32::MAX);
        self.state.set_userinfo(slot, userinfo);
        match self.simulation.admit_player(client) {
            Ok(()) => self
                .carried_player(client)
                .map(|player| Q3ApplicationAdmission::Accepted { player }),
            Err(error) => {
                let actor = self.actor_for_client(client);
                if let Some(actor) = actor {
                    self.simulation.disconnect_player(&actor);
                }
                if let Q3HostError::AdmissionDenied(reason) = error {
                    return Ok(Q3ApplicationAdmission::Rejected { reason });
                }
                Err(error)
            }
        }
    }

    fn entity_count(&self) -> i32 {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => i32::try_from(guest.game.data.num_entities()).unwrap_or(i32::MAX),
            Q3SourceBinding::Native(native) => native.num_entities(),
        }
    }

    fn wire_entity(&self, number: i32) -> Q3EntityState {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => qvm_entity_state_to_wire(&guest.records.entity(number).s),
            Q3SourceBinding::Native(native) => native.entity_wire_state(number),
        }
    }

    fn linked(&self, number: i32) -> bool {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => guest.records.entity(number).r.linked,
            Q3SourceBinding::Native(native) => native.entity_flags(number).linked,
        }
    }

    fn config_entries(&self) -> Vec<GamestateEntry> {
        let mut entries = Vec::new();
        for index in 0..Q3_CONFIGSTRING_COUNT {
            let value = self.state.configstring_get(index);
            if !value.is_empty() {
                entries.push(GamestateEntry::Configstring { index, value });
            }
        }
        entries
    }

    /// Donor `sourceRound.preflight()`.
    pub fn source_round_preflight(&self) -> Result<(), Q3HostError> {
        let native = self.native()?;
        match self.simulation.source_restart_plan() {
            Q3SourceRestartPlan::SourceReset => {}
            Q3SourceRestartPlan::Other { reason } => {
                return Err(Q3HostError::RestartIncompatible(reason));
            }
        }
        match self.simulation.q3_source() {
            Some(current) if Rc::ptr_eq(&current, &native) => Ok(()),
            _ => Err(Q3HostError::SourceRoundRetired),
        }
    }

    /// Donor `sourceRound.rebind()`.
    pub fn source_round_rebind(&mut self) -> Result<(), Q3HostError> {
        let native = self.native()?;
        let next = self.simulation.q3_source();
        let accept = next.as_ref().is_some_and(|next| {
            !Rc::ptr_eq(next, &native)
                && Rc::ptr_eq(&next.server_state(), &self.state)
                && next.product() == self.product
        });
        if accept {
            if let Some(next) = next {
                self.binding = Q3SourceBinding::Native(next);
                return Ok(());
            }
        }
        Err(Q3HostError::RebindRejected)
    }

    /// Donor `sourceRound.reconnect(client, userinfo, lastCommand)`.
    pub fn source_round_reconnect(
        &mut self,
        client: &ClientId,
        userinfo: &str,
        last_command: Q3StoredUserCommand,
    ) -> Result<Q3ApplicationAdmission, Q3HostError> {
        self.native()?;
        let slot = i32::try_from(client.slot()).unwrap_or(i32::MAX);
        self.state.set_user_command(slot, last_command);
        self.connect(client, userinfo)
    }

    /// Donor `prepare(checksumFeed, serverId, configstring?)`.
    pub fn prepare(
        &mut self,
        checksum_feed: u32,
        server_id: i32,
        mut configstring: Option<Q3ConfigstringFn<'_>>,
    ) -> Result<(), Q3HostError> {
        self.require_saved_server_id(server_id)?;
        if self.packages.is_none() {
            let opened = (self.open_packages)(&self.content, checksum_feed)?;
            self.packages = Some(opened);
        }
        let packages = self.packages.as_ref().ok_or(Q3HostError::PackagesNotPrepared)?;
        if packages.checksum_feed() != checksum_feed {
            return Err(Q3HostError::ChecksumFeedChanged);
        }
        if self.cvars().variable_value("sv_pure") != 0.0 && !self.touched_cgame {
            self.content.resolve_mount("vm/cgame.qvm")?;
            self.touched_cgame = true;
        }
        if let Some(packages) = self.packages.as_mut() {
            packages.collect(&self.content);
        }
        if self.mode == Q3ServerMode::Restore {
            return Ok(());
        }
        let (loaded_checksums, loaded_names, referenced_checksums, referenced_names) = {
            let packages = self.packages.as_ref().ok_or(Q3HostError::PackagesNotPrepared)?;
            (
                packages.loaded_pak_checksums(),
                packages.loaded_pak_names(),
                packages.referenced_pak_checksums(),
                packages.referenced_pak_names(),
            )
        };
        let pure = self.cvars().variable_value("sv_pure") != 0.0;
        self.set_cvar("sv_serverid", &server_id.to_string())?;
        self.set_cvar("sv_paks", if pure { &loaded_checksums } else { "" })?;
        self.set_cvar("sv_pakNames", if pure { &loaded_names } else { "" })?;
        self.set_cvar("sv_referencedPaks", &referenced_checksums)?;
        self.set_cvar("sv_referencedPakNames", &referenced_names)?;
        let system_info = self
            .state
            .cvars
            .borrow_mut()
            .info_string(flags::SYSTEM_INFO, Some(8192))
            .map_err(|error| Q3HostError::Cvar(format!("{error:?}")))?;
        let server_info = self.state.server_info();
        for (index, value) in [(1, system_info), (0, server_info)] {
            if self.is_guest() {
                if self.state.configstring_get(index) != value {
                    self.state.configstring_set(index, &value);
                    if let Some(emit) = configstring.as_deref_mut() {
                        emit(index, &value);
                    }
                }
            } else {
                self.state.configstring_set(index, &value);
            }
        }
        Ok(())
    }

    /// Donor `pure(serverId, checksumFeedServerId?)`.
    pub fn pure(&self, server_id: i32, checksum_feed_server_id: Option<i32>) -> Result<Q3PureInfo, Q3HostError> {
        self.require_saved_server_id(server_id)?;
        let packages = self.packages.as_ref().ok_or(Q3HostError::PackagesNotPrepared)?;
        Ok(Q3PureInfo {
            enabled: self.cvars().variable_value("sv_pure") != 0.0,
            checksum_feed: packages.checksum_feed() as i32,
            checksum_feed_server_id: checksum_feed_server_id.unwrap_or(server_id),
            cgame_checksum: packages.pure_checksum("vm/cgame.qvm"),
            ui_checksum: packages.pure_checksum("vm/ui.qvm"),
            loaded_pure_checksums: packages.pack_pure_checksums(),
        })
    }

    /// Donor `downloadsEnabled()`.
    #[must_use]
    pub fn downloads_enabled(&self) -> bool {
        self.cvars().variable_value("sv_allowDownload") != 0.0
    }

    /// Donor `openDownload(name)`.
    pub fn open_download(&self, name: &str) -> Result<Option<P::Download>, Q3HostError> {
        let packages = self.packages.as_ref().ok_or(Q3HostError::PackagesNotPrepared)?;
        Ok(packages.open_download(name))
    }

    /// Donor `rate(player)`.
    pub fn rate(&self, player: &Q3ApplicationPlayer) -> Q3RateInfo {
        let info = match &self.binding {
            Q3SourceBinding::Guest(_) => self.state.get_userinfo(player.source_entity).unwrap_or_default(),
            Q3SourceBinding::Native(native) => native.engine_userinfo(player.source_entity),
        };
        let fps = self.cvars().variable_value("sv_fps").max(1.0);
        let requested_rate = parse_info_int(&qa_net::q3_net::q3_info_value(&info, "rate").unwrap_or_default());
        let requested_snaps = parse_info_int(&qa_net::q3_net::q3_info_value(&info, "snaps").unwrap_or_default());
        let rate = requested_rate.map_or(3000, |value| value.clamp(1000, 90000));
        let snaps = requested_snaps.map_or(fps, |value| (value as f32).clamp(1.0, fps).max(1.0));
        Q3RateInfo {
            rate,
            max_rate: self.cvars().variable_value("sv_maxRate"),
            snapshot_msec: (1000.0 / snaps).trunc() as i32,
            local: qa_net::q3_net::q3_info_value(&info, "ip").unwrap_or_default() == "localhost",
            force_lan: false,
            lan: false,
        }
    }

    /// Donor `supportsSourceWire()`.
    #[must_use]
    pub fn supports_source_wire(&self) -> Q3WireSupport {
        let mut reasons = Vec::new();
        if !self.simulation.movement_provider().starts_with("q3:") {
            reasons.push("Native Q3 wire requires Q3 movement".to_string());
        }
        if !self.simulation.character_provider().starts_with("q3:") {
            reasons.push("Native Q3 wire requires a Q3 character".to_string());
        }
        if reasons.is_empty() {
            Q3WireSupport::Supported
        } else {
            Q3WireSupport::Unsupported { reasons }
        }
    }

    /// Donor `time()`.
    #[must_use]
    pub fn time(&self) -> i32 {
        match &self.binding {
            Q3SourceBinding::Guest(_) => (self.simulation.time_seconds() * 1000.0) as i32,
            Q3SourceBinding::Native(native) => native.now_ms(),
        }
    }

    /// Donor `occupiedSlots()`.
    #[must_use]
    pub fn occupied_slots(&self) -> Vec<i32> {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => guest.players().iter().map(|player| player.source_entity).collect(),
            Q3SourceBinding::Native(native) => self
                .simulation
                .players()
                .iter()
                .map(|actor| native.record_for_actor(actor).map_or(-1, |record| record.slot))
                .collect(),
        }
    }

    /// Donor `admit(request)`.
    pub fn admit(&mut self, request: &Q3AdmitRequest) -> Result<Q3ApplicationAdmission, Q3HostError> {
        let client = self.session.create_client(request.slot)?;
        let origin = match &request.address {
            NetworkAddress::Loopback { .. } => Q3ClientOrigin::Loopback,
            _ => Q3ClientOrigin::Remote,
        };
        self.session.connect_client(&client, origin);
        match self.connect(&client, &request.userinfo) {
            Ok(Q3ApplicationAdmission::Rejected { reason }) => {
                self.session.close_client(&client);
                Ok(Q3ApplicationAdmission::Rejected { reason })
            }
            Ok(admitted) => Ok(admitted),
            Err(error) => {
                self.session.close_client(&client);
                Err(error)
            }
        }
    }
}

impl<S, E, C, P, A> Q3ApplicationServerHost<S, E, C, P, A>
where
    S: Q3HostSimulation,
    E: Q3HostSession,
    C: Q3HostContent,
    P: Q3HostPackages,
{
    /// Donor `disconnect(player)`.
    pub fn disconnect(&mut self, player: &Q3ApplicationPlayer) -> Result<(), Q3HostError> {
        let outcome = match &self.binding {
            Q3SourceBinding::Guest(guest) => guest
                .disconnect(player)
                .map_err(|error| Q3HostError::Guest(format!("{error:?}"))),
            Q3SourceBinding::Native(_) => {
                self.simulation.disconnect_player(&player.actor);
                Ok(())
            }
        };
        self.session.close_client(&player.client);
        outcome
    }

    /// Donor `gameState(player, serverId)`.
    pub fn game_state(&self, player: &Q3ApplicationPlayer, server_id: i32) -> Result<Gamestate, Q3HostError> {
        self.require_saved_server_id(server_id)?;
        let mut entries: Vec<GamestateEntry> = self
            .config_entries()
            .into_iter()
            .filter(|entry| {
                self.mode == Q3ServerMode::Restore
                    || !matches!(entry, GamestateEntry::Configstring { index: 0 | 1, .. })
            })
            .collect();
        if self.mode == Q3ServerMode::New {
            self.set_cvar("sv_serverid", &server_id.to_string())?;
            let system_info = self
                .state
                .cvars
                .borrow_mut()
                .info_string(flags::SYSTEM_INFO, Some(8192))
                .map_err(|error| Q3HostError::Cvar(format!("{error:?}")))?;
            entries.insert(
                0,
                GamestateEntry::Configstring {
                    index: 1,
                    value: system_info,
                },
            );
            entries.insert(
                0,
                GamestateEntry::Configstring {
                    index: 0,
                    value: self.state.server_info(),
                },
            );
        }
        for number in 1..self.entity_count() {
            if self.linked(number) {
                entries.push(GamestateEntry::Baseline {
                    number,
                    entity: self.wire_entity(number),
                });
            }
        }
        let packages = self.packages.as_ref().ok_or(Q3HostError::PackagesNotPrepared)?;
        Ok(Gamestate {
            command_sequence: 0,
            entries,
            client_number: player.source_entity,
            checksum_feed: packages.checksum_feed() as i32,
        })
    }

    fn snapshot_player(&self, player: &Q3ApplicationPlayer) -> Result<Q3PlayerState, Q3HostError> {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => Ok(from_qvm_player_state(
                &guest.records.player(player.source_entity),
                &self.product,
            )),
            Q3SourceBinding::Native(native) => native
                .record_for_actor(&player.actor)
                .and_then(|record| record.client)
                .map(|client| client.player_state)
                .ok_or(Q3HostError::SnapshotPlayerGone),
        }
    }

    fn snapshot_link(&self, number: i32) -> Option<Q3VisibilityLink> {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => guest.records.visibility(number).map(|link| Q3VisibilityLink {
                areanum: link.areanum,
                areanum2: link.areanum2,
                clusters: link.clusters,
                last_cluster: link.last_cluster,
            }),
            Q3SourceBinding::Native(native) => {
                let actor = native.entity_actor(number)?;
                let bounds = self.simulation.body_bounds(&actor)?;
                let scene = self.simulation.scene();
                let leaves = scene.box_leaves(&bounds, Q3_LINK_LEAF_CAP);
                let mut areas = Vec::new();
                let mut seen_areas = HashSet::new();
                let mut clusters = Vec::new();
                let mut seen_clusters = HashSet::new();
                for leaf in leaves {
                    let area = scene.leaf_area(leaf);
                    if seen_areas.insert(area) {
                        areas.push(area);
                    }
                    let cluster = scene.leaf_cluster(leaf);
                    if cluster >= 0 && seen_clusters.insert(cluster) {
                        clusters.push(cluster);
                    }
                }
                Some(Q3VisibilityLink {
                    areanum: areas.first().copied().unwrap_or(-1),
                    areanum2: areas.get(1).copied().unwrap_or(-1),
                    clusters,
                    last_cluster: 0,
                })
            }
        }
    }

    /// Donor `snapshot(player)`.
    pub fn snapshot(&self, player: &Q3ApplicationPlayer) -> Result<Q3HostSnapshot, Q3HostError> {
        let state = self.snapshot_player(player)?;
        let count = self.entity_count();
        let mut entities = Vec::new();
        for number in 0..count {
            let (linked, sv_flags, single_client) = match &self.binding {
                Q3SourceBinding::Guest(guest) => {
                    let item = guest.records.entity(number);
                    (item.r.linked, item.r.sv_flags, item.r.single_client)
                }
                Q3SourceBinding::Native(native) => {
                    let item = native.entity_flags(number);
                    (item.linked, item.sv_flags, item.single_client)
                }
            };
            entities.push(Q3VisibilityEntity {
                state: self.wire_entity(number),
                linked,
                flags: sv_flags,
                single_client,
            });
        }
        let mut links = Vec::new();
        for number in 0..count {
            links.push(self.snapshot_link(number));
        }
        let mut bindings = Q3SnapshotBindings {
            entities,
            links,
            scene: self.simulation.scene(),
            cvars: Rc::clone(&self.state.cvars),
            print: Rc::clone(&self.print),
        };
        let visible = select_q3_snapshot_entities(&state, &mut bindings)?;
        Ok(Q3HostSnapshot {
            player: state,
            area_mask: visible.area_mask,
            entities: visible.entities,
        })
    }

    /// Donor `begin(player, command)` (guest only).
    pub fn begin(&self, player: &Q3ApplicationPlayer, command: &Q3StoredUserCommand) -> Result<(), Q3HostError> {
        let guest = self.guest()?;
        guest
            .begin(player, command)
            .map_err(|error| Q3HostError::Guest(format!("{error:?}")))
    }

    /// Donor `input(player, command, sequence)`.
    pub fn input(
        &mut self,
        player: &Q3ApplicationPlayer,
        command: &Q3StoredUserCommand,
        sequence: u64,
    ) -> Result<Option<ActorCommand>, Q3HostError> {
        let actor_command = ActorCommand {
            actor: player.actor.clone(),
            source: CommandSource::Remote {
                client: player.client.clone(),
            },
            sequence,
            command: to_q3_user_command(command),
            arsenal: None,
        };
        if let Q3SourceBinding::Guest(guest) = &self.binding {
            self.simulation.observe_client_command(actor_command);
            guest
                .think(player, command)
                .map_err(|error| Q3HostError::Guest(format!("{error:?}")))?;
            return Ok(None);
        }
        Ok(Some(actor_command))
    }

    /// Donor `command(player, name, args)`.
    pub fn command(&self, player: &Q3ApplicationPlayer, name: &str, args: &[String]) -> Result<(), Q3HostError> {
        match &self.binding {
            Q3SourceBinding::Guest(guest) => {
                let mut argv = Vec::with_capacity(args.len() + 1);
                argv.push(name.to_string());
                argv.extend(args.iter().cloned());
                guest
                    .command(player, &argv)
                    .map_err(|error| Q3HostError::Guest(format!("{error:?}")))
            }
            Q3SourceBinding::Native(native) => {
                native.player_command(&player.actor, name, args);
                Ok(())
            }
        }
    }

    /// Donor `userinfo(player, value)`.
    pub fn userinfo(&mut self, player: &Q3ApplicationPlayer, value: &str) -> Result<(), Q3HostError> {
        if let Q3SourceBinding::Guest(guest) = &self.binding {
            return guest
                .userinfo(player, value)
                .map_err(|error| Q3HostError::Guest(format!("{error:?}")));
        }
        let native = self.native()?;
        self.state.set_userinfo(player.source_entity, value);
        native.userinfo_changed(player.source_entity);
        self.simulation.notify_client_event("userinfo", &player.actor);
        Ok(())
    }

    fn status_put(&self, info: &str, key: &str, value: &str) -> Result<String, Q3HostError> {
        let mut emit = |text: &str| (self.print)(text);
        set_info_value(
            info,
            key,
            value,
            InfoOptions {
                dialect: Dialect::Q3,
                maximum_length: 1024,
                target: InfoTarget::ServerInfo,
                server_high_characters: false,
            },
            &mut emit,
        )
        .map_err(|error| Q3HostError::Cvar(format!("{error:?}")))
    }

    /// Donor `status(challenge, detailed)`.
    pub fn status(&self, challenge: &str, detailed: bool) -> Result<Option<String>, Q3HostError> {
        if self.cvars().variable_value("g_gametype") == 2.0
            || (!detailed && self.cvars().variable_value("ui_singlePlayerActive") != 0.0)
        {
            return Ok(None);
        }
        let server_info = if detailed {
            self.state
                .cvars
                .borrow_mut()
                .info_string(flags::SERVER_INFO, None)
                .map_err(|error| Q3HostError::Cvar(format!("{error:?}")))?
        } else {
            String::new()
        };
        let mut info = self.status_put(&server_info, "challenge", challenge)?;
        if !detailed {
            let private_clients = self.admission_private_clients();
            let occupied = self.occupied_slots();
            let clients = occupied
                .iter()
                .filter(|slot| **slot >= private_clients && **slot < self.max_clients)
                .count();
            let fields = [
                ("protocol", Q3_PROTOCOL_VERSION.to_string()),
                ("hostname", self.cvars().variable_string("sv_hostname")),
                ("mapname", self.cvars().variable_string("mapname")),
                ("clients", clients.to_string()),
                ("sv_maxclients", (self.max_clients - private_clients).to_string()),
                (
                    "gametype",
                    self.cvars()
                        .get("g_gametype")
                        .map_or(0, |snapshot| snapshot.integer_value)
                        .to_string(),
                ),
                (
                    "pure",
                    self.cvars()
                        .get("sv_pure")
                        .map_or(0, |snapshot| snapshot.integer_value)
                        .to_string(),
                ),
            ];
            for (key, value) in fields {
                info = self.status_put(&info, key, &value)?;
            }
            for (key, name) in [("minPing", "sv_minPing"), ("maxPing", "sv_maxPing")] {
                let value = self.cvars().get(name).map_or(0, |snapshot| snapshot.integer_value);
                if value != 0 {
                    info = self.status_put(&info, key, &value.to_string())?;
                }
            }
            info = self.status_put(&info, "game", &self.cvars().variable_string("fs_game"))?;
            return Ok(Some(format!("infoResponse\n{info}")));
        }
        if self.content.q3_demo_restricted() {
            let keywords = qa_net::q3_net::q3_info_value(&info, "sv_keywords").unwrap_or_default();
            info = self.status_put(&info, "sv_keywords", &format!("demo {keywords}"))?;
        }
        let mut rows = Vec::new();
        match &self.binding {
            Q3SourceBinding::Guest(guest) => {
                for player in guest.players() {
                    let ps = guest.records.player(player.source_entity);
                    let name = qa_net::q3_net::q3_info_value(
                        &self.state.get_userinfo(player.source_entity).unwrap_or_default(),
                        "name",
                    )
                    .unwrap_or_default();
                    let slot = usize::try_from(player.source_entity).unwrap_or(usize::MAX);
                    let ping = guest
                        .game
                        .data
                        .player_ping(slot)
                        .map_err(|error| Q3HostError::Guest(format!("{error:?}")))?;
                    rows.push(format!("{} {ping} \"{name}\"\n", ps.persistent[0]));
                }
            }
            Q3SourceBinding::Native(native) => {
                for actor in self.simulation.players() {
                    let Some(client) = native.record_for_actor(&actor).and_then(|record| record.client) else {
                        rows.push(String::new());
                        continue;
                    };
                    let score = client.player_state.persistant.get(0)?;
                    rows.push(format!("{score} {} \"{}\"\n", client.player_state.ping, client.netname));
                }
            }
        }
        let mut players = String::new();
        for row in rows {
            if players.len() + row.len() >= Q3_STATUS_ROW_BUDGET {
                break;
            }
            players.push_str(&row);
        }
        Ok(Some(format!("statusResponse\n{info}\n{players}")))
    }
}

/// Snapshot visibility bindings (donor `snapshot` bindings object).
struct Q3SnapshotBindings {
    entities: Vec<Q3VisibilityEntity>,
    links: Vec<Option<Q3VisibilityLink>>,
    scene: Rc<dyn Q3HostScene>,
    cvars: Rc<RefCell<CvarRegistry>>,
    print: Q3PrintFn,
}

impl Q3VisibilityWorld for Q3SnapshotBindings {
    fn point_leafnum(&self, point: Vec3) -> i32 {
        self.scene.point_leaf(point)
    }

    fn leaf_area(&self, leaf: i32) -> i32 {
        self.scene.leaf_area(leaf)
    }

    fn leaf_cluster(&self, leaf: i32) -> i32 {
        self.scene.leaf_cluster(leaf)
    }

    fn write_area_bits(&self, bytes: &mut [u8; 32], area: i32) -> i32 {
        let bits = self.scene.area_bits(area);
        for (index, byte) in bits.iter().enumerate().take(bytes.len()) {
            bytes[index] |= byte;
        }
        bits.len() as i32
    }

    fn cluster_pvs_byte(&self, cluster: i32, index: i32) -> u8 {
        let mut value = 0u8;
        for bit in 0..8 {
            if self.scene.cluster_visible(cluster, index * 8 + bit) {
                value |= 1 << bit;
            }
        }
        value
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.cvars.borrow().variable_value("cm_noAreas") != 0.0
            || first >= 0 && second >= 0 && self.scene.areas_connected(first, second)
    }
}

impl Q3VisibilityBindings for Q3SnapshotBindings {
    fn collision(&self) -> &dyn Q3VisibilityWorld {
        self
    }

    fn entity_count(&self) -> i32 {
        self.entities.len() as i32
    }

    fn dead(&self) -> bool {
        false
    }

    fn entity(&self, number: i32) -> Q3VisibilityEntity {
        self.entities
            .get(usize::try_from(number).unwrap_or(usize::MAX))
            .cloned()
            .unwrap_or_else(|| Q3VisibilityEntity {
                state: Q3EntityState::default(),
                linked: false,
                flags: 0,
                single_client: 0,
            })
    }

    fn fix_entity_number(&mut self, number: i32) {
        if let Some(entity) = self.entities.get_mut(usize::try_from(number).unwrap_or(usize::MAX)) {
            entity.state.number = number;
        }
    }

    fn link(&self, number: i32) -> Option<Q3VisibilityLink> {
        self.links
            .get(usize::try_from(number).unwrap_or(usize::MAX))
            .cloned()
            .flatten()
    }

    fn print(&mut self, text: &str) {
        (self.print)(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::simulation::q3::host::Q3HostSettings;
    use crate::bootstrap::simulation::q3::server_state::Q3ServerStateOptions;
    use qa_core::identity::IdentityOwner;

    #[test]
    fn protocol_version_is_68() {
        assert_eq!(Q3_PROTOCOL_VERSION, 68);
    }

    #[test]
    fn stored_command_maps_to_movement() {
        let stored = Q3StoredUserCommand {
            server_time: 1200,
            angles: [10, 20, 30],
            forwardmove: 127,
            rightmove: -64,
            upmove: 63,
            buttons: 33,
            weapon: 5,
        };
        let mapped = to_q3_user_command(&stored);
        match mapped {
            UserCommand::Q3 {
                server_time_milliseconds,
                angle_words,
                buttons,
                weapon,
                forward_move,
                right_move,
                up_move,
            } => {
                assert_eq!(server_time_milliseconds, 1200.0);
                assert_eq!(angle_words, [10.0, 20.0, 30.0]);
                assert_eq!(buttons, 33.0);
                assert_eq!(weapon, 5.0);
                assert_eq!(forward_move, 127.0);
                assert_eq!(right_move, -64.0);
                assert_eq!(up_move, 63.0);
            }
            _ => panic!("expected Q3 movement command"),
        }
    }

    #[test]
    fn info_int_parses_prefix() {
        assert_eq!(parse_info_int("125"), Some(125));
        assert_eq!(parse_info_int("  -42x"), Some(-42));
        assert_eq!(parse_info_int("abc"), None);
        assert_eq!(parse_info_int(""), None);
    }

    #[test]
    fn entity_state_copies_guest_record() {
        let trajectory = QvmTrajectory {
            trajectory_type: 1,
            time: 100,
            duration: 50,
            base: qa_core::math::vec3(1.0, 2.0, 3.0),
            delta: qa_core::math::vec3(4.0, 5.0, 6.0),
        };
        let guest = QvmEntityState {
            number: 7,
            e_type: 2,
            e_flags: 3,
            pos: trajectory.clone(),
            apos: trajectory,
            time: 11,
            time2: 12,
            origin: qa_core::math::vec3(7.0, 8.0, 9.0),
            origin2: qa_core::math::vec3(1.0, 1.0, 1.0),
            angles: qa_core::math::vec3(0.0, 90.0, 0.0),
            angles2: qa_core::math::vec3(0.0, 0.0, 0.0),
            other_entity_num: 5,
            other_entity_num2: 6,
            ground_entity_num: 4,
            constant_light: 0,
            loop_sound: 9,
            modelindex: 10,
            modelindex2: 11,
            client_num: 1,
            frame: 3,
            solid: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: 4,
            legs_anim: 1,
            torso_anim: 2,
            generic1: 8,
        };
        let wire = qvm_entity_state_to_wire(&guest);
        assert_eq!(wire.number, 7);
        assert_eq!(wire.e_type, 2);
        assert_eq!(wire.origin, [7.0, 8.0, 9.0]);
        assert_eq!(wire.pos.base, [1.0, 2.0, 3.0]);
        assert_eq!(wire.pos.delta, [4.0, 5.0, 6.0]);
        assert_eq!(wire.client_num, 1);
        assert_eq!(wire.generic1, 8);
    }

    #[derive(Debug)]
    struct StubScene;

    impl Q3HostScene for StubScene {
        fn box_leaves(&self, _bounds: &Bounds, _cap: usize) -> Vec<i32> {
            Vec::new()
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            -1
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            -1
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            false
        }

        fn area_bits(&self, _area: i32) -> Vec<u8> {
            Vec::new()
        }

        fn cluster_visible(&self, _cluster: i32, _leaf: i32) -> bool {
            false
        }
    }

    #[derive(Debug)]
    struct StubNative {
        state: Rc<Q3ServerState>,
    }

    impl Q3HostNativeSource for StubNative {
        fn product(&self) -> String {
            "baseq3".to_string()
        }

        fn server_state(&self) -> Rc<Q3ServerState> {
            Rc::clone(&self.state)
        }

        fn now_ms(&self) -> i32 {
            4242
        }

        fn engine_userinfo(&self, _slot: i32) -> String {
            String::new()
        }

        fn num_entities(&self) -> i32 {
            1
        }

        fn entity_wire_state(&self, _number: i32) -> Q3EntityState {
            Q3EntityState::default()
        }

        fn entity_flags(&self, _number: i32) -> Q3NativeEntityFlags {
            Q3NativeEntityFlags {
                linked: false,
                sv_flags: 0,
                single_client: 0,
            }
        }

        fn entity_actor(&self, _number: i32) -> Option<ActorId> {
            None
        }

        fn record_for_actor(&self, _actor: &ActorId) -> Option<Q3NativeRecord> {
            None
        }

        fn player_command(&self, _actor: &ActorId, _name: &str, _args: &[String]) {}

        fn userinfo_changed(&self, _slot: i32) {}
    }

    #[derive(Debug)]
    struct StubSim {
        native: Option<Rc<StubNative>>,
        movement: String,
        character: String,
    }

    impl Q3HostSimulation for StubSim {
        fn q3_guest(&self) -> Option<Rc<Q3QvmServerGame>> {
            None
        }

        fn q3_source(&self) -> Option<Rc<dyn Q3HostNativeSource>> {
            self.native.clone().map(|native| native as Rc<dyn Q3HostNativeSource>)
        }

        fn max_clients(&self) -> i32 {
            8
        }

        fn players(&self) -> Vec<ActorId> {
            Vec::new()
        }

        fn movement_client(&self, _actor: &ActorId) -> Option<ClientId> {
            None
        }

        fn admit_player(&mut self, _client: &ClientId) -> Result<(), Q3HostError> {
            Ok(())
        }

        fn disconnect_player(&mut self, _actor: &ActorId) {}

        fn source_restart_plan(&self) -> Q3SourceRestartPlan {
            Q3SourceRestartPlan::SourceReset
        }

        fn time_seconds(&self) -> f64 {
            1.5
        }

        fn movement_provider(&self) -> String {
            self.movement.clone()
        }

        fn character_provider(&self) -> String {
            self.character.clone()
        }

        fn observe_client_command(&mut self, _command: ActorCommand) {}

        fn notify_client_event(&mut self, _kind: &str, _actor: &ActorId) {}

        fn scene(&self) -> Rc<dyn Q3HostScene> {
            Rc::new(StubScene)
        }

        fn body_bounds(&self, _actor: &ActorId) -> Option<Bounds> {
            None
        }
    }

    #[derive(Debug, Default)]
    struct StubSession;

    impl Q3HostSession for StubSession {
        fn create_client(&mut self, _slot: u32) -> Result<ClientId, Q3HostError> {
            Err(Q3HostError::Session("closed".to_string()))
        }

        fn connect_client(&mut self, _client: &ClientId, _origin: Q3ClientOrigin) {}

        fn close_client(&mut self, _client: &ClientId) {}
    }

    #[derive(Debug, Default)]
    struct StubContent;

    impl Q3HostContent for StubContent {
        fn q3_demo_restricted(&self) -> bool {
            false
        }

        fn resolve_mount(&self, _path: &str) -> Result<(), Q3HostError> {
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct StubPackages;

    impl Q3HostPackages for StubPackages {
        type Download = Vec<u8>;

        fn checksum_feed(&self) -> u32 {
            0
        }

        fn loaded_pak_checksums(&self) -> String {
            String::new()
        }

        fn loaded_pak_names(&self) -> String {
            String::new()
        }

        fn referenced_pak_checksums(&self) -> String {
            String::new()
        }

        fn referenced_pak_names(&self) -> String {
            String::new()
        }

        fn collect(&mut self, _content: &dyn Q3HostContent) {}

        fn pure_checksum(&self, _path: &str) -> i32 {
            0
        }

        fn pack_pure_checksums(&self) -> Vec<i32> {
            Vec::new()
        }

        fn open_download(&self, _name: &str) -> Option<Vec<u8>> {
            None
        }
    }

    fn stub_state() -> Rc<Q3ServerState> {
        let owner = IdentityOwner::create("q3-test").unwrap();
        Rc::new(Q3ServerState::new(Q3ServerStateOptions {
            session: owner.session().clone(),
            settings: Q3HostSettings {
                game_type: 0,
                single_player: false,
                max_clients: 8,
                map_name: "q3dm1".to_string(),
                source_registry: None,
                source_archive: Vec::new(),
                cvars: Vec::new(),
            },
            now: Rc::new(|| 0),
            print: Rc::new(|_| {}),
            register_server_cvars: Rc::new(|_, _, _| {}),
        }))
    }

    fn native_host(
        movement: &str,
        character: &str,
    ) -> Q3ApplicationServerHost<StubSim, StubSession, StubContent, StubPackages, ()> {
        let native = Rc::new(StubNative { state: stub_state() });
        create_q3_application_server_host(Q3ApplicationServerBindingOptions {
            session: StubSession,
            simulation: StubSim {
                native: Some(native),
                movement: movement.to_string(),
                character: character.to_string(),
            },
            content: StubContent,
            mode: Q3ServerMode::New,
            administration: None,
            open_packages: Rc::new(|_, _| Ok(StubPackages)),
            print: Rc::new(|_| {}),
        })
        .unwrap()
    }

    #[test]
    fn missing_provider_errors() {
        let outcome = create_q3_application_server_host(Q3ApplicationServerBindingOptions {
            session: StubSession,
            simulation: StubSim {
                native: None,
                movement: "q3:base".to_string(),
                character: "q3:base".to_string(),
            },
            content: StubContent,
            mode: Q3ServerMode::New,
            administration: None::<()>,
            open_packages: Rc::new(|_, _| Ok(StubPackages)),
            print: Rc::new(|_| {}),
        });
        match outcome {
            Ok(_) => panic!("expected missing provider"),
            Err(error) => assert_eq!(error, Q3HostError::MissingProvider),
        }
    }

    #[test]
    fn native_wire_support_reports_reasons() {
        let host = native_host("q1:netquake", "q2:classic");
        match host.supports_source_wire() {
            Q3WireSupport::Unsupported { reasons } => assert_eq!(reasons.len(), 2),
            Q3WireSupport::Supported => panic!("expected unsupported wire"),
        }
        let host = native_host("q3:base", "q3:base");
        assert_eq!(host.supports_source_wire(), Q3WireSupport::Supported);
    }

    #[test]
    fn native_time_and_rate_defaults() {
        let host = native_host("q3:base", "q3:base");
        assert_eq!(host.time(), 4242);
        let owner = IdentityOwner::create("q3-rate").unwrap();
        let player = Q3ApplicationPlayer::for_native(owner.client(0, 0), owner.actor(1, 1), 0);
        let rate = host.rate(&player);
        assert_eq!(rate.rate, 3000);
        assert_eq!(rate.snapshot_msec, 1000);
        assert!(!rate.local);
        assert!(!rate.force_lan);
        assert!(!rate.lan);
    }
}
