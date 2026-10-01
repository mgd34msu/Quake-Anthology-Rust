//! Q2 CTF integration (`src/content/q2/multiplayer/ctf/index.ts`).
//!
//! Original Quake II CTF 1.09b g_ctf.c integration. GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3, vec3, Vec3};

use crate::contract::InventoryEntry;
use crate::q2::base::player::commands::userinfo_value;
use crate::q2::base::player::spawns::{
    q2_entities_named, q2_kill_box, q2_players_range, q2_spawn_origin, select_q2_spawn,
};
use crate::q2::equipment::ctf_grapple::ctf_grapple_callbacks;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::movedir;
use crate::q2::foundation::host::{
    Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2Think, Q2Touch,
    SpawnModule,
};
use crate::q2::foundation::weapons::player::{
    set_weapon_source_rules, CtfWeaponHooks, Q2WeaponContext, Q2WeaponSourceRules,
};
use crate::q2::foundation::weapons::types::Q2WeaponInput;
use crate::q2::foundation::weapons::WeaponSourceRules;
use crate::q2::support::contracts::{CombatTraitChanges, GrappleSelection, SharedGrappleControl, TouchContact};

use super::checkpoint::{capture_q2_ctf, restore_q2_ctf, Q2CtfCheckpoint};
use super::flags::{ctf_flag_callbacks, Q2CtfFlags};
use super::grapple::{ctf_grapple_equipment, Q2CtfGrapple, Q2CtfGrappleBinding};
use super::match_::{ctf_match_actions, Q2CtfAdminSettings, Q2CtfMatch};
use super::presentation::{ctf_tech, Q2CtfPresentation};
use super::techs::{ctf_tech_callbacks, Q2CtfTechs};
use super::types::{
    ctf_name, ctf_player, ctf_print, ctf_score, ctf_team_name, item_id, Q2CtfElectionKind, Q2CtfEvent, Q2CtfForceJoin,
    Q2CtfHooks, Q2CtfMatchPhase, Q2CtfMenuAction, Q2CtfMenuEntry, Q2CtfPlayerState, Q2CtfPlayingTeam, Q2CtfPrintLevel,
    Q2CtfRules,
};

/// CTF weapon haste (`setSourceRules` haste).
fn ctf_weapon_haste(context: &Q2WeaponContext, game: &mut Q2GameServices) -> bool {
    let techs = Q2CtfTechs {
        hooks: super::ctf_hooks(game),
    };
    techs.haste(context.owner.actor.id().clone(), game)
}

/// CTF weapon strength sound (`setSourceRules` strengthSound).
fn ctf_weapon_strength_sound(context: &Q2WeaponContext, game: &mut Q2GameServices) -> bool {
    let techs = Q2CtfTechs {
        hooks: super::ctf_hooks(game),
    };
    techs.strength_sound(context.owner.actor.id().clone(), game)
}

/// CTF weapon haste sound (`setSourceRules` hasteSound).
fn ctf_weapon_haste_sound(context: &Q2WeaponContext, game: &mut Q2GameServices) {
    let techs = Q2CtfTechs {
        hooks: super::ctf_hooks(game),
    };
    techs.haste_sound(context.owner.actor.id().clone(), game);
}

/// CTF integration callbacks.
pub fn ctf_index_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks
        .think
        .insert("misc_ctf_banner_think", ctf_banner_think as Q2Think);
    callbacks
        .touch
        .insert("old_teleporter_touch", ctf_teleport_touch as Q2Touch);
    callbacks
}

/// Merged CTF callbacks (`Q2Ctf::callbacks`).
pub fn ctf_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = ctf_flag_callbacks();
    let grapple = ctf_grapple_callbacks();
    let tech = ctf_tech_callbacks();
    let index = ctf_index_callbacks();
    callbacks.think.extend(tech.think);
    callbacks.think.extend(index.think);
    callbacks.touch.extend(grapple.touch);
    callbacks.touch.extend(index.touch);
    callbacks.use_.extend(grapple.use_);
    callbacks.use_.extend(tech.use_);
    callbacks.pain.extend(grapple.pain);
    callbacks.pain.extend(tech.pain);
    callbacks.die.extend(grapple.die);
    callbacks.die.extend(tech.die);
    callbacks.blocked.extend(grapple.blocked);
    callbacks.blocked.extend(tech.blocked);
    callbacks
}

/// CTF item name (`itemName`).
pub fn ctf_item_name(classname: &str) -> Option<String> {
    match classname {
        "item_flag_team1" => Some("Red Flag".to_string()),
        "item_flag_team2" => Some("Blue Flag".to_string()),
        "weapon_grapple" => Some("Grapple".to_string()),
        "item_tech1" => Some("Disruptor Shield".to_string()),
        "item_tech2" => Some("Power Amplifier".to_string()),
        "item_tech3" => Some("Time Accel".to_string()),
        "item_tech4" => Some("AutoDoc".to_string()),
        _ => None,
    }
}

