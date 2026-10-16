use qa_app::render_settings::{image_settings, material_finish, world_load_options};
use qa_console::{
    cvars::Cvars,
    views::{Context, RuleSetId},
};

fn vars(source: RuleSetId) -> Cvars {
    Cvars::with_context(Context {
        source,
        ..Context::default()
    })
    .unwrap()
}

fn set(vars: &mut Cvars, source: RuleSetId, name: &str, value: &str) {
    let context = Context {
        source,
        ..Context::default()
    };
    let view = vars.bind(name, context).unwrap();
    vars.write(view, value).unwrap();
}

#[test]
fn native_upload_defaults_follow_world_source_in_every_console_context() {
    for context in RuleSetId::ALL {
        let vars = vars(context);
        let q1 = image_settings(&vars, RuleSetId::Quake).unwrap();
        let q2 = image_settings(&vars, RuleSetId::Quake2).unwrap();
        let q3 = image_settings(&vars, RuleSetId::Quake3).unwrap();
        let maximum = vars.find("gl_max_size").unwrap();
        assert!(vars.default_available(maximum, RuleSetId::Quake2));
        assert!(!vars.native_default_available(maximum, RuleSetId::Quake2));
        assert!(vars.native_default_available(maximum, RuleSetId::Quake));
        assert_eq!(
            (
                q1.palette_exponent,
                q1.intensity,
                q1.picmip,
                q1.max_dimension
            ),
            (0.7, 1.0, 0, 1024)
        );
        assert_eq!(
            (
                q2.palette_exponent,
                q2.intensity,
                q2.picmip,
                q2.max_dimension
            ),
            (1.0, 2.0, 0, 256)
        );
        assert_eq!(
            (
                q3.palette_exponent,
                q3.intensity,
                q3.picmip,
                q3.max_dimension
            ),
            (1.0, 1.0, 1, 8192)
        );
        assert_eq!(q1.gamma_exponent, 1.0);
        assert_eq!(q2.gamma_exponent, 1.0);
        assert_eq!(q3.gamma_exponent, 1.0);
        assert!(q2.round_images_down && q3.round_images_down);
        assert!(q3.simple_mipmaps);
        assert!(!q2.sky_mip);
    }
}

#[test]
fn converted_aliases_and_canonical_overrides_apply_across_worlds() {
    let mut vars = vars(RuleSetId::Quake3);
    set(&mut vars, RuleSetId::Quake, "gamma", "0.8");
    set(&mut vars, RuleSetId::Quake2, "intensity", "3");
    set(&mut vars, RuleSetId::Quake, "gl_picmip", "2");
    set(&mut vars, RuleSetId::Quake2, "gl_round_down", "0");
    set(&mut vars, RuleSetId::Quake3, "r_simpleMipMaps", "0");
    set(&mut vars, RuleSetId::Quake2, "gl_skymip", "1");
    for source in RuleSetId::ALL {
        let settings = image_settings(&vars, source).unwrap();
        assert_eq!(settings.gamma_exponent, 0.8);
        assert_eq!(settings.intensity, 3.0);
        assert_eq!(settings.picmip, 2);
        assert!(!settings.round_images_down);
        assert!(!settings.simple_mipmaps);
        assert!(settings.sky_mip);
    }
    set(&mut vars, RuleSetId::Quake3, "r_gamma", "2");
    set(&mut vars, RuleSetId::Quake3, "r_picmip", "0");
    for source in RuleSetId::ALL {
        let settings = image_settings(&vars, source).unwrap();
        assert_eq!(settings.gamma_exponent, 0.5);
        assert_eq!(settings.picmip, 0);
    }
}

#[test]
fn native_q3_gamma_clamp_and_image_capacity_checks_are_cold() {
    let mut vars = vars(RuleSetId::Quake);
    set(&mut vars, RuleSetId::Quake3, "r_gamma", "10");
    assert_eq!(
        image_settings(&vars, RuleSetId::Quake3)
            .unwrap()
            .gamma_exponent,
        1.0 / 3.0
    );
    assert_eq!(
        image_settings(&vars, RuleSetId::Quake2)
            .unwrap()
            .gamma_exponent,
        0.1
    );
    set(&mut vars, RuleSetId::Quake, "gl_picmip", "32");
    assert!(image_settings(&vars, RuleSetId::Quake2).is_err());
    set(&mut vars, RuleSetId::Quake, "gl_picmip", "0");
    set(&mut vars, RuleSetId::Quake, "gl_max_size", "0");
    assert!(image_settings(&vars, RuleSetId::Quake).is_err());
}

