//! Local-server snapshot transport for one Quake III seat.
//!
//! Port of `src/app/bootstrap/q3-client/source.ts`
//! (`ApplicationQ3Source`). One seat's local server transport retains
//! the same snapshot and command rings as cgame. Selection, actor
//! resolution, prediction adaptation, and level shots are injected
//! closures; simulation events arrive as [`Q3SourcePresentationEvent`]
//! because the donor's `SimulationPresentationEvent` union lives outside
//! this wave's scope. Session sameness arrives as a closure because
//! [`ActorId`](qa_core::identity::ActorId) keeps its session token with
//! the identity owner.

use std::collections::HashMap;

use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::base::shared::player_state::UserCommand as PredictionCommand;
use qa_content::q3::presentation::prediction::ClientCommandHistory;
use qa_content::q3::presentation::snapshots::SnapshotCurrent;
use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::identity::ActorId;
use qa_guest::qvm::player_record::QvmPlayerState;
use qa_net::common::commands::{ActorCommand, UserCommand as ActorUserCommand};
use qa_net::q3_net::{Q3PlayerSlots, Q3PlayerState, Q3Product, Snapshot};
use qa_net::q3_visibility::Q3VisibleEntities;
use thiserror::Error;

use crate::bootstrap::simulation::q3::types::Q3SourcePresentationState;

/// Configstring space.
const MAX_CONFIGSTRINGS: usize = 1024;
/// Retained snapshots.
const SNAPSHOT_BACKUP: i32 = 32;
/// Retained reliable commands.
const COMMAND_BACKUP: i32 = 64;

/// Local-source failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3SourceError {
    /// Q3 presentation seat has no source player.
    #[error("Q3 presentation seat has no source player")]
    NoSourcePlayer,
    /// Local Q3 round restart is already pending.
    #[error("Local Q3 round restart is already pending")]
    RestartPending,
    /// Invalid local Q3 restart epoch.
    #[error("Invalid local Q3 restart epoch")]
    InvalidRestartEpoch,
    /// Invalid local Q3 round actor binding.
    #[error("Invalid local Q3 round actor binding")]
    InvalidRoundActor,
    /// Local Q3 restart requires actor rebinding before publication.
    #[error("Local Q3 restart requires actor rebinding before publication")]
    RestartRequiresRebind,
    /// Q3 presentation world time rewound.
    #[error("Q3 presentation world time rewound")]
    TimeRewound,
    /// Q3 presentation seat changed source player.
    #[error("Q3 presentation seat changed source player")]
    SeatChangedPlayer,
    /// Local Q3 actor binding is suspended for restart.
    #[error("Local Q3 actor binding is suspended for restart")]
    ActorBindingSuspended,
    /// Q3 sound entity has no source actor.
    #[error("Q3 sound entity {0} has no source actor")]
    NoSourceActor(i32),
    /// Invalid local source configstring command.
    #[error("Invalid local source configstring command")]
    BadConfigstringCommand,
    /// Area mask exceeds 32 bytes.
    #[error("Local Q3 area mask exceeds 32 bytes")]
    BadAreaMask,
    /// Foreign movement needs its private prediction adapter.
    #[error("Selected foreign movement requires its private cgame prediction command adapter")]
    ForeignMovement,
    /// Server-command text failed tokenizing.
    #[error("Local Q3 server command failed tokenizing: {0}")]
    BadServerCommand(String),
}

/// Presentation event feeding the local source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SourcePresentationEvent {
    /// Event sequence.
    pub sequence: i64,
    /// Event payload.
    pub event: Q3SourceEvent,
}

/// Local-source presentation payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3SourceEvent {
    /// Server command for one client (`client < 0` broadcasts).
    ServerCommand {
        /// Target client, or negative for broadcast.
        client: i32,
        /// Command text.
        text: String,
    },
    /// Any other presentation event (advances the sequence only).
    Other,
}

