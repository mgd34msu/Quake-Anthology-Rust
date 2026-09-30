//! Integration-run merger (donor `tools/inventory/merge-integration-candidates.ts`).
//!
//! Merges the per-game integration runs into one combined document: feature
//! bytes must agree across games, declaration and inventory joins union in
//! first-seen order, every game must retain all 477 feature IDs, and the
//! pinned comparison documents are hashed from git. Feature-byte equality
//! compares compact JSON renders, matching the donor's `JSON.stringify`
//! comparison over parsed values.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{parse_json, Json};
use crate::reference::environment::quake_typescript_root;
use crate::verify::hash::hash_bytes;

/// Expected feature IDs retained by every game run.
const FEATURE_COUNT: usize = 477;

/// An insertion-ordered string set (donor `Set` spread order).
#[derive(Debug, Clone, Default)]
struct OrderedSet {
    values: Vec<String>,
    seen: HashSet<String>,
}

impl OrderedSet {
    fn insert(&mut self, value: String) {
        if self.seen.insert(value.clone()) {
            self.values.push(value);
        }
    }

    fn to_json(&self) -> Json {
        Json::array(self.values.iter().map(Json::string).collect())
    }
}

/// A merged feature: first-seen bytes plus unioned joins.
#[derive(Debug, Clone)]
struct MergedFeature {
    declaration_ids: OrderedSet,
    inventories: OrderedSet,
    feature: Json,
}

impl MergedFeature {
    fn to_json(&self) -> Json {
        let mut pairs = Vec::new();
        if let Json::Object(feature) = &self.feature {
            pairs.extend(feature.iter().cloned());
        }
        pairs.push(("declarationIds".to_owned(), self.declaration_ids.to_json()));
        pairs.push(("inventories".to_owned(), self.inventories.to_json()));
        Json::object(pairs)
    }
}

pub(crate) fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, ToolsError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| ToolsError::command(format!("spawning git in {}: {error}", root.display())))?;
    if !output.status.success() {
        return Err(ToolsError::command(format!(
            "Git failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn read_json(path: &Path) -> Result<Json, ToolsError> {
    parse_json(&fsutil::read_text(path)?)
}

fn object<'a>(value: &'a Json, message: &str) -> Result<&'a [(String, Json)], ToolsError> {
    value.as_object().ok_or_else(|| ToolsError::invalid(message.to_owned()))
}

fn array<'a>(value: &'a Json, message: &str) -> Result<&'a [Json], ToolsError> {
    value.as_array().ok_or_else(|| ToolsError::invalid(message.to_owned()))
}

fn field<'a>(entries: &'a [(String, Json)], key: &str) -> Result<&'a Json, ToolsError> {
    entries
        .iter()
        .find(|(present, _)| present == key)
        .map(|(_, found)| found)
        .ok_or_else(|| ToolsError::invalid(format!("Missing field {key}")))
}

fn text(value: &Json) -> Result<String, ToolsError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ToolsError::invalid("Expected string"))
}

