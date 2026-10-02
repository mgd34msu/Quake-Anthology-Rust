//! Quake II remote view helpers.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/q2-remote-view.ts`
//! (`q2RemoteViewHeight`, `q2RemoteViewPosition`, `q2RemoteBodyBounds`,
//! `q2RemoteCommand`, `Q2RereleaseViewHeight`, `q2RereleaseViewContinuous`).
//! Player/command contracts reuse [`Q2Player`](qa_net::q2_adapters::Q2Player)
//! and [`Q2Command`](qa_net::q2_adapters::Q2Command); wire encoding reuses
//! [`from_q2_command`](qa_net::q2_adapters::from_q2_command), and pmove
//! constants reuse `qa-world` (`pm_type`, `kex_pm_type`, `pm_flags`).

use qa_core::math::{vec3, Bounds, Vec3};
use qa_net::q2::Usercmd;
use qa_net::q2_adapters::{Q2Command, Q2Player, Q2RereleasePlayerState, Q2RereleaseUserCommand};
use qa_world::movement::q2::{kex_pm_type, pm_flags, pm_type};
use thiserror::Error;

/// Remote view failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2RemoteViewError {
    /// Command and server movement editions differ.
    #[error("Q2 command and server movement editions differ")]
    EditionMismatch,
}

/// Remote camera sample (`q2RemoteViewPosition` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RemoteView {
    /// World-space eye origin.
    pub origin: Vec3,
    /// Smoothed view height.
    pub view_height: f32,
}

/// Eye height above the origin (`q2RemoteViewHeight`).
#[must_use]
pub fn q2_remote_view_height(player: &Q2Player) -> f32 {
    match player {
        Q2Player::Rerelease(state) => state.movement.view_height as f32,
        Q2Player::Classic(state) => state.view.view_offset.z as f32,
    }
}

/// Eye origin with the render offset applied (`q2RemoteViewPosition`).
///
/// Classic movement includes stance in `viewoffset`, so only the rerelease
/// path adds the vertical offset.
#[must_use]
pub fn q2_remote_view_position(player: &Q2Player, origin: Vec3, offset: Vec3, view_height: f32) -> Q2RemoteView {
    let z = origin.z
        + if matches!(player, Q2Player::Rerelease(_)) {
            offset.z
        } else {
            0.0
        };
    Q2RemoteView {
        origin: vec3(origin.x + offset.x, origin.y + offset.y, z),
        view_height,
    }
}

/// Solid body bounds for the current stance (`q2RemoteBodyBounds`).
#[must_use]
pub fn q2_remote_body_bounds(player: &Q2Player) -> Bounds {
    let (move_type, flags) = match player {
        Q2Player::Rerelease(state) => (i32::from(state.movement.move_type), state.movement.flags),
        Q2Player::Classic(state) => (i32::from(state.movement.move_type), state.movement.flags),
    };
    let (gib_type, dead_type) = match player {
        Q2Player::Rerelease(_) => (kex_pm_type::GIB, kex_pm_type::DEAD),
        Q2Player::Classic(_) => (pm_type::GIB, pm_type::DEAD),
    };
    let gib = move_type == gib_type;
    let dead = move_type == dead_type;
    let ducked = dead || (flags & pm_flags::DUCKED) != 0;
    Bounds {
        min: vec3(-16.0, -16.0, if gib { 0.0 } else { -24.0 }),
        max: vec3(
            16.0,
            16.0,
            if gib {
                16.0
            } else if ducked {
                4.0
            } else {
                32.0
            },
        ),
    }
}

