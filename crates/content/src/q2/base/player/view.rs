//! Q2 player view (`src/content/q2/base/player/view.ts`).

use qa_core::math::{Vec3, Vec4, add3, dot3, normalize3, scale3, sub3, vec3};

use crate::contract::RegularArmorState;
use crate::q2::foundation::entity_services::js_round;
use crate::q2::foundation::host::{Q2Edition, Q2EffectEvent, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop};
use crate::q2::foundation::weapons::vectors::angle_vectors;

use super::types::{Q2CharacterContext, Q2PlayerTimer, Q2PlayerView};

/// Clamp a value.
fn clamp(value: f64, minimum: f64, maximum: f64) -> f64 {
    value.max(minimum).min(maximum)
}

/// Add a screen blend (`addQ2Blend`).
pub fn add_q2_blend(blend: Vec4, color: Vec3, alpha: f64) -> Vec4 {
    if alpha <= 0.0 {
        return blend;
    }
    let total = f64::from(blend.w) + (1.0 - f64::from(blend.w)) * alpha;
    let old = f64::from(blend.w) / total;
    Vec4 {
        x: (f64::from(blend.x) * old + f64::from(color.x) * (1.0 - old)) as f32,
        y: (f64::from(blend.y) * old + f64::from(color.y) * (1.0 - old)) as f32,
        z: (f64::from(blend.z) * old + f64::from(color.z) * (1.0 - old)) as f32,
        w: total as f32,
    }
}

/// Pain animation frames (`q2PainAnimationFrames`).
pub fn q2_pain_animation_frames(ducked: bool, index: i32) -> (i32, i32) {
    if ducked {
        (168, 172)
    } else {
        (53 + index * 4, 57 + index * 4)
    }
}

/// Death animation frames (`q2DeathAnimationFrames`).
pub fn q2_death_animation_frames(ducked: bool, index: i32) -> (i32, i32) {
    if ducked {
        (172, 177)
    } else if index == 0 {
        (177, 183)
    } else if index == 1 {
        (183, 189)
    } else {
        (189, 197)
    }
}

/// Damage feedback (`q2DamageFeedback`).
pub fn q2_damage_feedback(
    context: &mut impl Q2CharacterContext,
    pain_index: i32,
) -> (i32, i32) {
    let now = context.now();
    let powers = context.powerups();
    let snapshot = context.state_snapshot();
    let flashes = (if snapshot.damage_blood != 0.0 { 1 } else { 0 })
        | (if snapshot.damage_armor != 0.0
            && !snapshot.god
            && powers.invulnerability_until <= now
        {
            2
        } else {
            0
        });
    let total = snapshot.damage_blood + snapshot.damage_armor + snapshot.damage_power_armor;
    if total == 0.0 {
        return (flashes, pain_index);
    }
    let mut next_pain = pain_index;
    let movement = context.movement();
    if movement.animate_q2 && snapshot.animation_priority < 3 {
        if !movement.ducked {
            next_pain = (pain_index + 1) % 3;
        }
        let frames = q2_pain_animation_frames(movement.ducked, next_pain);
        context.with_state(|state| {
            state.animation_priority = 3;
            state.animation_end = frames.1;
        });
        context.with_entity(|entity| {
            entity.frame = frames.0;
        });
    }
    let count = total.max(10.0);
    let health = context.combat().map_or(0.0, |combat| combat.health);
    if now > snapshot.pain_debounce && !snapshot.god && powers.invulnerability_until <= now {
        let severity = if health < 25.0 {
            25
        } else if health < 50.0 {
            50
        } else if health < 75.0 {
            75
        } else {
            100
        };
        let variant = (context.random() * 2.0).floor() as i32 + 1;
        context.sound(&format!("*pain{severity}_{variant}.wav"), 2, 1.0, 1.0);
        context.with_state(|state| {
            state.pain_debounce = now + 0.7;
        });
    }
    context.with_state(|state| {
        state.damage_alpha = clamp(state.damage_alpha.max(0.0) + count * 0.01, 0.2, 0.6);
        state.damage_blend = vec3(
            ((state.damage_armor + state.damage_blood) / total) as f32,
            ((state.damage_power_armor + state.damage_armor) / total) as f32,
            (state.damage_armor / total) as f32,
        );
    });
    if snapshot.damage_knockback != 0.0 && health > 0.0 {
        let kick = clamp(
            snapshot.damage_knockback.abs() * 100.0 / health,
            count * 0.5,
            50.0,
        );
        let origin = context.body().origin;
        let direction = normalize3(sub3(snapshot.damage_from, origin));
        let vectors = angle_vectors(movement.view_angles);
        context.with_state(|state| {
            state.damage_roll = kick * f64::from(dot3(direction, vectors.right)) * 0.3;
            state.damage_pitch = kick * -f64::from(dot3(direction, vectors.forward)) * 0.3;
            state.damage_time = now + 0.5;
        });
    }
    context.with_state(|state| {
        state.damage_blood = 0.0;
        state.damage_armor = 0.0;
        state.damage_power_armor = 0.0;
        state.damage_knockback = 0.0;
    });
    (flashes, next_pain)
}

