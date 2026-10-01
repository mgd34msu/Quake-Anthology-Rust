//! Quake III presentation: collision host.
//!
//! Donor provenance: `src/content/q3/presentation/collision-host.ts`.
//!
//! Production host over the ported runtime in [`qa_world::collision::q3`]
//! (donor `world/collision/q3/world.ts`).

use qa_core::math::{vec3, Bounds, Vec3};
use qa_world::collision::q3::{
    create_box_model, CollisionCounters, CollisionWorld as RuntimeCollisionWorld, TraceQuery as RuntimeTraceQuery,
    TraceResult as RuntimeTraceResult, TraceShape as RuntimeTraceShape,
};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::world::*;

// ---------------------------------------------------------------------------
// collision-host.ts (unified from mirrors_present_client)
// ---------------------------------------------------------------------------

/// Trace query (`TraceQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: TraceShape,
    /// Mask.
    pub mask: i32,
    /// Model index.
    pub model_index: Option<i32>,
}

/// Trace result (`TraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceResult {
    /// Fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Collision world (`CollisionWorld`).
pub trait CollisionWorld {
    /// Trace the world.
    fn trace(&mut self, query: &TraceQuery) -> TraceResult;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Transformed model trace.
    fn transformed_trace(&mut self, query: &TraceQuery, model_index: i32, origin: Vec3, angles: Vec3) -> TraceResult;
    /// Transformed point contents.
    fn transformed_point_contents(&mut self, point: Vec3, model_index: i32, origin: Vec3, angles: Vec3) -> i32;
    /// Box-model trace (`createBoxModel(...).transformedTrace`).
    fn box_trace(&mut self, mins: Vec3, maxs: Vec3, query: &TraceQuery, origin: Vec3) -> TraceResult;
}

/// Production collision host over the ported Quake III runtime.
///
/// The trait surface is infallible, so runtime failures (bad model handles,
/// non-finite coordinates) degrade to neutral misses instead of panicking;
/// valid callers never observe them.
pub struct Q3CollisionHost {
    world: RuntimeCollisionWorld,
}

impl Q3CollisionHost {
    /// Wrap a runtime world built from decoded collision records.
    #[must_use]
    pub fn new(world: RuntimeCollisionWorld) -> Self {
        Self { world }
    }

    /// Borrow the runtime world.
    #[must_use]
    pub fn world(&self) -> &RuntimeCollisionWorld {
        &self.world
    }

    /// Shared trace counters (donor `counters` pick).
    #[must_use]
    pub fn counters(&self) -> &CollisionCounters {
        self.world.counters()
    }

    fn convert_query(query: &TraceQuery) -> RuntimeTraceQuery {
        RuntimeTraceQuery {
            start: query.start,
            end: query.end,
            shape: match query.shape {
                TraceShape::Point => RuntimeTraceShape::Point,
                TraceShape::Box { mins, maxs } => RuntimeTraceShape::Box { mins, maxs },
                TraceShape::Capsule { mins, maxs } => RuntimeTraceShape::Capsule { mins, maxs },
            },
            mask: query.mask,
            model_index: query.model_index,
            curves: None,
            player_curve_clip: None,
        }
    }

    fn convert_result(result: RuntimeTraceResult) -> TraceResult {
        TraceResult {
            fraction: result.fraction,
            end: result.end,
            solidity: match result.solidity {
                qa_world::collision::q3::world::TraceSolidity::Clear => TraceSolidity::Clear,
                qa_world::collision::q3::world::TraceSolidity::StartSolid => TraceSolidity::StartSolid,
                qa_world::collision::q3::world::TraceSolidity::AllSolid => TraceSolidity::AllSolid,
            },
            contact: match result.contact {
                qa_world::collision::q3::world::TraceContact::None => TraceContact::None,
                qa_world::collision::q3::world::TraceContact::Plane(plane) => TraceContact::Plane { plane },
            },
            contents: result.contents,
            surface_flags: result.surface_flags,
        }
    }

    fn miss(query: &TraceQuery) -> TraceResult {
        TraceResult {
            fraction: 1.0,
            end: query.end,
            solidity: TraceSolidity::Clear,
            contact: TraceContact::None,
            contents: 0,
            surface_flags: 0,
        }
    }
}

impl CollisionWorld for Q3CollisionHost {
    fn trace(&mut self, query: &TraceQuery) -> TraceResult {
        self.world
            .trace(&Self::convert_query(query))
            .map(Self::convert_result)
            .unwrap_or_else(|_| Self::miss(query))
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.world.point_contents(point, 0).unwrap_or(0)
    }

    fn transformed_trace(&mut self, query: &TraceQuery, model_index: i32, origin: Vec3, angles: Vec3) -> TraceResult {
        let mut runtime = Self::convert_query(query);
        runtime.model_index = Some(model_index);
        self.world
            .transformed_trace(&runtime, origin, angles)
            .map(Self::convert_result)
            .unwrap_or_else(|_| Self::miss(query))
    }

    fn transformed_point_contents(&mut self, point: Vec3, model_index: i32, origin: Vec3, angles: Vec3) -> i32 {
        self.world
            .transformed_point_contents(point, model_index, origin, angles)
            .unwrap_or(0)
    }

