use qa_core::primitives::{
    Bounds, EntityPose, GeometryId, IndexSpan, ModuleId, NativeEntityRef, Plane, ResourceNamespace,
    RuleSetId, SurfaceFlags, Vec3,
};
use qa_gameplay::rules::motion::{StepRules, step_rules};
use qa_session::physics::{
    MonsterHost, MotionTrace, PhysicsError, StepMove, StepPost, finish_step, move_step,
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
