//! Threewave weapon adapter (`src/content/q1/equipment/threewave-weapon.ts`).
//!
//! The weapon adapter drives the grapple service through the shared
//! attack/animation/holster flow. Hooks (held by the game-owned
//! service so the launch callback can reach them) observe or override
//! launches; selection owns everything else.

use qa_core::identity::ActorId;

use super::grapple::{grapple_fire, grapple_hook, grapple_release, grapple_state};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::{q1_error, Q1Error};

/// Weapon launch hooks (`ThreewaveWeaponHooks`).
pub trait ThreewaveWeaponHooks {
    /// Observe or replace a launch. Returning a value fires the hook.
    fn launch(&mut self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<Option<bool>, Q1Error>;
    /// Observe the post-attack animation selection.
    fn animated(&mut self, game: &mut Q1EntityServices, actor: &ActorId, frame: i32) -> Result<(), Q1Error>;
}

/// Game-owned weapon service state.
pub struct ThreewaveWeaponService {
    /// Weapon hooks.
    pub hooks: Box<dyn ThreewaveWeaponHooks>,
}

/// Character pose contribution (`threewaveCharacterPose`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreewaveCharacterPose {
    /// Pose uses the axe animation.
    pub axe_pose: bool,
    /// Override frame, if any.
    pub frame: Option<i32>,
}

/// Attack with the grapple (`ThreewaveWeapon.attack`).
pub fn weapon_attack(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    let id = actor.clone();
    let attack_finished = game.time + 0.5;
    {
        let state = grapple_state(game, &id)?;
        state.weapon_frame = 1;
        state.attack_finished = attack_finished;
    }
    if game.threewave_weapon.is_none() {
        return Err(q1_error("Q1 threewave weapon was not registered"));
    }
    let mut hooks = game.threewave_weapon.take().expect("weapon hooks");
    let launch = hooks.hooks.launch(game, &id);
    game.threewave_weapon = Some(hooks);
    let fired = match launch? {
        Some(fired) => fired,
        None => grapple_fire(game, &id)?,
    };
    if !fired {
        return Ok(false);
    }
    let release_time = game.time + 1.0;
    let state = grapple_state(game, &id)?;
    state.weapon_frame = 2;
    state.release_time = release_time;
    Ok(true)
}

/// Advance the attack animation (`ThreewaveWeapon.animate`).
pub fn weapon_animate(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let id = actor.clone();
    let frame = {
        let state = grapple_state(game, &id)?;
        if state.weapon_frame == 0 || state.weapon_frame >= 5 {
            state.weapon_frame = 0;
            return Ok(());
        }
        state.weapon_frame += 1;
        state.weapon_frame
    };
    if game.threewave_weapon.is_none() {
        return Err(q1_error("Q1 threewave weapon was not registered"));
    }
    let mut hooks = game.threewave_weapon.take().expect("weapon hooks");
    let animated = hooks.hooks.animated(game, &id, frame);
    game.threewave_weapon = Some(hooks);
    animated?;
    Ok(())
}

/// Whether the weapon is holstered (`ThreewaveWeapon.isHolstered`).
#[must_use]
pub fn weapon_is_holstered() -> bool {
    true
}

/// Holster the grapple (`ThreewaveWeapon.holster`).
pub fn weapon_holster(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    grapple_release(game, actor)?;
    grapple_state(game, actor)?.weapon_frame = 0;
    Ok(())
}

/// Resume after a holster (`ThreewaveWeapon.resume`).
pub fn weapon_resume(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let state = grapple_state(game, actor)?;
    state.animation = None;
    Ok(())
}

/// Character pose contribution (`threewaveCharacterPose`).
pub fn threewave_character_pose(
    game: &mut Q1EntityServices,
    actor: &ActorId,
) -> Result<ThreewaveCharacterPose, Q1Error> {
    if grapple_hook(game, actor).is_none() {
        return Ok(ThreewaveCharacterPose {
            axe_pose: true,
            frame: None,
        });
    }
    {
        let state = grapple_state(game, actor)?;
        state.weapon_frame = 1;
        state.animation = Some(actor.clone());
    }
    if let Ok(action) = game.named.action("threewave:grapple_launch") {
        game.invoke_action(actor, &action)?;
    }
    Ok(ThreewaveCharacterPose {
        axe_pose: true,
        frame: Some(76),
    })
}

