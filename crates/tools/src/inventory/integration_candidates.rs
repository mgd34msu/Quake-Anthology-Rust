//! Integration candidate snapshots (donor `tools/inventory/integration-candidates.ts`).
//!
//! The donor drives the TypeScript compiler API. This port drives the shared
//! [`ts_scan`](super::ts_scan) walker instead, so token-hash *values* and
//! diagnostic *messages* differ from the donor while matching behavior is
//! preserved: indexes and candidates are built and queried with the same
//! functions inside one run. Declaration offsets are UTF-16 code units, line
//! numbers count `\n` breaks, and signatures use JavaScript trim semantics,
//! matching the donor for real-world sources. The `parser` field reports the
//! ported scanner version because there is no TypeScript compiler here.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::merge_integration_candidates::git_bytes;
use super::source_census::census_functions_from_scan;
use super::ts_scan::{self, DeclInfo};
use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{parse_json, Json};
use crate::js::{trim_js, Utf16Map};
use crate::reference::environment::{projects_root, quake_typescript_root};
use crate::sha256::hash_hex;

/// Frozen unified revision the donor snapshots.
pub const BASELINE: &str = "3dc9ca6ca404dc4522ab9e9669fff458805b0993";

/// Donor game key to sibling repository directory.
pub const DONORS: [(&str, &str); 3] = [("q1", "quake-1-re-ts"), ("q2", "quake-2-re-ts"), ("q3", "quake-3-ts")];

/// CLI options (donor `options`).
pub struct Options {
    /// Donor game key.
    pub game: String,
    /// Output directory.
    pub out: PathBuf,
    /// Donor revision expression.
    pub donor_revision: String,
}

/// Parse CLI options from key/value pairs.
pub fn parse_options(args: &[String]) -> Result<Options, ToolsError> {
    let mut game = "q1".to_owned();
    let mut out = PathBuf::from("/tmp/quake-integration-q1");
    let mut donor_revision = "HEAD".to_owned();
    let mut index = 0;
    while index < args.len() {
        let key = &args[index];
        let value = args.get(index + 1).ok_or_else(|| ToolsError::invalid(format!("Missing value for {key}")))?;
        if key == "--game" && (value == "q1" || value == "q2" || value == "q3") {
            game = value.clone();
        } else if key == "--out" {
            let cwd = std::env::current_dir().map_err(|error| ToolsError::io("resolving current directory", error))?;
            out = fsutil::lexical_absolute(&cwd, value);
        } else if key == "--donor-revision" {
            donor_revision = value.clone();
        } else {
            return Err(ToolsError::invalid(format!("Unknown argument {key} {value}")));
        }
        index += 2;
    }
    Ok(Options { game, out, donor_revision })
}

/// Whether a git tree path is a scanned TypeScript file (case-sensitive donor regex).
fn selected(path: &str) -> bool {
    path.ends_with(".ts") || path.ends_with(".tsx") || path.ends_with(".mts") || path.ends_with(".cts")
}

/// Classify a file the way the donor `kind` expression does.
fn classify_kind(path: &str) -> &'static str {
    if path.ends_with(".d.ts") || path.ends_with(".d.mts") || path.ends_with(".d.cts") {
        return "declaration";
    }
    if is_test_path(path) {
        return "test";
    }
    if path.starts_with("src/") {
        return "runtime";
    }
    "tool"
}

/// Donor test-path expression: `/(?:^|\/)(?:tests?|__tests__)(?:\/|\.)|\.(?:test|spec)\./`.
fn is_test_path(path: &str) -> bool {
    if path.contains(".test.") || path.contains(".spec.") {
        return true;
    }
    let segments: Vec<&str> = path.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        for marker in ["test", "tests", "__tests__"] {
            if *segment == marker && index + 1 < segments.len() {
                return true;
            }
            if segment.starts_with(marker) && segment[marker.len()..].starts_with('.') {
                return true;
            }
        }
    }
    false
}

/// Split a NUL-separated `ls-tree` entry into its path plus symlink flag.
fn parse_ls_entry(entry: &str) -> (String, bool) {
    let path = entry.find('\t').map_or(entry, |tab| &entry[tab + 1..]).to_owned();
    (path, entry.starts_with("120000 "))
}

/// POSIX basename (donor `basename` on git-relative paths).
fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A feature-evidence source anchor with numeric lines.
pub struct Anchor {
    /// Feature id.
    pub id: String,
    /// Repository id.
    pub repository: String,
    /// Repository-relative path.
    pub path: String,
    /// 1-based start line.
    pub line: f64,
    /// 1-based end line.
    pub end_line: f64,
}

impl Anchor {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("repository".to_owned(), Json::string(&self.repository)),
            ("path".to_owned(), Json::string(&self.path)),
            ("line".to_owned(), Json::float(self.line)),
            ("endLine".to_owned(), Json::float(self.end_line)),
        ])
    }
}

