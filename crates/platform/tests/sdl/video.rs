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
fn swap_intervals_keep_the_full_native_integer_and_require_confirmed_state() {
    for requested in [-1, 0, 1, 3, i32::MAX] {
        let mut received = None;
        assert!(configure_swap_interval(
            requested,
            |interval| {
                received = Some(interval);
                true
            },
            |actual| {
                *actual = requested;
                true
            }
        ));
        assert_eq!(received, Some(requested));
    }
    // Unsupported swap control may still confirm the already-active zero mode.
    assert!(configure_swap_interval(
        0,
        |_| false,
        |actual| {
            *actual = 0;
            true
        }
    ));
    for requested in [-1, 1, 3] {
        assert!(!configure_swap_interval(
            requested,
            |_| false,
            |actual| {
                *actual = 0;
                true
            }
        ));
    }
    assert!(!configure_swap_interval(0, |_| true, |_| false));
}
