//! Rogue deathmatch runes (`src/content/q1/missionpacks/world/rogue-runes.ts`).
//!
//! runes.qc rune lifecycle and effects.
//!
//! The donor keeps per-player rune state in a side table with an explicit
//! state extension and release hook. Here the same state lives in player
//! entity fields, which the foundation checkpoint already captures and
//! drops with the entity, so no extension or release hook is needed.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::Q1WeaponRules;
use crate::q1::foundation::gameplay::{BodyPatch, Q1DamageSourceEffects, TouchSurface};
use crate::q1::foundation::types::{Q1Event, Q1MoveType, Q1Solid, Q1SoundChannel, Q1Weapon};
use crate::q1::missionpacks::types::fround;
use crate::q1::{q1_error, Q1Error};

use super::common::{later, number};
use super::with_missionpack_hooks;

/// Rune bit fields on player entities.
const RUNE_BITS: &str = "rogue:rune";
/// Notice cooldown field.
const RUNE_NOTICE: &str = "rogue:rune_notice";
/// Resistance noise cooldown field.
const RUNE_EARTH_NOISE: &str = "rogue:rune_earth_noise";
/// Strength noise cooldown field.
const RUNE_BLACK_NOISE: &str = "rogue:rune_black_noise";
/// Haste noise cooldown field.
const RUNE_HELL_NOISE: &str = "rogue:rune_hell_noise";
/// Regeneration tick field.
const RUNE_REGEN: &str = "rogue:rune_regen";

/// Require a live rune carrier (`state` liveness check).
fn require_live(game: &Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    if game.host.actors.resolve_owned(actor).is_none() {
        return Err(q1_error("Rune operation needs a live actor"));
    }
    Ok(())
}

/// Read a player's rune bits.
fn rune_bits(game: &Q1EntityServices, actor: &ActorId) -> i32 {
    game.entity(actor)
        .map(|entity| entity.number(RUNE_BITS) as i32)
        .unwrap_or(0)
}

/// Pick the next round-robin deathmatch rune spawn (`spawnPoint`).
fn spawn_point(game: &mut Q1EntityServices) -> Result<Vec3, Q1Error> {
    let world = game
        .world
        .clone()
        .ok_or_else(|| q1_error("Rune spawn requires worldspawn"))?;
    let points: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity(id)
                .is_some_and(|entity| entity.classname == "info_player_deathmatch")
                && game.is_live(id)
        })
        .collect();
    if points.is_empty() {
        return Err(q1_error("Rogue runes require an info_player_deathmatch spawn"));
    }
    let previous = game
        .entity(&world)
        .and_then(|world| world.references.get("rogue:rune_spawn_spot").cloned().flatten());
    let index = previous
        .as_ref()
        .and_then(|previous| points.iter().position(|point| point == previous));
    let next = points[(index.map(|index| index + 1).unwrap_or(0)) % points.len()].clone();
    game.update_entity(&world, |world| {
        world
            .references
            .insert("rogue:rune_spawn_spot".to_string(), Some(next.clone()));
    })?;
    Ok(game.body(&next)?.origin)
}

/// Roll a rune scatter velocity (`velocity`).
fn scatter_velocity(game: &mut Q1EntityServices) -> Vec3 {
    Vec3 {
        x: (-300.0 + game.host.random() * 600.0) as f32,
        y: (-300.0 + game.host.random() * 600.0) as f32,
        z: 300.0,
    }
}

/// Spawn one rune item (`spawn`).
fn spawn_rune(game: &mut Q1EntityServices, rune: i32, origin: Vec3) -> Result<ActorId, Q1Error> {
    let item = game.create("rogue_rune", None, None)?;
    let model = if rune & 1 != 0 {
        "progs/end1.mdl"
    } else if rune & 2 != 0 {
        "progs/end2.mdl"
    } else if rune & 4 != 0 {
        "progs/end3.mdl"
    } else {
        "progs/end4.mdl"
    };
    game.update_entity(&item, |item| {
        number(item, "rune", f64::from(rune));
        item.movement_flags = 256;
        item.solid = Q1Solid::Trigger;
        item.movement = Q1MoveType::Toss;
        item.model = model.to_string();
    })?;
    let velocity = scatter_velocity(game);
    game.set_body(
        &item,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            bounds: Some(qa_core::math::Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: 0.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 56.0,
                },
            }),
            ..Default::default()
        },
    )?;
    let touch_name = game.named.touch("rogue:rune_touch")?;
    game.update_entity(&item, |item| item.touch = Some(touch_name))?;
    later(game, &item, 120.0, "rogue:rune_respawn")?;
    game.link(&item)?;
    Ok(item)
}

