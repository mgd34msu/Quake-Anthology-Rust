//! Headless simulation ported from the `src/contracts/session.ts`
//! `Simulation` shape and `src/world/session/session.ts` structure: a
//! dedicated host constructs it without a renderer or SDL device. Owns the
//! registry, bodies, inventories, combat states, scheduler, and clock;
//! releases actors in source order. Also carries the transition
//! coordinator (`transitions.ts`) and a small dedicated-server command set
//! parsed with the `qa-core` command text layer. Ports
//! `src/world/session/resources.ts` (`ResourceScope`) and the
//! `EngineSession` lifecycle surface from `src/world/session/session.ts`;
//! seats, stepping, and presentations land with their owners.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::cmd::{tokenize_command, Dialect, TextMode};
use qa_core::identity::{ActorId, ClientId, IdentityOwner, OwnedActor, ProviderId, SavedActorId, SessionId};
use qa_core::math::{Bounds, Vec3};
use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};

use crate::body::{BodyState, BodyTable};
use crate::clocks::SourceClock;
use crate::combat::{q1_health_take, q2_health_take, q3_health_take, CombatState, Reaction};
use crate::inventory::{InventoryEntry, InventoryTable};
use crate::registry::{ActorRegistry, ActorSlotCheckpoint, SourceActorCheckpoint};
use crate::scheduler::{
    FrameOrdering, InvocationOrder, ScheduleOptions, Scheduler, ThinkBoundary, ThinkCallback, ThinkCheckpoint,
    ThinkTiming,
};
use crate::WorldError;

/// Simulation event payload.
#[derive(Debug, Clone, PartialEq)]
pub enum SimEventPayload {
    /// A think callback ran.
    Think {
        /// Actor.
        actor: SavedActorId,
        /// Callback identifier.
        callback: String,
    },
    /// Operator damage applied.
    Damage {
        /// Target.
        target: SavedActorId,
        /// Applied amount.
        amount: f64,
        /// Reaction.
        reaction: Reaction,
    },
    /// Operator message.
    Message(String),
}

/// One simulation event.
#[derive(Debug, Clone, PartialEq)]
pub struct SimEvent {
    /// Event sequence.
    pub sequence: u64,
    /// Source time.
    pub time: SourceTime,
    /// Payload.
    pub payload: SimEventPayload,
}

/// Snapshot actor.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotActor {
    /// Saved handle.
    pub id: SavedActorId,
    /// Owning provider.
    pub owner: ProviderId,
    /// Definition.
    pub definition: String,
}

/// Snapshot body.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotBody {
    /// Saved handle.
    pub id: SavedActorId,
    /// Body state.
    pub state: BodyState,
}

/// Snapshot inventory.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotInventory {
    /// Saved handle.
    pub id: SavedActorId,
    /// Entries.
    pub entries: Vec<InventoryEntry>,
}

/// World snapshot for one step.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldSnapshot {
    /// Frame context.
    pub frame: FrameContext,
    /// Live actors.
    pub actors: Vec<SnapshotActor>,
    /// Live bodies.
    pub bodies: Vec<SnapshotBody>,
    /// Live inventories.
    pub inventories: Vec<SnapshotInventory>,
}

/// Output of one simulation step.
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationOutput {
    /// World snapshot.
    pub snapshot: WorldSnapshot,
    /// Events since the previous step.
    pub events: Vec<SimEvent>,
}

/// Saved body state with a save-safe ground reference.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedBodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
    /// Ground reference.
    pub ground: Option<SavedActorId>,
}

/// Saved body record.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyCheckpoint {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
    /// Body state.
    pub state: SavedBodyState,
    /// Link count.
    pub link_count: u64,
    /// Linked snapshot, if linked.
    pub linked: Option<(SavedBodyState, Bounds)>,
}

/// Saved combat record.
#[derive(Debug, Clone, PartialEq)]
pub struct CombatCheckpoint {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
    /// Combat state.
    pub state: CombatState,
}

/// Saved inventory record.
#[derive(Debug, Clone, PartialEq)]
pub struct InventoryCheckpoint {
    /// Registry slot.
    pub slot: u32,
    /// Slot generation.
    pub generation: u32,
    /// Entries.
    pub entries: Vec<InventoryEntry>,
}

/// Save image, schema version 3.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveImage {
    /// Schema version.
    pub schema_version: u32,
    /// Frame context.
    pub frame: FrameContext,
    /// Next event sequence.
    pub next_event: u64,
    /// Actor slots.
    pub actors: Vec<ActorSlotCheckpoint>,
    /// Source bindings.
    pub sources: Vec<SourceActorCheckpoint>,
    /// Bodies.
    pub bodies: Vec<BodyCheckpoint>,
    /// Combat states.
    pub combats: Vec<CombatCheckpoint>,
    /// Inventories.
    pub inventories: Vec<InventoryCheckpoint>,
    /// Pending thinks.
    pub thinks: Vec<ThinkCheckpoint>,
}

/// Headless simulation.
pub struct Simulation {
    registry: ActorRegistry,
    bodies: BodyTable,
    inventories: InventoryTable,
    combats: HashMap<ActorId, CombatState>,
    scheduler: Scheduler,
    clock: SourceClock,
    primary: ProviderId,
    events: Vec<SimEvent>,
    next_event: u64,
    think_handlers: HashMap<String, ThinkCallback>,
    closed: bool,
}

impl Simulation {
    /// Create a simulation with one primary provider clock.
    pub fn new(
        session_name: &str,
        primary: ProviderId,
        profile: ClockProfile,
        initial: SourceTime,
        capacity: usize,
    ) -> Result<Self, WorldError> {
        let owner = IdentityOwner::create(session_name)
            .map_err(|_| WorldError::CommandUsage("A session identity needs a name".to_string()))?;
        let registry = ActorRegistry::new(owner, capacity)?;
        let scheduler = Scheduler::new(
            FrameOrdering::Native { clock: profile },
            vec![(primary.clone(), profile)],
        )?;
        Ok(Self {
            registry,
            bodies: BodyTable::new(),
            inventories: InventoryTable::new(),
            combats: HashMap::new(),
            scheduler,
            clock: SourceClock::new(initial)?,
            primary,
            events: Vec::new(),
            next_event: 0,
            think_handlers: HashMap::new(),
            closed: false,
        })
    }

    /// Current frame context.
    #[must_use]
    pub fn frame(&self) -> FrameContext {
        self.clock.frame()
    }

    /// Bind a think handler by callback identifier.
    pub fn bind_think(&mut self, callback: &str, handler: ThinkCallback) {
        self.think_handlers.insert(callback.to_string(), handler);
    }

