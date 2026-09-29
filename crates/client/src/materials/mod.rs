//! Materials: shader parsing, compilation, and data-level evaluation.
//!
//! Ported from the TypeScript donor's `src/materials/*`: shader scripts
//! (`material`), compilation (`compile`), finishing (`material-finish`),
//! iterators (`material-iterator`), stage color (`color`), deformations
//! plus noise (`deform`), fog volumes and legacy fog (`fog`,
//! `legacy-fog`), sky (`sky`), Q1/Q2 lightmaps (`lighting`), Q2 fragment
//! and grid lighting (`q2-lighting`, `q2-lightgrid`), Q3 grid lighting
//! (`q3-lighting`), classic materials (`legacy`), projected dlights
//! (`dlight`), turbulence (`turbulence`), source state bits
//! (`source-state`), geometry (`geometry`), cinematic sources
//! (`cinematic`), and ordered evaluation (`evaluate`).
//!
//! Data level plus evaluation math only: no GPU calls are made from
//! these modules, keeping `NullRenderer` compatibility. Image handles
//! are opaque `u32` values resolved by the registration host.

pub mod cinematic;
pub mod color;
pub mod compile;
pub mod deform;
pub mod dlight;
pub mod evaluate;
pub mod finish;
pub mod fog;
pub mod geometry;
pub mod iterator;
pub mod legacy;
pub mod lighting;
pub mod material;
pub mod q2_lighting;
pub mod q3_lighting;
pub mod sky;
pub mod state;
pub mod turbulence;
