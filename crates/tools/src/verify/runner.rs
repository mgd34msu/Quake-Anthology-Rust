//! Manifest execution (donor `tools/verify/runner.ts`).
//!
//! Runs every case in a manifest inside an owned snapshot with leased ports,
//! pinned inputs, drift detection, and resume support.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::error::ToolsError;
use crate::fsutil::{make_temp_dir, random_uuid};
use crate::json::Json;
use crate::process::which;
use crate::sys::{detach, kill_process_group, SIGKILL};
use crate::time::now_iso;
use crate::verify::accounting::{artifact_failures, assertion_failures, can_resume, reconcile};
use crate::verify::generate::create_exclusive;
use crate::verify::hash::{hash_bytes, hash_file, hash_json};
use crate::verify::isolation::{copy_pinned_file, lease_ports, start_private_display, PortLease, PrivateDisplay};
use crate::verify::product::{belongs_to_shard, Shard};
use crate::verify::schema::{
    Artifact, AttemptLink, AttemptProvenance, CaseManifest, Checkpoint, CommandContract, DisplayMode, DriverOutput,
    EvidenceKind, ExecutionRecord, ExpectedCase, Fingerprints, InputRequirement, Outcome, Reconciliation, RuntimeEnvironment,
    VerificationProfile, VerificationStatus,
};
use crate::verify::snapshot::{is_within, resolve, WorkspaceSnapshot};

/// A compiled executable candidate with its provenance.
#[derive(Debug, Clone)]
pub struct CandidateExecutable {
    /// Workspace-relative executable path.
    pub path: String,
    /// SHA-256 of the executable.
    pub sha256: String,
    /// Source snapshot it was built from.
    pub source_sha256: String,
}

/// Manifest run options.
pub struct RunOptions {
    /// Workspace root.
    pub workspace: PathBuf,
    /// Output parent directory.
    pub output_parent: PathBuf,
    /// Selecting profile.
    pub profile: VerificationProfile,
    /// Shard selection.
    pub shard: Shard,
    /// Changed paths for `--changed`, or `None`.
    pub changed_paths: Option<Vec<String>>,
    /// Compiled executable candidate, or `None`.
    pub executable: Option<CandidateExecutable>,
    /// Extra environment variables.
    pub environment: HashMap<String, String>,
    /// Extra library inputs.
    pub libraries: Vec<InputRequirement>,
    /// Previous records for resume.
    pub previous_records: Vec<ExecutionRecord>,
}

/// A finished manifest run.
pub struct RunReport {
    /// Run id.
    pub run_id: String,
    /// Output root.
    pub output_root: String,
    /// Snapshot root.
    pub snapshot_root: String,
    /// Source hash.
    pub source_sha256: String,
    /// Selecting profile.
    pub profile: VerificationProfile,
    /// Shard selection.
    pub shard: Shard,
    /// Manifest.
    pub manifest: CaseManifest,
    /// Selected case ids.
    pub selected_case_ids: Vec<String>,
    /// Records.
    pub records: Vec<ExecutionRecord>,
    /// Reconciliation.
    pub reconciliation: Reconciliation,
    /// Whether the selection is complete.
    pub selected_complete: bool,
}

impl RunReport {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("runId".to_owned(), Json::string(&self.run_id)),
            ("outputRoot".to_owned(), Json::string(&self.output_root)),
            ("snapshotRoot".to_owned(), Json::string(&self.snapshot_root)),
            ("sourceSha256".to_owned(), Json::string(&self.source_sha256)),
            ("profile".to_owned(), Json::string(self.profile.as_str())),
            (
                "shard".to_owned(),
                Json::object(vec![
                    ("index".to_owned(), Json::int(self.shard.index)),
                    ("count".to_owned(), Json::int(self.shard.count)),
                ]),
            ),
            ("manifest".to_owned(), self.manifest.to_json()),
            ("selectedCaseIds".to_owned(), Json::array(self.selected_case_ids.iter().map(Json::string).collect())),
            ("records".to_owned(), Json::array(self.records.iter().map(ExecutionRecord::to_json).collect())),
            ("reconciliation".to_owned(), self.reconciliation.to_json()),
            ("selectedComplete".to_owned(), Json::boolean(self.selected_complete)),
        ])
    }
}

