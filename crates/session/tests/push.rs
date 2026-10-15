use qa_core::primitives::*;
use qa_gameplay::rules::motion::push_rules;
use qa_session::physics::{
    MotionWrite, PhysicsError, PushFrame, PushHost, PushNumber, PushUndo, push_frame,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent},
    collision::{
        CollisionStore, Contents, TraceQuery, TraceScratch, WorldTrace,
        brushes::{Brush, BrushTree},
        surfaces::SurfaceTable,
        trace_policy,
    },
    entities::{AllocationPolicy, EntityTable},
};

#[derive(Clone)]
struct Meta {
    mode: f32,
    solid: f32,
    local: f32,
    deadline: f32,
    ground: CollisionOwner,
    free: bool,
}
#[derive(Debug, PartialEq)]
enum Event {
    Link(usize, bool),
    Push(usize),
    Blocked(usize),
    Think,
    Solid(f32),
}
struct Host {
    entities: EntityTable,
    area: AreaGrid,
    store: CollisionStore,
    geometry: GeometryId,
    scratch: TraceScratch,
    meta: Vec<Meta>,
    events: Vec<Event>,
    source: RuleSetId,
    on_block: Option<fn(&mut Host)>,
    on_link: Option<fn(&mut Host, usize, bool)>,
    on_think: Option<fn(&mut Host)>,
    fail_push: bool,
    local_alias: bool,
}
fn native(slot: usize) -> CollisionOwner {
    CollisionOwner::Native(NativeEntity {
        module: ModuleId(7),
        slot: slot as i32,
    })
}
fn map(store: &mut CollisionStore, boxes: &[Bounds]) -> GeometryId {
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
            contents: Contents::SOLID,
        });
    }
    store
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
        .unwrap()
}
impl Host {
    fn new(source: RuleSetId, boxes: &[Bounds]) -> Self {
        let mut store = CollisionStore::new();
        let geometry = map(&mut store, boxes);
        let scratch = store.scratch();
        let mut host = Self {
            entities: EntityTable::new(16, 1).unwrap(),
            area: AreaGrid::load(
                16,
                Bounds {
                    mins: Vec3([-4096.0; 3]),
                    maxs: Vec3([4096.0; 3]),
                },
            )
            .unwrap(),
            store,
            geometry,
            scratch,
            meta: vec![Meta {
                mode: 0.0,
                solid: 0.0,
                local: 0.0,
                deadline: 0.0,
                ground: CollisionOwner::None,
                free: false,
            }],
            events: Vec::new(),
            source,
            on_block: None,
            on_link: None,
            on_think: None,
            fail_push: false,
            local_alias: false,
        };
        assert_eq!(host.add(Vec3::default(), 7.0, 4.0), 1);
        host.meta[1].deadline = 10.0;
        host.entities.columns.velocity[1] = Vec3([5.0, 0.0, 0.0]);
        host.entities.columns.mins[1] = Vec3([-2.0; 3]);
        host.entities.columns.maxs[1] = Vec3([2.0; 3]);
        host.link(native(1), false).unwrap();
        host.events.clear();
        host
    }
    fn add(&mut self, position: Vec3, mode: f32, solid: f32) -> usize {
        let id = self
            .entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(7),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let slot = id.slot as usize;
        assert_eq!(slot, self.meta.len());
        self.meta.push(Meta {
            mode,
            solid,
            local: 0.0,
            deadline: 10.0,
            ground: native(1),
            free: false,
        });
        self.entities.columns.position[slot] = position;
        self.entities.columns.mins[slot] = Vec3([-0.5; 3]);
        self.entities.columns.maxs[slot] = Vec3([0.5; 3]);
        self.entities.columns.flags[slot] = 512;
        self.entities.columns.collision_owner[slot] = CollisionOwner::None;
        self.entities.columns.collision_shape[slot] = if solid == 0.0 {
            CollisionShape::None
        } else {
            CollisionShape::Box
        };
        self.entities.columns.collision_contents[slot] = Contents::SOLID.0;
        self.link(native(slot), false).unwrap();
        slot
    }
    fn slot(&self, owner: CollisionOwner) -> Result<usize, PhysicsError> {
        match owner {
            CollisionOwner::Native(n)
                if n.module == ModuleId(7)
                    && n.slot >= 0
                    && (n.slot as usize) < self.meta.len() =>
            {
                Ok(n.slot as usize)
            }
            CollisionOwner::Lifetime(id) => self.entities.resolve(id).ok_or(PhysicsError::Entity),
            _ => Err(PhysicsError::Entity),
        }
    }
    fn trace(
        &mut self,
        slot: usize,
        end: Vec3,
    ) -> Result<qa_world::collision::Trace, PhysicsError> {
        let (rules, policy) = trace_policy(self.source);
        let body = self.entities.columns.body(slot);
        let pass = self
            .entities
            .id_at(slot)
            .map(CollisionOwner::Lifetime)
            .unwrap_or_default();
        let mut query = TraceQuery::point(body.position, end, rules, policy);
        query.mins = body.mins;
        query.maxs = body.maxs;
        WorldTrace::new(
            &self.store,
            self.geometry,
            0,
            &self.entities,
            &self.area,
            &mut self.scratch,
            pass,
        )
        .trace(query)
        .map_err(|_| PhysicsError::Entity)
    }
    fn run(&mut self, capacity: usize, elapsed: f64) -> Result<(), PhysicsError> {
        let rules = push_rules(self.source).unwrap();
        push_frame(
            self,
            native(1),
            &mut PushUndo::load(capacity),
            PushFrame {
                elapsed,
                world_time: ThinkTime::Seconds(77.0),
            },
            &rules,
        )
    }
}
impl PushHost for Host {
    type Error = PhysicsError;
    fn next_candidate(
        &mut self,
        cursor: &mut usize,
    ) -> Result<Option<CollisionOwner>, PhysicsError> {
        if *cursor >= self.meta.len() {
            return Ok(None);
        }
        let slot = *cursor;
        *cursor += 1;
        Ok(Some(native(slot)))
    }
    fn free(&mut self, owner: CollisionOwner) -> Result<bool, PhysicsError> {
        Ok(self.meta[self.slot(owner)?].free)
    }
    fn number(&mut self, owner: CollisionOwner, field: PushNumber) -> Result<f32, PhysicsError> {
        let s = self.slot(owner)?;
        Ok(match field {
            PushNumber::Motion(MotionWrite::Origin(a)) => self.entities.columns.position[s].0[a],
            PushNumber::Motion(MotionWrite::Velocity(a)) => self.entities.columns.velocity[s].0[a],
            PushNumber::Motion(_) => return Err(PhysicsError::MissingField),
            PushNumber::LocalTime => self.meta[s].local,
            PushNumber::NextThink => self.meta[s].deadline,
            PushNumber::MoveType => self.meta[s].mode,
            PushNumber::Solid => self.meta[s].solid,
            PushNumber::Min(a) => self.entities.columns.mins[s].0[a],
            PushNumber::Max(a) => self.entities.columns.maxs[s].0[a],
            PushNumber::AbsoluteMin(a) => {
                self.area
                    .bounds(self.entities.id_at(s).ok_or(PhysicsError::Entity)?)
                    .ok_or(PhysicsError::Entity)?
                    .mins
                    .0[a]
            }
            PushNumber::AbsoluteMax(a) => {
                self.area
                    .bounds(self.entities.id_at(s).ok_or(PhysicsError::Entity)?)
                    .ok_or(PhysicsError::Entity)?
                    .maxs
                    .0[a]
            }
        })
    }
    fn store(
        &mut self,
        owner: CollisionOwner,
        field: PushNumber,
        value: f32,
    ) -> Result<(), PhysicsError> {
        let s = self.slot(owner)?;
        match field {
            PushNumber::Motion(MotionWrite::Origin(a)) => {
                self.entities.columns.position[s].0[a] = value;
                if self.local_alias && s == 1 && a == 0 {
                    self.meta[1].local = value;
                }
            }
            PushNumber::LocalTime => self.meta[s].local = value,
            PushNumber::NextThink => self.meta[s].deadline = value,
            PushNumber::Min(a) => self.entities.columns.mins[s].0[a] = value,
            PushNumber::Max(a) => self.entities.columns.maxs[s].0[a] = value,
            PushNumber::Solid => {
                self.meta[s].solid = value;
                self.entities.columns.collision_shape[s] = if value == 0.0 {
                    CollisionShape::None
                } else {
                    CollisionShape::Box
                };
                self.events.push(Event::Solid(value));
            }
            _ => return Err(PhysicsError::MissingField),
        }
        Ok(())
    }
    fn flags(&mut self, owner: CollisionOwner) -> Result<u32, PhysicsError> {
        Ok(self.entities.columns.flags[self.slot(owner)?])
    }
    fn store_flags(&mut self, owner: CollisionOwner, value: u32) -> Result<(), PhysicsError> {
        let s = self.slot(owner)?;
        self.entities.columns.flags[s] = value;
        Ok(())
    }
    fn ground_owner(&mut self, owner: CollisionOwner) -> Result<CollisionOwner, PhysicsError> {
        Ok(self.meta[self.slot(owner)?].ground)
    }
    fn same_owner(&self, left: CollisionOwner, right: CollisionOwner) -> bool {
        self.entities.same_owner(left, right)
    }
    fn test_position(&mut self, owner: CollisionOwner) -> Result<bool, PhysicsError> {
        let s = self.slot(owner)?;
        Ok(self
            .trace(s, self.entities.columns.position[s])?
            .start_solid)
    }
    fn link(&mut self, owner: CollisionOwner, touch: bool) -> Result<(), PhysicsError> {
        let s = self.slot(owner)?;
        self.events.push(Event::Link(s, touch));
        if let Some(id) = self.entities.id_at(s) {
            assert!(self.area.link(
                &self.entities,
                id,
                LinkFlags::SOLID,
                LinkOrder::Tail,
                LinkIntent::Explicit
            ));
        }
        if let Some(callback) = self.on_link {
            callback(self, s, touch);
        }
        Ok(())
    }
    fn push_entity(&mut self, owner: CollisionOwner, movement: Vec3) -> Result<(), PhysicsError> {
        let s = self.slot(owner)?;
        self.events.push(Event::Push(s));
        if self.fail_push {
            return Err(PhysicsError::MissingField);
        }
        let trace = self.trace(s, self.entities.columns.position[s] + movement)?;
        for a in 0..3 {
            self.store(
                owner,
                PushNumber::Motion(MotionWrite::Origin(a)),
                trace.end.0[a],
            )?;
        }
        self.link(owner, true)
    }
    fn blocked(&mut self, _: CollisionOwner, blocker: CollisionOwner) -> Result<(), PhysicsError> {
        self.events.push(Event::Blocked(self.slot(blocker)?));
        if let Some(callback) = self.on_block {
            callback(self);
        }
        Ok(())
    }
    fn think_once(&mut self, pusher: CollisionOwner, time: ThinkTime) -> Result<(), PhysicsError> {
        assert_eq!(time, ThinkTime::Seconds(77.0));
        let s = self.slot(pusher)?;
        self.meta[s].deadline = 0.0;
        self.events.push(Event::Think);
        if let Some(callback) = self.on_think {
            callback(self);
        }
        Ok(())
    }
}

