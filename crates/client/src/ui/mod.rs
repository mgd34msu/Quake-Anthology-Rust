//! Seat-owned UI: native menus, HUD overlays, and legacy Q3 menu runtime.
//!
//! Ported from the TypeScript donor's `src/ui/*` (all of `common/`, `hud/`,
//! `library/`, `mods/`, `saves/`, `settings/`) plus the pure UI contract
//! types from `src/contracts/ui.ts` (absorbed as [`types`]). Layout and draw
//! commands are headless data: tests drive them through fake sinks, and the
//! application emits them through the render and text ports. Rendering itself
//! lives in [`crate::render`] and text in [`crate::text`]; this module builds
//! on those types without duplicating them.

pub mod common;
pub mod hud;
pub mod library;
pub mod mods;
pub mod saves;
pub mod settings;
pub mod types;
