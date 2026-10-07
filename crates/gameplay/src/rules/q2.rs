//! quake-2/game/g_combat.c: T_Damage, CheckArmor, CheckPowerArmor, T_RadiusDamage.
use crate::combat::*;
use qa_core::primitives::{DamageEvent, DamageFlags, Vec3};

pub static DAMAGE: DamageRules = DamageRules {
    prepare,
    minimum_health: -999,
    radius,
};

fn power_save(target: &mut DamageTarget<'_>, event: DamageEvent, damage: i32) -> i32 {
    if damage == 0
        || event.flags.contains(DamageFlags::NO_ARMOR)
        || !(target.traits.client || target.traits.monster)
        || *target.power_cells == 0
    {
        return 0;
    }
    let (per_cell, protected) = match target.traits.power_armor {
        PowerArmor::None => return 0,
        PowerArmor::Screen => {
            let incoming = normalized(difference(event.point, target.traits.position));
            if incoming.dot(target.traits.forward) <= 0.3 {
                return 0;
            }
            (1, damage / 3)
        }
        PowerArmor::Shield => (2, 2 * damage / 3),
    };
    let save = (*target.power_cells * per_cell).min(protected);
    // Original integer division can save one point without spending a shield cell.
    *target.power_cells -= save / per_cell;
    save
}

fn prepare(target: &mut DamageTarget<'_>, event: DamageEvent, ctx: DamageContext) -> DamageResult {
    let mut damage = event.amount as i32;
    if !ctx.self_hit && ctx.same_team && ctx.prevent_team_damage {
        damage = 0;
    }
    if ctx.easy_single_player && target.traits.client {
        damage = ((damage as f64 * 0.5) as i32).max(1);
    }
    if !event.flags.contains(DamageFlags::RADIUS)
        && target.traits.monster
        && ctx.attacker_client
        && !target.traits.has_enemy
        && *target.health > 0
    {
        damage *= 2;
    }
    let knockback = if target.traits.no_knockback {
        0
    } else {
        event.knockback
    };
    if !event.flags.contains(DamageFlags::NO_KNOCKBACK)
        && knockback != 0
        && !matches!(
            target.traits.motion,
            Motion::None | Motion::Bounce | Motion::Push | Motion::Stop
        )
    {
        let mass = target.traits.mass.max(50.0);
        let scale = if target.traits.client && ctx.self_hit {
            1600.0
        } else {
            500.0
        };
        // qsrc's unsuffixed literals promote this expression before the vec3_t store.
        let force = scale * f64::from(knockback) / f64::from(mass);
        let direction = normalized(event.direction.unwrap_or_default());
        for axis in 0..3 {
            let delta = (f64::from(direction.0[axis]) * force) as f32;
            target.velocity.0[axis] += delta;
        }
    }
    let protected = !event.flags.contains(DamageFlags::NO_PROTECTION)
        && (target.traits.god || (target.traits.client && target.traits.invincible));
    let mut take = if protected { 0 } else { damage };
    let psave = power_save(target, event, take);
    take -= psave;
    let mut asave = 0;
    if take != 0 && target.traits.client && !event.flags.contains(DamageFlags::NO_ARMOR) {
        let fraction = if event.flags.contains(DamageFlags::ENERGY) {
            target.energy_absorption
        } else {
            *target.absorption
        };
        asave = ((fraction * take as f32).ceil() as i32).min(*target.armor);
        *target.armor -= asave;
        take -= asave;
    }
    let dead = take != 0 && *target.health - take <= 0;
    let pain = take != 0
        && if target.traits.monster {
            !target.traits.ducked
        } else {
            !target.traits.client || !target.traits.god
        };
    DamageResult {
        health_damage: take,
        armor_saved: asave + if protected { damage } else { 0 },
        power_saved: psave,
        knockback,
        reaction: if dead {
            Reaction::Death
        } else if pain {
            Reaction::Pain
        } else {
            Reaction::None
        },
        blocked: protected,
        disable_knockback: dead && (target.traits.client || target.traits.monster),
        ..DamageResult::default()
    }
}

fn radius(blast: Blast, target: BlastTarget) -> Option<(f32, Vec3)> {
    let amount = center_falloff(blast, target, blast.radius)? as i32;
    Some((
        amount as f32,
        difference(target.body.position, blast.origin),
    ))
}
