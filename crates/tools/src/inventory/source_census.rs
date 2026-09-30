//! Source census (donor `tools/inventory/source-census.ts`).
//!
//! Inventories pinned repositories: tracked files with hashes, submodule
//! pins, per-file TypeScript functions, and a source-set fingerprint.
//!
//! Scope note: the donor pins thirteen repositories, ten of which live
//! under a sibling source tree that is out of scope for this port. Those
//! ten entries (`q1-original`, `q1-rerelease-qc`, `q1-ironwail`,
//! `q2-lmctf-original`, `q2-original`, `q2-rerelease-game`, `q2-repro`,
//! `q2-proto`, `q2-repro-game`, `q3-original`) are omitted here; the three
//! TypeScript sibling checkouts are inventoried. Capture logic stays fully
//! general over whatever specs are listed.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::ToolsError;
use crate::fsutil::{lexical_absolute, make_temp_dir, posix_relative};
use crate::inventory::ts_scan::{self, DeclInfo};
use crate::js::{compare_text, head_utf16, line_starts, Utf16Map};
use crate::json::Json;
use crate::time::now_iso;
use crate::verify::hash::{hash_bytes, hash_str};

/// Role of a censused repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryRole {
    /// Original reference source.
    OriginalReference,
    /// TypeScript implementation candidate.
    ImplementationCandidate,
}

impl RepositoryRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::OriginalReference => "original-reference",
            Self::ImplementationCandidate => "implementation-candidate",
        }
    }
}

/// A pinned repository specification.
#[derive(Debug, Clone)]
pub struct RepositorySpec {
    /// Stable repository identifier.
    pub id: String,
    /// Repository role.
    pub role: RepositoryRole,
    /// Project-relative path.
    pub path: String,
    /// Reviewed revision the checkout must match.
    pub expected_revision: String,
}

/// Pinned repository specifications (see module docs for the scope cut).
#[must_use]
pub fn repositories() -> Vec<RepositorySpec> {
    [
        ("q1-ts", RepositoryRole::ImplementationCandidate, "../quake-1-re-ts", "6bc6a8bf29b66981e3b6ce7251ebbc6413120270"),
        ("q2-ts", RepositoryRole::ImplementationCandidate, "../quake-2-re-ts", "0d73750cbe5683c7411934d0a5d4eb5acd4da676"),
        ("q3-ts", RepositoryRole::ImplementationCandidate, "../quake-3-ts", "8453c49824eb7a5ed5aee452f74e19336965d1f8"),
    ]
    .into_iter()
    .map(|(id, role, path, expected_revision)| RepositorySpec {
        id: id.to_owned(),
        role,
        path: path.to_owned(),
        expected_revision: expected_revision.to_owned(),
    })
    .collect()
}

/// Run git in `root`, returning lossy standard output.
pub fn git(root: &Path, args: &[&str]) -> Result<String, ToolsError> {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().map_err(|error| {
        ToolsError::command(format!("spawning git in {}: {error}", root.display()))
    })?;
    if !output.status.success() {
        return Err(ToolsError::command(format!(
            "Git failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Validate a repository-relative path.
pub fn safe_relative_path(path: &str) -> Result<String, ToolsError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|part| part == ".." || part == "." || part.is_empty())
    {
        return Err(ToolsError::invalid(format!("Invalid repository-relative path: {path}")));
    }
    Ok(path.to_owned())
}

/// Classification of a censused file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// TypeScript declaration file.
    Declaration,
    /// Test file.
    Test,
    /// Runtime source under `src/`.
    Runtime,
    /// Tooling source.
    Tool,
    /// Original reference code.
    ReferenceCode,
    /// Documentation.
    Document,
    /// Configuration.
    Configuration,
    /// Anything else.
    Other,
}

impl FileKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::Test => "test",
            Self::Runtime => "runtime",
            Self::Tool => "tool",
            Self::ReferenceCode => "reference-code",
            Self::Document => "document",
            Self::Configuration => "configuration",
            Self::Other => "other",
        }
    }
}

