//! Unified-save codec and shared world-state restore.
//!
//! Donor provenance: `src/persistence/{value,source-bytes,source-json,
//! shared,save-image,provider-ownership,protection,source-items,
//! world-state}.ts`. [`json`] keeps lossless source literals,
//! [`value`] encodes tagged checkpoint payloads, [`bytes`] covers Quake
//! byte strings, [`shared`] reads time/geometry/clock/profile state,
//! [`records`] reads the unified-save shared records, [`ownership`]
//! fixes provider owners, and [`protection`]/[`source_items`]/
//! [`world_state`] restore hidden and shared state through host traits.
//!
//! Why a save-local JSON model instead of `qa-app` settings JSON:
//! dependency direction forbids `world` from depending on `app`, and the
//! save envelope needs lossless literals, `$qts` tags, big integers, and
//! raw bytes that settings documents reject. `qa-guest` and `qa-app`
//! persistence reuse this codec rather than duplicating it.

pub mod bytes;
pub mod json;
pub mod ownership;
pub mod protection;
pub mod records;
pub mod shared;
pub mod source_items;
pub mod value;
pub mod world_state;
