//! QuakeC guest execution: VM, builtins, entity/host bridges, and mod providers.
//!
//! Port of `src/compat/qc/*` (executor, machine, memory, program, builtins,
//! profile, source-call, save, compatibility, actor/entity/client/movement/
//! presentation/pusher/spatial/world hosts, message effects/routing, and the
//! `mod-*` providers plus the weapon-behavior profile). The barrel
//! `src/compat/qc/index.ts` is absorbed here: every sibling module is public
//! and re-exports its donor surface directly.

pub mod actor_state;
pub mod borrowed_actors;
pub mod builtins;
pub mod client_host;
pub mod compatibility;
pub mod entity_host;
pub mod executor;
pub mod machine;
pub mod memory;
pub mod message_effects;
pub mod message_routing;
pub mod mod_actors;
pub mod mod_clients;
pub mod mod_combat;
pub mod mod_commands;
pub mod mod_environment;
pub mod mod_input;
pub mod mod_items;
pub mod mod_messages;
pub mod mod_objectives;
pub mod mod_pickups;
pub mod mod_protection;
pub mod mod_provider;
pub mod movement_host;
pub mod presentation_host;
pub mod profile;
pub mod program;
pub mod pusher_host;
pub mod quakeworld_presentation;
pub mod save;
pub mod source_call;
pub mod spatial_host;
pub mod weapon_behavior_profile;
pub mod world_host;