#[test]
fn canonical_ground_owner_carries_a_rider_outside_the_pushers_bounds() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut host = Host::new(source, &[]);
        let pusher = host.entities.id_at(1).unwrap();
        let CollisionOwner::Native(native_pusher) = native(1) else {
            unreachable!()
        };
        host.entities.columns.native_entity[pusher.slot as usize] = Some(native_pusher);
        let rider = host.add(Vec3([0.0, 0.0, 20.0]), 3.0, 2.0);
        host.meta[rider].ground = CollisionOwner::Lifetime(pusher);
        host.run(8, 1.0).unwrap();
        assert_eq!(
            host.entities.columns.position[rider],
            Vec3([5.0, 0.0, 20.0])
        );
    }
}

#[test]
fn local_clock_zero_velocity_and_double_gate_keep_native_float_boundaries() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut h = Host::new(source, &[]);
        h.entities.columns.velocity[1] = Vec3::default();
        h.meta[1].local = 16_777_216.0;
        h.meta[1].deadline = 16_777_216.0;
        h.run(0, 0.5).unwrap();
        assert_eq!(h.meta[1].local, 16_777_216.0);
        assert!(h.events.is_empty());
        h.meta[1].local = 0.0;
        h.meta[1].deadline = 0.0;
        h.run(0, 1.0).unwrap();
        assert_eq!(h.meta[1].local, 0.0);
        h.meta[1].deadline = 0.25;
        h.run(0, 1.0).unwrap();
        assert_eq!(h.meta[1].local, 0.25);
        assert_eq!(h.events, [Event::Think]);
    }
}
#[test]
fn moving_passengers_uses_traced_touch_only_for_nq_and_commits_local_time() {
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut h = Host::new(source, &[]);
        let p = h.add(Vec3([3.0, 0.0, 0.0]), 3.0, 2.0);
        h.events.clear();
        h.run(4, 1.0).unwrap();
        assert_eq!(h.entities.columns.position[1], Vec3([5.0, 0.0, 0.0]));
        assert_eq!(h.entities.columns.position[p], Vec3([8.0, 0.0, 0.0]));
        assert_eq!(h.meta[1].local, 1.0);
        assert_eq!(
            h.events.contains(&Event::Push(p)),
            source == RuleSetId::Quake
        );
        assert_eq!(
            h.events.contains(&Event::Link(p, true)),
            source == RuleSetId::Quake
        );
    }
}
#[test]
fn blocked_observes_restored_pusher_before_forward_origin_only_rollback() {
    let wall = Bounds {
        mins: Vec3([7.0, 0.0, -20.0]),
        maxs: Vec3([9.0, 3.0, 20.0]),
    };
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut h = Host::new(source, &[wall]);
        let first = h.add(Vec3([3.0, -1.0, 0.0]), 6.0, 2.0);
        let second = h.add(Vec3([3.0, 1.0, 0.0]), 3.0, 2.0);
        h.events.clear();
        h.on_block = Some(|h| {
            assert_eq!(h.entities.columns.position[1], Vec3::default());
            assert_eq!(h.entities.columns.position[2].0[0], 8.0);
            assert_eq!(h.meta[1].local, 0.0);
            h.entities.columns.velocity[2] = Vec3([99.0; 3]);
            h.entities.columns.flags[2] |= 0x8000;
            h.entities.columns.position[2].0[1] = 123.0;
        });
        h.run(8, 1.0).unwrap();
        assert!(h.events.contains(&Event::Blocked(second)));
        assert_eq!(h.entities.columns.position[first], Vec3([3.0, -1.0, 0.0]));
        assert_eq!(h.entities.columns.position[second], Vec3([3.0, 1.0, 0.0]));
        assert_eq!(h.entities.columns.velocity[first], Vec3([99.0; 3]));
        assert_ne!(h.entities.columns.flags[first] & 0x8000, 0);
        assert_eq!(
            h.entities.columns.flags[first] & 512,
            if source == RuleSetId::Quake { 0 } else { 512 }
        );
        let blocked = h
            .events
            .iter()
            .position(|e| *e == Event::Blocked(second))
            .unwrap();
        assert_eq!(
            &h.events[blocked + 1..],
            [Event::Link(first, false), Event::Link(second, false)]
        );
        assert_eq!(
            h.events[..blocked]
                .iter()
                .filter(|e| **e == Event::Link(second, true))
                .count(),
            if source == RuleSetId::Quake { 2 } else { 0 }
        );
    }
}
#[test]
fn qw_leaves_usable_old_position_and_skips_preexisting_world_stuck() {
    let mut h = Host::new(RuleSetId::QuakeWorld, &[]);
    let p = h.add(Vec3::default(), 3.0, 2.0);
    h.events.clear();
    h.run(4, 1.0).unwrap();
    assert_eq!(h.entities.columns.position[p], Vec3::default());
    assert!(!h.events.iter().any(|e| matches!(e, Event::Blocked(_))));
    assert!(!h.events.contains(&Event::Link(p, false)));
    let box_world = Bounds {
        mins: Vec3([-1.0; 3]),
        maxs: Vec3([1.0; 3]),
    };
    let mut h = Host::new(RuleSetId::QuakeWorld, &[box_world]);
    let p = h.add(Vec3::default(), 3.0, 2.0);
    h.events.clear();
    h.run(0, 1.0).unwrap();
    assert_eq!(h.entities.columns.position[p], Vec3::default());
    assert_eq!(h.meta[1].local, 1.0);
}
#[test]
fn temporary_solid_restores_on_error_and_capacity_is_bounded() {
    let mut h = Host::new(RuleSetId::Quake, &[]);
    h.add(Vec3([3.0, 0.0, 0.0]), 3.0, 2.0);
    h.fail_push = true;
    assert_eq!(h.run(2, 1.0), Err(PhysicsError::MissingField));
    assert_eq!(h.meta[1].solid, 4.0);
    let mut h = Host::new(RuleSetId::Quake, &[]);
    h.add(Vec3([3.0, 0.0, 0.0]), 3.0, 2.0);
    assert_eq!(h.run(0, 1.0), Err(PhysicsError::Capacity));
    assert!(!h.events.iter().any(|e| matches!(e, Event::Push(_))));
}
#[test]
fn current_localtime_after_origin_alias_is_used_for_commit_and_failure_subtraction() {
    let mut h = Host::new(RuleSetId::Quake, &[]);
    h.local_alias = true;
    h.run(0, 1.0).unwrap();
    assert_eq!(h.meta[1].local, 6.0);
    let wall = Bounds {
        mins: Vec3([7.0, -4.0, -20.0]),
        maxs: Vec3([9.0, 4.0, 20.0]),
    };
    let mut h = Host::new(RuleSetId::Quake, &[wall]);
    h.add(Vec3([3.0, 0.0, 0.0]), 3.0, 2.0);
    h.on_link = Some(|h, s, touch| {
        if s == 1 && !touch && h.entities.columns.position[1].0[0] == 0.0 {
            h.meta[1].local = 12.0;
        }
    });
    h.run(3, 1.0).unwrap();
    assert_eq!(h.meta[1].local, 11.0);
}
#[test]
fn qw_postthink_snap_reuses_displacement_without_advancing_clock_again() {
    let mut h = Host::new(RuleSetId::QuakeWorld, &[]);
    h.meta[1].deadline = 0.25;
    h.on_think = Some(|h| {
        h.entities.columns.position[1].0[1] += 2.0;
    });
    h.run(0, 1.0).unwrap();
    assert_eq!(h.entities.columns.position[1], Vec3([1.25, 2.0, 0.0]));
    assert_eq!(h.meta[1].local, 0.25);
    assert_eq!(
        h.events
            .iter()
            .filter(|e| **e == Event::Link(1, false))
            .count(),
        2
    );
}

