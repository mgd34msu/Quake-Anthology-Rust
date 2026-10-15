use qa_app::{client_scene::ClientEntities, snapshots::Interpolation};
use qa_core::primitives::*;
use qa_gameplay::rules::appearance::{AliasRoll, policy};
use qa_network::{
    commands::packet::Protocol,
    projection::EntityProjection,
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};

const CLIENT: ClientId = ClientId(3);

fn row(number: u32, kind: u32, trajectory: Trajectory) -> Entity<{ states::ENTITY_WORDS }> {
    let mut published = EntityPresentation::new(ResourceNamespace::Client(CLIENT));
    published.trajectories.position = trajectory;
    published.trajectories.angles = Trajectory {
        base: Vec3([0., 350., 0.]),
        ..Default::default()
    };
    let record = EntityRecord {
        appearance: EntityAppearance {
            source: RuleSetId::Quake3,
            presentation: Some(published),
            native: NativeAppearanceTail {
                entity_type: kind,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    encode(number, record)
}
fn encode(number: u32, record: EntityRecord) -> Entity<{ states::ENTITY_WORDS }> {
    let mut words = [0; states::ENTITY_WORDS];
    assert!(EntityProjection::load(Protocol::Quake3_68).reduce(
        &record,
        &qa_network::projection::EntityContext::default(),
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal()),
    ));
    Entity { number, words }
}
fn frame<'a>(
    sequence: u32,
    time: i64,
    rows: &'a [Entity<{ states::ENTITY_WORDS }>],
) -> ReceivedFrame<'a> {
    ReceivedFrame::Quake3(Frame {
        sequence,
        time: ThinkTime::Milliseconds(time),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player: &[0; states::PLAYER_WORDS],
        entities: rows,
    })
}
fn entities<'a>(
    projection: &'a EntityProjection,
    current: ReceivedFrame<'a>,
    previous: Option<ReceivedFrame<'a>>,
    time: i64,
    fraction: f32,
    smooth: bool,
) -> ClientEntities<'a> {
    ClientEntities {
        client: CLIENT,
        player: None,
        current: Some(current),
        previous,
        projection,
        appearance: policy(RuleSetId::Quake3).with_smooth_clients(smooth),
        alias_roll: AliasRoll::default(),
        interpolation: Interpolation {
            time: ThinkTime::Milliseconds(time),
            fraction,
            frame_ms: 100,
        },
    }
}
fn trajectory(curve: TrajectoryCurve) -> Trajectory {
    Trajectory {
        curve,
        start: ThinkTime::Milliseconds(1000),
        duration_ms: 1000,
        base: Vec3([10., 20., 30.]),
        delta: Vec3([4., -8., 12.]),
    }
}

#[test]
fn render_and_sound_origins_evaluate_accepted_trajectories_at_the_native_clock() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    for (curve, expected) in [
        (TrajectoryCurve::Stationary, Vec3([10., 20., 30.])),
        (TrajectoryCurve::Interpolate, Vec3([10., 20., 30.])),
        (TrajectoryCurve::Linear, Vec3([11., 18., 33.])),
        (TrajectoryCurve::LinearStop, Vec3([11., 18., 33.])),
        (TrajectoryCurve::Sine, Vec3([14., 12., 42.])),
        (TrajectoryCurve::Gravity, Vec3([11., 18., 8.])),
    ] {
        let rows = [row(90, 3, trajectory(curve))];
        let view = entities(&projection, frame(2, 1300, &rows), None, 1250, 0.25, true);
        let pose = view.get(90).unwrap();
        assert_eq!(pose.origin, expected, "{curve:?}");
        assert_eq!(
            view.sound_origin(90, Some(LoopOrigin::Presented)),
            Some(expected)
        );
        assert_eq!(
            view.sound_origin(90, Some(LoopOrigin::Accepted)),
            Some(Vec3([10., 20., 30.]))
        );
        // Presentation borrows the sole accepted row; it does not rewrite its base.
        assert_eq!(
            pose.record
                .appearance
                .presentation
                .unwrap()
                .trajectories
                .position
                .base,
            Vec3([10., 20., 30.])
        );
    }
}