fn is_test_path(path: &str) -> bool {
    for needle in ["__tests__", "tests", "test"] {
        let mut from = 0;
        while let Some(found) = path[from..].find(needle) {
            let index = from + found;
            let before = index == 0 || path.as_bytes()[index - 1] == b'/';
            let after = path.as_bytes().get(index + needle.len()).is_some_and(|next| *next == b'/' || *next == b'.');
            if before && after {
                return true;
            }
            from = index + 1;
        }
    }
    for needle in [".test.", ".spec."] {
        if let Some(found) = path.rfind(needle) {
            let rest = &path[found + needle.len()..];
            if !rest.is_empty() && !rest.contains('.') {
                return true;
            }
        }
    }
    false
}

fn extension(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rfind('.') {
        Some(dot) if dot > 0 => &base[dot..],
        _ => "",
    }
}

/// Classify a file by role and repository-relative path.
#[must_use]
pub fn classify_file(role: RepositoryRole, path: &str) -> FileKind {
    if path.ends_with(".d.ts") {
        return FileKind::Declaration;
    }
    if path.ends_with(".ts") || path.ends_with(".tsx") {
        if is_test_path(path) {
            return FileKind::Test;
        }
        return if path.starts_with("src/") { FileKind::Runtime } else { FileKind::Tool };
    }
    if role == RepositoryRole::OriginalReference {
        let ext = extension(path).to_lowercase();
        if matches!(ext.as_str(), ".c" | ".h" | ".cc" | ".cpp" | ".cxx" | ".hpp" | ".qc" | ".asm" | ".s") {
            return FileKind::ReferenceCode;
        }
    }
    {
        let ext = extension(path).to_lowercase();
        if matches!(ext.as_str(), ".md" | ".rst" | ".txt") {
            return FileKind::Document;
        }
    }
    for needle in ["LICENSE", "COPYING", "NOTICE", "README"] {
        let mut from = 0;
        while let Some(found) = path[from..].find(needle) {
            let index = from + found;
            let before = index == 0 || path.as_bytes()[index - 1] == b'/';
            let after = path.as_bytes().get(index + needle.len()).is_none_or(|next| *next == b'.');
            if before && after {
                return FileKind::Document;
            }
            from = index + 1;
        }
    }
    {
        let ext = extension(path).to_lowercase();
        if matches!(ext.as_str(), ".json" | ".jsonc" | ".yaml" | ".yml" | ".toml" | ".ini" | ".cfg" | ".lock") || path.starts_with('.') {
            return FileKind::Configuration;
        }
    }
    FileKind::Other
}

/// Function-like declaration kinds the census records.
pub const FUNCTION_KINDS: [&str; 7] = [
    "FunctionDeclaration",
    "FunctionExpression",
    "ArrowFunction",
    "MethodDeclaration",
    "GetAccessor",
    "SetAccessor",
    "Constructor",
];

/// A censused TypeScript function.
#[derive(Debug, Clone)]
pub struct CensusFunction {
    /// `{repository}:{path}#{startOffset}`.
    pub id: String,
    /// Inferred name.
    pub name: String,
    /// Syntax kind.
    pub kind: String,
    /// 1-based start line.
    pub line: usize,
    /// 1-based end line.
    pub end_line: usize,
    /// UTF-16 start offset.
    pub start_offset: usize,
    /// UTF-16 end offset.
    pub end_offset: usize,
    /// Whether the function has a body.
    pub has_body: bool,
}

impl CensusFunction {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("name".to_owned(), Json::string(&self.name)),
            ("kind".to_owned(), Json::string(&self.kind)),
            ("line".to_owned(), Json::uint(self.line as u64)),
            ("endLine".to_owned(), Json::uint(self.end_line as u64)),
            ("startOffset".to_owned(), Json::uint(self.start_offset as u64)),
            ("endOffset".to_owned(), Json::uint(self.end_offset as u64)),
            ("hasBody".to_owned(), Json::boolean(self.has_body)),
        ])
    }
}

fn line_of(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|start| *start <= offset).max(1)
}

/// Truncate a call-derived name the way `inferredName` does.
///
/// Only callee-derived names end with `" callback"`: identifiers cannot
/// contain spaces, and quoted/computed member names keep their quotes or
/// brackets. The callee head is cut to 120 UTF-16 code units.
fn census_name(name: &str) -> String {
    if let Some(callee) = name.strip_suffix(" callback") {
        format!("{} callback", head_utf16(callee, 120))
    } else {
        name.to_owned()
    }
}

