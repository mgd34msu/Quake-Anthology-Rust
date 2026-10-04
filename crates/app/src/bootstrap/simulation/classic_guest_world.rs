//! Classic Quake II guest world facade.
//!
//! Port of donor `src/app/bootstrap/simulation/classic-guest-world.ts`
//! (`ClassicGuestWorld`, `ClassicGuestMap`, `ClassicGuestWorldOptions`,
//! `ClassicGuestRevisit`, `ClassicGuestClient`, `classicGuestUserCommand`).
//!
//! API 3 owns time, movement and private gameplay data; the facade only
//! sequences its exported calls. The donor's `async` loading twins take a
//! `nextFrame` pump here instead of a promise: the synchronous host calls
//! complete immediately, so each loading operation pumps once per host call
//! to preserve the yield points. Donor `RangeError`s fold into
//! [`ClassicQ2Error::Invalid`] with the donor messages.
//!
//! Input retirement adapts the Rust [`NativeInputBinding`]: the donor
//! binding observes the host itself and retires slots synchronously, while
//! the Rust binding reports retirements through dispatch calls, so
//! [`ClassicGuestWorld::bind_input`] wraps the caller's `retired` hook,
//! queues retirements, and [`ClassicGuestWorld`] processes them at the end
//! of each operation exactly like the donor's `finishInputRetirements`. The
//! [`ClassicGuestWorld::input_binding`] accessor exposes the binding to the
//! engine that drives dispatch.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_compat::q2::classic::host::ClientConnectOutcome;
use qa_compat::q2::classic::layout::{ClassicQ2Error, ClassicResult};
use qa_compat::q2::classic::pmove::EquipmentMovement;
use qa_compat::q2::classic::world_profile::classic_primary_world_profile;
use qa_compat::q2::native_input::{InputIdentity, InputServices, NativeInputBinding};
use qa_compat::q2::native_primary::NativePrimaryProfile;
use qa_core::identity::ActorId;
use qa_guest::core::contracts::{ContentDigest, GuestAddress, ModuleIdentity};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::runtime::windows::contracts::WindowsCapabilities;
use qa_net::q2_adapters::{Q2EntityState, Q2PlayerState, Q2UserCommand};

use super::classic_guest_services::{
    ClassicGuestMapServices, ClassicGuestMessage, ClassicGuestServices, ClassicGuestServicesOptions, GuestCommandLine,
    ModelAppearance, NativeInputViewState,
};
use super::classic_guest_source::{ClassicGuestSource, ClassicGuestSourceOptions, PreparedClassicGuest};
use crate::persistence::recipe::ExecutionImplementation;

/// Destination map for [`ClassicGuestWorld::travel`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicGuestMap {
    /// Map path.
    pub map: String,
    /// Entity lump.
    pub entities: String,
    /// Spawn point.
    pub spawn_point: String,
}

/// Construction options for [`ClassicGuestWorld::create`].
pub struct ClassicGuestWorldOptions {
    /// Prepared guest module.
    pub prepared: PreparedClassicGuest,
    /// Guest import services options.
    pub services: ClassicGuestServicesOptions,
    /// Windows capabilities for guest file access.
    pub capabilities: WindowsCapabilities,
    /// Instruction budget override.
    pub instruction_budget: Option<u64>,
}

/// Hub-level revisit: restore server state, then read the level file.
pub struct ClassicGuestRevisit {
    /// Saved level path.
    pub level_path: String,
    /// Server-state restoration over the transferred world.
    pub restore_server_state: Box<dyn FnMut(&mut ClassicGuestWorld)>,
}

/// Connection phase of a guest client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassicGuestClientPhase {
    /// Connected, not yet begun.
    Connected,
    /// Begun and playing.
    Active,
}

impl ClassicGuestClientPhase {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            ClassicGuestClientPhase::Connected => "connected",
            ClassicGuestClientPhase::Active => "active",
        }
    }
}

/// Connected guest client record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicGuestClient {
    /// Client slot.
    pub slot: u32,
    /// Connection phase.
    pub phase: ClassicGuestClientPhase,
    /// Current userinfo.
    pub userinfo: String,
}

/// Edict observation for a slot (donor `entityInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicEntityInfo {
    /// Bound actor, if any.
    pub actor: Option<ActorId>,
    /// Whether the edict is in use.
    pub active: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Area numbers.
    pub areas: (i32, i32),
    /// Visibility clusters (`None` when the edict is everywhere).
    pub clusters: Option<Vec<i32>>,
    /// First cluster.
    pub first_cluster: i32,
    /// Head node.
    pub headnode: i32,
    /// Owner slot, if the edict has an owner.
    pub owner_slot: Option<u32>,
}

/// Lifecycle phase of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorldPhase {
    Created,
    Initialized,
    Running,
    Transferred,
    Closed,
}

/// Shared caller retirement hook.
type RetiredHook = Rc<RefCell<Box<dyn FnMut(InputIdentity)>>>;

/// Queued input retirement with the caller hook that completes it.
struct PendingRetirement {
    identity: InputIdentity,
    retired: RetiredHook,
}

fn mem_i32(memory: &mut SparseGuestMemory, base: GuestAddress, offset: i64) -> ClassicResult<i32> {
    Ok(memory.read_i32(memory.offset(base, offset)?)?)
}

/// API 3 guest world: sequences the DLL's exported calls.
pub struct ClassicGuestWorld {
    source: Option<ClassicGuestSource>,
    services: Option<ClassicGuestServices>,
    command_context: Rc<RefCell<Option<GuestCommandLine>>>,
    clients: HashMap<u32, ClassicGuestClient>,
    input_binding: Option<NativeInputBinding>,
    pending_retirements: Rc<RefCell<Vec<PendingRetirement>>>,
    phase: WorldPhase,
    busy: bool,
}

