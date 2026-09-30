//! Q1 ThreeWave CTF addon (src/content/q1/addons/ctf/index.ts).
//!
//! Session lifecycle entry points; all mutations remain on shared
//! actors and named source callbacks.

pub mod arsenal;
pub mod drops;
pub mod flags;
pub mod grapple;
pub mod maps;
pub mod observer;
pub mod runes;
pub mod scoring;
pub mod state;
pub mod teams;
pub mod travel;
pub mod types;

pub use crate::q1::addons::ctf::arsenal::CtfCharacterPose;
pub use crate::q1::addons::ctf::runes::{ctf_haste_interval, CTF_HASTE_INTERVALS, CTF_HASTE_NAIL_SPEED};
pub use crate::q1::addons::ctf::types::{
    CtfFlags, CtfInput, CtfPromptChoice, CtfRune, CtfStatus, CtfTeam, Q1CtfServices, CTF_RUNES,
};

use qa_core::identity::ActorId;

use crate::contract::{GrappleBinding, GrappleMechanic, GrappleMechanicDetail, GrappleSelection, SharedGrappleControl};
use crate::q1::addons::context::{addon_program, Q1AddonProgram};
use crate::q1::addons::ctf::arsenal::{
    character_pose as arsenal_character_pose, grapple_attack, register_arsenal, spawn_arsenal,
};
use crate::q1::addons::ctf::drops::{register_drops, toss_ammo, toss_weapon};
use crate::q1::addons::ctf::flags::{drop_flag, register_flags};
use crate::q1::addons::ctf::grapple::{grapple_trail, register_ctf_grapple, unhook};
use crate::q1::addons::ctf::maps::register_maps;
use crate::q1::addons::ctf::observer::{become_observer, observer_frame};
use crate::q1::addons::ctf::runes::{drop_rune, regenerate, register_runes};
use crate::q1::addons::ctf::scoring::{register_combat, score_death};
use crate::q1::addons::ctf::state::{
    ctf_announce, ctf_grant, ctf_grapple_pulling, ctf_number, ctf_set, ctf_start_map, ctf_team, ctf_teamplay,
    ctf_teamplay_bits, ctf_update, drain_ctf_deferred, native_grapple_enabled, register_ctf_state,
    shared_grapple_disabled, sync_ctf_policy, with_ctf_services,
};
use crate::q1::addons::ctf::teams::{check_team, check_team_lock, observer_impulse, show_team_prompt, spawn_point};
use crate::q1::addons::ctf::travel::{capture_travel as capture_ctf_travel, restore_travel as restore_ctf_travel};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::extensions::Q1PlayerExtension;
use crate::q1::{q1_error, Q1Error};

/// Register CTF content on a game (`registerCTF`). Requires the
/// selected CTF source program.
pub fn register_ctf(
    game: &mut Q1EntityServices,
    services: Box<dyn Q1CtfServices>,
    shared_grapple: Option<&dyn SharedGrappleControl>,
) -> Result<(), Q1Error> {
    if addon_program(game)? != Q1AddonProgram::Ctf {
        return Err(q1_error("CTF requires the selected CTF source program"));
    }
    let native = match shared_grapple {
        None => true,
        Some(shared) => {
            matches!(
                shared.selection(),
                GrappleSelection::Enabled {
                    binding: GrappleBinding::Slot,
                    mechanic: GrappleMechanicDetail::Q1Threewave { .. },
                    ..
                }
            ) && shared.native_slot(GrappleMechanic::Q1Threewave)
        }
    };
    let disabled = shared_grapple.is_some_and(|shared| matches!(shared.selection(), GrappleSelection::Disabled));
    register_ctf_state(game, services, native, disabled);
    register_ctf_grapple(game)?;
    register_flags(game)?;
    register_runes(game)?;
    register_combat(game)?;
    register_drops(game)?;
    register_arsenal(game)?;
    register_maps(game)?;
    game.register_player_extension(Q1PlayerExtension {
        id: String::from("ctf:spawn_parameters"),
        capture_travel: Some(capture_ctf_travel),
        restore_travel: Some(restore_ctf_travel),
        ..Default::default()
    })?;
    sync_ctf_policy(game)
}

