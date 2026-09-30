//! Q1 CTF teammate protection and frag bonuses (src/content/q1/addons/ctf/scoring.ts).

use qa_core::identity::{same_actor, ActorId};

use crate::q1::addons::ctf::state::{
    ctf_announce, ctf_body, ctf_by_key, ctf_carried, ctf_flag, ctf_key, ctf_last_team, ctf_number, ctf_set, ctf_team,
    ctf_teamplay, ctf_teamplay_bits, with_ctf_services, CtfDeferred,
};
use crate::q1::addons::ctf::types::{CtfFlags, CtfTeam};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::{AttackCause, DamagePreparation, EnvironmentHazard, Q1DamageSourceEffects};
use crate::q1::foundation::types::{length, vadd, vscale, vsub, Q1MessageArg, Q1Solid};
use crate::q1::Q1Error;

/// Whether the attacker is a live teammate distinct from the target
/// (`friendly`).
fn friendly(game: &Q1EntityServices, target: &ActorId, attacker: Option<&ActorId>) -> Result<bool, Q1Error> {
    let Some(attacker) = attacker else {
        return Ok(false);
    };
    if same_actor(target, attacker) {
        return Ok(false);
    }
    let target_team = ctf_last_team(game, target)?;
    if target_team.is_none() {
        return Ok(false);
    }
    Ok(target_team == ctf_last_team(game, attacker)?)
}

/// Whether the request is falling damage.
fn is_falling(request: &crate::q1::foundation::gameplay::DamageRequest) -> bool {
    match &request.attack.cause {
        AttackCause::Q1 { death_type, .. } => death_type == "falling",
        AttackCause::Environment { hazard } => *hazard == EnvironmentHazard::Fall,
        _ => false,
    }
}

/// Register ordered teammate protection stages (`registerCombat`).
pub fn register_combat(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let key = ctf_key(game);
    let fall_key = key;
    let armor_key = key;
    let protection_key = key;
    let health_key = key;
    game.register_damage_source_effects(
        "ctf:team_damage",
        Q1DamageSourceEffects {
            before_quad: Some(Box::new(move |request, amount, _target, _attacker| {
                ctf_by_key(fall_key, |state| {
                    if is_falling(request) && state.policy.pulling.contains(&request.target) {
                        DamagePreparation::Cancel
                    } else {
                        DamagePreparation::Continue { amount }
                    }
                })
                .unwrap_or(DamagePreparation::Continue { amount })
            })),
            armor_allowed: Some(Box::new(move |request, _damage, _target, _attacker| {
                ctf_by_key(armor_key, |state| {
                    let policy = &state.policy;
                    if policy.teamplay_raw < 0.0 || policy.start_map || policy.teamplay & CtfFlags::ARMOR_PROTECT == 0 {
                        return true;
                    }
                    !friendly_policy(policy, &request.target, request.attack.attacker.as_ref())
                })
                .unwrap_or(true)
            })),
            protection_applies: Some(Box::new(move |request, target, _attacker| {
                ctf_by_key(protection_key, |state| {
                    let live = target.team.as_deref().and_then(CtfTeam::parse);
                    live == state.policy.lastteam.get(&request.target).copied().flatten()
                })
                .unwrap_or(true)
            })),
            before_health: Some(Box::new(move |request, damage, _target, _attacker| {
                ctf_by_key(health_key, |state| {
                    let policy = &state.policy;
                    if policy.teamplay_raw < 0.0
                        || policy.start_map
                        || !friendly_policy(policy, &request.target, request.attack.attacker.as_ref())
                    {
                        return true;
                    }
                    let mut reflect = None;
                    if let Some(attacker) = request.attack.attacker.as_ref() {
                        // TeamHealthDam receives full post-rune damage
                        // after the first armor and momentum commit.
                        if policy.teamplay & CtfFlags::REFLECT_DAMAGE != 0 {
                            reflect = Some(CtfDeferred::Reflect {
                                attacker: attacker.clone(),
                                inflictor: request.attack.inflictor.clone(),
                                damage,
                            });
                        }
                    }
                    let allowed = policy.teamplay & CtfFlags::HEALTH_PROTECT == 0;
                    if let Some(reflect) = reflect {
                        state.pending.push(reflect);
                    }
                    allowed
                })
                .unwrap_or(true)
            })),
            ..Default::default()
        },
    )?;
    Ok(())
}

