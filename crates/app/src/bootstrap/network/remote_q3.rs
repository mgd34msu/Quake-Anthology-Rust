//! Quake III remote presentation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/remote-q3.ts`
//! (`Q3RemotePresentation`, `Q3RemotePresentationOptions`, `Q3RemoteWorld`).
//! Q3 `CL_ParseGamestate` / `CL_SetCGameTime` and cgame host projection.
//! The donor's asynchronous admission resolves inline, preserving the
//! donor's receive order.
//!
//! Missing siblings (host-seam surface, documented per the lane rule):
//! `./q3-client.ts` (`Q3ApplicationClientHost`) is not ported yet, so the
//! presentation exposes the donor's client-host methods as inherent
//! methods and implements only [`RemotePresentationAccess`]; the canonical
//! trait adopts these methods when its lane lands. `../simulation/q3/guest-player.ts`
//! (`q3GuestPlayerUi`) plus `network/q3/adapters.ts` (`toQ3PlayerState`)
//! behind [`Q3RemoteHost::guest_player_ui`]; `network/q3/clock.ts`
//! (`Q3ClientClock`) behind [`Q3RemoteClock`];
//! `../simulation/prediction/*` (`SelectedMovementPrediction`,
//! `createPresentationMovementHost`, `PresentationPredictionAdapter`,
//! `movementProfile`, `readPredictionSourceState`, `predictionSourceHit`)
//! behind [`Q3RemotePrediction`], [`Q3RemoteHost::build_movement`], and the
//! [`Q3PredictionBase`] mirror; `../q3-client.ts`
//! (`ApplicationQ3ClientSource`) behind the [`Q3RemoteClientSource`]
//! mirror. The `presentationSourceCommand` / `relativeQ3SourceCommand`
//! pair (`../simulation/q3-commands.ts`,
//! `../simulation/prediction/presentation.ts`) is mirrored locally in
//! [`Q3RemotePresentation::command`]: on this path the relative step is a
//! proven no-op (angle space undefined, dialect `q3`) and the presentation
//! step is pure over local command types. `predictionSourceHit` is
//! mirrored locally as [`Q3SourceHit`] for the same reason.
//!
//! The connection, clock, prediction adapter, downloads, and cinematic
//! lifecycle follow the donor's ownership: the presentation stores the
//! attached [`Q3ClientConnection`](qa_net::q3_net::Q3ClientConnection) and
//! reaches content, scene, and prediction through [`Q3RemoteHost`].
//! Published snapshots use
//! [`SimulationOutput`](qa_world::session::SimulationOutput); the donor's
//! `configurations` and `scene` blocks have no `qa-world` home, so they
//! travel beside the output as [`Q3RemoteSceneOutput`].

use std::cell::RefCell;
use std::collections::HashMap;

use qa_content::catalog::{remote_content_selection, RemoteContentBase};
use qa_content::contract::{ContentId, InventoryEntry, ProviderReference, ResolvedResourceReference};
use qa_content::q3::foundation::arsenal::q3_weapon_item;
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId, SeatId};
use qa_core::math::{Bounds, Vec3};
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_net::common::commands::{ActorCommand, UserCommand};
use qa_net::q3::WireUserCommand;
use qa_net::q3_net::{
    DownloadBlock, Gamestate, Q3ClientConnection, Q3NetError, Q3PlayerState, Snapshot, SnapshotStatus,
};
use qa_world::body::BodyState;
use qa_world::collision::q3::CollisionMapSettings;
use qa_world::save::shared::{CharacterSelection, ProviderRef};
use qa_world::session::{
    EngineSession, SessionClient, SimulationOutput, SnapshotActor, SnapshotBody, SnapshotInventory,
};
use thiserror::Error;

use super::remote_world::{RemoteCollisionSettings, RemoteWorldContent, RemoteWorldError};
use super::types::{
    ApplicationNetworkPlayer, NetworkPresentationEvent, PlayerUi, PlayerView, PresentationModel,
    RemotePresentationAccess, WorldText,
};
use crate::persistence::recipe::ExecutableRecipe;

/// Zero vector (donor `zero`).
const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

/// Player body bounds (donor `bounds`).
const Q3_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -15.0,
        y: -15.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 15.0,
        y: 15.0,
        z: 32.0,
    },
};

/// Snapshot read window (donor `32` in `HistorySnapshotSource.read`).
const SNAPSHOT_WINDOW: i32 = 32;

/// Maximum entities returned by a snapshot read (donor `256`).
const MAX_SNAPSHOT_ENTITIES: usize = 256;

/// Parse-entity staleness span (donor `MAX_PARSE_ENTITIES`).
const MAX_PARSE_ENTITIES: i32 = 2048;

/// Padded area-mask length (donor `new Uint8Array(32)`).
const AREA_MASK_LEN: usize = 32;

/// Solid marker for brush entities in prediction (donor `0xffffff`).
const BRUSH_SOLID: i32 = 0x00ff_ffff;

/// Ground entity number for the world (donor `1022` in `predictionSourceHit`).
const GROUND_WORLD: i32 = 1022;

/// Ground entity number for no hit (donor `1023` in `predictionSourceHit`).
const GROUND_NONE: i32 = 1023;

/// Remote world description for content loading (donor `Q3RemoteWorld`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3RemoteWorld {
    /// World model path.
    pub map: String,
    /// Precached model paths.
    pub models: Vec<String>,
    /// Precached sound paths.
    pub sounds: Vec<String>,
}

/// Loaded-content surface the Q3 presentation reads.
pub trait Q3RemoteContent {
    /// Recipe the content was loaded from.
    fn recipe(&self) -> &ExecutableRecipe;
    /// World geometry identity (donor `content.world`).
    fn world_geometry(&self) -> &str;
}

/// Client clock (donor `Q3ClientClock`).
pub trait Q3RemoteClock {
    /// Current client time in milliseconds (donor `time`).
    fn time(&self) -> i32;
    /// Publish a snapshot (donor `publish`).
    fn publish(&mut self, snapshot: &Snapshot);
    /// Clear clock state (donor `clear`).
    fn clear(&mut self);
    /// Advance the clock (donor `advance`, `None` until the first active snapshot).
    fn advance(&mut self, real_time_ms: f64, options: &Q3ClockOptions) -> Option<i32>;
}

/// Clock advance options (donor `Q3ClockOptions`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ClockOptions {
    /// Clock paused flag.
    pub paused: bool,
    /// Time nudge in milliseconds.
    pub time_nudge: f64,
    /// Timescale multiplier.
    pub timescale: f64,
    /// Demo playback flag.
    pub demo: bool,
    /// Frozen demo flag.
    pub freeze_demo: bool,
    /// Timedemo flag.
    pub timedemo: bool,
}

/// Package downloads (donor `Q3RemotePresentationOptions['downloads']`).
pub trait Q3RemoteDownloads {
    /// Prepare downloads for a connection (donor `prepare`, true while downloading).
    fn prepare(&mut self, connection: &Q3ClientConnection<'_>) -> bool;
    /// Publish an admitted download size (donor `publishSize`).
    fn publish_size(&mut self, size: i32) -> i32;
    /// Receive a download block (donor `receive`).
    fn receive(&mut self, block: &DownloadBlock);
    /// Close downloads (donor `close`).
    fn close(&mut self);
}

/// Source-hit projection (mirrors donor `predictionSourceHit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3SourceHit {
    /// World hit.
    World,
    /// No hit.
    None,
    /// Actor hit.
    Actor(ActorId),
}

