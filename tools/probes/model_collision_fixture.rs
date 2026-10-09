//! Developer model fixture transport; every trace calls the production store.
use qa_core::primitives::{
    Axis, Bounds, ClipNode, GeometryId, ModelRotation, ModelRules, Plane, RotatedLinkBounds, Vec3,
};
use qa_world::collision::{
    CollisionStore, Contents, EntityTracePolicy, Trace, TraceQuery, TraceRules, TraceScratch,
    hulls::HullModel,
};

#[path = "brush_tree_fixture.rs"]
mod brush;
pub const WORDS: usize = brush::WORDS;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Quake,
    Quake2,
    Quake3,
}

impl Rule {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "q1" => Ok(Self::Quake),
            "q2" => Ok(Self::Quake2),
            "q3" => Ok(Self::Quake3),
            _ => Err("expected q1, q2 or q3 caller/model rules"),
        }
    }

    fn trace_rules(self) -> TraceRules {
        if self == Self::Quake3 {
            TraceRules::ARENA
        } else {
            TraceRules::LEGACY
        }
    }

    fn entity_rules(self) -> EntityTracePolicy {
        match self {
            Self::Quake => {
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake).1
            }
            Self::Quake2 => {
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1
            }
            Self::Quake3 => {
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
            }
        }
    }

    fn model_rules(self) -> ModelRules {
        let (rotation, link_bounds) = match self {
            Self::Quake => (ModelRotation::TranslationOnly, RotatedLinkBounds::Unrotated),
            Self::Quake2 => (ModelRotation::NegativeEuler, RotatedLinkBounds::MaxAbsCube),
            Self::Quake3 => (ModelRotation::TransposeBasis, RotatedLinkBounds::RadiusCube),
        };
        ModelRules {
            rotation,
            link_bounds,
        }
    }

    pub fn words(self) -> usize {
        if self == Self::Quake { 9 } else { WORDS }
    }
}

#[derive(Clone, Copy)]
pub struct Pose {
    pub origin: Vec3,
    pub angles: Vec3,
}

pub struct Fixture {
    pub store: CollisionStore,
    pub geometries: Vec<GeometryId>,
    pub queries: Vec<brush::Query>,
    pub poses: Vec<Pose>,
    pub rule: Rule,
}

fn word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data
        .split_first_chunk::<4>()
        .ok_or("truncated model fixture")?;
    *data = tail;
    Ok(u32::from_le_bytes(*head))
}

fn count(data: &mut &[u8], min: usize, max: usize) -> Result<usize, &'static str> {
    let value = word(data)? as usize;
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err("model fixture count exceeds capacity")
    }
}

fn vector(data: &mut &[u8]) -> Result<Vec3, &'static str> {
    let value = Vec3([
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
        f32::from_bits(word(data)?),
    ]);
    if value.0.iter().all(|component| component.is_finite()) {
        Ok(value)
    } else {
        Err("nonfinite model fixture vector")
    }
}

