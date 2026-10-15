use qa_core::primitives::{
    BodyAttachment, BodyFollow, Bounds, EntityId, LinkOrder, ModuleId, Motion, NativeEntityRef,
    ResourceNamespace, RuleSetId, ThinkTime, UserCmd, Vec3,
};
use qa_gameplay::rules::{
    motion::{client_motion_rules, step_rules, toss_rules},
    sweep::entity_sweep,
};
use qa_session::physics::{
    ClientHost, MotionFrame, MotionHost, MotionState, MotionTrace, MotionWrite, PhysicsError,
    check_velocity, client_move, step_fall, sweep_motion, toss, water_transition,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent},
    collision::{QuakeTraceKind, Trace},
    entities::{AllocationPolicy, EntityTable, FullTable},
};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Event {
    Check,
    Prepare,
    Write(MotionWrite, f32),
    Flags(u32),
    Gravity,
    Trace(QuakeTraceKind, Vec3),
    Link,
    Impact(Option<EntityId>),
    Ground(Option<NativeEntityRef>),
    Water,
    Sound,
    WaterLevel(f32),
    WaterType(f32),
    OldOrigin(usize, f32),
    Contents(Vec3),
    Diagnostic,
}

struct Host {
    entities: EntityTable,
    entity: EntityId,
    state: MotionState,
    scale: Option<f32>,
    trace: Option<MotionTrace>,
    events: Vec<Event>,
    remove_on_link: bool,
    bounce_on_impact: bool,
    shifted_clip: bool,
    remove_after_write: Option<MotionWrite>,
    traces: VecDeque<MotionTrace>,
    impact_velocities: VecDeque<Option<Vec3>>,
    impact_flags: Vec<u32>,
    origin_velocity: Option<Vec3>,
    remove_on_impact: bool,
    reuse_on_impact: bool,
    clear_ground_on_link: bool,
    client_water_level: f32,
    client_water_type: f32,
    old_origin: Vec3,
    contents: VecDeque<i32>,
    old_origin_alias: bool,
    client_teleport_time: f32,
}

impl Host {
    fn new(mode: f32) -> Self {
        let mut entities = EntityTable::new(8, 1).unwrap();
        let entity = entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(3),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        entities.columns.angular_velocity[entity.slot as usize] = Some(Vec3::default());
        Self {
            entities,
            entity,
            state: MotionState {
                move_type: mode,
                solid: Some(2.0),
                flags: Some(0),
            },
            scale: None,
            trace: None,
            events: Vec::new(),
            remove_on_link: false,
            bounce_on_impact: false,
            shifted_clip: false,
            remove_after_write: None,
            traces: VecDeque::new(),
            impact_velocities: VecDeque::new(),
            impact_flags: Vec::new(),
            origin_velocity: None,
            remove_on_impact: false,
            reuse_on_impact: false,
            clear_ground_on_link: false,
            client_water_level: 0.0,
            client_water_type: -1.0,
            old_origin: Vec3::default(),
            contents: VecDeque::new(),
            old_origin_alias: false,
            client_teleport_time: 0.0,
        }
    }

    fn run(&mut self, rules: RuleSetId, frame: MotionFrame) -> Result<(), PhysicsError> {
        let entity = self.entity;
        toss(self, entity, frame, &toss_rules(rules).unwrap())
    }

    fn fall(
        &mut self,
        rules: RuleSetId,
        frame: MotionFrame,
    ) -> Result<Option<EntityId>, PhysicsError> {
        let entity = self.entity;
        step_fall(
            self,
            entity,
            frame,
            &step_rules(rules).unwrap(),
            &entity_sweep(rules).unwrap(),
        )
    }

    fn collision(&mut self, normal: Vec3, end: Vec3) {
        let target = self
            .entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(3),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let mut trace = Trace::clear(end);
        trace.fraction = 0.5;
        trace.plane.normal = normal;
        trace.entity = Some(target);
        self.trace = Some(MotionTrace {
            trace,
            ground: NativeEntityRef::native(ResourceNamespace::Module(ModuleId(3)), 2),
        });
    }
}

