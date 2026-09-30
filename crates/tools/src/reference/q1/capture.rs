//! Q1 source-derived capture (donor `tools/reference/q1/capture.ts`).
//!
//! Verifies pinned source excerpts, evaluates every case through the oracle,
//! and assembles the capture document; stored captures re-verify against
//! recomputed expectations. Port notes: capture-program identities name the
//! Rust sources plus the workspace manifest, lockfile, and crate manifest
//! (standing in for `package.json`, `bun.lock`, and `tsconfig.json`); the
//! command names the library entry point because the port keeps the existing
//! binary surface; the runtime version records the executing tool.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{deep_strict_equal, Json};
use crate::reference::environment::{identify_file, quake_typescript_root, source_root};
use crate::reference::q1::cases::{q1_cases, Q1Case};
use crate::reference::q1::oracle::{run_q1_oracle, Q1Output};
use crate::reference::q1::sources::{q1_oracle_limits, q1_source_pins, SourcePin};
use crate::reference::{node_arch, node_platform};
use crate::time::now_iso;
use crate::verify::hash::{hash_json, hash_str};

/// Verify one pinned source: identity, stability, and excerpt bounds.
pub fn verify_source_pin(source_root: &str, pin: &SourcePin) -> Result<Json, ToolsError> {
    let path = fsutil::lexical_absolute(Path::new(source_root), &pin.path);
    let path_text = path.to_string_lossy().into_owned();
    let identity = identify_file(&path_text)?;
    if identity.sha256 != pin.sha256 {
        return Err(ToolsError::invalid(format!(
            "Source hash mismatch for {}: expected {}, observed {}",
            pin.id, pin.sha256, identity.sha256
        )));
    }
    let bytes = fsutil::read_bytes(&path)?;
    if crate::verify::hash::hash_bytes(&bytes) != pin.sha256 {
        return Err(ToolsError::invalid(format!(
            "Source changed during capture: {}",
            pin.id
        )));
    }
    let decoded = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = decoded.split('\n').collect();
    let mut excerpts = Vec::with_capacity(pin.excerpts.len());
    for excerpt in &pin.excerpts {
        if excerpt.first_line < 1
            || excerpt.last_line < excerpt.first_line
            || u64::from(excerpt.last_line) > lines.len() as u64
        {
            return Err(ToolsError::invalid(format!(
                "Invalid source excerpt bounds: {}",
                pin.id
            )));
        }
        let code = lines[excerpt.first_line as usize - 1..excerpt.last_line as usize].join("\n");
        excerpts.push(Json::object(vec![
            ("firstLine".to_owned(), Json::uint(u64::from(excerpt.first_line))),
            ("lastLine".to_owned(), Json::uint(u64::from(excerpt.last_line))),
            ("purpose".to_owned(), Json::string(&excerpt.purpose)),
            ("code".to_owned(), Json::string(&code)),
            ("sha256".to_owned(), Json::string(hash_str(&code))),
        ]));
    }
    Ok(Json::object(vec![
        ("id".to_owned(), Json::string(&pin.id)),
        ("relativePath".to_owned(), Json::string(&pin.path)),
        ("identity".to_owned(), identity.to_json()),
        ("excerpts".to_owned(), Json::array(excerpts)),
    ]))
}

/// Render one evaluated case with fingerprints (donor spread order).
fn render_case(case: &Q1Case, observed: &Q1Output) -> Result<Json, ToolsError> {
    let Json::Object(mut pairs) = case.to_json() else {
        return Err(ToolsError::parse(format!(
            "Q1 case did not render as an object: {}",
            case.id
        )));
    };
    pairs.push(("observed".to_owned(), observed.to_json()));
    pairs.push(("passed".to_owned(), Json::boolean(true)));
    pairs.push((
        "inputSha256".to_owned(),
        Json::string(hash_json(&case.input.to_json())?),
    ));
    pairs.push((
        "expectedSha256".to_owned(),
        Json::string(hash_json(&case.expected.to_json())?),
    ));
    Ok(Json::object(pairs))
}

/// Evaluate one case and render it, failing on oracle disagreement.
fn evaluate_case(case: &Q1Case) -> Result<Json, ToolsError> {
    let observed = run_q1_oracle(&case.input.to_json())?;
    if !deep_strict_equal(&observed.to_json(), &case.expected.to_json()) {
        return Err(ToolsError::invalid(format!(
            "Source-derived Q1 oracle disagrees with hand-derived case {}",
            case.id
        )));
    }
    render_case(case, &observed)
}

