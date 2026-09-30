//! Q2 CTF flags (`src/content/q2/multiplayer/ctf/flags.ts`).
//!
//! Original Quake II CTF flag/bonus source. GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3, add3, length3, scale3, sub3, vec3};

use crate::contract::InventoryEntry;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::movedir;
use crate::q2::foundation::host::{
    Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2SoundEvent, Q2SoundLoop, Q2Think, Q2Touch, Q2TraceRequest,
};
use crate::q2::foundation::items::{Q2ConsoleGive, Q2ItemDefinition, Q2ItemKindData};
use crate::q2::support::contracts::TouchContact;

use super::types::{
    CTF_FLAGS, Q2CtfFlagState, Q2CtfHooks, Q2CtfMatchPhase, Q2CtfPlayingTeam, Q2CtfPrintLevel, ctf_carried_flag, ctf_flag,
    ctf_name, ctf_player, ctf_print, ctf_score, ctf_team_name, item_id, other_ctf_team,
};

/// Flag team for a classname (`ctfFlagTeam`).
pub fn ctf_flag_team(classname: &str) -> Option<Q2CtfPlayingTeam> {
    if classname == CTF_FLAGS[0].classname {
        Some(1)
    } else if classname == CTF_FLAGS[1].classname {
        Some(2)
    } else {
        None
    }
}

/// Whether the viewer can see the target (`ctfCanSee`).
pub fn ctf_can_see(game: &mut Q2GameServices, target: &ActorId, viewer: &ActorId) -> bool {
    if game.require_entity(target).motion == Q2MotionKind::Push {
        return false;
    }
    let body = game.body_of(target.clone());
    let viewer_body = game.body_of(viewer.clone());
    let height = f64::from(game.require_entity(viewer).view_height);
    let eye = add3(viewer_body.origin, vec3(0.0, 0.0, height as f32));
    for x in [body.bounds.min.x, body.bounds.max.x] {
        for y in [body.bounds.min.y, body.bounds.max.y] {
            for z in [body.bounds.min.z, body.bounds.max.z] {
                let end = add3(body.origin, vec3(x, y, z));
                let trace = game.host.trace(&Q2TraceRequest {
                    start: eye,
                    end,
                    bounds: None,
                    ignore: Some(viewer.clone()),
                    mask: 3,
                    exclude: Vec::new(),
                });
                if trace.fraction == 1.0 {
                    return true;
                }
            }
        }
    }
    false
}

/// CTF flags (`Q2CtfFlags`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfFlags {
    /// Session hooks.
    pub hooks: Q2CtfHooks,
}

/// Flag callbacks (`Q2CtfFlags::callbacks`).
pub fn ctf_flag_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("CTFFlagThink", ctf_flag_animate as Q2Think);
    callbacks.think.insert("CTFFlagSetup", ctf_flag_setup as Q2Think);
    callbacks.think.insert("CTFDropFlagThink", ctf_drop_flag_return as Q2Think);
    callbacks.touch.insert("CTFDropFlagTouch", ctf_drop_flag_touch as Q2Touch);
    callbacks.touch.insert("CTF_FlagTouch", ctf_flag_touch as Q2Touch);
    callbacks
}

/// Flag item pickup (`register` pickup).
fn ctf_flag_item_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let flags = Q2CtfFlags { hooks: super::ctf_hooks(game) };
    flags.pickup(entity, game, player.id().clone());
    false
}

/// Flag setup think (`setup`).
fn ctf_flag_setup(entity: ActorId, game: &mut Q2GameServices) {
    let bounds = Bounds { min: vec3(-15.0, -15.0, -15.0), max: vec3(15.0, 15.0, 15.0) };
    let origin = game.body_of(entity.clone()).origin;
    let trace = game.host.trace(&Q2TraceRequest {
        start: origin,
        end: add3(origin, vec3(0.0, 0.0, -128.0)),
        bounds: Some(bounds),
        ignore: Some(entity.clone()),
        mask: 3,
        exclude: Vec::new(),
    });
    if trace.start_solid {
        let classname = game.require_entity(&entity).classname.clone();
        game.host.diagnostic(&format!("CTFFlagSetup: {classname} starts solid"));
        game.remove_actor(entity);
        return;
    }
    let mut body = game.body_of(entity.clone());
    body.bounds = bounds;
    body.origin = trace.end;
    game.write_body(entity.clone(), &body, true);
    game.require_entity_mut(&entity).touch = Some(ctf_flag_touch as Q2Touch);
    game.set_solid(entity.clone(), Q2Solid::Trigger);
    game.set_motion_kind(entity.clone(), Q2MotionKind::Toss);
    game.show(entity.clone());
    game.schedule(entity, 0.1, ctf_flag_animate as Q2Think);
}