    /// Spawn an actor with optional body, combat, and inventory state.
    pub fn spawn(
        &mut self,
        owner: ProviderId,
        definition: &str,
        body: Option<BodyState>,
        combat: Option<CombatState>,
        inventory: Vec<InventoryEntry>,
    ) -> Result<OwnedActor, WorldError> {
        self.assert_open()?;
        let actor = self.registry.allocate(owner, definition)?;
        if let Some(body) = body {
            self.bodies.create(&self.registry, &actor, body)?;
        }
        if let Some(combat) = combat {
            self.combats.insert(actor.id().clone(), combat);
        }
        if !inventory.is_empty() {
            self.inventories.create(&self.registry, &actor, &inventory)?;
        }
        Ok(actor)
    }

    /// Release an actor and its attached children in source order.
    pub fn release(&mut self, actor: &OwnedActor) -> Result<(), WorldError> {
        self.assert_open()?;
        self.scheduler.cancel(&self.registry, actor)?;
        let children = self.bodies.release_actor(actor.id());
        for child in &children {
            if let Some(owned) = self.registry.resolve_owned(child) {
                self.inventories.release_actor(child);
                self.combats.remove(child);
                self.scheduler.cancel(&self.registry, &owned)?;
                self.registry.release(&owned)?;
            }
        }
        self.inventories.release_actor(actor.id());
        self.combats.remove(actor.id());
        self.registry.release(actor)?;
        Ok(())
    }

    /// Live actor count.
    #[must_use]
    pub fn actor_count(&self) -> usize {
        self.registry.observations().len()
    }

    /// Borrow the actor registry (server trigger/timer passes).
    #[must_use]
    pub fn registry(&self) -> &ActorRegistry {
        &self.registry
    }

    /// Borrow the body table (server trigger passes).
    #[must_use]
    pub fn bodies(&self) -> &BodyTable {
        &self.bodies
    }

    /// Read one body state.
    #[must_use]
    pub fn body_state(&self, actor: &ActorId) -> Option<BodyState> {
        self.bodies.read(&self.registry, actor)
    }

    /// Actors with bodies, in observation order.
    #[must_use]
    pub fn body_actors(&self) -> Vec<ActorId> {
        self.registry
            .observations()
            .into_iter()
            .filter(|observed| self.bodies.read(&self.registry, &observed.id).is_some())
            .map(|observed| observed.id)
            .collect()
    }

    /// Move a body to an origin (server mover pass).
    pub fn set_body_origin(&mut self, actor: &ActorId, origin: Vec3) -> Result<(), WorldError> {
        self.assert_open()?;
        let Some(owned) = self.registry.resolve_owned(actor) else {
            return Err(WorldError::StaleActor);
        };
        let Some(mut state) = self.bodies.read(&self.registry, actor) else {
            return Err(WorldError::BodyMissing);
        };
        state.origin = origin;
        self.bodies.write(&self.registry, &owned, state)?;
        Ok(())
    }

    /// Capture a body's spatial snapshot (server trigger pass).
    pub fn link_body(&mut self, actor: &ActorId) -> Result<(), WorldError> {
        self.assert_open()?;
        let Some(owned) = self.registry.resolve_owned(actor) else {
            return Err(WorldError::StaleActor);
        };
        self.bodies.link(&self.registry, &owned, None)?;
        Ok(())
    }

    /// Resolve a live actor by registry slot.
    #[must_use]
    pub fn actor_by_slot(&self, slot: u32) -> Option<OwnedActor> {
        self.actor_by_slot_inner(slot)
    }

    /// Advance one source step, running due thinks.
    pub fn step(&mut self, elapsed: SourceTime) -> Result<SimulationOutput, WorldError> {
        self.assert_open()?;
        let frame = self.clock.advance(elapsed, FramePhase::FrameEntry)?;
        let fired: Rc<RefCell<Vec<(SavedActorId, String)>>> = Rc::new(RefCell::new(vec![]));
        {
            let handlers = &self.think_handlers;
            // Borrow dance: scheduler and registry are disjoint fields.
            let (scheduler, registry, primary) = (&self.scheduler, &self.registry, &self.primary);
            let resolver = |_: &ProviderId, callback: &str| -> Option<ThinkCallback> {
                let handler = handlers.get(callback)?.clone();
                let fired = Rc::clone(&fired);
                let callback = callback.to_owned();
                Some(
                    Rc::new(move |owned: &OwnedActor, frame: FrameContext, scheduler: &Scheduler| {
                        fired
                            .borrow_mut()
                            .push((SavedActorId::from(owned.id()), callback.clone()));
                        handler(owned, frame, scheduler);
                    }) as ThinkCallback,
                )
            };
            scheduler.advance(
                registry,
                &resolver,
                &[(primary.clone(), frame)],
                ThinkBoundary::BeforePhysics,
            )?;
            scheduler.advance(
                registry,
                &resolver,
                &[(primary.clone(), frame)],
                ThinkBoundary::DuringPhysics,
            )?;
            scheduler.advance(
                registry,
                &resolver,
                &[(primary.clone(), frame)],
                ThinkBoundary::AfterPhysics,
            )?;
        }
        for (actor, callback) in fired.borrow().iter() {
            self.push_event(SimEventPayload::Think {
                actor: *actor,
                callback: callback.clone(),
            });
        }
        let snapshot = self.snapshot();
        let events = std::mem::take(&mut self.events);
        Ok(SimulationOutput { snapshot, events })
    }

    fn push_event(&mut self, payload: SimEventPayload) {
        let sequence = self.next_event;
        self.next_event += 1;
        self.events.push(SimEvent {
            sequence,
            time: self.clock.frame().time,
            payload,
        });
    }

    fn snapshot(&self) -> WorldSnapshot {
        let actors: Vec<SnapshotActor> = self
            .registry
            .observations()
            .into_iter()
            .map(|observed| SnapshotActor {
                id: SavedActorId::from(&observed.id),
                owner: observed.owner,
                definition: observed.definition,
            })
            .collect();
        let mut bodies = Vec::new();
        let mut inventories = Vec::new();
        for actor in &actors {
            let live = self.registry.observations().into_iter().find(|observed| {
                observed.id.slot() == actor.id.slot && observed.id.generation() == actor.id.generation
            });
            if let Some(live) = live {
                if let Some(state) = self.bodies.read(&self.registry, &live.id) {
                    bodies.push(SnapshotBody { id: actor.id, state });
                }
                let entries = self.inventories.entries(&self.registry, &live.id);
                if !entries.is_empty() {
                    inventories.push(SnapshotInventory { id: actor.id, entries });
                }
            }
        }
        WorldSnapshot {
            frame: self.clock.frame(),
            actors,
            bodies,
            inventories,
        }
    }

