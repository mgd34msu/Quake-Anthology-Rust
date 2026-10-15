use qa_core::{
    math,
    primitives::{EntityId, ModuleId, NativeEntityRef, RuleSetId, ThinkTime, UserCmd, Vec3},
};
use qa_gameplay::rules::motion::client_motion_rules;
use qa_movement::ControlStart;
use qa_movement::ControlTuning;
use qa_session::physics::{
    ClientHost, MotionFrame, MotionHost, MotionState, MotionTrace, MotionWrite, PhysicsError,
    client_control,
};
use qa_world::{
    collision::{QuakeTraceKind, Trace},
    entities::{AllocationPolicy, EntityTable},
};

#[derive(Debug, PartialEq)]
enum Event {
    Flags(u32),
    Deadline(f32),
    Direction(usize),
    Velocity(usize, f32),
    Probe(Vec3, Vec3),
}

struct Host {
    entities: EntityTable,
    entity: EntityId,
    state: MotionState,
    command: UserCmd,
    level: f32,
    deadline: f32,
    direction: Vec3,
    tuning: ControlTuning,
    events: Vec<Event>,
    alias_velocity: bool,
    alias_direction: bool,
    fail_axis: Option<usize>,
}

impl Host {
    fn new(mode: f32) -> Self {
        let mut entities = EntityTable::new(8, 1).unwrap();
        let entity = entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(1),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        entities.columns.mins[entity.slot as usize] = Vec3([0.0, 0.0, -24.0]);
        Self {
            entities,
            entity,
            state: MotionState {
                move_type: mode,
                flags: Some(0),
                solid: Some(2.0),
            },
            command: UserCmd {
                units: qa_gameplay::rules::command_units(RuleSetId::Quake),
                ..Default::default()
            },
            level: 0.0,
            deadline: 0.0,
            direction: Vec3::default(),
            tuning: ControlTuning {
                max_speed: 320.0,
                stop_speed: 0.0,
                friction: 1.0,
                accelerate: 10.0,
                edge_friction: 3.0,
                no_step: false,
            },
            events: Vec::new(),
            alias_velocity: false,
            alias_direction: false,
            fail_axis: None,
        }
    }

    fn run(&mut self, on_ground: bool) -> Result<(), PhysicsError> {
        let entity = self.entity;
        client_control(
            self,
            entity,
            ControlStart {
                on_ground,
                body_angles: Vec3::default(),
            },
            MotionFrame {
                elapsed: 0.1,
                gravity: 800.0,
                max_velocity: 2000.0,
            },
            ThinkTime::Seconds(2.0),
            &client_motion_rules(RuleSetId::Quake).unwrap(),
        )
    }

    fn velocity(&self) -> Vec3 {
        self.entities.columns.velocity[self.entity.slot as usize]
    }
}

impl MotionHost for Host {
    type Error = PhysicsError;
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
    fn state(&mut self, _: EntityId) -> Result<MotionState, Self::Error> {
        Ok(self.state)
    }
    fn prepare_motion(&mut self, _: EntityId) -> Result<(), Self::Error> {
        Ok(())
    }
    fn check_velocity(&mut self, _: EntityId, _: f32) -> Result<(), Self::Error> {
        panic!("control has no velocity sanitizer")
    }
    fn write_motion(
        &mut self,
        entity: EntityId,
        field: MotionWrite,
        value: f32,
    ) -> Result<(), Self::Error> {
        let MotionWrite::Velocity(axis) = field else {
            panic!("control writes only velocity")
        };
        if self.fail_axis == Some(axis) {
            return Err(PhysicsError::MissingField);
        }
        let slot = self.entities.resolve(entity).ok_or(PhysicsError::Entity)?;
        self.entities.columns.velocity[slot].0[axis] = value;
        if axis == 0 && self.alias_velocity {
            self.entities.columns.velocity[slot].0[1] = value;
        }
        if axis == 0 && self.alias_direction {
            self.direction.0[1] = value;
        }
        self.events.push(Event::Velocity(axis, value));
        Ok(())
    }
    fn write_flags(&mut self, _: EntityId, flags: u32) -> Result<(), Self::Error> {
        self.state.flags = Some(flags);
        self.events.push(Event::Flags(flags));
        Ok(())
    }
    fn gravity_scale(&mut self, _: EntityId) -> Result<Option<f32>, Self::Error> {
        panic!("control has no gravity")
    }
    fn trace(
        &mut self,
        _: EntityId,
        _: Vec3,
        _: QuakeTraceKind,
    ) -> Result<MotionTrace, Self::Error> {
        panic!("control uses a point probe")
    }
    fn relink(&mut self, _: EntityId) -> Result<Option<EntityId>, Self::Error> {
        panic!("control does not relink")
    }
    fn impact(
        &mut self,
        _: Option<EntityId>,
        _: &MotionTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        panic!("control has no impact")
    }
    fn ground(&mut self, _: EntityId, _: Option<NativeEntityRef>) -> Result<(), Self::Error> {
        panic!("control does not assign ground")
    }
    fn sound(&mut self, _: EntityId, _: &[u8]) -> Result<(), Self::Error> {
        panic!("QC owns action sounds")
    }
    fn water_transition(&mut self, _: EntityId) -> Result<(), Self::Error> {
        panic!("control does not transition water")
    }
}

