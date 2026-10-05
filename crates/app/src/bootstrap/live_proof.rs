//! Live-proof test support: one gate for tests that prove behavior against
//! real Steel corpus data or a real display.
//!
//! Converted live-proof tests carry `#[ignore]` (default `cargo test` skips
//! them) and run in the live gate as
//! `QA_REQUIRE_LIVE=1 xvfb-run -a cargo test -p qa-app -- --ignored`.
//! Every helper below panics when `QA_REQUIRE_LIVE=1` and its data is
//! missing, and otherwise prints one loud skip line and returns `None` so
//! the test returns. A gate run without corpus/display therefore fails
//! instead of reporting green while proving nothing.

use std::path::PathBuf;

/// Environment switch that turns live-proof skips into failures.
pub const REQUIRE_LIVE_ENV: &str = "QA_REQUIRE_LIVE";

/// Witness files proving a Steel corpus root holds all three game families.
pub const CORPUS_WITNESSES: [&str; 3] = ["q1/id1/pak0.pak", "q2/baseq2/pak0.pak", "q3a/baseq3/pak0.pk3"];

/// Whether live proofs must run (`QA_REQUIRE_LIVE=1` in the environment).
#[must_use]
pub fn live_proofs_required() -> bool {
    std::env::var(REQUIRE_LIVE_ENV).as_deref() == Ok("1")
}

/// First ancestor `target/` directory (walking up from this crate) holding
/// every `witness` entry. Entries may be files or directories.
fn find_corpus_with(witnesses: &[&str]) -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("target"))
        .find(|root| witnesses.iter().all(|witness| root.join(witness).exists()))
}

/// First ancestor `target/` directory holding Steel data for any entry of
/// `families` (family directory names such as `"q1"`).
fn find_corpus_with_any(families: &[&str]) -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("target"))
        .find(|root| families.iter().any(|family| root.join(family).exists()))
}

/// Require an already-located live value (map bytes, catalog, witness
/// file): `Some` passes through, `None` skips loudly, or panics when
/// `QA_REQUIRE_LIVE=1`.
pub fn require_live_data<T>(what: &str, found: Option<T>) -> Option<T> {
    match found {
        Some(value) => Some(value),
        None => {
            if live_proofs_required() {
                panic!("{REQUIRE_LIVE_ENV}=1: live proof needs {what}, which is missing");
            }
            eprintln!("skipped: LIVE PROOF needs {what}; re-run with {REQUIRE_LIVE_ENV}=1 to fail instead");
            None
        }
    }
}

/// Require a Steel corpus root holding every `witness` entry: skips loudly,
/// or panics when `QA_REQUIRE_LIVE=1`. `what` names the needed data, e.g.
/// `"Q1 Steel data"`.
pub fn require_live_corpus(what: &str, witnesses: &[&str]) -> Option<PathBuf> {
    let found = find_corpus_with(witnesses);
    if found.is_some() {
        return found;
    }
    require_live_data(
        &format!("{what} (ancestor target/ with {})", witnesses.join(", ")),
        None,
    )
}

/// Require a Steel corpus root holding Steel data for any entry of
/// `families`: skips loudly, or panics when `QA_REQUIRE_LIVE=1`.
pub fn require_live_corpus_any(what: &str, families: &[&str]) -> Option<PathBuf> {
    let found = find_corpus_with_any(families);
    if found.is_some() {
        return found;
    }
    require_live_data(
        &format!("{what} (ancestor target/ with any of {})", families.join(", ")),
        None,
    )
}

/// Require a windowed open to succeed: `Ok` passes through, `Err` keeps the
/// honest-open-failure contract (the error must be non-empty) and then
/// skips loudly, or panics when `QA_REQUIRE_LIVE=1`.
pub fn require_live_window<T, E: std::fmt::Display>(what: &str, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(opened) => Some(opened),
        Err(error) => {
            let detail = error.to_string();
            assert!(!detail.is_empty(), "honest open failure");
            require_live_data(&format!("{what} (open failed: {detail})"), None)
        }
    }
}
