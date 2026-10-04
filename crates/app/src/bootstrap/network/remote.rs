//! Quake II remote presentation.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/network/remote.ts`
//! (`Q2RemotePresentation`, `Q2RemotePresentationOptions`,
//! `q2RemoteEntityBounds`). Decoded source records are presentation state;
//! this owner has no `Simulation` or combat table. The donor's asynchronous
//! content loads resolve inline, preserving the donor's receive order, and
//! the donor's retirement guards collapse onto download-revision checks
//! around each host call.
//!
//! Sibling homes (host-seam surface over live siblings):
//! [`q2_remote_view`](super::q2_remote_view) (`q2RemoteViewHeight`,
//! `q2RemoteViewPosition`, `q2RemoteBodyBounds`, `q2RemoteCommand`,
//! `Q2RereleaseViewHeight`, `q2RereleaseViewContinuous`) behind
//! [`Q2RemoteHost`]'s view methods and the [`Q2RereleaseViewHeight`]
//! seam trait (the ported struct keeps host-owned smoothing state);
//! [`q2_mvd_layout`](super::q2_mvd_presentation::q2_mvd_layout) plus
//! [`Q2MvdVisibility`](super::q2_mvd_presentation::Q2MvdVisibility)
//! (`q2-mvd-presentation.ts`) behind [`Q2RemoteHost::mvd_layout`] and
//! [`Q2RemoteHost::mvd_visibility`];
//! [`SelectedMovementPrediction`](crate::bootstrap::simulation::prediction::runtime::SelectedMovementPrediction)
//! plus [`movement_profile`](crate::bootstrap::simulation::players::movement_profile)
//! and [`movement_origin`](crate::bootstrap::simulation::players::movement_origin)
//! (`prediction.ts`, `players.ts`) behind [`Q2RemotePredictor`],
//! [`Q2RemoteHost::movement_profile_kind`], and the [`Q2PredictionBase`] /
//! [`Q2PredictedPlayer`] host-local shapes;
//! [`Q2MvdPresentation`](super::q2_demo::Q2MvdPresentation) (`q2-demo.ts`)
//! behind the [`Q2RemotePresentationMvd`] binding. The `q2WeaponStatus` helper
//! ([`arsenal::weapon_status`](crate::bootstrap::simulation::arsenal::weapon_status))
//! is mirrored locally as [`q2_weapon_status`] because this path reads
//! the `network::types` weapon table and [`WeaponStatus`] instead of the
//! arsenal HUD status.
//!
//! [`Q2DownloadReceiver`](super::q2_downloads::Q2DownloadReceiver) borrows
//! its mounts and sinks, so the host owns the receiver and this
//! presentation reaches it through [`Q2RemoteHost::downloads`] and
//! [`Q2RemoteHost::download_revision`]; the donor's `refreshDownloads` and
//! `downloadPermission` options live with that host-owned receiver. The
//! lane traits ([`Q2ApplicationClientHost`],
//! [`RemotePresentationAccess`]) are infallible, so donor `throw` paths
//! panic with the donor's message. Published snapshots use
//! [`SimulationOutput`](qa_world::session::SimulationOutput); the donor's
//! `configurations` and `scene` blocks have no `qa-world` home, so they
//! travel beside the output as [`Q2RemoteSceneOutput`].

use std::cell::RefCell;
use std::collections::HashMap;

use qa_content::contract::{
    ArmorState, ContentId, InventoryEntry, PoweredProtectionState, ProviderReference, RegularArmorState,
    ResolvedResourceReference,
};
use qa_content::q2::foundation::host::{Q2Edition, Q2EffectEvent};
use qa_content::q2::foundation::items::q2_base_weapon_display_name;
use qa_content::q2::foundation::monsters::ai::angles_vectors;
use qa_content::q2::foundation::monsters::muzzle::muzzle_offset;
use qa_content::q2::foundation::weapons::definitions::base_weapons;
use qa_content::q2::foundation::weapons::types::{Q2WeaponDefinition, Q2WeaponEvent};
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId, SeatId};
use qa_core::math::{Bounds, Vec3, Vec4};
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_net::common::commands::{ActorCommand, UserCommand};
use qa_net::common::hash::md4_block_checksum;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2::Usercmd;
use qa_net::q2_adapters::{
    to_q2_command, to_q2_player, to_q2_rerelease_command, to_q2_rerelease_player, Q2Command, Q2Player,
    Q2RereleasePlayerState, Q2RereleaseUserCommand, Q2UserCommand, Q2Vec3,
};
use qa_net::q2_net::{
    negotiated_r1q2_protocol, Q2ServerData, Q2ServerEvent, Q2ServerMessageOptions, Q2ServerRecord, Q2WireFrame,
};
use qa_net::q2_solid::{q2_solid_encoding, unpack_q2_solid, Q2SolidEncoding};
use qa_net::q2_svc::MvdVisibility;
use qa_net::q2_variants::{MvdProfile, Q2ProFeatures};
use qa_world::body::BodyState;
use qa_world::save::shared::{CharacterSelection, ProviderRef};
use qa_world::session::{
    EngineSession, SessionClient, SimulationOutput, SnapshotActor, SnapshotBody, SnapshotInventory,
};

use super::client_download_policy::ClientDownloadProgress;
use super::q2_downloads::Q2ApplicationClientDownloads;
use super::q2_effects::{q2_beam_from_wire, q2_effect_from_wire};
use super::q2_layout::{q2_application_layout, Q2ApplicationLayout};
use super::remote_world::{lerp_angles, lerp_vec, provider_id, snapshot_entry, vec3, RemoteWorldContent, ZERO};
use super::types::{
    ApplicationNetworkPlayer, ArsenalWarning, PlayerAmmo, PlayerArsenalItem, PlayerArsenalKind, PlayerUi, PlayerView,
    PresentationFamily, PresentationModel, Q2ApplicationClientHost, Q2ApplicationGameState, Q2ClientPrediction,
    RemotePresentationAccess, WeaponAmmoStatus, WeaponStatus, WorldText,
};
use crate::bootstrap::demo_recording::{Q2ProtocolIdentity, R1Q2Revision};
use crate::bootstrap::q2_damage_blend::interpolate_q2_damage_blend;
use crate::persistence::recipe::ExecutableRecipe;

/// Solid marker for brush-model entities (donor `entity.solid === 31`).
const BRUSH_SOLID: u32 = 31;

/// Model index selecting the player-skin path (donor `modelindex === 255`).
const PLAYER_SKIN_MODEL: u16 = 255;

/// Remote-armor item id (donor `q2:remote-armor`).
const REMOTE_ARMOR_ITEM: &str = "q2:remote-armor";

/// Entity bounds for a wire solid (donor `q2RemoteEntityBounds`).
#[must_use]
pub fn q2_remote_entity_bounds(solid: u32, long_solid: bool) -> Bounds {
    unpack_q2_solid(
        solid,
        if long_solid {
            Q2SolidEncoding::R1q2
        } else {
            Q2SolidEncoding::Short
        },
    )
}

/// Loaded-content surface the Q2 presentation reads.
pub trait Q2RemoteContent {
    /// Recipe the content was loaded from.
    fn recipe(&self) -> &ExecutableRecipe;
    /// World geometry identity (donor `content.world`).
    fn world_geometry(&self) -> &str;
    /// Requested map-geometry path (donor `recipe.map.geometry.requestedPath`).
    fn map_geometry_path(&self) -> &str;
    /// Map-geometry bytes (donor `mounts.read(recipe.map.geometry)`).
    fn map_bytes(&self) -> Vec<u8>;
}

/// Scene queries the Q2 layer needs (donor `scene` uses in `linkSolids`).
pub trait Q2RemoteScene {
    /// Unlink a previously linked actor.
    fn unlink_actor(&mut self, actor: &ActorId);
    /// Collision bounds of a brush model.
    fn model_bounds(&self, model: u32) -> Bounds;
    /// Link a solid body.
    fn link_solid(&mut self, link: Q2SolidLink);
}

/// One linked solid (donor `scene.link` call in `linkSolids`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SolidLink {
    /// Linked actor.
    pub actor: ActorId,
    /// Linked body state (bounds already resolved to the brush or body bounds).
    pub state: BodyState,
    /// Link count (donor `frame.serverFrame`).
    pub link_count: i32,
    /// Absolute bounds (local bounds expanded by one unit per side, cubed when rotated).
    pub absolute_bounds: Bounds,
    /// Brush model number, when the solid is a brush (`None` selects the box shape).
    pub model: Option<u32>,
    /// Whether the solid follows a player (donor `monster`, true for non-brush solids).
    pub monster: bool,
}

/// Cinematic media owner (donor `Q2RemotePresentationOptions['cinematic']`).
pub trait Q2RemoteCinematic {
    /// Start cinematic media; on media end the host sends
    /// `nextserver {server_count}\n` while its download revision still
    /// equals `revision` (donor `start` with the `ended` closure, including
    /// its completed/current guards).
    fn start(&mut self, name: &str, server_count: i32, revision: u64);
    /// Stop cinematic media (donor `stop`).
    fn stop(&mut self);
}

/// Rerelease stance smoother (donor `Q2RereleaseViewHeight`).
pub trait Q2RereleaseViewHeight {
    /// Forget smoother state (donor `reset`).
    fn reset(&mut self);
    /// Sample the smoothed height (donor `sample`).
    fn sample(&mut self, height: f64, time_milliseconds: f64) -> f64;
}

/// Camera position (donor `q2RemoteViewPosition` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ViewPosition {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera view height.
    pub view_height: f64,
}

/// Movement-profile family (projection of donor `MovementProfile['kind']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2MovementProfileKind {
    /// Classic Q2 prediction profile.
    Q2Classic,
    /// Rerelease Q2 prediction profile.
    Q2Rerelease,
    /// Any non-Q2 profile (presentation skips prediction).
    Other,
}

/// Per-step movement tunables (donor `SelectedMovementPrediction` profile getters).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MovementTunables {
    /// Air acceleration (donor `airAccelerate` config value).
    pub air_accelerate: f64,
    /// Strafejump hack negotiated (donor `strafejumpHack`).
    pub strafejump_hack: bool,
    /// N64 physics selected (donor `n64Physics`).
    pub n64_physics: bool,
}

/// Client-prediction snapshot (donor `MovementPredictionSnapshot` projection).
///
/// The host's snapshot assembly fills the donor's literal constants around
/// these fields: environment `flight`/`haste`/`invulnerable` false with
/// gravity multiplier 1, arsenal state `q2` with state 0, no pending weapon,
/// zero machinegun shots, zero grenade time, and no grenade blowup,
/// animation state `q2` with zeroed frame/priority and no duck or run, and
/// null contact and Q3 arsenal.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PredictionBase {
    /// Acknowledged command sequence (donor `packetAcknowledged`).
    pub sequence: u32,
    /// Command time in milliseconds (donor `serverFrame * frameMilliseconds`).
    pub command_time_ms: f64,
    /// Native movement state and view (donor `state`, `viewAngles`, `viewOffset`).
    pub player: Q2Player,
    /// Player view height (donor `q2RemoteViewHeight`).
    pub view_height: f64,
    /// Player body bounds (donor `q2RemoteBodyBounds`).
    pub bounds: Bounds,
    /// Health (donor `native.stats[1]`).
    pub health: f64,
    /// Active weapon item id.
    pub active_weapon: Option<String>,
    /// Ammunition inventory.
    pub ammo: Vec<InventoryEntry>,
    /// Weapon view-model frame (donor `native.gunFrame`).
    pub gun_frame: i32,
}

/// Prediction result status (donor `MovementPredictionResult['status']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2PredictionStatus {
    /// Fresh prediction.
    Predicted,
    /// Unchanged prediction.
    Unchanged,
    /// Prediction disabled.
    Disabled,
    /// Prediction history exhausted.
    HistoryExhausted,
}

/// Predicted player state (donor `MovementPredictionResult` projection).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PredictedPlayer {
    /// Prediction status.
    pub status: Q2PredictionStatus,
    /// Predicted origin (donor `movementOrigin(player.state)`).
    pub origin: Vec3,
    /// Predicted view angles.
    pub view_angles: Vec3,
    /// Predicted view height.
    pub view_height: f64,
    /// Predicted body bounds.
    pub bounds: Bounds,
}

/// Trace-hit projection for brush classification (donor `TraceHit` uses in `isBrush`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2BrushHit {
    /// World hit (always a brush).
    World,
    /// Actor hit (a brush when the actor's current entity solid is 31).
    Actor(ActorId),
    /// Any other hit kind (never a brush).
    Other,
}

/// Predictor construction seed (donor `SelectedMovementPrediction` options).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2MovementSeed<'recipe> {
    /// Owned prediction actor.
    pub actor: OwnedActor,
    /// Local seat.
    pub seat: SeatId,
    /// Executable recipe (donor `recipe`, read during construction).
    pub recipe: &'recipe ExecutableRecipe,
    /// Selected Q2 movement profile kind.
    pub profile: Q2MovementProfileKind,
    /// Initial snapshot.
    pub base: Q2PredictionBase,
}