/// Angle difference clamped to +-45.
fn angle_difference(old: f32, current: f32) -> f64 {
    let mut delta = f64::from(old) - f64::from(current);
    if delta > 180.0 {
        delta -= 360.0;
    }
    if delta < -180.0 {
        delta += 360.0;
    }
    clamp(delta, -45.0, 45.0)
}

/// Build the player view (`q2BuildView`).
///
/// Each seat gets a complete source view; no module-global current player/vectors.
pub fn q2_build_view(
    context: &mut impl Q2CharacterContext,
    flashes: i32,
    intermission: bool,
) -> Q2PlayerView {
    let zero = vec3(0.0, 0.0, 0.0);
    let body = context.body();
    let now = context.now();
    let health = context.combat().map_or(0.0, |combat| combat.health);
    let weapon = context.weapon_state();
    let powers = context.powerups();
    let movement = context.movement();
    let rules = context.rules();
    let snapshot = context.state_snapshot();
    let vectors = angle_vectors(movement.view_angles);
    let speed = f64::from(body.velocity.x).hypot(f64::from(body.velocity.y));
    let bob_time = if movement.ducked {
        snapshot.bob_time * 4.0
    } else {
        snapshot.bob_time
    };
    let bob_cycle = bob_time.trunc() as i32;
    let bob = (bob_time * std::f64::consts::PI).sin().abs();
    let mut angles = movement.view_angles;
    let mut kicks = weapon.as_ref().map_or(zero, |weapon| weapon.kick_angles);
    if snapshot.dead {
        angles = vec3(-15.0, snapshot.killer_yaw as f32, 40.0);
        kicks = zero;
    } else {
        let damage_ratio = ((snapshot.damage_time - now) / 0.5).max(0.0);
        let fall_ratio = ((snapshot.fall_time - now) / 0.3).max(0.0);
        if damage_ratio == 0.0 {
            context.with_state(|state| {
                state.damage_pitch = 0.0;
                state.damage_roll = 0.0;
            });
        }
        let duck = if movement.ducked { 6.0 } else { 1.0 };
        let snapshot = context.state_snapshot();
        kicks = vec3(
            (f64::from(kicks.x)
                + damage_ratio * snapshot.damage_pitch
                + fall_ratio * snapshot.fall_value
                + f64::from(dot3(body.velocity, vectors.forward)) * rules.run_pitch
                + bob * rules.bob_pitch * speed * duck) as f32,
            kicks.y,
            (f64::from(kicks.z)
                + damage_ratio * snapshot.damage_roll
                + f64::from(dot3(body.velocity, vectors.right)) * rules.run_roll
                + bob * rules.bob_roll
                    * speed
                    * duck
                    * (if bob_cycle & 1 != 0 { -1.0 } else { 1.0 })) as f32,
        );
    }
    let snapshot = context.state_snapshot();
    let entity = context.entity_snapshot();
    let recoil = weapon.as_ref().map_or(zero, |weapon| weapon.kick_origin);
    let offset = vec3(
        clamp(f64::from(recoil.x), -14.0, 14.0) as f32,
        clamp(f64::from(recoil.y), -14.0, 14.0) as f32,
        clamp(
            f64::from(entity.view_height)
                - ((snapshot.fall_time - now) / 0.3).max(0.0) * snapshot.fall_value * 0.4
                + (bob * speed * rules.bob_up).min(6.0)
                + f64::from(recoil.z),
            -22.0,
            30.0,
        ) as f32,
    );
    let yaw_delta = angle_difference(snapshot.old_view_angles.y, angles.y);
    let gun_angles = vec3(
        (speed * bob * 0.005 + angle_difference(snapshot.old_view_angles.x, angles.x) * 0.2) as f32,
        (speed * bob * 0.01 * (if bob_cycle & 1 != 0 { -1.0 } else { 1.0 }) + yaw_delta * 0.2) as f32,
        (speed * bob * 0.005 * (if bob_cycle & 1 != 0 { -1.0 } else { 1.0 })
            + angle_difference(snapshot.old_view_angles.z, angles.z) * 0.2
            + yaw_delta * 0.1) as f32,
    );
    let gun_offset = add3(
        add3(
            scale3(vectors.forward, rules.gun_offset.y),
            scale3(vectors.right, rules.gun_offset.x),
        ),
        scale3(vectors.up, -rules.gun_offset.z),
    );
    let contents = context.point_contents(add3(body.origin, offset));
    let mut blend = Vec4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 0.0,
    };
    if contents & 9 != 0 {
        blend = add_q2_blend(blend, vec3(1.0, 0.3, 0.0), 0.6);
    } else if contents & 16 != 0 {
        blend = add_q2_blend(blend, vec3(0.0, 0.1, 0.05), 0.6);
    } else if contents & 32 != 0 {
        blend = add_q2_blend(blend, vec3(0.5, 0.3, 0.2), 0.4);
    }
    let power = if powers.quad_until > now {
        Some(("q2:item_quad", powers.quad_until, "items/damage2.wav", vec3(0.0, 0.0, 1.0), 0.08))
    } else if powers.invulnerability_until > now {
        Some(("q2:item_invulnerability", powers.invulnerability_until, "items/protect2.wav", vec3(1.0, 1.0, 0.0), 0.08))
    } else if powers.enviro_until > now {
        Some(("q2:item_enviro", powers.enviro_until, "items/airout.wav", vec3(0.0, 1.0, 0.0), 0.08))
    } else if powers.breather_until > now {
        Some(("q2:item_breather", powers.breather_until, "items/airout.wav", vec3(0.4, 1.0, 0.4), 0.04))
    } else {
        None
    };
    if let Some((_, until, sound, color, alpha)) = power {
        let remaining = js_round((until - now) * 10.0) as i64;
        if remaining == 30 {
            context.sound(sound, 3, 1.0, 1.0);
        }
        if remaining > 30 || remaining & 4 != 0 {
            blend = add_q2_blend(blend, color, alpha);
        }
    }
    blend = add_q2_blend(blend, snapshot.damage_blend, snapshot.damage_alpha);
    blend = add_q2_blend(blend, vec3(0.85, 0.7, 0.3), snapshot.bonus_alpha);
    context.with_state(|state| {
        state.damage_alpha = (state.damage_alpha - 0.06).max(0.0);
        state.bonus_alpha = (state.bonus_alpha - 0.1).max(0.0);
    });
    let combat = context.combat();
    let armor = combat.as_ref().map(|combat| combat.armor.clone());
    let armor_points = match armor.as_ref().map(|armor| &armor.regular) {
        None => 0.0,
        Some(RegularArmorState::None) => 0.0,
        Some(RegularArmorState::Q1 { points, .. })
        | Some(RegularArmorState::Q2 { points, .. })
        | Some(RegularArmorState::Q3 { points, .. })
        | Some(RegularArmorState::Source { points, .. }) => *points,
    };
    let ammo_item = weapon.as_ref().and_then(|weapon| {
        if weapon.q2_name.is_none() {
            None
        } else {
            weapon.ammo.clone()
        }
    });
    let ammo = match ammo_item {
        None => 0.0,
        Some(item) => context.inventory_count(&item),
    };
    let timer = if powers.quad_until > now {
        Some(Q2PlayerTimer {
            item: "q2:item_quad".to_string(),
            seconds: (powers.quad_until - now).trunc() as i32,
        })
    } else if powers.invulnerability_until > now {
        Some(Q2PlayerTimer {
            item: "q2:item_invulnerability".to_string(),
            seconds: (powers.invulnerability_until - now).trunc() as i32,
        })
    } else if powers.enviro_until > now {
        Some(Q2PlayerTimer {
            item: "q2:item_enviro".to_string(),
            seconds: (powers.enviro_until - now).trunc() as i32,
        })
    } else if powers.breather_until > now {
        Some(Q2PlayerTimer {
            item: "q2:item_breather".to_string(),
            seconds: (powers.breather_until - now).trunc() as i32,
        })
    } else {
        None
    };
    let powered_cells = armor.as_ref().and_then(|armor| match &armor.powered {
        crate::contract::PoweredProtectionState::Screen { cells } => Some(*cells),
        crate::contract::PoweredProtectionState::Shield { cells } => Some(*cells),
        crate::contract::PoweredProtectionState::None => None,
    });
    let armor_display = match (armor.as_ref(), powered_cells) {
        (Some(_), Some(cells))
            if armor_points == 0.0 || js_round(now * 10.0) as i64 & 8 != 0 =>
        {
            cells
        }
        _ => armor_points,
    };
    Q2PlayerView {
        angles,
        offset: if intermission { zero } else { offset },
        kick_angles: if intermission { zero } else { kicks },
        gun_angles,
        gun_offset,
        blend: if intermission {
            Vec4 { x: 0.0, y: 0.0, z: 0.0, w: 0.0 }
        } else {
            blend
        },
        fov: if intermission { 90 } else { snapshot.fov },
        underwater: !intermission && contents & 56 != 0,
        flashes,
        health,
        armor: armor_display,
        ammo,
        score: snapshot.score,
        selected_item: snapshot.selected_item.clone(),
        timer,
        spectator: snapshot.spectator,
        layouts: (if snapshot.show_scores || snapshot.show_help || health <= 0.0 || intermission {
            1
        } else {
            0
        }) | (if snapshot.show_inventory && health > 0.0 {
            2
        } else {
            0
        }),
    }
}

