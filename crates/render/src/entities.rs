//! View-dependent entity geometry shared by both scene consumers.
use crate::{
    Vertex,
    scene::{Refdef, SceneEntity, SceneModel},
};
use qa_core::{math, primitives::Vec3};

pub fn register_beam_material(
    assets: &mut crate::Assets,
) -> Result<crate::MaterialId, &'static str> {
    use crate::{
        assets::{ImageId, MaterialSettings, Stage, StageTexture},
        shader::{AlphaGen, BlendFactor, Cull, RgbGen, StageBlend},
    };
    assets.register_material(
        "engine/beam",
        &[Stage {
            texture: StageTexture::Image(ImageId(0)),
            rgb_gen: RgbGen::ExactVertex,
            alpha_gen: AlphaGen::Vertex,
            blend: Some(StageBlend {
                source: BlendFactor::One,
                destination: BlendFactor::One,
            }),
            depth_write: false,
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..Default::default()
        },
    )
}

pub enum Prepared {
    Quad([Vertex; 4]),
    Beam {
        points: [Vec3; 6],
        direction: Vec3,
    },
    Core {
        start: Vec3,
        end: Vec3,
        right: Vec3,
        direction: Vec3,
        length: i32,
        width: i32,
        count: usize,
        color: [u8; 4],
    },
    Rings {
        points: [Vec3; 4],
        step: Vec3,
        count: usize,
        color: [u8; 4],
    },
    Empty,
}
impl Prepared {
    pub fn triangle_count(&self) -> usize {
        match self {
            Self::Quad(_) => 2,
            Self::Beam { .. } => 12,
            Self::Core { count, .. } | Self::Rings { count, .. } => count.saturating_mul(2),
            Self::Empty => 0,
        }
    }
    pub fn triangles(self, mut emit: impl FnMut([Vertex; 3])) {
        match self {
            Self::Quad(vertices) => quad(vertices, [[0, 1, 3], [3, 1, 2]], &mut emit),
            Self::Beam { points, direction } => {
                for index in 0..6 {
                    let a = points[index];
                    let b = points[(index + 1) % 6];
                    quad(
                        [
                            vertex(a, [0.; 2], [255, 0, 0, 255]),
                            vertex(a + direction, [0.; 2], [255, 0, 0, 255]),
                            vertex(b, [0.; 2], [255, 0, 0, 255]),
                            vertex(b + direction, [0.; 2], [255, 0, 0, 255]),
                        ],
                        [[0, 1, 2], [2, 1, 3]],
                        &mut emit,
                    );
                }
            }
            Self::Core {
                start,
                end,
                mut right,
                direction,
                length,
                width,
                count,
                mut color,
            } => {
                color[3] = 0; // Native rail calls leave this tess cell unwritten.
                let t = length as f32 / 256.;
                let mut dim = color;
                for channel in &mut dim[..3] {
                    *channel = (f32::from(*channel) * 0.25) as u8;
                }
                for _ in 0..count {
                    let offset = right * width as f32;
                    quad(
                        [
                            vertex(start + offset, [0., 0.], dim),
                            vertex(start - offset, [0., 1.], color),
                            vertex(end + offset, [t, 0.], color),
                            vertex(end - offset, [t, 1.], color),
                        ],
                        [[0, 1, 2], [2, 1, 3]],
                        &mut emit,
                    );
                    if count == 4 {
                        right = math::rotate_about_axis(direction, right, 45.);
                    }
                }
            }
            Self::Rings {
                mut points,
                step,
                count,
                mut color,
            } => {
                color[3] = 0;
                for _ in 0..count {
                    let vertices = std::array::from_fn(|index| {
                        vertex(
                            points[index],
                            [
                                u8::from(index < 2) as f32,
                                u8::from(index != 0 && index != 3) as f32,
                            ],
                            color,
                        )
                    });
                    quad(vertices, [[0, 1, 3], [3, 1, 2]], &mut emit);
                    for point in &mut points {
                        *point = *point + step;
                    }
                }
            }
            Self::Empty => {}
        }
    }
}
fn quad(vertices: [Vertex; 4], indices: [[usize; 3]; 2], emit: &mut impl FnMut([Vertex; 3])) {
    for triangle in indices {
        emit(triangle.map(|index| vertices[index]));
    }
}
fn vertex(position: Vec3, uv: [f32; 2], color: [u8; 4]) -> Vertex {
    Vertex {
        position,
        normal: Vec3::default(),
        texcoord: uv,
        lightmap_coord: [0.; 2],
        color,
    }
}

