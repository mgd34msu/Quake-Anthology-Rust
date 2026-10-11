//! One engine implementation of module calls, independent of module encoding.
use qa_console::{
    command_buffer::CommandBuffer,
    cvars::{Cvars, View, WriteError},
    views::Context,
};
use qa_content::vfs::{FileRef, Vfs};
use qa_core::names::NameTable;
use qa_core::primitives::ThinkTime;
use qa_core::{
    events::FrameEvent,
    primitives::{
        Body, Bounds, ClientId, CollisionOwner, CollisionShape, CvarHandle, EffectEvent, EntityId,
        GeometryId, IndexSpan, ModelRules, ModuleId, NameId, NativeDelivery, NativeEntity,
        PrintKind, SoundEvent, UserCmd, Vec3,
    },
    text::FixedText,
};
use qa_session::clients::Server;
use qa_world::{
    area::{LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, EntityTracePolicy, NativeTraceEntities, Trace, TraceQuery,
        TraceScratch, WorldTrace,
    },
    entities::AllocationPolicy,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    Aborted,
    Capacity,
    Entity,
    File,
    Cvar,
    Text,
    Geometry,
    ConfigString,
    Memory,
}
impl From<crate::memory::MemoryError> for CallError {
    fn from(_: crate::memory::MemoryError) -> Self {
        Self::Memory
    }
}

/// Supplied by the module host. No capability inherits another role's rules.
#[derive(Clone, Copy)]
pub struct CallContext {
    pub module: ModuleId,
    pub clock: ThinkTime,
    /// SERVER frame number before/after dispatch, projected from provider ticks.
    pub server_frame: u64,
    pub console: Context,
    pub allocation: AllocationPolicy,
    pub link_order: LinkOrder,
}

struct OpenFile {
    owner: ModuleId,
    reference: FileRef,
    cursor: u64,
}
struct ConfigString {
    text: FixedText<8192>,
    revision: u64,
    resource_name: Option<NameId>,
}
/// Native Q2 resource names select player-relative or explicit file paths.
pub fn sound_path(name: &[u8]) -> Result<Option<FixedText<8198>>, CallError> {
    if name.is_empty() || name.starts_with(b"*") {
        return Ok(None);
    }
    let mut path = FixedText::default();
    if let Some(explicit) = name.strip_prefix(b"#") {
        path.append_bytes(explicit).map_err(|_| CallError::Text)?;
    } else {
        path.append_bytes(b"sound/").map_err(|_| CallError::Text)?;
        path.append_bytes(name).map_err(|_| CallError::Text)?;
    }
    Ok(Some(path))
}
struct ConfigRange {
    module: ModuleId,
    span: IndexSpan,
    message: Box<[u8]>,
    written: usize,
    revision: u64,
    entity_text: Option<EntityText>,
    sounds: Option<IndexSpan>,
}
struct EntityText {
    bytes: std::sync::Arc<[u8]>,
    syntax: qa_formats::entities::EntitySyntax,
    length: usize,
    position: usize,
}

/// One load-sized store for module file handles and configstrings. Native
/// ordinals belong to each module's range, not the common entity reservation.
pub struct ServiceStorage {
    files: Box<[Option<OpenFile>]>,
    configs: Box<[ConfigString]>,
    sounds: Box<[Option<qa_core::primitives::SoundId>]>,
    sound_bank: Option<std::sync::Arc<qa_audio::Bank>>,
    ranges: Box<[ConfigRange]>,
    resource_names: NameTable,
    reader: qa_formats::archive::ArchiveReader,
    no_areas: CvarHandle,
    max_clients: CvarHandle,
}

