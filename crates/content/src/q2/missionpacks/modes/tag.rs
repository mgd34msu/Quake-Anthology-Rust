//! Tag mode (`src/content/q2/missionpacks/modes/tag.ts`).
//!
//! Original Rogue dm_tag.c. Shared inventory and scoring remain
//! authoritative.

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{Bounds, Vec3, add3, scale3, vec3};

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{
    Q2GameServices, Q2Mode, Q2MotionKind, Q2Solid, Q2SpawnFn, Q2Think, Q2Touch,
    Q2TraceRequest, SpawnModule,
};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData, Q2ItemModule};
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::TouchContact;

/// Tag hooks (`Q2TagHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2TagHooks {
    /// Item module.
    pub items: Q2ItemModule,
    /// Select a spawn placement.
    pub select_spawn: fn(ActorId, &mut Q2GameServices) -> (Vec3, Vec3),
    /// Farthest spawn point.
    pub farthest_spawn: fn(&mut Q2GameServices) -> Option<ActorId>,
    /// Add score.
    pub add_score: fn(ActorId, &mut Q2GameServices, f64),
}

/// Tag checkpoint (`Q2TagCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2TagCheckpoint {
    /// Token actor.
    pub token: Option<SavedActorId>,
    /// Owner actor.
    pub owner: Option<SavedActorId>,
    /// Kill count.
    pub count: i32,
}

/// Arena runtime state for tag.
#[derive(Debug, Clone)]
pub struct TagRuntime {
    /// Session hooks.
    pub hooks: Option<Q2TagHooks>,
    /// Token actor.
    pub token: Option<ActorId>,
    /// Owner actor.
    pub owner: Option<ActorId>,
    /// Kill count.
    pub count: i32,
}

impl Default for TagRuntime {
    fn default() -> Self {
        Self {
            hooks: None,
            token: None,
            owner: None,
            count: 0,
        }
    }
}

/// Tag callbacks (`Q2Tag::callbacks`).
pub fn tag_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("Tag_Respawn", tag_respawn as Q2Think);
    callbacks.think.insert("Tag_MakeTouchable", tag_make_touchable as Q2Think);
    callbacks.touch.insert("Tag_TouchItem", tag_touch_item as Q2Touch);
    callbacks
}

/// Tag mode (`Q2Tag`).
#[derive(Debug, Clone, Copy)]
pub struct Q2Tag {
    /// Session hooks.
    pub hooks: Q2TagHooks,
}

/// Session tag hooks.
pub fn tag_hooks(game: &Q2GameServices) -> Q2TagHooks {
    game.tag.hooks.expect("Q2 tag mode is not registered")
}

