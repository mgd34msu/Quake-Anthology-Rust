use qa_core::primitives::{
    Body, MotionWrite, MovementMagnitude, MovementTimer, Plane, PlayerState, RuleSetId, UserCmd,
    Vec3,
};
use qa_gameplay::rules::sweep::{entity_sweep, player_sweep};
use qa_movement::{SweepExit, SweepHost, SweepResult, TraceServices, sweep};
use qa_world::collision::{Contents, EntityTracePolicy, Trace, TraceError, TraceQuery};

struct Host {
    words: [f32; 6],
    velocity: usize,
    live: bool,
    traces: [Option<Trace>; 4],
    ends: Vec<(Vec3, Vec3)>,
    writes: Vec<(MotionWrite, u32)>,
    contacts: Vec<(u32, u8, Vec3)>,
    contact_velocity: Option<(usize, Vec3)>,
    remove_contact: Option<usize>,
    remove_write: Option<MotionWrite>,
    fail_trace: Option<usize>,
    recovered: Option<Vec3>,
    recoveries: usize,
}
impl Host {
    fn new(velocity: Vec3, traces: &[Trace]) -> Self {
        let mut words = [0.0; 6];
        words[3..].copy_from_slice(&velocity.0);
        Self {
            words,
            velocity: 3,
            live: true,
            traces: std::array::from_fn(|i| traces.get(i).copied()),
            ends: Vec::new(),
            writes: Vec::new(),
            contacts: Vec::new(),
            contact_velocity: None,
            remove_contact: None,
            remove_write: None,
            fail_trace: None,
            recovered: None,
            recoveries: 0,
        }
    }
    fn body(&self) -> Body {
        Body {
            position: Vec3(self.words[..3].try_into().unwrap()),
            velocity: Vec3(
                self.words[self.velocity..self.velocity + 3]
                    .try_into()
                    .unwrap(),
            ),
            ..Default::default()
        }
    }
    fn velocity_writes(&self) -> Vec<(usize, u32)> {
        self.writes
            .iter()
            .filter_map(|&(field, bits)| match field {
                MotionWrite::Velocity(axis) => Some((axis, bits)),
                _ => None,
            })
            .collect()
    }
}
impl SweepHost for Host {
    type Error = &'static str;
    type Hit = u32;

