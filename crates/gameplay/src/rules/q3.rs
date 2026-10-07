//! quake-iii-arena/code/game/g_combat.c: G_Damage, CheckArmor and G_RadiusDamage.
use crate::combat::*;
use qa_core::primitives::{DamageEvent, DamageFlags, Vec3};

pub static DAMAGE: DamageRules = DamageRules {
    prepare,
    minimum_health: -999,
    radius,
};

fn prepare(target: &mut DamageTarget<'_>, event: DamageEvent, ctx: DamageContext) -> DamageResult {
    if ctx.intermission || (target.traits.client && target.traits.noclip) {
        return DamageResult {
            blocked: true,
            ..DamageResult::default()
        };
    }
    let mut damage = event.amount as i32;
    if ctx.attacker_client && !ctx.self_hit {
        damage = damage * ctx.attacker_handicap / 100;
    }
    let knockback = if target.traits.no_knockback
        || event.flags.contains(DamageFlags::NO_KNOCKBACK)
        || event.direction.is_none()
    {
        0
    } else {
        damage.min(200)
    };
    if knockback != 0 && target.traits.client {
        impulse(
            target,
            normalized(event.direction.unwrap_or_default()),
            ctx.knockback_scale * knockback as f32 / 200.0,
        );
        if target.traits.knockback_time_ms == 0 {
            target.traits.knockback_time_ms = (knockback * 2).clamp(50, 200);
        }
    }
    if !event.flags.contains(DamageFlags::NO_PROTECTION)
        && ((!ctx.self_hit && ctx.same_team && ctx.prevent_team_damage) || target.traits.god)
    {
        return DamageResult {
            knockback,
            blocked: true,
            ..DamageResult::default()
        };
    }
    if target.traits.battlesuit && target.traits.client {
        if event
            .flags
            .contains(DamageFlags::RADIUS | DamageFlags::FALLING)
        {
            return DamageResult {
                knockback,
                blocked: true,
                ..DamageResult::default()
            };
        }
        damage = (f64::from(damage) * 0.5) as i32;
    }
    if ctx.self_hit {
        damage = (f64::from(damage) * 0.5) as i32;
    }
    damage = damage.max(1);
    let save = if target.traits.client && !event.flags.contains(DamageFlags::NO_ARMOR) {
        // ARMOR_PROTECTION is the original unsuffixed 0.66 macro.
        ((f64::from(damage) * 0.66).ceil() as i32).min(*target.armor)
    } else {
        0
    };
    *target.armor -= save;
    let take = damage - save;
    let dead = take != 0 && *target.health - take <= 0;
    DamageResult {
        health_damage: take,
        armor_saved: save,
        knockback,
        reaction: if dead {
            Reaction::Death
        } else if take != 0 {
            Reaction::Pain
        } else {
            Reaction::None
        },
        disable_knockback: dead && target.traits.client,
        ..DamageResult::default()
    }
}

fn radius(blast: Blast, target: BlastTarget) -> Option<(f32, Vec3)> {
    let radius = blast.radius.max(1.0);
    let out = Vec3(std::array::from_fn(|axis| {
        let min = target.body.position.0[axis] + target.body.mins.0[axis];
        let max = target.body.position.0[axis] + target.body.maxs.0[axis];
        if blast.origin.0[axis] < min {
            min - blast.origin.0[axis]
        } else if blast.origin.0[axis] > max {
            blast.origin.0[axis] - max
        } else {
            0.0
        }
    }));
    let distance = length(out);
    if distance >= radius {
        return None;
    }
    let amount = (f64::from(blast.amount) * (1.0 - f64::from(distance / radius))) as f32;
    let amount = (amount * target.multiplier) as i32;
    let mut direction = difference(target.body.position, blast.origin);
    direction.0[2] += 24.0;
    Some((amount as f32, direction))
}
