//! QuakeC motion borrows native fields only between shared physics operations.
use super::{ClientCommandError, FrameHost, Program, contacts, edit_quakec, native_self};
use qa_compat::{
    quakec::{MotionField, Vm},
    services::{CallContext, CallError, EngineServices},
};
use qa_core::primitives::{EntityId, ModuleId, NativeEntityRef, ThinkTime, Vec3};
use qa_session::physics::{
    self, MotionState, MotionWrite, PhysicsError, TossFrame, TossHost, TossTrace,
};
use qa_world::{
    collision::{QuakeTraceKind, TraceQuery, trace_policy},
    entities::EntityTable,
};

impl From<PhysicsError> for ClientCommandError {
    fn from(error: PhysicsError) -> Self {
        Self::Usercmd(error.into())
    }
}

/// Attachment transport owns the common pose. Publish it before a callback
/// can import native fields over that pose. Each scalar write may refresh
/// overlapping fields, so retain only this row's already transported position.
pub(super) fn sync_attachments(
    vm: &mut Vm,
    entities: &mut EntityTable,
    context: CallContext,
) -> Result<(), CallError> {
    for index in 0..entities.attachment_ids().len() {
        let entity = entities.attachment_ids()[index];
        let Some(slot) = entities.resolve(entity) else {
            continue;
        };
        let Some(native) =
            entities.columns.native_entity[slot].filter(|native| native.module == context.module)
        else {
            continue;
        };
        let reference = NativeEntityRef::entity(
            qa_core::primitives::ResourceNamespace::Module(native.module),
            u32::try_from(native.slot).map_err(|_| CallError::Entity)?,
        );
        let offset = vm.weak_entity_offset(context.module, reference)?;
        let position = entities.columns.position[slot];
        for axis in 0..3 {
            let slot = entities.resolve(entity).ok_or(CallError::Entity)?;
            entities.columns.position[slot].0[axis] = position.0[axis];
            vm.write_motion(entities, context, offset, MotionWrite::Origin(axis))?;
        }
    }
    Ok(())
}

pub(super) fn noclip(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    step: f32,
) -> Result<(), ClientCommandError> {
    let native = native_offset(host, module, entity)?;
    edit_quakec(host, module, |vm, services, context| {
        vm.prepare_motion(&mut services.server.entities, context, native)?;
        physics::noclip(
            &mut services.server.entities,
            entity,
            step,
            |entities, field| vm.write_motion(entities, context, native, field),
        )?;
        vm.relink_entity(services, context, native)
    })
}

/// The same scalar check serves native PlayerPreThink and nonclient physics.
pub(super) fn check_velocity(
    vm: &mut Vm,
    services: &mut EngineServices<'_>,
    context: CallContext,
    native: u32,
    entity: EntityId,
    maximum: f32,
) -> Result<(), CallError> {
    vm.prepare_velocity(&mut services.server.entities, context, native)?;
    let events = &mut services.server.events;
    physics::check_velocity(
        &mut services.server.entities,
        entity,
        maximum,
        |entities, field, invalid| {
            if invalid {
                let prefix = match field {
                    MotionWrite::Velocity(_) => b"Got a NaN velocity on ".as_slice(),
                    _ => b"Got a NaN origin on ".as_slice(),
                };
                let _ = events.console_parts(&[prefix, vm.classname(native)?, b"\n"]);
            }
            vm.write_motion(entities, context, native, field)
        },
    )?;
    Ok(())
}

fn native_offset(
    host: &FrameHost,
    module: ModuleId,
    entity: EntityId,
) -> Result<u32, ClientCommandError> {
    let row = host
        .module_row(module)
        .ok_or(ClientCommandError::MissingModule)?;
    let Program::QuakeC(qc) = &row.request.program else {
        return Err(ClientCommandError::NeedsNativeModule);
    };
    native_self(&host.runtime, entity, module, &qc.vm).ok_or(ClientCommandError::InvalidSlot)
}

pub(super) fn toss(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
    elapsed: f64,
) -> Result<(), ClientCommandError> {
    let native = native_offset(host, module, entity)?;
    let mover = contacts::reference(host, entity)?;
    let rules = host
        .module_row(module)
        .and_then(|row| row.toss)
        .ok_or(ClientCommandError::Unsupported)?;
    let frame = edit_quakec(host, module, |vm, services, _| {
        Ok(TossFrame {
            elapsed,
            gravity: vm.motion_world_gravity(services.cvars)?,
            max_velocity: vm.max_velocity(services.cvars)?,
        })
    })?;
    physics::toss(
        &mut Host {
            host,
            module,
            native,
            mover,
            time,
        },
        entity,
        frame,
        &rules,
    )
}

struct Host<'a> {
    host: &'a mut FrameHost,
    module: ModuleId,
    native: u32,
    mover: NativeEntityRef,
    time: ThinkTime,
}
impl Host<'_> {
    fn current(&self) -> Result<Option<EntityId>, ClientCommandError> {
        contacts::current(self.host, self.mover)
    }
    fn edit<R>(
        &mut self,
        entity: EntityId,
        call: impl FnOnce(&mut Vm, &mut EngineServices<'_>, CallContext, u32) -> Result<R, CallError>,
    ) -> Result<R, ClientCommandError> {
        let native = native_offset(self.host, self.module, entity)?;
        if native != self.native {
            return Err(ClientCommandError::InvalidSlot);
        }
        edit_quakec(self.host, self.module, |vm, services, context| {
            call(vm, services, context, native)
        })
    }
}

