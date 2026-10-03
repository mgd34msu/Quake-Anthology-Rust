//! World operations cache: prepared surface batches retained across frames.
//!
//! Donor provenance: none; this is a renderer-side memo for the unified
//! world scene (`super::world::WorldScene`). Profiling a release
//! `--windowed` Quake II `base1` run (7905 surfaces) showed the full world
//! surface list cloned, sorted, and rebuilt into draw operations every
//! frame, with `WorldSurface` cloning alone over half of frame CPU. The
//! windowed camera sits at the map spawn point, so visibility, culling,
//! and batch contents are identical from frame to frame; only time-driven
//! inputs (texture animation, liquid turbulence, shader clocks) change.
//!
//! The cache stores prepared [`SceneOperation`] lists per surface, keyed by
//! a snapshot of every view input that can change them
//! ([`WorldOpsKey`]). Surfaces whose output depends on continuous view
//! time (Quake III shaders, skies, liquids, animated frames) are classified
//! by [`surface_is_static`] and always prepared fresh, so cached frames
//! stay pixel-identical to uncached ones. The key carries no clock: static
//! surfaces are, by classification, time-independent, and animated or
//! scrolling surfaces never consult the store. Any key mismatch, world
//! edit (remap publication), or scene close invalidates the retained
//! operations and the next prepare rebuilds them.

use std::collections::HashMap;

use crate::materials::deform::ProjectionShadowContext;
use crate::materials::legacy::{LegacyMaterial, Q1Surface};
use crate::materials::lighting::{Q2LightStyle, SurfaceDynamicLight};
use crate::materials::q3_lighting::EntityLighting;
use crate::render::types::{Q2Fog, RendererImage};
use crate::view::{ModelTransform, SceneCamera};

use super::q2_sky::Q2SkyView;
use super::submissions::SceneOperation;
use super::visibility::{VisibleWorld, WorldVisibilityOptions};
use super::world::{
    FlareHook, Q1FogParams, Q2FragmentLighting, SceneMaterialRemap, WorldSurface, WorldSurfaceData, WorldViewInput,
};

/// Whether two projection-shadow contexts select identical geometry.
fn shadow_context_eq(left: &ProjectionShadowContext, right: &ProjectionShadowContext) -> bool {
    left.axis == right.axis
        && left.origin == right.origin
        && left.shadow_plane == right.shadow_plane
        && left.light_dir == right.light_dir
}

/// Whether two flare hooks are the same registration (pointer identity).
fn flare_eq(left: &Option<FlareHook>, right: &Option<FlareHook>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => std::sync::Arc::ptr_eq(left, right),
        (None, Some(_)) | (Some(_), None) => false,
    }
}

/// Snapshot of every view input that can change cached static surface
/// operations, plus the scene generation they were built under.
///
/// View time is deliberately absent: only surfaces proven
/// time-independent by [`surface_is_static`] are retained, and every
/// time-driven surface is prepared fresh on every frame. View assembly
/// inputs (`target`, `clear`, `before_view`, extra `operations`,
/// `inline_models`, `no_world_model`) are also absent: they shape the
/// finished view around the world operations, never the operations
/// themselves. Source admissions bypass the cache entirely (the admission
/// keeps its own one-view retention), so `source` is absent too.
#[derive(Clone)]
pub struct WorldOpsKey {
    camera: SceneCamera,
    visibility: WorldVisibilityOptions,
    q1_styles: Vec<i32>,
    q2_styles: Vec<Q2LightStyle>,
    lights: Vec<SurfaceDynamicLight>,
    q2_fragment_lighting: Option<Q2FragmentLighting>,
    q1_fog: Option<Q1FogParams>,
    q2_fog: Option<Q2Fog>,
    q2_sky: Option<Q2SkyView>,
    source_sky: Option<Q2SkyView>,
    animation_frame: Option<f32>,
    alternate_animation: bool,
    curve_error: f32,
    identity_light: f32,
    render_text: Vec<String>,
    lighting: Option<EntityLighting>,
    entity_rgba: [u8; 4],
    projection_shadow: Option<ProjectionShadowContext>,
    flare: Option<FlareHook>,
    dynamic_images: HashMap<u32, RendererImage>,
    material_revision: u64,
    raw_remaps: HashMap<usize, SceneMaterialRemap>,
}

