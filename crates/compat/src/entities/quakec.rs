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
    water_jump: Option<usize>,
    old_origin: Option<usize>,
    move_direction: Option<usize>,
    button_attack: Option<usize>,
    button_jump: Option<usize>,
    impulse: Option<usize>,
    team: Option<usize>,
    net_name: Option<usize>,
    max_speed: Option<usize>,
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
            water_jump: field(b"teleport_time", 2),
            old_origin: field(b"oldorigin", 3),
            move_direction: field(b"movedir", 3),
            button_attack: field(b"button0", 2),
            button_jump: field(b"button2", 2),
            impulse: field(b"impulse", 2),
            team: field(b"team", 2),
            net_name: field(b"netname", 1),
            max_speed: field(b"maxspeed", 2),
            stats: [None; 32],
            items_binding: None,
            extra_stats: [None; 4],
            server_flags: 0,
            player_stats: None,
        }
    }
}

struct ClientEnvironment {
    flags: Option<u32>,
    ground: Option<EntityId>,
    water_level: Option<u8>,
    water_contents: Option<u64>,
}
struct ClientEnvironmentWrite {
    water_level: u8,
    water_type: i32,
    flags: Option<u32>,
    ground: Option<u32>,
}
fn apply_client_pose(
    player: &mut PlayerState,
    origin: Vec3,
    velocity: Vec3,
    native_mins: Option<Vec3>,
) -> Vec3 {
    let offset = native_mins.map(|mins| mins - player.body.mins);
    player.body.position = offset.map_or(origin, |offset| origin + offset);
    player.body.velocity = velocity;
    offset.unwrap_or_default()
}
fn apply_client_mode(player: &mut PlayerState, mode: Option<qa_core::primitives::MovementMode>) {
    if player.movement.mode != qa_core::primitives::MovementMode::Spectator
        && let Some(mode) = mode
    {
        player.movement.mode = mode;
    }
}
impl ClientEnvironment {
    fn apply(self, player: &mut PlayerState) {
        if let Some(level) = self.water_level {
            player.movement.water_level = level;
        }
        if let Some(contents) = self.water_contents {
            player.movement.water_contents = contents;
        }
        if let Some(flags) = self.flags {
            player.flags = flags;
            player.movement.grounded = flags & 512 != 0;
            player.movement.ground = self.ground;
        }
    }
}

#[derive(Clone, Copy)]
struct MotionFields {
    gravity: Option<usize>,
    water_level: Option<usize>,
    water_type: Option<usize>,
    ground_entity: Option<usize>,
    touch: Option<usize>,
    ideal_yaw: Option<usize>,
    yaw_speed: Option<usize>,
    enemy: Option<usize>,
    goal: Option<usize>,
    local_time: Option<usize>,
    blocked: Option<usize>,
}
impl MotionFields {
    fn load(field: impl Fn(&[u8], u16) -> Option<usize>) -> Self {
        Self {
            gravity: field(b"gravity", 2),
            water_level: field(b"waterlevel", 2),
            water_type: field(b"watertype", 2),
            ground_entity: field(b"groundentity", 4),
            touch: field(b"touch", 6),
            ideal_yaw: field(b"ideal_yaw", 2),
            yaw_speed: field(b"yaw_speed", 2),
            enemy: field(b"enemy", 4),
            goal: field(b"goalentity", 4),
            local_time: field(b"ltime", 2),
            blocked: field(b"blocked", 6),
        }
    }
}

struct QuakeCSupport<'a, 'engine> {
    services: &'a mut EngineServices<'engine>,
    view: &'a dyn qa_world::collision::NativeTraceEntities,
    pass: CollisionOwner,
    rules: qa_world::collision::TraceRules,
    entities: qa_world::collision::EntityTracePolicy,
}
impl qa_session::physics::SupportQueries for QuakeCSupport<'_, '_> {
    type Error = CallError;

