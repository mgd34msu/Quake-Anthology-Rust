//! Quake III team-arena: session.
//!
//! Donor provenance: `src/content/q3/team-arena/session.ts`.

use std::cell::Cell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;

// ---------------------------------------------------------------------------
// session.ts
// ---------------------------------------------------------------------------

/// Session world state (`SessionWorldState`).
pub struct SessionWorld {
    /// Client records.
    pub clients: Vec<ClientRef>,
    /// Configured client count.
    pub max_clients: usize,
    /// Shared team scores.
    pub team_scores: SharedSlots,
    /// Game type code.
    pub game_type: Cell<i32>,
    /// Auto-join teams.
    pub team_auto_join: Cell<bool>,
    /// Maximum game clients.
    pub max_game_clients: Cell<i32>,
    /// Current time.
    pub time: Cell<i32>,
    /// Non-spectator client count.
    pub num_non_spectator_clients: Cell<i32>,
    /// Fresh session.
    pub new_session: Cell<bool>,
}

/// Session userinfo (`SessionUserinfo`).
pub trait SessionUserinfo {
    /// Value for an info key.
    fn value_for_key(&self, key: &str) -> String;
}

pub(crate) const SESSION_MAX_STRING_CHARS: usize = 1024;

pub(crate) fn cvar_buffer(value: &str) -> String {
    let visible = match value.find('\0') {
        Some(nul) => &value[..nul],
        None => value,
    };
    let bounded: String = visible.chars().take(SESSION_MAX_STRING_CHARS - 1).collect();
    for ch in bounded.chars() {
        if ch as u32 > 255 {
            panic!("session cvar must contain byte characters");
        }
    }
    bounded
}

pub(crate) fn signed_byte(buffer: &[u8], offset: usize) -> i32 {
    let byte = buffer[offset];
    if byte < 128 {
        byte as i32
    } else {
        byte as i32 - 256
    }
}

pub(crate) fn scan_integer(buffer: &str, offset: usize) -> (i32, usize) {
    let bytes: Vec<u8> = buffer.chars().map(|ch| ch as u8).collect();
    if offset > bytes.len() {
        panic!("truncated session data scans beyond its terminating NUL");
    }
    let value = game_atoi(&buffer[offset..]);
    let mut cursor = offset;
    while cursor < bytes.len() && signed_byte(&bytes, cursor) <= 32 {
        cursor += 1;
    }
    if cursor == bytes.len() {
        return (value, cursor);
    }
    let sign = bytes[cursor];
    if sign == 43 || sign == 45 {
        cursor += 1;
    }
    loop {
        if cursor == bytes.len() {
            return (value, cursor + 1);
        }
        let character = bytes[cursor];
        cursor += 1;
        if !(48..=57).contains(&character) {
            return (value, cursor);
        }
    }
}

pub(crate) fn session_client_at(world: &SessionWorld, client_num: i32) -> ClientRef {
    if client_num < 0 || client_num as usize >= world.max_clients {
        panic!("session client {client_num} outside configured clients");
    }
    match world.clients.get(client_num as usize) {
        Some(client) => client.clone(),
        None => panic!("session client {client_num} has no backing state"),
    }
}

/// Count connected clients on a team (`teamCount`).
#[must_use]
pub fn team_count(clients: &[ClientRef], max_clients: usize, ignore_client_num: i32, team_code: i32) -> i32 {
    let mut count = 0;
    for index in 0..max_clients {
        if index as i32 == ignore_client_num {
            continue;
        }
        let client = match clients.get(index) {
            Some(client) => client,
            None => panic!("session client {index} has no backing state"),
        };
        let record = client.borrow();
        if record.pers.connected == connection_state::DISCONNECTED {
            continue;
        }
        if record.sess.session_team == team_code {
            count += 1;
        }
    }
    count
}

/// Pick the smaller team (`pickTeam`).
#[must_use]
pub fn pick_team(clients: &[ClientRef], max_clients: usize, team_scores: &SlotArray, ignore_client_num: i32) -> i32 {
    let blue = team_count(clients, max_clients, ignore_client_num, team::BLUE);
    let red = team_count(clients, max_clients, ignore_client_num, team::RED);
    if blue > red {
        return team::RED;
    }
    if red > blue {
        return team::BLUE;
    }
    if team_scores.get(team::BLUE as usize) > team_scores.get(team::RED as usize) {
        return team::RED;
    }
    team::BLUE
}

/// Session persistence manager (`GameSessionManager`).
pub struct GameSessionManager {
    /// World state.
    pub world: Rc<SessionWorld>,
    /// Services.
    pub services: Rc<dyn SessionServices>,
    cvars: Rc<dyn SessionCvarService>,
}

