//! Legacy Q1/Q2 particles and beams (donor
//! `src/render/scene/particles/legacy.ts`).
//!
//! Quake `r_part.c` and Quake II `gl_rmain.c` particle and beam assembly.

use qa_core::math::{
    add3, length3, normalize3, perpendicular_vector, rotate_point_around_vector, scale3, sub3, vec2, vec3, vec4, Vec3,
    Vec4,
};

use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DrawBatch, ImageLevel, RenderImage,
    RenderState, RenderVertex, RendererImage, TextureBinding,
};
use crate::view::{CameraClip, SceneCamera};

use super::primitives::{sprite_geometry, SpritePose};

/// One submitted scene particle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SceneParticle {
    /// True-color billboard.
    Rgba {
        /// Center.
        origin: Vec3,
        /// Size in world units.
        size: f32,
        /// Rotation in degrees.
        rotation: f32,
        /// Normalized color.
        color: Vec4,
    },
    /// Indexed palette triangle.
    Indexed {
        /// Palette index.
        palette_index: u8,
        /// Opacity.
        alpha: f32,
        /// Size scale.
        size: f32,
        /// Center.
        origin: Vec3,
    },
}

/// Indexed particle profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexedProfile {
    /// Quake profile (opaque, depth-writing).
    Q1,
    /// Quake II profile (alpha-blended).
    Q2,
}

/// Particle preparation context.
pub struct ParticlePreparationContext<'a> {
    /// View camera.
    pub camera: SceneCamera,
    /// Indexed profile.
    pub indexed_profile: IndexedProfile,
    /// Palette byte colors.
    pub palette_color: &'a dyn Fn(u8) -> Vec3,
}

/// Assemble particle billboards and indexed triangles in world space.
#[must_use]
pub fn prepare_particle_geometry(
    particles: &[SceneParticle],
    context: &ParticlePreparationContext,
) -> MaterialGeometry {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for particle in particles {
        let offset = vertices.len() as u32;
        match *particle {
            SceneParticle::Rgba {
                origin,
                size,
                rotation,
                color,
            } => {
                let mirror = matches!(context.camera.clip, CameraClip::Portal { mirror: true, .. });
                let geometry = sprite_geometry(
                    &SpritePose {
                        origin,
                        radius: size,
                        rotation,
                        shader_rgba: vec4(color.x * 255.0, color.y * 255.0, color.z * 255.0, color.w * 255.0),
                    },
                    &context.camera.axis,
                    mirror,
                );
                vertices.extend(geometry.vertices);
                indices.extend(geometry.indices.iter().map(|index| index + offset));
            }
            SceneParticle::Indexed {
                palette_index,
                alpha,
                size,
                origin,
            } => {
                let delta = sub3(origin, context.camera.origin);
                let forward = context.camera.axis[0];
                let depth = delta.x * forward.x + delta.y * forward.y + delta.z * forward.z;
                let scale = (if depth < 20.0 { 1.0 } else { 1.0 + depth * 0.004 }) * size;
                let palette = (context.palette_color)(palette_index);
                let alpha_byte = match context.indexed_profile {
                    IndexedProfile::Q1 => 255,
                    IndexedProfile::Q2 => (alpha * 255.0).trunc() as i32 & 255,
                } as u8;
                let color = [
                    palette.x.clamp(0.0, 255.0) as u8,
                    palette.y.clamp(0.0, 255.0) as u8,
                    palette.z.clamp(0.0, 255.0) as u8,
                    alpha_byte,
                ];
                let normal = scale3(forward, -1.0);
                let up = scale3(context.camera.axis[2], 1.5 * scale);
                let right = scale3(context.camera.axis[1], -1.5 * scale);
                let uv = if context.indexed_profile == IndexedProfile::Q2 {
                    0.0625
                } else {
                    0.0
                };
                let vertex = |position: Vec3, s: f32, t: f32| {
                    MaterialVertex::new(position, normal, vec2(s, t), vec2(0.0, 0.0), color)
                };
                vertices.push(vertex(origin, uv, uv));
                vertices.push(vertex(add3(origin, up), 1.0 + uv, uv));
                vertices.push(vertex(add3(origin, right), uv, 1.0 + uv));
                indices.extend([offset, offset + 1, offset + 2]);
            }
        }
    }
    MaterialGeometry { vertices, indices }
}

