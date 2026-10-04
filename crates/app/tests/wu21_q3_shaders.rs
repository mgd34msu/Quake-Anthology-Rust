//! wu-21: authored Q3 `.shader` scripts load so glow/transparency surfaces render.
//!
//! Before this lane the windowed presentation built its shader registry
//! empty, so MD3/world surfaces naming authored shaders with no image file
//! (e.g. `models/weapons2/plasma/plasma_glass`) bound the missing handle
//! and implicit materials. These tests pin discovery plus parsing from the
//! installed paks/pk3s, registry application of blend/glow/anim stages, and
//! the q3dm1 spawn-view effect, and capture a windowed frame for the
//! before/after pixel diff.
//!
//! Corpus discovery walks up from this crate (wu-12 pattern), so the tests
//! run both in-tree and from an isolated worktree; corpus-backed tests skip
//! when the Steel corpus is absent, and the capture test additionally
//! requires `WU21_CAPTURE_OUT` (raw RGBA output path).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use qa_app::bootstrap::startup::StartupEntry;
use qa_app::bootstrap::windowed::{drive_windowed_application, open_windowed_application};
use qa_app::bootstrap::windowed_shaders::{
    discover_shader_script_paths, load_registry_scripts, read_shader_scripts, ShaderImageIndex, ShaderScript,
    SkinResolution,
};
use qa_app::bootstrap::windowed_world::load_windowed_world;
use qa_app::options::ApplicationOptions;
use qa_app::startup::StartupConfig;
use qa_client::materials::material::{
    inspect_shader_script, AlphaGen, ColorGen, ShaderEntryResult, ShaderMap, TexGen, TexMod,
};
use qa_client::materials::state::{ADDITIVE_BLEND, FILTER_BLEND, OPAQUE_BLEND};
use qa_client::render::scene::image_policy::ImageUsage;
use qa_client::render::scene::material_registrations::alloc_world_identity;
use qa_client::render::scene::resources::SceneImageRegistry;
use qa_client::render::scene::shaders::{SceneShaderBinding, SceneShaderRegistry};
use qa_client::render::scene::textures::{
    DecodedSceneImage, RgbaLevel, SceneAsset, SceneAssetReader, SceneImageDecoder, SceneTextureLoadOptions,
    SceneTextureLoader, TextureFamily,
};
use qa_client::render::types::{
    BatchVertices, BlendFactor, ImageSource, RenderOperation, ResourceOwner, SourceTime, ViewTarget,
};
use qa_client::render::RenderError;
use qa_client::view::{perspective_projection, CameraClip, Rect as ViewRect, SceneCamera};
use qa_content::catalog::{discover_installed_content, DiscoverContentOptions};
use qa_content::contract::{create_mount_plan_id, ResolvedMountPlan};
use qa_content::mounts::{open_mount_plan, MountedContent, OpenMountOptions};
use qa_core::identity::IdentityOwner;
use qa_core::math::{angles_to_axis, vec3};

/// Witness files proving a Steel corpus root holds all three families.
const CORPUS_WITNESSES: [&str; 3] = ["q1/id1/pak0.pak", "q2/baseq2/pak0.pak", "q3a/baseq3/pak0.pk3"];

/// Locate the Steel corpus root without hardcoding any absolute path.
fn find_steel_corpus() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("target"))
        .find(|root| CORPUS_WITNESSES.iter().all(|witness| root.join(witness).is_file()))
}

/// Open one installed product's mounts once (mirrors the production
/// `open_product_mounts`, which is crate-private).
fn open_product_mounts(corpus: &Path, content: &str) -> MountedContent {
    let catalog = discover_installed_content(&DiscoverContentOptions::new(corpus.to_path_buf())).unwrap();
    let mounts = catalog.mounts_for(content).unwrap();
    let plan = ResolvedMountPlan {
        id: create_mount_plan_id("wu21", content).unwrap(),
        mounts: mounts.clone(),
        default_order: mounts.iter().map(|mount| mount.identity().id.clone()).collect(),
        prefix_orders: Vec::new(),
    };
    open_mount_plan(&plan, OpenMountOptions::default()).unwrap()
}

/// Asset reader over opened mounts (mirrors the production catalog reader).
struct MountReader {
    mounts: MountedContent,
}