#[test]
fn upload_switches_preserve_native_float_and_atoi_consumers() {
    let mut vars = vars(RuleSetId::Quake3);
    set(&mut vars, RuleSetId::Quake2, "gl_round_down", "0.5");
    set(&mut vars, RuleSetId::Quake3, "r_simpleMipMaps", "0.5");
    let q2 = image_settings(&vars, RuleSetId::Quake2).unwrap();
    let q3 = image_settings(&vars, RuleSetId::Quake3).unwrap();
    assert!(q2.round_images_down);
    assert!(!q3.round_images_down);
    assert!(!q3.simple_mipmaps);
    set(&mut vars, RuleSetId::Quake3, "r_roundImagesDown", "1e-2");
    set(&mut vars, RuleSetId::Quake3, "r_simpleMipMaps", "1e-2");
    set(&mut vars, RuleSetId::Quake3, "r_picmip", "1e-2");
    let q2 = image_settings(&vars, RuleSetId::Quake2).unwrap();
    let q3 = image_settings(&vars, RuleSetId::Quake3).unwrap();
    assert!(q3.round_images_down && q3.simple_mipmaps);
    assert_eq!(q3.picmip, 1);
    assert_eq!(q2.picmip, 0);
    let rerelease = image_settings(&vars, RuleSetId::Quake2Rerelease).unwrap();
    assert_eq!(rerelease.picmip, 1);
    assert!(rerelease.round_images_down);
}

#[test]
fn native_integer_limits_clamp_before_upload_and_rerelease_retains_large_images() {
    let mut vars = vars(RuleSetId::Quake2Rerelease);
    let rerelease = image_settings(&vars, RuleSetId::Quake2Rerelease).unwrap();
    assert_eq!(rerelease.max_dimension, 8192);
    assert_eq!(rerelease.intensity, 1.0);
    assert!(!rerelease.round_images_down);
    set(&mut vars, RuleSetId::Quake3, "r_picmip", "-1");
    assert_eq!(image_settings(&vars, RuleSetId::Quake3).unwrap().picmip, 0);
    assert_eq!(
        image_settings(&vars, RuleSetId::Quake2Rerelease)
            .unwrap()
            .picmip,
        0
    );
    assert!(image_settings(&vars, RuleSetId::Quake2).is_err());
    set(&mut vars, RuleSetId::Quake3, "r_picmip", "40");
    assert_eq!(image_settings(&vars, RuleSetId::Quake3).unwrap().picmip, 16);
    assert_eq!(
        image_settings(&vars, RuleSetId::Quake2Rerelease)
            .unwrap()
            .picmip,
        31
    );
    set(&mut vars, RuleSetId::Quake3, "r_picmip", "0");
    set(&mut vars, RuleSetId::Quake2, "intensity", "6");
    assert_eq!(
        image_settings(&vars, RuleSetId::Quake2Rerelease)
            .unwrap()
            .intensity,
        5.0
    );
    assert_eq!(
        image_settings(&vars, RuleSetId::Quake2).unwrap().intensity,
        6.0
    );
    set(&mut vars, RuleSetId::Quake2, "gl_round_down", "0.5");
    assert!(
        !image_settings(&vars, RuleSetId::Quake2Rerelease)
            .unwrap()
            .round_images_down
    );
}

#[test]
fn subdivision_policy_is_global_and_current_until_its_selected_latch_is_applied() {
    use qa_console::{cvars::RuntimePolicy, pending::PendingSettings};
    for context in RuleSetId::ALL {
        let mut vars = vars(context);
        assert_eq!(
            world_load_options(&vars, None)
                .unwrap()
                .geometry
                .patch_subdivisions,
            4.0
        );
        let subdivision = vars.bind("r_subdivisions", Context::default()).unwrap();
        vars.set_latch_active(subdivision.canonical(), true);
        vars.set_runtime_policy(RuntimePolicy {
            initialized: true,
            server_active: true,
        });
        vars.write(subdivision, "2").unwrap();
        assert_eq!(
            world_load_options(&vars, None)
                .unwrap()
                .geometry
                .patch_subdivisions,
            4.0
        );
        let game = vars.bind("g_gametype", Context::default()).unwrap();
        vars.write(game, "4").unwrap();
        let selected = PendingSettings::capture(&vars, [subdivision]);
        selected.apply(&mut vars).unwrap();
        assert_eq!(
            world_load_options(&vars, None)
                .unwrap()
                .geometry
                .patch_subdivisions,
            2.0
        );
        assert!(vars.latched(game).unwrap().is_some());
        for value in ["0", "-1.5"] {
            vars.force_write(subdivision, value).unwrap();
            assert_eq!(
                world_load_options(&vars, None)
                    .unwrap()
                    .geometry
                    .patch_subdivisions,
                qa_console::numbers::number(value, RuleSetId::Quake3)
            );
        }
        vars.force_write(subdivision, "nan").unwrap();
        assert!(world_load_options(&vars, None).is_err());
    }
}

