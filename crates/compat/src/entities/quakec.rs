//! QuakeC byte-offset edicts use the same namespace/lifetime projection as
//! native modules. Only ABI field storage and ordinal reuse live here.
use super::*;
use qa_formats::program::quakec::Image;
use qa_world::entities::FullTable;

pub(super) struct ThinkBindings {
    module: qa_session::dispatch::ModuleBinding,
    functions: Box<[qa_core::primitives::ThinkBinding]>,
}
impl ThinkBindings {
    fn missing(
        module: qa_session::dispatch::ModuleBinding,
        word: u32,
    ) -> qa_core::primitives::ThinkBinding {
        let mut missing = module.think(None);
        missing.callback = Some(qa_core::primitives::CallbackId(word));
        missing
    }
}

fn client_edicts(
    rules: qa_core::primitives::RuleSetId,
    max_clients: i32,
) -> Result<usize, CallError> {
    // QW reserves fixed protocol client ordinals, independently of the live
    // maxclients cvar. NQ reserves the server's actual client count.
    if rules == qa_core::primitives::RuleSetId::QuakeWorld {
        Ok(32)
    } else {
        usize::try_from(max_clients).map_err(|_| CallError::Entity)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct QuakeCLayout {
    stride: usize,
    header: usize,
    fields: usize,
    origin: Option<usize>,
    velocity: Option<usize>,
    angles: Option<usize>,
    angular_velocity: Option<usize>,
    mins: Option<usize>,
    maxs: Option<usize>,
    size: Option<usize>,
    abs_min: Option<usize>,
    abs_max: Option<usize>,
    solid: Option<usize>,
    flags: Option<usize>,
    owner: Option<usize>,
    model: Option<usize>,
    model_index: Option<usize>,
    frame: Option<usize>,
    skin: Option<usize>,
    effects: Option<usize>,
    take_damage: Option<usize>,
    colormap: Option<usize>,
    next_think: Option<usize>,
    think: Option<usize>,
    move_type: Option<usize>,
}
impl QuakeCLayout {
    fn load(image: &Image, stride: usize, header: usize) -> Self {
        let field = |name: &[u8], kind| {
            crate::quakec::definition(image, true, name, kind).map(|offset| header + offset * 4)
        };
        Self {
            stride,
            header,
            fields: image.entityfields * 4,
            origin: field(b"origin", 3),
            velocity: field(b"velocity", 3),
            angles: field(b"angles", 3),
            angular_velocity: field(b"avelocity", 3),
            mins: field(b"mins", 3),
            maxs: field(b"maxs", 3),
            size: field(b"size", 3),
            abs_min: field(b"absmin", 3),
            abs_max: field(b"absmax", 3),
            solid: field(b"solid", 2),
            flags: field(b"flags", 2),
            owner: field(b"owner", 4),
            model: field(b"model", 1),
            model_index: field(b"modelindex", 2),
            frame: field(b"frame", 2),
            skin: field(b"skin", 2),
            effects: field(b"effects", 2),
            take_damage: field(b"takedamage", 2),
            colormap: field(b"colormap", 2),
            next_think: field(b"nextthink", 2),
            think: field(b"think", 6),
            move_type: field(b"movetype", 2),
        }
    }
    fn address(self, memory: &ModuleMemory<'_>, value: u32) -> Result<u64, CallError> {
        let address = value as usize;
        if !address.is_multiple_of(self.stride) {
            return Err(CallError::Entity);
        }
        memory.read(value as u64, self.stride)?;
        Ok(value as u64)
    }
    fn identity(
        self,
        memory: &ModuleMemory<'_>,
        module: ModuleId,
        value: u32,
    ) -> Result<NativeEntity, CallError> {
        self.address(memory, value)?;
        Ok(NativeEntity {
            module,
            slot: (value as usize / self.stride) as i32,
        })
    }
    fn owner(
        self,
        memory: &ModuleMemory<'_>,
        module: ModuleId,
        address: u64,
    ) -> Result<CollisionOwner, CallError> {
        let value = self.word(memory, address, self.owner)?;
        let owner = self.identity(memory, module, value)?;
        Ok(if value == 0 {
            CollisionOwner::None
        } else {
            CollisionOwner::Native(owner)
        })
    }
    fn flags(self, memory: &ModuleMemory<'_>, address: u64) -> Result<u32, CallError> {
        Ok(
            crate::numbers::native_integer(f64::from(self.number(memory, address, self.flags)?))
                as u32,
        )
    }
    fn tags(flags: u32) -> CollisionTags {
        if flags & 32 != 0 {
            CollisionTags::MONSTER
        } else {
            CollisionTags::default()
        }
    }
    fn word(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
    ) -> Result<u32, CallError> {
        Ok(memory.read_word(address + field.ok_or(CallError::Memory)? as u64)? as u32)
    }
    fn number(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
    ) -> Result<f32, CallError> {
        Ok(f32::from_bits(self.word(memory, address, field)?))
    }
    fn vector(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
    ) -> Result<Vec3, CallError> {
        Ok(memory.read_vec3(address + field.ok_or(CallError::Memory)? as u64)?)
    }
    fn write_number(
        self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
        value: f32,
    ) -> Result<(), CallError> {
        memory.write_word(
            address + field.ok_or(CallError::Memory)? as u64,
            value.to_bits() as i32,
        )?;
        Ok(())
    }
    fn write_vector(
        self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
        value: Vec3,
    ) -> Result<(), CallError> {
        memory.write_vec3(address + field.ok_or(CallError::Memory)? as u64, value)?;
        Ok(())
    }
}

pub(crate) struct QuakeCTraceView<'a, 'memory> {
    memory: &'a ModuleMemory<'memory>,
    layout: QuakeCLayout,
    module: ModuleId,
}
impl qa_world::collision::NativeTraceEntities for QuakeCTraceView<'_, '_> {
    fn module(&self) -> ModuleId {
        self.module
    }
    fn fields(&self, slot: i32) -> Option<(CollisionOwner, CollisionTags)> {
        let address = u32::try_from(slot)
            .ok()?
            .checked_mul(self.layout.stride as u32)?;
        self.layout
            .identity(self.memory, self.module, address)
            .ok()?;
        Some((
            self.layout
                .owner(self.memory, self.module, u64::from(address))
                .ok()?,
            QuakeCLayout::tags(self.layout.flags(self.memory, u64::from(address)).ok()?),
        ))
    }
}

impl EntityProjection {
    pub(crate) fn quakec_trace_view<'a, 'memory>(
        &self,
        memory: &'a ModuleMemory<'memory>,
        context: CallContext,
        pass: u32,
    ) -> Result<(QuakeCTraceView<'a, 'memory>, CollisionOwner), CallError> {
        let pass = self.quakec_identity(memory, context, pass)?;
        Ok((
            QuakeCTraceView {
                memory,
                layout: self.quakec_layout()?,
                module: context.module,
            },
            CollisionOwner::Native(pass),
        ))
    }
    pub(crate) fn load_quakec(
        image: &Image,
        stride: usize,
        header: usize,
        capacity: usize,
    ) -> Self {
        Self {
            table_offset: 0,
            wide_stride: false,
            schema: EntitySchema::QuakeC(QuakeCLayout::load(image, stride, header)),
            models: qa_core::primitives::IndexSpan::default(),
            model_handles: Box::new([]),
            owner: None,
            bindings: vec![None; capacity].into_boxed_slice(),
            quakec_thinks: None,
            observed_count: 1,
            leaves: [0; 128],
        }
    }
    fn quakec_layout(&self) -> Result<QuakeCLayout, CallError> {
        match self.schema {
            EntitySchema::QuakeC(layout) => Ok(layout),
            EntitySchema::Native(_) => Err(CallError::Entity),
        }
    }
    pub(crate) fn quakec_bind_thinks(
        &mut self,
        module: qa_session::dispatch::ModuleBinding,
        entries: &[u32],
        function_count: usize,
    ) {
        let mut functions = (0..function_count)
            .map(|word| ThinkBindings::missing(module, word as u32))
            .collect::<Box<[_]>>();
        for (callback, &entry) in entries.iter().enumerate().rev() {
            if let Some(to) = functions.get_mut(entry as usize) {
                *to = module.think(Some(qa_core::primitives::CallbackId(callback as u32)));
            }
        }
        self.quakec_thinks = Some(ThinkBindings { module, functions });
        for row in self.bindings.iter_mut().flatten() {
            row.think_word = None;
        }
    }
    pub(crate) fn quakec_physics_entity(
        &self,
        entities: &EntityTable,
        memory: &ModuleMemory<'_>,
        cursor: &mut usize,
        rules: qa_core::primitives::RuleSetId,
        max_clients: i32,
    ) -> Result<Option<crate::quakec::PhysicsEntity>, CallError> {
        let layout = self.quakec_layout()?;
        let module = self.owner.ok_or(CallError::Entity)?;
        while *cursor < self.observed_count {
            let native_slot = *cursor;
            *cursor += 1;
            let native = NativeEntity {
                module,
                slot: native_slot as i32,
            };
            // The namespace rows establish observable native order, while
            // EntityTable's generation and sole liveness bitset own lifetime.
            let Some(entity) = self.binding(entities, native)? else {
                continue;
            };
            let client = native_slot > 0 && native_slot <= client_edicts(rules, max_clients)?;
            let phase = if !client && layout.move_type.is_some() {
                let move_type = layout.number(
                    memory,
                    (native_slot * layout.stride) as u64,
                    layout.move_type,
                )?;
                let (none, noclip) = if rules == qa_core::primitives::RuleSetId::QuakeWorld {
                    let move_type = crate::numbers::native_integer(f64::from(move_type));
                    (move_type == 0, move_type == 8)
                } else {
                    (move_type == 0.0, move_type == 8.0)
                };
                if none {
                    crate::quakec::PhysicsPhase::None
                } else if noclip {
                    crate::quakec::PhysicsPhase::Noclip
                } else {
                    crate::quakec::PhysicsPhase::Unsupported
                }
            } else {
                crate::quakec::PhysicsPhase::Unsupported
            };
            return Ok(Some(crate::quakec::PhysicsEntity {
                entity,
                native,
                phase,
            }));
        }
        Ok(None)
    }
    pub(crate) fn quakec_clear_think(
        &self,
        memory: &mut ModuleMemory<'_>,
        value: u32,
    ) -> Result<(), CallError> {
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        layout.write_number(memory, address, layout.next_think, 0.0)
    }
    fn quakec_identity(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<NativeEntity, CallError> {
        if self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        self.quakec_layout()?
            .identity(memory, context.module, value)
    }
    pub(crate) fn quakec_spawn(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
    ) -> Result<u32, CallError> {
        let layout = self.quakec_layout()?;
        if self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        let clients = client_edicts(
            context.native_rules,
            services
                .storage
                .max_clients(services.cvars, context.native_rules),
        )?;
        let first = clients.checked_add(1).ok_or(CallError::Capacity)?;
        if first >= self.bindings.len() {
            return Err(CallError::Capacity);
        }
        self.observed_count = self.observed_count.max(first);
        let eligible = (first..self.observed_count).find(|&slot| {
            let row = self.bindings[slot];
            let free = row
                .is_none_or(|binding| services.server.entities.resolve(binding.entity).is_none());
            free && context
                .allocation
                .reuse
                .eligible(context.clock, row.and_then(|binding| binding.freed_at))
        });
        let mut claim = context;
        claim.allocation.full = FullTable::Reject;
        let slot = if let Some(slot) = eligible {
            slot
        } else if self.observed_count < self.bindings.len() {
            self.observed_count
        } else if context.allocation.full == FullTable::OverwriteLastOwned {
            let slot = self.bindings.len() - 1;
            let binding = self.bindings[slot].ok_or(CallError::Capacity)?;
            if !binding.owned {
                return Err(CallError::Capacity);
            }
            claim.allocation.full = FullTable::OverwriteOwned(binding.entity);
            slot
        } else {
            return Err(CallError::Capacity);
        };
        self.bind(services, claim, slot, None)?;
        self.observed_count = self.observed_count.max(slot + 1);
        let address = slot.checked_mul(layout.stride).ok_or(CallError::Memory)?;
        // ED_ClearEdict clears only entvars. Boundary metadata does not inhabit
        // VM bytes and common lifetime liveness remains in EntityTable.
        memory
            .read_mut((address + layout.header) as u64, layout.fields)?
            .fill(0);
        u32::try_from(address).map_err(|_| CallError::Memory)
    }
    pub(crate) fn quakec_remove(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self
            .binding(&services.server.entities, native)?
            .ok_or(CallError::Entity)?;
        let slot = native.slot as usize;
        let binding = self.bindings[slot].ok_or(CallError::Entity)?;
        // World/client lifetime ownership remains at the session boundary.
        if slot == 0 || !binding.owned {
            return Err(CallError::Entity);
        }
        let layout = self.quakec_layout()?;
        (ENGINE_CALLS.free)(services, context, entity)?;
        self.bindings[slot]
            .as_mut()
            .ok_or(CallError::Entity)?
            .freed_at = Some(context.allocation.reuse.freetime(context.clock));
        for field in [
            layout.model,
            layout.take_damage,
            layout.model_index,
            layout.colormap,
            layout.skin,
            layout.frame,
            layout.solid,
        ] {
            layout.write_number(memory, value as u64, field, 0.0)?;
        }
        for field in [layout.origin, layout.angles] {
            layout.write_vector(memory, value as u64, field, Vec3::default())?;
        }
        layout.write_number(memory, value as u64, layout.next_think, -1.0)
    }
    pub(crate) fn quakec_set_origin(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        origin: Vec3,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        layout.write_vector(memory, value as u64, layout.origin, origin)?;
        self.quakec_link(services, memory, strings, context, value)
    }
    pub(crate) fn quakec_set_size(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        bounds: Bounds,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        // PF_setsize rejects backwards extents before modifying any field.
        if !bounds.is_valid() {
            return Err(CallError::Entity);
        }
        let layout = self.quakec_layout()?;
        layout.write_vector(memory, value as u64, layout.mins, bounds.mins)?;
        layout.write_vector(memory, value as u64, layout.maxs, bounds.maxs)?;
        layout.write_vector(memory, value as u64, layout.size, bounds.maxs - bounds.mins)?;
        self.quakec_link(services, memory, strings, context, value)
    }
    pub(crate) fn quakec_set_model(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        name: u32,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        let models = services
            .storage
            .resources(context.module)
            .ok_or(CallError::ConfigString)?[0];
        let text = strings.cstring(u64::from(name))?;
        let ordinal = (ENGINE_CALLS.resource_find)(services, context.module, models, text)?
            .ok_or(CallError::Geometry)?;
        let layout = self.quakec_layout()?;
        memory.write_word(
            u64::from(value) + layout.model.ok_or(CallError::Memory)? as u64,
            name as i32,
        )?;
        layout.write_number(memory, u64::from(value), layout.model_index, ordinal as f32)?;
        let bounds = if text.starts_with(b"*") {
            Some(
                Self::inline_model(services, services.world.ok_or(CallError::Geometry)?.0, text)?.1,
            )
        } else if context.native_rules == qa_core::primitives::RuleSetId::QuakeWorld {
            None
        } else if ordinal == 0 {
            Some(Bounds::default())
        } else {
            Some((ENGINE_CALLS.model_bounds)(
                services,
                text,
                context.native_rules,
            )?)
        };
        if let Some(bounds) = bounds {
            self.quakec_set_size(services, memory, strings, context, value, bounds)
        } else {
            self.quakec_publish(services, memory, strings, context, value)?;
            Ok(())
        }
    }
    pub(crate) fn quakec_sound_source(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(EntityId, Vec3, bool), CallError> {
        let (entity, _) = self
            .quakec_publish(services, memory, strings, context, value)?
            .ok_or(CallError::Entity)?;
        let slot = entity.slot as usize;
        let body = services.server.entities.columns.body(slot);
        let layout = self.quakec_layout()?;
        let centered = context.native_rules != qa_core::primitives::RuleSetId::QuakeWorld
            || layout.number(memory, u64::from(value), layout.solid)? == 4.0;
        let position = if centered {
            body.center()
        } else {
            body.position
        };
        Ok((entity, position, centered))
    }
    pub(crate) fn quakec_noclip(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        step: f32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        self.binding(&services.server.entities, native)?
            .ok_or(CallError::Entity)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        // SV_Physics_Noclip calls VectorMA for angles before origin. Retain
        // that order, including native fields read after the angle write.
        let angles = layout.vector(memory, address, layout.angles)?;
        let angular_velocity = layout.vector(memory, address, layout.angular_velocity)?;
        layout.write_vector(
            memory,
            address,
            layout.angles,
            angles + angular_velocity * step,
        )?;
        let origin = layout.vector(memory, address, layout.origin)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        layout.write_vector(memory, address, layout.origin, origin + velocity * step)?;
        if value == 0 {
            // Native SV_LinkEdict ignores world0, but its pose writes still
            // belong in the common columns used by presentation.
            self.quakec_publish(services, memory, strings, context, value)?;
            return Ok(());
        }
        self.quakec_link(services, memory, strings, context, value)
    }
    pub(crate) fn quakec_link(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        if value == 0 {
            return Ok(());
        }
        let layout = self.quakec_layout()?;
        if [
            layout.origin,
            layout.angles,
            layout.mins,
            layout.maxs,
            layout.solid,
            layout.flags,
            layout.owner,
        ]
        .iter()
        .any(Option::is_none)
        {
            return Err(CallError::Memory);
        }
        let Some((entity, link_flags)) =
            self.quakec_publish(services, memory, strings, context, value)?
        else {
            return Ok(());
        };
        (ENGINE_CALLS.link)(services, context, entity, link_flags)?;
        let bounds = services
            .server
            .area
            .bounds(entity)
            .ok_or(CallError::Entity)?;
        layout.write_vector(memory, value as u64, layout.abs_min, bounds.mins)?;
        layout.write_vector(memory, value as u64, layout.abs_max, bounds.maxs)?;
        Ok(())
    }
    fn quakec_publish(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<Option<(EntityId, LinkFlags)>, CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let Some(entity) = self.binding(&services.server.entities, native)? else {
            return Ok(None);
        };
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let slot = entity.slot as usize;
        let mut body = services.server.entities.columns.body(slot);
        let mut angles = services.server.entities.columns.angles[slot];
        for (field, to) in [
            (layout.origin, &mut body.position),
            (layout.velocity, &mut body.velocity),
            (layout.angles, &mut angles),
            (layout.mins, &mut body.mins),
            (layout.maxs, &mut body.maxs),
        ] {
            if field.is_some() {
                *to = layout.vector(memory, address, field)?;
            }
        }
        if !body.position.is_finite()
            || !angles.is_finite()
            || !(Bounds {
                mins: body.mins,
                maxs: body.maxs,
            })
            .is_valid()
        {
            return Err(CallError::Entity);
        }
        let number = |field| {
            layout
                .number(memory, address, field)
                .map(|value| crate::numbers::native_integer(f64::from(value)))
        };
        let flags = if layout.flags.is_some() {
            layout.flags(memory, address)?
        } else {
            0
        };
        let solid = if layout.solid.is_some() {
            number(layout.solid)?
        } else {
            0
        };
        let shape = match solid {
            0 | 1 => CollisionShape::None,
            2 | 3 => CollisionShape::Box,
            4 if value == 0 => CollisionShape::None,
            4 => {
                let offset = layout.word(memory, address, layout.model)?;
                let name = strings.cstring(u64::from(offset))?;
                Self::inline_model(services, services.world.ok_or(CallError::Geometry)?.0, name)?.0
            }
            _ => return Err(CallError::Entity),
        };
        let owner = if layout.owner.is_some() {
            layout.owner(memory, context.module, address)?
        } else {
            CollisionOwner::None
        };
        let mut appearance = EntityAppearance {
            source: context.native_rules,
            ..EntityAppearance::default()
        };
        if layout.model_index.is_some() {
            appearance.models[0] = ModelRef::native(
                ResourceNamespace::Module(context.module),
                number(layout.model_index)? as u32,
            );
        }
        if layout.frame.is_some() {
            appearance.frame = number(layout.frame)?;
        }
        if layout.skin.is_some() {
            appearance.skin = number(layout.skin)?;
        }
        if layout.effects.is_some() {
            appearance.native.effects = u64::from(number(layout.effects)? as u32);
        }
        let columns = &mut services.server.entities.columns;
        columns.set_body(slot, body);
        columns.angles[slot] = angles;
        columns.flags[slot] = flags;
        columns.collision_shape[slot] = shape;
        columns.collision_owner[slot] = owner;
        columns.model_rules[slot] = ModelRules::default();
        columns.collision_tags[slot] = QuakeCLayout::tags(flags);
        columns.collision_contents[slot] = Contents::SOLID.0;
        columns.appearance[slot] = appearance;
        if let Some(thinks) = &self.quakec_thinks {
            if layout.next_think.is_some() {
                columns.next_think[slot] = Some(qa_core::primitives::ThinkTime::Seconds(
                    f64::from(layout.number(memory, address, layout.next_think)?),
                ));
            }
            let word = layout
                .think
                .map(|field| layout.word(memory, address, Some(field)))
                .transpose()?;
            let row = self.bindings[native.slot as usize]
                .as_mut()
                .ok_or(CallError::Entity)?;
            if row.think_word != word || columns.think_binding(slot).path == 0 {
                let binding = word.map_or_else(
                    || thinks.module.think(None),
                    |word| {
                        thinks
                            .functions
                            .get(word as usize)
                            .copied()
                            .unwrap_or_else(|| ThinkBindings::missing(thinks.module, word))
                    },
                );
                columns.set_think_function(slot, binding);
                row.think_word = word;
            }
        }
        let mut link_flags = match solid {
            1 => LinkFlags::TRIGGER,
            2..=4 => LinkFlags::SOLID,
            _ => LinkFlags::LINKED,
        };
        if flags & 256 != 0 {
            link_flags.0 |= LinkFlags::ITEM;
        }
        Ok(Some((entity, link_flags)))
    }
    pub(crate) fn quakec_project(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
    ) -> Result<(), CallError> {
        // Synchronize all bound rows at exports. Raw field stores
        // change the live narrow-phase pose but never implicitly reinsert the
        // existing area links, matching the native explicit-link contract.
        for native_slot in 0..self.observed_count {
            self.quakec_project_row(services, memory, strings, context, native_slot)?;
        }
        Ok(())
    }
    pub(crate) fn quakec_project_row(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        native_slot: usize,
    ) -> Result<(), CallError> {
        let layout = self.quakec_layout()?;
        self.quakec_publish(
            services,
            memory,
            strings,
            context,
            (native_slot * layout.stride) as u32,
        )?;
        Ok(())
    }
    pub(crate) fn quakec_hit(&self, services: &EngineServices<'_>, hit: Option<EntityId>) -> u32 {
        let Some(entity) = hit.and_then(|entity| services.server.entities.resolve(entity)) else {
            return 0;
        };
        let Some(native) = services.server.entities.columns.native_entity[entity] else {
            return 0;
        };
        if Some(native.module) != self.owner {
            return 0;
        }
        let Ok(layout) = self.quakec_layout() else {
            return 0;
        };
        u32::try_from(native.slot)
            .ok()
            .and_then(|slot| (slot as usize).checked_mul(layout.stride))
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0)
    }
}
