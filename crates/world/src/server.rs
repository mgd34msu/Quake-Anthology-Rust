//! Deterministic server tick: fixed-step simulation over [`Simulation`].
//! The caller supplies the timestep (donor `SourceClock`: "no renderer
//! tick or protocol chooses it"); the [`TickPlan`] derives per-family
//! steps from [`ClockProfile`]: Q2 classic runs a 100ms server frame, Q2
//! rerelease and Q3 use their configured frame milliseconds, and Q1
//! clamps variable frames (or takes a forced fixed step). Each frame
//! applies queued client commands in slot order, steps thinks, advances
//! timers, moves doors/plats/pushers, and sweeps triggers — all in slot
//! order so replays are bit-identical.

use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::time::{ClockProfile, FrameContext, SourceTime};

use crate::body::{BodyState, LinkedBody};
use crate::client::ClientCommand;
use crate::inventory::InventoryEntry;
use crate::movers::{step_mover, MoverPhase, MoverState, MoverTable};
use crate::registry::ActorRegistry;
use crate::session::{SaveImage, SimEvent, Simulation};
use crate::spatial::{ActorCollision, CollisionFamily, CollisionRole, CollisionShape, SpatialIndex};
use crate::spawn::{SpawnFields, SpawnRegistry};
use crate::timers::{TimerCheckpoint, TimerFired, TimerTable};
use crate::triggers::{touch_q1_triggers, TouchContact, TriggerTable};
use crate::WorldError;

/// Maximum fixed steps per [`Server::tick`] call; leftover time stays in
/// the accumulator for the next call.
pub const MAX_STEPS_PER_TICK: u32 = 8;
/// Server checkpoint schema version.
pub const SERVER_SCHEMA_VERSION: u32 = 1;

/// Per-frame step plan derived from a clock profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TickPlan {
    /// Fixed step; elapsed time accumulates and runs whole steps.
    Fixed {
        /// Step length.
        step: SourceTime,
    },
    /// Variable step clamped into `[min_seconds, max_seconds]`.
    Clamped {
        /// Minimum frame length in seconds.
        min_seconds: f64,
        /// Maximum frame length in seconds.
        max_seconds: f64,
    },
}

/// Derive a tick plan from a clock profile.
pub fn plan_for_profile(profile: &ClockProfile) -> Result<TickPlan, WorldError> {
    match *profile {
        ClockProfile::Q1Netquake {
            minimum_frame_seconds,
            maximum_frame_seconds,
            fixed_frame_seconds,
        } => {
            if let Some(step) = fixed_frame_seconds {
                if !step.is_finite() || step <= 0.0 {
                    return Err(WorldError::BadFixedStep);
                }
                Ok(TickPlan::Fixed {
                    step: SourceTime::Seconds(step as f32),
                })
            } else {
                if !minimum_frame_seconds.is_finite()
                    || !maximum_frame_seconds.is_finite()
                    || minimum_frame_seconds < 0.0
                    || maximum_frame_seconds <= 0.0
                    || minimum_frame_seconds > maximum_frame_seconds
                {
                    return Err(WorldError::BadFixedStep);
                }
                Ok(TickPlan::Clamped {
                    min_seconds: minimum_frame_seconds,
                    max_seconds: maximum_frame_seconds,
                })
            }
        }
        ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds,
        } => {
            if !maximum_command_milliseconds.is_finite() || maximum_command_milliseconds <= 0.0 {
                return Err(WorldError::BadFixedStep);
            }
            Ok(TickPlan::Clamped {
                min_seconds: 0.0,
                max_seconds: maximum_command_milliseconds / 1000.0,
            })
        }
        ClockProfile::Q2Classic => Ok(TickPlan::Fixed {
            step: SourceTime::Milliseconds(100),
        }),
        ClockProfile::Q2Rerelease { frame_milliseconds } => {
            if !frame_milliseconds.is_finite() || frame_milliseconds <= 0.0 {
                return Err(WorldError::BadFixedStep);
            }
            Ok(TickPlan::Fixed {
                step: SourceTime::Milliseconds(frame_milliseconds.trunc() as i32),
            })
        }
        ClockProfile::Q3 {
            server_frame_milliseconds,
            ..
        } => {
            if !server_frame_milliseconds.is_finite() || server_frame_milliseconds <= 0.0 {
                return Err(WorldError::BadFixedStep);
            }
            Ok(TickPlan::Fixed {
                step: SourceTime::Milliseconds(server_frame_milliseconds.trunc() as i32),
            })
        }
    }
}