/// Merge per-game runs under `input` into `output/combined.json`.
pub fn merge_runs(input: &Path, output: &Path) -> Result<String, ToolsError> {
    let mut features: Vec<MergedFeature> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();
    let mut runs = Vec::new();
    let mut revision: Option<String> = None;
    for game in ["q1", "q2", "q3"] {
        let directory = input.join(format!("quake-integration-{game}"));
        let summary = read_json(&directory.join("summary.json"))?;
        let entries = object(&summary, "Invalid summary")?;
        let current = text(field(entries, "unifiedRevision")?)?;
        if revision.as_ref().is_some_and(|pinned| *pinned != current) {
            return Err(ToolsError::invalid("Unified snapshots differ"));
        }
        revision = Some(current);
        let joined = read_json(&directory.join("feature-join.json"))?;
        let joined_entries = object(&joined, "Invalid feature join")?;
        let inventories = array(
            field(joined_entries, "inventories").map_err(|_| ToolsError::invalid("Invalid feature join"))?,
            "Invalid feature join",
        )?;
        let mut seen = HashSet::new();
        for inventory in inventories {
            let inventory_entries = object(inventory, "Invalid inventory")?;
            let joins = array(
                field(inventory_entries, "joins").map_err(|_| ToolsError::invalid("Invalid inventory"))?,
                "Invalid inventory",
            )?;
            let inner = object(
                field(inventory_entries, "inventory").map_err(|_| ToolsError::invalid("Invalid inventory"))?,
                "Invalid inventory",
            )?;
            let raw_list = array(
                field(inner, "features").map_err(|_| ToolsError::invalid("Invalid inventory"))?,
                "Invalid inventory",
            )?;
            let mut raw_features = HashMap::new();
            for feature in raw_list {
                let feature_entries = object(feature, "Invalid feature")?;
                raw_features.insert(text(field(feature_entries, "id")?)?, feature.clone());
            }
            for join in joins {
                let join_entries = object(join, "Invalid join")?;
                let declarations = array(
                    field(join_entries, "declarationIds").map_err(|_| ToolsError::invalid("Invalid join"))?,
                    "Invalid join",
                )?;
                let id = text(field(join_entries, "id")?)?;
                if !seen.insert(id.clone()) {
                    return Err(ToolsError::invalid(format!("Duplicate feature {id} in {game}")));
                }
                let raw = raw_features
                    .get(&id)
                    .ok_or_else(|| ToolsError::invalid(format!("Missing raw feature {id}")))?;
                let index = *indexes.get(&id).unwrap_or(&features.len());
                if index == features.len() {
                    indexes.insert(id.clone(), index);
                    features.push(MergedFeature {
                        declaration_ids: OrderedSet::default(),
                        inventories: OrderedSet::default(),
                        feature: raw.clone(),
                    });
                }
                if features[index].feature.render() != raw.render() {
                    return Err(ToolsError::invalid(format!("Feature bytes disagree: {id}")));
                }
                for declaration in declarations {
                    features[index].declaration_ids.insert(text(declaration)?);
                }
                let path_value = inventory_entries
                    .iter()
                    .find(|(present, _)| present == "path")
                    .map(|(_, found)| found);
                features[index]
                    .inventories
                    .insert(text(path_value.unwrap_or(&Json::Null))?);
            }
        }
        if seen.len() != FEATURE_COUNT {
            return Err(ToolsError::invalid(format!(
                "{game} did not retain all {FEATURE_COUNT} existing feature IDs"
            )));
        }
        let mut artifacts = Vec::new();
        for name in [
            "inventory.json",
            "candidates.json",
            "summary.json",
            "feature-join.json",
            "anchor-inventory.json",
            "historical-completion.json",
        ] {
            let path = directory.join(name);
            artifacts.push(Json::object(vec![
                ("path".to_owned(), Json::string(path.to_string_lossy())),
                (
                    "sha256".to_owned(),
                    Json::string(hash_bytes(&fsutil::read_bytes(&path)?)),
                ),
            ]));
        }
        runs.push(Json::object(vec![
            ("game".to_owned(), Json::string(game)),
            ("summary".to_owned(), summary),
            ("artifacts".to_owned(), Json::array(artifacts)),
        ]));
    }
    let revision = revision.ok_or_else(|| ToolsError::invalid("No snapshots"))?;
    let root = quake_typescript_root();
    let mut comparison_docs = Vec::new();
    for game in ["q1", "q2", "q3"] {
        let path = format!("docs/comparison-{game}.md");
        let reference = format!("{revision}:{path}");
        let bytes = git_bytes(&root, &["show", &reference])
            .map_err(|_| ToolsError::invalid(format!("Missing pinned comparison {path}")))?;
        comparison_docs.push(Json::object(vec![
            ("path".to_owned(), Json::string(&path)),
            ("revision".to_owned(), Json::string(&revision)),
            ("sha256".to_owned(), Json::string(hash_bytes(&bytes))),
        ]));
    }
    std::fs::create_dir_all(output).map_err(|error| ToolsError::io(format!("creating {}", output.display()), error))?;
    let result = Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("unifiedRevision".to_owned(), Json::string(&revision)),
        (
            "limits".to_owned(),
            Json::array(
                [
                    "Candidate groups remain complete in the referenced hashed artifacts.",
                    "Historical verdicts remain separate in historical-completion.json; no completion totals are changed.",
                    "Declaration joins are lexical source-line overlaps, not behavior verdicts.",
                    "Comparison documents are pinned source context, not newly verified claims.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
        ("comparisonDocs".to_owned(), Json::array(comparison_docs)),
        ("runs".to_owned(), Json::array(runs)),
        ("features".to_owned(), Json::array(features.iter().map(MergedFeature::to_json).collect())),
    ]);
    let destination = output.join("combined.json");
    fsutil::write_text(&destination, &format!("{}\n", result.render_pretty()))?;
    Ok(format!(
        "Merged 3 runs and {} feature IDs into {}/combined.json",
        features.len(),
        output.display()
    ))
}

/// Run the merger over `--input/--out` pairs (donor `main`).
pub fn run(args: &[String]) -> Result<(), ToolsError> {
    let mut input = PathBuf::from("/tmp");
    let mut output = PathBuf::from("/tmp/quake-integration-combined");
    let mut index = 0;
    while index < args.len() {
        let key = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| ToolsError::invalid(format!("Missing value for {key}")))?;
        if key == "--input" {
            input = fsutil::lexical_absolute(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")), value);
        } else if key == "--out" {
            output = fsutil::lexical_absolute(&std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")), value);
        } else {
            return Err(ToolsError::invalid(format!("Unknown option {key}")));
        }
        index += 2;
    }
    println!("{}", merge_runs(&input, &output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_input(revision: &str, games: &[&str], features_per_game: usize) -> PathBuf {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-merge-").expect("temp dir");
        for game in games {
            let run = directory.join(format!("quake-integration-{game}"));
            std::fs::create_dir_all(&run).expect("run dir");
            fsutil::write_text(
                &run.join("summary.json"),
                &format!(r#"{{"unifiedRevision": "{revision}"}}"#),
            )
            .expect("summary");
            let raws: Vec<String> = (0..features_per_game)
                .map(|index| format!(r#"{{"id": "f{index:04}", "n": {index}}}"#))
                .collect();
            let joins: Vec<String> = (0..features_per_game)
                .map(|index| format!(r#"{{"id": "f{index:04}", "declarationIds": ["{game}:d{index:04}"]}}"#))
                .collect();
            fsutil::write_text(
                &run.join("feature-join.json"),
                &format!(r#"{{"inventories": [{{"path": "inv-{game}", "joins": [{}], "inventory": {{"features": [{}]}}}}]}}"#, joins.join(","), raws.join(",")),
            )
            .expect("join");
            for name in [
                "inventory.json",
                "candidates.json",
                "anchor-inventory.json",
                "historical-completion.json",
            ] {
                fsutil::write_text(&run.join(name), "{}").expect("artifact");
            }
        }
        directory
    }

    fn head_revision() -> String {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(quake_typescript_root())
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git rev-parse");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    fn merges_runs_and_unions_joins() {
        let revision = head_revision();
        let input = fixture_input(&revision, &["q1", "q2", "q3"], FEATURE_COUNT);
        let output = input.join("out");
        let message = merge_runs(&input, &output).expect("merge");
        assert!(message.starts_with("Merged 3 runs and 477 feature IDs"), "{message}");
        let combined = parse_json(&fsutil::read_text(&output.join("combined.json")).expect("read")).expect("parse");
        assert_eq!(
            combined.get("unifiedRevision").and_then(Json::as_str),
            Some(revision.as_str())
        );
        let features = combined.get("features").and_then(Json::as_array).expect("features");
        assert_eq!(features.len(), FEATURE_COUNT);
        assert_eq!(
            features[0]
                .get("declarationIds")
                .and_then(Json::as_array)
                .map(<[Json]>::len),
            Some(3)
        );
        assert_eq!(
            features[0]
                .get("inventories")
                .and_then(Json::as_array)
                .map(<[Json]>::len),
            Some(3)
        );
        assert_eq!(
            combined
                .get("comparisonDocs")
                .and_then(Json::as_array)
                .map(<[Json]>::len),
            Some(3)
        );
        fsutil::remove_forced(&input);
    }

    #[test]
    fn rejects_mismatched_and_short_runs() {
        let revision = head_revision();
        let input = fixture_input(&revision, &["q1", "q2", "q3"], FEATURE_COUNT);
        let output = input.join("out");
        fsutil::write_text(
            &input.join("quake-integration-q2/summary.json"),
            r#"{"unifiedRevision": "xxx"}"#,
        )
        .expect("write");
        assert_eq!(
            merge_runs(&input, &output).expect_err("revision").to_string(),
            "Unified snapshots differ"
        );
        fsutil::write_text(
            &input.join("quake-integration-q2/summary.json"),
            &format!(r#"{{"unifiedRevision": "{revision}"}}"#),
        )
        .expect("write");
        fsutil::write_text(
            &input.join("quake-integration-q3/feature-join.json"),
            r#"{"inventories": [{"path": "inv", "joins": [], "inventory": {"features": []}}]}"#,
        )
        .expect("write");
        let error = merge_runs(&input, &output).expect_err("count");
        assert!(error.to_string().contains("did not retain all 477"), "{error}");
        fsutil::remove_forced(&input);
    }

    #[test]
    fn rejects_unknown_options_and_missing_values() {
        assert!(run(&["--bogus".to_owned(), "x".to_owned()]).is_err());
        assert!(run(&["--input".to_owned()]).is_err());
    }
}
