//! Interactive play: the seat-to-world game layer.
//!
//! The game composition renders the map and samples input; this module is
//! everything between them: the admitted player body, the family user
//! command, the movement step, and the eye the camera follows. It follows
//! the donor `Application.step` shape (sample seat, build command, step
//! simulation, present from the player) and the qsrc physics it ports
//! (WinQuake `world.c` hull traces, `cl_input.c` command building).
//!
//! One shared path with family arms: [`PlayerBody`] and [`PlayerClip`]
//! dispatch per family, and every family reuses the same admit, step, and
//! eye flow over its own trace and movement cores. Each arm runs one
//! authoritative movement step per frame and commits the result back to
//! the sim body.

use std::collections::HashSet;
use std::rc::Rc;

use qa_bots::q1_collision::q1_collision_geometry;
use qa_bots::q2_collision::{q2_collision_geometry, Q2Collision};
use qa_bots::q3_collision::{create_source_q3_collision, Q3Collision};
use qa_bots::scene::{
    scene_expect, TraceDetail as SceneTraceDetail, TraceHit as SceneTraceHit, WorldKind as SceneWorldKind,
};
use qa_bots::scene::{
    PointContentsQuery as ScenePointContentsQuery, PointContentsResult as ScenePointContentsResult,
    Q1MoveRule as SceneQ1MoveRule, QueryTarget as SceneQueryTarget, TraceContact as SceneTraceContact,
    TracePolicy as SceneTracePolicy, TraceQuery as SceneTraceQuery, TraceResult as SceneTraceResult,
    TraceShape as SceneTraceShape,
};
use qa_bots::shared_scene::{DecodedCollisionWorld, SharedSceneQueries};
use qa_content::bsp::{read_q1_bsp, Q1BspOptions};
use qa_content::bsp2::{read_q2_bsp, to_q2_world_geometry};
use qa_content::contract::GameFamily;
use qa_core::cmd::Dialect;
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE, Q2_DONOR_PROFILE, Q3_BINARY32_PROFILE};
use qa_core::time::FrameContext as ClockFrame;
use qa_world::body::{translated_body_bounds, BodyState, LinkedBody};
use qa_world::collision::q1::{CONTENTS_EMPTY, CONTENTS_SOLID};
use qa_world::hull::{axis_box_hull, trace_hull_solid, BspPlane, HullTrace};
use qa_world::movement::q1::netquake::move_netquake;
use qa_world::movement::q1::quakeworld::move_quake_world;
use qa_world::movement::q1::types::{
    NoQ1Hooks, Q1AnimationStepInput, Q1AnimationStepResult, Q1Edition, Q1MovementInput, Q1MovementOptions,
    Q1MovementProfile, Q1MovementServices, Q1MovementState, Q1State, Q1Trace, Q1TraceMove, Q1TraceQuery,
    Q1WeaponStepInput, Q1WeaponStepResult, QwMovementInput, QwMovementProfile, QwMovementState, Q1_MOVE_WALK,
};
use qa_world::movement::q2::dimensions::Q2_PLAYER_BOUNDS;
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::q2::types::{
    Q2ContentsQuery, Q2MovementInput, Q2MovementProfile, Q2MovementServices, Q2MovementState, Q2RereleaseMovementInput,
    Q2RereleaseMovementProfile, Q2RereleaseMovementState, Q2State, Q2Surface, Q2TouchContact, Q2Trace, Q2TracePlane,
    Q2TraceQuery,
};
use qa_world::movement::q2::{move_q2_classic, move_q2_rerelease};
use qa_world::movement::q3::postures::Q3_SOURCE_POSTURES;
use qa_world::movement::q3::provider::{move_q3, NoQ3Hooks, Q3MovementProviderOptions};
use qa_world::movement::q3::types::{
    Q3MovementInput, Q3MovementProfile, Q3MovementServices, Q3MovementState, Q3Product, Q3Trace, Q3TraceQuery,
};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, MovementContinuation, MovementEnvironment, MovementExecution,
    MovementInputFields, MovementOutcome, MovementTouchContact, Q1UserCommand, Q2RereleaseUserCommand, Q2UserCommand,
    QwUserCommand, TraceContact, TraceHit, TraceShape, UserCommand as WorldUserCommand, WeaponState,
};
use qa_world::movement::Q1MovementParameters;
use qa_world::session::Simulation;
use qa_world::spatial::{ActorCollision, CollisionFamily, CollisionRole, CollisionShape};
use qa_world::triggers::TriggerTable;

use super::simulation::native_q1_spawns::{Q1EdictSet, Q1EdictTable};

/// Quake I player collision box, matching qsrc hull 1 (`gl_model.c`
/// `Mod_LoadClipnodes`): x/y half-width 16, feet at -24, head at +32.
#[must_use]
pub fn q1_player_bounds() -> Bounds {
    Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, 32.0),
    }
}

/// Quake I eye height above the feet origin (donor `eye_height`, qsrc
/// `VIEW_OFS` 22).
pub const Q1_VIEW_HEIGHT: f32 = 22.0;

/// Build the shared collision scene from raw Quake I BSP bytes.
///
/// Parses the map once, converts it into collision geometry, and fails
/// honestly when the map has no world model or no usable hull headnodes.
pub fn build_q1_scene(bytes: &[u8], map: &str) -> Result<SharedSceneQueries, String> {
    let parsed = read_q1_bsp(bytes, map, Q1BspOptions::default()).map_err(|error| error.to_string())?;
    let world = parsed
        .models
        .first()
        .ok_or_else(|| format!("{map}: BSP has no world model"))?;
    for (hull, headnode) in world.headnodes.iter().take(3).enumerate() {
        if *headnode < 0 {
            return Err(format!("{map}: world model has no hull {hull} headnode"));
        }
    }
    SharedSceneQueries::new(DecodedCollisionWorld::Q1(q1_collision_geometry(&parsed)))
        .map_err(|error| error.to_string())
}

/// Quake I link bounds for a live body (WinQuake `SV_LinkEdict`
/// `world.c:428-437`): no rotation expansion, and one unit of padding so
/// epsilon-clipped movers still meet edge-touching bodies. Item pickup
/// expansion stays out: the live path spawns no items.
fn q1_link_bounds(state: &BodyState) -> Bounds {
    let absolute = translated_body_bounds(state);
    Bounds {
        min: vec3(absolute.min.x - 1.0, absolute.min.y - 1.0, absolute.min.z - 1.0),
        max: vec3(absolute.max.x + 1.0, absolute.max.y + 1.0, absolute.max.z + 1.0),
    }
}

/// Live Q1 gamecode collision side channels, borrowed from the spawn
/// registry for one step: brush-model indices size doors, the solid
/// set admits box-solid bodies.
pub struct Q1SceneLinks<'b> {
    /// Brush-model index by door actor.
    pub door_models: &'b Q1EdictTable<u32>,
    /// Box-solid actors (monsters, the admitted player).
    pub solids: &'b Q1EdictSet,
}

/// Relink every solid live body into the shared scene in simulation
/// order, one fresh link per body. Brush doors link as solid inline
/// models (stock `SOLID_BSP` blocks movement and still takes touches);
/// box-solid gamecode actors (monsters, the admitted player) link as
/// solid boxes; marked triggers link as trigger volumes. Anything
/// gamecode left `SOLID_NOT` stays out. The trigger set is built once,
/// so classification stays linear.
pub(crate) fn link_q1_scene(
    scene: &mut SharedSceneQueries,
    simulation: &Simulation,
    triggers: &TriggerTable,
    links: Option<&Q1SceneLinks<'_>>,
) {
    scene.clear_actors();
    let marked: HashSet<&ActorId> = triggers.iter().collect();
    for actor in simulation.body_actors() {
        let Some(state) = simulation.body_state(&actor) else {
            continue;
        };
        let model = links.and_then(|links| links.door_models.get(&actor).copied());
        let solid = model.is_some() || links.is_some_and(|links| links.solids.contains(&actor));
        if !solid && !marked.contains(&actor) {
            // Stock SOLID_NOT: gamecode assigned no solidity (info
            // points, lights, items, map triggers), so the body never
            // links and can neither block nor take touches.
            continue;
        }
        let shape = model.map_or(CollisionShape::Box, CollisionShape::Model);
        let role = if solid {
            CollisionRole::Solid
        } else {
            CollisionRole::Trigger
        };
        scene.link_fresh(
            &LinkedBody {
                actor,
                state: state.clone(),
                absolute_bounds: q1_link_bounds(&state),
                // Stock Quake I has no link serial; the live play path
                // never links bodies into the body table either.
                link_count: 0,
            },
            &ActorCollision {
                family: CollisionFamily::Q1,
                shape,
                contents: CONTENTS_SOLID,
                owner: None,
                role,
                monster: false,
                dead_monster: false,
                q1_corpse: false,
                q3_owner: None,
            },
        );
    }
}

/// A corrupt hull trace (bad node numbers) becomes a blocking trace at
/// the start point instead of an error: callers are infallible movement
/// services, and stopping beats falling through the world.
fn solid_on_corrupt(result: Result<HullTrace, qa_world::WorldError>, end: Vec3) -> HullTrace {
    result.unwrap_or(HullTrace {
        fraction: 0.0,
        end,
        start_solid: true,
        all_solid: true,
        in_open: false,
        in_water: false,
        plane: qa_core::math::Plane {
            normal: vec3(0.0, 0.0, 1.0),
            distance: 0.0,
        },
        contents: CONTENTS_SOLID,
    })
}

/// Trace one entity body as a box (qsrc `SV_ClipMoveToEntity` non-BSP
/// branch): the swept box becomes a static box hull spanning
/// `entity - trace` extents, traced as a point.
pub fn trace_entity_box(
    entity_origin: Vec3,
    entity_bounds: &Bounds,
    trace_bounds: &Bounds,
    start: Vec3,
    end: Vec3,
    ops: &NumericOps,
) -> HullTrace {
    let expanded = Bounds {
        min: vec3(
            entity_origin.x + entity_bounds.min.x - trace_bounds.max.x,
            entity_origin.y + entity_bounds.min.y - trace_bounds.max.y,
            entity_origin.z + entity_bounds.min.z - trace_bounds.max.z,
        ),
        max: vec3(
            entity_origin.x + entity_bounds.max.x - trace_bounds.min.x,
            entity_origin.y + entity_bounds.max.y - trace_bounds.min.y,
            entity_origin.z + entity_bounds.max.z - trace_bounds.min.z,
        ),
    };
    solid_on_corrupt(trace_hull_solid(&axis_box_hull(&expanded), start, end, ops), end)
}

/// Empty-contents marker for tests and services.
#[must_use]
pub fn q1_empty_contents() -> i32 {
    CONTENTS_EMPTY
}

/// Canonical Quake I movement tuning (qsrc cvar defaults: `sv_gravity`
/// 800, `sv_stopspeed` 100, `sv_maxspeed` 320, `sv_accelerate` 10,
/// `sv_friction` 4; water/air companions from the ported provider tests).
#[must_use]
pub fn q1_parameters() -> Q1MovementParameters {
    Q1MovementParameters {
        gravity: 800.0,
        stop_speed: 100.0,
        max_speed: 320.0,
        spectator_max_speed: 500.0,
        accelerate: 10.0,
        air_accelerate: 1.0,
        water_accelerate: 4.0,
        friction: 4.0,
        water_friction: 2.0,
        entity_gravity: 1.0,
    }
}

/// Canonical Quake I NetQuake movement profile for a provider.
#[must_use]
pub fn q1_profile(provider: ProviderId, edition: Q1Edition) -> Q1MovementProfile {
    Q1MovementProfile {
        id: provider,
        clock: qa_core::time::ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        },
        numeric: Q1_DONOR_PROFILE,
        edition,
        parameters: q1_parameters(),
        edge_friction: 2.0,
        no_clip_angle_hack: false,
    }
}

