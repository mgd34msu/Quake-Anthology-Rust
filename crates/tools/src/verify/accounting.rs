//! Record accounting (donor `tools/verify/accounting.ts`).
//!
//! Assertion checks, record validation, manifest reconciliation, artifact
//! verification, and resume eligibility.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::ToolsError;
use crate::json::{parse_json, Json};
use crate::time::parse_iso;
use crate::verify::hash::{hash_bytes, hash_json};
use crate::verify::schema::{
    AssertionObservation, CaseManifest, EvidenceKind, ExecutionRecord, ExpectedCase, Fingerprints, InvalidRecord,
    Outcome, Reconciliation, StatusCounts, VerificationStatus,
};

/// Check a record's assertions against its expected contracts.
pub fn assertion_failures(expected: &ExpectedCase, assertions: &[AssertionObservation], assertion_count: i64) -> Result<Vec<String>, ToolsError> {
    let mut failures = Vec::new();
    if assertion_count != assertions.len() as i64 {
        failures.push("Assertion count does not match observations".to_owned());
    }
    if assertions.is_empty() {
        failures.push("Zero-assertion success is forbidden".to_owned());
    }
    let mut ids = HashSet::new();
    let contracts: HashSet<&str> = expected.contracts.iter().map(|contract| contract.id.as_str()).collect();
    for assertion in assertions {
        if !ids.insert(assertion.id.as_str()) {
            failures.push(format!("Duplicate assertion ID {}", assertion.id));
        }
        if !contracts.contains(assertion.contract_id.as_str()) {
            failures.push(format!("Unknown assertion contract {}", assertion.contract_id));
        }
        if !assertion.passed {
            failures.push(format!("Failed assertion {}", assertion.id));
        }
    }
    for contract in &expected.contracts {
        let count = assertions.iter().filter(|assertion| assertion.contract_id == contract.id).count() as i64;
        if count < contract.minimum_assertions {
            failures.push(format!(
                "Contract {} requires {} assertions, received {count}",
                contract.id, contract.minimum_assertions
            ));
        }
        if contract.oracle.sha256.is_none() {
            failures.push(format!("Contract {} has no pinned oracle", contract.id));
        }
    }
    Ok(failures)
}

/// Check a record against its expected case.
pub fn record_failures(expected: &ExpectedCase, record: &ExecutionRecord, manifest_hash: &str) -> Result<Vec<String>, ToolsError> {
    let mut failures = Vec::new();
    if record.case_id != expected.id || record.configuration_id != expected.configuration_id || record.suite_id != expected.suite_id {
        failures.push("Record identity does not match expected case".to_owned());
    }
    if record.evidence_kind != expected.evidence_kind {
        failures.push("Record evidence kind does not match expected case".to_owned());
    }
    if hash_json(&Json::array(record.contracts.iter().map(crate::verify::schema::ExpectedContract::to_json).collect()))?
        != hash_json(&Json::array(expected.contracts.iter().map(crate::verify::schema::ExpectedContract::to_json).collect()))?
    {
        failures.push("Recorded contracts do not match expected contracts".to_owned());
    }
    if hash_json(&Json::array(record.inputs.iter().map(crate::verify::schema::InputRequirement::to_json).collect()))?
        != hash_json(&Json::array(expected.requirements.iter().map(crate::verify::schema::InputRequirement::to_json).collect()))?
    {
        failures.push("Recorded inputs do not match required inputs".to_owned());
    }
    let record_command = record.command.as_ref().map_or(Json::Null, crate::verify::schema::CommandContract::to_json);
    let expected_command = expected.command.as_ref().map_or(Json::Null, crate::verify::schema::CommandContract::to_json);
    if hash_json(&record_command)? != hash_json(&expected_command)? {
        failures.push("Recorded command does not match expected command".to_owned());
    }
    if record.fingerprints.manifest != manifest_hash || hash_json(&expected.to_json())? != record.fingerprints.expected_case {
        failures.push("Stale manifest or case fingerprint".to_owned());
    }
    if hash_json(&record.environment.to_json())? != record.fingerprints.environment {
        failures.push("Environment fingerprint does not match recorded conditions".to_owned());
    }
    if record.fingerprints.seed != expected.seed
        || record.fingerprints.clock_schedule != expected.clock_schedule_sha256
        || record.fingerprints.network_schedule != expected.network_schedule_sha256
    {
        failures.push("Seed or schedule fingerprint does not match expected case".to_owned());
    }
    if parse_iso(&record.finished_at)? < parse_iso(&record.started_at)? {
        failures.push("Record finishes before it starts".to_owned());
    }
    if record.outcome.status() == VerificationStatus::Pass {
        failures.extend(assertion_failures(expected, &record.assertions, record.assertion_count)?);
        if expected.command.is_none() {
            failures.push("Unbound command cannot pass".to_owned());
        }
        if record.resolved_command.is_empty() {
            failures.push("PASS has no executed command".to_owned());
        }
        if record.fingerprints.source != record.fingerprints.snapshot {
            failures.push("Snapshot does not match source fingerprint".to_owned());
        }
        if expected.clock_schedule_sha256.is_none() || expected.network_schedule_sha256.is_none() {
            failures.push("PASS has an unpinned clock or network schedule".to_owned());
        }
        if record.artifacts.is_empty() {
            failures.push("PASS has no raw evidence artifacts".to_owned());
        }
        if expected.requirements.iter().any(|requirement| requirement.sha256.is_none()) {
            failures.push("PASS contains unpinned required inputs".to_owned());
        }
    }
    if let Outcome::Fail { reasons, .. } = &record.outcome {
        if reasons.is_empty() {
            failures.push("FAIL must explain its failure".to_owned());
        }
    }
    if let Outcome::BlockedMissingInput { missing_inputs } = &record.outcome {
        if missing_inputs.is_empty() {
            failures.push("BLOCKED_MISSING_INPUT must identify its inputs".to_owned());
        }
    }
    Ok(failures)
}

