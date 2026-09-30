//! Weapon behavior tool CLI options.
//!
//! Donor: `src/app/bootstrap/weapon-behavior-tool-options.ts`
//! (`parseWeaponBehaviorTool`). `ProjectileRole` is absorbed from
//! `src/contracts/weapon-behavior.ts`; content roots reuse
//! `qa_content`.

use std::path::PathBuf;

use qa_content::paths::{normalize_resource_path, PathError};
use qa_content::user_data::default_user_content_root;
use thiserror::Error;

/// Tool help text (also the bad-action error).
pub const WEAPON_BEHAVIOR_TOOL_HELP: &str = "Usage:\n\
  quake-typescript weapon-behavior inspect PRODUCT [options]\n\
  quake-typescript weapon-behavior declare-qvm PRODUCT --profile MOUNTED_PROFILE_JSON [options]\n\
  quake-typescript weapon-behavior declare-native PRODUCT --profile MOUNTED_PROFILE_JSON [options]\n\
  quake-typescript weapon-behavior declare PRODUCT --id NAMESPACE:ID --role ROLE --fire CALLBACK [options]\n\
\n\
  --content PATH        Installed content root\n\
  --user-content PATH   Writable user content root\n\
  --artifact PATH       Mounted program (QC descriptor/progs.dat/qwprogs.dat, vm/qagame.qvm, or native game DLL)\n\
  --profile PATH        Author-written mounted QVM or API2023 Windows x64 native profile with exact digest, entries and entity layout\n\
  --title TEXT          Display title (defaults to declaration ID)\n\
  --activate CALLBACK   Optional source activation callback\n\
  --role ROLE           rocket, grenade, nail, bolt, plasma, energy, grapple\n\
  --help                Show this help\n\
\n\
Inspection reports actual bytecode callbacks and think assignments. It does not infer\n\
projectile roles or activation gates. Declare only callbacks whose behavior you have\n\
established from the source. Declarations bind the exact current program digest.\n";

/// Projectile role (donor `contracts/weapon-behavior.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileRole {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Nail.
    Nail,
    /// Bolt.
    Bolt,
    /// Plasma.
    Plasma,
    /// Energy.
    Energy,
    /// Grapple.
    Grapple,
}

impl ProjectileRole {
    /// Role name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rocket => "rocket",
            Self::Grenade => "grenade",
            Self::Nail => "nail",
            Self::Bolt => "bolt",
            Self::Plasma => "plasma",
            Self::Energy => "energy",
            Self::Grapple => "grapple",
        }
    }

    /// Parse a role name.
    pub fn parse(value: &str) -> Result<Self, WeaponBehaviorToolError> {
        match value {
            "rocket" => Ok(Self::Rocket),
            "grenade" => Ok(Self::Grenade),
            "nail" => Ok(Self::Nail),
            "bolt" => Ok(Self::Bolt),
            "plasma" => Ok(Self::Plasma),
            "energy" => Ok(Self::Energy),
            "grapple" => Ok(Self::Grapple),
            _ => Err(WeaponBehaviorToolError::BadRole(value.to_owned())),
        }
    }
}

/// Parsed tool command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponBehaviorToolCommand {
    /// Show help.
    Help,
    /// Inspect a product program.
    Inspect {
        /// Product name.
        product: String,
        /// Installed content root.
        corpus_root: String,
        /// Writable user content root.
        user_content_root: String,
        /// Mounted program resource path.
        artifact: Option<String>,
    },
    /// Declare from a QVM profile.
    DeclareQvm {
        /// Product name.
        product: String,
        /// Installed content root.
        corpus_root: String,
        /// Writable user content root.
        user_content_root: String,
        /// Mounted program resource path.
        artifact: Option<String>,
        /// Mounted profile resource path.
        profile: String,
    },
    /// Declare from a native profile.
    DeclareNative {
        /// Product name.
        product: String,
        /// Installed content root.
        corpus_root: String,
        /// Writable user content root.
        user_content_root: String,
        /// Mounted program resource path.
        artifact: Option<String>,
        /// Mounted profile resource path.
        profile: String,
    },
    /// Declare callbacks directly.
    Declare {
        /// Product name.
        product: String,
        /// Installed content root.
        corpus_root: String,
        /// Writable user content root.
        user_content_root: String,
        /// Mounted program resource path.
        artifact: Option<String>,
        /// Declaration id (`NAMESPACE:ID`).
        id: String,
        /// Display title.
        title: String,
        /// Projectile role.
        role: ProjectileRole,
        /// Fire callback.
        fire: String,
        /// Activation callback.
        activate: Option<String>,
    },
}