/// Accepted anchors plus rejected evidence rows.
pub struct AnchorInventory {
    /// Numeric source-location anchors.
    pub accepted: Vec<Anchor>,
    /// `{featureId, evidence, reason}` rows.
    pub rejected: Vec<Json>,
}

impl AnchorInventory {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("accepted".to_owned(), Json::array(self.accepted.iter().map(Anchor::to_json).collect())),
            ("rejected".to_owned(), Json::array(self.rejected.clone())),
        ])
    }
}

/// A scanned declaration record.
pub struct Declaration {
    /// `{repository}:{path}#{start}:{end}:{kind}` with UTF-16 offsets.
    pub id: String,
    /// Repository-relative path.
    pub path: String,
    /// Inferred name.
    pub name: String,
    /// Syntax kind name.
    pub kind: String,
    /// 1-based start line.
    pub line: usize,
    /// 1-based end line.
    pub end_line: usize,
    /// UTF-16 start offset.
    pub start_offset: usize,
    /// UTF-16 end offset.
    pub end_offset: usize,
    /// Source slice before the body, trimmed.
    pub signature: String,
    /// Token hash of the signature.
    pub signature_hash: String,
    /// Token hash of the body text, when present.
    pub body_hash: Option<String>,
    /// Enclosing names, outermost first.
    pub enclosing: Vec<String>,
    /// Own plus enclosing `export`/`default` modifiers.
    pub exports: Vec<String>,
    /// Overlapping feature-evidence ids.
    pub evidence_ids: Vec<String>,
}

impl Declaration {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("path".to_owned(), Json::string(&self.path)),
            ("name".to_owned(), Json::string(&self.name)),
            ("kind".to_owned(), Json::string(&self.kind)),
            ("line".to_owned(), Json::uint(self.line as u64)),
            ("endLine".to_owned(), Json::uint(self.end_line as u64)),
            ("startOffset".to_owned(), Json::uint(self.start_offset as u64)),
            ("endOffset".to_owned(), Json::uint(self.end_offset as u64)),
            ("signature".to_owned(), Json::string(&self.signature)),
            ("signatureTokenHash".to_owned(), Json::string(&self.signature_hash)),
            ("bodyTokenHash".to_owned(), self.body_hash.as_ref().map_or(Json::Null, Json::string)),
            ("enclosing".to_owned(), Json::array(self.enclosing.iter().map(Json::string).collect())),
            ("exports".to_owned(), Json::array(self.exports.iter().map(Json::string).collect())),
            ("evidenceIds".to_owned(), Json::array(self.evidence_ids.iter().map(Json::string).collect())),
        ])
    }
}

/// A captured source file record.
pub struct FileRecord {
    /// Repository-relative path.
    pub path: String,
    /// SHA-256 of the raw blob bytes.
    pub sha256: String,
    /// Blob byte length.
    pub bytes: usize,
    /// declaration, test, runtime, or tool.
    pub kind: String,
    /// Import/export module specifiers with quotes.
    pub imports: Vec<String>,
    /// Scanned declarations in source order.
    pub declarations: Vec<Declaration>,
    /// Census function count.
    pub census_functions: usize,
    /// Syntactic diagnostics.
    pub diagnostics: Vec<String>,
    /// Skip reason for symlink blobs.
    pub skipped: Option<String>,
}

impl FileRecord {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("bytes".to_owned(), Json::uint(self.bytes as u64)),
            ("kind".to_owned(), Json::string(&self.kind)),
            ("imports".to_owned(), Json::array(self.imports.iter().map(Json::string).collect())),
            ("declarations".to_owned(), Json::array(self.declarations.iter().map(Declaration::to_json).collect())),
            ("censusFunctions".to_owned(), Json::uint(self.census_functions as u64)),
            ("diagnostics".to_owned(), Json::array(self.diagnostics.iter().map(Json::string).collect())),
            ("skipped".to_owned(), self.skipped.as_ref().map_or(Json::Null, Json::string)),
        ])
    }
}

/// A captured repository snapshot.
pub struct Repository {
    /// Repository id (`{game}-ts` or `unified`).
    pub id: String,
    /// Absolute repository root.
    pub root: String,
    /// Captured commit revision.
    pub revision: String,
    /// Hash over the path/sha256 file list.
    pub source_set_sha256: String,
    /// Captured files in `ls-tree` order.
    pub files: Vec<FileRecord>,
}

impl Repository {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("root".to_owned(), Json::string(&self.root)),
            ("revision".to_owned(), Json::string(&self.revision)),
            ("sourceSetSha256".to_owned(), Json::string(&self.source_set_sha256)),
            ("files".to_owned(), Json::array(self.files.iter().map(FileRecord::to_json).collect())),
        ])
    }
}

