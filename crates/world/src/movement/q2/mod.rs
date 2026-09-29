//! Quake II movement providers.
//!
//! Donor provenance: `src/movement/q2/index.ts` (Quake II ClientThink/Pmove
//! and rerelease game/cgame integration).

use std::cell::RefCell;

use qa_core::identity::{same_actor, ProviderId};
use qa_core::math::{vec3, Bounds, Plane, Vec3};
use qa_core::time::SourceTime;

pub mod classic;
pub mod dimensions;
pub mod math;
pub mod rerelease;
pub mod swept;
pub mod types;
pub mod view;

pub use classic::pmove_classic;
pub use dimensions::Q2_PLAYER_BOUNDS;
pub use rerelease::{create_rerelease_movement, pmove_rerelease, Q2RereleaseMovementContext};
pub use types::{button, kex_pm_type, pm_flags, pm_type, water_level, ClassicPmove, KexPmove, PmConfig, TraceT};

use super::client_outputs::{client_movement_mode, client_movement_type, client_stance_command};
use super::types::{
    MovementContinuation, MovementDialect, MovementEffect, MovementError, MovementExecution, MovementInputContinuation,
    MovementOutcome, MovementResultFields, OrderedMovementEffect, TouchSurface, TraceContact, TraceHit, TraceShape,
    UserCommand,
};
use dimensions::command_duration;
use types::{pm_flags as flags, pm_type as classic_type, Q2MovementServices};
use types::{
    CPlane, CSurface, ClassicPmoveCmd, ClassicPmoveState, KexPmoveCmd, KexPmoveState, KexTouchList, MovementEntity,
    Q2ContentsQuery, Q2MovementContact, Q2MovementInput, Q2MovementResult, Q2MovementState, Q2RereleaseMovementInput,
    Q2RereleaseMovementResult, Q2RereleaseMovementState, Q2RereleasePresentation, Q2SourceTrace, Q2State, Q2Surface,
    Q2TouchContact, Q2Trace, Q2TracePlane, Q2TraceQuery, SrcVec3, MASK_CLASSIC_PLAYERSOLID, PM_CONFIG_DEFAULT,
};

const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

fn vector(source: SrcVec3) -> Vec3 {
    vec3(source[0] as f32, source[1] as f32, source[2] as f32)
}

fn source_vector(value: Vec3) -> SrcVec3 {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

fn body_bounds(shape: &TraceShape) -> Bounds {
    match *shape {
        TraceShape::Point => Bounds { min: ZERO, max: ZERO },
        TraceShape::Box(bounds) | TraceShape::Capsule(bounds) => bounds,
    }
}

fn source_plane(value: &Q2TracePlane) -> CPlane {
    CPlane {
        normal: source_vector(value.normal),
        dist: value.dist,
        plane_type: value.plane_type,
        signbits: value.signbits,
    }
}

fn scene_plane(value: &CPlane) -> Q2TracePlane {
    Q2TracePlane {
        normal: vector(value.normal),
        dist: value.dist,
        plane_type: value.plane_type,
        signbits: value.signbits,
    }
}

struct TraceAdapter<'s, S: Q2MovementServices> {
    services: &'s mut S,
    entities: Vec<MovementEntity>,
    leaf: crate::collision::LeafContents,
    point_shape: bool,
}

impl<S: Q2MovementServices> TraceAdapter<'_, S> {
    fn canonical(&mut self, hit: &TraceHit) -> Option<MovementEntity> {
        if matches!(hit, TraceHit::None) {
            return None;
        }
        if let Some(prior) = self.entities.iter().find(|candidate| match (candidate, hit) {
            (TraceHit::World { model: a }, TraceHit::World { model: b }) => a == b,
            (TraceHit::Actor { actor: a }, TraceHit::Actor { actor: b }) => same_actor(a, b),
            _ => false,
        }) {
            return Some(prior.clone());
        }
        self.entities.push(hit.clone());
        Some(hit.clone())
    }

    fn trace(
        &mut self,
        start: SrcVec3,
        mins: SrcVec3,
        maxs: SrcVec3,
        end: SrcVec3,
        mask: i32,
        world_only: bool,
    ) -> TraceT {
        let result = self.services.trace(Q2TraceQuery {
            start: vector(start),
            end: vector(end),
            mins,
            maxs,
            point: self.point_shape,
            mask,
            world_only,
            leaf: self.leaf,
        });
        TraceT {
            allsolid: result.all_solid,
            startsolid: result.start_solid,
            fraction: result.fraction,
            endpos: source_vector(result.end),
            plane: source_plane(&result.source_plane),
            plane2: result
                .secondary
                .as_ref()
                .map(|(plane, _)| source_plane(plane))
                .unwrap_or_else(types::plane),
            surface: result.surface.as_ref().map(CSurface::from),
            surface2: result
                .secondary
                .as_ref()
                .and_then(|(_, surface)| surface.as_ref().map(CSurface::from)),
            contents: result.contents,
            ent: self.canonical(&result.hit),
            native: Some(result),
        }
    }

    fn pointcontents(&mut self, point: SrcVec3) -> i32 {
        let (stored, merged) = self.services.point_contents(Q2ContentsQuery {
            point: vector(point),
            leaf: self.leaf,
        });
        if self.leaf == crate::collision::LeafContents::Stored {
            stored
        } else {
            merged
        }
    }
}

