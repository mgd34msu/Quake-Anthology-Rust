use qa_app::{renderer::Kind, window_settings};
use qa_console::{
    cvars::Cvars,
    views::{Context, RuleSetId},
};
use qa_core::presentation::{
    DisplayConfiguration, DisplayMode, ExclusivePolicy, VideoCapabilities, WindowMode,
    WindowRequest,
};
use std::sync::Arc;

fn registry(source: RuleSetId) -> Cvars {
    let context = Context {
        source,
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
    vars
}
fn write(vars: &mut Cvars, name: &str, text: &str) {
    let view = vars.bind(name, vars.context()).unwrap();
    vars.write(view, text).unwrap();
}

#[test]
fn startup_cli_axis_override_keeps_the_other_native_saved_dimension() {
    let mut vars = registry(RuleSetId::Quake2);
    let context = vars.context();
    // Q2's original mode 4 is 800x600; the same conversion serves saved files.
    vars.write(vars.bind("gl_mode", context).unwrap(), "4")
        .unwrap();
    vars.write(vars.bind("vid_fullscreen", context).unwrap(), "0")
        .unwrap();
    let plan = window_settings::startup(&mut vars, Kind::Cpu, Some(1024), None).unwrap();
    assert_eq!((plan.request().width, plan.request().height), (1024, 600));
    assert_eq!(plan.request().mode, WindowMode::Windowed);
    assert_eq!(plan.request().window_aspect, None);
    plan.apply(&mut vars, &DisplayConfiguration::default())
        .unwrap();
    plan.validate(&vars).unwrap();
    let custom_height = vars.bind("r_customheight", context).unwrap();
    vars.write(custom_height, "720").unwrap();
    assert!(plan.validate(&vars).is_err());
    assert_eq!((plan.request().width, plan.request().height), (1024, 600));
    let next = window_settings::WindowSettingsPlan::prepare(&vars, Kind::Cpu, None).unwrap();
    assert_eq!((next.request().width, next.request().height), (1024, 720));
    write(&mut vars, "r_customaspect", "1.25");
    let resized = window_settings::startup(&mut vars, Kind::Cpu, Some(1280), None).unwrap();
    assert_eq!(
        (resized.request().width, resized.request().height),
        (1280, 720)
    );
    assert_eq!(resized.request().window_aspect, Some(1.25));
}

#[test]
fn actual_windowed_fallback_reflects_q2_and_retains_q3_fullscreen_intent() {
    for source in [RuleSetId::Quake2, RuleSetId::Quake3] {
        let mut vars = registry(source);
        write(&mut vars, "r_mode", "4");
        write(&mut vars, "r_fullscreen", "1");
        let plan = window_settings::WindowSettingsPlan::prepare(&vars, Kind::Gl, None).unwrap();
        assert!(matches!(plan.request().mode, WindowMode::Exclusive { .. }));
        assert_eq!(
            plan.request().exclusive_policy,
            if source == RuleSetId::Quake2 {
                ExclusivePolicy::ExactOrWindowed
            } else {
                ExclusivePolicy::EnclosingOrWindowed
            }
        );
        let fullscreen = vars.bind("r_fullscreen", vars.context()).unwrap();
        assert_eq!(vars.integer_view(fullscreen).unwrap(), 1);
        plan.validate(&vars).unwrap();
        plan.apply(
            &mut vars,
            &DisplayConfiguration {
                width: 800,
                height: 600,
                fullscreen: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            vars.integer_view(fullscreen).unwrap(),
            i32::from(source == RuleSetId::Quake3)
        );
    }
}

#[test]
fn invalid_geometry_keeps_the_live_baseline_and_startup_uses_native_safe_geometry() {
    let mut vars = registry(RuleSetId::Quake2Rerelease);
    write(&mut vars, "vid_fullscreen", "0");
    write(&mut vars, "vid_geometry", "invalid geometry");
    let mut baseline = WindowRequest::windowed(1024, 768);
    baseline.position = Some([-120, 45]);
    let plan =
        window_settings::WindowSettingsPlan::prepare(&vars, Kind::Cpu, Some(baseline)).unwrap();
    assert_eq!(
        (
            plan.request().width,
            plan.request().height,
            plan.request().position
        ),
        (1024, 768, Some([-120, 45]))
    );
    let initial = window_settings::startup(&mut vars, Kind::Cpu, None, None).unwrap();
    assert_eq!(
        (
            initial.request().width,
            initial.request().height,
            initial.request().position
        ),
        (640, 480, None)
    );
    write(&mut vars, "vid_geometry", "800X600-90+30 trailing");
    assert!(plan.validate(&vars).is_err());
    let changed =
        window_settings::WindowSettingsPlan::prepare(&vars, Kind::Cpu, Some(baseline)).unwrap();
    assert_eq!(
        (
            changed.request().width,
            changed.request().height,
            changed.request().position
        ),
        (800, 600, Some([-90, 30]))
    );
}
