//! Application bot transport over Q3 and shared observation worlds.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bots.ts`
//! (`SimulationBotServices`, `ApplicationBots`, `decodeApplicationBotsCheckpoint`,
//! `RestoredBotReliableCommands`, `openApplicationBotLog`, `resumeApplicationBotLog`,
//! `botAdmissionError`).
//!
//! Refactor adaptations against the ported `qa-bots` director, which differs
//! from donor `src/bots/behavior/director.ts`:
//!
//! - The director is constructed with owned host/game/navigation handles and
//!   borrows only bot files; `ApplicationBots` therefore shares its interior
//!   through `Rc<RefCell<..>>` handles instead of closing over `this`.
//! - `SharedBotPopulation` borrows the director, so it is built transiently
//!   per frame rather than stored.
//! - Encoded brain commands cross the population as `ClientCommand` plus a
//!   side-channel stash carrying the full `UserCommand` and arsenal intent,
//!   which `frame` recombines into `ActorCommand`.
//! - qa-bots has no director persistence, round-restart phase, queued-begin
//!   removal, match interbreed, per-client view/weapon state, or log plumbing:
//!   those operations live behind [`BotDirectorBacking`] for the bots lane,
//!   and `open_log`/`resume_log` options are carried for interface parity.
//! - `CommandSource::Bot` carries no provider tag; every command here is
//!   implicitly `q3:bot`.
//!
//! # Missing siblings
//!
//! - `simulation/runtime.ts` (`SharedSimulation`): [`ApplicationBotSimulation`].
//! - `simulation/q3/runtime.ts` (`Q3SourceRuntime`): [`BotQ3Source`].
//! - `world/session/session.ts` (`EngineSession`): [`ApplicationBotSession`].
//! - `bots/behavior/q3/game-host.ts` (`q3BotGame`): [`BotQ3GameFactory`].
//! - qa-bots director persistence/rounds/client state: [`BotDirectorBacking`].
//! - `bots/behavior/library/log.ts` (`BotLogOpenResult`, `BotLogIoResult`):
//!   mirrored here as [`BotLogOpenResult`]/[`BotLogIoResult`].
//! - `compat/q2/rerelease/navigation.ts` (`RereleaseGoalStatus`): mirrored as
//!   [`RereleaseGoalStatus`].

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::rc::Rc;

use qa_bots::behavior::assets::BotSourceFiles;
use qa_bots::behavior::director::{SourceBotDirector, SourceBotDirectorParams};
use qa_bots::behavior::library::genetic::BotRandom;
use qa_bots::behavior::library::goals::BotGoal;
use qa_bots::behavior::library::log::BotLogSink;
use qa_bots::behavior::library::weapons::{WeaponAi, MAX_WEAPON_STATES};
use qa_bots::behavior::population::SharedBotPopulation;
use qa_bots::behavior::q3::ai_state::CommandButtons;
use qa_bots::behavior::q3::ai_state::{BotState, BotUserCommand};
use qa_bots::behavior::q3::game_host::{
    BotArsenalKnowledge, BotObservedPickup, BotProduct, BotTraceQuery, BotTraceResult, SourceBotGame,
};
use qa_bots::behavior::q3::movement_state::BotMoveResult;
use qa_bots::behavior::q3::navigation::SourceBotNavigation;
use qa_bots::behavior::q3::navigation_types::{
    AlternativeGoal, AlternativeRouteQuery, AreaTravelTimeQuery, BotNavigation, BotNavigationArea, PredictRouteQuery,
    PredictedRoute, RouteQuery, RouteResult,
};
use qa_bots::behavior::{director::SourceBotDirectorHost, population::BotFrame};
use qa_bots::runtime::{NavigationRouteQuery, NavigationRuntime};
use qa_bots::save::SaveValue;
use qa_bots::types::NavigationRouteResult;
use qa_bots::BotsError;
use qa_content::contract::ItemId;
use qa_content::q3::base::shared::player_state::UserCommand as Q3SourceUserCommand;
use qa_content::q3::foundation::arsenal::Q3_WEAPON_ITEMS;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, ClientId, OwnedActor, SavedActorId, SessionId};
use qa_core::math::{Bounds, Vec3};
use qa_net::common::commands::{ActorCommand, ArsenalIntent, CommandSource, UserCommand};
use qa_net::q3_net::{Q3PlayerState, ServerReliableCommands, MAX_RELIABLE_COMMANDS};
use qa_world::client::ClientCommand;
use qa_world::movement::types::MovementDialect;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{arr, int, obj, str, SaveJson, SaveReader};
use qa_world::session::SessionClient;
use qa_world::WorldError;
use thiserror::Error;

use super::bot_arsenal::{create_bot_arsenal_binding, BotArsenalBinding, BotArsenalError};
use super::bot_q1_knowledge::Q1BotKnowledgeSimulation;
use super::bot_q2_knowledge::Q2BotKnowledgeSimulation;
use super::bot_q3_knowledge::Q3BotKnowledgeSimulation;
use super::bot_world::{create_shared_bot_world, BotWorldSimulation, SharedBotGame, SharedBotWorldOptions};
use super::prediction::types::MovementPredictionProfile;
use super::q3::host::Q3SourceEvent;
use super::q3::types::Q3SourceBots;
use super::q3_commands::selected_q3_command;
use super::types::{ApplicationBotNavigation, SimulationPresentationEvent, SourcePresentationEvent, ViewResetReason};
use crate::bootstrap::q3_client::visibility::{
    select_application_q3_snapshot, ApplicationQ3SceneQueries, ApplicationQ3SourceEntity,
};

/// Rerelease goal status (donor `RereleaseGoalStatus` numeric union).
pub type RereleaseGoalStatus = i32;

/// Bot transport failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApplicationBotError {
    /// Admission policy rejection.
    #[error("{0}")]
    Admission(String),
    /// Bot clients and simulation belong to different sessions.
    #[error("Bot clients and simulation belong to different sessions")]
    SessionMismatch,
    /// Shared configuration requires the application console registry.
    #[error("Shared bot configuration requires the application console registry")]
    MissingConfiguration,
    /// Bot world projection is unavailable.
    #[error("Bot world projection is unavailable")]
    MissingProjection,
    /// Source bot services already have a director.
    #[error("Source bot services already have a director")]
    AlreadyAttached,
    /// Bot transport is already attached.
    #[error("Bot transport is already attached")]
    TransportAttached,
    /// Bot transport lifetime differs.
    #[error("Bot transport lifetime differs")]
    TransportLifetime,
    /// Cannot detach a different source bot director.
    #[error("Cannot detach a different source bot director")]
    DirectorMismatch,
    /// Source bot transport must be attached before bot commands.
    #[error("Source bot transport must be attached before bot commands")]
    Detached,
    /// Bot transport is closed.
    #[error("Bot transport is closed")]
    Closed,
    /// Cannot checkpoint a closed bot transport.
    #[error("Cannot checkpoint a closed bot transport")]
    CheckpointClosed,
    /// Invalid bot transport restoration.
    #[error("Invalid bot transport restoration")]
    InvalidRestoration,
    /// Invalid saved bot transport binding.
    #[error("Invalid saved bot transport binding")]
    InvalidBinding,
    /// Saved bot transport does not match the restored player client.
    #[error("Saved bot transport does not match the restored player client")]
    ClientMismatch,
    /// Invalid saved bot entity snapshot.
    #[error("Invalid saved bot entity snapshot")]
    InvalidSnapshot,
    /// Saved bot world projection differs from selected source.
    #[error("Saved bot world projection differs from selected source")]
    ProjectionMismatch,
    /// Invalid saved bot observation authority.
    #[error("Invalid saved bot observation authority")]
    InvalidObservations,
    /// Saved bot observation authority differs from restored source.
    #[error("Saved bot observation authority differs from restored source")]
    ObservationMismatch,
    /// Saved bot observation authority is incomplete.
    #[error("Saved bot observation authority is incomplete")]
    IncompleteObservations,
    /// Saved bot arsenal knowledge differs from selected source.
    #[error("Saved bot arsenal knowledge differs from selected source")]
    KnowledgeMismatch,
    /// Saved bot knowledge weapon handle exceeds source capacity.
    #[error("Saved bot knowledge weapon handle exceeds source capacity")]
    WeaponHandle,
    /// Bot fast restart requires an active native Q3 round.
    #[error("Bot fast restart requires an active native Q3 round")]
    RestartRequiresActive,
    /// Publish a new source round before rebinding bots.
    #[error("Publish a new source round before rebinding bots")]
    RestartRequiresRound,
    /// Bind the new bot round before reconnecting clients.
    #[error("Bind the new bot round before reconnecting clients")]
    RestartRequiresBind,
    /// Bot restart client was already reconnected.
    #[error("Bot restart client was already reconnected")]
    RestartDuplicate,
    /// Reconnect every preserved bot before resuming.
    #[error("Reconnect every preserved bot before resuming")]
    RestartPending,
    /// Source bot restoration rejected.
    #[error("Source bot restoration rejected: {0}")]
    RestorationRejected(String),
    /// Bot setup failed.
    #[error("Bot setup failed")]
    SetupFailed,
    /// Cannot admit a closed bot client.
    #[error("Cannot admit a closed bot client")]
    ClosedClient,
    /// Admitted bot has no selected movement player.
    #[error("Admitted bot has no selected movement player")]
    MissingPlayer,
    /// Bot source observation is unavailable.
    #[error("Bot source observation is unavailable")]
    MissingSource,
    /// Source bot lost its player state.
    #[error("Source bot lost its player state")]
    MissingPlayerState,
    /// Bot snapshot sequence must be a signed source integer.
    #[error("Bot snapshot sequence must be a signed source integer")]
    BadSequence,
    /// Bot begin has no actual player.
    #[error("Bot begin has no actual player")]
    BeginPlayer,
    /// Source bot has no session connection.
    #[error("Source bot {0} has no session connection")]
    MissingConnection(i32),
    /// Bot begin has no actual view.
    #[error("Bot begin has no actual view")]
    BeginView,
    /// Bot spawn has no actual arsenal observation.
    #[error("Bot spawn has no actual arsenal observation")]
    SpawnArsenal,
    /// Session failure.
    #[error("Bot session: {0}")]
    Session(String),
    /// Simulation failure.
    #[error("Bot simulation: {0}")]
    Simulation(String),
    /// Director failure.
    #[error("Bot director: {0}")]
    Director(String),
    /// Backing failure.
    #[error("Bot backing: {0}")]
    Backing(String),
    /// Save failure.
    #[error("Bot save: {0}")]
    Save(String),
}

impl From<WorldError> for ApplicationBotError {
    fn from(value: WorldError) -> Self {
        Self::Save(format!("{value:?}"))
    }
}

impl From<BotsError> for ApplicationBotError {
    fn from(value: BotsError) -> Self {
        Self::Director(format!("{value:?}"))
    }
}

impl From<BotArsenalError> for ApplicationBotError {
    fn from(value: BotArsenalError) -> Self {
        Self::Director(format!("{value:?}"))
    }
}

/// Bot log write/flush/close outcome (donor `BotLogIoResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotLogIoResult {
    /// Operation succeeded.
    Ok,
    /// Operation failed with a message.
    Failed {
        /// Failure message.
        error: String,
    },
}

/// Bot log open outcome (donor `BotLogOpenResult`).
pub enum BotLogOpenResult {
    /// Log stream opened.
    Opened {
        /// Open stream.
        stream: BotLogStream,
    },
    /// Open failed with a message.
    Failed {
        /// Failure message.
        error: String,
    },
}

/// Open bot log stream (donor `BotLogOpenResult["stream"]`).
pub struct BotLogStream {
    file: File,
    position: u64,
}

impl BotLogStream {
    /// Capture the write position (donor `checkpoint()`).
    #[must_use]
    pub fn checkpoint(&self) -> BotLogCheckpoint {
        BotLogCheckpoint {
            position: self.position,
        }
    }

    /// Append bytes at the write position (donor `write(bytes)`).
    pub fn write(&mut self, bytes: &[u8]) -> BotLogIoResult {
        let mut offset = 0;
        while offset < bytes.len() {
            if self.file.seek(SeekFrom::Start(self.position)).is_err() {
                return BotLogIoResult::Failed {
                    error: "Bot log seek failed".to_string(),
                };
            }
            match self.file.write(&bytes[offset..]) {
                Ok(0) => {
                    return BotLogIoResult::Failed {
                        error: "Bot log write made no progress".to_string(),
                    };
                }
                Ok(written) => {
                    offset += written;
                    self.position += written as u64;
                }
                Err(error) => {
                    return BotLogIoResult::Failed {
                        error: error.to_string(),
                    };
                }
            }
        }
        BotLogIoResult::Ok
    }

    /// Flush the stream (donor `flush()`).
    pub fn flush(&mut self) -> BotLogIoResult {
        match self.file.sync_all() {
            Ok(()) => BotLogIoResult::Ok,
            Err(error) => BotLogIoResult::Failed {
                error: error.to_string(),
            },
        }
    }

    /// Close the stream (donor `close()`; the file closes on drop).
    pub fn close(self) -> BotLogIoResult {
        match self.file.sync_all() {
            Ok(()) => BotLogIoResult::Ok,
            Err(error) => BotLogIoResult::Failed {
                error: error.to_string(),
            },
        }
    }
}

/// Bot log position checkpoint (donor `stream.checkpoint()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotLogCheckpoint {
    /// Write position.
    pub position: u64,
}

fn bot_log_directory() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|error| error.to_string())?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("state")
        .join("quake-typescript")
        .join("bots"))
}

fn application_bot_log(filename: &str, resume_position: Option<u64>) -> BotLogOpenResult {
    let failed = |error: String| BotLogOpenResult::Failed { error };
    let directory = match bot_log_directory() {
        Ok(directory) => directory,
        Err(error) => return failed(error),
    };
    if resume_position.is_none() && std::fs::create_dir_all(&directory).is_err() {
        return failed("Bot log directory is unavailable".to_string());
    }
    let name = PathBuf::from(filename)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(resume_position.is_none())
        .open(directory.join(name))
    {
        Ok(file) => file,
        Err(error) => return failed(error.to_string()),
    };
    if let Some(position) = resume_position {
        let size = match file.metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) => return failed(error.to_string()),
        };
        if size < position {
            return failed("Saved bot log position exceeds retained file".to_string());
        }
        return BotLogOpenResult::Opened {
            stream: BotLogStream { file, position },
        };
    }
    BotLogOpenResult::Opened {
        stream: BotLogStream { file, position: 0 },
    }
}

/// Open a bot log file for writing (donor `openApplicationBotLog`).
#[must_use]
pub fn open_application_bot_log(filename: &str) -> BotLogOpenResult {
    application_bot_log(filename, None)
}

/// Resume a bot log file at a saved position (donor `resumeApplicationBotLog`).
#[must_use]
pub fn resume_application_bot_log(filename: &str, position: u64) -> BotLogOpenResult {
    application_bot_log(filename, Some(position))
}

/// Saved client identity (donor checkpoint `client`/`actor` records).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SavedBotIdentity {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
}

/// Saved reliable ring image (donor transport `reliable` record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBotReliable {
    /// Current sequence.
    pub sequence: i64,
    /// Acknowledged sequence.
    pub acknowledge: i64,
    /// Ring slots.
    pub slots: Vec<String>,
}

