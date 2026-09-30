//! Quake III team-arena: match.
//!
//! Donor provenance: `src/content/q3/team-arena/match.ts`.

use qa_core::math::{sub3, vec3, vector_to_angles, Vec3};
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::client_spawn::*;
use crate::q3::team_arena::mirrors::*;
use crate::q3::team_arena::session::*;

// ---------------------------------------------------------------------------
// match.ts
// ---------------------------------------------------------------------------

/// Vote state (`VoteState`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VoteState {
    /// Vote start time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// Command string.
    pub string: String,
    /// Display string.
    pub display_string: String,
    /// Execution time.
    pub execute_time: i32,
}

/// Team vote state (`TeamVoteState`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TeamVoteState {
    /// Vote start time.
    pub time: i32,
    /// Yes votes.
    pub yes: i32,
    /// No votes.
    pub no: i32,
    /// Command string.
    pub string: String,
}

/// Level match record (`MatchState`).
#[derive(Debug, Clone, PartialEq)]
pub struct MatchState {
    /// Current time.
    pub time: i32,
    /// Start time.
    pub start_time: i32,
    /// Warmup time.
    pub warmup_time: i32,
    /// Warmup modification count.
    pub warmup_modification_count: i32,
    /// Restarted flag.
    pub restarted: bool,
    /// Connected clients.
    pub num_connected_clients: i32,
    /// Non-spectator clients.
    pub num_non_spectator_clients: i32,
    /// Playing clients.
    pub num_playing_clients: i32,
    /// Voting clients.
    pub num_voting_clients: i32,
    /// Per-team voting clients.
    pub num_team_voting_clients: [i32; 2],
    /// Rank-sorted clients.
    pub sorted_clients: [i32; MAX_CLIENTS],
    /// First follow target.
    pub follow1: i32,
    /// Second follow target.
    pub follow2: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Queued intermission time.
    pub intermission_queued: i32,
    /// Intermission origin.
    pub intermission_origin: Vec3,
    /// Intermission angles.
    pub intermission_angle: Vec3,
    /// Pending map change.
    pub changemap: Option<String>,
    /// Ready to exit.
    pub ready_to_exit: bool,
    /// Exit time.
    pub exit_time: i32,
    /// Current vote.
    pub vote: VoteState,
    /// Per-team votes.
    pub team_votes: [TeamVoteState; 2],
}

impl Default for MatchState {
    fn default() -> Self {
        Self {
            time: 0,
            start_time: 0,
            warmup_time: 0,
            warmup_modification_count: 0,
            restarted: false,
            num_connected_clients: 0,
            num_non_spectator_clients: 0,
            num_playing_clients: 0,
            num_voting_clients: 0,
            num_team_voting_clients: [0, 0],
            sorted_clients: [0; MAX_CLIENTS],
            follow1: 0,
            follow2: 0,
            intermission_time: 0,
            intermission_queued: 0,
            intermission_origin: vec3(0.0, 0.0, 0.0),
            intermission_angle: vec3(0.0, 0.0, 0.0),
            changemap: None,
            ready_to_exit: false,
            exit_time: 0,
            vote: VoteState::default(),
            team_votes: [TeamVoteState::default(), TeamVoteState::default()],
        }
    }
}

/// Shared match state.
pub type MatchStateRef = Rc<RefCell<MatchState>>;

/// Match settings (`MatchSettings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchSettings {
    /// Game type code.
    pub game_type: i32,
    /// Time limit in minutes.
    pub time_limit: i32,
    /// Frag limit.
    pub frag_limit: i32,
    /// Capture limit.
    pub capture_limit: i32,
    /// Warmup in seconds.
    pub warmup_seconds: i32,
    /// Warmup modification count.
    pub warmup_modification_count: i32,
    /// Server password.
    pub password: String,
    /// Password modification count.
    pub password_modification_count: i32,
}

/// Spawn selection plus respawn (`ClientSpawnRuntime` subset).
pub trait SpawnShort {
    /// Select a deathmatch spawn point.
    fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint;
    /// Respawn an entity.
    fn respawn(&self, entity: &EntityRef);
}

impl SpawnShort for ClientSpawnRuntime {
    fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint {
        ClientSpawnRuntime::select_spawn_point(self, avoid)
    }

    fn respawn(&self, entity: &EntityRef) {
        ClientSpawnRuntime::respawn(self, entity);
    }
}

/// Match services (`MatchHost`).
pub trait MatchHost {
    /// Product.
    fn product(&self) -> Product;
    /// Match state.
    fn state(&self) -> &MatchStateRef;
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Shared team scores.
    fn team_scores(&self) -> SharedSlots;
    /// Game random.
    fn random(&self) -> &GameRandom;
    /// Spawn services.
    fn spawn(&self) -> Rc<dyn SpawnShort>;
    /// Match settings.
    fn settings(&self) -> MatchSettings;
    /// Set a team (`"f"` or `"s"`).
    fn set_team(&self, entity: &EntityRef, team: &str);
    /// Stop following.
    fn stop_following(&self, entity: &EntityRef);
    /// Send the scoreboard.
    fn send_scoreboard(&self, entity: &EntityRef);
    /// Refresh client userinfo.
    fn client_userinfo_changed(&self, client_num: usize);
    /// Persist session data.
    fn write_session_data(&self);
    /// Queue a console command.
    fn append_console_command(&self, text: &str);
    /// Send a server command.
    fn send_server_command(&self, client_num: i32, text: &str);
    /// Write a config string.
    fn set_configstring(&self, index: i32, text: &str);
    /// Write a cvar.
    fn set_cvar(&self, name: &str, value: &str);
    /// Log a line.
    fn log(&self, text: &str);
    /// Warn a line.
    fn warn(&self, text: &str);
    /// Bot end-of-match hook.
    fn bot_interbreed_end_match(&self);
    /// Tournament info update.
    fn update_tournament_info(&self);
    /// Spawn victory-pad models (baseq3 single-player).
    fn spawn_models_on_victory_pads(&self);
    /// Whether single-player rules apply (missionpack).
    fn single_player(&self) -> bool;
}

