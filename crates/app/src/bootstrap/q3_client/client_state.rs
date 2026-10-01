//! Local Quake III client state for a seat without a network channel.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/client-state.ts`
//! (`LocalQ3ClientState`). A local seat receives authoritative server
//! records directly: reliable commands advance without gaps, snapshots
//! advance message numbers, and server commands execute through
//! [`Q3ServerCommandExecutor`]. [`LocalQ3ClientState::read`] mirrors the
//! donor's `HistorySnapshotSource.read` window, validity, and
//! entity-truncation rules; the parse-entity staleness check is vacuous
//! because the Rust history retains full snapshots inline.

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_net::q3::WireUserCommand;
use qa_net::q3_net::{
    ClientGameStateStorage, Gamestate, GamestateEntry, Q3CommandHistory, Q3NetError, Q3ServerCommandBindings,
    Q3ServerCommandExecutor, ServerOperation, Snapshot, SnapshotHistory, SnapshotStatus, SnapshotValidity,
    SourceParseEntities,
};
use thiserror::Error;

/// Retained reliable commands.
const COMMAND_BACKUP: i32 = 64;
/// Retained snapshot pings.
const PING_BACKUP: i32 = 32;
/// Snapshot read window.
const SNAPSHOT_WINDOW: i32 = 32;
/// Maximum entities returned by a snapshot read.
const MAX_SNAPSHOT_ENTITIES: usize = 256;

/// Local client failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q3ClientStateError {
    /// Local Q3 client belongs to a retired gamestate.
    #[error("Local Q3 client belongs to a retired gamestate")]
    Retired,
    /// Reliable command sequence must advance without gaps.
    #[error("Local Q3 reliable command sequence must advance without gaps")]
    ReliableGap,
    /// Snapshot sequence must advance.
    #[error("Local Q3 snapshot sequence must advance")]
    SnapshotNotAdvancing,
    /// Snapshot must follow its reliable commands.
    #[error("Local Q3 snapshot must follow its reliable commands")]
    SnapshotBeforeCommands,
    /// Snapshot number is newer than the latest snapshot.
    #[error("CL_GetSnapshot: snapshotNumber > cl.snapshot.messageNum")]
    SnapshotTooNew,
    /// Stored snapshot area mask exceeds 32 bytes.
    #[error("Local Q3 snapshot area mask exceeds 32 bytes")]
    BadAreaMask,
    /// Authoritative drop from the donor (`CommonError("drop", ...)`).
    #[error("{0}")]
    Drop(String),
    /// Network layer failure.
    #[error("{0}")]
    Net(String),
}

impl From<Q3NetError> for Q3ClientStateError {
    fn from(error: Q3NetError) -> Self {
        Self::Net(error.to_string())
    }
}

/// Seat bindings for the local client.
pub trait LocalQ3ClientBindings {
    /// Panic when the seat moved past this client.
    fn assert_current(&mut self);
    /// Actor behind a client slot.
    fn actor_at(&mut self, number: i32) -> ActorId;
    /// Apply a system-info string.
    fn system_info(&mut self, info: &str);
    /// Restart the map.
    fn map_restart(&mut self);
    /// Take a level shot.
    fn level_shot(&mut self);
    /// Print a diagnostic line.
    fn print(&mut self, text: &str);
}

/// Executor host borrowing the client's game state, commands, and bindings.
struct ExecutorHost<'a, B> {
    game_state: &'a mut ClientGameStateStorage,
    commands: &'a mut Q3CommandHistory,
    bindings: &'a mut B,
    retired: bool,
}

impl<B: LocalQ3ClientBindings> Q3ServerCommandBindings for ExecutorHost<'_, B> {
    fn assert_current(&mut self) {
        if self.retired {
            panic!("Local Q3 client belongs to a retired gamestate");
        }
        self.bindings.assert_current();
    }

    fn system_info(&mut self) -> Result<(), Q3NetError> {
        let info = self.game_state.get(1)?.unwrap_or_default();
        self.bindings.system_info(&info);
        Ok(())
    }

    fn map_restart(&mut self) {
        self.commands.restart();
        self.bindings.map_restart();
    }

    fn local_server_running(&self) -> bool {
        true
    }

    fn level_shot(&mut self) {
        self.bindings.level_shot();
    }

    fn game_state(&mut self) -> &mut ClientGameStateStorage {
        self.game_state
    }
}

/// A local seat's authoritative client records.
pub struct LocalQ3ClientState<B> {
    bindings: B,
    game_state: ClientGameStateStorage,
    commands: Q3CommandHistory,
    parse_entities: SourceParseEntities,
    history: SnapshotHistory<'static>,
    server_commands: HashMap<i32, String>,
    pings: HashMap<i32, i32>,
    executor: Q3ServerCommandExecutor,
    retired: bool,
    /// Seat's client slot.
    pub client_number: i32,
    /// Gamestate generation, bumped by retirement.
    pub generation: i32,
    /// Latest server message number.
    pub server_message_sequence: i32,
    /// Latest reliable server-command sequence.
    pub server_command_sequence: i32,
    /// Last executed server command.
    pub last_executed_server_command: i32,
}

