//! Q2 rerelease players (`src/content/q2/rerelease/players.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, length3, scale3, vec3, Vec3, Vec4};

use crate::contract::{InventoryEntry, ItemId, PoweredProtectionState, RegularArmorState};
use crate::q2::base::player::commands::{parse_command_int, userinfo_value};
use crate::q2::base::player::index::{
    create_q2_players, player_callbacks, player_hooks, player_items, Q2ConnectionResult, Q2Intermission,
    Q2PlayerOverrides, Q2Players, Q2SpawnSolution,
};
use crate::q2::base::player::landmarks::place_q2_landmark;
use crate::q2::base::player::obituary::Q2ObituaryRecipient;
use crate::q2::base::player::spawns::q2_entities_named;
use crate::q2::base::player::types::{
    Q2PlayerCarry, Q2PlayerContext, Q2PlayerEvent, Q2PlayerHand, Q2PlayerMovementChange, Q2PlayerSpawnChange,
    Q2PlayerView,
};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::restore_q2_actor;
use crate::q2::foundation::fields::movedir;
use crate::q2::foundation::host::{
    Q2GameServices, Q2LandmarkCarry, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2TraceRequest,
};
use crate::q2::foundation::items::Q2ItemKind;
use crate::q2::foundation::weapons::player::registered_weapon_definitions;
use crate::q2::support::contracts::{
    AttackCause, CombatTraitChanges, DamageDecision, DamageFeedback, DamageReactionKind, DeathReaction, Q2NativeCause,
    TraceContact, TraceHit,
};

use super::checkpoint::{Q2RereleasePlayerCheckpointEntry, Q2RereleasePlayersCheckpoint};
use super::environment::{q2_rerelease_falling_damage, q2_rerelease_world_effects};
use super::killbox::kill_q2_rerelease_box;
use super::obituary::q2_rerelease_obituary;
use super::spawns::{q2_rerelease_spawns_callbacks, select_q2_rerelease_spawn};
use super::types::{
    q2_is_n64, q2_uses_instanced_items, Q2CoopRespawnState, Q2LocalizedPrintLevel, Q2PendingLandmark, Q2RereleaseEvent,
    Q2RereleasePlayerState,
};
use super::{rerelease_hooks, sort_rerelease_actors};

/// Rerelease squad spawn placement (`squadSpawns` entry).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseSquadSpawn {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Rerelease selected spawn placement (`selectedSpawns` entry).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseSelectedSpawn {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Whether placed from a landmark.
    pub from_landmark: bool,
}

/// Rerelease player extension (`Q2RereleasePlayerExtension`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleasePlayerExtension {
    /// Admitted handler.
    pub admitted: fn(ActorId, &mut Q2GameServices),
    /// End-player-frame handler.
    pub end_player_frame: fn(ActorId, &mut Q2GameServices),
    /// Spawned handler.
    pub spawned: fn(ActorId, &mut Q2GameServices),
    /// Before-level-change handler.
    pub before_level_change: fn(&mut Q2GameServices),
    /// End-of-unit handler.
    pub end_of_unit: fn(&mut Q2GameServices),
    /// Leave-unit handler.
    pub leave_unit: fn(&mut Q2GameServices),
    /// Begin-player-frame handler.
    pub begin_player_frame: fn(ActorId, &mut Q2GameServices),
    /// Help handler.
    pub help: fn(ActorId, &mut Q2GameServices),
}

/// Rerelease players (`Q2RereleasePlayers`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleasePlayers {
    /// Base players module.
    pub players: Q2Players,
}

/// Read the rerelease extra state for an admitted player.
pub fn rerelease_extra(game: &Q2GameServices, actor: &ActorId) -> Q2RereleasePlayerState {
    game.rerelease
        .states
        .get(actor)
        .expect("Q2 rerelease player is not admitted")
        .clone()
}

/// Admit rerelease extra state for a player, reporting whether it is new.
fn admit_rerelease_extra(game: &mut Q2GameServices, actor: &ActorId) -> bool {
    if game.rerelease.states.contains_key(actor) {
        return false;
    }
    let identity = (rerelease_hooks(game).player_identity)(game, actor.clone());
    let mut extra = Q2RereleasePlayerState::new(identity.seat, identity.social_id);
    if game.options.mode == Q2Mode::Coop && game.rerelease.options.coop_lives {
        extra.lives = game.rerelease.options.coop_num_lives + 1;
    }
    game.rerelease.states.insert(actor.clone(), extra);
    true
}

/// Rerelease player callbacks (`Q2RereleasePlayers[callbacks]`).
pub fn rerelease_player_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = player_callbacks();
    let spawns = q2_rerelease_spawns_callbacks();
    callbacks.think.extend(spawns.think);
    callbacks.use_.extend(spawns.use_);
    callbacks.touch.extend(spawns.touch);
    callbacks.pain.extend(spawns.pain);
    callbacks.die.extend(spawns.die);
    callbacks.blocked.extend(spawns.blocked);
    callbacks.trajectory.extend(spawns.trajectory);
    callbacks
}

/// Rerelease player spawn (`Q2RereleasePlayers[spawn]`).
pub fn rerelease_player_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    super::spawns::q2_rerelease_spawns_spawn(entity, game)
}

/// Build the rerelease override table (`Q2RereleasePlayers` overrides).
pub fn rerelease_player_overrides() -> Q2PlayerOverrides {
    Q2PlayerOverrides {
        connect: Some(rerelease_connect),
        userinfo_changed: Some(rerelease_userinfo_changed),
        restore_carry: Some(rerelease_restore_carry),
        obituary: Some(rerelease_obituary),
        clear_death_inventory: Some(rerelease_clear_death_inventory),
        record_death: Some(rerelease_record_death),
        dead_frame: Some(rerelease_dead_frame),
        world_effects: Some(rerelease_world_effects),
        falling_damage: Some(rerelease_falling_damage),
        build_view: Some(rerelease_build_view),
        damage_feedback: Some(rerelease_damage_feedback),
        client_animation: Some(rerelease_client_animation),
        update_bob: Some(rerelease_update_bob),
        spawn_placement: Some(rerelease_spawn_placement),
        kill_box: Some(rerelease_kill_box),
        put_in_server: Some(rerelease_put_in_server),
        begin_intermission: Some(rerelease_begin_intermission),
        before_exit_level: Some(rerelease_before_exit_level),
        end_frame: Some(rerelease_end_frame),
        can_drop_coop_stay_items: Some(rerelease_can_drop_coop_stay_items),
        death: Some(rerelease_death),
        save_carry: Some(rerelease_save_carry),
        respawn: Some(rerelease_respawn),
    }
}

/// Bind the base module for `super` behavior.
pub(crate) fn base_module(game: &Q2GameServices) -> Q2Players {
    create_q2_players(player_items(game), player_hooks(game))
}

/// Connect a player (`connect`).
fn rerelease_connect(game: &mut Q2GameServices, userinfo: String) -> Q2ConnectionResult {
    let spectator = game.options.mode == Q2Mode::Deathmatch
        && userinfo_value(&userinfo, "spectator").is_some_and(|value| !value.is_empty() && value != "0");
    let password = if spectator {
        game.players.rules.spectator_password.clone()
    } else {
        game.players.rules.password.clone()
    };
    let mut reason = String::new();
    if !password.is_empty()
        && password != "none"
        && password != userinfo_value(&userinfo, if spectator { "spectator" } else { "password" }).unwrap_or_default()
    {
        reason = if spectator {
            "Spectator password required or incorrect.".to_string()
        } else {
            "Password required or incorrect.".to_string()
        };
    } else if spectator
        && game
            .players
            .states
            .values()
            .filter(|state| state.connected && state.requested_spectator)
            .count() as i32
            >= game.players.rules.max_spectators
    {
        reason = "Server spectator limit is full.".to_string();
    }
    if reason.is_empty() {
        return Q2ConnectionResult {
            allowed: true,
            userinfo,
            reason: None,
        };
    }
    Q2ConnectionResult {
        allowed: false,
        userinfo: format_userinfo_with_rejmsg(&userinfo_value_pairs(&userinfo), &reason),
        reason: Some(reason),
    }
}

