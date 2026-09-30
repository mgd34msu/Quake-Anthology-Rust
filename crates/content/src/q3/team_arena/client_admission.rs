//! Quake III team-arena: client admission.
//!
//! Donor provenance: `src/content/q3/team-arena/client-admission.ts`.

use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::team_arena::mirrors::*;
use crate::q3::team_arena::r#match::*;
use crate::q3::team_arena::session::*;

// ---------------------------------------------------------------------------
// client-admission.ts
// ---------------------------------------------------------------------------

/// Admission settings (`ClientAdmissionSettings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionSettings {
    /// Game type code.
    pub game_type: i32,
    /// Server password.
    pub password: String,
}

/// Bot services (`ClientBotServices`).
#[derive(Clone)]
pub enum ClientBotServices {
    /// Bots available.
    Available {
        /// Remove a queued begin.
        remove_queued_begin: Rc<dyn Fn(usize)>,
        /// Connect a bot.
        connect: Rc<dyn Fn(usize, bool) -> bool>,
        /// Shut down a bot client.
        shutdown_client: Rc<dyn Fn(usize, bool)>,
    },
    /// Bots unavailable.
    Unavailable {
        /// Reason.
        reason: String,
    },
}

/// Admission command services (`ClientAdmissionCommands`).
pub trait ClientAdmissionCommands {
    /// Announce a team change.
    fn broadcast_team_change(&self, client_num: usize, old_team: i32);
    /// Stop following.
    fn stop_following(&self, entity: &EntityRef);
}

/// Client-admission services (`ClientAdmissionHost`).
pub trait ClientAdmissionHost {
    /// Product.
    fn product(&self) -> Product;
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Shared team scores.
    fn team_scores(&self) -> SharedSlots;
    /// Server world.
    fn world(&self) -> WorldRef;
    /// Match state.
    fn match_state(&self) -> MatchStateRef;
    /// Whether the session is new.
    fn new_session(&self) -> bool;
    /// Session manager.
    fn session(&self) -> Rc<GameSessionManager>;
    /// Spawn a client.
    fn spawn_client(&self, entity: &EntityRef);
    /// Death services.
    fn death(&self) -> Rc<dyn DeathHost>;
    /// Recalculate ranks.
    fn calculate_ranks(&self);
    /// Command services.
    fn commands(&self) -> Rc<dyn ClientAdmissionCommands>;
    /// Bot services.
    fn bots(&self) -> ClientBotServices;
    /// Admission settings.
    fn settings(&self) -> ClientAdmissionSettings;
    /// Raw userinfo for a client.
    fn get_userinfo(&self, client_num: usize) -> String;
    /// Write a config string.
    fn set_configstring(&self, index: i32, value: &str);
    /// Send a server command.
    fn send_server_command(&self, client_num: i32, value: &str);
    /// Log a line.
    fn log(&self, value: &str);
    /// Packet filter (true rejects).
    fn filter_packet(&self, address: &str) -> bool;
}

pub(crate) const MAX_INFO_STRING: usize = 1024;

pub(crate) const MAX_QPATH: usize = 64;

pub(crate) const MAX_NETNAME: usize = 36;

pub(crate) const CS_PLAYERS: i32 = 544;

pub(crate) fn byte_buffer(value: &str, capacity: usize, label: &str) -> String {
    let visible = match value.find('\0') {
        Some(nul) => &value[..nul],
        None => value,
    };
    let bounded: String = visible.chars().take(capacity - 1).collect();
    for ch in bounded.chars() {
        if ch as u32 > 255 {
            panic!("{label} requires byte characters");
        }
    }
    bounded
}

pub(crate) fn ascii_upper(byte: u8) -> u8 {
    if (97..=122).contains(&byte) {
        byte - 32
    } else {
        byte
    }
}

pub(crate) fn case_insensitive_equal(first: &str, second: &str) -> bool {
    let left: Vec<u8> = first.chars().map(|ch| ch as u8).collect();
    let right: Vec<u8> = second.chars().map(|ch| ch as u8).collect();
    let mut index = 0;
    loop {
        let a = left.get(index).copied().unwrap_or(0);
        let b = right.get(index).copied().unwrap_or(0);
        if ascii_upper(a) != ascii_upper(b) {
            return false;
        }
        if a == 0 {
            return true;
        }
        index += 1;
    }
}

