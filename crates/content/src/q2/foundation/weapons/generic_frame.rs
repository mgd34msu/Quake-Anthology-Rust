//! Generic weapon frames (`src/content/q2/foundation/weapons/generic-frame.ts`).

use super::types::{Q2WeaponDefinition, Q2WeaponPhase, Q2WeaponState};

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

/// Live weapon frame state (`Q2GenericFrameState` projection).
///
/// The steppers run against live `Q2WeaponState` through this trait so
/// hook callbacks observe and mutate the same state the donor steps.
pub trait GenericFrameState {
    /// Phase.
    fn phase(&self) -> Q2WeaponPhase;
    /// Set the phase.
    fn set_phase(&mut self, phase: Q2WeaponPhase);
    /// Frame.
    fn frame(&self) -> i32;
    /// Set the frame.
    fn set_frame(&mut self, frame: i32);
    /// Latched attack.
    fn latched_attack(&self) -> bool;
    /// Set latched attack.
    fn set_latched_attack(&mut self, value: bool);
    /// Source firing.
    fn source_firing(&self) -> bool;
    /// Set source firing.
    fn set_source_firing(&mut self, value: bool);
    /// Think time.
    fn think_time(&self) -> f64;
    /// Set think time.
    fn set_think_time(&mut self, value: f64);
    /// Fire finished.
    fn fire_finished(&self) -> f64;
    /// Set fire finished.
    fn set_fire_finished(&mut self, value: f64);
    /// Fire buffered.
    fn fire_buffered(&self) -> bool;
    /// Set fire buffered.
    fn set_fire_buffered(&mut self, value: bool);
    /// Last firing time.
    fn last_firing_time(&self) -> f64;
    /// Set last firing time.
    fn set_last_firing_time(&mut self, value: f64);
}

macro_rules! generic_frame_state {
    ($type:ty) => {
        impl GenericFrameState for $type {
            fn phase(&self) -> Q2WeaponPhase { self.phase }
            fn set_phase(&mut self, phase: Q2WeaponPhase) { self.phase = phase; }
            fn frame(&self) -> i32 { self.frame }
            fn set_frame(&mut self, frame: i32) { self.frame = frame; }
            fn latched_attack(&self) -> bool { self.latched_attack }
            fn set_latched_attack(&mut self, value: bool) { self.latched_attack = value; }
            fn source_firing(&self) -> bool { self.source_firing }
            fn set_source_firing(&mut self, value: bool) { self.source_firing = value; }
            fn think_time(&self) -> f64 { self.think_time }
            fn set_think_time(&mut self, value: f64) { self.think_time = value; }
            fn fire_finished(&self) -> f64 { self.fire_finished }
            fn set_fire_finished(&mut self, value: f64) { self.fire_finished = value; }
            fn fire_buffered(&self) -> bool { self.fire_buffered }
            fn set_fire_buffered(&mut self, value: bool) { self.fire_buffered = value; }
            fn last_firing_time(&self) -> f64 { self.last_firing_time }
            fn set_last_firing_time(&mut self, value: f64) { self.last_firing_time = value; }
        }
    };
}

