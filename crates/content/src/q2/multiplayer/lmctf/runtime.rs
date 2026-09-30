//! Q2 LMCTF match manager (`src/content/q2/multiplayer/lmctf/runtime.ts`).
//!
//! LM_CTF selected source mode over the shared Q2 game authority.
//! GPL-2.0-or-later.

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::InventoryEntry;
use crate::q2::equipment::grapple_services::{
    LmctfGrappleCheckpoint, capture_lmctf_grapple, restore_lmctf_grapple,
};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2PresentationEvent, Q2Solid, SpawnModule};
use crate::q2::foundation::weapons::WeaponSourceRules;
use crate::q2::foundation::weapons::player::{
    LmctfWeaponHooks, Q2WeaponContext, Q2WeaponSourceRules, set_weapon_source_rules,
};
use crate::q2::support::contracts::{CombatTraitChanges, GrappleSelection, SharedGrappleControl};

use super::super::ctf::types::item_id;
use super::admin::lmctf_admin_command;
use super::flags::{LmctfFlags, LmctfFlagsCheckpoint, lmctf_flag_callbacks};
use super::grapple::{LmctfGrapple, lmctf_grapple_equipment};
use super::match_::{LmctfMatch, LmctfMatchCheckpoint, LmctfMatchState};
use super::presentation::{lmctf_menu, lmctf_scoreboard};
use super::runes::{LmctfRunes, LmctfRunesCheckpoint, lmctf_rune_callbacks};
use super::spawns::select_lmctf_spawn;
use super::types::{
    LmctfHooks, LmctfPlayerState, LmctfPlayingTeam, LmctfRules, LmctfTeam, LmctfTravel, lmctf_name, lmctf_player,
    lmctf_print, lmctf_score, lmctf_stat,
};
use super::vote::{LmctfVote, LmctfVoteCheckpoint};
use super::weapons::{LmctfWeapons, lmctf_weapon_callbacks};

/// LMCTF rules checkpoint (`Q2Lmctf::capture` rules).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfRulesCheckpoint {
    /// Time limit minutes.
    pub time_limit_minutes: f64,
    /// Frag limit.
    pub frag_limit: i32,
    /// Map list.
    pub map_list: Vec<String>,
    /// CTF flags.
    pub ctf_flags: i32,
    /// Referee flags.
    pub ref_flags: i32,
    /// Rune bits.
    pub runes: i32,
    /// Skin set.
    pub skin_set: i32,
    /// Flag init.
    pub flag_init: bool,
    /// Disabled weapons.
    pub disabled_weapons: i32,
    /// Fast switch.
    pub fast_switch: bool,
    /// Auto lock.
    pub auto_lock: bool,
    /// Countdown seconds.
    pub countdown_seconds: f64,
    /// Quad seconds.
    pub quad_seconds: f64,
}

/// LMCTF player checkpoint (`Q2Lmctf::capture` players entry).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfPlayerCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Plasma mode.
    pub plasma_mode: bool,
    /// Team.
    pub team: LmctfTeam,
    /// Observer team.
    pub observer_team: LmctfTeam,
    /// Held rune.
    pub rune: Option<SavedActorId>,
    /// Regeneration frame.
    pub regen_frame: i32,
    /// Kill-carrier time.
    pub kill_carrier_time: f64,
    /// Hit-carrier time.
    pub hit_carrier_time: f64,
    /// Return-flag time.
    pub return_flag_time: f64,
    /// Defend-flag time.
    pub defend_flag_time: f64,
    /// Extra flags.
    pub extra_flags: i32,
    /// Spawn state.
    pub spawn_state: i32,
    /// Statistics.
    pub statistics: Vec<(String, f64)>,
    /// Grapple state.
    pub grapple: LmctfGrappleCheckpoint,
}

