//! Quake III foundation: assets.
//!
//! Donor provenance: `src/content/q3/foundation/assets.ts`.

use crate::contract::ResolvedResourceReference;
use crate::md3::{parse_md3, parse_skin, SkinSurface};
use crate::mounts::OpenedResource;
use crate::q3scene::{to_scene_md3, SceneMd3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation_config::*;
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// assets.ts: CG_FindClientModelFile, CG_FindClientHeadFile,
// CG_RegisterClientModelname.
// ---------------------------------------------------------------------------

/// Character team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Team {
    /// Red.
    Red,
    /// Blue.
    Blue,
}

impl Q3Team {
    fn as_str(self) -> &'static str {
        match self {
            Q3Team::Red => "red",
            Q3Team::Blue => "blue",
        }
    }
}

/// Character selection (`Q3CharacterSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CharacterSelection {
    /// Model name.
    pub model: String,
    /// Skin name.
    pub skin: String,
    /// Head model name.
    pub head_model: String,
    /// Head skin name.
    pub head_skin: String,
    /// Team.
    pub team: Option<Q3Team>,
    /// Team name.
    pub team_name: String,
}

/// Character part (`Q3CharacterPart`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterPart {
    /// Mesh resource.
    pub resource: ResolvedResourceReference,
    /// Mesh model.
    pub model: SceneMd3,
    /// Skin resource.
    pub skin_resource: ResolvedResourceReference,
    /// Skin surfaces.
    pub surfaces: Vec<SkinSurface>,
}

/// Character assets (`Q3CharacterAssets`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterAssets {
    /// Selection.
    pub selection: Q3CharacterSelection,
    /// Lower part.
    pub lower: Q3CharacterPart,
    /// Upper part.
    pub upper: Q3CharacterPart,
    /// Head part.
    pub head: Q3CharacterPart,
    /// Animation resource.
    pub animation_resource: ResolvedResourceReference,
    /// Animation config.
    pub animation: PlayerAnimationConfig,
    /// Icon resource.
    pub icon: Option<ResolvedResourceReference>,
}

/// Character resource bytes (`Q3CharacterResources`, sync adaptation of the
/// donor async plan reader).
pub trait Q3CharacterResources {
    /// Open a resource path.
    fn open(&self, path: &str) -> Result<Option<OpenedResource>, Q3FoundationError>;
}

pub(crate) fn byte_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| *byte as char).collect()
}

pub(crate) fn validate_component(value: &str, name: &str, star: bool) -> Result<(), Q3FoundationError> {
    let plain = if star && value.starts_with('*') {
        &value[1..]
    } else {
        value
    };
    if plain.is_empty() || plain == "." || plain == ".." || plain.contains(['\0', '/', '\\']) {
        return Err(range(format!("Invalid Q3 {name}: {value}")));
    }
    Ok(())
}

pub(crate) fn first_resource(
    resources: &impl Q3CharacterResources,
    paths: &[String],
) -> Result<Option<OpenedResource>, Q3FoundationError> {
    for path in paths {
        let resource = resources.open(path)?;
        if resource.as_ref().is_some_and(|open| !open.bytes.is_empty()) {
            return Ok(resource);
        }
    }
    Ok(None)
}

pub(crate) fn required_resource(
    resources: &impl Q3CharacterResources,
    paths: &[String],
) -> Result<OpenedResource, Q3FoundationError> {
    first_resource(resources, paths)?
        .ok_or_else(|| failed(format!("Q3 character resource missing: {}", paths.join(", "))))
}

pub(crate) fn truncate_path(path: String, limit: usize) -> String {
    path.chars().take(limit).collect()
}

pub(crate) fn body_files(selection: &Q3CharacterSelection, base: &str, team_prefix: &str) -> Vec<String> {
    let team = selection.team.map_or("default", Q3Team::as_str);
    let fallback = match selection.team {
        Some(team) => team.as_str(),
        None => selection.skin.as_str(),
    };
    let mut paths = Vec::new();
    for folder in ["", "characters/"] {
        let prefixes: &[&str] = if team_prefix.is_empty() {
            &[""]
        } else {
            &[team_prefix, ""]
        };
        for prefix in prefixes {
            paths.push(truncate_path(
                format!(
                    "models/players/{folder}{}/{prefix}{base}_{}_{team}.skin",
                    selection.model, selection.skin
                ),
                63,
            ));
            paths.push(truncate_path(
                format!(
                    "models/players/{folder}{}/{prefix}{base}_{fallback}.skin",
                    selection.model
                ),
                63,
            ));
        }
    }
    paths
}

