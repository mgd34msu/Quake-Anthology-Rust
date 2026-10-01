//! Q2 LMCTF flags (`src/content/q2/multiplayer/lmctf/flags.ts`).
//!
//! LM_CTF g_ctffunc.c and p_client.c flag bonuses. GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{Bounds, Vec3, add3, length3, sub3, vec3};

use crate::contract::InventoryEntry;
use crate::q2::base::player::spawns::q2_entities_named;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2EffectEvent, Q2GameServices, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2Think, Q2Touch, Q2TraceRequest,
};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData};
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::TouchContact;

use super::super::ctf::types::{CTF_FLAGS, item_id, other_ctf_team};
use super::types::{
    LmctfHooks, LmctfPlayingTeam, lmctf_active, lmctf_flags_touchable, lmctf_name, lmctf_player, lmctf_print, lmctf_score, lmctf_stat,
    lmctf_toss,
};

/// LMCTF flag bounds.
const FLAG_BOUNDS: Bounds = Bounds { min: Vec3 { x: -15.0, y: -15.0, z: -15.0 }, max: Vec3 { x: 15.0, y: 15.0, z: 33.0 } };

/// LMCTF flag state (`state` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LmctfFlagState {
    /// At base.
    Base,
    /// Taken.
    Taken,
    /// Dropped.
    Dropped,
}

/// LMCTF flags checkpoint (`LmctfFlagsCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfFlagsCheckpoint {
    /// Flags by team.
    pub flags: Vec<LmctfFlagSlot>,
    /// Last taken sound time.
    pub last_taken_sound: f64,
}

/// LMCTF flag slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LmctfFlagSlot {
    /// Team.
    pub team: LmctfPlayingTeam,
    /// Actor.
    pub actor: SavedActorId,
}

/// LMCTF flags (`LmctfFlags`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfFlags {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

/// Flag callbacks (`LmctfFlags::callbacks`).
pub fn lmctf_flag_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("lmctf:ctf_flagwave", lmctf_flag_wave as Q2Think);
    callbacks.think.insert("lmctf:Drop_Flag_Think", lmctf_drop_flag_think as Q2Think);
    callbacks.touch.insert("lmctf:ctf_flagtouch", lmctf_flag_touch as Q2Touch);
    callbacks.touch.insert("lmctf:Flag_DropTouch", lmctf_flag_drop_touch as Q2Touch);
    callbacks
}

/// Flag item pickup (`register` pickup).
fn lmctf_flag_item_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let flags = LmctfFlags { hooks: super::lmctf_hooks(game) };
    flags.pickup(entity, game, player.id().clone());
    false
}

/// Flag item use (`register` use).
fn lmctf_flag_item_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    let flags = LmctfFlags { hooks: super::lmctf_hooks(game) };
    if game.entity(player.id()).is_none() || flags.carried(player.id(), game).is_none() {
        return false;
    }
    flags.drop(player.id().clone(), game);
    true
}

/// Flag wave think (`wave`).
fn lmctf_flag_wave(flag: ActorId, game: &mut Q2GameServices) {
    let flags = LmctfFlags { hooks: super::lmctf_hooks(game) };
    if game.require_entity(&flag).solid != Q2Solid::None {
        let frame = game.require_entity(&flag).frame;
        game.require_entity_mut(&flag).frame = 173 + (frame - 172) % 16;
        game.show(flag.clone());
    }
    game.schedule(flag.clone(), 0.1, lmctf_flag_wave as Q2Think);
    let record = game.require_entity(&flag);
    let (timestamp, owner) = (record.timestamp, record.owner.clone());
    if timestamp != 0.0 && game.now() > timestamp + 30.0 && (owner.is_none() || owner.is_some_and(|owner| !lmctf_active(game, &owner))) {
        let team = flags.team(&flag, game);
        flags.sound(flag.clone(), game, if team == 1 { "ctf/r_returned.wav" } else { "ctf/b_returned.wav" }, 0.8);
        flags.reset(Some(flag), None, game);
    }
}

/// Flag touch (`touch`).
fn lmctf_flag_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let flags = LmctfFlags { hooks: super::lmctf_hooks(game) };
    flags.pickup(entity, game, contact.other);
}

