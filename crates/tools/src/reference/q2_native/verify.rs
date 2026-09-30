//! Native capture verification (donor `tools/reference/q2-native/verify.ts`).
//!
//! Re-validates retained native captures: every artifact identity recorded
//! under the artifacts directory is re-hashed and compared, `provenance.json`
//! identities recurse, and the four q2repro cases plus the Steam classic
//! document must show their live checks as observed. This verifies retained
//! bytes and recorded checks; it never reruns native processes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{parse_json, Json};
use crate::reference::environment::{identify_file, quake_typescript_root};

/// Verification summary.
#[derive(Debug, Clone)]
pub struct VerifySummary {
    /// Distinct artifact identities re-hashed.
    pub checked_artifact_identities: usize,
    /// Observed case ids.
    pub observed: Vec<String>,
    /// Unsupported entries with reasons.
    pub unsupported: Vec<String>,
}

impl VerifySummary {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("checkedArtifactIdentities".to_owned(), Json::uint(self.checked_artifact_identities as u64)),
            ("observed".to_owned(), Json::array(self.observed.iter().map(Json::string).collect())),
            ("unsupported".to_owned(), Json::array(self.unsupported.iter().map(Json::string).collect())),
            (
                "scope".to_owned(),
                Json::string(
                    "Verifies retained artifact bytes and recorded live checks; does not establish engine equivalence or rerun native processes.",
                ),
            ),
        ])
    }
}

fn is_safe_size(value: &Json) -> Option<u64> {
    let size = value.as_f64()?;
    if size.fract() != 0.0 || !(0.0..=9_007_199_254_740_991.0).contains(&size) {
        return None;
    }
    Some(size as u64)
}

fn is_sha256(value: &Json) -> bool {
    value.as_str().is_some_and(|sha| sha.len() == 64 && sha.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
}

/// Collect artifact identities under `artifacts_prefix`, recursing into `provenance.json`.
fn visit(value: &Json, identities: &mut HashMap<String, (u64, String)>, artifacts_prefix: &str) -> Result<(), ToolsError> {
    if let Some(items) = value.as_array() {
        for item in items {
            visit(item, identities, artifacts_prefix)?;
        }
        return Ok(());
    }
    let Some(entries) = value.as_object() else {
        return Ok(());
    };
    if let (Some(path), Some(size), Some(sha256)) = (
        entries.iter().find(|(key, _)| key == "path").map(|(_, found)| found),
        entries.iter().find(|(key, _)| key == "size").map(|(_, found)| found),
        entries.iter().find(|(key, _)| key == "sha256").map(|(_, found)| found),
    ) {
        if let Some(path) = path.as_str() {
            if path.starts_with(artifacts_prefix) {
                let (Some(size), true) = (is_safe_size(size), is_sha256(sha256)) else {
                    return Err(ToolsError::invalid(format!("Invalid identity: {path}")));
                };
                let sha = sha256.as_str().expect("checked sha").to_owned();
                if let Some((_, prior)) = identities.get(path) {
                    if *prior != sha {
                        return Err(ToolsError::invalid(format!("Conflicting captured identities: {path}")));
                    }
                }
                identities.insert(path.to_owned(), (size, sha));
                if path.ends_with("provenance.json") {
                    let provenance = parse_json(&fsutil::read_text(Path::new(path))?)?;
                    visit(&provenance, identities, artifacts_prefix)?;
                }
            }
        }
    }
    for (_, entry) in entries {
        visit(entry, identities, artifacts_prefix)?;
    }
    Ok(())
}

fn failure_text(value: &Json) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| value.render())
}

/// Verify capture documents, re-hashing identities under `artifacts_dir`.
pub fn verify_documents(paths: &[PathBuf], artifacts_dir: &Path) -> Result<VerifySummary, ToolsError> {
    let prefix = format!("{}/", artifacts_dir.to_string_lossy());
    let mut identities: HashMap<String, (u64, String)> = HashMap::new();
    let mut observed = Vec::new();
    let mut unsupported = Vec::new();
    for path in paths {
        let text = path.to_string_lossy().into_owned();
        let value = parse_json(&fsutil::read_text(path)?)?;
        let Some(entries) = value.as_object() else {
            return Err(ToolsError::invalid(format!("Invalid capture: {text}")));
        };
        let version = entries.iter().find(|(key, _)| key == "schemaVersion").map(|(_, found)| found);
        if version.and_then(Json::as_f64) != Some(1.0) {
            return Err(ToolsError::invalid(format!("Invalid capture: {text}")));
        }
        let cases = entries.iter().find(|(key, _)| key == "cases").map(|(_, found)| found);
        if let Some(cases) = cases.and_then(Json::as_array) {
            if cases.len() != 4 {
                return Err(ToolsError::invalid("Expected four q2repro native cases"));
            }
            for item in cases {
                let id = item.get("id").and_then(Json::as_str);
                if item.as_object().is_none() || id.is_none() || item.get("observed").and_then(Json::as_bool) != Some(true) {
                    return Err(ToolsError::invalid("Native case did not pass its live checks"));
                }
                observed.push(id.expect("checked id").to_owned());
            }
        } else if value.get("observed").and_then(Json::as_bool) == Some(true) {
            observed.push("steam-classic-dedicated".to_owned());
        } else {
            let failure = value.get("failure").unwrap_or(&Json::Null);
            unsupported.push(format!("steam-classic-dedicated: {}", failure_text(failure)));
        }
        visit(&value, &mut identities, &prefix)?;
    }
    for (path, (size, sha256)) in &identities {
        let actual = identify_file(path)?;
        if actual.size != *size || actual.sha256 != *sha256 {
            return Err(ToolsError::invalid(format!("Captured artifact changed: {path}")));
        }
    }
    Ok(VerifySummary { checked_artifact_identities: identities.len(), observed, unsupported })
}