pub(crate) fn match_at(values: &[i32], index: usize) -> i32 {
    match values.get(index) {
        Some(value) => *value,
        None => panic!("Match rank index has no source storage"),
    }
}

/// BSD `qsort` rank sort (`sortRankPrefix`).
pub(crate) fn sort_rank_prefix(values: &mut [i32], count: usize, compare: &dyn Fn(i32, i32) -> i32) {
    fn sort(values: &mut [i32], mut start: usize, mut length: usize, compare: &dyn Fn(i32, i32) -> i32) {
        let cmp = |values: &[i32], first: usize, second: usize| -> i32 {
            compare(match_at(values, first), match_at(values, second))
        };
        loop {
            if length < 7 {
                let mut mid = start + 1;
                while mid < start + length {
                    let mut left = mid;
                    while left > start && cmp(values, left - 1, left) > 0 {
                        values.swap(left, left - 1);
                        left -= 1;
                    }
                    mid += 1;
                }
                return;
            }
            let mut mid = start + length / 2;
            if length > 7 {
                let mut left = start;
                let mut end = start + length - 1;
                if length > 40 {
                    let distance = length / 8;
                    let median = |values: &[i32], a: usize, b: usize, c: usize| -> usize {
                        if cmp(values, a, b) < 0 {
                            if cmp(values, b, c) < 0 {
                                b
                            } else if cmp(values, a, c) < 0 {
                                c
                            } else {
                                a
                            }
                        } else if cmp(values, b, c) > 0 {
                            b
                        } else if cmp(values, a, c) < 0 {
                            a
                        } else {
                            c
                        }
                    };
                    left = median(values, left, left + distance, left + 2 * distance);
                    mid = median(values, mid - distance, mid, mid + distance);
                    end = median(values, end - 2 * distance, end - distance, end);
                }
                mid = if cmp(values, left, mid) < 0 {
                    if cmp(values, mid, end) < 0 {
                        mid
                    } else if cmp(values, left, end) < 0 {
                        end
                    } else {
                        left
                    }
                } else if cmp(values, mid, end) > 0 {
                    mid
                } else if cmp(values, left, end) < 0 {
                    left
                } else {
                    end
                };
            }
            values.swap(start, mid);
            let mut a = start + 1;
            let mut b = a;
            let mut c = start + length - 1;
            let mut d = c;
            let mut swapped = false;
            loop {
                while b <= c {
                    let result = cmp(values, b, start);
                    if result > 0 {
                        break;
                    }
                    if result == 0 {
                        swapped = true;
                        values.swap(a, b);
                        a += 1;
                    }
                    b += 1;
                }
                while b <= c {
                    let result = cmp(values, c, start);
                    if result < 0 {
                        break;
                    }
                    if result == 0 {
                        swapped = true;
                        values.swap(c, d);
                        d = d.saturating_sub(1);
                    }
                    if c == 0 {
                        break;
                    }
                    c -= 1;
                }
                if b > c {
                    break;
                }
                values.swap(b, c);
                swapped = true;
                b += 1;
                if c == 0 {
                    break;
                }
                c -= 1;
            }
            if !swapped {
                let mut mid = start + 1;
                while mid < start + length {
                    let mut left = mid;
                    while left > start && cmp(values, left - 1, left) > 0 {
                        values.swap(left, left - 1);
                        left -= 1;
                    }
                    mid += 1;
                }
                return;
            }
            let end = start + length;
            let mut range = (a - start).min(b - a);
            for index in 0..range {
                values.swap(start + index, b - range + index);
            }
            range = (d.saturating_sub(c)).min(end - d - 1);
            for index in 0..range {
                values.swap(b + index, end - range + index);
            }
            if b - a > 1 {
                sort(values, start, b - a, compare);
            }
            range = d.saturating_sub(c);
            if range <= 1 {
                return;
            }
            start = end - range;
            length = range;
        }
    }
    sort(values, 0, count, compare);
}

/// Match module state (`MatchModuleState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchModuleState {
    /// Password modification count.
    pub password_modification_count: i32,
}

impl MatchModuleState {
    /// Fresh module state.
    pub fn new() -> Self {
        Self {
            password_modification_count: -1,
        }
    }
}

impl Default for MatchModuleState {
    fn default() -> Self {
        Self::new()
    }
}

/// Match runtime (`MatchRuntime`).
pub struct MatchRuntime {
    /// Host services.
    pub host: Rc<dyn MatchHost>,
    module_state: RefCell<MatchModuleState>,
}

