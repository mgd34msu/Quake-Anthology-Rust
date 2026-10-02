//! Quake III source host: engine imports publishing to a session consumer.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/host.ts`
//! (`Q3SourceEvent`, `Q3HostOperations`, `Q3HostSettings`,
//! `createQ3SourceHost`).
//!
//! This module is the canonical home of [`Q3SourceEvent`]: the hub mirror at
//! `super::super::types` unifies with this definition post-merge. Note the
//! hub mirror's `Cinematic`/`Connect`/`Disconnect`/`Travel`/`Restart`/
//! `Record`/`StopRecord` variants have no donor basis (donor `host.ts` only
//! defines print/log, server-command, console-command, drop-client,
//! configstring, sound, entity-event, and player-event); only its
//! `Entity`/`Player` rows match this real type, so unification must replace
//! the mirror wholesale.
//!
//! Rust cannot spread a trait object the way the donor spreads `operations`,
//! so [`create_q3_source_host`] takes the operations handle plus the two
//! values the donor reads off neighbouring modules (the session id, since
//! the Rust session traits are minimal, and the server-cvar registration
//! seam from [`super::server_state`]) and returns a concrete
//! [`Q3SourceHostInstance`] implementing [`Q3SourceHost`].

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::q3::base::combat_bridge::VictimArmorContext;
use qa_content::q3::base::game::item_lifecycle::{
    OriginalPickupAdmission, SourcePickupAdmission, SourcePickupDescriptor, SourcePickupPreview,
};
use qa_content::q3::base::game::utilities::ConfigStringStore;
use qa_content::q3::base::records::EntityRef as RecordsEntityRef;
use qa_content::q3::base::records::{
    DamageRequest, Q3ActorCallbacks, Q3SessionActors, Q3SessionBodies, Q3SessionCombat, Q3SessionInventory,
};
use qa_content::q3::base::shared::player_state::UserCommand as Q3UserCommand;
use qa_content::q3::base::world_adapter::ActorCollision;
use qa_content::q3::foundation::character::Q3DeathAnimationSequence;
use qa_content::q3::team_arena::client_effects::ClientTimerOwnership;
use qa_content::q3::team_arena::client_spawn::SpawnPose;
use qa_content::q3::team_arena::movement_host::{ClientMovementOptions, ClientMovementResult, MovementHost};
use qa_content::q3::team_arena::support::EntityRef as SupportEntityRef;
use qa_core::cvar::{CvarArchiveEntry, CvarRegistry};
use qa_core::identity::{ActorId, OwnedActor, SessionId};
use qa_core::math::Vec3;
use qa_net::common::commands::ActorCommand;

use super::server_state::{Q3ServerCvarRegistration, Q3ServerState, Q3ServerStateOptions, Q3StoredUserCommand};
use super::types::{
    Q3ArsenalCategory, Q3SourceBots, Q3SourceEngine, Q3SourceEntityEvent, Q3SourceHost, Q3SourceMoverActors,
    Q3SourcePlayerEvent, Q3SourceScene,
};
use crate::bootstrap::simulation::types::CvarNameValue;

/// Console-command execution timing (donor `"append" | "now"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ConsoleExecution {
    /// Queue for later execution.
    Append,
    /// Execute immediately.
    Now,
}

