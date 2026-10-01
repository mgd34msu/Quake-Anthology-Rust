//! Quake III collision runtime.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/index.ts`
//! (re-exports; the `Q3Collision` adapter joins this module once the world
//! port lands) and the sibling `src/world/collision/q3/*.ts` donors named
//! per submodule.

pub mod allocation;
pub mod checksum;
pub mod counters;
pub mod map_resource;
pub mod patch;
pub mod polylib;
pub mod settings;

pub use allocation::{
    HunkAccounting, HunkAccountingProfile, HunkAllocation, HunkKind, HunkPreference, SOURCE_HUNK_RELEASE32,
};
pub use checksum::{block_checksum, collision_checksum, collision_lump_checksum, CollisionChecksumLump};
pub use counters::CollisionCounters;
pub use map_resource::{
    decoded_collision_map, BoxSideView, CollisionArea, CollisionBoxHull, CollisionBoxModel, CollisionBrush,
    CollisionBrushSide, CollisionLeaf, CollisionMapData, CollisionMapResource, CollisionModel, CollisionNode,
    CollisionPlane, CollisionShader, IndexRange, Q3BspChild, Q3CollisionBrushInput, Q3CollisionBrushSideInput,
    Q3CollisionGeometry, Q3CollisionLeafInput, Q3CollisionModelInput, Q3CollisionNodeInput, Q3CollisionSurfaceInput,
    Q3CollisionVisibilityInput, Q3SurfaceKind, BODY_CONTENTS,
};
pub use patch::{
    generate_patch_collide, position_in_patch, trace_patch, CollisionDebugHost, CollisionDebugSurface, PatchAllocSite,
    PatchAllocator, PatchBorder, PatchCollide, PatchFacet, PatchHit, PatchPlane, PatchShape,
};
pub use polylib::{
    ClipSplit, CollisionWinding, CollisionWindingLibrary, CollisionWindingMemory, WindingSide, MAX_MAP_BOUNDS,
    ON_EPSILON,
};
pub use settings::{CollisionMapSettings, COLLISION_MAP_CVAR_DEFINITIONS};
