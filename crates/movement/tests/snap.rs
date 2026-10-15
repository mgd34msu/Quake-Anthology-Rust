use qa_core::primitives::{MovementMagnitude, PlayerState, RuleSetId, UserCmd, Vec3};
use qa_movement::{TraceServices, no_boundary, pmove, set_bounds};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

struct JitterWorld {
    clear: Vec3,
    candidates: Vec<Vec3>,
}

impl TraceServices for JitterWorld {
    fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
        let mut trace = Trace::clear(query.end);
        if query.start == query.end {
            self.candidates.push(query.start);
            trace.all_solid = query.start != self.clear;
            trace.start_solid = trace.all_solid;
        }
        Ok(trace)
    }

    fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
        Ok(Contents::EMPTY)
    }
}

fn move_to_jitter(
    rules: RuleSetId,
    origin: Vec3,
    velocity: Vec3,
    clear: Vec3,
) -> (PlayerState, JitterWorld) {
    let mut player = PlayerState {
        movement_rules: rules,
        trace_rules: RuleSetId::Quake3,
        health: 100,
        ..Default::default()
    };
    set_bounds(&mut player);
    player.body.position = origin;
    player.body.velocity = velocity;
    player.movement.tuning.gravity = Some(MovementMagnitude::Base(0.0));
    player.movement.tuning.no_step = true;
    let mut world = JitterWorld {
        clear,
        candidates: Vec::new(),
    };
    pmove(
        UserCmd {
            duration_ms: 8,
            server_time_ms: 8,
            ..Default::default()
        },
        &mut player,
        &mut world,
        Default::default(),
        no_boundary,
    )
    .expect("valid movement queries");
    (player, world)
}

#[test]
fn classic_snap_does_not_jitter_an_already_aligned_axis() {
    let origin = Vec3([0.0, 0.0, 64.0]);
    let (player, world) = move_to_jitter(
        RuleSetId::Quake2,
        origin,
        Vec3([1.0, 1.0, 0.0]),
        Vec3([0.0, 0.0, 64.125]),
    );
    // All legal snap candidates are blocked. Z was already an exact eighth,
    // so the otherwise clear point above it must not defeat native fallback.
    assert_eq!(player.body.position, origin);
    assert_eq!(
        world.candidates,
        [
            origin,
            origin,
            Vec3([0.125, 0.0, 64.0]),
            Vec3([0.0, 0.125, 64.0]),
            Vec3([0.125, 0.125, 64.0]),
            Vec3([0.125, 0.0, 64.0]),
            Vec3([0.0, 0.125, 64.0]),
            Vec3([0.125, 0.125, 64.0]),
        ]
    );
}

#[test]
fn classic_snap_jitter_wraps_the_native_signed_short_coordinate() {
    for (start, velocity, accepted) in [(4095.875, 4.0, -4096.0), (-4096.0, -4.0, 4095.875)] {
        let origin = Vec3([start, 0.0, 64.0]);
        let clear = Vec3([accepted, 0.0, 64.0]);
        let (player, world) =
            move_to_jitter(RuleSetId::Quake2, origin, Vec3([velocity, 0.0, 0.0]), clear);
        assert_eq!(player.body.position, clear);
        assert_eq!(world.candidates, [origin, origin, clear]);
    }
}

#[test]
fn rerelease_keeps_float_motion_instead_of_classic_snap_jitter() {
    let origin = Vec3([4095.875, 0.0, 64.0]);
    let velocity = Vec3([4.0, 0.0, 0.0]);
    let expected = origin + velocity * (8.0f32 * 0.001);
    let (player, world) = move_to_jitter(RuleSetId::Quake2Rerelease, origin, velocity, expected);
    assert_eq!(player.body.position, expected);
    assert_eq!(player.body.velocity, velocity);
    assert!(
        world
            .candidates
            .iter()
            .all(|&candidate| candidate == expected)
    );
}
