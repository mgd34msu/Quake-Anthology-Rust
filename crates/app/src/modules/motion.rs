//! QuakeC motion borrows native fields only between shared physics operations.
use super::{ClientCommandError, FrameHost, Program, contacts, edit_quakec, native_self};
use qa_compat::{
    quakec::{MotionField, Vm},
    services::{CallContext, CallError, EngineServices},
};
use qa_core::primitives::{
    CallbackCall, CollisionOwner, EntityId, ModuleId, NativeEntity, NativeEntityRef,
    ResourceNamespace, ThinkTime, Vec3,
};
use qa_session::dispatch::{CallOutcome, call_think_once};
use qa_session::physics::{
    self, MotionFrame, MotionHost, MotionState, MotionTrace, MotionWrite, PhysicsError, PushFrame,
    PushHost, PushNumber,
};
use qa_world::{
    collision::{QuakeTraceKind, TraceQuery, trace_policy},
    entities::EntityTable,
};
use std::sync::Arc;

impl From<PhysicsError> for ClientCommandError {
    fn from(error: PhysicsError) -> Self {
        Self::Usercmd(error.into())
    }
}

/// Commit the shared attachment graph before a callback can import native
/// fields over its authoritative pose. Retain one transported row's position
/// across scalar stores, which may refresh overlapping native fields.
pub(super) fn sync_attachments(
    vm: &mut Vm,
    server: &mut qa_session::clients::Server,
    context: CallContext,
) -> Result<(), CallError> {
    server.commit_attachments();
    let entities = &mut server.entities;
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
    let rules = host
        .module_row(module)
        .and_then(|row| row.toss)
        .ok_or(ClientCommandError::Unsupported)?;
    let mut host = Host::new(host, module, entity, time)?;
    let frame = host.frame(elapsed)?;
    physics::toss(&mut host, entity, frame, &rules)
}

pub(super) fn push(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
    elapsed: f64,
) -> Result<(), ClientCommandError> {
    let (anchor, rules, mut undo) = {
        let row = host
            .module_row_mut(module)
            .ok_or(ClientCommandError::MissingModule)?;
        let rules = row.push.ok_or(ClientCommandError::Unsupported)?;
        let undo = row.push_undo.take().ok_or(ClientCommandError::Inactive)?;
        (row.request.anchor, rules, undo)
    };
    let result = (|| {
        let native = host.runtime.server.entities.columns.native_entity[host
            .runtime
            .server
            .entities
            .resolve(entity)
            .ok_or(ClientCommandError::Inactive)?]
        .ok_or(ClientCommandError::InvalidSlot)?;
        let mut adapter = Host::new(host, module, entity, time)?;
        physics::push_frame(
            &mut adapter,
            CollisionOwner::Native(native),
            &mut undo,
            PushFrame {
                elapsed,
                world_time: time,
            },
            &rules,
        )
    })();
    let row = host
        .module_row_mut(module)
        .filter(|row| row.request.anchor == anchor)
        .ok_or(ClientCommandError::MissingModule)?;
    row.push_undo = Some(undo);
    result
}

pub(super) fn step_fall(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
    elapsed: f64,
) -> Result<Option<EntityId>, ClientCommandError> {
    let (rules, sweep) = host
        .module_row(module)
        .and_then(|row| row.step)
        .ok_or(ClientCommandError::Unsupported)?;
    let mut host = Host::new(host, module, entity, time)?;
    let frame = host.frame(elapsed)?;
    physics::step_fall(&mut host, entity, frame, &rules, &sweep)
}

pub(super) fn water_transition(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
) -> Result<(), ClientCommandError> {
    Host::new(host, module, entity, time)?.water_transition(entity)
}

struct Host<'a> {
    host: &'a mut FrameHost,
    module: ModuleId,
    native: u32,
    mover: NativeEntityRef,
    time: ThinkTime,
}
impl<'a> Host<'a> {
    fn new(
        host: &'a mut FrameHost,
        module: ModuleId,
        entity: EntityId,
        time: ThinkTime,
    ) -> Result<Self, ClientCommandError> {
        let native = native_offset(host, module, entity)?;
        let mover = contacts::reference(host, entity)?;
        Ok(Self {
            host,
            module,
            native,
            mover,
            time,
        })
    }
    fn frame(&mut self, elapsed: f64) -> Result<MotionFrame, ClientCommandError> {
        edit_quakec(self.host, self.module, |vm, services, _| {
            Ok(MotionFrame {
                elapsed,
                gravity: vm.motion_world_gravity(services.cvars)?,
                max_velocity: vm.max_velocity(services.cvars)?,
            })
        })
    }
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