/// Saved transport connection (donor transport `connections` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBotConnection {
    /// Saved client.
    pub client: SavedBotIdentity,
    /// Saved actor.
    pub actor: SavedActorId,
    /// Saved ring.
    pub reliable: SavedBotReliable,
}

/// Saved entity snapshot row (donor transport `snapshots` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBotSnapshot {
    /// Client number.
    pub client: i64,
    /// Entity numbers.
    pub entities: Vec<i64>,
}

/// Transport checkpoint (donor `ApplicationBotTransportCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationBotTransportCheckpoint {
    /// Checkpoint version.
    pub version: i64,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: f64,
    /// Saved connections.
    pub connections: Vec<SavedBotConnection>,
    /// Saved snapshots.
    pub snapshots: Vec<SavedBotSnapshot>,
}

/// Saved observation authority entry (donor `observations` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedBotObservation {
    /// Observation number.
    pub number: i64,
    /// Saved actor.
    pub actor: SavedActorId,
}

/// Decoded bots checkpoint (donor `DecodedApplicationBotsCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedApplicationBotsCheckpoint {
    /// Checkpoint version.
    pub version: i64,
    /// Transport image.
    pub transport: ApplicationBotTransportCheckpoint,
    /// Opaque director image.
    pub director: SaveJson,
    /// Opaque navigation image.
    pub navigation: SaveJson,
    /// Opaque knowledge image.
    pub knowledge: SaveJson,
    /// Opaque shared-world image.
    pub shared_world: SaveJson,
    /// Observation authority.
    pub observations: Vec<SavedBotObservation>,
}

/// Bot client identity (donor `ApplicationBotClient`).
pub struct ApplicationBotClient {
    /// Session client.
    pub client: SessionClient,
    /// Reliable ring.
    pub reliable: ServerReliableCommands,
    /// Client userinfo.
    pub userinfo: Option<String>,
}

/// Restore a saved reliable ring (donor `RestoredBotReliableCommands`).
pub fn restored_bot_reliable_commands(image: &SavedBotReliable) -> Result<ServerReliableCommands, ApplicationBotError> {
    ServerReliableCommands::restore_ring(image.sequence, image.acknowledge, &image.slots)
        .map_err(|error| ApplicationBotError::Save(format!("{error:?}")))
}

fn decode_saved_identity(reader: SaveReader<'_>) -> Result<SavedBotIdentity, WorldError> {
    Ok(SavedBotIdentity {
        slot: u32::try_from(reader.field("slot").integer(0)?)
            .map_err(|_| reader.field("slot").fail("expected an integer in range"))?,
        generation: u32::try_from(reader.field("generation").integer(0)?)
            .map_err(|_| reader.field("generation").fail("expected an integer in range"))?,
    })
}

fn decode_bot_transport(value: &SaveJson) -> Result<ApplicationBotTransportCheckpoint, WorldError> {
    let reader = SaveReader::at(value, "application.bots.transport");
    Ok(ApplicationBotTransportCheckpoint {
        version: reader.field("version").literal_i64(1)?,
        elapsed_milliseconds: reader.field("elapsedMilliseconds").finite()?,
        connections: reader
            .field("connections")
            .list(|entry| -> Result<SavedBotConnection, WorldError> {
                let reliable = entry.field("reliable");
                Ok(SavedBotConnection {
                    client: decode_saved_identity(entry.field("client"))?,
                    actor: read_saved_actor(entry.field("actor"))?,
                    reliable: SavedBotReliable {
                        sequence: reliable.field("sequence").integer(0)?,
                        acknowledge: reliable.field("acknowledge").integer(i64::MIN)?,
                        slots: reliable.field("slots").list(|slot| slot.string())?,
                    },
                })
            })?,
        snapshots: reader
            .field("snapshots")
            .list(|entry| -> Result<SavedBotSnapshot, WorldError> {
                Ok(SavedBotSnapshot {
                    client: entry.field("client").integer(0)?,
                    entities: entry.field("entities").list(|entity| entity.integer(0))?,
                })
            })?,
    })
}

/// Decode a bots checkpoint (donor `decodeApplicationBotsCheckpoint`).
pub fn decode_application_bots_checkpoint(
    value: &SaveJson,
) -> Result<DecodedApplicationBotsCheckpoint, ApplicationBotError> {
    let reader = SaveReader::at(value, "application.bots");
    Ok(DecodedApplicationBotsCheckpoint {
        version: reader.field("version").literal_i64(1)?,
        transport: decode_bot_transport(
            reader
                .field("transport")
                .value
                .ok_or_else(|| reader.field("transport").fail("missing bot transport"))?,
        )?,
        director: reader.field("director").value.cloned().unwrap_or(SaveJson::Null),
        navigation: reader.field("navigation").value.cloned().unwrap_or(SaveJson::Null),
        knowledge: reader.field("knowledge").value.cloned().unwrap_or(SaveJson::Null),
        shared_world: reader.field("sharedWorld").value.cloned().unwrap_or(SaveJson::Null),
        observations: reader
            .field("observations")
            .list(|entry| -> Result<SavedBotObservation, WorldError> {
                Ok(SavedBotObservation {
                    number: entry.field("number").integer(0)?,
                    actor: read_saved_actor(entry.field("actor"))?,
                })
            })?,
    })
}

/// Mover view (donor `movementPlayer(actor)` used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct BotMoverView {
    /// Owning client, when bound.
    pub client: Option<ClientId>,
    /// Movement profile.
    pub profile: MovementPredictionProfile,
    /// Arsenal provider.
    pub arsenal_provider: String,
    /// View angles (donor `player.viewAngles`).
    pub view_angles: Vec3,
}

/// Admission policy view (donor `botAdmissionError`/`legacyBotAdmissionError` inputs).
#[derive(Debug, Clone, PartialEq)]
pub struct BotAdmissionView {
    /// Whether a Q3 source is bound.
    pub has_q3: bool,
    /// First recipe weapon provider, if any.
    pub weapons_provider: Option<String>,
    /// Q3 product tag, if any.
    pub q3_product: Option<String>,
    /// Whether a Q2 weapon source is bound.
    pub has_q2_weapons: bool,
    /// Whether a Q1 weapon source is bound.
    pub has_q1_weapons: bool,
    /// Whether a selected Q3 weapon source is bound.
    pub has_selected_q3_weapons: bool,
    /// Whether the mode is deathmatch.
    pub deathmatch: bool,
    /// Q3 game type, if any.
    pub q3_game_type: Option<i32>,
    /// Whether a Q1 source is bound.
    pub has_q1: bool,
    /// Whether a Q2 source is bound.
    pub has_q2: bool,
    /// Q1 composition program, if any.
    pub q1_program: Option<String>,
    /// Q1 `teamplay` value.
    pub q1_teamplay: f32,
    /// Whether the Q2 match selection is standard.
    pub q2_match_standard: bool,
}

/// Q3 pool client view (donor `source.pool.at(slot).client` used surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotQ3PoolClient {
    /// Delta angle words.
    pub delta_angles: [i32; 3],
    /// Current weapon.
    pub weapon: i32,
}

/// Q3 ownership record (donor `records.captureOwnership()` entry surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotQ3Ownership {
    /// Owning actor, if any.
    pub actor: Option<OwnedActor>,
}

/// Q3 source client snapshot row (donor `sourceState().clients` entry surface).
#[derive(Debug, Clone, PartialEq)]
pub struct BotQ3SourceClient {
    /// Client slot.
    pub slot: i32,
    /// Wire player state.
    pub state: Q3PlayerState,
}

/// Native Q3 source seam (donor `Q3SourceRuntime` used surface).
pub trait BotQ3Source: std::fmt::Debug {
    /// Donor `source.options.product`.
    fn product(&self) -> String;
    /// Donor `source.options.entities`.
    fn entities(&self) -> String;
    /// Donor `source.gameType`.
    fn game_type(&self) -> i32;
    /// Donor `source.host.cvars`.
    fn cvars(&self) -> Rc<RefCell<CvarRegistry>>;
    /// Donor `source.level.time`.
    fn level_time_ms(&self) -> i32;
    /// Donor `source.pool.at(slot).client`.
    fn pool_client(&self, slot: i32) -> Option<BotQ3PoolClient>;
    /// Donor `entity.r.svFlags |= BOT`.
    fn set_bot_flag(&self, slot: i32);
    /// Donor `source.pool.activateClient(slot)`.
    fn activate_client(&self, slot: i32);
    /// Donor `source.admission.connect(slot, firstTime, bot)`; rejection reason or `None`.
    fn admission_connect(&self, slot: i32, first_time: bool, bot: bool) -> Option<String>;
    /// Donor `source.admission.begin(slot)`.
    fn admission_begin(&self, slot: i32);
    /// Donor `source.records.captureOwnership()`.
    fn capture_ownership(&self) -> Vec<BotQ3Ownership>;
    /// Donor `source.sourceState().clients`.
    fn source_clients(&self) -> Vec<BotQ3SourceClient>;
    /// Donor snapshot selector source rows.
    fn source_entities(&self) -> Vec<ApplicationQ3SourceEntity>;
    /// Donor `source.world.linkState(number)?.absbounds`.
    fn link_bounds(&self, number: i32) -> Option<Bounds>;
    /// Q3 game engine userinfo store read.
    fn engine_userinfo(&self, slot: i32) -> String;
    /// Q3 game engine userinfo store write.
    fn set_engine_userinfo(&self, slot: i32, userinfo: &str);
    /// Donor `source.random`.
    fn random(&self) -> f64;
    /// Donor `source.pool` entity count projection (`game.entityCount`).
    fn entity_count(&self) -> i32;
    /// Donor Q3 game engine `dropClient(slot, reason)`.
    fn drop_client(&self, slot: i32, reason: &str);
    /// Donor Q3 player name for admission checks.
    fn player_name(&self, actor: &ActorId) -> Option<String>;
}

/// Engine session seam (donor `EngineSession` used surface).
pub trait ApplicationBotSession: std::fmt::Debug + 'static {
    /// Donor `session.createClient(slot)`.
    fn create_client(&mut self, slot: u32) -> Result<SessionClient, ApplicationBotError>;
    /// Donor `session.closeClient(client)`.
    fn close_client(&mut self, client: &ClientId);
    /// Donor `session.session`.
    fn session_id(&self) -> SessionId;
    /// Whether the session issued a client (donor `client.id.session` check).
    fn owns_client(&self, client: &ClientId) -> bool;
}

/// Shared simulation seam (donor `SharedSimulation` used surface).
pub trait ApplicationBotSimulation<'a>:
    Q1BotKnowledgeSimulation + Q2BotKnowledgeSimulation + Q3BotKnowledgeSimulation + BotWorldSimulation + Clone + 'static
{
    /// Donor `simulation.q3Source()`.
    fn q3_bot_source(&self) -> Option<Rc<dyn BotQ3Source>>;
    /// Donor admission policy inputs.
    fn bot_admission(&self) -> BotAdmissionView;
    /// Donor `simulation.session`.
    fn session_id(&self) -> SessionId;
    /// Donor `simulation.movementPlayer(actor)` (inherits `players`,
    /// `is_live`, and `time_seconds` from [`BotWorldSimulation`]).
    fn mover(&self, actor: &ActorId) -> Option<BotMoverView>;
    /// Donor `simulation.actors.referenceSaved(saved)`.
    fn reference_saved(&self, saved: SavedActorId) -> ActorId;
    /// Donor `simulation.actors.resolveSaved(saved)`.
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<OwnedActor>;
    /// Donor `simulation.prepareBotClient(client)`.
    fn prepare_bot_client(&mut self, client: &ClientId) -> Result<OwnedActor, ApplicationBotError>;
    /// Donor `simulation.disconnectPlayer(actor)`.
    fn disconnect_player(&mut self, actor: &ActorId);
    /// Donor `simulation.botServices`.
    fn bot_services(&mut self) -> &mut SimulationBotServices<'a>;
    /// Donor `simulation.sourceEntityText`.
    fn source_entity_text(&self) -> String;
    /// Q3 snapshot scene queries over `simulation.scene`.
    fn q3_scene_queries(&self) -> Rc<dyn ApplicationQ3SceneQueries>;
    /// Donor `source.game.host.random()` for the shared observation world.
    fn bot_random(&self) -> f64;
}

/// Q3 game factory (donor `q3BotGame(source, console, arsenal)`).
pub type BotQ3GameFactory = Rc<dyn Fn(&dyn BotQ3Source, Rc<dyn Fn(&str)>) -> Box<dyn SourceBotGame>>;

/// Director restore context (donor `restoreSaveState` callback surface).
pub trait BotDirectorRestoreContext {
    /// Donor `actor(saved)` (`referenceSaved`).
    fn reference_saved(&self, saved: SavedActorId) -> ActorId;
    /// Donor `resolveSaved(saved)`.
    fn resolve_saved(&self, saved: &SavedActorId) -> ActorId;
    /// Donor navigation edge lookup.
    fn find_edge(&self, client: i32, edge: i32) -> Option<i32>;
    /// Donor observation remap.
    fn remap_observation(&self, number: i32, generation: i32) -> (i32, i32);
}

/// Bots-lane director backing seam: qa-bots director persistence, round
/// phase, queued-begin removal, match interbreed, and per-client view/weapon
/// state have no ported counterpart yet.
pub trait BotDirectorBacking: std::fmt::Debug + 'static {
    /// Capture the opaque director image (donor `director.captureSaveState()`).
    fn capture_director_state(&self) -> SaveJson;
    /// Restore the opaque director image (donor `director.restoreSaveState()`).
    fn restore_director_state(
        &mut self,
        image: &SaveJson,
        context: &dyn BotDirectorRestoreContext,
    ) -> Result<(), ApplicationBotError>;
    /// Begin a round restart (donor `director.beginRoundRestart()`).
    fn director_begin_round(&mut self);
    /// Bind a restarted round (donor `director.bindRestartedRound()`).
    fn director_bind_round(&mut self);
    /// Resume round bots (donor `director.resumeRoundBots()`).
    fn director_resume_round(&mut self);
    /// Remove a queued begin (donor `director.removeQueuedBegin()`).
    fn director_remove_queued_begin(&mut self, client: i32);
    /// Interbreed at match end (donor `director.interbreedEndMatch()`).
    fn director_interbreed_end_match(&mut self);
    /// Reset a bot view to spawn angles (donor `resetView` roster write).
    fn director_set_bot_view(&mut self, client: i32, angles: Vec3);
    /// Reset a bot weapon to its spawn observation (donor `resetWeapon` roster write).
    fn director_set_spawn_weapon(&mut self, client: i32, weapon: i32);
}

/// Admission error for bot worlds (donor `botAdmissionError`).
#[must_use]
pub fn bot_admission_error(view: &BotAdmissionView) -> Option<String> {
    if view.has_q3 {
        let native = view
            .weapons_provider
            .as_ref()
            .is_some_and(|provider| provider.starts_with("q3:"));
        let selected = view.q3_product.as_deref() == Some("baseq3")
            && (view.has_q2_weapons || view.has_q1_weapons)
            && view.deathmatch
            && view.q3_game_type == Some(0);
        return if native || selected {
            None
        } else {
            Some("Q3-map bots support native Q3 weapons or selected Q1/Q2 weapons in base Q3 deathmatch".to_string())
        };
    }
    if !view.has_q2_weapons && !view.has_q1_weapons && !view.has_selected_q3_weapons {
        return Some("Shared bot arsenal observation is unavailable".to_string());
    }
    if view.has_q1 || view.has_q2 {
        None
    } else {
        Some("Bot world observation is unavailable".to_string())
    }
}

