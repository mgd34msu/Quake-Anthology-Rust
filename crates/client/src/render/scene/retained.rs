//! Retained world-geometry arena (Rendering R3).
//!
//! Fully static world surfaces evaluate once per content key and stay
//! resident: positions, indices, and statically evaluated pass attributes
//! live in one [`RetainedSurfaceData`] allocation shared by every frame's
//! operations through an [`Arc`]. Per-frame operations carry only cheap
//! parameters (projector rows, texture selections, pipeline state) plus a
//! [`RetainedId`] so backends skip re-upload while the allocation matches.
//!
//! Projection is deferred, never duplicated: the scene evaluates static
//! attributes with an identity projection, the CPU backend resolves through
//! a rebuilt [`ViewProjector`] (bitwise identical to the legacy path), and
//! the GL backend transforms in the vertex shader from a composed MVP
//! uniform. Anything time- or view-varying (deforms, waveforms, animated
//! texture coordinates, fog volumes, dynamic lights, Q2 fragment lighting)
//! classifies [`Legacy`](SurfaceRetainClass::Legacy) and keeps today's
//! immediate path, so behavior on those surfaces cannot change.

use std::collections::HashMap;
use std::sync::Arc;

use qa_core::math::Vec3;

use crate::materials::evaluate::MaterialDrawContext;
use crate::materials::legacy::{Q1Material, Q1Surface, Q2Material};
use crate::materials::material::{AlphaGen, ColorGen, ShaderMap, TexGen, TexMod};
use crate::render::types::{MultitextureVertex, RenderVertex, RetainedDraw, RetainedId, RetainedSurfaceData};
use crate::view::ViewProjector;

/// Arena key: everything the cached arrays depend on. Texture selections
/// and pipeline state stay per-frame parameters, so animation frames and
/// blend state never appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetainedKey {
    /// Source surface index.
    pub surface: u32,
    /// Material table revision (remaps invalidate entries).
    pub revision: u64,
    /// Entity color bytes (baked into cached colors).
    pub entity: [u8; 4],
    /// Identity-light bits (baked into identity-lit colors).
    pub identity_light: u32,
    /// Legacy shape bits: Q1 fog active plus lightmap encoding.
    pub shape: u32,
}

/// Resident arena mapping content keys to shared surface allocations.
/// Entries are append-only within a map; generation counters only grow.
#[derive(Debug, Default)]
pub struct RetainedArena {
    entries: HashMap<RetainedKey, Arc<RetainedSurfaceData>>,
    next_generation: u64,
}

impl RetainedArena {
    /// Empty arena.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a cached allocation without building.
    #[must_use]
    pub fn get(&self, key: &RetainedKey) -> Option<Arc<RetainedSurfaceData>> {
        self.entries.get(key).cloned()
    }

    /// Intern one allocation: return the cached entry on a hit, otherwise
    /// build with a fresh generation and store it.
    pub fn intern(
        &mut self,
        key: RetainedKey,
        build: impl FnOnce(RetainedId) -> RetainedSurfaceData,
    ) -> Arc<RetainedSurfaceData> {
        if let Some(hit) = self.entries.get(&key) {
            return hit.clone();
        }
        self.next_generation += 1;
        let id = RetainedId {
            surface: key.surface,
            generation: self.next_generation,
        };
        let stored = Arc::new(build(id));
        self.entries.insert(key, stored.clone());
        stored
    }

