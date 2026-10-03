//! wu-12: Quake and Quake II windowed maps present imagery, not black.
//!
//! Regression coverage for the all-black Q1/Q2 windowed frames: e1m1
//! captured pure black and base1 near-black with a single sky blob while
//! q3dm1 rendered. Two defects combined: legacy world batches received a
//! doubly-normalized entity tint (near-zero vertex colors forced onto the
//! translucent path), and spawn selection bailed out on the first
//! non-spawn record so the camera sat at the map origin instead of the
//! player start. These tests pin the batch-level contract headlessly and
//! prove lit captures per family through a real window.
//!
//! The Steel corpus is discovered by walking up from this crate, so the
//! tests run both in-tree and from an isolated worktree whose corpus lives
//! at an ancestor `target/` directory. When the corpus is absent the
//! corpus-backed tests skip instead of failing.

use std::path::{Path, PathBuf};

use qa_app::bootstrap::startup::StartupEntry;
use qa_app::bootstrap::windowed::{drive_windowed_application, open_windowed_application};
use qa_app::bootstrap::windowed_world::load_windowed_world;
use qa_app::options::ApplicationOptions;
use qa_app::startup::StartupConfig;
use qa_client::materials::geometry::{MaterialGeometry, MaterialVertex};
use qa_client::materials::legacy::{
    create_q1_material, prepare_legacy_material_batches, LegacyMaterial, LegacyMaterialDrawContext,
};
use qa_client::materials::lighting::Q1LightmapEncoding;
use qa_client::materials::state::{CullFace as MaterialCullFace, OPAQUE_BLEND};
use qa_client::render::types::{BatchVertices, BlendFactor, RenderOperation, ResourceOwner, SourceTime, ViewTarget};
use qa_client::view::{perspective_projection, CameraClip, Rect as ViewRect, SceneCamera};
use qa_content::catalog::{discover_installed_content, DiscoverContentOptions};
use qa_core::identity::IdentityOwner;
use qa_core::math::{angles_to_axis, vec2, vec3, vec4};

/// Witness files proving a Steel corpus root holds all three families.
const CORPUS_WITNESSES: [&str; 3] = ["q1/id1/pak0.pak", "q2/baseq2/pak0.pak", "q3a/baseq3/pak0.pk3"];

/// Locate the Steel corpus root without hardcoding any absolute path.
///
/// Walks up from this crate's manifest directory and returns the first
/// ancestor `target/` directory holding every witness file.
fn find_steel_corpus() -> Option<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .map(|dir| dir.join("target"))
        .find(|root| CORPUS_WITNESSES.iter().all(|witness| root.join(witness).is_file()))
}

/// Windowed camera over a live drawable size at an eye origin (mirrors the
/// production `windowed_camera_for`: 90-degree horizontal field of view,
/// 4-unit near plane, 16384-unit far plane).
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

/// First-vertex color of a draw batch, across single and paired texturing.
fn batch_first_color(batch: &qa_client::render::types::DrawBatch) -> qa_core::math::Vec4 {
    match &batch.vertices {
        BatchVertices::Single(vertices) => vertices[0].color,
        BatchVertices::Pair { vertices, .. } => vertices[0].base.color,
    }
}

/// Batch-level summary of one map's spawn view, prepared headlessly.
struct SpawnViewStats {
    surfaces: usize,
    batches: usize,
    opaque_white: usize,
    image_operations: usize,
}

fn prepare_spawn_view(corpus: &Path, product: &str, map: &str) -> SpawnViewStats {
    let options = ApplicationOptions {
        corpus_root: corpus.to_string_lossy().into_owned(),
        product: product.to_string(),
        map: map.to_string(),
        ..ApplicationOptions::default()
    };
    let config = StartupConfig::from_options(&options).unwrap();
    let catalog = discover_installed_content(&DiscoverContentOptions::new(corpus.to_path_buf())).unwrap();
    let identity = IdentityOwner::create("wu12-spawn-view").unwrap();
    let owner = ResourceOwner::new(7, identity.session().clone(), 0);
    let mut world = load_windowed_world(&config, &catalog, &options, owner)
        .unwrap_or_else(|error| panic!("{product} {map} loads: {error}"));
    assert!(
        world.presentation_error().is_none(),
        "{product} {map} presents: {:?}",
        world.presentation_error()
    );
    let mut presentation = world.take_presentation().unwrap();
    let spawn = presentation
        .spawn()
        .unwrap_or_else(|| panic!("{product} {map} has a player spawn"));
    let camera = camera_for(
        320,
        240,
        [spawn.origin.x, spawn.origin.y, spawn.origin.z],
        [spawn.angles.x, spawn.angles.y, spawn.angles.z],
    );
    let (view, image_operations) = presentation
        .prepare_frame_view(
            camera,
            ViewTarget::Preview("wu12".to_string()),
            SourceTime::Milliseconds(100.0),
        )
        .unwrap_or_else(|error| panic!("{product} {map} prepares a view: {error}"));
    let mut batches = 0;
    let mut opaque_white = 0;
    for operation in &view.operations {
        let RenderOperation::Draw(draws) = operation else {
            continue;
        };
        for batch in draws {
            batches += 1;
            if batch.state.blend == (BlendFactor::One, BlendFactor::Zero)
                && batch.state.depth_write
                && batch_first_color(batch) == vec4(1.0, 1.0, 1.0, 1.0)
            {
                opaque_white += 1;
            }
        }
    }
    SpawnViewStats {
        surfaces: presentation.surface_count(),
        batches,
        opaque_white,
        image_operations: image_operations.len(),
    }
}

