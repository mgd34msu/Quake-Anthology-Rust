//! Native mod camera projection over the public guest player state.
//!
//! Port of donor `src/app/bootstrap/simulation/native-mod-camera.ts`
//! (`nativeModCamera`, `resolveNativeModCamera`).

use qa_core::math::Vec3;
use qa_net::q2_adapters::{Q2Player, Q2Vec3};

use super::classic_guest_player::classic_guest_player_view;
use super::rerelease_guest_player::rerelease_guest_player_view;
use super::types::PlayerView;

/// Mirror of the `native` payload of `NativeModCameraView` from donor
/// `src/world/session/mod-client-presentation.ts` (canonical home:
/// `qa_world::session::mod_client_presentation`); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeCameraProjection {
    /// Source edition.
    pub edition: NativeCameraEdition,
    /// Unpredicted source movement origin.
    pub movement_origin: Vec3,
    /// Source render flags.
    pub render_flags: u8,
    /// Selected movement controls the position.
    pub position_prediction: bool,
    /// Selected movement controls the angles.
    pub angular_prediction: bool,
    /// Source weapon model is visible.
    pub weapon_visible: bool,
}

/// Mirror of `NativeModCameraView["native"]["edition"]` from donor
/// `src/world/session/mod-client-presentation.ts` (canonical home:
/// `qa_world::session::mod_client_presentation`); unify post-merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeCameraEdition {
    /// Classic API 3 source.
    Classic,
    /// Rerelease API2023 source.
    Rerelease,
}

/// Mirror of `NativeModCameraView` from donor
/// `src/world/session/mod-client-presentation.ts` (canonical home:
/// `qa_world::session::mod_client_presentation`); unify post-merge.
///
/// Rust has no interface extension, so the donor's `PlayerView` base is
/// nested as [`NativeModCameraView::view`] instead of flattened; every
/// donor field is preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeModCameraView {
    /// Projected player view.
    pub view: PlayerView,
    /// Native prediction metadata.
    pub native: NativeCameraProjection,
}

fn to_vec3(value: &Q2Vec3) -> Vec3 {
    Vec3 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
    }
}

/// Public API3/API2023 prediction flags decide whether source or selected
/// movement controls the view.
#[must_use]
pub fn native_mod_camera(state: &Q2Player) -> NativeModCameraView {
    match state {
        Q2Player::Classic(state) => {
            let movement = &state.movement;
            let origin = movement.origin_eighths;
            NativeModCameraView {
                view: classic_guest_player_view(state),
                native: NativeCameraProjection {
                    edition: NativeCameraEdition::Classic,
                    movement_origin: Vec3 {
                        x: f64::from(origin[0]) as f32 / 8.0,
                        y: f64::from(origin[1]) as f32 / 8.0,
                        z: f64::from(origin[2]) as f32 / 8.0,
                    },
                    render_flags: state.view.render_flags,
                    position_prediction: movement.move_type != 4 && movement.flags & 64 == 0,
                    angular_prediction: movement.move_type < 2,
                    weapon_visible: state.view.gun_index != 0 && state.view.fov <= 90,
                },
            }
        }
        Q2Player::Rerelease(state) => {
            let movement = &state.movement;
            NativeModCameraView {
                view: rerelease_guest_player_view(state),
                native: NativeCameraProjection {
                    edition: NativeCameraEdition::Rerelease,
                    movement_origin: to_vec3(&movement.origin),
                    render_flags: state.view.render_flags,
                    position_prediction: movement.move_type != 6 && movement.flags & 64 == 0,
                    angular_prediction: movement.move_type < 4 && movement.flags & 256 == 0,
                    weapon_visible: state.view.gun_index != 0,
                },
            }
        }
    }
}

/// Resolve the presented view from selected movement.
#[must_use]
pub fn resolve_native_mod_camera(view: &NativeModCameraView, origin: Option<Vec3>, angles: Vec3) -> PlayerView {
    let source = &view.native;
    let base = &view.view;
    let resolved_origin = match (source.position_prediction, origin) {
        (true, Some(origin)) => Vec3 {
            x: origin.x + base.origin.x - source.movement_origin.x,
            y: origin.y + base.origin.y - source.movement_origin.y,
            z: origin.z + base.origin.z - source.movement_origin.z,
        },
        _ => base.origin,
    };
    PlayerView {
        origin: resolved_origin,
        angles: if source.angular_prediction { angles } else { base.angles },
        ..base.clone()
    }
}

