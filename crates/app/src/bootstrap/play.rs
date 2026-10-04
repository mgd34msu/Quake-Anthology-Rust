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
//! eye flow. [`Q1ClipWorld`] builds the Quake I collision hulls from the
//! parsed BSP, [`Q1PlayerServices`] implements the movement services over
//! those hulls plus the live server bodies, and the Q1 arm runs one
//! [`move_netquake`](qa_world::movement::q1::netquake::move_netquake)
//! step per frame and commits the result back to the sim body. Quake II
//! and III arrive as new arms on these same enums, extending the existing
//! trace and movement cores rather than forking them.

use qa_content::bsp::{read_q1_bsp, ClipChild as BspClipChild, NodeChild, Plane as BspPlaneData, Q1BspOptions};
use qa_content::contract::GameFamily;
use qa_core::cmd::Dialect;
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::numeric::{NumericOps, Q1_DONOR_PROFILE};
use qa_core::time::FrameContext as ClockFrame;
use qa_world::body::BodyState;
use qa_world::collision::q1::{CONTENTS_EMPTY, CONTENTS_SOLID};
use qa_world::hull::{
    axis_box_hull, hull_point_contents, trace_hull, trace_hull_solid, BspPlane, ClipChild as HullClipChild,
    ClipNode as HullClipNode, Hull, HullTrace,
};
use qa_world::movement::q1::netquake::move_netquake;
use qa_world::movement::q1::quakeworld::move_quake_world;
use qa_world::movement::q1::types::{
    NoQ1Hooks, Q1AnimationStepInput, Q1AnimationStepResult, Q1Edition, Q1MovementInput, Q1MovementOptions,
    Q1MovementProfile, Q1MovementServices, Q1MovementState, Q1State, Q1Trace, Q1TraceQuery, Q1WeaponStepInput,
    Q1WeaponStepResult, QwMovementInput, QwMovementProfile, QwMovementState, Q1_MOVE_WALK,
};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, MovementContinuation, MovementEnvironment, MovementExecution,
    MovementInputFields, MovementOutcome, MovementTouchContact, Q1UserCommand, QwUserCommand, TraceContact, TraceHit,
    TraceShape, UserCommand as WorldUserCommand, WeaponState,
};
use qa_world::movement::Q1MovementParameters;
use qa_world::session::Simulation;
use qa_world::triggers::TriggerTable;

/// Quake I player collision box, matching qsrc hull 1 (`gl_model.c`
/// `Mod_LoadClipnodes`): x/y half-width 16, feet at -24, head at +32.
#[must_use]
pub fn q1_player_bounds() -> Bounds {
    Bounds {
        min: vec3(-16.0, -16.0, -24.0),
        max: vec3(16.0, 16.0, 32.0),
    }
}

/// Quake I player hull expansion: hull 1 clip mins/maxs from qsrc
/// `gl_model.c`. The player box matches exactly, so the trace offset is
/// zero against the world; the offset math stays general for other boxes.
const Q1_HULL1_CLIP_MINS: [f32; 3] = [-16.0, -16.0, -24.0];

/// Quake I eye height above the feet origin (donor `eye_height`, qsrc
/// `VIEW_OFS` 22).
pub const Q1_VIEW_HEIGHT: f32 = 22.0;

/// Quake I collision hulls for one map: hull 0 (point traces and contents
/// over the BSP nodes, qsrc `Mod_MakeHull0`) plus hull 1 (player-box
/// traces over the shared clip tree, qsrc `Mod_LoadClipnodes`).
#[derive(Debug, Clone)]
pub struct Q1ClipWorld {
    hull0: Hull,
    hull1: Hull,
}

/// Build the collision hulls from raw BSP bytes.
///
/// Parses the map with the format reader, converts world-model headnodes
/// into hull 0 (nodes copied to clip form with leaf contents) and hull 1
/// (the shared clip tree), and fails honestly when the map has no world
/// model, no usable headnodes, or dangling plane/node indices.
pub fn build_q1_clip_world(bytes: &[u8], map: &str) -> Result<Q1ClipWorld, String> {
    let parsed = read_q1_bsp(bytes, map, Q1BspOptions::default()).map_err(|error| error.to_string())?;
    let world = parsed
        .models
        .first()
        .ok_or_else(|| format!("{map}: BSP has no world model"))?;
    let planes = convert_planes(&parsed.planes, map)?;
    let hull0 = Hull {
        planes: planes.clone(),
        clipnodes: convert_nodes(&parsed, map)?,
        first: world.headnodes[0],
        last: parsed.nodes.len() as i32 - 1,
    };
    let hull1 = Hull {
        planes,
        clipnodes: convert_clipnodes(&parsed, map)?,
        first: world.headnodes[1],
        last: parsed.clipnodes.len() as i32 - 1,
    };
    if hull0.first < 0 {
        return Err(format!("{map}: world model has no hull 0 headnode"));
    }
    if hull1.first < 0 {
        return Err(format!("{map}: world model has no hull 1 headnode"));
    }
    Ok(Q1ClipWorld { hull0, hull1 })
}

