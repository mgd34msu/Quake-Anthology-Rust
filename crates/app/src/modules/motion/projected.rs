//! Public player/entity views for passengers owned by QVM or native modules.
use super::*;
use qa_core::primitives::CollisionShape;
use qa_gameplay::rules::motion::PushRules;
use qa_session::physics::DisplacementHost;

pub(super) fn native(host: &FrameHost, owner: CollisionOwner) -> Option<NativeEntity> {
    let native = host.runtime.server.entities.native_owner(owner)?;
    let row = host.module_row(native.module)?;
    (!matches!(&row.request.program, Program::QuakeC(_))).then_some(native)
}

pub(super) fn client(host: &FrameHost, native: NativeEntity) -> Option<(ClientId, EntityId)> {
    let row = host.module_row(native.module)?;
    let entities = &host.runtime.server.entities;
    if row.state != State::Running
        || matches!(&row.request.program, Program::QuakeC(_))
        || entities
            .resolve(row.request.anchor)
            .is_none_or(|slot| entities.columns.owner[slot] != native.module)
    {
        return None;
    }
    let entity = row.request.program.bound_entity(entities, native).ok()??;
    let id = ClientId(entities.client_index(entity)? as u32);
    let player = host.runtime.server.clients.get(id.0 as usize)?;
    (player.entity == entity
        && player.is_active()
        && player.server_authoritative()
        && player.module == native.module)
        .then_some((id, entity))
}

pub(super) fn number(
    host: &FrameHost,
    native: NativeEntity,
    field: PushNumber,
    rules: PushRules,
) -> Result<f32, ClientCommandError> {
    let (id, entity) = client(host, native).ok_or(ClientCommandError::Inactive)?;
    let columns = &host.runtime.server.entities.columns;
    let slot = entity.slot as usize;
    Ok(match field {
        PushNumber::MoveType => rules.player_move_type(
            host.runtime.server.clients[id.0 as usize]
                .player
                .movement
                .mode,
            columns.collision_shape[slot] != CollisionShape::None,
        ),
        PushNumber::Solid => {
            if columns.collision_shape[slot] == CollisionShape::None {
                rules.solid_not()
            } else {
                rules.box_solid()
            }
        }
        PushNumber::Motion(MotionWrite::Origin(axis)) => columns.position[slot].0[axis],
        PushNumber::Min(axis) => columns.mins[slot].0[axis],
        PushNumber::Max(axis) => columns.maxs[slot].0[axis],
        PushNumber::AbsoluteMin(axis) => {
            host.runtime
                .server
                .area
                .bounds(entity)
                .ok_or(ClientCommandError::Inactive)?
                .mins
                .0[axis]
        }
        PushNumber::AbsoluteMax(axis) => {
            host.runtime
                .server
                .area
                .bounds(entity)
                .ok_or(ClientCommandError::Inactive)?
                .maxs
                .0[axis]
        }
        _ => return Err(PhysicsError::MissingField.into()),
    })
}

pub(super) fn transport(
    host: &mut FrameHost,
    native: NativeEntity,
    position: Vec3,
    relink: bool,
) -> Result<(), ClientCommandError> {
    let Some((_, entity)) = client(host, native) else {
        return Ok(());
    };
    super::super::edit_module(
        host,
        native.module,
        super::super::execution::TraceAccess::empty(),
        |program, services, context| match program {
            Program::Qvm { vm, .. } => vm
                .transport_entity(services, context, entity, position, relink)
                .map_err(ClientCommandError::Usercmd),
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Program::Native { vm, .. } => vm
                .transport_entity(services, context, entity, position, relink)
                .map_err(|error| match error {
                    qa_compat::native::Error::Service(error) => ClientCommandError::Usercmd(error),
                    qa_compat::native::Error::Process(error) => ClientCommandError::Memory(
                        qa_compat::memory::ProcessMemoryError::Process(error),
                    ),
                    _ => ClientCommandError::InvalidSlot,
                }),
            Program::QuakeC(_) => Err(ClientCommandError::NeedsNativeModule),
        },
    )
}

pub(super) fn store(
    host: &mut FrameHost,
    native: NativeEntity,
    field: PushNumber,
    value: f32,
) -> Result<(), ClientCommandError> {
    let Some((_, entity)) = client(host, native) else {
        return Ok(());
    };
    let PushNumber::Motion(MotionWrite::Origin(axis)) = field else {
        return Err(PhysicsError::MissingField.into());
    };
    let mut position = host.runtime.server.entities.columns.position[entity.slot as usize];
    position.0[axis] = value;
    transport(host, native, position, false)
}

