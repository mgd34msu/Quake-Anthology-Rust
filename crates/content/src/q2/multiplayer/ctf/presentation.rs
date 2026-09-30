//! Q2 CTF presentation (`src/content/q2/multiplayer/ctf/presentation.ts`).
//!
//! Original CTF scoreboard, status, player ID and team communication.
//! GPL-2.0-or-later.

use qa_core::identity::ActorId;
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3};

use crate::contract::{PoweredProtectionState, RegularArmorState};
use crate::q2::foundation::host::{Q2GameServices, Q2Solid, Q2TraceRequest};
use crate::q2::foundation::weapons::player::weapon_definition;
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::TraceHit;

use super::flags::{Q2CtfFlags, ctf_can_see};
use super::types::{
    Q2CtfEvent, Q2CtfForceJoin, Q2CtfHooks, Q2CtfMatchPhase, Q2CtfMenuAction, Q2CtfMenuEntry, Q2CtfPrintLevel, Q2CtfScoreRow,
    Q2CtfTeam, Q2CtfTech, ctf_carried_flag, ctf_name, ctf_player, ctf_print, item_id,
};

/// Tech display name.
struct TechName {
    /// Tech.
    tech: Q2CtfTech,
    /// Classname.
    classname: &'static str,
    /// Display name.
    name: &'static str,
}

/// Tech names (`techNames`).
const TECH_NAMES: [TechName; 4] = [
    TechName { tech: Q2CtfTech::Tech1, classname: "item_tech1", name: "Disruptor Shield" },
    TechName { tech: Q2CtfTech::Tech2, classname: "item_tech2", name: "Power Amplifier" },
    TechName { tech: Q2CtfTech::Tech3, classname: "item_tech3", name: "Time Accel" },
    TechName { tech: Q2CtfTech::Tech4, classname: "item_tech4", name: "AutoDoc" },
];

/// Location entry.
struct LocationEntry {
    /// Classname.
    classname: &'static str,
    /// Priority.
    priority: i32,
}

/// Location entries (`locations`).
const LOCATIONS: [LocationEntry; 24] = [
    LocationEntry { classname: "item_flag_team1", priority: 1 },
    LocationEntry { classname: "item_flag_team2", priority: 1 },
    LocationEntry { classname: "item_quad", priority: 2 },
    LocationEntry { classname: "item_invulnerability", priority: 2 },
    LocationEntry { classname: "weapon_bfg", priority: 3 },
    LocationEntry { classname: "weapon_railgun", priority: 4 },
    LocationEntry { classname: "weapon_rocketlauncher", priority: 4 },
    LocationEntry { classname: "weapon_hyperblaster", priority: 4 },
    LocationEntry { classname: "weapon_chaingun", priority: 4 },
    LocationEntry { classname: "weapon_grenadelauncher", priority: 4 },
    LocationEntry { classname: "weapon_machinegun", priority: 4 },
    LocationEntry { classname: "weapon_supershotgun", priority: 4 },
    LocationEntry { classname: "weapon_shotgun", priority: 4 },
    LocationEntry { classname: "item_power_screen", priority: 5 },
    LocationEntry { classname: "item_power_shield", priority: 5 },
    LocationEntry { classname: "item_armor_body", priority: 6 },
    LocationEntry { classname: "item_armor_combat", priority: 6 },
    LocationEntry { classname: "item_armor_jacket", priority: 6 },
    LocationEntry { classname: "item_silencer", priority: 7 },
    LocationEntry { classname: "item_breather", priority: 7 },
    LocationEntry { classname: "item_enviro", priority: 7 },
    LocationEntry { classname: "item_adrenaline", priority: 7 },
    LocationEntry { classname: "item_bandolier", priority: 8 },
    LocationEntry { classname: "item_pack", priority: 8 },
];

/// Location candidate.
struct LocationCandidate {
    /// Target.
    target: ActorId,
    /// Priority.
    priority: i32,
    /// Distance.
    distance: f32,
    /// Visible.
    visible: bool,
}