/// Reconcile a manifest with its records.
pub fn reconcile(manifest: &CaseManifest, records: &[ExecutionRecord]) -> Result<Reconciliation, ToolsError> {
    let manifest_sha256 = hash_json(&manifest.to_json())?;
    let expected: HashMap<&str, &ExpectedCase> = manifest.cases.iter().map(|item| (item.id.as_str(), item)).collect();
    let mut actual: HashMap<&str, &ExecutionRecord> = HashMap::new();
    let mut duplicate_case_ids = HashSet::new();
    let mut unexpected_case_ids = HashSet::new();
    let mut invalid_records = Vec::new();
    let mut counts = StatusCounts::default();
    for record in records {
        if actual.contains_key(record.case_id.as_str()) {
            duplicate_case_ids.insert(record.case_id.clone());
        }
        actual.insert(record.case_id.as_str(), record);
        match expected.get(record.case_id.as_str()) {
            None => {
                unexpected_case_ids.insert(record.case_id.clone());
            }
            Some(item) => {
                let reasons = record_failures(item, record, &manifest_sha256)?;
                if !reasons.is_empty() {
                    invalid_records.push(InvalidRecord {
                        case_id: record.case_id.clone(),
                        reasons,
                    });
                }
            }
        }
        counts.add(record.outcome.status());
    }
    let missing_case_ids: Vec<String> = manifest.cases.iter().filter(|item| !actual.contains_key(item.id.as_str())).map(|item| item.id.clone()).collect();
    let complete = missing_case_ids.is_empty()
        && unexpected_case_ids.is_empty()
        && duplicate_case_ids.is_empty()
        && invalid_records.is_empty()
        && counts.pass == manifest.cases.len() as i64;
    let mut unexpected: Vec<String> = unexpected_case_ids.into_iter().collect();
    unexpected.sort();
    let mut duplicates: Vec<String> = duplicate_case_ids.into_iter().collect();
    duplicates.sort();
    Ok(Reconciliation {
        manifest_id: manifest.id.clone(),
        manifest_sha256,
        expected: manifest.cases.len() as i64,
        records: records.len() as i64,
        counts,
        missing_case_ids,
        unexpected_case_ids: unexpected,
        duplicate_case_ids: duplicates,
        invalid_records,
        complete,
        gameplay_complete: complete
            && manifest.cases.iter().any(|item| item.evidence_kind == EvidenceKind::Gameplay || item.evidence_kind == EvidenceKind::Release),
    })
}

