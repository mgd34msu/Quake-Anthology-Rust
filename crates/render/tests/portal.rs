use qa_core::{
    math,
    primitives::{Plane, Vec3},
};
use qa_render::{Refdef, SceneEntity, SceneModel, Viewport, portal::camera};

fn view() -> Refdef {
    Refdef {
        viewport: Viewport {
            width: 64,
            height: 64,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn portal(origin: Vec3, destination: Vec3) -> SceneEntity {
    SceneEntity {
        origin,
        model: SceneModel::Portal {
            camera: destination,
            rotation: 0,
        },
        ..Default::default()
    }
}

#[test]
fn reflected_camera_flips_the_eye_and_keeps_destination_clipping_and_pvs_origin() {
    let plane = Plane::oriented(Vec3([-1., 0., 0.]), -10.);
    let entity = portal(Vec3([10., 0., 0.]), Vec3([10., 0., 0.]));
    let input = Refdef {
        origin: Vec3([1., 2., 3.]),
        blend: [1.; 4],
        ..view()
    };
    let output = camera(plane, plane, &[entity], input).unwrap();
    assert_eq!(output.origin, Vec3([19., 2., 3.]));
    assert_eq!(
        output.axes,
        [Vec3([-1., 0., 0.]), Vec3([0., 1., 0.]), Vec3([0., 0., 1.])]
    );
    assert!(output.portal && output.reflected_view);
    assert_eq!(output.pvs_origin, Some(entity.origin));
    let clip = output.clip_plane.unwrap();
    assert_eq!(clip, plane);
    assert!(clip.signed_distance(Vec3([9., 0., 0.])) > 0.);
    assert!(clip.signed_distance(Vec3([11., 0., 0.])) < 0.);
    assert_eq!(output.blend, [0.; 4]);
    assert!(camera(plane, plane, &[entity], output).is_none());
}

#[test]
fn remote_cameras_keep_first_match_threshold_and_all_native_rotation_modes() {
    let plane = Plane::oriented(Vec3([-1., 0., 0.]), -10.);
    let destination = Vec3([100., 200., 300.]);
    let base = portal(Vec3([10., 0., 0.]), destination);
    let far = portal(Vec3([75., 0., 0.]), Vec3([900.; 3]));
    let second = portal(Vec3([10., 0., 0.]), Vec3([500.; 3]));
    let input = Refdef {
        origin: Vec3([1., 2., 3.]),
        time_ms: 1000,
        ..view()
    };
    let output = camera(plane, plane, &[far, base, second], input).unwrap();
    assert_eq!(output.origin, Vec3([91., 198., 297.]));
    assert_eq!(
        output.axes,
        [Vec3([1., 0., 0.]), Vec3([0., -1., 0.]), Vec3([0., 0., -1.])]
    );
    assert_eq!(output.pvs_origin, Some(destination));
    assert!(!output.reflected_view);
    assert_eq!(
        camera(
            plane,
            plane,
            &[portal(Vec3([74., 0., 0.]), destination)],
            input
        )
        .unwrap()
        .pvs_origin,
        Some(destination)
    );
    assert!(camera(plane, plane, &[far], input).is_none());
    for (frame, old_frame, rotation, degrees) in [
        (90_u32, 1, 0, 90.),
        ((-90_i32) as u32, 1, 0, -90.),
        (0, 0, 45, 45.),
        (
            0,
            1,
            20,
            20. + (f64::from(1000_f32 * 0.003).sin() as f32) * 4.,
        ),
    ] {
        let entity = SceneEntity {
            frame,
            old_frame,
            model: SceneModel::Portal {
                camera: destination,
                rotation,
            },
            ..base
        };
        let output = camera(plane, plane, &[entity], input).unwrap();
        let forward = -base.axes[0];
        let left = math::rotate_about_axis(forward, -base.axes[1], degrees);
        let axes = [forward, left, math::cross(forward, left)];
        assert_eq!(output.clip_plane.unwrap().normal, -axes[0]);
        assert_eq!(output.axes[1], axes[1]);
        assert_eq!(output.axes[2], -axes[2]);
    }
}

#[test]
fn an_inline_matching_plane_is_independent_of_the_rendered_orientation() {
    let original = Plane::oriented(Vec3([-1., 0., 0.]), -10.);
    let transformed = Plane::oriented(Vec3([0., -1., 0.]), -200.);
    let entity = portal(Vec3([10., 999., 0.]), Vec3([300., 400., 500.]));
    assert!(camera(transformed, original, &[entity], view()).is_some());
    assert!(camera(transformed, transformed, &[entity], view()).is_none());
}