/// Census functions from an already-scanned file.
#[must_use]
pub fn census_functions_from_scan(repository: &str, path: &str, source: &str, decls: &[DeclInfo]) -> Vec<CensusFunction> {
    let starts = line_starts(source);
    let utf16 = Utf16Map::new(source);
    let mut functions = Vec::new();
    for decl in decls {
        if !FUNCTION_KINDS.contains(&decl.kind) {
            continue;
        }
        let start_offset = utf16.to_utf16(decl.start);
        let end_offset = utf16.to_utf16(decl.end);
        let end_line = line_of(&starts, decl.start.max(decl.end.saturating_sub(1)));
        functions.push(CensusFunction {
            id: format!("{repository}:{path}#{start_offset}"),
            name: census_name(&decl.name),
            kind: decl.kind.to_owned(),
            line: line_of(&starts, decl.start),
            end_line,
            start_offset,
            end_offset,
            has_body: decl.body.is_some(),
        });
    }
    functions.sort_by(|left, right| left.start_offset.cmp(&right.start_offset));
    functions
}

/// Enumerate census functions in one TypeScript source file.
#[must_use]
pub fn enumerate_functions(repository: &str, path: &str, source: &str) -> Vec<CensusFunction> {
    let scanned = ts_scan::scan_declarations(source);
    census_functions_from_scan(repository, path, source, &scanned.declarations)
}

/// Tracking state of a censused file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTracking {
    /// Tracked by git.
    Tracked,
    /// Untracked but not ignored.
    Untracked,
}

impl FileTracking {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tracked => "tracked",
            Self::Untracked => "untracked",
        }
    }
}

/// A censused file.
#[derive(Debug, Clone)]
pub struct CensusFile {
    /// Repository-relative path.
    pub path: String,
    /// Tracking state.
    pub tracking: FileTracking,
    /// Classification.
    pub kind: FileKind,
    /// Byte size.
    pub bytes: u64,
    /// SHA-256 hex of the bytes.
    pub sha256: String,
    /// Newline-split line count.
    pub line_count: usize,
    /// Census functions (TypeScript only).
    pub functions: Vec<CensusFunction>,
}

impl CensusFile {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("tracking".to_owned(), Json::string(self.tracking.as_str())),
            ("kind".to_owned(), Json::string(self.kind.as_str())),
            ("bytes".to_owned(), Json::uint(self.bytes)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("lineCount".to_owned(), Json::uint(self.line_count as u64)),
            ("functions".to_owned(), Json::array(self.functions.iter().map(CensusFunction::to_json).collect())),
        ])
    }
}

/// A pinned submodule inside a censused repository.
#[derive(Debug, Clone)]
pub struct CensusSubmodule {
    /// Repository-relative path.
    pub path: String,
    /// Pinned revision.
    pub revision: String,
    /// Repository identifier providing the pin.
    pub repository: String,
}

impl CensusSubmodule {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("revision".to_owned(), Json::string(&self.revision)),
            ("repository".to_owned(), Json::string(&self.repository)),
        ])
    }
}

/// A captured repository.
#[derive(Debug, Clone)]
pub struct CapturedRepository {
    /// Specification.
    pub spec: RepositorySpec,
    /// Observed HEAD revision.
    pub revision: String,
    /// Source-set fingerprint.
    pub source_set_sha256: String,
    /// Pinned submodules.
    pub submodules: Vec<CensusSubmodule>,
    /// Censused files.
    pub files: Vec<CensusFile>,
}

impl CapturedRepository {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.spec.id)),
            ("role".to_owned(), Json::string(self.spec.role.as_str())),
            ("path".to_owned(), Json::string(&self.spec.path)),
            ("expectedRevision".to_owned(), Json::string(&self.spec.expected_revision)),
            ("revision".to_owned(), Json::string(&self.revision)),
            ("sourceSetSha256".to_owned(), Json::string(&self.source_set_sha256)),
            ("submodules".to_owned(), Json::array(self.submodules.iter().map(CensusSubmodule::to_json).collect())),
            ("files".to_owned(), Json::array(self.files.iter().map(CensusFile::to_json).collect())),
        ])
    }
}

