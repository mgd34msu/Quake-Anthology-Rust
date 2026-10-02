//! Native QuakeWorld client prediction continuation.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/prediction/qw-source-state.ts`
//! (`qwPredictionProfile`, `QwPredictionStatus`, `qwPredictionSnapshot`,
//! `QuakeWorldPrediction`).
//!
//! Native QW client continuation follows quake-1-re-ts qw/client/cl_pred.ts.

use std::collections::BTreeMap;

use qa_core::math::vec3;
use qa_net::common::commands::UserCommand as SelectedUserCommand;
use qa_net::q1_net::{QwMoveVariables, QwPlayerState};
use qa_world::movement::q1::types::{QwMovementProfile, QwMovementState};
use qa_world::movement::types::{QwUserCommand, UserCommand};
use qa_world::movement::Q1MovementParameters;

use super::super::arsenal::selected::MovementState;
use super::super::q3_commands::selected_to_movement;
use super::runtime::SelectedMovementPrediction;
use super::step::{copy_prediction_snapshot, PredictError};
use super::types::{
    MovementPredictionOptions, MovementPredictionProfile, MovementPredictionResult, MovementPredictionSnapshot,
};

/// Errors in native QuakeWorld prediction.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum QwPredictionError {
    /// Spectators are not admitted.
    #[error("The native QW graphical player slice does not admit spectators")]
    Spectator,
    /// Snapshot is not QuakeWorld.
    #[error("QW prediction requires a QW snapshot")]
    NotQwSnapshot,
    /// Profile is not Quake I movement.
    #[error("QW prediction requires Q1 movement parameters")]
    NotQ1Parameters,
    /// Initial state is not an admitted native player.
    #[error("QW prediction requires an admitted native player")]
    NotAdmitted,
    /// Replay changed movement family.
    #[error("QW replay changed movement family")]
    FamilyChanged,
    /// Wrapped prediction failure.
    #[error(transparent)]
    Predict(#[from] PredictError),
}

/// QuakeWorld movement profile over Q1 parameters and move variables.
pub fn qw_prediction_profile(
    base: &MovementPredictionProfile,
    variables: &QwMoveVariables,
) -> Result<QwMovementProfile, QwPredictionError> {
    let (id, numeric) = match base {
        MovementPredictionProfile::Q1Netquake(profile) => (profile.id.clone(), profile.numeric),
        MovementPredictionProfile::Q1Quakeworld(profile) => (profile.id.clone(), profile.numeric),
        _ => return Err(QwPredictionError::NotQ1Parameters),
    };
    Ok(QwMovementProfile {
        id,
        clock: qa_core::time::ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: 50.0,
        },
        numeric,
        parameters: Q1MovementParameters {
            gravity: f64::from(variables.gravity),
            stop_speed: f64::from(variables.stop_speed),
            max_speed: f64::from(variables.max_speed),
            spectator_max_speed: f64::from(variables.spectator_max_speed),
            accelerate: f64::from(variables.accelerate),
            air_accelerate: f64::from(variables.air_accelerate),
            water_accelerate: f64::from(variables.water_accelerate),
            friction: f64::from(variables.friction),
            water_friction: f64::from(variables.water_friction),
            entity_gravity: f64::from(variables.entity_gravity),
        },
    })
}

/// Native player status slice for prediction snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QwPredictionStatus {
    /// Health.
    pub health: i32,
    /// Spectator flag.
    pub spectator: i32,
}

