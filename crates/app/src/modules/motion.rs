//! QuakeC motion borrows native fields only between shared physics operations.
use super::{ClientCommandError, FrameHost, Program, State, contacts, edit_quakec, native_self};
use qa_compat::{
    quakec::{MotionField, Vm},
    services::{CallContext, CallError, EngineServices},
};
use qa_core::primitives::{
    CallbackCall, ClientId, CollisionOwner, EntityId, ModuleId, Motion, NativeEntity,
    NativeEntityRef, ResourceNamespace, RuleSetId, ThinkTime, UserCmd, Vec3,
};
use qa_session::dispatch::call_think_once_with_completion;
use qa_session::physics::{
    self, ClientHost, MotionFrame, MotionHost, MotionState, MotionTrace, MotionWrite, PhysicsError,
    PushFrame, PushHost, PushNumber,
};
use qa_world::{
    collision::{QuakeTraceKind, TraceQuery, trace_policy},
    entities::{EntityReference, EntityTable},
};
use std::sync::Arc;

mod projected;

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
    services.sync_attachment_positions(context.module, |services, entity, native, position| {
        let reference = NativeEntityRef::entity(
            ResourceNamespace::Module(native.module),
            u32::try_from(native.slot).map_err(|_| CallError::Entity)?,
        );
        let offset = vm.weak_entity_offset(context.module, reference)?;
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
        vm.publish_link_bounds(services, context, offset)
    })
}

pub(super) fn noclip(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    step: f32,
) -> Result<(), ClientCommandError> {
    let native = native_offset(host, module, entity)?;
    edit_quakec(
        host,
        module,
        crate::modules::execution::TraceAccess::empty(),
        |vm, services, context| {
            vm.prepare_motion(&mut services.server.entities, context, native)?;
            physics::noclip(
                &mut services.server.entities,
                entity,
                step,
                |entities, field| vm.write_motion(entities, context, native, field),
            )?;
            vm.relink_entity(services, context, native)
        },
    )
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
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let rules = host
        .module_row(module)
        .and_then(|row| row.toss)
        .ok_or(ClientCommandError::Unsupported)?;
    let mut host = Host::new(host, module, entity, time, driver)?;
    let frame = host.frame(elapsed)?;
    physics::toss(&mut host, entity, frame, &rules)
}

pub(super) fn push(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
    elapsed: f64,
    driver: &mut crate::modules::Driver<'_>,
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
        let mut adapter = Host::new(host, module, entity, time, driver)?;
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
    driver: &mut crate::modules::Driver<'_>,
) -> Result<Option<EntityId>, ClientCommandError> {
    let (rules, sweep) = host
        .module_row(module)
        .and_then(|row| row.step)
        .ok_or(ClientCommandError::Unsupported)?;
    let mut host = Host::new(host, module, entity, time, driver)?;
    let frame = host.frame(elapsed)?;
    physics::step_fall(&mut host, entity, frame, &rules, &sweep)
}

pub(super) fn water_transition(
    host: &mut FrameHost,
    module: ModuleId,
    entity: EntityId,
    time: ThinkTime,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    Host::new(host, module, entity, time, driver)?.water_transition(entity)
}

pub(super) fn client_control(
    host: &mut FrameHost,
    module: ModuleId,
    id: ClientId,
    start: qa_movement::ControlStart,
    time: ThinkTime,
    elapsed: f64,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let mut host = Host::client(host, module, id, time, driver)?;
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
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let mut host = Host::client(host, module, id, time, driver)?;
    let entity = host.client_entity().ok_or(ClientCommandError::Inactive)?;
    let rules = qa_gameplay::rules::motion::client_motion_rules(RuleSetId::Quake)
        .ok_or(ClientCommandError::Unsupported)?;
    let frame = host.frame(elapsed)?;
    physics::client_move(&mut host, entity, phase, frame, &rules)
}

enum PushCursor {
    Native(usize),
    Foreign(usize),
    Done,
}

