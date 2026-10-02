//! Shared foundation: math, numerics, randomness, time, identity, command
//! text, cvars, and bounded byte storage. Depends on no internal crate.

pub mod binary;
pub mod cmd;
pub mod cmd_buffer;
pub mod common_error;
pub mod cvar;
pub mod diagnostics;
pub mod identity;
pub mod math;
pub mod numeric;
pub mod omnitimer;
pub mod resource_name_index;
pub mod rng;
pub mod time;
