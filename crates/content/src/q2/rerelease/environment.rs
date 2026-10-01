//! Q2 rerelease environment (`src/content/q2/rerelease/environment.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use crate::q2::base::player::types::{Q2CharacterContext, Q2PlayerState};
use crate::q2::foundation::entity_services::js_round;
use crate::q2::foundation::host::Q2Mode;

use super::types::{Q2RereleaseOptions, Q2RereleasePlayerState};

/// Run rerelease world effects (`q2RereleaseWorldEffects`).
pub fn q2_rerelease_world_effects(
    context: &mut impl Q2CharacterContext,
    extra: &mut Q2RereleasePlayerState,
) {
    let now = context.now();
    let snapshot = context.state_snapshot();
    if snapshot.noclip || snapshot.spectator {
        context.with_state(|state| {
            state.air_finished = now + 12.0;
        });
        return;
    }
    let movement = context.movement();
    let water = movement.water_level;
    let old = snapshot.old_water_level;
    let powers = context.powerups();
    let breather = powers.breather_until > now;
    let suit = powers.enviro_until > now;
    context.with_state(|state| {
        state.old_water_level = water;
    });
    if old == 0 && water != 0 {
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
    if old != 0 && water == 0 {
        let origin = context.body().origin;
        context.noise(origin);
        context.sound("player/watr_out.wav", 4, 1.0, 1.0);
        context.with_entity(|entity| {
            entity.flags &= !8;
        });
    }
    if old != 3 && water == 3 {
        context.sound("player/watr_un.wav", 4, 1.0, 1.0);
    }
    if old == 3 && water != 3 {
        let health = context.combat().map_or(0.0, |combat| combat.health);
        if health > 0.0 {
            let air_finished = context.state_snapshot().air_finished;
            if air_finished < now {
                context.sound("player/gasp1.wav", 2, 1.0, 1.0);
                let origin = context.body().origin;
                context.noise(origin);
            } else if air_finished < now + 11.0 {
                context.sound("player/gasp2.wav", 2, 1.0, 1.0);
            }
        }
    }
    if water == 3 {
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
                "*drown1.wav"
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
            context.environment_damage(drown_damage, 17, 2);
        } else if snapshot.air_finished <= now + 3.0 && snapshot.next_drown_time < now {
            context.sound(&format!("player/wade{}.wav", 1 + (now as i64 % 3)), 2, 1.0, 1.0);
            context.with_state(|state| {
                state.next_drown_time = now + 1.0;
            });
        }
    } else {
        context.with_state(|state| {
            state.air_finished = now + 12.0;
            state.drown_damage = 2.0;
        });
    }
    if water != 0 && movement.water_type & 24 != 0 && extra.slime_debounce <= now {
        if movement.water_type & 8 != 0 {
            let health = context.combat().map_or(0.0, |combat| combat.health);
            let pain_debounce = context.state_snapshot().pain_debounce;
            if health > 0.0 && pain_debounce <= now && powers.invulnerability_until < now {
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
            context.environment_damage((if suit { 1.0 } else { 3.0 }) * f64::from(water), 19, 0);
        }
        if movement.water_type & 16 != 0 && !suit {
            context.environment_damage(f64::from(water), 18, 0);
        }
        extra.slime_debounce = now + 0.1;
    }
}

/// Run rerelease falling damage (`q2RereleaseFallingDamage`).
///
/// Rerelease P_FallingDamage consumes pmove's impact delta after each
/// ClientThink.
pub fn q2_rerelease_falling_damage(
    context: &mut impl Q2CharacterContext,
    extra: &mut Q2RereleasePlayerState,
    options: &Q2RereleaseOptions,
    frame_seconds: f64,
) {
    let now = context.now();
    let impact = extra.impact_delta;
    extra.impact_delta = 0.0;
    let movement = context.movement();
    let snapshot: Q2PlayerState = context.state_snapshot();
    let health = context.combat().map_or(0.0, |combat| combat.health);
    if snapshot.dead
        || snapshot.noclip
        || snapshot.spectator
        || health <= 0.0
        || movement.water_level == 3
        || extra.grapple_released_until >= now
        || extra.grapple_attached
    {
        return;
    }
    let mut delta = impact * impact * 0.0001;
    if movement.water_level == 2 {
        delta *= 0.25;
    }
    if movement.water_level == 1 {
        delta *= 0.5;
    }
    if delta < 1.0 {
        return;
    }
    context.with_state(|state| {
        state.bob_time = 0.0;
    });
    if snapshot.landmark_free_fall {
        delta = delta.min(30.0);
        context.with_state(|state| {
            state.landmark_free_fall = false;
            state.landmark_noise_time = now + 0.1;
        });
    }
    if delta < 15.0 {
        if !extra.on_ladder {
            context.with_state(|state| {
                state.event = "q2:footstep".to_string();
            });
        }
        return;
    }
    context.with_state(|state| {
        state.fall_value = (delta * 0.5).min(40.0);
        state.fall_time = now + 0.3 + (0.1 - frame_seconds);
    });
    if delta > 30.0 {
        context.with_state(|state| {
            state.event = if delta >= 55.0 {
                "q2:fall-far".to_string()
            } else {
                "q2:fall".to_string()
            };
            state.pain_debounce = now + frame_seconds;
        });
        if context.mode() != Q2Mode::Deathmatch || !options.deathmatch_no_fall_damage {
            context.environment_damage(((delta - 30.0) / 2.0).trunc().max(1.0), 22, 0);
        }
    } else {
        context.with_state(|state| {
            state.event = "q2:fall-short".to_string();
        });
    }
    if context.combat().map_or(0.0, |combat| combat.health) != 0.0 {
        let origin = context.body().origin;
        context.noise(origin);
    }
}