/// Flag drop touch (`dropTouch`).
fn lmctf_flag_drop_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.require_entity(&entity).owner.as_ref() == Some(&contact.other) {
        return;
    }
    let flags = LmctfFlags { hooks: super::lmctf_hooks(game) };
    flags.pickup(entity, game, contact.other);
}

/// Dropped flag think (`droppedThink`).
fn lmctf_drop_flag_think(entity: ActorId, game: &mut Q2GameServices) {
    {
        let record = game.require_entity_mut(&entity);
        record.owner = None;
        record.touch = Some(lmctf_flag_touch as Q2Touch);
    }
    game.schedule(entity, 0.1, lmctf_flag_wave as Q2Think);
}

impl LmctfFlags {
    /// Register the flag item (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        self.hooks.items.register_item(
            game,
            Q2ItemDefinition {
                classname: "flag".to_string(),
                model: "players/male/flag1.md2".to_string(),
                icon: "a_redflag".to_string(),
                name: "Enemy Flag".to_string(),
                sound: "misc/am_pkup.wav".to_string(),
                rotate: false,
                respawn: 0.0,
                console_give: None,
                kind: Q2ItemKindData::Custom {
                    capacity: 32767.0,
                    quantity: 1.0,
                    coop_stay: false,
                    droppable: false,
                    pickup: lmctf_flag_item_pickup,
                    use_item: Some(lmctf_flag_item_use),
                },
            },
        );
    }

    /// Capture flag slots (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> LmctfFlagsCheckpoint {
        let mut flags: Vec<LmctfFlagSlot> = game
            .lmctf
            .flag_slots
            .iter()
            .map(|(team, actor)| LmctfFlagSlot { team: *team, actor: SavedActorId::from(actor) })
            .collect();
        flags.sort_by(|left, right| left.team.cmp(&right.team));
        LmctfFlagsCheckpoint { flags, last_taken_sound: game.lmctf.flag_taken_sound }
    }

    /// Restore flag slots (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, saved: &LmctfFlagsCheckpoint) {
        game.lmctf.flag_slots.clear();
        for flag in &saved.flags {
            let Some(actor) = game.host.actors().resolve_saved(flag.actor) else {
                panic!("Saved LMCTF flag is absent");
            };
            game.lmctf.flag_slots.insert(flag.team, actor.id().clone());
        }
        game.lmctf.flag_taken_sound = saved.last_taken_sound;
    }

    /// Read a team flag (`flag`).
    pub fn flag(&self, game: &mut Q2GameServices, team: LmctfPlayingTeam) -> Option<ActorId> {
        let actor = game.lmctf.flag_slots.get(&team).cloned()?;
        if game.entity(&actor).is_some() { Some(actor) } else { None }
    }

    /// Read the carried enemy flag (`carried`).
    pub fn carried(&self, actor: &ActorId, game: &mut Q2GameServices) -> Option<ActorId> {
        let team = game.lmctf.states.get(actor).map(|state| state.team)?;
        if team == 0 || game.host.inventory().count(actor, &item_id("q2:flag")) == 0.0 {
            return None;
        }
        self.flag(game, other_ctf_team(team))
    }

    /// Whether the flag is at home (`atHome`).
    pub fn at_home(&self, flag: &ActorId, game: &mut Q2GameServices) -> bool {
        let home = game.require_entity(flag).pos1;
        length3(sub3(home, game.body_of(flag.clone()).origin)) <= 32.0
    }

    /// Read the flag state (`state`).
    pub fn state(&self, game: &mut Q2GameServices, team: LmctfPlayingTeam) -> LmctfFlagState {
        let flag = self.flag(game, team);
        match flag {
            None => LmctfFlagState::Base,
            Some(flag) => {
                if self.at_home(&flag, game) && game.require_entity(&flag).solid != Q2Solid::None {
                    LmctfFlagState::Base
                } else if game.require_entity(&flag).solid == Q2Solid::None {
                    LmctfFlagState::Taken
                } else {
                    LmctfFlagState::Dropped
                }
            }
        }
    }

    /// Apply flag properties (`properties`).
    fn properties(&self, flag: ActorId, game: &mut Q2GameServices) {
        {
            let record = game.require_entity_mut(&flag);
            record.visible = true;
            record.owner = None;
            record.timestamp = 0.0;
            record.touch = Some(lmctf_flag_touch as Q2Touch);
        }
        let mut body = game.body_of(flag.clone());
        body.bounds = FLAG_BOUNDS;
        game.write_body(flag.clone(), &body, true);
        game.set_solid(flag.clone(), Q2Solid::Trigger);
        game.set_motion_kind(flag.clone(), Q2MotionKind::Toss);
        let origin = game.body_of(flag.clone()).origin;
        let end = game
            .host
            .trace(&Q2TraceRequest {
                start: origin,
                end: add3(origin, vec3(0.0, 0.0, -128.0)),
                bounds: Some(FLAG_BOUNDS),
                ignore: Some(flag.clone()),
                mask: 3,
                exclude: Vec::new(),
            })
            .end;
        let mut moved = game.body_of(flag.clone());
        moved.origin = end;
        moved.angles = Vec3::default();
        moved.velocity = Vec3::default();
        game.write_body(flag.clone(), &moved, true);
        game.show(flag.clone());
        game.schedule(flag, 0.1, lmctf_flag_wave as Q2Think);
    }

    /// Reset a flag (`reset`).
    pub fn reset(&self, flag: Option<ActorId>, player: Option<ActorId>, game: &mut Q2GameServices) {
        if let Some(flag) = flag {
            let (pos1, pos2) = {
                let record = game.require_entity(&flag);
                (record.pos1, record.pos2)
            };
            let mut body = game.body_of(flag.clone());
            body.origin = pos1;
            body.angles = pos2;
            game.write_body(flag.clone(), &body, true);
            self.properties(flag, game);
        }
        let Some(player) = player else {
            return;
        };
        if game.entity(&player).is_none() {
            return;
        }
        {
            let record = game.require_entity_mut(&player);
            record.effects &= !0x100;
            record.model3 = String::new();
        }
        game.show(player.clone());
        let owned = game.owned_of(player.clone());
        let count = game.host.inventory().count(&player, &item_id("q2:flag"));
        game.host.inventory().consume(&owned, &item_id("q2:flag"), count);
    }

    /// Reset all flags (`resetAll`).
    pub fn reset_all(&self, game: &mut Q2GameServices) {
        for team in [1u8, 2u8] {
            let flag = self.flag(game, team);
            let owner = flag.as_ref().and_then(|flag| game.require_entity(flag).owner.clone());
            self.reset(flag, owner, game);
        }
    }

    /// Find the farthest spawn (`farthest`).
    fn farthest(&self, from: ActorId, game: &mut Q2GameServices) -> Option<ActorId> {
        let mut candidates = q2_entities_named(game, "info_player_deathmatch");
        candidates.extend(q2_entities_named(game, "info_flag_red").into_iter().take(1));
        candidates.extend(q2_entities_named(game, "info_flag_blue").into_iter().take(1));
        let origin = game.body_of(from).origin;
        let mut best = None;
        let mut distance = 0.0f32;
        for candidate in &candidates {
            let current = length3(sub3(origin, game.body_of(candidate.clone()).origin));
            if current > distance {
                best = Some(candidate.clone());
                distance = current;
            }
        }
        best.or_else(|| q2_entities_named(game, "info_player_deathmatch").into_iter().next())
    }

    /// Spawn a team flag (`spawnFlag`).
    fn spawn_flag(&self, team: LmctfPlayingTeam, game: &mut Q2GameServices) {
        if self.flag(game, team).is_some() || !matches!(game.options.mode, Q2Mode::Deathmatch) || game.lmctf.rules.ctf_flags & 256 != 0 {
            return;
        }
        let classname = if team == 1 { "info_flag_red" } else { "info_flag_blue" };
        let mut spot = q2_entities_named(game, classname).into_iter().next();
        if spot.is_none() {
            spot = q2_entities_named(game, "info_player_deathmatch").into_iter().next();
            if let Some(found) = spot.clone() {
                spot = self.farthest(found, game);
            }
            if team == 2 {
                if let Some(found) = spot.clone() {
                    spot = self.farthest(found, game);
                }
            }
            if spot.is_none() {
                spot = q2_entities_named(game, if team == 1 { "info_player_start" } else { "target_changelevel" }).into_iter().next();
            }
            let Some(spot) = spot else {
                return;
            };
            {
                let record = game.require_entity_mut(&spot);
                record.classname = classname.to_string();
                record.effects |= 0x100;
                record.render_flags |= if team == 1 { 1024 } else { 4096 };
            }
            game.show(spot.clone());
            self.place_flag(team, spot, game);
            return;
        }
        if let Some(spot) = spot {
            self.place_flag(team, spot, game);
        }
    }

    /// Place a flag at a spot (`spawnFlag` placement).
    fn place_flag(&self, team: LmctfPlayingTeam, spot: ActorId, game: &mut Q2GameServices) {
        let flag = game.create("flag", BTreeMap::new());
        let body = game.body_of(spot);
        let flag_init = game.lmctf.rules.flag_init;
        {
            let record = game.require_entity_mut(&flag);
            record.count = i32::from(team);
            record.model = CTF_FLAGS[usize::from(team - 1)].model.to_string();
            record.effects = CTF_FLAGS[usize::from(team - 1)].effect;
            record.frame = if flag_init { 173 } else { 0 };
            record.pos1 = body.origin;
            record.pos2 = body.angles;
        }
        game.lmctf.flag_slots.insert(team, flag.clone());
        self.reset(Some(flag.clone()), None, game);
        self.properties(flag.clone(), game);
        let home = game.require_entity(&flag).pos1;
        let victims: Vec<ActorId> = q2_entities_named(game, "info_player_deathmatch")
            .into_iter()
            .filter(|spawn| length3(sub3(game.body_of(spawn.clone()).origin, home)) <= 256.0)
            .collect();
        for victim in victims {
            game.remove_actor(victim);
        }
    }

    /// Spawn a flag entity (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        if classname == "info_flag_red" || classname == "info_flag_blue" {
            game.set_solid(entity, Q2Solid::None);
            return true;
        }
        if classname != "flag" {
            return false;
        }
        let count = game.require_entity(&entity).count;
        if count == 1 || count == 2 {
            game.lmctf.flag_slots.insert(count as u8, entity.clone());
            self.properties(entity, game);
        }
        true
    }

    /// Run post-spawn setup (`postSpawn`).
    pub fn post_spawn(&self, game: &mut Q2GameServices) {
        game.source_callbacks.register(&lmctf_flag_callbacks());
        self.spawn_flag(1, game);
        self.spawn_flag(2, game);
    }

    /// Read the flag team (`team`).
    fn team(&self, flag: &ActorId, game: &Q2GameServices) -> LmctfPlayingTeam {
        let count = game.require_entity(flag).count;
        if count != 1 && count != 2 {
            panic!("LMCTF flag has no source team");
        }
        count as u8
    }

    /// Announce to teams (`announce`).
    fn announce(&self, game: &mut Q2GameServices, team: LmctfPlayingTeam, own: &str, other: &str) {
        for actor in game.host.players() {
            let message = if game.lmctf.states.get(&actor).map(|state| state.team) == Some(team) { own } else { other };
            lmctf_print(game, message, Some(actor));
        }
    }

    /// Play a flag sound (`sound`).
    fn sound(&self, flag: ActorId, game: &mut Q2GameServices, path: &str, volume: f64) {
        game.sound(&flag, path, 5, volume, 0.0);
    }

    /// Drop the carried flag (`drop`).
    pub fn drop(&self, player: ActorId, game: &mut Q2GameServices) {
        let Some(flag) = self.carried(&player, game) else {
            return;
        };
        self.reset(Some(flag.clone()), Some(player.clone()), game);
        self.properties(flag.clone(), game);
        {
            let record = game.require_entity_mut(&flag);
            record.owner = Some(player.clone());
            record.touch = Some(lmctf_flag_drop_touch as Q2Touch);
        }
        game.schedule(flag.clone(), 1.0, lmctf_drop_flag_think as Q2Think);
        let angles = game.host.player_view_state(&player).map(|view| view.view_angles).unwrap_or_else(|| game.body_of(player.clone()).angles);
        lmctf_toss(flag.clone(), player.clone(), game, angle_vectors(angles).forward);
        game.require_entity_mut(&flag).timestamp = game.now();
        lmctf_stat(game, &player, "flag-lost", 1);
        lmctf_score(game, &player, 0, "FC LostFlag", None);
        let team = self.team(&flag, game);
        let name = lmctf_name(game, &player);
        lmctf_print(game, &format!("{name} lost the {} flag.\n", if team == 1 { "red" } else { "blue" }), None);
    }

    /// Pick up a flag (`pickup`).
    pub fn pickup(&self, flag: ActorId, game: &mut Q2GameServices, actor: ActorId) -> bool {
        let health = game.host.combat().read(&actor).map(|combat| combat.health).unwrap_or(0.0);
        if !lmctf_flags_touchable(game)
            || !lmctf_active(game, &actor)
            || health <= 0.0
            || game.require_entity(&flag).solid != Q2Solid::Trigger
        {
            return false;
        }
        let team = self.team(&flag, game);
        let live = game.entity(&actor).is_some();
        let member = lmctf_player(game, &actor).team;
        let name = lmctf_name(game, &actor);
        let color = if team == 1 { "red" } else { "blue" };
        let now = game.now();
        if !live {
            return false;
        }
        if member == team {
            if self.at_home(&flag, game) {
                if game.host.inventory().count(&actor, &item_id("q2:flag")) > 0.0 {
                    self.capture_flag(flag, game, actor, team);
                }
            } else {
                self.sound(flag.clone(), game, if team == 1 { "ctf/r_returned.wav" } else { "ctf/b_returned.wav" }, 0.8);
                lmctf_stat(game, &actor, "returns", 1);
                lmctf_score(game, &actor, 1, "F Return", None);
                lmctf_player(game, &actor).return_flag_time = now;
                self.announce(game, team, &format!("{name} returned your flag!\n"), &format!("{name} returned the {color} flag.\n"));
                let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
                for other in actors {
                    let assist = game
                        .lmctf
                        .states
                        .get(&other)
                        .map(|state| (state.team, state.kill_carrier_time))
                        .filter(|(other_team, _)| *other_team == team && other != actor);
                    if let Some((_, kill_time)) = assist {
                        if now < kill_time + 6.0 {
                            let helper = lmctf_name(game, &other);
                            lmctf_print(game, &format!("{helper} helped {name} return the {color} flag.\n"), None);
                            lmctf_score(game, &other, 1, "F Return Assist", None);
                            lmctf_stat(game, &other, "assists", 1);
                            if let Some(member) = game.lmctf.states.get_mut(&other) {
                                member.kill_carrier_time = 0.0;
                            }
                        }
                    }
                }
                self.reset(Some(flag), None, game);
            }
            return false;
        }
        if game.lmctf.rules.ref_flags & i32::from(team) != 0 {
            return false;
        }
        {
            let record = game.require_entity_mut(&actor);
            record.effects |= 0x100;
            record.render_flags |= if team == 2 { 1024 } else { 4096 };
        }
        self.announce(game, team, &format!("{name} stole your flag!\n"), &format!("{name} stole the {color} flag.\n"));
        lmctf_stat(game, &actor, "flag-taken", 1);
        lmctf_score(game, &actor, 0, "F Pickup", None);
        if self.at_home(&flag, game) {
            game.sound(&flag, "ctf/flagtk.wav", 0, 0.7, 1.0);
            self.sound(flag.clone(), game, if team == 1 { "ctf/r_stolen.wav" } else { "ctf/b_stolen.wav" }, 0.8);
        } else if now > game.lmctf.flag_taken_sound + 8.0 {
            game.lmctf.flag_taken_sound = now;
            self.sound(flag.clone(), game, if team == 1 { "ctf/r_stolen.wav" } else { "ctf/b_stolen.wav" }, 0.8);
        }
        {
            let record = game.require_entity_mut(&flag);
            record.owner = Some(actor.clone());
            record.visible = false;
        }
        game.set_solid(flag.clone(), Q2Solid::None);
        game.show(flag.clone());
        game.schedule(flag.clone(), 0.1, lmctf_flag_wave as Q2Think);
        let model = game.require_entity(&flag).model.clone();
        game.require_entity_mut(&actor).model3 = model;
        game.show(actor.clone());
        let owned = game.owned_of(actor.clone());
        if !game.host.inventory().entries(&actor).iter().any(|entry| entry.item == "q2:flag") {
            game.host.inventory().configure(&owned, &InventoryEntry { item: item_id("q2:flag"), count: 0.0, capacity: 32767.0, count_policy: None });
        }
        game.host.inventory().give(&owned, &item_id("q2:flag"), 1.0);
        game.host_emit(Q2PresentationEvent::Pickup {
            player: actor.clone(),
            item: item_id("q2:flag"),
            icon: "a_redflag".to_string(),
            name: "Enemy Flag".to_string(),
        });
        game.sound(&actor, "misc/am_pkup.wav", 3, 1.0, 1.0);
        true
    }

    /// Capture the enemy flag (`captureFlag`).
    fn capture_flag(&self, home: ActorId, game: &mut Q2GameServices, player: ActorId, team: LmctfPlayingTeam) {
        let Some(enemy) = self.flag(game, other_ctf_team(team)) else {
            return;
        };
        let name = lmctf_name(game, &player);
        let color = if team == 1 { "blue" } else { "red" };
        let now = game.now();
        self.announce(
            game,
            other_ctf_team(team),
            &format!("{name} captured your flag!\n"),
            &format!("{name} captured the {color} flag.\n"),
        );
        let assists = [("kill_carrier_time", 6.0, "FC Frag Assist", "killing the flag carrier"), ("return_flag_time", 3.0, "F Return Assist", "returning the flag"), ("defend_flag_time", 2.0, "F Defend Assist", "defending the flag")];
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        for actor in actors {
            if game.lmctf.states.get(&actor).map(|state| state.team) != Some(team) {
                continue;
            }
            for (field, window, log, reason) in assists {
                let at = game
                    .lmctf
                    .states
                    .get(&actor)
                    .map(|state| match field {
                        "kill_carrier_time" => state.kill_carrier_time,
                        "return_flag_time" => state.return_flag_time,
                        _ => state.defend_flag_time,
                    })
                    .unwrap_or(0.0);
                if now < at + window {
                    let helper = lmctf_name(game, &actor);
                    lmctf_print(game, &format!("{helper} assisted the capture by {reason}.\n"), None);
                    lmctf_score(game, &actor, 1, log, None);
                    lmctf_stat(game, &actor, "assists", 1);
                    if let Some(member) = game.lmctf.states.get_mut(&actor) {
                        match field {
                            "kill_carrier_time" => member.kill_carrier_time = 0.0,
                            "return_flag_time" => member.return_flag_time = 0.0,
                            _ => member.defend_flag_time = 0.0,
                        }
                    }
                }
            }
        }
        let skin = game.lmctf.rules.skin_set;
        self.sound(home.clone(), game, &format!("ctf/{}score{}.wav", if team == 1 { "red" } else { "blue" }, skin + 1), 1.0);
        let origin = game.body_of(home).origin;
        game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
            effect: "bfg-explosion".to_string(),
            origin,
            direction: Vec3::default(),
            count: 0,
            color: 0,
        }));
        lmctf_score(game, &player, 5, "F Capture", None);
        lmctf_stat(game, &player, "captures", 1);
        let mut bonus = 10;
        if game.lmctf.rules.ctf_flags & 512 != 0 {
            let mut allies = 1;
            let mut enemies = 1;
            let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
            for actor in actors {
                if !lmctf_active(game, &actor) {
                    continue;
                }
                if game.lmctf.states.get(&actor).map(|state| state.team) == Some(team) {
                    allies += 1;
                } else if game.lmctf.states.get(&actor).map(|state| state.team).unwrap_or(0) != 0 {
                    enemies += 1;
                }
            }
            bonus = bonus * enemies / allies;
        }
        let actors: Vec<ActorId> = game.lmctf.states.keys().cloned().collect();
        for actor in actors {
            if game.lmctf.states.get(&actor).map(|state| state.team) == Some(team) {
                lmctf_score(game, &actor, bonus, "Team Score", None);
            }
        }
        let owner = game.require_entity(&enemy).owner.clone();
        self.reset(Some(enemy), owner, game);
    }

    /// Mark carrier damage (`hurtCarrier`).
    pub fn hurt_carrier(&self, target: &ActorId, attacker: Option<ActorId>, game: &mut Q2GameServices) {
        let Some(source) = attacker else {
            return;
        };
        if self.carried(target, game).is_none() {
            return;
        }
        if game.lmctf.states.contains_key(&source) {
            let now = game.now();
            if let Some(state) = game.lmctf.states.get_mut(&source) {
                state.hit_carrier_time = now;
            }
        }
    }

    /// Score a frag (`frag`).
    pub fn frag(&self, victim: ActorId, attacker: ActorId, game: &mut Q2GameServices) {
        let source_team = game.lmctf.states.get(&attacker).map(|state| state.team);
        let target = game.lmctf.states.get(&victim).map(|state| (state.team, state.hit_carrier_time));
        let (Some(source_team), Some((target_team, hit_time))) = (source_team, target) else {
            return;
        };
        if source_team == 0 || target_team == 0 || source_team == target_team || attacker == victim {
            return;
        }
        let own = self.flag(game, source_team);
        let enemy = self.flag(game, target_team);
        let now = game.now();
        let color = if source_team == 1 { "red" } else { "blue" };
        let attacker_origin = game.body_of(attacker.clone()).origin;
        let victim_origin = game.body_of(victim.clone()).origin;
        if let Some(own) = own {
            let owner = game.require_entity(&own).owner.clone();
            let owner_active = owner.as_ref().is_some_and(|owner| lmctf_active(game, owner));
            if owner.is_none() || !owner_active {
                if self.at_home(&own, game) {
                    let origin = game.body_of(own.clone()).origin;
                    if length3(sub3(attacker_origin, origin)) < 800.0 || length3(sub3(victim_origin, origin)) < 800.0 {
                        let name = lmctf_name(game, &attacker);
                        lmctf_score(game, &attacker, 2, "F Def", None);
                        lmctf_stat(game, &attacker, "defense-flag", 1);
                        lmctf_print(game, &format!("{name} defends the {color} flag.\n"), None);
                        if let Some(state) = game.lmctf.states.get_mut(&attacker) {
                            state.defend_flag_time = now;
                        }
                    }
                } else {
                    let home = game.require_entity(&own).pos1;
                    if length3(sub3(attacker_origin, home)) < 600.0 || length3(sub3(victim_origin, home)) < 600.0 {
                        let name = lmctf_name(game, &attacker);
                        lmctf_score(game, &attacker, 1, "F Base Def", None);
                        lmctf_stat(game, &attacker, "defense-base", 1);
                        lmctf_print(game, &format!("{name} defends the {color} base.\n"), None);
                    }
                    let origin = game.body_of(own.clone()).origin;
                    if length3(sub3(attacker_origin, origin)) < 400.0 || length3(sub3(victim_origin, origin)) < 400.0 {
                        if let Some(state) = game.lmctf.states.get_mut(&attacker) {
                            state.defend_flag_time = now;
                        }
                    }
                }
                let carrier = enemy.as_ref().and_then(|enemy| game.require_entity(enemy).owner.clone());
                if let Some(carrier) = carrier {
                    if carrier != attacker && lmctf_active(game, &carrier) {
                        if now < hit_time + 2.0 {
                            let name = lmctf_name(game, &attacker);
                            lmctf_score(game, &attacker, 3, "FC Def", None);
                            lmctf_stat(game, &attacker, "defense-carrier", 1);
                            lmctf_print(game, &format!("{name} defends the {color} flag carrier from an aggressive enemy.\n"), None);
                        } else {
                            let origin = game.body_of(carrier).origin;
                            if length3(sub3(attacker_origin, origin)) < 500.0 || length3(sub3(victim_origin, origin)) < 500.0 {
                                let name = lmctf_name(game, &attacker);
                                lmctf_score(game, &attacker, 2, "FC Def", None);
                                lmctf_stat(game, &attacker, "defense-carrier", 1);
                                lmctf_print(game, &format!("{name} defends the {color} flag carrier.\n"), None);
                            }
                        }
                    }
                }
            }
        }
        if self.carried(&victim, game).is_some() {
            let name = lmctf_name(game, &attacker);
            lmctf_score(game, &attacker, 2, "FC Frag", None);
            lmctf_stat(game, &attacker, "offense-carrier", 1);
            lmctf_print(game, &format!("{name} killed the enemy flag carrier.\n"), None);
            if let Some(state) = game.lmctf.states.get_mut(&attacker) {
                state.kill_carrier_time = now;
            }
        }
    }
}