    fn box_trace(&mut self, mins: Vec3, maxs: Vec3, query: &TraceQuery, origin: Vec3) -> TraceResult {
        let model = create_box_model(Bounds { min: mins, max: maxs }, self.world.counters(), None);
        let neutral = || -> TraceResult {
            TraceResult {
                fraction: 1.0,
                end: query.end,
                solidity: TraceSolidity::Clear,
                contact: TraceContact::None,
                contents: 0,
                surface_flags: 0,
            }
        };
        let Ok(model) = model else {
            return neutral();
        };
        model
            .transformed_trace(&Self::convert_query(query), origin, vec3(0.0, 0.0, 0.0))
            .map(Self::convert_result)
            .unwrap_or_else(|_| Self::miss(query))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    use qa_core::math::Plane;
    use qa_world::collision::q3::{
        decoded_collision_map, CollisionCounters as RuntimeCounters, CollisionShader, CollisionWorldProfile,
        IndexRange, Q3BspChild, Q3CollisionBrushInput, Q3CollisionBrushSideInput, Q3CollisionGeometry,
        Q3CollisionLeafInput, Q3CollisionModelInput, Q3CollisionNodeInput,
    };

    fn host_world() -> RuntimeCollisionWorld {
        let mut planes = vec![Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
        }];
        for (normal, distance) in [
            (vec3(1.0, 0.0, 0.0), 64.0),
            (vec3(-1.0, 0.0, 0.0), 64.0),
            (vec3(0.0, 1.0, 0.0), 64.0),
            (vec3(0.0, -1.0, 0.0), 64.0),
            (vec3(0.0, 0.0, 1.0), 64.0),
            (vec3(0.0, 0.0, -1.0), 64.0),
        ] {
            planes.push(Plane { normal, distance });
        }
        let geometry = Q3CollisionGeometry {
            entities: String::new(),
            shaders: vec![CollisionShader {
                name: "solid".to_string(),
                surface_flags: 1,
                content_flags: 1,
            }],
            planes,
            nodes: vec![Q3CollisionNodeInput {
                plane: 0,
                children: [Q3BspChild::Leaf(0), Q3BspChild::Leaf(1)],
            }],
            leaves: vec![
                Q3CollisionLeafInput {
                    cluster: 0,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 1 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
                Q3CollisionLeafInput {
                    cluster: 1,
                    area: 0,
                    brushes: IndexRange { first: 0, count: 0 },
                    surfaces: IndexRange { first: 0, count: 0 },
                },
            ],
            leaf_brushes: vec![0],
            leaf_surfaces: Vec::new(),
            models: vec![Q3CollisionModelInput {
                bounds: Bounds {
                    min: vec3(-64.0, -64.0, -64.0),
                    max: vec3(64.0, 64.0, 64.0),
                },
                brushes: IndexRange { first: 0, count: 0 },
                surfaces: IndexRange { first: 0, count: 0 },
            }],
            brushes: vec![Q3CollisionBrushInput {
                shader: 0,
                sides: IndexRange { first: 0, count: 6 },
            }],
            brush_sides: (1..7)
                .map(|plane| Q3CollisionBrushSideInput { plane, shader: 0 })
                .collect(),
            vertices: Vec::new(),
            surfaces: Vec::new(),
            visibility: None,
        };
        let map = decoded_collision_map(&geometry, None).expect("decode");
        RuntimeCollisionWorld::new(
            Rc::new(map),
            CollisionWorldProfile::Disabled,
            Rc::new(RuntimeCounters::new()),
        )
    }

    fn point_query(start: Vec3, end: Vec3, mask: i32) -> TraceQuery {
        TraceQuery {
            start,
            end,
            shape: TraceShape::Point,
            mask,
            model_index: None,
        }
    }

    fn assert_wired<CW: CollisionWorld>(host: CW) -> CW {
        host
    }

    #[test]
    fn production_host_matches_runtime() {
        let mut host = assert_wired(Q3CollisionHost::new(host_world()));
        let hit = host.trace(&point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1));
        assert_eq!(hit.fraction, 0.17937499284744263f64 as f32);
        assert_eq!(hit.end, vec3(0.0, 0.0, 64.125));
        assert_eq!(hit.contents, 1);
        assert_eq!(host.point_contents(vec3(0.0, 0.0, 0.0)), 1);
        assert_eq!(host.point_contents(vec3(0.0, 0.0, 100.0)), 0);
        let shifted = host.transformed_trace(
            &point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1),
            0,
            vec3(0.0, 0.0, 10.0),
            vec3(0.0, 0.0, 0.0),
        );
        assert_eq!(shifted.fraction, 0.12937499582767487f64 as f32);
        assert_eq!(
            host.transformed_point_contents(vec3(0.0, 0.0, 0.0), 0, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)),
            1
        );
        let boxed = host.box_trace(
            vec3(-16.0, -16.0, -16.0),
            vec3(16.0, 16.0, 16.0),
            &point_query(vec3(0.0, 0.0, 50.0), vec3(0.0, 0.0, -50.0), 0x0200_0000),
            vec3(0.0, 0.0, 8.0),
        );
        assert_eq!(boxed.fraction, 0.25874999165534973f64 as f32);
        assert_eq!(boxed.end, vec3(0.0, 0.0, 24.125));
        // Runtime failures degrade to neutral misses, never panics.
        let miss = host.transformed_trace(
            &point_query(vec3(0.0, 0.0, 100.0), vec3(0.0, 0.0, -100.0), 1),
            9,
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
        );
        assert_eq!(miss.fraction, 1.0);
        assert_eq!(miss.contents, 0);
        assert_eq!(host.point_contents(vec3(f32::NAN, 0.0, 0.0)), 0);
        assert!(host.counters().c_traces.get() > 0);
    }
}
