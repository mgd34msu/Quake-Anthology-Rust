//! Q1 CTF rune items and effects (src/content/q1/addons/ctf/runes.ts).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::contract::RegularArmorState;
use crate::q1::addons::context::{fround, set_addon_number};
use crate::q1::addons::ctf::state::{
    ctf_body, ctf_by_key, ctf_grant, ctf_key, ctf_number, ctf_rune, ctf_set, ctf_start_map, ctf_update, ctf_world,
    rune_item, with_ctf_services, CtfDeferred,
};
use crate::q1::addons::ctf::types::{CtfRune, CTF_RUNES};
use crate::q1::foundation::callbacks::{Q1ActionHandler, Q1CallbackHandlers, Q1TouchHandler};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::gameplay::{DamagePreparation, Q1DamageSourceEffects, TouchSurface};
use crate::q1::foundation::types::{vadd, Q1Effect, Q1MoveType, Q1Solid, Q1SoundChannel};
use crate::q1::{q1_error, Q1Error};

/// Haste attack intervals by weapon item (`CTF_HASTE_INTERVALS`).
pub const CTF_HASTE_INTERVALS: [(&str, f64); 5] = [
    ("q1:weapon/axe", 0.3),
    ("q1:weapon/shotgun", 0.3),
    ("q1:weapon/supershotgun", 0.4),
    ("q1:weapon/grenadelauncher", 0.3),
    ("q1:weapon/rocketlauncher", 0.4),
];

/// Doubled nail velocity under haste (`CTF_HASTE_NAIL_SPEED`).
pub const CTF_HASTE_NAIL_SPEED: f64 = 2000.0;

/// Look up a haste attack interval by weapon item.
#[must_use]
pub fn ctf_haste_interval(item: &str) -> Option<f64> {
    CTF_HASTE_INTERVALS
        .iter()
        .find(|(candidate, _)| *candidate == item)
        .map(|(_, interval)| *interval)
}

/// Dropped rune bounds.
const RUNE_BOUNDS: Bounds = Bounds {
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
};

/// Next deathmatch spawn in round-robin order (`nextRuneSpawn`).
fn next_rune_spawn(game: &mut Q1EntityServices) -> Result<ActorId, Q1Error> {
    let spots: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|id| {
            game.entity_ref(id)
                .is_some_and(|entity| entity.classname == "info_player_deathmatch")
        })
        .collect();
    if spots.is_empty() {
        return Err(q1_error("CTF has no info_player_deathmatch to spawn a rune"));
    }
    let world = ctf_world(game)?;
    let previous = game
        .entity_ref(&world)
        .and_then(|entity| entity.references.get("ctf.runeSpawn").cloned().flatten());
    let ordinal = previous
        .as_ref()
        .and_then(|previous| spots.iter().position(|spot| spot == previous))
        .map_or(-1, |index| index as i32);
    let spot = spots[((ordinal + 1) as usize) % spots.len()].clone();
    let reference = spot.clone();
    game.update_entity(&world, |entity| {
        entity.references.insert(String::from("ctf.runeSpawn"), Some(reference));
    })?;
    Ok(spot)
}

/// Spawn a tossed rune item (`droppedRune`).
fn dropped_rune(game: &mut Q1EntityServices, rune: CtfRune, origin: Vec3) -> Result<ActorId, Q1Error> {
    let item = game.create(&format!("item_rune_{}", rune.as_str()), None, None)?;
    let ordinal = rune.ordinal();
    let touch = game.named.touch("ctf:rune_touch")?;
    game.update_entity(&item, |entity| {
        entity.model = format!("progs/end{}.mdl", ordinal + 1);
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::Toss;
        entity.movement_flags |= 256;
        entity
            .fields
            .insert(String::from("ctf.rune"), rune.as_str().to_string());
        entity.touch = Some(touch);
    })?;
    let velocity = Vec3 {
        x: (-500.0 + game.host.random() * 1000.0) as f32,
        y: (-500.0 + game.host.random() * 1000.0) as f32,
        z: 400.0,
    };
    game.set_body(
        &item,
        &BodyPatch {
            origin: Some(vadd(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: -24.0,
                },
            )),
            velocity: Some(velocity),
            bounds: Some(RUNE_BOUNDS),
            ..Default::default()
        },
    )?;
    game.link(&item)?;
    game.schedule(&item, 120.0, "ctf:rune_respawn")?;
    Ok(item)
}

/// Rune kind of a dropped item, or fail with the donor message (`kind`).
fn rune_kind(game: &Q1EntityServices, item: &ActorId) -> Result<CtfRune, Q1Error> {
    let kind = game
        .entity_ref(item)
        .map(|entity| entity.text("ctf.rune"))
        .unwrap_or_default();
    CtfRune::parse(&kind).ok_or_else(|| q1_error("CTF rune lost its source kind"))
}