impl MotionHost for Host {
    type Error = PhysicsError;
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
    fn state(&mut self, entity: EntityId) -> Result<MotionState, Self::Error> {
        self.entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        Ok(self.state)
    }
    fn prepare_motion(&mut self, entity: EntityId) -> Result<(), Self::Error> {
        self.entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        self.events.push(Event::Prepare);
        Ok(())
    }
    fn check_velocity(&mut self, entity: EntityId, max: f32) -> Result<(), Self::Error> {
        self.events.push(Event::Check);
        let events = &mut self.events;
        check_velocity::<PhysicsError>(&mut self.entities, entity, max, |entities, field, _| {
            let slot = entities.resolve(entity).ok_or(PhysicsError::Entity)?;
            let value = match field {
                MotionWrite::Velocity(axis) => entities.columns.velocity[slot].0[axis],
                MotionWrite::Origin(axis) => entities.columns.position[slot].0[axis],
                _ => unreachable!(),
            };
            events.push(Event::Write(field, value));
            Ok(())
        })?;
        Ok(())
    }
    fn write_motion(
        &mut self,
        entity: EntityId,
        field: MotionWrite,
        value: f32,
    ) -> Result<(), Self::Error> {
        let slot = self.entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        match field {
            MotionWrite::Angles(axis) => self.entities.columns.angles[slot].0[axis] = value,
            MotionWrite::AngularVelocity(axis) => {
                self.entities.columns.angular_velocity[slot]
                    .as_mut()
                    .ok_or(PhysicsError::AngularVelocity)?
                    .0[axis] = value
            }
            MotionWrite::Origin(axis) => {
                self.entities.columns.position[slot].0[axis] = value;
                if axis == 2
                    && let Some(velocity) = self.origin_velocity.take()
                {
                    self.entities.columns.velocity[slot] = velocity;
                }
            }
            MotionWrite::Velocity(axis) => {
                self.entities.columns.velocity[slot].0[axis] = value;
                if self.shifted_clip && axis == 0 {
                    // A native shifted vector overlaps the next scalar input.
                    self.entities.columns.velocity[slot].0[1] = value;
                }
            }
        }
        self.events.push(Event::Write(field, value));
        if self.remove_after_write == Some(field) {
            assert!(self.entities.release(entity, ThinkTime::Seconds(1.0)));
        }
        Ok(())
    }
    fn write_flags(&mut self, entity: EntityId, flags: u32) -> Result<(), Self::Error> {
        self.entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        self.state.flags = Some(flags);
        self.events.push(Event::Flags(flags));
        Ok(())
    }
    fn gravity_scale(&mut self, _: EntityId) -> Result<Option<f32>, Self::Error> {
        self.events.push(Event::Gravity);
        Ok(self.scale)
    }
    fn trace(
        &mut self,
        _: EntityId,
        end: Vec3,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, Self::Error> {
        self.events.push(Event::Trace(kind, end));
        Ok(self
            .traces
            .pop_front()
            .or(self.trace)
            .unwrap_or(MotionTrace {
                trace: Trace::clear(end),
                ground: None,
            }))
    }
    fn relink(&mut self, entity: EntityId) -> Result<Option<EntityId>, Self::Error> {
        self.events.push(Event::Link);
        if self.remove_on_link {
            assert!(self.entities.release(entity, ThinkTime::Seconds(1.0)));
        }
        if self.clear_ground_on_link {
            self.state.flags = Some(0);
        }
        Ok(self.entities.resolve(entity).map(|_| entity))
    }
    fn impact(
        &mut self,
        entity: Option<EntityId>,
        _: &MotionTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        self.events.push(Event::Impact(entity));
        self.impact_flags.push(self.state.flags.unwrap_or(0));
        if let Some(entity) = entity {
            if self.remove_on_impact || self.reuse_on_impact {
                assert!(self.entities.release(entity, ThinkTime::Seconds(1.0)));
                if !self.reuse_on_impact {
                    return Ok(None);
                }
                let replacement = self
                    .entities
                    .allocate(
                        ThinkTime::Seconds(1.0),
                        ModuleId(3),
                        AllocationPolicy {
                            full: FullTable::OverwriteOwned(entity),
                            ..AllocationPolicy::EDICT
                        },
                    )
                    .unwrap()
                    .id;
                self.entity = replacement;
                self.reuse_on_impact = false;
                return Ok(Some(replacement));
            }
            if let Some(Some(velocity)) = self.impact_velocities.pop_front() {
                self.entities.columns.velocity[entity.slot as usize] = velocity;
            }
        }
        if self.bounce_on_impact {
            self.state.move_type = 10.0;
        }
        Ok(entity)
    }
    fn ground(&mut self, _: EntityId, ground: Option<NativeEntityRef>) -> Result<(), Self::Error> {
        self.events.push(Event::Ground(ground));
        Ok(())
    }
    fn sound(&mut self, _: EntityId, name: &[u8]) -> Result<(), Self::Error> {
        assert_eq!(name, b"demon/dland2.wav");
        self.events.push(Event::Sound);
        Ok(())
    }
    fn water_transition(&mut self, _: EntityId) -> Result<(), Self::Error> {
        self.events.push(Event::Water);
        Ok(())
    }
}

impl ClientHost for Host {
    fn command(&mut self, _: EntityId) -> Result<UserCmd, Self::Error> {
        Ok(UserCmd::default())
    }
    fn view_angles(&mut self, _: EntityId) -> Result<Vec3, Self::Error> {
        Ok(Vec3::default())
    }
    fn view_height(&mut self, _: EntityId) -> Result<f32, Self::Error> {
        Ok(22.0)
    }
    fn water_level(&mut self, _: EntityId) -> Result<f32, Self::Error> {
        Ok(self.client_water_level)
    }
    fn write_water_level(&mut self, _: EntityId, value: f32) -> Result<(), Self::Error> {
        self.client_water_level = value;
        self.events.push(Event::WaterLevel(value));
        Ok(())
    }
    fn write_water_type(&mut self, _: EntityId, value: f32) -> Result<(), Self::Error> {
        self.client_water_type = value;
        self.events.push(Event::WaterType(value));
        Ok(())
    }
    fn old_origin(&mut self, _: EntityId, axis: usize) -> Result<f32, Self::Error> {
        Ok(self.old_origin.0[axis])
    }
    fn write_old_origin(
        &mut self,
        entity: EntityId,
        axis: usize,
        value: f32,
    ) -> Result<(), Self::Error> {
        self.old_origin.0[axis] = value;
        if self.old_origin_alias && axis == 0 {
            self.entities.columns.position[entity.slot as usize].0[1] = value;
        }
        self.events.push(Event::OldOrigin(axis, value));
        Ok(())
    }
    fn teleport_time(&mut self, _: EntityId) -> Result<f32, Self::Error> {
        Ok(self.client_teleport_time)
    }
    fn write_teleport_time(&mut self, _: EntityId, value: f32) -> Result<(), Self::Error> {
        self.client_teleport_time = value;
        Ok(())
    }
    fn move_direction(&mut self, _: EntityId, _: usize) -> Result<f32, Self::Error> {
        Ok(0.0)
    }
    fn point_contents(&mut self, _: EntityId, point: Vec3) -> Result<i32, Self::Error> {
        self.events.push(Event::Contents(point));
        Ok(self.contents.pop_front().unwrap_or(-1))
    }
    fn trace_point(&mut self, entity: EntityId, _: Vec3, end: Vec3) -> Result<Trace, Self::Error> {
        MotionHost::trace(self, entity, end, QuakeTraceKind::IgnoreBoxes).map(|trace| trace.trace)
    }
    fn tuning(&mut self, _: EntityId) -> Result<qa_movement::ControlTuning, Self::Error> {
        Ok(qa_movement::ControlTuning {
            max_speed: 320.0,
            stop_speed: 100.0,
            friction: 4.0,
            accelerate: 10.0,
            edge_friction: 2.0,
            no_step: false,
        })
    }
    fn diagnostic(&mut self, _: &'static [u8]) -> Result<(), Self::Error> {
        self.events.push(Event::Diagnostic);
        Ok(())
    }
    fn current(&mut self) -> Result<Option<EntityId>, Self::Error> {
        Ok(self.entities.resolve(self.entity).map(|_| self.entity))
    }
}

const FRAME: MotionFrame = MotionFrame {
    elapsed: 0.013271937412,
    gravity: 800.0,
    max_velocity: 2000.0,
};

const CLIENT_FRAME: MotionFrame = MotionFrame {
    elapsed: 0.1,
    ..FRAME
};

fn clear_motion(end: Vec3) -> MotionTrace {
    MotionTrace {
        trace: Trace::clear(end),
        ground: None,
    }
}

#[test]
fn client_noclip_moves_only_origin_without_internal_link_or_angles() {
    let mut host = Host::new(8.0);
    let entity = host.entity;
    let slot = entity.slot as usize;
    host.entities.columns.position[slot] = Vec3([1.0, 2.0, 3.0]);
    host.entities.columns.velocity[slot] = Vec3([4.0, 5.0, 6.0]);
    host.entities.columns.angles[slot] = Vec3([7.0, 8.0, 9.0]);
    host.entities.columns.angular_velocity[slot] = Some(Vec3([11.0, 12.0, 13.0]));
    client_move(
        &mut host,
        entity,
        Motion::Noclip,
        MotionFrame {
            elapsed: 0.25,
            ..CLIENT_FRAME
        },
        &client_motion_rules(RuleSetId::Quake).unwrap(),
    )
    .unwrap();
    assert_eq!(host.entities.columns.position[slot], Vec3([2.0, 3.25, 4.5]));
    assert_eq!(host.entities.columns.angles[slot], Vec3([7.0, 8.0, 9.0]));
    assert!(!host.events.contains(&Event::Link));
    assert!(
        !host
            .events
            .iter()
            .any(|event| matches!(event, Event::Water | Event::Impact(_)))
    );
}

#[test]
fn motion_sweep_retains_wall_plane_before_an_immediate_removal() {
    let mut host = Host::new(5.0);
    let entity = host.entity;
    host.entities.columns.velocity[entity.slot as usize] = Vec3([20.0, 0.0, 0.0]);
    host.collision(Vec3([-1.0, 0.0, 0.0]), Vec3([1.0, 0.0, 0.0]));
    host.remove_on_impact = true;
    let result = sweep_motion(
        &mut host,
        entity,
        0.1,
        &entity_sweep(RuleSetId::Quake).unwrap(),
        512,
    )
    .unwrap();
    assert_eq!(result.current, None);
    assert_eq!(result.result.blocked, 2);
    assert_eq!(
        result.wall.unwrap().trace.plane.normal,
        Vec3([-1.0, 0.0, 0.0])
    );
    assert_eq!(host.events.last(), Some(&Event::Impact(Some(entity))));
    assert!(!host.events.contains(&Event::Link));
}

#[test]
fn client_walk_samples_native_water_then_copies_live_recovery_origin() {
    let mut host = Host::new(3.0);
    let entity = host.entity;
    let slot = entity.slot as usize;
    host.state.flags = Some(512);
    host.entities.columns.position[slot] = Vec3([3.0, 4.0, 24.0]);
    host.entities.columns.velocity[slot] = Vec3([10.0, 0.0, 0.0]);
    host.entities.columns.mins[slot] = Vec3([-16.0, -16.0, -24.0]);
    host.entities.columns.maxs[slot] = Vec3([16.0, 16.0, 32.0]);
    host.contents.extend([-3, -3, -3]);
    host.old_origin_alias = true;
    client_move(
        &mut host,
        entity,
        Motion::Walk,
        CLIENT_FRAME,
        &client_motion_rules(RuleSetId::Quake).unwrap(),
    )
    .unwrap();
    let samples = host
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Contents(point) => Some(*point),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        samples,
        [
            Vec3([3.0, 4.0, 1.0]),
            Vec3([3.0, 4.0, 28.0]),
            Vec3([3.0, 4.0, 46.0])
        ]
    );
    assert_eq!(host.client_water_level, 3.0);
    assert_eq!(host.client_water_type, -3.0);
    assert_eq!(host.old_origin, Vec3([3.0, 3.0, 24.0]));
    assert_eq!(host.entities.columns.position[slot], Vec3([4.0, 3.0, 24.0]));
    assert_eq!(host.entities.columns.velocity[slot].0[2], 0.0);
    assert!(!host.events.contains(&Event::Link));
    assert!(!host.events.contains(&Event::Gravity));
}

