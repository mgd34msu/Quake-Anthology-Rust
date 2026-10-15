//! Nonclient motion policy, selected by the entity's owning module at load.
use qa_core::primitives::RuleSetId;

#[derive(Clone, Copy, Debug)]
pub struct GravityRules {
    pub entity_scale: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct TossRules {
    pub clear_ground_on_rise: bool,
    pub gravity: GravityRules,
    pub ground_flag: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct StepRules {
    pub gravity: GravityRules,
    pub ground_flag: u32,
    pub no_fall_flags: u32,
}

impl StepRules {
    pub const fn falls(&self, flags: u32) -> bool {
        flags & self.no_fall_flags == 0
    }

    pub fn landing_sound(&self, velocity_z: f32, world_gravity: f32) -> bool {
        // Native comparison promotes through the double literal before gravity
        // or velocity checking can change the captured falling velocity.
        f64::from(velocity_z) < f64::from(world_gravity) * -0.1
    }
}

impl TossRules {
    pub const fn gravity_applies(&self, move_type: f32) -> bool {
        move_type != 5.0 && !self.is_missile(move_type)
    }

    pub const fn is_missile(&self, move_type: f32) -> bool {
        move_type == 9.0
    }

    pub const fn is_bounce(&self, move_type: f32) -> bool {
        move_type == 10.0
    }

    pub const fn ignores_boxes(&self, solid: f32) -> bool {
        solid == 0.0 || solid == 1.0
    }
}

pub const fn toss_rules(rules: RuleSetId) -> Option<TossRules> {
    match gravity_rules(rules) {
        Some(gravity) => Some(TossRules {
            clear_ground_on_rise: matches!(rules, RuleSetId::QuakeWorld),
            gravity,
            ground_flag: GROUND,
        }),
        None => None,
    }
}

pub const fn step_rules(rules: RuleSetId) -> Option<StepRules> {
    match gravity_rules(rules) {
        Some(gravity) => Some(StepRules {
            gravity,
            ground_flag: GROUND,
            no_fall_flags: GROUND | 1 | 2,
        }),
        None => None,
    }
}

const GROUND: u32 = 512;

const fn gravity_rules(rules: RuleSetId) -> Option<GravityRules> {
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => Some(GravityRules {
            entity_scale: matches!(rules, RuleSetId::Quake),
        }),
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease | RuleSetId::Quake3 => None,
    }
}
