//! Rogue teamplay and capture-the-flag (`src/content/q1/missionpacks/world/rogue-teams.ts`).
//!
//! teamplay.qc flag and team rules.
//!
//! The Rust services default `base_team_health` to disabled, which is the
//! state the donor's constructor selects, so no setter call is needed here.

use qa_core::identity::{same_actor, ActorId};
use qa_core::math::Vec3;

use crate::contract::ItemId;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, DamageRequest, Q1DamageSourceEffects, TouchSurface};
use crate::q1::foundation::spawns::spawn_map_actor;
use crate::q1::foundation::types::{length, vadd, vscale, vsub, Q1Event, Q1MessageArg, Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};

use super::common::{brush, later, number, vector};
use super::with_missionpack_hooks;

/// Carried flag bounds (`flagBounds`).
const FLAG_BOUNDS: qa_core::math::Bounds = qa_core::math::Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: 0.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 74.0,
    },
};

/// Team name (`teamName`).
fn team_name(team: i32) -> &'static str {
    if team == 5 {
        "Red"
    } else if team == 14 {
        "Blue"
    } else if team == 1 {
        "Grey"
    } else {
        "UNKNOWN"
    }
}

/// Teamplay mode (`mode`).
fn teamplay_mode(game: &Q1EntityServices) -> i32 {
    game.options().teamplay.unwrap_or(0)
}

/// Whether the mode plays capture-the-flag (`ctf`).
fn is_ctf(game: &Q1EntityServices) -> bool {
    matches!(teamplay_mode(game), 4..=6)
}

/// Read the Rogue gamecfg flags (0 when the session provides none).
fn gamecfg(game: &mut Q1EntityServices) -> Result<i32, Q1Error> {
    with_missionpack_hooks(game, |_, hooks| {
        Ok(hooks.gamecfg.as_ref().map(|gamecfg| gamecfg()).unwrap_or(0))
    })
}

/// Inert team damage effects. The donor `armorAllowed`/`beforeHealth`
/// logic lives in the gameful [`RogueTeams::armor_allowed`]/
/// [`RogueTeams::before_health`], which the session damage pipeline calls
/// with game access.
fn team_damage_effects() -> Q1DamageSourceEffects {
    Q1DamageSourceEffects {
        before_quad: None,
        after_quad: None,
        armor_allowed: None,
        protection_applies: None,
        before_health: None,
        after_armor: None,
        lethal_health: None,
    }
}

/// Rogue team services (`RogueTeams`).
pub struct RogueTeams;