pub fn prepare(entity: &SceneEntity, view: Refdef) -> Option<Prepared> {
    Some(match entity.model {
        SceneModel::Sprite { radius, rotation } => {
            let (mut left, up) = if rotation == 0. {
                (view.axes[1] * radius, view.axes[2] * radius)
            } else {
                let angle = (std::f64::consts::PI * f64::from(rotation) / 180.) as f32;
                let sine = f64::from(angle).sin() as f32;
                let cosine = f64::from(angle).cos() as f32;
                (
                    view.axes[1] * (cosine * radius) + view.axes[2] * (-sine * radius),
                    view.axes[2] * (cosine * radius) + view.axes[1] * (sine * radius),
                )
            };
            if view.reflected_view {
                left = -left;
            }
            let corner = |l: f32, u: f32, uv| Vertex {
                position: (entity.origin + left * l) + up * u,
                normal: -view.axes[0],
                texcoord: uv,
                lightmap_coord: uv,
                color: entity.color,
            };
            Prepared::Quad([
                corner(1., 1., [0., 0.]),
                corner(-1., 1., [1., 0.]),
                corner(-1., -1., [1., 1.]),
                corner(1., -1., [0., 1.]),
            ])
        }
        SceneModel::Beam { end } => {
            let direction = end - entity.origin;
            let mut normalized = direction;
            if math::normalize(&mut normalized) == 0. {
                return Some(Prepared::Empty);
            }
            let perpendicular = math::perpendicular_vector(normalized) * 4.;
            // The source's origin addition is commented out; retain local-zero placement.
            Prepared::Beam {
                points: std::array::from_fn(|index| {
                    math::rotate_about_axis(normalized, perpendicular, (index * 60) as f32)
                }),
                direction,
            }
        }
        SceneModel::RailCore { end, .. } | SceneModel::Lightning { end } => {
            let lightning = matches!(entity.model, SceneModel::Lightning { .. });
            let (start, end) = if lightning {
                (entity.origin, end)
            } else {
                (end, entity.origin)
            };
            let mut direction = end - start;
            let distance = math::normalize(&mut direction);
            if distance == 0. {
                return Some(Prepared::Empty);
            }
            let length = distance as i32;
            let right = math::normalized(math::cross(
                math::normalized(start - view.origin),
                math::normalized(end - view.origin),
            ));
            let width = if let SceneModel::RailCore { width, .. } = entity.model {
                width
            } else {
                8
            };
            Prepared::Core {
                start,
                end,
                right,
                direction,
                length,
                width,
                count: if lightning { 4 } else { 1 },
                color: entity.color,
            }
        }
        SceneModel::RailRings {
            end,
            width,
            segment_length,
        } => {
            let mut direction = entity.origin - end;
            let length = math::normalize(&mut direction) as i32;
            let seed = Vec3([direction.0[2], -direction.0[0], direction.0[1]]);
            let right = math::normalized(seed - direction * seed.dot(direction));
            let up = math::cross(right, direction);
            let step = direction * segment_length;
            let count = ((length as f32 / segment_length) as i32).max(1);
            let count = if count > 1 { count - 1 } else { count };
            let points = std::array::from_fn(|index| {
                let angle = (45. + index as f32 * 90.) as f64 * std::f64::consts::PI / 180.;
                let cosine = angle.cos() as f32;
                let sine = angle.sin() as f32;
                let point = end + ((right * cosine + up * sine) * 0.25) * width as f32;
                if count > 1 { point + step } else { point }
            });
            Prepared::Rings {
                points,
                step,
                count: count as usize,
                color: entity.color,
            }
        }
        SceneModel::Mesh(_) | SceneModel::World { .. } => return None,
        SceneModel::Portal { .. } | SceneModel::PolyMarker => Prepared::Empty,
    })
}