/// Encode a command for the wire, subtracting source delta angles
/// (`q2RemoteCommand`).
pub fn q2_remote_command(command: &Q2Command, player: &Q2Player) -> Result<Usercmd, Q2RemoteViewError> {
    match (command, player) {
        (Q2Command::Rerelease(command), Q2Player::Rerelease(state)) => {
            let delta = &state.movement.delta_angles;
            // `Math.fround` rounds each difference to 32 bits.
            let biased = Q2RereleaseUserCommand {
                angles: qa_net::q2_adapters::Q2Vec3 {
                    x: f64::from((command.angles.x - delta.x) as f32),
                    y: f64::from((command.angles.y - delta.y) as f32),
                    z: f64::from((command.angles.z - delta.z) as f32),
                },
                ..command.clone()
            };
            Ok(qa_net::q2_adapters::from_q2_command(&Q2Command::Rerelease(biased)))
        }
        (Q2Command::Classic(command), Q2Player::Classic(state)) => {
            let mut wire = qa_net::q2_adapters::from_q2_command(&Q2Command::Classic(command.clone()));
            for (index, angle) in wire.angles.iter_mut().enumerate() {
                // `& 65535` on the difference is a wrapping 16-bit subtract.
                *angle = angle.wrapping_sub(state.movement.delta_angle_shorts[index]);
            }
            Ok(wire)
        }
        _ => Err(Q2RemoteViewError::EditionMismatch),
    }
}

/// Rerelease stance smoothing (`Q2RereleaseViewHeight`).
///
/// Reversals start from the previous target, not the sampled height; stance
/// smoothing survives the discontinuities that
/// [`q2_rerelease_view_continuous`] reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q2RereleaseViewHeight {
    state: Option<Q2ViewHeightState>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Q2ViewHeightState {
    previous: f64,
    current: f64,
    changed_at: i64,
}

impl Q2RereleaseViewHeight {
    /// Drop the smoothing state.
    pub fn reset(&mut self) {
        self.state = None;
    }

    /// Sample the smoothed height at `time_milliseconds`.
    pub fn sample(&mut self, height: f64, time_milliseconds: i64) -> f64 {
        let state = self.state.get_or_insert(Q2ViewHeightState {
            previous: height,
            current: height,
            changed_at: time_milliseconds,
        });
        if state.current != height {
            state.previous = state.current;
            state.current = height;
            state.changed_at = time_milliseconds;
        }
        let elapsed = (time_milliseconds - state.changed_at).clamp(0, 100) as f64;
        state.current + (state.previous - state.current) * (100.0 - elapsed) * 0.01
    }
}