/// Canonical QuakeWorld movement profile: same donor tuning as
/// [`q1_profile`], with the QuakeWorld command clock.
#[must_use]
pub fn qw_profile(provider: ProviderId) -> QwMovementProfile {
    QwMovementProfile {
        id: provider,
        clock: qa_core::time::ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: crate::startup::QW_COMMAND_MILLISECONDS,
        },
        numeric: Q1_DONOR_PROFILE,
        parameters: q1_parameters(),
    }
}

/// Fresh Quake I arsenal: no weapon yet, no ammo. Weapon grants arrive
/// with the spawn loadout in the weapons phase.
#[must_use]
pub fn q1_empty_arsenal(provider: ProviderId) -> ArsenalState {
    ArsenalState {
        provider,
        active_weapon: None,
        state: WeaponState::Q1 {
            frame: 0,
            attack_finished_seconds: 0.0,
            source_weapon: 0,
        },
        ammo: Vec::new(),
    }
}

/// Fresh Quake I player animation (reference pose, donor `frame: 12`).
#[must_use]
pub fn q1_rest_animation(provider: ProviderId) -> ActorAnimationState {
    ActorAnimationState {
        provider,
        state: AnimationState::Q1 {
            frame: 12,
            next_frame_seconds: 0.0,
        },
    }
}

/// Quake I movement services over the shared collision scene: one
/// scene trace covers the world hulls and every linked body, and trigger
/// overlap stays with the server's trigger sweep each tick.
pub struct Q1PlayerServices<'s> {
    ops: NumericOps,
    scene: &'s SharedSceneQueries,
    ignore: ActorId,
}

impl<'s> Q1PlayerServices<'s> {
    /// Borrow the shared scene, ignoring the moving actor's own body.
    #[must_use]
    pub fn new(scene: &'s SharedSceneQueries, ignore: &ActorId) -> Self {
        Self {
            ops: NumericOps::select(Q1_DONOR_PROFILE).expect("Q1 donor numeric profile"),
            scene,
            ignore: ignore.clone(),
        }
    }

    /// Trace the query against the shared scene, returning its hit.
    fn trace_combined(&self, query: &Q1TraceQuery) -> Q1Trace {
        let shape = match &query.shape {
            TraceShape::Point => SceneTraceShape::Point,
            // Quake I has no capsules; movement treats them as boxes.
            TraceShape::Box(bounds) | TraceShape::Capsule(bounds) => SceneTraceShape::Box { bounds: *bounds },
        };
        let move_rule = match query.policy {
            Q1TraceMove::Normal => SceneQ1MoveRule::Normal,
            Q1TraceMove::NoMonsters => SceneQ1MoveRule::NoMonsters,
            Q1TraceMove::Missile => SceneQ1MoveRule::Missile,
        };
        let scene_query = SceneTraceQuery {
            start: query.start,
            end: query.end,
            shape,
            target: SceneQueryTarget::World,
            policy: SceneTracePolicy::Q1 { move_rule, hull: None },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: Some(self.ignore.clone()),
        };
        match self.scene.trace(&scene_query) {
            Ok(trace) => q1_trace_from_scene(&trace),
            Err(_) => q1_blocked_trace(query),
        }
    }
}

/// Convert a shared-scene trace to a movement trace result.
pub(crate) fn q1_trace_from_scene(trace: &SceneTraceResult) -> Q1Trace {
    let (in_open, in_water, source_plane) = match &trace.detail {
        SceneTraceDetail::Q1 {
            in_open,
            in_water,
            source_plane,
            ..
        } => (*in_open, *in_water, *source_plane),
        _ => (
            false,
            false,
            qa_core::math::Plane {
                normal: vec3(0.0, 0.0, 0.0),
                distance: 0.0,
            },
        ),
    };
    Q1Trace {
        fraction: trace.fraction,
        end: trace.end,
        start_solid: trace.start_solid,
        all_solid: trace.all_solid,
        contact: match &trace.contact {
            SceneTraceContact::None => TraceContact::None,
            SceneTraceContact::Plane { plane } => TraceContact::Plane(*plane),
        },
        hit: hit_from_scene(&trace.hit),
        in_open,
        in_water,
        source_plane,
        surface_flags: None,
    }
}

/// A failed scene trace becomes a blocking trace at the start point
/// instead of an error: callers are infallible movement services, and
/// stopping beats falling through the world.
pub(crate) fn q1_blocked_trace(query: &Q1TraceQuery) -> Q1Trace {
    Q1Trace {
        fraction: 0.0,
        end: query.start,
        start_solid: true,
        all_solid: true,
        contact: TraceContact::None,
        hit: TraceHit::None,
        in_open: false,
        in_water: false,
        source_plane: qa_core::math::Plane {
            normal: vec3(0.0, 0.0, 0.0),
            distance: 0.0,
        },
        surface_flags: None,
    }
}

impl Q1MovementServices for Q1PlayerServices<'_> {
    fn numeric(&self) -> NumericOps {
        self.ops
    }

    fn trace(&mut self, query: Q1TraceQuery) -> Q1Trace {
        self.trace_combined(&query)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        let query = ScenePointContentsQuery {
            point,
            target: SceneQueryTarget::World,
            policy: SceneTracePolicy::Q1 {
                move_rule: SceneQ1MoveRule::Normal,
                hull: None,
            },
            numeric: Q1_DONOR_PROFILE,
            pass_actor: None,
        };
        match self.scene.point_contents(&query) {
            Ok(ScenePointContentsResult::Q1 { contents }) => contents,
            _ => CONTENTS_SOLID,
        }
    }

    fn touch(&mut self, _contact: MovementTouchContact, state: Q1State) -> MovementContinuation<Q1State> {
        MovementContinuation::Continue(state)
    }

    fn weapon_step(&mut self, input: Q1WeaponStepInput<'_>, _state: &Q1State) -> Q1WeaponStepResult {
        Q1WeaponStepResult {
            continuation: None,
            arsenal: (*input.arsenal).clone(),
            animation: (*input.animation).clone(),
            effects: Vec::new(),
        }
    }

    fn animation_step(&mut self, input: Q1AnimationStepInput<'_>) -> Q1AnimationStepResult {
        Q1AnimationStepResult {
            animation: (*input.animation).clone(),
            effects: Vec::new(),
        }
    }
}

/// Quake II eye height above the feet origin, matching the spawn
/// selection and the rerelease movement state seed.
const Q2_VIEW_HEIGHT: f32 = 22.0;

/// Build Quake II map collision from BSP bytes: parse, decode (merging
/// leaf contents), and convert into the shared collision core.
fn build_q2_collision(bytes: &[u8], map: &str) -> Result<Q2Collision, String> {
    let parsed = read_q2_bsp(bytes, map).map_err(|error| error.to_string())?;
    let decoded = to_q2_world_geometry(parsed, None).map_err(|error| error.to_string())?;
    Ok(Q2Collision::new(q2_collision_geometry(&decoded)))
}

/// Quake II movement services over map collision plus the live server
/// bodies: world traces run the shared collision core, entity traces
/// sweep every non-trigger body but the mover.
pub struct Q2PlayerServices<'s> {
    ops: NumericOps,
    collision: &'s Q2Collision,
    simulation: &'s Simulation,
    triggers: &'s TriggerTable,
    ignore: ActorId,
}

impl<'s> Q2PlayerServices<'s> {
    /// Borrow the collision world, the server simulation and trigger
    /// table, ignoring the moving actor's own body in entity traces.
    #[must_use]
    pub fn new(
        collision: &'s Q2Collision,
        simulation: &'s Simulation,
        triggers: &'s TriggerTable,
        ignore: &ActorId,
    ) -> Self {
        Self {
            ops: NumericOps::select(Q2_DONOR_PROFILE).expect("Q2 donor numeric profile"),
            collision,
            simulation,
            triggers,
            ignore: ignore.clone(),
        }
    }

    /// Trace the query against the world brushes and server bodies,
    /// returning the nearest hit.
    fn trace_combined(&self, query: &Q2TraceQuery) -> Q2Trace {
        let shape = if query.point {
            SceneTraceShape::Point
        } else {
            SceneTraceShape::Box {
                bounds: Bounds {
                    min: vec3(query.mins[0] as f32, query.mins[1] as f32, query.mins[2] as f32),
                    max: vec3(query.maxs[0] as f32, query.maxs[1] as f32, query.maxs[2] as f32),
                },
            }
        };
        let world = scene_expect(
            self.collision.trace(&SceneTraceQuery {
                start: query.start,
                end: query.end,
                shape,
                target: SceneQueryTarget::World,
                policy: SceneTracePolicy::Q2 {
                    contents_mask: query.mask,
                    leaf_contents: match query.leaf {
                        qa_world::collision::LeafContents::Stored => qa_bots::scene::LeafContents::Stored,
                        qa_world::collision::LeafContents::Merged => qa_bots::scene::LeafContents::Merged,
                    },
                },
                numeric: Q2_DONOR_PROFILE,
                pass_actor: Some(self.ignore.clone()),
            }),
            SceneWorldKind::Q2Bsp,
            "trace",
        );
        let mut best = q2_trace_from_scene(&world);
        if !query.world_only {
            let trace_bounds = Bounds {
                min: vec3(query.mins[0] as f32, query.mins[1] as f32, query.mins[2] as f32),
                max: vec3(query.maxs[0] as f32, query.maxs[1] as f32, query.maxs[2] as f32),
            };
            for actor in self.simulation.body_actors() {
                if actor == self.ignore || self.triggers.is_trigger(&actor) {
                    continue;
                }
                let Some(body) = self.simulation.body_state(&actor) else {
                    continue;
                };
                let entity = trace_entity_box(
                    body.origin,
                    &body.bounds,
                    &trace_bounds,
                    query.start,
                    query.end,
                    &self.ops,
                );
                if entity.fraction < best.fraction {
                    best = q2_trace_from_entity(&entity, actor);
                }
            }
        }
        best
    }
}

/// Convert a scene trace to a Quake II movement trace result. Surface
/// value and material default: the collision geometry stores only the
/// surface name and flags.
fn q2_trace_from_scene(trace: &qa_bots::scene::TraceResult) -> Q2Trace {
    let SceneTraceDetail::Q2 {
        contents,
        surface,
        source_plane,
        secondary,
    } = &trace.detail
    else {
        panic!("Quake II collision returned non-Quake-II trace detail");
    };
    Q2Trace {
        fraction: trace.fraction,
        end: trace.end,
        start_solid: trace.start_solid,
        all_solid: trace.all_solid,
        contact: match &trace.contact {
            qa_bots::scene::TraceContact::None => TraceContact::None,
            qa_bots::scene::TraceContact::Plane { plane } => TraceContact::Plane(*plane),
        },
        hit: hit_from_scene(&trace.hit),
        contents: *contents,
        surface: surface.as_ref().map(|surface| Q2Surface {
            name: surface.name.clone(),
            flags: surface.flags,
            value: 0,
            material: String::new(),
        }),
        source_plane: Q2TracePlane {
            normal: source_plane.normal,
            dist: f64::from(source_plane.distance),
            plane_type: source_plane.plane_type,
            signbits: source_plane.signbits,
        },
        secondary: secondary.as_ref().map(|impact| {
            (
                Q2TracePlane {
                    normal: impact.plane.normal,
                    dist: f64::from(impact.plane.distance),
                    plane_type: impact.plane.plane_type,
                    signbits: impact.plane.signbits,
                },
                impact.surface.as_ref().map(|surface| Q2Surface {
                    name: surface.name.clone(),
                    flags: surface.flags,
                    value: 0,
                    material: String::new(),
                }),
            )
        }),
    }
}

