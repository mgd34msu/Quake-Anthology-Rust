//! Spawn, match and intermission rules (`src/content/q1/base/rules.ts`).
//!
//! client.qc source spawn, match and intermission rules.
//! GPL-2.0-or-later.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::q1::base::finales::q1_finale_text;
use crate::q1::base::messages::{classic_monster_obituary, classic_obituary_text};
use crate::q1::base::provider::Q1CampaignHandle;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{length, vadd, vscale, vsub, Q1Edition, Q1Event, Q1TraceRequest, Q1Weapon, POINT};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, boolean, int, num, obj, str as save_str, SaveJson, SaveReader};

/// Spawn selection outcome: use a point, deliberately defer admission,
/// or delegate to the next source rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1SpawnDecision {
    /// Spawn at this point.
    Use(ActorId),
    /// Defer admission (the caller retries and forces it later).
    Defer,
    /// Delegate to the next source rule.
    Delegate,
}

/// Source spawn selection (`Q1SpawnSelector["sourceSelections"]`).
pub type Q1SpawnSelection = fn(game: &mut Q1EntityServices, force_spawn: bool) -> Result<Q1SpawnDecision, Q1Error>;

fn entity_classname(game: &Q1EntityServices, id: &ActorId) -> String {
    game.entity_ref(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default()
}

/// Cooperative/deathmatch spawn selection (`Q1SpawnSelector`).
#[derive(Clone)]
pub struct Q1SpawnSelector {
    /// Last selected spawn point.
    pub last_spawn: Option<ActorId>,
    /// Source selection rules in registration order.
    pub source_selections: Vec<(String, Q1SpawnSelection)>,
    campaign: Q1CampaignHandle,
}

impl Q1SpawnSelector {
    /// Fresh selector sharing a campaign binding.
    #[must_use]
    pub fn new(campaign: Q1CampaignHandle) -> Self {
        Self {
            last_spawn: None,
            source_selections: Vec::new(),
            campaign,
        }
    }

    /// Register a source selection rule.
    pub fn register_selection(&mut self, id: &str, select: Q1SpawnSelection) -> Result<(), Q1Error> {
        if self.source_selections.iter().any(|(candidate, _)| candidate == id) {
            return Err(q1_error(format!("Duplicate Q1 source spawn selector {id}")));
        }
        self.source_selections.push((id.to_string(), select));
        Ok(())
    }

    /// Capture the last spawn point.
    #[must_use]
    pub fn capture(&self) -> SaveJson {
        match &self.last_spawn {
            None => SaveJson::Null,
            Some(actor) => obj(vec![
                ("slot", int(i64::from(actor.slot()))),
                ("generation", int(i64::from(actor.generation()))),
            ]),
        }
    }

    /// Restore the last spawn point.
    pub fn restore(&mut self, game: &Q1EntityServices, reader: SaveReader) -> Result<(), Q1Error> {
        self.last_spawn = reader.nullable(|value| {
            let saved = super::provider::saved_actor(value.clone())?;
            let owned = game
                .host
                .actors
                .resolve_saved(&saved)
                .ok_or_else(|| Q1Error::from(value.fail("missing spawn point")))?;
            let id = game
                .entity_ref(owned.id())
                .map(|entity| entity.actor.id().clone())
                .ok_or_else(|| Q1Error::from(value.fail("missing spawn point")))?;
            Ok::<_, Q1Error>(id)
        })?;
        Ok(())
    }

    fn nearby(&self, game: &mut Q1EntityServices, point: &ActorId, radius: f64, living: bool) -> Result<bool, Q1Error> {
        let origin = game.body(point).map(|body| body.origin)?;
        for player in (game.host.players)() {
            if living && game.health(&player) <= 0.0 {
                continue;
            }
            let Some(body) = game.host.bodies.read(&player) else {
                continue;
            };
            let center = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
            if f64::from(length(vsub(center, origin))) <= radius {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn visible(&self, game: &mut Q1EntityServices, point: &ActorId) -> Result<bool, Q1Error> {
        let origin = game.body(point).map(|body| body.origin)?;
        for player in (game.host.players)() {
            if game.health(&player) <= 0.0 {
                continue;
            }
            let Some(body) = game.host.bodies.read(&player) else {
                continue;
            };
            let trace = game.host.trace(&Q1TraceRequest {
                start: vadd(
                    origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 22.0,
                    },
                ),
                end: vadd(
                    body.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 22.0,
                    },
                ),
                bounds: POINT,
                ignore: Some(point.clone()),
                monsters: false,
                missile: false,
            });
            if trace.fraction >= 1.0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Select a spawn point. Rerelease may defer an occupied spawn; the
    /// caller retries and forces it after five seconds.
    pub fn select(&mut self, game: &mut Q1EntityServices, force_spawn: bool) -> Result<Option<ActorId>, Q1Error> {
        let selections: Vec<Q1SpawnSelection> = self.source_selections.iter().map(|(_, select)| *select).collect();
        for select in selections {
            match select(game, force_spawn)? {
                Q1SpawnDecision::Use(point) => return Ok(Some(point)),
                Q1SpawnDecision::Defer => return Ok(None),
                Q1SpawnDecision::Delegate => {}
            }
        }
        let entities = game.entity_ids();
        if let Some(region) = entities
            .iter()
            .find(|id| entity_classname(game, id) == "testplayerstart")
        {
            return Ok(Some(region.clone()));
        }
        if game.options().coop {
            let after = self
                .last_spawn
                .as_ref()
                .and_then(|last| entities.iter().position(|id| id == last))
                .map_or(0, |index| index + 1);
            let point = entities[after..]
                .iter()
                .find(|id| entity_classname(game, id) == "info_player_coop")
                .or_else(|| {
                    entities
                        .iter()
                        .find(|id| entity_classname(game, id) == "info_player_start")
                })
                .cloned();
            if let Some(point) = point {
                self.last_spawn = Some(point.clone());
                return Ok(Some(point));
            }
        } else if game.options().deathmatch != 0 {
            let points: Vec<ActorId> = entities
                .iter()
                .filter(|id| entity_classname(game, id) == "info_player_deathmatch")
                .cloned()
                .collect();
            if points.is_empty() {
                return Err(q1_error("No info_player_deathmatch on level"));
            }
            if game.options().edition == Q1Edition::Classic {
                let start = self
                    .last_spawn
                    .as_ref()
                    .and_then(|last| points.iter().position(|id| id == last))
                    .map_or(-1, |index| index as i32);
                for step in 1..=points.len() {
                    let point = &points[((start + step as i32) as usize) % points.len()];
                    if self.last_spawn.as_ref().is_some_and(|last| last == point)
                        || !self.nearby(game, point, 32.0, false)?
                    {
                        self.last_spawn = Some(point.clone());
                        return Ok(Some(point.clone()));
                    }
                }
                return Ok(force_spawn.then(|| points[0].clone()));
            }
            let mut available: Vec<ActorId> = Vec::new();
            for point in &points {
                if !self.nearby(game, point, 384.0, true)? && !self.visible(game, point)? {
                    available.push(point.clone());
                }
            }
            available.reverse();
            if available.is_empty() {
                for point in &points {
                    if !self.nearby(game, point, 84.0, true)? {
                        available.push(point.clone());
                    }
                }
                available.reverse();
            }
            if available.is_empty() {
                if !force_spawn {
                    return Ok(None);
                }
                let pick = (game.host.random() * (points.len() as f64 - 1.0) + 0.5).floor() as usize;
                return Ok(points.get(pick).cloned());
            }
            let pick = (game.host.random() * (available.len() as f64 - 1.0) + 0.5).floor() as usize;
            return Ok(available.get(pick).cloned());
        }
        if self.campaign.read_flags() != 0 {
            if let Some(returned) = entities
                .iter()
                .find(|id| entity_classname(game, id) == "info_player_start2")
            {
                return Ok(Some(returned.clone()));
            }
        }
        entities
            .iter()
            .find(|id| entity_classname(game, id) == "info_player_start")
            .cloned()
            .map(Some)
            .ok_or_else(|| q1_error("PutClientInServer: no info_player_start on level"))
    }
}

/// Water the victim died in (`Q1ObituaryActor["waterType"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1DeathWater {
    /// Not in liquid.
    Empty,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
}

/// Obituary participant (`Q1ObituaryActor`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ObituaryActor {
    /// Actor id.
    pub actor: ActorId,
    /// Display name.
    pub name: String,
    /// Gameplay classname.
    pub classname: String,
    /// Whether a player.
    pub is_player: bool,
    /// Whether a monster.
    pub is_monster: bool,
    /// Team.
    pub team: i32,
    /// Health.
    pub health: f64,
    /// Water type.
    pub water_type: Q1DeathWater,
    /// Water level.
    pub water_level: i32,
    /// Selected weapon.
    pub weapon: Option<Q1Weapon>,
    /// Quad expiry in seconds.
    pub quad_expires: f64,
    /// Invulnerability expiry in seconds.
    pub invulnerable_expires: f64,
    /// Whether a brush model.
    pub brush: bool,
    /// Kill string.
    pub kill_string: String,
}

/// Obituary message (`Q1Obituary["message"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ObituaryMessage {
    /// Message text.
    pub text: String,
    /// Format arguments.
    pub arguments: Vec<String>,
}

/// Obituary score (`Q1Obituary["score"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ObituaryScore {
    /// Credited actor.
    pub actor: ActorId,
    /// Score delta.
    pub delta: i32,
}