/// Source engine event published to the session consumer.
///
/// Faithful port of donor `Q3SourceEvent`: print/log lines, server and
/// console commands, client drops, configstrings, sounds, and the
/// entity/player event rows.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Q3SourceEvent {
    /// Engine print line.
    Print {
        /// Text.
        text: String,
    },
    /// Engine log line.
    Log {
        /// Text.
        text: String,
    },
    /// Server command for a client.
    ServerCommand {
        /// Client slot.
        client: i32,
        /// Text.
        text: String,
    },
    /// Console command.
    ConsoleCommand {
        /// Execution timing.
        execution: Q3ConsoleExecution,
        /// Text.
        text: String,
    },
    /// Client drop.
    DropClient {
        /// Client slot.
        client: i32,
        /// Reason.
        reason: String,
    },
    /// Configstring write.
    Configstring {
        /// Index.
        index: i32,
        /// Value.
        value: String,
    },
    /// Positional sound.
    Sound {
        /// Acting actor.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Velocity.
        velocity: Vec3,
        /// Sound path.
        path: String,
        /// Channel.
        channel: i32,
        /// Volume.
        volume: f64,
        /// Looping flag.
        loop_sound: bool,
    },
    /// Entity event.
    Entity(Q3SourceEntityEvent),
    /// Player event.
    Player(Q3SourcePlayerEvent),
}

/// Operations supplied by the session, mirroring donor `Q3HostOperations`
/// (the [`Q3SourceHost`] surface minus engine, cvars, configstrings, bots,
/// entity events, and server state, plus bot services, event emission, and
/// client-number lookup).
pub trait Q3HostOperations: MovementHost {
    /// Ammunition timer store notification.
    fn ammo_timer_stored(&self, _actor: &ActorId, _weapon: usize, _value: i32) {}
    /// Per-actor timer ownership override.
    fn timer_ownership(&self, _actor: &ActorId) -> Option<ClientTimerOwnership> {
        None
    }
    /// Per-actor speed multiplier override.
    fn speed_multiplier(&self, _actor: &ActorId) -> Option<f32> {
        None
    }
    /// Session mover actors.
    fn mover_actors(&self) -> Rc<dyn Q3SourceMoverActors>;
    /// Whether primary attack is allowed.
    fn primary_attack_allowed(&self, _actor: &ActorId) -> bool {
        true
    }
    /// Grant the selected arsenal category.
    fn grant_selected_arsenal(&self, _actor: &ActorId, _category: Q3ArsenalCategory) -> bool {
        false
    }
    /// Give the selected item.
    fn give_selected_item(&self, _actor: &ActorId, _args: &[String]) -> bool {
        false
    }
    /// Preview a source pickup.
    fn preview_pickup(&self, _item: &SourcePickupDescriptor) -> SourcePickupPreview {
        SourcePickupPreview::Native
    }
    /// Admit a source pickup.
    fn admit_pickup(&self, _item: &SourcePickupDescriptor) -> SourcePickupAdmission {
        SourcePickupAdmission::Native
    }
    /// Session actors.
    fn actors(&self) -> Rc<dyn Q3SessionActors>;
    /// Shared bodies.
    fn bodies(&self) -> Rc<dyn Q3SessionBodies>;
    /// Actor callbacks.
    fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks>;
    /// Gameplay authority.
    fn combat(&self) -> Rc<dyn Q3SessionCombat>;
    /// Shared inventory.
    fn inventory(&self) -> Rc<dyn Q3SessionInventory>;
    /// Original pickup admission, if any.
    fn original_pickups(&self) -> Option<OriginalPickupAdmission> {
        None
    }
    /// Shared scene.
    fn scene(&self) -> Rc<dyn Q3SourceScene>;
    /// Death animation sequence.
    fn death_animations(&self) -> Q3DeathAnimationSequence;
    /// Bot services.
    fn bots(&self) -> Q3SourceBots<'static>;
    /// Current time in milliseconds.
    fn now(&self) -> i32;
    /// Schedule an actor think.
    fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>);
    /// Run an actor think.
    fn run_think(&self, actor: &OwnedActor, time_milliseconds: i32);
    /// Report an actor collision.
    fn collision(&self, actor: &OwnedActor, collision: ActorCollision);
    /// Victim armor context for a damage request.
    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext;
    /// Project a foreign actor into the source-slot view.
    fn foreign(&self, actor: &ActorId) -> Option<RecordsEntityRef>;
    /// Whether an actor is a player.
    fn is_player(&self, actor: &ActorId) -> bool;
    /// Convert command units for source policy.
    fn source_command(&self, input: &ActorCommand) -> Q3UserCommand;
    /// Spawn the selected player entity at a pose.
    fn spawn_player(&self, entity: &SupportEntityRef, pose: &SpawnPose);
    /// Emit a source event.
    fn emit(&self, event: Q3SourceEvent);
    /// Client number for an actor.
    fn client_number(&self, actor: &ActorId) -> i32;
}