/// Invoke on admission/respawn, after the selected character and
/// arsenal are initialized (`spawnPlayer`).
pub fn spawn_player(game: &mut Q1EntityServices, actor: &ActorId, first_admission: bool) -> Result<(), Q1Error> {
    let id = actor.clone();
    spawn_arsenal(game, &id)?;
    if shared_grapple_disabled(game)? {
        ctf_grant(game, &id, "q1:ctf/weapon/grapple", 0.0, 1.0)?;
    }
    ctf_set(game, &id, "lastHurtCarrier", -10.0)?;
    ctf_set(game, &id, "regenTime", 0.0)?;
    ctf_set(game, &id, "runeNotice", 0.0)?;
    if first_admission {
        ctf_set(game, &id, "killed", 0.0)?;
        ctf_set(game, &id, "motd", 0.0)?;
        if ctf_teamplay_bits(game)? & CtfFlags::SELECT_TEAM != 0 && !ctf_start_map(game) {
            become_observer(game, &id)?;
        } else {
            check_team(game, &id)?;
        }
    } else if with_ctf_services(game, |services| services.observer(&id))? {
        become_observer(game, &id)?;
    }
    if native_grapple_enabled(game)?
        && !ctf_start_map(game)
        && ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE == 0
    {
        ctf_grant(game, &id, "q1:ctf/weapon/grapple", 1.0, 1.0)?;
    }
    ctf_set(game, &id, "stuffColor", 1.0)?;
    ctf_update(game, Some(&id))
}

/// Select a spawn point (`selectSpawn`).
pub fn select_spawn(game: &mut Q1EntityServices, actor: &ActorId) -> Result<Option<ActorId>, Q1Error> {
    spawn_point(game, actor)
}

/// Character pose contribution (`characterPose`).
pub fn character_pose(game: &mut Q1EntityServices, actor: &ActorId) -> Result<CtfCharacterPose, Q1Error> {
    arsenal_character_pose(game, actor)
}

/// Whether falling damage applies (`fallDamageAllowed`).
#[must_use]
pub fn fall_damage_allowed(game: &Q1EntityServices, actor: &ActorId) -> bool {
    !ctf_grapple_pulling(game, actor)
}

/// Capture the spawn-parameters extension bytes (`captureTravel`).
pub fn capture_travel(game: &mut Q1EntityServices, actor: &ActorId) -> Vec<u8> {
    capture_ctf_travel(game, actor)
}

/// Restore the spawn-parameters extension bytes (`restoreTravel`).
pub fn restore_travel(game: &mut Q1EntityServices, actor: &ActorId, bytes: &[u8]) -> Result<(), Q1Error> {
    restore_ctf_travel(game, actor, bytes)
}

/// Source prethink, called once per player by the shared session even
/// with foreign characters (`playerFrame`).
pub fn player_frame(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    sync_ctf_policy(game)?;
    drain_ctf_deferred(game)?;
    if game.intermission.is_some() {
        return Ok(());
    }
    if ctf_number(game, &id, "motd")? == 2.0 {
        ctf_update(game, Some(&id))?;
        if with_ctf_services(game, |services| services.observer(&id))? {
            show_team_prompt(game, &id)?;
        } else {
            let key = if ctf_start_map(game) {
                "$qc_choose_exit"
            } else if ctf_team(game, &id) == Some(CtfTeam::Red) {
                "$qc_ctf_red"
            } else {
                "$qc_ctf_blue"
            };
            game.message(Some(&id), key, true, Vec::new());
        }
    }
    if ctf_number(game, &id, "motd")? <= 2.0 {
        ctf_set(game, &id, "motd", ctf_number(game, &id, "motd")? + 1.0)?;
    }
    let bot_homeless = with_ctf_services(game, |services| services.is_bot(&id))? && ctf_team(game, &id).is_none();
    if observer_impulse(game, &id, bot_homeless.then_some(103))? {
        return Ok(());
    }
    check_team_lock(game, &id)?;
    if with_ctf_services(game, |services| services.observer(&id))? {
        return observer_frame(game, &id);
    }
    if game.health(&id) <= 0.0 {
        return Ok(());
    }
    regenerate(game, &id)?;
    Ok(())
}

/// Emit the post-physics grapple trail (`afterPhysics`).
pub fn after_physics(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    grapple_trail(game, actor)
}

