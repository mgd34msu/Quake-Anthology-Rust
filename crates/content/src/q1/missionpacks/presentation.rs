//! Mission-pack presentation (src/content/q1/missionpacks/presentation.ts).

use std::rc::Rc;

use qa_core::identity::{ActorId, same_actor};

use crate::q1::Q1Error;
use crate::q1::base::player::{
    Q1CharacterDefinition, Q1CharacterFrameRange, Q1CharacterPresentation, Q1CharacterSourcePose,
    Q1PlayerLife,
};
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Event, Q1SoundChannel, Q1Weapon, length, vsub};

use super::types::{Q1MissionPack, fround};

/// Mjolnir character layout (`hammer`).
fn hammer_definition() -> Q1CharacterDefinition {
    Q1CharacterDefinition {
        model: String::from("progs/playham.mdl"),
        stand: Q1CharacterFrameRange {
            first: 6.0,
            count: 12.0,
        },
        run: Q1CharacterFrameRange {
            first: 0.0,
            count: 6.0,
        },
        pain: Q1CharacterFrameRange {
            first: 18.0,
            count: 6.0,
        },
        death: Q1CharacterFrameRange {
            first: 24.0,
            count: 8.0,
        },
    }
}

/// Mission-pack character pose (`missionPackCharacterPose`).
pub fn mission_pack_character_pose(
    game: &Q1EntityServices,
    actor: &ActorId,
    pack: Q1MissionPack,
) -> Q1CharacterSourcePose {
    let player = match game.player_ref(actor) {
        Some(player) => player.clone(),
        None => return Q1CharacterSourcePose::default(),
    };
    let elapsed = if player.weapon_animation_at < 0.0 {
        -1
    } else {
        ((game.time - player.weapon_animation_at) / 0.1).floor() as i64
    };
    let attacking = (0..6).contains(&elapsed);
    if pack == Q1MissionPack::Hipnotic && player.weapon == Q1Weapon::HipnoticMjolnir {
        return Q1CharacterSourcePose {
            definition: Some(hammer_definition()),
            frame: if attacking {
                Some((player.weapon_animation_base as i64 + elapsed) as f64)
            } else {
                None
            },
            ..Default::default()
        };
    }
    if player.continuous_firing {
        if player.weapon == Q1Weapon::HipnoticLaser {
            return Q1CharacterSourcePose {
                frame: Some(if player.weapon_frame == 1 {
                    103.0
                } else {
                    104.0
                }),
                ..Default::default()
            };
        }
        if matches!(
            player.weapon,
            Q1Weapon::RogueLavaNailgun | Q1Weapon::RogueLavaSupernailgun
        ) {
            return Q1CharacterSourcePose {
                frame: Some(if player.weapon_frame % 2 == 1 {
                    103.0
                } else {
                    104.0
                }),
                ..Default::default()
            };
        }
    }
    if attacking
        && matches!(
            player.weapon,
            Q1Weapon::HipnoticProximity | Q1Weapon::RogueMultiGrenade | Q1Weapon::RogueMultiRocket
        )
    {
        return Q1CharacterSourcePose {
            frame: Some((107 + elapsed) as f64),
            ..Default::default()
        };
    }
    Q1CharacterSourcePose::default()
}

/// Head flies timer (`hipnotic:head-flies`).
fn head_flies(game: &mut Q1EntityServices, timer: &ActorId) -> Result<(), Q1Error> {
    let owner = game
        .entity_ref(timer)
        .and_then(|timer| timer.owner.clone())
        .and_then(|owner| game.host.actors.resolve_owned(&owner));
    if let Some(owner) = owner {
        let worldtype = game
            .world
            .as_ref()
            .and_then(|world| game.entity_ref(world))
            .map(|world| world.number("worldtype"))
            .unwrap_or(0.0);
        if game.health(owner.id()) <= 0.0 && worldtype != 2.0 && game.host.random() < 0.1 {
            game.host.emit(Q1Event::Sound {
                origin: None,
                actor: owner.id().clone(),
                path: String::from("misc/flys.wav"),
                channel: Q1SoundChannel::Raw(6),
                attenuation: 3.0,
                volume: 0.7,
            });
        }
    }
    game.remove(timer)
}