    fn read(&mut self) -> Result<Option<Body>, Self::Error> {
        Ok(self.live.then(|| self.body()))
    }
    fn write(&mut self, field: MotionWrite, value: f32) -> Result<(), Self::Error> {
        let offset = match field {
            MotionWrite::Origin(axis) => axis,
            MotionWrite::Velocity(axis) => self.velocity + axis,
            _ => return Err("nonlinear write"),
        };
        self.words[offset] = value;
        self.writes.push((field, value.to_bits()));
        if self.remove_write == Some(field) {
            self.live = false;
        }
        Ok(())
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<(Trace, u32), Self::Error> {
        let index = self.ends.len();
        self.ends.push((start, end));
        if self.fail_trace == Some(index + 1) {
            return Err("trace");
        }
        Ok((
            self.traces[index].unwrap_or_else(|| Trace::clear(end)),
            100 + index as u32,
        ))
    }
    fn contact(&mut self, trace: &Trace, hit: u32, bits: u8) -> Result<(), Self::Error> {
        self.contacts.push((hit, bits, trace.plane.normal));
        if let Some((at, velocity)) = self.contact_velocity
            && at == self.contacts.len()
        {
            self.words[self.velocity..self.velocity + 3].copy_from_slice(&velocity.0);
        }
        if self.remove_contact == Some(self.contacts.len()) {
            self.live = false;
        }
        Ok(())
    }
    fn recover_position(&mut self, _: Vec3) -> Result<Option<Vec3>, Self::Error> {
        self.recoveries += 1;
        Ok(self.recovered)
    }
}

fn hit(fraction: f32, normal: Vec3, end: Vec3) -> Trace {
    Trace {
        fraction,
        plane: Plane::oriented(normal, 0.0),
        ..Trace::clear(end)
    }
}

#[test]
fn native_progress_rereads_velocity_after_all_aliased_origin_stores() {
    let mut host = Host::new(
        Vec3::default(),
        &[hit(0.5, Vec3([1.0, 0.0, 0.0]), Vec3([11.0, 5.0, 7.0]))],
    );
    // Raw origin at0 and velocity at1 overlap exactly as legal QC definitions.
    host.words = [10.0, 2.0, 3.0, 4.0, 0.0, 0.0];
    host.velocity = 1;
    let result = sweep(&mut host, 1.0, &entity_sweep(RuleSetId::Quake).unwrap()).unwrap();
    assert_eq!(
        result,
        SweepResult {
            blocked: 2,
            exit: SweepExit::Complete
        }
    );
    assert_eq!(
        &host.writes[..3],
        &[
            (MotionWrite::Origin(0), 11.0_f32.to_bits()),
            (MotionWrite::Origin(1), 5.0_f32.to_bits()),
            (MotionWrite::Origin(2), 7.0_f32.to_bits()),
        ]
    );
    assert_eq!(
        host.velocity_writes(),
        [(0, 0), (1, 7.0_f32.to_bits()), (2, 4.0_f32.to_bits())]
    );
    assert_eq!(
        host.ends[1],
        (Vec3([11.0, 0.0, 7.0]), Vec3([11.0, 3.5, 9.0]))
    );
    assert_eq!(host.body().position, Vec3([11.0, 3.5, 9.0]));
    assert_eq!(host.body().velocity, Vec3([3.5, 9.0, 4.0]));
}

#[test]
fn native_crease_uses_current_impact_velocity_while_qw_player_keeps_candidate_rule() {
    let traces = [
        hit(0.0, Vec3([1.0, 0.0, 0.0]), Vec3::default()),
        hit(0.0, Vec3([-0.8, 0.6, 0.0]), Vec3::default()),
    ];
    for (rules, expected_z) in [
        (entity_sweep(RuleSetId::Quake).unwrap(), 0x4066_6667),
        (entity_sweep(RuleSetId::QuakeWorld).unwrap(), 0x4066_6667),
        (player_sweep(RuleSetId::QuakeWorld).unwrap(), 0x3f38_51ec),
    ] {
        let mut host = Host::new(Vec3([-1.0, -1.0, 2.0]), &traces);
        host.contact_velocity = Some((2, Vec3([9.0, 9.0, 10.0])));
        let result = sweep(&mut host, 1.0, &rules).unwrap();
        assert_eq!(result.blocked, 2);
        assert_eq!(result.exit, SweepExit::Complete);
        assert_eq!(host.velocity_writes()[5].1, expected_z);
        assert_eq!(
            host.contacts
                .iter()
                .map(|contact| contact.0)
                .collect::<Vec<_>>(),
            [100, 101]
        );
    }
}

#[test]
fn native_qw_preserves_startsolid_and_zero_motion_distinctions_from_player() {
    let mut still = Host::new(Vec3::default(), &[]);
    assert_eq!(
        sweep(&mut still, 1.0, &entity_sweep(RuleSetId::Quake).unwrap())
            .unwrap()
            .exit,
        SweepExit::Still
    );
    assert!(still.ends.is_empty());
    let mut still = Host::new(Vec3::default(), &[]);
    assert_eq!(
        sweep(
            &mut still,
            1.0,
            &entity_sweep(RuleSetId::QuakeWorld).unwrap()
        )
        .unwrap()
        .exit,
        SweepExit::Complete
    );
    assert_eq!(still.ends.len(), 1);

    let trace = Trace {
        start_solid: true,
        ..Trace::clear(Vec3([4.0, 5.0, 6.0]))
    };
    let mut entity = Host::new(Vec3([1.0, 2.0, 3.0]), &[trace]);
    assert_eq!(
        sweep(
            &mut entity,
            1.0,
            &entity_sweep(RuleSetId::QuakeWorld).unwrap()
        )
        .unwrap()
        .exit,
        SweepExit::Complete
    );
    assert_eq!(entity.body().position, trace.end);
    let mut player = Host::new(Vec3([1.0, 2.0, 3.0]), &[trace]);
    let result = sweep(
        &mut player,
        1.0,
        &player_sweep(RuleSetId::QuakeWorld).unwrap(),
    )
    .unwrap();
    assert_eq!(
        result,
        SweepResult {
            blocked: 3,
            exit: SweepExit::AllSolid
        }
    );
    assert!(!result.exit.restores_timed_velocity());
    assert_eq!(player.body().position, Vec3::default());
    assert_eq!(player.body().velocity, Vec3::default());
}

#[test]
fn allsolid_stops_only_native_selected_components_and_rr_retains_contact() {
    let trace = Trace {
        all_solid: true,
        ..Trace::clear(Vec3([8.0; 3]))
    };
    for (rules, expected, contacts) in [
        (entity_sweep(RuleSetId::Quake).unwrap(), Vec3::default(), 0),
        (
            player_sweep(RuleSetId::Quake2).unwrap(),
            Vec3([1.0, 2.0, 0.0]),
            0,
        ),
        (
            player_sweep(RuleSetId::Quake2Rerelease).unwrap(),
            Vec3([1.0, 2.0, 0.0]),
            1,
        ),
    ] {
        let mut host = Host::new(Vec3([1.0, 2.0, 3.0]), &[trace]);
        let result = sweep(&mut host, 1.0, &rules).unwrap();
        assert_eq!(result.exit, SweepExit::AllSolid);
        assert_eq!(host.body().velocity, expected);
        assert_eq!(host.contacts.len(), contacts);
        assert_eq!(host.body().position, Vec3::default());
    }
}

#[test]
fn removal_after_contact_or_scalar_store_stops_without_writing_another_lifetime() {
    let trace = hit(0.5, Vec3([0.0, 0.0, 1.0]), Vec3([1.0, 2.0, 3.0]));
    let mut host = Host::new(Vec3([1.0, 2.0, -3.0]), &[trace]);
    host.remove_contact = Some(1);
    let result = sweep(&mut host, 1.0, &entity_sweep(RuleSetId::Quake).unwrap()).unwrap();
    assert_eq!(
        result,
        SweepResult {
            blocked: 1,
            exit: SweepExit::Removed
        }
    );
    assert_eq!(host.contacts, [(100, 1, trace.plane.normal)]);
    assert_eq!(host.writes.len(), 3);
    assert!(host.velocity_writes().is_empty());
    assert_eq!(host.ends.len(), 1);

    let mut host = Host::new(Vec3([1.0; 3]), &[]);
    host.remove_write = Some(MotionWrite::Origin(1));
    assert_eq!(
        sweep(&mut host, 1.0, &entity_sweep(RuleSetId::Quake).unwrap())
            .unwrap()
            .exit,
        SweepExit::Removed
    );
    assert_eq!(host.writes.len(), 2);
    assert_eq!(host.writes[1].0, MotionWrite::Origin(1));
}

#[test]
fn native_negative_dot_rejection_keeps_player_nan_acceptance_distinct() {
    let traces = [
        hit(0.0, Vec3([f32::NAN, 0.0, 0.0]), Vec3::default()),
        hit(0.0, Vec3([0.0, 1.0, 0.0]), Vec3::default()),
        hit(0.0, Vec3([0.0, 0.0, 1.0]), Vec3::default()),
    ];
    let mut entity = Host::new(Vec3([1.0; 3]), &traces);
    assert_eq!(
        sweep(&mut entity, 1.0, &entity_sweep(RuleSetId::Quake).unwrap())
            .unwrap()
            .exit,
        SweepExit::Complete
    );
    assert!(entity.body().velocity.0.into_iter().all(f32::is_nan));
    let mut player = Host::new(Vec3([1.0; 3]), &traces);
    let result = sweep(&mut player, 1.0, &player_sweep(RuleSetId::Quake).unwrap()).unwrap();
    assert_eq!(
        result,
        SweepResult {
            blocked: 7,
            exit: SweepExit::NoCrease
        }
    );
    assert_eq!(player.body().velocity, Vec3::default());
    assert!(!result.exit.restores_timed_velocity());
}

#[test]
fn rr_duplicate_plane_uses_one_recovery_boundary_and_retains_local_hit_metadata() {
    let traces = [
        hit(0.0, Vec3([1.0, 0.0, 0.0]), Vec3::default()),
        hit(0.0, Vec3([1.0, 0.0, 0.0]), Vec3::default()),
    ];
    let mut host = Host::new(Vec3([-10.0, 10.0, 0.0]), &traces);
    host.recovered = Some(Vec3([4.0, 5.0, 6.0]));
    let result = sweep(
        &mut host,
        1.0,
        &player_sweep(RuleSetId::Quake2Rerelease).unwrap(),
    )
    .unwrap();
    assert_eq!(result.exit, SweepExit::Complete);
    assert_eq!(host.recoveries, 1);
    assert_eq!(host.velocity_writes().len(), 3);
    assert_eq!(host.ends[2].0, Vec3([4.0, 5.0, 6.0]));
    assert_eq!(
        host.contacts
            .iter()
            .map(|contact| contact.0)
            .collect::<Vec<_>>(),
        [100, 101]
    );
}

#[test]
fn trace_failure_keeps_already_executed_scalar_stores_for_native_failure_handling() {
    let trace = hit(0.5, Vec3([1.0, 0.0, 0.0]), Vec3([1.0, 2.0, 3.0]));
    let mut host = Host::new(Vec3([-1.0, 2.0, 3.0]), &[trace]);
    host.fail_trace = Some(2);
    assert_eq!(
        sweep(&mut host, 1.0, &entity_sweep(RuleSetId::Quake).unwrap()),
        Err("trace")
    );
    assert_eq!(host.body().position, trace.end);
    assert_eq!(host.body().velocity, Vec3([0.0, 2.0, 3.0]));
    assert_eq!(host.writes.len(), 6);
}

#[test]
fn player_water_jump_restores_timed_velocity_only_after_regular_sweep_exit() {
    struct Wall {
        all_solid: bool,
        moves: usize,
    }
    impl TraceServices for Wall {
        fn trace(&mut self, query: TraceQuery) -> Result<Trace, TraceError> {
            if query.start.0[0] != query.end.0[0] {
                self.moves += 1;
                if self.moves == 1 {
                    return Ok(Trace {
                        all_solid: self.all_solid,
                        ..hit(
                            0.5,
                            Vec3([-1.0, 0.0, 0.0]),
                            query.start.lerp(query.end, 0.5),
                        )
                    });
                }
            }
            Ok(Trace::clear(query.end))
        }
        fn point_contents(&self, _: Vec3, _: EntityTracePolicy) -> Result<Contents, TraceError> {
            Ok(Contents::EMPTY)
        }
    }
    for (all_solid, expected) in [
        (false, Vec3([100.0, 0.0, 100.0])),
        (true, Vec3([100.0, 0.0, 0.0])),
    ] {
        let mut player = PlayerState {
            movement_rules: RuleSetId::Quake2,
            trace_rules: RuleSetId::Quake,
            health: 100,
            ..Default::default()
        };
        player.body.position = Vec3([0.0, 0.0, 64.0]);
        player.body.velocity = Vec3([100.0, 0.0, 100.0]);
        player.movement.timer = MovementTimer::WATER_JUMP;
        player.movement.remaining_ms = 100;
        player.movement.tuning.gravity = Some(MovementMagnitude::Base(0.0));
        player.movement.tuning.no_step = true;
        let mut world = Wall {
            all_solid,
            moves: 0,
        };
        qa_movement::pmove(
            UserCmd {
                duration_ms: 8,
                server_time_ms: 8,
                ..Default::default()
            },
            &mut player,
            &mut world,
            Default::default(),
            qa_movement::no_boundary,
        )
        .unwrap();
        assert_eq!(
            player.body.velocity.0.map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
        assert_eq!(player.movement.remaining_ms, 92);
        assert!(world.moves > 0);
    }
}
