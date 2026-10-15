use qa_core::primitives::{
    Bounds, EntityPose, GeometryId, IndexSpan, ModuleId, NativeEntityRef, Plane, ResourceNamespace,
    RuleSetId, SurfaceFlags, Vec3,
};
use qa_gameplay::rules::motion::{StepRules, step_rules};
use qa_session::physics::{
    GoalCursor, GoalHost, GoalStep, MonsterHost, MotionTrace, PhysicsError, StepMove, StepPost,
    finish_step, move_step,
};
use qa_world::collision::{
    CollisionStore, Contents, EntityTracePolicy, QuakeTraceKind, TraceQuery, TraceRules,
    TraceScratch,
    brushes::{Brush, BrushTree},
    surfaces::SurfaceTable,
    trace_policy,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Write {
    Origin(usize, f32),
    Flags(u32),
    Ground(NativeEntityRef),
}
struct Host {
    pose: EntityPose,
    bounds: Bounds,
    flags: u32,
    enemy: Option<NativeEntityRef>,
    target: Option<Vec3>,
    ground: Option<NativeEntityRef>,
    geometry: CollisionStore,
    resource: GeometryId,
    scratch: TraceScratch,
    trace_rules: TraceRules,
    entity_rules: EntityTracePolicy,
    writes: Vec<Write>,
    traces: Vec<(Vec3, Vec3, Bounds, QuakeTraceKind)>,
    contents: Vec<Vec3>,
    enemy_reads: usize,
    target_reads: usize,
    bounds_reads: usize,
    on_origin: Option<fn(&mut Host, usize)>,
    fail_axis: Option<usize>,
    goal: NativeEntityRef,
    goal_position: Vec3,
    goal_bounds: Bounds,
    ideal_yaw: f32,
    yaw: f32,
    yaw_speed: f32,
    turns: Vec<f32>,
    random: qa_core::random::Random,
    draws: Vec<u32>,
    absolute_reads: Vec<(NativeEntityRef, usize)>,
}
fn reference(ordinal: u32) -> NativeEntityRef {
    NativeEntityRef::entity(ResourceNamespace::Module(ModuleId(7)), ordinal)
}
fn floor(max_x: f32) -> Bounds {
    Bounds {
        mins: Vec3([-2048.0, -2048.0, -64.0]),
        maxs: Vec3([max_x, 2048.0, 0.0]),
    }
}
impl Host {
    fn new(boxes: &[Bounds]) -> Self {
        Self::with_contents(boxes, Contents::SOLID)
    }
    fn with_contents(boxes: &[Bounds], contents: Contents) -> Self {
        let mut planes = Vec::new();
        let mut brushes = Vec::new();
        for bounds in boxes {
            let first = planes.len() as u32;
            for axis in 0..3 {
                let mut normal = Vec3::default();
                normal.0[axis] = 1.0;
                planes.push(Plane::oriented(normal, bounds.maxs.0[axis]));
                normal.0[axis] = -1.0;
                planes.push(Plane::oriented(normal, -bounds.mins.0[axis]));
            }
            brushes.push(Brush {
                sides: IndexSpan { first, count: 6 },
                contents,
            });
        }
        let mut geometry = CollisionStore::new();
        let resource = geometry
            .load_brushes(
                planes,
                brushes,
                SurfaceTable::flags(vec![SurfaceFlags::default(); boxes.len() * 6]),
                BrushTree::direct(boxes.len()).unwrap(),
                vec![Bounds {
                    mins: Vec3([-4096.0; 3]),
                    maxs: Vec3([4096.0; 3]),
                }],
            )
            .unwrap();
        let scratch = geometry.scratch();
        let (trace_rules, entity_rules) = trace_policy(RuleSetId::Quake);
        Self {
            pose: EntityPose::default(),
            bounds: Bounds {
                mins: Vec3([-1.0, -1.0, 0.0]),
                maxs: Vec3([1.0; 3]),
            },
            flags: 0,
            enemy: None,
            target: None,
            ground: Some(reference(0)),
            geometry,
            resource,
            scratch,
            trace_rules,
            entity_rules,
            writes: Vec::new(),
            traces: Vec::new(),
            contents: Vec::new(),
            enemy_reads: 0,
            target_reads: 0,
            bounds_reads: 0,
            on_origin: None,
            fail_axis: None,
            goal: reference(0),
            goal_position: Vec3([100.0, 100.0, 0.0]),
            goal_bounds: Bounds::default(),
            ideal_yaw: 0.0,
            yaw: 0.0,
            yaw_speed: 360.0,
            turns: Vec::new(),
            random: qa_core::random::Random::with_algorithm(
                1,
                qa_core::random::Algorithm::GlibcType3,
            ),
            draws: Vec::new(),
            absolute_reads: Vec::new(),
        }
    }
}
impl MonsterHost for Host {
    type Error = PhysicsError;
    fn origin(&mut self) -> Result<Vec3, PhysicsError> {
        Ok(self.pose.position)
    }
    fn bounds(&mut self) -> Result<Bounds, PhysicsError> {
        self.bounds_reads += 1;
        Ok(self.bounds)
    }
    fn flags(&mut self) -> Result<u32, PhysicsError> {
        Ok(self.flags)
    }
    fn enemy(&mut self) -> Result<Option<NativeEntityRef>, PhysicsError> {
        self.enemy_reads += 1;
        Ok(self.enemy)
    }
    fn target_origin(&mut self, target: NativeEntityRef) -> Result<Vec3, PhysicsError> {
        self.target_reads += 1;
        if target == self.goal {
            return Ok(self.goal_position);
        }
        assert_eq!(Some(target), self.enemy);
        self.target.ok_or(PhysicsError::MissingField)
    }
    fn contents(&mut self, point: Vec3) -> Result<i32, PhysicsError> {
        self.contents.push(point);
        self.geometry
            .point_contents_model(self.resource, 0, point, self.entity_rules)
            .map(|value| value.to_q1())
            .map_err(|_| PhysicsError::Entity)
    }
    fn trace(
        &mut self,
        start: Vec3,
        end: Vec3,
        bounds: Bounds,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, PhysicsError> {
        self.traces.push((start, end, bounds, kind));
        let mut query = TraceQuery::point(
            start,
            end,
            self.trace_rules,
            self.entity_rules.with_quake_kind(Some(kind)),
        );
        query.mins = bounds.mins;
        query.maxs = bounds.maxs;
        let trace = self
            .geometry
            .trace_model(self.resource, 0, query, &mut self.scratch)
            .map_err(|_| PhysicsError::Entity)?;
        Ok(MotionTrace {
            ground: if trace.fraction < 1.0 || trace.start_solid || trace.all_solid {
                self.ground
            } else {
                None
            },
            trace,
        })
    }
    fn write_origin(&mut self, axis: usize, value: f32) -> Result<(), PhysicsError> {
        self.pose.position.0[axis] = value;
        self.writes.push(Write::Origin(axis, value));
        if let Some(alias) = self.on_origin {
            alias(self, axis);
        }
        if self.fail_axis == Some(axis) {
            Err(PhysicsError::Entity)
        } else {
            Ok(())
        }
    }
    fn write_flags(&mut self, flags: u32) -> Result<(), PhysicsError> {
        self.flags = flags;
        self.writes.push(Write::Flags(flags));
        Ok(())
    }
    fn write_ground(&mut self, ground: NativeEntityRef) -> Result<(), PhysicsError> {
        self.writes.push(Write::Ground(ground));
        Ok(())
    }
}
impl GoalHost for Host {
    fn ideal_yaw(&mut self) -> Result<f32, PhysicsError> {
        Ok(self.ideal_yaw)
    }
    fn yaw(&mut self) -> Result<f32, PhysicsError> {
        Ok(self.yaw)
    }
    fn write_ideal_yaw(&mut self, value: f32) -> Result<(), PhysicsError> {
        self.ideal_yaw = value;
        Ok(())
    }
    fn change_yaw(&mut self) -> Result<(), PhysicsError> {
        self.turns.push(self.ideal_yaw);
        if let Some(yaw) = qa_movement::change_yaw(self.yaw, self.ideal_yaw, self.yaw_speed) {
            self.yaw = yaw;
        }
        Ok(())
    }
    fn enemy_present(&mut self) -> Result<bool, PhysicsError> {
        Ok(self.enemy.is_some())
    }
    fn absolute_min(&mut self, target: NativeEntityRef, axis: usize) -> Result<f32, PhysicsError> {
        self.absolute_reads.push((target, axis));
        Ok(if target == self.goal {
            self.goal_bounds.mins.0[axis]
        } else {
            self.pose.position.0[axis] + self.bounds.mins.0[axis]
        })
    }
    fn absolute_max(&mut self, target: NativeEntityRef, axis: usize) -> Result<f32, PhysicsError> {
        self.absolute_reads.push((target, axis));
        Ok(if target == self.goal {
            self.goal_bounds.maxs.0[axis]
        } else {
            self.pose.position.0[axis] + self.bounds.maxs.0[axis]
        })
    }
    fn random_word(&mut self) -> u32 {
        let word = self.random.next_u32();
        self.draws.push(word);
        word
    }
}
fn rules(source: RuleSetId) -> StepRules {
    step_rules(source).unwrap()
}

#[test]
fn supported_step_clears_partial_before_ground_and_never_invents_onground() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let rules = rules(source);
        let mut host = Host::new(&[floor(2048.0)]);
        host.flags = rules.partial_ground_flag;
        assert_eq!(
            move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
            Ok(StepMove::Moved(StepPost::None))
        );
        assert_eq!(host.pose.position, Vec3([10.0, 0.0, 0.03125]));
        assert_eq!(host.flags, 0);
        assert_eq!(
            host.writes,
            [
                Write::Origin(0, 10.0),
                Write::Origin(1, 0.0),
                Write::Origin(2, 0.03125),
                Write::Flags(0),
                Write::Ground(reference(0)),
            ]
        );
        assert_eq!(host.enemy_reads, 0);
        assert_eq!(host.target_reads, 0);
    }
}

