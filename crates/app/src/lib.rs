//! Application: options, startup, the host main loop, CLI dispatch,
//! console, settings, and debug lines.
//!
//! Ported from the TypeScript donor's `src/main.ts`,
//! `src/app/bootstrap/{options,startup-commands,application,frame-time}.ts`
//! selections, `src/console/{buffer,field,commands,log,session,metrics,
//! discovery,dedicated,source-field,draw,llm,llm-batch}.ts`,
//! `src/settings/{config,restart,server/*}.ts`, `src/llm/*`, and
//! `src/debug/*`. Bots and the interactive menu complete under their
//! owning port lanes; the dedicated-server loop here runs without
//! presentation (`NullRenderer` + audio channel pool) and
//! deterministically. Native backends live in `qa-platform`.

pub mod application;
pub mod bootstrap;
pub mod cli;
pub mod console;
pub mod debug;
pub mod directories;
pub mod error;
pub mod llm;
pub mod options;
pub mod persistence;
pub mod settings;
pub mod startup;

pub use application::{Application, HeadlessSeat, HostFrame, RunStats, UNBOUND_SEAT};
pub use debug::DebugError;
pub use error::AppError;
