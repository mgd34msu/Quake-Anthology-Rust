//! Rerelease Quake II guest import services.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-guest-services.ts`
//! (`RereleaseGuestServices`).
//!
//! Public API2023 imports borrow the shared engine while source RunFrame
//! owns gameplay. The Rust [`RereleaseQ2GuestHost`] is headless: the DLL
//! never runs, so the donor `hostOptions` closures become inherent methods
//! taking the host explicitly, and the engine-facing surface stays on the
//! options. `bindMemory` collapses into [`RereleaseGuestServices::bind_host`]
//! (see the contract): only double binding is rejected.
//!
//! Headless adaptations, each noted at its method:
//! - Slot-to-actor tracking via [`RereleaseGuestServices::admit_entity`]
//!   (donor: host-bound `currentActor`); the synthetic host numbers actors
//!   without engine identities.
//! - Inventory roster seam via
//!   [`RereleaseGuestServices::set_inventory_items`] (donor
//!   `rereleaseInventoryItems` reads the roster from the running module).
//! - Entity-body clip seam via [`RereleaseGuestServices::set_body_trace`]
//!   (donor `traceQ2Box` in `src/world/collision/body.ts`, canonical home:
//!   `qa_world::collision`).
//! - `combat.bind` has no Rust counterpart (`RereleaseCombatBindings`
//!   covers armor/health only); binding decomposes into
//!   [`RereleaseGuestServices::read_body`] and
//!   [`RereleaseGuestServices::write_body`], mirroring the classic lane.

use std::collections::HashMap;

use qa_compat::q2::classic::records::RawEntityView;
use qa_compat::q2::rerelease::combat_binding::RereleaseCombatBindings;
use qa_compat::q2::rerelease::host::{Q2Trace, RereleaseQ2GuestHost, TraceHit as HostTraceHit, TraceSurface};
use qa_compat::q2::rerelease::imports::{read_guest_string, RereleaseCoreServices};
use qa_compat::q2::rerelease::layouts::{edict_layout, field_offset, private_client_layout};
use qa_compat::q2::rerelease::module::{guest_bool, RereleaseImportCall};
use qa_compat::q2::rerelease::navigation::{NavigationServices, RereleaseNavigationImports};
use qa_compat::q2::rerelease::player_state::read_player_state;
use qa_compat::q2::rerelease::public_state::RereleasePublicEdict;
use qa_compat::q2::rerelease::sounds::{RereleaseSoundEvent, SoundAudience};
use qa_compat::q2::rerelease::source_state::{retail_client_fields, RereleaseInventoryItem, RereleaseSourceClient};
use qa_compat::q2::rerelease::spatial::{rerelease_link_bounds, rerelease_network_solid, LinkBody};
use qa_content::contract::ItemId;
use qa_content::q2::foundation::host::{Q2ModelEvent, Q2PresentationEvent};
use qa_content::q2::support::contracts::BodyState as EngineBodyState;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::float_to_wrapped_i32;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_net::msg::{MsgError, MsgWriter};
use qa_net::q2_adapters::{
    Q2EntityState, Q2PlayerView, Q2RereleaseEntityState, Q2RereleaseMovementState, Q2RereleasePlayerState, Q2Vec3,
    Q2Vec4,
};
use qa_world::inventory::InventoryEntry;
use qa_world::spatial::{ActorCollision, CollisionFamily, CollisionRole, CollisionShape, QueryRole};

use super::classic_guest_services::{
    ClassicGuestAudience, ClassicGuestMessage, ModelAppearance, MulticastScope, NativeInputMotion, NativeInputPose,
    NativeInputViewState, PlayerVelocityRead, PlayerVelocityWrite, ResourceNameIndex,
};
use super::rerelease_guest_services_contract::{
    RereleaseEntityInfo, RereleaseGuestClipboard, RereleaseGuestMapServices, RereleaseGuestMessage,
    RereleaseGuestServicesError, RereleaseGuestServicesOptions, RereleaseGuestServicesPort, RereleaseResult,
};
use super::source_hosts::SceneTextureInfo;

/// Resource kind for configstring ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// Models.
    Model,
    /// Sounds.
    Sound,
    /// Images.
    Image,
}

impl ResourceKind {
    fn base(self) -> i32 {
        match self {
            Self::Model => 62,
            Self::Sound => 8254,
            Self::Image => 10302,
        }
    }

    fn count(self) -> i32 {
        match self {
            Self::Model => 8192,
            Self::Sound => 2048,
            Self::Image => 512,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Sound => "sound",
            Self::Image => "image",
        }
    }
}

/// Link metadata cached per linked actor.
#[derive(Debug, Clone, PartialEq)]
struct LinkMetadata {
    clusters: Option<Vec<i32>>,
    first_cluster: i32,
    headnode: i32,
    areas: (i32, i32),
}

/// Link report for an entity (donor `linkMetadata` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseLinkReport {
    /// First area.
    pub area: i32,
    /// Second area.
    pub area2: i32,
    /// Network solid.
    pub network_solid: u32,
}

/// Actor description for the entity-body clip seam.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseBodyTraceActor {
    /// Hit actor.
    pub actor: ActorId,
    /// Body state.
    pub state: EngineBodyState,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
    /// Link count.
    pub link_count: i32,
    /// Collision record.
    pub collision: ActorCollision,
}

/// Seam for donor `traceQ2Box` via `traceActorBody` (donor
/// `src/world/collision/body.ts`, canonical home: `qa_world::collision`);
/// unify post-merge.
pub type RereleaseBodyTraceFn = Box<
    dyn FnMut(&qa_bots::scene::TraceQuery, &RereleaseBodyTraceActor) -> RereleaseResult<qa_bots::scene::TraceResult>,
>;

/// Source inventory binding for a slot (donor `InventoryStateBinding`
/// object shape, with the host passed explicitly per call).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseInventoryBinding {
    slot: u32,
    actor: ActorId,
}

impl RereleaseInventoryBinding {
    /// Bound slot.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// Bound actor.
    #[must_use]
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }

    /// Read the declared roster as shared inventory entries.
    pub fn read(
        &self,
        services: &mut RereleaseGuestServices,
        host: &mut RereleaseQ2GuestHost,
    ) -> RereleaseResult<Vec<InventoryEntry>> {
        services.read_source_inventory(host, self.slot, &self.actor)
    }

    /// Write one inventory entry.
    pub fn write(
        &self,
        services: &mut RereleaseGuestServices,
        host: &mut RereleaseQ2GuestHost,
        entry: &InventoryEntry,
    ) -> RereleaseResult<()> {
        services.write_source_inventory(host, self.slot, &self.actor, entry)
    }

    /// Whether an item has explicit mutable capacity.
    #[must_use]
    pub fn mutable_capacity(&self, services: &RereleaseGuestServices, item: &ItemId) -> bool {
        services.source_inventory_mutable_capacity(item)
    }
}

/// Navigation services adapter over the boxed options services.
struct OptionsNavigation<'a> {
    services: &'a mut dyn NavigationServices,
}

impl NavigationServices for OptionsNavigation<'_> {
    fn runtime(&self) -> Option<&qa_compat::q2::rerelease::navigation::NavRuntime> {
        self.services.runtime()
    }

    fn move_to_point(&mut self, actor: u32, point: Vec3, tolerance: f32) -> u8 {
        self.services.move_to_point(actor, point, tolerance)
    }

    fn follow_actor(&mut self, actor: u32, target: u32) -> u8 {
        self.services.follow_actor(actor, target)
    }
}

fn msg_write(result: Result<(), MsgError>) -> RereleaseResult<()> {
    result.map_err(|error| RereleaseGuestServicesError::invalid(format!("API2023 message write failed: {error}")))
}

fn write_byte_wrapped(writer: &mut MsgWriter, value: f64) -> RereleaseResult<()> {
    msg_write(writer.write_byte((float_to_wrapped_i32(value) & 0xff) as u8))
}

fn write_short_direct(writer: &mut MsgWriter, value: i32) -> RereleaseResult<()> {
    msg_write(writer.write_short(value as i16))
}

fn write_string_direct(writer: &mut MsgWriter, value: &str) -> RereleaseResult<()> {
    msg_write(writer.write_string(value))
}

fn rr_number(values: &[GuestCallValue], index: usize) -> RereleaseResult<f64> {
    match values.get(index) {
        Some(GuestCallValue::Int32(value)) => Ok(f64::from(*value)),
        Some(GuestCallValue::Uint32(value)) => Ok(f64::from(*value)),
        Some(GuestCallValue::Int64(value)) => Ok(*value as f64),
        Some(GuestCallValue::Uint64(value)) => Ok(*value as f64),
        Some(GuestCallValue::Float32(value)) => Ok(f64::from(*value)),
        Some(GuestCallValue::Float64(value)) => Ok(*value),
        _ => Err(RereleaseGuestServicesError::invalid(format!(
            "API2023 argument {index} must be numeric"
        ))),
    }
}

fn rr_pointer(values: &[GuestCallValue], index: usize) -> RereleaseResult<Option<GuestAddress>> {
    match values.get(index) {
        Some(GuestCallValue::Pointer(value)) => Ok(*value),
        _ => Err(RereleaseGuestServicesError::invalid(format!(
            "API2023 argument {index} must be a pointer"
        ))),
    }
}

fn rr_required_pointer(values: &[GuestCallValue], index: usize) -> RereleaseResult<GuestAddress> {
    rr_pointer(values, index)?
        .ok_or_else(|| RereleaseGuestServicesError::invalid(format!("API2023 argument {index} cannot be null")))
}

fn q2_vec(value: Vec3) -> Q2Vec3 {
    Q2Vec3 {
        x: f64::from(value.x),
        y: f64::from(value.y),
        z: f64::from(value.z),
    }
}

fn host_mapped(error: impl ToString) -> RereleaseGuestServicesError {
    RereleaseGuestServicesError::Host(error.to_string())
}

/// API2023 guest import services.
pub struct RereleaseGuestServices {
    options: RereleaseGuestServicesOptions,
    strings: HashMap<i32, String>,
    resource_names: [ResourceNameIndex; 3],
    weapon_models: HashMap<i32, String>,
    weapon_model_list: Vec<String>,
    messages: Vec<RereleaseGuestMessage>,
    links: HashMap<ActorId, LinkMetadata>,
    actors: HashMap<u32, ActorId>,
    buffer: MsgWriter,
    bound: bool,
    combat: Option<RereleaseCombatBindings>,
    inventory_items: Option<Vec<RereleaseInventoryItem>>,
    body_trace: Option<RereleaseBodyTraceFn>,
    write_player_velocity: Option<PlayerVelocityWrite>,
    read_player_velocity: Option<PlayerVelocityRead>,
    input_motion: HashMap<ActorId, Box<dyn NativeInputMotion>>,
    loading: bool,
    frame: u32,
}

impl RereleaseGuestServices {
    /// Bound services options (donor `services.options`).
    #[must_use]
    pub fn options(&self) -> &RereleaseGuestServicesOptions {
        &self.options
    }

    /// Build the services over validated options.
    pub fn new(options: RereleaseGuestServicesOptions) -> RereleaseResult<Self> {
        if options.base.max_clients < 1 || options.base.max_clients > 256 {
            return Err(RereleaseGuestServicesError::invalid("Invalid API2023 maxclients"));
        }
        if options.frame_milliseconds <= 0 || 1000 % options.frame_milliseconds != 0 {
            return Err(RereleaseGuestServicesError::invalid("Invalid API2023 frame duration"));
        }
        Self::validate_map_options(&options)?;
        let mut services = Self {
            options,
            strings: HashMap::new(),
            resource_names: [
                ResourceNameIndex::new(8192, &[255]),
                ResourceNameIndex::new(2048, &[]),
                ResourceNameIndex::new(512, &[]),
            ],
            weapon_models: HashMap::new(),
            weapon_model_list: vec!["weapon.md2".to_string()],
            messages: Vec::new(),
            links: HashMap::new(),
            actors: HashMap::new(),
            buffer: MsgWriter::new(65536, false),
            bound: false,
            combat: None,
            inventory_items: None,
            body_trace: None,
            write_player_velocity: None,
            read_player_velocity: None,
            input_motion: HashMap::new(),
            loading: true,
            frame: 0,
        };
        services.seed_map();
        let max_clients = services.options.base.max_clients;
        services
            .options
            .base
            .cvars
            .set("maxclients", &max_clients.to_string(), true)
            .map_err(|error| {
                RereleaseGuestServicesError::invalid(format!("API2023 maxclients cvar failed: {error}"))
            })?;
        Ok(services)
    }