/// Predicted weapon state (donor arsenal `state` block).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PredictionWeapon {
    /// Source weapon number.
    pub source_weapon: i32,
    /// Weapon state.
    pub state: i32,
    /// Weapon time in milliseconds.
    pub time_ms: i32,
}

/// Predicted animation state (donor animation `state` block).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3PredictionAnimation {
    /// Legs animation.
    pub legs: i32,
    /// Torso animation.
    pub torso: i32,
    /// Legs timer in milliseconds.
    pub legs_timer_ms: i32,
    /// Torso timer in milliseconds.
    pub torso_timer_ms: i32,
}

/// Client-prediction snapshot (donor `MovementPredictionSnapshot` projection).
///
/// The host's snapshot assembly applies the donor's simulation-lane
/// composition around these wire-level fields: the canonical
/// `toQ3PlayerState` conversion, the `readPredictionSourceState` merge over
/// `retailSnapshot` (an identity clone, so the carried player state is
/// already the merged input), the view offset `{0, 0, viewHeight}`,
/// environment `flight`/`haste`/`invulnerable` false with gravity
/// multiplier 1, and null contact and Q3 arsenal.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PredictionBase {
    /// Current outgoing command number (donor `commands.currentNumber`).
    pub sequence: i32,
    /// Command time in milliseconds (donor `ps.commandTime`).
    pub command_time_ms: i32,
    /// Decoded player state (donor `ps`, via `retailSnapshot`).
    pub player_state: Q3PlayerState,
    /// Ground hit (donor `predictionSourceHit(ps.groundEntityNum)`).
    pub ground: Q3SourceHit,
    /// Jump-pad actor (donor `jumppadEnt`, none when zero).
    pub jump_pad: Option<ActorId>,
    /// Active weapon item id (donor `q3WeaponItem(ps.weapon)?.item`).
    pub active_weapon: Option<String>,
    /// Ammunition inventory (donor `playerUi` inventory).
    pub ammo: Vec<InventoryEntry>,
    /// Weapon state block.
    pub weapon: Q3PredictionWeapon,
    /// Animation state block.
    pub animation: Q3PredictionAnimation,
    /// Health (donor `ps.stats.get(0)`).
    pub health: i32,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
}

/// Predictor construction seed (donor `createPresentationMovementHost` options).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3MovementSeed<'recipe> {
    /// Owned prediction actor.
    pub actor: OwnedActor,
    /// Local seat.
    pub seat: SeatId,
    /// Executable recipe (donor `recipe`, read during construction).
    pub recipe: &'recipe ExecutableRecipe,
    /// Initial snapshot.
    pub base: Q3PredictionBase,
}

/// Movement predictor behind the replica (donor `PresentationPredictionAdapter`).
///
/// The host binds the movement profile, scene, standing bounds
/// (`-15/-15/-24` to `15/15/32`), and standing view height (`26`) from the
/// seed; per-snapshot brush actors arrive on every capture because the
/// donor's `isBrush` closes over the current snapshot entities.
pub trait Q3RemotePrediction {
    /// Capture a server snapshot (donor `capture`).
    fn capture(&mut self, base: &Q3PredictionBase, brushes: &[ActorId]);
}

/// Host callbacks behind [`Q3RemotePresentationOptions`].
pub trait Q3RemoteHost {
    /// Loaded content handle.
    type Content: Q3RemoteContent;
    /// Scene queries handle.
    type Scene;
    /// Client clock.
    type Clock: Q3RemoteClock;
    /// Movement predictor.
    type Prediction: Q3RemotePrediction;
    /// Build scene queries for loaded content.
    fn build_scene(content: &Self::Content) -> Self::Scene;
    /// Build the client clock.
    fn build_clock(&mut self) -> Self::Clock;
    /// Build the movement predictor.
    fn build_movement(&mut self, seed: &Q3MovementSeed<'_>) -> Self::Prediction;
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
    /// Client time nudge in milliseconds (donor `timeNudge`, `0` when unbound).
    fn time_nudge(&self) -> f64 {
        0.0
    }
    /// Client timescale multiplier (donor `timescale`, `1` when unbound).
    fn timescale(&self) -> f64 {
        1.0
    }
    /// Client userinfo string.
    fn userinfo(&self) -> String;
    /// Load content for an admitted world and connection.
    fn load_content(&mut self, world: &Q3RemoteWorld, connection: &Q3ClientConnection<'_>) -> Self::Content;
    /// Shut down the previous connection binding (donor `shutdown`, no-op when unbound).
    fn shutdown(&mut self) {}
    /// Whether the host binds connection setup (donor `initialize` definedness).
    fn has_initializer(&self) -> bool {
        false
    }
    /// Set up a connection binding (donor `initialize`).
    fn initialize(&mut self, _connection: &Q3ClientConnection<'_>) {}
    /// Download sink, when the session lane binds one.
    fn downloads(&mut self) -> Option<&mut dyn Q3RemoteDownloads> {
        None
    }
    /// Guest HUD state (donor `q3GuestPlayerUi(toQ3PlayerState(ps), source,
    /// time, undefined, ps.product)`); the host applies the missing adapter
    /// and UI builder to the wire player state.
    fn guest_player_ui(&self, player: &Q3PlayerState, source: &ProviderReference, server_time_ms: i32) -> PlayerUi;
}

/// Quake III remote presentation options (donor `Q3RemotePresentationOptions`).
pub struct Q3RemotePresentationOptions<H: Q3RemoteHost> {
    /// Identity authority.
    pub identity: IdentityOwner,
    /// Engine session.
    pub session: EngineSession,
    /// Bound client.
    pub client: SessionClient,
    /// Preloaded content, if any.
    pub content: Option<H::Content>,
    /// Host callbacks.
    pub host: H,
}

/// Cgame source mode (donor `sourceMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SourceMode {
    /// Demo playback.
    Demo,
    /// Live connection.
    Live,
}

/// Latest snapshot cursor (donor `source.current()` return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3SnapshotCursor {
    /// Message number.
    pub number: i32,
    /// Server time in milliseconds.
    pub server_time: i32,
}

/// Snapshot history read failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3RemoteHistoryError {
    /// Snapshot number is newer than the latest snapshot.
    #[error("CL_GetSnapshot: snapshotNumber > cl.snapshot.messageNum")]
    SnapshotTooNew,
    /// Stored snapshot area mask exceeds 32 bytes.
    #[error("Q3 snapshot area mask exceeds 32 bytes")]
    BadAreaMask,
    /// Underlying network failure.
    #[error("{0}")]
    Net(#[from] Q3NetError),
}

/// Actor configuration beside the snapshot (donor `configurations` block).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3RemoteConfiguration {
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

/// Scene blocks beside the snapshot (donor `configurations`/`scene` blocks).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3RemoteSceneOutput {
    /// World geometry resource.
    pub world_resource: crate::persistence::recipe::ResolvedResourceReference,
    /// World geometry identity.
    pub world_geometry: String,
    /// Actor configurations.
    pub configurations: Vec<Q3RemoteConfiguration>,
    /// Snapshot area mask.
    pub area_bits: Vec<u8>,
}