#[test]
fn allsolid_blocks_without_writes_and_startsolid_retries_from_one_step_lower() {
    let rules = rules(RuleSetId::Quake);
    let mut blocked = Host::new(&[Bounds {
        mins: Vec3([-100.0; 3]),
        maxs: Vec3([100.0; 3]),
    }]);
    assert_eq!(
        move_step(&mut blocked, Vec3([10.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Blocked)
    );
    assert!(blocked.writes.is_empty());
    let mut retry = Host::new(&[
        floor(2048.0),
        Bounds {
            mins: Vec3([-100.0, -100.0, 20.0]),
            maxs: Vec3([100.0, 100.0, 28.0]),
        },
    ]);
    retry.pose.position.0[2] = 4.0;
    assert_eq!(
        move_step(&mut retry, Vec3([10.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Moved(StepPost::None))
    );
    assert_eq!(retry.traces[0].0, Vec3([10.0, 0.0, 22.0]));
    assert_eq!(retry.traces[1].0, Vec3([10.0, 0.0, 4.0]));
    assert_eq!(retry.traces[0].1, retry.traces[1].1);
    assert_eq!(retry.bounds_reads, 3); // first trace, retry, then bottom check
}

#[test]
fn no_floor_is_blocked_or_defers_only_onground_clear_until_after_touch() {
    let rules = rules(RuleSetId::Quake);
    let mut host = Host::new(&[]);
    host.flags = rules.ground_flag;
    assert_eq!(
        move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Blocked)
    );
    assert!(host.writes.is_empty());
    host.flags |= rules.partial_ground_flag;
    let result = move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules).unwrap();
    assert_eq!(result, StepMove::Moved(StepPost::ClearGround));
    assert_eq!(host.pose.position, Vec3([10.0, 0.0, 0.0]));
    assert_eq!(host.flags, rules.ground_flag | rules.partial_ground_flag);
    // Stand in for the caller's synchronous touch: its new flag must survive.
    host.flags |= 0x8000;
    finish_step(&mut host, StepPost::ClearGround, &rules).unwrap();
    assert_eq!(host.flags, rules.partial_ground_flag | 0x8000);
    assert_eq!(host.writes.last(), Some(&Write::Flags(host.flags)));
    assert!(
        !host
            .writes
            .iter()
            .any(|write| matches!(write, Write::Ground(_)))
    );
}

#[test]
fn unsupported_corner_restores_origin_unless_current_partial_ground_allows_it() {
    let rules = rules(RuleSetId::Quake);
    for partial in [false, true] {
        let mut host = Host::new(&[floor(0.0)]);
        host.pose.position = Vec3([-4.0, 0.0, 0.0]);
        host.flags = rules.ground_flag
            | if partial {
                rules.partial_ground_flag
            } else {
                0
            };
        let result = move_step(&mut host, Vec3([4.0, 0.0, 0.0]), &rules).unwrap();
        assert_eq!(
            result,
            if partial {
                StepMove::Moved(StepPost::None)
            } else {
                StepMove::Blocked
            }
        );
        assert_eq!(host.pose.position.0[0], if partial { 0.0 } else { -4.0 });
        assert_eq!(host.writes.len(), if partial { 3 } else { 6 });
        assert!(
            host.traces
                .iter()
                .any(|query| query.3 == QuakeTraceKind::IgnoreBoxes)
        );
    }
}

#[test]
fn flight_retries_without_height_adjustment_and_does_not_reread_unused_target_origin() {
    let rules = rules(RuleSetId::Quake);
    let mut host = Host::new(&[Bounds {
        mins: Vec3([4.0, -10.0, -6.0]),
        maxs: Vec3([6.0, 10.0, -2.0]),
    }]);
    host.flags = rules.fly_flag;
    host.enemy = Some(reference(12));
    host.target = Some(Vec3([0.0, 0.0, -100.0]));
    assert_eq!(
        move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Moved(StepPost::None))
    );
    assert_eq!(host.traces.len(), 2);
    assert_eq!(host.traces[0].1, Vec3([10.0, 0.0, -8.0]));
    assert_eq!(host.traces[1].1, Vec3([10.0, 0.0, 0.0]));
    assert_eq!((host.enemy_reads, host.target_reads), (2, 1));
    assert_eq!(host.pose.position, Vec3([10.0, 0.0, 0.0]));
    assert!(host.contents.is_empty());
}

#[test]
fn swimmers_reject_empty_without_origin_writes_and_flyers_skip_contents() {
    let rules = rules(RuleSetId::Quake);
    for flags in [rules.swim_flag, rules.fly_flag] {
        let mut host = Host::new(&[]);
        host.flags = flags;
        assert_eq!(
            move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
            Ok(if flags == rules.swim_flag {
                StepMove::Blocked
            } else {
                StepMove::Moved(StepPost::None)
            })
        );
        assert_eq!(host.traces.len(), 1);
        assert_eq!(host.target_reads, 0);
        if flags == rules.swim_flag {
            assert_eq!(host.contents, [Vec3([10.0, 0.0, 0.0])]);
            assert!(host.writes.is_empty());
        } else {
            assert!(host.contents.is_empty());
        }
    }
    let mut water = Host::with_contents(
        &[Bounds {
            mins: Vec3([-100.0; 3]),
            maxs: Vec3([100.0; 3]),
        }],
        Contents::WATER,
    );
    water.flags = rules.swim_flag;
    assert_eq!(
        move_step(&mut water, Vec3([10.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Moved(StepPost::None))
    );
    assert_eq!(water.pose.position, Vec3([10.0, 0.0, 0.0]));
    assert_eq!(water.contents, [Vec3([10.0, 0.0, 0.0])]);
}

#[test]
fn flight_height_thresholds_are_strict_and_a_world_enemy_never_retries() {
    let rules = rules(RuleSetId::Quake);
    for (height, adjustment) in [(29.0f32, 8.0f32), (30.0, 0.0), (40.0, 0.0), (41.0, -8.0)] {
        let mut host = Host::new(&[]);
        host.flags = rules.fly_flag;
        host.enemy = Some(reference(12));
        host.target = Some(Vec3([0.0, 0.0, -height]));
        assert_eq!(
            move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
            Ok(StepMove::Moved(StepPost::None))
        );
        assert_eq!(host.pose.position, Vec3([10.0, 0.0, adjustment]));
    }
    let mut host = Host::new(&[Bounds {
        mins: Vec3([4.0, -10.0, -10.0]),
        maxs: Vec3([6.0, 10.0, 10.0]),
    }]);
    host.flags = rules.fly_flag;
    assert_eq!(
        move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Blocked)
    );
    assert_eq!((host.traces.len(), host.target_reads), (1, 0));
    assert!(host.writes.is_empty());
}

#[test]
fn scalar_alias_effects_change_later_support_bounds_and_partial_ground_decision() {
    let rules = rules(RuleSetId::Quake);
    let mut host = Host::new(&[floor(0.0)]);
    host.pose.position = Vec3([-4.0, 0.0, 0.0]);
    host.bounds.mins.0[0] = 0.0;
    host.bounds.maxs.0[0] = 0.0;
    host.flags = rules.ground_flag;
    host.on_origin = Some(|host, axis| {
        if axis == 0 {
            host.flags = step_rules(RuleSetId::Quake).unwrap().partial_ground_flag;
            host.bounds.mins.0[0] = -2.0;
            host.bounds.maxs.0[0] = 2.0;
        }
    });
    assert_eq!(
        move_step(&mut host, Vec3([4.0, 0.0, 0.0]), &rules),
        Ok(StepMove::Moved(StepPost::None))
    );
    assert_eq!(host.pose.position.0[0], 0.0);
    assert_eq!(host.flags, rules.partial_ground_flag);
    let bottom = host
        .traces
        .iter()
        .find(|query| query.3 == QuakeTraceKind::IgnoreBoxes)
        .unwrap();
    assert_eq!(bottom.2, Bounds::default());
    assert!(host.contents.iter().any(|point| point.0[0] == 2.0));
}

#[test]
fn scalar_failure_keeps_admitted_prefix_and_missing_ground_does_not_invent_world_zero() {
    let rules = rules(RuleSetId::Quake);
    let mut host = Host::new(&[floor(2048.0)]);
    host.fail_axis = Some(1);
    assert_eq!(
        move_step(&mut host, Vec3([10.0, 5.0, 0.0]), &rules),
        Err(PhysicsError::Entity)
    );
    assert_eq!(host.pose.position, Vec3([10.0, 5.0, 0.0]));
    assert_eq!(host.writes, [Write::Origin(0, 10.0), Write::Origin(1, 5.0)]);
    let mut host = Host::new(&[floor(2048.0)]);
    host.flags = rules.partial_ground_flag;
    host.ground = None;
    assert_eq!(
        move_step(&mut host, Vec3([10.0, 0.0, 0.0]), &rules),
        Err(PhysicsError::Entity)
    );
    assert_eq!(host.flags, 0); // native partial flag clear precedes ground store
    assert!(
        !host
            .writes
            .iter()
            .any(|write| matches!(write, Write::Ground(_)))
    );
}

fn linked(step: GoalStep) -> GoalCursor {
    match step {
        GoalStep::Link(cursor) => cursor,
        other => panic!("expected native relink boundary, got {other:?}"),
    }
}
fn begin_goal(host: &mut Host, distance: f32) -> Result<GoalStep, PhysicsError> {
    let goal = host.goal;
    GoalCursor::begin(host, reference(1), goal, distance, rules(RuleSetId::Quake))
}

#[test]
fn goal_mobility_rejection_and_nan_close_enough_do_not_draw_or_probe() {
    let mut host = Host::new(&[]);
    host.ideal_yaw = f32::NAN;
    assert!(matches!(begin_goal(&mut host, 1.0), Ok(GoalStep::Rejected)));
    assert!(host.draws.is_empty());
    assert!(host.absolute_reads.is_empty());
    assert!(host.traces.is_empty());

    host.flags = rules(RuleSetId::Quake).ground_flag;
    host.enemy = Some(reference(9));
    host.goal_bounds = Bounds {
        mins: Vec3([f32::NAN; 3]),
        maxs: Vec3([f32::NAN; 3]),
    };
    assert!(matches!(begin_goal(&mut host, 1.0), Ok(GoalStep::Complete)));
    assert_eq!(host.absolute_reads.len(), 12);
    assert_eq!(host.target_reads, 0);
    assert!(host.draws.is_empty());
    assert!(host.traces.is_empty());
}

#[test]
fn goal_success_latches_moved_even_when_facing_restores_origin() {
    let mut host = Host::new(&[]);
    host.flags = rules(RuleSetId::Quake).fly_flag;
    host.yaw = 90.0;
    host.yaw_speed = 0.0;
    let cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    assert_eq!(host.pose.position, Vec3::default());
    assert_eq!(host.turns, [0.0]);
    assert_eq!(host.draws, [1804289383]);
    // The relink callback can move the actor again. Completion must neither
    // compare this pose with the old one nor replay the attempted step.
    host.pose.position = Vec3([25.0, 30.0, 40.0]);
    assert!(matches!(cursor.resume(&mut host), Ok(GoalStep::Complete)));
    assert_eq!(host.pose.position, Vec3([25.0, 30.0, 40.0]));
    assert_eq!(host.draws.len(), 1);
}

#[test]
fn goal_partial_ground_clears_before_its_link_boundary() {
    let mut host = Host::new(&[]);
    let rules = rules(RuleSetId::Quake);
    host.flags = rules.ground_flag | rules.partial_ground_flag;
    let cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    assert_eq!(host.pose.position, Vec3([5.0, 0.0, 0.0]));
    assert_eq!(host.flags, rules.partial_ground_flag);
    host.flags |= rules.ground_flag;
    assert!(matches!(cursor.resume(&mut host), Ok(GoalStep::Complete)));
    assert_eq!(host.flags, rules.ground_flag | rules.partial_ground_flag);
}

#[test]
fn chase_captures_after_initial_callback_then_retains_locals_across_later_callbacks() {
    let mut host = Host::new(&[]);
    host.flags = rules(RuleSetId::Quake).ground_flag;
    let cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    assert_eq!(host.turns, [0.0]);
    host.ideal_yaw = 90.0;
    host.goal_position = Vec3([-100.0, -100.0, 0.0]);
    let cursor = linked(cursor.resume(&mut host).unwrap());
    assert_eq!(host.turns, [0.0, 215.0]);
    // These changes happen after the diagonal's link. The captured southwest
    // locals and old=90/turnaround=270 must survive them.
    host.ideal_yaw = 0.0;
    host.goal_position = Vec3([100.0, 100.0, 0.0]);
    host.pose.position = Vec3([8.0, 9.0, 10.0]);
    let _cursor = linked(cursor.resume(&mut host).unwrap());
    assert_eq!(host.turns, [0.0, 215.0, 180.0]);
    assert_eq!(host.draws, [1804289383, 846930886]);
}

#[test]
fn chase_integer_abs_uses_truncated_magnitudes_and_random_is_read_after_callbacks() {
    let mut host = Host::new(&[]);
    host.flags = rules(RuleSetId::Quake).ground_flag;
    host.goal_position = Vec3([10.1, 10.9, 0.0]);
    let cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    let cursor = linked(cursor.resume(&mut host).unwrap());
    let _cursor = linked(cursor.resume(&mut host).unwrap());
    assert_eq!(host.turns, [0.0, 45.0, 0.0]);

    let mut host = Host::new(&[]);
    host.flags = rules(RuleSetId::Quake).ground_flag;
    host.goal_position = Vec3([10.1, 10.9, 0.0]);
    let cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    let cursor = linked(cursor.resume(&mut host).unwrap());
    assert_eq!(GoalHost::random_word(&mut host), 846930886);
    let _cursor = linked(cursor.resume(&mut host).unwrap());
    assert_eq!(host.turns, [0.0, 45.0, 90.0]);
    assert_eq!(host.draws, [1804289383, 846930886, 1681692777]);
}

#[test]
fn exhausted_chase_keeps_duplicate_attempts_and_both_native_sweep_orders() {
    for (seed, expected, draws) in [
        (
            1,
            vec![
                0.0, 45.0, 0.0, 90.0, 0.0, 0.0, 45.0, 90.0, 135.0, 225.0, 270.0, 315.0, 180.0,
            ],
            vec![1804289383, 846930886, 1681692777],
        ),
        (
            2,
            vec![
                0.0, 45.0, 90.0, 0.0, 0.0, 315.0, 270.0, 225.0, 135.0, 90.0, 45.0, 0.0, 180.0,
            ],
            vec![1505335290, 1738766719, 190686788],
        ),
    ] {
        let mut host = Host::new(&[]);
        let rules = rules(RuleSetId::Quake);
        host.flags = rules.ground_flag;
        host.random =
            qa_core::random::Random::with_algorithm(seed, qa_core::random::Algorithm::GlibcType3);
        let mut step = begin_goal(&mut host, 5.0).unwrap();
        let mut links = 0;
        while let GoalStep::Link(cursor) = step {
            links += 1;
            step = cursor.resume(&mut host).unwrap();
        }
        assert!(matches!(step, GoalStep::Complete));
        assert_eq!(links, expected.len());
        assert_eq!(host.turns, expected);
        assert_eq!(host.draws, draws);
        assert_eq!(host.ideal_yaw, 0.0);
        assert_eq!(host.flags, rules.ground_flag | rules.partial_ground_flag);
        assert_eq!(host.pose.position, Vec3::default());
    }
}

#[test]
fn random_initial_short_circuit_skips_the_initial_direction() {
    let mut host = Host::new(&[]);
    host.flags = rules(RuleSetId::Quake).ground_flag;
    host.random.next_u32();
    host.random.next_u32();
    let _cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    assert_eq!(host.turns, [45.0]);
    assert_eq!(host.draws, [1681692777]);
}

#[test]
fn goal_facing_rereads_ideal_yaw_after_overlapping_origin_stores() {
    let mut host = Host::new(&[]);
    host.flags = rules(RuleSetId::Quake).fly_flag;
    host.on_origin = Some(|host, axis| {
        if axis == 0 {
            host.ideal_yaw = -90.0;
        }
    });
    let cursor = linked(begin_goal(&mut host, 5.0).unwrap());
    assert_eq!(host.ideal_yaw, -90.0);
    assert_eq!(host.pose.position, Vec3::default());
    assert_eq!(host.writes.len(), 6);
    assert!(matches!(cursor.resume(&mut host), Ok(GoalStep::Complete)));
}