/// Host construction settings, mirroring donor `Q3HostSettings`.
#[derive(Clone)]
pub struct Q3HostSettings {
    /// Game type.
    pub game_type: i32,
    /// Single-player flag.
    pub single_player: bool,
    /// Maximum clients.
    pub max_clients: usize,
    /// Map name.
    pub map_name: String,
    /// Adopted source registry, if any (shared handle, like the donor).
    pub source_registry: Option<Rc<RefCell<CvarRegistry>>>,
    /// Source archive applied when no registry is adopted.
    pub source_archive: Vec<CvarArchiveEntry>,
    /// Extra cvar assignments.
    pub cvars: Vec<CvarNameValue>,
}

impl std::fmt::Debug for Q3HostSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3HostSettings")
            .field("game_type", &self.game_type)
            .field("single_player", &self.single_player)
            .field("max_clients", &self.max_clients)
            .field("map_name", &self.map_name)
            .field("has_source_registry", &self.source_registry.is_some())
            .field("source_archive", &self.source_archive)
            .field("cvars", &self.cvars)
            .finish()
    }
}

/// Engine imports publishing to the session event consumer.
pub struct Q3HostEngine {
    operations: Rc<dyn Q3HostOperations>,
    server: Rc<Q3ServerState>,
}

impl Q3SourceEngine for Q3HostEngine {
    fn print(&self, text: &str) {
        self.operations.emit(Q3SourceEvent::Print { text: text.to_string() });
    }

    fn log(&self, text: &str) {
        self.operations.emit(Q3SourceEvent::Log { text: text.to_string() });
    }

    fn send_server_command(&self, client: i32, text: &str) {
        self.operations.emit(Q3SourceEvent::ServerCommand {
            client,
            text: text.to_string(),
        });
    }

    fn drop_client(&self, client: i32, reason: &str) {
        self.operations.emit(Q3SourceEvent::DropClient {
            client,
            reason: reason.to_string(),
        });
    }

    fn get_userinfo(&self, client: i32) -> String {
        self.server.get_userinfo(client).unwrap_or_else(|| {
            format!(
                "\\\\name\\\\Player {}\\\\ip\\\\localhost\\\\handicap\\\\100\\\\model\\\\sarge/default\\\\team_model\\\\sarge/default",
                client + 1
            )
        })
    }

    fn set_userinfo(&self, client: i32, value: &str) {
        self.server.set_userinfo(client, value);
    }

    fn get_user_command(&self, client: i32) -> Q3UserCommand {
        match self.server.get_user_command(client) {
            Some(command) => Q3UserCommand {
                server_time: command.server_time,
                angles: Vec3 {
                    x: command.angles[0] as f32,
                    y: command.angles[1] as f32,
                    z: command.angles[2] as f32,
                },
                buttons: command.buttons,
                weapon: command.weapon,
                forwardmove: command.forwardmove,
                rightmove: command.rightmove,
                upmove: command.upmove,
            },
            None => Q3UserCommand {
                server_time: self.operations.now(),
                angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                buttons: 0,
                weapon: 2,
                forwardmove: 0,
                rightmove: 0,
                upmove: 0,
            },
        }
    }

    fn append_console_command(&self, text: &str) {
        self.operations.emit(Q3SourceEvent::ConsoleCommand {
            execution: Q3ConsoleExecution::Append,
            text: text.to_string(),
        });
    }

