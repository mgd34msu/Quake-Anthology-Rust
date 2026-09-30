//! Q3 source-derived capture (donor `tools/reference/q3/capture.ts`).
//!
//! Verifies the pinned original sources, evaluates every scenario, and
//! assembles the capture document. Port notes: capture-program identities
//! name the Rust sources plus the crate and workspace manifests (standing in
//! for `tsconfig.json` and `bun.lock`); the command names the library entry
//! point because the port keeps the existing binary surface; the runtime
//! version records the executing tool.

use std::path::Path;
use std::time::Instant;

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::reference::environment::{identify_file, quake_typescript_root, source_root};
use crate::reference::q3::scenarios::evaluate_scenarios;
use crate::reference::q3::sources::SOURCE_PINS;
use crate::reference::{node_arch, node_platform};
use crate::time::now_iso;

/// Capture-program identities: Rust sources plus build manifests.
fn capture_program_paths() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = fsutil::lexical_absolute(manifest, "../..");
    [
        manifest.join("src/reference/q3/capture.rs"),
        manifest.join("src/reference/q3/semantics.rs"),
        manifest.join("src/reference/q3/sources.rs"),
        manifest.join("src/reference/environment.rs"),
        manifest.join("src/reference/schema.rs"),
        manifest.join("Cargo.toml"),
        root.join("Cargo.lock"),
    ]
    .into_iter()
    .map(|path| path.to_string_lossy().into_owned())
    .collect()
}

/// Capture limits recorded on every capture (donor verbatim).
fn capture_limits() -> Vec<String> {
    [
        "PASS establishes agreement between the source-derived evaluator and separately written literal expectations, not parity of the TypeScript engine or a measured original executable.",
        "The evaluator imports no code from the Quake III TypeScript donor. Expected values come from the listed original C operations, fixed arithmetic examples, and explicit fixture assumptions.",
        "This bounded set omits complete movement/collision, active game entities, actual QVM instructions, server sockets, rendering, audio, campaigns, and performance.",
        "C compiler floating-point behavior can differ with excess precision, reassociation, or a different runtime profile. These records pin an explicit numeric profile rather than inferring all native builds behave identically.",
        "ClientConnect import results and string offset are synthetic fixture inputs. Observable calls and immediate return ordering are source-derived, not packet captures or measured retail traces.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Evaluate every Q3 scenario and assemble the capture document.
pub fn capture_q3_reference(command: &[String]) -> Result<Json, ToolsError> {
    let started_at = now_iso();
    let started = Instant::now();
    let project_root = quake_typescript_root();
    let source_base = format!("{}/quake-iii-arena", source_root());
    let mut sources = Vec::with_capacity(SOURCE_PINS.len());
    for pin in SOURCE_PINS {
        let identity = identify_file(&format!("{source_base}/{}", pin.path))?;
        if identity.sha256 != pin.sha256 {
            return Err(ToolsError::invalid(format!(
                "Original source hash mismatch: {}; review the source and expectations before changing this pin",
                pin.path
            )));
        }
        sources.push(identity.to_json());
    }
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    let runtime_executable = identify_file(&exe.to_string_lossy())?.to_json();
    let mut capture_program = Vec::new();
    for path in capture_program_paths() {
        capture_program.push(identify_file(&path)?.to_json());
    }
    let scenarios_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference/q3/scenarios.rs").to_string_lossy().into_owned();
    let fixture_files = vec![identify_file(&scenarios_path)?.to_json()];
    let scenarios = evaluate_scenarios();
    let mut rendered = Vec::with_capacity(scenarios.len());
    let mut assertion_count = 0_usize;
    let mut status = "PASS";
    for scenario in &scenarios {
        for assertion in &scenario.assertions {
            assertion_count += 1;
            if !assertion.passed {
                status = "FAIL";
            }
        }
        rendered.push(scenario.to_json());
    }
    Ok(Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("oracleKind".to_owned(), Json::string("source-derived")),
        ("engineFamily".to_owned(), Json::string("q3")),
        ("command".to_owned(), Json::array(command.iter().map(Json::string).collect())),
        ("cwd".to_owned(), Json::string(project_root.to_string_lossy())),
        ("startedAt".to_owned(), Json::string(started_at)),
        ("finishedAt".to_owned(), Json::string(now_iso())),
        ("durationMs".to_owned(), Json::float(started.elapsed().as_secs_f64() * 1000.0)),
        ("sources".to_owned(), Json::array(sources)),
        (
            "runtime".to_owned(),
            Json::object(vec![
                ("executable".to_owned(), runtime_executable),
                ("toolVersion".to_owned(), Json::string(format!("qa-tools/{}", env!("CARGO_PKG_VERSION")))),
                ("platform".to_owned(), Json::string(node_platform())),
                ("architecture".to_owned(), Json::string(node_arch())),
            ]),
        ),
        ("captureProgram".to_owned(), Json::array(capture_program)),
        ("fixtureFiles".to_owned(), Json::array(fixture_files)),
        (
            "environment".to_owned(),
            Json::object(vec![(
                "LC_ALL".to_owned(),
                Json::string(std::env::var("LC_ALL").unwrap_or_else(|_| "inherited-unset".to_owned())),
            )]),
        ),
        ("rng".to_owned(), Json::object(vec![("kind".to_owned(), Json::string("unused"))])),
        (
            "clock".to_owned(),
            Json::object(vec![
                ("kind".to_owned(), Json::string("synthetic-input")),
                (
                    "schedule".to_owned(),
                    Json::string(
                        "Exact integer times and call sequence appear in each scenario input; the fixture file is SHA-256 identified.",
                    ),
                ),
            ]),
        ),
        ("network".to_owned(), Json::object(vec![("kind".to_owned(), Json::string("not-executed"))])),
        ("content".to_owned(), Json::object(vec![("kind".to_owned(), Json::string("not-required"))])),
        (
            "originalExecutable".to_owned(),
            Json::object(vec![
                ("kind".to_owned(), Json::string("not-run")),
                (
                    "reason".to_owned(),
                    Json::string(
                        "This runner evaluates bounded transcriptions of pinned original source operations. It does not invoke a native, retail, donor, or QVM executable.",
                    ),
                ),
            ]),
        ),
        (
            "numericProfile".to_owned(),
            Json::string(
                "binary32-per-float-operation; binary64-double-literal-promotion; round-to-nearest-ties-to-even; int-conversion-toward-zero; bounded-int32-inputs",
            ),
        ),
        (
            "tolerances".to_owned(),
            Json::array(vec![Json::object(vec![
                ("metric".to_owned(), Json::string("source-derived-values-and-event-order")),
                ("absolute".to_owned(), Json::int(0)),
                ("relative".to_owned(), Json::int(0)),
            ])]),
        ),
        ("status".to_owned(), Json::string(status)),
        ("assertionCount".to_owned(), Json::uint(assertion_count as u64)),
        ("scenarios".to_owned(), Json::array(rendered)),
        ("limits".to_owned(), Json::array(capture_limits().iter().map(Json::string).collect())),
    ]))
}

