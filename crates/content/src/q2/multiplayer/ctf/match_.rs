//! Q2 CTF match rules (`src/content/q2/multiplayer/ctf/match.ts`).
//!
//! Original CTF match, election, ghost and admin rules. GPL-2.0-or-later.

use qa_core::identity::ActorId;

use crate::q2::foundation::host::{Q2GameServices, Q2PresentationEvent};

use super::types::{
    ctf_name, ctf_player, ctf_print, ctf_team_name, Q2CtfElection, Q2CtfElectionKind, Q2CtfEvent, Q2CtfGhost,
    Q2CtfHooks, Q2CtfMatchPhase, Q2CtfMenuAction, Q2CtfMenuEntry, Q2CtfPlayingTeam, Q2CtfPrintLevel,
};

/// CTF match actions (`Q2CtfMatchActions`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfMatchActions {
    /// Reset players.
    pub reset_players: fn(&mut Q2GameServices),
    /// Reset a grapple.
    pub reset_grapple: fn(ActorId, &mut Q2GameServices),
    /// Join a team.
    pub join: fn(ActorId, &mut Q2GameServices, Q2CtfPlayingTeam, bool) -> bool,
}

/// CTF admin settings (`Q2CtfAdminSettings`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfAdminSettings {
    /// Match minutes.
    pub match_minutes: f64,
    /// Setup minutes.
    pub setup_minutes: f64,
    /// Start seconds.
    pub start_seconds: f64,
    /// Weapons stay.
    pub weapons_stay: bool,
    /// Instant items.
    pub instant_items: bool,
    /// Quad drop.
    pub quad_drop: bool,
    /// Instant weapons.
    pub instant_weapons: bool,
    /// Match lock.
    pub match_lock: bool,
}

/// Canonical CTF match actions.
pub fn ctf_match_actions() -> Q2CtfMatchActions {
    Q2CtfMatchActions {
        reset_players: super::index::ctf_reset_players,
        reset_grapple: super::index::ctf_reset_grapple,
        join: super::index::ctf_join,
    }
}

/// CTF match (`Q2CtfMatch`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfMatch {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
    /// Match actions.
    pub actions: Q2CtfMatchActions,
}

impl Q2CtfMatch {
    /// Run post-spawn match setup (`afterSpawn`).
    pub fn after_spawn(&self, game: &mut Q2GameServices) {
        if game.ctf.rules.competition > 1 {
            game.ctf.match_state.phase = Q2CtfMatchPhase::Setup;
            let setup = game.ctf.rules.setup_minutes;
            game.ctf.match_state.match_time = game.now() + setup * 60.0;
        }
    }

    /// Start match setup (`setup`).
    pub fn setup(&self, game: &mut Q2GameServices) {
        game.ctf.match_state.phase = Q2CtfMatchPhase::Setup;
        let setup = game.ctf.rules.setup_minutes;
        game.ctf.match_state.match_time = game.now() + setup * 60.0;
        if game.ctf.rules.competition < 3 {
            game.ctf.rules.competition = 2;
        }
        (self.actions.reset_players)(game);
    }

    /// Assign a ghost code (`assignGhost`).
    pub fn assign_ghost(&self, entity: ActorId, game: &mut Q2GameServices) {
        let team = ctf_player(game, &entity).team;
        if team == 0 {
            return;
        }
        if let Some(code) = ctf_player(game, &entity).ghost_code {
            game.ctf.match_state.ghosts.remove(&code);
        }
        if game.ctf.match_state.ghosts.len() >= game.options.max_clients as usize {
            ctf_player(game, &entity).ghost_code = None;
            return;
        }
        let mut code = 10000 + (game.random() * 90000.0).floor() as i32;
        // A deterministic source random stream may repeat; probe unused codes rather
        // than hanging forever while still keeping each live ghost code unique.
        while game.ctf.match_state.ghosts.contains_key(&code) {
            code = 10000 + (code - 9999) % 90000;
        }
        let name = ctf_name(game, &entity);
        game.ctf.match_state.ghosts.insert(
            code,
            Q2CtfGhost {
                code,
                team,
                name,
                actor: Some(entity.clone()),
                score: 0,
                deaths: 0,
                kills: 0,
                captures: 0,
                base_defense: 0,
                carrier_defense: 0,
            },
        );
        ctf_player(game, &entity).ghost_code = Some(code);
        ctf_print(
            game,
            &format!("Your ghost code is **** {code} ****\n"),
            Some(entity.clone()),
            Q2CtfPrintLevel::Chat,
        );
        ctf_print(
            game,
            &format!("If you lose connection, rejoin with your score intact by typing \"ghost {code}\".\n"),
            Some(entity),
            Q2CtfPrintLevel::High,
        );
    }

