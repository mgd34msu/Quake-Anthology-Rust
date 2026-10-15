//! Bounded legacy body sweep policy, selected independently of trace rules.
use qa_core::primitives::RuleSetId;

#[derive(Clone, Copy, Debug)]
pub struct LegacySweepRules {
    pub stop_when_still: bool,
    pub stop_start_solid: bool,
    pub vertical_solid_stop: bool,
    pub contact_all_solid: bool,
    pub reset_original: bool,
    pub clip_current: bool,
    pub crease_current: bool,
    /// Native SV_FlyMove rejects only negative dots, including its NaN behavior.
    pub reject_negative_only: bool,
    pub time_first: bool,
    pub secondary_plane: bool,
    pub duplicate_plane: bool,
    pub ground_on_brush: bool,
    pub overbounce: f32,
    pub stop_epsilon: f32,
}

const NQ_PLAYER: LegacySweepRules = LegacySweepRules {
    stop_when_still: true,
    stop_start_solid: false,
    vertical_solid_stop: false,
    contact_all_solid: false,
    reset_original: true,
    clip_current: false,
    crease_current: true,
    reject_negative_only: false,
    time_first: false,
    secondary_plane: false,
    duplicate_plane: false,
    ground_on_brush: true,
    overbounce: 1.0,
    stop_epsilon: 0.1,
};
const QW_PLAYER: LegacySweepRules = LegacySweepRules {
    stop_when_still: false,
    stop_start_solid: true,
    reset_original: false,
    crease_current: false,
    ground_on_brush: false,
    ..NQ_PLAYER
};
const Q2_PLAYER: LegacySweepRules = LegacySweepRules {
    stop_start_solid: false,
    vertical_solid_stop: true,
    clip_current: true,
    overbounce: 1.01,
    ..QW_PLAYER
};
const RR_PLAYER: LegacySweepRules = LegacySweepRules {
    contact_all_solid: true,
    secondary_plane: true,
    duplicate_plane: true,
    ..Q2_PLAYER
};
const NQ_ENTITY: LegacySweepRules = LegacySweepRules {
    reject_negative_only: true,
    time_first: true,
    ..NQ_PLAYER
};
const QW_ENTITY: LegacySweepRules = LegacySweepRules {
    stop_when_still: false,
    ..NQ_ENTITY
};

pub const fn player_sweep(rules: RuleSetId) -> Option<LegacySweepRules> {
    [
        Some(NQ_PLAYER),
        Some(QW_PLAYER),
        Some(Q2_PLAYER),
        Some(RR_PLAYER),
        None,
    ][rules as usize]
}

pub const fn entity_sweep(rules: RuleSetId) -> Option<LegacySweepRules> {
    [Some(NQ_ENTITY), Some(QW_ENTITY), None, None, None][rules as usize]
}