/// Collect anchors from one raw feature inventory per game (pure part of donor `anchors`).
fn collect_anchors(inventories: &[(&str, &[u8])]) -> Result<AnchorInventory, ToolsError> {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    for (_, bytes) in inventories {
        let value = parse_json(&String::from_utf8_lossy(bytes))?;
        let object = value.as_object().ok_or_else(|| ToolsError::invalid("Invalid feature inventory"))?;
        let features = object
            .iter()
            .find(|(key, _)| key == "features")
            .and_then(|(_, features)| features.as_array())
            .ok_or_else(|| ToolsError::invalid("Invalid feature inventory"))?;
        for feature in features {
            let entries = feature.as_object().ok_or_else(|| ToolsError::invalid("Invalid feature"))?;
            let id = entries
                .iter()
                .find(|(key, _)| key == "id")
                .and_then(|(_, id)| id.as_str())
                .ok_or_else(|| ToolsError::invalid("Invalid feature"))?;
            let evidence = entries
                .iter()
                .find(|(key, _)| key == "evidence")
                .and_then(|(_, evidence)| evidence.as_array())
                .ok_or_else(|| ToolsError::invalid("Invalid feature"))?;
            for row in evidence {
                let fields = row.as_object();
                let anchor = fields.and_then(|fields| {
                    let get = |key: &str| fields.iter().find(|(name, _)| name == key).map(|(_, value)| value);
                    let repository = get("repository")?.as_str()?;
                    let path = get("path")?.as_str()?;
                    let line = get("line")?.as_f64()?;
                    let end_line = get("endLine")?.as_f64()?;
                    Some(Anchor { id: id.to_owned(), repository: repository.to_owned(), path: path.to_owned(), line, end_line })
                });
                match anchor {
                    Some(anchor) => accepted.push(anchor),
                    None => rejected.push(Json::object(vec![
                        ("featureId".to_owned(), Json::string(id)),
                        ("evidence".to_owned(), row.clone()),
                        ("reason".to_owned(), Json::string("Not a numeric source-location anchor")),
                    ])),
                }
            }
        }
    }
    Ok(AnchorInventory { accepted, rejected })
}

/// Read per-game feature inventories at the baseline revision (donor `anchors`).
fn anchors(root: &Path) -> Result<AnchorInventory, ToolsError> {
    let mut inventories = Vec::new();
    for (game, _) in DONORS {
        let spec = format!("{BASELINE}:verification/features/{game}.json");
        inventories.push((game, git_bytes(root, &["show", &spec])?));
    }
    let borrowed: Vec<(&str, &[u8])> = inventories.iter().map(|(game, bytes)| (*game, bytes.as_slice())).collect();
    collect_anchors(&borrowed)
}

/// Map walker declarations onto donor visitor records.
fn map_declarations(
    repository: &str,
    path: &str,
    source: &str,
    decls: &[DeclInfo],
    evidence: &[Anchor],
) -> Vec<Declaration> {
    let starts = ts_scan::line_starts(source);
    let utf16 = Utf16Map::new(source);
    let mut declarations = Vec::with_capacity(decls.len());
    for decl in decls {
        let signature_end = decl.body.map(|(start, _)| start).or(decl.members_pos).or(decl.initializer_start).unwrap_or(decl.end);
        let signature = trim_js(&source[decl.start..signature_end]).to_owned();
        let start_offset = utf16.to_utf16(decl.start);
        let end_offset = utf16.to_utf16(decl.end);
        let line = ts_scan::line_of(&starts, decl.start);
        let end_line = ts_scan::line_of(&starts, decl.end.saturating_sub(1));
        let mut enclosing = decl.enclosing.clone();
        enclosing.reverse();
        let evidence_ids: Vec<String> = evidence
            .iter()
            .filter(|anchor| {
                anchor.repository == repository
                    && anchor.path == path
                    && anchor.line <= end_line as f64
                    && anchor.end_line >= line as f64
            })
            .map(|anchor| anchor.id.clone())
            .collect();
        declarations.push(Declaration {
            id: format!("{repository}:{path}#{start_offset}:{end_offset}:{}", decl.kind),
            path: path.to_owned(),
            name: decl.name.clone(),
            kind: decl.kind.to_owned(),
            line,
            end_line,
            start_offset,
            end_offset,
            signature_hash: ts_scan::token_hash(&signature),
            body_hash: decl.body.map(|(start, end)| ts_scan::token_hash(&source[start..end])),
            signature,
            enclosing,
            exports: decl.exports.clone(),
            evidence_ids,
        });
    }
    declarations
}

