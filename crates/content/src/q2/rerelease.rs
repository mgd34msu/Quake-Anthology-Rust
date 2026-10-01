//! Q2 rerelease (`src/content/q2/rerelease`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;

use qa_core::identity::ActorId;

use crate::q2::foundation::host::{Q2GameServices, Q2Use};

pub mod campaign;
pub mod checkpoint;
pub mod debug_shapes;
pub mod environment;
pub mod fog;
pub mod goals;
pub mod killbox;
pub mod lights;
pub mod monsters;
pub mod obituary;
pub mod types;
pub mod world_text;

pub use self::campaign::{
    Q2RereleaseCampaignState, Q2RereleaseLevelEntry, Q2RereleaseMission, create_q2_rerelease_campaign_state,
    enter_q2_rerelease_level, q2_rerelease_unit_report, update_q2_rerelease_level,
};
pub use self::checkpoint::{
    Q2RereleaseCampaignCheckpoint, Q2RereleaseHealthBarCheckpoint, Q2RereleaseHealthTarget, Q2RereleaseIntermissionCamera,
    Q2RereleaseLightCheckpoint, Q2RereleaseModuleCheckpoint, Q2RereleasePickupRecord, Q2RereleasePlayerCheckpointEntry,
    Q2RereleasePlayersCheckpoint, Q2RereleasePoiCheckpoint, Q2RereleaseQ64CameraCheckpoint, Q2RereleaseQ64CameraState,
    Q2RereleaseQ64Checkpoint, Q2RereleaseQ64DummyCheckpoint, Q2RereleaseQ64DummyState, Q2RereleaseQ64EyeCheckpoint,
    Q2RereleaseQ64EyeState, Q2RereleaseSkyCheckpoint, Q2RereleaseSquadSpawn, Q2RereleaseTriggerSoundTime,
};
pub use self::debug_shapes::{q2_debug_shape, rerelease_debug_lifetime};
pub use self::environment::{q2_rerelease_falling_damage, q2_rerelease_world_effects};
pub use self::fog::{equal_q2_fog, interpolate_q2_fog, q2_rerelease_fog_fields};
pub use self::goals::{
    Q2RereleaseGoals, Q2RereleaseGoalsCheckpoint, rerelease_goal_callbacks, rerelease_goals_spawn,
};
pub use self::killbox::kill_q2_rerelease_box;
pub use self::lights::{Q2RereleaseLights, q2_rerelease_color, rerelease_light_callbacks, rerelease_light_spawn};
pub use self::obituary::{Q2RereleaseObituary, q2_rerelease_obituary, q2_rerelease_obituary_scored};
pub use self::types::{
    Q2CoopRespawnState, Q2ExpansionPowerups, Q2Fog, Q2FogState, Q2HeightFog, Q2LocalizedPrintLevel, Q2PendingLandmark,
    Q2RereleaseEvent, Q2RereleaseHooks, Q2RereleaseNavigation, Q2RereleaseOptions, Q2RereleasePlayerIdentity,
    Q2RereleasePlayerState, create_q2_fog, create_q2_rerelease_options, q2_is_n64, q2_uses_instanced_items,
};
pub use self::world_text::{Q2WorldTextRequest, q2_world_text};

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
            .finish()
    }
}

/// Session rerelease hooks.
pub fn rerelease_hooks(game: &Q2GameServices) -> types::Q2RereleaseHooks {
    game.rerelease.hooks.expect("Q2 rerelease is not registered")
}