fn census_clean(root: &Path, spec: &RepositorySpec) -> Result<String, ToolsError> {
    let revision = git(root, &["rev-parse", "HEAD"])?;
    let revision = revision.trim().to_owned();
    if revision != spec.expected_revision {
        return Err(ToolsError::invalid(format!("{}: HEAD {revision} differs from reviewed revision {}", spec.id, spec.expected_revision)));
    }
    if !git(root, &["status", "--porcelain=v1", "--untracked-files=no"])?.is_empty() {
        return Err(ToolsError::invalid(format!(
            "{}: tracked source files changed; review and repin before generating the census",
            spec.id
        )));
    }
    Ok(revision)
}

/// Capture one repository.
pub fn capture_repository(project_root: &Path, spec: &RepositorySpec) -> Result<CapturedRepository, ToolsError> {
    let specs = repositories();
    let repository_root = lexical_absolute(project_root, &spec.path);
    let revision = census_clean(&repository_root, spec)?;
    let mut tracked: std::collections::BTreeSet<String> = git(&repository_root, &["ls-files", "--cached", "-z"])?
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect();
    let mut submodules = Vec::new();
    for entry in git(&repository_root, &["ls-files", "--stage", "-z"])?.split('\0').filter(|entry| !entry.is_empty()) {
        if !entry.starts_with("160000 ") {
            continue;
        }
        let Some(tab) = entry.find('\t') else {
            return Err(ToolsError::invalid(format!("{}:{entry} needs an explicitly pinned submodule source", spec.id)));
        };
        let metadata: Vec<&str> = entry[..tab].split(' ').collect();
        let path = &entry[tab + 1..];
        let child = specs.iter().find(|candidate| {
            lexical_absolute(project_root, &candidate.path) == lexical_absolute(&repository_root, path)
                && metadata.get(1) == Some(&candidate.expected_revision.as_str())
        });
        let Some(child) = child else {
            return Err(ToolsError::invalid(format!("{}:{path} needs an explicitly pinned submodule source", spec.id)));
        };
        submodules.push(CensusSubmodule {
            path: path.to_owned(),
            revision: child.expected_revision.clone(),
            repository: child.id.clone(),
        });
        tracked.remove(path);
    }
    let untracked: Vec<String> =
        git(&repository_root, &["ls-files", "--others", "--exclude-standard", "-z"])?.split('\0').filter(|entry| !entry.is_empty()).map(str::to_owned).collect();
    let mut paths: Vec<String> = tracked.iter().cloned().collect();
    paths.extend(untracked.iter().cloned());
    paths.sort_by(|left, right| compare_text(left, right));
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        safe_relative_path(path)?;
        let absolute = lexical_absolute(&repository_root, path);
        let details = std::fs::symlink_metadata(&absolute)
            .map_err(|error| ToolsError::io(format!("statting {}", absolute.display()), error))?;
        if details.file_type().is_symlink() {
            let target = std::fs::read_link(&absolute)
                .map_err(|error| ToolsError::io(format!("reading link {}", absolute.display()), error))?;
            let target = target.to_string_lossy().into_owned();
            files.push(CensusFile {
                path: path.clone(),
                tracking: if tracked.contains(path) { FileTracking::Tracked } else { FileTracking::Untracked },
                kind: FileKind::Other,
                bytes: target.len() as u64,
                sha256: hash_str(&target),
                line_count: 0,
                functions: Vec::new(),
            });
            continue;
        }
        if !details.is_file() {
            return Err(ToolsError::invalid(format!("{}:{path} is not a regular source file", spec.id)));
        }
        let bytes = std::fs::read(&absolute).map_err(|error| ToolsError::io(format!("reading {}", absolute.display()), error))?;
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let is_typescript = path.ends_with(".ts") || path.ends_with(".tsx");
        files.push(CensusFile {
            path: path.clone(),
            tracking: if tracked.contains(path) { FileTracking::Tracked } else { FileTracking::Untracked },
            kind: classify_file(spec.role, path),
            bytes: bytes.len() as u64,
            sha256: hash_bytes(&bytes),
            line_count: source.split('\n').count(),
            functions: if is_typescript { enumerate_functions(&spec.id, path, &source) } else { Vec::new() },
        });
    }
    if census_clean(&repository_root, spec)? != revision {
        return Err(ToolsError::invalid(format!("{}: source changed during the census", spec.id)));
    }
    let fingerprint = Json::object(vec![
        ("revision".to_owned(), Json::string(&revision)),
        ("submodules".to_owned(), Json::array(submodules.iter().map(CensusSubmodule::to_json).collect())),
        (
            "files".to_owned(),
            Json::array(
                files
                    .iter()
                    .map(|file| {
                        Json::object(vec![
                            ("path".to_owned(), Json::string(&file.path)),
                            ("tracking".to_owned(), Json::string(file.tracking.as_str())),
                            ("sha256".to_owned(), Json::string(&file.sha256)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    Ok(CapturedRepository { spec: spec.clone(), revision, source_set_sha256: hash_str(&fingerprint.render()), submodules, files })
}

/// A built source manifest.
#[derive(Debug, Clone)]
pub struct SourceManifest {
    /// Captured repositories.
    pub repositories: Vec<CapturedRepository>,
    /// ISO-8601 capture time.
    pub captured_at: String,
}

impl SourceManifest {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let total_files: usize = self.repositories.iter().map(|repository| repository.files.len()).sum();
        let total_functions: usize =
            self.repositories.iter().map(|repository| repository.files.iter().map(|file| file.functions.len()).sum::<usize>()).sum();
        Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("generator".to_owned(), Json::string("crates/tools/src/bin/qa-source-census.rs")),
            ("capturedAt".to_owned(), Json::string(&self.captured_at)),
            (
                "repositories".to_owned(),
                Json::array(self.repositories.iter().map(CapturedRepository::to_json).collect()),
            ),
            (
                "totals".to_owned(),
                Json::object(vec![
                    ("repositories".to_owned(), Json::uint(self.repositories.len() as u64)),
                    ("files".to_owned(), Json::uint(total_files as u64)),
                    ("typeScriptFunctions".to_owned(), Json::uint(total_functions as u64)),
                ]),
            ),
            (
                "functionInventory".to_owned(),
                Json::object(vec![
                    ("parser".to_owned(), Json::string(format!("qa-tools-ts-scan@{}", env!("CARGO_PKG_VERSION")))),
                    (
                        "includes".to_owned(),
                        Json::array(
                            [
                                "TypeScript function declarations, methods, accessors, constructors and arrow functions",
                                "Function expressions assigned to variables, properties and callbacks",
                                "Overloads and ambient signatures are separate entries",
                            ]
                            .into_iter()
                            .map(Json::string)
                            .collect(),
                        ),
                    ),
                    (
                        "limits".to_owned(),
                        Json::array(
                            [
                                "Original C, header, QuakeC and assembly files enumerate bytes and hashes only; their functions are not parsed by the TypeScript compiler",
                                "A file revision is accepted only when the working tree is clean and HEAD matches the reviewed revision",
                                "Untracked files are included only when they survive the repository exclude rules",
                                "Names record lexical intent, not overload resolution or call targets",
                            ]
                            .into_iter()
                            .map(Json::string)
                            .collect(),
                        ),
                    ),
                ]),
            ),
        ])
    }
}

/// Build the source manifest for a project root.
pub fn build_source_manifest(project_root: &Path) -> Result<SourceManifest, ToolsError> {
    let mut captured = Vec::new();
    for spec in repositories() {
        captured.push(capture_repository(project_root, &spec)?);
    }
    Ok(SourceManifest { repositories: captured, captured_at: now_iso() })
}

/// Parsed inventory arguments.
#[derive(Debug, Clone)]
pub struct InventoryArguments {
    /// Project root.
    pub root: PathBuf,
    /// Whether to verify instead of writing.
    pub check: bool,
}

/// Parse `[--root <dir>] [--check]` arguments.
pub fn inventory_arguments(args: &[String], default_root: &Path) -> Result<InventoryArguments, ToolsError> {
    let mut root = default_root.to_path_buf();
    let mut check = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let value = args.get(index + 1);
                if value.is_none_or(|next| next.starts_with("--")) {
                    return Err(ToolsError::invalid("--root requires a project directory"));
                }
                root = lexical_absolute(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")), &value.expect("root value"));
                index += 2;
            }
            "--check" => {
                check = true;
                index += 1;
            }
            other => return Err(ToolsError::invalid(format!("Unknown argument: {other}"))),
        }
    }
    Ok(InventoryArguments { root, check })
}

/// Verify generated text against a file, or stage and atomically replace it.
pub fn write_or_check(path: &Path, text: &str, check: bool) -> Result<(), ToolsError> {
    if check {
        let current = std::fs::read_to_string(path).map_err(|error| ToolsError::io(format!("reading {}", path.display()), error))?;
        if current != text {
            return Err(ToolsError::invalid(format!("Generated inventory differs: {}", path.display())));
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    }
    let staging = make_temp_dir(path.parent().unwrap_or_else(|| Path::new(".")), ".inventory-")?;
    let staged = staging.join("manifest.json");
    std::fs::write(&staged, text).map_err(|error| ToolsError::io(format!("writing {}", staged.display()), error))?;
    std::fs::rename(&staged, path).map_err(|error| ToolsError::io(format!("replacing {}", path.display()), error))?;
    Ok(())
}

/// Run the census: build the manifest, write or verify it, and report.
pub fn run_census(options: &InventoryArguments) -> Result<String, ToolsError> {
    let manifest = build_source_manifest(&options.root)?;
    let output = options.root.join("verification/source-manifest.json");
    let text = format!("{}\n", manifest.to_json().render_pretty());
    write_or_check(&output, &text, options.check)?;
    let total_files: usize = manifest.repositories.iter().map(|repository| repository.files.len()).sum();
    let total_functions: usize =
        manifest.repositories.iter().map(|repository| repository.files.iter().map(|file| file.functions.len()).sum::<usize>()).sum();
    Ok(format!(
        "{} {}: {} repositories, {} files, {} TypeScript functions.\n",
        if options.check { "Verified" } else { "Wrote" },
        posix_relative(&options.root, &output),
        manifest.repositories.len(),
        total_files,
        total_functions
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "src/a.ts"), FileKind::Runtime);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "tools/a.ts"), FileKind::Tool);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "src/a.test.ts"), FileKind::Test);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "src/__tests__/a.ts"), FileKind::Test);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "tests/x.ts"), FileKind::Test);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "src/contest.ts"), FileKind::Runtime);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "a.d.ts"), FileKind::Declaration);
        assert_eq!(classify_file(RepositoryRole::OriginalReference, "a.C"), FileKind::ReferenceCode);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "a.c"), FileKind::Other);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "README.md"), FileKind::Document);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "LICENSE"), FileKind::Document);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "LICENSES"), FileKind::Other);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "a.json"), FileKind::Configuration);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, ".hidden/x"), FileKind::Configuration);
        assert_eq!(classify_file(RepositoryRole::ImplementationCandidate, "a.bin"), FileKind::Other);
    }

    #[test]
    fn rejects_bad_relative_paths() {
        for bad in ["", "/abs", "a\\b", "../x", "a/../b", "./x", "a//b"] {
            assert!(safe_relative_path(bad).is_err(), "{bad}");
        }
        assert_eq!(safe_relative_path("a/b.ts").unwrap(), "a/b.ts");
    }

    #[test]
    fn truncates_callback_names_at_120_units() {
        let long = format!("{} callback", "f".repeat(200));
        assert_eq!(census_name(&long), format!("{} callback", "f".repeat(120)));
        assert_eq!(census_name("plain"), "plain");
        assert_eq!(census_name("x.y"), "x.y");
        assert_eq!(census_name("\"quoted callback\""), "\"quoted callback\"");
    }

    #[test]
    fn enumerates_functions_with_offsets() {
        let functions = enumerate_functions("repo", "a.ts", "export function f(a: number): void {\n  g(() => 1);\n}\n");
        assert_eq!(functions.len(), 2);
        assert_eq!(functions[0].id, "repo:a.ts#0");
        assert_eq!(functions[0].name, "f");
        assert_eq!(functions[0].line, 1);
        assert_eq!(functions[0].end_line, 3);
        assert_eq!(functions[1].name, "g callback");
    }

    #[test]
    fn parses_inventory_arguments() {
        let root = PathBuf::from("/repo");
        let parsed = inventory_arguments(&[], &root).unwrap();
        assert_eq!(parsed.root, root);
        assert!(!parsed.check);
        let parsed = inventory_arguments(&["--check".to_owned(), "--root".to_owned(), "sub".to_owned()], &root).unwrap();
        assert!(parsed.check);
        assert!(parsed.root.is_absolute());
        assert!(inventory_arguments(&["--root".to_owned()], &root).is_err());
        assert!(inventory_arguments(&["--nope".to_owned()], &root).is_err());
    }
}
