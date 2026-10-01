//! Quake III source provider host vocabulary.
//!
//! Port of donor `src/app/bootstrap/simulation/q3/types.ts`
//! (`Q3SourceHost`, `Q3SourceOptions`, `Q3SourceSessionCarry`,
//! `Q3SourceEngine`, `Q3SourceBots`, source events). Session,
//! inventory, collision, and combat handles reuse the `qa-content`
//! session traits; player protocol state reuses the `qa-guest`
//! `Q3PlayerState` mirror; unified commands reuse the `qa-net` actor
//! command.
//!
//! The server state home has landed: [`Q3ServerState`] is re-exported from
//! `super::server_state` (donor `./server-state.ts`). The source
//! presentation state/models (donor `./presentation.ts`) are still declared
//! here as [`Q3SourcePresentationState`]/[`Q3SourceModel`] with exactly the
//! fields that donor computes; the presentation home rewires them when it
//! lands.
//!
//! Entity handles straddle the two `qa-content` entity cores until
//! the content lanes unify them: records-bound callbacks take
//! [`RecordsEntityRef`], while movement/spawn callbacks take the
//! team-arena [`SupportEntityRef`] their runtimes borrow.

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::contract::{ExecutableRecipe, ModuleIdentity, ProviderReference, QvmAbiProfile};
use qa_content::q3::base::combat_bridge::VictimArmorContext;
use qa_content::q3::base::game::item_lifecycle::{
    OriginalPickupAdmission, SourcePickupAdmission, SourcePickupDescriptor, SourcePickupPreview,
};
use qa_content::q3::base::game::mover::SharedMoverBody;
use qa_content::q3::base::records::EntityRef as RecordsEntityRef;
use qa_content::q3::base::records::{
    DamageRequest, Q3ActorCallbacks, Q3SessionActors, Q3SessionBodies, Q3SessionCombat, Q3SessionInventory,
};
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::base::shared::entity_state::EntityState;
use qa_content::q3::base::shared::player_state::UserCommand as Q3UserCommand;
use qa_content::q3::base::world_adapter::{ActorCollision, Q3WorldAdapterHost};
use qa_content::q3::foundation::character::Q3DeathAnimationSequence;
use qa_content::q3::team_arena::client_admission::ClientBotServices;
use qa_content::q3::team_arena::client_effects::ClientTimerOwnership;
use qa_content::q3::team_arena::client_spawn::SpawnPose;
use qa_content::q3::team_arena::movement_host::MovementHost;
pub use qa_content::q3::team_arena::movement_host::{ClientMovementOptions, ClientMovementResult};
use qa_content::q3::team_arena::support::EntityRef as SupportEntityRef;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_guest::qvm::player_record::QvmPlayerState;
use qa_net::common::commands::ActorCommand;

use super::super::q3_ballistics::Q3WeaponBehaviorPort;
use crate::bootstrap::q3_client::visibility::ApplicationQ3SceneQueries;
use qa_content::q3::base::game::utilities::ConfigStringStore;

/// Source entity event for presentation consumers.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourceEntityEvent {
    /// Acting actor.
    pub actor: ActorId,
    /// Entity state snapshot.
    pub state: EntityState,
    /// Event origin.
    pub origin: Vec3,
    /// Event time in milliseconds.
    pub time: i32,
}

/// Originating presentation module of a player event.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Q3EventSource {
    /// Presenting module.
    pub module: ModuleIdentity,
    /// Module ABI profile.
    pub abi_profile: QvmAbiProfile,
}

/// Player-event sequence word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3PlayerEventSequence {
    /// External event with a timestamp.
    External {
        /// Event time in milliseconds.
        time: i32,
    },
    /// Predictable event with a sequence number.
    Predictable {
        /// Sequence number.
        sequence: i32,
    },
}

/// Original component enums and source client numbers require the
/// originating presentation module.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourcePlayerEvent {
    /// Acting actor.
    pub actor: ActorId,
    /// Originating presentation module.
    pub source: Q3EventSource,
    /// Player state snapshot.
    pub player_state: QvmPlayerState,
    /// Event id.
    pub event: i32,
    /// Event parameter.
    pub parameter: i32,
    /// Sequence word.
    pub sequence: Q3PlayerEventSequence,
    /// Event origin.
    pub origin: Vec3,
    /// Event time in milliseconds.
    pub time: i32,
}

