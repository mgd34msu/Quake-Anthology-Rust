//! Developer-tool LLM edges: providers, streaming, catalogs, settings.
//!
//! Donor provenance: `src/llm/{api,auth,codex,errors,models,request,
//! responses,settings,sse}.ts`. The donor is async over `fetch`; this
//! port is synchronous over an injectable [`request::LlmFetch`], so unit
//! and integration tests run with stub fetchers and never touch the
//! network. JSON reuses [`crate::settings::json`]; randomness comes from
//! the OS with a time-seeded fallback; SHA-256 (OAuth PKCE) is local.
//!
//! [`settings::LlmSettingsService`] is the composition root: file-backed
//! credentials and preferences, model discovery, subscription refresh,
//! and provider routing for [`LlmRequestInput`](request::LlmRequestInput).

pub mod api;
pub mod auth;
pub mod codex;
pub mod errors;
pub mod models;
pub mod request;
pub mod responses;
pub mod settings;
pub mod sse;