/// CTF spawn dispatch (`spawn`).
pub fn ctf_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    let ctf = Q2Ctf {
        hooks: super::ctf_hooks(game),
    };
    ctf.spawn(entity, game)
}

/// Banner think (`bannerThink`).
fn ctf_banner_think(entity: ActorId, game: &mut Q2GameServices) {
    let frame = (game.require_entity(&entity).frame + 1) % 16;
    game.require_entity_mut(&entity).frame = frame;
    game.show(entity.clone());
    game.schedule(entity, 0.1, ctf_banner_think as Q2Think);
}

/// Teleporter touch (`teleportTouch`).
fn ctf_teleport_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.entity(&contact.other).is_none() || !game.host.is_player(&contact.other) {
        return;
    }
    let target = game.require_entity(&entity).target.clone();
    let Some(destination) = game.targets(&target).into_iter().next() else {
        game.host.diagnostic("Couldn't find CTF teleporter destination");
        return;
    };
    let grapple = Q2CtfGrapple {
        hooks: super::ctf_hooks(game),
    };
    grapple.reset(contact.other.clone(), game);
    let body = game.body_of(destination);
    let velocity = scale3(movedir(body.angles), 200.0);
    let owned = game.owned_of(contact.other.clone());
    game.host.bodies().unlink(&owned);
    let mut moved = game.body_of(contact.other.clone());
    moved.origin = body.origin;
    moved.velocity = velocity;
    moved.angles = Vec3 {
        x: 0.0,
        y: body.angles.y,
        z: 0.0,
    };
    game.write_body(contact.other.clone(), &moved, false);
    let hooks = super::ctf_hooks(game);
    (hooks.teleport)(contact.other.clone(), game, body.origin, body.angles, velocity);
    if let Some(enemy) = game.require_entity(&entity).enemy.clone() {
        game.host_emit(Q2PresentationEvent::EntityEvent { actor: enemy, event: 6 });
    }
    game.host_emit(Q2PresentationEvent::EntityEvent {
        actor: contact.other.clone(),
        event: 6,
    });
    q2_kill_box(contact.other.clone(), game);
    game.link_actor(contact.other);
}

/// Reset CTF players (`resetPlayers` action).
pub fn ctf_reset_players(game: &mut Q2GameServices) {
    let ctf = Q2Ctf {
        hooks: super::ctf_hooks(game),
    };
    ctf.reset_players(game);
}

/// Reset a CTF grapple (`resetGrapple` action).
pub fn ctf_reset_grapple(entity: ActorId, game: &mut Q2GameServices) {
    let grapple = Q2CtfGrapple {
        hooks: super::ctf_hooks(game),
    };
    grapple.reset(entity, game);
}

/// Join a CTF team (`join` action).
pub fn ctf_join(entity: ActorId, game: &mut Q2GameServices, team: Q2CtfPlayingTeam, ghost: bool) -> bool {
    let ctf = Q2Ctf {
        hooks: super::ctf_hooks(game),
    };
    ctf.join(entity, game, team, ghost)
}

/// One selected CTF match attached to the existing source player and game
/// services (`Q2Ctf`).
#[derive(Debug, Clone, Copy)]
pub struct Q2Ctf {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
}

impl Q2Ctf {
    /// Session flags.
    pub fn flags(&self) -> Q2CtfFlags {
        Q2CtfFlags { hooks: self.hooks }
    }

    /// Session match.
    fn match_service(&self) -> Q2CtfMatch {
        Q2CtfMatch {
            hooks: self.hooks,
            actions: ctf_match_actions(),
        }
    }

    /// Session presentation.
    fn presentation(&self) -> Q2CtfPresentation {
        Q2CtfPresentation { hooks: self.hooks }
    }

    /// Session grapple.
    fn grapple(&self) -> Q2CtfGrapple {
        Q2CtfGrapple { hooks: self.hooks }
    }

    /// Session techs.
    pub fn techs(&self) -> Q2CtfTechs {
        Q2CtfTechs { hooks: self.hooks }
    }

    /// Register the match and build the spawn module.
    pub fn register(
        &self,
        game: &mut Q2GameServices,
        rules: Q2CtfRules,
        shared: Option<Box<dyn SharedGrappleControl>>,
    ) -> SpawnModule {
        game.ctf.hooks = Some(self.hooks);
        game.ctf.rules = rules;
        game.ctf.match_state = super::types::Q2CtfMatchState::new();
        game.ctf.equipment = ctf_grapple_equipment(shared.as_deref());
        game.ctf.shared = shared;
        self.flags().register(game);
        self.grapple().register(game);
        self.techs().register(game);
        set_weapon_source_rules(
            game,
            &Q2WeaponSourceRules {
                kind: WeaponSourceRules::Ctf,
                ctf: Some(CtfWeaponHooks {
                    haste: ctf_weapon_haste,
                    strength_sound: ctf_weapon_strength_sound,
                    haste_sound: ctf_weapon_haste_sound,
                }),
                lmctf: None,
            },
        );
        SpawnModule {
            spawn: ctf_spawn,
            item_name: ctf_item_name,
            callbacks: ctf_callbacks(),
        }
    }

