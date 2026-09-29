//! Geometry deformations and renderer noise (`tr_shade_calc.c`).
//!
//! Donor provenance: `src/materials/deform.ts` (translated from
//! `tr_shade_calc.c`, `tr_surface.c`, `tr_shadows.c`, `tr_noise.c`).

use qa_content::md3::{normalize_fast3, renderer_sine};
use qa_core::math::{
    add3, cross3, dot3, length3, normalize3, scale3, sub3, vec3, Axis, Vec3,
};

use super::geometry::{MaterialDeformState, MaterialGeometry, MaterialVertex};
use super::material::{evaluate_waveform, SourceWaveFunc, VertexDeformation, WaveKind};
use crate::ClientError;

/// Deformable geometry view.
#[derive(Debug, Clone, PartialEq)]
pub struct DeformGeometry {
    /// Vertices.
    pub vertices: Vec<MaterialVertex>,
    /// Indices.
    pub indices: Vec<u32>,
}

impl From<MaterialGeometry> for DeformGeometry {
    fn from(geometry: MaterialGeometry) -> Self {
        Self {
            vertices: geometry.vertices,
            indices: geometry.indices,
        }
    }
}

impl From<DeformGeometry> for MaterialGeometry {
    fn from(geometry: DeformGeometry) -> Self {
        Self {
            vertices: geometry.vertices,
            indices: geometry.indices,
        }
    }
}

/// Deform view orientation (`DeformView`).
#[derive(Debug, Clone, Copy)]
pub struct DeformView {
    /// View axis.
    pub axis: Axis,
    /// Mirror mode.
    pub mirror: bool,
    /// Entity axis (or `None` for world).
    pub entity_axis: Option<Axis>,
    /// Non-normalized axis scale flag.
    pub non_normalized_axis: Option<Vec3>,
}

/// Projection-shadow context (`ProjectionShadowContext`).
#[derive(Debug, Clone, Copy)]
pub struct ProjectionShadowContext {
    /// Entity axis.
    pub axis: Axis,
    /// Entity origin.
    pub origin: Vec3,
    /// Shadow plane height.
    pub shadow_plane: f32,
    /// Light direction.
    pub light_dir: Vec3,
}

fn at(vertices: &[MaterialVertex], index: usize) -> Result<MaterialVertex, ClientError> {
    vertices.get(index).copied().ok_or_else(|| {
        ClientError::BadMaterial(format!(
            "deformation index {index} outside {}",
            vertices.len()
        ))
    })
}

/// Linux `rand()` sequence (`linuxRandom`).
struct LinuxRandom {
    state: [u32; 31],
    front: usize,
    rear: usize,
}

impl LinuxRandom {
    fn new(seed: u32) -> Self {
        let mut state = [if seed == 0 { 1 } else { seed }; 31];
        for index in 1..31 {
            state[index] = (16_807u64 * u64::from(state[index - 1]) % 2_147_483_647) as u32;
        }
        let mut random = Self {
            state,
            front: 3,
            rear: 0,
        };
        for _ in 0..310 {
            random.next();
        }
        random
    }

    fn next(&mut self) -> u32 {
        let value = self.state[self.front].wrapping_add(self.state[self.rear]);
        self.state[self.front] = value;
        self.front = (self.front + 1) % 31;
        self.rear = (self.rear + 1) % 31;
        value >> 1
    }
}

/// Seeded renderer noise (`RendererNoise`, `srand(1001)` tables).
#[derive(Debug, Clone)]
pub struct RendererNoise {
    values: [f32; 256],
    permutation: [u8; 256],
}

impl RendererNoise {
    /// Build the immutable noise tables.
    #[must_use]
    pub fn new() -> Self {
        let mut random = LinuxRandom::new(1001);
        let mut values = [0.0f32; 256];
        let mut permutation = [0u8; 256];
        for index in 0..256 {
            let value = random.next() as f32 / 2_147_483_647.0;
            values[index] = value * 2.0 - 1.0;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let slot = ((random.next() as f32 / 2_147_483_647.0) * 255.0) as u8;
            permutation[index] = slot;
        }
        Self {
            values,
            permutation,
        }
    }

