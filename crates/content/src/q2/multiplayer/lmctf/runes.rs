//! Q2 LMCTF runes (`src/content/q2/multiplayer/lmctf/runes.ts`).
//!
//! LM_CTF g_runes.c and Vampire g_combat.c additions. GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3, add3, length3, sub3, vec3};

use crate::contract::{InventoryEntry, RegularArmorState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2Think, Q2Touch, Q2TraceRequest};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData};
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::TouchContact;

use super::super::ctf::types::item_id;
use super::flags::LmctfFlags;
use super::types::{LMCTF_RUNES, LmctfHooks, LmctfRune, LmctfRuneDefinition, lmctf_player, lmctf_toss};

/// LMCTF rune bounds.
const RUNE_BOUNDS: Bounds = Bounds { min: Vec3 { x: -15.0, y: -15.0, z: -15.0 }, max: Vec3 { x: 15.0, y: 15.0, z: 15.0 } };

/// Single-precision rounding (`Math.fround`).
fn fround(value: f64) -> f64 {
    f64::from(value as f32)
}

/// LMCTF runes checkpoint (`LmctfRunes::capture`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LmctfRunesCheckpoint {
    /// Forward animation.
    pub forward: bool,
}

/// LMCTF runes (`LmctfRunes`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfRunes {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

/// Rune callbacks (`LmctfRunes::callbacks`).
pub fn lmctf_rune_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("lmctf:Rune_Think", lmctf_rune_think as Q2Think);
    callbacks.think.insert("lmctf:Drop_Rune_Think", lmctf_drop_rune_think as Q2Think);
    callbacks.touch.insert("lmctf:Rune_Touch", lmctf_rune_touch as Q2Touch);
    callbacks.touch.insert("lmctf:Rune_DropTouch", lmctf_rune_drop_touch as Q2Touch);
    callbacks
}

/// Rune item pickup (`register` pickup).
fn lmctf_rune_item_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    let runes = LmctfRunes { hooks: super::lmctf_hooks(game) };
    runes.pickup(entity, game, player.id().clone());
    false
}

/// Rune item use (`register` use).
fn lmctf_rune_item_use(player: OwnedActor, game: &mut Q2GameServices) -> bool {
    let runes = LmctfRunes { hooks: super::lmctf_hooks(game) };
    runes.drop(player.id(), game)
}

/// Rune think (`think`).
fn lmctf_rune_think(entity: ActorId, game: &mut Q2GameServices) {
    let runes = LmctfRunes { hooks: super::lmctf_hooks(game) };
    if game.require_entity(&entity).solid != Q2Solid::None {
        let kind = runes.definition(&game.require_entity(&entity).classname.clone()).kind;
        let frame = game.require_entity(&entity).frame;
        match kind {
            LmctfRune::Damage => {
                let frame = if game.lmctf.runes_forward { frame + 1 } else { frame - 1 };
                game.require_entity_mut(&entity).frame = frame;
                if frame >= 5 {
                    game.lmctf.runes_forward = false;
                } else if frame <= 0 {
                    game.lmctf.runes_forward = true;
                }
            }
            LmctfRune::Haste => game.require_entity_mut(&entity).frame = if (1..=15).contains(&frame) { frame + 1 } else { 5 },
            LmctfRune::Regen => game.require_entity_mut(&entity).frame = (frame + 1) % 14,
            LmctfRune::Resist | LmctfRune::Vampire => game.require_entity_mut(&entity).frame = (frame + 1) % 15,
        }
    }
    game.show(entity.clone());
    game.schedule(entity.clone(), 0.1, lmctf_rune_think as Q2Think);
    if game.require_entity(&entity).timestamp + 30.0 < game.now() {
        let spot = runes.random_spot(game).or_else(|| LmctfFlags { hooks: super::lmctf_hooks(game) }.flag(game, 1));
        if let Some(spot) = spot {
            let origin = game.body_of(spot).origin;
            let mut body = game.body_of(entity.clone());
            body.origin = origin;
            game.write_body(entity.clone(), &body, true);
            runes.toss(entity.clone(), game);
        }
        game.require_entity_mut(&entity).timestamp = game.now();
    }
}

