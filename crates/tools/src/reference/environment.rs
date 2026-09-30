//! Reference environment inventory (donor `tools/reference/environment.ts`).
//!
//! Identifies files, observes commands, tools, sources, binaries, and Steam
//! state, and assembles the reference-environment manifest. The donor is
//! `async` over Bun primitives; this port is synchronous and uses the
//! crate's [`crate::process`] capture, [`crate::sha256`] streaming hash,
//! and [`crate::fsutil`] helpers instead.
//!
//! Root resolution: the donor derives the quake-typescript checkout from its
//! own file URL. Rust binaries have no equivalent, so the checkout defaults
//! to the `quake-typescript` sibling of the build workspace (via
//! `CARGO_MANIFEST_DIR`) and `QA_QUAKE_TS_ROOT` overrides it. The
//! `bunVersion` manifest field records the executing tool version because the
//! capture program is now this Rust binary, and `captureProgram` identifies
//! the three Rust sources that replaced the donor's TypeScript files.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::ToolsError;
use crate::fsutil;
use crate::inventory::source_census::{self, RepositoryRole, RepositorySpec};
use crate::process::{self, EnvSpec};
use crate::reference::schema::{
    BinaryFormat, BinaryObservation, BinaryProvenance, BinaryPurpose, CensusReference, CommandObservation,
    CommandOutcome, Discovery, FileIdentity, PathObservation, ReadObservation, ReferenceEnvironment, SourceIdentity,
    SourceRole, SourceState, SteamEdition, SteamObservation, TitleAvailability, ToolObservation,
};
use crate::reference::steam::{self, SteamTitle};
use crate::reference::{node_arch, node_platform};
use crate::sha256::Hasher;
use crate::time::now_iso;

/// quake-typescript checkout root (donor `projectRoot`).
///
/// Defaults to the nearest `quake-typescript` sibling found by walking up
/// from the build workspace (worktree checkouts nest deeper than the main
/// checkout, so no fixed depth is assumed).
#[must_use]
pub fn quake_typescript_root() -> PathBuf {
    if let Ok(root) = std::env::var("QA_QUAKE_TS_ROOT") {
        return PathBuf::from(root);
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        let candidate = ancestor.join("quake-typescript");
        if candidate.join("tools").is_dir() {
            return candidate;
        }
    }
    fsutil::lexical_absolute(manifest, "../../../quake-typescript")
}

/// Parent directory holding the sibling checkouts (donor `projectsRoot`).
#[must_use]
pub fn projects_root() -> PathBuf {
    quake_typescript_root()
        .parent()
        .map_or_else(|| PathBuf::from(".."), Path::to_path_buf)
}

/// Original-source tree root (donor `sourceRoot`).
#[must_use]
pub fn source_root() -> String {
    projects_root().join("qsrc").to_string_lossy().into_owned()
}

/// Supplied-corpus root (donor `corpusRoot`).
#[must_use]
pub fn corpus_root() -> String {
    projects_root().join("qfiles").to_string_lossy().into_owned()
}

#[cfg(unix)]
fn file_id(details: &std::fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    details.ino()
}

#[cfg(not(unix))]
fn file_id(_details: &std::fs::Metadata) -> u64 {
    0
}

/// Identify a file: canonical path, byte size, streaming SHA-256.
///
/// The file must be a regular file and must not change (size, mtime, inode)
/// while it is hashed, matching the donor's before/after guard.
pub fn identify_file(path: &str) -> Result<FileIdentity, ToolsError> {
    let absolute = std::fs::canonicalize(path).map_err(|error| ToolsError::io(format!("resolving {path}"), error))?;
    let absolute_text = absolute.to_string_lossy().into_owned();
    let before =
        std::fs::metadata(&absolute).map_err(|error| ToolsError::io(format!("stating {absolute_text}"), error))?;
    if !before.is_file() {
        return Err(ToolsError::invalid(format!("Expected a regular file: {absolute_text}")));
    }
    let mut file = File::open(&absolute).map_err(|error| ToolsError::io(format!("reading {absolute_text}"), error))?;
    let mut hasher = Hasher::new();
    let mut chunk = [0u8; 65536];
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|error| ToolsError::io(format!("reading {absolute_text}"), error))?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    let after =
        std::fs::metadata(&absolute).map_err(|error| ToolsError::io(format!("stating {absolute_text}"), error))?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || file_id(&before) != file_id(&after)
    {
        return Err(ToolsError::invalid(format!(
            "Input changed during capture: {absolute_text}"
        )));
    }
    Ok(FileIdentity {
        path: absolute_text,
        size: after.len(),
        sha256: crate::sha256::to_hex(&hasher.finish()),
    })
}

