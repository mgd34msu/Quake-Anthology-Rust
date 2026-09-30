//! Q2 source-derived capture (donor `tools/reference/q2/capture.ts`).
//!
//! Validates case source spans, evaluates every case through the oracle,
//! and assembles the capture document. Port notes:
//!
//! * Model identities name the Rust sources that replaced the donor scripts:
//!   `capture.rs`, `oracle.rs`, `sources.rs`, and `cases.rs` stand in for
//!   their TypeScript namesakes (plus the donor case-data authority, which
//!   is still identified); `json.rs` replaces `verification/schema/contracts.ts`
//!   as the JSON value model; `Cargo.toml` replaces `tsconfig.json` as the
//!   build manifest.
//! * Capture/check commands name the library entry point
//!   (`reference::q2::capture::run`) because the port keeps the existing
//!   binary surface unchanged.
//! * The runtime record keeps the donor keys with the executing tool version.

use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::json::{deep_strict_equal, Json};
use crate::reference::environment::{identify_file, quake_typescript_root};
use crate::reference::q2::cases::q2_cases;
use crate::reference::q2::oracle::evaluate;
use crate::reference::q2::sources::{load_verified_sources, source_text};
use crate::reference::{node_arch, node_platform};
use crate::time::now_iso;

/// Model inputs identified on every capture.
fn model_paths(project_root: &Path) -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        manifest.join("src/reference/q2/capture.rs"),
        manifest.join("src/reference/q2/oracle.rs"),
        manifest.join("src/reference/q2/sources.rs"),
        manifest.join("src/reference/q2/cases.rs"),
        project_root.join("verification/reference-cases/q2/cases.ts"),
        manifest.join("src/reference/environment.rs"),
        manifest.join("src/reference/schema.rs"),
        manifest.join("src/json.rs"),
        manifest.join("Cargo.toml"),
    ]
}

/// Capture limits recorded on every capture (donor verbatim).
fn capture_limits() -> Vec<String> {
    [
        "The checks compare a bounded independent equation/control-flow transcription to reviewed literal expectations and pinned original source. They do not prove a complete TS port or original binary equivalent.",
        "Save cases project FIELD_AUTO declarations from original source. They do not run a save codec, reference relocation, file I/O lifecycle, or fresh-process restore.",
        "Pickup feedback is represented by an ordered semantic marker; individual image/sound network bytes are not captured.",
        "Frame actor mutations are authored test actions. Allocator reuse, arbitrary nested damage, physics, and mixed-family scheduling require further cases.",
        "Native command predicates establish classic upmove and rerelease button meanings only. LegacyKEX conversion magnitudes, both-buttons precedence, 4038 wire encoding, and LMCTF prediction agreement remain project adapter obligations without independent original authority here.",
        "Q64 cases cover the original configuration block and input predicates, not movement trajectories or server/client runtime agreement.",
        "Retail assets, render/audio outputs, network packets, performance, and gameplay sessions were not exercised.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Evaluate every Q2 case and assemble the capture document.
pub fn capture_q2() -> Result<Json, ToolsError> {
    let project_root = quake_typescript_root();
    let sources = load_verified_sources(&project_root)?;
    let mut rendered = Vec::new();
    for reference in q2_cases()? {
        for location in &reference.sources {
            let line_count = source_text(&sources.text, location.source.as_str())?.split('\n').count();
            if location.first_line < 1
                || location.last_line < location.first_line
                || u64::from(location.last_line) > line_count as u64
            {
                return Err(ToolsError::invalid(format!(
                    "Invalid original-source span: {}/{}",
                    reference.id,
                    location.source.as_str()
                )));
            }
        }
        let actual = evaluate(&reference.input, &sources.text)?;
        let passed = deep_strict_equal(&actual, &reference.expected);
        rendered.push((reference.to_json(&actual, passed), passed));
    }
    let passed = rendered.iter().all(|(_, passed)| *passed);
    let cases: Vec<Json> = rendered.into_iter().map(|(rendered, _)| rendered).collect();
    let mut model_identities = Vec::with_capacity(9);
    for path in model_paths(&project_root) {
        model_identities.push(identify_file(&path.to_string_lossy())?.to_json());
    }
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    let audit = project_root.join("docs/research/q2-interoperability.md");
    Ok(Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("id".to_owned(), Json::string("q2-initial-source-derived-contracts")),
        ("oracleKind".to_owned(), Json::string("source-derived")),
        ("capturedAt".to_owned(), Json::string(now_iso())),
        (
            "captureCommand".to_owned(),
            Json::array(vec![Json::string("qa-tools"), Json::string("reference::q2::capture::run")]),
        ),
        (
            "checkCommand".to_owned(),
            Json::array(vec![
                Json::string("qa-tools"),
                Json::string("reference::q2::capture::run"),
                Json::string("--check"),
            ]),
        ),
        ("cwd".to_owned(), Json::string(project_root.to_string_lossy())),
        (
            "runtime".to_owned(),
            Json::object(vec![
                ("toolVersion".to_owned(), Json::string(format!("qa-tools/{}", env!("CARGO_PKG_VERSION")))),
                ("platform".to_owned(), Json::string(node_platform())),
                ("architecture".to_owned(), Json::string(node_arch())),
                ("executable".to_owned(), identify_file(&exe.to_string_lossy())?.to_json()),
            ]),
        ),
        (
            "originalSources".to_owned(),
            Json::array(sources.identities.iter().map(crate::reference::q2::sources::SourcedIdentity::to_json).collect()),
        ),
        ("modelIdentities".to_owned(), Json::array(model_identities)),
        ("auditContext".to_owned(), identify_file(&audit.to_string_lossy())?.to_json()),
        (
            "numericContract".to_owned(),
            Json::string(
                "Explicit binary32 stores/products for selected classic expressions; binary64 unsuffixed FRAMETIME evaluation; exact bounded integer RR milliseconds. Native compiler excess-precision behavior is not measured.",
            ),
        ),
        (
            "nativeEngineExecution".to_owned(),
            Json::object(vec![
                ("status".to_owned(), Json::string("NOT_RUN")),
                (
                    "reason".to_owned(),
                    Json::string(
                        "No original Q2 engine or DLL executable was invoked. These are independent source-derived TypeScript evaluations.",
                    ),
                ),
            ]),
        ),
        ("limits".to_owned(), Json::array(capture_limits().iter().map(Json::string).collect())),
        ("assertionCount".to_owned(), Json::uint(cases.len() as u64)),
        ("passed".to_owned(), Json::boolean(passed)),
        ("cases".to_owned(), Json::array(cases)),
    ]))
}