    fn execute_console_now(&self, text: &str) {
        self.operations.emit(Q3SourceEvent::ConsoleCommand {
            execution: Q3ConsoleExecution::Now,
            text: text.to_string(),
        });
    }
}

/// Configstring store writing through to server state with dedup emission.
pub struct Q3HostConfigstrings {
    operations: Rc<dyn Q3HostOperations>,
    server: Rc<Q3ServerState>,
}

impl ConfigStringStore for Q3HostConfigstrings {
    fn get(&self, index: usize) -> String {
        let index = i32::try_from(index).expect("Q3 configstring outside source range");
        self.server.configstring_get(index)
    }

    fn set(&mut self, index: usize, value: &str) {
        let index = i32::try_from(index).expect("Q3 configstring outside source range");
        if self.server.has_configstring(index) && self.server.configstring_get(index) == value {
            return;
        }
        self.server.configstring_set(index, value);
        self.operations.emit(Q3SourceEvent::Configstring {
            index,
            value: value.to_string(),
        });
    }
}

/// Concrete source host built by [`create_q3_source_host`].
pub struct Q3SourceHostInstance {
    operations: Rc<dyn Q3HostOperations>,
    server: Rc<Q3ServerState>,
    engine: Rc<Q3HostEngine>,
    configstrings: Rc<RefCell<Q3HostConfigstrings>>,
}

impl MovementHost for Q3SourceHostInstance {
    fn move_client(
        &self,
        entity: &SupportEntityRef,
        command: &Q3UserCommand,
        options: &ClientMovementOptions,
    ) -> ClientMovementResult {
        self.operations.move_client(entity, command, options)
    }
}

impl Q3SourceHost for Q3SourceHostInstance {
    fn ammo_timer_stored(&self, actor: &ActorId, weapon: usize, value: i32) {
        self.operations.ammo_timer_stored(actor, weapon, value);
    }

    fn timer_ownership(&self, actor: &ActorId) -> Option<ClientTimerOwnership> {
        self.operations.timer_ownership(actor)
    }

    fn speed_multiplier(&self, actor: &ActorId) -> Option<f32> {
        self.operations.speed_multiplier(actor)
    }

    fn server_state(&self) -> Rc<Q3ServerState> {
        self.server.clone()
    }

    fn mover_actors(&self) -> Rc<dyn Q3SourceMoverActors> {
        self.operations.mover_actors()
    }

    fn primary_attack_allowed(&self, actor: &ActorId) -> bool {
        self.operations.primary_attack_allowed(actor)
    }

    fn grant_selected_arsenal(&self, actor: &ActorId, category: Q3ArsenalCategory) -> bool {
        self.operations.grant_selected_arsenal(actor, category)
    }

    fn give_selected_item(&self, actor: &ActorId, args: &[String]) -> bool {
        self.operations.give_selected_item(actor, args)
    }

    fn preview_pickup(&self, item: &SourcePickupDescriptor) -> SourcePickupPreview {
        self.operations.preview_pickup(item)
    }

    fn admit_pickup(&self, item: &SourcePickupDescriptor) -> SourcePickupAdmission {
        self.operations.admit_pickup(item)
    }

    fn actors(&self) -> Rc<dyn Q3SessionActors> {
        self.operations.actors()
    }

    fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
        self.operations.bodies()
    }

    fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
        self.operations.callbacks()
    }

    fn combat(&self) -> Rc<dyn Q3SessionCombat> {
        self.operations.combat()
    }

    fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
        self.operations.inventory()
    }

    fn original_pickups(&self) -> Option<OriginalPickupAdmission> {
        self.operations.original_pickups()
    }

    fn scene(&self) -> Rc<dyn Q3SourceScene> {
        self.operations.scene()
    }

    fn engine(&self) -> Rc<dyn Q3SourceEngine> {
        self.engine.clone()
    }

    fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
        self.server.cvars.clone()
    }

    fn configstrings(&self) -> Rc<RefCell<dyn ConfigStringStore>> {
        self.configstrings.clone()
    }

    fn death_animations(&self) -> Q3DeathAnimationSequence {
        self.operations.death_animations()
    }

    fn bots(&self) -> Q3SourceBots<'static> {
        self.operations.bots()
    }

    fn now(&self) -> i32 {
        self.operations.now()
    }

    fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>) {
        self.operations.schedule(actor, due_milliseconds);
    }

    fn run_think(&self, actor: &OwnedActor, time_milliseconds: i32) {
        self.operations.run_think(actor, time_milliseconds);
    }

    fn collision(&self, actor: &OwnedActor, collision: ActorCollision) {
        self.operations.collision(actor, collision);
    }

    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext {
        self.operations.armor_context(request)
    }

    fn foreign(&self, actor: &ActorId) -> Option<RecordsEntityRef> {
        self.operations.foreign(actor)
    }

    fn is_player(&self, actor: &ActorId) -> bool {
        self.operations.is_player(actor)
    }

    fn source_command(&self, input: &ActorCommand) -> Q3UserCommand {
        let command = self.operations.source_command(input);
        self.server.set_user_command(
            self.operations.client_number(&input.actor),
            Q3StoredUserCommand {
                server_time: command.server_time,
                angles: [
                    command.angles.x as i32,
                    command.angles.y as i32,
                    command.angles.z as i32,
                ],
                forwardmove: command.forwardmove,
                rightmove: command.rightmove,
                upmove: command.upmove,
                buttons: command.buttons,
                weapon: command.weapon,
            },
        );
        command
    }

    fn spawn_player(&self, entity: &SupportEntityRef, pose: &SpawnPose) {
        self.operations.spawn_player(entity, pose);
    }

    fn entity_event(&self, event: Q3SourceEntityEvent) {
        self.operations.emit(Q3SourceEvent::Entity(event));
    }
}

/// Build a source host whose engine imports publish to the operations event
/// consumer, mirroring donor `createQ3SourceHost`.
#[must_use]
pub fn create_q3_source_host(
    operations: Rc<dyn Q3HostOperations>,
    settings: Q3HostSettings,
    session: SessionId,
    register_server_cvars: Q3ServerCvarRegistration,
) -> Q3SourceHostInstance {
    let mut cvars = Vec::new();
    if settings.source_registry.is_none() {
        cvars.push(CvarNameValue {
            name: "g_gametype".to_string(),
            value: settings.game_type.to_string(),
        });
    }
    cvars.push(CvarNameValue {
        name: "ui_singlePlayerActive".to_string(),
        value: if settings.single_player {
            "1".to_string()
        } else {
            "0".to_string()
        },
    });
    cvars.extend(settings.cvars.iter().cloned());
    let print_operations = operations.clone();
    let now_operations = operations.clone();
    let server = Rc::new(Q3ServerState::new(Q3ServerStateOptions {
        session,
        settings: Q3HostSettings { cvars, ..settings },
        now: Rc::new(move || now_operations.now()),
        print: Rc::new(move |text| {
            print_operations.emit(Q3SourceEvent::Print { text: text.to_string() });
        }),
        register_server_cvars,
    }));
    Q3SourceHostInstance {
        operations: operations.clone(),
        server: server.clone(),
        engine: Rc::new(Q3HostEngine {
            operations: operations.clone(),
            server: server.clone(),
        }),
        configstrings: Rc::new(RefCell::new(Q3HostConfigstrings { operations, server })),
    }
}

#[cfg(test)]
mod tests {
    use qa_content::q3::base::shared::player_state::UserCommand as Q3UserCommand;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Vec3;
    use qa_net::common::commands::{ActorCommand, CommandSource, UserCommand};

    use super::*;

