//! Rerelease senses from `src/bots/behavior/rerelease/senses.ts`.
//!
//! The senses model: each enemy carries an awareness level in
//! `[0, 1]` that fills while seen or heard and drains otherwise, at
//! skill-set rates. A tighter weapon cone gates the trigger; forget
//! timers drop stale contacts.

use qa_core::math::Vec3;

use crate::behavior::rerelease::data::botdata::{BotSensesSettings, BotWeaponSenseSettings};
use crate::behavior::rerelease::math::{angle_between, bvec_distance, bvec_sub};

/// Awareness record for one enemy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotAwarenessT {
    /// Entity id.
    pub id: i32,
    /// Sight awareness `[0, 1]`.
    pub sight: f32,
    /// Weapon-cone tracker `[0, 1]`.
    pub weapon: f32,
    /// Last contact time.
    pub last_contact: f32,
    /// Last seen time.
    pub last_seen: f32,
    /// Last heard time.
    pub last_heard: f32,
    /// Last known origin.
    pub last_known_origin: Vec3,
}

/// New awareness record.
#[must_use]
pub fn new_awareness(id: i32, now: f32, origin: Vec3) -> BotAwarenessT {
    BotAwarenessT {
        id,
        sight: 0.0,
        weapon: 0.0,
        last_contact: now,
        last_seen: -1.0,
        last_heard: -1.0,
        last_known_origin: origin,
    }
}

/// One frame's contact with an enemy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotContactT {
    /// Clear eye trace.
    pub line_of_sight: bool,
    /// Inside the sight cone.
    pub in_sight_fov: bool,
    /// Inside the weapon cone.
    pub in_weapon_fov: bool,
    /// Audible.
    pub audible: bool,
    /// Invisible.
    pub invisible: bool,
    /// Distance.
    pub distance: f32,
    /// Origin.
    pub origin: Vec3,
}

/// Sight geometry evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SightGeometry {
    /// Inside the sight cone.
    pub in_sight_fov: bool,
    /// Inside the weapon cone.
    pub in_weapon_fov: bool,
    /// Distance.
    pub distance: f32,
    /// Within invisible range.
    pub within_invis_range: bool,
}

/// Evaluate sight geometry for a target.
#[must_use]
pub fn evaluate_sight_geometry(
    eye: Vec3,
    pitch: f32,
    yaw: f32,
    target: Vec3,
    invisible: bool,
    senses: &BotSensesSettings,
    weapons: &BotWeaponSenseSettings,
) -> SightGeometry {
    let dir = bvec_sub(target, eye);
    let distance = bvec_distance(target, eye);
    let angle = angle_between(pitch, yaw, dir);
    SightGeometry {
        in_sight_fov: angle <= senses.fov_angle / 2.0,
        in_weapon_fov: angle <= weapons.fov_angle / 2.0,
        distance,
        within_invis_range: !invisible
            || senses.max_invis_enemy_sight_dist <= 0.0
            || distance <= senses.max_invis_enemy_sight_dist,
    }
}

/// Advance one enemy's awareness by a frame.
pub fn sense_step(
    awareness: &mut BotAwarenessT,
    contact: &BotContactT,
    senses: &BotSensesSettings,
    weapons: &BotWeaponSenseSettings,
    dt: f32,
    now: f32,
) {
    let visible = contact.line_of_sight && contact.in_sight_fov;
    let mut fill_time = senses.sight_time;
    if contact.invisible && senses.invis_enemy_sight_scalar > 0.0 {
        fill_time *= senses.invis_enemy_sight_scalar;
    }
    if visible {
        awareness.sight = if fill_time > 0.0 {
            (awareness.sight + dt / fill_time).clamp(0.0, 1.0)
        } else {
            1.0
        };
        awareness.last_seen = now;
        awareness.last_contact = now;
        awareness.last_known_origin = contact.origin;
    } else if contact.audible {
        awareness.sight = if senses.sound_time > 0.0 {
            (awareness.sight + dt / senses.sound_time).clamp(0.0, 1.0)
        } else {
            1.0
        };
        awareness.last_heard = now;
        awareness.last_contact = now;
        awareness.last_known_origin = contact.origin;
    } else {
        let decay = if awareness.last_seen >= awareness.last_heard {
            senses.sight_decay_time
        } else {
            senses.sound_decay_time
        };
        awareness.sight = if decay > 0.0 {
            (awareness.sight - dt / decay).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    if contact.line_of_sight && contact.in_weapon_fov {
        awareness.weapon = if weapons.sight_time > 0.0 {
            (awareness.weapon + dt / weapons.sight_time).clamp(0.0, 1.0)
        } else {
            1.0
        };
    } else {
        awareness.weapon = if weapons.decay_time > 0.0 {
            (awareness.weapon - dt / weapons.decay_time).clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
}

/// Whether the bot is sure enough of an enemy to act.
#[must_use]
pub fn is_aware(awareness: &BotAwarenessT) -> bool {
    awareness.sight >= 1.0
}

/// Whether the weapon cone has settled enough to fire.
#[must_use]
pub fn can_fire(awareness: &BotAwarenessT) -> bool {
    awareness.weapon >= 1.0
}

/// Whether the enemy should drop from memory entirely.
#[must_use]
pub fn should_forget(awareness: &BotAwarenessT, senses: &BotSensesSettings, now: f32) -> bool {
    if awareness.sight > 0.0 {
        return false;
    }
    now - awareness.last_contact >= senses.forget_non_vis_enemy_time
}

/// Whether a sound is still in memory and close enough to hear.
#[must_use]
pub fn sound_audible(
    origin: Vec3,
    time: f32,
    loudness: f32,
    listener: Vec3,
    senses: &BotSensesSettings,
    now: f32,
) -> bool {
    if now - time > senses.sound_persist_time {
        return false;
    }
    let range = senses.sound_range * if loudness > 0.0 { loudness } else { 1.0 };
    bvec_distance(origin, listener) <= range
}