/// Read userinfo pairs in order.
fn userinfo_value_pairs(source: &str) -> Vec<(String, String)> {
    let stripped = source.strip_prefix('\\').unwrap_or(source);
    let fields: Vec<&str> = stripped.split('\\').collect();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < fields.len() {
        let key = fields[index].to_string();
        if !pairs.iter().any(|(seen, _): &(String, String)| *seen == key) {
            pairs.push((key, fields[index + 1].to_string()));
        }
        index += 2;
    }
    pairs
}

/// Rebuild userinfo with a rejection message.
fn format_userinfo_with_rejmsg(pairs: &[(String, String)], reason: &str) -> String {
    let mut out = String::new();
    for (key, value) in pairs.iter().filter(|(key, _)| key != "rejmsg") {
        out.push('\\');
        out.push_str(key);
        out.push('\\');
        out.push_str(value);
    }
    out.push_str("\\rejmsg\\");
    out.push_str(reason);
    out
}

impl Q2RereleasePlayers {
    /// Capture rerelease players (`captureRerelease`).
    pub fn capture_rerelease(&self, game: &Q2GameServices) -> Q2RereleasePlayersCheckpoint {
        let mut actors: Vec<ActorId> = game.rerelease.states.keys().cloned().collect();
        sort_rerelease_actors(&mut actors);
        let players = actors
            .into_iter()
            .map(|actor| Q2RereleasePlayerCheckpointEntry {
                actor: SavedActorId::from(&actor),
                state: game
                    .rerelease
                    .states
                    .get(&actor)
                    .expect("Q2 rerelease player is not admitted")
                    .clone(),
            })
            .collect();
        let mut squads: Vec<ActorId> = game.rerelease.squad_spawns.keys().cloned().collect();
        sort_rerelease_actors(&mut squads);
        let squad_spawns = squads
            .into_iter()
            .map(|actor| {
                let placement = game
                    .rerelease
                    .squad_spawns
                    .get(&actor)
                    .expect("Q2 rerelease squad spawn is missing");
                super::checkpoint::Q2RereleaseSquadSpawn {
                    actor: SavedActorId::from(&actor),
                    origin: placement.origin,
                    angles: placement.angles,
                }
            })
            .collect();
        Q2RereleasePlayersCheckpoint {
            version: 1,
            options: game.rerelease.options.clone(),
            coop_restart_time: game.rerelease.coop_restart_time,
            deadly_kill_box: game.rerelease.deadly_kill_box,
            intermission_flags: game.rerelease.intermission_flags,
            intermission_fade_until: game.rerelease.intermission_fade_until,
            intermission_camera: game.rerelease.intermission_camera,
            intermission_camera_set: game.rerelease.intermission_camera_set,
            players,
            squad_spawns,
        }
    }

    /// Restore rerelease players (`restoreRerelease`).
    pub fn restore_rerelease(&self, game: &mut Q2GameServices, checkpoint: &Q2RereleasePlayersCheckpoint) {
        game.rerelease.states.clear();
        game.rerelease.squad_spawns.clear();
        game.rerelease.options = checkpoint.options.clone();
        game.rerelease.coop_restart_time = checkpoint.coop_restart_time;
        game.rerelease.deadly_kill_box = checkpoint.deadly_kill_box;
        game.rerelease.intermission_flags = checkpoint.intermission_flags;
        game.rerelease.intermission_fade_until = checkpoint.intermission_fade_until;
        game.rerelease.intermission_camera = checkpoint.intermission_camera;
        game.rerelease.intermission_camera_set = checkpoint.intermission_camera_set;
        for entry in &checkpoint.players {
            let actor = restore_q2_actor(game, entry.actor).id().clone();
            game.rerelease.states.insert(actor, entry.state.clone());
        }
        for entry in &checkpoint.squad_spawns {
            let actor = restore_q2_actor(game, entry.actor).id().clone();
            game.rerelease.squad_spawns.insert(
                actor,
                Q2RereleaseSquadSpawn {
                    origin: entry.origin,
                    angles: entry.angles,
                },
            );
        }
    }
}

/// Admit a rerelease player after the base attach (`attach` extension half).
///
/// The base attach runs first (admitting player state, inventory, combat,
/// weapons and the dispatched userinfo, which lazily admits the extra
/// state); this finishes the donor `attach` tail.
pub fn rerelease_admit_player(actor: ActorId, game: &mut Q2GameServices) {
    admit_rerelease_extra(game, &actor);
    let auto_shield = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .auto_shield;
    if auto_shield >= 0 {
        game.require_entity_mut(&actor).flags |= 0x40000000;
    }
    if let Some(extension) = game.rerelease.extension {
        (extension.admitted)(actor, game);
    }
}

/// Apply userinfo (`userinfoChanged`).
fn rerelease_userinfo_changed(actor: ActorId, game: &mut Q2GameServices, userinfo: String) {
    if !game.players.states.contains_key(&actor) {
        panic!("Q2 rerelease userinfo requires an admitted player");
    }
    admit_rerelease_extra(game, &actor);
    let clipped: String = userinfo.chars().take(2047).collect();
    let name: String = userinfo_value(&clipped, "name")
        .unwrap_or_else(|| "badinfo".to_string())
        .chars()
        .take(31)
        .collect();
    let skin = userinfo_value(&clipped, "skin").unwrap_or_else(|| "male/grunt".to_string());
    let requested_spectator = game.options.mode == Q2Mode::Deathmatch
        && userinfo_value(&clipped, "spectator").is_some_and(|value| !value.is_empty() && value != "0");
    let fov = parse_command_int(userinfo_value(&clipped, "fov").as_deref()).clamp(1, 160);
    let hand = match parse_command_int(userinfo_value(&clipped, "hand").as_deref()).clamp(0, 2) {
        1 => Q2PlayerHand::Left,
        2 => Q2PlayerHand::Center,
        _ => Q2PlayerHand::Right,
    };
    let auto_switch = parse_command_int(userinfo_value(&clipped, "autoswitch").as_deref()).clamp(0, 3);
    let auto_switch = match auto_switch {
        1 => 1,
        2 => 2,
        3 => 3,
        _ => 0,
    };
    let bob_skip = userinfo_value(&clipped, "bobskip").is_some_and(|value| value.starts_with('1'));
    let auto_shield = if userinfo_value(&clipped, "autoshield").is_some() {
        parse_command_int(userinfo_value(&clipped, "autoshield").as_deref())
    } else {
        -1
    };
    let dogtag = userinfo_value(&clipped, "dogtag").unwrap_or_default();
    {
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease userinfo requires an admitted player");
        entry.userinfo = clipped;
        entry.skin = skin;
        entry.requested_spectator = requested_spectator;
        entry.fov = fov;
        entry.hand = hand;
    }
    {
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.bob_skip = bob_skip;
        extra.auto_switch = auto_switch;
        extra.auto_shield = auto_shield;
        extra.dogtag = dogtag;
    }
    let (slot, skin, server_flags) = {
        let record = game.require_entity(&actor);
        let state = game
            .players
            .states
            .get(&actor)
            .expect("Q2 rerelease userinfo requires an admitted player");
        (state.slot, state.skin.clone(), record.server_flags)
    };
    (player_hooks(game).emit)(Q2PlayerEvent::Userinfo {
        actor: actor.clone(),
        slot,
        name: name.clone(),
        skin,
    });
    let dogtag = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .dogtag
        .clone();
    (rerelease_hooks(game).emit)(
        game,
        Q2RereleaseEvent::PlayerDogtag {
            actor: actor.clone(),
            value: dogtag,
        },
    );
    let entry = game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 rerelease userinfo requires an admitted player");
    entry.name = if server_flags & 16 != 0 {
        name
    } else {
        format!("##P{}", entry.slot)
    };
}

