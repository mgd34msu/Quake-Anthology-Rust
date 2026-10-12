use qa_core::primitives::{RuleSetId, SpatialPolicy};

const Q1: SpatialPolicy = SpatialPolicy {
    distance_offset: 0.0,
    distance_scale: 0.001,
    separation: 1.0,
};
const Q2: SpatialPolicy = SpatialPolicy {
    distance_offset: 80.0,
    distance_scale: 0.001,
    separation: 0.5,
};
const Q3: SpatialPolicy = SpatialPolicy {
    distance_offset: 80.0,
    distance_scale: 1.0 / (1250.0 - 80.0),
    separation: 0.5,
};
const POLICIES: [SpatialPolicy; 5] = [Q1, Q1, Q2, Q2, Q3];

pub const fn sound_policy(rules: RuleSetId) -> SpatialPolicy {
    POLICIES[rules as usize]
}
