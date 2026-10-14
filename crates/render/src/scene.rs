//! Q3-style append-only scene ranges in alternating owned packets.
use crate::assets::{Assets, MaterialId, ModelId, PaletteId, Vertex};
use crate::surface_cache::IndexedLighting;
use crate::world::{VisibleSurface, WorldId, WorldPresentationId};
pub use qa_core::primitives::LightStyle;
use qa_core::primitives::{Orientation, Vec3};
use std::cmp::Ordering as SortOrdering;
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
#[derive(Clone, Copy, Debug)]
pub struct Refdef {
    pub viewport: Viewport,
    pub origin: Vec3,
    /// Forward, left, up: the native Q3 view axis convention.
    pub axes: [Vec3; 3],
    pub fov: [f32; 2],
    pub near: f32,
    pub far: f32,
    pub time_ms: i64,
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
    time_ms as f32 * 0.001
}

impl Refdef {
    pub fn hyperspace_color(&self) -> [u8; 4] {
        let value = (self.time_ms & 255) as u8;
        [value, value, value, 255]
    }
    pub fn time_seconds(&self) -> f32 {
        milliseconds_to_seconds(self.time_ms)
    }

    /// Shared view boundary for visibility and both drawing backends.
    /// Returns the native half-FOV tangents in the original operation order.
    pub fn validate(&self, width: u32, height: u32) -> Option<[f32; 2]> {
        if !self.viewport.fits(width, height)
            || (self.blend_phase == BlendPhase::FinalPalette
                && self.blend_viewport.is_some_and(|v| !v.fits(width, height)))
            || !self.origin.is_finite()
            || self.pvs_origin.is_some_and(|origin| !origin.is_finite())
            || !self.axes.iter().all(|axis| axis.is_finite())
            || !self
                .fov
                .iter()
                .chain(self.blend.iter())
                .all(|value| value.is_finite())
            || self.fov.iter().any(|&fov| fov <= 0.0 || fov >= 180.0)
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
        let tangent = self.fov.map(|fov| (fov.to_radians() * 0.5).tan());
        tangent
            .iter()
            .all(|&t| t > 0.0 && t.is_finite())
            .then_some(tangent)
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
            time_ms: 0,
            hyperspace: false,
            portal: false,
            reflected_view: false,
            clip_plane: None,
            pvs_origin: None,
            text: [[0; 32]; 8],
            identity_light: 1.0,
            blend: [0.0; 4],
            blend_phase: BlendPhase::AfterView,
            blend_viewport: None,
            cpu_presentation: CpuPresentation::Rgb,
            perspective_step: PerspectiveStep::Eight,
            palette_transform: None,
            lightstyles: [LightStyle::default(); 256],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SceneModel {
    Mesh(ModelId),
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
            Self::World { .. }
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
    pub material: Option<MaterialId>,
    pub frame: u32,
    pub old_frame: u32,
    pub back_lerp: f32,
    pub depth_hack: bool,
    pub shader_time: f32,
    pub shader_texcoord: [f32; 2],
    pub lighting: Option<crate::stage::EntityLighting>,
    pub lighting_origin: Option<Vec3>,
    pub minimum_light: bool,
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
    pub fn material(&self, assets: &Assets) -> Option<MaterialId> {
        self.material
            .or_else(|| Some(assets.model(self.model.mesh()?)?.material))
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
            material: None,
            frame: 0,
            old_frame: 0,
            back_lerp: 0.0,
            depth_hack: false,
            shader_time: 0.0,
            shader_texcoord: [0.0; 2],
            lighting: None,
            lighting_origin: None,
            minimum_light: false,
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
    pub material: MaterialId,
    pub vertices: Span,
    pub entity: Option<u32>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Draw2d {
    pub rect: [f32; 4],
    pub texcoords: [f32; 4],
    pub color: [u8; 4],
    pub material: MaterialId,
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
    pub material: MaterialId,
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
            material: MaterialId(0),
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
            for triangle in geometry.indices[surface.indices.range()].chunks_exact(3) {
                visit(
                    [triangle[0], triangle[1], triangle[2]]
                        .map(|index| transform(geometry.vertices[index as usize].vertex, entity)),
                );
            }
        }
        DrawKind::Entity => {
            let entity = list.entity(item.index);
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
                        .map(|index| transform(frames.vertex(index as usize), Some(entity))),
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
    /// Absolute index in this packet's one copied entity arena. Static world
    /// visibility keeps None; inline geometry borrows its instance inputs.
    pub entity: Option<u32>,
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
    key: DrawKey,
}
#[derive(Clone, Copy, Debug, Default)]
struct DrawKey {
    sort: f32,
    material: u32,
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
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub commands: usize,
    pub entities: usize,
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
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            commands: 4096,
            entities: 2048,
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
    polys: Box<[Poly]>,
    vertices: Box<[Vertex]>,
    lights: Box<[Light]>,
    particles: Box<[SceneParticle]>,
    area_bytes: Box<[u8]>,
    surfaces: Box<[SurfaceRef]>,
    draws: Box<[DrawItem]>,
    command_count: usize,
    entity_count: usize,
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
            polys: vec![Poly::default(); l.polys].into_boxed_slice(),
            vertices: vec![Vertex::default(); l.vertices].into_boxed_slice(),
            lights: vec![Light::default(); l.lights].into_boxed_slice(),
            particles: vec![SceneParticle::default(); l.particles].into_boxed_slice(),
            area_bytes: vec![0; l.area_bytes].into_boxed_slice(),
            surfaces: vec![SurfaceRef::default(); l.surfaces].into_boxed_slice(),
            draws: vec![DrawItem::default(); l.draws].into_boxed_slice(),
            command_count: 0,
            entity_count: 0,
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
    fn reset(&mut self, frame: u64) {
        self.command_count = 0;
        self.entity_count = 0;
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
    next: usize,
    frame: u64,
    owner: u64,
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
}
impl FrontEnd {
    pub fn load(mut limits: Limits) -> Result<Self, &'static str> {
        limits.draws = limits
            .draw_capacity()
            .ok_or("scene draw capacity overflow")?;
        if [
            limits.commands,
            limits.entities,
            limits.polys,
            limits.vertices,
            limits.lights,
            limits.particles,
            limits.area_bytes,
            limits.surfaces,
            limits.draws,
        ]
        .iter()
        .any(|&n| n == 0 || n > u32::MAX as usize)
        {
            return Err("invalid scene capacity");
        }
        let owner = crate::next_owner_id().map_err(|_| "scene owner ids exhausted")?;
        Ok(Self {
            lists: [
                Some(CommandList::load(limits, owner, 0)),
                Some(CommandList::load(limits, owner, 1)),
            ],
            next: 0,
            frame: 0,
            owner,
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
        let generated = self.list.entities
            [self.first.entities.first as usize..self.list.entity_count]
            .iter()
            .filter(|entity| entity.visibility.visible(refdef.portal))
            .filter_map(|entity| crate::entities::prepare(entity, refdef, assets))
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
    ) -> bool {
        let checkpoint = self.checkpoint();
        let source = checkpoint.first.through(checkpoint.end);
        let has_portal = !refdef.portal
            && !refdef.hyperspace
            && self
                .list
                .entities(source.entities)
                .iter()
                .any(|entity| matches!(entity.model, SceneModel::Portal { .. }));
        if has_portal {
            let first = self.list.draw_count;
            if source.draws.count as usize <= self.list.draws.len() - first {
                let mut count = 0;
                for index in source.draws.range() {
                    let mut draw = self.list.draws[index];
                    if let Some(key) = draw_key(&self.list, assets, draw) {
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
                if let Some(child) = crate::portal::first_child(&self.list, sorted, assets, refdef)
                {
                    let rejected = self.list.rejected;
                    let retained = self.copy_scene(source, Some(world))
                        && assets.world(world).is_some_and(|loaded| {
                            scratch
                                .query(loaded, child, None, hidden_areas)
                                .is_ok_and(|surfaces| self.add_world(world, presentation, surfaces))
                        })
                        && self.render_scene(child, hidden_areas, assets)
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
        self.render_scene(refdef, hidden_areas, assets)
    }

    pub fn reject_submission(&mut self) -> bool {
        self.list.rejected += 1;
        false
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
                && assets.material(particle.material).is_some()
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
                && matches!(entity.model, SceneModel::Mesh(_) | SceneModel::World { .. })
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
    /// One submitted model may contain several surfaces. Admit that complete
    /// group or restore all counts, without copying/growing packet storage.
    pub fn add_entities(&mut self, entities: impl IntoIterator<Item = SceneEntity>) -> bool {
        let before = (
            self.list.entity_count,
            self.list.surface_count,
            self.list.draw_count,
        );
        for entity in entities {
            if !self.add_entity_part(entity) {
                (
                    self.list.entity_count,
                    self.list.surface_count,
                    self.list.draw_count,
                ) = before;
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
        match entity.model {
            SceneModel::Mesh(_)
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
    pub fn add_poly(&mut self, material: MaterialId, vertices: &[Vertex]) -> bool {
        self.add_poly_from(material, vertices.len(), &mut |index| vertices[index])
    }
    /// Copy directly into the packet after reserving all polygon storage. This
    /// admits checked module vertices without a temporary allocation.
    pub fn add_poly_from(
        &mut self,
        material: MaterialId,
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
            material,
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
        };
        self.list.draw_count += 1;
    }
    pub fn render_scene(&mut self, refdef: Refdef, hidden_areas: &[u8], assets: &Assets) -> bool {
        if self.list.command_count == self.list.commands.len()
            || hidden_areas.len() > self.list.area_bytes.len() - self.list.area_count
        {
            self.list.rejected += 1;
            return false;
        }
        let end = self.list.draw_count;
        let first_draw = self.first.draws.first as usize;
        let mut valid = first_draw;
        for index in first_draw..end {
            let draw = self.list.draws[index];
            if draw_entity(&self.list, draw)
                .is_some_and(|entity| !entity.visibility.visible(refdef.portal))
            {
                continue;
            }
            if draw.kind == DrawKind::Entity
                && let Some(geometry) =
                    crate::entities::prepare(self.list.entity(draw.index), refdef, assets)
            {
                let count = geometry.triangle_count();
                if count > self.list.polys.len() - self.list.poly_count
                    || count > self.list.draws.len() - self.list.draw_count
                    || count > (self.list.vertices.len() - self.list.vertex_count) / 3
                {
                    self.list.rejected += 1;
                    continue;
                }
                let entity = *self.list.entity(draw.index);
                let Some(material) = entity.material(assets) else {
                    self.list.rejected += 1;
                    continue;
                };
                geometry.triangles(|vertices| {
                    let poly = self.list.poly_count;
                    let first = self.list.vertex_count;
                    self.list.vertices[first..first + 3].copy_from_slice(&vertices);
                    self.list.vertex_count += 3;
                    self.list.polys[poly] = Poly {
                        material,
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
            if let Some(key) = draw_key(&self.list, assets, draw) {
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
pub(crate) fn draw_entity(list: &CommandList, draw: DrawItem) -> Option<&SceneEntity> {
    match draw.kind {
        DrawKind::Entity => Some(list.entity(draw.index)),
        DrawKind::Surface => list
            .surface(draw.index)
            .entity
            .map(|index| list.entity(index)),
        DrawKind::Poly => list.poly(draw.index).entity.map(|index| list.entity(index)),
    }
}
pub(crate) fn item_material_lightmap(
    list: &CommandList,
    assets: &Assets,
    draw: DrawItem,
) -> Option<(MaterialId, u32)> {
    match draw.kind {
        DrawKind::Surface => {
            let surface = list.surface(draw.index);
            let binding = assets.scene_binding(*surface)?;
            let material = surface
                .entity
                .and_then(|index| list.entity(index).material)
                .unwrap_or(binding.material);
            Some((material, binding.lightmap.0))
        }
        DrawKind::Entity => Some((list.entity(draw.index).material(assets)?, 0)),
        DrawKind::Poly => Some((list.poly(draw.index).material, 0)),
    }
}
fn draw_key(list: &CommandList, assets: &Assets, draw: DrawItem) -> Option<DrawKey> {
    let (material, lightmap) = item_material_lightmap(list, assets, draw)?;
    let instance = match draw.kind {
        DrawKind::Entity => draw.index,
        DrawKind::Surface => list.surface(draw.index).entity.unwrap_or(u32::MAX),
        DrawKind::Poly => list.poly(draw.index).entity.unwrap_or(u32::MAX),
    };
    if draw_entity(list, draw).is_some_and(|entity| entity.material_opacity.is_some()) {
        return Some(DrawKey {
            sort: 6.0,
            material: 0,
            instance,
            lightmap: draw.index,
        });
    }
    Some(DrawKey {
        sort: assets.material(material)?.settings.sort,
        material: material.0,
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