/// Flag animate think (`animate`).
fn ctf_flag_animate(entity: ActorId, game: &mut Q2GameServices) {
    {
        let record = game.require_entity_mut(&entity);
        if record.solid != Q2Solid::None {
            record.frame = 173 + (record.frame - 173 + 1).rem_euclid(16);
        }
    }
    game.show(entity.clone());
    game.schedule(entity, 0.1, ctf_flag_animate as Q2Think);
}

/// Flag touch (`touch`).
fn ctf_flag_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let flags = Q2CtfFlags { hooks: super::ctf_hooks(game) };
    flags.pickup(entity, game, contact.other);
}

/// Dropped flag touch (`dropTouch`).
fn ctf_drop_flag_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let owner = game.require_entity(&entity).owner.clone();
    let fresh = game.now() < game.require_entity(&entity).timestamp + 2.0;
    if owner.as_ref() == Some(&contact.other) && fresh {
        return;
    }
    let flags = Q2CtfFlags { hooks: super::ctf_hooks(game) };
    flags.pickup(entity, game, contact.other);
}

/// Dropped flag return think (`returnDropped`).
fn ctf_drop_flag_return(entity: ActorId, game: &mut Q2GameServices) {
    let classname = game.require_entity(&entity).classname.clone();
    if let Some(team) = ctf_flag_team(&classname) {
        let flags = Q2CtfFlags { hooks: super::ctf_hooks(game) };
        flags.reset(game, team);
        ctf_print(game, &format!("The {} flag has returned!\n", ctf_team_name(team)), None, Q2CtfPrintLevel::High);
    }
}