/// Rune touch (`touch`).
fn lmctf_rune_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let runes = LmctfRunes { hooks: super::lmctf_hooks(game) };
    runes.pickup(entity, game, contact.other);
}

/// Rune drop touch (`dropTouch`).
fn lmctf_rune_drop_touch(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.require_entity(&entity).owner.as_ref() == Some(&contact.other) {
        return;
    }
    let runes = LmctfRunes { hooks: super::lmctf_hooks(game) };
    runes.pickup(entity, game, contact.other);
}

/// Dropped rune think (`droppedThink`).
fn lmctf_drop_rune_think(entity: ActorId, game: &mut Q2GameServices) {
    {
        let record = game.require_entity_mut(&entity);
        record.owner = None;
        record.touch = Some(lmctf_rune_touch as Q2Touch);
    }
    game.schedule(entity, 0.1, lmctf_rune_think as Q2Think);
}

impl LmctfRunes {
    /// Register rune items (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        for rune in LMCTF_RUNES {
            self.hooks.items.register_item(
                game,
                Q2ItemDefinition {
                    classname: rune.classname.to_string(),
                    model: rune.model.to_string(),
                    icon: rune.icon.to_string(),
                    name: rune.name.to_string(),
                    sound: "items/pkup.wav".to_string(),
                    rotate: false,
                    respawn: 0.0,
                    console_give: None,
                    kind: Q2ItemKindData::Custom {
                        capacity: 1.0,
                        quantity: 1.0,
                        coop_stay: false,
                        droppable: false,
                        pickup: lmctf_rune_item_pickup,
                        use_item: Some(lmctf_rune_item_use),
                    },
                },
            );
        }
    }

    /// Capture rune state (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> LmctfRunesCheckpoint {
        LmctfRunesCheckpoint { forward: game.lmctf.runes_forward }
    }

    /// Restore rune state (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, saved: &LmctfRunesCheckpoint) {
        game.lmctf.runes_forward = saved.forward;
    }

    /// Read the held rune (`held`).
    pub fn held(&self, actor: &ActorId, game: &mut Q2GameServices) -> Option<LmctfRune> {
        let rune = game.lmctf.states.get(actor).and_then(|state| state.rune.clone())?;
        let entity = game.entity(&rune)?;
        LMCTF_RUNES.iter().find(|definition| definition.classname == entity.classname).map(|definition| definition.kind)
    }

    /// Read a rune definition (`definition`).
    fn definition(&self, classname: &str) -> LmctfRuneDefinition {
        LMCTF_RUNES
            .iter()
            .find(|definition| definition.classname == classname)
            .copied()
            .unwrap_or_else(|| panic!("Unknown LMCTF rune {classname}"))
    }

    /// Find a random rune spot (`randomSpot`).
    fn random_spot(&self, game: &mut Q2GameServices) -> Option<ActorId> {
        for classname in ["item_health_small", "item_health_large", "item_health"] {
            let spots: Vec<ActorId> =
                game.entities.values().filter(|entity| entity.classname == classname).map(|entity| entity.actor.id().clone()).collect();
            if !spots.is_empty() {
                let index = 0.max(20.min((game.random() * spots.len() as f64).trunc() as i32) - 1) as usize;
                return spots.get(index).cloned();
            }
        }
        None
    }

    /// Find the farthest rune spot (`farthestSpot`).
    fn farthest_spot(&self, game: &mut Q2GameServices) -> Option<ActorId> {
        let entities: Vec<ActorId> = game.entities.values().map(|entity| entity.actor.id().clone()).collect();
        let mut best = None;
        let mut distance = 0.0f64;
        for (index, spot) in entities.iter().enumerate() {
            if game.require_entity(spot).classname != "item_health" {
                continue;
            }
            let mut nearest: f64 = 9999999.0;
            for definition in LMCTF_RUNES {
                // G_Find starts after the health edict and uses only the first of each rune.
                let rune = entities[index + 1..].iter().find(|entity| game.require_entity(entity).classname == definition.classname);
                if let Some(rune) = rune {
                    let origin = game.body_of(spot.clone()).origin;
                    nearest = nearest.min(f64::from(length3(sub3(origin, game.body_of(rune.clone()).origin))));
                }
            }
            if nearest > distance {
                best = Some(spot.clone());
                distance = nearest;
            }
        }
        best.or_else(|| self.random_spot(game))
    }

    /// Make a rune visible (`visible`).
    fn visible(&self, entity: ActorId, game: &mut Q2GameServices) {
        let model = self.definition(&game.require_entity(&entity).classname.clone()).model;
        {
            let record = game.require_entity_mut(&entity);
            record.visible = true;
            record.server_flags &= !1;
            record.spawnflags = 0x10000;
            record.model = model.to_string();
        }
        game.set_motion_kind(entity.clone(), Q2MotionKind::Toss);
        game.set_solid(entity.clone(), Q2Solid::Trigger);
        game.show(entity);
    }

    /// Toss a rune (`toss`).
    fn toss(&self, entity: ActorId, game: &mut Q2GameServices) {
        self.visible(entity.clone(), game);
        game.require_entity_mut(&entity).touch = Some(lmctf_rune_touch as Q2Touch);
        let origin = game.body_of(entity.clone()).origin;
        let destination = add3(origin, vec3(0.0, 0.0, 48.0));
        let owner = game.require_entity_mut(&entity).owner.take();
        let velocity =
            vec3((-2000.0 + game.random() * 4000.0) as f32, (-2000.0 + game.random() * 4000.0) as f32, (800.0 + game.random() * 200.0) as f32);
        let trace = game.host.trace(&Q2TraceRequest {
            start: origin,
            end: destination,
            bounds: Some(RUNE_BOUNDS),
            ignore: owner,
            mask: 3,
            exclude: Vec::new(),
        });
        let mut body = game.body_of(entity.clone());
        body.bounds = RUNE_BOUNDS;
        body.origin = if trace.fraction < 1.0 || trace.all_solid { origin } else { destination };
        body.velocity = velocity;
        body.ground = None;
        game.write_body(entity.clone(), &body, true);
        game.require_entity_mut(&entity).timestamp = game.now();
        game.schedule(entity, 0.1, lmctf_rune_think as Q2Think);
    }

    /// Spawn a rune (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let classname = game.require_entity(&entity).classname.clone();
        let Some(definition) = LMCTF_RUNES.iter().find(|definition| definition.classname == classname) else {
            return false;
        };
        {
            let record = game.require_entity_mut(&entity);
            record.model = definition.model.to_string();
            record.count = definition.bit;
            if definition.kind == LmctfRune::Vampire {
                record.effects |= 0x400;
                record.render_flags |= 1024;
            }
        }
        game.source_callbacks.register(&lmctf_rune_callbacks());
        self.toss(entity, game);
        true
    }

    /// Run post-spawn setup (`postSpawn`).
    pub fn post_spawn(&self, game: &mut Q2GameServices) {
        for definition in LMCTF_RUNES {
            if game.lmctf.rules.runes & definition.bit == 0 {
                continue;
            }
            let spot = self.farthest_spot(game).or_else(|| LmctfFlags { hooks: super::lmctf_hooks(game) }.flag(game, 1));
            let Some(spot) = spot else {
                continue;
            };
            let rune = game.create(definition.classname, BTreeMap::new());
            let origin = game.body_of(spot).origin;
            let mut body = game.body_of(rune.clone());
            body.origin = origin;
            game.write_body(rune.clone(), &body, true);
            self.spawn(rune, game);
        }
    }

    /// Pick up a rune (`pickup`).
    pub fn pickup(&self, entity: ActorId, game: &mut Q2GameServices, actor: ActorId) -> bool {
        if !game.lmctf.states.contains_key(&actor) || game.entity(&actor).is_none() {
            return false;
        }
        let hooks = super::lmctf_hooks(game);
        let spectator = match (hooks.player)(actor.clone(), game) {
            Some(player) => player.spectator,
            None => return false,
        };
        let health = game.host.combat().read(&actor).map(|combat| combat.health).unwrap_or(0.0);
        if spectator || health <= 0.0 || game.require_entity(&entity).solid != Q2Solid::Trigger {
            return false;
        }
        if game.lmctf.states.get(&actor).and_then(|state| state.rune.clone()).is_some() {
            game.require_entity_mut(&entity).touch = Some(lmctf_rune_touch as Q2Touch);
            game.schedule(entity, 0.1, lmctf_rune_think as Q2Think);
            return false;
        }
        let definition = self.definition(&game.require_entity(&entity).classname.clone());
        let owned = game.owned_of(actor.clone());
        if !game.host.inventory().entries(&actor).iter().any(|entry| entry.item == definition.item) {
            game.host.inventory().configure(
                &owned,
                &InventoryEntry { item: item_id(definition.item), count: 0.0, capacity: 1.0, count_policy: None },
            );
        }
        game.host.inventory().give(&owned, &item_id(definition.item), 1.0);
        if let Some(state) = game.lmctf.states.get_mut(&actor) {
            state.rune = Some(entity.clone());
        }
        {
            let record = game.require_entity_mut(&entity);
            record.owner = Some(actor.clone());
            record.visible = false;
            record.server_flags |= 1;
        }
        game.cancel_actor(entity.clone());
        game.set_solid(entity.clone(), Q2Solid::None);
        game.set_motion_kind(entity.clone(), Q2MotionKind::Stationary);
        let mut body = game.body_of(entity.clone());
        body.velocity = Vec3::default();
        game.write_body(entity.clone(), &body, true);
        game.show(entity.clone());
        game.sound(&entity, "misc/power1.wav", 3, 1.0, 1.0);
        game.host_emit(Q2PresentationEvent::Pickup {
            player: actor,
            item: item_id(definition.item),
            icon: definition.icon.to_string(),
            name: definition.name.to_string(),
        });
        true
    }

    /// Drop the held rune (`drop`).
    pub fn drop(&self, actor: &ActorId, game: &mut Q2GameServices) -> bool {
        let rune = game.lmctf.states.get(actor).and_then(|state| state.rune.clone());
        let (Some(rune), Some(_)) = (rune, game.entity(actor)) else {
            return false;
        };
        if game.entity(&rune).is_none() {
            return false;
        }
        let owned = game.owned_of(actor.clone());
        let item = self.definition(&game.require_entity(&rune).classname.clone()).item;
        game.host.inventory().consume(&owned, &item_id(item), 1.0);
        if let Some(state) = game.lmctf.states.get_mut(actor) {
            state.rune = None;
        }
        self.visible(rune.clone(), game);
        {
            let record = game.require_entity_mut(&rune);
            record.owner = Some(actor.clone());
            record.touch = Some(lmctf_rune_drop_touch as Q2Touch);
        }
        // The source computes random velocity before ctf_TossEnt overwrites it.
        game.random();
        game.random();
        game.random();
        let angles = game.host.player_view_state(actor).map(|view| view.view_angles).unwrap_or_else(|| game.body_of(actor.clone()).angles);
        lmctf_toss(rune.clone(), actor.clone(), game, angle_vectors(angles).forward);
        game.require_entity_mut(&rune).timestamp = game.now();
        game.schedule(rune, 1.0, lmctf_drop_rune_think as Q2Think);
        game.sound(actor, "misc/power2.wav", 3, 1.0, 1.0);
        true
    }

    /// Apply rune damage (`damage`).
    pub fn damage(&self, actor: Option<ActorId>, amount: f64, game: &mut Q2GameServices) -> f64 {
        match actor {
            Some(actor) if self.held(&actor, game) == Some(LmctfRune::Damage) => fround(amount * 1.75).trunc(),
            _ => amount,
        }
    }

    /// Apply post-armor resistance (`afterPowerArmor`).
    pub fn after_power_armor(&self, actor: ActorId, take: f64, game: &mut Q2GameServices) -> f64 {
        if self.held(&actor, game) != Some(LmctfRune::Resist) {
            return take;
        }
        if game.entity(&actor).is_some() {
            game.sound(&actor, "ctf/resist.wav", 3, 1.0, 1.0);
        }
        fround(take / 1.75).trunc()
    }

    /// Apply post-health vampire drain (`afterHealth`).
    pub fn after_health(&self, target: ActorId, attacker: Option<ActorId>, take: f64, game: &mut Q2GameServices) {
        let Some(source) = attacker else {
            return;
        };
        if source == target || take == 0.0 || self.held(&source, game) != Some(LmctfRune::Vampire) {
            return;
        }
        let victim_bodyque = game.entity(&target).map(|entity| entity.classname == "bodyque").unwrap_or(false);
        let health = game.host.combat().read(&source).map(|combat| combat.health);
        let Some(health) = health else {
            return;
        };
        if game.entity(&source).is_none() {
            return;
        }
        let shift = if game.host.is_player(&target) {
            1
        } else if victim_bodyque {
            2
        } else {
            0
        };
        if shift == 0 {
            return;
        }
        let owned = game.owned_of(source.clone());
        game.host.combat().set_health(&owned, 250.0f64.min(health + f64::from(take.trunc() as i32 >> shift)));
        game.sound(&source, "brain/brnatck3.wav", 3, 1.0, 1.0);
    }

    /// Run the player frame (`playerFrame`).
    pub fn player_frame(&self, entity: ActorId, game: &mut Q2GameServices) {
        if self.held(&entity, game) != Some(LmctfRune::Regen) {
            return;
        }
        let Some(combat) = game.host.combat().read(&entity) else {
            return;
        };
        let heart_rate = 25.min(5.max((combat.health / 5.0).trunc() as i32));
        let frame = (game.now() * 10.0).round() as i32;
        if frame < lmctf_player(game, &entity).regen_frame + heart_rate {
            return;
        }
        lmctf_player(game, &entity).regen_frame = frame;
        let mut sound = false;
        let max_health = game.require_entity(&entity).max_health;
        if combat.health < max_health + 25.0 {
            let owned = game.owned_of(entity.clone());
            game.host.combat().set_health(&owned, (max_health + 25.0).min((combat.health + fround(f64::from(heart_rate) / 3.0)).trunc()));
            sound = true;
        }
        let points = match &combat.armor.regular {
            RegularArmorState::None => None,
            RegularArmorState::Q1 { points, .. }
            | RegularArmorState::Q2 { points, .. }
            | RegularArmorState::Q3 { points, .. }
            | RegularArmorState::Source { points, .. } => Some(*points),
        };
        match points {
            None | Some(0.0) => {
                let owned = game.owned_of(entity.clone());
                game.host.combat().set_regular_armor(
                    &owned,
                    &RegularArmorState::Q2 {
                        points: (f64::from(heart_rate) / 4.0).trunc(),
                        normal_protection: 0.3,
                        energy_protection: 0.0,
                        item: item_id("q2:item_armor_jacket"),
                    },
                );
                sound = true;
            }
            Some(points) if points < 200.0 => {
                let owned = game.owned_of(entity.clone());
                game.host.combat().set_regular_points(&owned, 200.0f64.min((points + fround(f64::from(heart_rate) / 3.0)).trunc()), None);
                sound = true;
            }
            _ => {}
        }
        if sound {
            game.sound(&entity, "ctf/regen.wav", 3, 1.0, 1.0);
        }
    }

    /// Run the weapon frame (`weaponFrame`), reporting whether to repeat.
    pub fn weapon_frame(&self, entity: ActorId, game: &mut Q2GameServices, firing: bool) -> bool {
        match self.held(&entity, game) {
            Some(LmctfRune::Haste) => {
                if firing {
                    game.sound(&entity, "player/lava1.wav", 3, 1.0, 1.0);
                }
                game.weapons.states.get(&entity).map(|state| state.frame).unwrap_or(0) != 0
            }
            Some(LmctfRune::Damage) if firing => {
                game.sound(&entity, "ctf/strength.wav", 3, 1.0, 1.0);
                false
            }
            _ => false,
        }
    }
}
