//! Cgame prediction adapter over the selected movement provider.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/prediction/presentation.ts`
//! (`PresentationPredictionOptions`, `presentationSourceCommand`,
//! `PresentationPredictionAdapter`, `createPresentationMovementHost`,
//! `createSimulationPredictionHost`).
//!
//! # Sibling homes
//!
//! - [`SharedSimulation`](super::super::runtime::SharedSimulation)
//!   (`simulation/runtime.ts` port) and
//!   [`MovementPredictionPlayer`](super::super::player_movement::MovementPredictionPlayer)
//!   (`player-movement.ts` port):
//!   [`create_simulation_prediction_host`] takes the narrow
//!   [`SimulationPredictionSource`] seam instead of the full simulation.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_content::contract::GameFamily;
use qa_content::q3::base::shared::player_state::{PlayerState, UserCommand as Q3SourceUserCommand};
use qa_content::q3::base::world::{TraceContact as ContentTraceContact, TraceSolidity};
use qa_content::q3::presentation::movement_host::{
    CommandTiming, MoveBounds, PresentationMovementHost, PresentationMovementOptions,
};
use qa_core::identity::{ActorId, OwnedActor, SeatId};
use qa_core::math::{vec3, Bounds, Plane, Vec3};
use qa_net::common::commands::{ActorCommand, UserCommand as NetUserCommand};
use qa_world::hull::BspPlane;
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::q3::prediction::update_q3_prediction_view;
use qa_world::movement::q3::types::{Q3Trace, Q3TraceQuery};
use qa_world::movement::types::{
    AnimationState, ArsenalState, MovementDialect, MovementEffect, Q1UserCommand, Q2RereleaseUserCommand,
    Q2UserCommand, Q3UserCommand, QwUserCommand, TraceContact, TraceHit, TraceShape, UserCommand as WorldUserCommand,
    WeaponState,
};

use super::super::arsenal::selected::MovementState;
use super::super::arsenal_intent::resolve_q3_arsenal_controls;
use super::super::player_jump::publish_q3_character_movement_event;
use super::super::q3::presentation::Q3SourcePresentationState;
use super::super::q3_commands::relative_q3_source_command;
use super::runtime::copy_prediction_command;
use super::source_state::{
    prediction_source_hit, read_prediction_source_state, write_prediction_source_state, PredictionSourceEntities,
};
use super::step::{copy_prediction_snapshot, predict_movement_command, PredictedCommand};
use super::types::{
    MovementPredictionOptions, MovementPredictionProfile, MovementPredictionSnapshot, MovementProbeOptions,
    PredictionCommand, PredictionPosturesHandle, PredictionSceneHandle, PredictionStepOptions,
};

/// Presentation prediction options.
#[derive(Clone)]
pub struct PresentationPredictionOptions {
    /// Movement prediction options.
    pub movement: MovementPredictionOptions,
    /// Initial snapshot.
    pub initial: MovementPredictionSnapshot,
    /// Source entity mapping.
    pub entities: Rc<dyn PredictionSourceEntities>,
}

/// Errors presenting prediction commands and snapshots.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PresentationPredictionError {
    /// Snapshot changed the selected movement.
    #[error("Cgame prediction snapshot changed selected movement")]
    MovementChanged,
    /// Command belongs to another actor.
    #[error("Cgame prediction command belongs to another actor")]
    ForeignActor,
    /// No selected arsenal owner for the intent.
    #[error("Cgame has no selected arsenal owner for this intent")]
    NoArsenalOwner,
    /// No snapshot for a source command time.
    #[error("No selected movement snapshot for source command time {0}")]
    NoSnapshot(i32),
    /// No original command for a cgame time.
    #[error("No original selected command for cgame time {0}")]
    NoOriginalCommand(i32),
    /// Arsenal controls failed to resolve.
    #[error("Cgame arsenal controls failed: {0}")]
    Arsenal(String),
    /// Source state failed to read.
    #[error("Cgame source state failed: {0}")]
    SourceState(String),
    /// Movement step failed.
    #[error("Cgame movement step failed: {0}")]
    Step(String),
}