impl SceneAssetReader for MountReader {
    fn read(&self, path: &str) -> Result<Option<SceneAsset>, RenderError> {
        let asset = self
            .mounts
            .open(path, |_| true)
            .map_err(|error| RenderError::Backend(error.to_string()))?;
        Ok(asset.map(|asset| SceneAsset {
            bytes: asset.bytes,
            source: ImageSource::Resource {
                requested_path: asset.reference.requested_path,
            },
        }))
    }
}

/// Test decoder accepting any bytes as a 2x2 image (format decoders are
/// production-owned; tests only need load/no-load outcomes).
struct AnyDecoder;

impl SceneImageDecoder for AnyDecoder {
    fn decode(&self, _bytes: &[u8], _path: &str) -> Result<DecodedSceneImage, RenderError> {
        Ok(DecodedSceneImage::Rgba(RgbaLevel {
            width: 2,
            height: 2,
            pixels: vec![128; 2 * 2 * 4],
        }))
    }
}

fn test_owner(tag: &str) -> ResourceOwner {
    let identity = IdentityOwner::create(tag).unwrap();
    ResourceOwner::new(21, identity.session().clone(), 0)
}

fn loader_over(mounts: MountedContent, tag: &str) -> SceneTextureLoader {
    let mut loader = SceneTextureLoader::new(
        SceneImageRegistry::new(test_owner(tag)),
        Box::new(MountReader { mounts }),
        None,
        None,
        224,
    )
    .unwrap();
    loader.set_decoder(Box::new(AnyDecoder));
    loader
}

/// Windowed camera over a live drawable size at an eye origin (wu-12 shape).
fn camera_for(width: i32, height: i32, origin: [f32; 3], angles: [f32; 3]) -> SceneCamera {
    let fov_x = 90.0_f32;
    let fov_y = ((f64::from(height) / f64::from(width) * (f64::from(fov_x) * std::f64::consts::PI / 360.0).tan())
        .atan()
        * 360.0
        / std::f64::consts::PI) as f32;
    SceneCamera {
        origin: vec3(origin[0], origin[1], origin[2]),
        axis: angles_to_axis(vec3(angles[0], angles[1], angles[2])),
        viewport: ViewRect {
            x: 0,
            y: 0,
            width,
            height,
        },
        projection: perspective_projection(fov_x, fov_y, 16384.0, 4.0).unwrap(),
        clip: CameraClip::None,
    }
}

fn skin_options() -> SceneTextureLoadOptions {
    SceneTextureLoadOptions {
        mipmap: true,
        repeat: true,
        family: TextureFamily::Q3,
        usage: Some(ImageUsage::Skin),
    }
}

