//! Renderer: scene submission plus the CPU and GL backends.
//!
//! Donor provenance: `src/contracts/scene.ts` (`SceneEntity`,
//! `SceneLight`, `SceneLightProfile`, `SceneParticle`),
//! `src/contracts/render.ts` (ordered backend contract),
//! `src/content/q3/presentation/ref-entity.ts` (`RF_*`),
//! `src/content/q3/presentation/marks.ts` (`ImpactMarkRequest`,
//! `MAX_MARK_*`), `src/render/scene/submissions.ts`
//! (`sourceDrawGroup` field limits),
//! `src/render/scene/source-sort.ts` (`packSourceDrawSort`), and the full
//! `src/render/*` tree (commands, CPU rasterizer, GL backend, scene graph,
//! worker transport).
//!
//! This module keeps the headless scene-submission trait
//! ([`RendererBackend`]) with its recording implementation
//! ([`NullRenderer`]); the default backend is the real CPU rasterizer
//! ([`cpu::CpuRenderer`]). The ordered command-stream contract lives in
//! [`types`], consumed synchronously by the CPU (`cpu`) and GL (`gl`)
//! implementations. Scene-graph preparation lives in `scene`; the former
//! worker thread is a headless-compatible synchronous driver (`driver`)
//! with identical ordering semantics.

pub mod cpu;
pub mod debug_graph;
pub mod driver;
pub mod dynamic_texture;
pub mod error;
pub mod execution;
pub mod frame;
pub mod gl;
pub mod image_journal;
pub mod material2d;
pub mod output_gamma;
pub mod q3_hardware;
pub mod scene;
pub mod stage_timings;
pub mod types;

pub use error::RenderError;

use qa_core::math::{Vec3, Vec4};

use crate::view::{ModelTransform, SceneCamera};
use crate::ClientError;

/// `RF_MINLIGHT`: full-bright entity.
pub const RF_MINLIGHT: u32 = 1;
/// `RF_THIRD_PERSON`: hide in first person.
pub const RF_THIRD_PERSON: u32 = 2;
/// `RF_FIRST_PERSON`: depth-hacked view model.
pub const RF_FIRST_PERSON: u32 = 4;
/// `RF_DEPTHHACK`: depth-hacked entity.
pub const RF_DEPTHHACK: u32 = 8;
/// `RF_NOSHADOW`: skip shadow casting.
pub const RF_NOSHADOW: u32 = 64;
/// `RF_LIGHTING_ORIGIN`: use the lighting origin.
pub const RF_LIGHTING_ORIGIN: u32 = 128;
/// `RF_SHADOW_PLANE`: use the shadow plane.
pub const RF_SHADOW_PLANE: u32 = 256;
/// `RF_WRAP_FRAMES`: wrap animation frames.
pub const RF_WRAP_FRAMES: u32 = 512;

/// World-entity draw-sort sentinel (refentities stay below).
pub const ENTITY_WORLD: u32 = 1022;
/// Maximum shader rank in the draw-sort word.
pub const MAX_DRAW_SORT_SHADER: u32 = 16383;
/// Maximum fog index in the draw-sort word.
pub const MAX_DRAW_SORT_FOG: u32 = 31;
/// Maximum dlight flag in the draw-sort word.
pub const MAX_DRAW_SORT_DLIGHT: u32 = 3;

/// Pack a source draw-sort word: `shader:15 | entity:10 | fog:5 | dlight:2`.
pub fn pack_draw_sort(shader: u32, entity: u32, fog: u32, dlight: u32) -> Result<u32, ClientError> {
    if shader > MAX_DRAW_SORT_SHADER {
        return Err(ClientError::BadDrawSort {
            field: "shader",
            value: shader,
        });
    }
    if entity > ENTITY_WORLD {
        return Err(ClientError::BadDrawSort {
            field: "entity",
            value: entity,
        });
    }
    if fog > MAX_DRAW_SORT_FOG {
        return Err(ClientError::BadDrawSort {
            field: "fog",
            value: fog,
        });
    }
    if dlight > MAX_DRAW_SORT_DLIGHT {
        return Err(ClientError::BadDrawSort {
            field: "dlight",
            value: dlight,
        });
    }
    Ok((shader << 17) | (entity << 7) | (fog << 2) | dlight)
}

/// Entity animation pose.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModelPose {
    /// Current frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Inter-frame blend.
    pub back_lerp: f32,
}

