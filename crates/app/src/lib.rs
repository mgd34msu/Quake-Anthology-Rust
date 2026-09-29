//! Application: options, startup, the host main loop, CLI dispatch,
//! console, settings, and debug lines.
//!
//! Ported from the TypeScript donor's `src/main.ts`,
//! `src/app/bootstrap/{options,startup-commands,application,frame-time}.ts`
//! selections, `src/console/{buffer,field,commands,log,session,metrics,
//! discovery,dedicated,source-field}.ts`, `src/settings/{config,restart,
//! server/*}.ts`, and `src/debug/*`. Bots, persistence, LLM edges, and the
//! interactive menu stay out until their donor contracts are ported; the
//! loop here is headless (`NullRenderer` + audio channel pool) and
//! deterministic.
//!
//! Out of scope by plan (see `README.md`): `src/console/draw.ts`
//! (display rendering), `src/console/llm*.ts` and `src/llm/*` (external
//! API clients), and `src/platform/*` (native backends; headless traits
//! already exist).

pub mod application;
pub mod cli;
pub mod console;
pub mod debug;
pub mod error;
pub mod options;
pub mod persistence;
pub mod settings;
pub mod startup;

pub use application::{Application, HeadlessSeat, HostFrame, RunStats, UNBOUND_SEAT};
pub use debug::DebugError;
pub use error::AppError;