impl RogueTeams {
    /// Register team entities and rules (`constructor`).
    pub fn new(game: &mut Q1EntityServices) -> Result<Self, Q1Error> {
        game.register_damage_source_effects("rogue:teams", team_damage_effects())?;
        game.named.register(
            "rogue:flag_place",
            Q1CallbackHandlers {
                action: Some(flag_place),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:flag_think",
            Q1CallbackHandlers {
                action: Some(flag_think_action),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:flag_touch",
            Q1CallbackHandlers {
                touch: Some(flag_touch),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:flagbase_touch",
            Q1CallbackHandlers {
                touch: Some(flagbase_touch),
                ..Default::default()
            },
        )?;
        game.register_spawn("item_flag_team1", spawn_flag_team1)?;
        game.register_spawn("item_flag_team2", spawn_flag_team2)?;
        game.register_spawn("item_flag", spawn_flag_solo)?;
        game.register_spawn("info_player_team1", spawn_team_start)?;
        game.register_spawn("info_player_team2", spawn_team_start)?;
        game.register_spawn("func_ctf_wall", spawn_ctf_wall)?;
        game.register_spawn("trigger_teleport", spawn_teleport)?;
        Ok(Self)
    }

    /// Teamplay mode (`mode`).
    #[must_use]
    pub fn mode(&self, game: &Q1EntityServices) -> i32 {
        teamplay_mode(game)
    }

    /// Whether the mode plays capture-the-flag (`ctf`).
    #[must_use]
    pub fn ctf(&self, game: &Q1EntityServices) -> bool {
        is_ctf(game)
    }

    /// Resolve a player's team state entity, creating it (`state`).
    fn state(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<ActorId, Q1Error> {
        if let Some(found) = game.entity_ids().into_iter().find(|id| {
            game.entity(id).is_some_and(|entity| {
                entity.classname == "rogue_team_state"
                    && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, actor))
            })
        }) {
            return Ok(found);
        }
        let state = game.create("rogue_team_state", None, None)?;
        let owner = actor.clone();
        game.update_entity(&state, |state| state.owner = Some(owner))?;
        let steam = if is_ctf(game) && gamecfg(game)? & 8 == 0 {
            -1.0
        } else {
            f64::from(self.color(game, actor)?)
        };
        game.update_entity(&state, |state| number(state, "steam", steam))
            .map(|()| state)
    }

    /// Read a player's team color (`color`).
    fn color(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<i32, Q1Error> {
        with_missionpack_hooks(game, |game, hooks| {
            let Some(team_color) = hooks.team_color.as_ref() else {
                if teamplay_mode(game) <= 0 {
                    return Ok(0);
                }
                return Err(q1_error("Rogue teamplay requires shared player colors"));
            };
            Ok(team_color(actor))
        })
    }

    /// Read a player's team (`team`).
    pub fn team(&self, game: &mut Q1EntityServices, actor: Option<&ActorId>) -> Result<i32, Q1Error> {
        let Some(actor) = actor else {
            return Ok(0);
        };
        if !game.is_player(actor) {
            return Ok(0);
        }
        let state = self.state(game, actor)?;
        Ok(game
            .entity(&state)
            .map(|state| state.number("steam") as i32)
            .unwrap_or(0))
    }

    /// Write a player's team color (`setColor`).
    fn set_color(&self, game: &mut Q1EntityServices, actor: &ActorId, team: i32) -> Result<(), Q1Error> {
        with_missionpack_hooks(game, |_, hooks| {
            let Some(set_color) = hooks.set_team_color.as_ref() else {
                return Err(q1_error("Rogue CTF requires shared color mutation"));
            };
            set_color(actor, team);
            Ok(())
        })
    }

    /// Adjust a player's score (`score`).
    fn score(&self, game: &mut Q1EntityServices, actor: &ActorId, delta: i32) -> Result<(), Q1Error> {
        with_missionpack_hooks(game, |_, hooks| {
            let Some(add_frags) = hooks.add_frags.as_ref() else {
                return Err(q1_error("Rogue CTF requires shared score authority"));
            };
            add_frags(actor, delta);
            Ok(())
        })
    }

    /// Read a player's name (`name`).
    fn name(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<String, Q1Error> {
        with_missionpack_hooks(game, |_, hooks| {
            let Some(player_name) = hooks.player_name.as_ref() else {
                return Err(q1_error("Rogue CTF requires player names"));
            };
            Ok(player_name(actor))
        })
    }

    /// Message one player (`message`).
    fn message(&self, game: &mut Q1EntityServices, actor: &ActorId, text: &str, center: bool, args: Vec<Q1MessageArg>) {
        game.host.emit(Q1Event::Message {
            player: actor.clone(),
            text: text.to_string(),
            center,
            args: Some(args),
            parts: None,
        });
    }

    /// Broadcast to every player (`broadcast`).
    fn broadcast(&self, game: &mut Q1EntityServices, text: &str) {
        for actor in (game.host.players)() {
            self.message(game, &actor, text, false, Vec::new());
        }
    }

    /// Play a team sound for a player (`sound`).
    fn sound(&self, game: &mut Q1EntityServices, actor: &ActorId, path: &str, global: bool) -> Result<(), Q1Error> {
        if game.host.actors.resolve_owned(actor).is_none() {
            return Ok(());
        }
        if global {
            game.sound(
                actor,
                path,
                crate::q1::foundation::types::Q1SoundChannel::Voice,
                0.0,
                1.0,
            )
        } else {
            game.sound(
                actor,
                path,
                crate::q1::foundation::types::Q1SoundChannel::Item,
                1.0,
                1.0,
            )
        }
    }

    /// Whether a team id is legal in this mode (`legal`).
    fn legal(&self, game: &Q1EntityServices, team: i32) -> bool {
        let mode = teamplay_mode(game);
        if mode < 4 {
            return team > 0;
        }
        if is_ctf(game) {
            return team == 5 || team == 14 || mode == 6 && team == 1;
        }
        true
    }

    /// Assign a spawned player to a team (`playerSpawned`).
    pub fn player_spawned(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        let state = self.state(game, actor)?;
        let steam = game.entity(&state).map(|state| state.number("steam")).unwrap_or(0.0);
        if steam >= 0.0 || teamplay_mode(game) < 4 {
            let color = self.color(game, actor)?;
            if self.legal(game, color) {
                let color = f64::from(color);
                return game.update_entity(&state, |state| number(state, "steam", color));
            }
        }
        let mut red = 0;
        let mut blue = 0;
        let mut grey = 0;
        for player in (game.host.players)() {
            if same_actor(actor, &player) {
                continue;
            }
            match self.team(game, Some(&player))? {
                5 => red += 1,
                14 => blue += 1,
                1 => grey += 1,
                _ => {}
            }
        }
        let mut team = 5;
        let mut count = red;
        if blue < count || blue == count && game.host.random() < 0.5 {
            team = 14;
            count = blue;
        }
        if teamplay_mode(game) == 6 && grey * 2 < count {
            team = 1;
        }
        game.update_entity(&state, |state| {
            number(state, "steam", f64::from(team));
            number(state, "ctf_flags", (state.number("ctf_flags") as i32 | 4) as f64);
        })?;
        self.message(
            game,
            actor,
            &format!("You have been assigned to the {} team.\n", team_name(team)),
            false,
            Vec::new(),
        );
        self.set_color(game, actor, team)
    }

    /// Police team colors every frame (`frame`).
    pub fn frame(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        let state = self.state(game, actor)?;
        let color = self.color(game, actor)?;
        self.check_update(game)?;
        if game.options().deathmatch == 0 || teamplay_mode(game) < 4 {
            let color = f64::from(color);
            return game.update_entity(&state, |state| number(state, "steam", color));
        }
        if game
            .entity(&state)
            .map(|state| state.number("ctf_flags"))
            .unwrap_or(0.0) as i32
            & 4
            != 0
        {
            game.update_entity(&state, |state| {
                number(state, "ctf_flags", (state.number("ctf_flags") as i32 & !4) as f64);
            })?;
            let steam = game
                .entity(&state)
                .map(|state| state.number("steam") as i32)
                .unwrap_or(0);
            return self.set_color(game, actor, steam);
        }
        let steam = game
            .entity(&state)
            .map(|state| state.number("steam") as i32)
            .unwrap_or(0);
        if !self.legal(game, color) && color == steam {
            game.update_entity(&state, |state| number(state, "steam", -1.0))?;
        }
        let steam = game
            .entity(&state)
            .map(|state| state.number("steam") as i32)
            .unwrap_or(0);
        if color == steam {
            return Ok(());
        }
        let previous = steam;
        let changes = gamecfg(game)? & 16 != 0;
        if previous >= 0 && self.legal(game, previous) && !changes {
            if game
                .entity(&state)
                .map(|state| state.number("suicide_count"))
                .unwrap_or(0.0)
                > 3.0
            {
                self.message(game, actor, "$qc_color_games", false, Vec::new());
                with_missionpack_hooks(game, |_, hooks| {
                    let Some(disconnect) = hooks.disconnect.as_ref() else {
                        return Err(q1_error("Rogue team enforcement requires shared disconnect"));
                    };
                    disconnect(actor);
                    Ok(())
                })?;
            }
            if game
                .entity(&state)
                .map(|state| state.number("ctf_killed"))
                .unwrap_or(0.0)
                != 1.0
            {
                game.update_entity(&state, |state| number(state, "ctf_killed", 2.0))?;
            }
            let actor_copy = actor.clone();
            game.damage(
                actor,
                Some(&actor_copy),
                Some(&actor_copy),
                1000.0,
                &crate::q1::foundation::entity_services::Q1DamageParams::default(),
            );
            let suicides = game
                .entity(&state)
                .map(|state| state.number("suicide_count"))
                .unwrap_or(0.0)
                + 1.0;
            game.update_entity(&state, |state| number(state, "suicide_count", suicides))?;
            self.message(game, actor, "$qc_cannot_change_teams", false, Vec::new());
            return self.set_color(game, actor, previous);
        }
        if previous >= 0 && !self.legal(game, previous) {
            game.update_entity(&state, |state| number(state, "steam", -50.0))?;
        }
        if game.entity(&state).map(|state| state.number("steam")).unwrap_or(0.0) > 0.0 {
            if game
                .entity(&state)
                .map(|state| state.number("ctf_killed"))
                .unwrap_or(0.0)
                != 1.0
            {
                game.update_entity(&state, |state| number(state, "ctf_killed", 2.0))?;
            }
            let actor_copy = actor.clone();
            game.damage(
                actor,
                Some(&actor_copy),
                Some(&actor_copy),
                1000.0,
                &crate::q1::foundation::entity_services::Q1DamageParams::default(),
            );
        }
        let frags = with_missionpack_hooks(game, |_, hooks| {
            hooks
                .frags
                .as_ref()
                .map(|frags| frags(actor))
                .ok_or_else(|| q1_error("Rogue team changes require shared scores"))
        })?;
        self.score(game, actor, -frags)?;
        self.player_spawned(game, actor)
    }

    /// Select a team spawn point (`selectSpawn`).
    pub fn select_spawn(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<Option<ActorId>, Q1Error> {
        let world = game.world.clone();
        if game.options().coop || game.options().deathmatch == 0 || !is_ctf(game) || world.is_none() {
            return Ok(None);
        }
        let world = world.expect("world");
        let entities: Vec<ActorId> = game.entity_ids();
        if let Some(test) = entities.iter().find(|id| {
            game.entity(id)
                .is_some_and(|entity| entity.classname == "testplayerstart")
        }) {
            return Ok(Some(test.clone()));
        }
        let state = self.state(game, actor)?;
        let killed = game
            .entity(&state)
            .map(|state| state.number("ctf_killed"))
            .unwrap_or(0.0);
        let team = if killed == 0.0 {
            self.team(game, Some(actor))?
        } else {
            0
        };
        let (key, classname) = if team == 5 {
            ("rogue:team1_lastspawn", "info_player_team1")
        } else if team == 14 {
            ("rogue:team2_lastspawn", "info_player_team2")
        } else {
            ("rogue:lastspawn", "info_player_deathmatch")
        };
        let last = game
            .entity(&world)
            .and_then(|world| world.references.get(key).cloned().flatten())
            .filter(|last| game.entity(last).is_some());
        let points: Vec<ActorId> = entities
            .into_iter()
            .filter(|id| game.entity(id).is_some_and(|entity| entity.classname == classname))
            .collect();
        let start = last
            .as_ref()
            .and_then(|last| points.iter().position(|point| same_actor(point, last)))
            .map(|index| index as i32)
            .unwrap_or(-1);
        for count in 1..=points.len() {
            let point = points[((start + count as i32) % points.len() as i32) as usize].clone();
            let occupied_by_last = last.as_ref().is_some_and(|last| same_actor(&point, last));
            if occupied_by_last
                || !(game.host.players)().iter().any(|player| {
                    game.host.bodies.read(player).is_some_and(|body| {
                        let center = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
                        game.body(&point)
                            .map(|point| f64::from(length(vsub(center, point.origin))) <= 32.0)
                            .unwrap_or(false)
                    })
                })
            {
                let point_id = point.clone();
                game.update_entity(&world, |world| {
                    world.references.insert(key.to_string(), Some(point_id));
                })?;
                return Ok(Some(point));
            }
        }
        Ok(None)
    }

    /// Report flag status on impulse 23 (`impulse`).
    pub fn impulse(&self, game: &mut Q1EntityServices, actor: &ActorId, impulse: i32) -> Result<bool, Q1Error> {
        if impulse != 23 {
            return Ok(false);
        }
        if game.options().deathmatch == 0 {
            return Ok(true);
        }
        if !is_ctf(game) {
            self.message(game, actor, "$qc_ctf_disabled", false, Vec::new());
            return Ok(true);
        }
        let flags = self.flags(game);
        if teamplay_mode(game) == 5 {
            let flag = flags
                .iter()
                .find(|flag| game.entity(flag).is_some_and(|entity| entity.classname == "item_flag"));
            let text = match flag {
                None => "$qc_flag_missing".to_string(),
                Some(flag) => {
                    let status = game.entity(flag).map(|entity| entity.number("cnt")).unwrap_or(0.0);
                    let owner = game.entity(flag).and_then(|entity| entity.owner.clone());
                    if status == 0.0 {
                        "$qc_flag_at_base".to_string()
                    } else if status == 2.0 {
                        "$qc_flag_lying_about".to_string()
                    } else if status == 1.0 {
                        match owner {
                            Some(owner) if same_actor(&owner, actor) => "$qc_you_have_flag".to_string(),
                            Some(owner) => {
                                format!(
                                    "{} of the {} team has the flag!\n",
                                    self.name(game, &owner)?,
                                    team_name(self.team(game, Some(&owner))?)
                                )
                            }
                            None => "$qc_flag_screwed_up".to_string(),
                        }
                    } else {
                        "$qc_flag_screwed_up".to_string()
                    }
                }
            };
            self.message(game, actor, &text, false, Vec::new());
            return Ok(true);
        }
        let red = flags
            .iter()
            .find(|flag| {
                game.entity(flag)
                    .is_some_and(|entity| entity.classname == "item_flag_team1")
            })
            .cloned();
        let blue = flags
            .iter()
            .find(|flag| {
                game.entity(flag)
                    .is_some_and(|entity| entity.classname == "item_flag_team2")
            })
            .cloned();
        let ordered = if teamplay_mode(game) == 4 && self.color(game, actor)? != 5 {
            [blue.clone(), red.clone()]
        } else {
            [red.clone(), blue.clone()]
        };
        for (index, flag) in ordered.into_iter().enumerate() {
            let red_team = red
                .as_ref()
                .and_then(|red| game.entity(red))
                .map(|red| red.number("team") as i32);
            let own = if teamplay_mode(game) == 4 {
                index == 0
            } else {
                Some(self.team(game, Some(actor))?) == red_team
            };
            let label = if own {
                "Your flag".to_string()
            } else if teamplay_mode(game) == 4 {
                "The enemy flag".to_string()
            } else {
                format!("{} flag", if index == 0 { "Red" } else { "Blue" })
            };
            let status = flag
                .as_ref()
                .and_then(|flag| game.entity(flag))
                .map(|flag| flag.number("cnt"));
            let owner = flag
                .as_ref()
                .and_then(|flag| game.entity(flag))
                .and_then(|flag| flag.owner.clone());
            if status == Some(1.0) {
                if let Some(owner) = owner {
                    let text = if same_actor(&owner, actor) {
                        if teamplay_mode(game) == 4 {
                            "$qc_you_have_enemy_flag".to_string()
                        } else {
                            format!("You have the {} flag!\n", if index == 0 { "Red" } else { "Blue" })
                        }
                    } else if teamplay_mode(game) == 4 {
                        format!(
                            "{} has {} flag.\n",
                            self.name(game, &owner)?,
                            if own { "your" } else { "the enemy" }
                        )
                    } else {
                        format!(
                            "{} of the {} team has the {} flag.\n",
                            self.name(game, &owner)?,
                            team_name(self.team(game, Some(&owner))?),
                            if index == 0 { "Red" } else { "Blue" }
                        )
                    };
                    self.message(game, actor, &text, false, Vec::new());
                    continue;
                }
            }
            let state = match flag.as_ref() {
                None => "missing!".to_string(),
                Some(flag) => {
                    let status = game.entity(flag).map(|entity| entity.number("cnt")).unwrap_or(-1.0);
                    if status == 0.0 {
                        if teamplay_mode(game) == 6 {
                            "at base.".to_string()
                        } else if own {
                            "in your base.".to_string()
                        } else {
                            "in their base.".to_string()
                        }
                    } else if status == 2.0 {
                        "lying about.".to_string()
                    } else {
                        " corrupt.".to_string()
                    }
                }
            };
            self.message(game, actor, &format!("{label} is {state}\n"), false, Vec::new());
        }
        Ok(true)
    }

    /// Broadcast score updates every two minutes (`checkUpdate`).
    fn check_update(&self, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        let world = game.world.clone();
        let Some(world) = world else {
            return Ok(());
        };
        if game
            .entity(&world)
            .map(|world| world.number("rogue:nextteamupdtime"))
            .unwrap_or(0.0)
            > game.time
            || teamplay_mode(game) < 1
            || game.options().deathmatch == 0
        {
            return Ok(());
        }
        let time = game.time;
        game.update_entity(&world, |world| number(world, "rogue:nextteamupdtime", time + 120.0))?;
        if !is_ctf(game) {
            return Ok(());
        }
        let mut red = 0;
        let mut blue = 0;
        let mut grey = 0;
        for player in (game.host.players)() {
            let team = self.team(game, Some(&player))?;
            let score = with_missionpack_hooks(game, |_, hooks| {
                hooks
                    .frags
                    .as_ref()
                    .map(|frags| frags(&player))
                    .ok_or_else(|| q1_error("Rogue score update requires shared frags"))
            })?;
            if team == 5 {
                red += score;
            } else if team == 14 {
                blue += score;
            } else if team == 1 {
                grey += score;
            }
        }
        let mut scores = if teamplay_mode(game) == 6 {
            vec![(5, red), (14, blue), (1, grey)]
        } else {
            vec![(5, red), (14, blue)]
        };
        scores.sort_by_key(|score| std::cmp::Reverse(score.1));
        let (first, second) = (scores[0], scores[1]);
        if first.1 > second.1 {
            self.broadcast(
                game,
                &format!(
                    "{} team is leading by {} points!\n",
                    team_name(first.0),
                    first.1 - second.1
                ),
            );
            return Ok(());
        }
        let tied = if red == blue {
            (5, 14)
        } else if grey == blue {
            (14, 1)
        } else {
            (5, 1)
        };
        self.broadcast(
            game,
            &format!(
                "{} and {} teams are tied with {} points!\n",
                team_name(tied.0),
                team_name(tied.1),
                first.1
            ),
        );
        Ok(())
    }

    /// All flag entities (`flags`).
    fn flags(&self, game: &Q1EntityServices) -> Vec<ActorId> {
        game.entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity(id).is_some_and(|entity| {
                    entity.classname == "item_flag_team1"
                        || entity.classname == "item_flag_team2"
                        || entity.classname == "item_flag"
                })
            })
            .collect()
    }

    /// Drop a flag or base to the floor (`dropFloor`).
    fn drop_floor(&self, game: &mut Q1EntityServices, id: &ActorId) -> Result<bool, Q1Error> {
        let body = game.body(id)?;
        let origin = vadd(body.origin, Vec3 { x: 0.0, y: 0.0, z: 6.0 });
        let hit = game.host.trace(&crate::q1::foundation::types::Q1TraceRequest {
            start: origin,
            end: vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -256.0,
                },
            ),
            bounds: body.bounds,
            ignore: Some(id.clone()),
            monsters: true,
            missile: false,
        });
        if hit.fraction == 1.0 || hit.all_solid {
            return Ok(false);
        }
        game.set_body(
            id,
            &BodyPatch {
                origin: Some(hit.end),
                velocity: Some(ZERO),
                ground: Some(hit.actor.clone()),
                ..Default::default()
            },
        )?;
        game.link(id)?;
        Ok(true)
    }

    /// Spawn a flag base (`flagBase`).
    fn flag_base(&self, game: &mut Q1EntityServices, flag: &ActorId, classname: &str) -> Result<(), Q1Error> {
        let base = game.create(classname, None, None)?;
        let (skin, team, origin, angles) = game
            .entity(flag)
            .map(|entity| {
                (
                    entity.skin,
                    entity.number("team"),
                    game.body(entity.actor.id()).map(|body| body.origin),
                    game.body(entity.actor.id()).map(|body| body.angles),
                )
            })
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        let (origin, angles) = (origin?, angles?);
        let mode = teamplay_mode(game);
        game.update_entity(&base, |base| {
            base.model = "progs/ctfbase.mdl".to_string();
            base.skin = skin;
            number(base, "team", team);
            base.movement_flags = 256;
            base.movement = Q1MoveType::Toss;
            base.solid = if mode == 5 || mode == 6 {
                Q1Solid::Trigger
            } else {
                Q1Solid::None
            };
        })?;
        if mode == 5 || mode == 6 {
            let touch_name = game.named.touch("rogue:flagbase_touch")?;
            game.update_entity(&base, |base| base.touch = Some(touch_name))?;
        }
        game.set_body(
            &base,
            &BodyPatch {
                origin: Some(origin),
                angles: Some(angles),
                bounds: Some(qa_core::math::Bounds {
                    min: Vec3 {
                        x: -8.0,
                        y: -8.0,
                        z: 0.0,
                    },
                    max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
                }),
                ..Default::default()
            },
        )?;
        if !self.drop_floor(game, &base)? {
            game.remove(&base)?;
        }
        Ok(())
    }

    /// Return a flag to its base (`regenerate`).
    fn regenerate(&self, game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
        game.update_entity(flag, |flag| {
            flag.movement = Q1MoveType::Toss;
            flag.solid = Q1Solid::Trigger;
        })?;
        game.sound_simple(flag, "items/itembk2.wav")?;
        let (origin, mangle) = game
            .entity(flag)
            .map(|flag| (flag.vector("oldorigin"), flag.mangle))
            .unwrap_or((ZERO, ZERO));
        game.set_body(
            flag,
            &BodyPatch {
                origin: Some(origin),
                angles: Some(mangle),
                ..Default::default()
            },
        )?;
        game.update_entity(flag, |flag| {
            number(flag, "cnt", 0.0);
            flag.owner = None;
        })?;
        game.link(flag)
    }

    /// Return a flag with announcements (`returnFlag`).
    fn return_flag(&self, game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
        self.regenerate(game, flag)?;
        let (team, mode) = (
            game.entity(flag).map(|flag| flag.number("team") as i32).unwrap_or(0),
            teamplay_mode(game),
        );
        for actor in (game.host.players)() {
            let text = if mode == 5 {
                "$qc_flag_returned".to_string()
            } else if mode == 6 {
                format!("{} flag has been returned to base!\n", team_name(team))
            } else if self.team(game, Some(&actor))? == team {
                "$qc_your_flag_returned_base".to_string()
            } else {
                "$qc_enemy_flag_returned_base".to_string()
            };
            self.message(game, &actor, &text, true, Vec::new());
        }
        Ok(())
    }

    /// Drop a carried flag (`dropFlag`).
    fn drop_flag(&self, game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
        let actor = game.entity(flag).and_then(|flag| flag.owner.clone());
        let body = actor.as_ref().and_then(|actor| game.host.bodies.read(actor));
        let Some(body) = body else {
            return self.return_flag(game, flag);
        };
        if let Some(actor) = actor.as_ref() {
            let team = game.entity(flag).map(|flag| flag.number("team") as i32).unwrap_or(0);
            let name = self.name(game, actor)?;
            let ctf = teamplay_mode(game) == 5;
            self.broadcast(
                game,
                &format!(
                    "{} lost the {}flag!\n",
                    name,
                    if ctf {
                        String::new()
                    } else {
                        format!("{} ", team_name(team))
                    }
                ),
            );
        }
        game.set_body(
            flag,
            &BodyPatch {
                origin: Some(vadd(
                    body.origin,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: -24.0,
                    },
                )),
                velocity: Some(Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 300.0,
                }),
                bounds: Some(FLAG_BOUNDS),
                ..Default::default()
            },
        )?;
        let time = game.time;
        game.update_entity(flag, |flag| {
            number(flag, "cnt", 2.0);
            flag.movement_flags = 256 | 131072;
            flag.solid = Q1Solid::Trigger;
            flag.movement = Q1MoveType::Toss;
            number(flag, "super_time", time + 40.0);
        })?;
        game.link(flag)
    }

    /// Think a flag: stick to carriers, time out drops (`flagThink`).
    fn flag_think(&self, game: &mut Q1EntityServices, flag: &ActorId) -> Result<(), Q1Error> {
        later(game, flag, 0.1, "rogue:flag_think")?;
        let status = game.entity(flag).map(|flag| flag.number("cnt")).unwrap_or(0.0);
        if status == 0.0 {
            return Ok(());
        }
        if status == 2.0 {
            if game.time - game.entity(flag).map(|flag| flag.number("super_time")).unwrap_or(0.0) > 40.0 {
                return self.return_flag(game, flag);
            }
            return Ok(());
        }
        if status != 1.0 {
            return Err(q1_error("Flag in invalid state"));
        }
        let actor = game.entity(flag).and_then(|flag| flag.owner.clone());
        let body = actor.as_ref().and_then(|actor| game.host.bodies.read(actor));
        let (Some(actor), Some(body)) = (actor, body) else {
            return self.drop_flag(game, flag);
        };
        if !game.is_player(&actor) || game.health(&actor) <= 0.0 {
            return self.drop_flag(game, flag);
        }
        let state = self.state(game, &actor)?;
        let bits = game
            .entity(&state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0);
        let team = game.entity(flag).map(|flag| flag.number("team") as i32).unwrap_or(0);
        if teamplay_mode(game) == 5 && bits & 1 == 0 || team == 5 && bits & 1 == 0 || team == 14 && bits & 2 == 0 {
            return self.drop_flag(game, flag);
        }
        let frame = with_missionpack_hooks(game, |_, hooks| {
            hooks
                .player_frame
                .as_ref()
                .map(|player_frame| player_frame(&actor))
                .ok_or_else(|| q1_error("Rogue carried flags require character source frames"))
        })?;
        const OFFSETS: [f64; 12] = [2.0, 8.0, 12.0, 11.0, 10.0, 4.0, 2.0, 10.0, 10.0, 8.0, 4.0, 2.0];
        let extra = if (29..=40).contains(&frame) {
            OFFSETS[(frame - 29) as usize]
        } else if (103..=106).contains(&frame) {
            6.0
        } else if (107..=118).contains(&frame) {
            7.0
        } else {
            0.0
        };
        let basis = game.make_vectors(body.angles);
        let forward = Vec3 {
            x: basis.forward.x,
            y: basis.forward.y,
            z: -basis.forward.z,
        };
        game.set_body(
            flag,
            &BodyPatch {
                origin: Some(vadd(
                    vsub(
                        vadd(
                            body.origin,
                            Vec3 {
                                x: 0.0,
                                y: 0.0,
                                z: -16.0,
                            },
                        ),
                        vscale(forward, 14.0 + extra),
                    ),
                    vscale(basis.right, 22.0),
                )),
                angles: Some(Vec3 {
                    x: body.angles.x,
                    y: body.angles.y,
                    z: body.angles.z - 45.0,
                }),
                ..Default::default()
            },
        )?;
        game.link(flag)?;
        later(game, flag, 0.01, "rogue:flag_think")
    }

    /// Strip keys from a capturing player (`clearKeys`).
    fn clear_keys(&self, game: &mut Q1EntityServices, actor: &ActorId) {
        if let Some(owned) = game.host.actors.resolve_owned(actor) {
            for item in ["q1:key/silver", "q1:key/gold"] {
                let item = ItemId::from(item);
                let count = game.host.inventory.count(actor, &item);
                game.host.inventory.consume(&owned, &item, count);
            }
        }
    }

    /// Capture a flag (`capture`).
    fn capture(&self, game: &mut Q1EntityServices, actor: &ActorId, alternate: bool) -> Result<(), Q1Error> {
        let state = self.state(game, actor)?;
        let team = self.team(game, Some(actor))?;
        let bits = game
            .entity(&state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0);
        let name = self.name(game, actor)?;
        self.broadcast(game, &format!("{name} captured the flag!\n"));
        self.clear_keys(game, actor);
        self.sound(game, actor, "misc/flagcap.wav", true)?;
        self.score(game, actor, if alternate { 8 } else { 15 })?;
        for player in (game.host.players)() {
            let other = self.state(game, &player)?;
            if self.color(game, &player)? == team {
                if !same_actor(actor, &player) {
                    self.score(game, &player, if alternate { 4 } else { 10 })?;
                }
                if !alternate {
                    if teamplay_mode(game) != 5
                        && game
                            .entity(&other)
                            .map(|other| other.number("ctf_lastreturnedflag"))
                            .unwrap_or(0.0)
                            + 4.0
                            > game.time
                    {
                        self.score(game, &player, 1)?;
                    }
                    if game
                        .entity(&other)
                        .map(|other| other.number("ctf_lastfraggedcarrier"))
                        .unwrap_or(0.0)
                        + 6.0
                        > game.time
                    {
                        self.score(game, &player, 2)?;
                    }
                }
                self.message(game, &player, "$qc_your_team_captured", true, Vec::new());
            } else {
                game.update_entity(&other, |other| number(other, "ctf_lasthurtcarrier", -5.0))?;
                self.message(game, &player, "$qc_your_flag_captured", true, Vec::new());
            }
            if !alternate {
                game.update_entity(&other, |other| {
                    number(other, "ctf_flags", (other.number("ctf_flags") as i32 & !3) as f64);
                })?;
            }
        }
        for flag in self.flags(game) {
            let classname = game
                .entity(&flag)
                .map(|flag| flag.classname.clone())
                .unwrap_or_default();
            let team_number = game.entity(&flag).map(|flag| flag.number("team") as i32).unwrap_or(0);
            let wanted = if alternate {
                team_number == (if bits & 1 != 0 { 5 } else { 14 })
            } else if teamplay_mode(game) == 5 {
                classname == "item_flag"
            } else {
                classname != "item_flag"
            };
            if wanted {
                self.regenerate(game, &flag)?;
            }
        }
        if alternate {
            game.update_entity(&state, |state| number(state, "ctf_flags", (bits & !3) as f64))?;
        }
        Ok(())
    }

    /// Pick up (or return) a flag on touch (`touch`).
    fn touch(&self, game: &mut Q1EntityServices, flag: &ActorId, actor: &ActorId) -> Result<(), Q1Error> {
        if !game.is_player(actor)
            || game.health(actor) <= 0.0
            || self.color(game, actor)? != self.team(game, Some(actor))?
            || game.entity(flag).map(|flag| flag.number("cnt")).unwrap_or(0.0) == 1.0
        {
            return Ok(());
        }
        let state = self.state(game, actor)?;
        let bits = game
            .entity(&state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0);
        let team = game.entity(flag).map(|flag| flag.number("team") as i32).unwrap_or(0);
        if teamplay_mode(game) != 5 {
            if teamplay_mode(game) != 4 && teamplay_mode(game) != 6 {
                return Ok(());
            }
            if team == self.team(game, Some(actor))? {
                if game.entity(flag).map(|flag| flag.number("cnt")).unwrap_or(0.0) == 0.0 {
                    if team == 5 && bits & 2 != 0 || team == 14 && bits & 1 != 0 {
                        return self.capture(game, actor, false);
                    }
                    return Ok(());
                }
                self.score(game, actor, 1)?;
                let time = game.time;
                game.update_entity(&state, |state| number(state, "ctf_lastreturnedflag", time))?;
                let noise1 = game.entity(flag).map(|flag| flag.text("noise1")).unwrap_or_default();
                self.sound(game, actor, &noise1, false)?;
                return self.return_flag(game, flag);
            }
            if bits & 3 != 0 {
                return Ok(());
            }
        }
        let name = self.name(game, actor)?;
        let ctf = teamplay_mode(game) == 5;
        self.broadcast(
            game,
            &format!(
                "{} got the {}flag!\n",
                name,
                if ctf {
                    String::new()
                } else {
                    format!("{} ", team_name(team))
                }
            ),
        );
        let noise = game.entity(flag).map(|flag| flag.text("noise")).unwrap_or_default();
        self.sound(game, actor, &noise, false)?;
        let time = game.time;
        game.update_entity(&state, |state| {
            number(state, "ctf_flags", (bits | if team == 14 { 2 } else { 1 }) as f64);
            number(state, "ctf_flagsince", time);
        })?;
        if let Some(owned) = game.host.actors.resolve_owned(actor) {
            if team == 0 || team == 14 {
                game.host.inventory.give(&owned, &ItemId::from("q1:key/silver"), 1.0);
            }
            if team == 0 || team == 5 {
                game.host.inventory.give(&owned, &ItemId::from("q1:key/gold"), 1.0);
            }
        }
        let actor_id = actor.clone();
        game.update_entity(flag, |flag| {
            flag.owner = Some(actor_id);
            flag.movement = Q1MoveType::Noclip;
            flag.solid = Q1Solid::None;
            number(flag, "cnt", 1.0);
        })?;
        game.link(flag)?;
        self.message(
            game,
            actor,
            if teamplay_mode(game) == 5 {
                "YOU GOT THE FLAG\n\nTAKE IT TO THEIR BASE\n"
            } else {
                "YOU GOT THE ENEMY FLAG\n\nRETURN TO BASE\n"
            },
            true,
            Vec::new(),
        );
        for player in (game.host.players)() {
            if same_actor(actor, &player) {
                continue;
            }
            let text = if teamplay_mode(game) == 5 {
                "$qc_flag_taken".to_string()
            } else if self.team(game, Some(&player))? == team {
                "$qc_your_flag_taken".to_string()
            } else {
                format!(
                    "{} team has the {} flag!\n",
                    team_name(self.team(game, Some(actor))?),
                    team_name(team)
                )
            };
            self.message(game, &player, &text, true, Vec::new());
        }
        Ok(())
    }

    /// Capture on a flag base touch (`baseTouch`).
    fn base_touch(&self, game: &mut Q1EntityServices, base: &ActorId, actor: &ActorId) -> Result<(), Q1Error> {
        if !game.is_player(actor)
            || game.health(actor) <= 0.0
            || self.color(game, actor)? != self.team(game, Some(actor))?
        {
            return Ok(());
        }
        let state = self.state(game, actor)?;
        let bits = game
            .entity(&state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0);
        let team = game.entity(base).map(|base| base.number("team") as i32).unwrap_or(0);
        if teamplay_mode(game) == 5 {
            let own = self.team(game, Some(actor))?;
            if (team == 5 && own == 14 || team == 14 && own == 5) && bits & 1 != 0 {
                return self.capture(game, actor, false);
            }
            return Ok(());
        }
        if teamplay_mode(game) == 6
            && self.team(game, Some(actor))? == 1
            && (bits & 1 != 0 && team == 14 || bits & 2 != 0 && team == 5)
        {
            return self.capture(game, actor, true);
        }
        Ok(())
    }

    /// Note carrier damage for assists (`confirmedDamage`).
    pub fn confirmed_damage(
        &self,
        game: &mut Q1EntityServices,
        target: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        let Some(attacker) = attacker else {
            return Ok(());
        };
        if !is_ctf(game) || !game.is_player(target) || !game.is_player(attacker) {
            return Ok(());
        }
        let target_state = self.state(game, target)?;
        if game
            .entity(&target_state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0)
            & 3
            == 0
        {
            return Ok(());
        }
        if self.team(game, Some(target))? == self.team(game, Some(attacker))? {
            return Ok(());
        }
        let attacker_state = self.state(game, attacker)?;
        let time = game.time;
        game.update_entity(&attacker_state, |state| number(state, "ctf_lasthurtcarrier", time))
    }

    /// Handle player death: assists, flag drops (`playerDied`).
    pub fn player_died(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        let state = self.state(game, actor)?;
        let killed = game
            .entity(&state)
            .map(|state| state.number("ctf_killed"))
            .unwrap_or(0.0);
        game.update_entity(&state, |state| {
            number(state, "ctf_killed", if killed == 2.0 { 0.0 } else { 1.0 })
        })?;
        if !is_ctf(game) {
            return Ok(());
        }
        let bits = game
            .entity(&state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0);
        if let Some(attacker) = attacker {
            if game.is_player(attacker) && !same_actor(actor, attacker) {
                self.assists(game, actor, attacker)?;
            }
        }
        if bits & 3 != 0 {
            for player in (game.host.players)() {
                if teamplay_mode(game) == 5
                    || bits & 1 != 0 && self.team(game, Some(&player))? == 5
                    || bits & 2 != 0 && self.team(game, Some(&player))? == 14
                {
                    let other = self.state(game, &player)?;
                    game.update_entity(&other, |other| number(other, "ctf_lasthurtcarrier", -10.0))?;
                }
            }
        }
        self.drop_carried_flag(game, actor)
    }

    /// Drop a player's carried flag (`dropCarriedFlag`).
    pub fn drop_carried_flag(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if !is_ctf(game) {
            return Ok(());
        }
        let state = self.state(game, actor)?;
        let bits = game
            .entity(&state)
            .map(|state| state.number("ctf_flags") as i32)
            .unwrap_or(0);
        let wanted = if teamplay_mode(game) == 5 && bits & 1 != 0 {
            "item_flag"
        } else if bits & 1 != 0 {
            "item_flag_team1"
        } else if bits & 2 != 0 {
            "item_flag_team2"
        } else {
            ""
        };
        if let Some(flag) = self
            .flags(game)
            .into_iter()
            .find(|flag| game.entity(flag).is_some_and(|entity| entity.classname == wanted))
        {
            game.update_entity(&state, |state| number(state, "ctf_flags", (bits & !3) as f64))?;
            self.drop_flag(game, &flag)?;
        }
        Ok(())
    }

    /// Award defense/offense assist bonuses (`assists`).
    fn assists(&self, game: &mut Q1EntityServices, target: &ActorId, attacker: &ActorId) -> Result<(), Q1Error> {
        let victim = self.state(game, target)?;
        let killer = self.state(game, attacker)?;
        let team = self.team(game, Some(attacker))?;
        if game
            .entity(&victim)
            .map(|victim| victim.number("ctf_flags") as i32)
            .unwrap_or(0)
            & 3
            != 0
            && self.team(game, Some(target))? != team
        {
            let time = game.time;
            game.update_entity(&killer, |killer| number(killer, "ctf_lastfraggedcarrier", time))?;
            if game
                .entity(&victim)
                .map(|victim| victim.number("ctf_flagsince"))
                .unwrap_or(0.0)
                + 2.0
                <= game.time
            {
                self.score(game, attacker, 2)?;
                self.message(
                    game,
                    attacker,
                    "$qc_enemy_killed_bonus",
                    false,
                    vec![Q1MessageArg::Number(2.0)],
                );
            } else {
                self.message(game, attacker, "$qc_enemy_killed_no_bonus", false, Vec::new());
            }
        }
        let mut flag_bonus = false;
        let mut carrier_bonus = false;
        if game
            .entity(&victim)
            .map(|victim| victim.number("ctf_lasthurtcarrier"))
            .unwrap_or(0.0)
            + 4.0
            > game.time
            && game
                .entity(&killer)
                .map(|killer| killer.number("ctf_flags") as i32)
                .unwrap_or(0)
                & 3
                == 0
        {
            self.score(game, attacker, 2)?;
            carrier_bonus = true;
        }
        for origin_actor in [attacker.clone(), target.clone()] {
            let origin = game.host.bodies.read(&origin_actor).map(|body| body.origin);
            let Some(origin) = origin else {
                continue;
            };
            for observation in game.host.actors.observations().iter().rev() {
                let actor = observation.id.clone();
                let body = game.host.bodies.read(&actor);
                let Some(body) = body else {
                    continue;
                };
                let center = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
                if f64::from(length(vsub(center, origin))) > 400.0 {
                    continue;
                }
                if game.is_player(&actor) && self.team(game, Some(&actor))? == team {
                    let carrier = self.state(game, &actor)?;
                    let flags = game
                        .entity(&carrier)
                        .map(|state| state.number("ctf_flags") as i32)
                        .unwrap_or(0);
                    if flags & 3 != 0 && !same_actor(&actor, attacker) && !carrier_bonus {
                        self.score(game, attacker, 1)?;
                        carrier_bonus = true;
                    }
                }
                let classname = game.host.classname(&actor);
                if team == 5 && classname == "item_flag_team1"
                    || team == 14 && classname == "item_flag_team2"
                    || classname == "item_flag" && (!same_actor(&origin_actor, target) || !flag_bonus)
                {
                    self.score(game, attacker, 1)?;
                    flag_bonus = true;
                }
            }
        }
        Ok(())
    }

    /// Gameful `armorAllowed` stage for the session damage pipeline.
    pub fn armor_allowed(&self, game: &mut Q1EntityServices, request: &DamageRequest) -> Result<bool, Q1Error> {
        if !is_ctf(game) {
            return Ok(true);
        }
        if request
            .attack
            .attacker
            .as_ref()
            .is_some_and(|attacker| same_actor(attacker, &request.target))
        {
            return Ok(true);
        }
        if self.team(game, request.attack.attacker.as_ref())? != self.team(game, Some(&request.target))? {
            return Ok(true);
        }
        Ok(gamecfg(game)? & 2 != 0)
    }

    /// Gameful `beforeHealth` stage for the session damage pipeline.
    pub fn before_health(&self, game: &mut Q1EntityServices, request: &DamageRequest) -> Result<bool, Q1Error> {
        if teamplay_mode(game) <= 0 {
            return Ok(true);
        }
        let same_teams =
            self.team(game, request.attack.attacker.as_ref())? == self.team(game, Some(&request.target))?;
        if teamplay_mode(game) == 1 && same_teams {
            return Ok(false);
        }
        if !is_ctf(game) {
            return Ok(true);
        }
        if request
            .attack
            .attacker
            .as_ref()
            .is_some_and(|attacker| same_actor(attacker, &request.target))
        {
            return Ok(true);
        }
        if !same_teams {
            return Ok(true);
        }
        Ok(gamecfg(game)? & 4 != 0)
    }
}

