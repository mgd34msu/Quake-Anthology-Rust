//! Guest game logic: QC-style game-module interface over the headless
//! world. Modules implement [`traits::GameModule`] (sync
//! spawn/think/touch/use/pain/die over named entity fields), the
//! [`registry::ModuleRegistry`] dispatches in registration order with
//! invocation tracking, and [`save`] checkpoints field state. Donor:
//! `src/contracts/world.ts` (`ActorCallbacks`), `src/contracts/execution.ts`
//! (guest checkpoints), `src/world/actors/callbacks.ts` (invocation stack),
//! `src/compat/qc/entity-host.ts` (edict fields).

// `GuestError` is intentionally descriptive; boxing it at every fallible
// boundary would obscure signatures for no runtime gain.
#![allow(clippy::result_large_err)]

pub mod abi;
pub mod checkpoint;
pub mod core;
pub mod elf;
pub mod error;
pub mod fields;
pub mod floating_point;
pub mod pe;
pub mod qc;
pub mod qvm;
pub mod registry;
pub mod runtime;
pub mod save;
pub mod server;
pub mod traits;
pub mod x64;
pub mod x86;

pub use error::GuestError;
