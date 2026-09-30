//! Q2 native-mod bridges: classic + rerelease shims over guest and world.
//!
//! Donor provenance: `src/compat/q2/*.ts` (top-level native/primary/mod
//! bridges). Submodules `classic` and `rerelease` cover
//! `src/compat/q2/classic/*.ts` and `src/compat/q2/rerelease/*.ts`.
//! Built on the `qa-guest` loaders/ABI and `qa-world`, never duplicated.

pub mod classic;
pub mod compatibility;
pub mod native_combat_call;
pub mod native_damage;
pub mod native_input;
pub mod native_mod_actors;
pub mod native_mod_armor;
pub mod native_mod_client_stages;
pub mod native_mod_clients;
pub mod native_mod_combat;
pub mod native_mod_deferred;
pub mod native_mod_entries;
pub mod native_mod_invocations;
pub mod native_mod_items;
pub mod native_mod_pickups;
pub mod native_mod_protection;
pub mod native_mod_provider;
pub mod native_mod_region;
pub mod native_mod_weapon_stage;
pub mod native_pickups;
pub mod native_primary;
pub mod native_primary_command_profile;
pub mod native_primary_commands;
pub mod native_primary_drop;
pub mod native_primary_drop_profile;
pub mod native_primary_inventory;
pub mod native_primary_inventory_profile;
pub mod native_primary_player;
pub mod native_primary_player_profile;
pub mod native_primary_profiles;
pub mod native_primary_reader;
pub mod native_primary_validation;
pub mod native_primary_weapon_profile;
pub mod native_primary_weapons;
pub mod rerelease;
