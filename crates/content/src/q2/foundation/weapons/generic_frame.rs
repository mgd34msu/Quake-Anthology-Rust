//! Generic weapon frames (`src/content/q2/foundation/weapons/generic-frame.ts`).

use super::types::{Q2WeaponDefinition, Q2WeaponPhase};

/// Generic frame phase.
pub type Q2GenericPhase = Q2WeaponPhase;

/// Generic frame state (`Q2GenericFrameState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2GenericFrameState {
    /// Phase.
    pub phase: Q2WeaponPhase,
    /// Frame.
    pub frame: i32,
    /// Latched attack.
    pub latched_attack: bool,
    /// Source firing.
    pub source_firing: bool,
    /// Think time.
    pub think_time: f64,
    /// Fire finished.
    pub fire_finished: f64,
    /// Fire buffered.
    pub fire_buffered: bool,
    /// Last firing time.
    pub last_firing_time: f64,
}

/// Generic frame definition (`Q2GenericDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2GenericDefinition {
    /// Weapon name.
    pub name: String,
    /// Activate last frame.
    pub activate_last: i32,
    /// Fire last frame.
    pub fire_last: i32,
    /// Idle last frame.
    pub idle_last: i32,
    /// Deactivate last frame.
    pub deactivate_last: i32,
    /// Pause frames.
    pub pauses: Vec<i32>,
    /// Fire frames.
    pub fires: Vec<i32>,
    /// Ammo quantity per shot.
    pub quantity: i32,
    /// Whether repeating.
    pub repeating: bool,
}

impl From<&Q2WeaponDefinition> for Q2GenericDefinition {
    fn from(definition: &Q2WeaponDefinition) -> Self {
        Self {
            name: definition.name.clone(),
            activate_last: definition.activate_last,
            fire_last: definition.fire_last,
            idle_last: definition.idle_last,
            deactivate_last: definition.deactivate_last,
            pauses: definition.pauses.clone(),
            fires: definition.fires.clone(),
            quantity: definition.quantity,
            repeating: definition.repeating,
        }
    }
}

/// Classic frame input (`Q2ClassicFrameInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ClassicFrameInput {
    /// Attack held.
    pub attack: bool,
    /// Change requested.
    pub change_requested: bool,
}

/// Rerelease frame input (`Q2RereleaseFrameInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseFrameInput {
    /// Attack held.
    pub attack: bool,
    /// Change requested.
    pub change_requested: bool,
    /// Now.
    pub now: f64,
    /// Frame seconds.
    pub frame_seconds: f64,
    /// Instant switch.
    pub instant_switch: bool,
    /// Holster.
    pub holster: bool,
    /// Weapon thunk.
    pub weapon_thunk: bool,
}

/// Classic frame hooks (`Q2ClassicFrameHooks`).
pub trait ClassicFrameHooks {
    /// Draw a unit random number.
    fn random(&mut self) -> f64;
    /// Read ammo.
    fn ammo(&mut self) -> f64;
    /// Report no ammo.
    fn no_ammo(&mut self);
    /// Fire.
    fn fire(&mut self, buffered: bool);
    /// Change weapon.
    fn change_weapon(&mut self);
    /// Play reverse animation.
    fn reverse_animation(&mut self);
    /// Play attack animation.
    fn attack_animation(&mut self);
    /// Play powerup sound.
    fn powerup_sound(&mut self);
}

/// Rerelease frame hooks (`Q2RereleaseFrameHooks`).
pub trait RereleaseFrameHooks: ClassicFrameHooks {
    /// Animation frame time.
    fn animation_time(&mut self) -> f64;
    /// Prepare a drop.
    fn prepare_drop(&mut self);
}

/// Millisecond clock sum (`millisecondSum`).
pub fn millisecond_sum(time: f64, seconds: f64) -> f64 {
    ((time * 1000.0 + 0.5).floor() + (seconds * 1000.0 + 0.5).floor()) / 1000.0
}

/// Step a classic weapon frame (`stepQ2ClassicFrame`).
pub fn step_q2_classic_frame(
    state: &mut Q2GenericFrameState,
    definition: &Q2GenericDefinition,
    input: &Q2ClassicFrameInput,
    hooks: &mut dyn ClassicFrameHooks,
) {
    let idle_first = definition.fire_last + 1;
    if state.phase == Q2WeaponPhase::Dropping {
        if state.frame == definition.deactivate_last {
            hooks.change_weapon();
            return;
        }
        if definition.deactivate_last - state.frame == 4 {
            hooks.reverse_animation();
        }
        state.frame += 1;
        return;
    }
    if state.phase == Q2WeaponPhase::Activating {
        if state.frame == definition.activate_last {
            state.phase = Q2WeaponPhase::Ready;
            state.frame = idle_first;
        } else {
            state.frame += 1;
        }
        return;
    }
    if input.change_requested && state.phase != Q2WeaponPhase::Firing {
        state.phase = Q2WeaponPhase::Dropping;
        state.frame = definition.idle_last + 1;
        if definition.deactivate_last - state.frame < 4 {
            hooks.reverse_animation();
        }
        return;
    }
    if state.phase == Q2WeaponPhase::Ready {
        if input.attack || state.latched_attack {
            state.latched_attack = false;
            if hooks.ammo() < f64::from(definition.quantity) {
                hooks.no_ammo();
                return;
            }
            state.frame = definition.activate_last + 1;
            state.phase = Q2WeaponPhase::Firing;
            hooks.attack_animation();
        } else {
            if state.frame == definition.idle_last {
                state.frame = idle_first;
                return;
            }
            if definition.pauses.contains(&state.frame) && (hooks.random() * 16.0).floor() as i32 != 0 {
                return;
            }
            state.frame += 1;
            return;
        }
    }
    if state.phase == Q2WeaponPhase::Firing {
        if definition.fires.contains(&state.frame) {
            state.source_firing = true;
            hooks.powerup_sound();
            hooks.fire(false);
        } else {
            state.frame += 1;
        }
        if state.frame == idle_first + 1 {
            state.phase = Q2WeaponPhase::Ready;
        }
    }
}