impl ClassicGuestWorld {
    /// World edition tag.
    #[must_use]
    pub fn edition(&self) -> &'static str {
        "classic"
    }

    /// Whether ownership moved away or the world closed.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.phase == WorldPhase::Transferred || self.phase == WorldPhase::Closed
    }

    /// Bound module identity.
    pub fn module(&self) -> ClassicResult<ModuleIdentity> {
        let (source, _) = self.split()?;
        Ok(source.host.memory.module().clone())
    }

    /// Bound guest services, while this world owns them.
    #[must_use]
    pub fn services(&self) -> Option<&ClassicGuestServices> {
        self.services.as_ref()
    }

    /// Install the player-velocity writer/reader pair (donor ctor 712-715).
    pub fn set_player_velocity_writer(
        &mut self,
        write: super::classic_guest_services::PlayerVelocityWrite,
        read: super::classic_guest_services::PlayerVelocityRead,
    ) {
        self.services
            .as_mut()
            .expect("Classic guest services are installed")
            .set_player_velocity_writer(write, read);
    }

    /// Connected clients.
    pub fn clients(&self) -> ClassicResult<Vec<ClassicGuestClient>> {
        self.require_ownership()?;
        Ok(self.clients.values().cloned().collect())
    }

    /// Build a world over a prepared guest.
    pub fn create(options: ClassicGuestWorldOptions) -> ClassicResult<Self> {
        let command_context: Rc<RefCell<Option<GuestCommandLine>>> = Rc::new(RefCell::new(None));
        let declared = options
            .prepared
            .primary
            .as_ref()
            .and_then(|primary| match &primary.profile {
                NativePrimaryProfile::Classic { world, pickups, .. } => Some((world.digest.clone(), pickups.clone())),
                _ => None,
            });
        let artifact_digest = match &options.prepared.execution.implementation {
            ExecutionImplementation::Native { artifact, .. } => Some(artifact.digest.clone()),
            _ => None,
        };
        // The services profile is the digest-keyed builtin projection; the
        // declared native profile keeps donor precedence through its digest.
        // A declared non-builtin profile has no base-port converter
        // (canonical home: compat q2 classic world_profile); builtins
        // coincide with the declared profile for the same digest.
        let digest = declared.as_ref().map(|(digest, _)| digest.clone()).or(artifact_digest);
        let primary_world = digest.map(|value| classic_primary_world_profile(&ContentDigest::new("sha256", &value)));
        let primary_world = primary_world.flatten();
        let pickup_profile = declared.map(|(_, pickups)| pickups);
        let mut source = ClassicGuestSource::create(
            options.prepared,
            ClassicGuestSourceOptions {
                capabilities: options.capabilities,
                instruction_budget: options.instruction_budget,
            },
        )
        .map_err(|error| ClassicQ2Error::invalid(error.to_string()))?;
        let caller_command = options.services.command.clone();
        let mut services_options = options.services;
        services_options.primary_world = primary_world;
        services_options.pickup_profile = pickup_profile;
        services_options.command = {
            let context = command_context.clone();
            Rc::new(move || {
                context
                    .borrow()
                    .as_ref()
                    .map(|current| GuestCommandLine {
                        arguments: current.arguments.clone(),
                        args: current.args.clone(),
                    })
                    .unwrap_or_else(|| caller_command())
            })
        };
        let mut services = ClassicGuestServices::new(services_options)?;
        let image_base = source.image_base();
        if let Err(error) = services.bind_host(&mut source.host, Some(image_base)) {
            let _ = source.discard();
            return Err(error);
        }
        Ok(Self {
            source: Some(source),
            services: Some(services),
            command_context,
            clients: HashMap::new(),
            input_binding: None,
            pending_retirements: Rc::new(RefCell::new(Vec::new())),
            phase: WorldPhase::Created,
            busy: false,
        })
    }

    /// Bind native input retirement over caller-owned services.
    pub fn bind_input(&mut self, mut services: InputServices) {
        self.input_binding = None;
        let caller = std::mem::replace(&mut services.retired, Box::new(|_| {}));
        let retired: RetiredHook = Rc::new(RefCell::new(caller));
        let pending = self.pending_retirements.clone();
        services.retired = Box::new(move |identity| {
            pending.borrow_mut().push(PendingRetirement {
                identity,
                retired: retired.clone(),
            });
        });
        self.input_binding = Some(NativeInputBinding::new(services));
    }

    /// Bound input binding, for the engine driving dispatch.
    #[must_use]
    pub fn input_binding(&mut self) -> Option<&mut NativeInputBinding> {
        self.input_binding.as_mut()
    }

    fn require_ownership(&self) -> ClassicResult<()> {
        if self.phase == WorldPhase::Transferred {
            return Err(ClassicQ2Error::invalid("Native world ownership was transferred"));
        }
        Ok(())
    }

    fn split(&self) -> ClassicResult<(&ClassicGuestSource, &ClassicGuestServices)> {
        self.require_ownership()?;
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| ClassicQ2Error::invalid("Native world has no retained source"))?;
        let services = self
            .services
            .as_ref()
            .ok_or_else(|| ClassicQ2Error::invalid("Native world has no retained services"))?;
        Ok((source, services))
    }

    fn split_mut(&mut self) -> ClassicResult<(&mut ClassicGuestSource, &mut ClassicGuestServices)> {
        self.require_ownership()?;
        let source = self
            .source
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("Native world has no retained source"))?;
        let services = self
            .services
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("Native world has no retained services"))?;
        Ok((source, services))
    }

    fn operation<T>(&mut self, run: impl FnOnce(&mut Self) -> ClassicResult<T>) -> ClassicResult<T> {
        self.require_ownership()?;
        if self.phase == WorldPhase::Closed {
            return Err(ClassicQ2Error::invalid("Native world is closed"));
        }
        if self.busy {
            return Err(ClassicQ2Error::invalid(
                "External native world operation is already active",
            ));
        }
        self.busy = true;
        let result = run(self);
        self.busy = false;
        let value = result?;
        self.finish_input_retirements()?;
        Ok(value)
    }

    fn require_running(&self) -> ClassicResult<()> {
        if self.phase != WorldPhase::Running {
            return Err(ClassicQ2Error::invalid("Native world has not spawned"));
        }
        Ok(())
    }

    fn require_client(&self, slot: u32, phase: Option<ClassicGuestClientPhase>) -> ClassicResult<ClassicGuestClient> {
        let client = self.clients.get(&slot).cloned().ok_or_else(|| {
            ClassicQ2Error::invalid(format!(
                "Native client {slot} is not {}",
                match phase {
                    Some(ClassicGuestClientPhase::Active) => "active",
                    _ => "connected",
                }
            ))
        })?;
        if let Some(phase) = phase {
            if client.phase != phase {
                let want = if phase == ClassicGuestClientPhase::Active {
                    "active"
                } else {
                    "connected"
                };
                return Err(ClassicQ2Error::invalid(format!("Native client {slot} is not {want}")));
            }
        }
        Ok(client)
    }

    fn process_retirement(&mut self, pending: PendingRetirement) -> ClassicResult<()> {
        let slot = pending.identity.slot;
        {
            let (source, _) = self.split_mut()?;
            let host = &mut source.host;
            let edicts = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
            edicts.retire_input_client(slot);
            let record = edicts.at(&mut host.memory, slot)?;
            let active = mem_i32(&mut host.memory, record.address, 88)? != 0;
            if active {
                host.client_event("ClientDisconnect", slot)?;
            }
            let edicts = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
            edicts.release_client(&mut host.memory, &mut host.registry, slot)?;
            edicts.finish_input_retirement(slot);
        }
        self.clients.remove(&slot);
        let mut retired = pending.retired.borrow_mut();
        retired(pending.identity);
        Ok(())
    }

    fn finish_input_retirements(&mut self) -> ClassicResult<()> {
        let queued: Vec<PendingRetirement> = self.pending_retirements.borrow_mut().drain(..).collect();
        for pending in queued {
            self.process_retirement(pending)?;
        }
        Ok(())
    }

    fn discard_input_retirements(&mut self) {
        let queued: Vec<PendingRetirement> = self.pending_retirements.borrow_mut().drain(..).collect();
        for pending in queued {
            self.clients.remove(&pending.identity.slot);
            let mut retired = pending.retired.borrow_mut();
            retired(pending.identity);
        }
    }
}