/// Convert an actor command to the source cgame command.
#[must_use]
pub fn presentation_source_command(input: &ActorCommand, milliseconds: f64, weapon: i32) -> Q3SourceUserCommand {
    match &input.command {
        NetUserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon: command_weapon,
            forward_move,
            right_move,
            up_move,
        } => {
            let buttons = *buttons as i32;
            Q3SourceUserCommand {
                server_time: *server_time_milliseconds as i32,
                angles: vec3(angle_words[0] as f32, angle_words[1] as f32, angle_words[2] as f32),
                buttons: match &input.arsenal {
                    None => buttons,
                    Some(arsenal) => (buttons & !4) | (i32::from(arsenal.use_holdable) * 4),
                },
                weapon: if input.arsenal.is_none() {
                    *command_weapon as i32
                } else {
                    weapon
                },
                forwardmove: *forward_move as i32,
                rightmove: *right_move as i32,
                upmove: *up_move as i32,
            }
        }
        other => {
            let angles = match other {
                NetUserCommand::Q2Classic { angle_shorts, .. } => {
                    vec3(angle_shorts[0] as f32, angle_shorts[1] as f32, angle_shorts[2] as f32)
                }
                NetUserCommand::Q1Netquake { view_angles, .. } => angle_words_f64(view_angles),
                NetUserCommand::Q1Quakeworld { angles, .. } => angle_words_f64(angles),
                NetUserCommand::Q2Rerelease { .. } => vec3(0.0, 0.0, 0.0),
                NetUserCommand::Q3 { .. } => unreachable!("q3 handled above"),
            };
            let (forward_move, side_move, buttons) = match other {
                NetUserCommand::Q1Netquake {
                    forward_move,
                    side_move,
                    buttons,
                    ..
                }
                | NetUserCommand::Q1Quakeworld {
                    forward_move,
                    side_move,
                    buttons,
                    ..
                }
                | NetUserCommand::Q2Classic {
                    forward_move,
                    side_move,
                    buttons,
                    ..
                } => (*forward_move, *side_move, *buttons as i32),
                NetUserCommand::Q2Rerelease {
                    forward_move,
                    side_move,
                    buttons,
                    ..
                } => (*forward_move, *side_move, *buttons as i32),
                NetUserCommand::Q3 { .. } => unreachable!("q3 handled above"),
            };
            let speed = if matches!(
                other,
                NetUserCommand::Q1Netquake { .. } | NetUserCommand::Q1Quakeworld { .. }
            ) {
                320.0
            } else {
                200.0
            };
            let axis = |value: f64| (value * 127.0 / speed).clamp(-127.0, 127.0).trunc() as i32;
            let upmove = match other {
                NetUserCommand::Q2Rerelease { .. } => {
                    if buttons & 8 != 0 {
                        127
                    } else if buttons & 16 != 0 {
                        -127
                    } else {
                        0
                    }
                }
                NetUserCommand::Q1Netquake { up_move, .. } | NetUserCommand::Q1Quakeworld { up_move, .. } => {
                    if buttons & 2 != 0 {
                        127
                    } else {
                        axis(*up_move)
                    }
                }
                NetUserCommand::Q2Classic { up_move, .. } => axis(*up_move),
                NetUserCommand::Q3 { .. } => unreachable!("q3 handled above"),
            };
            Q3SourceUserCommand {
                server_time: milliseconds.trunc() as i32,
                angles,
                buttons: (buttons & 1)
                    | if input.arsenal.as_ref().is_some_and(|arsenal| arsenal.use_holdable) {
                        4
                    } else {
                        0
                    },
                weapon,
                forwardmove: axis(forward_move),
                rightmove: axis(side_move),
                upmove,
            }
        }
    }
}

/// Convert degree angles to Q3 angle words.
fn angle_words_f64(angles: &[f64; 3]) -> Vec3 {
    vec3(word_of(angles[0]), word_of(angles[1]), word_of(angles[2]))
}

/// Convert one degree angle to a Q3 angle word.
fn word_of(degrees: f64) -> f32 {
    ((degrees * 65536.0 / 360.0).trunc() as i32 & 65535) as f32
}

/// One cgame seat's prediction adapter over one world lifetime.
pub struct PresentationPredictionAdapter {
    /// Construction options.
    pub options: PresentationPredictionOptions,
    rerelease_movement: Rc<RefCell<Q2RereleaseMovementContext>>,
    snapshots: HashMap<i32, MovementPredictionSnapshot>,
    snapshot_order: Vec<i32>,
    commands: HashMap<i32, PredictionCommand>,
    command_order: Vec<i32>,
    states: HashMap<i32, MovementPredictionSnapshot>,
    latest: MovementPredictionSnapshot,
}