    /// Checkpoint the simulation.
    pub fn checkpoint(&self) -> Result<SaveImage, WorldError> {
        self.assert_open()?;
        let mut bodies = Vec::new();
        let mut combats = Vec::new();
        let mut inventories = Vec::new();
        for observed in self.registry.observations() {
            let id = SavedActorId::from(&observed.id);
            if let Some(state) = self.bodies.read(&self.registry, &observed.id) {
                bodies.push(BodyCheckpoint {
                    slot: id.slot,
                    generation: id.generation,
                    state: saved_body(&state),
                    link_count: self
                        .bodies
                        .linked(&self.registry, &observed.id)
                        .map_or(0, |linked| linked.link_count),
                    linked: self
                        .bodies
                        .linked(&self.registry, &observed.id)
                        .map(|linked| (saved_body(&linked.state), linked.absolute_bounds)),
                });
            }
            if let Some(state) = self.combats.get(&observed.id) {
                combats.push(CombatCheckpoint {
                    slot: id.slot,
                    generation: id.generation,
                    state: state.clone(),
                });
            }
            let entries = self.inventories.entries(&self.registry, &observed.id);
            if !entries.is_empty() {
                inventories.push(InventoryCheckpoint {
                    slot: id.slot,
                    generation: id.generation,
                    entries,
                });
            }
        }
        Ok(SaveImage {
            schema_version: 3,
            frame: self.clock.frame(),
            next_event: self.next_event,
            actors: self.registry.checkpoint(),
            sources: self.registry.source_checkpoint(),
            bodies,
            combats,
            inventories,
            thinks: self.scheduler.think_checkpoint()?,
        })
    }

    /// Restore a simulation from a save image under a fresh authority.
    pub fn restore(
        session_name: &str,
        primary: ProviderId,
        profile: ClockProfile,
        image: SaveImage,
        capacity: usize,
    ) -> Result<Self, WorldError> {
        if image.schema_version != 3 {
            return Err(WorldError::BadSave("Unsupported save schema".to_string()));
        }
        let owner = IdentityOwner::create(session_name)
            .map_err(|_| WorldError::CommandUsage("A session identity needs a name".to_string()))?;
        let registry = ActorRegistry::restore(owner, &image.actors, &image.sources, capacity)?;
        let scheduler = Scheduler::new(
            FrameOrdering::Native { clock: profile },
            vec![(primary.clone(), profile)],
        )?;
        let mut simulation = Self {
            registry,
            bodies: BodyTable::new(),
            inventories: InventoryTable::new(),
            combats: HashMap::new(),
            scheduler,
            clock: SourceClock::restore(image.frame)?,
            primary,
            events: Vec::new(),
            next_event: image.next_event,
            think_handlers: HashMap::new(),
            closed: false,
        };
        for body in &image.bodies {
            let Some(owned) = simulation.actor_by_saved(body.slot, body.generation) else {
                return Err(WorldError::BadSave("Body names a missing actor".to_string()));
            };
            let state = restore_body(&simulation.registry, &body.state)?;
            simulation.bodies.create(&simulation.registry, &owned, state)?;
            let linked = body
                .linked
                .as_ref()
                .map(|(state, bounds)| restore_body(&simulation.registry, state).map(|state| (state, *bounds)))
                .transpose()?;
            simulation
                .bodies
                .restore_link_state(&simulation.registry, &owned, body.link_count, linked)?;
        }
        for combat in &image.combats {
            let Some(owned) = simulation.actor_by_saved(combat.slot, combat.generation) else {
                return Err(WorldError::BadSave("Combat names a missing actor".to_string()));
            };
            simulation.combats.insert(owned.id().clone(), combat.state.clone());
        }
        for inventory in &image.inventories {
            let Some(owned) = simulation.actor_by_saved(inventory.slot, inventory.generation) else {
                return Err(WorldError::BadSave("Inventory names a missing actor".to_string()));
            };
            simulation
                .inventories
                .create(&simulation.registry, &owned, &inventory.entries)?;
        }
        for think in &image.thinks {
            let Some(owned) = simulation.actor_by_saved(think.slot, think.generation) else {
                return Err(WorldError::BadSave("Think names a missing actor".to_string()));
            };
            simulation.scheduler.schedule(
                &simulation.registry,
                &owned,
                &think.callback,
                ThinkTiming {
                    execution_provider: think.execution_provider.clone(),
                    due: think.due,
                    boundary: think.boundary,
                    order: InvocationOrder {
                        provider: owned.owner().clone(),
                        actor: owned.id().clone(),
                        sequence: think.sequence,
                    },
                },
                ScheduleOptions {
                    registered_execution: None,
                    source_slot: Some(think.source_slot),
                },
            )?;
        }
        Ok(simulation)
    }

    fn actor_by_saved(&self, slot: u32, generation: u32) -> Option<OwnedActor> {
        self.registry
            .observations()
            .into_iter()
            .find(|observed| observed.id.slot() == slot && observed.id.generation() == generation)
            .and_then(|observed| self.registry.resolve_owned(&observed.id))
    }

    fn actor_by_slot_inner(&self, slot: u32) -> Option<OwnedActor> {
        self.registry
            .observations()
            .into_iter()
            .find(|observed| observed.id.slot() == slot)
            .and_then(|observed| self.registry.resolve_owned(&observed.id))
    }

