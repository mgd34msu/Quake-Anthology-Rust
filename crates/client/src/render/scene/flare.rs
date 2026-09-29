//! Lens flare billboard fan.
//!
//! Donor provenance: `src/render/scene/flare.ts` (`defaultFlareImage`,
//! `flareGeometry`, `prepareFlare`) and `src/contracts/flare.ts`
//! (`SceneFlare`). Colors stay byte-ranged until batch projection, matching
//! the donor's 0..255 flare colors.

use qa_core::math::{add3, cross3, dot3, length3, normalize3, scale3, sub3, vec2, vec3, vec4, Vec3};

use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest, DrawBatch, RenderState,
    RenderVertex, RendererImage, TextureBinding,
};
use crate::view::{project_point, SceneCamera};

/// Lens flare parameters (`SceneFlare`).
#[derive(Debug, Clone, PartialEq)]
pub struct Flare {
    /// Distance where the flare starts fading in.
    pub fade_start: f32,
    /// Distance where the flare reaches full alpha.
    pub fade_end: f32,
    /// Size multiplier.
    pub scale: f32,
    /// Keep the quad aligned to the camera instead of rotating it.
    pub lock_angle: bool,
    /// Center color, byte-ranged.
    pub color: Vec3,
    /// Rim color, byte-ranged; falls back to `color`.
    pub rim_color: Option<Vec3>,
}

/// Whether `path` names the standard flare sprite (full-size fan with the
/// luminance-alpha texture effect).
#[must_use]
pub fn default_flare_image(path: &str) -> bool {
    let name = path.to_lowercase().replace('\\', "/");
    name == "misc/flare.tga" || name.starts_with("sprites/psx_flare")
}

fn byte(value: f32) -> u8 {
    value.trunc().clamp(0.0, 255.0) as u8
}

/// Build the five-vertex flare fan in world space.
///
/// Returns empty geometry when the flare origin is nearer than `fade_start`.
#[must_use]
pub fn flare_geometry(flare: &Flare, origin: Vec3, camera: &SceneCamera, image_path: &str) -> MaterialGeometry {
    let delta = sub3(origin, camera.origin);
    let distance = length3(delta);
    if distance < flare.fade_start {
        return MaterialGeometry::empty();
    }
    let fraction = if distance >= flare.fade_end {
        1.0
    } else {
        (distance - flare.fade_start) / (flare.fade_end - flare.fade_start)
    };
    let standard = default_flare_image(image_path);
    let size = (if standard { 50.0 } else { 25.0 }) * flare.scale;
    let direction = normalize3(delta);
    let rotated = vec3(direction.z, -direction.x, direction.y);
    let right = if flare.lock_angle {
        scale3(camera.axis[1], -1.0)
    } else {
        normalize3(sub3(rotated, scale3(direction, dot3(rotated, direction))))
    };
    let up = if flare.lock_angle {
        camera.axis[2]
    } else {
        cross3(right, direction)
    };
    let alpha = byte((if standard { 160.0 } else { 128.0 }) * fraction);
    let rim = flare.rim_color.unwrap_or(flare.color);
    let vertex = |horizontal: f32, vertical: f32, s: f32, t: f32, color: Vec3| {
        MaterialVertex::new(
            add3(
                origin,
                add3(scale3(right, horizontal * size), scale3(up, vertical * size)),
            ),
            scale3(direction, -1.0),
            vec2(s, t),
            vec2(0.0, 0.0),
            [byte(color.x), byte(color.y), byte(color.z), alpha],
        )
    };
    MaterialGeometry {
        vertices: vec![
            vertex(0.0, 0.0, 0.5, 0.5, flare.color),
            vertex(-1.0, -1.0, 0.0, 1.0, rim),
            vertex(-1.0, 1.0, 0.0, 0.0, rim),
            vertex(1.0, 1.0, 1.0, 0.0, rim),
            vertex(1.0, -1.0, 1.0, 1.0, rim),
        ],
        indices: vec![0, 2, 3, 0, 3, 4, 0, 4, 1, 0, 1, 2],
    }
}

