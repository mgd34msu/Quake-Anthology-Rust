use qa_core::primitives::{RuleSetId, ThinkTime, Trajectory, TrajectoryCurve, Vec3};
use qa_gameplay::rules::entities::{
    ModelCellRole, ParameterRole, TrajectoryRules, TrajectorySample, entity_roles,
};

fn trajectory(curve: TrajectoryCurve) -> Trajectory {
    Trajectory {
        curve,
        start: ThinkTime::Milliseconds(1000),
        duration_ms: 2000,
        base: Vec3([10., 20., 30.]),
        delta: Vec3([4., -8., 12.]),
    }
}
fn sample(curve: TrajectoryCurve, time: i64, kind: TrajectorySample) -> Vec3 {
    TrajectoryRules::Q3_NATIVE
        .sample(&trajectory(curve), ThinkTime::Milliseconds(time), kind)
        .unwrap()
}

#[test]
fn source_entity_roles_distinguish_models_items_sounds_and_reference_domains() {
    let roles = entity_roles(RuleSetId::Quake3);
    assert_eq!(roles.cells(2, 0, 0).models, ModelCellRole::Item);
    assert_eq!(roles.cells(12, 0, 0).models, ModelCellRole::Team);
    assert_eq!(roles.cells(7, 0, 0).parameter, ParameterRole::Sound);
    assert!(!roles.cells(7, 0, 0).client_reference);
    assert!(!roles.cells(11, 0, 0).client_reference); // Portal roll scalar.
    for kind in [2, 8, 9] {
        assert!(!roles.client_solid(kind));
        assert!(entity_roles(RuleSetId::Quake2).client_solid(kind));
    }
    for kind in [0, 1, 3, 4, 10, 11, 12] {
        assert!(roles.client_solid(kind));
    }
    assert!(roles.cells(1, 0, 0).client_reference);
    for event in [45, 46] {
        assert_eq!(
            roles.cells(13 + event, 0, 0).parameter,
            ParameterRole::Sound
        );
        assert_eq!(
            roles.cells(0, event | 0x300, 0).parameter,
            ParameterRole::Sound
        );
    }
    assert_eq!(roles.cells(13 + 44, 0, 0).parameter, ParameterRole::Scalar);
    assert_eq!(roles.cells(13 + 48, 0, 0).parameter, ParameterRole::Entity);
    assert!(roles.cells(13 + 60, 0, 0).other2_reference);
    assert!(roles.cells(13 + 75, 0, 0x10).other_reference);
    assert_eq!(
        entity_roles(RuleSetId::Quake2).cells(7, 45, 0).parameter,
        ParameterRole::Scalar
    );
}

#[test]
fn native_trajectory_values_and_delta_keep_original_stop_and_half_cosine_rules() {
    use TrajectoryCurve::*;
    use TrajectorySample::*;
    for curve in [Stationary, Interpolate] {
        assert_eq!(sample(curve, 1500, Value), Vec3([10., 20., 30.]));
        assert_eq!(sample(curve, 1500, Delta), Vec3::default());
    }
    assert_eq!(sample(Linear, 1250, Value), Vec3([11., 18., 33.]));
    assert_eq!(sample(Linear, 1250, Delta), Vec3([4., -8., 12.]));
    assert_eq!(sample(LinearStop, 750, Value), Vec3([10., 20., 30.]));
    assert_eq!(sample(LinearStop, 750, Delta), Vec3([4., -8., 12.]));
    assert_eq!(sample(LinearStop, 3001, Value), Vec3([18., 4., 54.]));
    assert_eq!(sample(LinearStop, 3000, Delta), Vec3([4., -8., 12.]));
    assert_eq!(sample(LinearStop, 3001, Delta), Vec3::default());
    assert_eq!(sample(Sine, 1500, Value), Vec3([14., 12., 42.]));
    assert_eq!(sample(Sine, 1000, Delta), Vec3([2., -4., 6.]));
    assert_eq!(sample(Gravity, 1250, Value), Vec3([11., 18., 8.]));
    assert_eq!(sample(Gravity, 1250, Delta), Vec3([4., -8., -188.]));
}

#[test]
fn trajectory_clock_wrap_and_invalid_capabilities_are_explicit() {
    use TrajectorySample::Value;
    let mut tr = trajectory(TrajectoryCurve::Linear);
    tr.start = ThinkTime::Milliseconds(i64::from(i32::MAX - 99));
    assert_eq!(
        TrajectoryRules::Q3_NATIVE.sample(
            &tr,
            ThinkTime::Milliseconds(i64::from(i32::MIN + 150)),
            Value
        ),
        Some(Vec3([11., 18., 33.]))
    );
    assert_eq!(
        TrajectoryRules::Q3_NATIVE.sample(&tr, ThinkTime::Seconds(1.), Value),
        None
    );
    tr.curve = TrajectoryCurve::Unsupported(255);
    assert_eq!(
        TrajectoryRules::Q3_NATIVE.sample(&tr, tr.start, Value),
        None
    );
    tr.curve = TrajectoryCurve::Sine;
    tr.duration_ms = 0;
    assert_eq!(
        TrajectoryRules::Q3_NATIVE.sample(&tr, tr.start, Value),
        None
    );
}

#[test]
fn qvm_and_native_duration_products_use_their_selected_arithmetic() {
    let mut tr = trajectory(TrajectoryCurve::Linear);
    tr.start = ThinkTime::Milliseconds(0);
    tr.base = Vec3::default();
    tr.delta = Vec3([1., 1., 1.]);
    // 5ms is a binary32 product one ULP above native's double product
    // narrowed on assignment to deltaTime (original bg_misc.c).
    let native = TrajectoryRules::Q3_NATIVE
        .sample(&tr, ThinkTime::Milliseconds(5), TrajectorySample::Value)
        .unwrap();
    let qvm = TrajectoryRules::Q3_QVM
        .sample(&tr, ThinkTime::Milliseconds(5), TrajectorySample::Value)
        .unwrap();
    assert_eq!(native.0[0].to_bits(), 0x3ba3_d70a);
    assert_eq!(qvm.0[0].to_bits(), 0x3ba3_d70b);
}
