//! Q3 generated entity geometry (donor
//! `src/render/scene/particles/primitives.ts`).
//!
//! `tr_surface.c` rail, poly, sprite, beam, and missing-model geometry.

use qa_core::math::{
    add3, cross3, dot3, length3, normalize3, normalize3_or_zero, perpendicular_vector, rotate_point_around_vector,
    scale3, sub3, vec2, vec3, vec4, Axis, Bounds, Vec3, Vec4,
};

use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DrawBatch, RenderState,
    RenderVertex, RendererImage, TextureBinding,
};

use super::super::models::transform::model_world_point;
use super::super::models::types::EntityTransform;

/// Rail, ring, or lightning pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailPose {
    /// Primitive kind.
    pub kind: RailKind,
    /// Rail origin.
    pub origin: Vec3,
    /// Rail endpoint.
    pub old_origin: Vec3,
    /// Shader color in byte range.
    pub shader_rgba: Vec4,
}

/// Rail primitive kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailKind {
    /// Rail core quad.
    RailCore,
    /// Rail ring segments.
    RailRings,
    /// Lightning bolt quads.
    Lightning,
}

/// Sprite pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpritePose {
    /// Center.
    pub origin: Vec3,
    /// Radius.
    pub radius: f32,
    /// Rotation in degrees.
    pub rotation: f32,
    /// Shader color in byte range.
    pub shader_rgba: Vec4,
}

/// Beam endpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeamPose {
    /// Beam origin.
    pub origin: Vec3,
    /// Beam endpoint.
    pub old_origin: Vec3,
}

/// Rail width settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailSettings {
    /// Core width.
    pub core_width: i32,
    /// Ring width.
    pub ring_width: i32,
    /// Ring segment length.
    pub segment_length: f32,
}

/// Default rail settings.
pub const DEFAULT_RAIL_SETTINGS: RailSettings = RailSettings {
    core_width: 6,
    ring_width: 16,
    segment_length: 32.0,
};

fn int32(value: f32, label: &str) -> Result<i32, RenderError> {
    if !value.is_finite() || value < i32::MIN as f32 || value > i32::MAX as f32 {
        return Err(RenderError::BadBatch {
            index: 0,
            detail: format!("{label} exceeds source int32 conversion"),
        });
    }
    Ok(value as i32)
}

/// `RB_SurfaceRailCore`/`RailRings`/`LightningBolt`, including integer length
/// truncation.
pub fn rail_geometry(
    entity: &RailPose,
    view_origin: Vec3,
    settings: &RailSettings,
) -> Result<MaterialGeometry, RenderError> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let (start, end) = if entity.kind == RailKind::Lightning {
        (entity.origin, entity.old_origin)
    } else {
        (entity.old_origin, entity.origin)
    };
    let delta = sub3(end, start);
    let direction = normalize3(delta);
    let length = int32(length3(delta).trunc(), "rail length")?;
    let vertex = |position: Vec3, s: f32, t: f32, dim: bool| {
        let pick = |channel: f32| if dim { (channel * 0.25).trunc() } else { channel };
        MaterialVertex::new(
            position,
            vec3(0.0, 0.0, 0.0),
            vec2(s, t),
            vec2(0.0, 0.0),
            [
                pick(entity.shader_rgba.x).clamp(0.0, 255.0) as u8,
                pick(entity.shader_rgba.y).clamp(0.0, 255.0) as u8,
                pick(entity.shader_rgba.z).clamp(0.0, 255.0) as u8,
                0,
            ],
        )
    };
    if entity.kind != RailKind::RailRings {
        let mut right = normalize3(cross3(
            normalize3(sub3(start, view_origin)),
            normalize3(sub3(end, view_origin)),
        ));
        let passes = if entity.kind == RailKind::Lightning { 4 } else { 1 };
        for _ in 0..passes {
            let width = if entity.kind == RailKind::Lightning {
                8
            } else {
                settings.core_width
            };
            int32(width as f32, "rail core width")?;
            let base = vertices.len() as u32;
            let offset = scale3(right, width as f32);
            let t = length as f32 / 256.0;
            vertices.push(vertex(add3(start, offset), 0.0, 0.0, true));
            vertices.push(vertex(sub3(start, offset), 0.0, 1.0, false));
            vertices.push(vertex(add3(end, offset), t, 0.0, false));
            vertices.push(vertex(sub3(end, offset), t, 1.0, false));
            indices.extend([base, base + 1, base + 2, base + 2, base + 1, base + 3]);
            if entity.kind == RailKind::Lightning {
                right = rotate_point_around_vector(direction, right, 45.0);
            }
        }
        return Ok(MaterialGeometry { vertices, indices });
    }
    // MakeNormalVectors uses a permutation before Gram-Schmidt.
    let seed = vec3(direction.z, -direction.x, direction.y);
    let right = normalize3(sub3(seed, scale3(direction, dot3(seed, direction))));
    let up = cross3(right, direction);
    let mut segments = int32((length as f32 / settings.segment_length).trunc(), "rail ring count")?;
    int32(settings.ring_width as f32, "rail ring width")?;
    if segments > 1_000_000 {
        return Err(RenderError::BadBatch {
            index: 0,
            detail: "rail ring geometry exceeds safe allocation".to_string(),
        });
    }
    if segments <= 0 {
        segments = 1;
    }
    let step = vec3(
        direction.x * settings.segment_length,
        direction.y * settings.segment_length,
        direction.z * settings.segment_length,
    );
    if segments > 1 {
        segments -= 1;
    }
    let mut positions = Vec::with_capacity(4);
    for index in 0..4 {
        let angle = (45 + index * 90) as f32 * std::f32::consts::PI / 180.0;
        let (sine, cosine) = angle.sin_cos();
        let offset = scale3(
            scale3(add3(scale3(right, cosine), scale3(up, sine)), 0.25),
            settings.ring_width as f32,
        );
        let position = add3(start, offset);
        positions.push(if segments > 1 { add3(position, step) } else { position });
    }
    for _ in 0..segments {
        let base = vertices.len() as u32;
        for (index, position) in positions.clone().iter().enumerate() {
            vertices.push(vertex(
                *position,
                if index < 2 { 1.0 } else { 0.0 },
                if index == 0 || index == 3 { 0.0 } else { 1.0 },
                false,
            ));
            positions[index] = add3(*position, step);
        }
        indices.extend([base, base + 1, base + 3, base + 3, base + 1, base + 2]);
    }
    Ok(MaterialGeometry { vertices, indices })
}