#[test]
fn client_stuck_recovery_stops_after_a_trigger_removes_its_connection_body() {
    let mut host = Host::new(3.0);
    let entity = host.entity;
    host.entities.columns.position[entity.slot as usize] = Vec3([3.0, 4.0, 5.0]);
    host.old_origin = Vec3([-1.0, -2.0, -3.0]);
    let mut stuck = Trace::clear(Vec3([3.0, 4.0, 5.0]));
    stuck.start_solid = true;
    host.traces.push_back(MotionTrace {
        trace: stuck,
        ground: None,
    });
    host.traces.push_back(clear_motion(host.old_origin));
    host.remove_on_link = true;
    client_move(
        &mut host,
        entity,
        Motion::Walk,
        CLIENT_FRAME,
        &client_motion_rules(RuleSetId::Quake).unwrap(),
    )
    .unwrap();
    assert!(host.entities.resolve(entity).is_none());
    assert_eq!(host.events.last(), Some(&Event::Link));
    assert_eq!(
        host.events
            .iter()
            .filter(|event| matches!(event, Event::Trace(..)))
            .count(),
        2
    );
    let gravity = host
        .events
        .iter()
        .position(|event| *event == Event::Write(MotionWrite::Velocity(2), -80.0))
        .unwrap();
    let recovery = host
        .events
        .iter()
        .position(|event| matches!(event, Event::Write(MotionWrite::Origin(0), -1.0)))
        .unwrap();
    assert!(gravity < recovery);
}