/// Drop the carried rune at the actor (`dropRune`).
pub fn drop_rune(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let Some(rune) = ctf_rune(game, &id) else {
        return Ok(());
    };
    let origin = ctf_body(game, &id)?.origin;
    dropped_rune(game, rune, origin)?;
    ctf_grant(game, &id, &rune_item(rune), 0.0, 1.0)?;
    with_ctf_services(game, |services| services.haste(&id, false))?;
    ctf_update(game, Some(&id))
}

/// Apply haste flags and regeneration ticks (`regenerate`).
pub fn regenerate(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let rune = ctf_rune(game, &id);
    with_ctf_services(game, |services| services.haste(&id, rune == Some(CtfRune::Haste)))?;
    if rune != Some(CtfRune::Regeneration)
        || ctf_number(game, &id, "regenTime")? >= game.time
        || game.health(&id) <= 0.0
    {
        return Ok(());
    }
    let owner = game
        .host
        .actors
        .resolve_owned(&id)
        .ok_or_else(|| q1_error("CTF player is no longer admitted"))?;
    let mut delay = 0.0;
    if game.health(&id) < 150.0 {
        game.host
            .combat
            .set_health(&owner, game.health(&id).min(150.0 - 5.0) + 5.0)?;
        delay += 0.5;
    }
    let regular = game.host.combat.read(&id).map(|combat| combat.armor.regular);
    let repair = match regular {
        Some(RegularArmorState::Q1 { points, absorption, .. }) => {
            if points < 150.0 && absorption > 0.0 {
                Some(points)
            } else {
                None
            }
        }
        Some(
            RegularArmorState::Q2 { points, .. }
            | RegularArmorState::Q3 { points, .. }
            | RegularArmorState::Source { points, .. },
        ) => {
            if points < 150.0 {
                Some(points)
            } else {
                None
            }
        }
        Some(RegularArmorState::None) | None => None,
    };
    if let Some(points) = repair {
        game.host
            .combat
            .set_regular_points(&owner, points.min(150.0 - 5.0) + 5.0)?;
        delay += 0.5;
    }
    ctf_set(game, &id, "regenTime", game.time + delay)?;
    if delay > 0.0 && ctf_number(game, &id, "regenSound")? < game.time {
        ctf_set(game, &id, "regenSound", game.time + 1.0)?;
        game.sound(owner.id(), "rune/rune4.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
    }
    Ok(())
}

/// Spawn the rune set once per map (`startRunes`).
pub fn start_runes(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if ctf_start_map(game) {
        return Ok(());
    }
    let world = ctf_world(game)?;
    let spawned = game
        .entity_ref(&world)
        .map(|entity| entity.number("ctf.runesSpawned"))
        .unwrap_or(0.0);
    if spawned != 0.0 {
        return Ok(());
    }
    set_addon_number(game, &world, "ctf.runesSpawned", 1.0)?;
    let timer = game.create("ctf_rune_spawn", None, None)?;
    game.schedule(&timer, 0.1, "ctf:rune_spawn")
}

fn rune_touch(
    game: &mut Q1EntityServices,
    item: &ActorId,
    actor: &ActorId,
    _normal: Option<Vec3>,
    _surface: Option<&TouchSurface>,
) -> Result<(), Q1Error> {
    let item = item.clone();
    let id = actor.clone();
    if !game.is_player(&id) || game.health(&id) <= 0.0 {
        return Ok(());
    }
    if with_ctf_services(game, |services| services.observer(&id))? {
        return Ok(());
    }
    if ctf_rune(game, &id).is_some() {
        if ctf_number(game, &id, "runeNotice")? < game.time {
            game.message(Some(&id), "$qc_already_have_rune", true, Vec::new());
            ctf_set(game, &id, "runeNotice", game.time + 5.0)?;
        }
        return Ok(());
    }
    let rune = rune_kind(game, &item)?;
    let ordinal = rune.ordinal();
    ctf_grant(game, &id, &rune_item(rune), 1.0, 1.0)?;
    game.message(Some(&id), &format!("$qc_rune{}_hud", ordinal + 1), true, Vec::new());
    let owner = game
        .host
        .actors
        .resolve_owned(&id)
        .map(|owned| owned.id().clone())
        .unwrap_or_else(|| id.clone());
    game.sound(&owner, "weapons/lock4.wav", Q1SoundChannel::Item, 1.0, 1.0)?;
    game.effect(Q1Effect::Pickup, ctf_body(game, &id)?.origin, Some(&id), 1);
    let teamplay = crate::q1::addons::ctf::state::ctf_teamplay(game)?;
    if teamplay == 0.0 && rune != CtfRune::Regeneration || teamplay == 2147483648.0 && rune == CtfRune::Regeneration {
        crate::q1::addons::ctf::state::ctf_announce(game, &format!("$qc_got_rune{}", ordinal + 1), Some(&id), "")?;
    }
    game.remove(&item)?;
    ctf_update(game, Some(&id))
}

fn rune_respawn(game: &mut Q1EntityServices, item: &ActorId) -> Result<(), Q1Error> {
    let item = item.clone();
    let rune = rune_kind(game, &item)?;
    let spot = next_rune_spawn(game)?;
    let origin = game.body(&spot)?.origin;
    dropped_rune(game, rune, origin)?;
    game.remove(&item)
}

fn rune_spawn(game: &mut Q1EntityServices, timer: &ActorId) -> Result<(), Q1Error> {
    let timer = timer.clone();
    let mut count = game.host.random() * 10.0;
    while count > 0.0 {
        next_rune_spawn(game)?;
        count -= 1.0;
    }
    for rune in CTF_RUNES {
        let spot = next_rune_spawn(game)?;
        let origin = game.body(&spot)?.origin;
        dropped_rune(game, rune, origin)?;
    }
    game.remove(&timer)
}

/// Register rune callbacks and post-quad damage effects
/// (`registerRunes`).
pub fn register_runes(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "ctf:rune_touch",
        Q1CallbackHandlers {
            touch: Some(rune_touch as Q1TouchHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:rune_respawn",
        Q1CallbackHandlers {
            action: Some(rune_respawn as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    game.named.register(
        "ctf:rune_spawn",
        Q1CallbackHandlers {
            action: Some(rune_spawn as Q1ActionHandler),
            ..Default::default()
        },
    )?;
    let key = ctf_key(game);
    game.register_damage_source_effects(
        "ctf:runes",
        Q1DamageSourceEffects {
            after_quad: Some(Box::new(move |request, initial, _target, _attacker| {
                ctf_by_key(key, |state| {
                    let (amount, sound, hurts_carrier, time) = {
                        let policy = &state.policy;
                        let mut amount = initial;
                        let attacker_rune = request
                            .attack
                            .attacker
                            .as_ref()
                            .and_then(|attacker| policy.runes.get(attacker).copied().flatten());
                        if attacker_rune == Some(CtfRune::Strength) {
                            amount = fround(amount * 2.0);
                        }
                        let mut sound = false;
                        if policy.runes.get(&request.target).copied().flatten() == Some(CtfRune::Resistance) {
                            amount = fround(amount / 2.0);
                            let throttle = policy.resistance_throttle.get(&request.target).copied().unwrap_or(0.0);
                            sound = throttle < policy.time;
                        }
                        let mut hurts_carrier = false;
                        if let Some(attacker) = request.attack.attacker.as_ref() {
                            let attacker_team = policy.lastteam.get(attacker).copied().flatten();
                            let target_team = policy.lastteam.get(&request.target).copied().flatten();
                            hurts_carrier = policy.players.contains(attacker)
                                && policy.carried.contains(&request.target)
                                && attacker_team != target_team
                                && target_team.is_some();
                        }
                        (amount, sound, hurts_carrier, policy.time)
                    };
                    if sound {
                        state
                            .policy
                            .resistance_throttle
                            .insert(request.target.clone(), time + 1.0);
                        state.pending.push(CtfDeferred::Sound {
                            actor: request.target.clone(),
                            path: "rune/rune1.wav",
                            channel: Q1SoundChannel::Body,
                        });
                    }
                    if hurts_carrier {
                        if let Some(attacker) = request.attack.attacker.clone() {
                            state.pending.push(CtfDeferred::PlayerNumber {
                                actor: attacker,
                                name: String::from("ctf.lastHurtCarrier"),
                                value: time,
                            });
                        }
                    }
                    DamagePreparation::Continue { amount }
                })
                .unwrap_or(DamagePreparation::Continue { amount: initial })
            })),
            ..Default::default()
        },
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::{register_ctf_state, sync_ctf_policy};
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn haste_intervals_match_donor() {
        assert_eq!(ctf_haste_interval("q1:weapon/axe"), Some(0.3));
        assert_eq!(ctf_haste_interval("q1:weapon/supershotgun"), Some(0.4));
        assert_eq!(ctf_haste_interval("q1:weapon/nailgun"), None);
        assert_eq!(CTF_HASTE_NAIL_SPEED, 2000.0);
    }

    #[test]
    fn regenerate_ticks_health_and_armor() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        register_runes(&mut game).expect("runes");
        ctf_grant(&mut game, &player, &rune_item(CtfRune::Regeneration), 1.0, 1.0).expect("grant");
        game.set_health(&player, 100.0).expect("health");
        ctf_set(&mut game, &player, "regenTime", -1.0).expect("regen time");
        regenerate(&mut game, &player).expect("regen");
        assert_eq!(game.health(&player), 105.0);
        sync_ctf_policy(&mut game).expect("sync");
        drop_rune(&mut game, &player).expect("drop");
        assert_eq!(ctf_rune(&game, &player), None);
    }
}