impl ServiceStorage {
    pub fn max_clients(&self, cvars: &Cvars, rules: qa_core::primitives::RuleSetId) -> i32 {
        cvars.integer_in(self.max_clients, rules)
    }
    pub fn load(
        configs: &[(ModuleId, usize)],
        files: usize,
        cvars: &Cvars,
    ) -> Result<Self, CallError> {
        let mut total = 0usize;
        let mut ranges = Vec::with_capacity(configs.len());
        for &(module, count) in configs {
            if ranges.iter().any(|r: &ConfigRange| r.module == module) {
                return Err(CallError::ConfigString);
            }
            ranges.push(ConfigRange {
                module,
                span: IndexSpan {
                    first: u32::try_from(total).map_err(|_| CallError::Capacity)?,
                    count: u32::try_from(count).map_err(|_| CallError::Capacity)?,
                },
                message: vec![0; 0x8000].into_boxed_slice(),
                written: 0,
                revision: 0,
                entity_text: None,
                sounds: None,
            });
            total = total.checked_add(count).ok_or(CallError::Capacity)?;
        }
        if total > 65536 || files > 65535 {
            return Err(CallError::Capacity);
        }
        Ok(Self {
            files: std::iter::repeat_with(|| None).take(files).collect(),
            configs: std::iter::repeat_with(|| ConfigString {
                text: FixedText::default(),
                revision: 0,
                resource_name: None,
            })
            .take(total)
            .collect(),
            sounds: vec![None; total].into_boxed_slice(),
            sound_bank: None,
            ranges: ranges.into_boxed_slice(),
            resource_names: NameTable::load_reserved(std::iter::empty(), total + 1, total * 128)
                .map_err(|_| CallError::Capacity)?,
            reader: qa_formats::archive::ArchiveReader::default(),
            no_areas: cvars.find("cm_noAreas").ok_or(CallError::Cvar)?,
            max_clients: cvars.find("sv_maxclients").ok_or(CallError::Cvar)?,
        })
    }
    fn range(&self, module: ModuleId) -> Option<&ConfigRange> {
        self.ranges.iter().find(|range| range.module == module)
    }
    fn range_mut(&mut self, module: ModuleId) -> Option<&mut ConfigRange> {
        self.ranges.iter_mut().find(|range| range.module == module)
    }
    fn config(&self, module: ModuleId, ordinal: usize) -> Result<usize, CallError> {
        let range = self.range(module).ok_or(CallError::ConfigString)?;
        range.span.get(ordinal).ok_or(CallError::ConfigString)
    }
    fn check_range(&self, module: ModuleId, resource: IndexSpan) -> Result<usize, CallError> {
        let range = self.range(module).ok_or(CallError::ConfigString)?;
        if !resource.fits(range.span.count as usize) {
            return Err(CallError::ConfigString);
        }
        Ok(range.span.first as usize + resource.first as usize)
    }
    pub fn bind_entity_text(
        &mut self,
        module: ModuleId,
        bytes: std::sync::Arc<[u8]>,
        syntax: qa_formats::entities::EntitySyntax,
    ) -> Result<(), CallError> {
        let length = qa_formats::entities::EntityTokens::new(&bytes, syntax).source_length();
        let range = self.entity_range(module)?;
        range.entity_text = Some(EntityText {
            bytes,
            syntax,
            length,
            position: 0,
        });
        Ok(())
    }
    fn entity_token(&mut self, module: ModuleId) -> Result<Option<&[u8]>, CallError> {
        let range = self.entity_range(module)?;
        let Some(source) = &mut range.entity_text else {
            return Ok(None);
        };
        let mut reader = qa_formats::entities::EntityTokens::resume(
            &source.bytes[..source.length],
            source.syntax,
            source.position,
        )
        .map_err(|_| CallError::Text)?;
        let token = reader.next_token().map_err(|_| CallError::Text)?;
        source.position = reader.position();
        Ok(token)
    }
    fn entity_range(&mut self, module: ModuleId) -> Result<&mut ConfigRange, CallError> {
        self.range_mut(module).ok_or(CallError::Text)
    }
    pub fn reset_message(&mut self, module: ModuleId) -> Result<(), CallError> {
        self.entity_range(module)?.written = 0;
        Ok(())
    }
    fn write_message(
        &mut self,
        module: ModuleId,
        write: impl FnOnce(
            &mut qa_network::message::Writer<'_>,
        ) -> Result<(), qa_network::message::Error>,
    ) -> Result<(), CallError> {
        let row = self.range_mut(module).ok_or(CallError::Memory)?;
        let mut writer = qa_network::message::Writer::append_bytes(&mut row.message, row.written)
            .map_err(|_| CallError::Capacity)?;
        write(&mut writer).map_err(|_| CallError::Capacity)?;
        row.written = writer.size();
        Ok(())
    }
    pub fn config_revision(&self, module: ModuleId) -> Result<u64, CallError> {
        self.range(module)
            .map(|range| range.revision)
            .ok_or(CallError::ConfigString)
    }
    pub fn configstring(
        &self,
        module: ModuleId,
        ordinal: usize,
    ) -> Result<(&[u8], u64), CallError> {
        let row = &self.configs[self.config(module, ordinal)?];
        Ok((row.text.as_bytes(), row.revision))
    }
    /// Attach the load-owned bank and bind each native sound row through the
    /// same path used when gamecode changes a registered name later.
    pub fn bind_sound_bank(
        &mut self,
        module: ModuleId,
        sounds: IndexSpan,
        bank: std::sync::Arc<qa_audio::Bank>,
    ) -> Result<(), CallError> {
        if sounds.count == 0 {
            return Err(CallError::ConfigString);
        }
        self.check_range(module, sounds)?;
        self.range_mut(module)
            .ok_or(CallError::ConfigString)?
            .sounds = Some(sounds);
        self.sound_bank = Some(bank);
        // A storage may hold several modules. Rebinding the bank updates all
        // registered sound ranges, so none retain handles from an older bank.
        for i in 0..self.ranges.len() {
            let owner = self.ranges[i].module;
            if let Some(range) = self.ranges[i].sounds {
                for ordinal in range.range() {
                    self.bind_sound(owner, ordinal)?;
                }
            }
        }
        Ok(())
    }
    fn bind_sound(&mut self, module: ModuleId, ordinal: usize) -> Result<(), CallError> {
        let index = self.config(module, ordinal)?;
        let Some(range) = self.range(module).and_then(|range| range.sounds) else {
            return Ok(());
        };
        if !range.contains(ordinal) {
            return Ok(());
        }
        self.bind_sound_at(index)
    }
    fn bind_sound_at(&mut self, index: usize) -> Result<(), CallError> {
        self.sounds[index] = None;
        let sound = match self
            .sound_bank
            .as_ref()
            .zip(sound_path(self.configs[index].text.as_bytes())?)
        {
            Some((bank, path)) => {
                bank.find(std::str::from_utf8(path.as_bytes()).map_err(|_| CallError::Text)?)
            }
            _ => None,
        };
        self.sounds[index] = sound;
        Ok(())
    }
    fn set_configstring(
        &mut self,
        module: ModuleId,
        ordinal: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        let index = self.config(module, ordinal)?;
        self.set_configstring_at(module, index, text)
    }
    fn set_configstring_at(
        &mut self,
        module: ModuleId,
        index: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        if text.len() > 8192 {
            return Err(CallError::Text);
        }
        let row = &mut self.configs[index];
        if row.text.as_bytes() != text {
            let name = if row.resource_name.is_some() && !text.is_empty() {
                Some(
                    self.resource_names
                        .intern(text)
                        .map_err(|_| CallError::Capacity)?,
                )
            } else {
                None
            };
            row.text.set_bytes(text).map_err(|_| CallError::Text)?;
            row.resource_name = name;
            row.revision = row.revision.wrapping_add(1);
            let range = self.range_mut(module).ok_or(CallError::ConfigString)?;
            range.revision = range.revision.wrapping_add(1);
            let ordinal = index - range.span.first as usize;
            if range.sounds.is_some_and(|sounds| sounds.contains(ordinal)) {
                self.bind_sound_at(index)?;
            }
        }
        Ok(())
    }
    fn resource_index(
        &mut self,
        module: ModuleId,
        range: IndexSpan,
        name: &[u8],
    ) -> Result<u32, CallError> {
        if name.is_empty() {
            return Ok(0);
        }
        if range.count < 2 || name.len() > 8192 {
            return Err(CallError::ConfigString);
        }
        let first = self.check_range(module, range)?;
        let name_id = self
            .resource_names
            .intern(name)
            .map_err(|_| CallError::Capacity)?;
        // SV_FindIndex stops at the first empty slot. Keep that native order,
        // including a gap made by a module's direct configstring update.
        for ordinal in 1..range.count as usize {
            let row = &mut self.configs[first + ordinal];
            if row.text.as_bytes().is_empty() {
                self.set_configstring_at(module, first + ordinal, name)?;
                self.configs[first + ordinal].resource_name = Some(name_id);
                return u32::try_from(ordinal).map_err(|_| CallError::Capacity);
            }
            let stored = if let Some(id) = row.resource_name {
                id
            } else {
                let id = self
                    .resource_names
                    .intern(row.text.as_bytes())
                    .map_err(|_| CallError::Capacity)?;
                row.resource_name = Some(id);
                id
            };
            if stored == name_id {
                return u32::try_from(ordinal).map_err(|_| CallError::Capacity);
            }
        }
        Err(CallError::Capacity)
    }
}