fn contact_trace(trace: &TraceT) -> Q2Trace {
    match &trace.native {
        Some(native) => {
            let mut rebuilt = native.clone();
            rebuilt.source_plane = scene_plane(&trace.plane);
            rebuilt.surface = trace.surface.as_ref().map(Q2Surface::from);
            if !matches!(rebuilt.contact, TraceContact::None) {
                let plane = scene_plane(&trace.plane);
                rebuilt.contact = TraceContact::Plane(Plane {
                    normal: plane.normal,
                    distance: plane.dist as f32,
                });
            }
            rebuilt
        }
        None => {
            let plane = scene_plane(&trace.plane);
            Q2Trace {
                fraction: trace.fraction,
                end: vector(trace.endpos),
                start_solid: trace.startsolid,
                all_solid: trace.allsolid,
                contact: TraceContact::Plane(Plane {
                    normal: plane.normal,
                    distance: plane.dist as f32,
                }),
                hit: trace.ent.clone().unwrap_or(TraceHit::None),
                contents: trace.contents,
                surface: trace.surface.as_ref().map(Q2Surface::from),
                source_plane: plane,
                secondary: trace
                    .surface2
                    .as_ref()
                    .map(|surface| (scene_plane(&trace.plane2), Some(Q2Surface::from(surface)))),
            }
        }
    }
}

fn movement_contacts(traces: &[TraceT]) -> Vec<Q2MovementContact> {
    let mut contacts = Vec::new();
    let mut touched: Vec<MovementEntity> = Vec::new();
    for trace in traces {
        let Some(ent) = trace.ent.clone() else {
            continue;
        };
        if touched.contains(&ent) {
            continue;
        }
        touched.push(ent.clone());
        contacts.push(Q2MovementContact {
            target: ent,
            trace: contact_trace(trace),
            substep: 0,
        });
    }
    contacts
}

#[allow(clippy::too_many_arguments)]
fn touch_contacts<S: Q2MovementServices>(
    rerelease: bool,
    actor: &qa_core::identity::OwnedActor,
    time: SourceTime,
    services: &mut S,
    initial: Q2State,
    contacts: &[Q2MovementContact],
    effects: &mut Vec<OrderedMovementEffect>,
) -> Result<MovementContinuation<Q2State>, MovementError> {
    let mut state = initial;
    for contact in contacts {
        if matches!(contact.target, TraceHit::None) {
            continue;
        }
        let sequence = effects.len();
        effects.push(OrderedMovementEffect {
            substep: 0,
            sequence,
            time,
            effect: MovementEffect::Touch {
                target: contact.target.clone(),
                substep: 0,
            },
        });
        let source = &contact.trace;
        let plane = Plane {
            normal: source.source_plane.normal,
            distance: source.source_plane.dist as f32,
        };
        let surface = source.surface.as_ref().map(|surface| TouchSurface {
            name: surface.name.clone(),
            native_flags: surface.flags,
            native_value: surface.value,
        });
        let source_trace = if rerelease {
            Some(Q2SourceTrace {
                trace: source.clone(),
                inverted: true,
            })
        } else {
            None
        };
        let continuation = services.touch(
            Q2TouchContact::from_trace(
                actor.clone(),
                contact.target.clone(),
                Some(plane),
                surface,
                source_trace,
            ),
            state,
        );
        match continuation {
            MovementContinuation::ActorRemoved => {
                return Ok(MovementContinuation::ActorRemoved);
            }
            MovementContinuation::Continue(next) => {
                let same = matches!(
                    (&state, &next),
                    (Q2State::Classic(_), Q2State::Classic(_)) | (Q2State::Rerelease(_), Q2State::Rerelease(_))
                );
                if !same {
                    return Err(MovementError::Contract(
                        "Touch changed Quake II movement family during one source command",
                    ));
                }
                state = next;
            }
        }
    }
    Ok(MovementContinuation::Continue(state))
}

