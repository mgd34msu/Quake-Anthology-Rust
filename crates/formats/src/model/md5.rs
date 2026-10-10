use super::{read::*, *};
use crate::text::Tokens;
use qa_core::math::{cross, normalized, rotate_quaternion};

fn vector(t: &mut Tokens<'_>) -> Result<Vec3, FormatError> {
    t.require(b"(")?;
    let v = Vec3([t.scalar()?, t.scalar()?, t.scalar()?]);
    t.require(b")")?;
    Ok(v)
}
fn count(t: &mut Tokens<'_>, label: &[u8], max: i32) -> Result<usize, FormatError> {
    t.require(label)?;
    let n = t.integer(0, max)? as usize;
    if n > t.bytes.len() - t.at {
        return Err(FormatError::InvalidRange);
    }
    Ok(n)
}
fn record(t: &mut Tokens<'_>, label: &[u8], seen: &mut [bool]) -> Result<usize, FormatError> {
    t.require(label)?;
    let index = t.integer(0, seen.len() as i32 - 1)? as usize;
    if seen[index] {
        return Err(FormatError::InvalidValue);
    }
    seen[index] = true;
    Ok(index)
}
pub(super) fn load<'a>(bytes: &'a [u8], mut m: Model<'a>) -> Result<Model<'a>, FormatError> {
    let mut t = Tokens::new(bytes);
    t.require(b"MD5Version")?;
    if t.integer(10, 10)? != 10 {
        return Err(FormatError::Unsupported);
    }
    t.require(b"commandline")?;
    m.command_line = t.value()?;
    let bones = count(&mut t, b"numJoints", 256)?;
    let meshes = count(&mut t, b"numMeshes", 32)?;
    if bones == 0 || meshes == 0 {
        return Err(FormatError::InvalidRange);
    }
    t.require(b"joints")?;
    t.require(b"{")?;
    reserve(&mut m.bones, bones)?;
    for _ in 0..bones {
        let name = t.value()?;
        let parent = t.integer(-1, bones as i32 - 1)?;
        let position = vector(&mut t)?;
        let q = vector(&mut t)?.0;
        let d = 1.0 - q[0] * q[0] - q[1] * q[1] - q[2] * q[2];
        finite(d)?;
        let w = if d < 0.0 { 0.0 } else { -d.sqrt() };
        m.bones.push(Bone {
            name,
            parent: (parent >= 0).then_some(parent as u32),
            position,
            orientation: [q[0], q[1], q[2], w],
        });
    }
    t.require(b"}")?;
    for i in 0..bones {
        let mut index = Some(i as u32);
        let mut visits = 0;
        while let Some(current) = index {
            if visits == bones {
                return Err(FormatError::Cycle);
            }
            index = m.bones[current as usize].parent;
            visits += 1;
        }
    }
    let mut decoded_bytes = 0usize;
    for _ in 0..meshes {
        t.require(b"mesh")?;
        t.require(b"{")?;
        t.require(b"shader")?;
        let shader = t.value()?;
        let vertices = count(&mut t, b"numverts", 65535)?;
        let mut mesh = Mesh {
            vertices_per_frame: vertices,
            ..Mesh::default()
        };
        mesh.shaders.push(Shader {
            name: shader,
            native_index: 0,
        });
        mesh.texcoords.resize(vertices, [0.0; 2]);
        mesh.vertex_weights.resize(vertices, WeightRange::default());
        let mut seen = vec![false; vertices];
        for _ in 0..vertices {
            let index = record(&mut t, b"vert", &mut seen)?;
            t.require(b"(")?;
            mesh.texcoords[index] = [t.scalar()?, t.scalar()?];
            t.require(b")")?;
            mesh.vertex_weights[index] = WeightRange {
                first: t.integer(0, i32::MAX)? as u32,
                count: t.integer(0, i32::MAX)? as u32,
            };
        }
        let triangles = count(&mut t, b"numtris", 65535)?;
        mesh.triangles.resize(triangles, Triangle::default());
        let mut seen = vec![false; triangles];
        for _ in 0..triangles {
            let index = record(&mut t, b"tri", &mut seen)?;
            let mut indices = [0; 3];
            for value in &mut indices {
                *value = t.integer(0, vertices as i32 - 1)? as u32;
            }
            mesh.triangles[index] = Triangle {
                vertex: indices,
                texcoord: indices,
            };
        }
        let weights = count(&mut t, b"numweights", 1048576)?;
        decoded_bytes += vertices
            * (std::mem::size_of::<Vertex>()
                + std::mem::size_of::<WeightRange>()
                + std::mem::size_of::<Vec3>()
                + 8)
            + weights * std::mem::size_of::<Weight>()
            + triangles * std::mem::size_of::<Triangle>();
        if decoded_bytes > MAX_DECODED_BYTES {
            return Err(FormatError::InvalidRange);
        }
        reserve(&mut mesh.weights, weights)?;
        mesh.weights.resize(
            weights,
            Weight {
                bone: 0,
                bias: 0.0,
                offset: Vec3::default(),
            },
        );
        let mut seen = vec![false; weights];
        for _ in 0..weights {
            let index = record(&mut t, b"weight", &mut seen)?;
            let bone = t.integer(0, bones as i32 - 1)? as u32;
            let bias = t.scalar()?;
            if !(0.0..=1.0).contains(&bias) {
                return Err(FormatError::InvalidValue);
            }
            mesh.weights[index] = Weight {
                bone,
                bias,
                offset: vector(&mut t)?,
            };
        }
        t.require(b"}")?;
        reserve(&mut mesh.vertices, vertices)?;
        for range in &mesh.vertex_weights {
            let end = (range.first as usize)
                .checked_add(range.count as usize)
                .ok_or(FormatError::InvalidRange)?;
            let weights = mesh
                .weights
                .get(range.first as usize..end)
                .ok_or(FormatError::InvalidRange)?;
            let mut position = Vec3::default();
            for w in weights {
                let bone = &m.bones[w.bone as usize];
                position = position
                    + (bone.position + rotate_quaternion(bone.orientation, w.offset)) * w.bias;
            }
            finite_vec(position)?;
            m.bounds.add_point(position);
            mesh.vertices.push(Vertex {
                position,
                normal: Vec3::default(),
            });
        }
        bind_normals(&mut mesh, &m.bones)?;
        m.meshes.push(mesh);
    }
    if t.next()?.is_some() {
        return Err(FormatError::InvalidValue);
    }
    if m.bounds.mins.0[0] > m.bounds.maxs.0[0] {
        m.bounds = Bounds::default();
    }
    m.frames.push(Frame {
        bounds: m.bounds,
        ..Frame::default()
    });
    Ok(m)
}
fn bind_normals(mesh: &mut Mesh<'_>, bones: &[Bone<'_>]) -> Result<(), FormatError> {
    let n = mesh.vertices_per_frame;
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by(|&a, &b| {
        let a = mesh.vertices[a].position.0;
        let b = mesh.vertices[b].position.0;
        (0..3)
            .map(|i| a[i].partial_cmp(&b[i]).unwrap_or(std::cmp::Ordering::Equal))
            .find(|v| !v.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut groups = vec![0usize; n];
    let mut group = 0;
    for (i, &vertex) in order.iter().enumerate() {
        if i > 0 && mesh.vertices[vertex].position != mesh.vertices[order[i - 1]].position {
            group += 1;
        }
        groups[vertex] = group;
    }
    let mut normals = vec![Vec3::default(); n];
    for tri in &mesh.triangles {
        let a = mesh.vertices[tri.vertex[0] as usize].position;
        let b = mesh.vertices[tri.vertex[1] as usize].position;
        let c = mesh.vertices[tri.vertex[2] as usize].position;
        let d1 = normalized(c - a);
        let d2 = normalized(b - a);
        let normal = normalized(cross(d1, d2));
        let angle = d1.dot(d2).clamp(-1.0, 1.0).acos();
        finite_vec(normal)?;
        finite(angle)?;
        for vertex in tri.vertex {
            normals[groups[vertex as usize]] = normals[groups[vertex as usize]] + normal * angle;
        }
    }
    for normal in &mut normals {
        *normal = normalized(*normal);
        finite_vec(*normal)?;
    }
    reserve(&mut mesh.bind_normals, n)?;
    for (vertex, range) in mesh.vertex_weights.iter().enumerate() {
        let normal = normals[groups[vertex]];
        let mut local = Vec3::default();
        for weight in
            &mesh.weights[range.first as usize..range.first as usize + range.count as usize]
        {
            let q = bones[weight.bone as usize].orientation;
            local = local + rotate_quaternion([-q[0], -q[1], -q[2], q[3]], normal) * weight.bias;
        }
        finite_vec(local)?;
        mesh.bind_normals.push(local);
        // Keep the common vertex array ready for bind-pose upload. Animation
        // uses bind_normals and the retained numeric weight ranges.
        let mut output = Vec3::default();
        for weight in
            &mesh.weights[range.first as usize..range.first as usize + range.count as usize]
        {
            output = output
                + rotate_quaternion(bones[weight.bone as usize].orientation, local) * weight.bias;
        }
        finite_vec(output)?;
        mesh.vertices[vertex].normal = output;
    }
    Ok(())
}
