//! Quake II game-content provider (`src/content/q2`, `src/content/composition/q2`).
//!
//! Donor provenance: all of `src/content/q2` plus
//! `src/content/composition/q2`, `src/content/composition/q1-q2-supply.ts`
//! and `src/content/composition/q2-expansion-arsenal.ts`.
//!
//! The donor is class-per-module TypeScript with shared mutable entity
//! objects. The Rust port keeps one central state arena
//! ([`foundation::host::Q2GameServices`]) plus free functions and plain
//! `fn` callbacks, so the borrow checker enforces the donor's callback
//! discipline instead of an aliasing runtime:
//!
//! * Entities live in the arena and are addressed by [`ActorId`] handles.
//!   A function that needs services takes `(ActorId, &mut Q2GameServices)`
//!   and borrows the entity only for tight scopes; a function that needs
//!   only entity data takes `&Q2Entity`/`&mut Q2Entity` and no services.
//! * Source callbacks are `fn` pointers registered by name
//!   ([`foundation::callbacks`]), exactly like the donor's save tables.
//! * Monster logic runs through the transient
//!   [`foundation::monsters::types::MonsterContext`] facade, which borrows
//!   the arena; per-monster state stays in the arena's monster runtime.
//! * Vectors are [`qa_core::math::Vec3`] (`f32`, matching the original C
//!   and the rest of this workspace); gameplay scalars stay `f64` like the
//!   donor. Donor `Math.fround` on a scalar becomes `as f32`.
//! * Donor `Map` iteration order is insertion order; where order matters
//!   the port sorts explicitly (spawn fields use a [`BTreeMap`], entity
//!   capture sorts by `(slot, generation)`).
//! * Donor `throw` on internal invariants (missing context, duplicate
//!   registration, unnamed callback) becomes `panic!` with the donor
//!   message; parse/decode/checkpoint paths return `Result`.
//!
//! [`ActorId`]: qa_core::identity::ActorId
//! [`BTreeMap`]: std::collections::BTreeMap

pub mod base;
pub mod equipment;
pub mod foundation;
pub mod missionpacks;
pub mod multiplayer;
pub mod rerelease;
pub mod support;
