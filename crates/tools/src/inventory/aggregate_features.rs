//! Feature ledger aggregation (donor `tools/inventory/aggregate-features.ts`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::source_census::{inventory_arguments, safe_relative_path, write_or_check};
use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{parse_json, Json};
use crate::js::{compare_text, trim_js};
use crate::reference::environment::quake_typescript_root;
use crate::sha256::hash_hex;

/// A source-evidence anchor on a feature.
#[derive(Debug, Clone)]
pub struct Evidence {
    /// Repository id.
    pub repository: String,
    /// Repository-relative path.
    pub path: String,
    /// Required symbol name, when specified.
    pub symbol: Option<String>,
    /// 1-based start line.
    pub line: u64,
    /// 1-based end line.
    pub end_line: u64,
    /// Anchor role.
    pub role: String,
    /// Human observation.
    pub observation: String,
}

impl Evidence {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("repository".to_owned(), Json::string(&self.repository)),
            ("path".to_owned(), Json::string(&self.path)),
            ("symbol".to_owned(), self.symbol.as_ref().map_or(Json::Null, Json::string)),
            ("line".to_owned(), Json::uint(self.line)),
            ("endLine".to_owned(), Json::uint(self.end_line)),
            ("role".to_owned(), Json::string(&self.role)),
            ("observation".to_owned(), Json::string(&self.observation)),
        ])
    }
}

/// An exposed workflow on a feature.
#[derive(Debug, Clone)]
pub struct Workflow {
    /// Workflow id.
    pub id: String,
    /// Trigger description.
    pub trigger: String,
    /// Ordered steps.
    pub steps: Vec<String>,
    /// Observable outcome.
    pub observable_outcome: String,
}

impl Workflow {
    fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("trigger".to_owned(), Json::string(&self.trigger)),
            ("steps".to_owned(), Json::array(self.steps.iter().map(Json::string).collect())),
            ("observableOutcome".to_owned(), Json::string(&self.observable_outcome)),
        ])
    }
}

/// A required feature.
#[derive(Debug, Clone)]
pub struct Feature {
    /// Feature id.
    pub id: String,
    /// Source family ids.
    pub source_family_ids: Vec<String>,
    /// Unified feature ids.
    pub unified_feature_ids: Vec<String>,
    /// Title.
    pub title: String,
    /// Requirement text.
    pub requirement: String,
    /// Donor implementation status.
    pub source_status: String,
    /// Required products.
    pub products: Vec<String>,
    /// Owning task ids.
    pub owner_task_ids: Vec<String>,
    /// Expected acceptance case ids.
    pub acceptance_case_ids: Vec<String>,
    /// Source evidence anchors.
    pub evidence: Vec<Evidence>,
    /// Exposed workflows.
    pub workflows: Vec<Workflow>,
    /// Donor gaps.
    pub gaps: Vec<String>,
}

/// One family's feature shard.
#[derive(Debug, Clone)]
pub struct FeatureShard {
    /// Family key.
    pub family: String,
    /// Pinned donor revision.
    pub source_revision: String,
    /// Shard features.
    pub features: Vec<Feature>,
}

/// An indexed census function.
struct IndexedFunction {
    id: String,
    name: String,
    line: u64,
    end_line: u64,
}

/// An indexed source file.
struct IndexedFile {
    path: String,
    kind: String,
    sha256: String,
    line_count: u64,
    functions: Vec<IndexedFunction>,
}

/// An indexed source repository.
struct IndexedRepository {
    id: String,
    path: String,
    revision: String,
    source_set_sha256: String,
    files: Vec<IndexedFile>,
}

fn object<'a>(value: &'a Json, location: &str) -> Result<&'a [(String, Json)], ToolsError> {
    value.as_object().ok_or_else(|| ToolsError::invalid(format!("{location} must be an object")))
}

fn field<'a>(entries: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    entries.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

fn unknown_array<'a>(value: &'a Json, location: &str) -> Result<&'a [Json], ToolsError> {
    value.as_array().ok_or_else(|| ToolsError::invalid(format!("{location} must be an array")))
}

fn parse_array<T>(value: &Json, location: &str, parse: impl Fn(&Json, &str) -> Result<T, ToolsError>) -> Result<Vec<T>, ToolsError> {
    unknown_array(value, location)?
        .iter()
        .enumerate()
        .map(|(index, entry)| parse(entry, &format!("{location}[{index}]")))
        .collect()
}

fn nonempty_string(value: &Json, location: &str) -> Result<String, ToolsError> {
    match value.as_str() {
        Some(text) if !trim_js(text).is_empty() => Ok(text.to_owned()),
        _ => Err(ToolsError::invalid(format!("{location} must be a nonempty string"))),
    }
}

fn unique(values: &[String], location: &str) -> Result<(), ToolsError> {
    let distinct: HashSet<&str> = values.iter().map(String::as_str).collect();
    if distinct.len() != values.len() {
        return Err(ToolsError::invalid(format!("{location} contains duplicates")));
    }
    Ok(())
}

fn strings(value: &Json, location: &str) -> Result<Vec<String>, ToolsError> {
    let result = parse_array(value, location, |entry, entry_location| nonempty_string(entry, entry_location))?;
    unique(&result, location)?;
    Ok(result)
}

fn nonempty_strings(value: &Json, location: &str) -> Result<Vec<String>, ToolsError> {
    let result = strings(value, location)?;
    if result.is_empty() {
        return Err(ToolsError::invalid(format!("{location} must not be empty")));
    }
    Ok(result)
}

fn safe_integer(value: f64) -> Option<u64> {
    const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
    if value.is_finite() && value.fract() == 0.0 && value >= 0.0 && value <= MAX_SAFE {
        Some(value as u64)
    } else {
        None
    }
}

