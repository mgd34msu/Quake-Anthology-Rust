//! Quake II application network vocabulary.
//!
//! Port of `src/app/bootstrap/network/types.ts` (`RemotePresentationAccess`,
//! `ApplicationNetwork`, Q2 application players, game state, server/client
//! hosts, network options, `q2GameCallback`). The donor's asynchronous host
//! calls become synchronous; every state transition keeps the donor's order.
//!
//! Presentation shapes (`PlayerView`, `PlayerUi`, [`PresentationModel`],
//! [`WorldText`], [`NetworkPresentationEvent`]) mirror the out-of-scope
//! donor `../simulation/types.ts`, following the `q1_session_actions`
//! precedent: the simulation lane owns the canonical port, and these
//! mirrors carry exactly the fields the network wave reads and publishes.
//! Wire state reuses `qa-net` (`Q2ServerEvent`, `Q2WireFrame`,
//! `Q2ServerRecord`, `Q2ServerData`, `Q2ConnectRequest`,
//! `Q2ServerMessageOptions`, `MvdCapture`), protocol identity reuses the
//! bootstrap [`Q2ProtocolIdentity`](crate::bootstrap::demo_recording::Q2ProtocolIdentity),
//! and simulation I/O reuses [`ActorCommand`](qa_net::common::commands::ActorCommand)
//! with [`SimulationOutput`](qa_world::session::SimulationOutput).

use std::collections::HashMap;

use qa_content::contract::{
    ArmorState, ContentId, HeldWeaponDeclaration, InventoryEntry, ProviderReference, ResolvedResourceReference,
    WeaponHudIcon,
};
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_core::identity::{ActorId, ClientId};
use qa_core::math::{Vec3, Vec4};
use qa_net::common::commands::ActorCommand;
use qa_net::common::endpoint::NetworkAddress;
use qa_net::common::session::{WireAdmission, WireSelection};
use qa_net::common::transport::DatagramTransport;
use qa_net::q2::{EntityState, Usercmd};
use qa_net::q2_net::{
    Q2ConnectRequest, Q2Info, Q2LimitedRcon, Q2ServerData, Q2ServerEvent, Q2ServerMessageOptions, Q2ServerProfile,
    Q2ServerRecord, Q2Status, Q2WireFrame,
};
use qa_net::q2_svc::MvdCapture;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::q2_downloads::{Q2ApplicationClientDownloads, Q2ApplicationDownloads};
use crate::bootstrap::demo_recording::{DemoRecordingSeed, DemoRecordingSink, Q2ProtocolIdentity};

// ---------------------------------------------------------------------------
// Presentation mirrors (donor `../simulation/types.ts`, simulation lane owns)
// ---------------------------------------------------------------------------

/// Opaque presentation event (mirrors `SimulationPresentationEvent`).
///
/// Network hosts only carry these between `poll` and `observe`/`publish`,
/// so the mirror keeps the routing envelope: source family, event kind, and
/// an optional per-actor recipient (`None` broadcasts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkPresentationEvent {
    /// Source family (for example `q2`).
    pub family: String,
    /// Inner event kind.
    pub kind: String,
    /// Optional per-actor recipient.
    pub recipient: Option<ActorId>,
}

/// Player camera view (mirrors `PlayerView`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    /// World origin.
    pub origin: Vec3,
    /// View angles in degrees.
    pub angles: Vec3,
    /// Eye height above the origin.
    pub view_height: f64,
    /// Screen blend color.
    pub blend: Option<Vec4>,
    /// Rerelease damage blend color.
    pub damage_blend: Option<Vec4>,
    /// View kick angles.
    pub kick_angles: Option<Vec3>,
    /// Horizontal field of view in degrees.
    pub field_of_view: Option<f64>,
    /// Client view-offset delta.
    pub client_view_offset_delta: Option<Vec3>,
    /// Foreign character death flag.
    pub foreign_character_death: Option<bool>,
    /// Pitch drift state.
    pub pitch_drift: Option<PlayerPitchDrift>,
}

/// Pitch drift state (`PlayerView['pitchDrift']`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPitchDrift {
    /// Grounded flag.
    pub grounded: bool,
    /// Ideal pitch in degrees.
    pub ideal_pitch: f64,
    /// Drift disabled flag.
    pub disabled: bool,
}