/// Project assembled particles into one draw batch.
#[must_use]
pub fn prepare_particle_batch(
    particles: &[SceneParticle],
    context: &ParticlePreparationContext,
    texture: &RendererImage,
    project: &dyn Fn(Vec3) -> Vec4,
) -> DrawBatch {
    let geometry = prepare_particle_geometry(particles, context);
    DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices: geometry.indices,
        texture: TextureBinding::BindImage(texture.clone()),
        state: RenderState {
            blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
            depth_test: crate::render::types::DepthTest::LessEqual,
            depth_write: context.indexed_profile == IndexedProfile::Q1,
            alpha_test: AlphaTest::None,
            cull: CullFace::None,
            depth_range: [0.0, 1.0],
            polygon_offset: None,
        },
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Triangles,
        vertices: BatchVertices::Single(
            geometry
                .vertices
                .iter()
                .map(|vertex| RenderVertex {
                    position: project(vertex.position),
                    tex_coord: vertex.tex_coord,
                    color: vec4(
                        f32::from(vertex.color[0]) / 255.0,
                        f32::from(vertex.color[1]) / 255.0,
                        f32::from(vertex.color[2]) / 255.0,
                        f32::from(vertex.color[3]) / 255.0,
                    ),
                })
                .collect(),
        ),
    }
}

/// Q2 six-sided beam: frame supplies the diameter, the previous origin the
/// endpoint.
#[must_use]
pub fn q2_beam_geometry(origin: Vec3, old_origin: Vec3, diameter: f32, color: [u8; 4]) -> MaterialGeometry {
    let delta = sub3(old_origin, origin);
    if length3(delta) == 0.0 {
        return MaterialGeometry::empty();
    }
    let direction = normalize3(delta);
    let perpendicular = scale3(perpendicular_vector(direction), diameter / 2.0);
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for segment in 0..6 {
        let normal = rotate_point_around_vector(direction, perpendicular, f64::from(segment * 60));
        let start = add3(origin, normal);
        let end = add3(start, delta);
        for position in [start, end] {
            vertices.push(MaterialVertex::new(
                position,
                normalize3(normal),
                vec2(0.0, 0.0),
                vec2(0.0, 0.0),
                color,
            ));
        }
        let a = segment * 2;
        let b = ((segment + 1) % 6) * 2;
        indices.extend([a, a + 1, b, b, a + 1, b + 1]);
    }
    MaterialGeometry { vertices, indices }
}

/// Project a Q2 beam into one additive draw batch.
#[must_use]
pub fn q2_beam_batch(
    origin: Vec3,
    old_origin: Vec3,
    diameter: f32,
    color: [u8; 4],
    project: &dyn Fn(Vec3) -> Vec4,
    white_image: &RendererImage,
    state: &RenderState,
) -> DrawBatch {
    let geometry = q2_beam_geometry(origin, old_origin, diameter, color);
    let normalized = vec4(
        f32::from(color[0]) / 255.0,
        f32::from(color[1]) / 255.0,
        f32::from(color[2]) / 255.0,
        f32::from(color[3]) / 255.0,
    );
    DrawBatch {
        fog: None,
        luminance_alpha: false,
        indices: geometry.indices,
        texture: TextureBinding::BindImage(white_image.clone()),
        state: RenderState {
            depth_write: false,
            blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
            alpha_test: AlphaTest::None,
            ..*state
        },
        lighting: BatchLighting::Vertex,
        primitive: BatchPrimitive::Triangles,
        vertices: BatchVertices::Single(
            geometry
                .vertices
                .iter()
                .map(|vertex| RenderVertex {
                    position: project(vertex.position),
                    tex_coord: vertex.tex_coord,
                    color: normalized,
                })
                .collect(),
        ),
    }
}

/// Legacy 8x8 particle dot image.
#[must_use]
pub fn legacy_particle_image(profile: IndexedProfile) -> RenderImage {
    let dot: [&str; 8] = match profile {
        IndexedProfile::Q1 => [
            "01100000", "11110000", "11110000", "01100000", "00000000", "00000000", "00000000", "00000000",
        ],
        IndexedProfile::Q2 => [
            "00000000", "00110000", "01111000", "01111000", "00110000", "00000000", "00000000", "00000000",
        ],
    };
    let mut pixels = vec![0u8; 8 * 8 * 4];
    for y in 0..8 {
        for (x, row) in dot.iter().enumerate() {
            let lit = row.as_bytes()[y] == b'1';
            let base = (y * 8 + x) * 4;
            pixels[base] = 255;
            pixels[base + 1] = 255;
            pixels[base + 2] = 255;
            pixels[base + 3] = u8::from(lit) * 255;
        }
    }
    RenderImage::Rgba8 {
        levels: vec![ImageLevel {
            width: 8,
            height: 8,
            pixels,
        }],
        border_color: vec4(0.0, 0.0, 0.0, 0.0),
    }
}

/// Q1 particle behavior kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1ParticleType {
    /// Stationary.
    Static,
    /// Rising fire.
    Fire,
    /// Explosion.
    Explode,
    /// Fast explosion.
    Explode2,
    /// Blob.
    Blob,
    /// Slow blob.
    Blob2,
    /// Gravity-affected.
    Gravity,
    /// Slow gravity.
    SlowGravity,
}