fn positive_integer(value: &Json, location: &str) -> Result<u64, ToolsError> {
    match value.as_f64().and_then(safe_integer) {
        Some(number) if number > 0 => Ok(number),
        _ => Err(ToolsError::invalid(format!("{location} must be a positive integer"))),
    }
}

fn hash(value: &Json, location: &str) -> Result<String, ToolsError> {
    let result = nonempty_string(value, location)?;
    if result.len() != 64 || !result.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
        return Err(ToolsError::invalid(format!("{location} must be a SHA-256 digest")));
    }
    Ok(result)
}

fn revision(value: &Json, location: &str) -> Result<String, ToolsError> {
    let result = nonempty_string(value, location)?;
    if result.len() != 40 || !result.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
        return Err(ToolsError::invalid(format!("{location} must be a Git revision")));
    }
    Ok(result)
}

fn parse_evidence(value: &Json, location: &str) -> Result<Evidence, ToolsError> {
    let item = object(value, location)?;
    let role = field(item, "role").and_then(Json::as_str).unwrap_or_default();
    if role != "implementation" && role != "original-contract" && role != "test" && role != "document" {
        return Err(ToolsError::invalid(format!("{location}.role is invalid")));
    }
    let symbol = match field(item, "symbol") {
        Some(Json::Null) => None,
        Some(value) => Some(nonempty_string(value, &format!("{location}.symbol"))?),
        None => return Err(ToolsError::invalid(format!("{location}.symbol must be a nonempty string"))),
    };
    let empty = Json::Null;
    let line = positive_integer(field(item, "line").unwrap_or(&empty), &format!("{location}.line"))?;
    let end_line = positive_integer(field(item, "endLine").unwrap_or(&empty), &format!("{location}.endLine"))?;
    if end_line < line {
        return Err(ToolsError::invalid(format!("{location} has a reversed line range")));
    }
    Ok(Evidence {
        repository: nonempty_string(field(item, "repository").unwrap_or(&empty), &format!("{location}.repository"))?,
        path: safe_relative_path(&nonempty_string(field(item, "path").unwrap_or(&empty), &format!("{location}.path"))?)?,
        symbol,
        line,
        end_line,
        role: role.to_owned(),
        observation: nonempty_string(field(item, "observation").unwrap_or(&empty), &format!("{location}.observation"))?,
    })
}

fn parse_workflow(value: &Json, location: &str) -> Result<Workflow, ToolsError> {
    let item = object(value, location)?;
    let empty = Json::Null;
    Ok(Workflow {
        id: nonempty_string(field(item, "id").unwrap_or(&empty), &format!("{location}.id"))?,
        trigger: nonempty_string(field(item, "trigger").unwrap_or(&empty), &format!("{location}.trigger"))?,
        steps: nonempty_strings(field(item, "steps").unwrap_or(&empty), &format!("{location}.steps"))?,
        observable_outcome: nonempty_string(
            field(item, "observableOutcome").unwrap_or(&empty),
            &format!("{location}.observableOutcome"),
        )?,
    })
}

fn parse_feature(value: &Json, location: &str) -> Result<Feature, ToolsError> {
    let item = object(value, location)?;
    let empty = Json::Null;
    let source_status = field(item, "sourceStatus").and_then(Json::as_str).unwrap_or_default();
    if source_status != "implemented" && source_status != "partial" && source_status != "missing" {
        return Err(ToolsError::invalid(format!("{location}.sourceStatus is invalid")));
    }
    if field(item, "targetStatus").and_then(Json::as_str) != Some("required") {
        return Err(ToolsError::invalid(format!("{location}.targetStatus must retain the required feature")));
    }
    let evidence =
        parse_array(field(item, "evidence").unwrap_or(&empty), &format!("{location}.evidence"), parse_evidence)?;
    let workflows =
        parse_array(field(item, "workflows").unwrap_or(&empty), &format!("{location}.workflows"), parse_workflow)?;
    let gaps = strings(field(item, "gaps").unwrap_or(&empty), &format!("{location}.gaps"))?;
    if source_status != "missing" && evidence.is_empty() {
        return Err(ToolsError::invalid(format!("{location} needs source evidence")));
    }
    if source_status != "implemented" && gaps.is_empty() {
        return Err(ToolsError::invalid(format!("{location} must describe the donor gap")));
    }
    if workflows.is_empty() {
        return Err(ToolsError::invalid(format!("{location} needs an exposed workflow")));
    }
    unique(&workflows.iter().map(|workflow| workflow.id.clone()).collect::<Vec<_>>(), &format!("{location}.workflows"))?;
    Ok(Feature {
        id: nonempty_string(field(item, "id").unwrap_or(&empty), &format!("{location}.id"))?,
        source_family_ids: nonempty_strings(
            field(item, "sourceFamilyIds").unwrap_or(&empty),
            &format!("{location}.sourceFamilyIds"),
        )?,
        unified_feature_ids: nonempty_strings(
            field(item, "unifiedFeatureIds").unwrap_or(&empty),
            &format!("{location}.unifiedFeatureIds"),
        )?,
        title: nonempty_string(field(item, "title").unwrap_or(&empty), &format!("{location}.title"))?,
        requirement: nonempty_string(field(item, "requirement").unwrap_or(&empty), &format!("{location}.requirement"))?,
        source_status: source_status.to_owned(),
        products: nonempty_strings(field(item, "products").unwrap_or(&empty), &format!("{location}.products"))?,
        owner_task_ids: nonempty_strings(field(item, "ownerTaskIds").unwrap_or(&empty), &format!("{location}.ownerTaskIds"))?,
        acceptance_case_ids: nonempty_strings(
            field(item, "acceptanceCaseIds").unwrap_or(&empty),
            &format!("{location}.acceptanceCaseIds"),
        )?,
        evidence,
        workflows,
        gaps,
    })
}