impl RemoteCollisionSettings for CollisionMapSettings {
    fn register_map(&mut self) {
        CollisionMapSettings::register_map(self).unwrap_or_else(|error| panic!("{error}"));
    }
}

/// Quake III remote presentation (donor `Q3RemotePresentation`).
pub struct Q3RemotePresentation<'conn, H: Q3RemoteHost> {
    options: Q3RemotePresentationOptions<H>,
    connection: Option<Q3ClientConnection<'conn>>,
    world: RemoteWorldContent<H::Content, H::Scene, CollisionMapSettings>,
    clock: H::Clock,
    actors: RefCell<HashMap<u32, ActorId>>,
    current: Option<Snapshot>,
    loading_downloads: bool,
    published: Option<SimulationOutput>,
    published_scene: Option<Q3RemoteSceneOutput>,
    game_state_message: i32,
    game_state_commands: i32,
    prediction: Option<H::Prediction>,
}

fn provider_id(provider: &str) -> ProviderId {
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    ProviderId::new(namespace, name)
}

fn vec3(value: [f32; 3]) -> Vec3 {
    Vec3 {
        x: value[0],
        y: value[1],
        z: value[2],
    }
}

/// Map a HUD inventory entry onto the snapshot entry shape.
fn snapshot_entry(entry: &InventoryEntry) -> qa_world::inventory::InventoryEntry {
    use qa_content::contract::{InventoryCountPolicy, SourceCounterArithmetic};
    use qa_world::inventory::{CountArithmetic, CountPolicy};
    qa_world::inventory::InventoryEntry {
        item: entry.item.clone(),
        count: entry.count,
        capacity: entry.capacity,
        count_policy: entry.count_policy.map(|policy| match policy {
            InventoryCountPolicy::Stack => CountPolicy::Stack,
            InventoryCountPolicy::SourceCounter(arithmetic) => CountPolicy::SourceCounter(match arithmetic {
                SourceCounterArithmetic::Binary32 => CountArithmetic::Binary32,
                SourceCounterArithmetic::Binary64 => CountArithmetic::Binary64,
                SourceCounterArithmetic::Int32 => CountArithmetic::Int32,
            }),
        }),
    }
}

/// Read an info value (donor `q3InfoValue`).
fn info_value(info: &str, key: &str) -> String {
    qa_net::q3_net::q3_info_value(info, key).unwrap_or_else(|error| panic!("{error}"))
}

/// Whether a map name is admissible (donor `/^[A-Za-z0-9_/-]+$/` without `..`).
fn valid_map_name(map: &str) -> bool {
    !map.is_empty()
        && map
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '/' | '-'))
        && !map.contains("..")
}