impl Q2RereleasePlayers {
    /// Auto-switch on weapon pickup (`weaponPicked`).
    pub fn weapon_picked(&self, player: ActorId, game: &mut Q2GameServices, item: ItemId, first: bool) {
        let weapon = registered_weapon_definitions(game)
            .into_iter()
            .find(|definition| definition.item == item);
        let state = game.weapons.states.get(&player).cloned();
        let (Some(weapon), Some(state)) = (weapon, state) else {
            return;
        };
        if state.weapon.as_ref() == Some(&weapon.name) || state.pending.as_ref() == Some(&weapon.name) {
            return;
        }
        let ammo_weapon = player_items(game)
            .lookup(game, &item)
            .is_some_and(|descriptor| descriptor.kind == Q2ItemKind::Ammo);
        let required = if ammo_weapon { 1.0 } else { f64::from(weapon.quantity) };
        if weapon
            .ammo
            .as_ref()
            .is_some_and(|ammo| game.host.inventory().count(&player, ammo) < required)
        {
            return;
        }
        let mode = game
            .rerelease
            .states
            .get(&player)
            .expect("Q2 rerelease player is not admitted")
            .auto_switch;
        if mode == 3
            || mode == 2 && ammo_weapon
            || mode == 0
                && state.weapon.as_deref() != Some("blaster")
                && (game.options.mode == Q2Mode::Deathmatch || !first)
        {
            return;
        }
        if let Some(entry) = game.weapons.states.get_mut(&player) {
            entry.pending = Some(weapon.name);
        }
    }

    /// Report a weapon firing (`recordWeaponFire`).
    pub fn record_weapon_fire(&self, game: &mut Q2GameServices, actor: ActorId, now: f64) {
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .last_firing_until = now + 2.5;
    }

    /// Reveal invisibility (`revealInvisibility`).
    pub fn reveal_invisibility(&self, game: &mut Q2GameServices, actor: ActorId, until: f64) {
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .invisibility_fade_until = until;
    }

    /// Record a movement impact (`movementImpact`).
    pub fn movement_impact(&self, game: &mut Q2GameServices, actor: ActorId, impact_delta: f64, on_ladder: bool) {
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.impact_delta = impact_delta;
        extra.on_ladder = on_ladder;
    }
}

/// Emit a rerelease obituary, returning `None` to skip the base print.
fn rerelease_obituary(actor: ActorId, game: &mut Q2GameServices, attacker: Option<ActorId>) -> Option<String> {
    let cause = game.require_entity(&actor).last_attack.clone();
    let victim = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .clone();
    let attacker_state = attacker
        .as_ref()
        .and_then(|attacker| game.players.states.get(attacker).cloned());
    let means = match cause.as_ref().map(|attack| &attack.cause) {
        Some(AttackCause::Q2 { means_of_death, .. }) => *means_of_death,
        _ => 0,
    };
    let no_point_loss = match cause.as_ref().map(|attack| &attack.cause) {
        Some(AttackCause::Q2 {
            native: Some(Q2NativeCause::Rerelease { no_point_loss, .. }),
            ..
        }) => *no_point_loss,
        _ => false,
    };
    let suicide = attacker.as_ref() == Some(&actor);
    let victim_actor = actor.clone();
    let attacker_actor = attacker.clone();
    let mode = game.options.mode;
    let base = base_module(game);
    let mut apply = |recipient: Q2ObituaryRecipient, change: i32| {
        let recipient = match recipient {
            Q2ObituaryRecipient::Victim => victim_actor.clone(),
            Q2ObituaryRecipient::Attacker => attacker_actor.clone().unwrap_or(victim_actor.clone()),
        };
        base.apply_score(
            victim_actor.clone(),
            attacker_actor.clone(),
            game,
            change,
            means,
            recipient,
        );
    };
    let obituary = q2_rerelease_obituary(
        &victim,
        attacker_state.as_ref(),
        suicide,
        means,
        mode,
        no_point_loss,
        &mut apply,
    );
    (rerelease_hooks(game).emit)(
        game,
        Q2RereleaseEvent::LocalizedPrint {
            actor: None,
            level: Q2LocalizedPrintLevel::Medium,
            text: obituary.text,
            args: obituary.args,
        },
    );
    None
}

/// Clear the death inventory (`clearDeathInventory`).
fn rerelease_clear_death_inventory(actor: ActorId, game: &mut Q2GameServices) {
    if game.options.mode == Q2Mode::Coop && !q2_uses_instanced_items(&game.rerelease.options) {
        base_module(game).clear_death_inventory(actor, game);
    }
}

/// Whether coop-stay items may drop (`canDropCoopStayItems`).
fn rerelease_can_drop_coop_stay_items(game: &Q2GameServices) -> bool {
    game.options.mode == Q2Mode::Coop && q2_uses_instanced_items(&game.rerelease.options)
}

/// Run death (`death`).
fn rerelease_death(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction, callback: bool) {
    let means = game
        .require_entity(&actor)
        .last_attack
        .as_ref()
        .map(|attack| match &attack.cause {
            AttackCause::Q2 { means_of_death, .. } => means_of_death & !0x8000000,
            _ => 0,
        })
        .unwrap_or(0);
    {
        let record = game.require_entity_mut(&actor);
        record.model2 = String::new();
        record.model3 = String::new();
        record.sound = String::new();
    }
    game.players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted")
        .loop_sound = String::new();
    if means == 51 {
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_health(&owned, -100.0);
        let mut reaction = reaction;
        reaction.pain.damage = 400.0;
        return base_module(game).death(actor, game, reaction, callback);
    }
    if means == 47 && game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) < -80.0 {
        game.require_entity_mut(&actor).flags |= 0x10000;
    }
    base_module(game).death(actor, game, reaction, callback)
}

/// Record damage (`recordDamage`).
pub fn rerelease_record_damage(actor: ActorId, game: &mut Q2GameServices, decision: &DamageDecision) {
    base_module(game).record_damage(actor.clone(), game, decision);
    if decision
        .feedback
        .as_ref()
        .is_some_and(|feedback| matches!(feedback, DamageFeedback::Q2 { .. }))
        && decision.reaction != DamageReactionKind::Death
    {
        let now = game.now();
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .last_damage_until = now + 2.0;
    }
}

/// Save the carry (`saveCarry`).
fn rerelease_save_carry(actor: ActorId, game: &mut Q2GameServices) -> Q2PlayerCarry {
    let mut carry = base_module(game).save_carry(actor.clone(), game);
    carry.flags |= game.require_entity(&actor).flags & (0x400000 | 0x40000000);
    carry
}

