//! Cross-family compatibility shims: versions, demos, userinfo.
//!
//! Minimal donor-justified helpers that belong to no single family
//! crate: protocol version quirks (`src/network/q2/constants.ts`,
//! `src/network/q3/admission.ts`, `src/network/q1/*constants.ts`),
//! demo-file kind detection (`src/network/q1/demos.ts`,
//! `src/network/q3/recording.ts`, `src/app/bootstrap/q2-travel.ts`),
//! and userinfo lookup/cleaning (`src/core/info-string.ts`,
//! `src/content/q3/team-arena/client-admission.ts`). Wire codecs stay
//! in `qa-net`; pair mutation stays in `qa-core`.

pub mod demo;
pub mod q2;
pub mod q3;
pub mod userinfo;
pub mod versions;

mod error;

pub use error::CompatError;