/// Convert BSP planes to hull planes.
fn convert_planes(planes: &[BspPlaneData], map: &str) -> Result<Vec<BspPlane>, String> {
    planes
        .iter()
        .map(|plane| {
            u8::try_from(plane.plane_type).map_or_else(
                |_| Err(format!("{map}: plane type {} out of range", plane.plane_type)),
                |plane_type| {
                    Ok(BspPlane {
                        normal: vec3(plane.normal[0], plane.normal[1], plane.normal[2]),
                        distance: plane.distance,
                        plane_type,
                        signbits: plane.signbits,
                    })
                },
            )
        })
        .collect()
}

/// Convert BSP nodes to clip form (qsrc `Mod_MakeHull0`): node children
/// stay node indices, leaf children become leaf-contents terminals.
fn convert_nodes(parsed: &qa_content::bsp::Q1Map<'_>, map: &str) -> Result<Vec<HullClipNode>, String> {
    parsed
        .nodes
        .iter()
        .map(|node| {
            let plane =
                usize::try_from(node.plane).map_err(|_| format!("{map}: node plane {} out of range", node.plane))?;
            if plane >= parsed.planes.len() {
                return Err(format!(
                    "{map}: node plane {plane} beyond {} planes",
                    parsed.planes.len()
                ));
            }
            let mut children = [HullClipChild::Contents(CONTENTS_SOLID); 2];
            for (index, child) in node.children.iter().enumerate() {
                children[index] = match child {
                    NodeChild::Node(node_index) => {
                        let node_ref = usize::try_from(*node_index)
                            .map_err(|_| format!("{map}: node child {node_index} out of range"))?;
                        if node_ref >= parsed.nodes.len() {
                            return Err(format!(
                                "{map}: node child {node_ref} beyond {} nodes",
                                parsed.nodes.len()
                            ));
                        }
                        HullClipChild::Node(node_ref)
                    }
                    NodeChild::Leaf(leaf_index) => {
                        let leaf_ref = usize::try_from(*leaf_index)
                            .map_err(|_| format!("{map}: leaf child {leaf_index} out of range"))?;
                        let leaf = parsed.leaves.get(leaf_ref).ok_or_else(|| {
                            format!("{map}: leaf child {leaf_ref} beyond {} leaves", parsed.leaves.len())
                        })?;
                        HullClipChild::Contents(leaf.contents)
                    }
                };
            }
            Ok(HullClipNode { plane, children })
        })
        .collect()
}

/// Convert the shared clip tree to hull form.
fn convert_clipnodes(parsed: &qa_content::bsp::Q1Map<'_>, map: &str) -> Result<Vec<HullClipNode>, String> {
    parsed
        .clipnodes
        .iter()
        .map(|node| {
            let plane = usize::try_from(node.plane)
                .map_err(|_| format!("{map}: clipnode plane {} out of range", node.plane))?;
            if plane >= parsed.planes.len() {
                return Err(format!(
                    "{map}: clipnode plane {plane} beyond {} planes",
                    parsed.planes.len()
                ));
            }
            let mut children = [HullClipChild::Contents(CONTENTS_SOLID); 2];
            for (index, child) in node.children.iter().enumerate() {
                children[index] = match child {
                    BspClipChild::Contents(contents) => HullClipChild::Contents(*contents),
                    BspClipChild::ClipNode(clip_index) => {
                        let clip_ref = usize::try_from(*clip_index)
                            .map_err(|_| format!("{map}: clipnode child {clip_index} out of range"))?;
                        if clip_ref >= parsed.clipnodes.len() {
                            return Err(format!(
                                "{map}: clipnode child {clip_ref} beyond {} clipnodes",
                                parsed.clipnodes.len()
                            ));
                        }
                        HullClipChild::Node(clip_ref)
                    }
                };
            }
            Ok(HullClipNode { plane, children })
        })
        .collect()
}

