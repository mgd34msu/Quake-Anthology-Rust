//! Particle presentation through the ordinary image, material and vertex paths.
use crate::{
    assets::{
        Assets, Cull, Filter, MaterialId, MaterialSettings, Sampler, Stage, StageTexture, Vertex,
        Wrap,
    },
    scene::{ParticlePresentation, Refdef, SceneParticle},
    shader::{AlphaGen, RgbGen, StageBlend},
    surface_cache::PaletteLighting,
};

/// gl_rmisc.c's original 8x8 particle alpha mask, including its empty padding.
const POINT_MASK: [u8; 64] = [
    0, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 1, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// Register once at load, before either backend freezes the shared Assets.
pub fn register_point_material(
    assets: &mut Assets,
    name: &str,
) -> Result<MaterialId, &'static str> {
    let mut rgba = [255; 8 * 8 * 4];
    for (pixel, mask) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(POINT_MASK) {
        pixel[3] = mask * 255;
    }
    let image = assets.register_image(8, 8, &rgba)?;
    assets.register_material(
        name,
        &[Stage {
            texture: StageTexture::Image(image),
            sampler: Sampler {
                wrap: Wrap::Repeat,
                filter: Filter::Linear,
                mipmaps: false,
            },
            blend: Some(StageBlend::ALPHA),
            rgb_gen: RgbGen::ExactVertex,
            alpha_gen: AlphaGen::Vertex,
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    )
}

/// Fixed-capacity vertices borrowed by the existing GL dynamic upload path.
pub struct ParticleVertices {
    vertices: [Vertex; 4],
    count: usize,
}
impl ParticleVertices {
    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices[..self.count]
    }
}

/// GLQuake's triangle order and scale arithmetic, or a material-backed card.
/// The scene boundary has already admitted the descriptor's finite values.
pub fn gl_vertices(
    particle: SceneParticle,
    view: &Refdef,
    palette: &PaletteLighting,
) -> Option<ParticleVertices> {
    let mut color = palette.color(particle.color).to_le_bytes();
    color[3] = particle.alpha;
    let mut vertices = [Vertex {
        normal: -view.axes[0],
        color,
        ..Vertex::default()
    }; 4];
    let count = match particle.presentation {
        ParticlePresentation::Square { .. } => {
            let depth = (particle.position - view.origin).dot(view.axes[0]);
            // The original unsuffixed 0.004 expression promotes to double
            // before assigning its scale back to a float.
            let scale = if depth < 20.0 {
                1.0
            } else {
                (1.0 + f64::from(depth) * 0.004) as f32
            } * particle.size;
            let up = view.axes[2] * 1.5;
            let right = -view.axes[1] * 1.5;
            vertices[0].position = particle.position;
            vertices[1].position = particle.position + up * scale;
            vertices[2].position = particle.position + right * scale;
            vertices[0].texcoord = [0.0, 0.0];
            vertices[1].texcoord = [1.0, 0.0];
            vertices[2].texcoord = [0.0, 1.0];
            3
        }
        ParticlePresentation::Card => {
            let right = -view.axes[1] * (particle.size * 0.5);
            let up = view.axes[2] * (particle.size * 0.5);
            for (vertex, (position, texcoord)) in vertices.iter_mut().zip([
                (particle.position - right - up, [0.0, 0.0]),
                (particle.position + right - up, [1.0, 0.0]),
                (particle.position + right + up, [1.0, 1.0]),
                (particle.position - right + up, [0.0, 1.0]),
            ]) {
                vertex.position = position;
                vertex.texcoord = texcoord;
            }
            4
        }
    };
    vertices[..count]
        .iter()
        .all(|vertex| vertex.is_finite())
        .then_some(ParticleVertices { vertices, count })
}