/// Restore a carry (`restoreCarry`).
fn rerelease_restore_carry(actor: ActorId, game: &mut Q2GameServices, carry: Q2PlayerCarry) {
    base_module(game).restore_carry(actor.clone(), game, carry.clone());
    if carry.health <= 0.0 {
        let owned = game.owned_of(actor.clone());
        for entry in game.host.inventory().entries(&actor) {
            game.host.inventory().configure(
                &owned,
                &InventoryEntry {
                    item: entry.item.clone(),
                    count: 0.0,
                    capacity: entry.capacity,
                    count_policy: entry.count_policy,
                },
            );
        }
        let use_q2_inventory = game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .use_q2_inventory;
        if use_q2_inventory {
            player_items(game).configure_player(&owned, game, true);
        } else {
            let spawn = game
                .players
                .states
                .get(&actor)
                .expect("Q2 player has not been admitted")
                .spawn_inventory
                .clone();
            for entry in spawn {
                game.host.inventory().configure(&owned, &entry);
            }
        }
        game.host.combat().set_health(&owned, 100.0);
        game.host.combat().set_armor(
            &owned,
            &crate::contract::ArmorState {
                regular: RegularArmorState::None,
                powered: PoweredProtectionState::None,
            },
        );
        {
            let record = game.require_entity_mut(&actor);
            record.max_health = 100.0;
            record.flags &= !(16 | 32 | 4096 | 0x400000 | 0x40000000);
            record.power_cubes = 0;
        }
        {
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.god = false;
            entry.notarget = false;
            entry.selected_item = if use_q2_inventory {
                Some("q2:weapon_blaster".to_string())
            } else {
                entry.selected_item.clone()
            };
        }
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: None,
                mass: None,
                invulnerable: Some(false),
                team: None,
                no_knockback: None,
            },
        );
        if let Some(weapon) = game.weapons.states.get_mut(&actor) {
            weapon.weapon = Some("blaster".to_string());
            weapon.pending = None;
        }
        if use_q2_inventory {
            if let Some(initialized) = player_hooks(game).persistent_inventory_initialized {
                initialized(actor.clone(), game);
            }
        }
        if let Some(extension) = game.rerelease.extension {
            (extension.spawned)(actor.clone(), game);
        }
        let carry = base_module(game).save_carry(actor.clone(), game);
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .coop_respawn = Some(carry);
    }
    let flashlight = carry.flags & 0x400000 != 0;
    game.rerelease
        .states
        .get_mut(&actor)
        .expect("Q2 rerelease player is not admitted")
        .flashlight = flashlight;
}

/// Record death (`recordDeath`).
fn rerelease_record_death(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) -> bool {
    let carry = base_module(game).save_carry(actor.clone(), game);
    let max_health = game.require_entity(&actor).max_health;
    let first = base_module(game).record_death(actor.clone(), game, reaction);
    game.rerelease
        .states
        .get_mut(&actor)
        .expect("Q2 rerelease player is not admitted")
        .invisibility_until = 0.0;
    if !first {
        return false;
    }
    {
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.animation_time = 0.0;
    }
    let options = game.rerelease.options.clone();
    if game.options.mode == Q2Mode::Deathmatch && options.deathmatch_force_respawn_time != 0.0 {
        let now = game.now();
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .respawn_time = now + options.deathmatch_force_respawn_time;
    }
    if game.options.mode == Q2Mode::Coop && q2_uses_instanced_items(&options) {
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .coop_respawn = Some(Q2PlayerCarry {
            health: max_health,
            maximum_health: max_health,
            ..carry
        });
    }
    if game.options.mode == Q2Mode::Coop && (options.coop_squad_respawn || options.coop_lives) {
        if options.coop_lives {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            if extra.lives != 0 {
                extra.lives -= 1;
            }
        }
        let deadly = game.rerelease.deadly_kill_box;
        let all_dead = game.host.players().into_iter().all(|other| {
            let health = game.host.combat().read(&other).map_or(0.0, |combat| combat.health);
            let lives = game.rerelease.states.get(&other).map_or(0, |extra| extra.lives);
            health <= 0.0 && (deadly || !options.coop_lives || lives <= 0)
        });
        if all_dead {
            let now = game.now();
            game.rerelease.coop_restart_time = now + 5.0;
            for other in game.host.players() {
                game.host_emit(Q2PresentationEvent::CenterPrint {
                    actor: other,
                    text: "$g_coop_lose".to_string(),
                    instant: false,
                    duration_seconds: None,
                });
            }
        } else {
            let now = game.now();
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .respawn_time = now + 3.0;
        }
    }
    true
}

/// Run after client think (`afterClientThink`).
pub fn rerelease_after_client_think(actor: ActorId, game: &mut Q2GameServices) {
    let held = match game.players.intermission.clone() {
        Q2Intermission::Intermission { map, .. } => {
            map.is_empty()
                || q2_is_n64(game) && game.options.mode != Q2Mode::Deathmatch && !game.rerelease.intermission_camera_set
        }
        Q2Intermission::Playing => false,
    };
    if held {
        let buttons = (player_hooks(game).movement)(actor.clone()).buttons;
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .buttons = buttons;
        return;
    }
    base_module(game).after_client_think(actor.clone(), game);
    if game.players.intermission != Q2Intermission::Playing {
        return;
    }
    let mut extra = game
        .rerelease
        .states
        .remove(&actor)
        .expect("Q2 rerelease player is not admitted");
    let options = game.rerelease.options.clone();
    let frame_seconds = game.host.frame_seconds();
    let mut context = Q2PlayerContext {
        actor: actor.clone(),
        game,
    };
    q2_rerelease_falling_damage(&mut context, &mut extra, &options, frame_seconds);
    context.game.rerelease.states.insert(actor, extra);
}

/// Run begin frame (`beginFrame`).
pub fn rerelease_begin_frame(actor: ActorId, game: &mut Q2GameServices) {
    if game.players.intermission != Q2Intermission::Playing {
        return;
    }
    let now = game.now();
    if game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .awaiting_respawn
    {
        if (now * 1000.0).round() as i64 % 500 == 0 {
            rerelease_put_in_server(actor, game, true, None);
        }
        return;
    }
    base_module(game).begin_frame(actor, game);
}

/// Run the dead frame (`deadFrame`).
fn rerelease_dead_frame(actor: ActorId, game: &mut Q2GameServices) {
    let now = game.now();
    let respawn_time = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .respawn_time;
    if now <= respawn_time || game.rerelease.coop_restart_time != 0.0 {
        return;
    }
    let options = game.rerelease.options.clone();
    if game.options.mode == Q2Mode::Coop && (options.coop_squad_respawn || options.coop_lives) {
        return coop_respawn(actor, game);
    }
    let mask = if game.options.mode == Q2Mode::Deathmatch { 1 } else { -1 };
    let latched = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .latched_buttons;
    if latched & mask != 0 || game.options.mode == Q2Mode::Deathmatch && options.deathmatch_force_respawn {
        rerelease_respawn(actor.clone(), game);
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .latched_buttons = 0;
    }
}

/// Begin an intermission (`beginIntermission`).
fn rerelease_begin_intermission(game: &mut Q2GameServices, map: String, landmark: Option<Q2LandmarkCarry>) {
    begin_rerelease_intermission(game, map, landmark, 0);
}

