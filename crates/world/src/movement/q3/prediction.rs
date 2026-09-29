//! Quake III command replay and correction (`cg_predict.c`).
//!
//! Donor provenance: `src/movement/q3/prediction.ts` (from id Software
//! `code/cgame/cg_predict.c`).

use qa_core::math::{add3, length3, scale3, sub3, vec3, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;

use super::super::types::{
    ActorAnimationState, ArsenalState, MovementError, MovementExecution, OrderedMovementEffect, Q3UserCommand, TraceHit,
};
use super::constants::{move_flags, move_type};
use super::provider::Q3MovementProvider;
use super::types::{Q3MovementInput, Q3MovementServices, Q3MovementState};

/// Current command number plus ring reads.
pub trait Q3CommandSource {
    /// Current command number.
    fn current_number(&self) -> i32;
    /// Read a command by number.
    fn read(&self, number: i32) -> Result<Option<Q3UserCommand>, MovementError>;
}

fn empty_command() -> Q3UserCommand {
    Q3UserCommand {
        server_time_milliseconds: 0,
        angle_words: [0, 0, 0],
        buttons: 0,
        weapon: 0,
        forward_move: 0,
        right_move: 0,
        up_move: 0,
    }
}

/// `CL_GetUserCmd`'s zeroed `CMD_BACKUP` ring survives `map_restart`.
#[derive(Debug, Clone)]
pub struct Q3CommandHistory {
    commands: [Q3UserCommand; 64],
    number: i32,
}

impl Q3CommandHistory {
    /// Create an empty history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            commands: [empty_command(); 64],
            number: 0,
        }
    }

    /// Append a command, returning its number.
    pub fn append(&mut self, command: Q3UserCommand) -> i32 {
        self.number = self.number.wrapping_add(1);
        self.commands[(self.number & 63) as usize] = command;
        self.number
    }
}

impl Default for Q3CommandHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl Q3CommandSource for Q3CommandHistory {
    fn current_number(&self) -> i32 {
        self.number
    }

    fn read(&self, number: i32) -> Result<Option<Q3UserCommand>, MovementError> {
        if number > self.number {
            return Err(MovementError::Contract("CL_GetUserCmd read past newest command"));
        }
        if number <= self.number.wrapping_sub(64) {
            return Ok(None);
        }
        Ok(Some(self.commands[(number & 63) as usize]))
    }
}

/// Movement, weapon/ammo and animation rewind together; the host restores
/// other owner state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PredictedActor {
    /// Movement state.
    pub movement: Q3MovementState,
    /// Arsenal snapshot.
    pub arsenal: ArsenalState,
    /// Animation snapshot.
    pub animation: ActorAnimationState,
}

/// Prediction snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PredictionSnapshot {
    /// Server time in milliseconds.
    pub server_time_milliseconds: i32,
    /// Predicted actor.
    pub actor: Q3PredictedActor,
}

/// Prediction settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3PredictionSettings {
    /// Demo playback.
    pub demo_playback: bool,
    /// Prediction disabled.
    pub no_predict: bool,
    /// Synchronous clients.
    pub synchronous_clients: bool,
    /// Fixed timestep.
    pub fixed: bool,
    /// Movement step in milliseconds.
    pub movement_milliseconds: i32,
    /// Error-decay integer flag.
    pub error_decay_integer: i32,
    /// Error-decay value.
    pub error_decay_value: f32,
    /// Show-miss level.
    pub show_miss: i32,
}

/// Prediction frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PredictionFrame {
    /// Frame time in milliseconds.
    pub time_milliseconds: i32,
    /// Previous frame time in milliseconds.
    pub previous_time_milliseconds: i32,
    /// Current snapshot.
    pub snapshot: Q3PredictionSnapshot,
    /// Next snapshot.
    pub next_snapshot: Option<Q3PredictionSnapshot>,
    /// Next frame teleports.
    pub next_frame_teleport: bool,
    /// This frame teleports.
    pub this_frame_teleport: bool,
    /// Health.
    pub health: f64,
}