#[cfg(test)]
mod tests {
    use qa_net::q2_adapters::{
        Q2MovementState, Q2PlayerState, Q2PlayerView as Q2View, Q2RereleaseMovementState, Q2RereleasePlayerState,
        Q2Vec4,
    };

    use super::*;

    fn classic(move_type: u8, flags: i32, gun_index: i32, fov: u8) -> Q2Player {
        Q2Player::Classic(Q2PlayerState {
            view: Q2View {
                view_angles: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                view_offset: Q2Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 22.0,
                },
                kick_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_index,
                gun_frame: 0,
                fov,
                render_flags: 7,
                stats: Vec::new(),
            },
            movement: Q2MovementState {
                move_type,
                origin_eighths: [80, 160, 240],
                velocity_eighths: [0, 0, 0],
                flags,
                time: 0,
                gravity: 800,
                delta_angle_shorts: [0, 0, 0],
            },
            blend: Q2Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        })
    }

    fn rerelease(move_type: u8, flags: i32, gun_index: i32) -> Q2Player {
        Q2Player::Rerelease(Q2RereleasePlayerState {
            view: Q2View {
                view_angles: Q2Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                view_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                kick_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_index,
                gun_frame: 0,
                fov: 110,
                render_flags: 9,
                stats: Vec::new(),
            },
            movement: Q2RereleaseMovementState {
                move_type,
                origin: Q2Vec3 { x: 5.0, y: 6.0, z: 7.0 },
                velocity: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                flags,
                time: 0,
                gravity: 800,
                delta_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                view_height: 22,
            },
            gun_skin: 0,
            gun_rate: 0,
            screen_blend: Q2Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
            damage_blend: Q2Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
            team_id: 0,
        })
    }

    #[test]
    fn classic_prediction_flags_follow_source_type() {
        let camera = native_mod_camera(&classic(0, 0, 3, 90));
        assert_eq!(camera.native.edition, NativeCameraEdition::Classic);
        assert!(camera.native.position_prediction);
        assert!(camera.native.angular_prediction);
        assert!(camera.native.weapon_visible);
        assert_eq!(camera.native.render_flags, 7);
        assert_eq!(
            camera.native.movement_origin,
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0
            }
        );
        let frozen = native_mod_camera(&classic(4, 64, 0, 100));
        assert!(!frozen.native.position_prediction);
        assert!(!frozen.native.angular_prediction);
        assert!(!frozen.native.weapon_visible);
    }

    #[test]
    fn rerelease_flags_use_rerelease_thresholds() {
        let camera = native_mod_camera(&rerelease(0, 0, 2));
        assert_eq!(camera.native.edition, NativeCameraEdition::Rerelease);
        assert!(camera.native.position_prediction);
        assert!(camera.native.angular_prediction);
        assert!(camera.native.weapon_visible);
        assert_eq!(camera.native.movement_origin, Vec3 { x: 5.0, y: 6.0, z: 7.0 });
        let frozen = native_mod_camera(&rerelease(6, 64 | 256, 2));
        assert!(!frozen.native.position_prediction);
        assert!(!frozen.native.angular_prediction);
    }

    #[test]
    fn resolve_applies_selected_prediction() {
        let camera = native_mod_camera(&classic(0, 0, 1, 90));
        let angles = Vec3 { x: 9.0, y: 8.0, z: 7.0 };
        let resolved = resolve_native_mod_camera(
            &camera,
            Some(Vec3 {
                x: 12.0,
                y: 22.0,
                z: 32.0,
            }),
            angles,
        );
        assert_eq!(resolved.angles, angles);
        assert_eq!(
            resolved.origin,
            Vec3 {
                x: 12.0,
                y: 22.0,
                z: 32.0,
            }
        );
        let frozen = native_mod_camera(&classic(4, 64, 1, 90));
        let kept = resolve_native_mod_camera(&frozen, Some(Vec3 { x: 1.0, y: 1.0, z: 1.0 }), angles);
        assert_eq!(kept.origin, frozen.view.origin);
        assert_eq!(kept.angles, frozen.view.angles);
    }
}