/// Arsenal warning level (`PlayerUi['arsenalWarning']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArsenalWarning {
    /// No warning.
    None,
    /// Low ammunition.
    Low,
    /// Empty.
    Empty,
}

/// Native inventory presentation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeInventoryKind {
    /// Weapon.
    Weapon,
    /// Ammunition.
    Ammunition,
    /// Item.
    Item,
}

/// Native inventory presentation (`PlayerUi['nativeInventory']['presentation']`).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInventoryPresentation {
    /// Provider reference.
    pub source: ProviderReference,
    /// Presentation kind.
    pub kind: NativeInventoryKind,
    /// Item icon (item kind only).
    pub icon: Option<WeaponHudIcon>,
    /// Weapon id (weapon/ammunition kinds).
    pub weapon: Option<String>,
}

/// One native inventory row.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInventoryItem {
    /// Item id.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Owned count.
    pub count: f64,
}

/// Native inventory block (`PlayerUi['nativeInventory']`).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeInventory {
    /// Inventory rows.
    pub items: Vec<NativeInventoryItem>,
    /// Selected item id.
    pub selected: Option<String>,
    /// Presentation override.
    pub presentation: Option<NativeInventoryPresentation>,
}

/// Weapon ammunition status (`PlayerUi['weaponStatus']['ammo']`).
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponAmmoStatus {
    /// Unmetered weapon.
    Unmetered,
    /// Metered ammunition.
    Finite {
        /// Ammunition item id.
        item: String,
        /// Rounds carried.
        count: f64,
        /// Whether the weapon starts with ammunition.
        has_ammo_to_start: bool,
        /// Low-ammunition flag.
        low: bool,
    },
}

/// Weapon status block (`PlayerUi['weaponStatus']`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStatus {
    /// Provider reference.
    pub source: ProviderReference,
    /// Weapon item id.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Ammunition status.
    pub ammo: WeaponAmmoStatus,
}

/// One HUD powerup row.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPowerup {
    /// Powerup item id.
    pub item: String,
    /// Display label.
    pub label: String,
    /// Seconds remaining.
    pub remaining_seconds: f64,
}

/// One HUD arsenal row (`PlayerUi['items']`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerArsenalItem {
    /// Item id.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Row kind.
    pub kind: PlayerArsenalKind,
    /// Source ordinal.
    pub source_ordinal: f64,
    /// Owned flag.
    pub owned: bool,
    /// Has-ammunition flag.
    pub has_ammo: bool,
    /// Round count, when metered.
    pub count: Option<f64>,
    /// Warning threshold count.
    pub warning_count: f64,
}

/// HUD arsenal row kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerArsenalKind {
    /// Weapon row.
    Weapon,
    /// Powerup row.
    Powerup,
}

/// Carried ammunition (`PlayerUi['ammo']`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAmmo {
    /// Ammunition item id.
    pub item: String,
    /// Rounds carried.
    pub count: f64,
}

/// Player HUD state (mirrors `PlayerUi`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerUi {
    /// Selected-arsenal marker.
    pub selected_arsenal: Option<bool>,
    /// Native inventory block.
    pub native_inventory: Option<NativeInventory>,
    /// Health.
    pub health: f64,
    /// Armor state.
    pub armor: ArmorState,
    /// Active weapon item id.
    pub active_weapon: Option<String>,
    /// Carried ammunition.
    pub ammo: Option<PlayerAmmo>,
    /// Inventory entries.
    pub inventory: Vec<InventoryEntry>,
    /// Arsenal warning.
    pub arsenal_warning: ArsenalWarning,
    /// Active powerups.
    pub powerups: Vec<PlayerPowerup>,
    /// Arsenal rows.
    pub items: Vec<PlayerArsenalItem>,
    /// Weapon status.
    pub weapon_status: Option<WeaponStatus>,
}

/// Model flare decoration (`SimulationPresentation['flare']`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationFlare {
    /// Flare image path.
    pub image: String,
    /// Fade start.
    pub fade_start: f64,
    /// Fade end.
    pub fade_end: f64,
    /// Scale.
    pub scale: f64,
    /// Flare color.
    pub color: Vec3,
    /// Rim color.
    pub rim_color: Option<Vec3>,
    /// Lock-angle flag.
    pub lock_angle: bool,
}

