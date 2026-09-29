//! Q2 shadow atlas preparation (donor `src/render/scene/shadows.ts`).
//!
//! The first eight Q2 lights are eligible for depth rendering; cone lights
//! take one atlas tile and point lights take a 3x2 cube grid. Per-light
//! signature strings skip rebuilds when neither the light, its tile, nor any
//! caster geometry changed since the previous preparation.

use std::collections::HashMap;
use std::f64::consts::{PI, SQRT_2};

use qa_core::math::{cross3, dot3, length3, scale3, sub3, vec3, vec4, Mat4, Vec3, Vec4};

use super::resources::SceneImageRegistry;
use crate::materials::geometry::MaterialGeometry;
use crate::render::error::RenderError;
use crate::render::types::{
    CullFace, DepthAtlasDraw, DepthAtlasPass, DepthImageLevel, ImageResourceOperation, PolygonOffset, Q2FragmentLight,
    Q2LightCone, Q2ShadowAtlas, Q2ShadowProjection, Rect, RenderImage, RenderOperation, RendererImage, ResourceOwner,
    TextureFilter, TextureSampling,
};
use crate::render::{LightProfile, LightShadow, SceneLight};

/// Shadow atlas edge length in texels.
pub const Q2_SHADOW_ATLAS_SIZE: u32 = 2048;
/// Shadow projection near plane in world units.
pub const Q2_SHADOW_NEAR: f32 = 4.0;

/// Smallest per-face shadow resolution.
const MINIMUM_RESOLUTION: u32 = 128;
/// Lights considered for the atlas, before eligibility filtering.
const MAXIMUM_LIGHTS: usize = 8;
/// Depth range remap applied to cone projections.
const DEPTH_BIAS: Mat4 = [
    0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.5, 0.5, 1.0,
];

/// World-space triangle soup rendered into the atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowMesh {
    /// Vertex positions.
    pub positions: Vec<Vec3>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

/// Bounding sphere tested against light volumes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSphere {
    /// Sphere center.
    pub origin: Vec3,
    /// Sphere radius.
    pub radius: f32,
}

/// Entity caster: a bounding sphere plus its meshes.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowCaster {
    /// Sphere center.
    pub origin: Vec3,
    /// Sphere radius.
    pub radius: f32,
    /// Caster meshes.
    pub meshes: Vec<ShadowMesh>,
}

/// Cube-face camera basis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceBasis {
    /// View direction.
    pub forward: Vec3,
    /// Right vector.
    pub right: Vec3,
    /// Up vector.
    pub up: Vec3,
}

/// Cube-face bases in donor order (+x, -x, +y, -y, +z, -z).
pub const Q2_SHADOW_CUBE_FACES: [FaceBasis; 6] = [
    FaceBasis {
        forward: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        right: Vec3 {
            x: 0.0,
            y: -1.0,
            z: 0.0,
        },
        up: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    },
    FaceBasis {
        forward: Vec3 {
            x: -1.0,
            y: 0.0,
            z: 0.0,
        },
        right: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        up: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    },
    FaceBasis {
        forward: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        right: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        up: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    },
    FaceBasis {
        forward: Vec3 {
            x: 0.0,
            y: -1.0,
            z: 0.0,
        },
        right: Vec3 {
            x: -1.0,
            y: 0.0,
            z: 0.0,
        },
        up: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    },
    FaceBasis {
        forward: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
        right: Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        up: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
    },
    FaceBasis {
        forward: Vec3 {
            x: 0.0,
            y: 0.0,
            z: -1.0,
        },
        right: Vec3 {
            x: 0.0,
            y: -1.0,
            z: 0.0,
        },
        up: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
    },
];

/// Atlas preparation options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowAtlasOptions {
    /// Whether shadows render at all.
    pub enabled: bool,
    /// Maximum per-face resolution.
    pub resolution_cap: u32,
}

impl Default for ShadowAtlasOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            resolution_cap: 1024,
        }
    }
}

/// Per-preparation counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShadowStats {
    /// Shadowed lights (cached plus rebuilt).
    pub lights: usize,
    /// Lights reused from the previous preparation.
    pub cached_lights: usize,
    /// Lights rendered this preparation.
    pub rebuilt_lights: usize,
    /// Atlas passes rendered.
    pub faces_rendered: usize,
    /// Entity casters drawn into rebuilt passes.
    pub entity_casters: usize,
}