fn move_q2_classic_physics<S: Q2MovementServices>(
    input: Q2MovementInput,
    services: &mut S,
) -> Result<Q2MovementResult, MovementError> {
    let output = input.fields.environment.client_outputs;
    let mode = client_movement_mode(output.as_ref(), input.fields.environment.health);
    let command = client_stance_command(UserCommand::Q2Classic(input.command), output.and_then(|o| o.stance))
        .map_err(|error| MovementError::Contract(error.0))?;
    let UserCommand::Q2Classic(command) = command else {
        return Err(MovementError::Contract("Client output changed movement dialect"));
    };
    let input = Q2MovementInput { command, ..input };
    command_duration(input.command.milliseconds)?;
    let n = services.numeric();
    let pose = input.fields.environment.pose;
    let body = pose
        .map(|pose| pose.bounds)
        .unwrap_or_else(|| body_bounds(&input.fields.shape));
    let adapter = RefCell::new(TraceAdapter {
        services,
        entities: Vec::new(),
        leaf: crate::collision::LeafContents::Stored,
        point_shape: matches!(input.fields.shape, TraceShape::Point),
    });
    let mut pm = ClassicPmove {
        s: ClassicPmoveState {
            pm_type: match (pose, mode) {
                (Some(_), _) => classic_type::FREEZE,
                (None, None) => input.state.move_type,
                (None, Some(mode)) => client_movement_type(MovementDialect::Q2Classic, mode),
            },
            origin: input.state.origin_eighths,
            velocity: if pose.is_none() {
                input.state.velocity_eighths
            } else {
                [0, 0, 0]
            },
            pm_flags: input.state.flags,
            pm_time: input.state.time_eight_milliseconds,
            gravity: input.state.gravity,
            delta_angles: input.state.delta_angle_shorts,
        },
        cmd: ClassicPmoveCmd {
            msec: input.command.milliseconds,
            angles: input.command.angle_shorts,
            forwardmove: input.command.forward_move,
            sidemove: input.command.side_move,
            upmove: input.command.up_move,
            buttons: input.command.buttons,
            impulse: input.command.impulse,
            lightlevel: input.command.light_level,
        },
        snapinitial: input.profile.snap_initial,
        numtouch: 0,
        touchents: Vec::new(),
        touchtraces: Vec::new(),
        viewangles: [0.0, 0.0, 0.0],
        viewheight: 0.0,
        mins: source_vector(body.min),
        maxs: source_vector(body.max),
        groundentity: None,
        watertype: 0,
        waterlevel: 0,
        character_bounds: body,
        previous_bounds: input.fields.current_bounds.or(Some(body)),
        body_bounds: match (pose, output.and_then(|o| o.body_bounds)) {
            (Some(_), _) | (None, None) => None,
            (None, Some(bounds)) => Some(bounds),
        },
        trace: Box::new(|start, mins, maxs, end| {
            adapter
                .borrow_mut()
                .trace(start, mins, maxs, end, MASK_CLASSIC_PLAYERSOLID, false)
        }),
        pointcontents: Box::new(|point| adapter.borrow_mut().pointcontents(point)),
    };
    let flight = input.fields.environment.flight && input.fields.environment.health > 0.0;
    let speed = input.fields.environment.speed_multiplier.unwrap_or(1.0);
    classic::pmove_classic(
        &mut pm,
        n,
        input.profile.air_accelerate,
        input.profile.strafejump_hack,
        flight,
        speed,
    );
    if let Some(pose) = pose {
        pm.s.pm_type = input.state.move_type;
        pm.viewheight = pose.view_height;
        if pose.crouched {
            pm.s.pm_flags |= flags::DUCKED;
        } else {
            pm.s.pm_flags &= !flags::DUCKED;
        }
    }
    let state = Q2MovementState {
        move_type: if mode.is_none() {
            pm.s.pm_type
        } else {
            input.state.move_type
        },
        origin_eighths: pm.s.origin,
        velocity_eighths: pm.s.velocity,
        flags: pm.s.pm_flags,
        time_eight_milliseconds: pm.s.pm_time,
        gravity: pm.s.gravity,
        delta_angle_shorts: pm.s.delta_angles,
    };
    let contacts = movement_contacts(&pm.touchtraces[..pm.numtouch.min(pm.touchtraces.len())]);
    let horizontal = n.sqrt(n.add(
        n.mul(pm.s.velocity[0] as f64 / 8.0, pm.s.velocity[0] as f64 / 8.0),
        n.mul(pm.s.velocity[1] as f64 / 8.0, pm.s.velocity[1] as f64 / 8.0),
    ));
    Ok(MovementOutcome::Active {
        fields: MovementResultFields {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            bounds: Bounds {
                min: vector(pm.mins),
                max: vector(pm.maxs),
            },
            view_angles: vector(pm.viewangles),
            view_height: pm.viewheight,
            ground: pm.groundentity.clone().unwrap_or(TraceHit::None),
            water_level: pm.waterlevel,
            water_type: pm.watertype,
            horizontal_speed: horizontal,
            contacts,
            effects: Vec::new(),
            arsenal: input.fields.arsenal.clone(),
            animation: input.fields.animation.clone(),
        },
        state,
    })
}