    /// Execute one dedicated-server command.
    pub fn execute(&mut self, text: &str, dialect: Dialect) -> Result<String, WorldError> {
        self.assert_open()?;
        let tokens = tokenize_command(text, dialect, TextMode::Console)
            .map_err(|error| WorldError::CommandUsage(error.to_string()))?;
        let argv = tokens.argv;
        let command = argv.first().map_or("", String::as_str);
        match command {
            "status" => {
                let frame = self.clock.frame();
                Ok(format!(
                    "actors {} frame {} time {:?}",
                    self.actor_count(),
                    frame.frame,
                    frame.time
                ))
            }
            "spawn" => {
                let (Some(namespace), Some(name), Some(definition)) = (argv.get(1), argv.get(2), argv.get(3)) else {
                    return Err(WorldError::CommandUsage(
                        "Usage: spawn NAMESPACE NAME DEFINITION".to_string(),
                    ));
                };
                let actor = self.spawn(ProviderId::new(namespace, name), definition, None, None, Vec::new())?;
                Ok(format!(
                    "spawned slot {} generation {}",
                    actor.id().slot(),
                    actor.id().generation()
                ))
            }
            "release" => {
                let slot = self.parse_slot(&argv)?;
                let Some(actor) = self.actor_by_slot(slot) else {
                    return Err(WorldError::StaleActor);
                };
                self.release(&actor)?;
                Ok(format!("released slot {slot}"))
            }
            "give" => {
                let slot = self.parse_slot(&argv)?;
                let (Some(item), Some(count)) = (argv.get(2), argv.get(3)) else {
                    return Err(WorldError::CommandUsage("Usage: give SLOT ITEM COUNT".to_string()));
                };
                let count = parse_count(count)?;
                let Some(actor) = self.actor_by_slot(slot) else {
                    return Err(WorldError::StaleActor);
                };
                let given = self.inventories.give(&self.registry, &actor, item, count)?;
                Ok(format!("gave {given}"))
            }
            "hurt" => {
                let slot = self.parse_slot(&argv)?;
                let (Some(amount), Some(family)) = (argv.get(2), argv.get(3)) else {
                    return Err(WorldError::CommandUsage("Usage: hurt SLOT AMOUNT q1|q2|q3".to_string()));
                };
                let amount = parse_count(amount)?;
                let Some(actor) = self.actor_by_slot(slot) else {
                    return Err(WorldError::StaleActor);
                };
                let combat = self.combats.get(actor.id()).cloned().unwrap_or_default();
                if !combat.can_take_damage {
                    return Ok("immune".to_string());
                }
                let health = match family.as_str() {
                    "q1" => q1_health_take(combat.health, amount),
                    "q2" => q2_health_take(combat.health, amount),
                    "q3" => q3_health_take(combat.health, amount),
                    _ => {
                        return Err(WorldError::CommandUsage("Usage: hurt SLOT AMOUNT q1|q2|q3".to_string()));
                    }
                };
                let reaction = if health <= 0.0 { Reaction::Death } else { Reaction::Pain };
                self.combats
                    .insert(actor.id().clone(), CombatState { health, ..combat });
                self.push_event(SimEventPayload::Damage {
                    target: SavedActorId::from(actor.id()),
                    amount,
                    reaction,
                });
                Ok(format!("health {health}"))
            }
            "teleport" => {
                let slot = self.parse_slot(&argv)?;
                let (Some(x), Some(y), Some(z)) = (argv.get(2), argv.get(3), argv.get(4)) else {
                    return Err(WorldError::CommandUsage("Usage: teleport SLOT X Y Z".to_string()));
                };
                let origin = Vec3 {
                    x: parse_coordinate(x)?,
                    y: parse_coordinate(y)?,
                    z: parse_coordinate(z)?,
                };
                let Some(actor) = self.actor_by_slot(slot) else {
                    return Err(WorldError::StaleActor);
                };
                let Some(mut state) = self.bodies.read(&self.registry, actor.id()) else {
                    return Err(WorldError::BodyMissing);
                };
                state.origin = origin;
                self.bodies.write(&self.registry, &actor, state)?;
                Ok(format!("moved slot {slot}"))
            }
            "think" => {
                let slot = self.parse_slot(&argv)?;
                let (Some(callback), Some(due), Some(boundary)) = (argv.get(2), argv.get(3), argv.get(4)) else {
                    return Err(WorldError::CommandUsage(
                        "Usage: think SLOT CALLBACK DUE before|during|after".to_string(),
                    ));
                };
                let boundary = match boundary.as_str() {
                    "before" => ThinkBoundary::BeforePhysics,
                    "during" => ThinkBoundary::DuringPhysics,
                    "after" => ThinkBoundary::AfterPhysics,
                    _ => {
                        return Err(WorldError::CommandUsage(
                            "Usage: think SLOT CALLBACK DUE before|during|after".to_string(),
                        ));
                    }
                };
                let due = self.parse_due(due)?;
                let Some(actor) = self.actor_by_slot(slot) else {
                    return Err(WorldError::StaleActor);
                };
                self.scheduler.schedule(
                    &self.registry,
                    &actor,
                    callback,
                    ThinkTiming {
                        execution_provider: None,
                        due,
                        boundary,
                        order: InvocationOrder {
                            provider: actor.owner().clone(),
                            actor: actor.id().clone(),
                            sequence: 0,
                        },
                    },
                    ScheduleOptions::default(),
                )?;
                Ok(format!("scheduled slot {slot}"))
            }
            "tick" => {
                let Some(elapsed) = argv.get(1) else {
                    return Err(WorldError::CommandUsage("Usage: tick ELAPSED".to_string()));
                };
                let elapsed = self.parse_due(elapsed)?;
                let output = self.step(elapsed)?;
                Ok(format!(
                    "frame {} events {}",
                    output.snapshot.frame.frame,
                    output.events.len()
                ))
            }
            _ => Err(WorldError::UnknownCommand(command.to_string())),
        }
    }

    fn parse_slot(&self, argv: &[String]) -> Result<u32, WorldError> {
        argv.get(1)
            .ok_or_else(|| WorldError::CommandUsage("Missing actor slot".to_string()))
            .and_then(|slot| {
                slot.parse::<u32>()
                    .map_err(|_| WorldError::CommandUsage("Invalid actor slot".to_string()))
            })
    }

    fn parse_due(&self, text: &str) -> Result<SourceTime, WorldError> {
        match self.clock.frame().time {
            SourceTime::Seconds(_) => {
                let value = text
                    .parse::<f64>()
                    .map_err(|_| WorldError::CommandUsage("Invalid source time".to_string()))?;
                if !value.is_finite() {
                    return Err(WorldError::CommandUsage("Invalid source time".to_string()));
                }
                Ok(SourceTime::Seconds(value as f32))
            }
            SourceTime::Milliseconds(_) => {
                let value = text
                    .parse::<i32>()
                    .map_err(|_| WorldError::CommandUsage("Invalid source time".to_string()))?;
                Ok(SourceTime::Milliseconds(value))
            }
        }
    }

    fn assert_open(&self) -> Result<(), WorldError> {
        if self.closed {
            return Err(WorldError::SimulationClosed);
        }
        Ok(())
    }

    /// Close the simulation.
    pub fn close(&mut self) {
        self.closed = true;
        self.scheduler.close();
        self.registry.close();
    }
}

fn parse_count(text: &str) -> Result<f64, WorldError> {
    let value = text
        .parse::<f64>()
        .map_err(|_| WorldError::CommandUsage("Invalid count".to_string()))?;
    if !value.is_finite() || value < 0.0 {
        return Err(WorldError::CommandUsage("Invalid count".to_string()));
    }
    Ok(value)
}

fn parse_coordinate(text: &str) -> Result<f32, WorldError> {
    let value = text
        .parse::<f64>()
        .map_err(|_| WorldError::CommandUsage("Invalid coordinate".to_string()))?;
    if !value.is_finite() {
        return Err(WorldError::CommandUsage("Invalid coordinate".to_string()));
    }
    Ok(value as f32)
}