/// Prepared fragment lighting plus its atlas.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowLighting {
    /// Fragment lights for the first eight Q2 source lights.
    pub lights: Vec<Q2FragmentLight>,
    /// Atlas backing the shadow projections, if any light casts.
    pub atlas: Option<Q2ShadowAtlas>,
}

/// Atlas preparation output.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedShadows {
    /// Lighting state.
    pub lighting: ShadowLighting,
    /// Atlas render operations (empty when fully cached).
    pub operations: Vec<RenderOperation>,
    /// Counters.
    pub stats: ShadowStats,
}

/// World geometry input: a borrowed slice or a digest-caching static world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShadowWorldInput<'a> {
    /// Caller-owned meshes, digested every preparation.
    Meshes(&'a [ShadowMesh]),
    /// Static world with a cached digest.
    Static(&'a StaticShadowWorld),
}

/// Shadow mesh over material vertex positions.
#[must_use]
pub fn shadow_mesh(geometry: &MaterialGeometry) -> ShadowMesh {
    ShadowMesh {
        positions: geometry.vertices.iter().map(|vertex| vertex.position).collect(),
        indices: geometry.indices.clone(),
    }
}

/// Caster whose radius covers every mesh position, defaulting to 64.
#[must_use]
pub fn shadow_caster(origin: Vec3, meshes: Vec<ShadowMesh>) -> ShadowCaster {
    let mut radius = 0.0f32;
    for mesh in &meshes {
        for position in &mesh.positions {
            radius = radius.max(length3(sub3(*position, origin)));
        }
    }
    ShadowCaster {
        origin,
        radius: if radius > 0.0 { radius } else { 64.0 },
        meshes,
    }
}

/// Largest power of two honoring the requested resolution and cap.
#[must_use]
pub fn shadow_map_resolution(requested: u32, cap: u32) -> u32 {
    let maximum = MINIMUM_RESOLUTION.max(if cap > 0 { cap } else { 1024 }.min(1024));
    let wanted = MINIMUM_RESOLUTION.max(if requested > 0 { requested } else { 512 }.min(maximum));
    1 << wanted.ilog2()
}

/// Shelf-pack requests into a square atlas; failures pack as `None`.
#[must_use]
pub fn pack_shadow_atlas(requests: &[(u32, u32)], size: u32) -> Vec<Option<Rect>> {
    let (mut x, mut y, mut shelf_height) = (0u32, 0u32, 0u32);
    requests
        .iter()
        .map(|&(width, height)| {
            if width == 0 || height == 0 || width > size || height > size {
                return None;
            }
            if x + width > size {
                y += shelf_height;
                shelf_height = 0;
                x = 0;
            }
            if y + height > size {
                return None;
            }
            let rect = Rect {
                x: x as f32,
                y: y as f32,
                width: width as f32,
                height: height as f32,
            };
            x += width;
            shelf_height = shelf_height.max(height);
            Some(rect)
        })
        .collect()
}

/// Column-major product; each cell accumulates in `f64` and rounds once.
#[must_use]
pub fn shadow_matrix_multiply(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0f32; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut acc = 0.0f64;
            for k in 0..4 {
                acc += f64::from(a[k * 4 + row]) * f64::from(b[column * 4 + k]);
            }
            out[column * 4 + row] = acc as f32;
        }
    }
    out
}

/// Project a world position through a shadow matrix.
#[must_use]
pub fn shadow_project(matrix: &Mat4, position: Vec3) -> Vec4 {
    let (x, y, z) = (f64::from(position.x), f64::from(position.y), f64::from(position.z));
    let cell = |index: usize| f64::from(matrix[index]);
    vec4(
        (cell(0) * x + cell(4) * y + cell(8) * z + cell(12)) as f32,
        (cell(1) * x + cell(5) * y + cell(9) * z + cell(13)) as f32,
        (cell(2) * x + cell(6) * y + cell(10) * z + cell(14)) as f32,
        (cell(3) * x + cell(7) * y + cell(11) * z + cell(15)) as f32,
    )
}