    /// Entry count (map-lifetime bound for tests).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the arena holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Static-vs-animated classification for one surface draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceRetainClass {
    /// Fully static: positions and attributes cache by content key.
    Retained,
    /// Keep the immediate path; the reason names the varying input.
    Legacy(&'static str),
}

/// Classify a Q3 shader draw. Static stages keep cached attributes while
/// animated *texture selection* stays a per-frame parameter, so `animMap`
/// and video maps remain retained.
#[must_use]
pub fn classify_q3(
    compiled: &crate::materials::compile::CompiledMaterial,
    context: &MaterialDrawContext,
) -> SurfaceRetainClass {
    if !compiled.registered.definition.deforms.is_empty() {
        return SurfaceRetainClass::Legacy("deform moves vertices per frame");
    }
    if context.dynamic_light_batches.is_some() || context.dynamic_lights.is_some() {
        return SurfaceRetainClass::Legacy("projected dynamic lights vary per frame");
    }
    if context.fog.is_some() {
        return SurfaceRetainClass::Legacy("fog volume coordinates track the camera");
    }
    if context.q1_fog.is_some_and(|fog| fog.density > 0.0) {
        return SurfaceRetainClass::Legacy("Q1 fog pass shape varies per frame");
    }
    for pass in &compiled.finished.iterator.passes {
        if pass.bundles.iter().any(|bundle| bundle.is_lightmap) {
            return SurfaceRetainClass::Legacy("Q2 lightmap pass needs fragment lighting");
        }
        let stage = &pass.stage;
        if !matches!(
            stage.rgb_gen,
            ColorGen::Identity
                | ColorGen::IdentityLighting
                | ColorGen::Entity
                | ColorGen::OneMinusEntity
                | ColorGen::Vertex
                | ColorGen::ExactVertex
                | ColorGen::OneMinusVertex
                | ColorGen::Const(_)
        ) {
            return SurfaceRetainClass::Legacy("rgbGen varies per frame");
        }
        if !matches!(
            stage.alpha_gen,
            AlphaGen::Identity
                | AlphaGen::Entity
                | AlphaGen::OneMinusEntity
                | AlphaGen::Vertex
                | AlphaGen::OneMinusVertex
                | AlphaGen::Const(_)
        ) {
            return SurfaceRetainClass::Legacy("alphaGen varies per frame");
        }
        if matches!(stage.tc_gen, TexGen::Environment) {
            return SurfaceRetainClass::Legacy("environment tcGen tracks the camera");
        }
        for tex_mod in &stage.tc_mods {
            match tex_mod {
                TexMod::None => {}
                TexMod::Scale(_) | TexMod::Transform { .. } => {}
                TexMod::Scroll(_)
                | TexMod::Stretch(_)
                | TexMod::Turb(_)
                | TexMod::Rotate(_)
                | TexMod::EntityTranslate => {
                    return SurfaceRetainClass::Legacy("tcMod varies per frame");
                }
            }
        }
        if matches!(stage.map, ShaderMap::None) {
            return SurfaceRetainClass::Legacy("unmapped stage has no retained binding");
        }
    }
    SurfaceRetainClass::Retained
}

/// Classify a Q1 legacy surface. Ordinary and fence surfaces cache;
/// liquids warp per frame and sky keeps its billboard path.
#[must_use]
pub const fn classify_legacy_q1(material: &Q1Material) -> SurfaceRetainClass {
    match material.surface {
        Q1Surface::Ordinary | Q1Surface::Fence => SurfaceRetainClass::Retained,
        Q1Surface::Sky => SurfaceRetainClass::Legacy("sky keeps its billboard path"),
        Q1Surface::Water | Q1Surface::Slime | Q1Surface::Lava | Q1Surface::Teleport => {
            SurfaceRetainClass::Legacy("liquid warp varies per frame")
        }
    }
}

/// Classify a Q2 legacy surface. Warp and flowing surfaces recompute
/// texture coordinates per frame; the rest cache.
#[must_use]
pub const fn classify_legacy_q2(material: &Q2Material) -> SurfaceRetainClass {
    if material.surface_flags & 4 != 0 {
        return SurfaceRetainClass::Legacy("sky keeps its sides path");
    }
    if material.warp {
        return SurfaceRetainClass::Legacy("warp varies per frame");
    }
    if material.flowing {
        return SurfaceRetainClass::Legacy("flowing scroll varies per frame");
    }
    SurfaceRetainClass::Retained
}

/// Resolve cached positions through the frame projector into scratch.
/// Identical operations to the legacy per-vertex projection, so output is
/// bitwise identical to the immediate path.
pub fn resolve_retained_positions(draw: &RetainedDraw, projector: &ViewProjector, out: &mut Vec<qa_core::math::Vec4>) {
    out.clear();
    out.reserve(draw.surface.positions.len());
    for position in &draw.surface.positions {
        out.push(projector.project_or_zero(*position));
    }
}

/// Fill one batch's vertices from resolved positions plus cached pass
/// attributes into caller scratch (single or paired payload plus indices).
#[allow(clippy::too_many_arguments)]
pub fn fill_retained_batch_vertices(
    draw: &RetainedDraw,
    batch_index: usize,
    positions: &[qa_core::math::Vec4],
    paired: bool,
    single: &mut Vec<RenderVertex>,
    pair: &mut Vec<MultitextureVertex>,
    indices: &mut Vec<u32>,
) {
    single.clear();
    pair.clear();
    indices.clear();
    let batch = &draw.batches[batch_index];
    let pass = &draw.surface.passes[batch.pass as usize];
    debug_assert_eq!(positions.len(), draw.surface.positions.len());
    debug_assert_eq!(pass.tex_coords.len(), draw.surface.positions.len());
    debug_assert_eq!(pass.colors.len(), draw.surface.positions.len());
    if paired {
        debug_assert_eq!(pass.tex_coords2.len(), draw.surface.positions.len());
        pair.reserve(draw.surface.positions.len());
        for ((position, tex_coord), color) in positions.iter().zip(pass.tex_coords.iter()).zip(pass.colors.iter()) {
            pair.push(MultitextureVertex {
                base: RenderVertex {
                    position: *position,
                    tex_coord: *tex_coord,
                    color: *color,
                },
                tex_coord2: pass.tex_coords2[pair.len()],
            });
        }
    } else {
        single.reserve(draw.surface.positions.len());
        for ((position, tex_coord), color) in positions.iter().zip(pass.tex_coords.iter()).zip(pass.colors.iter()) {
            single.push(RenderVertex {
                position: *position,
                tex_coord: *tex_coord,
                color: *color,
            });
        }
    }
    let start = batch.range.start as usize;
    let end = start + batch.range.count as usize;
    indices.extend_from_slice(&draw.surface.indices[start..end]);
}

/// Recover model-local positions from identity-projected batch vertices.
/// Retained evaluation projects with the identity, so `w` is exactly `1.0`.
#[must_use]
pub fn unproject_identity(position: qa_core::math::Vec4) -> Vec3 {
    debug_assert_eq!(position.w, 1.0);
    qa_core::math::vec3(position.x, position.y, position.z)
}

#[cfg(test)]
mod tests {
    use qa_core::math::{vec2, vec3, vec4};