/// References are borrowed for a module call. The server, cvar table, command
/// buffer, VFS, geometry and output ring remain their existing single owners.
pub struct EngineServices<'a> {
    pub server: &'a mut Server,
    pub cvars: &'a mut Cvars,
    pub commands: &'a mut CommandBuffer,
    pub vfs: &'a Vfs,
    pub localization: &'a qa_content::localization::Catalog,
    pub registry: &'a qa_gameplay::registry::Registry,
    pub storage: &'a mut ServiceStorage,
    pub geometry: &'a CollisionStore,
    pub world: Option<(GeometryId, u32)>,
    pub visibility: Option<(
        &'a qa_world::visibility::VisibilityWorld,
        &'a mut qa_world::leaves::LeafScratch,
        &'a mut [u8],
    )>,
    pub portals: Option<&'a mut qa_world::portals::AreaPortals>,
    pub scratch: &'a mut TraceScratch,
}

/// These typed entries are shared by every numbered ABI table. Boundary
/// adapters only decode arguments and convert the native result layout.
pub type PrintCall = fn(
    &mut EngineServices<'_>,
    Option<qa_core::events::OutputTarget>,
    Option<ClientId>,
    PrintKind,
    u8,
    &[u8],
) -> Result<u64, CallError>;
pub type LocalizedPrintCall = for<'a> fn(
    &mut EngineServices<'_>,
    Option<qa_core::events::OutputTarget>,
    Option<ClientId>,
    PrintKind,
    u8,
    &[u8],
    usize,
    &dyn Fn(usize) -> Result<Option<&'a [u8]>, CallError>,
) -> Result<u64, CallError>;
pub type FileOpenCall =
    fn(&mut EngineServices<'_>, ModuleId, &[u8]) -> Result<(u32, u64), CallError>;
pub type CvarRegisterCall =
    fn(&mut EngineServices<'_>, Context, &str, Option<&str>, u32) -> Result<View, CallError>;
pub type ResourceIndexCall =
    fn(&mut EngineServices<'_>, ModuleId, IndexSpan, &[u8]) -> Result<u32, CallError>;
pub type AreaQueryCall =
    fn(&EngineServices<'_>, Bounds, LinkFlags, &mut dyn FnMut(EntityId) -> bool);
pub type EntityTokenCall =
    for<'a> fn(&'a mut EngineServices<'_>, ModuleId) -> Result<Option<&'a [u8]>, CallError>;
/// A direct clip selects one target, independently of area linking and owners.
pub enum ClipTarget {
    World,
    Body {
        body: Body,
        shape: CollisionShape,
        angles: Vec3,
        rules: ModelRules,
    },
}
pub struct EngineCallTable {
    pub pmove: fn(
        &mut EngineServices<'_>,
        ClientId,
        Option<&dyn NativeTraceEntities>,
    ) -> Result<qa_movement::MovementResult, CallError>,
    pub in_visibility: fn(
        &mut EngineServices<'_>,
        Vec3,
        Vec3,
        qa_world::visibility::PointVisibility,
        bool,
    ) -> Result<bool, CallError>,
    pub native_message:
        fn(&mut EngineServices<'_>, NativeDelivery) -> Result<Option<u64>, CallError>,
    pub message_bits: fn(&mut EngineServices<'_>, ModuleId, u32, u8) -> Result<(), CallError>,
    pub message_string: fn(&mut EngineServices<'_>, ModuleId, &[u8]) -> Result<(), CallError>,
    pub area_query: AreaQueryCall,
    pub portal_set: fn(&mut EngineServices<'_>, u32, bool, bool) -> Result<bool, CallError>,
    pub portal_adjust: fn(&mut EngineServices<'_>, u32, u32, bool) -> Result<(), CallError>,
    pub areas_connected: fn(&EngineServices<'_>, u32, u32) -> Option<bool>,
    pub box_leaves: fn(
        &mut EngineServices<'_>,
        Bounds,
        &mut [u32],
    ) -> Result<qa_world::leaves::BoxLeaves, CallError>,
    pub print: PrintCall,
    pub localized_print: LocalizedPrintCall,
    pub console: fn(&mut EngineServices<'_>, &[u8]) -> Result<u64, CallError>,
    pub console_formatted:
        fn(&mut EngineServices<'_>, std::fmt::Arguments<'_>) -> Result<u64, CallError>,
    pub usercmd: fn(&EngineServices<'_>, NativeEntity) -> Option<UserCmd>,
    pub sound: fn(&mut EngineServices<'_>, SoundEvent) -> Result<u64, CallError>,
    pub sound_id: fn(
        &EngineServices<'_>,
        ModuleId,
        IndexSpan,
        i32,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError>,
    pub effect: fn(&mut EngineServices<'_>, EffectEvent) -> Result<u64, CallError>,
    pub trace: fn(
        &mut EngineServices<'_>,
        TraceQuery<'_>,
        Option<&dyn NativeTraceEntities>,
    ) -> Result<Trace, CallError>,
    pub clip: fn(&mut EngineServices<'_>, TraceQuery<'_>, ClipTarget) -> Result<Trace, CallError>,
    pub point_contents: fn(
        &mut EngineServices<'_>,
        Vec3,
        EntityTracePolicy,
        CollisionOwner,
    ) -> Result<Contents, CallError>,
    pub link:
        fn(&mut EngineServices<'_>, CallContext, EntityId, LinkFlags) -> Result<bool, CallError>,
    pub unlink: fn(&mut EngineServices<'_>, EntityId) -> Result<(), CallError>,
    pub bot_registration:
        fn(&mut EngineServices<'_>, Option<EntityId>, bool) -> Result<(), CallError>,
    pub spawn: fn(&mut EngineServices<'_>, CallContext) -> Result<EntityId, CallError>,
    pub free: fn(&mut EngineServices<'_>, CallContext, EntityId) -> Result<(), CallError>,
    pub cvar_register: CvarRegisterCall,
    pub cvar_view: fn(&mut EngineServices<'_>, View) -> Result<u32, CallError>,
    pub cvar_snapshot: fn(&EngineServices<'_>, u32) -> Result<(View, i32), CallError>,
    pub cvar_latch: fn(&mut EngineServices<'_>, View) -> Result<(), CallError>,
    pub cvar_set: fn(&mut EngineServices<'_>, View, &str) -> Result<(), CallError>,
    pub cvar_force: fn(&mut EngineServices<'_>, View, &str) -> Result<(), CallError>,
    pub cvar_reset: fn(&mut EngineServices<'_>, View, bool) -> Result<(), CallError>,
    pub cvar_number: fn(&EngineServices<'_>, View) -> Result<f32, CallError>,
    pub cvar_integer: fn(&EngineServices<'_>, View) -> Result<i32, CallError>,
    pub command: fn(&mut EngineServices<'_>, Context, &str) -> Result<(), CallError>,
    pub command_insert: fn(&mut EngineServices<'_>, Context, &str) -> Result<(), CallError>,
    pub configstring: fn(&mut EngineServices<'_>, ModuleId, usize, &[u8]) -> Result<(), CallError>,
    pub resource_index: ResourceIndexCall,
    pub file_open: FileOpenCall,
    pub entity_token: EntityTokenCall,
    pub file_length: fn(&mut EngineServices<'_>, &[u8]) -> Result<u64, CallError>,
    pub file_read:
        fn(&mut EngineServices<'_>, ModuleId, u32, &mut [u8]) -> Result<usize, CallError>,
    pub file_seek:
        fn(&mut EngineServices<'_>, ModuleId, u32, std::io::SeekFrom) -> Result<bool, CallError>,
    pub file_close: fn(&mut EngineServices<'_>, ModuleId, u32) -> Result<(), CallError>,
}

pub const ENGINE_CALLS: EngineCallTable = EngineCallTable {
    pmove: |services, id, native| services.pmove(id, native),
    in_visibility: |s, first, second, policy, check_portals| {
        let (world, _, row) = s.visibility.as_mut().ok_or(CallError::Geometry)?;
        world
            .test_points(
                first,
                second,
                row,
                policy,
                s.portals.as_deref().filter(|_| check_portals),
            )
            .map_err(|_| CallError::Geometry)
    },
    native_message: |s, delivery| {
        let row = s.storage.entity_range(delivery.module)?;
        if row.written == 0 {
            return Ok(None);
        }
        let result = s
            .server
            .events
            .native_message(delivery, &row.message[..row.written]);
        // Native multicast/unicast consumes staging even on bounded admission failure.
        row.written = 0;
        result.map(Some).map_err(|_| CallError::Capacity)
    },
    message_bits: |s, module, value, width| {
        s.storage
            .write_message(module, |w| w.write_bits(value, width))
    },
    message_string: |s, module, bytes| {
        s.storage.write_message(module, |w| {
            w.write_data(bytes)?;
            w.write_bits(0, 8)
        })
    },
    portal_set: |s, number, open, strict| {
        let Some(portals) = s.portals.as_mut() else {
            return Ok(false);
        };
        match portals.set(number, open) {
            Err(qa_world::portals::PortalError::Number) if !strict => Ok(false),
            result => result.map_err(|_| CallError::Geometry),
        }
    },
    portal_adjust: |s, first, second, open| {
        s.portals
            .as_mut()
            .ok_or(CallError::Geometry)?
            .adjust(first, second, open)
            .map_err(|_| CallError::Geometry)
    },
    areas_connected: |s, first, second| {
        s.world?;
        s.portals
            .as_ref()
            .map_or(Some(true), |p| p.connected(first, second))
    },
    area_query: |s, bounds, flags, accept| {
        for row in s.server.area.query(&s.server.entities, bounds, flags) {
            if !accept(row.id) {
                break;
            }
        }
    },
    box_leaves: |s, bounds, output| {
        let (world, scratch, _) = s.visibility.as_mut().ok_or(CallError::Geometry)?;
        world
            .box_leaves(bounds, output, scratch)
            .ok_or(CallError::Capacity)
    },
    print: |s, target, c, k, l, t| s.print(target, c, k, l, t),
    localized_print: |s, target, client, kind, level, base, count, argument| {
        let mut text = qa_formats::localization::LocText::default();
        s.localization.format(base, count, argument, &mut text)?;
        s.print(target, client, kind, level, text.as_bytes())
    },
    console: |s, text| s.console(text),
    console_formatted: |s, text| EngineServices::console_formatted(&mut s.server.events, text),
    usercmd: |s, native| s.usercmd(native),
    sound: |s, e| s.sound(e),
    sound_id: |s, module, range, index| {
        let index = usize::try_from(index).map_err(|_| CallError::Text)?;
        let ordinal = range.get(index).ok_or(CallError::Text)?;
        let row = s.storage.config(module, ordinal)?;
        Ok(s.storage.sounds[row])
    },
    effect: |s, e| s.effect(e),
    trace: |s, q, native| s.trace(q, native),
    clip: |s, q, target| s.clip(q, target),
    point_contents: |s, p, rules, pass| s.point_contents(p, rules, pass),
    link: |s, c, e, f| s.link(c, e, f),
    unlink: |s, e| s.unlink(e),
    bot_registration: |s, e, registered| s.bot_registration(e, registered),
    spawn: |s, c| s.spawn(c),
    free: |s, c, e| s.free(c, e),
    cvar_register: |s, c, n, d, f| s.cvar_register(c, n, d, f),
    cvar_view: |s, view| s.cvars.native_view(view).map_err(|_| CallError::Capacity),
    cvar_snapshot: |s, handle| s.cvars.native_snapshot(handle).ok_or(CallError::Cvar),
    cvar_latch: |s, view| s.cvars.apply_latch(view).map_err(|_| CallError::Cvar),
    cvar_set: |s, v, t| s.cvar_set(v, t),
    cvar_force: |s, v, t| s.cvar_force(v, t),
    cvar_reset: |s, v, force| s.cvar_reset(v, force),
    cvar_number: |s, view| s.cvars.numeric(view).map_err(|_| CallError::Cvar),
    cvar_integer: |s, view| s.cvars.integer_view(view).map_err(|_| CallError::Cvar),
    command: |s, c, t| s.command(c, t),
    command_insert: |s, c, t| s.commands.insert(t, c).map_err(|_| CallError::Text),
    configstring: |s, m, i, t| s.configstring(m, i, t),
    resource_index: |s, m, r, n| s.storage.resource_index(m, r, n),
    file_open: |s, m, p| s.file_open(m, p),
    entity_token: |s, m| s.storage.entity_token(m),
    file_length: |s, p| s.file_length(p),
    file_read: |s, m, h, b| s.file_read(m, h, b),
    file_seek: |s, m, h, from| s.file_seek(m, h, from),
    file_close: |s, m, h| s.file_close(m, h),
};

impl EngineServices<'_> {
    pub fn pmove(
        &mut self,
        id: ClientId,
        native: Option<&dyn NativeTraceEntities>,
    ) -> Result<qa_movement::MovementResult, CallError> {
        let (geometry, index) = self.world.ok_or(CallError::Geometry)?;
        let client = self
            .server
            .clients
            .get_mut(id.0 as usize)
            .filter(|c| c.connection.is_some())
            .ok_or(CallError::Entity)?;
        let mut trace = WorldTrace::new(
            self.geometry,
            geometry,
            index,
            &self.server.entities,
            &self.server.area,
            self.scratch,
            CollisionOwner::Lifetime(client.entity),
        )
        .with_native_entities(native);
        Ok(qa_movement::pmove(
            client.command,
            &mut client.player,
            &mut trace,
        ))
    }
    pub fn ignore_areas(&self, source: qa_core::primitives::RuleSetId, integer: bool) -> bool {
        if integer {
            self.cvars.integer_in(self.storage.no_areas, source) != 0
        } else {
            self.cvars.value_in(self.storage.no_areas, source) != 0.0
        }
    }
    pub fn print(
        &mut self,
        target: Option<qa_core::events::OutputTarget>,
        client: Option<ClientId>,
        kind: PrintKind,
        level: u8,
        text: &[u8],
    ) -> Result<u64, CallError> {
        self.server
            .events
            .print_level_bytes(target, client, kind, level, text)
            .map_err(|_| CallError::Capacity)
    }
    pub fn console(&mut self, text: &[u8]) -> Result<u64, CallError> {
        self.server
            .events
            .console_bytes(text)
            .map_err(|_| CallError::Capacity)
    }
    pub fn console_formatted(
        events: &mut qa_core::events::EventRing,
        text: std::fmt::Arguments<'_>,
    ) -> Result<u64, CallError> {
        events
            .console_formatted(text)
            .map_err(|_| CallError::Capacity)
    }
    pub fn usercmd(&self, native: NativeEntity) -> Option<UserCmd> {
        let client = self.server.client_for_native(native)?;
        Some(self.server.clients[client.0 as usize].command)
    }
    pub fn sound(&mut self, event: SoundEvent) -> Result<u64, CallError> {
        self.server
            .events
            .push(FrameEvent::Sound(event))
            .map_err(|_| CallError::Capacity)
    }
    pub fn effect(&mut self, event: EffectEvent) -> Result<u64, CallError> {
        self.server
            .events
            .push(FrameEvent::Effect(event))
            .map_err(|_| CallError::Capacity)
    }
    pub fn trace(
        &mut self,
        query: TraceQuery<'_>,
        native: Option<&dyn NativeTraceEntities>,
    ) -> Result<Trace, CallError> {
        let (geometry, index) = self.world.ok_or(CallError::Geometry)?;
        Ok(WorldTrace::new(
            self.geometry,
            geometry,
            index,
            &self.server.entities,
            &self.server.area,
            self.scratch,
            CollisionOwner::None,
        )
        .with_native_entities(native)
        .trace(query))
    }
    pub fn clip(&mut self, query: TraceQuery<'_>, target: ClipTarget) -> Result<Trace, CallError> {
        Ok(match target {
            ClipTarget::World => {
                let (geometry, index) = self.world.ok_or(CallError::Geometry)?;
                self.geometry
                    .trace_model(geometry, index, query, self.scratch)
            }
            ClipTarget::Body {
                body,
                shape,
                angles,
                rules,
            } => self
                .geometry
                .trace_body(query, shape, &body, angles, rules, None, self.scratch),
        })
    }
    pub fn point_contents(
        &mut self,
        point: Vec3,
        rules: EntityTracePolicy,
        pass: CollisionOwner,
    ) -> Result<Contents, CallError> {
        let (geometry, index) = self.world.ok_or(CallError::Geometry)?;
        Ok(WorldTrace::new(
            self.geometry,
            geometry,
            index,
            &self.server.entities,
            &self.server.area,
            self.scratch,
            pass,
        )
        .point_contents(point, rules, &[]))
    }
    pub fn link(
        &mut self,
        context: CallContext,
        entity: EntityId,
        flags: LinkFlags,
    ) -> Result<bool, CallError> {
        if self.server.entities.resolve(entity).is_none() {
            return Err(CallError::Entity);
        }
        Ok(self.server.area.link(
            &self.server.entities,
            entity,
            flags,
            context.link_order,
            LinkIntent::Explicit,
        ))
    }
    pub fn unlink(&mut self, entity: EntityId) -> Result<(), CallError> {
        if self.server.entities.resolve(entity).is_none() {
            return Err(CallError::Entity);
        }
        self.server.area.unlink(entity);
        Ok(())
    }
    pub fn spawn(&mut self, context: CallContext) -> Result<EntityId, CallError> {
        let allocation = self
            .server
            .entities
            .allocate(context.clock, context.module, context.allocation)
            .ok_or(CallError::Capacity)?;
        if let Some(old) = allocation.displaced {
            self.server.area.unlink(old);
        }
        Ok(allocation.id)
    }
    pub fn bot_registration(
        &mut self,
        entity: Option<EntityId>,
        registered: bool,
    ) -> Result<(), CallError> {
        let Some(entity) = entity else {
            return if registered {
                Err(CallError::Entity)
            } else {
                Ok(())
            };
        };
        if self.server.entities.resolve(entity).is_none() {
            return Err(CallError::Entity);
        }
        if registered {
            if !self.server.navigation.register(entity) {
                return Err(CallError::Capacity);
            }
        } else {
            self.server.navigation.unregister(entity);
        }
        Ok(())
    }
    pub fn free(&mut self, context: CallContext, entity: EntityId) -> Result<(), CallError> {
        self.unlink(entity)?;
        if !self.server.entities.release(entity, context.clock) {
            return Err(CallError::Entity);
        }
        self.server.navigation.unregister(entity);
        Ok(())
    }
    pub fn cvar_register(
        &mut self,
        context: Context,
        name: &str,
        default: Option<&str>,
        flags: u32,
    ) -> Result<View, CallError> {
        self.cvars
            .register(name, default, flags, context)
            .map_err(|error| {
                if error == WriteError::Capacity {
                    CallError::Capacity
                } else {
                    CallError::Cvar
                }
            })
    }
    pub fn cvar_set(&mut self, view: View, text: &str) -> Result<(), CallError> {
        self.cvars.write(view, text).map_err(|_| CallError::Cvar)
    }
    pub fn cvar_force(&mut self, view: View, text: &str) -> Result<(), CallError> {
        self.cvars
            .force_write(view, text)
            .map_err(|_| CallError::Cvar)
    }
    pub fn cvar_reset(&mut self, view: View, force: bool) -> Result<(), CallError> {
        let reason = match self.cvars.reset_view(view, force) {
            Ok(()) => return Ok(()),
            Err(WriteError::ReadOnly) => "read only",
            Err(WriteError::InitOnly) => "write protected",
            Err(WriteError::Cheats) => "cheat protected",
            Err(_) => return Err(CallError::Cvar),
        };
        // Native protection rejects the write, not the calling module. A full
        // output ring counts its dropped warning without aborting the import.
        let _ = Self::console_formatted(
            &mut self.server.events,
            format_args!("{} is {reason}.\n", self.cvars.name(view)),
        );
        Ok(())
    }
    pub fn command(&mut self, context: Context, text: &str) -> Result<(), CallError> {
        self.commands
            .append(text, context)
            .map_err(|_| CallError::Text)
    }
    pub fn configstring(
        &mut self,
        module: ModuleId,
        ordinal: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        self.storage.set_configstring(module, ordinal, text)
    }
    pub fn file_open(&mut self, owner: ModuleId, path: &[u8]) -> Result<(u32, u64), CallError> {
        let reference = self.vfs.open(path).ok_or(CallError::File)?;
        let length = self.vfs.length(reference).map_err(|_| CallError::File)?;
        let slot = self
            .storage
            .files
            .iter()
            .position(Option::is_none)
            .ok_or(CallError::Capacity)?;
        self.storage.files[slot] = Some(OpenFile {
            owner,
            reference,
            cursor: 0,
        });
        Ok((slot as u32 + 1, length))
    }
    pub fn file_length(&self, path: &[u8]) -> Result<u64, CallError> {
        let reference = self.vfs.open(path).ok_or(CallError::File)?;
        self.vfs.length(reference).map_err(|_| CallError::File)
    }
    fn file(&mut self, owner: ModuleId, handle: u32) -> Result<&mut OpenFile, CallError> {
        self.storage
            .files
            .get_mut(handle.wrapping_sub(1) as usize)
            .and_then(Option::as_mut)
            .filter(|f| f.owner == owner)
            .ok_or(CallError::File)
    }
    pub fn file_read(
        &mut self,
        owner: ModuleId,
        handle: u32,
        buffer: &mut [u8],
    ) -> Result<usize, CallError> {
        let file = self.file(owner, handle)?;
        let (reference, offset) = (file.reference, file.cursor);
        if offset >= self.vfs.length(reference).map_err(|_| CallError::File)? {
            return Ok(0);
        }
        let read = self
            .vfs
            .read_range_reusing(reference, offset, buffer, &mut self.storage.reader)
            .map_err(|_| CallError::File)?;
        self.file(owner, handle)?.cursor += read as u64;
        Ok(read)
    }
    pub fn file_seek(
        &mut self,
        owner: ModuleId,
        handle: u32,
        from: std::io::SeekFrom,
    ) -> Result<bool, CallError> {
        let file = self.file(owner, handle)?;
        let (reference, cursor) = (file.reference, file.cursor);
        let target = match from {
            std::io::SeekFrom::Start(offset) => Some(offset),
            std::io::SeekFrom::Current(offset) => cursor.checked_add_signed(offset),
            std::io::SeekFrom::End(offset) => self
                .vfs
                .length(reference)
                .map_err(|_| CallError::File)?
                .checked_add_signed(offset),
        };
        let Some(target) = target else {
            return Ok(false);
        };
        self.file(owner, handle)?.cursor = target;
        Ok(true)
    }
    pub fn file_close(&mut self, owner: ModuleId, handle: u32) -> Result<(), CallError> {
        self.file(owner, handle)?;
        self.storage.files[handle as usize - 1] = None;
        Ok(())
    }
}