/// Run client animation (`q2ClientAnimation`).
pub fn q2_client_animation(context: &mut impl Q2CharacterContext) {
    let snapshot = context.state_snapshot();
    let movement = context.movement();
    if !movement.animate_q2 || snapshot.gibbed {
        return;
    }
    let velocity = context.body().velocity;
    let run = f64::from(velocity.x).hypot(f64::from(velocity.y)) != 0.0;
    advance_q2_player_animation(
        context,
        movement.grounded,
        movement.ducked,
        run,
    );
}

/// Advance player animation (`advanceQ2PlayerAnimation`).
pub fn advance_q2_player_animation(
    context: &mut impl Q2CharacterContext,
    grounded: bool,
    duck: bool,
    run: bool,
) {
    let snapshot = context.state_snapshot();
    let entity = context.entity_snapshot();
    let changed = snapshot.animation_duck != duck && snapshot.animation_priority < 5
        || snapshot.animation_run != run && snapshot.animation_priority == 0
        || !grounded && snapshot.animation_priority <= 1;
    if !changed {
        if snapshot.animation_priority == 6 {
            if entity.frame > snapshot.animation_end {
                context.with_entity(|entity| {
                    entity.frame -= 1;
                });
                return;
            }
        } else if entity.frame < snapshot.animation_end {
            context.with_entity(|entity| {
                entity.frame += 1;
            });
            return;
        }
        if snapshot.animation_priority == 5 {
            return;
        }
        if snapshot.animation_priority == 2 {
            if !grounded {
                return;
            }
            context.with_state(|state| {
                state.animation_priority = 1;
                state.animation_end = 71;
            });
            context.with_entity(|entity| {
                entity.frame = 68;
            });
            return;
        }
    }
    context.with_state(|state| {
        state.animation_priority = 0;
        state.animation_duck = duck;
        state.animation_run = run;
    });
    if !grounded {
        context.with_state(|state| {
            state.animation_priority = 2;
            state.animation_end = 67;
        });
        if entity.frame != 67 {
            context.with_entity(|entity| {
                entity.frame = 66;
            });
        }
    } else if run {
        context.with_state(|state| {
            state.animation_end = if duck { 159 } else { 45 };
        });
        context.with_entity(|entity| {
            entity.frame = if duck { 154 } else { 40 };
        });
    } else {
        context.with_state(|state| {
            state.animation_end = if duck { 153 } else { 39 };
        });
        context.with_entity(|entity| {
            entity.frame = if duck { 135 } else { 0 };
        });
    }
}

