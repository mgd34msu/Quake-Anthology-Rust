//! Per-seat player identity cvars and userinfo assembly.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/player-userinfo.ts`
//! (`registerPlayerUserinfo`, `playerUserinfo`). The donor's `find` is the merged
//! [`CvarRegistry`](qa_core::cvar::CvarRegistry)'s `get`, `infoString` is `info_string`,
//! and `nativeAtoi` failures yield `0` (the donor's invalid-input result). The NetQuake
//! `name`/`topcolor`/`bottomcolor` strip is a manual scan — the workspace has no regex
//! crate — matching the donor's `/\\(?:name|topcolor|bottomcolor)\\[^\\]*/g` exactly,
//! and the 15-character name clamp counts Unicode scalar values instead of UTF-16 units.

use qa_core::cmd::Dialect;
use qa_core::cvar::{flags, CvarError, CvarRegistry};
use qa_core::numeric::native_atoi;

/// Declare one identity variable, merging flags over an existing Q1 declaration.
fn declare(cvars: &mut CvarRegistry, name: &str, value: &str, declaration_flags: u32) -> Result<(), CvarError> {
    let existing = cvars.get(name);
    if !cvars.dialect().is_q1() || existing.is_none() || cvars.is_console_created(name) {
        cvars.register(name, value, declaration_flags)?;
    } else if let Some(snapshot) = existing {
        if snapshot.flags & declaration_flags != declaration_flags {
            cvars.add_flags(name, declaration_flags)?;
        }
    }
    Ok(())
}

/// Register the identity variables for seat `index` (donor `registerPlayerUserinfo`).
///
/// `model` selects the default Q2 skin and gender; pass `"male"` for the donor default.
/// Quake III registries are untouched.
pub fn register_player_userinfo(cvars: &mut CvarRegistry, index: u32, model: &str) -> Result<(), CvarError> {
    let dialect = cvars.dialect();
    if dialect == Dialect::Q3 {
        return Ok(());
    }
    let info_flags = flags::ARCHIVE | flags::USER_INFO;
    if dialect == Dialect::Q1Netquake || dialect == Dialect::Q1Quakeworld {
        declare(cvars, "qts_weapon_autoswitch", "always", info_flags)?;
    }
    if dialect == Dialect::Q2Rerelease {
        declare(cvars, "autoswitch", "0", info_flags)?;
    }
    if dialect == Dialect::Q1Netquake {
        let name = cvars
            .get("name")
            .map_or_else(|| format!("Player {}", index + 1), |snapshot| snapshot.value);
        declare(cvars, "_cl_name", &name, flags::ARCHIVE)?;
        let color = cvars
            .get("color")
            .map_or_else(|| "0".to_string(), |snapshot| snapshot.value);
        declare(cvars, "_cl_color", &color, flags::ARCHIVE)?;
        return Ok(());
    }
    declare(cvars, "name", &format!("Player {}", index + 1), info_flags)?;
    if dialect == Dialect::Q1Quakeworld {
        for (name, value) in [("topcolor", "0"), ("bottomcolor", "0"), ("team", ""), ("skin", "")] {
            declare(cvars, name, value, info_flags)?;
        }
        return Ok(());
    }
    declare(cvars, "spectator", "0", flags::USER_INFO)?;
    declare(cvars, "password", "", flags::USER_INFO)?;
    let skin_model = if model == "female" {
        "athena"
    } else if model == "cyborg" {
        "oni911"
    } else {
        "grunt"
    };
    let gender = if model == "female" { "female" } else { "male" };
    let skin = format!("{model}/{skin_model}");
    for (name, value) in [
        ("skin", skin.as_str()),
        ("rate", "25000"),
        ("msg", "1"),
        ("hand", "0"),
        ("fov", "90"),
        ("gender", gender),
    ] {
        declare(cvars, name, value, info_flags)?;
    }
    Ok(())
}