/// Handle a CTF impulse, returning whether it was consumed
/// (`impulse`). Returning true consumes only a CTF-specific command;
/// the selected arsenal handles all others.
pub fn impulse(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    let id = actor.clone();
    let input = with_ctf_services(game, |services| services.input(&id))?;
    if observer_impulse(game, &id, None)? {
        return Ok(true);
    }
    if input.impulse == 22 || input.impulse == 1 && !input.grapple_selected {
        if !native_grapple_enabled(game)? || ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE != 0 {
            game.message(Some(&id), "$qc_no_weapon", true, Vec::new());
        } else {
            with_ctf_services(game, |services| services.select_grapple(&id))?;
        }
    } else if !with_ctf_services(game, |services| services.observer(&id))?
        && ctf_teamplay_bits(game)? & CtfFlags::DROP_ITEMS != 0
        && input.impulse == 20
    {
        toss_ammo(game, &id)?;
    } else if !with_ctf_services(game, |services| services.observer(&id))?
        && ctf_teamplay_bits(game)? & CtfFlags::DROP_ITEMS != 0
        && input.impulse == 21
    {
        toss_weapon(game, &id)?;
    } else if input.impulse == 25 {
        let teamplay = ctf_teamplay(game)?;
        let bits = ctf_teamplay_bits(game)?;
        let text = if teamplay < 0.0 {
            format!("Frag Penalty: {}", -teamplay)
        } else {
            [
                (CtfFlags::HEALTH_PROTECT, "Health-Protect"),
                (CtfFlags::ARMOR_PROTECT, "Armor-Protect"),
                (CtfFlags::REFLECT_DAMAGE, "Mirror-Damage"),
                (CtfFlags::FRAG_PENALTY, "Frag-Penalty"),
                (CtfFlags::DEATH_PENALTY, "Death-Penalty"),
                (CtfFlags::STATIC_TEAMS, "Static-Teams"),
                (
                    CtfFlags::DROP_ITEMS,
                    "Drop-Items (Backpack Impulse 20, Weapon Impulse 21)",
                ),
            ]
            .into_iter()
            .filter(|(flag, _)| bits & *flag != 0)
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
            .join(" ")
        };
        game.message(Some(&id), &text, false, Vec::new());
    } else {
        return Ok(false);
    }
    with_ctf_services(game, |services| services.consume_impulse(&id))?;
    Ok(true)
}

/// Attack with the grapple when selected (`attack`).
pub fn attack(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    let id = actor.clone();
    let input = with_ctf_services(game, |services| services.input(&id))?;
    if !native_grapple_enabled(game)?
        || !input.grapple_selected
        || !input.attack
        || with_ctf_services(game, |services| services.observer(&id))?
        || game.health(&id) <= 0.0
    {
        return Ok(false);
    }
    grapple_attack(game, &id)?;
    Ok(true)
}

/// Score a death and drop carriers (`death`). Replaces ordinary frag
/// credit, before dropping the carried flag needed for bonuses.
pub fn death(game: &mut Q1EntityServices, victim: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let victim = victim.clone();
    score_death(game, &victim, attacker)?;
    ctf_set(game, &victim, "killed", 1.0)?;
    drop_flag(game, &victim)?;
    drop_rune(game, &victim)?;
    unhook(game, &victim)
}

/// Drop carriers on disconnect (`disconnectPlayer`).
pub fn disconnect_player(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    drop_flag(game, actor)?;
    drop_rune(game, actor)?;
    unhook(game, actor)
}

/// Handle a suicide with the CTF penalty (`suicide`).
pub fn suicide(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    if with_ctf_services(game, |services| services.observer(&id))? || ctf_start_map(game) {
        return Ok(());
    }
    if ctf_number(game, &id, "suicideCount")? > 3.0 {
        game.message(Some(&id), "$qc_ctf_too_many_suicide", true, Vec::new());
        return Ok(());
    }
    ctf_announce(game, "$qc_suicides", Some(&id), "")?;
    drop_flag(game, &id)?;
    drop_rune(game, &id)?;
    unhook(game, &id)?;
    with_ctf_services(game, |services| services.add_score(&id, -2.0))?;
    ctf_set(game, &id, "suicideCount", ctf_number(game, &id, "suicideCount")? + 1.0)?;
    let spot = spawn_point(game, &id)?;
    with_ctf_services(game, |services| services.respawn(&id, spot.as_ref()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons};
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf(game, Box::new(services), None).expect("ctf");
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn registration_requires_the_ctf_program() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        register_test_addons(&mut game, Q1AddonProgram::Mg1);
        let (services, _) = FakeCtfServices::new();
        assert!(register_ctf(&mut game, Box::new(services), None).is_err());
    }

    #[test]
    fn spawn_and_suicide_flow_scores() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        let start = game.create("info_player_start", None, None).expect("start");
        game.spawn_entity(&start, None).expect("spawn start");
        spawn_player(&mut game, &player, true).expect("spawn");
        assert!(ctf_team(&game, &player).is_some());
        player_frame(&mut game, &player).expect("frame");
        suicide(&mut game, &player).expect("suicide");
        let score = with_ctf_services(&game, |services| services.score(&player)).expect("score");
        assert_eq!(score, -2.0);
        assert_eq!(impulse(&mut game, &player), Ok(false));
        assert_eq!(attack(&mut game, &player), Ok(false));
        assert!(fall_damage_allowed(&game, &player));
    }
}