/// Legacy admission error for shared worlds (donor `legacyBotAdmissionError`).
#[must_use]
pub fn legacy_bot_admission_error(view: &BotAdmissionView) -> Option<String> {
    if !view.deathmatch
        || view.has_q1 && (view.q1_program.as_deref() != Some("id1") || view.q1_teamplay != 0.0)
        || view.has_q2 && !view.q2_match_standard
    {
        return Some("Team and campaign bots require mounted native bot definitions".to_string());
    }
    None
}

/// Bot connection (donor `BotConnection`).
pub struct BotConnection {
    /// Session client.
    pub client: SessionClient,
    /// Admitted actor.
    pub actor: OwnedActor,
    /// Reliable ring.
    pub reliable: ServerReliableCommands,
}

/// Shared bot connection handle (donor `ApplicationBotClient` with live shared state).
#[derive(Clone)]
pub struct BotClientSnapshot {
    /// Live connection.
    pub connection: Rc<RefCell<BotConnection>>,
    /// Client userinfo.
    pub userinfo: Option<String>,
}

/// Bots restore input (donor `ApplicationBotsOptions["restore"]`).
pub struct ApplicationBotsRestore {
    /// Decoded checkpoint image.
    pub image: DecodedApplicationBotsCheckpoint,
    /// Resolve a saved client.
    pub resolve_client: Rc<dyn Fn(SavedBotIdentity) -> Option<SessionClient>>,
}

/// Bots options (donor `ApplicationBotsOptions`).
///
/// `backing` and `q3_game_factory` cover the qa-bots gaps documented above;
/// `open_log`/`resume_log` are carried for interface parity (the refactored
/// director opens no logs).
pub struct ApplicationBotsOptions<'a, S, E, B> {
    /// Engine session.
    pub session: Rc<RefCell<E>>,
    /// Shared simulation.
    pub simulation: Rc<RefCell<S>>,
    /// Bot files.
    pub files: &'a dyn BotSourceFiles,
    /// Selected navigation.
    pub navigation: ApplicationBotNavigation<'a>,
    /// Snapshot leaf budget.
    pub leaf_count: i32,
    /// Shared configuration registry.
    pub configuration: Option<Rc<RefCell<CvarRegistry>>>,
    /// Restart load.
    pub restart: bool,
    /// Automatic framing.
    pub automatic_frame: bool,
    /// Preserved clients.
    pub clients: Vec<BotClientSnapshot>,
    /// Restore input.
    pub restore: Option<ApplicationBotsRestore>,
    /// Director backing seam.
    pub backing: B,
    /// Q3 game factory seam.
    pub q3_game_factory: Option<BotQ3GameFactory>,
    /// Console command sink.
    pub insert_console_command: Rc<dyn Fn(&str)>,
    /// Print sink.
    pub print: Rc<dyn Fn(&str)>,
    /// Log opener.
    pub open_log: Rc<dyn Fn(&str) -> BotLogOpenResult>,
    /// Log resumer.
    pub resume_log: Option<BotResumeLogFn>,
}

/// Log-resume sink (donor `ApplicationBotsOptions["resumeLog"]`).
pub type BotResumeLogFn = Rc<dyn Fn(&str, u64) -> BotLogOpenResult>;

/// Shared bot transport handle (donor `ApplicationBots` constructor result).
pub type ApplicationBotsHandle<'a, S, E, B> = Rc<RefCell<ApplicationBots<'a, S, E, B>>>;

/// Bot transport service surface (donor `ApplicationBotService`).
///
/// `options` is replaced by granular accessors (`automatic_frame`,
/// `route_for_client`) because interior-shared simulation, session, and
/// navigation cannot be borrowed out of the transport.
pub trait ApplicationBotService: std::fmt::Debug {
    /// Shared configuration registry.
    fn configuration(&self) -> Rc<RefCell<CvarRegistry>>;
    /// Whether services frame this transport.
    fn automatic_frame(&self) -> bool;
    /// Route for a client (donor `navigation.forClient(slot).route(...)`).
    fn route_for_client(&self, client: i32, start: Vec3, goal: Vec3) -> Result<NavigationRouteResult, BotsError>;
    /// Mover client slot, if any.
    fn mover_client_slot(&self, actor: &ActorId) -> Option<i32>;
    /// Drive a bot toward a point.
    fn move_to_point(&mut self, actor: &ActorId, point: Vec3, tolerance: f64) -> RereleaseGoalStatus;
    /// Drive a bot to follow an actor.
    fn follow_actor(&mut self, actor: &ActorId, target: &ActorId) -> RereleaseGoalStatus;
    /// Whether an actor is a bot.
    fn is_bot(&self, actor: &ActorId) -> bool;
    /// Bot actor for a client.
    fn service_actor(&self, client: &ClientId) -> Option<ActorId>;
    /// Run a bot frame.
    fn frame(
        &mut self,
        time_milliseconds: f64,
        elapsed_milliseconds: f64,
    ) -> Result<Vec<ActorCommand>, ApplicationBotError>;
    /// Receive presentation events.
    fn receive(&mut self, events: &[SimulationPresentationEvent]);
    /// Live client snapshots.
    fn clients(&self) -> Vec<BotClientSnapshot>;
    /// Run a console command.
    fn console_command(&mut self, argv: &[String]) -> Result<(), ApplicationBotError>;
    /// Disconnect a client slot.
    fn disconnect(&mut self, client: i32) -> bool;
    /// Close the transport.
    fn close(&mut self, restart: bool);
    /// Capture the checkpoint image.
    fn checkpoint(&self) -> Result<SaveJson, ApplicationBotError>;
    /// Detach for a round restart, preserving clients.
    fn begin_round_restart(&mut self) -> Result<Vec<BotClientSnapshot>, ApplicationBotError>;
    /// Bind a restarted round.
    fn bind_restarted_round(&mut self) -> Result<(), ApplicationBotError>;
    /// Reconnect one preserved client.
    fn reconnect_restarted_client(&mut self, client: &ClientId) -> Result<bool, ApplicationBotError>;
    /// Resume round bots.
    fn resume_round_bots(&mut self) -> Result<(), ApplicationBotError>;
    /// Remove a queued begin through the director backing.
    fn director_remove_queued_begin(&mut self, client: i32);
    /// Interbreed at match end through the director backing.
    fn director_interbreed_end_match(&mut self);
}

/// Navigation outcome (donor `SimulationBotServices.navigation` surface).
#[derive(Debug, Clone, PartialEq)]
pub enum BotNavigationOutcome {
    /// Route found.
    Path {
        /// Squared path distance.
        distance_squared: f64,
        /// Path points.
        points: Vec<Vec3>,
    },
    /// No navigation available.
    NoNavigation,
    /// Goal unreachable.
    Unreachable,
}

/// Stable source imports installed before the game and bound before bot admission
/// (donor `SimulationBotServices`).
pub struct SimulationBotServices<'a> {
    director: Option<Rc<RefCell<SourceBotDirector<'a>>>>,
    transport: Option<Rc<RefCell<dyn ApplicationBotService + 'a>>>,
}

impl<'a> SimulationBotServices<'a> {
    /// Empty services.
    #[must_use]
    pub fn new() -> Self {
        Self {
            director: None,
            transport: None,
        }
    }

    /// Drive a bot toward a point.
    pub fn move_to_point(&mut self, actor: &ActorId, point: Vec3, tolerance: f64) -> RereleaseGoalStatus {
        self.transport.as_ref().map_or(0, |transport| {
            transport.borrow_mut().move_to_point(actor, point, tolerance)
        })
    }

    /// Drive a bot to follow an actor.
    pub fn follow_actor(&mut self, actor: &ActorId, target: &ActorId) -> RereleaseGoalStatus {
        self.transport
            .as_ref()
            .map_or(0, |transport| transport.borrow_mut().follow_actor(actor, target))
    }

    /// Whether an actor is a bot.
    #[must_use]
    pub fn is_bot(&self, actor: &ActorId) -> bool {
        self.transport
            .as_ref()
            .is_some_and(|transport| transport.borrow().is_bot(actor))
    }

    /// Shared configuration registry, if any.
    #[must_use]
    pub fn configuration(&self) -> Option<Rc<RefCell<CvarRegistry>>> {
        self.transport
            .as_ref()
            .map(|transport| transport.borrow().configuration())
    }

    /// Capture the checkpoint image, if any.
    pub fn checkpoint(&self) -> Result<Option<SaveJson>, ApplicationBotError> {
        self.transport
            .as_ref()
            .map(|transport| transport.borrow().checkpoint())
            .transpose()
    }

    /// Run a services frame.
    pub fn frame(
        &mut self,
        time_milliseconds: f64,
        elapsed_milliseconds: f64,
    ) -> Result<Vec<ActorCommand>, ApplicationBotError> {
        let automatic = self
            .transport
            .as_ref()
            .is_some_and(|transport| transport.borrow().automatic_frame());
        if !automatic {
            return Ok(Vec::new());
        }
        match self.transport.as_ref() {
            Some(transport) => transport.borrow_mut().frame(time_milliseconds, elapsed_milliseconds),
            None => Ok(Vec::new()),
        }
    }

    /// Route between two points.
    pub fn navigation(&self, actor: &ActorId, start: Vec3, goal: Vec3) -> BotNavigationOutcome {
        let Some(transport) = self.transport.as_ref() else {
            return BotNavigationOutcome::NoNavigation;
        };
        let slot = transport.borrow().mover_client_slot(actor);
        let Some(slot) = slot else {
            return BotNavigationOutcome::NoNavigation;
        };
        match transport.borrow().route_for_client(slot, start, goal) {
            Ok(NavigationRouteResult::Route { route }) => {
                let mut distance = 0.0;
                let mut previous: Option<Vec3> = None;
                for current in &route.points {
                    if let Some(previous) = previous {
                        distance += ((current.x - previous.x) as f64)
                            .hypot((current.y - previous.y) as f64)
                            .hypot((current.z - previous.z) as f64);
                    }
                    previous = Some(*current);
                }
                BotNavigationOutcome::Path {
                    distance_squared: distance * distance,
                    points: route.points,
                }
            }
            Ok(NavigationRouteResult::Unreachable { .. }) | Err(_) => BotNavigationOutcome::Unreachable,
        }
    }

    /// Q3 source bot services.
    #[must_use]
    pub fn source(&self) -> Q3SourceBots<'a> {
        let (Some(director), Some(transport)) = (self.director.as_ref(), self.transport.as_ref()) else {
            return Q3SourceBots::Unavailable {
                reason: "Source bot transport must be attached before bot commands".to_string(),
            };
        };
        let remove_transport = Rc::clone(transport);
        let connect_director = Rc::clone(director);
        let shutdown_director = Rc::clone(director);
        let console_director = Rc::clone(director);
        let aas_director = Rc::clone(director);
        let interbreed_transport = Rc::clone(transport);
        Q3SourceBots::Available {
            remove_queued_begin: Rc::new(move |client| {
                remove_transport
                    .borrow_mut()
                    .director_remove_queued_begin(client as i32);
            }),
            connect: Rc::new(move |client, restart| connect_director.borrow_mut().connect(client as i32, !restart)),
            shutdown_client: Rc::new(move |client, restart| {
                shutdown_director.borrow_mut().shutdown_client(client as i32, restart);
            }),
            test_aas: Rc::new(move |origin| {
                let _ = aas_director.borrow().ai().test_aas(origin);
            }),
            interbreed_end_match: Rc::new(move || {
                interbreed_transport.borrow_mut().director_interbreed_end_match();
            }),
            console_command: Rc::new(move |argv| {
                let _ = console_director.borrow_mut().console_command(argv);
            }),
        }
    }

    /// Attach a director and transport.
    pub fn attach(
        &mut self,
        director: Rc<RefCell<SourceBotDirector<'a>>>,
        transport: Rc<RefCell<dyn ApplicationBotService + 'a>>,
    ) -> Result<(), ApplicationBotError> {
        if self.director.is_some() || self.transport.is_some() {
            return Err(ApplicationBotError::AlreadyAttached);
        }
        self.director = Some(director);
        self.transport = Some(transport);
        Ok(())
    }

    /// Attach a transport without a director.
    pub fn attach_transport(
        &mut self,
        transport: Rc<RefCell<dyn ApplicationBotService + 'a>>,
    ) -> Result<(), ApplicationBotError> {
        if self.director.is_some() || self.transport.is_some() {
            return Err(ApplicationBotError::TransportAttached);
        }
        self.transport = Some(transport);
        Ok(())
    }

    /// Detach a transport.
    pub fn detach_transport(
        &mut self,
        transport: &Rc<RefCell<dyn ApplicationBotService + 'a>>,
    ) -> Result<(), ApplicationBotError> {
        let same = self
            .transport
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, transport));
        if self.director.is_some() || !same {
            return Err(ApplicationBotError::TransportLifetime);
        }
        self.transport = None;
        Ok(())
    }

    /// Detach a director.
    pub fn detach(&mut self, director: &Rc<RefCell<SourceBotDirector<'a>>>) -> Result<(), ApplicationBotError> {
        let same = self
            .director
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, director));
        if !same {
            return Err(ApplicationBotError::DirectorMismatch);
        }
        self.director = None;
        self.transport = None;
        Ok(())
    }
}

impl Default for SimulationBotServices<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Encoded brain command stash (recombined by `frame`).
#[derive(Debug, Clone, PartialEq)]
struct EncodedBotCommand {
    command: UserCommand,
    provider: String,
    weapon: Option<ItemId>,
    use_holdable: bool,
}

/// Restart phase (donor `restartPhase`).
#[derive(Clone)]
enum BotRestartPhase {
    /// Active transport.
    Active,
    /// Detached with a preserved source round and clients.
    Detached {
        /// Preserved source.
        source: Rc<dyn BotQ3Source>,
        /// Preserved clients.
        clients: Vec<BotClientSnapshot>,
    },
    /// Bound with preserved clients and pending slots.
    Bound {
        /// Preserved clients.
        clients: Vec<BotClientSnapshot>,
        /// Pending slots.
        pending: HashSet<i32>,
    },
}