impl Q2CtfFlags {
    /// Register flag items (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        for team in [1u8, 2u8] {
            let flag = ctf_flag(team);
            self.hooks.items.register_item(
                game,
                Q2ItemDefinition {
                    classname: flag.classname.to_string(),
                    model: flag.model.to_string(),
                    icon: flag.icon.to_string(),
                    name: format!("{} Flag", if team == 1 { "Red" } else { "Blue" }),
                    sound: "ctf/flagtk.wav".to_string(),
                    rotate: false,
                    respawn: 0.0,
                    console_give: Some(Q2ConsoleGive::IndividualOnly),
                    kind: Q2ItemKindData::Custom {
                        capacity: 1.0,
                        quantity: 1.0,
                        coop_stay: false,
                        droppable: false,
                        pickup: ctf_flag_item_pickup,
                        use_item: None,
                    },
                },
            );
        }
    }

    /// Spawn a flag (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        let Some(team) = ctf_flag_team(&classname) else {
            return false;
        };
        {
            let record = game.require_entity_mut(&entity);
            if record.model.is_empty() {
                record.model = ctf_flag(team).model.to_string();
            }
            record.effects |= ctf_flag(team).effect;
            record.render_flags |= 512;
            record.frame = 173;
        }
        game.source_callbacks.register(&ctf_flag_callbacks());
        let delay = game.host.frame_seconds() * 2.0;
        game.schedule(entity, delay, ctf_flag_setup as Q2Think);
        true
    }

    /// Read the base flag (`base`).
    pub fn base(&self, game: &mut Q2GameServices, team: Q2CtfPlayingTeam) -> Option<ActorId> {
        let classname = ctf_flag(team).classname;
        game.entities
            .values()
            .find(|entity| entity.classname == classname && entity.spawnflags & 0x30000 == 0)
            .map(|entity| entity.actor.id().clone())
    }

    /// Read the flag state (`state`).
    pub fn state(&self, game: &mut Q2GameServices, team: Q2CtfPlayingTeam) -> Q2CtfFlagState {
        let classname = ctf_flag(team).classname;
        if game.entities.values().any(|entity| entity.classname == classname && entity.spawnflags & 0x30000 != 0) {
            return Q2CtfFlagState::Dropped;
        }
        match self.base(game, team) {
            Some(base) if game.require_entity(&base).solid == Q2Solid::Trigger => Q2CtfFlagState::Base,
            _ => Q2CtfFlagState::Taken,
        }
    }

    /// Reset a flag (`reset`).
    pub fn reset(&self, game: &mut Q2GameServices, team: Q2CtfPlayingTeam) {
        let classname = ctf_flag(team).classname;
        let actors: Vec<ActorId> = game
            .entities
            .values()
            .filter(|entity| entity.classname == classname)
            .map(|entity| entity.actor.id().clone())
            .collect();
        for actor in actors {
            if game.require_entity(&actor).spawnflags & 0x30000 != 0 {
                game.remove_actor(actor);
            } else {
                {
                    let record = game.require_entity_mut(&actor);
                    record.visible = true;
                    record.server_flags &= !1;
                }
                game.set_solid(actor.clone(), Q2Solid::Trigger);
                game.show(actor.clone());
                game.host_emit(Q2PresentationEvent::EntityEvent { actor, event: 1 });
            }
        }
    }

    /// Reset all flags (`resetAll`).
    pub fn reset_all(&self, game: &mut Q2GameServices) {
        self.reset(game, 1);
        self.reset(game, 2);
    }

    /// Pick up a flag (`pickup`).
    pub fn pickup(&self, entity: ActorId, game: &mut Q2GameServices, actor: ActorId) {
        let phase = game.ctf.match_state.phase;
        if phase == Q2CtfMatchPhase::Setup || phase == Q2CtfMatchPhase::Pregame {
            return;
        }
        let classname = game.require_entity(&entity).classname.clone();
        let Some(team) = ctf_flag_team(&classname) else {
            return;
        };
        let Some(member_team) = game.ctf.states.get(&actor).map(|state| state.team) else {
            return;
        };
        if member_team == 0 || game.entity(&actor).is_none() {
            return;
        }
        let hooks = super::ctf_hooks(game);
        let spectator = match (hooks.player)(actor.clone(), game) {
            Some(player) => player.spectator,
            None => return,
        };
        let health = game.host.combat().read(&actor).map(|combat| combat.health).unwrap_or(0.0);
        if spectator || health <= 0.0 {
            return;
        }
        if game.require_entity(&entity).solid != Q2Solid::Trigger {
            return;
        }
        let now = game.now();
        let enemy = other_ctf_team(team);
        let flag = ctf_flag(team);
        let dropped = game.require_entity(&entity).spawnflags & 0x30000 != 0;
        if member_team == team {
            if dropped {
                ctf_score(game, &actor, 1);
                ctf_player(game, &actor).last_returned_flag = Some(now);
                let name = ctf_name(game, &actor);
                ctf_print(
                    game,
                    &format!("{name} returned the {} flag!\n", ctf_team_name(team)),
                    None,
                    Q2CtfPrintLevel::High,
                );
                self.flag_sound(entity.clone(), game, "ctf/flagret.wav");
                self.reset(game, team);
                return;
            }
            let enemy_item = item_id(ctf_flag(enemy).item);
            if game.host.inventory().count(&actor, &enemy_item) == 0.0 {
                self.targets(entity, game, actor);
                return;
            }
            let owned = game.owned_of(actor.clone());
            game.host.inventory().consume(&owned, &enemy_item, 1.0);
            {
                let matched = &mut game.ctf.match_state;
                if team == 1 {
                    matched.team1 += 1;
                } else {
                    matched.team2 += 1;
                }
                matched.last_flag_capture = Some(now);
                matched.last_capture_team = Some(team);
            }
            ctf_score(game, &actor, 15);
            let ghost_code = ctf_player(game, &actor).ghost_code;
            if let Some(code) = ghost_code {
                if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                    ghost.captures += 1;
                }
            }
            let name = ctf_name(game, &actor);
            ctf_print(
                game,
                &format!("{name} captured the {} flag!\n", ctf_team_name(enemy)),
                None,
                Q2CtfPrintLevel::High,
            );
            self.flag_sound(entity.clone(), game, "ctf/flagcap.wav");
            for teammate in game.host.players() {
                let member = game
                    .ctf
                    .states
                    .get(&teammate)
                    .map(|state| (state.team, state.last_returned_flag, state.last_fragged_carrier));
                let Some((their_team, returned, fragged)) = member else {
                    continue;
                };
                if their_team != team {
                    if let Some(state) = game.ctf.states.get_mut(&teammate) {
                        state.last_hurt_carrier = None;
                    }
                    continue;
                }
                if teammate != actor {
                    ctf_score(game, &teammate, 10);
                }
                if let Some(at) = returned {
                    if at + 10.0 > now {
                        ctf_score(game, &teammate, 1);
                        let name = ctf_name(game, &teammate);
                        ctf_print(game, &format!("{name} gets an assist for returning the flag!\n"), None, Q2CtfPrintLevel::High);
                    }
                }
                if let Some(at) = fragged {
                    if at + 10.0 > now {
                        ctf_score(game, &teammate, 2);
                        let name = ctf_name(game, &teammate);
                        ctf_print(
                            game,
                            &format!("{name} gets an assist for fragging the flag carrier!\n"),
                            None,
                            Q2CtfPrintLevel::High,
                        );
                    }
                }
            }
            self.reset_all(game);
            self.targets(entity, game, actor);
            return;
        }
        let item = item_id(flag.item);
        if game.host.inventory().count(&actor, &item) != 0.0 {
            return;
        }
        let owned = game.owned_of(actor.clone());
        if !game.host.inventory().entries(&actor).iter().any(|entry| entry.item == item) {
            game.host.inventory().configure(
                &owned,
                &InventoryEntry { item: item.clone(), count: 0.0, capacity: 1.0, count_policy: None },
            );
        }
        game.host.inventory().give(&owned, &item, 1.0);
        ctf_player(game, &actor).flag_since = now;
        let name = ctf_name(game, &actor);
        ctf_print(game, &format!("{name} got the {} flag!\n", ctf_team_name(team)), None, Q2CtfPrintLevel::High);
        game.host_emit(Q2PresentationEvent::Pickup {
            player: actor.clone(),
            item: item.clone(),
            icon: flag.icon.to_string(),
            name: format!("{} Flag", if team == 1 { "Red" } else { "Blue" }),
        });
        game.sound(&actor, "ctf/flagtk.wav", 3, 1.0, 1.0);
        self.targets(entity.clone(), game, actor);
        if game.entity(&entity).is_none() {
            return;
        }
        if dropped {
            game.remove_actor(entity);
            return;
        }
        {
            let record = game.require_entity_mut(&entity);
            record.visible = false;
            record.server_flags |= 1;
        }
        game.set_solid(entity.clone(), Q2Solid::None);
        game.show(entity);
    }

    /// Fire flag targets once (`targets`).
    fn targets(&self, entity: ActorId, game: &mut Q2GameServices, actor: ActorId) {
        if game.require_entity(&entity).spawnflags & 0x40000 == 0 {
            game.require_entity_mut(&entity).spawnflags |= 0x40000;
            let authored = game.require_entity(&entity).authored_target();
            game.use_targets(&authored, Some(&actor), false);
        }
    }

    /// Play a flag sound (`flagSound`).
    fn flag_sound(&self, entity: ActorId, game: &mut Q2GameServices, path: &str) {
        let origin = game.body_of(entity.clone()).origin;
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(entity),
            origin,
            path: path.to_string(),
            channel: 2,
            volume: 1.0,
            attenuation: 0.0,
            reliable: true,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }

    /// Drop the carried flag (`drop`).
    pub fn drop(&self, entity: ActorId, game: &mut Q2GameServices) -> Option<ActorId> {
        let team = ctf_carried_flag(game, &entity)?;
        let flag = ctf_flag(team);
        let dropped = game.create(flag.classname, BTreeMap::new());
        let body = game.body_of(entity.clone());
        let view = game.host.player_view_state(&entity).map(|state| state.view_angles).unwrap_or(body.angles);
        let forward = movedir(view);
        let now = game.now();
        {
            let record = game.require_entity_mut(&dropped);
            record.model = flag.model.to_string();
            record.effects = flag.effect;
            record.render_flags = 512;
            record.spawnflags = 0x10000;
            record.owner = Some(entity.clone());
            record.timestamp = now;
            record.touch = Some(ctf_drop_flag_touch as Q2Touch);
        }
        let bounds = Bounds { min: vec3(-15.0, -15.0, -15.0), max: vec3(15.0, 15.0, 15.0) };
        let origin = game
            .host
            .trace(&Q2TraceRequest {
                start: body.origin,
                end: add3(add3(body.origin, scale3(forward, 24.0)), vec3(0.0, 0.0, -16.0)),
                bounds: Some(bounds),
                ignore: Some(entity.clone()),
                mask: 3,
                exclude: Vec::new(),
            })
            .end;
        {
            let mut moved = game.body_of(dropped.clone());
            moved.origin = origin;
            moved.bounds = bounds;
            let push = scale3(forward, 100.0);
            moved.velocity = Vec3 { x: push.x, y: push.y, z: 300.0 };
            game.write_body(dropped.clone(), &moved, true);
        }
        game.set_solid(dropped.clone(), Q2Solid::Trigger);
        game.set_motion_kind(dropped.clone(), Q2MotionKind::Toss);
        game.show(dropped.clone());
        game.schedule(dropped.clone(), 30.0, ctf_drop_flag_return as Q2Think);
        let owned = game.owned_of(entity.clone());
        game.host.inventory().consume(&owned, &item_id(flag.item), 1.0);
        let name = ctf_name(game, &entity);
        ctf_print(game, &format!("{name} lost the {} flag!\n", ctf_team_name(team)), None, Q2CtfPrintLevel::High);
        Some(dropped)
    }

    /// Mark carrier damage (`hurtCarrier`).
    pub fn hurt_carrier(&self, target: ActorId, attacker: Option<ActorId>, game: &mut Q2GameServices) {
        let victim = game.ctf.states.get(&target).map(|state| state.team);
        let aggressor = attacker.as_ref().and_then(|actor| game.ctf.states.get(actor).map(|state| state.team));
        let (Some(victim_team), Some(aggressor_team)) = (victim, aggressor) else {
            return;
        };
        if victim_team == 0 || aggressor_team == 0 || victim_team == aggressor_team {
            return;
        }
        if ctf_carried_flag(game, &target) == Some(other_ctf_team(victim_team)) {
            let now = game.now();
            if let Some(state) = attacker.as_ref().and_then(|actor| game.ctf.states.get_mut(actor)) {
                state.last_hurt_carrier = Some(now);
            }
        }
    }

    /// Score a frag (`frag`).
    pub fn frag(&self, victim: ActorId, attacker: Option<ActorId>, game: &mut Q2GameServices) {
        let victim_code = game.ctf.states.get(&victim).and_then(|state| state.ghost_code);
        if let Some(code) = victim_code {
            if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                ghost.deaths += 1;
            }
        }
        let Some(source) = attacker.clone() else {
            return;
        };
        let attacker_team = game.ctf.states.get(&source).map(|state| state.team);
        let victim_state = game.ctf.states.get(&victim).map(|state| (state.team, state.last_hurt_carrier));
        let (Some(attacker_team), Some((victim_team, hurt))) = (attacker_team, victim_state) else {
            return;
        };
        if source == victim {
            return;
        }
        let attacker_code = game.ctf.states.get(&source).and_then(|state| state.ghost_code);
        if let Some(code) = attacker_code {
            if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                ghost.kills += 1;
            }
        }
        if victim_team == 0 || attacker_team == 0 || victim_team == attacker_team {
            return;
        }
        let now = game.now();
        if ctf_carried_flag(game, &victim) == Some(attacker_team) {
            if let Some(state) = game.ctf.states.get_mut(&source) {
                state.last_fragged_carrier = Some(now);
            }
            ctf_score(game, &source, 2);
            for member in game.ctf.states.values_mut() {
                if member.team == attacker_team {
                    member.last_hurt_carrier = None;
                }
            }
            ctf_print(
                game,
                "BONUS: 2 points for fragging enemy flag carrier.\n",
                Some(source),
                Q2CtfPrintLevel::Medium,
            );
            return;
        }
        if let Some(at) = hurt {
            if now - at < 8.0 && ctf_carried_flag(game, &source).is_none() {
                ctf_score(game, &source, 2);
                if let Some(code) = attacker_code {
                    if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                        ghost.carrier_defense += 1;
                    }
                }
                let name = ctf_name(game, &source);
                ctf_print(
                    game,
                    &format!("{name} defends {}'s flag carrier against an aggressive enemy\n", ctf_team_name(attacker_team)),
                    None,
                    Q2CtfPrintLevel::Medium,
                );
                return;
            }
        }
        let Some(flag) = self.base(game, attacker_team) else {
            return;
        };
        let victim_origin = game.body_of(victim.clone()).origin;
        let attacker_origin = game.body_of(source.clone()).origin;
        let flag_origin = game.body_of(flag.clone()).origin;
        if length3(sub3(victim_origin, flag_origin)) < 400.0
            || length3(sub3(attacker_origin, flag_origin)) < 400.0
            || ctf_can_see(game, &flag, &victim)
            || ctf_can_see(game, &flag, &source)
        {
            ctf_score(game, &source, 1);
            if let Some(code) = attacker_code {
                if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                    ghost.base_defense += 1;
                }
            }
            let name = ctf_name(game, &source);
            let at_base = game.require_entity(&flag).solid == Q2Solid::None;
            ctf_print(
                game,
                &format!(
                    "{name} defends the {} {}.\n",
                    ctf_team_name(attacker_team),
                    if at_base { "base" } else { "flag" }
                ),
                None,
                Q2CtfPrintLevel::Medium,
            );
            return;
        }
        let mut carrier = None;
        for actor in game.host.players() {
            let same_team = game.ctf.states.get(&actor).map(|state| state.team) == Some(attacker_team);
            if same_team && ctf_carried_flag(game, &actor) == Some(victim_team) {
                carrier = Some(actor);
                break;
            }
        }
        let Some(carrier) = carrier else {
            return;
        };
        if game.entity(&carrier).is_none() || carrier == source {
            return;
        }
        let carrier_origin = game.body_of(carrier.clone()).origin;
        if length3(sub3(victim_origin, carrier_origin)) < 400.0
            || length3(sub3(attacker_origin, carrier_origin)) < 400.0
            || ctf_can_see(game, &carrier, &victim)
            || ctf_can_see(game, &carrier, &source)
        {
            ctf_score(game, &source, 1);
            if let Some(code) = attacker_code {
                if let Some(ghost) = game.ctf.match_state.ghosts.get_mut(&code) {
                    ghost.carrier_defense += 1;
                }
            }
            let name = ctf_name(game, &source);
            ctf_print(
                game,
                &format!("{name} defends {}'s flag carrier.\n", ctf_team_name(attacker_team)),
                None,
                Q2CtfPrintLevel::Medium,
            );
        }
    }

    /// Update carrier effects (`effects`).
    pub fn effects(&self, entity: ActorId, game: &mut Q2GameServices) {
        let team = ctf_carried_flag(game, &entity);
        let health = game.host.combat().read(&entity).map(|combat| combat.health).unwrap_or(0.0);
        {
            let record = game.require_entity_mut(&entity);
            record.effects &= !0xc0000;
            if let Some(carried) = team {
                if health > 0.0 {
                    record.effects |= ctf_flag(carried).effect;
                }
            }
            record.model3 = team.map(|carried| ctf_flag(carried).model.to_string()).unwrap_or_default();
        }
        game.show(entity);
    }
}
