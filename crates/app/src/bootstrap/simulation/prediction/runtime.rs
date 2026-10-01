//! Seat-local movement prediction with command replay.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/prediction/runtime.ts`
//! (`copyPredictionCommand`, `SelectedMovementPrediction`).

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::math::vec3;
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::types::{OrderedMovementEffect, TraceShape, UserCommand};

use super::super::arsenal::selected::MovementState;
use super::super::q3_commands::relative_movement_command;
use super::step::{copy_prediction_snapshot, predict_movement_command, PredictError};
use super::types::{
    MovementPredictionOptions, MovementPredictionResult, MovementPredictionSnapshot, MovementProbeOptions,
    PredictionCommand, PredictionStatus, PredictionStepOptions,
};

/// Deep copy of a user command.
pub fn copy_prediction_command(command: &UserCommand) -> UserCommand {
    *command
}

fn short_angle(word: i32) -> f64 {
    f64::from(word as i16) * 360.0 / 65536.0
}

/// Each seat rewinds its own movement, inventory, animation, and event state
/// together.
pub struct SelectedMovementPrediction {
    /// Prediction options. The actor and seat handles must be minted from one
    /// session authority; the registry enforces that at mint time.
    pub options: MovementPredictionOptions,
    rerelease_movement: Rc<RefCell<Q2RereleaseMovementContext>>,
    snapshot: MovementPredictionSnapshot,
    commands: Vec<PredictionCommand>,
    discarded_sequence: i64,
}

impl SelectedMovementPrediction {
    /// Create seat-local prediction over an authoritative snapshot.
    pub fn new(options: MovementPredictionOptions, initial: &MovementPredictionSnapshot) -> Result<Self, PredictError> {
        if state_dialect(&initial.state) != options.profile.dialect() {
            return Err(PredictError::InitialSnapshotMismatch);
        }
        Ok(SelectedMovementPrediction {
            options,
            rerelease_movement: Rc::new(RefCell::new(Q2RereleaseMovementContext::new())),
            snapshot: copy_prediction_snapshot(initial),
            commands: Vec::new(),
            discarded_sequence: -1,
        })
    }

    /// Receive an authoritative snapshot, dropping acknowledged commands.
    pub fn receive(&mut self, snapshot: &MovementPredictionSnapshot) -> Result<(), PredictError> {
        if state_dialect(&snapshot.state) != self.options.profile.dialect() {
            return Err(PredictError::SnapshotFamilyChanged);
        }
        self.snapshot = copy_prediction_snapshot(snapshot);
        while self
            .commands
            .first()
            .is_some_and(|command| command.sequence <= snapshot.sequence)
        {
            self.commands.remove(0);
        }
        Ok(())
    }

    /// Submit a predicted command.
    pub fn submit(&mut self, entry: &PredictionCommand) -> Result<(), PredictError> {
        if !entry.time_milliseconds.is_finite() {
            return Err(PredictError::InvalidCommandClock);
        }
        if entry.command.dialect() != self.options.profile.dialect() {
            return Err(PredictError::CommandDialectMismatch);
        }
        if entry.sequence <= self.snapshot.sequence
            || self
                .commands
                .last()
                .is_some_and(|previous| entry.sequence <= previous.sequence)
        {
            return Ok(());
        }
        self.commands.push(PredictionCommand {
            angle_space: entry.angle_space,
            sequence: entry.sequence,
            time_milliseconds: entry.time_milliseconds,
            command: copy_prediction_command(&entry.command),
            arsenal: entry.arsenal.clone(),
        });
        if self.commands.len() > 64 {
            let dropped = self.commands.remove(0);
            self.discarded_sequence = dropped.sequence;
        }
        Ok(())
    }