/// Obituary achievement (`Q1Obituary["achievement"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ObituaryAchievement {
    /// Earning actor.
    pub actor: ActorId,
    /// Achievement id.
    pub id: String,
}

/// Accepted-death decision (`Q1Obituary`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1Obituary {
    /// Broadcast message, if any.
    pub message: Option<Q1ObituaryMessage>,
    /// Score change, if any.
    pub score: Option<Q1ObituaryScore>,
    /// Achievement, if any.
    pub achievement: Option<Q1ObituaryAchievement>,
}

/// Obituary inputs (`q1Obituary` input without the random draw).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ObituaryInput {
    /// Source content edition.
    pub edition: Q1Edition,
    /// Victim.
    pub victim: Q1ObituaryActor,
    /// Attacker, if any.
    pub attacker: Option<Q1ObituaryActor>,
    /// Telefrag owner, if any.
    pub telefrag_owner: Option<Q1ObituaryActor>,
    /// Teamplay mode.
    pub teamplay: i32,
    /// Death type.
    pub death_type: String,
}

/// Client lifecycle notice (`Q1ClientNotice`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ClientNotice {
    /// Notice text.
    pub text: String,
    /// Format arguments.
    pub arguments: Vec<String>,
    /// Score delta.
    pub score_delta: i32,
}

/// Client lifecycle event (`q1ClientNotice` event).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1ClientEvent {
    /// Connected.
    Connect,
    /// Disconnected.
    Disconnect,
    /// Suicide.
    Suicide,
    /// Exited the level.
    Exit,
}