fn saved_body(state: &BodyState) -> SavedBodyState {
    SavedBodyState {
        origin: state.origin,
        angles: state.angles,
        velocity: state.velocity,
        bounds: state.bounds,
        ground: state.ground.as_ref().map(SavedActorId::from),
    }
}

fn restore_body(registry: &ActorRegistry, saved: &SavedBodyState) -> Result<BodyState, WorldError> {
    let ground = saved
        .ground
        .map(|ground| {
            registry
                .observations()
                .into_iter()
                .find(|observed| observed.id.slot() == ground.slot && observed.id.generation() == ground.generation)
                .map(|observed| observed.id)
                .ok_or_else(|| WorldError::BadSave("Ground names a missing actor".to_string()))
        })
        .transpose()?;
    Ok(BodyState {
        origin: saved.origin,
        angles: saved.angles,
        velocity: saved.velocity,
        bounds: saved.bounds,
        ground,
    })
}

/// Mission gate for campaign transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissionGate {
    /// Objective identifier.
    pub objective: String,
    /// Whether satisfied.
    pub satisfied: bool,
}

/// Transition intent from campaign or match controllers.
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionIntent {
    /// Travel to a campaign level.
    CampaignLevel {
        /// Campaign provider.
        campaign: ProviderId,
        /// Destination map.
        map: String,
        /// Spawn point.
        spawn: String,
        /// Mission gates.
        gates: Vec<MissionGate>,
    },
    /// Campaign complete.
    CampaignComplete {
        /// Campaign provider.
        campaign: ProviderId,
        /// Mission gates.
        gates: Vec<MissionGate>,
    },
    /// Round complete.
    RoundComplete {
        /// Match provider.
        match_id: ProviderId,
        /// Winner, if any.
        winner: Option<String>,
    },
    /// Match rotation.
    MatchRotation {
        /// Match provider.
        match_id: ProviderId,
        /// Destination map.
        map: String,
    },
}

/// Transition resolution mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionMode {
    /// Campaign mode.
    Campaign {
        /// Campaign provider.
        campaign: ProviderId,
        /// Allow round restarts.
        allow_round_restart: bool,
    },
    /// Competitive mode.
    Competitive {
        /// Match provider.
        match_id: ProviderId,
    },
    /// Combined mode; match rotation cannot bypass campaign gates.
    Combined {
        /// Campaign provider.
        campaign: ProviderId,
        /// Match provider.
        match_id: ProviderId,
        /// Resolve rounds before campaign intents.
        round_first: bool,
    },
}

/// Committed transition decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionDecision {
    /// Stay with blocked objectives.
    Stay {
        /// Blocked objectives.
        blocked: Vec<String>,
    },
    /// Round restart.
    Round {
        /// Winner, if any.
        winner: Option<String>,
    },
    /// Travel to a map.
    Travel {
        /// Destination map.
        map: String,
        /// Spawn point.
        spawn: String,
        /// Completes the campaign.
        complete_campaign: bool,
    },
    /// Campaign complete.
    CampaignComplete {
        /// Campaign provider.
        campaign: ProviderId,
    },
}

/// The coordinator alone commits travel after resolving campaign and match
/// intent order.
#[derive(Debug, Default)]
pub struct TransitionCoordinator {
    pending: Vec<TransitionDecision>,
    committing: bool,
}

impl TransitionCoordinator {
    /// Empty coordinator.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve intents in mode order.
    pub fn resolve(&mut self, mode: &TransitionMode, intents: &[TransitionIntent]) -> TransitionDecision {
        let campaign: Vec<&TransitionIntent> = intents
            .iter()
            .filter(|intent| {
                matches!(
                    intent,
                    TransitionIntent::CampaignLevel { .. } | TransitionIntent::CampaignComplete { .. }
                ) && match (intent, mode) {
                    (
                        TransitionIntent::CampaignLevel { campaign, .. }
                        | TransitionIntent::CampaignComplete { campaign, .. },
                        TransitionMode::Campaign { campaign: wanted, .. }
                        | TransitionMode::Combined { campaign: wanted, .. },
                    ) => campaign == wanted,
                    _ => false,
                }
            })
            .collect();
        let matches: Vec<&TransitionIntent> = intents
            .iter()
            .filter(|intent| match (intent, mode) {
                (
                    TransitionIntent::RoundComplete { .. },
                    TransitionMode::Campaign {
                        allow_round_restart: true,
                        ..
                    },
                ) => true,
                (
                    TransitionIntent::RoundComplete { match_id, .. } | TransitionIntent::MatchRotation { match_id, .. },
                    TransitionMode::Competitive { match_id: wanted }
                    | TransitionMode::Combined { match_id: wanted, .. },
                ) => match_id == wanted,
                _ => false,
            })
            .collect();
        let round_first = matches!(mode, TransitionMode::Combined { round_first: true, .. });
        let mut blocked: Vec<String> = Vec::new();
        let candidates: Vec<&&TransitionIntent> = if round_first {
            matches.iter().chain(campaign.iter()).collect()
        } else {
            campaign.iter().chain(matches.iter()).collect()
        };
        for intent in candidates {
            match intent {
                TransitionIntent::CampaignLevel { map, spawn, gates, .. } => {
                    let missing: Vec<String> = gates
                        .iter()
                        .filter(|gate| !gate.satisfied)
                        .map(|gate| gate.objective.clone())
                        .collect();
                    if !missing.is_empty() {
                        blocked.extend(missing);
                        continue;
                    }
                    let decision = TransitionDecision::Travel {
                        map: map.clone(),
                        spawn: spawn.clone(),
                        complete_campaign: false,
                    };
                    self.pending.push(decision.clone());
                    return decision;
                }
                TransitionIntent::CampaignComplete { campaign, gates } => {
                    let missing: Vec<String> = gates
                        .iter()
                        .filter(|gate| !gate.satisfied)
                        .map(|gate| gate.objective.clone())
                        .collect();
                    if !missing.is_empty() {
                        blocked.extend(missing);
                        continue;
                    }
                    let decision = TransitionDecision::CampaignComplete {
                        campaign: campaign.clone(),
                    };
                    self.pending.push(decision.clone());
                    return decision;
                }
                TransitionIntent::RoundComplete { winner, .. } => {
                    let decision = TransitionDecision::Round { winner: winner.clone() };
                    self.pending.push(decision.clone());
                    return decision;
                }
                TransitionIntent::MatchRotation { map, .. } => {
                    if matches!(mode, TransitionMode::Combined { .. }) {
                        continue;
                    }
                    let decision = TransitionDecision::Travel {
                        map: map.clone(),
                        spawn: String::new(),
                        complete_campaign: false,
                    };
                    self.pending.push(decision.clone());
                    return decision;
                }
            }
        }
        blocked.sort();
        blocked.dedup();
        TransitionDecision::Stay { blocked }
    }