fn artifact(output_root: &Path, path: &str) -> Result<Artifact, ToolsError> {
    let root = std::fs::canonicalize(output_root).map_err(|error| ToolsError::io(format!("resolving {}", output_root.display()), error))?;
    let relative = Path::new(path);
    let absolute = root.join(relative);
    let real = std::fs::canonicalize(&absolute).map_err(|error| ToolsError::io(format!("resolving {}", absolute.display()), error))?;
    if relative.is_absolute() || !is_within(&root, &absolute) || absolute == root || !is_within(&root, &real) {
        return Err(ToolsError::invalid(format!("Artifact escapes owned output: {path}")));
    }
    let stat = std::fs::symlink_metadata(&absolute).map_err(|error| ToolsError::io(format!("stating {}", absolute.display()), error))?;
    if !stat.is_file() || stat.is_symlink() {
        return Err(ToolsError::invalid(format!("Artifact is not a regular owned file: {path}")));
    }
    let bytes = std::fs::read(&absolute).map_err(|error| ToolsError::io(format!("reading {}", absolute.display()), error))?;
    Ok(Artifact {
        path: path.to_owned(),
        sha256: hash_bytes(&bytes),
        bytes: bytes.len() as i64,
    })
}

fn read_driver(path: &Path) -> Result<DriverOutput, ToolsError> {
    let text = std::fs::read_to_string(path).map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))?;
    crate::verify::schema::parse_driver_output(&crate::json::parse_json(&text)?)
}

struct InputIdentity {
    id: String,
    path: String,
    sha256: Option<String>,
}

impl InputIdentity {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("path".to_owned(), Json::string(&self.path)),
            ("sha256".to_owned(), self.sha256.as_deref().map_or(Json::Null, Json::string)),
        ])
    }
}

struct CheckedInputs {
    identities: Vec<InputIdentity>,
    missing: Vec<String>,
    changed: Vec<String>,
}

fn inspect_inputs(inputs: &[InputRequirement], workspace: &Path) -> Result<CheckedInputs, ToolsError> {
    let mut identities = Vec::new();
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    for input in inputs {
        let path = resolve(&workspace.join(&input.path));
        let check = (|| -> Result<String, ToolsError> {
            let stat = std::fs::symlink_metadata(&path).map_err(|error| ToolsError::io(format!("stating {}", path.display()), error))?;
            let real = std::fs::canonicalize(&path).map_err(|error| ToolsError::io(format!("resolving {}", path.display()), error))?;
            if !stat.is_file() || stat.is_symlink() || real != path {
                return Err(ToolsError::invalid("Required input is not a regular file without symlink ancestors"));
            }
            hash_file(&path)
        })();
        match check {
            Ok(actual) => {
                if input.sha256.is_none() {
                    missing.push(format!("{}: expected SHA-256 is not pinned", input.id));
                } else if Some(actual.as_str()) != input.sha256.as_deref() {
                    changed.push(format!("{}: input hash differs from the expected fixture", input.id));
                }
                identities.push(InputIdentity {
                    id: input.id.clone(),
                    path: input.path.clone(),
                    sha256: Some(actual),
                });
            }
            Err(error) => {
                identities.push(InputIdentity {
                    id: input.id.clone(),
                    path: input.path.clone(),
                    sha256: None,
                });
                missing.push(format!("{}: {error}", input.id));
            }
        }
    }
    Ok(CheckedInputs {
        identities,
        missing,
        changed,
    })
}

fn selected(item: &ExpectedCase, options: &RunOptions) -> Result<bool, ToolsError> {
    let profile_selected = matches!(options.profile, VerificationProfile::Full | VerificationProfile::Release) || item.profiles.contains(&options.profile);
    let changed_selected = match &options.changed_paths {
        None => true,
        Some(paths) => {
            item.source_paths.is_empty()
                || item.source_paths.iter().any(|source| paths.iter().any(|path| path == source || path.starts_with(&format!("{source}/"))))
        }
    };
    Ok(profile_selected && changed_selected && belongs_to_shard(&item.id, item.seed, options.shard)?)
}

fn substitute(value: &str, variables: &HashMap<String, String>) -> Result<String, ToolsError> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                if key.is_empty() || key.contains('{') || key.contains('}') {
                    return Err(ToolsError::invalid(format!("Unbound command placeholder {{{key}}}")));
                }
                match variables.get(key) {
                    Some(replacement) => out.push_str(replacement),
                    None => return Err(ToolsError::invalid(format!("Unbound command placeholder {{{key}}}"))),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

struct ProcessResult {
    outcome: Outcome,
    resolved_command: Vec<String>,
    output: Option<DriverOutput>,
    artifacts: Vec<Artifact>,
    checkpoints: Vec<Checkpoint>,
}

struct ExecuteContext<'a> {
    run_id: &'a str,
    output_root: PathBuf,
    snapshot_root: PathBuf,
    workspace: PathBuf,
    runtime_path: PathBuf,
    candidate_path: Option<PathBuf>,
    environment: &'a RuntimeEnvironment,
    display_provider: Option<InputRequirement>,
}