fn capture_field<'a>(capture: &'a Json, key: &str) -> Result<&'a Json, ToolsError> {
    capture.get(key).ok_or_else(|| ToolsError::parse(format!("Capture is missing {key}")))
}

/// Run the Q2 capture (donor `main`): write `capture.json` unless `--check`
/// is given, print the summary line, and return the exit code.
pub fn run(args: &[String]) -> Result<i32, ToolsError> {
    if args.iter().any(|arg| arg != "--check") {
        return Err(ToolsError::invalid("Usage: qa-tools reference::q2::capture::run [--check]"));
    }
    let capture =
        capture_q2().map_err(|error| ToolsError::invalid(format!("Q2 source reference capture failed: {error}")))?;
    if !args.iter().any(|arg| arg == "--check") {
        let destination = quake_typescript_root().join("verification/reference-cases/q2/capture.json");
        crate::fsutil::write_text(&destination, &format!("{}\n", capture.render_pretty()))?;
    }
    let cases = capture_field(&capture, "cases")?
        .as_array()
        .ok_or_else(|| ToolsError::parse("Capture cases are not an array"))?;
    let mut failed = Vec::new();
    for case in cases {
        if capture_field(case, "passed")?.as_bool() != Some(true) {
            failed.push(capture_field(case, "id")?.clone());
        }
    }
    let summary = Json::object(vec![
        ("id".to_owned(), capture_field(&capture, "id")?.clone()),
        ("oracleKind".to_owned(), capture_field(&capture, "oracleKind")?.clone()),
        ("cases".to_owned(), capture_field(&capture, "assertionCount")?.clone()),
        ("passed".to_owned(), capture_field(&capture, "passed")?.clone()),
        ("failed".to_owned(), Json::array(failed)),
        ("nativeEngineExecution".to_owned(), capture_field(&capture, "nativeEngineExecution")?.clone()),
    ]);
    println!("{}", summary.render());
    Ok(if capture_field(&capture, "passed")?.as_bool() == Some(true) { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_arguments() {
        let error = run(&["--bogus".to_owned()]).expect_err("usage");
        assert!(error.to_string().starts_with("Usage:"), "{error}");
    }

    #[test]
    fn capture_passes_all_cases() {
        let capture = capture_q2().expect("capture");
        assert_eq!(capture.get("id").and_then(Json::as_str), Some("q2-initial-source-derived-contracts"));
        assert_eq!(capture.get("assertionCount").and_then(Json::as_f64), Some(32.0));
        assert_eq!(capture.get("passed").and_then(Json::as_bool), Some(true));
        let cases = capture.get("cases").and_then(Json::as_array).expect("cases array");
        assert_eq!(cases.len(), 32);
        for case in cases {
            assert_eq!(case.get("passed").and_then(Json::as_bool), Some(true), "{}", case.render());
        }
    }

    #[test]
    fn check_mode_returns_zero() {
        assert_eq!(run(&["--check".to_owned()]).expect("check"), 0);
    }
}
