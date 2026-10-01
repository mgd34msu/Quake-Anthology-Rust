//! Q2 LMCTF weapons (`src/content/q2/multiplayer/lmctf/weapons.ts`).
//!
//! LMCTF plasma.c / p_weapon.c. Copyright Team HOSTILE and LMCTF.
//! GPL-2.0-or-later.

use std::collections::BTreeMap;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3, add3, scale3, vec3};

use crate::contract::{InventoryCountPolicy, SourceCounterArithmetic};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2EffectEvent, Q2GameServices, Q2MotionKind, Q2PresentationEvent, Q2Solid, Q2Think, Q2Touch};
use crate::q2::foundation::items::{Q2ItemDefinition, Q2ItemKindData};
use crate::q2::foundation::weapons::ballistics::{NoiseKind, weapon_player_noise};
use crate::q2::foundation::weapons::player::{
    Q2WeaponContext, Q2WeaponExtension, Q2WeaponSelectionRule, register_weapon_extension, weapon_ammo,
    weapon_ammo_changed, weapon_generic_classic, weapon_kick, weapon_no_ammo, weapon_project,
};
use crate::q2::foundation::weapons::presentation::q2_weapon_recoil;
use crate::q2::foundation::weapons::types::{Q2WeaponDefinition, Q2WeaponOwner, Q2WeaponPhase, Q2WeaponState};
use crate::q2::foundation::weapons::vectors::{angle_vectors, vector_angles};
use crate::q2::support::contracts::TouchContact;

use super::super::ctf::types::item_id;
use super::types::{LmctfHooks, lmctf_player, lmctf_print};

/// LMCTF plasma item.
pub const LMCTF_PLASMA_ITEM: &str = "q2:weapon_plasma";

/// LMCTF plasma definition (`plasma`).
pub fn lmctf_plasma() -> Q2WeaponDefinition {
    Q2WeaponDefinition {
        name: "lmctf:plasma".to_string(),
        item: LMCTF_PLASMA_ITEM.to_string(),
        classname: "weapon_plasma".to_string(),
        ammo: Some("q2:ammo_cells".to_string()),
        quantity: 10,
        warning: 10,
        view_model: "models/weapons/v_plasma/tris.md2".to_string(),
        world_model: "models/weapons/g_plasma/tris.md2".to_string(),
        player_model: 12,
        activate_last: 3,
        fire_last: 11,
        idle_last: 46,
        deactivate_last: 51,
        pauses: vec![16, 46],
        fires: vec![4, 5],
        repeating: false,
    }
}

/// LMCTF weapons (`LmctfWeapons`).
#[derive(Debug, Clone, Copy)]
pub struct LmctfWeapons {
    /// Session hooks.
    pub hooks: LmctfHooks,
}

/// Plasma callbacks (`LmctfWeapons::callbacks`).
pub fn lmctf_weapon_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("lmctf:plasma_free", lmctf_plasma_free as Q2Think);
    callbacks.touch.insert("lmctf:plasma_reflect_touch", lmctf_plasma_reflect as Q2Touch);
    callbacks.touch.insert("lmctf:plasma_spread_touch", lmctf_plasma_spread as Q2Touch);
    callbacks
}

/// Plasma free think (`free`).
fn lmctf_plasma_free(entity: ActorId, game: &mut Q2GameServices) {
    game.remove_actor(entity);
}

/// Plasma reflect touch (`reflect`).
fn lmctf_plasma_reflect(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let weapons = LmctfWeapons { hooks: super::lmctf_hooks(game) };
    weapons.impact(entity, game, contact, true);
}

/// Plasma spread touch (`spread`).
fn lmctf_plasma_spread(entity: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let weapons = LmctfWeapons { hooks: super::lmctf_hooks(game) };
    weapons.impact(entity, game, contact, false);
}

/// Plasma selection (`selection` choose).
fn lmctf_plasma_choose(owner: &Q2WeaponOwner, game: &mut Q2GameServices, state: &Q2WeaponState) -> bool {
    if state.weapon.as_deref() == Some("lmctf:plasma") {
        let id = owner.actor.id().clone();
        let mode = {
            let player = lmctf_player(game, &id);
            player.plasma_mode = !player.plasma_mode;
            player.plasma_mode
        };
        lmctf_print(game, if mode { "bounce plasma\n" } else { "spread plasma\n" }, Some(id));
    }
    true
}

