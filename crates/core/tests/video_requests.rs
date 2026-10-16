use qa_core::presentation::{GlContextRequest, MAX_WINDOW_DIMENSION, WindowMode, WindowRequest};

#[test]
fn ordinary_window_requests_preserve_explicit_size_and_current_context_defaults() {
    let request = WindowRequest::windowed(320, 200);
    assert_eq!(
        (request.width, request.height, request.mode),
        (320, 200, WindowMode::Windowed)
    );
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
        color_bits: 32,
    };
    assert!(request.validate(false).is_err());
    request.mode = WindowMode::Desktop;
    request.width = MAX_WINDOW_DIMENSION + 1;
    assert!(request.validate(false).is_err());
}