/// Indexed skin (`SimulationPresentation['indexedSkin']`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationIndexedSkin {
    /// Skin name.
    pub name: String,
    /// Width in pixels.
    pub width: i64,
    /// Height in pixels.
    pub height: i64,
    /// Pixel data (`width * height` bytes).
    pub pixels: Vec<u8>,
}

/// Player color pair (`SimulationPresentation['playerColors']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationPlayerColors {
    /// Top color.
    pub top: i64,
    /// Bottom color.
    pub bottom: i64,
}

/// Model beam (`SimulationPresentation['modelBeam']`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationModelBeam {
    /// Segment length.
    pub segment_length: f64,
}

/// Shader beam (`SimulationPresentation['shaderBeam']`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationShaderBeam {
    /// Shader path.
    pub path: String,
    /// Beam end.
    pub end: Vec3,
    /// Beam width.
    pub width: i64,
}

/// Model attachment (`SimulationPresentation['modelAttachments']`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationModelAttachment {
    /// Model path.
    pub path: String,
    /// Tag name.
    pub tag: String,
}

/// Field-of-view offset (`SimulationPresentation['modelAnchor']['fovOffset']`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationFovOffset {
    /// Rows above.
    pub above: i64,
    /// Scale factor.
    pub scale: f64,
}

/// Model anchor (`SimulationPresentation['modelAnchor']`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationModelAnchor {
    /// Model path.
    pub path: String,
    /// Tag name.
    pub tag: String,
    /// Offset.
    pub offset: Vec3,
    /// Field-of-view offset.
    pub fov_offset: PresentationFovOffset,
}

/// Quake III grapple cable (`SimulationPresentation['q3GrappleCable']`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationGrappleCable {
    /// Owning actor.
    pub owner: ActorId,
    /// Owner origin.
    pub owner_origin: Vec3,
    /// Owner angles.
    pub owner_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Offhand flag.
    pub offhand: bool,
    /// Attached flag.
    pub attached: bool,
    /// Flight effect.
    pub flight: String,
    /// Pull effect.
    pub pull: String,
    /// Hold effect.
    pub hold: String,
    /// Segment length.
    pub segment_length: i64,
}

/// Quake III weapon animation (`SimulationPresentation['q3Weapon']`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationQ3Weapon {
    /// Time in milliseconds.
    pub time_milliseconds: f64,
    /// Torso animation.
    pub torso_animation: i64,
    /// Last fire time in milliseconds.
    pub last_fire_milliseconds: Option<f64>,
    /// Firing flag.
    pub firing: bool,
    /// Horizontal speed.
    pub horizontal_speed: f64,
    /// Bob cycle.
    pub bob_cycle: f64,
    /// Weapon number.
    pub weapon: i64,
}

/// Source family tag for one presentation model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// One presented model (mirrors `SimulationPresentation`).
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationModel {
    /// Owning actor.
    pub actor: ActorId,
    /// Content identity.
    pub content: ContentId,
    /// Source family.
    pub family: PresentationFamily,
    /// Model path.
    pub path: String,
    /// Current frame.
    pub frame: i64,
    /// Previous frame.
    pub old_frame: i64,
    /// Skin number.
    pub skin: i64,
    /// Effects bitmask.
    pub effects: i64,
    /// Render flags.
    pub render_flags: i64,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Scale.
    pub scale: f64,
    /// Visibility flag.
    pub visible: bool,
    /// View-weapon flag.
    pub view_weapon: bool,
    /// Replaces-body marker.
    pub replaces_body: Option<bool>,
    /// Source-client render owner marker.
    pub render_owner: Option<bool>,
    /// Held weapon declaration.
    pub held_weapon: Option<HeldWeaponDeclaration>,
    /// Native held-weapon marker.
    pub native_held_weapon: Option<bool>,
    /// Weapon item id.
    pub weapon_item: Option<String>,
    /// Flare decoration.
    pub flare: Option<PresentationFlare>,
    /// Back-lerp fraction.
    pub back_lerp: Option<f64>,
    /// Skin path override.
    pub skin_path: Option<Option<String>>,
    /// Indexed skin.
    pub indexed_skin: Option<PresentationIndexedSkin>,
    /// Player colors.
    pub player_colors: Option<PresentationPlayerColors>,
    /// Previous origin.
    pub previous_origin: Option<Vec3>,
    /// Model beam.
    pub model_beam: Option<PresentationModelBeam>,
    /// Shader beam.
    pub shader_beam: Option<PresentationShaderBeam>,
    /// Model attachments.
    pub model_attachments: Option<Vec<PresentationModelAttachment>>,
    /// Model anchor.
    pub model_anchor: Option<PresentationModelAnchor>,
    /// Grapple cable.
    pub q3_grapple_cable: Option<PresentationGrappleCable>,
    /// Alpha override.
    pub alpha: Option<f64>,
    /// Quake III weapon animation.
    pub q3_weapon: Option<PresentationQ3Weapon>,
}