/// Run a classic Quake II step.
pub fn move_q2_classic<S: Q2MovementServices>(
    input: Q2MovementInput,
    services: &mut S,
) -> Result<Q2MovementResult, MovementError> {
    let authoritative = input.fields.execution == MovementExecution::Authoritative;
    if !authoritative || services.input_application().is_none() {
        return move_q2_classic_physics(input, services);
    }
    let mut frame = input.fields.frame.clone();
    frame.elapsed = SourceTime::Milliseconds(input.command.milliseconds);
    let before = services.input_application().expect("input application present").begin(
        UserCommand::Q2Classic(input.command),
        &frame,
        Q2State::Classic(input.state),
    );
    let result = match before {
        MovementInputContinuation::ActorRemoved => MovementOutcome::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: Vec::new(),
        },
        MovementInputContinuation::Continue { state, command } => {
            let (UserCommand::Q2Classic(command), Q2State::Classic(state)) = (command, state) else {
                services.input_application().expect("input application present").end(
                    Q2State::Classic(input.state),
                    true,
                    None,
                );
                return Err(MovementError::Contract(
                    "Input callback changed Quake II movement family",
                ));
            };
            match move_q2_classic_physics(
                Q2MovementInput {
                    command,
                    state,
                    ..input.clone()
                },
                services,
            ) {
                Ok(result) => result,
                Err(error) => {
                    services.input_application().expect("input application present").end(
                        Q2State::Classic(input.state),
                        true,
                        None,
                    );
                    return Err(error);
                }
            }
        }
    };
    let after = match &result {
        MovementOutcome::Active { fields, state } => {
            services.input_application().expect("input application present").end(
                Q2State::Classic(*state),
                false,
                Some((fields.bounds, fields.view_height)),
            )
        }
        MovementOutcome::ActorRemoved { .. } => services.input_application().expect("input application present").end(
            Q2State::Classic(input.state),
            false,
            None,
        ),
    };
    match after {
        MovementContinuation::ActorRemoved => Ok(MovementOutcome::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: match result {
                MovementOutcome::Active { fields, .. } => fields.effects,
                MovementOutcome::ActorRemoved { effects, .. } => effects,
            },
        }),
        MovementContinuation::Continue(state) => match result {
            MovementOutcome::ActorRemoved {
                actor,
                command_sequence,
                effects,
            } => Ok(MovementOutcome::ActorRemoved {
                actor,
                command_sequence,
                effects,
            }),
            MovementOutcome::Active { fields, .. } => {
                let Q2State::Classic(state) = state else {
                    return Err(MovementError::Contract(
                        "Input callback changed Quake II movement family",
                    ));
                };
                Ok(MovementOutcome::Active { fields, state })
            }
        },
    }
}

