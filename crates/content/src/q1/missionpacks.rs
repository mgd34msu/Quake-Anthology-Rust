//! Q1 mission-pack root (`src/content/q1/missionpacks`, barrel `index.ts`).
//!
//! Renames against the donor barrel follow the sibling-module
//! snake-case convention (`registerMissionPackArsenal` is
//! [`arsenal::register_mission_pack_arsenal`], `missionWeapons` is
//! [`types::MISSION_WEAPONS`], and so on).

pub mod arsenal;
pub mod backpacks;
pub mod commands;
pub mod grapple;
pub mod hipnotic_weapons;
pub mod items;
pub mod messages;
pub mod monsters;
pub mod obituaries;
pub mod pickup_rules;
pub mod player;
pub mod presentation;
pub mod rogue_weapons;
pub mod runtime;
pub mod selection;
pub mod travel;
pub mod types;
pub mod world;

pub use arsenal::{register_mission_pack_arsenal, MissionPackArsenal};
pub use backpacks::drop_mission_pack_backpack;
pub use hipnotic_weapons::{
    launch_hipnotic_laser, launch_hipnotic_proximity, register_hipnotic_hammer_callbacks,
    register_hipnotic_laser_callbacks, spawn_hipnotic_hammer_base, HipnoticLaserProfile,
};
pub use obituaries::{mission_pack_obituary, MissionPackObituaryContext, Q1MissionPackObituaryInput};
pub use player::MissionPackPlayers;
pub use presentation::mission_pack_character_pose;
pub use rogue_weapons::{launch_rogue_lava_spike, launch_rogue_multi_grenade, launch_rogue_plasma};
pub use runtime::{register_q1_mission_pack, Q1MissionPackOptions, Q1MissionPackRuntime};
pub use travel::{
    admit_mission_pack_travel, capture_mission_pack_travel, decode_mission_pack_travel, new_mission_pack_travel,
};
pub use types::{MissionPowerup, MissionWeapon, MissionWeaponDefinition, Q1MissionPack, MISSION_WEAPONS};