/// World text orientation (mirrors `WorldText['orientation']`).
#[derive(Debug, Clone, PartialEq)]
pub enum WorldTextOrientation {
    /// Camera-facing billboard.
    Billboard,
    /// Fixed orientation.
    Fixed {
        /// Angles in degrees.
        angles: Vec3,
    },
}

/// World text font (mirrors `WorldText['font']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorldTextFont {
    /// Classic font.
    Classic,
    /// Selected font.
    Selected,
}

/// World text entry (mirrors `WorldText`).
#[derive(Debug, Clone, PartialEq)]
pub struct WorldText {
    /// Content identity.
    pub content: ContentId,
    /// Text.
    pub text: String,
    /// Origin.
    pub origin: Vec3,
    /// Color.
    pub color: Vec4,
    /// Cell size.
    pub cell_size: f64,
    /// Orientation.
    pub orientation: WorldTextOrientation,
    /// Depth-test flag.
    pub depth_test: bool,
    /// Font.
    pub font: WorldTextFont,
    /// Distance cull factor.
    pub distance_cull_factor: Option<f64>,
}

/// Native camera edition (mirrors `NativeModCameraView['native']['edition']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCameraEdition {
    /// Classic edition.
    Classic,
    /// Rerelease edition.
    Rerelease,
}

/// Native camera block (mirrors `NativeModCameraView['native']`).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeCameraBlock {
    /// Edition.
    pub edition: NativeCameraEdition,
    /// Movement origin.
    pub movement_origin: Vec3,
    /// Render flags.
    pub render_flags: i64,
    /// Position prediction flag.
    pub position_prediction: bool,
    /// Angular prediction flag.
    pub angular_prediction: bool,
    /// Weapon visibility flag.
    pub weapon_visible: bool,
}

/// Native mod camera view (mirrors `NativeModCameraView`: a player view with
/// a native block; classic cameras cannot publish a rerelease damage blend).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModCameraView {
    /// Player view.
    pub view: PlayerView,
    /// Native block.
    pub native: NativeCameraBlock,
}

// ---------------------------------------------------------------------------
// Application network vocabulary
// ---------------------------------------------------------------------------

/// Connection phase (`ApplicationNetworkPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationNetworkPhase {
    /// Exchanging challenges.
    Challenging,
    /// Connecting.
    Connecting,
    /// Loading game state.
    Loading,
    /// Active.
    Active,
    /// Closed.
    Closed,
    /// Rejected.
    Rejected,
}

/// Network role (`ApplicationNetwork['role']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplicationNetworkRole {
    /// Authoritative server.
    Server,
    /// Remote client.
    Client,
}

/// Application network failure (`ApplicationNetwork` rejections).
///
/// The donor's endpoints are `async` and reject with `Error`s; the sync
/// ports return this error with the donor message text intact.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApplicationNetworkError {
    /// Endpoint failure with donor text.
    #[error("{0}")]
    Message(String),
}

/// Demo recording tap (`ApplicationNetwork['serverRecording' | ...]`).
pub trait ApplicationNetworkRecording {
    /// Recording seed.
    fn seed(&self) -> Result<DemoRecordingSeed, ApplicationNetworkError>;
    /// Attach a sink; the returned closure detaches it.
    fn attach(&mut self, sink: Box<dyn DemoRecordingSink>) -> Result<Box<dyn FnOnce() + '_>, ApplicationNetworkError>;
}