/// LMCTF checkpoint (`Q2Lmctf::capture`).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfCheckpoint {
    /// Rules.
    pub rules: LmctfRulesCheckpoint,
    /// Match.
    pub match_state: LmctfMatchCheckpoint,
    /// Vote.
    pub vote: LmctfVoteCheckpoint,
    /// Whether the plasma quad is live.
    pub plasma_quad: bool,
    /// Flags.
    pub flags: LmctfFlagsCheckpoint,
    /// Runes.
    pub runes: LmctfRunesCheckpoint,
    /// Players.
    pub players: Vec<LmctfPlayerCheckpoint>,
}

/// Merged LMCTF callbacks (`Q2Lmctf::callbacks`).
pub fn lmctf_callbacks(game: &Q2GameServices) -> Q2CallbackDefinitions {
    let mut callbacks = lmctf_flag_callbacks();
    let runes = lmctf_rune_callbacks();
    let grapple = LmctfGrapple { hooks: super::lmctf_hooks(game) }.callbacks(game);
    let weapons = lmctf_weapon_callbacks();
    callbacks.think.extend(runes.think);
    callbacks.think.extend(grapple.think);
    callbacks.think.extend(weapons.think);
    callbacks.touch.extend(runes.touch);
    callbacks.touch.extend(grapple.touch);
    callbacks.touch.extend(weapons.touch);
    callbacks.die.extend(grapple.die);
    callbacks
}

/// LMCTF spawn dispatch (`spawn`).
pub fn lmctf_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let lmctf = Q2Lmctf { hooks: super::lmctf_hooks(game) };
    lmctf.spawn(entity, game)
}

/// Post-native-think hook running the rune weapon frame (`postNativeThink`).
fn lmctf_post_native_think(context: &Q2WeaponContext, game: &mut Q2GameServices) -> bool {
    let owner = context.owner.actor.id().clone();
    let firing = game.weapons.states.get(&owner).map(|state| state.source_firing).unwrap_or(false);
    LmctfRunes { hooks: super::lmctf_hooks(game) }.weapon_frame(owner, game, firing)
}

/// One selected LMCTF match attached to the existing source player and game
/// services (`Q2Lmctf`).
#[derive(Debug, Clone, Copy)]
pub struct Q2Lmctf {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

impl Q2Lmctf {
    /// Session match.
    fn match_service(&self) -> LmctfMatch {
        LmctfMatch { hooks: self.hooks }
    }

    /// Session vote.
    fn vote(&self) -> LmctfVote {
        LmctfVote { hooks: self.hooks }
    }

    /// Session flags.
    fn flags(&self) -> LmctfFlags {
        LmctfFlags { hooks: self.hooks }
    }

    /// Session runes.
    pub fn runes(&self) -> LmctfRunes {
        LmctfRunes { hooks: self.hooks }
    }

    /// Session grapple.
    fn grapple(&self) -> LmctfGrapple {
        LmctfGrapple { hooks: self.hooks }
    }

    /// Session weapons.
    fn weapons(&self) -> LmctfWeapons {
        LmctfWeapons { hooks: self.hooks }
    }

    /// Register the match and build the spawn module.
    pub fn register(
        &self,
        game: &mut Q2GameServices,
        rules: LmctfRules,
        travel: Option<LmctfTravel>,
        shared: Option<Box<dyn SharedGrappleControl>>,
    ) -> SpawnModule {
        game.lmctf.hooks = Some(self.hooks);
        game.lmctf.rules = rules;
        game.lmctf.match_state = LmctfMatchState::default();
        game.lmctf.match_state.paused = travel.as_ref().map(|travel| travel.paused).unwrap_or(false);
        game.lmctf.travel = travel;
        game.lmctf.equipment = lmctf_grapple_equipment(shared.as_deref());
        game.lmctf.shared = shared;
        self.flags().register(game);
        self.runes().register(game);
        self.grapple().register(game);
        self.weapons().register(game);
        set_weapon_source_rules(
            game,
            &Q2WeaponSourceRules {
                kind: WeaponSourceRules::Lmctf,
                ctf: None,
                lmctf: Some(LmctfWeaponHooks { post_native_think: lmctf_post_native_think }),
            },
        );
        SpawnModule { spawn: lmctf_spawn, item_name: |_| None, callbacks: lmctf_callbacks(game) }
    }