    /// Commit a resolved decision exactly once.
    pub fn commit(
        &mut self,
        decision: &TransitionDecision,
        apply: &mut dyn FnMut(&TransitionDecision),
    ) -> Result<(), WorldError> {
        if matches!(decision, TransitionDecision::Stay { .. }) {
            return Ok(());
        }
        if self.committing {
            return Err(WorldError::TransitionBusy);
        }
        let index = self
            .pending
            .iter()
            .position(|pending| pending == decision)
            .ok_or(WorldError::TransitionStale)?;
        self.pending.remove(index);
        self.committing = true;
        apply(decision);
        self.committing = false;
        Ok(())
    }
}

// Session resources (donor `src/world/session/resources.ts`).

/// A closable session resource.
pub trait SessionResource {
    /// Release the resource; closing twice succeeds without running cleanups again.
    fn close(&mut self) -> Result<(), WorldError>;
}

/// Reverse-acquisition cleanup scope: cleanups run last-in-first-out, like
/// the Q3 host's deferred resource cleanup.
pub struct ResourceScope {
    name: String,
    cleanups: Vec<Box<dyn FnOnce() -> Result<(), WorldError>>>,
    closed: bool,
}

impl ResourceScope {
    /// Open a scope.
    #[must_use]
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            cleanups: Vec::new(),
            closed: false,
        }
    }

    /// Scope name used in failure messages.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether the scope is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Fail when the scope is closed.
    pub fn assert_open(&self) -> Result<(), WorldError> {
        if self.closed {
            return Err(WorldError::ResourceClosed(self.name.clone()));
        }
        Ok(())
    }

    /// Defer a cleanup to scope close.
    pub fn defer<F>(&mut self, cleanup: F) -> Result<(), WorldError>
    where
        F: FnOnce() -> Result<(), WorldError> + 'static,
    {
        self.assert_open()?;
        self.cleanups.push(Box::new(cleanup));
        Ok(())
    }

    /// Own a shared resource: its close runs at scope close, after later
    /// cleanups. Returns the same handle. Callers must not hold a borrow of
    /// the resource across scope close.
    pub fn own<T>(&mut self, resource: Rc<RefCell<T>>) -> Result<Rc<RefCell<T>>, WorldError>
    where
        T: SessionResource + 'static,
    {
        self.assert_open()?;
        let owned = resource.clone();
        self.cleanups.push(Box::new(move || owned.borrow_mut().close()));
        Ok(resource)
    }

    /// Run cleanups last-in-first-out, aggregating failures. Idempotent.
    pub fn close(&mut self) -> Result<(), WorldError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut errors = Vec::new();
        while let Some(cleanup) = self.cleanups.pop() {
            if let Err(error) = cleanup() {
                errors.push(error.to_string());
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(WorldError::CloseFailed {
                name: self.name.clone(),
                errors,
            })
        }
    }
}

impl SessionResource for ResourceScope {
    fn close(&mut self) -> Result<(), WorldError> {
        ResourceScope::close(self)
    }
}

impl Drop for ResourceScope {
    /// Backstop: run unclaimed cleanups without reporting failures.
    /// Prefer explicit [`ResourceScope::close`] so failures surface.
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        while let Some(cleanup) = self.cleanups.pop() {
            let _ = cleanup();
        }
    }
}

// Engine session lifecycle (donor `src/world/session/session.ts`; lifecycle
// surface only: `new`, `attachWorld`, `createClient`, `close`, plus the
// `SessionClient` id. Connections, seats, stepping, and presentations land
// with their owners. Rust ownership replaces two donor checks: an attached
// simulation is moved into the session so it cannot attach twice, and the
// simulation keeps its own internal registry owner until the full port
// reconciles simulation identity with the session.

/// Session mode: headless simulation or local play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMode {
    /// Headless simulation without local seats.
    Headless,
    /// Local play; seats attach through the seat-management surface.
    Local,
}

/// A connected client handle with its own resource scope.
pub struct SessionClient {
    id: ClientId,
    resources: ResourceScope,
}

impl SessionClient {
    /// Wrap a minted client handle.
    #[must_use]
    pub fn new(id: ClientId) -> Self {
        Self {
            id,
            resources: ResourceScope::new("Client resources"),
        }
    }

    /// Client handle.
    #[must_use]
    pub fn id(&self) -> &ClientId {
        &self.id
    }

    /// Whether the client is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.resources.is_closed()
    }

    /// Client resource scope.
    #[must_use]
    pub fn resources(&self) -> &ResourceScope {
        &self.resources
    }

    /// Mutable client resource scope.
    pub fn resources_mut(&mut self) -> &mut ResourceScope {
        &mut self.resources
    }

    /// Release the client.
    pub fn close(&mut self) -> Result<(), WorldError> {
        self.resources.close()
    }
}

impl SessionResource for SessionClient {
    fn close(&mut self) -> Result<(), WorldError> {
        SessionClient::close(self)
    }
}

/// An attached world: one simulation plus its resource scope.
pub struct WorldLifetime {
    simulation: Simulation,
    resources: ResourceScope,
}

impl WorldLifetime {
    /// Attach a simulation to a fresh resource scope.
    #[must_use]
    pub fn new(simulation: Simulation) -> Self {
        Self {
            simulation,
            resources: ResourceScope::new("World resources"),
        }
    }

    /// Attached simulation.
    #[must_use]
    pub fn simulation(&self) -> &Simulation {
        &self.simulation
    }

    /// Mutable attached simulation.
    pub fn simulation_mut(&mut self) -> &mut Simulation {
        &mut self.simulation
    }

    /// World resource scope.
    #[must_use]
    pub fn resources(&self) -> &ResourceScope {
        &self.resources
    }

    /// Mutable world resource scope.
    pub fn resources_mut(&mut self) -> &mut ResourceScope {
        &mut self.resources
    }

    /// Whether the world is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.resources.is_closed()
    }

    /// Shut the world down: simulation first, then its resources.
    pub fn close(&mut self) -> Result<(), WorldError> {
        self.simulation.close();
        self.resources.close()
    }
}

impl SessionResource for WorldLifetime {
    fn close(&mut self) -> Result<(), WorldError> {
        WorldLifetime::close(self)
    }
}

impl SessionResource for Simulation {
    fn close(&mut self) -> Result<(), WorldError> {
        Simulation::close(self);
        Ok(())
    }
}