    /// Sample 4D noise (`R_NoiseGet4f`).
    #[must_use]
    pub fn sample(&self, x: f32, y: f32, z: f32, time: f32) -> f32 {
        let ix = x.floor() as i32;
        let iy = y.floor() as i32;
        let iz = z.floor() as i32;
        let it = time.floor() as i32;
        let fx = x - ix as f32;
        let fy = y - iy as f32;
        let fz = z - iz as f32;
        let ft = time - it as f32;
        let perm = |index: i32| -> usize { self.permutation[(index & 255) as usize] as usize };
        let lattice = |dx: i32, dy: i32, dz: i32, dt: i32| -> f32 {
            self.values[perm(ix + dx + perm(iy + dy + perm(iz + dz + perm(it + dt))) as i32)]
        };
        let lerp = |a: f32, b: f32, amount: f32| a * (1.0 - amount) + b * amount;
        let plane = |dz: i32, dt: i32| {
            lerp(
                lerp(lattice(0, 0, dz, dt), lattice(1, 0, dz, dt), fx),
                lerp(lattice(0, 1, dz, dt), lattice(1, 1, dz, dt), fx),
                fy,
            )
        };
        lerp(
            lerp(plane(0, 0), plane(1, 0), fz),
            lerp(plane(0, 1), plane(1, 1), fz),
            ft,
        )
    }
}

impl Default for RendererNoise {
    fn default() -> Self {
        Self::new()
    }
}

fn local_direction(direction: Vec3, view: &DeformView) -> Vec3 {
    match view.entity_axis {
        None => direction,
        Some(axis) => vec3(
            dot3(direction, axis[0]),
            dot3(direction, axis[1]),
            dot3(direction, axis[2]),
        ),
    }
}

fn projection_shadow_geometry(
    mesh: &DeformGeometry,
    context: &ProjectionShadowContext,
) -> DeformGeometry {
    let ground = vec3(context.axis[0].z, context.axis[1].z, context.axis[2].z);
    let ground_dist = context.origin.z - context.shadow_plane;
    let mut light_dir = context.light_dir;
    let mut d = dot3(light_dir, ground);
    if d < 0.5 {
        light_dir = vec3(
            light_dir.x + ground.x * (0.5 - d),
            light_dir.y + ground.y * (0.5 - d),
            light_dir.z + ground.z * (0.5 - d),
        );
        d = dot3(light_dir, ground);
    }
    let light = scale3(light_dir, 1.0 / d);
    DeformGeometry {
        indices: mesh.indices.clone(),
        vertices: mesh
            .vertices
            .iter()
            .map(|vertex| {
                let height = dot3(vertex.position, ground) + ground_dist;
                MaterialVertex {
                    position: sub3(vertex.position, scale3(light, height)),
                    ..*vertex
                }
            })
            .collect(),
    }
}

fn sprite_geometry(mesh: &DeformGeometry, view: &DeformView) -> Result<DeformGeometry, ClientError> {
    if mesh.vertices.len() % 4 != 0 || mesh.indices.len() != mesh.vertices.len() / 4 * 6 {
        return Err(ClientError::BadMaterial(
            "autosprite requires independent four-vertex quads".to_string(),
        ));
    }
    let mut vertices = Vec::with_capacity(mesh.vertices.len());
    let mut indices = Vec::with_capacity(mesh.indices.len());
    let left_dir = local_direction(view.axis[1], view);
    let up_dir = local_direction(view.axis[2], view);
    let mut axis_scale = 1.0f32;
    if let Some(axis) = view.non_normalized_axis {
        let length = length3(axis);
        axis_scale = if length == 0.0 { 0.0 } else { 1.0 / length };
    }
    let mut start = 0usize;
    while start < mesh.vertices.len() {
        let first = at(&mesh.vertices, start)?;
        let center = scale3(
            add3(
                add3(first.position, at(&mesh.vertices, start + 1)?.position),
                add3(
                    at(&mesh.vertices, start + 2)?.position,
                    at(&mesh.vertices, start + 3)?.position,
                ),
            ),
            0.25,
        );
        let delta = sub3(first.position, center);
        let radius = dot3(delta, delta).sqrt() * 0.707;
        let left = scale3(
            scale3(left_dir, if view.mirror { -radius } else { radius }),
            axis_scale,
        );
        let up = scale3(scale3(up_dir, radius), axis_scale);
        let normal = sub3(vec3(0.0, 0.0, 0.0), view.axis[0]);
        let positions = [
            add3(add3(center, left), up),
            add3(sub3(center, left), up),
            sub3(sub3(center, left), up),
            sub3(add3(center, left), up),
        ];
        let coords = [
            qa_core::math::vec2(0.0, 0.0),
            qa_core::math::vec2(1.0, 0.0),
            qa_core::math::vec2(1.0, 1.0),
            qa_core::math::vec2(0.0, 1.0),
        ];
        for corner in 0..4 {
            vertices.push(MaterialVertex {
                position: positions[corner],
                normal,
                tex_coord: coords[corner],
                lightmap_coord: coords[corner],
                color: first.color,
            });
        }
        let base = start as u32;
        indices.extend([base, base + 1, base + 3, base + 3, base + 1, base + 2]);
        start += 4;
    }
    Ok(DeformGeometry { vertices, indices })
}