/// Whether rerelease player lerp continues across frames
/// (`q2RereleaseViewContinuous`).
///
/// `check_player_lerp` duplicates state after discontinuities without
/// resetting stance smoothing.
#[must_use]
pub fn q2_rerelease_view_continuous(
    previous: &Q2RereleasePlayerState,
    current: &Q2RereleasePlayerState,
    previous_frame: i32,
    current_frame: i32,
    event: i32,
) -> bool {
    if current_frame != previous_frame + 1 || event == 6 || event == 7 {
        return false;
    }
    let before = &previous.movement.origin;
    let after = &current.movement.origin;
    let jump = (before.x - after.x)
        .abs()
        .max((before.y - after.y).abs())
        .max((before.z - after.z).abs());
    if jump > 256.0 {
        return false;
    }
    (previous.view.render_flags ^ current.view.render_flags) & 16 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::q2_adapters::{Q2PlayerState, Q2UserCommand, Q2Vec3};

    fn classic_player() -> Q2Player {
        let mut state = Q2PlayerState::default();
        state.view.view_offset = Q2Vec3 {
            x: 0.0,
            y: 0.0,
            z: 22.0,
        };
        Q2Player::Classic(state)
    }

    fn rerelease_player() -> Q2Player {
        let mut state = Q2RereleasePlayerState::default();
        state.movement.view_height = 22;
        Q2Player::Rerelease(state)
    }

    #[test]
    fn view_height_reads_view_offset_or_pmove() {
        assert_eq!(q2_remote_view_height(&classic_player()), 22.0);
        assert_eq!(q2_remote_view_height(&rerelease_player()), 22.0);
    }

    #[test]
    fn view_position_applies_vertical_offset_for_rerelease_only() {
        let origin = vec3(1.0, 2.0, 4.0);
        let offset = vec3(8.0, 16.0, 32.0);
        let classic = q2_remote_view_position(&classic_player(), origin, offset, 22.0);
        assert_eq!(classic.origin, vec3(9.0, 18.0, 4.0));
        assert_eq!(classic.view_height, 22.0);
        let rerelease = q2_remote_view_position(&rerelease_player(), origin, offset, 22.0);
        assert_eq!(rerelease.origin, vec3(9.0, 18.0, 36.0));
    }

    #[test]
    fn body_bounds_follow_stance_and_edition() {
        assert_eq!(q2_remote_body_bounds(&classic_player()).max.z, 32.0);
        let mut dead = Q2PlayerState::default();
        dead.movement.move_type = pm_type::DEAD as u8;
        assert_eq!(q2_remote_body_bounds(&Q2Player::Classic(dead)).max.z, 4.0);
        let mut ducked = Q2PlayerState::default();
        ducked.movement.flags = pm_flags::DUCKED;
        assert_eq!(q2_remote_body_bounds(&Q2Player::Classic(ducked)).max.z, 4.0);
        let mut gib = Q2RereleasePlayerState::default();
        gib.movement.move_type = kex_pm_type::GIB as u8;
        let bounds = q2_remote_body_bounds(&Q2Player::Rerelease(gib));
        assert_eq!((bounds.min.z, bounds.max.z), (0.0, 16.0));
    }

    #[test]
    fn remote_command_subtracts_delta_angles() {
        let command = Q2UserCommand {
            angle_shorts: [1000, -2000, 3000],
            ..Q2UserCommand::default()
        };
        let mut player = Q2PlayerState::default();
        player.movement.delta_angle_shorts = [100, 200, -300];
        let wire = q2_remote_command(&Q2Command::Classic(command), &Q2Player::Classic(player)).expect("classic");
        assert_eq!(wire.angles, [900, -2200_i16, 3300]);
        let rcommand = Q2RereleaseUserCommand {
            angles: Q2Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0,
            },
            ..Q2RereleaseUserCommand::default()
        };
        let mut rplayer = Q2RereleasePlayerState::default();
        rplayer.movement.delta_angles = Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        let rwire =
            q2_remote_command(&Q2Command::Rerelease(rcommand), &Q2Player::Rerelease(rplayer)).expect("rerelease");
        let expected = qa_net::q2::angle_to_short(9.0) as i16;
        assert_eq!(rwire.angles[0], expected);
    }

    #[test]
    fn remote_command_rejects_mixed_editions() {
        let error =
            q2_remote_command(&Q2Command::Classic(Q2UserCommand::default()), &rerelease_player()).expect_err("mixed");
        assert_eq!(error, Q2RemoteViewError::EditionMismatch);
    }

    #[test]
    fn view_height_smooths_reversals_from_previous_target() {
        let mut tracker = Q2RereleaseViewHeight::default();
        assert_eq!(tracker.sample(22.0, 1000), 22.0);
        // The reversal starts from the previous target, then blends over 100 ms.
        assert_eq!(tracker.sample(0.0, 1050), 22.0);
        let mid = tracker.sample(0.0, 1100);
        assert!((mid - 11.0).abs() < 1e-9, "midpoint blend, got {mid}");
        tracker.reset();
        assert_eq!(tracker.sample(5.0, 2000), 5.0);
    }

    #[test]
    fn view_continuity_matches_source_lerp_rules() {
        let mut before = Q2RereleasePlayerState::default();
        let mut after = Q2RereleasePlayerState::default();
        after.movement.origin = Q2Vec3 {
            x: 10.0,
            y: 0.0,
            z: 0.0,
        };
        assert!(q2_rerelease_view_continuous(&before, &after, 7, 8, 0));
        assert!(!q2_rerelease_view_continuous(&before, &after, 7, 9, 0));
        assert!(!q2_rerelease_view_continuous(&before, &after, 7, 8, 6));
        after.view.render_flags = 16;
        before.view.render_flags = 0;
        assert!(!q2_rerelease_view_continuous(&before, &after, 7, 8, 0));
    }
}