/// Application network endpoint (`ApplicationNetwork`).
///
/// Input/render consumers never acquire authority to step a remote server:
/// a remote client sends input and publishes decoded state only.
pub trait ApplicationNetwork {
    /// Server-side demo tap, when the endpoint records one.
    fn server_recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        None
    }
    /// Multiview demo tap, when the endpoint records one.
    fn mvd_recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        None
    }
    /// Client-side demo tap, when the endpoint records one.
    fn recording(&mut self) -> Option<&mut dyn ApplicationNetworkRecording> {
        None
    }
    /// Periodic heartbeat.
    fn heartbeat(&mut self, _now_milliseconds: u64) {}
    /// Endpoint role.
    fn role(&self) -> ApplicationNetworkRole;
    /// Connection phase.
    fn phase(&self) -> ApplicationNetworkPhase;
    /// Selected wire.
    fn wire(&self) -> WireSelection;
    /// Poll before the application's only authoritative simulation step.
    fn poll(&mut self, now_milliseconds: u64) -> Result<Vec<ActorCommand>, ApplicationNetworkError>;
    /// Submit local input (a remote client sends; it never applies input
    /// to a second server).
    fn submit(&mut self, commands: &[ActorCommand], now_milliseconds: u64) -> Result<(), ApplicationNetworkError>;
    /// Publish after the simulation step and after source presentation
    /// events are drained.
    fn publish(
        &mut self,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
        now_milliseconds: u64,
    ) -> Result<(), ApplicationNetworkError>;
    /// Close the endpoint.
    fn close(&mut self);
}

/// Read-only presentation surface for remote clients
/// (`RemotePresentationAccess`).
pub trait RemotePresentationAccess {
    /// Visible world text.
    fn world_text(&self) -> Vec<WorldText>;
    /// HUD state for an actor.
    fn player_ui(&self, actor: &ActorId) -> PlayerUi;
    /// Visible character views.
    fn character_views(&self) -> Vec<Q3CharacterView>;
    /// Run a player command (returns nothing by design).
    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]);
    /// Visible presentations.
    fn presentations(&self) -> Vec<PresentationModel>;
    /// Register a resolved resource.
    fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference);
    /// Camera view for an actor.
    fn player_view(&self, actor: &ActorId) -> PlayerView;
}

/// Admitted application player (`ApplicationNetworkPlayer`).
///
/// `source_entity` is supplied by the source entity registry, never derived
/// from `ActorId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationNetworkPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Bound actor.
    pub actor: ActorId,
    /// Source entity number.
    pub source_entity: u32,
}

/// Quake II application player (`Q2ApplicationPlayer`).
pub type Q2ApplicationPlayer = ApplicationNetworkPlayer;

/// Admission verdict (`Q2ApplicationAdmission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2ApplicationAdmission {
    /// Accepted with a bound player.
    Accepted {
        /// Bound player.
        player: Q2ApplicationPlayer,
    },
    /// Rejected with a reason.
    Rejected {
        /// Reason text.
        reason: String,
    },
}

/// Decoded Quake II game state (`Q2ApplicationGameState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ApplicationGameState {
    /// Parsed server data.
    pub data: Q2ServerData,
    /// Configstrings by index.
    pub config_strings: HashMap<u32, String>,
    /// Spawn baselines by entity number.
    pub baselines: HashMap<u32, EntityState>,
}

/// Writable server event (`Q2ApplicationServerEvent`).
///
/// The donor excludes `frame` and `private` events at the type level; this
/// port enforces the same exclusion in [`Self::new`].
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ApplicationServerEvent {
    /// Event payload.
    pub event: Q2ServerEvent,
    /// Reliable flag.
    pub reliable: bool,
}

impl Q2ApplicationServerEvent {
    /// Whether an event is writable (neither `frame` nor `private`).
    #[must_use]
    pub fn is_writable(event: &Q2ServerEvent) -> bool {
        !matches!(event, Q2ServerEvent::Frame { .. } | Q2ServerEvent::Private { .. })
    }

    /// Build a writable event, rejecting `frame`/`private` payloads.
    #[must_use]
    pub fn new(event: Q2ServerEvent) -> Option<Self> {
        if Self::is_writable(&event) {
            Some(Self { event, reliable: false })
        } else {
            None
        }
    }

