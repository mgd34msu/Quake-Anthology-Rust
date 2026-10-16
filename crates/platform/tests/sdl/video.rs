use super::*;

fn mode(width: u32, height: u32, frequency: i32, color_bits: u32) -> DisplayMode {
    DisplayMode {
        width,
        height,
        frequency,
        color_bits,
    }
}

#[test]
fn display_mode_storage_depth_and_sdl_layout_follow_the_installed_header() {
    let raw = SdlDisplayMode {
        display: 1,
        format: 0x1616_1804, // SDL_PIXELFORMAT_XRGB8888: 24 RGB bits, 32 storage bits.
        width: 1920,
        height: 1080,
        pixel_density: 1.0,
        refresh_rate: 59.94,
        refresh_numerator: 60000,
        refresh_denominator: 1001,
        internal: std::ptr::null_mut(),
    };
    assert_eq!(display_mode(&raw).unwrap(), mode(1920, 1080, 59, 32));
    let doubled = SdlDisplayMode {
        pixel_density: 2.0,
        ..raw
    };
    assert_eq!(display_mode(&doubled).unwrap(), mode(3840, 2160, 59, 32));
    let fractional = SdlDisplayMode {
        width: 641,
        height: 481,
        pixel_density: 1.25,
        ..raw
    };
    assert_eq!(display_mode(&fractional).unwrap(), mode(802, 602, 59, 32));
    let rgb565 = SdlDisplayMode {
        format: 0x1515_1002,
        ..raw
    };
    assert_eq!(display_mode(&rgb565).unwrap().color_bits, 16);
    let invalid = SdlDisplayMode {
        refresh_rate: f32::NAN,
        ..rgb565
    };
    assert!(display_mode(&invalid).is_err());
    #[cfg(target_pointer_width = "64")]
    {
        assert_eq!(std::mem::size_of::<SdlDisplayMode>(), 40);
        assert_eq!(std::mem::offset_of!(SdlDisplayMode, internal), 32);
    }
}

#[test]
fn exclusive_selection_keeps_exact_extent_depth_and_nearest_supported_frequency() {
    let modes = [
        mode(1920, 1080, 60, 32),
        mode(1280, 720, 144, 16),
        mode(1280, 720, 60, 32),
        mode(1280, 720, 120, 32),
    ];
    let select = |width, height, frequency, bits| {
        exclusive_mode(
            modes.iter().copied().enumerate(),
            width,
            height,
            frequency,
            bits,
            modes[0],
        )
    };
    assert_eq!(select(1280, 720, 0, 0), Some(2));
    assert_eq!(select(1280, 720, 110, 0), Some(3));
    assert_eq!(select(1280, 720, 110, 16), Some(1));
    assert_eq!(select(1280, 720, 110, 24), None);
    assert_eq!(select(640, 480, 60, 0), None);
}

#[test]
fn desktop_fullscreen_preserves_the_requested_software_framebuffer_extent() {
    let request = WindowRequest {
        width: 320,
        height: 200,
        mode: WindowMode::Desktop,
        gl: GlContextRequest {
            color_bits: u32::MAX,
            ..Default::default()
        },
        ..WindowRequest::default()
    };
    assert_eq!(window_dimensions(request, false).unwrap(), (320, 200));
    // An unused GL policy never rejects a software window's dimensions.
    assert!(gl_attributes(request.gl).is_err());
    assert!(
        window_dimensions(
            WindowRequest {
                width: 0,
                ..request
            },
            false
        )
        .is_err()
    );
    assert!(
        window_dimensions(
            WindowRequest {
                height: 8193,
                ..request
            },
            false
        )
        .is_err()
    );
    assert!(
        window_dimensions(
            WindowRequest {
                mode: WindowMode::Exclusive {
                    frequency: -1,
                    color_bits: 32
                },
                ..request
            },
            false
        )
        .is_err()
    );
}

#[test]
fn gl_attributes_keep_native_precision_stereo_and_independent_context_ownership() {
    let attributes = gl_attributes(GlContextRequest {
        color_bits: 16,
        depth_bits: 24,
        stencil_bits: 8,
        stereo: true,
        swap_interval: -1,
    })
    .unwrap();
    let value = |attribute| {
        attributes
            .iter()
            .flatten()
            .find_map(|&(key, value)| (key == attribute).then_some(value))
    };
    assert_eq!(value(GL_RED_SIZE), Some(5));
    assert_eq!(value(GL_GREEN_SIZE), Some(6));
    assert_eq!(value(GL_BLUE_SIZE), Some(5));
    assert_eq!(value(GL_ALPHA_SIZE), Some(0));
    assert_eq!(value(GL_DEPTH_SIZE), Some(24));
    assert_eq!(value(GL_STENCIL_SIZE), Some(8));
    assert_eq!(value(GL_STEREO), Some(1));
    assert_eq!(value(GL_SHARE_WITH_CURRENT_CONTEXT), Some(0));
    let automatic = gl_attributes(GlContextRequest {
        color_bits: 0,
        depth_bits: 0,
        ..Default::default()
    })
    .unwrap();
    assert!(!automatic.iter().flatten().any(|&(key, _)| matches!(
        key,
        GL_BUFFER_SIZE | GL_RED_SIZE | GL_GREEN_SIZE | GL_BLUE_SIZE | GL_ALPHA_SIZE | GL_DEPTH_SIZE
    )));
}

