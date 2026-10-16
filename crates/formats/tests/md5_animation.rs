use qa_core::primitives::Vec3;
use qa_formats::model::{Md5Animation, Md5ScaleDiagnostic, Model};

const MESH: &[u8] = include_bytes!("fixtures/models/mesh.md5mesh");

const ANIMATION: &str = r#"MD5Version 10
commandline "authored hierarchy"
numFrames 2
numJoints 3
frameRate 24
numAnimatedComponents 6
hierarchy {
    "Root" -1 33 0
    "Child" 0 10 2
    "Tip" 1 20 4
}
bounds {
    ( -8 10 20 ) ( 10 30 50 )
    ( 90 10 20 ) ( 120 40 50 )
}
baseframe {
    ( 10 20 30 ) ( 0 0 0 )
    ( 2 3 4 ) ( 0 0 0 )
    ( 5 6 7 ) ( 0 0 0 )
}
frame 1 { 100 0 8 1 9 0 }
frame 0 { 1 1 3 0 7 1 }
"#;

const SINGLE: &[u8] = br#"MD5Version 10
commandline "existing mesh"
numFrames 1
numJoints 1
frameRate 20
numAnimatedComponents 0
hierarchy { "origin" -1 0 0 }
bounds { ( 1 2 3 ) ( 2 3 3 ) }
baseframe { ( 1 2 3 ) ( 0 0 0 ) }
frame 0 { }
"#;

fn mesh(joints: &str) -> String {
    std::str::from_utf8(MESH)
        .unwrap()
        .replace("numJoints 1", "numJoints 3")
        .replace("joints { \"origin\" -1 ( 1 2 3 ) ( 0 0 0 ) }", joints)
}

const JOINTS: &str = r#"joints {
    "Root" -1 ( 1 20 30 ) ( 0 0 1 )
    "Child" 0 ( -1 17 34 ) ( 0 0 1 )
    "Tip" 1 ( -6 11 41 ) ( -1 0 0 )
}"#;

#[test]
fn indexed_frames_apply_component_flags_before_parent_rotation() {
    let animation = Md5Animation::parse(ANIMATION.as_bytes(), None).unwrap();
    assert_eq!(animation.command_line, b"authored hierarchy");
    assert_eq!(animation.frame_rate, 24);
    assert_eq!(animation.hierarchy.len(), 3);
    assert_eq!(animation.hierarchy[0].name, b"Root");
    assert_eq!(animation.hierarchy[0].parent, None);
    assert_eq!(animation.hierarchy[1].parent, Some(0));
    assert_eq!(animation.hierarchy[2].parent, Some(1));
    assert_eq!(animation.hierarchy[0].flags, 33);
    assert_eq!(animation.hierarchy[1].start_component, 2);
    assert_eq!(animation.base_frame[0].position, Vec3([10.0, 20.0, 30.0]));
    assert_eq!(animation.base_frame[2].orientation, [0.0, 0.0, 0.0, -1.0]);
    assert_eq!(animation.poses.len(), 6);
    assert_eq!(animation.frames[0].joints.range(), 0..3);
    assert_eq!(animation.frames[1].joints.range(), 3..6);
    assert_eq!(animation.frames[0].bounds.mins, Vec3([-8.0, 10.0, 20.0]));
    assert_eq!(animation.frames[1].bounds.maxs, Vec3([120.0, 40.0, 50.0]));

    // Frame zero is declared last. Its root rotates both descendants by 180° Z.
    let first = animation.frame(0).unwrap();
    assert_eq!(first[0].position, Vec3([1.0, 20.0, 30.0]));
    assert_eq!(first[0].orientation, [0.0, 0.0, 1.0, 0.0]);
    assert_eq!(first[1].position, Vec3([-1.0, 17.0, 34.0]));
    assert_eq!(first[2].position, Vec3([-6.0, 11.0, 41.0]));
    assert_eq!(first[2].orientation, [1.0, 0.0, 0.0, 0.0]);

    // Frame one has an identity root and a child rotated by 180° X.
    let second = animation.frame(1).unwrap();
    assert_eq!(second[0].position, Vec3([100.0, 20.0, 30.0]));
    assert_eq!(second[1].position, Vec3([102.0, 28.0, 34.0]));
    assert_eq!(second[2].position, Vec3([107.0, 22.0, 25.0]));
    assert_eq!(second[2].orientation, [1.0, 0.0, 0.0, 0.0]);
    assert!(
        first
            .iter()
            .chain(second)
            .all(|joint| joint.scale == Vec3([1.0; 3]))
    );
    assert!(animation.frame(2).is_none());
    assert!(animation.frame(u32::MAX).is_none());
    assert!(animation.scale_diagnostics.is_empty());
    assert!(std::ptr::eq(first.as_ptr(), animation.poses.as_ptr()));
    assert!(std::ptr::eq(second.as_ptr(), animation.poses[3..].as_ptr()));

    let fast = std::str::from_utf8(SINGLE)
        .unwrap()
        .replace("frameRate 20", "frameRate 1001");
    assert_eq!(
        Md5Animation::parse(fast.as_bytes(), None)
            .unwrap()
            .frame_rate,
        1001
    );
}