    /// Build a writable event with the reliable flag set.
    #[must_use]
    pub fn reliable(event: Q2ServerEvent) -> Option<Self> {
        Self::new(event).map(|mut writable| {
            writable.reliable = true;
            writable
        })
    }
}

/// Raw wire bytes with a reliability flag (donor `rawMessages` rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2RawServerMessage {
    /// Wire bytes.
    pub bytes: Vec<u8>,
    /// Reliable flag.
    pub reliable: bool,
}

/// Server administration surface (`Omit<Q2RconHost, 'reply'>`).
pub trait Q2ApplicationRcon {
    /// Server profile.
    fn profile(&self) -> Q2ServerProfile;
    /// Full rcon password.
    fn rcon_password(&self) -> String;
    /// Limited rcon credential.
    fn limited_rcon(&self) -> Option<Q2LimitedRcon>;
    /// Rerelease rate limiter check.
    fn rcon_rate_allowed(&self, now: u64) -> bool;
    /// Recharge the rerelease rate limiter.
    fn recharge_rcon_rate(&mut self);
    /// Execute an rcon command, streaming output through the sink.
    fn execute_rcon(&mut self, command: &str, limited: bool, output: &mut dyn FnMut(&str));
}

/// Server discovery surface (`Pick<Q2ConnectionlessHost, 'status'|'info'>`).
pub trait Q2ApplicationDiscovery {
    /// Current status.
    fn status(&self) -> Q2Status;
    /// Info row.
    fn info(&self) -> Q2Info;
}

/// Multiview settings (`Q2ApplicationServerHost['mvdSettings']`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MvdSettings {
    /// Multiview enabled flag.
    pub enabled: bool,
    /// Maximum viewers.
    pub max_viewers: u32,
    /// Viewer password.
    pub password: String,
}

/// Quake II application server host (`Q2ApplicationServerHost`).
pub trait Q2ApplicationServerHost {
    /// Whether an address is rejected before admission.
    fn rejects(&self, _address: &NetworkAddress) -> bool {
        false
    }
    /// Administration surface, when the host exposes one.
    fn administration(&mut self) -> Option<&mut dyn Q2ApplicationRcon> {
        None
    }
    /// Master servers, when the host advertises any.
    fn masters(&self) -> Option<Vec<NetworkAddress>> {
        None
    }
    /// Discovery surface, when the host exposes one.
    fn discovery(&self) -> Option<&dyn Q2ApplicationDiscovery> {
        None
    }
    /// Download policy.
    fn downloads(&mut self) -> &mut dyn Q2ApplicationDownloads;
    /// Wire protocol.
    fn protocol(&self) -> Q2ProtocolIdentity;
    /// Server message options.
    fn message_options(&self) -> Q2ServerMessageOptions;
    /// Maximum clients.
    fn max_clients(&self) -> u32;
    /// Native source wire binding (unified composition negotiation stays
    /// separate from this binding).
    fn supports_source_wire(&self) -> WireAdmission;
    /// Observe a completed simulation step.
    fn observe(&mut self, output: &SimulationOutput, events: &[NetworkPresentationEvent]);
    /// Multiview capture for a completed step.
    fn mvd_capture(
        &mut self,
        _output: &SimulationOutput,
        _events: &[NetworkPresentationEvent],
        _servercount: i32,
    ) -> Option<MvdCapture> {
        None
    }
    /// Multiview settings.
    fn mvd_settings(&self) -> Option<Q2MvdSettings> {
        None
    }
    /// Admit a connect request.
    fn admit(&mut self, from: &NetworkAddress, request: &Q2ConnectRequest) -> Q2ApplicationAdmission;
    /// Disconnect a player.
    fn disconnect(&mut self, player: &Q2ApplicationPlayer, reason: &str);
    /// Resolve a carried client after the travel owner admits the new actor.
    fn carried_player(&self, client: &ClientId) -> Q2ApplicationPlayer;
    /// Begin a player session.
    fn begin(&mut self, _player: &Q2ApplicationPlayer) {}
    /// Raw wire messages for a player.
    fn raw_messages(&self, _player: &Q2ApplicationPlayer) -> Vec<Q2RawServerMessage> {
        Vec::new()
    }
    /// Game-import records for local source presentation, before native
    /// peer wire conversion.
    fn source_messages(&self, _player: &Q2ApplicationPlayer) -> Vec<Q2RawServerMessage> {
        Vec::new()
    }
    /// Collect a completed tick for local presentation while retaining the
    /// batch for final wire/demo publication.
    fn observe_progress(&mut self) {}
    /// Fresh local source records since the preceding completed-tick
    /// presentation.
    fn local_messages(&self, _player: &Q2ApplicationPlayer) -> Vec<Q2RawServerMessage> {
        Vec::new()
    }
    /// Game state for a player.
    fn game_state(&self, player: &Q2ApplicationPlayer, protocol: Option<Q2ProtocolIdentity>) -> Q2ApplicationGameState;
    /// Wire frame for a player.
    fn frame(
        &self,
        player: &Q2ApplicationPlayer,
        output: &SimulationOutput,
        protocol: Option<Q2ProtocolIdentity>,
    ) -> Q2WireFrame;
    /// Server events for a player.
    fn events(
        &mut self,
        player: &Q2ApplicationPlayer,
        output: &SimulationOutput,
        events: &[NetworkPresentationEvent],
    ) -> Vec<Q2ApplicationServerEvent>;
    /// Convert a client command into an actor command.
    fn input(&mut self, player: &Q2ApplicationPlayer, command: &Usercmd, sequence: u32) -> Option<ActorCommand>;
    /// Run a client command.
    fn command(&mut self, player: &Q2ApplicationPlayer, name: &str, args: &[String]);
    /// Run raw client command text.
    fn command_text(&mut self, _player: &Q2ApplicationPlayer, _text: &str) {}
    /// Expand a client command alias.
    fn expand_client_command(&self, _text: &str) -> Option<String> {
        None
    }
    /// Update a player userinfo string.
    fn userinfo(&mut self, player: &Q2ApplicationPlayer, value: &str);
    /// Print server text.
    fn print(&mut self, text: &str);
}

