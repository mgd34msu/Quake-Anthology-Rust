//! Shared sky projection and fixed clipping scratch.
//!
//! Sources: quake/WinQuake/gl_warp.c EmitSkyPolys/R_InitSky;
//! quake/WinQuake/d_sky.c D_Sky_uv_To_st; quake-2/ref_gl/gl_warp.c
//! DrawSkyPolygon/ClipSkyPolygon/MakeSkyVec; quake-iii-arena/code/renderer/
//! tr_sky.c R_InitSkyTexCoords/MakeSkyVec/ClipSkyPolygon.
use qa_core::primitives::Vec3;

pub const LAYER_SIZE: usize = 128;
pub const CLOUD_SUBDIVISIONS: usize = 8;
pub const CLOUD_GRID_SIZE: usize = CLOUD_SUBDIVISIONS + 1;
const CLIP_VERTICES: usize = 64;
const CLIP_STAGES: usize = 6;
const ON_EPSILON: f32 = 0.1;
const SKY_CLIP: [Vec3; CLIP_STAGES] = [
    Vec3([1.0, 1.0, 0.0]),
    Vec3([1.0, -1.0, 0.0]),
    Vec3([0.0, -1.0, 1.0]),
    Vec3([0.0, 1.0, 1.0]),
    Vec3([1.0, 0.0, 1.0]),
    Vec3([-1.0, 0.0, 1.0]),
];
const ST_TO_VEC: [[i8; 3]; 6] = [
    [3, -1, 2],
    [-3, 1, 2],
    [1, 3, 2],
    [-1, -3, 2],
    [-2, -1, 3],
    [2, -1, -3],
];
const VEC_TO_ST: [[i8; 3]; 6] = [
    [-2, 3, 1],
    [2, 3, -1],
    [1, 3, 2],
    [-1, 3, -2],
    [-2, -1, 3],
    [-2, 1, -3],
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayeredSphere {
    pub flatten_z: f32,
    pub projected_scale: f32,
    pub texture_size: f32,
    pub scroll_speeds: [f32; 2],
}
impl LayeredSphere {
    pub const NATIVE: Self = Self {
        flatten_z: 3.0,
        projected_scale: 6.0 * 63.0,
        texture_size: 128.0,
        scroll_speeds: [8.0, 16.0],
    };
    pub fn valid(self) -> bool {
        [self.flatten_z, self.projected_scale, self.texture_size]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
            && self.scroll_speeds.iter().all(|v| v.is_finite())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudSphere {
    /// Native Q3 has an 8192-unit diameter, radiusWorld=4096.
    pub radius: f32,
    pub height: f32,
}
impl CloudSphere {
    pub fn native(height: f32) -> Self {
        Self {
            radius: 4096.0,
            height,
        }
    }
    pub fn valid(self) -> bool {
        self.radius.is_finite() && self.radius > 0.0 && self.height.is_finite() && self.height > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation {
    /// Normalized once at loading, independently of the material's origin.
    pub axis: Vec3,
    pub degrees_per_second: f32,
}

/// Native flattened sphere UV. Direction is world point minus view origin for
/// GL vertices, or the corresponding world-space ray for software spans.
pub fn sphere_uv(
    direction: Vec3,
    time_seconds: f32,
    flatten_z: f32,
    projected_scale: f32,
    texture_size: f32,
    scroll_speed: f32,
) -> Option<[f32; 2]> {
    if !direction.0.iter().all(|f| f.is_finite())
        || ![
            time_seconds,
            flatten_z,
            projected_scale,
            texture_size,
            scroll_speed,
        ]
        .iter()
        .all(|f| f.is_finite())
        || flatten_z <= 0.0
        || projected_scale <= 0.0
        || texture_size <= 0.0
    {
        return None;
    }
    let direction = Vec3([direction.0[0], direction.0[1], direction.0[2] * flatten_z]);
    let length = direction.dot(direction).sqrt();
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    let scroll = time_seconds * scroll_speed;
    if !scroll.is_finite() {
        return None;
    }
    // Equivalent to native speedscale -= (int)speedscale & ~127 for its
    // 128-texel layer, including the fractional part of negative shader times.
    let scroll = scroll - (scroll.trunc() / texture_size).floor() * texture_size;
    let scale = projected_scale / length;
    let uv = [
        (scroll + direction.0[0] * scale) / texture_size,
        (scroll + direction.0[1] * scale) / texture_size,
    ];
    uv.iter().all(|value| value.is_finite()).then_some(uv)
}

pub fn layered_uv(
    direction: Vec3,
    time_seconds: f32,
    sphere: LayeredSphere,
    layer: usize,
) -> Option<[f32; 2]> {
    sphere_uv(
        direction,
        time_seconds,
        sphere.flatten_z,
        sphere.projected_scale,
        sphere.texture_size,
        *sphere.scroll_speeds.get(layer)?,
    )
}

/// Undo the cube's rotation when looking up a direction; rendered cube vertices
/// use the forward angle. The load-time axis is normalized before this call.
pub fn unrotate(direction: Vec3, rotation: Rotation, time_seconds: f32) -> Vec3 {
    let angle = -(time_seconds * rotation.degrees_per_second).to_radians();
    let (sine, cosine) = angle.sin_cos();
    let cross = Vec3([
        rotation.axis.0[1] * direction.0[2] - rotation.axis.0[2] * direction.0[1],
        rotation.axis.0[2] * direction.0[0] - rotation.axis.0[0] * direction.0[2],
        rotation.axis.0[0] * direction.0[1] - rotation.axis.0[1] * direction.0[0],
    ]);
    direction * cosine
        + cross * sine
        + rotation.axis * (rotation.axis.dot(direction) * (1.0 - cosine))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CubeFace {
    PositiveX,
    NegativeX,
    PositiveY,
    NegativeY,
    PositiveZ,
    NegativeZ,
}
impl CubeFace {
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];
    pub fn index(self) -> usize {
        self as usize
    }
    /// Load suffixes in directional order. Native file order rt,bk,lf,ft,up,dn
    /// becomes this order through sky_texorder={0,2,1,3,4,5}.
    pub fn suffix(self) -> &'static str {
        ["rt", "lf", "bk", "ft", "up", "dn"][self.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeSample {
    pub face: CubeFace,
    pub uv: [f32; 2],
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeVertex {
    pub direction: Vec3,
    pub uv: [f32; 2],
}

pub fn cube_vertex(
    face: CubeFace,
    st: [f32; 2],
    distance: f32,
    texcoord_range: [f32; 2],
) -> CubeVertex {
    let base = [st[0] * distance, st[1] * distance, distance];
    let direction = Vec3(ST_TO_VEC[face.index()].map(|axis| component(base, axis)));
    CubeVertex {
        direction,
        uv: box_uv(st, texcoord_range),
    }
}

pub fn cube_sample(direction: Vec3, texcoord_range: [f32; 2]) -> Option<CubeSample> {
    if !direction.0.iter().all(|value| value.is_finite())
        || !texcoord_range.iter().all(|value| value.is_finite())
        || texcoord_range[0] > texcoord_range[1]
    {
        return None;
    }
    let abs = direction.0.map(f32::abs);
    // Exact edge/corner rays select a stable valid face. Native clipped polygon
    // bounds use their averaged direction and the original strict comparisons.
    let face = if abs[0] >= abs[1] && abs[0] >= abs[2] {
        if direction.0[0] < 0.0 {
            CubeFace::NegativeX
        } else {
            CubeFace::PositiveX
        }
    } else if abs[1] >= abs[2] {
        if direction.0[1] < 0.0 {
            CubeFace::NegativeY
        } else {
            CubeFace::PositiveY
        }
    } else if direction.0[2] < 0.0 {
        CubeFace::NegativeZ
    } else {
        CubeFace::PositiveZ
    };
    let st = project_face(direction, face)?;
    Some(CubeSample {
        face,
        uv: box_uv(st, texcoord_range),
    })
}

fn component(vector: [f32; 3], axis: i8) -> f32 {
    if axis < 0 {
        -vector[(-axis - 1) as usize]
    } else {
        vector[(axis - 1) as usize]
    }
}
fn project_face(direction: Vec3, face: CubeFace) -> Option<[f32; 2]> {
    let map = VEC_TO_ST[face.index()];
    let divisor = component(direction.0, map[2]);
    if divisor < 0.001 {
        return None;
    }
    let st = [
        component(direction.0, map[0]) / divisor,
        component(direction.0, map[1]) / divisor,
    ];
    st.iter().all(|value| value.is_finite()).then_some(st)
}
fn box_uv(st: [f32; 2], texcoord_range: [f32; 2]) -> [f32; 2] {
    let uv = st.map(|value| {
        ((value + 1.0) * 0.5)
            .max(texcoord_range[0])
            .min(texcoord_range[1])
    });
    [uv[0], 1.0 - uv[1]]
}

/// Q3's cloud layer is a sphere centered radius units below the view. Its
/// coordinates are acos of the normalized intersection's X and Y components.
pub fn cloud_uv(direction: Vec3, sphere: CloudSphere) -> Option<[f32; 2]> {
    let (_, uv) = cloud_intersection(direction, sphere)?;
    Some(uv)
}
fn cloud_intersection(direction: Vec3, sphere: CloudSphere) -> Option<(f32, [f32; 2])> {
    if !sphere.valid() || !direction.0.iter().all(|value| value.is_finite()) {
        return None;
    }
    let length = direction.dot(direction);
    if length <= 0.0 || !length.is_finite() {
        return None;
    }
    let radius = sphere.radius;
    let height = sphere.height;
    let discriminant = direction.0[2] * direction.0[2] * radius * radius
        + length * (2.0 * radius * height + height * height);
    let p = (-direction.0[2] * radius + discriminant.sqrt()) / length;
    if !p.is_finite() || p < 0.0 {
        return None;
    }
    let mut intersection = direction * p;
    intersection.0[2] += radius;
    let norm = intersection.dot(intersection).sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        return None;
    }
    intersection = intersection / norm;
    let uv = [
        intersection.0[0].clamp(-1.0, 1.0).acos(),
        intersection.0[1].clamp(-1.0, 1.0).acos(),
    ];
    Some((p, uv))
}

/// Generated once per cloud height, matching Q3's six 9x9 lookup grids. Cloud
/// drawing omits NegativeZ in stock presentation; generation retains all faces.
pub struct CloudGrid {
    pub sphere: CloudSphere,
    pub uv: [[[[f32; 2]; CLOUD_GRID_SIZE]; CLOUD_GRID_SIZE]; 6],
    pub intersection: [[[f32; CLOUD_GRID_SIZE]; CLOUD_GRID_SIZE]; 6],
}
impl CloudGrid {
    pub fn generate(sphere: CloudSphere) -> Result<Self, &'static str> {
        if !sphere.valid() {
            return Err("invalid sky cloud sphere");
        }
        let mut result = Self {
            sphere,
            uv: [[[[0.0; 2]; CLOUD_GRID_SIZE]; CLOUD_GRID_SIZE]; 6],
            intersection: [[[0.0; CLOUD_GRID_SIZE]; CLOUD_GRID_SIZE]; 6],
        };
        for face in CubeFace::ALL {
            for t in 0..CLOUD_GRID_SIZE {
                for s in 0..CLOUD_GRID_SIZE {
                    let st = [(s as f32 - 4.0) / 4.0, (t as f32 - 4.0) / 4.0];
                    let direction = cube_vertex(face, st, 1024.0 / 1.75, [0.0, 1.0]).direction;
                    let (p, uv) = cloud_intersection(direction, sphere)
                        .ok_or("sky cloud projection failed")?;
                    result.uv[face.index()][t][s] = uv;
                    result.intersection[face.index()][t][s] = p;
                }
            }
        }
        Ok(result)
    }
}

/// Load-time split preserves native indices for software sampling. The masked
/// half's index zero has opaque-half average RGB and alpha zero to avoid fringe.
pub struct LayeredImages {
    pub opaque_indices: Box<[u8]>,
    pub masked_indices: Box<[u8]>,
    pub opaque_rgba: Box<[u8]>,
    pub masked_rgba: Box<[u8]>,
    pub average_rgb: [u8; 3],
}
pub fn split_layered_sky(
    indices: &[u8],
    palette: &[[u8; 4]; 256],
) -> Result<LayeredImages, &'static str> {
    if indices.len() != LAYER_SIZE * LAYER_SIZE * 2 {
        return Err("native layered sky requires 256x128 indices");
    }
    let count = LAYER_SIZE * LAYER_SIZE;
    let mut opaque_indices = vec![0; count].into_boxed_slice();
    let mut masked_indices = vec![0; count].into_boxed_slice();
    let mut opaque_rgba = vec![0; count * 4].into_boxed_slice();
    let mut masked_rgba = vec![0; count * 4].into_boxed_slice();
    let mut sum = [0_u32; 3];
    for y in 0..LAYER_SIZE {
        for x in 0..LAYER_SIZE {
            let out = y * LAYER_SIZE + x;
            let index = indices[y * LAYER_SIZE * 2 + x + LAYER_SIZE];
            opaque_indices[out] = index;
            let color = palette[index as usize];
            opaque_rgba[out * 4..out * 4 + 4].copy_from_slice(&[color[0], color[1], color[2], 255]);
            for channel in 0..3 {
                sum[channel] += u32::from(color[channel]);
            }
        }
    }
    let average_rgb = sum.map(|value| (value / count as u32) as u8);
    for y in 0..LAYER_SIZE {
        for x in 0..LAYER_SIZE {
            let out = y * LAYER_SIZE + x;
            let index = indices[y * LAYER_SIZE * 2 + x];
            masked_indices[out] = index;
            let color = if index == 0 {
                [average_rgb[0], average_rgb[1], average_rgb[2], 0]
            } else {
                palette[index as usize]
            };
            masked_rgba[out * 4..out * 4 + 4].copy_from_slice(&color);
        }
    }
    Ok(LayeredImages {
        opaque_indices,
        masked_indices,
        opaque_rgba,
        masked_rgba,
        average_rgb,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceBounds {
    pub mins: [f32; 2],
    pub maxs: [f32; 2],
}
impl FaceBounds {
    pub const EMPTY: Self = Self {
        mins: [9999.0; 2],
        maxs: [-9999.0; 2],
    };
    pub fn visible(self) -> bool {
        self.mins[0] < self.maxs[0] && self.mins[1] < self.maxs[1]
    }
    /// Native Q3 rounds outward onto the fixed eight-division face grid.
    pub fn grid_bounds(self) -> Option<[[usize; 2]; 2]> {
        if !self.visible() {
            return None;
        }
        let mins = self
            .mins
            .map(|value| ((value * 4.0).floor().clamp(-4.0, 4.0) + 4.0) as usize);
        let maxs = self
            .maxs
            .map(|value| ((value * 4.0).ceil().clamp(-4.0, 4.0) + 4.0) as usize);
        (mins[0] < maxs[0] && mins[1] < maxs[1]).then_some([mins, maxs])
    }
}

#[derive(Clone, Copy)]
struct ClipPolygon {
    vertices: [Vec3; CLIP_VERTICES],
    count: usize,
    stage: usize,
}
impl Default for ClipPolygon {
    fn default() -> Self {
        Self {
            vertices: [Vec3([0.0; 3]); CLIP_VERTICES],
            count: 0,
            stage: 0,
        }
    }
}

pub struct SkyClip {
    bounds: [FaceBounds; 6],
    pub rejected: u64,
}
impl Default for SkyClip {
    fn default() -> Self {
        Self::new()
    }
}
impl SkyClip {
    pub fn new() -> Self {
        Self {
            bounds: [FaceBounds::EMPTY; 6],
            rejected: 0,
        }
    }
    pub fn clear(&mut self) {
        self.bounds.fill(FaceBounds::EMPTY);
        self.rejected = 0;
    }
    pub fn bounds(&self) -> &[FaceBounds; 6] {
        &self.bounds
    }
    /// World-space triangles or convex polygons. All clipping work and bounds
    /// updates are fixed scratch; a failed polygon leaves previous bounds intact.
    pub fn add_polygon(&mut self, vertices: &[Vec3], view_origin: Vec3) -> bool {
        if vertices.len() < 3
            || vertices.len() > CLIP_VERTICES - 2
            || !view_origin.0.iter().all(|v| v.is_finite())
            || !vertices.iter().all(|v| v.0.iter().all(|c| c.is_finite()))
        {
            self.rejected += 1;
            return false;
        }
        let mut polygon = ClipPolygon {
            count: vertices.len(),
            ..ClipPolygon::default()
        };
        for (out, &vertex) in polygon.vertices.iter_mut().zip(vertices) {
            *out = vertex - view_origin;
            if !out.0.iter().all(|v| v.is_finite()) {
                self.rejected += 1;
                return false;
            }
        }
        let mut bounds = self.bounds;
        let mut pending = [ClipPolygon::default(); CLIP_STAGES + 1];
        pending[0] = polygon;
        let mut pending_count = 1;
        while pending_count != 0 {
            pending_count -= 1;
            let mut polygon = pending[pending_count];
            loop {
                if polygon.count > CLIP_VERTICES - 2 {
                    self.rejected += 1;
                    return false;
                }
                if polygon.stage == CLIP_STAGES {
                    if !add_face_bounds(&mut bounds, &polygon.vertices[..polygon.count]) {
                        self.rejected += 1;
                        return false;
                    }
                    break;
                }
                let mut distance = [0.0; CLIP_VERTICES];
                let mut sides = [0_i8; CLIP_VERTICES];
                let mut front = false;
                let mut back = false;
                for index in 0..polygon.count {
                    let d = polygon.vertices[index].dot(SKY_CLIP[polygon.stage]);
                    if !d.is_finite() {
                        self.rejected += 1;
                        return false;
                    }
                    distance[index] = d;
                    sides[index] = if d > ON_EPSILON {
                        front = true;
                        1
                    } else if d < -ON_EPSILON {
                        back = true;
                        -1
                    } else {
                        0
                    };
                }
                if !front || !back {
                    polygon.stage += 1;
                    continue;
                }
                let mut split = [ClipPolygon {
                    stage: polygon.stage + 1,
                    ..ClipPolygon::default()
                }; 2];
                for index in 0..polygon.count {
                    let vertex = polygon.vertices[index];
                    if sides[index] >= 0 && !append_vertex(&mut split[0], vertex) {
                        self.rejected += 1;
                        return false;
                    }
                    if sides[index] <= 0 && !append_vertex(&mut split[1], vertex) {
                        self.rejected += 1;
                        return false;
                    }
                    let next = (index + 1) % polygon.count;
                    if sides[index] == 0 || sides[next] == 0 || sides[index] == sides[next] {
                        continue;
                    }
                    let fraction = distance[index] / (distance[index] - distance[next]);
                    let crossing = vertex + (polygon.vertices[next] - vertex) * fraction;
                    if !crossing.0.iter().all(|v| v.is_finite())
                        || !append_vertex(&mut split[0], crossing)
                        || !append_vertex(&mut split[1], crossing)
                    {
                        self.rejected += 1;
                        return false;
                    }
                }
                if pending_count == pending.len() {
                    self.rejected += 1;
                    return false;
                }
                pending[pending_count] = split[1];
                pending_count += 1;
                polygon = split[0];
            }
        }
        self.bounds = bounds;
        true
    }
}
fn append_vertex(polygon: &mut ClipPolygon, vertex: Vec3) -> bool {
    if polygon.count == CLIP_VERTICES {
        return false;
    }
    polygon.vertices[polygon.count] = vertex;
    polygon.count += 1;
    true
}
#[expect(
    clippy::needless_range_loop,
    reason = "Keep tr_sky.c AddSkyPolygon numeric s/t axis correspondence when updating face bounds."
)]
fn add_face_bounds(bounds: &mut [FaceBounds; 6], vertices: &[Vec3]) -> bool {
    let mut direction = Vec3([0.0; 3]);
    for &vertex in vertices {
        direction = direction + vertex;
    }
    if !direction.0.iter().all(|value| value.is_finite()) {
        return false;
    }
    let abs = direction.0.map(f32::abs);
    let face = if abs[0] > abs[1] && abs[0] > abs[2] {
        if direction.0[0] < 0.0 {
            CubeFace::NegativeX
        } else {
            CubeFace::PositiveX
        }
    } else if abs[1] > abs[2] && abs[1] > abs[0] {
        if direction.0[1] < 0.0 {
            CubeFace::NegativeY
        } else {
            CubeFace::PositiveY
        }
    } else if direction.0[2] < 0.0 {
        CubeFace::NegativeZ
    } else {
        CubeFace::PositiveZ
    };
    for &vertex in vertices {
        if let Some(st) = project_face(vertex, face) {
            for axis in 0..2 {
                bounds[face.index()].mins[axis] = bounds[face.index()].mins[axis].min(st[axis]);
                bounds[face.index()].maxs[axis] = bounds[face.index()].maxs[axis].max(st[axis]);
            }
        }
    }
    true
}