/// Begin a rerelease intermission (`beginRereleaseIntermission`).
pub fn begin_rerelease_intermission(
    game: &mut Q2GameServices,
    map: String,
    landmark: Option<Q2LandmarkCarry>,
    flags: i32,
) {
    if game.players.intermission != Q2Intermission::Playing {
        return;
    }
    game.rerelease.intermission_flags = flags;
    game.rerelease.intermission_fade_until = None;
    let now = game.now();
    game.players.intermission = Q2Intermission::Intermission {
        map: map.clone(),
        landmark,
        started: now,
        exit: false,
    };
    let mut actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
    sort_rerelease_actors(&mut actors);
    for actor in actors {
        let state = game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .clone();
        if game.entity(&actor).is_none()
            || !state.connected
            || game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) > 0.0
        {
            continue;
        }
        if q2_uses_instanced_items(&game.rerelease.options) {
            if let Some(carry) = state.coop_respawn.clone() {
                let max_health = game.require_entity(&actor).max_health;
                game.players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted")
                    .coop_respawn = Some(Q2PlayerCarry {
                    health: max_health,
                    maximum_health: max_health,
                    ..carry
                });
            }
        }
        rerelease_respawn(actor, game);
    }
    if let Some(extension) = game.rerelease.extension {
        (extension.before_level_change)(game);
    }
    let end_unit = map.contains('*');
    if end_unit {
        if game.options.mode == Q2Mode::Coop {
            let keys: Vec<ItemId> = player_items(game)
                .list(game)
                .into_iter()
                .filter(|item| item.kind == Q2ItemKind::Key)
                .map(|item| item.id.clone())
                .collect();
            let mut actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
            sort_rerelease_actors(&mut actors);
            for actor in actors {
                let state = game
                    .players
                    .states
                    .get(&actor)
                    .expect("Q2 player has not been admitted")
                    .clone();
                if game.entity(&actor).is_none() || !state.connected {
                    continue;
                }
                let owned = game.owned_of(actor.clone());
                for item in &keys {
                    let entries = game.host.inventory().entries(&actor);
                    if let Some(entry) = entries.into_iter().find(|entry| &entry.item == item) {
                        game.host.inventory().configure(
                            &owned,
                            &InventoryEntry {
                                item: entry.item.clone(),
                                count: 0.0,
                                capacity: entry.capacity,
                                count_policy: entry.count_policy,
                            },
                        );
                    }
                }
            }
        }
        let achievement = q2_entities_named(game, "worldspawn")
            .first()
            .and_then(|world| game.require_entity(world).spawn.values.get("achievement").cloned());
        if let Some(achievement) = achievement {
            if !achievement.is_empty() {
                (rerelease_hooks(game).emit)(game, Q2RereleaseEvent::Achievement { id: achievement });
            }
        }
        if flags & 16 == 0 {
            if let Some(extension) = game.rerelease.extension {
                (extension.end_of_unit)(game);
            }
        } else if flags & 64 != 0 && game.options.mode != Q2Mode::Deathmatch {
            if let Q2Intermission::Intermission { ref mut exit, .. } = game.players.intermission {
                *exit = true;
            }
            return;
        }
    } else if game.options.mode != Q2Mode::Deathmatch {
        if let Q2Intermission::Intermission { ref mut exit, .. } = game.players.intermission {
            *exit = true;
        }
        return;
    }
    if !game.rerelease.intermission_camera_set || game.rerelease.intermission_camera.is_none() {
        let authored = q2_entities_named(game, "info_player_intermission");
        let selection = if authored.is_empty() {
            0
        } else if let Some(source) = game.host.rerelease_random() {
            source.integer_max(4) as usize
        } else {
            (game.random() * 4.0).floor() as usize
        };
        let spot = if authored.is_empty() {
            q2_entities_named(game, "info_player_start")
                .first()
                .cloned()
                .or_else(|| q2_entities_named(game, "info_player_deathmatch").first().cloned())
        } else {
            authored.get(selection % authored.len()).cloned()
        };
        let Some(spot) = spot else {
            panic!("Q2 rerelease intermission has no camera or player spawn");
        };
        let body = game.body_of(spot);
        game.rerelease.intermission_camera = Some(super::checkpoint::Q2RereleaseIntermissionCamera {
            origin: body.origin,
            angles: body.angles,
        });
    }
    let camera = game
        .rerelease
        .intermission_camera
        .expect("Q2 rerelease intermission camera is missing");
    rerelease_move_to_camera(game, camera.origin, camera.angles, true);
}

/// Check rules (`checkRules`).
pub fn rerelease_check_rules(game: &mut Q2GameServices) {
    match game.players.intermission.clone() {
        Q2Intermission::Intermission { map, exit, .. } => {
            if map.is_empty() {
                return;
            }
            if exit && game.rerelease.intermission_flags & 32 != 0 {
                if game.rerelease.intermission_fade_until.is_none() {
                    game.rerelease.intermission_fade_until = Some(game.now() + 1.3);
                }
                if game.now() < game.rerelease.intermission_fade_until.expect("fade just set") {
                    return;
                }
                game.rerelease.intermission_flags &= !32;
                game.rerelease.intermission_fade_until = None;
            }
        }
        Q2Intermission::Playing => {}
    }
    if game.rerelease.coop_restart_time != 0.0 && game.now() >= game.rerelease.coop_restart_time {
        game.rerelease.coop_restart_time = 0.0;
        let map = game.options.map_name.clone();
        return (rerelease_hooks(game).emit)(game, Q2RereleaseEvent::RestartLevel { map });
    }
    base_module(game).check_rules(game);
}

/// Run before exiting the level (`beforeExitLevel`).
fn rerelease_before_exit_level(game: &mut Q2GameServices, map: String) {
    if game.rerelease.intermission_flags & 8 != 0 {
        game.rerelease.intermission_flags &= !8;
        let mut actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
        sort_rerelease_actors(&mut actors);
        for actor in actors {
            if game.entity(&actor).is_none() {
                continue;
            }
            let owned = game.owned_of(actor.clone());
            for entry in game.host.inventory().entries(&actor) {
                game.host.inventory().configure(
                    &owned,
                    &InventoryEntry {
                        item: entry.item.clone(),
                        count: 0.0,
                        capacity: entry.capacity,
                        count_policy: entry.count_policy,
                    },
                );
            }
            game.host.combat().set_health(&owned, 0.0);
            game.host.combat().set_armor(
                &owned,
                &crate::contract::ArmorState {
                    regular: RegularArmorState::None,
                    powered: PoweredProtectionState::None,
                },
            );
            player_items(game).clear_powerups(game, &actor);
            if let Some(clear) = rerelease_hooks(game).clear_expansion_powerups {
                clear(actor.clone(), game);
            }
            {
                let record = game.require_entity_mut(&actor);
                record.flags &= !(16 | 32 | 4096 | 0x400000 | 0x40000000);
                record.power_cubes = 0;
            }
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.god = false;
            entry.notarget = false;
            entry.coop_respawn = None;
            entry.selected_item = None;
        }
    }
    if map.contains('*') {
        if let Some(extension) = game.rerelease.extension {
            (extension.leave_unit)(game);
        }
    }
}

/// Run the intermission fade frame (`fadeFrame`).
pub fn rerelease_fade_frame(game: &mut Q2GameServices) {
    let Some(until) = game.rerelease.intermission_fade_until else {
        return;
    };
    if game.now() >= until {
        return rerelease_check_rules(game);
    }
    let alpha = (1.0 - (until - game.now() - 0.3)).clamp(0.0, 1.0);
    let mut actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
    sort_rerelease_actors(&mut actors);
    for actor in actors {
        if !game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .connected
        {
            continue;
        }
        (rerelease_hooks(game).emit)(
            game,
            Q2RereleaseEvent::ScreenBlend {
                actor,
                blend: qa_core::math::Vec4 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: alpha as f32,
                },
            },
        );
    }
}

/// Move players to an intermission camera (`moveToCamera`).
pub fn rerelease_move_to_camera(game: &mut Q2GameServices, origin: Vec3, angles: Vec3, entering: bool) {
    game.rerelease.intermission_camera = Some(super::checkpoint::Q2RereleaseIntermissionCamera { origin, angles });
    let mut actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
    sort_rerelease_actors(&mut actors);
    for actor in actors {
        if game.entity(&actor).is_none() {
            continue;
        }
        if entering && game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) <= 0.0 {
            rerelease_respawn(actor.clone(), game);
        }
        if entering {
            game.host_emit(Q2PresentationEvent::EntityEvent {
                actor: actor.clone(),
                event: 7,
            });
        }
        {
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.show_help = false;
            entry.show_scores = game.options.mode == Q2Mode::Deathmatch;
            entry.damage_alpha = 0.0;
            entry.bonus_alpha = 0.0;
            entry.loop_sound = String::new();
        }
        player_items(game).clear_powerups(game, &actor);
        if let Some(clear) = rerelease_hooks(game).clear_expansion_powerups {
            clear(actor.clone(), game);
        }
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .invisibility_until = 0.0;
        let god = game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .god;
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: None,
                mass: None,
                invulnerable: Some(god),
                team: None,
                no_knockback: None,
            },
        );
        if let Some(weapon) = game.weapons.states.get_mut(&actor) {
            weapon.grenade_blew_up = false;
            weapon.grenade_time = 0.0;
            weapon.view_model = None;
        }
        {
            let record = game.require_entity_mut(&actor);
            record.view_height = 0;
            record.model = String::new();
            record.model2 = String::new();
            record.model3 = String::new();
            record.effects = 0;
            record.sound = String::new();
            record.visible = false;
        }
        game.set_solid(actor.clone(), Q2Solid::None);
        game.set_motion_kind(actor.clone(), Q2MotionKind::Stationary);
        let mut body = game.body_of(actor.clone());
        body.origin = origin;
        game.write_body(actor.clone(), &body, false);
        game.show(actor.clone());
        (player_hooks(game).set_movement)(actor.clone(), Q2PlayerMovementChange::Freeze { origin, angles });
        if game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .show_scores
        {
            base_module(game).scoreboard(actor.clone(), game, true);
        }
    }
}