/// The script parser accepts blend/glow/anim/videomap stage directives.
#[test]
fn parser_accepts_blend_glow_anim_stages() {
    let entries = inspect_shader_script(
        r"
models/weapons2/plasma/plasma_glass
{
    {
        map textures/effects/tinfxb.tga
        tcGen environment
        tcMod scroll .01 .02
        blendfunc GL_ONE GL_ONE
        rgbGen lightingDiffuse
    }
}
textures/sfx/anim
{
    {
        animMap 4 textures/a.tga textures/b.tga
        blendFunc add
    }
    {
        map $lightmap
        blendFunc filter
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
        "scripts/test.shader",
    )
    .expect("script parses");
    assert_eq!(entries.len(), 3);
    let named: HashMap<&str, _> = entries.iter().map(|entry| (entry.name.as_str(), entry)).collect();

    let glass = named["models/weapons2/plasma/plasma_glass"];
    let ShaderEntryResult::Accepted(glass) = &glass.result else {
        panic!("plasma glass accepts: {:?}", glass.result);
    };
    assert_eq!(glass.stages.len(), 1);
    let stage = &glass.stages[0].stage;
    assert_eq!(
        stage.map,
        ShaderMap::Image {
            name: "textures/effects/tinfxb.tga".to_string(),
            clamp: false,
        }
    );
    assert_eq!(stage.blend, ADDITIVE_BLEND);
    assert_eq!(stage.rgb_gen, ColorGen::LightingDiffuse);
    assert_eq!(stage.tc_gen, TexGen::Environment);
    assert_eq!(stage.tc_mods.len(), 1);
    assert!(matches!(stage.tc_mods[0], TexMod::Scroll(_)));

    let anim = named["textures/sfx/anim"];
    let ShaderEntryResult::Accepted(anim) = &anim.result else {
        panic!("anim shader accepts: {:?}", anim.result);
    };
    assert_eq!(anim.stages.len(), 2);
    assert_eq!(
        anim.stages[0].stage.map,
        ShaderMap::Animation {
            frequency: 4.0,
            frames: vec!["textures/a.tga".to_string(), "textures/b.tga".to_string()],
        }
    );
    assert_eq!(anim.stages[0].stage.blend, ADDITIVE_BLEND);
    assert_eq!(anim.stages[1].stage.map, ShaderMap::Lightmap);
    assert_eq!(anim.stages[1].stage.blend, FILTER_BLEND);
    assert!(matches!(anim.stages[1].stage.alpha_gen, AlphaGen::Identity));

    let video = named["textures/sfx/video"];
    let ShaderEntryResult::Accepted(video) = &video.result else {
        panic!("video shader accepts: {:?}", video.result);
    };
    assert_eq!(
        video.stages[0].stage.map,
        ShaderMap::Video {
            name: "intro.roq".to_string(),
        }
    );
}

/// Corpus `.shader` scripts are discovered top-level and sorted, and parse
/// without fatal errors.
#[test]
fn corpus_scripts_discover_and_parse() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let mounts = open_product_mounts(&corpus, "q3-baseq3");
    let paths = discover_shader_script_paths(&mounts).expect("scripts discover");
    assert!(!paths.is_empty(), "installed paks hold shader scripts");
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "discovery order is sorted");
    for path in &paths {
        assert!(path.starts_with("scripts/"), "top-level script: {path}");
        assert!(!path["scripts/".len()..].contains('/'), "no subdirectories: {path}");
        assert!(path.to_lowercase().ends_with(".shader"), "shader suffix: {path}");
    }
    assert!(
        paths.iter().any(|path| path == "scripts/models.shader"),
        "models.shader ships the plasma glow: {paths:?}"
    );

    let scripts = read_shader_scripts(&mounts).expect("scripts read");
    assert_eq!(scripts.len(), paths.len());
    let mut accepted = 0_usize;
    let mut rejected = Vec::new();
    for script in &scripts {
        for entry in inspect_shader_script(&script.text, &script.path).expect("script parses") {
            match entry.result {
                ShaderEntryResult::Accepted(_) => accepted += 1,
                ShaderEntryResult::Rejected { message, .. } => {
                    rejected.push(format!("{}: {}: {message}", script.path, entry.name));
                }
            }
        }
    }
    eprintln!("parsed {accepted} shader definitions ({} rejected)", rejected.len());
    for rejection in rejected.iter().take(8) {
        eprintln!("rejected: {rejection}");
    }
    assert!(accepted > 1000, "retail scripts hold thousands of shaders: {accepted}");
    assert!(rejected.len() < accepted / 10, "rejections stay rare: {rejected:?}");

    let index = ShaderImageIndex::build(&scripts).expect("index builds");
    assert!(index.contains("models/weapons2/plasma/plasma_glass"));
    assert_eq!(
        index.resolve("models/weapons2/plasma/plasma_glass"),
        Some(&SkinResolution::Image("textures/effects/tinfxb.tga".to_string()))
    );
    // q3dm1 world glass/energy shaders are authored too.
    for name in [
        "textures/gothic_floor/center2trn",
        "textures/gothic_trim/pitted_rust2_trans",
        "textures/gothic_block/demon_block15fx",
        "textures/gothic_floor/largerblock3b_ow",
    ] {
        assert!(index.contains(name), "{name} is an authored shader");
    }
}