/// Verify a record's artifacts against owned output bytes.
pub fn artifact_failures(record: &ExecutionRecord) -> Result<Vec<String>, ToolsError> {
    use crate::verify::schema::parse_driver_output;
    use crate::verify::snapshot::is_within;

    let mut failures = Vec::new();
    let mut paths = HashSet::new();
    let mut found_driver = false;
    for artifact in &record.artifacts {
        if !paths.insert(artifact.path.as_str()) {
            failures.push(format!("Duplicate artifact {}", artifact.path));
        }
        let check = (|| -> Result<(), ToolsError> {
            let root = std::fs::canonicalize(&record.output_root)
                .map_err(|error| ToolsError::io(format!("resolving {}", record.output_root), error))?;
            let relative = Path::new(&artifact.path);
            let path = root.join(relative);
            let real = std::fs::canonicalize(&path).map_err(|error| ToolsError::io(format!("resolving {}", path.display()), error))?;
            if relative.is_absolute() || !is_within(&root, &path) || path == root || !is_within(&root, &real) {
                return Err(ToolsError::invalid("Artifact escapes owned output root"));
            }
            let stat = std::fs::symlink_metadata(&path).map_err(|error| ToolsError::io(format!("stating {}", path.display()), error))?;
            if !stat.is_file() || stat.is_symlink() {
                return Err(ToolsError::invalid("Artifact is not a regular owned file"));
            }
            let bytes = std::fs::read(&path).map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))?;
            if bytes.len() as i64 != artifact.bytes || hash_bytes(&bytes) != artifact.sha256 {
                failures.push(format!("Artifact changed: {}", artifact.path));
            }
            if artifact.path == "driver-result.json" {
                found_driver = true;
                let text = String::from_utf8(bytes).map_err(|_| ToolsError::invalid("driver-result.json is not UTF-8"))?;
                let output = parse_driver_output(&parse_json(&text)?)?;
                let output_assertions = Json::array(output.assertions.iter().map(AssertionObservation::to_json).collect());
                let record_assertions = Json::array(record.assertions.iter().map(AssertionObservation::to_json).collect());
                let output_checkpoints =
                    Json::array(output.checkpoints.iter().map(crate::verify::schema::Checkpoint::to_json).collect());
                let record_checkpoints =
                    Json::array(record.checkpoints.iter().map(crate::verify::schema::Checkpoint::to_json).collect());
                if output.case_id != record.case_id || hash_json(&output_assertions)? != hash_json(&record_assertions)? || hash_json(&output_checkpoints)? != hash_json(&record_checkpoints)? {
                    failures.push("Raw driver output does not match recorded assertions/checkpoints".to_owned());
                }
                for declared in &output.artifact_paths {
                    if !record.artifacts.iter().any(|item| &item.path == declared) {
                        failures.push(format!("Unrecorded driver artifact {declared}"));
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = check {
            failures.push(format!("Artifact {}: {error}", artifact.path));
        }
    }
    if record.outcome.status() == VerificationStatus::Pass && !found_driver {
        failures.push("PASS has no raw driver-result.json".to_owned());
    }
    Ok(failures)
}

/// Resume eligibility for a previous record.
pub struct ResumeDecision {
    /// Whether the previous record is reusable.
    pub reusable: bool,
    /// Reasons when it is not.
    pub reasons: Vec<String>,
}

/// Decide whether a previous passing record can be reused.
pub fn can_resume(expected: &ExpectedCase, current: &Fingerprints, previous: &ExecutionRecord) -> Result<ResumeDecision, ToolsError> {
    let mut reasons = record_failures(expected, previous, &current.manifest)?;
    if previous.outcome.status() != VerificationStatus::Pass {
        reasons.push(format!("Previous attempt status is {}", previous.outcome.status().as_str()));
    }
    if hash_json(&current.to_json())? != hash_json(&previous.fingerprints.to_json())? {
        reasons.push("Source, fixture, environment, executable, or schedule fingerprint changed".to_owned());
    }
    reasons.extend(artifact_failures(previous)?);
    Ok(ResumeDecision {
        reusable: reasons.is_empty(),
        reasons,
    })
}