impl WorldOpsKey {
    /// Capture the key for one view input under a scene generation.
    #[must_use]
    pub fn capture(
        input: &WorldViewInput,
        material_revision: u64,
        raw_remaps: &HashMap<usize, SceneMaterialRemap>,
    ) -> Self {
        Self {
            camera: input.camera,
            visibility: input.visibility.clone(),
            q1_styles: input.q1_styles.clone(),
            q2_styles: input.q2_styles.clone(),
            lights: input.lights.clone(),
            q2_fragment_lighting: input.q2_fragment_lighting.clone(),
            q1_fog: input.q1_fog,
            q2_fog: input.q2_fog,
            q2_sky: input.q2_sky.clone(),
            source_sky: input.source_sky.clone(),
            animation_frame: input.animation_frame,
            alternate_animation: input.alternate_animation,
            curve_error: input.curve_error,
            identity_light: input.identity_light,
            render_text: input.render_text.clone(),
            lighting: input.lighting,
            entity_rgba: input.entity_rgba,
            projection_shadow: input.projection_shadow,
            flare: input.prepare_flare.clone(),
            dynamic_images: input.dynamic_images.clone(),
            material_revision,
            raw_remaps: raw_remaps.clone(),
        }
    }

    /// Whether a live view input still matches this snapshot.
    #[must_use]
    pub fn matches(
        &self,
        input: &WorldViewInput,
        material_revision: u64,
        raw_remaps: &HashMap<usize, SceneMaterialRemap>,
    ) -> bool {
        self.camera == input.camera
            && self.visibility == input.visibility
            && self.q1_styles == input.q1_styles
            && self.q2_styles == input.q2_styles
            && self.lights == input.lights
            && self.q2_fragment_lighting == input.q2_fragment_lighting
            && self.q1_fog == input.q1_fog
            && self.q2_fog == input.q2_fog
            && self.q2_sky == input.q2_sky
            && self.source_sky == input.source_sky
            && self.animation_frame == input.animation_frame
            && self.alternate_animation == input.alternate_animation
            && self.curve_error == input.curve_error
            && self.identity_light == input.identity_light
            && self.render_text == input.render_text
            && self.lighting == input.lighting
            && self.entity_rgba == input.entity_rgba
            && self
                .projection_shadow
                .as_ref()
                .map_or(input.projection_shadow.is_none(), |left| {
                    input
                        .projection_shadow
                        .as_ref()
                        .is_some_and(|right| shadow_context_eq(left, right))
                })
            && flare_eq(&self.flare, &input.prepare_flare)
            && self.dynamic_images == input.dynamic_images
            && self.material_revision == material_revision
            && self.raw_remaps == *raw_remaps
    }
}

