use qa_app::{renderer::Kind, window_settings};
use qa_console::{
    cvars::Cvars,
    views::{Context, RuleSetId},
};
use qa_core::presentation::{DisplayMode, VideoCapabilities, WindowMode};
use std::sync::Arc;

#[test]
fn startup_cli_axis_override_keeps_the_other_native_saved_dimension() {
    let context = Context {
        source: RuleSetId::Quake2,
        ..Context::default()
    };
    let mut vars = Cvars::with_context(context).unwrap();
    let baseline = DisplayMode {
        width: 640,
        height: 400,
        frequency: 60,
        color_bits: 32,
    };
    vars.set_video_policy(
        Arc::new(VideoCapabilities {
            desktop: DisplayMode {
                width: 1920,
                height: 1080,
                ..baseline
            },
            fullscreen: Box::new([]),
        }),
        baseline,
    )
    .unwrap();
    // Q2's original mode 4 is 800x600; the same conversion serves saved files.
    vars.write(vars.bind("gl_mode", context).unwrap(), "4")
        .unwrap();
    vars.write(vars.bind("vid_fullscreen", context).unwrap(), "0")
        .unwrap();
    let plan = window_settings::startup(&mut vars, Kind::Cpu, Some(1024), None).unwrap();
    assert_eq!((plan.request().width, plan.request().height), (1024, 600));
    assert_eq!(plan.request().mode, WindowMode::Windowed);
    plan.apply(&mut vars).unwrap();
    plan.validate(&vars).unwrap();
    let custom_height = vars.bind("r_customheight", context).unwrap();
    vars.write(custom_height, "720").unwrap();
    assert!(plan.validate(&vars).is_err());
    assert_eq!((plan.request().width, plan.request().height), (1024, 600));
    let next = window_settings::WindowSettingsPlan::prepare(&vars, Kind::Cpu).unwrap();
    assert_eq!((next.request().width, next.request().height), (1024, 720));
}