/// Movement predictor behind the replica (donor `SelectedMovementPrediction`).
///
/// The host binds the recipe, scene, standing bounds (`-16/-16/-24` to
/// `16/16/32`), and standing view height (`22`) from the seed; per-step
/// tunables arrive on every call because the donor reads them through
/// getters.
pub trait Q2RemotePredictor {
    /// Record a sent command (donor `submit`).
    fn submit(&mut self, sequence: u32, time_milliseconds: u64, command: Q2Command, tunables: &Q2MovementTunables);
    /// Receive a server snapshot (donor `receive`).
    fn receive(&mut self, base: &Q2PredictionBase, tunables: &Q2MovementTunables);
    /// Replay pending moves (donor `replay`).
    fn replay(&mut self, is_brush: &dyn Fn(&Q2BrushHit) -> bool, tunables: &Q2MovementTunables) -> Q2PredictedPlayer;
}

/// Host callbacks behind [`Q2RemotePresentationOptions`].
pub trait Q2RemoteHost {
    /// Loaded content handle.
    type Content: Q2RemoteContent;
    /// Scene queries handle.
    type Scene: Q2RemoteScene;
    /// Movement predictor.
    type Predictor: Q2RemotePredictor;
    /// Rerelease stance smoother.
    type ViewHeight: Q2RereleaseViewHeight;
    /// Build scene queries for loaded content.
    fn build_scene(content: &Self::Content) -> Self::Scene;
    /// Build the stance smoother.
    fn build_view_height(&mut self) -> Self::ViewHeight;
    /// Build the movement predictor.
    fn build_predictor(&mut self, seed: &Q2MovementSeed<'_>) -> Self::Predictor;
    /// Send a console command to the server.
    fn send_command(&mut self, text: &str);
    /// Print server text.
    fn print(&mut self, text: &str);
    /// Publish a sampled output.
    fn publish(&mut self, output: &SimulationOutput);
    /// Handle a disconnect.
    fn disconnected(&mut self, reason: &str);
    /// Mint the next actor generation for a slot.
    fn next_generation(&self, slot: u32) -> u32;
    /// Presentation clock override, when the host drives time.
    fn presentation_time(&self) -> Option<u64> {
        None
    }
    /// Client userinfo string.
    fn userinfo(&self) -> String;
    /// Load content for admitted game state; `None` keeps the world content
    /// (donor `loadContent`, `None` when the option is unbound).
    fn load_content(&mut self, state: &Q2ApplicationGameState) -> Option<Self::Content>;
    /// Prepare server content for downloads (donor `prepareServerData`).
    fn prepare_server_data(&mut self, data: &Q2ServerData);
    /// Refresh host-owned download roots after content replacement (donor
    /// `downloadContent` catalog/product/mounts merge in `gameState`).
    fn note_content_replaced(&mut self);
    /// Download receiver, when the session lane binds one.
    fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads>;
    /// Download revision counter (donor `downloads.revision`).
    fn download_revision(&self) -> u64;
    /// Cinematic media owner, when the session lane binds one.
    fn cinematic(&mut self) -> Option<&mut dyn Q2RemoteCinematic>;
    /// Camera height for native player state (donor `q2RemoteViewHeight`).
    fn view_height(&self, player: &Q2Player) -> f64;
    /// Camera position for native player state (donor `q2RemoteViewPosition`).
    fn view_position(&self, player: &Q2Player, origin: Vec3, offset: Vec3, view_height: f64) -> Q2ViewPosition;
    /// Body bounds for native player state (donor `q2RemoteBodyBounds`).
    fn body_bounds(&self, player: &Q2Player) -> Bounds;
    /// Encode a wire command for native player state (donor `q2RemoteCommand`).
    fn remote_command(&self, command: &Q2Command, player: &Q2Player) -> Usercmd;
    /// Rerelease frame continuity (donor `q2RereleaseViewContinuous`).
    fn view_continuous(
        &self,
        previous: &Q2RereleasePlayerState,
        current: &Q2RereleasePlayerState,
        previous_frame: i32,
        current_frame: i32,
        event: u8,
    ) -> bool;
    /// Recorded-source configstring layout (donor `q2MvdLayout`).
    fn mvd_layout(&self, profile: &MvdProfile) -> Q2ApplicationLayout;
    /// Recorded-source BSP visibility (donor `q2MvdVisibility`).
    fn mvd_visibility(&mut self, profile: &MvdProfile) -> Box<dyn MvdVisibility>;
    /// Movement-profile family for a recipe (donor `movementProfile` kind).
    fn movement_profile_kind(&self, recipe: &ExecutableRecipe) -> Q2MovementProfileKind;
}

/// Quake II remote presentation options (donor `Q2RemotePresentationOptions`).
pub struct Q2RemotePresentationOptions<H: Q2RemoteHost> {
    /// Identity authority.
    pub identity: IdentityOwner,
    /// Engine session.
    pub session: EngineSession,
    /// Bound client.
    pub client: SessionClient,
    /// Local seat.
    pub seat: SeatId,
    /// Preloaded content, if any.
    pub content: Option<H::Content>,
    /// Offered wire protocol.
    pub protocol: Q2ProtocolIdentity,
    /// Recorded-source profile, when presenting a demo.
    pub recorded_profile: Option<MvdProfile>,
    /// Host callbacks.
    pub host: H,
}

/// Recorded-source presentation binding (mirrors donor `Q2MvdPresentation`
/// from `./q2-demo.ts`). Demo playback pairs the visibility handle with
/// [`Q2RemotePresentation::select_recorded_view`] (donor `selectView`).
pub struct Q2RemotePresentationMvd {
    /// Source-BSP visibility (donor `visibility`).
    pub visibility: Box<dyn MvdVisibility>,
}

/// Q2 print level (donor `print` event `level`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2PrintLevel {
    /// Chat print.
    Chat,
}

/// Q2 loop-sound lifecycle (donor `sound` event `loop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2SoundLoop {
    /// Stop a looping sound.
    Stop,
    /// Start a looping sound.
    Start,
    /// One-shot sound.
    Once,
}

/// Presentation event payload (donor `SimulationPresentationEvent['event']`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2RemoteEvent {
    /// Player userinfo row (donor `userinfo`).
    PlayerUserinfo {
        /// Row actor.
        actor: ActorId,
        /// Player slot.
        slot: u32,
        /// Player name.
        name: String,
        /// Player skin.
        skin: String,
    },
    /// Player print (donor `print`).
    PlayerPrint {
        /// Target actor.
        target: ActorId,
        /// Print level.
        level: Q2PrintLevel,
        /// Print text.
        text: String,
    },
    /// Entity event number (donor `entity-event`).
    EntityEvent {
        /// Source actor.
        actor: ActorId,
        /// Event number.
        event: u8,
    },
    /// Positional sound (donor `sound`).
    Sound {
        /// Source actor (`None` for entity zero).
        actor: Option<ActorId>,
        /// Sound origin.
        origin: Vec3,
        /// Sound path.
        path: String,
        /// Sound channel.
        channel: u8,
        /// Sound volume.
        volume: f64,
        /// Sound attenuation.
        attenuation: f64,
        /// Reliability flag.
        reliable: bool,
        /// Loop lifecycle.
        loop_state: Q2SoundLoop,
    },
    /// Center print (donor `centerprint`).
    Centerprint {
        /// Target actor.
        actor: ActorId,
        /// Print text.
        text: String,
    },
    /// Translated temporary entity (donor `q2EffectFromWire` result).
    Effect(Q2EffectEvent),
    /// Translated beam trail (donor `q2BeamFromWire` result).
    Beam(Q2WeaponEvent),
    /// Player muzzle flash (donor `muzzleflash`).
    Muzzleflash {
        /// Source actor.
        actor: ActorId,
        /// Flash number.
        flash: i32,
        /// Silenced flag.
        silenced: bool,
    },
    /// Monster muzzle flash (donor `monster-muzzleflash`).
    MonsterMuzzleflash {
        /// Source actor.
        actor: ActorId,
        /// Flash number.
        flash: i32,
        /// Muzzle origin.
        origin: Vec3,
        /// Muzzle direction.
        direction: Vec3,
    },
}

/// Presentation event envelope (donor `SimulationPresentationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RemotePresentationEvent {
    /// Event sequence.
    pub sequence: u64,
    /// Content identity.
    pub content: ContentId,
    /// Presentation time in seconds.
    pub seconds: f64,
    /// Source entity number, if any.
    pub source_entity: Option<u32>,
    /// Event payload.
    pub event: Q2RemoteEvent,
}

impl Q2RemotePresentationEvent {
    /// Donor event family tag (`q2`, `q2-player`, or `q2-weapon`).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match &self.event {
            Q2RemoteEvent::PlayerUserinfo { .. } | Q2RemoteEvent::PlayerPrint { .. } => "q2-player",
            Q2RemoteEvent::Beam(_) | Q2RemoteEvent::Muzzleflash { .. } => "q2-weapon",
            _ => "q2",
        }
    }
}

/// Actor configuration beside the snapshot (donor `configurations` block).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RemoteConfiguration {
    /// Configured actor.
    pub actor: ActorId,
    /// Movement provider.
    pub movement: ProviderRef,
    /// Character selection.
    pub character: CharacterSelection,
    /// Weapon providers.
    pub weapons: Vec<ProviderRef>,
    /// Inventory provider.
    pub inventory: ProviderRef,
}

/// Scene light style beside the snapshot (donor `lightStyles` block).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RemoteLightStyle {
    /// Light style number.
    pub style: u32,
    /// Style color.
    pub rgb: Vec3,
    /// White intensity.
    pub white: f64,
}

/// Scene blocks beside the snapshot (donor `configurations`/`scene` blocks).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RemoteSceneOutput {
    /// World geometry resource.
    pub world_resource: crate::persistence::recipe::ResolvedResourceReference,
    /// World geometry identity.
    pub world_geometry: String,
    /// Actor configurations.
    pub configurations: Vec<Q2RemoteConfiguration>,
    /// Scene light styles.
    pub light_styles: Vec<Q2RemoteLightStyle>,
    /// Frame area bits.
    pub area_bits: Vec<u8>,
}

/// Quake II remote presentation (donor `Q2RemotePresentation`).
pub struct Q2RemotePresentation<H: Q2RemoteHost> {
    options: Q2RemotePresentationOptions<H>,
    selected_protocol: Q2ProtocolIdentity,
    strafejump_hack: bool,
    extended_game: bool,
    message_options: Q2ServerMessageOptions,
    layout: Q2ApplicationLayout,
    actors: RefCell<HashMap<u32, ActorId>>,
    configs: HashMap<u32, String>,
    resources: HashMap<String, ResolvedResourceReference>,
    events: Vec<Q2RemotePresentationEvent>,
    current: Option<Q2WireFrame>,
    previous_frame: Option<Q2WireFrame>,
    received_at: u64,
    fraction: f64,
    view_time: f64,
    view_height: RefCell<H::ViewHeight>,
    current_player: Option<ApplicationNetworkPlayer>,
    published: Option<SimulationOutput>,
    published_scene: Option<Q2RemoteSceneOutput>,
    event_sequence: u64,
    frame_milliseconds: f64,
    inventory: Vec<i16>,
    last_records: Vec<Q2ServerRecord>,
    layout_text: String,
    world: RemoteWorldContent<H::Content, H::Scene, ()>,
    prediction_owner: Option<H::Predictor>,
    predicted: Option<Q2PredictedPlayer>,
    packet_acknowledged: u32,
}

fn contract_vec3(value: &Q2Vec3) -> Vec3 {
    Vec3 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
    }
}

fn vec4(x: f64, y: f64, z: f64, w: f64) -> Vec4 {
    Vec4 {
        x: x as f32,
        y: y as f32,
        z: z as f32,
        w: w as f32,
    }
}

/// Parse a configstring number (donor `Number(...)`, `NaN` when missing).
fn config_number(value: Option<&str>) -> f64 {
    value.map_or(f64::NAN, |text| text.parse().unwrap_or(f64::NAN))
}

/// Whether a configstring number is an integer (donor `Number.isInteger`).
fn is_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0
}

/// Weapon display label (donor `displayName` in `weapon-status.ts`).
fn display_name(value: &str) -> String {
    let mut label = String::with_capacity(value.len());
    let mut boundary = true;
    for character in value.chars() {
        if character == '_' || character == '-' {
            label.push(' ');
            boundary = true;
        } else if boundary && character.is_alphanumeric() {
            for upper in character.to_uppercase() {
                label.push(upper);
            }
            boundary = false;
        } else {
            label.push(character);
            boundary = !character.is_alphanumeric();
        }
    }
    label
}

/// Weapon HUD status (mirrors donor `q2WeaponStatus`; the donor's count
/// closure ignores its argument and always reads the ammo stat, so the
/// count arrives evaluated).
fn q2_weapon_status(
    definition: Option<&Q2WeaponDefinition>,
    count: f64,
    source: ProviderReference,
) -> Option<WeaponStatus> {
    let definition = definition?;
    Some(WeaponStatus {
        source,
        item: definition.item.clone(),
        label: q2_base_weapon_display_name(&definition.item).unwrap_or_else(|| display_name(&definition.name)),
        ammo: match &definition.ammo {
            None => WeaponAmmoStatus::Unmetered,
            Some(ammo) => WeaponAmmoStatus::finite(
                ammo.clone(),
                count,
                count >= f64::from(definition.quantity),
                count <= f64::from(definition.warning),
            ),
        },
    })
}