/// Visible-entity selection for a transport player state.
pub type Q3SourceSelect = Box<dyn FnMut(&Q3PlayerState, &Q3SourcePresentationState) -> Q3VisibleEntities>;
/// Source actor fallback by entity number.
pub type Q3SourceActor = Box<dyn FnMut(i32) -> Option<ActorId>>;
/// Seat-session membership probe.
pub type Q3SameSession = Box<dyn Fn(&ActorId) -> bool>;
/// Private prediction-command adapter.
pub type Q3PredictionCommand = Box<dyn FnMut(&ActorCommand, i32) -> PredictionCommand>;
/// Level-shot hook.
pub type Q3LevelShot = Box<dyn FnMut()>;

/// Options for [`ApplicationQ3Source`].
pub struct ApplicationQ3SourceOptions {
    /// Seat's current actor.
    pub actor: ActorId,
    /// Initial source state.
    pub initial: Q3SourcePresentationState,
    /// Visible-entity selection for a transport player state.
    pub select: Q3SourceSelect,
    /// Source actor fallback by entity number.
    pub source_actor: Q3SourceActor,
    /// Whether an actor shares the seat's session.
    pub same_session: Q3SameSession,
    /// Private prediction-command adapter, if any.
    pub prediction_command: Option<Q3PredictionCommand>,
    /// Level-shot hook, if any.
    pub level_shot: Option<Q3LevelShot>,
}

fn slots(values: &[i32; 16]) -> Q3PlayerSlots {
    let mut slots = Q3PlayerSlots::new();
    for (index, value) in values.iter().enumerate() {
        slots.set(index, *value).expect("player slot index is in range");
    }
    slots
}

fn transport_product(product: Product) -> Q3Product {
    match product {
        Product::Baseq3 => Q3Product::Base,
        Product::Missionpack => Q3Product::MissionPack,
    }
}

/// Copy a source player state into its transport record.
#[must_use]
pub fn transport_player(product: Product, source: &QvmPlayerState) -> Q3PlayerState {
    Q3PlayerState {
        product: transport_product(product),
        command_time: source.command_time_ms,
        pm_type: source.movement_type,
        bob_cycle: source.bob_cycle,
        pm_flags: source.movement_flags,
        pm_time: source.movement_time_ms,
        origin: [source.origin.x, source.origin.y, source.origin.z],
        velocity: [source.velocity.x, source.velocity.y, source.velocity.z],
        weapon_time: source.weapon_time_ms,
        gravity: source.gravity,
        speed: source.speed,
        delta_angles: [
            source.delta_angle_words[0] as f32,
            source.delta_angle_words[1] as f32,
            source.delta_angle_words[2] as f32,
        ],
        ground_entity_num: source.ground_entity_number,
        legs_timer: source.legs_timer_ms,
        legs_anim: source.legs_animation,
        torso_timer: source.torso_timer_ms,
        torso_anim: source.torso_animation,
        movement_dir: source.movement_direction,
        grapple_point: [source.grapple_point.x, source.grapple_point.y, source.grapple_point.z],
        e_flags: source.flags,
        event_sequence: source.event_sequence,
        events: source.events,
        event_parms: source.event_parameters,
        external_event: source.external_event,
        external_event_parm: source.external_event_parameter,
        external_event_time: source.external_event_time_ms,
        client_num: source.client_number,
        weapon: source.weapon,
        weapon_state: source.weapon_state,
        viewangles: [source.view_angles.x, source.view_angles.y, source.view_angles.z],
        viewheight: source.view_height,
        damage_event: source.damage_event,
        damage_yaw: source.damage_yaw,
        damage_pitch: source.damage_pitch,
        damage_count: source.damage_count,
        stats: slots(&source.stats),
        persistant: slots(&source.persistent),
        powerups: slots(&source.powerups),
        ammo: slots(&source.ammo),
        generic1: source.generic1,
        loop_sound: source.loop_sound,
        jumppad_ent: source.jump_pad_entity,
        ping: source.ping_ms,
        pmove_framecount: source.movement_frame_count,
        jumppad_frame: source.jump_pad_frame,
        entity_event_sequence: source.entity_event_sequence,
    }
}