/// Parse one family's feature shard (donor `parseFeatureShard`).
pub fn parse_feature_shard(value: &Json, location: &str) -> Result<FeatureShard, ToolsError> {
    let item = object(value, location)?;
    let empty = Json::Null;
    if field(item, "schemaVersion").and_then(Json::as_f64) != Some(1.0) {
        return Err(ToolsError::invalid(format!("{location}.schemaVersion must equal 1")));
    }
    let family = field(item, "family").and_then(Json::as_str).unwrap_or_default();
    if family != "q1" && family != "q2" && family != "q3" {
        return Err(ToolsError::invalid(format!("{location}.family is invalid")));
    }
    let features =
        parse_array(field(item, "features").unwrap_or(&empty), &format!("{location}.features"), parse_feature)?;
    if features.is_empty() {
        return Err(ToolsError::invalid(format!("{location} cannot drop its features")));
    }
    unique(&features.iter().map(|feature| feature.id.clone()).collect::<Vec<_>>(), &format!("{location}.features"))?;
    if features.iter().any(|feature| !feature.id.starts_with(&format!("{family}."))) {
        return Err(ToolsError::invalid(format!("{location} contains a feature outside {family}")));
    }
    Ok(FeatureShard {
        family: family.to_owned(),
        source_revision: revision(field(item, "sourceRevision").unwrap_or(&empty), &format!("{location}.sourceRevision"))?,
        features,
    })
}

fn parse_indexed_function(value: &Json, location: &str) -> Result<IndexedFunction, ToolsError> {
    let item = object(value, location)?;
    let empty = Json::Null;
    Ok(IndexedFunction {
        id: nonempty_string(field(item, "id").unwrap_or(&empty), &format!("{location}.id"))?,
        name: nonempty_string(field(item, "name").unwrap_or(&empty), &format!("{location}.name"))?,
        line: positive_integer(field(item, "line").unwrap_or(&empty), &format!("{location}.line"))?,
        end_line: positive_integer(field(item, "endLine").unwrap_or(&empty), &format!("{location}.endLine"))?,
    })
}

fn parse_indexed_file(value: &Json, location: &str) -> Result<IndexedFile, ToolsError> {
    let item = object(value, location)?;
    let empty = Json::Null;
    let line_count = match field(item, "lineCount").unwrap_or(&empty).as_f64().and_then(safe_integer) {
        Some(count) => count,
        None => return Err(ToolsError::invalid(format!("{location}.lineCount is invalid"))),
    };
    Ok(IndexedFile {
        path: safe_relative_path(&nonempty_string(field(item, "path").unwrap_or(&empty), &format!("{location}.path"))?)?,
        kind: nonempty_string(field(item, "kind").unwrap_or(&empty), &format!("{location}.kind"))?,
        sha256: hash(field(item, "sha256").unwrap_or(&empty), &format!("{location}.sha256"))?,
        line_count,
        functions: parse_array(
            field(item, "functions").unwrap_or(&empty),
            &format!("{location}.functions"),
            parse_indexed_function,
        )?,
    })
}

fn parse_repository(value: &Json, location: &str) -> Result<IndexedRepository, ToolsError> {
    let item = object(value, location)?;
    let empty = Json::Null;
    Ok(IndexedRepository {
        id: nonempty_string(field(item, "id").unwrap_or(&empty), &format!("{location}.id"))?,
        path: nonempty_string(field(item, "path").unwrap_or(&empty), &format!("{location}.path"))?,
        revision: revision(field(item, "revision").unwrap_or(&empty), &format!("{location}.revision"))?,
        source_set_sha256: hash(
            field(item, "sourceSetSha256").unwrap_or(&empty),
            &format!("{location}.sourceSetSha256"),
        )?,
        files: parse_array(field(item, "files").unwrap_or(&empty), &format!("{location}.files"), parse_indexed_file)?,
    })
}

fn require_members(actual: &[String], expected: &HashSet<String>, location: &str) -> Result<(), ToolsError> {
    for id in actual {
        if !expected.contains(id) {
            return Err(ToolsError::invalid(format!("{location} refers to unknown {id}")));
        }
    }
    Ok(())
}

fn linked_evidence_json(evidence: &Evidence, revision: &str, sha256: &str, function_ids: &[String]) -> Json {
    let mut pairs = Vec::new();
    if let Json::Object(rows) = evidence.to_json() {
        pairs.extend(rows);
    }
    pairs.push(("revision".to_owned(), Json::string(revision)));
    pairs.push(("sha256".to_owned(), Json::string(sha256)));
    pairs.push(("functionIds".to_owned(), Json::array(function_ids.iter().map(Json::string).collect())));
    Json::object(pairs)
}

fn linked_feature_json(feature: &Feature, evidence: Vec<Json>) -> Json {
    Json::object(vec![
        ("id".to_owned(), Json::string(&feature.id)),
        ("sourceFamilyIds".to_owned(), Json::array(feature.source_family_ids.iter().map(Json::string).collect())),
        ("unifiedFeatureIds".to_owned(), Json::array(feature.unified_feature_ids.iter().map(Json::string).collect())),
        ("title".to_owned(), Json::string(&feature.title)),
        ("requirement".to_owned(), Json::string(&feature.requirement)),
        ("sourceStatus".to_owned(), Json::string(&feature.source_status)),
        ("targetStatus".to_owned(), Json::string("required")),
        ("products".to_owned(), Json::array(feature.products.iter().map(Json::string).collect())),
        ("ownerTaskIds".to_owned(), Json::array(feature.owner_task_ids.iter().map(Json::string).collect())),
        ("acceptanceCaseIds".to_owned(), Json::array(feature.acceptance_case_ids.iter().map(Json::string).collect())),
        ("evidence".to_owned(), Json::array(evidence)),
        ("workflows".to_owned(), Json::array(feature.workflows.iter().map(Workflow::to_json).collect())),
        ("gaps".to_owned(), Json::array(feature.gaps.iter().map(Json::string).collect())),
        ("acceptanceState".to_owned(), Json::string("not-run")),
    ])
}

