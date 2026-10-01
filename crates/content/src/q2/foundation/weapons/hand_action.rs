//! Hand grenade action (`src/content/q2/foundation/weapons/hand-action.ts`).
//!
//! Quake II hand grenade source timing adapted to an independent input
//! action (id Software, GPL-2.0-or-later).

use qa_core::math::Vec3;

use super::hand_grenade::{
    calculate_hand_throw, hand_deadline, hand_frame_seconds, hand_fuse_deadline, hand_recovery_seconds,
    HandGrenadeTempo, HandProjectileSpec, HandThrowInput,
};
use crate::q2::foundation::host::Q2Edition;

/// Hand action state (`HandAction`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HandAction {
    /// Idle.
    Idle,
    /// Disarmed.
    Disarmed,
    /// Preparing.
    Preparing {
        /// Frame.
        frame: i32,
        /// Next frame at.
        next_at: f64,
        /// Release queued.
        release_queued: bool,
    },
    /// Cooking.
    Cooking {
        /// Expires at.
        expires_at: f64,
    },
    /// Releasing.
    Releasing {
        /// Expires at.
        expires_at: f64,
        /// Throw at.
        throw_at: f64,
    },
    /// Recovering.
    Recovering {
        /// Ready at.
        ready_at: f64,
        /// Require release.
        require_release: bool,
    },
}

/// Actor lifecycle for hand actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandLifecycle {
    /// Alive.
    Alive,
    /// Dead.
    Dead,
    /// Removing.
    Removing,
    /// Removed.
    Removed,
}

/// Hand action input (`HandActionInput`).
pub struct HandActionInput<'a> {
    /// Tempo.
    pub tempo: HandGrenadeTempo,
    /// Now.
    pub now: f64,
    /// Pressed.
    pub pressed: bool,
    /// Held.
    pub held: bool,
    /// Released.
    pub released: bool,
    /// Lifecycle.
    pub lifecycle: HandLifecycle,
    /// Whether enabled.
    pub enabled: bool,
    /// Throw angles.
    pub angles: Vec3,
    /// Damage multiplier.
    pub damage_multiplier: f64,
    /// Gravity.
    pub gravity: f64,
    /// Muzzle projection.
    pub project: &'a mut dyn FnMut(Vec3, Vec3) -> (Vec3, Vec3),
}

/// Hand action sound event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandSound {
    /// Cock.
    Cock,
    /// Cook start.
    CookStart,
    /// Cook stop.
    CookStop,
}

/// Hand action host (`HandActionHost`).
///
/// The host owns the actor and reservation.
pub trait HandActionHost {
    /// Firing interval adjustment.
    fn firing_interval(&mut self, seconds: f64) -> f64 {
        seconds
    }
    /// Atomically debit one shared grenade, or reserve an explicitly
    /// infinite allowance. The reservation belongs to this action until
    /// consume or refund.
    fn reserve(&mut self) -> bool;
    /// Commit or discard the already debited grenade. Must work after
    /// actor removal without accessing inventory. Never debit inventory
    /// a second time.
    fn consume(&mut self);
    /// Refund the reservation.
    fn refund(&mut self);
    /// Emit a projectile.
    fn emit(&mut self, spec: &HandProjectileSpec);
    /// Play a sound event.
    fn sound(&mut self, event: HandSound);
}

fn firing_interval(host: &mut dyn HandActionHost, seconds: f64) -> f64 {
    let result = host.firing_interval(seconds);
    if !result.is_finite() || result < 0.0 {
        panic!("Original hand grenade cadence must produce a finite nonnegative interval");
    }
    result
}

fn emit_action(expires_at: f64, input: &mut HandActionInput, host: &mut dyn HandActionHost, held: bool) -> HandAction {
    host.sound(HandSound::CookStop);
    host.consume();
    let edition = input.tempo.edition;
    let fuse_deadline = if input.lifecycle == HandLifecycle::Dead && edition == Q2Edition::Classic {
        input.now
    } else {
        expires_at
    };
    let spec = calculate_hand_throw(HandThrowInput {
        edition,
        angles: input.angles,
        alive: input.lifecycle == HandLifecycle::Alive || input.lifecycle == HandLifecycle::Removing,
        now: input.now,
        fuse_deadline,
        damage_multiplier: input.damage_multiplier,
        gravity: input.gravity,
        held,
        project: input.project,
    });
    host.emit(&spec);
    HandAction::Recovering {
        ready_at: hand_deadline(
            input.now,
            firing_interval(host, hand_recovery_seconds(&input.tempo)),
            edition,
        ),
        require_release: held && input.held,
    }
}