/// Engine services behind the source runtime.
pub trait Q3SourceEngine {
    /// Print engine text.
    fn print(&self, text: &str);
    /// Log engine text.
    fn log(&self, text: &str);
    /// Send a server command to a client.
    fn send_server_command(&self, client: i32, text: &str);
    /// Drop a client with a reason.
    fn drop_client(&self, client: i32, reason: &str);
    /// Read a client userinfo string.
    fn get_userinfo(&self, client: i32) -> String;
    /// Write a client userinfo string.
    fn set_userinfo(&self, client: i32, value: &str);
    /// Read the latest user command for a client.
    fn get_user_command(&self, client: i32) -> Q3UserCommand;
    /// Queue a console command.
    fn append_console_command(&self, text: &str);
    /// Execute a console command immediately.
    fn execute_console_now(&self, text: &str);
}

/// Bot console command callback.
pub type Q3BotConsoleCommand = Rc<dyn Fn(&[String])>;

/// Source bot services: admission bot services plus the match-level
/// hooks the runtime drives.
#[derive(Clone)]
pub enum Q3SourceBots {
    /// Bots unavailable.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Bots available.
    Available {
        /// Remove a queued begin.
        remove_queued_begin: Rc<dyn Fn(usize)>,
        /// Connect a bot.
        connect: Rc<dyn Fn(usize, bool) -> bool>,
        /// Shut down a bot client.
        shutdown_client: Rc<dyn Fn(usize, bool)>,
        /// Test AAS at an origin.
        test_aas: Rc<dyn Fn(Vec3)>,
        /// Interbreed at match end.
        interbreed_end_match: Rc<dyn Fn()>,
        /// Run a bot console command.
        console_command: Q3BotConsoleCommand,
    },
}

impl Q3SourceBots {
    /// Admission-level bot services.
    #[must_use]
    pub fn client_services(&self) -> ClientBotServices {
        match self {
            Self::Unavailable { reason } => ClientBotServices::Unavailable { reason: reason.clone() },
            Self::Available {
                remove_queued_begin,
                connect,
                shutdown_client,
                ..
            } => ClientBotServices::Available {
                remove_queued_begin: remove_queued_begin.clone(),
                connect: connect.clone(),
                shutdown_client: shutdown_client.clone(),
            },
        }
    }
}

/// Engine-owned storage shared by the selected game and its network host,
/// re-exported from its real home (donor `./server-state.ts`).
pub use super::server_state::Q3ServerState;

/// Mover actor access owned by the session (donor
/// `Pick<MoverActorAccess, "observe" | "write" | "link" | "release">`).
pub trait Q3SourceMoverActors {
    /// Observe a shared mover body.
    fn observe(&self, actor: &ActorId) -> Option<SharedMoverBody>;
    /// Write a mover origin and ground.
    fn write(&self, actor: &ActorId, origin: Vec3, ground: Option<ActorId>);
    /// Link a mover actor.
    fn link_actor(&self, actor: &ActorId);
    /// Release a mover actor.
    fn release_actor(&self, actor: &ActorId);
}

/// Shared scene queries owned by the session: world-adapter tracing
/// plus the visibility, model-bounds, and area-portal queries the
/// runtime drives directly.
pub trait Q3SourceScene: Q3WorldAdapterHost + ApplicationQ3SceneQueries {
    /// Inline model bounds.
    fn model_bounds(&self, index: i32) -> Bounds;
    /// Open or close the portal between two areas.
    fn adjust_area_portal_state(&self, first: i32, second: i32, open: bool);
}

/// Arsenal grant category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ArsenalCategory {
    /// Weapons.
    Weapons,
    /// Ammunition.
    Ammo,
}

/// W73 supplies the existing owners; this provider contains source
/// game records and phase functions.
pub trait Q3SourceHost: MovementHost {
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
    /// Engine-owned server storage.
    fn server_state(&self) -> Rc<Q3ServerState>;
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
    /// Engine services.
    fn engine(&self) -> Rc<dyn Q3SourceEngine>;
    /// Cvar registry.
    fn cvars(&self) -> Rc<RefCell<CvarRegistry>>;
    /// Configstring store.
    fn configstrings(&self) -> Rc<RefCell<dyn ConfigStringStore>>;
    /// Death animation sequence.
    fn death_animations(&self) -> Q3DeathAnimationSequence;
    /// Bot services.
    fn bots(&self) -> Q3SourceBots;
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
    /// Convert command units for source policy; moveClient still runs
    /// the selected provider.
    fn source_command(&self, input: &ActorCommand) -> Q3UserCommand;
    /// Spawn the selected player entity at a pose.
    fn spawn_player(&self, entity: &SupportEntityRef, pose: &SpawnPose);
    /// Publish a source entity event.
    fn entity_event(&self, event: Q3SourceEntityEvent);
}