/// Place a spawned flag.
fn flag_place(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.update_entity(id, |entity| {
        entity.movement_flags = 256 | 131072;
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
    })?;
    let touch_name = game.named.touch("rogue:flag_touch")?;
    let angles = game.body(id)?.angles;
    game.update_entity(id, |entity| {
        entity.touch = Some(touch_name);
        entity.mangle = angles;
        entity.effects |= 8;
        number(entity, "cnt", 0.0);
    })?;
    if !RogueTeams.drop_floor(game, id)? {
        return game.remove(id);
    }
    let origin = game.body(id)?.origin;
    game.update_entity(id, |entity| vector(entity, "oldorigin", origin))?;
    later(game, id, 0.1, "rogue:flag_think")
}

/// Think a flag from a scheduled dispatch.
fn flag_think_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    RogueTeams.flag_think(game, id)
}

/// Touch a flag from a touch dispatch.
fn flag_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let other = other.clone();
    RogueTeams.touch(game, id, &other)
}

/// Touch a flag base from a touch dispatch.
fn flagbase_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let other = other.clone();
    RogueTeams.base_touch(game, id, &other)
}

/// Spawn a flag entity.
fn spawn_flag(game: &mut Q1EntityServices, id: &ActorId, team: i32, skin: i32, base: &str) -> Result<(), Q1Error> {
    if team == 0 {
        if teamplay_mode(game) != 5 {
            return game.remove(id);
        }
    } else if game.options().deathmatch == 0 || !is_ctf(game) {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        number(entity, "team", f64::from(team));
        entity.skin = skin;
    })?;
    RogueTeams.flag_base(game, id, base)?;
    if team != 0 && teamplay_mode(game) == 5 {
        return game.remove(id);
    }
    game.update_entity(id, |entity| {
        entity.model = "progs/ctfmodel.mdl".to_string();
        entity.fields.insert("noise".to_string(), "misc/flagtk.wav".to_string());
        entity
            .fields
            .insert("noise1".to_string(), "misc/flagret.wav".to_string());
    })?;
    game.set_bounds(id, FLAG_BOUNDS)?;
    later(game, id, 0.2, "rogue:flag_place")
}

