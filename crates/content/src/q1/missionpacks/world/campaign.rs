//! Mission-pack campaign finales (`src/content/q1/missionpacks/world/campaign.ts`).
//!
//! Mission pack client.qc / hipmisc.qc finale control.

use qa_core::identity::ActorId;

use crate::q1::base::provider::{
    base_registered_flag, campaign_read_flags, has_finished_finale, level_advance_finale, level_begin_cutscene,
    level_defer_exit, level_register_rule, official_campaign_flag,
};
use crate::q1::base::rules::{Q1FinaleDecision, Q1IntermissionResult, Q1IntermissionRule, Q1SourceFinale};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1CutsceneControl;
use crate::q1::foundation::types::{Q1Edition, Q1Event, ZERO};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::missionpacks::world::finale_text::mission_finale_text;
use crate::q1::{q1_error, Q1Error};

use super::common::{later, number};
use super::with_missionpack_hooks;

/// Poll for finale dismissal on rerelease builds (`startFinaleTimer`).
pub fn start_finale_timer(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if game.options().edition == Q1Edition::Classic {
        return Ok(());
    }
    let timer = game.create("mission_finale_timer", None, None)?;
    later(game, &timer, 1.0, "mission:finale_check")
}

/// Leave the finale: travel on coop, drop to credits otherwise.
fn finale_transition(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().coop {
        game.travel("start", None);
    } else {
        game.host.emit(Q1Event::ServerCommand {
            text: "menu_credits\n".to_string(),
        });
        game.host.emit(Q1Event::ServerCommand {
            text: "disconnect\n".to_string(),
        });
    }
    game.remove(id)
}

/// Re-poll until the finale is dismissed.
fn finale_check(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if has_finished_finale(game)? {
        return later(game, id, 5.0, "mission:finale_transition");
    }
    later(game, id, 0.1, "mission:finale_check")
}

/// Build a mission-pack source finale.
fn finale(game: &Q1EntityServices, key: &str, track: i32) -> Q1SourceFinale {
    Q1SourceFinale::Finale {
        text: mission_finale_text(game.options().edition, key),
        track,
    }
}

/// Rogue campaign finale rule.
fn rogue_campaign_finale(
    game: &mut Q1EntityServices,
    stage: i32,
    _next_map: &str,
) -> Result<Q1FinaleDecision, Q1Error> {
    let map = game.map_name.clone();
    if stage == 2 && map == "r1m7" {
        return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_r1", 3)));
    }
    if stage == 2 && map == "r2m8" && game.options().coop && game.options().edition == Q1Edition::Rerelease {
        game.host.emit(Q1Event::ServerCommand {
            text: "menu_credits\ndisconnect\n".to_string(),
        });
        let until = game.time + 10_000_000.0;
        level_defer_exit(game, until)?;
        return Ok(Q1FinaleDecision::Finale(Q1SourceFinale::Finale {
            text: String::new(),
            track: 3,
        }));
    }
    Ok(Q1FinaleDecision::Delegate)
}

/// Hipnotic campaign finale rule.
fn hipnotic_campaign_finale(
    game: &mut Q1EntityServices,
    stage: i32,
    _next_map: &str,
) -> Result<Q1FinaleDecision, Q1Error> {
    let map = game.map_name.clone();
    if stage == 2 {
        if map == "hip1m4" {
            return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_hip1", 6)));
        }
        if map == "hip2m5" {
            return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_hip2", 6)));
        }
        if map == "hipend" {
            if game.options().edition == Q1Edition::Rerelease && official_campaign_flag(game)? {
                game.host.emit(Q1Event::Achievement {
                    player: None,
                    id: "ACH_COMPLETE_HIPEND".to_string(),
                });
                if game.options().skill == 3 {
                    game.host.emit(Q1Event::Achievement {
                        player: None,
                        id: "ACH_COMPLETE_HIPEND_NIGHTMARE".to_string(),
                    });
                }
            }
            return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_hipend", 2)));
        }
    }
    if stage == 3 && base_registered_flag(game)? && campaign_read_flags(game)? & 15 != 15 {
        if map == "hip1m4" {
            return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_hip1m4", 6)));
        }
        if map == "hip2m5" {
            return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_hip2m5", 6)));
        }
        if map == "hipend" {
            let until = game.time + 10_000_000.0;
            level_defer_exit(game, until)?;
            start_finale_timer(game)?;
            return Ok(Q1FinaleDecision::Finale(finale(game, "$qc_finale_hipend2", 2)));
        }
    }
    Ok(Q1FinaleDecision::Delegate)
}