/// Read the held tech (`ctfTech`).
pub fn ctf_tech(game: &mut Q2GameServices, actor: &ActorId) -> Option<Q2CtfTech> {
    TECH_NAMES
        .iter()
        .find(|tech| game.host.inventory().count(actor, &item_id(&format!("q2:{}", tech.classname))) != 0.0)
        .map(|tech| tech.tech)
}

/// CTF presentation (`Q2CtfPresentation`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfPresentation {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
}

impl Q2CtfPresentation {
    /// Show the scoreboard (`scoreboard`).
    pub fn scoreboard(&self, entity: ActorId, game: &mut Q2GameServices) {
        let hooks = self.hooks;
        let rows = |game: &mut Q2GameServices, team: Q2CtfTeam| -> Vec<Q2CtfScoreRow> {
            let mut out = Vec::new();
            for actor in game.host.players() {
                if game.ctf.states.get(&actor).map(|state| state.team) != Some(team) {
                    continue;
                }
                let Some(player) = (hooks.player)(actor.clone(), game) else {
                    continue;
                };
                let (slot, name, score, ping) = (player.slot, player.name.clone(), player.score, player.ping);
                let carried = ctf_carried_flag(game, &actor);
                out.push(Q2CtfScoreRow { slot, name, score, ping: 999.min(ping), carried_flag: carried });
            }
            out.sort_by(|left, right| right.score.cmp(&left.score).then(left.slot.cmp(&right.slot)));
            out
        };
        let red = rows(game, 1);
        let blue = rows(game, 2);
        let spectators = rows(game, 0);
        let totals = [red.iter().map(|row| row.score).sum::<i32>(), blue.iter().map(|row| row.score).sum::<i32>()];
        let (team1, team2) = (game.ctf.match_state.team1, game.ctf.match_state.team2);
        let mut layout = format!(
            "if 24 xv 8 yv 8 pic 24 endif xv 40 yv 28 string \"{:>4}/{:<3}\" xv 98 yv 12 num 2 18 if 25 xv 168 yv 8 pic 25 endif xv 200 yv 28 string \"{:>4}/{:<3}\" xv 256 yv 12 num 2 20 ",
            totals[0],
            red.len(),
            totals[1],
            blue.len()
        );
        let mut append = |text: &str| -> bool {
            if layout.len() + text.len() >= 1000 {
                return false;
            }
            layout.push_str(text);
            true
        };
        let mut red_shown = 0usize;
        let mut blue_shown = 0usize;
        for index in 0..16usize {
            for (rows, x, enemy) in [(&red, 0, 2u8), (&blue, 160, 1u8)] {
                let Some(row) = rows.get(index) else {
                    continue;
                };
                let y = 42 + index * 8;
                let mut text = format!("ctf {x} {y} {} {} {} ", row.slot, row.score, row.ping);
                if row.carried_flag == Some(enemy) {
                    text.push_str(&format!("xv {} yv {y} picn sbfctf{enemy} ", x + 56));
                }
                if append(&text) {
                    if x == 0 {
                        red_shown += 1;
                    } else {
                        blue_shown += 1;
                    }
                }
            }
        }
        let mut y = (red_shown.max(blue_shown) + 1) * 8 + 42;
        if !spectators.is_empty() && append(&format!("xv 0 yv {y} string2 \"Spectators\" ")) {
            y += 8;
            for (index, row) in spectators.iter().enumerate() {
                append(&format!("ctf {} {} {} {} {} ", index % 2 * 160, y + index / 2 * 8, row.slot, row.score, row.ping));
            }
        }
        for (total, shown, x) in [(red.len(), red_shown, 8), (blue.len(), blue_shown, 168)] {
            if total > shown {
                append(&format!("xv {x} yv {} string \"..and {} more\" ", 42 + shown * 8, total - shown));
            }
        }
        (self.hooks.emit)(
            game,
            Q2CtfEvent::Scoreboard { actor: entity, red, blue, spectators, totals, captures: [team1, team2], layout },
        );
    }