    /// Replay queued commands over the authoritative snapshot.
    pub fn replay(
        &mut self,
        mut observe: Option<impl FnMut(MovementPredictionSnapshot)>,
    ) -> Result<MovementPredictionResult, PredictError> {
        let mut player = copy_prediction_snapshot(&self.snapshot);
        let mut effects: Vec<OrderedMovementEffect> = Vec::new();
        if matches!(
            player.state,
            MovementState::Q2Classic(_) | MovementState::Q2Rerelease(_)
        ) && state_flags(&player.state) & 64 != 0
        {
            let latest = self.commands.last();
            let command = latest.map(|entry| relative_movement_command(entry, &player.state).command);
            match (&player.state, command) {
                (MovementState::Q2Classic(state), Some(UserCommand::Q2Classic(command))) => {
                    player.view_angles = vec3(
                        (short_angle(command.angle_shorts[0]) + short_angle(state.delta_angle_shorts[0])) as f32,
                        (short_angle(command.angle_shorts[1]) + short_angle(state.delta_angle_shorts[1])) as f32,
                        (short_angle(command.angle_shorts[2]) + short_angle(state.delta_angle_shorts[2])) as f32,
                    );
                }
                (MovementState::Q2Rerelease(state), Some(UserCommand::Q2Rerelease(command))) => {
                    player.view_angles = vec3(
                        command.angles.x + state.delta_angles.x,
                        command.angles.y + state.delta_angles.y,
                        command.angles.z + state.delta_angles.z,
                    );
                }
                _ => {}
            }
            return Ok(MovementPredictionResult {
                status: PredictionStatus::Disabled,
                player,
                effects,
            });
        }
        if self.discarded_sequence > player.sequence
            || self
                .commands
                .last()
                .map(|command| command.sequence)
                .unwrap_or(player.sequence)
                - player.sequence
                >= 63
        {
            return Ok(MovementPredictionResult {
                status: PredictionStatus::HistoryExhausted,
                player,
                effects,
            });
        }
        let probe = MovementProbeOptions::from(&self.options);
        let (fixed_milliseconds, no_footsteps) = match &self.options.profile {
            super::types::MovementPredictionProfile::Q3(profile) => (profile.fixed_milliseconds, profile.no_footsteps),
            _ => (None, false),
        };
        let mut first = true;
        for entry in self.commands.clone() {
            let output = predict_movement_command(
                &probe,
                &player,
                &entry,
                &PredictionStepOptions {
                    scene: self.options.scene.clone(),
                    rerelease_movement: self.rerelease_movement.clone(),
                    fixed_milliseconds,
                    no_footsteps,
                    gauntlet_hit: false,
                    trace_mask: None,
                    first_command: first,
                },
                TraceShape::Box(self.options.standing_bounds),
            )?;
            player = output.player;
            effects.extend(output.effects);
            first = false;
            if let Some(observe) = observe.as_mut() {
                observe(copy_prediction_snapshot(&player));
            }
        }
        Ok(MovementPredictionResult {
            status: if first {
                PredictionStatus::Unchanged
            } else {
                PredictionStatus::Predicted
            },
            player,
            effects,
        })
    }
}

fn state_dialect(state: &MovementState) -> qa_world::movement::types::MovementDialect {
    match state {
        MovementState::Q1Netquake(_) => qa_world::movement::types::MovementDialect::Q1Netquake,
        MovementState::Q1Quakeworld(_) => qa_world::movement::types::MovementDialect::Q1Quakeworld,
        MovementState::Q2Classic(_) => qa_world::movement::types::MovementDialect::Q2Classic,
        MovementState::Q2Rerelease(_) => qa_world::movement::types::MovementDialect::Q2Rerelease,
        MovementState::Q3(_) => qa_world::movement::types::MovementDialect::Q3,
    }
}

