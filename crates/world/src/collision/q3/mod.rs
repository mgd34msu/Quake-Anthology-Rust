//! Quake III collision runtime.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/index.ts`
//! (re-exports; the `Q3Collision` adapter joins this module once the world
//! port lands) and the sibling `src/world/collision/q3/*.ts` donors named
//! per submodule.

pub mod allocation;
pub mod checksum;
pub mod counters;

pub use allocation::{
    HunkAccounting, HunkAccountingProfile, HunkAllocation, HunkKind, HunkPreference, SOURCE_HUNK_RELEASE32,
};
pub use checksum::{block_checksum, collision_checksum, collision_lump_checksum, CollisionChecksumLump};
pub use counters::CollisionCounters;
