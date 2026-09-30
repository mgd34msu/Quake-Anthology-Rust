//! Workspace builds (donor `tools/build.ts`).
//!
//! Snapshots the TypeScript workspace, runs the `typecheck` and `policy`
//! gates inside the snapshot, compiles entries with the `bun build --compile`
//! toolchain, and records evidence with input-stability checks.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::ToolsError;
use crate::fsutil::{make_temp_dir, random_uuid, read_text};
use crate::json::{parse_json, Json};
use crate::process::which;
use crate::time::now_iso;
use crate::verify::hash::hash_bytes;
use crate::verify::snapshot::WorkspaceSnapshot;

/// Build kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    /// Runtime build.
    Runtime,
    /// Tools build.
    Tools,
}

impl BuildKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::Tools => "tools",
        }
    }
}

/// Parse build arguments.
pub fn parse_build_kind(args: &[String]) -> Result<BuildKind, ToolsError> {
    if args.is_empty() {
        Ok(BuildKind::Runtime)
    } else if args.len() == 1 && args[0] == "--tools" {
        Ok(BuildKind::Tools)
    } else {
        Err(ToolsError::invalid("Usage: qa-build [--tools]"))
    }
}

struct BuildEntry {
    source: &'static str,
    executable: &'static str,
}

struct BuiltEntry {
    source: &'static str,
    executable: &'static str,
    sha256: String,
    bytes: i64,
}

impl BuiltEntry {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("source".to_owned(), Json::string(self.source)),
            ("executable".to_owned(), Json::string(self.executable)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("bytes".to_owned(), Json::int(self.bytes)),
        ])
    }
}

/// Build evidence.
pub struct BuildEvidence {
    /// Build kind.
    pub kind: BuildKind,
    /// Evidence JSON.
    pub evidence: Json,
    /// Published directory.
    pub directory: PathBuf,
}

fn bun() -> Result<PathBuf, ToolsError> {
    which("bun").ok_or_else(|| ToolsError::invalid("Builds require bun on PATH"))
}

fn run_gate(snapshot_path: &Path, name: &str) -> Result<(), ToolsError> {
    let status = Command::new(bun()?)
        .args(["run", name])
        .current_dir(snapshot_path)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| ToolsError::io(format!("running build gate {name}"), error))?;
    if !status.success() {
        return Err(ToolsError::invalid(format!("Build snapshot {name} failed with exit {}.", status.code().unwrap_or(-1))));
    }
    Ok(())
}

fn bun_info() -> Result<(String, String, String), ToolsError> {
    let executable = bun()?;
    let version_output = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|error| ToolsError::io("querying bun version", error))?;
    let revision_output = Command::new(&executable)
        .arg("--revision")
        .output()
        .map_err(|error| ToolsError::io("querying bun revision", error))?;
    Ok((
        String::from_utf8_lossy(&version_output.stdout).trim().to_owned(),
        String::from_utf8_lossy(&revision_output.stdout).trim().to_owned(),
        executable.to_string_lossy().into_owned(),
    ))
}

fn typescript_version(snapshot_path: &Path) -> Result<String, ToolsError> {
    let package = parse_json(&read_text(&snapshot_path.join("node_modules/typescript/package.json"))?)?;
    match package.get("version").and_then(Json::as_str) {
        Some(version) => Ok(version.to_owned()),
        None => Err(ToolsError::invalid("Installed TypeScript compiler has no version")),
    }
}

fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Build the workspace at `directory`.
pub fn build_workspace(directory: &Path, kind: BuildKind) -> Result<BuildEvidence, ToolsError> {
    if std::env::consts::OS != "linux" {
        return Err(ToolsError::invalid("The first build target is Linux. Run this build on Linux."));
    }
    let workspace = crate::verify::snapshot::resolve(directory);
    let entries: &[BuildEntry] = match kind {
        BuildKind::Runtime => &[BuildEntry {
            source: "src/main.ts",
            executable: "quake-typescript",
        }],
        BuildKind::Tools => &[
            BuildEntry {
                source: "docs/validate-plan.ts",
                executable: "validate-plan",
            },
            BuildEntry {
                source: "tools/check-policy.ts",
                executable: "check-policy",
            },
            BuildEntry {
                source: "tools/source-camera.ts",
                executable: "source-camera",
            },
            BuildEntry {
                source: "tools/navigation/aas.ts",
                executable: "aas",
            },
        ],
    };
    for entry in entries {
        if !workspace.join(entry.source).is_file() {
            if kind == BuildKind::Runtime {
                return Err(ToolsError::invalid(
                    "Runtime build unavailable: src/main.ts is not implemented. W73 owns the real application entry. Use build:tools to compile existing tooling.",
                ));
            }
            return Err(ToolsError::invalid(format!("Build entry is missing: {}", entry.source)));
        }
    }
    let snapshot = WorkspaceSnapshot::capture(&workspace, &[])?;
    let snapshot_path = snapshot.materialize(&workspace.join(".artifacts/snapshots"), "build-")?;
    run_gate(&snapshot_path, "typecheck")?;
    run_gate(&snapshot_path, "policy")?;
    let dist = workspace.join("dist");
    std::fs::create_dir_all(&dist).map_err(|error| ToolsError::io(format!("creating {}", dist.display()), error))?;
    let candidate = make_temp_dir(&dist, &format!(".candidate-{}-", kind.as_str()))?;
    let result = build_entries(&workspace, &snapshot_path, &candidate, kind, entries, &snapshot.manifest.sha256);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&candidate);
    }
    let (_, final_directory, evidence) = result?;
    let metadata = format!("{}\n", evidence.render_pretty());
    let pointer = dist.join(format!(".{}-{}-{}.json", kind.as_str(), crate::time::now_millis(), random_uuid()?));
    std::fs::write(&pointer, &metadata).map_err(|error| ToolsError::io(format!("writing {}", pointer.display()), error))?;
    std::fs::rename(&pointer, dist.join(format!("{}.json", kind.as_str())))
        .map_err(|error| ToolsError::io(format!("publishing {}.json", kind.as_str()), error))?;
    println!("Built {} from {}: {}", kind.as_str(), snapshot.manifest.sha256, final_directory.display());
    Ok(BuildEvidence {
        kind,
        evidence,
        directory: final_directory,
    })
}

