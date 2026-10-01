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
pub mod q1_session_actions;
pub mod q2_damage_blend;
pub mod q2_localization;
pub mod q2_travel;
pub mod q3_client;
pub mod remote_seat_identities;
pub mod remote_seat_source;
pub mod remote_seat_view;
pub mod remote_seats;
pub mod render_settings;
pub mod renderer;
pub mod renderer_diagnostics;
pub mod save_requests;
pub mod seat_hud_state;
pub mod server_administration;
pub mod server_browser;
pub mod server_browser_addresses;
pub mod server_browser_cache;
pub mod server_master_list;
pub mod shared_setting_cvars;
pub mod simulation;
pub mod sound_captions;
pub mod startup_audio;
pub mod startup_commands;
pub mod startup_config;
pub mod startup_input_profile;
pub mod startup_saves;
pub mod startup_summary;
pub mod team_arena_demo;
pub mod team_arena_overrides;
pub mod team_arena_results;
pub mod team_arena_scores;
pub mod team_arena_skirmish;
pub mod video_restart;
pub mod view_settings;
pub mod weapon_behavior_selection;
pub mod weapon_behavior_tool;
pub mod weapon_behavior_tool_options;
pub mod weapon_hud;
pub mod weapon_view;