/// Project the flare fan into one additive draw batch.
pub fn prepare_flare(
    flare: &Flare,
    origin: Vec3,
    camera: &SceneCamera,
    image: &RendererImage,
    image_path: &str,
) -> Result<DrawBatch, RenderError> {
    let geometry = flare_geometry(flare, origin, camera, image_path);
    let mut vertices = Vec::with_capacity(geometry.vertices.len());
    for vertex in &geometry.vertices {
        let position =
            project_point(camera, None, vertex.position).map_err(|error| RenderError::Backend(error.to_string()))?;
        vertices.push(RenderVertex {
            position,
            tex_coord: vertex.tex_coord,
            color: vec4(
                f32::from(vertex.color[0]) / 255.0,
                f32::from(vertex.color[1]) / 255.0,
                f32::from(vertex.color[2]) / 255.0,
                f32::from(vertex.color[3]) / 255.0,
            ),
        });
    }
    Ok(DrawBatch {
        fog: None,
        luminance_alpha: default_flare_image(image_path),
        indices: geometry.indices,
        texture: TextureBinding::BindImage(image.clone()),
        state: RenderState {
            blend: (BlendFactor::SrcAlpha, BlendFactor::One),
            depth_test: DepthTest::LessEqual,
            depth_write: false,
            alpha_test: AlphaTest::None,
            cull: CullFace::None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
        },
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Triangles,
        vertices: BatchVertices::Single(vertices),
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, vec4};

    use crate::render::types::{fresh_owner_identity, ImageSource, ResourceOwner};
    use crate::view::{CameraClip, Rect};

    use super::*;

    fn test_camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn test_image() -> RendererImage {
        let authority = IdentityOwner::create("flare-test").unwrap();
        RendererImage {
            owner: ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0),
            ordinal: 3,
            source: ImageSource::Generated {
                name: "flare".to_string(),
            },
            width: 64,
            height: 64,
        }
    }

    fn test_flare() -> Flare {
        Flare {
            fade_start: 10.0,
            fade_end: 100.0,
            scale: 1.0,
            lock_angle: false,
            color: vec3(255.0, 128.0, 0.0),
            rim_color: Some(vec3(0.0, 0.0, 255.0)),
        }
    }

    #[test]
    fn near_flare_is_empty() {
        let camera = test_camera();
        let geometry = flare_geometry(&test_flare(), vec3(5.0, 0.0, 0.0), &camera, "misc/flare.tga");
        assert!(geometry.vertices.is_empty());
        assert!(geometry.indices.is_empty());
    }

    #[test]
    fn standard_fan_is_larger_with_higher_alpha() {
        let camera = test_camera();
        let flare = test_flare();
        let origin = vec3(200.0, 0.0, 0.0);
        let standard = flare_geometry(&flare, origin, &camera, "MISC\\FLARE.TGA");
        let custom = flare_geometry(&flare, origin, &camera, "sprites/custom.tga");
        assert_eq!(standard.vertices.len(), 5);
        assert_eq!(standard.indices.len(), 12);
        assert_eq!(standard.vertices[0].color[3], 160);
        assert_eq!(custom.vertices[0].color[3], 128);
        let standard_span = (standard.vertices[3].position.x - standard.vertices[1].position.x).abs()
            + (standard.vertices[3].position.y - standard.vertices[1].position.y).abs()
            + (standard.vertices[3].position.z - standard.vertices[1].position.z).abs();
        let custom_span = (custom.vertices[3].position.x - custom.vertices[1].position.x).abs()
            + (custom.vertices[3].position.y - custom.vertices[1].position.y).abs()
            + (custom.vertices[3].position.z - custom.vertices[1].position.z).abs();
        assert!((standard_span - 2.0 * custom_span).abs() < 1e-3);
        assert_eq!(standard.vertices[0].color[0..3], [255, 128, 0]);
        assert_eq!(standard.vertices[1].color[0..3], [0, 0, 255]);
    }

    #[test]
    fn rim_falls_back_to_color_and_alpha_fades() {
        let camera = test_camera();
        let mut flare = test_flare();
        flare.rim_color = None;
        let geometry = flare_geometry(&flare, vec3(55.0, 0.0, 0.0), &camera, "misc/flare.tga");
        assert_eq!(geometry.vertices[2].color[0..3], [255, 128, 0]);
        assert_eq!(geometry.vertices[0].color[3], 80);
    }

    #[test]
    fn batch_state_and_binding() {
        let camera = test_camera();
        let image = test_image();
        let batch = prepare_flare(&test_flare(), vec3(200.0, 0.0, 0.0), &camera, &image, "misc/flare.tga").unwrap();
        assert!(batch.luminance_alpha);
        assert_eq!(batch.texture, TextureBinding::BindImage(image));
        assert_eq!(batch.state.blend, (BlendFactor::SrcAlpha, BlendFactor::One));
        assert_eq!(batch.state.depth_test, DepthTest::LessEqual);
        assert!(!batch.state.depth_write);
        assert_eq!(batch.state.cull, CullFace::None);
        assert_eq!(batch.lighting, BatchLighting::Vertex);
        assert_eq!(batch.primitive, BatchPrimitive::Triangles);
        let BatchVertices::Single(vertices) = &batch.vertices else {
            panic!("flare batch must be single-textured");
        };
        assert_eq!(vertices.len(), 5);
        assert_eq!(vertices[0].color, vec4(1.0, 128.0 / 255.0, 0.0, 160.0 / 255.0));
        let plain = prepare_flare(
            &test_flare(),
            vec3(200.0, 0.0, 0.0),
            &camera,
            &test_image(),
            "other.tga",
        )
        .unwrap();
        assert!(!plain.luminance_alpha);
    }
}
