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
    pub fly_flag: u32,
    pub swim_flag: u32,
    pub partial_ground_flag: u32,
    pub height: f32,
    pub follow_above: f32,
    pub follow_below: f32,
    pub follow_adjust: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PushRules {
    pub ground_flag: u32,
    pub time_before_move: bool,
    pub test_preexisting_stuck: bool,
    pub traced_passengers: bool,
    pub clear_ground_nonwalk: bool,
    pub leave_behind: bool,
    pub relink_exceptions: bool,
    pub post_think_snap: Option<f32>,
}
impl PushRules {
    pub const fn candidate(&self, move_type: f32) -> bool {
        move_type != 7.0 && move_type != 0.0 && move_type != 8.0
    }
    pub const fn walking(&self, move_type: f32) -> bool {
        move_type == 3.0
    }
    pub const fn corpse(&self, solid: f32) -> bool {
        solid == 0.0 || solid == 1.0
    }
    pub const fn solid_not(&self) -> f32 {
        0.0
    }
    pub const fn solid_bsp(&self) -> f32 {
        4.0
    }
}

pub const fn push_rules(source: RuleSetId) -> Option<PushRules> {
    match source {
        RuleSetId::Quake | RuleSetId::QuakeWorld => {
            let world = matches!(source, RuleSetId::QuakeWorld);
            Some(PushRules {
                ground_flag: GROUND,
                time_before_move: !world,
                test_preexisting_stuck: world,
                traced_passengers: !world,
                clear_ground_nonwalk: !world,
                leave_behind: world,
                relink_exceptions: world,
                post_think_snap: if world { Some(1.0 / 64.0) } else { None },
            })
        }
        _ => None,
    }
}

impl StepRules {
    pub const fn mobility_flags(&self) -> u32 {
        self.ground_flag | self.fly_flag | self.swim_flag
    }

    pub const fn falls(&self, flags: u32) -> bool {
        flags & self.mobility_flags() == 0
    }

    pub fn landing_sound(&self, velocity_z: f32, world_gravity: f32) -> bool {
        // Native comparison promotes through the double literal before gravity
        // or velocity checking can change the captured falling velocity.
        f64::from(velocity_z) < f64::from(world_gravity) * -0.1
    }
}

impl TossRules {
    pub const fn gravity_applies(&self, move_type: f32) -> bool {
        move_type != 5.0 && !Self::is_missile(move_type)
    }

    pub const fn is_missile(move_type: f32) -> bool {
        move_type == 9.0
    }

    pub const fn is_bounce(&self, move_type: f32) -> bool {
        move_type == 10.0
    }

    pub const fn ignores_boxes(solid: f32) -> bool {
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
            fly_flag: 1,
            swim_flag: 2,
            partial_ground_flag: 1024,
            height: 18.0,
            follow_above: 40.0,
            follow_below: 30.0,
            follow_adjust: 8.0,
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