#[test]
fn client_stuck_search_preserves_native_z_then_x_then_y_order() {
    let mut host = Host::new(3.0);
    let entity = host.entity;
    host.entities.columns.position[entity.slot as usize] = Vec3([10.0, 20.0, 30.0]);
    let mut stuck = Trace::clear(Vec3::default());
    stuck.start_solid = true;
    host.traces.extend([
        MotionTrace {
            trace: stuck,
            ground: None,
        },
        MotionTrace {
            trace: stuck,
            ground: None,
        },
        MotionTrace {
            trace: stuck,
            ground: None,
        },
        clear_motion(Vec3([9.0, 20.0, 30.0])),
    ]);
    client_move(
        &mut host,
        entity,
        Motion::Walk,
        MotionFrame {
            gravity: 0.0,
            ..CLIENT_FRAME
        },
        &client_motion_rules(RuleSetId::Quake).unwrap(),
    )
    .unwrap();
    let probes = host
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Trace(_, end) => Some(*end),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        probes,
        [
            Vec3([10.0, 20.0, 30.0]),
            Vec3::default(),
            Vec3([9.0, 19.0, 30.0]),
            Vec3([9.0, 20.0, 30.0])
        ]
    );
    assert_eq!(
        host.entities.columns.position[entity.slot as usize],
        Vec3([9.0, 20.0, 30.0])
    );
    assert_eq!(
        host.events
            .iter()
            .filter(|event| **event == Event::Link)
            .count(),
        1
    );
}

#[test]
fn client_stairs_touch_on_lift_and_drop_and_use_the_players_own_bsp_solid() {
    for good_floor in [false, true] {
        for bsp in [false, true] {
            let mut host = Host::new(3.0);
            let entity = host.entity;
            let slot = entity.slot as usize;
            host.state.flags = Some(512);
            host.state.solid = Some(if bsp { 4.0 } else { 2.0 });
            host.entities.columns.velocity[slot] = Vec3([100.0, 20.0, 0.0]);
            host.shifted_clip = true;
            host.collision(Vec3([-1.0, 0.0, 0.0]), Vec3::default());
            let mut wall = host.trace.take().unwrap();
            wall.trace.fraction = 0.0;
            let mut second = wall;
            second.trace.fraction = 0.5;
            second.trace.end = Vec3([2.0, 0.0, 18.0]);
            let mut down = wall;
            down.trace.fraction = 0.5;
            down.trace.end = Vec3([2.0, 1.0, 0.0]);
            down.trace.plane.normal = Vec3([0.0, 0.0, if good_floor { 1.0 } else { 0.5 }]);
            host.traces.extend([
                clear_motion(Vec3::default()),
                wall,
                clear_motion(Vec3([0.0, 2.0, 0.0])),
                clear_motion(Vec3([0.0, 0.0, 18.0])),
                second,
                clear_motion(Vec3([2.0, 1.0, 18.0])),
                down,
            ]);
            client_move(
                &mut host,
                entity,
                Motion::Walk,
                MotionFrame {
                    gravity: 0.0,
                    ..CLIENT_FRAME
                },
                &client_motion_rules(RuleSetId::Quake).unwrap(),
            )
            .unwrap();
            assert_eq!(
                host.entities.columns.position[slot],
                if good_floor {
                    Vec3([2.0, 1.0, 0.0])
                } else {
                    Vec3([0.0, 2.0, 0.0])
                }
            );
            assert_eq!(
                host.entities.columns.velocity[slot],
                Vec3([0.0, if good_floor { 10.0 } else { 20.0 }, 0.0])
            );
            assert_eq!(
                host.state.flags,
                Some(if bsp && good_floor { 512 } else { 0 })
            );
            let links = host
                .events
                .iter()
                .enumerate()
                .filter_map(|(i, e)| (*e == Event::Link).then_some(i))
                .collect::<Vec<_>>();
            let impacts = host
                .events
                .iter()
                .enumerate()
                .filter_map(|(i, e)| matches!(e, Event::Impact(_)).then_some(i))
                .collect::<Vec<_>>();
            assert_eq!((links.len(), impacts.len()), (2, 3));
            assert!(
                impacts[0] < links[0]
                    && links[0] < impacts[1]
                    && impacts[1] < links[1]
                    && links[1] < impacts[2]
            );
            assert_eq!(
                host.events
                    .iter()
                    .filter(|event| matches!(event, Event::Ground(_)))
                    .count(),
                usize::from(bsp && good_floor)
            );
        }
    }
}