    fn reference(&self, owner: CollisionOwner) -> Result<NativeEntityRef, ClientCommandError> {
        let native = match owner {
            CollisionOwner::Native(native) => native,
            CollisionOwner::Lifetime(entity) => {
                let entities = &self.host.runtime.server.entities;
                let slot = entities
                    .resolve(entity)
                    .ok_or(ClientCommandError::Inactive)?;
                entities.columns.native_entity[slot].ok_or(ClientCommandError::InvalidSlot)?
            }
            CollisionOwner::None => return Err(ClientCommandError::InvalidSlot),
        };
        if native.module != self.module {
            return Err(ClientCommandError::DifferentModule);
        }
        Ok(NativeEntityRef::entity(
            ResourceNamespace::Module(native.module),
            u32::try_from(native.slot).map_err(|_| ClientCommandError::InvalidSlot)?,
        ))
    }

    fn edit_owner<R>(
        &mut self,
        owner: CollisionOwner,
        call: impl FnOnce(&mut Vm, &mut EngineServices<'_>, CallContext, u32) -> Result<R, CallError>,
    ) -> Result<R, ClientCommandError> {
        let reference = self.reference(owner)?;
        edit_quakec(self.host, self.module, |vm, services, context| {
            let native = vm.weak_entity_offset(context.module, reference)?;
            call(vm, services, context, native)
        })
    }
}

impl PushHost for Host<'_> {
    type Error = ClientCommandError;

    fn next_candidate(
        &mut self,
        cursor: &mut usize,
    ) -> Result<Option<CollisionOwner>, Self::Error> {
        edit_quakec(self.host, self.module, |vm, _, _| {
            vm.push_candidate(cursor)
                .map(|native| native.map(CollisionOwner::Native))
        })
    }

    fn free(&mut self, owner: CollisionOwner) -> Result<bool, Self::Error> {
        let reference = self.reference(owner)?;
        let native = NativeEntity {
            module: self.module,
            slot: i32::try_from(reference.ordinal).map_err(|_| ClientCommandError::InvalidSlot)?,
        };
        edit_quakec(self.host, self.module, |vm, services, _| {
            vm.push_free(&services.server.entities, native)
        })
    }

    fn number(&mut self, owner: CollisionOwner, field: PushNumber) -> Result<f32, Self::Error> {
        self.edit_owner(owner, |vm, _, _, native| vm.push_number(native, field))
    }

    fn store(
        &mut self,
        owner: CollisionOwner,
        field: PushNumber,
        value: f32,
    ) -> Result<(), Self::Error> {
        self.edit_owner(owner, |vm, services, context, native| {
            vm.push_store(&mut services.server.entities, context, native, field, value)
        })
    }

    fn flags(&mut self, owner: CollisionOwner) -> Result<u32, Self::Error> {
        self.edit_owner(owner, |vm, _, _, native| vm.push_flags(native))
    }

    fn store_flags(&mut self, owner: CollisionOwner, flags: u32) -> Result<(), Self::Error> {
        self.edit_owner(owner, |vm, services, context, native| {
            vm.push_store_flags(&mut services.server.entities, context, native, flags)
        })
    }

    fn ground_owner(&mut self, owner: CollisionOwner) -> Result<CollisionOwner, Self::Error> {
        let ground = self.edit_owner(owner, |vm, _, context, native| {
            vm.push_ground(context, native)
        })?;
        let ResourceNamespace::Module(module) = ground.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        Ok(CollisionOwner::Native(NativeEntity {
            module,
            slot: i32::try_from(ground.ordinal).map_err(|_| ClientCommandError::InvalidSlot)?,
        }))
    }

    fn test_position(&mut self, owner: CollisionOwner) -> Result<bool, Self::Error> {
        self.edit_owner(owner, |vm, services, context, native| {
            vm.push_test_position(services, context, native)
        })
    }