impl ClassicGuestWorld {
    /// Run the DLL entry point.
    pub fn init(&mut self) -> ClassicResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Created {
                return Err(ClassicQ2Error::invalid("Native Init requires a fresh world"));
            }
            {
                let (source, _) = world.split_mut()?;
                source
                    .init()
                    .map_err(|error| ClassicQ2Error::invalid(error.to_string()))?;
            }
            world.phase = WorldPhase::Initialized;
            Ok(())
        })
    }

    /// Run the DLL entry point, pumping `next_frame` while loading.
    pub fn init_loading(&mut self, next_frame: &mut dyn FnMut()) -> ClassicResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Created {
                return Err(ClassicQ2Error::invalid("Native Init requires a fresh world"));
            }
            {
                let (source, _) = world.split_mut()?;
                source
                    .init_loading(next_frame)
                    .map_err(|error| ClassicQ2Error::invalid(error.to_string()))?;
            }
            world.phase = WorldPhase::Initialized;
            Ok(())
        })
    }

    /// Spawn entities and settle the first two frames.
    pub fn spawn(&mut self, map: &str, entities: &str, spawn_point: &str) -> ClassicResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Initialized {
                return Err(ClassicQ2Error::invalid(
                    "Native SpawnEntities requires an initialized candidate",
                ));
            }
            {
                let (source, services) = world.split_mut()?;
                source.host.spawn_entities(map, entities, spawn_point)?;
                source.host.run_frame()?;
                source.host.run_frame()?;
                services.complete_spawn();
            }
            world.phase = WorldPhase::Running;
            Ok(())
        })
    }

    /// Spawn entities, pumping `next_frame` while loading.
    pub fn spawn_loading(
        &mut self,
        map: &str,
        entities: &str,
        next_frame: &mut dyn FnMut(),
        spawn_point: &str,
    ) -> ClassicResult<()> {
        self.require_ownership()?;
        if self.phase != WorldPhase::Initialized || self.busy {
            return Err(ClassicQ2Error::invalid(
                "Native loading requires an idle initialized world",
            ));
        }
        self.busy = true;
        let result = (|| -> ClassicResult<()> {
            {
                let (source, services) = self.split_mut()?;
                source.host.spawn_entities(map, entities, spawn_point)?;
                next_frame();
                source.host.call("RunFrame", &[])?;
                next_frame();
                source.host.call("RunFrame", &[])?;
                next_frame();
                services.complete_spawn();
            }
            self.phase = WorldPhase::Running;
            Ok(())
        })();
        self.busy = false;
        result
    }

    /// Write the travel level file.
    pub fn write_travel_level(&mut self, path: &str) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let (source, services) = world.split_mut()?;
            let max_clients = services.max_clients();
            source.host.write_travel_level(path, max_clients)?;
            Ok(())
        })
    }

    /// Write the travel level file, pumping `next_frame` while loading.
    pub fn write_travel_level_loading(&mut self, path: &str, next_frame: &mut dyn FnMut()) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let (source, services) = world.split_mut()?;
            let max_clients = services.max_clients();
            source.host.write_travel_level(path, max_clients)?;
            next_frame();
            Ok(())
        })
    }

    /// Irreversible ownership transfer after the caller staged the destination map.
    pub fn travel(
        &mut self,
        map: ClassicGuestMap,
        binding: ClassicGuestMapServices,
        revisit: Option<ClassicGuestRevisit>,
    ) -> ClassicResult<ClassicGuestWorld> {
        self.operation(|world| {
            world.require_running()?;
            ClassicGuestServices::validate_map(&binding)?;
            {
                let (_, services) = world.split()?;
                if revisit.is_some() && services.cvar_value("deathmatch") != 0.0 {
                    return Err(ClassicQ2Error::invalid(
                        "Original API 3 deathmatch does not restore hub levels",
                    ));
                }
            }
            world.finish_input_retirements()?;
            let clients: Vec<ClassicGuestClient> = world.clients.values().cloned().collect();
            let mut next = ClassicGuestWorld {
                source: world.source.take(),
                services: world.services.take(),
                command_context: world.command_context.clone(),
                clients: HashMap::new(),
                input_binding: None,
                pending_retirements: Rc::new(RefCell::new(Vec::new())),
                phase: WorldPhase::Initialized,
                busy: false,
            };
            world.input_binding = None;
            world.phase = WorldPhase::Transferred;
            world.clients.clear();
            let settled = next.travel_spawn(map, binding, revisit, &clients);
            match settled {
                Ok(()) => {
                    next.phase = WorldPhase::Running;
                    Ok(next)
                }
                Err(error) => {
                    if let Err(cleanup) = next.close() {
                        next.phase = WorldPhase::Closed;
                        next.clients.clear();
                        return Err(ClassicQ2Error::invalid(format!(
                            "Native map travel and retained module cleanup failed: {error} + {cleanup}"
                        )));
                    }
                    next.phase = WorldPhase::Closed;
                    next.clients.clear();
                    Err(error)
                }
            }
        })
    }

    /// Irreversible ownership transfer, pumping `next_frame` while loading.
    pub fn travel_loading(
        &mut self,
        map: ClassicGuestMap,
        binding: ClassicGuestMapServices,
        next_frame: &mut dyn FnMut(),
        revisit: Option<ClassicGuestRevisit>,
    ) -> ClassicResult<ClassicGuestWorld> {
        self.operation(|world| {
            world.require_running()?;
            ClassicGuestServices::validate_map(&binding)?;
            {
                let (_, services) = world.split()?;
                if revisit.is_some() && services.cvar_value("deathmatch") != 0.0 {
                    return Err(ClassicQ2Error::invalid(
                        "Original API 3 deathmatch does not restore hub levels",
                    ));
                }
            }
            world.finish_input_retirements()?;
            let clients: Vec<ClassicGuestClient> = world.clients.values().cloned().collect();
            let mut next = ClassicGuestWorld {
                source: world.source.take(),
                services: world.services.take(),
                command_context: world.command_context.clone(),
                clients: HashMap::new(),
                input_binding: None,
                pending_retirements: Rc::new(RefCell::new(Vec::new())),
                phase: WorldPhase::Initialized,
                busy: false,
            };
            world.input_binding = None;
            world.phase = WorldPhase::Transferred;
            world.clients.clear();
            let settled = next.travel_spawn_loading(map, binding, next_frame, revisit, &clients);
            match settled {
                Ok(()) => {
                    next.phase = WorldPhase::Running;
                    Ok(next)
                }
                Err(error) => {
                    if let Err(cleanup) = next.close() {
                        next.phase = WorldPhase::Closed;
                        next.clients.clear();
                        return Err(ClassicQ2Error::invalid(format!(
                            "Native map travel and retained module cleanup failed: {error} + {cleanup}"
                        )));
                    }
                    next.phase = WorldPhase::Closed;
                    next.clients.clear();
                    Err(error)
                }
            }
        })
    }

    fn rebound_command(&self, binding: ClassicGuestMapServices) -> ClassicGuestMapServices {
        let context = self.command_context.clone();
        let caller = binding.command.clone();
        let mut rebound = binding;
        rebound.command = Rc::new(move || {
            context
                .borrow()
                .as_ref()
                .map(|current| GuestCommandLine {
                    arguments: current.arguments.clone(),
                    args: current.args.clone(),
                })
                .unwrap_or_else(|| caller())
        });
        rebound
    }

    fn retain_clients(&mut self, clients: &[ClassicGuestClient]) -> ClassicResult<()> {
        for client in clients {
            {
                let (source, _) = self.split_mut()?;
                let host = &mut source.host;
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                edicts.retain_client(&mut host.memory, &mut host.registry, client.slot)?;
            }
            self.clients.insert(
                client.slot,
                ClassicGuestClient {
                    phase: ClassicGuestClientPhase::Connected,
                    ..client.clone()
                },
            );
        }
        Ok(())
    }

    fn travel_spawn(
        &mut self,
        map: ClassicGuestMap,
        binding: ClassicGuestMapServices,
        mut revisit: Option<ClassicGuestRevisit>,
        clients: &[ClassicGuestClient],
    ) -> ClassicResult<()> {
        let provider = self.split()?.0.host.provider.clone();
        self.split_mut()?.1.release_provider_actors(&provider);
        let rebound = self.rebound_command(binding);
        self.split_mut()?.1.rebind_world(rebound)?;
        let (source, _) = self.split_mut()?;
        source.host.rebind_world()?;
        source.host.spawn_entities(&map.map, &map.entities, &map.spawn_point)?;
        source.host.run_frame()?;
        source.host.run_frame()?;
        if let Some(revisit) = revisit.as_mut() {
            self.split_mut()?.1.drain_messages();
            (revisit.restore_server_state)(self);
            self.split_mut()?.0.host.save("ReadLevel", &revisit.level_path, false)?;
            for _ in 0..100 {
                self.split_mut()?.0.host.run_frame()?;
            }
        }
        self.retain_clients(clients)?;
        self.split_mut()?.1.complete_spawn();
        Ok(())
    }

    fn travel_spawn_loading(
        &mut self,
        map: ClassicGuestMap,
        binding: ClassicGuestMapServices,
        next_frame: &mut dyn FnMut(),
        mut revisit: Option<ClassicGuestRevisit>,
        clients: &[ClassicGuestClient],
    ) -> ClassicResult<()> {
        let provider = self.split()?.0.host.provider.clone();
        self.split_mut()?.1.release_provider_actors(&provider);
        let rebound = self.rebound_command(binding);
        self.split_mut()?.1.rebind_world(rebound)?;
        let (source, _) = self.split_mut()?;
        source.host.rebind_world()?;
        source.host.spawn_entities(&map.map, &map.entities, &map.spawn_point)?;
        next_frame();
        source.host.call("RunFrame", &[])?;
        next_frame();
        source.host.call("RunFrame", &[])?;
        next_frame();
        if let Some(revisit) = revisit.as_mut() {
            self.split_mut()?.1.drain_messages();
            (revisit.restore_server_state)(self);
            self.split_mut()?.0.host.save("ReadLevel", &revisit.level_path, false)?;
            next_frame();
            for _ in 0..100 {
                self.split_mut()?.0.host.call("RunFrame", &[])?;
                next_frame();
            }
        }
        self.retain_clients(clients)?;
        self.split_mut()?.1.complete_spawn();
        Ok(())
    }

    /// Connect a client slot.
    pub fn connect(&mut self, slot: u32, userinfo: &str) -> ClassicResult<ClientConnectOutcome> {
        self.operation(|world| {
            world.require_running()?;
            if world.clients.contains_key(&slot) {
                return Err(ClassicQ2Error::invalid("Native client slot is unavailable"));
            }
            let max_clients = world.split()?.1.max_clients();
            if slot < 1 || slot > max_clients {
                return Err(ClassicQ2Error::invalid("Native client slot is unavailable"));
            }
            let result = world.split_mut()?.0.host.client_connect(slot, userinfo)?;
            if result.allowed {
                {
                    let (source, _) = world.split_mut()?;
                    let host = &mut source.host;
                    let edicts = host
                        .edicts
                        .as_mut()
                        .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                    edicts.retain_client(&mut host.memory, &mut host.registry, slot)?;
                }
                world.clients.insert(
                    slot,
                    ClassicGuestClient {
                        slot,
                        phase: ClassicGuestClientPhase::Connected,
                        userinfo: result.userinfo.clone(),
                    },
                );
            }
            Ok(result)
        })
    }

    /// Begin a connected client.
    pub fn begin(&mut self, slot: u32) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let client = world.require_client(slot, Some(ClassicGuestClientPhase::Connected))?;
            world.split_mut()?.0.host.client_event("ClientBegin", slot)?;
            world.clients.insert(
                slot,
                ClassicGuestClient {
                    phase: ClassicGuestClientPhase::Active,
                    ..client
                },
            );
            Ok(())
        })
    }

    /// Connect and immediately begin a client.
    pub fn admit(&mut self, slot: u32, userinfo: &str) -> ClassicResult<ClientConnectOutcome> {
        let result = self.connect(slot, userinfo)?;
        if result.allowed {
            self.begin(slot)?;
        }
        Ok(result)
    }

    /// Disconnect a client.
    pub fn disconnect(&mut self, slot: u32) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, None)?;
            {
                let (source, _) = world.split_mut()?;
                source.host.client_event("ClientDisconnect", slot)?;
                let host = &mut source.host;
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                edicts.release_client(&mut host.memory, &mut host.registry, slot)?;
            }
            world.clients.remove(&slot);
            Ok(())
        })
    }

    /// Push a userinfo change through the DLL.
    pub fn userinfo(&mut self, slot: u32, value: &str) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let client = world.require_client(slot, None)?;
            let userinfo = world.split_mut()?.0.host.client_userinfo_changed(slot, value)?;
            world.clients.insert(slot, ClassicGuestClient { userinfo, ..client });
            Ok(())
        })
    }

    /// Replace stored userinfo without calling the DLL.
    pub fn set_userinfo_storage(&mut self, slot: u32, value: &str) -> ClassicResult<()> {
        self.require_running()?;
        let client = self.require_client(slot, None)?;
        if value.len() >= 512 || value.contains('\0') {
            return Err(ClassicQ2Error::invalid(
                "Classic userinfo exceeds its source string limits",
            ));
        }
        self.clients.insert(
            slot,
            ClassicGuestClient {
                userinfo: value.to_string(),
                ..client
            },
        );
        Ok(())
    }

    /// Run a client command through the DLL.
    pub fn command(&mut self, slot: u32, arguments: &[String], args: &str) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, Some(ClassicGuestClientPhase::Active))?;
            *world.command_context.borrow_mut() = Some(GuestCommandLine {
                arguments: arguments.to_vec(),
                args: args.to_string(),
            });
            let result = world.split_mut()?.0.host.client_event("ClientCommand", slot);
            *world.command_context.borrow_mut() = None;
            result?;
            Ok(())
        })
    }

    /// Run a server command through the DLL.
    pub fn server_command(&mut self, arguments: &[String], args: &str) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            *world.command_context.borrow_mut() = Some(GuestCommandLine {
                arguments: arguments.to_vec(),
                args: args.to_string(),
            });
            let result = world.split_mut()?.0.host.call("ServerCommand", &[]);
            *world.command_context.borrow_mut() = None;
            result?;
            Ok(())
        })
    }

    /// Run client input through the DLL.
    pub fn think(
        &mut self,
        slot: u32,
        command: Q2UserCommand,
        movement: Option<EquipmentMovement>,
    ) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, Some(ClassicGuestClientPhase::Active))?;
            let bytes = classic_guest_user_command(&command);
            let (source, services) = world.split_mut()?;
            services.with_player_movement(&mut source.host, movement, |host| host.client_think(slot, &bytes))?;
            Ok(())
        })
    }

    /// Run one 100ms server frame.
    pub fn frame(&mut self, milliseconds: u32) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            if milliseconds != 100 {
                return Err(ClassicQ2Error::invalid(
                    "API 3 RunFrame requires the source 100ms cadence",
                ));
            }
            let (source, services) = world.split_mut()?;
            source.host.run_frame()?;
            services.publish_entities(&mut source.host)?;
            Ok(())
        })
    }

    /// Bound actor for a slot.
    pub fn actor(&mut self, slot: u32) -> ClassicResult<Option<ActorId>> {
        self.require_running()?;
        let (source, _) = self.split_mut()?;
        let host = &mut source.host;
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        let record = edicts.at(&mut host.memory, slot)?;
        let current = edicts.current(&mut host.memory, &host.registry, &record)?;
        Ok(current.map(|owned| owned.id().clone()))
    }

    /// Edict observation for a slot.
    pub fn entity_info(&mut self, slot: u32) -> ClassicResult<ClassicEntityInfo> {
        self.require_running()?;
        let (source, _) = self.split_mut()?;
        let host = &mut source.host;
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        let record = edicts.at(&mut host.memory, slot)?;
        let count = mem_i32(&mut host.memory, record.address, 104)?;
        if !(-1..=16).contains(&count) {
            return Err(ClassicQ2Error::invalid(
                "API 3 edict has an invalid visibility cluster count",
            ));
        }
        let clusters = if count == -1 {
            None
        } else {
            let mut list = Vec::with_capacity(count as usize);
            for index in 0..count {
                list.push(mem_i32(&mut host.memory, record.address, 108 + i64::from(index) * 4)?);
            }
            Some(list)
        };
        let owner = host.memory.read_pointer(host.memory.offset(record.address, 256)?)?;
        let owner_slot = match owner {
            None => None,
            Some(address) => {
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                Some(edicts.record_from_pointer(&mut host.memory, address)?.slot)
            }
        };
        let host = &mut source.host;
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        let current = edicts.current(&mut host.memory, &host.registry, &record)?;
        Ok(ClassicEntityInfo {
            actor: current.map(|owned| owned.id().clone()),
            active: mem_i32(&mut host.memory, record.address, 88)? != 0,
            server_flags: mem_i32(&mut host.memory, record.address, 184)?,
            areas: (
                mem_i32(&mut host.memory, record.address, 176)?,
                mem_i32(&mut host.memory, record.address, 180)?,
            ),
            clusters,
            first_cluster: mem_i32(&mut host.memory, record.address, 108)?,
            headnode: mem_i32(&mut host.memory, record.address, 172)?,
            owner_slot,
        })
    }

    /// Entity states for every visible in-use edict.
    pub fn entity_states(&mut self) -> ClassicResult<Vec<Q2EntityState>> {
        self.require_running()?;
        let count = {
            let (source, _) = self.split_mut()?;
            let host = &mut source.host;
            let edicts = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
            edicts.descriptor(&mut host.memory)?.count as u32
        };
        let mut states = Vec::new();
        for slot in 1..count {
            let active = {
                let (source, _) = self.split_mut()?;
                let host = &mut source.host;
                let edicts = host
                    .edicts
                    .as_mut()
                    .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
                let record = edicts.at(&mut host.memory, slot)?;
                let active = mem_i32(&mut host.memory, record.address, 88)? != 0;
                let server_flags = mem_i32(&mut host.memory, record.address, 184)?;
                if active && server_flags & 1 == 0 {
                    let number = mem_i32(&mut host.memory, record.address, 0)?;
                    if number != slot as i32 {
                        let at = host.memory.offset(record.address, 0)?;
                        host.memory.write_i32(at, slot as i32)?;
                    }
                }
                active && server_flags & 1 == 0
            };
            if active {
                let (source, _) = self.split_mut()?;
                states.push(ClassicGuestServices::entity_state(&mut source.host, slot)?);
            }
        }
        Ok(states)
    }

    /// Entity state for one slot.
    pub fn entity_state(&mut self, slot: u32) -> ClassicResult<Q2EntityState> {
        self.require_running()?;
        let (source, _) = self.split_mut()?;
        ClassicGuestServices::entity_state(&mut source.host, slot)
    }

    /// Player ping for a slot.
    pub fn player_ping(&mut self, slot: u32) -> ClassicResult<i32> {
        self.require_running()?;
        let (source, _) = self.split_mut()?;
        let host = &mut source.host;
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
        let client = edicts
            .client_prefix_address(&mut host.memory, slot)?
            .ok_or_else(|| ClassicQ2Error::invalid("Native slot has no public client prefix"))?;
        mem_i32(&mut host.memory, client, 184)
    }

    /// Set the player ping for a slot.
    pub fn set_player_ping(&mut self, slot: u32, ping: i32) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, None)?;
            let (source, _) = world.split_mut()?;
            let host = &mut source.host;
            let edicts = host
                .edicts
                .as_mut()
                .ok_or_else(|| ClassicQ2Error::invalid("API 3 services have no guest host"))?;
            edicts.set_client_ping(&mut host.memory, slot, ping)?;
            Ok(())
        })
    }

    /// Player state for one slot.
    pub fn player_state(&mut self, slot: u32) -> ClassicResult<Q2PlayerState> {
        self.require_running()?;
        let (source, _) = self.split_mut()?;
        ClassicGuestServices::player_state(&mut source.host, slot)
    }

    /// Player view for a client actor (donor `services.playerView`).
    pub fn player_view(&mut self, slot: u32, actor: &ActorId) -> ClassicResult<NativeInputViewState> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.player_view(&mut source.host, slot, actor)
    }

    /// Whether a source inventory profile is known (donor `services.hasSourceInventory`).
    #[must_use]
    pub fn has_source_inventory(&self) -> bool {
        self.services
            .as_ref()
            .is_some_and(super::classic_guest_services::ClassicGuestServices::has_source_inventory)
    }

    /// Model appearance for one slot.
    pub fn model_appearance(&mut self, slot: u32) -> ClassicResult<ModelAppearance> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.model_appearance(&mut source.host, slot)
    }

    /// Drain queued guest messages.
    pub fn raw_messages(&mut self) -> ClassicResult<Vec<ClassicGuestMessage>> {
        Ok(self.split_mut()?.1.drain_messages())
    }

    /// Current configstrings.
    pub fn configstrings(&self) -> ClassicResult<HashMap<i32, String>> {
        Ok(self.split()?.1.configstrings())
    }

    /// Restore engine configstring metadata before the DLL reads its level file.
    pub fn restore_configstrings(&mut self, values: &HashMap<i32, String>) -> ClassicResult<()> {
        if self.phase != WorldPhase::Initialized {
            return Err(ClassicQ2Error::invalid(
                "Native configstring restoration requires the initialized candidate",
            ));
        }
        let stale: Vec<i32> = self
            .split()?
            .1
            .configstrings()
            .keys()
            .copied()
            .filter(|index| !values.contains_key(index))
            .collect();
        for index in stale {
            let (source, services) = self.split_mut()?;
            services.set_configstring(&mut source.host, index, "")?;
        }
        for (index, value) in values {
            let (source, services) = self.split_mut()?;
            services.set_configstring(&mut source.host, *index, value)?;
        }
        Ok(())
    }

    /// Write original game and level files.
    pub fn write_original(&mut self, game_path: &str, level_path: &str, autosave: bool) -> ClassicResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let (source, _) = world.split_mut()?;
            source.host.save("WriteGame", game_path, autosave)?;
            source.host.save("WriteLevel", level_path, false)?;
            Ok(())
        })
    }

    /// Restore original save files over a fresh initialized candidate.
    pub fn restore_original(
        &mut self,
        game_path: &str,
        level_path: &str,
        map: &ClassicGuestMap,
        restore_server_state: impl FnMut(),
    ) -> ClassicResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Initialized {
                return Err(ClassicQ2Error::invalid(
                    "Original API 3 import requires a fresh initialized candidate",
                ));
            }
            let mut restore_server_state = restore_server_state;
            {
                let (source, services) = world.split_mut()?;
                source.host.save("ReadGame", game_path, false)?;
                source.host.spawn_entities(&map.map, &map.entities, &map.spawn_point)?;
                source.host.run_frame()?;
                source.host.run_frame()?;
                services.drain_messages();
                restore_server_state();
                source.host.save("ReadLevel", level_path, false)?;
                services.complete_spawn();
            }
            world.phase = WorldPhase::Running;
            Ok(())
        })
    }

    /// Restore original save files, pumping `next_frame` while loading.
    pub fn restore_original_loading(
        &mut self,
        game_path: &str,
        level_path: &str,
        map: &ClassicGuestMap,
        restore_server_state: impl FnMut(),
        next_frame: &mut dyn FnMut(),
    ) -> ClassicResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Initialized {
                return Err(ClassicQ2Error::invalid(
                    "Original API 3 import requires a fresh initialized candidate",
                ));
            }
            let mut restore_server_state = restore_server_state;
            {
                let (source, services) = world.split_mut()?;
                source.host.save("ReadGame", game_path, false)?;
                next_frame();
                source.host.spawn_entities(&map.map, &map.entities, &map.spawn_point)?;
                next_frame();
                source.host.call("RunFrame", &[])?;
                next_frame();
                source.host.call("RunFrame", &[])?;
                next_frame();
                services.drain_messages();
                restore_server_state();
                source.host.save("ReadLevel", level_path, false)?;
                next_frame();
                services.complete_spawn();
            }
            world.phase = WorldPhase::Running;
            Ok(())
        })
    }

    /// Shut the DLL down and release the world.
    pub fn close(&mut self) -> ClassicResult<()> {
        if self.is_retired() {
            return Ok(());
        }
        self.input_binding = None;
        self.operation(|world| {
            world.discard_input_retirements();
            let result = match world.split_mut() {
                Ok((source, _)) => source
                    .close()
                    .map_err(|error| ClassicQ2Error::invalid(error.to_string())),
                Err(error) => Err(error),
            };
            world.phase = WorldPhase::Closed;
            world.clients.clear();
            result
        })
    }

    /// Discard the world without running DLL shutdown.
    pub fn discard(&mut self) -> ClassicResult<()> {
        if self.is_retired() {
            return Ok(());
        }
        self.input_binding = None;
        self.operation(|world| {
            world.discard_input_retirements();
            let result = match world.split_mut() {
                Ok((source, _)) => source
                    .discard()
                    .map_err(|error| ClassicQ2Error::invalid(error.to_string())),
                Err(error) => Err(error),
            };
            world.phase = WorldPhase::Closed;
            world.clients.clear();
            result
        })
    }
}

