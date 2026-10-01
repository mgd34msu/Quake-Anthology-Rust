//! Weapon presentation rules (`src/content/q2/foundation/weapons/presentation.ts`).
//!
//! `p_weapon.c`/`p_weapon.cpp` weapon presentation rules (GPL-2.0-or-later).

use qa_core::math::{scale3, Vec3};

use super::generic_frame::millisecond_sum;
use super::types::Q2WeaponState;
use crate::q2::foundation::host::Q2Edition;

/// Set weapon recoil (`setQ2WeaponRecoil`).
pub fn set_q2_weapon_recoil(
    state: &mut Q2WeaponState,
    edition: Q2Edition,
    now: f64,
    origin: Vec3,
    angles: Vec3,
    duration: Option<f64>,
) {
    let duration = duration.unwrap_or(if edition == Q2Edition::Rerelease { 0.2 } else { 0.0 });
    state.kick_origin = origin;
    state.kick_angles = angles;
    state.kick_time = now;
    state.kick_duration = duration;
    state.kick_until = if edition == Q2Edition::Rerelease {
        millisecond_sum(now, duration)
    } else {
        now + duration
    };
}

/// Weapon recoil (`q2WeaponRecoil`).
///
/// Classic kick vectors last one source frame; damage pitch and
/// rerelease kicks decay with source time.
pub fn q2_weapon_recoil(state: &Q2WeaponState, edition: Q2Edition, now: f64) -> (Vec3, Vec3) {
    let impulse = f64::from(i32::from(now == state.kick_time));
    let factor = if state.kick_duration == 0.0 {
        impulse
    } else {
        0.0f64.max(1.0f64.min((state.kick_until - now) / state.kick_duration))
    };
    (
        scale3(
            state.kick_origin,
            if edition == Q2Edition::Classic {
                impulse as f32
            } else {
                factor as f32
            },
        ),
        scale3(state.kick_angles, factor as f32),
    )
}

/// Attack frames (`q2AttackFrames`).
pub fn q2_attack_frames(ducked: bool, offset: i32) -> (i32, i32) {
    (if ducked { 160 } else { 46 } - offset, if ducked { 168 } else { 53 })
}

/// Reverse frames (`q2ReverseFrames`).
pub fn q2_reverse_frames(ducked: bool) -> (i32, i32) {
    (if ducked { 173 } else { 66 }, if ducked { 169 } else { 62 })
}

/// Weapon animation rate input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2AnimationRateInput {
    /// Quick switch.
    pub quick_switch: bool,
    /// Frame seconds.
    pub frame_seconds: f64,
    /// Phase.
    pub phase: super::generic_frame::Q2GenericPhase,
    /// Frame.
    pub frame: i32,
    /// Quad-fire expiry.
    pub quad_fire_until: f64,
    /// Haste.
    pub haste: bool,
    /// Now.
    pub now: f64,
}

/// Weapon animation rate (`q2WeaponAnimationRate`).
pub fn q2_weapon_animation_rate(input: &Q2AnimationRateInput) -> i32 {
    let mut rate = if input.quick_switch
        && input.frame_seconds <= 0.05
        && (input.phase == super::generic_frame::Q2GenericPhase::Activating
            || input.phase == super::generic_frame::Q2GenericPhase::Dropping)
    {
        20
    } else {
        10
    };
    if input.frame != 0 {
        if input.quad_fire_until > input.now {
            rate *= 2;
        }
        if input.haste {
            rate *= 2;
        }
    }
    rate
}

/// Powerup sound input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2PowerupSoundInput {
    /// Quad expiry.
    pub quad_until: f64,
    /// Double expiry.
    pub double_until: f64,
    /// Whether rerelease.
    pub rerelease: bool,
    /// Now.
    pub now: f64,
}

/// Powerup sound (`q2PowerupSound`).
pub fn q2_powerup_sound(input: &Q2PowerupSoundInput) -> Option<&'static str> {
    if input.quad_until > input.now && input.double_until > input.now && input.rerelease {
        return Some("ctf/tech2x.wav");
    }
    if input.quad_until > input.now {
        return Some("items/damage3.wav");
    }
    if input.double_until > input.now {
        return Some("misc/ddamage3.wav");
    }
    None
}
