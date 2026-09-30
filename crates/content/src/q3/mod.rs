//! Quake III content records (`src/content/q3/*`), donor-mirrored.
//!
//! This tree ports the seam-touching Q3 content modules: shared base-game
//! definitions and items (`base/shared`), the source arsenal adapter
//! (`foundation`), guest item catalogs (`guest-items`), input profiles,
//! equipment QVM profiles (`equipment`), and cgame body submissions
//! (`presentation`). Sibling-owned Q3 files land in the same layout and are
//! united at merge; only the modules below are this lane's scope.
//! The core-resume lane adds the remaining content-game modules in the same
//! donor-mirrored layout.

pub mod base;
pub mod equipment;
pub mod foundation;
pub mod guest_items;
pub mod input_profile;
pub mod presentation;
pub mod product_restriction;
pub mod team_arena;
#[cfg(test)]
pub(crate) mod test_support;