impl PresentationPredictionAdapter {
    /// Capture the initial snapshot.
    pub fn new(options: PresentationPredictionOptions) -> Result<Self, PresentationPredictionError> {
        // Donor checks actor/session equality here; the Rust identity keeps
        // session words private, so construction sites own both handles.
        let latest = copy_prediction_snapshot(&options.initial);
        let mut adapter = Self {
            options,
            rerelease_movement: Rc::new(RefCell::new(Q2RereleaseMovementContext::new())),
            snapshots: HashMap::new(),
            snapshot_order: Vec::new(),
            commands: HashMap::new(),
            command_order: Vec::new(),
            states: HashMap::new(),
            latest: latest.clone(),
        };
        adapter.capture(latest)?;
        Ok(adapter)
    }

    /// Capture an authoritative snapshot.
    pub fn capture(&mut self, snapshot: MovementPredictionSnapshot) -> Result<(), PresentationPredictionError> {
        if movement_dialect(&snapshot.state) != self.options.movement.profile.dialect() {
            return Err(PresentationPredictionError::MovementChanged);
        }
        let time = snapshot.command_time_milliseconds as i32;
        self.states.clear();
        let latest = copy_prediction_snapshot(&snapshot);
        self.snapshots.insert(time, latest.clone());
        self.snapshot_order.push(time);
        if self.snapshots.len() > 64 {
            let first = self.snapshot_order.remove(0);
            self.snapshots.remove(&first);
        }
        self.latest = latest;
        Ok(())
    }

    /// Submit an actor command, returning the presented source command.
    pub fn submit(
        &mut self,
        input: &ActorCommand,
        milliseconds: f64,
    ) -> Result<Q3SourceUserCommand, PresentationPredictionError> {
        if input.actor != *self.options.movement.actor.id() {
            return Err(PresentationPredictionError::ForeignActor);
        }
        let world_command = world_command_of(&input.command);
        let q3_arsenal = self.latest.q3_arsenal.is_some();
        let is_q3 = matches!(self.latest.arsenal.state, WeaponState::Q3 { .. }) && q3_arsenal;
        let weapon = if is_q3 {
            let product = self.latest.q3_arsenal.as_ref().expect("q3 arsenal").product;
            resolve_q3_arsenal_controls(&self.latest.arsenal, input.arsenal.as_ref(), &world_command, product)
                .map_err(|error| PresentationPredictionError::Arsenal(error.to_string()))?
                .requested_weapon
        } else {
            0
        };
        if input.arsenal.is_some() && !is_q3 {
            return Err(PresentationPredictionError::NoArsenalOwner);
        }
        let presented = presentation_source_command(input, milliseconds, weapon);
        let source = match &self.latest.state {
            MovementState::Q3(state) => relative_q3_source_command(
                &input.source,
                input.command.dialect(),
                &presented,
                &vec3(
                    state.delta_angle_words[0] as f32,
                    state.delta_angle_words[1] as f32,
                    state.delta_angle_words[2] as f32,
                ),
                None,
            ),
            _ => presented,
        };
        let entry = PredictionCommand {
            angle_space: None,
            sequence: input.sequence as i64,
            time_milliseconds: source.server_time as f64,
            command: copy_prediction_command(&world_command),
            arsenal: input.arsenal.clone(),
        };
        self.commands.insert(source.server_time, entry);
        self.command_order.push(source.server_time);
        if self.commands.len() > 64 {
            let first = self.command_order.remove(0);
            self.commands.remove(&first);
        }
        Ok(source)
    }

    /// Seed replay state for a source player state.
    fn seed(&self, ps: &PlayerState) -> Result<MovementPredictionSnapshot, PresentationPredictionError> {
        let prior = self.states.get(&ps.client_num);
        let baseline = if self.options.movement.profile.dialect() == MovementDialect::Q3 {
            Some(&self.latest)
        } else if prior.is_some_and(|snapshot| snapshot.command_time_milliseconds as i32 == ps.command_time) {
            prior
        } else {
            self.snapshots.get(&ps.command_time)
        };
        let baseline = baseline.ok_or(PresentationPredictionError::NoSnapshot(ps.command_time))?;
        read_prediction_source_state(baseline, ps, self.options.entities.as_ref())
            .map_err(|error| PresentationPredictionError::SourceState(error.to_string()))
    }
}