    fn validate_map_options(options: &RereleaseGuestServicesOptions) -> RereleaseResult<()> {
        if options.base.scene.model_count() >= 8191 {
            return Err(RereleaseGuestServicesError::invalid(
                "API2023 map exceeds model capacity",
            ));
        }
        Ok(())
    }

    /// Current server frame.
    #[must_use]
    pub fn server_frame(&self) -> u32 {
        self.frame
    }

    /// Run `run` with an input-motion projection installed for an actor.
    pub fn with_input_movement<T>(
        &mut self,
        actor: &ActorId,
        projection: Box<dyn NativeInputMotion>,
        run: impl FnOnce() -> T,
    ) -> T {
        let previous = self.input_motion.insert(actor.clone(), projection);
        let result = run();
        match previous {
            None => {
                self.input_motion.remove(actor);
            }
            Some(projection) => {
                self.input_motion.insert(actor.clone(), projection);
            }
        }
        result
    }

    /// Install the player-velocity writer/reader pair.
    pub fn set_player_velocity_writer(&mut self, write: PlayerVelocityWrite, read: PlayerVelocityRead) {
        self.write_player_velocity = Some(write);
        self.read_player_velocity = Some(read);
    }

    /// Install the source inventory roster (donor `rereleaseInventoryItems`
    /// reads it from the running module; the headless port receives it
    /// out-of-band).
    pub fn set_inventory_items(&mut self, items: Vec<RereleaseInventoryItem>) {
        self.inventory_items = Some(items);
    }

    /// Install the entity-body clip hook (donor `traceQ2Box` seam).
    pub fn set_body_trace(&mut self, trace: RereleaseBodyTraceFn) {
        self.body_trace = Some(trace);
    }

    /// Shared console variables.
    #[must_use]
    pub fn cvars(&self) -> &CvarRegistry {
        &self.options.base.cvars
    }

    /// Shared console variables, mutably.
    pub fn cvars_mut(&mut self) -> &mut CvarRegistry {
        &mut self.options.base.cvars
    }

    /// Configured client capacity.
    #[must_use]
    pub fn max_clients(&self) -> u32 {
        self.options.base.max_clients
    }

    fn resource_index_table(&mut self, kind: ResourceKind) -> &mut ResourceNameIndex {
        let index = match kind {
            ResourceKind::Model => 0,
            ResourceKind::Sound => 1,
            ResourceKind::Image => 2,
        };
        &mut self.resource_names[index]
    }

    fn seed_map(&mut self) {
        for table in &mut self.resource_names {
            table.clear();
        }
        self.weapon_models.clear();
        self.weapon_model_list = vec!["weapon.md2".to_string()];
        let map_path = self.options.base.map_path.clone();
        self.store_configstring(63, &map_path);
        let models = self.options.base.scene.model_count();
        for model in 1..models {
            let index = 63 + model as i32 + i32::from(model >= 254);
            self.store_configstring(index, &format!("*{model}"));
        }
    }

    fn validate_configstring(index: i32, value: &str) -> RereleaseResult<()> {
        if !(0..12448).contains(&index) || value.contains('\0') {
            return Err(RereleaseGuestServicesError::invalid("Invalid API2023 configstring"));
        }
        Ok(())
    }

    fn store_configstring(&mut self, index: i32, value: &str) {
        self.strings.insert(index, value.to_string());
        for kind in [ResourceKind::Model, ResourceKind::Sound, ResourceKind::Image] {
            let slot = index - kind.base();
            if slot > 0 && slot < kind.count() {
                self.resource_index_table(kind).set(slot, value);
                break;
            }
        }
    }

    fn is_weapon_model(index: i32, value: &str) -> bool {
        let base = ResourceKind::Model.base();
        index > base && index < base + ResourceKind::Model.count() && value.starts_with('#')
    }

    fn rebuild_weapon_models(&mut self) {
        let mut entries: Vec<(i32, String)> = self
            .weapon_models
            .iter()
            .map(|(index, name)| (*index, name.clone()))
            .collect();
        entries.sort_by_key(|(index, _)| *index);
        self.weapon_model_list = vec!["weapon.md2".to_string()];
        self.weapon_model_list.extend(entries.into_iter().map(|(_, name)| name));
    }

    /// Resolve a resource path by kind and index.
    #[must_use]
    pub fn resource(&self, kind: ResourceKind, index: i32) -> String {
        self.strings.get(&(kind.base() + index)).cloned().unwrap_or_default()
    }

    /// Resolve or allocate a resource index by kind and name.
    pub fn resource_index(&mut self, kind: ResourceKind, name: &str) -> RereleaseResult<i32> {
        let index = self
            .resource_index_table(kind)
            .find(name)
            .ok_or_else(|| RereleaseGuestServicesError::invalid(format!("API2023 {} index overflow", kind.as_str())))?;
        if index != 0 && self.resource(kind, index) != name {
            let slot = kind.base() + index;
            RereleaseGuestServicesPort::set_configstring(self, slot, name)?;
        }
        Ok(index)
    }

    fn inline_model(&self, index: i32) -> RereleaseResult<i32> {
        let path = self.resource(ResourceKind::Model, index);
        if index == 1 && path == self.options.base.map_path {
            return Ok(0);
        }
        let model = path.strip_prefix('*').unwrap_or("").parse::<i32>().unwrap_or(-1);
        if !path.starts_with('*') || model < 0 || model >= self.options.base.scene.model_count() as i32 {
            return Err(RereleaseGuestServicesError::invalid(format!(
                "Invalid API2023 inline model {path}"
            )));
        }
        Ok(model)
    }

    /// Whether two areas connect (donor `hostOptions.spatial.areasConnected`).
    #[must_use]
    pub fn areas_connected(&self, first: i32, second: i32) -> bool {
        self.options.base.scene.areas_connected(first, second)
    }

    /// Whether two points see each other (donor `hostOptions.spatial.visibility`).
    #[must_use]
    pub fn visibility(&self, kind: qa_bots::scene::VisibilityKind, first: Vec3, second: Vec3, portals: bool) -> bool {
        let scene = &self.options.base.scene;
        let a = scene.point_leaf(first);
        let b = scene.point_leaf(second);
        scene.cluster_visible(scene.leaf_cluster(a), scene.leaf_cluster(b), kind)
            && (!portals || scene.areas_connected(scene.leaf_area(a), scene.leaf_area(b)))
    }

    /// Texture record id for a surface (donor `hostOptions.spatial.surfaceId`).
    #[must_use]
    pub fn surface_id(&self, surface: &SceneTextureInfo) -> i32 {
        match self.options.base.scene.q2_texture_info() {
            None => 0,
            Some(info) => info
                .iter()
                .position(|value| {
                    value.name == surface.name && value.flags == surface.flags && value.value == surface.value
                })
                .map_or(0, |index| index as i32),
        }
    }

    /// Actors touching bounds (donor `hostOptions.spatial.boxEdicts`).
    pub fn box_edicts(&self, min: Vec3, max: Vec3, area: i32) -> RereleaseResult<Vec<ActorId>> {
        let role = match area {
            1 => QueryRole::Solid,
            2 => QueryRole::Trigger,
            _ => return Err(RereleaseGuestServicesError::invalid("Invalid API2023 BoxEdicts area")),
        };
        Ok(self
            .options
            .base
            .scene
            .query_actors(&Bounds { min, max }, role)
            .into_iter()
            .map(|value| value.body.actor)
            .collect())
    }

