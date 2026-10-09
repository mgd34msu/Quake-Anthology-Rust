//! Shared cold decoder and raw output projection for developer brush-tree
//! comparison/timing fixtures. It performs no files, clocks, threads or game I/O.
use qa_core::primitives::{Axis, Bounds, ClipNode, GeometryId, Plane, SurfaceFlags, Vec3};
use qa_world::collision::brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot};
use qa_world::collision::{
    CollisionStore, Contents, EntityTracePolicy, Trace, TraceQuery, TraceRules,
};

pub const WORDS: usize = 13;

pub struct Fixture {
    pub store: CollisionStore,
    pub geometries: Vec<GeometryId>,
    pub queries: Vec<Query>,
}

#[derive(Clone, Copy)]
pub struct Query {
    pub map: usize,
    pub model: usize,
    pub trace: TraceQuery<'static>,
    pub point: Vec3,
}

fn word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data
        .split_first_chunk::<4>()
        .ok_or("truncated tree fixture")?;
    *data = tail;
    Ok(u32::from_le_bytes(*head))
}

fn count(data: &mut &[u8], lower: usize, upper: usize) -> Result<usize, &'static str> {
    let count = word(data)? as usize;
    if (lower..=upper).contains(&count) {
        Ok(count)
    } else {
        Err("tree fixture count exceeds bounds")
    }
}

fn vector(data: &mut &[u8]) -> Result<Vec3, &'static str> {
    let value = Vec3([
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
    ]);
    if value.0.iter().any(|value| !value.is_finite()) {
        return Err("nonfinite tree fixture vector");
    }
    Ok(value)
}

fn plane(data: &mut &[u8]) -> Result<Plane, &'static str> {
    let normal = vector(data)?;
    let distance = f32::from_bits(word(data)?);
    let axis = match word(data)? {
        0 => Some(Axis::X),
        1 => Some(Axis::Y),
        2 => Some(Axis::Z),
        3 => None,
        _ => return Err("invalid tree fixture plane axis"),
    };
    if !distance.is_finite() {
        return Err("nonfinite tree fixture plane distance");
    }
    Ok(Plane {
        normal,
        distance,
        axis,
    })
}

fn contents(raw: u32) -> Result<Contents, &'static str> {
    if raw & !(1 | 4 | 8 | 16 | 32 | 0x10000 | 0x20000) != 0 {
        return Err("unsupported fixture contents bit; this is not a wire conversion test");
    }
    Ok(Contents(u64::from(raw)))
}