#[test]
fn optional_scales_affect_only_the_selected_joint_local_position() {
    let scales = br#"{
        "Root": { "scale_positions": true, "1": 2 },
        "Child": { "scale_positions": true, "1": 3 },
        "Tip": { "1": -2 }
    }"#;
    let animation = Md5Animation::parse(ANIMATION.as_bytes(), Some(scales)).unwrap();
    assert!(animation.scale_diagnostics.is_empty());
    assert!(animation.hierarchy[0].scale_positions);
    assert!(animation.hierarchy[1].scale_positions);
    assert!(!animation.hierarchy[2].scale_positions);
    assert_eq!(
        animation.frame(0).unwrap()[2].position,
        Vec3([-6.0, 11.0, 41.0])
    );

    let frame = animation.frame(1).unwrap();
    assert_eq!(frame[0].scale, Vec3([2.0; 3]));
    assert_eq!(frame[0].position, Vec3([200.0, 40.0, 60.0]));
    assert_eq!(frame[1].scale, Vec3([3.0; 3]));
    assert_eq!(frame[1].position, Vec3([206.0, 64.0, 72.0]));
    // Tip's unscaled (5,6,9) translation is rotated by Child; neither ancestor
    // scale multiplies it, and Tip's own scale does not imply scale_positions.
    assert_eq!(frame[2].scale, Vec3([-2.0; 3]));
    assert_eq!(frame[2].position, Vec3([211.0, 58.0, 63.0]));

    let zero = Md5Animation::parse(
        SINGLE,
        Some(br#"{"origin":{"scale_positions":true,"0":0}}"#),
    )
    .unwrap();
    assert_eq!(zero.frame(0).unwrap()[0].position, Vec3([0.0; 3]));
    assert_eq!(zero.frame(0).unwrap()[0].scale, Vec3([0.0; 3]));
}