/// Step a rerelease weapon frame (`stepQ2RereleaseFrame`).
pub fn step_q2_rerelease_frame(
    state: &mut Q2GenericFrameState,
    definition: &Q2GenericDefinition,
    input: &Q2RereleaseFrameInput,
    hooks: &mut dyn RereleaseFrameHooks,
) {
    let now = input.now;
    let idle_first = definition.fire_last + 1;
    let idle_last = if definition.name == "bfg" { 54 } else { definition.idle_last };
    if state.phase == Q2WeaponPhase::Dropping {
        if state.think_time <= now {
            if state.frame == definition.deactivate_last {
                hooks.change_weapon();
                return;
            }
            if definition.deactivate_last - state.frame == 4 {
                hooks.reverse_animation();
            }
            state.frame += 1;
            state.think_time = millisecond_sum(now, hooks.animation_time());
        }
        return;
    }
    if state.phase == Q2WeaponPhase::Activating {
        if state.think_time <= now || input.instant_switch {
            state.think_time = millisecond_sum(now, hooks.animation_time());
            if state.frame == definition.activate_last || input.instant_switch {
                state.phase = Q2WeaponPhase::Ready;
                state.frame = idle_first;
                state.fire_buffered = false;
                state.fire_finished = if input.instant_switch { 0.0 } else { millisecond_sum(now, hooks.animation_time()) };
            } else {
                state.frame += 1;
            }
            return;
        }
    }
    if (input.change_requested || !input.instant_switch && input.holster) && state.phase != Q2WeaponPhase::Firing {
        if input.instant_switch || state.think_time <= now {
            hooks.prepare_drop();
            state.phase = Q2WeaponPhase::Dropping;
            if input.instant_switch {
                hooks.change_weapon();
                return;
            }
            state.frame = idle_last + 1;
            if definition.deactivate_last - state.frame < 4 {
                hooks.reverse_animation();
            }
            state.think_time = millisecond_sum(now, hooks.animation_time());
        }
        return;
    }
    if state.phase == Q2WeaponPhase::Ready {
        if (state.fire_buffered || state.latched_attack || input.attack) && state.fire_finished <= now {
            state.latched_attack = false;
            state.think_time = now;
            if hooks.ammo() < f64::from(definition.quantity) {
                hooks.no_ammo();
                return;
            }
            state.phase = Q2WeaponPhase::Firing;
            state.last_firing_time = millisecond_sum(now, 2.5);
            if !definition.repeating {
                state.frame = definition.activate_last + 1;
                state.fire_buffered = false;
                state.think_time = millisecond_sum(
                    state.think_time,
                    (if input.weapon_thunk { input.frame_seconds } else { 0.0 }) + hooks.animation_time(),
                );
                state.fire_finished = millisecond_sum(now, hooks.animation_time());
                if definition.fires.contains(&state.frame) {
                    hooks.powerup_sound();
                    hooks.fire(false);
                }
                hooks.attack_animation();
                return;
            }
        } else if state.think_time <= now {
            state.think_time = millisecond_sum(now, hooks.animation_time());
            if state.frame == idle_last {
                state.frame = idle_first;
                return;
            }
            if !definition.pauses.contains(&state.frame) || (hooks.random() * 16.0).floor() as i32 == 0 {
                state.frame += 1;
            }
            return;
        }
    }
    if state.phase == Q2WeaponPhase::Firing && state.think_time <= now {
        state.last_firing_time = millisecond_sum(now, 2.5);
        if !definition.repeating {
            state.frame += 1;
        }
        state.fire_finished = millisecond_sum(now, hooks.animation_time());
        let buffered = state.fire_buffered;
        state.fire_buffered = false;
        if definition.repeating {
            hooks.fire(buffered);
        } else if definition.fires.contains(&state.frame) && !(definition.name == "shotgun" && state.frame == 9) {
            hooks.powerup_sound();
            hooks.fire(buffered);
        }
        if state.frame == idle_first {
            state.phase = Q2WeaponPhase::Ready;
            state.fire_buffered = false;
        }
        state.think_time = millisecond_sum(
            now,
            hooks.animation_time() + (if definition.repeating && input.weapon_thunk { input.frame_seconds } else { 0.0 }),
        );
    }
}