/// Selected-movement dialect of a movement state.
fn movement_dialect(state: &MovementState) -> MovementDialect {
    match state {
        MovementState::Q1Netquake(_) => MovementDialect::Q1Netquake,
        MovementState::Q1Quakeworld(_) => MovementDialect::Q1Quakeworld,
        MovementState::Q2Classic(_) => MovementDialect::Q2Classic,
        MovementState::Q2Rerelease(_) => MovementDialect::Q2Rerelease,
        MovementState::Q3(_) => MovementDialect::Q3,
    }
}

/// Convert a unified net command to its world movement twin.
fn world_command_of(command: &NetUserCommand) -> WorldUserCommand {
    match command {
        NetUserCommand::Q1Netquake {
            acknowledged_server_time_seconds,
            view_angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => WorldUserCommand::Q1Netquake(Q1UserCommand {
            acknowledged_server_time_seconds: *acknowledged_server_time_seconds,
            view_angles: vec3(view_angles[0] as f32, view_angles[1] as f32, view_angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
        }),
        NetUserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => WorldUserCommand::Q1Quakeworld(QwUserCommand {
            milliseconds: *milliseconds as i32,
            angles: vec3(angles[0] as f32, angles[1] as f32, angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
        }),
        NetUserCommand::Q2Classic {
            milliseconds,
            angle_shorts,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            light_level,
        } => WorldUserCommand::Q2Classic(Q2UserCommand {
            milliseconds: *milliseconds as i32,
            angle_shorts: [angle_shorts[0] as i32, angle_shorts[1] as i32, angle_shorts[2] as i32],
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse: *impulse as i32,
            light_level: *light_level as i32,
        }),
        NetUserCommand::Q2Rerelease {
            milliseconds,
            angles,
            forward_move,
            side_move,
            buttons,
            server_frame,
        } => WorldUserCommand::Q2Rerelease(Q2RereleaseUserCommand {
            milliseconds: *milliseconds as i32,
            angles: vec3(angles[0] as f32, angles[1] as f32, angles[2] as f32),
            forward_move: *forward_move,
            side_move: *side_move,
            buttons: *buttons as i32,
            server_frame: *server_frame as i32,
        }),
        NetUserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
        } => WorldUserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: *server_time_milliseconds as i32,
            angle_words: [angle_words[0] as i32, angle_words[1] as i32, angle_words[2] as i32],
            buttons: *buttons as i32,
            weapon: *weapon as i32,
            forward_move: *forward_move as i32,
            right_move: *right_move as i32,
            up_move: *up_move as i32,
        }),
    }
}

/// Scene overlay routing Q3 world traces through presentation movement.
struct SceneOverlay<'a> {
    base: PredictionSceneHandle,
    movement: &'a dyn PresentationMovementOptions,
    client_num: i32,
    entities: Rc<dyn PredictionSourceEntities>,
    route_q3: bool,
}

impl<'a> SceneOverlay<'a> {
    /// Trace Q3 through presentation movement.
    fn trace_q3_routed(&self, query: Q3TraceQuery) -> Q3Trace {
        let bounds = if query.point {
            Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            }
        } else {
            query.bounds
        };
        let result = self
            .movement
            .trace(query.start, query.end, bounds, self.client_num, query.mask);
        let normal = match &result.base.contact {
            ContentTraceContact::Plane { plane } => plane.normal,
            ContentTraceContact::None => vec3(0.0, 0.0, 0.0),
        };
        let source_plane = BspPlane {
            normal,
            distance: match &result.base.contact {
                ContentTraceContact::Plane { plane } => plane.distance,
                ContentTraceContact::None => 0.0,
            },
            plane_type: if normal.x == 1.0 {
                0
            } else if normal.y == 1.0 {
                1
            } else if normal.z == 1.0 {
                2
            } else {
                3
            },
            signbits: (u8::from(normal.x < 0.0)) | (u8::from(normal.y < 0.0) << 1) | (u8::from(normal.z < 0.0) << 2),
        };
        Q3Trace {
            fraction: result.base.fraction as f64,
            end: result.base.end,
            all_solid: result.base.solidity == TraceSolidity::AllSolid,
            start_solid: result.base.solidity != TraceSolidity::Clear,
            source_plane,
            contact: match result.base.contact {
                ContentTraceContact::None => TraceContact::None,
                ContentTraceContact::Plane { .. } => TraceContact::Plane(Plane {
                    normal: source_plane.normal,
                    distance: source_plane.distance,
                }),
            },
            hit: prediction_source_hit(result.entity_num, self.entities.as_ref()),
            contents: result.base.contents,
            surface_flags: result.base.surface_flags,
        }
    }
}

