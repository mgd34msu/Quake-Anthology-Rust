//! Q1 base-game root (`src/content/q1/base`, barrel `index.ts`).
//!
//! Renames against the donor barrel: `Q1Base` is [`provider::Q1BaseState`],
//! `Q1_BASE_CLASSNAMES` is [`provider::q1_base_classnames`],
//! `BaseMonsterSource` is [`monsters::MonsterSource`], and `baseSpecies` is
//! [`species::BASE_SPECIES`]. The donor `MonsterServices` interface has no
//! analogue: monsters hold entity services directly.

pub mod animation;
pub mod creatures;
pub mod finales;
pub mod frames;
pub mod map_entities;
pub mod messages;
pub mod monster_actions;
pub mod monsters;
pub mod player;
pub mod projectiles;
pub mod provider;
pub mod rules;
pub mod species;
pub mod travel;

pub use animation::{MonsterAi, MonsterFrame, MonsterOperation};
pub use monsters::{register_monster_callbacks, BaseMonster, MonsterSource};
pub use player::{Q1CharacterActor, Q1CharacterInput, Q1CharacterOptions, Q1CharacterPresentation, Q1PlayerLife};
pub use projectiles::{drop_backpack, launch_laser, launch_spike, spawn_meat_spray, throw_head, BackpackContents};
pub use provider::{
    q1_base_classnames, register_q1_base, Q1BaseOptions, Q1BaseState, Q1CampaignBinding, Q1CampaignState,
};
pub use rules::{
    q1_client_notice, q1_obituary, Q1ClientNotice, Q1IntermissionResult, Q1IntermissionRule, Q1LevelRules, Q1Obituary,
    Q1ObituaryActor, Q1SourceFinale, Q1SpawnSelector,
};
pub use species::{BaseSpecies, MonsterSpecies, BASE_SPECIES};
pub use travel::{admit_q1_travel, capture_q1_travel, decode_q1_travel, new_q1_travel, Q1TravelState};
