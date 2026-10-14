use qa_core::primitives::{LoopDoppler, RuleSetId, Vec3};
use qa_gameplay::rules::loop_rules;

#[test]
fn native_rate_policy_keeps_squared_distance_order_and_real_loop_exclusion() {
    let rules = loop_rules(RuleSetId::Quake3).unwrap();
    let native = rules.native(false);
    let rate = rules.doppler(
        native,
        Vec3::default(),
        Vec3([1., 0., 0.]),
        Vec3([99., 0., 0.]),
    );
    assert_eq!(
        rate,
        Some(LoopDoppler {
            scale: 100.,
            previous_scale: 1.
        })
    );
    assert_eq!(
        rules.doppler(
            rules.native(true),
            Vec3::default(),
            Vec3([1., 0., 0.]),
            Vec3([99., 0., 0.])
        ),
        None
    );
    for (origin, velocity) in [
        (Vec3([1., 0., 0.]), Vec3::default()),
        (Vec3([1., 0., 0.]), Vec3([-1., 0., 0.])),
        (Vec3::default(), Vec3([99., 0., 0.])),
    ] {
        assert_eq!(
            rules.doppler(native, Vec3::default(), origin, velocity),
            None
        );
    }
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let rules = loop_rules(source).unwrap();
        assert_eq!(
            rules.doppler(
                rules.native(false),
                Vec3::default(),
                Vec3([1., 0., 0.]),
                Vec3([99., 0., 0.])
            ),
            None
        );
    }
}