pub(super) fn trace(
    host: &mut FrameHost,
    native: NativeEntity,
    end: Vec3,
    kind: QuakeTraceKind,
) -> Result<MotionTrace, ClientCommandError> {
    let (id, entity) = client(host, native).ok_or(ClientCommandError::Inactive)?;
    let result = super::super::edit_module(
        host,
        native.module,
        super::super::execution::TraceAccess::empty(),
        |_, services, _| {
            let columns = &services.server.entities.columns;
            let body = columns.body(entity.slot as usize);
            let (rules, entity_rules) =
                trace_policy(services.server.clients[id.0 as usize].player.trace_rules);
            let mut query = TraceQuery {
                mins: body.mins,
                maxs: body.maxs,
                pass: CollisionOwner::Lifetime(entity),
                ..TraceQuery::point(
                    body.position,
                    end,
                    rules,
                    entity_rules.with_quake_kind(Some(kind)),
                )
            };
            if columns.collision_shape[entity.slot as usize] == CollisionShape::Capsule {
                query = query.with_capsule();
            }
            services
                .trace(query, None)
                .map_err(ClientCommandError::Usercmd)
        },
    )?;
    let ground = result
        .entity
        .map(|hit| contacts::capture(host, hit))
        .transpose()?;
    Ok(MotionTrace {
        trace: result,
        ground,
    })
}

pub(super) fn link(
    host: &mut FrameHost,
    native: NativeEntity,
    touch: bool,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let Some((_, entity)) = client(host, native) else {
        return Ok(());
    };
    let position = host.runtime.server.entities.columns.position[entity.slot as usize];
    transport(host, native, position, true)?;
    if touch {
        let time = host
            .module_row(native.module)
            .ok_or(ClientCommandError::MissingModule)?
            .time(host.time);
        contacts::triggers(
            host,
            native.module,
            super::super::execution::TraceAccess::empty(),
            entity,
            time,
            driver,
        )?;
    }
    Ok(())
}

pub(super) struct Passenger<'a, 'd> {
    pub host: &'a mut FrameHost,
    pub driver: &'a mut crate::modules::Driver<'d>,
    pub native: NativeEntity,
    pub entity: EntityId,
    pub rules: PushRules,
}
impl Passenger<'_, '_> {
    fn current(&self) -> Option<EntityId> {
        client(self.host, self.native)
            .filter(|(_, entity)| *entity == self.entity)
            .map(|(_, entity)| entity)
    }
}
impl DisplacementHost for Passenger<'_, '_> {
    type Error = ClientCommandError;
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.host.runtime.server.entities
    }
    fn state(&mut self, _: EntityId) -> Result<MotionState, Self::Error> {
        Ok(MotionState {
            move_type: number(self.host, self.native, PushNumber::MoveType, self.rules)?,
            solid: Some(number(
                self.host,
                self.native,
                PushNumber::Solid,
                self.rules,
            )?),
            flags: None,
        })
    }
    fn write_origin(&mut self, _: EntityId, axis: usize, value: f32) -> Result<(), Self::Error> {
        store(
            self.host,
            self.native,
            PushNumber::Motion(MotionWrite::Origin(axis)),
            value,
        )
    }
    fn trace(
        &mut self,
        _: EntityId,
        end: Vec3,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, Self::Error> {
        trace(self.host, self.native, end, kind)
    }
    fn relink(&mut self, _: EntityId) -> Result<Option<EntityId>, Self::Error> {
        link(self.host, self.native, true, self.driver)?;
        Ok(self.current())
    }
    fn impact(
        &mut self,
        _: Option<EntityId>,
        trace: &MotionTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        let obstacle = trace.ground.ok_or(ClientCommandError::InvalidSlot)?;
        let module = self.native.module;
        let time = self
            .host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?
            .time(self.host.time);
        let mover = contacts::capture(self.host, self.entity)?;
        let current = self.current();
        contacts::impact(
            self.host,
            module,
            super::super::execution::TraceAccess::empty(),
            current,
            mover,
            obstacle,
            time,
            self.driver,
        )?;
        Ok(self.current())
    }
}