/// Playerinfo has no oldbuttons, waterjumptime or ground: retain the matching
/// command's continuation.
pub fn qw_prediction_snapshot(
    base: &MovementPredictionSnapshot,
    player: &QwPlayerState,
    status: &QwPredictionStatus,
    acknowledged: Option<&MovementPredictionSnapshot>,
    sequence: i64,
    command_time_milliseconds: f64,
) -> Result<MovementPredictionSnapshot, QwPredictionError> {
    if status.spectator != 0 {
        return Err(QwPredictionError::Spectator);
    }
    let MovementState::Q1Quakeworld(base_state) = &base.state else {
        return Err(QwPredictionError::NotQwSnapshot);
    };
    let continuation = acknowledged
        .and_then(|snapshot| match &snapshot.state {
            MovementState::Q1Quakeworld(state) => Some(state),
            _ => None,
        })
        .unwrap_or(base_state);
    let view_height = if player.flags & 1024 != 0 {
        8.0
    } else if player.flags & 512 != 0 {
        -16.0
    } else {
        22.0
    };
    Ok(copy_prediction_snapshot(&MovementPredictionSnapshot {
        sequence,
        command_time_milliseconds,
        state: MovementState::Q1Quakeworld(QwMovementState {
            origin: vec3(
                player.origin[0] as f32,
                player.origin[1] as f32,
                player.origin[2] as f32,
            ),
            velocity: vec3(
                f32::from(player.velocity[0]),
                f32::from(player.velocity[1]),
                f32::from(player.velocity[2]),
            ),
            angles: base.view_angles,
            old_buttons: continuation.old_buttons,
            water_jump_time_seconds: continuation.water_jump_time_seconds,
            ground: continuation.ground.clone(),
            dead: status.health <= 0,
            spectator: 0,
        }),
        view_height,
        view_offset: vec3(0.0, 0.0, view_height as f32),
        environment: qa_world::movement::types::MovementEnvironment {
            health: f64::from(status.health),
            gravity_multiplier: 1.0,
            ..base.environment
        },
        contact: acknowledged
            .and_then(|snapshot| snapshot.contact.clone())
            .or_else(|| base.contact.clone()),
        ..copy_prediction_snapshot(base)
    }))
}

struct SentPrediction {
    command: QwUserCommand,
    time_milliseconds: f64,
    player: Option<MovementPredictionSnapshot>,
}

/// One seat and one server-world lifetime; collision and movement remain
/// shared engine services.
pub struct QuakeWorldPrediction {
    profile: QwMovementProfile,
    prediction: SelectedMovementPrediction,
    history: BTreeMap<i64, SentPrediction>,
    acknowledged_sequence: i64,
    last_sent: i64,
    view_height: f64,
    result: MovementPredictionResult,
}

impl QuakeWorldPrediction {
    /// Create QuakeWorld prediction over an admitted native player.
    pub fn new(
        options: MovementPredictionOptions,
        initial: &MovementPredictionSnapshot,
        variables: &QwMoveVariables,
    ) -> Result<Self, QwPredictionError> {
        if !matches!(
            options.profile,
            MovementPredictionProfile::Q1Netquake(_) | MovementPredictionProfile::Q1Quakeworld(_)
        ) {
            return Err(QwPredictionError::NotQ1Parameters);
        }
        let MovementState::Q1Quakeworld(state) = &initial.state else {
            return Err(QwPredictionError::NotAdmitted);
        };
        if state.spectator != 0 {
            return Err(QwPredictionError::NotAdmitted);
        }
        let profile = qw_prediction_profile(&options.profile, variables)?;
        let mut options = options;
        options.profile = MovementPredictionProfile::Q1Quakeworld(profile.clone());
        let mut prediction = SelectedMovementPrediction::new(options, initial)?;
        let acknowledged_sequence = initial.sequence;
        let result = prediction.replay(None::<fn(MovementPredictionSnapshot)>)?;
        Ok(QuakeWorldPrediction {
            profile,
            prediction,
            history: BTreeMap::new(),
            acknowledged_sequence,
            last_sent: acknowledged_sequence,
            view_height: initial.view_height,
            result,
        })
    }

