//! Verification command line (donor `tools/verify/cli.ts`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use crate::error::ToolsError;
use crate::fsutil::read_json;
use crate::json::Json;
use crate::verify::product::{parse_shard, Shard};
use crate::verify::runner::{run_manifest, verify_archived_report, CandidateExecutable, RunOptions};
use crate::verify::schema::{
    list, object, parse_manifest, parse_record, profile as parse_profile, sha256_hash, ExecutionRecord,
    VerificationProfile,
};

/// Parsed CLI options.
pub struct CliOptions {
    /// Selecting profile.
    pub profile: VerificationProfile,
    /// Manifest path.
    pub manifest: String,
    /// Output parent.
    pub output_parent: String,
    /// Shard selection.
    pub shard: Shard,
    /// Executable path, or `None`.
    pub executable: Option<String>,
    /// Resume report path, or `None`.
    pub resume: Option<String>,
    /// Changed-paths selection.
    pub changed: bool,
    /// Reconcile only.
    pub reconcile_only: bool,
}

/// Parse CLI arguments.
pub fn parse_args(args: &[String]) -> Result<CliOptions, ToolsError> {
    let mut profile = VerificationProfile::Dev;
    let mut manifest = "verification/suites.json".to_owned();
    let mut output_parent = ".artifacts/verification".to_owned();
    let mut shard = Shard { index: 0, count: 1 };
    let mut executable: Option<String> = None;
    let mut resume: Option<String> = None;
    let mut resume_latest = false;
    let mut changed = false;
    let mut reconcile_only = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--changed" {
            changed = true;
            index += 1;
            continue;
        }
        if argument == "--reconcile" {
            reconcile_only = true;
            index += 1;
            continue;
        }
        if argument == "--resume" && args.get(index + 1).is_none_or(|next| next.starts_with("--")) {
            resume_latest = true;
            index += 1;
            continue;
        }
        let value = args.get(index + 1);
        match value {
            None => return Err(ToolsError::invalid(format!("Missing value for {argument}"))),
            Some(next) if next.starts_with("--") => {
                return Err(ToolsError::invalid(format!("Missing value for {argument}")))
            }
            Some(next) => {
                if argument == "--profile" {
                    profile = parse_profile(&Json::string(next))?;
                } else if argument == "--manifest" {
                    manifest = next.clone();
                } else if argument == "--output-root" {
                    output_parent = next.clone();
                } else if argument == "--shard" {
                    shard = parse_shard(next)?;
                } else if argument == "--executable" {
                    executable = Some(next.clone());
                } else if argument == "--resume" {
                    resume = Some(next.clone());
                } else {
                    return Err(ToolsError::invalid(format!("Unknown verification argument {argument}")));
                }
            }
        }
        index += 2;
    }
    if changed && profile != VerificationProfile::Dev {
        return Err(ToolsError::invalid(
            "--changed is only valid for the explicitly partial dev profile",
        ));
    }
    if resume_latest {
        resume = latest_report(&output_parent)?;
        if resume.is_none() {
            eprintln!("No completed prior report exists; every selected case will receive a fresh attempt.");
        }
    }
    Ok(CliOptions {
        profile,
        manifest,
        output_parent,
        shard,
        executable,
        resume,
        changed,
        reconcile_only,
    })
}

fn latest_report(output_parent: &str) -> Result<Option<String>, ToolsError> {
    let mut candidates: Vec<(String, std::time::SystemTime)> = Vec::new();
    let entries = match std::fs::read_dir(output_parent) {
        Ok(entries) => entries,
        Err(_) => return Ok(None),
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) || !name.starts_with("run-") {
            continue;
        }
        let path = PathBuf::from(output_parent).join(&name).join("report.json");
        if let Ok(meta) = std::fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                candidates.push((path.to_string_lossy().into_owned(), modified));
            }
        }
    }
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    Ok(candidates.into_iter().next().map(|(path, _)| path))
}