fn net_protocol(protocol: Q2ProtocolIdentity) -> ProtocolIdentity {
    match protocol {
        Q2ProtocolIdentity::Classic => ProtocolIdentity::Q2Classic,
        Q2ProtocolIdentity::R1Q2 { revision } => ProtocolIdentity::Q2R1q2 {
            revision: u32::from(revision.revision()),
        },
        Q2ProtocolIdentity::Q2Pro { revision } => ProtocolIdentity::Q2Q2pro {
            revision: u32::from(revision.revision()),
        },
        Q2ProtocolIdentity::Rerelease => ProtocolIdentity::Q2Rerelease,
        Q2ProtocolIdentity::Kex => ProtocolIdentity::Q2Kex,
        Q2ProtocolIdentity::KexDemo => ProtocolIdentity::Q2KexDemo,
    }
}

fn is_rerelease_family(protocol: Q2ProtocolIdentity) -> bool {
    matches!(protocol, Q2ProtocolIdentity::Rerelease | Q2ProtocolIdentity::Kex)
}

fn negotiated_revision(offered: u32, reported: Option<u16>) -> R1Q2Revision {
    let revision = negotiated_r1q2_protocol(offered, reported.map(i64::from)).unwrap_or_else(|error| panic!("{error}"));
    match revision {
        1903 => R1Q2Revision::R1903,
        1904 => R1Q2Revision::R1904,
        1905 => R1Q2Revision::R1905,
        _ => panic!("Unsupported R1Q2 server revision"),
    }
}

impl<H: Q2RemoteHost> Q2RemotePresentation<H> {
    /// Build a presentation over its options (donor `constructor`).
    pub fn new(mut options: Q2RemotePresentationOptions<H>) -> Self {
        if options.protocol == Q2ProtocolIdentity::KexDemo {
            panic!("KEX native live transport is not bound");
        }
        let layout = match &options.recorded_profile {
            None => q2_application_layout(net_protocol(options.protocol))
                .expect("Q2 protocol always selects an application layout"),
            Some(profile) => options.host.mvd_layout(profile),
        };
        let message_options = Q2ServerMessageOptions {
            max_config_strings: layout.max_config_strings as u16,
            inventory_slots: 256,
            ..Q2ServerMessageOptions::default()
        };
        let view_height = RefCell::new(options.host.build_view_height());
        let world = RemoteWorldContent::new(options.content.take(), |content: &H::Content, _: Option<&()>| {
            H::build_scene(content)
        });
        Self {
            selected_protocol: options.protocol,
            strafejump_hack: false,
            extended_game: false,
            message_options,
            layout,
            actors: RefCell::new(HashMap::new()),
            configs: HashMap::new(),
            resources: HashMap::new(),
            events: Vec::new(),
            current: None,
            previous_frame: None,
            received_at: 0,
            fraction: 1.0,
            view_time: 0.0,
            view_height,
            current_player: None,
            published: None,
            published_scene: None,
            event_sequence: 0,
            frame_milliseconds: 100.0,
            inventory: Vec::new(),
            last_records: Vec::new(),
            layout_text: String::new(),
            world,
            prediction_owner: None,
            predicted: None,
            packet_acknowledged: 0,
            options,
        }
    }

    /// Bound client.
    #[must_use]
    pub fn client(&self) -> &SessionClient {
        &self.options.client
    }

    /// Identity authority.
    #[must_use]
    pub fn identity(&self) -> &IdentityOwner {
        &self.options.identity
    }

    /// Borrow the loaded content, failing before server admission.
    pub fn content(&self) -> &H::Content {
        self.world.content().expect("remote content is loaded")
    }

    /// Map-entities content identity.
    fn map_content(&self) -> ContentId {
        ContentId(self.content().recipe().map.entities.content.clone())
    }

    /// Borrow the scene queries.
    pub fn scene(&mut self) -> Result<&H::Scene, super::remote_world::RemoteWorldError> {
        self.world.scene()
    }

    /// Recorded-source presentation binding (donor `mvdPresentation`).
    pub fn mvd_presentation(&mut self) -> Q2RemotePresentationMvd {
        let profile = self
            .options
            .recorded_profile
            .unwrap_or_else(|| panic!("MVD presentation requires its recorded profile"));
        let visibility = self.options.host.mvd_visibility(&profile);
        Q2RemotePresentationMvd { visibility }
    }

    /// Select a recorded viewpoint (donor `selectRecordedView`).
    pub fn select_recorded_view(&mut self, clientnum: i32) {
        let maximum = config_number(self.configs.get(&self.layout.max_clients).map(String::as_str));
        if clientnum < 0 || !is_integer(maximum) || f64::from(clientnum) >= maximum {
            panic!("Recorded viewpoint is outside its admitted player range");
        }
        let source_entity = (clientnum + 1) as u32;
        if self.current_player.as_ref().map(|player| player.source_entity) != Some(source_entity) {
            self.current = None;
            self.previous_frame = None;
            self.view_height.borrow_mut().reset();
        }
        let actor = self.actor(source_entity);
        self.current_player = Some(ApplicationNetworkPlayer {
            client: self.options.client.id().clone(),
            actor,
            source_entity,
        });
        self.prediction_owner = None;
        self.predicted = None;
    }

    /// Configstring layout (donor `configLayout`).
    #[must_use]
    pub fn config_layout(&self) -> Q2ApplicationLayout {
        self.layout
    }

    /// Current player, if any.
    #[must_use]
    pub fn player(&self) -> Option<ApplicationNetworkPlayer> {
        self.current_player.clone()
    }

    /// Latest published output, if any.
    #[must_use]
    pub fn output(&self) -> Option<&SimulationOutput> {
        self.published.as_ref()
    }

    /// Scene blocks beside the latest output, if any.
    #[must_use]
    pub fn scene_output(&self) -> Option<&Q2RemoteSceneOutput> {
        self.published_scene.as_ref()
    }

    /// Retained native records (donor `sourceRecords`).
    #[must_use]
    pub fn source_records(&self) -> &[Q2ServerRecord] {
        &self.last_records
    }

    /// Native layout text (donor `nativeLayout`).
    #[must_use]
    pub fn native_layout(&self) -> &str {
        &self.layout_text
    }

    /// Whether an actor is a player (donor `isPlayer`).
    #[must_use]
    pub fn is_player(&self, actor: &ActorId) -> bool {
        if self
            .current_player
            .as_ref()
            .is_some_and(|player| player.actor == *actor)
        {
            return true;
        }
        let Some(current) = self.current.as_ref() else {
            return false;
        };
        let maximum = config_number(self.configs.get(&self.layout.max_clients).map(String::as_str));
        if !is_integer(maximum) || maximum < 1.0 || maximum > 256.0 {
            panic!("Remote Q2 frame has no valid advertised client range");
        }
        let maximum = maximum as u32;
        let actors = self.actors.borrow();
        current.entities.iter().any(|entity| {
            let number = u32::from(entity.number);
            number >= 1 && number <= maximum && actors.get(&number) == Some(actor)
        })
    }

    /// Borrow a registered actor without creating it.
    #[must_use]
    pub fn actor_at(&self, number: u32) -> Option<ActorId> {
        self.actors.borrow().get(&number).cloned()
    }

    fn actor(&self, number: u32) -> ActorId {
        if let Some(found) = self.actor_at(number) {
            return found;
        }
        let value = self
            .options
            .identity
            .actor(number, self.options.host.next_generation(number));
        self.actors.borrow_mut().insert(number, value.clone());
        value
    }

    fn push_event(&mut self, event: Q2RemoteEvent, source_entity: Option<u32>, seconds: f64) {
        let sequence = self.event_sequence;
        self.event_sequence += 1;
        let content = self.map_content();
        self.events.push(Q2RemotePresentationEvent {
            sequence,
            content,
            seconds,
            source_entity,
            event,
        });
    }

    fn player_info(&mut self, index: u32, value: &str, seconds: f64) {
        if index < self.layout.player_skins || index >= self.layout.player_skins + 256 {
            return;
        }
        let slot = index - self.layout.player_skins;
        let (name, skin) = match value.find('\\') {
            None => (value.to_string(), String::new()),
            Some(split) => (value[..split].to_string(), value[split + 1..].to_string()),
        };
        let actor = self.actor(slot + 1);
        self.push_event(
            Q2RemoteEvent::PlayerUserinfo {
                actor,
                slot,
                name,
                skin,
            },
            Some(slot + 1),
            seconds,
        );
    }

    /// Handle server data (donor `serverData`, resolves inline).
    pub fn server_data(&mut self, data: &Q2ServerData, assert_current: &dyn Fn()) {
        if let Some(cinematic) = self.options.host.cinematic() {
            cinematic.stop();
        }
        let revision = self.options.host.download_revision();
        let assert_live = |host: &H| {
            assert_current();
            if revision != host.download_revision() {
                panic!("Q2 server content preparation was retired");
            }
        };
        self.options.host.prepare_server_data(data);
        assert_live(&self.options.host);
        if data.clientnum() < 0 {
            if self.options.host.cinematic().is_none() {
                panic!("Q2 cinematic serverdata requires the shared media owner");
            }
            self.current = None;
            self.view_height.borrow_mut().reset();
            self.view_time = 0.0;
            self.previous_frame = None;
            self.published = None;
            self.current_player = None;
            self.prediction_owner = None;
            self.predicted = None;
            let levelname = data.levelname().to_string();
            let servercount = data.servercount();
            if let Some(cinematic) = self.options.host.cinematic() {
                cinematic.start(&levelname, servercount, revision);
            }
            assert_live(&self.options.host);
        }
    }

    /// Handle admitted game state (donor `gameState`, resolves inline).
    pub fn game_state(&mut self, state: &Q2ApplicationGameState) {
        let offered = self.options.protocol;
        if let Q2ProtocolIdentity::R1Q2 { revision } = offered {
            let negotiated = negotiated_revision(u32::from(revision.revision()), state.data.r1q2_version());
            self.selected_protocol = Q2ProtocolIdentity::R1Q2 { revision: negotiated };
        }
        self.strafejump_hack = if matches!(offered, Q2ProtocolIdentity::R1Q2 { .. }) {
            matches!(&state.data, Q2ServerData::R1Q2(data) if data.strafejump_hack)
        } else {
            matches!(&state.data, Q2ServerData::Q2Pro(data) if data.strafejump_hack())
        };
        self.extended_game = match offered {
            Q2ProtocolIdentity::Q2Pro { revision } => Q2ProFeatures {
                revision: match &state.data {
                    Q2ServerData::Q2Pro(data) => data.version,
                    _ => revision.revision(),
                },
                flags: match &state.data {
                    Q2ServerData::Q2Pro(data) => data.wire_flags,
                    _ => 0,
                },
            }
            .extensions(),
            _ => false,
        };
        let revision = self.options.host.download_revision();
        let loaded = self.options.host.load_content(state);
        if revision != self.options.host.download_revision() {
            return;
        }
        {
            let candidate = match loaded.as_ref() {
                Some(content) => content,
                None => self.content(),
            };
            let path = state.config_strings.get(&(self.layout.models + 1));
            if path.map(String::as_str) != Some(candidate.map_geometry_path()) {
                panic!(
                    "Q2 server map {} requires application content replacement",
                    path.map_or("<missing>", String::as_str)
                );
            }
            let checksum = state.config_strings.get(&self.layout.map_checksum);
            let map_bytes = candidate.map_bytes();
            if revision != self.options.host.download_revision() {
                return;
            }
            if checksum.is_none()
                || config_number(checksum.map(String::as_str)) as u32 != md4_block_checksum(&map_bytes)
            {
                panic!("Q2 server map checksum differs from mounted content");
            }
        }
        if state.data.clientnum() < 0 {
            panic!("Q2 remote multi-seat/cinematic serverdata requires its source presentation binding");
        }
        match loaded {
            Some(content) => self.world.set_content(content),
            None => self.world.reset_scene(),
        }
        self.options.host.note_content_replaced();
        self.actors.borrow_mut().clear();
        self.configs.clear();
        self.current = None;
        self.view_height.borrow_mut().reset();
        self.view_time = 0.0;
        self.previous_frame = None;
        self.published = None;
        self.prediction_owner = None;
        self.predicted = None;
        self.inventory = Vec::new();
        self.layout_text = String::new();
        self.events.clear();
        for (index, value) in &state.config_strings {
            self.configs.insert(*index, value.clone());
        }
        let source_entity = (state.data.clientnum() + 1) as u32;
        let actor = self.actor(source_entity);
        self.current_player = Some(ApplicationNetworkPlayer {
            client: self.options.client.id().clone(),
            actor,
            source_entity,
        });
        let server_fps = match &state.data {
            Q2ServerData::Rerelease(data) => f64::from(data.server_fps),
            Q2ServerData::Kex(data) => f64::from(data.server_fps),
            _ => 10.0,
        };
        self.frame_milliseconds = 1000.0 / server_fps;
        let configs: Vec<(u32, String)> = self
            .configs
            .iter()
            .map(|(index, value)| (*index, value.clone()))
            .collect();
        for (index, value) in &configs {
            self.player_info(*index, value, 0.0);
        }
    }