impl LmctfWeapons {
    /// Register the plasma weapon and item (`register`).
    pub fn register(&self, game: &mut Q2GameServices) {
        let definition = lmctf_plasma();
        self.hooks.items.register_item(
            game,
            Q2ItemDefinition {
                classname: definition.classname.clone(),
                model: definition.world_model.clone(),
                icon: "w_plasma".to_string(),
                name: "Plasma Rifle".to_string(),
                sound: "misc/w_pkup.wav".to_string(),
                rotate: true,
                respawn: 30.0,
                console_give: None,
                kind: Q2ItemKindData::Weapon { ammo: definition.ammo.clone(), coop_stay: Some(true) },
            },
        );
        register_weapon_extension(game, Box::new(LmctfPlasmaExtension { hooks: self.hooks, definition }));
    }

    /// Print the plasma mode (`mode`).
    fn mode(&self, owner: ActorId, game: &mut Q2GameServices) {
        let mode = lmctf_player(game, &owner).plasma_mode;
        lmctf_print(game, if mode { "bounce plasma\n" } else { "spread plasma\n" }, Some(owner));
    }

    /// Launch plasma (`launch`).
    pub fn launch(&self, owner: ActorId, game: &mut Q2GameServices, start: Vec3, direction: Vec3, reflect: bool) {
        let angles = vector_angles(direction);
        let yaws: &[i32] = if reflect { &[0] } else { &[0, 10, -10] };
        for yaw in yaws {
            let yaw = *yaw;
            let goop = game.create("goop", BTreeMap::new());
            let facing = if yaw == 0 {
                direction
            } else {
                angle_vectors(Vec3 { x: angles.x, y: angles.y + yaw as f32, z: angles.z }).forward
            };
            let velocity = scale3(facing, 1200.0);
            {
                let record = game.require_entity_mut(&goop);
                record.owner = Some(owner.clone());
                record.clip_mask = 0x6000003;
                record.server_flags = 2;
                record.damage = if reflect { 39.0 } else { 1.0 };
                record.effects = 0x100000 | 0x2000;
                record.render_flags = 32;
                record.model = "sprites/s_plasma1.sp2".to_string();
                record.sound = "weapons/plasma/flyby.wav".to_string();
                record.touch = Some(if reflect { lmctf_plasma_reflect as Q2Touch } else { lmctf_plasma_spread as Q2Touch });
            }
            let mut body = game.body_of(goop.clone());
            body.origin = start;
            body.velocity = velocity;
            body.angles = if reflect { velocity } else { Vec3::default() };
            body.bounds = if reflect {
                Bounds { min: vec3(-12.0, -12.0, -12.0), max: vec3(12.0, 12.0, 12.0) }
            } else {
                Bounds { min: Vec3::default(), max: Vec3::default() }
            };
            game.write_body(goop.clone(), &body, true);
            game.set_solid(goop.clone(), Q2Solid::Box);
            game.set_motion_kind(goop.clone(), if reflect { Q2MotionKind::WallBounce } else { Q2MotionKind::FlyMissile });
            game.show(goop.clone());
            game.schedule(goop, if reflect { 1.5 } else { 3.0 }, lmctf_plasma_free as Q2Think);
        }
    }

    /// Impact plasma (`impact`).
    fn impact(&self, goop: ActorId, game: &mut Q2GameServices, contact: TouchContact, reflect: bool) {
        if contact.surface.as_ref().map(|surface| surface.native_flags).unwrap_or(0) & 4 != 0 {
            game.remove_actor(goop);
            return;
        }
        if !reflect && game.entity(&contact.other).map(|entity| entity.classname == "goop").unwrap_or(false) {
            return;
        }
        let body = game.body_of(goop.clone());
        let owner = game.require_entity(&goop).owner.clone();
        let normal = contact.plane.as_ref().map(|plane| plane.normal).unwrap_or_default();
        let damage = (if reflect { 39.0 } else { 28.0 }) * if game.lmctf.plasma_quad { 4.0 } else { 1.0 };
        let hurt = game.host.combat().read(&contact.other).map(|combat| combat.can_take_damage).unwrap_or(false);
        if let Some(owner) = owner.clone() {
            if game.host.is_player(&owner) {
                weapon_player_noise(game, &owner, body.origin, NoiseKind::Impact);
            }
        }
        if hurt {
            game.damage(
                contact.other.clone(),
                goop.clone(),
                owner.clone(),
                damage,
                1.0,
                body.velocity,
                body.origin,
                normal,
                34,
                4,
                Some(item_id(LMCTF_PLASMA_ITEM)),
            );
        } else {
            game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                effect: "q2:laser_sparks".to_string(),
                origin: body.origin,
                direction: normal,
                count: 32,
                color: 176,
            }));
            game.radius_damage(goop.clone(), owner.clone(), damage, None, damage + 70.0, 34, 0, Some(item_id(LMCTF_PLASMA_ITEM)));
        }
        if reflect && !hurt {
            game.sound(&goop, "weapons/plasma/bounce.wav", 4, 1.0, 3.0);
            return;
        }
        game.sound(&goop, "weapons/plasma/hit.wav", 4, 1.0, 2.0);
        game.set_solid(goop.clone(), Q2Solid::None);
        {
            let record = game.require_entity_mut(&goop);
            record.touch = None;
            record.model = "sprites/s_plasma2.sp2".to_string();
            record.frame = 0;
            record.sound = String::new();
        }
        let mut moved = game.body_of(goop.clone());
        moved.origin = add3(body.origin, scale3(body.velocity, -0.1));
        moved.velocity = Vec3::default();
        game.write_body(goop.clone(), &moved, true);
        game.show(goop.clone());
        game.schedule(goop, 0.1, lmctf_plasma_free as Q2Think);
    }
}

