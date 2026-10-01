//! Q2 rerelease (`src/content/q2/rerelease`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::{HashMap, HashSet};

use qa_core::identity::ActorId;

use crate::q2::foundation::host::{Q2GameServices, Q2Use};

pub mod campaign;
pub mod checkpoint;
pub mod debug_shapes;
pub mod entities;
pub mod environment;
pub mod fog;
pub mod goals;
pub mod index;
pub mod killbox;
pub mod lights;
pub mod monsters;
pub mod obituary;
pub mod players;
pub mod q64;
pub mod spawns;
pub mod triggers;
pub mod types;
pub mod view;
pub mod world_text;

pub use self::campaign::{
    create_q2_rerelease_campaign_state, enter_q2_rerelease_level, q2_rerelease_unit_report, update_q2_rerelease_level,
    Q2RereleaseCampaignState, Q2RereleaseLevelEntry, Q2RereleaseMission,
};
pub use self::checkpoint::{
    Q2RereleaseCampaignCheckpoint, Q2RereleaseHealthBarCheckpoint, Q2RereleaseHealthTarget,
    Q2RereleaseIntermissionCamera, Q2RereleaseLightCheckpoint, Q2RereleaseModuleCheckpoint, Q2RereleasePickupRecord,
    Q2RereleasePlayerCheckpointEntry, Q2RereleasePlayersCheckpoint, Q2RereleasePoiCheckpoint,
    Q2RereleaseQ64CameraCheckpoint, Q2RereleaseQ64CameraState, Q2RereleaseQ64Checkpoint, Q2RereleaseQ64DummyCheckpoint,
    Q2RereleaseQ64DummyState, Q2RereleaseQ64EyeCheckpoint, Q2RereleaseQ64EyeState, Q2RereleaseSkyCheckpoint,
    Q2RereleaseTriggerSoundTime,
};
pub use self::debug_shapes::{q2_debug_shape, rerelease_debug_lifetime};
pub use self::entities::{
    rerelease_entities_callbacks, rerelease_entities_spawn, Q2RereleaseEntities, Q2RereleaseHealthBar, Q2RereleasePoi,
};
pub use self::environment::{q2_rerelease_falling_damage, q2_rerelease_world_effects};
pub use self::fog::{equal_q2_fog, interpolate_q2_fog, q2_rerelease_fog_fields};
pub use self::goals::{rerelease_goal_callbacks, rerelease_goals_spawn, Q2RereleaseGoals, Q2RereleaseGoalsCheckpoint};
pub use self::index::{
    create_q2_rerelease_module, rerelease_actor_released, rerelease_module_callbacks, rerelease_module_spawn,
    rerelease_player_extension, Q2RereleaseModule,
};
pub use self::killbox::kill_q2_rerelease_box;
pub use self::lights::{q2_rerelease_color, rerelease_light_callbacks, rerelease_light_spawn, Q2RereleaseLights};
pub use self::obituary::{q2_rerelease_obituary, q2_rerelease_obituary_scored, Q2RereleaseObituary};
pub use self::players::{
    rerelease_player_callbacks, rerelease_player_overrides, Q2RereleasePlayerExtension, Q2RereleasePlayers,
    Q2RereleaseSelectedSpawn, Q2RereleaseSquadSpawn,
};
pub use self::q64::{rerelease_q64_callbacks, rerelease_q64_spawn, Q2RereleaseQ64};
pub use self::spawns::{
    q2_rerelease_player_bounds, q2_rerelease_single_spawn, q2_rerelease_spawns_callbacks, q2_rerelease_spawns_spawn,
    select_q2_rerelease_spawn,
};
pub use self::triggers::{rerelease_trigger_callbacks, rerelease_trigger_spawn, Q2RereleaseTriggers};
pub use self::types::{
    create_q2_fog, create_q2_rerelease_options, q2_is_n64, q2_uses_instanced_items, Q2CoopRespawnState,
    Q2ExpansionPowerups, Q2Fog, Q2FogState, Q2HeightFog, Q2LocalizedPrintLevel, Q2PendingLandmark, Q2RereleaseEvent,
    Q2RereleaseHooks, Q2RereleaseNavigation, Q2RereleaseOptions, Q2RereleasePlayerIdentity, Q2RereleasePlayerState,
};
pub use self::view::{q2_rerelease_build_view, q2_rerelease_client_animation, q2_rerelease_damage_feedback};
pub use self::world_text::{q2_world_text, Q2WorldTextRequest};