/// Play a rune effect noise (`noise`).
fn rune_noise(game: &mut Q1EntityServices, actor: &ActorId, rune: i32) -> Result<(), Q1Error> {
    if game.host.actors.resolve_owned(actor).is_none() {
        return Ok(());
    }
    game.sound(actor, &format!("runes/end{rune}.wav"), Q1SoundChannel::Item, 1.0, 1.0)
}

/// Apply the strength rune to outgoing damage.
fn rune_damage_amount(game: &mut Q1EntityServices, actor: &ActorId, amount: f64) -> Result<f64, Q1Error> {
    require_live(game, actor)?;
    if rune_bits(game, actor) & 2 != 0 {
        return Ok(fround(amount * 2.0));
    }
    Ok(amount)
}

/// Apply the resistance rune to incoming damage.
fn rune_resistance_amount(game: &mut Q1EntityServices, actor: &ActorId, amount: f64) -> Result<f64, Q1Error> {
    require_live(game, actor)?;
    if rune_bits(game, actor) & 1 == 0 {
        return Ok(amount);
    }
    if game
        .entity(actor)
        .map(|entity| entity.number(RUNE_EARTH_NOISE))
        .unwrap_or(0.0)
        < game.time
    {
        rune_noise(game, actor, 1)?;
        let time = game.time;
        game.update_entity(actor, |entity| number(entity, RUNE_EARTH_NOISE, time + 1.0))?;
    }
    Ok(fround(amount / 2.0))
}

/// Play the strength rune attack sound.
fn rune_attack_sound_inner(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    require_live(game, actor)?;
    if rune_bits(game, actor) & 2 != 0
        && game
            .entity(actor)
            .map(|entity| entity.number(RUNE_BLACK_NOISE))
            .unwrap_or(0.0)
            < game.time
    {
        rune_noise(game, actor, 2)?;
        let time = game.time;
        game.update_entity(actor, |entity| number(entity, RUNE_BLACK_NOISE, time + 1.0))?;
    }
    Ok(())
}

/// Apply the haste rune to an attack delay.
fn rune_attack_delay_inner(game: &mut Q1EntityServices, actor: &ActorId, delay: f64) -> Result<f64, Q1Error> {
    require_live(game, actor)?;
    if rune_bits(game, actor) & 4 == 0 {
        return Ok(delay);
    }
    if game
        .entity(actor)
        .map(|entity| entity.number(RUNE_HELL_NOISE))
        .unwrap_or(0.0)
        < game.time
    {
        rune_noise(game, actor, 3)?;
        let time = game.time;
        game.update_entity(actor, |entity| number(entity, RUNE_HELL_NOISE, time + 1.0))?;
    }
    Ok(fround(fround(delay * 2.0) / 3.0))
}

/// Weapon pre-fire hook: strength rune sound.
fn rune_before_fire(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    rune_attack_sound_inner(game, player)
}

/// Weapon attack-delay hook: haste rune on applicable weapons.
fn rune_weapon_attack_delay(game: &mut Q1EntityServices, player: &ActorId, delay: f64) -> Result<f64, Q1Error> {
    let hasted = game.player_ref(player).is_some_and(|state| {
        matches!(
            state.weapon,
            Q1Weapon::Axe
                | Q1Weapon::Shotgun
                | Q1Weapon::Supershotgun
                | Q1Weapon::Grenadelauncher
                | Q1Weapon::Rocketlauncher
                | Q1Weapon::RogueMultiGrenade
                | Q1Weapon::RogueMultiRocket
                | Q1Weapon::RoguePlasma
        )
    });
    if hasted {
        return rune_attack_delay_inner(game, player, delay);
    }
    Ok(delay)
}

/// Respawn a dropped rune at the next spawn point.
fn rune_respawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let origin = spawn_point(game)?;
    let velocity = scatter_velocity(game);
    game.set_body(
        id,
        &BodyPatch {
            origin: Some(origin),
            velocity: Some(velocity),
            ..Default::default()
        },
    )?;
    game.link(id)?;
    later(game, id, 120.0, "rogue:rune_respawn")
}