pub fn load(
    mut data: &[u8],
    rules: TraceRules,
    entity_rules: EntityTracePolicy,
) -> Result<Fixture, &'static str> {
    if word(&mut data)? != 0x45525442 || word(&mut data)? != 1 {
        return Err("unsupported brush-tree fixture");
    }
    let map_count = count(&mut data, 1, 10)?;
    let query_count = count(&mut data, 10000, 50000)?;
    let mut store = CollisionStore::new();
    let mut geometries = Vec::with_capacity(map_count);
    let mut model_counts = Vec::with_capacity(map_count);
    for _ in 0..map_count {
        let brush_count = count(&mut data, 1, 16)?;
        let mut side_planes = Vec::new();
        let mut surfaces = Vec::new();
        let mut brushes = Vec::with_capacity(brush_count);
        for _ in 0..brush_count {
            let plane_count = count(&mut data, 6, 64)?;
            let contents = contents(word(&mut data)?)?;
            if word(&mut data)? > 1 {
                return Err("invalid fixture axial prefix order");
            }
            let bound_mins = vector(&mut data)?;
            let bound_maxs = vector(&mut data)?;
            if bound_mins
                .0
                .iter()
                .zip(bound_maxs.0)
                .any(|(min, max)| *min > max)
            {
                return Err("invalid fixture explicit native brush bounds");
            }
            if side_planes.len() + plane_count > 256 {
                return Err("fixture side arena exceeds native capacity");
            }
            brushes.push(Brush {
                first_plane: side_planes.len() as u32,
                plane_count: plane_count as u32,
                contents,
            });
            for _ in 0..plane_count {
                side_planes.push(plane(&mut data)?);
                surfaces.push(SurfaceFlags(word(&mut data)?));
            }
        }
        let plane_count = count(&mut data, 1, 64)?;
        let mut planes = Vec::with_capacity(plane_count);
        for _ in 0..plane_count {
            planes.push(plane(&mut data)?);
        }
        let node_count = count(&mut data, 1, 128)?;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            nodes.push(ClipNode {
                plane: word(&mut data)?,
                children: [word(&mut data)? as i32, word(&mut data)? as i32],
            });
        }
        let leaf_count = count(&mut data, 1, 64)?;
        let mut leaves = Vec::with_capacity(leaf_count);
        for _ in 0..leaf_count {
            if word(&mut data)? != 1 {
                return Err("native paired fixtures require explicit stored Q2 leaf contents");
            }
            leaves.push(CollisionLeaf {
                stored_contents: Some(contents(word(&mut data)?)?),
                first_brush: word(&mut data)?,
                brush_count: word(&mut data)?,
            });
        }
        let refs = count(&mut data, 0, 512)?;
        let mut leaf_brushes = Vec::with_capacity(refs);
        for _ in 0..refs {
            leaf_brushes.push(word(&mut data)?);
        }
        let model_count = count(&mut data, 1, 8)?;
        let mut models = Vec::with_capacity(model_count);
        for index in 0..model_count {
            let tag = word(&mut data)?;
            let root = word(&mut data)?;
            // These paired models preserve native Q3 world Tree(0)/inline leaf
            // semantics; Q2 represents the same direct leaf by -1-leaf.
            let model = match (index, tag, root) {
                (0, 0, 0) => ModelRoot::Tree(0),
                (1.., 1, leaf) if (leaf as usize) < leaf_count => ModelRoot::Leaf(leaf),
                _ => return Err("fixture model root exceeds shared native subset"),
            };
            models.push(model);
        }
        geometries.push(
            store
                .load_brushes(
                    side_planes,
                    brushes,
                    surfaces,
                    BrushTree {
                        planes,
                        nodes,
                        leaves,
                        leaf_brushes,
                        models,
                    },
                    vec![
                        Bounds {
                            mins: Vec3([-16384.0; 3]),
                            maxs: Vec3([16384.0; 3])
                        };
                        model_count
                    ],
                )
                .map_err(|_| "native tree fixture geometry rejected")?,
        );
        model_counts.push(model_count);
    }
    let mut queries = Vec::with_capacity(query_count);
    for _ in 0..query_count {
        let map = word(&mut data)? as usize;
        let model = word(&mut data)? as usize;
        let mask = contents(word(&mut data)?)?;
        let start = vector(&mut data)?;
        let end = vector(&mut data)?;
        let mins = vector(&mut data)?;
        let maxs = vector(&mut data)?;
        let point = vector(&mut data)?;
        if map >= geometries.len()
            || model >= model_counts[map]
            || mins.0.iter().zip(maxs.0).any(|(min, max)| *min > max)
        {
            return Err("invalid tree fixture query");
        }
        queries.push(Query {
            map,
            model,
            point,
            trace: TraceQuery {
                start,
                end,
                mins,
                maxs,
                mask,
                rules,
                entity_rules,
                pass: None,
                excluded: &[],
            },
        });
    }
    if !data.is_empty() {
        return Err("trailing brush-tree fixture bytes");
    }
    Ok(Fixture {
        store,
        geometries,
        queries,
    })
}

pub fn result(trace: Trace, point: Contents) -> [u32; WORDS] {
    // Native zeroed no-contact planes have type0; the common representation
    // has no axis. Normalize only that invalid-plane tag, preserving all floats.
    let axis = if trace.plane.normal == Vec3::default() {
        3
    } else {
        match trace.plane.axis {
            Some(Axis::X) => 0,
            Some(Axis::Y) => 1,
            Some(Axis::Z) => 2,
            None => 3,
        }
    };
    [
        trace.fraction.to_bits(),
        trace.end.0[0].to_bits(),
        trace.end.0[1].to_bits(),
        trace.end.0[2].to_bits(),
        trace.plane.normal.0[0].to_bits(),
        trace.plane.normal.0[1].to_bits(),
        trace.plane.normal.0[2].to_bits(),
        trace.plane.distance.to_bits(),
        axis,
        u32::from(trace.start_solid) | u32::from(trace.all_solid) << 1,
        trace.contents.0 as u32,
        trace.surface.0,
        point.0 as u32,
    ]
}
