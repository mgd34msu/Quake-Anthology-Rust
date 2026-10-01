//! Q2 base entities (`src/content/q2/base/entities`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

pub mod movers;
pub mod scenery;
pub mod targets;
pub mod triggers;
pub mod turrets;
pub mod types;

pub use movers::{
    Q2BaseMoversCheckpoint, Q2PlatformPhase, Q2PlatformState, Q2PlatformTraversal,
};
pub use scenery::{Q2AnimationState, Q2BaseSceneryCheckpoint, Q2ClockState, q2_clock_text};
pub use triggers::Q2WindTimeEntry;
pub use turrets::{Q2BreachState, Q2DriverState, Q2TurretsCheckpoint, snap_q2_turret_eighth};
pub use types::Q2BaseEntityHooks;

use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2GameServices, Q2ItemNameFn, Q2SpawnFn, SpawnModule};
use crate::q2::foundation::motion::LinearMoveState;

/// Arena runtime state for this module.
#[derive(Debug, Default)]
pub struct BaseEntitiesRuntime {
    /// Registered hooks.
    pub hooks: Option<Q2BaseEntityHooks>,
    /// Active base linear moves.
    pub linear_moves: HashMap<ActorId, LinearMoveState>,
    /// Platform states.
    pub platforms: HashMap<ActorId, Q2PlatformState>,
    /// Secret-door states.
    pub secrets: HashMap<ActorId, movers::Q2SecretState>,
    /// Turret breach states.
    pub breaches: HashMap<ActorId, Q2BreachState>,
    /// Turret driver states.
    pub drivers: HashMap<ActorId, Q2DriverState>,
    /// Scenery animations.
    pub animations: HashMap<ActorId, Q2AnimationState>,
    /// Clock states.
    pub clocks: HashMap<ActorId, Q2ClockState>,
    /// Wind-sound throttles.
    pub wind_times: HashMap<ActorId, f64>,
}

impl BaseEntitiesRuntime {
    /// Drop per-actor state on release.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.linear_moves.remove(actor);
        self.platforms.remove(actor);
        self.secrets.remove(actor);
        self.breaches.remove(actor);
        self.drivers.remove(actor);
        self.animations.remove(actor);
        self.clocks.remove(actor);
        self.wind_times.remove(actor);
    }
}

/// Base entities checkpoint (`Q2BaseEntitiesCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2BaseEntitiesCheckpoint {
    /// Checkpoint version.
    pub version: u32,
    /// Movers.
    pub movers: Q2BaseMoversCheckpoint,
    /// Scenery.
    pub scenery: Q2BaseSceneryCheckpoint,
    /// Turrets.
    pub turrets: Q2TurretsCheckpoint,
    /// Wind-sound throttles.
    pub wind_times: Vec<Q2WindTimeEntry>,
}

/// Source classnames supplied here (`q2BaseEntityClassnames`).
pub const Q2_BASE_ENTITY_CLASSNAMES: &[&str] = &[
    "func_plat",
    "func_door_secret",
    "trigger_elevator",
    "func_conveyor",
    "func_killbox",
    "func_object",
    "trigger_push",
    "trigger_hurt",
    "trigger_gravity",
    "trigger_monsterjump",
    "target_temp_entity",
    "target_spawner",
    "target_blaster",
    "target_crosslevel_trigger",
    "target_crosslevel_target",
    "target_laser",
    "target_lightramp",
    "target_earthquake",
    "target_character",
    "target_string",
    "func_clock",
    "viewthing",
    "misc_blackhole",
    "misc_eastertank",
    "misc_easterchick",
    "misc_easterchick2",
    "monster_commander_body",
    "misc_bigviper",
    "misc_viper_bomb",
    "light_mine1",
    "light_mine2",
    "misc_gib_arm",
    "misc_gib_leg",
    "misc_teleporter",
    "misc_teleporter_dest",
    "turret_base",
    "turret_breach",
    "turret_driver",
];

/// Spawn dispatch across the base entity modules.
fn spawn_base_entity(actor: ActorId, game: &mut Q2GameServices) -> bool {
    movers::spawn_mover(actor.clone(), game)
        || triggers::spawn_trigger(actor.clone(), game)
        || targets::spawn_target(actor.clone(), game)
        || scenery::spawn_scenery(actor.clone(), game)
        || turrets::spawn_turret(actor, game)
}

/// Base entity item-name handler (unused).
fn base_entity_item_name(_classname: &str) -> Option<String> {
    None
}

/// Q2 base entity module (`Q2BaseEntityModule`).
#[derive(Debug, Clone, Copy)]
pub struct Q2BaseEntityModule {
    /// Provider hooks.
    hooks: Q2BaseEntityHooks,
}

/// Create the Q2 base entity module (`createQ2BaseEntityModule`).
pub fn create_q2_base_entity_module(hooks: Q2BaseEntityHooks) -> Q2BaseEntityModule {
    Q2BaseEntityModule { hooks }
}

impl Q2BaseEntityModule {
    /// Register hooks and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices) -> SpawnModule {
        game.base_entities.hooks = Some(self.hooks);
        let mut callbacks = Q2CallbackDefinitions::default();
        for source in [
            movers::mover_callbacks(),
            targets::target_callbacks(),
            triggers::trigger_callbacks(),
            scenery::scenery_callbacks(),
            turrets::turret_callbacks(),
        ] {
            callbacks.think.extend(source.think);
            callbacks.use_.extend(source.use_);
            callbacks.touch.extend(source.touch);
            callbacks.pain.extend(source.pain);
            callbacks.die.extend(source.die);
            callbacks.blocked.extend(source.blocked);
            callbacks.trajectory.extend(source.trajectory);
        }
        let spawn: Q2SpawnFn = spawn_base_entity;
        let item_name: Q2ItemNameFn = base_entity_item_name;
        SpawnModule {
            spawn,
            item_name,
            callbacks,
        }
    }

    /// Read mover traversal (`moverTraversal`).
    pub fn mover_traversal(
        &self,
        actor: &ActorId,
        game: &mut Q2GameServices,
    ) -> Option<Q2PlatformTraversal> {
        movers::mover_traversal(game, actor)
    }

    /// Read platform state (`platformState`).
    pub fn platform_state(
        &self,
        actor: &ActorId,
        game: &Q2GameServices,
    ) -> Option<Q2PlatformState> {
        movers::mover_platform_state(game, actor)
    }

    /// Capture base entities (`capture`).
    pub fn capture(&self, game: &mut Q2GameServices) -> Q2BaseEntitiesCheckpoint {
        Q2BaseEntitiesCheckpoint {
            version: 1,
            movers: movers::capture_movers(game),
            scenery: scenery::capture_scenery(game),
            turrets: turrets::capture_turrets(game),
            wind_times: triggers::capture_triggers(game),
        }
    }

    /// Restore base entities (`restore`).
    ///
    /// Restore after shared tables, foundation entities and monster
    /// contexts; no source callback runs.
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2BaseEntitiesCheckpoint) {
        movers::restore_movers(game, &checkpoint.movers);
        scenery::restore_scenery(game, &checkpoint.scenery);
        triggers::restore_triggers(game, &checkpoint.wind_times);
        turrets::restore_turrets(game, &checkpoint.turrets);
    }
}