/// Shared transport interior.
struct BotsInner<'a, S, E, B> {
    session: Rc<RefCell<E>>,
    simulation: Rc<RefCell<S>>,
    files: &'a dyn BotSourceFiles,
    transport: Option<std::rc::Weak<RefCell<ApplicationBots<'a, S, E, B>>>>,
    leaf_count: i32,
    configuration: Option<Rc<RefCell<CvarRegistry>>>,
    automatic_frame: bool,
    for_client: Rc<dyn Fn(i32) -> NavigationRuntime<'a>>,
    insert_console_command: Rc<dyn Fn(&str)>,
    print: Rc<dyn Fn(&str)>,
    q3_game_factory: Option<BotQ3GameFactory>,
    backing: Rc<RefCell<B>>,
    nav: Rc<RefCell<SourceBotNavigation<'a>>>,
    source: Option<Rc<dyn BotQ3Source>>,
    arsenal: Option<Rc<RefCell<Box<dyn BotArsenalBinding>>>>,
    shared: Option<Rc<RefCell<SharedBotGame<S>>>>,
    q3game: Option<Rc<RefCell<Box<dyn SourceBotGame>>>>,
    director: Option<Rc<RefCell<SourceBotDirector<'a>>>>,
    connections: HashMap<i32, Rc<RefCell<BotConnection>>>,
    snapshots: HashMap<i32, Vec<i32>>,
    elapsed_ms: f64,
    closed: bool,
    phase: BotRestartPhase,
    stash: HashMap<i32, EncodedBotCommand>,
}

/// Bot transport (donor `ApplicationBots`).
pub struct ApplicationBots<'a, S, E, B> {
    inner: Rc<RefCell<BotsInner<'a, S, E, B>>>,
}

impl<S, E, B> std::fmt::Debug for ApplicationBots<'_, S, E, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApplicationBots").finish_non_exhaustive()
    }
}

/// Convert a navigation save value to checkpoint JSON.
fn save_value_to_json(value: &SaveValue) -> SaveJson {
    match value {
        SaveValue::Null => SaveJson::Null,
        SaveValue::Bool(value) => SaveJson::Bool(*value),
        SaveValue::Int(value) => SaveJson::Number(*value as f64),
        SaveValue::Float(value) => SaveJson::Number(*value),
        SaveValue::Str(value) => SaveJson::String(value.clone()),
        SaveValue::Bytes(value) => SaveJson::Bytes(value.clone()),
        SaveValue::List(items) => SaveJson::Array(items.iter().map(save_value_to_json).collect()),
        SaveValue::Map(entries) => SaveJson::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), save_value_to_json(value)))
                .collect(),
        ),
    }
}

/// Convert checkpoint JSON to a navigation save value.
///
/// Integral numbers map back to `Int`: the navigation restore reader is
/// strict about integer cells.
fn save_json_to_value(value: &SaveJson) -> SaveValue {
    match value {
        SaveJson::Null => SaveValue::Null,
        SaveJson::Bool(value) => SaveValue::Bool(*value),
        SaveJson::Number(value) => {
            if value.fract() == 0.0 && *value >= i64::MIN as f64 && *value <= i64::MAX as f64 {
                SaveValue::Int(*value as i64)
            } else {
                SaveValue::Float(*value)
            }
        }
        SaveJson::BigInt(value) => SaveValue::Int(*value as i64),
        SaveJson::Bytes(value) => SaveValue::Bytes(value.clone()),
        SaveJson::String(value) => SaveValue::Str(value.clone()),
        SaveJson::Array(items) => SaveValue::List(items.iter().map(save_json_to_value).collect()),
        SaveJson::Object(entries) => SaveValue::Map(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), save_json_to_value(item)))
                .collect(),
        ),
    }
}

/// Map a selected movement command to the wire command (donor `selectedQ3Command`
/// output reinterpreted as the network `UserCommand`).
fn movement_to_wire_command(command: &qa_world::movement::types::UserCommand) -> UserCommand {
    use qa_world::movement::types::UserCommand as MovementCommand;
    match command {
        MovementCommand::Q1Netquake(value) => UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: value.acknowledged_server_time_seconds,
            view_angles: [
                f64::from(value.view_angles.x),
                f64::from(value.view_angles.y),
                f64::from(value.view_angles.z),
            ],
            forward_move: value.forward_move,
            side_move: value.side_move,
            up_move: value.up_move,
            buttons: f64::from(value.buttons),
            impulse: f64::from(value.impulse),
        },
        MovementCommand::Q1Quakeworld(value) => UserCommand::Q1Quakeworld {
            milliseconds: f64::from(value.milliseconds),
            angles: [
                f64::from(value.angles.x),
                f64::from(value.angles.y),
                f64::from(value.angles.z),
            ],
            forward_move: value.forward_move,
            side_move: value.side_move,
            up_move: value.up_move,
            buttons: f64::from(value.buttons),
            impulse: f64::from(value.impulse),
        },
        MovementCommand::Q2Classic(value) => UserCommand::Q2Classic {
            milliseconds: f64::from(value.milliseconds),
            angle_shorts: [
                f64::from(value.angle_shorts[0]),
                f64::from(value.angle_shorts[1]),
                f64::from(value.angle_shorts[2]),
            ],
            forward_move: value.forward_move,
            side_move: value.side_move,
            up_move: value.up_move,
            buttons: f64::from(value.buttons),
            impulse: f64::from(value.impulse),
            light_level: f64::from(value.light_level),
        },
        MovementCommand::Q2Rerelease(value) => UserCommand::Q2Rerelease {
            milliseconds: f64::from(value.milliseconds),
            angles: [
                f64::from(value.angles.x),
                f64::from(value.angles.y),
                f64::from(value.angles.z),
            ],
            forward_move: value.forward_move,
            side_move: value.side_move,
            buttons: f64::from(value.buttons),
            server_frame: f64::from(value.server_frame),
        },
        MovementCommand::Q3(value) => UserCommand::Q3 {
            server_time_milliseconds: f64::from(value.server_time_milliseconds),
            angle_words: [
                f64::from(value.angle_words[0]),
                f64::from(value.angle_words[1]),
                f64::from(value.angle_words[2]),
            ],
            buttons: f64::from(value.buttons),
            weapon: f64::from(value.weapon),
            forward_move: f64::from(value.forward_move),
            right_move: f64::from(value.right_move),
            up_move: f64::from(value.up_move),
        },
    }
}

/// Print log sink for director shutdown.
struct BotPrintSink {
    print: Rc<dyn Fn(&str)>,
}

impl BotLogSink for BotPrintSink {
    fn print(&mut self, _severity: qa_bots::behavior::library::log::BotLogSeverity, text: &str) {
        (self.print)(text);
    }

    fn write(&mut self, bytes: &[u8]) {
        (self.print)(&String::from_utf8_lossy(bytes));
    }

    fn flush(&mut self) {}
}

/// Director host handle (donor `directorHost(game)`).
struct BotHostHandle<'a, S, E, B> {
    bots: ApplicationBots<'a, S, E, B>,
}

impl<'a, S, E, B> SourceBotDirectorHost for BotHostHandle<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn allocate_client(&mut self) -> Option<i32> {
        self.bots.allocate_client()
    }

    fn actor(&self, client: i32) -> Option<(ActorId, u32)> {
        self.bots.connection_actor(client)
    }

    fn encode_command(&self, client: i32, command: &BotUserCommand) -> ClientCommand {
        self.bots.encode_command(client, command)
    }

    fn snapshot_entity(&self, client: i32, sequence: i32) -> i32 {
        self.bots.snapshot_entity(client, sequence)
    }

    fn console_message(&self, client: i32) -> Option<String> {
        self.bots.console_message(client)
    }

    fn point_contents(&self, point: Vec3) -> i32 {
        self.bots.point_contents(point)
    }

    fn print(&mut self, text: &str) {
        self.bots.print(text);
    }

    fn time_ms(&self) -> i32 {
        self.bots.time_ms()
    }

    fn name_in_use(&self, name: &str) -> bool {
        self.bots.name_in_use(name)
    }
}

/// Director knowledge handle over the live arsenal binding.
struct BotKnowledgeHandle<'a, S, E, B> {
    bots: ApplicationBots<'a, S, E, B>,
}

impl<'a, S, E, B> BotArsenalKnowledge for BotKnowledgeHandle<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn pickup_utility(
        &self,
        characters: &qa_bots::behavior::library::character::BotCharacterLibrary,
        state: &BotState,
        pickup: &BotObservedPickup,
    ) -> f32 {
        self.bots
            .with_knowledge(|knowledge| knowledge.pickup_utility(characters, state, pickup))
    }

    fn choose_weapon(
        &self,
        characters: &qa_bots::behavior::library::character::BotCharacterLibrary,
        state: &BotState,
    ) -> i32 {
        self.bots
            .with_knowledge(|knowledge| knowledge.choose_weapon(characters, state))
    }

    fn activation_weapon(
        &self,
        characters: &qa_bots::behavior::library::character::BotCharacterLibrary,
        state: &BotState,
    ) -> i32 {
        self.bots
            .with_knowledge(|knowledge| knowledge.activation_weapon(characters, state))
    }

    fn tactics(&self, weapon: i32) -> qa_bots::behavior::q3::game_host::BotWeaponTactics {
        self.bots.with_knowledge(|knowledge| knowledge.tactics(weapon))
    }

    fn aggression(&self, state: &BotState) -> f32 {
        self.bots.with_knowledge(|knowledge| knowledge.aggression(state))
    }

    fn update_inventory(&mut self, state: &mut BotState) {
        self.bots.update_knowledge_inventory(state);
    }
}

/// Director random handle over the live host random.
struct BotRandomHandle<'a, S, E, B> {
    bots: ApplicationBots<'a, S, E, B>,
}

impl<'a, S, E, B> BotRandom for BotRandomHandle<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn next_int(&mut self) -> i32 {
        (self.bots.host_random() * f64::from(i32::MAX)) as i32
    }

    fn next_unit(&mut self) -> f32 {
        self.bots.host_random() as f32
    }
}

/// Director game handle over the live shared/Q3 game.
struct BotGameHandle<'a, S, E, B> {
    bots: ApplicationBots<'a, S, E, B>,
    knowledge: BotKnowledgeHandle<'a, S, E, B>,
    random: BotRandomHandle<'a, S, E, B>,
}

impl<'a, S, E, B> SourceBotGame for BotGameHandle<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn product(&self) -> BotProduct {
        self.bots.game_product()
    }

    fn game_type(&self) -> i32 {
        self.bots.game_type()
    }

    fn max_clients(&self) -> i32 {
        self.bots.game_max_clients()
    }

    fn entity_count(&self) -> i32 {
        self.bots.game_entity_count()
    }

    fn clock(&self) -> qa_bots::behavior::q3::game_host::BotGameClock {
        self.bots.game_clock()
    }

    fn entity(&self, number: i32) -> qa_bots::behavior::q3::game_host::BotObservedEntity {
        self.bots.game_entity(number)
    }

    fn model_index(&self, name: &str) -> i32 {
        self.bots.game_model_index(name)
    }

    fn trace(&self, query: &BotTraceQuery) -> BotTraceResult {
        self.bots.game_trace(query)
    }

    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32 {
        self.bots.game_point_contents(point, pass_entity)
    }

    fn random(&mut self) -> &mut dyn BotRandom {
        &mut self.random
    }

    fn choose_team(&mut self, client: i32) -> i32 {
        self.bots.game_choose_team(client)
    }

    fn activate_bot(&mut self, client: i32) {
        self.bots.game_activate_bot(client);
    }

    fn exit_level(&mut self) {
        self.bots.game_exit_level();
    }

    fn client_userinfo_changed(&mut self, client: i32) {
        self.bots.game_client_userinfo_changed(client);
    }

    fn client_connect(&mut self, client: i32, first_time: bool, is_bot: bool) -> Option<String> {
        self.bots.game_client_connect(client, first_time, is_bot)
    }

    fn client_begin(&mut self, client: i32) {
        self.bots.game_client_begin(client);
    }

    fn pickup_candidates(&self, client: i32) -> Vec<BotObservedPickup> {
        self.bots.game_pickup_candidates(client)
    }

    fn knowledge(&mut self) -> &mut dyn BotArsenalKnowledge {
        &mut self.knowledge
    }
}

/// Director navigation handle over the live navigation runtime.
struct BotNavHandle<'w> {
    nav: Rc<RefCell<SourceBotNavigation<'w>>>,
}

impl BotNavigation for BotNavHandle<'_> {
    fn ready(&self) -> bool {
        self.nav.borrow().ready()
    }

    fn point_area(&self, origin: Vec3) -> i32 {
        self.nav.borrow().point_area(origin)
    }

    fn reachability_area(&self, origin: Vec3, client: i32) -> i32 {
        self.nav.borrow().reachability_area(origin, client)
    }

    fn fuzzy_point_reachability_area(&self, origin: Vec3) -> i32 {
        self.nav.borrow().fuzzy_point_reachability_area(origin)
    }

    fn area(&self, number: i32) -> BotNavigationArea {
        self.nav.borrow().area(number)
    }

    fn trace_areas(&self, start: Vec3, end: Vec3, maximum: usize) -> Vec<(i32, Vec3)> {
        self.nav.borrow().trace_areas(start, end, maximum)
    }

    fn bbox_areas(&self, bounds: &Bounds) -> Vec<i32> {
        self.nav.borrow().bbox_areas(bounds)
    }

    fn set_area_enabled(&mut self, area: i32, enabled: bool) {
        self.nav.borrow_mut().set_area_enabled(area, enabled);
    }

    fn area_travel_time_to_goal(&mut self, query: &AreaTravelTimeQuery) -> i32 {
        self.nav.borrow_mut().area_travel_time_to_goal(query)
    }

    fn route(&mut self, query: &RouteQuery) -> RouteResult {
        self.nav.borrow_mut().route(query)
    }

    fn predict_route(&mut self, query: &PredictRouteQuery) -> PredictedRoute {
        self.nav.borrow_mut().predict_route(query)
    }

    fn alternative_route_goals(&mut self, query: &AlternativeRouteQuery) -> Vec<AlternativeGoal> {
        self.nav.borrow_mut().alternative_route_goals(query)
    }

    fn move_to_goal(&mut self, result: &mut BotMoveResult, move_state: i32, goal: &BotGoal, travel_flags: i32) {
        self.nav
            .borrow_mut()
            .move_to_goal(result, move_state, goal, travel_flags);
    }

    fn move_in_direction(&mut self, move_state: i32, direction: Vec3, speed: f32, move_type: i32) -> bool {
        self.nav
            .borrow_mut()
            .move_in_direction(move_state, direction, speed, move_type)
    }

    fn movement_view_target(
        &self,
        move_state: i32,
        goal: &BotGoal,
        travel_flags: i32,
        look_ahead: f32,
    ) -> Option<Vec3> {
        self.nav
            .borrow()
            .movement_view_target(move_state, goal, travel_flags, look_ahead)
    }

    fn predict_visible_position(&self, origin: Vec3, area: i32, goal: &BotGoal, travel_flags: i32) -> Option<Vec3> {
        self.nav
            .borrow()
            .predict_visible_position(origin, area, goal, travel_flags)
    }

    fn swimming(&self, origin: Vec3) -> bool {
        self.nav.borrow().swimming(origin)
    }

    fn presence_bounds(&self, presence: i32) -> Bounds {
        self.nav.borrow().presence_bounds(presence)
    }
}