#[test]
fn client_unstick_tries_all_eight_native_directions_before_the_final_drop() {
    let mut host = Host::new(3.0);
    let entity = host.entity;
    host.state.flags = Some(512);
    host.entities.columns.velocity[entity.slot as usize] = Vec3([100.0, 0.0, 0.0]);
    host.collision(Vec3([-1.0, 0.0, 0.0]), Vec3::default());
    let mut wall = host.trace.take().unwrap();
    wall.trace.fraction = 0.0;
    host.traces.extend([
        clear_motion(Vec3::default()),
        wall,
        clear_motion(Vec3([0.0, 0.0, 18.0])),
        wall,
    ]);
    let directions = [
        [2.0, 0.0],
        [0.0, 2.0],
        [-2.0, 0.0],
        [0.0, -2.0],
        [2.0, 2.0],
        [-2.0, 2.0],
        [2.0, -2.0],
        [-2.0, -2.0],
    ];
    for [x, y] in directions {
        host.traces.push_back(clear_motion(Vec3([x, y, 18.0])));
        host.traces.push_back(wall);
    }
    host.traces.push_back(clear_motion(Vec3::default()));
    client_move(
        &mut host,
        entity,
        Motion::Walk,
        MotionFrame {
            gravity: 0.0,
            ..CLIENT_FRAME
        },
        &client_motion_rules(RuleSetId::Quake).unwrap(),
    )
    .unwrap();
    let ends = host
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Trace(_, end) => Some(*end),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut expected = vec![
        Vec3::default(),
        Vec3([10.0, 0.0, 0.0]),
        Vec3([0.0, 0.0, 18.0]),
        Vec3([10.0, 0.0, 18.0]),
    ];
    for [x, y] in directions {
        expected.extend([Vec3([x, y, 18.0]), Vec3([x + 10.0, y, 18.0])]);
    }
    expected.push(Vec3::default());
    assert_eq!(ends, expected);
    assert_eq!(
        host.events
            .iter()
            .filter(|event| **event == Event::Link)
            .count(),
        10
    );
    assert_eq!(
        host.entities.columns.position[entity.slot as usize],
        Vec3::default()
    );
    assert_eq!(
        host.entities.columns.velocity[entity.slot as usize],
        Vec3::default()
    );
}

#[test]
fn toss_narrows_only_native_stores_and_vector_arguments() {
    let mut host = Host::new(6.0);
    let slot = host.entity.slot as usize;
    let velocity = Vec3([10.125, -1.0, 177.12346]);
    let angles = Vec3([-25.25, 7.5, 0.75]);
    let angular = Vec3([17.125, -39.2, 2.0]);
    host.entities.columns.velocity[slot] = velocity;
    host.entities.columns.angles[slot] = angles;
    host.entities.columns.angular_velocity[slot] = Some(angular);
    host.scale = Some(0.8317);
    host.run(RuleSetId::Quake, FRAME).unwrap();
    let z =
        (f64::from(velocity.0[2]) - f64::from(0.8317f32 * FRAME.gravity) * FRAME.elapsed) as f32;
    assert_eq!(
        host.entities.columns.velocity[slot].0[2].to_bits(),
        z.to_bits()
    );
    assert_eq!(
        host.entities.columns.angles[slot].0.map(f32::to_bits),
        std::array::from_fn(
            |axis| (angles.0[axis] + FRAME.elapsed as f32 * angular.0[axis]).to_bits()
        )
    );
    let end = Vec3([velocity.0[0], velocity.0[1], z]) * FRAME.elapsed as f32;
    assert!(
        host.events
            .contains(&Event::Trace(QuakeTraceKind::Normal, end))
    );
    assert_eq!(host.events.last(), Some(&Event::Link));
    assert!(!host.events.contains(&Event::Water));
}

#[test]
fn grounded_toss_does_not_require_unused_fields() {
    let mut host = Host::new(6.0);
    host.state.flags = Some(512);
    host.state.solid = None;
    host.entities.columns.angular_velocity[host.entity.slot as usize] = None;
    host.run(RuleSetId::Quake, FRAME).unwrap();
    assert!(host.events.is_empty());
}