fn changed_paths() -> Result<Vec<String>, ToolsError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .map_err(|error| ToolsError::io("determining changed files", error))?;
    if !output.status.success() {
        return Err(ToolsError::invalid(format!(
            "Cannot determine changed files: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let status = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<&str> = status.split('\0').collect();
    let mut paths = Vec::new();
    let mut index = 0;
    while index < entries.len() {
        let entry = entries[index];
        index += 1;
        if entry.is_empty() {
            continue;
        }
        paths.push(entry.get(3..).unwrap_or("").to_owned());
        let bytes = entry.as_bytes();
        if bytes.first() == Some(&b'R')
            || bytes.first() == Some(&b'C')
            || bytes.get(1) == Some(&b'R')
            || bytes.get(1) == Some(&b'C')
        {
            if let Some(previous) = entries.get(index) {
                paths.push((*previous).to_owned());
            }
            index += 1;
        }
    }
    Ok(paths)
}

/// Run the CLI, returning the exit code.
pub fn run(args: &[String]) -> Result<i32, ToolsError> {
    let options = parse_args(args)?;
    let manifest = parse_manifest(&read_json(PathBuf::from(&options.manifest).as_path())?)?;
    let mut previous_records: Vec<ExecutionRecord> = Vec::new();
    if let Some(resume) = &options.resume {
        let report = object(&read_json(PathBuf::from(resume).as_path())?, "previous report")?.to_vec();
        let records = report
            .iter()
            .find(|(key, _)| key == "records")
            .map(|(_, value)| value)
            .ok_or_else(|| ToolsError::parse("previous records must be an array"))?;
        for item in list(records, "previous records")? {
            previous_records.push(parse_record(item)?);
        }
    }
    if options.reconcile_only {
        if options.resume.is_none() {
            return Err(ToolsError::invalid("--reconcile requires --resume REPORT.json"));
        }
        let result = verify_archived_report(&manifest, &previous_records)?;
        println!("{}", result.to_json().render_pretty());
        return Ok(i32::from(!result.complete));
    }
    let mut executable: Option<CandidateExecutable> = None;
    if let Some(path) = &options.executable {
        let build = object(
            &read_json(PathBuf::from(format!("{path}.build.json")).as_path())?,
            "build provenance",
        )?
        .to_vec();
        let version = build
            .iter()
            .find(|(key, _)| key == "schemaVersion")
            .map(|(_, value)| value);
        if version.and_then(Json::as_f64) != Some(1.0) {
            return Err(ToolsError::invalid("Unsupported build provenance version"));
        }
        let source = build
            .iter()
            .find(|(key, _)| key == "sourceSha256")
            .map(|(_, value)| value)
            .ok_or_else(|| ToolsError::parse("build source hash must be a nonempty string"))?;
        let digest = build
            .iter()
            .find(|(key, _)| key == "executableSha256")
            .map(|(_, value)| value)
            .ok_or_else(|| ToolsError::parse("build executable hash must be a nonempty string"))?;
        executable = Some(CandidateExecutable {
            path: path.clone(),
            source_sha256: sha256_hash(source, "build source hash")?,
            sha256: sha256_hash(digest, "build executable hash")?,
        });
    }
    let changed = if options.changed { Some(changed_paths()?) } else { None };
    let report = run_manifest(
        &manifest,
        &RunOptions {
            workspace: std::env::current_dir().map_err(|error| ToolsError::io("resolving current directory", error))?,
            output_parent: PathBuf::from(&options.output_parent),
            profile: options.profile,
            shard: options.shard,
            changed_paths: changed,
            executable,
            environment: HashMap::new(),
            libraries: Vec::new(),
            previous_records,
        },
    )?;
    let summary = Json::object(vec![
        (
            "report".to_owned(),
            Json::string(format!("{}/report.json", report.output_root)),
        ),
        ("selected".to_owned(), Json::int(report.selected_case_ids.len() as i64)),
        ("selectedComplete".to_owned(), Json::boolean(report.selected_complete)),
        (
            "completeRequiredManifest".to_owned(),
            Json::boolean(report.reconciliation.complete),
        ),
        (
            "gameplayComplete".to_owned(),
            Json::boolean(report.reconciliation.gameplay_complete),
        ),
        ("counts".to_owned(), report.reconciliation.counts.to_json()),
    ]);
    println!("{}", summary.render());
    Ok(i32::from(!report.selected_complete))
}