fn move_q2_rerelease_physics<S: Q2MovementServices>(
    input: Q2RereleaseMovementInput,
    services: &mut S,
    context: &mut Q2RereleaseMovementContext,
) -> Result<Q2RereleaseMovementResult, MovementError> {
    let output = input.fields.environment.client_outputs;
    let mode = client_movement_mode(output.as_ref(), input.fields.environment.health);
    let command = client_stance_command(UserCommand::Q2Rerelease(input.command), output.and_then(|o| o.stance))
        .map_err(|error| MovementError::Contract(error.0))?;
    let UserCommand::Q2Rerelease(command) = command else {
        return Err(MovementError::Contract("Client output changed movement dialect"));
    };
    let input = Q2RereleaseMovementInput { command, ..input };
    command_duration(input.command.milliseconds)?;
    let n = services.numeric();
    let pose = input.fields.environment.pose;
    let body = pose
        .map(|pose| pose.bounds)
        .unwrap_or_else(|| body_bounds(&input.fields.shape));
    let adapter = RefCell::new(TraceAdapter {
        services,
        entities: Vec::new(),
        leaf: crate::collision::LeafContents::Merged,
        point_shape: matches!(input.fields.shape, TraceShape::Point),
    });
    let mut pm = KexPmove {
        s: KexPmoveState {
            pm_type: match (pose, mode) {
                (Some(_), _) => types::kex_pm_type::FREEZE,
                (None, None) => input.state.move_type,
                (None, Some(mode)) => client_movement_type(MovementDialect::Q2Rerelease, mode),
            },
            origin: source_vector(input.state.origin),
            velocity: source_vector(if pose.is_none() { input.state.velocity } else { ZERO }),
            pm_flags: input.state.flags,
            pm_time: input.state.time_milliseconds,
            gravity: input.state.gravity,
            delta_angles: source_vector(input.state.delta_angles),
            viewheight: input.state.view_height,
        },
        cmd: KexPmoveCmd {
            msec: input.command.milliseconds,
            angles: source_vector(input.command.angles),
            forwardmove: input.command.forward_move,
            sidemove: input.command.side_move,
            buttons: input.command.buttons,
            server_frame: input.command.server_frame,
        },
        snapinitial: input.snap_initial,
        touch: KexTouchList {
            num: 0,
            traces: Vec::new(),
        },
        viewangles: [0.0, 0.0, 0.0],
        mins: source_vector(body.min),
        maxs: source_vector(body.max),
        character_bounds: body,
        previous_bounds: input.fields.current_bounds.or(Some(body)),
        body_bounds: match (pose, output.and_then(|o| o.body_bounds)) {
            (Some(_), _) | (None, None) => None,
            (None, Some(bounds)) => Some(bounds),
        },
        groundentity: None,
        groundplane: types::plane(),
        watertype: 0,
        waterlevel: 0,
        player: Some(TraceHit::Actor {
            actor: input.fields.actor.id().clone(),
        }),
        trace: Box::new(|start, mins, maxs, end, _pass, mask| {
            adapter.borrow_mut().trace(start, mins, maxs, end, mask, false)
        }),
        clip: Box::new(|start, mins, maxs, end, mask| adapter.borrow_mut().trace(start, mins, maxs, end, mask, true)),
        pointcontents: Box::new(|point| adapter.borrow_mut().pointcontents(point)),
        viewoffset: source_vector(input.view_offset),
        screen_blend: [0.0, 0.0, 0.0, 0.0],
        rdflags: 0,
        jump_sound: false,
        step_clip: false,
        impact_delta: 0.0,
    };
    let flight = input.fields.environment.flight && input.fields.environment.health > 0.0;
    let speed = input.fields.environment.speed_multiplier.unwrap_or(1.0);
    let config = PmConfig {
        airaccel: input.profile.air_accelerate,
        n64_physics: input.profile.n64_physics,
    };
    rerelease::pmove_rerelease(&mut pm, n, &config, context, flight, speed);
    let _ = PM_CONFIG_DEFAULT;
    if let Some(pose) = pose {
        pm.s.pm_type = input.state.move_type;
        pm.s.viewheight = pose.view_height;
        if pose.crouched {
            pm.s.pm_flags |= flags::DUCKED;
        } else {
            pm.s.pm_flags &= !flags::DUCKED;
        }
    }
    let presentation = Q2RereleasePresentation {
        screen_blend: qa_core::math::Vec4 {
            x: pm.screen_blend[0] as f32,
            y: pm.screen_blend[1] as f32,
            z: pm.screen_blend[2] as f32,
            w: pm.screen_blend[3] as f32,
        },
        render_flags: pm.rdflags,
        jump_sound: pm.jump_sound,
        step_clip: pm.step_clip,
        impact_delta: pm.impact_delta,
    };
    let state = Q2RereleaseMovementState {
        move_type: if mode.is_none() {
            pm.s.pm_type
        } else {
            input.state.move_type
        },
        origin: vector(pm.s.origin),
        velocity: vector(pm.s.velocity),
        flags: pm.s.pm_flags,
        time_milliseconds: pm.s.pm_time,
        gravity: pm.s.gravity,
        delta_angles: vector(pm.s.delta_angles),
        view_height: pm.s.viewheight,
    };
    let contacts = movement_contacts(&pm.touch.traces[..pm.touch.num.min(pm.touch.traces.len())]);
    let horizontal = n.sqrt(n.add(
        n.mul(pm.s.velocity[0], pm.s.velocity[0]),
        n.mul(pm.s.velocity[1], pm.s.velocity[1]),
    ));
    Ok(Q2RereleaseMovementResult::Active {
        fields: MovementResultFields {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            bounds: Bounds {
                min: vector(pm.mins),
                max: vector(pm.maxs),
            },
            view_angles: vector(pm.viewangles),
            view_height: pm.s.viewheight,
            ground: pm.groundentity.clone().unwrap_or(TraceHit::None),
            water_level: pm.waterlevel,
            water_type: pm.watertype,
            horizontal_speed: horizontal,
            contacts,
            effects: Vec::new(),
            arsenal: input.fields.arsenal.clone(),
            animation: input.fields.animation.clone(),
        },
        state,
        presentation,
    })
}