impl Q1ClipWorld {
    /// Trace a point through hull 0, blocking on solid.
    pub fn trace_point(&self, start: Vec3, end: Vec3, ops: &NumericOps) -> HullTrace {
        solid_on_corrupt(trace_hull_solid(&self.hull0, start, end, ops), end)
    }

    /// Trace a box through hull 1 with the qsrc hull offset
    /// (`clip_mins - trace_mins + origin`, `SV_ClipMoveToEntity`): the
    /// pre-expanded clip tree sees the trace shifted so contact lands
    /// where the box face touches. Blocks on solid.
    pub fn trace_box(&self, start: Vec3, end: Vec3, bounds: &Bounds, ops: &NumericOps) -> HullTrace {
        let offset = vec3(
            Q1_HULL1_CLIP_MINS[0] - bounds.min.x,
            Q1_HULL1_CLIP_MINS[1] - bounds.min.y,
            Q1_HULL1_CLIP_MINS[2] - bounds.min.z,
        );
        let shifted = |point: Vec3| vec3(point.x + offset.x, point.y + offset.y, point.z + offset.z);
        let trace = trace_hull(&self.hull1, shifted(start), shifted(end), ops, &|contents| {
            contents == CONTENTS_SOLID
        });
        let mut trace = solid_on_corrupt(trace, end);
        trace.end = vec3(trace.end.x - offset.x, trace.end.y - offset.y, trace.end.z - offset.z);
        trace.plane.distance -=
            trace.plane.normal.x * offset.x + trace.plane.normal.y * offset.y + trace.plane.normal.z * offset.z;
        trace
    }