fn text_geometry(
    tess: &mut MaterialDeformState,
    index: usize,
    view: &DeformView,
) -> Result<(), ClientError> {
    let text = tess
        .render_text
        .get(index)
        .ok_or_else(|| {
            ClientError::BadMaterial(format!("deformation index {index} outside render text"))
        })?
        .clone();
    let quad = tess.text_quad()?;
    let mut width = cross3(quad[0].normal, vec3(0.0, 0.0, -1.0));
    let mut mid = vec3(0.0, 0.0, 0.0);
    let mut bottom = 999_999.0f32;
    let mut top = -999_999.0f32;
    for vertex in &quad {
        mid = add3(vertex.position, mid);
        bottom = bottom.min(vertex.position.z);
        top = top.max(vertex.position.z);
    }
    let height = vec3(0.0, 0.0, (top - bottom) * 0.5);
    width = scale3(width, height.z * -0.75);
    let nul = text.find('\0').unwrap_or(text.len());
    let length = text[..nul].chars().count();
    let chars: Vec<char> = text[..nul].chars().collect();
    let mut origin = add3(scale3(mid, 0.25), scale3(width, (length as i32 - 1) as f32));
    let normal = sub3(vec3(0.0, 0.0, 0.0), view.axis[0]);
    tess.reset_geometry();
    for character in 0..length {
        let ch = chars[character] as u32 & 255;
        if ch != 32 {
            let s = f32::from((ch & 15) as u8) * 0.0625;
            let t = f32::from((ch >> 4) as u8) * 0.0625;
            let positions = [
                add3(add3(origin, width), height),
                add3(sub3(origin, width), height),
                sub3(sub3(origin, width), height),
                sub3(add3(origin, width), height),
            ];
            let coords = [
                qa_core::math::vec2(s, t),
                qa_core::math::vec2(s + 0.0625, t),
                qa_core::math::vec2(s + 0.0625, t + 0.0625),
                qa_core::math::vec2(s, t + 0.0625),
            ];
            tess.append_geometry(&MaterialGeometry {
                vertices: positions
                    .iter()
                    .zip(coords.iter())
                    .map(|(position, coord)| MaterialVertex {
                        position: *position,
                        normal,
                        tex_coord: *coord,
                        lightmap_coord: *coord,
                        color: [255, 255, 255, 255],
                    })
                    .collect(),
                indices: vec![0, 1, 3, 3, 1, 2],
            });
        }
        origin = add3(origin, scale3(width, -2.0));
    }
    Ok(())
}

fn pivot_geometry(mesh: &DeformGeometry, view: &DeformView) -> Result<DeformGeometry, ClientError> {
    if mesh.vertices.len() % 4 != 0 || mesh.indices.len() != mesh.vertices.len() / 4 * 6 {
        return Err(ClientError::BadMaterial(
            "autosprite2 requires independent four-vertex quads".to_string(),
        ));
    }
    let forward = local_direction(view.axis[0], view);
    let mut vertices = mesh.vertices.clone();
    let edges: [(usize, usize); 6] = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
    let mut start = 0usize;
    while start < vertices.len() {
        let mut shortest = 0usize;
        let mut second = 0usize;
        let mut first_length = 999_999.0f32;
        let mut second_length = 999_999.0f32;
        for (index, (a, b)) in edges.iter().enumerate() {
            let delta = sub3(vertices[start + a].position, vertices[start + b].position);
            let length = dot3(delta, delta);
            if length < first_length {
                second = shortest;
                second_length = first_length;
                shortest = index;
                first_length = length;
            } else if length < second_length {
                second = index;
                second_length = length;
            }
        }
        let midpoint = |edge: usize| {
            let (a, b) = edges[edge];
            scale3(
                add3(vertices[start + a].position, vertices[start + b].position),
                0.5,
            )
        };
        let first_mid = midpoint(shortest);
        let second_mid = midpoint(second);
        let minor = normalize3(cross3(sub3(second_mid, first_mid), forward));
        for (edge, mid, length) in [
            (shortest, first_mid, first_length),
            (second, second_mid, second_length),
        ] {
            let (a, b) = edges[edge];
            let mut follows = false;
            for k in 0..5 {
                if mesh.indices[start / 4 * 6 + k] == (start + a) as u32
                    && mesh.indices[start / 4 * 6 + k + 1] == (start + b) as u32
                {
                    follows = true;
                }
            }
            let radius = length.sqrt() * 0.5 * if follows { -1.0 } else { 1.0 };
            let pa = vertices[start + a];
            let pb = vertices[start + b];
            vertices[start + a] = MaterialVertex {
                position: add3(mid, scale3(minor, radius)),
                ..pa
            };
            vertices[start + b] = MaterialVertex {
                position: add3(mid, scale3(minor, -radius)),
                ..pb
            };
        }
        start += 4;
    }
    Ok(DeformGeometry {
        vertices,
        indices: mesh.indices.clone(),
    })
}

