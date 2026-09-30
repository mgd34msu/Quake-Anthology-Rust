//! Quake II damage-blend interpolation and vignette batches.
//!
//! Sync port of donor `src/app/bootstrap/q2-damage-blend.ts`
//! (`q2repro client/entities.c` blend lerp and `refresh/draw.c`
//! `GL_DrawVignette`). All arithmetic is `f32`, preserving the donor's
//! `Math.fround` behavior; the `& 255` byte wrap replicates JS `ToInt32`.

use qa_client::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DrawBatch, Rect, RenderState,
    RenderVertex, RendererImage, TextureBinding,
};
use qa_core::math::{Vec2, Vec4};

/// Donor default vignette border fraction (`fraction = 0.2`).
pub const DAMAGE_BLEND_BORDER: f32 = 0.2;

/// Interpolate a damage blend; a newly visible blend never fades in late.
#[must_use]
pub fn interpolate_q2_damage_blend(previous: Option<&Vec4>, current: &Vec4, fraction: f32) -> Vec4 {
    match previous {
        None => *current,
        Some(before) if before.w == 0.0 => *current,
        Some(before) => {
            let component = |past: f32, now: f32| past + ((now - past) * fraction);
            Vec4 {
                x: component(before.x, current.x),
                y: component(before.y, current.y),
                z: component(before.z, current.z),
                w: component(before.w, current.w),
            }
        }
    }
}

/// JS `ToInt32` via modulo 2^32 (`NaN`/infinities map to 0).
fn to_int32(value: f32) -> i32 {
    let narrowed = (f64::from(value) % 4_294_967_296.0 + 4_294_967_296.0) % 4_294_967_296.0;
    let signed = if narrowed >= 2_147_483_648.0 {
        narrowed - 4_294_967_296.0
    } else {
        narrowed
    };
    signed as i32
}

/// Donor `byte`: `(Math.trunc(Math.fround(v * 255)) & 255) / 255`.
fn blend_byte(value: f32) -> f32 {
    f32::from((to_int32((value * 255.0).trunc()) & 255) as u8) / 255.0
}

/// Build the vignette batch for a damage blend over `viewport`. Returns no
/// batch for a zero-alpha blend or an empty viewport.
#[must_use]
pub fn prepare_q2_damage_blend(blend: &Vec4, viewport: &Rect, white: RendererImage, fraction: f32) -> Vec<DrawBatch> {
    if blend.w == 0.0 || viewport.width <= 0.0 || viewport.height <= 0.0 {
        return Vec::new();
    }
    let outer = Vec4 {
        x: blend_byte(blend.x),
        y: blend_byte(blend.y),
        z: blend_byte(blend.z),
        w: blend_byte(blend.w),
    };
    let inner = Vec4 {
        x: outer.x,
        y: outer.y,
        z: outer.z,
        w: 0.0,
    };
    let width = viewport.width;
    let height = viewport.height;
    let vertex = |x: f32, y: f32, color: Vec4| RenderVertex {
        position: Vec4 {
            x: 2.0 * x / width - 1.0,
            y: 1.0 - 2.0 * y / height,
            z: 0.0,
            w: 1.0,
        },
        tex_coord: Vec2 { x: 0.0, y: 0.0 },
        color,
    };
    let mut vertices = vec![
        vertex(0.0, 0.0, outer),
        vertex(width, 0.0, outer),
        vertex(width, height, outer),
        vertex(0.0, height, outer),
    ];
    let mut indices = vec![0, 1, 2, 0, 2, 3];
    if fraction > 0.0 {
        let distance = (width.min(height) * fraction.min(0.5)).trunc();
        vertices.push(vertex(distance, distance, inner));
        vertices.push(vertex(width - distance, distance, inner));
        vertices.push(vertex(width - distance, height - distance, inner));
        vertices.push(vertex(distance, height - distance, inner));
        indices = vec![0, 5, 4, 0, 1, 5, 1, 6, 5, 1, 2, 6, 6, 2, 3, 6, 3, 7, 0, 7, 3, 0, 4, 7];
    }
    vec![DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices,
        texture: TextureBinding::BindImage(white),
        state: RenderState {
            blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
            depth_test: qa_client::render::types::DepthTest::Always,
            depth_write: false,
            alpha_test: AlphaTest::None,
            cull: CullFace::None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
        },
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Triangles,
        vertices: BatchVertices::Single(vertices),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::render::types::{DepthTest, ImageSource, ResourceOwner};
    use qa_core::identity::IdentityOwner;

    fn white() -> RendererImage {
        let owner = IdentityOwner::create("q2-blend").unwrap();
        RendererImage {
            owner: ResourceOwner::new(1, owner.session().clone(), 1),
            ordinal: 0,
            source: ImageSource::Generated {
                name: "white".to_string(),
            },
            width: 1,
            height: 1,
        }
    }

    fn viewport() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 320.0,
            height: 200.0,
        }
    }

    #[test]
    fn interpolation_skips_invisible_previous() {
        let current = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 0.5,
        };
        assert_eq!(interpolate_q2_damage_blend(None, &current, 0.5), current);
        let invisible = Vec4 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
            w: 0.0,
        };
        assert_eq!(interpolate_q2_damage_blend(Some(&invisible), &current, 0.5), current);
        let previous = Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        let mixed = interpolate_q2_damage_blend(Some(&previous), &current, 0.5);
        assert_eq!(mixed.x, 0.5);
        assert_eq!(mixed.w, 0.75);
    }

    #[test]
    fn empty_blend_or_viewport_yields_no_batch() {
        let clear = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 0.0,
        };
        assert!(prepare_q2_damage_blend(&clear, &viewport(), white(), DAMAGE_BLEND_BORDER).is_empty());
        let blend = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 0.5,
        };
        let flat = Rect {
            width: 0.0,
            ..viewport()
        };
        assert!(prepare_q2_damage_blend(&blend, &flat, white(), DAMAGE_BLEND_BORDER).is_empty());
    }

    #[test]
    fn zero_fraction_draws_a_full_quad() {
        let blend = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 0.5,
        };
        let batches = prepare_q2_damage_blend(&blend, &viewport(), white(), 0.0);
        assert_eq!(batches.len(), 1);
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("expected single-textured vertices");
        };
        assert_eq!(vertices.len(), 4);
        assert_eq!(batches[0].indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(batches[0].primitive, BatchPrimitive::Triangles);
        assert_eq!(batches[0].lighting, BatchLighting::Vertex);
        assert_eq!(batches[0].state.depth_test, DepthTest::Always);
        assert!(!batches[0].state.depth_write);
        assert_eq!(vertices[0].position.x, -1.0);
        assert_eq!(vertices[0].position.y, 1.0);
        assert_eq!(vertices[2].position.x, 1.0);
        assert_eq!(vertices[2].position.y, -1.0);
        assert_eq!(vertices[0].color.w, 127.0 / 255.0);
    }

    #[test]
    fn default_border_draws_a_vignette_ring() {
        let blend = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        let batches = prepare_q2_damage_blend(&blend, &viewport(), white(), DAMAGE_BLEND_BORDER);
        let BatchVertices::Single(vertices) = &batches[0].vertices else {
            panic!("expected single-textured vertices");
        };
        assert_eq!(vertices.len(), 8);
        assert_eq!(batches[0].indices.len(), 24);
        assert_eq!(vertices[4].color.w, 0.0);
        assert_eq!(vertices[0].color.w, 1.0);
        let expected = 2.0 * 40.0 / 320.0 - 1.0;
        assert!((vertices[4].position.x - expected).abs() < 1e-6);
    }
}
