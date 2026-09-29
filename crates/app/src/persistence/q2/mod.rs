//! Quake II save providers.
//!
//! Donor provenance: `src/persistence/{q2-foundation,q2-base-entities,
//! q2-monsters,q2-items,q2-movers,q2-players,q2-weapons,q2-hand-grenades,
//! q2-missionpacks,q2-typescript,q2-rerelease-state,q2-rerelease,
//! q2-containers,q2-classic,q2-classic-guest}.ts`. [`containers`] holds
//! the engine-owned `server.ssv`/`level.sv2` metadata,
//! [`classic`] the original `game/g_save.c` struct records,
//! [`classic_guest`] the callback-owned original-file overlay,
//! [`typescript`] and [`rerelease`] the JSON source formats, and the
//! remaining modules the TypeScript foundation checkpoints.

pub mod base_entities;
pub mod classic;
pub mod classic_guest;
pub mod containers;
pub mod foundation;
pub mod hand_grenades;
pub mod items;
pub mod missionpacks;
pub mod monsters;
pub mod movers;
pub mod players;
pub mod rerelease;
pub mod rerelease_state;
pub mod typescript;
pub mod weapons;