generic_frame_state!(Q2GenericFrameState);
generic_frame_state!(super::types::Q2WeaponState);

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
    /// Live frame hooks.frame_state().
    fn frame_state(&mut self) -> &mut dyn GenericFrameState;
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
    definition: &Q2GenericDefinition,
    input: &Q2ClassicFrameInput,
    hooks: &mut dyn ClassicFrameHooks,
) {
    let idle_first = definition.fire_last + 1;
    if hooks.frame_state().phase() == Q2WeaponPhase::Dropping {
        if hooks.frame_state().frame() == definition.deactivate_last {
            hooks.change_weapon();
            return;
        }
        if definition.deactivate_last - hooks.frame_state().frame() == 4 {
            hooks.reverse_animation();
        }
        let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
        return;
    }
    if hooks.frame_state().phase() == Q2WeaponPhase::Activating {
        if hooks.frame_state().frame() == definition.activate_last {
            hooks.frame_state().set_phase(Q2WeaponPhase::Ready);
            hooks.frame_state().set_frame(idle_first);
        } else {
            let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
        }
        return;
    }
    if input.change_requested && hooks.frame_state().phase() != Q2WeaponPhase::Firing {
        hooks.frame_state().set_phase(Q2WeaponPhase::Dropping);
        hooks.frame_state().set_frame(definition.idle_last + 1);
        if definition.deactivate_last - hooks.frame_state().frame() < 4 {
            hooks.reverse_animation();
        }
        return;
    }
    if hooks.frame_state().phase() == Q2WeaponPhase::Ready {
        if input.attack || hooks.frame_state().latched_attack() {
            hooks.frame_state().set_latched_attack(false);
            if hooks.ammo() < f64::from(definition.quantity) {
                hooks.no_ammo();
                return;
            }
            hooks.frame_state().set_frame(definition.activate_last + 1);
            hooks.frame_state().set_phase(Q2WeaponPhase::Firing);
            hooks.attack_animation();
        } else {
            if hooks.frame_state().frame() == definition.idle_last {
                hooks.frame_state().set_frame(idle_first);
                return;
            }
            if definition.pauses.contains(&hooks.frame_state().frame()) && (hooks.random() * 16.0).floor() as i32 != 0 {
                return;
            }
            let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
            return;
        }
    }
    if hooks.frame_state().phase() == Q2WeaponPhase::Firing {
        if definition.fires.contains(&hooks.frame_state().frame()) {
            hooks.frame_state().set_source_firing(true);
            hooks.powerup_sound();
            hooks.fire(false);
        } else {
            let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
        }
        if hooks.frame_state().frame() == idle_first + 1 {
            hooks.frame_state().set_phase(Q2WeaponPhase::Ready);
        }
    }
}