/// Loading scripts into a registry applies authored multi-stage materials
/// (blend/glow/anim) instead of implicit single-stage fallbacks.
#[test]
fn registry_applies_authored_blend_stages() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let mounts = open_product_mounts(&corpus, "q3-baseq3");
    let scripts = read_shader_scripts(&mounts).expect("scripts read");
    let mut registry = SceneShaderRegistry::with_defaults(loader_over(mounts, "wu21-registry"));
    load_registry_scripts(&mut registry, &scripts, true).expect("scripts load");
    registry.source_materials().expect("source materials initialize");
    assert!(registry.has_authored("textures/gothic_floor/center2trn"));

    let material = registry
        .register(
            "textures/gothic_floor/center2trn",
            SceneShaderBinding::World {
                world: alloc_world_identity(),
                lightmap_index: 0,
                lightmap: None,
                base_texture: None,
            },
        )
        .expect("authored world shader registers");
    assert_eq!(material.material.stages.len(), 4, "energy swirl keeps its stages");
    assert!(
        material.material.stages.iter().any(|stage| stage.blend != OPAQUE_BLEND),
        "at least one stage blends: {:?}",
        material
            .material
            .stages
            .iter()
            .map(|stage| stage.blend)
            .collect::<Vec<_>>()
    );
    assert!(
        material.finished.num_unfogged_passes >= 2,
        "multi-stage shader finishes multiple passes: {}",
        material.finished.num_unfogged_passes
    );

    // A name with neither script nor image still defaults honestly.
    let defaulted = registry
        .register(
            "textures/wu21_absent_shader_xyz",
            SceneShaderBinding::World {
                world: alloc_world_identity(),
                lightmap_index: 0,
                lightmap: None,
                base_texture: None,
            },
        )
        .expect("absent shader registers");
    assert!(
        registry
            .warnings
            .iter()
            .any(|warning| warning.contains("textures/wu21_absent_shader_xyz")),
        "absent shader warns: {:?}",
        registry.warnings
    );
    assert_ne!(material.registration, defaulted.registration);
}

/// The q3dm1 spawn view draws authored blend batches (glass/energy), not
/// only opaque implicit batches.
#[test]
fn q3dm1_spawn_view_applies_authored_blend_stages() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let options = ApplicationOptions {
        corpus_root: corpus.to_string_lossy().into_owned(),
        product: "q3-baseq3".to_string(),
        map: "maps/q3dm1.bsp".to_string(),
        ..ApplicationOptions::default()
    };
    let config = StartupConfig::from_options(&options).unwrap();
    let catalog = discover_installed_content(&DiscoverContentOptions::new(corpus.clone())).unwrap();
    let owner = test_owner("wu21-spawn-view");
    let mut world =
        load_windowed_world(&config, &catalog, &options, owner).unwrap_or_else(|error| panic!("q3dm1 loads: {error}"));
    assert!(
        world.presentation_error().is_none(),
        "q3dm1 presents: {:?}",
        world.presentation_error()
    );
    let mut presentation = world.take_presentation().unwrap();
    let spawn = presentation
        .spawn()
        .unwrap_or_else(|| panic!("q3dm1 has a player spawn"));
    let camera = camera_for(
        320,
        240,
        [spawn.origin.x, spawn.origin.y, spawn.origin.z],
        [spawn.angles.x, spawn.angles.y, spawn.angles.z],
    );
    let (view, image_operations) = presentation
        .prepare_frame_view(
            camera,
            ViewTarget::Preview("wu21".to_string()),
            SourceTime::Milliseconds(100.0),
        )
        .unwrap_or_else(|error| panic!("q3dm1 prepares a view: {error}"));
    assert!(!image_operations.is_empty(), "texture/lightmap uploads must exist");
    let mut batches = 0_usize;
    let mut blended = 0_usize;
    let mut vertices = 0_usize;
    for operation in &view.operations {
        let RenderOperation::Draw(draws) = operation else {
            continue;
        };
        for batch in draws {
            batches += 1;
            if batch.state.blend != (BlendFactor::One, BlendFactor::Zero) || !batch.state.depth_write {
                blended += 1;
            }
            vertices += match &batch.vertices {
                BatchVertices::Single(list) => list.len(),
                BatchVertices::Pair { vertices, .. } => vertices.len(),
            };
        }
    }
    eprintln!("q3dm1 spawn view: {batches} batches ({blended} blended), {vertices} vertices");
    assert!(batches > 100, "spawn view draws the map: {batches}");
    assert!(blended > 0, "authored glass/energy stages blend instead of defaulting");
}

