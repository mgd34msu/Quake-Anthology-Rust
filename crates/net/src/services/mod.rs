//! Network services ported from `src/network/services/*`.
//!
//! Donor provenance: `admin.ts`, `discovery.ts`, `downloads.ts`,
//! `http-downloads.ts`, `online.ts`, `rank-codec.ts`, `rankings.ts`. Async
//! donor operations run synchronously here over caller-provided blocking
//! providers; HTTP fetching, TLS, and password hashing are host-owned (see
//! [`online`] and [`http_downloads`]), and JSON saves parse through the
//! shared [`json`] module into [`Json`](crate::common::session::Json).

pub mod admin;
pub mod discovery;
pub mod downloads;
pub mod http_downloads;
pub mod json;
pub mod online;
pub mod rank_codec;
pub mod rankings;
