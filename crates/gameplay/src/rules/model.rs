use qa_core::primitives::{ModelRotation, ModelRules, RotatedLinkBounds, RuleSetId};

const Q1: ModelRules = ModelRules {
    rotation: ModelRotation::TranslationOnly,
    link_bounds: RotatedLinkBounds::Unrotated,
};
const Q2: ModelRules = ModelRules {
    rotation: ModelRotation::NegativeEuler,
    link_bounds: RotatedLinkBounds::MaxAbsCube,
};
const Q3: ModelRules = ModelRules {
    rotation: ModelRotation::TransposeBasis,
    link_bounds: RotatedLinkBounds::RadiusCube,
};

/// Model-role behavior is independent of geometry and caller trace/movement.
pub const fn model_rules(source: RuleSetId) -> ModelRules {
    [Q1, Q1, Q2, Q2, Q3][source as usize]
}
