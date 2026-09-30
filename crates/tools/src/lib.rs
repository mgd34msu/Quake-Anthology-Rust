//! Development tooling: verification runner, inventories, reference captures,
//! policy checks, and build orchestration.
//!
//! Donor provenance: `tools/` in quake-typescript (GPL-2.0-or-later). Library
//! modules mirror the donor subdirectories; thin binaries under `src/bin`
//! expose the executable tools.

pub mod build;
pub mod check_policy;
pub mod content;
pub mod error;
pub mod fsutil;
pub mod inventory;
pub mod js;
pub mod json;
pub mod process;
pub mod reference;
pub mod sha256;
pub mod source_camera;
pub mod sys;
pub mod time;
pub mod verify;
