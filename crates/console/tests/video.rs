use qa_console::{
    conversion::Error,
    cvars::{Cvars, RuntimePolicy, WriteError},
    pending::PendingSettings,
    views::{Context, RuleSetId},
};
use qa_core::presentation::{DisplayMode, VideoCapabilities, WindowMode};
use std::sync::Arc;

fn context(source: RuleSetId) -> Context {
    Context {
        source,
        ..Context::default()
    }
}
fn mode(width: u32, height: u32, frequency: i32, color_bits: u32) -> DisplayMode {
    DisplayMode {
        width,
        height,
        frequency,
        color_bits,
    }
}
fn capabilities() -> Arc<VideoCapabilities> {
    Arc::new(VideoCapabilities {
        desktop: mode(1920, 1080, 60, 32),
        fullscreen: vec![
            mode(640, 480, 60, 32),
            mode(800, 600, 75, 24),
            mode(1280, 720, 60, 32),
        ]
        .into_boxed_slice(),
    })
}
fn registry() -> Cvars {
    let mut vars = Cvars::new().unwrap();
    vars.set_video_policy(capabilities(), mode(640, 400, 0, 0))
        .unwrap();
    vars
}

fn write(vars: &mut Cvars, name: &str, value: &str, source: RuleSetId) {
    vars.write(vars.bind(name, context(source)).unwrap(), value)
        .unwrap();
}
fn read(vars: &Cvars, name: &str, source: RuleSetId) -> String {
    vars.read(vars.bind(name, context(source)).unwrap())
        .unwrap()
        .as_str()
        .to_owned()
}
#[test]
fn native_tables_convert_to_one_canonical_mode_and_preserve_dynamic_window_zero() {
    let mut vars = registry();
    write(&mut vars, "gl_mode", "4", RuleSetId::Quake2);
    assert_eq!(read(&vars, "r_mode", RuleSetId::Quake3), "4.000000");
    assert_eq!(vars.window_request(false, false).unwrap().width, 800);
    write(&mut vars, "vid_mode", "2", RuleSetId::Quake);
    assert_eq!(read(&vars, "vid_mode", RuleSetId::Quake), "2");
    assert_eq!(read(&vars, "r_mode", RuleSetId::Quake3), "4.000000");
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Exclusive {
            frequency: 75,
            color_bits: 24
        }
    );
    write(&mut vars, "vid_mode", "0", RuleSetId::Quake);
    let request = vars.window_request(false, false).unwrap();
    assert_eq!(
        (request.width, request.height, request.mode),
        (640, 400, WindowMode::Windowed)
    );
    assert_eq!(read(&vars, "vid_mode", RuleSetId::Quake), "0");
    let view = vars.bind("gl_mode", context(RuleSetId::Quake2)).unwrap();
    vars.write(view, "10").unwrap();
    let safe = vars.window_request(false, false).unwrap();
    assert_eq!(
        (safe.width, safe.height, safe.mode),
        (640, 480, WindowMode::Windowed)
    );
    assert_eq!(read(&vars, "gl_mode", RuleSetId::Quake2), "3");
    let q1 = vars.bind("vid_mode", context(RuleSetId::Quake)).unwrap();
    assert_eq!(
        vars.write(q1, "99"),
        Err(WriteError::Conversion(Error::InvalidVideoMode))
    );
    write(&mut vars, "r_mode", "999", RuleSetId::Quake3);
    let safe = vars.window_request(false, false).unwrap();
    assert_eq!((safe.width, safe.height), (640, 480));
    assert_eq!(read(&vars, "r_mode", RuleSetId::Quake3), "999");
    write(&mut vars, "r_mode", "11", RuleSetId::Quake3);
    assert_eq!(
        (
            vars.window_request(false, false).unwrap().width,
            vars.window_request(false, false).unwrap().height
        ),
        (856, 480)
    );
}
#[test]
fn rerelease_fullscreen_preserves_desktop_frequency_depth_and_original_index() {
    let mut vars = registry();
    write(
        &mut vars,
        "vid_modelist",
        "desktop 1024x768@75:24 800x600:32@60",
        RuleSetId::Quake2Rerelease,
    );
    write(&mut vars, "vid_fullscreen", "2", RuleSetId::Quake2Rerelease);
    let request = vars.window_request(false, true).unwrap();
    assert_eq!((request.width, request.height), (1024, 768));
    assert_eq!(
        request.mode,
        WindowMode::Exclusive {
            frequency: 75,
            color_bits: 24
        }
    );
    assert_eq!(request.gl.color_bits, 24);
    assert_eq!(
        read(&vars, "vid_fullscreen", RuleSetId::Quake2Rerelease),
        "2"
    );
    write(&mut vars, "vid_fullscreen", "3", RuleSetId::Quake2Rerelease);
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Exclusive {
            frequency: 60,
            color_bits: 32
        }
    );
    write(&mut vars, "vid_fullscreen", "1", RuleSetId::Quake2Rerelease);
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Desktop
    );
    write(&mut vars, "vid_fullscreen", "0", RuleSetId::Quake2Rerelease);
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Windowed
    );
    write(
        &mut vars,
        "vid_modelist",
        "bad 640x480",
        RuleSetId::Quake2Rerelease,
    );
    let view = vars
        .bind("vid_fullscreen", context(RuleSetId::Quake2Rerelease))
        .unwrap();
    vars.write(view, "2").unwrap();
    assert_eq!(
        read(&vars, "vid_fullscreen", RuleSetId::Quake2Rerelease),
        "2"
    );
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Desktop
    );
    vars.write(view, "99").unwrap();
    assert_eq!(
        read(&vars, "vid_fullscreen", RuleSetId::Quake2Rerelease),
        "99"
    );
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Desktop
    );
    write(
        &mut vars,
        "vid_modelist",
        "1x1 800x600@75:24tail",
        RuleSetId::Quake2Rerelease,
    );
    vars.write(view, "2").unwrap();
    let request = vars.window_request(false, false).unwrap();
    assert_eq!((request.width, request.height), (800, 600));
    assert_eq!(
        request.mode,
        WindowMode::Exclusive {
            frequency: 75,
            color_bits: 24
        }
    );
}
#[test]
fn pending_alias_groups_update_native_dependencies_and_apply_without_server_latches() {
    let mut vars = registry();
    write(&mut vars, "r_mode", "3", RuleSetId::Quake3);
    write(&mut vars, "r_fullscreen", "0", RuleSetId::Quake3);
    vars.set_runtime_policy(RuntimePolicy {
        initialized: true,
        server_active: true,
    });
    let alias = vars.bind("vid_mode", context(RuleSetId::Quake)).unwrap();
    let native = vars.native_view(alias).unwrap();
    let before = vars.native_snapshot(native).unwrap().1;
    let selected = PendingSettings::capture(&vars, vars.window_views(false).unwrap());
    vars.write(alias, "3").unwrap();
    assert!(!selected.unchanged(&vars));
    assert!(vars.native_snapshot(native).unwrap().1 > before);
    assert_eq!(vars.latched(alias).unwrap().unwrap().as_str(), "3");
    assert_eq!(vars.window_request(false, false).unwrap().width, 640);
    assert_eq!(vars.window_request(true, false).unwrap().width, 1280);
    let game = vars.bind("g_gametype", Context::default()).unwrap();
    vars.write(game, "4").unwrap();
    let selected = PendingSettings::capture(&vars, vars.window_views(false).unwrap());
    assert!(selected.unchanged(&vars));
    selected.apply(&mut vars).unwrap();
    assert_eq!(vars.window_request(false, false).unwrap().width, 1280);
    assert!(vars.latched(alias).unwrap().is_none());
    assert!(vars.latched(game).unwrap().is_some());
}
#[test]
fn unresolved_defaults_use_the_supplied_window_and_cpu_ignores_gl_only_pending_values() {
    let mut vars = registry();
    vars.select_context(context(RuleSetId::Quake));
    let request = vars.window_request(false, false).unwrap();
    assert_eq!((request.width, request.height), (640, 400));
    vars.select_context(Context::default());
    write(&mut vars, "r_mode", "3", RuleSetId::Quake3);
    write(&mut vars, "r_depthbits", "-1", RuleSetId::Quake3);
    assert!(vars.window_request(false, false).is_ok());
    assert_eq!(
        vars.window_request(false, true),
        Err(Error::InvalidVideoMode)
    );
    assert!(
        !vars
            .window_views(false)
            .unwrap()
            .iter()
            .any(|&view| vars.name(view) == "r_depthbits")
    );
}