/// Capture-program identities: Rust sources plus build manifests.
fn capture_program_paths() -> Vec<String> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = fsutil::lexical_absolute(manifest, "../..");
    [
        manifest.join("src/reference/q1/capture.rs"),
        manifest.join("src/reference/q1/oracle.rs"),
        manifest.join("src/reference/q1/cases.rs"),
        manifest.join("src/reference/q1/sources.rs"),
        manifest.join("src/reference/environment.rs"),
        manifest.join("src/reference/schema.rs"),
        manifest.join("src/verify/hash.rs"),
        root.join("Cargo.toml"),
        root.join("Cargo.lock"),
        manifest.join("Cargo.toml"),
    ]
    .into_iter()
    .map(|path| path.to_string_lossy().into_owned())
    .collect()
}

fn environment_text(name: &str) -> Json {
    std::env::var(name).map_or(Json::Null, Json::string)
}

/// Capture the Q1 source-derived reference under `source_root`.
pub fn capture_q1(source_root: &str) -> Result<Json, ToolsError> {
    let pins = q1_source_pins();
    let mut sources = Vec::new();
    for pin in &pins {
        sources.push(verify_source_pin(source_root, pin)?);
    }
    let source_ids: BTreeSet<&str> = pins.iter().map(|pin| pin.id.as_str()).collect();
    let mut case_ids = BTreeSet::new();
    let mut cases = Vec::new();
    for case in q1_cases() {
        if !case_ids.insert(case.id) {
            return Err(ToolsError::invalid(format!("Duplicate Q1 case {}", case.id)));
        }
        for id in case.sources {
            if !source_ids.contains(id) {
                return Err(ToolsError::invalid(format!("Unknown source {id} for {}", case.id)));
            }
        }
        cases.push(evaluate_case(&case)?);
    }
    let mut capture_program = Vec::new();
    for path in capture_program_paths() {
        capture_program.push(identify_file(&path)?.to_json());
    }
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    Ok(Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("game".to_owned(), Json::string("q1")),
        (
            "oracle".to_owned(),
            Json::object(vec![
                ("kind".to_owned(), Json::string("source-derived")),
                ("measuredOriginalExecution".to_owned(), Json::boolean(false)),
                (
                    "method".to_owned(),
                    Json::string(
                        "Independently transcribed original-source equations evaluated in TypeScript, compared with fixed hand-derived expected steps.",
                    ),
                ),
            ]),
        ),
        ("capturedAt".to_owned(), Json::string(now_iso())),
        (
            "command".to_owned(),
            Json::array(vec![
                Json::string(exe.to_string_lossy()),
                Json::string("reference::q1::capture::run"),
                Json::string(source_root),
            ]),
        ),
        ("runtime".to_owned(), identify_file(&exe.to_string_lossy())?.to_json()),
        ("toolVersion".to_owned(), Json::string(format!("qa-tools/{}", env!("CARGO_PKG_VERSION")))),
        ("platform".to_owned(), Json::string(node_platform())),
        ("architecture".to_owned(), Json::string(node_arch())),
        (
            "environment".to_owned(),
            Json::object(vec![
                ("LANG".to_owned(), environment_text("LANG")),
                ("LC_ALL".to_owned(), environment_text("LC_ALL")),
                ("TZ".to_owned(), environment_text("TZ")),
            ]),
        ),
        (
            "arithmetic".to_owned(),
            Json::object(vec![
                ("floatStorage".to_owned(), Json::string("IEEE-754 binary32 round-to-nearest ties-to-even")),
                ("hostClock".to_owned(), Json::string("IEEE-754 binary64")),
                (
                    "integerConversion".to_owned(),
                    Json::string("signed int32 truncation within representable range"),
                ),
                ("randomSeed".to_owned(), Json::Null),
                ("clockSchedule".to_owned(), Json::string("explicit per-case inputs")),
            ]),
        ),
        ("captureProgram".to_owned(), Json::array(capture_program)),
        ("sources".to_owned(), Json::array(sources)),
        ("cases".to_owned(), Json::array(cases.clone())),
        ("caseCount".to_owned(), Json::uint(cases.len() as u64)),
        ("limits".to_owned(), Json::array(q1_oracle_limits().iter().map(Json::string).collect())),
    ]))
}

