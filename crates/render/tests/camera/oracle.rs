use super::{Camera, Refdef, Vec3, Viewport};

#[test]
fn cached_gradients_keep_previous_float_operation_order() {
    for viewport in [
        Viewport {
            x: 0,
            y: 0,
            width: 640,
            height: 400,
        },
        Viewport {
            x: 37,
            y: 23,
            width: 301,
            height: 177,
        },
    ] {
        for fov in [[90.0, 75.0], [117.0, 53.0]] {
            let camera = Camera::load(
                Refdef {
                    viewport,
                    fov,
                    ..Refdef::default()
                },
                1024,
                768,
            )
            .unwrap();
            let scale = [
                viewport.width as f32 / (2.0 * camera.tangent[0]),
                viewport.height as f32 / (2.0 * camera.tangent[1]),
            ];
            let center = [
                viewport.x as f32 + viewport.width as f32 * 0.5 - 0.5,
                viewport.y as f32 + viewport.height as f32 * 0.5 - 0.5,
            ];
            assert_eq!(
                camera.screen_scale().map(f32::to_bits),
                scale.map(f32::to_bits)
            );
            assert_eq!(
                camera.screen_center().map(f32::to_bits),
                center.map(f32::to_bits)
            );
            for axis in [
                Vec3([0.0, -0.0, 1.0]),
                Vec3([0.713, -0.371, 12.375]),
                Vec3([-1047.3, 777.1, -17.3]),
            ] {
                for factor in [1.0, 0.5, 0.25, 0.125] {
                    let x = axis.0[0] * ((1.0 / scale[0]) * factor);
                    let y = -axis.0[1] * ((1.0 / scale[1]) * factor);
                    let origin = axis.0[2] * factor - center[0] * x - center[1] * y;
                    let actual = camera.gradient(axis, factor);
                    assert_eq!(
                        [actual.x, actual.y, actual.origin].map(f32::to_bits),
                        [x, y, origin].map(f32::to_bits)
                    );
                    for [px, py] in [[0.0, 0.0], [127.0, 81.0], [301.0, 177.0]] {
                        assert_eq!(
                            actual.at(px, py).to_bits(),
                            (origin + y * py + x * px).to_bits()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn viewport_fit_preserves_zero_overflow_and_edge_rejection() {
    for (viewport, width, height) in [
        (
            Viewport {
                x: 0,
                y: 0,
                width: 0,
                height: 1,
            },
            1,
            1,
        ),
        (
            Viewport {
                x: u32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
            u32::MAX,
            1,
        ),
        (
            Viewport {
                x: 7,
                y: 3,
                width: 9,
                height: 5,
            },
            16,
            8,
        ),
        (
            Viewport {
                x: 7,
                y: 3,
                width: 10,
                height: 5,
            },
            16,
            8,
        ),
    ] {
        let original = viewport.width > 0
            && viewport.height > 0
            && u64::from(viewport.x) + u64::from(viewport.width) <= u64::from(width)
            && u64::from(viewport.y) + u64::from(viewport.height) <= u64::from(height);
        assert_eq!(viewport.fits(width, height), original);
    }
}