/// `RB_SurfacePolychain`: copy attributes and emit a triangle fan.
#[must_use]
pub fn poly_geometry(poly: &[PolyVertex]) -> MaterialGeometry {
    let vertices = poly
        .iter()
        .map(|vertex| {
            MaterialVertex::new(
                vertex.position,
                vec3(0.0, 0.0, 0.0),
                vertex.tex_coord,
                vec2(0.0, 0.0),
                vertex.color,
            )
        })
        .collect::<Vec<_>>();
    let mut indices = Vec::new();
    for index in 0..vertices.len().saturating_sub(2) {
        indices.extend([0, index as u32 + 1, index as u32 + 2]);
    }
    MaterialGeometry { vertices, indices }
}

/// Polychain input vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolyVertex {
    /// Position.
    pub position: Vec3,
    /// Texture coordinates.
    pub tex_coord: qa_core::math::Vec2,
    /// Byte color.
    pub color: [u8; 4],
}

/// Billboard sprite quad facing the view axes.
#[must_use]
pub fn sprite_geometry(entity: &SpritePose, view_axis: &Axis, mirror: bool) -> MaterialGeometry {
    let radius = entity.radius;
    let (mut left, up) = if entity.rotation == 0.0 {
        (scale3(view_axis[1], radius), scale3(view_axis[2], radius))
    } else {
        let angle = std::f32::consts::PI * entity.rotation / 180.0;
        let (sine, cosine) = angle.sin_cos();
        (
            add3(
                scale3(view_axis[1], cosine * radius),
                scale3(view_axis[2], -sine * radius),
            ),
            add3(
                scale3(view_axis[2], cosine * radius),
                scale3(view_axis[1], sine * radius),
            ),
        )
    };
    if mirror {
        left = scale3(left, -1.0);
    }
    let corner = |l: f32, u: f32| {
        vec3(
            entity.origin.x + l * left.x + u * up.x,
            entity.origin.y + l * left.y + u * up.y,
            entity.origin.z + l * left.z + u * up.z,
        )
    };
    let normal = scale3(view_axis[0], -1.0);
    let color = [
        entity.shader_rgba.x.clamp(0.0, 255.0) as u8,
        entity.shader_rgba.y.clamp(0.0, 255.0) as u8,
        entity.shader_rgba.z.clamp(0.0, 255.0) as u8,
        entity.shader_rgba.w.clamp(0.0, 255.0) as u8,
    ];
    let vertex = |position: Vec3, s: f32, t: f32| MaterialVertex::new(position, normal, vec2(s, t), vec2(s, t), color);
    MaterialGeometry {
        vertices: vec![
            vertex(corner(1.0, 1.0), 0.0, 0.0),
            vertex(corner(-1.0, 1.0), 1.0, 0.0),
            vertex(corner(-1.0, -1.0), 1.0, 1.0),
            vertex(corner(1.0, -1.0), 0.0, 1.0),
        ],
        indices: vec![0, 1, 3, 3, 1, 2],
    }
}