/// Whether a surface's prepared operations are independent of view time
/// (and therefore safe to retain across frames).
///
/// `raw_remapped` must report whether `surface.index` currently holds a
/// raw material remap: remapped legacy surfaces take the shader path,
/// whose clocks the cache cannot freeze. Dynamic classes mirror the
/// time reads in preparation: Quake III shaders (material clocks,
/// deformations, patch selection aside), Quake I skies (scrolling layer
/// coordinates) and liquids (turbulence), Quake II skies (rotation),
/// warp/flowing liquids (turbulence/scroll), and animated texture frames
/// on either legacy family. Everything else on the legacy path (texture
/// selection with a single frame, lightmap ordinals, cull state, fog
/// activity) is fixed by the cache key, so the retained batches rebuild
/// bit-identically.
#[must_use]
pub fn surface_is_static(surface: &WorldSurface, raw_remapped: bool) -> bool {
    if raw_remapped {
        return false;
    }
    match &surface.data {
        WorldSurfaceData::Q3 { .. } => false,
        WorldSurfaceData::Legacy { shader: Some(_), .. } => false,
        WorldSurfaceData::Legacy { material, q1_sky, .. } => {
            if q1_sky.is_some() {
                return false;
            }
            match material {
                LegacyMaterial::Q1(material) => {
                    !matches!(
                        material.surface,
                        Q1Surface::Water | Q1Surface::Slime | Q1Surface::Lava | Q1Surface::Teleport
                    ) && material.animation.is_empty()
                        && material.alternate_animation.is_empty()
                }
                LegacyMaterial::Q2 { material, .. } => {
                    material.surface_flags & 4 == 0 && !material.warp && !material.flowing && material.frames.count <= 1
                }
            }
        }
    }
}

/// Cache hit/miss counters for tests and profiling proof.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldCacheStats {
    /// Prepares served from retained operations.
    pub hits: u64,
    /// Prepares that rebuilt (and retained) operations.
    pub misses: u64,
}

/// Retained world-model operations: the sorted visible order plus each
/// static surface's prepared batches.
#[derive(Clone)]
struct WorldEntry {
    key: WorldOpsKey,
    order: Vec<usize>,
    surfaces: Vec<Option<Vec<SceneOperation>>>,
    visible: VisibleWorld,
    sky_drawn: Option<bool>,
}

/// Retained inline-model operations for one model and transform.
#[derive(Clone)]
struct ModelEntry {
    model: usize,
    first: usize,
    count: usize,
    transform: ModelTransform,
    key: WorldOpsKey,
    surfaces: Vec<Option<Vec<SceneOperation>>>,
}

/// Prepared world operations retained across frames.
///
/// The world model keeps one entry; each inline model keeps its own entry
/// keyed additionally by model index and transform. Entries are replaced
/// wholesale on any key mismatch, so a moving camera, new light styles,
/// or a remap publication transparently rebuilds. Hosts that mutate the
/// shader registry between frames through `WorldScene::shaders_mut` must
/// call [`WorldOperationsCache::invalidate`] (or the scene's
/// `invalidate_world_cache`), because registry growth is invisible to the
/// key.
#[derive(Default)]
pub struct WorldOperationsCache {
    world: Option<WorldEntry>,
    models: Vec<ModelEntry>,
    sky_drawn: Option<(u64, HashMap<usize, SceneMaterialRemap>, bool)>,
    stats: WorldCacheStats,
}

impl WorldOperationsCache {
    /// Drop every retained entry (world edits, registry edits, close).
    pub fn invalidate(&mut self) {
        self.world = None;
        self.models.clear();
        self.sky_drawn = None;
    }

    /// Hit/miss counters.
    #[must_use]
    pub const fn stats(&self) -> WorldCacheStats {
        self.stats
    }

    /// Whether the retained world entry matches a live view input.
    pub fn match_world(
        &mut self,
        input: &WorldViewInput,
        material_revision: u64,
        raw_remaps: &HashMap<usize, SceneMaterialRemap>,
    ) -> bool {
        let matched = self
            .world
            .as_ref()
            .is_some_and(|entry| entry.key.matches(input, material_revision, raw_remaps));
        if matched {
            self.stats.hits += 1;
        }
        matched
    }

    /// Sorted visible order of the retained world entry, if any.
    #[must_use]
    pub fn world_order(&self) -> Option<&[usize]> {
        self.world.as_ref().map(|entry| entry.order.as_slice())
    }

    /// Retained batches for one world surface, if any.
    #[must_use]
    pub fn world_surface(&self, index: usize) -> Option<&[SceneOperation]> {
        self.world
            .as_ref()
            .and_then(|entry| entry.surfaces.get(index).and_then(Option::as_ref).map(Vec::as_slice))
    }

