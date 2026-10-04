//! Windowed Q3 shader scripts: discovery, parsing, and skin resolution.
//!
//! Donor provenance: `src/app/bootstrap/assets.ts` (`shaderPaths` plus
//! `loadScripts`: sorted top-level `scripts/*.shader` across archives and
//! loose mounts) and `src/render/scene/shaders.ts` (`addScript`,
//! `initializeSourceMaterials`: first definition wins per name).
//!
//! The windowed presentation used to build its [`SceneShaderRegistry`]
//! empty, so world surfaces and MD3 skins naming authored shaders with no
//! image file (e.g. `models/weapons2/plasma/plasma_glass`) fell back to the
//! missing handle and implicit materials: glow/transparency/anim stages
//! never evaluated. [`read_shader_scripts`] discovers the installed scripts
//! through the product mounts, [`load_registry_scripts`] parses them into
//! the registry before the world scene builds (so affected world batches
//! evaluate their authored blend/glow/anim stages), and [`ShaderImageIndex`]
//! resolves MD3 shader names to the shader's representative stage image so
//! model skins bind decoded bytes instead of the missing handle.

use std::collections::HashMap;

use qa_client::materials::material::{
    inspect_shader_script, normalize_shader_name, ShaderDefinition, ShaderEntryResult, ShaderMap,
};
use qa_client::render::scene::shaders::SceneShaderRegistry;
use qa_client::render::RenderError;
use qa_client::ClientError;
use qa_content::mounts::{MountError, MountedContent};

/// One discovered shader script: mount path plus decoded text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderScript {
    /// Mount path (`scripts/<name>.shader`).
    pub path: String,
    /// Decoded script text.
    pub text: String,
}

/// Sorted top-level `scripts/*.shader` paths across archives and loose
/// mounts (donor `shaderPaths`: `/^scripts\/[^/]+\.shader$/i`, sorted).
pub fn discover_shader_script_paths(mounts: &MountedContent) -> Result<Vec<String>, MountError> {
    let mut paths: Vec<String> = mounts
        .list_files("scripts", ".shader")?
        .into_iter()
        .filter(|tail| !tail.contains('/') && !tail.contains('\\'))
        .map(|tail| format!("scripts/{tail}"))
        .collect();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Read every discovered script, skipping paths that no longer open
/// (donor `loadScripts` null check).
pub fn read_shader_scripts(mounts: &MountedContent) -> Result<Vec<ShaderScript>, MountError> {
    let mut scripts = Vec::new();
    for path in discover_shader_script_paths(mounts)? {
        if let Some(asset) = mounts.open(&path, |_| true)? {
            scripts.push(ShaderScript {
                path,
                text: String::from_utf8_lossy(&asset.bytes).into_owned(),
            });
        }
    }
    Ok(scripts)
}

/// Load scripts into a registry: Quake III initializes source materials
/// over the scripts (donor `initializeSourceMaterials(loadScripts)`),
/// other families only add the scripts (donor `loadScripts`).
pub fn load_registry_scripts(
    registry: &mut SceneShaderRegistry,
    scripts: &[ShaderScript],
    q3: bool,
) -> Result<(), RenderError> {
    if q3 {
        let borrowed: Vec<(&str, &str)> = scripts
            .iter()
            .map(|script| (script.text.as_str(), script.path.as_str()))
            .collect();
        registry.initialize_source_materials(&borrowed)?;
    } else {
        for script in scripts {
            registry.add_script(&script.text, &script.path)?;
        }
    }
    Ok(())
}

/// Representative image for an authored MD3 shader name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkinResolution {
    /// Load this stage image through the texture loader.
    Image(String),
    /// Special-map-only shader (`$lightmap`, `$whiteimage`): the white
    /// handle (models have no lightmap; the unlit binding is white).
    White,
    /// No loadable stage image: fall back to the missing handle.
    Missing,
}

/// Representative stage image per authored shader name, keyed by the
/// normalized shader name (donor `normalizeShaderName`, so explicit
/// extensions and case match the registry lookup).
///
/// First definition wins per name; within a definition the first stage
/// with an image map wins (image name, animation first frame). This is
/// the single-image counterpart of the registry's full multi-stage
/// registration, which the windowed model path cannot express: model
/// batches bind one skin image, so the index resolves that binding while
/// world batches evaluate every authored stage.
#[derive(Debug, Clone, Default)]
pub struct ShaderImageIndex {
    entries: HashMap<String, SkinResolution>,
}

