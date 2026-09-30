//! Quake III presentation: marks.
//!
//! Donor provenance: `src/content/q3/presentation/marks.ts`.

use qa_core::math::{
    cross3, dot3, normalize3_or_zero, perpendicular_vector, rotate_point_around_vector, scale3, sub3, vec2, vec3, vec4,
    Vec3, Vec4,
};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mark_projector::*;
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::ref_entity::*;

// ---------------------------------------------------------------------------
// marks.ts
// ---------------------------------------------------------------------------

/// Maximum mark polygons.
pub const MAX_MARK_POLYS: usize = 256;

/// Maximum vertices per mark polygon.
pub const MAX_MARK_POLY_VERTICES: usize = 10;

/// Maximum mark fragments.
pub const MAX_MARK_FRAGMENTS: usize = 128;

/// Maximum mark points.
pub const MAX_MARK_POINTS: usize = 384;

/// Mark total lifetime milliseconds.
pub const MARK_TOTAL_TIME: i32 = 10000;

/// Mark fade time milliseconds.
pub const MARK_FADE_TIME: i32 = 1000;

/// Impact mark request (`ImpactMarkRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct ImpactMarkRequest {
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Orientation degrees.
    pub orientation: f32,
    /// Unit color.
    pub color: Vec4,
    /// Alpha fade.
    pub alpha_fade: bool,
    /// Radius.
    pub radius: f32,
    /// Temporary (returned, not stored).
    pub temporary: bool,
}

/// Impact mark options (`ImpactMarkOptions`).
pub trait ImpactMarkOptions {
    /// Clock milliseconds (signed 32-bit).
    fn clock(&self) -> i32;
    /// Marks enabled.
    fn enabled(&self) -> bool;
    /// Energy shader.
    fn energy_shader(&self) -> Option<SceneShader>;
}

/// Stored mark.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredMark {
    time: i32,
    shader: Option<SceneShader>,
    alpha_fade: bool,
    color: Vec4,
    vertices: Vec<RefPolyVertex>,
}

pub(crate) fn mark_finite(value: f32, name: &str) -> PresentResult<f32> {
    if !value.is_finite() {
        return Err(PresentError::range(format!("{name} must be finite float32")));
    }
    Ok(value)
}

pub(crate) fn mark_position(value: Vec3) -> PresentResult<Vec3> {
    Ok(vec3(
        mark_finite(value.x, "mark coordinate")?,
        mark_finite(value.y, "mark coordinate")?,
        mark_finite(value.z, "mark coordinate")?,
    ))
}

pub(crate) fn mark_byte(value: f32) -> f32 {
    ((value.trunc() as i32) & 255) as f32
}

pub(crate) fn mark_poly(shader: Option<SceneShader>, vertices: &[RefPolyVertex]) -> RefPoly {
    RefPoly {
        shader,
        vertices: vertices.to_vec(),
    }
}

pub(crate) fn mark_unit_color(value: f32) -> PresentResult<f32> {
    let result = mark_finite(value, "mark color")?;
    if !(0.0..=1.0).contains(&result) {
        return Err(PresentError::range("mark colors must be in [0, 1]"));
    }
    Ok(result)
}

/// Impact mark system (`ImpactMarkSystem`).
pub struct ImpactMarkSystem {
    /// Projector.
    projector: BspMarkProjector,
    /// Options.
    options: Box<dyn ImpactMarkOptions>,
    /// Active marks, newest first.
    active: Vec<StoredMark>,
}

impl ImpactMarkSystem {
    /// New system.
    pub fn new(projector: BspMarkProjector, options: Box<dyn ImpactMarkOptions>) -> Self {
        Self {
            projector,
            options,
            active: Vec::new(),
        }
    }

    /// Reset.
    pub fn reset(&mut self) {
        self.active.clear();
    }

    /// Active mark count.
    #[must_use]
    pub fn active_mark_count(&self) -> usize {
        self.active.len()
    }

