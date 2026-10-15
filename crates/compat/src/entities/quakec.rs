//! QuakeC byte-offset edicts use the same namespace/lifetime projection as
//! native modules. Only ABI field storage and ordinal reuse live here.
use super::*;
use qa_core::primitives::{Motion, PlayerState, WeaponReadiness, WeaponState};
use qa_formats::program::quakec::Image;
use qa_world::entities::FullTable;

#[derive(Clone, Copy)]
struct ClientFields {
    health: Option<usize>,
    armor: Option<usize>,
    armor_type: Option<usize>,
    frags: Option<usize>,
    ammo: Option<usize>,
    ammo_counts: [Option<usize>; 4],
    items: Option<usize>,
    items2: Option<usize>,
    weapon: Option<usize>,
    weapon_model: Option<usize>,
    weapon_frame: Option<usize>,
    view_offset: Option<usize>,
    punch: Option<usize>,
    view_angles: Option<usize>,
    fix_angle: Option<usize>,
    water_level: Option<usize>,
    water_type: Option<usize>,
    water_jump: Option<usize>,
    button_attack: Option<usize>,
    button_jump: Option<usize>,
    impulse: Option<usize>,
    team: Option<usize>,
    net_name: Option<usize>,
    gravity: Option<usize>,
    max_speed: Option<usize>,
    touch: Option<usize>,
    stats: [Option<qa_core::primitives::ValueBinding>; 32],
    items_binding: Option<qa_core::primitives::ValueBinding>,
    extra_stats: [Option<u32>; 4],
    server_flags: u32,
    player_stats: Option<qa_gameplay::registry::NativePlayerBindings>,
}
impl ClientFields {
    fn load(field: impl Fn(&[u8], u16) -> Option<usize>) -> Self {
        Self {
            health: field(b"health", 2),
            armor: field(b"armorvalue", 2),
            armor_type: field(b"armortype", 2),
            frags: field(b"frags", 2),
            ammo: field(b"currentammo", 2),
            ammo_counts: [
                b"ammo_shells".as_slice(),
                b"ammo_nails",
                b"ammo_rockets",
                b"ammo_cells",
            ]
            .map(|name| field(name, 2)),
            items: field(b"items", 2),
            items2: field(b"items2", 2),
            weapon: field(b"weapon", 2),
            weapon_model: field(b"weaponmodel", 1),
            weapon_frame: field(b"weaponframe", 2),
            view_offset: field(b"view_ofs", 3),
            punch: field(b"punchangle", 3),
            view_angles: field(b"v_angle", 3),
            fix_angle: field(b"fixangle", 2),
            water_level: field(b"waterlevel", 2),
            water_type: field(b"watertype", 2),
            water_jump: field(b"teleport_time", 2),
            button_attack: field(b"button0", 2),
            button_jump: field(b"button2", 2),
            impulse: field(b"impulse", 2),
            team: field(b"team", 2),
            net_name: field(b"netname", 1),
            gravity: field(b"gravity", 2),
            max_speed: field(b"maxspeed", 2),
            touch: field(b"touch", 6),
            stats: [None; 32],
            items_binding: None,
            extra_stats: [None; 4],
            server_flags: 0,
            player_stats: None,
        }
    }
}

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
    // Native startup fixes these ordinals before any module callback.
    if rules == qa_core::primitives::RuleSetId::QuakeWorld {
        Ok(32)
    } else {
        usize::try_from(max_clients).map_err(|_| CallError::Entity)
    }
}

fn motion(value: f32) -> Motion {
    if value == 0.0 {
        Motion::None
    } else if value == 8.0 {
        Motion::Noclip
    } else {
        Motion::Unsupported
    }
}

fn integer_motion(value: f32) -> Motion {
    motion(crate::numbers::native_integer(f64::from(value)) as f32)
}

#[derive(Clone, Copy)]
pub(crate) struct QuakeCLayout {
    stride: usize,
    header: usize,
    fields: usize,
    client_edicts: Option<usize>,
    integer_motion: bool,
    motion_decode: Option<fn(f32) -> Motion>,
    model_rules: ModelRules,
    name_fields: [Option<usize>; 3],
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
    ground_entity: Option<usize>,
    attack_finished: Option<usize>,
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
    client: ClientFields,
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
            client_edicts: None,
            integer_motion: false,
            motion_decode: None,
            model_rules: qa_gameplay::rules::model_rules(RuleSetId::Quake),
            name_fields: [b"classname".as_slice(), b"target", b"targetname"]
                .map(|name| field(name, 1)),
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
            ground_entity: field(b"groundentity", 4),
            attack_finished: field(b"attack_finished", 2),
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
            client: ClientFields::load(field),
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
    fn motion_state(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
    ) -> Result<(Motion, Option<Vec3>), CallError> {
        let angular_velocity = self
            .angular_velocity
            .map(|field| self.vector(memory, address, Some(field)))
            .transpose()?;
        let motion = if let Some(field) = self.move_type {
            let decode = self.motion_decode.ok_or(CallError::Entity)?;
            decode(self.number(memory, address, Some(field))?)
        } else {
            Motion::Unsupported
        };
        Ok((motion, angular_velocity))
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
    fn read_client_pose(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        player: &mut PlayerState,
    ) -> Result<(), CallError> {
        let origin = self.vector(memory, address, self.origin)?;
        let velocity = self.vector(memory, address, self.velocity)?;
        player.body.position = origin;
        player.body.velocity = velocity;
        Ok(())
    }
    fn client_noclip(self, memory: &ModuleMemory<'_>, address: u64) -> Result<(), CallError> {
        if self.number(memory, address, self.move_type)? != 8.0
            || self.flags(memory, address)? & (1 << 11) != 0
        {
            return Err(CallError::Unsupported);
        }
        Ok(())
    }
    fn removal_words(self) -> [Option<usize>; 7] {
        [
            self.model,
            self.take_damage,
            self.model_index,
            self.colormap,
            self.skin,
            self.frame,
            self.solid,
        ]
    }
    fn removal_vectors(self) -> [Option<usize>; 2] {
        [self.origin, self.angles]
    }
    fn link_fields(self, memory: &ModuleMemory<'_>, address: u64) -> Result<(), CallError> {
        for field in [
            self.origin,
            self.angles,
            self.mins,
            self.maxs,
            self.abs_min,
            self.abs_max,
        ] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 12)?;
        }
        for field in [self.solid, self.flags, self.owner] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 4)?;
        }
        Ok(())
    }
}