/// Trigger-touch outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PredictionTriggers {
    /// Predicted actor.
    pub actor: Q3PredictedActor,
    /// Hyperspace flag.
    pub hyperspace: bool,
    /// Emitted effects.
    pub effects: Vec<OrderedMovementEffect>,
}

/// Prediction output.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3PredictionOutput {
    /// Predicted through new commands.
    Predicted(Q3PredictionSuccess),
    /// Interpolated between snapshots.
    Interpolated(Q3PredictionSuccess),
    /// No new commands.
    Unchanged(Q3PredictionSuccess),
    /// Command backup exceeded.
    CommandBackupExceeded(Q3PredictionSuccess),
    /// Actor removed mid-prediction.
    ActorRemoved {
        /// Emitted effects.
        effects: Vec<OrderedMovementEffect>,
        /// Consumed teleport flag.
        consumed_teleport: bool,
    },
}

/// Successful prediction output fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3PredictionSuccess {
    /// Predicted actor.
    pub actor: Q3PredictedActor,
    /// Hyperspace flag.
    pub hyperspace: bool,
    /// Emitted effects.
    pub effects: Vec<OrderedMovementEffect>,
    /// Consumed teleport flag.
    pub consumed_teleport: bool,
}

/// Prediction host: commands, movement, snapshots, triggers, presentation.
pub trait Q3PredictionHost {
    /// Command source.
    fn commands(&self) -> &dyn Q3CommandSource;
    /// Movement provider.
    fn movement(&self) -> &Q3MovementProvider;
    /// Current settings.
    fn settings(&self) -> Q3PredictionSettings;
    /// Set the movement step.
    fn set_movement_milliseconds(&mut self, value: i32);
    /// Restore selected providers' holdables, game words and other
    /// prediction-owned state.
    fn restore_snapshot(&mut self, snapshot: &Q3PredictionSnapshot);
    /// Build movement input for a predicted command.
    fn movement_input(
        &mut self,
        actor: &Q3PredictedActor,
        command: &Q3UserCommand,
        command_number: i32,
        physics_time_milliseconds: i32,
    ) -> Q3MovementInput;
    /// Movement services for a physics time.
    fn movement_services(&mut self, physics_time_milliseconds: i32) -> &mut dyn Q3MovementServices;
    /// Source item admission, teleport and jump-pad prediction after Pmove.
    fn touch_triggers(
        &mut self,
        actor: &Q3PredictedActor,
        bounds: &Bounds,
        physics_time_milliseconds: i32,
    ) -> Q3PredictionTriggers;
    /// Adjust for mover interpolation.
    fn adjust_for_mover(
        &mut self,
        origin: Vec3,
        ground: &TraceHit,
        from_milliseconds: i32,
        to_milliseconds: i32,
    ) -> Vec3;
    /// Transition between predictions.
    fn transition(
        &mut self,
        current: &Q3PredictedActor,
        previous: &Q3PredictedActor,
        effects: &[OrderedMovementEffect],
    );
    /// Warn.
    fn warn(&mut self, message: &str);
}

fn interpolate_vector(a: Vec3, b: Vec3, fraction: f32) -> Vec3 {
    vec3(
        a.x + (fraction * (b.x - a.x)),
        a.y + (fraction * (b.y - a.y)),
        a.z + (fraction * (b.z - a.z)),
    )
}

fn lerp_angle(from: f32, to: f32, fraction: f32) -> f32 {
    let mut to = to;
    if to - from > 180.0 {
        to -= 360.0;
    }
    if to - from < -180.0 {
        to += 360.0;
    }
    from + (fraction * (to - from))
}

