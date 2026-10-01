//! Rerelease Quake II guest world facade.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-guest-world.ts`
//! (`RereleaseGuestWorld`, `RereleaseGuestWorldOptions`, `RereleaseGuestSave`,
//! `RereleaseGuestRevisit`).
//!
//! The donor drives the DLL through the host module (`preInit`, `prepFrame`,
//! `runFrame`, `spawnEntities`, `ClientConnect`, `clientBegin`,
//! `clientDisconnect`, `ClientUserinfoChanged`, `ClientCommand`,
//! `ServerCommand`, `clientThink`). The Rust
//! [`RereleaseQ2GuestHost`](qa_compat::q2::rerelease::host::RereleaseQ2GuestHost)
//! is headless: game code never runs. The facade therefore ports the full
//! lifecycle state machine, client roster, input retirement, travel, and
//! save flows against the calls the headless host answers
//! (reserve/release/rebind/reconcile/saves), and each method notes where DLL
//! execution is skipped. The skipped execution shares one root cause with
//! the source lane's reported gap: live CPU/runner/runtime wiring is wave 2.
//!
//! Async loading twins take a `nextFrame` pump instead of a promise; each
//! loading operation pumps once per host call. Client admission accepts with
//! the original userinfo: game admission rules live in the DLL, so the
//! headless port admits unconditionally.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_compat::q2::classic::pmove::EquipmentMovement;
use qa_compat::q2::native_input::{InputIdentity, InputServices, NativeInputBinding};
use qa_compat::q2::rerelease::host::SourceSave;
use qa_compat::q2::rerelease::public_state::RereleasePublicEdict;
use qa_core::identity::ActorId;
use qa_guest::core::contracts::ModuleIdentity;
use qa_guest::runtime::windows::contracts::WindowsCapabilities;
use qa_net::q2_adapters::{Q2RereleaseEntityState, Q2RereleasePlayerState, Q2RereleaseUserCommand};

use super::classic_guest_services::GuestCommandLine;
use super::classic_guest_world::{ClassicGuestClient, ClassicGuestClientPhase, ClassicGuestMap};
use super::rerelease_guest_services::RereleaseGuestServices;
use super::rerelease_guest_services_contract::{
    RereleaseEntityInfo, RereleaseGuestMapServices, RereleaseGuestMessage, RereleaseGuestServicesError,
    RereleaseGuestServicesOptions, RereleaseGuestServicesPort, RereleaseResult,
};
use super::rerelease_guest_source::{
    GuestClock, PreparedRereleaseGuest, RereleaseGuestSource, RereleaseGuestSourceOptions,
};

/// Construction options for [`RereleaseGuestWorld::create`].
pub struct RereleaseGuestWorldOptions {
    /// Prepared guest module.
    pub prepared: PreparedRereleaseGuest,
    /// Guest import services options.
    pub services: RereleaseGuestServicesOptions,
    /// Windows capabilities for guest file access.
    pub capabilities: WindowsCapabilities,
    /// Instruction budget override.
    pub instruction_budget: Option<u64>,
}

/// Game plus level saves (donor `RereleaseGuestSave`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseGuestSave {
    /// Game save.
    pub game: SourceSave,
    /// Level save.
    pub level: SourceSave,
}

/// Hub-level revisit: restore server state, then read the level save.
pub struct RereleaseGuestRevisit {
    /// Saved level.
    pub level: SourceSave,
    /// Server-state restoration over the transferred world.
    pub restore_server_state: Box<dyn FnMut(&mut RereleaseGuestWorld)>,
}

/// Client connection outcome (donor `connect`/`admit` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseConnectOutcome {
    /// Whether the client was admitted.
    pub allowed: bool,
    /// Current userinfo.
    pub userinfo: String,
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

fn host_mapped(error: impl ToString) -> RereleaseGuestServicesError {
    RereleaseGuestServicesError::Host(error.to_string())
}

