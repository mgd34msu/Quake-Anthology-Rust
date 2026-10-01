//! Multi-command prediction probes for navigation.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/prediction/sequence.ts`
//! (`predictMovementSequence`).

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::math::{vec3, Vec3};
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::types::{OrderedMovementEffect, TraceShape};

use super::super::arsenal::selected::MovementState;
use super::step::{copy_prediction_snapshot, predict_movement_command, PredictError};
use super::types::{MovementPredictionSnapshot, MovementProbeOptions, PredictionCommand, PredictionStepOptions};

/// One navigation probe: the final snapshot, effects, trajectory, and seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictedSequence {
    /// Final player snapshot.
    pub player: MovementPredictionSnapshot,
    /// Ordered effects.
    pub effects: Vec<OrderedMovementEffect>,
    /// Origin trajectory, including the start.
    pub trajectory: Vec<Vec3>,
    /// Simulated seconds.
    pub seconds: f64,
}

fn origin_of(state: &MovementState) -> Vec3 {
    match state {
        MovementState::Q1Netquake(state) => state.origin,
        MovementState::Q1Quakeworld(state) => state.origin,
        MovementState::Q2Classic(state) => vec3(
            state.origin_eighths[0] as f32 / 8.0,
            state.origin_eighths[1] as f32 / 8.0,
            state.origin_eighths[2] as f32 / 8.0,
        ),
        MovementState::Q2Rerelease(state) => state.origin,
        MovementState::Q3(state) => state.origin,
    }
}

/// Callers retain the returned state across accepted navigation edges; no
/// actor store is changed.
pub fn predict_movement_sequence(
    options: &MovementProbeOptions,
    initial: &MovementPredictionSnapshot,
    commands: &[PredictionCommand],
    shape: Option<TraceShape>,
) -> Result<PredictedSequence, PredictError> {
    let shape = shape.unwrap_or(TraceShape::Box(options.standing_bounds));
    let mut player = copy_prediction_snapshot(initial);
    let mut trajectory = vec![origin_of(&player.state)];
    let mut effects: Vec<OrderedMovementEffect> = Vec::new();
    let rerelease_movement = Rc::new(RefCell::new(Q2RereleaseMovementContext::new()));
    let (fixed_milliseconds, no_footsteps) = match &options.profile {
        super::types::MovementPredictionProfile::Q3(profile) => (profile.fixed_milliseconds, profile.no_footsteps),
        _ => (None, false),
    };
    let mut first = true;
    for command in commands {
        let result = predict_movement_command(
            options,
            &player,
            command,
            &PredictionStepOptions {
                scene: options.scene.clone(),
                rerelease_movement: rerelease_movement.clone(),
                fixed_milliseconds,
                no_footsteps,
                gauntlet_hit: false,
                trace_mask: None,
                first_command: first,
            },
            shape,
        )?;
        player = result.player;
        effects.extend(result.effects);
        trajectory.push(origin_of(&player.state));
        first = false;
    }
    Ok(PredictedSequence {
        seconds: (player.command_time_milliseconds - initial.command_time_milliseconds) / 1000.0,
        player,
        effects,
        trajectory,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;
    use qa_core::math::vec3;
    use qa_world::movement::q1::types::{QwMovementProfile, QwMovementState};
    use qa_world::movement::types::{
        ActorAnimationState, AnimationState, ArsenalState, QwUserCommand, TraceHit, UserCommand, WeaponState,
    };
    use qa_world::movement::Q1MovementParameters;

    use super::super::test_support::{empty_scene, standing_bounds, stub_postures, test_actor, test_recipe};
    use super::*;

    fn probe() -> MovementProbeOptions {
        MovementProbeOptions {
            movement_only: false,
            actor: test_actor(),
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

    fn initial() -> MovementPredictionSnapshot {
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

    #[test]
    fn probes_collect_trajectories() {
        let options = probe();
        let start = initial();
        let commands: Vec<PredictionCommand> = (1..=3)
            .map(|sequence| PredictionCommand {
                angle_space: None,
                sequence,
                time_milliseconds: sequence as f64 * 50.0,
                command: UserCommand::Q1Quakeworld(QwUserCommand {
                    milliseconds: 50,
                    angles: vec3(0.0, 0.0, 0.0),
                    forward_move: 320.0,
                    side_move: 0.0,
                    up_move: 0.0,
                    buttons: 0,
                    impulse: 0,
                }),
                arsenal: None,
            })
            .collect();
        let probed = predict_movement_sequence(&options, &start, &commands, None).unwrap();
        assert_eq!(probed.trajectory.len(), 4);
        assert_eq!(probed.trajectory[0], vec3(0.0, 0.0, 64.0));
        assert_eq!(probed.player.sequence, 3);
        assert_eq!(probed.seconds, 0.15);
    }

    #[test]
    fn empty_probes_keep_the_start() {
        let options = probe();
        let start = initial();
        let probed = predict_movement_sequence(&options, &start, &[], None).unwrap();
        assert_eq!(probed.trajectory, vec![vec3(0.0, 0.0, 64.0)]);
        assert_eq!(probed.seconds, 0.0);
        assert!(probed.effects.is_empty());
    }
}