/// One seat's local server transport.
pub struct ApplicationQ3Source {
    current_actor: ActorId,
    select: Q3SourceSelect,
    source_actor: Q3SourceActor,
    same_session: Q3SameSession,
    prediction_command: Option<Q3PredictionCommand>,
    level_shot: Option<Q3LevelShot>,
    /// Predicted command history.
    pub commands: ClientCommandHistory,
    snapshots: HashMap<i32, Snapshot>,
    server_commands: HashMap<i32, Vec<String>>,
    actors: HashMap<i32, ActorId>,
    strings: [String; MAX_CONFIGSTRINGS],
    applied_strings: [String; MAX_CONFIGSTRINGS],
    number: i32,
    reliable: i32,
    sequence: i64,
    command_sequence: Option<u64>,
    level_shot_sequence: i32,
    snapshot_server_bit: i32,
    pending_round_time: Option<i32>,
    product: Product,
    /// Presentation time in milliseconds.
    pub time: i32,
    /// Seat's client slot.
    pub client_number: i32,
}

impl ApplicationQ3Source {
    /// Build a source over an initial presentation state.
    pub fn new(mut options: ApplicationQ3SourceOptions) -> Result<Self, Q3SourceError> {
        let client = options
            .initial
            .clients
            .iter()
            .find(|client| client.actor == options.actor)
            .ok_or(Q3SourceError::NoSourcePlayer)?;
        let client_number = client.slot;
        let product = options.initial.product;
        let mut source = Self {
            current_actor: options.actor,
            select: std::mem::replace(
                &mut options.select,
                Box::new(|_, _| Q3VisibleEntities {
                    area_mask: Vec::new(),
                    entities: Vec::new(),
                }),
            ),
            source_actor: std::mem::replace(&mut options.source_actor, Box::new(|_| None)),
            same_session: std::mem::replace(&mut options.same_session, Box::new(|_| false)),
            prediction_command: options.prediction_command.take(),
            level_shot: options.level_shot.take(),
            commands: ClientCommandHistory::new(),
            snapshots: HashMap::new(),
            server_commands: HashMap::new(),
            actors: HashMap::new(),
            strings: std::array::from_fn(|_| String::new()),
            applied_strings: std::array::from_fn(|_| String::new()),
            number: 0,
            reliable: 0,
            sequence: -1,
            command_sequence: None,
            level_shot_sequence: -1,
            snapshot_server_bit: 0,
            pending_round_time: None,
            product,
            time: 0,
            client_number,
        };
        for entry in &options.initial.configstrings {
            if entry.index >= 0 && (entry.index as usize) < MAX_CONFIGSTRINGS {
                source.strings[entry.index as usize] = entry.value.clone();
                source.applied_strings[entry.index as usize] = entry.value.clone();
            }
        }
        source.receive(&options.initial, &[], &[])?;
        Ok(source)
    }

    /// Seat's current actor.
    #[must_use]
    pub fn actor(&self) -> &ActorId {
        &self.current_actor
    }