    /// Spawn CTF entities (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        if self.flags().spawn(entity.clone(), game) || self.techs().spawn(entity.clone(), game) {
            return true;
        }
        let classname = game.require_entity(&entity).classname.clone();
        match classname.as_str() {
            "info_player_team1" | "info_player_team2" => true,
            "misc_ctf_banner" | "misc_ctf_small_banner" => {
                {
                    let record = game.require_entity_mut(&entity);
                    record.model = if classname == "misc_ctf_banner" {
                        "models/ctf/banner/tris.md2".to_string()
                    } else {
                        "models/ctf/banner/small.md2".to_string()
                    };
                    record.skin = if record.spawnflags & 1 != 0 { 1 } else { 0 };
                }
                let frame = (game.random() * 16.0).floor() as i32;
                game.require_entity_mut(&entity).frame = frame;
                game.source_callbacks.register(&ctf_index_callbacks());
                game.set_solid(entity.clone(), Q2Solid::None);
                game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
                game.show(entity.clone());
                game.schedule(entity, 0.1, ctf_banner_think as Q2Think);
                true
            }
            "info_teleport_destination" => {
                let mut body = game.body_of(entity.clone());
                body.origin = add3(body.origin, vec3(0.0, 0.0, 16.0));
                game.write_body(entity, &body, true);
                true
            }
            "trigger_teleport" => {
                if game.require_entity(&entity).target.is_empty() {
                    game.host.diagnostic("teleporter without a target");
                    game.remove_actor(entity);
                    return true;
                }
                {
                    let record = game.require_entity_mut(&entity);
                    record.visible = false;
                    record.server_flags |= 1;
                    record.touch = Some(ctf_teleport_touch as Q2Touch);
                }
                game.source_callbacks.register(&ctf_index_callbacks());
                if game.require_entity(&entity).model.starts_with('*') {
                    let model = game.require_entity(&entity).model.clone();
                    let bounds = game.host.inline_model_bounds(model[1..].parse::<i32>().unwrap_or(0));
                    let mut body = game.body_of(entity.clone());
                    body.bounds = bounds;
                    game.write_body(entity.clone(), &body, true);
                }
                game.set_solid(entity.clone(), Q2Solid::Trigger);
                game.show(entity.clone());
                let sound = game.create("ctf_teleport_sound", BTreeMap::new());
                game.require_entity_mut(&entity).enemy = Some(sound.clone());
                let body = game.body_of(entity.clone());
                let mut moved = game.body_of(sound.clone());
                moved.origin = add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5));
                game.write_body(sound.clone(), &moved, true);
                let origin = game.body_of(sound.clone()).origin;
                game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                    actor: Some(sound),
                    origin,
                    path: "world/hum1.wav".to_string(),
                    channel: 0,
                    volume: 1.0,
                    attenuation: 1.0,
                    reliable: false,
                    loop_: Q2SoundLoop::Start,
                    loop_owner: None,
                }));
                true
            }
            _ => false,
        }
    }

    /// Run post-spawn setup (`afterSpawn`).
    pub fn after_spawn(&self, game: &mut Q2GameServices) {
        self.match_service().after_spawn(game);
        self.techs().setup(game);
    }

    /// Admit a player (`admitted`).
    pub fn admitted(&self, entity: ActorId, game: &mut Q2GameServices) {
        if let Some(equipment) = game.ctf.equipment {
            equipment.bind(game);
        }
        let disabled = matches!(
            game.ctf.shared.as_ref().map(|shared| shared.selection()),
            Some(GrappleSelection::Disabled)
        );
        if disabled {
            let owned = game.owned_of(entity.clone());
            game.host.inventory().configure(
                &owned,
                &InventoryEntry {
                    item: item_id("q2:weapon_grapple"),
                    count: 0.0,
                    capacity: 1.0,
                    count_policy: None,
                },
            );
        }
        if game.ctf.states.contains_key(&entity) {
            return;
        }
        let requested = match (self.hooks.player)(entity.clone(), game) {
            Some(player) => player.requested_spectator,
            None => panic!("CTF admission requires the shared player state"),
        };
        game.ctf.states.insert(entity.clone(), Q2CtfPlayerState::default());
        if game.options.deathmatch_flags & 131072 != 0
            && game.ctf.match_state.phase == Q2CtfMatchPhase::None
            && !requested
        {
            let mut one = 0;
            let mut two = 0;
            for member in game.ctf.states.values() {
                if member.team == 1 {
                    one += 1;
                } else if member.team == 2 {
                    two += 1;
                }
            }
            let team = if one < two {
                1
            } else if two < one {
                2
            } else if game.random() < 0.5 {
                1
            } else {
                2
            };
            if let Some(state) = game.ctf.states.get_mut(&entity) {
                state.team = team;
            }
            self.assign_skin(entity.clone(), game);
            self.spawn_player(entity, game);
            return;
        }
        self.set_observer(entity.clone(), game);
        self.presentation().join_menu(entity, game);
    }

    /// Assign the team skin (`assignSkin`).
    pub fn assign_skin(&self, entity: ActorId, game: &mut Q2GameServices) {
        let (userinfo, skin) = match (self.hooks.player)(entity.clone(), game) {
            Some(player) => (player.userinfo.clone(), player.skin.clone()),
            None => panic!("Missing shared CTF player"),
        };
        let team = ctf_player(game, &entity).team;
        let original = userinfo_value(&userinfo, "skin").unwrap_or(skin);
        let model = match original.rfind('/') {
            Some(slash) => original[..slash + 1].to_string(),
            None => "male/".to_string(),
        };
        (self.hooks.set_skin)(
            entity,
            game,
            if team == 0 {
                original
            } else {
                format!("{model}{}", if team == 1 { "ctf_r" } else { "ctf_b" })
            },
        );
    }

    /// Spawn a team player (`spawnPlayer`).
    fn spawn_player(&self, entity: ActorId, game: &mut Q2GameServices) {
        let use_weapons = match (self.hooks.player)(entity.clone(), game) {
            Some(player) => {
                player.spectator = false;
                player.requested_spectator = false;
                player.dead = false;
                player.noclip = false;
                player.god = false;
                player.use_q2_weapons
            }
            None => panic!("Missing shared CTF player"),
        };
        {
            let record = game.require_entity_mut(&entity);
            record.server_flags &= !1;
            record.visible = true;
        }
        (self.hooks.spawn_player)(entity.clone(), game);
        let team = ctf_player(game, &entity).team;
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                team: Some(Some(ctf_team_name(team).to_string())),
                ..CombatTraitChanges::default()
            },
        );
        if game.ctf.equipment.is_some() && use_weapons {
            game.host.inventory().configure(
                &owned,
                &InventoryEntry {
                    item: item_id("q2:weapon_grapple"),
                    count: 1.0,
                    capacity: 1.0,
                    count_policy: None,
                },
            );
        }
        self.assign_skin(entity, game);
    }

    /// Join a team (`join`).
    pub fn join(&self, entity: ActorId, game: &mut Q2GameServices, team: Q2CtfPlayingTeam, ghost: bool) -> bool {
        let phase = game.ctf.match_state.phase;
        let (match_lock, force) = (game.ctf.rules.match_lock, game.ctf.rules.force_join);
        if !ghost
            && (match_lock && (phase == Q2CtfMatchPhase::Pregame || phase == Q2CtfMatchPhase::Game)
                || force == Q2CtfForceJoin::Red && team != 1
                || force == Q2CtfForceJoin::Blue && team != 2)
        {
            return false;
        }
        self.drop_inventory(entity.clone(), game);
        if let Some(state) = game.ctf.states.get_mut(&entity) {
            state.team = team;
            state.spawn_state = 0;
            state.ready = false;
        }
        if phase == Q2CtfMatchPhase::Game && !ghost {
            self.match_service().assign_ghost(entity.clone(), game);
        }
        self.spawn_player(entity.clone(), game);
        game.host_emit(Q2PresentationEvent::EntityEvent {
            actor: entity.clone(),
            event: 6,
        });
        let name = ctf_name(game, &entity);
        ctf_print(
            game,
            &format!("{name} joined the {} team.\n", ctf_team_name(team)),
            None,
            Q2CtfPrintLevel::High,
        );
        if phase == Q2CtfMatchPhase::Setup {
            game.host_emit(Q2PresentationEvent::CenterPrint {
                actor: entity,
                text: "Type \"ready\" in console to ready up.".to_string(),
                instant: false,
                duration_seconds: None,
            });
        }
        true
    }

    /// Run the team command (`teamCommand`).
    pub fn team_command(&self, entity: ActorId, game: &mut Q2GameServices, name: &str) {
        let team = if name.to_lowercase() == "red" {
            Some(1)
        } else if name.to_lowercase() == "blue" {
            Some(2)
        } else {
            None
        };
        if name.is_empty() {
            let state_team = ctf_player(game, &entity).team;
            ctf_print(
                game,
                &format!("You are on the {} team.\n", ctf_team_name(state_team)),
                Some(entity),
                Q2CtfPrintLevel::High,
            );
            return;
        }
        let phase = game.ctf.match_state.phase;
        if phase != Q2CtfMatchPhase::None && phase != Q2CtfMatchPhase::Setup {
            ctf_print(
                game,
                "Can't change teams in a match.\n",
                Some(entity),
                Q2CtfPrintLevel::High,
            );
            return;
        }
        let Some(team) = team else {
            ctf_print(
                game,
                &format!("Unknown team {name}.\n"),
                Some(entity),
                Q2CtfPrintLevel::High,
            );
            return;
        };
        if ctf_player(game, &entity).team == team {
            ctf_print(
                game,
                &format!("You are already on the {} team.\n", ctf_team_name(team)),
                Some(entity),
                Q2CtfPrintLevel::High,
            );
            return;
        }
        let previous = ctf_player(game, &entity).team;
        let has_player = (self.hooks.player)(entity.clone(), game).is_some();
        self.drop_inventory(entity.clone(), game);
        if previous != 0 && has_player {
            if let Some(player) = (self.hooks.player)(entity.clone(), game) {
                player.god = false;
            }
            let owned = game.owned_of(entity.clone());
            game.host.combat().set_traits(
                &owned,
                &CombatTraitChanges {
                    invulnerable: Some(false),
                    ..CombatTraitChanges::default()
                },
            );
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
            if let Some(player) = (self.hooks.player)(entity.clone(), game) {
                player.score = 0;
            }
        }
        self.join(entity, game, team, false);
    }

    /// Move a player to an observer (`setObserver`).
    fn set_observer(&self, entity: ActorId, game: &mut Q2GameServices) {
        match (self.hooks.player)(entity.clone(), game) {
            Some(player) => {
                player.spectator = true;
                player.requested_spectator = true;
                player.noclip = true;
                player.god = false;
            }
            None => panic!("Missing shared CTF player"),
        }
        let owned = game.owned_of(entity.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                team: Some(None),
                can_take_damage: Some(false),
                invulnerable: Some(false),
                ..CombatTraitChanges::default()
            },
        );
        {
            let record = game.require_entity_mut(&entity);
            record.visible = false;
            record.server_flags |= 1;
        }
        game.set_solid(entity.clone(), Q2Solid::None);
        game.show(entity.clone());
        (self.hooks.observer)(entity, game);
    }

    /// Observe (`observer`).
    pub fn observer(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.drop_inventory(entity.clone(), game);
        if let Some(state) = game.ctf.states.get_mut(&entity) {
            state.team = 0;
            state.ready = false;
            state.match_respawn_at = None;
        }
        if let Some(player) = (self.hooks.player)(entity.clone(), game) {
            player.score = 0;
        }
        self.set_observer(entity.clone(), game);
        self.assign_skin(entity.clone(), game);
        self.presentation().join_menu(entity, game);
    }

    /// Select a spawn (`selectSpawn`).
    pub fn select_spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> Option<(Vec3, Vec3)> {
        let (team, spawn_state) = {
            let state = game.ctf.states.get(&entity)?;
            (state.team, state.spawn_state)
        };
        if team == 0 {
            return None;
        }
        let snapshot = (self.hooks.player)(entity.clone(), game).map(|player| player.clone())?;
        let mut spot = None;
        if spawn_state == 0 {
            if let Some(state) = game.ctf.states.get_mut(&entity) {
                state.spawn_state += 1;
            }
            let starts = q2_entities_named(game, &format!("info_player_team{team}"));
            let mut ranged: Vec<(ActorId, f64)> = starts
                .iter()
                .map(|spot| (spot.clone(), q2_players_range(game, spot.clone())))
                .collect();
            ranged.sort_by(|left, right| left.1.partial_cmp(&right.1).unwrap_or(std::cmp::Ordering::Equal));
            let candidates = if starts.len() > 2 {
                starts
                    .iter()
                    .filter(|spot| *spot != &ranged[0].0 && *spot != &ranged[1].0)
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                starts
            };
            spot = candidates
                .get((game.random() * candidates.len() as f64).floor() as usize)
                .cloned();
        }
        let spot = spot.unwrap_or_else(|| select_q2_spawn(game, &snapshot, ""));
        let origin = q2_spawn_origin(game, spot.clone());
        let angles = game.body_of(spot).angles;
        Some((origin, angles))
    }

    /// Score a frag (`score`).
    pub fn score(
        &self,
        victim: ActorId,
        attacker: Option<ActorId>,
        game: &mut Q2GameServices,
        change: i32,
        _means: i32,
        recipient: ActorId,
    ) {
        ctf_score(game, &recipient, change);
        self.flags().frag(victim, attacker, game);
    }

    /// Whether two actors share a team (`sameTeam`).
    pub fn same_team(&self, one: Option<ActorId>, two: ActorId, game: &Q2GameServices) -> bool {
        let first = one
            .as_ref()
            .and_then(|actor| game.ctf.states.get(actor).map(|state| state.team));
        first.is_some_and(|team| team != 0 && Some(team) == game.ctf.states.get(&two).map(|state| state.team))
    }

    /// Drop match inventory (`dropInventory`).
    ///
    /// Composition invokes this before the shared player clears death
    /// inventory.
    pub fn drop_inventory(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.grapple().reset(entity.clone(), game);
        self.flags().drop(entity.clone(), game);
        self.techs().drop(entity, game, true);
    }

    /// Run death cleanup (`death`).
    pub fn death(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.grapple().reset(entity, game);
    }

    /// Disconnect a player (`disconnect`).
    pub fn disconnect(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.match_service().sync_ghost(entity.clone(), game);
        self.drop_inventory(entity.clone(), game);
        if let Some(code) = game.ctf.states.get(&entity).and_then(|state| state.ghost_code) {
            if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                ghost.actor = None;
            }
        }
        if game
            .ctf
            .match_state
            .election
            .as_ref()
            .is_some_and(|election| election.target == entity)
        {
            game.ctf.match_state.election = None;
        }
        game.ctf.states.remove(&entity);
        if game.ctf.equipment.is_some() {
            game.equipment.ctf_states.remove(&entity);
        } else {
            game.ctf.inactive_grapple.remove(&entity);
        }
    }

    /// Run the pre-player frame (`beforePlayer`).
    pub fn before_player(&self, entity: ActorId, game: &mut Q2GameServices) {
        let respawn_at = match game.ctf.states.get(&entity) {
            Some(state) => state.match_respawn_at,
            None => return,
        };
        if let Some(at) = respawn_at {
            if at <= game.now() {
                if let Some(state) = game.ctf.states.get_mut(&entity) {
                    state.match_respawn_at = None;
                }
                self.spawn_player(entity, game);
            }
            return;
        }
        let team = game.ctf.states.get(&entity).map(|state| state.team).unwrap_or(0);
        let dead = (self.hooks.player)(entity.clone(), game).map(|player| (player.dead, player.respawn_time));
        if team != 0 && game.ctf.match_state.phase == Q2CtfMatchPhase::Game {
            if let Some((true, respawn_time)) = dead {
                if respawn_time < game.now() {
                    self.spawn_player(entity, game);
                }
            }
        }
    }

    /// Adjust weapon input (`weaponInput`).
    pub fn weapon_input(&self, entity: ActorId, game: &mut Q2GameServices, input: &Q2WeaponInput) -> Q2WeaponInput {
        let mut out = input.clone();
        out.haste = input.haste || self.techs().haste(entity, game);
        out.instant_switch = input.instant_switch || game.ctf.rules.instant_weapons;
        out
    }

    /// Run the post-player frame (`afterPlayer`).
    pub fn after_player(&self, entity: ActorId, game: &mut Q2GameServices) {
        if !game.ctf.states.contains_key(&entity) {
            return;
        }
        let team = game.ctf.states.get(&entity).map(|state| state.team).unwrap_or(0);
        let player = (self.hooks.player)(entity.clone(), game).map(|player| (player.use_q2_weapons, player.dead));
        if game.ctf.equipment.is_some()
            && team != 0
            && player == Some((true, false))
            && game.host.inventory().count(&entity, &item_id("q2:weapon_grapple")) == 0.0
        {
            let owned = game.owned_of(entity.clone());
            game.host.inventory().configure(
                &owned,
                &InventoryEntry {
                    item: item_id("q2:weapon_grapple"),
                    count: 1.0,
                    capacity: 1.0,
                    count_policy: None,
                },
            );
        }
        self.grapple()
            .player_frame(entity.clone(), game, Q2CtfGrappleBinding::WeaponSlot);
        self.techs().regenerate(entity.clone(), game);
        self.flags().effects(entity.clone(), game);
        self.match_service().sync_ghost(entity.clone(), game);
        let status = self.match_service().status(game);
        self.presentation().hud(entity, game, status);
    }

    /// Sync ghosts after player frames (`afterPlayerFrames`).
    pub fn after_player_frames(&self, game: &mut Q2GameServices) {
        for actor in game.host.players() {
            self.match_service().sync_ghost(actor, game);
        }
    }

    /// Check match rules (`checkRules`).
    pub fn check_rules(&self, game: &mut Q2GameServices) -> bool {
        self.match_service().check_rules(game)
    }

    /// Whether a match is setting up (`matchSetup`).
    pub fn match_setup(&self, game: &Q2GameServices) -> bool {
        game.ctf.match_state.phase == Q2CtfMatchPhase::Setup || game.ctf.match_state.phase == Q2CtfMatchPhase::Pregame
    }

    /// Whether pickups are allowed (`pickupsAllowed`).
    pub fn pickups_allowed(&self, game: &Q2GameServices) -> bool {
        !self.match_setup(game)
    }

    /// Reset players (`resetPlayers`).
    pub fn reset_players(&self, game: &mut Q2GameServices) {
        for actor in game.host.players() {
            if game.entity(&actor).is_some() && game.ctf.states.contains_key(&actor) {
                self.observer(actor, game);
            }
        }
        self.techs().reset(game);
        self.flags().reset_all(game);
        let respawn = crate::q2::foundation::items::Q2ItemModule::respawn_item_callback();
        let now = game.now();
        let ids: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        for id in ids {
            let record = game.require_entity(&id);
            if record.solid == Q2Solid::None
                && record.think.is_some_and(|think| std::ptr::fn_addr_eq(think, respawn))
                && record.next_think.is_some_and(|at| at >= now)
            {
                game.cancel_actor(id.clone());
                respawn(id, game);
            }
        }
    }

    /// Run a client command (`command`).
    pub fn command(&self, entity: ActorId, game: &mut Q2GameServices, command: &str, args: &[String]) -> bool {
        if command == "hook" || command == "+hook" || command == "unhook" || command == "-hook" {
            self.grapple()
                .command(entity, game, command == "hook" || command == "+hook");
            return true;
        }
        if !game.ctf.states.contains_key(&entity) {
            return false;
        }
        let words = args.join(" ");
        match command.to_lowercase().as_str() {
            "ctf-menu" => {
                if args.len() != 1 {
                    return false;
                }
                let Some(action) = Q2CtfMenuAction::parse(&args[0]) else {
                    return false;
                };
                self.menu_action(entity, game, action);
            }
            "ctf-settings" => {
                if args.len() != 2 || !ctf_player(game, &entity).admin {
                    return false;
                }
                let (key, value) = (&args[0], &args[1]);
                let rules = &game.ctf.rules;
                let flags = game.options.deathmatch_flags;
                let mut settings = Q2CtfAdminSettings {
                    match_minutes: rules.match_minutes,
                    setup_minutes: rules.setup_minutes,
                    start_seconds: rules.start_seconds,
                    weapons_stay: flags & 4 != 0,
                    instant_items: flags & 16 != 0,
                    quad_drop: flags & 16384 != 0,
                    instant_weapons: rules.instant_weapons,
                    match_lock: rules.match_lock,
                };
                match key.as_str() {
                    "matchMinutes" | "setupMinutes" | "startSeconds" => {
                        let Ok(number) = value.parse::<f64>() else {
                            return false;
                        };
                        if value.trim().is_empty() || !number.is_finite() || number <= 0.0 {
                            return false;
                        }
                        match key.as_str() {
                            "matchMinutes" => settings.match_minutes = number,
                            "setupMinutes" => settings.setup_minutes = number,
                            _ => settings.start_seconds = number,
                        }
                    }
                    "weaponsStay" | "instantItems" | "quadDrop" | "instantWeapons" | "matchLock" => {
                        if value != "true" && value != "false" && value != "1" && value != "0" {
                            return false;
                        }
                        let enabled = value == "true" || value == "1";
                        match key.as_str() {
                            "weaponsStay" => settings.weapons_stay = enabled,
                            "instantItems" => settings.instant_items = enabled,
                            "quadDrop" => settings.quad_drop = enabled,
                            "instantWeapons" => settings.instant_weapons = enabled,
                            _ => settings.match_lock = enabled,
                        }
                    }
                    _ => return false,
                }
                self.match_service().configure(entity.clone(), game, &settings);
                self.match_service().settings_menu(entity, game);
            }
            "team" => self.team_command(entity, game, &words),
            "observer" => self.observer(entity, game),
            "id" => {
                let id_view = {
                    let state = ctf_player(game, &entity);
                    state.id_view = !state.id_view;
                    state.id_view
                };
                ctf_print(
                    game,
                    &format!(
                        "Disabling player identification {}.\n",
                        if id_view { "off" } else { "on" }
                    ),
                    Some(entity),
                    Q2CtfPrintLevel::High,
                );
            }
            "say_team" => self.presentation().say_team(entity, game, &words),
            "score" | "help" => {
                let show = match (self.hooks.player)(entity.clone(), game) {
                    Some(player) => {
                        player.show_scores = !player.show_scores;
                        player.show_scores
                    }
                    None => false,
                };
                if show {
                    self.presentation().scoreboard(entity, game);
                }
            }
            "inven" => {
                if ctf_player(game, &entity).team != 0 {
                    return false;
                }
                self.presentation().join_menu(entity, game);
            }
            "ready" => {
                self.match_service().ready(entity, game, true);
            }
            "notready" => {
                self.match_service().ready(entity, game, false);
            }
            "yes" => {
                self.match_service().vote(entity, game, true);
            }
            "no" => {
                self.match_service().vote(entity, game, false);
            }
            "ghost" => {
                if !words.is_empty() && words.bytes().all(|byte| byte.is_ascii_digit()) {
                    if let Ok(code) = words.parse::<i32>() {
                        self.match_service().restore_ghost(entity, game, code);
                    }
                }
            }
            "admin" => {
                self.match_service().admin(entity, game, &words);
            }
            "stats" => self.presentation().stats(entity, game),
            "warp" => {
                self.match_service().warp(entity, game, &words);
            }
            "boot" => {
                self.match_service().boot(entity, game, &words);
            }
            "playerlist" => {
                let now = game.now();
                let mut lines = String::new();
                for actor in game.host.players() {
                    let common = (self.hooks.player)(actor.clone(), game).map(|player| {
                        (
                            player.slot,
                            player.entered_at,
                            player.ping,
                            player.score,
                            player.name.clone(),
                        )
                    });
                    let member = game.ctf.states.get(&actor).map(|state| (state.team, state.admin));
                    let (Some(common), Some(member)) = (common, member) else {
                        continue;
                    };
                    lines.push_str(&format!(
                        "{} {}:{} {} {} {}{}{}\n",
                        common.0 + 1,
                        ((now - common.1) / 60.0).floor() as i32,
                        (now - common.1).trunc() as i32 % 60,
                        common.2,
                        common.3,
                        common.4,
                        if member.0 == 0 { " (spectator)" } else { "" },
                        if member.1 { " (admin)" } else { "" }
                    ));
                }
                let lines: String = lines.chars().take(1399).collect();
                ctf_print(game, &lines, Some(entity), Q2CtfPrintLevel::High);
            }
            "drop" => {
                if words.to_lowercase() != "tech" {
                    return false;
                }
                if ctf_tech(game, &entity).is_some() {
                    self.techs().drop(entity, game, false);
                }
            }
            _ => return false,
        }
        true
    }

    /// Run a menu action (`menuAction`).
    pub fn menu_action(&self, entity: ActorId, game: &mut Q2GameServices, action: Q2CtfMenuAction) {
        match action {
            Q2CtfMenuAction::JoinRed => {
                self.join(entity, game, 1, false);
            }
            Q2CtfMenuAction::JoinBlue => {
                self.join(entity, game, 2, false);
            }
            Q2CtfMenuAction::Observer => self.observer(entity, game),
            Q2CtfMenuAction::Chase => (self.hooks.chase)(entity, game),
            Q2CtfMenuAction::Match => {
                self.match_service()
                    .begin_election(entity, game, Q2CtfElectionKind::Match, "");
            }
            Q2CtfMenuAction::Ready => {
                self.match_service().ready(entity, game, true);
            }
            Q2CtfMenuAction::NotReady => {
                self.match_service().ready(entity, game, false);
            }
            Q2CtfMenuAction::AdminStart => {
                if ctf_player(game, &entity).admin {
                    if game.ctf.match_state.phase == Q2CtfMatchPhase::Setup {
                        game.ctf.match_state.phase = Q2CtfMatchPhase::Pregame;
                        let start = game.ctf.rules.start_seconds;
                        game.ctf.match_state.match_time = game.now() + start;
                    } else {
                        self.match_service().setup(game);
                    }
                }
            }
            Q2CtfMenuAction::AdminCancel => {
                if ctf_player(game, &entity).admin {
                    game.ctf.match_state.phase = Q2CtfMatchPhase::None;
                    self.reset_players(game);
                }
            }
            Q2CtfMenuAction::AdminSettings => self.match_service().settings_menu(entity, game),
            Q2CtfMenuAction::Credits => (self.hooks.emit)(
                game,
                Q2CtfEvent::Menu {
                    actor: entity,
                    title: "ThreeWave CTF credits".to_string(),
                    entries: vec![
                        Q2CtfMenuEntry {
                            label: "Design and code: David 'Zoid' Kirsch".to_string(),
                            action: None,
                        },
                        Q2CtfMenuEntry {
                            label: "id Software Quake II".to_string(),
                            action: None,
                        },
                        Q2CtfMenuEntry {
                            label: "Close".to_string(),
                            action: Some(Q2CtfMenuAction::Close),
                        },
                    ],
                },
            ),
            Q2CtfMenuAction::Close => (self.hooks.emit)(
                game,
                Q2CtfEvent::Menu {
                    actor: entity,
                    title: String::new(),
                    entries: Vec::new(),
                },
            ),
        }
    }

    /// Capture the match (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2CtfCheckpoint {
        capture_q2_ctf(game)
    }

    /// Restore the match (`restore`).
    pub fn restore(&self, checkpoint: &Q2CtfCheckpoint, game: &mut Q2GameServices) {
        restore_q2_ctf(game, checkpoint);
    }
}