/// Observe a command execution with captured streams and timing.
///
/// The child inherits the environment plus `LC_ALL=C`; `timeout_ms`
/// defaults to 15 seconds. A signaled process outside a timeout is an error
/// because the schema has no representation for it.
pub fn observe_command(
    command: &[String],
    cwd: &str,
    timeout_ms: Option<u64>,
) -> Result<CommandObservation, ToolsError> {
    let timeout = timeout_ms.unwrap_or(15_000);
    let started_at = now_iso();
    let start = Instant::now();
    let mut extra = HashMap::new();
    extra.insert("LC_ALL".to_owned(), "C".to_owned());
    let completed = process::run_capture(command, Path::new(cwd), &EnvSpec::InheritWith(extra), timeout)?;
    let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
    let outcome = if completed.timed_out {
        CommandOutcome::TimedOut { timeout_ms: timeout }
    } else {
        match completed.exit_code {
            Some(exit_code) => CommandOutcome::Exited { exit_code },
            None => {
                return Err(ToolsError::command(format!(
                    "Command terminated by signal: {}",
                    command.join(" ")
                )))
            }
        }
    };
    Ok(CommandObservation {
        command: command.to_vec(),
        cwd: cwd.to_owned(),
        environment: vec![("LC_ALL".to_owned(), "C".to_owned())],
        started_at,
        duration_ms,
        outcome,
        stdout: completed.stdout,
        stderr: completed.stderr,
    })
}

/// Standard output of a successful observation, else a `Command failed` error.
fn successful_output(observation: &CommandObservation) -> Result<&str, ToolsError> {
    if !matches!(observation.outcome, CommandOutcome::Exited { exit_code: 0 }) {
        return Err(ToolsError::invalid(format!(
            "Command failed: {}: {}",
            observation.command.join(" "),
            observation.stderr
        )));
    }
    Ok(&observation.stdout)
}

/// Observe one changed path: missing links read as deleted.
fn observe_path(path: &str) -> Result<PathObservation, ToolsError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PathObservation::Deleted { path: path.to_owned() })
        }
        Err(error) => Err(ToolsError::io(format!("stating {path}"), error)),
        Ok(details) => {
            let file_type = details.file_type();
            if file_type.is_symlink() {
                let target =
                    std::fs::read_link(path).map_err(|error| ToolsError::io(format!("reading link {path}"), error))?;
                Ok(PathObservation::Symlink {
                    path: path.to_owned(),
                    target: target.to_string_lossy().into_owned(),
                })
            } else if file_type.is_dir() {
                Ok(PathObservation::Directory { path: path.to_owned() })
            } else {
                Ok(PathObservation::File {
                    identity: identify_file(path)?,
                })
            }
        }
    }
}