/// Run the Q3 capture (donor `main`): capture or `--check`, then summarize.
pub fn run(args: &[String]) -> Result<i32, ToolsError> {
    if args.len() > 1 || args.len() == 1 && args[0] != "--check" {
        return Err(ToolsError::invalid("Usage: qa-tools reference::q3::capture::run [--check]"));
    }
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    let mut command = vec![exe.to_string_lossy().into_owned(), "reference::q3::capture::run".to_owned()];
    command.extend(args.iter().cloned());
    let capture = capture_q3_reference(&command)?;
    if args.is_empty() {
        let directory = quake_typescript_root().join("verification/reference-cases/q3");
        std::fs::create_dir_all(&directory)
            .map_err(|error| ToolsError::io(format!("creating {}", directory.display()), error))?;
        fsutil::write_text(&directory.join("capture.json"), &format!("{}\n", capture.render_pretty()))?;
    }
    let summary = Json::object(vec![
        ("oracleKind".to_owned(), Json::string("source-derived")),
        ("status".to_owned(), capture.get("status").unwrap_or(&Json::Null).clone()),
        (
            "scenarios".to_owned(),
            Json::uint(capture.get("scenarios").and_then(Json::as_array).map_or(0, <[Json]>::len) as u64),
        ),
        ("assertions".to_owned(), capture.get("assertionCount").unwrap_or(&Json::Null).clone()),
    ]);
    println!("{}", summary.render());
    if capture.get("status").and_then(Json::as_str) != Some("PASS") {
        if let Some(scenarios) = capture.get("scenarios").and_then(Json::as_array) {
            for scenario in scenarios {
                let id = scenario.get("id").and_then(Json::as_str).unwrap_or("?");
                if let Some(assertions) = scenario.get("assertions").and_then(Json::as_array) {
                    for assertion in assertions {
                        if assertion.get("passed").and_then(Json::as_bool) != Some(true) {
                            eprintln!(
                                "{id}/{}: expected {}; actual {}",
                                assertion.get("id").and_then(Json::as_str).unwrap_or("?"),
                                assertion.get("expected").unwrap_or(&Json::Null).render(),
                                assertion.get("actual").unwrap_or(&Json::Null).render()
                            );
                        }
                    }
                }
            }
        }
        return Ok(1);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_passes_all_scenarios() {
        let capture = capture_q3_reference(&["qa-tools".to_owned(), "reference::q3::capture::run".to_owned()])
            .expect("capture");
        assert_eq!(capture.get("status").and_then(Json::as_str), Some("PASS"));
        assert_eq!(capture.get("assertionCount").and_then(Json::as_f64), Some(25.0));
        assert_eq!(capture.get("engineFamily").and_then(Json::as_str), Some("q3"));
        let scenarios = capture.get("scenarios").and_then(Json::as_array).expect("scenarios");
        assert_eq!(scenarios.len(), 8);
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(run(&["--bogus".to_owned()]).is_err());
        assert!(run(&["--check".to_owned(), "--check".to_owned()]).is_err());
    }

    #[test]
    fn check_mode_returns_zero() {
        assert_eq!(run(&["--check".to_owned()]).expect("check"), 0);
    }
}