    /// Source mode (`live`).
    #[must_use]
    pub fn source_mode(&self) -> &'static str {
        "live"
    }

    /// Reject a round restart while one is pending.
    pub fn assert_can_restart_round(&self) -> Result<(), Q3SourceError> {
        if self.pending_round_time.is_some() {
            return Err(Q3SourceError::RestartPending);
        }
        Ok(())
    }

    /// Begin a round restart on a new server bit.
    pub fn begin_round_restart(&mut self, bit: i32, source: &Q3SourcePresentationState) -> Result<(), Q3SourceError> {
        self.assert_can_restart_round()?;
        if (bit != 0 && bit != 4)
            || bit == self.snapshot_server_bit
            || source.product != self.product
            || source.time < self.time
        {
            return Err(Q3SourceError::InvalidRestartEpoch);
        }
        self.push_configstrings(source);
        self.command(vec!["map_restart".to_string()]);
        self.snapshot_server_bit = bit;
        self.pending_round_time = Some(source.time);
        self.actors.clear();
        Ok(())
    }

    /// Validate a round-restart actor rebinding.
    pub fn validate_round_actor(
        &self,
        actor: &ActorId,
        source: &Q3SourcePresentationState,
    ) -> Result<(), Q3SourceError> {
        let Some(pending) = self.pending_round_time else {
            return Err(Q3SourceError::InvalidRoundActor);
        };
        if !(self.same_session)(actor)
            || *actor == self.current_actor
            || source.product != self.product
            || source.time < pending
            || !source
                .clients
                .iter()
                .any(|client| client.slot == self.client_number && client.actor == *actor)
        {
            return Err(Q3SourceError::InvalidRoundActor);
        }
        Ok(())
    }

    /// Rebind the seat actor after a round restart.
    pub fn rebind_round(&mut self, actor: ActorId, source: &Q3SourcePresentationState) -> Result<(), Q3SourceError> {
        self.validate_round_actor(&actor, source)?;
        self.current_actor = actor;
        self.pending_round_time = None;
        Ok(())
    }

    fn push_configstrings(&mut self, source: &Q3SourcePresentationState) {
        let mut strings = std::array::from_fn::<String, MAX_CONFIGSTRINGS, _>(|_| String::new());
        for entry in &source.configstrings {
            if entry.index >= 0 && (entry.index as usize) < MAX_CONFIGSTRINGS {
                strings[entry.index as usize] = entry.value.clone();
            }
        }
        for (index, value) in strings.into_iter().enumerate() {
            if self.strings[index] == value {
                continue;
            }
            self.strings[index] = value.clone();
            self.command(vec!["cs".to_string(), index.to_string(), value]);
        }
    }

    /// Latest snapshot cursor.
    #[must_use]
    pub fn current(&self) -> SnapshotCurrent {
        SnapshotCurrent {
            number: self.number,
            server_time: self.time,
        }
    }

    /// Read a retained snapshot.
    #[must_use]
    pub fn read(&self, number: i32) -> Option<Snapshot> {
        self.snapshots.get(&number).cloned()
    }

    /// Applied game-state strings.
    #[must_use]
    pub fn game_state(&self) -> Vec<String> {
        self.applied_strings.to_vec()
    }

    /// Read a reliable server command, applying its side effects.
    pub fn get_server_command(&mut self, sequence: i32) -> Result<Option<Vec<String>>, Q3SourceError> {
        let command = self.server_commands.get(&sequence).cloned();
        if command
            .as_ref()
            .is_some_and(|argv| argv.first().is_some_and(|head| head == "clientLevelShot"))
            && sequence > self.level_shot_sequence
        {
            self.level_shot_sequence = sequence;
            if let Some(level_shot) = self.level_shot.as_mut() {
                level_shot();
            }
        }
        if let Some(argv) = command.as_ref() {
            if argv.first().is_some_and(|head| head == "cs") {
                let index = argv.get(1).and_then(|text| text.parse::<i32>().ok());
                let value = argv.get(2).cloned();
                match (index, value) {
                    (Some(index), Some(value)) if index >= 0 && (index as usize) < MAX_CONFIGSTRINGS => {
                        self.applied_strings[index as usize] = value;
                    }
                    _ => return Err(Q3SourceError::BadConfigstringCommand),
                }
            }
        }
        Ok(command)
    }

    /// Resolve the actor behind an entity number.
    pub fn actor_at(&mut self, number: i32) -> Result<ActorId, Q3SourceError> {
        if self.pending_round_time.is_some() {
            return Err(Q3SourceError::ActorBindingSuspended);
        }
        if let Some(actor) = self.actors.get(&number) {
            return Ok(actor.clone());
        }
        (self.source_actor)(number).ok_or(Q3SourceError::NoSourceActor(number))
    }

    fn command(&mut self, argv: Vec<String>) {
        self.reliable += 1;
        self.server_commands.insert(self.reliable, argv);
        self.server_commands.remove(&(self.reliable - COMMAND_BACKUP));
    }

    /// Fold presentation events into reliable commands.
    pub fn receive_events(&mut self, events: &[Q3SourcePresentationEvent]) -> Result<(), Q3SourceError> {
        for event in events {
            if event.sequence <= self.sequence {
                continue;
            }
            self.sequence = event.sequence;
            if let Q3SourceEvent::ServerCommand { client, text } = &event.event {
                if *client < 0 || *client == self.client_number {
                    let argv = tokenize_command(text, Dialect::Q3, TextMode::Source)
                        .map_err(|error| Q3SourceError::BadServerCommand(error.to_string()))?
                        .argv;
                    self.command(argv);
                }
            }
        }
        Ok(())
    }

    /// Publish source state, events, and actor commands.
    pub fn receive(
        &mut self,
        source: &Q3SourcePresentationState,
        events: &[Q3SourcePresentationEvent],
        commands: &[ActorCommand],
    ) -> Result<(), Q3SourceError> {
        if self.pending_round_time.is_some() {
            return Err(Q3SourceError::RestartRequiresRebind);
        }
        if source.time < self.time {
            return Err(Q3SourceError::TimeRewound);
        }
        let player = source
            .clients
            .iter()
            .find(|client| client.actor == self.current_actor)
            .filter(|client| client.slot == self.client_number)
            .ok_or(Q3SourceError::SeatChangedPlayer)?;
        for row in &source.entities {
            self.actors.insert(row.state.number, row.actor.clone());
        }
        for row in &source.clients {
            self.actors.insert(row.slot, row.actor.clone());
        }
        self.push_configstrings(source);
        self.receive_events(events)?;
        for input in commands {
            if input.actor != self.current_actor || self.command_sequence.is_some_and(|last| input.sequence <= last) {
                continue;
            }
            self.command_sequence = Some(input.sequence);
            if let Some(prediction) = self.prediction_command.as_mut() {
                let adapted = prediction(input, source.time);
                self.commands.append(&adapted);
            } else if let ActorUserCommand::Q3 {
                server_time_milliseconds,
                angle_words,
                buttons,
                weapon,
                forward_move,
                right_move,
                up_move,
            } = &input.command
            {
                self.commands.append(&PredictionCommand {
                    server_time: *server_time_milliseconds as i32,
                    angles: qa_core::math::Vec3 {
                        x: angle_words[0] as f32,
                        y: angle_words[1] as f32,
                        z: angle_words[2] as f32,
                    },
                    buttons: *buttons as i32,
                    weapon: *weapon as i32,
                    forwardmove: *forward_move as i32,
                    rightmove: *right_move as i32,
                    upmove: *up_move as i32,
                });
            } else {
                return Err(Q3SourceError::ForeignMovement);
            }
        }
        if self.number > 0 && source.time == self.time {
            return Ok(());
        }
        self.time = source.time;
        let player_state = transport_player(source.product, &player.state);
        let visible = (self.select)(&player_state, source);
        if visible.area_mask.len() > 32 {
            return Err(Q3SourceError::BadAreaMask);
        }
        let mut area_mask = vec![0u8; 32];
        area_mask[..visible.area_mask.len()].copy_from_slice(&visible.area_mask);
        let previous = self.number;
        self.number += 1;
        self.snapshots.insert(
            self.number,
            Snapshot {
                message_number: self.number,
                server_time: source.time,
                delta_number: if previous == 0 { -1 } else { previous },
                flags: self.snapshot_server_bit,
                server_command_number: self.reliable,
                parse_entities_number: 0,
                area_mask,
                player_state,
                entities: visible.entities,
            },
        );
        self.snapshots.remove(&(self.number - SNAPSHOT_BACKUP));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q3::base::shared::entity_state::EntityState;
    use qa_content::q3::presentation::prediction::CommandSource as PredictionCommandSource;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_net::common::commands::CommandSource;

    use crate::bootstrap::simulation::q3::types::{
        Q3SourcePresentationClient, Q3SourcePresentationEntity, Q3SourcePresentationString,
    };

    fn owner() -> IdentityOwner {
        IdentityOwner::create("q3-source-test").expect("owner")
    }

    fn presentation(actor: &ActorId, time: i32) -> Q3SourcePresentationState {
        let entity = EntityState {
            number: 9,
            ..EntityState::default()
        };
        Q3SourcePresentationState {
            product: Product::Baseq3,
            time,
            entities: vec![Q3SourcePresentationEntity {
                actor: actor.clone(),
                state: entity,
                origin: vec3(0.0, 0.0, 0.0),
                linked: true,
                server_flags: 0,
                single_client: -1,
            }],
            clients: vec![Q3SourcePresentationClient {
                actor: actor.clone(),
                slot: 2,
                state: QvmPlayerState::default(),
            }],
            configstrings: vec![Q3SourcePresentationString {
                index: 1,
                value: "sysinfo".to_string(),
            }],
        }
    }

    fn source(actor: &ActorId, initial: Q3SourcePresentationState) -> ApplicationQ3Source {
        ApplicationQ3Source::new(ApplicationQ3SourceOptions {
            actor: actor.clone(),
            initial,
            select: Box::new(|_, _| Q3VisibleEntities {
                area_mask: vec![1, 2],
                entities: Vec::new(),
            }),
            source_actor: Box::new(|_| None),
            same_session: Box::new(|_| true),
            prediction_command: None,
            level_shot: None,
        })
        .expect("source")
    }

    #[test]
    fn constructor_publishes_initial_snapshot() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let source = source(&actor, presentation(&actor, 100));
        assert_eq!(source.client_number, 2);
        assert_eq!(source.time, 100);
        let cursor = source.current();
        assert_eq!((cursor.number, cursor.server_time), (1, 100));
        let snapshot = source.read(1).expect("snapshot");
        assert_eq!(snapshot.delta_number, -1);
        assert_eq!(snapshot.flags, 0);
        assert_eq!(&snapshot.area_mask[..2], &[1, 2]);
        assert_eq!(source.game_state()[1], "sysinfo");
    }

    #[test]
    fn constructor_rejects_missing_player() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let stranger = registry.actor(5, 1);
        let result = ApplicationQ3Source::new(ApplicationQ3SourceOptions {
            actor: stranger,
            initial: presentation(&actor, 100),
            select: Box::new(|_, _| Q3VisibleEntities {
                area_mask: Vec::new(),
                entities: Vec::new(),
            }),
            source_actor: Box::new(|_| None),
            same_session: Box::new(|_| true),
            prediction_command: None,
            level_shot: None,
        });
        assert_eq!(result.err(), Some(Q3SourceError::NoSourcePlayer));
    }

    #[test]
    fn configstring_changes_become_reliable_commands() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let mut source = source(&actor, presentation(&actor, 100));
        let mut next = presentation(&actor, 200);
        next.configstrings.push(Q3SourcePresentationString {
            index: 7,
            value: "seven".to_string(),
        });
        source.receive(&next, &[], &[]).expect("receive");
        let argv = source.get_server_command(1).expect("command").expect("argv");
        assert_eq!(argv, vec!["cs".to_string(), "7".to_string(), "seven".to_string()]);
        assert_eq!(source.game_state()[7], "seven");
        assert_eq!(source.current().number, 2);
    }

    #[test]
    fn server_command_events_filter_by_client() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let mut source = source(&actor, presentation(&actor, 100));
        source
            .receive_events(&[
                Q3SourcePresentationEvent {
                    sequence: 1,
                    event: Q3SourceEvent::ServerCommand {
                        client: 3,
                        text: "print ignored".to_string(),
                    },
                },
                Q3SourcePresentationEvent {
                    sequence: 2,
                    event: Q3SourceEvent::ServerCommand {
                        client: 2,
                        text: "print hello".to_string(),
                    },
                },
                Q3SourcePresentationEvent {
                    sequence: 2,
                    event: Q3SourceEvent::Other,
                },
            ])
            .expect("events");
        let argv = source.get_server_command(1).expect("command").expect("argv");
        assert_eq!(argv, vec!["print".to_string(), "hello".to_string()]);
        assert!(source.get_server_command(2).expect("command").is_none());
    }

    #[test]
    fn q3_actor_commands_append_predictions() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let seat = registry.seat(0);
        let mut source = source(&actor, presentation(&actor, 100));
        let next = presentation(&actor, 200);
        source
            .receive(
                &next,
                &[],
                &[ActorCommand {
                    actor: actor.clone(),
                    source: CommandSource::LocalSeat { seat: seat.clone() },
                    sequence: 4,
                    command: ActorUserCommand::Q3 {
                        server_time_milliseconds: 200.0,
                        angle_words: [1.0, 2.0, 3.0],
                        buttons: 5.0,
                        weapon: 6.0,
                        forward_move: 7.0,
                        right_move: 8.0,
                        up_move: 9.0,
                    },
                    arsenal: None,
                }],
            )
            .expect("receive");
        assert_eq!(source.commands.current_number(), 1);
        // Replays and foreign actors are skipped.
        source.receive(&presentation(&actor, 300), &[], &[]).expect("receive");
        assert_eq!(source.commands.current_number(), 1);
    }

    #[test]
    fn foreign_movement_without_adapter_fails() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let seat = registry.seat(0);
        let mut source = source(&actor, presentation(&actor, 100));
        let result = source.receive(
            &presentation(&actor, 200),
            &[],
            &[ActorCommand {
                actor: actor.clone(),
                source: CommandSource::LocalSeat { seat },
                sequence: 9,
                command: ActorUserCommand::Q2Classic {
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
            }],
        );
        assert_eq!(result.unwrap_err(), Q3SourceError::ForeignMovement);
    }

    #[test]
    fn round_restart_rebinds_actor() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let mut source = source(&actor, presentation(&actor, 100));
        let restart = presentation(&actor, 200);
        source.begin_round_restart(4, &restart).expect("restart");
        assert_eq!(
            source.begin_round_restart(0, &restart).unwrap_err(),
            Q3SourceError::RestartPending
        );
        assert_eq!(source.actor_at(9).unwrap_err(), Q3SourceError::ActorBindingSuspended);
        let rebound = registry.actor(7, 1);
        let mut rebound_state = presentation(&rebound, 200);
        rebound_state.clients[0].slot = 2;
        source.rebind_round(rebound.clone(), &rebound_state).expect("rebind");
        assert_eq!(source.actor(), &rebound);
        source.receive(&rebound_state, &[], &[]).expect("receive");
        assert_eq!(source.read(2).expect("snapshot").flags, 4);
    }

    #[test]
    fn round_restart_validates_epoch_and_actor() {
        let registry = owner();
        let actor = registry.actor(2, 1);
        let mut source = source(&actor, presentation(&actor, 100));
        let stale = presentation(&actor, 50);
        assert_eq!(
            source.begin_round_restart(4, &stale).unwrap_err(),
            Q3SourceError::InvalidRestartEpoch
        );
        assert_eq!(
            source.begin_round_restart(0, &presentation(&actor, 200)).unwrap_err(),
            Q3SourceError::InvalidRestartEpoch
        );
        let restart = presentation(&actor, 200);
        source.begin_round_restart(4, &restart).expect("restart");
        assert_eq!(
            source.validate_round_actor(&actor, &restart).unwrap_err(),
            Q3SourceError::InvalidRoundActor
        );
    }

    #[test]
    fn transport_player_copies_fields_and_slots() {
        let mut stats = [0; 16];
        stats[3] = 11;
        let mut ammo = [0; 16];
        ammo[15] = 22;
        let state = QvmPlayerState {
            movement_type: 4,
            origin: vec3(1.0, 2.0, 3.0),
            stats,
            ammo,
            ..QvmPlayerState::default()
        };
        let record = transport_player(Product::Missionpack, &state);
        assert_eq!(record.product, Q3Product::MissionPack);
        assert_eq!(record.pm_type, 4);
        assert_eq!(record.origin, [1.0, 2.0, 3.0]);
        assert_eq!(record.stats.get(3).expect("slot"), 11);
        assert_eq!(record.ammo.get(15).expect("slot"), 22);
    }
}