impl ClientHost for Host {
    fn command(&mut self, _: EntityId) -> Result<UserCmd, Self::Error> {
        Ok(self.command)
    }
    fn view_angles(&mut self, _: EntityId) -> Result<Vec3, Self::Error> {
        Ok(Vec3::default())
    }
    fn view_height(&mut self, _: EntityId) -> Result<f32, Self::Error> {
        panic!("unused view height")
    }
    fn water_level(&mut self, _: EntityId) -> Result<f32, Self::Error> {
        Ok(self.level)
    }
    fn write_water_level(&mut self, _: EntityId, _: f32) -> Result<(), Self::Error> {
        panic!("control reads water")
    }
    fn write_water_type(&mut self, _: EntityId, _: f32) -> Result<(), Self::Error> {
        panic!("control reads water")
    }
    fn old_origin(&mut self, _: EntityId, _: usize) -> Result<f32, Self::Error> {
        panic!("unused oldorigin")
    }
    fn write_old_origin(&mut self, _: EntityId, _: usize, _: f32) -> Result<(), Self::Error> {
        panic!("unused oldorigin")
    }
    fn teleport_time(&mut self, _: EntityId) -> Result<f32, Self::Error> {
        Ok(self.deadline)
    }
    fn write_teleport_time(&mut self, _: EntityId, value: f32) -> Result<(), Self::Error> {
        self.deadline = value;
        self.events.push(Event::Deadline(value));
        Ok(())
    }
    fn move_direction(&mut self, _: EntityId, axis: usize) -> Result<f32, Self::Error> {
        self.events.push(Event::Direction(axis));
        Ok(self.direction.0[axis])
    }
    fn point_contents(&mut self, _: EntityId, _: Vec3) -> Result<i32, Self::Error> {
        panic!("unused point contents")
    }
    fn trace_point(&mut self, _: EntityId, start: Vec3, end: Vec3) -> Result<Trace, Self::Error> {
        self.events.push(Event::Probe(start, end));
        Ok(Trace::clear(end))
    }
    fn tuning(&mut self, _: EntityId) -> Result<ControlTuning, Self::Error> {
        Ok(self.tuning)
    }
    fn diagnostic(&mut self, _: &'static [u8]) -> Result<(), Self::Error> {
        panic!("no diagnostic expected")
    }
    fn current(&mut self) -> Result<Option<EntityId>, Self::Error> {
        Ok(Some(self.entity))
    }
}

#[test]
fn waterjump_clears_flags_and_deadline_before_live_movedir_scalar_reads() {
    let mut host = Host::new(3.0);
    host.state.flags = Some(512 | 2048);
    host.deadline = 1.0;
    host.direction = Vec3([11.0, 22.0, 0.0]);
    host.alias_direction = true;
    host.entities.columns.velocity[host.entity.slot as usize] = Vec3([0.0, 0.0, 7.0]);
    host.run(false).unwrap();
    assert_eq!(host.velocity(), Vec3([11.0, 11.0, 7.0]));
    assert_eq!(
        host.events,
        [
            Event::Flags(512),
            Event::Deadline(0.0),
            Event::Direction(0),
            Event::Velocity(0, 11.0),
            Event::Direction(1),
            Event::Velocity(1, 11.0),
        ]
    );
}

#[test]
fn captured_onground_survives_changed_flags_and_friction_rereads_aliases() {
    let mut host = Host::new(3.0);
    host.entities.columns.velocity[host.entity.slot as usize] = Vec3([100.0, 0.0, 30.0]);
    host.alias_velocity = true;
    host.run(true).unwrap();
    assert_eq!(host.velocity(), Vec3([70.0, 49.0, 21.0]));
    assert_eq!(
        host.events[0],
        Event::Probe(Vec3([16.0, 0.0, -24.0]), Vec3([16.0, 0.0, -58.0]))
    );
}

#[test]
fn rejected_friction_store_retains_the_successful_scalar_prefix() {
    let mut host = Host::new(3.0);
    host.entities.columns.velocity[host.entity.slot as usize] = Vec3([100.0, 0.0, 30.0]);
    host.alias_velocity = true;
    host.fail_axis = Some(1);
    assert_eq!(host.run(true), Err(PhysicsError::MissingField));
    assert_eq!(host.velocity(), Vec3([70.0, 70.0, 30.0]));
    assert_eq!(host.events.len(), 2);
}

#[test]
fn water_acceleration_keeps_cached_newspeed_but_rereads_velocity_per_store() {
    let mut host = Host::new(3.0);
    host.level = 2.0;
    host.command.movement = [300.0, 0.0, 0.0];
    host.entities.columns.velocity[host.entity.slot as usize] = Vec3([100.0, 0.0, 0.0]);
    host.alias_velocity = true;
    host.run(false).unwrap();
    assert_eq!(host.velocity(), Vec3([210.0, 210.0, 0.0]));
}

#[test]
fn noclip_requires_the_exact_raw_mode_while_walk_vertical_intent_uses_cast() {
    let mut exact = Host::new(8.0);
    exact.command.movement = [100.0, 0.0, 20.0];
    exact.run(false).unwrap();
    assert_eq!(exact.velocity(), Vec3([100.0, 0.0, 20.0]));
    let mut fractional = Host::new(8.5);
    fractional.command = exact.command;
    fractional.run(false).unwrap();
    assert!(math::length(fractional.velocity()) < 31.0);
    let mut walking = Host::new(3.5);
    walking.command = exact.command;
    walking.run(false).unwrap();
    assert_eq!(walking.velocity(), Vec3([30.0, 0.0, 0.0]));
}
