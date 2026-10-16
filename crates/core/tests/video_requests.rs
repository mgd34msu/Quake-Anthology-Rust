use qa_core::presentation::{
    DisplayConfiguration, ExclusivePolicy, GlContextRequest, MAX_WINDOW_DIMENSION, WindowMode,
    WindowRequest,
};

#[test]
fn ordinary_window_requests_preserve_explicit_size_and_current_context_defaults() {
    let request = WindowRequest::windowed(320, 200);
    assert_eq!(
        (request.width, request.height, request.mode),
        (320, 200, WindowMode::Windowed)
    );
    assert_eq!(request.exclusive_policy, ExclusivePolicy::Exact);
    assert_eq!(request.position, None);
    assert_eq!(request.window_aspect, None);
    assert_eq!(
        request.gl,
        GlContextRequest {
            color_bits: 24,
            depth_bits: 24,
            stencil_bits: 0,
            stereo: false,
            swap_interval: 0,
        }
    );
}

#[test]
fn aspect_metadata_preserves_native_float_values_independently_of_extent() {
    let mut display = DisplayConfiguration {
        width: 640,
        height: 480,
        ..Default::default()
    };
    assert_eq!(display.aspect().to_bits(), (640.0f32 / 480.0).to_bits());
    for aspect in [1.0f32, 1.25, -0.0, -2.0, f32::INFINITY, f32::NAN] {
        display.window_aspect = Some(aspect);
        assert_eq!(display.aspect().to_bits(), aspect.to_bits());
        let request = WindowRequest {
            window_aspect: Some(aspect),
            position: Some([-100, 20]),
            ..WindowRequest::windowed(display.width, display.height)
        };
        assert!(request.validate(false).is_ok());
        assert!(request.validate(true).is_ok());
    }
}

#[test]
fn request_validation_selects_only_the_executed_backend_domain() {
    let mut request = WindowRequest::windowed(MAX_WINDOW_DIMENSION, 1);
    request.gl.color_bits = u32::MAX;
    request.gl.swap_interval = i32::MIN;
    assert!(request.validate(false).is_ok());
    assert!(request.validate(true).is_err());
    request.gl.color_bits = 30;
    assert!(request.validate(true).is_ok());
    for interval in [i32::MIN, -1, 0, 1, i32::MAX] {
        request.gl.swap_interval = interval;
        assert!(request.validate(true).is_ok());
    }
    request.mode = WindowMode::Exclusive {
        frequency: -1,
        color_bits: u32::MAX,
    };
    assert!(request.validate(false).is_err());
    request.exclusive_policy = ExclusivePolicy::EnclosingOrWindowed;
    assert!(request.validate(false).is_ok());
    assert!(request.validate(true).is_ok());
    request.mode = WindowMode::Desktop;
    request.width = MAX_WINDOW_DIMENSION + 1;
    assert!(request.validate(false).is_err());
}
