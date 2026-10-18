//! Canonical client passengers with optional QVM or native public views.
use super::*;
use qa_core::primitives::CollisionShape;
use qa_gameplay::rules::motion::PushRules;
use qa_session::physics::DisplacementHost;
use qa_world::area::{LinkFlags, LinkIntent};

pub(super) fn native(host: &FrameHost, owner: CollisionOwner) -> Option<NativeEntity> {
    let native = host.runtime.server.entities.native_owner(owner)?;
    let row = host.module_row(native.module)?;
    (!matches!(&row.request.program, Program::QuakeC(_))).then_some(native)
}

pub(super) fn handles(host: &FrameHost, owner: CollisionOwner) -> bool {
    native(host, owner).is_some()
        || (matches!(owner, CollisionOwner::Lifetime(_))
            && host.runtime.server.entities.native_owner(owner).is_none())
}

pub(super) fn client(host: &FrameHost, owner: CollisionOwner) -> Option<(ClientId, EntityId)> {
    let entities = &host.runtime.server.entities;
    let entity = if let Some(native) = entities.native_owner(owner) {
        let row = host.module_row(native.module)?;
        if row.state != State::Running
            || matches!(&row.request.program, Program::QuakeC(_))
            || entities
                .resolve(row.request.anchor)
                .is_none_or(|slot| entities.columns.owner[slot] != native.module)
        {
            return None;
        }
        row.request.program.bound_entity(entities, native).ok()??
    } else {
        let CollisionOwner::Lifetime(entity) = owner else {
            return None;
        };
        entities.resolve(entity)?;
        entity
    };
    let id = ClientId(entities.client_index(entity)? as u32);
    let player = host.runtime.server.clients.get(id.0 as usize)?;
    (player.entity == entity
        && player.is_server_active()
        && player.module == entities.columns.owner[entity.slot as usize])
        .then_some((id, entity))
}

pub(super) fn candidate(host: &FrameHost, owner: CollisionOwner) -> bool {
    client(host, owner).is_some_and(|(_, entity)| host.runtime.server.area.bounds(entity).is_some())
}

fn service_module(host: &FrameHost, owner: CollisionOwner, provider: ModuleId) -> ModuleId {
    host.runtime
        .server
        .entities
        .native_owner(owner)
        .map_or(provider, |native| native.module)
}

pub(super) fn number(
    host: &FrameHost,
    owner: CollisionOwner,
    field: PushNumber,
    rules: PushRules,
) -> Result<f32, ClientCommandError> {
    let (id, entity) = client(host, owner).ok_or(ClientCommandError::Inactive)?;
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
    owner: CollisionOwner,
    position: Vec3,
    relink: bool,
) -> Result<(), ClientCommandError> {
    let Some((id, entity)) = client(host, owner) else {
        return Ok(());
    };
    let Some(native) = host.runtime.server.entities.native_owner(owner) else {
        let server = &mut host.runtime.server;
        server
            .entities
            .columns
            .transport_position(entity.slot as usize, position);
        let client = &mut server.clients[id.0 as usize];
        client.player.body.position = position;
        if relink {
            server.area.link(
                &server.entities,
                entity,
                LinkFlags::SOLID,
                client.link_order,
                LinkIntent::Explicit,
            );
        }
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
    owner: CollisionOwner,
    field: PushNumber,
    value: f32,
) -> Result<(), ClientCommandError> {
    let Some((_, entity)) = client(host, owner) else {
        return Ok(());
    };
    let PushNumber::Motion(MotionWrite::Origin(axis)) = field else {
        return Err(PhysicsError::MissingField.into());
    };
    let mut position = host.runtime.server.entities.columns.position[entity.slot as usize];
    position.0[axis] = value;
    transport(host, owner, position, false)
}

pub(super) fn trace(
    host: &mut FrameHost,
    owner: CollisionOwner,
    provider: ModuleId,
    end: Vec3,
    kind: QuakeTraceKind,
) -> Result<MotionTrace, ClientCommandError> {
    let (id, entity) = client(host, owner).ok_or(ClientCommandError::Inactive)?;
    let result = super::super::edit_module(
        host,
        service_module(host, owner, provider),
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
    owner: CollisionOwner,
    provider: ModuleId,
    touch: bool,
    driver: &mut crate::modules::Driver<'_>,
) -> Result<(), ClientCommandError> {
    let Some((_, entity)) = client(host, owner) else {
        return Ok(());
    };
    let position = host.runtime.server.entities.columns.position[entity.slot as usize];
    transport(host, owner, position, true)?;
    if touch {
        let module = service_module(host, owner, provider);
        let time = host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?
            .time(host.time);
        contacts::triggers(
            host,
            module,
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
    pub owner: CollisionOwner,
    pub provider: ModuleId,
    pub mover: EntityReference,
    pub rules: PushRules,
}
impl Passenger<'_, '_> {
    fn current(&self) -> Option<EntityId> {
        client(self.host, self.owner)
            .filter(|(_, entity)| *entity == self.mover.entity())
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
            move_type: number(self.host, self.owner, PushNumber::MoveType, self.rules)?,
            solid: Some(number(
                self.host,
                self.owner,
                PushNumber::Solid,
                self.rules,
            )?),
            flags: None,
        })
    }
    fn write_origin(&mut self, _: EntityId, axis: usize, value: f32) -> Result<(), Self::Error> {
        store(
            self.host,
            self.owner,
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
        trace(self.host, self.owner, self.provider, end, kind)
    }
    fn relink(&mut self, _: EntityId) -> Result<Option<EntityId>, Self::Error> {
        link(self.host, self.owner, self.provider, true, self.driver)?;
        Ok(self.current())
    }
    fn impact(
        &mut self,
        _: Option<EntityId>,
        trace: &MotionTrace,
    ) -> Result<Option<EntityId>, Self::Error> {
        let obstacle = trace.ground.ok_or(ClientCommandError::InvalidSlot)?;
        let module = service_module(self.host, self.owner, self.provider);
        let time = self
            .host
            .module_row(module)
            .ok_or(ClientCommandError::MissingModule)?
            .time(self.host.time);
        let current = self.current();
        contacts::impact(
            self.host,
            module,
            super::super::execution::TraceAccess::empty(),
            current,
            self.mover,
            obstacle,
            time,
            self.driver,
        )?;
        Ok(self.current())
    }
}