#[test]
fn attached_projectile_retains_transport_with_angular_and_contact_phases() {
    for collide in [false, true] {
        let mut host = Host::new(9.0);
        let anchor = host
            .entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(3),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        let slot = host.entity.slot as usize;
        host.entities.columns.position[anchor.slot as usize] = Vec3([10.0, 20.0, 30.0]);
        host.entities.columns.velocity[slot] = Vec3([100.0, 20.0, 0.0]);
        host.entities.columns.angular_velocity[slot] = Some(Vec3([1.0, 2.0, 3.0]));
        host.entities
            .attach(
                host.entity,
                BodyAttachment {
                    anchor,
                    follow: BodyFollow::Translation,
                    offset: Vec3([2.0, 3.0, 4.0]),
                },
            )
            .unwrap();
        let mut area = AreaGrid::load(
            host.entities.capacity(),
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
        )
        .unwrap();
        assert!(area.link(
            &host.entities,
            host.entity,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit,
        ));
        let transported = area.transport_attachments(&mut host.entities);
        assert_eq!(
            (transported.visited, transported.moved, transported.relinked),
            (1, 1, 1)
        );
        let position = host.entities.columns.position[slot];
        assert_eq!(position, Vec3([12.0, 23.0, 34.0]));
        if collide {
            host.collision(Vec3([1.0, 0.0, 0.0]), position);
        }
        host.run(RuleSetId::Quake, FRAME).unwrap();
        assert_eq!(
            host.entities.columns.position[slot].0.map(f32::to_bits),
            position.0.map(f32::to_bits)
        );
        assert_eq!(
            host.entities.columns.angles[slot],
            Vec3([1.0, 2.0, 3.0]) * FRAME.elapsed as f32
        );
        assert!(
            host.events
                .contains(&Event::Trace(QuakeTraceKind::Missile, position))
        );
        assert!(host.events.contains(&Event::Link));
        assert_eq!(host.events.contains(&Event::Water), collide);
        assert_eq!(
            host.events
                .iter()
                .any(|event| matches!(event, Event::Impact(_))),
            collide
        );
    }
}

#[test]
fn a_scalar_write_cannot_continue_into_a_retired_lifetime() {
    let mut host = Host::new(9.0);
    host.remove_after_write = Some(MotionWrite::Angles(0));
    assert_eq!(host.run(RuleSetId::Quake, FRAME), Err(PhysicsError::Entity));
    assert_eq!(
        host.events.last(),
        Some(&Event::Write(MotionWrite::Angles(0), 0.0))
    );
    assert!(!host.events.contains(&Event::Link));
}

#[test]
fn quakeworld_wakes_rising_entities_and_ignores_entity_gravity() {
    let mut host = Host::new(6.0);
    let slot = host.entity.slot as usize;
    host.state.flags = Some(512 | 32);
    host.entities.columns.velocity[slot].0[2] = 120.0;
    host.scale = Some(100.0);
    host.run(RuleSetId::QuakeWorld, FRAME).unwrap();
    assert_eq!(host.events[0], Event::Flags(32));
    assert!(!host.events.contains(&Event::Gravity));
    let expected = (120.0 - f64::from(FRAME.gravity) * FRAME.elapsed) as f32;
    assert_eq!(
        host.entities.columns.velocity[slot].0[2].to_bits(),
        expected.to_bits()
    );
}

#[test]
fn missiles_skip_gravity_and_solid_while_fly_uses_native_solid_filter() {
    for (mode, solid, kind) in [
        (9.0, None, QuakeTraceKind::Missile),
        (5.0, Some(1.0), QuakeTraceKind::IgnoreBoxes),
        (5.0, Some(0.0), QuakeTraceKind::IgnoreBoxes),
        (5.0, Some(2.0), QuakeTraceKind::Normal),
    ] {
        let mut host = Host::new(mode);
        host.state.solid = solid;
        host.run(RuleSetId::Quake, FRAME).unwrap();
        assert!(!host.events.contains(&Event::Gravity));
        assert!(host.events.contains(&Event::Trace(kind, Vec3::default())));
    }
}

#[test]
fn trigger_removal_still_runs_target_impact_without_following_stale_lifetime() {
    let mut host = Host::new(9.0);
    host.collision(Vec3([0.0, 0.0, 1.0]), Vec3([2.0, 3.0, 4.0]));
    host.remove_on_link = true;
    host.run(RuleSetId::Quake, FRAME).unwrap();
    assert_eq!(
        &host.events[host.events.len() - 2..],
        &[Event::Link, Event::Impact(None)]
    );
    assert_eq!(host.entities.resolve(host.entity), None);
    assert!(!host.events.contains(&Event::Water));
}

#[test]
fn impact_rereads_bounce_and_clip_reads_refreshed_scalar_inputs() {
    let mut host = Host::new(9.0);
    let slot = host.entity.slot as usize;
    host.entities.columns.velocity[slot] = Vec3([100.0, 200.0, 300.0]);
    host.collision(Vec3([1.0, 0.0, 0.0]), Vec3::default());
    host.bounce_on_impact = true;
    host.shifted_clip = true;
    host.run(RuleSetId::Quake, FRAME).unwrap();
    assert_eq!(
        host.entities.columns.velocity[slot],
        Vec3([-50.0, -50.0, 300.0])
    );
    let link = host
        .events
        .iter()
        .position(|event| *event == Event::Link)
        .unwrap();
    assert_eq!(host.events[link + 1], Event::Impact(Some(host.entity)));
    assert_eq!(
        host.events[link + 2],
        Event::Write(MotionWrite::Velocity(0), -50.0)
    );
    assert_eq!(host.events.last(), Some(&Event::Water));
}