#[test]
fn custom_dimensions_refresh_cached_alias_values_without_promoting_a_mode() {
    let mut vars = registry();
    write(&mut vars, "r_mode", "-1", RuleSetId::Quake3);
    write(&mut vars, "r_customwidth", "640", RuleSetId::Quake3);
    write(&mut vars, "r_customheight", "480", RuleSetId::Quake3);
    let alias = vars.bind("gl_mode", context(RuleSetId::Quake2)).unwrap();
    let native = vars.native_view(alias).unwrap();
    let generation = vars.view_generation(alias);
    let modifications = vars.native_snapshot(native).unwrap().1;
    assert_eq!(vars.integer_view(alias).unwrap(), 3);
    write(&mut vars, "r_customwidth", "800", RuleSetId::Quake3);
    write(&mut vars, "r_customheight", "600", RuleSetId::Quake3);
    assert_eq!(vars.integer_view(alias).unwrap(), 4);
    assert!(vars.view_generation(alias) > generation);
    assert!(vars.native_snapshot(native).unwrap().1 > modifications);
    assert_eq!(read(&vars, "r_mode", RuleSetId::Quake3), "-1");
}

#[test]
fn installing_video_capabilities_refreshes_already_registered_native_aliases() {
    let mut vars = Cvars::new().unwrap();
    let mode_handle = vars.find("r_mode").unwrap();
    // The catalog intentionally leaves the Q2 GL3/SW0 default unresolved.
    // Seed an explicit common mode before testing policy-cache installation.
    assert!(!vars.default_available(mode_handle, RuleSetId::Quake2));
    write(&mut vars, "r_mode", "3", RuleSetId::Quake3);
    let view = vars.bind("gl_mode", context(RuleSetId::Quake2)).unwrap();
    let native = vars.native_view(view).unwrap();
    let before = vars.native_snapshot(native).unwrap().1;
    assert_eq!(vars.read(view).err(), Some(Error::VideoPolicyRequired));
    vars.set_video_policy(capabilities(), mode(640, 400, 0, 0))
        .unwrap();
    assert_eq!(vars.integer_view(view).unwrap(), 3);
    assert!(vars.native_snapshot(native).unwrap().1 > before);
}