fn collect_close(errors: &mut Vec<String>, context: &str, result: Result<(), WorldError>) {
    if let Err(error) = result {
        match error {
            WorldError::CloseFailed { errors: members, name } => {
                if members.is_empty() {
                    errors.push(format!("{context}: failed to close {name}"));
                } else {
                    for member in members {
                        errors.push(format!("{context}: {member}"));
                    }
                }
            }
            other => errors.push(format!("{context}: {other}")),
        }
    }
}

/// One session calls one simulation.
pub struct EngineSession {
    identity: IdentityOwner,
    mode: SessionMode,
    resources: ResourceScope,
    clients: HashMap<u32, SessionClient>,
    client_order: Vec<u32>,
    generations: HashMap<u32, u32>,
    current_world: Option<WorldLifetime>,
}

impl EngineSession {
    /// Open a session; the identity authority moves into the session.
    #[must_use]
    pub fn new(identity: IdentityOwner, mode: SessionMode) -> Self {
        Self {
            identity,
            mode,
            resources: ResourceScope::new("Session resources"),
            clients: HashMap::new(),
            client_order: Vec::new(),
            generations: HashMap::new(),
            current_world: None,
        }
    }

    /// Session handle.
    #[must_use]
    pub fn session(&self) -> &SessionId {
        self.identity.session()
    }

    /// Session mode.
    #[must_use]
    pub fn mode(&self) -> SessionMode {
        self.mode
    }

    /// Whether the session is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.resources.is_closed()
    }

    /// Attached world, if any.
    #[must_use]
    pub fn world(&self) -> Option<&WorldLifetime> {
        self.current_world.as_ref()
    }

    /// Mutable attached world, if any.
    pub fn world_mut(&mut self) -> Option<&mut WorldLifetime> {
        self.current_world.as_mut()
    }

    /// Live client occupying a slot, if any.
    #[must_use]
    pub fn client_at(&self, slot: u32) -> Option<&SessionClient> {
        self.clients.get(&slot).filter(|client| !client.is_closed())
    }

    /// Attach a simulation, closing the retired world. The replacement stays
    /// installed even when the retired world fails to close.
    pub fn attach_world(&mut self, simulation: Simulation) -> Result<(), WorldError> {
        self.resources.assert_open()?;
        let mut retired = self.current_world.replace(WorldLifetime::new(simulation));
        if let Some(previous) = retired.as_mut() {
            previous.close()?;
        }
        Ok(())
    }

    /// Create a client in a free slot, returning its minted handle. Callers
    /// use [`EngineSession::client_at`] for access so the session stays
    /// mutable; the donor returns the live object.
    pub fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError> {
        self.resources.assert_open()?;
        if self.clients.get(&slot).is_some_and(|client| !client.is_closed()) {
            return Err(WorldError::ClientSlotOccupied(slot));
        }
        let generation = match self.generations.get(&slot) {
            None => 0,
            Some(previous) => previous.checked_add(1).ok_or(WorldError::ClientGenerationExhausted)?,
        };
        let id = self.identity.client(slot, generation);
        self.clients.insert(slot, SessionClient::new(id.clone()));
        self.generations.insert(slot, generation);
        if !self.client_order.contains(&slot) {
            self.client_order.push(slot);
        }
        Ok(id)
    }

    /// Shut the session down: session cleanups, then the world, then clients
    /// in reverse insertion order. Idempotent.
    pub fn close(&mut self) -> Result<(), WorldError> {
        if self.resources.is_closed() {
            return Ok(());
        }
        let mut errors = Vec::new();
        collect_close(&mut errors, "session", self.resources.close());
        if let Some(world) = self.current_world.as_mut() {
            collect_close(&mut errors, "world", world.close());
        }
        for slot in self.client_order.iter().rev() {
            if let Some(client) = self.clients.get_mut(slot) {
                collect_close(&mut errors, &format!("client {}", slot), client.close());
            }
        }
        self.clients.clear();
        self.client_order.clear();
        self.current_world = None;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(WorldError::CloseFailed {
                name: "Session resources".to_string(),
                errors,
            })
        }
    }
}

