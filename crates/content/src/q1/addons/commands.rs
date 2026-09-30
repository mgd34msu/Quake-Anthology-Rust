//! Q1 addon impulses and debug commands.
//!
//! Provenance: `src/content/q1/addons/commands.ts`.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::contract::{InventoryEntry, ItemId};
use crate::q1::addons::campaign::{BLOODY_NIGHTMARE_ACTIVE, BLOODY_NIGHTMARE_DISCOVERED, BLOODY_NIGHTMARE_NEWGAME};
use crate::q1::addons::context::{
    addon_cheat_arsenal, addon_cvar, addon_emit, addon_player_number, addon_program, addon_set_cvar,
    set_addon_player_number, Q1AddonCheatCategory, Q1AddonEvent, Q1AddonProgram,
};
use crate::q1::addons::monsters::startup::waiting_mg3_monster;
use crate::q1::base::provider::{campaign_set_skill, update_base};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::types::{normalize, vadd, vscale, vsub, Q1Event, Q1TraceRequest, Q1Weapon, POINT, WEAPONS};
use crate::q1::{q1_error, Q1Error};

/// Sets an inventory count, preserving the existing entry when present
/// (`noKeys` `setCount`).
fn set_inventory_count(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    player: &qa_core::identity::OwnedActor,
    item: ItemId,
    count: f64,
    capacity: f64,
) -> Result<(), Q1Error> {
    let previous = game
        .host
        .inventory
        .entries(actor)
        .into_iter()
        .find(|entry| entry.item == item);
    let entry = match previous {
        Some(mut found) => {
            found.count = count;
            found
        }
        None => InventoryEntry {
            item,
            count,
            capacity,
            count_policy: None,
        },
    };
    game.host.inventory.configure(player, &entry)
}

/// Grants the id1 arsenal without keys (`noKeys`).
fn no_keys(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let player = game
        .player_owned(actor)
        .ok_or_else(|| q1_error("Addon command requires an admitted Q1 arsenal"))?;
    if (game.options().deathmatch != 0 || game.options().coop) && addon_cvar(game, "sv_cheats")? == 0.0 {
        return Ok(());
    }
    let weapons = addon_cheat_arsenal(game, actor, Q1AddonCheatCategory::Weapons)?;
    let ammo = addon_cheat_arsenal(game, actor, Q1AddonCheatCategory::Ammo)?;
    if !ammo {
        set_inventory_count(game, actor, &player, String::from("q1:ammo/rockets"), 100.0, 100.0)?;
        set_inventory_count(game, actor, &player, String::from("q1:ammo/nails"), 200.0, 200.0)?;
        set_inventory_count(game, actor, &player, String::from("q1:ammo/shells"), 100.0, 100.0)?;
        set_inventory_count(game, actor, &player, String::from("q1:ammo/cells"), 200.0, 100.0)?;
    }
    if !weapons {
        for weapon in WEAPONS {
            let item = game.weapon_item(Q1Weapon::from(weapon));
            set_inventory_count(game, actor, &player, item, 1.0, 1.0)?;
        }
        game.select_weapon(&player, Q1Weapon::Rocketlauncher)?;
    }
    Ok(())
}

/// Kills every live monster, firing boss end actions first
/// (`omnicideQ1Addons`).
pub fn omnicide_q1_addons(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let mg3 = addon_program(game)? == Q1AddonProgram::Mg3;
    let ids = game.entity_ids();
    for id in ids {
        let candidate = game.entity_ref(&id).map(|entity| {
            (
                entity.movement_flags,
                entity.target.clone(),
                entity.killtarget.clone(),
                entity.classname.clone(),
            )
        });
        let Some((movement_flags, target, killtarget, classname)) = candidate else {
            continue;
        };
        if !game.is_live(&id) || movement_flags & (32 | 16384) == 0 {
            continue;
        }
        if !target.is_empty() || !killtarget.is_empty() {
            game.use_targets(&id, Some(actor))?;
        }
        if mg3 {
            if classname == "monster_oldone_new" {
                addon_emit(
                    game,
                    Q1AddonEvent::Music {
                        track: 3,
                        loop_track: 3,
                    },
                )?;
                game.host.emit(Q1Event::Lightstyle {
                    style: 0,
                    pattern: String::from("m"),
                });
                game.invoke_action(&id, "mg3:bosses:oldnew_credits")?;
            } else if classname == "monster_boss" {
                game.invoke_action(&id, "mg3:bosses:boss_end")?;
            }
        }
        if game.is_live(&id) {
            game.remove(&id)?;
        }
    }
    let total = game.total_monsters;
    game.killed_monsters = total;
    addon_emit(game, Q1AddonEvent::MonsterCount { count: total })
}