impl<'a> super::types::PredictionScene for SceneOverlay<'a> {
    fn trace_q1(
        &mut self,
        query: qa_world::movement::q1::types::Q1TraceQuery,
    ) -> qa_world::movement::q1::types::Q1Trace {
        self.base.borrow_mut().trace_q1(query)
    }

    fn point_contents_q1(&mut self, point: Vec3) -> i32 {
        self.base.borrow_mut().point_contents_q1(point)
    }

    fn trace_q2(
        &mut self,
        query: qa_world::movement::q2::types::Q2TraceQuery,
    ) -> qa_world::movement::q2::types::Q2Trace {
        self.base.borrow_mut().trace_q2(query)
    }

    fn point_contents_q2(&mut self, query: qa_world::movement::q2::types::Q2ContentsQuery) -> (i32, i32) {
        self.base.borrow_mut().point_contents_q2(query)
    }

    fn trace_q3(&mut self, query: Q3TraceQuery) -> Q3Trace {
        if self.route_q3 {
            self.trace_q3_routed(query)
        } else {
            self.base.borrow_mut().trace_q3(query)
        }
    }

    fn point_contents_q3(&mut self, point: Vec3, pass_actor: &ActorId) -> i32 {
        if self.route_q3 {
            self.movement.point_contents(point, self.client_num)
        } else {
            self.base.borrow_mut().point_contents_q3(point, pass_actor)
        }
    }
}

impl PresentationMovementHost for PresentationPredictionAdapter {
    fn command_timing(&self) -> CommandTiming {
        if self.options.movement.profile.dialect() == MovementDialect::Q3 {
            CommandTiming::Q3
        } else {
            CommandTiming::Provider
        }
    }