    /// Add an impact mark (`impactMark`); returns temporary polys.
    pub fn impact_mark(&mut self, request: &ImpactMarkRequest) -> PresentResult<Vec<RefPoly>> {
        if !self.options.enabled() {
            return Ok(Vec::new());
        }
        let radius = mark_finite(request.radius, "mark radius")?;
        if radius <= 0.0 {
            return Err(PresentError::range("CG_ImpactMark called with <= 0 radius"));
        }
        let origin = mark_position(request.origin)?;
        let direction = mark_position(request.direction)?;
        if dot3(direction, direction) == 0.0 {
            return Ok(Vec::new());
        }
        let normal = normalize3_or_zero(direction);
        let axis2 = rotate_point_around_vector(
            normal,
            perpendicular_vector(normal),
            f64::from(mark_finite(request.orientation, "mark orientation")?),
        );
        let axis1 = cross3(normal, axis2);
        let scale = 0.5 / radius;
        let point = |first: f32, second: f32| {
            vec3(
                origin.x + first * (radius * axis1.x) + second * (radius * axis2.x),
                origin.y + first * (radius * axis1.y) + second * (radius * axis2.y),
                origin.z + first * (radius * axis1.z) + second * (radius * axis2.z),
            )
        };
        let fragments = self.projector.mark_fragments(&MarkProjection {
            points: vec![point(-1.0, -1.0), point(1.0, -1.0), point(1.0, 1.0), point(-1.0, 1.0)],
            projection: scale3(direction, -20.0),
            max_points: MAX_MARK_POINTS,
            max_fragments: MAX_MARK_FRAGMENTS,
        })?;
        let color = vec4(
            mark_unit_color(request.color.x)?,
            mark_unit_color(request.color.y)?,
            mark_unit_color(request.color.z)?,
            mark_unit_color(request.color.w)?,
        );
        let modulate = vec4(
            mark_byte(color.x * 255.0),
            mark_byte(color.y * 255.0),
            mark_byte(color.z * 255.0),
            mark_byte(color.w * 255.0),
        );
        let mut temporary = Vec::new();
        for fragment in &fragments.fragments {
            let end = fragment.first_point + fragment.point_count.min(MAX_MARK_POLY_VERTICES);
            let vertices: Vec<RefPolyVertex> = fragments.points[fragment.first_point..end.min(fragments.points.len())]
                .iter()
                .map(|point| {
                    let delta = sub3(*point, origin);
                    RefPolyVertex {
                        position: *point,
                        tex_coord: vec2(0.5 + dot3(delta, axis1) * scale, 0.5 + dot3(delta, axis2) * scale),
                        color: modulate,
                    }
                })
                .collect();
            if request.temporary {
                temporary.push(mark_poly(request.shader.clone(), &vertices));
                continue;
            }
            if self.active.len() == MAX_MARK_POLYS {
                let oldest = self
                    .active
                    .last()
                    .ok_or_else(|| PresentError::state("full mark pool has no oldest polygon"))?;
                let oldest_time = oldest.time;
                while self.active.last().is_some_and(|mark| mark.time == oldest_time) {
                    self.active.pop();
                }
            }
            let now = self.now();
            self.active.unshift_insert(StoredMark {
                time: now,
                shader: request.shader.clone(),
                alpha_fade: request.alpha_fade,
                color,
                vertices,
            });
        }
        Ok(temporary)
    }

    /// Emit active marks (`addMarks`).
    pub fn add_marks(&mut self) -> Vec<RefPoly> {
        if !self.options.enabled() {
            return Vec::new();
        }
        let mut output = Vec::new();
        let now = self.now();
        let mut index = 0;
        while index < self.active.len() {
            let expires = self.active[index].time.wrapping_add(MARK_TOTAL_TIME);
            if now > expires {
                self.active.remove(index);
                continue;
            }
            let is_energy = self.active[index].shader == self.options.energy_shader();
            if is_energy {
                let age = now.wrapping_sub(self.active[index].time) as f32;
                let fade = (450.0 - 450.0 * (age / 3000.0)).trunc() as i32;
                let first = self.active[index].vertices.first();
                if fade < 255 && first.is_some_and(|vertex| vertex.color.x != 0.0) {
                    let color = self.active[index].color;
                    let faded = fade.max(0) as f32;
                    let rgb = vec3(
                        mark_byte(color.x * faded),
                        mark_byte(color.y * faded),
                        mark_byte(color.z * faded),
                    );
                    for vertex in &mut self.active[index].vertices {
                        vertex.color.x = rgb.x;
                        vertex.color.y = rgb.y;
                        vertex.color.z = rgb.z;
                    }
                }
            }
            let remaining = expires.wrapping_sub(now);
            if remaining < MARK_FADE_TIME {
                let fade = (255i32.wrapping_mul(remaining) / MARK_FADE_TIME) & 255;
                if self.active[index].alpha_fade {
                    for vertex in &mut self.active[index].vertices {
                        vertex.color.w = fade as f32;
                    }
                } else {
                    let color = self.active[index].color;
                    let faded = fade as f32;
                    let rgb = vec3(
                        mark_byte(color.x * faded),
                        mark_byte(color.y * faded),
                        mark_byte(color.z * faded),
                    );
                    for vertex in &mut self.active[index].vertices {
                        vertex.color.x = rgb.x;
                        vertex.color.y = rgb.y;
                        vertex.color.z = rgb.z;
                    }
                }
            }
            let mark = &self.active[index];
            output.push(mark_poly(mark.shader.clone(), &mark.vertices));
            index += 1;
        }
        output
    }

    fn now(&self) -> i32 {
        self.options.clock()
    }
}

pub(crate) trait VecUnshift<T> {
    fn unshift_insert(&mut self, value: T);
}

impl<T> VecUnshift<T> for Vec<T> {
    fn unshift_insert(&mut self, value: T) {
        self.insert(0, value);
    }
}