/// Q1 particle simulation state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1ParticleState {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Palette color.
    pub color: u8,
    /// Animation ramp.
    pub ramp: f32,
    /// Death time; negative retires the particle.
    pub die: f32,
    /// Behavior kind.
    pub particle_type: Q1ParticleType,
}

const RAMP1: [u8; 8] = [111, 109, 107, 105, 103, 101, 99, 97];
const RAMP2: [u8; 8] = [111, 110, 109, 108, 107, 106, 104, 102];
const RAMP3: [u8; 8] = [109, 107, 6, 5, 4, 3, 0, 0];

/// Advance one Q1 particle. Call once per client frame after taking the render
/// sample, never once per seat.
#[must_use]
pub fn advance_q1_particle(particle: &Q1ParticleState, seconds: f32, gravity: f32) -> Q1ParticleState {
    let origin = vec3(
        particle.origin.x + particle.velocity.x * seconds,
        particle.origin.y + particle.velocity.y * seconds,
        particle.origin.z + particle.velocity.z * seconds,
    );
    let (mut velocity, mut ramp, mut die, mut color) = (particle.velocity, particle.ramp, particle.die, particle.color);
    let grav = seconds * gravity * 0.05;
    let dvel = 4.0 * seconds;
    let drag = |amount: f32, z: bool| {
        vec3(
            velocity.x + velocity.x * amount,
            velocity.y + velocity.y * amount,
            if z {
                velocity.z + velocity.z * amount
            } else {
                velocity.z
            },
        )
    };
    match particle.particle_type {
        Q1ParticleType::Static => {}
        Q1ParticleType::Fire => {
            ramp += seconds * 5.0;
            if ramp >= 6.0 {
                die = -1.0;
            } else {
                color = RAMP3[ramp.trunc() as usize];
            }
            velocity.z += grav;
        }
        Q1ParticleType::Explode => {
            ramp += seconds * 10.0;
            if ramp >= 8.0 {
                die = -1.0;
            } else {
                color = RAMP1[ramp.trunc() as usize];
            }
            velocity = drag(dvel, true);
            velocity.z -= grav;
        }
        Q1ParticleType::Explode2 => {
            ramp += seconds * 15.0;
            if ramp >= 8.0 {
                die = -1.0;
            } else {
                color = RAMP2[ramp.trunc() as usize];
            }
            velocity = drag(-seconds, true);
            velocity.z -= grav;
        }
        Q1ParticleType::Blob => {
            velocity = drag(dvel, true);
            velocity.z -= grav;
        }
        Q1ParticleType::Blob2 => {
            velocity = drag(-dvel, false);
            velocity.z -= grav;
        }
        Q1ParticleType::Gravity | Q1ParticleType::SlowGravity => {
            velocity.z -= grav;
        }
    }
    Q1ParticleState {
        origin,
        velocity,
        color,
        ramp,
        die,
        particle_type: particle.particle_type,
    }
}

/// Q2 particle simulation state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ParticleState {
    /// Spawn time in milliseconds.
    pub spawn_milliseconds: i32,
    /// Spawn origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Acceleration.
    pub acceleration: Vec3,
    /// Palette color.
    pub color: u8,
    /// Initial alpha.
    pub alpha: f32,
    /// Alpha decay per second (-10000 pins an instant particle).
    pub alpha_velocity: f32,
}

