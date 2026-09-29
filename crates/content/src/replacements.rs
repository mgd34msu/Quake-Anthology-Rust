//! MD5 replacement selection for Q1/Q2 models (`src/formats/q3-model/replacements.ts`).
//!
//! Donor provenance: `src/formats/q3-model/replacements.ts` (selection
//! from the Q1/Q2 rerelease ports). Skin groups and animation timing
//! reuse [`crate::common::TimedFrames`]; the selection itself lives in
//! [`crate::md5::SkinSelection`].

use crate::common::{TimedFrame, TimedFrames};
use crate::md2::Md2Model;
use crate::md5::{Q1AnimationTiming, SkinSelection};
use crate::mdl::MdlModel;

/// Q1 or Q2 replacement selection (`"q1" | "q2"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
}

/// MD5 sibling and scale paths (`Md5PathsFor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Md5Paths {
    /// Mesh path.
    pub mesh: String,
    /// Animation path.
    pub animation: String,
    /// Scale path.
    pub scales: String,
}

/// Select MD5 sibling paths (`md5PathsFor` / `md5SkinPathFor`).
///
/// Extensionless paths keep the donor `slice(-1)` quirk, which appends
/// the final character as the suffix.
#[must_use]
pub fn md5_paths_for(model_path: &str, family: QFamily) -> Md5Paths {
    let (base, suffix) = match model_path.rfind('.') {
        Some(dot) => (&model_path[..dot], &model_path[dot..]),
        None => (
            model_path,
            model_path.get(model_path.len().saturating_sub(1)..).unwrap_or(""),
        ),
    };
    let skin = match family {
        QFamily::Q1 => format!("{base}_md5{suffix}"),
        QFamily::Q2 => format!("{base}-md5{suffix}"),
    };
    let skin_base = match skin.rsplit_once('.') {
        Some((base, _)) => base,
        None => skin.as_str(),
    };
    Md5Paths {
        mesh: format!("{skin_base}.md5mesh"),
        animation: format!("{skin_base}.md5anim"),
        scales: format!("{skin_base}.md5scales"),
    }
}

/// Select a model or animation sibling from the VFS search order
/// (`md5ReplacementAllowed`).
///
/// Search ranks increase toward lower priority. Unknown ranks retain
/// source behavior.
#[must_use]
pub fn md5_replacement_allowed(primary_rank: Option<u32>, mesh_rank: Option<u32>) -> bool {
    match (primary_rank, mesh_rank) {
        (Some(primary), Some(mesh)) => mesh <= primary,
        _ => true,
    }
}

/// Map MD2 skins to a replacement selection (`md2ReplacementSkinSelection`).
#[must_use]
pub fn md2_replacement_skin_selection(
    alias: &Md2Model,
    scale_source: Option<String>,
    diagnostics: Vec<String>,
) -> SkinSelection {
    SkinSelection::Q2Md2Replacement {
        skins: (0..alias.skins.len())
            .map(|skin| format!("replacement_skin_{skin}"))
            .collect(),
        source_frame_count: alias.frames.len(),
        scale_source,
        diagnostics,
    }
}

/// Build a Q1 MD5 skin path (`q1Md5SkinPath`).
///
/// Group and frame numbers are `u32`, so the donor range check is
/// enforced by the type.
#[must_use]
pub fn q1_md5_skin_path(shader: &str, group: u32, frame: u32) -> String {
    format!("progs/{shader}_{group:02}_{frame:02}")
}

/// Select Q1 replacement animation timing (`q1Md5AnimationTiming`).
#[must_use]
pub fn q1_md5_animation_timing(alias_frame_count: usize, model_frame_count: usize) -> Q1AnimationTiming {
    if alias_frame_count > 1 || model_frame_count == 1 {
        Q1AnimationTiming::EntityFrame
    } else {
        Q1AnimationTiming::ElapsedTime { frame_rate: 2.0 }
    }
}

/// Map MDL skins to a replacement selection (`q1ReplacementSkinSelection`).
#[must_use]
pub fn q1_replacement_skin_selection(
    mesh_shaders: &[String],
    alias: &MdlModel,
    model_frame_count: usize,
) -> SkinSelection {
    let mesh_skin_groups = mesh_shaders
        .iter()
        .map(|shader| {
            alias
                .skins
                .iter()
                .enumerate()
                .map(|(group, skins)| match skins {
                    TimedFrames::Single(_) => TimedFrames::Single(q1_md5_skin_path(shader, group as u32, 0)),
                    TimedFrames::Group(frames) => TimedFrames::Group(
                        frames
                            .iter()
                            .enumerate()
                            .map(|(frame, item)| TimedFrame {
                                interval_seconds: item.interval_seconds,
                                frame: q1_md5_skin_path(shader, group as u32, frame as u32),
                            })
                            .collect(),
                    ),
                })
                .collect()
        })
        .collect();
    SkinSelection::Q1MdlReplacement {
        mesh_skin_groups,
        flags: alias.flags,
        timing: q1_md5_animation_timing(alias.frames.len(), model_frame_count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_paths() {
        assert_eq!(
            md5_paths_for("progs/armor.mdl", QFamily::Q1),
            Md5Paths {
                mesh: "progs/armor_md5.md5mesh".to_string(),
                animation: "progs/armor_md5.md5anim".to_string(),
                scales: "progs/armor_md5.md5scales".to_string(),
            }
        );
        assert_eq!(
            md5_paths_for("players/male/tris.md2", QFamily::Q2),
            Md5Paths {
                mesh: "players/male/tris-md5.md5mesh".to_string(),
                animation: "players/male/tris-md5.md5anim".to_string(),
                scales: "players/male/tris-md5.md5scales".to_string(),
            }
        );
        assert_eq!(q1_md5_skin_path("armor", 1, 2), "progs/armor_01_02");
        assert!(md5_replacement_allowed(Some(2), Some(1)));
        assert!(!md5_replacement_allowed(Some(1), Some(2)));
        assert!(md5_replacement_allowed(None, Some(2)));
    }
}