/// Client prediction hooks (`Q2ApplicationClientHost['prediction']`).
///
/// Packet command history uses channel sequence/acknowledgement,
/// independently of `serverFrame`.
pub trait Q2ClientPrediction {
    /// A command was sent.
    fn sent(&mut self, sequence: u32, command: &Usercmd, now_milliseconds: u64);
    /// A command was acknowledged.
    fn acknowledged(&mut self, sequence: u32, now_milliseconds: u64);
}

/// Quake II application client host (`Q2ApplicationClientHost`).
pub trait Q2ApplicationClientHost {
    /// Handle server data (donor async; resolves inline here).
    fn server_data(&mut self, _data: &Q2ServerData, _assert_current: &dyn Fn()) {}
    /// Client downloads, when the host accepts them.
    fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads> {
        None
    }
    /// Wire protocol.
    fn protocol(&self) -> Q2ProtocolIdentity;
    /// Server message options.
    fn message_options(&self) -> Q2ServerMessageOptions;
    /// Client userinfo string.
    fn userinfo(&self) -> String;
    /// Opaque platform-issued identity; absence is anonymous and never a
    /// LAN player id.
    fn social_id(&self) -> Option<String> {
        None
    }
    /// Prediction hooks.
    fn prediction(&mut self) -> Option<&mut dyn Q2ClientPrediction> {
        None
    }
    /// Resolve map/model/sound references using mounted content (donor
    /// async; resolves inline here).
    fn game_state(&mut self, state: &Q2ApplicationGameState);
    /// Publish decoded state to a read-only presentation owner.
    fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64);
    /// Handle raw server records.
    fn records(&mut self, records: &[Q2ServerRecord]);
    /// Convert an actor command into a wire command.
    fn command(&self, command: &ActorCommand) -> Usercmd;
    /// Handle a disconnect.
    fn disconnected(&mut self, reason: &str);
    /// Print client text.
    fn print(&mut self, text: &str);
}

/// Quake II server network options (`Q2ServerNetworkOptions`).
pub struct Q2ServerNetworkOptions<T, H> {
    /// Datagram transport.
    pub transport: T,
    /// Server host.
    pub host: H,
    /// Random source.
    pub random: Box<dyn FnMut() -> f64>,
    /// Idle timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
}