impl Q2Tag {
    /// Register tag and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices) -> SpawnModule {
        game.tag.hooks = Some(self.hooks);
        self.hooks.items.register_item(
            game,
            Q2ItemDefinition {
                classname: "dm_tag_token".to_string(),
                model: "models/items/tagtoken/tris.md2".to_string(),
                icon: "i_tagtoken".to_string(),
                name: "Tag Token".to_string(),
                sound: "items/pkup.wav".to_string(),
                rotate: true,
                respawn: 0.0,
                console_give: None,
                kind: Q2ItemKindData::Custom {
                    capacity: 32767.0,
                    quantity: 1.0,
                    coop_stay: false,
                    droppable: false,
                    pickup: tag_token_pickup,
                    use_item: None,
                },
            },
        );
        SpawnModule {
            spawn: tag_spawn as Q2SpawnFn,
            item_name: |_| None,
            callbacks: tag_callbacks(),
        }
    }

    /// Spawn the tag token (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        if game.require_entity(&entity).classname != "dm_tag_token" {
            return false;
        }
        if game.options.mode != Q2Mode::Deathmatch {
            game.remove_actor(entity);
            return true;
        }
        game.source_callbacks.register(&tag_callbacks());
        game.tag.token = Some(entity.clone());
        game.tag.count = 0;
        game.require_entity_mut(&entity).model = "models/items/tagtoken/tris.md2".to_string();
        game.require_entity_mut(&entity).count = 1;
        self.hooks.items.spawn_item(game, entity.clone(), "dm_tag_token");
        game.require_entity_mut(&entity).effects |= 0x20000000;
        game.show(entity);
        true
    }

    /// Place the token after spawning (`postSpawn`).
    pub fn post_spawn(&self, game: &mut Q2GameServices) {
        for entity in game.entities.values() {
            if entity.classname == "dm_tag_token" {
                return;
            }
        }
        let token = game.create("dm_tag_token", std::collections::BTreeMap::new());
        let (origin, angles) = (self.hooks.select_spawn)(token.clone(), game);
        let mut moved = game.body_of(token.clone());
        moved.origin = origin;
        moved.angles = angles;
        moved.velocity = Vec3::default();
        game.write_body(token.clone(), &moved, true);
        self.spawn(token, game);
    }

    /// Capture tag state (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2TagCheckpoint {
        let _ = self;
        Q2TagCheckpoint {
            token: game.tag.token.as_ref().map(SavedActorId::from),
            owner: game.tag.owner.as_ref().map(SavedActorId::from),
            count: game.tag.count,
        }
    }

    /// Restore tag state (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, saved: Q2TagCheckpoint) {
        let _ = self;
        game.tag.token = saved.token.map(|saved| {
            game.host.actors().resolve_saved(saved).map(|owned| owned.id().clone()).unwrap_or_else(|| game.host.actors().reference_saved(saved))
        });
        game.tag.owner = saved.owner.map(|saved| {
            game.host.actors().resolve_saved(saved).map(|owned| owned.id().clone()).unwrap_or_else(|| game.host.actors().reference_saved(saved))
        });
        game.tag.count = saved.count;
    }

    /// Token owner (`ownerActor`).
    pub fn owner_actor(&self, game: &Q2GameServices) -> Option<ActorId> {
        let _ = self;
        game.tag.owner.clone()
    }

    /// Owner effects (`effects`).
    pub fn effects(&self, actor: &ActorId, game: &Q2GameServices) -> i64 {
        let _ = self;
        if Some(actor) == game.tag.owner.as_ref() {
            0x20000000
        } else {
            0
        }
    }

    /// Owner dog tag (`dogTag`).
    pub fn dog_tag(&self, actor: &ActorId, game: &Q2GameServices) -> Option<String> {
        let _ = self;
        if Some(actor) == game.tag.owner.as_ref() {
            Some("tag3".to_string())
        } else {
            None
        }
    }

    /// Scale non-owner damage (`changeDamage`).
    pub fn change_damage(
        &self,
        target: &ActorId,
        attacker: Option<&ActorId>,
        damage: f64,
        game: &Q2GameServices,
    ) -> f64 {
        let _ = self;
        if Some(target) != game.tag.owner.as_ref() && attacker != game.tag.owner.as_ref() {
            (damage * 3.0 / 4.0).trunc()
        } else {
            damage
        }
    }

    /// Drop the token on owner death (`playerDeath`).
    pub fn player_death(&self, entity: &ActorId, game: &mut Q2GameServices) {
        if game.tag.token.is_some() && Some(entity) == game.tag.owner.as_ref() {
            self.drop(entity, game);
        }
    }

    /// Drop the token on disconnect (`disconnect`).
    pub fn disconnect(&self, entity: &ActorId, game: &mut Q2GameServices) {
        self.player_death(entity, game);
    }

    /// Score a tag kill (`score`).
    pub fn score(
        &self,
        attacker: &ActorId,
        victim: &ActorId,
        game: &mut Q2GameServices,
        mut change: f64,
        means_of_death: i32,
    ) {
        if game.tag.token.is_some() && game.tag.owner.is_some() {
            if change > 0.0 && Some(attacker) == game.tag.owner.as_ref() {
                change = 3.0;
                game.tag.count += 1;
                if game.tag.count == 5 {
                    let owned = game.owned_of(attacker.clone());
                    game.host.inventory().give(&owned, &"q2:item_quad".to_string(), 1.0);
                    self.hooks.items.use_inventory_item(&owned, "q2:item_quad", game, 30.0);
                    game.tag.count = 0;
                }
            } else if Some(victim) == game.tag.owner.as_ref() && Some(attacker) != game.tag.owner.as_ref() {
                change = 5.0;
                let means = means_of_death & !0x8000000u32 as i32;
                if [49, 53, 54, 55].contains(&means)
                    || game.host.combat().read(attacker).map(|combat| combat.health).unwrap_or(0.0) <= 0.0
                {
                    self.drop(victim, game);
                } else {
                    self.bonus(attacker, game);
                    game.tag.owner = Some(attacker.clone());
                    game.tag.count = 0;
                }
            }
        }
        (self.hooks.add_score)(attacker.clone(), game, change);
    }

    /// Grant a token bonus (`bonus`).
    fn bonus(&self, entity: &ActorId, game: &mut Q2GameServices) {
        let _ = self;
        let health = game.host.combat().read(entity).map(|combat| combat.health).unwrap_or(0.0);
        let maximum = {
            let max_health = game.require_entity(entity).max_health;
            if max_health == 0.0 { 100.0 } else { max_health }
        };
        if health < maximum {
            let owned = game.owned_of(entity.clone());
            game.host.combat().set_health(&owned, maximum.min(health + 200.0));
        }
        let armor = game.create("item_armor_body", std::collections::BTreeMap::new());
        game.require_entity_mut(&armor).spawnflags |= 0x10000;
        tag_hooks(game).items.spawn_item(game, armor.clone(), "item_armor_body");
        if game.host.actors().is_live(&armor) {
            tag_hooks(game).items.touch(armor.clone(), game, entity.clone());
        }
        if game.host.actors().is_live(&armor) {
            game.remove_actor(armor);
        }
    }

    /// Drop the token (`drop`).
    pub fn drop(&self, entity: &ActorId, game: &mut Q2GameServices) {
        let _ = self;
        game.tag.count = 0;
        game.tag.owner = None;
        let token = game.create("dm_tag_token", std::collections::BTreeMap::new());
        game.tag.token = Some(token.clone());
        game.require_entity_mut(&token).spawnflags = 0x10000;
        tag_hooks(game).items.spawn_item(game, token.clone(), "dm_tag_token");
        {
            let record = game.require_entity_mut(&token);
            record.effects = 1 | 0x20000000;
            record.render_flags = 512;
            record.owner = Some(entity.clone());
            record.touch = None;
        }
        let body = game.body_of(entity.clone());
        let forward = angle_vectors(
            game.host.player_view_state(entity).map(|state| state.view_angles).unwrap_or(body.angles),
        )
        .forward;
        let trace = game.host.trace(&Q2TraceRequest {
            start: body.origin,
            end: add3(
                add3(body.origin, scale3(forward, 24.0)),
                vec3(0.0, 0.0, -16.0),
            ),
            bounds: Some(Bounds {
                min: vec3(-15.0, -15.0, -15.0),
                max: vec3(15.0, 15.0, 15.0),
            }),
            ignore: Some(entity.clone()),
            mask: 1,
            exclude: Vec::new(),
        });
        let mut moved = game.body_of(token.clone());
        moved.origin = trace.end;
        moved.bounds.min = vec3(-15.0, -15.0, -15.0);
        moved.bounds.max = vec3(15.0, 15.0, 15.0);
        moved.velocity = vec3(scale3(forward, 100.0).x, scale3(forward, 100.0).y, 300.0);
        game.write_body(token.clone(), &moved, true);
        game.set_solid(token.clone(), Q2Solid::Trigger);
        game.set_motion_kind(token.clone(), Q2MotionKind::Toss);
        game.show(token.clone());
        game.schedule(token, 1.0, tag_make_touchable as Q2Think);
        let owned = game.owned_of(entity.clone());
        game.host.inventory().consume(&owned, &"q2:dm_tag_token".to_string(), 1.0);
    }
}