/// ClientConnect/ClientDisconnect/ClientKill/changelevel_touch feedback
/// before the session commits lifecycle work (`q1ClientNotice`).
#[must_use]
pub fn q1_client_notice(edition: Q1Edition, event: Q1ClientEvent, name: &str, frags: i32) -> Q1ClientNotice {
    let score_delta = if event == Q1ClientEvent::Suicide { -2 } else { 0 };
    if edition == Q1Edition::Rerelease {
        let text = match event {
            Q1ClientEvent::Connect => "$qc_entered",
            Q1ClientEvent::Disconnect => "$qc_left_game",
            Q1ClientEvent::Suicide => "$qc_suicides",
            Q1ClientEvent::Exit => "$qc_exited",
        };
        let arguments = if event == Q1ClientEvent::Disconnect {
            vec![name.to_string(), frags.to_string()]
        } else {
            vec![name.to_string()]
        };
        return Q1ClientNotice {
            text: text.to_string(),
            arguments,
            score_delta,
        };
    }
    let text = match event {
        Q1ClientEvent::Connect => format!("{name} entered the game\n"),
        Q1ClientEvent::Disconnect => format!("{name} left the game with {frags} frags\n"),
        Q1ClientEvent::Suicide => format!("{name} suicides\n"),
        Q1ClientEvent::Exit => format!("{name} exited the level\n"),
    };
    Q1ClientNotice {
        text,
        arguments: Vec::new(),
        score_delta,
    }
}

/// The selected score authority commits this decision once for an
/// accepted death (`q1Obituary`).
pub fn q1_obituary(input: &Q1ObituaryInput, random: &mut dyn FnMut() -> f64) -> Q1Obituary {
    let none = || Q1Obituary {
        message: None,
        score: None,
        achievement: None,
    };
    if !input.victim.is_player {
        return none();
    }
    let victim_name = input.victim.name.clone();
    let result = |text: &str,
                  credited: &ActorId,
                  delta: i32,
                  args: &[String],
                  achievement: Option<Q1ObituaryAchievement>|
     -> Q1Obituary {
        Q1Obituary {
            message: if text.is_empty() {
                None
            } else if input.edition == Q1Edition::Classic {
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                Some(Q1ObituaryMessage {
                    text: classic_obituary_text(text, &refs),
                    arguments: Vec::new(),
                })
            } else {
                Some(Q1ObituaryMessage {
                    text: text.to_string(),
                    arguments: args.to_vec(),
                })
            },
            score: Some(Q1ObituaryScore {
                actor: credited.clone(),
                delta,
            }),
            achievement,
        }
    };
    let roll = random();
    let victim_args = vec![victim_name.clone()];
    if input
        .attacker
        .as_ref()
        .is_some_and(|attacker| attacker.classname == "teledeath")
    {
        if let Some(owner) = &input.telefrag_owner {
            return result(
                "$qc_telefragged",
                &owner.actor,
                1,
                &[victim_name.clone(), owner.name.clone()],
                None,
            );
        }
    }
    if input
        .attacker
        .as_ref()
        .is_some_and(|attacker| attacker.classname == "teledeath2")
    {
        return result("$qc_satans_power", &input.victim.actor, -1, &victim_args, None);
    }
    if let Some(attacker) = input.attacker.as_ref().filter(|attacker| attacker.is_player) {
        if same_actor(&input.victim.actor, &attacker.actor) {
            if input.victim.weapon == Some(Q1Weapon::Lightning) && input.victim.water_level > 1 {
                let text = match input.victim.water_type {
                    Q1DeathWater::Slime => "$qc_discharge_slime",
                    Q1DeathWater::Lava => "$qc_discharge_lava",
                    _ => "$qc_discharge_water",
                };
                return result(text, &input.victim.actor, -1, &victim_args, None);
            }
            let text = if input.victim.weapon == Some(Q1Weapon::Grenadelauncher) {
                "$qc_suicide_pin"
            } else if roll != 0.0 {
                "$qc_suicide_bored"
            } else {
                "$qc_suicide_loaded"
            };
            return result(text, &input.victim.actor, -1, &victim_args, None);
        }
        if input.teamplay == 2
            && input.victim.team == attacker.team
            && (if input.edition == Q1Edition::Classic {
                input.victim.team > 0
            } else {
                attacker.team != 0
            })
        {
            let text = if roll < 0.25 {
                "$qc_ff_teammate"
            } else if roll < 0.5 {
                "$qc_ff_glasses"
            } else if roll < 0.75 {
                "$qc_ff_otherteam"
            } else {
                "$qc_ff_friend"
            };
            return result(text, &attacker.actor, -1, std::slice::from_ref(&attacker.name), None);
        }
        let pair = vec![victim_name.clone(), attacker.name.clone()];
        let kill = |text: &str, achievement: Option<Q1ObituaryAchievement>| {
            result(text, &attacker.actor, 1, &pair, achievement)
        };
        match attacker.weapon {
            Some(Q1Weapon::Axe) => return kill("$qc_death_ax", None),
            Some(Q1Weapon::Shotgun) => return kill("$qc_death_sg", None),
            Some(Q1Weapon::Supershotgun) => return kill("$qc_death_dbl", None),
            Some(Q1Weapon::Nailgun) => return kill("$qc_death_nail", None),
            Some(Q1Weapon::Supernailgun) => return kill("$qc_death_sng", None),
            Some(Q1Weapon::Grenadelauncher) => {
                return kill(
                    if input.victim.health < -40.0 {
                        "$qc_death_gl1"
                    } else {
                        "$qc_death_gl2"
                    },
                    None,
                );
            }
            Some(Q1Weapon::Rocketlauncher) => {
                if input.edition == Q1Edition::Rerelease && attacker.quad_expires > 0.0 && input.victim.health < -40.0 {
                    let drawn = random();
                    let text = if drawn < 0.3 {
                        "$qc_death_rl_quad1"
                    } else if drawn < 0.6 {
                        "$qc_death_rl_quad2"
                    } else {
                        "$qc_death_rl1"
                    };
                    return kill(text, None);
                }
                return kill(
                    if input.victim.health < -40.0 {
                        "$qc_death_rl2"
                    } else {
                        "$qc_death_rl3"
                    },
                    None,
                );
            }
            Some(Q1Weapon::Lightning) => {
                let achievement = if input.edition == Q1Edition::Rerelease
                    && attacker.water_level > 1
                    && attacker.invulnerable_expires != 0.0
                {
                    Some(Q1ObituaryAchievement {
                        actor: attacker.actor.clone(),
                        id: String::from("ACH_SURVIVE_DISCHARGE"),
                    })
                } else {
                    None
                };
                return kill(
                    if attacker.water_level > 1 {
                        "$qc_death_lg1"
                    } else {
                        "$qc_death_lg2"
                    },
                    achievement,
                );
            }
            _ => return kill(&attacker.kill_string, None),
        }
    }
    if input.edition == Q1Edition::Classic {
        if let Some(attacker) = &input.attacker {
            if attacker.is_monster {
                let text = format!(
                    "{}{}",
                    victim_name,
                    classic_monster_obituary(&attacker.classname).unwrap_or("")
                );
                return result(&text, &input.victim.actor, -1, &victim_args, None);
            }
            let trap = if attacker.classname == "explo_box"
                || attacker.classname == "misc_explobox"
                || attacker.classname == "misc_explobox2"
            {
                Some(" blew up\n")
            } else if attacker.brush && attacker.classname != "worldspawn" {
                Some(" was squished\n")
            } else if attacker.classname == "trap_shooter" || attacker.classname == "trap_spikeshooter" {
                Some(" was spiked\n")
            } else if attacker.classname == "fireball" || attacker.classname == "misc_fireball" {
                Some(" ate a lavaball\n")
            } else if attacker.classname == "trigger_changelevel" {
                Some(" tried to leave\n")
            } else {
                None
            };
            if let Some(suffix) = trap {
                let text = format!("{victim_name}{suffix}");
                return result(&text, &input.victim.actor, -1, &victim_args, None);
            }
        }
    }
    if input.victim.water_type == Q1DeathWater::Water {
        return result(
            if random() < 0.5 {
                "$qc_death_drown1"
            } else {
                "$qc_death_drown2"
            },
            &input.victim.actor,
            -1,
            &victim_args,
            None,
        );
    }
    if input.victim.water_type == Q1DeathWater::Slime {
        return result(
            if random() < 0.5 {
                "$qc_death_slime1"
            } else {
                "$qc_death_slime2"
            },
            &input.victim.actor,
            -1,
            &victim_args,
            None,
        );
    }
    if input.victim.water_type == Q1DeathWater::Lava {
        let text = if input.victim.health < -15.0 {
            "$qc_death_lava1"
        } else if random() < 0.5 {
            "$qc_death_lava2"
        } else {
            "$qc_death_lava3"
        };
        return result(text, &input.victim.actor, -1, &victim_args, None);
    }
    if input
        .attacker
        .as_ref()
        .is_some_and(|attacker| attacker.brush && attacker.classname != "worldspawn")
    {
        return result("$qc_death_squish", &input.victim.actor, -1, &victim_args, None);
    }
    if let Some(attacker) = &input.attacker {
        if !attacker.kill_string.is_empty() {
            return result(&attacker.kill_string, &input.victim.actor, -1, &victim_args, None);
        }
    }
    result(
        if input.death_type == "falling" {
            "$qc_death_fall"
        } else {
            "$qc_death_died"
        },
        &input.victim.actor,
        -1,
        &victim_args,
        None,
    )
}