#[test]
fn floor_settle_orders_flags_ground_velocity_and_angular_velocity() {
    let mut host = Host::new(9.0);
    let slot = host.entity.slot as usize;
    host.entities.columns.velocity[slot] = Vec3([100.0, 20.0, -40.0]);
    host.collision(Vec3([0.0, 0.0, 1.0]), Vec3::default());
    host.run(RuleSetId::Quake, FRAME).unwrap();
    let flags = host
        .events
        .iter()
        .position(|event| *event == Event::Flags(512))
        .unwrap();
    assert_eq!(
        host.events[flags + 1],
        Event::Ground(host.trace.unwrap().ground)
    );
    assert_eq!(
        &host.events[flags + 2..flags + 8],
        &[
            Event::Write(MotionWrite::Velocity(0), 0.0),
            Event::Write(MotionWrite::Velocity(1), 0.0),
            Event::Write(MotionWrite::Velocity(2), 0.0),
            Event::Write(MotionWrite::AngularVelocity(0), 0.0),
            Event::Write(MotionWrite::AngularVelocity(1), 0.0),
            Event::Write(MotionWrite::AngularVelocity(2), 0.0),
        ]
    );
    assert_eq!(host.events.last(), Some(&Event::Water));
}

#[test]
fn water_transition_preserves_spawn_and_dry_native_values() {
    for (previous, contents, expected) in [
        (0.0, -1, (false, -1.0, 1.0)),
        (0.0, -3, (false, -3.0, 1.0)),
        (-1.0, -3, (true, -3.0, 1.0)),
        (-3.0, -4, (false, -4.0, 1.0)),
        (-3.0, -1, (true, -1.0, -1.0)),
        (-1.0, -2, (false, -1.0, -2.0)),
    ] {
        let actual = water_transition(previous, contents);
        assert_eq!(
            (actual.splash, actual.water_type, actual.water_level),
            expected
        );
    }
}

#[test]
fn step_applies_gravity_before_velocity_limits_without_angular_fields() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut host = Host::new(4.0);
        let slot = host.entity.slot as usize;
        host.entities.columns.angular_velocity[slot] = None;
        host.state.solid = None;
        let frame = MotionFrame {
            elapsed: 1.0,
            gravity: 800.0,
            max_velocity: 100.0,
        };
        assert_eq!(host.fall(rules, frame).unwrap(), Some(host.entity));
        assert_eq!(
            host.entities.columns.velocity[slot],
            Vec3([0.0, 0.0, -100.0])
        );
        assert!(host.events.contains(&Event::Trace(
            QuakeTraceKind::Normal,
            Vec3([0.0, 0.0, -100.0])
        )));
        let gravity = host
            .events
            .iter()
            .position(|event| *event == Event::Write(MotionWrite::Velocity(2), -800.0))
            .unwrap();
        let checking = host
            .events
            .iter()
            .position(|event| *event == Event::Check)
            .unwrap();
        assert!(gravity < checking);
        assert!(!host.events.contains(&Event::Prepare));
        assert!(!host.events.contains(&Event::Water));
        assert_eq!(host.events.last(), Some(&Event::Link));
    }
}

#[test]
fn step_ground_fly_and_swim_skip_only_the_falling_phase() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for flags in [512, 1, 2, 515] {
            let mut host = Host::new(4.0);
            host.state.flags = Some(flags);
            host.state.solid = None;
            host.entities.columns.angular_velocity[host.entity.slot as usize] = None;
            assert_eq!(host.fall(rules, FRAME).unwrap(), Some(host.entity));
            assert!(host.events.is_empty());
        }
    }
}

#[test]
fn step_grounding_precedes_impact_and_landing_sound_follows_trigger_relink() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for brush in [false, true] {
            let mut host = Host::new(4.0);
            host.entities.columns.velocity[host.entity.slot as usize] = Vec3([0.0, 0.0, -81.0]);
            host.collision(Vec3([0.0, 0.0, 1.0]), Vec3::default());
            host.trace.as_mut().unwrap().trace.brush_solid = brush;
            // One collision then a clear trace, including QW's zero-speed trace.
            host.traces.push_back(host.trace.take().unwrap());
            host.fall(rules, FRAME).unwrap();
            assert_eq!(host.impact_flags, [if brush { 512 } else { 0 }]);
            let impact = host
                .events
                .iter()
                .position(|event| matches!(event, Event::Impact(_)))
                .unwrap();
            let relink = host
                .events
                .iter()
                .position(|event| *event == Event::Link)
                .unwrap();
            assert!(impact < relink);
            if brush {
                let ground = host
                    .events
                    .iter()
                    .position(|event| matches!(event, Event::Ground(_)))
                    .unwrap();
                assert!(ground < impact);
                assert_eq!(&host.events[relink..], &[Event::Link, Event::Sound]);
            } else {
                assert!(!host.events.contains(&Event::Sound));
                assert!(
                    !host
                        .events
                        .iter()
                        .any(|event| matches!(event, Event::Ground(_)))
                );
            }
            assert!(!host.events.contains(&Event::Water));
        }
    }
}