/// Submitted scene entity (`SceneEntity` contract).
///
/// Models are content-owned handles; `entity_number` is the wire slot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneEntity {
    /// Wire entity number.
    pub entity_number: u32,
    /// Content model handle.
    pub model: u32,
    /// World transform.
    pub transform: ModelTransform,
    /// Previous origin for interpolation.
    pub previous_origin: Vec3,
    /// Animation pose.
    pub pose: ModelPose,
    /// Skin index.
    pub skin: i32,
    /// Shader color.
    pub color: Vec4,
    /// `RF_*` flag bits.
    pub flags: u32,
    /// Lighting origin (`RF_LIGHTING_ORIGIN`).
    pub lighting_origin: Option<Vec3>,
    /// Shadow plane (`RF_SHADOW_PLANE`).
    pub shadow_plane: Option<f32>,
    /// Whole-object opacity.
    pub opacity: Option<f32>,
}

/// Submitted particle (`SceneParticle` contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SceneParticle {
    /// Palette-indexed particle (Q1/Q2).
    Indexed {
        /// World origin.
        origin: Vec3,
        /// Palette index.
        palette_index: i32,
        /// Opacity.
        alpha: f32,
        /// Pixel size.
        size: f32,
    },
    /// True-color particle (Q3).
    Rgba {
        /// World origin.
        origin: Vec3,
        /// Shader color.
        color: Vec4,
        /// World size.
        size: f32,
        /// Billboard rotation.
        rotation: f32,
    },
}

/// Q2 light shadow mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightShadow {
    /// No shadow.
    None,
    /// Cast at a resolution.
    Cast {
        /// Shadow resolution.
        resolution: i32,
    },
}

/// Dynamic-light profile (`SceneLightProfile` contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightProfile {
    /// Q1/Q3 point light.
    Simple,
    /// Q2 scaled/coned light.
    Q2 {
        /// Intensity scale.
        scale: f32,
        /// Spot cone direction and cosine half-angle.
        cone: Option<(Vec3, f32)>,
        /// Shadow mode.
        shadow: LightShadow,
    },
}

/// Submitted dynamic light (`SceneLight` contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLight {
    /// World origin.
    pub origin: Vec3,
    /// Light color.
    pub color: Vec3,
    /// Radius in world units.
    pub radius: f32,
    /// Additive blending.
    pub additive: bool,
    /// Family profile.
    pub profile: LightProfile,
}

/// Retained impact-mark caps (`cg_marks.c`).
pub const MAX_MARK_POLYS: usize = 256;
/// Maximum vertices per mark poly.
pub const MAX_MARK_POLY_VERTICES: usize = 10;
/// Mark lifetime in milliseconds.
pub const MARK_TOTAL_TIME_MS: i32 = 10000;
/// Mark fade-out in milliseconds.
pub const MARK_FADE_TIME_MS: i32 = 1000;

/// Submitted impact decal (`ImpactMarkRequest`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneDecal {
    /// Content shader handle (`u32::MAX` when untextured).
    pub shader: u32,
    /// Impact origin.
    pub origin: Vec3,
    /// Impact direction.
    pub direction: Vec3,
    /// Texture orientation in degrees.
    pub orientation: f32,
    /// Mark color.
    pub color: Vec4,
    /// Fade alpha over the lifetime.
    pub alpha_fade: bool,
    /// Mark radius.
    pub radius: f32,
    /// Temporary (single-frame) mark.
    pub temporary: bool,
}

/// Frame view: camera plus client clock and flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderView {
    /// Scene camera.
    pub camera: SceneCamera,
    /// Client time in milliseconds.
    pub time_ms: i32,
    /// View flags (family-defined).
    pub flags: u32,
}

/// Per-frame submission counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameStats {
    /// Submitted entities.
    pub entities: usize,
    /// Submitted particles.
    pub particles: usize,
    /// Submitted decals.
    pub decals: usize,
    /// Submitted lights.
    pub lights: usize,
}

/// Headless renderer backend: ordered synchronous scene submission.
pub trait RendererBackend {
    /// Open a frame for a view.
    fn begin_frame(&mut self, view: &RenderView);
    /// Submit entities.
    fn submit_entities(&mut self, entities: &[SceneEntity]);
    /// Submit particles.
    fn submit_particles(&mut self, particles: &[SceneParticle]);
    /// Submit decals.
    fn submit_decals(&mut self, decals: &[SceneDecal]);
    /// Submit dynamic lights.
    fn submit_lights(&mut self, lights: &[SceneLight]);
    /// Close the frame, returning submission counts.
    fn end_frame(&mut self) -> FrameStats;
}

/// Recording headless backend for tests and dedicated servers.
#[derive(Debug, Clone, Default)]
pub struct NullRenderer {
    /// Last opened view.
    pub view: Option<RenderView>,
    /// Submitted entities since the last frame began.
    pub entities: Vec<SceneEntity>,
    /// Submitted particles since the last frame began.
    pub particles: Vec<SceneParticle>,
    /// Submitted decals since the last frame began.
    pub decals: Vec<SceneDecal>,
    /// Submitted lights since the last frame began.
    pub lights: Vec<SceneLight>,
    frames: u64,
}