impl<B: LocalQ3ClientBindings> LocalQ3ClientState<B> {
    /// Build a client over an initial gamestate.
    pub fn new(initial: &Gamestate, bindings: B) -> Result<Self, Q3ClientStateError> {
        let mut game_state = ClientGameStateStorage::new();
        game_state.begin_entries();
        for entry in &initial.entries {
            if let GamestateEntry::Configstring { index, value } = entry {
                game_state.append(*index as usize, value)?;
            }
        }
        Ok(Self {
            bindings,
            game_state,
            commands: Q3CommandHistory::new(),
            parse_entities: SourceParseEntities::new(),
            history: SnapshotHistory::new(None),
            server_commands: HashMap::new(),
            pings: HashMap::new(),
            executor: Q3ServerCommandExecutor::new(),
            retired: false,
            client_number: initial.client_number,
            generation: 1,
            server_message_sequence: 0,
            server_command_sequence: initial.command_sequence,
            last_executed_server_command: initial.command_sequence,
        })
    }

    /// Source mode (`live`).
    #[must_use]
    pub fn source_mode(&self) -> &'static str {
        "live"
    }

    /// Game-state storage.
    #[must_use]
    pub fn game_state(&self) -> &ClientGameStateStorage {
        &self.game_state
    }

    /// Predicted command history.
    #[must_use]
    pub fn commands(&self) -> &Q3CommandHistory {
        &self.commands
    }

    /// Seat bindings.
    pub fn bindings_mut(&mut self) -> &mut B {
        &mut self.bindings
    }

    fn assert_current(&mut self) -> Result<(), Q3ClientStateError> {
        if self.retired {
            return Err(Q3ClientStateError::Retired);
        }
        self.bindings.assert_current();
        Ok(())
    }

    /// Current server time.
    pub fn time(&mut self) -> Result<i32, Q3ClientStateError> {
        self.assert_current()?;
        Ok(self.history.latest().map_or(0, |latest| latest.server_time))
    }

    /// Latest snapshot cursor.
    pub fn current(&mut self) -> Result<(i32, i32), Q3ClientStateError> {
        self.assert_current()?;
        Ok(self
            .history
            .latest()
            .map_or((0, 0), |latest| (latest.message_number, latest.server_time)))
    }

    /// Read a retained snapshot.
    pub fn read(&mut self, number: i32) -> Result<Option<Snapshot>, Q3ClientStateError> {
        self.assert_current()?;
        let latest = self.history.latest();
        let latest_number = latest.as_ref().map_or(0, |latest| latest.message_number);
        if number > latest_number {
            return Err(Q3ClientStateError::SnapshotTooNew);
        }
        if latest_number.wrapping_sub(number) >= SNAPSHOT_WINDOW || latest.is_none() {
            return Ok(None);
        }
        let entry = self.history.read_slot(number)?;
        let Some(entry) = entry else {
            return Ok(None);
        };
        if entry.status != SnapshotStatus::Valid {
            return Ok(None);
        }
        let snapshot = entry.snapshot;
        if snapshot.area_mask.len() > 32 {
            return Err(Q3ClientStateError::BadAreaMask);
        }
        let mut area_mask = vec![0u8; 32];
        area_mask[..snapshot.area_mask.len()].copy_from_slice(&snapshot.area_mask);
        let mut entities = snapshot.entities.clone();
        if entities.len() > MAX_SNAPSHOT_ENTITIES {
            self.bindings.print(&format!(
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

    /// Copied game-state strings.
    pub fn game_state_strings(&mut self) -> Result<Vec<String>, Q3ClientStateError> {
        self.assert_current()?;
        Ok(self.game_state.copy_strings())
    }

    /// System-info string.
    pub fn system_info_string(&mut self) -> Result<String, Q3ClientStateError> {
        self.assert_current()?;
        Ok(self.game_state.get(1)?.unwrap_or_default())
    }

    /// Current outgoing user-command number.
    pub fn commands_current_number(&mut self) -> Result<i32, Q3ClientStateError> {
        self.assert_current()?;
        Ok(self.commands.current_number())
    }

    /// Read a retained user command.
    pub fn commands_read(&mut self, number: i32) -> Result<Option<WireUserCommand>, Q3ClientStateError> {
        self.assert_current()?;
        Ok(self.commands.read(number)?)
    }

    /// Actor behind a client slot.
    pub fn actor_at(&mut self, number: i32) -> Result<ActorId, Q3ClientStateError> {
        self.assert_current()?;
        Ok(self.bindings.actor_at(number))
    }

    /// Receive one reliable server command.
    pub fn receive_server_command(&mut self, sequence: i32, text: &str) -> Result<(), Q3ClientStateError> {
        self.assert_current()?;
        if sequence != self.server_command_sequence + 1 {
            return Err(Q3ClientStateError::ReliableGap);
        }
        self.server_command_sequence = sequence;
        self.server_commands.insert(sequence, text.chars().take(1023).collect());
        self.server_commands.remove(&(sequence - COMMAND_BACKUP));
        Ok(())
    }

    /// Receive one snapshot with its ping.
    pub fn receive_snapshot(&mut self, snapshot: &Snapshot, ping: i32) -> Result<(), Q3ClientStateError> {
        self.assert_current()?;
        if snapshot.message_number <= self.server_message_sequence {
            return Err(Q3ClientStateError::SnapshotNotAdvancing);
        }
        if snapshot.server_command_number != self.server_command_sequence {
            return Err(Q3ClientStateError::SnapshotBeforeCommands);
        }
        let start = self.parse_entities.number;
        for entity in &snapshot.entities {
            *self.parse_entities.at(self.parse_entities.number) = entity.clone();
            self.parse_entities.advance();
        }
        let mut stored = snapshot.clone();
        stored.parse_entities_number = start;
        self.history.publish(&ServerOperation::Snapshot {
            validity: SnapshotValidity::Valid,
            snapshot: Box::new(stored),
        })?;
        self.server_message_sequence = snapshot.message_number;
        self.pings
            .retain(|number, _| snapshot.message_number.wrapping_sub(*number) < PING_BACKUP);
        self.pings.insert(snapshot.message_number, ping);
        Ok(())
    }

    /// Ping recorded for a retained snapshot.
    pub fn snapshot_ping(&mut self, number: i32) -> Result<Option<i32>, Q3ClientStateError> {
        self.assert_current()?;
        if self.read(number)?.is_none() {
            return Ok(None);
        }
        Ok(self.pings.get(&number).copied())
    }

    /// Execute a reliable server command, returning its argv.
    pub fn get_server_command(&mut self, sequence: i32) -> Result<Option<Vec<String>>, Q3ClientStateError> {
        self.assert_current()?;
        if sequence <= self.server_command_sequence - COMMAND_BACKUP {
            return Err(Q3ClientStateError::Drop(
                "CL_GetServerCommand: a reliable command was cycled out".to_string(),
            ));
        }
        if sequence > self.server_command_sequence {
            return Err(Q3ClientStateError::Drop(
                "CL_GetServerCommand: requested a command not received".to_string(),
            ));
        }
        let command = self.server_commands.get(&sequence).cloned().ok_or_else(|| {
            Q3ClientStateError::Drop("CL_GetServerCommand: command predates local gamestate".to_string())
        })?;
        self.last_executed_server_command = sequence;
        let Self {
            executor,
            game_state,
            commands,
            bindings,
            retired,
            ..
        } = self;
        let mut host = ExecutorHost {
            game_state,
            commands,
            bindings,
            retired: *retired,
        };
        executor
            .execute(&command, &mut host)
            .map_err(|error| Q3ClientStateError::Drop(error.to_string()))
    }

    /// Retire the client, clearing every ring.
    pub fn retire(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        self.generation += 1;
        self.history.clear();
        self.parse_entities.clear();
        self.commands.clear();
        self.game_state.clear();
        self.server_commands.clear();
        self.pings.clear();
        self.executor.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_net::q3_net::{Q3EntityState, Q3PlayerState, Q3Product};

    struct StubBindings {
        owner: IdentityOwner,
        system_info: Vec<String>,
        map_restarts: usize,
        level_shots: usize,
        printed: Vec<String>,
        current: bool,
    }

    impl StubBindings {
        fn new() -> Self {
            Self {
                owner: IdentityOwner::create("q3-client-state-test").expect("owner"),
                system_info: Vec::new(),
                map_restarts: 0,
                level_shots: 0,
                printed: Vec::new(),
                current: true,
            }
        }
    }

    impl LocalQ3ClientBindings for StubBindings {
        fn assert_current(&mut self) {
            assert!(self.current, "seat moved on");
        }

        fn actor_at(&mut self, number: i32) -> ActorId {
            self.owner.actor(number as u32, 1)
        }

        fn system_info(&mut self, info: &str) {
            self.system_info.push(info.to_string());
        }

        fn map_restart(&mut self) {
            self.map_restarts += 1;
        }

        fn level_shot(&mut self) {
            self.level_shots += 1;
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }
    }

    fn gamestate() -> Gamestate {
        Gamestate {
            command_sequence: 5,
            entries: vec![
                GamestateEntry::Configstring {
                    index: 0,
                    value: String::new(),
                },
                GamestateEntry::Configstring {
                    index: 1,
                    value: "system".to_string(),
                },
            ],
            client_number: 2,
            checksum_feed: 0,
        }
    }

    fn snapshot(message: i32, server_command: i32) -> Snapshot {
        Snapshot {
            message_number: message,
            server_time: 100 + message,
            delta_number: message - 1,
            flags: 0,
            server_command_number: server_command,
            parse_entities_number: 0,
            area_mask: vec![1, 2],
            player_state: Q3PlayerState::new(Q3Product::Base),
            entities: Vec::new(),
        }
    }

    #[test]
    fn constructor_loads_configstrings() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        assert_eq!(client.client_number, 2);
        assert_eq!(client.server_command_sequence, 5);
        assert_eq!(client.last_executed_server_command, 5);
        assert_eq!(client.system_info_string().expect("sysinfo"), "system");
        assert_eq!(client.time().expect("time"), 0);
        assert_eq!(client.current().expect("cursor"), (0, 0));
    }

    #[test]
    fn reliable_commands_advance_without_gaps() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        client.receive_server_command(6, "print hello").expect("command");
        assert_eq!(
            client.receive_server_command(8, "print skip").unwrap_err(),
            Q3ClientStateError::ReliableGap
        );
        let argv = client.get_server_command(6).expect("argv").expect("argv");
        assert_eq!(argv, vec!["print".to_string(), "hello".to_string()]);
        assert_eq!(client.last_executed_server_command, 6);
    }

    #[test]
    fn server_command_window_reports_drops() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        assert!(matches!(
            client.get_server_command(99).unwrap_err(),
            Q3ClientStateError::Drop(_)
        ));
        for sequence in 6..=6 + COMMAND_BACKUP {
            client.receive_server_command(sequence, "print x").expect("command");
        }
        assert!(matches!(
            client.get_server_command(6).unwrap_err(),
            Q3ClientStateError::Drop(_)
        ));
    }

    #[test]
    fn configstring_commands_update_game_state() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        client.receive_server_command(6, "cs 1 newsystem").expect("command");
        client.get_server_command(6).expect("argv");
        assert_eq!(client.system_info_string().expect("sysinfo"), "newsystem");
        assert_eq!(client.bindings_mut().system_info, vec!["newsystem".to_string()]);
        client.receive_server_command(7, "map_restart").expect("command");
        client.get_server_command(7).expect("argv");
        assert_eq!(client.bindings_mut().map_restarts, 1);
    }

    #[test]
    fn snapshots_advance_and_read_back() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        let first = snapshot(1, 5);
        client.receive_snapshot(&first, 42).expect("snapshot");
        assert_eq!(client.time().expect("time"), 101);
        assert_eq!(client.snapshot_ping(1).expect("ping"), Some(42));
        let read = client.read(1).expect("read").expect("snapshot");
        assert_eq!(read.server_time, 101);
        assert_eq!(read.area_mask.len(), 32);
        assert_eq!(
            client.receive_snapshot(&first, 0).unwrap_err(),
            Q3ClientStateError::SnapshotNotAdvancing
        );
        let ahead = snapshot(2, 99);
        assert_eq!(
            client.receive_snapshot(&ahead, 0).unwrap_err(),
            Q3ClientStateError::SnapshotBeforeCommands
        );
        assert_eq!(client.read(9).unwrap_err(), Q3ClientStateError::SnapshotTooNew);
    }

    #[test]
    fn snapshot_entities_truncate_with_print() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        let mut crowded = snapshot(1, 5);
        crowded.entities = vec![Q3EntityState::default(); 300];
        client.receive_snapshot(&crowded, 0).expect("snapshot");
        let read = client.read(1).expect("read").expect("snapshot");
        assert_eq!(read.entities.len(), MAX_SNAPSHOT_ENTITIES);
        assert_eq!(client.bindings_mut().printed.len(), 1);
    }

    #[test]
    fn retire_clears_everything() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        client.receive_server_command(6, "print x").expect("command");
        client.receive_snapshot(&snapshot(1, 6), 0).expect("snapshot");
        client.retire();
        assert_eq!(client.generation, 2);
        assert_eq!(client.time().unwrap_err(), Q3ClientStateError::Retired);
        assert_eq!(client.read(1).unwrap_err(), Q3ClientStateError::Retired);
        client.retire();
        assert_eq!(client.generation, 2);
    }

    #[test]
    fn actor_binding_reaches_seat() {
        let mut client = LocalQ3ClientState::new(&gamestate(), StubBindings::new()).expect("client");
        let actor = client.actor_at(3).expect("actor");
        assert_eq!(actor.slot(), 3);
    }
}