impl<'a, S, E, B> ApplicationBots<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn print(&self, text: &str) {
        (self.inner.borrow().print)(text);
    }

    fn time_ms(&self) -> i32 {
        (self.inner.borrow().simulation.borrow().time_seconds() * 1000.0) as i32
    }

    fn host_random(&self) -> f64 {
        let inner = self.inner.borrow();
        match inner.source.as_ref() {
            Some(source) => source.random(),
            None => inner.simulation.borrow().bot_random(),
        }
    }

    fn connection(&self, client: i32) -> Result<Rc<RefCell<BotConnection>>, ApplicationBotError> {
        self.inner
            .borrow()
            .connections
            .get(&client)
            .cloned()
            .ok_or(ApplicationBotError::MissingConnection(client))
    }

    fn connection_actor(&self, client: i32) -> Option<(ActorId, u32)> {
        self.inner.borrow().connections.get(&client).map(|connection| {
            let connection = connection.borrow();
            (connection.actor.id().clone(), connection.client.id().slot())
        })
    }

    /// Admit a session client (donor `prepare`).
    fn prepare(&self, client: SessionClient, reliable: ServerReliableCommands) -> Result<(), ApplicationBotError> {
        if client.is_closed() {
            return Err(ApplicationBotError::ClosedClient);
        }
        let actor = self
            .inner
            .borrow()
            .simulation
            .borrow_mut()
            .prepare_bot_client(client.id())?;
        let slot = client.id().slot() as i32;
        self.inner.borrow_mut().connections.insert(
            slot,
            Rc::new(RefCell::new(BotConnection {
                client,
                actor,
                reliable,
            })),
        );
        Ok(())
    }

    /// Allocate a bot client (donor `allocateClient`).
    fn allocate_client(&self) -> Option<i32> {
        let occupied: HashSet<u32> = {
            let inner = self.inner.borrow();
            let simulation = inner.simulation.borrow();
            simulation
                .players()
                .iter()
                .filter_map(|actor| simulation.mover(actor))
                .filter_map(|mover| mover.client.map(|client| client.slot()))
                .collect()
        };
        let max = self.game_max_clients();
        for slot in 0..max {
            if occupied.contains(&(slot as u32)) {
                continue;
            }
            let client = match self.inner.borrow().session.borrow_mut().create_client(slot as u32) {
                Ok(client) => client,
                Err(_) => continue,
            };
            let id = client.id().clone();
            match self.prepare(client, ServerReliableCommands::new()) {
                Ok(()) => return Some(slot),
                Err(_) => {
                    self.inner.borrow().session.borrow_mut().close_client(&id);
                }
            }
        }
        None
    }

    /// Encode a brain command (donor `encodeCommand`).
    ///
    /// Panics with the donor message when the connection, mover, source, or
    /// player state is missing: the director host contract is infallible.
    fn encode_command(&self, client: i32, command: &BotUserCommand) -> ClientCommand {
        let connection = self.connection(client).expect("Source bot has no session connection");
        let actor = connection.borrow().actor.id().clone();
        let mover = self
            .inner
            .borrow()
            .simulation
            .borrow()
            .mover(&actor)
            .expect("Admitted bot has no selected movement player");
        let source_command = Q3SourceUserCommand {
            server_time: command.server_time,
            angles: command.angles,
            buttons: command.buttons,
            weapon: command.weapon,
            forwardmove: command.forwardmove,
            rightmove: command.rightmove,
            upmove: command.upmove,
        };
        let elapsed = self.inner.borrow().elapsed_ms as i32;
        let (selected, provider, weapon, use_holdable) = {
            let (selected, provider, weapon, use_holdable) = if self.inner.borrow().shared.is_some() {
                let weapon = self
                    .with_binding(|binding| binding.resolve_weapon(client, command.weapon))
                    .flatten();
                (
                    selected_q3_command(&source_command, &mover.profile, elapsed),
                    mover.arsenal_provider.clone(),
                    weapon,
                    false,
                )
            } else {
                let source = self
                    .inner
                    .borrow()
                    .source
                    .clone()
                    .expect("Bot source observation is unavailable");
                let pool = source.pool_client(client).expect("Source bot lost its player state");
                let angles = if mover.profile.dialect() == MovementDialect::Q3 {
                    command.angles
                } else {
                    Vec3 {
                        x: ((command.angles.x as i32 + pool.delta_angles[0]) & 65535) as f32,
                        y: ((command.angles.y as i32 + pool.delta_angles[1]) & 65535) as f32,
                        z: ((command.angles.z as i32 + pool.delta_angles[2]) & 65535) as f32,
                    }
                };
                let has_arsenal = self.inner.borrow().arsenal.is_some();
                let weapon = if has_arsenal {
                    self.with_binding(|binding| binding.resolve_weapon(client, command.weapon))
                        .flatten()
                } else {
                    Q3_WEAPON_ITEMS
                        .iter()
                        .find(|item| item.weapon as i32 == command.weapon)
                        .map(|item| item.item.clone())
                };
                let adjusted = Q3SourceUserCommand {
                    angles,
                    weapon: if has_arsenal { pool.weapon } else { command.weapon },
                    ..source_command
                };
                (
                    selected_q3_command(&adjusted, &mover.profile, elapsed),
                    mover.arsenal_provider.clone(),
                    weapon,
                    command.buttons & CommandButtons::USE_HOLDABLE != 0,
                )
            };
            (movement_to_wire_command(&selected), provider, weapon, use_holdable)
        };
        self.inner.borrow_mut().stash.insert(
            client,
            EncodedBotCommand {
                command: selected,
                provider,
                weapon,
                use_holdable,
            },
        );
        ClientCommand {
            family: qa_world::client::ClientFamily::Q3,
            buttons: command.buttons,
            impulse: 0,
            forward_move: f64::from(command.forwardmove),
            side_move: 0.0,
            right_move: f64::from(command.rightmove),
            up_move: f64::from(command.upmove),
        }
    }

    /// Snapshot an entity number (donor `snapshotEntity`; the sequence arrives
    /// typed, so the donor range check is inherent).
    fn snapshot_entity(&self, client: i32, sequence: i32) -> i32 {
        if sequence < 0 {
            return -1;
        }
        if let Some(cached) = self.inner.borrow().snapshots.get(&client) {
            return cached.get(sequence as usize).copied().unwrap_or(-1);
        }
        let snapshot = if self.inner.borrow().source.is_none() {
            (0..self.game_entity_count())
                .filter(|number| {
                    let entity = self.game_entity(*number);
                    entity.present && entity.linked && !entity.hidden
                })
                .collect()
        } else {
            let (source, queries, leaf_count, print) = {
                let inner = self.inner.borrow();
                let queries = inner.simulation.borrow().q3_scene_queries();
                (
                    inner.source.clone().expect("Bot source observation is unavailable"),
                    queries,
                    inner.leaf_count,
                    Rc::clone(&inner.print),
                )
            };
            let player = source.source_clients().into_iter().find(|row| row.slot == client);
            let Some(player) = player else {
                return -1;
            };
            let mut emit = |text: &str| print(text);
            let visible = select_application_q3_snapshot(
                &player.state,
                &source.source_entities(),
                queries.as_ref(),
                &|number| source.link_bounds(number),
                leaf_count,
                &mut emit,
            )
            .expect("Bot snapshot selection failed");
            visible.entities.into_iter().map(|entity| entity.number).collect()
        };
        self.inner.borrow_mut().snapshots.insert(client, snapshot);
        self.inner
            .borrow()
            .snapshots
            .get(&client)
            .and_then(|snapshot| snapshot.get(sequence as usize).copied())
            .unwrap_or(-1)
    }

    /// Next console message (donor `consoleMessage`).
    fn console_message(&self, client: i32) -> Option<String> {
        let connection = self.connection(client).ok()?;
        let mut connection = connection.borrow_mut();
        if connection.reliable.acknowledge() == connection.reliable.sequence() {
            return None;
        }
        let acknowledged = connection.reliable.acknowledge();
        connection.reliable.assign_acknowledgement(acknowledged + 1);
        let text = connection.reliable.lookup_masked(connection.reliable.acknowledge());
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    /// Point contents (donor `game.world.pointContents(point, -1)`).
    fn point_contents(&self, point: Vec3) -> i32 {
        self.game_point_contents(point, -1)
    }

    /// Whether a bot name is taken (donor `nameInUse`).
    fn name_in_use(&self, name: &str) -> bool {
        let inner = self.inner.borrow();
        let simulation = inner.simulation.borrow();
        for actor in simulation.players() {
            let mover = simulation.mover(&actor);
            let slot = mover
                .as_ref()
                .and_then(|mover| mover.client.as_ref().map(|client| client.slot()));
            let candidate = match slot {
                None => match inner.source.as_ref() {
                    Some(source) => source.player_name(&actor),
                    None => inner
                        .shared
                        .as_ref()
                        .and_then(|shared| shared.borrow().client_name(&actor)),
                }
                .unwrap_or_default(),
                Some(slot) => {
                    let userinfo = match inner.source.as_ref() {
                        Some(source) => source.engine_userinfo(slot as i32),
                        None => inner
                            .shared
                            .as_ref()
                            .map(|shared| shared.borrow().engine_userinfo(slot as i32))
                            .unwrap_or_default(),
                    };
                    qa_net::q3_net::q3_info_value(&userinfo, "name").unwrap_or_default()
                }
            };
            if candidate == name {
                return true;
            }
        }
        false
    }

    fn with_q3game<T>(&self, read: impl for<'r> FnOnce(&'r (dyn SourceBotGame + 'r)) -> T) -> Option<T> {
        let game = self.inner.borrow().q3game.clone()?;
        let borrowed = game.borrow();
        Some(read(borrowed.as_ref()))
    }

    fn with_q3game_mut(&self, update: impl FnOnce(&mut dyn SourceBotGame)) {
        if let Some(game) = self.inner.borrow().q3game.clone() {
            update(game.borrow_mut().as_mut());
        }
    }

    fn game_product(&self) -> BotProduct {
        match self.inner.borrow().source.as_ref() {
            Some(source) if source.product() == "missionpack" => BotProduct::MissionPack,
            _ => BotProduct::BaseQ3,
        }
    }

    fn game_type(&self) -> i32 {
        self.inner
            .borrow()
            .source
            .as_ref()
            .map_or(0, |source| source.game_type())
    }

    fn game_max_clients(&self) -> i32 {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().max_clients();
        }
        self.with_q3game(|game| game.max_clients()).unwrap_or(0)
    }

    fn game_entity_count(&self) -> i32 {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().entity_count();
        }
        self.with_q3game(|game| game.entity_count()).unwrap_or(0)
    }

    fn game_clock(&self) -> qa_bots::behavior::q3::game_host::BotGameClock {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().clock();
        }
        self.with_q3game(|game| game.clock())
            .expect("Bot world projection is unavailable")
    }

    fn game_entity(&self, number: i32) -> qa_bots::behavior::q3::game_host::BotObservedEntity {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().entity(number);
        }
        self.with_q3game(|game| game.entity(number))
            .expect("Bot world projection is unavailable")
    }

    fn game_model_index(&self, name: &str) -> i32 {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().model_index(name);
        }
        self.with_q3game(|game| game.model_index(name))
            .expect("Bot world projection is unavailable")
    }

    fn game_trace(&self, query: &BotTraceQuery) -> BotTraceResult {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().trace(query);
        }
        self.with_q3game(|game| game.trace(query))
            .expect("Bot world projection is unavailable")
    }

    fn game_point_contents(&self, point: Vec3, pass_entity: i32) -> i32 {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().point_contents(point, pass_entity);
        }
        self.with_q3game(|game| game.point_contents(point, pass_entity))
            .expect("Bot world projection is unavailable")
    }

    fn game_choose_team(&self, client: i32) -> i32 {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow_mut().choose_team(client);
        }
        let mut outcome = 0;
        self.with_q3game_mut(|game| outcome = game.choose_team(client));
        outcome
    }

    fn game_activate_bot(&self, client: i32) {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            shared.borrow_mut().activate_bot(client);
            return;
        }
        self.with_q3game_mut(|game| game.activate_bot(client));
    }

    fn game_exit_level(&self) {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            shared.borrow_mut().exit_level();
            return;
        }
        self.with_q3game_mut(|game| game.exit_level());
    }

    fn game_client_userinfo_changed(&self, client: i32) {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            shared.borrow_mut().client_userinfo_changed(client);
            return;
        }
        self.with_q3game_mut(|game| game.client_userinfo_changed(client));
    }

    fn game_client_connect(&self, client: i32, first_time: bool, is_bot: bool) -> Option<String> {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow_mut().client_connect(client, first_time, is_bot);
        }
        let mut outcome = None;
        self.with_q3game_mut(|game| outcome = game.client_connect(client, first_time, is_bot));
        outcome
    }

    fn game_client_begin(&self, client: i32) {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            shared.borrow_mut().client_begin(client);
            return;
        }
        self.with_q3game_mut(|game| game.client_begin(client));
    }

    fn game_pickup_candidates(&self, client: i32) -> Vec<BotObservedPickup> {
        if let Some(shared) = self.inner.borrow().shared.clone() {
            return shared.borrow().pickup_candidates(client);
        }
        self.with_q3game(|game| game.pickup_candidates(client))
            .unwrap_or_default()
    }

    /// Read through the live arsenal binding (donor `arsenal ?? shared.knowledge`).
    fn with_binding<T>(&self, read: impl FnOnce(&dyn BotArsenalBinding) -> T) -> Option<T> {
        if let Some(arsenal) = self.inner.borrow().arsenal.clone() {
            return Some(read(arsenal.borrow().as_ref()));
        }
        if let Some(shared) = self.inner.borrow().shared.clone() {
            let world = shared.borrow();
            let binding = world.binding();
            return Some(read(&*binding));
        }
        None
    }

    /// Read through the live arsenal knowledge.
    fn with_knowledge<T>(&self, read: impl FnOnce(&dyn BotArsenalKnowledge) -> T) -> T {
        if let Some(arsenal) = self.inner.borrow().arsenal.clone() {
            let borrowed = arsenal.borrow();
            let knowledge: &dyn BotArsenalKnowledge = borrowed.as_ref();
            return read(knowledge);
        }
        if let Some(shared) = self.inner.borrow().shared.clone() {
            let world = shared.borrow();
            let binding = world.binding();
            let knowledge: &dyn BotArsenalKnowledge = &*binding;
            return read(knowledge);
        }
        read(&qa_bots::behavior::q3::arsenal_knowledge::DefaultArsenalKnowledge::new())
    }

    /// Update through the live arsenal binding.
    fn update_knowledge_inventory(&self, state: &mut BotState) {
        if let Some(arsenal) = self.inner.borrow().arsenal.clone() {
            arsenal.borrow_mut().update_inventory(state);
            return;
        }
        if let Some(shared) = self.inner.borrow().shared.clone() {
            shared.borrow_mut().knowledge().update_inventory(state);
        }
    }
}