/// Encode a user command into the 16 wire bytes the DLL reads.
#[must_use]
pub fn classic_guest_user_command(command: &Q2UserCommand) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[0] = command.milliseconds;
    bytes[1] = command.buttons;
    for (index, value) in command.angle_shorts.iter().enumerate() {
        bytes[2 + index * 2..4 + index * 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[8..10].copy_from_slice(&command.forward_move.to_le_bytes());
    bytes[10..12].copy_from_slice(&command.side_move.to_le_bytes());
    bytes[12..14].copy_from_slice(&command.up_move.to_le_bytes());
    bytes[14] = command.impulse;
    bytes[15] = command.light_level;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_compat::q2::native_input::SyntheticInputTable;
    use qa_compat::q2::native_primary_weapons::NativeActorId;

    fn hollow_world(phase: WorldPhase) -> ClassicGuestWorld {
        ClassicGuestWorld {
            source: None,
            services: None,
            command_context: Rc::new(RefCell::new(None)),
            clients: HashMap::new(),
            input_binding: None,
            pending_retirements: Rc::new(RefCell::new(Vec::new())),
            phase,
            busy: false,
        }
    }

    fn connected_client(slot: u32) -> ClassicGuestClient {
        ClassicGuestClient {
            slot,
            phase: ClassicGuestClientPhase::Connected,
            userinfo: "name Hollow".to_string(),
        }
    }

    fn test_services(retired: Rc<RefCell<Vec<InputIdentity>>>) -> InputServices {
        InputServices {
            applications_active: false,
            identity: Box::new(|slot| {
                Some(InputIdentity {
                    actor: NativeActorId { slot, generation: 0 },
                    slot,
                })
            }),
            live: Box::new(|_| false),
            accepted: Box::new(|_| None),
            frame_tick: Box::new(|| 0),
            retired: Box::new(move |identity| retired.borrow_mut().push(identity)),
            original_command: None,
            client_outputs: None,
            body_bounds: None,
            begin_application: Box::new(|_| 0),
            finish_application: Box::new(|_, _| {}),
            movement: None,
        }
    }

    fn invalid_message(error: ClassicQ2Error) -> String {
        match error {
            ClassicQ2Error::Invalid(message) => message,
            other => panic!("expected invalid error, got {other:?}"),
        }
    }

    #[test]
    fn user_command_encodes_wire_bytes() {
        let bytes = classic_guest_user_command(&Q2UserCommand {
            milliseconds: 50,
            buttons: 3,
            angle_shorts: [0x0102, -2, 0x7fff],
            forward_move: -300,
            side_move: 301,
            up_move: 0,
            impulse: 7,
            light_level: 9,
        });
        assert_eq!(
            bytes,
            [50, 3, 0x02, 0x01, 0xfe, 0xff, 0xff, 0x7f, 0xd4, 0xfe, 0x2d, 0x01, 0, 0, 7, 9]
        );
    }

    #[test]
    fn edition_and_retirement_flags() {
        let world = hollow_world(WorldPhase::Created);
        assert_eq!(world.edition(), "classic");
        assert!(!world.is_retired());
        assert!(hollow_world(WorldPhase::Transferred).is_retired());
        assert!(hollow_world(WorldPhase::Closed).is_retired());
        assert!(!hollow_world(WorldPhase::Running).is_retired());
    }

    #[test]
    fn transferred_world_blocks_access() {
        let mut world = hollow_world(WorldPhase::Transferred);
        assert!(invalid_message(world.clients().unwrap_err()).contains("transferred"));
        assert!(invalid_message(world.module().unwrap_err()).contains("transferred"));
        assert!(invalid_message(world.init().unwrap_err()).contains("transferred"));
    }

    #[test]
    fn lifecycle_guards_reject_wrong_phase() {
        let mut world = hollow_world(WorldPhase::Initialized);
        assert!(invalid_message(world.init().unwrap_err()).contains("fresh world"));
        let mut world = hollow_world(WorldPhase::Created);
        assert!(invalid_message(world.spawn("m", "e", "").unwrap_err()).contains("initialized candidate"));
        assert!(
            invalid_message(world.spawn_loading("m", "e", &mut || {}, "").unwrap_err()).contains("idle initialized")
        );
        assert!(
            invalid_message(world.restore_configstrings(&HashMap::new()).unwrap_err())
                .contains("initialized candidate")
        );
        let map = ClassicGuestMap {
            map: "m".to_string(),
            entities: "e".to_string(),
            spawn_point: String::new(),
        };
        assert!(
            invalid_message(world.restore_original("g", "l", &map, || {}).unwrap_err()).contains("fresh initialized")
        );
    }

    #[test]
    fn running_guards_reject_unspawned_world() {
        let mut world = hollow_world(WorldPhase::Created);
        for result in [
            world.actor(1).map(|_| ()),
            world.entity_info(1).map(|_| ()),
            world.entity_states().map(|_| ()),
            world.entity_state(1).map(|_| ()),
            world.player_ping(1).map(|_| ()),
            world.player_state(1).map(|_| ()),
            world.model_appearance(1).map(|_| ()),
        ] {
            assert!(invalid_message(result.unwrap_err()).contains("has not spawned"));
        }
    }

    #[test]
    fn frame_rejects_foreign_cadence_and_reentrancy() {
        let mut world = hollow_world(WorldPhase::Running);
        assert!(invalid_message(world.frame(50).unwrap_err()).contains("100ms cadence"));
        world.busy = true;
        assert!(invalid_message(world.frame(100).unwrap_err()).contains("already active"));
    }

    #[test]
    fn client_guards_reject_unknown_slots() {
        let mut world = hollow_world(WorldPhase::Running);
        assert!(invalid_message(world.begin(1).unwrap_err()).contains("not connected"));
        assert!(invalid_message(world.disconnect(1).unwrap_err()).contains("not connected"));
        assert!(invalid_message(world.userinfo(1, "x").unwrap_err()).contains("not connected"));
        assert!(invalid_message(world.set_player_ping(1, 0).unwrap_err()).contains("not connected"));
        assert!(invalid_message(world.command(1, &[], "").unwrap_err()).contains("not active"));
        assert!(invalid_message(world.think(1, Q2UserCommand::default(), None).unwrap_err()).contains("not active"));
    }

    #[test]
    fn userinfo_storage_validates_limits() {
        let mut world = hollow_world(WorldPhase::Running);
        world.clients.insert(1, connected_client(1));
        assert!(world.set_userinfo_storage(1, "name Solid").is_ok());
        assert_eq!(world.clients.get(&1).unwrap().userinfo, "name Solid");
        assert!(
            invalid_message(world.set_userinfo_storage(1, &"x".repeat(512)).unwrap_err())
                .contains("source string limits")
        );
        assert!(invalid_message(world.set_userinfo_storage(1, "a\0b").unwrap_err()).contains("source string limits"));
        assert!(invalid_message(world.set_userinfo_storage(9, "x").unwrap_err()).contains("not connected"));
    }

    #[test]
    fn connect_rejects_duplicate_slot_before_host() {
        let mut world = hollow_world(WorldPhase::Running);
        world.clients.insert(1, connected_client(1));
        assert!(invalid_message(world.connect(1, "x").unwrap_err()).contains("unavailable"));
    }

    #[test]
    fn input_retirement_completes_on_close() {
        let mut world = hollow_world(WorldPhase::Created);
        world.clients.insert(0, connected_client(0));
        let retired = Rc::new(RefCell::new(Vec::new()));
        world.bind_input(test_services(retired.clone()));
        let mut table = SyntheticInputTable::table(1, 16, true);
        world
            .input_binding()
            .expect("binding")
            .dispatch_think(&mut table, 0, || Ok(()))
            .expect("dispatch");
        assert_eq!(world.pending_retirements.borrow().len(), 1);
        assert!(world.close().is_err());
        assert_eq!(retired.borrow().len(), 1);
        assert_eq!(retired.borrow()[0].slot, 0);
        assert!(world.pending_retirements.borrow().is_empty());
        assert!(world.clients.is_empty());
        assert_eq!(world.phase, WorldPhase::Closed);
    }

    #[test]
    fn client_phase_spells_donor_strings() {
        assert_eq!(ClassicGuestClientPhase::Connected.as_str(), "connected");
        assert_eq!(ClassicGuestClientPhase::Active.as_str(), "active");
    }
}