fn execute_case(item: &ExpectedCase, command: &CommandContract, context: &ExecuteContext) -> Result<ProcessResult, ToolsError> {
    let root = context.output_root.clone();
    let home = root.join("home");
    let temporary = root.join("tmp");
    let data = root.join("inputs");
    for path in [&home, &temporary, &data] {
        fs::create_dir_all(path).map_err(|error| ToolsError::io(format!("creating {}", path.display()), error))?;
    }
    let mut ports: PortLease = lease_ports(command.ports as usize, &format!("{}/{}", context.run_id, item.id))?;
    let outcome = execute_case_inner(item, command, context, &root, &home, &temporary, &data, &mut ports);
    ports.dispose();
    outcome
}

#[allow(clippy::too_many_lines)]
fn execute_case_inner(
    item: &ExpectedCase,
    command: &CommandContract,
    context: &ExecuteContext,
    root: &Path,
    home: &Path,
    temporary: &Path,
    data: &Path,
    ports: &mut PortLease,
) -> Result<ProcessResult, ToolsError> {
    let mut resolved_command = Vec::new();
    let mut artifacts = Vec::new();
    let mut display: Option<PrivateDisplay> = None;
    let mut variables = HashMap::from([
        ("snapshot".to_owned(), context.snapshot_root.to_string_lossy().into_owned()),
        ("output".to_owned(), root.to_string_lossy().into_owned()),
        ("home".to_owned(), home.to_string_lossy().into_owned()),
        ("data".to_owned(), data.to_string_lossy().into_owned()),
        ("case".to_owned(), item.id.clone()),
        ("result".to_owned(), root.join("driver-result.json").to_string_lossy().into_owned()),
        ("bun".to_owned(), context.runtime_path.to_string_lossy().into_owned()),
    ]);
    if let Some(candidate) = &context.candidate_path {
        variables.insert("executable".to_owned(), candidate.to_string_lossy().into_owned());
    }
    for (index, port) in ports.ports.iter().enumerate() {
        variables.insert(format!("port:{index}"), port.to_string());
    }
    let result = (|resolved_command: &mut Vec<String>, artifacts: &mut Vec<Artifact>, display: &mut Option<PrivateDisplay>| -> Result<ProcessResult, ToolsError> {
        for input in &item.requirements {
            match &input.sha256 {
                None => return Err(ToolsError::invalid(format!("Unpinned input {} reached execution", input.id))),
                Some(pinned) => {
                    let destination = data.join(hash_bytes(input.id.as_bytes()));
                    copy_pinned_file(&resolve(&context.workspace.join(&input.path)), &destination, pinned, input.kind == crate::verify::schema::InputKind::Executable)?;
                    variables.insert(format!("input:{}", input.id), destination.to_string_lossy().into_owned());
                }
            }
        }
        let executable = substitute(&command.executable, &variables)?;
        let executable_path = {
            let candidate = PathBuf::from(&executable);
            if candidate.is_absolute() {
                candidate
            } else {
                resolve(&context.snapshot_root.join(candidate))
            }
        };
        let from_snapshot = is_within(&context.snapshot_root, &executable_path);
        let is_runtime = executable_path == context.runtime_path;
        let is_candidate = context.candidate_path.as_ref().is_some_and(|candidate| *candidate == executable_path);
        let from_data = is_within(data, &executable_path);
        if !from_snapshot && !is_runtime && !is_candidate && !from_data {
            return Err(ToolsError::invalid(
                "Command executable must come from owned source, pinned runtime, candidate, or required inputs",
            ));
        }
        resolved_command.push(executable_path.to_string_lossy().into_owned());
        for value in &command.args {
            resolved_command.push(substitute(value, &variables)?);
        }
        let mut env: HashMap<String, String> = context.environment.variables.iter().cloned().collect();
        for (key, value) in &command.environment {
            env.insert(key.clone(), substitute(value, &variables)?);
        }
        let sdl_audio = command.environment.iter().find(|(key, _)| key == "SDL_AUDIODRIVER").map_or("dummy", |(_, value)| value.as_str());
        env.insert("HOME".to_owned(), home.to_string_lossy().into_owned());
        env.insert("TMPDIR".to_owned(), temporary.to_string_lossy().into_owned());
        env.insert("XDG_CONFIG_HOME".to_owned(), home.join("config").to_string_lossy().into_owned());
        env.insert("XDG_DATA_HOME".to_owned(), home.join("data").to_string_lossy().into_owned());
        env.insert("XDG_CACHE_HOME".to_owned(), home.join("cache").to_string_lossy().into_owned());
        env.insert("XDG_RUNTIME_DIR".to_owned(), home.join("runtime").to_string_lossy().into_owned());
        env.insert("VERIFY_OUTPUT_ROOT".to_owned(), root.to_string_lossy().into_owned());
        env.insert("VERIFY_DATA_ROOT".to_owned(), data.to_string_lossy().into_owned());
        env.insert("VERIFY_CASE_ID".to_owned(), item.id.clone());
        env.insert("VERIFY_RESULT_PATH".to_owned(), root.join("driver-result.json").to_string_lossy().into_owned());
        env.insert("VERIFY_PORTS".to_owned(), ports.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(","));
        env.insert("VERIFY_SEED".to_owned(), item.seed.to_string());
        env.insert(
            "SDL_VIDEODRIVER".to_owned(),
            match command.display {
                DisplayMode::Offscreen => "offscreen",
                DisplayMode::Xvfb => "x11",
                DisplayMode::Headless => "dummy",
            }
            .to_owned(),
        );
        env.insert("SDL_AUDIODRIVER".to_owned(), sdl_audio.to_owned());
        if command.display == DisplayMode::Xvfb {
            match &context.display_provider {
                Some(provider) if provider.sha256.is_some() => {
                    let xvfb = root.join("xvfb");
                    copy_pinned_file(
                        Path::new(&provider.path),
                        &xvfb,
                        provider.sha256.as_deref().unwrap_or_default(),
                        true,
                    )?;
                    let started = start_private_display(&xvfb, root, &env)?;
                    env.insert("DISPLAY".to_owned(), started.display.clone());
                    *display = Some(started);
                }
                _ => return Err(ToolsError::invalid("Private display executable was not pinned")),
            }
        }
        let runtime_dir = env
            .get("XDG_RUNTIME_DIR")
            .cloned()
            .unwrap_or_else(|| home.join("runtime").to_string_lossy().into_owned());
        fs::create_dir_all(&runtime_dir).map_err(|error| ToolsError::io(format!("creating {runtime_dir}"), error))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime_dir, fs::Permissions::from_mode(0o700))
                .map_err(|error| ToolsError::io(format!("setting mode on {runtime_dir}"), error))?;
        }
        let launch = Json::object(vec![
            ("command".to_owned(), Json::array(resolved_command.iter().map(Json::string).collect())),
            ("cwd".to_owned(), Json::string(context.snapshot_root.to_string_lossy())),
            (
                "environment".to_owned(),
                Json::object({
                    let mut pairs: Vec<(String, Json)> = env.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect();
                    pairs.sort_by(|left, right| left.0.cmp(&right.0));
                    pairs
                }),
            ),
            ("ports".to_owned(), Json::array(ports.ports.iter().map(|port| Json::int(i64::from(*port))).collect())),
        ]);
        {
            let mut file = create_exclusive(&root.join("launch.json"))?;
            use std::io::Write;
            file.write_all(launch.render_pretty().as_bytes()).map_err(|error| ToolsError::io("writing launch.json", error))?;
        }
        ports.release_sockets();
        let stdout_path = root.join("stdout.txt");
        let stderr_path = root.join("stderr.txt");
        let stdout = File::create(&stdout_path).map_err(|error| ToolsError::io(format!("creating {}", stdout_path.display()), error))?;
        let stderr = File::create(&stderr_path).map_err(|error| ToolsError::io(format!("creating {}", stderr_path.display()), error))?;
        let (program, args) = resolved_command.split_first().ok_or_else(|| ToolsError::invalid("Command executable must come from owned source, pinned runtime, candidate, or required inputs"))?;
        let mut spawn = Command::new(program);
        spawn
            .args(args)
            .current_dir(&context.snapshot_root)
            .env_clear()
            .envs(&env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        detach(&mut spawn);
        let mut child = spawn.spawn().map_err(|error| ToolsError::io(format!("spawning {program}"), error))?;
        let pid = child.id();
        let (timed_out, exit_code) = wait_child(&mut child, command.timeout_ms)?;
        kill_process_group(pid, SIGKILL);
        let _ = child.kill();
        let mut names = vec!["launch.json", "stdout.txt", "stderr.txt"];
        if display.is_some() {
            names.extend(["display.txt", "display-stderr.txt"]);
        }
        for path in names {
            artifacts.push(artifact(root, path)?);
        }
        let mut output: Option<DriverOutput> = None;
        let mut reasons = Vec::new();
        match read_driver(&root.join("driver-result.json")) {
            Ok(parsed) => {
                artifacts.push(artifact(root, "driver-result.json")?);
                for path in &parsed.artifact_paths {
                    if artifacts.iter().any(|existing| &existing.path == path) {
                        return Err(ToolsError::invalid(format!("Duplicate or reserved driver artifact {path}")));
                    }
                    artifacts.push(artifact(root, path)?);
                }
                output = Some(parsed);
            }
            Err(error) => reasons.push(format!("Missing or invalid driver output: {error}")),
        }
        if timed_out {
            let checkpoints = output.as_ref().map_or_else(Vec::new, |parsed| parsed.checkpoints.clone());
            return Ok(ProcessResult {
                outcome: Outcome::Timeout {
                    timeout_ms: command.timeout_ms,
                    exit_code,
                },
                resolved_command: std::mem::take(resolved_command),
                output,
                artifacts: std::mem::take(artifacts),
                checkpoints,
            });
        }
        let checkpoints = output.as_ref().map_or_else(Vec::new, |parsed| parsed.checkpoints.clone());
        if exit_code != Some(0) {
            reasons.push(format!("Driver exited {}", exit_code.map_or("null".to_owned(), |code| code.to_string())));
        }
        if let Some(parsed) = &output {
            if parsed.case_id != item.id {
                reasons.push("Driver output case ID differs from expected case".to_owned());
            }
            reasons.extend(assertion_failures(item, &parsed.assertions, parsed.assertions.len() as i64)?);
        }
        Ok(ProcessResult {
            outcome: if reasons.is_empty() {
                Outcome::Pass
            } else {
                Outcome::Fail {
                    exit_code,
                    reasons,
                }
            },
            resolved_command: std::mem::take(resolved_command),
            output,
            artifacts: std::mem::take(artifacts),
            checkpoints,
        })
    })(&mut resolved_command, &mut artifacts, &mut display);
    if let Some(started) = display {
        started.dispose();
    }
    match result {
        Ok(process) => Ok(process),
        Err(error) => Ok(ProcessResult {
            outcome: Outcome::Fail {
                exit_code: None,
                reasons: vec![error.to_string()],
            },
            resolved_command,
            output: None,
            artifacts,
            checkpoints: Vec::new(),
        }),
    }
}