/// Quake II client network options (`Q2ClientNetworkOptions`).
pub struct Q2ClientNetworkOptions<T, H>
where
    T: DatagramTransport,
{
    /// Datagram transport.
    pub transport: T,
    /// Remote address.
    pub remote: T::Address,
    /// Client host.
    pub host: H,
    /// Client port id.
    pub qport: u32,
    /// Idle timeout in milliseconds.
    pub timeout_milliseconds: Option<u64>,
}

/// Game callback failure (`Q2GameCallbackError`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2GameCallbackError {
    /// The callback panicked; Rust callbacks cannot throw, so a panic is
    /// the only failure this wrapper can observe.
    #[error("Q2 game callback failed: {0}")]
    Failed(String),
}

/// Run a game callback, wrapping a panic as [`Q2GameCallbackError`]
/// (`q2GameCallback`).
pub fn q2_game_callback<T>(callback: impl FnOnce() -> T) -> Result<T, Q2GameCallbackError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(value) => Ok(value),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_else(|| "unknown panic".to_string());
            Err(Q2GameCallbackError::Failed(message))
        }
    }
}

/// Leased self-referential connection cell.
///
/// `qa-net`'s Q3 connections borrow their bindings `&mut`, so an endpoint
/// cannot own both pieces directly. The cell owns the bindings adapter on
/// the heap and leases it to one connection at a time:
///
/// - construction leaks the adapter box and keeps the raw pointer;
/// - [`build`](Self::build) drops any live connection before re-leasing;
/// - `Drop` drops the connection first, then reclaims the box.
///
/// The adapter itself must be `'static` (shared state travels through
/// `Rc` handles, never borrows), and callers must never touch the adapter
/// except through [`build`](Self::build); all live state stays reachable
/// through the shared handles. The raw pointer makes the cell `!Send` and
/// `!Sync`, pinning endpoints to one thread like the donor event loop.
pub struct Q3ConnectionCell<B, C> {
    adapter: *mut B,
    connection: Option<C>,
}

impl<B, C> Q3ConnectionCell<B, C> {
    /// Own an adapter with no live connection.
    pub fn new(adapter: B) -> Self {
        Self {
            adapter: Box::into_raw(Box::new(adapter)),
            connection: None,
        }
    }

    /// Drop any live connection, then build a fresh one over the adapter.
    pub fn build(&mut self, make: impl FnOnce(&mut B) -> C) {
        self.connection = None;
        // SAFETY: the old connection (the only outstanding lease) was just
        // dropped, and the pointer is uniquely owned, so re-leasing is
        // exclusive. The adapter outlives the cell (see `Drop`).
        let adapter = unsafe { &mut *self.adapter };
        self.connection = Some(make(adapter));
    }

    /// Borrow the live connection, if any.
    #[must_use]
    pub fn connection(&self) -> Option<&C> {
        self.connection.as_ref()
    }

    /// Mutably borrow the live connection, if any.
    pub fn connection_mut(&mut self) -> Option<&mut C> {
        self.connection.as_mut()
    }

    /// Drop the live connection, if any.
    pub fn clear(&mut self) {
        self.connection = None;
    }
}

impl<B, C> Drop for Q3ConnectionCell<B, C> {
    fn drop(&mut self) {
        self.connection = None;
        // SAFETY: the connection (the only lease) is gone and the pointer
        // came from `Box::into_raw` exactly once, so reclaiming is sound.
        unsafe {
            drop(Box::from_raw(self.adapter));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writable_events_exclude_frame_and_private() {
        assert!(Q2ApplicationServerEvent::new(Q2ServerEvent::Nop).is_some());
        assert!(Q2ApplicationServerEvent::reliable(Q2ServerEvent::Disconnect).is_some_and(|event| event.reliable));
    }

    #[test]
    fn q2_game_callback_passes_values_and_wraps_panics() {
        assert_eq!(q2_game_callback(|| 7).expect("value"), 7);
        let error = q2_game_callback(|| -> i32 { panic!("boom") }).expect_err("panic");
        assert_eq!(error, Q2GameCallbackError::Failed("boom".to_string()));
    }
}
