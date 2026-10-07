//! quake/progs106/combat.qc: T_Damage, Killed and T_RadiusDamage.
use crate::combat::*;
use qa_core::primitives::{DamageEvent, Vec3};

pub static DAMAGE: DamageRules = DamageRules {
    prepare,
    minimum_health: -99,
    radius,
};

fn prepare(target: &mut DamageTarget<'_>, event: DamageEvent, ctx: DamageContext) -> DamageResult {
    let save = (*target.absorption * event.amount).ceil() as i32;
    let save = if save >= *target.armor {
        *target.absorption = 0.0;
        *target.armor
    } else {
        save
    };
    *target.armor -= save;
    let take = (event.amount - save as f32).ceil() as i32;
    if event.inflictor.is_some() && target.traits.motion == Motion::Walk {
        let dir = normalized(difference(target.traits.position, ctx.inflictor_center));
        for axis in 0..3 {
            target.velocity.0[axis] += dir.0[axis] * event.amount * 8.0;
        }
    }
    if target.traits.god || target.traits.invincible || (ctx.prevent_team_damage && ctx.same_team) {
        return DamageResult {
            armor_saved: save,
            blocked: true,
            ..DamageResult::default()
        };
    }
    let dead = *target.health - take <= 0;
    DamageResult {
        health_damage: take,
        armor_saved: save,
        reaction: if dead {
            Reaction::Death
        } else {
            Reaction::Pain
        },
        disable_damage: dead && !matches!(target.traits.motion, Motion::Push | Motion::None),
        ..DamageResult::default()
    }
}

fn radius(blast: Blast, target: BlastTarget) -> Option<(f32, Vec3)> {
    let amount = center_falloff(blast, target, blast.amount + 40.0)?;
    Some((amount, difference(target.body.position, blast.origin)))
}