/// Tool option failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WeaponBehaviorToolError {
    /// Bad action (carries the help text).
    #[error("{0}")]
    Usage(&'static str),
    /// Unknown flag.
    #[error("Unknown behavior option: {0}")]
    UnknownOption(String),
    /// Missing or invalid flag value.
    #[error("Missing or invalid value for {0}")]
    MissingValue(String),
    /// Repeated flag.
    #[error("Repeated behavior option: {0}")]
    RepeatedOption(String),
    /// Profile action without `--profile`.
    #[error("{action} requires --profile and takes identity, role and callbacks from that declaration")]
    ProfileRequired {
        /// Action name.
        action: String,
    },
    /// Misplaced `--profile`.
    #[error("--profile requires declare-qvm or declare-native")]
    ProfileMisplaced,
    /// Declaration flags without `declare`.
    #[error("Declaration options require the declare action")]
    DeclarationMisplaced,
    /// Incomplete `declare`.
    #[error("Declare requires --id NAMESPACE:ID, --role ROLE and --fire CALLBACK")]
    DeclareMissing,
    /// Invalid role.
    #[error("Invalid projectile role: {0}")]
    BadRole(String),
    /// Invalid resource path.
    #[error(transparent)]
    BadPath(#[from] PathError),
}

/// `^[^:\s]+:[^\s]+$`: a head without colons or whitespace, a colon,
/// then a non-empty tail without whitespace (tail colons allowed).
fn is_behavior_id(value: &str) -> bool {
    let Some(colon) = value.find(':') else {
        return false;
    };
    let (head, tail) = value.split_at(colon);
    let tail = &tail[1..];
    !head.is_empty()
        && head.chars().all(|c| c != ':' && !c.is_whitespace())
        && !tail.is_empty()
        && !tail.chars().any(char::is_whitespace)
}

fn home_dir() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home),
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
    }
}