/// Game-logic hooks driven by the server tick. The engine runs guest gamecode
/// ([`GuestServerLogic`](qa_guest::server::GuestServerLogic)) here; game
/// modules implement this trait, and [`NullLogic`] remains only as a test seed.
pub trait ServerLogic {
    /// Apply one queued client command to its actor.
    fn client_think(&mut self, _simulation: &mut Simulation, _slot: u32, _command: &ClientCommand) {}

    /// Run per-frame game logic after thinks.
    fn entity_frame(&mut self, _simulation: &mut Simulation, _frame: FrameContext) {}

    /// React to a trigger touch.
    fn touch(&mut self, _simulation: &mut Simulation, _contact: &TouchContact) {}

    /// React to a mover think crossing its local think time. `arrived` is
    /// true when the endpoint was reached on the same step (arrival think);
    /// a think at a resting endpoint is the wait think, where gamecode
    /// returns doors and plats via [`MoverTable`].
    fn mover_think(
        &mut self,
        _simulation: &mut Simulation,
        _movers: &mut MoverTable,
        _actor: &ActorId,
        _phase: MoverPhase,
        _arrived: bool,
    ) {
    }
}

/// No-op game logic.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullLogic;

impl ServerLogic for NullLogic {}

/// Server tick event.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    /// A fixed frame completed.
    Frame {
        /// Frame number.
        frame: i32,
    },
    /// A queued client command applied.
    ClientApplied {
        /// Client slot.
        slot: u32,
    },
    /// Simulation event from think/damage dispatch.
    Sim(SimEvent),
    /// Countdown timer fired.
    Timer(TimerFired),
    /// Trigger touched.
    Touch(TouchContact),
    /// Mover reached its endpoint.
    MoverArrived {
        /// Mover actor.
        actor: SavedActorId,
    },
    /// Mover think crossed its local think time.
    MoverThink {
        /// Mover actor.
        actor: SavedActorId,
    },
}

/// Output of one [`Server::tick`] call.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTick {
    /// Fixed frames executed.
    pub frames: u32,
    /// Events in frame order.
    pub events: Vec<ServerEvent>,
}

/// Bot command source polled at the start of each server frame.
///
/// Bot decisions (qa-bots behavior) enter the ordinary client pipeline:
/// generated commands queue alongside player input and apply in slot
/// order, so bot play replays bit-identically.
pub trait BotCommandSource {
    /// Generate bot client commands for the frame at `time_ms`.
    fn bot_commands(&mut self, time_ms: i32) -> Vec<(u32, ClientCommand)>;
}

/// Deterministic game server over a headless simulation.
pub struct Server<L: ServerLogic> {
    simulation: Simulation,
    logic: L,
    plan: TickPlan,
    accumulator: f64,
    timers: TimerTable,
    triggers: TriggerTable,
    movers: MoverTable,
    spawns: SpawnRegistry,
    queue: Vec<(u32, ClientCommand)>,
    bots: Option<Box<dyn BotCommandSource>>,
    game_provider: ProviderId,
    default_bounds: Bounds,
    spatial_bounds: Bounds,
    closed: bool,
}

