//! Application bootstrap: startup wiring, browser, demos, audio, effects,
//! network, client, and simulation helpers.
//!
//! Ported from the TypeScript donor's `src/app/bootstrap/*` standalone
//! selections. Later lanes extend this tree; each module documents its donor
//! provenance.

pub mod audio;
pub mod audio_settings;
pub mod component_client_save;
pub mod controller_settings;
pub mod demo_library;
pub mod demo_playback;
pub mod demo_recording;
pub mod effects;
pub mod frame_clock;
pub mod loading;
pub mod local_lobby;
pub mod media;
pub mod menu_art;
pub mod network;
pub mod player_progress;
pub mod player_progress_library;
pub mod player_service_status;
pub mod player_userinfo;
pub mod q1_client_commands;
pub mod q1_client_settings;
pub mod q1_fog;
pub mod q1_localization;
pub mod q1_service_presentation;
pub mod q1_session_actions;
pub mod q1_source_cvars;
pub mod q1_wheel;
pub mod q2_client_commands;
pub mod q2_console;
pub mod q2_damage_blend;
pub mod q2_localization;
pub mod q2_travel;
pub mod q3_client;
pub mod q3_common_cvars;
pub mod q3_map_command;
pub mod remote_seat_identities;
pub mod save_requests;
pub mod server_browser;
pub mod server_browser_addresses;
pub mod server_browser_cache;
pub mod server_master_list;
pub mod simulation;
pub mod startup_commands;
pub mod startup_summary;
pub mod team_arena_demo;
pub mod team_arena_scores;
pub mod weapon_behavior_tool_options;
pub mod weapon_view;