/// Pick up a rune.
fn rune_touch(
    game: &mut Q1EntityServices,
    id: &ActorId,
    other: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    if !game.is_player(other) || game.health(other) <= 0.0 {
        return Ok(());
    }
    require_live(game, other)?;
    if rune_bits(game, other) != 0 {
        if game
            .entity(other)
            .map(|entity| entity.number(RUNE_NOTICE))
            .unwrap_or(0.0)
            < game.time
        {
            game.host.emit(Q1Event::Message {
                player: other.clone(),
                text: "$qc_already_have_rune".to_string(),
                center: true,
                args: None,
                parts: None,
            });
        }
        let time = game.time;
        return game.update_entity(other, |entity| number(entity, RUNE_NOTICE, time + 5.0));
    }
    let rune = game.entity(id).map(|entity| entity.number("rune")).unwrap_or(0.0);
    game.update_entity(other, |entity| {
        let bits = entity.number(RUNE_BITS) as i32 | rune as i32;
        number(entity, RUNE_BITS, f64::from(bits));
    })?;
    if game.host.actors.resolve_owned(other).is_none() {
        return Ok(());
    }
    game.sound(other, "weapons/pkup.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    let bits = rune_bits(game, other);
    game.host.emit(Q1Event::Message {
        player: other.clone(),
        text: if bits & 1 != 0 {
            "$qc_rune_resistance"
        } else if bits & 2 != 0 {
            "$qc_rune_strength"
        } else if bits & 4 != 0 {
            "$qc_rune_haste"
        } else {
            "$qc_rune_regeneration"
        }
        .to_string(),
        center: true,
        args: None,
        parts: None,
    });
    game.remove(id)
}

/// Spawn the opening rune set.
fn rune_spawn(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    game.remove(id)?;
    for rune in [1, 2, 4, 8] {
        let origin = spawn_point(game)?;
        spawn_rune(game, rune, origin)?;
    }
    Ok(())
}

/// Inert rune damage effects. The donor `afterQuad`/`afterArmor` logic
/// lives in the gameful [`RogueRunes::after_quad`]/[`RogueRunes::after_armor`],
/// which the session damage pipeline calls with game access.
fn rune_damage_effects() -> Q1DamageSourceEffects {
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

/// Rogue rune services (`RogueRunes`).
pub struct RogueRunes;

impl RogueRunes {
    /// Register rune entities and rules (`constructor`).
    pub fn new(game: &mut Q1EntityServices) -> Result<Self, Q1Error> {
        game.named.register(
            "rogue:rune_respawn",
            Q1CallbackHandlers {
                action: Some(rune_respawn),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:rune_touch",
            Q1CallbackHandlers {
                touch: Some(rune_touch),
                ..Default::default()
            },
        )?;
        game.named.register(
            "rogue:rune_spawn",
            Q1CallbackHandlers {
                action: Some(rune_spawn),
                ..Default::default()
            },
        )?;
        game.register_damage_source_effects("rogue:runes", rune_damage_effects())?;
        game.register_weapon_rules(Q1WeaponRules {
            id: "rogue:runes".to_string(),
            before_fire: Some(rune_before_fire),
            attack_delay: Some(rune_weapon_attack_delay),
            ..Default::default()
        })?;
        Ok(Self)
    }

    /// Spawn opening runes and tick regeneration (`frame`).
    pub fn frame(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        let gamecfg = with_missionpack_hooks(game, |_, hooks| Ok(hooks.gamecfg.as_ref().map(|gamecfg| gamecfg())))?;
        if game.options().deathmatch != 0 && gamecfg.unwrap_or(0) & 1 != 0 {
            let world = game.world.clone();
            if let Some(world) = world {
                if game
                    .entity(&world)
                    .map(|world| world.number("rogue:runes_spawned"))
                    .unwrap_or(0.0)
                    == 0.0
                {
                    game.update_entity(&world, |world| number(world, "rogue:runes_spawned", 1.0))?;
                    let spawner = game.create("rogue_rune_spawner", None, None)?;
                    later(game, &spawner, 0.1, "rogue:rune_spawn")?;
                }
            }
        }
        require_live(game, actor)?;
        if rune_bits(game, actor) & 8 == 0
            || game
                .entity(actor)
                .map(|entity| entity.number(RUNE_REGEN))
                .unwrap_or(0.0)
                >= game.time
            || game.health(actor) >= 100.0
        {
            return Ok(());
        }
        if game.host.actors.resolve_owned(actor).is_none() {
            return Ok(());
        }
        game.sound(actor, "runes/end4.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
        let health = (game.health(actor) + 5.0).min(100.0);
        game.set_health(actor, health)?;
        let time = game.time;
        game.update_entity(actor, |entity| number(entity, RUNE_REGEN, time + 1.0))
    }

    /// Drop carried runes on death (`drop`).
    pub fn drop(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        require_live(game, actor)?;
        let body = game.host.bodies.read(actor);
        let Some(body) = body else {
            return Ok(());
        };
        let bits = rune_bits(game, actor);
        for rune in [1, 2, 4, 8] {
            if bits & rune != 0 {
                spawn_rune(game, rune, body.origin)?;
            }
        }
        game.update_entity(actor, |entity| number(entity, RUNE_BITS, 0.0))
    }

    /// Apply the strength rune to outgoing damage (`damage`).
    pub fn damage(&self, game: &mut Q1EntityServices, actor: &ActorId, amount: f64) -> Result<f64, Q1Error> {
        rune_damage_amount(game, actor, amount)
    }

    /// Apply the resistance rune to incoming damage (`resistance`).
    pub fn resistance(&self, game: &mut Q1EntityServices, actor: &ActorId, amount: f64) -> Result<f64, Q1Error> {
        rune_resistance_amount(game, actor, amount)
    }

    /// Play the strength rune attack sound (`attackSound`).
    pub fn attack_sound(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        rune_attack_sound_inner(game, actor)
    }

    /// Apply the haste rune to an attack delay (`attackDelay`).
    pub fn attack_delay(&self, game: &mut Q1EntityServices, actor: &ActorId, delay: f64) -> Result<f64, Q1Error> {
        rune_attack_delay_inner(game, actor, delay)
    }

    /// Whether an actor carries the regeneration rune (`hasRegeneration`).
    pub fn has_regeneration(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
        require_live(game, actor)?;
        Ok(rune_bits(game, actor) & 8 != 0)
    }

    /// Gameful `afterQuad` stage for the session damage pipeline.
    pub fn after_quad(
        &self,
        game: &mut Q1EntityServices,
        attacker: Option<&ActorId>,
        amount: f64,
    ) -> Result<f64, Q1Error> {
        if game.options().deathmatch != 0 {
            if let Some(attacker) = attacker {
                return self.damage(game, attacker, amount);
            }
        }
        Ok(amount)
    }

    /// Gameful `afterArmor` stage for the session damage pipeline.
    pub fn after_armor(&self, game: &mut Q1EntityServices, target: &ActorId, take: f64) -> Result<f64, Q1Error> {
        if game.options().deathmatch != 0 {
            return self.resistance(game, target, take);
        }
        Ok(take)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::missionpacks::types::test_game;

    fn carrier(game: &mut Q1EntityServices, bits: i32) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        game.update_entity(&player, |entity| number(entity, RUNE_BITS, f64::from(bits)))
            .expect("rune");
        game.set_health(&player, 100.0).expect("health");
        player
    }

    #[test]
    fn strength_doubles_and_haste_shortens() {
        let mut game = test_game();
        let runes = RogueRunes::new(&mut game).expect("runes");
        let strong = carrier(&mut game, 2);
        assert_eq!(runes.damage(&mut game, &strong, 10.0).expect("damage"), 20.0);
        let hasted = carrier(&mut game, 4);
        assert_eq!(
            runes.attack_delay(&mut game, &hasted, 0.9).expect("delay"),
            fround(fround(1.8) / 3.0)
        );
        let plain = carrier(&mut game, 0);
        assert_eq!(runes.damage(&mut game, &plain, 10.0).expect("plain"), 10.0);
        assert!(!runes.has_regeneration(&mut game, &plain).expect("regen"));
        let regen = carrier(&mut game, 8);
        assert!(runes.has_regeneration(&mut game, &regen).expect("regen"));
    }

    #[test]
    fn resistance_halves_with_cooldown_noise() {
        let mut game = test_game();
        let runes = RogueRunes::new(&mut game).expect("runes");
        let tank = carrier(&mut game, 1);
        game.time = 0.5;
        assert_eq!(runes.resistance(&mut game, &tank, 10.0).expect("resist"), 5.0);
        assert_eq!(
            game.entity(&tank).expect("tank").number(RUNE_EARTH_NOISE),
            game.time + 1.0
        );
    }

    #[test]
    fn rune_touch_claims_and_removes_item() {
        let mut game = test_game();
        let runes = RogueRunes::new(&mut game).expect("runes");
        let player = carrier(&mut game, 0);
        let watch = player.clone();
        game.host.players = Box::new(move || vec![watch.clone()]);
        let item = spawn_rune(&mut game, 2, Vec3 { x: 0.0, y: 0.0, z: 0.0 }).expect("item");
        game.invoke_touch(&item, &player, None, None).expect("touch");
        assert_eq!(rune_bits(&game, &player), 2);
        assert!(game.entity(&item).is_none());
        runes.drop(&mut game, &player).expect("drop");
        assert_eq!(rune_bits(&game, &player), 0);
        let items = game
            .entity_ids()
            .into_iter()
            .filter(|id| game.entity(id).is_some_and(|entity| entity.classname == "rogue_rune"));
        assert_eq!(items.count(), 1);
    }
}