/// Tag spawn entry.
fn tag_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    Q2Tag { hooks: tag_hooks(game) }.spawn(entity, game)
}

/// Pick up the tag token.
fn tag_token_pickup(entity: ActorId, game: &mut Q2GameServices, player: OwnedActor) -> bool {
    game.tag.token = Some(entity.clone());
    game.tag.owner = Some(player.id().clone());
    game.tag.count = 0;
    game.host.inventory().give(&player, &"q2:dm_tag_token".to_string(), 1.0);
    if game.entity(player.id()).is_some() {
        Q2Tag { hooks: tag_hooks(game) }.bonus(player.id(), game);
    }
    true
}

/// Tag token touch (`touch`).
fn tag_touch_item(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    tag_hooks(game).items.touch(entity, game, contact.other);
}

/// Tag respawn (`respawn`).
fn tag_respawn(entity: ActorId, game: &mut Q2GameServices) {
    match (tag_hooks(game).farthest_spawn)(game) {
        None => {
            game.schedule(entity, 1.0, tag_respawn as Q2Think);
        }
        Some(spot) => {
            let mut moved = game.body_of(entity.clone());
            moved.origin = game.body_of(spot).origin;
            game.write_body(entity, &moved, true);
        }
    }
}

/// Tag make touchable (`makeTouchable`).
fn tag_make_touchable(entity: ActorId, game: &mut Q2GameServices) {
    game.require_entity_mut(&entity).touch = Some(tag_touch_item as Q2Touch);
    let token = game.tag.token.clone();
    let Some(token) = token.as_ref().and_then(|token| game.entity(token).map(|entity| entity.actor.id().clone())) else {
        return;
    };
    let origin = game.body_of(entity).origin;
    let delay = if game.host.point_contents(origin) & 24 != 0 { 3.0 } else { 30.0 };
    game.schedule(token, delay, tag_respawn as Q2Think);
}
