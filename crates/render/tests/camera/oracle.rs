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

#[test]
fn shared_view_point_keeps_post_dot_sign_with_caller_axes() {
    for axes in [
        [
            Vec3([1.0, 0.0, 0.0]),
            Vec3([0.0, 1.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
        ],
        [
            Vec3([0.3, -0.7, 0.1]),
            Vec3([-0.2, 0.6, 0.3]),
            Vec3([0.7, 0.3, -0.1]),
        ],
    ] {
        for point in [
            Vec3([0.0, -0.0, 0.0]),
            Vec3([-0.0; 3]),
            Vec3([0.6, 0.2, 0.0]),
            Vec3([37.3, -81.7, 23.0]),
        ] {
            let expected = Vec3([-point.dot(axes[1]), point.dot(axes[2]), point.dot(axes[0])]);
            assert_eq!(
                super::view_point(Vec3::default(), axes, point)
                    .0
                    .map(f32::to_bits),
                expected.0.map(f32::to_bits)
            );
        }
    }
}

#[test]
fn shared_refdef_boundary_and_tangents_match_camera_and_frustum() {
    let base = Refdef {
        viewport: Viewport {
            x: 7,
            y: 3,
            width: 640,
            height: 400,
        },
        ..Refdef::default()
    };
    for fov in [
        0.0,
        -0.0,
        1.0,
        90.0,
        178.0,
        179.0,
        179.5,
        180.0,
        f32::NAN,
        f32::INFINITY,
    ] {
        let view = Refdef {
            fov: [fov; 2],
            ..base
        };
        let expected = fov > 0.0 && fov < 180.0;
        let tangent = view.validate(1024, 768);
        assert_eq!(tangent.is_some(), expected, "fov {fov}");
        assert_eq!(Camera::load(view, 1024, 768).is_some(), expected);
        assert_eq!(crate::world::frustum(view).is_ok(), expected);
        if let Some(tangent) = tangent {
            let original = (fov.to_radians() * 0.5).tan();
            assert_eq!(tangent.map(f32::to_bits), [original.to_bits(); 2]);
        }
    }
    for identity_light in [-0.01, 0.0, 0.5, 1.0, 1.01, f32::NAN, f32::INFINITY] {
        let view = Refdef {
            identity_light,
            ..base
        };
        let expected = (0.0..=1.0).contains(&identity_light);
        assert_eq!(view.validate(1024, 768).is_some(), expected);
        assert_eq!(Camera::load(view, 1024, 768).is_some(), expected);
        assert_eq!(crate::world::frustum(view).is_ok(), expected);
    }
    for view in [
        Refdef {
            near: f32::from_bits(1),
            ..base
        },
        Refdef {
            far: base.near,
            ..base
        },
        Refdef {
            origin: Vec3([f32::NAN, 0.0, 0.0]),
            ..base
        },
        Refdef {
            axes: [Vec3([f32::INFINITY; 3]); 3],
            ..base
        },
        Refdef {
            viewport: Viewport::default(),
            ..base
        },
        Refdef {
            viewport: Viewport {
                x: u32::MAX,
                width: 1,
                ..base.viewport
            },
            ..base
        },
    ] {
        assert!(view.validate(1024, 768).is_none());
        assert!(Camera::load(view, 1024, 768).is_none());
        assert!(crate::world::frustum(view).is_err());
    }
}

#[test]
fn palette_validation_stays_in_cpu_and_blend_bounds_stay_shared() {
    let base = Refdef {
        viewport: Viewport {
            x: 0,
            y: 0,
            width: 640,
            height: 400,
        },
        ..Refdef::default()
    };
    let mut transform = crate::scene::PaletteTransform::default();
    transform.shifts[0].percent = 257;
    let palette = Refdef {
        palette_transform: Some(transform),
        ..base
    };
    assert!(palette.validate(640, 400).is_some());
    assert!(Camera::load(palette, 640, 400).is_none());
    let blend = Refdef {
        blend_phase: crate::scene::BlendPhase::FinalPalette,
        blend_viewport: Some(Viewport {
            width: 641,
            ..base.viewport
        }),
        ..base
    };
    assert!(blend.validate(640, 400).is_none());
    assert!(Camera::load(blend, 640, 400).is_none());
    assert!(
        Refdef {
            blend_phase: crate::scene::BlendPhase::AfterView,
            ..blend
        }
        .validate(640, 400)
        .is_some()
    );
}
