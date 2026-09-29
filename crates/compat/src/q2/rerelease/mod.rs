//! Q2 rerelease-dll shims over guest and world.
//!
//! Donor provenance: `src/compat/q2/rerelease/index.ts` (re-export root)
//! plus `src/compat/q2/rerelease/*.ts` per submodule.

pub mod api;
pub mod body_shape;
pub mod cgame;
pub mod client_profile;
pub mod combat_binding;
pub mod debug_shapes;
pub mod deferred_damage;
pub mod equipment_movement;
pub mod foreign_actors;
pub mod host;
pub mod imports;
pub mod layouts;
pub mod messages;
pub mod module;
pub mod native_entries;
pub mod native_weapon_declaration;
pub mod navigation;
pub mod pickup_profile;
pub mod pickup_protection;
pub mod player_state;
pub mod public_state;
pub mod q2eaks_weapon_profile;
pub mod semantics;
pub mod sounds;
pub mod source_state;
pub mod spatial;
pub mod weapon_behavior_profile;
pub mod world_profile;
pub mod world_text;