/// Re-verify a stored Q1 capture against recomputed expectations.
pub fn verify_captured_cases(value: &Json) -> Result<(), ToolsError> {
    if value.as_object().is_none()
        || value.get("schemaVersion").and_then(Json::as_f64) != Some(1.0)
        || value.get("game").and_then(Json::as_str) != Some("q1")
    {
        return Err(ToolsError::invalid("Invalid Q1 capture header"));
    }
    let oracle = value.get("oracle").unwrap_or(&Json::Null);
    if oracle.as_object().is_none()
        || oracle.get("kind").and_then(Json::as_str) != Some("source-derived")
        || oracle.get("measuredOriginalExecution").and_then(Json::as_bool) != Some(false)
    {
        return Err(ToolsError::invalid(
            "Q1 capture must identify source-derived, unmeasured evidence",
        ));
    }
    let mut expected = Vec::new();
    for case in q1_cases() {
        expected.push(evaluate_case(&case)?);
    }
    if value.get("caseCount").and_then(Json::as_f64) != Some(expected.len() as f64)
        || value
            .get("cases")
            .is_none_or(|cases| !deep_strict_equal(cases, &Json::array(expected)))
    {
        return Err(ToolsError::invalid(
            "Stored Q1 cases differ from the pinned source-derived expectations",
        ));
    }
    Ok(())
}

/// Run the Q1 capture (donor `main`): capture or `--check` against the stored record.
pub fn run(args: &[String]) -> Result<(), ToolsError> {
    let check = args.first().is_some_and(|arg| arg == "--check");
    let positional: Vec<&String> = if check {
        args.iter().skip(1).collect()
    } else {
        args.iter().collect()
    };
    if positional.len() > 1 || positional.iter().any(|arg| arg.starts_with("--")) {
        return Err(ToolsError::invalid(
            "Usage: qa-tools reference::q1::capture::run [--check] [qsrc-root]",
        ));
    }
    let source_root = match positional.first() {
        Some(root) => {
            let cwd = std::env::current_dir().map_err(|error| ToolsError::io("resolving current directory", error))?;
            fsutil::lexical_absolute(&cwd, root.as_str())
                .to_string_lossy()
                .into_owned()
        }
        None => source_root(),
    };
    let record = capture_q1(&source_root)?;
    let output = quake_typescript_root().join("verification/reference-cases/q1/source-derived.json");
    if check {
        verify_captured_cases(&crate::json::parse_json(&fsutil::read_text(&output)?)?)?;
    } else {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
        }
        fsutil::write_text(&output, &format!("{}\n", record.render_pretty()))?;
    }
    let summary = Json::object(vec![
        ("path".to_owned(), Json::string(output.to_string_lossy())),
        (
            "cases".to_owned(),
            record.get("caseCount").unwrap_or(&Json::Null).clone(),
        ),
        ("oracle".to_owned(), Json::string("source-derived")),
        ("mode".to_owned(), Json::string(if check { "check" } else { "capture" })),
    ]);
    println!("{}", summary.render());
    Ok(())
}

#[cfg(test)]
mod tests {
    // Covers the capture-backed cases of tools/reference/q1/oracle.test.ts (source
    // identities, engine-independence, stored captures); oracle cases live in q1/oracle.rs tests.
    use crate::reference::q1::sources::SourceExcerpt;
    use crate::verify::hash::hash_str;

    use super::*;