/// Source finale presentation (`Q1SourceFinale`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1SourceFinale {
    /// Finale text.
    Finale {
        /// Finale text.
        text: String,
        /// CD track.
        track: i32,
    },
    /// Shareware sell screen.
    SellScreen,
}

/// Intermission exit outcome (`Q1IntermissionResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1IntermissionResult {
    /// Keep waiting.
    Waiting,
    /// Travel to a map.
    Travel {
        /// Destination map.
        map: String,
    },
    /// Show finale text.
    Finale {
        /// Finale text.
        text: String,
        /// CD track.
        track: i32,
    },
    /// Show the shareware sell screen.
    SellScreen,
}

/// Source intermission rule decision (`Q1IntermissionRule["finale"]`
/// return: delegate, skip the base text and travel, or present).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1FinaleDecision {
    /// Delegate to the next rule.
    Delegate,
    /// Skip the base text and travel.
    Travel,
    /// Present this finale.
    Finale(Q1SourceFinale),
}

/// Changelevel touch observer.
pub type Q1IntermissionTouch =
    fn(game: &mut Q1EntityServices, trigger: &ActorId, player: &ActorId) -> Result<(), Q1Error>;
/// Intermission begin observer.
pub type Q1IntermissionBegin =
    fn(game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<(), Q1Error>;
/// Finale override.
pub type Q1IntermissionFinale =
    fn(game: &mut Q1EntityServices, stage: i32, next_map: &str) -> Result<Q1FinaleDecision, Q1Error>;
/// Travel override (true when handled).
pub type Q1IntermissionTravel =
    fn(game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<bool, Q1Error>;

/// Source intermission rule (`Q1IntermissionRule`).
#[derive(Debug, Clone)]
pub struct Q1IntermissionRule {
    /// Rule id.
    pub id: String,
    /// Changelevel touch observer.
    pub touch: Option<Q1IntermissionTouch>,
    /// Intermission begin observer.
    pub begin: Option<Q1IntermissionBegin>,
    /// Finale override.
    pub finale: Option<Q1IntermissionFinale>,
    /// Travel override (true when handled).
    pub travel: Option<Q1IntermissionTravel>,
}

/// Per-player level statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1LevelPlayerStats {
    /// Whether the player fired a weapon.
    pub fired_weapon: bool,
    /// Whether the player took damage.
    pub took_damage: bool,
}

/// Level and intermission rules (`Q1LevelRules`).
pub struct Q1LevelRules {
    /// Next map.
    pub next_map: String,
    /// Intermission stage.
    pub stage: i32,
    /// Exit availability in seconds.
    pub exit_after: f64,
    /// Per-player statistics in admission order.
    pub player_stats: Vec<(ActorId, Q1LevelPlayerStats)>,
    /// Source intermission rules in registration order.
    pub source_rules: Vec<(String, Q1IntermissionRule)>,
    campaign: Q1CampaignHandle,
    registered: bool,
    official_campaign: bool,
}

impl Clone for Q1LevelRules {
    fn clone(&self) -> Self {
        Self {
            next_map: self.next_map.clone(),
            stage: self.stage,
            exit_after: self.exit_after,
            player_stats: self.player_stats.clone(),
            source_rules: self.source_rules.clone(),
            campaign: self.campaign.clone(),
            registered: self.registered,
            official_campaign: self.official_campaign,
        }
    }
}

impl Q1LevelRules {
    /// Fresh rules sharing a campaign binding.
    #[must_use]
    pub fn new(campaign: Q1CampaignHandle, registered: bool, official_campaign: bool) -> Self {
        Self {
            next_map: String::new(),
            stage: 0,
            exit_after: 0.0,
            player_stats: Vec::new(),
            source_rules: Vec::new(),
            campaign,
            registered,
            official_campaign,
        }
    }

    /// Whether this is an official campaign.
    #[must_use]
    pub fn official_campaign(&self) -> bool {
        self.official_campaign
    }

    /// Capture rules state.
    #[must_use]
    pub fn capture(&self) -> SaveJson {
        obj(vec![
            ("nextMap", save_str(&self.next_map)),
            ("stage", int(i64::from(self.stage))),
            ("exitAfter", num(self.exit_after)),
            (
                "players",
                arr(self
                    .player_stats
                    .iter()
                    .map(|(actor, stats)| {
                        obj(vec![
                            (
                                "actor",
                                obj(vec![
                                    ("slot", int(i64::from(actor.slot()))),
                                    ("generation", int(i64::from(actor.generation()))),
                                ]),
                            ),
                            ("firedWeapon", boolean(stats.fired_weapon)),
                            ("tookDamage", boolean(stats.took_damage)),
                        ])
                    })
                    .collect()),
            ),
        ])
    }

    /// Restore rules state.
    pub fn restore(&mut self, game: &Q1EntityServices, reader: SaveReader) -> Result<(), Q1Error> {
        self.next_map = reader.field("nextMap").string()?;
        self.stage = i32::try_from(reader.field("stage").integer(0)?)
            .map_err(|_| reader.field("stage").fail("stage out of range"))?;
        self.exit_after = reader.field("exitAfter").number()?;
        self.player_stats.clear();
        for stats in reader.field("players").list(|value| {
            let saved = super::provider::saved_actor(value.field("actor"))?;
            let actor = game
                .host
                .actors
                .resolve_saved(&saved)
                .ok_or_else(|| Q1Error::from(value.fail("missing level player")))?;
            Ok::<_, Q1Error>((
                actor.id().clone(),
                Q1LevelPlayerStats {
                    fired_weapon: value.field("firedWeapon").boolean()?,
                    took_damage: value.field("tookDamage").boolean()?,
                },
            ))
        })? {
            self.player_stats.push(stats);
        }
        Ok(())
    }

    /// Reset one player's statistics.
    pub fn reset_player(&mut self, actor: &ActorId) {
        let stats = Q1LevelPlayerStats {
            fired_weapon: false,
            took_damage: false,
        };
        match self.player_stats.iter_mut().find(|(candidate, _)| candidate == actor) {
            Some((_, current)) => *current = stats,
            None => self.player_stats.push((actor.clone(), stats)),
        }
    }

    /// Register a source intermission rule.
    pub fn register_intermission_rule(&mut self, rule: Q1IntermissionRule) -> Result<(), Q1Error> {
        if self.source_rules.iter().any(|(candidate, _)| candidate == &rule.id) {
            return Err(q1_error(format!("Duplicate Q1 intermission rule {}", rule.id)));
        }
        self.source_rules.push((rule.id.clone(), rule));
        Ok(())
    }

    /// Notify rules of a changelevel touch.
    pub fn changelevel_touched(
        &self,
        game: &mut Q1EntityServices,
        trigger: &ActorId,
        player: &ActorId,
    ) -> Result<(), Q1Error> {
        let rules: Vec<Q1IntermissionRule> = self.source_rules.iter().map(|(_, rule)| rule.clone()).collect();
        for rule in rules {
            if let Some(touch) = rule.touch {
                touch(game, trigger, player)?;
            }
        }
        Ok(())
    }

    /// Travel, letting source rules override the destination.
    pub fn travel_to(&self, game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<(), Q1Error> {
        let rules: Vec<Q1IntermissionRule> = self.source_rules.iter().map(|(_, rule)| rule.clone()).collect();
        for rule in rules {
            if let Some(travel) = rule.travel {
                if travel(game, map, cause)? {
                    return Ok(());
                }
            }
        }
        game.travel(map, cause);
        Ok(())
    }

    /// Note a weapon attack (axe-only attacks do not count).
    pub fn note_attack(&mut self, actor: &ActorId, axe_only: bool) {
        if axe_only {
            return;
        }
        match self.player_stats.iter_mut().find(|(candidate, _)| candidate == actor) {
            Some((_, stats)) => stats.fired_weapon = true,
            None => self.player_stats.push((
                actor.clone(),
                Q1LevelPlayerStats {
                    fired_weapon: true,
                    took_damage: false,
                },
            )),
        }
    }

    /// Note health damage.
    pub fn note_damage(&mut self, actor: &ActorId, health_damage: f64) {
        if health_damage == 0.0 {
            return;
        }
        match self.player_stats.iter_mut().find(|(candidate, _)| candidate == actor) {
            Some((_, stats)) => stats.took_damage = true,
            None => self.player_stats.push((
                actor.clone(),
                Q1LevelPlayerStats {
                    fired_weapon: false,
                    took_damage: true,
                },
            )),
        }
    }

    /// Begin an intermission to a map.
    pub fn begin(&mut self, game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<(), Q1Error> {
        self.next_map = map.to_string();
        self.stage = 1;
        self.exit_after = game.time + if game.options().deathmatch != 0 { 5.0 } else { 2.0 };
        let rules: Vec<Q1IntermissionRule> = self.source_rules.iter().map(|(_, rule)| rule.clone()).collect();
        for rule in rules {
            if let Some(begin) = rule.begin {
                begin(game, map, cause)?;
            }
        }
        game.begin_intermission(map, cause)?;
        if game.options().edition == Q1Edition::Rerelease {
            if game.options().skill == 3 {
                for actor_id in (game.host.players)() {
                    let owned = game.host.actors.resolve_owned(&actor_id);
                    if owned.is_none() {
                        continue;
                    }
                    let stats = self
                        .player_stats
                        .iter()
                        .find(|(candidate, _)| candidate == &actor_id)
                        .map(|(_, stats)| *stats);
                    if game.map_name == "e1m1" && !stats.is_some_and(|stats| stats.fired_weapon) {
                        game.host.emit(Q1Event::Achievement {
                            player: Some(actor_id.clone()),
                            id: String::from("ACH_PACIFIST"),
                        });
                    }
                    if game.map_name == "e4m6" && !stats.is_some_and(|stats| stats.took_damage) {
                        game.host.emit(Q1Event::Achievement {
                            player: Some(actor_id),
                            id: String::from("ACH_PAINLESS_MAZE"),
                        });
                    }
                }
            }
            let completed =
                if self.official_campaign && ["e1m7", "e2m6", "e3m6", "e4m7"].contains(&game.map_name.as_str()) {
                    Some(format!("ACH_COMPLETE_{}", game.map_name.to_uppercase()))
                } else {
                    None
                };
            if let Some(id) = completed {
                game.host.emit(Q1Event::Achievement { player: None, id });
            }
            let secret = game.map_name == "e1m4" && map == "e1m8"
                || game.map_name == "e2m3" && map == "e2m7"
                || game.map_name == "e3m4" && map == "e3m7"
                || game.map_name == "e4m5" && map == "e4m8";
            if secret {
                game.host.emit(Q1Event::Achievement {
                    player: None,
                    id: format!("ACH_FIND_{}", map.to_uppercase()),
                });
            }
        }
        Ok(())
    }

    /// Begin a cutscene intermission without engine presentation.
    pub fn begin_cutscene(&mut self, game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>, exit_after: f64) {
        self.next_map = map.to_string();
        self.stage = 1;
        self.exit_after = exit_after;
        game.intermission = Some(crate::q1::foundation::entity_services::Q1Intermission {
            map: map.to_string(),
            cause: cause.cloned(),
            exit_after,
        });
    }

    /// Check deathmatch time and frag limits, scheduling the next level.
    pub fn check_limits(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
        scores: &[f64],
        timelimit_minutes: f64,
        fraglimit: f64,
    ) -> Result<bool, Q1Error> {
        if !self.next_map.is_empty() || timelimit_minutes == 0.0 && fraglimit == 0.0 {
            return Ok(false);
        }
        if !(timelimit_minutes != 0.0 && seconds >= timelimit_minutes * 60.0)
            && !(fraglimit != 0.0 && scores.iter().any(|score| *score >= fraglimit))
        {
            return Ok(false);
        }
        game.time = seconds;
        let mut next = game.map_name.clone();
        if next == "start" {
            let flags = self.campaign.read_flags();
            if !self.registered {
                next = String::from("e1m1");
            } else if flags & 1 == 0 {
                next = String::from("e1m1");
                self.campaign.write_flags(flags | 1);
            } else if flags & 2 == 0 {
                next = String::from("e2m1");
                self.campaign.write_flags(flags | 2);
            } else if flags & 4 == 0 {
                next = String::from("e3m1");
                self.campaign.write_flags(flags | 4);
            } else if flags & 8 == 0 {
                next = String::from("e4m1");
                self.campaign.write_flags(flags - 7);
            }
        } else {
            let trigger = game.entity_ids().into_iter().find(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.classname == "trigger_changelevel")
            });
            if let Some(trigger) = trigger {
                let map = game
                    .entity_ref(&trigger)
                    .map(|entity| entity.text("map"))
                    .unwrap_or_default();
                if !map.is_empty() {
                    next = map;
                }
            }
        }
        self.next_map = next;
        let timer = game.create("nextlevel", None, None)?;
        game.schedule(&timer, 0.1, "base:next_level")?;
        Ok(true)
    }

    /// Request an intermission exit.
    pub fn request_exit(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
        pressed: bool,
        same_level: bool,
    ) -> Result<Q1IntermissionResult, Q1Error> {
        if self.stage == 0 || !pressed || seconds < self.exit_after {
            return Ok(Q1IntermissionResult::Waiting);
        }
        if game.options().deathmatch != 0 {
            return self.travel(game, seconds, same_level);
        }
        self.exit_after = seconds + 1.0;
        self.stage += 1;
        let rules: Vec<Q1IntermissionRule> = self.source_rules.iter().map(|(_, rule)| rule.clone()).collect();
        for rule in rules {
            if let Some(finale) = rule.finale {
                match finale(game, self.stage, &self.next_map.clone())? {
                    Q1FinaleDecision::Delegate => {}
                    Q1FinaleDecision::Travel => return self.travel(game, seconds, same_level),
                    Q1FinaleDecision::Finale(Q1SourceFinale::Finale { text, track }) => {
                        return Ok(Q1IntermissionResult::Finale { text, track });
                    }
                    Q1FinaleDecision::Finale(Q1SourceFinale::SellScreen) => {
                        return Ok(Q1IntermissionResult::SellScreen)
                    }
                }
            }
        }
        if self.stage == 2 {
            let text = match game.map_name.as_str() {
                "e1m7" => Some(if self.registered {
                    "$qc_finale_e1"
                } else {
                    "$qc_finale_e1_shareware"
                }),
                "e2m6" => Some("$qc_finale_e2"),
                "e3m6" => Some("$qc_finale_e3"),
                "e4m7" => Some("$qc_finale_e4"),
                _ => None,
            };
            if let Some(key) = text {
                return Ok(Q1IntermissionResult::Finale {
                    text: q1_finale_text(game.options().edition, key),
                    track: 2,
                });
            }
            return self.travel(game, seconds, same_level);
        }
        if self.stage == 3 {
            if !self.registered {
                return Ok(Q1IntermissionResult::SellScreen);
            }
            if self.campaign.read_flags() & 15 == 15 {
                return Ok(Q1IntermissionResult::Finale {
                    text: q1_finale_text(game.options().edition, "$qc_finale_all_runes"),
                    track: 2,
                });
            }
        }
        self.travel(game, seconds, same_level)
    }

    fn travel(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
        same_level: bool,
    ) -> Result<Q1IntermissionResult, Q1Error> {
        let map = if same_level {
            game.map_name.clone()
        } else {
            self.next_map.clone()
        };
        let cause = game
            .intermission
            .as_ref()
            .and_then(|intermission| intermission.cause.clone());
        game.time = seconds;
        game.intermission = None;
        self.travel_to(game, &map.clone(), cause.as_ref())?;
        self.stage = 0;
        Ok(Q1IntermissionResult::Travel { map })
    }

    /// Exit immediately when a client connects mid-intermission.
    pub fn client_connected(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
        same_level: bool,
    ) -> Result<Q1IntermissionResult, Q1Error> {
        if self.stage == 0 {
            return Ok(Q1IntermissionResult::Waiting);
        }
        self.exit_after = seconds;
        self.request_exit(game, seconds, true, same_level)
    }

    /// Advance a finale exactly like a fresh client connection.
    pub fn advance_finale(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
    ) -> Result<Q1IntermissionResult, Q1Error> {
        self.client_connected(game, seconds, false)
    }

    /// Defer the intermission exit.
    pub fn defer_exit(&mut self, game: &mut Q1EntityServices, until_seconds: f64) {
        self.exit_after = until_seconds;
        if let Some(intermission) = game.intermission.clone() {
            game.intermission = Some(crate::q1::foundation::entity_services::Q1Intermission {
                exit_after: until_seconds,
                ..intermission
            });
        }
    }
}

/// Register the delayed next-level callback (`base:next_level`).
pub fn register_level_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "base:next_level",
        crate::q1::foundation::callbacks::Q1CallbackHandlers {
            action: Some(next_level),
            ..Default::default()
        },
    )
}