    /// Identify the aimed player (`identify`).
    pub fn identify(&self, entity: ActorId, game: &mut Q2GameServices) -> Option<ActorId> {
        let origin = game.body_of(entity.clone()).origin;
        let angles = game.host.player_view_state(&entity).map(|view| view.view_angles).unwrap_or_else(|| game.body_of(entity.clone()).angles);
        let forward = angle_vectors(angles).forward;
        let trace = game.host.trace(&Q2TraceRequest {
            start: origin,
            end: add3(origin, scale3(forward, 1024.0)),
            bounds: None,
            ignore: Some(entity.clone()),
            mask: 3,
            exclude: Vec::new(),
        });
        if let TraceHit::Actor { actor } = &trace.hit {
            if game.host.is_player(actor) {
                return Some(actor.clone());
            }
        }
        let mut best = None;
        let mut alignment = 0.9f32;
        for actor in game.host.players() {
            let Some(other) = game.entity(&actor) else {
                continue;
            };
            if other.actor.id() == &entity || other.solid == Q2Solid::None {
                continue;
            }
            let candidate = dot3(forward, normalize3(sub3(game.body_of(actor.clone()).origin, origin)));
            if candidate > alignment && ctf_can_see(game, &actor, &entity) {
                best = Some(actor.clone());
                alignment = candidate;
            }
        }
        best
    }

    /// Show the hud (`hud`).
    pub fn hud(&self, entity: ActorId, game: &mut Q2GameServices, status: String) {
        let team = ctf_player(game, &entity).team;
        let (team1, team2) = (game.ctf.match_state.team1, game.ctf.match_state.team2);
        let flags = Q2CtfFlags { hooks: self.hooks };
        let red = flags.state(game, 1);
        let blue = flags.state(game, 2);
        let carried = ctf_carried_flag(game, &entity);
        let tech = ctf_tech(game, &entity);
        let id_target = if ctf_player(game, &entity).id_view { self.identify(entity.clone(), game) } else { None };
        let (last_capture, last_team) = (game.ctf.match_state.last_flag_capture, game.ctf.match_state.last_capture_team);
        let blink = (game.now() * 10.0).trunc() as i32 & 8;
        let blink_team = if last_capture.is_some_and(|at| game.now() - at < 5.0) && blink != 0 { last_team } else { None };
        (self.hooks.emit)(
            game,
            Q2CtfEvent::Hud {
                actor: entity,
                captures: [team1, team2],
                flag_states: [red, blue],
                team,
                carried_flag: carried,
                tech,
                id_target,
                blink_team,
                match_text: status,
            },
        );
    }

    /// Describe the location (`location`).
    pub fn location(&self, entity: ActorId, game: &mut Q2GameServices) -> String {
        let origin = game.body_of(entity.clone()).origin;
        let ids: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        let mut candidates = Vec::new();
        for target in ids {
            let classname = game.require_entity(&target).classname.clone();
            let Some(entry) = LOCATIONS.iter().find(|item| item.classname == classname) else {
                continue;
            };
            let distance = length3(sub3(game.body_of(target.clone()).origin, origin));
            if distance > 1024.0 {
                continue;
            }
            let visible = ctf_can_see(game, &target, &entity);
            candidates.push(LocationCandidate { target, priority: entry.priority, distance, visible });
        }
        candidates.sort_by(|left, right| {
            (i32::from(right.visible)).cmp(&i32::from(left.visible)).then(if left.visible {
                left.priority.cmp(&right.priority)
            } else {
                std::cmp::Ordering::Equal
            }).then(left.distance.partial_cmp(&right.distance).unwrap_or(std::cmp::Ordering::Equal))
        });
        let Some(hot) = candidates.into_iter().next().map(|candidate| candidate.target) else {
            return "nowhere".to_string();
        };
        let mut team = String::new();
        let hot_classname = game.require_entity(&hot).classname.clone();
        if game.entities.values().any(|other| other.actor.id() != &hot && other.classname == hot_classname) {
            let flags = Q2CtfFlags { hooks: self.hooks };
            if let (Some(red), Some(blue)) = (flags.base(game, 1), flags.base(game, 2)) {
                let hot_origin = game.body_of(hot.clone()).origin;
                let one = length3(sub3(hot_origin, game.body_of(red).origin));
                let two = length3(sub3(hot_origin, game.body_of(blue).origin));
                team = if one < two { "red " } else if one > two { "blue " } else { "" }.to_string();
            }
        }
        let delta = sub3(origin, game.body_of(hot.clone()).origin);
        let where_ = if delta.z.abs() > delta.x.abs() && delta.z.abs() > delta.y.abs() {
            if delta.z > 0.0 { "above" } else { "below" }
        } else {
            "near"
        };
        let bounds_min_z = game.body_of(entity.clone()).bounds.min.z;
        let water = if game.host.point_contents(add3(origin, vec3(0.0, 0.0, bounds_min_z + 1.0))) & 56 != 0 {
            "in the water "
        } else {
            ""
        };
        let name = self.hooks.items.item_name(game, &hot_classname).unwrap_or(hot_classname);
        format!("{water}{where_} the {team}{name}")
    }