/// Register the weapon service and launch callback.
pub fn register_threewave_weapon(
    game: &mut Q1EntityServices,
    hooks: Box<dyn ThreewaveWeaponHooks>,
) -> Result<(), Q1Error> {
    game.threewave_weapon = Some(ThreewaveWeaponService { hooks });
    game.named.register(
        "threewave:grapple_launch",
        crate::q1::foundation::callbacks::Q1CallbackHandlers {
            action: Some(|game: &mut Q1EntityServices, id: &ActorId| weapon_launch(game, id).map(|_| ())),
            ..Default::default()
        },
    )
}

fn weapon_launch(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    let id = actor.clone();
    let animation = game
        .threewave_grapple
        .as_ref()
        .and_then(|service| service.states.get(&id))
        .and_then(|state| state.animation.clone());
    if animation.as_ref() != Some(&id) {
        return Ok(false);
    }
    if game.threewave_weapon.is_none() {
        return Err(q1_error("Q1 threewave weapon was not registered"));
    }
    let mut hooks = game.threewave_weapon.take().expect("weapon hooks");
    let launch = hooks.hooks.launch(game, &id);
    game.threewave_weapon = Some(hooks);
    match launch? {
        Some(fired) => Ok(fired),
        None => weapon_attack(game, &id),
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::super::super::foundation::host::mock::{mock_host, MockEvents};
    use super::super::super::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram, ZERO};
    use super::super::grapple::{
        register_threewave_grapple, ThreewaveAnchor, ThreewaveGrappleHost, ThreewaveGrappleInput,
    };
    use super::*;
    use qa_core::math::Vec3;

    struct TestHost;

    impl ThreewaveGrappleHost for TestHost {
        fn input(&self, _actor: &ActorId) -> ThreewaveGrappleInput {
            ThreewaveGrappleInput {
                held: true,
                release: false,
                jump: false,
                view_angles: ZERO,
                teleport_until: 0.0,
            }
        }

        fn aim(&self, _actor: &ActorId, forward: Vec3) -> Vec3 {
            forward
        }

        fn anchor(&self, _actor: &ActorId) -> ThreewaveAnchor {
            ThreewaveAnchor {
                solid: true,
                centered: false,
                player: false,
            }
        }

        fn can_attach(&self, _owner: &ActorId, _target: &ActorId) -> bool {
            true
        }

        fn can_pulse(&self, _owner: &ActorId, _target: &ActorId) -> bool {
            true
        }

        fn can_damage(&self, _target: &ActorId, _owner: &ActorId) -> bool {
            true
        }
    }

    struct TestHooks;

    impl ThreewaveWeaponHooks for TestHooks {
        fn launch(&mut self, _game: &mut Q1EntityServices, _actor: &ActorId) -> Result<Option<bool>, Q1Error> {
            Ok(None)
        }

        fn animated(&mut self, _game: &mut Q1EntityServices, _actor: &ActorId, _frame: i32) -> Result<(), Q1Error> {
            Ok(())
        }
    }

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn game() -> (Q1EntityServices, std::rc::Rc<std::cell::RefCell<MockEvents>>) {
        let (host, events) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        register_threewave_grapple(&mut game, Box::new(TestHost)).expect("register");
        register_threewave_weapon(&mut game, Box::new(TestHooks)).expect("register");
        (game, events)
    }

    #[test]
    fn attack_animates_and_holsters() {
        let (mut game, _) = game();
        let player = game.create("player", None, None).expect("player");
        game.set_health(&player, 100.0).expect("health");
        assert!(weapon_attack(&mut game, &player).expect("attack"));
        assert!(weapon_is_holstered());
        weapon_animate(&mut game, &player).expect("animate");
        weapon_holster(&mut game, &player).expect("holster");
        weapon_resume(&mut game, &player).expect("resume");
        let pose = threewave_character_pose(&mut game, &player).expect("pose");
        assert_eq!(
            pose,
            ThreewaveCharacterPose {
                axe_pose: true,
                frame: None
            }
        );
    }
}