    use super::*;
    use crate::materials::compile::{
        compile_shader_script, CompileOptions, RegisteredImage, ShaderRegistrationHost, SourceImageRequest,
    };
    use crate::materials::deform::{DeformView, RendererNoise};
    use crate::materials::evaluate::{prepare_material_batches, MaterialDrawContext};
    use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
    use crate::materials::legacy::Q2Frames;

    struct Host;

    impl ShaderRegistrationHost for Host {
        fn white_image(&self) -> RegisteredImage {
            RegisteredImage { image: 1, tmu: 0 }
        }

        fn default_image(&self) -> RegisteredImage {
            RegisteredImage { image: 2, tmu: 0 }
        }

        fn lightmap_image(&self) -> RegisteredImage {
            RegisteredImage { image: 3, tmu: 1 }
        }

        fn find_image(&mut self, _request: &SourceImageRequest) -> Option<RegisteredImage> {
            Some(RegisteredImage { image: 4, tmu: 0 })
        }

        fn play_shader_cinematic(&mut self, _name: &str) -> Option<crate::materials::compile::RegisteredShaderVideo> {
            None
        }

        fn apply_sun(&mut self, _sun: crate::materials::material::RegisteredSun) {}

        fn initialize_sky_tex_coords(&mut self, _height: f32) {}