/// Finish a camera (`finishCamera`).
pub fn rerelease_finish_camera(game: &mut Q2GameServices) {
    let now = game.now();
    match game.players.intermission.clone() {
        Q2Intermission::Playing => {
            game.players.intermission = Q2Intermission::Intermission {
                map: String::new(),
                landmark: None,
                started: now,
                exit: false,
            };
        }
        Q2Intermission::Intermission { map, landmark, .. } => {
            let exit = !map.is_empty() && !map.contains('*');
            game.players.intermission = Q2Intermission::Intermission {
                map,
                landmark,
                started: now,
                exit,
            };
        }
    }
}

/// Run end of unit from a Q64 camera (`endOfUnit`).
pub fn rerelease_end_of_unit(game: &mut Q2GameServices) {
    super::entities::Q2RereleaseEntities {
        players: Q2RereleasePlayers {
            players: base_module(game),
        },
        hooks: rerelease_hooks(game),
    }
    .end_of_unit(game);
}

/// Emit the flashlight state (`emitFlashlight`).
pub fn rerelease_emit_flashlight(actor: ActorId, game: &mut Q2GameServices) {
    let state = game
        .players
        .states
        .get(&actor)
        .expect("Flashlight player is not admitted")
        .clone();
    let enabled = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .flashlight
        && game.players.intermission == Q2Intermission::Playing
        && game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) > 0.0;
    (rerelease_hooks(game).emit)(
        game,
        Q2RereleaseEvent::Flashlight {
            actor,
            enabled,
            hand: state.hand,
        },
    );
}

/// Run the end frame (`endFrame`).
fn rerelease_end_frame(actor: ActorId, game: &mut Q2GameServices) {
    if let Some(extension) = game.rerelease.extension {
        (extension.begin_player_frame)(actor.clone(), game);
    }
    base_module(game).end_frame(actor.clone(), game);
    if let Some(extension) = game.rerelease.extension {
        (extension.end_player_frame)(actor.clone(), game);
    }
    let now = game.now();
    let health = game.host.combat().read(&actor).map_or(0.0, |combat| combat.health);
    let extra = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .clone();
    let alpha =
        if game.players.intermission == Q2Intermission::Playing && health > 0.0 && extra.invisibility_until > now {
            ((extra.invisibility_fade_until - now) / 2.0).clamp(0.1, 1.0)
        } else {
            1.0
        };
    (rerelease_hooks(game).emit)(
        game,
        Q2RereleaseEvent::Alpha {
            actor: actor.clone(),
            alpha,
        },
    );
    rerelease_emit_flashlight(actor.clone(), game);
    let clip = game.require_entity(&actor).clip_mask;
    let can_take = game
        .host
        .combat()
        .read(&actor)
        .is_some_and(|combat| combat.can_take_damage);
    if game.players.intermission == Q2Intermission::Playing
        && game.options.mode == Q2Mode::Coop
        && game.rerelease.options.coop_player_collision
        && clip & 0x40000000 == 0
        && can_take
    {
        let body = game.body_of(actor.clone());
        let trace = game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end: body.origin,
            bounds: Some(body.bounds),
            ignore: Some(actor.clone()),
            mask: 0x40000000,
            exclude: Vec::new(),
        });
        if !trace.start_solid && !trace.all_solid {
            game.require_entity_mut(&actor).clip_mask |= 0x40000000;
            if let Some(collision) = rerelease_hooks(game).player_collision {
                collision(actor, game, true);
            }
        }
    }
}

/// Run a client command (`clientCommand`).
pub fn rerelease_client_command(actor: ActorId, game: &mut Q2GameServices, command: &str, args: &[String]) -> bool {
    if command.to_lowercase() == "help" && game.options.mode != Q2Mode::Deathmatch && game.rerelease.extension.is_some()
    {
        let extension = game.rerelease.extension.expect("extension checked");
        (extension.help)(actor, game);
        return true;
    }
    base_module(game).client_command(actor, game, command, args)
}