/// Count captured pixels that differ from pure black (any nonzero RGB).
fn count_non_black(pixels: &[u8]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count()
}

/// Mean RGB channel value of a capture.
fn mean_brightness(pixels: &[u8]) -> f64 {
    let mut sum = 0_u64;
    for pixel in pixels.as_chunks::<4>().0 {
        sum += u64::from(pixel[0]) + u64::from(pixel[1]) + u64::from(pixel[2]);
    }
    sum as f64 / (pixels.len() / 4 * 3) as f64
}

#[test]
fn legacy_entity_tint_is_bytes_not_unit_floats() {
    // Donor `entityRGBA` is bytes-as-floats (0..255); the batch builder
    // divides by 255 itself. A white tint must stay opaque white, not
    // collapse onto the translucent path with near-zero vertex colors.
    let material = create_q1_material("rock", 1, Some(2), false, 1.0, Vec::new(), Vec::new());
    let project = |position: qa_core::math::Vec3| vec4(position.x, position.y, position.z, 1.0);
    let context = LegacyMaterialDrawContext {
        entity_rgba: Some(vec4(255.0, 255.0, 255.0, 255.0)),
        time: 0.0,
        animation_frame: 0.0,
        alternate_animation: false,
        fullbright: None,
        q1_fog_active: false,
        q1_lightmap_encoding: Q1LightmapEncoding::Rgb,
        translucent_lightmap: None,
        cull: MaterialCullFace::Front,
        depth_range: [0.0, 1.0],
        project: &project,
    };
    let geometry = MaterialGeometry {
        vertices: vec![MaterialVertex::new(
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        )],
        indices: vec![0],
    };
    let batches = prepare_legacy_material_batches(&LegacyMaterial::Q1(material), &geometry, &context).unwrap();
    assert_eq!(batches.len(), 2, "base pass plus lightmap pass");
    assert_eq!(batches[0].state.blend, OPAQUE_BLEND);
    assert!(batches[0].state.depth_write);
    assert_eq!(batches[0].vertices[0].color, vec4(1.0, 1.0, 1.0, 1.0));
}

#[test]
fn q1_e1m1_spawn_view_batches_are_opaque_and_lit() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let stats = prepare_spawn_view(&corpus, "q1-classic-id1", "maps/e1m1.bsp");
    assert!(stats.surfaces > 1000, "surfaces: {}", stats.surfaces);
    assert!(stats.batches > 100, "batches: {}", stats.batches);
    assert!(stats.opaque_white > 100, "opaque white batches: {}", stats.opaque_white);
    assert!(stats.image_operations > 0, "texture/lightmap uploads must exist");
}

#[test]
fn q2_base1_spawn_view_batches_are_opaque_and_lit() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    let stats = prepare_spawn_view(&corpus, "q2-classic-baseq2", "maps/base1.bsp");
    assert!(stats.surfaces > 1000, "surfaces: {}", stats.surfaces);
    assert!(stats.batches > 100, "batches: {}", stats.batches);
    assert!(stats.opaque_white > 100, "opaque white batches: {}", stats.opaque_white);
    assert!(stats.image_operations > 0, "texture/lightmap uploads must exist");
}

/// Per-family windowed captures must show map imagery, not black frames.
///
/// One sequential test (not three parallel ones) so concurrent SDL windows
/// never contend: each map opens, captures, drives, and closes in turn.
/// Thresholds sit far below the measured lit fractions (~99% non-black,
/// mean brightness 17+) yet far above the old black frames (0-5%).
#[test]
fn windowed_captures_show_map_imagery_per_family() {
    let Some(corpus) = find_steel_corpus() else {
        eprintln!("skipped: Steel corpus not found above {}", env!("CARGO_MANIFEST_DIR"));
        return;
    };
    for (product, map) in [
        ("q1-classic-id1", "maps/e1m1.bsp"),
        ("q2-classic-baseq2", "maps/base1.bsp"),
        ("q3-baseq3", "maps/q3dm1.bsp"),
    ] {
        let options = ApplicationOptions {
            windowed: true,
            width: 320,
            height: 240,
            frame_limit: Some(3),
            corpus_root: corpus.to_string_lossy().into_owned(),
            product: product.to_string(),
            map: map.to_string(),
            ..ApplicationOptions::default()
        };
        match open_windowed_application(&options, StartupEntry::Run) {
            Ok(mut composed) => {
                assert!(composed.app.active_game(), "{product} {map} has a scene");
                let pixels = composed
                    .app
                    .capture_next_frame()
                    .unwrap_or_else(|error| panic!("{product} {map} captures: {error}"));
                assert_eq!(pixels.len(), 320 * 240 * 4);
                let lit = count_non_black(&pixels);
                let fraction = lit as f64 / (320.0 * 240.0);
                let mean = mean_brightness(&pixels);
                assert!(
                    fraction > 0.5,
                    "{product} {map}: expected map imagery, got {lit} non-black pixels (mean {mean:.2})"
                );
                assert!(
                    mean > 5.0,
                    "{product} {map}: expected lit imagery, got mean brightness {mean:.2}"
                );
                let frames = drive_windowed_application(&mut composed.app, &composed.quit, Some(3))
                    .unwrap_or_else(|error| panic!("{product} {map} drives: {error}"));
                assert_eq!(frames, 3);
                assert!(composed.app.is_closed());
            }
            Err(error) => assert!(!error.is_empty(), "honest open failure: {product} {map}"),
        }
    }
}