fn spawn_flag_team1(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_flag(game, id, 5, 0, "item_flagbase_team1")
}

fn spawn_flag_team2(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_flag(game, id, 14, 1, "item_flagbase_team2")
}

fn spawn_flag_solo(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    spawn_flag(game, id, 0, 2, "item_flagbase")
}

/// Spawn a team start (marker only).
fn spawn_team_start(_game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    Ok(())
}

/// Spawn a `func_ctf_wall`.
fn spawn_ctf_wall(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if is_ctf(game) {
        return brush(game, id);
    }
    game.remove(id)
}

/// Spawn a `trigger_teleport`, dropping CTF-only exits outside CTF.
fn spawn_teleport(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0) & 4 != 0 && !is_ctf(game) {
        return game.remove(id);
    }
    spawn_map_actor(game, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};
    use qa_core::identity::ProviderId;

    use super::super::{install_missionpack_hooks, MissionpackWorldHooks};

    fn ctf_game() -> Q1EntityServices {
        let (host, _) = mock_host();
        let options = Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 1,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: Some(4),
            aim_threshold: None,
        };
        Q1EntityServices::new(host, options).expect("game")
    }

    fn with_hooks(game: &mut Q1EntityServices) {
        install_missionpack_hooks(
            game,
            MissionpackWorldHooks {
                team_color: Some(Box::new(|_| 5)),
                set_team_color: Some(Box::new(|_, _| {})),
                add_frags: Some(Box::new(|_, _| {})),
                frags: Some(Box::new(|_| 0)),
                player_name: Some(Box::new(|_| "player".to_string())),
                ..Default::default()
            },
        );
    }

    fn player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        game.set_health(&player, 100.0).expect("health");
        player
    }

    #[test]
    fn spawned_players_take_legal_colors() {
        let mut game = ctf_game();
        let teams = RogueTeams::new(&mut game).expect("teams");
        with_hooks(&mut game);
        let first = player(&mut game);
        let watch = first.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        teams.player_spawned(&mut game, &first).expect("spawn");
        assert_eq!(teams.team(&mut game, Some(&first)).expect("team"), 5);
    }

    #[test]
    fn enemy_flag_pickup_captures_at_home() {
        let mut game = ctf_game();
        let teams = RogueTeams::new(&mut game).expect("teams");
        with_hooks(&mut game);
        let red = game.create("item_flag_team1", None, None).expect("red");
        game.spawn_entity(&red, None).expect("spawn red");
        let blue = game.create("item_flag_team2", None, None).expect("blue");
        game.spawn_entity(&blue, None).expect("spawn blue");
        let runner = player(&mut game);
        let watch = runner.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        teams.player_spawned(&mut game, &runner).expect("spawned");
        game.update_entity(&red, |entity| number(entity, "cnt", 0.0))
            .expect("red home");
        game.update_entity(&blue, |entity| number(entity, "cnt", 0.0))
            .expect("blue home");
        teams.touch(&mut game, &blue, &runner).expect("take blue");
        assert_eq!(game.entity(&blue).expect("blue").number("cnt"), 1.0);
        teams.touch(&mut game, &red, &runner).expect("capture");
        assert_eq!(game.entity(&red).expect("red").number("cnt"), 0.0);
        assert_eq!(game.entity(&blue).expect("blue").number("cnt"), 0.0);
    }

    #[test]
    fn impulse_reports_and_rejects() {
        let mut game = ctf_game();
        let teams = RogueTeams::new(&mut game).expect("teams");
        with_hooks(&mut game);
        let runner = player(&mut game);
        teams.player_spawned(&mut game, &runner).expect("spawned");
        assert!(!teams.impulse(&mut game, &runner, 5).expect("other"));
        assert!(teams.impulse(&mut game, &runner, 23).expect("flag"));
    }

    #[test]
    fn damage_stages_gate_friendly_fire() {
        use crate::q1::foundation::gameplay::{AttackCause, AttackProvenance};
        use qa_core::time::SourceTime;

        fn request(target: &ActorId, attacker: Option<&ActorId>) -> DamageRequest {
            DamageRequest {
                attack: AttackProvenance {
                    sequence: 0,
                    time: SourceTime::Seconds(0.0),
                    attacker: attacker.cloned(),
                    inflictor: None,
                    originating_projectile: None,
                    weapon: None,
                    weapon_provider: ProviderId::new("q1", "official"),
                    damage_powerup_owner: None,
                    combat_provider: ProviderId::new("q1", "combat"),
                    inventory_provider: ProviderId::new("q1", "inventory"),
                    movement_provider: ProviderId::new("q1", "movement"),
                    cause: AttackCause::Q1 {
                        death_type: String::new(),
                        armor_effect: None,
                    },
                },
                target: target.clone(),
                amount: 10.0,
                knockback: 10.0,
                direction: ZERO,
                point: ZERO,
                normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                delivery: crate::q1::foundation::gameplay::DamageDelivery::Direct,
            }
        }

        let mut game = ctf_game();
        let teams = RogueTeams::new(&mut game).expect("teams");
        with_hooks(&mut game);
        let red = player(&mut game);
        teams.player_spawned(&mut game, &red).expect("red");
        let red2 = player(&mut game);
        teams.player_spawned(&mut game, &red2).expect("red2");
        let (watch_red, watch_red2) = (red.clone(), red2.clone());
        game.host.players = Box::new(move || vec![watch_red.clone(), watch_red2.clone()]);
        let friendly = request(&red, Some(&red2));
        assert!(!teams.armor_allowed(&mut game, &friendly).expect("armor"));
        assert!(!teams.before_health(&mut game, &friendly).expect("health"));
        let state = teams.state(&mut game, &red2).expect("state");
        game.update_entity(&state, |state| number(state, "steam", 14.0))
            .expect("blue");
        let hostile = request(&red, Some(&red2));
        assert!(teams.armor_allowed(&mut game, &hostile).expect("armor"));
        assert!(teams.before_health(&mut game, &hostile).expect("health"));
    }
}