impl<'conn, H: Q3RemoteHost> Q3RemotePresentation<'conn, H> {
    /// Build a presentation over its options (donor `constructor`).
    pub fn new(mut options: Q3RemotePresentationOptions<H>) -> Self {
        let world = RemoteWorldContent::new(
            options.content.take(),
            |content: &H::Content, _: Option<&CollisionMapSettings>| H::build_scene(content),
        );
        let clock = options.host.build_clock();
        Self {
            options,
            connection: None,
            world,
            clock,
            actors: RefCell::new(HashMap::new()),
            current: None,
            loading_downloads: false,
            published: None,
            published_scene: None,
            game_state_message: 0,
            game_state_commands: 0,
            prediction: None,
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

    /// Borrow the scene queries.
    pub fn scene(&mut self) -> Result<&H::Scene, RemoteWorldError> {
        self.world.scene()
    }

    /// Bind collision settings onto the live scene (donor `bindCollisionSettings`).
    pub fn bind_collision_settings(&mut self, settings: CollisionMapSettings) {
        self.world.bind_collision_settings(settings);
    }

    /// Latest published output, if any.
    #[must_use]
    pub fn output(&self) -> Option<&SimulationOutput> {
        self.published.as_ref()
    }

    /// Scene blocks beside the latest output, if any.
    #[must_use]
    pub fn scene_output(&self) -> Option<&Q3RemoteSceneOutput> {
        self.published_scene.as_ref()
    }

    /// Whether package downloads are in flight (donor `downloading`).
    #[must_use]
    pub fn downloading(&self) -> bool {
        self.loading_downloads
    }

    /// Current player, if a snapshot is decoded (donor `player`).
    #[must_use]
    pub fn player(&self) -> Option<ApplicationNetworkPlayer> {
        let current = self.current.as_ref()?;
        Some(ApplicationNetworkPlayer {
            client: self.options.client.id().clone(),
            actor: self.actor_at(current.player_state.client_num),
            source_entity: current.player_state.client_num as u32,
        })
    }

    /// Admitted player from the decoded connection (donor `admittedPlayer`).
    #[must_use]
    pub fn admitted_player(&self) -> ApplicationNetworkPlayer {
        let Some(connection) = self.connection.as_ref() else {
            panic!("Q3 seat has no decoded connection");
        };
        ApplicationNetworkPlayer {
            client: self.options.client.id().clone(),
            actor: self.actor_at(connection.client_number),
            source_entity: connection.client_number as u32,
        }
    }

    /// First decoded player state (donor `initialPlayer`).
    #[must_use]
    pub fn initial_player(&self) -> &Q3PlayerState {
        match self.current.as_ref() {
            Some(current) => &current.player_state,
            None => panic!("Q3 cgame needs its first decoded player state"),
        }
    }

    /// Cgame connection source (donor `cgameSource`).
    pub fn cgame_source(&mut self) -> Q3RemoteClientSource<'_, 'conn, H> {
        if self.connection.is_none() {
            panic!("Q3 cgame has no connection source");
        }
        Q3RemoteClientSource { presentation: self }
    }

    /// Copied game-state strings (donor `sourceRecords`).
    #[must_use]
    pub fn source_records(&self) -> Vec<String> {
        self.connection
            .as_ref()
            .map_or(Vec::new(), |connection| connection.game_state.copy_strings())
    }

    /// Actor for a source entity number (donor `actorAt`).
    pub fn actor_at(&self, number: i32) -> ActorId {
        if !(0..1024).contains(&number) {
            panic!("Invalid Q3 source entity");
        }
        let number = number as u32;
        if let Some(found) = self.actors.borrow().get(&number).cloned() {
            return found;
        }
        let value = self
            .options
            .identity
            .actor(number, self.options.host.next_generation(number));
        self.actors.borrow_mut().insert(number, value.clone());
        value
    }

    /// Source entity number behind an actor, if registered (donor `numberOf`).
    #[must_use]
    pub fn number_of(&self, actor: &ActorId) -> Option<u32> {
        self.actors
            .borrow()
            .iter()
            .find_map(|(number, value)| if value == actor { Some(*number) } else { None })
    }

    /// Whether an actor is a player (donor `isPlayer`).
    #[must_use]
    pub fn is_player(&self, actor: &ActorId) -> bool {
        self.number_of(actor).is_some_and(|number| number < 64)
    }

    /// Attach a decoded connection (donor `attach`).
    pub fn attach(&mut self, connection: Q3ClientConnection<'conn>) {
        self.connection = Some(connection);
    }

    /// Clear the active connection binding (donor `clearActive`, resolves inline).
    pub fn clear_active(&mut self, assert_current: &dyn Fn()) {
        assert_current();
        self.options.host.shutdown();
        assert_current();
        if let Some(downloads) = self.options.host.downloads() {
            downloads.close();
        }
        self.loading_downloads = false;
        self.actors.borrow_mut().clear();
        self.current = None;
        self.published = None;
        self.published_scene = None;
        self.prediction = None;
        self.clock.clear();
    }

    /// Handle a system-info string (donor `systemInfo`).
    pub fn system_info(&self, info: &str) {
        let demo = self.connection.as_ref().is_some_and(|connection| connection.is_demo());
        if !demo
            && info_value(info, "sv_pure").parse::<f64>().unwrap_or(0.0) != 0.0
            && !self.options.host.has_initializer()
        {
            panic!("This server requires pure verification, which is not supported yet.");
        }
        remote_content_selection(RemoteContentBase::Q3Baseq3, &info_value(info, "fs_game")).expect("selection");
    }

    /// Handle admitted game state (donor `gamestate`, resolves inline).
    pub fn gamestate(&mut self, state: &Gamestate, _generation: i32, assert_current: &dyn Fn()) {
        assert_current();
        let Some(connection) = self.connection.as_ref() else {
            panic!("Q3 gamestate has no connection");
        };
        let info = connection
            .game_state
            .get(0)
            .unwrap_or_else(|error| panic!("{error}"))
            .unwrap_or_default();
        if info_value(&info, "protocol") != "68" {
            panic!("Q3 remote requires protocol 68");
        }
        let game_type = info_value(&info, "g_gametype").parse::<f64>().unwrap_or(f64::NAN);
        if !game_type.is_finite() || game_type.fract() != 0.0 || game_type < 0.0 || game_type > 4.0 {
            panic!("Q3 remote requires a baseq3 game type");
        }
        let map = info_value(&info, "mapname");
        if !valid_map_name(&map) {
            panic!("Invalid Q3 remote map name");
        }
        let demo = connection.is_demo();
        let names = |first: usize, count: usize| -> Vec<String> {
            let connection = self.connection.as_ref().expect("Q3 gamestate has no connection");
            (0..count)
                .filter_map(|index| {
                    connection
                        .game_state
                        .get(first + index)
                        .unwrap_or_else(|error| panic!("{error}"))
                })
                .filter(|value| !value.is_empty())
                .collect()
        };
        let models = names(32, 256);
        let sounds = names(288, 256);
        self.game_state_message = self
            .connection
            .as_ref()
            .expect("Q3 gamestate has no connection")
            .server_message_sequence;
        self.game_state_commands = state.command_sequence;
        let downloading = if demo {
            false
        } else {
            let connection = self.connection.as_ref().expect("Q3 gamestate has no connection");
            match self.options.host.downloads() {
                Some(downloads) => downloads.prepare(connection),
                None => false,
            }
        };
        assert_current();
        self.loading_downloads = downloading;
        if self.loading_downloads {
            return;
        }
        let world = Q3RemoteWorld {
            map: format!("maps/{map}.bsp"),
            models,
            sounds,
        };
        let loaded = {
            let connection = self.connection.as_ref().expect("Q3 gamestate has no connection");
            self.options.host.load_content(&world, connection)
        };
        assert_current();
        self.world.set_content(loaded);
        if self.options.host.has_initializer() {
            let connection = self.connection.as_ref().expect("Q3 gamestate has no connection");
            self.options.host.initialize(connection);
        }
        assert_current();
    }

    /// Admit a package download size (donor `downloadSize`).
    pub fn download_size(&mut self, size: i32) -> i32 {
        if self.connection.as_ref().is_some_and(|connection| connection.is_demo()) {
            panic!("Q3 demo cannot request package downloads");
        }
        match self.options.host.downloads() {
            Some(downloads) => downloads.publish_size(size),
            None => panic!("Q3 package downloads have no writable content owner"),
        }
    }

    /// Receive a package download block (donor `download`, resolves inline).
    pub fn download(&mut self, block: &DownloadBlock) {
        if self.connection.as_ref().is_some_and(|connection| connection.is_demo()) {
            panic!("Q3 demo cannot request package downloads");
        }
        match self.options.host.downloads() {
            Some(downloads) => downloads.receive(block),
            None => panic!("Q3 package downloads have no writable content owner"),
        }
    }

    /// Handle a decoded snapshot (donor `snapshot`).
    pub fn snapshot(&mut self, snapshot: &Snapshot, _ping: i32) {
        self.current = Some(snapshot.clone());
        self.clock.publish(snapshot);
        self.capture();
        self.publish(snapshot.server_time);
    }

    /// Handle a map restart (donor `mapRestart`).
    pub fn map_restart(&mut self) {
        self.capture();
    }

    /// Capture the current snapshot into the predictor, when bound.
    fn capture(&mut self) {
        if self.prediction.is_none() || self.current.is_none() {
            return;
        }
        let base = self.prediction_snapshot();
        let brushes = self.brushes();
        if let Some(prediction) = self.prediction.as_mut() {
            prediction.capture(&base, &brushes);
        }
    }

    /// Current brush-entity actors (owned-data projection of donor `isBrush`).
    fn brushes(&self) -> Vec<ActorId> {
        self.current.as_ref().map_or(Vec::new(), |current| {
            current
                .entities
                .iter()
                .filter(|entity| entity.solid == BRUSH_SOLID)
                .map(|entity| self.actor_at(entity.number))
                .collect()
        })
    }

    fn require_player(&self, actor: &ActorId) -> &Snapshot {
        match self.current.as_ref() {
            Some(current) if self.actor_at(current.player_state.client_num) == *actor => current,
            _ => panic!("No decoded Q3 player state"),
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
        if self.current.is_none() && self.admitted_player().actor == *actor {
            return PlayerView {
                origin: ZERO,
                angles: ZERO,
                view_height: 0.0,
                blend: None,
                damage_blend: None,
                kick_angles: None,
                field_of_view: None,
                client_view_offset_delta: None,
                foreign_character_death: None,
                pitch_drift: None,
            };
        }
        let ps = &self.require_player(actor).player_state;
        PlayerView {
            origin: vec3(ps.origin),
            angles: vec3(ps.viewangles),
            view_height: f64::from(ps.viewheight),
            blend: None,
            damage_blend: None,
            kick_angles: None,
            field_of_view: None,
            client_view_offset_delta: None,
            foreign_character_death: None,
            pitch_drift: None,
        }
    }

    /// HUD state for an actor (donor `playerUi`).
    #[must_use]
    pub fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        let current = self.require_player(actor).clone();
        let ps = &current.player_state;
        let source = self
            .content()
            .recipe()
            .weapons
            .first()
            .unwrap_or_else(|| panic!("Q3 remote has no native weapon provider"));
        let (namespace, provider) = source.provider.split_once(':').unwrap_or(("", ""));
        self.options.host.guest_player_ui(
            ps,
            &ProviderReference {
                provider: ProviderId::new(namespace, provider),
                content: ContentId(source.content.clone()),
            },
            self.current
                .as_ref()
                .map_or(ps.command_time, |current| current.server_time),
        )
    }

    /// Visible character views (donor `characterViews`, always empty).
    #[must_use]
    pub fn character_views(&self) -> Vec<Q3CharacterView> {
        Vec::new()
    }

    /// Visible presentations (donor `presentations`, always empty).
    #[must_use]
    pub fn presentations(&self) -> Vec<PresentationModel> {
        Vec::new()
    }

    /// Register a resolved resource (donor `registerResource`, ignored).
    pub fn register_resource(&mut self, _content: &ContentId, _path: &str, _resource: &ResolvedResourceReference) {}

    /// Run a player command (donor `playerCommand`).
    pub fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        self.require_player(actor);
        if std::iter::once(name)
            .chain(args.iter().map(String::as_str))
            .any(|value| value.contains(['"', '\n', '\r', ';']))
        {
            panic!("Invalid Q3 command delimiter");
        }
        let mut text = name.to_string();
        for arg in args {
            text.push_str(&format!(" \"{arg}\""));
        }
        self.options.host.send_command(&text);
    }

    /// Convert an actor command into a wire command (donor `command`).
    ///
    /// The donor's `relativeQ3SourceCommand` is a no-op on this path: the
    /// angle space is undefined and the dialect is `q3`, so the relative
    /// step returns its input unchanged. The `presentationSourceCommand`
    /// `q3` branch it feeds is pure over local command types and mirrored
    /// below.
    #[must_use]
    pub fn command(&self, command: &ActorCommand) -> WireUserCommand {
        let snapshot = self.require_player(&command.actor).clone();
        let UserCommand::Q3 {
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
            ..
        } = &command.command
        else {
            panic!("Native Q3 remote requires Q3 commands");
        };
        let _delta = snapshot.player_state.delta_angles;
        let buttons = match &command.arsenal {
            None => *buttons as i32,
            Some(arsenal) => ((*buttons as i32) & !4) | (i32::from(arsenal.use_holdable) * 4),
        };
        let angles = [
            (angle_words[0] as i32) & 0xffff,
            (angle_words[1] as i32) & 0xffff,
            (angle_words[2] as i32) & 0xffff,
        ];
        WireUserCommand {
            server_time: self.clock.time(),
            angles: [angles[0] as i16, angles[1] as i16, angles[2] as i16],
            moves: [*forward_move as i8, *right_move as i8, *up_move as i8],
            buttons: buttons as u16,
            weapon: *weapon as u8,
        }
    }

    /// Publish a snapshot at a client time (donor `publish`).
    fn publish(&mut self, time: i32) {
        let (Some(snapshot), Some(player)) = (self.current.clone(), self.player()) else {
            return;
        };
        let recipe = self.content().recipe().clone();
        let ps = &snapshot.player_state;
        let mut bodies: Vec<SnapshotBody> = snapshot
            .entities
            .iter()
            .filter(|entity| entity.number != ps.client_num)
            .map(|entity| SnapshotBody {
                id: SavedActorId::from(&self.actor_at(entity.number)),
                state: BodyState {
                    origin: vec3(entity.pos.base),
                    angles: vec3(entity.apos.base),
                    velocity: vec3(entity.pos.delta),
                    bounds: Q3_BOUNDS,
                    ground: None,
                },
            })
            .collect();
        bodies.push(SnapshotBody {
            id: SavedActorId::from(&player.actor),
            state: BodyState {
                origin: vec3(ps.origin),
                angles: vec3(ps.viewangles),
                velocity: vec3(ps.velocity),
                bounds: Q3_BOUNDS,
                ground: None,
            },
        });
        let owner = provider_id(&recipe.map.entities.provider);
        let published = SimulationOutput {
            snapshot: qa_world::session::WorldSnapshot {
                frame: FrameContext {
                    frame: snapshot.message_number,
                    time: SourceTime::Milliseconds(time),
                    elapsed: SourceTime::Milliseconds(0),
                    phase: FramePhase::FrameExit,
                },
                actors: bodies
                    .iter()
                    .map(|body| SnapshotActor {
                        id: body.id,
                        owner: owner.clone(),
                        definition: "q3:remote-entity".to_string(),
                    })
                    .collect(),
                bodies,
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
        self.published_scene = Some(Q3RemoteSceneOutput {
            world_resource: recipe.map.geometry.clone(),
            world_geometry: self.content().world_geometry().to_string(),
            configurations: vec![Q3RemoteConfiguration {
                actor: player.actor.clone(),
                movement: recipe.movement.clone(),
                character: recipe.character.clone(),
                weapons: recipe.weapons.clone(),
                inventory: recipe.inventory.clone(),
            }],
            area_bits: snapshot.area_mask.clone(),
        });
        self.published = Some(published);
        if let Some(published) = self.published.clone() {
            self.options.host.publish(&published);
        }
    }

    /// Sample presentation time (donor `samplePresentation`).
    pub fn sample_presentation(&mut self, now: u64) -> Option<&SimulationOutput> {
        let demo = self.connection.as_ref().is_some_and(|connection| connection.is_demo());
        let time = if demo {
            self.current.as_ref().map(|_| self.clock.time())
        } else {
            let real_time = self.options.host.presentation_time().unwrap_or(now) as f64;
            let options = Q3ClockOptions {
                paused: false,
                time_nudge: self.options.host.time_nudge(),
                timescale: self.options.host.timescale(),
                demo: false,
                freeze_demo: false,
                timedemo: false,
            };
            self.clock.advance(real_time, &options)
        };
        let time = time?;
        self.publish(time);
        self.published.as_ref()
    }

    /// Prediction snapshot for the current snapshot (donor `predictionSnapshot`).
    fn prediction_snapshot(&self) -> Q3PredictionBase {
        let (Some(current), Some(player)) = (self.current.as_ref(), self.player()) else {
            panic!("Q3 prediction needs a snapshot");
        };
        let ps = current.player_state.clone();
        let ui = self.player_ui(&player.actor);
        Q3PredictionBase {
            sequence: self
                .connection
                .as_ref()
                .map_or(0, |connection| connection.commands.current_number()),
            command_time_ms: ps.command_time,
            ground: match ps.ground_entity_num {
                GROUND_WORLD => Q3SourceHit::World,
                GROUND_NONE => Q3SourceHit::None,
                number => Q3SourceHit::Actor(self.actor_at(number)),
            },
            jump_pad: if ps.jumppad_ent == 0 {
                None
            } else {
                Some(self.actor_at(ps.jumppad_ent))
            },
            active_weapon: q3_weapon_item(ps.weapon).map(|definition| definition.item.clone()),
            ammo: ui.inventory,
            weapon: Q3PredictionWeapon {
                source_weapon: ps.weapon,
                state: ps.weapon_state,
                time_ms: ps.weapon_time,
            },
            animation: Q3PredictionAnimation {
                legs: ps.legs_anim,
                torso: ps.torso_anim,
                legs_timer_ms: ps.legs_timer,
                torso_timer_ms: ps.torso_timer,
            },
            health: ps.stats.get(0).expect("player stat slot 0 exists"),
            view_angles: vec3(ps.viewangles),
            view_height: f64::from(ps.viewheight),
            player_state: ps,
        }
    }

    /// Movement predictor for a seat (donor `movement`).
    pub fn movement(&mut self, seat: SeatId) -> &mut H::Prediction {
        if self.prediction.is_none() {
            let Some(player) = self.player() else {
                panic!("Q3 prediction has no player");
            };
            let recipe = self.content().recipe().clone();
            let owned = self
                .options
                .identity
                .owned_actor(&player.actor, provider_id(&recipe.map.entities.provider))
                .expect("prediction actor belongs to the presentation session");
            let seed = Q3MovementSeed {
                actor: owned,
                seat,
                recipe: &recipe,
                base: self.prediction_snapshot(),
            };
            self.prediction = Some(self.options.host.build_movement(&seed));
        }
        self.prediction.as_mut().expect("predictor is built")
    }

    /// Drain presentation events (donor `drainPresentationEvents`, always empty).
    #[must_use]
    pub fn drain_presentation_events(&self) -> Vec<NetworkPresentationEvent> {
        Vec::new()
    }

    /// Print client text (donor `print`).
    pub fn print(&mut self, text: &str) {
        self.options.host.print(text);
    }

    /// Handle a disconnect (donor `disconnected`).
    pub fn disconnected(&mut self, reason: &str) {
        self.print(&format!("{reason}\n"));
        self.options.host.disconnected(reason);
    }
}

/// Cgame connection source (mirrors donor `ApplicationQ3ClientSource`).
///
/// Snapshot reads mirror the donor's `HistorySnapshotSource` window,
/// validity, staleness, area-mask, and entity-truncation rules over the
/// network history, following the local-client precedent; the int32 range
/// check is inherent to `i32`.
pub struct Q3RemoteClientSource<'source, 'conn, H: Q3RemoteHost> {
    presentation: &'source mut Q3RemotePresentation<'conn, H>,
}

impl<'source, 'conn, H: Q3RemoteHost> Q3RemoteClientSource<'source, 'conn, H> {
    fn connection(&self) -> &Q3ClientConnection<'conn> {
        self.presentation
            .connection
            .as_ref()
            .expect("connection source is attached")
    }

    /// Source mode (donor `sourceMode`).
    #[must_use]
    pub fn source_mode(&self) -> Q3SourceMode {
        if self.connection().is_demo() {
            Q3SourceMode::Demo
        } else {
            Q3SourceMode::Live
        }
    }

    /// Client time in milliseconds (donor `time`).
    #[must_use]
    pub fn time(&self) -> i32 {
        self.presentation.clock.time()
    }

    /// Client number (donor `clientNumber`).
    #[must_use]
    pub fn client_number(&self) -> i32 {
        self.connection().client_number
    }

    /// Server message sequence at admission (donor `serverMessageSequence`).
    #[must_use]
    pub fn server_message_sequence(&self) -> i32 {
        self.presentation.game_state_message
    }

    /// Last executed server command at admission (donor `lastExecutedServerCommand`).
    #[must_use]
    pub fn last_executed_server_command(&self) -> i32 {
        self.presentation.game_state_commands
    }

    /// Latest snapshot cursor (donor `current()`).
    #[must_use]
    pub fn current(&self) -> Q3SnapshotCursor {
        self.connection().history.latest().map_or(
            Q3SnapshotCursor {
                number: 0,
                server_time: 0,
            },
            |latest| Q3SnapshotCursor {
                number: latest.message_number,
                server_time: latest.server_time,
            },
        )
    }

    /// Read a snapshot by number (donor `read()`).
    pub fn read(&mut self, number: i32) -> Result<Option<Snapshot>, Q3RemoteHistoryError> {
        let (snapshot, parse_number) = {
            let connection = self
                .presentation
                .connection
                .as_ref()
                .expect("connection source is attached");
            let latest = connection.history.latest();
            let latest_number = latest.as_ref().map_or(0, |latest| latest.message_number);
            if number > latest_number {
                return Err(Q3RemoteHistoryError::SnapshotTooNew);
            }
            if latest_number.wrapping_sub(number) >= SNAPSHOT_WINDOW || latest.is_none() {
                return Ok(None);
            }
            let Some(entry) = connection.history.read_slot(number)? else {
                return Ok(None);
            };
            if entry.status != SnapshotStatus::Valid {
                return Ok(None);
            }
            (entry.snapshot, connection.parse_entities.number)
        };
        if parse_number.wrapping_sub(snapshot.parse_entities_number) >= MAX_PARSE_ENTITIES {
            return Ok(None);
        }
        if snapshot.area_mask.len() > AREA_MASK_LEN {
            return Err(Q3RemoteHistoryError::BadAreaMask);
        }
        let mut area_mask = vec![0u8; AREA_MASK_LEN];
        area_mask[..snapshot.area_mask.len()].copy_from_slice(&snapshot.area_mask);
        let mut entities = snapshot.entities.clone();
        if entities.len() > MAX_SNAPSHOT_ENTITIES {
            self.presentation.options.host.print(&format!(
                "CL_GetSnapshot: truncated {} entities to {MAX_SNAPSHOT_ENTITIES}\n",
                entities.len()
            ));
            entities.truncate(MAX_SNAPSHOT_ENTITIES);
        }
        Ok(Some(Snapshot {
            message_number: snapshot.message_number,
            server_time: snapshot.server_time,
            delta_number: snapshot.delta_number,
            flags: snapshot.flags,
            server_command_number: snapshot.server_command_number,
            parse_entities_number: snapshot.parse_entities_number,
            area_mask,
            player_state: snapshot.player_state.clone(),
            entities,
        }))
    }

    /// Actor for a source entity number (donor `actorAt()`).
    pub fn actor_at(&self, number: i32) -> ActorId {
        self.presentation.actor_at(number)
    }

    /// Copied game-state strings (donor `getGameState()`).
    #[must_use]
    pub fn get_game_state(&self) -> Vec<String> {
        self.connection().game_state.copy_strings()
    }

    /// System-info string (donor `systemInfo()`).
    #[must_use]
    pub fn system_info(&self) -> String {
        self.connection()
            .game_state
            .get(1)
            .unwrap_or_else(|error| panic!("{error}"))
            .unwrap_or_default()
    }

    /// Server command by sequence (donor `getServerCommand()`).
    pub fn get_server_command(&mut self, sequence: i32) -> Result<Option<Vec<String>>, Q3NetError> {
        self.presentation
            .connection
            .as_mut()
            .expect("connection source is attached")
            .get_server_command(sequence)
    }

    /// Snapshot ping (donor `snapshotPing()`).
    pub fn snapshot_ping(&self, number: i32) -> Result<Option<i32>, Q3NetError> {
        self.connection().snapshot_ping(number)
    }

    /// Current outgoing user-command number (donor `commands.currentNumber`).
    #[must_use]
    pub fn commands_current_number(&self) -> i32 {
        self.connection().commands.current_number()
    }

    /// Read a recorded user command (donor `commands.read()`).
    pub fn commands_read(&self, number: i32) -> Result<Option<WireUserCommand>, Q3NetError> {
        self.connection().commands.read(number)
    }
}

impl<H: Q3RemoteHost + 'static> RemotePresentationAccess for Q3RemotePresentation<'_, H> {
    fn world_text(&self) -> Vec<WorldText> {
        Q3RemotePresentation::world_text(self)
    }

    fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        Q3RemotePresentation::player_ui(self, actor)
    }

    fn character_views(&self) -> Vec<Q3CharacterView> {
        Q3RemotePresentation::character_views(self)
    }

    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        Q3RemotePresentation::player_command(self, actor, name, args);
    }