/// Capture one repository snapshot (donor `capture`).
fn capture(root: &Path, id: &str, revision: &str, evidence: &[Anchor]) -> Result<Repository, ToolsError> {
    let listing = git_bytes(root, &["ls-tree", "-r", "-z", revision])?;
    let mut files = Vec::new();
    for entry in String::from_utf8_lossy(&listing).split('\0').filter(|entry| !entry.is_empty()) {
        let (path, symlink) = parse_ls_entry(entry);
        if !selected(&path) {
            continue;
        }
        let spec = format!("{revision}:{path}");
        let bytes = git_bytes(root, &["show", &spec])?;
        let source = String::from_utf8_lossy(&bytes).into_owned();
        let mut file = FileRecord {
            path,
            sha256: hash_hex(&bytes),
            bytes: bytes.len(),
            kind: String::new(),
            imports: Vec::new(),
            declarations: Vec::new(),
            census_functions: 0,
            diagnostics: Vec::new(),
            skipped: symlink.then(|| "symlink Git blob is a link target, not TypeScript source".to_owned()),
        };
        file.kind = classify_kind(&file.path).to_owned();
        if file.skipped.is_none() {
            let scanned = ts_scan::scan_declarations(&source);
            file.imports = scanned.imports.clone();
            file.diagnostics = scanned.diagnostics.clone();
            file.declarations = map_declarations(id, &file.path, &source, &scanned.declarations, evidence);
            let ids: HashSet<&str> = file.declarations.iter().map(|declaration| declaration.id.as_str()).collect();
            if ids.len() != file.declarations.len() {
                return Err(ToolsError::invalid(format!("Duplicate declaration IDs in {id}:{}", file.path)));
            }
            let census = census_functions_from_scan(id, &file.path, &source, &scanned.declarations);
            file.census_functions = census.len();
            for row in &census {
                if !file.declarations.iter().any(|candidate| candidate.start_offset == row.start_offset) {
                    return Err(ToolsError::invalid(format!("Lost census declaration {}", row.id)));
                }
            }
        }
        files.push(file);
    }
    let listing_json = Json::array(
        files
            .iter()
            .map(|file| {
                Json::object(vec![
                    ("path".to_owned(), Json::string(&file.path)),
                    ("sha256".to_owned(), Json::string(&file.sha256)),
                ])
            })
            .collect(),
    );
    Ok(Repository {
        id: id.to_owned(),
        root: root.to_string_lossy().into_owned(),
        revision: revision.to_owned(),
        source_set_sha256: hash_hex(listing_json.render().as_bytes()),
        files,
    })
}

/// First-seen-ordered match index over unified declarations.
struct Index {
    /// Keys in first-seen order.
    order: Vec<String>,
    /// Key to unified declaration ids.
    map: HashMap<String, Vec<String>>,
}

/// Deduplicated match keys for one declaration (donor key expression).
fn declaration_keys(
    name: &str,
    signature_hash: &str,
    body_hash: Option<&str>,
    file_path: &str,
    imports: &[String],
) -> Vec<String> {
    let mut keys = vec![format!("name:{name}"), format!("signature:{signature_hash}")];
    if let Some(hash) = body_hash {
        keys.push(format!("body:{hash}"));
    }
    keys.push(format!("path-basename:{}", basename(file_path)));
    for specifier in imports {
        keys.push(format!("import-specifier:{specifier}"));
    }
    let mut unique = Vec::with_capacity(keys.len());
    for key in keys {
        if !unique.contains(&key) {
            unique.push(key);
        }
    }
    unique
}

/// Build the unified match index (donor `indexes`).
fn build_index(files: &[FileRecord]) -> Index {
    let mut order = Vec::new();
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for file in files {
        for declaration in &file.declarations {
            let keys = declaration_keys(
                &declaration.name,
                &declaration.signature_hash,
                declaration.body_hash.as_deref(),
                &file.path,
                &file.imports,
            );
            for key in keys {
                if !map.contains_key(&key) {
                    order.push(key.clone());
                }
                map.entry(key).or_default().push(declaration.id.clone());
            }
        }
    }
    Index { order, map }
}

/// A donor declaration match verdict.
struct Candidate {
    /// Donor declaration id.
    donor_id: String,
    /// unmatched, single-candidate, or ambiguous.
    status: &'static str,
    /// Index keys shared with the unified snapshot.
    group_ids: Vec<String>,
}

impl Candidate {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("donorId".to_owned(), Json::string(&self.donor_id)),
            ("status".to_owned(), Json::string(self.status)),
            ("groupIds".to_owned(), Json::array(self.group_ids.iter().map(Json::string).collect())),
        ])
    }
}

/// Match one donor declaration against the unified index (donor candidate expression).
fn match_candidate(keys: &[String], index: &Index) -> (&'static str, Vec<String>) {
    let group_ids: Vec<String> = keys.iter().filter(|key| index.map.contains_key(*key)).cloned().collect();
    let mut witnesses: Vec<&str> = Vec::new();
    'outer: for key in &group_ids {
        if let Some(ids) = index.map.get(key) {
            for id in ids {
                if !witnesses.contains(&id.as_str()) {
                    witnesses.push(id.as_str());
                    if witnesses.len() > 1 {
                        break 'outer;
                    }
                }
            }
        }
    }
    let status = match witnesses.len() {
        0 => "unmatched",
        1 => "single-candidate",
        _ => "ambiguous",
    };
    (status, group_ids)
}