/// Run client effects (`q2ClientEffects`).
pub fn q2_client_effects(context: &mut impl Q2CharacterContext) {
    let now = context.now();
    let powers = context.powerups();
    let combat = context.combat();
    let edition = context.edition();
    context.with_entity(|entity| {
        entity.effects = 0;
        entity.render_flags = if edition == Q2Edition::Rerelease {
            32768
        } else {
            0
        };
    });
    let flashing = |until: f64| {
        until > now && (until - now > 3.0 || js_round((until - now) * 10.0) as i64 & 4 != 0)
    };
    if combat.as_ref().map_or(0.0, |combat| combat.health) > 0.0 {
        let snapshot = context.state_snapshot();
        if snapshot.power_armor_time > now {
            if let Some(combat) = combat.as_ref() {
                match &combat.armor.powered {
                    crate::contract::PoweredProtectionState::Screen { .. } => {
                        context.with_entity(|entity| {
                            entity.effects |= 0x200;
                        });
                    }
                    crate::contract::PoweredProtectionState::Shield { .. } => {
                        context.with_entity(|entity| {
                            entity.effects |= 0x100;
                            entity.render_flags |= 0x800;
                        });
                    }
                    crate::contract::PoweredProtectionState::None => {}
                }
            }
        }
        if flashing(powers.quad_until) {
            context.with_entity(|entity| {
                entity.effects |= 0x8000;
            });
        }
        if flashing(powers.invulnerability_until) {
            context.with_entity(|entity| {
                entity.effects |= 0x10000;
            });
        }
        if snapshot.god {
            context.with_entity(|entity| {
                entity.effects |= 0x100;
                entity.render_flags |= 0x1c00;
            });
        }
    }
    let weapon = context.weapon_state();
    let movement = context.movement();
    let weapon_name = weapon.as_ref().and_then(|weapon| weapon.q2_name.clone());
    let weapon_loop = weapon.as_ref().map_or("", |weapon| weapon.loop_sound.as_str());
    let selected = if movement.water_level != 0 && movement.water_type & 24 != 0 {
        "player/fry.wav"
    } else if weapon_name.as_deref() == Some("railgun") {
        "weapons/rg_hum.wav"
    } else if weapon_name.as_deref() == Some("bfg") {
        "weapons/bfg_hum.wav"
    } else {
        weapon_loop
    }
    .to_string();
    let current = context.state_snapshot().loop_sound;
    if selected != current {
        if !current.is_empty() {
            let origin = context.body().origin;
            let actor = context.actor_id();
            context.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(actor),
                origin,
                path: current,
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: Q2SoundLoop::Stop,
                loop_owner: None,
            }));
        }
        context.with_state(|state| {
            state.loop_sound = selected.clone();
        });
        if !selected.is_empty() {
            let origin = context.body().origin;
            let actor = context.actor_id();
            context.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                actor: Some(actor),
                origin,
                path: selected,
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_: Q2SoundLoop::Start,
                loop_owner: None,
            }));
        }
    }
}

/// Emit a named player event effect.
pub fn emit_player_effect(
    context: &mut impl Q2CharacterContext,
    effect: &str,
    origin: Vec3,
) {
    let zero = vec3(0.0, 0.0, 0.0);
    context.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
        effect: effect.to_string(),
        origin,
        direction: zero,
        count: 1,
        color: 0,
    }));
}