/// Step a rerelease weapon frame (`stepQ2RereleaseFrame`).
pub fn step_q2_rerelease_frame(
    definition: &Q2GenericDefinition,
    input: &Q2RereleaseFrameInput,
    hooks: &mut dyn RereleaseFrameHooks,
) {
    let now = input.now;
    let idle_first = definition.fire_last + 1;
    let idle_last = if definition.name == "bfg" { 54 } else { definition.idle_last };
    if hooks.frame_state().phase() == Q2WeaponPhase::Dropping {
        if hooks.frame_state().think_time() <= now {
            if hooks.frame_state().frame() == definition.deactivate_last {
                hooks.change_weapon();
                return;
            }
            if definition.deactivate_last - hooks.frame_state().frame() == 4 {
                hooks.reverse_animation();
            }
            let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
            let think = millisecond_sum(now, hooks.animation_time());
            hooks.frame_state().set_think_time(think);
        }
        return;
    }
    if hooks.frame_state().phase() == Q2WeaponPhase::Activating {
        if hooks.frame_state().think_time() <= now || input.instant_switch {
            let think = millisecond_sum(now, hooks.animation_time());
            hooks.frame_state().set_think_time(think);
            if hooks.frame_state().frame() == definition.activate_last || input.instant_switch {
                hooks.frame_state().set_phase(Q2WeaponPhase::Ready);
                hooks.frame_state().set_frame(idle_first);
                hooks.frame_state().set_fire_buffered(false);
                let finished = if input.instant_switch { 0.0 } else { millisecond_sum(now, hooks.animation_time()) };
                hooks.frame_state().set_fire_finished(finished);
            } else {
                let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
            }
            return;
        }
    }
    if (input.change_requested || !input.instant_switch && input.holster) && hooks.frame_state().phase() != Q2WeaponPhase::Firing {
        if input.instant_switch || hooks.frame_state().think_time() <= now {
            hooks.prepare_drop();
            hooks.frame_state().set_phase(Q2WeaponPhase::Dropping);
            if input.instant_switch {
                hooks.change_weapon();
                return;
            }
            hooks.frame_state().set_frame(idle_last + 1);
            if definition.deactivate_last - hooks.frame_state().frame() < 4 {
                hooks.reverse_animation();
            }
            let think = millisecond_sum(now, hooks.animation_time());
            hooks.frame_state().set_think_time(think);
        }
        return;
    }
    if hooks.frame_state().phase() == Q2WeaponPhase::Ready {
        if (hooks.frame_state().fire_buffered() || hooks.frame_state().latched_attack() || input.attack) && hooks.frame_state().fire_finished() <= now {
            hooks.frame_state().set_latched_attack(false);
            hooks.frame_state().set_think_time(now);
            if hooks.ammo() < f64::from(definition.quantity) {
                hooks.no_ammo();
                return;
            }
            hooks.frame_state().set_phase(Q2WeaponPhase::Firing);
            hooks.frame_state().set_last_firing_time(millisecond_sum(now, 2.5));
            if !definition.repeating {
                hooks.frame_state().set_frame(definition.activate_last + 1);
                hooks.frame_state().set_fire_buffered(false);
                let think_time = hooks.frame_state().think_time();
                let think = millisecond_sum(
                    think_time,
                    (if input.weapon_thunk { input.frame_seconds } else { 0.0 })
                        + hooks.animation_time(),
                );
                hooks.frame_state().set_think_time(think);
                let finished = millisecond_sum(now, hooks.animation_time());
                hooks.frame_state().set_fire_finished(finished);
                if definition.fires.contains(&hooks.frame_state().frame()) {
                    hooks.powerup_sound();
                    hooks.fire(false);
                }
                hooks.attack_animation();
                return;
            }
        } else if hooks.frame_state().think_time() <= now {
            let think = millisecond_sum(now, hooks.animation_time());
            hooks.frame_state().set_think_time(think);
            if hooks.frame_state().frame() == idle_last {
                hooks.frame_state().set_frame(idle_first);
                return;
            }
            if !definition.pauses.contains(&hooks.frame_state().frame()) || (hooks.random() * 16.0).floor() as i32 == 0 {
                let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
            }
            return;
        }
    }
    if hooks.frame_state().phase() == Q2WeaponPhase::Firing && hooks.frame_state().think_time() <= now {
        hooks.frame_state().set_last_firing_time(millisecond_sum(now, 2.5));
        if !definition.repeating {
            let frame = hooks.frame_state().frame();
        hooks.frame_state().set_frame(frame + 1);
        }
        let finished = millisecond_sum(now, hooks.animation_time());
        hooks.frame_state().set_fire_finished(finished);
        let buffered = hooks.frame_state().fire_buffered();
        hooks.frame_state().set_fire_buffered(false);
        if definition.repeating {
            hooks.fire(buffered);
        } else if definition.fires.contains(&hooks.frame_state().frame()) && !(definition.name == "shotgun" && hooks.frame_state().frame() == 9) {
            hooks.powerup_sound();
            hooks.fire(buffered);
        }
        if hooks.frame_state().frame() == idle_first {
            hooks.frame_state().set_phase(Q2WeaponPhase::Ready);
            hooks.frame_state().set_fire_buffered(false);
        }
        let think = millisecond_sum(
            now,
            hooks.animation_time()
                + (if definition.repeating && input.weapon_thunk { input.frame_seconds } else { 0.0 }),
        );
        hooks.frame_state().set_think_time(think);
    }
}

/// Project weapon state onto a generic frame.
///
/// Match-mode grapple weapons step the shared frame functions with the
/// weapon phase and frame; the remaining generic fields start unset.
pub fn project_weapon_animation(state: &Q2WeaponState) -> Q2GenericFrameState {
    Q2GenericFrameState {
        phase: state.phase,
        frame: state.frame,
        latched_attack: false,
        source_firing: false,
        think_time: 0.0,
        fire_finished: 0.0,
        fire_buffered: false,
        last_firing_time: 0.0,
    }
}