    struct FakeOperations {
        owner: IdentityOwner,
        events: RefCell<Vec<Q3SourceEvent>>,
        actors: Rc<FakeActors>,
    }

    struct FakeActors {
        owner: IdentityOwner,
    }

    impl Q3SessionActors for FakeActors {
        fn assert_owned(&self, _actor: &OwnedActor) -> Result<(), qa_content::q3::base::records::Q3BaseError> {
            Ok(())
        }

        fn allocate_at_source(
            &self,
            provider: &qa_core::identity::ProviderId,
            slot: usize,
            _definition: &str,
        ) -> OwnedActor {
            self.owner
                .owned_actor(&self.owner.actor(slot as u32, 0), provider.clone())
                .unwrap()
        }

        fn is_live(&self, _actor: &ActorId) -> bool {
            true
        }

        fn on_release(&self, _callback: Box<dyn Fn(&OwnedActor)>) -> Box<dyn Fn()> {
            Box::new(|| {})
        }

        fn release(&self, _actor: &OwnedActor) {}

        fn resolve_owned(&self, _actor: &ActorId) -> Option<OwnedActor> {
            None
        }
    }

    impl MovementHost for FakeOperations {
        fn move_client(
            &self,
            _entity: &SupportEntityRef,
            _command: &Q3UserCommand,
            _options: &ClientMovementOptions,
        ) -> ClientMovementResult {
            panic!("unused in host tests");
        }
    }

    impl Q3HostOperations for FakeOperations {
        fn mover_actors(&self) -> Rc<dyn Q3SourceMoverActors> {
            panic!("unused in host tests");
        }

        fn actors(&self) -> Rc<dyn Q3SessionActors> {
            self.actors.clone()
        }

        fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
            panic!("unused in host tests");
        }

        fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
            panic!("unused in host tests");
        }

        fn combat(&self) -> Rc<dyn Q3SessionCombat> {
            panic!("unused in host tests");
        }

        fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
            panic!("unused in host tests");
        }

        fn scene(&self) -> Rc<dyn Q3SourceScene> {
            panic!("unused in host tests");
        }

        fn death_animations(&self) -> Q3DeathAnimationSequence {
            Q3DeathAnimationSequence::new()
        }

        fn bots(&self) -> Q3SourceBots<'static> {
            Q3SourceBots::Unavailable {
                reason: "test".to_string(),
            }
        }

        fn now(&self) -> i32 {
            4242
        }

        fn schedule(&self, _actor: &OwnedActor, _due_milliseconds: Option<i32>) {}

        fn run_think(&self, _actor: &OwnedActor, _time_milliseconds: i32) {}

        fn collision(&self, _actor: &OwnedActor, _collision: ActorCollision) {}

        fn armor_context(&self, _request: &DamageRequest) -> VictimArmorContext {
            panic!("unused in host tests");
        }

        fn foreign(&self, _actor: &ActorId) -> Option<RecordsEntityRef> {
            None
        }

        fn is_player(&self, _actor: &ActorId) -> bool {
            true
        }

        fn source_command(&self, _input: &ActorCommand) -> Q3UserCommand {
            Q3UserCommand {
                server_time: 100,
                angles: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                buttons: 5,
                weapon: 7,
                forwardmove: 11,
                rightmove: 12,
                upmove: 13,
            }
        }

        fn spawn_player(&self, _entity: &SupportEntityRef, _pose: &SpawnPose) {}

        fn emit(&self, event: Q3SourceEvent) {
            self.events.borrow_mut().push(event);
        }

        fn client_number(&self, _actor: &ActorId) -> i32 {
            3
        }
    }

    fn operations() -> Rc<FakeOperations> {
        Rc::new(FakeOperations {
            owner: IdentityOwner::create("q3-host-test").unwrap(),
            events: RefCell::new(Vec::new()),
            actors: Rc::new(FakeActors {
                owner: IdentityOwner::create("q3-host-actors").unwrap(),
            }),
        })
    }

    fn settings() -> Q3HostSettings {
        Q3HostSettings {
            game_type: 0,
            single_player: true,
            max_clients: 8,
            map_name: "q3dm1".to_string(),
            source_registry: None,
            source_archive: Vec::new(),
            cvars: Vec::new(),
        }
    }

    fn register() -> Q3ServerCvarRegistration {
        Rc::new(|registry, max_clients, map_name| {
            registry.register("sv_maxclients", &max_clients.to_string(), 0).unwrap();
            registry.register("mapname", map_name, 0).unwrap();
        })
    }

    fn host() -> (Rc<FakeOperations>, Q3SourceHostInstance) {
        let operations = operations();
        let session = operations.owner.session().clone();
        let host = create_q3_source_host(operations.clone(), settings(), session, register());
        (operations, host)
    }

    #[test]
    fn engine_publishes_events_and_defaults() {
        let (operations, host) = host();
        let engine = host.engine();
        engine.print("hi");
        engine.log("lo");
        engine.send_server_command(1, "cmd");
        engine.drop_client(2, "bye");
        engine.append_console_command("a");
        engine.execute_console_now("b");
        let events = operations.events.borrow();
        assert!(matches!(events[0], Q3SourceEvent::Print { .. }));
        assert!(matches!(events[1], Q3SourceEvent::Log { .. }));
        assert!(matches!(events[2], Q3SourceEvent::ServerCommand { client: 1, .. }));
        assert!(matches!(events[3], Q3SourceEvent::DropClient { client: 2, .. }));
        assert!(matches!(
            events[4],
            Q3SourceEvent::ConsoleCommand {
                execution: Q3ConsoleExecution::Append,
                ..
            }
        ));
        assert!(matches!(
            events[5],
            Q3SourceEvent::ConsoleCommand {
                execution: Q3ConsoleExecution::Now,
                ..
            }
        ));
        drop(events);
        assert!(engine.get_userinfo(0).contains("Player 1"));
        engine.set_userinfo(0, "x");
        assert_eq!(engine.get_userinfo(0), "x");
        let default = engine.get_user_command(0);
        assert_eq!(default.server_time, 4242);
        assert_eq!(default.weapon, 2);
        assert_eq!(host.cvars().borrow().variable_string("ui_singlePlayerActive"), "1");
    }

    #[test]
    fn configstrings_dedup_writes() {
        let (operations, host) = host();
        let strings = host.configstrings();
        assert_eq!(strings.borrow().get(12), "");
        strings.borrow_mut().set(12, "v");
        strings.borrow_mut().set(12, "v");
        assert_eq!(strings.borrow().get(12), "v");
        let config_events = operations
            .events
            .borrow()
            .iter()
            .filter(|event| matches!(event, Q3SourceEvent::Configstring { .. }))
            .count();
        assert_eq!(config_events, 1);
    }

    #[test]
    fn source_command_records_wire_command() {
        let (operations, host) = host();
        let input = ActorCommand {
            actor: operations.owner.actor(0, 0),
            source: CommandSource::LocalSeat {
                seat: operations.owner.seat(0),
            },
            sequence: 1,
            command: UserCommand::Q3 {
                server_time_milliseconds: 0.0,
                angle_words: [0.0, 0.0, 0.0],
                buttons: 0.0,
                weapon: 0.0,
                forward_move: 0.0,
                right_move: 0.0,
                up_move: 0.0,
            },
            arsenal: None,
        };
        let command = Q3SourceHost::source_command(&host, &input);
        assert_eq!(command.weapon, 7);
        let stored = host.server_state().get_user_command(3).unwrap();
        assert_eq!(stored.server_time, 100);
        assert_eq!(stored.angles, [1, 2, 3]);
        let round = host.engine().get_user_command(3);
        assert_eq!(round.weapon, 7);
        assert_eq!(round.angles.x, 1.0);
    }
}