fn hulls(mut data: &[u8]) -> Result<brush::Fixture, &'static str> {
    if word(&mut data)? != 0x4c48514d || word(&mut data)? != 1 {
        return Err("unsupported hull-model fixture");
    }
    let plane_count = count(&mut data, 1, 64)?;
    let draw_count = count(&mut data, 1, 128)?;
    let clip_count = count(&mut data, 1, 128)?;
    let model_count = count(&mut data, 2, 8)?;
    let query_count = count(&mut data, 1, 50000)?;
    let mut planes = Vec::with_capacity(plane_count);
    for _ in 0..plane_count {
        let normal = vector(&mut data)?;
        let distance = f32::from_bits(word(&mut data)?);
        let axis = match word(&mut data)? {
            0 => Some(Axis::X),
            1 => Some(Axis::Y),
            2 => Some(Axis::Z),
            3 => None,
            _ => return Err("invalid hull-model axis"),
        };
        if !distance.is_finite() {
            return Err("nonfinite hull-model distance");
        }
        planes.push(Plane {
            normal,
            distance,
            axis,
        });
    }
    let mut drawing = Vec::with_capacity(draw_count);
    let mut clips = Vec::with_capacity(clip_count);
    for index in 0..draw_count + clip_count {
        let node = ClipNode {
            plane: word(&mut data)?,
            children: [word(&mut data)? as i32, word(&mut data)? as i32],
        };
        if index < draw_count {
            drawing.push(node);
        } else {
            clips.push(node);
        }
    }
    let mut models = Vec::with_capacity(model_count);
    let mut bounds = Vec::with_capacity(model_count);
    for _ in 0..model_count {
        models.push(HullModel {
            roots: [
                word(&mut data)? as i32,
                word(&mut data)? as i32,
                word(&mut data)? as i32,
            ],
        });
        bounds.push(Bounds {
            mins: vector(&mut data)?,
            maxs: vector(&mut data)?,
        });
    }
    let mut store = CollisionStore::new();
    let geometry = store
        .load_hulls(planes, drawing, clips, models, bounds)
        .map_err(|_| "hull-model fixture geometry rejected")?;
    let mut queries = Vec::with_capacity(query_count);
    for _ in 0..query_count {
        let map = word(&mut data)? as usize;
        let model = word(&mut data)? as usize;
        let mask = Contents(u64::from(word(&mut data)?));
        let start = vector(&mut data)?;
        let end = vector(&mut data)?;
        let mins = vector(&mut data)?;
        let maxs = vector(&mut data)?;
        let point = vector(&mut data)?;
        if map != 0 || model >= model_count || mins.0.iter().zip(maxs.0).any(|(a, b)| *a > b) {
            return Err("invalid hull-model query");
        }
        queries.push(brush::Query {
            map,
            model,
            point,
            trace: TraceQuery {
                start,
                end,
                mins,
                maxs,
                mask,
                rules: TraceRules::LEGACY,
                entity_rules: qa_world::collision::trace_policy(
                    qa_core::primitives::RuleSetId::Quake,
                )
                .1,
                pass: None,
                excluded: &[],
            },
        });
    }
    if !data.is_empty() {
        return Err("trailing hull-model fixture data");
    }
    Ok(brush::Fixture {
        store,
        geometries: vec![geometry],
        queries,
    })
}

pub fn load(rule: Rule, data: &[u8], mut pose_data: &[u8]) -> Result<Fixture, &'static str> {
    let brush::Fixture {
        store,
        geometries,
        queries,
    } = if rule == Rule::Quake {
        hulls(data)?
    } else {
        brush::load(data, rule.trace_rules(), rule.entity_rules())?
    };
    if word(&mut pose_data)? != 0x45534f50
        || word(&mut pose_data)? != 1
        || word(&mut pose_data)? as usize != queries.len()
    {
        return Err("model pose header/row count differs");
    }
    let mut poses = Vec::with_capacity(queries.len());
    for _ in &queries {
        poses.push(Pose {
            origin: vector(&mut pose_data)?,
            angles: vector(&mut pose_data)?,
        });
    }
    if !pose_data.is_empty() {
        return Err("trailing model pose data");
    }
    Ok(Fixture {
        store,
        geometries,
        queries,
        poses,
        rule,
    })
}

impl Fixture {
    pub fn sample(&self, index: usize, scratch: &mut TraceScratch) -> (Trace, [u32; WORDS]) {
        let query = self.queries[index];
        let pose = self.poses[index];
        let geometry = self.geometries[query.map];
        let trace = self.store.trace_transformed(
            geometry,
            query.model as u32,
            query.trace,
            pose.origin,
            pose.angles,
            self.rule.model_rules(),
            scratch,
        );
        let point = if self.rule == Rule::Quake {
            Contents::EMPTY
        } else {
            self.store.point_contents_transformed(
                geometry,
                query.model as u32,
                query.point,
                pose.origin,
                pose.angles,
                self.rule.model_rules(),
                self.rule.entity_rules(),
            )
        };
        let mut row = brush::result(trace, point);
        if self.rule == Rule::Quake {
            // Q1 trace_t has no brush contents/surface fields. Match the nine
            // original fields used by check_hull_trace, including media flags.
            row[8] = u32::from(trace.start_solid)
                | u32::from(trace.all_solid) << 1
                | u32::from(trace.in_open) << 2
                | u32::from(trace.in_water) << 3;
        }
        (trace, row)
    }
}
