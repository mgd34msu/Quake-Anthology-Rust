//! Network-common foundation ported from `src/network/common/*`.
//!
//! Donor provenance: `endpoint.ts`, `transport.ts`, `loopback.ts`,
//! `reliability.ts`, `fragments.ts`, `scheduling.ts`, `session.ts`,
//! `socks.ts`, `ipx.ts`, `ipx-host.ts`, `ipx-dosbox.ts`, `commands.ts`,
//! `value.ts`. Streams and sockets use [`std::net`] only; there is no async
//! runtime. Hashing uses the self-contained [`hash`] module (SHA-256) so the
//! crate keeps zero new dependencies.

pub mod commands;
pub mod endpoint;
pub mod fragments;
pub mod hash;
pub mod ipx;
pub mod loopback;
pub mod reliability;
pub mod scheduling;
pub mod session;
pub mod socks;
pub mod transport;
