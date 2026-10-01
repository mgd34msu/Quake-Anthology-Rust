//! Q2 player environment (`src/content/q2/base/player/environment.ts`).

use crate::q2::foundation::entity_services::js_round;
use crate::q2::foundation::host::Q2Mode;

use super::types::Q2CharacterContext;

/// Apply environment damage (`q2EnvironmentDamage`).
pub fn q2_environment_damage(
    context: &mut impl Q2CharacterContext,
    amount: f64,
    means: i32,
    flags: i32,
) {
    context.environment_damage(amount, means, flags);
}

/// Run world effects (`q2WorldEffects`).
///
/// p_view.c P_WorldEffects runs once on the selected Q2 source frame.
pub fn q2_world_effects(context: &mut impl Q2CharacterContext) {
    let now = context.now();
    let snapshot = context.state_snapshot();
    if snapshot.noclip || snapshot.spectator {
        context.with_state(|state| {
            state.air_finished = now + 12.0;
        });
        return;
    }
    let movement = context.movement();
    let level = movement.water_level;
    let old = snapshot.old_water_level;
    let powers = context.powerups();
    let breather = powers.breather_until > now;
    let suit = powers.enviro_until > now;
    context.with_state(|state| {
        state.old_water_level = level;
    });
    if old == 0 && level != 0 {
        let origin = context.body().origin;
        context.noise(origin);
        context.sound(
            if movement.water_type & 8 != 0 {
                "player/lava_in.wav"
            } else {
                "player/watr_in.wav"
            },
            4,
            1.0,
            1.0,
        );
        context.with_entity(|entity| {
            entity.flags |= 8;
        });
    }
    if old != 0 && level == 0 {
        let origin = context.body().origin;
        context.noise(origin);
        context.sound("player/watr_out.wav", 4, 1.0, 1.0);
        context.with_entity(|entity| {
            entity.flags &= !8;
        });
    }
    if old != 3 && level == 3 {
        context.sound("player/watr_un.wav", 4, 1.0, 1.0);
    }
    if old == 3 && level != 3 {
        let air_finished = context.state_snapshot().air_finished;
        if air_finished < now {
            context.sound("player/gasp1.wav", 2, 1.0, 1.0);
            let origin = context.body().origin;
            context.noise(origin);
        } else if air_finished < now + 11.0 {
            context.sound("player/gasp2.wav", 2, 1.0, 1.0);
        }
    }
    if level == 3 {
        if breather || suit {
            context.with_state(|state| {
                state.air_finished = now + 10.0;
            });
            let remaining = js_round((powers.breather_until - now) * 10.0) as i64;
            if remaining % 25 == 0 {
                let breather_sound = context.state_snapshot().breather_sound;
                context.sound(
                    if breather_sound == 0 {
                        "player/u_breath1.wav"
                    } else {
                        "player/u_breath2.wav"
                    },
                    0,
                    1.0,
                    1.0,
                );
                context.with_state(|state| {
                    state.breather_sound ^= 1;
                });
                let origin = context.body().origin;
                context.noise(origin);
            }
        }
        let health = context.combat().map_or(0.0, |combat| combat.health);
        let snapshot = context.state_snapshot();
        if snapshot.air_finished < now && snapshot.next_drown_time < now && health > 0.0 {
            let drown_damage = (snapshot.drown_damage + 2.0).min(15.0);
            let path = if health <= drown_damage {
                "player/drown1.wav"
            } else if context.random() < 0.5 {
                "*gurp2.wav"
            } else {
                "*gurp1.wav"
            };
            context.sound(path, 2, 1.0, 1.0);
            context.with_state(|state| {
                state.next_drown_time = now + 1.0;
                state.drown_damage = drown_damage;
                state.pain_debounce = now;
            });
            q2_environment_damage(context, drown_damage, 17, 2);
        }
    } else {
        context.with_state(|state| {
            state.air_finished = now + 12.0;
            state.drown_damage = 2.0;
        });
    }
    if level != 0 && movement.water_type & (8 | 16) != 0 {
        if movement.water_type & 8 != 0 {
            let health = context.combat().map_or(0.0, |combat| combat.health);
            let pain_debounce = context.state_snapshot().pain_debounce;
            if health > 0.0
                && pain_debounce <= now
                && powers.invulnerability_until < now
            {
                let path = if context.random() < 0.5 {
                    "player/burn2.wav"
                } else {
                    "player/burn1.wav"
                };
                context.sound(path, 2, 1.0, 1.0);
                context.with_state(|state| {
                    state.pain_debounce = now + 1.0;
                });
            }
            q2_environment_damage(
                context,
                (if suit { 1.0 } else { 3.0 }) * f64::from(level),
                19,
                0,
            );
        }
        if movement.water_type & 16 != 0 && !suit {
            q2_environment_damage(context, f64::from(level), 18, 0);
        }
    }
}

/// Run falling damage (`q2FallingDamage`).
pub fn q2_falling_damage(context: &mut impl Q2CharacterContext) {
    let movement = context.movement();
    let snapshot = context.state_snapshot();
    if !movement.animate_q2 || snapshot.noclip || snapshot.spectator {
        return;
    }
    let velocity = context.body().velocity;
    let mut delta: f64;
    if snapshot.old_velocity.z < 0.0
        && velocity.z > snapshot.old_velocity.z
        && !movement.grounded
    {
        delta = f64::from(snapshot.old_velocity.z);
    } else {
        if !movement.grounded {
            return;
        }
        delta = f64::from(velocity.z - snapshot.old_velocity.z);
    }
    delta = delta * delta * 0.0001;
    if movement.water_level == 3 {
        return;
    }
    if movement.water_level == 2 {
        delta *= 0.25;
    }
    if movement.water_level == 1 {
        delta *= 0.5;
    }
    if delta < 1.0 {
        return;
    }
    if snapshot.landmark_free_fall {
        delta = delta.min(30.0);
        let now = context.now();
        context.with_state(|state| {
            state.landmark_free_fall = false;
            state.landmark_noise_time = now + 0.1;
        });
    }
    if delta < 15.0 {
        context.with_state(|state| {
            state.event = "q2:footstep".to_string();
        });
        return;
    }
    let now = context.now();
    context.with_state(|state| {
        state.fall_value = (delta * 0.5).min(40.0);
        state.fall_time = now + 0.3;
    });
    if delta > 30.0 {
        let health = context.combat().map_or(0.0, |combat| combat.health);
        if health > 0.0 {
            context.with_state(|state| {
                state.event = if delta >= 55.0 {
                    "q2:fall-far".to_string()
                } else {
                    "q2:fall".to_string()
                };
            });
        }
        let now = context.now();
        context.with_state(|state| {
            state.pain_debounce = now;
        });
        if context.mode() != Q2Mode::Deathmatch || context.deathmatch_flags() & 8 == 0 {
            q2_environment_damage(context, ((delta - 30.0) / 2.0).trunc().max(1.0), 22, 0);
        }
    } else {
        context.with_state(|state| {
            state.event = "q2:fall-short".to_string();
        });
    }
}