pub(crate) fn head_files(
    selection: &Q3CharacterSelection,
    base: &str,
    extension: &str,
    team_prefix: &str,
) -> Vec<String> {
    let model = if selection.head_model.is_empty() {
        selection.model.as_str()
    } else {
        selection.head_model.as_str()
    };
    let name = model.strip_prefix('*').unwrap_or(model);
    let team = selection.team.map_or("default", Q3Team::as_str);
    let fallback = match selection.team {
        Some(team) => team.as_str(),
        None => selection.head_skin.as_str(),
    };
    let limit = if base == "head" { 63 } else { 127 };
    let folders: &[&str] = if model.starts_with('*') {
        &["heads/"]
    } else {
        &["", "heads/"]
    };
    let mut paths = Vec::new();
    for folder in folders {
        let prefixes: &[&str] = if team_prefix.is_empty() {
            &[""]
        } else {
            &[team_prefix, ""]
        };
        for prefix in prefixes {
            paths.push(truncate_path(
                format!(
                    "models/players/{folder}{name}/{}/{prefix}{base}_{team}.{extension}",
                    selection.head_skin
                ),
                limit,
            ));
            paths.push(truncate_path(
                format!("models/players/{folder}{name}/{prefix}{base}_{fallback}.{extension}"),
                limit,
            ));
        }
    }
    paths
}

/// Load character assets (`loadQ3Character`).
pub fn load_q3_character(
    resources: &impl Q3CharacterResources,
    selection: &Q3CharacterSelection,
) -> Result<Q3CharacterAssets, Q3FoundationError> {
    validate_component(&selection.model, "model", false)?;
    validate_component(&selection.skin, "skin", false)?;
    if !selection.head_model.is_empty() {
        validate_component(&selection.head_model, "head model", true)?;
    }
    validate_component(&selection.head_skin, "head skin", false)?;
    if !selection.team_name.is_empty() {
        validate_component(&selection.team_name, "team name", false)?;
    }
    let model = selection.model.as_str();
    let head = if selection.head_model.is_empty() {
        model
    } else {
        selection.head_model.as_str()
    };
    let head_name = head.strip_prefix('*').unwrap_or(head);
    let lower = required_resource(
        resources,
        &[
            format!("models/players/{model}/lower.md3"),
            format!("models/players/characters/{model}/lower.md3"),
        ],
    )?;
    let upper = required_resource(
        resources,
        &[
            format!("models/players/{model}/upper.md3"),
            format!("models/players/characters/{model}/upper.md3"),
        ],
    )?;
    let face_paths: Vec<String> = if head.starts_with('*') {
        vec![format!("models/players/heads/{head_name}/{head_name}.md3")]
    } else {
        vec![
            format!("models/players/{head}/head.md3"),
            format!("models/players/heads/{head_name}/{head_name}.md3"),
        ]
    };
    let face = required_resource(resources, &face_paths)?;
    let animation = required_resource(
        resources,
        &[
            format!("models/players/{model}/animation.cfg"),
            format!("models/players/characters/{model}/animation.cfg"),
        ],
    )?;
    let team_names: Vec<String> = if selection.team_name.is_empty() {
        vec![String::new()]
    } else {
        vec![
            format!("{}/", selection.team_name),
            if selection.team == Some(Q3Team::Blue) {
                "Pagans/".to_string()
            } else {
                "Stroggs/".to_string()
            },
        ]
    };
    let mut skins: Option<[OpenedResource; 3]> = None;
    for team in &team_names {
        let legs = first_resource(resources, &body_files(selection, "lower", team))?;
        let torso = first_resource(resources, &body_files(selection, "upper", team))?;
        let head_skin = first_resource(resources, &head_files(selection, "head", "skin", team))?;
        if let (Some(legs), Some(torso), Some(head_skin)) = (legs, torso, head_skin) {
            skins = Some([legs, torso, head_skin]);
            break;
        }
    }
    let Some(skins) = skins else {
        return Err(failed(format!(
            "Q3 character skin missing: {}/{}, {}/{}",
            selection.model, selection.skin, head, selection.head_skin
        )));
    };
    let [legs_skin, torso_skin, head_skin] = skins;
    let make_part = |mesh: OpenedResource, skin: OpenedResource| -> Result<Q3CharacterPart, Q3FoundationError> {
        Ok(Q3CharacterPart {
            resource: mesh.reference.clone(),
            model: to_scene_md3(parse_md3(&mesh.bytes, &mesh.reference.requested_path)?.model),
            skin_resource: skin.reference.clone(),
            surfaces: parse_skin(&byte_text(&skin.bytes))?,
        })
    };
    let icon_prefix = if selection.team_name.is_empty() {
        String::new()
    } else {
        format!("{}/", selection.team_name)
    };
    let mut icon_paths = head_files(selection, "icon", "skin", &icon_prefix);
    icon_paths.extend(head_files(selection, "icon", "tga", &icon_prefix));
    let icon = first_resource(resources, &icon_paths)?;
    let animation_text = byte_text(&animation.bytes);
    let animation_path = animation.reference.requested_path.clone();
    Ok(Q3CharacterAssets {
        selection: selection.clone(),
        lower: make_part(lower, legs_skin)?,
        upper: make_part(upper, torso_skin)?,
        head: make_part(face, head_skin)?,
        animation_resource: animation.reference.clone(),
        animation: parse_player_animation_config(&animation_text, &animation_path)?,
        icon: icon.map(|open| open.reference),
    })
}