impl<'a, S, E, B> ApplicationBots<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn from_inner(inner: Rc<RefCell<BotsInner<'a, S, E, B>>>) -> Self {
        Self { inner }
    }

    /// Reset a bot view to spawn angles (donor `resetView`).
    fn reset_view(&self, actor: &ActorId, angles: Vec3) {
        if !self.inner.borrow().simulation.borrow().is_live(actor) {
            return;
        }
        let entry = self.inner.borrow().director.clone().and_then(|director| {
            director
                .borrow()
                .roster()
                .into_iter()
                .find(|entry| &entry.actor == actor)
        });
        if let Some(entry) = entry {
            self.inner
                .borrow()
                .backing
                .borrow_mut()
                .director_set_bot_view(entry.source_client, angles);
        }
    }

    /// Reset a bot weapon to its spawn observation (donor `resetWeapon`).
    fn reset_weapon(&self, actor: &ActorId) -> Result<(), ApplicationBotError> {
        if !self.inner.borrow().simulation.borrow().is_live(actor) {
            return Ok(());
        }
        let entry = self.inner.borrow().director.clone().and_then(|director| {
            director
                .borrow()
                .roster()
                .into_iter()
                .find(|entry| &entry.actor == actor)
        });
        let Some(entry) = entry else {
            return Ok(());
        };
        let observed = self.game_entity(entry.source_client).player;
        let Some(observed) = observed else {
            return Err(ApplicationBotError::SpawnArsenal);
        };
        self.inner
            .borrow()
            .backing
            .borrow_mut()
            .director_set_spawn_weapon(entry.source_client, observed.weapon);
        Ok(())
    }

    /// Restore one client (donor `restoreClient`).
    fn restore_client(&self, saved: &BotClientSnapshot) -> Result<(), ApplicationBotError> {
        // Donor `restoreClient`: the preserved connection object itself goes
        // live (shared with the snapshot) with a freshly prepared actor.
        let id = saved.connection.borrow().client.id().clone();
        if saved.connection.borrow().client.is_closed() {
            return Err(ApplicationBotError::ClosedClient);
        }
        let actor = self.inner.borrow().simulation.borrow_mut().prepare_bot_client(&id)?;
        let slot = id.slot() as i32;
        saved.connection.borrow_mut().actor = actor;
        self.inner
            .borrow_mut()
            .connections
            .insert(slot, Rc::clone(&saved.connection));
        if self.inner.borrow().shared.is_some() {
            self.game_activate_bot(slot);
            let userinfo = saved.userinfo.clone().unwrap_or_default();
            if let Some(shared) = self.inner.borrow().shared.clone() {
                shared.borrow().set_engine_userinfo(slot, &userinfo);
            }
            if let Some(rejection) = self.game_client_connect(slot, false, true) {
                return Err(ApplicationBotError::RestorationRejected(rejection));
            }
            self.game_client_begin(slot);
            return Ok(());
        }
        let source = self
            .inner
            .borrow()
            .source
            .clone()
            .ok_or(ApplicationBotError::MissingSource)?;
        let userinfo = saved.userinfo.clone().unwrap_or_else(|| source.engine_userinfo(slot));
        source.set_engine_userinfo(slot, &userinfo);
        source.set_bot_flag(slot);
        source.activate_client(slot);
        if let Some(rejection) = source.admission_connect(slot, false, true) {
            return Err(ApplicationBotError::RestorationRejected(rejection));
        }
        source.admission_begin(slot);
        Ok(())
    }
}