/// Convert a scene hit record to a movement hit record, shared by the
/// Quake II and III arms (the mapping is family-agnostic).
pub(crate) fn hit_from_scene(hit: &SceneTraceHit) -> TraceHit {
    match hit {
        SceneTraceHit::None => TraceHit::None,
        SceneTraceHit::World { model } => TraceHit::World { model: *model as u32 },
        SceneTraceHit::Actor { actor } => TraceHit::Actor { actor: actor.clone() },
    }
}

/// Convert a swept server body to a Quake II movement trace result.
/// Box bodies carry no brush surface, so surface detail stays empty.
fn q2_trace_from_entity(trace: &HullTrace, actor: ActorId) -> Q2Trace {
    Q2Trace {
        fraction: trace.fraction,
        end: trace.end,
        start_solid: trace.start_solid,
        all_solid: trace.all_solid,
        contact: if trace.fraction < 1.0 {
            TraceContact::Plane(trace.plane)
        } else {
            TraceContact::None
        },
        hit: TraceHit::Actor { actor },
        contents: 0,
        surface: None,
        source_plane: Q2TracePlane {
            normal: trace.plane.normal,
            dist: f64::from(trace.plane.distance),
            plane_type: 0,
            signbits: 0,
        },
        secondary: None,
    }
}

impl Q2MovementServices for Q2PlayerServices<'_> {
    fn numeric(&self) -> NumericOps {
        self.ops
    }

    fn trace(&mut self, query: Q2TraceQuery) -> Q2Trace {
        self.trace_combined(&query)
    }

    fn point_contents(&mut self, query: Q2ContentsQuery) -> (i32, i32) {
        let result = scene_expect(
            self.collision.point_contents(&ScenePointContentsQuery {
                point: query.point,
                target: SceneQueryTarget::World,
                policy: SceneTracePolicy::Q2 {
                    contents_mask: -1,
                    leaf_contents: match query.leaf {
                        qa_world::collision::LeafContents::Stored => qa_bots::scene::LeafContents::Stored,
                        qa_world::collision::LeafContents::Merged => qa_bots::scene::LeafContents::Merged,
                    },
                },
                numeric: Q2_DONOR_PROFILE,
                pass_actor: Some(self.ignore.clone()),
            }),
            SceneWorldKind::Q2Bsp,
            "contents",
        );
        match result {
            ScenePointContentsResult::Q2 { stored, merged } => (stored, merged),
            _ => panic!("Quake II collision returned non-Quake-II contents"),
        }
    }

    fn touch(&mut self, _contact: Q2TouchContact, state: Q2State) -> MovementContinuation<Q2State> {
        MovementContinuation::Continue(state)
    }
}

/// Classic Quake II movement profile: no strafe-jump hack, zero air
/// acceleration override, snapped spawn (donor `movementProfile`).
#[must_use]
pub fn q2_profile(provider: ProviderId) -> Q2MovementProfile {
    Q2MovementProfile {
        id: provider,
        clock: qa_core::time::ClockProfile::Q2Classic,
        numeric: Q2_DONOR_PROFILE,
        strafejump_hack: false,
        air_accelerate: 0.0,
        snap_initial: true,
    }
}

/// Rerelease Quake II movement profile: 25ms frames, zero air
/// acceleration override, no N64 physics (donor `movementProfile`).
#[must_use]
pub fn q2_rerelease_profile(provider: ProviderId) -> Q2RereleaseMovementProfile {
    Q2RereleaseMovementProfile {
        id: provider,
        clock: qa_core::time::ClockProfile::Q2Rerelease {
            frame_milliseconds: 25.0,
        },
        numeric: Q2_DONOR_PROFILE,
        air_accelerate: 0.0,
        n64_physics: false,
    }
}

/// Fresh Quake II arsenal: no weapon yet, no ammo. Weapon grants arrive
/// with the spawn loadout in the weapons phase.
#[must_use]
pub fn q2_empty_arsenal(provider: ProviderId) -> ArsenalState {
    ArsenalState {
        provider,
        active_weapon: None,
        state: WeaponState::Q2 {
            gun_frame: 0,
            state: 0,
            pending_weapon: None,
            machinegun_shots: 0,
            grenade_time: qa_core::time::SourceTime::Seconds(0.0),
            grenade_blew_up: false,
        },
        ammo: Vec::new(),
    }
}

/// Resting Quake II animation: standing, no duck or run.
#[must_use]
pub fn q2_rest_animation(provider: ProviderId) -> ActorAnimationState {
    ActorAnimationState {
        provider,
        state: AnimationState::Q2 {
            frame: 0,
            end_frame: 0,
            priority: 0,
            duck: false,
            run: false,
        },
    }
}

/// Movement profile matching the admitted Quake II provider.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2BodyProfile {
    /// Classic profile.
    Classic(Q2MovementProfile),
    /// Rerelease profile.
    Rerelease(Q2RereleaseMovementProfile),
}

/// Origin of either Quake II movement state (classic stores eighths).
fn q2_state_origin(state: &Q2State) -> Vec3 {
    match state {
        Q2State::Classic(state) => vec3(
            state.origin_eighths[0] as f32 / 8.0,
            state.origin_eighths[1] as f32 / 8.0,
            state.origin_eighths[2] as f32 / 8.0,
        ),
        Q2State::Rerelease(state) => state.origin,
    }
}

/// One admitted Quake II player body: the sim actor plus its
/// authoritative movement state, view angles, and command sequence.
/// State and profile always match the admitted provider (classic or
/// rerelease); the step dispatch treats any skew as a contract error.
pub struct Q2PlayerBody {
    /// Sim actor id.
    pub actor: ActorId,
    owned: OwnedActor,
    /// Authoritative movement state.
    pub state: Q2State,
    /// View angles in degrees.
    pub view_angles: Vec3,
    sequence: i32,
    arsenal: ArsenalState,
    animation: ActorAnimationState,
    profile: Q2BodyProfile,
    rerelease: Q2RereleaseMovementContext,
}

impl Q2PlayerBody {
    /// Admit a player: spawn a body at the feet origin and seed
    /// movement state with the spawn angles for the resolved provider.
    /// Non-Quake-II dialects are contract errors, never silent classic.
    pub fn admit(
        simulation: &mut Simulation,
        provider: ProviderId,
        feet: Vec3,
        angles: Vec3,
        dialect: Dialect,
    ) -> Result<Self, String> {
        let body = BodyState {
            origin: feet,
            angles,
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Q2_PLAYER_BOUNDS,
            ground: None,
        };
        let owned = simulation
            .spawn(provider.clone(), "player", Some(body), None, Vec::new())
            .map_err(|error| error.to_string())?;
        let actor = owned.id().clone();
        let zero = vec3(0.0, 0.0, 0.0);
        let (state, profile) = match dialect {
            Dialect::Q2Classic => (
                Q2State::Classic(Q2MovementState {
                    move_type: 0,
                    origin_eighths: [
                        (f64::from(feet.x) * 8.0) as i32,
                        (f64::from(feet.y) * 8.0) as i32,
                        (f64::from(feet.z) * 8.0) as i32,
                    ],
                    velocity_eighths: [0, 0, 0],
                    flags: 0,
                    time_eight_milliseconds: 0,
                    gravity: 800.0,
                    delta_angle_shorts: [0, 0, 0],
                }),
                Q2BodyProfile::Classic(q2_profile(provider.clone())),
            ),
            Dialect::Q2Rerelease => (
                Q2State::Rerelease(Q2RereleaseMovementState {
                    move_type: 0,
                    origin: feet,
                    velocity: zero,
                    flags: 0,
                    time_milliseconds: 0,
                    gravity: 800.0,
                    delta_angles: zero,
                    view_height: f64::from(Q2_VIEW_HEIGHT),
                }),
                Q2BodyProfile::Rerelease(q2_rerelease_profile(provider.clone())),
            ),
            other => {
                return Err(format!(
                    "Q2 player body needs a Quake II movement dialect, got {other:?}"
                ));
            }
        };
        Ok(Self {
            actor,
            owned,
            state,
            view_angles: angles,
            sequence: 0,
            arsenal: q2_empty_arsenal(provider.clone()),
            animation: q2_rest_animation(provider),
            profile,
            rerelease: Q2RereleaseMovementContext::new(),
        })
    }

    /// Eye origin: feet plus the Quake II view height.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        let origin = q2_state_origin(&self.state);
        vec3(origin.x, origin.y, origin.z + Q2_VIEW_HEIGHT)
    }

    /// Run one authoritative movement step for a user command, then
    /// commit the resulting origin back to the sim body. The command
    /// must match the admitted provider; mismatches are contract
    /// errors, never silent drops.
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        collision: &Q2Collision,
        command: WorldUserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        self.sequence += 1;
        match command {
            WorldUserCommand::Q2Classic(command) => self.step_classic(simulation, triggers, collision, command, frame),
            WorldUserCommand::Q2Rerelease(command) => {
                self.step_rerelease(simulation, triggers, collision, command, frame)
            }
            other => Err(format!(
                "Q2 player body needs a Quake II user command, got {:?}",
                other.dialect()
            )),
        }
    }

    /// Shared command fields; Quake II states carry no health, so the
    /// environment stays at its default.
    fn fields(&self, frame: &ClockFrame) -> MovementInputFields {
        MovementInputFields {
            actor: self.owned.clone(),
            command_sequence: self.sequence,
            frame: *frame,
            shape: TraceShape::Box(Q2_PLAYER_BOUNDS),
            current_bounds: None,
            environment: MovementEnvironment::default(),
            arsenal: self.arsenal.clone(),
            animation: self.animation.clone(),
            execution: MovementExecution::Authoritative,
        }
    }

    /// Commit a stepped origin back to the sim body.
    fn commit_origin(&self, simulation: &mut Simulation, origin: Vec3) -> Result<(), String> {
        simulation
            .set_body_origin(&self.actor, origin)
            .map_err(|error| error.to_string())
    }

    /// One classic step; rerelease-admitted bodies reject classic
    /// commands instead of stepping the wrong core.
    fn step_classic(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        collision: &Q2Collision,
        command: Q2UserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        let (Q2State::Classic(state), Q2BodyProfile::Classic(profile)) = (&self.state, &self.profile) else {
            return Err("Classic command reached a rerelease-admitted body".to_string());
        };
        let input = Q2MovementInput {
            fields: self.fields(frame),
            command,
            state: *state,
            profile: profile.clone(),
        };
        let result = {
            let mut services = Q2PlayerServices::new(collision, simulation, triggers, &self.actor);
            move_q2_classic(input, &mut services).map_err(|error| error.to_string())?
        };
        match result {
            MovementOutcome::Active { fields, state } => {
                self.view_angles = fields.view_angles;
                self.state = Q2State::Classic(state);
                let origin = q2_state_origin(&self.state);
                self.commit_origin(simulation, origin)
            }
            MovementOutcome::ActorRemoved { .. } => Err("Q2 player body was removed mid-step".to_string()),
        }
    }

    /// One rerelease step; classic-admitted bodies reject rerelease
    /// commands instead of stepping the wrong core.
    fn step_rerelease(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        collision: &Q2Collision,
        command: Q2RereleaseUserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        let (Q2State::Rerelease(state), Q2BodyProfile::Rerelease(profile)) = (&self.state, &self.profile) else {
            return Err("Rerelease command reached a classic-admitted body".to_string());
        };
        let input = Q2RereleaseMovementInput {
            fields: self.fields(frame),
            command,
            state: *state,
            profile: profile.clone(),
            view_offset: vec3(0.0, 0.0, Q2_VIEW_HEIGHT),
            snap_initial: true,
        };
        let result = {
            let mut services = Q2PlayerServices::new(collision, simulation, triggers, &self.actor);
            move_q2_rerelease(input, &mut services, &mut self.rerelease).map_err(|error| error.to_string())?
        };
        match result {
            qa_world::movement::q2::types::Q2RereleaseMovementResult::Active { fields, state, .. } => {
                self.view_angles = fields.view_angles;
                self.state = Q2State::Rerelease(state);
                let origin = q2_state_origin(&self.state);
                self.commit_origin(simulation, origin)
            }
            qa_world::movement::q2::types::Q2RereleaseMovementResult::ActorRemoved { .. } => {
                Err("Q2 player body was removed mid-step".to_string())
            }
        }
    }
}