impl MatchRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn MatchHost>, module_state: MatchModuleState) -> Self {
        if host.pool().product() != host.product() {
            panic!("Match product differs from its entity pool");
        }
        if host.team_scores().len() != 4 {
            panic!("Match needs the shared four-team score storage");
        }
        Self {
            host,
            module_state: RefCell::new(module_state),
        }
    }

    /// Checkpoint capture.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveValue {
        SaveValue::map(vec![(
            "passwordModificationCount",
            SaveValue::Int(self.module_state.borrow().password_modification_count as i64),
        )])
    }

    /// Checkpoint restore.
    pub fn restore_save_state(&self, value: &SaveValue) {
        let count = SaveReader::new(value, "q3.matchModule")
            .field("passwordModificationCount")
            .integer(i64::MIN);
        self.module_state.borrow_mut().password_modification_count = count as i32;
    }

    fn client(&self, entity: &EntityRef) -> ClientRef {
        let slot = entity.borrow().slot;
        let owned = match self.host.pool().get(slot) {
            Some(owned) => Rc::ptr_eq(&owned, entity),
            None => false,
        };
        if !owned || entity.borrow().client.is_none() {
            panic!("Match requires an owned client entity");
        }
        entity
            .borrow()
            .client
            .clone()
            .unwrap_or_else(|| panic!("Match requires an owned client entity"))
    }

    fn sorted(&self, index: usize) -> i32 {
        match self.host.state().borrow().sorted_clients.get(index) {
            Some(value) => *value,
            None => panic!("Match rank index has no source storage"),
        }
    }

    fn score(&self, index: usize) -> i32 {
        self.host
            .pool()
            .client_at(index)
            .borrow()
            .ps
            .persistant
            .get(persistent_index::SCORE as usize)
    }

    /// Add a tournament player (`addTournamentPlayer`).
    pub fn add_tournament_player(&self) {
        {
            let state = self.host.state();
            let record = state.borrow();
            if record.num_playing_clients >= 2 || record.intermission_time != 0 {
                return;
            }
        }
        let mut selected: Option<usize> = None;
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            let record = client.borrow();
            if record.pers.connected != connection_state::CONNECTED
                || record.sess.session_team != team::SPECTATOR
                || record.sess.spectator_state == spectator_state::SCOREBOARD
                || record.sess.spectator_client < 0
            {
                continue;
            }
            let earlier = match selected {
                Some(selected) => {
                    record.sess.spectator_time < self.host.pool().client_at(selected).borrow().sess.spectator_time
                }
                None => true,
            };
            if earlier {
                selected = Some(index);
            }
        }
        if let Some(selected) = selected {
            self.host.state().borrow_mut().warmup_time = -1;
            self.host.set_team(&self.host.pool().at(selected), "f");
        }
    }

    fn remove_tournament_player(&self, rank: usize) {
        if self.host.state().borrow().num_playing_clients != 2 {
            return;
        }
        let number = self.sorted(rank);
        if self.host.pool().client_at(number as usize).borrow().pers.connected == connection_state::CONNECTED {
            self.host.set_team(&self.host.pool().at(number as usize), "s");
        }
    }

    /// Remove the tournament loser (`removeTournamentLoser`).
    pub fn remove_tournament_loser(&self) {
        self.remove_tournament_player(1);
    }

    /// Remove the tournament winner (`removeTournamentWinner`).
    pub fn remove_tournament_winner(&self) {
        self.remove_tournament_player(0);
    }

    /// Adjust tournament scores (`adjustTournamentScores`).
    pub fn adjust_tournament_scores(&self) {
        let winner = self.sorted(0);
        let first = self.host.pool().client_at(winner as usize);
        if first.borrow().pers.connected == connection_state::CONNECTED {
            let wins = first.borrow().sess.wins;
            first.borrow_mut().sess.wins = wins.wrapping_add(1);
            self.host.client_userinfo_changed(winner as usize);
        }
        let loser = self.sorted(1);
        let second = self.host.pool().client_at(loser as usize);
        if second.borrow().pers.connected == connection_state::CONNECTED {
            let losses = second.borrow().sess.losses;
            second.borrow_mut().sess.losses = losses.wrapping_add(1);
            self.host.client_userinfo_changed(loser as usize);
        }
    }

    /// Rank comparison (`sortRanks`).
    #[must_use]
    pub fn sort_ranks(&self, first: usize, second: usize) -> i32 {
        let a = self.host.pool().client_at(first);
        let b = self.host.pool().client_at(second);
        let (a_state, a_client, a_connected, a_team) = {
            let record = a.borrow();
            (
                record.sess.spectator_state,
                record.sess.spectator_client,
                record.pers.connected,
                record.sess.session_team,
            )
        };
        let (b_state, b_client, b_connected, b_team) = {
            let record = b.borrow();
            (
                record.sess.spectator_state,
                record.sess.spectator_client,
                record.pers.connected,
                record.sess.session_team,
            )
        };
        if a_state == spectator_state::SCOREBOARD || a_client < 0 {
            return 1;
        }
        if b_state == spectator_state::SCOREBOARD || b_client < 0 {
            return -1;
        }
        if a_connected == connection_state::CONNECTING {
            return 1;
        }
        if b_connected == connection_state::CONNECTING {
            return -1;
        }
        if a_team == team::SPECTATOR && b_team == team::SPECTATOR {
            let a_time = a.borrow().sess.spectator_time;
            let b_time = b.borrow().sess.spectator_time;
            return if a_time < b_time {
                -1
            } else if a_time > b_time {
                1
            } else {
                0
            };
        }
        if a_team == team::SPECTATOR {
            return 1;
        }
        if b_team == team::SPECTATOR {
            return -1;
        }
        if self.score(first) > self.score(second) {
            -1
        } else if self.score(first) < self.score(second) {
            1
        } else {
            0
        }
    }

    /// Recalculate ranks (`calculateRanks`).
    pub fn calculate_ranks(&self) {
        let game_type = self.host.settings().game_type;
        {
            let state = self.host.state();
            let mut record = state.borrow_mut();
            record.follow1 = -1;
            record.follow2 = -1;
            record.num_connected_clients = 0;
            record.num_non_spectator_clients = 0;
            record.num_playing_clients = 0;
            record.num_voting_clients = 0;
            record.num_team_voting_clients = [0, 0];
        }
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            let (connected, team_code) = {
                let record = client.borrow();
                (record.pers.connected, record.sess.session_team)
            };
            if connected == connection_state::DISCONNECTED {
                continue;
            }
            {
                let state = self.host.state();
                let mut record = state.borrow_mut();
                let slot = record.num_connected_clients as usize;
                record.sorted_clients[slot] = index as i32;
                record.num_connected_clients += 1;
            }
            if team_code == team::SPECTATOR {
                continue;
            }
            self.host.state().borrow_mut().num_non_spectator_clients += 1;
            if connected != connection_state::CONNECTED {
                continue;
            }
            self.host.state().borrow_mut().num_playing_clients += 1;
            if self.host.pool().at(index).borrow().r.sv_flags & server_entity_flags::BOT == 0 {
                self.host.state().borrow_mut().num_voting_clients += 1;
                if team_code == team::RED {
                    self.host.state().borrow_mut().num_team_voting_clients[0] += 1;
                } else if team_code == team::BLUE {
                    self.host.state().borrow_mut().num_team_voting_clients[1] += 1;
                }
            }
            let mut record = self.host.state().borrow_mut();
            if record.follow1 == -1 {
                record.follow1 = index as i32;
            } else if record.follow2 == -1 {
                record.follow2 = index as i32;
            }
        }
        {
            let count = self.host.state().borrow().num_connected_clients as usize;
            let mut sorted = self.host.state().borrow().sorted_clients;
            sort_rank_prefix(&mut sorted, count, &|a, b| self.sort_ranks(a as usize, b as usize));
            self.host.state().borrow_mut().sorted_clients = sorted;
        }
        if game_type >= game_type::TEAM {
            let red = self.host.team_scores().get(team::RED as usize);
            let blue = self.host.team_scores().get(team::BLUE as usize);
            let count = self.host.state().borrow().num_connected_clients as usize;
            for index in 0..count {
                let slot = self.sorted(index);
                self.host
                    .pool()
                    .client_at(slot as usize)
                    .borrow_mut()
                    .ps
                    .persistant
                    .set(
                        persistent_index::RANK as usize,
                        if red == blue {
                            2
                        } else if red > blue {
                            0
                        } else {
                            1
                        },
                    );
            }
        } else {
            let mut rank = -1;
            let mut score = 0;
            let playing = self.host.state().borrow().num_playing_clients as usize;
            for index in 0..playing {
                let slot = self.sorted(index);
                let client = self.host.pool().client_at(slot as usize);
                let new_score = client.borrow().ps.persistant.get(persistent_index::SCORE as usize);
                if index == 0 || new_score != score {
                    rank = index as i32;
                    client
                        .borrow_mut()
                        .ps
                        .persistant
                        .set(persistent_index::RANK as usize, rank);
                } else {
                    let previous = self.sorted(index - 1);
                    self.host
                        .pool()
                        .client_at(previous as usize)
                        .borrow_mut()
                        .ps
                        .persistant
                        .set(persistent_index::RANK as usize, rank | 0x4000);
                    client
                        .borrow_mut()
                        .ps
                        .persistant
                        .set(persistent_index::RANK as usize, rank | 0x4000);
                }
                score = new_score;
                if game_type == game_type::SINGLE_PLAYER && playing == 1 {
                    client
                        .borrow_mut()
                        .ps
                        .persistant
                        .set(persistent_index::RANK as usize, rank | 0x4000);
                }
            }
        }
        if game_type >= game_type::TEAM {
            self.host.set_configstring(
                6,
                &game_format_default("%i", &[FormatArg::Int(self.host.team_scores().get(team::RED as usize))]),
            );
            self.host.set_configstring(
                7,
                &game_format_default(
                    "%i",
                    &[FormatArg::Int(self.host.team_scores().get(team::BLUE as usize))],
                ),
            );
        } else {
            let connected = self.host.state().borrow().num_connected_clients;
            let first = if connected == 0 {
                -9999
            } else {
                self.score(self.sorted(0) as usize)
            };
            let second = if connected < 2 {
                -9999
            } else {
                self.score(self.sorted(1) as usize)
            };
            self.host
                .set_configstring(6, &game_format_default("%i", &[FormatArg::Int(first)]));
            self.host
                .set_configstring(7, &game_format_default("%i", &[FormatArg::Int(second)]));
        }
        self.check_exit_rules();
        if self.host.state().borrow().intermission_time != 0 {
            self.send_scoreboard_message_to_all_clients();
        }
    }

    /// Scoreboard refresh for all clients (`sendScoreboardMessageToAllClients`).
    pub fn send_scoreboard_message_to_all_clients(&self) {
        for index in 0..self.host.pool().max_clients() {
            if self.host.pool().client_at(index).borrow().pers.connected == connection_state::CONNECTED {
                self.host.send_scoreboard(&self.host.pool().at(index));
            }
        }
    }

    /// Move a client to intermission (`moveClientToIntermission`).
    pub fn move_client_to_intermission(&self, entity: &EntityRef) {
        let client = self.client(entity);
        if client.borrow().sess.spectator_state == spectator_state::FOLLOW {
            self.host.stop_following(entity);
        }
        let client = self.client(entity);
        let (origin, angles) = {
            let record = self.host.state().borrow();
            (record.intermission_origin, record.intermission_angle)
        };
        entity.borrow_mut().s.origin = origin;
        {
            let mut record = client.borrow_mut();
            record.ps.origin = origin;
            record.ps.viewangles = angles;
            record.ps.pm_type = move_type::INTERMISSION;
            for index in 0..record.ps.powerups.len() {
                record.ps.powerups.set(index, 0);
            }
            record.ps.e_flags = 0;
        }
        {
            let mut body = entity.borrow_mut();
            body.s.e_flags = 0;
            body.s.e_type = entity_type::GENERAL;
            body.s.modelindex = 0;
            body.s.loop_sound = 0;
            body.s.event = 0;
            body.r.contents = 0;
        }
    }

    /// Locate the intermission point (`findIntermissionPoint`).
    pub fn find_intermission_point(&self) -> SpawnPose {
        let entity = find_entity(
            &self.host.pool(),
            None,
            EntityStringField::Classname,
            Some("info_player_intermission"),
        );
        match entity {
            None => {
                let selected = self.host.spawn().select_spawn_point(vec3(0.0, 0.0, 0.0));
                let mut record = self.host.state().borrow_mut();
                record.intermission_origin = selected.origin;
                record.intermission_angle = selected.angles;
                SpawnPose {
                    origin: selected.origin,
                    angles: selected.angles,
                }
            }
            Some(entity) => {
                let (origin, angles, target) = {
                    let body = entity.borrow();
                    (body.s.origin, body.s.angles, body.target.clone())
                };
                {
                    let mut record = self.host.state().borrow_mut();
                    record.intermission_origin = origin;
                    record.intermission_angle = angles;
                }
                if target.is_some() {
                    let pool = self.host.pool();
                    let picked = pick_target(
                        &pool,
                        &|| self.host.random().rand(),
                        &|text| self.host.warn(text),
                        target.as_deref(),
                    );
                    if let Some(picked) = picked {
                        let angles = vector_to_angles(sub3(
                            picked.borrow().s.origin,
                            self.host.state().borrow().intermission_origin,
                        ));
                        self.host.state().borrow_mut().intermission_angle = angles;
                    }
                }
                let record = self.host.state().borrow();
                SpawnPose {
                    origin: record.intermission_origin,
                    angles: record.intermission_angle,
                }
            }
        }
    }

    /// Begin intermission (`beginIntermission`).
    pub fn begin_intermission(&self) {
        if self.host.state().borrow().intermission_time != 0 {
            return;
        }
        if self.host.settings().game_type == game_type::TOURNAMENT {
            self.adjust_tournament_scores();
        }
        let now = self.host.state().borrow().time;
        self.host.state().borrow_mut().intermission_time = now;
        self.find_intermission_point();
        if self.host.product() == Product::MissionPack {
            if self.host.single_player() {
                self.host.set_cvar("ui_singlePlayerActive", "0");
                self.host.update_tournament_info();
            }
        } else if self.host.settings().game_type == game_type::SINGLE_PLAYER {
            self.host.update_tournament_info();
            self.host.spawn_models_on_victory_pads();
        }
        for index in 0..self.host.pool().max_clients() {
            let entity = self.host.pool().at(index);
            if !entity.borrow().inuse {
                continue;
            }
            if entity.borrow().health <= 0 {
                self.host.spawn().respawn(&entity);
            }
            self.move_client_to_intermission(&entity);
        }
        self.send_scoreboard_message_to_all_clients();
    }

    /// Exit the level (`exitLevel`).
    pub fn exit_level(&self) {
        self.host.bot_interbreed_end_match();
        if self.host.settings().game_type == game_type::TOURNAMENT {
            if !self.host.state().borrow().restarted {
                self.remove_tournament_loser();
                self.host.append_console_command("map_restart 0\n");
                let mut record = self.host.state().borrow_mut();
                record.restarted = true;
                record.changemap = None;
                record.intermission_time = 0;
            }
            return;
        }
        self.host.append_console_command("vstr nextmap\n");
        {
            let mut record = self.host.state().borrow_mut();
            record.changemap = None;
            record.intermission_time = 0;
        }
        self.host.team_scores().set(team::RED as usize, 0);
        self.host.team_scores().set(team::BLUE as usize, 0);
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            if client.borrow().pers.connected == connection_state::CONNECTED {
                client
                    .borrow_mut()
                    .ps
                    .persistant
                    .set(persistent_index::SCORE as usize, 0);
            }
        }
        self.host.write_session_data();
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            if client.borrow().pers.connected == connection_state::CONNECTED {
                client.borrow_mut().pers.connected = connection_state::CONNECTING;
            }
        }
    }

    /// Log a match exit (`logExit`).
    pub fn log_exit(&self, reason: &str) {
        let settings = self.host.settings();
        let mut won = true;
        self.host
            .log(&game_format("Exit: %s\n", &[FormatArg::Text(reason.to_string())], 1024));
        let now = self.host.state().borrow().time;
        self.host.state().borrow_mut().intermission_queued = now;
        self.host.set_configstring(22, "1");
        if settings.game_type >= game_type::TEAM {
            self.host.log(&game_format(
                "red:%i  blue:%i\n",
                &[
                    FormatArg::Int(self.host.team_scores().get(team::RED as usize)),
                    FormatArg::Int(self.host.team_scores().get(team::BLUE as usize)),
                ],
                1024,
            ));
        }
        let connected = self.host.state().borrow().num_connected_clients.min(32);
        for index in 0..connected {
            let number = self.sorted(index as usize);
            let client = self.host.pool().client_at(number as usize);
            let record = client.borrow();
            if record.sess.session_team == team::SPECTATOR || record.pers.connected == connection_state::CONNECTING {
                continue;
            }
            let score = self.score(number as usize);
            let ping = record.ps.ping.min(999);
            let netname = record.pers.netname.clone();
            let rank = record.ps.persistant.get(persistent_index::RANK as usize);
            drop(record);
            self.host.log(&game_format(
                "score: %i  ping: %i  client: %i %s\n",
                &[
                    FormatArg::Int(score),
                    FormatArg::Int(ping),
                    FormatArg::Int(number),
                    FormatArg::Text(netname),
                ],
                1024,
            ));
            if self.host.product() == Product::MissionPack
                && self.host.single_player()
                && settings.game_type == game_type::TOURNAMENT
                && self.host.pool().at(number as usize).borrow().r.sv_flags & server_entity_flags::BOT != 0
                && rank == 0
            {
                won = false;
            }
        }
        if self.host.product() == Product::MissionPack && self.host.single_player() {
            if settings.game_type >= game_type::CTF {
                won =
                    self.host.team_scores().get(team::RED as usize) > self.host.team_scores().get(team::BLUE as usize);
            }
            self.host
                .append_console_command(if won { "spWin\n" } else { "spLose\n" });
        }
    }

    /// Intermission exit polling (`checkIntermissionExit`).
    pub fn check_intermission_exit(&self) {
        if self.host.settings().game_type == game_type::SINGLE_PLAYER {
            return;
        }
        let mut ready = 0;
        let mut not_ready = 0;
        let mut mask = 0;
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            let record = client.borrow();
            if record.pers.connected != connection_state::CONNECTED
                || self.host.pool().at(record.ps.client_num as usize).borrow().r.sv_flags & server_entity_flags::BOT
                    != 0
            {
                continue;
            }
            if record.ready_to_exit {
                ready += 1;
                if index < 16 {
                    mask |= 1 << index;
                }
            } else {
                not_ready += 1;
            }
        }
        let schema = stat_schema(self.host.product());
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            if client.borrow().pers.connected == connection_state::CONNECTED {
                client.borrow_mut().ps.stats.set(schema.clients_ready, mask);
            }
        }
        let (time, intermission_time) = {
            let record = self.host.state().borrow();
            (record.time, record.intermission_time)
        };
        if time < intermission_time.wrapping_add(5000) {
            return;
        }
        if ready == 0 {
            self.host.state().borrow_mut().ready_to_exit = false;
            return;
        }
        if not_ready == 0 {
            self.exit_level();
            return;
        }
        if !self.host.state().borrow().ready_to_exit {
            let mut record = self.host.state().borrow_mut();
            record.ready_to_exit = true;
            record.exit_time = record.time;
        }
        let (time, exit_time) = {
            let record = self.host.state().borrow();
            (record.time, record.exit_time)
        };
        if time >= exit_time.wrapping_add(10_000) {
            self.exit_level();
        }
    }

    /// Whether the score is tied (`scoreIsTied`).
    #[must_use]
    pub fn score_is_tied(&self) -> bool {
        if self.host.state().borrow().num_playing_clients < 2 {
            return false;
        }
        if self.host.settings().game_type >= game_type::TEAM {
            self.host.team_scores().get(team::RED as usize) == self.host.team_scores().get(team::BLUE as usize)
        } else {
            self.score(self.sorted(0) as usize) == self.score(self.sorted(1) as usize)
        }
    }

    /// Time/frag/capture exit rules (`checkExitRules`).
    pub fn check_exit_rules(&self) {
        if self.host.state().borrow().intermission_time != 0 {
            self.check_intermission_exit();
            return;
        }
        if self.host.state().borrow().intermission_queued != 0 {
            let delay = if self.host.product() == Product::MissionPack && self.host.single_player() {
                5000
            } else {
                1000
            };
            let (time, queued) = {
                let record = self.host.state().borrow();
                (record.time, record.intermission_queued)
            };
            if time.wrapping_sub(queued) >= delay {
                let mut record = self.host.state().borrow_mut();
                record.intermission_queued = 0;
                drop(record);
                self.begin_intermission();
            }
            return;
        }
        if self.score_is_tied() {
            return;
        }
        let settings = self.host.settings();
        {
            let record = self.host.state().borrow();
            if settings.time_limit != 0
                && record.warmup_time == 0
                && record.time.wrapping_sub(record.start_time) >= settings.time_limit.wrapping_mul(60_000)
            {
                drop(record);
                self.host.send_server_command(-1, "print \"Timelimit hit.\n\"");
                self.log_exit("Timelimit hit.");
                return;
            }
            if record.num_playing_clients < 2 {
                return;
            }
        }
        if settings.game_type < game_type::CTF && settings.frag_limit != 0 {
            for (team_code, name) in [(team::RED, "Red"), (team::BLUE, "Blue")] {
                if self.host.team_scores().get(team_code as usize) >= settings.frag_limit {
                    self.host
                        .send_server_command(-1, &format!("print \"{name} hit the fraglimit.\n\""));
                    self.log_exit("Fraglimit hit.");
                    return;
                }
            }
            for index in 0..self.host.pool().max_clients() {
                let client = self.host.pool().client_at(index);
                let record = client.borrow();
                if record.pers.connected != connection_state::CONNECTED || record.sess.session_team != team::FREE {
                    continue;
                }
                let netname = record.pers.netname.clone();
                drop(record);
                if self.score(index) >= settings.frag_limit {
                    self.log_exit("Fraglimit hit.");
                    self.host.send_server_command(
                        -1,
                        &game_format("print \"%s^7 hit the fraglimit.\n\"", &[FormatArg::Text(netname)], 1024),
                    );
                    return;
                }
            }
        }
        if settings.game_type >= game_type::CTF && settings.capture_limit != 0 {
            for (team_code, name) in [(team::RED, "Red"), (team::BLUE, "Blue")] {
                if self.host.team_scores().get(team_code as usize) >= settings.capture_limit {
                    self.host
                        .send_server_command(-1, &format!("print \"{name} hit the capturelimit.\n\""));
                    self.log_exit("Capturelimit hit.");
                    return;
                }
            }
        }
    }

    /// Tournament and warmup polling (`checkTournament`).
    pub fn check_tournament(&self) {
        if self.host.state().borrow().num_playing_clients == 0 {
            return;
        }
        let settings = self.host.settings();
        if settings.game_type == game_type::TOURNAMENT {
            if self.host.state().borrow().num_playing_clients < 2 {
                self.add_tournament_player();
            }
            if self.host.state().borrow().num_playing_clients != 2 {
                self.wait_for_players();
                return;
            }
            if self.host.state().borrow().warmup_time == 0 {
                return;
            }
        } else {
            if settings.game_type == game_type::SINGLE_PLAYER || self.host.state().borrow().warmup_time == 0 {
                return;
            }
            let not_enough = if settings.game_type > game_type::TEAM {
                team_count(
                    self.host.pool().clients(),
                    self.host.pool().max_clients(),
                    -1,
                    team::RED,
                ) < 1
                    || team_count(
                        self.host.pool().clients(),
                        self.host.pool().max_clients(),
                        -1,
                        team::BLUE,
                    ) < 1
            } else {
                self.host.state().borrow().num_playing_clients < 2
            };
            if not_enough {
                self.wait_for_players();
                return;
            }
        }
        let settings = self.host.settings();
        if settings.warmup_modification_count != self.host.state().borrow().warmup_modification_count {
            let mut record = self.host.state().borrow_mut();
            record.warmup_modification_count = settings.warmup_modification_count;
            record.warmup_time = -1;
        }
        if self.host.state().borrow().warmup_time < 0 {
            let time = self.host.state().borrow().time;
            let warmup = time.wrapping_add(settings.warmup_seconds.wrapping_sub(1).wrapping_mul(1000));
            self.host.state().borrow_mut().warmup_time = warmup;
            self.host
                .set_configstring(5, &game_format_default("%i", &[FormatArg::Int(warmup)]));
            return;
        }
        let (time, warmup_time) = {
            let record = self.host.state().borrow();
            (record.time, record.warmup_time)
        };
        if time > warmup_time {
            self.host.state().borrow_mut().warmup_time = warmup_time.wrapping_add(10_000);
            self.host.set_cvar("g_restarted", "1");
            self.host.append_console_command("map_restart 0\n");
            self.host.state().borrow_mut().restarted = true;
        }
    }

    fn wait_for_players(&self) {
        if self.host.state().borrow().warmup_time == -1 {
            return;
        }
        self.host.state().borrow_mut().warmup_time = -1;
        self.host.set_configstring(5, "-1");
        self.host.log("Warmup:\n");
    }

    /// Vote polling (`checkVote`).
    pub fn check_vote(&self) {
        {
            let state = self.host.state();
            let mut record = state.borrow_mut();
            if record.vote.execute_time != 0 && record.vote.execute_time < record.time {
                record.vote.execute_time = 0;
                let string = record.vote.string.clone();
                drop(record);
                self.host.append_console_command(&format!("{string}\n"));
            }
        }
        if self.host.state().borrow().vote.time == 0 {
            return;
        }
        let (time, vote_time, yes, no, voting) = {
            let record = self.host.state().borrow();
            (
                record.time,
                record.vote.time,
                record.vote.yes,
                record.vote.no,
                record.num_voting_clients,
            )
        };
        if time.wrapping_sub(vote_time) >= 30_000 {
            self.host.send_server_command(-1, "print \"Vote failed.\n\"");
        } else if yes > voting / 2 {
            self.host.send_server_command(-1, "print \"Vote passed.\n\"");
            self.host.state().borrow_mut().vote.execute_time = time.wrapping_add(3000);
        } else if no >= voting / 2 {
            self.host.send_server_command(-1, "print \"Vote failed.\n\"");
        } else {
            return;
        }
        self.host.state().borrow_mut().vote.time = 0;
        self.host.set_configstring(8, "");
    }

    /// Team broadcast (`printTeam`).
    pub fn print_team(&self, team_code: i32, message: &str) {
        for index in 0..self.host.pool().max_clients() {
            if self.host.pool().client_at(index).borrow().sess.session_team == team_code {
                self.host.send_server_command(index as i32, message);
            }
        }
    }

    /// Set the team leader (`setLeader`).
    pub fn set_leader(&self, team_code: i32, client_num: usize) {
        let client = self.host.pool().client_at(client_num);
        let (connected, team_now, netname) = {
            let record = client.borrow();
            (
                record.pers.connected,
                record.sess.session_team,
                record.pers.netname.clone(),
            )
        };
        if connected == connection_state::DISCONNECTED {
            self.print_team(
                team_code,
                &game_format("print \"%s is not connected\n\"", &[FormatArg::Text(netname)], 1024),
            );
            return;
        }
        if team_now != team_code {
            self.print_team(
                team_code,
                &game_format(
                    "print \"%s is not on the team anymore\n\"",
                    &[FormatArg::Text(netname)],
                    1024,
                ),
            );
            return;
        }
        for index in 0..self.host.pool().max_clients() {
            let other = self.host.pool().client_at(index);
            let mut record = other.borrow_mut();
            if record.sess.session_team == team_code && record.sess.team_leader != 0 {
                record.sess.team_leader = 0;
                drop(record);
                self.host.client_userinfo_changed(index);
            }
        }
        client.borrow_mut().sess.team_leader = 1;
        self.host.client_userinfo_changed(client_num);
        self.print_team(
            team_code,
            &game_format(
                "print \"%s is the new team leader\n\"",
                &[FormatArg::Text(netname)],
                1024,
            ),
        );
    }

    /// Ensure a team leader (`checkTeamLeader`).
    pub fn check_team_leader(&self, team_code: i32) {
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            let record = client.borrow();
            if record.sess.session_team == team_code && record.sess.team_leader != 0 {
                return;
            }
        }
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            if client.borrow().sess.session_team == team_code
                && self.host.pool().at(index).borrow().r.sv_flags & server_entity_flags::BOT == 0
            {
                client.borrow_mut().sess.team_leader = 1;
                break;
            }
        }
        // The second source loop is unconditional and may make a preceding
        // bot a second leader.
        for index in 0..self.host.pool().max_clients() {
            let client = self.host.pool().client_at(index);
            if client.borrow().sess.session_team == team_code {
                client.borrow_mut().sess.team_leader = 1;
                break;
            }
        }
    }

    /// Team-vote polling (`checkTeamVote`).
    pub fn check_team_vote(&self, team_code: i32) {
        if team_code != team::RED && team_code != team::BLUE {
            return;
        }
        let offset = if team_code == team::RED { 0 } else { 1 };
        let (vote_time, yes, no, string, voters) = {
            let record = self.host.state().borrow();
            let vote = &record.team_votes[offset];
            (
                vote.time,
                vote.yes,
                vote.no,
                vote.string.clone(),
                record.num_team_voting_clients[offset],
            )
        };
        if vote_time == 0 {
            return;
        }
        let majority = voters / 2;
        let time = self.host.state().borrow().time;
        if time.wrapping_sub(vote_time) >= 30_000 {
            self.host.send_server_command(-1, "print \"Team vote failed.\n\"");
        } else if yes > majority {
            self.host.send_server_command(-1, "print \"Team vote passed.\n\"");
            if let Some(target) = string.strip_prefix("leader") {
                self.set_leader(team_code, game_atoi(target) as usize);
            } else {
                self.host.append_console_command(&format!("{string}\n"));
            }
        } else if no >= majority {
            self.host.send_server_command(-1, "print \"Team vote failed.\n\"");
        } else {
            return;
        }
        self.host.state().borrow_mut().team_votes[offset].time = 0;
        self.host.set_configstring(12 + offset as i32, "");
    }

    /// Password cvar polling (`checkCvars`).
    pub fn check_cvars(&self) {
        let settings = self.host.settings();
        if settings.password_modification_count == self.module_state.borrow().password_modification_count {
            return;
        }
        self.module_state.borrow_mut().password_modification_count = settings.password_modification_count;
        let needed = !settings.password.is_empty() && ascii_fold(&settings.password) != "none";
        self.host.set_cvar("g_needpass", if needed { "1" } else { "0" });
    }
}