fn wait_child(child: &mut std::process::Child, timeout_ms: i64) -> Result<(bool, Option<i64>), ToolsError> {
    let start = Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms.max(0) as u64);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok((false, status.code().map(i64::from))),
            Ok(None) => {}
            Err(error) => return Err(ToolsError::io("waiting for driver", error)),
        }
        if start.elapsed() >= timeout {
            kill_process_group(child.id(), SIGKILL);
            let _ = child.kill();
            let _ = child.wait();
            return Ok((true, None));
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn os_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|text| text.trim().to_owned()).unwrap_or_else(|_| "unknown".to_owned())
}

fn cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("model name")?.split_once(':').map(|(_, model)| model.trim().to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

fn runtime_info() -> Result<(PathBuf, String, String), ToolsError> {
    let bun = which("bun").ok_or_else(|| ToolsError::invalid("Verification runtime requires bun on PATH"))?;
    let output = Command::new(&bun)
        .arg("--version")
        .output()
        .map_err(|error| ToolsError::io("querying bun version", error))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let version = if version.is_empty() { "unknown".to_owned() } else { version };
    let canonical = std::fs::canonicalize(&bun).map_err(|error| ToolsError::io(format!("resolving {}", bun.display()), error))?;
    let digest = hash_file(&canonical)?;
    Ok((canonical, version, digest))
}

/// Run every case in a manifest.
pub fn run_manifest(manifest: &CaseManifest, options: &RunOptions) -> Result<RunReport, ToolsError> {
    let workspace = std::fs::canonicalize(&options.workspace).map_err(|error| ToolsError::io(format!("resolving {}", options.workspace.display()), error))?;
    let output_parent = resolve(&options.output_parent);
    if is_within(&workspace, &output_parent) && !is_within(&workspace.join(".artifacts"), &output_parent) {
        return Err(ToolsError::invalid("Output roots inside source must be under .artifacts"));
    }
    let mut previous: HashMap<&str, &ExecutionRecord> = HashMap::new();
    for record in &options.previous_records {
        if previous.contains_key(record.case_id.as_str()) {
            return Err(ToolsError::invalid(format!("Resume input contains duplicate case ID {}", record.case_id)));
        }
        previous.insert(record.case_id.as_str(), record);
    }
    let snapshot = WorkspaceSnapshot::capture(&workspace, &[])?;
    fs::create_dir_all(&output_parent).map_err(|error| ToolsError::io(format!("creating {}", output_parent.display()), error))?;
    let output_root = make_temp_dir(&output_parent, "run-")?;
    let run_id = random_uuid()?;
    let snapshot_root = snapshot.materialize(&output_root.join("source"), "verify-")?;
    {
        let mut file = create_exclusive(&output_root.join("snapshot.json"))?;
        use std::io::Write;
        file.write_all(snapshot.manifest.to_json().render_pretty().as_bytes())
            .map_err(|error| ToolsError::io("writing snapshot.json", error))?;
    }
    let (runtime_source, bun_version, runtime_hash) = runtime_info()?;
    let runtime_path = output_root.join("bun");
    copy_pinned_file(&runtime_source, &runtime_path, &runtime_hash, true)?;
    let mut candidate_path: Option<PathBuf> = None;
    let mut candidate_failure: Option<String> = None;
    if let Some(executable) = &options.executable {
        if executable.source_sha256 != snapshot.manifest.sha256 {
            candidate_failure = Some("Compiled executable was built from a different source snapshot".to_owned());
        } else {
            let staged = output_root.join("candidate");
            match copy_pinned_file(&resolve(&workspace.join(&executable.path)), &staged, &executable.sha256, true) {
                Ok(()) => candidate_path = Some(staged),
                Err(error) => candidate_failure = Some(format!("Compiled executable is stale or absent: {error}")),
            }
        }
    }
    let mut records = Vec::new();
    let mut selected_case_ids = Vec::new();
    for item in &manifest.cases {
        let start = Instant::now();
        let started_at = now_iso();
        let attempt_id = random_uuid()?;
        let attempt_root = output_root.join("attempts").join(&attempt_id);
        fs::create_dir_all(&attempt_root).map_err(|error| ToolsError::io(format!("creating {}", attempt_root.display()), error))?;
        let display_path = if item.command.as_ref().is_some_and(|command| command.display == DisplayMode::Xvfb) {
            which("Xvfb")
        } else {
            None
        };
        let display_provider = match display_path {
            None => None,
            Some(path) => {
                let canonical = std::fs::canonicalize(&path).map_err(|error| ToolsError::io(format!("resolving {}", path.display()), error))?;
                let digest = hash_file(&canonical)?;
                Some(InputRequirement {
                    id: "runner:private-display".to_owned(),
                    kind: crate::verify::schema::InputKind::Executable,
                    path: canonical.to_string_lossy().into_owned(),
                    sha256: Some(digest),
                })
            }
        };
        let mut variables = HashMap::from([
            ("PATH".to_owned(), std::env::var("PATH").unwrap_or_default()),
            ("LANG".to_owned(), "C.UTF-8".to_owned()),
            ("TZ".to_owned(), "UTC".to_owned()),
        ]);
        variables.extend(options.environment.iter().map(|(key, value)| (key.clone(), value.clone())));
        if let Some(command) = &item.command {
            variables.extend(command.environment.iter().map(|(key, value)| (key.clone(), value.clone())));
        }
        let mut libraries = options.libraries.clone();
        if let Some(provider) = &display_provider {
            libraries.push(provider.clone());
        }
        let environment = RuntimeEnvironment {
            platform: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            os_release: os_release(),
            bun_version: bun_version.clone(),
            cpu: cpu_model(),
            libraries: libraries.clone(),
            variables: {
                let mut pairs: Vec<(String, String)> = variables.into_iter().collect();
                pairs.sort_by(|left, right| left.0.cmp(&right.0));
                pairs
            },
        };
        let mut all_inputs = item.requirements.to_vec();
        all_inputs.extend(environment.libraries.iter().cloned());
        let checked = inspect_inputs(&all_inputs, &workspace)?;
        let fingerprints = Fingerprints {
            manifest: hash_json(&manifest.to_json())?,
            expected_case: hash_json(&item.to_json())?,
            source: snapshot.manifest.sha256.clone(),
            snapshot: snapshot.manifest.sha256.clone(),
            executable: options.executable.as_ref().map_or_else(|| runtime_hash.clone(), |executable| executable.sha256.clone()),
            runtime_executable: runtime_hash.clone(),
            fixtures: hash_json(&Json::array(checked.identities.iter().map(InputIdentity::to_json).collect()))?,
            environment: hash_json(&environment.to_json())?,
            clock_schedule: item.clock_schedule_sha256.clone(),
            network_schedule: item.network_schedule_sha256.clone(),
            seed: item.seed,
        };
        let prior = previous.get(item.id.as_str()).copied();
        let previous_attempt = prior.map(|record| AttemptLink {
            run_id: record.provenance.run_id.clone(),
            attempt_id: record.provenance.attempt_id.clone(),
            record_sha256: hash_json(&record.to_json()).unwrap_or_default(),
        });
        let mut missing = checked.missing.clone();
        for contract in &item.contracts {
            if contract.oracle.sha256.is_none() {
                missing.push(format!("{}: independent oracle is not pinned", contract.id));
            }
        }
        if item.clock_schedule_sha256.is_none() {
            missing.push("Clock schedule is not pinned".to_owned());
        }
        if item.network_schedule_sha256.is_none() {
            missing.push("Network schedule is not pinned".to_owned());
        }
        if item.command.as_ref().is_some_and(|command| command.display == DisplayMode::Xvfb) && display_provider.is_none() {
            missing.push("Private Xvfb display executable is unavailable".to_owned());
        }
        let chosen = selected(item, options)?;
        if chosen {
            selected_case_ids.push(item.id.clone());
        }
        let base = |outcome: Outcome| ExecutionRecord {
            case_id: item.id.clone(),
            configuration_id: item.configuration_id.clone(),
            suite_id: item.suite_id.clone(),
            evidence_kind: item.evidence_kind,
            contracts: item.contracts.clone(),
            provenance: AttemptProvenance {
                run_id: run_id.clone(),
                attempt_id: attempt_id.clone(),
                previous_attempt: previous_attempt.clone(),
                reuse: None,
            },
            fingerprints: fingerprints.clone(),
            inputs: item.requirements.clone(),
            environment: environment.clone(),
            command: item.command.clone(),
            resolved_command: Vec::new(),
            output_root: attempt_root.to_string_lossy().into_owned(),
            started_at: started_at.clone(),
            finished_at: started_at.clone(),
            duration_ms: 0.0,
            assertion_count: 0,
            assertions: Vec::new(),
            checkpoints: Vec::new(),
            artifacts: Vec::new(),
            outcome,
        };
        let binds_executable = item.command.as_ref().is_some_and(|command| {
            std::iter::once(command.executable.as_str()).chain(command.args.iter().map(String::as_str)).any(|value| value.contains("{executable}"))
        });
        let record = if !chosen {
            base(Outcome::NotRun {
                reason: "Case is outside this explicit profile, changed-path selection, or shard; it remains required".to_owned(),
            })
        } else if !missing.is_empty() {
            base(Outcome::BlockedMissingInput { missing_inputs: missing })
        } else if !checked.changed.is_empty() || candidate_failure.is_some() {
            let mut reasons = checked.changed.clone();
            if let Some(failure) = &candidate_failure {
                reasons.push(failure.clone());
            }
            base(Outcome::Fail { exit_code: None, reasons })
        } else if item.command.is_none() {
            base(Outcome::NotRun {
                reason: "Required behavior has no bound executable driver".to_owned(),
            })
        } else if options.profile == VerificationProfile::Release && candidate_path.is_none() {
            base(Outcome::BlockedMissingInput {
                missing_inputs: vec!["Release profile requires an exact compiled executable and matching build provenance".to_owned()],
            })
        } else if options.profile == VerificationProfile::Release && item.evidence_kind != EvidenceKind::Tooling && !binds_executable {
            base(Outcome::NotRun {
                reason: "Release behavior driver does not bind the exact candidate executable".to_owned(),
            })
        } else if prior.is_some_and(|record| can_resume(item, &fingerprints, record).is_ok_and(|decision| decision.reusable)) {
            let prior = prior.expect("resume prior");
            for prior_artifact in &prior.artifacts {
                let destination = attempt_root.join(&prior_artifact.path);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
                }
                copy_pinned_file(
                    &resolve(&Path::new(&prior.output_root).join(&prior_artifact.path)),
                    &destination,
                    &prior_artifact.sha256,
                    false,
                )?;
            }
            ExecutionRecord {
                provenance: AttemptProvenance {
                    run_id: run_id.clone(),
                    attempt_id: attempt_id.clone(),
                    previous_attempt: previous_attempt.clone(),
                    reuse: previous_attempt.clone(),
                },
                output_root: attempt_root.to_string_lossy().into_owned(),
                started_at: started_at.clone(),
                finished_at: now_iso(),
                duration_ms: start.elapsed().as_secs_f64() * 1000.0,
                ..prior.clone()
            }
        } else {
            let command = item.command.as_ref().expect("bound command");
            let context = ExecuteContext {
                run_id: &run_id,
                output_root: attempt_root.clone(),
                snapshot_root: snapshot_root.clone(),
                workspace: workspace.clone(),
                runtime_path: runtime_path.clone(),
                candidate_path: candidate_path.clone(),
                environment: &environment,
                display_provider,
            };
            let result = execute_case(item, command, &context)?;
            let after = inspect_inputs(&all_inputs, &workspace)?;
            let source_after = WorkspaceSnapshot::capture(&workspace, &[])?;
            let snapshot_after = WorkspaceSnapshot::capture(&snapshot_root, &[])?;
            let mut drift = Vec::new();
            if hash_json(&Json::array(after.identities.iter().map(InputIdentity::to_json).collect()))?
                != hash_json(&Json::array(checked.identities.iter().map(InputIdentity::to_json).collect()))?
            {
                drift.push("Required input changed during execution".to_owned());
            }
            if source_after.manifest.sha256 != snapshot.manifest.sha256 {
                drift.push("Source workspace changed during execution".to_owned());
            }
            if snapshot_after.manifest.sha256 != snapshot.manifest.sha256 {
                drift.push("Owned source snapshot changed during execution".to_owned());
            }
            if hash_file(&runtime_path)? != runtime_hash {
                drift.push("Runtime executable changed during execution".to_owned());
            }
            if let Some(candidate) = &candidate_path {
                let expected = options.executable.as_ref().map(|executable| executable.sha256.as_str());
                if hash_file(candidate)? != expected.unwrap_or_default() {
                    drift.push("Candidate executable changed during execution".to_owned());
                }
            }
            let outcome = if drift.is_empty() {
                result.outcome
            } else {
                let exit_code = match &result.outcome {
                    Outcome::Pass => Some(0),
                    Outcome::Fail { exit_code, .. } | Outcome::Timeout { exit_code, .. } => *exit_code,
                    Outcome::BlockedMissingInput { .. } | Outcome::NotRun { .. } => None,
                };
                Outcome::Fail {
                    exit_code,
                    reasons: drift,
                }
            };
            ExecutionRecord {
                resolved_command: result.resolved_command,
                finished_at: now_iso(),
                duration_ms: start.elapsed().as_secs_f64() * 1000.0,
                assertion_count: result.output.as_ref().map_or(0, |output| output.assertions.len() as i64),
                assertions: result.output.as_ref().map_or_else(Vec::new, |output| output.assertions.clone()),
                checkpoints: result.checkpoints,
                artifacts: result.artifacts,
                outcome,
                ..base(Outcome::NotRun { reason: String::new() })
            }
        };
        {
            let mut file = create_exclusive(&attempt_root.join("record.json"))?;
            use std::io::Write;
            file.write_all(record.to_json().render_pretty().as_bytes()).map_err(|error| ToolsError::io("writing record.json", error))?;
        }
        records.push(record);
    }
    let reconciliation = reconcile(manifest, &records)?;
    let selected_set: std::collections::HashSet<&str> = selected_case_ids.iter().map(String::as_str).collect();
    let selected_complete = !selected_case_ids.is_empty()
        && records.iter().filter(|record| selected_set.contains(record.case_id.as_str())).all(|record| record.outcome.status() == VerificationStatus::Pass)
        && reconciliation.invalid_records.is_empty();
    let report = RunReport {
        run_id,
        output_root: output_root.to_string_lossy().into_owned(),
        snapshot_root: snapshot_root.to_string_lossy().into_owned(),
        source_sha256: snapshot.manifest.sha256.clone(),
        profile: options.profile,
        shard: options.shard,
        manifest: manifest.clone(),
        selected_case_ids,
        records,
        reconciliation,
        selected_complete,
    };
    {
        let mut file = create_exclusive(&output_root.join("report.json"))?;
        use std::io::Write;
        file.write_all(report.to_json().render_pretty().as_bytes()).map_err(|error| ToolsError::io("writing report.json", error))?;
    }
    Ok(report)
}

/// Reconcile an archived manifest and records, including artifact checks.
pub fn verify_archived_report(manifest: &CaseManifest, records: &[ExecutionRecord]) -> Result<Reconciliation, ToolsError> {
    let report = reconcile(manifest, records)?;
    let mut invalid_records = report.invalid_records;
    for record in records {
        let reasons = artifact_failures(record)?;
        if !reasons.is_empty() {
            invalid_records.push(crate::verify::schema::InvalidRecord {
                case_id: record.case_id.clone(),
                reasons,
            });
        }
    }
    Ok(Reconciliation {
        complete: report.complete && invalid_records.is_empty(),
        gameplay_complete: report.gameplay_complete && invalid_records.is_empty(),
        invalid_records,
        ..report
    })
}