/// Quake III eye height above the feet origin (donor `eye_height`, qsrc
/// `DEFAULT_VIEWHEIGHT` 26).
const Q3_VIEW_HEIGHT: f32 = 26.0;

/// Quake III player collision box (donor postures: x/y half-width 15,
/// feet at -24, head at +32).
const Q3_PLAYER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -15.0,
        y: -15.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 15.0,
        y: 15.0,
        z: 32.0,
    },
};

/// Build Quake III collision from raw map bytes through the shared
/// source-collision loader.
fn build_q3_collision(bytes: &[u8], map: &str) -> Result<Q3Collision, String> {
    create_source_q3_collision(bytes, map).map_err(|error| error.to_string())
}

/// Quake III movement services over map collision plus the live server
/// bodies: world traces run the shared collision core, entity traces
/// sweep every non-trigger body but the mover.
pub struct Q3PlayerServices<'s> {
    ops: NumericOps,
    collision: &'s Q3Collision,
    simulation: &'s Simulation,
    triggers: &'s TriggerTable,
    ignore: ActorId,
}

impl<'s> Q3PlayerServices<'s> {
    /// Borrow the collision world, the server simulation and trigger
    /// table, ignoring the moving actor's own body in entity traces.
    #[must_use]
    pub fn new(
        collision: &'s Q3Collision,
        simulation: &'s Simulation,
        triggers: &'s TriggerTable,
        ignore: &ActorId,
    ) -> Self {
        Self {
            ops: NumericOps::select(Q3_BINARY32_PROFILE).expect("Q3 binary32 numeric profile"),
            collision,
            simulation,
            triggers,
            ignore: ignore.clone(),
        }
    }

    /// Trace the query against the world brushes and server bodies,
    /// returning the nearest hit.
    fn trace_combined(&self, query: &Q3TraceQuery) -> Q3Trace {
        let shape = if query.point {
            SceneTraceShape::Point
        } else {
            SceneTraceShape::Box { bounds: query.bounds }
        };
        let world = scene_expect(
            self.collision.trace(&SceneTraceQuery {
                start: query.start,
                end: query.end,
                shape,
                target: SceneQueryTarget::World,
                policy: SceneTracePolicy::Q3 {
                    contents_mask: query.mask,
                    curves: query.curves,
                    player_curve_clip: query.player_curve_clip,
                },
                numeric: Q3_BINARY32_PROFILE,
                pass_actor: Some(self.ignore.clone()),
            }),
            SceneWorldKind::Q3Bsp,
            "trace",
        );
        let mut best = q3_trace_from_scene(&world);
        for actor in self.simulation.body_actors() {
            if actor == self.ignore || self.triggers.is_trigger(&actor) {
                continue;
            }
            let Some(body) = self.simulation.body_state(&actor) else {
                continue;
            };
            let entity = trace_entity_box(
                body.origin,
                &body.bounds,
                &query.bounds,
                query.start,
                query.end,
                &self.ops,
            );
            if entity.fraction < best.fraction {
                best = q3_trace_from_entity(&entity, actor);
            }
        }
        best
    }
}

/// Convert a scene trace to a Quake III movement trace result.
fn q3_trace_from_scene(trace: &qa_bots::scene::TraceResult) -> Q3Trace {
    let SceneTraceDetail::Q3 {
        contents,
        surface_flags,
        source_plane,
    } = &trace.detail
    else {
        panic!("Quake III collision returned non-Quake-III trace detail");
    };
    Q3Trace {
        fraction: trace.fraction,
        end: trace.end,
        start_solid: trace.start_solid,
        all_solid: trace.all_solid,
        contact: match &trace.contact {
            qa_bots::scene::TraceContact::None => TraceContact::None,
            qa_bots::scene::TraceContact::Plane { plane } => TraceContact::Plane(*plane),
        },
        hit: hit_from_scene(&trace.hit),
        contents: *contents,
        surface_flags: *surface_flags,
        source_plane: BspPlane {
            normal: source_plane.normal,
            distance: source_plane.distance,
            plane_type: source_plane.plane_type as u8,
            signbits: source_plane.signbits as u8,
        },
    }
}

/// Convert a swept server body to a Quake III movement trace result.
/// Box bodies carry no brush surface, so surface detail stays empty.
fn q3_trace_from_entity(trace: &HullTrace, actor: ActorId) -> Q3Trace {
    Q3Trace {
        fraction: trace.fraction,
        end: trace.end,
        start_solid: trace.start_solid,
        all_solid: trace.all_solid,
        contact: if trace.fraction < 1.0 {
            TraceContact::Plane(trace.plane)
        } else {
            TraceContact::None
        },
        hit: TraceHit::Actor { actor },
        contents: 0,
        surface_flags: 0,
        source_plane: BspPlane {
            normal: trace.plane.normal,
            distance: trace.plane.distance,
            plane_type: 0,
            signbits: 0,
        },
    }
}

impl Q3MovementServices for Q3PlayerServices<'_> {
    fn numeric(&self) -> NumericOps {
        self.ops
    }

    fn trace(&mut self, query: Q3TraceQuery) -> Q3Trace {
        self.trace_combined(&query)
    }

    fn point_contents(&mut self, point: Vec3, pass_actor: &ActorId) -> i32 {
        let result = scene_expect(
            self.collision.point_contents(&ScenePointContentsQuery {
                point,
                target: SceneQueryTarget::World,
                policy: SceneTracePolicy::Q3 {
                    contents_mask: -1,
                    curves: true,
                    player_curve_clip: true,
                },
                numeric: Q3_BINARY32_PROFILE,
                pass_actor: Some(pass_actor.clone()),
            }),
            SceneWorldKind::Q3Bsp,
            "contents",
        );
        match result {
            ScenePointContentsResult::Q3 { contents } => contents,
            _ => panic!("Quake III collision returned non-Quake-III contents"),
        }
    }
}

/// Base Quake III movement profile: 50ms server frames, no fixed-step
/// override, footsteps on (donor `movementProfile`).
#[must_use]
pub fn q3_profile(provider: ProviderId) -> Q3MovementProfile {
    Q3MovementProfile {
        id: provider,
        clock: qa_core::time::ClockProfile::Q3 {
            server_frame_milliseconds: 50.0,
            fixed_movement_milliseconds: None,
        },
        numeric: Q3_BINARY32_PROFILE,
        product: Q3Product::BaseQ3,
        fixed_milliseconds: None,
        no_footsteps: false,
    }
}

/// Fresh Quake III arsenal: no weapon yet, no ammo. Weapon grants arrive
/// with the spawn loadout in the weapons phase.
#[must_use]
pub fn q3_empty_arsenal(provider: ProviderId) -> ArsenalState {
    ArsenalState {
        provider,
        active_weapon: None,
        state: WeaponState::Q3 {
            source_weapon: 0,
            state: 0,
            time_milliseconds: 0,
        },
        ammo: Vec::new(),
    }
}

/// Resting Quake III animation: standing legs and torso, timers clear.
#[must_use]
pub fn q3_rest_animation(provider: ProviderId) -> ActorAnimationState {
    ActorAnimationState {
        provider,
        state: AnimationState::Q3 {
            legs: 0,
            torso: 0,
            legs_timer_milliseconds: 0,
            torso_timer_milliseconds: 0,
        },
    }
}

/// One admitted Quake III player body: the sim actor plus its
/// authoritative movement state, view angles, and command sequence.
pub struct Q3PlayerBody {
    /// Sim actor id.
    pub actor: ActorId,
    owned: OwnedActor,
    /// Authoritative movement state.
    pub state: Q3MovementState,
    /// View angles in degrees.
    pub view_angles: Vec3,
    sequence: i32,
    arsenal: ArsenalState,
    animation: ActorAnimationState,
    profile: Q3MovementProfile,
}

impl Q3PlayerBody {
    /// Admit a player: spawn a body at the feet origin and seed walk
    /// movement state with the spawn angles. Non-Quake-III dialects are
    /// contract errors, never silent defaults.
    pub fn admit(
        simulation: &mut Simulation,
        provider: ProviderId,
        feet: Vec3,
        angles: Vec3,
        dialect: Dialect,
    ) -> Result<Self, String> {
        if !matches!(dialect, Dialect::Q3) {
            return Err(format!(
                "Q3 player body needs a Quake III movement dialect, got {dialect:?}"
            ));
        }
        let body = BodyState {
            origin: feet,
            angles,
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Q3_PLAYER_BOUNDS,
            ground: None,
        };
        let owned = simulation
            .spawn(provider.clone(), "player", Some(body), None, Vec::new())
            .map_err(|error| error.to_string())?;
        let actor = owned.id().clone();
        let zero = vec3(0.0, 0.0, 0.0);
        let state = Q3MovementState {
            command_time_milliseconds: 0,
            movement_type: 0,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_milliseconds: 0,
            origin: feet,
            velocity: zero,
            gravity: 800.0,
            speed: 320.0,
            delta_angle_words: [0, 0, 0],
            movement_direction: 0,
            grapple_point: zero,
            flags: 0,
            view_angles: angles,
            view_height: f64::from(Q3_VIEW_HEIGHT),
            ground: TraceHit::None,
            predictable_event_sequence: 0,
            jump_pad: None,
            movement_frame: 0,
            jump_pad_frame: 0,
        };
        Ok(Self {
            actor,
            owned,
            state,
            view_angles: angles,
            sequence: 0,
            arsenal: q3_empty_arsenal(provider.clone()),
            animation: q3_rest_animation(provider.clone()),
            profile: q3_profile(provider),
        })
    }

    /// Eye origin: feet plus the Quake III view height.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        vec3(
            self.state.origin.x,
            self.state.origin.y,
            self.state.origin.z + Q3_VIEW_HEIGHT,
        )
    }

    /// Run one authoritative movement step for a user command, then
    /// commit the resulting origin back to the sim body. The command
    /// must be Quake III; anything else is a contract error.
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        collision: &Q3Collision,
        command: WorldUserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        self.sequence += 1;
        match command {
            WorldUserCommand::Q3(command) => {
                let input = Q3MovementInput {
                    fields: self.fields(frame),
                    command,
                    state: self.state.clone(),
                    profile: self.profile.clone(),
                };
                let result = {
                    let mut services = Q3PlayerServices::new(collision, simulation, triggers, &self.actor);
                    move_q3(
                        input,
                        &mut services,
                        Q3MovementProviderOptions {
                            id: self.profile.id.clone(),
                            hooks: NoQ3Hooks,
                            postures: Rc::new(|_| Q3_SOURCE_POSTURES),
                            trace_policy: None,
                            diagnostics: None,
                        },
                    )
                    .map_err(|error| error.to_string())?
                };
                match result {
                    MovementOutcome::Active { fields, state } => {
                        self.view_angles = fields.view_angles;
                        self.state = state;
                        self.commit_origin(simulation, self.state.origin)
                    }
                    MovementOutcome::ActorRemoved { .. } => Err("Q3 player body was removed mid-step".to_string()),
                }
            }
            other => Err(format!(
                "Q3 player body needs a Quake III user command, got {:?}",
                other.dialect()
            )),
        }
    }

    /// Shared command fields; the environment stays at its default.
    fn fields(&self, frame: &ClockFrame) -> MovementInputFields {
        MovementInputFields {
            actor: self.owned.clone(),
            command_sequence: self.sequence,
            frame: *frame,
            shape: TraceShape::Box(Q3_PLAYER_BOUNDS),
            current_bounds: None,
            environment: MovementEnvironment::default(),
            arsenal: self.arsenal.clone(),
            animation: self.animation.clone(),
            execution: MovementExecution::Authoritative,
        }
    }

    /// Commit a stepped origin back to the sim body.
    fn commit_origin(&self, simulation: &mut Simulation, origin: Vec3) -> Result<(), String> {
        simulation
            .set_body_origin(&self.actor, origin)
            .map_err(|error| error.to_string())
    }
}