/// First matching info key, ASCII case-insensitive (`valueForKey`).
pub(crate) fn info_value_for_key(info: &str, wanted: &str) -> String {
    let bytes: Vec<u8> = info.chars().map(|ch| ch as u8).collect();
    let mut cursor = if info.starts_with('\\') { 1 } else { 0 };
    loop {
        let key_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != 92 {
            cursor += 1;
        }
        if cursor == bytes.len() {
            return String::new();
        }
        let key = String::from_utf8_lossy(&bytes[key_start..cursor]).into_owned();
        cursor += 1;
        let value_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != 92 {
            cursor += 1;
        }
        let value = String::from_utf8_lossy(&bytes[value_start..cursor]).into_owned();
        if case_insensitive_equal(wanted, &key) {
            return value;
        }
        if cursor == bytes.len() {
            return String::new();
        }
        cursor += 1;
    }
}

/// Client-sized info lookup (`clientInfoValue`).
#[must_use]
pub fn client_info_value(info: &str, wanted: &str) -> String {
    info_value_for_key(
        &byte_buffer(info, MAX_INFO_STRING, "userinfo"),
        &byte_buffer(wanted, MAX_INFO_STRING, "userinfo key"),
    )
}

pub(crate) fn valid_info(info: &str) -> bool {
    !info.contains('"') && !info.contains(';')
}

/// Clean a client name (`cleanClientName`).
#[must_use]
pub fn clean_client_name(input: &str) -> String {
    let source = byte_buffer(input, MAX_INFO_STRING, "client name");
    let bytes: Vec<u8> = source.chars().map(|ch| ch as u8).collect();
    let output_size = MAX_NETNAME - 1;
    let mut output = String::new();
    let mut colorless_length = 0;
    let mut spaces = 0;
    let mut cursor = 0;
    while cursor < bytes.len() {
        let character = bytes[cursor];
        cursor += 1;
        if output.is_empty() && character == 32 {
            continue;
        }
        if character == 94 {
            if cursor == bytes.len() {
                break;
            }
            let color = bytes[cursor] as i32;
            if ((color - 48) & 7) == 0 {
                cursor += 1;
                continue;
            }
            if output.len() > output_size - 2 {
                break;
            }
            output.push('^');
            output.push(bytes[cursor] as char);
            cursor += 1;
            continue;
        }
        if character == 32 {
            spaces += 1;
            if spaces > 3 {
                continue;
            }
        } else {
            spaces = 0;
        }
        if output.len() > output_size - 1 {
            break;
        }
        output.push(character as char);
        colorless_length += 1;
    }
    if output.is_empty() || colorless_length == 0 {
        "UnnamedPlayer".to_string()
    } else {
        output
    }
}

pub(crate) fn reset_client(client: &ClientRef, product: Product) {
    // The donor memsets the record while preserving the pool's stable object
    // graph; nothing else aliases these sub-records here, so a fresh client
    // is equivalent.
    *client.borrow_mut() = GameClient::new(product);
}

pub(crate) fn reset_player_state(ps: &mut PlayerState) {
    let fresh = PlayerState::new(ps.product);
    ps.copy_from(&fresh, AuthorityCopy::PreserveAuthority);
}

/// `CS_PLAYERS` encoding (`clientPresentationConfig`).
#[must_use]
pub fn client_presentation_config(client: &ClientRef, userinfo: &str, game_type: i32, bot_team: Option<i32>) -> String {
    let (model_key, head_key) = if game_type >= game_type::TEAM {
        ("team_model", "team_headmodel")
    } else {
        ("model", "headmodel")
    };
    let model = byte_buffer(&client_info_value(userinfo, model_key), MAX_QPATH, "model");
    let head_model = byte_buffer(&client_info_value(userinfo, head_key), MAX_QPATH, "head model");
    let record = client.borrow();
    let team_task = game_atoi(&client_info_value(userinfo, "teamtask"));
    let team_leader = record.sess.team_leader;
    let color1 = client_info_value(userinfo, "color1");
    let color2 = client_info_value(userinfo, "color2");
    match bot_team {
        Some(bot_team) => game_format_default(
            "n\\%s\\t\\%i\\model\\%s\\hmodel\\%s\\c1\\%s\\c2\\%s\\hc\\%i\\w\\%i\\l\\%i\\skill\\%s\\tt\\%d\\tl\\%d",
            &[
                FormatArg::Text(record.pers.netname.clone()),
                FormatArg::Int(bot_team),
                FormatArg::Text(model),
                FormatArg::Text(head_model),
                FormatArg::Text(color1),
                FormatArg::Text(color2),
                FormatArg::Int(record.pers.max_health),
                FormatArg::Int(record.sess.wins),
                FormatArg::Int(record.sess.losses),
                FormatArg::Text(client_info_value(userinfo, "skill")),
                FormatArg::Int(team_task),
                FormatArg::Int(team_leader),
            ],
        ),
        None => game_format_default(
            "n\\%s\\t\\%i\\model\\%s\\hmodel\\%s\\g_redteam\\%s\\g_blueteam\\%s\\c1\\%s\\c2\\%s\\hc\\%i\\w\\%i\\l\\%i\\tt\\%d\\tl\\%d",
            &[
                FormatArg::Text(record.pers.netname.clone()),
                FormatArg::Int(record.sess.session_team),
                FormatArg::Text(model),
                FormatArg::Text(head_model),
                FormatArg::Text(client_info_value(userinfo, "g_redteam")),
                FormatArg::Text(client_info_value(userinfo, "g_blueteam")),
                FormatArg::Text(color1),
                FormatArg::Text(color2),
                FormatArg::Int(record.pers.max_health),
                FormatArg::Int(record.sess.wins),
                FormatArg::Int(record.sess.losses),
                FormatArg::Int(team_task),
                FormatArg::Int(team_leader),
            ],
        ),
    }
}