impl SessionResource for EngineSession {
    fn close(&mut self) -> Result<(), WorldError> {
        EngineSession::close(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn q3() -> (ProviderId, ClockProfile) {
        (
            ProviderId::new("q3", "game"),
            ClockProfile::Q3 {
                server_frame_milliseconds: 50.0,
                fixed_movement_milliseconds: None,
            },
        )
    }

    #[test]
    fn spawn_step_and_status_commands() {
        let (primary, profile) = q3();
        let mut simulation = Simulation::new("test", primary, profile, SourceTime::Milliseconds(0), 8).unwrap();
        let spawned = simulation.execute("spawn q3 game q3:player", Dialect::Q3).unwrap();
        assert!(spawned.contains("slot 0"));
        let status = simulation.execute("status", Dialect::Q3).unwrap();
        assert!(status.contains("actors 1"));
        let ticked = simulation.execute("tick 50", Dialect::Q3).unwrap();
        assert!(ticked.contains("frame 1"));
        assert!(simulation.execute("frobnicate", Dialect::Q3).is_err());
    }

    #[test]
    fn thinks_fire_and_checkpoint_round_trip() {
        let (primary, profile) = q3();
        let mut simulation =
            Simulation::new("test", primary.clone(), profile, SourceTime::Milliseconds(1000), 8).unwrap();
        simulation.execute("spawn q3 game q3:item", Dialect::Q3).unwrap();
        simulation
            .execute("think 0 q3:respawn 1000 before", Dialect::Q3)
            .unwrap();
        let fired = Rc::new(RefCell::new(0));
        let seen = fired.clone();
        simulation.bind_think(
            "q3:respawn",
            Rc::new(move |_, _, _| {
                *seen.borrow_mut() += 1;
            }),
        );
        let image = simulation.checkpoint().unwrap();
        assert_eq!(image.thinks.len(), 1);
        let mut restored = Simulation::restore("restored", primary, profile, image, 8).unwrap();
        let seen = fired.clone();
        restored.bind_think(
            "q3:respawn",
            Rc::new(move |_, _, _| {
                *seen.borrow_mut() += 1;
            }),
        );
        let output = restored.step(SourceTime::Milliseconds(50)).unwrap();
        assert_eq!(*fired.borrow(), 1);
        assert_eq!(output.events.len(), 1);
        assert!(matches!(output.events[0].payload, SimEventPayload::Think { .. }));
    }

    #[test]
    fn hurt_uses_explicit_family_floors() {
        let (primary, profile) = q3();
        let mut simulation = Simulation::new("test", primary, profile, SourceTime::Milliseconds(0), 8).unwrap();
        simulation.execute("spawn q1 game q1:ogre", Dialect::Q3).unwrap();
        let health = simulation.execute("hurt 0 200 q1", Dialect::Q3).unwrap();
        assert!(health.contains("-99"));
        let health = simulation.execute("hurt 0 5000 q3", Dialect::Q3).unwrap();
        assert!(health.contains("-999"));
    }

    #[test]
    fn transitions_gate_campaigns_and_order_rounds() {
        let campaign = ProviderId::new("q1", "campaign");
        let match_id = ProviderId::new("q3", "match");
        let mut coordinator = TransitionCoordinator::new();
        let mode = TransitionMode::Combined {
            campaign: campaign.clone(),
            match_id: match_id.clone(),
            round_first: false,
        };
        let blocked = coordinator.resolve(
            &mode,
            &[TransitionIntent::CampaignLevel {
                campaign: campaign.clone(),
                map: "q1:e1m2".to_string(),
                spawn: "start".to_string(),
                gates: vec![MissionGate {
                    objective: "q1:key".to_string(),
                    satisfied: false,
                }],
            }],
        );
        assert!(matches!(blocked, TransitionDecision::Stay { .. }));
        let travel = coordinator.resolve(
            &mode,
            &[
                TransitionIntent::MatchRotation {
                    match_id: match_id.clone(),
                    map: "q3:q3dm1".to_string(),
                },
                TransitionIntent::CampaignLevel {
                    campaign: campaign.clone(),
                    map: "q1:e1m2".to_string(),
                    spawn: "start".to_string(),
                    gates: Vec::new(),
                },
            ],
        );
        assert!(matches!(travel, TransitionDecision::Travel { .. }));
        let mut applied = 0;
        coordinator.commit(&travel, &mut |_| applied += 1).unwrap();
        assert_eq!(applied, 1);
        assert_eq!(
            coordinator.commit(&travel, &mut |_| {}),
            Err(WorldError::TransitionStale)
        );
    }

    #[test]
    fn resource_scope_closes_last_in_first_out() {
        let mut scope = ResourceScope::new("test");
        let order = Rc::new(RefCell::new(Vec::new()));
        for name in ["first", "second", "third"] {
            let order = order.clone();
            scope
                .defer(move || {
                    order.borrow_mut().push(name);
                    Ok(())
                })
                .unwrap();
        }
        scope.close().unwrap();
        assert_eq!(*order.borrow(), vec!["third", "second", "first"]);
        assert!(scope.is_closed());
        scope.close().unwrap();
        assert_eq!(order.borrow().len(), 3);
    }

    #[test]
    fn resource_scope_aggregates_close_failures() {
        let mut scope = ResourceScope::new("flaky");
        scope
            .defer(|| Err(WorldError::CommandUsage("first failure".to_string())))
            .unwrap();
        scope.defer(|| Ok(())).unwrap();
        scope
            .defer(|| Err(WorldError::CommandUsage("second failure".to_string())))
            .unwrap();
        let error = scope.close().unwrap_err();
        assert_eq!(
            error,
            WorldError::CloseFailed {
                name: "flaky".to_string(),
                errors: vec!["second failure".to_string(), "first failure".to_string()],
            }
        );
        assert_eq!(error.to_string(), "Failed to close flaky");
        assert_eq!(
            scope.defer(|| Ok(())),
            Err(WorldError::ResourceClosed("flaky".to_string()))
        );
    }

    #[test]
    fn resource_scope_owns_shared_resources() {
        let mut scope = ResourceScope::new("owner");
        let owned = Rc::new(RefCell::new(ResourceScope::new("child")));
        let closed = Rc::new(RefCell::new(false));
        let seen = closed.clone();
        owned
            .borrow_mut()
            .defer(move || {
                *seen.borrow_mut() = true;
                Ok(())
            })
            .unwrap();
        let handle = scope.own(owned).unwrap();
        assert!(!handle.borrow().is_closed());
        scope.close().unwrap();
        assert!(*closed.borrow());
        assert!(handle.borrow().is_closed());
    }

    fn engine(mode: SessionMode) -> EngineSession {
        EngineSession::new(IdentityOwner::create("test").unwrap(), mode)
    }

    fn headless_simulation() -> Simulation {
        let (primary, profile) = q3();
        Simulation::new("test", primary, profile, SourceTime::Milliseconds(0), 8).unwrap()
    }

    #[test]
    fn engine_creates_generational_clients() {
        let mut session = engine(SessionMode::Local);
        assert_eq!(session.mode(), SessionMode::Local);
        assert!(!session.is_closed());
        let first = session.create_client(0).unwrap();
        assert_eq!(first.slot(), 0);
        assert_eq!(first.generation(), 0);
        let second = session.create_client(1).unwrap();
        assert_eq!(second.slot(), 1);
        assert_eq!(session.client_at(0).unwrap().id(), &first);
        assert_eq!(session.create_client(0).unwrap_err(), WorldError::ClientSlotOccupied(0));
        assert!(session.client_at(2).is_none());
        session.close().unwrap();
        assert!(session.is_closed());
        assert!(session.client_at(0).is_none());
        session.close().unwrap();
        let _ = second;
    }

    #[test]
    fn engine_attaches_worlds_and_retires_old() {
        let mut session = engine(SessionMode::Headless);
        assert!(session.world().is_none());
        session.attach_world(headless_simulation()).unwrap();
        assert!(session.world().is_some());
        let retired = Rc::new(RefCell::new(false));
        let seen = retired.clone();
        session
            .world_mut()
            .unwrap()
            .resources_mut()
            .defer(move || {
                *seen.borrow_mut() = true;
                Ok(())
            })
            .unwrap();
        session.attach_world(headless_simulation()).unwrap();
        assert!(*retired.borrow());
        assert!(session.world().is_some());
        session.close().unwrap();
        assert!(session.world().is_none());
        assert_eq!(
            session.attach_world(headless_simulation()).unwrap_err(),
            WorldError::ResourceClosed("Session resources".to_string())
        );
    }

    #[test]
    fn engine_close_aggregates_client_failures() {
        let mut session = engine(SessionMode::Local);
        session.create_client(0).unwrap();
        session.create_client(1).unwrap();
        session
            .clients
            .get_mut(&1)
            .unwrap()
            .resources_mut()
            .defer(|| Err(WorldError::CommandUsage("client 1 cleanup".to_string())))
            .unwrap();
        let error = session.close().unwrap_err();
        assert_eq!(
            error,
            WorldError::CloseFailed {
                name: "Session resources".to_string(),
                errors: vec!["client 1: client 1 cleanup".to_string()],
            }
        );
        assert!(session.is_closed());
    }
}