/// Present the current finale text through the shared journal (`endText`).
fn end_text(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if game.intermission.is_none() {
        let map = game.map_name.clone();
        let time = game.time;
        level_begin_cutscene(game, &map, None, time)?;
    }
    let result = level_advance_finale(game, game.time)?;
    let result = match result {
        Q1IntermissionResult::Finale { text, track } => Some(Q1SourceFinale::Finale { text, track }),
        Q1IntermissionResult::SellScreen => Some(Q1SourceFinale::SellScreen),
        _ => None,
    };
    if let Some(result) = result {
        with_missionpack_hooks(game, |_, hooks| {
            let present = hooks
                .present_finale
                .as_ref()
                .ok_or_else(|| q1_error("Hipnotic end text requires the shared finale journal"))?;
            present(&result);
            Ok(())
        })?;
    }
    Ok(())
}

/// Present finale text from a scheduled dispatch.
fn start_end_text_action(game: &mut Q1EntityServices, _id: &ActorId) -> Result<(), Q1Error> {
    end_text(game)
}

/// Present finale text from a use dispatch.
fn start_end_text_use(
    game: &mut Q1EntityServices,
    _id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    end_text(game)
}

/// Run the Hipnotic finale cutscene trigger.
fn effect_finale_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    _activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    if game
        .entity(id)
        .map(|entity| entity.number("finale_state"))
        .unwrap_or(0.0)
        == 1.0
    {
        return Ok(());
    }
    game.update_entity(id, |entity| number(entity, "finale_state", 1.0))?;
    let target = game.entity(id).map(|entity| entity.target.clone()).unwrap_or_default();
    let point = game
        .find(&target)
        .first()
        .cloned()
        .ok_or_else(|| q1_error("no target in finale"))?;
    game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 1,
    });
    let spawnflags = game.entity(id).map(|entity| entity.spawnflags).unwrap_or(0);
    if spawnflags & 2 == 0 {
        let mdl = game.entity(id).map(|entity| entity.text("mdl")).unwrap_or_default();
        let target = game
            .find(&mdl)
            .first()
            .cloned()
            .ok_or_else(|| q1_error("Hipnotic finale decoy path is missing"))?;
        let players = (game.host.players)();
        let first = players.first().cloned();
        let origin = if spawnflags & 1 != 0 {
            first
                .as_ref()
                .and_then(|player| game.host.bodies.read(player))
                .map(|body| body.origin)
                .unwrap_or(ZERO)
        } else {
            game.body(&target)?.origin
        };
        let target_name = game
            .entity(&target)
            .map(|entity| entity.target.clone())
            .unwrap_or_default();
        with_missionpack_hooks(game, |game, hooks| {
            let become_decoy = hooks
                .become_decoy
                .as_ref()
                .ok_or_else(|| q1_error("Hipnotic finale requires the source decoy controller"))?;
            become_decoy(game, &target_name, origin)?;
            Ok(())
        })?;
    }
    let (point_origin, point_mangle) = game
        .entity(&point)
        .map(|entity| (game.body(entity.actor.id()).map(|body| body.origin), entity.mangle))
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let point_origin = point_origin?;
    for player in (game.host.players)() {
        game.control_player(
            &player,
            &Q1CutsceneControl {
                origin: point_origin,
                angles: point_mangle,
                view_offset: ZERO,
            },
        )?;
    }
    let callback = game
        .entity(id)
        .map(|entity| entity.text("spawnfunction"))
        .unwrap_or_default();
    if !callback.is_empty() {
        game.update_entity(id, |entity| {
            entity.fields.insert("hip:finale_callback".to_string(), callback);
        })?;
        let wait = game.entity(id).map(|entity| entity.wait).unwrap_or(0.0);
        return later(game, id, wait, "hip:finale_callback");
    }
    Ok(())
}

/// Dispatch a stored finale map callback.
fn finale_callback(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let callback = game
        .entity(id)
        .map(|entity| entity.text("hip:finale_callback"))
        .unwrap_or_default();
    if callback == "info_startendtext_use" {
        return end_text(game);
    }
    if callback == "SUB_UseTargets" {
        let activator = game.entity(id).and_then(|entity| entity.activator.clone());
        return game.use_targets(id, activator.as_ref());
    }
    if callback == "SUB_Remove" {
        return game.remove(id);
    }
    if callback == "SUB_Null" {
        return Ok(());
    }
    let previous = game
        .entity(id)
        .map(|entity| entity.classname.clone())
        .unwrap_or_default();
    game.update_entity(id, |entity| entity.classname.clone_from(&callback))?;
    let result = game.spawn_entity(id, None);
    if game.entity(id).is_some() {
        game.update_entity(id, |entity| entity.classname = previous)?;
    }
    result
}

/// Spawn an `effect_finale`.
fn spawn_effect_finale(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if game.options().deathmatch != 0 {
        return game.remove(id);
    }
    let mangle = game.entity(id).map(|entity| entity.mangle).unwrap_or(ZERO);
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(mangle),
            ..Default::default()
        },
    )?;
    game.update_entity(id, |entity| number(entity, "finale_state", 0.0))?;
    let use_name = game.named.use_callback("hip:effect_finale")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Spawn an `info_startendtext`.