impl<L: ServerLogic> Server<L> {
    /// Build a server over an existing simulation.
    #[must_use]
    pub fn new(
        simulation: Simulation,
        logic: L,
        plan: TickPlan,
        game_provider: ProviderId,
        default_bounds: Bounds,
        spatial_bounds: Bounds,
    ) -> Self {
        Self {
            simulation,
            logic,
            plan,
            accumulator: 0.0,
            timers: TimerTable::new(),
            triggers: TriggerTable::new(),
            movers: MoverTable::new(),
            spawns: SpawnRegistry::new(),
            queue: Vec::new(),
            bots: None,
            game_provider,
            default_bounds,
            spatial_bounds,
            closed: false,
        }
    }

    /// Borrow the simulation.
    #[must_use]
    pub fn simulation(&self) -> &Simulation {
        &self.simulation
    }

    /// Borrow the simulation mutably.
    pub fn simulation_mut(&mut self) -> &mut Simulation {
        &mut self.simulation
    }

    /// Borrow the game logic.
    #[must_use]
    pub fn logic(&self) -> &L {
        &self.logic
    }

    /// Borrow the game logic mutably.
    pub fn logic_mut(&mut self) -> &mut L {
        &mut self.logic
    }

    /// Tick plan.
    #[must_use]
    pub const fn plan(&self) -> TickPlan {
        self.plan
    }

    /// Borrow the timer table mutably.
    pub fn timers_mut(&mut self) -> &mut TimerTable {
        &mut self.timers
    }

    /// Borrow the trigger table mutably.
    pub fn triggers_mut(&mut self) -> &mut TriggerTable {
        &mut self.triggers
    }

    /// Borrow the mover table mutably.
    pub fn movers_mut(&mut self) -> &mut MoverTable {
        &mut self.movers
    }

    /// Borrow the spawn registry mutably.
    pub fn spawns_mut(&mut self) -> &mut SpawnRegistry {
        &mut self.spawns
    }

    /// Queue a client command for the next frame.
    pub fn queue_client(&mut self, slot: u32, command: ClientCommand) -> Result<(), WorldError> {
        self.assert_open()?;
        self.queue.push((slot, command));
        Ok(())
    }

    /// Attach a bot command source, polled at the start of each frame.
    pub fn set_bot_source(&mut self, source: Option<Box<dyn BotCommandSource>>) {
        self.bots = source;
    }

    /// Whether a bot command source is attached.
    #[must_use]
    pub fn has_bot_source(&self) -> bool {
        self.bots.is_some()
    }

    /// Spawn a map entity through the registered spawn function.
    pub fn spawn_entity(&mut self, fields: &SpawnFields) -> Result<OwnedActor, WorldError> {
        self.assert_open()?;
        let request = self.spawns.spawn(fields)?;
        let body = request.origin.map(|origin| BodyState {
            origin,
            angles: fields.angles,
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: self.default_bounds,
            ground: None,
        });
        let inventory: Vec<InventoryEntry> = request
            .grants
            .iter()
            .map(|(item, count)| InventoryEntry {
                item: item.clone(),
                count: *count,
                capacity: *count,
                count_policy: None,
            })
            .collect();
        self.simulation.spawn(
            self.game_provider.clone(),
            &request.definition,
            body,
            request.combat,
            inventory,
        )
    }

    /// Advance the server by `elapsed` source time.
    pub fn tick(&mut self, elapsed: SourceTime) -> Result<ServerTick, WorldError> {
        self.assert_open()?;
        match self.plan {
            TickPlan::Fixed { step } => {
                let step_value = plan_value(step)?;
                let elapsed_value = same_unit_value(step, elapsed)?;
                if elapsed_value < 0.0 {
                    return Err(WorldError::NegativeTime);
                }
                self.accumulator += elapsed_value;
                let mut events = Vec::new();
                let mut frames = 0;
                while self.accumulator >= step_value && frames < MAX_STEPS_PER_TICK {
                    self.accumulator -= step_value;
                    events.extend(self.run_frame(step)?);
                    frames += 1;
                }
                Ok(ServerTick { frames, events })
            }
            TickPlan::Clamped {
                min_seconds,
                max_seconds,
            } => {
                let seconds = elapsed.as_seconds_f64();
                if !seconds.is_finite() {
                    return Err(WorldError::NonFiniteTime);
                }
                if seconds < 0.0 {
                    return Err(WorldError::NegativeTime);
                }
                let clamped = seconds.clamp(min_seconds, max_seconds);
                let step = match elapsed {
                    SourceTime::Seconds(_) => SourceTime::Seconds(clamped as f32),
                    SourceTime::Milliseconds(_) => SourceTime::Milliseconds((clamped * 1000.0) as i32),
                };
                let events = self.run_frame(step)?;
                Ok(ServerTick { frames: 1, events })
            }
        }
    }