/// Removes hunter marker entities (`cleanupMarkers`).
fn cleanup_markers(game: &mut Q1EntityServices, name: &str) -> Result<(), Q1Error> {
    let ids = game.entity_ids();
    for id in ids {
        let is_marker = game.entity_ref(&id).is_some_and(|entity| entity.classname == name);
        if is_marker {
            game.remove(&id)?;
        }
    }
    Ok(())
}

/// Handles an addon impulse, returning whether it was consumed
/// (`handleQ1AddonImpulse`).
pub fn handle_q1_addon_impulse(game: &mut Q1EntityServices, actor: &ActorId, impulse: i32) -> Result<bool, Q1Error> {
    let program = addon_program(game)?;
    if program == Q1AddonProgram::Ctf {
        return Ok(false);
    }
    if impulse == 219 {
        omnicide_q1_addons(game, actor)?;
        return Ok(true);
    }
    if program != Q1AddonProgram::Mg3 {
        if impulse == 99 {
            no_keys(game, actor)?;
            return Ok(true);
        }
        return Ok(false);
    }
    let flags = update_base(game, |state| state.campaign.read_flags())?;
    if impulse == 11 {
        for rune in [1, 2, 4, 8] {
            if flags & rune == 0 {
                update_base(game, |state| state.campaign.write_flags(flags | rune))?;
                return Ok(true);
            }
        }
        addon_emit(
            game,
            Q1AddonEvent::DeveloperMessage {
                text: String::from("already has all runes!\n"),
            },
        )?;
        return Ok(true);
    }
    if (101..=105).contains(&impulse) {
        let granted = if impulse == 105 { 15 } else { 1 << (impulse - 101) };
        update_base(game, |state| {
            state.campaign.write_flags(flags | granted);
        })?;
        return Ok(true);
    }
    match impulse {
        116 | 117 => {
            let field = if impulse == 116 { "secrethunter" } else { "exithunter" };
            let enabled = addon_player_number(game, actor, field)? == 0.0;
            set_addon_player_number(game, actor, field, f64::from(i32::from(enabled)))?;
            if !enabled {
                cleanup_markers(game, if impulse == 116 { "secret_marker" } else { "exit_marker" })?;
            }
            Ok(true)
        }
        119 | 121 => {
            let field = if impulse == 119 { "monsterhunter" } else { "buddha" };
            let enabled = addon_player_number(game, actor, field)? == 0.0;
            set_addon_player_number(game, actor, field, f64::from(i32::from(enabled)))?;
            Ok(true)
        }
        220 => {
            let base = match game.entity_ref(actor) {
                Some(entity) => entity.effects,
                None => addon_player_number(game, actor, "effects")? as i32,
            };
            let effects = base | 8;
            if game.entity_ref(actor).is_some() {
                game.update_entity(actor, |entity| entity.effects = effects)?;
            }
            set_addon_player_number(game, actor, "effects", f64::from(effects))?;
            addon_emit(
                game,
                Q1AddonEvent::ActorEffects {
                    actor: actor.clone(),
                    effects,
                },
            )?;
            Ok(true)
        }
        222 => {
            let map = game.map_name.clone();
            game.travel(&map, Some(actor));
            Ok(true)
        }
        223 => {
            if flags & BLOODY_NIGHTMARE_ACTIVE != 0 {
                update_base(game, |state| {
                    state.campaign.write_flags(flags & !BLOODY_NIGHTMARE_ACTIVE);
                })?;
            } else {
                update_base(game, |state| {
                    state
                        .campaign
                        .write_flags(flags | BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED);
                })?;
                if addon_cvar(game, "skill")? != 3.0 {
                    addon_set_cvar(game, "skill", "3")?;
                    campaign_set_skill(game, 3)?;
                }
            }
            Ok(true)
        }
        224 => {
            update_base(game, |state| {
                state.campaign.write_flags(flags ^ BLOODY_NIGHTMARE_NEWGAME);
            })?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Draws hunter markers and monster bounds for one player each frame
/// (`frameQ1AddonPlayer`).
pub fn frame_q1_addon_player(game: &mut Q1EntityServices, actor: &ActorId, view_offset: Vec3) -> Result<(), Q1Error> {
    if addon_program(game)? != Q1AddonProgram::Mg3 {
        return Ok(());
    }
    let body = match game.host.bodies.read(actor) {
        Some(body) => body,
        None => return Ok(()),
    };
    let eye = vadd(body.origin, view_offset);
    for (field, classname, marker_name) in [
        ("secrethunter", "trigger_secret", "secret_marker"),
        ("exithunter", "trigger_changelevel", "exit_marker"),
    ] {
        if addon_player_number(game, actor, field)? == 0.0 {
            continue;
        }
        cleanup_markers(game, marker_name)?;
        let ids = game.entity_ids();
        for id in ids {
            let target = game.entity_ref(&id).and_then(|entity| {
                if entity.classname == classname {
                    Some(entity.trigger_bounds)
                } else {
                    None
                }
            });
            let Some(trigger_bounds) = target else {
                continue;
            };
            let target_body = game.body(&id)?;
            let bounds = trigger_bounds.unwrap_or(target_body.bounds);
            let middle = vadd(target_body.origin, vscale(vadd(bounds.min, bounds.max), 0.5));
            let trace = game.host.trace(&Q1TraceRequest {
                start: eye,
                end: middle,
                bounds: POINT,
                ignore: Some(actor.clone()),
                monsters: false,
                missile: false,
            });
            let marker = game.create(marker_name, None, None)?;
            game.update_entity(&marker, |entity| {
                entity.model = String::from("progs/s_bubble.spr");
            })?;
            game.set_body(
                &marker,
                &BodyPatch {
                    origin: Some(vsub(trace.end, vscale(normalize(vsub(middle, eye)), 4.0))),
                    ..BodyPatch::default()
                },
            )?;
            game.link(&marker)?;
        }
    }
    if addon_player_number(game, actor, "monsterhunter")? != 0.0 {
        let ids = game.entity_ids();
        for id in ids {
            let current = game.body(&id)?;
            let observed = game
                .entity_ref(&id)
                .map(|entity| (entity.movement_flags, entity.actor.id().clone()));
            let Some((movement_flags, target)) = observed else {
                continue;
            };
            if movement_flags & 32 != 0 && game.health(&target) > 0.0 {
                addon_emit(
                    game,
                    Q1AddonEvent::DebugBounds {
                        min: vadd(current.origin, current.bounds.min),
                        max: vadd(current.origin, current.bounds.max),
                        color: 251,
                        lifetime: 0.0,
                        depth_test: false,
                    },
                )?;
            } else if waiting_mg3_monster(game, &id) {
                let half = Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 16.0,
                };
                addon_emit(
                    game,
                    Q1AddonEvent::DebugBounds {
                        min: vsub(current.origin, half),
                        max: vadd(current.origin, half),
                        color: 244,
                        lifetime: 0.0,
                        depth_test: false,
                    },
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, test_addon_events};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::types::ZERO;
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, program: Q1AddonProgram) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, program);
        let player = attach_test_player(game);
        (guard, player)
    }

    fn campaign_flags(game: &Q1EntityServices) -> i32 {
        update_base(game, |state| state.campaign.read_flags()).expect("flags")
    }

    #[test]
    fn ctf_ignores_impulses() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Ctf);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 219), Ok(false));
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 99), Ok(false));
    }

    #[test]
    fn impulse_99_grants_arsenal_outside_mg3() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg1);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 99), Ok(true));
        let entries = game.host.inventory.entries(&player);
        let rockets = entries
            .iter()
            .find(|entry| entry.item == "q1:ammo/rockets")
            .expect("rockets");
        assert_eq!(rockets.count, 100.0);
        let launcher = entries
            .iter()
            .find(|entry| entry.item == "q1:weapon/rocketlauncher")
            .expect("launcher");
        assert_eq!(launcher.count, 1.0);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 100), Ok(false));
    }

    #[test]
    fn mg3_grants_runes_one_at_a_time() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 11), Ok(true));
        assert_eq!(campaign_flags(&game), 1);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 11), Ok(true));
        assert_eq!(campaign_flags(&game), 3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 102), Ok(true));
        assert_eq!(campaign_flags(&game), 3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 105), Ok(true));
        assert_eq!(campaign_flags(&game), 15);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 11), Ok(true));
        assert!(test_addon_events(&game).expect("events").iter().any(|event| matches!(
            event,
            Q1AddonEvent::DeveloperMessage { text } if text == "already has all runes!\n"
        )));
    }

    #[test]
    fn hunter_toggles_flip_player_numbers() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 116), Ok(true));
        assert_eq!(addon_player_number(&game, &player, "secrethunter"), Ok(1.0));
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 116), Ok(true));
        assert_eq!(addon_player_number(&game, &player, "secrethunter"), Ok(0.0));
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 119), Ok(true));
        assert_eq!(addon_player_number(&game, &player, "monsterhunter"), Ok(1.0));
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 121), Ok(true));
        assert_eq!(addon_player_number(&game, &player, "buddha"), Ok(1.0));
    }

    #[test]
    fn impulse_220_sets_effects_flag() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 220), Ok(true));
        let entity = game.entity_ref(&player).expect("player entity");
        assert_eq!(entity.effects, 8);
        assert!(test_addon_events(&game).expect("events").iter().any(|event| matches!(
            event,
            Q1AddonEvent::ActorEffects { effects, .. } if *effects == 8
        )));
    }

    #[test]
    fn nightmare_and_newgame_impulses_toggle_flags() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 223), Ok(true));
        assert_eq!(
            campaign_flags(&game),
            BLOODY_NIGHTMARE_ACTIVE | BLOODY_NIGHTMARE_DISCOVERED
        );
        assert_eq!(addon_cvar(&game, "skill"), Ok(3.0));
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 223), Ok(true));
        assert_eq!(campaign_flags(&game), BLOODY_NIGHTMARE_DISCOVERED);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 224), Ok(true));
        assert_eq!(
            campaign_flags(&game),
            BLOODY_NIGHTMARE_DISCOVERED | BLOODY_NIGHTMARE_NEWGAME
        );
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 224), Ok(true));
        assert_eq!(campaign_flags(&game), BLOODY_NIGHTMARE_DISCOVERED);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 222), Ok(true));
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 200), Ok(false));
    }

    #[test]
    fn omnicide_removes_live_monsters() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg1);
        let monster = game.create("monster_dog", None, None).expect("monster");
        game.update_entity(&monster, |entity| entity.movement_flags = 32)
            .expect("flags");
        game.total_monsters = 1;
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 219), Ok(true));
        assert!(game.entity_ref(&monster).is_none());
        assert_eq!(game.killed_monsters, 1);
        assert!(test_addon_events(&game)
            .expect("events")
            .iter()
            .any(|event| matches!(event, Q1AddonEvent::MonsterCount { count: 1 })));
    }

    #[test]
    fn frame_player_skips_without_hunters() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg1);
        frame_q1_addon_player(&mut game, &player, ZERO).expect("frame");
        let markers = game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.classname == "secret_marker")
            })
            .count();
        assert_eq!(markers, 0);
    }

    #[test]
    fn frame_player_marks_secrets() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 116), Ok(true));
        let secret = game.create("trigger_secret", None, None).expect("secret");
        game.set_body(
            &secret,
            &BodyPatch {
                origin: Some(Vec3 {
                    x: 64.0,
                    y: 0.0,
                    z: 0.0,
                }),
                ..BodyPatch::default()
            },
        )
        .expect("origin");
        frame_q1_addon_player(&mut game, &player, ZERO).expect("frame");
        let markers: Vec<ActorId> = game
            .entity_ids()
            .into_iter()
            .filter(|id| {
                game.entity_ref(id)
                    .is_some_and(|entity| entity.classname == "secret_marker")
            })
            .collect();
        assert_eq!(markers.len(), 1);
        let marker = game.entity_ref(&markers[0]).expect("marker");
        assert_eq!(marker.model, "progs/s_bubble.spr");
    }

    #[test]
    fn frame_player_reports_monster_bounds() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, Q1AddonProgram::Mg3);
        assert_eq!(handle_q1_addon_impulse(&mut game, &player, 119), Ok(true));
        let monster = game.create("monster_dog", None, None).expect("monster");
        game.update_entity(&monster, |entity| entity.movement_flags = 32)
            .expect("flags");
        game.set_health(&monster, 100.0).expect("health");
        frame_q1_addon_player(&mut game, &player, ZERO).expect("frame");
        assert!(test_addon_events(&game)
            .expect("events")
            .iter()
            .any(|event| matches!(event, Q1AddonEvent::DebugBounds { color: 251, .. })));
    }
}