fn state_flags(state: &MovementState) -> i32 {
    match state {
        MovementState::Q2Classic(state) => state.flags,
        MovementState::Q2Rerelease(state) => state.flags,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_world::movement::q1::types::{QwMovementProfile, QwMovementState};
    use qa_world::movement::q2::types::Q2MovementState;
    use qa_world::movement::types::{
        ActorAnimationState, AnimationState, ArsenalState, QwUserCommand, TraceHit, WeaponState,
    };
    use qa_world::movement::Q1MovementParameters;

    use super::super::test_support::{empty_scene, standing_bounds, stub_postures, test_actor, test_recipe};
    use super::*;

    fn options() -> MovementPredictionOptions {
        let owner = qa_core::identity::IdentityOwner::create("prediction-runtime-test").unwrap();
        MovementPredictionOptions {
            movement_only: false,
            actor: test_actor(),
            seat: owner.seat(0),
            recipe: test_recipe(),
            profile: super::super::types::MovementPredictionProfile::Q1Quakeworld(QwMovementProfile {
                id: ProviderId::new("sim", "test"),
                clock: qa_core::time::ClockProfile::Q1Quakeworld {
                    maximum_command_milliseconds: 50.0,
                },
                numeric: qa_core::numeric::Q1_DONOR_PROFILE,
                parameters: Q1MovementParameters {
                    gravity: 800.0,
                    stop_speed: 100.0,
                    max_speed: 320.0,
                    spectator_max_speed: 500.0,
                    accelerate: 10.0,
                    air_accelerate: 1.0,
                    water_accelerate: 10.0,
                    friction: 4.0,
                    water_friction: 4.0,
                    entity_gravity: 1.0,
                },
            }),
            standing_bounds: standing_bounds(),
            standing_view_height: 22.0,
            scene: empty_scene(),
            is_brush: Rc::new(|_| false),
            postures: stub_postures(),
        }
    }

    fn snapshot() -> MovementPredictionSnapshot {
        MovementPredictionSnapshot {
            sequence: 0,
            command_time_milliseconds: 0.0,
            state: MovementState::Q1Quakeworld(QwMovementState {
                origin: vec3(0.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                old_buttons: 0,
                water_jump_time_seconds: 0.0,
                dead: false,
                spectator: 0,
                ground: TraceHit::None,
            }),
            arsenal: ArsenalState {
                provider: ProviderId::new("sim", "test"),
                active_weapon: None,
                state: WeaponState::Q1 {
                    frame: 0,
                    attack_finished_seconds: 0.0,
                    source_weapon: 0,
                },
                ammo: Vec::new(),
            },
            animation: ActorAnimationState {
                provider: ProviderId::new("sim", "test"),
                state: AnimationState::Q1 {
                    frame: 0,
                    next_frame_seconds: 0.0,
                },
            },
            environment: qa_world::movement::types::MovementEnvironment::default(),
            bounds: standing_bounds(),
            view_angles: vec3(0.0, 0.0, 0.0),
            view_height: 22.0,
            view_offset: vec3(0.0, 0.0, 22.0),
            contact: None,
            q3_arsenal: None,
        }
    }

    fn command(sequence: i64) -> PredictionCommand {
        PredictionCommand {
            angle_space: None,
            sequence,
            time_milliseconds: sequence as f64 * 50.0,
            command: UserCommand::Q1Quakeworld(QwUserCommand {
                milliseconds: 50,
                angles: vec3(0.0, 0.0, 0.0),
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0,
                impulse: 0,
            }),
            arsenal: None,
        }
    }

    #[test]
    fn replays_from_unchanged_to_predicted() {
        let initial = snapshot();
        let mut prediction = SelectedMovementPrediction::new(options(), &initial).unwrap();
        let idle = prediction.replay(None::<fn(MovementPredictionSnapshot)>).unwrap();
        assert_eq!(idle.status, PredictionStatus::Unchanged);
        prediction.submit(&command(1)).unwrap();
        prediction.submit(&command(2)).unwrap();
        let mut observed = 0;
        let replayed = prediction
            .replay(Some(|_: MovementPredictionSnapshot| {
                observed += 1;
            }))
            .unwrap();
        assert_eq!(replayed.status, PredictionStatus::Predicted);
        assert_eq!(replayed.player.sequence, 2);
        assert_eq!(observed, 2);
    }

    #[test]
    fn receive_drops_acknowledged_commands() {
        let initial = snapshot();
        let mut prediction = SelectedMovementPrediction::new(options(), &initial).unwrap();
        prediction.submit(&command(1)).unwrap();
        prediction.submit(&command(2)).unwrap();
        let mut acknowledged = snapshot();
        acknowledged.sequence = 1;
        acknowledged.command_time_milliseconds = 50.0;
        prediction.receive(&acknowledged).unwrap();
        let replayed = prediction.replay(None::<fn(MovementPredictionSnapshot)>).unwrap();
        assert_eq!(replayed.player.sequence, 2);
        assert_eq!(replayed.status, PredictionStatus::Predicted);
    }

    #[test]
    fn history_exhaustion_and_validation() {
        let initial = snapshot();
        let mut prediction = SelectedMovementPrediction::new(options(), &initial).unwrap();
        for sequence in 1..=65 {
            prediction.submit(&command(sequence)).unwrap();
        }
        let replayed = prediction.replay(None::<fn(MovementPredictionSnapshot)>).unwrap();
        assert_eq!(replayed.status, PredictionStatus::HistoryExhausted);

        let mut bad = command(100);
        bad.time_milliseconds = f64::NAN;
        assert_eq!(prediction.submit(&bad).unwrap_err(), PredictError::InvalidCommandClock);
    }

    #[test]
    fn frozen_q2_reports_disabled() {
        let mut initial = snapshot();
        initial.state = MovementState::Q2Classic(Q2MovementState {
            move_type: 0,
            origin_eighths: [0, 0, 0],
            velocity_eighths: [0, 0, 0],
            flags: 64,
            time_eight_milliseconds: 0,
            gravity: 800.0,
            delta_angle_shorts: [0, 0, 0],
        });
        let mut fixed = options();
        fixed.profile = super::super::types::MovementPredictionProfile::Q2Classic(
            qa_world::movement::q2::types::Q2MovementProfile {
                id: ProviderId::new("sim", "test"),
                clock: qa_core::time::ClockProfile::Q2Classic,
                numeric: qa_core::numeric::Q2_DONOR_PROFILE,
                strafejump_hack: false,
                air_accelerate: 0.0,
                snap_initial: false,
            },
        );
        let mut prediction = SelectedMovementPrediction::new(fixed, &initial).unwrap();
        let replayed = prediction.replay(None::<fn(MovementPredictionSnapshot)>).unwrap();
        assert_eq!(replayed.status, PredictionStatus::Disabled);
    }
}