/// Sample a Q2 particle at a client time, or `None` once fully faded.
/// Instant particles survive one submission; the owner retires them after.
#[must_use]
pub fn sample_q2_particle(particle: &Q2ParticleState, milliseconds: i32) -> Option<SceneParticle> {
    let seconds = if particle.alpha_velocity == -10000.0 {
        0.0
    } else {
        (milliseconds - particle.spawn_milliseconds) as f32 * 0.001
    };
    let alpha = particle.alpha + seconds * particle.alpha_velocity;
    if alpha <= 0.0 {
        return None;
    }
    let square = seconds * seconds;
    Some(SceneParticle::Indexed {
        palette_index: particle.color,
        alpha: alpha.min(1.0),
        size: 1.0,
        origin: vec3(
            particle.origin.x + particle.velocity.x * seconds + particle.acceleration.x * square,
            particle.origin.y + particle.velocity.y * seconds + particle.acceleration.y * square,
            particle.origin.z + particle.velocity.z * seconds + particle.acceleration.z * square,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, -10.0, 0.0),
            axis: [vec3(0.0, 1.0, 0.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [0.0; 16],
            viewport: crate::view::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn palette(_index: u8) -> Vec3 {
        vec3(255.0, 128.0, 0.0)
    }

    #[test]
    fn rgba_particle_emits_billboard() {
        let context = ParticlePreparationContext {
            camera: camera(),
            indexed_profile: IndexedProfile::Q2,
            palette_color: &palette,
        };
        let geometry = prepare_particle_geometry(
            &[SceneParticle::Rgba {
                origin: vec3(0.0, 0.0, 0.0),
                size: 4.0,
                rotation: 0.0,
                color: vec4(1.0, 1.0, 1.0, 1.0),
            }],
            &context,
        );
        assert_eq!(geometry.vertices.len(), 4);
        assert_eq!(geometry.indices.len(), 6);
    }

    #[test]
    fn indexed_particle_emits_triangle() {
        let context = ParticlePreparationContext {
            camera: camera(),
            indexed_profile: IndexedProfile::Q2,
            palette_color: &palette,
        };
        let geometry = prepare_particle_geometry(
            &[SceneParticle::Indexed {
                palette_index: 7,
                alpha: 1.0,
                size: 1.0,
                origin: vec3(0.0, 0.0, 0.0),
            }],
            &context,
        );
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.indices, vec![0, 1, 2]);
    }

    #[test]
    fn batch_projects_vertices() {
        let context = ParticlePreparationContext {
            camera: camera(),
            indexed_profile: IndexedProfile::Q1,
            palette_color: &palette,
        };
        let authority = qa_core::identity::IdentityOwner::create("test").expect("owner");
        let image = RendererImage {
            owner: crate::render::types::ResourceOwner::new(1, authority.session().clone(), 0),
            ordinal: 3,
            source: crate::render::types::ImageSource::Generated {
                name: "dot".to_string(),
            },
            width: 8,
            height: 8,
        };
        let batch = prepare_particle_batch(
            &[SceneParticle::Indexed {
                palette_index: 7,
                alpha: 1.0,
                size: 1.0,
                origin: vec3(0.0, 0.0, 0.0),
            }],
            &context,
            &image,
            &|point| vec4(point.x, point.y, point.z, 1.0),
        );
        assert!(batch.state.depth_write);
        match batch.vertices {
            BatchVertices::Single(vertices) => assert_eq!(vertices.len(), 3),
            _ => panic!("expected single texturing"),
        }
    }

    #[test]
    fn beam_has_twelve_vertices() {
        let geometry = q2_beam_geometry(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 32.0), 4.0, [255; 4]);
        assert_eq!(geometry.vertices.len(), 12);
        assert_eq!(geometry.indices.len(), 36);
        assert!(
            q2_beam_geometry(vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), 4.0, [255; 4])
                .vertices
                .is_empty()
        );
    }

    #[test]
    fn particle_dot_is_8x8_rgba() {
        match legacy_particle_image(IndexedProfile::Q2) {
            RenderImage::Rgba8 { levels, .. } => {
                assert_eq!(levels.len(), 1);
                assert_eq!(levels[0].width, 8);
                assert_eq!(levels[0].pixels.len(), 8 * 8 * 4);
            }
            _ => panic!("expected rgba image"),
        }
    }

    #[test]
    fn q1_gravity_pulls_down() {
        let particle = Q1ParticleState {
            origin: vec3(0.0, 0.0, 100.0),
            velocity: vec3(0.0, 0.0, 0.0),
            color: 0,
            ramp: 0.0,
            die: 10.0,
            particle_type: Q1ParticleType::Gravity,
        };
        let next = advance_q1_particle(&particle, 1.0, 800.0);
        assert!(next.velocity.z < 0.0);
        assert_eq!(next.origin, vec3(0.0, 0.0, 100.0));
    }

    #[test]
    fn q1_fire_ramps_and_dies() {
        let particle = Q1ParticleState {
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            color: 109,
            ramp: 0.0,
            die: 10.0,
            particle_type: Q1ParticleType::Fire,
        };
        let next = advance_q1_particle(&particle, 2.0, 800.0);
        assert_eq!(next.die, -1.0);
    }

    #[test]
    fn q2_particle_fades_and_moves() {
        let particle = Q2ParticleState {
            spawn_milliseconds: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(10.0, 0.0, 0.0),
            acceleration: vec3(0.0, 0.0, 0.0),
            color: 14,
            alpha: 1.0,
            alpha_velocity: -0.5,
        };
        let sampled = sample_q2_particle(&particle, 1000).expect("alive");
        match sampled {
            SceneParticle::Indexed { origin, alpha, .. } => {
                assert_eq!(origin.x, 10.0);
                assert!((alpha - 0.5).abs() < 1e-6);
            }
            _ => panic!("expected indexed particle"),
        }
        assert!(sample_q2_particle(&particle, 3000).is_none());
    }
}
