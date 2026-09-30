//! Quake III presentation: snapshots.
//!
//! Donor provenance: `src/content/q3/presentation/snapshots.ts`.

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::prediction::*;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Snapshots (snapshots.ts)
// ---------------------------------------------------------------------------

/// Latest snapshot cursor (`SnapshotSource.current`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotCurrent {
    /// Message number.
    pub number: i32,
    /// Server time.
    pub server_time: i32,
}

/// Snapshot transport (`SnapshotSource`).
pub trait SnapshotSource {
    /// Latest snapshot cursor.
    fn current(&self) -> SnapshotCurrent;
    /// Read a snapshot by number.
    fn read(&mut self, number: i32) -> PresentResult<Option<Snapshot>>;
}

/// Latest history entry view.
#[derive(Debug, Clone, Copy)]
pub struct HistoryLatest {
    /// Message number.
    pub message_number: i32,
    /// Server time.
    pub server_time: i32,
    /// Player-state product.
    pub product: Q3Product,
}

/// Snapshot-history view for `CL_GetSnapshot` (`SnapshotHistory` mirror).
pub trait SnapshotHistoryView {
    /// Latest entry, if any.
    fn latest(&self) -> Option<HistoryLatest>;
    /// Valid slot snapshot, if the slot is valid.
    fn borrow_slot(&self, number: i32, product: Q3Product) -> Option<Snapshot>;
    /// Retained parse-entity ring base, if retained.
    fn retained_parse_entities_number(&self) -> Option<i32>;
    /// Retained parse entity by absolute index.
    fn retained_entity_at(&self, absolute: i32) -> Option<EntityState>;
}

/// `CL_GetSnapshot` adapter over snapshot history (`HistorySnapshotSource`).
pub struct HistorySnapshotSource<H, F, G> {
    /// History.
    pub history: H,
    parse_entities_number: F,
    debug_print: G,
}

impl<H, F, G> HistorySnapshotSource<H, F, G> {
    /// New adapter.
    pub fn new(history: H, parse_entities_number: F, debug_print: G) -> Self {
        Self {
            history,
            parse_entities_number,
            debug_print,
        }
    }
}

impl<H, F, G> SnapshotSource for HistorySnapshotSource<H, F, G>
where
    H: SnapshotHistoryView,
    F: FnMut() -> i32,
    G: FnMut(String),
{
    fn current(&self) -> SnapshotCurrent {
        self.history.latest().map_or(
            SnapshotCurrent {
                number: 0,
                server_time: 0,
            },
            |latest| SnapshotCurrent {
                number: latest.message_number,
                server_time: latest.server_time,
            },
        )
    }

    fn read(&mut self, number: i32) -> PresentResult<Option<Snapshot>> {
        let latest = self.history.latest();
        let latest_number = latest.map_or(0, |entry| entry.message_number);
        if number > latest_number {
            return Err(drop_msg("CL_GetSnapshot: snapshotNumber > cl.snapshot.messageNum"));
        }
        if latest_number.wrapping_sub(number) >= SNAPSHOT_HISTORY_WINDOW || latest.is_none() {
            return Ok(None);
        }
        let latest = latest.expect("checked latest snapshot");
        let Some(snapshot) = self.history.borrow_slot(number, latest.product) else {
            return Ok(None);
        };
        let parse_entities_number = self
            .history
            .retained_parse_entities_number()
            .unwrap_or_else(|| (self.parse_entities_number)());
        if parse_entities_number.wrapping_sub(snapshot.parse_entities_number) >= MAX_PARSE_ENTITIES {
            return Ok(None);
        }
        let retained = self.history.retained_parse_entities_number().is_some();
        let mut count = snapshot.entities.len();
        if count > 256 {
            (self.debug_print)(format!("CL_GetSnapshot: truncated {count} entities to 256\n"));
            count = 256;
        }
        let mut entities = Vec::with_capacity(count);
        if retained {
            for index in 0..count as i32 {
                let Some(entity) = self
                    .history
                    .retained_entity_at(snapshot.parse_entities_number.wrapping_add(index))
                else {
                    return Ok(None);
                };
                entities.push(entity);
            }
        } else {
            entities.extend(snapshot.entities.iter().take(count).cloned());
        }
        Ok(Some(Snapshot {
            message_number: snapshot.message_number,
            server_time: snapshot.server_time,
            delta_number: snapshot.delta_number,
            flags: snapshot.flags,
            server_command_number: snapshot.server_command_number,
            parse_entities_number: snapshot.parse_entities_number,
            area_mask: snapshot.area_mask,
            player_state: snapshot.player_state.clone(),
            entities,
        }))
    }
}