    #[test]
    fn verifies_source_pins_and_rejects_bad_evidence() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-q1-source-").expect("temp dir");
        let text = directory.to_string_lossy().into_owned();
        fsutil::write_text(&directory.join("evidence.txt"), "one\ntwo\nthree\n").expect("write");
        let pin = SourcePin {
            id: "test-source".to_owned(),
            path: "evidence.txt".to_owned(),
            sha256: hash_str("one\ntwo\nthree\n"),
            excerpts: vec![SourceExcerpt {
                first_line: 2,
                last_line: 3,
                purpose: "line slicing".to_owned(),
            }],
        };
        let result = verify_source_pin(&text, &pin).expect("verify");
        let excerpts = result.get("excerpts").and_then(Json::as_array).expect("excerpts");
        assert_eq!(excerpts.len(), 1);
        assert_eq!(excerpts[0].get("code").and_then(Json::as_str), Some("two\nthree"));
        assert_eq!(
            excerpts[0].get("sha256").and_then(Json::as_str),
            Some(hash_str("two\nthree").as_str())
        );
        let bad_bounds = SourcePin {
            excerpts: vec![SourceExcerpt {
                first_line: 0,
                last_line: 1,
                purpose: "invalid".to_owned(),
            }],
            ..pin.clone()
        };
        let error = verify_source_pin(&text, &bad_bounds).expect_err("bounds");
        assert!(error.to_string().contains("bounds"), "{error}");
        fsutil::write_text(&directory.join("evidence.txt"), "changed").expect("change");
        let error = verify_source_pin(&text, &pin).expect_err("changed");
        assert!(error.to_string().contains("hash mismatch"), "{error}");
        std::fs::remove_file(directory.join("evidence.txt")).expect("remove");
        assert!(verify_source_pin(&text, &pin).is_err());
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn oracle_has_no_engine_dependency() {
        let source = fsutil::read_text(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference/q1/oracle.rs"))
            .expect("oracle source");
        for forbidden in [
            "qa_client",
            "qa_core",
            "qa_app",
            "qa_platform",
            "qa_net",
            "qa_bots",
            "qa_render",
        ] {
            assert!(!source.contains(forbidden), "oracle references {forbidden}");
        }
    }

    #[test]
    fn stored_captures_reject_relabeling_and_changed_cases() {
        let mut rendered = Vec::new();
        for case in q1_cases() {
            rendered.push(evaluate_case(&case).expect("evaluate"));
        }
        let record = Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("game".to_owned(), Json::string("q1")),
            (
                "oracle".to_owned(),
                Json::object(vec![
                    ("kind".to_owned(), Json::string("source-derived")),
                    ("measuredOriginalExecution".to_owned(), Json::boolean(false)),
                ]),
            ),
            ("caseCount".to_owned(), Json::uint(rendered.len() as u64)),
            ("cases".to_owned(), Json::array(rendered.clone())),
        ]);
        verify_captured_cases(&record).expect("valid record verifies");
        let relabeled = Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("game".to_owned(), Json::string("q1")),
            (
                "oracle".to_owned(),
                Json::object(vec![
                    ("kind".to_owned(), Json::string("retail-trace")),
                    ("measuredOriginalExecution".to_owned(), Json::boolean(true)),
                ]),
            ),
            ("caseCount".to_owned(), Json::uint(rendered.len() as u64)),
            ("cases".to_owned(), Json::array(rendered.clone())),
        ]);
        assert!(verify_captured_cases(&relabeled).is_err());
        let sliced = Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("game".to_owned(), Json::string("q1")),
            (
                "oracle".to_owned(),
                Json::object(vec![
                    ("kind".to_owned(), Json::string("source-derived")),
                    ("measuredOriginalExecution".to_owned(), Json::boolean(false)),
                ]),
            ),
            ("caseCount".to_owned(), Json::uint(rendered.len() as u64)),
            ("cases".to_owned(), Json::array(rendered[1..].to_vec())),
        ]);
        assert!(verify_captured_cases(&sliced).is_err());
        assert!(verify_captured_cases(&Json::Null).is_err());
    }

    #[test]
    fn capture_passes_all_cases() {
        let capture = capture_q1(&source_root()).expect("capture");
        assert_eq!(capture.get("game").and_then(Json::as_str), Some("q1"));
        let cases = capture.get("cases").and_then(Json::as_array).expect("cases");
        assert!(!cases.is_empty());
        assert_eq!(
            capture.get("caseCount").and_then(Json::as_f64),
            Some(cases.len() as f64)
        );
        for case in cases {
            assert_eq!(case.get("passed").and_then(Json::as_bool), Some(true));
        }
    }

    #[test]
    fn rejects_bad_arguments() {
        assert!(run(&["--bogus".to_owned()]).is_err());
        assert!(run(&["a".to_owned(), "b".to_owned()]).is_err());
        assert!(run(&["--check".to_owned(), "--check".to_owned()]).is_err());
    }

    #[test]
    fn check_mode_verifies_the_stored_record() {
        run(&["--check".to_owned()]).expect("stored record verifies");
    }
}
