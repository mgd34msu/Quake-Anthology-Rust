//! Movement-role cvars bound once against the shared registry.
use crate::cvars::Cvars;
use qa_core::primitives::{CvarHandle, MovementTuning, RuleSetId};

#[derive(Clone, Copy, Debug)]
pub struct MovementHandles {
    speed: CvarHandle,
    gravity: CvarHandle,
    friction: CvarHandle,
    stop_speed: CvarHandle,
    accelerate: CvarHandle,
    air_accelerate: CvarHandle,
    water_accelerate: CvarHandle,
    water_friction: CvarHandle,
    spectator_speed: CvarHandle,
    fixed: CvarHandle,
    fixed_msec: CvarHandle,
}

impl MovementHandles {
    pub fn bind(vars: &Cvars) -> Result<Self, &'static str> {
        let bind = |name| vars.find(name).ok_or(name);
        Ok(Self {
            speed: bind("g_speed")?,
            gravity: bind("g_gravity")?,
            friction: bind("sv_friction")?,
            stop_speed: bind("sv_stopspeed")?,
            accelerate: bind("sv_accelerate")?,
            air_accelerate: bind("sv_airaccelerate")?,
            water_accelerate: bind("sv_wateraccelerate")?,
            water_friction: bind("sv_waterfriction")?,
            spectator_speed: bind("sv_spectatormaxspeed")?,
            fixed: bind("pmove_fixed")?,
            fixed_msec: bind("pmove_msec")?,
        })
    }

    /// Resolve a temporary movement input from the player's module-owned base.
    /// Store that base separately or restore it after pmove; a resolved console
    /// override must not become the base used after the cvar is reset.
    pub fn resolve(&self, vars: &Cvars, rules: RuleSetId, base: MovementTuning) -> MovementTuning {
        let mut tuning = base;
        tuning.max_speed = number(vars, self.speed, rules, base.max_speed);
        tuning.gravity = number(vars, self.gravity, rules, base.gravity);
        tuning.friction = number(vars, self.friction, rules, base.friction);
        tuning.stop_speed = number(vars, self.stop_speed, rules, base.stop_speed);
        tuning.accelerate = number(vars, self.accelerate, rules, base.accelerate);
        tuning.air_accelerate = number(vars, self.air_accelerate, rules, base.air_accelerate);
        tuning.water_accelerate = number(vars, self.water_accelerate, rules, base.water_accelerate);
        tuning.water_friction = number(vars, self.water_friction, rules, base.water_friction);
        tuning.spectator_max_speed =
            number(vars, self.spectator_speed, rules, base.spectator_max_speed);
        if rules == RuleSetId::Quake {
            tuning.water_accelerate = tuning.water_accelerate.or(tuning.accelerate);
            tuning.water_friction = tuning.water_friction.or(tuning.friction);
        }
        // Q3 clamps the cvar at its game/cgame settings boundary. The movement
        // entry retains its wider bound for directly supplied module tuning.
        let fixed = (base.fixed_step_ms == 0)
            .then(|| vars.native_integer(self.fixed, rules))
            .flatten()
            .map(|value| value != 0);
        if let Some(enabled) = fixed {
            tuning.fixed_step_ms = if enabled {
                let msec = vars
                    .native_integer(self.fixed_msec, rules)
                    .unwrap_or_else(|| i32::from(base.fixed_step_ms.max(8)));
                msec.clamp(8, 33) as u16
            } else {
                0
            };
        }
        tuning
    }
}

fn number(vars: &Cvars, handle: CvarHandle, rules: RuleSetId, base: Option<f32>) -> Option<f32> {
    // Native modules have already applied cvars and per-player modifiers such
    // as haste or trigger gravity before passing their movement input.
    base.or_else(|| vars.native_number(handle, rules))
}