/// Snapshot runtime services (`SnapshotHost`).
pub trait SnapshotHost {
    /// Latest snapshot cursor.
    fn source_current(&mut self) -> SnapshotCurrent;
    /// Read a snapshot by number.
    fn source_read(&mut self, number: i32) -> PresentResult<Option<Snapshot>>;
    /// Demo playback.
    fn demo_playback(&self) -> bool;
    /// Prediction disabled.
    fn no_predict(&self) -> bool;
    /// Synchronous clients.
    fn synchronous_clients(&self) -> bool;
    /// Execute server commands through a sequence.
    fn execute_server_commands(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        sequence: i32,
    ) -> PresentResult<()>;
    /// Respawn presentation.
    fn respawn(&mut self, state: &mut ClientGameState) -> PresentResult<()>;
    /// Reset a player entity.
    fn reset_player_entity(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_number: usize,
    );
    /// Check an entity's events.
    fn check_events(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_number: usize,
    ) -> PresentResult<()>;
    /// Transition player state.
    fn transition_player_state(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        current: &PlayerState,
        previous: &mut PlayerState,
    ) -> PresentResult<()>;
    /// Lagometer snapshot hook.
    fn lagometer_snapshot(&mut self, snapshot: Option<&Snapshot>);
    /// Warning print.
    fn warn(&mut self, message: &str);
}

/// Snapshot transitions (`SnapshotRuntime`).
pub struct SnapshotRuntime<H> {
    /// Host services.
    pub host: H,
    transitioning: bool,
}