/// Full spotlight field of view in degrees for a cone cosine, with margin.
#[must_use]
pub fn shadow_cone_fov(cos_half_angle: f32) -> f32 {
    let half_angle = f64::from(cos_half_angle).clamp(-1.0, 1.0).acos();
    (half_angle * 180.0 / PI * 2.0 * 1.15).min(175.0) as f32
}

/// Cone projection for a spotlight; point lights have no single matrix.
pub fn shadow_cone_matrix(light: &Q2FragmentLight) -> Result<Mat4, RenderError> {
    let cone = light
        .cone
        .as_ref()
        .ok_or_else(|| RenderError::BadProjection("A cone projection requires a spotlight direction".to_string()))?;
    Ok(shadow_matrix_multiply(
        &perspective(shadow_cone_fov(cone.cos_half_angle), light.radius),
        &view(light.origin, &cone_basis(cone.direction)),
    ))
}

/// Predicate matching spheres inside any eligible shadow light volume.
///
/// The atlas selects the first eight Q2 lights before testing eligibility.
pub fn shadow_body_filter(source: &[SceneLight]) -> impl Fn(&ShadowSphere) -> bool {
    let tests: Vec<_> = source
        .iter()
        .filter(|light| matches!(light.profile, LightProfile::Q2 { .. }))
        .take(MAXIMUM_LIGHTS)
        .filter(|light| eligible_shadow_light(light))
        .map(|light| light_contains_sphere(fragment_light(light)))
        .collect();
    move |sphere| tests.iter().any(|test| test(sphere))
}

fn perspective(fov: f32, radius: f32) -> Mat4 {
    let scale = 1.0 / (f64::from(fov) * PI / 360.0).tan();
    let far = f64::from(radius).max(8.0);
    let near = f64::from(Q2_SHADOW_NEAR);
    [
        scale as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        scale as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        ((far + near) / (near - far)) as f32,
        -1.0,
        0.0,
        0.0,
        ((2.0 * far * near) / (near - far)) as f32,
        0.0,
    ]
}

fn view(origin: Vec3, basis: &FaceBasis) -> Mat4 {
    let (r, u, v) = (basis.right, basis.up, basis.forward);
    let (ox, oy, oz) = (f64::from(origin.x), f64::from(origin.y), f64::from(origin.z));
    let dot = |a: Vec3| f64::from(a.x) * ox + f64::from(a.y) * oy + f64::from(a.z) * oz;
    [
        r.x,
        u.x,
        -v.x,
        0.0,
        r.y,
        u.y,
        -v.y,
        0.0,
        r.z,
        u.z,
        -v.z,
        0.0,
        (-dot(r)) as f32,
        (-dot(u)) as f32,
        dot(v) as f32,
        1.0,
    ]
}

fn cone_basis(direction: Vec3) -> FaceBasis {
    let forward = if length3(direction) < 1e-6 {
        vec3(0.0, 0.0, -1.0)
    } else {
        scale3(direction, 1.0 / length3(direction))
    };
    let reference = if forward.z.abs() >= 0.99 {
        vec3(1.0, 0.0, 0.0)
    } else {
        vec3(0.0, 0.0, 1.0)
    };
    let across = cross3(forward, reference);
    let right = scale3(across, 1.0 / length3(across));
    FaceBasis {
        forward,
        right,
        up: cross3(right, forward),
    }
}

fn fragment_light(light: &SceneLight) -> Q2FragmentLight {
    let (scale, cone) = match light.profile {
        LightProfile::Q2 { scale, cone, .. } => (
            scale,
            cone.map(|(direction, cos_half_angle)| Q2LightCone {
                direction,
                cos_half_angle,
            }),
        ),
        LightProfile::Simple => (1.0, None),
    };
    Q2FragmentLight {
        origin: light.origin,
        radius: light.radius,
        color: light.color,
        scale,
        cone,
        shadow: Q2ShadowProjection::None,
    }
}

fn eligible_shadow_light(light: &SceneLight) -> bool {
    match light.profile {
        LightProfile::Q2 { cone, shadow, .. } => {
            light.radius > 0.0 && (matches!(shadow, LightShadow::Cast { .. }) || cone.is_some())
        }
        LightProfile::Simple => false,
    }
}