#[allow(clippy::too_many_lines)]
fn build_entries(
    workspace: &Path,
    snapshot_path: &Path,
    candidate: &Path,
    kind: BuildKind,
    entries: &[BuildEntry],
    source_sha: &str,
) -> Result<(Vec<BuiltEntry>, PathBuf, Json), ToolsError> {
    let mut built_entries = Vec::new();
    for entry in entries {
        let mut build_args = vec!["build".to_owned(), snapshot_path.join(entry.source).to_string_lossy().into_owned()];
        if kind == BuildKind::Runtime {
            build_args.push(snapshot_path.join("src/render/worker-entry.ts").to_string_lossy().into_owned());
        }
        let outfile = candidate.join(entry.executable);
        build_args.extend([
            "--compile".to_owned(),
            "--outfile".to_owned(),
            outfile.to_string_lossy().into_owned(),
            "--target".to_owned(),
            "bun".to_owned(),
            "--sourcemap=inline".to_owned(),
            "--loader:.png:file".to_owned(),
        ]);
        let output = Command::new(bun()?)
            .args(&build_args)
            .current_dir(snapshot_path)
            .output()
            .map_err(|error| ToolsError::io(format!("compiling {}", entry.source), error))?;
        if !output.status.success() {
            let logs = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            return Err(ToolsError::invalid(logs.trim()));
        }
        let bytes = std::fs::read(&outfile).map_err(|error| ToolsError::io(format!("reading {}", outfile.display()), error))?;
        built_entries.push(BuiltEntry {
            source: entry.source,
            executable: entry.executable,
            sha256: hash_bytes(&bytes),
            bytes: bytes.len() as i64,
        });
    }
    let current = WorkspaceSnapshot::capture(workspace, &[])?;
    let copied = WorkspaceSnapshot::capture(snapshot_path, &[])?;
    if current.manifest.sha256 != source_sha || copied.manifest.sha256 != source_sha {
        return Err(ToolsError::invalid(format!(
            "Build inputs changed. Existing published artifacts were preserved. Snapshot: {}",
            snapshot_path.display()
        )));
    }
    let suffix = candidate.file_name().and_then(|name| name.to_string_lossy().rsplit('-').next().map(str::to_owned)).unwrap_or_default();
    let final_directory = workspace.join("dist").join(format!("{}-{}-{suffix}", kind.as_str(), &source_sha[..16.min(source_sha.len())]));
    let (bun_version, bun_revision, bun_executable) = bun_info()?;
    let typescript = typescript_version(snapshot_path)?;
    let evidence = Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("kind".to_owned(), Json::string(kind.as_str())),
        ("builtAt".to_owned(), Json::string(now_iso())),
        ("directory".to_owned(), Json::string(final_directory.to_string_lossy())),
        ("sourceSha256".to_owned(), Json::string(source_sha)),
        ("snapshotPath".to_owned(), Json::string(snapshot_path.to_string_lossy())),
        (
            "bun".to_owned(),
            Json::object(vec![
                ("version".to_owned(), Json::string(&bun_version)),
                ("revision".to_owned(), Json::string(&bun_revision)),
                ("executable".to_owned(), Json::string(&bun_executable)),
            ]),
        ),
        ("typescript".to_owned(), Json::string(&typescript)),
        ("platform".to_owned(), Json::string(std::env::consts::OS)),
        ("architecture".to_owned(), Json::string(node_arch())),
        (
            "gates".to_owned(),
            Json::array(vec![Json::string("typecheck"), Json::string("policy"), Json::string("input-stability")]),
        ),
        ("entries".to_owned(), Json::array(built_entries.iter().map(BuiltEntry::to_json).collect())),
    ]);
    let metadata = format!("{}\n", evidence.render_pretty());
    std::fs::write(candidate.join("build.json"), &metadata).map_err(|error| ToolsError::io("writing build.json", error))?;
    for entry in &built_entries {
        let provenance = Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("sourceSha256".to_owned(), Json::string(source_sha)),
            ("executableSha256".to_owned(), Json::string(&entry.sha256)),
            ("source".to_owned(), Json::string(entry.source)),
            (
                "bun".to_owned(),
                Json::object(vec![
                    ("version".to_owned(), Json::string(&bun_version)),
                    ("revision".to_owned(), Json::string(&bun_revision)),
                    ("executable".to_owned(), Json::string(&bun_executable)),
                ]),
            ),
            ("typescript".to_owned(), Json::string(&typescript)),
        ]);
        std::fs::write(candidate.join(format!("{}.build.json", entry.executable)), format!("{}\n", provenance.render_pretty()))
            .map_err(|error| ToolsError::io("writing entry provenance", error))?;
    }
    std::fs::rename(candidate, &final_directory).map_err(|error| ToolsError::io(format!("publishing {}", final_directory.display()), error))?;
    Ok((built_entries, final_directory, evidence))
}