    fn view<'m>(host: &'m mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<RereleasePublicEdict<'m>> {
        let address = host.record_at(slot).map_err(host_mapped)?;
        RereleasePublicEdict::new(&mut host.memory, address).map_err(host_mapped)
    }

    /// Read the body state for a slot (donor `bindEntity` read half).
    pub fn read_body(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<EngineBodyState> {
        let actor = self.actors.get(&slot).cloned();
        let pending = match actor.as_ref() {
            None => None,
            Some(actor) => match self.read_player_velocity.as_mut() {
                None => None,
                Some(read) => read(actor),
            },
        };
        let motion = actor
            .as_ref()
            .and_then(|actor| self.input_motion.get_mut(actor).map(|motion| motion.read()));
        let mut view = Self::view(host, slot)?;
        let velocity = if let Some(motion) = motion {
            Some(motion.velocity)
        } else if let Some(pending) = pending {
            Some(pending)
        } else if slot > 0
            && slot <= self.options.base.max_clients
            && view.pointer("client").map_err(host_mapped)?.is_some()
        {
            Some(view.player_velocity().map_err(host_mapped)?)
        } else {
            None
        };
        let origin = motion.map(|motion| motion.origin);
        let body = view.body(velocity, origin).map_err(host_mapped)?;
        Ok(EngineBodyState {
            origin: body.origin,
            angles: body.angles,
            velocity: body.velocity,
            bounds: Bounds {
                min: body.min,
                max: body.max,
            },
            ground: None,
        })
    }

    /// Write a body state through to the guest record (donor `bindEntity` write half).
    pub fn write_body(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &OwnedActor,
        state: &EngineBodyState,
    ) -> RereleaseResult<()> {
        let velocity = self.read_body(host, slot)?.velocity;
        if self.input_motion.contains_key(actor.id()) {
            let motion = self.input_motion.get_mut(actor.id()).expect("motion present");
            motion.write(&NativeInputPose {
                origin: state.origin,
                velocity: state.velocity,
            });
            let mut view = Self::view(host, slot)?;
            view.set_vector("s.angles", state.angles).map_err(host_mapped)?;
            view.set_vector("mins", state.bounds.min).map_err(host_mapped)?;
            view.set_vector("maxs", state.bounds.max).map_err(host_mapped)?;
            return Ok(());
        }
        if state.velocity != velocity {
            let client = Self::view(host, slot)?.pointer("client").map_err(host_mapped)?;
            if slot < 1
                || slot > self.options.base.max_clients
                || client.is_none()
                || self.write_player_velocity.is_none()
            {
                return Err(RereleaseGuestServicesError::invalid(
                    "API2023 velocity writes require a source semantic binding",
                ));
            }
            if let Some(write) = self.write_player_velocity.as_mut() {
                write(actor, state.velocity);
            }
        }
        let mut view = Self::view(host, slot)?;
        view.set_vector("s.origin", state.origin).map_err(host_mapped)?;
        view.set_vector("s.angles", state.angles).map_err(host_mapped)?;
        view.set_vector("mins", state.bounds.min).map_err(host_mapped)?;
        view.set_vector("maxs", state.bounds.max).map_err(host_mapped)?;
        Ok(())
    }

    /// Admit a bound entity (donor `hostOptions.semantics.bound` chain).
    ///
    /// The donor also invokes the semantic override's own `bound` hook; the
    /// foundation [`RereleaseSemanticBindings`](super::types::RereleaseSemanticBindings)
    /// mirror carries that hook at module granularity, so only admission runs here.
    pub fn admit_entity(&mut self, record: &RawEntityView, actor: &OwnedActor) {
        self.actors.insert(record.slot, actor.id().clone());
        (self.options.base.admit)(record, actor);
    }

    /// Foreign address of an actor (donor `hostOptions.semantics.foreignAddress`).
    pub fn foreign_address(&self, actor: &ActorId) -> RereleaseResult<GuestAddress> {
        Err(RereleaseGuestServicesError::invalid(format!(
            "API2023 module requires a semantic projection extension for foreign actor {}",
            actor.slot()
        )))
    }

    fn world_link(&self, bounds: &Bounds) -> LinkMetadata {
        let scene = &self.options.base.scene;
        let query = scene.box_leaves(bounds, 128);
        let mut clusters: Vec<i32> = Vec::new();
        let (mut first, mut second) = (0, 0);
        for leaf in &query.leaves {
            let area = scene.leaf_area(*leaf);
            let cluster = scene.leaf_cluster(*leaf);
            if area != 0 {
                if first != 0 && first != area {
                    second = area;
                } else {
                    first = area;
                }
            }
            if cluster >= 0 && !clusters.contains(&cluster) {
                clusters.push(cluster);
            }
        }
        LinkMetadata {
            clusters: if query.overflow || query.leaves.len() >= 128 || clusters.len() > 16 {
                None
            } else {
                Some(clusters.clone())
            },
            first_cluster: clusters.first().copied().unwrap_or(0),
            headnode: query.topnode.unwrap_or(0),
            areas: (first, second),
        }
    }

    /// Register collision for a linked entity (donor `hostOptions.spatial.prepareLink`).
    pub fn prepare_link(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        actor: &OwnedActor,
        slot: u32,
    ) -> RereleaseResult<()> {
        let (solid, flags, owner_address, model_index) = {
            let mut view = Self::view(host, slot)?;
            (
                view.byte("solid").map_err(host_mapped)?,
                view.uint("svflags").map_err(host_mapped)?,
                view.pointer("owner").map_err(host_mapped)?,
                view.int("s.modelindex").map_err(host_mapped)?,
            )
        };
        let owner = match owner_address {
            None => None,
            Some(address) => {
                let owner_slot = host.slot_for_address(address).map_err(host_mapped)?;
                self.actors.get(&owner_slot).cloned()
            }
        };
        let shape = if solid == 3 {
            CollisionShape::Model(self.inline_model(model_index)? as u32)
        } else {
            CollisionShape::Box
        };
        let contents = if solid == 0 {
            0
        } else if solid == 3 {
            1
        } else if flags & 2 != 0 {
            0x4000000
        } else if flags & 8 != 0 {
            0x40000000
        } else if flags & 128 != 0 {
            0x80000000u32 as i32
        } else {
            0x2000000
        };
        (self.options.base.collision)(
            actor,
            &ActorCollision {
                family: CollisionFamily::Q2,
                shape,
                contents,
                owner,
                role: if solid == 1 {
                    CollisionRole::Trigger
                } else {
                    CollisionRole::Solid
                },
                monster: flags & 4 != 0,
                dead_monster: flags & 2 != 0,
                q1_corpse: false,
                q3_owner: None,
            },
        );
        Ok(())
    }

    /// Link metadata for an entity (donor `hostOptions.spatial.linkMetadata`).
    pub fn link_metadata(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        actor: &OwnedActor,
        slot: u32,
    ) -> RereleaseResult<RereleaseLinkReport> {
        let mut view = Self::view(host, slot)?;
        let solid = view.byte("solid").map_err(host_mapped)?;
        let flags = view.uint("svflags").map_err(host_mapped)?;
        let state = self.read_body(host, slot)?;
        let link = self.world_link(&rerelease_link_bounds(
            &LinkBody {
                origin: state.origin,
                angles: state.angles,
                bounds: state.bounds,
            },
            i32::from(solid),
        ));
        let report = RereleaseLinkReport {
            area: link.areas.0,
            area2: link.areas.1,
            network_solid: rerelease_network_solid(&state.bounds, i32::from(solid), flags as i32),
        };
        self.links.insert(actor.id().clone(), link);
        Ok(report)
    }

    fn write_entity_number(host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<()> {
        let base = host.record_at(slot).map_err(host_mapped)?;
        let edict = edict_layout();
        let offset = field_offset(&edict, "s.number").map_err(host_mapped)?;
        let at = host.memory.offset(base, offset as i64).map_err(host_mapped)?;
        host.memory.write_u32(at, slot).map_err(host_mapped)?;
        Ok(())
    }

    fn appearance(&self, model_indexes: &[i32; 4], skin: i32) -> ModelAppearance {
        if model_indexes.iter().all(|index| *index != 255) {
            return ModelAppearance {
                path: self.resource(ResourceKind::Model, model_indexes[0]),
                skin,
                skin_path: None,
                attached_models: model_indexes[1..]
                    .iter()
                    .map(|index| self.resource(ResourceKind::Model, *index))
                    .collect(),
            };
        }
        let value = self
            .strings
            .get(&(11582 + (skin & 255)))
            .cloned()
            .unwrap_or_else(|| "player\\male/grunt".to_string());
        let appearance = value.find('\\').map_or(value.as_str(), |index| &value[index + 1..]);
        let (model, skin_name) = match appearance.find('/') {
            None => ("male".to_string(), "grunt".to_string()),
            Some(slash) => (appearance[..slash].to_string(), appearance[slash + 1..].to_string()),
        };
        let weapon = self
            .weapon_model_list
            .get(((skin as u32 >> 8) & 255) as usize)
            .cloned()
            .unwrap_or_else(|| "weapon.md2".to_string());
        let head = model_indexes[0] == 255;
        ModelAppearance {
            path: if head {
                format!("players/{model}/tris.md2")
            } else {
                self.resource(ResourceKind::Model, model_indexes[0])
            },
            skin: if head { 0 } else { skin },
            skin_path: if head {
                Some(format!("players/{model}/{skin_name}.pcx"))
            } else {
                None
            },
            attached_models: model_indexes[1..]
                .iter()
                .map(|index| {
                    if *index == 255 {
                        format!("players/{model}/{weapon}")
                    } else {
                        self.resource(ResourceKind::Model, *index)
                    }
                })
                .collect(),
        }
    }

    fn accepts_client(&self, host: &RereleaseQ2GuestHost, slot: u32) -> bool {
        if let Some(probe) = self.options.base.accepts_client.as_ref() {
            return probe(slot);
        }
        slot >= 1 && slot <= self.options.base.max_clients && slot < host.entity_count && host.is_client_reserved(slot)
    }

    fn enqueue(&mut self, audience: ClassicGuestAudience, reliable: bool, bytes: Vec<u8>, dupe_key: u32) {
        self.messages.push(RereleaseGuestMessage {
            base: ClassicGuestMessage {
                audience,
                reliable,
                bytes,
            },
            dupe_key,
        });
    }

    fn print(
        &mut self,
        host: &RereleaseQ2GuestHost,
        slot: Option<u32>,
        level: i32,
        text: &str,
        broadcast: bool,
        center: bool,
    ) -> RereleaseResult<()> {
        if !broadcast && slot.is_none() {
            (self.options.base.print)(text);
            return Ok(());
        }
        if let Some(slot) = slot {
            if !self.accepts_client(host, slot) {
                return Ok(());
            }
        }
        let mut writer = MsgWriter::new(65536, false);
        write_byte_wrapped(&mut writer, f64::from(if center { 15 } else { 10 }))?;
        if !center {
            write_byte_wrapped(&mut writer, f64::from(level))?;
        }
        write_string_direct(&mut writer, text)?;
        let bytes = writer.bytes().to_vec();
        if broadcast {
            self.enqueue(
                ClassicGuestAudience::Multicast {
                    origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scope: MulticastScope::All,
                },
                true,
                bytes,
                0,
            );
            (self.options.base.print)(text);
        } else {
            self.enqueue(
                ClassicGuestAudience::Unicast {
                    slot: slot.unwrap_or(0),
                },
                true,
                bytes,
                0,
            );
        }
        Ok(())
    }

    /// Voice a positioned sound event (donor `hostOptions.sound`).
    pub fn sound(&mut self, host: &mut RereleaseQ2GuestHost, event: &RereleaseSoundEvent) -> RereleaseResult<()> {
        if event.volume < 0.0
            || event.volume > 1.0
            || event.attenuation < 0.0
            || event.attenuation > 4.0
            || event.time_offset < 0.0
            || event.time_offset > 0.255
            || event.sound_index < 0
            || event.sound_index >= 2048
        {
            return Err(RereleaseGuestServicesError::invalid("Invalid API2023 sound arguments"));
        }
        let state = match event.entity_slot {
            None => None,
            Some(slot) => Some(self.read_body(host, slot)?),
        };
        if state.is_none() && event.origin.is_none() {
            return Err(RereleaseGuestServicesError::invalid(
                "API2023 sound has neither entity nor origin",
            ));
        }
        let solid = match event.entity_slot {
            None => None,
            Some(slot) => Some(Self::view(host, slot)?.byte("solid").map_err(host_mapped)?),
        };
        let origin = match event.origin {
            Some(origin) => origin,
            None => {
                let state = state.as_ref().expect("sound state present");
                if solid == Some(3) {
                    Vec3 {
                        x: state.origin.x + (state.bounds.min.x + state.bounds.max.x) * 0.5,
                        y: state.origin.y + (state.bounds.min.y + state.bounds.max.y) * 0.5,
                        z: state.origin.z + (state.bounds.min.z + state.bounds.max.z) * 0.5,
                    }
                } else {
                    state.origin
                }
            }
        };
        let flags = (if event.entity_slot.is_none() { 0 } else { 8 }
            | if event.volume != 1.0 { 1 } else { 0 }
            | if event.attenuation != 1.0 { 2 } else { 0 }
            | 4
            | if event.time_offset != 0.0 { 16 } else { 0 }
            | if event.sound_index > 255 { 32 } else { 0 }) as u8;
        let mut writer = MsgWriter::new(65536, false);
        write_byte_wrapped(&mut writer, 9.0)?;
        write_byte_wrapped(&mut writer, f64::from(flags))?;
        if flags & 32 != 0 {
            write_short_direct(&mut writer, event.sound_index)?;
        } else {
            write_byte_wrapped(&mut writer, f64::from(event.sound_index))?;
        }
        if flags & 1 != 0 {
            write_byte_wrapped(&mut writer, f64::from(event.volume * 255.0))?;
        }
        if flags & 2 != 0 {
            write_byte_wrapped(&mut writer, f64::from(event.attenuation * 64.0))?;
        }
        if flags & 16 != 0 {
            write_byte_wrapped(&mut writer, f64::from(event.time_offset * 1000.0))?;
        }
        if let Some(slot) = event.entity_slot {
            write_short_direct(&mut writer, ((slot << 3) | (event.channel as u32 & 7)) as i32)?;
        }
        msg_write(writer.write_float(origin.x))?;
        msg_write(writer.write_float(origin.y))?;
        msg_write(writer.write_float(origin.z))?;
        let bytes = writer.bytes().to_vec();
        let (audience, dupe_key) = match event.audience {
            SoundAudience::Client { client_slot, dupe_key } => {
                (ClassicGuestAudience::Unicast { slot: client_slot }, dupe_key)
            }
            SoundAudience::World => (
                ClassicGuestAudience::Multicast {
                    origin,
                    scope: if event.channel & 8 != 0 || event.attenuation == 0.0 {
                        MulticastScope::All
                    } else {
                        MulticastScope::Phs
                    },
                },
                0,
            ),
        };
        self.enqueue(audience, event.channel & 16 != 0, bytes, dupe_key);
        Ok(())
    }

    /// Whether a slot ignores its enemies.
    #[must_use]
    pub fn notarget(&self, slot: u32) -> Option<bool> {
        self.combat.as_ref().and_then(|combat| combat.notarget(slot))
    }

    fn check_client_actor(&mut self, slot: u32, actor: &ActorId, what: &str) -> RereleaseResult<()> {
        let current = self.actors.get(&slot).cloned();
        if slot < 1
            || slot > self.options.base.max_clients
            || !self.options.base.engine.actors().is_live(actor)
            || current.as_ref() != Some(actor)
        {
            return Err(RereleaseGuestServicesError::invalid(format!(
                "API2023 {what} requires the current client actor"
            )));
        }
        Ok(())
    }

    /// Whether a client body is grounded.
    pub fn player_grounded(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &ActorId,
    ) -> RereleaseResult<bool> {
        self.check_client_actor(slot, actor, "movement state")?;
        if let Some(motion) = self.input_motion.get_mut(actor) {
            return Ok(motion.grounded());
        }
        let mut view = Self::view(host, slot)?;
        Ok(view.player_movement_flags().map_err(host_mapped)? & 4 != 0)
    }

    /// Client view projection.
    pub fn player_view(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &ActorId,
    ) -> RereleaseResult<NativeInputViewState> {
        self.check_client_actor(slot, actor, "player view")?;
        if let Some(motion) = self.input_motion.get_mut(actor) {
            return Ok(motion.view());
        }
        let mut view = Self::view(host, slot)?;
        let state = view.player_view().map_err(host_mapped)?;
        Ok(NativeInputViewState {
            view_offset: Vec3 {
                x: state.view_offset.x,
                y: state.view_offset.y,
                z: state.view_offset.z + f32::from(state.view_height),
            },
            crouched: state.movement_flags & 1 != 0,
        })
    }

    /// Write the view roll into the source client record.
    pub fn set_player_view_roll(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &ActorId,
        roll: f64,
    ) -> RereleaseResult<()> {
        self.check_client_actor(slot, actor, "source view")?;
        let client = Self::view(host, slot)?.pointer("client").map_err(host_mapped)?;
        let client = client.ok_or_else(|| RereleaseGuestServicesError::invalid("API2023 source view has no client"))?;
        let layout = private_client_layout();
        let offset = field_offset(&layout, "v_angle")
            .map_err(|_| RereleaseGuestServicesError::invalid("API2023 source view has no client"))?;
        let at = host.memory.offset(client, offset as i64 + 8).map_err(host_mapped)?;
        host.memory.write_f32(at, roll as f32).map_err(host_mapped)?;
        Ok(())
    }

    /// Whether a source inventory profile is known.
    #[must_use]
    pub fn has_source_inventory(&self) -> bool {
        self.inventory_items.is_some()
    }

    /// Bind the source inventory for a slot.
    pub fn source_inventory(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
    ) -> RereleaseResult<Option<RereleaseInventoryBinding>> {
        if self.inventory_items.is_none() {
            return Ok(None);
        }
        host.record_at(slot).map_err(host_mapped)?;
        let actor = self
            .actors
            .get(&slot)
            .cloned()
            .ok_or_else(|| RereleaseGuestServicesError::invalid("Native inventory requires a live source actor"))?;
        Ok(Some(RereleaseInventoryBinding { slot, actor }))
    }

    fn source_client<'m>(
        &self,
        host: &'m mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &ActorId,
    ) -> RereleaseResult<RereleaseSourceClient<'m>> {
        let current = self.actors.get(&slot).cloned();
        if current.as_ref() != Some(actor) {
            return Err(RereleaseGuestServicesError::invalid(
                "Native inventory owner was released",
            ));
        }
        let address = Self::view(host, slot)?.pointer("client").map_err(host_mapped)?;
        let address =
            address.ok_or_else(|| RereleaseGuestServicesError::invalid("Native inventory requires a source client"))?;
        RereleaseSourceClient::new(&mut host.memory, address, retail_client_fields(), 84, 12).map_err(host_mapped)
    }

    fn live_inventory_actor(&mut self, slot: u32, actor: &ActorId) -> RereleaseResult<()> {
        if !self.options.base.engine.actors().is_live(actor) || self.actors.get(&slot) != Some(actor) {
            return Err(RereleaseGuestServicesError::invalid(
                "Native inventory owner was released",
            ));
        }
        Ok(())
    }

    fn read_source_inventory(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &ActorId,
    ) -> RereleaseResult<Vec<InventoryEntry>> {
        self.live_inventory_actor(slot, actor)?;
        let items = self.inventory_items.clone().unwrap_or_default();
        self.source_client(host, slot, actor)?
            .read_inventory(&items)
            .map_err(host_mapped)
    }

    fn write_source_inventory(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        slot: u32,
        actor: &ActorId,
        entry: &InventoryEntry,
    ) -> RereleaseResult<()> {
        self.live_inventory_actor(slot, actor)?;
        let items = self.inventory_items.clone().unwrap_or_default();
        self.source_client(host, slot, actor)?
            .write_inventory(&items, entry)
            .map_err(host_mapped)
    }

    fn source_inventory_mutable_capacity(&self, item: &ItemId) -> bool {
        match self.inventory_items.as_ref() {
            None => false,
            Some(items) => RereleaseSourceClient::mutable_capacity(items, item),
        }
    }

    fn vector(host: &mut RereleaseQ2GuestHost, address: GuestAddress) -> RereleaseResult<Vec3> {
        let x = host.memory.read_f32(address).map_err(host_mapped)?;
        let y = host
            .memory
            .read_f32(host.memory.offset(address, 4).map_err(host_mapped)?)
            .map_err(host_mapped)?;
        let z = host
            .memory
            .read_f32(host.memory.offset(address, 8).map_err(host_mapped)?)
            .map_err(host_mapped)?;
        Ok(Vec3 { x, y, z })
    }

    /// Dispatch one game import (donor `bindMemory` `invoke`).
    pub fn invoke(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        call: &RereleaseImportCall,
    ) -> RereleaseResult<Option<GuestCallResult>> {
        let args = &call.arguments;
        let slot_of = |host: &RereleaseQ2GuestHost, index: usize| -> RereleaseResult<Option<u32>> {
            match rr_pointer(args, index)? {
                None => Ok(None),
                Some(address) => Ok(Some(host.slot_for_address(address).map_err(host_mapped)?)),
            }
        };
        match call.name.as_str() {
            "Broadcast_Print" => {
                let level = rr_number(args, 0)? as i32;
                let text = read_guest_string(&mut host.memory, rr_required_pointer(args, 1)?, 1_048_576)
                    .map_err(host_mapped)?;
                self.print(host, None, level, &text, true, false)?;
            }
            "Client_Print" => {
                let slot = slot_of(host, 0)?;
                let level = rr_number(args, 1)? as i32;
                let text = read_guest_string(&mut host.memory, rr_required_pointer(args, 2)?, 1_048_576)
                    .map_err(host_mapped)?;
                self.print(host, slot, level, &text, false, false)?;
            }
            "Center_Print" => {
                let slot = slot_of(host, 0)?;
                let text = read_guest_string(&mut host.memory, rr_required_pointer(args, 1)?, 1_048_576)
                    .map_err(host_mapped)?;
                self.print(host, slot, 5, &text, false, true)?;
            }
            "Loc_Print" => {
                let count = rr_number(args, 4)? as i32;
                if !(0..=8).contains(&count) {
                    return Err(RereleaseGuestServicesError::invalid(
                        "API2023 localization argument count",
                    ));
                }
                let mut values = Vec::new();
                if count > 0 {
                    let base = rr_required_pointer(args, 3)?;
                    for index in 0..count {
                        let at = host
                            .memory
                            .read_pointer(host.memory.offset(base, i64::from(index) * 8).map_err(host_mapped)?)
                            .map_err(host_mapped)?;
                        match at {
                            None => values.push(String::new()),
                            Some(at) => {
                                values.push(read_guest_string(&mut host.memory, at, 1_048_576).map_err(host_mapped)?)
                            }
                        }
                    }
                }
                let level = rr_number(args, 1)? as i32;
                let slot = slot_of(host, 0)?;
                let format = read_guest_string(&mut host.memory, rr_required_pointer(args, 2)?, 1_048_576)
                    .map_err(host_mapped)?;
                let text = (self.options.localize)(&format, &values);
                self.print(host, slot, level & !8, &text, level & 8 != 0, false)?;
            }
            "DebugGraph" => match args.first() {
                Some(GuestCallValue::Float32(value)) => {
                    let value = f64::from(*value);
                    let bucket = rr_number(args, 1)? as i32;
                    (self.options.base.debug_graph)(value, bucket);
                }
                _ => {
                    return Err(RereleaseGuestServicesError::invalid("API2023 graph requires a float"));
                }
            },
            "SendToClipBoard" => {
                if let RereleaseGuestClipboard::Client(write) = &mut self.options.clipboard {
                    let text = read_guest_string(&mut host.memory, rr_required_pointer(args, 0)?, 1_048_576)
                        .map_err(host_mapped)?;
                    write(&text);
                }
            }
            "ReportMatchDetails_Multicast" => {
                self.buffer.clear();
            }
            "Bot_MoveToPoint" | "Bot_FollowActor" | "GetPathToGoal" => {
                let mut navigation = RereleaseNavigationImports::new(OptionsNavigation {
                    services: &mut *self.options.navigation,
                });
                // The navigation call borrows guest memory mutably, so slot
                // resolution mirrors `RereleaseQ2GuestHost::slot_for_address`
                // over copied table geometry instead of borrowing the host.
                let base = host.entity_base.offset;
                let stride = host.entity_stride as u64;
                let capacity = u64::from(host.entity_capacity);
                let result = navigation
                    .invoke(&mut host.memory, &call.name, args, &|address| {
                        if address.offset < base || stride == 0 {
                            return None;
                        }
                        let delta = address.offset - base;
                        if !delta.is_multiple_of(stride) || delta / stride >= capacity {
                            return None;
                        }
                        Some((delta / stride) as u32)
                    })
                    .map_err(host_mapped)?;
                return Ok(Some(result));
            }
            "Info_RemoveKey" | "Info_SetValueForKey" => {
                return Ok(Some(self.info(host, call)?));
            }
            "clip" => {
                return Ok(Some(self.clip(host, call)?));
            }
            _ => return Ok(None),
        }
        Ok(Some(GuestCallResult::Void))
    }

    fn info(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        call: &RereleaseImportCall,
    ) -> RereleaseResult<GuestCallResult> {
        let args = &call.arguments;
        let address = rr_required_pointer(args, 0)?;
        let key = read_guest_string(&mut host.memory, rr_required_pointer(args, 1)?, 2048).map_err(host_mapped)?;
        let input = read_guest_string(&mut host.memory, address, 2048).map_err(host_mapped)?;
        let mut output = String::new();
        let mut found = false;
        let mut cursor = 0;
        while cursor < input.len() {
            let start = cursor;
            let rest = &input[cursor..];
            let rest = rest.strip_prefix('\\').unwrap_or(rest);
            cursor += input[cursor..].len() - rest.len();
            let Some(separator) = rest.find('\\') else {
                output.push_str(&input[start..]);
                break;
            };
            let end = rest[separator + 1..]
                .find('\\')
                .map_or(input.len(), |next| cursor + separator + 1 + next);
            if rest[..separator] == key {
                found = true;
            } else {
                output.push_str(&input[start..end]);
            }
            cursor = end;
        }
        let mut result = found;
        if call.name == "Info_SetValueForKey" {
            let value =
                read_guest_string(&mut host.memory, rr_required_pointer(args, 2)?, 2048).map_err(host_mapped)?;
            if key.len() >= 64
                || value.len() >= 256
                || key.contains(['\\', '"', ';'])
                || value.contains(['\\', '"', ';'])
            {
                return Ok(GuestCallResult::Value(guest_bool(false)));
            }
            result = value.is_empty() || output.len() + key.len() + value.len() + 2 < 2048;
            if result && !value.is_empty() {
                let pair = format!("\\{key}\\{value}");
                for unit in pair.encode_utf16() {
                    let byte = (unit & 127) as u8;
                    if (32..127).contains(&byte) {
                        output.push(byte as char);
                    }
                }
            }
        }
        let bytes = output.as_bytes();
        host.memory.write(address, bytes).map_err(host_mapped)?;
        let end = host.memory.offset(address, bytes.len() as i64).map_err(host_mapped)?;
        host.memory.write_u8(end, 0).map_err(host_mapped)?;
        Ok(GuestCallResult::Value(guest_bool(result)))
    }

    fn clip(
        &mut self,
        host: &mut RereleaseQ2GuestHost,
        call: &RereleaseImportCall,
    ) -> RereleaseResult<GuestCallResult> {
        use qa_bots::scene::{LeafContents, QueryTarget, TracePolicy, TraceShape};
        let args = &call.arguments;
        let address = rr_required_pointer(args, 0)?;
        let slot = host.slot_for_address(address).map_err(host_mapped)?;
        let actor = self.actors.get(&slot).cloned();
        let min = rr_pointer(args, 2)?;
        let max = rr_pointer(args, 3)?;
        if min.is_none() != max.is_none() {
            return Err(RereleaseGuestServicesError::invalid(
                "API2023 clip requires both bounds or neither",
            ));
        }
        let state = self.read_body(host, slot)?;
        let shape = match (min, max) {
            (Some(min), Some(max)) => TraceShape::Box {
                bounds: Bounds {
                    min: Self::vector(host, min)?,
                    max: Self::vector(host, max)?,
                },
            },
            _ => TraceShape::Point,
        };
        let query = qa_bots::scene::TraceQuery {
            start: Self::vector(host, rr_required_pointer(args, 1)?)?,
            end: Self::vector(host, rr_required_pointer(args, 4)?)?,
            shape,
            target: QueryTarget::World,
            policy: TracePolicy::Q2 {
                contents_mask: rr_number(args, 5)? as i32,
                leaf_contents: LeafContents::Merged,
            },
            numeric: self.options.base.numeric.profile,
            pass_actor: None,
        };
        let mut view = Self::view(host, slot)?;
        let solid = view.byte("solid").map_err(host_mapped)?;
        let model_index = view.int("s.modelindex").map_err(host_mapped)?;
        let source_model = self.resource(ResourceKind::Model, model_index);
        let hit = if slot == 0 {
            self.options.base.scene.geometry_trace(&query)
        } else if solid == 3 || solid == 1 && source_model.starts_with('*') {
            self.options.base.scene.geometry_trace(&qa_bots::scene::TraceQuery {
                target: QueryTarget::Model {
                    model: self.inline_model(model_index)?,
                    origin: state.origin,
                    angles: state.angles,
                },
                ..query.clone()
            })
        } else {
            let trace = self.body_trace.as_mut().ok_or_else(|| {
                RereleaseGuestServicesError::invalid(
                    "API2023 entity clip requires the world collision trace (donor traceQ2Box)",
                )
            })?;
            let mut view = Self::view(host, slot)?;
            let link_count = view.int("linkcount").map_err(host_mapped)?;
            trace(
                &query,
                &RereleaseBodyTraceActor {
                    actor: actor.unwrap_or_else(|| self.options.base.engine.world_actor()),
                    state: state.clone(),
                    absolute_bounds: rerelease_link_bounds(
                        &LinkBody {
                            origin: state.origin,
                            angles: state.angles,
                            bounds: state.bounds,
                        },
                        i32::from(solid),
                    ),
                    link_count,
                    collision: ActorCollision {
                        family: CollisionFamily::Q2,
                        shape: CollisionShape::Box,
                        contents: 0x2000000,
                        owner: None,
                        role: CollisionRole::Solid,
                        monster: false,
                        dead_monster: false,
                        q1_corpse: false,
                        q3_owner: None,
                    },
                },
            )?
        };
        let trace = Self::convert_trace(&hit);
        host.encode_trace(&trace, Some(address)).map_err(host_mapped)
    }

    fn convert_trace(hit: &qa_bots::scene::TraceResult) -> Q2Trace {
        use qa_bots::scene::{TraceContact, TraceDetail};
        let (plane_normal, plane_distance) = match &hit.contact {
            TraceContact::Plane { plane } => (plane.normal, plane.distance),
            TraceContact::None => (Vec3 { x: 0.0, y: 0.0, z: 0.0 }, 0.0),
        };
        let (contents, surface, plane_type, signbits) = match &hit.detail {
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane,
            } => (
                *contents as u32,
                surface.as_ref().map(|info| TraceSurface {
                    name: info.name.clone(),
                    flags: info.flags as u32,
                    value: 0,
                    material: String::new(),
                }),
                source_plane.plane_type as u8,
                source_plane.signbits as u8,
            ),
            _ => (0, None, 0, 0),
        };
        Q2Trace {
            all_solid: hit.all_solid,
            start_solid: hit.start_solid,
            fraction: hit.fraction as f32,
            end: hit.end,
            plane_normal,
            plane_distance,
            plane_type,
            plane_signbits: signbits,
            surface,
            contents,
            hit: HostTraceHit::World,
            secondary: None,
        }
    }
}