#[test]
fn optional_scale_failures_keep_the_animation_and_prior_accepted_values() {
    for scales in [
        &b"not json"[..],
        &b"[]"[..],
        &br#"{"origin":{"0":2},"broken":"#[..],
    ] {
        let animation = Md5Animation::parse(SINGLE, Some(scales)).unwrap();
        assert_eq!(
            animation.scale_diagnostics,
            [Md5ScaleDiagnostic::InvalidData]
        );
        assert_eq!(
            animation.frame(0).unwrap()[0].position,
            Vec3([1.0, 2.0, 3.0])
        );
        assert_eq!(animation.frame(0).unwrap()[0].scale, Vec3([1.0; 3]));
    }

    let animation = Md5Animation::parse(
        ANIMATION.as_bytes(),
        Some(br#"{"Root":{"1":2},"absent":{"1":7},"Child":{"99":4,"1":1e1000},"Tip":false}"#),
    )
    .unwrap();
    assert_eq!(
        animation.scale_diagnostics,
        [
            Md5ScaleDiagnostic::UnknownJoint(b"absent"),
            Md5ScaleDiagnostic::InvalidFrame(b"99"),
            Md5ScaleDiagnostic::InvalidScale(b"1e1000"),
            Md5ScaleDiagnostic::InvalidData,
        ]
    );
    assert_eq!(animation.frame(1).unwrap()[0].scale, Vec3([2.0; 3]));
    assert_eq!(animation.frame(1).unwrap()[1].scale, Vec3([1.0; 3]));
    assert_eq!(animation.frame(1).unwrap()[2].scale, Vec3([1.0; 3]));
    assert_eq!(
        animation.frame(1).unwrap()[2].position,
        Vec3([107.0, 22.0, 25.0])
    );
}

#[test]
fn scale_entries_keep_raw_joint_keys_and_apply_duplicates_in_source_order() {
    let animation = Md5Animation::parse(
        SINGLE,
        Some(
            br#"{"or\u0069gin":{"0":9},"origin":{"0":2,"0":3},"origin":{"scale_positions":true,"0":4}}"#,
        ),
    )
    .unwrap();
    assert_eq!(
        animation.scale_diagnostics,
        [Md5ScaleDiagnostic::UnknownJoint(br"or\u0069gin")]
    );
    assert!(animation.hierarchy[0].scale_positions);
    assert_eq!(animation.frame(0).unwrap()[0].scale, Vec3([4.0; 3]));
    assert_eq!(
        animation.frame(0).unwrap()[0].position,
        Vec3([4.0, 8.0, 12.0])
    );
}

#[test]
fn non_axis_parent_rotation_composes_before_the_child_local_rotation() {
    let animation = Md5Animation::parse(
        br#"MD5Version 10
commandline "non-axis composition"
numFrames 1
numJoints 3
frameRate 24
numAnimatedComponents 0
hierarchy { "Root" -1 0 0 "Child" 0 0 0 "Tip" 1 0 0 }
bounds { ( -10 -10 -10 ) ( 10 10 10 ) }
baseframe {
    ( 1 2 3 ) ( 0.5 0.5 0.5 )
    ( 4 5 6 ) ( 0.5 0 0 )
    ( 0 1 0 ) ( 0 0 0 )
}
frame 0 { }
"#,
        None,
    )
    .unwrap();
    let frame = animation.frame(0).unwrap();
    // Root is -120° about (1,1,1): (x,y,z) becomes (y,z,x). Child then
    // contributes -60° about local X, so Tip's local Y becomes (1/2,-√3/2,0).
    assert_eq!(frame[0].orientation, [0.5, 0.5, 0.5, -0.5]);
    assert_eq!(frame[1].position, Vec3([6.0, 8.0, 7.0]));
    for (actual, expected) in frame[1].orientation.into_iter().zip([
        -0.683_012_7,
        -0.183_012_7,
        -0.683_012_7,
        0.183_012_7,
    ]) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
    for (actual, expected) in frame[2].position.0.into_iter().zip([6.5, 7.133_974_6, 7.0]) {
        assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
    }
}

#[test]
fn mesh_pairing_checks_the_exact_joint_names_and_parent_structure() {
    let single = Md5Animation::parse(SINGLE, None).unwrap();
    let existing = Model::parse(MESH).unwrap();
    single.validate_mesh(&existing).unwrap();

    let animation = Md5Animation::parse(ANIMATION.as_bytes(), None).unwrap();
    let text = mesh(JOINTS);
    let model = Model::parse(text.as_bytes()).unwrap();
    animation.validate_mesh(&model).unwrap();
    assert!(animation.validate_mesh(&existing).is_err());
    assert!(single.validate_mesh(&model).is_err());

    for joints in [
        JOINTS.replace("\"Child\"", "\"child\""),
        JOINTS.replace("\"Child\" 0", "\"Child\" -1"),
        JOINTS.replace("\"Tip\" 1", "\"Tip\" 0"),
    ] {
        let text = mesh(&joints);
        let model = Model::parse(text.as_bytes()).unwrap();
        assert!(animation.validate_mesh(&model).is_err());
    }
    let md3 = Model::parse(include_bytes!("fixtures/models/mesh.md3")).unwrap();
    assert!(animation.validate_mesh(&md3).is_err());
}

#[test]
fn malformed_headers_hierarchies_and_component_spans_are_rejected() {
    for (from, to) in [
        ("MD5Version 10", "MD5Version 9"),
        ("numFrames 2", "numFrames 0"),
        ("numFrames 2", "numFrames -1"),
        ("numJoints 3", "numJoints 0"),
        ("numJoints 3", "numJoints 257"),
        ("frameRate 24", "frameRate 0"),
        ("frameRate 24", "frameRate -1"),
        ("frameRate 24", "frameRate 2147483648"),
        ("numAnimatedComponents 6", "numAnimatedComponents 19"),
        ("\"Root\" -1 33 0", "\"Root\" -1 64 0"),
        ("\"Root\" -1 33 0", "\"Root\" -1 -1 0"),
        ("\"Tip\" 1 20 4", "\"Tip\" 1 20 5"),
        ("\"Tip\" 1 20 4", "\"Tip\" 1 20 -1"),
        ("\"Root\" -1", "\"Root\" 0"),
        ("\"Root\" -1", "\"Root\" 1"),
        ("\"Child\" 0", "\"Child\" 1"),
        ("\"Child\" 0", "\"Child\" 2"),
        ("\"Tip\" 1", "\"Tip\" 3"),
        ("\"Root\" -1", "\"Root\" -2"),
        ("( -8 10 20 )", "( 11 10 20 )"),
        ("( 10 20 30 )", "( 1e100 20 30 )"),
        ("( 10 20 30 )", "( NaN 20 30 )"),
    ] {
        let invalid = ANIMATION.replacen(from, to, 1);
        assert!(
            Md5Animation::parse(invalid.as_bytes(), None).is_err(),
            "{from} -> {to}"
        );
    }
}

#[test]
fn frame_indices_counts_and_truncated_text_cannot_leave_partial_poses() {
    for (from, to) in [
        ("frame 1 {", "frame 0 {"),
        ("frame 1 {", "frame 2 {"),
        ("frame 1 {", "frame -1 {"),
        ("frame 1 {", "frame 0.5 {"),
        ("100 0 8 1 9 0", "100 0 8 1 9"),
        ("100 0 8 1 9 0", "100 0 8 1 9 0 7"),
        ("100 0 8 1 9 0", "100 0 8 1 9 inf"),
    ] {
        let invalid = ANIMATION.replacen(from, to, 1);
        assert!(
            Md5Animation::parse(invalid.as_bytes(), None).is_err(),
            "{from} -> {to}"
        );
    }
    let end = ANIMATION.rfind('}').unwrap();
    for prefix in 0..=end {
        assert!(
            Md5Animation::parse(&ANIMATION.as_bytes()[..prefix], None).is_err(),
            "prefix {prefix}"
        );
    }
    assert!(Md5Animation::parse(format!("{ANIMATION} frame 0 {{ }}").as_bytes(), None).is_err());
    let comments = format!("/* animation */\n{ANIMATION}// trailing\n");
    assert!(Md5Animation::parse(comments.as_bytes(), None).is_ok());
}