impl NullRenderer {
    /// Fresh recorder.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            view: None,
            entities: Vec::new(),
            particles: Vec::new(),
            decals: Vec::new(),
            lights: Vec::new(),
            frames: 0,
        }
    }

    /// Completed frames.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }
}

impl RendererBackend for NullRenderer {
    fn begin_frame(&mut self, view: &RenderView) {
        self.view = Some(*view);
        self.entities.clear();
        self.particles.clear();
        self.decals.clear();
        self.lights.clear();
    }

    fn submit_entities(&mut self, entities: &[SceneEntity]) {
        self.entities.extend_from_slice(entities);
    }

    fn submit_particles(&mut self, particles: &[SceneParticle]) {
        self.particles.extend_from_slice(particles);
    }

    fn submit_decals(&mut self, decals: &[SceneDecal]) {
        self.decals.extend_from_slice(decals);
    }

    fn submit_lights(&mut self, lights: &[SceneLight]) {
        self.lights.extend_from_slice(lights);
    }

    fn end_frame(&mut self) -> FrameStats {
        self.frames += 1;
        FrameStats {
            entities: self.entities.len(),
            particles: self.particles.len(),
            decals: self.decals.len(),
            lights: self.lights.len(),
        }
    }
}

/// Build the default backend: the real CPU rasterizer.
#[must_use]
pub fn default_backend(width: u32, height: u32, session: qa_core::identity::SessionId) -> cpu::CpuRenderer {
    cpu::CpuRenderer::new(width, height, session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{CameraClip, Rect};
    use qa_core::math::{vec3, vec4};

    fn axis() -> qa_core::math::Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    #[test]
    fn draw_sort_packs_and_validates_fields() {
        assert_eq!(pack_draw_sort(1, 2, 3, 1).unwrap(), (1 << 17) | (2 << 7) | (3 << 2) | 1);
        assert_eq!(pack_draw_sort(0, ENTITY_WORLD, 0, 0).unwrap(), 1022 << 7);
        assert!(pack_draw_sort(16384, 0, 0, 0).is_err());
        assert!(pack_draw_sort(0, 1023, 0, 0).is_err());
        assert!(pack_draw_sort(0, 0, 32, 0).is_err());
        assert!(pack_draw_sort(0, 0, 0, 4).is_err());
    }

    #[test]
    fn null_renderer_records_a_frame() {
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: axis(),
            projection: [0.0; 16],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        let view = RenderView {
            camera,
            time_ms: 100,
            flags: 0,
        };
        let entity = SceneEntity {
            entity_number: 1,
            model: 7,
            transform: ModelTransform {
                origin: vec3(0.0, 0.0, 0.0),
                axis: axis(),
                scale: 1.0,
            },
            previous_origin: vec3(0.0, 0.0, 0.0),
            pose: ModelPose {
                frame: 1,
                old_frame: 0,
                back_lerp: 0.5,
            },
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            flags: RF_MINLIGHT,
            lighting_origin: None,
            shadow_plane: None,
            opacity: None,
        };
        let mut renderer = NullRenderer::new();
        renderer.begin_frame(&view);
        renderer.submit_entities(&[entity]);
        renderer.submit_particles(&[SceneParticle::Indexed {
            origin: vec3(1.0, 2.0, 3.0),
            palette_index: 111,
            alpha: 1.0,
            size: 1.0,
        }]);
        renderer.submit_decals(&[SceneDecal {
            shader: 3,
            origin: vec3(0.0, 0.0, 0.0),
            direction: vec3(0.0, 0.0, 1.0),
            orientation: 45.0,
            color: vec4(1.0, 0.0, 0.0, 1.0),
            alpha_fade: true,
            radius: 8.0,
            temporary: false,
        }]);
        renderer.submit_lights(&[SceneLight {
            origin: vec3(0.0, 0.0, 64.0),
            color: vec3(1.0, 1.0, 1.0),
            radius: 200.0,
            additive: true,
            profile: LightProfile::Simple,
        }]);
        let stats = renderer.end_frame();
        assert_eq!(
            stats,
            FrameStats {
                entities: 1,
                particles: 1,
                decals: 1,
                lights: 1
            }
        );
        assert_eq!(renderer.frames(), 1);
        assert_eq!(renderer.view, Some(view));
        assert_eq!(renderer.entities[0].model, 7);
        renderer.begin_frame(&view);
        assert_eq!(renderer.entities.len(), 0);
    }
}