/// Build the feature ledger for a project root (donor `buildFeatureLedger`).
pub fn build_feature_ledger(root: &Path) -> Result<Json, ToolsError> {
    let source_text = fsutil::read_text(&root.join("verification/source-manifest.json"))?;
    let source_value = parse_json(&source_text)?;
    let source = object(&source_value, "source-manifest")?;
    let empty = Json::Null;
    if field(source, "schemaVersion").unwrap_or(&empty).as_f64() != Some(1.0) {
        return Err(ToolsError::invalid("source-manifest.schemaVersion must equal 1"));
    }
    let repositories = parse_array(
        field(source, "repositories").unwrap_or(&empty),
        "source-manifest.repositories",
        parse_repository,
    )?;
    unique(&repositories.iter().map(|repository| repository.id.clone()).collect::<Vec<_>>(), "source-manifest.repositories")?;
    let repository_index: HashMap<&str, &IndexedRepository> =
        repositories.iter().map(|repository| (repository.id.as_str(), repository)).collect();
    let mut file_index: HashMap<String, &IndexedFile> = HashMap::new();
    for repository in &repositories {
        unique(
            &repository.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            &format!("{}.files", repository.id),
        )?;
        for file in &repository.files {
            file_index.insert(format!("{}:{}", repository.id, file.path), file);
        }
    }
    let graph_text = fsutil::read_text(&root.join("docs/work-packages.json"))?;
    let graph_value = parse_json(&graph_text)?;
    let graph = object(&graph_value, "work-packages")?;
    let task_ids: HashSet<String> = parse_array(field(graph, "tasks").unwrap_or(&empty), "work-packages.tasks", |value, location| {
        nonempty_string(field(object(value, location)?, "id").unwrap_or(&empty), &format!("{location}.id"))
    })?
    .into_iter()
    .collect();
    let source_family_ids: HashSet<String> =
        strings(field(graph, "sourceFeatureIds").unwrap_or(&empty), "work-packages.sourceFeatureIds")?
            .into_iter()
            .collect();
    let feature_coverage = parse_array(
        field(graph, "featureCoverage").unwrap_or(&empty),
        "work-packages.featureCoverage",
        |value, location| object(value, location).map(|_| value.clone()),
    )?;
    let mut unified_feature_ids = HashSet::new();
    for row in &feature_coverage {
        let entries = object(row, "featureCoverage.id")?;
        unified_feature_ids.insert(nonempty_string(field(entries, "id").unwrap_or(&empty), "featureCoverage.id")?);
    }
    let product_text = fsutil::read_text(&root.join("verification/product-manifest.json"))?;
    let product_value = parse_json(&product_text)?;
    let product_root = object(&product_value, "product-manifest")?;
    let product_ids: HashSet<String> = parse_array(
        field(product_root, "products").unwrap_or(&empty),
        "product-manifest.products",
        |value, location| nonempty_string(field(object(value, location)?, "id").unwrap_or(&empty), &format!("{location}.id")),
    )?
    .into_iter()
    .collect();
    let mut shard_inputs = Vec::new();
    let mut features = Vec::new();
    for family in ["q1", "q2", "q3"] {
        let path = format!("verification/features/{family}.json");
        let input = fsutil::read_text(&root.join(&path))?;
        let shard = parse_feature_shard(&parse_json(&input)?, &path)?;
        let repository = repository_index.get(format!("{family}-ts").as_str());
        let pinned = repository.is_some_and(|repository| shard.source_revision == repository.revision);
        if shard.family != family || !pinned {
            return Err(ToolsError::invalid(format!("{path} does not match its pinned donor source")));
        }
        shard_inputs.push(Json::object(vec![
            ("path".to_owned(), Json::string(&path)),
            ("sha256".to_owned(), Json::string(hash_hex(input.as_bytes()))),
        ]));
        features.extend(shard.features);
    }
    features.sort_by(|left, right| compare_text(&left.id, &right.id));
    unique(&features.iter().map(|feature| feature.id.clone()).collect::<Vec<_>>(), "features")?;
    let mut linked: HashMap<&str, HashSet<&str>> = HashMap::new();
    let mut checked_files: HashSet<String> = HashSet::new();
    let mut enriched = Vec::new();
    for feature in &features {
        require_members(&feature.source_family_ids, &source_family_ids, &format!("{}.sourceFamilyIds", feature.id))?;
        require_members(&feature.unified_feature_ids, &unified_feature_ids, &format!("{}.unifiedFeatureIds", feature.id))?;
        require_members(&feature.owner_task_ids, &task_ids, &format!("{}.ownerTaskIds", feature.id))?;
        require_members(&feature.products, &product_ids, &format!("{}.products", feature.id))?;
        let mut evidence_rows = Vec::new();
        for anchor in &feature.evidence {
            let key = format!("{}:{}", anchor.repository, anchor.path);
            let repository = repository_index.get(anchor.repository.as_str());
            let file = file_index.get(&key);
            let (Some(repository), Some(file)) = (repository, file) else {
                return Err(ToolsError::invalid(format!("{}: unpinned evidence {key}", feature.id)));
            };
            if anchor.end_line > file.line_count {
                return Err(ToolsError::invalid(format!(
                    "{}: evidence exceeds {}'s {} lines",
                    feature.id, key, file.line_count
                )));
            }
            if !checked_files.contains(&key) {
                let actual = fsutil::read_bytes(&root.join(&repository.path).join(&anchor.path))?;
                if hash_hex(&actual) != file.sha256 {
                    return Err(ToolsError::invalid(format!(
                        "{}: evidence changed since source capture: {key}",
                        feature.id
                    )));
                }
                checked_files.insert(key.clone());
            }
            let matches: Vec<&IndexedFunction> = file
                .functions
                .iter()
                .filter(|function| {
                    function.line <= anchor.end_line
                        && function.end_line >= anchor.line
                        && (anchor.symbol.is_none() || anchor.symbol.as_deref() == Some(function.name.as_str()))
                })
                .collect();
            if anchor.symbol.is_some() && (anchor.path.ends_with(".ts") || anchor.path.ends_with(".tsx")) && matches.is_empty() {
                return Err(ToolsError::invalid(format!(
                    "{}: symbol {} does not match {key}:{}-{}",
                    feature.id,
                    anchor.symbol.as_deref().unwrap_or_default(),
                    anchor.line,
                    anchor.end_line
                )));
            }
            for function in &matches {
                linked.entry(function.id.as_str()).or_default().insert(feature.id.as_str());
            }
            evidence_rows.push(linked_evidence_json(
                anchor,
                &repository.revision,
                &file.sha256,
                &matches.iter().map(|function| function.id.clone()).collect::<Vec<_>>(),
            ));
        }
        enriched.push(linked_feature_json(feature, evidence_rows));
    }
    let covered_source: HashSet<&str> =
        features.iter().flat_map(|feature| feature.source_family_ids.iter().map(String::as_str)).collect();
    let covered_unified: HashSet<&str> =
        features.iter().flat_map(|feature| feature.unified_feature_ids.iter().map(String::as_str)).collect();
    let source_family_list: Vec<String> = {
        let mut ids: Vec<String> = source_family_ids.iter().cloned().collect();
        ids.sort_by(|left, right| compare_text(left, right));
        ids
    };
    let unified_list: Vec<String> = {
        let mut ids: Vec<String> = unified_feature_ids.iter().cloned().collect();
        ids.sort_by(|left, right| compare_text(left, right));
        ids
    };
    let covered_source_owned: HashSet<String> = covered_source.iter().map(ToString::to_string).collect();
    let covered_unified_owned: HashSet<String> = covered_unified.iter().map(ToString::to_string).collect();
    require_members(&source_family_list, &covered_source_owned, "source-family coverage")?;
    require_members(&unified_list, &covered_unified_owned, "unified-feature coverage")?;
    let mut function_repositories = Vec::new();
    let mut unassigned_runtime = 0_u64;
    for repository in &repositories {
        let mut files = Vec::new();
        let mut runtime = 0_u64;
        let mut linked_runtime = 0_u64;
        let mut total = 0_u64;
        for file in repository.files.iter().filter(|file| !file.functions.is_empty()) {
            let mut functions = Vec::new();
            for function in &file.functions {
                let mut feature_ids: Vec<&str> = linked.get(function.id.as_str()).map_or(Vec::new(), |owners| owners.iter().copied().collect());
                feature_ids.sort_by(|left, right| compare_text(left, right));
                if file.kind == "runtime" {
                    runtime += 1;
                    if feature_ids.is_empty() {
                        unassigned_runtime += 1;
                    } else {
                        linked_runtime += 1;
                    }
                }
                total += 1;
                functions.push(Json::object(vec![
                    ("id".to_owned(), Json::string(&function.id)),
                    ("featureIds".to_owned(), Json::array(feature_ids.iter().copied().map(Json::string).collect())),
                ]));
            }
            files.push(Json::object(vec![
                ("path".to_owned(), Json::string(&file.path)),
                ("kind".to_owned(), Json::string(&file.kind)),
                ("functions".to_owned(), Json::array(functions)),
            ]));
        }
        function_repositories.push(Json::object(vec![
            ("repository".to_owned(), Json::string(&repository.id)),
            ("sourceSetSha256".to_owned(), Json::string(&repository.source_set_sha256)),
            ("totalFunctions".to_owned(), Json::uint(total)),
            ("runtimeFunctions".to_owned(), Json::uint(runtime)),
            ("linkedRuntimeFunctions".to_owned(), Json::uint(linked_runtime)),
            ("unassignedRuntimeFunctions".to_owned(), Json::uint(runtime - linked_runtime)),
            ("files".to_owned(), Json::array(files)),
        ]));
    }
    let workflows_total: u64 = features.iter().map(|feature| feature.workflows.len() as u64).sum();
    let acceptance_total: HashSet<&str> =
        features.iter().flat_map(|feature| feature.acceptance_case_ids.iter().map(String::as_str)).collect();
    Ok(Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("generator".to_owned(), Json::string("tools/inventory/aggregate-features.ts")),
        ("status".to_owned(), Json::string("required-features-inventoried; implementation-and-acceptance-not-established")),
        (
            "inputs".to_owned(),
            Json::object(vec![
                (
                    "sourceManifest".to_owned(),
                    Json::object(vec![
                        ("path".to_owned(), Json::string("verification/source-manifest.json")),
                        ("sha256".to_owned(), Json::string(hash_hex(source_text.as_bytes()))),
                    ]),
                ),
                (
                    "productManifest".to_owned(),
                    Json::object(vec![
                        ("path".to_owned(), Json::string("verification/product-manifest.json")),
                        ("sha256".to_owned(), Json::string(hash_hex(product_text.as_bytes()))),
                    ]),
                ),
                ("featureShards".to_owned(), Json::array(shard_inputs)),
            ]),
        ),
        (
            "summary".to_owned(),
            Json::object(vec![
                ("features".to_owned(), Json::uint(features.len() as u64)),
                ("sourceFamilies".to_owned(), Json::uint(covered_source.len() as u64)),
                ("unifiedFeatures".to_owned(), Json::uint(covered_unified.len() as u64)),
                ("workflows".to_owned(), Json::uint(workflows_total)),
                ("expectedAcceptanceCases".to_owned(), Json::uint(acceptance_total.len() as u64)),
                (
                    "implementedDonorFeatures".to_owned(),
                    Json::uint(features.iter().filter(|feature| feature.source_status == "implemented").count() as u64),
                ),
                (
                    "partialDonorFeatures".to_owned(),
                    Json::uint(features.iter().filter(|feature| feature.source_status == "partial").count() as u64),
                ),
                (
                    "missingDonorFeatures".to_owned(),
                    Json::uint(features.iter().filter(|feature| feature.source_status == "missing").count() as u64),
                ),
                ("unassignedRuntimeFunctions".to_owned(), Json::uint(unassigned_runtime)),
                ("acceptedTargetFeatures".to_owned(), Json::uint(0)),
            ]),
        ),
        ("features".to_owned(), Json::array(enriched)),
        (
            "functionAccounting".to_owned(),
            Json::object(vec![
                (
                    "method".to_owned(),
                    Json::string("Every TypeScript function is retained. Direct links require a matching evidence file and overlapping line range, plus an exact symbol when specified."),
                ),
                (
                    "limits".to_owned(),
                    Json::array(
                        [
                            "A source anchor identifies relevant implementation; it does not prove the complete call path or passing behavior.",
                            "Unassigned runtime functions remain required census follow-up. An empty featureIds array never means discarded functionality.",
                            "Test and tool functions remain inventoried separately from runtime requirements. Original-language function semantics require original-contract evidence and independent baselines.",
                            "Donor implementation status is separate from target acceptance. Every target feature remains required and not-run.",
                        ]
                        .into_iter()
                        .map(Json::string)
                        .collect(),
                    ),
                ),
                ("repositories".to_owned(), Json::array(function_repositories)),
            ]),
        ),
    ]))
}