/// Put a player in the server (`putInServer`).
fn rerelease_put_in_server(
    actor: ActorId,
    game: &mut Q2GameServices,
    restore_loadout: bool,
    landmark: Option<Q2LandmarkCarry>,
) {
    if let Some(select) = player_hooks(game).select_spawn {
        if let Some(placement) = select(actor.clone(), game) {
            game.rerelease.squad_spawns.insert(
                actor.clone(),
                Q2RereleaseSquadSpawn {
                    origin: placement.origin,
                    angles: placement.angles,
                },
            );
        }
    }
    if let Some(landmark) = &landmark {
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .pending_landmark = Some(Q2PendingLandmark {
            name: landmark.name.clone(),
            relative_origin: landmark.relative_origin,
            relative_velocity: landmark.relative_velocity,
            relative_view_angles: landmark.relative_view_angles,
        });
    }
    let carry = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .pending_landmark
        .clone()
        .map(|pending| Q2LandmarkCarry {
            player: actor.clone(),
            name: pending.name,
            relative_origin: pending.relative_origin,
            relative_velocity: pending.relative_velocity,
            relative_view_angles: pending.relative_view_angles,
        });
    if !game.rerelease.squad_spawns.contains_key(&actor) {
        let bounds = (player_hooks(game).movement)(actor.clone()).standing_bounds;
        let options = game.rerelease.options.clone();
        let spawn_point = game.players.rules.spawn_point.clone();
        let force = {
            let extra = game
                .rerelease
                .states
                .get(&actor)
                .expect("Q2 rerelease player is not admitted");
            extra.awaiting_respawn && game.now() > extra.respawn_timeout
        };
        let spot = select_q2_rerelease_spawn(game, actor.clone(), bounds, &options, &spawn_point, force);
        if spot.is_none() && game.options.mode != Q2Mode::Singleplayer {
            let now = game.now();
            {
                let extra = game
                    .rerelease
                    .states
                    .get_mut(&actor)
                    .expect("Q2 rerelease player is not admitted");
                if !extra.awaiting_respawn {
                    extra.respawn_timeout = now + 3.0;
                }
                extra.awaiting_respawn = true;
                extra.spawned = false;
            }
            let points = q2_entities_named(game, "info_player_intermission");
            let camera = if points.is_empty() {
                q2_entities_named(game, "info_player_start")
                    .first()
                    .cloned()
                    .or_else(|| q2_entities_named(game, "info_player_deathmatch").first().cloned())
            } else {
                let selection = (game.random() * 4.0).floor() as usize % points.len();
                points.get(selection).cloned()
            };
            let (origin, angles) = camera
                .map(|camera| {
                    let body = game.body_of(camera);
                    (body.origin, body.angles)
                })
                .unwrap_or((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)));
            {
                let entry = game
                    .players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted");
                entry.dead = false;
                entry.noclip = true;
            }
            {
                let record = game.require_entity_mut(&actor);
                record.visible = false;
                record.server_flags |= 1;
            }
            let mut body = game.body_of(actor.clone());
            body.origin = origin;
            body.velocity = vec3(0.0, 0.0, 0.0);
            game.write_body(actor.clone(), &body, false);
            game.set_solid(actor.clone(), Q2Solid::None);
            (player_hooks(game).set_movement)(actor.clone(), Q2PlayerMovementChange::Freeze { origin, angles });
            game.host_emit(Q2PresentationEvent::Visibility {
                actor: actor.clone(),
                visible: false,
            });
            game.link_actor(actor);
            return;
        }
        let placed = match (&spot, &carry) {
            (Some(spot), Some(carry)) => place_q2_landmark(actor.clone(), game, carry, spot.clone(), bounds),
            _ => None,
        };
        let (origin, angles) = match (&spot, &placed) {
            (Some(spot), _) if placed.is_none() => {
                let body = game.body_of(spot.clone());
                (body.origin, body.angles)
            }
            _ => placed
                .as_ref()
                .map(|placed| (placed.origin, placed.angles))
                .unwrap_or((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
        };
        let lift = if game.options.mode == Q2Mode::Deathmatch {
            10.0
        } else {
            1.0
        };
        game.rerelease.selected_spawns.insert(
            actor.clone(),
            Q2RereleaseSelectedSpawn {
                origin: add3(origin, vec3(0.0, 0.0, lift)),
                angles: vec3(angles.x / 3.0, angles.y, angles.z),
                velocity: placed
                    .as_ref()
                    .map(|placed| placed.velocity)
                    .unwrap_or(vec3(0.0, 0.0, 0.0)),
                from_landmark: placed.is_some(),
            },
        );
    }
    let was_waiting = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .awaiting_respawn;
    {
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.awaiting_respawn = false;
        extra.respawn_timeout = 0.0;
        extra.pending_landmark = None;
    }
    game.require_entity_mut(&actor).clip_mask =
        if game.options.mode == Q2Mode::Coop && !game.rerelease.options.coop_player_collision {
            0x2010003
        } else {
            0x42010003
        };
    base_module(game).put_in_server(actor.clone(), game, restore_loadout, carry.as_ref());
    let colliding = game.require_entity(&actor).clip_mask & 0x40000000 != 0;
    if let Some(collision) = rerelease_hooks(game).player_collision {
        collision(actor.clone(), game, colliding);
    }
    {
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted");
        extra.slime_debounce = 0.0;
        extra.animation_time = 0.0;
        extra.slow_view_angles = vec3(0.0, 0.0, 0.0);
        extra.coop_respawn_state = Q2CoopRespawnState::None;
        extra.invisibility_until = 0.0;
        extra.invisibility_fade_until = 0.0;
    }
    if let Some(extension) = game.rerelease.extension {
        (extension.spawned)(actor.clone(), game);
    }
    let use_q2_inventory = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .use_q2_inventory;
    if game.options.map_name.to_lowercase() == "rboss" && game.options.mode != Q2Mode::Deathmatch && use_q2_inventory {
        let owned = game.owned_of(actor.clone());
        game.host.inventory().configure(
            &owned,
            &InventoryEntry {
                item: "q2:key_nuke".to_string(),
                count: 1.0,
                capacity: 1.0,
                count_policy: None,
            },
        );
    }
    if was_waiting {
        post_respawn(actor, game);
    }
}

/// Finish a deferred respawn (`postRespawn`).
fn post_respawn(actor: ActorId, game: &mut Q2GameServices) {
    if game.require_entity(&actor).server_flags & 1 != 0 {
        return;
    }
    let body = game.body_of(actor.clone());
    let movement = (player_hooks(game).movement)(actor.clone());
    let spectator = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .spectator;
    (player_hooks(game).set_movement)(
        actor.clone(),
        Q2PlayerMovementChange::Spawn(Q2PlayerSpawnChange {
            origin: body.origin,
            velocity: body.velocity,
            angles: movement.view_angles,
            command_angles: movement.command_angles,
            hold_milliseconds: 112,
            spectator,
        }),
    );
    let now = game.now();
    let entry = game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted");
    entry.event = "q2:player-teleport".to_string();
    entry.respawn_time = now;
}

/// Respawn a player (`respawn`).
fn rerelease_respawn(actor: ActorId, game: &mut Q2GameServices) {
    if game.options.mode == Q2Mode::Singleplayer {
        (player_hooks(game).emit)(Q2PlayerEvent::LoadMenu { actor });
        return;
    }
    if !game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .spectator
    {
        base_module(game).copy_to_body_queue(actor.clone(), game);
    }
    game.require_entity_mut(&actor).server_flags &= !1;
    rerelease_put_in_server(actor.clone(), game, true, None);
    post_respawn(actor, game);
}

/// Run the kill box (`killBox`).
fn rerelease_kill_box(actor: ActorId, game: &mut Q2GameServices) -> bool {
    kill_q2_rerelease_box(actor, game, true, true)
}

/// Run a coop respawn (`coopRespawn`).
fn coop_respawn(actor: ActorId, game: &mut Q2GameServices) {
    let options = game.rerelease.options.clone();
    let mut allowed = true;
    if options.coop_lives
        && game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease player is not admitted")
            .lives
            == 0
    {
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease player is not admitted")
            .coop_respawn_state = Q2CoopRespawnState::NoLives;
        allowed = false;
    } else if options.coop_squad_respawn
        && game
            .host
            .players()
            .into_iter()
            .any(|other| game.host.combat().read(&other).map_or(0.0, |combat| combat.health) > 0.0)
    {
        match squad_target(game) {
            None => allowed = false,
            Some(target) => {
                game.rerelease.squad_spawns.insert(actor.clone(), target);
            }
        }
    }
    if allowed {
        {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            extra.coop_respawn_state = Q2CoopRespawnState::None;
        }
        {
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.spectator = false;
            entry.requested_spectator = false;
        }
        rerelease_respawn(actor.clone(), game);
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .latched_buttons = 0;
    } else {
        {
            let extra = game
                .rerelease
                .states
                .get_mut(&actor)
                .expect("Q2 rerelease player is not admitted");
            if extra.coop_respawn_state == Q2CoopRespawnState::None {
                extra.coop_respawn_state = Q2CoopRespawnState::Waiting;
            }
        }
        if !game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .spectator
        {
            base_module(game).copy_to_body_queue(actor.clone(), game);
            {
                let entry = game
                    .players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted");
                entry.spectator = true;
                entry.noclip = true;
            }
            game.set_solid(actor.clone(), Q2Solid::None);
            let owned = game.owned_of(actor.clone());
            game.host.combat().set_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: Some(false),
                    mass: None,
                    invulnerable: None,
                    team: None,
                    no_knockback: None,
                },
            );
            game.require_entity_mut(&actor).visible = false;
            game.host_emit(Q2PresentationEvent::Visibility {
                actor: actor.clone(),
                visible: false,
            });
            {
                let entry = game
                    .players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted");
                entry.damage_alpha = 0.0;
                entry.bonus_alpha = 0.0;
            }
            (player_hooks(game).set_movement)(actor.clone(), Q2PlayerMovementChange::Noclip { enabled: true });
            game.link_actor(actor.clone());
            base_module(game).chase(actor.clone(), game, 1, true);
        }
    }
    let extra = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease player is not admitted")
        .clone();
    (rerelease_hooks(game).emit)(
        game,
        Q2RereleaseEvent::CoopRespawn {
            actor,
            state: extra.coop_respawn_state,
            lives: extra.lives,
        },
    );
}