    fn run_frame(&mut self, step: SourceTime) -> Result<Vec<ServerEvent>, WorldError> {
        let mut events = Vec::new();
        if let Some(source) = self.bots.as_mut() {
            let time_ms = self.simulation.frame().time.as_milliseconds_truncated();
            for (slot, command) in source.bot_commands(time_ms) {
                self.queue.push((slot, command));
            }
        }
        let mut queued = std::mem::take(&mut self.queue);
        queued.sort_by_key(|(slot, _)| *slot);
        for (slot, command) in &queued {
            if self.simulation.actor_by_slot(*slot).is_some() {
                self.logic.client_think(&mut self.simulation, *slot, command);
                events.push(ServerEvent::ClientApplied { slot: *slot });
            }
        }
        let output = self.simulation.step(step)?;
        self.logic.entity_frame(&mut self.simulation, output.snapshot.frame);
        for event in output.events {
            events.push(ServerEvent::Sim(event));
        }
        let now = self.simulation.frame().time;
        for fired in self.timers.advance(self.simulation.registry(), now) {
            events.push(ServerEvent::Timer(fired));
        }
        self.step_movers(&mut events)?;
        self.sweep_triggers(&mut events)?;
        events.push(ServerEvent::Frame {
            frame: self.simulation.frame().frame,
        });
        Ok(events)
    }

    fn step_movers(&mut self, events: &mut Vec<ServerEvent>) -> Result<(), WorldError> {
        let actors: Vec<ActorId> = self
            .simulation
            .body_actors()
            .into_iter()
            .filter(|actor| self.movers.get(actor).is_some())
            .collect();
        let elapsed = self.simulation.frame().elapsed.as_seconds_f64();
        for actor in actors {
            let Some(origin) = self.simulation.body_state(&actor).map(|state| state.origin) else {
                continue;
            };
            let Some(mover) = self.movers.get_mut(&actor) else {
                continue;
            };
            let step = step_mover(mover, origin, elapsed);
            let phase = mover.phase;
            let next = Vec3 {
                x: origin.x + step.displacement.x,
                y: origin.y + step.displacement.y,
                z: origin.z + step.displacement.z,
            };
            self.simulation.set_body_origin(&actor, next)?;
            let saved = SavedActorId::from(&actor);
            if step.think_due {
                self.logic
                    .mover_think(&mut self.simulation, &mut self.movers, &actor, phase, step.arrived);
                events.push(ServerEvent::MoverThink { actor: saved });
            }
            if step.arrived {
                events.push(ServerEvent::MoverArrived { actor: saved });
            }
        }
        Ok(())
    }