/// Movement profile matching the admitted Quake I provider.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1BodyProfile {
    /// NetQuake profile.
    Netquake(Q1MovementProfile),
    /// QuakeWorld profile.
    Quakeworld(QwMovementProfile),
}

/// Origin of either Quake I movement state.
fn q1_state_origin(state: &Q1State) -> Vec3 {
    match state {
        Q1State::Netquake(state) => state.origin,
        Q1State::Quakeworld(state) => state.origin,
    }
}

/// One admitted Quake I player body: the sim actor plus its
/// authoritative movement state, view angles, and command sequence.
/// State and profile always match the admitted provider (NetQuake or
/// QuakeWorld); the step dispatch treats any skew as a contract error.
pub struct Q1PlayerBody {
    /// Sim actor id.
    pub actor: ActorId,
    owned: OwnedActor,
    /// Authoritative movement state.
    pub state: Q1State,
    /// View angles in degrees.
    pub view_angles: Vec3,
    sequence: i32,
    arsenal: ArsenalState,
    animation: ActorAnimationState,
    profile: Q1BodyProfile,
}

impl Q1PlayerBody {
    /// Admit a player: spawn a body at the feet origin and seed walk
    /// movement state with the spawn angles for the resolved provider.
    /// Non-Quake-I dialects are contract errors, never silent NetQuake.
    pub fn admit(
        simulation: &mut Simulation,
        provider: ProviderId,
        feet: Vec3,
        angles: Vec3,
        dialect: Dialect,
        q1_edition: Q1Edition,
    ) -> Result<Self, String> {
        let body = BodyState {
            origin: feet,
            angles,
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: q1_player_bounds(),
            ground: None,
        };
        let owned = simulation
            .spawn(provider.clone(), "player", Some(body), None, Vec::new())
            .map_err(|error| error.to_string())?;
        let actor = owned.id().clone();
        let (state, profile) = match dialect {
            Dialect::Q1Netquake => (
                Q1State::Netquake(Q1MovementState {
                    origin: feet,
                    velocity: vec3(0.0, 0.0, 0.0),
                    angles,
                    old_origin: feet,
                    angular_velocity: vec3(0.0, 0.0, 0.0),
                    view_angles: angles,
                    punch_angles: vec3(0.0, 0.0, 0.0),
                    move_type: Q1_MOVE_WALK,
                    flags: 0,
                    ground: TraceHit::None,
                    water_level: 0,
                    water_type: CONTENTS_EMPTY,
                    teleport_time_seconds: 0.0,
                    water_jump_direction: vec3(0.0, 0.0, 0.0),
                    ideal_pitch: 0.0,
                    fix_angle: false,
                    health: 100.0,
                }),
                Q1BodyProfile::Netquake(q1_profile(provider.clone(), q1_edition)),
            ),
            Dialect::Q1Quakeworld => (
                Q1State::Quakeworld(QwMovementState {
                    origin: feet,
                    velocity: vec3(0.0, 0.0, 0.0),
                    angles,
                    old_buttons: 0,
                    water_jump_time_seconds: 0.0,
                    dead: false,
                    spectator: 0,
                    ground: TraceHit::None,
                }),
                Q1BodyProfile::Quakeworld(qw_profile(provider.clone())),
            ),
            other => {
                return Err(format!(
                    "Q1 player body needs a Quake I movement dialect, got {other:?}"
                ));
            }
        };
        Ok(Self {
            actor,
            owned,
            state,
            view_angles: angles,
            sequence: 0,
            arsenal: q1_empty_arsenal(provider.clone()),
            animation: q1_rest_animation(provider),
            profile,
        })
    }

    /// Eye origin: feet plus the Quake I view height.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        let origin = q1_state_origin(&self.state);
        vec3(origin.x, origin.y, origin.z + Q1_VIEW_HEIGHT)
    }

    /// Run one authoritative movement step for a user command, then
    /// commit the resulting origin back to the sim body. Solid bodies
    /// relink into the shared scene once per step; gamecode side
    /// channels resolve through the spawn registry when the live world
    /// passes them. The command must match the admitted provider;
    /// mismatches are contract errors, never silent drops.
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        scene: &mut SharedSceneQueries,
        links: Option<&Q1SceneLinks<'_>>,
        command: WorldUserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        self.sequence += 1;
        link_q1_scene(scene, simulation, triggers, links);
        match command {
            WorldUserCommand::Q1Netquake(command) => self.step_netquake(simulation, scene, command, frame),
            WorldUserCommand::Q1Quakeworld(command) => self.step_quakeworld(simulation, scene, command, frame),
            other => Err(format!(
                "Q1 player body needs a Quake I user command, got {:?}",
                other.dialect()
            )),
        }
    }

    /// Shared command fields; NetQuake reports live health while
    /// QuakeWorld carries none and uses the default environment.
    fn fields(&self, frame: &ClockFrame) -> MovementInputFields {
        let environment = match &self.state {
            Q1State::Netquake(state) => MovementEnvironment {
                health: state.health,
                ..MovementEnvironment::default()
            },
            Q1State::Quakeworld(_) => MovementEnvironment::default(),
        };
        MovementInputFields {
            actor: self.owned.clone(),
            command_sequence: self.sequence,
            frame: *frame,
            shape: TraceShape::Box(q1_player_bounds()),
            current_bounds: None,
            environment,
            arsenal: self.arsenal.clone(),
            animation: self.animation.clone(),
            execution: MovementExecution::Authoritative,
        }
    }

    /// Commit a stepped origin back to the sim body.
    fn commit_origin(&self, simulation: &mut Simulation, origin: Vec3) -> Result<(), String> {
        simulation
            .set_body_origin(&self.actor, origin)
            .map_err(|error| error.to_string())
    }

    /// One NetQuake step; QuakeWorld-admitted bodies reject NetQuake
    /// commands instead of stepping the wrong core.
    fn step_netquake(
        &mut self,
        simulation: &mut Simulation,
        scene: &SharedSceneQueries,
        command: Q1UserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        let (Q1State::Netquake(state), Q1BodyProfile::Netquake(profile)) = (&self.state, &self.profile) else {
            return Err("NetQuake command reached a QuakeWorld-admitted body".to_string());
        };
        let input = Q1MovementInput {
            fields: self.fields(frame),
            command,
            state: state.clone(),
            profile: profile.clone(),
        };
        let options = Q1MovementOptions::<NoQ1Hooks>::default();
        let result = {
            let mut services = Q1PlayerServices::new(scene, &self.actor);
            move_netquake(input, &mut services, options).map_err(|error| error.to_string())?
        };
        match result {
            MovementOutcome::Active { fields, state } => {
                self.view_angles = fields.view_angles;
                self.state = Q1State::Netquake(state);
                let origin = q1_state_origin(&self.state);
                self.commit_origin(simulation, origin)
            }
            MovementOutcome::ActorRemoved { .. } => Err("Q1 player body was removed mid-step".to_string()),
        }
    }

    /// One QuakeWorld step; NetQuake-admitted bodies reject QuakeWorld
    /// commands instead of stepping the wrong core.
    fn step_quakeworld(
        &mut self,
        simulation: &mut Simulation,
        scene: &SharedSceneQueries,
        command: QwUserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        let (Q1State::Quakeworld(state), Q1BodyProfile::Quakeworld(profile)) = (&self.state, &self.profile) else {
            return Err("QuakeWorld command reached a NetQuake-admitted body".to_string());
        };
        let input = QwMovementInput {
            fields: self.fields(frame),
            command,
            state: state.clone(),
            profile: profile.clone(),
        };
        let options = Q1MovementOptions::<NoQ1Hooks>::default();
        let result = {
            let mut services = Q1PlayerServices::new(scene, &self.actor);
            move_quake_world(input, &mut services, options).map_err(|error| error.to_string())?
        };
        match result {
            MovementOutcome::Active { fields, state } => {
                self.view_angles = fields.view_angles;
                self.state = Q1State::Quakeworld(state);
                let origin = q1_state_origin(&self.state);
                self.commit_origin(simulation, origin)
            }
            MovementOutcome::ActorRemoved { .. } => Err("Q1 player body was removed mid-step".to_string()),
        }
    }
}

/// Eye height above the feet origin per family (Quake I/II 22, Quake III
/// 26, matching the spawn selection).
#[must_use]
pub fn eye_height_for_family(family: GameFamily) -> f32 {
    match family {
        GameFamily::Q1 | GameFamily::Q2 => 22.0,
        GameFamily::Q3 => 26.0,
    }
}

/// Default play bindings as direct action targets: the same keys as the
/// donor defaults (WASD, Space, Ctrl, Shift, Mouse1, Tab, gamepad face
/// buttons), but bound straight to input actions so presses drive seat
/// buttons without a command buffer (the game composition wires none; the
/// `+command` text path would append to a null registry and go nowhere).
/// Wheel and weapon-wheel entries need the command buffer and arrive with
/// weapon selection.
#[must_use]
pub fn play_action_bindings(dialect: Dialect) -> Vec<qa_client::input::InputBinding> {
    use qa_client::input::bindings::named_physical_input;
    use qa_client::input::{InputAction, InputBinding, InputBindingTarget};

    let q1 = dialect.is_q1();
    let rows: &[(&str, InputAction)] = &[
        ("w", InputAction::Forward),
        ("s", InputAction::Back),
        ("a", InputAction::MoveLeft),
        ("d", InputAction::MoveRight),
        ("SPACE", if q1 { InputAction::Jump } else { InputAction::MoveUp }),
        ("CTRL", InputAction::MoveDown),
        ("SHIFT", InputAction::Walk),
        ("MOUSE1", InputAction::Attack),
        ("TAB", InputAction::Scores),
        ("GAMEPAD_RIGHT_TRIGGER", InputAction::Attack),
        (
            "GAMEPAD_A_BUTTON",
            if q1 { InputAction::Jump } else { InputAction::MoveUp },
        ),
        ("GAMEPAD_B_BUTTON", InputAction::MoveDown),
        ("GAMEPAD_X_BUTTON", InputAction::Use),
        ("GAMEPAD_BACK", InputAction::Scores),
    ];
    rows.iter()
        .filter_map(|(name, action)| {
            named_physical_input(name, 0).map(|input| InputBinding {
                input,
                target: InputBindingTarget::Action(*action),
            })
        })
        .collect()
}

/// Movement provider for a launch selection: the donor selects movement
/// by family or exact product (`--movement q1|q2|q3|qw|PRODUCT`), so a
/// QuakeWorld product moves as QuakeWorld even on Quake I maps, and a
/// rerelease edition moves as rerelease. Mirrors the client-family and
/// clock-profile resolution; the catalog-family half of product
/// resolution already ran upstream.
#[must_use]
pub fn movement_dialect_for_selection(movement: GameFamily, movement_product: Option<&str>, edition: &str) -> Dialect {
    if movement_product == Some("q1-quakeworld") {
        return Dialect::Q1Quakeworld;
    }
    match movement {
        GameFamily::Q1 => Dialect::Q1Netquake,
        GameFamily::Q2 => {
            if edition == "rerelease" {
                Dialect::Q2Rerelease
            } else {
                Dialect::Q2Classic
            }
        }
        GameFamily::Q3 => Dialect::Q3,
    }
}