fn source_mapped(error: impl ToString) -> RereleaseGuestServicesError {
    RereleaseGuestServicesError::Host(error.to_string())
}
/// API2023 guest world: sequences the module lifecycle.
pub struct RereleaseGuestWorld {
    source: Option<RereleaseGuestSource>,
    services: Option<RereleaseGuestServices>,
    command_context: Rc<RefCell<Option<GuestCommandLine>>>,
    clients: HashMap<u32, ClassicGuestClient>,
    input_binding: Option<NativeInputBinding>,
    pending_retirements: Rc<RefCell<Vec<PendingRetirement>>>,
    frame: u32,
    phase: WorldPhase,
    busy: bool,
}

impl RereleaseGuestWorld {
    /// World edition tag.
    #[must_use]
    pub fn edition(&self) -> &'static str {
        "rerelease"
    }

    /// Whether ownership moved away or the world closed.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.phase == WorldPhase::Transferred || self.phase == WorldPhase::Closed
    }

    /// Bound module identity.
    pub fn module(&self) -> RereleaseResult<ModuleIdentity> {
        let (source, _) = self.split()?;
        Ok(source.host.memory.module().clone())
    }

    /// Connected clients.
    pub fn clients(&self) -> RereleaseResult<Vec<ClassicGuestClient>> {
        self.require_ownership()?;
        Ok(self.clients.values().cloned().collect())
    }

    /// Build a world over a prepared guest.
    pub fn create(options: RereleaseGuestWorldOptions) -> RereleaseResult<Self> {
        let command_context: Rc<RefCell<Option<GuestCommandLine>>> = Rc::new(RefCell::new(None));
        let caller_command = options.services.base.command.clone();
        let mut services_options = options.services;
        services_options.base.command = {
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
        let mut services = RereleaseGuestServices::new(services_options)?;
        let clock = options.capabilities;
        let (Some(now_milliseconds), Some(performance_counter), Some(performance_frequency)) = (
            clock.now_milliseconds,
            clock.performance_counter,
            clock.performance_frequency,
        ) else {
            return Err(RereleaseGuestServicesError::invalid(
                "Native rerelease game requires the host clock capabilities",
            ));
        };
        let mut source = RereleaseGuestSource::create(
            options.prepared,
            RereleaseGuestSourceOptions {
                clock: GuestClock {
                    now_milliseconds,
                    performance_counter,
                    performance_frequency,
                },
                instruction_budget: options.instruction_budget,
                foreign_damage: services.options().foreign_damage.clone(),
                pickups: services.options().base.pickups.clone(),
                intercept_import: None,
            },
        )
        .map_err(source_mapped)?;
        if let Err(error) = services.bind_host(&mut source.host) {
            return match source.close() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(RereleaseGuestServicesError::invalid(format!(
                    "Native services binding and cleanup failed: {error} + {cleanup}"
                ))),
            };
        }
        Ok(Self {
            source: Some(source),
            services: Some(services),
            command_context,
            clients: HashMap::new(),
            input_binding: None,
            pending_retirements: Rc::new(RefCell::new(Vec::new())),
            frame: 0,
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

    fn require_ownership(&self) -> RereleaseResult<()> {
        if self.is_retired() {
            return Err(RereleaseGuestServicesError::invalid(
                "Native rerelease world is retired",
            ));
        }
        Ok(())
    }

    fn split(&self) -> RereleaseResult<(&RereleaseGuestSource, &RereleaseGuestServices)> {
        self.require_ownership()?;
        let source = self
            .source
            .as_ref()
            .ok_or_else(|| RereleaseGuestServicesError::invalid("Native world has no retained source"))?;
        let services = self
            .services
            .as_ref()
            .ok_or_else(|| RereleaseGuestServicesError::invalid("Native world has no retained services"))?;
        Ok((source, services))
    }

    fn split_mut(&mut self) -> RereleaseResult<(&mut RereleaseGuestSource, &mut RereleaseGuestServices)> {
        self.require_ownership()?;
        let source = self
            .source
            .as_mut()
            .ok_or_else(|| RereleaseGuestServicesError::invalid("Native world has no retained source"))?;
        let services = self
            .services
            .as_mut()
            .ok_or_else(|| RereleaseGuestServicesError::invalid("Native world has no retained services"))?;
        Ok((source, services))
    }

    fn operation<T>(&mut self, run: impl FnOnce(&mut Self) -> RereleaseResult<T>) -> RereleaseResult<T> {
        self.require_ownership()?;
        if self.busy {
            return Err(RereleaseGuestServicesError::invalid(
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

    fn require_running(&self) -> RereleaseResult<()> {
        self.require_ownership()?;
        if self.phase != WorldPhase::Running {
            return Err(RereleaseGuestServicesError::invalid("Native world has not spawned"));
        }
        Ok(())
    }

    fn require_client(&self, slot: u32, phase: Option<ClassicGuestClientPhase>) -> RereleaseResult<ClassicGuestClient> {
        let client = self.clients.get(&slot).cloned().ok_or_else(|| {
            RereleaseGuestServicesError::invalid(format!(
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
                return Err(RereleaseGuestServicesError::invalid(format!(
                    "Native client {slot} is not {want}"
                )));
            }
        }
        Ok(client)
    }

    fn process_retirement(&mut self, pending: PendingRetirement) -> RereleaseResult<()> {
        let slot = pending.identity.slot;
        {
            let (source, _) = self.split_mut()?;
            source.host.retire_input_client(slot);
            let address = source.host.record_at(slot).map_err(host_mapped)?;
            let mut edict = RereleasePublicEdict::new(&mut source.host.memory, address).map_err(host_mapped)?;
            let active = edict.byte("inuse").map_err(host_mapped)? != 0;
            // The DLL `clientDisconnect` has no headless counterpart; the
            // reservation release below drops the client record.
            let _ = active;
            source.host.release_client_reservation(slot).map_err(host_mapped)?;
            source.host.finish_input_retirement(slot);
        }
        self.clients.remove(&slot);
        let mut retired = pending.retired.borrow_mut();
        retired(pending.identity);
        Ok(())
    }

    fn finish_input_retirements(&mut self) -> RereleaseResult<()> {
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

impl RereleaseGuestWorld {
    /// Run one server frame: begin, run, publish.
    ///
    /// The DLL `prepFrame`/`runFrame` pair has no headless counterpart; only
    /// the services frame bookkeeping and entity publication run.
    fn run_frame(&mut self, main_loop: bool) -> RereleaseResult<()> {
        self.frame = self.frame.wrapping_add(1);
        let frame = self.frame;
        let _ = main_loop;
        self.split_mut()?.1.begin_frame(frame);
        let (source, services) = self.split_mut()?;
        services.publish_entities(&mut source.host)?;
        Ok(())
    }

    /// Spawn a map over the initialized candidate.
    ///
    /// DLL `spawnEntities` and frame execution are skipped headless; frames
    /// still begin and publish.
    fn spawn_map(&mut self, map: &ClassicGuestMap) -> RereleaseResult<()> {
        if self.phase != WorldPhase::Initialized {
            return Err(RereleaseGuestServicesError::invalid(
                "Native SpawnEntities requires an initialized candidate",
            ));
        }
        self.frame = 0;
        let _ = map;
        self.run_frame(false)?;
        self.run_frame(false)?;
        self.split_mut()?.1.complete_spawn();
        self.phase = WorldPhase::Running;
        Ok(())
    }

    /// Run the DLL entry point.
    ///
    /// The donor `preInit` (cvar refresh plus import-table bind) folds into
    /// the source init; the headless host answers init directly.
    pub fn init(&mut self) -> RereleaseResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Created {
                return Err(RereleaseGuestServicesError::invalid(
                    "Native Init requires a fresh world",
                ));
            }
            {
                let (source, _) = world.split_mut()?;
                source.init().map_err(source_mapped)?;
            }
            world.phase = WorldPhase::Initialized;
            Ok(())
        })
    }

    /// Run the DLL entry point, pumping `next_frame` while loading.
    pub fn init_loading(&mut self, next_frame: &mut dyn FnMut()) -> RereleaseResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Created {
                return Err(RereleaseGuestServicesError::invalid(
                    "Native Init requires a fresh world",
                ));
            }
            {
                let (source, _) = world.split_mut()?;
                source.init_loading(next_frame).map_err(source_mapped)?;
            }
            world.phase = WorldPhase::Initialized;
            Ok(())
        })
    }

    /// Spawn entities and settle the first two frames.
    pub fn spawn(&mut self, map: &ClassicGuestMap) -> RereleaseResult<()> {
        self.operation(|world| world.spawn_map(map))
    }

    /// Spawn entities, pumping `next_frame` while loading.
    pub fn spawn_loading(&mut self, map: &ClassicGuestMap, next_frame: &mut dyn FnMut()) -> RereleaseResult<()> {
        self.require_ownership()?;
        if self.phase != WorldPhase::Initialized || self.busy {
            return Err(RereleaseGuestServicesError::invalid(
                "Native loading requires an idle initialized world",
            ));
        }
        self.busy = true;
        let result = (|| -> RereleaseResult<()> {
            self.spawn_map(map)?;
            next_frame();
            Ok(())
        })();
        self.busy = false;
        result
    }

    fn rebound_command(&self, binding: RereleaseGuestMapServices) -> RereleaseGuestMapServices {
        let context = self.command_context.clone();
        let caller = binding.base.command.clone();
        let mut rebound = binding;
        rebound.base.command = Rc::new(move || {
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

    fn transfer(
        &mut self,
        map: ClassicGuestMap,
        binding: RereleaseGuestMapServices,
        revisit: Option<RereleaseGuestRevisit>,
    ) -> RereleaseResult<RereleaseGuestWorld> {
        self.require_running()?;
        RereleaseGuestServices::validate_map(&binding)?;
        {
            let (_, services) = self.split()?;
            if revisit.is_some() && services.options().base.cvars.variable_value("deathmatch") != 0.0 {
                return Err(RereleaseGuestServicesError::invalid(
                    "Original API 3 deathmatch does not restore hub levels",
                ));
            }
        }
        self.finish_input_retirements()?;
        let clients: Vec<ClassicGuestClient> = self.clients.values().cloned().collect();
        let mut next = RereleaseGuestWorld {
            source: self.source.take(),
            services: self.services.take(),
            command_context: self.command_context.clone(),
            clients: HashMap::new(),
            input_binding: None,
            pending_retirements: Rc::new(RefCell::new(Vec::new())),
            frame: 0,
            phase: WorldPhase::Initialized,
            busy: false,
        };
        self.input_binding = None;
        self.phase = WorldPhase::Transferred;
        self.clients.clear();
        let rebound = next.rebound_command(binding);
        let settled = (|| -> RereleaseResult<()> {
            let (source, services) = next.split_mut()?;
            // The map validated above, so the commit-time rebind only swaps tables.
            source
                .host
                .rebind_world(|| {
                    let _ = services.rebind_world(rebound);
                })
                .map_err(host_mapped)?;
            Ok(())
        })();
        let settled = settled.and_then(|()| {
            next.spawn_map(&map)?;
            if let Some(mut revisit) = revisit {
                next.split_mut()?.1.drain_messages();
                (revisit.restore_server_state)(&mut next);
                next.split_mut()?
                    .0
                    .host
                    .read_save(&revisit.level)
                    .map_err(host_mapped)?;
                next.split_mut()?.1.complete_spawn();
            }
            for client in &clients {
                next.split_mut()?
                    .0
                    .host
                    .reserve_client(client.slot)
                    .map_err(host_mapped)?;
                next.clients.insert(
                    client.slot,
                    ClassicGuestClient {
                        phase: ClassicGuestClientPhase::Connected,
                        ..client.clone()
                    },
                );
            }
            Ok(())
        });
        match settled {
            Ok(()) => {
                next.phase = WorldPhase::Running;
                Ok(next)
            }
            Err(error) => {
                if let Err(cleanup) = next.close() {
                    next.phase = WorldPhase::Closed;
                    next.clients.clear();
                    return Err(RereleaseGuestServicesError::invalid(format!(
                        "Native map travel and retained module cleanup failed: {error} + {cleanup}"
                    )));
                }
                next.phase = WorldPhase::Closed;
                next.clients.clear();
                Err(error)
            }
        }
    }

    /// Irreversible ownership transfer after the caller staged the destination map.
    pub fn travel(
        &mut self,
        map: ClassicGuestMap,
        binding: RereleaseGuestMapServices,
        revisit: Option<RereleaseGuestRevisit>,
    ) -> RereleaseResult<RereleaseGuestWorld> {
        self.operation(|world| world.transfer(map, binding, revisit))
    }

    /// Irreversible ownership transfer, pumping `next_frame` while loading.
    pub fn travel_loading(
        &mut self,
        map: ClassicGuestMap,
        binding: RereleaseGuestMapServices,
        next_frame: &mut dyn FnMut(),
        revisit: Option<RereleaseGuestRevisit>,
    ) -> RereleaseResult<RereleaseGuestWorld> {
        let next = self.operation(|world| world.transfer(map, binding, revisit))?;
        next_frame();
        Ok(next)
    }

    /// Connect a client slot.
    ///
    /// Game admission rules live in the DLL; the headless port admits
    /// unconditionally with the original userinfo.
    pub fn connect(
        &mut self,
        slot: u32,
        userinfo: &str,
        _social_id: u64,
        _is_bot: bool,
    ) -> RereleaseResult<RereleaseConnectOutcome> {
        self.operation(|world| {
            world.require_running()?;
            if world.clients.contains_key(&slot) {
                return Err(RereleaseGuestServicesError::invalid(
                    "Native client slot is unavailable",
                ));
            }
            let max_clients = world.split()?.1.options().base.max_clients;
            if slot < 1 || slot > max_clients {
                return Err(RereleaseGuestServicesError::invalid(
                    "Native client slot is unavailable",
                ));
            }
            world.split_mut()?.0.host.reserve_client(slot).map_err(host_mapped)?;
            world.clients.insert(
                slot,
                ClassicGuestClient {
                    slot,
                    phase: ClassicGuestClientPhase::Connected,
                    userinfo: userinfo.to_string(),
                },
            );
            Ok(RereleaseConnectOutcome {
                allowed: true,
                userinfo: userinfo.to_string(),
            })
        })
    }

    /// Begin a connected client.
    ///
    /// The DLL `clientBegin` event is skipped headless; the roster advances.
    pub fn begin(&mut self, slot: u32) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let client = world.require_client(slot, Some(ClassicGuestClientPhase::Connected))?;
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
    pub fn admit(
        &mut self,
        slot: u32,
        userinfo: &str,
        social_id: u64,
        is_bot: bool,
    ) -> RereleaseResult<RereleaseConnectOutcome> {
        let result = self.connect(slot, userinfo, social_id, is_bot)?;
        if result.allowed {
            self.begin(slot)?;
        }
        Ok(result)
    }

    /// Disconnect a client.
    ///
    /// The DLL `clientDisconnect` event is skipped headless; the reservation
    /// release drops the client record.
    pub fn disconnect(&mut self, slot: u32) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, None)?;
            world
                .split_mut()?
                .0
                .host
                .release_client_reservation(slot)
                .map_err(host_mapped)?;
            world.clients.remove(&slot);
            Ok(())
        })
    }

    /// Replace a client userinfo string.
    ///
    /// The DLL `ClientUserinfoChanged` round-trip is skipped headless; the
    /// validated value stores directly.
    pub fn userinfo(&mut self, slot: u32, value: &str) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let client = world.require_client(slot, None)?;
            if value.len() >= 2048 || value.contains('\0') {
                return Err(RereleaseGuestServicesError::invalid(
                    "Rerelease userinfo exceeds its source string limits",
                ));
            }
            world.clients.insert(
                slot,
                ClassicGuestClient {
                    userinfo: value.to_string(),
                    ..client
                },
            );
            Ok(())
        })
    }

    /// Run a client command.
    ///
    /// The DLL `ClientCommand` dispatch is skipped headless; the command
    /// context still scopes the call and the host reconciles.
    pub fn command(&mut self, slot: u32, arguments: &[String], args: &str) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, Some(ClassicGuestClientPhase::Active))?;
            *world.command_context.borrow_mut() = Some(GuestCommandLine {
                arguments: arguments.to_vec(),
                args: args.to_string(),
            });
            let result = world.split_mut()?.0.host.reconcile().map_err(host_mapped);
            *world.command_context.borrow_mut() = None;
            result?;
            Ok(())
        })
    }

    /// Run a server command.
    ///
    /// The DLL `ServerCommand` dispatch is skipped headless; the command
    /// context still scopes the call and the host reconciles.
    pub fn server_command(&mut self, arguments: &[String], args: &str) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            *world.command_context.borrow_mut() = Some(GuestCommandLine {
                arguments: arguments.to_vec(),
                args: args.to_string(),
            });
            let result = world.split_mut()?.0.host.reconcile().map_err(host_mapped);
            *world.command_context.borrow_mut() = None;
            result?;
            Ok(())
        })
    }

    /// Run client input.
    ///
    /// The DLL `clientThink` dispatch is skipped headless; the call validates
    /// the roster only.
    pub fn think(
        &mut self,
        slot: u32,
        _command: Q2RereleaseUserCommand,
        _movement: Option<EquipmentMovement>,
    ) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, Some(ClassicGuestClientPhase::Active))?;
            Ok(())
        })
    }

    /// Run one server frame.
    pub fn frame(&mut self, milliseconds: i32) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            let frame_milliseconds = world.split()?.1.options().frame_milliseconds;
            if milliseconds != frame_milliseconds {
                return Err(RereleaseGuestServicesError::invalid(
                    "API2023 RunFrame requires the configured source cadence",
                ));
            }
            world.run_frame(true)
        })
    }
}