    /// Retained first-encounter light mask for one world surface, if any.
    #[must_use]
    pub fn world_mask(&self, index: usize) -> Option<u32> {
        self.world
            .as_ref()
            .and_then(|entry| entry.visible.surface_dlight_masks.get(&index).copied())
    }

    /// Retain one world surface's batches (newly static surfaces on hits).
    pub fn store_world_surface(&mut self, index: usize, operations: Vec<SceneOperation>) {
        if let Some(entry) = self.world.as_mut() {
            if entry.surfaces.len() <= index {
                entry.surfaces.resize(index + 1, None);
            }
            entry.surfaces[index] = Some(operations);
        }
    }

    /// Retain a freshly prepared world entry, replacing any previous one.
    pub fn store_world(
        &mut self,
        key: WorldOpsKey,
        order: Vec<usize>,
        surfaces: Vec<Option<Vec<SceneOperation>>>,
        visible: VisibleWorld,
    ) {
        self.stats.misses += 1;
        self.world = Some(WorldEntry {
            key,
            order,
            surfaces,
            visible,
            sky_drawn: None,
        });
    }

    /// Retained sky-drawn flag for the world entry, if computed.
    #[must_use]
    pub fn world_sky_drawn(&self) -> Option<bool> {
        self.world.as_ref().and_then(|entry| entry.sky_drawn)
    }

    /// Remember the sky-drawn flag for the retained world entry.
    pub fn set_world_sky_drawn(&mut self, sky_drawn: bool) {
        if let Some(entry) = self.world.as_mut() {
            entry.sky_drawn = Some(sky_drawn);
        }
    }

    /// Whether the retained entry for one inline model matches.
    #[allow(clippy::too_many_arguments)]
    pub fn match_model(
        &mut self,
        model: usize,
        first: usize,
        count: usize,
        transform: &ModelTransform,
        input: &WorldViewInput,
        material_revision: u64,
        raw_remaps: &HashMap<usize, SceneMaterialRemap>,
    ) -> bool {
        let matched = self.models.iter().any(|entry| {
            entry.model == model
                && entry.first == first
                && entry.count == count
                && entry.transform == *transform
                && entry.key.matches(input, material_revision, raw_remaps)
        });
        if matched {
            self.stats.hits += 1;
        }
        matched
    }

    /// Retained batches for one inline-model surface, if any.
    #[must_use]
    pub fn model_surface(&self, model: usize, transform: &ModelTransform, index: usize) -> Option<&[SceneOperation]> {
        self.models
            .iter()
            .find(|entry| entry.model == model && entry.transform == *transform)
            .and_then(|entry| {
                entry
                    .surfaces
                    .get(index.wrapping_sub(entry.first))
                    .and_then(Option::as_ref)
                    .map(Vec::as_slice)
            })
    }

    /// Retain one inline-model surface's batches (newly static on hits).
    pub fn store_model_surface(
        &mut self,
        model: usize,
        transform: &ModelTransform,
        first: usize,
        index: usize,
        operations: Vec<SceneOperation>,
    ) {
        if let Some(entry) = self
            .models
            .iter_mut()
            .find(|entry| entry.model == model && entry.transform == *transform)
        {
            let slot = index.wrapping_sub(first);
            if entry.surfaces.len() <= slot {
                entry.surfaces.resize(slot + 1, None);
            }
            entry.surfaces[slot] = Some(operations);
        }
    }

    /// Retain a freshly prepared inline-model entry, replacing any
    /// previous entry for the same model and transform.
    pub fn store_model(
        &mut self,
        model: usize,
        first: usize,
        count: usize,
        transform: ModelTransform,
        key: WorldOpsKey,
        surfaces: Vec<Option<Vec<SceneOperation>>>,
    ) {
        self.stats.misses += 1;
        self.models
            .retain(|entry| !(entry.model == model && entry.transform == transform));
        self.models.push(ModelEntry {
            model,
            first,
            count,
            transform,
            key,
            surfaces,
        });
    }

