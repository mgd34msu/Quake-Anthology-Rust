use qa_app::render_settings::image_settings;
use qa_console::{
    cvars::Cvars,
    views::{Context, RuleSetId},
};

fn vars(source: RuleSetId) -> Cvars {
    Cvars::with_context(Context {
        source,
        ..Context::default()
    })
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