impl GameSessionManager {
    /// Fresh manager.
    pub fn new(world: Rc<SessionWorld>, services: Rc<dyn SessionServices>, cvars: Rc<dyn SessionCvarService>) -> Self {
        if world.max_clients > world.clients.len() {
            panic!("session maxClients exceeds its client storage");
        }
        if world.team_scores.len() != team::NUM_TEAMS as usize {
            panic!("session team scores require TEAM_NUM_TEAMS slots");
        }
        Self { world, services, cvars }
    }

    /// Persist one client's session record (`writeClient`).
    pub fn write_client(&self, client_num: i32) {
        let client = session_client_at(&self.world, client_num);
        let record = client.borrow();
        let value = game_format(
            "%i %i %i %i %i %i %i",
            &[
                FormatArg::Int(record.sess.session_team),
                FormatArg::Int(record.sess.spectator_time),
                FormatArg::Int(record.sess.spectator_state),
                FormatArg::Int(record.sess.spectator_client),
                FormatArg::Int(record.sess.wins),
                FormatArg::Int(record.sess.losses),
                FormatArg::Int(record.sess.team_leader),
            ],
            SESSION_MAX_STRING_CHARS,
        );
        self.cvars.set(&SessionCvarName::SessionN(client_num as usize), &value);
    }

    /// Restore one client's session record (`readClient`).
    pub fn read_client(&self, client_num: i32) {
        let client = session_client_at(&self.world, client_num);
        let buffer = cvar_buffer(&self.cvars.get(&SessionCvarName::SessionN(client_num as usize)));
        let (session_team, next) = scan_integer(&buffer, 0);
        let (spectator_time, next) = scan_integer(&buffer, next);
        let (spectator_state, next) = scan_integer(&buffer, next);
        let (spectator_client, next) = scan_integer(&buffer, next);
        let (wins, next) = scan_integer(&buffer, next);
        let (losses, next) = scan_integer(&buffer, next);
        let (team_leader, _) = scan_integer(&buffer, next);
        let mut record = client.borrow_mut();
        record.sess.spectator_time = spectator_time;
        record.sess.spectator_client = spectator_client;
        record.sess.wins = wins;
        record.sess.losses = losses;
        record.sess.session_team = session_team;
        record.sess.spectator_state = spectator_state;
        record.sess.team_leader = team_leader;
    }

    /// Initialize one client's session record (`initializeClient`).
    pub fn initialize_client(&self, client_num: i32, userinfo: &dyn SessionUserinfo) {
        let client = session_client_at(&self.world, client_num);
        let game_type = self.world.game_type.get();
        if game_type >= game_type::TEAM {
            if self.world.team_auto_join.get() {
                let team_code = pick_team(&self.world.clients, self.world.max_clients, &self.world.team_scores, -1);
                client.borrow_mut().sess.session_team = team_code;
                self.services.broadcast_team_change(client_num as usize, -1);
            } else {
                client.borrow_mut().sess.session_team = team::SPECTATOR;
            }
        } else if userinfo.value_for_key("team").starts_with('s') {
            client.borrow_mut().sess.session_team = team::SPECTATOR;
        } else {
            let team_code = match game_type {
                game_type::TOURNAMENT => {
                    if self.world.num_non_spectator_clients.get() >= 2 {
                        team::SPECTATOR
                    } else {
                        team::FREE
                    }
                }
                _ => {
                    if self.world.max_game_clients.get() > 0
                        && self.world.num_non_spectator_clients.get() >= self.world.max_game_clients.get()
                    {
                        team::SPECTATOR
                    } else {
                        team::FREE
                    }
                }
            };
            client.borrow_mut().sess.session_team = team_code;
        }
        client.borrow_mut().sess.spectator_state = spectator_state::FREE;
        let time = self.world.time.get();
        client.borrow_mut().sess.spectator_time = time;
        self.write_client(client_num);
    }

    /// Detect gametype changes (`initializeWorld`).
    pub fn initialize_world(&self) {
        let previous = game_atoi(&cvar_buffer(&self.cvars.get(&SessionCvarName::Session)));
        if self.world.game_type.get() != previous {
            self.world.new_session.set(true);
            self.services.print("Gametype changed, clearing session data.\n");
        }
    }

    /// Persist the world session record (`writeWorld`).
    pub fn write_world(&self) {
        self.cvars.set(
            &SessionCvarName::Session,
            &game_format(
                "%i",
                &[FormatArg::Int(self.world.game_type.get())],
                SESSION_MAX_STRING_CHARS,
            ),
        );
        for index in 0..self.world.max_clients {
            let client = session_client_at(&self.world, index as i32);
            if client.borrow().pers.connected == connection_state::CONNECTED {
                self.write_client(index as i32);
            }
        }
    }
}
