//! One engine implementation of module calls, independent of module encoding.
use qa_console::{
    command_buffer::CommandBuffer,
    cvars::{Cvars, View, WriteError},
    views::{Context, Role},
};
use qa_content::vfs::{FileRef, Vfs};
use qa_core::names::NameTable;
use qa_core::primitives::ThinkTime;
use qa_core::text::{InfoBytes, InfoRules};
use qa_core::{
    configstrings::{ConfigLayout, ConfigRef, ConfigStrings, ResourceIndexPolicy},
    events::FrameEvent,
    primitives::{
        Body, Bounds, ClientId, CollisionOwner, CollisionShape, CvarHandle, EffectEvent, EntityId,
        EntityRecord, GeometryId, IndexSpan, ModelRules, ModuleId, NATIVE_PAYLOAD_BYTES,
        NATIVE_WIRE_BYTES, NameId, NativeDelivery, NativeEntity, Orientation, PrintKind, RuleSetId,
        SoundEvent, UserCmd, Vec3,
    },
};
use qa_session::clients::Server;
use qa_world::collision::store::BodyTraceMode;
use qa_world::{
    area::{LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, EntityTracePolicy, NativeTraceEntities, Trace, TraceQuery,
        TraceScratch, WorldTrace,
    },
    entities::AllocationPolicy,
};

mod drop_client;
mod input;
pub use drop_client::DropClientTarget;
mod native_message;
mod signon;
pub use signon::{AmbientSound, SIGNON_BYTES, SignonCursor};

pub const Q3_USERINFO_BYTES: usize = 1024;