/// Policy-backed teammate check for game-less stages.
fn friendly_policy(
    policy: &crate::q1::addons::ctf::state::CtfPolicy,
    target: &ActorId,
    attacker: Option<&ActorId>,
) -> bool {
    let Some(attacker) = attacker else {
        return false;
    };
    if same_actor(target, attacker) {
        return false;
    }
    let target_team = policy.lastteam.get(target).copied().flatten();
    target_team.is_some() && target_team == policy.lastteam.get(attacker).copied().flatten()
}

/// Replace the ordinary Q1 frag decision before death drops
/// (`scoreDeath`). Called once by the score owner.
pub fn score_death(game: &mut Q1EntityServices, victim: &ActorId, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
    let victim = victim.clone();
    if !game.is_player(&victim) {
        return Ok(());
    }
    let attacker = match attacker {
        Some(attacker) if game.is_player(attacker) && !same_actor(attacker, &victim) => attacker.clone(),
        _ => {
            with_ctf_services(game, |services| services.add_score(&victim, -1.0))?;
            return Ok(());
        }
    };
    let team = ctf_team(game, &attacker);
    let teammates = friendly(game, &victim, Some(&attacker))?;
    let teamplay = ctf_teamplay(game)?;
    if teamplay == 2.0 && team.is_some() && team == ctf_team(game, &victim) {
        with_ctf_services(game, |services| services.add_score(&attacker, -1.0))?;
        return Ok(());
    }
    let penalty = if teamplay < 0.0 {
        -teamplay
    } else if teammates && ctf_teamplay_bits(game)? & CtfFlags::FRAG_PENALTY != 0 {
        1.0
    } else {
        0.0
    };
    if penalty > 0.0 {
        with_ctf_services(game, |services| services.add_score(&attacker, -penalty))?;
    } else {
        with_ctf_services(game, |services| services.add_score(&attacker, 1.0))?;
        if ctf_carried(game, &victim).is_some() && ctf_team(game, &victim) != team {
            ctf_set(game, &attacker, "lastFraggedCarrier", game.time)?;
            if ctf_number(game, &victim, "flagSince")? + 2.0 > game.time {
                game.message(Some(&attacker), "$qc_ctf_carrier_no_bonus", true, Vec::new());
            } else {
                with_ctf_services(game, |services| services.add_score(&attacker, 2.0))?;
                game.message(
                    Some(&attacker),
                    "$qc_ctf_kill_carrier",
                    false,
                    vec![Q1MessageArg::Number(2.0)],
                );
            }
        }
        let team_name = match team {
            Some(CtfTeam::Red) => "$qc_ctf_redteam",
            Some(CtfTeam::Blue) => "$qc_ctf_blueteam",
            None => "",
        };
        let mut carrier_bonus = false;
        let mut flag_bonus = false;
        if ctf_number(game, &victim, "lastHurtCarrier")? + 4.0 > game.time && ctf_carried(game, &attacker).is_none() {
            with_ctf_services(game, |services| services.add_score(&attacker, 2.0))?;
            carrier_bonus = true;
            ctf_announce(game, "$qc_ks_defends_carrier_aggressive", Some(&attacker), team_name)?;
        }
        let centers = [ctf_body(game, &attacker)?.origin, ctf_body(game, &victim)?.origin];
        for (pass, center) in centers.into_iter().enumerate() {
            for player in (game.host.players)() {
                let body = ctf_body(game, &player)?;
                let player_center = vadd(body.origin, vscale(vadd(body.bounds.min, body.bounds.max), 0.5));
                let observed = with_ctf_services(game, |services| services.observer(&player))?;
                if !carrier_bonus
                    && !observed
                    && !same_actor(&player, &attacker)
                    && ctf_team(game, &player) == team
                    && ctf_carried(game, &player).is_some()
                    && length(vsub(player_center, center)) <= 550.0
                {
                    with_ctf_services(game, |services| services.add_score(&attacker, 1.0))?;
                    carrier_bonus = true;
                    ctf_announce(game, "$qc_ks_defends_carrier", Some(&attacker), team_name)?;
                }
            }
            let flag = team.and_then(|team| ctf_flag(game, team));
            if let Some(flag) = flag {
                let flag_body = game.body(&flag)?;
                let solid = game
                    .entity_ref(&flag)
                    .map(|entity| entity.solid)
                    .unwrap_or(Q1Solid::None);
                let center_body = vadd(
                    flag_body.origin,
                    vscale(vadd(flag_body.bounds.min, flag_body.bounds.max), 0.5),
                );
                // Preserve the QC red-team branch precedence: its second
                // radius pass can award a second flag bonus.
                if (!flag_bonus || team == Some(CtfTeam::Red) && pass == 1)
                    && solid != Q1Solid::None
                    && length(vsub(center_body, center)) <= 550.0
                {
                    with_ctf_services(game, |services| services.add_score(&attacker, 1.0))?;
                    flag_bonus = true;
                    ctf_announce(game, "$qc_ks_defends_flag", Some(&attacker), team_name)?;
                }
            }
        }
    }
    if teamplay >= 0.0 && teammates && ctf_teamplay_bits(game)? & CtfFlags::DEATH_PENALTY != 0 {
        game.damage(
            &attacker,
            Some(&attacker),
            Some(&attacker),
            1000.0,
            &Q1DamageParams {
                death_type: String::from("ctf:teamkill"),
                ..Default::default()
            },
        );
        with_ctf_services(game, |services| services.add_score(&attacker, 1.0))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::register_ctf_state;
    use crate::q1::addons::ctf::teams::set_team;
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let killer = attach_test_player(game);
        let victim = attach_test_player(game);
        (guard, killer, victim)
    }

    #[test]
    fn enemy_frag_scores_and_suicide_penalizes() {
        let mut game = test_game();
        let (_guard, killer, victim) = setup(&mut game);
        register_combat(&mut game).expect("combat");
        set_team(&mut game, &killer, Some(CtfTeam::Red)).expect("red");
        set_team(&mut game, &victim, Some(CtfTeam::Blue)).expect("blue");
        ctf_set(&mut game, &victim, "lastHurtCarrier", -10.0).expect("carrier");
        score_death(&mut game, &victim, Some(&killer)).expect("score");
        let killer_score = with_ctf_services(&game, |services| services.score(&killer)).expect("score");
        assert_eq!(killer_score, 1.0);
        score_death(&mut game, &victim, None).expect("suicide");
        let victim_score = with_ctf_services(&game, |services| services.score(&victim)).expect("score");
        assert_eq!(victim_score, -1.0);
    }

    #[test]
    fn teamplay_two_teamkill_penalizes() {
        let mut game = test_game();
        let (_guard, killer, victim) = setup(&mut game);
        register_combat(&mut game).expect("combat");
        crate::q1::addons::context::addon_set_cvar(&game, "teamplay", "2").expect("cvar");
        set_team(&mut game, &killer, Some(CtfTeam::Red)).expect("red");
        set_team(&mut game, &victim, Some(CtfTeam::Red)).expect("red");
        score_death(&mut game, &victim, Some(&killer)).expect("score");
        let killer_score = with_ctf_services(&game, |services| services.score(&killer)).expect("score");
        assert_eq!(killer_score, -1.0);
    }
}