/// Verify the retained native captures and print the summary (donor `verifyCaptures`).
pub fn verify_captures() -> Result<VerifySummary, ToolsError> {
    let project = quake_typescript_root();
    let summary = verify_documents(
        &[
            project.join("verification/reference-cases/q2-native/latest.json"),
            project.join("verification/reference-cases/q2-native/steam-classic.json"),
        ],
        &project.join(".artifacts"),
    )?;
    println!("{}", summary.to_json().render());
    Ok(summary)
}

/// Run verification (donor `main` ignores arguments).
pub fn run() -> Result<(), ToolsError> {
    verify_captures()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str) -> (PathBuf, PathBuf) {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-verify-").expect("temp dir");
        let artifacts = directory.join(".artifacts");
        std::fs::create_dir_all(&artifacts).expect("artifacts dir");
        fsutil::write_text(&artifacts.join("output.bin"), text).expect("write");
        (directory, artifacts)
    }

    fn latest_json(identity: &Json, observed: bool) -> String {
        let cases: Vec<String> = (0..4)
            .map(|index| format!(r#"{{"id": "case-{index}", "observed": {observed}, "output": {identity}}}"#, identity = identity.render()))
            .collect();
        format!(r#"{{"schemaVersion": 1, "cases": [{}]}}"#, cases.join(", "))
    }

    #[test]
    fn verifies_retained_documents() {
        let (directory, artifacts) = fixture("retained-bytes");
        let retained = artifacts.join("output.bin");
        let identity = identify_file(&retained.to_string_lossy()).expect("identify").to_json();
        let latest = directory.join("latest.json");
        let steam = directory.join("steam-classic.json");
        fsutil::write_text(&latest, &latest_json(&identity, true)).expect("write latest");
        fsutil::write_text(
            &steam,
            &format!(r#"{{"schemaVersion": 1, "observed": true, "output": {}}}"#, identity.render()),
        )
        .expect("write steam");
        let summary = verify_documents(&[latest, steam], &artifacts).expect("verify");
        assert_eq!(summary.checked_artifact_identities, 1);
        assert_eq!(summary.observed.len(), 5);
        assert!(summary.unsupported.is_empty());
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn detects_changed_artifacts_and_bad_documents() {
        let (directory, artifacts) = fixture("original");
        let retained = artifacts.join("output.bin");
        let identity = identify_file(&retained.to_string_lossy()).expect("identify").to_json();
        let latest = directory.join("latest.json");
        let steam = directory.join("steam-classic.json");
        fsutil::write_text(&latest, &latest_json(&identity, true)).expect("write latest");
        fsutil::write_text(&steam, r#"{"schemaVersion": 1, "observed": true}"#).expect("write steam");
        fsutil::write_text(&retained, "tampered!").expect("tamper");
        let error = verify_documents(&[latest.clone(), steam.clone()], &artifacts).expect_err("tamper");
        assert!(error.to_string().starts_with("Captured artifact changed"), "{error}");
        fsutil::write_text(&retained, "original").expect("restore");
        fsutil::write_text(&latest, &latest_json(&identity, false)).expect("unobserved");
        let error = verify_documents(&[latest.clone(), steam.clone()], &artifacts).expect_err("unobserved");
        assert_eq!(error.to_string(), "Native case did not pass its live checks");
        fsutil::write_text(&latest, r#"{"schemaVersion": 1, "cases": []}"#).expect("short");
        let error = verify_documents(&[latest.clone(), steam.clone()], &artifacts).expect_err("short");
        assert_eq!(error.to_string(), "Expected four q2repro native cases");
        fsutil::write_text(&latest, r#"{"schemaVersion": 2}"#).expect("version");
        let error = verify_documents(&[latest, steam], &artifacts).expect_err("version");
        assert!(error.to_string().starts_with("Invalid capture"), "{error}");
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn records_unsupported_steam_documents() {
        let (directory, artifacts) = fixture("x");
        let latest = directory.join("latest.json");
        let steam = directory.join("steam-classic.json");
        let retained = artifacts.join("output.bin");
        let identity = identify_file(&retained.to_string_lossy()).expect("identify").to_json();
        fsutil::write_text(&latest, &latest_json(&identity, true)).expect("write latest");
        fsutil::write_text(&steam, r#"{"schemaVersion": 1, "observed": false, "failure": "no wine"}"#).expect("write steam");
        let summary = verify_documents(&[latest, steam], &artifacts).expect("verify");
        assert_eq!(summary.unsupported, vec!["steam-classic-dedicated: no wine".to_owned()]);
        fsutil::remove_forced(&directory);
    }
}