#[test]
fn swap_intervals_keep_native_intent_and_accept_queried_state_when_control_is_optional() {
    for requested in [-1, 0, 1, 3, i32::MAX] {
        let mut received = None;
        assert_eq!(
            configure_swap_interval(
                requested,
                |interval| {
                    received = Some(interval);
                    true
                },
                |actual| {
                    *actual = requested;
                    true
                }
            ),
            Some(requested)
        );
        assert_eq!(received, Some(requested));
    }
    // GLX without swap control keeps its usable zero-interval context, even
    // when a native default requests interval one.
    for requested in [-1, 0, 1, 3] {
        assert_eq!(
            configure_swap_interval(
                requested,
                |_| false,
                |actual| {
                    *actual = 0;
                    true
                }
            ),
            Some(0)
        );
    }
    assert_eq!(
        configure_swap_interval(
            3,
            |_| true,
            |actual| {
                *actual = 1;
                true
            }
        ),
        Some(1)
    );
    assert_eq!(configure_swap_interval(0, |_| true, |_| false), None);
    assert_eq!(configure_swap_interval(1, |_| false, |_| false), None);
}

#[test]
fn enclosing_modes_keep_native_distance_first_tie_and_cutoff() {
    let modes = [
        mode(800, 600, 60, 16),
        mode(800, 600, 144, 32),
        mode(1024, 768, 120, 32),
        mode(640, 480, 60, 32),
    ];
    let select = |width, height| enclosing_mode(modes.iter().copied().enumerate(), width, height);
    assert_eq!(select(700, 500), Some(0));
    assert_eq!(select(640, 480), Some(3));
    assert_eq!(select(1000, 700), Some(2));
    assert_eq!(select(1600, 900), None);
    assert_eq!(
        enclosing_mode([mode(3803, 600, 60, 32)].into_iter().enumerate(), 640, 600),
        None
    );
    assert_eq!(
        enclosing_mode([mode(3802, 600, 60, 32)].into_iter().enumerate(), 640, 600),
        Some(0)
    );
}

#[test]
fn exclusive_fallback_is_bounded_and_retains_extent_position_and_aspect() {
    for policy in [
        ExclusivePolicy::Exact,
        ExclusivePolicy::ExactOrWindowed,
        ExclusivePolicy::EnclosingOrWindowed,
    ] {
        let request = WindowRequest {
            mode: WindowMode::Exclusive {
                frequency: 144,
                color_bits: 32,
            },
            exclusive_policy: policy,
            position: Some([-100, 20]),
            window_aspect: Some(1.25),
            ..WindowRequest::windowed(800, 600)
        };
        let mut attempts = Vec::new();
        let result = configure_request(request, |attempt| {
            attempts.push(attempt);
            if attempt.mode == WindowMode::Windowed {
                Ok(WindowState {
                    request: attempt,
                    physical_mode: None,
                })
            } else {
                Err("unsupported exclusive mode".to_owned())
            }
        });
        if policy == ExclusivePolicy::Exact {
            assert!(result.is_err());
            assert_eq!(attempts, [request]);
        } else {
            let expected = WindowRequest {
                mode: WindowMode::Windowed,
                ..request
            };
            assert_eq!(result.unwrap().request, expected);
            assert_eq!(attempts, [request, expected]);
        }
        let mut attempts = 0;
        assert!(
            configure_request(request, |_| {
                attempts += 1;
                Err("mode failure".to_owned())
            })
            .is_err()
        );
        assert_eq!(
            attempts,
            if policy == ExclusivePolicy::Exact {
                1
            } else {
                2
            }
        );
    }
}

#[test]
fn retained_mode_identity_keeps_fractional_refresh_and_density() {
    let first = SdlDisplayMode {
        display: 1,
        format: 0x1616_1804,
        width: 1920,
        height: 1080,
        pixel_density: 1.0,
        refresh_rate: 59.94,
        refresh_numerator: 60000,
        refresh_denominator: 1001,
        internal: std::ptr::null_mut(),
    };
    let second = SdlDisplayMode {
        refresh_rate: 59.95,
        ..first
    };
    assert_eq!(
        display_mode(&first).unwrap(),
        display_mode(&second).unwrap()
    );
    assert!(first.physical_mode() != second.physical_mode());
    let density = SdlDisplayMode {
        width: 960,
        height: 540,
        pixel_density: 2.0,
        ..first
    };
    assert_eq!(
        display_mode(&first).unwrap(),
        display_mode(&density).unwrap()
    );
    assert!(first.physical_mode() != density.physical_mode());
}

#[test]
fn denied_global_position_preserves_a_usable_window_without_claiming_placement() {
    let mut attempts = Vec::new();
    assert_eq!(
        configure_position(Some([-100, 20]), |x, y| {
            attempts.push([x, y]);
            false
        }),
        None
    );
    assert_eq!(attempts, [[-100, 20]]);
    assert_eq!(configure_position(Some([0, 0]), |_, _| true), Some([0, 0]));
    assert_eq!(
        configure_position(None, |_, _| panic!(
            "unspecified position must not call SDL"
        )),
        None
    );
}
