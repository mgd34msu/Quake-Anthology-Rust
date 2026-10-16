//! Q3-style append-only scene ranges in alternating owned packets.
use crate::assets::{Assets, MaterialId, ModelId, PaletteId, ShaderBindingId, Vertex};
use crate::shader::reference::{
    NativeShaders, ResolvedShader, ShaderDefault, ShaderRef, ShaderView,
};
use crate::surface_cache::IndexedLighting;
use crate::world::{VisibleSurface, WorldId, WorldPresentationId};
pub use qa_core::primitives::LightStyle;
use qa_core::primitives::{Orientation, PlayerViewFlags, ThinkTime, Vec3};
use std::cmp::Ordering as SortOrdering;
mod model_lod;
mod patch;
pub use crate::assets::FrameTiming;
pub use model_lod::ModelLodSettings;
pub(crate) use model_lod::projection_scales;
pub use patch::{PatchSelection, PatchTriangles};
pub const DEFAULT_CURVE_ERROR: f32 = 250.0;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Viewport {
    pub fn fits(self, width: u32, height: u32) -> bool {
        self.width != 0
            && self.height != 0
            && self.x.checked_add(self.width).is_some_and(|x| x <= width)
            && self.y.checked_add(self.height).is_some_and(|y| y <= height)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendPhase {
    #[default]
    AfterView,
    FinalPalette,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CpuPresentation {
    #[default]
    Rgb,
    Indexed {
        palette: PaletteId,
        lighting: IndexedLighting,
        ambient: u8,
        fullbright: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PerspectiveStep {
    #[default]
    Eight,
    Sixteen,
}
impl PerspectiveStep {
    pub fn pixels(self) -> u32 {
        match self {
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaletteShift {
    pub destination: [i32; 3],
    /// Native 0..256 fixed-point weight, before gamma-table application.
    pub percent: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteTransform {
    /// Native contents, damage, bonus and powerup order.
    pub shifts: [PaletteShift; 4],
    pub gamma: [u8; 256],
    pub operation: PaletteOperation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaletteOperation {
    #[default]
    SequentialShifts,
    /// ref_soft uses refdef's combined blend before gamma lookup.
    ScreenBlend,
}
impl Default for PaletteTransform {
    fn default() -> Self {
        Self {
            shifts: [PaletteShift::default(); 4],
            gamma: std::array::from_fn(|i| i as u8),
            operation: PaletteOperation::SequentialShifts,
        }
    }
}
/// Per-view capability data. Source adapters resolve cvars before copying a
/// Refdef; visibility and both drawing backends consume the same submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewPolicy {
    pub ignore_pvs: bool,
    pub ignore_areas: bool,
    pub cull_bounds: bool,
    pub draw_world: bool,
    pub draw_entities: bool,
}
impl Default for ViewPolicy {
    fn default() -> Self {
        Self {
            ignore_pvs: false,
            ignore_areas: false,
            cull_bounds: true,
            draw_world: true,
            draw_entities: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Refdef {
    pub viewport: Viewport,
    pub origin: Vec3,
    /// Forward, left, up: the native Q3 view axis convention.
    pub axes: [Vec3; 3],
    pub fov: [f32; 2],
    pub near: f32,
    pub far: f32,
    pub policy: ViewPolicy,
    pub time_ms: i64,
    /// CLIENT presentation time can retain fractional native seconds.
    /// None uses this view's ordinary millisecond clock.
    pub model_time: Option<ThinkTime>,
    pub frame_timing: FrameTiming,
    pub hyperspace: bool,
    pub portal: bool,
    pub reflected_view: bool,
    /// World-space retained half-space for a portal child. Native module
    /// refdefs acquire it at the shared scene boundary, not their ABI.
    pub clip_plane: Option<qa_core::primitives::Plane>,
    pub pvs_origin: Option<Vec3>,
    /// Native shader text rows retain byte values after their first NUL.
    pub text: [[u8; 32]; 8],
    /// Cached presentation identity light (Q3 native overbright response).
    pub identity_light: f32,
    pub blend: [f32; 4],
    pub damage_blend: [f32; 4],
    /// Native damage vignette fraction; zero requests a full-screen blend.
    pub damage_border: f32,
    pub flags: PlayerViewFlags,
    pub software_waterwarp: bool,
    pub blend_phase: BlendPhase,
    /// Final palette shifts also cover a seat's statusbar/console outside the
    /// camera viewport. None uses the camera viewport for standalone scenes.
    pub blend_viewport: Option<Viewport>,
    /// Presentation is independent of the map format, movement and modules.
    pub cpu_presentation: CpuPresentation,
    pub perspective_step: PerspectiveStep,
    /// Native software color shifts preserve integer rounding and gamma;
    /// GL continues to use the combined screen blend. None is unshifted.
    pub palette_transform: Option<PaletteTransform>,
    pub lightstyles: [LightStyle; 256],
    pub curve_error: f32,
    pub model_lod: ModelLodSettings,
}
pub fn fov_from_x(fov_x: f32, width: u32, height: u32) -> [f32; 2] {
    let fov_x = if fov_x.is_finite() {
        fov_x.clamp(1.0, 179.0)
    } else {
        90.0
    };
    let aspect = height as f32 / width.max(1) as f32;
    let fov_y = (2.0 * ((fov_x.to_radians() * 0.5).tan() * aspect).atan()).to_degrees();
    [fov_x, fov_y]
}

pub(crate) fn milliseconds_to_seconds(time_ms: i64) -> f32 {
    (time_ms as f64 * 0.001) as f32
}

impl Refdef {
    pub fn hyperspace_color(&self) -> [u8; 4] {
        let value = (self.time_ms & 255) as u8;
        [value, value, value, 255]
    }
    pub fn time_seconds(&self) -> f32 {
        milliseconds_to_seconds(self.time_ms)
    }
    pub fn model_time(&self) -> ThinkTime {
        self.model_time
            .unwrap_or(ThinkTime::Milliseconds(self.time_ms))
    }

    /// Shared view boundary for visibility and both drawing backends.
    /// Returns the native half-FOV tangents in the original operation order.
    pub fn validate(&self, width: u32, height: u32) -> Option<[f32; 2]> {
        let tangent = self.tangents()?;
        if !self.viewport.fits(width, height)
            || (self.blend_phase == BlendPhase::FinalPalette
                && self.blend_viewport.is_some_and(|v| !v.fits(width, height)))
            || !self.origin.is_finite()
            || self.pvs_origin.is_some_and(|origin| !origin.is_finite())
            || !self.axes.iter().all(|axis| axis.is_finite())
            || !self
                .blend
                .iter()
                .chain(self.damage_blend.iter())
                .all(|value| value.is_finite())
            || self
                .model_time
                .is_some_and(|time| !time.seconds().is_finite())
            || !self.damage_border.is_finite()
            || !self.near.is_finite()
            || self.near <= 0.0
            || !(1.0 / self.near).is_finite()
            || !self.far.is_finite()
            || self.far <= self.near
            || !self.identity_light.is_finite()
            || self.clip_plane.is_some_and(|plane| !plane.is_valid())
            || !(0.0..=1.0).contains(&self.identity_light)
        {
            return None;
        }
        Some(tangent)
    }
    fn tangents(&self) -> Option<[f32; 2]> {
        if self
            .fov
            .iter()
            .any(|&fov| !fov.is_finite() || fov <= 0.0 || fov >= 180.0)
        {
            return None;
        }
        let tangent = self.fov.map(|fov| (fov.to_radians() * 0.5).tan());
        tangent
            .iter()
            .all(|&t| t > 0.0 && t.is_finite())
            .then_some(tangent)
    }
    pub fn projection_scales(&self) -> Option<[f32; 2]> {
        self.tangents().map(projection_scales)
    }
}
impl Default for Refdef {
    fn default() -> Self {
        Self {
            viewport: Viewport::default(),
            origin: Vec3::default(),
            axes: Orientation::default().axes,
            fov: [90.0, 75.0],
            near: 4.0,
            far: 65536.0,
            policy: ViewPolicy::default(),
            time_ms: 0,
            model_time: None,
            frame_timing: FrameTiming::default(),
            hyperspace: false,
            portal: false,
            reflected_view: false,
            clip_plane: None,
            pvs_origin: None,
            text: [[0; 32]; 8],
            identity_light: 1.0,
            blend: [0.0; 4],
            damage_blend: [0.0; 4],
            damage_border: 0.2,
            flags: PlayerViewFlags::default(),
            software_waterwarp: false,
            blend_phase: BlendPhase::AfterView,
            blend_viewport: None,
            cpu_presentation: CpuPresentation::Rgb,
            perspective_step: PerspectiveStep::Eight,
            palette_transform: None,
            lightstyles: [LightStyle::default(); 256],
            curve_error: DEFAULT_CURVE_ERROR,
            model_lod: ModelLodSettings::default(),
        }
    }
}

pub(crate) struct DamageVignette {
    color: [u8; 4],
    inset: Option<u32>,
}
impl DamageVignette {
    pub(crate) fn load(view: &Refdef) -> Option<Self> {
        let color = view
            .damage_blend
            .map(|value| qa_core::math::native_integer(f64::from(value * 255.0)) as u8);
        if color[3] == 0 {
            return None;
        }
        let inset = (view.damage_border > 0.0).then(|| {
            (view.viewport.width.min(view.viewport.height) as f32 * view.damage_border.min(0.5))
                as u32
        });
        // A positive native fraction can truncate to a degenerate border.
        if inset == Some(0) {
            return None;
        }
        Some(Self { color, inset })
    }

    pub(crate) fn color_at(&self, viewport: Viewport, x: u32, y: u32) -> [f32; 4] {
        let mut color = self.color.map(|value| f32::from(value) / 255.0);
        if let Some(inset) = self.inset {
            let x = (x - viewport.x) as f32 + 0.5;
            let y = (y - viewport.y) as f32 + 0.5;
            let edge = x
                .min(y)
                .min(viewport.width as f32 - x)
                .min(viewport.height as f32 - y);
            color[3] *= (1.0 - edge / inset as f32).max(0.0);
        }
        color
    }

    pub(crate) fn triangles(&self, viewport: Viewport) -> ([Vertex; 24], usize) {
        let width = viewport.width as f32;
        let height = viewport.height as f32;
        let inset = self.inset.unwrap_or(0) as f32;
        let points = [
            [0.0, 0.0],
            [width, 0.0],
            [width, height],
            [0.0, height],
            [inset, inset],
            [width - inset, inset],
            [width - inset, height - inset],
            [inset, height - inset],
        ];
        let indices: &[usize] = if self.inset.is_some() {
            &[
                0, 5, 4, 0, 1, 5, 1, 6, 5, 1, 2, 6, 6, 2, 3, 6, 3, 7, 0, 7, 3, 0, 4, 7,
            ]
        } else {
            &[0, 1, 2, 0, 2, 3]
        };
        let mut vertices = [Vertex::default(); 24];
        for (to, &index) in vertices.iter_mut().zip(indices) {
            let [x, y] = points[index];
            *to = Vertex {
                position: Vec3([viewport.x as f32 + x, viewport.y as f32 + y, 0.0]),
                color: if index < 4 {
                    self.color
                } else {
                    [self.color[0], self.color[1], self.color[2], 0]
                },
                ..Vertex::default()
            };
        }
        (vertices, indices.len())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ModelPart {
    pub model: ModelId,
    pub shader: ShaderRef,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneModel {
    Mesh(ModelId),
    Lods {
        choices: Span,
        radius: Option<f32>,
    },
    World {
        world: WorldId,
        presentation: WorldPresentationId,
        surfaces: Span,
    },
    Sprite {
        radius: f32,
        rotation: f32,
    },
    Beam {
        end: Vec3,
        radius: f32,
        /// World beams ignore entity axes; local beams use the entity pose.
        world_space: bool,
    },
    RailCore {
        end: Vec3,
        width: i32,
    },
    RailRings {
        end: Vec3,
        width: i32,
        segment_length: f32,
    },
    Lightning {
        end: Vec3,
    },
    Portal {
        camera: Vec3,
        rotation: i32,
    },
    PolyMarker,
}
impl From<ModelId> for SceneModel {
    fn from(model: ModelId) -> Self {
        Self::Mesh(model)
    }
}
impl SceneModel {
    pub fn mesh(self) -> Option<ModelId> {
        match self {
            Self::Mesh(model) => Some(model),
            Self::Lods { .. }
            | Self::World { .. }
            | Self::Sprite { .. }
            | Self::Beam { .. }
            | Self::RailCore { .. }
            | Self::RailRings { .. }
            | Self::Lightning { .. }
            | Self::Portal { .. }
            | Self::PolyMarker => None,
        }
    }
}

/// Opacity applied once after the material's complete ordered stage sequence.
/// Keeping it separate from shader alpha preserves destination-dependent stages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialOpacity(f32);
impl MaterialOpacity {
    pub fn new(value: f32) -> Option<Self> {
        (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(Self(value))
    }
    pub fn value(self) -> f32 {
        self.0
    }
    pub(crate) fn composite(self, original: u32, completed: u32) -> u32 {
        if original == completed || self.0 == 0.0 {
            return original;
        }
        if self.0 == 1.0 {
            return completed;
        }
        crate::stage::pack_unit_rgba(qa_core::math::lerp_array(
            crate::stage::unit_rgba(original.to_le_bytes()),
            crate::stage::unit_rgba(completed.to_le_bytes()),
            self.0,
        ))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SceneEntity {
    pub model: SceneModel,
    pub origin: Vec3,
    /// Native alias origin motion, separate from frame-major vertex blending.
    pub previous_origin: Option<Vec3>,
    pub axes: [Vec3; 3],
    pub non_normalized_axes: bool,
    pub color: [u8; 4],
    pub alpha: Option<f32>,
    /// A translucent material transaction retains submission order and scene
    /// depth even at opacity one. None keeps the ordinary opaque/shader path.
    pub material_opacity: Option<MaterialOpacity>,
    pub palette_color: Option<(PaletteId, u8)>,
    pub shader: Option<ShaderRef>,
    pub frame: u32,
    pub old_frame: u32,
    pub back_lerp: f32,
    pub depth_hack: bool,
    pub shader_time: f32,
    /// Native model synchronization, independent of shader animation time.
    pub sync_base: f32,
    pub shader_texcoord: [f32; 2],
    pub lighting: Option<crate::stage::EntityLighting>,
    pub lighting_origin: Option<Vec3>,
    pub minimum_light: bool,
    pub infrared_visible: bool,
    pub shadow_plane: Option<f32>,
    pub casts_shadow: bool,
    pub visibility: EntityVisibility,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntityVisibility {
    #[default]
    Always,
    Direct,
    Portal,
    Hidden,
}
impl EntityVisibility {
    pub fn visible(self, portal: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Direct => !portal,
            Self::Portal => portal,
            Self::Hidden => false,
        }
    }
}
impl SceneEntity {
    pub fn light_origin(&self) -> Vec3 {
        self.lighting_origin.unwrap_or(self.origin)
    }
    pub fn shader(&self, assets: &Assets) -> Option<ShaderRef> {
        self.shader
            .or_else(|| Some(assets.model(self.model.mesh()?)?.shader.into()))
    }
}
impl Default for SceneEntity {
    fn default() -> Self {
        Self {
            model: SceneModel::Mesh(ModelId(0)),
            origin: Vec3::default(),
            previous_origin: None,
            axes: Orientation::default().axes,
            non_normalized_axes: false,
            color: [255; 4],
            alpha: None,
            material_opacity: None,
            palette_color: None,
            shader: None,
            frame: 0,
            old_frame: 0,
            back_lerp: 0.0,
            depth_hack: false,
            shader_time: 0.0,
            sync_base: 0.0,
            shader_texcoord: [0.0; 2],
            lighting: None,
            lighting_origin: None,
            minimum_light: false,
            infrared_visible: false,
            shadow_plane: None,
            casts_shadow: false,
            visibility: EntityVisibility::Always,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Light {
    pub origin: Vec3,
    pub radius: f32,
    pub color: [f32; 3],
    pub additive: bool,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Poly {
    pub shader: ShaderRef,
    pub vertices: Span,
    pub entity: Option<u32>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Draw2d {
    pub rect: [f32; 4],
    pub texcoords: [f32; 4],
    pub color: [u8; 4],
    pub shader: ShaderRef,
}
impl Draw2d {
    /// Preserve UV interpolation while confining a module picture to its seat.
    /// Negative dimensions retain their native texture reversal.
    pub fn clipped(mut self, viewport: Viewport) -> Option<Self> {
        if !self
            .rect
            .iter()
            .chain(self.texcoords.iter())
            .all(|value| value.is_finite())
        {
            return None;
        }
        for axis in 0..2 {
            let start = self.rect[axis];
            let end = start + self.rect[axis + 2];
            let limit = if axis == 0 {
                (viewport.x, viewport.width)
            } else {
                (viewport.y, viewport.height)
            };
            let low = start.min(end).max(limit.0 as f32);
            let high = start.max(end).min((limit.0 + limit.1) as f32);
            if low >= high || !end.is_finite() {
                return None;
            }
            let uv_start = self.texcoords[axis];
            let uv_end = self.texcoords[axis + 2];
            self.texcoords[axis] =
                qa_core::math::lerp(uv_start, uv_end, (low - start) / (end - start));
            self.texcoords[axis + 2] =
                qa_core::math::lerp(uv_start, uv_end, (high - start) / (end - start));
            self.rect[axis] = low;
            self.rect[axis + 2] = high - low;
        }
        Some(self)
    }
}
/// Backend appearance data, independent of the effect's source recipe.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParticlePresentation {
    /// Native software point footprint; GL uses the original facing triangle.
    Square { pixel_aspect: f32 },
    /// A material-backed, camera-facing square with the requested world size.
    Card,
}
impl ParticlePresentation {
    pub fn for_target(self, width: u32, height: u32) -> Self {
        match self {
            Self::Square { .. } => Self::Square {
                pixel_aspect: (height as f32 / width as f32) * (320.0 / 240.0),
            },
            Self::Card => Self::Card,
        }
    }
}
impl Default for ParticlePresentation {
    fn default() -> Self {
        Self::Square { pixel_aspect: 1.0 }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneParticle {
    pub position: Vec3,
    pub color: u8,
    pub alpha: u8,
    pub palette: PaletteId,
    pub shader: ShaderRef,
    pub size: f32,
    pub presentation: ParticlePresentation,
}
impl Default for SceneParticle {
    fn default() -> Self {
        Self {
            position: Vec3::default(),
            color: 0,
            alpha: 255,
            palette: PaletteId(0),
            shader: ShaderBindingId(0).into(),
            size: 1.0,
            presentation: ParticlePresentation::default(),
        }
    }
}
impl SceneParticle {
    fn is_valid(self) -> bool {
        self.position.is_finite()
            && self.size.is_finite()
            && self.size > 0.0
            && match self.presentation {
                ParticlePresentation::Square { pixel_aspect } => {
                    pixel_aspect.is_finite() && pixel_aspect > 0.0
                }
                ParticlePresentation::Card => true,
            }
    }
}
pub use qa_core::primitives::IndexSpan as Span;
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneRanges {
    pub entities: Span,
    pub polys: Span,
    pub lights: Span,
    pub particles: Span,
    pub surfaces: Span,
    pub draws: Span,
}
impl SceneRanges {
    fn through(self, end: Self) -> Self {
        let span = |begin: Span, end: Span| Span {
            first: begin.first,
            count: end.first - begin.first,
        };
        Self {
            entities: span(self.entities, end.entities),
            polys: span(self.polys, end.polys),
            lights: span(self.lights, end.lights),
            particles: span(self.particles, end.particles),
            surfaces: span(self.surfaces, end.surfaces),
            draws: span(self.draws, end.draws),
        }
    }
}

/// Source triangles in world coordinates, before view-specific deformation.
/// Sky clipping and portal visibility share this one numeric traversal.
#[expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "Consume complete native triangle triples"
)]
pub(crate) fn for_each_triangle(
    list: &CommandList,
    item: DrawItem,
    assets: &Assets,
    mut visit: impl FnMut([Vertex; 3]),
) -> bool {
    let transform = |mut vertex: Vertex, entity: Option<&SceneEntity>| {
        if let Some(entity) = entity {
            vertex.position =
                qa_core::math::transform_point(entity.origin, entity.axes, vertex.position);
            vertex.normal =
                qa_core::math::transform_point(Vec3::default(), entity.axes, vertex.normal);
        }
        vertex
    };
    match item.kind {
        DrawKind::Surface => {
            let reference = list.surface(item.index);
            let Some(world) = assets.world(reference.world) else {
                return false;
            };
            let geometry = world.geometry();
            let Some(surface) = geometry.surfaces.get(reference.surface as usize) else {
                return false;
            };
            let entity = reference.entity.map(|index| list.entity(index));
            let mut emit = |triangle: [u32; 3]| {
                visit(
                    triangle
                        .map(|index| transform(geometry.vertices[index as usize].vertex, entity)),
                );
            };
            if let Some(patch) = &surface.patch
                && !reference.patch_lod.is_full(patch)
            {
                for triangle in reference.patch_lod.triangles(patch, surface.vertices.first) {
                    emit(triangle);
                }
            } else {
                for triangle in geometry.indices[surface.indices.range()].chunks_exact(3) {
                    emit([triangle[0], triangle[1], triangle[2]]);
                }
            }
        }
        DrawKind::Entity => {
            let Some(entity) = draw_entity(list, item) else {
                return false;
            };
            let Some(model) = entity.model.mesh().and_then(|id| assets.model(id)) else {
                return false;
            };
            let Some(frames) = model.frames(entity.frame, entity.old_frame, entity.back_lerp)
            else {
                return false;
            };
            for triangle in model.indices.chunks_exact(3) {
                visit(
                    [triangle[0], triangle[1], triangle[2]]
                        .map(|index| transform(frames.vertex(index as usize), Some(&entity))),
                );
            }
        }
        DrawKind::Poly => {
            let vertices = list.vertices(list.poly(item.index).vertices);
            for triangle in crate::affine::fan(vertices.len()) {
                visit(triangle.map(|index| vertices[index]));
            }
        }
    }
    true
}
#[derive(Clone, Copy, Debug, Default)]
pub struct SurfaceRef {
    pub world: WorldId,
    pub presentation: WorldPresentationId,
    pub surface: u32,
    pub depth_key: u32,
    /// View-local position in the one sorted draw list. Exact-depth ties use
    /// the later draw, matching the shared LEQUAL stage order.
    pub draw_rank: u32,
    /// Absolute draw selection captured for this surface in its submitted view.
    pub draw_index: Option<u32>,
    /// Absolute index in this packet's one copied entity arena. Static world
    /// visibility keeps None; inline geometry borrows its instance inputs.
    pub entity: Option<u32>,
    pub patch_lod: PatchSelection,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawKind {
    #[default]
    Surface,
    Entity,
    Poly,
}
/// Absolute payload index in this packet. Sorting moves this record alone,
/// leaving visibility/depth order and every earlier view's payload intact.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrawItem {
    pub kind: DrawKind,
    pub index: u32,
    pub shader: Option<ShaderRef>,
    pub mesh: Option<ModelId>,
    /// Flattened current/previous poses selected for this copied view.
    pub frames: Option<[u32; 2]>,
    key: DrawKey,
}
#[derive(Clone, Copy, Debug, Default)]
struct DrawKey {
    sort: f32,
    material: u64,
    instance: u32,
    lightmap: u32,
}
impl DrawKey {
    fn compare(self, other: Self) -> SortOrdering {
        // Q3 SortNewShader preserves fractional shader sort and registration
        // order. Its draw key then groups entity instance and lighting data.
        // Signed zero sorts equally, as in the native float comparisons.
        let sort = if self.sort == other.sort {
            SortOrdering::Equal
        } else {
            self.sort.total_cmp(&other.sort)
        };
        sort.then_with(|| self.material.cmp(&other.material))
            .then_with(|| self.instance.cmp(&other.instance))
            .then_with(|| self.lightmap.cmp(&other.lightmap))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub refdef: Refdef,
    pub scene: SceneRanges,
    pub hidden_areas: Span,
    pub particle_client: Option<crate::effects::EffectClient>,
}
#[derive(Clone, Copy, Debug, Default)]
#[expect(
    clippy::large_enum_variant,
    reason = "Load-sized command slots own copied refdefs inline; boxing view commands would allocate during frame submission."
)]
pub enum Command {
    #[default]
    Empty,
    Clear([u8; 4]),
    View(View),
    Draw2d(Draw2d),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub commands: usize,
    pub entities: usize,
    /// Zero disables multi-level model choices; ordinary meshes remain available.
    pub lod_choices: usize,
    pub polys: usize,
    pub vertices: usize,
    pub lights: usize,
    pub particles: usize,
    pub area_bytes: usize,
    pub surfaces: usize,
    /// Zero derives the sum of payload capacities at load time.
    pub draws: usize,
}
impl Limits {
    /// Resolve the load-time draw bound shared by front-end and consumers.
    pub fn draw_capacity(self) -> Option<usize> {
        if self.draws != 0 {
            Some(self.draws)
        } else {
            self.surfaces
                .checked_add(self.entities)?
                .checked_add(self.polys)
        }
    }

    fn required_capacities(self) -> [usize; 9] {
        [
            self.commands,
            self.entities,
            self.polys,
            self.vertices,
            self.lights,
            self.particles,
            self.area_bytes,
            self.surfaces,
            self.draws,
        ]
    }

    fn capacities(self) -> impl Iterator<Item = usize> {
        self.required_capacities()
            .into_iter()
            .chain(std::iter::once(self.lod_choices))
    }

    pub(crate) fn resolved(mut self) -> Result<Self, &'static str> {
        self.draws = self.draw_capacity().ok_or("scene draw capacity overflow")?;
        if self
            .required_capacities()
            .iter()
            .any(|&n| n == 0 || n > u32::MAX as usize)
            || self.lod_choices > u32::MAX as usize
        {
            return Err("invalid scene capacity");
        }
        Ok(self)
    }

    pub(crate) fn validate_growth(self, current: Self) -> Result<Self, &'static str> {
        let next = self.resolved()?;
        let current = current.resolved()?;
        if next
            .capacities()
            .zip(current.capacities())
            .any(|(next, current)| next < current)
        {
            return Err("scene capacity cannot shrink");
        }
        Ok(next)
    }
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            commands: 4096,
            entities: 2048,
            lod_choices: 6144,
            polys: 8192,
            vertices: 65536,
            lights: 256,
            particles: 2048,
            area_bytes: 8192,
            surfaces: 65536,
            draws: 0,
        }
    }
}
/// Backends borrow a sealed packet. Client-owned pointers never cross this boundary.
pub struct CommandList {
    commands: Box<[Command]>,
    entities: Box<[SceneEntity]>,
    lod_choices: Box<[Option<ModelPart>]>,
    polys: Box<[Poly]>,
    vertices: Box<[Vertex]>,
    lights: Box<[Light]>,
    particles: Box<[SceneParticle]>,
    area_bytes: Box<[u8]>,
    surfaces: Box<[SurfaceRef]>,
    draws: Box<[DrawItem]>,
    command_count: usize,
    entity_count: usize,
    lod_count: usize,
    poly_count: usize,
    vertex_count: usize,
    light_count: usize,
    particle_count: usize,
    area_count: usize,
    surface_count: usize,
    draw_count: usize,
    owner: u64,
    slot: usize,
    pub frame: u64,
    pub rejected: u64,
}
impl CommandList {
    fn load(l: Limits, owner: u64, slot: usize) -> Self {
        Self {
            commands: vec![Command::Empty; l.commands].into_boxed_slice(),
            entities: vec![SceneEntity::default(); l.entities].into_boxed_slice(),
            lod_choices: vec![None; l.lod_choices].into_boxed_slice(),
            polys: vec![Poly::default(); l.polys].into_boxed_slice(),
            vertices: vec![Vertex::default(); l.vertices].into_boxed_slice(),
            lights: vec![Light::default(); l.lights].into_boxed_slice(),
            particles: vec![SceneParticle::default(); l.particles].into_boxed_slice(),
            area_bytes: vec![0; l.area_bytes].into_boxed_slice(),
            surfaces: vec![SurfaceRef::default(); l.surfaces].into_boxed_slice(),
            draws: vec![DrawItem::default(); l.draws].into_boxed_slice(),
            command_count: 0,
            entity_count: 0,
            lod_count: 0,
            poly_count: 0,
            vertex_count: 0,
            light_count: 0,
            particle_count: 0,
            area_count: 0,
            surface_count: 0,
            draw_count: 0,
            owner,
            slot,
            frame: 0,
            rejected: 0,
        }
    }
    pub fn commands(&self) -> &[Command] {
        &self.commands[..self.command_count]
    }
    pub fn entities(&self, range: Span) -> &[SceneEntity] {
        &self.entities[range.range()]
    }
    pub fn polys(&self, range: Span) -> &[Poly] {
        &self.polys[range.range()]
    }
    pub fn vertices(&self, range: Span) -> &[Vertex] {
        &self.vertices[range.range()]
    }
    pub fn lights(&self, range: Span) -> &[Light] {
        &self.lights[range.range()]
    }
    pub fn particles(&self, range: Span) -> &[SceneParticle] {
        &self.particles[range.range()]
    }
    pub fn hidden_areas(&self, range: Span) -> &[u8] {
        &self.area_bytes[range.range()]
    }
    pub fn surfaces(&self, range: Span) -> &[SurfaceRef] {
        &self.surfaces[range.range()]
    }
    pub fn draws(&self, range: Span) -> &[DrawItem] {
        &self.draws[range.range()]
    }
    pub fn surface(&self, index: u32) -> &SurfaceRef {
        &self.surfaces[index as usize]
    }
    pub fn entity(&self, index: u32) -> &SceneEntity {
        &self.entities[index as usize]
    }
    pub fn poly(&self, index: u32) -> &Poly {
        &self.polys[index as usize]
    }
    pub fn surface_draw(&self, surface: &SurfaceRef) -> Option<DrawItem> {
        let draw = *self.draws.get(surface.draw_index? as usize)?;
        (draw.kind == DrawKind::Surface
            && self
                .surfaces
                .get(draw.index as usize)
                .is_some_and(|selected| std::ptr::eq(selected, surface)))
        .then_some(draw)
    }
    fn resolve_draw_shaders(&mut self, assets: &Assets, native: &dyn NativeShaders) {
        for draw in &mut self.draws[..self.draw_count] {
            draw.shader = draw
                .shader
                .and_then(|shader| shader.resolve(assets, native))
                .map(ShaderRef::Resolved);
        }
        for command in &mut self.commands[..self.command_count] {
            if let Command::Draw2d(draw) = command {
                if let Some(shader) = draw.shader.resolve(assets, native) {
                    draw.shader = ShaderRef::Resolved(shader);
                } else {
                    *command = Command::Empty;
                    self.rejected += 1;
                }
            }
        }
    }
    /// Resolve the captured choices after every producer has finished this packet.
    pub fn resolve_shaders(&mut self, assets: &Assets, native: &dyn NativeShaders) {
        self.resolve_draw_shaders(assets, native);
        // Keep each view's particle range stable while retiring failed choices
        // before either the procedural or material-backed consumer can draw.
        for particle in &mut self.particles[..self.particle_count] {
            if particle.alpha == 0 {
                continue;
            }
            if let Some(shader) = particle.shader.resolve(assets, native) {
                particle.shader = ShaderRef::Resolved(shader);
            } else {
                particle.alpha = 0;
                self.rejected += 1;
            }
        }
    }
    fn reset(&mut self, frame: u64) {
        self.command_count = 0;
        self.entity_count = 0;
        self.lod_count = 0;
        self.poly_count = 0;
        self.vertex_count = 0;
        self.light_count = 0;
        self.particle_count = 0;
        self.area_count = 0;
        self.surface_count = 0;
        self.draw_count = 0;
        self.rejected = 0;
        self.frame = frame;
    }
}
pub struct FrontEnd {
    lists: [Option<CommandList>; 2],
    limits: Limits,
    automatic_draws: bool,
    next: usize,
    frame: u64,
    owner: u64,
}

/// A cold candidate borrows the existing owner until committed or discarded.
/// No live frame or outstanding consumer can coexist with this borrow.
pub struct FrontEndCapacity<'a> {
    frontend: &'a mut FrontEnd,
    lists: Option<[Option<CommandList>; 2]>,
    limits: Limits,
}
impl FrontEndCapacity<'_> {
    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn commit(self) {
        if let Some(lists) = self.lists {
            self.frontend.lists = lists;
        }
        self.frontend.limits = self.limits;
    }
}
pub struct Frame {
    list: CommandList,
    first: SceneRanges,
    particle_client: Option<crate::effects::EffectClient>,
}
#[derive(Clone, Copy)]
struct SceneCheckpoint {
    first: SceneRanges,
    end: SceneRanges,
    vertices: usize,
    commands: usize,
    areas: usize,
    lod_choices: usize,
}
impl FrontEnd {
    pub fn load(limits: Limits) -> Result<Self, &'static str> {
        let automatic_draws = limits.draws == 0;
        let limits = limits.resolved()?;
        let owner = crate::next_owner_id().map_err(|_| "scene owner ids exhausted")?;
        Ok(Self {
            lists: [
                Some(CommandList::load(limits, owner, 0)),
                Some(CommandList::load(limits, owner, 1)),
            ],
            limits,
            automatic_draws,
            next: 0,
            frame: 0,
            owner,
        })
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Reserve larger arenas only after both existing lists have returned.
    /// Dropping the candidate leaves capacities and sequence ownership intact.
    pub fn stage_capacity(
        &mut self,
        required: Limits,
    ) -> Result<FrontEndCapacity<'_>, &'static str> {
        if self.lists.iter().any(Option::is_none) {
            return Err("scene command lists are outstanding");
        }
        let current = self.limits;
        let mut limits = Limits {
            commands: current.commands.max(required.commands),
            entities: current.entities.max(required.entities),
            lod_choices: current.lod_choices.max(required.lod_choices),
            polys: current.polys.max(required.polys),
            vertices: current.vertices.max(required.vertices),
            lights: current.lights.max(required.lights),
            particles: current.particles.max(required.particles),
            area_bytes: current.area_bytes.max(required.area_bytes),
            surfaces: current.surfaces.max(required.surfaces),
            draws: current.draws.max(required.draws),
        };
        if self.automatic_draws {
            limits.draws = limits.draws.max(
                Limits { draws: 0, ..limits }
                    .draw_capacity()
                    .ok_or("scene draw capacity overflow")?,
            );
        }
        let limits = limits.validate_growth(current)?;
        let lists = limits.capacities().ne(current.capacities()).then(|| {
            [
                Some(CommandList::load(limits, self.owner, 0)),
                Some(CommandList::load(limits, self.owner, 1)),
            ]
        });
        Ok(FrontEndCapacity {
            frontend: self,
            lists,
            limits,
        })
    }
    /// A packet leaves the pool until its consumer returns ownership. Two
    /// outstanding packets exhaust the pool rather than allocating or waiting.
    pub fn begin_frame(&mut self, clear: [u8; 4]) -> Option<Frame> {
        let index = self.next;
        let mut list = self.lists[index].take()?;
        self.next ^= 1;
        self.frame = self.frame.wrapping_add(1);
        list.reset(self.frame);
        let mut frame = Frame {
            list,
            first: SceneRanges::default(),
            particle_client: None,
        };
        frame.command(Command::Clear(clear));
        Some(frame)
    }
    #[expect(
        clippy::result_large_err,
        reason = "Rejected recycling returns ownership of the packet's existing arenas without boxing or cloning during a frame."
    )]
    pub fn recycle(&mut self, packet: CommandList) -> Result<(), CommandList> {
        if packet.owner != self.owner || self.lists[packet.slot].is_some() {
            return Err(packet);
        }
        let slot = packet.slot;
        self.lists[slot] = Some(packet);
        Ok(())
    }
}
impl Frame {
    fn checkpoint(&self) -> SceneCheckpoint {
        SceneCheckpoint {
            first: self.first,
            end: self.current(),
            vertices: self.list.vertex_count,
            commands: self.list.command_count,
            areas: self.list.area_count,
            lod_choices: self.list.lod_count,
        }
    }
    fn restore(&mut self, checkpoint: SceneCheckpoint) {
        self.first = checkpoint.first;
        self.list.entity_count = checkpoint.end.entities.first as usize;
        self.list.poly_count = checkpoint.end.polys.first as usize;
        self.list.light_count = checkpoint.end.lights.first as usize;
        self.list.particle_count = checkpoint.end.particles.first as usize;
        self.list.surface_count = checkpoint.end.surfaces.first as usize;
        self.list.draw_count = checkpoint.end.draws.first as usize;
        self.list.vertex_count = checkpoint.vertices;
        self.list.command_count = checkpoint.commands;
        self.list.area_count = checkpoint.areas;
        self.list.lod_count = checkpoint.lod_choices;
    }
    fn copy_scene(&mut self, source: SceneRanges, skip_world: Option<WorldId>) -> bool {
        if source.entities.count as usize > self.list.entities.len() - self.list.entity_count
            || source.polys.count as usize > self.list.polys.len() - self.list.poly_count
            || source.lights.count as usize > self.list.lights.len() - self.list.light_count
            || source.particles.count as usize
                > self.list.particles.len() - self.list.particle_count
            || source.surfaces.count as usize > self.list.surfaces.len() - self.list.surface_count
            || source.draws.count as usize > self.list.draws.len() - self.list.draw_count
        {
            return self.reject_submission();
        }
        self.clear_scene();
        let entity_delta = self.list.entity_count as u32 - source.entities.first;
        self.list
            .entities
            .copy_within(source.entities.range(), self.list.entity_count);
        self.list.entity_count += source.entities.count as usize;
        self.list
            .lights
            .copy_within(source.lights.range(), self.list.light_count);
        self.list.light_count += source.lights.count as usize;
        self.list
            .particles
            .copy_within(source.particles.range(), self.list.particle_count);
        self.list.particle_count += source.particles.count as usize;
        for index in source.draws.range() {
            let mut draw = self.list.draws[index];
            match draw.kind {
                DrawKind::Entity => draw.index += entity_delta,
                DrawKind::Poly => {
                    let mut poly = self.list.polys[draw.index as usize];
                    poly.entity = poly.entity.map(|index| index + entity_delta);
                    draw.index = self.list.poly_count as u32;
                    self.list.polys[self.list.poly_count] = poly;
                    self.list.poly_count += 1;
                }
                DrawKind::Surface => {
                    let mut surface = self.list.surfaces[draw.index as usize];
                    if skip_world == Some(surface.world) && surface.entity.is_none() {
                        continue;
                    }
                    surface.entity = surface.entity.map(|index| index + entity_delta);
                    draw.index = self.list.surface_count as u32;
                    self.list.surfaces[self.list.surface_count] = surface;
                    self.list.surface_count += 1;
                }
            }
            self.list.draws[self.list.draw_count] = draw;
            self.list.draw_count += 1;
        }
        true
    }
    fn generated_fits(&self, refdef: Refdef, assets: &Assets) -> bool {
        let projection = refdef.projection_scales();
        let generated = self.list.draws[self.first.draws.first as usize..self.list.draw_count]
            .iter()
            .filter(|&&draw| {
                draw.kind == DrawKind::Entity && draw_allowed(&self.list, draw, refdef.policy)
            })
            .filter_map(|&draw| {
                model_draw(&self.list, assets, draw, &refdef, projection)
                    .ok()
                    .flatten()
            })
            .filter_map(|draw| draw_entity(&self.list, draw))
            .filter(|entity| entity.visibility.visible(refdef.portal))
            .filter_map(|entity| crate::entities::prepare(&entity, refdef, assets))
            .try_fold(0usize, |total, geometry| {
                total.checked_add(geometry.triangle_count())
            });
        generated.is_some_and(|count| {
            count <= self.list.polys.len() - self.list.poly_count
                && count <= self.list.draws.len() - self.list.draw_count
                && count <= (self.list.vertices.len() - self.list.vertex_count) / 3
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub fn render_world_scene(
        &mut self,
        world: WorldId,
        presentation: WorldPresentationId,
        refdef: Refdef,
        hidden_areas: &[u8],
        scratch: &mut crate::world::WorldView,
        assets: &Assets,
        shaders: ShaderView<'_>,
    ) -> bool {
        let checkpoint = self.checkpoint();
        let source = checkpoint.first.through(checkpoint.end);
        self.select_patch_lod(source.surfaces, refdef, assets);
        let has_portal = !refdef.portal
            && !refdef.hyperspace
            && self
                .list
                .entities(source.entities)
                .iter()
                .any(|entity| matches!(entity.model, SceneModel::Portal { .. }));
        if has_portal {
            let projection = refdef.projection_scales();
            let first = self.list.draw_count;
            if source.draws.count as usize <= self.list.draws.len() - first {
                let mut count = 0;
                for index in source.draws.range() {
                    if !draw_allowed(&self.list, self.list.draws[index], refdef.policy) {
                        continue;
                    }
                    let Ok(Some(mut draw)) = model_draw(
                        &self.list,
                        assets,
                        self.list.draws[index],
                        &refdef,
                        projection,
                    ) else {
                        continue;
                    };
                    draw.shader = source_shader(&self.list, assets, draw, shaders);
                    if let Some(key) = draw_key(&self.list, assets, draw, shaders) {
                        draw.key = key;
                        self.list.draws[first + count] = draw;
                        count += 1;
                    }
                }
                native_draw_sort(&mut self.list.draws[first..first + count]);
                let sorted = SceneRanges {
                    draws: Span {
                        first: first as u32,
                        count: count as u32,
                    },
                    ..source
                };
                if let Some(child) =
                    crate::portal::first_child(&self.list, sorted, assets, refdef, shaders.native)
                {
                    let rejected = self.list.rejected;
                    let retained = self.copy_scene(source, Some(world))
                        && assets.world(world).is_some_and(|loaded| {
                            scratch
                                .query(loaded, child, None, hidden_areas)
                                .is_ok_and(|surfaces| self.add_world(world, presentation, surfaces))
                        })
                        && self.render_scene(child, hidden_areas, assets, shaders)
                        && self.copy_scene(source, None)
                        && self.generated_fits(refdef, assets)
                        && self.list.command_count < self.list.commands.len()
                        && hidden_areas.len() <= self.list.area_bytes.len() - self.list.area_count;
                    if !retained {
                        self.restore(checkpoint);
                        if self.list.rejected == rejected {
                            self.reject_submission();
                        }
                    }
                }
            } else {
                self.reject_submission();
            }
        }
        self.render_scene(refdef, hidden_areas, assets, shaders)
    }

    pub fn reject_submission(&mut self) -> bool {
        self.list.rejected += 1;
        false
    }
    /// Freeze submitted views and 2D draws at a native registration barrier.
    /// Unsubmitted scene inputs retain their original choices for later views.
    pub fn resolve_draw_shaders(&mut self, assets: &Assets, native: &dyn NativeShaders) {
        self.list.resolve_draw_shaders(assets, native);
    }
    /// Built-in seats are background content. Move their newly appended command
    /// range before module views/UI in this same packet; payload indices remain
    /// absolute and no second command list or replay queue is created.
    pub fn background(&mut self, compose: impl FnOnce(&mut Self)) {
        let first = self.list.command_count;
        compose(self);
        let added = self.list.command_count - first;
        if added != 0 {
            self.list.commands[1..self.list.command_count].rotate_right(added);
        }
    }
    fn command(&mut self, command: Command) -> bool {
        if self.list.command_count == self.list.commands.len() {
            self.list.rejected += 1;
            return false;
        }
        self.list.commands[self.list.command_count] = command;
        self.list.command_count += 1;
        true
    }
    fn current(&self) -> SceneRanges {
        SceneRanges {
            entities: Span {
                first: self.list.entity_count as u32,
                count: 0,
            },
            polys: Span {
                first: self.list.poly_count as u32,
                count: 0,
            },
            lights: Span {
                first: self.list.light_count as u32,
                count: 0,
            },
            particles: Span {
                first: self.list.particle_count as u32,
                count: 0,
            },
            surfaces: Span {
                first: self.list.surface_count as u32,
                count: 0,
            },
            draws: Span {
                first: self.list.draw_count as u32,
                count: 0,
            },
        }
    }
    pub fn clear_scene(&mut self) {
        self.first = self.current();
    }
    /// Recipient metadata also follows any portal views emitted by this call.
    pub fn for_client<T>(
        &mut self,
        client: Option<crate::effects::EffectClient>,
        compose: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.particle_client;
        self.particle_client = client;
        let result = compose(self);
        self.particle_client = previous;
        result
    }

    /// Complete recorded client views after output receipt, before backend
    /// submission. Particle simulation remains in the one effects store.
    /// Call after native scene submissions and before composing built-in views.
    pub fn append_client_effects(
        &mut self,
        effects: &mut crate::effects::Effects,
        assets: &Assets,
    ) {
        let open = self.first.particles.first as usize;
        let end = self.list.particle_count;
        for index in 0..self.list.command_count {
            let Command::View(mut view) = self.list.commands[index] else {
                continue;
            };
            let Some(client) = view.particle_client.take() else {
                continue;
            };
            self.list.commands[index] = Command::View(view);
            let first = self.list.particle_count;
            let existing = view.scene.particles;
            if existing.count as usize > self.list.particles.len() - first {
                self.reject_submission();
                continue;
            }
            self.list.particles.copy_within(existing.range(), first);
            self.list.particle_count += existing.count as usize;
            let rejected = self.list.rejected;
            effects.append(self, client.client, client.lifetime);
            self.validate_particles(first, assets);
            if self.list.rejected != rejected {
                self.list.particle_count = first;
            } else {
                view.scene.particles = Span {
                    first: first as u32,
                    count: (self.list.particle_count - first) as u32,
                };
                self.list.commands[index] = Command::View(view);
            }
        }
        // Keep an unsubmitted scene's particles at the tail. The in-place
        // rotation only moves newly copied view payloads, whose ranges follow.
        let pending = end - open;
        if pending != 0 {
            self.list.particles[open..self.list.particle_count].rotate_left(pending);
            for command in &mut self.list.commands[..self.list.command_count] {
                if let Command::View(view) = command
                    && view.scene.particles.first as usize >= end
                {
                    view.scene.particles.first -= pending as u32;
                }
            }
        }
        self.first.particles.first = (self.list.particle_count - pending) as u32;
    }

    fn validate_particles(&mut self, first: usize, assets: &Assets) {
        let mut valid = first;
        for index in first..self.list.particle_count {
            let particle = self.list.particles[index];
            if assets.palette(particle.palette).is_some()
                && match particle.shader {
                    ShaderRef::Native(_) | ShaderRef::Surface { .. } => true,
                    _ => particle.shader.resolve(assets, &()).is_some(),
                }
            {
                self.list.particles[valid] = particle;
                valid += 1;
            } else {
                self.list.rejected += 1;
            }
        }
        self.list.particle_count = valid;
    }
    /// Resolve only this view's models, once. Explicit sampler results remain
    /// authoritative; all outputs stay in the existing entity arena.
    pub fn light_entities(
        &mut self,
        identity_light: f32,
        mut sample: impl FnMut(&SceneEntity) -> crate::stage::EntityLighting,
    ) {
        let entities =
            &mut self.list.entities[self.first.entities.first as usize..self.list.entity_count];
        let lights = &self.list.lights[self.first.lights.first as usize..self.list.light_count];
        for entity in entities {
            if entity.lighting.is_none()
                && matches!(
                    entity.model,
                    SceneModel::Mesh(_) | SceneModel::Lods { .. } | SceneModel::World { .. }
                )
            {
                entity.lighting = Some(crate::lighting::setup_entity(
                    entity,
                    sample(entity),
                    identity_light,
                    lights,
                ));
            }
        }
    }
    pub fn add_entity(&mut self, entity: SceneEntity) -> bool {
        self.add_entities(std::iter::once(entity))
    }
    /// Capture every part's independent levels into this packet, or leave all
    /// payload counts unchanged. Copied views share these immutable choices.
    pub fn add_model_lods(
        &mut self,
        pose: SceneEntity,
        radius: Option<f32>,
        parts: impl IntoIterator<Item = impl IntoIterator<Item = Option<ModelPart>>>,
    ) -> bool {
        let checkpoint = self.checkpoint();
        for levels in parts {
            let mut levels = levels.into_iter();
            let Some(first_level) = levels.next() else {
                self.restore(checkpoint);
                return self.reject_submission();
            };
            let Some(second_level) = levels.next() else {
                if let Some(part) = first_level {
                    let mut entity = pose;
                    entity.model = SceneModel::Mesh(part.model);
                    entity.shader = pose.shader.or(Some(part.shader));
                    if !self.add_entity_part(entity) {
                        self.restore(checkpoint);
                        return false;
                    }
                }
                continue;
            };
            let first = self.list.lod_count;
            for level in [first_level, second_level].into_iter().chain(levels) {
                if self.list.lod_count == self.list.lod_choices.len() {
                    self.restore(checkpoint);
                    return self.reject_submission();
                }
                self.list.lod_choices[self.list.lod_count] = level;
                self.list.lod_count += 1;
            }
            let choices = Span {
                first: first as u32,
                count: (self.list.lod_count - first) as u32,
            };
            let mut entity = pose;
            entity.model = SceneModel::Lods { choices, radius };
            if !self.add_entity_part(entity) {
                self.restore(checkpoint);
                return false;
            }
        }
        true
    }
    /// One submitted model may contain several surfaces. Admit that complete
    /// group or restore all counts, without copying/growing packet storage.
    pub fn add_entities(&mut self, entities: impl IntoIterator<Item = SceneEntity>) -> bool {
        let checkpoint = self.checkpoint();
        for entity in entities {
            if !self.add_entity_part(entity) {
                self.restore(checkpoint);
                return false;
            }
        }
        true
    }
    fn add_entity_part(&mut self, entity: SceneEntity) -> bool {
        if self.list.entity_count == self.list.entities.len() {
            self.list.rejected += 1;
            return false;
        }
        if let SceneModel::Lods { choices, .. } = entity.model
            && (choices.count == 0 || !choices.fits(self.list.lod_count))
        {
            return self.reject_submission();
        }
        match entity.model {
            SceneModel::Mesh(_)
            | SceneModel::Lods { .. }
            | SceneModel::Sprite { .. }
            | SceneModel::Beam { .. }
            | SceneModel::RailCore { .. }
            | SceneModel::RailRings { .. }
            | SceneModel::Lightning { .. } => {
                if self.list.draw_count == self.list.draws.len() {
                    self.list.rejected += 1;
                    return false;
                }
                self.draw(DrawKind::Entity, self.list.entity_count as u32);
            }
            SceneModel::World {
                world,
                presentation,
                surfaces,
            } => {
                if surfaces.first.checked_add(surfaces.count).is_none() {
                    self.list.rejected += 1;
                    return false;
                }
                let index = self.list.entity_count as u32;
                if !self.add_surfaces(surfaces.range().map(|surface| SurfaceRef {
                    world,
                    presentation,
                    surface: surface as u32,
                    entity: Some(index),
                    ..SurfaceRef::default()
                })) {
                    return false;
                }
            }
            SceneModel::Portal { .. } | SceneModel::PolyMarker => {}
        }
        self.list.entities[self.list.entity_count] = entity;
        self.list.entity_count += 1;
        true
    }
    pub fn add_poly(&mut self, shader: ShaderRef, vertices: &[Vertex]) -> bool {
        self.add_poly_from(shader, vertices.len(), &mut |index| vertices[index])
    }
    /// Copy directly into the packet after reserving all polygon storage. This
    /// admits checked module vertices without a temporary allocation.
    pub fn add_poly_from(
        &mut self,
        shader: ShaderRef,
        count: usize,
        vertex: &mut dyn FnMut(usize) -> Vertex,
    ) -> bool {
        if count < 3
            || self.list.poly_count == self.list.polys.len()
            || self.list.draw_count == self.list.draws.len()
            || count > self.list.vertices.len() - self.list.vertex_count
        {
            self.list.rejected += 1;
            return false;
        }
        let first = self.list.vertex_count;
        for (index, target) in self.list.vertices[first..first + count]
            .iter_mut()
            .enumerate()
        {
            *target = vertex(index);
        }
        self.list.vertex_count += count;
        self.list.polys[self.list.poly_count] = Poly {
            shader,
            vertices: Span {
                first: first as u32,
                count: count as u32,
            },
            entity: None,
        };
        self.draw(DrawKind::Poly, self.list.poly_count as u32);
        self.list.poly_count += 1;
        true
    }
    pub fn add_light(&mut self, light: Light) -> bool {
        if self.list.light_count == self.list.lights.len() {
            self.list.rejected += 1;
            return false;
        }
        self.list.lights[self.list.light_count] = light;
        self.list.light_count += 1;
        true
    }
    pub fn add_particle(&mut self, particle: SceneParticle) -> bool {
        if self.list.particle_count == self.list.particles.len() || !particle.is_valid() {
            self.list.rejected += 1;
            return false;
        }
        self.list.particles[self.list.particle_count] = particle;
        self.list.particle_count += 1;
        true
    }
    /// Copy the one frontend visibility result into this owned packet. Both
    /// consumers receive these ids and the CPU's original BSP depth keys.
    pub fn add_world(
        &mut self,
        world: WorldId,
        presentation: WorldPresentationId,
        surfaces: &[VisibleSurface],
    ) -> bool {
        self.add_surfaces(surfaces.iter().map(|source| SurfaceRef {
            world,
            presentation,
            surface: source.surface,
            depth_key: source.depth_key,
            ..SurfaceRef::default()
        }))
    }
    fn add_surfaces(&mut self, surfaces: impl ExactSizeIterator<Item = SurfaceRef>) -> bool {
        let count = surfaces.len();
        if count > self.list.surfaces.len() - self.list.surface_count
            || count > self.list.draws.len() - self.list.draw_count
        {
            self.list.rejected += 1;
            return false;
        }
        let first = self.list.surface_count;
        for (target, source) in self.list.surfaces[first..first + count]
            .iter_mut()
            .zip(surfaces)
        {
            *target = source;
        }
        for index in first..first + count {
            self.draw(DrawKind::Surface, index as u32);
        }
        self.list.surface_count += count;
        true
    }
    fn draw(&mut self, kind: DrawKind, index: u32) {
        self.list.draws[self.list.draw_count] = DrawItem {
            kind,
            index,
            key: DrawKey::default(),
            shader: None,
            mesh: None,
            frames: None,
        };
        self.list.draw_count += 1;
    }
    fn select_patch_lod(&mut self, range: Span, view: Refdef, assets: &Assets) {
        let entities = &self.list.entities;
        for reference in &mut self.list.surfaces[range.range()] {
            reference.patch_lod = PatchSelection::default();
            if let Some(surface) = assets
                .world(reference.world)
                .and_then(|world| world.geometry().surfaces.get(reference.surface as usize))
                && !surface.no_draw
                && surface.indices.count != 0
                && let Some(patch) = &surface.patch
            {
                let pose = reference.entity.map(|index| {
                    let entity = &entities[index as usize];
                    (entity.origin, entity.axes)
                });
                reference.patch_lod = PatchSelection::select(
                    patch,
                    pose,
                    view.origin,
                    view.axes[0],
                    view.curve_error,
                );
            }
        }
    }
    pub fn render_scene(
        &mut self,
        refdef: Refdef,
        hidden_areas: &[u8],
        assets: &Assets,
        shaders: ShaderView<'_>,
    ) -> bool {
        if self.list.command_count == self.list.commands.len()
            || hidden_areas.len() > self.list.area_bytes.len() - self.list.area_count
        {
            self.list.rejected += 1;
            return false;
        }
        self.select_patch_lod(
            Span {
                first: self.first.surfaces.first,
                count: self.list.surface_count as u32 - self.first.surfaces.first,
            },
            refdef,
            assets,
        );
        for surface in
            &mut self.list.surfaces[self.first.surfaces.first as usize..self.list.surface_count]
        {
            surface.draw_index = None;
        }
        let end = self.list.draw_count;
        let projection = refdef.projection_scales();
        let first_draw = self.first.draws.first as usize;
        let mut valid = first_draw;
        for index in first_draw..end {
            if !draw_allowed(&self.list, self.list.draws[index], refdef.policy) {
                continue;
            }
            let draw = match model_draw(
                &self.list,
                assets,
                self.list.draws[index],
                &refdef,
                projection,
            ) {
                Ok(Some(draw)) => draw,
                Ok(None) => continue,
                Err(()) => {
                    self.list.rejected += 1;
                    continue;
                }
            };
            if draw_entity(&self.list, draw)
                .is_some_and(|entity| !entity.visibility.visible(refdef.portal))
            {
                continue;
            }
            if draw.kind == DrawKind::Entity
                && let Some(entity) = draw_entity(&self.list, draw)
                && let Some(geometry) = crate::entities::prepare(&entity, refdef, assets)
            {
                let count = geometry.triangle_count();
                if count > self.list.polys.len() - self.list.poly_count
                    || count > self.list.draws.len() - self.list.draw_count
                    || count > (self.list.vertices.len() - self.list.vertex_count) / 3
                {
                    self.list.rejected += 1;
                    continue;
                }
                let Some(shader) = entity.shader(assets) else {
                    self.list.rejected += 1;
                    continue;
                };
                geometry.triangles(|vertices| {
                    let poly = self.list.poly_count;
                    let first = self.list.vertex_count;
                    self.list.vertices[first..first + 3].copy_from_slice(&vertices);
                    self.list.vertex_count += 3;
                    self.list.polys[poly] = Poly {
                        shader,
                        vertices: Span {
                            first: first as u32,
                            count: 3,
                        },
                        entity: Some(draw.index),
                    };
                    self.list.poly_count += 1;
                    self.list.draws[self.list.draw_count] = DrawItem {
                        kind: DrawKind::Poly,
                        index: poly as u32,
                        mesh: draw.mesh,
                        frames: draw.frames,
                        ..Default::default()
                    };
                    self.list.draw_count += 1;
                });
            } else {
                self.list.draws[valid] = draw;
                valid += 1;
            }
        }
        let generated = self.list.draw_count - end;
        self.list.draws.copy_within(end..end + generated, valid);
        self.list.draw_count = valid + generated;
        // Frozen numeric handles are resolved at the scene boundary, once per
        // item. Invalid client submissions are omitted without affecting peers.
        let first_draw = self.first.draws.first as usize;
        let mut valid = first_draw;
        for index in first_draw..self.list.draw_count {
            let mut draw = self.list.draws[index];
            draw.shader = source_shader(&self.list, assets, draw, shaders);
            if let Some(key) = draw_key(&self.list, assets, draw, shaders) {
                draw.key = key;
                self.list.draws[valid] = draw;
                valid += 1;
            } else {
                self.list.rejected += 1;
            }
        }
        self.list.draw_count = valid;
        native_draw_sort(&mut self.list.draws[first_draw..valid]);
        self.validate_particles(self.first.particles.first as usize, assets);
        for (rank, draw) in self.list.draws[first_draw..valid].iter().enumerate() {
            if draw.kind == DrawKind::Surface {
                self.list.surfaces[draw.index as usize].draw_rank = rank as u32;
                self.list.surfaces[draw.index as usize].draw_index =
                    Some((first_draw + rank) as u32);
            }
        }
        let current = self.current();
        let scene = self.first.through(current);
        let hidden = Span {
            first: self.list.area_count as u32,
            count: hidden_areas.len() as u32,
        };
        self.list.area_bytes[hidden.range()].copy_from_slice(hidden_areas);
        self.list.area_count += hidden_areas.len();
        self.command(Command::View(View {
            refdef,
            scene,
            hidden_areas: hidden,
            particle_client: self.particle_client,
        }));
        self.first = current;
        true
    }
    pub fn draw_2d(&mut self, draw: Draw2d) -> bool {
        self.command(Command::Draw2d(draw))
    }
}
fn draw_entity_index(list: &CommandList, draw: DrawItem) -> Option<u32> {
    match draw.kind {
        DrawKind::Entity => Some(draw.index),
        DrawKind::Surface => list.surface(draw.index).entity,
        DrawKind::Poly => list.poly(draw.index).entity,
    }
}

fn draw_allowed(list: &CommandList, draw: DrawItem, policy: ViewPolicy) -> bool {
    if draw_entity_index(list, draw).is_some() {
        policy.draw_entities
    } else {
        draw.kind != DrawKind::Surface || policy.draw_world
    }
}

pub(crate) fn draw_entity(list: &CommandList, draw: DrawItem) -> Option<SceneEntity> {
    let mut entity = *list.entity(draw_entity_index(list, draw)?);
    if let Some(mesh) = draw.mesh {
        entity.model = SceneModel::Mesh(mesh);
    } else if matches!(entity.model, SceneModel::Lods { .. }) {
        return None;
    }
    if let Some(shader) = draw.shader {
        entity.shader = Some(shader);
    }
    if let Some([current, previous]) = draw.frames {
        entity.frame = current;
        entity.old_frame = previous;
    }
    Some(entity)
}

fn model_draw(
    list: &CommandList,
    assets: &Assets,
    mut draw: DrawItem,
    view: &Refdef,
    projection: Option<[f32; 2]>,
) -> Result<Option<DrawItem>, ()> {
    if draw.kind != DrawKind::Entity {
        return Ok(Some(draw));
    }
    let entity = list.entity(draw.index);
    match entity.model {
        SceneModel::Mesh(model) => draw.mesh = Some(model),
        SceneModel::Lods { choices, radius } => {
            let level = view
                .model_lod
                .select(
                    radius.ok_or(())?,
                    choices.count,
                    entity.origin,
                    view,
                    projection.ok_or(())?,
                )
                .ok_or(())?;
            let Some(part) = list.lod_choices[choices.range()][level as usize] else {
                return Ok(None);
            };
            draw.mesh = Some(part.model);
            draw.shader = entity.shader.or(Some(part.shader));
        }
        _ => {}
    }
    if let Some(mesh) = draw.mesh {
        let model = assets.model(mesh).ok_or(())?;
        if let Some(animation) = model.animation() {
            let time = view.model_time();
            draw.frames = Some([
                animation
                    .frame_pose(entity.frame, time, entity.sync_base, view.frame_timing)
                    .ok_or(())?,
                animation
                    .frame_pose(entity.old_frame, time, entity.sync_base, view.frame_timing)
                    .ok_or(())?,
            ]);
        }
        if matches!(entity.model, SceneModel::Lods { .. }) || model.animation().is_some() {
            let selected = draw_entity(list, draw).ok_or(())?;
            model.posed_frames(&selected).ok_or(())?;
        }
    }
    Ok(Some(draw))
}
fn source_shader(
    list: &CommandList,
    assets: &Assets,
    draw: DrawItem,
    shaders: ShaderView<'_>,
) -> Option<ShaderRef> {
    if let Some(shader) = draw.shader {
        return Some(shader);
    }
    match draw.kind {
        DrawKind::Surface => {
            let surface = list.surface(draw.index);
            let binding = assets.scene_binding(*surface)?;
            if let Some(shader) = surface.entity.and_then(|index| list.entity(index).shader) {
                return Some(shader);
            }
            let presentation = assets.world_presentation(surface.presentation)?;
            shaders
                .choice(
                    binding.shader,
                    presentation.shader_registrations(),
                    shaders.context.and_then(|context| {
                        shaders.native.resource_nodes(
                            context,
                            crate::shader::registration::ShaderResource::World(
                                surface.presentation.0,
                            ),
                        )
                    }),
                    ShaderDefault::Global(presentation.native_default()),
                )?
                .for_surface(assets, presentation.surface_shader(surface.surface)?)
        }
        DrawKind::Entity => draw_entity(list, draw)?.shader(assets),
        DrawKind::Poly => Some(list.poly(draw.index).shader),
    }
}
pub(crate) fn item_shader(
    _list: &CommandList,
    assets: &Assets,
    draw: DrawItem,
) -> Option<ResolvedShader> {
    draw.shader?.resolve(assets, &())
}
pub(crate) fn item_material_lightmap(
    list: &CommandList,
    assets: &Assets,
    draw: DrawItem,
) -> Option<(MaterialId, u32)> {
    let binding = item_shader(list, assets, draw)?.resources(assets)?;
    Some((binding.material, binding.lightmap.0))
}
pub(crate) fn source_material_lightmap(
    assets: &Assets,
    draw: DrawItem,
    native: &dyn NativeShaders,
) -> Option<(MaterialId, u32)> {
    let binding = assets.shader_binding(draw.shader?.source(native)?)?;
    Some((binding.material, binding.lightmap.0))
}
fn draw_key(
    list: &CommandList,
    assets: &Assets,
    draw: DrawItem,
    shaders: ShaderView<'_>,
) -> Option<DrawKey> {
    let (material, lightmap) = source_material_lightmap(assets, draw, shaders.native)?;
    let instance = draw_entity_index(list, draw).unwrap_or(u32::MAX);
    if draw_entity(list, draw).is_some_and(|entity| entity.material_opacity.is_some()) {
        return Some(DrawKey {
            sort: 6.,
            material: 0,
            instance,
            lightmap: draw.index,
        });
    }
    let order = match draw.shader? {
        ShaderRef::Native(reference) | ShaderRef::Surface { reference, .. } => {
            u64::from(reference.node.0)
        }
        _ => (1_u64 << 32) | u64::from(material.0),
    };
    Some(DrawKey {
        sort: assets.material(material)?.settings.sort,
        material: order,
        instance,
        lightmap,
    })
}
/// Q3 tr_main.c qsortFast/shortsort. Equal keys deliberately retain the native
/// swap order: alpha draws observe it. SortNewShader supplies the float/material
/// ordering; neither camera distance nor an insertion ordinal enters this key.
fn native_draw_sort(draws: &mut [DrawItem]) {
    if draws.len() < 2 {
        return;
    }
    // The larger partition is stacked and the smaller is processed first.
    // Packet capacity is at most u32::MAX; this fixed stack therefore suffices.
    let mut pending = [(0_usize, 0_usize); u32::BITS as usize];
    let mut pending_count = 0;
    let mut lo = 0;
    let mut hi = draws.len() - 1;
    loop {
        let size = hi - lo + 1;
        if size <= 8 {
            let mut last = hi;
            while last > lo {
                let mut maximum = lo;
                for index in lo + 1..=last {
                    if draws[index].key.compare(draws[maximum].key) == SortOrdering::Greater {
                        maximum = index;
                    }
                }
                draws.swap(maximum, last);
                last -= 1;
            }
        } else {
            draws.swap(lo + size / 2, lo);
            let mut low = lo;
            let mut high = hi + 1;
            loop {
                loop {
                    low += 1;
                    if low > hi || draws[low].key.compare(draws[lo].key) == SortOrdering::Greater {
                        break;
                    }
                }
                loop {
                    high -= 1;
                    if high <= lo || draws[high].key.compare(draws[lo].key) == SortOrdering::Less {
                        break;
                    }
                }
                if high < low {
                    break;
                }
                draws.swap(low, high);
            }
            draws.swap(lo, high);
            let left_count = high - lo;
            let right_count = hi + 1 - low;
            if left_count >= right_count {
                if left_count > 1 {
                    pending[pending_count] = (lo, high - 1);
                    pending_count += 1;
                }
                if right_count > 1 {
                    lo = low;
                    continue;
                }
            } else {
                if right_count > 1 {
                    pending[pending_count] = (low, hi);
                    pending_count += 1;
                }
                if left_count > 1 {
                    hi = high - 1;
                    continue;
                }
            }
        }
        if pending_count == 0 {
            break;
        }
        pending_count -= 1;
        (lo, hi) = pending[pending_count];
    }
}
impl Frame {
    pub fn finish(self) -> CommandList {
        self.list
    }
}