#[test]
fn world_lighting_reads_native_integer_requests_across_console_contexts() {
    for context in RuleSetId::ALL {
        let mut vars = vars(context);
        let lighting = world_load_options(&vars, None).unwrap().lighting;
        assert_eq!(lighting.map_overbright(), 2);
        assert_eq!(lighting.renderer_overbright(), 0);
        set(
            &mut vars,
            RuleSetId::Quake3,
            "r_mapOverBrightBits",
            "3.9suffix",
        );
        set(&mut vars, RuleSetId::Quake3, "r_overBrightBits", "99");
        let lighting = world_load_options(&vars, None).unwrap().lighting;
        assert_eq!(lighting.map_overbright(), 3);
        assert_eq!(lighting.renderer_overbright(), 0);
        assert_eq!(lighting.shift(), 3);
        for value in ["-1", "16"] {
            set(&mut vars, RuleSetId::Quake3, "r_mapOverBrightBits", value);
            assert!(world_load_options(&vars, None).is_err());
        }
        set(&mut vars, RuleSetId::Quake3, "r_mapOverBrightBits", "15");
        assert_eq!(
            world_load_options(&vars, None).unwrap().lighting.shift(),
            15
        );
    }
}

#[test]
fn effective_output_overbright_follows_actual_configuration_without_changing_requests() {
    use qa_core::presentation::RendererConfiguration;
    let mut vars = vars(RuleSetId::Quake3);
    set(&mut vars, RuleSetId::Quake3, "r_mapOverBrightBits", "4");
    set(&mut vars, RuleSetId::Quake3, "r_overBrightBits", "99");
    let mut configuration = RendererConfiguration::default();
    configuration.display.fullscreen = true;
    configuration.display.color_bits = 24;
    assert_eq!(
        world_load_options(&vars, Some(&configuration))
            .unwrap()
            .lighting
            .renderer_overbright(),
        0
    );
    configuration.hardware_gamma = true;
    for (color_bits, expected) in [(24, 2), (16, 1)] {
        configuration.display.color_bits = color_bits;
        let lighting = world_load_options(&vars, Some(&configuration))
            .unwrap()
            .lighting;
        assert_eq!(lighting.renderer_overbright(), expected);
        assert_eq!(lighting.shift(), (4 - expected) as i8);
    }
    configuration.display.fullscreen = false;
    assert_eq!(
        world_load_options(&vars, Some(&configuration))
            .unwrap()
            .lighting
            .renderer_overbright(),
        0
    );
    let output = vars.bind("r_overBrightBits", Context::default()).unwrap();
    assert_eq!(vars.integer_view(output).unwrap(), 99);
    set(&mut vars, RuleSetId::Quake3, "r_overBrightBits", "-4");
    configuration.display.fullscreen = true;
    assert_eq!(
        world_load_options(&vars, Some(&configuration))
            .unwrap()
            .lighting
            .renderer_overbright(),
        0
    );
}

#[test]
fn lighting_requests_stay_current_until_only_the_selected_latches_are_applied() {
    use qa_console::{cvars::RuntimePolicy, pending::PendingSettings};
    let mut vars = vars(RuleSetId::Quake2);
    let map = vars
        .bind("r_mapOverBrightBits", Context::default())
        .unwrap();
    let output = vars.bind("r_overBrightBits", Context::default()).unwrap();
    for view in [map, output] {
        vars.set_latch_active(view.canonical(), true);
    }
    vars.set_runtime_policy(RuntimePolicy {
        initialized: true,
        server_active: true,
    });
    vars.write(map, "1").unwrap();
    vars.write(output, "2").unwrap();
    let game = vars.bind("g_gametype", Context::default()).unwrap();
    vars.write(game, "4").unwrap();
    assert_eq!(
        world_load_options(&vars, None)
            .unwrap()
            .lighting
            .map_overbright(),
        2
    );
    PendingSettings::capture(&vars, [map, output])
        .apply(&mut vars)
        .unwrap();
    let lighting = world_load_options(&vars, None).unwrap().lighting;
    assert_eq!(
        (lighting.map_overbright(), lighting.renderer_overbright()),
        (1, 0)
    );
    assert_eq!(vars.integer_view(output).unwrap(), 2);
    assert!(vars.latched(game).unwrap().is_some());
}

#[test]
fn material_flags_read_one_shared_renderer_policy_in_every_console_context() {
    let default = qa_render::material::MaterialFinishPolicy::default();
    for source in RuleSetId::ALL {
        let mut vars = vars(source);
        assert_eq!(material_finish(&vars).unwrap(), default);
        for (name, text) in [
            ("r_detailtextures", "0"),
            ("r_vertexLight", "1"),
            ("r_fullbright", "1"),
        ] {
            let view = vars
                .bind(
                    name,
                    Context {
                        source: RuleSetId::Quake3,
                        ..Context::default()
                    },
                )
                .unwrap();
            vars.force_write(view, text).unwrap();
        }
        let finish = material_finish(&vars).unwrap();
        assert!(!finish.detail_textures);
        assert!(finish.vertex_light && finish.fullbright);
    }
}
