//! MG3 monster resource generation (donor
//! `tools/content/generate-mg3-monster-resources.ts`).
//!
//! Reads `quakec_mg3` monster sources, collects their precache declarations,
//! and writes the `mg3-resources` module.

use std::path::{Path, PathBuf};

use crate::error::ToolsError;
use crate::fsutil::{read_text, write_bytes};
use crate::json::Json;

/// Class name to monster module mapping.
pub const MODULES: &[(&str, &str)] = &[
    ("monster_ogre_rocket", "ogre"),
    ("monster_demodog", "mg3_demodog"),
    ("monster_army_infected", "mg3_soldier_infected"),
    ("monster_knight_infected", "mg3_knight_infected"),
    ("monster_enforcer_infected", "mg3_enforcer_infected"),
    ("monster_hell_knight_infected", "mg3_hknight_infected"),
    ("monster_ranged_knight", "mg3_rknight"),
    ("monster_super_shambler", "mg3_super_shambler"),
    ("monster_lava_man", "mg3_lavaman"),
    ("monster_ghost", "mg3_player_ghost"),
    ("monster_orb", "mg3_orb"),
    ("monster_szombie", "mg3_shub_zombie"),
    ("monster_oldone_new", "mg3_oldone_new"),
    ("monster_boss_final", "boss_final"),
];

/// Strip block comments (non-greedy) then line comments, like the donor.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            match source[index + 2..].find("*/") {
                Some(end) => index += end + 4,
                None => break,
            }
        } else {
            let ch = source[index..].chars().next().unwrap_or('\u{FFFD}');
            out.push(ch);
            index += ch.len_utf8();
        }
    }
    let mut stripped = String::with_capacity(out.len());
    for line in out.split_inclusive('\n') {
        match line.find("//") {
            Some(pos) => {
                stripped.push_str(&line[..pos]);
                if line.ends_with('\n') {
                    stripped.push('\n');
                }
            }
            None => stripped.push_str(line),
        }
    }
    stripped
}

/// Collect precache paths (`precache_(model|sound)\d?\s*\(\s*"([^"]+)"\s*\)`).
fn precache_paths(source: &str) -> Vec<String> {
    let mut paths = vec![
        "progs/gib1.mdl".to_owned(),
        "progs/gib2.mdl".to_owned(),
        "progs/gib3.mdl".to_owned(),
        "sound/player/udeath.wav".to_owned(),
    ];
    let bytes = source.as_bytes();
    let mut index = 0;
    while let Some(found) = source[index..].find("precache_") {
        let mut cursor = index + found + "precache_".len();
        let kind = if source[cursor..].starts_with("model") {
            cursor += "model".len();
            "model"
        } else if source[cursor..].starts_with("sound") {
            cursor += "sound".len();
            "sound"
        } else {
            index = cursor;
            continue;
        };
        if cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'(') {
            index = cursor;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'"') {
            index = cursor;
            continue;
        }
        cursor += 1;
        let start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b'"' {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        let path = &source[start..cursor];
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b')') {
            index = cursor;
            continue;
        }
        index = cursor + 1;
        if kind == "sound" {
            paths.push(format!("sound/{path}"));
        } else {
            paths.push(path.to_owned());
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Collect resources for every monster class.
pub fn collect_resources(donor: &Path) -> Result<Vec<(String, Vec<String>)>, ToolsError> {
    let mut result = Vec::new();
    for (classname, module) in MODULES {
        let source = read_text(&donor.join("monsters").join(format!("{module}.qc")))?;
        result.push(((*classname).to_owned(), precache_paths(&strip_comments(&source))));
    }
    Ok(result)
}

/// Render the generated module text.
#[must_use]
pub fn render_module(resources: &[(String, Vec<String>)]) -> String {
    let value = Json::object(
        resources
            .iter()
            .map(|(classname, paths)| {
                (
                    classname.clone(),
                    Json::object(vec![(
                        "resources".to_owned(),
                        Json::array(paths.iter().map(Json::string).collect()),
                    )]),
                )
            })
            .collect(),
    );
    format!(
        "// Generated from quakec_mg3/monsters source precache declarations.\n// Regenerate with tools/content/generate-mg3-monster-resources.ts.\nexport const mg3MonsterResources: Readonly<Record<string, {{ readonly resources: readonly string[] }}>> = {};\n",
        value.render_pretty()
    )
}

/// Run the generator: `args` is `[donor, project-root?]`.
pub fn run(args: &[String]) -> Result<PathBuf, ToolsError> {
    let donor = args
        .first()
        .ok_or_else(|| ToolsError::invalid("Pass the quakec_mg3 source directory"))?;
    let root = args.get(1).map_or_else(
        || std::env::current_dir().map_err(|error| ToolsError::io("resolving current directory", error)),
        |path| Ok(PathBuf::from(path)),
    )?;
    let resources = collect_resources(Path::new(donor))?;
    let destination = root.join("src/content/monsters/mg3-resources.ts");
    write_bytes(&destination, render_module(&resources).as_bytes())?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precache_scan_matches_donor_shape() {
        let source = "precache_model(\"progs/a.mdl\");\nprecache_sound2 ( \"x.wav\" )\n// precache_model(\"no.mdl\")\n/* precache_sound(\"no.wav\") */\nprecache_model(\"progs/a.mdl\");\n";
        let stripped = strip_comments(source);
        let paths = precache_paths(&stripped);
        assert!(paths.contains(&"progs/a.mdl".to_owned()));
        assert!(paths.contains(&"sound/x.wav".to_owned()));
        assert!(!paths.iter().any(|path| path.contains("no.")));
        assert_eq!(paths.iter().filter(|path| path.as_str() == "progs/a.mdl").count(), 1);
        let sorted = {
            let mut clone = paths.clone();
            clone.sort();
            clone
        };
        assert_eq!(paths, sorted);
    }
}