/// Run a rerelease Quake II step.
pub fn move_q2_rerelease<S: Q2MovementServices>(
    input: Q2RereleaseMovementInput,
    services: &mut S,
    context: &mut Q2RereleaseMovementContext,
) -> Result<Q2RereleaseMovementResult, MovementError> {
    let authoritative = input.fields.execution == MovementExecution::Authoritative;
    if !authoritative || services.input_application().is_none() {
        return move_q2_rerelease_physics(input, services, context);
    }
    let mut frame = input.fields.frame.clone();
    frame.elapsed = SourceTime::Milliseconds(input.command.milliseconds);
    let before = services.input_application().expect("input application present").begin(
        UserCommand::Q2Rerelease(input.command),
        &frame,
        Q2State::Rerelease(input.state),
    );
    let result = match before {
        MovementInputContinuation::ActorRemoved => Q2RereleaseMovementResult::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: Vec::new(),
        },
        MovementInputContinuation::Continue { state, command } => {
            let (UserCommand::Q2Rerelease(command), Q2State::Rerelease(state)) = (command, state) else {
                services.input_application().expect("input application present").end(
                    Q2State::Rerelease(input.state),
                    true,
                    None,
                );
                return Err(MovementError::Contract(
                    "Input callback changed Quake II movement family",
                ));
            };
            match move_q2_rerelease_physics(
                Q2RereleaseMovementInput {
                    command,
                    state,
                    ..input.clone()
                },
                services,
                context,
            ) {
                Ok(result) => result,
                Err(error) => {
                    services.input_application().expect("input application present").end(
                        Q2State::Rerelease(input.state),
                        true,
                        None,
                    );
                    return Err(error);
                }
            }
        }
    };
    let after = match &result {
        Q2RereleaseMovementResult::Active { fields, state, .. } => {
            services.input_application().expect("input application present").end(
                Q2State::Rerelease(*state),
                false,
                Some((fields.bounds, fields.view_height)),
            )
        }
        Q2RereleaseMovementResult::ActorRemoved { .. } => services
            .input_application()
            .expect("input application present")
            .end(Q2State::Rerelease(input.state), false, None),
    };
    match (after, result) {
        (MovementContinuation::ActorRemoved, result) => Ok(Q2RereleaseMovementResult::ActorRemoved {
            actor: input.fields.actor.id().clone(),
            command_sequence: input.fields.command_sequence,
            effects: match result {
                Q2RereleaseMovementResult::Active { fields, .. } => fields.effects,
                Q2RereleaseMovementResult::ActorRemoved { effects, .. } => effects,
            },
        }),
        (MovementContinuation::Continue(_), result @ Q2RereleaseMovementResult::ActorRemoved { .. }) => Ok(result),
        (
            MovementContinuation::Continue(state),
            Q2RereleaseMovementResult::Active {
                fields, presentation, ..
            },
        ) => {
            let Q2State::Rerelease(state) = state else {
                return Err(MovementError::Contract(
                    "Input callback changed Quake II movement family",
                ));
            };
            Ok(Q2RereleaseMovementResult::Active {
                fields,
                state,
                presentation,
            })
        }
    }
}

/// One source usercmd is one movement call. Q2 does not apply QW's
/// recursive splitting.
#[derive(Debug, Clone)]
pub struct Q2ClassicMovementProvider {
    /// Provider identity.
    pub id: ProviderId,
}

/// Build a classic Q2 movement provider.
pub fn create_q2_classic_movement_provider(id: ProviderId) -> Q2ClassicMovementProvider {
    Q2ClassicMovementProvider { id }
}

impl Q2ClassicMovementProvider {
    /// Run a classic Q2 step.
    pub fn move_step<S: Q2MovementServices>(
        &self,
        input: Q2MovementInput,
        services: &mut S,
    ) -> Result<Q2MovementResult, MovementError> {
        move_q2_classic(input, services)
    }
}

/// Rerelease Q2 movement provider with its shared pml context.
#[derive(Debug)]
pub struct Q2RereleaseMovementProvider {
    /// Provider identity.
    pub id: ProviderId,
    /// Shared pml context.
    pub context: Q2RereleaseMovementContext,
}

/// Build a rerelease Q2 movement provider.
pub fn create_q2_rerelease_movement_provider(
    id: ProviderId,
    context: Q2RereleaseMovementContext,
) -> Q2RereleaseMovementProvider {
    Q2RereleaseMovementProvider { id, context }
}

impl Q2RereleaseMovementProvider {
    /// Run a rerelease Q2 step.
    pub fn move_step<S: Q2MovementServices>(
        &mut self,
        input: Q2RereleaseMovementInput,
        services: &mut S,
    ) -> Result<Q2RereleaseMovementResult, MovementError> {
        move_q2_rerelease(input, services, &mut self.context)
    }
}

/// Collision body defaults are explicit at recipe assembly, never inferred
/// from a character model.
#[must_use]
pub fn q2_player_shape() -> TraceShape {
    TraceShape::Box(Q2_PLAYER_BOUNDS)
}