/// Mission-pack character effects (`MissionPackCharacterEffects`).
pub struct MissionPackCharacterEffects {
    /// Mission pack.
    pub pack: Q1MissionPack,
    /// Footstep probe.
    footsteps: Rc<dyn Fn() -> bool>,
}

impl MissionPackCharacterEffects {
    /// Register mission-pack character effects on a game.
    pub fn new(
        game: &mut Q1EntityServices,
        pack: Q1MissionPack,
        footsteps: Rc<dyn Fn() -> bool>,
    ) -> Result<Self, Q1Error> {
        if pack == Q1MissionPack::Hipnotic {
            game.named.register(
                "hipnotic:head-flies",
                Q1CallbackHandlers {
                    action: Some(head_flies),
                    ..Default::default()
                },
            )?;
        }
        Ok(Self { pack, footsteps })
    }

    /// Find or create a character state entity (`state`).
    fn state_entity(game: &mut Q1EntityServices, actor: &ActorId) -> Result<ActorId, Q1Error> {
        if let Some(existing) = game
            .entities
            .values()
            .find(|entity| {
                entity.classname == "missionpack_character_state"
                    && entity
                        .owner
                        .as_ref()
                        .is_some_and(|owner| same_actor(owner, actor))
            })
            .map(|entity| entity.actor.id().clone())
        {
            return Ok(existing);
        }
        let entity = game.create("missionpack_character_state", None, None)?;
        let owner = actor.clone();
        game.update_entity(&entity, |entity| entity.owner = Some(owner))?;
        Ok(entity)
    }