    fn require_player(&self, actor: &ActorId) -> (&ApplicationNetworkPlayer, &Q2WireFrame) {
        match (self.current_player.as_ref(), self.current.as_ref()) {
            (Some(player), Some(frame)) if player.actor == *actor => (player, frame),
            _ => panic!("Remote Q2 player has no decoded frame"),
        }
    }

    fn native_player(&self, frame: &Q2WireFrame) -> Q2Player {
        if is_rerelease_family(self.selected_protocol) {
            Q2Player::Rerelease(to_q2_rerelease_player(&frame.player))
        } else {
            Q2Player::Classic(to_q2_player(&frame.player))
        }
    }

    fn player_origin(&self, frame: &Q2WireFrame) -> Vec3 {
        match self.native_player(frame) {
            Q2Player::Rerelease(player) => contract_vec3(&player.movement.origin),
            Q2Player::Classic(player) => Vec3 {
                x: player.movement.origin_eighths[0] as f32 / 8.0,
                y: player.movement.origin_eighths[1] as f32 / 8.0,
                z: player.movement.origin_eighths[2] as f32 / 8.0,
            },
        }
    }

    fn sampled_player_origin(&self, frame: &Q2WireFrame) -> Vec3 {
        if let Some(predicted) = self.predicted.as_ref() {
            if matches!(
                predicted.status,
                Q2PredictionStatus::Predicted | Q2PredictionStatus::Disabled
            ) {
                return predicted.origin;
            }
        }
        let origin = self.player_origin(frame);
        let Some(previous) = self.previous_frame.as_ref() else {
            return origin;
        };
        let before = self.player_origin(previous);
        let teleport = (origin.x - before.x)
            .abs()
            .max((origin.y - before.y).abs())
            .max((origin.z - before.z).abs())
            > 256.0;
        if teleport {
            origin
        } else {
            lerp_vec(before, origin, self.fraction)
        }
    }

    /// Visible world text (donor `worldText`, always empty).
    #[must_use]
    pub fn world_text(&self) -> Vec<WorldText> {
        Vec::new()
    }

    /// Camera view for an actor (donor `playerView`).
    #[must_use]
    pub fn player_view(&self, actor: &ActorId) -> PlayerView {
        let (_, frame) = self.require_player(actor);
        let origin = self.sampled_player_origin(frame);
        let previous = self.previous_frame.as_ref();
        let field_of_view = match previous {
            None => f64::from(frame.player.fov),
            Some(previous) => {
                f64::from(previous.player.fov)
                    + (f64::from(frame.player.fov) - f64::from(previous.player.fov)) * self.fraction
            }
        };
        let native = self.native_player(frame);
        let prior_native = previous.map(|frame| self.native_player(frame));
        let damage_blend = match &native {
            Q2Player::Rerelease(player) => {
                let prior = match prior_native.as_ref() {
                    Some(Q2Player::Rerelease(prior)) => Some(vec4(
                        prior.damage_blend.x,
                        prior.damage_blend.y,
                        prior.damage_blend.z,
                        prior.damage_blend.w,
                    )),
                    _ => None,
                };
                let current = vec4(
                    player.damage_blend.x,
                    player.damage_blend.y,
                    player.damage_blend.z,
                    player.damage_blend.w,
                );
                Some(interpolate_q2_damage_blend(
                    prior.as_ref(),
                    &current,
                    self.fraction as f32,
                ))
            }
            Q2Player::Classic(_) => None,
        };
        let (native_offset, native_view) = match &native {
            Q2Player::Rerelease(player) => (contract_vec3(&player.view.view_offset), &player.view),
            Q2Player::Classic(player) => (contract_vec3(&player.view.view_offset), &player.view),
        };
        let offset = match prior_native.as_ref() {
            None => native_offset,
            Some(Q2Player::Rerelease(prior)) => {
                lerp_vec(contract_vec3(&prior.view.view_offset), native_offset, self.fraction)
            }
            Some(Q2Player::Classic(prior)) => {
                lerp_vec(contract_vec3(&prior.view.view_offset), native_offset, self.fraction)
            }
        };
        let height = self.options.host.view_height(&native);
        let prior_height = prior_native
            .as_ref()
            .map(|prior| self.options.host.view_height(prior))
            .unwrap_or(height);
        let source_height = match &native {
            Q2Player::Rerelease(_) => Some(self.view_height.borrow_mut().sample(height, self.view_time)),
            Q2Player::Classic(_) => None,
        };
        if let Some(predicted) = self.predicted.as_ref() {
            if matches!(
                predicted.status,
                Q2PredictionStatus::Predicted | Q2PredictionStatus::Disabled
            ) {
                let position = self.options.host.view_position(
                    &native,
                    origin,
                    offset,
                    source_height.unwrap_or(predicted.view_height),
                );
                return PlayerView {
                    origin: position.origin,
                    angles: predicted.view_angles,
                    view_height: position.view_height,
                    blend: None,
                    damage_blend,
                    kick_angles: None,
                    field_of_view: Some(field_of_view),
                    client_view_offset_delta: None,
                    foreign_character_death: None,
                    pitch_drift: None,
                };
            }
        }
        let position = self.options.host.view_position(
            &native,
            origin,
            offset,
            source_height.unwrap_or(prior_height + (height - prior_height) * self.fraction),
        );
        PlayerView {
            origin: position.origin,
            angles: match previous {
                None => contract_vec3(&native_view.view_angles),
                Some(previous) => lerp_angles(
                    vec3(previous.player.viewangles),
                    vec3(frame.player.viewangles),
                    self.fraction,
                ),
            },
            view_height: position.view_height,
            blend: None,
            damage_blend,
            kick_angles: None,
            field_of_view: Some(field_of_view),
            client_view_offset_delta: None,
            foreign_character_death: None,
            pitch_drift: None,
        }
    }

