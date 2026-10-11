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
        Bounds, ClientId, EffectEvent, EntityId, GeometryId, ModuleId, NameId, PrintKind,
        SoundEvent, Vec3,
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
/// Native ordinals are relative to a caller-selected configuration range.
#[derive(Clone, Copy)]
pub struct ResourceRange {
    pub first: usize,
    pub count: usize,
}
struct ConfigRange {
    module: ModuleId,
    first: usize,
    count: usize,
}

/// One load-sized store for module file handles and configstrings. Native
/// ordinals belong to each module's range, not the common entity reservation.
pub struct ServiceStorage {
    files: Box<[Option<OpenFile>]>,
    configs: Box<[ConfigString]>,
    ranges: Box<[ConfigRange]>,
    resource_names: NameTable,
    reader: qa_formats::archive::ArchiveReader,
}

impl ServiceStorage {
    pub fn load(configs: &[(ModuleId, usize)], files: usize) -> Result<Self, CallError> {
        let mut total = 0usize;
        let mut ranges = Vec::with_capacity(configs.len());
        for &(module, count) in configs {
            if ranges.iter().any(|r: &ConfigRange| r.module == module) {
                return Err(CallError::ConfigString);
            }
            ranges.push(ConfigRange {
                module,
                first: total,
                count,
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
            ranges: ranges.into_boxed_slice(),
            resource_names: NameTable::load_reserved(std::iter::empty(), total + 1, total * 128)
                .map_err(|_| CallError::Capacity)?,
            reader: qa_formats::archive::ArchiveReader::default(),
        })
    }
    fn config(&self, module: ModuleId, ordinal: usize) -> Result<usize, CallError> {
        let range = self
            .ranges
            .iter()
            .find(|r| r.module == module)
            .ok_or(CallError::ConfigString)?;
        if ordinal >= range.count {
            return Err(CallError::ConfigString);
        }
        Ok(range.first + ordinal)
    }
    pub fn configstring(
        &self,
        module: ModuleId,
        ordinal: usize,
    ) -> Result<(&[u8], u64), CallError> {
        let row = &self.configs[self.config(module, ordinal)?];
        Ok((row.text.as_bytes(), row.revision))
    }
    fn set_configstring(
        &mut self,
        module: ModuleId,
        ordinal: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        let index = self.config(module, ordinal)?;
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
        }
        Ok(())
    }
    fn resource_index(
        &mut self,
        module: ModuleId,
        range: ResourceRange,
        name: &[u8],
    ) -> Result<u32, CallError> {
        if name.is_empty() {
            return Ok(0);
        }
        let end = range
            .first
            .checked_add(range.count)
            .ok_or(CallError::ConfigString)?;
        if range.count < 2 || name.len() > 8192 {
            return Err(CallError::ConfigString);
        }
        let first = self.config(module, range.first)?;
        self.config(module, end - 1)?;
        let name_id = self
            .resource_names
            .intern(name)
            .map_err(|_| CallError::Capacity)?;
        // SV_FindIndex stops at the first empty slot. Keep that native order,
        // including a gap made by a module's direct configstring update.
        for ordinal in 1..range.count {
            let row = &mut self.configs[first + ordinal];
            if row.text.as_bytes().is_empty() {
                self.set_configstring(module, range.first + ordinal, name)?;
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
    pub storage: &'a mut ServiceStorage,
    pub geometry: &'a CollisionStore,
    pub world: Option<(GeometryId, u32)>,
    pub visibility: Option<(
        &'a qa_world::visibility::VisibilityWorld,
        &'a mut qa_world::leaves::LeafScratch,
    )>,
    pub scratch: &'a mut TraceScratch,
}

/// These typed entries are shared by every numbered ABI table. Boundary
/// adapters only decode arguments and convert the native result layout.
pub type PrintCall =
    fn(&mut EngineServices<'_>, Option<ClientId>, PrintKind, &[u8]) -> Result<u64, CallError>;
pub type FileOpenCall =
    fn(&mut EngineServices<'_>, ModuleId, &[u8]) -> Result<(u32, u64), CallError>;
pub type CvarRegisterCall =
    fn(&mut EngineServices<'_>, Context, &str, Option<&str>, u32) -> Result<View, CallError>;
pub type ResourceIndexCall =
    fn(&mut EngineServices<'_>, ModuleId, ResourceRange, &[u8]) -> Result<u32, CallError>;
pub type AreaQueryCall =
    fn(&EngineServices<'_>, Bounds, LinkFlags, &mut dyn FnMut(EntityId) -> bool);
pub struct EngineCallTable {
    pub area_query: AreaQueryCall,
    pub box_leaves: fn(
        &mut EngineServices<'_>,
        Bounds,
        &mut [u32],
    ) -> Result<qa_world::leaves::BoxLeaves, CallError>,
    pub print: PrintCall,
    pub sound: fn(&mut EngineServices<'_>, SoundEvent) -> Result<u64, CallError>,
    pub effect: fn(&mut EngineServices<'_>, EffectEvent) -> Result<u64, CallError>,
    pub trace: fn(
        &mut EngineServices<'_>,
        TraceQuery<'_>,
        Option<&dyn NativeTraceEntities>,
    ) -> Result<Trace, CallError>,
    pub point_contents:
        fn(&mut EngineServices<'_>, Vec3, EntityTracePolicy) -> Result<Contents, CallError>,
    pub link:
        fn(&mut EngineServices<'_>, CallContext, EntityId, LinkFlags) -> Result<bool, CallError>,
    pub unlink: fn(&mut EngineServices<'_>, EntityId) -> Result<(), CallError>,
    pub bot_registration:
        fn(&mut EngineServices<'_>, Option<EntityId>, bool) -> Result<(), CallError>,
    pub spawn: fn(&mut EngineServices<'_>, CallContext) -> Result<EntityId, CallError>,
    pub free: fn(&mut EngineServices<'_>, CallContext, EntityId) -> Result<(), CallError>,
    pub cvar_register: CvarRegisterCall,
    pub cvar_set: fn(&mut EngineServices<'_>, View, &str) -> Result<(), CallError>,
    pub cvar_force: fn(&mut EngineServices<'_>, View, &str) -> Result<(), CallError>,
    pub command: fn(&mut EngineServices<'_>, Context, &str) -> Result<(), CallError>,
    pub configstring: fn(&mut EngineServices<'_>, ModuleId, usize, &[u8]) -> Result<(), CallError>,
    pub resource_index: ResourceIndexCall,
    pub file_open: FileOpenCall,
    pub file_length: fn(&mut EngineServices<'_>, &[u8]) -> Result<u64, CallError>,
    pub file_read:
        fn(&mut EngineServices<'_>, ModuleId, u32, &mut [u8]) -> Result<usize, CallError>,
    pub file_close: fn(&mut EngineServices<'_>, ModuleId, u32) -> Result<(), CallError>,
}

pub const ENGINE_CALLS: EngineCallTable = EngineCallTable {
    area_query: |s, bounds, flags, accept| {
        for row in s.server.area.query(&s.server.entities, bounds, flags) {
            if !accept(row.id) {
                break;
            }
        }
    },
    box_leaves: |s, bounds, output| {
        let (world, scratch) = s.visibility.as_mut().ok_or(CallError::Geometry)?;
        world
            .box_leaves(bounds, output, scratch)
            .ok_or(CallError::Capacity)
    },
    print: |s, c, k, t| s.print(c, k, t),
    sound: |s, e| s.sound(e),
    effect: |s, e| s.effect(e),
    trace: |s, q, native| s.trace(q, native),
    point_contents: |s, p, rules| s.point_contents(p, rules),
    link: |s, c, e, f| s.link(c, e, f),
    unlink: |s, e| s.unlink(e),
    bot_registration: |s, e, registered| s.bot_registration(e, registered),
    spawn: |s, c| s.spawn(c),
    free: |s, c, e| s.free(c, e),
    cvar_register: |s, c, n, d, f| s.cvar_register(c, n, d, f),
    cvar_set: |s, v, t| s.cvar_set(v, t),
    cvar_force: |s, v, t| s.cvar_force(v, t),
    command: |s, c, t| s.command(c, t),
    configstring: |s, m, i, t| s.configstring(m, i, t),
    resource_index: |s, m, r, n| s.storage.resource_index(m, r, n),
    file_open: |s, m, p| s.file_open(m, p),
    file_length: |s, p| s.file_length(p),
    file_read: |s, m, h, b| s.file_read(m, h, b),
    file_close: |s, m, h| s.file_close(m, h),
};

impl EngineServices<'_> {
    pub fn print(
        &mut self,
        client: Option<ClientId>,
        kind: PrintKind,
        text: &[u8],
    ) -> Result<u64, CallError> {
        self.server
            .events
            .print_bytes(client, kind, text)
            .map_err(|_| CallError::Capacity)
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
            None,
        )
        .with_native_entities(native)
        .trace(query))
    }
    pub fn point_contents(
        &mut self,
        point: Vec3,
        rules: EntityTracePolicy,
    ) -> Result<Contents, CallError> {
        let (geometry, index) = self.world.ok_or(CallError::Geometry)?;
        Ok(WorldTrace::new(
            self.geometry,
            geometry,
            index,
            &self.server.entities,
            &self.server.area,
            self.scratch,
            None,
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
        let read = self
            .vfs
            .read_range_reusing(reference, offset, buffer, &mut self.storage.reader)
            .map_err(|_| CallError::File)?;
        self.file(owner, handle)?.cursor += read as u64;
        Ok(read)
    }
    pub fn file_close(&mut self, owner: ModuleId, handle: u32) -> Result<(), CallError> {
        self.file(owner, handle)?;
        self.storage.files[handle as usize - 1] = None;
        Ok(())
    }
}