/// Node `resolve`: absolute against the working directory, lexically
/// normalized (`.`/`..` resolved without touching the filesystem).
fn resolve_tool_path(explicit: Option<&str>, default: PathBuf) -> String {
    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let path = match explicit {
        Some(text) => {
            let candidate = PathBuf::from(text);
            if candidate.is_absolute() {
                candidate
            } else {
                base.join(candidate)
            }
        }
        None => default,
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized.to_string_lossy().into_owned()
}

/// Parse tool arguments.
pub fn parse_weapon_behavior_tool(
    argv: &[String],
) -> Result<WeaponBehaviorToolCommand, WeaponBehaviorToolError> {
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(WeaponBehaviorToolCommand::Help);
    }
    let (Some(action), Some(product)) = (argv.first(), argv.get(1)) else {
        return Err(WeaponBehaviorToolError::Usage(WEAPON_BEHAVIOR_TOOL_HELP));
    };
    if !["inspect", "declare", "declare-qvm", "declare-native"].contains(&action.as_str())
        || product.starts_with("--")
    {
        return Err(WeaponBehaviorToolError::Usage(WEAPON_BEHAVIOR_TOOL_HELP));
    }
    let known = [
        "--content",
        "--user-content",
        "--artifact",
        "--id",
        "--title",
        "--role",
        "--fire",
        "--activate",
        "--profile",
    ];
    let mut flags = std::collections::HashMap::new();
    let mut index = 2;
    while index < argv.len() {
        let key = &argv[index];
        let value = argv.get(index + 1);
        if !known.contains(&key.as_str()) {
            return Err(WeaponBehaviorToolError::UnknownOption(key.clone()));
        }
        let Some(value) = value else {
            return Err(WeaponBehaviorToolError::MissingValue(key.clone()));
        };
        if value.starts_with("--") || value.is_empty() || value.contains('\0') {
            return Err(WeaponBehaviorToolError::MissingValue(key.clone()));
        }
        if flags.contains_key(key) {
            return Err(WeaponBehaviorToolError::RepeatedOption(key.clone()));
        }
        flags.insert(key.clone(), value.clone());
        index += 2;
    }
    let get = |key: &str| flags.get(key).map(String::as_str);
    let corpus_root =
        resolve_tool_path(get("--content"), home_dir().join("Projects/qfiles"));
    let user_content_root = resolve_tool_path(get("--user-content"), default_user_content_root());
    let artifact = get("--artifact").map(normalize_resource_path).transpose()?;
    if action == "declare-qvm" || action == "declare-native" {
        let profile = get("--profile");
        if profile.is_none()
            || ["--id", "--title", "--role", "--fire", "--activate"]
                .iter()
                .any(|key| flags.contains_key(*key))
        {
            return Err(WeaponBehaviorToolError::ProfileRequired { action: action.clone() });
        }
        let profile = normalize_resource_path(profile.unwrap_or(""))?;
        let content = (product.clone(), corpus_root, user_content_root, artifact, profile);
        return Ok(if action == "declare-qvm" {
            WeaponBehaviorToolCommand::DeclareQvm {
                product: content.0,
                corpus_root: content.1,
                user_content_root: content.2,
                artifact: content.3,
                profile: content.4,
            }
        } else {
            WeaponBehaviorToolCommand::DeclareNative {
                product: content.0,
                corpus_root: content.1,
                user_content_root: content.2,
                artifact: content.3,
                profile: content.4,
            }
        });
    }
    if flags.contains_key("--profile") {
        return Err(WeaponBehaviorToolError::ProfileMisplaced);
    }
    if action == "inspect" {
        if ["--id", "--title", "--role", "--fire", "--activate"]
            .iter()
            .any(|key| flags.contains_key(*key))
        {
            return Err(WeaponBehaviorToolError::DeclarationMisplaced);
        }
        return Ok(WeaponBehaviorToolCommand::Inspect {
            product: product.clone(),
            corpus_root,
            user_content_root,
            artifact,
        });
    }
    let (id, fire, role_name, activate) =
        (get("--id"), get("--fire"), get("--role"), get("--activate"));
    let (Some(id), Some(fire), Some(role_name)) = (id, fire, role_name) else {
        return Err(WeaponBehaviorToolError::DeclareMissing);
    };
    if !is_behavior_id(id) {
        return Err(WeaponBehaviorToolError::DeclareMissing);
    }
    Ok(WeaponBehaviorToolCommand::Declare {
        product: product.clone(),
        corpus_root,
        user_content_root,
        artifact,
        id: id.to_owned(),
        title: get("--title").unwrap_or(id).to_owned(),
        role: ProjectileRole::parse(role_name)?,
        fire: fire.to_owned(),
        activate: activate.map(str::to_owned),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn help_flag_wins_anywhere() {
        assert_eq!(parse_weapon_behavior_tool(&argv(&["inspect", "q3", "--help"])), Ok(WeaponBehaviorToolCommand::Help));
        assert_eq!(parse_weapon_behavior_tool(&argv(&["-h"])), Ok(WeaponBehaviorToolCommand::Help));
    }

    #[test]
    fn inspect_happy_path() {
        let command = parse_weapon_behavior_tool(&argv(&[
            "inspect",
            "q3",
            "--content",
            "/tmp/corpus",
            "--artifact",
            "vm/qagame.qvm",
        ]))
        .unwrap();
        let WeaponBehaviorToolCommand::Inspect { product, corpus_root, artifact, .. } = command
        else {
            panic!("expected inspect");
        };
        assert_eq!(product, "q3");
        assert_eq!(corpus_root, "/tmp/corpus");
        assert_eq!(artifact.as_deref(), Some("vm/qagame.qvm"));
    }

    #[test]
    fn default_roots_follow_home_pattern() {
        let command = parse_weapon_behavior_tool(&argv(&["inspect", "q3"])).unwrap();
        let WeaponBehaviorToolCommand::Inspect { corpus_root, user_content_root, .. } = command
        else {
            panic!("expected inspect");
        };
        assert!(corpus_root.ends_with("Projects/qfiles"), "{corpus_root}");
        assert!(user_content_root.ends_with(".local/share/quake-typescript/content"), "{user_content_root}");
    }

    #[test]
    fn declare_qvm_requires_profile() {
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["declare-qvm", "q3"])),
            Err(WeaponBehaviorToolError::ProfileRequired { action: "declare-qvm".to_owned() })
        );
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["declare-qvm", "q3", "--profile", "p.json", "--role", "rocket"])),
            Err(WeaponBehaviorToolError::ProfileRequired { action: "declare-qvm".to_owned() })
        );
        assert!(parse_weapon_behavior_tool(&argv(&["declare-qvm", "q3", "--profile", "prof.json"])).is_ok());
    }

    #[test]
    fn declare_happy_path_and_errors() {
        let command = parse_weapon_behavior_tool(&argv(&[
            "declare", "q2", "--id", "ns:rl", "--role", "rocket", "--fire", "fire_rocket",
        ]))
        .unwrap();
        let WeaponBehaviorToolCommand::Declare { id, title, role, fire, activate, .. } = command
        else {
            panic!("expected declare");
        };
        assert_eq!((id.as_str(), title.as_str()), ("ns:rl", "ns:rl"));
        assert_eq!((role, fire.as_str(), activate), (ProjectileRole::Rocket, "fire_rocket", None));
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["declare", "q2", "--id", "nope", "--role", "rocket", "--fire", "f"])),
            Err(WeaponBehaviorToolError::DeclareMissing)
        );
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["declare", "q2", "--id", "a:b", "--role", "nuke", "--fire", "f"])),
            Err(WeaponBehaviorToolError::BadRole("nuke".to_owned()))
        );
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["inspect", "q2", "--role", "rocket"])),
            Err(WeaponBehaviorToolError::DeclarationMisplaced)
        );
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["inspect", "q2", "--bogus", "x"])),
            Err(WeaponBehaviorToolError::UnknownOption("--bogus".to_owned()))
        );
        assert_eq!(
            parse_weapon_behavior_tool(&argv(&["bogus", "q2"])),
            Err(WeaponBehaviorToolError::Usage(WEAPON_BEHAVIOR_TOOL_HELP))
        );
    }
}