/// Select a squad respawn target (`squadTarget`).
fn squad_target(game: &mut Q2GameServices) -> Option<Q2RereleaseSquadSpawn> {
    let searching = (rerelease_hooks(game).monsters_searching)(game, None);
    let now = game.now();
    for other in game.host.players() {
        if game.entity(&other).is_none()
            || !game.players.states.contains_key(&other)
            || !game.rerelease.states.contains_key(&other)
        {
            continue;
        }
        if game.players.states.get(&other).expect("player checked").dead {
            continue;
        }
        let extra_firing = game
            .rerelease
            .states
            .get(&other)
            .expect("extra checked")
            .last_firing_until;
        let firing_until = game
            .weapons
            .states
            .get(&other)
            .map(|weapon| weapon.last_firing_time)
            .unwrap_or(extra_firing);
        let last_damage = game
            .rerelease
            .states
            .get(&other)
            .expect("extra checked")
            .last_damage_until;
        if last_damage >= now
            || (rerelease_hooks(game).monsters_searching)(game, Some(other.clone()))
            || searching && firing_until >= now
        {
            game.rerelease
                .states
                .get_mut(&other)
                .expect("extra checked")
                .coop_respawn_state = Q2CoopRespawnState::InCombat;
            continue;
        }
        if !(rerelease_hooks(game).grounded_on_world)(other.clone(), game)
            || (player_hooks(game).movement)(other.clone()).water_level >= 3
        {
            game.rerelease
                .states
                .get_mut(&other)
                .expect("extra checked")
                .coop_respawn_state = Q2CoopRespawnState::BadArea;
            continue;
        }
        let origin = find_respawn_spot(other.clone(), game);
        if origin.is_none() {
            game.rerelease
                .states
                .get_mut(&other)
                .expect("extra checked")
                .coop_respawn_state = Q2CoopRespawnState::Blocked;
            continue;
        }
        let angles = game.body_of(other).angles;
        return Some(Q2RereleaseSquadSpawn {
            origin: origin.expect("origin checked"),
            angles: vec3(angles.x, angles.y, 0.0),
        });
    }
    None
}

impl Q2RereleasePlayers {
    /// Find a coop respawn spot (`findRespawnSpot`).
    pub fn find_respawn_spot(&self, player: ActorId, game: &mut Q2GameServices) -> Option<Vec3> {
        find_respawn_spot(player, game)
    }
}

/// Find a coop respawn spot (`findRespawnSpot`).
pub fn find_respawn_spot(player: ActorId, game: &mut Q2GameServices) -> Option<Vec3> {
    let body = game.body_of(player.clone());
    let bounds = (player_hooks(game).movement)(player.clone()).standing_bounds;
    let trace = |game: &mut Q2GameServices, start: Vec3, end: Vec3, point: bool| {
        game.host.trace(&Q2TraceRequest {
            start,
            end,
            bounds: if point { None } else { Some(bounds) },
            ignore: Some(player.clone()),
            mask: 0x201001b,
            exclude: Vec::new(),
        })
    };
    let first = trace(game, body.origin, body.origin, false);
    if first.start_solid || first.all_solid {
        return None;
    }
    for yaw in [0.0, 90.0, 45.0, -45.0, -90.0] {
        let up = trace(game, body.origin, add3(body.origin, vec3(0.0, 0.0, 128.0)), false);
        if up.start_solid || up.all_solid || game.host.point_contents(up.end) & 24 != 0 {
            continue;
        }
        let back = trace(
            game,
            up.end,
            add3(
                up.end,
                scale3(movedir(vec3(0.0, body.angles.y + 180.0 + yaw, 0.0)), 128.0),
            ),
            false,
        );
        if back.start_solid || back.all_solid || game.host.point_contents(back.end) & 24 != 0 {
            continue;
        }
        let floor = trace(game, back.end, add3(back.end, vec3(0.0, 0.0, -512.0)), false);
        if floor.start_solid
            || floor.all_solid
            || floor.fraction == 1.0
            || !matches!(floor.hit, TraceHit::World { .. })
            || game.host.point_contents(floor.end) & 24 != 0
        {
            continue;
        }
        let contact_z = match floor.contact {
            TraceContact::Plane { plane } => Some(plane.normal.z),
            TraceContact::None => None,
        };
        if game.host.point_contents(add3(floor.end, vec3(0.0, 0.0, 22.0))) & 56 != 0
            || contact_z.is_none_or(|z| z < 0.7)
        {
            continue;
        }
        let height = (f64::from(body.origin.z) - f64::from(floor.end.z)).abs();
        if height > 72.0 {
            continue;
        }
        if height > 18.0
            && (trace(game, body.origin, floor.end, true).fraction != 1.0
                || trace(
                    game,
                    add3(body.origin, vec3(0.0, 0.0, 22.0)),
                    add3(floor.end, vec3(0.0, 0.0, 22.0)),
                    true,
                )
                .fraction
                    != 1.0)
        {
            continue;
        }
        return Some(floor.end);
    }
    None
}

/// Place a spawn (`spawnPlacement`).
fn rerelease_spawn_placement(
    actor: ActorId,
    game: &mut Q2GameServices,
    landmark: Option<Q2LandmarkCarry>,
) -> Q2SpawnSolution {
    if let Some(squad) = game.rerelease.squad_spawns.remove(&actor) {
        return Q2SpawnSolution {
            origin: squad.origin,
            angles: squad.angles,
            velocity: vec3(0.0, 0.0, 0.0),
            from_landmark: false,
        };
    }
    if let Some(selected) = game.rerelease.selected_spawns.remove(&actor) {
        return Q2SpawnSolution {
            origin: selected.origin,
            angles: selected.angles,
            velocity: selected.velocity,
            from_landmark: selected.from_landmark,
        };
    }
    base_module(game).spawn_placement(actor, game, landmark.as_ref())
}

/// Run world effects (`worldEffects`).
fn rerelease_world_effects(actor: ActorId, game: &mut Q2GameServices) {
    let mut extra = game
        .rerelease
        .states
        .remove(&actor)
        .expect("Q2 rerelease player is not admitted");
    let mut context = Q2PlayerContext {
        actor: actor.clone(),
        game,
    };
    q2_rerelease_world_effects(&mut context, &mut extra);
    context.game.rerelease.states.insert(actor, extra);
}

/// Run falling damage (`fallingDamage`).
fn rerelease_falling_damage(_actor: ActorId, _game: &mut Q2GameServices) {}

/// Run damage feedback (`damageFeedback`).
fn rerelease_damage_feedback(actor: ActorId, game: &mut Q2GameServices, pain_index: i32) -> (i32, i32) {
    super::view::q2_rerelease_damage_feedback(actor, game, pain_index)
}

/// Build the player view (`buildView`).
fn rerelease_build_view(actor: ActorId, game: &mut Q2GameServices, flashes: i32, intermission: bool) -> Q2PlayerView {
    let fade_until = game.rerelease.intermission_fade_until;
    let now = game.now();
    let mut view = super::view::q2_rerelease_build_view(actor, game, flashes, intermission);
    if let Some(until) = fade_until {
        view.blend = Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: (1.0 - (until - now - 0.3)).clamp(0.0, 1.0) as f32,
        };
    }
    view
}

/// Update the view bob (`updateBob`).
fn rerelease_update_bob(actor: ActorId, game: &mut Q2GameServices) {
    let body = game.body_of(actor.clone());
    let speed = length3(vec3(body.velocity.x, body.velocity.y, 0.0));
    let grounded = (player_hooks(game).movement)(actor.clone()).grounded;
    let frame_seconds = game.host.frame_seconds();
    let entry = game
        .players
        .states
        .get_mut(&actor)
        .expect("Q2 player has not been admitted");
    if speed < 5.0 {
        entry.bob_move = 0.0;
        entry.bob_time = 0.0;
    } else if grounded {
        entry.bob_move = frame_seconds
            / if speed > 210.0 {
                0.4
            } else if speed > 100.0 {
                0.8
            } else {
                1.6
            };
    }
    entry.bob_time += entry.bob_move;
}

/// Run client animation (`clientAnimation`).
fn rerelease_client_animation(actor: ActorId, game: &mut Q2GameServices) {
    super::view::q2_rerelease_client_animation(actor, game);
}