/// Source runtime construction options.
pub struct Q3SourceOptions {
    /// Weapon behavior overrides, if any.
    pub weapon_behavior: Option<Rc<dyn Q3WeaponBehaviorPort>>,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
    /// Weapon provider.
    pub weapon_provider: ProviderReference,
    /// Product.
    pub product: Product,
    /// Entity text.
    pub entities: String,
    /// Random seed.
    pub seed: i32,
    /// Maximum clients.
    pub max_clients: usize,
    /// Build date.
    pub build_date: String,
    /// Session carry from the previous map, if any.
    pub session_carry: Option<Q3SourceSessionCarry>,
}

/// One carried client session row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SourceSessionClient {
    /// Client slot.
    pub slot: i32,
    /// Session string.
    pub session: String,
    /// Userinfo string.
    pub userinfo: String,
}

/// `g_session.c`'s seven source fields survive maps; scores, health
/// and weapons are new-level state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SourceSessionCarry {
    /// World session string.
    pub world: String,
    /// Carried clients.
    pub clients: Vec<Q3SourceSessionClient>,
}

/// One copied source entity row (donor `Q3SourcePresentationState`
/// entity word from `./presentation.ts`, out of scope).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourcePresentationEntity {
    /// Acting actor.
    pub actor: ActorId,
    /// Entity state snapshot.
    pub state: EntityState,
    /// World origin.
    pub origin: Vec3,
    /// Whether linked.
    pub linked: bool,
    /// Server flags.
    pub server_flags: i32,
    /// Single-client target.
    pub single_client: i32,
}

/// One copied source client row (donor `Q3SourcePresentationState`
/// client word).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourcePresentationClient {
    /// Acting actor.
    pub actor: ActorId,
    /// Client slot.
    pub slot: i32,
    /// Player state snapshot.
    pub state: QvmPlayerState,
}

/// One copied configstring row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SourcePresentationString {
    /// Configstring index.
    pub index: i32,
    /// Configstring value.
    pub value: String,
}

/// Copied source state for cgame/network consumers (donor
/// `Q3SourcePresentationState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourcePresentationState {
    /// Product.
    pub product: Product,
    /// Presentation time in milliseconds.
    pub time: i32,
    /// Copied entities.
    pub entities: Vec<Q3SourcePresentationEntity>,
    /// Copied clients.
    pub clients: Vec<Q3SourcePresentationClient>,
    /// Non-empty configstrings.
    pub configstrings: Vec<Q3SourcePresentationString>,
}

/// Source model presentation for the shared renderer (minimal
/// `SimulationPresentation` from donor `../types.ts`, out of scope:
/// only the fields `q3PoolModels` writes are named).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SourceModel {
    /// Acting actor.
    pub actor: ActorId,
    /// Presenting content.
    pub content: qa_content::contract::ContentId,
    /// Render owner.
    pub render_owner: Q3ModelOwner,
    /// Model path.
    pub path: String,
    /// Current frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Skin index.
    pub skin: i32,
    /// Effects flags.
    pub effects: i32,
    /// Render flags.
    pub render_flags: i32,
    /// World origin.
    pub origin: Vec3,
    /// World angles.
    pub angles: Vec3,
    /// Scale.
    pub scale: f32,
    /// Whether visible.
    pub visible: bool,
    /// Whether a view weapon.
    pub view_weapon: bool,
}

/// Source model render owner (donor `"source-client"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ModelOwner {
    /// Source client.
    SourceClient,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_services_split_admission_level() {
        let unavailable = Q3SourceBots::Unavailable {
            reason: "no-aas".to_string(),
        };
        assert!(matches!(
            unavailable.client_services(),
            ClientBotServices::Unavailable { .. }
        ));
        let available = Q3SourceBots::Available {
            remove_queued_begin: Rc::new(|_| {}),
            connect: Rc::new(|_, _| true),
            shutdown_client: Rc::new(|_, _| {}),
            test_aas: Rc::new(|_| {}),
            interbreed_end_match: Rc::new(|| {}),
            console_command: Rc::new(|_| {}),
        };
        assert!(matches!(
            available.client_services(),
            ClientBotServices::Available { .. }
        ));
    }

    #[test]
    fn player_event_sequence_covers_both_kinds() {
        let external = Q3PlayerEventSequence::External { time: 100 };
        let predictable = Q3PlayerEventSequence::Predictable { sequence: 7 };
        assert_ne!(external, predictable);
    }
}