    fn sweep_triggers(&mut self, events: &mut Vec<ServerEvent>) -> Result<(), WorldError> {
        for actor in self.simulation.body_actors() {
            self.simulation.link_body(&actor)?;
        }
        let mut spatial = SpatialIndex::new(&self.spatial_bounds);
        for actor in self.simulation.body_actors() {
            let Some(linked) = self.simulation.bodies().linked(self.simulation.registry(), &actor) else {
                continue;
            };
            let role = if self.triggers.is_trigger(&actor) {
                CollisionRole::Trigger
            } else {
                CollisionRole::Solid
            };
            spatial.link(
                &LinkedBody {
                    actor: linked.actor.clone(),
                    state: linked.state.clone(),
                    absolute_bounds: linked.absolute_bounds,
                    link_count: linked.link_count,
                },
                &ActorCollision {
                    family: CollisionFamily::Q1,
                    shape: CollisionShape::Box,
                    contents: 0,
                    owner: None,
                    role,
                    monster: false,
                    dead_monster: false,
                    q1_corpse: false,
                    q3_owner: None,
                },
            );
        }
        let movers = self.simulation.body_actors();
        let mut contacts = Vec::new();
        for mover in &movers {
            if self.triggers.is_trigger(mover) {
                continue;
            }
            touch_q1_triggers(
                self.simulation.registry(),
                self.simulation.bodies(),
                &spatial,
                &self.triggers,
                mover,
                &mut |contact| contacts.push(contact),
            );
        }
        for contact in contacts {
            self.logic.touch(&mut self.simulation, &contact);
            events.push(ServerEvent::Touch(contact));
        }
        Ok(())
    }

    /// Checkpoint the server.
    pub fn save(&self) -> Result<ServerCheckpoint, WorldError> {
        self.assert_open()?;
        Ok(ServerCheckpoint {
            schema_version: SERVER_SCHEMA_VERSION,
            save: self.simulation.checkpoint()?,
            timers: self.timers.checkpoint(),
            triggers: self.triggers.checkpoint(),
            movers: self.movers.checkpoint(),
            accumulator: self.accumulator,
        })
    }

    /// Restore a server from a checkpoint.
    pub fn restore(checkpoint: ServerCheckpoint, params: RestoreParams<L>) -> Result<Self, WorldError> {
        if checkpoint.schema_version != SERVER_SCHEMA_VERSION {
            return Err(WorldError::BadSave("Unsupported server save schema".to_string()));
        }
        if !checkpoint.accumulator.is_finite() || checkpoint.accumulator < 0.0 {
            return Err(WorldError::BadSave("Invalid server accumulator".to_string()));
        }
        let simulation = Simulation::restore(
            &params.session_name,
            params.primary,
            params.profile,
            checkpoint.save,
            params.capacity,
        )?;
        let mut server = Self::new(
            simulation,
            params.logic,
            params.plan,
            params.game_provider,
            params.default_bounds,
            params.spatial_bounds,
        );
        server.accumulator = checkpoint.accumulator;
        server
            .timers
            .restore(server.simulation.registry(), &checkpoint.timers)?;
        server
            .triggers
            .restore(server.simulation.registry(), &checkpoint.triggers)?;
        for ((slot, generation), state) in &checkpoint.movers {
            let actor = find_actor(server.simulation.registry(), *slot, *generation)?;
            server.movers.insert(actor, state.clone());
        }
        Ok(server)
    }

    fn assert_open(&self) -> Result<(), WorldError> {
        if self.closed {
            return Err(WorldError::ServerClosed);
        }
        Ok(())
    }

    /// Close the server.
    pub fn close(&mut self) {
        self.closed = true;
        self.simulation.close();
    }
}

fn find_actor(registry: &ActorRegistry, slot: u32, generation: u32) -> Result<ActorId, WorldError> {
    registry
        .observations()
        .into_iter()
        .find(|observed| observed.id.slot() == slot && observed.id.generation() == generation)
        .map(|observed| observed.id)
        .ok_or_else(|| WorldError::BadSave("Mover names a missing actor".to_string()))
}

fn plan_value(step: SourceTime) -> Result<f64, WorldError> {
    let value = match step {
        SourceTime::Seconds(value) => f64::from(value),
        SourceTime::Milliseconds(value) => f64::from(value),
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(WorldError::BadFixedStep);
    }
    Ok(value)
}

fn same_unit_value(step: SourceTime, elapsed: SourceTime) -> Result<f64, WorldError> {
    match (step, elapsed) {
        (SourceTime::Seconds(_), SourceTime::Seconds(value)) => Ok(f64::from(value)),
        (SourceTime::Milliseconds(_), SourceTime::Milliseconds(value)) => Ok(f64::from(value)),
        _ => Err(WorldError::TimeUnit),
    }
}

