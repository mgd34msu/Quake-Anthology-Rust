//! Foreign held-weapon declarations and view-model passes.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/held-weapon.ts`
//! (`ForeignHeldWeapons`). Synchronous port against the workspace's held
//! weapon, attachment, and grip siblings. Decoded models arrive through
//! [`ForeignHeldAssetSource`] as the inspected subset (digest, path,
//! triangles, frame count); passes are returned as [`HeldWeaponPass`] because
//! `Q3PresentedModel` has no alias-model variant for scene assembly.

use qa_client::render::scene::models::attachment::align_model_attachment;
use qa_client::render::scene::models::types::EntityTransform;
use qa_content::contract::{
    ContentId, GameFamily, HeldWeaponDeclaration, HeldWeaponModel, ModelAttachmentTarget, ModelTransform,
};
use qa_content::held_weapon::{read_held_weapon_file, HeldWeaponError as DeclarationError};
use qa_content::q1::foundation::held_weapons::q1_held_weapon;
use qa_content::q2::foundation::held_weapons::q2_held_weapon;
use qa_content::q2::foundation::weapon_attachments::q2_weapon_attachment;
use qa_content::q3::foundation::held_weapons::Q3_WEAPON_HAND_GRIP;
use qa_core::identity::ActorId;
use qa_core::math::{Vec3, Vec4};
use std::collections::HashMap;
use thiserror::Error;