#[test]
fn windowed_cpu_publication_keeps_unused_gl_latches_pending() {
    let mut vars = registry();
    write(&mut vars, "r_mode", "3", RuleSetId::Quake3);
    write(&mut vars, "r_fullscreen", "0", RuleSetId::Quake3);
    vars.set_runtime_policy(RuntimePolicy {
        initialized: true,
        server_active: false,
    });
    let color = vars.bind("r_colorbits", Context::default()).unwrap();
    let depth = vars.bind("r_depthbits", Context::default()).unwrap();
    let swap = vars.bind("r_swapInterval", Context::default()).unwrap();
    vars.set_latch_active(swap.canonical(), true);
    // r_swapInterval is native nonlatched, so register a real module latch
    // on the existing same canonical row for this selected-group proof.
    vars.register("r_swapInterval", None, 33, Context::default())
        .unwrap();
    vars.write(color, "16").unwrap();
    vars.write(depth, "16").unwrap();
    vars.write(swap, "1").unwrap();
    let views = vars.window_views(false).unwrap();
    assert!(
        !views
            .iter()
            .any(|&view| view.canonical() == color.canonical()
                || view.canonical() == depth.canonical()
                || view.canonical() == swap.canonical())
    );
    PendingSettings::capture(&vars, views)
        .apply(&mut vars)
        .unwrap();
    assert!(vars.latched(color).unwrap().is_some());
    assert!(vars.latched(depth).unwrap().is_some());
    assert!(vars.latched(swap).unwrap().is_some());
    let gl = vars.window_views(true).unwrap();
    assert!([color, depth, swap].into_iter().all(|selected| {
        gl.iter()
            .any(|view| view.canonical() == selected.canonical())
    }));
    PendingSettings::capture(&vars, gl)
        .apply(&mut vars)
        .unwrap();
    assert!(vars.latched(color).unwrap().is_none());
    assert!(vars.latched(depth).unwrap().is_none());
    assert!(vars.latched(swap).unwrap().is_none());
    vars.write(vars.bind("r_fullscreen", Context::default()).unwrap(), "1")
        .unwrap();
    let views = vars.window_views(false).unwrap();
    assert!(
        views
            .iter()
            .any(|view| view.canonical() == color.canonical())
    );
    assert_eq!(
        vars.window_request(true, false).unwrap().mode,
        WindowMode::Exclusive {
            frequency: 0,
            color_bits: 16
        }
    );
}

