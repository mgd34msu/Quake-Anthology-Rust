//! Alias model decoding with MD5 replacement variants.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/model-loader.ts`
//! (`ApplicationModelProvider`, `LoadedApplicationModel`,
//! `ApplicationModelVariants`, `loadApplicationModel`). Direct port over
//! the workspace's Q1/Q2/MD3/MD5 decoders and replacement helpers. Q1/Q2
//! dispatch reads magic bytes like `parseQ12Model`; mounts and texture
//! loads arrive through local traits.

use qa_content::common::TimedFrames;
use qa_content::contract::{GameFamily, ResolvedResourceReference, ResourceResolution};
use qa_content::images::indexed::decode_qpic;
use qa_content::md2::{parse_md2, Md2Model};
use qa_content::md3::parse_md3;
use qa_content::md5::{
    create_md5_model, parse_md5_anim, parse_md5_mesh, DecodedMd5Model, Md5ScaleSource, SkinSelection,
};
use qa_content::mdl::{parse_mdl, MdlModel};
use qa_content::mounts::OpenedResource;
use qa_content::q3scene::{to_scene_md3, SceneMd3};
use qa_content::replacements::{
    md2_replacement_skin_selection, md5_paths_for, md5_replacement_allowed, q1_replacement_skin_selection, QFamily,
};
use qa_core::binary::BinaryError;
use std::collections::HashSet;
use thiserror::Error;

/// Failure of model loading.
#[derive(Debug, Error)]
pub enum ModelLoaderError {
    /// Model misuse.
    #[error("{0}")]
    Model(String),
    /// Decode failure.
    #[error(transparent)]
    Binary(#[from] BinaryError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] qa_content::mounts::MountError),
    /// Texture failure.
    #[error("{0}")]
    Texture(String),
}

/// Mount opening for model loading.
pub trait ModelMounts {
    /// Open a resource, returning it when present.
    fn open(&mut self, path: &str) -> Result<Option<OpenedResource>, ModelLoaderError>;
}

/// Texture loading for replacement skins.
pub trait ModelTextureLoader {
    /// Load a skin texture.
    fn load(&mut self, name: &str, family: GameFamily, usage: &'static str) -> Result<(), ModelLoaderError>;
}

/// Model loading provider: family, mounts, and textures.
pub struct ApplicationModelProvider<M, T> {
    /// Content family.
    pub family: GameFamily,
    /// Mounts.
    pub mounts: M,
    /// Texture loader.
    pub textures: T,
}

/// Decoded native model.
#[derive(Debug, Clone)]
pub enum LoadedModel {
    /// Quake alias model.
    Q1Mdl(MdlModel),
    /// Quake II alias model.
    Q2Md2(Md2Model),
    /// Quake III mesh model.
    Q3Md3(SceneMd3),
}

/// Committed MD5 replacement pair.
#[derive(Debug, Clone)]
pub struct ModelReplacement {
    /// Mesh resource.
    pub resource: ResolvedResourceReference,
    /// Replacement model.
    pub model: DecodedMd5Model,
}

/// Loaded model with optional replacement variants.
#[derive(Debug, Clone)]
pub struct LoadedApplicationModel {
    /// Source resource.
    pub resource: ResolvedResourceReference,
    /// Decoded model.
    pub model: LoadedModel,
    /// Replacement variants, for matching-family alias models.
    pub variants: Option<ApplicationModelVariants>,
}

/// Retained scene entities observe only successfully committed replacement pairs.
#[derive(Debug, Clone)]
pub struct ApplicationModelVariants {
    resource: ResolvedResourceReference,
    native: LoadedModel,
    current: Option<ModelReplacement>,
}

impl ApplicationModelVariants {
    /// Committed replacement, if any.
    #[must_use]
    pub fn replacement(&self) -> Option<&ModelReplacement> {
        self.current.as_ref()
    }

    /// Prepare a replacement, returning its commit token.
    pub fn prepare_replacement<M: ModelMounts, T: ModelTextureLoader>(
        &self,
        provider: &mut ApplicationModelProvider<M, T>,
        enabled: bool,
    ) -> Result<ReplacementCommit, ModelLoaderError> {
        let replacement = if !enabled {
            None
        } else {
            match &self.native {
                LoadedModel::Q1Mdl(alias) => q1_replacement(provider, &self.resource, alias)?,
                LoadedModel::Q2Md2(alias) => q2_replacement(provider, &self.resource, alias)?,
                LoadedModel::Q3Md3(_) => None,
            }
        };
        if let Some(replacement) = replacement.as_ref() {
            if let SkinSelection::Q2Md2Replacement { skins, .. } = &replacement.model.skin_selection {
                for skin in skins {
                    provider.textures.load(skin, GameFamily::Q2, "skin")?;
                }
            }
        }
        Ok(ReplacementCommit { replacement })
    }

