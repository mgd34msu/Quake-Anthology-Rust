//! QVM guest execution: interpreter, syscall bridges, and game/cgame/mod records.
//!
//! Port of `src/compat/qvm/*` (ABI, allocation, artifacts, body-scope,
//! bot/client/cvar/file syscalls, cgame, game combat/data/equipment/input/
//! inventory/pickups/weapons, grapple, guest memory, image, interpreter,
//! item catalog/storage, legacy ABIs, math/memory/snap/vector syscalls,
//! mod-* providers and records, player/render/script/trace records, regions,
//! registry, symbols, syscalls, ui, weapon-behavior profile). The barrel
//! `src/compat/qvm/index.ts` is absorbed here: every sibling module is public
//! and re-exports its donor surface directly.

pub mod abi;
pub mod allocation;
pub mod artifacts;
pub mod body_scope;
pub mod bot_library_syscalls;
pub mod bot_navigation_records;
pub mod bot_navigation_syscalls;
pub mod cgame;
pub mod cgame_body;
pub mod client_audio_syscalls;
pub mod client_browser_syscalls;
pub mod client_cinematic_syscalls;
pub mod client_collision_syscalls;
pub mod client_game_syscalls;
pub mod client_mark_syscalls;
pub mod client_render_syscalls;
pub mod client_script_syscalls;
pub mod client_state;
pub mod client_state_record;
pub mod client_state_syscalls;
pub mod common_syscalls;
pub mod compatibility;
pub mod cvar_syscalls;
pub mod entity_record;
pub mod entity_tokens;
pub mod file_syscalls;
pub mod game;
pub mod game_combat;
pub mod game_combat_binding;
pub mod game_combat_scope;
pub mod game_data;
pub mod game_equipment_movement;
pub mod game_input;
pub mod game_inventory;
pub mod game_pickups;
pub mod game_weapons;
pub mod grapple_profile;
pub mod grapple_provider;
pub mod guest_memory;
pub mod image;
pub mod interpreter;
pub mod item_catalog;
pub mod item_storage;
pub mod legacy_bot_abi;
pub mod legacy_bot_syscalls;
pub mod legacy_client_abi;
pub mod legacy_presentation;
pub mod math_syscalls;
pub mod memory;
pub mod memory_syscalls;
pub mod memory_writes;
pub mod mod_actor_frame;
pub mod mod_actors;
pub mod mod_clients;
pub mod mod_input;
pub mod mod_items;
pub mod mod_objectives;
pub mod mod_pickups;
pub mod mod_player_events;
pub mod mod_presentation;
pub mod mod_presentation_checkpoint;
pub mod mod_protection;
pub mod mod_provider;
pub mod mod_weapon_stage;
pub mod module;
pub mod operations;
pub mod player_record;
pub mod primary_inventory_profile;
pub mod primary_pickup_profile;
pub mod primary_player_profile;
pub mod primary_presentation_profile;
pub mod primary_profile;
pub mod raw_entities;
pub mod regions;
pub mod registry;
pub mod render_record;
pub mod script_record;
pub mod server_game_syscalls;
pub mod server_info_syscalls;
pub mod shared_entity_record;
pub mod snap_vector_syscalls;
pub mod symbols;
pub mod syscalls;
pub mod trace_record;
pub mod ui;
pub mod ui_key_syscalls;
pub mod vector_syscalls;
pub mod weapon_behavior_profile;