/// Native module admission chooses this policy once, independently of the
/// player's movement, connection protocol and console view.
pub fn userinfo_rules(source: RuleSetId) -> InfoRules {
    if let Some(layout) = crate::q2_abi::client_layout(source) {
        return InfoRules {
            bytes: layout.userinfo_bytes,
            key_bytes: crate::q2_abi::INFO_KEY_BYTES,
            value_bytes: layout.value_bytes,
            filter: InfoBytes::Ascii7Printable,
        };
    }
    let (bytes, fields) = if source == RuleSetId::Quake3 {
        (Q3_USERINFO_BYTES, Q3_USERINFO_BYTES)
    } else {
        (crate::quakec::QW_USERINFO_BYTES, 64)
    };
    InfoRules {
        bytes,
        key_bytes: fields,
        value_bytes: fields,
        filter: InfoBytes::Exact,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallError {
    Aborted,
    Unsupported,
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
impl From<qa_render::marks::MarkError<CallError>> for CallError {
    fn from(error: qa_render::marks::MarkError<CallError>) -> Self {
        match error {
            qa_render::marks::MarkError::Boundary(error) => error,
            qa_render::marks::MarkError::Invalid => Self::Geometry,
            qa_render::marks::MarkError::Scratch => Self::Capacity,
        }
    }
}
impl From<qa_session::clients::ClientInfoError> for CallError {
    fn from(error: qa_session::clients::ClientInfoError) -> Self {
        match error {
            qa_session::clients::ClientInfoError::Client => Self::Entity,
            qa_session::clients::ClientInfoError::Info(qa_core::text::InfoError::Invalid) => {
                Self::Text
            }
            qa_session::clients::ClientInfoError::Info(qa_core::text::InfoError::Capacity) => {
                Self::Capacity
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedWorldBinding {
    pub render: qa_render::world::WorldId,
    pub collision: Option<GeometryId>,
}

/// A CLIENT module selects collision independently of its renderer and SERVER.
#[derive(Default)]
pub struct ClientCollision {
    selected: Option<GeometryId>,
    temporary: Bounds,
    capsule: Option<Bounds>,
}
impl ClientCollision {
    pub fn geometry(&self) -> Option<GeometryId> {
        self.selected
    }
    pub fn reset(&mut self) {
        self.selected = None;
        self.temporary = Bounds::default();
        self.capsule = None;
    }
}

/// The cold and frozen model catalog share the same native metadata owner.
pub trait ModelLookup {
    /// Cold retained resources capture the file before their CLIENT profile is bound.
    fn file_handle(&self, path: &[u8]) -> Option<i32> {
        let _ = path;
        None
    }
    fn inline_handle(&self, world: qa_render::world::WorldId, index: u32) -> Option<i32> {
        let _ = (world, index);
        None
    }
    fn loaded_world(
        &self,
        name: &[u8],
        _search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<LoadedWorldBinding>;
    fn bounds(
        &self,
        path: &[u8],
        source: RuleSetId,
        search: Option<&qa_content::vfs::SearchView>,
    ) -> Option<Bounds>;
    fn shader_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32>;
    fn skin_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32> {
        let _ = (name, source);
        None
    }
    fn model_handle(&self, name: &[u8], source: RuleSetId) -> Option<i32>;
    fn model_bounds(&self, handle: i32) -> Option<Bounds>;
    fn model_has_tags(&self, handle: i32) -> bool;
    fn lerp_tag(
        &self,
        handle: i32,
        start: i32,
        end: i32,
        fraction: f32,
        name: &[u8],
    ) -> Option<Orientation>;
}
pub trait ModelLoader {
    fn lookup(&self) -> &dyn ModelLookup;
    fn set_image_search_order(&mut self, order: qa_render::material::resources::ImageSearchOrder);
    fn set_model_replacement_load(&mut self, enhanced: bool, skeletal: bool);

    fn register_inline(
        &mut self,
        world: qa_render::world::WorldId,
        index: u32,
    ) -> Result<i32, CallError> {
        self.lookup()
            .inline_handle(world, index)
            .ok_or(CallError::File)
    }
    fn register_skin(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
    ) -> Result<i32, CallError> {
        let _ = (vfs, name, source);
        Err(CallError::Unsupported)
    }
    fn precache(
        &mut self,
        vfs: &Vfs,
        path: &[u8],
        source: RuleSetId,
        search: Option<&qa_content::vfs::SearchView>,
    ) -> Result<Bounds, CallError>;
    fn register_shader(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
        mipmaps: bool,
        lighting: crate::services::ShaderLighting,
    ) -> Result<i32, CallError>;
    fn register_model(
        &mut self,
        vfs: &Vfs,
        name: &[u8],
        source: RuleSetId,
    ) -> Result<i32, CallError> {
        self.precache(vfs, name, source, None)?;
        self.lookup()
            .model_handle(name, source)
            .ok_or(CallError::File)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderLighting {
    TwoDimensional,
    World(Option<qa_render::world::WorldPresentationId>),
}

#[derive(Clone, Copy, Debug)]
pub enum RendererRegistration {
    Model,
    Skin,
    Shader {
        mipmaps: bool,
        lighting: ShaderLighting,
    },
}

#[derive(Clone, Debug)]
pub struct RendererRequest<N = qa_core::text::FixedText<64>> {
    /// Retained file selection for dependencies; None selects the engine default.
    pub search: Option<qa_content::vfs::SearchView>,
    pub name: N,
    pub source: RuleSetId,
    pub kind: RendererRegistration,
    pub world: Option<qa_render::world::WorldId>,
}

/// Sound registration extends the one retained bank through scoped service borrows.
pub trait SoundRegistrar {
    fn register(
        &mut self,
        vfs: &Vfs,
        sample: &[u8],
        source: RuleSetId,
        search: Option<&qa_content::vfs::SearchView>,
    ) -> Result<i32, CallError>;
    /// Native config resources admit PCM without claiming Q3 engine handles.
    fn precache(
        &mut self,
        vfs: &Vfs,
        sample: &[u8],
        source: RuleSetId,
        search: Option<&qa_content::vfs::SearchView>,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError>;
    fn source(
        &self,
        view: qa_content::vfs::ReadView<'_>,
    ) -> Result<Option<qa_audio::SoundSource>, CallError>;
    fn bank(&self) -> &qa_audio::Bank;
}

/// A native disconnect retains transport; a rejected request terminates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientTermination {
    ServerDisconnect,
    Rejected,
}

/// Accepted CLIENT snapshots and current command argv borrow the same
/// connection for one synchronous module call, without reading SERVER state.
pub trait ClientConnection {
    fn identity(&self, server: &Server) -> Result<(ClientId, EntityId), CallError>;
    fn send_command(&mut self, text: &[u8], server: &mut Server) -> Result<bool, CallError>;
    fn command_number(&self, server: &Server) -> Result<u64, CallError>;
    fn usercmd(&mut self, number: i32, server: &Server) -> Result<Option<UserCmd>, CallError>;
    fn select(
        &mut self,
        number: i32,
        server: &mut Server,
        bank: Option<&qa_audio::Bank>,
    ) -> Result<bool, CallError>;
    fn arguments(&self) -> qa_console::command_text::Arguments<'_>;
    fn disconnected(&self) -> Option<ClientTermination>;
    fn configstrings(&self, server: &Server) -> Result<&ConfigStrings, CallError>;
    fn snapshot_number(&self, server: &Server) -> Result<(u32, ThinkTime), CallError>;
    fn snapshot(
        &mut self,
        sequence: i32,
        server: &Server,
    ) -> Result<Option<qa_network::snapshots::Q3ClientSnapshot<'_>>, CallError>;
}

/// Synchronous CLIENT audio publication uses the receiving connection's
/// existing loop state and the shared output ring, never the mixer directly.
pub trait ClientAudio {
    fn publish(
        &mut self,
        events: &mut qa_core::events::EventRing,
        event: SoundEvent,
        native_entity: Option<i32>,
        doppler_enabled: bool,
    ) -> Result<(), CallError>;
    fn respatialize(
        &mut self,
        events: &mut qa_core::events::EventRing,
        producer: ModuleId,
        queued_at: qa_core::sys_events::EventTime,
        native_entity: u32,
        origin: Vec3,
        right: Vec3,
    ) -> Result<(), CallError>;
}

/// Supplied by the module host. No capability inherits another role's rules.
#[derive(Clone, Copy)]
pub struct CallContext {
    pub module: ModuleId,
    pub clock: ThinkTime,
    /// SERVER frame number before/after dispatch, projected from provider ticks.
    pub server_frame: u64,
    /// Native ABI, limits and source provenance are independent of cvar views.
    pub native_rules: RuleSetId,
    /// Collision behaviour belongs to the calling trace role.
    pub trace_rules: RuleSetId,
    /// Spatial sound behaviour belongs to the emitting role.
    pub sound_policy: qa_core::primitives::SpatialPolicy,
    pub console: Context,
    pub allocation: AllocationPolicy,
    pub link_order: LinkOrder,
}

struct OpenFile {
    owner: ModuleId,
    reference: FileRef,
    cursor: u64,
}
#[derive(Clone, Copy)]
struct FreeClientClosing {
    started: qa_core::sys_events::EventTime,
    in_callback: bool,
}
struct ConfigRange {
    module: ModuleId,
    lifetime: Option<EntityId>,
    native_rules: RuleSetId,
    client_capacity: Option<u32>,
    free_closing: Box<[Option<FreeClientClosing>]>,
    client_first: i32,
    info: InfoRules,
    info_scratch: Box<[u8]>,
    span: IndexSpan,
    files: Option<IndexSpan>,
    file_search: Option<qa_content::vfs::FileSearch>,
    resources: Option<[IndexSpan; 3]>,
    resource_policies: [ResourceIndexPolicy; 3],
    message: Box<[u8]>,
    written: usize,
    revision: u64,
    entity_text: Option<EntityText>,
    sounds: Option<IndexSpan>,
    sound_bindings: Option<qa_audio::SoundBindings>,
    sound_revisions: Box<[Option<u64>]>,
    sound_attached: bool,
    music_owner: Option<qa_core::primitives::MusicOwner>,
    signon: signon::SignonRegistry,
}
impl ConfigRange {
    fn bound_sound(&self, ordinal: u32) -> Option<qa_core::primitives::SoundId> {
        self.sound_bindings.as_ref()?.sound(ordinal)
    }
    fn sound_ordinal(&self, index: usize) -> Option<usize> {
        let sounds = self.sounds?;
        index
            .checked_sub(self.span.first as usize)?
            .checked_sub(sounds.first as usize)
            .filter(|&ordinal| ordinal < sounds.count as usize)
    }
    fn attach_sounds(&mut self, bank: &qa_audio::Bank) -> Result<(), CallError> {
        if !self.sound_attached {
            self.sound_bindings
                .as_mut()
                .ok_or(CallError::ConfigString)?
                .attach(bank);
            self.sound_attached = true;
        }
        Ok(())
    }
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
    configs: ConfigStrings,
    resource_keys: Box<[Option<NameId>]>,
    ranges: Box<[ConfigRange]>,
    resource_names: NameTable,
    reader: qa_formats::archive::ArchiveReader,
    file_listing: qa_content::vfs::FileListing,
    no_areas: CvarHandle,
    max_clients: CvarHandle,
    zombie_time: CvarHandle,
    doppler: CvarHandle,
    movement: qa_console::movement::MovementHandles,
    collision: qa_console::collision::CollisionHandles,
    qw_phs: CvarHandle,
    image_search: View,
    model: qa_console::model::ModelHandles,
}

impl ServiceStorage {
    pub fn prepare_files(&mut self, vfs: &Vfs) -> Result<(), CallError> {
        self.file_listing
            .prepare(vfs)
            .map_err(|_| CallError::Capacity)?;
        let default = vfs.bind_search(None).map_err(|_| CallError::File)?;
        for range in &mut self.ranges {
            range.file_search.get_or_insert_with(|| default.clone());
        }
        Ok(())
    }

    pub fn file_search(&self, module: ModuleId) -> Option<&qa_content::vfs::SearchView> {
        self.range(module)
            .and_then(|range| range.file_search.as_ref())
            .map(qa_content::vfs::FileSearch::view)
    }
    pub fn file_source(&self, module: ModuleId) -> Option<qa_core::primitives::FileSearchId> {
        self.range(module)
            .and_then(|range| range.file_search.as_ref())
            .and_then(qa_content::vfs::FileSearch::id)
    }
    /// Bind an immutable source order at module load/restart. Rebinding closes
    /// this module's old handles; None restores the default shared search.
    pub fn set_file_search(
        &mut self,
        module: ModuleId,
        view: Option<qa_content::vfs::SearchView>,
        vfs: &Vfs,
    ) -> Result<(), CallError> {
        let view = vfs
            .bind_search(view.as_ref())
            .map_err(|_| CallError::File)?;
        let range = self
            .ranges
            .iter_mut()
            .find(|range| range.module == module)
            .ok_or(CallError::File)?;
        if range
            .file_search
            .as_ref()
            .map_or(view.id().is_some(), |old| {
                !old.view().same_selection(view.view())
            })
        {
            range.sound_revisions.fill(None);
            if let Some(bindings) = &mut range.sound_bindings {
                bindings.clear();
            }
        }
        range.file_search = Some(view);
        for file in self
            .files
            .iter_mut()
            .filter(|file| file.as_ref().is_some_and(|file| file.owner == module))
        {
            *file = None;
        }
        Ok(())
    }
    pub fn max_clients(&self, cvars: &Cvars, rules: qa_core::primitives::RuleSetId) -> i32 {
        cvars.integer_in(self.max_clients, rules)
    }
    pub fn load(
        configs: &[(ModuleId, usize)],
        files: usize,
        cvars: &Cvars,
    ) -> Result<Self, CallError> {
        Self::load_with(
            configs
                .iter()
                .map(|&(module, count)| (module, count, None, None)),
            files,
            cvars,
        )
    }
    pub fn load_native(
        configs: &[(ModuleId, usize, RuleSetId)],
        files: usize,
        cvars: &Cvars,
    ) -> Result<Self, CallError> {
        Self::load_with(
            configs
                .iter()
                .map(|&(module, count, rules)| (module, count, Some(rules), None)),
            files,
            cvars,
        )
    }
    /// One shared store, with native handles scoped to each module's reserved
    /// range. A module cannot consume another module's load-sized capacity.
    pub fn load_modules(
        modules: &[(ModuleId, usize, RuleSetId, usize)],
        cvars: &Cvars,
    ) -> Result<Self, CallError> {
        let files = modules.iter().try_fold(0usize, |total, row| {
            total.checked_add(row.3).ok_or(CallError::Capacity)
        })?;
        Self::load_with(
            modules
                .iter()
                .map(|&(module, count, rules, files)| (module, count, Some(rules), Some(files))),
            files,
            cvars,
        )
    }
    fn load_with(
        configs: impl Iterator<Item = (ModuleId, usize, Option<RuleSetId>, Option<usize>)>,
        files: usize,
        cvars: &Cvars,
    ) -> Result<Self, CallError> {
        let mut total = 0usize;
        let mut first_file = 0usize;
        let mut ranges = Vec::new();
        let mut capacities = Vec::new();
        for (module, count, native_rules, module_files) in configs {
            let layout = native_rules.map(ConfigLayout::for_rules);
            let resources = if count == 0 {
                None
            } else {
                layout.map(|layout| layout.resources)
            };
            if resources.is_some_and(|ranges| ranges.iter().any(|range| !range.fits(count))) {
                return Err(CallError::ConfigString);
            }
            if ranges.iter().any(|r: &ConfigRange| r.module == module) {
                return Err(CallError::ConfigString);
            }
            if layout.is_some_and(|layout| count > layout.count) {
                return Err(CallError::ConfigString);
            }
            if total.checked_add(count).is_none_or(|total| total > 65536) {
                return Err(CallError::Capacity);
            }
            for index in 0..count {
                capacities.push(
                    layout
                        .map_or(Some(qa_core::configstrings::MAX_TEXT), |layout| {
                            layout.capacity(index)
                        })
                        .ok_or(CallError::ConfigString)?,
                );
            }
            let file_span = if let Some(count) = module_files {
                let end = first_file.checked_add(count).ok_or(CallError::Capacity)?;
                if end > files || end > 65535 {
                    return Err(CallError::Capacity);
                }
                let span = IndexSpan {
                    first: first_file as u32,
                    count: count as u32,
                };
                first_file = end;
                Some(span)
            } else {
                None
            };
            let source = native_rules.unwrap_or(RuleSetId::Quake2);
            let info = userinfo_rules(source);
            ranges.push(ConfigRange {
                module,
                lifetime: None,
                native_rules: source,
                client_capacity: None,
                free_closing: Box::default(),
                client_first: i32::from(source != RuleSetId::Quake3),
                info,
                info_scratch: vec![0; info.bytes].into_boxed_slice(),
                resources,
                resource_policies: layout.map_or([ResourceIndexPolicy::DEFAULT; 3], |layout| {
                    layout.resource_policies
                }),
                span: IndexSpan {
                    first: u32::try_from(total).map_err(|_| CallError::Capacity)?,
                    count: u32::try_from(count).map_err(|_| CallError::Capacity)?,
                },
                files: file_span,
                file_search: None,
                message: vec![0; NATIVE_PAYLOAD_BYTES].into_boxed_slice(),
                written: 0,
                revision: 0,
                entity_text: None,
                music_owner: None,
                sounds: resources.map(|ranges| ranges[1]),
                sound_bindings: resources
                    .map(|ranges| qa_audio::SoundBindings::load(ranges[1].count as usize)),
                sound_revisions: vec![None; resources.map_or(0, |ranges| ranges[1].count as usize)]
                    .into_boxed_slice(),
                sound_attached: false,
                signon: signon::SignonRegistry::load(if resources.is_some() {
                    SIGNON_BYTES / 10
                } else {
                    0
                }),
            });
            total = total.checked_add(count).ok_or(CallError::Capacity)?;
        }
        if total > 65536 || files > 65535 {
            return Err(CallError::Capacity);
        }
        Ok(Self {
            files: std::iter::repeat_with(|| None).take(files).collect(),
            configs: ConfigStrings::load(capacities).map_err(|_| CallError::Capacity)?,
            resource_keys: vec![None; total].into_boxed_slice(),
            ranges: ranges.into_boxed_slice(),
            resource_names: NameTable::load_reserved(std::iter::empty(), total + 1, total * 128)
                .map_err(|_| CallError::Capacity)?,
            reader: qa_formats::archive::ArchiveReader::default(),
            file_listing: qa_content::vfs::FileListing::default(),
            no_areas: cvars.find("cm_noAreas").ok_or(CallError::Cvar)?,
            max_clients: cvars.find("sv_maxclients").ok_or(CallError::Cvar)?,
            zombie_time: cvars.find("sv_zombietime").ok_or(CallError::Cvar)?,
            doppler: cvars.find("s_doppler").ok_or(CallError::Cvar)?,
            movement: qa_console::movement::MovementHandles::bind(cvars)
                .map_err(|_| CallError::Cvar)?,
            collision: qa_console::collision::CollisionHandles::bind(cvars)
                .map_err(|_| CallError::Cvar)?,
            qw_phs: cvars.find("sv_phs").ok_or(CallError::Cvar)?,
            image_search: cvars
                .bind(
                    "r_texture_formats",
                    Context {
                        source: RuleSetId::Quake2Rerelease,
                        role: Role::Engine,
                        ..Context::default()
                    },
                )
                .ok_or(CallError::Cvar)?,
            model: qa_console::model::ModelHandles::bind(cvars).map_err(|_| CallError::Cvar)?,
        })
    }
    fn range(&self, module: ModuleId) -> Option<&ConfigRange> {
        self.ranges.iter().find(|range| range.module == module)
    }
    fn range_mut(&mut self, module: ModuleId) -> Option<&mut ConfigRange> {
        self.ranges.iter_mut().find(|range| range.module == module)
    }
    pub fn retire_music(
        &mut self,
        module: ModuleId,
        events: &mut qa_core::events::EventRing,
    ) -> Result<(), CallError> {
        let Some(row) = self.range_mut(module) else {
            return Ok(());
        };
        if let Some(owner) = row.music_owner {
            events
                .music_control(qa_core::primitives::MusicControl::Retire(owner))
                .map_err(|_| CallError::Capacity)?;
            row.music_owner = None;
        }
        Ok(())
    }
    /// End one guest instance while retaining its module namespace and resources.
    pub fn reset_instance(&mut self, module: ModuleId) -> Result<(), CallError> {
        let span = self.file_span(module)?;
        for file in &mut self.files[span.range()] {
            if file.as_ref().is_some_and(|file| file.owner == module) {
                *file = None;
            }
        }
        let row = self.range_mut(module).ok_or(CallError::ConfigString)?;
        row.written = 0;
        if let Some(text) = &mut row.entity_text {
            text.position = 0;
        }
        Ok(())
    }

    /// Freeze the native client boundary before the first module callback.
    /// Identical rebinding preserves a level's retained client records.
    pub fn bind_client_capacity(
        &mut self,
        module: ModuleId,
        capacity: usize,
    ) -> Result<(), CallError> {
        let capacity = u32::try_from(capacity)
            .ok()
            .filter(|&capacity| capacity != 0 && capacity <= i32::MAX as u32)
            .ok_or(CallError::Capacity)?;
        let range = self.range_mut(module).ok_or(CallError::Entity)?;
        if range.client_capacity.is_some_and(|old| old != capacity) {
            return Err(CallError::Capacity);
        }
        if range.client_capacity.is_none() {
            range.free_closing = vec![None; capacity as usize].into_boxed_slice();
        }
        range.client_capacity = Some(capacity);
        Ok(())
    }
    pub fn client_capacity(&self, module: ModuleId) -> Option<u32> {
        self.range(module)?.client_capacity
    }
    /// A new native admission ends the free slot's prior close lifetime.
    pub fn reset_native_close(&mut self, native: NativeEntity) -> Result<(), CallError> {
        let range = self.range_mut(native.module).ok_or(CallError::Entity)?;
        let ordinal = native
            .slot
            .checked_sub(range.client_first)
            .and_then(|ordinal| usize::try_from(ordinal).ok())
            .ok_or(CallError::Entity)?;
        *range
            .free_closing
            .get_mut(ordinal)
            .ok_or(CallError::Entity)? = None;
        Ok(())
    }
    fn client_info(&self, native: NativeEntity) -> Result<&ConfigRange, CallError> {
        let range = self.range(native.module).ok_or(CallError::Entity)?;
        let limit = range.client_capacity.ok_or(CallError::Entity)?;
        if native.slot < range.client_first || (native.slot - range.client_first) as u32 >= limit {
            return Err(CallError::Entity);
        }
        Ok(range)
    }
    pub fn userinfo<'a>(
        &self,
        server: &'a Server,
        native: NativeEntity,
    ) -> Result<&'a [u8], CallError> {
        self.client_info(native)?;
        let Some(id) = server.client_for_native(native) else {
            return Ok(b"");
        };
        let client = &server.clients[id.0 as usize];
        if client.module != native.module {
            return Err(CallError::Entity);
        }
        server.userinfo(id).ok_or(CallError::Entity)
    }
    pub fn set_userinfo(
        &self,
        server: &mut Server,
        native: NativeEntity,
        text: &[u8],
    ) -> Result<(), CallError> {
        let range = self.client_info(native)?;
        let id = server.client_for_native(native).ok_or(CallError::Entity)?;
        if server.clients[id.0 as usize].module != native.module {
            return Err(CallError::Entity);
        }
        server.replace_userinfo_raw(id, text, range.info.bytes)?;
        Ok(())
    }
    fn serverinfo(
        &mut self,
        cvars: &Cvars,
        module: ModuleId,
        output: &mut [u8],
    ) -> Result<usize, CallError> {
        if output.is_empty() {
            return Err(CallError::Memory);
        }
        let range = self.range_mut(module).ok_or(CallError::Entity)?;
        let length = cvars
            .info_string(
                Context {
                    source: range.native_rules,
                    side: qa_console::catalog::Scope::Server,
                    role: qa_console::views::Role::Game,
                    ..Context::default()
                },
                4,
                range.info,
                &mut range.info_scratch,
            )
            .map_err(|_| CallError::Cvar)?;
        let copied = length.min(output.len() - 1);
        output[..copied].copy_from_slice(&range.info_scratch[..copied]);
        output[copied] = 0;
        Ok(copied)
    }
    fn file_span(&self, module: ModuleId) -> Result<IndexSpan, CallError> {
        if let Some(span) = self.range(module).and_then(|range| range.files) {
            return Ok(span);
        }
        // Scoped module loads only admit their registered namespaces. The
        // standalone loader retains its shared owner-checked handle pool.
        if self
            .ranges
            .first()
            .is_some_and(|range| range.files.is_some())
        {
            return Err(CallError::File);
        }
        Ok(IndexSpan {
            first: 0,
            count: self.files.len() as u32,
        })
    }
    fn file_index(&self, module: ModuleId, handle: u32) -> Result<usize, CallError> {
        self.file_span(module)?
            .get(handle.wrapping_sub(1) as usize)
            .ok_or(CallError::File)
    }
    pub fn bind_lifetime(&mut self, module: ModuleId, lifetime: EntityId) -> Result<(), CallError> {
        let range = self.range_mut(module).ok_or(CallError::ConfigString)?;
        if range.lifetime.is_some_and(|old| old != lifetime) {
            range.signon.reset();
            range.free_closing.fill(None);
        }
        range.lifetime = Some(lifetime);
        Ok(())
    }
    pub fn module_lifetime(&self, module: ModuleId) -> Option<EntityId> {
        self.range(module)?.lifetime
    }
    /// An interrupted synchronous export cannot leave a FREE callback active.
    /// Its original closing timestamp still governs the next independent call.
    pub fn abort_client_callbacks(&mut self, module: ModuleId) {
        if let Some(range) = self.range_mut(module) {
            for closing in range.free_closing.iter_mut().flatten() {
                closing.in_callback = false;
            }
        }
    }
    pub fn emission(&self, context: CallContext) -> Option<qa_core::primitives::NativeEmission> {
        Some(qa_core::primitives::NativeEmission {
            module_lifetime: self.range(context.module)?.lifetime?,
            provider_frame: context.server_frame,
        })
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
        let mut writer = qa_network::message::Writer::append_bytes(
            &mut row.message[..NATIVE_WIRE_BYTES],
            row.written,
        )
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
    pub fn config_count(&self, module: ModuleId) -> Result<usize, CallError> {
        self.range(module)
            .map(|range| range.span.count as usize)
            .ok_or(CallError::ConfigString)
    }
    pub fn resources(&self, module: ModuleId) -> Option<[IndexSpan; 3]> {
        self.range(module)?.resources
    }
    /// Exact resource identities are cached when native config rows change.
    pub fn bound_model_name(&self, module: ModuleId, ordinal: u32) -> Option<NameId> {
        let module = self.range(module)?;
        let models = module.resources?[0];
        let local = models.get(ordinal as usize)?;
        let index = module.span.get(local)?;
        self.resource_keys.get(index).copied().flatten()
    }
    pub fn resource_name(&self, id: NameId) -> Option<&[u8]> {
        self.resource_names.get(id)
    }
    pub fn resource_name_capacity(&self) -> usize {
        self.resource_names.capacity()
    }
    pub fn uses_sound_phs(&self, cvars: &Cvars) -> bool {
        cvars.value_in(self.qw_phs, RuleSetId::QuakeWorld) != 0.0
    }
    pub fn config_row(&self, module: ModuleId, ordinal: usize) -> Result<ConfigRef<'_>, CallError> {
        self.configs
            .config(self.config(module, ordinal)?)
            .ok_or(CallError::ConfigString)
    }
    pub fn configstring(
        &self,
        module: ModuleId,
        ordinal: usize,
    ) -> Result<(&[u8], u64), CallError> {
        let row = self.config_row(module, ordinal)?;
        Ok((row.text, row.revision))
    }
    /// Numeric configstrings are projected once when their text changes.
    /// A module's undeclared ordinal carries no configuration value.
    pub fn config_integer(&self, module: ModuleId, ordinal: usize) -> Option<i32> {
        let index = self.range(module)?.span.get(ordinal)?;
        Some(self.configs.config(index)?.integer)
    }
    /// Native ordinals share the load-bound cache with module sound imports.
    pub fn sound_id(
        &self,
        module: ModuleId,
        range: IndexSpan,
        index: i32,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError> {
        let index = usize::try_from(index).map_err(|_| CallError::Text)?;
        let ordinal = range.get(index).ok_or(CallError::Text)?;
        self.config(module, ordinal)?;
        let row = self.range(module).ok_or(CallError::ConfigString)?;
        if row.sounds.is_some_and(|sounds| sounds != range) {
            return Err(CallError::ConfigString);
        }
        Ok(row.bound_sound(index as u32))
    }
    /// Load-bound PCM identity; a native ordinal never becomes a SoundId cast.
    pub fn bound_sound(
        &self,
        module: ModuleId,
        ordinal: u32,
    ) -> Option<qa_core::primitives::SoundId> {
        self.range(module)?.bound_sound(ordinal)
    }
    /// Attach one native namespace once. Prepared source failures remain bound to None.
    pub fn bind_sound_bank(
        &mut self,
        module: ModuleId,
        sounds: IndexSpan,
        bank: &qa_audio::Bank,
        source: Option<qa_audio::SoundSource>,
    ) -> Result<(), CallError> {
        if sounds.count == 0 {
            return Err(CallError::ConfigString);
        }
        self.check_range(module, sounds)?;
        let row = self
            .ranges
            .iter_mut()
            .find(|row| row.module == module)
            .ok_or(CallError::ConfigString)?;
        if row.sounds != Some(sounds) {
            if row.sound_attached {
                return Err(CallError::ConfigString);
            }
            row.sound_bindings = Some(qa_audio::SoundBindings::load(sounds.count as usize));
            row.sound_revisions = vec![None; sounds.count as usize].into_boxed_slice();
        }
        row.sounds = Some(sounds);
        row.attach_sounds(bank)?;
        for ordinal in 0..sounds.count as usize {
            let index = row
                .span
                .get(sounds.get(ordinal).ok_or(CallError::ConfigString)?)
                .ok_or(CallError::ConfigString)?;
            let config = self.configs.config(index).ok_or(CallError::ConfigString)?;
            if row.sound_revisions[ordinal] != Some(config.revision) {
                row.sound_bindings
                    .as_mut()
                    .ok_or(CallError::ConfigString)?
                    .bind_name_in(
                        Some(bank),
                        source,
                        ordinal as u32,
                        config.text,
                        row.native_rules,
                    )
                    .map_err(|_| CallError::Text)?;
                row.sound_revisions[ordinal] = Some(config.revision);
            }
        }
        Ok(())
    }
    fn set_configstring_at(
        &mut self,
        module: ModuleId,
        index: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        if text.len()
            > self
                .configs
                .capacity(index)
                .ok_or(CallError::ConfigString)?
            || text.contains(&0)
        {
            return Err(CallError::Text);
        }
        let row = self.configs.config(index).ok_or(CallError::ConfigString)?;
        if row.text != text {
            let range = self.range(module).ok_or(CallError::ConfigString)?;
            let ordinal = index - range.span.first as usize;
            let resource = range
                .resources
                .is_some_and(|ranges| ranges.iter().any(|range| range.contains(ordinal)));
            let name = if (resource || self.resource_keys[index].is_some()) && !text.is_empty() {
                Some(
                    self.resource_names
                        .intern(text)
                        .map_err(|_| CallError::Capacity)?,
                )
            } else {
                None
            };
            self.configs.set(index, text).map_err(|_| CallError::Text)?;
            self.resource_keys[index] = name;
            let range = self
                .ranges
                .iter_mut()
                .find(|range| range.module == module)
                .ok_or(CallError::ConfigString)?;
            range.revision = range.revision.wrapping_add(1);
            let ordinal = index - range.span.first as usize;
            if range.sounds.is_some_and(|sounds| sounds.contains(ordinal)) {
                let sound = range.sound_ordinal(index).ok_or(CallError::ConfigString)?;
                range.sound_revisions[sound] = None;
                range
                    .sound_bindings
                    .as_mut()
                    .ok_or(CallError::ConfigString)?
                    .set(sound as u32, None)
                    .map_err(|_| CallError::Text)?;
            }
        }
        Ok(())
    }
    fn resource_lookup(
        &mut self,
        module: ModuleId,
        range: IndexSpan,
        name: &[u8],
        create: bool,
    ) -> Result<Option<(u32, bool)>, CallError> {
        if name.is_empty() {
            return Ok(Some((0, false)));
        }
        if range.count < 2 || name.len() > 8192 {
            return Err(CallError::ConfigString);
        }
        let first = self.check_range(module, range)?;
        let module_range = self.range(module).ok_or(CallError::ConfigString)?;
        let policy = if let Some(resources) = module_range.resources {
            let kind = resources
                .iter()
                .position(|&resource| resource == range)
                .ok_or(CallError::ConfigString)?;
            module_range.resource_policies[kind]
        } else {
            ResourceIndexPolicy::DEFAULT
        };
        let name_id = if create {
            self.resource_names
                .intern(name)
                .map_err(|_| CallError::Capacity)?
        } else if let Some(name) = self.resource_names.find(name) {
            name
        } else {
            return Ok(None);
        };
        // SV_FindIndex stops at the first empty slot. Keep that native order,
        // including a gap made by a module's direct configstring update.
        for ordinal in 0..range.count as usize {
            if !policy.permits(ordinal as u32) {
                continue;
            }
            let row = self
                .configs
                .config(first + ordinal)
                .ok_or(CallError::ConfigString)?;
            if row.text.is_empty() {
                if !create {
                    return Ok(None);
                }
                self.set_configstring_at(module, first + ordinal, name)?;
                self.resource_keys[first + ordinal] = Some(name_id);
                return Ok(Some((
                    u32::try_from(ordinal).map_err(|_| CallError::Capacity)?,
                    true,
                )));
            }
            let stored = if let Some(id) = self.resource_keys[first + ordinal] {
                id
            } else {
                let id = self
                    .resource_names
                    .intern(row.text)
                    .map_err(|_| CallError::Capacity)?;
                self.resource_keys[first + ordinal] = Some(id);
                id
            };
            if stored == name_id {
                return Ok(Some((
                    u32::try_from(ordinal).map_err(|_| CallError::Capacity)?,
                    false,
                )));
            }
        }
        if create {
            Err(CallError::Capacity)
        } else {
            Ok(None)
        }
    }
}

/// References are borrowed for a module call. The server, cvar table, command
/// buffer, VFS, geometry and output ring remain their existing single owners.
#[derive(Clone, Copy)]
pub struct ForeignTraceRead<'a> {
    pub view: &'a dyn NativeTraceEntities,
    pub error: &'a crate::entities::TraceError,
}
impl ForeignTraceRead<'_> {
    pub fn complete<T>(read: Option<Self>, result: Result<T, CallError>) -> Result<T, CallError> {
        if let Some(read) = read {
            read.error.take()?;
        }
        result
    }
}

pub struct EngineServices<'a> {
    pub server: &'a mut Server,
    pub callbacks: Option<&'a dyn qa_session::dispatch::CallbackResolver>,
    pub client_disconnect: Option<qa_core::primitives::CallbackId>,
    pub input: &'a mut qa_input::Input,
    pub editor: Option<&'a mut qa_console::editor::Editor>,
    pub cvars: &'a mut Cvars,
    pub commands: &'a mut CommandBuffer,
    pub client_connection: Option<&'a mut dyn ClientConnection>,
    pub client_collision: Option<&'a mut ClientCollision>,
    pub client_audio: Option<&'a mut dyn ClientAudio>,
    pub renderer: Option<&'a qa_core::presentation::RendererConfiguration>,
    pub scene: Option<&'a mut dyn crate::renderer::SceneService>,
    pub vfs: &'a Vfs,
    pub model_loader: Option<&'a mut dyn ModelLoader>,
    pub model_lookup: Option<&'a dyn ModelLookup>,
    pub sound_loader: Option<&'a mut dyn SoundRegistrar>,
    pub sound_bank: Option<&'a qa_audio::Bank>,
    pub localization: &'a qa_content::localization::Catalog,
    pub registry: &'a qa_gameplay::registry::Registry,
    pub names: &'a mut NameTable,
    pub storage: &'a mut ServiceStorage,
    pub geometry: &'a CollisionStore,
    pub world: Option<(GeometryId, u32)>,
    pub native_geometry: Option<GeometryId>,
    pub foreign_trace: Option<ForeignTraceRead<'a>>,
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
pub type ClientSnapshotCall =
    for<'a> fn(
        &'a mut EngineServices<'_>,
        i32,
    ) -> Result<Option<qa_network::snapshots::Q3ClientSnapshot<'a>>, CallError>;
pub type ClientConfigStringsCall =
    for<'a> fn(&'a EngineServices<'_>) -> Result<&'a ConfigStrings, CallError>;
pub type CvarRegisterCall =
    fn(&mut EngineServices<'_>, Context, &str, Option<&str>, u32) -> Result<View, CallError>;
pub type ResourceIndexCall =
    fn(&mut EngineServices<'_>, ModuleId, IndexSpan, &[u8]) -> Result<u32, CallError>;
pub type AreaQueryCall =
    fn(&EngineServices<'_>, Bounds, LinkFlags, &mut dyn FnMut(EntityId) -> bool);
pub type EntityTokenCall =
    for<'a> fn(&'a mut EngineServices<'_>, ModuleId) -> Result<Option<&'a [u8]>, CallError>;
pub type UserinfoCall =
    for<'a> fn(&'a EngineServices<'_>, NativeEntity) -> Result<&'a [u8], CallError>;
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
    pub key_down: fn(&EngineServices<'_>, qa_core::sys_events::SeatId, i32) -> bool,
    pub key_catchers: fn(&EngineServices<'_>, qa_core::sys_events::SeatId) -> u32,
    pub set_key_catchers: fn(
        &mut EngineServices<'_>,
        qa_core::sys_events::SeatId,
        u32,
        ModuleId,
        qa_core::sys_events::EventTime,
    ) -> Result<(), CallError>,
    pub key_binding: for<'a> fn(&'a EngineServices<'_>, i32) -> &'a [u8],
    pub set_key_binding: fn(
        &mut EngineServices<'_>,
        i32,
        &[u8],
        Context,
        qa_core::sys_events::EventTime,
    ) -> Result<(), CallError>,
    pub key_for_binding: fn(&EngineServices<'_>, Option<&[u8]>) -> i32,
    pub key_name: fn(i32, &mut qa_core::text::FixedText<32>) -> Result<(), CallError>,
    pub clear_keys: fn(
        &mut EngineServices<'_>,
        qa_core::sys_events::SeatId,
        Context,
        qa_core::sys_events::EventTime,
    ) -> Result<(), CallError>,
    pub overstrike: fn(&EngineServices<'_>) -> Result<bool, CallError>,
    pub set_overstrike: fn(&mut EngineServices<'_>, bool) -> Result<(), CallError>,
    pub client_collision_load:
        fn(&mut EngineServices<'_>, ModuleId, &[u8]) -> Result<(), CallError>,
    pub client_collision_count: fn(&EngineServices<'_>) -> Result<u32, CallError>,
    pub client_collision_inline: fn(&EngineServices<'_>, i32) -> Result<i32, CallError>,
    pub client_collision_temp: fn(&mut EngineServices<'_>, Bounds, bool) -> Result<i32, CallError>,
    pub client_collision_ready: fn(&EngineServices<'_>) -> Result<bool, CallError>,
    pub client_collision_target:
        fn(&EngineServices<'_>, i32, bool) -> Result<(CollisionShape, Bounds), CallError>,
    pub client_collision_contents: fn(
        &EngineServices<'_>,
        Vec3,
        (CollisionShape, Bounds),
        Vec3,
        Vec3,
        EntityTracePolicy,
        BodyTraceMode,
    ) -> Result<Contents, CallError>,
    pub client_collision_trace: fn(
        &mut EngineServices<'_>,
        TraceQuery<'_>,
        (CollisionShape, Bounds),
        Vec3,
        Vec3,
        BodyTraceMode,
    ) -> Result<Trace, CallError>,
    pub renderer_world: fn(&mut EngineServices<'_>, ModuleId, &[u8]) -> Result<u32, CallError>,
    pub mark_fragments: fn(
        &mut EngineServices<'_>,
        &qa_render::marks::MarkProjection,
        usize,
        usize,
        &mut dyn FnMut(IndexSpan, &[Vec3]) -> Result<(), CallError>,
    ) -> Result<usize, CallError>,
    pub clear_scene: fn(&mut EngineServices<'_>),
    pub add_light: fn(&mut EngineServices<'_>, qa_render::scene::Light) -> bool,
    pub add_entity: fn(&mut EngineServices<'_>, crate::renderer::EntitySubmission) -> bool,
    pub add_poly:
        fn(&mut EngineServices<'_>, i32, usize, &mut dyn FnMut(usize) -> qa_render::Vertex) -> bool,
    pub scene_color: fn(&mut EngineServices<'_>, Option<[f32; 4]>),
    pub scene_picture: fn(&mut EngineServices<'_>, i32, [f32; 4], [f32; 4]) -> bool,
    pub scene_view:
        fn(&mut EngineServices<'_>, crate::renderer::SceneView) -> Result<bool, CallError>,
    pub userinfo: UserinfoCall,
    pub set_userinfo: fn(&mut EngineServices<'_>, NativeEntity, &[u8]) -> Result<(), CallError>,
    pub serverinfo: fn(&mut EngineServices<'_>, ModuleId, &mut [u8]) -> Result<usize, CallError>,
    pub send_server_command: fn(
        &mut EngineServices<'_>,
        ModuleId,
        RuleSetId,
        Option<NativeEntity>,
        &[u8],
    ) -> Result<(), CallError>,
    pub send_client_command: fn(&mut EngineServices<'_>, &[u8]) -> Result<bool, CallError>,
    pub drop_client_target: fn(
        &mut EngineServices<'_>,
        NativeEntity,
        qa_core::sys_events::EventTime,
    ) -> Result<Option<DropClientTarget>, CallError>,
    pub begin_drop_client: fn(
        &mut EngineServices<'_>,
        DropClientTarget,
        &[u8],
        qa_core::sys_events::EventTime,
    ) -> Result<(), CallError>,
    pub finish_drop_client:
        fn(&mut EngineServices<'_>, DropClientTarget, &[u8]) -> Result<(), CallError>,
    pub server_command: fn(&mut EngineServices<'_>, i32) -> Result<bool, CallError>,
    pub client_configstrings: ClientConfigStringsCall,
    pub renderer: for<'a> fn(
        &'a EngineServices<'_>,
    )
        -> Result<&'a qa_core::presentation::RendererConfiguration, CallError>,
    pub snapshot_number: fn(&EngineServices<'_>) -> Result<(u32, ThinkTime), CallError>,
    pub snapshot: ClientSnapshotCall,
    pub client_command_number: fn(&EngineServices<'_>) -> Result<u64, CallError>,
    pub client_usercmd: fn(&mut EngineServices<'_>, i32) -> Result<Option<UserCmd>, CallError>,
    pub pmove: fn(
        &mut EngineServices<'_>,
        ClientId,
        Option<&dyn NativeTraceEntities>,
        qa_movement::MoveOptions,
    ) -> Result<qa_movement::MovementResult, CallError>,
    pub in_visibility: fn(
        &mut EngineServices<'_>,
        Vec3,
        Vec3,
        qa_world::visibility::PointVisibility,
        bool,
    ) -> Result<bool, CallError>,
    pub native_message: fn(
        &mut EngineServices<'_>,
        NativeDelivery,
        qa_core::primitives::SpatialPolicy,
    ) -> Result<Option<u64>, CallError>,
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
    pub set_usercmd_value: fn(
        &mut EngineServices<'_>,
        qa_core::sys_events::SeatId,
        Option<qa_core::primitives::WeaponId>,
        f32,
    ),
    pub sound: fn(&mut EngineServices<'_>, NativeDelivery, SoundEvent) -> Result<u64, CallError>,
    pub client_sound: fn(&mut EngineServices<'_>, SoundEvent, Option<i32>) -> Result<(), CallError>,
    pub client_listener: fn(
        &mut EngineServices<'_>,
        ModuleId,
        qa_core::sys_events::EventTime,
        u32,
        Vec3,
        Vec3,
    ) -> Result<(), CallError>,
    pub local_sound: fn(&mut EngineServices<'_>, SoundEvent) -> Result<(), CallError>,
    pub background: fn(
        &mut EngineServices<'_>,
        ModuleId,
        qa_core::primitives::MusicPlayback,
        &[u8],
        &[u8],
    ) -> Result<u64, CallError>,
    pub stop_background: fn(&mut EngineServices<'_>) -> Result<u64, CallError>,
    pub register_sound:
        fn(&mut EngineServices<'_>, ModuleId, &[u8], RuleSetId) -> Result<i32, CallError>,
    pub registered_model_bounds: fn(&EngineServices<'_>, i32) -> Option<Bounds>,
    pub lerp_tag: fn(&EngineServices<'_>, i32, i32, i32, f32, &[u8]) -> Option<Orientation>,
    pub model_has_tags: fn(&EngineServices<'_>, i32) -> bool,
    pub registered_sound: fn(
        &EngineServices<'_>,
        ModuleId,
        i32,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError>,
    pub static_entity:
        fn(&mut EngineServices<'_>, CallContext, EntityRecord) -> Result<u32, CallError>,
    pub ambient: fn(&mut EngineServices<'_>, CallContext, AmbientSound) -> Result<u32, CallError>,
    pub sound_id: fn(
        &mut EngineServices<'_>,
        ModuleId,
        IndexSpan,
        i32,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError>,
    pub effect: fn(&mut EngineServices<'_>, NativeDelivery, EffectEvent) -> Result<u64, CallError>,
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
        Option<&dyn NativeTraceEntities>,
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
    pub command: fn(&mut EngineServices<'_>, Context, &[u8]) -> Result<(), CallError>,
    pub command_insert: fn(&mut EngineServices<'_>, Context, &[u8]) -> Result<(), CallError>,
    pub configstring: fn(&mut EngineServices<'_>, ModuleId, usize, &[u8]) -> Result<(), CallError>,
    pub lightstyle: fn(&mut EngineServices<'_>, ModuleId, usize, &[u8]) -> Result<(), CallError>,
    pub resource_index: ResourceIndexCall,
    pub resource_register:
        fn(&mut EngineServices<'_>, ModuleId, IndexSpan, &[u8]) -> Result<(u32, bool), CallError>,
    pub resource_find:
        fn(&mut EngineServices<'_>, ModuleId, IndexSpan, &[u8]) -> Result<Option<u32>, CallError>,
    pub model_precache:
        fn(&mut EngineServices<'_>, ModuleId, &[u8], RuleSetId) -> Result<Bounds, CallError>,
    pub model_bounds:
        fn(&EngineServices<'_>, ModuleId, &[u8], RuleSetId) -> Result<Bounds, CallError>,
    pub file_open: FileOpenCall,
    pub entity_token: EntityTokenCall,
    pub file_length: fn(&mut EngineServices<'_>, ModuleId, &[u8]) -> Result<u64, CallError>,
    pub file_list: fn(
        &mut EngineServices<'_>,
        ModuleId,
        qa_content::vfs::FileListRequest<'_>,
        &mut [u8],
    ) -> Result<usize, CallError>,
    pub file_read:
        fn(&mut EngineServices<'_>, ModuleId, u32, &mut [u8]) -> Result<usize, CallError>,
    pub file_seek:
        fn(&mut EngineServices<'_>, ModuleId, u32, std::io::SeekFrom) -> Result<bool, CallError>,
    pub file_close: fn(&mut EngineServices<'_>, ModuleId, u32) -> Result<(), CallError>,
}

pub const ENGINE_CALLS: EngineCallTable = EngineCallTable {
    key_down: |s, seat, key| s.key_down(seat, key),
    key_catchers: |s, seat| s.input.key_catchers(seat),
    set_key_catchers: |s, seat, mask, module, time| s.set_key_catchers(seat, mask, module, time),
    key_binding: |s, key| s.key_binding(key),
    set_key_binding: |s, key, text, context, time| s.set_key_binding(key, text, context, time),
    key_for_binding: |s, text| s.key_for_binding(text),
    key_name: input::key_name,
    clear_keys: |s, seat, context, time| s.clear_keys(seat, context, time),
    overstrike: |s| {
        s.editor
            .as_deref()
            .map(qa_console::editor::Editor::overstrike)
            .ok_or(CallError::Unsupported)
    },
    set_overstrike: |s, active| {
        s.editor
            .as_deref_mut()
            .ok_or(CallError::Unsupported)?
            .set_overstrike(active);
        Ok(())
    },
    client_collision_load: |s, module, name| s.client_collision_load(module, name),
    client_collision_count: |s| s.client_collision_count(),
    client_collision_inline: |s, index| s.client_collision_inline(index),
    client_collision_temp: |s, bounds, capsule| {
        let selection = s
            .client_collision
            .as_deref_mut()
            .ok_or(CallError::Unsupported)?;
        if capsule {
            selection.capsule = Some(bounds);
            Ok(254)
        } else {
            selection.temporary = bounds;
            Ok(255)
        }
    },
    client_collision_ready: |s| {
        s.client_collision
            .as_deref()
            .ok_or(CallError::Unsupported)?
            .selected
            .map_or(Ok(false), |geometry| {
                s.geometry
                    .has_bsp_nodes(geometry)
                    .map_err(|_| CallError::Geometry)
            })
    },
    client_collision_target: |s, handle, trace| s.client_collision_target(handle, trace),
    client_collision_contents: |s, point, (shape, bounds), origin, angles, caller, mode| {
        s.geometry
            .point_contents_body(
                point,
                shape,
                &Body {
                    position: origin,
                    mins: bounds.mins,
                    maxs: bounds.maxs,
                    ..Body::default()
                },
                angles,
                qa_gameplay::rules::model_rules(RuleSetId::Quake3),
                caller,
                mode,
            )
            .map_err(|_| CallError::Geometry)
    },
    client_collision_trace: |s, query, (shape, bounds), origin, angles, mode| {
        let query = s.collision_query(query);
        s.geometry
            .trace_body(
                query,
                shape,
                &Body {
                    position: origin,
                    mins: bounds.mins,
                    maxs: bounds.maxs,
                    ..Body::default()
                },
                angles,
                qa_gameplay::rules::model_rules(RuleSetId::Quake3),
                mode,
                None,
                s.scratch,
            )
            .map_err(|_| CallError::Geometry)
    },
    renderer_world: |s, module, name| Ok(s.loaded_world(module, name)?.render.0),
    mark_fragments: |s, projection, max_points, max_fragments, emit| {
        s.scene.as_deref_mut().map_or(Ok(0), |scene| {
            scene.mark_fragments(projection, max_points, max_fragments, emit)
        })
    },
    clear_scene: |s| {
        if let Some(scene) = s.scene.as_deref_mut() {
            scene.clear();
        }
    },
    add_light: |s, light| {
        s.scene
            .as_deref_mut()
            .is_some_and(|scene| scene.light(light))
    },
    add_entity: |s, entity| {
        s.scene
            .as_deref_mut()
            .is_some_and(|scene| scene.entity(entity, s.cvars))
    },
    add_poly: |s, shader, count, vertex| {
        s.scene
            .as_deref_mut()
            .is_some_and(|scene| scene.poly(shader, count, vertex))
    },
    scene_color: |s, color| {
        if let Some(scene) = s.scene.as_deref_mut() {
            scene.color(color);
        }
    },
    scene_picture: |s, shader, rect, texcoords| {
        s.scene
            .as_deref_mut()
            .is_some_and(|scene| scene.picture(shader, rect, texcoords))
    },
    scene_view: |s, view| {
        s.scene
            .as_deref_mut()
            .map_or(Ok(false), |scene| scene.view(view, s.cvars))
    },
    renderer: |s| s.renderer.ok_or(CallError::Geometry),
    userinfo: |s, native| s.storage.userinfo(s.server, native),
    set_userinfo: |s, native, text| s.storage.set_userinfo(s.server, native, text),
    serverinfo: |s, module, output| s.storage.serverinfo(s.cvars, module, output),
    send_server_command: |s, module, source, native, text| {
        let client = match native {
            None => None,
            Some(native) => {
                let Some(client) = s.server.client_for_native(native) else {
                    return Ok(());
                };
                Some(client)
            }
        };
        s.publish_server_command(client, module, source, text)
            .map(|_| ())
    },
    send_client_command: |s, text| s.send_client_command(text),
    drop_client_target: |s, native, time| s.drop_client_target(native, time),
    begin_drop_client: |s, target, reason, time| s.begin_drop_client(target, reason, time),
    finish_drop_client: |s, target, reason| s.finish_drop_client(target, reason),
    server_command: |s, number| {
        s.client_connection
            .as_deref_mut()
            .ok_or(CallError::Text)?
            .select(
                number,
                s.server,
                s.sound_loader
                    .as_deref()
                    .map(SoundRegistrar::bank)
                    .or(s.sound_bank),
            )
    },
    client_configstrings: |s| {
        s.client_connection
            .as_deref()
            .ok_or(CallError::Entity)?
            .configstrings(s.server)
    },
    snapshot_number: |s| {
        s.client_connection
            .as_deref()
            .ok_or(CallError::Entity)?
            .snapshot_number(s.server)
    },
    snapshot: |s, number| {
        s.client_connection
            .as_deref_mut()
            .ok_or(CallError::Entity)?
            .snapshot(number, s.server)
    },
    client_command_number: |s| {
        s.client_connection
            .as_deref()
            .ok_or(CallError::Entity)?
            .command_number(s.server)
    },
    client_usercmd: |s, number| {
        s.client_connection
            .as_deref_mut()
            .ok_or(CallError::Entity)?
            .usercmd(number, s.server)
    },
    pmove: |services, id, native, options| services.pmove(id, native, options),
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
    native_message: native_message::publish,
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
    set_usercmd_value: |s, seat, weapon, scale| s.input.set_usercmd_value(seat, weapon, scale),
    sound: |s, delivery, e| s.sound(delivery, e),
    client_sound: |s, event, entity| {
        let doppler = s.cvars.integer_in(s.storage.doppler, RuleSetId::Quake3) != 0;
        s.client_audio
            .as_deref_mut()
            .ok_or(CallError::Entity)?
            .publish(&mut s.server.events, event, entity, doppler)
    },
    client_listener: |s, producer, time, entity, origin, right| {
        s.client_audio
            .as_deref_mut()
            .ok_or(CallError::Entity)?
            .respatialize(&mut s.server.events, producer, time, entity, origin, right)
    },
    local_sound: |s, event| {
        s.server
            .events
            .push_to(
                qa_core::events::OutputTarget::Presentation,
                FrameEvent::Sound(event),
            )
            .map(|_| ())
            .map_err(|_| CallError::Capacity)
    },
    background: |s, module, playback, intro, loop_track| {
        let client = s
            .client_connection
            .as_deref()
            .map(|connection| connection.identity(s.server))
            .transpose()?;
        let file_search = s.storage.file_source(module);
        let row = s.storage.range_mut(module).ok_or(CallError::ConfigString)?;
        let owner = qa_core::primitives::MusicOwner {
            module: Some(module),
            client,
        };
        let sequence = s
            .server
            .events
            .music(owner, playback, intro, loop_track, file_search)
            .map_err(|_| CallError::Capacity)?;
        row.music_owner = Some(owner);
        Ok(sequence)
    },
    stop_background: |s| {
        let sequence = s
            .server
            .events
            .music_control(qa_core::primitives::MusicControl::Stop { only_active: true })
            .map_err(|_| CallError::Capacity)?;
        Ok(sequence)
    },
    register_sound: |s, module, sample, source| s.register_sound(module, sample, source),
    registered_model_bounds: |s, handle| s.models()?.model_bounds(handle),
    model_has_tags: |s, handle| {
        s.models()
            .is_some_and(|models| models.model_has_tags(handle))
    },
    lerp_tag: |s, handle, start, end, fraction, name| {
        s.models()?.lerp_tag(handle, start, end, fraction, name)
    },
    registered_sound: |s, module, handle| {
        let (bank, source) = s.registered_source(module)?;
        Ok(source.and_then(|source| bank.registered_sound_in(source, handle)))
    },
    ambient: |s, context, sound| {
        let result = s.storage.register_ambient(context, sound);
        if result == Err(CallError::Capacity) {
            s.server.events.reject_publication();
        }
        result
    },
    static_entity: |s, context, entity| {
        let result = s.storage.register_static(context, entity);
        if result == Err(CallError::Capacity) {
            s.server.events.reject_publication();
        }
        result
    },
    sound_id: |s, module, range, index| s.sound_id(module, range, index),
    effect: |s, d, e| s.effect(d, e),
    trace: |s, q, native| s.trace(q, native),
    clip: |s, q, target| s.clip(q, target),
    point_contents: |s, p, rules, pass, native| s.point_contents(p, rules, pass, native),
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
    command_insert: |s, c, t| {
        let context = s.command_context(c);
        s.commands
            .insert_bytes(t, context)
            .map_err(|_| CallError::Text)
    },
    configstring: |s, m, i, t| s.configstring(m, i, t),
    lightstyle: |s, m, i, t| s.lightstyle(m, i, t),
    resource_index: |s, m, r, n| s.resource_register(m, r, n).map(|(ordinal, _)| ordinal),
    resource_register: |s, m, r, n| s.resource_register(m, r, n),
    resource_find: |s, m, r, n| {
        Ok(s.storage
            .resource_lookup(m, r, n, false)?
            .map(|(ordinal, _)| ordinal))
    },
    model_precache: |s, module, path, rules| {
        let text = s
            .cvars
            .read(s.storage.image_search)
            .map_err(|_| CallError::File)?;
        let order =
            qa_render::material::resources::ImageSearchOrder::parse(text.as_str().as_bytes())
                .map_err(|_| CallError::File)?;
        let (enhanced, skeletal) = s
            .storage
            .model
            .replacement_load(s.cvars)
            .map_err(|_| CallError::Cvar)?;
        let loader = s.model_loader.as_mut().ok_or(CallError::File)?;
        loader.set_image_search_order(order);
        loader.set_model_replacement_load(enhanced, skeletal);
        loader.precache(s.vfs, path, rules, s.storage.file_search(module))
    },
    model_bounds: |s, module, path, rules| {
        let search = s.storage.file_search(module);
        s.vfs
            .read_view(search)
            .retain()
            .map_err(|_| CallError::File)?;
        s.models()
            .and_then(|models| models.bounds(path, rules, search))
            .ok_or(CallError::Geometry)
    },
    file_open: |s, m, p| s.file_open(m, p),
    entity_token: |s, m| s.storage.entity_token(m),
    file_length: |s, m, p| s.file_length(m, p),
    file_list: |s, m, q, out| s.file_list(m, q, out),
    file_read: |s, m, h, b| s.file_read(m, h, b),
    file_seek: |s, m, h, from| s.file_seek(m, h, from),
    file_close: |s, m, h| s.file_close(m, h),
};

impl EngineServices<'_> {
    pub fn patch_clip(&self) -> qa_core::primitives::PatchClip {
        self.storage.collision.resolve(self.cvars)
    }
    fn loaded_world(&self, module: ModuleId, name: &[u8]) -> Result<LoadedWorldBinding, CallError> {
        let search = self
            .vfs
            .read_view(self.storage.file_search(module))
            .retain()
            .map_err(|_| CallError::File)?;
        self.models()
            .and_then(|models| models.loaded_world(name, Some(&search)))
            .ok_or(CallError::Geometry)
    }
    fn client_collision_load(&mut self, module: ModuleId, name: &[u8]) -> Result<(), CallError> {
        let geometry = self
            .loaded_world(module, name)?
            .collision
            .ok_or(CallError::Geometry)?;
        let count = self
            .geometry
            .model_count(geometry)
            .ok_or(CallError::Geometry)?;
        // Stock cgame's native collision model namespace is MAX_SUBMODELS.
        if count > 256 {
            return Err(CallError::Capacity);
        }
        let selection = self
            .client_collision
            .as_deref_mut()
            .ok_or(CallError::Unsupported)?;
        if selection.selected != Some(geometry) {
            selection.temporary = Bounds::default();
            selection.capsule = None;
            selection.selected = Some(geometry);
        }
        Ok(())
    }
    fn client_collision_count(&self) -> Result<u32, CallError> {
        let selection = self
            .client_collision
            .as_deref()
            .ok_or(CallError::Unsupported)?;
        selection.selected.map_or(Ok(0), |geometry| {
            self.geometry
                .model_count(geometry)
                .ok_or(CallError::Geometry)
        })
    }
    fn client_collision_inline(&self, index: i32) -> Result<i32, CallError> {
        let count = self.client_collision_count()?;
        (index >= 0 && (index as u32) < count)
            .then_some(index)
            .ok_or(CallError::Geometry)
    }
    fn client_collision_target(
        &self,
        handle: i32,
        trace: bool,
    ) -> Result<(CollisionShape, Bounds), CallError> {
        let selection = self
            .client_collision
            .as_deref()
            .ok_or(CallError::Unsupported)?;
        let count = self.client_collision_count()?;
        // Real inline models take precedence, including native ordinal 255.
        if handle >= 0 && (handle as u32) < count {
            let geometry = selection.selected.ok_or(CallError::Geometry)?;
            // Native traces use the resolved model's bounds for raw254's
            // capsule path; point contents still uses its actual inline brush.
            if trace && handle == 254 {
                return Ok((
                    CollisionShape::Capsule,
                    self.geometry
                        .model_bounds(geometry, 254)
                        .ok_or(CallError::Geometry)?,
                ));
            }
            return Ok((
                CollisionShape::Model {
                    geometry,
                    index: handle as u32,
                },
                Bounds::default(),
            ));
        }
        if handle == 255 {
            return Ok((CollisionShape::Box, selection.temporary));
        }
        // Original Q3 returns254 from TempCapsuleModel but rejects it in the
        // common handle resolver. Admit only an explicitly registered capsule,
        // without aliasing its bounds to the independently retained box255.
        if handle == 254
            && let Some(bounds) = selection.capsule
        {
            return Ok((CollisionShape::Capsule, bounds));
        }
        Err(CallError::Geometry)
    }
    fn models(&self) -> Option<&dyn ModelLookup> {
        self.model_loader
            .as_deref()
            .map(|loader| loader.lookup())
            .or(self.model_lookup)
    }
    /// The trace borrows spatial state while the caller may update the disjoint
    /// client array. Both borrows end before returning to the native module.
    fn world_trace<'a>(
        &'a mut self,
        pass: CollisionOwner,
        native: Option<&'a dyn NativeTraceEntities>,
    ) -> Result<(WorldTrace<'a>, &'a mut [qa_session::clients::Client]), CallError> {
        let (geometry, index) = self.world.ok_or(CallError::Geometry)?;
        let patches = self.patch_clip();
        Ok((
            WorldTrace::new(
                self.geometry,
                geometry,
                index,
                &self.server.entities,
                &self.server.area,
                self.scratch,
                pass,
            )
            .with_patch_clip(patches)
            .with_native_entities(native)
            .with_foreign_entities(self.foreign_trace.map(|read| read.view))
            .with_native_models(Some(self.storage)),
            &mut self.server.clients,
        ))
    }
    pub fn pmove(
        &mut self,
        id: ClientId,
        native: Option<&dyn NativeTraceEntities>,
        options: qa_movement::MoveOptions,
    ) -> Result<qa_movement::MovementResult, CallError> {
        let foreign = self.foreign_trace;
        let result = (|| {
            let player = &self
                .server
                .clients
                .get(id.0 as usize)
                .filter(|client| client.connection.is_some())
                .ok_or(CallError::Entity)?
                .player;
            let tuning = self.storage.movement.resolve(
                self.cvars,
                player.movement_rules,
                player.movement.tuning,
            );
            let (trace, clients) = self.world_trace(CollisionOwner::None, native)?;
            // Admission was checked before the disjoint world/client borrow.
            let client = &mut clients[id.0 as usize];
            let mut trace = trace.with_pass(CollisionOwner::Lifetime(client.entity));
            let base = std::mem::replace(&mut client.player.movement.tuning, tuning);
            let result = qa_movement::pmove(
                client.command,
                &mut client.player,
                &mut trace,
                options,
                qa_movement::no_boundary,
            );
            client.player.movement.tuning = base;
            result.map_err(|_| CallError::Geometry)
        })();
        ForeignTraceRead::complete(foreign, result)
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
    pub fn sound(&mut self, delivery: NativeDelivery, event: SoundEvent) -> Result<u64, CallError> {
        self.server
            .events
            .push_delivered(delivery, FrameEvent::Sound(event))
            .map_err(|_| CallError::Capacity)
    }
    pub fn sound_id(
        &self,
        module: ModuleId,
        sounds: IndexSpan,
        ordinal: i32,
    ) -> Result<Option<qa_core::primitives::SoundId>, CallError> {
        self.storage.sound_id(module, sounds, ordinal)
    }
    /// Prepare configurations accepted before the registrar was lent to services.
    pub fn prepare_sound_bindings(&mut self, module: ModuleId) -> Result<(), CallError> {
        let Some(sounds) = self
            .storage
            .range(module)
            .ok_or(CallError::ConfigString)?
            .sounds
        else {
            return Ok(());
        };
        for ordinal in sounds.range() {
            let index = self.storage.config(module, ordinal)?;
            self.prepare_sound_at(module, index)?;
        }
        Ok(())
    }
    fn prepare_sound_at(&mut self, module: ModuleId, index: usize) -> Result<(), CallError> {
        let fallback_source = if self.sound_loader.is_none() && self.sound_bank.is_some() {
            self.registered_source(module)?.1
        } else {
            None
        };
        let row = self
            .storage
            .ranges
            .iter_mut()
            .find(|row| row.module == module)
            .ok_or(CallError::ConfigString)?;
        let Some(ordinal) = row.sound_ordinal(index) else {
            return Ok(());
        };
        let config = self
            .storage
            .configs
            .config(index)
            .ok_or(CallError::ConfigString)?;
        if row.sound_revisions[ordinal] == Some(config.revision) {
            return Ok(());
        }
        let sound = if let Some(loader) = self.sound_loader.as_deref_mut() {
            row.attach_sounds(loader.bank())?;
            if ordinal == 0 || config.text.is_empty() {
                None
            } else {
                match loader.precache(
                    self.vfs,
                    config.text,
                    row.native_rules,
                    row.file_search
                        .as_ref()
                        .map(qa_content::vfs::FileSearch::view),
                ) {
                    Ok(sound) => sound,
                    Err(error @ (CallError::File | CallError::Capacity | CallError::Text)) => {
                        Self::registration_diagnostic(&mut self.server.events, "sound", error);
                        None
                    }
                    Err(error) => return Err(error),
                }
            }
        } else if let Some(bank) = self.sound_bank {
            row.attach_sounds(bank)?;
            let bindings = row.sound_bindings.as_mut().ok_or(CallError::ConfigString)?;
            bindings
                .bind_name_in(
                    Some(bank),
                    fallback_source,
                    ordinal as u32,
                    config.text,
                    row.native_rules,
                )
                .map_err(|_| CallError::Text)?;
            row.sound_revisions[ordinal] = Some(config.revision);
            return Ok(());
        } else if ordinal == 0 || config.text.is_empty() {
            None
        } else {
            return Ok(());
        };
        row.sound_bindings
            .as_mut()
            .ok_or(CallError::ConfigString)?
            .set(ordinal as u32, sound)
            .map_err(|_| CallError::Text)?;
        row.sound_revisions[ordinal] = Some(config.revision);
        Ok(())
    }
    fn registration_diagnostic(
        events: &mut qa_core::events::EventRing,
        kind: &str,
        error: CallError,
    ) {
        let _ = Self::console_formatted(
            events,
            format_args!("{kind} registration rejected: {error:?}\n"),
        );
    }
    fn registered_source(
        &self,
        module: ModuleId,
    ) -> Result<(&qa_audio::Bank, Option<qa_audio::SoundSource>), CallError> {
        let view = self.vfs.read_view(self.storage.file_search(module));
        if let Some(loader) = self.sound_loader.as_deref() {
            Ok((loader.bank(), loader.source(view)?))
        } else {
            view.retain().map_err(|_| CallError::File)?;
            // A supplied PCM-only bank has no file provenance. Production
            // file-backed banks resolve through SoundRegistrar above.
            Ok((
                self.sound_bank.ok_or(CallError::Unsupported)?,
                Some(qa_audio::SoundSource::DEFAULT),
            ))
        }
    }
    pub fn register_sound(
        &mut self,
        module: ModuleId,
        sample: &[u8],
        source: RuleSetId,
    ) -> Result<i32, CallError> {
        if sample.is_empty() {
            return Err(CallError::Text);
        }
        // Native Q3 RegisterSound rejects MAX_QPATH bytes before attempting a
        // name admission. A failed path still occupies its original handle.
        let result = if sample.len() >= 64 {
            Err(CallError::Text)
        } else if let Some(loader) = self.sound_loader.as_deref_mut() {
            loader.register(self.vfs, sample, source, self.storage.file_search(module))
        } else {
            let (bank, source) = self.registered_source(module)?;
            source
                .and_then(|source| bank.registered_handle_in(source, sample))
                .ok_or(CallError::File)
        };
        self.registration_result("sound", result)
    }
    fn registration_result(
        &mut self,
        kind: &str,
        result: Result<i32, CallError>,
    ) -> Result<i32, CallError> {
        match result {
            Ok(handle) => Ok(handle),
            Err(error @ (CallError::File | CallError::Capacity | CallError::Text)) => {
                Self::registration_diagnostic(&mut self.server.events, kind, error);
                Ok(0)
            }
            Err(error) => Err(error),
        }
    }
    pub fn effect(
        &mut self,
        delivery: NativeDelivery,
        event: EffectEvent,
    ) -> Result<u64, CallError> {
        self.server
            .events
            .push_delivered(delivery, FrameEvent::Effect(event))
            .map_err(|_| CallError::Capacity)
    }
    pub fn trace(
        &mut self,
        query: TraceQuery<'_>,
        native: Option<&dyn NativeTraceEntities>,
    ) -> Result<Trace, CallError> {
        let foreign = self.foreign_trace;
        let result = (|| {
            let (mut trace, _) = self.world_trace(CollisionOwner::None, native)?;
            trace.trace(query).map_err(|_| CallError::Geometry)
        })();
        ForeignTraceRead::complete(foreign, result)
    }
    fn collision_query<'a>(&self, mut query: TraceQuery<'a>) -> TraceQuery<'a> {
        query.rules.patches = self.patch_clip();
        query
    }
    pub fn clip(&mut self, query: TraceQuery<'_>, target: ClipTarget) -> Result<Trace, CallError> {
        let query = self.collision_query(query);
        match target {
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
            } => self.geometry.trace_body(
                query,
                shape,
                &body,
                angles,
                rules,
                BodyTraceMode::Transformed,
                None,
                self.scratch,
            ),
        }
        .map_err(|_| CallError::Geometry)
    }
    pub fn point_contents(
        &mut self,
        point: Vec3,
        rules: EntityTracePolicy,
        pass: CollisionOwner,
        native: Option<&dyn NativeTraceEntities>,
    ) -> Result<Contents, CallError> {
        let foreign = self.foreign_trace;
        let result = (|| {
            let (trace, _) = self.world_trace(pass, native)?;
            trace
                .point_contents(point, rules, &[])
                .map_err(|_| CallError::Geometry)
        })();
        ForeignTraceRead::complete(foreign, result)
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
        if !self.server.free_entity(entity, context.clock) {
            return Err(CallError::Entity);
        }
        Ok(())
    }
    pub fn cvar_register(
        &mut self,
        context: Context,
        name: &str,
        default: Option<&str>,
        flags: u32,
    ) -> Result<View, CallError> {
        let name = self.cvar_name(name, context.source);
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
    pub fn cvar_name<'a>(
        &mut self,
        name: &'a str,
        source: qa_core::primitives::RuleSetId,
    ) -> &'a str {
        let normalized = qa_console::cvars::registration_name(name, source);
        if normalized != name {
            let _ = Self::console_formatted(
                &mut self.server.events,
                format_args!("invalid cvar name string: {name}\n"),
            );
        }
        normalized
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
    fn command_context(&self, mut context: Context) -> Context {
        if let Some(producer) = context.producer {
            context.file_search = self.storage.file_source(producer.module);
        }
        context
    }
    pub fn command(&mut self, context: Context, text: &[u8]) -> Result<(), CallError> {
        let context = self.command_context(context);
        self.commands
            .append_bytes(text, context)
            .map_err(|_| CallError::Text)
    }
    pub fn send_client_command(&mut self, text: &[u8]) -> Result<bool, CallError> {
        self.client_connection
            .as_deref_mut()
            .ok_or(CallError::Entity)?
            .send_command(text, self.server)
    }
    pub fn configstring(
        &mut self,
        module: ModuleId,
        ordinal: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        let index = self.storage.config(module, ordinal)?;
        self.storage.set_configstring_at(module, index, text)?;
        self.prepare_sound_at(module, index)
    }
    fn resource_register(
        &mut self,
        module: ModuleId,
        range: IndexSpan,
        name: &[u8],
    ) -> Result<(u32, bool), CallError> {
        let result = self
            .storage
            .resource_lookup(module, range, name, true)?
            .ok_or(CallError::Capacity)?;
        if name.is_empty() {
            return Ok(result);
        }
        let ordinal = range
            .get(result.0 as usize)
            .ok_or(CallError::ConfigString)?;
        let index = self.storage.config(module, ordinal)?;
        self.prepare_sound_at(module, index)?;
        Ok(result)
    }
    pub fn lightstyle(
        &mut self,
        module: ModuleId,
        ordinal: usize,
        text: &[u8],
    ) -> Result<(), CallError> {
        let rules = self
            .storage
            .range(module)
            .ok_or(CallError::ConfigString)?
            .native_rules;
        let index = ConfigLayout::for_rules(rules)
            .style_index(ordinal)
            .ok_or(CallError::ConfigString)?;
        self.configstring(module, index, text)
    }
    pub fn file_open(&mut self, owner: ModuleId, path: &[u8]) -> Result<(u32, u64), CallError> {
        let reference = self.file_reference(owner, path)?;
        let length = self.vfs.length(&reference).map_err(|_| CallError::File)?;
        let span = self.storage.file_span(owner)?;
        let ordinal = self.storage.files[span.range()]
            .iter()
            .position(Option::is_none)
            .ok_or(CallError::Capacity)?;
        let slot = span.first as usize + ordinal;
        self.storage.files[slot] = Some(OpenFile {
            owner,
            reference,
            cursor: 0,
        });
        Ok((ordinal as u32 + 1, length))
    }
    fn file_reference(&self, owner: ModuleId, path: &[u8]) -> Result<FileRef, CallError> {
        let view = self.storage.file_search(owner);
        self.vfs.open_with(path, view).ok_or(CallError::File)
    }
    pub fn file_length(&self, owner: ModuleId, path: &[u8]) -> Result<u64, CallError> {
        self.storage.range(owner).ok_or(CallError::File)?;
        let reference = self
            .vfs
            .read_view(self.storage.file_search(owner))
            .query(path)
            .ok_or(CallError::File)?;
        self.vfs.length(&reference).map_err(|_| CallError::File)
    }
    pub fn file_list(
        &mut self,
        owner: ModuleId,
        request: qa_content::vfs::FileListRequest<'_>,
        output: &mut [u8],
    ) -> Result<usize, CallError> {
        use qa_content::vfs::{FileListName, FileListRequest};
        if output.is_empty() {
            return Err(CallError::Text);
        }
        self.storage.range(owner).ok_or(CallError::File)?;
        let view = self.storage.file_search(owner).cloned();
        output[0] = 0;
        let mut used = 0;
        let result = match request {
            FileListRequest::Empty => Ok(0),
            FileListRequest::Files(query) => {
                self.storage
                    .file_listing
                    .visit(self.vfs, view.as_ref(), query, |name| {
                        write_list_row(output, &mut used, &[name])
                    })
            }
            FileListRequest::Mods {
                base_game,
                package_suffix,
                maximum,
            } => {
                let reader = &mut self.storage.reader;
                self.storage.file_listing.visit_mods(
                    self.vfs,
                    view.as_ref(),
                    base_game,
                    package_suffix,
                    maximum,
                    |name, description| {
                        let mut text = [0; 48];
                        let length = description
                            .as_ref()
                            .and_then(|file| {
                                self.vfs.read_range_reusing(file, 0, &mut text, reader).ok()
                            })
                            .unwrap_or(0);
                        let description = if length != 0 {
                            qa_core::text::until_nul(&text[..length])
                        } else {
                            name
                        };
                        write_list_row(
                            output,
                            &mut used,
                            &[
                                FileListName {
                                    bytes: name,
                                    trailing_slash: false,
                                },
                                FileListName {
                                    bytes: description,
                                    trailing_slash: false,
                                },
                            ],
                        )
                    },
                )
            }
        };
        result.map_err(|error| match error {
            qa_content::vfs::VfsError::Capacity => CallError::Capacity,
            _ => CallError::File,
        })
    }
    fn file_index(&self, owner: ModuleId, handle: u32) -> Result<usize, CallError> {
        let slot = self.storage.file_index(owner, handle)?;
        self.storage
            .files
            .get(slot)
            .and_then(Option::as_ref)
            .filter(|file| file.owner == owner)
            .ok_or(CallError::File)?;
        Ok(slot)
    }
    pub fn file_read(
        &mut self,
        owner: ModuleId,
        handle: u32,
        buffer: &mut [u8],
    ) -> Result<usize, CallError> {
        let slot = self.file_index(owner, handle)?;
        let file = self
            .storage
            .files
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(CallError::File)?;
        if file.cursor
            >= self
                .vfs
                .length(&file.reference)
                .map_err(|_| CallError::File)?
        {
            return Ok(0);
        }
        let read = self
            .vfs
            .read_range_reusing(
                &file.reference,
                file.cursor,
                buffer,
                &mut self.storage.reader,
            )
            .map_err(|_| CallError::File)?;
        file.cursor += read as u64;
        Ok(read)
    }
    pub fn file_seek(
        &mut self,
        owner: ModuleId,
        handle: u32,
        from: std::io::SeekFrom,
    ) -> Result<bool, CallError> {
        let slot = self.file_index(owner, handle)?;
        let file = self
            .storage
            .files
            .get_mut(slot)
            .and_then(Option::as_mut)
            .ok_or(CallError::File)?;
        let target = match from {
            std::io::SeekFrom::Start(offset) => Some(offset),
            std::io::SeekFrom::Current(offset) => file.cursor.checked_add_signed(offset),
            std::io::SeekFrom::End(offset) => self
                .vfs
                .length(&file.reference)
                .map_err(|_| CallError::File)?
                .checked_add_signed(offset),
        };
        let Some(target) = target else {
            return Ok(false);
        };
        file.cursor = target;
        Ok(true)
    }
    pub fn file_close(&mut self, owner: ModuleId, handle: u32) -> Result<(), CallError> {
        let slot = self.file_index(owner, handle)?;
        self.storage.files[slot] = None;
        Ok(())
    }
}

impl qa_world::collision::NativeModelNames for ServiceStorage {
    fn model_name(&self, module: ModuleId, index: u32) -> Option<(&[u8], u64)> {
        self.configstring(module, index as usize).ok()
    }
}

// Native FS_GetFileList reserves one extra byte per NUL-delimited field and
// leaves bytes after the last written terminator untouched.
fn write_list_row(
    output: &mut [u8],
    used: &mut usize,
    fields: &[qa_content::vfs::FileListName<'_>],
) -> bool {
    let bytes = fields.iter().map(|name| name.len() + 1).sum::<usize>();
    if used
        .checked_add(bytes)
        .and_then(|end| end.checked_add(fields.len()))
        .is_none_or(|end| end >= output.len())
    {
        return false;
    }
    for name in fields {
        let end = *used + name.bytes.len();
        output[*used..end].copy_from_slice(name.bytes);
        *used = end;
        if name.trailing_slash {
            output[*used] = b'/';
            *used += 1;
        }
        output[*used] = 0;
        *used += 1;
    }
    true
}
