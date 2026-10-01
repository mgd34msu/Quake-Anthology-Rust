//! Q2 grapple services (`src/content/q2/equipment/grapple-services.ts`).

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use crate::q2::foundation::checkpoint::save_q2_actor;
use crate::q2::foundation::host::Q2GameServices;
use crate::q2::support::contracts::BodyState;

/// Grapple hand (`GrapplePose["hand"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleHand {
    /// Left.
    Left,
    /// Center.
    Center,
    /// Right.
    Right,
}

/// Grapple pose (`GrapplePose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrapplePose {
    /// Aim angles.
    pub angles: Vec3,
    /// Hand.
    pub hand: GrappleHand,
    /// View height.
    pub view_height: f64,
    /// Gravity scale.
    pub gravity: f64,
    /// Gravity vector.
    pub gravity_vector: Vec3,
}

/// Grapple anchor kind (`GrappleAnchor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleAnchor {
    /// No anchor.
    None,
    /// Box anchor.
    Box,
    /// Brush anchor.
    Brush,
    /// World anchor.
    World,
    /// Player anchor.
    Player,
    /// Corpse anchor.
    Corpse,
}

/// Grapple noise kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrappleNoise {
    /// Weapon noise.
    Weapon,
    /// Impact noise.
    Impact,
}

/// Grapple cable event (`emit` payload).
#[derive(Debug, Clone, PartialEq)]
pub struct GrappleCableEvent {
    /// Owning actor.
    pub actor: ActorId,
    /// Cable start.
    pub start: Vec3,
    /// Cable end.
    pub end: Vec3,
    /// Start offset from the owner origin.
    pub offset: Vec3,
}

/// Grapple hooks (`GrappleHooks`).
#[derive(Debug, Clone, Copy)]
pub struct GrappleHooks {
    /// Read the firing pose.
    pub pose: fn(ActorId, &mut Q2GameServices) -> GrapplePose,
    /// Read the anchor kind.
    pub anchor: fn(ActorId, &mut Q2GameServices) -> GrappleAnchor,
    /// Whether an actor is dead.
    pub dead: fn(ActorId, &mut Q2GameServices) -> bool,
    /// Read the previous velocity.
    pub previous_velocity: fn(ActorId, &mut Q2GameServices) -> Vec3,
    /// Write the previous velocity.
    pub set_previous_velocity: fn(ActorId, Vec3, &mut Q2GameServices),
    /// Read the grapple volume.
    pub volume: fn(ActorId, &mut Q2GameServices) -> f64,
    /// Emit a grapple noise.
    pub noise: fn(ActorId, &mut Q2GameServices, Vec3, GrappleNoise),
    /// Suppress or restore grapple prediction.
    pub set_grapple_prediction: fn(ActorId, bool, &mut Q2GameServices),
    /// Read gravity.
    pub gravity: fn(&mut Q2GameServices) -> f64,
    /// Emit a grapple cable.
    pub emit: fn(GrappleCableEvent, &mut Q2GameServices),
}

/// Read the grapple owner body (`grappleBody`).
pub fn grapple_body(actor: ActorId, game: &mut Q2GameServices) -> BodyState {
    game.host
        .bodies()
        .read(&actor)
        .unwrap_or_else(|| panic!("Grapple owner requires an existing shared body"))
}

/// Write the grapple owner velocity (`grappleVelocity`).
pub fn grapple_velocity(actor: ActorId, game: &mut Q2GameServices, velocity: Vec3) {
    let Some(owned) = game.host.actors().resolve_owned(&actor) else {
        return;
    };
    let mut body = grapple_body(actor, game);
    body.velocity = velocity;
    game.host.bodies().write(&owned, &body);
    game.host.bodies().link(&owned, None);
}