/// Identify one censused source tree: head, tree, status, and changed paths.
fn identify_source(project_root: &Path, spec: &RepositorySpec) -> Result<SourceIdentity, ToolsError> {
    let path = project_root.join(&spec.path);
    let path_text = path.to_string_lossy().into_owned();
    let probe = |args: &[&str]| {
        observe_command(
            &args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<String>>(),
            &path_text,
            None,
        )
    };
    let head = probe(&["git", "rev-parse", "HEAD"])?;
    let tree = probe(&["git", "rev-parse", "HEAD^{tree}"])?;
    let status = probe(&["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let changed = probe(&["git", "diff", "--name-only", "-z", "HEAD"])?;
    let untracked = probe(&["git", "ls-files", "--others", "--exclude-standard", "-z"])?;
    let mut changed_paths = BTreeSet::new();
    for output in [successful_output(&changed)?, successful_output(&untracked)?] {
        for relative in output.split('\0').filter(|part| !part.is_empty()) {
            changed_paths.insert(relative);
        }
    }
    let mut changes = Vec::with_capacity(changed_paths.len());
    for relative in changed_paths {
        changes.push(observe_path(&path.join(relative).to_string_lossy().into_owned())?);
    }
    let final_status = probe(&["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    if successful_output(&final_status)? != successful_output(&status)? {
        return Err(ToolsError::invalid(format!("Source state changed: {path_text}")));
    }
    let head_text = successful_output(&head)?.trim().to_owned();
    if head_text != spec.expected_revision {
        return Err(ToolsError::invalid(format!(
            "Source revision differs from census: {path_text}"
        )));
    }
    Ok(SourceIdentity {
        source_id: spec.id.clone(),
        path: path_text,
        role: match spec.role {
            RepositoryRole::OriginalReference => SourceRole::OriginalSource,
            RepositoryRole::ImplementationCandidate => SourceRole::TypescriptDonor,
        },
        expected_head: spec.expected_revision.clone(),
        head: head_text,
        tree: successful_output(&tree)?.trim().to_owned(),
        state: if successful_output(&status)?.is_empty() {
            SourceState::Clean
        } else {
            SourceState::Modified
        },
        changes,
        observations: vec![head, tree, status, changed, untracked, final_status],
    })
}

/// Observe one tool: missing executables degrade to `unavailable`.
fn observe_tool(name: &str, arguments: &[&[&str]], project_root: &str) -> Result<ToolObservation, ToolsError> {
    let Some(path) = process::which(name) else {
        return Ok(ToolObservation::Unavailable { name: name.to_owned() });
    };
    let executable = path.to_string_lossy().into_owned();
    let mut observations = Vec::with_capacity(arguments.len());
    for args in arguments {
        let mut command = Vec::with_capacity(args.len() + 1);
        command.push(executable.clone());
        command.extend(args.iter().map(|arg| (*arg).to_owned()));
        observations.push(observe_command(&command, project_root, None)?);
    }
    Ok(ToolObservation::Available {
        name: name.to_owned(),
        executable: identify_file(&executable)?,
        observations,
    })
}

#[cfg(unix)]
fn has_exec_bit(details: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    details.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn has_exec_bit(_details: &std::fs::Metadata) -> bool {
    false
}

/// Read up to `count` leading bytes without loading the whole file.
fn read_prefix(path: &str, count: usize) -> Result<Vec<u8>, ToolsError> {
    let file = File::open(path).map_err(|error| ToolsError::io(format!("reading {path}"), error))?;
    let mut bytes = Vec::new();
    file.take(count as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| ToolsError::io(format!("reading {path}"), error))?;
    Ok(bytes)
}

/// Whether a file name matches the donor's retail-engine pattern
/// (`/^(?:winquake|glquake|(?:gl)?qwcl|quake.*)\.exe$/i`).
fn retail_engine_name(name: &str) -> bool {
    let Some(stem) = name.to_lowercase().strip_suffix(".exe").map(str::to_owned) else {
        return false;
    };
    stem == "winquake" || stem == "glquake" || stem == "qwcl" || stem == "glqwcl" || stem.starts_with("quake")
}

/// Classify an MZ binary: a PE signature at the header offset reads as PE.
fn mz_format(path: &str, header: &[u8]) -> BinaryFormat {
    if header.len() < 64 {
        return BinaryFormat::DosMz;
    }
    let offset = u32::from_le_bytes([header[0x3c], header[0x3d], header[0x3e], header[0x3f]]) as u64;
    let mut signature = [0u8; 4];
    let is_pe = (|| -> std::io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut signature)?;
        Ok(())
    })()
    .is_ok()
        && signature == [0x50, 0x45, 0, 0];
    if is_pe {
        BinaryFormat::Pe
    } else {
        BinaryFormat::DosMz
    }
}

/// Provenance of an observed binary path.
fn binary_provenance(path: &str) -> BinaryProvenance {
    if let Some(title) = steam::steam_title_for_path(path) {
        return binary_steam_provenance(path, &title);
    }
    if path.starts_with(&format!("{}/", source_root())) {
        BinaryProvenance::SourceTree
    } else {
        BinaryProvenance::SuppliedCorpus
    }
}

fn binary_steam_provenance(path: &str, title: &SteamTitle) -> BinaryProvenance {
    BinaryProvenance::SteamInstallation {
        installation_path: title.path.clone(),
        family: title.family,
        edition: if path.starts_with(&format!("{}/rerelease/", title.path)) {
            SteamEdition::Rerelease
        } else {
            SteamEdition::Classic
        },
    }
}

/// Inspect one regular file as a binary candidate; non-binaries are skipped.
fn inspect_binary(
    path: &str,
    name: &str,
    project_root: &str,
    binaries: &mut Vec<BinaryObservation>,
) -> Result<(), ToolsError> {
    let in_source_tree = path.starts_with(&format!("{}/", source_root()));
    if !in_source_tree && !steam::is_corpus_binary_candidate(name) {
        return Ok(());
    }
    let details = std::fs::metadata(path).map_err(|error| ToolsError::io(format!("stating {path}"), error))?;
    let executable_permission = has_exec_bit(&details);
    if !executable_permission && !steam::is_corpus_binary_candidate(name) {
        return Ok(());
    }
    let header = read_prefix(path, 64)?;
    let format = if header.starts_with(&[0x7f, 0x45, 0x4c, 0x46]) {
        Some(BinaryFormat::Elf)
    } else if header.starts_with(&[0x4d, 0x5a]) {
        Some(mz_format(path, &header))
    } else {
        None
    };
    let Some(format) = format else {
        return Ok(());
    };
    let purpose = if name == "q1rets" || name == "quake3-ts" {
        BinaryPurpose::TypescriptDonorExecutable
    } else if path.starts_with(&format!("{}/", source_root())) {
        BinaryPurpose::SourceTreeBinary
    } else if retail_engine_name(name) {
        if matches!(format, BinaryFormat::DosMz) {
            BinaryPurpose::RetailDosEngine
        } else {
            BinaryPurpose::RetailWindowsEngine
        }
    } else {
        BinaryPurpose::OtherCorpusBinary
    };
    binaries.push(BinaryObservation {
        identity: identify_file(path)?,
        format,
        purpose,
        executable_permission,
        provenance: binary_provenance(path),
        header: observe_command(
            &[String::from("file"), String::from("--brief"), path.to_owned()],
            project_root,
            None,
        )?,
        execution_reason: if matches!(purpose, BinaryPurpose::TypescriptDonorExecutable) {
            "A compiled TypeScript donor is not an independent original-engine reference."
        } else {
            "Discovery establishes file presence and identity only; execution needs a reviewed workload and isolated output/display."
        }
        .to_owned(),
    });
    Ok(())
}

/// Walk one root without following directory symlinks; entries visit in name order.
fn visit_directory(
    directory: &str,
    project_root: &str,
    binaries: &mut Vec<BinaryObservation>,
) -> Result<(), ToolsError> {
    let mut entries = Vec::new();
    let listing =
        std::fs::read_dir(directory).map_err(|error| ToolsError::io(format!("listing {directory}"), error))?;
    for entry in listing {
        let entry = entry.map_err(|error| ToolsError::io(format!("listing {directory}"), error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
        entries.push((entry.file_name().to_string_lossy().into_owned(), file_type));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, file_type) in entries {
        if name == ".git" || name == "node_modules" {
            continue;
        }
        let path = format!("{directory}/{name}");
        if file_type.is_dir() {
            visit_directory(&path, project_root, binaries)?;
        } else if file_type.is_file() {
            inspect_binary(&path, &name, project_root, binaries)?;
        }
    }
    Ok(())
}

/// Discover binaries under each root; per-root failures become error strings.
fn discover_binaries(roots: &[String], project_root: &str) -> (Vec<BinaryObservation>, Vec<String>) {
    let mut binaries = Vec::new();
    let mut errors = Vec::new();
    for root in roots {
        if let Err(error) = visit_directory(root, project_root, &mut binaries) {
            errors.push(format!("{root}: {error}"));
        }
    }
    (binaries, errors)
}

/// Whether a DRM entry names a connector (`/^card\d+-/`).
fn drm_connector(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("card") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && rest.as_bytes().get(digits) == Some(&b'-')
}

/// Read the fixed system files plus thermal and DRM connector entries.
fn system_files() -> Vec<ReadObservation> {
    let mut paths = vec![
        "/etc/os-release".to_owned(),
        "/proc/sys/kernel/osrelease".to_owned(),
        "/proc/cpuinfo".to_owned(),
        "/proc/meminfo".to_owned(),
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor".to_owned(),
        "/sys/devices/system/cpu/intel_pstate/status".to_owned(),
        "/sys/firmware/acpi/platform_profile".to_owned(),
    ];
    for root in ["/sys/class/thermal", "/sys/class/drm"] {
        match std::fs::read_dir(root) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with("thermal_zone") {
                        paths.push(format!("{root}/{name}/type"));
                        paths.push(format!("{root}/{name}/temp"));
                    } else if drm_connector(&name) {
                        paths.push(format!("{root}/{name}/status"));
                        paths.push(format!("{root}/{name}/modes"));
                    }
                }
            }
            Err(_) => paths.push(root.to_owned()),
        }
    }
    paths.iter().map(|path| ReadObservation::read_file(path)).collect()
}

/// Library names selected from `ldconfig -p` output (donor regex alternation).
const LDCONFIG_LIBRARIES: [&str; 13] = [
    "SDL2-2.0",
    "GL",
    "GLX",
    "GLX_nvidia",
    "OpenGL",
    "EGL",
    "vorbisfile",
    "vorbis",
    "ogg",
    "freetype",
    "c",
    "m",
    "stdc++",
];

/// Resolve path for a selected `ldconfig` line, if the line selects a library.
fn ldconfig_library_path(line: &str) -> Option<String> {
    if line.len() == line.trim_start().len() {
        return None;
    }
    let rest = line.trim_start().strip_prefix("lib")?;
    let selected = LDCONFIG_LIBRARIES
        .iter()
        .any(|name| rest.starts_with(name) && rest[name.len()..].starts_with(".so"));
    if !selected {
        return None;
    }
    line.split(" => ").nth(1).map(|path| path.trim().to_owned())
}

/// Identify the selected libraries from successful `ldconfig` probes.
fn identify_available_libraries(tools: &[ToolObservation]) -> Result<Vec<FileIdentity>, ToolsError> {
    let found = tools.iter().find(|tool| {
        matches!(
            tool,
            ToolObservation::Available { name, .. } | ToolObservation::Unavailable { name }
                if name == "ldconfig"
        )
    });
    let Some(ToolObservation::Available { observations, .. }) = found else {
        return Ok(Vec::new());
    };
    let mut paths = BTreeSet::new();
    for observation in observations {
        if !matches!(observation.outcome, CommandOutcome::Exited { exit_code: 0 }) {
            continue;
        }
        for line in observation.stdout.split('\n') {
            if let Some(path) = ldconfig_library_path(line) {
                let resolved =
                    std::fs::canonicalize(&path).map_err(|error| ToolsError::io(format!("resolving {path}"), error))?;
                paths.insert(resolved.to_string_lossy().into_owned());
            }
        }
    }
    paths.iter().map(|path| identify_file(path)).collect()
}

/// Observe the donor's fixed tool list.
fn observe_tools(project_root: &str) -> Result<Vec<ToolObservation>, ToolsError> {
    let specs: Vec<(&str, Vec<&[&str]>)> = vec![
        ("bun", vec![&["--version"]]),
        ("git", vec![&["--version"]]),
        ("file", vec![&["--version"]]),
        ("lscpu", vec![&[]]),
        ("ldconfig", vec![&["-p"]]),
        (
            "nvidia-smi",
            vec![&[
                "--query-gpu=name,driver_version,memory.total,pstate,temperature.gpu,power.draw,power.limit",
                "--format=csv,noheader",
            ]],
        ),
        ("Xvfb", vec![]),
        ("xvfb-run", vec![]),
        ("glxinfo", vec![]),
        ("eglinfo", vec![]),
        ("vulkaninfo", vec![]),
        ("wine", vec![&["--version"]]),
        ("wine64", vec![&["--version"]]),
        ("quake", vec![]),
        ("quake2", vec![]),
        ("quake3", vec![]),
        ("quakespasm", vec![]),
        ("vkquake", vec![]),
        ("ioquake3", vec![]),
        ("q2pro", vec![]),
        ("yamagi-quake2", vec![]),
        ("q2ded", vec![]),
        ("q3ded", vec![]),
    ];
    let mut tools = Vec::with_capacity(specs.len());
    for (name, arguments) in &specs {
        tools.push(observe_tool(name, arguments, project_root)?);
    }
    Ok(tools)
}

/// Capture-program sources: the Rust files that replaced the donor scripts.
fn capture_program_files() -> [PathBuf; 3] {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference");
    [
        directory.join("environment.rs"),
        directory.join("schema.rs"),
        directory.join("steam.rs"),
    ]
}

/// Capture limits recorded on every manifest (donor verbatim).
fn capture_limits() -> Vec<String> {
    [
        "This capture inventories availability and hardware. It does not execute gameplay or establish rendering, wire, image-tolerance, or performance acceptance.",
        "Git commits and tree IDs identify tracked source content; all changed and untracked paths exposed by Git are separately SHA-256 identified. Ignored source files are not included.",
        "CPU frequencies, memory, thermal and GPU power readings are capture-time observations. They do not establish controlled benchmark conditions.",
        "availableLibraryFiles identifies selected ldconfig candidates, including installed architectures. These are available libraries, not proof of which libraries a future reference process actually loads.",
        "DRM connector modes are advertised modes; no desktop display was queried or changed and no active reference viewport is claimed.",
        "Binary discovery is limited to the recorded roots and PATH names. An undiscovered executable elsewhere may exist.",
        "Steam installation metadata records family/edition associations, not vendor authenticity. Runtime availability and wine --version success do not establish a working game launch.",
        "Retail data stays external. This environment manifest contains executable identities; each behavioral capture must separately identify its actual archive entries and selected mount order.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Capture the full reference environment.
pub fn capture_environment() -> Result<ReferenceEnvironment, ToolsError> {
    let project_root = quake_typescript_root();
    let project_root_text = project_root.to_string_lossy().into_owned();
    let steam: SteamObservation = steam::observe_steam(
        &|path| identify_file(path),
        &|command, cwd, timeout| observe_command(command, cwd, timeout),
        &project_root_text,
    );
    let mut roots = vec![source_root(), corpus_root()];
    for title in steam
        .titles
        .iter()
        .filter(|title| matches!(title.availability, TitleAvailability::Present))
    {
        roots.push(title.path.clone());
    }
    let mut sources = Vec::new();
    for spec in source_census::repositories() {
        sources.push(identify_source(&project_root, &spec)?);
    }
    let tools = observe_tools(&project_root_text)?;
    let system = system_files();
    let (binaries, errors) = discover_binaries(&roots, &project_root_text);
    let available_library_files = identify_available_libraries(&tools)?;
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    let runtime = identify_file(&exe.to_string_lossy())?;
    let manifest_path = project_root.join("verification/source-manifest.json");
    let manifest = if manifest_path.exists() {
        Some(identify_file(&manifest_path.to_string_lossy())?)
    } else {
        None
    };
    let mut capture_program = Vec::with_capacity(3);
    for path in capture_program_files() {
        capture_program.push(identify_file(&path.to_string_lossy())?);
    }
    let definition_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/inventory/source_census.rs")
        .to_string_lossy()
        .into_owned();
    Ok(ReferenceEnvironment {
        command: std::env::args().collect(),
        captured_at: now_iso(),
        runtime,
        platform: node_platform().to_owned(),
        architecture: node_arch().to_owned(),
        runtime_version: format!("qa-tools/{}", env!("CARGO_PKG_VERSION")),
        capture_program,
        source_census: CensusReference { definition: identify_file(&definition_path)?, manifest },
        sources,
        tools,
        available_library_files,
        system,
        binaries,
        steam,
        discovery: Discovery {
            roots,
            method: "In source trees, inspect regular executable-permission files and .exe files for ELF/MZ headers. In qfiles and the three selected Steam titles, inspect only .exe/.dll/.so names and known donor executables, excluding account/config/data files before opening them. Skip .git and node_modules and do not follow directory symlinks. Runtime inventory hashes a fixed set of Proton launch files and reads only runtime version metadata; unrelated Steam titles and account state are excluded. PATH probes are explicitly listed in tools.".to_owned(),
            errors,
        },
        limits: capture_limits(),
    })
}

/// Run the environment capture (donor `main`): write
/// `verification/reference-environment.json` and print the summary.
pub fn run(args: &[String]) -> Result<(), ToolsError> {
    if !args.is_empty() {
        return Err(ToolsError::invalid(
            "Usage: reference environment capture takes no arguments",
        ));
    }
    let environment = capture_environment()?;
    let destination = quake_typescript_root().join("verification/reference-environment.json");
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    }
    fsutil::write_text(&destination, &format!("{}\n", environment.to_json().render_pretty()))?;
    println!("{}", destination.display());
    println!(
        "{} source trees; {} candidate binaries; {} discovery errors",
        environment.sources.len(),
        environment.binaries.len(),
        environment.discovery.errors.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    // Covers tools/reference/environment.test.ts: all 5 donor cases (corpus
    // discovery, Steam titles, file identity, command capture, command timeout).
    use crate::reference::schema::QuakeFamily;

    use super::*;

    #[test]
    fn corpus_discovery_excludes_account_and_profile_files() {
        for name in [
            "q3key",
            "config.cfg",
            "q3config.cfg",
            "localconfig.vdf",
            "loginusers.vdf",
            "appmanifest_2200.acf",
            "pak0.pak",
        ] {
            assert!(!steam::is_corpus_binary_candidate(name), "{name}");
        }
        for name in [
            "Winquake.exe",
            "quake3.exe",
            "game_x64.dll",
            "gamei386.so.glibc",
            "game.so.1",
            "quake3-ts",
        ] {
            assert!(steam::is_corpus_binary_candidate(name), "{name}");
        }
    }

    #[test]
    fn steam_title_classification_needs_exact_directories() {
        let common = steam::steam_common_path().to_string_lossy().into_owned();
        for (title, family) in [
            ("Quake", QuakeFamily::Q1),
            ("Quake 2", QuakeFamily::Q2),
            ("Quake 3 Arena", QuakeFamily::Q3),
        ] {
            let found = steam::steam_title_for_path(&format!("{common}/{title}/game.exe"));
            assert!(found.is_some_and(|title| title.family == family));
        }
        assert!(steam::steam_title_for_path(&format!("{common}/Quake Live/quakelive_steam.exe")).is_none());
        assert!(steam::steam_title_for_path(&format!("{common}/Quake-unrelated/game.exe")).is_none());
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        fsutil::make_temp_dir(&std::env::temp_dir(), prefix).expect("temp dir")
    }

    #[test]
    fn file_identity_hashes_bytes_and_resolves_symlinks() {
        let directory = temp_dir("quake-reference-hash-");
        let canonical = std::fs::canonicalize(&directory)
            .expect("canonical temp dir")
            .to_string_lossy()
            .into_owned();
        let path = format!("{canonical}/input.txt");
        let link = format!("{canonical}/alias.txt");
        fsutil::write_text(Path::new(&path), "abc").expect("write input");
        fsutil::create_symlink(Path::new(&path), Path::new(&link)).expect("symlink");
        let identity = identify_file(&link).expect("identify");
        assert_eq!(identity.path, path);
        assert_eq!(identity.size, 3);
        assert_eq!(
            identity.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn command_capture_preserves_streams_identity_and_exit_status() {
        let directory = temp_dir("quake-reference-command-");
        let cwd = directory.to_string_lossy().into_owned();
        let command = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf 'output\\n'; printf 'diagnostic\\n' >&2; exit 7".to_owned(),
        ];
        let result = observe_command(&command, &cwd, None).expect("observe");
        assert_eq!(result.command, command);
        assert!(matches!(result.outcome, CommandOutcome::Exited { exit_code: 7 }));
        assert_eq!(result.stdout, "output\n");
        assert_eq!(result.stderr, "diagnostic\n");
        assert_eq!(result.cwd, cwd);
        assert_eq!(result.environment, vec![("LC_ALL".to_owned(), "C".to_owned())]);
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn command_timeout_terminates_and_preserves_partial_output() {
        let directory = temp_dir("quake-reference-timeout-");
        let cwd = directory.to_string_lossy().into_owned();
        let command = vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf 'started\\n'; sleep 30".to_owned(),
        ];
        let result = observe_command(&command, &cwd, Some(250)).expect("observe");
        assert!(matches!(result.outcome, CommandOutcome::TimedOut { timeout_ms: 250 }));
        assert!(result.stdout.contains("started\n"), "stdout: {:?}", result.stdout);
        assert!(result.duration_ms < 5_000.0, "duration: {}", result.duration_ms);
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn classifies_retail_engine_names() {
        for name in [
            "winquake.exe",
            "GLQuake.EXE",
            "qwcl.exe",
            "glqwcl.exe",
            "quake2.exe",
            "quake.exe",
            "Quake3.exe",
        ] {
            assert!(retail_engine_name(name), "{name}");
        }
        for name in [
            "quake",
            "quake.txt",
            "xquake.exe",
            "winquake.com",
            "qwcl",
            "quake3-ts",
            "game.exe",
        ] {
            assert!(!retail_engine_name(name), "{name}");
        }
    }

    #[test]
    fn parses_ldconfig_library_paths() {
        let line = "\tlibGL.so.1 (libc6,x86-64) => /lib/x86_64-linux-gnu/libGL.so.1";
        assert_eq!(
            ldconfig_library_path(line).as_deref(),
            Some("/lib/x86_64-linux-gnu/libGL.so.1")
        );
        let line = "\tlibc.so.6 (libc6,x86-64) => /lib/x86_64-linux-gnu/libc.so.6";
        assert_eq!(
            ldconfig_library_path(line).as_deref(),
            Some("/lib/x86_64-linux-gnu/libc.so.6")
        );
        assert!(ldconfig_library_path("libGL.so.1 => /lib/libGL.so.1").is_none());
        assert!(ldconfig_library_path("\tlibz.so.1 (libc6,x86-64) => /lib/libz.so.1").is_none());
        assert!(ldconfig_library_path("\tlibGLU.so.1 (libc6,x86-64) => /lib/libGLU.so.1").is_none());
    }

    #[test]
    fn classifies_mz_signatures() {
        let directory = temp_dir("quake-reference-mz-");
        let pe_path = directory.join("game.exe").to_string_lossy().into_owned();
        let mut header = vec![0u8; 64];
        header[0] = 0x4d;
        header[1] = 0x5a;
        header[0x3c] = 0x40;
        let mut bytes = header.clone();
        bytes.extend_from_slice(&[0x50, 0x45, 0, 0]);
        fsutil::write_bytes(Path::new(&pe_path), &bytes).expect("write pe");
        assert!(matches!(mz_format(&pe_path, &header), BinaryFormat::Pe));
        let dos_path = directory.join("old.exe").to_string_lossy().into_owned();
        fsutil::write_bytes(Path::new(&dos_path), &header).expect("write dos");
        assert!(matches!(mz_format(&dos_path, &header), BinaryFormat::DosMz));
        assert!(matches!(mz_format(&dos_path, &[0x4d, 0x5a]), BinaryFormat::DosMz));
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn rejects_non_file_identity() {
        let directory = temp_dir("quake-reference-dir-");
        let text = directory.to_string_lossy().into_owned();
        assert!(identify_file(&text).is_err());
        fsutil::remove_forced(&directory);
    }
}