impl ShaderImageIndex {
    /// Parse scripts into an index, first definition wins per name.
    pub fn build(scripts: &[ShaderScript]) -> Result<Self, ClientError> {
        let mut entries = HashMap::new();
        for script in scripts {
            for entry in inspect_shader_script(&script.text, &script.path)? {
                let ShaderEntryResult::Accepted(definition) = entry.result else {
                    continue;
                };
                entries
                    .entry(normalize_shader_name(&entry.name))
                    .or_insert_with(|| representative_image(&definition));
            }
        }
        Ok(Self { entries })
    }

    /// Resolve an MD3 shader name: `None` when no authored definition
    /// exists (the caller keeps the legacy load-by-name behavior).
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<&SkinResolution> {
        self.entries.get(&normalize_shader_name(name))
    }

    /// Whether any authored definition exists for the name.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(&normalize_shader_name(name))
    }

    /// Number of indexed shader names.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index holds no shader names.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Representative stage image of one definition: the first image or
/// animation map in stage order, else white for special-map-only
/// shaders, else missing.
fn representative_image(definition: &ShaderDefinition) -> SkinResolution {
    let mut special = false;
    for stage in &definition.stages {
        match &stage.stage.map {
            ShaderMap::Image { name, .. } => return SkinResolution::Image(name.clone()),
            ShaderMap::Animation { frames, .. } => {
                if let Some(first) = frames.first() {
                    return SkinResolution::Image(first.clone());
                }
            }
            ShaderMap::Lightmap | ShaderMap::WhiteImage => special = true,
            ShaderMap::Video { .. } | ShaderMap::None => {}
        }
    }
    if special {
        SkinResolution::White
    } else {
        SkinResolution::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(path: &str, text: &str) -> ShaderScript {
        ShaderScript {
            path: path.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn index_resolves_first_image_stage_and_special_maps() {
        let scripts = vec![script(
            "scripts/test.shader",
            r"
models/weapons2/plasma/plasma_glass
{
    {
        map textures/effects/tinfxb.tga
        tcGen environment
        blendfunc GL_ONE GL_ONE
        rgbGen lightingDiffuse
    }
}
textures/sfx/anim
{
    {
        animMap 4 textures/a.tga textures/b.tga
        blendFunc GL_ONE GL_ONE
    }
}
textures/sfx/lightonly
{
    {
        map $lightmap
        rgbGen identity
    }
}
textures/sfx/video
{
    {
        videoMap intro.roq
    }
}
",
        )];
        let index = ShaderImageIndex::build(&scripts).expect("index builds");
        assert_eq!(index.len(), 4);
        assert_eq!(
            index.resolve("models/weapons2/plasma/plasma_glass"),
            Some(&SkinResolution::Image("textures/effects/tinfxb.tga".to_string()))
        );
        // Explicit extensions and case fold to the registry lookup key.
        assert_eq!(
            index.resolve("MODELS/weapons2/plasma/plasma_glass.TGA"),
            Some(&SkinResolution::Image("textures/effects/tinfxb.tga".to_string()))
        );
        assert_eq!(
            index.resolve("textures/sfx/anim"),
            Some(&SkinResolution::Image("textures/a.tga".to_string()))
        );
        assert_eq!(index.resolve("textures/sfx/lightonly"), Some(&SkinResolution::White));
        assert_eq!(index.resolve("textures/sfx/video"), Some(&SkinResolution::Missing));
        assert_eq!(index.resolve("textures/sfx/absent"), None);
        assert!(index.contains("textures/sfx/anim"));
        assert!(!index.is_empty());
    }

    #[test]
    fn first_definition_wins_per_name() {
        let scripts = vec![
            script("scripts/a.shader", "textures/dup\n{\n\t{\n\t\tmap first.tga\n\t}\n}\n"),
            script("scripts/b.shader", "textures/dup\n{\n\t{\n\t\tmap second.tga\n\t}\n}\n"),
        ];
        let index = ShaderImageIndex::build(&scripts).expect("index builds");
        assert_eq!(index.len(), 1);
        assert_eq!(
            index.resolve("textures/dup"),
            Some(&SkinResolution::Image("first.tga".to_string()))
        );
    }

    #[test]
    fn rejected_definitions_do_not_shadow_later_valid_ones() {
        let scripts = vec![
            script(
                "scripts/broken.shader",
                "textures/ok\n{\n\t{\n\t\tmap $whiteimage\n\t}\n",
            ),
            script("scripts/good.shader", "textures/ok\n{\n\t{\n\t\tmap good.tga\n\t}\n}\n"),
        ];
        let index = ShaderImageIndex::build(&scripts).expect("index builds");
        assert_eq!(
            index.resolve("textures/ok"),
            Some(&SkinResolution::Image("good.tga".to_string()))
        );
    }
}