pub(crate) struct QuakeCTraceView<'a, 'memory> {
    memory: &'a ModuleMemory<'memory>,
    strings: &'a ModuleMemory<'memory>,
    layout: QuakeCLayout,
    module: ModuleId,
    error: TraceError,
}
impl QuakeCTraceView<'_, '_> {
    pub fn finish(&self) -> Result<(), CallError> {
        self.error.finish()
    }
    fn address(&self, slot: i32) -> Result<u64, CallError> {
        let address = u32::try_from(slot)
            .ok()
            .and_then(|slot| slot.checked_mul(self.layout.stride as u32))
            .ok_or(CallError::Entity)?;
        self.layout.address(self.memory, address)
    }
}
impl qa_world::collision::NativeTraceEntities for QuakeCTraceView<'_, '_> {
    fn module(&self) -> ModuleId {
        self.module
    }
    fn owner(&self, slot: i32) -> Option<CollisionOwner> {
        self.error.capture(|| {
            self.layout
                .owner(self.memory, self.module, self.address(slot)?)
        })
    }
    fn fields(&self, slot: i32, point_contents: bool) -> Option<NativeTraceFields> {
        self.error.capture(|| {
            let address = self.address(slot)?;
            let solid = self.layout.solid(self.memory, address)?;
            if !point_contents && solid <= 1 {
                return Ok(NativeTraceFields::default());
            }
            Ok(NativeTraceFields {
                tags: if point_contents {
                    CollisionTags::default()
                } else {
                    QuakeCLayout::tags(self.layout.flags(self.memory, address)?)
                },
                enabled: point_contents || solid > 1,
                box_target: solid != 4,
                contents: Contents::SOLID,
            })
        })
    }
    fn bounds(&self, slot: i32, cached: Bounds) -> Option<Bounds> {
        self.error
            .capture(|| self.layout.bounds(self.memory, self.address(slot)?, cached))
    }
    fn target(
        &self,
        slot: i32,
        cached: NativeTraceTarget,
        point_contents: bool,
        store: &CollisionStore,
        geometry: qa_core::primitives::GeometryId,
        _names: Option<&dyn NativeModelNames>,
    ) -> Option<NativeTraceTarget> {
        self.error.capture(|| {
            let address = self.address(slot)?;
            self.layout.target(
                self.memory,
                self.strings,
                address,
                cached,
                store,
                Some(geometry),
                Some(point_contents),
            )
        })
    }
}