fn next_level(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let next = super::provider::update_base(game, |state| state.level_rules.next_map.clone())?;
    super::provider::level_begin(game, &next, None)?;
    game.remove(&id)
}

#[cfg(test)]
mod tests {
    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::*;
    use crate::q1::base::provider::{Q1BaseOptions, Q1CampaignState};
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn game() -> (
        Q1EntityServices,
        std::rc::Rc<std::cell::RefCell<crate::q1::foundation::host::mock::MockEvents>>,
    ) {
        let (host, events) = mock_host();
        (Q1EntityServices::new(host, options()).expect("game"), events)
    }

    fn actor(owner: &IdentityOwner, slot: u32) -> ActorId {
        owner.actor(slot, 0)
    }

    fn victim(owner: &IdentityOwner) -> Q1ObituaryActor {
        Q1ObituaryActor {
            actor: actor(owner, 1),
            name: String::from("Victim"),
            classname: String::from("player"),
            is_player: true,
            is_monster: false,
            team: 0,
            health: 0.0,
            water_type: Q1DeathWater::Empty,
            water_level: 0,
            weapon: Some(Q1Weapon::Shotgun),
            quad_expires: 0.0,
            invulnerable_expires: 0.0,
            brush: false,
            kill_string: String::new(),
        }
    }