/// Edition of the resolved movement content: the exact movement product
/// when one was selected, else classic (every family base product is a
/// classic edition, and the donor defaults movement the same way).
pub fn movement_content_edition(
    catalog: &qa_content::catalog::InstalledCatalog,
    movement_product: Option<&str>,
) -> Result<String, String> {
    match movement_product {
        Some(id) => catalog
            .require(id)
            .map(|product| product.expectation.edition.clone())
            .map_err(|error| error.to_string()),
        None => Ok("classic".to_string()),
    }
}

/// Quake I profile edition for a movement content edition: rerelease
/// only gates gib bounce, never walk physics (donor `movementProfile`).
#[must_use]
pub fn q1_edition_for_movement(edition: &str) -> Q1Edition {
    if edition == "rerelease" {
        Q1Edition::Rerelease
    } else {
        Q1Edition::Classic
    }
}

/// Simulation spawn provider id for a catalog family and campaign.
#[must_use]
pub fn provider_for_product(family: GameFamily, campaign: &str) -> ProviderId {
    let namespace = match family {
        GameFamily::Q1 => "q1",
        GameFamily::Q2 => "q2",
        GameFamily::Q3 => "q3",
    };
    ProviderId::new(namespace, campaign)
}

/// Admitted player body for any family: one enum, one step dispatch.
/// Every family rides the same admit, step, and eye flow over its own
/// trace and movement cores.
pub enum PlayerBody {
    /// Quake I player.
    Q1(Q1PlayerBody),
    /// Quake II player.
    Q2(Q2PlayerBody),
    /// Quake III player.
    Q3(Q3PlayerBody),
}

/// Map collision for any family, matching [`PlayerBody`].
pub enum PlayerClip {
    /// Quake I shared collision scene (boxed: the scene dwarfs the hulls).
    Q1(Box<SharedSceneQueries>),
    /// Quake II collision (boxed: the shape store dwarfs the hulls).
    Q2(Box<Q2Collision>),
    /// Quake III collision (boxed: same reason).
    Q3(Box<Q3Collision>),
}

/// Admit a player for a catalog family and resolved movement provider,
/// or `None` when the family has no body wired yet (the camera falls
/// back to the static spawn, exactly the pre-play behavior, until its
/// arm lands). A provider outside the family's own is a contract error.
/// Cross-family movement (donor presets like `q2-q1-q3`) also returns
/// `None` until every body can step against every map clip; erroring
/// here would refuse to load the map at all. The movement content
/// edition seeds the Quake I profile (gib bounce); Quake II/III physics
/// ride the dialect and ignore it.
pub fn admit_player(
    simulation: &mut Simulation,
    family: GameFamily,
    provider: ProviderId,
    feet: Vec3,
    angles: Vec3,
    dialect: Dialect,
    movement_edition: &str,
) -> Result<Option<PlayerBody>, String> {
    let matched = match family {
        GameFamily::Q1 => dialect.is_q1(),
        GameFamily::Q2 => dialect.is_q2(),
        GameFamily::Q3 => matches!(dialect, Dialect::Q3),
    };
    if !matched {
        return Ok(None);
    }
    match family {
        GameFamily::Q1 => Ok(Some(PlayerBody::Q1(Q1PlayerBody::admit(
            simulation,
            provider,
            feet,
            angles,
            dialect,
            q1_edition_for_movement(movement_edition),
        )?))),
        GameFamily::Q2 => Ok(Some(PlayerBody::Q2(Q2PlayerBody::admit(
            simulation, provider, feet, angles, dialect,
        )?))),
        GameFamily::Q3 => Ok(Some(PlayerBody::Q3(Q3PlayerBody::admit(
            simulation, provider, feet, angles, dialect,
        )?))),
    }
}

/// Build map collision for a catalog family, or `None` when the family
/// has no collision wired yet (matching [`admit_player`]).
pub fn build_clip(bytes: &[u8], map: &str, family: GameFamily) -> Result<Option<PlayerClip>, String> {
    match family {
        GameFamily::Q1 => Ok(Some(PlayerClip::Q1(Box::new(build_q1_scene(bytes, map)?)))),
        GameFamily::Q2 => Ok(Some(PlayerClip::Q2(Box::new(build_q2_collision(bytes, map)?)))),
        GameFamily::Q3 => Ok(Some(PlayerClip::Q3(Box::new(build_q3_collision(bytes, map)?)))),
    }
}

impl PlayerBody {
    /// Eye origin plus view angles for the follow camera.
    #[must_use]
    pub fn eye(&self) -> (Vec3, Vec3) {
        match self {
            PlayerBody::Q1(player) => (player.eye(), player.view_angles),
            PlayerBody::Q2(player) => (player.eye(), player.view_angles),
            PlayerBody::Q3(player) => (player.eye(), player.view_angles),
        }
    }

    /// The body's simulation actor: the same identity the driving seat's
    /// [`LocalPlayer`](super::input::LocalPlayer) controls. The body is
    /// the embodiment half of that one concept, never a parallel player.
    #[must_use]
    pub fn actor(&self) -> &ActorId {
        match self {
            PlayerBody::Q1(player) => &player.actor,
            PlayerBody::Q2(player) => &player.actor,
            PlayerBody::Q3(player) => &player.actor,
        }
    }

