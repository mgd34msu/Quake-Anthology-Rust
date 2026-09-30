//! Q2 classic-dll shims over guest and world.
//!
//! Donor provenance: `src/compat/q2/classic/index.ts` (re-export root)
//! plus `src/compat/q2/classic/*.ts` per submodule.

pub mod combat_binding;
pub mod combat_profile;
pub mod cvars;
pub mod host;
pub mod inventory;
pub mod layout;
pub mod pickup_profile;
pub mod pmove;
pub mod printf;
pub mod records;
pub mod world_profile;