        fn print_warning(&mut self, _message: &str) {}
    }

    fn compiled(script: &str) -> crate::materials::compile::CompiledMaterial {
        let mut host = Host;
        let materials = compile_shader_script(script, &mut host, "<test>", &CompileOptions::default()).unwrap();
        assert_eq!(materials.len(), 1);
        materials.into_iter().next().unwrap()
    }

    fn context<'a>(
        noise: &'a RendererNoise,
        project: &'a dyn Fn(Vec3) -> qa_core::math::Vec4,
    ) -> MaterialDrawContext<'a> {
        MaterialDrawContext {
            time: 1.25,
            time_offset: 0.0,
            refdef_time: 0.0,
            identity_light: 1.0,
            entity_rgba: [255, 255, 255, 255],
            lighting: None,
            view_origin: vec3(0.0, 0.0, 0.0),
            local_view_origin: vec3(0.0, 0.0, 0.0),
            noise,
            shader_tex_coord: vec2(0.0, 0.0),
            deform_view: DeformView {
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                mirror: false,
                entity_axis: None,
                non_normalized_axis: None,
            },
            projection_shadow: None,
            render_text: Vec::new(),
            dynamic_lights: None,
            dynamic_light_batches: None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
            q1_fog: None,
            fog: None,
            project,
        }
    }

    fn geometry() -> MaterialGeometry {
        let vertex = |position: Vec3| {
            MaterialVertex::new(
                position,
                vec3(0.0, 0.0, 1.0),
                vec2(0.25, 0.5),
                vec2(0.75, 0.125),
                [10, 20, 30, 40],
            )
        };
        MaterialGeometry {
            vertices: vec![
                vertex(vec3(0.0, 0.0, 0.0)),
                vertex(vec3(1.0, 0.0, 0.0)),
                vertex(vec3(0.0, 1.0, 0.0)),
            ],
            indices: vec![0, 1, 2],
        }
    }

    fn key(surface: u32, revision: u64) -> RetainedKey {
        RetainedKey {
            surface,
            revision,
            entity: [255, 255, 255, 255],
            identity_light: 1.0f32.to_bits(),
            shape: 0,
        }
    }

    #[test]
    fn arena_interns_by_content_key() {
        let mut arena = RetainedArena::new();
        assert!(arena.is_empty());
        let build = |id: RetainedId| RetainedSurfaceData {
            id,
            positions: vec![vec3(1.0, 2.0, 3.0)],
            indices: vec![0],
            passes: Vec::new(),
        };
        let first = arena.intern(key(7, 3), build);
        let second = arena.intern(key(7, 3), build);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(arena.len(), 1);
        let bumped = arena.intern(key(7, 4), build);
        assert!(!Arc::ptr_eq(&first, &bumped));
        assert!(bumped.id.generation > first.id.generation);
        assert_eq!(bumped.id.surface, 7);
        assert_eq!(arena.len(), 2);
        assert!(Arc::ptr_eq(&bumped, &arena.get(&key(7, 4)).unwrap()));
        assert!(arena.get(&key(9, 3)).is_none());
    }

    #[test]
    fn static_q3_stages_classify_retained() {
        let noise = RendererNoise::new();
        let project = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        let context = context(&noise, &project);
        for script in [
            "rock\n{\n {\n map textures/rock.tga\n }\n}\n",
            "rock\n{\n {\n map textures/rock.tga\n tcMod scale 2 2\n }\n}\n",
            "rock\n{\n {\n map textures/rock.tga\n rgbGen vertex\n alphaGen const 0.5\n }\n}\n",
        ] {
            let material = compiled(script);
            assert_eq!(
                classify_q3(&material, &context),
                SurfaceRetainClass::Retained,
                "script: {script}"
            );
        }
    }

    #[test]
    fn varying_q3_inputs_classify_legacy() {
        let noise = RendererNoise::new();
        let project = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        let context = context(&noise, &project);
        for (script, reason) in [
            (
                "rock\n{\n deformVertexes wave 1 sin 0 1 0 1\n {\n map textures/rock.tga\n }\n}\n",
                "deform moves vertices per frame",
            ),
            (
                "rock\n{\n {\n map textures/rock.tga\n rgbGen wave sin 0 1 0 1\n }\n}\n",
                "rgbGen varies per frame",
            ),
            (
                "rock\n{\n {\n map textures/rock.tga\n tcMod scroll 1 0\n }\n}\n",
                "tcMod varies per frame",
            ),
            (
                "rock\n{\n {\n map textures/rock.tga\n tcGen environment\n }\n}\n",
                "environment tcGen tracks the camera",
            ),
        ] {
            let material = compiled(script);
            assert_eq!(
                classify_q3(&material, &context),
                SurfaceRetainClass::Legacy(reason),
                "script: {script}"
            );
        }
    }

    #[test]
    fn legacy_surfaces_classify_by_varying_inputs() {
        let q1 = |surface| Q1Material {
            name: "test".to_string(),
            texture: 1,
            lightmap: None,
            vertex_lit: false,
            surface,
            alpha: 1.0,
            animation: Vec::new(),
            alternate_animation: Vec::new(),
        };
        assert_eq!(
            classify_legacy_q1(&q1(Q1Surface::Ordinary)),
            SurfaceRetainClass::Retained
        );
        assert_eq!(classify_legacy_q1(&q1(Q1Surface::Fence)), SurfaceRetainClass::Retained);
        assert!(matches!(
            classify_legacy_q1(&q1(Q1Surface::Water)),
            SurfaceRetainClass::Legacy(_)
        ));
        assert!(matches!(
            classify_legacy_q1(&q1(Q1Surface::Sky)),
            SurfaceRetainClass::Legacy(_)
        ));
        let q2 = |surface_flags: u32, warp: bool, flowing: bool| Q2Material {
            frames: Q2Frames {
                frames: [1, 0, 0, 0, 0, 0, 0, 0],
                count: 1,
            },
            lightmap: None,
            vertex_lit: false,
            surface_flags,
            flowing,
            warp,
            alpha: 1.0,
        };
        assert_eq!(classify_legacy_q2(&q2(0, false, false)), SurfaceRetainClass::Retained);
        assert!(matches!(
            classify_legacy_q2(&q2(0, true, false)),
            SurfaceRetainClass::Legacy(_)
        ));
        assert!(matches!(
            classify_legacy_q2(&q2(0, false, true)),
            SurfaceRetainClass::Legacy(_)
        ));
        assert!(matches!(
            classify_legacy_q2(&q2(4, false, false)),
            SurfaceRetainClass::Legacy(_)
        ));
    }

    #[test]
    fn identity_evaluation_preserves_attributes_and_positions() {
        let noise = RendererNoise::new();
        let material = compiled("rock\n{\n {\n map textures/rock.tga\n }\n}\n");
        let identity = |point: Vec3| vec4(point.x, point.y, point.z, 1.0);
        let shifted = |point: Vec3| vec4(point.x + 10.0, point.y + 20.0, point.z + 30.0, 2.0);
        let plain = prepare_material_batches(&material, geometry(), &context(&noise, &identity)).unwrap();
        let moved = prepare_material_batches(&material, geometry(), &context(&noise, &shifted)).unwrap();
        assert_eq!(plain.len(), moved.len());
        for (left, right) in plain.iter().zip(moved.iter()) {
            assert_eq!(left.indices, right.indices);
            assert_eq!(left.vertices.len(), right.vertices.len());
            for (index, (a, b)) in left.vertices.iter().zip(right.vertices.iter()).enumerate() {
                assert_eq!(a.tex_coord, b.tex_coord, "uv {index}");
                assert_eq!(a.tex_coord2, b.tex_coord2, "uv2 {index}");
                assert_eq!(a.color, b.color, "color {index}");
                assert_eq!(unproject_identity(a.position), geometry().vertices[index].position);
            }
        }
    }
}
