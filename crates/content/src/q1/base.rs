//! Q1 base-game root. The base worker owns every other module here;
//! `messages` and `finales` are ported with the foundation because
//! foundation text formatting depends on them.

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