    fn presentations(&self) -> Vec<PresentationModel> {
        Q3RemotePresentation::presentations(self)
    }

    fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference) {
        Q3RemotePresentation::register_resource(self, content, path, resource);
    }

    fn player_view(&self, actor: &ActorId) -> PlayerView {
        Q3RemotePresentation::player_view(self, actor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_net::q3_net::{Q3PlayerSlots, Q3Product};
    use qa_world::session::SessionMode;

    use crate::bootstrap::network::types::{
        ArsenalWarning, PlayerAmmo, PlayerArsenalItem, WeaponAmmoStatus, WeaponStatus,
    };
    use crate::persistence::recipe::fixture_recipe;

    #[derive(Debug, Clone)]
    struct TestContent {
        recipe: ExecutableRecipe,
    }

    impl Q3RemoteContent for TestContent {
        fn recipe(&self) -> &ExecutableRecipe {
            &self.recipe
        }

        fn world_geometry(&self) -> &str {
            "test-world"
        }
    }

    #[derive(Debug)]
    struct TestScene;

    #[derive(Debug, Default)]
    struct TestClock {
        time: i32,
        published: Vec<i32>,
        advanced: Vec<f64>,
    }

    impl Q3RemoteClock for TestClock {
        fn time(&self) -> i32 {
            self.time
        }

        fn publish(&mut self, snapshot: &Snapshot) {
            self.published.push(snapshot.server_time);
        }

        fn clear(&mut self) {
            self.time = 0;
            self.published.clear();
            self.advanced.clear();
        }

        fn advance(&mut self, real_time_ms: f64, _options: &Q3ClockOptions) -> Option<i32> {
            self.advanced.push(real_time_ms);
            match self.published.first().copied() {
                Some(server_time) => {
                    self.time = server_time;
                    Some(server_time)
                }
                None => None,
            }
        }
    }

    #[derive(Debug, Default)]
    struct TestPrediction {
        captured: Vec<(Q3PredictionBase, Vec<ActorId>)>,
    }

    impl Q3RemotePrediction for TestPrediction {
        fn capture(&mut self, base: &Q3PredictionBase, brushes: &[ActorId]) {
            self.captured.push((base.clone(), brushes.to_vec()));
        }
    }

    #[derive(Debug, Default)]
    struct TestHostState {
        sent: Vec<String>,
        printed: Vec<String>,
        published: usize,
        disconnected: Vec<String>,
        generation: u32,
        built: usize,
    }

    struct TestHost {
        state: Rc<RefCell<TestHostState>>,
        prediction: Rc<RefCell<TestPrediction>>,
    }

    impl Q3RemoteHost for TestHost {
        type Content = TestContent;
        type Scene = TestScene;
        type Clock = TestClock;
        type Prediction = SharedPrediction;

        fn build_scene(_content: &TestContent) -> TestScene {
            TestScene
        }

        fn build_clock(&mut self) -> TestClock {
            TestClock::default()
        }

        fn build_movement(&mut self, _seed: &Q3MovementSeed<'_>) -> SharedPrediction {
            self.state.borrow_mut().built += 1;
            SharedPrediction {
                inner: self.prediction.clone(),
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

        fn load_content(&mut self, _world: &Q3RemoteWorld, _connection: &Q3ClientConnection<'_>) -> TestContent {
            TestContent {
                recipe: fixture_recipe(),
            }
        }

        fn guest_player_ui(
            &self,
            player: &Q3PlayerState,
            _source: &ProviderReference,
            server_time_ms: i32,
        ) -> PlayerUi {
            PlayerUi {
                selected_arsenal: None,
                native_inventory: None,
                health: f64::from(player.stats.get(0).unwrap_or(0)),
                armor: qa_content::contract::ArmorState {
                    regular: qa_content::contract::RegularArmorState::None,
                    powered: qa_content::contract::PoweredProtectionState::None,
                },
                active_weapon: None,
                ammo: Some(PlayerAmmo {
                    item: "q3:ammo/machinegun".to_string(),
                    count: f64::from(server_time_ms),
                }),
                inventory: Vec::new(),
                arsenal_warning: ArsenalWarning::None,
                powerups: Vec::new(),
                items: Vec::<PlayerArsenalItem>::new(),
                weapon_status: Some(WeaponStatus {
                    source: ProviderReference {
                        provider: ProviderId::new("", ""),
                        content: ContentId("test".to_string()),
                    },
                    item: "q3:weapon/machinegun".to_string(),
                    label: "Machinegun".to_string(),
                    ammo: WeaponAmmoStatus::Unmetered,
                }),
            }
        }
    }

    struct SharedPrediction {
        inner: Rc<RefCell<TestPrediction>>,
    }

    impl Q3RemotePrediction for SharedPrediction {
        fn capture(&mut self, base: &Q3PredictionBase, brushes: &[ActorId]) {
            self.inner.borrow_mut().capture(base, brushes);
        }
    }

    fn harness() -> (
        Q3RemotePresentation<'static, TestHost>,
        Rc<RefCell<TestHostState>>,
        Rc<RefCell<TestPrediction>>,
    ) {
        let state = Rc::new(RefCell::new(TestHostState::default()));
        let prediction = Rc::new(RefCell::new(TestPrediction::default()));
        let identity = IdentityOwner::create("test").unwrap();
        let client_id = identity.client(0, 1);
        let session = EngineSession::new(IdentityOwner::create("test").unwrap(), SessionMode::Local);
        let presentation = Q3RemotePresentation::new(Q3RemotePresentationOptions {
            identity,
            session,
            client: SessionClient::new(client_id),
            content: Some(TestContent {
                recipe: fixture_recipe(),
            }),
            host: TestHost {
                state: state.clone(),
                prediction: prediction.clone(),
            },
        });
        (presentation, state, prediction)
    }

    fn player_state() -> Q3PlayerState {
        Q3PlayerState {
            product: Q3Product::Base,
            command_time: 100,
            pm_type: 0,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: [1.0, 2.0, 3.0],
            velocity: [4.0, 5.0, 6.0],
            weapon_time: 0,
            gravity: 800,
            speed: 320,
            delta_angles: [0.0, 0.0, 0.0],
            ground_entity_num: GROUND_WORLD,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: [0.0, 0.0, 0.0],
            e_flags: 0,
            event_sequence: 0,
            events: [0, 0],
            event_parms: [0, 0],
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: 2,
            weapon_state: 0,
            viewangles: [10.0, 20.0, 30.0],
            viewheight: 26,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: Q3PlayerSlots::new(),
            persistant: Q3PlayerSlots::new(),
            powerups: Q3PlayerSlots::new(),
            ammo: Q3PlayerSlots::new(),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
        }
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            message_number: 1,
            server_time: 100,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: vec![1, 2],
            player_state: player_state(),
            entities: Vec::new(),
        }
    }

    #[test]
    fn actor_registry_validates_and_classifies_players() {
        let (presentation, _, _) = harness();
        let player = presentation.actor_at(5);
        assert!(presentation.is_player(&player));
        let other = presentation.actor_at(70);
        assert!(!presentation.is_player(&other));
        assert_eq!(presentation.number_of(&player), Some(5));
        assert_eq!(presentation.number_of(&presentation.actor_at(5)), Some(5));
    }

    #[test]
    #[should_panic(expected = "Invalid Q3 source entity")]
    fn actor_at_rejects_large_numbers() {
        let (presentation, _, _) = harness();
        let _ = presentation.actor_at(1024);
    }

    #[test]
    #[should_panic(expected = "Q3 seat has no decoded connection")]
    fn admitted_player_requires_connection() {
        let (presentation, _, _) = harness();
        let _ = presentation.admitted_player();
    }

    #[test]
    #[should_panic(expected = "Q3 cgame needs its first decoded player state")]
    fn initial_player_requires_snapshot() {
        let (presentation, _, _) = harness();
        let _ = presentation.initial_player();
    }

    #[test]
    #[should_panic(expected = "Q3 cgame has no connection source")]
    fn cgame_source_requires_connection() {
        let (mut presentation, _, _) = harness();
        let _ = presentation.cgame_source();
    }

    #[test]
    fn system_info_accepts_base_content_selection() {
        let (presentation, _, _) = harness();
        presentation.system_info("\\protocol\\68\\mapname\\q3dm1\\fs_game\\");
        assert!(presentation.source_records().is_empty());
    }

    #[test]
    #[should_panic(expected = "This server requires pure verification, which is not supported yet.")]
    fn system_info_rejects_pure_servers_without_initializer() {
        let (presentation, _, _) = harness();
        presentation.system_info("\\sv_pure\\1\\");
    }

    #[test]
    fn map_names_validate_donor_pattern() {
        assert!(valid_map_name("q3dm1"));
        assert!(valid_map_name("maps/q3dm1"));
        assert!(valid_map_name("my-map_01/x"));
        assert!(!valid_map_name(""));
        assert!(!valid_map_name("../x"));
        assert!(!valid_map_name("q3dm1;quit"));
    }

    #[test]
    #[should_panic(expected = "Q3 package downloads have no writable content owner")]
    fn download_size_requires_sink() {
        let (mut presentation, _, _) = harness();
        let _ = presentation.download_size(10);
    }

    #[test]
    fn snapshot_publishes_player_body() {
        let (mut presentation, state, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        let output = presentation.output().expect("snapshot publishes");
        assert_eq!(output.snapshot.frame.frame, 1);
        assert_eq!(output.snapshot.bodies.len(), 1);
        assert_eq!(output.snapshot.bodies[0].state.bounds, Q3_BOUNDS);
        let player = presentation.player().expect("player is decoded");
        assert_eq!(player.source_entity, 0);
        let scene = presentation.scene_output().expect("scene blocks travel");
        assert_eq!(scene.area_bits, vec![1, 2]);
        assert_eq!(state.borrow().published, 1);
    }

    #[test]
    fn command_builds_relative_wire_command() {
        let (mut presentation, _, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        let player = presentation.player().expect("player is decoded");
        let wire = presentation.command(&ActorCommand {
            actor: player.actor,
            source: qa_net::common::commands::CommandSource::Remote {
                client: presentation.client().id().clone(),
            },
            sequence: 1,
            command: UserCommand::Q3 {
                server_time_milliseconds: 90.0,
                angle_words: [100.0, 200.0, 300.0],
                buttons: 5.0,
                weapon: 2.0,
                forward_move: 10.0,
                right_move: 0.0,
                up_move: 0.0,
            },
            arsenal: None,
        });
        assert_eq!(wire.server_time, 0);
        assert_eq!(wire.angles, [100, 200, 300]);
        assert_eq!(wire.moves, [10, 0, 0]);
        assert_eq!(wire.buttons, 5);
        assert_eq!(wire.weapon, 2);
    }

    #[test]
    #[should_panic(expected = "Native Q3 remote requires Q3 commands")]
    fn command_rejects_foreign_dialects() {
        let (mut presentation, _, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        let player = presentation.player().expect("player is decoded");
        let _ = presentation.command(&ActorCommand {
            actor: player.actor,
            source: qa_net::common::commands::CommandSource::Remote {
                client: presentation.client().id().clone(),
            },
            sequence: 1,
            command: UserCommand::Q2Classic {
                milliseconds: 0.0,
                angle_shorts: [0.0, 0.0, 0.0],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
                light_level: 0.0,
            },
            arsenal: None,
        });
    }

    #[test]
    fn player_command_quotes_and_sends() {
        let (mut presentation, state, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        let player = presentation.player().expect("player is decoded");
        presentation.player_command(&player.actor, "say", &["hi there".to_string()]);
        assert_eq!(state.borrow().sent, vec!["say \"hi there\"".to_string()]);
    }

    #[test]
    #[should_panic(expected = "Invalid Q3 command delimiter")]
    fn player_command_rejects_delimiters() {
        let (mut presentation, _, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        let player = presentation.player().expect("player is decoded");
        presentation.player_command(&player.actor, "say", &["a;b".to_string()]);
    }

    #[test]
    fn movement_builds_once_and_captures_snapshots() {
        let (mut presentation, state, prediction) = harness();
        presentation.snapshot(&snapshot(), 0);
        let seat = presentation.identity().seat(0);
        let _ = presentation.movement(seat.clone());
        let _ = presentation.movement(seat);
        assert_eq!(state.borrow().built, 1);
        presentation.map_restart();
        let captured = prediction.borrow();
        assert_eq!(captured.captured.len(), 1);
        assert_eq!(captured.captured[0].0.command_time_ms, 100);
        assert_eq!(captured.captured[0].0.ground, Q3SourceHit::World);
        assert_eq!(captured.captured[0].0.jump_pad, None);
        assert_eq!(
            captured.captured[0].0.active_weapon.as_deref(),
            Some("q3:weapon/machinegun")
        );
        assert!(captured.captured[0].1.is_empty());
    }

    #[test]
    fn sample_presentation_advances_live_clock() {
        let (mut presentation, state, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        let output = presentation.sample_presentation(5000).expect("live clock advances");
        assert_eq!(output.snapshot.frame.frame, 1);
        assert_eq!(state.borrow().published, 2);
        assert_eq!(presentation.clock.advanced, vec![5000.0]);
    }

    #[test]
    fn clear_active_resets_presentation() {
        let (mut presentation, _, _) = harness();
        presentation.snapshot(&snapshot(), 0);
        assert!(presentation.output().is_some());
        presentation.clear_active(&|| {});
        assert!(presentation.output().is_none());
        assert!(presentation.player().is_none());
    }

    #[test]
    fn disconnected_prints_and_reports() {
        let (mut presentation, state, _) = harness();
        presentation.disconnected("bye");
        assert_eq!(state.borrow().printed, vec!["bye\n".to_string()]);
        assert_eq!(state.borrow().disconnected, vec!["bye".to_string()]);
    }
}