/// Fog volume index for a generated primitive's origin and radius.
#[must_use]
pub fn sprite_fog(origin: Vec3, radius: f32, bounds: &[Bounds]) -> Option<usize> {
    bounds.iter().position(|fog| {
        origin.x - radius < fog.max.x
            && origin.x + radius > fog.min.x
            && origin.y - radius < fog.max.y
            && origin.y + radius > fog.min.y
            && origin.z - radius < fog.max.z
            && origin.z + radius > fog.min.z
    })
}

/// Six-sided additive beam batch in projected space.
#[must_use]
pub fn beam_batch(
    entity: &BeamPose,
    project: &dyn Fn(Vec3) -> Vec4,
    previous: &RenderState,
    white_image: &RendererImage,
) -> DrawBatch {
    let direction = sub3(entity.old_origin, entity.origin);
    if length3(direction) == 0.0 {
        return DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: Vec::new(),
            texture: TextureBinding::BindImage(white_image.clone()),
            state: *previous,
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(Vec::new()),
        };
    }
    let normalized = normalize3_or_zero(direction);
    let perpendicular = scale3(perpendicular_vector(normalized), 4.0);
    let mut vertices = Vec::new();
    for index in 0..6 {
        let start = rotate_point_around_vector(normalized, perpendicular, f64::from(index * 60));
        for position in [start, add3(start, direction)] {
            vertices.push(RenderVertex {
                position: project(position),
                tex_coord: vec2(0.0, 0.0),
                color: vec4(1.0, 0.0, 0.0, 1.0),
            });
        }
    }
    let mut indices = Vec::new();
    for index in 0..12u32 {
        if index % 2 == 0 {
            indices.extend([index % 12, (index + 1) % 12, (index + 2) % 12]);
        } else {
            indices.extend([(index + 1) % 12, index % 12, (index + 2) % 12]);
        }
    }
    DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices,
        texture: TextureBinding::BindImage(white_image.clone()),
        state: RenderState {
            blend: (BlendFactor::One, BlendFactor::One),
            depth_test: crate::render::types::DepthTest::LessEqual,
            depth_write: false,
            alpha_test: AlphaTest::None,
            ..*previous
        },
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Triangles,
        vertices: BatchVertices::Single(vertices),
    }
}

/// Axis tripod drawn in place of a missing model.
#[must_use]
pub fn default_model_batch(
    entity: &EntityTransform,
    project: &dyn Fn(Vec3) -> Vec4,
    state: &RenderState,
    white_image: &RendererImage,
) -> DrawBatch {
    let colors = [
        vec4(1.0, 0.0, 0.0, 1.0),
        vec4(0.0, 1.0, 0.0, 1.0),
        vec4(0.0, 0.0, 1.0, 1.0),
    ];
    let mut vertices = Vec::new();
    for (index, color) in colors.iter().enumerate() {
        for local in [
            vec3(0.0, 0.0, 0.0),
            vec3(
                if index == 0 { 16.0 } else { 0.0 },
                if index == 1 { 16.0 } else { 0.0 },
                if index == 2 { 16.0 } else { 0.0 },
            ),
        ] {
            vertices.push(RenderVertex {
                position: project(model_world_point(entity, local)),
                tex_coord: vec2(0.0, 0.0),
                color: *color,
            });
        }
    }
    DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices: vec![0, 1, 2, 3, 4, 5],
        texture: TextureBinding::BindImage(white_image.clone()),
        state: *state,
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Lines { line_width: 3.0 },
        vertices: BatchVertices::Single(vertices),
    }
}