/// `plasma_glass` (authored, image-less) resolves to its glow stage image
/// through the loader instead of falling back to the missing handle.
#[test]
fn md3_plasma_glass_binds_stage_image_not_missing() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let mounts = open_product_mounts(&corpus, "q3-baseq3");
    let scripts = read_shader_scripts(&mounts).expect("scripts read");
    let index = ShaderImageIndex::build(&scripts).expect("index builds");
    let SkinResolution::Image(path) = index
        .resolve("models/weapons2/plasma/plasma_glass")
        .expect("plasma glass resolves")
    else {
        panic!("plasma glass resolves to a stage image");
    };

    let mut loader = loader_over(mounts, "wu21-plasma");
    // The old path loaded the shader name as an image: no such file exists.
    assert!(
        loader
            .load("models/weapons2/plasma/plasma_glass", &skin_options())
            .unwrap()
            .is_none(),
        "no image ships under the shader name itself"
    );
    // The stage image (`.tga` reference probing to the shipped `.jpg`)
    // loads through the same loader.
    let texture = loader
        .load(path, &skin_options())
        .unwrap()
        .unwrap_or_else(|| panic!("stage image {path} loads"));
    assert_ne!(texture.image, loader.missing().image);
    assert_eq!(path, "textures/effects/tinfxb.tga");
}

/// Windowed q3dm1 frame capture for the before/after pixel diff.
///
/// Writes raw 320x240 RGBA to `WU21_CAPTURE_OUT`; skipped unless set.
/// Run under `xvfb-run` like every windowed test.
#[test]
fn capture_q3dm1_spawn_for_before_after_diff() {
    let Ok(out) = std::env::var("WU21_CAPTURE_OUT") else {
        eprintln!("skipped: WU21_CAPTURE_OUT is unset");
        return;
    };
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let options = ApplicationOptions {
        windowed: true,
        width: 320,
        height: 240,
        frame_limit: Some(3),
        corpus_root: corpus.to_string_lossy().into_owned(),
        product: "q3-baseq3".to_string(),
        map: "maps/q3dm1.bsp".to_string(),
        ..ApplicationOptions::default()
    };
    let mut composed = match open_windowed_application(&options, StartupEntry::Run) {
        Ok(composed) => composed,
        Err(error) => panic!("q3dm1 windowed app opens: {error}"),
    };
    assert!(composed.app.active_game(), "q3dm1 has a scene");
    let pixels = composed.app.capture_next_frame().expect("q3dm1 captures");
    assert_eq!(pixels.len(), 320 * 240 * 4);
    std::fs::write(&out, &pixels).unwrap_or_else(|error| panic!("write {out}: {error}"));
    let lit = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count();
    eprintln!("captured {} non-black pixels to {out}", lit);
    let frames = drive_windowed_application(&mut composed.app, &composed.quit, Some(3)).expect("q3dm1 drives");
    assert_eq!(frames, 3);
    assert!(composed.app.is_closed());
}

/// Inline scripts (no corpus) still drive the registry path end to end.
#[test]
fn inline_scripts_drive_registry_without_corpus() {
    let scripts = vec![ShaderScript {
        path: "scripts/inline.shader".to_string(),
        text: "textures/inline/glow\n{\n\t{\n\t\tmap textures/inline/glow.tga\n\t\tblendFunc GL_ONE GL_ONE\n\t}\n\t{\n\t\tmap $lightmap\n\t\tblendFunc filter\n\t}\n}\n".to_string(),
    }];
    let loader = SceneTextureLoader::new(
        SceneImageRegistry::new(test_owner("wu21-inline")),
        Box::new(EmptyReader),
        None,
        None,
        224,
    )
    .unwrap();
    let mut registry = SceneShaderRegistry::with_defaults(loader);
    load_registry_scripts(&mut registry, &scripts, true).expect("inline scripts load");
    assert!(registry.has_authored("textures/inline/glow"));
    let material = registry
        .register(
            "textures/inline/glow",
            SceneShaderBinding::Unlit {
                lightmap_index: -1,
                mipmap: true,
            },
        )
        .expect("inline shader registers");
    assert_eq!(material.material.stages.len(), 2);
    assert_eq!(material.material.stages[0].blend, ADDITIVE_BLEND);
}

/// Reader with no files (inline-script tests need no mounts).
struct EmptyReader;

impl SceneAssetReader for EmptyReader {
    fn read(&self, _path: &str) -> Result<Option<SceneAsset>, RenderError> {
        Ok(None)
    }
}