/// LMCTF plasma weapon extension (`register` weapon).
pub struct LmctfPlasmaExtension {
    /// Session hooks.
    pub hooks: LmctfHooks,
    /// Weapon definition.
    pub definition: Q2WeaponDefinition,
}

impl Q2WeaponExtension for LmctfPlasmaExtension {
    fn definition(&self) -> &Q2WeaponDefinition {
        &self.definition
    }

    fn selection(&self) -> Option<Q2WeaponSelectionRule> {
        Some(Q2WeaponSelectionRule { requested: "lmctf:plasma".to_string(), choose: lmctf_plasma_choose })
    }

    fn fire(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) {
        if weapon_ammo(context, game) < 1.0 {
            state.frame += 1;
            if context.now >= state.empty_sound_time {
                game.sound(context.owner.actor.id(), "weapons/plasma/empty.wav", 2, 1.0, 1.0);
                state.empty_sound_time = context.now + 1.0;
            }
            weapon_no_ammo(context, game, state, false);
            return;
        }
        if state.frame == 4 {
            let owner = context.owner.actor.id().clone();
            let (start, direction) = weapon_project(context, game, vec3(8.0, 8.0, -8.0), None);
            let reflect = lmctf_player(game, &owner).plasma_mode;
            let kick_origin = scale3(angle_vectors(context.input.angles).forward, -2.0);
            let (_, kick_angles) = q2_weapon_recoil(state, game.options.edition, context.now);
            weapon_kick(context, game, state, kick_origin, kick_angles);
            game.sound(&owner, if reflect { "weapons/plasma/fire1.wav" } else { "weapons/plasma/fire2.wav" }, 1, 1.0, 1.0);
            let weapons = LmctfWeapons { hooks: self.hooks };
            weapons.launch(owner.clone(), game, start, direction, reflect);
            let cells = game.host.inventory().entries(&owner).into_iter().find(|entry| entry.item == "q2:ammo_cells");
            let Some(cells) = cells else {
                panic!("LMCTF plasma fired without its admitted cell counter");
            };
            let mut configured = cells.clone();
            configured.count_policy = Some(InventoryCountPolicy::SourceCounter(SourceCounterArithmetic::Int32));
            let owned = context.owner.actor.clone();
            game.host.inventory().configure(&owned, &configured);
            game.host.inventory().adjust_source_counter(&owned, &cells.item, -9.0);
            if game.options.deathmatch_flags & 8192 == 0 {
                game.host.inventory().adjust_source_counter(&owned, &cells.item, -1.0);
            }
            weapon_ammo_changed(game, &owner, &cells.item);
            let roll = (game.random() * 2.0 - 1.0) * 2.0;
            if let Some(player) = (self.hooks.player)(owner.clone(), game) {
                player.damage_pitch = -2.0;
                player.damage_roll = roll;
                player.damage_time = context.now + 0.5;
            }
            weapon_player_noise(game, &owner, start, NoiseKind::Weapon);
        }
        state.frame += 1;
    }

    fn think(&mut self, context: &Q2WeaponContext, game: &mut Q2GameServices, state: &mut Q2WeaponState) -> bool {
        game.lmctf.plasma_quad = context.input.quad_until > context.now;
        let activating = state.phase == Q2WeaponPhase::Activating && state.frame == 3;
        if state.phase == Q2WeaponPhase::Ready
            && state.frame == 35
            && !context.input.attack
            && !state.latched_attack
            && state.pending.is_none()
        {
            game.sound(context.owner.actor.id(), "weapons/plasma/vent.wav", 1, 1.0, 1.0);
        }
        weapon_generic_classic(context, game, state);
        if activating && state.phase == Q2WeaponPhase::Ready {
            let weapons = LmctfWeapons { hooks: self.hooks };
            weapons.mode(context.owner.actor.id().clone(), game);
        }
        true
    }
}
