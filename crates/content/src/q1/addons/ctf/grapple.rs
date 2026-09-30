//! Q1 CTF native Threewave activation and team rules (src/content/q1/addons/ctf/grapple.ts).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::addons::ctf::state::{
    ctf_by_key, ctf_key, ctf_teamplay_bits, native_grapple_enabled, with_ctf_services,
};
use crate::q1::addons::ctf::types::CtfFlags;
use crate::q1::equipment::grapple::{
    grapple_fire, grapple_release, grapple_trail as equipment_grapple_trail, register_threewave_grapple,
    ThreewaveAnchor, ThreewaveGrappleHost, ThreewaveGrappleInput,
};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::ZERO;
use crate::q1::Q1Error;

/// CTF grapple host policy (`createGrapple` closures). The equipment
/// host runs without game access, so team and player answers come from
/// the synced CTF policy while input stays live through the session
/// services in the registry.
pub(crate) struct CtfGrappleHost {
    /// Registry key of the owning game.
    pub game_key: usize,
}

impl ThreewaveGrappleHost for CtfGrappleHost {
    fn input(&self, actor: &ActorId) -> ThreewaveGrappleInput {
        ctf_by_key(self.game_key, |state| {
            let input = state.services.input(actor);
            ThreewaveGrappleInput {
                held: input.attack,
                release: !input.attack && input.grapple_selected,
                jump: input.jump,
                view_angles: input.view_angles,
                teleport_until: input.teleport_until,
            }
        })
        .unwrap_or(ThreewaveGrappleInput {
            held: false,
            release: false,
            jump: false,
            view_angles: ZERO,
            teleport_until: 0.0,
        })
    }

    fn aim(&self, _actor: &ActorId, forward: Vec3) -> Vec3 {
        // The equipment host has no game access for PF_aim traces, so
        // the hook fires along the view forward.
        forward
    }

    fn anchor(&self, actor: &ActorId) -> ThreewaveAnchor {
        // The equipment host cannot read entity solidity, so players
        // anchor centered while every other target anchors solid and
        // uncentered, matching world, door and brush hooks.
        let player = ctf_by_key(self.game_key, |state| state.policy.players.contains(actor)).unwrap_or(false);
        ThreewaveAnchor {
            solid: true,
            centered: player,
            player,
        }
    }

    fn can_attach(&self, owner: &ActorId, target: &ActorId) -> bool {
        ctf_by_key(self.game_key, |state| {
            let policy = &state.policy;
            !policy.players.contains(target)
                || policy.teamplay_raw == 0.0
                || policy.teams.get(target).copied().flatten() != policy.lastteam.get(owner).copied().flatten()
        })
        .unwrap_or(true)
    }

    fn can_pulse(&self, owner: &ActorId, target: &ActorId) -> bool {
        ctf_by_key(self.game_key, |state| {
            let policy = &state.policy;
            !policy.players.contains(target)
                || policy.teamplay_raw == 0.0
                || policy.lastteam.get(target).copied().flatten() != policy.lastteam.get(owner).copied().flatten()
        })
        .unwrap_or(true)
    }

    fn can_damage(&self, _target: &ActorId, _owner: &ActorId) -> bool {
        // The engine trace-based damage check needs the game; the
        // equipment already gates pulses on `can_take_damage`.
        true
    }
}

/// Register the native grapple service (`createGrapple`). Mechanics
/// live in the shared-actor equipment; this only wires CTF policy.
pub fn register_ctf_grapple(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    if !native_grapple_enabled(game)? {
        return Ok(());
    }
    register_threewave_grapple(
        game,
        Box::new(CtfGrappleHost {
            game_key: ctf_key(game),
        }),
    )
}

/// Release an owner's hook (`unhook`). Without the native grapple the
/// foreign session owns hook release.
pub fn unhook(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    if native_grapple_enabled(game)? {
        grapple_release(game, actor)?;
    }
    Ok(())
}

/// Dispatch a hook touch through the stored callback (`hookTouch`).
pub fn hook_touch(game: &mut Q1EntityServices, hook: &ActorId, other: &ActorId) -> Result<(), Q1Error> {
    if native_grapple_enabled(game)? {
        game.invoke_touch(hook, other, None, None)?;
    }
    Ok(())
}

/// Fire an owner's hook unless teamplay or observer mode forbids it
/// (`fireHook`).
pub fn fire_hook(game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    if ctf_teamplay_bits(game)? & CtfFlags::DISABLE_GRAPPLE != 0
        || with_ctf_services(game, |services| services.observer(actor))?
    {
        return Ok(false);
    }
    if !native_grapple_enabled(game)? {
        return Ok(false);
    }
    grapple_fire(game, actor)
}

/// Emit the rerelease beam trail (`grappleTrail`).
pub fn grapple_trail(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    if native_grapple_enabled(game)? {
        equipment_grapple_trail(game, actor);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{addon_set_cvar, attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::state::register_ctf_state;
    use crate::q1::addons::ctf::types::FakeCtfServices;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices, native: bool) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), native, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn fire_hook_honors_teamplay_and_observers() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, true);
        register_ctf_grapple(&mut game).expect("grapple");
        assert_eq!(fire_hook(&mut game, &player), Ok(true));
        addon_set_cvar(&game, "teamplay", "2048").expect("cvar");
        assert_eq!(fire_hook(&mut game, &player), Ok(false));
        addon_set_cvar(&game, "teamplay", "0").expect("cvar");
        with_ctf_services(&game, |services| services.set_observer(&player, true)).expect("observer");
        assert_eq!(fire_hook(&mut game, &player), Ok(false));
    }

    #[test]
    fn unhook_is_quiet_without_native_grapple() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game, false);
        register_ctf_grapple(&mut game).expect("grapple");
        assert_eq!(fire_hook(&mut game, &player), Ok(false));
        unhook(&mut game, &player).expect("unhook");
        grapple_trail(&mut game, &player).expect("trail");
    }
}