/// CTF grapple state (`CtfGrappleState`).
#[derive(Debug, Clone, PartialEq)]
pub struct CtfGrappleState {
    /// Hook actor.
    pub grapple: Option<ActorId>,
    /// Grapple state.
    pub grapple_state: CtfGrapplePhase,
    /// Release time.
    pub grapple_release_time: f64,
    /// Saved knockback immunity.
    pub grapple_no_knockback: Option<bool>,
}

impl Default for CtfGrappleState {
    fn default() -> Self {
        CtfGrappleState {
            grapple: None,
            grapple_state: CtfGrapplePhase::Fly,
            grapple_release_time: 0.0,
            grapple_no_knockback: None,
        }
    }
}

/// CTF grapple phase (`CtfGrappleState["grappleState"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CtfGrapplePhase {
    /// Flying.
    Fly,
    /// Pulling.
    Pull,
    /// Hanging.
    Hang,
}

/// LMCTF grapple state (`LmctfGrappleState`).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfGrappleState {
    /// Hook actor.
    pub hook: Option<ActorId>,
    /// Hook state.
    pub hook_state: i32,
    /// Hook length.
    pub hook_length: f64,
    /// Whether the hook is held.
    pub hook_held: bool,
}

impl Default for LmctfGrappleState {
    fn default() -> Self {
        LmctfGrappleState {
            hook: None,
            hook_state: 0,
            hook_length: 0.0,
            hook_held: false,
        }
    }
}

/// CTF grapple checkpoint (`captureCtfGrapple` result).
#[derive(Debug, Clone, PartialEq)]
pub struct CtfGrappleCheckpoint {
    /// Hook actor.
    pub grapple: Option<SavedActorId>,
    /// Grapple state.
    pub grapple_state: CtfGrapplePhase,
    /// Release time.
    pub grapple_release_time: f64,
    /// Saved knockback immunity.
    pub grapple_no_knockback: Option<bool>,
}

/// LMCTF grapple checkpoint (`captureLmctfGrapple` result).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfGrappleCheckpoint {
    /// Hook actor.
    pub hook: Option<SavedActorId>,
    /// Hook state.
    pub hook_state: i32,
    /// Hook length.
    pub hook_length: f64,
    /// Whether the hook is held.
    pub hook_held: bool,
}

/// Capture CTF grapple state (`captureCtfGrapple`).
pub fn capture_ctf_grapple(state: &CtfGrappleState) -> CtfGrappleCheckpoint {
    CtfGrappleCheckpoint {
        grapple: save_q2_actor(state.grapple.as_ref()),
        grapple_state: state.grapple_state,
        grapple_release_time: state.grapple_release_time,
        grapple_no_knockback: state.grapple_no_knockback,
    }
}

/// Capture LMCTF grapple state (`captureLmctfGrapple`).
pub fn capture_lmctf_grapple(state: &LmctfGrappleState) -> LmctfGrappleCheckpoint {
    LmctfGrappleCheckpoint {
        hook: save_q2_actor(state.hook.as_ref()),
        hook_state: state.hook_state,
        hook_length: state.hook_length,
        hook_held: state.hook_held,
    }
}

/// Restore CTF grapple state (`restoreCtfGrapple`).
pub fn restore_ctf_grapple(saved: CtfGrappleCheckpoint, game: &mut Q2GameServices) -> CtfGrappleState {
    CtfGrappleState {
        grapple: saved.grapple.map(|hook| game.host.actors().reference_saved(hook)),
        grapple_state: saved.grapple_state,
        grapple_release_time: saved.grapple_release_time,
        grapple_no_knockback: saved.grapple_no_knockback,
    }
}

/// Restore LMCTF grapple state (`restoreLmctfGrapple`).
pub fn restore_lmctf_grapple(saved: LmctfGrappleCheckpoint, game: &mut Q2GameServices) -> LmctfGrappleState {
    LmctfGrappleState {
        hook: saved.hook.map(|hook| game.host.actors().reference_saved(hook)),
        hook_state: saved.hook_state,
        hook_length: saved.hook_length,
        hook_held: saved.hook_held,
    }
}