/// Update predicted view angles from the newest command.
#[must_use]
pub fn update_q3_prediction_view(state: &Q3MovementState, health: f64, command: &Q3UserCommand) -> Q3MovementState {
    if state.movement_type == move_type::INTERMISSION
        || state.movement_type == move_type::SPINTERMISSION
        || (state.movement_type != move_type::SPECTATOR && health <= 0.0)
    {
        return state.clone();
    }
    let mut delta_pitch = state.delta_angle_words[0];
    let mut pitch = (command.angle_words[0].wrapping_add(delta_pitch) << 16) >> 16;
    if pitch > 16000 {
        delta_pitch = 16000_i32.wrapping_sub(command.angle_words[0]);
        pitch = 16000;
    } else if pitch < -16000 {
        delta_pitch = (-16000_i32).wrapping_sub(command.angle_words[0]);
        pitch = -16000;
    }
    let yaw = (command.angle_words[1].wrapping_add(state.delta_angle_words[1]) << 16) >> 16;
    let roll = (command.angle_words[2].wrapping_add(state.delta_angle_words[2]) << 16) >> 16;
    // Donor multiplies in float64; round once into the f32 view angles.
    let scale = 360.0 / 65536.0;
    Q3MovementState {
        delta_angle_words: [delta_pitch, state.delta_angle_words[1], state.delta_angle_words[2]],
        view_angles: vec3(
            (f64::from(pitch) * scale) as f32,
            (f64::from(yaw) * scale) as f32,
            (f64::from(roll) * scale) as f32,
        ),
        ..state.clone()
    }
}

/// Prediction runtime: replay, correction, and teleport handling.
pub struct Q3PredictionRuntime<H> {
    host: H,
    predicted: Option<Q3PredictedActor>,
    command: Q3UserCommand,
    error: Vec3,
    error_time: i32,
}

impl<H: Q3PredictionHost> Q3PredictionRuntime<H> {
    /// Build a runtime over a host.
    pub fn new(host: H) -> Self {
        Self {
            host,
            predicted: None,
            command: empty_command(),
            error: vec3(0.0, 0.0, 0.0),
            error_time: 0,
        }
    }

    /// Predicted actor.
    #[must_use]
    pub fn actor(&self) -> Option<&Q3PredictedActor> {
        self.predicted.as_ref()
    }

    /// Predicted error.
    #[must_use]
    pub fn predicted_error(&self) -> Vec3 {
        self.error
    }

    /// Predicted error time.
    #[must_use]
    pub fn predicted_error_time(&self) -> i32 {
        self.error_time
    }

