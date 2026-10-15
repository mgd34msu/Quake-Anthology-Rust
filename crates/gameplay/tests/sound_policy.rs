use qa_core::primitives::{AttenuationPrecision, DistanceAttenuation, RuleSetId};
use qa_gameplay::rules::{loop_spatial_policy, native_sound_spatial, sound_policy};

#[test]
fn one_shot_and_entity_loop_coefficients_are_independent_cached_capabilities() {
    for (rules, precision) in [
        (RuleSetId::Quake2, AttenuationPrecision::F64),
        (RuleSetId::Quake2Rerelease, AttenuationPrecision::F32),
    ] {
        let ordinary = sound_policy(rules);
        assert_eq!(ordinary.distance_offset, 80.0);
        assert_eq!(ordinary.separation, 0.5);
        assert_eq!(
            ordinary.attenuation,
            DistanceAttenuation::Product {
                ordinary: 0.0005,
                match_value: 3.0,
                matched: 0.001,
                precision,
            }
        );
        assert_eq!(native_sound_spatial(rules), ordinary);
        assert_eq!(
            loop_spatial_policy(rules).attenuation,
            DistanceAttenuation::Linear(0.003)
        );
    }
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        assert_eq!(
            sound_policy(rules).attenuation,
            DistanceAttenuation::Linear(0.001)
        );
        assert_eq!(loop_spatial_policy(rules), sound_policy(rules));
    }
    assert_eq!(
        sound_policy(RuleSetId::Quake3).attenuation,
        DistanceAttenuation::Linear(1.0 / 1170.0)
    );
    assert_eq!(
        loop_spatial_policy(RuleSetId::Quake3).attenuation,
        DistanceAttenuation::Linear(0.0008)
    );
    assert_eq!(
        native_sound_spatial(RuleSetId::Quake3),
        loop_spatial_policy(RuleSetId::Quake3)
    );
}
