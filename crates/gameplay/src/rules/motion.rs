//! Nonclient motion policy, selected by the entity's owning module at load.
use qa_core::primitives::RuleSetId;

#[derive(Clone, Copy, Debug)]
pub struct TossRules {
    pub clear_ground_on_rise: bool,
    pub entity_gravity: bool,
    pub ground_flag: u32,
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
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => Some(TossRules {
            clear_ground_on_rise: matches!(rules, RuleSetId::QuakeWorld),
            entity_gravity: matches!(rules, RuleSetId::Quake),
            ground_flag: 512,
        }),
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease | RuleSetId::Quake3 => None,
    }
}