/// Cull face for generated primitives (always unculled).
#[must_use]
pub const fn primitive_cull() -> CullFace {
    CullFace::None
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec3, vec4};

    fn view_axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    #[test]
    fn rail_core_emits_one_quad() {
        let geometry = rail_geometry(
            &RailPose {
                kind: RailKind::RailCore,
                origin: vec3(32.0, 0.0, 0.0),
                old_origin: vec3(0.0, 0.0, 0.0),
                shader_rgba: vec4(255.0, 255.0, 255.0, 255.0),
            },
            vec3(0.0, -64.0, 32.0),
            &DEFAULT_RAIL_SETTINGS,
        )
        .expect("rail");
        assert_eq!(geometry.vertices.len(), 4);
        assert_eq!(geometry.indices.len(), 6);
    }

    #[test]
    fn lightning_emits_four_quads() {
        let geometry = rail_geometry(
            &RailPose {
                kind: RailKind::Lightning,
                origin: vec3(0.0, 0.0, 0.0),
                old_origin: vec3(32.0, 0.0, 0.0),
                shader_rgba: vec4(255.0, 255.0, 255.0, 255.0),
            },
            vec3(0.0, -64.0, 32.0),
            &DEFAULT_RAIL_SETTINGS,
        )
        .expect("bolt");
        assert_eq!(geometry.vertices.len(), 16);
        assert_eq!(geometry.indices.len(), 24);
    }

    #[test]
    fn rail_rings_emit_segments() {
        let geometry = rail_geometry(
            &RailPose {
                kind: RailKind::RailRings,
                origin: vec3(128.0, 0.0, 0.0),
                old_origin: vec3(0.0, 0.0, 0.0),
                shader_rgba: vec4(255.0, 255.0, 255.0, 255.0),
            },
            vec3(0.0, 0.0, 64.0),
            &DEFAULT_RAIL_SETTINGS,
        )
        .expect("rings");
        assert_eq!(geometry.vertices.len() % 4, 0);
        assert!(!geometry.vertices.is_empty());
        assert_eq!(geometry.indices.len() % 6, 0);
    }

    #[test]
    fn poly_fan_triangulates() {
        let quad = vec![
            PolyVertex {
                position: vec3(0.0, 0.0, 0.0),
                tex_coord: vec2(0.0, 0.0),
                color: [255; 4],
            },
            PolyVertex {
                position: vec3(1.0, 0.0, 0.0),
                tex_coord: vec2(1.0, 0.0),
                color: [255; 4],
            },
            PolyVertex {
                position: vec3(1.0, 1.0, 0.0),
                tex_coord: vec2(1.0, 1.0),
                color: [255; 4],
            },
            PolyVertex {
                position: vec3(0.0, 1.0, 0.0),
                tex_coord: vec2(0.0, 1.0),
                color: [255; 4],
            },
        ];
        let geometry = poly_geometry(&quad);
        assert_eq!(geometry.indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn sprite_quad_faces_view() {
        let geometry = sprite_geometry(
            &SpritePose {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 8.0,
                rotation: 0.0,
                shader_rgba: vec4(255.0, 255.0, 255.0, 255.0),
            },
            &view_axis(),
            false,
        );
        assert_eq!(geometry.vertices.len(), 4);
        assert_eq!(geometry.indices, vec![0, 1, 3, 3, 1, 2]);
        assert_eq!(geometry.vertices[0].normal, vec3(-1.0, 0.0, 0.0));
    }

    #[test]
    fn sprite_fog_selects_volume() {
        let bounds = vec![
            Bounds {
                min: vec3(-64.0, -64.0, -64.0),
                max: vec3(-32.0, -32.0, -32.0),
            },
            Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
        ];
        assert_eq!(sprite_fog(vec3(0.0, 0.0, 0.0), 4.0, &bounds), Some(1));
        assert_eq!(sprite_fog(vec3(100.0, 0.0, 0.0), 4.0, &bounds), None);
    }

    #[test]
    fn degenerate_beam_is_empty() {
        let batch = beam_batch(
            &BeamPose {
                origin: vec3(1.0, 1.0, 1.0),
                old_origin: vec3(1.0, 1.0, 1.0),
            },
            &|point| vec4(point.x, point.y, point.z, 1.0),
            &RenderState::opaque(CullFace::None),
            &test_image(),
        );
        match batch.vertices {
            BatchVertices::Single(vertices) => assert!(vertices.is_empty()),
            _ => panic!("expected single texturing"),
        }
    }

    #[test]
    fn beam_batch_has_twelve_vertices() {
        let batch = beam_batch(
            &BeamPose {
                origin: vec3(0.0, 0.0, 0.0),
                old_origin: vec3(0.0, 0.0, 32.0),
            },
            &|point| vec4(point.x, point.y, point.z, 1.0),
            &RenderState::opaque(CullFace::None),
            &test_image(),
        );
        match batch.vertices {
            BatchVertices::Single(vertices) => assert_eq!(vertices.len(), 12),
            _ => panic!("expected single texturing"),
        }
        assert_eq!(batch.indices.len(), 36);
    }

    fn test_image() -> RendererImage {
        let authority = qa_core::identity::IdentityOwner::create("test").expect("owner");
        RendererImage {
            owner: crate::render::types::ResourceOwner::new(1, authority.session().clone(), 0),
            ordinal: 1,
            source: crate::render::types::ImageSource::Generated {
                name: "white".to_string(),
            },
            width: 1,
            height: 1,
        }
    }
}
