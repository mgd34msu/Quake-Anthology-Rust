//! QuakeC motion borrows native fields only between shared physics operations.
use super::{ClientCommandError, FrameHost, Program, contacts, edit_quakec, native_self};
use qa_compat::{
    quakec::{MotionField, Vm},
    services::{CallContext, CallError, EngineServices},
};
use qa_core::primitives::{
    CallbackCall, ClientId, CollisionOwner, EntityId, ModuleId, Motion, NativeEntity,
    NativeEntityRef, ResourceNamespace, RuleSetId, ThinkTime, UserCmd, Vec3,
};
use qa_session::dispatch::{CallOutcome, call_think_once};
use qa_session::physics::{
    self, ClientHost, MotionFrame, MotionHost, MotionState, MotionTrace, MotionWrite, PhysicsError,
    PushFrame, PushHost, PushNumber,
};
use qa_world::{
    collision::{QuakeTraceKind, TraceQuery, trace_policy},
    entities::{EntityReference, EntityTable},
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
    services: &mut EngineServices<'_>,
    context: CallContext,
) -> Result<(), CallError> {
    services.server.commit_attachments();
    for index in 0..services.server.entities.attachment_ids().len() {
        let entity = services.server.entities.attachment_ids()[index];
        let Some(slot) = services.server.entities.resolve(entity) else {
            continue;
        };
        let Some(native) = services.server.entities.columns.native_entity[slot]
            .filter(|native| native.module == context.module)
        else {
            continue;
        };
        let reference = NativeEntityRef::entity(
            qa_core::primitives::ResourceNamespace::Module(native.module),
            u32::try_from(native.slot).map_err(|_| CallError::Entity)?,
        );
        let offset = vm.weak_entity_offset(context.module, reference)?;
        let position = services.server.entities.columns.position[slot];
        for axis in 0..3 {
            let slot = services
                .server
                .entities
                .resolve(entity)
                .ok_or(CallError::Entity)?;
            services.server.entities.columns.position[slot].0[axis] = position.0[axis];
            vm.write_motion(
                &mut services.server.entities,
                context,
                offset,
                MotionWrite::Origin(axis),
            )?;
        }
        vm.publish_link_bounds(services, context, offset)?;
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

pub(super) fn client_control(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    start: qa_movement::ControlStart,
    time: ThinkTime,
    elapsed: f64,
) -> Result<(), ClientCommandError> {
    let mut host = Host::client(host, module, id, time)?;
    let entity = host.client_entity().ok_or(ClientCommandError::Inactive)?;
    let rules = qa_gameplay::rules::motion::client_motion_rules(RuleSetId::Quake)
        .ok_or(ClientCommandError::Unsupported)?;
    let frame = host.frame(elapsed)?;
    physics::client_control(&mut host, entity, start, frame, time, &rules)
}

pub(super) fn client_move(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    phase: Motion,
    time: ThinkTime,
    elapsed: f64,
) -> Result<(), ClientCommandError> {
    let mut host = Host::client(host, module, id, time)?;
    let entity = host.client_entity().ok_or(ClientCommandError::Inactive)?;
    let rules = qa_gameplay::rules::motion::client_motion_rules(RuleSetId::Quake)
        .ok_or(ClientCommandError::Unsupported)?;
    let frame = host.frame(elapsed)?;
    physics::client_move(&mut host, entity, phase, frame, &rules)
}

struct Host<'a> {
    host: &'a mut FrameHost,
    module: ModuleId,
    native: u32,
    mover: EntityReference,
    time: ThinkTime,
    client: Option<(ClientId, EntityId)>,
    trace_rules: Option<RuleSetId>,
}
impl<'a> Host<'a> {
    fn new(
        host: &'a mut FrameHost,
        module: ModuleId,
        entity: EntityId,
        time: ThinkTime,
    ) -> Result<Self, ClientCommandError> {
        let native = native_offset(host, module, entity)?;
        let mover = contacts::capture(host, entity)?;
        Ok(Self {
            host,
            module,
            native,
            mover,
            time,
            client: None,
            trace_rules: None,
        })
    }
    fn client(
        host: &'a mut FrameHost,
        module: ModuleId,
        id: ClientId,
        time: ThinkTime,
    ) -> Result<Self, ClientCommandError> {
        let client = host
            .runtime
            .server
            .clients
            .get(id.0 as usize)
            .filter(|client| client.connection.is_some() && client.module == module)
            .ok_or(ClientCommandError::Inactive)?;
        let entity = client.entity;
        let trace_rules = client.player.trace_rules;
        let mut result = Self::new(host, module, entity, time)?;
        result.client = Some((id, entity));
        result.trace_rules = Some(trace_rules);
        result.client_entity().ok_or(ClientCommandError::Inactive)?;
        Ok(result)
    }
    fn client_entity(&self) -> Option<EntityId> {
        let (id, captured) = self.client?;
        super::clients::quakec::same_client(self.host, self.module, id, captured)
            .then_some(captured)
    }

    fn frame(&mut self, elapsed: f64) -> Result<MotionFrame, ClientCommandError> {
        let mut frame = edit_quakec(self.host, self.module, |vm, services, _| {
            Ok(MotionFrame {
                elapsed,
                gravity: vm.motion_world_gravity(services.cvars)?,
                max_velocity: vm.max_velocity(services.cvars)?,
            })
        })?;
        if let Some((id, _)) = self.client {
            self.client_entity().ok_or(ClientCommandError::Inactive)?;
            let player = &self.host.runtime.server.clients[id.0 as usize].player;
            frame.gravity =
                qa_movement::effective_tuning(player.movement_rules, player.movement.tuning)
                    .gravity;
        }
        Ok(frame)
    }
    fn current(&self) -> Result<Option<EntityId>, ClientCommandError> {
        if self.client.is_some() {
            return Ok(self.client_entity());
        }
        Ok(self
            .host
            .runtime
            .server
            .entities
            .resolve_reference(self.mover))
    }
    fn edit<R>(
        &mut self,
        entity: EntityId,
        call: impl FnOnce(&mut Vm, &mut EngineServices<'_>, CallContext, u32) -> Result<R, CallError>,
    ) -> Result<R, ClientCommandError> {
        if self.client.is_some() && self.client_entity() != Some(entity) {
            return Err(ClientCommandError::Inactive);
        }
        let native = native_offset(self.host, self.module, entity)?;
        if native != self.native {
            return Err(ClientCommandError::InvalidSlot);
        }
        let trace_rules = self.trace_rules;
        edit_quakec(self.host, self.module, |vm, services, mut context| {
            if let Some(rules) = trace_rules {
                context.trace_rules = rules;
            }
            call(vm, services, context, native)
        })
    }

    fn write_field(
        &mut self,
        entity: EntityId,
        field: MotionField,
    ) -> Result<(), ClientCommandError> {
        self.edit(entity, |vm, services, context, native| {
            vm.write_motion_field(&mut services.server.entities, context, native, field)
        })
    }

    fn trace_between(
        &mut self,
        entity: EntityId,
        start: Vec3,
        end: Vec3,
        bounds: Option<(Vec3, Vec3)>,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, ClientCommandError> {
        let anchor = self
            .host
            .module_row(self.module)
            .ok_or(ClientCommandError::MissingModule)?
            .request
            .anchor;
        let mut trace = self.edit(entity, |vm, services, context, native| {
            let (rules, entity_rules) = trace_policy(context.trace_rules);
            let mut query =
                TraceQuery::point(start, end, rules, entity_rules.with_quake_kind(Some(kind)));
            if let Some((mins, maxs)) = bounds {
                query.mins = mins;
                query.maxs = maxs;
            }
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
            .map(|hit| contacts::capture(self.host, hit))
            .transpose()?;
        Ok(MotionTrace { trace, ground })
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
            sync_attachments(vm, services, context)
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
        let entity = match contacts::current(self.host, reference)? {
            Some(entity) => entity,
            None => self.edit_owner(owner, |vm, services, context, native| {
                vm.ensure_client_body(services, context, native)
            })?,
        };
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
        let binding = self.edit_owner(pusher, |vm, _, _, native| vm.blocked_binding(native))?;
        let Some(binding) = binding else {
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
            .resolve_binding(&self.host.runtime.server.entities, binding, entity)
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
        self.write_field(entity, MotionField::Flags(flags))
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
        let entities = &self.host.runtime.server.entities;
        let slot = entities
            .resolve(entity)
            .ok_or(ClientCommandError::Inactive)?;
        let body = entities.columns.body(slot);
        let trace = self.trace_between(
            entity,
            body.position,
            end,
            Some((body.mins, body.maxs)),
            kind,
        )?;
        Ok(trace)
    }
    fn relink(&mut self, entity: EntityId) -> Result<Option<EntityId>, Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            vm.relink_entity(services, context, native)?;
            sync_attachments(vm, services, context)
        })?;
        contacts::triggers(self.host, self.module, entity, self.time)?;
        self.current()
    }
    fn impact(
        &mut self,
        _: Option<EntityId>,
        trace: &MotionTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        let obstacle = trace.ground.ok_or(ClientCommandError::InvalidSlot)?;
        let mover_entity = self.current()?;
        contacts::impact(
            self.host,
            self.module,
            mover_entity,
            self.mover,
            obstacle,
            self.time,
        )?;
        self.current()
    }
    fn ground(
        &mut self,
        entity: EntityId,
        ground: Option<EntityReference>,
    ) -> Result<(), Self::Error> {
        let ground = ground.ok_or(ClientCommandError::InvalidSlot)?;
        self.write_field(entity, MotionField::Ground(ground))
    }
    fn water_transition(&mut self, entity: EntityId) -> Result<(), Self::Error> {
        let slot = self
            .host
            .runtime
            .server
            .entities
            .resolve(entity)
            .ok_or(ClientCommandError::Inactive)?;
        let origin = self.host.runtime.server.entities.columns.position[slot];
        let contents = ClientHost::point_contents(self, entity, origin)?;
        let previous = self.edit(entity, |vm, _, context, native| {
            vm.motion_water_type(context, native)
        })?;
        let transition = physics::water_transition(previous, contents);
        if transition.splash {
            self.sound(entity, b"misc/h2ohit1.wav")?;
        }
        self.write_field(entity, MotionField::WaterType(transition.water_type))?;
        self.write_field(entity, MotionField::WaterLevel(transition.water_level))
    }
    fn sound(&mut self, entity: EntityId, name: &[u8]) -> Result<(), Self::Error> {
        let queued = self.host.runtime.input_time;
        self.edit(entity, |vm, services, context, native| {
            vm.start_sound(services, context, queued, native, name, 0, 255, 1.0)
        })
    }
}

impl ClientHost for Host<'_> {
    fn command(&mut self, entity: EntityId) -> Result<UserCmd, Self::Error> {
        if self.client_entity() != Some(entity) {
            return Err(ClientCommandError::Inactive);
        }
        let (id, _) = self.client.ok_or(ClientCommandError::Inactive)?;
        Ok(self.host.runtime.server.clients[id.0 as usize].command)
    }
    fn view_angles(&mut self, entity: EntityId) -> Result<Vec3, Self::Error> {
        self.edit(entity, |vm, _, _, native| vm.client_view_angles(native))
    }
    fn view_height(&mut self, entity: EntityId) -> Result<f32, Self::Error> {
        self.edit(entity, |vm, _, _, native| vm.client_view_height(native))
    }
    fn water_level(&mut self, entity: EntityId) -> Result<f32, Self::Error> {
        self.edit(entity, |vm, _, _, native| vm.client_water_level(native))
    }
    fn write_water_level(&mut self, entity: EntityId, value: f32) -> Result<(), Self::Error> {
        self.write_field(entity, MotionField::WaterLevel(value))
    }
    fn write_water_type(&mut self, entity: EntityId, value: f32) -> Result<(), Self::Error> {
        self.write_field(entity, MotionField::WaterType(value))
    }
    fn old_origin(&mut self, entity: EntityId, axis: usize) -> Result<f32, Self::Error> {
        self.edit(entity, |vm, _, _, native| {
            vm.client_old_origin(native, axis)
        })
    }
    fn write_old_origin(
        &mut self,
        entity: EntityId,
        axis: usize,
        value: f32,
    ) -> Result<(), Self::Error> {
        self.write_field(entity, MotionField::OldOrigin { axis, value })
    }
    fn teleport_time(&mut self, entity: EntityId) -> Result<f32, Self::Error> {
        self.edit(entity, |vm, _, _, native| vm.client_teleport_time(native))
    }
    fn write_teleport_time(&mut self, entity: EntityId, value: f32) -> Result<(), Self::Error> {
        self.write_field(entity, MotionField::TeleportTime(value))
    }
    fn move_direction(&mut self, entity: EntityId, axis: usize) -> Result<f32, Self::Error> {
        self.edit(entity, |vm, _, _, native| {
            vm.client_move_direction(native, axis)
        })
    }
    fn point_contents(&mut self, entity: EntityId, point: Vec3) -> Result<i32, Self::Error> {
        self.edit(entity, |vm, services, context, native| {
            let (_, rules) = trace_policy(context.trace_rules);
            vm.with_trace_view(context, native, |view, pass| {
                services.point_contents(point, rules, pass, Some(view))
            })
            .map(|contents| contents.to_q1())
        })
    }
    fn trace_point(
        &mut self,
        entity: EntityId,
        start: Vec3,
        end: Vec3,
    ) -> Result<qa_world::collision::Trace, Self::Error> {
        self.trace_between(entity, start, end, None, QuakeTraceKind::IgnoreBoxes)
            .map(|result| result.trace)
    }
    fn tuning(&mut self, entity: EntityId) -> Result<qa_movement::ControlTuning, Self::Error> {
        if self.client_entity() != Some(entity) {
            return Err(ClientCommandError::Inactive);
        }
        let (id, _) = self.client.ok_or(ClientCommandError::Inactive)?;
        Ok(qa_movement::control_tuning(
            &self.host.runtime.server.clients[id.0 as usize].player,
        ))
    }
    fn diagnostic(&mut self, text: &'static [u8]) -> Result<(), Self::Error> {
        if self
            .host
            .console
            .cvars
            .value_in(self.host.developer, RuleSetId::Quake)
            != 0.0
        {
            let _ = self.host.runtime.server.events.console_bytes(text);
        }
        Ok(())
    }
    fn current(&mut self) -> Result<Option<EntityId>, Self::Error> {
        Host::current(self)
    }
}

#[cfg(test)]
#[path = "../../tests/modules/foreign_motion.rs"]
mod tests;
