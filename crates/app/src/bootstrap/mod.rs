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
pub mod demo_recording_commands;
pub mod diagnostic_tools;
pub mod effects;
pub mod finale;
pub mod frame_clock;
pub mod frame_time;
pub mod frontend_preferences;
pub mod game_prompt;
pub mod gtv_commands;
pub mod held_weapon;
pub mod image_reader;
pub mod image_settings;
pub mod input;
pub mod input_devices;
pub mod keys;
pub mod loading;
pub mod local_lobby;
pub mod local_seat_change;
pub mod match_modes;
pub mod match_preflight;
pub mod media;
pub mod menu_art;
pub mod menu_font;
pub mod mod_selection;
pub mod model_loader;
pub mod native_held_weapon;
pub mod native_q2_client;
pub mod network;
pub mod original_save;
pub mod player_death;
pub mod player_progress;
pub mod player_progress_library;
pub mod player_service_status;
pub mod q1_session_actions;
pub mod q2_damage_blend;
pub mod q2_localization;
pub mod q2_travel;
pub mod q3_client;
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