#[test]
fn rewriting_modelist_reprojects_the_retained_index_in_one_pending_group() {
    let mut vars = registry();
    let source = RuleSetId::Quake2Rerelease;
    write(&mut vars, "vid_modelist", "640x480 800x600", source);
    write(&mut vars, "vid_fullscreen", "2", source);
    let selector = vars.bind("vid_fullscreen", context(source)).unwrap();
    let native = vars.native_view(selector).unwrap();
    let before = vars.native_snapshot(native).unwrap().1;
    vars.set_runtime_policy(RuntimePolicy {
        initialized: true,
        server_active: true,
    });
    write(&mut vars, "vid_modelist", "640x480 1024x768@75:24", source);
    assert_eq!(vars.window_request(false, false).unwrap().width, 800);
    assert_eq!(read(&vars, "vid_fullscreen", source), "2");
    assert_eq!(vars.latched(selector).unwrap().unwrap().as_str(), "2");
    let pending = vars.window_request(true, false).unwrap();
    assert_eq!((pending.width, pending.height), (1024, 768));
    assert_eq!(
        pending.mode,
        WindowMode::Exclusive {
            frequency: 75,
            color_bits: 24
        }
    );
    assert!(vars.native_snapshot(native).unwrap().1 > before);
    // Replacing the same pending list must not preserve its previous target.
    write(&mut vars, "vid_modelist", "desktop 1280x720:32@60", source);
    let pending = vars.window_request(true, false).unwrap();
    assert_eq!((pending.width, pending.height), (1280, 720));
    assert_eq!(vars.latched(selector).unwrap().unwrap().as_str(), "2");
    PendingSettings::capture(&vars, vars.window_views(false).unwrap())
        .apply(&mut vars)
        .unwrap();
    assert_eq!(vars.window_request(false, false).unwrap(), pending);
    assert_eq!(read(&vars, "vid_fullscreen", source), "2");
    assert!(vars.latched(selector).unwrap().is_none());
    write(&mut vars, "vid_modelist", "bad", source);
    assert_eq!(
        vars.window_request(true, false).unwrap().mode,
        WindowMode::Desktop
    );
    PendingSettings::capture(&vars, vars.window_views(false).unwrap())
        .apply(&mut vars)
        .unwrap();
    assert_eq!(read(&vars, "vid_fullscreen", source), "2");
    assert_eq!(
        vars.window_request(false, false).unwrap().mode,
        WindowMode::Desktop
    );
}

#[test]
fn modelist_edit_does_not_restore_a_replaced_or_absent_native_selector() {
    let source = RuleSetId::Quake2Rerelease;
    for selected in [RuleSetId::Quake, RuleSetId::Quake2, RuleSetId::Quake3] {
        let mut vars = registry();
        write(&mut vars, "r_mode", "4", selected);
        write(&mut vars, "r_fullscreen", "0", selected);
        let before = vars.window_request(false, false).unwrap();
        write(&mut vars, "vid_modelist", "desktop 1024x768", source);
        assert_eq!(vars.window_request(false, false).unwrap(), before);
    }
    for pending in [false, true] {
        let mut vars = registry();
        write(&mut vars, "vid_modelist", "640x480 800x600", source);
        write(&mut vars, "vid_fullscreen", "2", source);
        if pending {
            vars.set_runtime_policy(RuntimePolicy {
                initialized: true,
                server_active: true,
            });
        }
        write(&mut vars, "r_mode", "6", RuleSetId::Quake3);
        let before = vars.window_request(true, false).unwrap();
        assert_eq!(before.width, 1024);
        write(&mut vars, "vid_modelist", "desktop 1280x720", source);
        assert_eq!(vars.window_request(true, false).unwrap(), before);
        if pending {
            PendingSettings::capture(&vars, vars.window_views(false).unwrap())
                .apply(&mut vars)
                .unwrap();
            assert_eq!(vars.window_request(false, false).unwrap(), before);
        }
    }
}