    /// Say to the team (`sayTeam`).
    pub fn say_team(&self, entity: ActorId, game: &mut Q2GameServices, words: &str) {
        if !(self.hooks.chat_allowed)(entity.clone(), game) {
            return;
        }
        let team = ctf_player(game, &entity).team;
        let combat = game.host.combat().read(&entity);
        let health = combat.as_ref().map(|combat| combat.health).unwrap_or(0.0);
        let armor_text = match combat.as_ref() {
            None => "no armor".to_string(),
            Some(combat) => {
                let cells = game.host.inventory().count(&entity, &item_id("q2:ammo_cells"));
                let power = match &combat.armor.powered {
                    PoweredProtectionState::None => String::new(),
                    PoweredProtectionState::Screen { .. } if cells > 0.0 => format!("Power Screen with {cells} cells"),
                    PoweredProtectionState::Shield { .. } if cells > 0.0 => format!("Power Shield with {cells} cells"),
                    _ => String::new(),
                };
                let conventional = match &combat.armor.regular {
                    RegularArmorState::None => String::new(),
                    RegularArmorState::Q2 { points, item, .. } if *points > 0.0 => {
                        format!("{points} units of {}", self.hooks.items.lookup(game, item).map(|found| found.name).unwrap_or_else(|| "armor".to_string()))
                    }
                    RegularArmorState::Q1 { points, .. } | RegularArmorState::Q3 { points, .. } | RegularArmorState::Source { points, .. }
                        if *points > 0.0 =>
                    {
                        format!("{points} armor")
                    }
                    _ => String::new(),
                };
                let parts: Vec<&str> =
                    [power.as_str(), conventional.as_str()].into_iter().filter(|part| !part.is_empty()).collect();
                if parts.is_empty() { "no armor".to_string() } else { parts.join(" and ") }
            }
        };
        let mut names = Vec::new();
        for actor in game.host.players() {
            if actor == entity || game.entity(&actor).is_none() {
                continue;
            }
            if ctf_can_see(game, &actor, &entity) {
                names.push(ctf_name(game, &actor));
            }
        }
        let sight = if names.len() < 2 {
            names.first().cloned().unwrap_or_else(|| "no one".to_string())
        } else {
            format!("{} and {}", names[..names.len() - 1].join(", "), names[names.len() - 1])
        };
        let mut words = words.to_string();
        if words.starts_with('"') {
            words = words[1..].to_string();
            if words.ends_with('"') {
                words.pop();
            }
        }
        let mut text = String::new();
        let mut chars = words.chars();
        while let Some(character) = chars.next() {
            if character != '%' {
                text.push(character);
                continue;
            }
            let Some(code) = chars.next() else {
                text.push('%');
                break;
            };
            match code.to_lowercase().next().unwrap_or(code) {
                'l' => text.push_str(&self.location(entity.clone(), game)),
                'a' => text.push_str(&armor_text),
                'h' => {
                    if health <= 0.0 {
                        text.push_str("dead");
                    } else {
                        text.push_str(&format!("{health} health"));
                    }
                }
                't' => {
                    let tech = ctf_tech(game, &entity);
                    text.push_str(&match TECH_NAMES.iter().find(|entry| Some(entry.tech) == tech) {
                        Some(entry) => format!("the {}", entry.name),
                        None => "no powerup".to_string(),
                    });
                }
                'w' => {
                    let weapon = game.weapons.states.get(&entity).and_then(|state| state.weapon.clone());
                    match weapon.as_deref() {
                        Some(weapon) => {
                            let definition = weapon_definition(game, weapon);
                            text.push_str(&self.hooks.items.lookup(game, &definition.item).map(|found| found.name).unwrap_or_else(|| weapon.to_string()));
                        }
                        None => text.push_str("none"),
                    }
                }
                'n' => text.push_str(&sight),
                other => text.push(other),
            }
        }
        let text: String = text.chars().take(1023).collect();
        let speaker = ctf_name(game, &entity);
        for actor in game.host.players() {
            if game.ctf.states.get(&actor).map(|state| state.team) == Some(team) {
                ctf_print(game, &format!("({speaker}): {text}\n"), Some(actor), Q2CtfPrintLevel::Chat);
            }
        }
    }