    fn move_player(
        &mut self,
        ps: &mut PlayerState,
        command: &Q3SourceUserCommand,
        movement: &dyn PresentationMovementOptions,
    ) -> MoveBounds {
        let seed = self.seed(ps).expect("cgame prediction seed");
        let original_time = movement.original_server_time().unwrap_or(command.server_time);
        let original = self.commands.get(&original_time);
        let entry = if self.options.movement.profile.dialect() != MovementDialect::Q3 {
            original
                .cloned()
                .unwrap_or_else(|| panic!("No original selected command for cgame time {}", command.server_time))
        } else {
            PredictionCommand {
                angle_space: original.and_then(|entry| entry.angle_space),
                sequence: original.map_or(command.server_time as i64, |entry| entry.sequence),
                time_milliseconds: command.server_time as f64,
                command: WorldUserCommand::Q3(Q3UserCommand {
                    server_time_milliseconds: command.server_time,
                    angle_words: [
                        command.angles.x as i32,
                        command.angles.y as i32,
                        command.angles.z as i32,
                    ],
                    buttons: command.buttons,
                    weapon: command.weapon,
                    forward_move: command.forwardmove,
                    right_move: command.rightmove,
                    up_move: command.upmove,
                }),
                arsenal: original.and_then(|entry| entry.arsenal.clone()),
            }
        };
        let first_command = !self.states.contains_key(&ps.client_num)
            || self
                .states
                .get(&ps.client_num)
                .is_some_and(|state| state.command_time_milliseconds as i32 != ps.command_time);
        let probe = MovementProbeOptions::from(&self.options.movement);
        // SAFETY: the overlay only lives for the synchronous predict call
        // below, so the borrowed movement outlives every use.
        let movement_static: &'static dyn PresentationMovementOptions = unsafe {
            std::mem::transmute::<&dyn PresentationMovementOptions, &'static dyn PresentationMovementOptions>(movement)
        };
        let overlay: PredictionSceneHandle = Rc::new(RefCell::new(SceneOverlay {
            base: self.options.movement.scene.clone(),
            movement: movement_static,
            client_num: ps.client_num,
            entities: self.options.entities.clone(),
            route_q3: self.options.movement.profile.dialect() == MovementDialect::Q3,
        }));
        let output: PredictedCommand = predict_movement_command(
            &probe,
            &seed,
            &entry,
            &PredictionStepOptions {
                scene: overlay,
                rerelease_movement: self.rerelease_movement.clone(),
                fixed_milliseconds: movement.fixed_msec(),
                no_footsteps: movement.no_footsteps(),
                gauntlet_hit: movement.gauntlet_hit(),
                trace_mask: Some(movement.trace_mask()),
                first_command,
            },
            TraceShape::Box(self.options.movement.standing_bounds),
        )
        .expect("cgame movement step");
        write_prediction_source_state(ps, &output.player, self.options.entities.as_ref());
        let family = match output.player.animation.state {
            AnimationState::Q1 { .. } => GameFamily::Q1,
            AnimationState::Q2 { .. } => GameFamily::Q2,
            AnimationState::Q3 { .. } => GameFamily::Q3,
        };
        for effect in &output.effects {
            if let MovementEffect::Event(event) = &effect.effect {
                let owner = format!("{}:{}", event.provider.namespace, event.provider.name);
                if owner.starts_with("q3:") && publish_q3_character_movement_event(family, event.event) {
                    ps.add_event(event.event, event.parameter);
                }
            }
        }
        let bounds = output.player.bounds;
        self.states
            .insert(ps.client_num, copy_prediction_snapshot(&output.player));
        MoveBounds { bounds }
    }

    fn update_view_angles(&mut self, ps: &mut PlayerState, command: &Q3SourceUserCommand) {
        let seed = self.seed(ps).expect("cgame prediction seed");
        if !matches!(seed.state, MovementState::Q3(_)) {
            let original = self.commands.get(&command.server_time).map(|entry| &entry.command);
            match (original, &seed.state) {
                (Some(WorldUserCommand::Q1Netquake(original)), MovementState::Q1Netquake(state)) => {
                    ps.viewangles = if state.fix_angle {
                        state.view_angles
                    } else {
                        original.view_angles
                    };
                }
                (Some(WorldUserCommand::Q1Quakeworld(original)), _) => {
                    ps.viewangles = original.angles;
                }
                (Some(WorldUserCommand::Q2Rerelease(original)), MovementState::Q2Rerelease(state)) => {
                    ps.viewangles = vec3(
                        original.angles.x + state.delta_angles.x,
                        original.angles.y + state.delta_angles.y,
                        original.angles.z + state.delta_angles.z,
                    );
                }
                (Some(WorldUserCommand::Q2Classic(original)), MovementState::Q2Classic(state)) => {
                    let angle = |word: i32, delta: i32| (((word + delta) << 16 >> 16) as f32) * 360.0 / 65536.0;
                    ps.viewangles = vec3(
                        angle(original.angle_shorts[0], state.delta_angle_shorts[0]),
                        angle(original.angle_shorts[1], state.delta_angle_shorts[1]),
                        angle(original.angle_shorts[2], state.delta_angle_shorts[2]),
                    );
                }
                _ => {}
            }
            return;
        }
        let MovementState::Q3(state) = &seed.state else {
            unreachable!("q3 checked above")
        };
        let updated = update_q3_prediction_view(
            state,
            ps.health() as f64,
            &Q3UserCommand {
                server_time_milliseconds: command.server_time,
                angle_words: [
                    command.angles.x as i32,
                    command.angles.y as i32,
                    command.angles.z as i32,
                ],
                buttons: command.buttons,
                weapon: command.weapon,
                forward_move: command.forwardmove,
                right_move: command.rightmove,
                up_move: command.upmove,
            },
        );
        ps.viewangles = updated.view_angles;
        ps.delta_angles = vec3(
            updated.delta_angle_words[0] as f32,
            updated.delta_angle_words[1] as f32,
            updated.delta_angle_words[2] as f32,
        );
    }
}

/// Create a presentation movement host.
pub fn create_presentation_movement_host(
    options: PresentationPredictionOptions,
) -> Result<PresentationPredictionAdapter, PresentationPredictionError> {
    PresentationPredictionAdapter::new(options)
}