/// Arena runtime state for the rerelease module.
pub struct RereleaseRuntime {
    /// Session hooks.
    pub hooks: Option<types::Q2RereleaseHooks>,
    /// Rerelease options.
    pub options: types::Q2RereleaseOptions,
    /// Admitted rerelease player states.
    pub states: HashMap<ActorId, types::Q2RereleasePlayerState>,
    /// Campaign state.
    pub campaign: campaign::Q2RereleaseCampaignState,
    /// Goal list text.
    pub goals: Option<String>,
    /// Completed goal index.
    pub goal_number: i32,
    /// Light activity by actor.
    pub light_active: HashMap<ActorId, bool>,
    /// POI use callback registered by the module assembly.
    pub set_poi: Option<Q2Use>,
    /// Coop restart time.
    pub coop_restart_time: f64,
    /// Whether the active killbox is deadly.
    pub deadly_kill_box: bool,
    /// Intermission flags.
    pub intermission_flags: i32,
    /// Intermission fade expiry.
    pub intermission_fade_until: Option<f64>,
    /// Intermission camera placement.
    pub intermission_camera: Option<checkpoint::Q2RereleaseIntermissionCamera>,
    /// Whether the intermission camera is set.
    pub intermission_camera_set: bool,
    /// Registered player extension.
    pub extension: Option<players::Q2RereleasePlayerExtension>,
    /// Squad respawn placements by actor.
    pub squad_spawns: HashMap<ActorId, players::Q2RereleaseSquadSpawn>,
    /// Selected spawn placements by actor.
    pub selected_spawns: HashMap<ActorId, players::Q2RereleaseSelectedSpawn>,
    /// World fog state.
    pub world_fog: types::Q2FogState,
    /// Story text.
    pub story: String,
    /// Sky state.
    pub sky: checkpoint::Q2RereleaseSkyCheckpoint,
    /// Active point of interest.
    pub poi: Option<entities::Q2RereleasePoi>,
    /// POI stage.
    pub poi_stage: i32,
    /// Last autosave time.
    pub last_auto_save: f64,
    /// Health bar slots.
    pub health_bars: [Option<entities::Q2RereleaseHealthBar>; 2],
    /// Health bar targets by controller.
    pub health_targets: HashMap<ActorId, ActorId>,
    /// Slots that picked up an item by item actor.
    pub picked_up_by: HashMap<ActorId, HashSet<i32>>,
    /// Stashed pickup messages by item actor.
    pub pickup_messages: HashMap<ActorId, String>,
    /// Trigger hurt sound times by actor.
    pub trigger_sound_times: HashMap<ActorId, f64>,
    /// Q64 eye states by actor.
    pub q64_eyes: HashMap<ActorId, checkpoint::Q2RereleaseQ64EyeState>,
    /// Q64 camera states by actor.
    pub q64_cameras: HashMap<ActorId, checkpoint::Q2RereleaseQ64CameraState>,
    /// Q64 dummy states by actor.
    pub q64_dummies: HashMap<ActorId, checkpoint::Q2RereleaseQ64DummyState>,
}

impl Default for RereleaseRuntime {
    fn default() -> Self {
        Self {
            hooks: None,
            options: types::create_q2_rerelease_options(),
            states: HashMap::new(),
            campaign: campaign::create_q2_rerelease_campaign_state(),
            goals: None,
            goal_number: 0,
            light_active: HashMap::new(),
            set_poi: None,
            coop_restart_time: 0.0,
            deadly_kill_box: false,
            intermission_flags: 0,
            intermission_fade_until: None,
            intermission_camera: None,
            intermission_camera_set: false,
            extension: None,
            squad_spawns: HashMap::new(),
            selected_spawns: HashMap::new(),
            world_fog: types::create_q2_fog(),
            story: String::new(),
            sky: checkpoint::Q2RereleaseSkyCheckpoint {
                name: "unit1_".to_string(),
                rotation: 0.0,
                auto_rotate: true,
                axis: qa_core::math::Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            },
            poi: None,
            poi_stage: 0,
            last_auto_save: 0.0,
            health_bars: [None, None],
            health_targets: HashMap::new(),
            picked_up_by: HashMap::new(),
            pickup_messages: HashMap::new(),
            trigger_sound_times: HashMap::new(),
            q64_eyes: HashMap::new(),
            q64_cameras: HashMap::new(),
            q64_dummies: HashMap::new(),
        }
    }
}

impl std::fmt::Debug for RereleaseRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RereleaseRuntime")
            .field("hooks", &self.hooks)
            .field("options", &self.options)
            .field("states", &self.states)
            .field("campaign", &self.campaign)
            .field("goals", &self.goals)
            .field("goal_number", &self.goal_number)
            .field("light_active", &self.light_active)
            .field("set_poi", &self.set_poi)
            .field("coop_restart_time", &self.coop_restart_time)
            .field("deadly_kill_box", &self.deadly_kill_box)
            .field("intermission_flags", &self.intermission_flags)
            .field("intermission_fade_until", &self.intermission_fade_until)
            .field("intermission_camera", &self.intermission_camera)
            .field("intermission_camera_set", &self.intermission_camera_set)
            .field("extension", &self.extension)
            .field("squad_spawns", &self.squad_spawns)
            .field("selected_spawns", &self.selected_spawns)
            .field("world_fog", &self.world_fog)
            .field("story", &self.story)
            .field("sky", &self.sky)
            .field("poi", &self.poi)
            .field("poi_stage", &self.poi_stage)
            .field("last_auto_save", &self.last_auto_save)
            .field("health_bars", &self.health_bars)
            .field("health_targets", &self.health_targets)
            .field("picked_up_by", &self.picked_up_by)
            .field("pickup_messages", &self.pickup_messages)
            .field("trigger_sound_times", &self.trigger_sound_times)
            .field("q64_eyes", &self.q64_eyes)
            .field("q64_cameras", &self.q64_cameras)
            .field("q64_dummies", &self.q64_dummies)
            .finish()
    }
}

/// Session rerelease hooks.
pub fn rerelease_hooks(game: &Q2GameServices) -> types::Q2RereleaseHooks {
    game.rerelease.hooks.expect("Q2 rerelease is not registered")
}

/// Sort actors into checkpoint-stable order.
///
/// Donor `Map` iteration order is insertion order; the arena stores
/// `HashMap`s, so every multi-actor capture or broadcast sorts first.
pub(crate) fn sort_rerelease_actors(actors: &mut [ActorId]) {
    actors.sort_by_key(|actor| (actor.slot(), actor.generation()));
}