    fn solid(&mut self, point: Vec3) -> Result<bool, CallError> {
        Ok((ENGINE_CALLS.point_contents)(
            self.services,
            point,
            self.entities,
            self.pass,
            Some(self.view),
        )?
        .to_q1()
            == -2)
    }
    fn trace(&mut self, start: Vec3, end: Vec3) -> Result<qa_world::collision::Trace, CallError> {
        let mut query =
            qa_world::collision::TraceQuery::point(start, end, self.rules, self.entities);
        query.pass = self.pass;
        (ENGINE_CALLS.trace)(self.services, query, Some(self.view))
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
    fn binding(&self, word: u32) -> qa_core::primitives::ThinkBinding {
        self.functions
            .get(word as usize)
            .copied()
            .unwrap_or_else(|| Self::missing(self.module, word))
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
    } else if value == 4.0 {
        Motion::Step
    } else if value == 5.0 {
        Motion::Fly
    } else if value == 6.0 {
        Motion::Toss
    } else if value == 7.0 {
        Motion::Push
    } else if value == 8.0 {
        Motion::Noclip
    } else if value == 9.0 {
        Motion::FlyMissile
    } else if value == 10.0 {
        Motion::Bounce
    } else {
        Motion::Unsupported
    }
}

fn integer_motion(value: f32) -> Motion {
    motion(qa_core::math::native_integer(f64::from(value)) as f32)
}

fn client_motion(value: f32) -> Motion {
    let value = qa_core::math::native_integer(value);
    if value == 3 {
        Motion::Walk
    } else {
        motion(value as f32)
    }
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
    alias_private: [Option<usize>; 2],
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
    motion: MotionFields,
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
            alias_private: [field(b"chain", 4), field(b"invincible_sound", 2)],
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
            motion: MotionFields::load(&field),
            client: ClientFields::load(field),
        }
    }
    fn foreign_fields(self) -> Box<[crate::quakec::ForeignWord]> {
        use crate::quakec::ForeignWord;
        let mut fields = vec![ForeignWord::Unsupported; self.fields / 4];
        for (offset, width, first) in [
            (self.origin, 3, ForeignWord::Position(0)),
            (self.velocity, 3, ForeignWord::Velocity(0)),
            (self.angles, 3, ForeignWord::Angles(0)),
            (self.mins, 3, ForeignWord::Min(0)),
            (self.maxs, 3, ForeignWord::Max(0)),
            (self.size, 3, ForeignWord::Size(0)),
            (self.abs_min, 3, ForeignWord::AbsoluteMin(0)),
            (self.abs_max, 3, ForeignWord::AbsoluteMax(0)),
            (self.name_fields[0], 1, ForeignWord::Classname),
            (self.alias_private[0], 1, ForeignWord::Private),
            (self.alias_private[1], 1, ForeignWord::Private),
        ] {
            let Some(offset) = offset.map(|offset| (offset - self.header) / 4) else {
                continue;
            };
            for axis in 0..width {
                let word = match first {
                    ForeignWord::Position(_) => ForeignWord::Position(axis),
                    ForeignWord::Velocity(_) => ForeignWord::Velocity(axis),
                    ForeignWord::Angles(_) => ForeignWord::Angles(axis),
                    ForeignWord::Min(_) => ForeignWord::Min(axis),
                    ForeignWord::Max(_) => ForeignWord::Max(axis),
                    ForeignWord::Size(_) => ForeignWord::Size(axis),
                    ForeignWord::AbsoluteMin(_) => ForeignWord::AbsoluteMin(axis),
                    ForeignWord::AbsoluteMax(_) => ForeignWord::AbsoluteMax(axis),
                    value => value,
                };
                if let Some(target) = fields.get_mut(offset + axis) {
                    *target = if *target == ForeignWord::Unsupported || *target == word {
                        word
                    } else {
                        ForeignWord::Conflict
                    };
                }
            }
        }
        fields.into_boxed_slice()
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
    fn flags(self, memory: &ModuleMemory<'_>, address: u64) -> Result<u32, CallError> {
        Ok(
            qa_core::math::native_integer(f64::from(self.number(memory, address, self.flags)?))
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
    fn refresh_motion(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        entities: &mut EntityTable,
        entity: EntityId,
    ) -> Result<(), CallError> {
        let slot = entities.resolve(entity).ok_or(CallError::Entity)?;
        use qa_core::primitives::EntityFields;
        for (field, target, authority) in [
            (
                self.origin,
                &mut entities.columns.position[slot],
                EntityFields::POSITION,
            ),
            (
                self.velocity,
                &mut entities.columns.velocity[slot],
                EntityFields::VELOCITY,
            ),
            (
                self.angles,
                &mut entities.columns.angles[slot],
                EntityFields::ANGLES,
            ),
            (
                self.mins,
                &mut entities.columns.mins[slot],
                EntityFields::default(),
            ),
            (
                self.maxs,
                &mut entities.columns.maxs[slot],
                EntityFields::default(),
            ),
        ] {
            if let Some(field) = field {
                *target = self.vector(memory, address, Some(field))?;
                entities.columns.published_fields[slot].insert(authority);
            }
        }
        if self.mins.is_some() && self.maxs.is_some() {
            entities.columns.published_fields[slot].insert(EntityFields::BOUNDS);
        }
        entities.columns.angular_velocity[slot] = self
            .angular_velocity
            .map(|field| self.vector(memory, address, Some(field)))
            .transpose()?;
        Ok(())
    }
    fn store_number(
        self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
        value: f32,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        store(
            memory,
            address + field.ok_or(CallError::Memory)? as u64,
            value.to_bits(),
        )
    }
    fn store_vector(
        self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        field: Option<usize>,
        value: Vec3,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let address = address + field.ok_or(CallError::Memory)? as u64;
        memory.read(address, 12)?;
        for (axis, word) in value.0.map(f32::to_bits).into_iter().enumerate() {
            store(memory, address + axis as u64 * 4, word)?;
        }
        Ok(())
    }
    fn clear_fields(
        self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let address = address + self.header as u64;
        memory.read(address, self.fields)?;
        for offset in (0..self.fields).step_by(4) {
            store(memory, address + offset as u64, 0)?;
        }
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
        apply_client_pose(player, origin, velocity, None);
        Ok(())
    }
    fn store_client_environment(
        self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        value: ClientEnvironmentWrite,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        for (field, value) in [
            (self.motion.water_level, f32::from(value.water_level)),
            (self.motion.water_type, value.water_type as f32),
        ] {
            if let Some(field) = field {
                self.store_number(memory, address, Some(field), value, store)?;
            }
        }
        if let Some(flags) = value.flags {
            self.store_number(memory, address, self.flags, flags as i32 as f32, store)?;
        }
        if let Some(ground) = value.ground
            && let Some(field) = self.motion.ground_entity
        {
            store(memory, address + field as u64, ground)?;
        }
        Ok(())
    }
    fn client_mode(
        self,
        memory: &ModuleMemory<'_>,
        address: u64,
        phase: Motion,
    ) -> Result<Option<qa_core::primitives::MovementMode>, CallError> {
        use qa_core::primitives::MovementMode;
        Ok(Some(match phase {
            Motion::None => MovementMode::Frozen,
            Motion::Noclip => MovementMode::Noclip,
            Motion::Fly | Motion::FlyMissile => MovementMode::Fly,
            Motion::Walk | Motion::Step | Motion::Toss | Motion::Bounce => {
                if self
                    .client
                    .health
                    .map(|field| self.number(memory, address, Some(field)))
                    .transpose()?
                    .is_some_and(|health| health <= 0.0)
                {
                    MovementMode::Dead
                } else {
                    MovementMode::Walk
                }
            }
            _ => return Ok(None),
        }))
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
    projection: &'a EntityProjection,
    module: ModuleId,
    geometry: Option<qa_core::primitives::GeometryId>,
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
    fn failed(&self) -> bool {
        self.error.failed()
    }
    fn supports(&self, module: ModuleId) -> bool {
        self.module == module
    }
    fn owner(&self, native: NativeEntity) -> Option<CollisionOwner> {
        if !self.supports(native.module) {
            return None;
        }
        let slot = native.slot;
        self.error.capture(|| {
            self.projection
                .quakec_collision_owner(self.memory, self.module, self.address(slot)?)
        })
    }
    fn fields(&self, native: NativeEntity, point_contents: bool) -> Option<NativeTraceFields> {
        if !self.supports(native.module) {
            return None;
        }
        let slot = native.slot;
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
    fn bounds(&self, native: NativeEntity, cached: Bounds) -> Option<Bounds> {
        if !self.supports(native.module) {
            return None;
        }
        let slot = native.slot;
        self.error
            .capture(|| self.layout.bounds(self.memory, self.address(slot)?, cached))
    }
    fn target(
        &self,
        native: NativeEntity,
        cached: NativeTraceTarget,
        point_contents: bool,
        store: &CollisionStore,
        _names: Option<&dyn NativeModelNames>,
    ) -> Option<NativeTraceTarget> {
        if !self.supports(native.module) {
            return None;
        }
        let slot = native.slot;
        self.error.capture(|| {
            let address = self.address(slot)?;
            self.layout.target(
                self.memory,
                self.strings,
                address,
                cached,
                store,
                self.geometry,
                Some(point_contents),
            )
        })
    }
}

impl EntityProjection {
    fn quakec_collision_owner(
        &self,
        memory: &ModuleMemory<'_>,
        module: ModuleId,
        address: u64,
    ) -> Result<CollisionOwner, CallError> {
        let layout = self.quakec_layout()?;
        let value = layout.word(memory, address, layout.owner)?;
        if value == 0 {
            return Ok(CollisionOwner::None);
        }
        self.quakec_entity_owner(memory, module, value)
    }
    pub(crate) fn quakec_entity_owner(
        &self,
        memory: &ModuleMemory<'_>,
        module: ModuleId,
        value: u32,
    ) -> Result<CollisionOwner, CallError> {
        let native = self.quakec_layout()?.identity(memory, module, value)?;
        Ok(self
            .quakec_foreign_entity(native.slot as usize)
            .map_or(CollisionOwner::Native(native), CollisionOwner::Lifetime))
    }
    pub(crate) fn quakec_is_foreign(&self, slot: usize) -> bool {
        self.bindings
            .get(slot)
            .copied()
            .flatten()
            .is_some_and(|binding| binding.foreign())
    }
    pub(crate) fn quakec_foreign_entity(&self, slot: usize) -> Option<EntityId> {
        self.bindings
            .get(slot)
            .copied()
            .flatten()
            .filter(|binding| binding.foreign())
            .map(|binding| binding.entity)
    }
    pub(crate) fn quakec_resolve_value(
        &self,
        entities: &EntityTable,
        native: NativeEntity,
    ) -> Result<Option<EntityId>, CallError> {
        if self.owner != Some(native.module) {
            return Err(CallError::Entity);
        }
        let slot = usize::try_from(native.slot).map_err(|_| CallError::Entity)?;
        if let Some(entity) = self.quakec_foreign_entity(slot) {
            return Ok(entities.resolve(entity).map(|_| entity));
        }
        self.binding(entities, native)
    }
    pub(crate) fn quakec_admit_entity(
        &mut self,
        entities: &EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        entity: EntityId,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(usize, bool), CallError> {
        if self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        let canonical = entities.resolve(entity).ok_or(CallError::Entity)?;
        if let Some(native) = entities.columns.native_entity[canonical]
            .filter(|native| native.module == context.module)
        {
            if self.binding(entities, native)? != Some(entity) {
                return Err(CallError::Entity);
            }
            return Ok((native.slot as usize, false));
        }
        self.quakec_admit_alias(entities, memory, context, entity, store)
    }
    pub(crate) fn quakec_admit_retired_alias(
        &mut self,
        entities: &EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        entity: EntityId,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(usize, bool), CallError> {
        if self.owner != Some(context.module) || entities.resolve(entity).is_some() {
            return Err(CallError::Entity);
        }
        self.quakec_admit_alias(entities, memory, context, entity, store)
    }
    fn quakec_admit_alias(
        &mut self,
        entities: &EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        entity: EntityId,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(usize, bool), CallError> {
        if let Some(slot) = self.bindings.iter().position(|row| {
            row.is_some_and(|binding| binding.foreign() && binding.entity == entity)
        }) {
            let binding = self.bindings[slot].as_mut().ok_or(CallError::Entity)?;
            let BindingRole::Foreign { active_uses, .. } = &mut binding.role else {
                return Err(CallError::Entity);
            };
            *active_uses = active_uses.checked_add(1).ok_or(CallError::Capacity)?;
            return Ok((slot, true));
        }
        let (slot, _) = self.quakec_claim_slot(entities, context, false)?;
        let layout = self.quakec_layout()?;
        layout.clear_fields(memory, (slot * layout.stride) as u64, store)?;
        self.bindings[slot] = Some(Binding {
            entity,
            role: BindingRole::Foreign {
                active_uses: 1,
                classname: None,
            },
            published: false,
            portal_areas: [None; 2],
            freed_at: None,
            think_word: None,
        });
        self.observed_count = self.observed_count.max(slot + 1);
        Ok((slot, true))
    }
    pub(crate) fn quakec_release_alias(
        &mut self,
        slot: usize,
        entity: EntityId,
    ) -> Result<(), CallError> {
        let binding = self
            .bindings
            .get_mut(slot)
            .and_then(Option::as_mut)
            .filter(|binding| binding.entity == entity)
            .ok_or(CallError::Entity)?;
        let BindingRole::Foreign { active_uses, .. } = &mut binding.role else {
            return Err(CallError::Entity);
        };
        *active_uses = active_uses.checked_sub(1).ok_or(CallError::Entity)?;
        Ok(())
    }
    pub(crate) fn quakec_alias_classname(
        &self,
        slot: usize,
    ) -> Option<(qa_core::primitives::NameId, i32, usize)> {
        match self.bindings.get(slot).copied().flatten()?.role {
            BindingRole::Foreign {
                classname: Some(value),
                ..
            } => Some((value.name, value.offset, value.capacity)),
            _ => None,
        }
    }
    pub(crate) fn quakec_cache_alias_classname(
        &mut self,
        slot: usize,
        name: qa_core::primitives::NameId,
        offset: i32,
        capacity: usize,
    ) -> Result<(), CallError> {
        let binding = self
            .bindings
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(CallError::Entity)?;
        let BindingRole::Foreign { classname, .. } = &mut binding.role else {
            return Err(CallError::Entity);
        };
        *classname = Some(ClassnameSlot {
            name,
            offset,
            capacity,
        });
        Ok(())
    }
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
    pub(crate) fn quakec_ensure_client_body(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<EntityId, CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        self.quakec_client_address(memory, value)?;
        if let Some(entity) = self.binding(&services.server.entities, native)? {
            return Ok(entity);
        }
        self.quakec_allocate_client_body(services, memory, strings, context, native, None)
    }
    pub(crate) fn quakec_retain_client_body(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        old: EntityId,
    ) -> Result<EntityId, CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        self.quakec_client_address(memory, value)?;
        let previous = self.bindings[native.slot as usize].ok_or(CallError::Entity)?;
        if previous.owned()
            || previous.entity != old
            || self.quakec_bound_entity(&services.server.entities, native)? != Some(old)
        {
            return Err(CallError::Entity);
        }
        self.quakec_allocate_client_body(services, memory, strings, context, native, Some(old))
    }
    fn quakec_allocate_client_body(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        native: NativeEntity,
        old: Option<EntityId>,
    ) -> Result<EntityId, CallError> {
        let slot = native.slot as usize;
        let previous = self.bindings[slot];
        let mut claim = context;
        claim.allocation.full = FullTable::Reject;
        let entity = self.bind(services, claim, slot, None)?;
        let published = self
            .quakec_project_row(services, memory, strings, context, slot)
            .and_then(|()| {
                if let Some(old) = old
                    && services.server.area.bounds(old).is_some()
                    && !services
                        .server
                        .area
                        .transfer_link(&services.server.entities, old, entity)
                {
                    return Err(CallError::Entity);
                }
                Ok(())
            });
        if let Err(error) = published {
            let cleanup = self.retire(services, context, slot);
            self.bindings[slot] = previous;
            cleanup?;
            return Err(error);
        }
        Ok(entity)
    }
    pub(crate) fn quakec_release_client_body(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        self.quakec_client_address(memory, value)?;
        let slot = native.slot as usize;
        let Some(binding) = self.bindings[slot].filter(|binding| binding.owned()) else {
            return Ok(());
        };
        if services.server.entities.resolve(binding.entity).is_some()
            && self.quakec_bound_entity(&services.server.entities, native)? != Some(binding.entity)
        {
            return Err(CallError::Entity);
        }
        if services.server.clients.iter().any(|client| {
            client.connection.is_some()
                && client.entity == binding.entity
                && services.server.entities.resolve(client.entity).is_some()
                && services.server.entities.columns.native_entity[client.entity.slot as usize]
                    == Some(native)
        }) {
            return Err(CallError::Entity);
        }
        self.retire(services, context, slot)
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let (address, client) = self.quakec_client(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        self.quakec_validate_client_spawn(memory, context, value)?;
        layout.clear_fields(memory, address, store)?;
        layout.store_number(
            memory,
            address,
            layout.colormap,
            (value as usize / layout.stride) as f32,
            store,
        )?;
        let team = if context.native_rules == RuleSetId::Quake {
            f32::from((colors & 15) + 1)
        } else {
            0.0
        };
        layout.store_number(memory, address, layout.client.team, team, store)?;
        store(
            memory,
            address + layout.client.net_name.ok_or(CallError::Memory)? as u64,
            name,
        )?;
        if let Some(maxspeed) = maxspeed {
            for (field, value) in [
                (layout.motion.gravity, 1.0),
                (layout.client.max_speed, maxspeed),
            ] {
                if let Some(field) = field {
                    layout.store_number(memory, address, Some(field), value, store)?;
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
        store: &mut crate::quakec::EntityStore<'_>,
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
            layout.store_vector(
                memory,
                address,
                layout.client.view_angles,
                command.view_angles,
                store,
            )?;
        }
        layout.store_number(
            memory,
            address,
            layout.client.button_attack,
            (command.buttons & qa_core::primitives::buttons::ATTACK) as f32,
            store,
        )?;
        layout.store_number(
            memory,
            address,
            layout.client.button_jump,
            ((command.buttons & qa_core::primitives::buttons::JUMP) >> 1) as f32,
            store,
        )?;
        if command.impulse != 0 {
            layout.store_number(
                memory,
                address,
                layout.client.impulse,
                f32::from(command.impulse),
                store,
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        if context.native_rules != RuleSetId::QuakeWorld {
            return Err(CallError::Unsupported);
        }
        self.quakec_receive_client_command(memory, context, value, command, store)?;
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        // SV_RunCmd reads these fields after the preceding stores. QuakeC
        // definitions can overlap, including one angle store with the next
        // v_angle component, so preserve each scalar native read/write.
        if layout.number(memory, address, layout.client.health)? > 0.0 {
            if layout.number(memory, address, layout.client.fix_angle)? == 0.0 {
                let pitch = layout.number(memory, address, layout.client.view_angles)?;
                layout.store_number(memory, address, layout.angles, -pitch / 3.0, store)?;
                let yaw = layout.number(
                    memory,
                    address,
                    layout.client.view_angles.map(|field| field + 4),
                )?;
                layout.store_number(
                    memory,
                    address,
                    layout.angles.map(|field| field + 4),
                    yaw,
                    store,
                )?;
            }
            let angles = layout.vector(memory, address, layout.angles)?;
            let velocity = layout.vector(memory, address, layout.velocity)?;
            let roll = qa_gameplay::rules::appearance::AliasRoll {
                angle: roll_angle,
                speed: roll_speed,
            }
            .calculate(angles, velocity)
                * 4.0;
            layout.store_number(
                memory,
                address,
                layout.angles.map(|field| field + 8),
                roll,
                store,
            )?;
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
        Ok(client_motion(layout.number(
            memory,
            address,
            layout.move_type,
        )?))
    }
    pub(crate) fn quakec_client_view_angles(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<Vec3, CallError> {
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        layout.vector(memory, address, layout.client.view_angles)
    }
    pub(crate) fn quakec_client_view_height(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<f32, CallError> {
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        layout.number(
            memory,
            address,
            layout.client.view_offset.map(|field| field + 8),
        )
    }
    pub(crate) fn quakec_client_water_level(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<f32, CallError> {
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        layout.number(memory, address, layout.motion.water_level)
    }
    pub(crate) fn quakec_client_old_origin(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        axis: usize,
    ) -> Result<f32, CallError> {
        if axis >= 3 {
            return Err(CallError::Memory);
        }
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        layout.number(
            memory,
            address,
            layout.client.old_origin.map(|field| field + axis * 4),
        )
    }
    pub(crate) fn quakec_client_teleport_time(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<f32, CallError> {
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        layout.number(memory, address, layout.client.water_jump)
    }
    pub(crate) fn quakec_client_move_direction(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        axis: usize,
    ) -> Result<f32, CallError> {
        if axis >= 3 {
            return Err(CallError::Memory);
        }
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        layout.number(
            memory,
            address,
            layout.client.move_direction.map(|field| field + axis * 4),
        )
    }
    pub(crate) fn quakec_prepare_control(
        &self,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        seconds: f64,
        roll: qa_gameplay::rules::appearance::AliasRoll,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<Option<qa_movement::ControlStart>, CallError> {
        self.quakec_identity(memory, context, value)?;
        let address = self.quakec_client_address(memory, value)?;
        let layout = self.quakec_layout()?;
        if layout.number(memory, address, layout.move_type)? == 0.0 {
            return Ok(None);
        }
        let on_ground = layout.flags(memory, address)? & 512 != 0;
        let mut punch = layout.vector(memory, address, layout.client.punch)?;
        let length = qa_core::math::normalize(&mut punch);
        layout.store_vector(memory, address, layout.client.punch, punch, store)?;
        let mut length = (f64::from(length) - 10.0 * seconds) as f32;
        if length < 0.0 {
            length = 0.0;
        }
        for axis in 0..3 {
            let field = layout.client.punch.map(|at| at + axis * 4);
            let value = layout.number(memory, address, field)? * length;
            layout.store_number(memory, address, field, value, store)?;
        }
        if layout.number(memory, address, layout.client.health)? <= 0.0 {
            return Ok(None);
        }
        let view = layout.vector(memory, address, layout.client.view_angles)?
            + layout.vector(memory, address, layout.client.punch)?;
        let body_angles = layout.vector(memory, address, layout.angles)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        layout.store_number(
            memory,
            address,
            layout.angles.map(|at| at + 8),
            roll.calculate(body_angles, velocity) * 4.0,
            store,
        )?;
        if layout.number(memory, address, layout.client.fix_angle)? == 0.0 {
            layout.store_number(memory, address, layout.angles, -view.0[0] / 3.0, store)?;
            layout.store_number(
                memory,
                address,
                layout.angles.map(|at| at + 4),
                view.0[1],
                store,
            )?;
        }
        Ok(Some(qa_movement::ControlStart {
            on_ground,
            body_angles: layout.vector(memory, address, layout.angles)?,
        }))
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
    fn quakec_client_environment(
        &self,
        entities: &EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        address: u64,
    ) -> Result<ClientEnvironment, CallError> {
        let layout = self.quakec_layout()?;
        let integer = |field: Option<usize>| {
            field
                .map(|at| layout.number(memory, address, Some(at)))
                .transpose()
                .map(|value| value.map(qa_core::math::native_integer))
        };
        let water_level = integer(layout.motion.water_level)?
            .map(|level| if level < 0 { 0 } else { level as u8 });
        let water_contents = integer(layout.motion.water_type)?
            .map(|contents| Contents::from_q1(contents, Contents::EMPTY).0);
        let flags = layout
            .flags
            .map(|_| layout.flags(memory, address))
            .transpose()?;
        let ground = if flags.is_some_and(|flags| flags & 512 != 0)
            && layout.motion.ground_entity.is_some()
        {
            let word = layout.word(memory, address, layout.motion.ground_entity)?;
            let native = layout.identity(memory, context.module, word)?;
            self.quakec_resolve_value(entities, native)?
        } else {
            None
        };
        Ok(ClientEnvironment {
            flags,
            ground,
            water_level,
            water_contents,
        })
    }
    fn quakec_client_environment_write(
        &mut self,
        services: &EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        address: u64,
        client: qa_core::primitives::ClientId,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<ClientEnvironmentWrite, CallError> {
        let layout = self.quakec_layout()?;
        let player = &services.server.clients[client.0 as usize].player;
        let flags = layout
            .flags
            .map(|_| layout.flags(memory, address))
            .transpose()?
            .map(|flags| {
                if player.movement.grounded {
                    flags | 512
                } else {
                    flags & !512
                }
            });
        Ok(ClientEnvironmentWrite {
            water_level: player.movement.water_level,
            water_type: Contents(player.movement.water_contents).to_q1(),
            flags,
            ground: if player.movement.grounded {
                Some(self.quakec_encode_hit(
                    &services.server.entities,
                    memory,
                    context,
                    player.movement.ground,
                    store,
                )?)
            } else {
                None
            },
        })
    }
    pub(crate) fn quakec_read_frame_client(
        &self,
        entities: &EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        entity: EntityId,
        phase: Motion,
        player: &mut PlayerState,
    ) -> Result<(), CallError> {
        let address = self.quakec_client_lifetime(entities, memory, context, value, entity)?;
        let layout = self.quakec_layout()?;
        let deadline = layout.number(memory, address, layout.client.water_jump)?;
        let environment = self.quakec_client_environment(entities, memory, context, address)?;
        let movement_mode = layout
            .client_mode(memory, address, phase)?
            .ok_or(CallError::Unsupported)?;
        layout.read_client_pose(memory, address, player)?;
        apply_client_mode(player, Some(movement_mode));
        environment.apply(player);
        player.movement.teleport_hold_until = f64::from(deadline);
        Ok(())
    }
    pub(crate) fn quakec_finish_frame_client(
        &mut self,
        services: &EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let (address, client) = self.quakec_client(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        let view = services.server.clients[client.0 as usize]
            .player
            .view_angles;
        let environment = self
            .quakec_client_environment_write(services, memory, context, address, client, store)?;
        layout.store_client_environment(memory, address, environment, store)?;
        if let Some(field) = layout.client.view_angles {
            layout.store_vector(memory, address, Some(field), view, store)?;
        }
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
        store: &mut crate::quakec::EntityStore<'_>,
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
            qa_core::primitives::MotionWrite::Angles(_)
            | qa_core::primitives::MotionWrite::AngularVelocity(_) => {
                return Err(CallError::Unsupported);
            }
        };
        let value = *vector.0.get(axis).ok_or(CallError::Memory)?;
        layout.store_number(memory, address, field.map(|at| at + axis * 4), value, store)?;
        layout.read_client_pose(memory, address, player)
    }
    pub(crate) fn quakec_classname<'a>(
        &self,
        memory: &ModuleMemory<'_>,
        strings: &'a ModuleMemory<'_>,
        value: u32,
    ) -> Result<&'a [u8], CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let Some(field) = layout.name_fields[0] else {
            return Ok(b"");
        };
        let word = layout.word(memory, address, Some(field))?;
        Ok(strings.cstring(u64::from(word))?)
    }
    pub(crate) fn quakec_prepare_velocity(
        &self,
        entities: &mut EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let entity = self
            .quakec_bound_entity(entities, native)?
            .ok_or(CallError::Entity)?;
        let origin = layout.vector(memory, address, layout.origin)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        let slot = entities.resolve(entity).ok_or(CallError::Entity)?;
        entities.columns.set_position(slot, origin);
        entities.columns.set_velocity(slot, velocity);
        Ok(())
    }
    pub(crate) fn quakec_refresh_client_pose(
        &self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        entity: EntityId,
    ) -> Result<(), CallError> {
        if !matches!(
            context.native_rules,
            RuleSetId::Quake | RuleSetId::QuakeWorld
        ) {
            return Err(CallError::Unsupported);
        }
        let Some(index) = services.server.entities.client_index(entity) else {
            return Ok(());
        };
        let client = services
            .server
            .clients
            .get(index)
            .ok_or(CallError::Entity)?;
        if !client.is_active() {
            return Ok(());
        }
        let native = self.quakec_identity(memory, context, value)?;
        if client.entity != entity
            || client.module != context.module
            || self.quakec_bound_entity(&services.server.entities, native)? != Some(entity)
        {
            return Err(CallError::Entity);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let origin = layout.vector(memory, address, layout.origin)?;
        let velocity = layout.vector(memory, address, layout.velocity)?;
        let mins = (context.native_rules == RuleSetId::QuakeWorld)
            .then(|| layout.vector(memory, address, layout.mins))
            .transpose()?;
        apply_client_pose(
            &mut services.server.clients[index].player,
            origin,
            velocity,
            mins,
        );
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
        layout.number(memory, address, layout.motion.water_level)?;
        layout.number(memory, address, layout.motion.water_type)?;
        layout.flags(memory, address)?;
        let gravity = layout
            .motion
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
        qa_movement::set_bounds(player);
        let offset = apply_client_pose(player, origin, velocity, Some(mins));
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let (address, client) = self.quakec_client(services, memory, context, value)?;
        let layout = self.quakec_layout()?;
        layout.link_fields(memory, address)?;
        for field in [layout.velocity, layout.client.view_angles] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 12)?;
        }
        for field in [
            layout.client.water_jump,
            layout.motion.water_level,
            layout.motion.water_type,
            layout.motion.ground_entity,
        ] {
            memory.read(address + field.ok_or(CallError::Memory)? as u64, 4)?;
        }
        let player = &services.server.clients[client.0 as usize].player;
        let water_jump = player.movement.water_jump_seconds;
        let origin = player.body.position - offset;
        let velocity = player.body.velocity;
        let view = player.view_angles;
        let environment = self
            .quakec_client_environment_write(services, memory, context, address, client, store)?;
        layout.store_number(memory, address, layout.client.water_jump, water_jump, store)?;
        layout.store_client_environment(memory, address, environment, store)?;
        layout.store_vector(memory, address, layout.origin, origin, store)?;
        layout.store_vector(memory, address, layout.velocity, velocity, store)?;
        layout.store_vector(memory, address, layout.client.view_angles, view, store)?;
        self.quakec_link(services, memory, strings, context, value, store)
    }
    pub(crate) fn quakec_touch_callback(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        trigger_only: bool,
    ) -> Result<Option<qa_core::primitives::CallbackId>, CallError> {
        self.quakec_touch_word(memory, value, trigger_only)?
            .map(|word| self.quakec_callback(word))
            .transpose()
    }
    pub(crate) fn quakec_touch_word(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        trigger_only: bool,
    ) -> Result<Option<u32>, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let word = layout.word(memory, address, layout.motion.touch)?;
        if word == 0 {
            Ok(None)
        } else {
            let solid = layout.number(memory, address, layout.solid)?;
            if if trigger_only {
                solid != 1.0
            } else {
                solid == 0.0
            } {
                Ok(None)
            } else {
                Ok(Some(word))
            }
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let layout = self.quakec_layout()?;
        let address = self.quakec_client_address(memory, value)?;
        layout.store_number(memory, address, layout.client.fix_angle, 0.0, store)
    }
    pub(crate) fn quakec_trace_view<'a, 'memory>(
        &'a self,
        memory: &'a ModuleMemory<'memory>,
        strings: &'a ModuleMemory<'memory>,
        context: CallContext,
        pass: u32,
        geometry: Option<qa_core::primitives::GeometryId>,
    ) -> Result<(QuakeCTraceView<'a, 'memory>, CollisionOwner), CallError> {
        self.quakec_validate_owner(context.module)?;
        let pass = self.quakec_entity_owner(memory, context.module, pass)?;
        Ok((
            QuakeCTraceView {
                memory,
                strings,
                layout: self.quakec_layout()?,
                projection: self,
                module: context.module,
                geometry,
                error: TraceError::default(),
            },
            pass,
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
    pub(crate) fn quakec_foreign_fields(
        &self,
    ) -> Result<Box<[crate::quakec::ForeignWord]>, CallError> {
        Ok(self.quakec_layout()?.foreign_fields())
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
        let binding = thinks.binding(function);
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
            if self.quakec_is_foreign(native_slot) {
                continue;
            }
            let client = native_slot > 0 && native_slot <= clients;
            // An owned reserved-row body has no active connection. It still
            // participates in links and pushes, but native client physics is
            // skipped, just as SV_Physics_Client skips inactive clients.
            if client
                && (layout.integer_motion
                    || self.bindings[native_slot].is_some_and(|binding| binding.owned()))
            {
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
            let entity = self.quakec_resolve_value(entities, native)?;
            // Native client edicts remain allocated when disconnected. They
            // are ABI-visible without inventing a common entity lifetime.
            if entity.is_some() || (slot > 0 && slot <= clients) {
                return Ok(Some((native, entity)));
            }
        }
        Ok(None)
    }
    pub(crate) fn quakec_push_candidate(
        &self,
        cursor: &mut usize,
    ) -> Result<Option<NativeEntity>, CallError> {
        let module = self.owner.ok_or(CallError::Entity)?;
        while *cursor < self.observed_count {
            let slot = i32::try_from(*cursor).map_err(|_| CallError::Entity)?;
            *cursor += 1;
            if self.quakec_is_foreign(slot as usize) {
                continue;
            }
            return Ok(Some(NativeEntity { module, slot }));
        }
        Ok(None)
    }
    pub(crate) fn quakec_push_free(
        &self,
        entities: &EntityTable,
        native: NativeEntity,
    ) -> Result<bool, CallError> {
        let slot = usize::try_from(native.slot).map_err(|_| CallError::Entity)?;
        if self.owner != Some(native.module) || slot >= self.observed_count {
            return Err(CallError::Entity);
        }
        let clients = self
            .quakec_layout()?
            .client_edicts
            .ok_or(CallError::Entity)?;
        Ok(slot > clients && self.quakec_bound_entity(entities, native)?.is_none())
    }
    pub(crate) fn quakec_function_word(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        blocked: bool,
    ) -> Result<u32, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        layout.word(
            memory,
            address,
            if blocked {
                layout.motion.blocked
            } else {
                layout.think
            },
        )
    }
    pub(crate) fn quakec_function_binding(
        &self,
        word: u32,
    ) -> Result<qa_core::primitives::ThinkBinding, CallError> {
        Ok(self
            .quakec_thinks
            .as_ref()
            .ok_or(CallError::Entity)?
            .binding(word))
    }
    pub(crate) fn quakec_refresh_raw_motion(
        &self,
        entities: &mut EntityTable,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        if let Some(entity) = self.quakec_bound_entity(entities, native)? {
            self.quakec_layout()?
                .refresh_motion(memory, u64::from(value), entities, entity)?;
        }
        Ok(())
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
        let layout = self.quakec_layout()?;
        let native = layout.identity(memory, context.module, value)?;
        if self.quakec_is_foreign(native.slot as usize) {
            return Err(CallError::Entity);
        }
        Ok(native)
    }
    pub(crate) fn quakec_validate_owner(&self, module: ModuleId) -> Result<(), CallError> {
        if self.owner == Some(module) {
            Ok(())
        } else {
            Err(CallError::Entity)
        }
    }
    fn quakec_claim_slot(
        &self,
        entities: &EntityTable,
        context: CallContext,
        overwrite: bool,
    ) -> Result<(usize, CallContext), CallError> {
        let layout = self.quakec_layout()?;
        let clients = layout.client_edicts.ok_or(CallError::Entity)?;
        let first = clients.checked_add(1).ok_or(CallError::Capacity)?;
        if first >= self.bindings.len() {
            return Err(CallError::Capacity);
        }
        let eligible = (first..self.observed_count).find(|&slot| {
            let row = self.bindings[slot];
            let free = row.is_none_or(|binding| {
                !binding.foreign() && entities.resolve(binding.entity).is_none()
            });
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
        } else if let Some(slot) = (first..self.observed_count).find(|&slot| {
            self.bindings[slot].is_some_and(|binding| {
                matches!(binding.role, BindingRole::Foreign { active_uses: 0, .. })
                    && entities.resolve(binding.entity).is_none()
            })
        }) {
            slot
        } else if overwrite && context.allocation.full == FullTable::OverwriteLastOwned {
            let slot = self.bindings.len() - 1;
            let binding = self.bindings[slot].ok_or(CallError::Capacity)?;
            if !binding.owned() {
                return Err(CallError::Capacity);
            }
            claim.allocation.full = FullTable::OverwriteOwned(binding.entity);
            slot
        } else {
            return Err(CallError::Capacity);
        };
        Ok((slot, claim))
    }
    pub(crate) fn quakec_spawn(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<u32, CallError> {
        let layout = self.quakec_layout()?;
        if self.owner != Some(context.module) {
            return Err(CallError::Entity);
        }
        let (slot, claim) = self.quakec_claim_slot(&services.server.entities, context, true)?;
        let entity = self.bind(services, claim, slot, None)?;
        self.observed_count = self.observed_count.max(slot + 1);
        let address = slot.checked_mul(layout.stride).ok_or(CallError::Memory)?;
        // ED_ClearEdict clears only entvars. Boundary metadata does not inhabit
        // VM bytes and common lifetime liveness remains in EntityTable.
        layout.clear_fields(memory, address as u64, store)?;
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
        if native.slot as usize <= self.quakec_client_count()?
            || !binding.owned()
            || !services.server.entities.can_release(entity)
        {
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let (native, entity) = self.quakec_removal(services, memory, context, value)?;
        self.quakec_free_validated(services, memory, context, value, native, entity, store)
    }

    fn quakec_free_validated(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        native: NativeEntity,
        entity: EntityId,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let slot = native.slot as usize;
        let layout = self.quakec_layout()?;
        (ENGINE_CALLS.free)(services, context, entity)?;
        self.bindings[slot]
            .as_mut()
            .ok_or(CallError::Entity)?
            .freed_at = Some(context.allocation.reuse.freetime(context.clock));
        for field in layout.removal_words() {
            layout.store_number(memory, value as u64, field, 0.0, store)?;
        }
        for field in layout.removal_vectors() {
            layout.store_vector(memory, value as u64, field, Vec3::default(), store)?;
        }
        layout.store_number(memory, value as u64, layout.next_think, -1.0, store)
    }
    pub(crate) fn quakec_make_static(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        store: &mut crate::quakec::EntityStore<'_>,
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
                qa_core::math::native_integer(f64::from(layout.number(memory, address, field)?))
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
                self.quakec_free_validated(services, memory, context, value, native, entity, store)
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        layout.store_vector(memory, value as u64, layout.origin, origin, store)?;
        self.quakec_link(services, memory, strings, context, value, store)
    }
    pub(crate) fn quakec_yaw_change(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<Option<(u64, f32)>, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        let yaw = layout.angles.ok_or(CallError::Memory)? + 4;
        let current = layout.number(memory, address, Some(yaw))?;
        let ideal = layout.number(memory, address, layout.motion.ideal_yaw)?;
        let speed = layout.number(memory, address, layout.motion.yaw_speed)?;
        Ok(qa_movement::change_yaw(current, ideal, speed)
            .map(|value| (address + yaw as u64, value)))
    }
    pub(crate) fn quakec_monster_origin(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<Vec3, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        layout.vector(memory, address, layout.origin)
    }
    pub(crate) fn quakec_monster_bounds(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<Bounds, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        Ok(Bounds {
            mins: layout.vector(memory, address, layout.mins)?,
            maxs: layout.vector(memory, address, layout.maxs)?,
        })
    }
    pub(crate) fn quakec_monster_flags(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<u32, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        layout.flags(memory, layout.address(memory, value)?)
    }
    pub(crate) fn quakec_monster_enemy(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<u32, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        layout.word(memory, layout.address(memory, value)?, layout.motion.enemy)
    }
    pub(crate) fn quakec_monster_goal(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
    ) -> Result<u32, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        let layout = self.quakec_layout()?;
        layout.word(memory, layout.address(memory, value)?, layout.motion.goal)
    }
    pub(crate) fn quakec_scalar_number(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        field: crate::quakec::Scalar,
    ) -> Result<f32, CallError> {
        Ok(f32::from_bits(
            memory.read_word(self.quakec_scalar_store(memory, value, field)?)? as u32,
        ))
    }
    pub(crate) fn quakec_scalar_store(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        field: crate::quakec::Scalar,
    ) -> Result<u64, CallError> {
        if self.quakec_is_foreign(value as usize / self.quakec_layout()?.stride) {
            return Err(CallError::Unsupported);
        }
        self.quakec_scalar_address(memory, value, field)
    }
    pub(crate) fn quakec_scalar_address(
        &self,
        memory: &ModuleMemory<'_>,
        value: u32,
        field: crate::quakec::Scalar,
    ) -> Result<u64, CallError> {
        let layout = self.quakec_layout()?;
        let address = layout.address(memory, value)?;
        use crate::quakec::Scalar;
        let vector = |field: Option<usize>, axis: usize| {
            field.filter(|_| axis < 3).map(|field| field + axis * 4)
        };
        let field = match field {
            Scalar::Origin(axis) => vector(layout.origin, axis),
            Scalar::Velocity(axis) => vector(layout.velocity, axis),
            Scalar::Angles(axis) => vector(layout.angles, axis),
            Scalar::AngularVelocity(axis) => vector(layout.angular_velocity, axis),
            Scalar::Flags => layout.flags,
            Scalar::Ground => layout.motion.ground_entity,
            Scalar::IdealYaw => layout.motion.ideal_yaw,
            Scalar::Yaw => vector(layout.angles, 1),
            Scalar::Min(axis) => vector(layout.mins, axis),
            Scalar::Max(axis) => vector(layout.maxs, axis),
            Scalar::AbsoluteMin(axis) => vector(layout.abs_min, axis),
            Scalar::AbsoluteMax(axis) => vector(layout.abs_max, axis),
            Scalar::LocalTime => layout.motion.local_time,
            Scalar::NextThink => layout.next_think,
            Scalar::MoveType => layout.move_type,
            Scalar::Solid => layout.solid,
        };
        let address = address + field.ok_or(CallError::Memory)? as u64;
        memory.read(address, 4)?;
        Ok(address)
    }
    pub(crate) fn quakec_check_bottom(
        &self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<bool, CallError> {
        use qa_world::collision::{QuakeTraceKind, trace_policy};
        self.quakec_identity(memory, context, value)?;
        let origin = self.quakec_monster_origin(memory, value)?;
        let bounds = self.quakec_monster_bounds(memory, value)?;
        let (rules, entity_rules) = trace_policy(context.trace_rules);
        let (view, pass) =
            self.quakec_trace_view(memory, strings, context, value, services.native_geometry)?;
        let result = qa_session::physics::check_bottom(
            &mut QuakeCSupport {
                services,
                view: &view,
                pass,
                rules,
                entities: entity_rules.with_quake_kind(Some(QuakeTraceKind::IgnoreBoxes)),
            },
            origin,
            bounds,
            qa_gameplay::rules::motion::step_rules(context.native_rules)
                .ok_or(CallError::Unsupported)?
                .height,
        );
        view.finish()?;
        result
    }
    pub(crate) fn quakec_drop_to_floor(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<bool, CallError> {
        use qa_world::collision::{QuakeTraceKind, TraceQuery, trace_policy};
        self.quakec_identity(memory, context, value)?;
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
        let (view, pass) =
            self.quakec_trace_view(memory, strings, context, value, services.native_geometry)?;
        query.pass = pass;
        let trace = (ENGINE_CALLS.trace)(services, query, Some(&view));
        view.finish()?;
        let trace = trace?;
        if trace.fraction == 1.0 || trace.all_solid {
            return Ok(false);
        }
        let ground_address =
            u64::from(value) + layout.motion.ground_entity.ok_or(CallError::Memory)? as u64;
        let flags_address = u64::from(value) + layout.flags.ok_or(CallError::Memory)? as u64;
        memory.read(ground_address, 4)?;
        memory.read(flags_address, 4)?;
        if value != 0 {
            layout.link_fields(memory, u64::from(value))?;
        }
        let ground = self.quakec_encode_hit(
            &services.server.entities,
            memory,
            context,
            trace.entity,
            store,
        )?;
        self.quakec_set_origin(services, memory, strings, context, value, trace.end, store)?;
        let flags = layout.flags(memory, u64::from(value))? as i32 | 512;
        layout.store_number(memory, u64::from(value), layout.flags, flags as f32, store)?;
        store(memory, ground_address, ground)?;
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
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        // PF_setsize rejects backwards extents before modifying any field.
        if !bounds.is_valid() {
            return Err(CallError::Entity);
        }
        let layout = self.quakec_layout()?;
        layout.store_vector(memory, value as u64, layout.mins, bounds.mins, store)?;
        layout.store_vector(memory, value as u64, layout.maxs, bounds.maxs, store)?;
        layout.store_vector(
            memory,
            value as u64,
            layout.size,
            bounds.maxs - bounds.mins,
            store,
        )?;
        self.quakec_link(services, memory, strings, context, value, store)
    }
    pub(crate) fn quakec_set_model(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        name: u32,
        store: &mut crate::quakec::EntityStore<'_>,
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
        store(
            memory,
            u64::from(value) + layout.model.ok_or(CallError::Memory)? as u64,
            name,
        )?;
        layout.store_number(
            memory,
            u64::from(value),
            layout.model_index,
            ordinal as f32,
            store,
        )?;
        let bounds = if text.starts_with(b"*") {
            Some(
                inline_model(
                    services.geometry,
                    services.native_geometry.ok_or(CallError::Geometry)?,
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
            self.quakec_set_size(services, memory, strings, context, value, bounds, store)
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
        self.quakec_validate_owner(context.module)?;
        let layout = self.quakec_layout()?;
        let slot = value as usize / layout.stride;
        layout.address(memory, value)?;
        if let Some(entity) = self.quakec_foreign_entity(slot) {
            use qa_core::primitives::EntityFields;
            let canonical = services
                .server
                .entities
                .resolve(entity)
                .ok_or(CallError::Entity)?;
            let columns = &services.server.entities.columns;
            if !columns.published_fields[canonical].contains(EntityFields::POSITION) {
                return Err(CallError::Unsupported);
            }
            let bsp = matches!(
                columns.collision_shape[canonical],
                CollisionShape::Model { .. }
            );
            let centered = context.native_rules != RuleSetId::QuakeWorld || bsp;
            if centered && !columns.published_fields[canonical].contains(EntityFields::BOUNDS) {
                return Err(CallError::Unsupported);
            }
            let body = columns.body(canonical);
            return Ok((
                entity,
                if centered {
                    body.center()
                } else {
                    body.position
                },
                centered,
            ));
        }
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
    pub(crate) fn quakec_prepare_motion(
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
        layout.refresh_motion(memory, address, entities, entity)
    }
    pub(crate) fn quakec_motion_state(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<qa_session::physics::MotionState, CallError> {
        self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let number = |field: Option<usize>| {
            field
                .map(|field| layout.number(memory, address, Some(field)))
                .transpose()
        };
        Ok(qa_session::physics::MotionState {
            move_type: number(layout.move_type)?.ok_or(CallError::Memory)?,
            solid: number(layout.solid)?,
            flags: layout
                .flags
                .map(|_| layout.flags(memory, address))
                .transpose()?,
        })
    }
    pub(crate) fn quakec_motion_gravity(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<Option<f32>, CallError> {
        self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        layout
            .motion
            .gravity
            .map(|field| layout.number(memory, u64::from(value), Some(field)))
            .transpose()
    }
    pub(crate) fn quakec_motion_water_type(
        &self,
        memory: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<f32, CallError> {
        self.quakec_identity(memory, context, value)?;
        let layout = self.quakec_layout()?;
        layout.number(memory, u64::from(value), layout.motion.water_type)
    }
    pub(crate) fn quakec_write_motion_field(
        &self,
        entities: &mut EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        field: crate::quakec::MotionField,
        ground: Option<u32>,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self.binding(entities, native)?.ok_or(CallError::Entity)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        use crate::quakec::MotionField;
        match field {
            MotionField::Flags(flags) => {
                layout.store_number(memory, address, layout.flags, flags as i32 as f32, store)?
            }
            MotionField::Ground(_) => store(
                memory,
                address + layout.motion.ground_entity.ok_or(CallError::Memory)? as u64,
                ground.ok_or(CallError::Entity)?,
            )?,
            MotionField::WaterType(value) => {
                layout.store_number(memory, address, layout.motion.water_type, value, store)?
            }
            MotionField::WaterLevel(value) => {
                layout.store_number(memory, address, layout.motion.water_level, value, store)?
            }
            MotionField::OldOrigin { axis, value } => {
                if axis >= 3 {
                    return Err(CallError::Memory);
                }
                layout.store_number(
                    memory,
                    address,
                    layout.client.old_origin.map(|field| field + axis * 4),
                    value,
                    store,
                )?;
            }
            MotionField::TeleportTime(value) => {
                layout.store_number(memory, address, layout.client.water_jump, value, store)?;
            }
        }
        layout.refresh_motion(memory, address, entities, entity)
    }

    pub(crate) fn quakec_write_motion(
        &mut self,
        entities: &mut EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        stage: qa_session::physics::MotionWrite,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let entity = self.binding(entities, native)?.ok_or(CallError::Entity)?;
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let slot = entity.slot as usize;
        let (axis, vector) = match stage {
            qa_session::physics::MotionWrite::Angles(axis) => (axis, entities.columns.angles[slot]),
            qa_session::physics::MotionWrite::AngularVelocity(axis) => (
                axis,
                entities.columns.angular_velocity[slot].ok_or(CallError::Memory)?,
            ),
            qa_session::physics::MotionWrite::Origin(axis) => {
                (axis, entities.columns.position[slot])
            }
            qa_session::physics::MotionWrite::Velocity(axis) => {
                (axis, entities.columns.velocity[slot])
            }
        };
        let number = *vector.0.get(axis).ok_or(CallError::Memory)?;
        let target = self.quakec_scalar_store(memory, value, stage.into())?;
        store(memory, target, number.to_bits())?;
        // A native scalar can alias any later physical input, including
        // velocity/angles overlap and the last component of a vector.
        layout.refresh_motion(memory, address, entities, entity)
    }

    pub(crate) fn quakec_relink_entity(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        self.quakec_identity(memory, context, value)?;
        if value == 0 {
            // Native SV_LinkEdict ignores world0, but its pose writes still
            // belong in the common columns used by presentation.
            self.quakec_publish(services, memory, strings, context, value)?;
            return Ok(());
        }
        self.quakec_link(services, memory, strings, context, value, store)
    }
    pub(crate) fn quakec_link(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        if value == 0 {
            return Ok(());
        }
        let layout = self.quakec_layout()?;
        layout.link_fields(memory, u64::from(value))?;
        if native.slot as usize <= self.quakec_client_count()?
            && self.binding(&services.server.entities, native)?.is_none()
        {
            self.quakec_ensure_client_body(services, memory, strings, context, value)?;
        }
        let Some((entity, link_flags)) =
            self.quakec_publish(services, memory, strings, context, value)?
        else {
            return Ok(());
        };
        (ENGINE_CALLS.link)(services, context, entity, link_flags)?;
        self.quakec_publish_link_bounds(services, memory, strings, context, value, store)
    }
    pub(crate) fn quakec_publish_link_bounds(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &mut ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<(), CallError> {
        let native = self.quakec_identity(memory, context, value)?;
        let Some(entity) = self.quakec_bound_entity(&services.server.entities, native)? else {
            return Ok(());
        };
        let Some(bounds) = services.server.area.bounds(entity) else {
            return Ok(());
        };
        let layout = self.quakec_layout()?;
        // Publish the actual linked/expanded bounds, in native vector order.
        // Each completed scalar remains dirty even if a later write rejects.
        layout.store_vector(memory, u64::from(value), layout.abs_min, bounds.mins, store)?;
        layout.store_vector(memory, u64::from(value), layout.abs_max, bounds.maxs, store)?;
        self.quakec_project_row(services, memory, strings, context, native.slot as usize)
    }
    fn quakec_publish(
        &mut self,
        services: &mut EngineServices<'_>,
        memory: &ModuleMemory<'_>,
        strings: &ModuleMemory<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<Option<(EntityId, LinkFlags)>, CallError> {
        let slot = value as usize / self.quakec_layout()?.stride;
        if self.quakec_is_foreign(slot) {
            return Ok(None);
        }
        let native = self.quakec_identity(memory, context, value)?;
        let Some(entity) = self.binding(&services.server.entities, native)? else {
            return Ok(None);
        };
        let layout = self.quakec_layout()?;
        let address = u64::from(value);
        let slot = entity.slot as usize;
        let client = services.server.client_for_native(native);
        let provider_motion = context.native_rules == RuleSetId::Quake
            && client.is_some_and(|client| services.server.module_moves(client) == Some(native));
        let (motion, angular_velocity) = layout.motion_state(memory, address)?;
        let motion = if provider_motion {
            client_motion(layout.number(memory, address, layout.move_type)?)
        } else {
            motion
        };
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
            services.native_geometry,
            None,
        )?;
        let body = target.body;
        let angles = target.angles;
        let number = |field| {
            layout
                .number(memory, address, field)
                .map(|value| qa_core::math::native_integer(f64::from(value)))
        };
        let flags = if layout.flags.is_some() {
            layout.flags(memory, address)?
        } else {
            0
        };
        let solid = layout.solid(memory, address)?;
        let shape = target.shape;
        let owner = if layout.owner.is_some() {
            self.quakec_collision_owner(memory, context.module, address)?
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
        if let Some(client) = client {
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
        columns.position[slot] = body.position;
        columns.velocity[slot] = body.velocity;
        columns.mins[slot] = body.mins;
        columns.maxs[slot] = body.maxs;
        columns.angles[slot] = angles;
        use qa_core::primitives::EntityFields;
        for (present, authority) in [
            (layout.origin.is_some(), EntityFields::POSITION),
            (layout.velocity.is_some(), EntityFields::VELOCITY),
            (layout.angles.is_some(), EntityFields::ANGLES),
            (
                layout.mins.is_some() && layout.maxs.is_some(),
                EntityFields::BOUNDS,
            ),
        ] {
            if present {
                columns.published_fields[slot].insert(authority);
            }
        }
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
                let binding =
                    word.map_or_else(|| thinks.module.think(None), |word| thinks.binding(word));
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
        let integer = |field| number(field).map(|value| value.map(qa_core::math::native_integer));
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
        let environment =
            self.quakec_client_environment(&services.server.entities, memory, context, address)?;
        let mode = if context.native_rules == RuleSetId::Quake
            && services.server.module_moves(id) == Some(native)
        {
            layout
                .move_type
                .map(|field| {
                    let phase = client_motion(layout.number(memory, address, Some(field))?);
                    layout.client_mode(memory, address, phase)
                })
                .transpose()?
                .flatten()
        } else {
            None
        };
        let mut stats = [None; 32];
        if let Some(health) = health {
            stats[0] = Some(qa_core::math::native_integer(health) as u32);
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
            player.view.accept_offset(view, None);
            player.movement.view_height = view.0[2];
        }
        if let Some(punch) = punch {
            player.punch_angles = punch;
        }
        if let Some(view) = view {
            player.view_angles = view;
        }
        environment.apply(player);
        apply_client_mode(player, mode);
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
    pub(crate) fn quakec_encode_hit(
        &mut self,
        entities: &EntityTable,
        memory: &mut ModuleMemory<'_>,
        context: CallContext,
        hit: Option<EntityId>,
        store: &mut crate::quakec::EntityStore<'_>,
    ) -> Result<u32, CallError> {
        let Some(entity) = hit else {
            return Ok(0);
        };
        // Collision world is the recipient ABI's explicit world ordinal zero.
        if Some(entity) == entities.id_at(0) {
            return Ok(0);
        }
        let (slot, foreign) = self.quakec_admit_entity(entities, memory, context, entity, store)?;
        if foreign {
            self.quakec_release_alias(slot, entity)?;
        }
        u32::try_from(slot * self.quakec_layout()?.stride).map_err(|_| CallError::Entity)
    }
}

impl QuakeCLayout {
    fn solid(self, memory: &ModuleMemory<'_>, address: u64) -> Result<i32, CallError> {
        self.solid.map_or(Ok(0), |field| {
            self.number(memory, address, Some(field))
                .map(|value| qa_core::math::native_integer(f64::from(value)))
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