#[test]
fn blocked_undo_writes_reused_native_backing_without_restoring_other_fields() {
    let wall = Bounds {
        mins: Vec3([7.0, 0.0, -20.0]),
        maxs: Vec3([9.0, 3.0, 20.0]),
    };
    let mut h = Host::new(RuleSetId::Quake, &[wall]);
    h.add(Vec3([3.0, -1.0, 0.0]), 6.0, 2.0);
    h.add(Vec3([3.0, 1.0, 0.0]), 3.0, 2.0);
    h.on_block = Some(|h| {
        let old = h.entities.id_at(2).unwrap();
        h.area.unlink(old);
        assert!(h.entities.release(old, ThinkTime::Seconds(1.0)));
        let new = h
            .entities
            .allocate(
                ThinkTime::Seconds(3.0),
                ModuleId(7),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        assert_eq!(new.slot, 2);
        assert_ne!(old, new);
        h.entities.columns.position[2] = Vec3([123.0; 3]);
        h.entities.columns.velocity[2] = Vec3([71.0; 3]);
        h.entities.columns.flags[2] = 0x8000;
    });
    h.run(8, 1.0).unwrap();
    assert_eq!(h.entities.columns.position[2], Vec3([3.0, -1.0, 0.0]));
    assert_eq!(h.entities.columns.velocity[2], Vec3([71.0; 3]));
    assert_eq!(h.entities.columns.flags[2], 0x8000);
}
#[test]
fn later_native_spawn_joins_the_same_monotonic_scan() {
    let mut h = Host::new(RuleSetId::Quake, &[]);
    h.add(Vec3([3.0, 0.0, 0.0]), 3.0, 2.0);
    h.on_link = Some(|h, s, touch| {
        if s == 2 && touch {
            h.on_link = None;
            assert_eq!(h.add(Vec3([3.0, 3.0, 0.0]), 3.0, 2.0), 3);
        }
    });
    h.run(8, 1.0).unwrap();
    assert_eq!(h.entities.columns.position[3], Vec3([8.0, 3.0, 0.0]));
    assert!(h.events.contains(&Event::Push(3)));
}
#[test]
fn point_and_corpse_exceptions_keep_native_relink_difference() {
    let wall = Bounds {
        mins: Vec3([7.0, -4.0, -20.0]),
        maxs: Vec3([9.0, 4.0, 20.0]),
    };
    for source in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for corpse in [false, true] {
            let mut h = Host::new(source, &[wall]);
            let p = h.add(Vec3([3.25, 0.0, 0.0]), 3.0, if corpse { 0.0 } else { 2.0 });
            if !corpse {
                h.entities.columns.mins[p].0[0] = 0.0;
                h.entities.columns.maxs[p].0[0] = 0.0;
            }
            h.link(native(p), false).unwrap();
            h.events.clear();
            h.run(4, 1.0).unwrap();
            assert!(!h.events.iter().any(|e| matches!(e, Event::Blocked(_))));
            if corpse {
                assert_eq!(h.entities.columns.mins[p], Vec3([0.0, 0.0, -0.5]));
                assert_eq!(h.entities.columns.maxs[p], Vec3([0.0, 0.0, -0.5]));
            }
            assert_eq!(
                h.events.contains(&Event::Link(p, false)),
                source == RuleSetId::QuakeWorld
            );
        }
    }
}