/// Narrow player view behind [`create_simulation_prediction_host`].
pub trait SimulationPredictionPlayer {
    /// Owning actor.
    fn actor(&self) -> OwnedActor;
    /// Last command sequence.
    fn last_sequence(&self) -> i64;
    /// Movement state.
    fn state(&self) -> MovementState;
    /// Arsenal state.
    fn arsenal(&self) -> ArsenalState;
    /// Animation state.
    fn animation(&self) -> qa_world::movement::types::ActorAnimationState;
    /// Source environment, when the provider publishes one.
    fn source_environment(&self) -> Option<qa_world::movement::types::MovementEnvironment>;
    /// Gravity multiplier.
    fn gravity_multiplier(&self) -> f64;
    /// Body bounds.
    fn bounds(&self) -> Bounds;
    /// View angles.
    fn view_angles(&self) -> Vec3;
    /// View height.
    fn view_height(&self) -> f64;
    /// Ground contact.
    fn ground(&self) -> TraceHit;
    /// Water level.
    fn water_level(&self) -> i32;
    /// Water type.
    fn water_type(&self) -> i32;
    /// Selected movement profile.
    fn profile(&self) -> MovementPredictionProfile;
    /// Standing bounds.
    fn standing_bounds(&self) -> Bounds;
    /// Character family.
    fn character(&self) -> GameFamily;
    /// World gravity.
    fn world_gravity(&self) -> f64;
}

/// Narrow Q3 source view behind [`create_simulation_prediction_host`].
pub trait SimulationPredictionSource {
    /// Actor at a source entity number.
    fn actor_at(&self, number: i32) -> Option<ActorId>;
    /// Source entity number of an actor.
    fn number_of(&self, actor: &ActorId) -> Option<i32>;
    /// Whether an actor's source model is inline.
    fn is_inline_model(&self, actor: &ActorId) -> bool;
    /// Copied source presentation state.
    fn source_state(&self) -> Q3SourcePresentationState;
}

/// Narrow simulation view behind [`create_simulation_prediction_host`].
pub trait SimulationPredictionSimulation {
    /// Admitted movement player.
    fn movement_player(&self, actor: &ActorId) -> Option<Rc<dyn SimulationPredictionPlayer>>;
    /// Q3 source runtime view.
    fn q3_source(&self) -> Option<Rc<dyn SimulationPredictionSource>>;
    /// Executable recipe.
    fn recipe(&self) -> qa_content::contract::ExecutableRecipe;
    /// Collision queries.
    fn scene(&self) -> PredictionSceneHandle;
    /// Posture source.
    fn postures(&self) -> PredictionPosturesHandle;
}

/// Entity mapping over a simulation source view.
struct SimulationEntities {
    source: Rc<dyn SimulationPredictionSource>,
}

impl PredictionSourceEntities for SimulationEntities {
    fn actor_at(&self, number: i32) -> Option<ActorId> {
        self.source.actor_at(number)
    }

    fn number_of(&self, actor: &ActorId) -> Option<i32> {
        self.source.number_of(actor)
    }
}

/// Presentation adapter with a source-capture entry point.
pub struct SimulationPredictionHost {
    /// Movement adapter.
    pub adapter: PresentationPredictionAdapter,
    player: Rc<dyn SimulationPredictionPlayer>,
    entities: Rc<SimulationEntities>,
}

impl SimulationPredictionHost {
    /// Capture a published source state into the adapter.
    pub fn capture_source(&mut self, published: &Q3SourcePresentationState) -> Result<(), PresentationPredictionError> {
        let snapshot = capture_published(&self.player, &self.entities, published)?;
        self.adapter.capture(snapshot)
    }
}

/// Capture a published source client into a prediction snapshot.
fn capture_published(
    player: &Rc<dyn SimulationPredictionPlayer>,
    entities: &Rc<SimulationEntities>,
    published: &Q3SourcePresentationState,
) -> Result<MovementPredictionSnapshot, PresentationPredictionError> {
    let actor = player.actor();
    let published_client = published
        .clients
        .iter()
        .find(|client| client.actor == *actor.id())
        .ok_or(PresentationPredictionError::NoSnapshot(published.time))?;
    // The published row is already a shared player state; stamp the
    // presentation time as the command clock the pool mirror lacks.
    let mut ps = published_client.state.clone();
    ps.command_time = published.time;
    let base = MovementPredictionSnapshot {
        sequence: player.last_sequence(),
        command_time_milliseconds: ps.command_time as f64,
        state: player.state(),
        arsenal: player.arsenal(),
        animation: player.animation(),
        environment: player
            .source_environment()
            .unwrap_or(qa_world::movement::types::MovementEnvironment {
                client_outputs: None,
                speed_multiplier: None,
                pose: None,
                health: ps.health() as f64,
                flight: false,
                haste: false,
                invulnerable: false,
                gravity_multiplier: player.gravity_multiplier(),
            }),
        bounds: player.bounds(),
        view_angles: player.view_angles(),
        view_height: player.view_height(),
        view_offset: vec3(0.0, 0.0, player.view_height() as f32),
        contact: Some(super::types::PredictionContact {
            ground: player.ground(),
            water_level: player.water_level(),
            water_type: player.water_type(),
        }),
        q3_arsenal: None,
    };
    read_prediction_source_state(&base, &ps, entities.as_ref())
        .map_err(|error| PresentationPredictionError::SourceState(error.to_string()))
}