    /// Run mission-pack character effects (`frame`).
    pub fn frame(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        presentation: &Q1CharacterPresentation,
    ) -> Result<(), Q1Error> {
        if self.pack != Q1MissionPack::Hipnotic {
            return Ok(());
        }
        let state_entity = Self::state_entity(game, actor)?;
        let body = match game.host.bodies.read(actor) {
            Some(body) => body,
            None => return Ok(()),
        };
        let model = game
            .entity_ref(&state_entity)
            .map(|entity| entity.text("model"))
            .unwrap_or_default();
        if presentation.model == "progs/h_player.mdl" && model != presentation.model {
            let timer = game.create("hipnotic_head_flies", None, None)?;
            let owner = actor.clone();
            game.update_entity(&timer, |timer| timer.owner = Some(owner))?;
            let flies = game.named.action("hipnotic:head-flies")?;
            game.schedule(&timer, 1.5, &flies)?;
        }
        game.update_entity(&state_entity, |entity| {
            entity
                .fields
                .insert("model".to_string(), presentation.model.clone());
        })?;
        let player = game.player_ref(actor).cloned();
        let locomotion = if presentation.model == "progs/playham.mdl" {
            presentation.frame < 18.0
        } else {
            presentation.frame < 29.0
        };
        let settled = player
            .as_ref()
            .map(|player| player.weapon_animation_at)
            .unwrap_or(f64::NAN);
        if !(self.footsteps)()
            || presentation.life != Q1PlayerLife::Alive
            || !locomotion
            || body.velocity.x == 0.0 && body.velocity.y == 0.0
            || game.time
                < game
                    .entity_ref(&state_entity)
                    .map(|entity| entity.number("next-step"))
                    .unwrap_or(0.0)
            || settled != -1.0
            || player
                .as_ref()
                .is_some_and(|player| player.continuous_firing)
        {
            return Ok(());
        }
        game.update_entity(&state_entity, |entity| {
            entity
                .fields
                .insert("next-step".to_string(), fround(game.time + 0.1).to_string());
        })?;
        let old_origin = game
            .entity_ref(&state_entity)
            .map(|entity| entity.vector("old-origin"))
            .unwrap_or(body.origin);
        let mut distance = fround(
            game.entity_ref(&state_entity)
                .map(|entity| entity.number("distance"))
                .unwrap_or(0.0)
                + f64::from(length(vsub(body.origin, old_origin))),
        );
        game.update_entity(&state_entity, |entity| {
            entity.fields.insert(
                "old-origin".to_string(),
                format!("{} {} {}", body.origin.x, body.origin.y, body.origin.z),
            );
        })?;
        if body.ground.is_some() && distance > 95.0 {
            distance = if distance > 190.0 {
                0.0
            } else {
                fround(0.5 * (distance - 95.0))
            };
            let roll = game.host.random();
            let step = if roll < 0.14 {
                1
            } else if roll < 0.29 {
                2
            } else if roll < 0.43 {
                3
            } else if roll < 0.58 {
                4
            } else if roll < 0.72 {
                5
            } else if roll < 0.86 {
                6
            } else {
                7
            };
            game.host.emit(Q1Event::Sound {
                origin: None,
                actor: actor.clone(),
                path: format!("misc/foot{step}.wav"),
                channel: Q1SoundChannel::Voice,
                attenuation: 1.0,
                volume: 0.5,
            });
        }
        game.update_entity(&state_entity, |entity| {
            entity
                .fields
                .insert("distance".to_string(), distance.to_string());
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::types::{Q1MoveType, Q1Solid, ZERO};

    fn attached_player(game: &mut Q1EntityServices) -> ActorId {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default())
            .expect("attach");
        player
    }

    fn presentation() -> Q1CharacterPresentation {
        Q1CharacterPresentation {
            model: String::from("progs/player.mdl"),
            frame: 0.0,
            view_offset: ZERO,
            life: Q1PlayerLife::Alive,
            solid: Q1Solid::Slidebox,
            movement: Q1MoveType::Step,
            weapon_visible: true,
        }
    }

    #[test]
    fn poses_match_donor_branches() {
        let mut game = test_game();
        let player = attached_player(&mut game);
        game.update_player(&player, |state| {
            state.weapon = Q1Weapon::HipnoticLaser;
            state.continuous_firing = true;
            state.weapon_frame = 1;
        })
        .expect("laser");
        let pose = mission_pack_character_pose(&game, &player, Q1MissionPack::Hipnotic);
        assert_eq!(pose.frame, Some(103.0));
        game.update_player(&player, |state| {
            state.weapon = Q1Weapon::HipnoticProximity;
            state.continuous_firing = false;
            state.weapon_animation_at = game.time;
        })
        .expect("proximity");
        let pose = mission_pack_character_pose(&game, &player, Q1MissionPack::Hipnotic);
        assert_eq!(pose.frame, Some(107.0));
        let stranger = game.create("player", None, None).expect("stranger");
        assert_eq!(
            mission_pack_character_pose(&game, &stranger, Q1MissionPack::Rogue).frame,
            None
        );
    }

    #[test]
    fn mjolnir_pose_uses_hammer_layout() {
        let mut game = test_game();
        let player = attached_player(&mut game);
        game.update_player(&player, |state| {
            state.weapon = Q1Weapon::HipnoticMjolnir;
            state.weapon_animation_at = game.time;
            state.weapon_animation_base = 32;
        })
        .expect("mjolnir");
        let pose = mission_pack_character_pose(&game, &player, Q1MissionPack::Hipnotic);
        assert_eq!(
            pose.definition.expect("definition").model,
            "progs/playham.mdl"
        );
        assert_eq!(pose.frame, Some(32.0));
    }

    #[test]
    fn effects_register_and_rest_without_footsteps() {
        let mut game = test_game();
        let effects =
            MissionPackCharacterEffects::new(&mut game, Q1MissionPack::Hipnotic, Rc::new(|| false))
                .expect("effects");
        assert!(game.named.action("hipnotic:head-flies").is_ok());
        let player = attached_player(&mut game);
        effects
            .frame(&mut game, &player, &presentation())
            .expect("frame");
        let rogue =
            MissionPackCharacterEffects::new(&mut game, Q1MissionPack::Rogue, Rc::new(|| true))
                .expect("rogue");
        rogue
            .frame(&mut game, &player, &presentation())
            .expect("frame");
    }
}