/// Create the bot transport (donor `new ApplicationBots(options)`).
///
/// Returns the shared transport handle; the constructor attaches it to
/// `simulation.botServices` like the donor.
pub fn create_application_bots<'a, S, E, B>(
    options: ApplicationBotsOptions<'a, S, E, B>,
) -> Result<ApplicationBotsHandle<'a, S, E, B>, ApplicationBotError>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
    'a: 'static,
{
    if let Some(error) = bot_admission_error(&options.simulation.borrow().bot_admission()) {
        return Err(ApplicationBotError::Admission(error));
    }
    let source = options.simulation.borrow().q3_bot_source();
    if source.is_none() {
        if let Some(error) = legacy_bot_admission_error(&options.simulation.borrow().bot_admission()) {
            return Err(ApplicationBotError::Admission(error));
        }
    }
    if options.session.borrow().session_id() != options.simulation.borrow().session_id() {
        return Err(ApplicationBotError::SessionMismatch);
    }
    if source.is_none() && options.configuration.is_none() {
        return Err(ApplicationBotError::MissingConfiguration);
    }
    let inner = Rc::new(RefCell::new(BotsInner {
        session: options.session,
        simulation: options.simulation,
        files: options.files,
        transport: None,
        leaf_count: options.leaf_count,
        configuration: options.configuration,
        automatic_frame: options.automatic_frame,
        for_client: options.navigation.for_client.clone(),
        insert_console_command: options.insert_console_command,
        print: options.print,
        q3_game_factory: options.q3_game_factory,
        backing: Rc::new(RefCell::new(options.backing)),
        nav: Rc::new(RefCell::new(SourceBotNavigation::new(options.navigation.runtime))),
        source: source.clone(),
        arsenal: None,
        shared: None,
        q3game: None,
        director: None,
        connections: HashMap::new(),
        snapshots: HashMap::new(),
        elapsed_ms: 0.0,
        closed: false,
        phase: BotRestartPhase::Active,
        stash: HashMap::new(),
    }));
    let bots = ApplicationBots::from_inner(Rc::clone(&inner));
    let files = options.files;
    if let Some(source) = source.as_ref() {
        let weapons = WeaponAi::new(files);
        let simulation = inner.borrow().simulation.clone();
        let snapshot = simulation.borrow().clone();
        let arsenal = create_bot_arsenal_binding(
            snapshot,
            move |client| {
                simulation.borrow().players().into_iter().find(|actor| {
                    simulation
                        .borrow()
                        .mover(actor)
                        .and_then(|mover| mover.client)
                        .is_some_and(|id| id.slot() as i32 == client)
                })
            },
            &weapons,
        )
        .map(|binding| Rc::new(RefCell::new(binding)));
        inner.borrow_mut().arsenal = arsenal;
        let factory = inner
            .borrow()
            .q3_game_factory
            .clone()
            .ok_or(ApplicationBotError::MissingProjection)?;
        let console = Rc::clone(&inner.borrow().insert_console_command);
        let game = factory(source.as_ref(), console);
        inner.borrow_mut().q3game = Some(Rc::new(RefCell::new(game)));
    } else {
        let configuration = inner
            .borrow()
            .configuration
            .clone()
            .ok_or(ApplicationBotError::MissingConfiguration)?;
        let restoring = options.restore.is_some();
        let actor_inner = Rc::clone(&inner);
        let connect_inner = Rc::clone(&inner);
        let drop_inner = Rc::clone(&inner);
        let begin_inner = Rc::clone(&inner);
        let message_inner = Rc::clone(&inner);
        let simulation = inner.borrow().simulation.clone();
        let shared = create_shared_bot_world(
            SharedBotWorldOptions {
                restoring,
                simulation: simulation.borrow().clone(),
                cvars: configuration,
                actor: Rc::new(move |client| {
                    actor_inner
                        .borrow()
                        .connections
                        .get(&client)
                        .map(|connection| connection.borrow().actor.id().clone())
                }),
                connect: Rc::new(move |client, restart| {
                    connect_inner
                        .borrow()
                        .director
                        .clone()
                        .is_some_and(|director| director.borrow_mut().connect(client, !restart))
                }),
                drop_client: Rc::new(move |client| {
                    ApplicationBots::from_inner(Rc::clone(&drop_inner)).disconnect(client);
                }),
                begin: Rc::new(move |client| {
                    let this = ApplicationBots::from_inner(Rc::clone(&begin_inner));
                    let actor = this
                        .inner
                        .borrow()
                        .connections
                        .get(&client)
                        .map(|connection| connection.borrow().actor.id().clone())
                        .expect("Bot begin has no actual player");
                    let player = this
                        .inner
                        .borrow()
                        .simulation
                        .borrow()
                        .mover(&actor)
                        .expect("Bot begin has no actual view");
                    this.reset_view(&actor, player.view_angles);
                    this.reset_weapon(&actor)
                        .expect("Bot spawn has no actual arsenal observation");
                }),
                print: Rc::clone(&inner.borrow().print),
                console: Rc::clone(&inner.borrow().insert_console_command),
                message: Rc::new(move |client, text| {
                    for (slot, connection) in message_inner.borrow().connections.iter() {
                        if client == -1 || client == *slot {
                            let _ = connection.borrow_mut().reliable.add(text);
                        }
                    }
                }),
            },
            &WeaponAi::new(files),
        )
        .map_err(|error| ApplicationBotError::Simulation(format!("{error:?}")))?;
        inner.borrow_mut().shared = Some(Rc::new(RefCell::new(shared)));
    }
    let entities = match source.as_ref() {
        Some(source) => source.entities(),
        None => inner.borrow().simulation.borrow().source_entity_text(),
    };
    let game_handle = BotGameHandle {
        bots: ApplicationBots::from_inner(Rc::clone(&inner)),
        knowledge: BotKnowledgeHandle {
            bots: ApplicationBots::from_inner(Rc::clone(&inner)),
        },
        random: BotRandomHandle {
            bots: ApplicationBots::from_inner(Rc::clone(&inner)),
        },
    };
    let director = SourceBotDirector::new(SourceBotDirectorParams {
        host: Box::new(BotHostHandle {
            bots: ApplicationBots::from_inner(Rc::clone(&inner)),
        }),
        game: Box::new(game_handle),
        navigation: Box::new(BotNavHandle {
            nav: Rc::clone(&inner.borrow().nav),
        }),
        files,
        entities,
        item_config: "botfiles/items.c".to_string(),
        gametype: bots.game_type(),
        max_clients: bots.game_max_clients(),
        debug: false,
    })?;
    let director = Rc::new(RefCell::new(director));
    inner.borrow_mut().director = Some(Rc::clone(&director));
    let transport = Rc::new(RefCell::new(ApplicationBots::from_inner(Rc::clone(&inner))));
    inner.borrow_mut().transport = Some(Rc::downgrade(&transport));
    let mut attached = false;
    let outcome = (|| -> Result<(), ApplicationBotError> {
        if let Some(restore) = options.restore {
            bots.restore_checkpoint(&restore.image, restore.resolve_client.as_ref())?;
            inner.borrow().simulation.borrow_mut().bot_services().attach(
                Rc::clone(&director),
                transport.clone() as Rc<RefCell<dyn ApplicationBotService + 'a>>,
            )?;
            attached = true;
        } else {
            inner.borrow().simulation.borrow_mut().bot_services().attach(
                Rc::clone(&director),
                transport.clone() as Rc<RefCell<dyn ApplicationBotService + 'a>>,
            )?;
            attached = true;
            // The refactored `load` takes no restart flag (donor `load(restart)`
            // forwards it to `catalog.initializeBots`).
            let _ = options.restart;
            director
                .borrow_mut()
                .load()
                .map_err(|error| ApplicationBotError::Director(format!("{error:?}")))?;
            for client in &options.clients {
                bots.restore_client(client)?;
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        if attached {
            let _ = inner.borrow().simulation.borrow_mut().bot_services().detach(&director);
        }
        let print = Rc::clone(&inner.borrow().print);
        director.borrow_mut().close(&mut BotPrintSink { print });
        return Err(error);
    }
    Ok(transport)
}

/// Director restore context over the transport interior.
struct BotRestoreContext<'x, 'a, S, E, B> {
    bots: &'x ApplicationBots<'a, S, E, B>,
    observations: HashMap<i32, SavedActorId>,
}

impl<'x, 'a, S, E, B> BotDirectorRestoreContext for BotRestoreContext<'x, 'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn reference_saved(&self, saved: SavedActorId) -> ActorId {
        self.bots.inner.borrow().simulation.borrow().reference_saved(saved)
    }

    fn resolve_saved(&self, saved: &SavedActorId) -> ActorId {
        self.bots
            .inner
            .borrow()
            .simulation
            .borrow()
            .resolve_saved(saved)
            .map(|owned| owned.id().clone())
            .unwrap_or_else(|| self.bots.inner.borrow().simulation.borrow().reference_saved(*saved))
    }

    fn find_edge(&self, client: i32, edge: i32) -> Option<i32> {
        let inner = self.bots.inner.borrow();
        (inner.for_client)(client)
            .graph
            .edges
            .iter()
            .find(|value| value.id == edge)
            .map(|value| value.id)
    }

    fn remap_observation(&self, number: i32, generation: i32) -> (i32, i32) {
        match self.observations.get(&number) {
            Some(saved) if generation != 0 => {
                let remapped = self
                    .bots
                    .inner
                    .borrow()
                    .simulation
                    .borrow()
                    .reference_saved(SavedActorId {
                        slot: saved.slot,
                        generation: u32::try_from(generation).unwrap_or(0),
                    });
                (number, remapped.generation() as i32)
            }
            _ => (number, generation),
        }
    }
}

impl<'a, S, E, B> ApplicationBots<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    /// Shared configuration registry (donor `configuration`).
    fn configuration(&self) -> Rc<RefCell<CvarRegistry>> {
        if let Some(source) = self.inner.borrow().source.clone() {
            return source.cvars();
        }
        self.inner
            .borrow()
            .configuration
            .clone()
            .expect("Shared bot configuration requires the application console registry")
    }

    /// Capture the checkpoint image (donor `checkpoint`).
    fn checkpoint(&self) -> Result<SaveJson, ApplicationBotError> {
        if self.inner.borrow().closed {
            return Err(ApplicationBotError::CheckpointClosed);
        }
        let mut observations = Vec::new();
        if let Some(source) = self.inner.borrow().source.clone() {
            for (number, record) in source.capture_ownership().iter().enumerate() {
                let Some(actor) = record.actor.as_ref() else {
                    continue;
                };
                observations.push(obj(vec![
                    ("number", int(number as i64)),
                    ("actor", write_saved_actor(SavedActorId::from(actor.id()))),
                ]));
            }
        } else {
            let inner = self.inner.borrow();
            let simulation = inner.simulation.borrow();
            for actor in simulation.players() {
                let slot = simulation.mover(&actor).and_then(|mover| mover.client);
                if let Some(client) = slot {
                    observations.push(obj(vec![
                        ("number", int(i64::from(client.slot()))),
                        ("actor", write_saved_actor(SavedActorId::from(&actor))),
                    ]));
                }
            }
        }
        let knowledge = if let Some(arsenal) = self.inner.borrow().arsenal.clone() {
            Some(arsenal.borrow().checkpoint_binding())
        } else {
            self.inner
                .borrow()
                .shared
                .clone()
                .map(|shared| shared.borrow().binding().checkpoint_binding())
        };
        let navigation = save_value_to_json(&self.inner.borrow().nav.borrow().runtime().checkpoint().to_save_value());
        let shared_world = self
            .inner
            .borrow()
            .shared
            .clone()
            .map(|shared| shared.borrow().checkpoint())
            .unwrap_or(SaveJson::Null);
        Ok(obj(vec![
            ("version", int(1)),
            ("transport", self.checkpoint_orchestration()?),
            (
                "director",
                self.inner.borrow().backing.borrow().capture_director_state(),
            ),
            ("navigation", navigation),
            ("knowledge", knowledge.unwrap_or(SaveJson::Null)),
            ("sharedWorld", shared_world),
            ("observations", arr(observations)),
        ]))
    }

    /// Capture the transport checkpoint (donor `checkpointOrchestration`).
    fn checkpoint_orchestration(&self) -> Result<SaveJson, ApplicationBotError> {
        let inner = self.inner.borrow();
        if inner.closed {
            return Err(ApplicationBotError::CheckpointClosed);
        }
        let mut connections: Vec<(&i32, &Rc<RefCell<BotConnection>>)> = inner.connections.iter().collect();
        connections.sort_by_key(|(slot, _)| **slot);
        let mut snapshots: Vec<(&i32, &Vec<i32>)> = inner.snapshots.iter().collect();
        snapshots.sort_by_key(|(slot, _)| **slot);
        Ok(obj(vec![
            ("version", int(1)),
            ("elapsedMilliseconds", SaveJson::Number(inner.elapsed_ms)),
            (
                "connections",
                arr(connections
                    .into_iter()
                    .map(|(_, connection)| {
                        let connection = connection.borrow();
                        obj(vec![
                            (
                                "client",
                                obj(vec![
                                    ("slot", int(i64::from(connection.client.id().slot()))),
                                    ("generation", int(i64::from(connection.client.id().generation()))),
                                ]),
                            ),
                            ("actor", write_saved_actor(SavedActorId::from(connection.actor.id()))),
                            (
                                "reliable",
                                obj(vec![
                                    ("sequence", int(i64::from(connection.reliable.sequence()))),
                                    ("acknowledge", int(i64::from(connection.reliable.acknowledge()))),
                                    (
                                        "slots",
                                        arr((0..MAX_RELIABLE_COMMANDS as i32)
                                            .map(|index| str(&connection.reliable.lookup_masked(index)))
                                            .collect()),
                                    ),
                                ]),
                            ),
                        ])
                    })
                    .collect()),
            ),
            (
                "snapshots",
                arr(snapshots
                    .into_iter()
                    .map(|(client, entities)| {
                        obj(vec![
                            ("client", int(i64::from(*client))),
                            (
                                "entities",
                                arr(entities.iter().map(|entity| int(i64::from(*entity))).collect()),
                            ),
                        ])
                    })
                    .collect()),
            ),
        ]))
    }

    /// Restore a checkpoint image (donor `restoreCheckpoint`).
    fn restore_checkpoint(
        &self,
        image: &DecodedApplicationBotsCheckpoint,
        resolve_client: &dyn Fn(SavedBotIdentity) -> Option<SessionClient>,
    ) -> Result<(), ApplicationBotError> {
        if (image.shared_world == SaveJson::Null) != self.inner.borrow().shared.is_none() {
            return Err(ApplicationBotError::ProjectionMismatch);
        }
        let navigation = save_json_to_value(&image.navigation);
        self.inner
            .borrow()
            .nav
            .borrow_mut()
            .runtime_mut()
            .restore_checkpoint(&navigation)?;
        if let Some(shared) = self.inner.borrow().shared.clone() {
            let actor = |saved: SavedActorId| self.inner.borrow().simulation.borrow().reference_saved(saved);
            shared
                .borrow_mut()
                .restore_checkpoint(&image.shared_world, &actor)
                .map_err(|error| ApplicationBotError::Simulation(format!("{error:?}")))?;
        }
        self.restore_orchestration(&image.transport, resolve_client)?;
        let mut observations = HashMap::new();
        let native_ownership = self
            .inner
            .borrow()
            .source
            .clone()
            .map(|source| source.capture_ownership());
        for entry in &image.observations {
            let number = i32::try_from(entry.number).unwrap_or(i32::MAX);
            if observations.contains_key(&number) || self.inner.borrow().source.is_none() && entry.number >= 64 {
                return Err(ApplicationBotError::InvalidObservations);
            }
            let current = native_ownership
                .as_ref()
                .and_then(|ownership| ownership.get(usize::try_from(number).unwrap_or(usize::MAX)))
                .and_then(|record| record.actor.as_ref())
                .map(|actor| actor.id().clone())
                .or_else(|| {
                    self.inner
                        .borrow()
                        .shared
                        .clone()
                        .and_then(|shared| shared.borrow().actor_for_id(number))
                });
            let expected = self.inner.borrow().simulation.borrow().reference_saved(entry.actor);
            if current.as_ref() != Some(&expected) {
                return Err(ApplicationBotError::ObservationMismatch);
            }
            observations.insert(number, entry.actor);
        }
        let expected = native_ownership.map_or_else(
            || self.inner.borrow().simulation.borrow().players().len(),
            |ownership| ownership.iter().filter(|record| record.actor.is_some()).count(),
        );
        if observations.len() != expected {
            return Err(ApplicationBotError::IncompleteObservations);
        }
        let context = BotRestoreContext {
            bots: self,
            observations,
        };
        self.inner
            .borrow()
            .backing
            .borrow_mut()
            .restore_director_state(&image.director, &context)?;
        let has_knowledge = self.inner.borrow().arsenal.is_some() || self.inner.borrow().shared.is_some();
        if (image.knowledge == SaveJson::Null) == has_knowledge {
            return Err(ApplicationBotError::KnowledgeMismatch);
        }
        if image.knowledge != SaveJson::Null {
            let reader = SaveReader::at(&image.knowledge, "application.bots.knowledge");
            reader
                .field("actors")
                .list(|entry| -> Result<(), ApplicationBotError> {
                    let handle = entry.field("handle").integer(0)?;
                    if handle > i64::from(MAX_WEAPON_STATES as i32) {
                        return Err(ApplicationBotError::WeaponHandle);
                    }
                    Ok(())
                })?;
            let actor = |saved: SavedActorId| self.inner.borrow().simulation.borrow().reference_saved(saved);
            let weapon_handle = |handle: u32| i64::from(handle);
            if let Some(arsenal) = self.inner.borrow().arsenal.clone() {
                arsenal.borrow_mut().restore_binding(reader, &actor, &weapon_handle)?;
            } else if let Some(shared) = self.inner.borrow().shared.clone() {
                shared.borrow().restore_binding(reader, &actor, &weapon_handle)?;
            }
        }
        Ok(())
    }

    /// Restore the transport checkpoint (donor `restoreOrchestration`).
    fn restore_orchestration(
        &self,
        image: &ApplicationBotTransportCheckpoint,
        resolve_client: &dyn Fn(SavedBotIdentity) -> Option<SessionClient>,
    ) -> Result<(), ApplicationBotError> {
        if self.inner.borrow().closed
            || image.version != 1
            || !image.elapsed_milliseconds.is_finite()
            || image.elapsed_milliseconds < 0.0
        {
            return Err(ApplicationBotError::InvalidRestoration);
        }
        let mut connections: HashMap<i32, Rc<RefCell<BotConnection>>> = HashMap::new();
        let mut snapshots: HashMap<i32, Vec<i32>> = HashMap::new();
        let mut actors: HashSet<ActorId> = HashSet::new();
        for entry in &image.connections {
            let client = resolve_client(entry.client).ok_or(ApplicationBotError::InvalidBinding)?;
            let actor = self.inner.borrow().simulation.borrow().resolve_saved(&entry.actor);
            let slot = client.id().slot();
            if client.is_closed()
                || !self.inner.borrow().session.borrow().owns_client(client.id())
                || slot != entry.client.slot
                || actor.is_none()
                || connections.contains_key(&(slot as i32))
            {
                return Err(ApplicationBotError::InvalidBinding);
            }
            let actor = actor.expect("saved bot transport binding is unavailable");
            if !actors.insert(actor.id().clone()) {
                return Err(ApplicationBotError::InvalidBinding);
            }
            let mover = self.inner.borrow().simulation.borrow().mover(actor.id());
            if mover.as_ref().and_then(|mover| mover.client.clone()) != Some(client.id().clone()) {
                return Err(ApplicationBotError::ClientMismatch);
            }
            let reliable = restored_bot_reliable_commands(&entry.reliable)?;
            connections.insert(
                slot as i32,
                Rc::new(RefCell::new(BotConnection {
                    client,
                    actor,
                    reliable,
                })),
            );
        }
        for snapshot in &image.snapshots {
            let client = i32::try_from(snapshot.client).unwrap_or(i32::MAX);
            if !connections.contains_key(&client) || snapshots.contains_key(&client) {
                return Err(ApplicationBotError::InvalidSnapshot);
            }
            let mut entities = Vec::new();
            for entity in &snapshot.entities {
                let entity = i32::try_from(*entity).unwrap_or(-1);
                if entity < 0 || entity >= self.game_entity_count() {
                    return Err(ApplicationBotError::InvalidSnapshot);
                }
                entities.push(entity);
            }
            snapshots.insert(client, entities);
        }
        let mut inner = self.inner.borrow_mut();
        inner.connections.clear();
        inner.snapshots.clear();
        inner.connections.extend(connections);
        inner.snapshots.extend(snapshots);
        inner.elapsed_ms = image.elapsed_milliseconds;
        Ok(())
    }
}

impl<'a, S, E, B> ApplicationBots<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    /// Live client snapshots (donor `clients`).
    fn clients(&self) -> Vec<BotClientSnapshot> {
        let inner = self.inner.borrow();
        if !matches!(inner.phase, BotRestartPhase::Active) {
            let preserved = match &inner.phase {
                BotRestartPhase::Active => Vec::new(),
                BotRestartPhase::Detached { clients, .. } | BotRestartPhase::Bound { clients, .. } => clients.clone(),
            };
            return preserved
                .into_iter()
                .filter(|snapshot| !snapshot.connection.borrow().client.is_closed())
                .collect();
        }
        inner
            .connections
            .values()
            .map(|connection| {
                let slot = connection.borrow().client.id().slot() as i32;
                let userinfo = match inner.source.as_ref() {
                    Some(source) => source.engine_userinfo(slot),
                    None => inner
                        .shared
                        .as_ref()
                        .map(|shared| shared.borrow().engine_userinfo(slot))
                        .unwrap_or_default(),
                };
                BotClientSnapshot {
                    connection: Rc::clone(connection),
                    userinfo: Some(userinfo),
                }
            })
            .collect()
    }

    /// Append a reliable command (donor `appendReliable`).
    fn append_reliable(&self, snapshot: &BotClientSnapshot, text: &str) -> bool {
        let overflow = {
            let connection = snapshot.connection.borrow();
            connection.reliable.clone()
        };
        let mut ring = overflow;
        if ring.add(text).is_ok() {
            snapshot.connection.borrow_mut().reliable = ring;
            return true;
        }
        let slot = snapshot.connection.borrow().client.id().slot() as i32;
        self.drop_engine_client(slot, "Server command overflow");
        self.disconnect(slot);
        false
    }

    /// Drop a client through the live game engine (donor `game.options.engine.dropClient`).
    fn drop_engine_client(&self, slot: i32, reason: &str) {
        let inner = self.inner.borrow();
        if let Some(shared) = inner.shared.as_ref() {
            shared.borrow().drop_client(slot, reason);
        } else if let Some(source) = inner.source.as_ref() {
            source.drop_client(slot, reason);
        }
    }

    /// Run a bot frame (donor `frame`).
    fn frame(
        &self,
        time_milliseconds: f64,
        elapsed_milliseconds: f64,
    ) -> Result<Vec<ActorCommand>, ApplicationBotError> {
        if self.inner.borrow().closed {
            return Err(ApplicationBotError::Closed);
        }
        if !matches!(self.inner.borrow().phase, BotRestartPhase::Active) {
            return Ok(Vec::new());
        }
        self.inner.borrow_mut().elapsed_ms = elapsed_milliseconds;
        self.inner.borrow_mut().snapshots.clear();
        if let Some(shared) = self.inner.borrow().shared.clone() {
            shared
                .borrow()
                .refresh()
                .map_err(|error| ApplicationBotError::Simulation(format!("{error:?}")))?;
        }
        let automatic = self.inner.borrow().automatic_frame;
        let source_time = if automatic {
            self.inner.borrow().source.clone().map_or(time_milliseconds, |source| {
                if source.cvars().borrow().variable_value("dedicated") != 0.0 {
                    f64::from(source.level_time_ms())
                } else {
                    time_milliseconds
                }
            })
        } else {
            time_milliseconds
        };
        let director = self
            .inner
            .borrow()
            .director
            .clone()
            .expect("Bot director is unavailable");
        let mut director = director.borrow_mut();
        let simulation = self.inner.borrow().simulation.clone();
        let mut population =
            SharedBotPopulation::new(&mut director, move |actor: &ActorId| simulation.borrow().is_live(actor));
        let frame = population.frame(BotFrame {
            time_milliseconds: source_time as i32,
            elapsed_milliseconds: elapsed_milliseconds as i32,
        })?;
        drop(population);
        drop(director);
        let mut commands = Vec::new();
        for issued in frame {
            let slot = self
                .inner
                .borrow()
                .connections
                .iter()
                .find(|(_, connection)| connection.borrow().actor.id() == &issued.actor)
                .map(|(slot, _)| *slot);
            let Some(slot) = slot else {
                continue;
            };
            let Some(stash) = self.inner.borrow().stash.get(&slot).cloned() else {
                continue;
            };
            let connection = self.connection(slot)?;
            let client = connection.borrow().client.id().clone();
            commands.push(ActorCommand {
                actor: issued.actor,
                source: CommandSource::Bot { client },
                sequence: u64::from(issued.sequence),
                command: stash.command,
                arsenal: Some(ArsenalIntent {
                    provider: stash.provider,
                    weapon: stash.weapon,
                    use_holdable: stash.use_holdable,
                }),
            });
        }
        Ok(commands)
    }

    /// Receive presentation events (donor `receive`).
    fn receive(&self, events: &[SimulationPresentationEvent]) {
        for event in events {
            if let SourcePresentationEvent::ViewReset { reason, actor, angles } = &event.event {
                if self.inner.borrow().shared.is_some() {
                    self.reset_view(actor, *angles);
                    if *reason == ViewResetReason::Spawn {
                        let _ = self.reset_weapon(actor);
                    }
                }
                continue;
            }
            let SourcePresentationEvent::Q3Source(Q3SourceEvent::ServerCommand { client, text }) = &event.event else {
                continue;
            };
            for snapshot in self.clients() {
                let (id, actor) = {
                    let connection = snapshot.connection.borrow();
                    (connection.client.id().clone(), connection.actor.id().clone())
                };
                if event.recipient.as_ref().is_some_and(|recipient| recipient != &actor) {
                    continue;
                }
                if *client != -1 && *client != id.slot() as i32 {
                    continue;
                }
                self.append_reliable(&snapshot, text);
            }
        }
    }

    /// Run a console command (donor `consoleCommand`).
    fn console_command(&self, argv: &[String]) {
        if let Some(director) = self.inner.borrow().director.clone() {
            let _ = director.borrow_mut().console_command(argv);
        }
    }

    /// Disconnect a client slot (donor `disconnect`).
    fn disconnect(&self, client: i32) -> bool {
        let connection = self.inner.borrow().connections.get(&client).cloned();
        let phase_client = match &self.inner.borrow().phase {
            BotRestartPhase::Active => None,
            BotRestartPhase::Detached { clients, .. } | BotRestartPhase::Bound { clients, .. } => clients
                .iter()
                .find(|snapshot| snapshot.connection.borrow().client.id().slot() as i32 == client)
                .cloned(),
        };
        let target = connection
            .clone()
            .or_else(|| phase_client.map(|snapshot| snapshot.connection));
        let Some(target) = target else {
            return false;
        };
        if target.borrow().client.is_closed() {
            return false;
        }
        if let Some(connection) = connection {
            if self.inner.borrow().source.is_none() {
                if let Some(director) = self.inner.borrow().director.clone() {
                    director.borrow_mut().shutdown_client(client, false);
                }
            }
            let actor = connection.borrow().actor.id().clone();
            self.inner.borrow().simulation.borrow_mut().disconnect_player(&actor);
        }
        self.inner.borrow_mut().connections.remove(&client);
        self.inner.borrow_mut().snapshots.remove(&client);
        if let BotRestartPhase::Bound { pending, .. } = &mut self.inner.borrow_mut().phase {
            pending.remove(&client);
        }
        let id = target.borrow().client.id().clone();
        self.inner.borrow().session.borrow_mut().close_client(&id);
        let _ = target.borrow_mut().client.close();
        true
    }

    /// Close the transport (donor `close`).
    fn close(&self, restart: bool) {
        if self.inner.borrow().closed {
            return;
        }
        let _ = restart;
        let director = self
            .inner
            .borrow()
            .director
            .clone()
            .expect("Bot director is unavailable");
        let print = Rc::clone(&self.inner.borrow().print);
        director.borrow_mut().close(&mut BotPrintSink { print });
        if !matches!(self.inner.borrow().phase, BotRestartPhase::Detached { .. }) {
            self.inner
                .borrow()
                .simulation
                .borrow_mut()
                .bot_services()
                .detach(&director)
                .expect("Cannot detach a different source bot director");
        }
        self.inner.borrow_mut().snapshots.clear();
        self.inner.borrow_mut().connections.clear();
        self.inner.borrow_mut().closed = true;
    }

    /// Detach for a round restart (donor `beginRoundRestart`).
    fn begin_round_restart(&self) -> Result<Vec<BotClientSnapshot>, ApplicationBotError> {
        let source = self.inner.borrow().source.clone();
        if self.inner.borrow().closed
            || !matches!(self.inner.borrow().phase, BotRestartPhase::Active)
            || source.is_none()
        {
            return Err(ApplicationBotError::RestartRequiresActive);
        }
        let clients = self.clients();
        self.inner.borrow().backing.borrow_mut().director_begin_round();
        let director = self
            .inner
            .borrow()
            .director
            .clone()
            .expect("Bot director is unavailable");
        self.inner
            .borrow()
            .simulation
            .borrow_mut()
            .bot_services()
            .detach(&director)?;
        self.inner.borrow_mut().connections.clear();
        self.inner.borrow_mut().snapshots.clear();
        self.inner.borrow_mut().elapsed_ms = 0.0;
        self.inner.borrow_mut().phase = BotRestartPhase::Detached {
            source: source.expect("Bot fast restart requires an active native Q3 round"),
            clients: clients.clone(),
        };
        Ok(clients)
    }

    /// Bind a restarted round (donor `bindRestartedRound`).
    fn bind_restarted_round(&self) -> Result<(), ApplicationBotError> {
        let detached = matches!(self.inner.borrow().phase, BotRestartPhase::Detached { .. });
        let source = self.inner.borrow().simulation.borrow().q3_bot_source();
        let stale = match (&self.inner.borrow().phase, source.as_ref()) {
            (BotRestartPhase::Detached { source: previous, .. }, Some(next)) => Rc::ptr_eq(previous, next),
            _ => true,
        };
        if self.inner.borrow().closed || !detached || source.is_none() || stale {
            return Err(ApplicationBotError::RestartRequiresRound);
        }
        let source = source.expect("Publish a new source round before rebinding bots");
        self.inner.borrow().nav.borrow_mut().runtime_mut().invalidate();
        self.inner.borrow_mut().source = Some(Rc::clone(&source));
        let files = self.inner.borrow().files;
        let weapons = WeaponAi::new(files);
        let simulation = self.inner.borrow().simulation.clone();
        let snapshot = simulation.borrow().clone();
        let arsenal = create_bot_arsenal_binding(
            snapshot,
            move |client| {
                simulation.borrow().players().into_iter().find(|actor| {
                    simulation
                        .borrow()
                        .mover(actor)
                        .and_then(|mover| mover.client)
                        .is_some_and(|id| id.slot() as i32 == client)
                })
            },
            &weapons,
        )
        .map(|binding| Rc::new(RefCell::new(binding)));
        self.inner.borrow_mut().arsenal = arsenal;
        let factory = self
            .inner
            .borrow()
            .q3_game_factory
            .clone()
            .ok_or(ApplicationBotError::MissingProjection)?;
        let console = Rc::clone(&self.inner.borrow().insert_console_command);
        let game = factory(source.as_ref(), console);
        match self.inner.borrow().q3game.clone() {
            Some(cell) => *cell.borrow_mut() = game,
            None => self.inner.borrow_mut().q3game = Some(Rc::new(RefCell::new(game))),
        }
        self.inner.borrow().backing.borrow_mut().director_bind_round();
        let director = self
            .inner
            .borrow()
            .director
            .clone()
            .expect("Bot director is unavailable");
        let transport = self
            .inner
            .borrow()
            .transport
            .clone()
            .and_then(|weak| weak.upgrade())
            .expect("Bot transport handle is unavailable");
        self.inner
            .borrow()
            .simulation
            .borrow_mut()
            .bot_services()
            .attach(director, transport as Rc<RefCell<dyn ApplicationBotService + 'a>>)?;
        let clients = match &self.inner.borrow().phase {
            BotRestartPhase::Detached { clients, .. } => clients.clone(),
            _ => Vec::new(),
        };
        let pending = clients
            .iter()
            .filter(|snapshot| !snapshot.connection.borrow().client.is_closed())
            .map(|snapshot| snapshot.connection.borrow().client.id().slot() as i32)
            .collect();
        self.inner.borrow_mut().phase = BotRestartPhase::Bound { clients, pending };
        Ok(())
    }

    /// Reconnect one preserved client (donor `reconnectRestartedClient`).
    fn reconnect_restarted_client(&self, client: &ClientId) -> Result<bool, ApplicationBotError> {
        if self.inner.borrow().closed || !matches!(self.inner.borrow().phase, BotRestartPhase::Bound { .. }) {
            return Err(ApplicationBotError::RestartRequiresBind);
        }
        let saved = match &self.inner.borrow().phase {
            BotRestartPhase::Bound { clients, .. } => clients
                .iter()
                .find(|snapshot| snapshot.connection.borrow().client.id() == client)
                .cloned(),
            _ => None,
        };
        let Some(saved) = saved else {
            return Ok(false);
        };
        let slot = client.slot() as i32;
        if saved.connection.borrow().client.is_closed() {
            if let BotRestartPhase::Bound { pending, .. } = &mut self.inner.borrow_mut().phase {
                pending.remove(&slot);
            }
            return Ok(true);
        }
        let pending = match &self.inner.borrow().phase {
            BotRestartPhase::Bound { pending, .. } => pending.contains(&slot),
            _ => false,
        };
        if !pending {
            return Err(ApplicationBotError::RestartDuplicate);
        }
        if !self.append_reliable(&saved, "map_restart\n") {
            return Ok(true);
        }
        self.restore_client(&saved)?;
        if let BotRestartPhase::Bound { pending, .. } = &mut self.inner.borrow_mut().phase {
            pending.remove(&slot);
        }
        Ok(true)
    }

    /// Resume round bots (donor `resumeRoundBots`).
    fn resume_round_bots(&self) -> Result<(), ApplicationBotError> {
        let ready = match &self.inner.borrow().phase {
            BotRestartPhase::Bound { pending, .. } => pending.is_empty(),
            _ => false,
        };
        if self.inner.borrow().closed || !ready {
            return Err(ApplicationBotError::RestartPending);
        }
        self.inner.borrow().backing.borrow_mut().director_resume_round();
        self.inner.borrow_mut().phase = BotRestartPhase::Active;
        Ok(())
    }
}

impl<'a, S, E, B> ApplicationBotService for ApplicationBots<'a, S, E, B>
where
    S: ApplicationBotSimulation<'a>,
    E: ApplicationBotSession,
    B: BotDirectorBacking,
{
    fn configuration(&self) -> Rc<RefCell<CvarRegistry>> {
        ApplicationBots::configuration(self)
    }

    fn automatic_frame(&self) -> bool {
        self.inner.borrow().automatic_frame
    }

    fn route_for_client(&self, client: i32, start: Vec3, goal: Vec3) -> Result<NavigationRouteResult, BotsError> {
        let runtime = (self.inner.borrow().for_client)(client);
        let mut runtime = runtime;
        runtime.route(&NavigationRouteQuery {
            start,
            goal,
            start_node: None,
            goal_node: None,
            travel_flags: None,
            disabled_areas: HashSet::new(),
            edge_filter: None,
            maximum_searches: None,
        })
    }

    fn mover_client_slot(&self, actor: &ActorId) -> Option<i32> {
        self.inner
            .borrow()
            .simulation
            .borrow()
            .mover(actor)
            .and_then(|mover| mover.client)
            .map(|client| client.slot() as i32)
    }

    fn move_to_point(&mut self, _actor: &ActorId, _point: Vec3, _tolerance: f64) -> RereleaseGoalStatus {
        0
    }

    fn follow_actor(&mut self, _actor: &ActorId, _target: &ActorId) -> RereleaseGoalStatus {
        0
    }

    fn is_bot(&self, actor: &ActorId) -> bool {
        self.inner
            .borrow()
            .connections
            .values()
            .any(|connection| connection.borrow().actor.id() == actor)
    }

    fn service_actor(&self, client: &ClientId) -> Option<ActorId> {
        let connection = self.inner.borrow().connections.get(&(client.slot() as i32)).cloned()?;
        let connection = connection.borrow();
        if connection.client.id() == client {
            Some(connection.actor.id().clone())
        } else {
            None
        }
    }

    fn frame(
        &mut self,
        time_milliseconds: f64,
        elapsed_milliseconds: f64,
    ) -> Result<Vec<ActorCommand>, ApplicationBotError> {
        ApplicationBots::frame(self, time_milliseconds, elapsed_milliseconds)
    }

    fn receive(&mut self, events: &[SimulationPresentationEvent]) {
        ApplicationBots::receive(self, events);
    }

    fn clients(&self) -> Vec<BotClientSnapshot> {
        ApplicationBots::clients(self)
    }

    fn console_command(&mut self, argv: &[String]) -> Result<(), ApplicationBotError> {
        ApplicationBots::console_command(self, argv);
        Ok(())
    }

    fn disconnect(&mut self, client: i32) -> bool {
        ApplicationBots::disconnect(self, client)
    }

    fn close(&mut self, restart: bool) {
        ApplicationBots::close(self, restart);
    }

    fn checkpoint(&self) -> Result<SaveJson, ApplicationBotError> {
        ApplicationBots::checkpoint(self)
    }

    fn begin_round_restart(&mut self) -> Result<Vec<BotClientSnapshot>, ApplicationBotError> {
        ApplicationBots::begin_round_restart(self)
    }

    fn bind_restarted_round(&mut self) -> Result<(), ApplicationBotError> {
        ApplicationBots::bind_restarted_round(self)
    }

    fn reconnect_restarted_client(&mut self, client: &ClientId) -> Result<bool, ApplicationBotError> {
        ApplicationBots::reconnect_restarted_client(self, client)
    }

    fn resume_round_bots(&mut self) -> Result<(), ApplicationBotError> {
        ApplicationBots::resume_round_bots(self)
    }

    fn director_remove_queued_begin(&mut self, client: i32) {
        self.inner
            .borrow()
            .backing
            .borrow_mut()
            .director_remove_queued_begin(client);
    }

    fn director_interbreed_end_match(&mut self) {
        self.inner.borrow().backing.borrow_mut().director_interbreed_end_match();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admission_view() -> BotAdmissionView {
        BotAdmissionView {
            has_q3: false,
            weapons_provider: None,
            q3_product: None,
            has_q2_weapons: false,
            has_q1_weapons: false,
            has_selected_q3_weapons: false,
            deathmatch: true,
            q3_game_type: None,
            has_q1: false,
            has_q2: false,
            q1_program: None,
            q1_teamplay: 0.0,
            q2_match_standard: true,
        }
    }

    #[test]
    fn q3_native_weapons_admit() {
        let view = BotAdmissionView {
            has_q3: true,
            weapons_provider: Some("q3:railgun".to_string()),
            ..admission_view()
        };
        assert_eq!(bot_admission_error(&view), None);
    }

    #[test]
    fn q3_foreign_weapons_reject() {
        let view = BotAdmissionView {
            has_q3: true,
            weapons_provider: Some("q2:railgun".to_string()),
            q3_product: Some("missionpack".to_string()),
            has_q2_weapons: true,
            ..admission_view()
        };
        assert!(bot_admission_error(&view).is_some());
    }

    #[test]
    fn shared_world_without_arsenal_rejects() {
        assert!(bot_admission_error(&admission_view()).is_some());
        let view = BotAdmissionView {
            has_q1: true,
            has_q1_weapons: true,
            ..admission_view()
        };
        assert_eq!(bot_admission_error(&view), None);
    }

    #[test]
    fn legacy_team_modes_reject() {
        assert!(legacy_bot_admission_error(&admission_view()).is_none());
        let view = BotAdmissionView {
            deathmatch: false,
            ..admission_view()
        };
        assert!(legacy_bot_admission_error(&view).is_some());
        let view = BotAdmissionView {
            has_q1: true,
            q1_program: Some("hipnotic".to_string()),
            ..admission_view()
        };
        assert!(legacy_bot_admission_error(&view).is_some());
    }

    #[test]
    fn reliable_ring_restore_validates_slots() {
        let image = SavedBotReliable {
            sequence: 3,
            acknowledge: 1,
            slots: vec![String::new(); MAX_RELIABLE_COMMANDS],
        };
        let ring = restored_bot_reliable_commands(&image).expect("valid ring restores");
        assert_eq!(ring.sequence(), 3);
        assert_eq!(ring.acknowledge(), 1);
        let short = SavedBotReliable {
            slots: vec![String::new(); 3],
            ..image
        };
        assert!(restored_bot_reliable_commands(&short).is_err());
    }

    #[test]
    fn checkpoint_decode_round_trip_empty_transport() {
        let value = obj(vec![
            ("version", int(1)),
            (
                "transport",
                obj(vec![
                    ("version", int(1)),
                    ("elapsedMilliseconds", SaveJson::Number(16.0)),
                    ("connections", arr(Vec::new())),
                    ("snapshots", arr(Vec::new())),
                ]),
            ),
            ("observations", arr(Vec::new())),
        ]);
        let decoded = decode_application_bots_checkpoint(&value).expect("empty image decodes");
        assert_eq!(decoded.version, 1);
        assert!(decoded.transport.connections.is_empty());
        assert!(decoded.transport.snapshots.is_empty());
        assert!(decoded.observations.is_empty());
    }

    #[test]
    fn checkpoint_decode_rejects_version() {
        let value = obj(vec![("version", int(2))]);
        assert!(decode_application_bots_checkpoint(&value).is_err());
    }

    #[test]
    fn save_numbers_keep_integer_cells() {
        assert_eq!(save_json_to_value(&SaveJson::Number(7.0)), SaveValue::Int(7));
        assert_eq!(save_json_to_value(&SaveJson::Number(7.5)), SaveValue::Float(7.5));
        assert_eq!(save_value_to_json(&SaveValue::Int(7)), SaveJson::Number(7.0));
    }
}