/// Match every donor declaration (donor `candidates`).
fn match_all(files: &[FileRecord], index: &Index) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for file in files {
        for declaration in &file.declarations {
            let keys = declaration_keys(
                &declaration.name,
                &declaration.signature_hash,
                declaration.body_hash.as_deref(),
                &file.path,
                &file.imports,
            );
            let (status, group_ids) = match_candidate(&keys, index);
            candidates.push(Candidate { donor_id: declaration.id.clone(), status, group_ids });
        }
    }
    candidates
}

/// Summary row for one captured repository.
fn repository_summary(repository: &Repository) -> Json {
    let declarations: Vec<&Declaration> =
        repository.files.iter().flat_map(|file| file.declarations.iter()).collect();
    Json::object(vec![
        ("id".to_owned(), Json::string(&repository.id)),
        ("revision".to_owned(), Json::string(&repository.revision)),
        ("sourceSetSha256".to_owned(), Json::string(&repository.source_set_sha256)),
        ("files".to_owned(), Json::uint(repository.files.len() as u64)),
        ("declarations".to_owned(), Json::uint(declarations.len() as u64)),
        (
            "censusFunctions".to_owned(),
            Json::uint(repository.files.iter().map(|file| file.census_functions as u64).sum::<u64>()),
        ),
        (
            "unanchored".to_owned(),
            Json::uint(declarations.iter().filter(|declaration| declaration.evidence_ids.is_empty()).count() as u64),
        ),
        (
            "classifications".to_owned(),
            Json::object(
                ["runtime", "test", "tool", "declaration"]
                    .into_iter()
                    .map(|kind| {
                        (
                            kind.to_owned(),
                            Json::uint(repository.files.iter().filter(|file| file.kind == kind).count() as u64),
                        )
                    })
                    .collect(),
            ),
        ),
        (
            "unparseable".to_owned(),
            Json::array(
                repository
                    .files
                    .iter()
                    .filter(|file| !file.diagnostics.is_empty())
                    .map(|file| {
                        Json::object(vec![
                            ("path".to_owned(), Json::string(&file.path)),
                            ("diagnostics".to_owned(), Json::array(file.diagnostics.iter().map(Json::string).collect())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "skipped".to_owned(),
            Json::array(
                repository
                    .files
                    .iter()
                    .filter(|file| file.skipped.is_some())
                    .map(|file| {
                        Json::object(vec![
                            ("path".to_owned(), Json::string(&file.path)),
                            ("reason".to_owned(), Json::string(file.skipped.as_ref().expect("filtered skip"))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Assemble the run summary (donor `summary`).
fn summarize(game: &str, donor: &Repository, unified: &Repository, candidates: &[Candidate]) -> Json {
    Json::object(vec![
        ("game".to_owned(), Json::string(game)),
        ("donorRevision".to_owned(), Json::string(&donor.revision)),
        ("unifiedRevision".to_owned(), Json::string(BASELINE)),
        (
            "limits".to_owned(),
            Json::array(
                [
                    "All matches are review candidates, never behavioral verdicts.",
                    "Current worktree and post-baseline integration are outside this snapshot.",
                    "Export modifiers are lexical; re-export resolution is not attempted.",
                    "Signatures retain written syntax; inferred return/parameter types are not synthesized.",
                    "Import and basename matches are intentionally broad hints. All are retained.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
        ("repositories".to_owned(), Json::array(vec![repository_summary(donor), repository_summary(unified)])),
        (
            "statuses".to_owned(),
            Json::object(
                ["unmatched", "single-candidate", "ambiguous"]
                    .into_iter()
                    .map(|status| {
                        (
                            status.to_owned(),
                            Json::uint(candidates.iter().filter(|candidate| candidate.status == status).count() as u64),
                        )
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Join feature ids to declaration ids across both snapshots (donor `featureInventories`).
fn feature_inventories(root: &Path, donor: &Repository, unified: &Repository) -> Result<Vec<Json>, ToolsError> {
    let mut inventories = Vec::new();
    for (family, _) in DONORS {
        let path = format!("verification/features/{family}.json");
        let bytes = git_bytes(root, &["show", &format!("{BASELINE}:{path}")])?;
        let inventory = parse_json(&String::from_utf8_lossy(&bytes))?;
        let features = inventory
            .as_object()
            .and_then(|object| object.iter().find(|(key, _)| key == "features"))
            .and_then(|(_, features)| features.as_array())
            .ok_or_else(|| ToolsError::invalid("Invalid feature inventory"))?;
        let mut joins = Vec::new();
        for feature in features {
            let id = feature
                .as_object()
                .and_then(|object| object.iter().find(|(key, _)| key == "id"))
                .and_then(|(_, id)| id.as_str())
                .ok_or_else(|| ToolsError::invalid("Invalid feature ID"))?;
            let mut declaration_ids = Vec::new();
            for repository in [donor, unified] {
                for file in &repository.files {
                    for declaration in &file.declarations {
                        if declaration.evidence_ids.iter().any(|evidence| evidence == id) {
                            declaration_ids.push(Json::string(&declaration.id));
                        }
                    }
                }
            }
            joins.push(Json::object(vec![
                ("id".to_owned(), Json::string(id)),
                ("declarationIds".to_owned(), Json::array(declaration_ids)),
            ]));
        }
        inventories.push(Json::object(vec![
            ("path".to_owned(), Json::string(&path)),
            ("sha256".to_owned(), Json::string(hash_hex(&bytes))),
            ("inventory".to_owned(), inventory),
            ("joins".to_owned(), Json::array(joins)),
        ]));
    }
    Ok(inventories)
}

/// Capture the historical completion verdicts (donor `historicalCompletion`).
fn historical_completion(root: &Path) -> Result<Json, ToolsError> {
    let bytes = git_bytes(root, &["show", &format!("{BASELINE}:docs/completion-status.json")])?;
    let inventory = parse_json(&String::from_utf8_lossy(&bytes))?;
    Ok(Json::object(vec![
        ("capturedAtRevision".to_owned(), Json::string(BASELINE)),
        ("path".to_owned(), Json::string("docs/completion-status.json")),
        ("sha256".to_owned(), Json::string(hash_hex(&bytes))),
        (
            "note".to_owned(),
            Json::string("Historical verdicts retain their own sourceCutoff; not current candidate conclusions."),
        ),
        ("inventory".to_owned(), inventory),
    ]))
}

/// Snapshot one donor game into the output directory (donor `main`).
pub fn run(args: &[String]) -> Result<(), ToolsError> {
    let options = parse_options(args)?;
    let root = quake_typescript_root();
    let donor_directory = DONORS.iter().find(|(game, _)| *game == options.game).map_or("quake-1-re-ts", |(_, directory)| *directory);
    let donor_root = projects_root().join(donor_directory);
    let anchor_inventory = anchors(&root)?;
    let revision_spec = format!("{}^{{commit}}", options.donor_revision);
    let donor_revision = String::from_utf8_lossy(&git_bytes(&donor_root, &["rev-parse", &revision_spec])?).trim().to_owned();
    let donor = capture(&donor_root, &format!("{}-ts", options.game), &donor_revision, &anchor_inventory.accepted)?;
    let unified = capture(&root, "unified", BASELINE, &anchor_inventory.accepted)?;
    let index = build_index(&unified.files);
    let candidates = match_all(&donor.files, &index);
    let summary = summarize(&options.game, &donor, &unified, &candidates);
    let inventories = feature_inventories(&root, &donor, &unified)?;
    let completion = historical_completion(&root)?;
    std::fs::create_dir_all(&options.out).map_err(|error| ToolsError::io(format!("creating {}", options.out.display()), error))?;
    fsutil::write_text(&options.out.join("anchor-inventory.json"), &anchor_inventory.to_json().render())?;
    fsutil::write_text(&options.out.join("historical-completion.json"), &completion.render())?;
    fsutil::write_text(
        &options.out.join("feature-join.json"),
        &Json::object(vec![
            ("revision".to_owned(), Json::string(BASELINE)),
            ("inventories".to_owned(), Json::array(inventories)),
        ])
        .render(),
    )?;
    fsutil::write_text(
        &options.out.join("inventory.json"),
        &Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("parser".to_owned(), Json::string(format!("qa-tools-ts-scan@{}", env!("CARGO_PKG_VERSION")))),
            ("repositories".to_owned(), Json::array(vec![donor.to_json(), unified.to_json()])),
        ])
        .render(),
    )?;
    fsutil::write_text(
        &options.out.join("candidates.json"),
        &Json::object(vec![
            (
                "groups".to_owned(),
                Json::object(
                    index
                        .order
                        .iter()
                        .map(|key| {
                            (
                                key.clone(),
                                Json::array(
                                    index.map.get(key).map_or(Vec::new(), |ids| ids.iter().map(Json::string).collect()),
                                ),
                            )
                        })
                        .collect(),
                ),
            ),
            ("candidates".to_owned(), Json::array(candidates.iter().map(Candidate::to_json).collect())),
        ])
        .render(),
    )?;
    let pretty = format!("{}\n", summary.render_pretty());
    fsutil::write_text(&options.out.join("summary.json"), &pretty)?;
    print!("{pretty}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(args: &[&str]) -> Options {
        parse_options(&args.iter().map(ToString::to_string).collect::<Vec<_>>()).expect("options")
    }

    #[test]
    fn parses_options() {
        let defaults = options(&[]);
        assert_eq!(defaults.game, "q1");
        assert_eq!(defaults.out, PathBuf::from("/tmp/quake-integration-q1"));
        assert_eq!(defaults.donor_revision, "HEAD");
        let custom = options(&["--game", "q3", "--out", "/tmp/custom", "--donor-revision", "abc123"]);
        assert_eq!(custom.game, "q3");
        assert_eq!(custom.out, PathBuf::from("/tmp/custom"));
        assert_eq!(custom.donor_revision, "abc123");
        assert!(parse_options(&["--game".to_owned()]).is_err());
        assert!(parse_options(&["--bogus".to_owned(), "x".to_owned()]).is_err());
        assert!(parse_options(&["--game".to_owned(), "q9".to_owned()]).is_err());
    }

    #[test]
    fn classifies_kinds() {
        for path in ["src/a.d.ts", "src/x.d.mts", "lib/y.d.cts"] {
            assert_eq!(classify_kind(path), "declaration", "{path}");
        }
        for path in ["tests/a.ts", "src/__tests__/a.ts", "test.ts", "src/tests.ts", "src/a.test.ts", "src/a.spec.ts", "test/x.ts"] {
            assert_eq!(classify_kind(path), "test", "{path}");
        }
        for path in ["src/a.ts", "src/testing/a.ts", "src/contest/a.ts", "src/mytest.ts"] {
            assert_eq!(classify_kind(path), "runtime", "{path}");
        }
        for path in ["tools/a.ts", "contest/a.ts", "attic/tester.ts"] {
            assert_eq!(classify_kind(path), "tool", "{path}");
        }
        assert!(!selected("src/a.TS"));
        assert!(selected("src/a.mts"));
    }

    #[test]
    fn parses_ls_entries() {
        assert_eq!(parse_ls_entry("100644 blob abc123\tsrc/a.ts"), ("src/a.ts".to_owned(), false));
        assert_eq!(parse_ls_entry("120000 blob abc123\tlink.ts"), ("link.ts".to_owned(), true));
        assert_eq!(parse_ls_entry("no-tab"), ("no-tab".to_owned(), false));
    }

    #[test]
    fn trims_like_javascript() {
        assert_eq!(trim_js("\u{feff} x \u{feff}"), "x");
        assert_eq!(trim_js("a\u{85}"), "a\u{85}");
        assert_eq!(trim_js("  padded\n"), "padded");
    }

    fn mapped(source: &str, evidence: &[Anchor]) -> Vec<Declaration> {
        let scanned = ts_scan::scan_declarations(source);
        map_declarations("unified", "src/a.ts", source, &scanned.declarations, evidence)
    }

    #[test]
    fn maps_function_records() {
        let source = "export function add(a: number, b: number): number {\n  return a + b;\n}\n";
        let declarations = mapped(source, &[]);
        let function = declarations.iter().find(|declaration| declaration.kind == "FunctionDeclaration").expect("function");
        assert_eq!(function.name, "add");
        assert_eq!(function.line, 1);
        assert_eq!(function.end_line, 3);
        assert_eq!(function.start_offset, 0);
        assert_eq!(function.end_offset, source.len() - 1);
        assert_eq!(function.signature, "export function add(a: number, b: number): number");
        assert_eq!(function.id, format!("unified:src/a.ts#0:{}:FunctionDeclaration", source.len() - 1));
        assert!(function.body_hash.is_some());
        assert_eq!(function.exports, vec!["export".to_owned()]);
    }

    #[test]
    fn reports_utf16_offsets() {
        let source = "// é\nfunction g() {\n}\n";
        let declarations = mapped(source, &[]);
        let function = declarations.iter().find(|declaration| declaration.name == "g").expect("g");
        assert_eq!(function.line, 2);
        assert_eq!(function.start_offset, 5);
        assert_eq!(source.as_bytes()[6], b'f');
    }

    #[test]
    fn joins_overlapping_evidence() {
        let source = "export function add(a: number): number {\n  return a;\n}\n";
        let anchor = |id: &str, line: f64, end_line: f64| Anchor {
            id: id.to_owned(),
            repository: "unified".to_owned(),
            path: "src/a.ts".to_owned(),
            line,
            end_line,
        };
        let foreign = Anchor {
            id: "other".to_owned(),
            repository: "q1-ts".to_owned(),
            path: "src/a.ts".to_owned(),
            line: 1.0,
            end_line: 3.0,
        };
        let declarations = mapped(
            source,
            &[anchor("inside", 2.0, 2.0), anchor("touching", 3.0, 3.0), anchor("outside", 4.0, 5.0), foreign],
        );
        let function = declarations.iter().find(|declaration| declaration.kind == "FunctionDeclaration").expect("function");
        assert_eq!(function.evidence_ids, vec!["inside".to_owned(), "touching".to_owned()]);
    }

    #[test]
    fn hashes_tokens_deterministically() {
        let first = ts_scan::token_hash("function f() { return 1; }");
        assert_eq!(first, ts_scan::token_hash("function f() { return 1; }"));
        assert_ne!(first, ts_scan::token_hash("function f() { return 2; }"));
    }

    fn fixture_declaration(id: &str, name: &str, signature: &str) -> Declaration {
        Declaration {
            id: id.to_owned(),
            path: "src/a.ts".to_owned(),
            name: name.to_owned(),
            kind: "FunctionDeclaration".to_owned(),
            line: 1,
            end_line: 1,
            start_offset: 0,
            end_offset: 1,
            signature: signature.to_owned(),
            signature_hash: ts_scan::token_hash(signature),
            body_hash: None,
            enclosing: Vec::new(),
            exports: Vec::new(),
            evidence_ids: Vec::new(),
        }
    }

    fn fixture_file(path: &str, declarations: Vec<Declaration>) -> FileRecord {
        FileRecord {
            path: path.to_owned(),
            sha256: String::new(),
            bytes: 0,
            kind: "runtime".to_owned(),
            imports: Vec::new(),
            declarations,
            census_functions: 0,
            diagnostics: Vec::new(),
            skipped: None,
        }
    }

    #[test]
    fn matches_statuses() {
        let unified = vec![fixture_file(
            "src/a.ts",
            vec![
                fixture_declaration("u1", "foo", "function foo()"),
                fixture_declaration("u2", "foo", "function foo(x)"),
            ],
        )];
        let index = build_index(&unified);
        let keys = |name: &str, signature: &str| declaration_keys(name, &ts_scan::token_hash(signature), None, "other/b.ts", &[]);
        let (status, groups) = match_candidate(&keys("foo", "function foo(q)"), &index);
        assert_eq!(status, "ambiguous");
        assert_eq!(groups, vec!["name:foo".to_owned()]);
        let (status, groups) = match_candidate(&keys("zzz", "function zzz()"), &index);
        assert_eq!(status, "unmatched");
        assert!(groups.is_empty());
        let (status, groups) = match_candidate(&keys("only", "function foo()"), &index);
        assert_eq!(status, "single-candidate");
        assert_eq!(groups, vec![format!("signature:{}", ts_scan::token_hash("function foo()"))]);
    }

    #[test]
    fn dedupes_declaration_keys() {
        let keys = declaration_keys("f", "sig", Some("body"), "src/a.ts", &["\"x\"".to_owned(), "\"x\"".to_owned()]);
        assert_eq!(keys, vec!["name:f", "signature:sig", "body:body", "path-basename:a.ts", "import-specifier:\"x\""]);
    }

    #[test]
    fn collects_anchors() {
        let valid = br#"{"features": [{"id": "F1", "evidence": [{"repository": "unified", "path": "src/a.ts", "line": 2, "endLine": 2}]}]}"#;
        let inventory = collect_anchors(&[("q1", &valid[..])]).expect("anchors");
        assert_eq!(inventory.accepted.len(), 1);
        assert_eq!(inventory.accepted[0].id, "F1");
        assert!(inventory.rejected.is_empty());
        assert!(collect_anchors(&[("q1", &br#"{"nope": 1}"#[..])]).is_err());
        assert!(collect_anchors(&[("q1", &br#"{"features": [{"id": 5}]}"#[..])]).is_err());
        let mixed = br#"{"features": [{"id": "F2", "evidence": [{"repository": "unified", "path": "src/a.ts", "line": "two", "endLine": 2}]}]}"#;
        let inventory = collect_anchors(&[("q1", &mixed[..])]).expect("mixed");
        assert!(inventory.accepted.is_empty());
        assert_eq!(inventory.rejected.len(), 1);
        assert_eq!(
            inventory.rejected[0].get("reason").and_then(Json::as_str),
            Some("Not a numeric source-location anchor")
        );
    }

    fn git(repo: &Path, args: &[&str]) {
        let output =
            std::process::Command::new("git").arg("-C").arg(repo).args(args).output().expect("spawn git");
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    }

    #[test]
    fn captures_git_snapshot() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "qa-integration-candidates-").expect("temp dir");
        let outcome = (|| -> Result<(), ToolsError> {
            git(&directory, &["init"]);
            git(&directory, &["config", "user.email", "test@example.com"]);
            git(&directory, &["config", "user.name", "Test"]);
            let source = "export function add(a: number, b: number): number {\n  return a + b;\n}\n";
            let file = directory.join("src/a.ts");
            std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
            std::fs::write(&file, source).expect("write");
            git(&directory, &["add", "."]);
            git(&directory, &["commit", "-m", "fixture"]);
            let repository = capture(&directory, "unified", "HEAD", &[])?;
            assert_eq!(repository.files.len(), 1);
            assert_eq!(repository.files[0].kind, "runtime");
            assert!(repository.files[0].declarations.iter().any(|declaration| declaration.kind == "FunctionDeclaration"));
            assert!(repository.files[0].census_functions >= 1);
            assert_eq!(repository.source_set_sha256.len(), 64);
            assert_eq!(repository.revision, "HEAD");
            Ok(())
        })();
        fsutil::remove_forced(&directory);
        outcome.expect("capture");
    }
}