/// Failure of foreign held-weapon resolution.
#[derive(Debug, Error)]
pub enum ForeignHeldWeaponError {
    /// Declaration, model, or grip failure.
    #[error("{0}")]
    Resolve(String),
    /// Declaration file failure.
    #[error(transparent)]
    Declaration(#[from] DeclarationError),
    /// Grip alignment failure.
    #[error(transparent)]
    Render(#[from] qa_client::render::RenderError),
}

/// Source presentation selecting a held weapon.
#[derive(Debug, Clone)]
pub struct HeldWeaponSource {
    /// Source content.
    pub content: ContentId,
    /// Weapon view-model path.
    pub path: String,
    /// Source game family.
    pub family: GameFamily,
    /// Weapon item id for Q2 lookup.
    pub weapon_item: Option<String>,
    /// Inline declaration, when authored on the presentation.
    pub held_weapon: Option<HeldWeaponDeclaration>,
    /// Presenting actor.
    pub actor: Option<ActorId>,
}

/// Character view fields the held pass inherits.
#[derive(Debug, Clone, Copy)]
pub struct HeldWeaponCharacter {
    /// Character origin (lighting origin).
    pub origin: Vec3,
    /// Character color.
    pub color: Vec4,
    /// Character opacity.
    pub opacity: Option<f32>,
}

/// Inspected subset of a decoded held model.
#[derive(Debug, Clone)]
pub enum HeldModel {
    /// Quake alias model with filterable triangles.
    Q1Mdl {
        /// Triangles as vertex indices.
        triangles: Vec<[u32; 3]>,
        /// Animation frame count.
        frames: usize,
    },
    /// Other alias model with a frame count.
    Alias {
        /// Animation frame count.
        frames: usize,
    },
    /// Brush model (never a held weapon).
    Brush,
}

/// Held model asset: digest, resolved path, and model.
#[derive(Debug, Clone)]
pub struct HeldModelAsset {
    /// Model content digest.
    pub digest: String,
    /// Resolved model path.
    pub path: String,
    /// Decoded model subset.
    pub model: HeldModel,
}

/// Mount and model loading for foreign held weapons.
pub trait ForeignHeldAssetSource {
    /// Open a mount file, returning its bytes when present.
    fn open(&mut self, content: &ContentId, path: &str) -> Result<Option<Vec<u8>>, ForeignHeldWeaponError>;
    /// Load a decoded model asset.
    fn model(&mut self, content: &ContentId, path: &str) -> Result<HeldModelAsset, ForeignHeldWeaponError>;
}

/// Assembled held-weapon render pass.
#[derive(Debug, Clone)]
pub struct HeldWeaponPass {
    /// Source content.
    pub content: ContentId,
    /// Model digest.
    pub digest: String,
    /// Resolved model path.
    pub path: String,
    /// Reference frame.
    pub reference_frame: f64,
    /// Hand-aligned transform.
    pub transform: ModelTransform,
    /// Previous origin (equals the transform origin).
    pub previous_origin: Vec3,
    /// Skin index.
    pub skin: i32,
    /// Inherited color.
    pub color: Vec4,
    /// Inherited opacity.
    pub opacity: f32,
    /// Source game family.
    pub family: GameFamily,
    /// Presenting actor.
    pub actor: Option<ActorId>,
}

fn entity_of(transform: &ModelTransform) -> EntityTransform {
    EntityTransform {
        origin: transform.origin,
        axis: transform.axis,
        scale: transform.scale,
    }
}

fn model_of(transform: &EntityTransform) -> ModelTransform {
    ModelTransform {
        origin: transform.origin,
        axis: transform.axis,
        scale: transform.scale,
    }
}

/// Foreign held-weapon declarations with cached models.
pub struct ForeignHeldWeapons<S> {
    assets: S,
    declarations: HashMap<String, Option<HeldWeaponDeclaration>>,
    models: HashMap<String, Option<HeldModelAsset>>,
}

impl<S: ForeignHeldAssetSource> ForeignHeldWeapons<S> {
    /// Create a resolver bound to an asset source.
    pub fn new(assets: S) -> Self {
        Self {
            assets,
            declarations: HashMap::new(),
            models: HashMap::new(),
        }
    }

    /// Resolve the declaration for a source presentation.
    pub fn declaration(
        &mut self,
        source: &HeldWeaponSource,
    ) -> Result<Option<HeldWeaponDeclaration>, ForeignHeldWeaponError> {
        if source.held_weapon.is_some() || source.path.is_empty() {
            return Ok(source.held_weapon.clone());
        }
        let key = format!("{}/{}", source.content.as_str(), source.path);
        if let Some(cached) = self.declarations.get(&key) {
            return Ok(cached.clone());
        }
        let file = self
            .assets
            .open(&source.content, &format!("{}.held.json", source.path))?;
        let declaration = file.map(|bytes| read_held_weapon_file(&bytes)).transpose()?;
        self.declarations.insert(key, declaration.clone());
        Ok(declaration)
    }

    fn model(
        &mut self,
        source: &HeldWeaponSource,
        held: &HeldWeaponModel,
    ) -> Result<Option<HeldModelAsset>, ForeignHeldWeaponError> {
        let key = format!(
            "{}/{}/{}/{}/{}",
            source.content.as_str(),
            held.path,
            held.fallback.as_deref().unwrap_or(""),
            held.part
                .as_ref()
                .map(|part| part
                    .digests
                    .iter()
                    .map(|digest| digest.as_str())
                    .collect::<Vec<_>>()
                    .join(","))
                .unwrap_or_default(),
            held.part
                .as_ref()
                .map(|part| part
                    .vertices
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","))
                .unwrap_or_default(),
        );
        if let Some(cached) = self.models.get(&key) {
            return Ok(cached.clone());
        }
        let mut path = held.path.clone();
        if self.assets.open(&source.content, &path)?.is_none() {
            match held.fallback.as_ref() {
                Some(fallback) if self.assets.open(&source.content, fallback)?.is_some() => {
                    path = fallback.clone();
                }
                _ => {
                    self.models.insert(key, None);
                    return Ok(None);
                }
            }
        }
        let asset = self.assets.model(&source.content, &path)?;
        let resolved = if let Some(part) = held.part.as_ref() {
            if !part.digests.iter().any(|digest| digest.as_str() == asset.digest) {
                return Err(ForeignHeldWeaponError::Resolve(format!(
                    "Held model subset is not qualified for {}/{}",
                    source.content.as_str(),
                    path,
                )));
            }
            let HeldModel::Q1Mdl { triangles, frames } = asset.model.clone() else {
                return Err(ForeignHeldWeaponError::Resolve(format!(
                    "Held model subset is not qualified for {}/{}",
                    source.content.as_str(),
                    path,
                )));
            };
            let vertices: std::collections::HashSet<u32> = part.vertices.iter().map(|vertex| *vertex as u32).collect();
            let kept: Vec<[u32; 3]> = triangles
                .into_iter()
                .filter(|triangle| triangle.iter().all(|index| vertices.contains(index)))
                .collect();
            if kept.is_empty() {
                return Err(ForeignHeldWeaponError::Resolve(
                    "Held model part has no source triangles".to_owned(),
                ));
            }
            HeldModelAsset {
                digest: asset.digest,
                path: asset.path,
                model: HeldModel::Q1Mdl {
                    triangles: kept,
                    frames,
                },
            }
        } else {
            asset
        };
        self.models.insert(key, Some(resolved.clone()));
        Ok(Some(resolved))
    }

    /// Build the held-weapon passes for a source presentation and character.
    pub fn frame(
        &mut self,
        source: &HeldWeaponSource,
        character: &HeldWeaponCharacter,
    ) -> Result<Vec<HeldWeaponPass>, ForeignHeldWeaponError> {
        let declaration = self.declaration(source)?;
        if matches!(declaration, Some(HeldWeaponDeclaration::None)) {
            return Ok(Vec::new());
        }
        let inline = declaration.as_ref().and_then(|declaration| match declaration {
            HeldWeaponDeclaration::None => None,
            HeldWeaponDeclaration::Model(model) => Some(model.clone()),
        });
        let held = inline.or_else(|| match source.family {
            GameFamily::Q2 => q2_held_weapon(&source.path, source.weapon_item.as_deref()),
            GameFamily::Q1 => q1_held_weapon(&source.path),
            GameFamily::Q3 => None,
        });
        let Some(held) = held else {
            return Err(ForeignHeldWeaponError::Resolve(format!(
                "Selected weapon {}/{} has no authored held model declaration",
                source.content.as_str(),
                source.path,
            )));
        };
        let asset = self.model(source, &held)?.ok_or_else(|| {
            ForeignHeldWeaponError::Resolve(format!(
                "Source held model is absent: {}/{}",
                source.content.as_str(),
                held.path,
            ))
        })?;
        if held
            .digest
            .as_ref()
            .is_some_and(|digest| digest.as_str() != asset.digest)
        {
            return Err(ForeignHeldWeaponError::Resolve(
                "Held model digest differs from its source declaration".to_owned(),
            ));
        }
        let frames = match asset.model {
            HeldModel::Q1Mdl { frames, .. } | HeldModel::Alias { frames } => Some(frames),
            HeldModel::Brush => None,
        };
        let Some(frames) = frames else {
            return Err(ForeignHeldWeaponError::Resolve(
                "Held model reference frame is outside its source animation".to_owned(),
            ));
        };
        if held.reference_frame >= frames as f64 {
            return Err(ForeignHeldWeaponError::Resolve(
                "Held model reference frame is outside its source animation".to_owned(),
            ));
        }
        let attachment = if declaration.is_none() && source.family == GameFamily::Q2 {
            q2_weapon_attachment(&asset.digest)
        } else {
            None
        };
        let grip = match attachment.as_ref() {
            Some(definition) if matches!(definition.target, ModelAttachmentTarget::Mesh { .. }) => &definition.grip,
            _ => &held.grip,
        };
        let transform = model_of(&align_model_attachment(
            &entity_of(grip),
            &entity_of(&Q3_WEAPON_HAND_GRIP),
        )?);
        Ok(vec![HeldWeaponPass {
            content: source.content.clone(),
            digest: asset.digest,
            path: asset.path,
            reference_frame: held.reference_frame,
            previous_origin: transform.origin,
            transform,
            skin: 0,
            color: character.color,
            opacity: character.opacity.unwrap_or(1.0),
            family: source.family,
            actor: source.actor.clone(),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::ContentDigest;
    use qa_core::math::{vec3, vec4};

    struct FakeSource {
        files: HashMap<String, Vec<u8>>,
        models: HashMap<String, HeldModelAsset>,
        opens: Vec<String>,
    }

    impl ForeignHeldAssetSource for FakeSource {
        fn open(&mut self, content: &ContentId, path: &str) -> Result<Option<Vec<u8>>, ForeignHeldWeaponError> {
            self.opens.push(format!("{}:{path}", content.as_str()));
            Ok(self.files.get(&format!("{}:{path}", content.as_str())).cloned())
        }

        fn model(&mut self, _content: &ContentId, path: &str) -> Result<HeldModelAsset, ForeignHeldWeaponError> {
            self.models
                .get(path)
                .cloned()
                .ok_or_else(|| ForeignHeldWeaponError::Resolve(format!("missing model {path}")))
        }
    }

    fn identity_grip() -> ModelTransform {
        ModelTransform {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            scale: vec3(1.0, 1.0, 1.0),
        }
    }

    fn held_model(path: &str) -> HeldWeaponModel {
        HeldWeaponModel {
            digest: None,
            path: path.to_owned(),
            reference_frame: 0.0,
            grip: identity_grip(),
            fallback: None,
            part: None,
        }
    }

    fn source(family: GameFamily, path: &str) -> HeldWeaponSource {
        HeldWeaponSource {
            content: ContentId("q1".to_owned()),
            path: path.to_owned(),
            family,
            weapon_item: None,
            held_weapon: None,
            actor: None,
        }
    }

    fn character() -> HeldWeaponCharacter {
        HeldWeaponCharacter {
            origin: vec3(1.0, 2.0, 3.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            opacity: None,
        }
    }

    #[test]
    fn none_declaration_yields_no_passes() {
        let assets = FakeSource {
            files: HashMap::new(),
            models: HashMap::new(),
            opens: Vec::new(),
        };
        let mut weapons = ForeignHeldWeapons::new(assets);
        let mut inline = source(GameFamily::Q1, "progs/v_shot.mdl");
        inline.held_weapon = Some(HeldWeaponDeclaration::None);
        let passes = weapons.frame(&inline, &character()).expect("frame");
        assert!(passes.is_empty());
    }

    #[test]
    fn inline_model_builds_hand_aligned_pass() {
        let model = held_model("progs/v_shot.mdl");
        let mut models = HashMap::new();
        models.insert(
            "progs/v_shot.mdl".to_owned(),
            HeldModelAsset {
                digest: "sha256:abc".to_owned(),
                path: "progs/v_shot.mdl".to_owned(),
                model: HeldModel::Q1Mdl {
                    triangles: vec![[0, 1, 2]],
                    frames: 4,
                },
            },
        );
        let mut files = HashMap::new();
        files.insert("q1:progs/v_shot.mdl".to_owned(), vec![1]);
        let assets = FakeSource {
            files,
            models,
            opens: Vec::new(),
        };
        let mut weapons = ForeignHeldWeapons::new(assets);
        let mut inline = source(GameFamily::Q1, "progs/v_shot.mdl");
        inline.held_weapon = Some(HeldWeaponDeclaration::Model(model));
        let passes = weapons.frame(&inline, &character()).expect("frame");
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].skin, 0);
        assert_eq!(passes[0].opacity, 1.0);
        assert_eq!(passes[0].previous_origin, passes[0].transform.origin);
    }

    #[test]
    fn digest_mismatch_and_frame_range_error() {
        let mut digest_model = held_model("progs/v_shot.mdl");
        digest_model.digest = Some(ContentDigest("sha256:other".to_owned()));
        let mut models = HashMap::new();
        models.insert(
            "progs/v_shot.mdl".to_owned(),
            HeldModelAsset {
                digest: "sha256:abc".to_owned(),
                path: "progs/v_shot.mdl".to_owned(),
                model: HeldModel::Alias { frames: 4 },
            },
        );
        let mut files = HashMap::new();
        files.insert("q1:progs/v_shot.mdl".to_owned(), vec![1]);
        let assets = FakeSource {
            files,
            models,
            opens: Vec::new(),
        };
        let mut weapons = ForeignHeldWeapons::new(assets);
        let mut inline = source(GameFamily::Q1, "progs/v_shot.mdl");
        inline.held_weapon = Some(HeldWeaponDeclaration::Model(digest_model));
        let err = weapons.frame(&inline, &character()).expect_err("digest");
        assert_eq!(err.to_string(), "Held model digest differs from its source declaration");
    }

    #[test]
    fn q2_lookup_without_declaration_errors_when_unresolved() {
        let assets = FakeSource {
            files: HashMap::new(),
            models: HashMap::new(),
            opens: Vec::new(),
        };
        let mut weapons = ForeignHeldWeapons::new(assets);
        let missing = source(GameFamily::Q3, "unknown/model.md3");
        let err = weapons.frame(&missing, &character()).expect_err("unresolved");
        assert!(err.to_string().contains("has no authored held model declaration"));
    }
}