    /// Show the join menu (`joinMenu`).
    pub fn join_menu(&self, entity: ActorId, game: &mut Q2GameServices) {
        let phase = game.ctf.match_state.phase;
        let locked = game.ctf.rules.match_lock && (phase == Q2CtfMatchPhase::Pregame || phase == Q2CtfMatchPhase::Game);
        let mut red = 0;
        let mut blue = 0;
        for player in game.ctf.states.values() {
            if player.team == 1 {
                red += 1;
            } else if player.team == 2 {
                blue += 1;
            }
        }
        let force = game.ctf.rules.force_join;
        let competition = game.ctf.rules.competition;
        (self.hooks.emit)(
            game,
            Q2CtfEvent::Menu {
                actor: entity,
                title: "ThreeWave Capture the Flag".to_string(),
                entries: vec![
                    Q2CtfMenuEntry {
                        label: format!("Join Red Team ({red})"),
                        action: if locked || force == Q2CtfForceJoin::Blue { None } else { Some(Q2CtfMenuAction::JoinRed) },
                    },
                    Q2CtfMenuEntry {
                        label: format!("Join Blue Team ({blue})"),
                        action: if locked || force == Q2CtfForceJoin::Red { None } else { Some(Q2CtfMenuAction::JoinBlue) },
                    },
                    Q2CtfMenuEntry { label: "Chase Camera".to_string(), action: Some(Q2CtfMenuAction::Chase) },
                    Q2CtfMenuEntry { label: "Credits".to_string(), action: Some(Q2CtfMenuAction::Credits) },
                    Q2CtfMenuEntry {
                        label: "Request match".to_string(),
                        action: if competition != 0 && phase == Q2CtfMatchPhase::None { Some(Q2CtfMenuAction::Match) } else { None },
                    },
                    Q2CtfMenuEntry { label: "Close".to_string(), action: Some(Q2CtfMenuAction::Close) },
                ],
            },
        );
    }

    /// Show ghost stats (`stats`).
    pub fn stats(&self, entity: ActorId, game: &mut Q2GameServices) {
        let mut text = "Name             Frags Deaths Caps Base Carrier\n".to_string();
        for ghost in game.ctf.match_state.ghosts.values() {
            text.push_str(&format!(
                "{:16} {} {} {} {} {}\n",
                ghost.name.chars().take(16).collect::<String>(),
                ghost.kills,
                ghost.deaths,
                ghost.captures,
                ghost.base_defense,
                ghost.carrier_defense
            ));
        }
        let text: String = text.chars().take(1399).collect();
        ctf_print(game, &text, Some(entity), Q2CtfPrintLevel::High);
    }
}