/// Server restore parameters.
#[derive(Debug, Clone)]
pub struct RestoreParams<L: ServerLogic> {
    /// Session name for the fresh authority.
    pub session_name: String,
    /// Primary provider.
    pub primary: ProviderId,
    /// Clock profile.
    pub profile: ClockProfile,
    /// Actor capacity.
    pub capacity: usize,
    /// Game logic.
    pub logic: L,
    /// Tick plan.
    pub plan: TickPlan,
    /// Game provider for spawned entities.
    pub game_provider: ProviderId,
    /// Default bounds for spawned bodies.
    pub default_bounds: Bounds,
    /// Spatial bounds for the trigger sweep.
    pub spatial_bounds: Bounds,
}

/// Saved server image.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerCheckpoint {
    /// Schema version.
    pub schema_version: u32,
    /// Simulation save image.
    pub save: SaveImage,
    /// Countdown timers.
    pub timers: Vec<TimerCheckpoint>,
    /// Marked triggers.
    pub triggers: Vec<(u32, u32)>,
    /// Movers.
    pub movers: Vec<((u32, u32), MoverState)>,
    /// Fixed-step accumulator in plan units.
    pub accumulator: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    use crate::client::ClientFamily;

    fn bounds(half: f32) -> Bounds {
        Bounds {
            min: vec3(-half, -half, -half),
            max: vec3(half, half, half),
        }
    }

    fn q3_server() -> Server<NullLogic> {
        let primary = ProviderId::new("q3", "game");
        let profile = ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        };
        let plan = plan_for_profile(&profile).unwrap();
        let simulation = Simulation::new("test", primary.clone(), profile, SourceTime::Milliseconds(0), 8).unwrap();
        Server::new(simulation, NullLogic, plan, primary, bounds(16.0), bounds(1024.0))
    }

    #[test]
    fn plans_match_family_steps() {
        let q2 = plan_for_profile(&ClockProfile::Q2Classic).unwrap();
        assert_eq!(
            q2,
            TickPlan::Fixed {
                step: SourceTime::Milliseconds(100)
            }
        );
        let q3 = plan_for_profile(&ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        })
        .unwrap();
        assert_eq!(
            q3,
            TickPlan::Fixed {
                step: SourceTime::Milliseconds(50)
            }
        );
        let q1 = plan_for_profile(&ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        })
        .unwrap();
        assert!(matches!(q1, TickPlan::Clamped { .. }));
        let fixed = plan_for_profile(&ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: Some(0.05),
        })
        .unwrap();
        assert_eq!(
            fixed,
            TickPlan::Fixed {
                step: SourceTime::Seconds(0.05)
            }
        );
        assert!(plan_for_profile(&ClockProfile::Q2Rerelease {
            frame_milliseconds: 0.0
        })
        .is_err());
    }

    #[test]
    fn fixed_tick_accumulates_and_runs_whole_steps() {
        let mut server = q3_server();
        let tick = server.tick(SourceTime::Milliseconds(30)).unwrap();
        assert_eq!(tick.frames, 0);
        let tick = server.tick(SourceTime::Milliseconds(30)).unwrap();
        assert_eq!(tick.frames, 1);
        assert_eq!(server.simulation().frame().frame, 1);
        let tick = server.tick(SourceTime::Milliseconds(120)).unwrap();
        assert_eq!(tick.frames, 2);
        assert_eq!(server.simulation().frame().frame, 3);
    }

    #[test]
    fn clamped_tick_runs_one_frame() {
        let primary = ProviderId::new("q1", "game");
        let profile = ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        };
        let plan = plan_for_profile(&profile).unwrap();
        let simulation = Simulation::new("test", primary.clone(), profile, SourceTime::Seconds(0.0), 8).unwrap();
        let mut server = Server::new(simulation, NullLogic, plan, primary, bounds(16.0), bounds(1024.0));
        let tick = server.tick(SourceTime::Seconds(1.0)).unwrap();
        assert_eq!(tick.frames, 1);
        assert_eq!(server.simulation().frame().elapsed, SourceTime::Seconds(0.1));
    }

    #[test]
    fn client_commands_apply_in_slot_order() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Recorder {
            slots: Rc<RefCell<Vec<u32>>>,
        }
        impl ServerLogic for Recorder {
            fn client_think(&mut self, _simulation: &mut Simulation, slot: u32, _command: &ClientCommand) {
                self.slots.borrow_mut().push(slot);
            }
        }
        let primary = ProviderId::new("q3", "game");
        let profile = ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        };
        let plan = plan_for_profile(&profile).unwrap();
        let simulation = Simulation::new("test", primary.clone(), profile, SourceTime::Milliseconds(0), 8).unwrap();
        let slots = Rc::new(RefCell::new(Vec::new()));
        let mut server = Server::new(
            simulation,
            Recorder { slots: slots.clone() },
            plan,
            primary,
            bounds(16.0),
            bounds(1024.0),
        );
        server
            .simulation_mut()
            .execute("spawn q3 game q3:player", qa_core::cmd::Dialect::Q3)
            .unwrap();
        server
            .simulation_mut()
            .execute("spawn q3 game q3:player", qa_core::cmd::Dialect::Q3)
            .unwrap();
        let command = ClientCommand::zero(ClientFamily::Q3);
        server.queue_client(1, command).unwrap();
        server.queue_client(0, command).unwrap();
        let tick = server.tick(SourceTime::Milliseconds(50)).unwrap();
        assert_eq!(*slots.borrow(), vec![0, 1]);
        assert!(tick
            .events
            .iter()
            .any(|event| matches!(event, ServerEvent::ClientApplied { slot: 0 })));
    }

    #[test]
    fn server_save_restore_round_trip() {
        let primary = ProviderId::new("q3", "game");
        let profile = ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        };
        let mut server = q3_server();
        server
            .simulation_mut()
            .execute("spawn q3 game q3:player", qa_core::cmd::Dialect::Q3)
            .unwrap();
        let actor = server.simulation().body_actors();
        assert!(actor.is_empty());
        server.tick(SourceTime::Milliseconds(50)).unwrap();
        let checkpoint = server.save().unwrap();
        assert_eq!(checkpoint.schema_version, SERVER_SCHEMA_VERSION);
        let plan = plan_for_profile(&profile).unwrap();
        let restored = Server::restore(
            checkpoint,
            RestoreParams {
                session_name: "restored".to_string(),
                primary: primary.clone(),
                profile,
                capacity: 8,
                logic: NullLogic,
                plan,
                game_provider: primary,
                default_bounds: bounds(16.0),
                spatial_bounds: bounds(1024.0),
            },
        )
        .unwrap();
        assert_eq!(restored.simulation().frame().frame, 1);
        assert_eq!(restored.simulation().actor_count(), 1);
    }

    #[test]
    fn spawn_entity_dispatches_registered_classname() {
        let mut server = q3_server();
        server.spawns_mut().register(
            "q3:item",
            Box::new(|fields| {
                Ok(crate::spawn::SpawnRequest {
                    definition: "q3:item".to_string(),
                    origin: Some(fields.origin),
                    combat: None,
                    grants: vec![("q3:shells".to_string(), 10.0)],
                })
            }),
        );
        let fields = SpawnFields::parse(&[("classname", "q3:item"), ("origin", "1 2 3")]).unwrap();
        let actor = server.spawn_entity(&fields).unwrap();
        let state = server.simulation().body_state(actor.id()).unwrap();
        assert_eq!(state.origin, vec3(1.0, 2.0, 3.0));
        let missing = SpawnFields::parse(&[("classname", "q3:nope")]).unwrap();
        assert!(server.spawn_entity(&missing).is_err());
    }
}