struct Host<'a, 'd> {
    host: &'a mut FrameHost,
    driver: &'a mut crate::modules::Driver<'d>,
    module: ModuleId,
    native: u32,
    mover: EntityReference,
    time: ThinkTime,
    client: Option<(ClientId, EntityId)>,
    trace_rules: Option<RuleSetId>,
}
impl<'a, 'd> Host<'a, 'd> {
    fn new(
        host: &'a mut FrameHost,
        module: ModuleId,
        entity: EntityId,
        time: ThinkTime,
        driver: &'a mut crate::modules::Driver<'d>,
    ) -> Result<Self, ClientCommandError> {
        let native = native_offset(host, module, entity)?;
        let mover = contacts::capture(host, entity)?;
        Ok(Self {
            host,
            driver,
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
        driver: &'a mut crate::modules::Driver<'d>,
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
        let mut result = Self::new(host, module, entity, time, driver)?;
        result.client = Some((id, entity));
        result.trace_rules = Some(trace_rules);
        result.client_entity().ok_or(ClientCommandError::Inactive)?;
        Ok(result)
    }
    fn active_client(&self, module: ModuleId, entity: EntityId) -> Option<ClientId> {
        let index = self.host.runtime.server.entities.client_index(entity)?;
        let id = ClientId(index as u32);
        super::clients::quakec::same_client(self.host, module, id, entity).then_some(id)
    }

    fn owner_time(&self, module: ModuleId) -> Result<ThinkTime, ClientCommandError> {
        let row = self
            .host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?;
        let entities = &self.host.runtime.server.entities;
        if row.state != State::Running
            || entities
                .resolve(row.request.anchor)
                .is_none_or(|slot| entities.columns.owner[slot] != module)
        {
            return Err(ClientCommandError::Inactive);
        }
        Ok(row.time(self.host.time))
    }

    fn push_rules(&self) -> Result<qa_gameplay::rules::motion::PushRules, ClientCommandError> {
        self.host
            .module_row(self.module)
            .and_then(|row| row.push)
            .ok_or(ClientCommandError::Unsupported)
    }

    fn client_entity(&self) -> Option<EntityId> {
        let (id, captured) = self.client?;
        super::clients::quakec::same_client(self.host, self.module, id, captured)
            .then_some(captured)
    }

    fn frame(&mut self, elapsed: f64) -> Result<MotionFrame, ClientCommandError> {
        let mut frame = edit_quakec(
            self.host,
            self.module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, _| {
                Ok(MotionFrame {
                    elapsed,
                    gravity: vm.motion_world_gravity(services.cvars)?,
                    max_velocity: vm.max_velocity(services.cvars)?,
                })
            },
        )?;
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
        edit_quakec(
            self.host,
            self.module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, mut context| {
                if let Some(rules) = trace_rules {
                    context.trace_rules = rules;
                }
                call(vm, services, context, native)
            },
        )
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
            vm.with_trace_view(context, native, services.native_geometry, |view, pass| {
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

    fn current_owner(&self, owner: CollisionOwner) -> Result<Option<EntityId>, ClientCommandError> {
        if let CollisionOwner::Lifetime(entity) = owner {
            return Ok(self
                .host
                .runtime
                .server
                .entities
                .resolve(entity)
                .map(|_| entity));
        }
        let reference = self.reference(owner)?;
        let ResourceNamespace::Module(module) = reference.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        let row = self
            .host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?;
        row.request
            .program
            .bound_entity(
                &self.host.runtime.server.entities,
                NativeEntity {
                    module,
                    slot: i32::try_from(reference.ordinal)
                        .map_err(|_| ClientCommandError::InvalidSlot)?,
                },
            )
            .map_err(ClientCommandError::Usercmd)
    }

    fn edit_owner<R>(
        &mut self,
        owner: CollisionOwner,
        call: impl FnOnce(&mut Vm, &mut EngineServices<'_>, CallContext, u32) -> Result<R, CallError>,
    ) -> Result<R, ClientCommandError> {
        let reference = self.reference(owner)?;
        let ResourceNamespace::Module(module) = reference.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        self.owner_time(module)?;
        edit_quakec(
            self.host,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                let native = vm.weak_entity_offset(context.module, reference)?;
                call(vm, services, context, native)
            },
        )
    }
}

impl PushHost for Host<'_, '_> {
    type Error = ClientCommandError;
    type Cursor = PushCursor;

    fn start_candidates(&self) -> Self::Cursor {
        PushCursor::Native(1)
    }

    fn next_candidate(
        &mut self,
        cursor: &mut Self::Cursor,
    ) -> Result<Option<CollisionOwner>, Self::Error> {
        loop {
            match cursor {
                PushCursor::Native(next) => {
                    let candidate = edit_quakec(
                        self.host,
                        self.module,
                        crate::modules::execution::TraceAccess::empty(),
                        |vm, _, _| vm.push_candidate(next),
                    )?;
                    if let Some(native) = candidate {
                        return Ok(Some(CollisionOwner::Native(native)));
                    }
                    *cursor = PushCursor::Foreign(1);
                }
                PushCursor::Foreign(next) => {
                    let entities = &self.host.runtime.server.entities;
                    let Some(entity) = entities.next_active(*next) else {
                        *cursor = PushCursor::Done;
                        return Ok(None);
                    };
                    *next = entity.slot as usize + 1;
                    let slot = entity.slot as usize;
                    let Some(native) = entities.columns.native_entity[slot] else {
                        let owner = CollisionOwner::Lifetime(entity);
                        if projected::candidate(self.host, owner) {
                            return Ok(Some(owner));
                        }
                        continue;
                    };
                    if native.module == self.module
                        || native.slot < 0
                        || entities.columns.owner[slot] != native.module
                    {
                        continue;
                    }
                    let Some(row) = self.host.module_row(native.module) else {
                        continue;
                    };
                    if row.state != State::Running
                        || entities
                            .resolve(row.request.anchor)
                            .is_none_or(|anchor| entities.columns.owner[anchor] != native.module)
                    {
                        continue;
                    }
                    if matches!(&row.request.program, Program::QuakeC(_)) {
                        if native.slot == 0 {
                            continue;
                        }
                    } else if !projected::candidate(self.host, CollisionOwner::Native(native)) {
                        continue;
                    }
                    if row
                        .request
                        .program
                        .bound_entity(entities, native)
                        .map_err(ClientCommandError::Usercmd)?
                        == Some(entity)
                    {
                        return Ok(Some(CollisionOwner::Native(native)));
                    }
                }
                PushCursor::Done => return Ok(None),
            }
        }
    }

    fn capture_blocker(
        &self,
        owner: CollisionOwner,
    ) -> Result<Option<EntityReference>, Self::Error> {
        self.current_owner(owner)?
            .map(|entity| contacts::capture(self.host, entity))
            .transpose()
    }

    fn free(&mut self, owner: CollisionOwner) -> Result<bool, Self::Error> {
        if projected::handles(self.host, owner) {
            return Ok(projected::client(self.host, owner).is_none());
        }
        let reference = self.reference(owner)?;
        let ResourceNamespace::Module(module) = reference.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        let native = NativeEntity {
            module,
            slot: i32::try_from(reference.ordinal).map_err(|_| ClientCommandError::InvalidSlot)?,
        };
        self.edit_owner(owner, |vm, services, _, _| {
            vm.push_free(&services.server.entities, native)
        })
    }

    fn number(&mut self, owner: CollisionOwner, field: PushNumber) -> Result<f32, Self::Error> {
        if projected::handles(self.host, owner) {
            return projected::number(self.host, owner, field, self.push_rules()?);
        }
        self.edit_owner(owner, |vm, _, _, native| vm.push_number(native, field))
    }

    fn store(
        &mut self,
        owner: CollisionOwner,
        field: PushNumber,
        value: f32,
    ) -> Result<(), Self::Error> {
        if projected::handles(self.host, owner) {
            return projected::store(self.host, owner, field, value);
        }
        let entity = self.current_owner(owner)?;
        self.edit_owner(owner, |vm, services, context, native| {
            vm.push_store(&mut services.server.entities, context, native, field, value)?;
            if matches!(
                field,
                PushNumber::Motion(MotionWrite::Origin(_) | MotionWrite::Velocity(_))
            ) && let Some(entity) = entity
            {
                vm.refresh_client_pose(services, context, native, entity)?;
            }
            Ok(())
        })
    }

    fn flags(&mut self, owner: CollisionOwner) -> Result<u32, Self::Error> {
        if projected::handles(self.host, owner) {
            let (id, _) =
                projected::client(self.host, owner).ok_or(ClientCommandError::Inactive)?;
            return Ok(
                if self.host.runtime.server.clients[id.0 as usize]
                    .player
                    .movement
                    .grounded
                {
                    self.push_rules()?.ground_flag
                } else {
                    0
                },
            );
        }
        self.edit_owner(owner, |vm, _, _, native| vm.push_flags(native))
    }

    fn store_flags(&mut self, owner: CollisionOwner, flags: u32) -> Result<(), Self::Error> {
        if projected::handles(self.host, owner) {
            let (id, _) =
                projected::client(self.host, owner).ok_or(ClientCommandError::Inactive)?;
            let grounded = flags & self.push_rules()?.ground_flag != 0;
            let movement = &mut self.host.runtime.server.clients[id.0 as usize]
                .player
                .movement;
            movement.grounded = grounded;
            if !grounded {
                movement.ground = None;
            }
            return Ok(());
        }
        self.edit_owner(owner, |vm, services, context, native| {
            vm.push_store_flags(&mut services.server.entities, context, native, flags)
        })
    }

    fn ground_owner(&mut self, owner: CollisionOwner) -> Result<CollisionOwner, Self::Error> {
        if projected::handles(self.host, owner) {
            let (id, _) =
                projected::client(self.host, owner).ok_or(ClientCommandError::Inactive)?;
            return Ok(self.host.runtime.server.clients[id.0 as usize]
                .player
                .movement
                .ground
                .map_or(CollisionOwner::None, CollisionOwner::Lifetime));
        }
        self.edit_owner(owner, |vm, _, context, native| {
            vm.push_ground(context, native)
        })
    }

    fn same_owner(&self, left: CollisionOwner, right: CollisionOwner) -> bool {
        self.host.runtime.server.entities.same_owner(left, right)
    }

    fn test_position(&mut self, owner: CollisionOwner) -> Result<bool, Self::Error> {
        if projected::handles(self.host, owner) {
            let Some((_, entity)) = projected::client(self.host, owner) else {
                return Ok(false);
            };
            let position = self.host.runtime.server.entities.columns.position[entity.slot as usize];
            return projected::trace(
                self.host,
                owner,
                self.module,
                position,
                QuakeTraceKind::Normal,
            )
            .map(|trace| trace.trace.start_solid);
        }
        let reference = self.reference(owner)?;
        let ResourceNamespace::Module(module) = reference.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        let trace_rules = self
            .current_owner(owner)?
            .and_then(|entity| self.active_client(module, entity))
            .map(|id| {
                self.host.runtime.server.clients[id.0 as usize]
                    .player
                    .trace_rules
            });
        self.edit_owner(owner, |vm, services, mut context, native| {
            if let Some(rules) = trace_rules {
                context.trace_rules = rules;
            }
            vm.push_test_position(services, context, native)
        })
    }

    fn link(&mut self, owner: CollisionOwner, touch: bool) -> Result<(), Self::Error> {
        if projected::handles(self.host, owner) {
            return projected::link(self.host, owner, self.module, touch, self.driver);
        }
        if PushHost::free(self, owner)? {
            return Ok(());
        }
        let reference = self.reference(owner)?;
        let entity = self.current_owner(owner)?;
        self.edit_owner(owner, |vm, services, context, native| {
            vm.relink_entity(services, context, native)?;
            if let Some(entity) = entity {
                vm.refresh_client_pose(services, context, native, entity)?;
            }
            sync_attachments(vm, services, context)
        })?;
        if touch && let Some(entity) = self.current_owner(owner)? {
            let ResourceNamespace::Module(module) = reference.namespace else {
                return Err(ClientCommandError::DifferentModule);
            };
            let time = if module == self.module {
                self.time
            } else {
                self.owner_time(module)?
            };
            contacts::triggers(
                self.host,
                module,
                crate::modules::execution::TraceAccess::empty(),
                entity,
                time,
                self.driver,
            )?;
        }
        Ok(())
    }

    fn push_entity(
        &mut self,
        owner: CollisionOwner,
        displacement: Vec3,
    ) -> Result<(), Self::Error> {
        if projected::handles(self.host, owner) {
            let (_, entity) =
                projected::client(self.host, owner).ok_or(ClientCommandError::Inactive)?;
            let rules = self.push_rules()?;
            let mover = contacts::capture(self.host, entity)?;
            return physics::push_entity(
                &mut projected::Passenger {
                    host: self.host,
                    driver: self.driver,
                    owner,
                    provider: self.module,
                    mover,
                    rules,
                },
                entity,
                displacement,
            )
            .map(|_| ());
        }
        let reference = self.reference(owner)?;
        let entity = match self.current_owner(owner)? {
            Some(entity) => entity,
            None => self.edit_owner(owner, |vm, services, context, native| {
                vm.ensure_client_body(services, context, native)
            })?,
        };
        let ResourceNamespace::Module(module) = reference.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        let time = if module == self.module {
            self.time
        } else {
            self.owner_time(module)?
        };
        let client = self.active_client(module, entity);
        let mut moving = if let Some(client) = client {
            Host::client(self.host, module, client, time, self.driver)?
        } else {
            Host::new(self.host, module, entity, time, self.driver)?
        };
        physics::push_entity(&mut moving, entity, displacement).map(|_| ())
    }

    fn blocked(
        &mut self,
        pusher: CollisionOwner,
        blocker: CollisionOwner,
        captured: Option<EntityReference>,
    ) -> Result<(), Self::Error> {
        let subject = self.reference(pusher)?;
        let binding = self.edit_owner(pusher, |vm, _, _, native| vm.blocked_binding(native))?;
        let Some(binding) = binding else {
            return Ok(());
        };
        let ResourceNamespace::Module(module) = subject.namespace else {
            return Err(ClientCommandError::DifferentModule);
        };
        let anchor = self
            .host
            .module_row(module)
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
        let other = self
            .host
            .runtime
            .server
            .entities
            .native_owner(blocker)
            .map(|_| self.reference(blocker))
            .transpose()?;
        let admitted = if other.is_some_and(|other| other.namespace == subject.namespace) {
            None
        } else {
            Some(contacts::admit_captured(
                self.host,
                module,
                crate::modules::execution::TraceAccess::empty(),
                captured.ok_or(ClientCommandError::InvalidSlot)?,
            )?)
        };
        let other = admitted
            .as_ref()
            .map(|admitted| admitted.reference())
            .or(other)
            .ok_or(ClientCommandError::InvalidSlot)?;
        let call = CallbackCall::Blocked {
            entity,
            subject,
            other,
        };
        let result = functions
            .dispatch_resolved(self.host, resolved, call)
            .map_err(ClientCommandError::Dispatch)
            .and_then(|dispatch| {
                self.driver
                    .complete(self.host, dispatch, call)
                    .then_some(())
                    .ok_or(ClientCommandError::Dispatch(
                        qa_session::dispatch::CallError::Rejected,
                    ))
            });
        let released = admitted
            .map(|admitted| contacts::release(self.host, admitted))
            .transpose()
            .map(|_| ());
        result.and(released)
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
        let result = call_think_once_with_completion(
            self.host,
            &functions,
            entity,
            Some(subject),
            time,
            |host| contacts::current(host, subject).ok().flatten(),
            |host, outcome, call| self.driver.complete(host, outcome, call),
        );
        (result.rejected == 0)
            .then_some(())
            .ok_or(ClientCommandError::Dispatch(
                qa_session::dispatch::CallError::Rejected,
            ))
    }
}

impl qa_session::physics::DisplacementHost for Host<'_, '_> {
    type Error = ClientCommandError;
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.host.runtime.server.entities
    }
    fn state(&mut self, entity: EntityId) -> Result<MotionState, Self::Error> {
        self.edit(entity, |vm, _, context, native| {
            vm.motion_state(context, native)
        })
    }
    fn write_origin(
        &mut self,
        entity: EntityId,
        axis: usize,
        value: f32,
    ) -> Result<(), Self::Error> {
        MotionHost::write_motion(self, entity, MotionWrite::Origin(axis), value)
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
        contacts::triggers(
            self.host,
            self.module,
            crate::modules::execution::TraceAccess::empty(),
            entity,
            self.time,
            self.driver,
        )?;
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
            crate::modules::execution::TraceAccess::empty(),
            mover_entity,
            self.mover,
            obstacle,
            self.time,
            self.driver,
        )?;
        self.current()
    }
}

impl MotionHost for Host<'_, '_> {
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
            vm.write_motion(entities, context, native, field)?;
            if matches!(field, MotionWrite::Origin(_) | MotionWrite::Velocity(_)) {
                vm.refresh_client_pose(services, context, native, entity)?;
            }
            Ok(())
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
    fn write_flags(&mut self, entity: EntityId, flags: u32) -> Result<(), Self::Error> {
        self.write_field(entity, MotionField::Flags(flags))
    }
    fn gravity_scale(&mut self, entity: EntityId) -> Result<Option<f32>, Self::Error> {
        self.edit(entity, |vm, _, context, native| {
            vm.motion_gravity(context, native)
        })
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

impl ClientHost for Host<'_, '_> {
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
            vm.with_trace_view(context, native, services.native_geometry, |view, pass| {
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

#[cfg(test)]
#[path = "../../tests/modules/foreign_push.rs"]
mod foreign_push;