impl EntityProjection {
    pub(crate) fn quakec_client_count(&self) -> Result<usize, CallError> {
        self.quakec_layout()?.client_edicts.ok_or(CallError::Entity)
    }
    pub(crate) fn quakec_check_address(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<(), CallError> {
        self.quakec_layout()?.address(memory, value).map(|_| ())
    }
    pub(crate) fn quakec_client_address(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<u64, CallError> {
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let slot = value as usize / layout.stride;
        if slot == 0 || slot > self.quakec_client_count()? {
            return Err(CallError::Entity);
        }
        Ok(address)
    }
    pub(crate) fn quakec_bind_client(
        &mut self,
        entities: &EntityTable,
        native: NativeEntity,
        entity: EntityId,
    ) -> Result<(), CallError> {
        if native.slot <= 0 || native.slot as usize > self.quakec_client_count()? {
            return Err(CallError::Entity);
        }
        self.bind_client(entities, native, entity)
    }
    pub(crate) fn quakec_bind_player_values(
        &mut self,
        stats: &[qa_core::primitives::ValueBinding],
        items: Option<qa_core::primitives::ValueBinding>,
    ) -> Result<(), CallError> {
        self.quakec_client_count()?;
        let mut layout = self.quakec_layout()?;
        if stats.len() != layout.client.stats.len() {
            return Err(CallError::Capacity);
        }
        for (to, value) in layout.client.stats.iter_mut().zip(stats) {
            *to = Some(*value);
        }
        layout.client.items_binding = items;
        self.schema = EntitySchema::QuakeC(layout);
        Ok(())
    }
    pub(crate) fn quakec_client_globals(
        &mut self,
        flags: u32,
        stats: [Option<u32>; 4],
    ) -> Result<(), CallError> {
        let mut layout = self.quakec_layout()?;
        layout.client.server_flags = flags;
        layout.client.extra_stats = stats;
        self.schema = EntitySchema::QuakeC(layout);
        Ok(())
    }
    pub(crate) fn quakec_bind_client_registry(
        &mut self,
        registry: &qa_gameplay::registry::Registry,
        source: RuleSetId,
    ) -> Result<(), CallError> {
        self.quakec_client_count()?;
        let mut layout = self.quakec_layout()?;
        layout.client.player_stats =
            qa_gameplay::registry::NativePlayerBindings::load(source, registry);
        if source != RuleSetId::Quake {
            layout.client.items2 = None;
        }
        self.schema = EntitySchema::QuakeC(layout);
        Ok(())
    }
    fn quakec_client(
        &self,
        services: &EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(u64, qa_core::primitives::ClientId), CallError> {
        if !matches!(
            context.native_rules,
            RuleSetId::Quake | RuleSetId::QuakeWorld
        ) || self.owner != Some(context.module)
        {
            return Err(CallError::Unsupported);
        }
        let address = self.quakec_client_address(memory, value)?;
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self
            .binding(&services.server.entities, native)?
            .ok_or(CallError::Entity)?;
        let client = services
            .server
            .client_for_native(native)
            .ok_or(CallError::Entity)?;
        let row = &services.server.clients[client.0 as usize];
        if row.entity != entity || row.module != context.module {
            return Err(CallError::Entity);
        }
        Ok((address, client))
    }
    pub(crate) fn quakec_validate_client_spawn(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        if !matches!(
            context.native_rules,
            RuleSetId::Quake | RuleSetId::QuakeWorld
        ) || self.owner != Some(context.module)
        {
            return Err(CallError::Unsupported);
        }
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        for field in [layout.colormap, layout.client.team, layout.client.net_name] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 4)?;
        }
        Ok(())
    }
    pub(crate) fn quakec_client_spawn(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        name: u32,
        maxspeed: Option<f32>,
        colors: u8,
    ) -> Result<(), CallError> {
        let (address, client) = self.quakec_client(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        self.quakec_validate_client_spawn(memory, context, value)?;
        memory
            .read_mut(address + layout.header as u64, layout.fields)?
            .fill(0);
        layout.write_number(
            memory,
            address,
            layout.colormap,
            (value as usize / layout.stride) as f32,
        )?;
        let team = if context.native_rules == RuleSetId::Quake {
            f32::from((colors & 15) + 1)
        } else {
            0.0
        };
        layout.write_number(memory, address, layout.client.team, team)?;
        memory.write_word(
            address + layout.client.net_name.ok_or(CallError::Memory)? as u64,
            name as i32,
        )?;
        if let Some(maxspeed) = maxspeed {
            for (field, value) in [
                (layout.client.gravity, 1.0),
                (layout.client.max_speed, maxspeed),
            ] {
                if let Some(field) = field {
                    layout.write_number(memory, address, Some(field), value)?;
                }
            }
        }
        let (motion, angular_velocity) = layout.motion_state(memory, address)?;
        let slot = services.server.clients[client.0 as usize].entity.slot as usize;
        services.server.entities.columns.motion[slot] = motion;
        services.server.entities.columns.angular_velocity[slot] = angular_velocity;
        Ok(())
    }
    pub(crate) fn quakec_receive_client_command(
        &self,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        command: &qa_core::primitives::UserCmd,
    ) -> Result<(), CallError> {
        if !matches!(
            context.native_rules,
            RuleSetId::Quake | RuleSetId::QuakeWorld
        ) || self.owner != Some(context.module)
        {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        if context.native_rules == RuleSetId::Quake
            || layout.number(memory, address, layout.client.fix_angle)? == 0.0
        {
            layout.write_vector(
                memory,
                address,
                layout.client.view_angles,
                command.view_angles,
            )?;
        }
        layout.write_number(
            memory,
            address,
            layout.client.button_attack,
            (command.buttons & qa_core::primitives::buttons::ATTACK) as f32,
        )?;
        layout.write_number(
            memory,
            address,
            layout.client.button_jump,
            ((command.buttons & qa_core::primitives::buttons::JUMP) >> 1) as f32,
        )?;
        if command.impulse != 0 {
            layout.write_number(
                memory,
                address,
                layout.client.impulse,
                f32::from(command.impulse),
            )?;
        }
        Ok(())
    }
    pub(crate) fn quakec_client_command(
        &self,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        command: &qa_core::primitives::UserCmd,
        roll_angle: f32,
        roll_speed: f32,
    ) -> Result<(), CallError> {
        if context.native_rules != RuleSetId::QuakeWorld {
            return Err(CallError::Unsupported);
        }
        self.quakec_receive_client_command(memory, context, value, command)?;
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        // SV_RunCmd reads these fields after the preceding stores. QuakeC
        // definitions can overlap, including one angle store with the next
        // v_angle component, so preserve each scalar native read/write.
        if layout.number(memory, address, layout.client.health)? > 0.0 {
            if layout.number(memory, address, layout.client.fix_angle)? == 0.0 {
                let pitch = layout.number(memory, address, layout.client.view_angles)?;
                layout.write_number(memory, address, layout.angles, -pitch / 3.0)?;
                let yaw = layout.number(
                    memory,
                    address,
                    layout.client.view_angles.map(|field| field + 4),
                )?;
                layout.write_number(memory, address, layout.angles.map(|field| field + 4), yaw)?;
            }
            let angles = layout.vector(memory, address, layout.angles)?;
            let velocity = layout.vector(memory, address, layout.velocity)?;
            let roll = qa_gameplay::rules::appearance::AliasRoll {
                angle: roll_angle,
                speed: roll_speed,
            }
            .calculate(angles, velocity)
                * 4.0;
            layout.write_number(memory, address, layout.angles.map(|field| field + 8), roll)?;
        }
        Ok(())
    }
    pub(crate) fn quakec_client_phase(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<Motion, CallError> {
        self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        Ok(motion(crate::numbers::native_integer(layout.number(
            memory,
            address,
            layout.move_type,
        )?) as f32))
    }
    pub(crate) fn quakec_client_noclip(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        let address = self.quakec_client_address(memory, value)?;
        self.quakec_layout()?.client_noclip(memory, address)
    }
    pub(crate) fn quakec_prepare_control(
        &self,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        seconds: f64,
        roll: qa_gameplay::rules::appearance::AliasRoll,
    ) -> Result<Option<Vec3>, CallError> {
        self.quakec_identity(memory, context, value)?;
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        if layout.number(memory, address, layout.move_type)? == 0.0 {
            return Ok(None);
        }
        let mut punch = layout.vector(memory, address, layout.client.punch)?;
        let length = qa_core::math::normalize(&mut punch);
        layout.write_vector(memory, address, layout.client.punch, punch)?;
        let mut length = (f64::from(length) - 10.0 * seconds) as f32;
        if length < 0.0 {
            length = 0.0;
        }
        for axis in 0..3 {
            let field = layout.client.punch.map(|at| at + axis * 4);
            let value = layout.number(memory, address, field)? * length;
            layout.write_number(memory, address, field, value)?;
        }
        if layout.number(memory, address, layout.client.health)? <= 0.0 {
            return Ok(None);
        }
        let view = layout.vector(memory, address, layout.client.view_angles)?
            + layout.vector(memory, address, layout.client.punch)?;
        let body_angles = layout.vector(memory, address, layout.angles)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        layout.write_number(
            memory,
            address,
            layout.angles.map(|at| at + 8),
            roll.calculate(body_angles, velocity) * 4.0,
        )?;
        if layout.number(memory, address, layout.client.fix_angle)? == 0.0 {
            layout.write_number(memory, address, layout.angles, -view.0[0] / 3.0)?;
            layout.write_number(memory, address, layout.angles.map(|at| at + 4), view.0[1])?;
        }
        layout.client_noclip(memory, address)?;
        Ok(Some(layout.vector(memory, address, layout.angles)?))
    }
    fn quakec_client_lifetime(
        &self,
        entities: &EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        entity: EntityId,
    ) -> Result<u64, CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        if self.quakec_bound_entity(entities, native)? != Some(entity) {
            return Err(CallError::Entity);
        }
        self.quakec_client_address(memory, value)
    }
    pub(crate) fn quakec_read_frame_client(
        &self,
        entities: &EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        entity: EntityId,
        player: &mut PlayerState,
    ) -> Result<(), CallError> {
        let address = self.quakec_client_lifetime(entities, memory, context, value, entity)?;
        let layout = self.quakec_layout()?;
        let deadline = layout.number(memory, address, layout.client.water_jump)?;
        layout.read_client_pose(memory, address, player)?;
        player.movement.mode = qa_core::primitives::MovementMode::Noclip;
        player.movement.teleport_hold_until = f64::from(deadline);
        Ok(())
    }
    pub(crate) fn quakec_write_frame_scalar(
        &self,
        entities: &EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        entity: EntityId,
        player: &mut PlayerState,
        stage: qa_core::primitives::MotionWrite,
    ) -> Result<(), CallError> {
        let address = self.quakec_client_lifetime(entities, memory, context, value, entity)?;
        let layout = self.quakec_layout()?;
        let (axis, field, vector) = match stage {
            qa_core::primitives::MotionWrite::Origin(axis) => {
                (axis, layout.origin, player.body.position)
            }
            qa_core::primitives::MotionWrite::Velocity(axis) => {
                (axis, layout.velocity, player.body.velocity)
            }
            qa_core::primitives::MotionWrite::Angles(_) => return Err(CallError::Unsupported),
        };
        let value = *vector.0.get(axis).ok_or(CallError::Memory)?;
        layout.write_number(memory, address, field.map(|at| at + axis * 4), value)?;
        layout.read_client_pose(memory, address, player)
    }
    pub(crate) fn quakec_client_classname<'a>(
        &self,
        memory: &ModuleMemory<'_>,
        strings: &'a ModuleMemory<'_>,
        value: u32,
    ) -> Result<&'a [u8], CallError> {
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        let Some(field) = layout.name_fields[0] else {
            return Ok(b"");
        };
        let word = layout.word(memory, address, Some(field))?;
        Ok(strings.cstring(u64::from(word))?)
    }
    pub(crate) fn quakec_prepare_client_velocity(
        &self,
        entities: &mut EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let address = self.quakec_client_address(memory, value)?;
        let entity = self
            .quakec_bound_entity(entities, native)?
            .ok_or(CallError::Entity)?;
        let layout = self.quakec_layout()?;
        let origin = layout.vector(memory, address, layout.origin)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        let slot = entities.resolve(entity).ok_or(CallError::Entity)?;
        entities.columns.position[slot] = origin;
        entities.columns.velocity[slot] = velocity;
        Ok(())
    }
    pub(crate) fn quakec_read_client(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<Vec3, CallError> {
        let (address, client) = self.quakec_client(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        let origin = layout.vector(memory, address, layout.origin)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        let mins = layout.vector(memory, address, layout.mins)?;
        let view = layout.vector(memory, address, layout.client.view_angles)?;
        let health = layout.number(memory, address, layout.client.health)?;
        let water_jump = layout.number(memory, address, layout.client.water_jump)?;
        layout.number(memory, address, layout.client.water_level)?;
        layout.number(memory, address, layout.client.water_type)?;
        layout.flags(memory, address)?;
        let gravity = layout
            .client
            .gravity
            .map(|field| layout.number(memory, address, Some(field)))
            .transpose()?
            .unwrap_or(1.0);
        let maxspeed = layout
            .client
            .max_speed
            .map(|field| layout.number(memory, address, Some(field)))
            .transpose()?;
        self.quakec_publish(services, memory, strings, context, value)?;
        let player = &mut services.server.clients[client.0 as usize].player;
        let view_offset = player.view_offset;
        qa_movement::set_bounds(player);
        if layout.client.view_offset.is_some() {
            player.view_offset = view_offset;
        }
        let offset = mins - player.body.mins;
        player.body.position = origin + offset;
        player.body.velocity = velocity;
        player.view_angles = view;
        if player.movement.mode != qa_core::primitives::MovementMode::Spectator {
            player.movement.mode = if health <= 0.0 {
                qa_core::primitives::MovementMode::Dead
            } else {
                qa_core::primitives::MovementMode::Walk
            };
        }
        player.movement.water_jump_seconds = water_jump;
        player.movement.tuning.gravity_multiplier = gravity;
        player.movement.tuning.max_speed = maxspeed.map(Into::into);
        Ok(offset)
    }
    pub(crate) fn quakec_write_client(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        offset: Vec3,
    ) -> Result<(), CallError> {
        let (address, client) = self.quakec_client(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        layout.link_fields(memory, address)?;
        for field in [layout.velocity, layout.client.view_angles] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 12)?;
        }
        for field in [
            layout.client.water_jump,
            layout.client.water_level,
            layout.client.water_type,
            layout.ground_entity,
        ] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 4)?;
        }
        let player = &services.server.clients[client.0 as usize].player;
        let water_jump = player.movement.water_jump_seconds;
        let level = player.movement.water_level;
        let contents = Contents(player.movement.water_contents).to_q1();
        let grounded = player.movement.grounded;
        let ground = self.quakec_hit(services, player.movement.ground);
        let origin = player.body.position - offset;
        let velocity = player.body.velocity;
        let view = player.view_angles;
        let flags = layout.flags(memory, address)?;
        layout.write_number(memory, address, layout.client.water_jump, water_jump)?;
        layout.write_number(memory, address, layout.client.water_level, f32::from(level))?;
        layout.write_number(memory, address, layout.client.water_type, contents as f32)?;
        layout.write_number(
            memory,
            address,
            layout.flags,
            (if grounded { flags | 512 } else { flags & !512 }) as i32 as f32,
        )?;
        if grounded {
            memory.write_word(
                address + layout.ground_entity.ok_or(CallError::Memory)? as u64,
                ground as i32,
            )?;
        }
        layout.write_vector(memory, address, layout.origin, origin)?;
        layout.write_vector(memory, address, layout.velocity, velocity)?;
        layout.write_vector(memory, address, layout.client.view_angles, view)?;
        self.quakec_link(services, memory, strings, context, value)
    }
    pub(crate) fn quakec_touch_callback(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<Option<qa_core::primitives::CallbackId>, CallError> {
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let word = layout.word(memory, address, layout.client.touch)?;
        if word == 0 {
            Ok(None)
        } else {
            self.quakec_callback(word).map(Some)
        }
    }
    pub(crate) fn quakec_client_fixangle(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<Option<Vec3>, CallError> {
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        if layout.number(memory, address, layout.client.fix_angle)? == 0.0 {
            return Ok(None);
        }
        layout.vector(memory, address, layout.angles).map(Some)
    }
    pub(crate) fn quakec_clear_client_fixangle(
        &self,
        memory: &mut ModuleMemory<'_>,
        value: u32,
    ) -> Result<(), CallError> {
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        layout.write_number(memory, address, layout.client.fix_angle, 0.0)
    }
    pub(crate) fn quakec_trace_view<'a, 'memory>(
        &self,
        memory: &'a ModuleMemory<'memory>,
        strings: &'a ModuleMemory<'memory>,
        context: CallContext,
        pass: u32,
    ) -> Result<(QuakeCTraceView<'a, 'memory>, CollisionOwner), CallError> {
        let pass = self.quakec_identity(memory, context, pass)?;
        Ok((
            QuakeCTraceView {
                memory,
                strings,
                layout: self.quakec_layout()?,
                module: context.module,
                error: TraceError::default(),
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
            memory: EntityMemory::VmOwned,
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
    pub(crate) fn quakec_seed(
        &mut self,
        entities: &EntityTable,
        module: ModuleId,
        rules: RuleSetId,
        max_clients: i32,
    ) -> Result<(), CallError> {
        let mut layout = self.quakec_layout()?;
        let clients = client_edicts(rules, max_clients)?;
        let integer_motion = rules == RuleSetId::QuakeWorld;
        let first = clients.checked_add(1).ok_or(CallError::Capacity)?;
        if first > self.bindings.len() {
            return Err(CallError::Capacity);
        }
        if layout
            .client_edicts
            .is_some_and(|bound| bound != clients || layout.integer_motion != integer_motion)
        {
            return Err(CallError::Entity);
        }
        self.seed(entities, module)?;
        layout.client_edicts = Some(clients);
        layout.integer_motion = integer_motion;
        layout.model_rules = qa_gameplay::rules::model_rules(rules);
        layout.motion_decode = Some(if integer_motion {
            self::integer_motion
        } else {
            motion
        });
        self.schema = EntitySchema::QuakeC(layout);
        self.observed_count = self.observed_count.max(first);
        Ok(())
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
    pub(crate) fn quakec_callback(
        &self,
        function: u32,
    ) -> Result<qa_core::primitives::CallbackId, CallError> {
        let thinks = self.quakec_thinks.as_ref().ok_or(CallError::Entity)?;
        let binding = thinks
            .functions
            .get(function as usize)
            .ok_or(CallError::Entity)?;
        if binding.path == thinks.module.think(None).path {
            return Err(CallError::Entity);
        }
        binding.callback.ok_or(CallError::Entity)
    }
    pub(crate) fn quakec_world_fields(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<[u64; 4], CallError> {
        if value != 0 || self.owner.is_none() {
            return Err(CallError::Entity);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        Ok([
            address + layout.model.ok_or(CallError::Memory)? as u64,
            address + layout.model_index.ok_or(CallError::Memory)? as u64,
            address + layout.solid.ok_or(CallError::Memory)? as u64,
            address + layout.move_type.ok_or(CallError::Memory)? as u64,
        ])
    }
    pub(crate) fn quakec_physics_entity(
        &self,
        entities: &EntityTable,
        cursor: &mut usize,
    ) -> Result<Option<crate::quakec::PhysicsEntity>, CallError> {
        let layout = self.quakec_layout()?;
        let clients = layout.client_edicts.ok_or(CallError::Entity)?;
        while let Some((native, entity)) = self.quakec_next_entity(entities, cursor)? {
            let Some(entity) = entity else {
                continue;
            };
            let native_slot = native.slot as usize;
            let client = native_slot > 0 && native_slot <= clients;
            if client && layout.integer_motion {
                continue;
            }
            let phase = if !client {
                entities.columns.motion[entity.slot as usize]
            } else {
                Motion::Unsupported
            };
            return Ok(Some(crate::quakec::PhysicsEntity {
                entity,
                native,
                phase,
            }));
        }
        Ok(None)
    }
    pub(crate) fn quakec_find_start(
        &self,
        module: ModuleId,
        ordinal: u32,
    ) -> Result<usize, CallError> {
        if self.owner != Some(module) || ordinal as usize >= self.observed_count {
            return Err(CallError::Entity);
        }
        Ok(ordinal as usize + 1)
    }
    pub(crate) fn quakec_bound_entity(
        &self,
        entities: &EntityTable,
        native: NativeEntity,
    ) -> Result<Option<EntityId>, CallError> {
        Ok(self
            .binding(entities, native)?
            .filter(|entity| entities.columns.owner[entity.slot as usize] == native.module))
    }
    pub(crate) fn quakec_next_entity(
        &self,
        entities: &EntityTable,
        cursor: &mut usize,
    ) -> Result<Option<(NativeEntity, Option<EntityId>)>, CallError> {
        let clients = self
            .quakec_layout()?
            .client_edicts
            .ok_or(CallError::Entity)?;
        let module = self.owner.ok_or(CallError::Entity)?;
        while *cursor < self.observed_count {
            let slot = *cursor;
            *cursor += 1;
            let native = NativeEntity {
                module,
                slot: slot as i32,
            };
            let entity = self.quakec_bound_entity(entities, native)?;
            // Native client edicts remain allocated when disconnected. They
            // are ABI-visible without inventing a common entity lifetime.
            if entity.is_some() || (slot > 0 && slot <= clients) {
                return Ok(Some((native, entity)));
            }
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
        let clients = layout.client_edicts.ok_or(CallError::Entity)?;
        let first = clients.checked_add(1).ok_or(CallError::Capacity)?;
        if first >= self.bindings.len() {
            return Err(CallError::Capacity);
        }
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
        let entity = self.bind(services, claim, slot, None)?;
        self.observed_count = self.observed_count.max(slot + 1);
        let address = slot.checked_mul(layout.stride).ok_or(CallError::Memory)?;
        // ED_ClearEdict clears only entvars. Boundary metadata does not inhabit
        // VM bytes and common lifetime liveness remains in EntityTable.
        memory
            .read_mut((address + layout.header) as u64, layout.fields)?
            .fill(0);
        let (motion, angular_velocity) = layout.motion_state(memory, address as u64)?;
        let common_slot = entity.slot as usize;
        let columns = &mut services.server.entities.columns;
        columns.motion[common_slot] = motion;
        columns.angular_velocity[common_slot] = angular_velocity;
        u32::try_from(address).map_err(|_| CallError::Memory)
    }
    fn quakec_removal(
        &self,
        services: &EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(NativeEntity, EntityId), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self
            .binding(&services.server.entities, native)?
            .ok_or(CallError::Entity)?;
        let binding = self.bindings[native.slot as usize].ok_or(CallError::Entity)?;
        if native.slot == 0 || !binding.owned || !services.server.entities.can_release(entity) {
            return Err(CallError::Entity);
        }
        let layout = self.quakec_layout()?;
        for field in layout
            .removal_words()
            .into_iter()
            .chain([layout.next_think])
        {
            memory.read(u64::from(value) + field.ok_or(CallError::Memory)? as u64, 4)?;
        }
        for field in layout.removal_vectors() {
            memory.read(
                u64::from(value) + field.ok_or(CallError::Memory)? as u64,
                12,
            )?;
        }
        Ok((native, entity))
    }

    pub(crate) fn quakec_remove(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let (native, entity) = self.quakec_removal(services, memory, context, value)?;
        self.quakec_free_validated(services, memory, context, value, native, entity)
    }

    fn quakec_free_validated(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        native: NativeEntity,
        entity: EntityId,
    ) -> Result<(), CallError> {
        let slot = native.slot as usize;
        let layout = self.quakec_layout()?;
        (ENGINE_CALLS.free)(services, context, entity)?;
        self.bindings[slot]
            .as_mut()
            .ok_or(CallError::Entity)?
            .freed_at = Some(context.allocation.reuse.freetime(context.clock));
        for field in layout.removal_words() {
            layout.write_number(memory, value as u64, field, 0.0)?;
        }
        for field in layout.removal_vectors() {
            layout.write_vector(memory, value as u64, field, Vec3::default())?;
        }
        layout.write_number(memory, value as u64, layout.next_think, -1.0)
    }
    pub(crate) fn quakec_make_static(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        use qa_core::{
            math::{AngleByteForm, angle_to_byte, byte_to_angle, narrow_eighth},
            primitives::EntityRecord,
        };
        let (native, entity) = self.quakec_removal(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let model = strings.cstring(u64::from(layout.word(memory, address, layout.model)?))?;
        let models = services
            .storage
            .resources(context.module)
            .ok_or(CallError::ConfigString)?[0];
        let ordinal = (ENGINE_CALLS.resource_find)(services, context.module, models, model)?
            .ok_or(CallError::Geometry)?;
        let byte = |field| -> Result<u8, CallError> {
            Ok(
                crate::numbers::native_integer(f64::from(layout.number(memory, address, field)?))
                    as u8,
            )
        };
        let angle_form = if context.native_rules == RuleSetId::QuakeWorld {
            AngleByteForm::Float
        } else {
            AngleByteForm::Integral
        };
        let record = EntityRecord {
            position: Vec3(
                layout
                    .vector(memory, address, layout.origin)?
                    .0
                    .map(narrow_eighth),
            ),
            angles: Vec3(
                layout
                    .vector(memory, address, layout.angles)?
                    .0
                    .map(|angle| byte_to_angle(angle_to_byte(angle, angle_form) as i8 as i32)),
            ),
            appearance: EntityAppearance {
                source: context.native_rules,
                models: [
                    ModelRef::native(
                        ResourceNamespace::Module(context.module),
                        ordinal as u8 as u32,
                    ),
                    None,
                    None,
                    None,
                ],
                frame: i32::from(byte(layout.frame)?),
                skin: i32::from(byte(layout.skin)?),
                native: NativeAppearanceTail {
                    colormap: u32::from(byte(layout.colormap)?),
                    ..NativeAppearanceTail::default()
                },
                ..EntityAppearance::default()
            },
            ..EntityRecord::default()
        };
        match (ENGINE_CALLS.static_entity)(services, context, record) {
            Ok(_) | Err(CallError::Capacity) => {
                self.quakec_free_validated(services, memory, context, value, native, entity)
            }
            Err(error) => Err(error),
        }
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
    pub(crate) fn quakec_drop_to_floor(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<bool, CallError> {
        use qa_world::collision::{QuakeTraceKind, TraceQuery, trace_policy};
        let layout = self.quakec_layout()?;
        let origin = layout.vector(memory, u64::from(value), layout.origin)?;
        let mins = layout.vector(memory, u64::from(value), layout.mins)?;
        let maxs = layout.vector(memory, u64::from(value), layout.maxs)?;
        let mut end = origin;
        end.0[2] -= 256.0;
        let (rules, entity_rules) = trace_policy(context.trace_rules);
        let mut query = TraceQuery::point(
            origin,
            end,
            rules,
            entity_rules.with_quake_kind(Some(QuakeTraceKind::Normal)),
        );
        query.mins = mins;
        query.maxs = maxs;
        let (view, pass) = self.quakec_trace_view(memory, strings, context, value)?;
        query.pass = pass;
        let trace = (ENGINE_CALLS.trace)(services, query, Some(&view))?;
        view.finish()?;
        if trace.fraction == 1.0 || trace.all_solid {
            return Ok(false);
        }
        let ground_address =
            u64::from(value) + layout.ground_entity.ok_or(CallError::Memory)? as u64;
        let flags_address = u64::from(value) + layout.flags.ok_or(CallError::Memory)? as u64;
        memory.read(ground_address, 4)?;
        memory.read(flags_address, 4)?;
        if value != 0 {
            layout.link_fields(memory, u64::from(value))?;
        }
        self.quakec_set_origin(services, memory, strings, context, value, trace.end)?;
        let flags = layout.flags(memory, u64::from(value))? as i32 | 512;
        layout.write_number(memory, u64::from(value), layout.flags, flags as f32)?;
        let ground = self.quakec_hit(services, trace.entity);
        memory.write_word(ground_address, ground as i32)?;
        Ok(true)
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
                inline_model(
                    services.geometry,
                    services.world.ok_or(CallError::Geometry)?.0,
                    text,
                )?
                .1,
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
    pub(crate) fn quakec_prepare_noclip(
        &mut self,
        entities: &mut EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self.binding(entities, native)?.ok_or(CallError::Entity)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let angles = layout.vector(memory, address, layout.angles)?;
        let angular_velocity = layout.vector(memory, address, layout.angular_velocity)?;
        let slot = entity.slot as usize;
        entities.columns.angles[slot] = angles;
        entities.columns.angular_velocity[slot] = Some(angular_velocity);
        Ok(())
    }
    pub(crate) fn quakec_write_noclip(
        &mut self,
        entities: &mut EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        stage: qa_session::physics::MotionWrite,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self.binding(entities, native)?.ok_or(CallError::Entity)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let slot = entity.slot as usize;
        let (axis, field, vector) = match stage {
            qa_session::physics::MotionWrite::Angles(axis) => {
                (axis, layout.angles, entities.columns.angles[slot])
            }
            qa_session::physics::MotionWrite::Origin(axis) => {
                (axis, layout.origin, entities.columns.position[slot])
            }
            qa_session::physics::MotionWrite::Velocity(axis) => {
                (axis, layout.velocity, entities.columns.velocity[slot])
            }
        };
        let value = *vector.0.get(axis).ok_or(CallError::Memory)?;
        layout.write_number(memory, address, field.map(|at| at + axis * 4), value)?;
        // VectorMA writes each component before reading the next. Refresh the
        // same common columns after writes that can alias later native input.
        match stage {
            qa_session::physics::MotionWrite::Angles(axis) if axis < 2 => {
                let angles = layout.vector(memory, address, layout.angles)?;
                let velocity = layout.vector(memory, address, layout.angular_velocity)?;
                entities.columns.angles[slot] = angles;
                entities.columns.angular_velocity[slot] = Some(velocity);
            }
            qa_session::physics::MotionWrite::Angles(_)
            | qa_session::physics::MotionWrite::Origin(_)
            | qa_session::physics::MotionWrite::Velocity(_) => {
                let origin = layout.vector(memory, address, layout.origin)?;
                let velocity = layout.vector(memory, address, layout.velocity)?;
                entities.columns.position[slot] = origin;
                entities.columns.velocity[slot] = velocity;
            }
        }
        Ok(())
    }
    pub(crate) fn quakec_relink_entity(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
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
        layout.link_fields(memory, u64::from(value))?;
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
        let (motion, angular_velocity) = layout.motion_state(memory, address)?;
        let weapon_state = layout
            .attack_finished
            .map(|field| {
                layout
                    .number(memory, address, Some(field))
                    .map(|deadline| WeaponState {
                        source: context.native_rules,
                        namespace: ResourceNamespace::Module(context.module),
                        readiness: WeaponReadiness::DeadlineSeconds(deadline),
                    })
            })
            .transpose()?;
        let mut names = [None; 3];
        for (field, name) in layout.name_fields.into_iter().zip(&mut names) {
            if let Some(field) = field {
                let string = layout.word(memory, address, Some(field))?;
                let bytes = strings.cstring(u64::from(string))?;
                *name = Some(
                    services
                        .names
                        .intern(bytes)
                        .map_err(|_| CallError::Capacity)?,
                );
            }
        }
        let columns = &services.server.entities.columns;
        let target = layout.target(
            memory,
            strings,
            address,
            NativeTraceTarget {
                body: columns.body(slot),
                angles: columns.angles[slot],
                shape: columns.collision_shape[slot],
                rules: columns.model_rules[slot],
                contents_pose: columns.point_contents_pose[slot],
            },
            services.geometry,
            services.world.map(|world| world.0),
            None,
        )?;
        let body = target.body;
        let angles = target.angles;
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
        let solid = layout.solid(memory, address)?;
        let shape = target.shape;
        let owner = if layout.owner.is_some() {
            layout.owner(memory, context.module, address)?
        } else {
            CollisionOwner::None
        };
        let mut appearance = EntityAppearance {
            source: context.native_rules,
            snapshot_excluded: false,
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
        self.quakec_publish_client(services, memory, strings, context, address, native)?;
        if let Some(client) = services.server.client_for_native(native) {
            let provider_motion = context.native_rules == RuleSetId::Quake
                && services.server.module_moves(client) == Some(native);
            let client = &mut services.server.clients[client.0 as usize];
            if client.module == context.module {
                client.player.weapon_state = weapon_state;
                // NQ provider physics publishes the native pose. QW packet
                // pmove retains its selected-bounds conversion in read_client.
                if provider_motion {
                    client.player.body = body;
                }
            }
        }
        services.server.entities.set_targetname(entity, names[2]);
        let tags = services
            .server
            .entities
            .collision_tags(entity, QuakeCLayout::tags(flags))
            .ok_or(CallError::Entity)?;
        let columns = &mut services.server.entities.columns;
        columns.classname[slot] = names[0];
        columns.target[slot] = names[1];
        columns.set_body(slot, body);
        columns.angles[slot] = angles;
        columns.angular_velocity[slot] = angular_velocity;
        columns.motion[slot] = motion;
        columns.flags[slot] = flags;
        columns.collision_shape[slot] = shape;
        columns.collision_owner[slot] = owner;
        columns.model_rules[slot] = layout.model_rules;
        columns.collision_tags[slot] = tags;
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
    fn quakec_publish_client(
        &self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        address: u64,
        native: NativeEntity,
    ) -> Result<(), CallError> {
        let Some(id) = services.server.client_for_native(native) else {
            return Ok(());
        };
        if services.server.clients[id.0 as usize].module != context.module {
            return Ok(());
        }
        let layout = self.quakec_layout()?;
        let fields = layout.client;
        let number = |field: Option<usize>| {
            field
                .map(|at| layout.number(memory, address, Some(at)))
                .transpose()
        };
        let integer = |field| number(field).map(|value| value.map(crate::numbers::native_integer));
        let health = number(fields.health)?;
        let armor = integer(fields.armor)?;
        let armor_type = number(fields.armor_type)?;
        let frags = integer(fields.frags)?;
        let items = integer(fields.items)?.map(|value| value as u32);
        let items2 = integer(fields.items2)?.map(|value| value as u32);
        let weapon = integer(fields.weapon)?.map(|value| value as u32);
        let ammo = fields.ammo_counts.map(integer);
        let current_ammo = integer(fields.ammo)?;
        let weapon_model = if let Some(field) = fields.weapon_model {
            let word = layout.word(memory, address, Some(field))?;
            let path = strings.cstring(u64::from(word))?;
            if path.is_empty() {
                Some(0)
            } else {
                let range = services
                    .storage
                    .resources(context.module)
                    .ok_or(CallError::ConfigString)?[0];
                Some(
                    (ENGINE_CALLS.resource_find)(services, context.module, range, path)?
                        .ok_or(CallError::Geometry)?,
                )
            }
        } else {
            None
        };
        let weapon_frame = integer(fields.weapon_frame)?;
        let view_offset = fields
            .view_offset
            .map(|field| layout.vector(memory, address, Some(field)))
            .transpose()?;
        let punch = fields
            .punch
            .map(|field| layout.vector(memory, address, Some(field)))
            .transpose()?;
        let view = fields
            .view_angles
            .map(|field| layout.vector(memory, address, Some(field)))
            .transpose()?;
        let water_level = integer(fields.water_level)?;
        let water_type = integer(fields.water_type)?;
        let ground_flags = layout
            .flags
            .map(|_| layout.flags(memory, address))
            .transpose()?;
        let ground = if ground_flags.is_some_and(|flags| flags & 512 != 0)
            && layout.ground_entity.is_some()
        {
            let word = layout.word(memory, address, layout.ground_entity)?;
            let ground = layout.identity(memory, context.module, word)?;
            self.binding(&services.server.entities, ground)?
        } else {
            None
        };
        let mut stats = [None; 32];
        if let Some(health) = health {
            stats[0] = Some(crate::numbers::native_integer(health) as u32);
        }
        if let Some(value) = frags {
            stats[1] = Some(value as u32);
        }
        if let Some(value) = weapon_frame {
            stats[5] = Some(value as u32);
        }
        if let Some(model) = weapon_model {
            stats[2] = Some(model);
        }
        if let Some(value) = current_ammo {
            stats[3] = Some(value as u32);
        }
        if let Some(value) = armor {
            stats[4] = Some(value as u32);
        }
        let mut ammo_values = [None; 4];
        for (index, value) in ammo.into_iter().enumerate() {
            ammo_values[index] = value?;
            if let Some(value) = ammo_values[index] {
                stats[6 + index] = Some(value as u32);
            }
        }
        if let Some(value) = weapon {
            stats[10] = Some(value);
        }
        stats[11..15].copy_from_slice(&fields.extra_stats);
        let items = items.map(|items| {
            items
                | items2.map_or_else(
                    || fields.server_flags.wrapping_shl(28),
                    |items2| items2.wrapping_shl(23),
                )
        });
        let player = &mut services.server.clients[id.0 as usize].player;
        if fields
            .stats
            .into_iter()
            .flatten()
            .any(|binding| binding.id.0 as usize >= player.values.capacity())
        {
            return Err(CallError::Capacity);
        }
        if fields
            .items_binding
            .is_some_and(|binding| binding.id.0 as usize >= player.values.capacity())
        {
            return Err(CallError::Capacity);
        }
        if let Some(native_stats) = fields.player_stats {
            if !native_stats.fits(player) {
                return Err(CallError::Capacity);
            }
            for (index, (binding, bits)) in fields.stats.into_iter().zip(stats).enumerate() {
                let Some(bits) = bits else { continue };
                if !native_stats.import(
                    binding,
                    ResourceNamespace::Module(context.module),
                    index,
                    bits,
                    player,
                    services.server.world_time,
                ) {
                    return Err(CallError::Capacity);
                }
            }
            if let Some(items) = items
                && !native_stats.import_items(
                    fields.items_binding,
                    items,
                    player,
                    services.server.world_time,
                )
            {
                return Err(CallError::Capacity);
            }
        }
        if let Some(value) = armor_type {
            player.armor_absorption = value;
            player.armor_energy_absorption = value;
        }
        if let Some(view) = view_offset {
            player.view_offset = view;
        }
        if let Some(punch) = punch {
            player.punch_angles = punch;
        }
        if let Some(view) = view {
            player.view_angles = view;
        }
        if let Some(level) = water_level {
            player.movement.water_level = level as u8;
        }
        if let Some(contents) = water_type {
            player.movement.water_contents = Contents::from_q1(contents).0;
        }
        if let Some(flags) = ground_flags {
            player.flags = flags;
            player.movement.grounded = flags & 512 != 0;
            player.movement.ground = ground;
        }
        Ok(())
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

impl QuakeCLayout {
    fn solid(self, memory: &ModuleMemory<'_>, address: u64) -> Result<i32, CallError> {
        self.solid.map_or(Ok(0), |field| {
            self.number(memory, address, Some(field))
                .map(|value| crate::numbers::native_integer(f64::from(value)))
        })
    }
    fn bounds(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        mut bounds: Bounds,
    ) -> Result<Bounds, CallError> {
        if self.mins.is_some() {
            bounds.mins = self.vector(memory, address, self.mins)?;
        }
        if self.maxs.is_some() {
            bounds.maxs = self.vector(memory, address, self.maxs)?;
        }
        bounds.is_valid().then_some(bounds).ok_or(CallError::Entity)
    }
    #[expect(
        clippy::too_many_arguments,
        reason = "One native row decoder serves publication and operation-specific live queries"
    )]
    fn target(
        self,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        address: u64,
        mut cached: NativeTraceTarget,
        store: &CollisionStore,
        geometry: Option<qa_core::primitives::GeometryId>,
        point_contents: Option<bool>,
    ) -> Result<NativeTraceTarget, CallError> {
        let solid = self.solid(memory, address)?;
        for (field, to) in [
            (self.origin, &mut cached.body.position),
            (self.velocity, &mut cached.body.velocity),
        ] {
            if field.is_some() {
                *to = self.vector(memory, address, field)?;
            }
        }
        let bounds = self.bounds(
            memory,
            address,
            Bounds {
                mins: cached.body.mins,
                maxs: cached.body.maxs,
            },
        )?;
        cached.body.mins = bounds.mins;
        cached.body.maxs = bounds.maxs;
        if point_contents.is_none() || solid == 4 {
            if self.angles.is_some() {
                cached.angles = self.vector(memory, address, self.angles)?;
            }
        } else {
            cached.angles = Vec3::default();
        }
        if !cached.body.position.is_finite() || !cached.angles.is_finite() {
            return Err(CallError::Entity);
        }
        cached.shape = match solid {
            0 | 1 if point_contents == Some(true) => CollisionShape::Box,
            0 | 1 => CollisionShape::None,
            2 | 3 => CollisionShape::Box,
            4 if address == 0 => CollisionShape::None,
            4 => {
                inline_model(
                    store,
                    geometry.ok_or(CallError::Geometry)?,
                    strings.cstring(u64::from(self.word(memory, address, self.model)?))?,
                )?
                .0
            }
            _ => return Err(CallError::Entity),
        };
        cached.rules = self.model_rules;
        cached.contents_pose = None;
        Ok(cached)
    }
}