/// Apply deformations (`deformGeometry`).
pub fn deform_geometry(
    tess: &mut MaterialDeformState,
    deforms: &[VertexDeformation],
    view: &DeformView,
    time: f32,
    noise: &RendererNoise,
    projection_shadow: Option<&ProjectionShadowContext>,
) -> Result<DeformGeometry, ClientError> {
    let mut result = DeformGeometry::from(tess.snapshot_geometry());
    for deform in deforms {
        match deform {
            VertexDeformation::None => {}
            VertexDeformation::Text { index } => {
                text_geometry(tess, usize::from(*index), view)?;
                result = DeformGeometry::from(tess.snapshot_geometry());
            }
            VertexDeformation::Autosprite => {
                result = sprite_geometry(&result, view)?;
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
            VertexDeformation::Autosprite2 => {
                result = pivot_geometry(&result, view)?;
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
            VertexDeformation::ProjectionShadow => {
                let context = projection_shadow.ok_or_else(|| {
                    ClientError::BadMaterial(
                        "projectionshadow deformation requires retained entity orientation and lighting"
                            .to_string(),
                    )
                })?;
                result = projection_shadow_geometry(&result, context);
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
            VertexDeformation::Move { direction, wave }
            | VertexDeformation::Wave { wave, .. }
                if matches!(wave.kind, WaveKind::None | WaveKind::Noise) =>
            {
                let material = tess.material.clone().ok_or_else(|| {
                    ClientError::BadMaterial(
                        "Invalid deformation waveform requires its begun source material"
                            .to_string(),
                    )
                })?;
                let func = if wave.kind == WaveKind::None {
                    SourceWaveFunc::None as u8
                } else {
                    SourceWaveFunc::Noise as u8
                };
                return Err(ClientError::BadMaterial(format!(
                    "TableForFunc called with invalid function '{func}' in shader '{material}'\n"
                )));
            }
            VertexDeformation::Move { direction, wave } => {
                let offset = scale3(*direction, evaluate_waveform(wave, time)?);
                result = DeformGeometry {
                    indices: result.indices.clone(),
                    vertices: result
                        .vertices
                        .iter()
                        .map(|vertex| MaterialVertex {
                            position: add3(vertex.position, offset),
                            ..*vertex
                        })
                        .collect(),
                };
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
            VertexDeformation::Wave { spread, wave } => {
                let constant = if wave.frequency == 0.0 {
                    Some(evaluate_waveform(wave, time)?)
                } else {
                    None
                };
                let mut vertices = Vec::with_capacity(result.vertices.len());
                for vertex in &result.vertices {
                    let sum = vertex.position.x + vertex.position.y + vertex.position.z;
                    let phase = wave.phase + sum * spread;
                    let scale = match constant {
                        Some(scale) => scale,
                        None => evaluate_waveform(
                            &super::material::Waveform { phase, ..*wave },
                            time,
                        )?,
                    };
                    vertices.push(MaterialVertex {
                        position: add3(vertex.position, scale3(vertex.normal, scale)),
                        ..*vertex
                    });
                }
                result = DeformGeometry {
                    indices: result.indices.clone(),
                    vertices,
                };
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
            VertexDeformation::Normal {
                amplitude,
                frequency,
            } => {
                let mut vertices = Vec::with_capacity(result.vertices.len());
                for vertex in &result.vertices {
                    let position = vertex.position;
                    let x = position.x * 0.98;
                    let y = position.y * 0.98;
                    let z = position.z * 0.98;
                    let t = time * frequency;
                    let normal = normalize_fast3(qa_core::math::vec3(
                        vertex.normal.x + amplitude * noise.sample(x, y, z, t),
                        vertex.normal.y + amplitude * noise.sample(100.0 + x, y, z, t),
                        vertex.normal.z + amplitude * noise.sample(200.0 + x, y, z, t),
                    ));
                    vertices.push(MaterialVertex { normal, ..*vertex });
                }
                result = DeformGeometry {
                    indices: result.indices.clone(),
                    vertices,
                };
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
            VertexDeformation::Bulge {
                width,
                height,
                speed,
            } => {
                let now = tess.refdef_time * speed * 0.001;
                let mut vertices = Vec::with_capacity(result.vertices.len());
                for vertex in &result.vertices {
                    let phase = vertex.tex_coord.x * width + now;
                    #[allow(clippy::cast_possible_truncation)]
                    let index = ((1024.0 / (core::f32::consts::PI * 2.0) * phase) as i32) & 1023;
                    vertices.push(MaterialVertex {
                        position: add3(
                            vertex.position,
                            scale3(vertex.normal, renderer_sine(index) * height),
                        ),
                        ..*vertex
                    });
                }
                result = DeformGeometry {
                    indices: result.indices.clone(),
                    vertices,
                };
                tess.replace_geometry(MaterialGeometry::from(result.clone()));
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::material::Waveform;
    use qa_core::math::{vec2, Vec2};

    fn quad() -> MaterialGeometry {
        let vertex = |x: f32, y: f32| MaterialVertex::new(
            vec3(x, y, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec2(0.0, 0.0),
            vec2(0.0, 0.0),
            [255, 255, 255, 255],
        );
        MaterialGeometry {
            vertices: vec![vertex(0.0, 0.0), vertex(1.0, 0.0), vertex(1.0, 1.0), vertex(0.0, 1.0)],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    fn view() -> DeformView {
        DeformView {
            axis: [
                vec3(1.0, 0.0, 0.0),
                vec3(0.0, 1.0, 0.0),
                vec3(0.0, 0.0, 1.0),
            ],
            mirror: false,
            entity_axis: None,
            non_normalized_axis: None,
        }
    }

    #[test]
    fn move_deform_offsets_positions() {
        let mut tess = MaterialDeformState::new(quad(), 0.0, Vec::new(), None);
        let noise = RendererNoise::new();
        let result = deform_geometry(
            &mut tess,
            &[VertexDeformation::Move {
                direction: vec3(1.0, 0.0, 0.0),
                wave: Waveform {
                    kind: WaveKind::Square,
                    base: 0.0,
                    amplitude: 2.0,
                    phase: 0.0,
                    frequency: 0.0,
                },
            }],
            &view(),
            0.0,
            &noise,
            None,
        )
        .unwrap();
        assert!((result.vertices[0].position.x - 2.0).abs() < 1e-5);
    }

    #[test]
    fn invalid_move_waveform_drops() {
        let mut tess = MaterialDeformState::new(
            quad(),
            0.0,
            Vec::new(),
            Some("test".to_string()),
        );
        let noise = RendererNoise::new();
        let err = deform_geometry(
            &mut tess,
            &[VertexDeformation::Move {
                direction: vec3(1.0, 0.0, 0.0),
                wave: Waveform::zero(WaveKind::None),
            }],
            &view(),
            0.0,
            &noise,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ClientError::BadMaterial(_)));
    }

    #[test]
    fn noise_is_deterministic() {
        let noise = RendererNoise::new();
        assert_eq!(noise.sample(1.0, 2.0, 3.0, 4.0), noise.sample(1.0, 2.0, 3.0, 4.0));
        let _ = Vec2 { x: 0.0, y: 0.0 };
    }

    #[test]
    fn autosprite_rejects_non_quads() {
        let mut tess = MaterialDeformState::new(MaterialGeometry::empty(), 0.0, Vec::new(), None);
        let noise = RendererNoise::new();
        // Empty geometry passes the modulo check but has no quads; use 5 vertices.
        tess.replace_geometry(MaterialGeometry {
            vertices: vec![
                MaterialVertex::new(
                    vec3(0.0, 0.0, 0.0),
                    vec3(0.0, 0.0, 1.0),
                    vec2(0.0, 0.0),
                    vec2(0.0, 0.0),
                    [255, 255, 255, 255],
                );
                5
            ],
            indices: Vec::new(),
        });
        assert!(
            deform_geometry(&mut tess, &[VertexDeformation::Autosprite], &view(), 0.0, &noise, None)
                .is_err()
        );
    }
}