/// ClientThink calls this after committing/linking the body and running
/// trigger touches. Pass the state returned by triggers; a removed result
/// is returned without callbacks.
pub fn apply_q2_movement_contacts<S: Q2MovementServices>(
    input: &Q2MovementInput,
    services: &mut S,
    result: Q2MovementResult,
) -> Result<Q2MovementResult, MovementError> {
    if !same_actor(input.fields.actor.id(), result_actor(&result)) {
        return Err(MovementError::Contract(
            "Q2 contact result does not belong to this movement input",
        ));
    }
    let MovementOutcome::Active { fields, state } = result else {
        return Ok(result);
    };
    let mut effects = fields.effects.clone();
    let next = touch_contacts(
        false,
        &input.fields.actor,
        input.fields.frame.time,
        services,
        Q2State::Classic(state),
        &fields.contacts,
        &mut effects,
    )?;
    match next {
        MovementContinuation::ActorRemoved => Ok(MovementOutcome::ActorRemoved {
            actor: fields.actor.clone(),
            command_sequence: fields.command_sequence,
            effects,
        }),
        MovementContinuation::Continue(Q2State::Classic(state)) => Ok(MovementOutcome::Active {
            fields: MovementResultFields { effects, ..fields },
            state,
        }),
        MovementContinuation::Continue(_) => Err(MovementError::Contract("Q2 contacts changed the movement family")),
    }
}

/// Rerelease contact application (see [`apply_q2_movement_contacts`]).
pub fn apply_q2_rerelease_contacts<S: Q2MovementServices>(
    input: &Q2RereleaseMovementInput,
    services: &mut S,
    result: Q2RereleaseMovementResult,
) -> Result<Q2RereleaseMovementResult, MovementError> {
    let actor = match &result {
        Q2RereleaseMovementResult::Active { fields, .. } => fields.actor.clone(),
        Q2RereleaseMovementResult::ActorRemoved { actor, .. } => actor.clone(),
    };
    if !same_actor(input.fields.actor.id(), &actor) {
        return Err(MovementError::Contract(
            "Q2 contact result does not belong to this movement input",
        ));
    }
    let Q2RereleaseMovementResult::Active {
        fields,
        state,
        presentation,
    } = result
    else {
        return Ok(result);
    };
    let mut effects = fields.effects.clone();
    let next = touch_contacts(
        true,
        &input.fields.actor,
        input.fields.frame.time,
        services,
        Q2State::Rerelease(state),
        &fields.contacts,
        &mut effects,
    )?;
    match next {
        MovementContinuation::ActorRemoved => Ok(Q2RereleaseMovementResult::ActorRemoved {
            actor: fields.actor.clone(),
            command_sequence: fields.command_sequence,
            effects,
        }),
        MovementContinuation::Continue(Q2State::Rerelease(state)) => Ok(Q2RereleaseMovementResult::Active {
            fields: MovementResultFields { effects, ..fields },
            state,
            presentation,
        }),
        MovementContinuation::Continue(_) => Err(MovementError::Contract("Q2 contacts changed the movement family")),
    }
}