impl RereleaseGuestServicesPort for RereleaseGuestServices {
    fn options(&self) -> &RereleaseGuestServicesOptions {
        &self.options
    }

    fn bind_host(&mut self, host: &mut RereleaseQ2GuestHost) -> RereleaseResult<()> {
        if self.bound {
            return Err(RereleaseGuestServicesError::invalid(
                "API2023 host/memory binding mismatch",
            ));
        }
        let _ = host;
        self.bound = true;
        self.combat = Some(RereleaseCombatBindings::new());
        Ok(())
    }

    fn complete_spawn(&mut self) {
        self.loading = false;
    }

    fn validate_map(binding: &RereleaseGuestMapServices) -> RereleaseResult<()> {
        if binding.base.scene.model_count() >= 8191 {
            return Err(RereleaseGuestServicesError::invalid(
                "API2023 map exceeds model capacity",
            ));
        }
        Ok(())
    }

    fn publish_entities(&mut self, host: &mut RereleaseQ2GuestHost) -> RereleaseResult<()> {
        for slot in 1..host.entity_count {
            let actor = match self.actors.get(&slot) {
                None => continue,
                Some(actor) => actor.clone(),
            };
            Self::write_entity_number(host, slot)?;
            let mut view = Self::view(host, slot)?;
            let state = view.state().map_err(host_mapped)?;
            if state.effects > (1u64 << 53) - 1 {
                return Err(RereleaseGuestServicesError::invalid(
                    "Shared model effect projection requires a lossless API2023 effect value",
                ));
            }
            let inuse = view.byte("inuse").map_err(host_mapped)? != 0;
            let svflags = view.uint("svflags").map_err(host_mapped)?;
            let appearance = self.appearance(&state.model_indexes, state.skin);
            self.options.base.engine.emit(Q2PresentationEvent::Visibility {
                actor: actor.clone(),
                visible: inuse && svflags & 1 == 0,
            });
            self.options.base.engine.emit(Q2PresentationEvent::Model(Q2ModelEvent {
                actor: actor.clone(),
                path: appearance.path,
                attached_models: appearance.attached_models,
                frame: state.frame,
                old_frame: state.old_frame,
                scale: if state.scale == 0.0 {
                    1.0
                } else {
                    f64::from(state.scale)
                },
                alpha: if state.alpha == 0.0 {
                    if state.render_effects & 32 != 0 {
                        0.3
                    } else {
                        1.0
                    }
                } else {
                    f64::from(state.alpha)
                },
                skin: appearance.skin,
                effects: state.effects as i64,
                render_flags: state.render_effects as i32,
            }));
            if state.event != 0 {
                self.options.base.engine.emit(Q2PresentationEvent::EntityEvent {
                    actor,
                    event: i32::from(state.event),
                });
            }
        }
        Ok(())
    }