    /// Host access.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Mutable host access.
    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }

    fn required_command(&self, number: i32) -> Result<Q3UserCommand, MovementError> {
        match self.host.commands().read(number)? {
            Some(command) => Ok(command),
            None => Err(MovementError::Contract(
                "Prediction command source violated its CMD_BACKUP window",
            )),
        }
    }

    fn interpolate(&self, frame: &Q3PredictionFrame, grab_angles: bool) -> Result<Q3PredictedActor, MovementError> {
        let previous = &frame.snapshot;
        let mut movement = previous.actor.movement.clone();
        if grab_angles {
            movement = update_q3_prediction_view(
                &movement,
                frame.health,
                &self.required_command(self.host.commands().current_number())?,
            );
        }
        let next = frame.next_snapshot.as_ref();
        if frame.next_frame_teleport
            || next.is_none()
            || next.is_some_and(|next| next.server_time_milliseconds <= previous.server_time_milliseconds)
        {
            return Ok(Q3PredictedActor {
                movement,
                arsenal: previous.actor.arsenal.clone(),
                animation: previous.actor.animation.clone(),
            });
        }
        let next = next.expect("snapshot checked");
        let fraction = ((frame.time_milliseconds - previous.server_time_milliseconds) as f32)
            / ((next.server_time_milliseconds - previous.server_time_milliseconds) as f32);
        let a = &previous.actor.movement;
        let b = &next.actor.movement;
        let cycle = if b.bob_cycle < a.bob_cycle {
            b.bob_cycle.wrapping_add(256)
        } else {
            b.bob_cycle
        };
        Ok(Q3PredictedActor {
            arsenal: previous.actor.arsenal.clone(),
            animation: previous.actor.animation.clone(),
            movement: Q3MovementState {
                bob_cycle: qvm_float_to_int(a.bob_cycle as f32 + (fraction * (cycle - a.bob_cycle) as f32)),
                origin: interpolate_vector(a.origin, b.origin, fraction),
                velocity: interpolate_vector(a.velocity, b.velocity, fraction),
                view_angles: if grab_angles {
                    movement.view_angles
                } else {
                    vec3(
                        lerp_angle(a.view_angles.x, b.view_angles.x, fraction),
                        lerp_angle(a.view_angles.y, b.view_angles.y, fraction),
                        lerp_angle(a.view_angles.z, b.view_angles.z, fraction),
                    )
                },
                ..movement
            },
        })
    }

    /// Run one prediction frame.
    pub fn predict(&mut self, frame: &Q3PredictionFrame) -> Result<Q3PredictionOutput, MovementError> {
        let mut settings = self.host.settings();
        let snapshot = frame.snapshot.clone();
        if self.predicted.is_none() {
            self.predicted = Some(snapshot.actor.clone());
        }
        if settings.demo_playback
            || snapshot.actor.movement.movement_flags & move_flags::FOLLOW != 0
            || settings.no_predict
            || settings.synchronous_clients
        {
            let grab_angles =
                !settings.demo_playback && snapshot.actor.movement.movement_flags & move_flags::FOLLOW == 0;
            let actor = self.interpolate(frame, grab_angles)?;
            self.predicted = Some(actor.clone());
            return Ok(Q3PredictionOutput::Interpolated(Q3PredictionSuccess {
                actor,
                hyperspace: false,
                effects: Vec::new(),
                consumed_teleport: false,
            }));
        }
        let old = self.predicted.clone().expect("predicted set");
        let current = self.host.commands().current_number();
        let oldest = self.required_command(current.wrapping_sub(63))?;
        if oldest.server_time_milliseconds > snapshot.actor.movement.command_time_milliseconds
            && oldest.server_time_milliseconds < frame.time_milliseconds
        {
            if settings.show_miss != 0 {
                self.host.warn("exceeded PACKET_BACKUP on commands\n");
            }
            return Ok(Q3PredictionOutput::CommandBackupExceeded(Q3PredictionSuccess {
                actor: old,
                hyperspace: false,
                effects: Vec::new(),
                consumed_teleport: false,
            }));
        }
        let latest = self.required_command(current)?;
        let selected = if frame.next_snapshot.is_some() && !frame.next_frame_teleport && !frame.this_frame_teleport {
            frame.next_snapshot.clone().expect("snapshot checked")
        } else {
            snapshot.clone()
        };
        self.host.restore_snapshot(&selected);
        self.predicted = Some(selected.actor.clone());
        let physics_time = selected.server_time_milliseconds;
        if settings.movement_milliseconds < 8 {
            self.host.set_movement_milliseconds(8);
        } else if settings.movement_milliseconds > 33 {
            self.host.set_movement_milliseconds(33);
        }
        settings = self.host.settings();
        let mut effects: Vec<OrderedMovementEffect> = Vec::new();
        let mut moved = false;
        let mut hyperspace = false;
        let mut consumed_teleport = false;
        let mut number = current.wrapping_sub(63);
        loop {
            if let Some(command) = self.host.commands().read(number)? {
                self.command = command;
            }
            if settings.fixed {
                let predicted = self.predicted.clone().expect("predicted set");
                self.predicted = Some(Q3PredictedActor {
                    movement: update_q3_prediction_view(&predicted.movement, frame.health, &self.command),
                    ..predicted
                });
            }
            let predicted = self.predicted.clone().expect("predicted set");
            let fresh = self.command.server_time_milliseconds > predicted.movement.command_time_milliseconds
                && self.command.server_time_milliseconds <= latest.server_time_milliseconds;
            if fresh {
                if predicted.movement.command_time_milliseconds == old.movement.command_time_milliseconds {
                    if frame.this_frame_teleport && !consumed_teleport {
                        self.error = vec3(0.0, 0.0, 0.0);
                        consumed_teleport = true;
                        if settings.show_miss != 0 {
                            self.host.warn("PredictionTeleport\n");
                        }
                    } else {
                        let adjusted = self.host.adjust_for_mover(
                            predicted.movement.origin,
                            &predicted.movement.ground,
                            physics_time,
                            frame.previous_time_milliseconds,
                        );
                        let delta = sub3(old.movement.origin, adjusted);
                        let length = length3(delta);
                        if length > 0.1 {
                            if settings.show_miss != 0 {
                                self.host.warn(&format!("Prediction miss: {length:.6}\n"));
                            }
                            if settings.error_decay_integer != 0 {
                                let elapsed = frame.time_milliseconds - self.error_time;
                                let mut fraction =
                                    (settings.error_decay_value - elapsed as f32) / settings.error_decay_value;
                                if fraction < 0.0 {
                                    fraction = 0.0;
                                }
                                self.error = scale3(self.error, fraction);
                            } else {
                                self.error = vec3(0.0, 0.0, 0.0);
                            }
                            self.error = add3(delta, self.error);
                            self.error_time = frame.previous_time_milliseconds;
                        }
                    }
                }
                if settings.fixed {
                    let step = settings.movement_milliseconds;
                    self.command.server_time_milliseconds =
                        ((self.command.server_time_milliseconds + step - 1) / step) * step;
                }
                let predicted = self.predicted.clone().expect("predicted set");
                let mut input = self
                    .host
                    .movement_input(&predicted, &self.command, number, physics_time);
                input.state = predicted.movement.clone();
                input.fields.arsenal = predicted.arsenal.clone();
                input.fields.animation = predicted.animation.clone();
                input.command = self.command;
                input.fields.command_sequence = number;
                input.fields.execution = MovementExecution::Prediction;
                input.profile.fixed_milliseconds = if settings.fixed {
                    Some(settings.movement_milliseconds)
                } else {
                    None
                };
                // Clone the provider so the services borrow below does not
                // alias the host borrow; the provider is configuration-only.
                let provider = self.host.movement().clone();
                let services = self.host.movement_services(physics_time);
                let output = provider.move_step(input, services)?;
                let (state, arsenal, animation, bounds) = match output {
                    super::super::types::MovementOutcome::Active { fields, state } => {
                        effects.extend(fields.effects.clone());
                        (state, fields.arsenal.clone(), fields.animation.clone(), fields.bounds)
                    }
                    super::super::types::MovementOutcome::ActorRemoved { effects: removed, .. } => {
                        effects.extend(removed);
                        return Ok(Q3PredictionOutput::ActorRemoved {
                            effects,
                            consumed_teleport,
                        });
                    }
                };
                self.predicted = Some(Q3PredictedActor {
                    movement: state,
                    arsenal,
                    animation,
                });
                moved = true;
                let predicted = self.predicted.clone().expect("predicted set");
                let triggers = self.host.touch_triggers(&predicted, &bounds, physics_time);
                self.predicted = Some(triggers.actor.clone());
                hyperspace |= triggers.hyperspace;
                effects.extend(triggers.effects);
            }
            if number == current {
                break;
            }
            number = number.wrapping_add(1);
        }
        if !moved {
            return Ok(Q3PredictionOutput::Unchanged(Q3PredictionSuccess {
                actor: self.predicted.clone().expect("predicted set"),
                hyperspace,
                effects,
                consumed_teleport,
            }));
        }
        let predicted = self.predicted.clone().expect("predicted set");
        let origin = self.host.adjust_for_mover(
            predicted.movement.origin,
            &predicted.movement.ground,
            physics_time,
            frame.time_milliseconds,
        );
        self.predicted = Some(Q3PredictedActor {
            movement: Q3MovementState {
                origin,
                ..predicted.movement
            },
            ..predicted
        });
        let predicted = self.predicted.clone().expect("predicted set");
        if settings.show_miss != 0
            && predicted.movement.predictable_event_sequence > old.movement.predictable_event_sequence.wrapping_add(2)
        {
            self.host.warn("WARNING: dropped event\n");
        }
        self.host.transition(&predicted, &old, &effects);
        Ok(Q3PredictionOutput::Predicted(Q3PredictionSuccess {
            actor: predicted,
            hyperspace,
            effects,
            consumed_teleport,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};

    fn owner() -> IdentityOwner {
        IdentityOwner::create("q3-pred").unwrap()
    }

    fn movement_state() -> Q3MovementState {
        Q3MovementState {
            command_time_milliseconds: 0,
            movement_type: move_type::NORMAL,
            bob_cycle: 0,
            movement_flags: 0,
            movement_time_milliseconds: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            gravity: 800.0,
            speed: 320.0,
            delta_angle_words: [0, 0, 0],
            movement_direction: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            flags: 0,
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 26.0,
            ground: TraceHit::None,
            predictable_event_sequence: 0,
            jump_pad: None,
            movement_frame: 0,
            jump_pad_frame: 0,
        }
    }

    #[test]
    fn history_rings_and_bounds_reads() {
        let mut history = Q3CommandHistory::new();
        let mut command = empty_command();
        command.server_time_milliseconds = 100;
        let number = history.append(command);
        assert_eq!(number, 1);
        assert_eq!(history.current_number(), 1);
        let read = history.read(1).unwrap().unwrap();
        assert_eq!(read.server_time_milliseconds, 100);
        assert!(history.read(1 - 64).unwrap().is_none());
        assert!(history.read(2).is_err());
    }

    #[test]
    fn prediction_view_updates_live_players() {
        let _ = owner();
        let state = movement_state();
        let mut command = empty_command();
        command.angle_words = [0, 8192, 0];
        let out = update_q3_prediction_view(&state, 100.0, &command);
        assert!((out.view_angles.y - 45.0).abs() < 1e-4);
        let dead = update_q3_prediction_view(&state, 0.0, &command);
        assert_eq!(dead, state);
        let intermission = Q3MovementState {
            movement_type: move_type::INTERMISSION,
            ..state.clone()
        };
        assert_eq!(update_q3_prediction_view(&intermission, 100.0, &command), intermission);
    }

    #[test]
    fn prediction_view_clamps_pitch() {
        let state = movement_state();
        let mut command = empty_command();
        command.angle_words = [20000, 0, 0];
        let out = update_q3_prediction_view(&state, 100.0, &command);
        assert_eq!(out.delta_angle_words[0], 16000 - 20000);
        assert!((out.view_angles.x - (16000.0 * 360.0 / 65536.0) as f32).abs() < 1e-3);
    }

    #[test]
    fn interpolation_helpers_match_donor() {
        assert_eq!(
            interpolate_vector(vec3(0.0, 0.0, 0.0), vec3(10.0, 0.0, 0.0), 0.5).x,
            5.0
        );
        assert_eq!(lerp_angle(0.0, 90.0, 0.5), 45.0);
        assert_eq!(lerp_angle(170.0, -170.0, 0.5), 180.0);
    }

    #[test]
    fn bob_cycle_wraps_through_interpolation() {
        let _ = ProviderId::new("q3", "pred");
        let cycle = 250_i32;
        let next = 5_i32;
        let wrapped = if next < cycle { next.wrapping_add(256) } else { next };
        let blended = qvm_float_to_int(cycle as f32 + (0.5 * (wrapped - cycle) as f32));
        assert_eq!(blended, 255);
    }
}