/// Aggregate the ledger and write or verify it (donor `import.meta.main`).
pub fn run(args: &[String]) -> Result<(), ToolsError> {
    let options = inventory_arguments(args, &quake_typescript_root())?;
    let ledger = build_feature_ledger(&options.root)?;
    let text = format!("{}\n", ledger.render_pretty());
    let destination = options.root.join("verification/feature-ledger.json");
    write_or_check(&destination, &text, options.check)?;
    let empty = Json::Null;
    let summary = ledger.get("summary").unwrap_or(&empty);
    let number = |key: &str| summary.get(key).and_then(Json::as_f64).unwrap_or(0.0) as u64;
    println!(
        "{} verification/feature-ledger.json: {} required features, {} workflows, {} runtime functions awaiting direct feature links. No target acceptance claimed.",
        if options.check { "Verified" } else { "Wrote" },
        number("features"),
        number("workflows"),
        number("unassignedRuntimeFunctions"),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
    const SOURCE: &str = "line one\nline two\nline three\n";

    fn source_manifest(sha: &str) -> String {
        format!(
            "{{\"schemaVersion\": 1, \"repositories\": [{{\"id\": \"q1-ts\", \"path\": \"repo\", \"revision\": \"{REVISION}\", \
             \"sourceSetSha256\": \"{}\", \"files\": [{{\"path\": \"src/a.ts\", \"kind\": \"runtime\", \"sha256\": \"{sha}\", \
             \"lineCount\": 3, \"functions\": [{{\"id\": \"q1-ts:src/a.ts#0\", \"name\": \"add\", \"line\": 1, \"endLine\": 3}}, \
             {{\"id\": \"q1-ts:src/a.ts#50\", \"name\": \"lonely\", \"line\": 3, \"endLine\": 3}}]}}]}}, \
             {{\"id\": \"q2-ts\", \"path\": \"repo\", \"revision\": \"{REVISION}\", \"sourceSetSha256\": \"{}\", \"files\": []}}, \
             {{\"id\": \"q3-ts\", \"path\": \"repo\", \"revision\": \"{REVISION}\", \"sourceSetSha256\": \"{}\", \"files\": []}}]}}",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
        )
    }

    fn feature(id: &str, status: &str, evidence: &str, gaps: &str, cases: &str) -> String {
        format!(
            "{{\"id\": \"{id}\", \"sourceFamilyIds\": [\"S1\"], \"unifiedFeatureIds\": [\"U1\"], \"title\": \"t\", \
             \"requirement\": \"r\", \"sourceStatus\": \"{status}\", \"targetStatus\": \"required\", \"products\": [\"P1\"], \
             \"ownerTaskIds\": [\"T1\"], \"acceptanceCaseIds\": {cases}, \"evidence\": {evidence}, \
             \"workflows\": [{{\"id\": \"w\", \"trigger\": \"t\", \"steps\": [\"s\"], \"observableOutcome\": \"o\"}}], \"gaps\": {gaps}}}"
        )
    }

    fn shard(family: &str, features: &str) -> String {
        format!("{{\"schemaVersion\": 1, \"family\": \"{family}\", \"sourceRevision\": \"{REVISION}\", \"features\": {features}}}")
    }

    fn fixture() -> PathBuf {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "qa-aggregate-features-").expect("temp dir");
        let evidence =
            "{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": \"add\", \"line\": 1, \"endLine\": 2, \
             \"role\": \"implementation\", \"observation\": \"o\"}";
        let files = [
            ("verification/source-manifest.json", source_manifest(&hash_hex(SOURCE.as_bytes()))),
            ("docs/work-packages.json", "{\"tasks\": [{\"id\": \"T1\"}], \"sourceFeatureIds\": [\"S1\"], \"featureCoverage\": [{\"id\": \"U1\"}]}".to_owned()),
            ("verification/product-manifest.json", "{\"products\": [{\"id\": \"P1\"}]}".to_owned()),
            ("verification/features/q1.json", shard("q1", &format!("[{}]", feature("q1.f1", "implemented", &format!("[{evidence}]"), "[]", "[\"A1\"]")))),
            ("verification/features/q2.json", shard("q2", &format!("[{}]", feature("q2.f9", "missing", "[]", "[\"g\"]", "[\"A2\"]")))),
            ("verification/features/q3.json", shard("q3", &format!("[{}]", feature("q3.f9", "missing", "[]", "[\"g\"]", "[\"A1\"]")))),
            ("repo/src/a.ts", SOURCE.to_owned()),
        ];
        for (path, text) in files {
            let destination = directory.join(path);
            std::fs::create_dir_all(destination.parent().expect("parent")).expect("mkdir");
            std::fs::write(destination, text).expect("write");
        }
        directory
    }

    fn build_fixture(mutate: impl FnOnce(&PathBuf)) -> Result<Json, ToolsError> {
        let directory = fixture();
        mutate(&directory);
        let outcome = build_feature_ledger(&directory);
        fsutil::remove_forced(&directory);
        outcome
    }

    fn write(directory: &PathBuf, path: &str, text: &str) {
        std::fs::write(directory.join(path), text).expect("rewrite");
    }

    #[test]
    fn aggregates_ledger() {
        let directory = fixture();
        let ledger = build_feature_ledger(&directory).expect("ledger");
        fsutil::remove_forced(&directory);
        let empty = Json::Null;
        let summary = ledger.get("summary").unwrap_or(&empty);
        let number = |key: &str| summary.get(key).and_then(Json::as_f64).unwrap_or(-1.0);
        assert_eq!(number("features"), 3.0);
        assert_eq!(number("sourceFamilies"), 1.0);
        assert_eq!(number("unifiedFeatures"), 1.0);
        assert_eq!(number("workflows"), 3.0);
        assert_eq!(number("expectedAcceptanceCases"), 2.0);
        assert_eq!(number("implementedDonorFeatures"), 1.0);
        assert_eq!(number("partialDonorFeatures"), 0.0);
        assert_eq!(number("missingDonorFeatures"), 2.0);
        assert_eq!(number("unassignedRuntimeFunctions"), 1.0);
        assert_eq!(number("acceptedTargetFeatures"), 0.0);
        let features = ledger.get("features").and_then(Json::as_array).expect("features");
        assert_eq!(features[0].get("id").and_then(Json::as_str), Some("q1.f1"));
        assert_eq!(features[0].get("acceptanceState").and_then(Json::as_str), Some("not-run"));
        let anchors = features[0].get("evidence").and_then(Json::as_array).expect("evidence");
        assert_eq!(anchors[0].get("revision").and_then(Json::as_str), Some(REVISION));
        assert_eq!(anchors[0].get("functionIds").and_then(Json::as_array).map(<[Json]>::len), Some(1));
        let accounting =
            ledger.get("functionAccounting").and_then(|value| value.get("repositories")).and_then(Json::as_array).expect("accounting");
        assert_eq!(accounting[0].get("totalFunctions").and_then(Json::as_f64), Some(2.0));
        assert_eq!(accounting[0].get("linkedRuntimeFunctions").and_then(Json::as_f64), Some(1.0));
        assert_eq!(accounting[0].get("unassignedRuntimeFunctions").and_then(Json::as_f64), Some(1.0));
    }

    #[test]
    fn rejects_changed_evidence() {
        let error = build_fixture(|directory| write(directory, "repo/src/a.ts", "tampered\n")).expect_err("changed");
        assert!(error.to_string().contains("evidence changed since source capture"), "{error}");
    }

    #[test]
    fn rejects_evidence_beyond_file() {
        let error = build_fixture(|directory| {
            let evidence =
                "{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": null, \"line\": 1, \"endLine\": 9, \
                 \"role\": \"implementation\", \"observation\": \"o\"}";
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "implemented", &format!("[{evidence}]"), "[]", "[\"A1\"]"))));
        })
        .expect_err("range");
        assert!(error.to_string().contains("evidence exceeds"), "{error}");
    }

    #[test]
    fn rejects_unpinned_evidence() {
        let error = build_fixture(|directory| {
            let evidence =
                "{\"repository\": \"nope\", \"path\": \"src/a.ts\", \"symbol\": null, \"line\": 1, \"endLine\": 1, \
                 \"role\": \"implementation\", \"observation\": \"o\"}";
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "implemented", &format!("[{evidence}]"), "[]", "[\"A1\"]"))));
        })
        .expect_err("unpinned");
        assert!(error.to_string().contains("unpinned evidence"), "{error}");
    }

    #[test]
    fn rejects_unmatched_symbol() {
        let error = build_fixture(|directory| {
            let evidence =
                "{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": \"zzz\", \"line\": 1, \"endLine\": 1, \
                 \"role\": \"implementation\", \"observation\": \"o\"}";
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "implemented", &format!("[{evidence}]"), "[]", "[\"A1\"]"))));
        })
        .expect_err("symbol");
        assert!(error.to_string().contains("symbol zzz does not match"), "{error}");
    }

    #[test]
    fn allows_symbol_free_or_untyped_misses() {
        build_fixture(|directory| {
            let free =
                "{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": null, \"line\": 3, \"endLine\": 3, \
                 \"role\": \"test\", \"observation\": \"o\"}";
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "implemented", &format!("[{free}]"), "[]", "[\"A1\"]"))));
        })
        .expect("symbol-free");
    }

    #[test]
    fn rejects_unpinned_shard() {
        let error = build_fixture(|directory| {
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]")))
                .replace(REVISION, &"0".repeat(40)));
        })
        .expect_err("pin");
        assert!(error.to_string().contains("does not match its pinned donor source"), "{error}");
    }

    #[test]
    fn rejects_unknown_members_and_gaps() {
        let error = build_fixture(|directory| {
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]")))
                .replace("\"P1\"", "\"PX\""));
        })
        .expect_err("product");
        assert!(error.to_string().contains("refers to unknown PX"), "{error}");
        let error = build_fixture(|directory| {
            write(directory, "docs/work-packages.json", "{\"tasks\": [{\"id\": \"T1\"}], \"sourceFeatureIds\": [\"S1\", \"S2\"], \"featureCoverage\": [{\"id\": \"U1\"}]}");
        })
        .expect_err("coverage");
        assert!(error.to_string().contains("source-family coverage refers to unknown S2"), "{error}");
        let error = build_fixture(|directory| {
            let evidence =
                "{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": null, \"line\": 1, \"endLine\": 1, \
                 \"role\": \"implementation\", \"observation\": \"o\"}";
            write(directory, "verification/features/q1.json", &shard("q1", &format!("[{}]", feature("q1.f1", "partial", &format!("[{evidence}]"), "[]", "[\"A1\"]"))));
        })
        .expect_err("gap");
        assert!(error.to_string().contains("must describe the donor gap"), "{error}");
    }

    #[test]
    fn validates_documents() {
        let shard_value = parse_json(&shard("q1", &format!("[{}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]")))).expect("parse");
        assert!(parse_feature_shard(&shard_value, "shard").is_ok());
        for (text, message) in [
            (shard("q1", "[]"), "cannot drop its features"),
            (shard("q1", &format!("[{}, {}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"), feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"))), "contains duplicates"),
            (shard("q1", &format!("[{}]", feature("q2.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"))), "contains a feature outside q1"),
            (shard("q9", &format!("[{}]", feature("q9.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"))), "family is invalid"),
        ] {
            let error = parse_feature_shard(&parse_json(&text).expect("parse"), "shard").expect_err("shard");
            assert!(error.to_string().contains(message), "{error}");
        }
        let bad_version = shard("q1", &format!("[{}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"))).replace("\"schemaVersion\": 1", "\"schemaVersion\": 2");
        let error = parse_feature_shard(&parse_json(&bad_version).expect("parse"), "shard").expect_err("version");
        assert!(error.to_string().contains("schemaVersion must equal 1"), "{error}");
        for (from, to, message) in [
            ("\"sourceStatus\": \"missing\"", "\"sourceStatus\": \"done\"", "sourceStatus is invalid"),
            ("\"targetStatus\": \"required\"", "\"targetStatus\": \"optional\"", "targetStatus must retain the required feature"),
        ] {
            let text = shard("q1", &format!("[{}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"))).replace(from, to);
            let error = parse_feature_shard(&parse_json(&text).expect("parse"), "shard").expect_err("field");
            assert!(error.to_string().contains(message), "{error}");
        }
        let bad_role = shard("q1", &format!("[{}]", feature("q1.f1", "implemented", "[{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": null, \"line\": 1, \"endLine\": 1, \"role\": \"nope\", \"observation\": \"o\"}]", "[]", "[\"A1\"]")));
        let error = parse_feature_shard(&parse_json(&bad_role).expect("parse"), "shard").expect_err("role");
        assert!(error.to_string().contains("role is invalid"), "{error}");
        let missing_symbol = shard("q1", &format!("[{}]", feature("q1.f1", "implemented", "[{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"line\": 1, \"endLine\": 1, \"role\": \"test\", \"observation\": \"o\"}]", "[]", "[\"A1\"]")));
        let error = parse_feature_shard(&parse_json(&missing_symbol).expect("parse"), "shard").expect_err("symbol");
        assert!(error.to_string().contains("symbol must be a nonempty string"), "{error}");
        let reversed = shard("q1", &format!("[{}]", feature("q1.f1", "implemented", "[{\"repository\": \"q1-ts\", \"path\": \"src/a.ts\", \"symbol\": null, \"line\": 2, \"endLine\": 1, \"role\": \"test\", \"observation\": \"o\"}]", "[]", "[\"A1\"]")));
        let error = parse_feature_shard(&parse_json(&reversed).expect("parse"), "shard").expect_err("range");
        assert!(error.to_string().contains("has a reversed line range"), "{error}");
        let no_workflows = shard("q1", &format!("[{}]", feature("q1.f1", "missing", "[]", "[\"g\"]", "[\"A1\"]"))).replace(", \"workflows\": [{\"id\": \"w\", \"trigger\": \"t\", \"steps\": [\"s\"], \"observableOutcome\": \"o\"}]", ", \"workflows\": []");
        let error = parse_feature_shard(&parse_json(&no_workflows).expect("parse"), "shard").expect_err("workflows");
        assert!(error.to_string().contains("needs an exposed workflow"), "{error}");
        let no_evidence = shard("q1", &format!("[{}]", feature("q1.f1", "implemented", "[]", "[]", "[\"A1\"]")));
        let error = parse_feature_shard(&parse_json(&no_evidence).expect("parse"), "shard").expect_err("evidence");
        assert!(error.to_string().contains("needs source evidence"), "{error}");
    }

    #[test]
    fn validates_scalars() {
        assert!(nonempty_string(&parse_json("\" x \"").expect("parse"), "here").is_ok());
        for text in ["\"  \"", "\"\\u00e9\"", "5", "null"] {
            if text == "\"\\u00e9\"" {
                assert!(nonempty_string(&parse_json(text).expect("parse"), "here").is_ok());
            } else {
                assert!(nonempty_string(&parse_json(text).expect("parse"), "here").is_err());
            }
        }
        assert_eq!(positive_integer(&parse_json("3").expect("parse"), "here").expect("int"), 3);
        for text in ["0", "-1", "1.5", "\"3\"", "9007199254740992"] {
            assert!(positive_integer(&parse_json(text).expect("parse"), "here").is_err(), "{text}");
        }
        assert!(hash(&parse_json(&format!("\"{}\"", "a".repeat(64))).expect("parse"), "here").is_ok());
        assert!(hash(&parse_json(&format!("\"{}\"", "A".repeat(64))).expect("parse"), "here").is_err());
        assert!(revision(&parse_json(&format!("\"{}\"", "b".repeat(40))).expect("parse"), "here").is_ok());
        assert!(revision(&parse_json("\"short\"").expect("parse"), "here").is_err());
    }
}