fn spawn_startendtext(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let use_name = game.named.use_callback("hip:start_end_text")?;
    game.update_entity(id, |entity| entity.use_callback = Some(use_name))
}

/// Register mission-pack campaign finales (`registerMissionCampaign`).
pub fn register_mission_campaign(game: &mut Q1EntityServices, pack: Q1MissionPack) -> Result<(), Q1Error> {
    game.named.register(
        "mission:finale_transition",
        Q1CallbackHandlers {
            action: Some(finale_transition),
            ..Default::default()
        },
    )?;
    game.named.register(
        "mission:finale_check",
        Q1CallbackHandlers {
            action: Some(finale_check),
            ..Default::default()
        },
    )?;
    level_register_rule(
        game,
        Q1IntermissionRule {
            id: format!("q1:{}:campaign", pack.as_str()),
            touch: None,
            begin: None,
            finale: Some(if pack == Q1MissionPack::Rogue {
                rogue_campaign_finale
            } else {
                hipnotic_campaign_finale
            }),
            travel: None,
        },
    )?;
    if pack != Q1MissionPack::Hipnotic {
        return Ok(());
    }
    game.named.register(
        "hip:start_end_text",
        Q1CallbackHandlers {
            action: Some(start_end_text_action),
            use_callback: Some(start_end_text_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:effect_finale",
        Q1CallbackHandlers {
            use_callback: Some(effect_finale_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "hip:finale_callback",
        Q1CallbackHandlers {
            action: Some(finale_callback),
            ..Default::default()
        },
    )?;
    game.register_spawn("effect_finale", spawn_effect_finale)?;
    game.register_spawn("info_startendtext", spawn_startendtext)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::base::provider::{dismiss_finale, Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn game_with_base(pack: Q1MissionPack) -> (Box<Q1EntityServices>, Q1BaseGuard) {
        let mut game = Box::new(test_game());
        let guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        register_mission_campaign(&mut game, pack).expect("campaign");
        (game, guard)
    }

    #[test]
    fn finale_check_waits_for_dismissal_then_transitions() {
        let (mut game, _guard) = game_with_base(Q1MissionPack::Hipnotic);
        let id = game.create("mission_finale_timer", None, None).expect("timer");
        game.invoke_action(&id, "mission:finale_check").expect("check");
        assert_eq!(
            game.entity(&id).expect("timer").think.as_deref(),
            Some("mission:finale_check")
        );
        dismiss_finale(&game).expect("dismiss");
        game.invoke_action(&id, "mission:finale_check").expect("check");
        assert_eq!(
            game.entity(&id).expect("timer").think.as_deref(),
            Some("mission:finale_transition")
        );
    }

    #[test]
    fn rogue_r1_finale_returns_overlord_text() {
        let (mut game, _guard) = game_with_base(Q1MissionPack::Rogue);
        game.map_name = "r1m7".to_string();
        let decision = rogue_campaign_finale(&mut game, 2, "").expect("finale");
        match decision {
            Q1FinaleDecision::Finale(Q1SourceFinale::Finale { text, track }) => {
                assert!(text.contains("Victory!"), "{text}");
                assert_eq!(track, 3);
            }
            other => panic!("unexpected {other:?}"),
        }
        game.map_name = "r2m1".to_string();
        assert_eq!(
            rogue_campaign_finale(&mut game, 2, "").expect("delegate"),
            Q1FinaleDecision::Delegate
        );
    }

    #[test]
    fn hipnotic_stage_three_returns_secret_level_text() {
        let (mut game, _guard) = game_with_base(Q1MissionPack::Hipnotic);
        game.map_name = "hip1m4".to_string();
        let decision = hipnotic_campaign_finale(&mut game, 2, "").expect("finale");
        match decision {
            Q1FinaleDecision::Finale(Q1SourceFinale::Finale { text, track }) => {
                assert!(text.contains("Research Facility"), "{text}");
                assert_eq!(track, 6);
            }
            other => panic!("unexpected {other:?}"),
        }
        let decision = hipnotic_campaign_finale(&mut game, 3, "").expect("secret");
        match decision {
            Q1FinaleDecision::Finale(Q1SourceFinale::Finale { text, track }) => {
                assert!(text.contains("portal"), "{text}");
                assert_eq!(track, 6);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn effect_finale_spawn_arms_trigger() {
        let (mut game, _guard) = game_with_base(Q1MissionPack::Hipnotic);
        let id = game.create("effect_finale", None, None).expect("finale");
        game.spawn_entity(&id, None).expect("spawn");
        assert_eq!(
            game.entity(&id).expect("finale").use_callback.as_deref(),
            Some("hip:effect_finale")
        );
    }
}