/// Remove every `\key\value` pair whose key is listed, matching the donor regex.
fn strip_info_keys(info: &str, keys: &[&str]) -> String {
    let bytes = info.as_bytes();
    let mut out = String::with_capacity(info.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let mut stripped = None;
            for key in keys {
                let rest = &info[index + 1..];
                if rest.len() > key.len() && rest.starts_with(key) && rest.as_bytes()[key.len()] == b'\\' {
                    stripped = Some(key.len());
                    break;
                }
            }
            if let Some(key_len) = stripped {
                index += 1 + key_len + 1;
                while index < bytes.len() && bytes[index] != b'\\' {
                    index += 1;
                }
                continue;
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

/// Assemble the userinfo string (donor `playerUserinfo`).
///
/// NetQuake rewrites `name`/`topcolor`/`bottomcolor` from the `_cl_*` console shadow
/// (or the live `color`/`name` when present); every other dialect returns the info
/// string unchanged.
pub fn player_userinfo(cvars: &mut CvarRegistry) -> Result<String, CvarError> {
    let info = cvars.info_string(flags::USER_INFO, None)?;
    if cvars.dialect() != Dialect::Q1Netquake {
        return Ok(info);
    }
    let color_name = if cvars.get("color").is_none() {
        "_cl_color"
    } else {
        "color"
    };
    let color = native_atoi(&cvars.variable_string(color_name)).unwrap_or(0);
    let name_source = if cvars.get("name").is_none() {
        "_cl_name"
    } else {
        "name"
    };
    let name: String = cvars.variable_string(name_source).chars().take(15).collect();
    let other = strip_info_keys(&info, &["name", "topcolor", "bottomcolor"]);
    Ok(format!(
        "{other}\\name\\{name}\\topcolor\\{}\\bottomcolor\\{}",
        ((color >> 4) & 15).min(13),
        (color & 15).min(13),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netquake_registers_shadow_identity() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_player_userinfo(&mut cvars, 0, "male").unwrap();
        assert_eq!(cvars.variable_string("_cl_name"), "Player 1");
        assert_eq!(cvars.variable_string("_cl_color"), "0");
        assert_eq!(cvars.variable_string("qts_weapon_autoswitch"), "always");
    }

    #[test]
    fn netquake_userinfo_rewrites_colors() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_player_userinfo(&mut cvars, 1, "male").unwrap();
        cvars.set("_cl_color", "35", true).unwrap();
        let info = player_userinfo(&mut cvars).unwrap();
        assert!(info.contains("\\name\\Player 2"), "{info}");
        assert!(info.contains("\\topcolor\\2"), "{info}");
        assert!(info.contains("\\bottomcolor\\3"), "{info}");
    }

    #[test]
    fn quakeworld_registers_team_colors() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Quakeworld);
        register_player_userinfo(&mut cvars, 0, "male").unwrap();
        assert_eq!(cvars.variable_string("name"), "Player 1");
        assert_eq!(cvars.variable_string("topcolor"), "0");
        assert_eq!(cvars.variable_string("team"), "");
    }

    #[test]
    fn q2_registers_skin_and_gender() {
        let mut cvars = CvarRegistry::new(Dialect::Q2Classic);
        register_player_userinfo(&mut cvars, 0, "female").unwrap();
        assert_eq!(cvars.variable_string("skin"), "female/athena");
        assert_eq!(cvars.variable_string("gender"), "female");
        assert_eq!(cvars.variable_string("rate"), "25000");
        let mut cvars = CvarRegistry::new(Dialect::Q2Rerelease);
        register_player_userinfo(&mut cvars, 0, "cyborg").unwrap();
        assert_eq!(cvars.variable_string("skin"), "cyborg/oni911");
        assert_eq!(cvars.variable_string("autoswitch"), "0");
    }

    #[test]
    fn q3_is_untouched() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_player_userinfo(&mut cvars, 0, "male").unwrap();
        assert!(cvars.get("name").is_none());
    }

    #[test]
    fn q1_existing_declaration_merges_flags() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Quakeworld);
        cvars.register("name", "Custom", flags::ARCHIVE).unwrap();
        register_player_userinfo(&mut cvars, 0, "male").unwrap();
        let snapshot = cvars.get("name").unwrap();
        assert_eq!(snapshot.value, "Custom");
        assert_eq!(
            snapshot.flags & (flags::ARCHIVE | flags::USER_INFO),
            flags::ARCHIVE | flags::USER_INFO
        );
    }

    #[test]
    fn strip_handles_malformed_tail() {
        assert_eq!(strip_info_keys("\\name\\a\\x", &["name"]), "\\x");
        assert_eq!(strip_info_keys("\\name", &["name"]), "\\name");
        assert_eq!(
            strip_info_keys("\\topcolor\\1\\bottomcolor\\2", &["topcolor", "bottomcolor"]),
            ""
        );
    }
}
