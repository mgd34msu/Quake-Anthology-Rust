//! Module ABIs convert into the engine's shared services and owned values.
#![forbid(unsafe_code)]

pub mod abi;
mod hooks;
pub mod memory;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub mod native;
mod numbers;
pub mod quakec;
pub mod qvm;
pub mod services;
