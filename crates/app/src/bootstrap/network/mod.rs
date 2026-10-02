//! Bootstrap network helpers (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/*`).
//!
//! Barrel (donor `index.ts`): `types`, `q2`, `q2-layout`, `q1-types`, `q1`,
//! `q3-types`, and `q3` publish through their modules below. `remote.ts`
//! has no Rust module yet (owned by another lane) and stays unwired here.

pub mod client_download_policy;
pub mod gtv_source;
pub mod q1;
pub mod q1_client;
pub mod q1_demo;
pub mod q1_types;
pub mod q2;
pub mod q2_client_receiver;
pub mod q2_demo;
pub mod q2_downloads;
pub mod q2_effects;
pub mod q2_layout;
pub mod q2_mvd_presentation;
pub mod q2_remote_view;
pub mod q2_rerelease_hud_events;
pub mod q2_service_presentation;
pub mod q3;
pub mod q3_client;
pub mod q3_client_content;
pub mod q3_client_downloads;
pub mod q3_demo;
pub mod q3_downloads;
pub mod q3_types;
pub mod qw_camera;
pub mod qw_client;
pub mod qw_downloads;
pub mod qw_server;
pub mod qw_server_types;
pub mod qw_skins;
pub mod qw_types;
pub mod recorded_source;
pub mod remote;
pub mod remote_q1;
pub mod remote_q3;
pub mod remote_qw;
pub mod remote_unified;
pub mod remote_world;
pub mod socks_settings;
pub mod transport;
pub mod types;
pub mod unified_client;
pub mod unified_component_consumer;
pub mod unified_component_publication;
pub mod unified_components;
pub mod unified_content;
pub mod unified_control;
pub mod unified_event_codec;
pub mod unified_frame_codec;
pub mod unified_frame_values;
pub mod unified_native_components;
pub mod unified_native_consumer;
pub mod unified_prediction;
pub mod unified_server;
pub mod unified_types;