    /// Run one authoritative movement step for a world user command. The
    /// command dialect must match the player family; mismatches are
    /// contract errors, never silent drops. Quake I gamecode side
    /// channels resolve through the spawn registry when the live world
    /// passes them.
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        clip: &mut PlayerClip,
        links: Option<&Q1SceneLinks<'_>>,
        command: WorldUserCommand,
    ) -> Result<(), String> {
        let frame = simulation.frame();
        match (self, clip) {
            (PlayerBody::Q1(player), PlayerClip::Q1(scene)) => {
                player.step(simulation, triggers, scene, links, command, &frame)
            }
            (PlayerBody::Q2(player), PlayerClip::Q2(clip)) => player.step(simulation, triggers, clip, command, &frame),
            (PlayerBody::Q3(player), PlayerClip::Q3(clip)) => player.step(simulation, triggers, clip, command, &frame),
            _ => Err("Player body and clip belong to different families".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_content::catalog::DiscoverContentOptions;
    use qa_content::BspKind;
    use qa_core::time::{FramePhase, SourceTime};
    use qa_world::movement::types::Q3UserCommand;

    use super::*;
    use crate::bootstrap::live_proof::{require_live_corpus, require_live_data};
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn e1m1_bsp_bytes() -> Option<Vec<u8>> {
        let root = require_live_corpus("Q1 Steel data", &["q1"])?;
        let catalog = require_live_data(
            "Q1 installed-content catalog",
            qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok(),
        )?;
        let mounts = require_live_data(
            "q1-classic-id1 mounts for maps/e1m1.bsp",
            super::super::windowed_scene::open_product_mounts(&catalog, "q1-classic-id1", "maps/e1m1.bsp").ok(),
        )?;
        require_live_data(
            "maps/e1m1.bsp bytes",
            mounts.read(qa_content::mounts::ResourceRef::Path("maps/e1m1.bsp")).ok(),
        )
    }

    fn e1m1_spawn_feet_and_angles(bytes: &[u8]) -> (Vec3, Vec3) {
        let parsed = read_q1_bsp(bytes, "maps/e1m1.bsp", Q1BspOptions::default()).unwrap();
        let records: Vec<Vec<(String, String)>> =
            parsed.entity_list.into_iter().map(|entity| entity.properties).collect();
        let spawn = super::super::windowed_scene::select_spawn(&records, BspKind::Q1).expect("e1m1 spawn");
        let feet = vec3(spawn.origin.x, spawn.origin.y, spawn.origin.z - Q1_VIEW_HEIGHT);
        (feet, spawn.angles)
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q1_e1m1_walk_ignores_nonsolid_stops_at_solid() {
        let Some(bytes) = e1m1_bsp_bytes() else {
            return;
        };
        let mut scene = build_q1_scene(&bytes, "maps/e1m1.bsp").unwrap();
        let (feet, angles) = e1m1_spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let yaw = f64::from(angles.y).to_radians();
        let mut body_at = |distance: f64| {
            let simulation = server.simulation_mut();
            simulation
                .spawn(
                    player_provider(),
                    "e1m1-body",
                    Some(BodyState {
                        origin: vec3(
                            feet.x + (distance * yaw.cos()) as f32,
                            feet.y + (distance * yaw.sin()) as f32,
                            feet.z,
                        ),
                        angles: vec3(0.0, 0.0, 0.0),
                        velocity: vec3(0.0, 0.0, 0.0),
                        bounds: Bounds {
                            min: vec3(-16.0, -16.0, -32.0),
                            max: vec3(16.0, 16.0, 32.0),
                        },
                        ground: None,
                    }),
                    None,
                    Vec::new(),
                )
                .unwrap()
        };
        // The near body stays SOLID_NOT (never recorded); the far body
        // is gamecode-solid. The old per-trace box loop swept both and
        // stalled at the near one.
        let _phantom = body_at(48.0);
        let solid = body_at(96.0);
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Netquake,
                Q1Edition::Classic,
            )
            .unwrap()
        };
        let door_models = Q1EdictTable::new();
        let mut solids = Q1EdictSet::new();
        solids.insert(solid.id());
        solids.insert(&player.actor);
        let links = Q1SceneLinks {
            door_models: &door_models,
            solids: &solids,
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            let command = WorldUserCommand::Q1Netquake(forward_command(angles, time));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &mut scene,
                    Some(&links),
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q1_state_origin(&player.state);
        let traveled = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(
            traveled > 40.0,
            "player stalled at the non-solid body: traveled {traveled}"
        );
        assert!(
            traveled < 96.0 - 16.0 - 16.0 + 2.0,
            "player passed through the solid body: traveled {traveled}"
        );
        assert!(
            moved.z <= feet.z + 1.0 && moved.z > feet.z - 40.0,
            "player left the ramp: {moved:?} from {feet:?}"
        );
    }

    fn start_bsp_bytes() -> Option<Vec<u8>> {
        let root = require_live_corpus("Q1 Steel data", &["q1"])?;
        let catalog = require_live_data(
            "Q1 installed-content catalog",
            qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok(),
        )?;
        let mounts = require_live_data(
            "q1-classic-id1 mounts for maps/start.bsp",
            super::super::windowed_scene::open_product_mounts(&catalog, "q1-classic-id1", "maps/start.bsp").ok(),
        )?;
        require_live_data(
            "maps/start.bsp bytes",
            mounts
                .read(qa_content::mounts::ResourceRef::Path("maps/start.bsp"))
                .ok(),
        )
    }

    fn q1_server() -> qa_world::server::Server<qa_guest::server::GuestServerLogic> {
        let options = ApplicationOptions {
            product: "q1-classic-id1".to_string(),
            map: "maps/start.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = StartupConfig::from_options(&options).unwrap();
        open_server(&config).unwrap()
    }

    fn player_provider() -> ProviderId {
        ProviderId::new("q1", "id1")
    }

    fn command_frame(frame: i32, time: f64, step: f64) -> ClockFrame {
        ClockFrame {
            frame,
            time: SourceTime::Seconds(time as f32),
            elapsed: SourceTime::Seconds(step as f32),
            phase: FramePhase::EntityPhysics,
        }
    }

    fn forward_command(view_angles: Vec3, time: f64) -> Q1UserCommand {
        Q1UserCommand {
            acknowledged_server_time_seconds: time,
            view_angles,
            forward_move: 200.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        }
    }

    fn still_command(view_angles: Vec3, time: f64) -> Q1UserCommand {
        Q1UserCommand {
            acknowledged_server_time_seconds: time,
            view_angles,
            forward_move: 0.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        }
    }

    fn qw_forward_command(angles: Vec3) -> QwUserCommand {
        QwUserCommand {
            milliseconds: 16,
            angles,
            forward_move: 200.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
        }
    }

    fn base1_bsp_bytes() -> Option<Vec<u8>> {
        let root = require_live_corpus("Q2 Steel data", &["q2"])?;
        let catalog = require_live_data(
            "Q2 installed-content catalog",
            qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok(),
        )?;
        let mounts = require_live_data(
            "q2-classic-baseq2 mounts for maps/base1.bsp",
            super::super::windowed_scene::open_product_mounts(&catalog, "q2-classic-baseq2", "maps/base1.bsp").ok(),
        )?;
        require_live_data(
            "maps/base1.bsp bytes",
            mounts
                .read(qa_content::mounts::ResourceRef::Path("maps/base1.bsp"))
                .ok(),
        )
    }

    fn q2_server() -> qa_world::server::Server<qa_guest::server::GuestServerLogic> {
        let options = ApplicationOptions {
            product: "q2-classic-baseq2".to_string(),
            map: "maps/base1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = StartupConfig::from_options(&options).unwrap();
        open_server(&config).unwrap()
    }

    fn q2_provider() -> ProviderId {
        ProviderId::new("q2", "baseq2")
    }

    fn q2_spawn_feet_and_angles(bytes: &[u8]) -> (Vec3, Vec3) {
        let records = super::super::play_world::decode_map_entities(bytes, "maps/base1.bsp", BspKind::Q2)
            .unwrap()
            .records;
        let spawn = super::super::windowed_scene::select_spawn(&records, BspKind::Q2).expect("base1 spawn");
        let feet = vec3(spawn.origin.x, spawn.origin.y, spawn.origin.z - Q2_VIEW_HEIGHT);
        (feet, spawn.angles)
    }

    fn angle_shorts(angles: Vec3) -> [i32; 3] {
        [
            (f64::from(angles.x) * 65536.0 / 360.0) as i32,
            (f64::from(angles.y) * 65536.0 / 360.0) as i32,
            (f64::from(angles.z) * 65536.0 / 360.0) as i32,
        ]
    }

    fn q2_classic_forward_command(angles: Vec3) -> Q2UserCommand {
        Q2UserCommand {
            milliseconds: 16,
            angle_shorts: angle_shorts(angles),
            forward_move: 200.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 0,
            impulse: 0,
            light_level: 0,
        }
    }

    fn q2_rerelease_forward_command(angles: Vec3) -> Q2RereleaseUserCommand {
        Q2RereleaseUserCommand {
            milliseconds: 16,
            angles,
            forward_move: 200.0,
            side_move: 0.0,
            buttons: 0,
            server_frame: 0,
        }
    }

    fn spawn_feet_and_angles(bytes: &[u8]) -> (Vec3, Vec3) {
        let parsed = read_q1_bsp(bytes, "maps/start.bsp", Q1BspOptions::default()).unwrap();
        let records: Vec<Vec<(String, String)>> =
            parsed.entity_list.into_iter().map(|entity| entity.properties).collect();
        let spawn = super::super::windowed_scene::select_spawn(&records, BspKind::Q1).expect("start spawn");
        let feet = vec3(spawn.origin.x, spawn.origin.y, spawn.origin.z - Q1_VIEW_HEIGHT);
        (feet, spawn.angles)
    }

    #[test]
    fn play_bindings_cover_the_movement_keys() {
        use qa_client::input::{InputAction, InputBindingTarget};
        use qa_core::cmd::Dialect;

        for dialect in [Dialect::Q1Netquake, Dialect::Q2Classic, Dialect::Q3] {
            let bindings = play_action_bindings(dialect);
            assert_eq!(bindings.len(), 14, "{dialect:?} resolves every row");
            let actions: Vec<InputAction> = bindings
                .iter()
                .map(|binding| match &binding.target {
                    InputBindingTarget::Action(action) => *action,
                    InputBindingTarget::Command(text) => panic!("command target {text}"),
                })
                .collect();
            for action in [
                InputAction::Forward,
                InputAction::Back,
                InputAction::MoveLeft,
                InputAction::MoveRight,
                InputAction::MoveDown,
                InputAction::Walk,
                InputAction::Attack,
                InputAction::Scores,
                InputAction::Use,
            ] {
                assert!(actions.contains(&action), "{dialect:?} binds {action:?}");
            }
            let jump = if dialect.is_q1() {
                InputAction::Jump
            } else {
                InputAction::MoveUp
            };
            assert!(actions.contains(&jump), "{dialect:?} binds {jump:?}");
        }
    }

    #[test]
    fn entity_box_trace_blocks_at_the_face() {
        let ops = NumericOps::select(Q1_DONOR_PROFILE).unwrap();
        let trace = trace_entity_box(
            vec3(0.0, 0.0, 0.0),
            &Bounds {
                min: vec3(-32.0, -32.0, -32.0),
                max: vec3(32.0, 32.0, 32.0),
            },
            &Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
            vec3(-100.0, 0.0, 0.0),
            vec3(100.0, 0.0, 0.0),
            &ops,
        );
        assert!(!trace.start_solid);
        assert!((trace.fraction - 0.34).abs() < 0.02, "fraction {}", trace.fraction);
        assert!((trace.end.x + 32.0).abs() < 1.0, "end {:?}", trace.end);
    }

    #[test]
    fn admit_places_feet_and_eye() {
        let mut server = q1_server();
        let simulation = server.simulation_mut();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        let player = Q1PlayerBody::admit(
            simulation,
            player_provider(),
            feet,
            angles,
            Dialect::Q1Netquake,
            Q1Edition::Classic,
        )
        .unwrap();
        let body = simulation.body_state(&player.actor).expect("player body");
        assert_eq!(body.origin, feet);
        assert_eq!(player.eye(), vec3(0.0, 0.0, 54.0));
        assert_eq!(player.view_angles, angles);
        let Q1State::Netquake(state) = &player.state else {
            panic!("NetQuake admit seeds NetQuake state");
        };
        assert_eq!(state.move_type, Q1_MOVE_WALK);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn start_spawn_is_empty_and_floor_is_solid() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let scene = build_q1_scene(&bytes, "maps/start.bsp").unwrap();
        let policy = SceneTracePolicy::Q1 {
            move_rule: SceneQ1MoveRule::Normal,
            hull: None,
        };
        let (feet, _) = spawn_feet_and_angles(&bytes);
        let eye = vec3(feet.x, feet.y, feet.z + Q1_VIEW_HEIGHT);
        for point in [eye, feet] {
            let contents = scene
                .point_contents(&ScenePointContentsQuery {
                    point,
                    target: SceneQueryTarget::World,
                    policy,
                    numeric: Q1_DONOR_PROFILE,
                    pass_actor: None,
                })
                .unwrap();
            assert_eq!(
                contents,
                ScenePointContentsResult::Q1 {
                    contents: CONTENTS_EMPTY
                }
            );
        }
        let down = scene
            .trace(&SceneTraceQuery {
                start: feet,
                end: vec3(feet.x, feet.y, feet.z - 256.0),
                shape: SceneTraceShape::Box {
                    bounds: q1_player_bounds(),
                },
                target: SceneQueryTarget::World,
                policy,
                numeric: Q1_DONOR_PROFILE,
                pass_actor: None,
            })
            .unwrap();
        assert!(!down.start_solid, "spawn feet start inside solid");
        assert!(down.fraction < 1.0, "no floor within 256 units of spawn");
        let SceneTraceContact::Plane { plane } = down.contact else {
            panic!("floor contact {:?}", down.contact);
        };
        assert!(plane.normal.z > 0.7, "floor plane {:?}", plane.normal);
        assert!(down.end.z < feet.z, "floor end {:?}", down.end);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q1_player_walks_forward_on_start() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let mut scene = build_q1_scene(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Netquake,
                Q1Edition::Classic,
            )
            .unwrap()
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            let command = WorldUserCommand::Q1Netquake(forward_command(angles, time));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &mut scene,
                    None,
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q1_state_origin(&player.state);
        let horizontal = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(horizontal > 10.0, "player did not advance: {moved:?} from {feet:?}");
        let yaw = f64::from(angles.y).to_radians();
        let along = ((moved.x - feet.x) as f64 * yaw.cos() + (moved.y - feet.y) as f64 * yaw.sin()) / horizontal;
        assert!(
            along > 0.9,
            "player walked off facing: {moved:?} from {feet:?} yaw {}",
            angles.y
        );
        assert!(
            moved.z >= feet.z - 72.0 && moved.z <= feet.z + 8.0,
            "player left the floor: {moved:?} from {feet:?}"
        );
        let body = server.simulation().body_state(&player.actor).expect("player body");
        assert_eq!(body.origin, moved);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q1_player_stops_at_entity_blocker() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let mut scene = build_q1_scene(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let yaw = f64::from(angles.y).to_radians();
        let ahead = vec3(
            feet.x + (64.0 * yaw.cos()) as f32,
            feet.y + (64.0 * yaw.sin()) as f32,
            feet.z,
        );
        let blocker = {
            let simulation = server.simulation_mut();
            simulation
                .spawn(
                    player_provider(),
                    "blocker",
                    Some(BodyState {
                        origin: ahead,
                        angles: vec3(0.0, 0.0, 0.0),
                        velocity: vec3(0.0, 0.0, 0.0),
                        bounds: Bounds {
                            min: vec3(-16.0, -16.0, -32.0),
                            max: vec3(16.0, 16.0, 32.0),
                        },
                        ground: None,
                    }),
                    None,
                    Vec::new(),
                )
                .unwrap()
        };
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Netquake,
                Q1Edition::Classic,
            )
            .unwrap()
        };
        let door_models = Q1EdictTable::new();
        let mut solids = Q1EdictSet::new();
        solids.insert(blocker.id());
        solids.insert(&player.actor);
        let links = Q1SceneLinks {
            door_models: &door_models,
            solids: &solids,
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            let command = WorldUserCommand::Q1Netquake(forward_command(angles, time));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &mut scene,
                    Some(&links),
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q1_state_origin(&player.state);
        let traveled = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(traveled > 1.0, "player never moved: {moved:?}");
        assert!(
            traveled < 64.0 - 16.0 - 16.0 + 2.0,
            "player passed through the blocker: traveled {traveled}"
        );
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q1_player_settles_without_drift() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let mut scene = build_q1_scene(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Netquake,
                Q1Edition::Classic,
            )
            .unwrap()
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..60 {
            time += step_seconds;
            let command = WorldUserCommand::Q1Netquake(still_command(angles, time));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &mut scene,
                    None,
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q1_state_origin(&player.state);
        let horizontal = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(horizontal < 2.0, "idle player drifted: {moved:?} from {feet:?}");
        assert!(moved.z <= feet.z + 1.0, "idle player rose: {moved:?} from {feet:?}");
        assert!(
            moved.z >= feet.z - 72.0,
            "idle player fell through: {moved:?} from {feet:?}"
        );
    }

    #[test]
    fn movement_dialect_resolution_follows_product_and_edition() {
        use qa_content::contract::GameFamily;
        assert_eq!(
            movement_dialect_for_selection(GameFamily::Q1, None, "classic"),
            Dialect::Q1Netquake
        );
        assert_eq!(
            movement_dialect_for_selection(GameFamily::Q1, Some("q1-quakeworld"), "classic"),
            Dialect::Q1Quakeworld
        );
        assert_eq!(
            movement_dialect_for_selection(GameFamily::Q2, None, "classic"),
            Dialect::Q2Classic
        );
        assert_eq!(
            movement_dialect_for_selection(GameFamily::Q2, None, "rerelease"),
            Dialect::Q2Rerelease
        );
        assert_eq!(
            movement_dialect_for_selection(GameFamily::Q3, None, "baseq3"),
            Dialect::Q3
        );
    }

    #[test]
    fn admit_quakeworld_seeds_quakeworld_state() {
        let mut server = q1_server();
        let simulation = server.simulation_mut();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        let player = Q1PlayerBody::admit(
            simulation,
            player_provider(),
            feet,
            angles,
            Dialect::Q1Quakeworld,
            Q1Edition::Classic,
        )
        .unwrap();
        assert!(matches!(player.state, Q1State::Quakeworld(_)));
        assert!(matches!(player.profile, Q1BodyProfile::Quakeworld(_)));
        assert_eq!(player.eye(), vec3(0.0, 0.0, 54.0));
        assert_eq!(player.view_angles, angles);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn qw_player_walks_forward_on_start() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let mut scene = build_q1_scene(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Quakeworld,
                Q1Edition::Classic,
            )
            .unwrap()
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            let command = WorldUserCommand::Q1Quakeworld(qw_forward_command(angles));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &mut scene,
                    None,
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q1_state_origin(&player.state);
        let horizontal = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(
            horizontal > 10.0,
            "QuakeWorld player did not advance: {moved:?} from {feet:?}"
        );
        assert!(
            moved.z >= feet.z - 72.0 && moved.z <= feet.z + 8.0,
            "QuakeWorld player left the floor: {moved:?} from {feet:?}"
        );
        let body = server.simulation().body_state(&player.actor).expect("player body");
        assert_eq!(body.origin, moved);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q2_classic_player_walks_forward_on_base1() {
        let Some(bytes) = base1_bsp_bytes() else {
            return;
        };
        let collision = build_q2_collision(&bytes, "maps/base1.bsp").unwrap();
        let (feet, angles) = q2_spawn_feet_and_angles(&bytes);
        let mut server = q2_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q2PlayerBody::admit(simulation, q2_provider(), feet, angles, Dialect::Q2Classic).unwrap()
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            let command = WorldUserCommand::Q2Classic(q2_classic_forward_command(angles));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &collision,
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q2_state_origin(&player.state);
        let horizontal = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(
            horizontal > 10.0,
            "Q2 classic player did not advance: {moved:?} from {feet:?}"
        );
        assert!(
            moved.z >= feet.z - 72.0 && moved.z <= feet.z + 8.0,
            "Q2 classic player left the floor: {moved:?} from {feet:?}"
        );
        let body = server.simulation().body_state(&player.actor).expect("player body");
        assert_eq!(body.origin, moved);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q2_rerelease_player_walks_forward_on_base1() {
        let Some(bytes) = base1_bsp_bytes() else {
            return;
        };
        let collision = build_q2_collision(&bytes, "maps/base1.bsp").unwrap();
        let (feet, angles) = q2_spawn_feet_and_angles(&bytes);
        let mut server = q2_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q2PlayerBody::admit(simulation, q2_provider(), feet, angles, Dialect::Q2Rerelease).unwrap()
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            let command = WorldUserCommand::Q2Rerelease(q2_rerelease_forward_command(angles));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &collision,
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = q2_state_origin(&player.state);
        let horizontal = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(
            horizontal > 10.0,
            "Q2 rerelease player did not advance: {moved:?} from {feet:?}"
        );
        assert!(
            moved.z >= feet.z - 72.0 && moved.z <= feet.z + 8.0,
            "Q2 rerelease player left the floor: {moved:?} from {feet:?}"
        );
        let body = server.simulation().body_state(&player.actor).expect("player body");
        assert_eq!(body.origin, moved);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q2_provider_mismatches_are_contract_errors() {
        let mut server = q2_server();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        assert!(Q2PlayerBody::admit(server.simulation_mut(), q2_provider(), feet, angles, Dialect::Q3).is_err());
        let Some(bytes) = base1_bsp_bytes() else {
            return;
        };
        let collision = build_q2_collision(&bytes, "maps/base1.bsp").unwrap();
        let mut player = {
            let simulation = server.simulation_mut();
            Q2PlayerBody::admit(simulation, q2_provider(), feet, angles, Dialect::Q2Classic).unwrap()
        };
        let frame = command_frame(0, 1.0 / 60.0, 1.0 / 60.0);
        let (simulation, triggers) = server.simulation_and_triggers();
        assert!(player
            .step(
                simulation,
                triggers,
                &collision,
                WorldUserCommand::Q2Rerelease(q2_rerelease_forward_command(angles)),
                &frame,
            )
            .is_err());
    }

    fn q3dm1_bsp_bytes() -> Option<Vec<u8>> {
        let root = require_live_corpus("Q3 Steel data", &["q3a"])?;
        let catalog = require_live_data(
            "Q3 installed-content catalog",
            qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok(),
        )?;
        let mounts = require_live_data(
            "q3-baseq3 mounts for maps/q3dm1.bsp",
            super::super::windowed_scene::open_product_mounts(&catalog, "q3-baseq3", "maps/q3dm1.bsp").ok(),
        )?;
        require_live_data(
            "maps/q3dm1.bsp bytes",
            mounts
                .read(qa_content::mounts::ResourceRef::Path("maps/q3dm1.bsp"))
                .ok(),
        )
    }

    fn q3_server() -> qa_world::server::Server<qa_guest::server::GuestServerLogic> {
        let options = ApplicationOptions {
            product: "q3-baseq3".to_string(),
            map: "maps/q3dm1.bsp".to_string(),
            ..ApplicationOptions::default()
        };
        let config = StartupConfig::from_options(&options).unwrap();
        open_server(&config).unwrap()
    }

    fn q3_provider() -> ProviderId {
        ProviderId::new("q3", "baseq3")
    }

    fn q3_spawn_feet_and_angles(bytes: &[u8]) -> (Vec3, Vec3) {
        let records = super::super::play_world::decode_map_entities(bytes, "maps/q3dm1.bsp", BspKind::Q3)
            .unwrap()
            .records;
        let spawn = super::super::windowed_scene::select_spawn(&records, BspKind::Q3).expect("q3dm1 spawn");
        let feet = vec3(spawn.origin.x, spawn.origin.y, spawn.origin.z - Q3_VIEW_HEIGHT);
        (feet, spawn.angles)
    }

    fn q3_forward_command(angles: Vec3, server_time_milliseconds: i32) -> Q3UserCommand {
        Q3UserCommand {
            server_time_milliseconds,
            angle_words: angle_shorts(angles),
            buttons: 0,
            weapon: 0,
            forward_move: 127,
            right_move: 0,
            up_move: 0,
        }
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q3_player_walks_forward_on_q3dm1() {
        let Some(bytes) = q3dm1_bsp_bytes() else {
            return;
        };
        let collision = build_q3_collision(&bytes, "maps/q3dm1.bsp").unwrap();
        let (feet, angles) = q3_spawn_feet_and_angles(&bytes);
        let mut server = q3_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q3PlayerBody::admit(simulation, q3_provider(), feet, angles, Dialect::Q3).unwrap()
        };
        let step_seconds = 1.0 / 60.0;
        let mut time = 0.0;
        for frame in 0..120 {
            time += step_seconds;
            // Quake III drops commands at or behind the state's command
            // time, so every step carries a fresh server timestamp.
            let command = WorldUserCommand::Q3(q3_forward_command(angles, (frame + 1) * 16));
            let (simulation, triggers) = server.simulation_and_triggers();
            player
                .step(
                    simulation,
                    triggers,
                    &collision,
                    command,
                    &command_frame(frame, time, step_seconds),
                )
                .unwrap();
        }
        let moved = player.state.origin;
        let horizontal = ((moved.x - feet.x) as f64).hypot((moved.y - feet.y) as f64);
        assert!(horizontal > 10.0, "Q3 player did not advance: {moved:?} from {feet:?}");
        assert!(
            moved.z >= feet.z - 72.0 && moved.z <= feet.z + 8.0,
            "Q3 player left the floor: {moved:?} from {feet:?}"
        );
        let body = server.simulation().body_state(&player.actor).expect("player body");
        assert_eq!(body.origin, moved);
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn q3_provider_mismatches_are_contract_errors() {
        let mut server = q3_server();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        assert!(Q3PlayerBody::admit(server.simulation_mut(), q3_provider(), feet, angles, Dialect::Q2Classic).is_err());
        let Some(bytes) = q3dm1_bsp_bytes() else {
            return;
        };
        let collision = build_q3_collision(&bytes, "maps/q3dm1.bsp").unwrap();
        let mut player = {
            let simulation = server.simulation_mut();
            Q3PlayerBody::admit(simulation, q3_provider(), feet, angles, Dialect::Q3).unwrap()
        };
        let frame = command_frame(0, 1.0 / 60.0, 1.0 / 60.0);
        let (simulation, triggers) = server.simulation_and_triggers();
        assert!(player
            .step(
                simulation,
                triggers,
                &collision,
                WorldUserCommand::Q2Classic(q2_classic_forward_command(angles)),
                &frame,
            )
            .is_err());
    }

    #[test]
    #[ignore = "live proof: needs Steel corpus"]
    fn provider_mismatches_are_contract_errors() {
        let mut server = q1_server();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Netquake,
                Q1Edition::Classic,
            )
            .unwrap()
        };
        assert!(Q1PlayerBody::admit(
            server.simulation_mut(),
            player_provider(),
            feet,
            angles,
            Dialect::Q3,
            Q1Edition::Classic
        )
        .is_err());
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let mut scene = build_q1_scene(&bytes, "maps/start.bsp").unwrap();
        let frame = command_frame(0, 1.0 / 60.0, 1.0 / 60.0);
        let (simulation, triggers) = server.simulation_and_triggers();
        assert!(player
            .step(
                simulation,
                triggers,
                &mut scene,
                None,
                WorldUserCommand::Q1Quakeworld(qw_forward_command(angles)),
                &frame,
            )
            .is_err());
    }

    fn edition_catalog() -> qa_content::catalog::InstalledCatalog {
        use qa_content::catalog::{CatalogProduct, InstalledCatalog, ProductAvailability, ProductExpectation};
        use qa_content::contract::ContentId;

        let product = |id: &str, family: GameFamily, edition: &str| CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: id.to_string(),
                family,
                edition: edition.to_string(),
                campaign: "id1".to_string(),
                title: id.to_string(),
                content_directory: "id1".to_string(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: ProductAvailability::Installed,
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        };
        InstalledCatalog::new(
            "edition-test".to_string(),
            vec![
                product("q1-classic-id1", GameFamily::Q1, "classic"),
                product("q1-rerelease-id1", GameFamily::Q1, "rerelease"),
            ],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
    }

    #[test]
    fn movement_edition_follows_the_movement_product() {
        let catalog = edition_catalog();
        assert_eq!(movement_content_edition(&catalog, None).unwrap(), "classic");
        assert_eq!(
            movement_content_edition(&catalog, Some("q1-rerelease-id1")).unwrap(),
            "rerelease"
        );
        assert!(movement_content_edition(&catalog, Some("q9-elsewhere")).is_err());
    }

    #[test]
    fn q1_rerelease_movement_seeds_the_rerelease_profile() {
        assert_eq!(q1_edition_for_movement("rerelease"), Q1Edition::Rerelease);
        assert_eq!(q1_edition_for_movement("classic"), Q1Edition::Classic);
        let profile = q1_profile(player_provider(), Q1Edition::Rerelease);
        assert_eq!(profile.edition, Q1Edition::Rerelease);
    }

    #[test]
    fn admit_carries_movement_edition_into_the_profile() {
        let mut server = q1_server();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        let player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(
                simulation,
                player_provider(),
                feet,
                angles,
                Dialect::Q1Netquake,
                Q1Edition::Rerelease,
            )
            .unwrap()
        };
        let Q1BodyProfile::Netquake(profile) = &player.profile else {
            panic!("admitted the wrong Quake I profile");
        };
        assert_eq!(profile.edition, Q1Edition::Rerelease);
    }
}