    fn player_ping(&self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<i32> {
        Self::view(host, slot)?.ping().map_err(host_mapped)
    }

    fn set_player_ping(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32, ping: i32) -> RereleaseResult<()> {
        Self::view(host, slot)?.set_ping(ping).map_err(host_mapped)
    }

    fn begin_frame(&mut self, frame: u32) {
        self.frame = frame;
        let dead: Vec<ActorId> = self
            .links
            .keys()
            .filter(|actor| !self.options.base.engine.actors().is_live(actor))
            .cloned()
            .collect();
        for actor in dead {
            self.links.remove(&actor);
        }
    }

    fn rebind_world(&mut self, binding: RereleaseGuestMapServices) -> RereleaseResult<()> {
        Self::validate_map(&binding)?;
        self.options.base.engine = binding.base.engine;
        self.options.base.scene = binding.base.scene;
        self.options.base.map_path = binding.base.map_path;
        self.options.base.admit = binding.base.admit;
        self.options.base.collision = binding.base.collision;
        self.options.base.print = binding.base.print;
        self.options.base.command = binding.base.command;
        self.options.base.add_command = binding.base.add_command;
        self.options.base.debug_graph = binding.base.debug_graph;
        self.options.base.damage_provenance = binding.base.damage_provenance;
        self.options.base.pickups = binding.base.pickups;
        self.options.frame_milliseconds = binding.frame_milliseconds;
        self.options.localize = binding.localize;
        self.options.debug_shapes = binding.debug_shapes;
        self.options.world_text = binding.world_text;
        self.options.navigation = binding.navigation;
        self.options.foreign_damage = binding.foreign_damage;
        self.loading = true;
        self.frame = 0;
        self.strings.clear();
        self.messages.clear();
        self.links.clear();
        self.actors.clear();
        self.buffer.clear();
        self.seed_map();
        Ok(())
    }

    fn entity_state(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<Q2RereleaseEntityState> {
        Self::write_entity_number(host, slot)?;
        let state = Self::view(host, slot)?.state().map_err(host_mapped)?;
        Ok(Q2RereleaseEntityState {
            base: Q2EntityState {
                number: state.number as u16,
                origin: q2_vec(state.origin),
                angles: q2_vec(state.angles),
                old_origin: q2_vec(state.old_origin),
                model_indexes: state.model_indexes.map(|index| index as u16),
                frame: state.frame,
                skin: state.skin,
                effects: state.effects as u32,
                render_effects: state.render_effects,
                solid: state.solid,
                sound: state.sound as u16,
                event: state.event,
            },
            effects: state.effects,
            alpha: f64::from(state.alpha),
            scale: f64::from(state.scale),
            instance_bits: state.instance_bits,
            loop_volume: f64::from(state.loop_volume),
            loop_attenuation: f64::from(state.loop_attenuation),
            owner: state.owner as u16,
            old_frame: state.old_frame as u16,
        })
    }

    fn player_state(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<Q2RereleasePlayerState> {
        let client = Self::view(host, slot)?.pointer("client").map_err(host_mapped)?;
        let client =
            client.ok_or_else(|| RereleaseGuestServicesError::invalid("API2023 player state has no client"))?;
        let state = read_player_state(&mut host.memory, client).map_err(host_mapped)?;
        Ok(Q2RereleasePlayerState {
            view: Q2PlayerView {
                view_angles: q2_vec(state.view_angles),
                view_offset: q2_vec(state.view_offset),
                kick_angles: q2_vec(state.kick_angles),
                gun_angles: q2_vec(state.gun_angles),
                gun_offset: q2_vec(state.gun_offset),
                gun_index: state.gun_index,
                gun_frame: state.gun_frame,
                fov: state.fov as u8,
                render_flags: state.render_flags,
                stats: state.stats.clone(),
            },
            movement: Q2RereleaseMovementState {
                move_type: state.movement.move_type as u8,
                origin: q2_vec(state.movement.origin),
                velocity: q2_vec(state.movement.velocity),
                flags: i32::from(state.movement.flags),
                time: i32::from(state.movement.time_milliseconds),
                gravity: state.movement.gravity,
                delta_angles: q2_vec(state.movement.delta_angles),
                view_height: i32::from(state.movement.view_height),
            },
            gun_skin: state.gun_skin,
            gun_rate: state.gun_rate as u8,
            screen_blend: Q2Vec4 {
                x: f64::from(state.screen_blend.x),
                y: f64::from(state.screen_blend.y),
                z: f64::from(state.screen_blend.z),
                w: f64::from(state.screen_blend.w),
            },
            damage_blend: Q2Vec4 {
                x: f64::from(state.damage_blend.x),
                y: f64::from(state.damage_blend.y),
                z: f64::from(state.damage_blend.z),
                w: f64::from(state.damage_blend.w),
            },
            team_id: state.team_id,
        })
    }

    fn model_appearance(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<ModelAppearance> {
        Self::write_entity_number(host, slot)?;
        let state = Self::view(host, slot)?.model_state().map_err(host_mapped)?;
        Ok(self.appearance(&state.model_indexes, state.skin))
    }

    fn entity_info(&mut self, host: &mut RereleaseQ2GuestHost, slot: u32) -> RereleaseResult<RereleaseEntityInfo> {
        let actor = self.actors.get(&slot).cloned();
        let link = actor.as_ref().and_then(|actor| self.links.get(actor).cloned());
        let (active, server_flags, areanum, areanum2, owner) = {
            let mut view = Self::view(host, slot)?;
            (
                view.byte("inuse").map_err(host_mapped)? != 0,
                view.uint("svflags").map_err(host_mapped)?,
                view.int("areanum").map_err(host_mapped)?,
                view.int("areanum2").map_err(host_mapped)?,
                view.pointer("owner").map_err(host_mapped)?,
            )
        };
        let owner_slot = match owner {
            None => None,
            Some(address) => Some(host.slot_for_address(address).map_err(host_mapped)?),
        };
        Ok(RereleaseEntityInfo {
            actor,
            active,
            server_flags: server_flags as i32,
            areas: (areanum, areanum2),
            clusters: link.as_ref().map_or(Some(Vec::new()), |link| link.clusters.clone()),
            first_cluster: link.as_ref().map_or(0, |link| link.first_cluster),
            headnode: link.as_ref().map_or(0, |link| link.headnode),
            owner_slot,
        })
    }

    fn drain_messages(&mut self) -> Vec<RereleaseGuestMessage> {
        std::mem::take(&mut self.messages)
    }

    fn configstrings(&self) -> HashMap<i32, String> {
        self.strings.clone()
    }

    fn set_configstring(&mut self, index: i32, value: &str) -> RereleaseResult<()> {
        Self::validate_configstring(index, value)?;
        if self.strings.get(&index).is_some_and(|current| current == value) {
            return Ok(());
        }
        self.store_configstring(index, value);
        let removed_weapon = self.weapon_models.remove(&index).is_some();
        if Self::is_weapon_model(index, value) {
            self.weapon_models.insert(index, value[1..].to_string());
            self.rebuild_weapon_models();
        } else if removed_weapon {
            self.rebuild_weapon_models();
        }
        if (10814..11070).contains(&index) {
            self.options.base.engine.emit(Q2PresentationEvent::LightStyle {
                style: index - 10814,
                pattern: value.to_string(),
            });
        }
        if index == 1 {
            self.options.base.engine.emit(Q2PresentationEvent::Music {
                track: value.to_string(),
            });
        }
        if !self.loading {
            let mut writer = MsgWriter::new(65536, false);
            write_byte_wrapped(&mut writer, 13.0)?;
            write_short_direct(&mut writer, index)?;
            write_string_direct(&mut writer, value)?;
            let bytes = writer.bytes().to_vec();
            self.enqueue(
                ClassicGuestAudience::Multicast {
                    origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    scope: MulticastScope::All,
                },
                true,
                bytes,
                0,
            );
        }
        Ok(())
    }

    fn restore_configstrings(&mut self, values: &HashMap<i32, String>) -> RereleaseResult<()> {
        for (index, value) in values {
            Self::validate_configstring(*index, value)?;
        }
        self.strings.clear();
        self.weapon_models.clear();
        for table in &mut self.resource_names {
            table.clear();
        }
        for (index, value) in values {
            self.store_configstring(*index, value);
            if Self::is_weapon_model(*index, value) {
                self.weapon_models.insert(*index, value[1..].to_string());
            }
        }
        self.rebuild_weapon_models();
        Ok(())
    }
}

impl RereleaseCoreServices for RereleaseGuestServices {
    fn print(&mut self, text: &str) {
        (self.options.base.print)(text);
    }

    fn get_configstring(&self, index: i32) -> String {
        self.strings.get(&index).cloned().unwrap_or_default()
    }

    fn set_configstring(&mut self, index: i32, value: &str) {
        // The core-services trait carries no error channel; invalid writes
        // are dropped. Use the checked port method for fallible writes.
        let _ = RereleaseGuestServicesPort::set_configstring(self, index, value);
    }

    fn resource_index(&mut self, kind: &str, name: &str) -> i32 {
        let kind = match kind {
            "model" => ResourceKind::Model,
            "sound" => ResourceKind::Sound,
            "image" => ResourceKind::Image,
            unknown => panic!("invalid API2023 resource kind {unknown}"),
        };
        // The core-services trait carries no error channel; overflow resolves
        // to the empty slot. Use the checked inherent method for fallible writes.
        self.resource_index(kind, name).unwrap_or(0)
    }

    fn server_frame(&self) -> u32 {
        self.frame
    }

    fn command_arguments(&self) -> Vec<String> {
        (self.options.base.command)().arguments
    }

    fn command_tail(&self) -> String {
        (self.options.base.command)().args
    }

    fn add_command(&mut self, text: &str) {
        (self.options.base.add_command)(text);
    }

    fn extension(&self, _name: &str, _api: &str) -> Option<GuestAddress> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_bots::scene::{
        LeafQueryResult, PointContentsQuery, PointContentsResult, TraceContact, TraceDetail, TraceHit, TraceQuery,
        TraceResult as BotsTraceResult, VisibilityKind,
    };
    use qa_compat::q2::rerelease::host::HostOptions;
    use qa_compat::q2::rerelease::module::GuestApi;
    use qa_compat::q2::rerelease::navigation::NavRuntime;
    use qa_content::q2::foundation::host::{
        Q2FoundationHost, Q2LandmarkCarry, Q2Motion, Q2PlayerViewState, Q2Solid, Q2TraceRequest,
    };
    use qa_content::q2::support::contracts::TransitionIntent;
    use qa_content::q2::support::contracts::{
        ActorObservation, BodyAttachment, BodyState as ContentBodyState, CombatState, DamageOutcome, DamageRequest,
        LinkedBody, Q2BspPlane as ContentBspPlane, TraceContact as ContentContact, TraceFamily, TraceHit as ContentHit,
        TraceResult as ContentTraceResult,
    };
    use qa_content::q2::support::tables::{
        Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
    };
    use qa_core::cmd::Dialect;
    use qa_core::cvar::CvarRegistry;
    use qa_core::identity::{IdentityOwner, ProviderId, SavedActorId};
    use qa_core::math::Bounds;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};
    use qa_guest::core::contracts::{GuestCallValue, ModuleIdentity};
    use qa_guest::core::memory::SparseGuestMemory;
    use qa_world::spatial::QueryRole;

    use super::super::classic_guest_services::{ClassicGuestServicesOptions, GuestCommandLine};
    use super::super::source_hosts::ActorHostScene;
    use super::*;

    const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    const ZERO_BOUNDS: Bounds = Bounds { min: ZERO, max: ZERO };

    struct FakeShared {
        next_slot: u32,
        actors: HashMap<ActorId, (ProviderId, u32, String)>,
        emitted: Vec<String>,
        prints: Vec<String>,
        admissions: Vec<u32>,
        collisions: Vec<String>,
    }

    struct FakeEngine {
        owner: Rc<IdentityOwner>,
        shared: Rc<RefCell<FakeShared>>,
        bodies: HashMap<ActorId, ContentBodyState>,
        callbacks: HashMap<ActorId, bool>,
        combats: HashMap<ActorId, CombatState>,
        inventories: HashMap<ActorId, Vec<qa_content::contract::InventoryEntry>>,
        world: ActorId,
    }

    impl FakeEngine {
        fn new() -> Self {
            let owner = Rc::new(IdentityOwner::create("rerelease-services-test").expect("owner"));
            let world = owner.actor(0, 0);
            Self {
                owner,
                shared: Rc::new(RefCell::new(FakeShared {
                    next_slot: 1,
                    actors: HashMap::new(),
                    emitted: Vec::new(),
                    prints: Vec::new(),
                    admissions: Vec::new(),
                    collisions: Vec::new(),
                })),
                bodies: HashMap::new(),
                callbacks: HashMap::new(),
                combats: HashMap::new(),
                inventories: HashMap::new(),
                world,
            }
        }
    }

    impl Q2ActorRegistry for FakeEngine {
        fn allocate(&mut self, owner: &ProviderId, definition: &str) -> OwnedActor {
            let mut shared = self.shared.borrow_mut();
            let slot = shared.next_slot;
            shared.next_slot += 1;
            let id = self.owner.actor(slot, 1);
            shared
                .actors
                .insert(id.clone(), (owner.clone(), slot, definition.to_string()));
            drop(shared);
            self.owner.owned_actor(&id, owner.clone()).expect("owned")
        }

        fn allocate_at_source(&mut self, owner: &ProviderId, source_slot: u32, definition: &str) -> OwnedActor {
            let id = self.owner.actor(source_slot, 1);
            self.shared
                .borrow_mut()
                .actors
                .insert(id.clone(), (owner.clone(), source_slot, definition.to_string()));
            self.owner.owned_actor(&id, owner.clone()).expect("owned")
        }

        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, u32)> {
            self.shared
                .borrow()
                .actors
                .get(actor)
                .map(|(owner, slot, _)| (owner.clone(), *slot))
        }

        fn release(&mut self, actor: &OwnedActor) {
            self.shared.borrow_mut().actors.remove(actor.id());
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            *actor == self.world || self.shared.borrow().actors.contains_key(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.shared
                .borrow()
                .actors
                .get(actor)
                .map(|(owner, _, _)| self.owner.owned_actor(actor, owner.clone()).expect("owned"))
        }

        fn observations(&self) -> Vec<ActorObservation> {
            self.shared
                .borrow()
                .actors
                .iter()
                .map(|(id, (owner, _, definition))| ActorObservation {
                    id: id.clone(),
                    owner: owner.clone(),
                    definition: definition.clone(),
                })
                .collect()
        }

        fn resolve_saved(&self, _saved: SavedActorId) -> Option<OwnedActor> {
            None
        }

        fn reference_saved(&self, saved: SavedActorId) -> ActorId {
            self.owner.actor(saved.slot, saved.generation)
        }

        fn assert_owned(&self, _actor: &OwnedActor) {}
    }

    impl Q2BodyTable for FakeEngine {
        fn create(&mut self, actor: &OwnedActor, initial: &ContentBodyState) {
            self.bodies.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<ContentBodyState> {
            self.bodies.get(actor).cloned()
        }

        fn write(&mut self, actor: &OwnedActor, state: &ContentBodyState) {
            self.bodies.insert(actor.id().clone(), state.clone());
        }

        fn attach(&mut self, _actor: &OwnedActor, _attachment: &BodyAttachment) {}

        fn detach(&mut self, _actor: &OwnedActor) {}

        fn attachment(&self, _actor: &ActorId) -> Option<BodyAttachment> {
            None
        }

        fn linked(&self, _actor: &ActorId) -> Option<LinkedBody> {
            None
        }

        fn link(&mut self, _actor: &OwnedActor, _origin: Option<Vec3>) {}

        fn unlink(&mut self, _actor: &OwnedActor) {}
    }

    impl Q2CallbackTable for FakeEngine {
        fn bind(&mut self, actor: &OwnedActor) {
            self.callbacks.insert(actor.id().clone(), true);
        }

        fn unbind(&mut self, actor: &ActorId) {
            self.callbacks.remove(actor);
        }

        fn is_bound(&self, actor: &ActorId) -> bool {
            self.callbacks.contains_key(actor)
        }

        fn forward_use(&mut self, _actor: &OwnedActor, _other: Option<&ActorId>, _activator: Option<&ActorId>) {}
    }

    impl Q2CombatAuthority for FakeEngine {
        fn create(&mut self, actor: &OwnedActor, initial: &CombatState) {
            self.combats.insert(actor.id().clone(), initial.clone());
        }

        fn read(&self, actor: &ActorId) -> Option<CombatState> {
            self.combats.get(actor).cloned()
        }

        fn set_health(&mut self, _actor: &OwnedActor, _health: f64) {}

        fn set_armor(&mut self, _actor: &OwnedActor, _armor: &qa_content::contract::ArmorState) {}

        fn set_regular_points(
            &mut self,
            _actor: &OwnedActor,
            _points: f64,
            _initial: Option<&qa_content::contract::RegularArmorState>,
        ) {
        }

        fn set_regular_armor(&mut self, _actor: &OwnedActor, _regular: &qa_content::contract::RegularArmorState) {}

        fn set_powered_protection(
            &mut self,
            _actor: &OwnedActor,
            _powered: &qa_content::contract::PoweredProtectionState,
        ) {
        }

        fn set_traits(
            &mut self,
            _actor: &OwnedActor,
            _changes: &qa_content::q2::support::contracts::CombatTraitChanges,
        ) {
        }

        fn bind_power_armor_cells(
            &mut self,
            _actor: &OwnedActor,
            _cells: Box<dyn qa_content::q2::support::contracts::PowerArmorCells>,
        ) {
        }

        fn apply(&mut self, input: &DamageRequest) -> DamageOutcome {
            DamageOutcome::StaleTarget { request: input.clone() }
        }
    }

    impl Q2InventoryTable for FakeEngine {
        fn create(&mut self, actor: &OwnedActor, entries: &[qa_content::contract::InventoryEntry]) {
            self.inventories.insert(actor.id().clone(), entries.to_vec());
        }

        fn entries(&self, actor: &ActorId) -> Vec<qa_content::contract::InventoryEntry> {
            self.inventories.get(actor).cloned().unwrap_or_default()
        }

        fn has(&self, actor: &ActorId) -> bool {
            self.inventories.contains_key(actor)
        }

        fn count(&self, _actor: &ActorId, _item: &ItemId) -> f64 {
            0.0
        }

        fn consume(&mut self, _actor: &OwnedActor, _item: &ItemId, _count: f64) -> bool {
            false
        }

        fn give(&mut self, _actor: &OwnedActor, _item: &ItemId, _count: f64) -> f64 {
            0.0
        }

        fn configure(&mut self, _actor: &OwnedActor, _entry: &qa_content::contract::InventoryEntry) {}

        fn adjust_source_counter(&mut self, _actor: &OwnedActor, _item: &ItemId, _delta: f64) -> f64 {
            0.0
        }
    }

    impl Q2FoundationHost for FakeEngine {
        fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
            self
        }

        fn bodies(&mut self) -> &mut dyn Q2BodyTable {
            self
        }

        fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
            self
        }

        fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
            self
        }

        fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
            self
        }

        fn now(&self) -> f64 {
            0.0
        }

        fn frame_seconds(&self) -> f64 {
            0.016
        }

        fn gravity(&self) -> f64 {
            800.0
        }

        fn random(&mut self) -> f64 {
            0.5
        }

        fn schedule(&mut self, _actor: &OwnedActor, _due_seconds: Option<f64>) {}

        fn touch_triggers(&mut self, _actor: &OwnedActor) {}

        fn trace(&mut self, request: &Q2TraceRequest) -> ContentTraceResult {
            ContentTraceResult {
                fraction: 1.0,
                end: request.end,
                start_solid: false,
                all_solid: false,
                contact: ContentContact::None,
                hit: ContentHit::None,
                family: TraceFamily::Q2(qa_content::q2::support::contracts::Q2TraceFields {
                    contents: 0,
                    surface: None,
                    source_plane: ContentBspPlane {
                        normal: ZERO,
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                    secondary: None,
                }),
            }
        }

        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }

        fn in_pvs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn in_phs(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn areas_connected(&mut self, _first: Vec3, _second: Vec3) -> bool {
            true
        }

        fn nearby(&mut self, _origin: Vec3, _radius: f64) -> Vec<ActorId> {
            Vec::new()
        }

        fn players(&mut self) -> Vec<ActorId> {
            Vec::new()
        }

        fn world_actor(&mut self) -> ActorId {
            self.world.clone()
        }

        fn is_player(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn is_monster(&mut self, _actor: &ActorId) -> bool {
            false
        }

        fn inline_model_bounds(&mut self, _model: i32) -> Bounds {
            ZERO_BOUNDS
        }

        fn set_solid(&mut self, _actor: &OwnedActor, _solid: Q2Solid, _model: Option<i32>) {}

        fn set_motion(&mut self, _motion: &Q2Motion) {}

        fn set_area_portal(&mut self, _portal: i32, _open: bool) {}

        fn emit(&mut self, event: Q2PresentationEvent) {
            let summary = match &event {
                Q2PresentationEvent::LightStyle { style, pattern } => {
                    format!("lightstyle {style} {pattern}")
                }
                Q2PresentationEvent::Music { track } => format!("music {track}"),
                Q2PresentationEvent::Visibility { visible, .. } => {
                    format!("visibility {visible}")
                }
                Q2PresentationEvent::Model(_) => "model".to_string(),
                Q2PresentationEvent::EntityEvent { event, .. } => format!("entity-event {event}"),
                _ => "other".to_string(),
            };
            self.shared.borrow_mut().emitted.push(summary);
        }

        fn player_view_state(&mut self, _player: &ActorId) -> Option<Q2PlayerViewState> {
            None
        }

        fn key_consumed(&mut self, _player: &ActorId) {}

        fn prepare_level_change(&mut self, _map: &str, _landmark: Option<&Q2LandmarkCarry>, _server_flags: i32) {}

        fn transition(&mut self, _intent: TransitionIntent) {}

        fn diagnostic(&mut self, _message: &str) {}
    }

    struct FakeScene {
        models: usize,
        textures: Option<Vec<SceneTextureInfo>>,
        leaves: Vec<i32>,
        actors: Vec<ActorId>,
        trace_hit: BotsTraceResult,
    }

    impl FakeScene {
        fn miss() -> BotsTraceResult {
            BotsTraceResult {
                fraction: 1.0,
                end: ZERO,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                detail: TraceDetail::Q2 {
                    contents: 0,
                    surface: None,
                    source_plane: qa_bots::scene::BspPlane {
                        normal: ZERO,
                        distance: 0.0,
                        plane_type: 0,
                        signbits: 0,
                    },
                },
            }
        }
    }

    impl qa_bots::scene::SceneQueries for FakeScene {
        fn trace(&self, _query: &TraceQuery) -> BotsTraceResult {
            self.trace_hit.clone()
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
            PointContentsResult::Q2 { stored: 0, merged: 0 }
        }

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
            LeafQueryResult {
                leaves: self.leaves.clone(),
                topnode: Some(7),
                overflow: false,
            }
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
            true
        }
    }

    impl ActorHostScene for FakeScene {
        fn trace_excluding(&self, _query: &TraceQuery, _excluded: &[ActorId]) -> BotsTraceResult {
            self.trace_hit.clone()
        }

        fn geometry_trace(&self, _query: &TraceQuery) -> BotsTraceResult {
            self.trace_hit.clone()
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            3
        }

        fn leaf_cluster(&self, leaf: i32) -> i32 {
            leaf
        }

        fn leaf_area(&self, leaf: i32) -> i32 {
            leaf
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            ZERO_BOUNDS
        }

        fn query_actors(&self, _bounds: &Bounds, _role: QueryRole) -> Vec<qa_world::spatial::SpatialActor> {
            self.actors
                .iter()
                .map(|actor| qa_world::spatial::SpatialActor {
                    body: qa_world::body::LinkedBody {
                        actor: actor.clone(),
                        state: qa_world::body::BodyState {
                            origin: ZERO,
                            angles: ZERO,
                            velocity: ZERO,
                            bounds: ZERO_BOUNDS,
                            ground: None,
                        },
                        absolute_bounds: ZERO_BOUNDS,
                        link_count: 1,
                    },
                    collision: ActorCollision {
                        family: CollisionFamily::Q2,
                        shape: CollisionShape::Box,
                        contents: 0,
                        owner: None,
                        role: CollisionRole::Solid,
                        monster: false,
                        dead_monster: false,
                        q1_corpse: false,
                        q3_owner: None,
                    },
                })
                .collect()
        }

        fn model_count(&self) -> usize {
            self.models
        }

        fn q2_texture_info(&self) -> Option<Vec<SceneTextureInfo>> {
            self.textures.clone()
        }
    }

    struct FakeNav;

    impl NavigationServices for FakeNav {
        fn runtime(&self) -> Option<&NavRuntime> {
            None
        }

        fn move_to_point(&mut self, _actor: u32, _point: Vec3, _tolerance: f32) -> u8 {
            0
        }

        fn follow_actor(&mut self, _actor: u32, _target: u32) -> u8 {
            0
        }
    }

    struct Fixture {
        services: RereleaseGuestServices,
        host: RereleaseQ2GuestHost,
        shared: Rc<RefCell<FakeShared>>,
        owner: Rc<IdentityOwner>,
    }

    fn test_host() -> RereleaseQ2GuestHost {
        let memory = SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "rerelease"),
                "q2gamex64.dll",
                qa_guest::core::contracts::ContentDigest::new("sha256", "test"),
                "test",
            ),
            8,
            0x1_0000,
        )
        .expect("memory");
        RereleaseQ2GuestHost::new(
            memory,
            &HostOptions {
                pickups_admitted: false,
                component_projection: false,
                headless_debug: false,
                debug_shapes_bound: false,
                world_text_bound: false,
                foreign_damage: false,
                native_entries: false,
            },
        )
        .expect("host")
    }

    fn write_guest_string(host: &mut RereleaseQ2GuestHost, text: &str) -> GuestAddress {
        let bytes = text.as_bytes();
        let address = host
            .memory
            .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(
                bytes.len() + 1,
            ))
            .expect("alloc");
        host.memory.write(address, bytes).expect("write");
        let end = host.memory.offset(address, bytes.len() as i64).expect("offset");
        host.memory.write_u8(end, 0).expect("nul");
        address
    }

    fn fixture() -> Fixture {
        fixture_with_scene(FakeScene {
            models: 4,
            textures: Some(vec![SceneTextureInfo {
                name: "rock".to_string(),
                flags: 1,
                value: 2,
            }]),
            leaves: vec![3],
            actors: Vec::new(),
            trace_hit: FakeScene::miss(),
        })
    }

    fn fixture_with_scene(scene: FakeScene) -> Fixture {
        let engine = FakeEngine::new();
        let shared = engine.shared.clone();
        let owner = engine.owner.clone();
        let print_shared = shared.clone();
        let admit_shared = shared.clone();
        let collision_shared = shared.clone();
        let base = ClassicGuestServicesOptions {
            primary_world: None,
            pickup_profile: None,
            pickups: None,
            damage_provenance: None,
            engine: Box::new(engine),
            scene: Rc::new(scene),
            cvars: CvarRegistry::new(Dialect::Q2Classic),
            numeric: NumericOps::select(Q2_DONOR_PROFILE).expect("numeric"),
            map_path: "q2dm1".to_string(),
            max_clients: 8,
            accepts_client: None,
            admit: Box::new(move |record, _actor| {
                admit_shared.borrow_mut().admissions.push(record.slot);
            }),
            collision: Box::new(move |actor, collision| {
                collision_shared
                    .borrow_mut()
                    .collisions
                    .push(format!("{}:{:?}", actor.id().slot(), collision.role));
            }),
            print: Box::new(move |text| {
                print_shared.borrow_mut().prints.push(text.to_string());
            }),
            command: Rc::new(|| GuestCommandLine {
                arguments: Vec::new(),
                args: String::new(),
            }),
            add_command: Box::new(|_| {}),
            debug_graph: Box::new(|_, _| {}),
        };
        let options = RereleaseGuestServicesOptions {
            base,
            frame_milliseconds: 100,
            localize: Rc::new(|format, values| format!("{format}?{}", values.join(","))),
            clipboard: RereleaseGuestClipboard::Dedicated,
            debug_shapes: Box::new(|_| {}),
            world_text: Box::new(|_| {}),
            navigation: Box::new(FakeNav),
            semantic_bindings: None,
            foreign_damage: None,
        };
        let services = RereleaseGuestServices::new(options).expect("services");
        Fixture {
            services,
            host: test_host(),
            shared,
            owner,
        }
    }

    fn error_message(error: RereleaseGuestServicesError) -> String {
        match error {
            RereleaseGuestServicesError::Invalid(message) => message,
            other => panic!("expected invalid error, got {other:?}"),
        }
    }

    fn owned(fixture: &Fixture, slot: u32) -> OwnedActor {
        let id = fixture.owner.actor(slot, 1);
        fixture
            .owner
            .owned_actor(&id, ProviderId::new("q2", "test"))
            .expect("owned")
    }

    fn record(host: &RereleaseQ2GuestHost, slot: u32) -> RawEntityView {
        RawEntityView {
            slot,
            address: host.record_at(slot).expect("record"),
            stride_bytes: host.entity_stride,
        }
    }

    fn import_call(name: &str, arguments: Vec<GuestCallValue>) -> RereleaseImportCall {
        RereleaseImportCall {
            api: GuestApi::Game,
            name: name.to_string(),
            arguments,
        }
    }

    #[test]
    fn rejects_invalid_options() {
        let build = |max_clients: u32, frame_ms: i32| {
            let fix = fixture();
            let services = fix.services;
            let mut options = services.options;
            options.base.max_clients = max_clients;
            options.frame_milliseconds = frame_ms;
            RereleaseGuestServices::new(options)
        };
        assert!(error_message(build(0, 100).err().expect("err")).contains("maxclients"));
        assert!(error_message(build(257, 100).err().expect("err")).contains("maxclients"));
        assert!(error_message(build(8, 0).err().expect("err")).contains("frame duration"));
        assert!(error_message(build(8, 7).err().expect("err")).contains("frame duration"));
        assert!(build(8, 100).is_ok());
    }

    #[test]
    fn seeds_map_configstrings() {
        let fix = fixture();
        let strings = RereleaseGuestServicesPort::configstrings(&fix.services);
        assert_eq!(strings.get(&63).map(String::as_str), Some("q2dm1"));
        assert_eq!(strings.get(&64).map(String::as_str), Some("*1"));
        assert_eq!(strings.get(&65).map(String::as_str), Some("*2"));
        assert_eq!(strings.get(&66).map(String::as_str), Some("*3"));
        assert_eq!(fix.services.server_frame(), 0);
    }

    #[test]
    fn set_configstring_validates_broadcasts_and_emits() {
        let mut fix = fixture();
        assert!(error_message(
            RereleaseGuestServicesPort::set_configstring(&mut fix.services, 12448, "x").unwrap_err()
        )
        .contains("configstring"));
        assert!(
            error_message(RereleaseGuestServicesPort::set_configstring(&mut fix.services, 5, "a\0b").unwrap_err())
                .contains("configstring")
        );
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 10814, "mmma").expect("style");
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 1, "track2").expect("music");
        assert!(fix
            .shared
            .borrow()
            .emitted
            .iter()
            .any(|event| event == "lightstyle 0 mmma"));
        assert!(fix.shared.borrow().emitted.iter().any(|event| event == "music track2"));
        assert!(fix.services.drain_messages().is_empty());
        fix.services.complete_spawn();
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 5, "x").expect("set");
        let messages = fix.services.drain_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(&messages[0].base.bytes[..4], &[13, 5, 0, b'x']);
        assert!(messages[0].base.reliable);
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 5, "x").expect("same");
        assert!(fix.services.drain_messages().is_empty());
    }

    #[test]
    fn resource_index_allocates_and_sets() {
        let mut fix = fixture();
        let index = fix
            .services
            .resource_index(ResourceKind::Sound, "weapons/rail.wav")
            .expect("index");
        assert!(index > 0);
        assert_eq!(fix.services.resource(ResourceKind::Sound, index), "weapons/rail.wav");
        assert_eq!(
            fix.services
                .resource_index(ResourceKind::Sound, "weapons/rail.wav")
                .expect("again"),
            index
        );
        assert_eq!(fix.services.resource(ResourceKind::Model, 0), "");
    }

    #[test]
    fn weapon_models_rebuild() {
        let mut fix = fixture();
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 70, "#railgun").expect("set");
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 68, "#shotgun").expect("set");
        assert_eq!(fix.services.weapon_model_list, vec!["weapon.md2", "shotgun", "railgun"]);
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 68, "plain").expect("clear");
        assert_eq!(fix.services.weapon_model_list, vec!["weapon.md2", "railgun"]);
    }

    #[test]
    fn box_edicts_maps_actors_and_validates_area() {
        let id = fixture().owner.actor(9, 1);
        let fix = fixture_with_scene(FakeScene {
            models: 4,
            textures: None,
            leaves: vec![3],
            actors: vec![id.clone()],
            trace_hit: FakeScene::miss(),
        });
        assert_eq!(fix.services.box_edicts(ZERO, ZERO, 1).expect("box"), vec![id]);
        assert!(error_message(fix.services.box_edicts(ZERO, ZERO, 3).unwrap_err()).contains("BoxEdicts"));
    }

    #[test]
    fn surface_id_matches_texture_records() {
        let fix = fixture();
        assert_eq!(
            fix.services.surface_id(&SceneTextureInfo {
                name: "rock".to_string(),
                flags: 1,
                value: 2
            }),
            0
        );
        assert_eq!(
            fix.services.surface_id(&SceneTextureInfo {
                name: "sand".to_string(),
                flags: 0,
                value: 0
            }),
            0
        );
        let bare = fixture_with_scene(FakeScene {
            models: 4,
            textures: None,
            leaves: vec![3],
            actors: Vec::new(),
            trace_hit: FakeScene::miss(),
        });
        assert_eq!(
            bare.services.surface_id(&SceneTextureInfo {
                name: "rock".to_string(),
                flags: 1,
                value: 2
            }),
            0
        );
    }

    #[test]
    fn visibility_combines_cluster_and_area() {
        let fix = fixture();
        assert!(fix.services.visibility(VisibilityKind::Pvs, ZERO, ZERO, true));
    }

    #[test]
    fn admit_entity_tracks_and_admits() {
        let mut fix = fixture();
        let actor = owned(&fix, 2);
        let view = record(&fix.host, 2);
        fix.services.admit_entity(&view, &actor);
        assert_eq!(fix.shared.borrow().admissions, vec![2]);
        let info = fix.services.entity_info(&mut fix.host, 2).expect("info");
        assert_eq!(info.actor, Some(actor.id().clone()));
        assert_eq!(info.clusters, Some(Vec::new()));
        assert!(error_message(fix.services.foreign_address(actor.id()).unwrap_err()).contains("semantic projection"));
    }

    #[test]
    fn bind_host_rejects_double_binding() {
        let mut fix = fixture();
        assert!(fix.services.notarget(1).is_none());
        RereleaseGuestServicesPort::bind_host(&mut fix.services, &mut fix.host).expect("bind");
        assert!(RereleaseGuestServicesPort::bind_host(&mut fix.services, &mut fix.host).is_err());
        let _ = fix.services.notarget(1);
    }

    #[test]
    fn info_set_and_remove_key() {
        let mut fix = fixture();
        let buffer = {
            let text = "\\name\\a\\team\\b";
            let address = fix
                .host
                .memory
                .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(64))
                .expect("alloc");
            fix.host.memory.write(address, text.as_bytes()).expect("write");
            let end = fix.host.memory.offset(address, text.len() as i64).expect("offset");
            fix.host.memory.write_u8(end, 0).expect("nul");
            address
        };
        let key = write_guest_string(&mut fix.host, "team");
        let call = import_call(
            "Info_RemoveKey",
            vec![
                GuestCallValue::Pointer(Some(buffer)),
                GuestCallValue::Pointer(Some(key)),
            ],
        );
        let result = fix
            .services
            .invoke(&mut fix.host, &call)
            .expect("invoke")
            .expect("result");
        assert!(matches!(result, GuestCallResult::Value(_)));
        let output = read_guest_string(&mut fix.host.memory, buffer, 2048).expect("readback");
        assert_eq!(output, "\\name\\a");
        let value = write_guest_string(&mut fix.host, "3");
        let key = write_guest_string(&mut fix.host, "skill");
        let call = import_call(
            "Info_SetValueForKey",
            vec![
                GuestCallValue::Pointer(Some(buffer)),
                GuestCallValue::Pointer(Some(key)),
                GuestCallValue::Pointer(Some(value)),
            ],
        );
        fix.services.invoke(&mut fix.host, &call).expect("set").expect("result");
        let output = read_guest_string(&mut fix.host.memory, buffer, 2048).expect("readback");
        assert_eq!(output, "\\name\\a\\skill\\3");
    }

    #[test]
    fn broadcast_print_enqueues_and_prints() {
        let mut fix = fixture();
        let text = write_guest_string(&mut fix.host, "hello");
        let call = import_call(
            "Broadcast_Print",
            vec![GuestCallValue::Int32(2), GuestCallValue::Pointer(Some(text))],
        );
        let result = fix
            .services
            .invoke(&mut fix.host, &call)
            .expect("invoke")
            .expect("result");
        assert_eq!(result, GuestCallResult::Void);
        assert_eq!(fix.shared.borrow().prints, vec!["hello".to_string()]);
        let messages = fix.services.drain_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(&messages[0].base.bytes[..3], &[10, 2, b'h']);
        assert!(matches!(
            messages[0].base.audience,
            ClassicGuestAudience::Multicast {
                scope: MulticastScope::All,
                ..
            }
        ));
    }

    #[test]
    fn clip_world_branch_encodes_trace() {
        let mut fix = fixture();
        let address = fix.host.record_at(0).expect("record");
        let start = write_vector(&mut fix.host, Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        let end = write_vector(&mut fix.host, Vec3 { x: 4.0, y: 5.0, z: 6.0 });
        let call = import_call(
            "clip",
            vec![
                GuestCallValue::Pointer(Some(address)),
                GuestCallValue::Pointer(Some(start)),
                GuestCallValue::Pointer(None),
                GuestCallValue::Pointer(None),
                GuestCallValue::Pointer(Some(end)),
                GuestCallValue::Int32(1),
            ],
        );
        let result = fix
            .services
            .invoke(&mut fix.host, &call)
            .expect("invoke")
            .expect("result");
        assert!(matches!(result, GuestCallResult::Value(_)));
    }

    #[test]
    fn clip_entity_branch_needs_seam_then_traces() {
        let mut fix = fixture();
        let address = fix.host.record_at(1).expect("record");
        let start = write_vector(&mut fix.host, ZERO);
        let end = write_vector(&mut fix.host, ZERO);
        let call = import_call(
            "clip",
            vec![
                GuestCallValue::Pointer(Some(address)),
                GuestCallValue::Pointer(Some(start)),
                GuestCallValue::Pointer(None),
                GuestCallValue::Pointer(None),
                GuestCallValue::Pointer(Some(end)),
                GuestCallValue::Int32(1),
            ],
        );
        assert!(error_message(fix.services.invoke(&mut fix.host, &call).unwrap_err()).contains("traceQ2Box"));
        fix.services.set_body_trace(Box::new(|_, _| Ok(FakeScene::miss())));
        let result = fix
            .services
            .invoke(&mut fix.host, &call)
            .expect("invoke")
            .expect("result");
        assert!(matches!(result, GuestCallResult::Value(_)));
    }

    #[test]
    fn link_metadata_reports_areas_and_caches_link() {
        let mut fix = fixture();
        let actor = owned(&fix, 2);
        let view = record(&fix.host, 2);
        fix.services.admit_entity(&view, &actor);
        fix.services.prepare_link(&mut fix.host, &actor, 2).expect("prepare");
        assert_eq!(fix.shared.borrow().collisions.len(), 1);
        let report = fix.services.link_metadata(&mut fix.host, &actor, 2).expect("link");
        assert_eq!((report.area, report.area2), (3, 0));
        let info = fix.services.entity_info(&mut fix.host, 2).expect("info");
        assert_eq!(info.clusters, Some(vec![3]));
        assert_eq!(info.first_cluster, 3);
        assert_eq!(info.headnode, 7);
    }

    #[test]
    fn sound_validates_and_enqueues() {
        let mut fix = fixture();
        let event = |volume: f32, origin: Option<Vec3>| RereleaseSoundEvent {
            entity_slot: None,
            origin,
            channel: 0,
            sound_index: 5,
            volume,
            attenuation: 1.0,
            time_offset: 0.0,
            audience: SoundAudience::World,
        };
        assert!(
            error_message(fix.services.sound(&mut fix.host, &event(2.0, None)).unwrap_err())
                .contains("sound arguments")
        );
        assert!(
            error_message(fix.services.sound(&mut fix.host, &event(1.0, None)).unwrap_err())
                .contains("neither entity nor origin")
        );
        let positioned = event(1.0, Some(Vec3 { x: 1.0, y: 0.0, z: 0.0 }));
        fix.services.sound(&mut fix.host, &positioned).expect("sound");
        let messages = fix.services.drain_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(&messages[0].base.bytes[..3], &[9, 4, 5]);
        assert!(!messages[0].base.reliable);
    }

    #[test]
    fn entity_state_numbers_slots_and_publishes() {
        let mut fix = fixture();
        fix.host.entity_count = 2;
        let actor = owned(&fix, 1);
        let view = record(&fix.host, 1);
        fix.services.admit_entity(&view, &actor);
        let state = fix.services.entity_state(&mut fix.host, 1).expect("state");
        assert_eq!(state.base.number, 1);
        let appearance = fix.services.model_appearance(&mut fix.host, 1).expect("appearance");
        assert_eq!(appearance.path, "");
        fix.services.publish_entities(&mut fix.host).expect("publish");
        assert!(fix
            .shared
            .borrow()
            .emitted
            .iter()
            .any(|event| event == "visibility false"));
        assert!(fix.shared.borrow().emitted.iter().any(|event| event == "model"));
    }

    #[test]
    fn restore_configstrings_roundtrip() {
        let mut fix = fixture();
        let mut values = HashMap::new();
        values.insert(63, "q2dm8".to_string());
        values.insert(70, "#bfg".to_string());
        fix.services.restore_configstrings(&values).expect("restore");
        let strings = RereleaseGuestServicesPort::configstrings(&fix.services);
        assert_eq!(strings.get(&63).map(String::as_str), Some("q2dm8"));
        assert_eq!(fix.services.weapon_model_list, vec!["weapon.md2", "bfg"]);
        let mut bad = HashMap::new();
        bad.insert(20000, "x".to_string());
        assert!(fix.services.restore_configstrings(&bad).is_err());
    }

    #[test]
    fn core_services_print_and_frame() {
        let mut fix = fixture();
        RereleaseGuestServicesPort::set_configstring(&mut fix.services, 9, "nine").expect("set");
        assert_eq!(RereleaseCoreServices::get_configstring(&fix.services, 9), "nine");
        RereleaseCoreServices::print(&mut fix.services, "core");
        assert!(fix.shared.borrow().prints.iter().any(|line| line == "core"));
        assert_eq!(RereleaseCoreServices::server_frame(&fix.services), 0);
        assert_eq!(RereleaseCoreServices::extension(&fix.services, "x", "game"), None);
        assert_eq!(
            RereleaseCoreServices::command_arguments(&fix.services),
            Vec::<String>::new()
        );
    }

    fn write_vector(host: &mut RereleaseQ2GuestHost, value: Vec3) -> GuestAddress {
        let address = host
            .memory
            .allocate(&qa_guest::core::contracts::GuestAllocationOptions::bytes(12))
            .expect("alloc");
        host.memory.write_f32(address, value.x).expect("x");
        let at = host.memory.offset(address, 4).expect("o4");
        host.memory.write_f32(at, value.y).expect("y");
        let at = host.memory.offset(address, 8).expect("o8");
        host.memory.write_f32(at, value.z).expect("z");
        address
    }
}
