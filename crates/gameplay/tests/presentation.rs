use qa_core::primitives::{RuleSetId, ThinkTime};
use qa_gameplay::rules::presentation::{center_y, lightstyle, text_rules};

#[test]
fn lightstyle_native_clock_and_software_scales_keep_original_rounding() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        assert_eq!(
            lightstyle(rules, b"amz", ThinkTime::Seconds(0.099))
                .unwrap()
                .indexed_scale,
            0
        );
        let m = lightstyle(rules, b"amz", ThinkTime::Seconds(0.1)).unwrap();
        assert_eq!(m.indexed_scale, 264);
        assert_eq!(m.rgb.map(f32::to_bits), [1.03125f32.to_bits(); 3]);
        assert_eq!(
            lightstyle(rules, b"", ThinkTime::Seconds(12.0))
                .unwrap()
                .indexed_scale,
            256
        );
    }
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let sample = lightstyle(rules, b"bn", ThinkTime::Milliseconds(100)).unwrap();
        let value = 13.0f32 / 12.0;
        assert_eq!(sample.rgb.map(f32::to_bits), [value.to_bits(); 3]);
        assert_eq!(
            sample.indexed_scale,
            ((value + value + value) * 128.0) as u16
        );
        assert_eq!(
            sample.interpolation_unimplemented,
            rules == RuleSetId::Quake2Rerelease
        );
        assert_eq!(
            lightstyle(rules, b"", ThinkTime::Milliseconds(0))
                .unwrap()
                .indexed_scale,
            384
        );
    }
    assert!(lightstyle(RuleSetId::Quake3, b"m", ThinkTime::Milliseconds(0)).is_none());
}
#[test]
fn text_roles_preserve_native_placement_without_inventing_q3_center_hud() {
    assert_eq!(center_y(200, 4), 70);
    assert_eq!(center_y(200, 5), 48);
    assert!(!text_rules(RuleSetId::Quake).colors);
    assert!(text_rules(RuleSetId::Quake3).colors);
    assert!(!text_rules(RuleSetId::Quake3).center);
    assert_eq!(text_rules(RuleSetId::Quake2).layout_stat, Some(13));
}

#[test]
fn native_camera_and_weapon_rules_share_angle_math_without_coupling_roles() {
    use qa_core::primitives::Vec3;
    use qa_gameplay::rules::presentation::view_rules;
    let base = Vec3([5.0, 20.0, -1.0]);
    let old = Vec3([350.0, -180.0, 1.0]);
    let new = Vec3([10.0, 0.0, -1.0]);
    let nq = view_rules(RuleSetId::Quake);
    let direct = nq.angles(base, new, Some(old), 0.5, true);
    assert_eq!(direct, Vec3([15.0, 20.0, -2.0]));
    assert_eq!(nq.weapon_angles(base, direct), base);
    for source in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let rules = view_rules(source);
        let presented = rules.angles(base, new, Some(old), 0.5, true);
        assert_eq!(presented, Vec3([365.0, -70.0, -1.0]));
        assert_eq!(rules.weapon_angles(base, presented), presented);
        assert_eq!(rules.angles(base, new, None, 0.5, true), direct);
    }
    assert_eq!(
        view_rules(RuleSetId::Quake2Rerelease).angles(base, new, Some(old), 0.5, false),
        base
    );
    for source in [RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        assert_eq!(
            view_rules(source).angles(base, new, Some(old), 0.5, true),
            base
        );
    }
}
