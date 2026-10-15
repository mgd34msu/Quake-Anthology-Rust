use qa_core::primitives::{ModelRotation, ModelRules, RotatedLinkBounds, RuleSetId};
use qa_gameplay::rules::model_rules;

// Native layouts and CLIENT collision admission use the same const boundary.
const STOCK: [ModelRules; 5] = [
    model_rules(RuleSetId::Quake),
    model_rules(RuleSetId::QuakeWorld),
    model_rules(RuleSetId::Quake2),
    model_rules(RuleSetId::Quake2Rerelease),
    model_rules(RuleSetId::Quake3),
];

#[test]
fn native_model_roles_keep_their_transform_and_conservative_link_bounds() {
    assert_eq!(STOCK[0], ModelRules::default());
    assert_eq!(STOCK[0], STOCK[1]);
    assert_eq!(STOCK[2], STOCK[3]);
    assert_eq!(STOCK[2].rotation, ModelRotation::NegativeEuler);
    assert_eq!(STOCK[2].link_bounds, RotatedLinkBounds::MaxAbsCube);
    assert_eq!(STOCK[4].rotation, ModelRotation::TransposeBasis);
    assert_eq!(STOCK[4].link_bounds, RotatedLinkBounds::RadiusCube);
    assert_ne!(STOCK[0], STOCK[2]);
    assert_ne!(STOCK[2], STOCK[4]);
}