fn caster_sphere(caster: &ShadowCaster) -> ShadowSphere {
    ShadowSphere {
        origin: caster.origin,
        radius: caster.radius,
    }
}

fn light_contains_sphere(light: Q2FragmentLight) -> impl Fn(&ShadowSphere) -> bool {
    let diagonal = match light.cone {
        None => PI,
        Some(cone) => {
            let half_angle = f64::from(shadow_cone_fov(cone.cos_half_angle)) * PI / 360.0;
            (SQRT_2 * half_angle.min(87.0 * PI / 180.0).tan()).atan()
        }
    };
    let limit = diagonal.cos();
    move |caster: &ShadowSphere| {
        let delta = sub3(caster.origin, light.origin);
        let distance = f64::from(length3(delta));
        if distance > f64::from(light.radius) + f64::from(caster.radius) {
            return false;
        }
        if let Some(cone) = light.cone {
            if distance > f64::from(caster.radius) {
                let cos_angle = (f64::from(dot3(delta, cone.direction)) / distance).clamp(-1.0, 1.0);
                let angle = cos_angle.acos();
                let separation = angle - (f64::from(caster.radius) / distance).min(1.0).asin();
                if separation.clamp(0.0, PI).cos() < limit {
                    return false;
                }
            }
        }
        true
    }
}

struct Candidate {
    index: usize,
    light: Q2FragmentLight,
    face: u32,
    casters: Vec<ShadowCaster>,
}

fn extent(candidate: &Candidate) -> (u32, u32) {
    if candidate.light.cone.is_none() {
        (candidate.face * 3, candidate.face * 2)
    } else {
        (candidate.face, candidate.face)
    }
}

fn fit(candidates: &mut [Candidate]) -> Vec<Option<Rect>> {
    let pack = |candidates: &mut [Candidate]| {
        candidates.sort_by(|a, b| {
            let (aw, ah) = extent(a);
            let (bw, bh) = extent(b);
            (bw * bh).cmp(&(aw * ah))
        });
        let requests: Vec<(u32, u32)> = candidates.iter().map(extent).collect();
        pack_shadow_atlas(&requests, Q2_SHADOW_ATLAS_SIZE)
    };
    let mut slots = pack(candidates);
    for _ in 0..4 {
        if slots.iter().all(Option::is_some) {
            break;
        }
        let mut shrank = false;
        for candidate in candidates.iter_mut() {
            if candidate.light.cone.is_none() && candidate.face > MINIMUM_RESOLUTION {
                candidate.face /= 2;
                shrank = true;
            }
        }
        if !shrank {
            break;
        }
        slots = pack(candidates);
    }
    slots
}

fn world_mesh_visible(mesh: &ShadowMesh, light: &Q2FragmentLight, forward: Option<Vec3>) -> bool {
    let radius_squared = light.radius.max(8.0).powi(2);
    let mut in_radius = false;
    let mut in_front = forward.is_none();
    for position in &mesh.positions {
        let delta = sub3(*position, light.origin);
        in_radius |= dot3(delta, delta) <= radius_squared;
        if let Some(facing) = forward {
            in_front |= dot3(delta, facing) > 0.0;
        }
        if in_radius && in_front {
            return true;
        }
    }
    false
}

fn draw(mesh: &ShadowMesh, matrix: &Mat4, entity: bool) -> DepthAtlasDraw {
    DepthAtlasDraw {
        positions: mesh
            .positions
            .iter()
            .map(|position| shadow_project(matrix, *position))
            .collect(),
        indices: mesh.indices.clone(),
        cull: CullFace::None,
        polygon_offset: Some(if entity {
            PolygonOffset {
                factor: 1.0,
                units: 2.0,
            }
        } else {
            PolygonOffset {
                factor: 2.0,
                units: 4.0,
            }
        }),
    }
}

