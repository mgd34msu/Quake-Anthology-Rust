use qa_core::primitives::{
    BodyAttachment, BodyFollow, Bounds, EntityId, LinkOrder, ModuleId, NativeEntityRef,
    ResourceNamespace, RuleSetId, ThinkTime, Vec3,
};
use qa_gameplay::rules::motion::toss_rules;
use qa_session::physics::{
    MotionState, MotionWrite, PhysicsError, TossFrame, TossHost, TossTrace, check_velocity, toss,
    water_transition,
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent},
    collision::{QuakeTraceKind, Trace},
    entities::{AllocationPolicy, EntityTable},
};

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
}

struct Host {
    entities: EntityTable,
    entity: EntityId,
    state: MotionState,
    scale: Option<f32>,
    trace: Option<TossTrace>,
    events: Vec<Event>,
    remove_on_link: bool,
    bounce_on_impact: bool,
    shifted_clip: bool,
    remove_after_write: Option<MotionWrite>,
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
        }
    }

    fn run(&mut self, rules: RuleSetId, frame: TossFrame) -> Result<(), PhysicsError> {
        let entity = self.entity;
        toss(self, entity, frame, &toss_rules(rules).unwrap())
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
        self.trace = Some(TossTrace {
            trace,
            ground: NativeEntityRef::native(ResourceNamespace::Module(ModuleId(3)), 2),
        });
    }
}

impl TossHost for Host {
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
            MotionWrite::Origin(axis) => self.entities.columns.position[slot].0[axis] = value,
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
    ) -> Result<TossTrace, Self::Error> {
        self.events.push(Event::Trace(kind, end));
        Ok(self.trace.unwrap_or(TossTrace {
            trace: Trace::clear(end),
            ground: None,
        }))
    }
    fn relink(&mut self, entity: EntityId) -> Result<Option<EntityId>, Self::Error> {
        self.events.push(Event::Link);
        if self.remove_on_link {
            assert!(self.entities.release(entity, ThinkTime::Seconds(1.0)));
        }
        Ok(self.entities.resolve(entity).map(|_| entity))
    }
    fn impact(
        &mut self,
        entity: Option<EntityId>,
        _: &TossTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        self.events.push(Event::Impact(entity));
        if self.bounce_on_impact {
            self.state.move_type = 10.0;
        }
        Ok(entity)
    }
    fn ground(&mut self, _: EntityId, ground: Option<NativeEntityRef>) -> Result<(), Self::Error> {
        self.events.push(Event::Ground(ground));
        Ok(())
    }
    fn water_transition(&mut self, _: EntityId) -> Result<(), Self::Error> {
        self.events.push(Event::Water);
        Ok(())
    }
}

const FRAME: TossFrame = TossFrame {
    elapsed: 0.013271937412,
    gravity: 800.0,
    max_velocity: 2000.0,
};

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