    fn attacker(owner: &IdentityOwner, weapon: Option<Q1Weapon>) -> Q1ObituaryActor {
        Q1ObituaryActor {
            actor: actor(owner, 2),
            name: String::from("Killer"),
            classname: String::from("player"),
            is_player: true,
            is_monster: false,
            team: 0,
            health: 100.0,
            water_type: Q1DeathWater::Empty,
            water_level: 0,
            weapon,
            quad_expires: 0.0,
            invulnerable_expires: 0.0,
            brush: false,
            kill_string: String::from("$qc_ks_custom"),
        }
    }

    #[test]
    fn obituary_covers_kills_and_suicides() {
        let owner = IdentityOwner::create("test").expect("owner");
        let input = Q1ObituaryInput {
            edition: Q1Edition::Rerelease,
            victim: victim(&owner),
            attacker: Some(attacker(&owner, Some(Q1Weapon::Axe))),
            telefrag_owner: None,
            teamplay: 0,
            death_type: String::new(),
        };
        let kill = q1_obituary(&input, &mut || 0.5);
        assert_eq!(
            kill.message.as_ref().map(|message| message.text.as_str()),
            Some("$qc_death_ax")
        );
        assert_eq!(
            kill.score.as_ref().map(|score| (score.delta, score.actor.slot())),
            Some((1, 2))
        );

        let suicide = Q1ObituaryInput {
            victim: Q1ObituaryActor {
                weapon: Some(Q1Weapon::Grenadelauncher),
                ..victim(&owner)
            },
            attacker: Some(Q1ObituaryActor {
                actor: actor(&owner, 1),
                ..attacker(&owner, Some(Q1Weapon::Grenadelauncher))
            }),
            ..input.clone()
        };
        let pin = q1_obituary(&suicide, &mut || 0.5);
        assert_eq!(
            pin.message.as_ref().map(|message| message.text.as_str()),
            Some("$qc_suicide_pin")
        );
        assert_eq!(pin.score.as_ref().map(|score| score.delta), Some(-1));

        let classic = Q1ObituaryInput {
            edition: Q1Edition::Classic,
            attacker: Some(Q1ObituaryActor {
                classname: String::from("monster_dog"),
                is_player: false,
                is_monster: true,
                ..attacker(&owner, None)
            }),
            ..input.clone()
        };
        let mauled = q1_obituary(&classic, &mut || 0.5);
        assert_eq!(
            mauled.message.as_ref().map(|message| message.text.as_str()),
            Some("Victim was mauled by a Rottweiler\n")
        );

        let drowned = Q1ObituaryInput {
            victim: Q1ObituaryActor {
                water_type: Q1DeathWater::Water,
                ..victim(&owner)
            },
            attacker: None,
            ..input.clone()
        };
        let drown = q1_obituary(&drowned, &mut || 0.1);
        assert_eq!(
            drown.message.as_ref().map(|message| message.text.as_str()),
            Some("$qc_death_drown1")
        );

        let fell = Q1ObituaryInput {
            death_type: String::from("falling"),
            attacker: None,
            ..input.clone()
        };
        let fall = q1_obituary(&fell, &mut || 0.5);
        assert_eq!(
            fall.message.as_ref().map(|message| message.text.as_str()),
            Some("$qc_death_fall")
        );

        let notice = q1_client_notice(Q1Edition::Rerelease, Q1ClientEvent::Disconnect, "Ann", 4);
        assert_eq!((notice.text.as_str(), notice.score_delta), ("$qc_left_game", 0));
        assert_eq!(notice.arguments, vec![String::from("Ann"), String::from("4")]);
        let classic_notice = q1_client_notice(Q1Edition::Classic, Q1ClientEvent::Suicide, "Bob", 0);
        assert_eq!(
            (classic_notice.text.as_str(), classic_notice.score_delta),
            ("Bob suicides\n", -2)
        );
    }