impl<H: SnapshotHost> SnapshotRuntime<H> {
    /// New runtime.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self {
            host,
            transitioning: false,
        }
    }

    fn exclusive(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        operation: SnapshotOperation,
        input: Option<Snapshot>,
    ) -> PresentResult<()> {
        if self.transitioning {
            return Err(state_msg("Snapshot transition already in progress"));
        }
        self.transitioning = true;
        let result = match operation {
            SnapshotOperation::Initial => {
                let input = input.expect("initial snapshot input");
                self.initial_snapshot(state, static_state, input)
            }
            SnapshotOperation::Transition => self.transition(state, static_state),
            SnapshotOperation::Process => self.process(state, static_state),
        };
        self.transitioning = false;
        result
    }

    /// Set the initial snapshot.
    pub fn set_initial_snapshot(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        input: Snapshot,
    ) -> PresentResult<()> {
        self.exclusive(state, static_state, SnapshotOperation::Initial, Some(input))
    }

    /// Transition to the next snapshot.
    pub fn transition_snapshot(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        self.exclusive(state, static_state, SnapshotOperation::Transition, None)
    }

    /// Process pending snapshots.
    pub fn process_snapshots(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        self.exclusive(state, static_state, SnapshotOperation::Process, None)
    }

    /// Set the next snapshot (synchronous, never reentrant).
    pub fn set_next_snapshot(&mut self, state: &mut ClientGameState, input: Snapshot) -> PresentResult<()> {
        if self.transitioning {
            return Err(state_msg("Snapshot transition already in progress"));
        }
        self.next_snapshot(state, input)
    }

    fn reset_entity(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_number: usize,
        server_time: i32,
    ) -> PresentResult<()> {
        let number = i32::try_from(entity_number).map_err(|_| range_msg("Entity number outside int32"))?;
        let time = state.time;
        let entity = state.entity_at_mut(number)?;
        if entity.snapshot_time < time.wrapping_sub(300) {
            entity.previous_event = 0;
        }
        entity.trail_time = server_time;
        entity.lerp_origin = entity.current_state.origin;
        entity.lerp_angles = entity.current_state.angles;
        let is_player = entity.current_state.e_type == EntityType::Player as i32;
        if is_player {
            self.host.reset_player_entity(state, static_state, entity_number);
        }
        Ok(())
    }

    fn initial_snapshot(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        input: Snapshot,
    ) -> PresentResult<()> {
        let mut snapshot = retail_snapshot(&input);
        let client_num = snapshot.player_state.client_num;
        player_state_to_entity_state(
            &mut snapshot.player_state,
            &mut state.entity_at_mut(client_num)?.current_state,
            false,
        )?;
        let server_time = snapshot.server_time;
        let numbers: Vec<i32> = snapshot.entities.iter().map(|entry| entry.number).collect();
        state.snap = Some(snapshot);
        build_solid_list(state)?;
        let command_number = state.snap.as_ref().expect("installed snapshot").server_command_number;
        self.host.execute_server_commands(state, static_state, command_number)?;
        self.host.respawn(state)?;
        for (index, number) in numbers.iter().enumerate() {
            let entry = state
                .snap
                .as_ref()
                .expect("installed snapshot")
                .entities
                .get(index)
                .cloned()
                .ok_or_else(|| state_msg("Snapshot entity vanished during install"))?;
            let entity = state.entity_at_mut(*number)?;
            entity.current_state = entry;
            entity.interpolate = false;
            entity.current_valid = true;
            let entity_number =
                usize::try_from(*number).map_err(|_| range_msg(format!("Invalid cgame entity number {number}")))?;
            self.reset_entity(state, static_state, entity_number, server_time)?;
            self.host.check_events(state, static_state, entity_number)?;
        }
        Ok(())
    }

    fn next_snapshot(&mut self, state: &mut ClientGameState, input: Snapshot) -> PresentResult<()> {
        let previous = state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("CG_SetNextSnap requires cg.snap"))?;
        let previous_client = previous.player_state.client_num;
        let previous_flags = previous.player_state.e_flags;
        let previous_snapshot_flags = previous.flags;
        let mut snapshot = retail_snapshot(&input);
        let client_num = snapshot.player_state.client_num;
        player_state_to_entity_state(
            &mut snapshot.player_state,
            &mut state.entity_at_mut(client_num)?.next_state,
            false,
        )?;
        state.entity_at_mut(previous_client)?.interpolate = true;
        for entry in snapshot.entities.clone() {
            let entity = state.entity_at_mut(entry.number)?;
            let interpolate = entity.current_valid && ((entity.current_state.e_flags ^ entry.e_flags) & 4) == 0;
            entity.next_state = entry;
            entity.interpolate = interpolate;
        }
        state.next_frame_teleport = ((snapshot.player_state.e_flags ^ previous_flags) & 4) != 0
            || snapshot.player_state.client_num != previous_client
            || ((snapshot.flags ^ previous_snapshot_flags) & 4) != 0;
        state.next_snap = Some(snapshot);
        build_solid_list(state)?;
        Ok(())
    }

    fn transition(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
    ) -> PresentResult<()> {
        let Some(mut previous) = state.snap.take() else {
            return Err(drop_msg("CG_TransitionSnapshot: NULL cg.snap"));
        };
        let Some(mut next) = state.next_snap.take() else {
            state.snap = Some(previous);
            return Err(drop_msg("CG_TransitionSnapshot: NULL cg.nextSnap"));
        };
        self.host
            .execute_server_commands(state, static_state, next.server_command_number)?;
        for entry in &previous.entities {
            state.entity_at_mut(entry.number)?.current_valid = false;
        }
        let local_num = next.player_state.client_num;
        player_state_to_entity_state(
            &mut next.player_state,
            &mut state.entity_at_mut(local_num)?.current_state,
            false,
        )?;
        state.entity_at_mut(local_num)?.interpolate = false;
        let next_server_time = next.server_time;
        let numbers: Vec<i32> = next.entities.iter().map(|entry| entry.number).collect();
        for number in numbers {
            let time = state.time;
            let entity = state.entity_at_mut(number)?;
            entity.current_state = entity.next_state.clone();
            entity.current_valid = true;
            let interpolate = entity.interpolate;
            let server_time = next_server_time;
            let entity_number =
                usize::try_from(number).map_err(|_| range_msg(format!("Invalid cgame entity number {number}")))?;
            if !interpolate {
                let trail_ok = entity.snapshot_time >= time.wrapping_sub(300);
                entity.trail_time = server_time;
                entity.lerp_origin = entity.current_state.origin;
                entity.lerp_angles = entity.current_state.angles;
                if !trail_ok {
                    entity.previous_event = 0;
                }
                if entity.current_state.e_type == EntityType::Player as i32 {
                    self.host.reset_player_entity(state, static_state, entity_number);
                }
            }
            state.entity_at_mut(number)?.interpolate = false;
            self.host.check_events(state, static_state, entity_number)?;
            state.entity_at_mut(number)?.snapshot_time = next_server_time;
        }
        if ((next.player_state.e_flags ^ previous.player_state.e_flags) & 4) != 0 {
            state.this_frame_teleport = true;
        }
        let follow = (next.player_state.pm_flags & MoveFlags::FOLLOW) != 0;
        state.snap = Some(next);
        state.next_snap = None;
        if self.host.demo_playback() || follow || self.host.no_predict() || self.host.synchronous_clients() {
            let current = state.snap.as_ref().expect("installed snapshot").player_state.clone();
            self.host
                .transition_player_state(state, static_state, &current, &mut previous.player_state)?;
        }
        Ok(())
    }

    fn read_next_snapshot(&mut self, state: &mut ClientGameState) -> PresentResult<Option<Snapshot>> {
        if state.latest_snapshot_num > state.processed_snapshot_num.wrapping_add(1000) {
            self.host.warn(&format!(
                "WARNING: CG_ReadNextSnapshot: way out of range, {} > {}",
                state.latest_snapshot_num, state.processed_snapshot_num
            ));
        }
        while state.processed_snapshot_num < state.latest_snapshot_num {
            state.processed_snapshot_num = state.processed_snapshot_num.wrapping_add(1);
            let snapshot = self.host.source_read(state.processed_snapshot_num)?;
            self.host.lagometer_snapshot(snapshot.as_ref());
            if snapshot.is_some() {
                return Ok(snapshot);
            }
        }
        Ok(None)
    }

    fn process(&mut self, state: &mut ClientGameState, static_state: &mut ClientGameStaticState) -> PresentResult<()> {
        let latest = self.host.source_current();
        state.latest_snapshot_time = latest.server_time;
        if latest.number < state.latest_snapshot_num {
            return Err(drop_msg("CG_ProcessSnapshots: n < cg.latestSnapshotNum"));
        }
        state.latest_snapshot_num = latest.number;
        while state.snap.is_none() {
            let Some(snapshot) = self.read_next_snapshot(state)? else {
                return Ok(());
            };
            if (snapshot.flags & 2) == 0 {
                self.initial_snapshot(state, static_state, snapshot)?;
            }
        }
        loop {
            if state.next_snap.is_none() {
                let Some(snapshot) = self.read_next_snapshot(state)? else {
                    break;
                };
                let server_time = snapshot.server_time;
                self.next_snapshot(state, snapshot)?;
                let snap_time = state.snap.as_ref().expect("installed snapshot").server_time;
                if server_time < snap_time {
                    return Err(drop_msg("CG_ProcessSnapshots: Server time went backwards"));
                }
            }
            let next_time = state
                .next_snap
                .as_ref()
                .ok_or_else(|| state_msg("CG_ProcessSnapshots: missing next snapshot"))?
                .server_time;
            let snap_time = state.snap.as_ref().expect("installed snapshot").server_time;
            if state.time >= snap_time && state.time < next_time {
                break;
            }
            self.transition(state, static_state)?;
        }
        let snap_time = state.snap.as_ref().expect("installed snapshot").server_time;
        if state.time < snap_time {
            state.time = snap_time;
        }
        if let Some(next) = state.next_snap.as_ref() {
            if next.server_time <= state.time {
                return Err(drop_msg("CG_ProcessSnapshots: cg.nextSnap->serverTime <= cg.time"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnapshotOperation {
    Initial,
    Transition,
    Process,
}