impl TossHost for Host<'_> {
    type Error = ClientCommandError;

    fn entities(&mut self) -> &mut EntityTable {
        &mut self.host.runtime.server.entities
    }
    fn state(&mut self, entity: EntityId) -> Result<MotionState, Self::Error> {
        self.edit(entity, |vm, _, context, native| {
            vm.motion_state(context, native)
        })
    }
    fn prepare_motion(&mut self, entity: EntityId) -> Result<(), Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            vm.prepare_motion(&mut services.server.entities, context, native)
        })
    }
    fn check_velocity(&mut self, entity: EntityId, maximum: f32) -> Result<(), Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            check_velocity(vm, services, context, native, entity, maximum)
        })
    }
    fn write_motion(
        &mut self,
        entity: EntityId,
        field: MotionWrite,
        value: f32,
    ) -> Result<(), Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            let entities = &mut services.server.entities;
            let slot = entities.resolve(entity).ok_or(CallError::Entity)?;
            match field {
                MotionWrite::Angles(axis) => entities.columns.angles[slot].0[axis] = value,
                MotionWrite::AngularVelocity(axis) => {
                    entities.columns.angular_velocity[slot]
                        .as_mut()
                        .ok_or(CallError::Memory)?
                        .0[axis] = value;
                }
                MotionWrite::Origin(axis) => entities.columns.position[slot].0[axis] = value,
                MotionWrite::Velocity(axis) => entities.columns.velocity[slot].0[axis] = value,
            }
            vm.write_motion(entities, context, native, field)
        })
    }
    fn write_flags(&mut self, entity: EntityId, flags: u32) -> Result<(), Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::Flags(flags),
            )
        })
    }
    fn gravity_scale(&mut self, entity: EntityId) -> Result<Option<f32>, Self::Error> {
        self.edit(entity, |vm, _, context, native| {
            vm.motion_gravity(context, native)
        })
    }
    fn trace(
        &mut self,
        entity: EntityId,
        end: Vec3,
        kind: QuakeTraceKind,
    ) -> Result<TossTrace, Self::Error> {
        let anchor = self
            .host
            .module_row(self.module)
            .ok_or(ClientCommandError::MissingModule)?
            .request
            .anchor;
        let mut trace = self.edit(entity, |vm, services, context, native| {
            let slot = services
                .server
                .entities
                .resolve(entity)
                .ok_or(CallError::Entity)?;
            let body = services.server.entities.columns.body(slot);
            let (rules, entity_rules) = trace_policy(context.trace_rules);
            let mut query = TraceQuery::point(
                body.position,
                end,
                rules,
                entity_rules.with_quake_kind(Some(kind)),
            );
            query.mins = body.mins;
            query.maxs = body.maxs;
            vm.with_trace_view(context, native, |view, pass| {
                query.pass = pass;
                services.trace(query, Some(view))
            })
        })?;
        // A native world pointer names this module's explicit world binding,
        // independently of common reservation slots or the loaded map family.
        if trace.entity.is_some() && trace.entity == self.host.runtime.server.entities.id_at(0) {
            trace.entity = Some(anchor);
        }
        let ground = trace
            .entity
            .map(|hit| contacts::reference(self.host, hit))
            .transpose()?;
        Ok(TossTrace { trace, ground })
    }
    fn relink(&mut self, entity: EntityId) -> Result<Option<EntityId>, Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            vm.relink_entity(services, context, native)
        })?;
        contacts::triggers(self.host, self.module, entity, self.time)?;
        self.current()
    }
    fn impact(
        &mut self,
        _: Option<EntityId>,
        trace: &TossTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        let native = trace.ground.ok_or(ClientCommandError::InvalidSlot)?;
        contacts::impact(self.host, self.module, self.mover, native, self.time)?;
        self.current()
    }
    fn ground(
        &mut self,
        entity: EntityId,
        ground: Option<NativeEntityRef>,
    ) -> Result<(), Self::Error> {
        let ground = ground.ok_or(ClientCommandError::InvalidSlot)?;
        self.edit(entity, |vm, services, context, native| {
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::Ground(ground),
            )
        })
    }
    fn water_transition(&mut self, entity: EntityId) -> Result<(), Self::Error> {
        let queued = self.host.runtime.input_time;
        self.edit(entity, |vm, services, context, native| {
            let slot = services
                .server
                .entities
                .resolve(entity)
                .ok_or(CallError::Entity)?;
            let origin = services.server.entities.columns.position[slot];
            let (_, rules) = trace_policy(context.trace_rules);
            let contents = vm
                .with_trace_view(context, native, |view, pass| {
                    services.point_contents(origin, rules, pass, Some(view))
                })?
                .to_q1();
            let old = vm.motion_water_type(context, native)?;
            let transition = physics::water_transition(old, contents);
            if transition.splash {
                vm.start_sound(
                    services,
                    context,
                    queued,
                    native,
                    b"misc/h2ohit1.wav",
                    0,
                    255,
                    1.0,
                )?;
            }
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::WaterType(transition.water_type),
            )?;
            vm.write_motion_field(
                &mut services.server.entities,
                context,
                native,
                MotionField::WaterLevel(transition.water_level),
            )
        })
    }
}