    #[test]
    fn spawn_selector_walks_coop_and_errors_without_start() {
        let (mut game, _) = game();
        let campaign = Q1CampaignHandle::new(Box::new(Q1CampaignState::new(0, 1)));
        let mut selector = Q1SpawnSelector::new(campaign);
        assert!(selector.select(&mut game, false).is_err());

        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(
            host,
            Q1FoundationOptions {
                coop: true,
                ..options()
            },
        )
        .expect("game");
        let first = game.create("info_player_coop", None, None).expect("coop");
        let second = game.create("info_player_coop", None, None).expect("coop");
        let campaign = Q1CampaignHandle::new(Box::new(Q1CampaignState::new(0, 1)));
        let mut selector = Q1SpawnSelector::new(campaign);
        assert_eq!(selector.select(&mut game, false).expect("first"), Some(first));
        assert_eq!(selector.select(&mut game, false).expect("second"), Some(second));
    }

    #[test]
    fn level_rules_travel_and_finale() {
        let (mut game2, _) = game();
        super::super::provider::register_q1_base(&mut game2, Q1BaseOptions::default()).expect("base");
        let spot = game2.create("info_intermission", None, None).expect("spot");
        game2
            .update_entity(&spot, |entity| entity.targetname = String::from("info_intermission"))
            .expect("name");
        super::super::provider::level_begin(&mut game2, "e1m2", None).expect("begin");
        let (next, stage) = super::super::provider::update_base(&game2, |state| {
            (state.level_rules.next_map.clone(), state.level_rules.stage)
        })
        .expect("state");
        assert_eq!((next.as_str(), stage), ("e1m2", 1));
        let waiting = super::super::provider::level_request_exit(&mut game2, 0.5, true, false).expect("exit");
        assert_eq!(waiting, Q1IntermissionResult::Waiting);
        let travel = super::super::provider::level_request_exit(&mut game2, 5.0, true, false).expect("exit");
        assert_eq!(
            travel,
            Q1IntermissionResult::Travel {
                map: String::from("e1m2")
            }
        );

        let (mut game2, _) = game();
        super::super::provider::register_q1_base(&mut game2, Q1BaseOptions::default()).expect("base");
        let spot = game2.create("info_intermission", None, None).expect("spot");
        game2
            .update_entity(&spot, |entity| entity.targetname = String::from("info_intermission"))
            .expect("name");
        game2.map_name = String::from("e1m7");
        super::super::provider::level_begin(&mut game2, "e1m8", None).expect("begin");
        let finale = super::super::provider::level_request_exit(&mut game2, 5.0, true, false).expect("exit");
        assert!(matches!(finale, Q1IntermissionResult::Finale { track: 2, .. }));
    }
}
