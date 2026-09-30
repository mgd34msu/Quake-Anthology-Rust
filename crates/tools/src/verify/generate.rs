//! Composition manifest generation (donor `tools/verify/generate.ts`).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil::{make_temp_dir, read_text};
use crate::json::{escape_string, parse_json};
use crate::verify::product::{composition_size, generate_composition, CaseCount};
use crate::verify::schema::{parse_domain, CompositionDomain};

/// Default generation limit.
pub const DEFAULT_MAX_CASES: &str = "100000";

/// Generation CLI options.
pub struct GenerateOptions {
    /// Domain document path.
    pub domain: PathBuf,
    /// Output manifest path, or `None` with `--count`.
    pub output: Option<PathBuf>,
    /// Maximum cases to emit.
    pub maximum: CaseCount,
    /// Count only.
    pub count_only: bool,
}

/// Parse generation arguments.
pub fn parse_args(args: &[String]) -> Result<GenerateOptions, ToolsError> {
    let mut domain: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut maximum = CaseCount::parse_decimal(DEFAULT_MAX_CASES)?;
    let mut count_only = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--count" {
            count_only = true;
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| ToolsError::invalid(format!("Missing value for {argument}")))?;
        if argument == "--domain" {
            domain = Some(PathBuf::from(value));
        } else if argument == "--output" {
            output = Some(PathBuf::from(value));
        } else if argument == "--max-cases" && !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
            maximum = CaseCount::parse_decimal(value)?;
        } else {
            return Err(ToolsError::invalid(format!("Unknown generation argument {argument}")));
        }
        index += 2;
    }
    match (domain, count_only, output) {
        (Some(domain), _, output) if count_only || output.is_some() => Ok(GenerateOptions {
            domain,
            output,
            maximum,
            count_only,
        }),
        _ => Err(ToolsError::invalid(
            "Usage: qa-tools-verify-generate --domain domain.json [--count | --output manifest.json] [--max-cases N]",
        )),
    }
}

/// Write a composition manifest, streaming cases without materializing them.
pub fn write_composition_manifest(
    domain: &CompositionDomain,
    output: &Path,
    maximum: &CaseCount,
) -> Result<(CaseCount, PathBuf), ToolsError> {
    let cases = composition_size(domain)?;
    if cases > *maximum {
        return Err(ToolsError::invalid(format!(
            "Declared product has {cases} cases, exceeding explicit generation limit {maximum}; no rows were omitted or emitted"
        )));
    }
    let destination = crate::verify::snapshot::resolve(output);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    }
    let parent = destination
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let temporary = make_temp_dir(&parent, ".composition-")?;
    let result = (|| -> Result<(CaseCount, PathBuf), ToolsError> {
        let path = temporary.join("manifest.json");
        let mut handle = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| ToolsError::io(format!("creating {}", path.display()), error))?;
        let mut id = String::new();
        escape_string(&domain.id, &mut id);
        handle
            .write_all(format!("{{\"schemaVersion\":1,\"id\":{id},\"cases\":[\n").as_bytes())
            .map_err(|error| ToolsError::io("writing manifest", error))?;
        let mut first = true;
        for item in generate_composition(domain)? {
            let case = item?;
            if first {
                first = false;
            } else {
                handle
                    .write_all(b",\n")
                    .map_err(|error| ToolsError::io("writing manifest", error))?;
            }
            handle
                .write_all(case.to_json().render().as_bytes())
                .map_err(|error| ToolsError::io("writing manifest", error))?;
        }
        handle
            .write_all(b"\n]}\n")
            .map_err(|error| ToolsError::io("writing manifest", error))?;
        handle
            .sync_all()
            .map_err(|error| ToolsError::io("syncing manifest", error))?;
        drop(handle);
        fs::rename(&path, &destination)
            .map_err(|error| ToolsError::io(format!("publishing {}", destination.display()), error))?;
        Ok((cases, destination))
    })();
    let _ = fs::remove_dir_all(&temporary);
    result
}

/// Run the generation CLI, returning the JSON line to print.
pub fn run(args: &[String]) -> Result<String, ToolsError> {
    let options = parse_args(args)?;
    let raw = parse_json(&read_text(&options.domain)?)?;
    let domain = parse_domain(&raw)?;
    if options.count_only {
        let cases = composition_size(&domain)?;
        let mut id = String::new();
        escape_string(&domain.id, &mut id);
        return Ok(format!(
            "{{\"domain\":{id},\"expectedCases\":\"{cases}\",\"generated\":false}}"
        ));
    }
    if let Some(output) = options.output {
        let (cases, destination) = write_composition_manifest(&domain, &output, &options.maximum)?;
        let mut id = String::new();
        escape_string(&domain.id, &mut id);
        let mut path = String::new();
        escape_string(&destination.to_string_lossy(), &mut path);
        return Ok(format!(
            "{{\"domain\":{id},\"expectedCases\":\"{cases}\",\"output\":{path},\"gameplayExecuted\":false}}"
        ));
    }
    Err(ToolsError::invalid(
        "Usage: qa-tools-verify-generate --domain domain.json [--count | --output manifest.json] [--max-cases N]",
    ))
}

/// Open a file for exclusive creation (shared with the runner).
pub fn create_exclusive(path: &Path) -> Result<File, ToolsError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| ToolsError::io(format!("creating {}", path.display()), error))
}