    /// Record a sent command and re-predict.
    pub fn sent(&mut self, sequence: i64, command: &SelectedUserCommand, now: f64) -> Result<(), QwPredictionError> {
        if sequence <= self.last_sent {
            return Ok(());
        }
        let UserCommand::Q1Quakeworld(command) = selected_to_movement(command) else {
            return Err(PredictError::CommandDialectMismatch.into());
        };
        self.prediction.submit(&super::types::PredictionCommand {
            angle_space: None,
            sequence,
            time_milliseconds: now,
            command: UserCommand::Q1Quakeworld(command),
            arsenal: None,
        })?;
        self.last_sent = sequence;
        self.history.insert(
            sequence,
            SentPrediction {
                command,
                time_milliseconds: now,
                player: None,
            },
        );
        while self.history.len() > 64 {
            let first = *self.history.keys().next().unwrap();
            self.history.remove(&first);
        }
        self.predict()
    }

    /// Record a channel acknowledgement.
    pub fn acknowledged(&mut self, sequence: i64) {
        self.acknowledged_sequence = self.acknowledged_sequence.max(sequence);
    }

    /// Call only for a packet containing this player's playerinfo, after its
    /// channel acknowledgement.
    pub fn receive(
        &mut self,
        base: &MovementPredictionSnapshot,
        player: &QwPlayerState,
        variables: &QwMoveVariables,
        status: &QwPredictionStatus,
    ) -> Result<(), QwPredictionError> {
        let acknowledged = self.history.get(&self.acknowledged_sequence);
        let snapshot = qw_prediction_snapshot(
            base,
            player,
            status,
            acknowledged.and_then(|entry| entry.player.as_ref()),
            self.acknowledged_sequence,
            acknowledged
                .map(|entry| entry.time_milliseconds)
                .unwrap_or(base.command_time_milliseconds),
        )?;
        self.profile = qw_prediction_profile(
            &MovementPredictionProfile::Q1Quakeworld(self.profile.clone()),
            variables,
        )?;
        self.view_height = snapshot.view_height;
        self.prediction.options.profile = MovementPredictionProfile::Q1Quakeworld(self.profile.clone());
        self.prediction.receive(&snapshot)?;
        self.history
            .retain(|sequence, _| *sequence > self.acknowledged_sequence);
        self.predict()
    }

    fn predict(&mut self) -> Result<(), QwPredictionError> {
        let mut observed: Vec<MovementPredictionSnapshot> = Vec::new();
        let result = self.prediction.replay(Some(|player: MovementPredictionSnapshot| {
            observed.push(player);
        }))?;
        for player in observed {
            let command = self.history.get(&player.sequence).map(|entry| entry.command);
            if let Some(command) = command {
                let continued = self.client_continuation(&player, &command)?;
                if let Some(entry) = self.history.get_mut(&player.sequence) {
                    entry.player = Some(continued);
                }
            }
        }
        let command = self.history.get(&result.player.sequence).map(|entry| entry.command);
        self.result = match command {
            None => result,
            Some(command) => MovementPredictionResult {
                player: self.client_continuation(&result.player, &command)?,
                ..result
            },
        };
        Ok(())
    }

    fn client_continuation(
        &self,
        player: &MovementPredictionSnapshot,
        command: &QwUserCommand,
    ) -> Result<MovementPredictionSnapshot, QwPredictionError> {
        let MovementState::Q1Quakeworld(state) = &player.state else {
            return Err(QwPredictionError::FamilyChanged);
        };
        // CL_PredictUsercmd writes to.oldbuttons = pmove.cmd.buttons, unlike the server's PMove latch.
        let mut next = state.clone();
        next.old_buttons = command.buttons;
        Ok(MovementPredictionSnapshot {
            view_height: self.view_height,
            view_offset: vec3(0.0, 0.0, self.view_height as f32),
            state: MovementState::Q1Quakeworld(next),
            ..copy_prediction_snapshot(player)
        })
    }