    /// Retained sky-drawn scan while the remap generation is unchanged.
    ///
    /// The scan depends only on surfaces (fixed at build) and remap state,
    /// never on the view input, so it keeps its own generation guard apart
    /// from the view-keyed entries.
    #[must_use]
    pub fn sky_drawn(&self, material_revision: u64, raw_remaps: &HashMap<usize, SceneMaterialRemap>) -> Option<bool> {
        self.sky_drawn.as_ref().and_then(|(revision, remaps, sky_drawn)| {
            (*revision == material_revision && remaps == raw_remaps).then_some(*sky_drawn)
        })
    }

    /// Remember a fresh sky-drawn scan under a remap generation.
    pub fn store_sky_drawn(
        &mut self,
        material_revision: u64,
        raw_remaps: &HashMap<usize, SceneMaterialRemap>,
        sky_drawn: bool,
    ) {
        self.sky_drawn = Some((material_revision, raw_remaps.clone(), sky_drawn));
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec3, Bounds};

    use crate::materials::geometry::MaterialGeometry;
    use crate::materials::legacy::{Q1Material, Q2Frames, Q2Material};
    use crate::render::types::{SourceTime, ViewTarget};
    use crate::view::{CameraClip, Rect, SceneCamera};

    use super::*;

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 64.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            clip: CameraClip::None,
        }
    }

    fn input() -> WorldViewInput {
        WorldViewInput::new(
            camera(),
            ViewTarget::Preview("cache".to_string()),
            SourceTime::Seconds(1.0),
        )
    }

    fn legacy_surface(material: LegacyMaterial) -> WorldSurface {
        WorldSurface {
            shader_name: "rock".to_string(),
            base_texture: None,
            index: 0,
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(16.0, 16.0, 16.0),
            },
            plane: None,
            geometry: MaterialGeometry::empty(),
            data: WorldSurfaceData::Legacy {
                shader: None,
                material,
                fullbright: None,
                lightmap: None,
                q1_sky: None,
            },
        }
    }

    fn q1_material(surface: Q1Surface) -> LegacyMaterial {
        LegacyMaterial::Q1(Q1Material {
            name: "rock".to_string(),
            texture: 1,
            lightmap: None,
            vertex_lit: false,
            surface,
            alpha: 1.0,
            animation: Vec::new(),
            alternate_animation: Vec::new(),
        })
    }

    fn q2_material() -> Q2Material {
        Q2Material {
            frames: Q2Frames {
                frames: [1, 0, 0, 0, 0, 0, 0, 0],
                count: 1,
            },
            lightmap: None,
            vertex_lit: false,
            surface_flags: 0,
            flowing: false,
            warp: false,
            alpha: 1.0,
        }
    }

    #[test]
    fn key_ignores_time_but_tracks_camera_and_styles() {
        let live = input();
        let key = WorldOpsKey::capture(&live, 7, &HashMap::new());
        assert!(key.matches(&live, 7, &HashMap::new()));

        let mut moved_time = live.clone();
        moved_time.time = SourceTime::Milliseconds(987654.0);
        assert!(key.matches(&moved_time, 7, &HashMap::new()));

        let mut moved_camera = live.clone();
        moved_camera.camera.origin = vec3(1.0, 2.0, 3.0);
        assert!(!key.matches(&moved_camera, 7, &HashMap::new()));

        let mut restyled = live.clone();
        restyled.q1_styles[0] = 1;
        assert!(!key.matches(&restyled, 7, &HashMap::new()));

        assert!(!key.matches(&live, 8, &HashMap::new()));
    }

    #[test]
    fn key_tracks_lights_entity_tint_and_overrides() {
        let live = input();
        let key = WorldOpsKey::capture(&live, 0, &HashMap::new());

        let mut lit = live.clone();
        lit.lights.push(SurfaceDynamicLight {
            origin: vec3(0.0, 0.0, 0.0),
            radius: 100.0,
            minimum: 0.0,
            color: vec3(1.0, 1.0, 1.0),
        });
        assert!(!key.matches(&lit, 0, &HashMap::new()));

        let mut tinted = live.clone();
        tinted.entity_rgba = [255, 0, 0, 255];
        assert!(!key.matches(&tinted, 0, &HashMap::new()));

        let mut framed = live.clone();
        framed.animation_frame = Some(3.0);
        assert!(!key.matches(&framed, 0, &HashMap::new()));

        let mut alternate = live;
        alternate.alternate_animation = true;
        assert!(!key.matches(&alternate, 0, &HashMap::new()));
    }

    #[test]
    fn plain_legacy_surfaces_are_static() {
        assert!(surface_is_static(
            &legacy_surface(q1_material(Q1Surface::Ordinary)),
            false
        ));
        assert!(surface_is_static(&legacy_surface(q1_material(Q1Surface::Fence)), false));
        let q2 = LegacyMaterial::Q2 {
            name: "rock".to_string(),
            material: q2_material(),
        };
        assert!(surface_is_static(&legacy_surface(q2), false));
    }

    #[test]
    fn skies_liquids_and_animated_frames_are_dynamic() {
        assert!(!surface_is_static(
            &legacy_surface(q1_material(Q1Surface::Water)),
            false
        ));
        assert!(!surface_is_static(
            &legacy_surface(q1_material(Q1Surface::Slime)),
            false
        ));
        assert!(!surface_is_static(&legacy_surface(q1_material(Q1Surface::Lava)), false));
        assert!(!surface_is_static(
            &legacy_surface(q1_material(Q1Surface::Teleport)),
            false
        ));

        let mut sky = legacy_surface(q1_material(Q1Surface::Sky));
        let session = qa_core::identity::IdentityOwner::create("cache-test")
            .unwrap()
            .session()
            .clone();
        let layer = |ordinal| RendererImage {
            owner: crate::render::types::ResourceOwner::new(7, session.clone(), 0),
            ordinal,
            source: crate::render::types::ImageSource::Generated { name: String::new() },
            width: 1,
            height: 1,
        };
        let WorldSurfaceData::Legacy { q1_sky, .. } = &mut sky.data else {
            panic!("expected legacy surface");
        };
        *q1_sky = Some(crate::render::scene::world::Q1SkyLayers {
            solid: layer(1),
            overlay: layer(2),
        });
        assert!(!surface_is_static(&sky, false));

        let mut animated = q2_material();
        animated.frames.count = 2;
        let animated = LegacyMaterial::Q2 {
            name: "anim".to_string(),
            material: animated,
        };
        assert!(!surface_is_static(&legacy_surface(animated), false));

        let mut sky_flags = q2_material();
        sky_flags.surface_flags = 4;
        let sky_flags = LegacyMaterial::Q2 {
            name: "sky".to_string(),
            material: sky_flags,
        };
        assert!(!surface_is_static(&legacy_surface(sky_flags), false));

        let mut warp = q2_material();
        warp.warp = true;
        let warp = LegacyMaterial::Q2 {
            name: "water".to_string(),
            material: warp,
        };
        assert!(!surface_is_static(&legacy_surface(warp), false));

        let mut flowing = q2_material();
        flowing.flowing = true;
        let flowing = LegacyMaterial::Q2 {
            name: "flow".to_string(),
            material: flowing,
        };
        assert!(!surface_is_static(&legacy_surface(flowing), false));

        assert!(!surface_is_static(
            &legacy_surface(q1_material(Q1Surface::Ordinary)),
            true
        ));
    }
}