    /// Spawn LMCTF entities (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        {
            let record = game.require_entity_mut(&entity);
            if record.classname == "item_flag_team1" {
                record.classname = "info_flag_red".to_string();
            } else if record.classname == "item_flag_team2" {
                record.classname = "info_flag_blue".to_string();
            } else if record.classname == "info_player_team1" {
                record.classname = "info_player_red".to_string();
            } else if record.classname == "info_player_team2" {
                record.classname = "info_player_blue".to_string();
            }
        }
        let classname = game.require_entity(&entity).classname.clone();
        if classname == "info_player_red" || classname == "info_player_blue" {
            game.set_solid(entity, Q2Solid::None);
            return true;
        }
        if classname == "info_position" {
            game.set_solid(entity, Q2Solid::None);
            return true;
        }
        if self.flags().spawn(entity.clone(), game) || self.runes().spawn(entity.clone(), game) {
            return true;
        }
        if classname == "item_invulnerability"
            && game.lmctf.rules.ctf_flags & 2 == 0
            && matches!(game.options.mode, Q2Mode::Deathmatch)
        {
            game.remove_actor(entity);
            return true;
        }
        false
    }

    /// Resolve spawns and start a travelled countdown (`postSpawn`).
    pub fn post_spawn(&self, game: &mut Q2GameServices) {
        game.source_callbacks.register(&lmctf_callbacks(game));
        self.flags().post_spawn(game);
        self.runes().post_spawn(game);
        if game.lmctf.travel.as_ref().map(|travel| travel.countdown).unwrap_or(false) {
            self.match_service().start(game);
        }
    }

    /// Capture time-travel (`captureTravel`).
    pub fn capture_travel(&self, game: &mut Q2GameServices) -> LmctfTravel {
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        let mut players = Vec::new();
        for actor in actors {
            let Some(slot) = (self.hooks.player)(actor.clone(), game).map(|player| player.slot) else {
                continue;
            };
            let Some(state) = game.lmctf.states.get(&actor) else {
                continue;
            };
            players.push(super::types::LmctfTravelPlayer {
                slot,
                team: state.team,
                observer_team: state.observer_team,
                extra_flags: state.extra_flags,
            });
        }
        players.sort_by(|left, right| left.slot.cmp(&right.slot));
        LmctfTravel {
            rules: game.lmctf.rules.clone(),
            countdown: game.lmctf.match_state.pending_map.as_ref().map(|pending| pending.countdown).unwrap_or(false),
            paused: game.lmctf.match_state.paused,
            players,
        }
    }

    /// Capture LMCTF state (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> LmctfCheckpoint {
        let rules = &game.lmctf.rules;
        let mut players: Vec<LmctfPlayerCheckpoint> = game
            .lmctf
            .states
            .iter()
            .map(|(actor, state)| {
                let mut statistics: Vec<(String, f64)> =
                    state.statistics.iter().map(|(key, count)| (key.clone(), *count)).collect();
                statistics.sort_by(|left, right| left.0.cmp(&right.0));
                let grapple = if game.lmctf.equipment.is_some() {
                    game.equipment.lmctf_states.get(actor).cloned().unwrap_or_default()
                } else {
                    game.lmctf.inactive_grapple.get(actor).cloned().unwrap_or_default()
                };
                LmctfPlayerCheckpoint {
                    actor: SavedActorId::from(actor),
                    plasma_mode: state.plasma_mode,
                    team: state.team,
                    observer_team: state.observer_team,
                    rune: state.rune.as_ref().map(SavedActorId::from),
                    regen_frame: state.regen_frame,
                    kill_carrier_time: state.kill_carrier_time,
                    hit_carrier_time: state.hit_carrier_time,
                    return_flag_time: state.return_flag_time,
                    defend_flag_time: state.defend_flag_time,
                    extra_flags: state.extra_flags,
                    spawn_state: state.spawn_state,
                    statistics,
                    grapple: capture_lmctf_grapple(&grapple),
                }
            })
            .collect();
        players.sort_by(|left, right| (left.actor.slot, left.actor.generation).cmp(&(right.actor.slot, right.actor.generation)));
        LmctfCheckpoint {
            rules: LmctfRulesCheckpoint {
                time_limit_minutes: rules.time_limit_minutes,
                frag_limit: rules.frag_limit,
                map_list: rules.map_list.clone(),
                ctf_flags: rules.ctf_flags,
                ref_flags: rules.ref_flags,
                runes: rules.runes,
                skin_set: rules.skin_set,
                flag_init: rules.flag_init,
                disabled_weapons: rules.disabled_weapons,
                fast_switch: rules.fast_switch,
                auto_lock: rules.auto_lock,
                countdown_seconds: rules.countdown_seconds,
                quad_seconds: rules.quad_seconds,
            },
            match_state: self.match_service().capture(game),
            vote: self.vote().capture(game),
            plasma_quad: game.lmctf.plasma_quad,
            flags: self.flags().capture(game),
            runes: self.runes().capture(game),
            players,
        }
    }

    /// Restore LMCTF state (`restore`).
    pub fn restore(&self, checkpoint: &LmctfCheckpoint, game: &mut Q2GameServices) {
        if let Some(equipment) = game.lmctf.equipment {
            equipment.bind(game);
        }
        {
            let rules = &mut game.lmctf.rules;
            rules.time_limit_minutes = checkpoint.rules.time_limit_minutes;
            rules.frag_limit = checkpoint.rules.frag_limit;
            rules.map_list = checkpoint.rules.map_list.clone();
            rules.ctf_flags = checkpoint.rules.ctf_flags;
            rules.ref_flags = checkpoint.rules.ref_flags;
            rules.runes = checkpoint.rules.runes;
            rules.skin_set = checkpoint.rules.skin_set;
            rules.flag_init = checkpoint.rules.flag_init;
            rules.disabled_weapons = checkpoint.rules.disabled_weapons;
            rules.fast_switch = checkpoint.rules.fast_switch;
            rules.auto_lock = checkpoint.rules.auto_lock;
            rules.countdown_seconds = checkpoint.rules.countdown_seconds;
            rules.quad_seconds = checkpoint.rules.quad_seconds;
        }
        LmctfGrapple::states(game).clear();
        let mut restored = Vec::with_capacity(checkpoint.players.len());
        for player in &checkpoint.players {
            let Some(actor) = game.host.actors().resolve_saved(player.actor) else {
                panic!("LMCTF restore requires an admitted source player");
            };
            let actor = actor.id().clone();
            if (self.hooks.player)(actor.clone(), game).is_none() {
                panic!("LMCTF restore requires an admitted source player");
            }
            let mut statistics = HashMap::new();
            for (key, count) in &player.statistics {
                statistics.insert(key.clone(), *count);
            }
            let state = LmctfPlayerState {
                plasma_mode: player.plasma_mode,
                team: player.team,
                observer_team: player.observer_team,
                rune: player.rune.map(|saved| game.host.actors().reference_saved(saved)),
                regen_frame: player.regen_frame,
                kill_carrier_time: player.kill_carrier_time,
                hit_carrier_time: player.hit_carrier_time,
                return_flag_time: player.return_flag_time,
                defend_flag_time: player.defend_flag_time,
                extra_flags: player.extra_flags,
                spawn_state: player.spawn_state,
                statistics,
            };
            if game.lmctf.equipment.is_some() {
                let grapple = restore_lmctf_grapple(player.grapple.clone(), game);
                LmctfGrapple::states(game).insert(actor.clone(), grapple);
            }
            restored.push((actor, state));
        }
        self.match_service().restore(game, &checkpoint.match_state);
        self.vote().restore(game, &checkpoint.vote);
        game.lmctf.plasma_quad = checkpoint.plasma_quad;
        self.flags().restore(game, &checkpoint.flags);
        self.runes().restore(game, &checkpoint.runes);
        game.lmctf.states.clear();
        for (actor, state) in restored {
            game.lmctf.states.insert(actor, state);
        }
    }

    /// Sum scores by team (`teamTotals`).
    pub fn team_totals(&self, game: &mut Q2GameServices) -> [i32; 2] {
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        let mut totals = [0, 0];
        for actor in actors {
            let team = game.lmctf.states.get(&actor).map(|state| state.team).unwrap_or(0);
            let score = (self.hooks.player)(actor, game).map(|player| player.score).unwrap_or(0);
            if team == 1 {
                totals[0] += score;
            } else if team == 2 {
                totals[1] += score;
            }
        }
        totals
    }

    /// Admit a player (`admitted`).
    pub fn admitted(&self, entity: ActorId, game: &mut Q2GameServices) {
        if let Some(equipment) = game.lmctf.equipment {
            equipment.bind(game);
        }
        if game.lmctf.states.contains_key(&entity) {
            return;
        }
        let snapshot =
            (self.hooks.player)(entity.clone(), game).map(|player| (player.slot, player.spectator, player.requested_spectator));
        let Some((slot, spectator, requested)) = snapshot else {
            panic!("LMCTF admission requires shared source player state");
        };
        game.lmctf.states.insert(entity.clone(), LmctfPlayerState::default());
        let carried = game
            .lmctf
            .travel
            .as_ref()
            .and_then(|travel| travel.players.iter().find(|player| player.slot == slot).cloned());
        if let Some(carried) = carried {
            lmctf_player(game, &entity).extra_flags = carried.extra_flags;
            if carried.team == 0 {
                self.observer(entity, game, carried.observer_team);
            } else {
                self.set_team(entity, game, carried.team);
            }
            return;
        }
        if spectator || requested {
            self.observer(entity, game, 0);
            return;
        }
        let mut red = 0;
        let mut blue = 0;
        for (actor, member) in game.lmctf.states.iter() {
            if *actor != entity {
                if member.team == 1 {
                    red += 1;
                } else if member.team == 2 {
                    blue += 1;
                }
            }
        }
        let totals = self.team_totals(game);
        let team = if red < blue {
            1
        } else if blue < red {
            2
        } else if totals[0] > totals[1] {
            2
        } else {
            1
        };
        self.set_team(entity, game, team);
    }

    /// Respawn an admitted player (`playerSpawned`).
    pub fn player_spawned(&self, entity: ActorId, game: &mut Q2GameServices) {
        let team = lmctf_player(game, &entity).team;
        self.grapple().abort(entity.clone(), game);
        LmctfGrapple::states(game).entry(entity.clone()).or_default().hook_held = false;
        let owned = game.owned_of(entity.clone());
        if !game.host.inventory().entries(owned.id()).iter().any(|entry| entry.item == "q2:weapon_hook") {
            game.host.inventory().configure(
                &owned,
                &InventoryEntry { item: item_id("q2:weapon_hook"), count: 0.0, capacity: 1.0, count_policy: None },
            );
        }
        if self.grapple().native_enabled(game) && team != 0 {
            game.host.inventory().give(&owned, &item_id("q2:weapon_hook"), 1.0);
        }
        let disabled =
            matches!(game.lmctf.shared.as_ref().map(|shared| shared.selection()), Some(GrappleSelection::Disabled));
        if disabled {
            game.host.inventory().configure(
                &owned,
                &InventoryEntry { item: item_id("q2:weapon_hook"), count: 0.0, capacity: 1.0, count_policy: None },
            );
        }
        let null_team = team == 0 || game.lmctf.rules.ctf_flags & 128 != 0;
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                team: Some(if null_team {
                    None
                } else if team == 1 {
                    Some("RED".to_string())
                } else {
                    Some("BLUE".to_string())
                }),
                ..CombatTraitChanges::default()
            },
        );
    }

    /// Assign a team directly (`setTeam`).
    fn set_team(&self, entity: ActorId, game: &mut Q2GameServices, team: LmctfPlayingTeam) {
        {
            let state = lmctf_player(game, &entity);
            state.team = team;
            state.observer_team = 0;
        }
        if let Some(player) = (self.hooks.player)(entity.clone(), game) {
            player.spectator = false;
            player.requested_spectator = false;
            player.noclip = false;
        }
        let owned = game.owned_of(entity.clone());
        let null_team = game.lmctf.rules.ctf_flags & 128 != 0;
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                team: Some(if null_team {
                    None
                } else if team == 1 {
                    Some("RED".to_string())
                } else {
                    Some("BLUE".to_string())
                }),
                ..CombatTraitChanges::default()
            },
        );
        let name = lmctf_name(game, &entity);
        lmctf_print(game, &format!("{name} is now on the {} team.\n", if team == 1 { "red" } else { "blue" }), None);
    }

    /// Join a team (`join`).
    pub fn join(&self, entity: ActorId, game: &mut Q2GameServices, team: LmctfPlayingTeam) {
        if lmctf_player(game, &entity).team == team {
            return;
        }
        if (self.hooks.player)(entity.clone(), game).is_none() {
            return;
        }
        if game.lmctf.match_state.teams_locked {
            lmctf_print(game, "Teams are locked.\n", Some(entity));
            return;
        }
        if game.lmctf.rules.ctf_flags & 8 != 0 {
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: entity,
                text: "Sorry.  Team switching has been turned\n off on this server.\n".to_string(),
                instant: false,
                duration_seconds: None,
            });
            return;
        }
        let spectator = (self.hooks.player)(entity.clone(), game).map(|player| player.spectator).unwrap_or(true);
        if !spectator {
            let origin = game.body_of(entity.clone()).origin;
            game.damage(
                entity.clone(),
                entity.clone(),
                Some(entity.clone()),
                100000.0,
                0.0,
                Vec3::default(),
                origin,
                Vec3::default(),
                23,
                32,
                None,
            );
            lmctf_score(game, &entity, 1, "Team Change", None);
            lmctf_stat(game, &entity, "deaths", -1);
        }
        self.drop_inventory(entity.clone(), game);
        self.set_team(entity.clone(), game, team);
        lmctf_player(game, &entity).spawn_state = 0;
        (self.hooks.spawn_player)(entity.clone(), game);
        self.player_spawned(entity, game);
    }

    /// Move a player to the observer team (`observer`).
    pub fn observer(&self, entity: ActorId, game: &mut Q2GameServices, team: LmctfTeam) {
        let _ = lmctf_player(game, &entity);
        if (self.hooks.player)(entity.clone(), game).is_none() {
            return;
        }
        self.drop_inventory(entity.clone(), game);
        {
            let state = lmctf_player(game, &entity);
            state.team = 0;
            state.observer_team = team;
        }
        if let Some(player) = (self.hooks.player)(entity.clone(), game) {
            player.spectator = true;
            player.requested_spectator = true;
            player.noclip = true;
        }
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                team: Some(None),
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
        (self.hooks.observer)(entity, game);
    }

    /// Select a spawn point (`selectSpawn`).
    pub fn select_spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> (Vec3, Vec3) {
        select_lmctf_spawn(entity, game)
    }

    /// Apply a score change (`score`).
    pub fn score(
        &self,
        victim: ActorId,
        attacker: Option<ActorId>,
        game: &mut Q2GameServices,
        change: i32,
        _means: i32,
        recipient: ActorId,
    ) {
        lmctf_score(game, &recipient, change, if change > 0 { "Kill" } else { "Suicide" }, Some(victim.clone()));
        if let Some(attacker) = attacker {
            self.flags().frag(victim, attacker, game);
        }
    }

    /// Drop match inventory (`dropInventory`).
    pub fn drop_inventory(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.flags().drop(entity.clone(), game);
        self.runes().drop(&entity, game);
        if LmctfGrapple::states(game).get(&entity).is_some_and(|state| state.hook.is_some()) {
            self.grapple().abort(entity, game);
        }
    }

    /// Handle a player death (`playerDeath`).
    pub fn player_death(&self, entity: ActorId, game: &mut Q2GameServices) {
        lmctf_stat(game, &entity, "deaths", 1);
        self.drop_inventory(entity, game);
    }

    /// Handle a disconnect (`disconnect`).
    pub fn disconnect(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.drop_inventory(entity.clone(), game);
        game.lmctf.states.remove(&entity);
        LmctfGrapple::states(game).remove(&entity);
    }

    /// Run the player frame (`playerFrame`).
    pub fn player_frame(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.match_service().frame(game);
        self.vote().frame(game);
        self.runes().player_frame(entity.clone(), game);
        if LmctfGrapple::states(game).get(&entity).map(|state| state.hook_state).unwrap_or(0) != 0 {
            self.grapple().fire(entity, game);
        }
    }

    /// Whether the player may move (`canMove`).
    pub fn can_move(&self, actor: &ActorId, game: &Q2GameServices) -> bool {
        !game.lmctf.match_state.paused || game.lmctf.states.get(actor).map(|state| state.extra_flags).unwrap_or(0) & 2 != 0
    }

    /// Read the gravity scale (`gravityScale`).
    pub fn gravity_scale(&self, actor: &ActorId, game: &Q2GameServices) -> i32 {
        self.grapple().gravity_scale(actor.clone(), game)
    }

    /// Handle a player command (`command`).
    pub fn command(&self, entity: ActorId, game: &mut Q2GameServices, name: &str, args: &[String]) -> bool {
        let name = name.to_ascii_lowercase();
        if lmctf_admin_command(game, entity.clone(), &name, args) {
            return true;
        }
        if !self.can_move(&entity, game)
            && !["ctfmenu", "voteyes", "voteno", "lmctf-vote", "score", "say", "say_team", "players", "playerlist"]
                .contains(&name.as_str())
        {
            return true;
        }
        match name.as_str() {
            "ctfmenu" => {
                lmctf_menu(game, entity);
                true
            }
            "voteyes" | "voteno" => {
                self.vote().ballot(entity, game, name == "voteyes");
                true
            }
            "lmctf-vote" => {
                if args.first().map(|arg| arg.as_str()) == Some("skip") {
                    self.vote().start(entity, game);
                } else {
                    self.vote().menu(entity, game);
                }
                true
            }
            "score" => {
                lmctf_scoreboard(game, entity);
                true
            }
            "hook" | "+hook" => {
                self.grapple().command(entity, game, true);
                true
            }
            "unhook" | "-hook" => {
                self.grapple().command(entity, game, false);
                true
            }
            "team" => {
                let choice = args.first().map(|arg| arg.to_ascii_lowercase()).unwrap_or_default();
                if choice == "red" || choice == "blue" {
                    self.join(entity, game, if choice == "red" { 1 } else { 2 });
                } else {
                    let team = lmctf_player(game, &entity).team;
                    lmctf_print(
                        game,
                        &format!(
                            "You are currently on the {} team.\nUse 'team red' or 'team blue' to change teams.\n",
                            if team == 1 { "red" } else { "blue" }
                        ),
                        Some(entity),
                    );
                }
                true
            }
            "observe" | "observe_red" | "observe_blue" => {
                self.observer(entity, game, if name == "observe_red" { 1 } else if name == "observe_blue" { 2 } else { 0 });
                true
            }
            "drop" | "use" => {
                let requested = args.join(" ").to_ascii_lowercase();
                if requested == "flag" || requested == "enemy flag" {
                    self.flags().drop(entity, game);
                    return true;
                }
                if requested == "rune" || requested.ends_with(" artifact") {
                    return self.runes().drop(&entity, game);
                }
                false
            }
            _ => false,
        }
    }
}