#[test]
fn interpolation_samples_each_snapshot_clock_and_keeps_shortest_angle_paths() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let old = [row(12, 1, trajectory(TrajectoryCurve::LinearStop))];
    let mut current = row(
        12,
        1,
        Trajectory {
            base: Vec3([40., 50., 60.]),
            ..trajectory(TrajectoryCurve::LinearStop)
        },
    );
    let mut record = EntityRecord::default();
    assert!(projection.apply(
        &current.words,
        &mut record,
        ResourceNamespace::Client(CLIENT)
    ));
    record
        .appearance
        .presentation
        .as_mut()
        .unwrap()
        .trajectories
        .angles
        .base = Vec3([0., 10., 0.]);
    current = encode(12, record);
    let rows = [current];
    let view = entities(
        &projection,
        frame(2, 1500, &rows),
        Some(frame(1, 1000, &old)),
        1250,
        0.5,
        true,
    );
    let pose = view.get(12).unwrap();
    assert_eq!(pose.origin, Vec3([26., 33., 48.])); // [10,20,30] -> [42,46,66].
    assert_eq!(pose.angles, Vec3([0., 360., 0.]));
    // Disabling smooth clients forces position interpolation to raw trBase;
    // it does not force angular trajectories or change native wire words.
    let forced = ClientEntities {
        appearance: view.appearance.with_smooth_clients(false),
        ..view
    };
    assert_eq!(forced.get(12).unwrap().origin, Vec3([25., 35., 45.]));
    let unsmoothed = ClientEntities {
        previous: None,
        ..forced
    };
    assert_eq!(unsmoothed.get(12).unwrap().origin, Vec3([40., 50., 60.]));
}

#[test]
fn accepted_mover_translation_never_reads_a_server_body_or_rotates_the_rider() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mover = row(
        80,
        4,
        Trajectory {
            base: Vec3([100., 0., 0.]),
            delta: Vec3([20., 0., 0.]),
            ..trajectory(TrajectoryCurve::Linear)
        },
    );
    let mut rider = row(90, 0, trajectory(TrajectoryCurve::Stationary));
    let mut record = EntityRecord::default();
    assert!(projection.apply(&rider.words, &mut record, ResourceNamespace::Client(CLIENT)));
    record.appearance.presentation.as_mut().unwrap().ground = Some(NativeEntityRef::entity(
        ResourceNamespace::Client(CLIENT),
        80,
    ));
    rider = encode(90, record);
    let rows = [mover, rider];
    let view = entities(&projection, frame(2, 1000, &rows), None, 1250, 1., true);
    assert_eq!(view.get(90).unwrap().origin, Vec3([15., 20., 30.]));
    assert_eq!(view.get(90).unwrap().angles, Vec3([0., 350., 0.]));
    let missing = [rider];
    let view = entities(&projection, frame(2, 1000, &missing), None, 1250, 1., true);
    assert_eq!(view.get(90).unwrap().origin, Vec3([10., 20., 30.]));
}

#[test]
fn malformed_trajectory_drops_only_that_pose_and_keeps_a_healthy_peer() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let rows = [
        row(90, 0, trajectory(TrajectoryCurve::Unsupported(255))),
        row(91, 0, trajectory(TrajectoryCurve::Linear)),
    ];
    let view = entities(&projection, frame(2, 1300, &rows), None, 1250, 0.25, true);
    assert!(view.get(90).is_none());
    assert_eq!(view.get(91).unwrap().origin, Vec3([11., 18., 33.]));
    let mut seen = Vec::new();
    view.visit(|number, pose| seen.push((number, pose.is_some())));
    assert_eq!(seen, [(90, false), (91, true)]);
}