/// Create a simulation-backed prediction host for a cgame seat.
pub fn create_simulation_prediction_host(
    simulation: &dyn SimulationPredictionSimulation,
    actor: &ActorId,
    seat: &SeatId,
) -> Result<SimulationPredictionHost, PresentationPredictionError> {
    let player = simulation.movement_player(actor).ok_or_else(|| {
        PresentationPredictionError::SourceState("Cgame prediction requires an admitted source player".to_string())
    })?;
    let source = simulation.q3_source().ok_or_else(|| {
        PresentationPredictionError::SourceState("Cgame prediction requires an admitted source player".to_string())
    })?;
    let entities = Rc::new(SimulationEntities { source: source.clone() });
    let mut profile = player.profile();
    match &mut profile {
        MovementPredictionProfile::Q1Netquake(profile) => {
            profile.parameters.gravity = player.world_gravity();
        }
        MovementPredictionProfile::Q1Quakeworld(profile) => {
            profile.parameters.gravity = player.world_gravity();
        }
        _ => {}
    }
    let movement = MovementPredictionOptions {
        movement_only: false,
        actor: player.actor(),
        seat: seat.clone(),
        recipe: simulation.recipe(),
        profile,
        standing_bounds: player.standing_bounds(),
        standing_view_height: if player.character() == GameFamily::Q3 {
            26.0
        } else {
            22.0
        },
        scene: simulation.scene(),
        is_brush: {
            let source = source.clone();
            Rc::new(move |hit: &TraceHit| match hit {
                TraceHit::World { .. } => true,
                TraceHit::Actor { actor } => source.is_inline_model(actor),
                TraceHit::None => false,
            })
        },
        postures: simulation.postures(),
    };
    let initial = capture_published(&player, &entities, &source.source_state())?;
    let adapter = create_presentation_movement_host(PresentationPredictionOptions {
        movement,
        initial,
        entities: entities.clone(),
    })?;
    Ok(SimulationPredictionHost {
        adapter,
        player,
        entities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use qa_core::identity::IdentityOwner;
    use qa_net::common::commands::CommandSource;

    fn test_owner() -> IdentityOwner {
        IdentityOwner::create("presentation-test").expect("owner")
    }

    #[test]
    fn q3_source_command_passthrough() {
        let owner = test_owner();
        let input = ActorCommand {
            actor: owner.actor(2, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 7,
            command: NetUserCommand::Q3 {
                server_time_milliseconds: 100.0,
                angle_words: [1.0, 2.0, 3.0],
                buttons: 5.0,
                weapon: 4.0,
                forward_move: 10.0,
                right_move: 20.0,
                up_move: 30.0,
            },
            arsenal: None,
        };
        let presented = presentation_source_command(&input, 100.0, 9);
        assert_eq!(presented.server_time, 100);
        assert_eq!(presented.buttons, 5);
        assert_eq!(presented.weapon, 4);
        assert_eq!(presented.forwardmove, 10);
    }

    #[test]
    fn q1_source_command_scales_moves() {
        let owner = test_owner();
        let input = ActorCommand {
            actor: owner.actor(2, 0),
            source: CommandSource::LocalSeat { seat: owner.seat(0) },
            sequence: 7,
            command: NetUserCommand::Q1Netquake {
                acknowledged_server_time_seconds: 0.0,
                view_angles: [0.0, 90.0, 0.0],
                forward_move: 320.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
            },
            arsenal: None,
        };
        let presented = presentation_source_command(&input, 50.0, 0);
        assert_eq!(presented.server_time, 50);
        assert_eq!(presented.forwardmove, 127);
        assert_eq!(presented.angles.y as i32, 16384);
    }
}