    /// Contents at a point through hull 0 (qsrc `SV_HullPointContents`).
    /// Corrupt trees report solid: blocking is safer than swimming blind.
    pub fn point_contents(&self, point: Vec3, ops: &NumericOps) -> i32 {
        hull_point_contents(&self.hull0, point, ops).unwrap_or(CONTENTS_SOLID)
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
pub fn q1_profile(provider: ProviderId) -> Q1MovementProfile {
    Q1MovementProfile {
        id: provider,
        clock: qa_core::time::ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        },
        numeric: Q1_DONOR_PROFILE,
        edition: Q1Edition::Classic,
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

/// Quake I movement services over a clip world plus the live server
/// bodies: world traces run the map hulls, entity traces sweep every
/// non-trigger body but the mover, and trigger overlap stays with the
/// server's trigger sweep each tick.
pub struct Q1PlayerServices<'s> {
    ops: NumericOps,
    clip: &'s Q1ClipWorld,
    simulation: &'s Simulation,
    triggers: &'s TriggerTable,
    ignore: ActorId,
}

impl<'s> Q1PlayerServices<'s> {
    /// Borrow the clip world, the server simulation and trigger table,
    /// ignoring the moving actor's own body in entity traces.
    #[must_use]
    pub fn new(
        clip: &'s Q1ClipWorld,
        simulation: &'s Simulation,
        triggers: &'s TriggerTable,
        ignore: &ActorId,
    ) -> Self {
        Self {
            ops: NumericOps::select(Q1_DONOR_PROFILE).expect("Q1 donor numeric profile"),
            clip,
            simulation,
            triggers,
            ignore: ignore.clone(),
        }
    }

    /// Trace the query against the world hulls and server bodies,
    /// returning the nearest hit.
    fn trace_combined(&self, query: &Q1TraceQuery) -> Q1Trace {
        let trace_bounds = match &query.shape {
            TraceShape::Point => Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
            TraceShape::Box(bounds) | TraceShape::Capsule(bounds) => *bounds,
        };
        let point = matches!(query.shape, TraceShape::Point);
        let world = if point {
            self.clip.trace_point(query.start, query.end, &self.ops)
        } else {
            self.clip.trace_box(query.start, query.end, &trace_bounds, &self.ops)
        };
        let mut best_fraction = world.fraction;
        let mut best_hit = if world.fraction < 1.0 {
            TraceHit::World { model: 0 }
        } else {
            TraceHit::None
        };
        let mut best_plane = world.plane;
        let mut best = world;
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
            if entity.fraction < best_fraction {
                best_fraction = entity.fraction;
                best_hit = TraceHit::Actor { actor };
                best_plane = entity.plane;
                best = entity;
            }
        }
        q1_trace_from_hull(&best, best_fraction, best_hit, &best_plane)
    }
}

/// Convert a hull trace to a movement trace result.
fn q1_trace_from_hull(trace: &HullTrace, fraction: f64, hit: TraceHit, plane: &qa_core::math::Plane) -> Q1Trace {
    Q1Trace {
        fraction,
        end: trace.end,
        start_solid: trace.start_solid,
        all_solid: trace.all_solid,
        contact: if fraction < 1.0 {
            TraceContact::Plane(*plane)
        } else {
            TraceContact::None
        },
        hit,
        in_open: trace.in_open,
        in_water: trace.in_water,
        source_plane: *plane,
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
        self.clip.point_contents(point, &self.ops)
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
                Q1BodyProfile::Netquake(q1_profile(provider.clone())),
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
    /// commit the resulting origin back to the sim body. The command
    /// must match the admitted provider; mismatches are contract
    /// errors, never silent drops.
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        clip: &Q1ClipWorld,
        command: WorldUserCommand,
        frame: &ClockFrame,
    ) -> Result<(), String> {
        self.sequence += 1;
        match command {
            WorldUserCommand::Q1Netquake(command) => self.step_netquake(simulation, triggers, clip, command, frame),
            WorldUserCommand::Q1Quakeworld(command) => self.step_quakeworld(simulation, triggers, clip, command, frame),
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
        triggers: &TriggerTable,
        clip: &Q1ClipWorld,
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
            let mut services = Q1PlayerServices::new(clip, simulation, triggers, &self.actor);
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
        triggers: &TriggerTable,
        clip: &Q1ClipWorld,
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
            let mut services = Q1PlayerServices::new(clip, simulation, triggers, &self.actor);
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
/// Quake I is live; Quake II and III arrive as new arms on this same
/// enum, extending the existing trace and movement cores.
pub enum PlayerBody {
    /// Quake I player.
    Q1(Q1PlayerBody),
}

/// Map collision for any family, matching [`PlayerBody`].
pub enum PlayerClip {
    /// Quake I clip hulls.
    Q1(Q1ClipWorld),
}

/// Admit a player for a catalog family and resolved movement provider,
/// or `None` when the family has no body wired yet (the camera falls
/// back to the static spawn, exactly the pre-play behavior, until its
/// arm lands). A provider outside the family's own is a contract error.
pub fn admit_player(
    simulation: &mut Simulation,
    family: GameFamily,
    provider: ProviderId,
    feet: Vec3,
    angles: Vec3,
    dialect: Dialect,
) -> Result<Option<PlayerBody>, String> {
    match family {
        GameFamily::Q1 => Ok(Some(PlayerBody::Q1(Q1PlayerBody::admit(
            simulation, provider, feet, angles, dialect,
        )?))),
        GameFamily::Q2 | GameFamily::Q3 => Ok(None),
    }
}

/// Build map collision for a catalog family, or `None` when the family
/// has no collision wired yet (matching [`admit_player`]).
pub fn build_clip(bytes: &[u8], map: &str, family: GameFamily) -> Result<Option<PlayerClip>, String> {
    match family {
        GameFamily::Q1 => Ok(Some(PlayerClip::Q1(build_q1_clip_world(bytes, map)?))),
        GameFamily::Q2 | GameFamily::Q3 => Ok(None),
    }
}

impl PlayerBody {
    /// Eye origin plus view angles for the follow camera.
    #[must_use]
    pub fn eye(&self) -> (Vec3, Vec3) {
        match self {
            PlayerBody::Q1(player) => (player.eye(), player.view_angles),
        }
    }

    /// The body's simulation actor: the same identity the driving seat's
    /// [`LocalPlayer`](super::input::LocalPlayer) controls. The body is
    /// the embodiment half of that one concept, never a parallel player.
    #[must_use]
    pub fn actor(&self) -> &ActorId {
        match self {
            PlayerBody::Q1(player) => &player.actor,
        }
    }

    /// Run one authoritative movement step for a world user command. The
    /// command dialect must match the player family; mismatches are
    /// contract errors, never silent drops.
    pub fn step(
        &mut self,
        simulation: &mut Simulation,
        triggers: &TriggerTable,
        clip: &PlayerClip,
        command: WorldUserCommand,
    ) -> Result<(), String> {
        let frame = simulation.frame();
        match (self, clip) {
            (PlayerBody::Q1(player), PlayerClip::Q1(clip)) => player.step(simulation, triggers, clip, command, &frame),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qa_content::catalog::DiscoverContentOptions;
    use qa_content::BspKind;
    use qa_core::time::{FramePhase, SourceTime};

    use super::*;
    use crate::options::ApplicationOptions;
    use crate::startup::{open_server, StartupConfig};

    fn steel_corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target")
    }

    fn start_bsp_bytes() -> Option<Vec<u8>> {
        let root = steel_corpus_root();
        if !root.join("q1").is_dir() {
            eprintln!("skipped: Steel corpus root {} has no Q1 data", root.display());
            return None;
        }
        let catalog = qa_content::catalog::discover_installed_content(&DiscoverContentOptions::new(root)).ok()?;
        let mounts =
            super::super::windowed_scene::open_product_mounts(&catalog, "q1-classic-id1", "maps/start.bsp").ok()?;
        mounts
            .read(qa_content::mounts::ResourceRef::Path("maps/start.bsp"))
            .ok()
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
        let player = Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Netquake).unwrap();
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
    fn start_spawn_is_empty_and_floor_is_solid() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let clip = build_q1_clip_world(&bytes, "maps/start.bsp").unwrap();
        let ops = NumericOps::select(Q1_DONOR_PROFILE).unwrap();
        let (feet, _) = spawn_feet_and_angles(&bytes);
        let eye = vec3(feet.x, feet.y, feet.z + Q1_VIEW_HEIGHT);
        assert_eq!(clip.point_contents(eye, &ops), CONTENTS_EMPTY);
        assert_eq!(clip.point_contents(feet, &ops), CONTENTS_EMPTY);
        let down = clip.trace_box(feet, vec3(feet.x, feet.y, feet.z - 256.0), &q1_player_bounds(), &ops);
        assert!(!down.start_solid, "spawn feet start inside solid");
        assert!(down.fraction < 1.0, "no floor within 256 units of spawn");
        assert!(down.plane.normal.z > 0.7, "floor plane {:?}", down.plane.normal);
        assert!(down.end.z < feet.z, "floor end {:?}", down.end);
    }

    #[test]
    fn q1_player_walks_forward_on_start() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let clip = build_q1_clip_world(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Netquake).unwrap()
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
                    &clip,
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
    fn q1_player_stops_at_entity_blocker() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let clip = build_q1_clip_world(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let yaw = f64::from(angles.y).to_radians();
        let ahead = vec3(
            feet.x + (64.0 * yaw.cos()) as f32,
            feet.y + (64.0 * yaw.sin()) as f32,
            feet.z,
        );
        {
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
                .unwrap();
        }
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Netquake).unwrap()
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
                    &clip,
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
    fn q1_player_settles_without_drift() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let clip = build_q1_clip_world(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Netquake).unwrap()
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
                    &clip,
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
        let player = Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Quakeworld).unwrap();
        assert!(matches!(player.state, Q1State::Quakeworld(_)));
        assert!(matches!(player.profile, Q1BodyProfile::Quakeworld(_)));
        assert_eq!(player.eye(), vec3(0.0, 0.0, 54.0));
        assert_eq!(player.view_angles, angles);
    }

    #[test]
    fn qw_player_walks_forward_on_start() {
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let clip = build_q1_clip_world(&bytes, "maps/start.bsp").unwrap();
        let (feet, angles) = spawn_feet_and_angles(&bytes);
        let mut server = q1_server();
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Quakeworld).unwrap()
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
                    &clip,
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
    fn provider_mismatches_are_contract_errors() {
        let mut server = q1_server();
        let feet = vec3(0.0, 0.0, 32.0);
        let angles = vec3(0.0, 180.0, 0.0);
        let mut player = {
            let simulation = server.simulation_mut();
            Q1PlayerBody::admit(simulation, player_provider(), feet, angles, Dialect::Q1Netquake).unwrap()
        };
        assert!(Q1PlayerBody::admit(server.simulation_mut(), player_provider(), feet, angles, Dialect::Q3).is_err());
        let Some(bytes) = start_bsp_bytes() else {
            return;
        };
        let clip = build_q1_clip_world(&bytes, "maps/start.bsp").unwrap();
        let frame = command_frame(0, 1.0 / 60.0, 1.0 / 60.0);
        let (simulation, triggers) = server.simulation_and_triggers();
        assert!(player
            .step(
                simulation,
                triggers,
                &clip,
                WorldUserCommand::Q1Quakeworld(qw_forward_command(angles)),
                &frame,
            )
            .is_err());
    }
}