fn result_actor(result: &Q2MovementResult) -> &qa_core::identity::ActorId {
    match result {
        MovementOutcome::Active { fields, .. } => &fields.actor,
        MovementOutcome::ActorRemoved { actor, .. } => actor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::numeric::{NumericOps, Q2_DONOR_PROFILE};
    use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};

    use super::super::types::{
        ActorAnimationState, AnimationState, ArsenalState, MovementEnvironment, MovementInputFields,
        Q2RereleaseUserCommand, Q2UserCommand, WeaponState,
    };
    use types::{Q2MovementProfile, Q2RereleaseMovementProfile};

    struct NullServices {
        ops: NumericOps,
        contents: i32,
        touches: usize,
    }

    impl Q2MovementServices for NullServices {
        fn numeric(&self) -> NumericOps {
            self.ops
        }
        fn trace(&mut self, query: Q2TraceQuery) -> Q2Trace {
            let _ = query.mins;
            Q2Trace {
                fraction: 1.0,
                end: query.end,
                start_solid: false,
                all_solid: false,
                contact: TraceContact::None,
                hit: TraceHit::None,
                contents: 0,
                surface: None,
                source_plane: Q2TracePlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    dist: 0.0,
                    plane_type: 0,
                    signbits: 0,
                },
                secondary: None,
            }
        }
        fn point_contents(&mut self, _query: Q2ContentsQuery) -> (i32, i32) {
            (self.contents, self.contents)
        }
        fn touch(&mut self, _contact: Q2TouchContact, state: Q2State) -> MovementContinuation<Q2State> {
            self.touches += 1;
            MovementContinuation::Continue(state)
        }
    }

    fn fields() -> MovementInputFields {
        let owner = IdentityOwner::create("q2-mod").unwrap();
        let id = owner.actor(1, 0);
        let actor = owner.owned_actor(&id, ProviderId::new("q2", "test")).unwrap();
        MovementInputFields {
            actor,
            command_sequence: 1,
            frame: FrameContext {
                frame: 1,
                time: SourceTime::Milliseconds(1000),
                elapsed: SourceTime::Milliseconds(50),
                phase: FramePhase::ClientCommand,
            },
            shape: TraceShape::Box(Q2_PLAYER_BOUNDS),
            current_bounds: None,
            environment: MovementEnvironment::default(),
            arsenal: ArsenalState {
                provider: ProviderId::new("q2", "test"),
                active_weapon: None,
                state: WeaponState::Q2 {
                    gun_frame: 0,
                    state: 0,
                    pending_weapon: None,
                    machinegun_shots: 0,
                    grenade_time: SourceTime::Seconds(0.0),
                    grenade_blew_up: false,
                },
                ammo: Vec::new(),
            },
            animation: ActorAnimationState {
                provider: ProviderId::new("q2", "test"),
                state: AnimationState::Q2 {
                    frame: 0,
                    end_frame: 0,
                    priority: 0,
                    duck: false,
                    run: false,
                },
            },
            execution: MovementExecution::Authoritative,
        }
    }

    fn classic_input() -> Q2MovementInput {
        Q2MovementInput {
            fields: fields(),
            command: Q2UserCommand {
                milliseconds: 50,
                angle_shorts: [0, 8192, 0],
                forward_move: 400.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
                light_level: 0,
            },
            state: Q2MovementState {
                move_type: types::pm_type::NORMAL,
                origin_eighths: [0, 0, 800],
                velocity_eighths: [0, 0, 0],
                flags: types::pm_flags::ON_GROUND,
                time_eight_milliseconds: 0,
                gravity: 800.0,
                delta_angle_shorts: [0, 0, 0],
            },
            profile: Q2MovementProfile {
                id: ProviderId::new("q2", "test"),
                clock: ClockProfile::Q2Classic,
                numeric: Q2_DONOR_PROFILE,
                strafejump_hack: false,
                air_accelerate: 0.0,
                snap_initial: false,
            },
        }
    }

    fn rerelease_input() -> Q2RereleaseMovementInput {
        Q2RereleaseMovementInput {
            fields: fields(),
            command: Q2RereleaseUserCommand {
                milliseconds: 16,
                angles: vec3(0.0, 90.0, 0.0),
                forward_move: 400.0,
                side_move: 0.0,
                buttons: 0,
                server_frame: 1,
            },
            state: Q2RereleaseMovementState {
                move_type: types::kex_pm_type::NORMAL,
                origin: vec3(0.0, 0.0, 100.0),
                velocity: vec3(0.0, 0.0, 0.0),
                flags: types::pm_flags::ON_GROUND,
                time_milliseconds: 0,
                gravity: 800.0,
                delta_angles: vec3(0.0, 0.0, 0.0),
                view_height: 22.0,
            },
            profile: Q2RereleaseMovementProfile {
                id: ProviderId::new("q2", "test"),
                clock: ClockProfile::Q2Rerelease {
                    frame_milliseconds: 100.0,
                },
                numeric: Q2_DONOR_PROFILE,
                air_accelerate: 0.0,
                n64_physics: false,
            },
            view_offset: vec3(0.0, 0.0, 0.0),
            snap_initial: false,
        }
    }

    fn services() -> NullServices {
        NullServices {
            ops: NumericOps::select(Q2_DONOR_PROFILE).unwrap(),
            contents: 0,
            touches: 0,
        }
    }

    #[test]
    fn classic_step_moves_and_snaps() {
        let mut services = services();
        let result = move_q2_classic(classic_input(), &mut services).unwrap();
        match result {
            MovementOutcome::Active { fields, state } => {
                assert!(state.velocity_eighths[1] > 0);
                assert!(fields.horizontal_speed > 0.0);
                assert_eq!(fields.view_height, 22.0);
            }
            MovementOutcome::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }

    #[test]
    fn classic_rejects_bad_durations() {
        let mut input = classic_input();
        input.command.milliseconds = 300;
        assert!(move_q2_classic(input, &mut services()).is_err());
    }

    #[test]
    fn rerelease_step_reports_presentation() {
        let mut services = services();
        let mut context = Q2RereleaseMovementContext::new();
        let result = move_q2_rerelease(rerelease_input(), &mut services, &mut context).unwrap();
        match result {
            Q2RereleaseMovementResult::Active {
                state,
                presentation,
                fields,
            } => {
                assert!(state.velocity.y > 0.0);
                assert!(!presentation.jump_sound);
                assert_eq!(fields.view_height, 22.0);
            }
            Q2RereleaseMovementResult::ActorRemoved { .. } => panic!("unexpected removal"),
        }
    }

    #[test]
    fn contacts_apply_without_touches() {
        let mut services = services();
        let input = classic_input();
        let result = move_q2_classic(input.clone(), &mut services).unwrap();
        let applied = apply_q2_movement_contacts(&input, &mut services, result).unwrap();
        assert!(!applied.removed());
        assert_eq!(services.touches, 0);
    }

    #[test]
    fn player_shape_is_source_box() {
        assert_eq!(q2_player_shape(), TraceShape::Box(Q2_PLAYER_BOUNDS));
    }
}