/// Client admission runtime (`ClientAdmissionRuntime`).
pub struct ClientAdmissionRuntime {
    /// Host services.
    pub host: Rc<dyn ClientAdmissionHost>,
}

impl ClientAdmissionRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn ClientAdmissionHost>) -> Self {
        if host.pool().product() != host.product() {
            panic!("Client admission product differs from its entity pool");
        }
        if host.team_scores().len() != team::NUM_TEAMS as usize {
            panic!("Client admission requires four shared team-score slots");
        }
        Self { host }
    }

    fn client_entity(&self, client_num: i32) -> EntityRef {
        if client_num < 0 || client_num as usize >= self.host.pool().max_clients() {
            panic!("client {client_num} outside configured clients");
        }
        self.host.pool().at(client_num as usize)
    }

    fn userinfo(&self, client_num: i32) -> String {
        byte_buffer(
            &self.host.get_userinfo(client_num as usize),
            MAX_INFO_STRING,
            "userinfo",
        )
    }

    /// Apply userinfo changes (`userinfoChanged`).
    pub fn userinfo_changed(&self, client_num: i32) {
        let entity = self.client_entity(client_num);
        let client = self.host.pool().client_at(client_num as usize);
        let mut userinfo = self.userinfo(client_num);
        if !valid_info(&userinfo) {
            userinfo = "\\name\\badinfo".to_string();
        }
        if client_info_value(&userinfo, "ip") == "localhost" {
            client.borrow_mut().pers.local_client = true;
        }
        client.borrow_mut().pers.predict_item_pickup = game_atoi(&client_info_value(&userinfo, "cg_predictItems")) != 0;
        let old_name = byte_buffer(&client.borrow().pers.netname, MAX_INFO_STRING, "client name");
        client.borrow_mut().pers.netname = clean_client_name(&client_info_value(&userinfo, "name"));
        {
            let record = client.borrow();
            if record.sess.session_team == team::SPECTATOR && record.sess.spectator_state == spectator_state::SCOREBOARD
            {
                drop(record);
                client.borrow_mut().pers.netname = "scoreboard".to_string();
            }
        }
        let (connected, netname) = {
            let record = client.borrow();
            (record.pers.connected, record.pers.netname.clone())
        };
        if connected == connection_state::CONNECTED && old_name != netname {
            self.host.send_server_command(
                -1,
                &game_format_default(
                    "print \"%s^7 renamed to %s\n\"",
                    &[FormatArg::Text(old_name), FormatArg::Text(netname)],
                ),
            );
        }
        let mut health = game_atoi(&client_info_value(&userinfo, "handicap"));
        let product = self.host.product();
        if product == Product::MissionPack && client.borrow().ps.powerups.get(powerup::GUARD as usize) != 0 {
            health = 200;
        } else if !(1..=100).contains(&health) {
            health = 100;
        }
        {
            let mut record = client.borrow_mut();
            record.pers.max_health = health;
            let schema = stat_schema(product);
            record.ps.stats.set(schema.max_health, health);
        }
        let settings = self.host.settings();
        let game_type = settings.game_type;
        let mut team_code = client.borrow().sess.session_team;
        if game_type >= game_type::TEAM && entity.borrow().r.sv_flags & server_entity_flags::BOT != 0 {
            let requested = client_info_value(&userinfo, "team");
            if case_insensitive_equal(&requested, "red") || case_insensitive_equal(&requested, "r") {
                team_code = team::RED;
            } else if case_insensitive_equal(&requested, "blue") || case_insensitive_equal(&requested, "b") {
                team_code = team::BLUE;
            } else {
                team_code = pick_team(
                    self.host.pool().clients(),
                    self.host.pool().max_clients(),
                    &self.host.team_scores(),
                    client_num,
                );
            }
        }
        if product == Product::MissionPack && game_type >= game_type::TEAM {
            client.borrow_mut().pers.team_info = true;
        } else {
            let overlay = client_info_value(&userinfo, "teamoverlay");
            client.borrow_mut().pers.team_info = overlay.is_empty() || game_atoi(&overlay) != 0;
        }
        let bot_team = if entity.borrow().r.sv_flags & server_entity_flags::BOT != 0 {
            Some(team_code)
        } else {
            None
        };
        let config = client_presentation_config(&client, &userinfo, game_type, bot_team);
        self.host.set_configstring(CS_PLAYERS + client_num, &config);
        self.host.log(&game_format_default(
            "ClientUserinfoChanged: %i %s\n",
            &[FormatArg::Int(client_num), FormatArg::Text(config)],
        ));
    }

    /// Connect a client, returning a rejection reason (`connect`).
    pub fn connect(&self, client_num: i32, first_time: bool, is_bot: bool) -> Option<String> {
        let entity = self.client_entity(client_num);
        let userinfo = self.userinfo(client_num);
        let address = client_info_value(&userinfo, "ip");
        if self.host.filter_packet(&address) {
            return Some("You are banned from this server.".to_string());
        }
        let password = byte_buffer(&self.host.settings().password, MAX_INFO_STRING, "password");
        if entity.borrow().r.sv_flags & server_entity_flags::BOT == 0
            && address != "localhost"
            && !password.is_empty()
            && !case_insensitive_equal(&password, "none")
            && password != client_info_value(&userinfo, "password")
        {
            return Some("Invalid password".to_string());
        }
        let client = self.host.pool().client_at(client_num as usize);
        entity.borrow_mut().client = Some(client.clone());
        reset_client(&client, self.host.product());
        client.borrow_mut().pers.connected = connection_state::CONNECTING;
        struct TeamInfo {
            team: String,
        }
        impl SessionUserinfo for TeamInfo {
            fn value_for_key(&self, _key: &str) -> String {
                self.team.clone()
            }
        }
        let session_info = TeamInfo {
            team: client_info_value(&userinfo, "team"),
        };
        if first_time || self.host.new_session() {
            self.host.session().initialize_client(client_num, &session_info);
        }
        self.host.session().read_client(client_num);
        if is_bot {
            entity.borrow_mut().r.sv_flags |= server_entity_flags::BOT;
            self.host.pool().activate_client(client_num as usize);
            match self.host.bots() {
                ClientBotServices::Unavailable { reason } => return Some(reason),
                ClientBotServices::Available { connect, .. } => {
                    if !connect(client_num as usize, !first_time) {
                        return Some("BotConnectfailed".to_string());
                    }
                }
            }
        }
        self.host.log(&game_format_default(
            "ClientConnect: %i\n",
            &[FormatArg::Int(client_num)],
        ));
        self.userinfo_changed(client_num);
        if first_time {
            let netname = client.borrow().pers.netname.clone();
            self.host.send_server_command(
                -1,
                &game_format_default("print \"%s^7 connected\n\"", &[FormatArg::Text(netname)]),
            );
        }
        let game_type = self.host.settings().game_type;
        if game_type >= game_type::TEAM && client.borrow().sess.session_team != team::SPECTATOR {
            self.host.commands().broadcast_team_change(client_num as usize, -1);
        }
        self.host.calculate_ranks();
        None
    }

    /// Begin a client (`begin`).
    pub fn begin(&self, client_num: i32) {
        let entity = self.client_entity(client_num);
        let client = self.host.pool().client_at(client_num as usize);
        let linked = self
            .host
            .world()
            .link_state(entity.borrow().slot as i32)
            .map(|state| state.linked)
            .unwrap_or(false);
        if linked {
            self.host.world().unlink(entity.borrow().slot as i32);
        }
        init_game_entity(&entity);
        entity.borrow_mut().touch = None;
        entity.borrow_mut().pain = None;
        entity.borrow_mut().client = Some(client.clone());
        client.borrow_mut().pers.connected = connection_state::CONNECTED;
        client.borrow_mut().pers.enter_time = self.host.match_state().borrow().time;
        client.borrow_mut().pers.team_state.state = team_state::BEGIN;
        let flags = client.borrow().ps.e_flags;
        reset_player_state(&mut client.borrow_mut().ps);
        client.borrow_mut().ps.e_flags = flags;
        self.host.spawn_client(&entity);
        if client.borrow().sess.session_team != team::SPECTATOR {
            let origin = client.borrow().ps.origin;
            let temporary = self.host.pool().temp_entity(origin, entity_event::PLAYER_TELEPORT_IN);
            temporary.borrow_mut().s.client_num = entity.borrow().s.client_num;
            if self.host.settings().game_type != game_type::TOURNAMENT {
                let netname = client.borrow().pers.netname.clone();
                self.host.send_server_command(
                    -1,
                    &game_format_default("print \"%s^7 entered the game\n\"", &[FormatArg::Text(netname)]),
                );
            }
        }
        self.host
            .log(&game_format_default("ClientBegin: %i\n", &[FormatArg::Int(client_num)]));
        self.host.calculate_ranks();
    }

    /// Disconnect a client (`disconnect`).
    pub fn disconnect(&self, client_num: i32) {
        let entity = self.client_entity(client_num);
        match self.host.bots() {
            ClientBotServices::Available {
                remove_queued_begin, ..
            } => {
                remove_queued_begin(client_num as usize);
            }
            ClientBotServices::Unavailable { reason } => {
                if entity.borrow().r.sv_flags & server_entity_flags::BOT != 0 {
                    panic!("{reason}");
                }
            }
        }
        let client = match entity.borrow().client.clone() {
            Some(client) => client,
            None => return,
        };
        for index in 0..self.host.pool().max_clients() {
            let follower = self.host.pool().client_at(index);
            let record = follower.borrow();
            if record.sess.session_team == team::SPECTATOR
                && record.sess.spectator_state == spectator_state::FOLLOW
                && record.sess.spectator_client == client_num
            {
                drop(record);
                self.host.commands().stop_following(&self.host.pool().at(index));
            }
        }
        {
            let record = client.borrow();
            if record.pers.connected == connection_state::CONNECTED && record.sess.session_team != team::SPECTATOR {
                let origin = record.ps.origin;
                drop(record);
                let temporary = self.host.pool().temp_entity(origin, entity_event::PLAYER_TELEPORT_OUT);
                temporary.borrow_mut().s.client_num = entity.borrow().s.client_num;
                self.host.death().toss_client_items(&entity);
                if self.host.product() == Product::MissionPack {
                    self.host.death().toss_client_persistant_powerups(&entity);
                    if self.host.settings().game_type == game_type::HARVESTER {
                        self.host.death().toss_client_cubes(&entity);
                    }
                }
            }
        }
        self.host.log(&game_format_default(
            "ClientDisconnect: %i\n",
            &[FormatArg::Int(client_num)],
        ));
        let game_type = self.host.settings().game_type;
        {
            let state = self.host.match_state();
            let record = state.borrow();
            if game_type == game_type::TOURNAMENT
                && record.intermission_time == 0
                && record.warmup_time == 0
                && record.sorted_clients.get(1).copied() == Some(client_num)
            {
                let winner = match record.sorted_clients.first().copied() {
                    Some(winner) => winner,
                    None => panic!("tournament winner is absent from sorted clients"),
                };
                drop(record);
                let winner_client = self.host.pool().client_at(winner as usize);
                let wins = winner_client.borrow().sess.wins;
                winner_client.borrow_mut().sess.wins = wins.wrapping_add(1);
                self.userinfo_changed(winner);
            }
        }
        self.host.world().unlink(entity.borrow().slot as i32);
        entity.borrow_mut().s.modelindex = 0;
        self.host.pool().deactivate_client(client_num as usize);
        entity.borrow_mut().set_classname(Some("disconnected".to_string()));
        client.borrow_mut().pers.connected = connection_state::DISCONNECTED;
        client
            .borrow_mut()
            .ps
            .persistant
            .set(persistent_index::TEAM as usize, team::FREE);
        client.borrow_mut().sess.session_team = team::FREE;
        self.host.set_configstring(CS_PLAYERS + client_num, "");
        self.host.calculate_ranks();
        if entity.borrow().r.sv_flags & server_entity_flags::BOT != 0 {
            if let ClientBotServices::Available { shutdown_client, .. } = self.host.bots() {
                shutdown_client(client_num as usize, false);
            }
        }
    }
}
