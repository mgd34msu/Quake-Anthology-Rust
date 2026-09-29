//! Q3 particle client types (donor `src/render/scene/particles/q3-types.ts`).

use qa_core::math::{Axis, Bounds, Vec2, Vec3, Vec4};

/// Registered particle shader. The caller supplies its actual registered
/// material identity; the name is retained for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParticleShader {
    /// Shader name.
    pub name: String,
    /// Registration handle, when the host assigns one.
    pub handle: Option<u32>,
}

impl ParticleShader {
    /// Shader with a name and no handle.
    #[must_use]
    pub fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            handle: None,
        }
    }
}

/// Shader registry available to the particle system.
pub trait ParticleResources {
    /// Register a shader by name, or `None` when missing.
    fn register_shader(&mut self, name: &str) -> Option<ParticleShader>;
}

/// One polygon vertex emitted by the particle system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RefPolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinates.
    pub tex_coord: Vec2,
    /// Byte color.
    pub color: Vec4,
}

/// One particle polygon with its shader.
#[derive(Debug, Clone, PartialEq)]
pub struct RefPoly {
    /// Polygon shader.
    pub shader: Option<ParticleShader>,
    /// Polygon vertices.
    pub vertices: Vec<RefPolyVertex>,
}

/// Client view state observed by the particle system.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleClientState {
    /// Client time in milliseconds.
    pub time: f32,
    /// View axes.
    pub view_axis: Axis,
    /// Snapshotted player origin, for distance culling.
    pub player_origin: Option<Vec3>,
}

/// Client entity state driving particle emission.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleClientEntity {
    /// Current origin.
    pub origin: Vec3,
    /// Secondary origin.
    pub origin2: Vec3,
    /// Current angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Primary time.
    pub time: i32,
    /// Secondary time.
    pub time2: i32,
    /// Frame number.
    pub frame: i32,
}

/// Trace result solidity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear trace.
    Clear,
    /// Started inside solid.
    StartSolid,
    /// Entirely inside solid.
    AllSolid,
}

/// Prediction trace result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleTrace {
    /// Trace end point.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Trace solidity.
    pub solidity: TraceSolidity,
    /// Completed fraction.
    pub fraction: f32,
}

/// Prediction tracer used for blood-pool validation.
pub trait ParticleTracer {
    /// Trace a swept box between two points.
    fn trace(&self, start: Vec3, end: Vec3, bounds: Bounds, pass_entity: i32, contents: i32) -> ParticleTrace;
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec2, vec3, vec4};

    #[test]
    fn shader_names_round_trip() {
        let shader = ParticleShader::named("blood1");
        assert_eq!(shader.name, "blood1");
        assert_eq!(shader.handle, None);
    }

    #[test]
    fn ref_poly_holds_vertices() {
        let poly = RefPoly {
            shader: Some(ParticleShader::named("smoke")),
            vertices: vec![RefPolyVertex {
                position: vec3(1.0, 2.0, 3.0),
                tex_coord: vec2(0.0, 1.0),
                color: vec4(255.0, 255.0, 255.0, 255.0),
            }],
        };
        assert_eq!(poly.vertices.len(), 1);
        assert_eq!(poly.shader.as_ref().expect("shader").name, "smoke");
    }

    #[test]
    fn trace_solidity_distinguishes_hits() {
        assert_ne!(TraceSolidity::Clear, TraceSolidity::AllSolid);
    }
}