    /// Commit a prepared replacement.
    pub fn commit(&mut self, commit: ReplacementCommit) {
        self.current = commit.replacement;
    }
}

/// Prepared replacement awaiting commit.
#[derive(Debug)]
pub struct ReplacementCommit {
    replacement: Option<ModelReplacement>,
}

fn rank(resource: &ResolvedResourceReference) -> Option<u32> {
    match &resource.resolution {
        ResourceResolution::Link { .. } => None,
        ResourceResolution::DefaultOrder { rank, .. } | ResourceResolution::PrefixOrder { rank, .. } => {
            Some(*rank as u32)
        }
    }
}

fn q1_replacement<M: ModelMounts, T: ModelTextureLoader>(
    provider: &mut ApplicationModelProvider<M, T>,
    resource: &ResolvedResourceReference,
    alias: &MdlModel,
) -> Result<Option<ModelReplacement>, ModelLoaderError> {
    let paths = md5_paths_for(&resource.requested_path, QFamily::Q1);
    let mesh = provider.mounts.open(&paths.mesh)?;
    let Some(mesh) = mesh else {
        return Ok(None);
    };
    if !md5_replacement_allowed(rank(resource), rank(&mesh.reference)) {
        return Ok(None);
    }
    let animation = provider.mounts.open(&paths.animation)?;
    let Some(animation) = animation else {
        return Ok(None);
    };
    let parsed = (|| {
        let mesh_text = String::from_utf8(mesh.bytes.clone())
            .map_err(|_| ModelLoaderError::Model("Invalid MD5 mesh text".to_owned()))?;
        let anim_text = String::from_utf8(animation.bytes.clone())
            .map_err(|_| ModelLoaderError::Model("Invalid MD5 animation text".to_owned()))?;
        let parsed_mesh = parse_md5_mesh(&mesh_text, &paths.mesh)?;
        let parsed_anim = parse_md5_anim(&anim_text, &paths.animation, None)?;
        let shaders: Vec<String> = parsed_mesh.meshes.iter().map(|mesh| mesh.shader.clone()).collect();
        let selection = q1_replacement_skin_selection(&shaders, alias, parsed_anim.frames.len());
        Ok::<DecodedMd5Model, ModelLoaderError>(create_md5_model(parsed_mesh, parsed_anim, selection))
    })();
    // Broken retail pairs, including mg3's ogre_rocket, keep their MDL.
    let model = match parsed {
        Ok(model) => model,
        Err(ModelLoaderError::Binary(_)) | Err(ModelLoaderError::Model(_)) => return Ok(None),
        Err(error) => return Err(error),
    };
    if !matches!(model.skin_selection, SkinSelection::Q1MdlReplacement { .. }) {
        return Err(ModelLoaderError::Model(
            "Q1 replacement lost its alias skin selection".to_owned(),
        ));
    }
    let mut skins = HashSet::new();
    if let SkinSelection::Q1MdlReplacement { mesh_skin_groups, .. } = &model.skin_selection {
        for groups in mesh_skin_groups {
            for group in groups {
                match group {
                    TimedFrames::Single(frame) => {
                        skins.insert(frame.clone());
                    }
                    TimedFrames::Group(frames) => {
                        for frame in frames {
                            skins.insert(frame.frame.clone());
                        }
                    }
                }
            }
        }
    }
    for name in skins {
        // Both donor renderers require the indexed sidecar before attaching a pair.
        let skin = provider.mounts.open(&format!("{name}.lmp"))?;
        if skin.is_none() {
            return Ok(None);
        }
        let skin = skin.expect("checked above");
        if decode_qpic(&skin.bytes, &format!("{name}.lmp")).is_err() {
            return Ok(None);
        }
        provider.textures.load(&format!("{name}.lmp"), GameFamily::Q1, "skin")?;
    }
    Ok(Some(ModelReplacement {
        resource: mesh.reference,
        model,
    }))
}

fn q2_replacement<M: ModelMounts, T: ModelTextureLoader>(
    provider: &mut ApplicationModelProvider<M, T>,
    resource: &ResolvedResourceReference,
    alias: &Md2Model,
) -> Result<Option<ModelReplacement>, ModelLoaderError> {
    let paths = md5_paths_for(&resource.requested_path, QFamily::Q2);
    let mesh = provider.mounts.open(&paths.mesh)?;
    let Some(mesh) = mesh else {
        return Ok(None);
    };
    if !md5_replacement_allowed(rank(resource), rank(&mesh.reference)) {
        return Ok(None);
    }
    let animation = provider.mounts.open(&paths.animation)?;
    let Some(animation) = animation else {
        return Ok(None);
    };
    let scale = provider.mounts.open(&paths.scales)?;
    let parsed = (|| {
        let mesh_text = String::from_utf8(mesh.bytes.clone())
            .map_err(|_| ModelLoaderError::Model("Invalid MD5 mesh text".to_owned()))?;
        let anim_text = String::from_utf8(animation.bytes.clone())
            .map_err(|_| ModelLoaderError::Model("Invalid MD5 animation text".to_owned()))?;
        let scale_source = match scale.as_ref() {
            Some(scale) => {
                let text = String::from_utf8(scale.bytes.clone())
                    .map_err(|_| ModelLoaderError::Model("Invalid MD5 scale text".to_owned()))?;
                Some(Md5ScaleSource {
                    source: paths.scales.clone(),
                    text,
                })
            }
            None => None,
        };
        let parsed = parse_md5_anim(&anim_text, &paths.animation, scale_source.as_ref())?;
        let mut diagnostics = parsed.diagnostics.clone();
        if parsed.frames.len() < alias.frames.len() {
            diagnostics.push(format!(
                "{} has fewer frames than {} ({} < {})",
                paths.animation,
                resource.requested_path,
                parsed.frames.len(),
                alias.frames.len()
            ));
        }
        Ok::<DecodedMd5Model, ModelLoaderError>(create_md5_model(
            parse_md5_mesh(&mesh_text, &paths.mesh)?,
            parsed,
            md2_replacement_skin_selection(alias, scale.as_ref().map(|_| paths.scales.clone()), diagnostics),
        ))
    })();
    match parsed {
        Ok(model) => Ok(Some(ModelReplacement {
            resource: mesh.reference,
            model,
        })),
        Err(ModelLoaderError::Binary(_)) | Err(ModelLoaderError::Model(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Decode the requested alias first: its source flags, skins and frames remain authoritative.
pub fn load_application_model<M: ModelMounts, T: ModelTextureLoader>(
    provider: &mut ApplicationModelProvider<M, T>,
    asset: &OpenedResource,
    enhanced_models: bool,
) -> Result<LoadedApplicationModel, ModelLoaderError> {
    let path = asset.reference.requested_path.clone();
    let model = if path.to_lowercase().ends_with(".md3") {
        LoadedModel::Q3Md3(to_scene_md3(parse_md3(&asset.bytes, &path)?.model))
    } else {
        let magic = asset.bytes.get(..4).unwrap_or_default();
        if magic == b"IDPO" {
            LoadedModel::Q1Mdl(parse_mdl(&asset.bytes, &path)?)
        } else if magic == b"IDP2" {
            LoadedModel::Q2Md2(parse_md2(&asset.bytes, &path)?)
        } else {
            return Err(ModelLoaderError::Model(format!("Unknown Q1/Q2 model magic {magic:?}")));
        }
    };
    if provider.family == GameFamily::Q1 && matches!(model, LoadedModel::Q1Mdl(_))
        || provider.family == GameFamily::Q2 && matches!(model, LoadedModel::Q2Md2(_))
    {
        let mut variants = ApplicationModelVariants {
            resource: asset.reference.clone(),
            native: model.clone(),
            current: None,
        };
        let commit = variants.prepare_replacement(provider, enhanced_models)?;
        variants.commit(commit);
        return Ok(LoadedApplicationModel {
            resource: asset.reference.clone(),
            model: variants.native.clone(),
            variants: Some(variants),
        });
    }
    Ok(LoadedApplicationModel {
        resource: asset.reference.clone(),
        model,
        variants: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{ContentDigest, ResourceId, ResourceProvenance};
    use std::collections::HashMap;

    struct FakeMounts {
        files: HashMap<String, OpenedResource>,
    }

    impl ModelMounts for FakeMounts {
        fn open(&mut self, path: &str) -> Result<Option<OpenedResource>, ModelLoaderError> {
            Ok(self.files.get(path).cloned())
        }
    }

    struct FakeTextures {
        loads: Vec<String>,
    }

    impl ModelTextureLoader for FakeTextures {
        fn load(&mut self, name: &str, _family: GameFamily, _usage: &'static str) -> Result<(), ModelLoaderError> {
            self.loads.push(name.to_owned());
            Ok(())
        }
    }

    fn resource(path: &str) -> ResolvedResourceReference {
        ResolvedResourceReference {
            id: ResourceId(format!("resource:{path}")),
            requested_path: path.to_owned(),
            provenance: ResourceProvenance::Loose {
                mount: qa_content::contract::LooseMount {
                    identity: qa_content::contract::MountIdentity {
                        id: qa_content::contract::MountId("mount:test:loose".to_owned()),
                        content: qa_content::contract::ContentId("q1".to_owned()),
                        generation: 1,
                    },
                    root_path: "/corpus".to_owned(),
                },
                member_path: path.to_owned(),
            },
            digest: ContentDigest("sha256:00".to_owned()),
            byte_length: 0,
            resolution: ResourceResolution::DefaultOrder {
                plan: qa_content::contract::MountPlanId("mount-plan:test:1".to_owned()),
                rank: 0,
            },
        }
    }

    #[test]
    fn unknown_magic_errors() {
        let mut provider = ApplicationModelProvider {
            family: GameFamily::Q1,
            mounts: FakeMounts { files: HashMap::new() },
            textures: FakeTextures { loads: Vec::new() },
        };
        let asset = OpenedResource::new(resource("progs/bogus.mdl"), b"NOPE".to_vec());
        let err = load_application_model(&mut provider, &asset, true).expect_err("magic");
        assert!(err.to_string().contains("Unknown Q1/Q2 model magic"));
    }

    fn mdl_fixture() -> Vec<u8> {
        use qa_core::binary::BinaryWriter;
        let mut writer = BinaryWriter::new(1024);
        writer.bytes(b"IDPO").unwrap();
        writer.i32(6).unwrap();
        for value in [0.5f32, 0.5, 0.5] {
            writer.f32(value).unwrap();
        }
        for value in [1.0f32, 2.0, 3.0] {
            writer.f32(value).unwrap();
        }
        writer.f32(10.0).unwrap();
        for value in [0.0f32, 0.0, 0.0] {
            writer.f32(value).unwrap();
        }
        writer.i32(1).unwrap();
        writer.i32(4).unwrap();
        writer.i32(4).unwrap();
        writer.i32(3).unwrap();
        writer.i32(1).unwrap();
        writer.i32(2).unwrap();
        writer.i32(0).unwrap();
        writer.i32(0).unwrap();
        writer.f32(1.0).unwrap();
        writer.i32(0).unwrap();
        writer.bytes(&[7u8; 16]).unwrap();
        for seam in [0, 1, 0] {
            writer.i32(seam).unwrap();
            writer.i32(8).unwrap();
            writer.i32(12).unwrap();
        }
        writer.i32(1).unwrap();
        writer.i32(0).unwrap();
        writer.i32(1).unwrap();
        writer.i32(2).unwrap();
        writer.i32(0).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        let mut name = [0u8; 16];
        name[..5].copy_from_slice(b"frame");
        writer.bytes(&name).unwrap();
        writer.bytes(&[0, 0, 0, 5, 2, 2, 2, 5, 4, 4, 4, 5]).unwrap();
        writer.i32(1).unwrap();
        writer.i32(1).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        writer.f32(0.1).unwrap();
        writer.bytes(&[0, 0, 0, 0, 4, 4, 4, 0]).unwrap();
        writer.bytes(&name).unwrap();
        writer.bytes(&[1, 1, 1, 5, 2, 2, 2, 5, 3, 3, 3, 5]).unwrap();
        writer.finish()
    }

    #[test]
    fn missing_replacement_keeps_native_model() {
        let mut provider = ApplicationModelProvider {
            family: GameFamily::Q1,
            mounts: FakeMounts { files: HashMap::new() },
            textures: FakeTextures { loads: Vec::new() },
        };
        let asset = OpenedResource::new(resource("progs/soldier.mdl"), mdl_fixture());
        let loaded = load_application_model(&mut provider, &asset, true).expect("load");
        assert!(matches!(loaded.model, LoadedModel::Q1Mdl(_)));
        let variants = loaded.variants.expect("variants");
        assert!(variants.replacement().is_none());
        assert!(provider.textures.loads.is_empty());
        let loaded = load_application_model(&mut provider, &asset, false).expect("load");
        assert!(loaded.variants.expect("variants").replacement().is_none());
    }

    #[test]
    fn foreign_family_skips_variants() {
        let mut provider = ApplicationModelProvider {
            family: GameFamily::Q3,
            mounts: FakeMounts { files: HashMap::new() },
            textures: FakeTextures { loads: Vec::new() },
        };
        let asset = OpenedResource::new(resource("progs/soldier.mdl"), mdl_fixture());
        let loaded = load_application_model(&mut provider, &asset, true).expect("load");
        assert!(loaded.variants.is_none());
    }
}