impl RereleaseGuestWorld {
    /// Bound actor for a slot.
    pub fn actor(&mut self, slot: u32) -> RereleaseResult<Option<ActorId>> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        Ok(services.entity_info(&mut source.host, slot)?.actor)
    }

    /// Entity observation for a slot.
    pub fn entity_info(&mut self, slot: u32) -> RereleaseResult<RereleaseEntityInfo> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.entity_info(&mut source.host, slot)
    }

    /// Entity states for every visible in-use slot.
    pub fn entity_states(&mut self) -> RereleaseResult<Vec<Q2RereleaseEntityState>> {
        self.require_running()?;
        let count = self.split()?.0.host.entity_count;
        let mut states = Vec::new();
        for slot in 1..count {
            let live = {
                let (source, services) = self.split_mut()?;
                let info = services.entity_info(&mut source.host, slot)?;
                info.active && info.server_flags & 1 == 0
            };
            if live {
                let (source, services) = self.split_mut()?;
                states.push(services.entity_state(&mut source.host, slot)?);
            }
        }
        Ok(states)
    }

    /// Entity state for one slot.
    pub fn entity_state(&mut self, slot: u32) -> RereleaseResult<Q2RereleaseEntityState> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.entity_state(&mut source.host, slot)
    }

    /// Player ping for a slot.
    pub fn player_ping(&mut self, slot: u32) -> RereleaseResult<i32> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.player_ping(&mut source.host, slot)
    }

    /// Set the player ping for a slot.
    pub fn set_player_ping(&mut self, slot: u32, ping: i32) -> RereleaseResult<()> {
        self.operation(|world| {
            world.require_running()?;
            world.require_client(slot, None)?;
            let (source, services) = world.split_mut()?;
            services.set_player_ping(&mut source.host, slot, ping)?;
            Ok(())
        })
    }

    /// Player state for one slot.
    pub fn player_state(&mut self, slot: u32) -> RereleaseResult<Q2RereleasePlayerState> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.player_state(&mut source.host, slot)
    }

    /// Model appearance for one slot.
    pub fn model_appearance(&mut self, slot: u32) -> RereleaseResult<super::classic_guest_services::ModelAppearance> {
        self.require_running()?;
        let (source, services) = self.split_mut()?;
        services.model_appearance(&mut source.host, slot)
    }

    /// Drain queued guest messages.
    pub fn raw_messages(&mut self) -> RereleaseResult<Vec<RereleaseGuestMessage>> {
        Ok(self.split_mut()?.1.drain_messages())
    }

    /// Current configstrings.
    pub fn configstrings(&self) -> RereleaseResult<HashMap<i32, String>> {
        Ok(self.split()?.1.configstrings())
    }

    /// Restore engine configstring metadata before the module reads its level save.
    pub fn restore_configstrings(&mut self, values: &HashMap<i32, String>) -> RereleaseResult<()> {
        if self.phase != WorldPhase::Initialized {
            return Err(RereleaseGuestServicesError::invalid(
                "Native configstring restoration requires the initialized candidate",
            ));
        }
        self.split_mut()?.1.restore_configstrings(values)
    }

    /// Write game and level saves.
    pub fn write_save(&mut self, automatic: bool) -> RereleaseResult<RereleaseGuestSave> {
        self.operation(|world| {
            world.require_running()?;
            let (source, _) = world.split_mut()?;
            let game = source.host.write_save("game", automatic).map_err(host_mapped)?;
            let level = source.host.write_save("level", false).map_err(host_mapped)?;
            Ok(RereleaseGuestSave { game, level })
        })
    }

    /// Write game and level saves, pumping `next_frame` while loading.
    pub fn write_save_loading(
        &mut self,
        automatic: bool,
        next_frame: &mut dyn FnMut(),
    ) -> RereleaseResult<RereleaseGuestSave> {
        self.operation(|world| {
            world.require_running()?;
            let (source, _) = world.split_mut()?;
            let game = source.host.write_save("game", automatic).map_err(host_mapped)?;
            next_frame();
            let level = source.host.write_save("level", false).map_err(host_mapped)?;
            next_frame();
            Ok(RereleaseGuestSave { game, level })
        })
    }

    /// Write the travel level save.
    pub fn write_travel_level(&mut self) -> RereleaseResult<SourceSave> {
        self.operation(|world| {
            world.require_running()?;
            world.split_mut()?.0.host.write_save("level", true).map_err(host_mapped)
        })
    }

    /// Write the travel level save, pumping `next_frame` while loading.
    pub fn write_travel_level_loading(&mut self, next_frame: &mut dyn FnMut()) -> RereleaseResult<SourceSave> {
        self.operation(|world| {
            world.require_running()?;
            let save = world
                .split_mut()?
                .0
                .host
                .write_save("level", true)
                .map_err(host_mapped)?;
            next_frame();
            Ok(save)
        })
    }

    /// Restore game and level saves over a fresh initialized candidate.
    pub fn restore(
        &mut self,
        save: &RereleaseGuestSave,
        map: &ClassicGuestMap,
        restore_server_state: impl FnMut(),
    ) -> RereleaseResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Initialized {
                return Err(RereleaseGuestServicesError::invalid(
                    "Original API2023 import requires a fresh initialized candidate",
                ));
            }
            let mut restore_server_state = restore_server_state;
            {
                let (source, _) = world.split_mut()?;
                source.host.read_save(&save.game).map_err(host_mapped)?;
            }
            world.spawn_map(map)?;
            {
                let (source, services) = world.split_mut()?;
                services.drain_messages();
                restore_server_state();
                source.host.read_save(&save.level).map_err(host_mapped)?;
                services.complete_spawn();
            }
            world.phase = WorldPhase::Running;
            Ok(())
        })
    }

    /// Restore saves, pumping `next_frame` while loading.
    pub fn restore_loading(
        &mut self,
        save: &RereleaseGuestSave,
        map: &ClassicGuestMap,
        restore_server_state: impl FnMut(),
        next_frame: &mut dyn FnMut(),
    ) -> RereleaseResult<()> {
        self.operation(|world| {
            if world.phase != WorldPhase::Initialized {
                return Err(RereleaseGuestServicesError::invalid(
                    "Original API2023 import requires a fresh initialized candidate",
                ));
            }
            let mut restore_server_state = restore_server_state;
            {
                let (source, _) = world.split_mut()?;
                source.host.read_save(&save.game).map_err(host_mapped)?;
                next_frame();
            }
            world.spawn_map(map)?;
            next_frame();
            {
                let (source, services) = world.split_mut()?;
                services.drain_messages();
                restore_server_state();
                source.host.read_save(&save.level).map_err(host_mapped)?;
                next_frame();
                services.complete_spawn();
            }
            world.phase = WorldPhase::Running;
            Ok(())
        })
    }

    /// Shut the module down and release the world.
    pub fn close(&mut self) -> RereleaseResult<()> {
        if self.is_retired() {
            return Ok(());
        }
        self.input_binding = None;
        self.operation(|world| {
            world.discard_input_retirements();
            let result = match world.split_mut() {
                Ok((source, _)) => source.close().map_err(source_mapped),
                Err(error) => Err(error),
            };
            world.phase = WorldPhase::Closed;
            world.clients.clear();
            result
        })
    }

    /// Discard the world without running module shutdown.
    pub fn discard(&mut self) -> RereleaseResult<()> {
        self.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_compat::q2::native_input::SyntheticInputTable;
    use qa_compat::q2::native_primary_weapons::NativeActorId;

    fn hollow_world(phase: WorldPhase) -> RereleaseGuestWorld {
        RereleaseGuestWorld {
            source: None,
            services: None,
            command_context: Rc::new(RefCell::new(None)),
            clients: HashMap::new(),
            input_binding: None,
            pending_retirements: Rc::new(RefCell::new(Vec::new())),
            frame: 0,
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

    fn invalid_message(error: RereleaseGuestServicesError) -> String {
        match error {
            RereleaseGuestServicesError::Invalid(message) => message,
            other => panic!("expected invalid error, got {other:?}"),
        }
    }

    #[test]
    fn edition_and_retirement_flags() {
        let world = hollow_world(WorldPhase::Created);
        assert_eq!(world.edition(), "rerelease");
        assert!(!world.is_retired());
        assert!(hollow_world(WorldPhase::Transferred).is_retired());
        assert!(hollow_world(WorldPhase::Closed).is_retired());
        assert!(!hollow_world(WorldPhase::Running).is_retired());
    }

    #[test]
    fn retired_world_blocks_access() {
        for phase in [WorldPhase::Transferred, WorldPhase::Closed] {
            let mut world = hollow_world(phase);
            assert!(invalid_message(world.clients().unwrap_err()).contains("retired"));
            assert!(invalid_message(world.module().unwrap_err()).contains("retired"));
            assert!(invalid_message(world.init().unwrap_err()).contains("retired"));
        }
    }

    #[test]
    fn lifecycle_guards_reject_wrong_phase() {
        let map = ClassicGuestMap {
            map: "m".to_string(),
            entities: "e".to_string(),
            spawn_point: String::new(),
        };
        let mut world = hollow_world(WorldPhase::Initialized);
        assert!(invalid_message(world.init().unwrap_err()).contains("fresh world"));
        let mut world = hollow_world(WorldPhase::Created);
        assert!(invalid_message(world.spawn(&map).unwrap_err()).contains("initialized candidate"));
        assert!(invalid_message(world.spawn_loading(&map, &mut || {}).unwrap_err()).contains("idle initialized"));
        assert!(
            invalid_message(world.restore_configstrings(&HashMap::new()).unwrap_err())
                .contains("initialized candidate")
        );
        assert!(invalid_message(world.restore(&save(), &map, || {}).unwrap_err()).contains("fresh initialized"));
    }

    fn save() -> RereleaseGuestSave {
        RereleaseGuestSave {
            game: SourceSave {
                native: Vec::new(),
                deferred: Vec::new(),
                projections: Vec::new(),
            },
            level: SourceSave {
                native: Vec::new(),
                deferred: Vec::new(),
                projections: Vec::new(),
            },
        }
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
    fn busy_blocks_reentrant_operations() {
        let mut world = hollow_world(WorldPhase::Running);
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
        assert!(
            invalid_message(world.think(1, Q2RereleaseUserCommand::default(), None).unwrap_err())
                .contains("not active")
        );
    }

    #[test]
    fn userinfo_storage_validates_limits() {
        let mut world = hollow_world(WorldPhase::Running);
        world.clients.insert(1, connected_client(1));
        assert!(world.userinfo(1, "name Solid").is_ok());
        assert_eq!(world.clients.get(&1).unwrap().userinfo, "name Solid");
        assert!(invalid_message(world.userinfo(1, &"x".repeat(2048)).unwrap_err()).contains("source string limits"));
        assert!(invalid_message(world.userinfo(1, "a\0b").unwrap_err()).contains("source string limits"));
    }

    #[test]
    fn connect_rejects_duplicate_slot_before_host() {
        let mut world = hollow_world(WorldPhase::Running);
        world.clients.insert(1, connected_client(1));
        assert!(invalid_message(world.connect(1, "x", 0, false).unwrap_err()).contains("unavailable"));
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
}