fn depth_pass(
    candidate: &Candidate,
    viewport: Rect,
    matrix: &Mat4,
    world: &[ShadowMesh],
    forward: Option<Vec3>,
) -> DepthAtlasPass {
    let mut draws: Vec<DepthAtlasDraw> = world
        .iter()
        .filter(|mesh| world_mesh_visible(mesh, &candidate.light, forward))
        .map(|mesh| draw(mesh, matrix, false))
        .collect();
    for caster in &candidate.casters {
        if let Some(facing) = forward {
            if dot3(sub3(caster.origin, candidate.light.origin), facing) + caster.radius <= 0.0 {
                continue;
            }
        }
        draws.extend(caster.meshes.iter().map(|mesh| draw(mesh, matrix, true)));
    }
    DepthAtlasPass {
        viewport,
        clear_depth: Some(1.0),
        draws,
    }
}

/// FNV-1a digest over mesh counts, position bit patterns, and indices.
///
/// Documented deviation: the donor hashes the same fields with SHA-256, but no
/// hash dependency is available here. The digest only drives change detection,
/// so any stable 64-bit mix preserves behavior.
fn geometry_digest(meshes: &[ShadowMesh]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |word: u64| {
        for byte in word.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for mesh in meshes {
        mix(mesh.positions.len() as u64);
        mix(mesh.indices.len() as u64);
        for position in &mesh.positions {
            mix(u64::from(position.x.to_bits()));
            mix(u64::from(position.y.to_bits()));
            mix(u64::from(position.z.to_bits()));
        }
        for index in &mesh.indices {
            mix(u64::from(*index));
        }
    }
    hash
}

/// Immutable world geometry with a cached digest.
#[derive(Debug, Clone, PartialEq)]
pub struct StaticShadowWorld {
    meshes: Vec<ShadowMesh>,
    digest: Option<u64>,
}

impl StaticShadowWorld {
    /// Freeze world meshes for repeated preparations.
    #[must_use]
    pub fn new(meshes: Vec<ShadowMesh>) -> Self {
        Self { meshes, digest: None }
    }

    /// Borrow the frozen meshes.
    #[must_use]
    pub fn meshes(&self) -> &[ShadowMesh] {
        &self.meshes
    }

    /// Digest of the frozen meshes, computed once.
    pub fn digest(&mut self) -> u64 {
        if self.digest.is_none() {
            self.digest = Some(geometry_digest(&self.meshes));
        }
        self.digest.unwrap_or(0)
    }
}

/// Q2 shadow atlas scene owning its image registry.
pub struct Q2ShadowScene {
    images: SceneImageRegistry,
    image: Option<RendererImage>,
    cached: HashMap<usize, String>,
}

impl Q2ShadowScene {
    /// Create a scene with a fresh image registry for `owner`.
    #[must_use]
    pub fn new(owner: ResourceOwner) -> Self {
        Self {
            images: SceneImageRegistry::new(owner),
            image: None,
            cached: HashMap::new(),
        }
    }

    /// Prepare atlas passes for the eligible shadow lights in `source`.
    pub fn prepare(
        &mut self,
        source: &[SceneLight],
        world: ShadowWorldInput<'_>,
        casters: &[ShadowCaster],
        options: &ShadowAtlasOptions,
    ) -> PreparedShadows {
        let mut lights: Vec<Q2FragmentLight> = source
            .iter()
            .filter(|light| matches!(light.profile, LightProfile::Q2 { .. }))
            .take(MAXIMUM_LIGHTS)
            .map(fragment_light)
            .collect();
        let q2_source: Vec<&SceneLight> = source
            .iter()
            .filter(|light| matches!(light.profile, LightProfile::Q2 { .. }))
            .collect();
        let mut candidates: Vec<Candidate> = Vec::new();
        if options.enabled {
            for (index, light) in lights.iter().take(MAXIMUM_LIGHTS).enumerate() {
                let Some(original) = q2_source.get(index).copied() else {
                    continue;
                };
                if !matches!(original.profile, LightProfile::Q2 { .. }) || !eligible_shadow_light(original) {
                    continue;
                }
                let requested = match original.profile {
                    LightProfile::Q2 {
                        shadow: LightShadow::Cast { resolution },
                        ..
                    } => u32::try_from(resolution).unwrap_or(0),
                    _ => 0,
                };
                let inside = light_contains_sphere(*light);
                candidates.push(Candidate {
                    index,
                    light: *light,
                    face: shadow_map_resolution(requested, options.resolution_cap),
                    casters: casters
                        .iter()
                        .filter(|caster| inside(&caster_sphere(caster)))
                        .cloned()
                        .collect(),
                });
            }
        }
        if !options.enabled {
            self.close();
        }
        if candidates.is_empty() {
            return PreparedShadows {
                lighting: ShadowLighting { lights, atlas: None },
                operations: Vec::new(),
                stats: ShadowStats::default(),
            };
        }
        if self.image.is_none() {
            let size = Q2_SHADOW_ATLAS_SIZE;
            let content = RenderImage::Depth32f {
                levels: vec![DepthImageLevel {
                    width: size,
                    height: size,
                    pixels: vec![1.0f32; (size * size) as usize],
                }],
            };
            let sampling = TextureSampling {
                repeat: false,
                filter: TextureFilter::Nearest,
            };
            match self.images.register("*q2-shadow-atlas", content, sampling) {
                Ok(image) => self.image = Some(image),
                Err(_) => {
                    return PreparedShadows {
                        lighting: ShadowLighting { lights, atlas: None },
                        operations: Vec::new(),
                        stats: ShadowStats::default(),
                    };
                }
            }
        }
        let Some(image) = self.image.clone() else {
            return PreparedShadows {
                lighting: ShadowLighting { lights, atlas: None },
                operations: Vec::new(),
                stats: ShadowStats::default(),
            };
        };
        let slots = fit(&mut candidates);
        let world_meshes: &[ShadowMesh] = match &world {
            ShadowWorldInput::Meshes(meshes) => meshes,
            ShadowWorldInput::Static(static_world) => static_world.meshes(),
        };
        // Static input shares only `&self` here, so digest from the borrowed
        // meshes; the value equals the cached digest for identical geometry.
        let world_key = geometry_digest(world_meshes);
        let mut passes: Vec<DepthAtlasPass> = Vec::new();
        let mut signatures: HashMap<usize, String> = HashMap::new();
        let (mut cached_lights, mut rebuilt_lights, mut entity_casters) = (0usize, 0usize, 0usize);
        for (order, candidate) in candidates.iter().enumerate() {
            let Some(slot) = slots.get(order).copied().flatten() else {
                continue;
            };
            let matrix = candidate
                .light
                .cone
                .map(|_| shadow_cone_matrix(&candidate.light).expect("cone presence was just checked"));
            let size = Q2_SHADOW_ATLAS_SIZE as f32;
            let atlas_rect = vec4(slot.x / size, slot.y / size, slot.width / size, slot.height / size);
            lights[candidate.index] = Q2FragmentLight {
                shadow: match matrix {
                    None => Q2ShadowProjection::Point { atlas_rect },
                    Some(projection) => Q2ShadowProjection::Cone {
                        matrix: shadow_matrix_multiply(&DEPTH_BIAS, &projection),
                        atlas_rect,
                    },
                },
                ..candidate.light
            };
            let caster_part: Vec<(Vec3, f32, u64)> = candidate
                .casters
                .iter()
                .map(|caster| (caster.origin, caster.radius, geometry_digest(&caster.meshes)))
                .collect();
            let signature = format!(
                "{:?}|{:?}|{:?}|{slot:?}|{world_key}|{caster_part:?}",
                candidate.light.origin, candidate.light.radius, candidate.light.cone,
            );
            signatures.insert(candidate.index, signature.clone());
            if self.cached.get(&candidate.index) == Some(&signature) {
                cached_lights += 1;
                continue;
            }
            rebuilt_lights += 1;
            entity_casters += candidate.casters.len();
            match matrix {
                Some(projection) => {
                    passes.push(depth_pass(candidate, slot, &projection, world_meshes, None));
                }
                None => {
                    for (face, basis) in Q2_SHADOW_CUBE_FACES.iter().enumerate() {
                        let projection = shadow_matrix_multiply(
                            &perspective(90.0, candidate.light.radius),
                            &view(candidate.light.origin, basis),
                        );
                        let tile = candidate.face as f32;
                        passes.push(depth_pass(
                            candidate,
                            Rect {
                                x: slot.x + (face % 3) as f32 * tile,
                                y: slot.y + (face / 3) as f32 * tile,
                                width: tile,
                                height: tile,
                            },
                            &projection,
                            world_meshes,
                            Some(basis.forward),
                        ));
                    }
                }
            }
        }
        self.cached = signatures;
        let faces_rendered = passes.len();
        PreparedShadows {
            lighting: ShadowLighting {
                lights,
                atlas: Some(Q2ShadowAtlas {
                    image: image.clone(),
                    texel_size: 1.0 / Q2_SHADOW_ATLAS_SIZE as f32,
                    near_plane: Q2_SHADOW_NEAR,
                }),
            },
            operations: if passes.is_empty() {
                Vec::new()
            } else {
                vec![RenderOperation::DepthAtlas { image, passes }]
            },
            stats: ShadowStats {
                lights: cached_lights + rebuilt_lights,
                cached_lights,
                rebuilt_lights,
                faces_rendered,
                entity_casters,
            },
        }
    }

    /// Drain pending image lifecycle operations.
    #[must_use]
    pub fn drain_operations(&mut self) -> Vec<ImageResourceOperation> {
        self.images.drain_operations()
    }

    /// Release the atlas image and drop cached signatures.
    pub fn close(&mut self) {
        if let Some(image) = self.image.take() {
            let _ = self.images.release(&image);
        }
        self.cached.clear();
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, Vec2};

    use crate::materials::geometry::{MaterialGeometry, MaterialVertex};

    use super::*;

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("shadow-test").expect("session");
        ResourceOwner::new(1, authority.session().clone(), 0)
    }

    fn spot_light() -> SceneLight {
        SceneLight {
            origin: vec3(0.0, 0.0, 100.0),
            color: vec3(1.0, 1.0, 1.0),
            radius: 256.0,
            additive: false,
            profile: LightProfile::Q2 {
                scale: 1.0,
                cone: Some((vec3(0.0, 0.0, -1.0), 0.9)),
                shadow: LightShadow::Cast { resolution: 512 },
            },
        }
    }

    fn triangle_mesh() -> ShadowMesh {
        ShadowMesh {
            positions: vec![vec3(0.0, 0.0, 0.0), vec3(16.0, 0.0, 0.0), vec3(0.0, 16.0, 0.0)],
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn resolution_snaps_to_power_of_two() {
        assert_eq!(shadow_map_resolution(512, 1024), 512);
        assert_eq!(shadow_map_resolution(700, 1024), 512);
        assert_eq!(shadow_map_resolution(0, 1024), 512);
        assert_eq!(shadow_map_resolution(100, 1024), 128);
        assert_eq!(shadow_map_resolution(2048, 1024), 1024);
        assert_eq!(shadow_map_resolution(512, 0), 512);
    }

    #[test]
    fn shelf_pack_reports_overflow_as_none() {
        let slots = pack_shadow_atlas(&[(2048, 2048), (128, 128)], 2048);
        assert!(slots[0].is_some());
        assert_eq!(slots[1], None);
        let slots = pack_shadow_atlas(&[(0, 64), (64, 64)], 2048);
        assert_eq!(slots[0], None);
        assert_eq!(
            slots[1],
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0
            })
        );
    }

    #[test]
    fn cone_fov_matches_donor_values() {
        assert_eq!(shadow_cone_fov(1.0), 0.0);
        assert_eq!(shadow_cone_fov(0.0), 175.0);
        let narrow = shadow_cone_fov(0.999);
        assert!(narrow > 0.0 && narrow < 175.0);
    }

    #[test]
    fn cone_matrix_requires_a_spotlight() {
        let spot = fragment_light(&spot_light());
        assert!(shadow_cone_matrix(&spot).is_ok());
        let point = Q2FragmentLight { cone: None, ..spot };
        assert!(matches!(shadow_cone_matrix(&point), Err(RenderError::BadProjection(_))));
    }

    #[test]
    fn matrix_multiply_accumulates_before_rounding() {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let scale = [
            2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 4.0, 0.0, 5.0, 6.0, 7.0, 1.0,
        ];
        assert_eq!(shadow_matrix_multiply(&identity, &scale), scale);
        assert_eq!(shadow_project(&identity, vec3(1.0, 2.0, 3.0)), vec4(1.0, 2.0, 3.0, 1.0));
    }

    #[test]
    fn caster_radius_covers_positions() {
        let mesh = triangle_mesh();
        let caster = shadow_caster(vec3(0.0, 0.0, 0.0), vec![mesh]);
        assert!((caster.radius - 16.0).abs() < 1e-4);
        let empty = shadow_caster(vec3(0.0, 0.0, 0.0), Vec::new());
        assert_eq!(empty.radius, 64.0);
    }

    #[test]
    fn shadow_mesh_projects_material_geometry() {
        let geometry = MaterialGeometry {
            vertices: vec![MaterialVertex::new(
                vec3(1.0, 2.0, 3.0),
                vec3(0.0, 0.0, 1.0),
                vec2(0.0, 0.0),
                Vec2 { x: 0.0, y: 0.0 },
                [255, 255, 255, 255],
            )],
            indices: vec![7],
        };
        let mesh = shadow_mesh(&geometry);
        assert_eq!(mesh.positions, vec![vec3(1.0, 2.0, 3.0)]);
        assert_eq!(mesh.indices, vec![7]);
    }

    #[test]
    fn body_filter_matches_eligible_volumes() {
        let filter = shadow_body_filter(&[spot_light()]);
        assert!(filter(&ShadowSphere {
            origin: vec3(0.0, 0.0, 0.0),
            radius: 8.0
        }));
        assert!(!filter(&ShadowSphere {
            origin: vec3(0.0, 0.0, 5000.0),
            radius: 8.0
        }));
        let simple = SceneLight {
            profile: LightProfile::Simple,
            ..spot_light()
        };
        let none = shadow_body_filter(&[simple]);
        assert!(!none(&ShadowSphere {
            origin: vec3(0.0, 0.0, 0.0),
            radius: 8.0
        }));
    }

    #[test]
    fn digest_detects_geometry_changes() {
        let mut world = StaticShadowWorld::new(vec![triangle_mesh()]);
        let first = world.digest();
        assert_eq!(world.digest(), first);
        let mut changed = triangle_mesh();
        changed.positions[0] = vec3(1.0, 0.0, 0.0);
        let mut other = StaticShadowWorld::new(vec![changed]);
        assert_ne!(other.digest(), first);
    }

    #[test]
    fn prepare_without_lights_stays_empty() {
        let mut scene = Q2ShadowScene::new(owner());
        let prepared = scene.prepare(&[], ShadowWorldInput::Meshes(&[]), &[], &ShadowAtlasOptions::default());
        assert!(prepared.lighting.atlas.is_none());
        assert!(prepared.operations.is_empty());
        assert_eq!(prepared.stats, ShadowStats::default());
    }

    #[test]
    fn prepare_caches_repeat_work() {
        let mut scene = Q2ShadowScene::new(owner());
        let world = vec![triangle_mesh()];
        let options = ShadowAtlasOptions::default();
        let first = scene.prepare(&[spot_light()], ShadowWorldInput::Meshes(&world), &[], &options);
        assert!(first.lighting.atlas.is_some());
        assert_eq!(first.stats.rebuilt_lights, 1);
        assert_eq!(first.stats.faces_rendered, 1);
        assert_eq!(first.operations.len(), 1);
        let second = scene.prepare(&[spot_light()], ShadowWorldInput::Meshes(&world), &[], &options);
        assert_eq!(second.stats.cached_lights, 1);
        assert_eq!(second.stats.rebuilt_lights, 0);
        assert!(second.operations.is_empty());
        assert!(second.lighting.atlas.is_some());
    }

    #[test]
    fn disabled_prepare_releases_the_atlas() {
        let mut scene = Q2ShadowScene::new(owner());
        let world = vec![triangle_mesh()];
        let enabled = ShadowAtlasOptions::default();
        let first = scene.prepare(&[spot_light()], ShadowWorldInput::Meshes(&world), &[], &enabled);
        assert!(first.lighting.atlas.is_some());
        let disabled = ShadowAtlasOptions {
            enabled: false,
            resolution_cap: 1024,
        };
        let second = scene.prepare(&[spot_light()], ShadowWorldInput::Meshes(&world), &[], &disabled);
        assert!(second.lighting.atlas.is_none());
        assert!(second.operations.is_empty());
        assert_eq!(second.stats, ShadowStats::default());
    }
}