    /// HUD state for an actor (donor `playerUi`).
    #[must_use]
    pub fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        let (_, frame) = self.require_player(actor);
        let table = base_weapons();
        let weapon_model = self
            .configs
            .get(&self.layout.models.wrapping_add(frame.player.gunindex as u32));
        let weapon = table
            .iter()
            .find(|definition| Some(&definition.definition.view_model) == weapon_model);
        let source = self
            .content()
            .recipe()
            .weapons
            .first()
            .unwrap_or_else(|| panic!("Remote Q2 presentation requires its selected weapon provider"));
        let (namespace, provider) = source.provider.split_once(':').unwrap_or(("", ""));
        let provider = ProviderReference {
            provider: ProviderId::new(namespace, provider),
            content: ContentId(source.content.clone()),
        };
        let ammo_count = f64::from(frame.player.stats[3]);
        let entries: Vec<InventoryEntry> = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(ordinal, count)| {
                if *count == 0 {
                    return None;
                }
                let label = self.configs.get(&(self.layout.items + ordinal as u32))?;
                let normalized: String = label
                    .to_lowercase()
                    .chars()
                    .filter(|character| *character != ' ')
                    .collect();
                let definition = table
                    .iter()
                    .find(|definition| definition.definition.name == normalized)?;
                Some(InventoryEntry {
                    item: definition.definition.item.clone(),
                    count: f64::from(*count),
                    capacity: f64::from(*count),
                    count_policy: None,
                })
            })
            .collect();
        let armor = ArmorState {
            regular: if frame.player.stats[5] == 0 {
                RegularArmorState::None
            } else {
                RegularArmorState::Q2 {
                    points: f64::from(frame.player.stats[5]),
                    normal_protection: 0.0,
                    energy_protection: 0.0,
                    item: REMOTE_ARMOR_ITEM.to_string(),
                }
            },
            powered: PoweredProtectionState::None,
        };
        PlayerUi {
            selected_arsenal: None,
            native_inventory: None,
            health: f64::from(frame.player.stats[1]),
            armor,
            active_weapon: weapon.map(|definition| definition.definition.item.clone()),
            ammo: weapon
                .and_then(|definition| definition.definition.ammo.clone())
                .map(|ammo| PlayerAmmo {
                    item: ammo,
                    count: ammo_count,
                }),
            inventory: entries.clone(),
            arsenal_warning: ArsenalWarning::None,
            powerups: Vec::new(),
            items: table
                .iter()
                .enumerate()
                .map(|(ordinal, definition)| {
                    let owned = entries.iter().any(|entry| entry.item == definition.definition.item)
                        || weapon.is_some_and(|active| std::ptr::eq(active, definition));
                    PlayerArsenalItem {
                        id: definition.definition.item.clone(),
                        label: definition.definition.name.clone(),
                        kind: PlayerArsenalKind::Weapon,
                        source_ordinal: ordinal as f64,
                        owned,
                        has_ammo: definition.definition.ammo.is_none()
                            || ammo_count >= f64::from(definition.definition.quantity),
                        count: definition.definition.ammo.as_ref().map(|_| ammo_count),
                        warning_count: f64::from(definition.definition.warning),
                    }
                })
                .collect(),
            weapon_status: q2_weapon_status(weapon.map(|definition| &definition.definition), ammo_count, provider),
        }
    }

    /// Visible character views (donor `characterViews`, always empty).
    #[must_use]
    pub fn character_views(&self) -> Vec<Q3CharacterView> {
        Vec::new()
    }

    /// Visible presentations (donor `presentations`).
    #[must_use]
    pub fn presentations(&self) -> Vec<PresentationModel> {
        let (Some(current), Some(player)) = (self.current.as_ref(), self.current_player.as_ref()) else {
            return Vec::new();
        };
        let mut result = Vec::new();
        for entity in &current.entities {
            let mut path = self
                .configs
                .get(&self.layout.models.wrapping_add(u32::from(entity.modelindex)))
                .cloned();
            let mut skin_path = None;
            if entity.modelindex == PLAYER_SKIN_MODEL {
                let value = self
                    .configs
                    .get(&self.layout.player_skins.wrapping_add((entity.skinnum & 255) as u32))
                    .map_or("player\\male/grunt", String::as_str);
                let appearance = value.split('\\').nth(1).unwrap_or(value);
                let (model, skin) = match appearance.split_once('/') {
                    Some((model, skin)) => (model, skin),
                    None => ("male", "grunt"),
                };
                path = Some(format!("players/{model}/tris.md2"));
                skin_path = Some(format!("players/{model}/{skin}.pcx"));
            }
            let Some(path) = path else { continue };
            if entity.modelindex == 0 {
                continue;
            }
            let prior = self
                .previous_frame
                .as_ref()
                .and_then(|previous| previous.entities.iter().find(|value| value.number == entity.number));
            let continuous = match prior {
                Some(prior) if prior.modelindex == entity.modelindex && entity.event != 6 && entity.event != 7 => {
                    [0, 1, 2]
                        .into_iter()
                        .map(|index| (entity.origin[index] - prior.origin[index]).abs())
                        .fold(0.0f64, f64::max)
                        <= 512.0
                }
                _ => false,
            };
            let origin = match (continuous, prior) {
                (true, Some(prior)) => lerp_vec(vec3(prior.origin), vec3(entity.origin), self.fraction),
                _ => vec3(entity.origin),
            };
            let angles = match (continuous, prior) {
                (true, Some(prior)) => lerp_angles(vec3(prior.angles), vec3(entity.angles), self.fraction),
                _ => vec3(entity.angles),
            };
            result.push(PresentationModel {
                actor: self.actor(u32::from(entity.number)),
                content: self.map_content(),
                family: PresentationFamily::Q2,
                path,
                frame: i64::from(entity.frame),
                old_frame: i64::from(prior.map_or(
                    entity.frame,
                    |prior| {
                        if continuous {
                            prior.frame
                        } else {
                            entity.frame
                        }
                    },
                )),
                skin: if entity.modelindex == PLAYER_SKIN_MODEL {
                    0
                } else {
                    i64::from(entity.skinnum)
                },
                effects: i64::from(entity.effects),
                render_flags: i64::from(entity.renderfx),
                origin,
                angles,
                scale: if entity.scale == 0.0 { 1.0 } else { entity.scale },
                visible: true,
                view_weapon: false,
                replaces_body: None,
                render_owner: None,
                held_weapon: None,
                native_held_weapon: None,
                weapon_item: None,
                flare: None,
                back_lerp: Some(if continuous { 1.0 - self.fraction } else { 0.0 }),
                skin_path: Some(skin_path),
                indexed_skin: None,
                player_colors: None,
                previous_origin: None,
                model_beam: None,
                shader_beam: None,
                model_attachments: None,
                model_anchor: None,
                q3_grapple_cable: None,
                alpha: None,
                q3_weapon: None,
            });
        }
        let gun_path = self
            .configs
            .get(&self.layout.models.wrapping_add(current.player.gunindex as u32));
        if let Some(gun_path) = gun_path {
            if current.player.gunindex != 0 {
                let view = self.player_view(&player.actor);
                let offset = vec3(current.player.gunoffset);
                let old_frame = match self.previous_frame.as_ref() {
                    Some(previous) if previous.player.gunindex == current.player.gunindex => previous.player.gunframe,
                    _ => current.player.gunframe,
                };
                result.push(PresentationModel {
                    actor: player.actor.clone(),
                    content: self.map_content(),
                    family: PresentationFamily::Q2,
                    path: gun_path.clone(),
                    frame: i64::from(current.player.gunframe),
                    old_frame: i64::from(old_frame),
                    skin: i64::from(current.player.gunskin),
                    effects: 0,
                    render_flags: 0,
                    origin: Vec3 {
                        x: view.origin.x + offset.x,
                        y: view.origin.y + offset.y,
                        z: view.origin.z + view.view_height as f32 + offset.z,
                    },
                    angles: view.angles,
                    scale: 1.0,
                    visible: true,
                    view_weapon: true,
                    replaces_body: None,
                    render_owner: None,
                    held_weapon: None,
                    native_held_weapon: None,
                    weapon_item: None,
                    flare: None,
                    back_lerp: Some(1.0 - self.fraction),
                    skin_path: None,
                    indexed_skin: None,
                    player_colors: None,
                    previous_origin: None,
                    model_beam: None,
                    shader_beam: None,
                    model_attachments: None,
                    model_anchor: None,
                    q3_grapple_cable: None,
                    alpha: None,
                    q3_weapon: None,
                });
            }
        }
        result
    }

    /// Register a resolved resource (donor `registerResource`).
    pub fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference) {
        self.resources.insert(format!("{content}/{path}"), resource.clone());
    }

    /// Run a player command (donor `playerCommand`).
    pub fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        self.require_player(actor);
        if std::iter::once(name)
            .chain(args.iter().map(String::as_str))
            .any(|value| value.contains(['"', '\n', '\r', ';']))
        {
            panic!("Q2 console argument contains a command delimiter");
        }
        let mut text = name.to_string();
        for arg in args {
            text.push_str(&format!(" \"{arg}\""));
        }
        self.options.host.send_command(&text);
    }

    /// Per-step movement tunables from the current configstrings.
    fn tunables(&self) -> Q2MovementTunables {
        Q2MovementTunables {
            air_accelerate: config_number(self.configs.get(&self.layout.air_accelerate).map(String::as_str)),
            strafejump_hack: self.strafejump_hack,
            n64_physics: self
                .layout
                .n64_physics
                .is_some_and(|index| config_number(self.configs.get(&index).map(String::as_str)) != 0.0),
        }
    }

    /// Prediction snapshot for a decoded frame (donor `receivePrediction` assembly).
    fn prediction_base(&self, frame: &Q2WireFrame, player: &ApplicationNetworkPlayer) -> Q2PredictionBase {
        let native = self.native_player(frame);
        let ui = self.player_ui(&player.actor);
        let (health, gun_frame) = match &native {
            Q2Player::Rerelease(native) => (
                f64::from(native.view.stats.get(1).copied().unwrap_or(0)),
                native.view.gun_frame,
            ),
            Q2Player::Classic(native) => (
                f64::from(native.view.stats.get(1).copied().unwrap_or(0)),
                native.view.gun_frame,
            ),
        };
        Q2PredictionBase {
            sequence: self.packet_acknowledged,
            command_time_ms: f64::from(frame.server_frame) * self.frame_milliseconds,
            view_height: self.options.host.view_height(&native),
            bounds: self.options.host.body_bounds(&native),
            player: native,
            health,
            active_weapon: ui.active_weapon,
            ammo: ui.inventory,
            gun_frame,
        }
    }

    /// Receive a frame into the movement predictor (donor `receivePrediction`).
    fn receive_prediction(&mut self, frame: &Q2WireFrame) {
        let Some(player) = self.current_player.clone() else {
            return;
        };
        let native = self.native_player(frame);
        let native_kind = match &native {
            Q2Player::Rerelease(_) => Q2MovementProfileKind::Q2Rerelease,
            Q2Player::Classic(_) => Q2MovementProfileKind::Q2Classic,
        };
        let profile = self.options.host.movement_profile_kind(self.content().recipe());
        if profile == Q2MovementProfileKind::Other {
            return;
        }
        if profile != native_kind {
            panic!("Q2 server movement API differs from selected prediction profile");
        }
        let tunables = self.tunables();
        let base = self.prediction_base(frame, &player);
        if self.prediction_owner.is_none() {
            let recipe = self.content().recipe().clone();
            let owned = self
                .options
                .identity
                .owned_actor(&player.actor, provider_id(&recipe.map.entities.provider))
                .expect("prediction actor belongs to the presentation session");
            let seed = Q2MovementSeed {
                actor: owned,
                seat: self.options.seat.clone(),
                recipe: &recipe,
                profile,
                base,
            };
            self.prediction_owner = Some(self.options.host.build_predictor(&seed));
        } else if let Some(owner) = self.prediction_owner.as_mut() {
            owner.receive(&base, &tunables);
        }
        for entity in &frame.entities {
            self.actor(u32::from(entity.number));
        }
        let owner = self.prediction_owner.as_mut().expect("predictor is built");
        let current = self.current.as_ref();
        let actors = self.actors.borrow();
        let predicted = owner.replay(
            &|hit| match hit {
                Q2BrushHit::World => true,
                Q2BrushHit::Actor(actor) => current.is_some_and(|frame| {
                    frame.entities.iter().any(|entity| {
                        entity.solid == BRUSH_SOLID && actors.get(&u32::from(entity.number)) == Some(actor)
                    })
                }),
                Q2BrushHit::Other => false,
            },
            &tunables,
        );
        self.predicted = Some(predicted);
    }

    /// Relink solid bodies into the scene (donor `linkSolids`).
    fn link_solids(&mut self, frame: &Q2WireFrame, bodies: &[SnapshotBody]) {
        let linked: Vec<ActorId> = self.actors.borrow().values().cloned().collect();
        for actor in &linked {
            self.world
                .scene_mut()
                .expect("remote content is loaded")
                .unlink_actor(actor);
        }
        for entity in &frame.entities {
            if entity.solid == 0 {
                continue;
            }
            let actor = self.actor(u32::from(entity.number));
            let Some(body) = bodies.iter().find(|body| body.id == SavedActorId::from(&actor)) else {
                continue;
            };
            let path = self
                .configs
                .get(&self.layout.models.wrapping_add(u32::from(entity.modelindex)));
            let model = if entity.solid == BRUSH_SOLID && path.is_some_and(|path| path.starts_with('*')) {
                let number: f64 = path.map_or(f64::NAN, |path| path[1..].parse().unwrap_or(f64::NAN));
                is_integer(number).then_some(number as u32)
            } else {
                None
            };
            if entity.solid == BRUSH_SOLID && model.is_none() {
                continue;
            }
            let bounds = match model {
                None => body.state.bounds,
                Some(model) => self
                    .world
                    .scene_mut()
                    .expect("remote content is loaded")
                    .model_bounds(model),
            };
            let origin = body.state.origin;
            let extent = |min: f32, max: f32| f64::from(min.abs().max(max.abs()));
            let radius = extent(bounds.min.x, bounds.max.x)
                .hypot(extent(bounds.min.y, bounds.max.y))
                .hypot(extent(bounds.min.z, bounds.max.z));
            let rotated = model.is_some()
                && (body.state.angles.x != 0.0 || body.state.angles.y != 0.0 || body.state.angles.z != 0.0);
            let linked_bounds = if rotated {
                Bounds {
                    min: Vec3 {
                        x: -(radius as f32),
                        y: -(radius as f32),
                        z: -(radius as f32),
                    },
                    max: Vec3 {
                        x: radius as f32,
                        y: radius as f32,
                        z: radius as f32,
                    },
                }
            } else {
                bounds
            };
            let mut state = body.state.clone();
            state.bounds = bounds;
            self.world
                .scene_mut()
                .expect("remote content is loaded")
                .link_solid(Q2SolidLink {
                    actor,
                    state,
                    link_count: frame.server_frame,
                    absolute_bounds: Bounds {
                        min: Vec3 {
                            x: origin.x + linked_bounds.min.x - 1.0,
                            y: origin.y + linked_bounds.min.y - 1.0,
                            z: origin.z + linked_bounds.min.z - 1.0,
                        },
                        max: Vec3 {
                            x: origin.x + linked_bounds.max.x + 1.0,
                            y: origin.y + linked_bounds.max.y + 1.0,
                            z: origin.z + linked_bounds.max.z + 1.0,
                        },
                    },
                    model,
                    monster: model.is_none(),
                });
        }
    }

    /// Handle a decoded frame (donor `frame`).
    pub fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64) {
        for record in records {
            if let Q2ServerEvent::ConfigString { index, value } = &record.event {
                self.configs.insert(u32::from(*index), value.clone());
            }
        }
        let Some(player) = self.current_player.clone() else {
            panic!("Q2 frame precedes application signon");
        };
        let previous = self.current.clone();
        let native = self.native_player(frame);
        let prior_native = previous.as_ref().map(|frame| self.native_player(frame));
        let continuous = match (&native, prior_native.as_ref(), previous.as_ref()) {
            (Q2Player::Rerelease(native), Some(Q2Player::Rerelease(prior)), Some(previous)) if previous.valid => {
                let event = frame
                    .entities
                    .iter()
                    .find(|entity| u32::from(entity.number) == player.source_entity)
                    .map_or(0, |entity| entity.event);
                self.options
                    .host
                    .view_continuous(prior, native, previous.server_frame, frame.server_frame, event)
            }
            (Q2Player::Rerelease(_), _, _) => false,
            (Q2Player::Classic(_), _, _) => true,
        };
        self.previous_frame = if continuous { previous.clone() } else { None };
        self.current = Some(frame.clone());
        self.predicted = None;
        self.fraction = 1.0;
        self.view_time = self
            .view_time
            .max(f64::from(frame.server_frame - 1) * self.frame_milliseconds);
        self.received_at = self.options.host.presentation_time().unwrap_or(now_milliseconds);
        let view = self.player_view(&player.actor);
        let velocity = match &native {
            Q2Player::Rerelease(native) => contract_vec3(&native.movement.velocity),
            Q2Player::Classic(native) => Vec3 {
                x: native.movement.velocity_eighths[0] as f32 / 8.0,
                y: native.movement.velocity_eighths[1] as f32 / 8.0,
                z: native.movement.velocity_eighths[2] as f32 / 8.0,
            },
        };
        let encoding = q2_solid_encoding(net_protocol(self.selected_protocol), self.extended_game)
            .expect("Q2 protocol always selects a solid encoding");
        let mut bodies: Vec<SnapshotBody> = frame
            .entities
            .iter()
            .filter(|entity| u32::from(entity.number) != player.source_entity)
            .map(|entity| SnapshotBody {
                id: SavedActorId::from(&self.actor(u32::from(entity.number))),
                state: BodyState {
                    origin: vec3(entity.origin),
                    angles: vec3(entity.angles),
                    velocity: ZERO,
                    bounds: unpack_q2_solid(entity.solid, encoding),
                    ground: None,
                },
            })
            .collect();
        bodies.push(SnapshotBody {
            id: SavedActorId::from(&player.actor),
            state: BodyState {
                origin: self.player_origin(frame),
                angles: view.angles,
                velocity,
                bounds: self.options.host.body_bounds(&native),
                ground: None,
            },
        });
        let time = f64::from(frame.server_frame) * self.frame_milliseconds;
        let recipe = self.content().recipe().clone();
        let mut light_styles = Vec::new();
        for style in 0..256 {
            let Some(pattern) = self.configs.get(&(self.layout.lights + style)) else {
                continue;
            };
            let value = if pattern.is_empty() {
                1.0
            } else {
                let at = (time / 100.0).floor().max(0.0) as usize % pattern.len();
                (f64::from(pattern.as_bytes()[at]) - 97.0) / 12.0
            };
            light_styles.push(Q2RemoteLightStyle {
                style,
                rgb: Vec3 {
                    x: value as f32,
                    y: value as f32,
                    z: value as f32,
                },
                white: value * 3.0,
            });
        }
        let owner = provider_id(&recipe.map.entities.provider);
        let published = SimulationOutput {
            snapshot: qa_world::session::WorldSnapshot {
                frame: FrameContext {
                    frame: frame.server_frame,
                    time: SourceTime::Milliseconds(time as i32),
                    elapsed: SourceTime::Milliseconds(previous.as_ref().map_or(self.frame_milliseconds, |previous| {
                        f64::from(frame.server_frame - previous.server_frame) * self.frame_milliseconds
                    }) as i32),
                    phase: FramePhase::FrameExit,
                },
                actors: bodies
                    .iter()
                    .map(|body| SnapshotActor {
                        id: body.id,
                        owner: owner.clone(),
                        definition: "q2:remote-entity".to_string(),
                    })
                    .collect(),
                bodies: bodies.clone(),
                inventories: vec![SnapshotInventory {
                    id: SavedActorId::from(&player.actor),
                    entries: self
                        .player_ui(&player.actor)
                        .inventory
                        .iter()
                        .map(snapshot_entry)
                        .collect(),
                }],
            },
            events: Vec::new(),
        };
        self.published_scene = Some(Q2RemoteSceneOutput {
            world_resource: recipe.map.geometry.clone(),
            world_geometry: self.content().world_geometry().to_string(),
            configurations: vec![Q2RemoteConfiguration {
                actor: player.actor.clone(),
                movement: recipe.movement.clone(),
                character: recipe.character.clone(),
                weapons: recipe.weapons.clone(),
                inventory: recipe.inventory.clone(),
            }],
            light_styles,
            area_bits: frame.area_bits.clone(),
        });
        self.published = Some(published);
        if let Some(published) = self.published.clone() {
            self.options.host.publish(&published);
        }
        self.link_solids(frame, &bodies);
        if self.options.recorded_profile.is_none() {
            self.receive_prediction(frame);
        }
        let seconds = time / 1000.0;
        for entity in &frame.entities {
            if entity.event != 0 {
                let actor = self.actor(u32::from(entity.number));
                self.push_event(
                    Q2RemoteEvent::EntityEvent {
                        actor,
                        event: entity.event,
                    },
                    Some(u32::from(entity.number)),
                    seconds,
                );
            }
        }
        if let Some(previous) = previous.as_ref() {
            for entity in &previous.entities {
                if entity.sound == 0
                    || frame
                        .entities
                        .iter()
                        .any(|current| current.number == entity.number && current.sound == entity.sound)
                {
                    continue;
                }
                let actor = self.actor(u32::from(entity.number));
                self.push_event(
                    Q2RemoteEvent::Sound {
                        actor: Some(actor),
                        origin: vec3(entity.origin),
                        path: self
                            .configs
                            .get(&self.layout.sounds.wrapping_add(u32::from(entity.sound)))
                            .cloned()
                            .unwrap_or_default(),
                        channel: 0,
                        volume: 0.0,
                        attenuation: 0.0,
                        reliable: false,
                        loop_state: Q2SoundLoop::Stop,
                    },
                    Some(u32::from(entity.number)),
                    seconds,
                );
            }
        }
        for entity in &frame.entities {
            if entity.sound == 0
                || previous.as_ref().is_some_and(|previous| {
                    previous
                        .entities
                        .iter()
                        .any(|old| old.number == entity.number && old.sound == entity.sound)
                })
            {
                continue;
            }
            let path = self
                .configs
                .get(&self.layout.sounds.wrapping_add(u32::from(entity.sound)))
                .cloned()
                .unwrap_or_else(|| panic!("Q2 loop sound {} has no configstring", entity.sound));
            let actor = self.actor(u32::from(entity.number));
            self.push_event(
                Q2RemoteEvent::Sound {
                    actor: Some(actor),
                    origin: vec3(entity.origin),
                    path,
                    channel: 0,
                    volume: 1.0,
                    attenuation: 1.0,
                    reliable: false,
                    loop_state: Q2SoundLoop::Start,
                },
                Some(u32::from(entity.number)),
                seconds,
            );
        }
    }

    /// Sample presentation time (donor `samplePresentation`).
    pub fn sample_presentation(&mut self, now_milliseconds: u64) -> Option<&SimulationOutput> {
        let now = self.options.host.presentation_time().unwrap_or(now_milliseconds);
        self.sample_frame(now.saturating_sub(self.received_at) as f64 / self.frame_milliseconds)
    }

    /// Sample a recorded timestamp (donor `sampleDemo`).
    pub fn sample_demo(&mut self, recorded_milliseconds: f64) -> Option<&SimulationOutput> {
        self.predicted = None;
        let base = f64::from(self.current.as_ref().map_or(0, |frame| frame.server_frame) - 1) * self.frame_milliseconds;
        self.sample_frame((recorded_milliseconds - base) / self.frame_milliseconds)
    }

    fn sample_frame(&mut self, fraction: f64) -> Option<&SimulationOutput> {
        let (Some(current), Some(output), Some(player)) = (
            self.current.clone(),
            self.published.clone(),
            self.current_player.clone(),
        ) else {
            return None;
        };
        self.fraction = fraction.clamp(0.0, 1.0);
        let time = (f64::from(current.server_frame) - 1.0 + self.fraction) * self.frame_milliseconds;
        self.view_time = time;
        let presentations = self.presentations();
        let view = self.player_view(&player.actor);
        let prior_time = match output.snapshot.frame.time {
            SourceTime::Milliseconds(value) => f64::from(value),
            SourceTime::Seconds(value) => f64::from(value) * 1000.0,
        };
        let native = self.native_player(&current);
        let predicted_bounds = match self.predicted.as_ref() {
            Some(predicted)
                if matches!(
                    predicted.status,
                    Q2PredictionStatus::Predicted | Q2PredictionStatus::Disabled
                ) =>
            {
                Some(predicted.bounds)
            }
            _ => None,
        };
        let bodies: Vec<SnapshotBody> = output
            .snapshot
            .bodies
            .iter()
            .map(|body| {
                if body.id == SavedActorId::from(&player.actor) {
                    SnapshotBody {
                        id: body.id,
                        state: BodyState {
                            origin: self.sampled_player_origin(&current),
                            angles: view.angles,
                            velocity: body.state.velocity,
                            bounds: predicted_bounds.unwrap_or_else(|| self.options.host.body_bounds(&native)),
                            ground: body.state.ground.clone(),
                        },
                    }
                } else {
                    match presentations.iter().find(|presentation| {
                        !presentation.view_weapon && SavedActorId::from(&presentation.actor) == body.id
                    }) {
                        None => body.clone(),
                        Some(presentation) => SnapshotBody {
                            id: body.id,
                            state: BodyState {
                                origin: presentation.origin,
                                angles: presentation.angles,
                                velocity: body.state.velocity,
                                bounds: body.state.bounds,
                                ground: body.state.ground.clone(),
                            },
                        },
                    }
                }
            })
            .collect();
        let sampled = SimulationOutput {
            snapshot: qa_world::session::WorldSnapshot {
                frame: FrameContext {
                    frame: output.snapshot.frame.frame,
                    time: SourceTime::Milliseconds(time as i32),
                    elapsed: SourceTime::Milliseconds((time - prior_time).max(0.0) as i32),
                    phase: output.snapshot.frame.phase,
                },
                actors: output.snapshot.actors.clone(),
                bodies,
                inventories: output.snapshot.inventories.clone(),
            },
            events: output.events.clone(),
        };
        self.published = Some(sampled);
        if let Some(published) = self.published.clone() {
            self.options.host.publish(&published);
        }
        self.published.as_ref()
    }

    /// Handle raw server records (donor `records`).
    pub fn records(&mut self, records: &[Q2ServerRecord]) {
        self.last_records = records.to_vec();
        let seconds =
            f64::from(self.current.as_ref().map_or(0, |frame| frame.server_frame)) * self.frame_milliseconds / 1000.0;
        for record in records {
            match &record.event {
                Q2ServerEvent::ConfigString { index, value } => {
                    self.configs.insert(u32::from(*index), value.clone());
                    if self.current_player.is_some() {
                        self.player_info(u32::from(*index), value, seconds);
                    }
                }
                Q2ServerEvent::Print { level: 3, text } => {
                    if let Some(player) = self.current_player.clone() {
                        self.push_event(
                            Q2RemoteEvent::PlayerPrint {
                                target: player.actor,
                                level: Q2PrintLevel::Chat,
                                text: text.clone(),
                            },
                            Some(player.source_entity),
                            seconds,
                        );
                    }
                }
                Q2ServerEvent::Inventory { counts } => {
                    self.inventory = counts.clone();
                }
                Q2ServerEvent::Layout { text } => {
                    self.layout_text = text.clone();
                }
                Q2ServerEvent::TempEntity { value } => {
                    if let Some(decoded) = q2_effect_from_wire(value) {
                        self.push_event(Q2RemoteEvent::Effect(decoded), None, seconds);
                    } else if let Some(beam) = q2_beam_from_wire(value).unwrap_or_else(|error| panic!("{error}")) {
                        self.push_event(Q2RemoteEvent::Beam(beam), None, seconds);
                    }
                }
                Q2ServerEvent::Sound { sound } => {
                    let path = self
                        .configs
                        .get(&self.layout.sounds.wrapping_add(u32::from(sound.index)))
                        .cloned()
                        .unwrap_or_else(|| panic!("Q2 sound {} has no configstring", sound.index));
                    let entity = self.current.as_ref().and_then(|frame| {
                        frame
                            .entities
                            .iter()
                            .find(|entity| u32::from(entity.number) == sound.entity)
                    });
                    let origin = sound.position.map(vec3).unwrap_or_else(|| match entity {
                        Some(entity) => vec3(entity.origin),
                        None => ZERO,
                    });
                    let actor = if sound.entity == 0 {
                        None
                    } else {
                        Some(self.actor(sound.entity))
                    };
                    self.push_event(
                        Q2RemoteEvent::Sound {
                            actor,
                            origin,
                            path,
                            volume: sound.volume,
                            attenuation: sound.attenuation,
                            channel: sound.channel,
                            reliable: false,
                            loop_state: Q2SoundLoop::Once,
                        },
                        Some(sound.entity),
                        seconds,
                    );
                }
                Q2ServerEvent::MuzzleFlash {
                    entity,
                    flash,
                    monster,
                    silenced,
                } => {
                    if *monster {
                        let current = self.current.clone();
                        let Some(wire) = current.as_ref().and_then(|frame| {
                            frame
                                .entities
                                .iter()
                                .find(|candidate| i32::from(candidate.number) == *entity)
                        }) else {
                            continue;
                        };
                        let axes = angles_vectors(vec3(wire.angles));
                        let offset = muzzle_offset(Q2Edition::Classic, *flash as usize);
                        let origin = vec3(wire.origin);
                        let actor = self.actor(*entity as u32);
                        self.push_event(
                            Q2RemoteEvent::MonsterMuzzleflash {
                                actor: actor.clone(),
                                flash: *flash,
                                origin: Vec3 {
                                    x: origin.x + axes.forward.x * offset.x + axes.right.x * offset.y,
                                    y: origin.y + axes.forward.y * offset.x + axes.right.y * offset.y,
                                    z: origin.z + axes.forward.z * offset.x + axes.right.z * offset.y + offset.z,
                                },
                                direction: axes.forward,
                            },
                            Some(*entity as u32),
                            seconds,
                        );
                    } else {
                        let actor = self.actor(*entity as u32);
                        self.push_event(
                            Q2RemoteEvent::Muzzleflash {
                                actor,
                                flash: *flash,
                                silenced: *silenced,
                            },
                            Some(*entity as u32),
                            seconds,
                        );
                    }
                }
                Q2ServerEvent::CenterPrint { text } => {
                    if let Some(player) = self.current_player.clone() {
                        self.push_event(
                            Q2RemoteEvent::Centerprint {
                                actor: player.actor,
                                text: text.clone(),
                            },
                            Some(player.source_entity),
                            seconds,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// Drain presentation events (donor `drainPresentationEvents`).
    pub fn drain_presentation_events(&mut self) -> Vec<Q2RemotePresentationEvent> {
        std::mem::take(&mut self.events)
    }

    /// In-flight transfers (donor `downloadProgress`).
    #[must_use]
    pub fn download_progress(&mut self) -> Vec<ClientDownloadProgress> {
        self.options
            .host
            .downloads()
            .map_or(Vec::new(), |downloads| downloads.progress())
    }

    /// Cancel all transfers (donor `cancelDownloads`).
    pub fn cancel_downloads(&mut self) {
        if let Some(downloads) = self.options.host.downloads() {
            downloads.cancel();
        }
    }

    /// Retry after a cancel (donor `retryDownloads`).
    pub fn retry_downloads(&mut self) {
        if let Some(downloads) = self.options.host.downloads() {
            downloads.retry();
        }
    }

    /// Handle a disconnect (donor `disconnected`).
    pub fn disconnected(&mut self, reason: &str) {
        if let Some(cinematic) = self.options.host.cinematic() {
            cinematic.stop();
        }
        self.options.host.print(&format!("{reason}\n"));
        self.options.host.disconnected(reason);
    }

    /// Print client text (donor `print`).
    pub fn print(&mut self, text: &str) {
        self.options.host.print(text);
    }

    /// Convert an actor command into a wire command (donor `command`).
    #[must_use]
    pub fn command(&self, command: &ActorCommand) -> Usercmd {
        let (_, frame) = self.require_player(&command.actor);
        let native = self.native_player(frame);
        let wire = match &command.command {
            UserCommand::Q2Classic {
                milliseconds,
                angle_shorts,
                forward_move,
                side_move,
                up_move,
                buttons,
                impulse,
                light_level,
            } => Q2Command::Classic(Q2UserCommand {
                milliseconds: *milliseconds as u8,
                angle_shorts: [angle_shorts[0] as i16, angle_shorts[1] as i16, angle_shorts[2] as i16],
                forward_move: *forward_move as i16,
                side_move: *side_move as i16,
                up_move: *up_move as i16,
                buttons: *buttons as u8,
                impulse: *impulse as u8,
                light_level: *light_level as u8,
            }),
            UserCommand::Q2Rerelease {
                milliseconds,
                angles,
                forward_move,
                side_move,
                buttons,
                server_frame,
            } => Q2Command::Rerelease(Q2RereleaseUserCommand {
                milliseconds: *milliseconds as u8,
                angles: Q2Vec3 {
                    x: angles[0],
                    y: angles[1],
                    z: angles[2],
                },
                forward_move: *forward_move as i16,
                side_move: *side_move as i16,
                buttons: *buttons as u8,
                server_frame: *server_frame as i32,
            }),
            _ => panic!("Native Q2 client requires Q2 movement commands"),
        };
        self.options.host.remote_command(&wire, &native)
    }
}

impl<H: Q2RemoteHost + 'static> Q2ClientPrediction for Q2RemotePresentation<H> {
    fn sent(&mut self, sequence: u32, command: &Usercmd, now_milliseconds: u64) {
        let tunables = self.tunables();
        let server_frame = self.current.as_ref().map_or(0, |frame| frame.server_frame);
        let wire = if is_rerelease_family(self.selected_protocol) {
            Q2Command::Rerelease(to_q2_rerelease_command(command, server_frame))
        } else {
            Q2Command::Classic(to_q2_command(command))
        };
        if let Some(owner) = self.prediction_owner.as_mut() {
            owner.submit(sequence, now_milliseconds, wire, &tunables);
        }
        if self.prediction_owner.is_some() {
            for entity in self.current.clone().map(|frame| frame.entities).unwrap_or_default() {
                self.actor(u32::from(entity.number));
            }
        }
        if let Some(owner) = self.prediction_owner.as_mut() {
            let current = self.current.as_ref();
            let actors = self.actors.borrow();
            let predicted = owner.replay(
                &|hit| match hit {
                    Q2BrushHit::World => true,
                    Q2BrushHit::Actor(actor) => current.is_some_and(|frame| {
                        frame.entities.iter().any(|entity| {
                            entity.solid == BRUSH_SOLID && actors.get(&u32::from(entity.number)) == Some(actor)
                        })
                    }),
                    Q2BrushHit::Other => false,
                },
                &tunables,
            );
            self.predicted = Some(predicted);
        } else {
            self.predicted = None;
        }
    }

    fn acknowledged(&mut self, sequence: u32, _now_milliseconds: u64) {
        self.packet_acknowledged = sequence;
    }
}

impl<H: Q2RemoteHost + 'static> Q2ApplicationClientHost for Q2RemotePresentation<H> {
    fn server_data(&mut self, data: &Q2ServerData, assert_current: &dyn Fn()) {
        Q2RemotePresentation::server_data(self, data, assert_current);
    }

    fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads> {
        self.options.host.downloads()
    }

    fn protocol(&self) -> Q2ProtocolIdentity {
        self.selected_protocol
    }

    fn message_options(&self) -> Q2ServerMessageOptions {
        self.message_options.clone()
    }

    fn userinfo(&self) -> String {
        self.options.host.userinfo()
    }

    fn prediction(&mut self) -> Option<&mut dyn Q2ClientPrediction> {
        Some(self)
    }

    fn game_state(&mut self, state: &Q2ApplicationGameState) {
        Q2RemotePresentation::game_state(self, state);
    }

    fn frame(&mut self, frame: &Q2WireFrame, records: &[Q2ServerRecord], now_milliseconds: u64) {
        Q2RemotePresentation::frame(self, frame, records, now_milliseconds);
    }

    fn records(&mut self, records: &[Q2ServerRecord]) {
        Q2RemotePresentation::records(self, records);
    }

    fn command(&self, command: &ActorCommand) -> Usercmd {
        Q2RemotePresentation::command(self, command)
    }

    fn disconnected(&mut self, reason: &str) {
        Q2RemotePresentation::disconnected(self, reason);
    }

    fn print(&mut self, text: &str) {
        Q2RemotePresentation::print(self, text);
    }
}

impl<H: Q2RemoteHost + 'static> RemotePresentationAccess for Q2RemotePresentation<H> {
    fn world_text(&self) -> Vec<WorldText> {
        Q2RemotePresentation::world_text(self)
    }

    fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        Q2RemotePresentation::player_ui(self, actor)
    }

    fn character_views(&self) -> Vec<Q3CharacterView> {
        Q2RemotePresentation::character_views(self)
    }

    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        Q2RemotePresentation::player_command(self, actor, name, args);
    }

    fn presentations(&self) -> Vec<PresentationModel> {
        Q2RemotePresentation::presentations(self)
    }

    fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference) {
        Q2RemotePresentation::register_resource(self, content, path, resource);
    }

    fn player_view(&self, actor: &ActorId) -> PlayerView {
        Q2RemotePresentation::player_view(self, actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_net::q2::{EntityState, PlayerState, ServerData};
    use qa_world::session::SessionMode;

    use crate::persistence::recipe::fixture_recipe;

    #[derive(Debug, Clone)]
    struct TestContent {
        recipe: ExecutableRecipe,
        geometry_path: String,
        bytes: Vec<u8>,
    }

    impl Q2RemoteContent for TestContent {
        fn recipe(&self) -> &ExecutableRecipe {
            &self.recipe
        }

        fn world_geometry(&self) -> &str {
            "test-world"
        }

        fn map_geometry_path(&self) -> &str {
            &self.geometry_path
        }

        fn map_bytes(&self) -> Vec<u8> {
            self.bytes.clone()
        }
    }

    #[derive(Debug, Default)]
    struct TestScene {
        unlinked: Vec<ActorId>,
        linked: Vec<Q2SolidLink>,
    }

    impl Q2RemoteScene for TestScene {
        fn unlink_actor(&mut self, actor: &ActorId) {
            self.unlinked.push(actor.clone());
        }

        fn model_bounds(&self, _model: u32) -> Bounds {
            Bounds {
                min: Vec3 {
                    x: -8.0,
                    y: -8.0,
                    z: -8.0,
                },
                max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
            }
        }

        fn link_solid(&mut self, link: Q2SolidLink) {
            self.linked.push(link);
        }
    }

    #[derive(Debug, Default)]
    struct TestPredictor {
        submitted: Vec<(u32, u64)>,
        received: Vec<Q2PredictionBase>,
        predicted: Option<Q2PredictedPlayer>,
    }

    impl Q2RemotePredictor for TestPredictor {
        fn submit(
            &mut self,
            sequence: u32,
            time_milliseconds: u64,
            _command: Q2Command,
            _tunables: &Q2MovementTunables,
        ) {
            self.submitted.push((sequence, time_milliseconds));
        }

        fn receive(&mut self, base: &Q2PredictionBase, _tunables: &Q2MovementTunables) {
            self.received.push(base.clone());
        }

        fn replay(
            &mut self,
            is_brush: &dyn Fn(&Q2BrushHit) -> bool,
            _tunables: &Q2MovementTunables,
        ) -> Q2PredictedPlayer {
            assert!(is_brush(&Q2BrushHit::World));
            assert!(!is_brush(&Q2BrushHit::Other));
            self.predicted.clone().expect("test prediction is set")
        }
    }

    /// Test double of the missing `Q2RereleaseViewHeight` (donor algorithm).
    #[derive(Debug, Default)]
    struct TestViewHeight {
        state: Option<(f64, f64, f64)>,
    }

    impl Q2RereleaseViewHeight for TestViewHeight {
        fn reset(&mut self) {
            self.state = None;
        }

        fn sample(&mut self, height: f64, time_milliseconds: f64) -> f64 {
            let state = self.state.get_or_insert((height, height, time_milliseconds));
            if state.1 != height {
                state.0 = state.1;
                state.1 = height;
                state.2 = time_milliseconds;
            }
            let elapsed = (time_milliseconds - state.2).clamp(0.0, 100.0);
            state.1 + (state.0 - state.1) * (100.0 - elapsed) * 0.01
        }
    }

    #[derive(Debug, Default)]
    struct TestHostState {
        sent: Vec<String>,
        printed: Vec<String>,
        published: usize,
        disconnected: Vec<String>,
        generation: u32,
        profile_kind: Option<Q2MovementProfileKind>,
        content: Option<TestContent>,
        prepared: bool,
        replaced: bool,
    }

    struct TestHost {
        state: Rc<RefCell<TestHostState>>,
        predictor: Rc<RefCell<TestPredictor>>,
    }

    struct TestVisibility;

    impl MvdVisibility for TestVisibility {
        fn entities(&self, entities: &[EntityState], _player: &PlayerState, _portal_bits: &[u8]) -> Vec<EntityState> {
            entities.to_vec()
        }

        fn visible(
            &self,
            _leaf: u16,
            _channel: qa_net::q2_svc::MvdChannel,
            _player: &PlayerState,
            _portal_bits: &[u8],
        ) -> bool {
            true
        }

        fn area_bits(&self, _player: &PlayerState, _portal_bits: &[u8]) -> Vec<u8> {
            Vec::new()
        }

        fn sound_audible(&self, _origin: [f64; 3], _player: &PlayerState, _portal_bits: &[u8]) -> bool {
            true
        }

        fn sound_origin(&self, entity: &EntityState) -> [f64; 3] {
            entity.origin
        }
    }

    impl Q2RemoteHost for TestHost {
        type Content = TestContent;
        type Scene = TestScene;
        type Predictor = SharedPredictor;
        type ViewHeight = TestViewHeight;

        fn build_scene(_content: &TestContent) -> TestScene {
            TestScene::default()
        }

        fn build_view_height(&mut self) -> TestViewHeight {
            TestViewHeight::default()
        }

        fn build_predictor(&mut self, _seed: &Q2MovementSeed<'_>) -> SharedPredictor {
            SharedPredictor {
                inner: self.predictor.clone(),
            }
        }

        fn send_command(&mut self, text: &str) {
            self.state.borrow_mut().sent.push(text.to_string());
        }

        fn print(&mut self, text: &str) {
            self.state.borrow_mut().printed.push(text.to_string());
        }

        fn publish(&mut self, _output: &SimulationOutput) {
            self.state.borrow_mut().published += 1;
        }

        fn disconnected(&mut self, reason: &str) {
            self.state.borrow_mut().disconnected.push(reason.to_string());
        }

        fn next_generation(&self, _slot: u32) -> u32 {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.generation
        }

        fn userinfo(&self) -> String {
            "test-userinfo".to_string()
        }

        fn load_content(&mut self, _state: &Q2ApplicationGameState) -> Option<TestContent> {
            self.state.borrow().content.clone()
        }

        fn prepare_server_data(&mut self, _data: &Q2ServerData) {
            self.state.borrow_mut().prepared = true;
        }

        fn note_content_replaced(&mut self) {
            self.state.borrow_mut().replaced = true;
        }

        fn downloads(&mut self) -> Option<&mut dyn Q2ApplicationClientDownloads> {
            None
        }

        fn download_revision(&self) -> u64 {
            0
        }

        fn cinematic(&mut self) -> Option<&mut dyn Q2RemoteCinematic> {
            None
        }

        fn view_height(&self, player: &Q2Player) -> f64 {
            match player {
                Q2Player::Rerelease(player) => f64::from(player.movement.view_height),
                Q2Player::Classic(player) => player.view.view_offset.z,
            }
        }

        fn view_position(&self, player: &Q2Player, origin: Vec3, offset: Vec3, view_height: f64) -> Q2ViewPosition {
            let rerelease = matches!(player, Q2Player::Rerelease(_));
            Q2ViewPosition {
                origin: Vec3 {
                    x: origin.x + offset.x,
                    y: origin.y + offset.y,
                    z: origin.z + if rerelease { offset.z } else { 0.0 },
                },
                view_height,
            }
        }

        fn body_bounds(&self, _player: &Q2Player) -> Bounds {
            Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            }
        }

        fn remote_command(&self, command: &Q2Command, player: &Q2Player) -> Usercmd {
            use qa_net::q2_adapters::from_q2_command;
            match (command, player) {
                (Q2Command::Rerelease(command), Q2Player::Rerelease(player)) => {
                    let mut adjusted = command.clone();
                    adjusted.angles.x -= player.movement.delta_angles.x;
                    adjusted.angles.y -= player.movement.delta_angles.y;
                    adjusted.angles.z -= player.movement.delta_angles.z;
                    from_q2_command(&Q2Command::Rerelease(adjusted))
                }
                (Q2Command::Classic(command), Q2Player::Classic(player)) => {
                    let mut wire = from_q2_command(&Q2Command::Classic(command.clone()));
                    for index in 0..3 {
                        wire.angles[index] = wire.angles[index].wrapping_sub(player.movement.delta_angle_shorts[index]);
                    }
                    wire
                }
                _ => panic!("Q2 command and server movement editions differ"),
            }
        }

        fn view_continuous(
            &self,
            previous: &Q2RereleasePlayerState,
            current: &Q2RereleasePlayerState,
            previous_frame: i32,
            current_frame: i32,
            event: u8,
        ) -> bool {
            let before = &previous.movement.origin;
            let after = &current.movement.origin;
            current_frame == previous_frame + 1
                && event != 6
                && event != 7
                && (before.x - after.x)
                    .abs()
                    .max((before.y - after.y).abs())
                    .max((before.z - after.z).abs())
                    <= 256.0
                && (previous.view.render_flags ^ current.view.render_flags) & 16 == 0
        }

        fn mvd_layout(&self, _profile: &MvdProfile) -> Q2ApplicationLayout {
            q2_application_layout(ProtocolIdentity::Q2Classic).expect("classic layout")
        }

        fn mvd_visibility(&mut self, _profile: &MvdProfile) -> Box<dyn MvdVisibility> {
            Box::new(TestVisibility)
        }

        fn movement_profile_kind(&self, _recipe: &ExecutableRecipe) -> Q2MovementProfileKind {
            self.state.borrow().profile_kind.unwrap_or(Q2MovementProfileKind::Other)
        }
    }

    struct SharedPredictor {
        inner: Rc<RefCell<TestPredictor>>,
    }

    impl Q2RemotePredictor for SharedPredictor {
        fn submit(&mut self, sequence: u32, time_milliseconds: u64, command: Q2Command, tunables: &Q2MovementTunables) {
            self.inner
                .borrow_mut()
                .submit(sequence, time_milliseconds, command, tunables);
        }

        fn receive(&mut self, base: &Q2PredictionBase, tunables: &Q2MovementTunables) {
            self.inner.borrow_mut().receive(base, tunables);
        }

        fn replay(
            &mut self,
            is_brush: &dyn Fn(&Q2BrushHit) -> bool,
            tunables: &Q2MovementTunables,
        ) -> Q2PredictedPlayer {
            self.inner.borrow_mut().replay(is_brush, tunables)
        }
    }

    fn harness() -> (
        Q2RemotePresentation<TestHost>,
        Rc<RefCell<TestHostState>>,
        Rc<RefCell<TestPredictor>>,
    ) {
        harness_with_protocol(Q2ProtocolIdentity::Classic)
    }

    fn harness_with_protocol(
        protocol: Q2ProtocolIdentity,
    ) -> (
        Q2RemotePresentation<TestHost>,
        Rc<RefCell<TestHostState>>,
        Rc<RefCell<TestPredictor>>,
    ) {
        let state = Rc::new(RefCell::new(TestHostState::default()));
        let predictor = Rc::new(RefCell::new(TestPredictor::default()));
        let identity = IdentityOwner::create("test").unwrap();
        let client_id = identity.client(0, 1);
        let seat = identity.seat(0);
        let session = EngineSession::new(IdentityOwner::create("test").unwrap(), SessionMode::Local);
        let presentation = Q2RemotePresentation::new(Q2RemotePresentationOptions {
            identity,
            session,
            client: SessionClient::new(client_id),
            seat,
            content: None,
            protocol,
            recorded_profile: None,
            host: TestHost {
                state: state.clone(),
                predictor: predictor.clone(),
            },
        });
        (presentation, state, predictor)
    }

    fn test_content() -> TestContent {
        TestContent {
            recipe: fixture_recipe(),
            geometry_path: "maps/test.bsp".to_string(),
            bytes: vec![1, 2, 3, 4],
        }
    }

    fn game_state(clientnum: i16) -> Q2ApplicationGameState {
        let mut config_strings = HashMap::new();
        config_strings.insert(33, "maps/test.bsp".to_string());
        config_strings.insert(30, "4".to_string());
        let content = test_content();
        config_strings.insert(31, md4_block_checksum(&content.bytes).to_string());
        Q2ApplicationGameState {
            data: Q2ServerData::Vanilla(ServerData {
                servercount: 1,
                attractloop: false,
                gamedir: "baseq2".to_string(),
                clientnum,
                levelname: "test".to_string(),
            }),
            config_strings,
            baselines: HashMap::new(),
        }
    }

    fn admit(presentation: &mut Q2RemotePresentation<TestHost>, state: &Rc<RefCell<TestHostState>>) {
        state.borrow_mut().content = Some(test_content());
        presentation.game_state(&game_state(0));
    }

    #[test]
    fn entity_bounds_select_encoding() {
        let short = q2_remote_entity_bounds(0x0001_0203, false);
        let long = q2_remote_entity_bounds(0x0001_0203, true);
        assert_eq!(short, unpack_q2_solid(0x0001_0203, Q2SolidEncoding::Short));
        assert_eq!(long, unpack_q2_solid(0x0001_0203, Q2SolidEncoding::R1q2));
        assert_ne!(short, long);
    }

    #[test]
    #[should_panic(expected = "KEX native live transport is not bound")]
    fn kex_demo_protocol_is_rejected() {
        let _ = harness_with_protocol(Q2ProtocolIdentity::KexDemo);
    }

    #[test]
    fn classic_layout_matches_donor_values() {
        let (presentation, _, _) = harness();
        assert_eq!(presentation.config_layout().models, 32);
        assert_eq!(presentation.config_layout().max_config_strings, 2080);
        assert_eq!(presentation.message_options.inventory_slots, 256);
    }

    #[test]
    #[should_panic(expected = "Q2 frame precedes application signon")]
    fn frame_before_signon_panics() {
        let (mut presentation, _, _) = harness();
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 1,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: qa_net::q2::PlayerState::default(),
                split_players: Vec::new(),
                entities: Vec::new(),
            },
            &[],
            1000,
        );
    }

    #[test]
    fn game_state_admits_player_and_reports_content() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        let player = presentation.player().expect("player is admitted");
        assert_eq!(player.source_entity, 1);
        assert!(state.borrow().replaced);
        assert!(presentation.output().is_none());
    }

    #[test]
    #[should_panic(expected = "Q2 server map maps/other.bsp requires application content replacement")]
    fn game_state_rejects_map_mismatch() {
        let (mut presentation, state, _) = harness();
        state.borrow_mut().content = Some(test_content());
        let mut game = game_state(0);
        game.config_strings.insert(33, "maps/other.bsp".to_string());
        presentation.game_state(&game);
    }

    #[test]
    #[should_panic(expected = "Q2 server map checksum differs from mounted content")]
    fn game_state_rejects_checksum_mismatch() {
        let (mut presentation, state, _) = harness();
        state.borrow_mut().content = Some(test_content());
        let mut game = game_state(0);
        game.config_strings.insert(31, "1234".to_string());
        presentation.game_state(&game);
    }

    #[test]
    fn records_cover_config_inventory_layout_and_print() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        presentation.records(&[
            Q2ServerRecord {
                seat: 0,
                opcode: 0,
                raw: Vec::new(),
                event: Q2ServerEvent::ConfigString {
                    index: 1312,
                    value: "player\\male/grunt".to_string(),
                },
            },
            Q2ServerRecord {
                seat: 0,
                opcode: 0,
                raw: Vec::new(),
                event: Q2ServerEvent::Inventory { counts: vec![0, 5] },
            },
            Q2ServerRecord {
                seat: 0,
                opcode: 0,
                raw: Vec::new(),
                event: Q2ServerEvent::Layout {
                    text: "layout".to_string(),
                },
            },
            Q2ServerRecord {
                seat: 0,
                opcode: 0,
                raw: Vec::new(),
                event: Q2ServerEvent::Print {
                    level: 3,
                    text: "hello".to_string(),
                },
            },
        ]);
        assert_eq!(presentation.native_layout(), "layout");
        assert_eq!(presentation.source_records().len(), 4);
        let events = presentation.drain_presentation_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind(), "q2-player");
        assert!(matches!(
            &events[0].event,
            Q2RemoteEvent::PlayerUserinfo { slot: 0, name, skin, .. }
                if name == "player" && skin == "male/grunt"
        ));
        assert!(matches!(
            &events[1].event,
            Q2RemoteEvent::PlayerPrint { text, .. } if text == "hello"
        ));
        assert!(presentation.drain_presentation_events().is_empty());
    }

    #[test]
    fn player_command_quotes_and_sends() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 1,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: qa_net::q2::PlayerState::default(),
                split_players: Vec::new(),
                entities: Vec::new(),
            },
            &[],
            1000,
        );
        let player = presentation.player().expect("player is admitted");
        presentation.player_command(&player.actor, "say", &["hi there".to_string()]);
        assert_eq!(state.borrow().sent, vec!["say \"hi there\"".to_string()]);
    }

    #[test]
    #[should_panic(expected = "Q2 console argument contains a command delimiter")]
    fn player_command_rejects_delimiters() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 1,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: qa_net::q2::PlayerState::default(),
                split_players: Vec::new(),
                entities: Vec::new(),
            },
            &[],
            1000,
        );
        let player = presentation.player().expect("player is admitted");
        presentation.player_command(&player.actor, "say", &["a;b".to_string()]);
    }

    #[test]
    fn frame_publishes_snapshot_with_scene_blocks() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 2,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: vec![1],
                player: qa_net::q2::PlayerState::default(),
                split_players: Vec::new(),
                entities: Vec::new(),
            },
            &[],
            1000,
        );
        let output = presentation.output().expect("frame publishes");
        assert_eq!(output.snapshot.frame.frame, 2);
        assert_eq!(output.snapshot.bodies.len(), 1);
        assert_eq!(output.snapshot.actors.len(), 1);
        let scene = presentation.scene_output().expect("scene blocks travel");
        assert_eq!(scene.configurations.len(), 1);
        assert_eq!(scene.area_bits, vec![1]);
        assert_eq!(state.borrow().published, 1);
    }

    #[test]
    fn command_converts_q2_classic_moves() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 1,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: qa_net::q2::PlayerState::default(),
                split_players: Vec::new(),
                entities: Vec::new(),
            },
            &[],
            1000,
        );
        let player = presentation.player().expect("player is admitted");
        let wire = presentation.command(&ActorCommand {
            actor: player.actor,
            source: qa_net::common::commands::CommandSource::Remote {
                client: presentation.client().id().clone(),
            },
            sequence: 1,
            command: UserCommand::Q2Classic {
                milliseconds: 50.0,
                angle_shorts: [100.0, 200.0, 300.0],
                forward_move: 10.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 1.0,
                impulse: 0.0,
                light_level: 0.0,
            },
            arsenal: None,
        });
        assert_eq!(wire.msec, 50);
        assert_eq!(wire.angles, [100, 200, 300]);
        assert_eq!(wire.forwardmove, 10);
    }

    #[test]
    fn weapon_status_mirror_matches_donor_rules() {
        let table = base_weapons();
        let blaster = table
            .iter()
            .find(|definition| definition.definition.name == "blaster")
            .unwrap();
        let source = ProviderReference {
            provider: ProviderId::new("", ""),
            content: ContentId("test".to_string()),
        };
        let status = q2_weapon_status(Some(&blaster.definition), 0.0, source.clone()).expect("status");
        assert_eq!(status.item, "q2:weapon_blaster");
        assert_eq!(status.ammo, WeaponAmmoStatus::Unmetered);
        assert!(q2_weapon_status(None, 0.0, source).is_none());
        assert_eq!(display_name("super_shotgun"), "Super Shotgun");
    }

    #[test]
    fn server_data_prepares_and_disconnect_reports() {
        let (mut presentation, state, _) = harness();
        let game = game_state(0);
        presentation.server_data(&game.data, &|| {});
        assert!(state.borrow().prepared);
        presentation.disconnected("bye");
        assert_eq!(state.borrow().printed, vec!["bye\n".to_string()]);
        assert_eq!(state.borrow().disconnected, vec!["bye".to_string()]);
    }

    #[test]
    fn solid_entities_link_into_scene() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 1,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: PlayerState::default(),
                split_players: Vec::new(),
                entities: vec![EntityState {
                    number: 5,
                    modelindex: 1,
                    solid: 0x0001_0203,
                    ..Default::default()
                }],
            },
            &[],
            1000,
        );
        let scene = presentation.scene().expect("scene builds");
        assert_eq!(scene.linked.len(), 1);
        assert!(scene.linked[0].model.is_none());
        assert!(scene.linked[0].monster);
        assert_eq!(scene.unlinked.len(), 2);
    }

    #[test]
    fn sent_commands_submit_and_replay_prediction() {
        let (mut presentation, state, predictor) = harness();
        state.borrow_mut().profile_kind = Some(Q2MovementProfileKind::Q2Classic);
        predictor.borrow_mut().predicted = Some(Q2PredictedPlayer {
            status: Q2PredictionStatus::Predicted,
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            view_angles: ZERO,
            view_height: 22.0,
            bounds: Bounds { min: ZERO, max: ZERO },
        });
        admit(&mut presentation, &state);
        let frame = |server_frame: i32| Q2WireFrame {
            valid: true,
            server_frame,
            delta_frame: 0,
            suppressed_count: 0,
            area_bits: Vec::new(),
            player: PlayerState::default(),
            split_players: Vec::new(),
            entities: Vec::new(),
        };
        presentation.frame(&frame(1), &[], 1000);
        assert!(presentation.predicted.is_some());
        presentation.frame(&frame(2), &[], 1100);
        assert_eq!(predictor.borrow().received.len(), 1);
        Q2ClientPrediction::sent(&mut presentation, 7, &Usercmd::default(), 2000);
        assert_eq!(predictor.borrow().submitted, vec![(7, 2000)]);
        Q2ClientPrediction::acknowledged(&mut presentation, 7, 2000);
        assert_eq!(presentation.packet_acknowledged, 7);
    }

    #[test]
    fn player_view_follows_decoded_origin() {
        let (mut presentation, state, _) = harness();
        admit(&mut presentation, &state);
        let mut player_state = qa_net::q2::PlayerState::default();
        player_state.pmove.origin = [80, 160, 240];
        player_state.viewangles = [10.0, 20.0, 30.0];
        player_state.viewoffset = [0.0, 0.0, 22.0];
        player_state.fov = 90;
        presentation.frame(
            &Q2WireFrame {
                valid: true,
                server_frame: 1,
                delta_frame: 0,
                suppressed_count: 0,
                area_bits: Vec::new(),
                player: player_state,
                split_players: Vec::new(),
                entities: Vec::new(),
            },
            &[],
            1000,
        );
        let player = presentation.player().expect("player is admitted");
        let view = presentation.player_view(&player.actor);
        assert_eq!(
            view.origin,
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0
            }
        );
        assert_eq!(view.view_height, 22.0);
        assert_eq!(view.field_of_view, Some(90.0));
    }
}