    /// Sync a ghost record (`syncGhost`).
    pub fn sync_ghost(&self, actor: ActorId, game: &mut Q2GameServices) {
        let code = game.ctf.states.get(&actor).and_then(|state| state.ghost_code);
        let Some(code) = code else {
            return;
        };
        if !game.ctf.match_state.ghosts.contains_key(&code) {
            return;
        }
        let Some(player) = (self.hooks.player)(actor.clone(), game) else {
            return;
        };
        let score = player.score;
        let name = player.name.clone();
        if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
            ghost.score = score;
            ghost.name = name;
        }
    }

    /// Restore a ghost (`restoreGhost`).
    pub fn restore_ghost(&self, entity: ActorId, game: &mut Q2GameServices, code: i32) -> bool {
        let team = ctf_player(game, &entity).team;
        if team != 0 || game.ctf.match_state.phase != Q2CtfMatchPhase::Game {
            return false;
        }
        let Some(ghost) = game.ctf.match_state.ghosts.get(&code).cloned() else {
            return false;
        };
        if (self.hooks.player)(entity.clone(), game).is_none() {
            return false;
        }
        if let Some(old_actor) = ghost.actor.clone() {
            if let Some(old) = game.ctf.states.get_mut(&old_actor) {
                old.ghost_code = None;
            }
        }
        ctf_player(game, &entity).ghost_code = Some(code);
        if let Some(entry) = game.ctf.match_state.ghosts.get_mut(&code) {
            entry.actor = Some(entity.clone());
        }
        ctf_player(game, &entity).spawn_state = 0;
        (self.actions.join)(entity.clone(), game, ghost.team, true);
        if let Some(player) = (self.hooks.player)(entity.clone(), game) {
            player.score = ghost.score;
        }
        let name = ctf_name(game, &entity);
        ctf_print(
            game,
            &format!("{name} has been reinstated to {} team.\n", ctf_team_name(ghost.team)),
            None,
            Q2CtfPrintLevel::High,
        );
        true
    }

    /// Mark ready state (`ready`).
    pub fn ready(&self, entity: ActorId, game: &mut Q2GameServices, ready: bool) -> bool {
        let (team, is_ready) = {
            let state = ctf_player(game, &entity);
            (state.team, state.ready)
        };
        let phase = game.ctf.match_state.phase;
        if team == 0
            || ready && phase != Q2CtfMatchPhase::Setup
            || !ready && phase != Q2CtfMatchPhase::Setup && phase != Q2CtfMatchPhase::Pregame
            || is_ready == ready
        {
            return false;
        }
        ctf_player(game, &entity).ready = ready;
        let name = ctf_name(game, &entity);
        ctf_print(
            game,
            &format!("{name} is {}.\n", if ready { "ready" } else { "no longer ready" }),
            None,
            Q2CtfPrintLevel::High,
        );
        if !ready && phase == Q2CtfMatchPhase::Pregame {
            game.ctf.match_state.phase = Q2CtfMatchPhase::Setup;
            let setup = game.ctf.rules.setup_minutes;
            game.ctf.match_state.match_time = game.now() + setup * 60.0;
            ctf_print(game, "Match halted.\n", None, Q2CtfPrintLevel::Chat);
        }
        if ready {
            let mut all_ready = true;
            let mut red = false;
            let mut blue = false;
            for actor in game.host.players() {
                if let Some(state) = game.ctf.states.get(&actor) {
                    if state.team == 0 {
                        continue;
                    }
                    if state.team == 1 {
                        red = true;
                    } else if state.team == 2 {
                        blue = true;
                    }
                    if !state.ready {
                        all_ready = false;
                    }
                }
            }
            if all_ready && red && blue {
                game.ctf.match_state.phase = Q2CtfMatchPhase::Pregame;
                let start = game.ctf.rules.start_seconds;
                game.ctf.match_state.match_time = game.now() + start;
                ctf_print(
                    game,
                    "All players are ready. Match starting.\n",
                    None,
                    Q2CtfPrintLevel::Chat,
                );
            }
        }
        true
    }

    /// Start the match (`start`).
    pub fn start(&self, game: &mut Q2GameServices) {
        game.ctf.match_state.phase = Q2CtfMatchPhase::Game;
        let minutes = game.ctf.rules.match_minutes;
        game.ctf.match_state.match_time = game.now() + minutes * 60.0;
        game.ctf.match_state.team1 = 0;
        game.ctf.match_state.team2 = 0;
        game.ctf.match_state.ghosts.clear();
        for actor in game.host.players() {
            if !game.ctf.states.contains_key(&actor) || game.entity(&actor).is_none() {
                continue;
            }
            if (self.hooks.player)(actor.clone(), game).is_none() {
                continue;
            }
            if let Some(player) = (self.hooks.player)(actor.clone(), game) {
                player.score = 0;
            }
            if let Some(state) = game.ctf.states.get_mut(&actor) {
                state.spawn_state = 0;
                state.ghost_code = None;
                state.last_returned_flag = None;
                state.last_fragged_carrier = None;
                state.last_hurt_carrier = None;
            }
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: actor.clone(),
                text: "******************\n\nMATCH HAS STARTED!\n\n******************".to_string(),
                instant: false,
                duration_seconds: None,
            });
            if game.ctf.states.get(&actor).map(|state| state.team).unwrap_or(0) == 0 {
                continue;
            }
            self.assign_ghost(actor.clone(), game);
            (self.actions.reset_grapple)(actor.clone(), game);
            (self.hooks.observer)(actor.clone(), game);
            if let Some(player) = (self.hooks.player)(actor.clone(), game) {
                player.dead = true;
                player.god = false;
            }
            game.require_entity_mut(&actor).visible = false;
            game.show(actor.clone());
            let at = game.now() + 1.0 + (game.random() * 30.0).floor() / 10.0;
            if let Some(state) = game.ctf.states.get_mut(&actor) {
                state.match_respawn_at = Some(at);
            }
            if let Some(player) = (self.hooks.player)(actor.clone(), game) {
                player.respawn_time = at;
            }
        }
    }

    /// Sum team scores (`totals`).
    pub fn totals(&self, game: &mut Q2GameServices) -> [i32; 2] {
        let mut one = 0;
        let mut two = 0;
        for actor in game.host.players() {
            let team = game.ctf.states.get(&actor).map(|state| state.team);
            let score = (self.hooks.player)(actor.clone(), game)
                .map(|player| player.score)
                .unwrap_or(0);
            if team == Some(1) {
                one += score;
            } else if team == Some(2) {
                two += score;
            }
        }
        [one, two]
    }

    /// End the match (`end`).
    pub fn end(&self, game: &mut Q2GameServices) {
        game.ctf.match_state.phase = Q2CtfMatchPhase::Post;
        let [one, two] = self.totals(game);
        game.ctf.match_state.total1 = one;
        game.ctf.match_state.total2 = two;
        let (team1, team2) = (game.ctf.match_state.team1, game.ctf.match_state.team2);
        ctf_print(
            game,
            &format!("MATCH COMPLETED!\nRED TEAM: {team1} captures, {one} points\nBLUE TEAM: {team2} captures, {two} points\n"),
            None,
            Q2CtfPrintLevel::Chat,
        );
        let captures = team1 - team2;
        let points = one - two;
        let difference = if captures != 0 { captures } else { points };
        if difference == 0 {
            ctf_print(game, "TIE GAME!\n", None, Q2CtfPrintLevel::Chat);
        } else {
            let winner = if difference > 0 { 1 } else { 2 };
            let unit = if captures != 0 { "CAPTURES" } else { "POINTS" };
            ctf_print(
                game,
                &format!("{} team won by {} {unit}!\n", ctf_team_name(winner), difference.abs()),
                None,
                Q2CtfPrintLevel::Chat,
            );
        }
        (self.hooks.end_level)(game, None);
    }

    /// Begin an election (`beginElection`).
    pub fn begin_election(
        &self,
        entity: ActorId,
        game: &mut Q2GameServices,
        kind: Q2CtfElectionKind,
        map: &str,
    ) -> bool {
        if game.ctf.match_state.election.is_some() {
            return false;
        }
        let percentage = game.ctf.rules.election_percentage;
        if percentage <= 0.0 {
            return false;
        }
        let mut count = 0;
        for actor in game.host.players() {
            if game.ctf.states.contains_key(&actor) {
                count += 1;
            }
        }
        if count < 2 {
            return false;
        }
        for state in game.ctf.states.values_mut() {
            state.voted = false;
        }
        let name = ctf_name(game, &entity);
        let subject = match kind {
            Q2CtfElectionKind::Map => format!("warping to {map}"),
            Q2CtfElectionKind::Admin => "admin rights".to_string(),
            Q2CtfElectionKind::Match => "match mode".to_string(),
        };
        let message = format!("{name} requested {subject}.");
        let needed = 1.max((f64::from(count) * percentage / 100.0).trunc() as i32);
        game.ctf.match_state.election = Some(Q2CtfElection {
            kind,
            target: entity,
            map: map.to_string(),
            message: message.clone(),
            votes: 0,
            needed,
            expires: game.now() + 20.0,
        });
        ctf_print(
            game,
            &format!("{message}\nType YES or NO to vote on this request.\n"),
            None,
            Q2CtfPrintLevel::Chat,
        );
        true
    }

    /// Vote in an election (`vote`).
    pub fn vote(&self, entity: ActorId, game: &mut Q2GameServices, yes: bool) -> bool {
        if ctf_player(game, &entity).voted {
            return false;
        }
        let Some(election) = game.ctf.match_state.election.clone() else {
            return false;
        };
        if election.target == entity || game.now() >= election.expires {
            return false;
        }
        ctf_player(game, &entity).voted = true;
        if yes {
            if let Some(entry) = game.ctf.match_state.election.as_mut() {
                entry.votes += 1;
            }
        }
        let votes = game
            .ctf
            .match_state
            .election
            .as_ref()
            .map(|entry| entry.votes)
            .unwrap_or(0);
        if votes >= election.needed {
            game.ctf.match_state.election = None;
            match election.kind {
                Q2CtfElectionKind::Match => self.setup(game),
                Q2CtfElectionKind::Map => (self.hooks.end_level)(game, Some(election.map)),
                Q2CtfElectionKind::Admin => {
                    if game.ctf.states.contains_key(&election.target) {
                        if let Some(state) = game.ctf.states.get_mut(&election.target) {
                            state.admin = true;
                        }
                        let name = ctf_name(game, &election.target);
                        ctf_print(
                            game,
                            &format!("{name} has become an admin.\n"),
                            None,
                            Q2CtfPrintLevel::High,
                        );
                    }
                }
            }
        } else {
            let left = 0.max((election.expires - game.now()).trunc() as i32);
            ctf_print(
                game,
                &format!("Votes: {votes} Needed: {} Time left: {left}s\n", election.needed),
                None,
                Q2CtfPrintLevel::High,
            );
        }
        true
    }

    /// Check match rules (`checkRules`).
    pub fn check_rules(&self, game: &mut Q2GameServices) -> bool {
        let now = game.now();
        if game
            .ctf
            .match_state
            .election
            .as_ref()
            .is_some_and(|election| election.expires <= now)
        {
            game.ctf.match_state.election = None;
            ctf_print(
                game,
                "Election timed out and has been cancelled.\n",
                None,
                Q2CtfPrintLevel::Chat,
            );
        }
        let phase = game.ctf.match_state.phase;
        if phase == Q2CtfMatchPhase::None {
            let limit = game.ctf.rules.capture_limit;
            if limit > 0 && game.ctf.match_state.team1.max(game.ctf.match_state.team2) >= limit {
                ctf_print(game, "Capturelimit hit.\n", None, Q2CtfPrintLevel::High);
                return true;
            }
            return false;
        }
        if game.ctf.match_state.match_time <= now {
            match phase {
                Q2CtfMatchPhase::Setup => {
                    if game.ctf.rules.competition < 3 {
                        game.ctf.match_state.phase = Q2CtfMatchPhase::None;
                        game.ctf.rules.competition = 1;
                        (self.actions.reset_players)(game);
                    } else {
                        let setup = game.ctf.rules.setup_minutes;
                        game.ctf.match_state.match_time = now + setup * 60.0;
                    }
                }
                Q2CtfMatchPhase::Pregame => self.start(game),
                Q2CtfMatchPhase::Game => self.end(game),
                _ => {}
            }
        }
        let remaining = 0.max((game.ctf.match_state.match_time - game.now()).trunc() as i32);
        if game.ctf.match_state.last_time != f64::from(remaining) {
            game.ctf.match_state.last_time = f64::from(remaining);
            let text = self.status(game);
            (self.hooks.emit)(game, Q2CtfEvent::MatchStatus { text });
        }
        false
    }

    /// Format match status (`status`).
    pub fn status(&self, game: &mut Q2GameServices) -> String {
        let time = 0.max((game.ctf.match_state.match_time - game.now()).trunc() as i32);
        let clock = format!("{:02}:{:02}", time / 60, time % 60);
        let phase = game.ctf.match_state.phase;
        if phase == Q2CtfMatchPhase::Setup {
            let mut waiting = 0;
            for actor in game.host.players() {
                if let Some(state) = game.ctf.states.get(&actor) {
                    if state.team != 0 && !state.ready {
                        waiting += 1;
                    }
                }
            }
            let prefix = if game.ctf.rules.competition < 3 {
                format!("{clock} ")
            } else {
                String::new()
            };
            return format!("{prefix}SETUP: {waiting} not ready");
        }
        if phase == Q2CtfMatchPhase::Pregame {
            return format!("{clock} UNTIL START");
        }
        if phase == Q2CtfMatchPhase::Game {
            return format!("{clock} MATCH");
        }
        String::new()
    }

    /// Open admin controls (`admin`).
    pub fn admin(&self, entity: ActorId, game: &mut Q2GameServices, password: &str) -> bool {
        let is_admin = ctf_player(game, &entity).admin;
        let password_ok = !password.is_empty()
            && !game.ctf.rules.admin_password.is_empty()
            && game.ctf.rules.admin_password == password;
        if !is_admin && password_ok {
            ctf_player(game, &entity).admin = true;
            let name = ctf_name(game, &entity);
            ctf_print(
                game,
                &format!("{name} has become an admin.\n"),
                None,
                Q2CtfPrintLevel::High,
            );
        }
        if !ctf_player(game, &entity).admin {
            return self.begin_election(entity, game, Q2CtfElectionKind::Admin, "");
        }
        let phase = game.ctf.match_state.phase;
        (self.hooks.emit)(
            game,
            Q2CtfEvent::Menu {
                actor: entity,
                title: "Administration Menu".to_string(),
                entries: vec![
                    Q2CtfMenuEntry {
                        label: "Settings".to_string(),
                        action: Some(Q2CtfMenuAction::AdminSettings),
                    },
                    Q2CtfMenuEntry {
                        label: if phase == Q2CtfMatchPhase::Setup {
                            "Force start match"
                        } else {
                            "Switch to match setup"
                        }
                        .to_string(),
                        action: Some(Q2CtfMenuAction::AdminStart),
                    },
                    Q2CtfMenuEntry {
                        label: "Cancel".to_string(),
                        action: Some(Q2CtfMenuAction::Close),
                    },
                ],
            },
        );
        true
    }

    /// Apply admin settings (`configure`).
    pub fn configure(&self, entity: ActorId, game: &mut Q2GameServices, values: &Q2CtfAdminSettings) -> bool {
        if !ctf_player(game, &entity).admin {
            return false;
        }
        for duration in [values.match_minutes, values.setup_minutes, values.start_seconds] {
            if !duration.is_finite() || duration <= 0.0 {
                panic!("CTF match durations must be positive");
            }
        }
        match game.ctf.match_state.phase {
            Q2CtfMatchPhase::Game => {
                let delta = (values.match_minutes - game.ctf.rules.match_minutes) * 60.0;
                game.ctf.match_state.match_time += delta;
            }
            Q2CtfMatchPhase::Setup => {
                let delta = (values.setup_minutes - game.ctf.rules.setup_minutes) * 60.0;
                game.ctf.match_state.match_time += delta;
            }
            Q2CtfMatchPhase::Pregame => {
                let delta = values.start_seconds - game.ctf.rules.start_seconds;
                game.ctf.match_state.match_time += delta;
            }
            _ => {}
        }
        {
            let rules = &mut game.ctf.rules;
            rules.match_minutes = values.match_minutes;
            rules.setup_minutes = values.setup_minutes;
            rules.start_seconds = values.start_seconds;
            rules.instant_weapons = values.instant_weapons;
            rules.match_lock = values.match_lock;
        }
        let flags = game.options.deathmatch_flags & !(4 | 16 | 16384)
            | if values.weapons_stay { 4 } else { 0 }
            | if values.instant_items { 16 } else { 0 }
            | if values.quad_drop { 16384 } else { 0 };
        (self.hooks.set_deathmatch_flags)(game, flags);
        let name = ctf_name(game, &entity);
        ctf_print(
            game,
            &format!("{name} changed match settings.\n"),
            None,
            Q2CtfPrintLevel::High,
        );
        true
    }

    /// Show the settings menu (`settingsMenu`).
    pub fn settings_menu(&self, entity: ActorId, game: &mut Q2GameServices) {
        if !ctf_player(game, &entity).admin {
            return;
        }
        let rules = &game.ctf.rules;
        let settings = Q2CtfAdminSettings {
            match_minutes: rules.match_minutes,
            setup_minutes: rules.setup_minutes,
            start_seconds: rules.start_seconds,
            weapons_stay: game.options.deathmatch_flags & 4 != 0,
            instant_items: game.options.deathmatch_flags & 16 != 0,
            quad_drop: game.options.deathmatch_flags & 16384 != 0,
            instant_weapons: rules.instant_weapons,
            match_lock: rules.match_lock,
        };
        (self.hooks.emit)(
            game,
            Q2CtfEvent::AdminSettings {
                actor: entity,
                settings,
            },
        );
    }

    /// Warp to a map (`warp`).
    pub fn warp(&self, entity: ActorId, game: &mut Q2GameServices, requested: &str) -> bool {
        let map = game
            .ctf
            .rules
            .warp_list
            .iter()
            .find(|value| value.to_lowercase() == requested.to_lowercase())
            .cloned();
        let Some(map) = map else {
            let list = game.ctf.rules.warp_list.join(" ");
            ctf_print(
                game,
                &format!("Available levels: {list}\n"),
                Some(entity),
                Q2CtfPrintLevel::High,
            );
            return false;
        };
        if ctf_player(game, &entity).admin {
            (self.hooks.end_level)(game, Some(map));
            return true;
        }
        self.begin_election(entity, game, Q2CtfElectionKind::Map, &map)
    }

    /// Boot a player (`boot`).
    pub fn boot(&self, entity: ActorId, game: &mut Q2GameServices, number: &str) -> bool {
        if !ctf_player(game, &entity).admin || number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
        let slot = number.parse::<i32>().unwrap_or(0) - 1;
        for actor in game.host.players() {
            if (self.hooks.player)(actor.clone(), game)
                .map(|player| player.slot)
                .unwrap_or(-2)
                == slot
            {
                (self.hooks.kick)(actor, game);
                return true;
            }
        }
        false
    }
}