    /// Latest replay result.
    pub fn replay(&self) -> MovementPredictionResult {
        MovementPredictionResult {
            player: copy_prediction_snapshot(&self.result.player),
            ..self.result.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_net::qw::QwUsercmd;
    use qa_world::movement::q1::types::{QwMovementProfile, QwMovementState};
    use qa_world::movement::types::{ActorAnimationState, AnimationState, ArsenalState, TraceHit, WeaponState};
    use qa_world::movement::Q1MovementParameters;

    use super::super::test_support::{empty_scene, standing_bounds, stub_postures, test_actor, test_recipe};
    use super::*;

    fn variables() -> QwMoveVariables {
        QwMoveVariables {
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
        }
    }

    fn options() -> MovementPredictionOptions {
        let owner = qa_core::identity::IdentityOwner::create("qw-prediction-test").unwrap();
        MovementPredictionOptions {
            movement_only: false,
            actor: test_actor(),
            seat: owner.seat(0),
            recipe: test_recipe(),
            profile: MovementPredictionProfile::Q1Quakeworld(QwMovementProfile {
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
                origin: vec3(0.0, 0.0, 64.0),
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

    fn player_state() -> QwPlayerState {
        QwPlayerState {
            number: 0,
            flags: 0,
            origin: [0.0, 0.0, 64.0],
            velocity: [0, 0, 0],
            model_index: 1,
            frame: 0,
            skin: 0,
            effects: 0,
            weapon_frame: 0,
            milliseconds: 50,
            command: QwUsercmd::default(),
        }
    }

    fn protocol_command() -> SelectedUserCommand {
        SelectedUserCommand::Q1Quakeworld {
            milliseconds: 50.0,
            angles: [0.0, 0.0, 0.0],
            forward_move: 320.0,
            side_move: 0.0,
            up_move: 0.0,
            buttons: 1.0,
            impulse: 0.0,
        }
    }

    #[test]
    fn profiles_adopt_move_variables() {
        let profile = qw_prediction_profile(
            &MovementPredictionProfile::Q1Netquake(qa_world::movement::q1::types::Q1MovementProfile {
                id: ProviderId::new("sim", "test"),
                clock: qa_core::time::ClockProfile::Q1Netquake {
                    minimum_frame_seconds: 0.001,
                    maximum_frame_seconds: 0.1,
                    fixed_frame_seconds: None,
                },
                numeric: qa_core::numeric::Q1_DONOR_PROFILE,
                edition: qa_world::movement::q1::types::Q1Edition::Classic,
                parameters: Q1MovementParameters {
                    gravity: 1.0,
                    stop_speed: 1.0,
                    max_speed: 1.0,
                    spectator_max_speed: 1.0,
                    accelerate: 1.0,
                    air_accelerate: 1.0,
                    water_accelerate: 1.0,
                    friction: 1.0,
                    water_friction: 1.0,
                    entity_gravity: 1.0,
                },
                edge_friction: 2.0,
                no_clip_angle_hack: false,
            }),
            &variables(),
        )
        .unwrap();
        assert_eq!(profile.parameters.gravity, 800.0);
        assert_eq!(profile.parameters.friction, 4.0);
    }

    #[test]
    fn snapshots_reject_spectators() {
        let base = snapshot();
        let status = QwPredictionStatus {
            health: 100,
            spectator: 1,
        };
        assert_eq!(
            qw_prediction_snapshot(&base, &player_state(), &status, None, 1, 50.0).unwrap_err(),
            QwPredictionError::Spectator
        );
    }

    #[test]
    fn send_receive_roundtrip() {
        let initial = snapshot();
        let mut prediction = QuakeWorldPrediction::new(options(), &initial, &variables()).unwrap();
        prediction.sent(1, &protocol_command(), 50.0).unwrap();
        prediction.sent(2, &protocol_command(), 100.0).unwrap();
        assert_eq!(prediction.replay().player.sequence, 2);
        prediction.acknowledged(1);
        let status = QwPredictionStatus {
            health: 100,
            spectator: 0,
        };
        prediction
            .receive(&initial, &player_state(), &variables(), &status)
            .unwrap();
        let replayed = prediction.replay();
        assert_eq!(replayed.player.sequence, 2);
        assert_eq!(replayed.player.view_height, 22.0);
    }
}