fn hand_frame_deadline(input: &mut HandActionInput, host: &mut dyn HandActionHost, from: f64) -> f64 {
    // Source frames land on the simulation clock. Round frame deadlines
    // to milliseconds so binary64 0.2 + 0.1 does not skip the classic
    // turn at 0.3.
    let tempo = input.tempo;
    ((from * 1000.0 + 0.5).floor() + (firing_interval(host, hand_frame_seconds(&tempo)) * 1000.0 + 0.5).floor())
        / 1000.0
}

fn release_action(
    expires_at: f64,
    input: &mut HandActionInput,
    host: &mut dyn HandActionHost,
    released_at: f64,
) -> HandAction {
    if input.tempo.edition == Q2Edition::Rerelease {
        return emit_action(expires_at, input, host, false);
    }
    let throw_at = hand_frame_deadline(input, host, released_at);
    if input.now < throw_at {
        HandAction::Releasing { expires_at, throw_at }
    } else {
        emit_action(expires_at, input, host, false)
    }
}

fn cook_action(expires_at: f64, input: &mut HandActionInput, host: &mut dyn HandActionHost) -> HandAction {
    if input.now >= expires_at {
        return emit_action(expires_at, input, host, true);
    }
    if !input.released && input.held {
        HandAction::Cooking { expires_at }
    } else {
        release_action(expires_at, input, host, input.now)
    }
}

/// Advance once per source simulation turn (`stepHandAction`).
///
/// Send removing before releasing the actor so a primed grenade can use
/// its last pose. Removed only discards a stale reservation; it cannot
/// refund or spawn without an actor.
pub fn step_hand_action(state: &HandAction, input: &mut HandActionInput, host: &mut dyn HandActionHost) -> HandAction {
    if input.lifecycle == HandLifecycle::Removed {
        if matches!(
            state,
            HandAction::Preparing { .. } | HandAction::Cooking { .. } | HandAction::Releasing { .. }
        ) {
            host.consume();
        }
        return HandAction::Disarmed;
    }
    if input.lifecycle == HandLifecycle::Dead || input.lifecycle == HandLifecycle::Removing || !input.enabled {
        if matches!(state, HandAction::Preparing { .. }) {
            host.refund();
        }
        if let HandAction::Cooking { expires_at } | HandAction::Releasing { expires_at, .. } = state {
            emit_action(
                *expires_at,
                input,
                host,
                input.lifecycle == HandLifecycle::Dead && input.tempo.edition == Q2Edition::Rerelease,
            );
        }
        return HandAction::Disarmed;
    }
    match *state {
        HandAction::Disarmed => {
            if input.held {
                *state
            } else {
                HandAction::Idle
            }
        }
        HandAction::Idle => {
            if !input.pressed || !host.reserve() {
                return *state;
            }
            let edition = input.tempo.edition;
            let next_at = hand_frame_deadline(input, host, input.now);
            HandAction::Preparing {
                frame: if edition == Q2Edition::Classic { 1 } else { 2 },
                next_at,
                release_queued: input.released || !input.held,
            }
        }
        HandAction::Preparing {
            frame,
            next_at,
            release_queued,
        } => {
            let mut frame = frame;
            let mut next_at = next_at;
            while input.now >= next_at && frame < 11 {
                if frame == 5 {
                    host.sound(HandSound::Cock);
                }
                frame += 1;
                next_at = hand_frame_deadline(input, host, next_at);
            }
            if input.now < next_at {
                return HandAction::Preparing {
                    frame,
                    next_at,
                    release_queued: release_queued || input.released || !input.held,
                };
            }
            host.sound(HandSound::CookStart);
            let expires_at = hand_fuse_deadline(next_at, input.tempo.edition);
            // A previously observed tap can complete its throw animation
            // during catch-up. A release first observed on this turn
            // belongs to now, not the earlier hold frame.
            if release_queued {
                release_action(expires_at, input, host, next_at)
            } else {
                cook_action(expires_at, input, host)
            }
        }
        HandAction::Cooking { expires_at } => cook_action(expires_at, input, host),
        HandAction::Releasing { expires_at, throw_at } => {
            if input.now < throw_at {
                *state
            } else {
                emit_action(expires_at, input, host, false)
            }
        }
        HandAction::Recovering {
            ready_at,
            require_release,
        } => {
            let require_release = require_release && input.held && !input.released;
            if input.now < ready_at {
                return HandAction::Recovering {
                    ready_at,
                    require_release,
                };
            }
            if require_release {
                HandAction::Disarmed
            } else {
                HandAction::Idle
            }
        }
    }
}