    fn link(&mut self, owner: CollisionOwner, touch: bool) -> Result<(), Self::Error> {
        if PushHost::free(self, owner)? {
            return Ok(());
        }
        let reference = self.reference(owner)?;
        self.edit_owner(owner, |vm, services, context, native| {
            vm.relink_entity(services, context, native)?;
            sync_attachments(vm, services.server, context)
        })?;
        if touch && let Some(entity) = contacts::current(self.host, reference)? {
            contacts::triggers(self.host, self.module, entity, self.time)?;
        }
        Ok(())
    }

    fn push_entity(
        &mut self,
        owner: CollisionOwner,
        displacement: Vec3,
    ) -> Result<(), Self::Error> {
        let reference = self.reference(owner)?;
        let entity =
            contacts::current(self.host, reference)?.ok_or(ClientCommandError::Inactive)?;
        let mut moving = Host::new(self.host, self.module, entity, self.time)?;
        physics::push_entity(&mut moving, entity, displacement).map(|_| ())
    }

    fn blocked(
        &mut self,
        pusher: CollisionOwner,
        blocker: CollisionOwner,
    ) -> Result<(), Self::Error> {
        let subject = self.reference(pusher)?;
        let other = self.reference(blocker)?;
        let callback = self.edit_owner(pusher, |vm, _, _, native| vm.blocked_callback(native))?;
        let Some(callback) = callback else {
            return Ok(());
        };
        let anchor = self
            .host
            .module_row(self.module)
            .ok_or(ClientCommandError::MissingModule)?
            .request
            .anchor;
        let entity = contacts::current(self.host, subject)?.unwrap_or(anchor);
        let functions = self
            .host
            .modules
            .as_ref()
            .map(|modules| Arc::clone(&modules.functions))
            .ok_or(ClientCommandError::MissingModule)?;
        let resolved = functions
            .resolve(&self.host.runtime.server.entities, callback, entity)
            .map_err(ClientCommandError::Dispatch)?;
        let call = CallbackCall::Blocked {
            entity,
            subject,
            other,
        };
        let accepted = functions
            .dispatch_resolved(self.host, resolved, call)
            .map_err(ClientCommandError::Dispatch)?
            .complete(self.host, call);
        accepted.then_some(()).ok_or(ClientCommandError::Dispatch(
            qa_session::dispatch::CallError::Rejected,
        ))
    }

    fn think_once(&mut self, pusher: CollisionOwner, time: ThinkTime) -> Result<(), Self::Error> {
        let subject = self.reference(pusher)?;
        let anchor = self
            .host
            .module_row(self.module)
            .ok_or(ClientCommandError::MissingModule)?
            .request
            .anchor;
        let current = contacts::current(self.host, subject)?;
        let entity = current.unwrap_or(anchor);
        let functions = self
            .host
            .modules
            .as_ref()
            .map(|modules| Arc::clone(&modules.functions))
            .ok_or(ClientCommandError::MissingModule)?;
        let result = call_think_once(self.host, &functions, entity, Some(subject), time, |host| {
            contacts::current(host, subject).ok().flatten()
        });
        (result.rejected == 0)
            .then_some(())
            .ok_or(ClientCommandError::Dispatch(
                qa_session::dispatch::CallError::Rejected,
            ))
    }
}

impl MotionHost for Host<'_> {
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
    ) -> Result<MotionTrace, Self::Error> {
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
        Ok(MotionTrace { trace, ground })
    }
    fn relink(&mut self, entity: EntityId) -> Result<Option<EntityId>, Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            vm.relink_entity(services, context, native)?;
            sync_attachments(vm, services.server, context)
        })?;
        contacts::triggers(self.host, self.module, entity, self.time)?;
        self.current()
    }
    fn impact(
        &mut self,
        _: Option<EntityId>,
        trace: &MotionTrace,
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
        let transition = self.edit(entity, |vm, services, context, native| {
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
            Ok(physics::water_transition(old, contents))
        })?;
        if transition.splash {
            self.sound(entity, b"misc/h2ohit1.wav")?;
        }
        self.edit(entity, |vm, services, context, native| {
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
    fn sound(&mut self, entity: EntityId, name: &[u8]) -> Result<(), Self::Error> {
        let queued = self.host.runtime.input_time;
        self.edit(entity, |vm, services, context, native| {
            vm.start_sound(services, context, queued, native, name, 0, 255, 1.0)
        })
    }
}