#[test]
fn step_landing_checks_current_flags_after_triggers_and_captures_speed_before_gravity() {
    for (speed, clear_ground, sound) in [
        (-80.0, false, false),
        (-81.0, true, false),
        (-81.0, false, true),
    ] {
        let mut host = Host::new(4.0);
        host.entities.columns.velocity[host.entity.slot as usize].0[2] = speed;
        host.collision(Vec3([0.0, 0.0, 1.0]), Vec3::default());
        host.clear_ground_on_link = clear_ground;
        host.fall(RuleSetId::Quake, FRAME).unwrap();
        assert_eq!(host.events.contains(&Event::Sound), sound);
    }
}

#[test]
fn step_qw_entity_sweep_does_not_inherit_player_startsolid_stop() {
    use qa_gameplay::rules::sweep::player_sweep;
    for player in [false, true] {
        let mut host = Host::new(4.0);
        let slot = host.entity.slot as usize;
        let initial = Vec3([100.0, 20.0, 0.0]);
        host.entities.columns.velocity[slot] = initial;
        let end = Vec3([5.0, 6.0, 7.0]);
        let mut trace = Trace::clear(end);
        trace.start_solid = true;
        host.trace = Some(MotionTrace {
            trace,
            ground: None,
        });
        let policy = if player {
            player_sweep(RuleSetId::QuakeWorld)
        } else {
            entity_sweep(RuleSetId::QuakeWorld)
        }
        .unwrap();
        let entity = host.entity;
        step_fall(
            &mut host,
            entity,
            MotionFrame {
                gravity: 0.0,
                ..FRAME
            },
            &step_rules(RuleSetId::QuakeWorld).unwrap(),
            &policy,
        )
        .unwrap();
        assert_eq!(
            host.entities.columns.position[slot],
            if player { Vec3::default() } else { end }
        );
        assert_eq!(
            host.entities.columns.velocity[slot],
            if player { Vec3::default() } else { initial }
        );
    }
}

#[test]
fn step_refreshes_original_velocity_after_aliased_origin_stores() {
    let mut host = Host::new(4.0);
    let slot = host.entity.slot as usize;
    host.entities.columns.velocity[slot] = Vec3([100.0, 200.0, 0.0]);
    host.origin_velocity = Some(Vec3([50.0, 25.0, 0.0]));
    host.collision(Vec3([1.0, 0.0, 0.0]), Vec3([2.0, 3.0, 4.0]));
    host.traces.push_back(host.trace.take().unwrap());
    host.fall(
        RuleSetId::Quake,
        MotionFrame {
            gravity: 0.0,
            ..FRAME
        },
    )
    .unwrap();
    assert_eq!(host.entities.columns.velocity[slot], Vec3([0.0, 25.0, 0.0]));
}

#[test]
fn step_crease_uses_current_velocity_after_the_second_impact() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let mut host = Host::new(4.0);
        let slot = host.entity.slot as usize;
        host.entities.columns.velocity[slot] = Vec3([-100.0, -100.0, 30.0]);
        host.collision(Vec3([1.0, 0.0, 0.0]), Vec3::default());
        let first = host.trace.take().unwrap();
        let second = MotionTrace {
            trace: Trace {
                fraction: 0.0,
                plane: qa_core::primitives::Plane::oriented(Vec3([-0.6, 0.8, 0.0]), 0.0),
                ..first.trace
            },
            ..first
        };
        host.traces.extend([first, second]);
        host.impact_velocities
            .extend([None, Some(Vec3([0.0, -100.0, 77.0]))]);
        host.fall(
            rules,
            MotionFrame {
                gravity: 0.0,
                ..FRAME
            },
        )
        .unwrap();
        let direction = qa_core::math::cross(first.trace.plane.normal, second.trace.plane.normal);
        let expected = direction * direction.dot(Vec3([0.0, -100.0, 77.0]));
        assert_eq!(
            host.entities.columns.velocity[slot].0.map(f32::to_bits),
            expected.0.map(f32::to_bits)
        );
        assert_eq!(host.impact_flags.len(), 2);
    }
}

#[test]
fn step_impact_removal_stops_and_native_slot_reuse_rebinds_explicitly() {
    for reuse in [false, true] {
        let mut host = Host::new(4.0);
        let original = host.entity;
        host.entities.columns.velocity[original.slot as usize] = Vec3([-100.0, 200.0, 0.0]);
        host.collision(Vec3([1.0, 0.0, 0.0]), Vec3::default());
        host.traces.push_back(host.trace.take().unwrap());
        host.remove_on_impact = true;
        host.reuse_on_impact = reuse;
        let current = host
            .fall(
                RuleSetId::Quake,
                MotionFrame {
                    gravity: 0.0,
                    ..FRAME
                },
            )
            .unwrap();
        assert_eq!(host.entities.resolve(original), None);
        if reuse {
            let current = current.unwrap();
            assert_eq!(current.slot, original.slot);
            assert_ne!(current.generation, original.generation);
            assert_eq!(
                host.entities.columns.velocity[current.slot as usize],
                Vec3([0.0, 200.0, 0.0])
            );
            assert_eq!(host.events.last(), Some(&Event::Link));
        } else {
            assert_eq!(current, None);
            assert!(!host.events.contains(&Event::Link));
        }
    }
}
