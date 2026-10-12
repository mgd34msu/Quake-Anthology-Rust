use qa_console::{
    cvars::Cvars,
    movement::MovementHandles,
    views::{Context, Role},
};
use qa_core::primitives::{MovementTuning, RuleSetId};

#[test]
fn registered_defaults_are_selected_by_movement_role_without_name_lookups() {
    let vars = Cvars::new().unwrap();
    let handles = MovementHandles::bind(&vars).unwrap();
    vars.reset_lookup_count();
    for rules in RuleSetId::ALL {
        let tuning = handles.resolve(&vars, rules, MovementTuning::default());
        assert_eq!(tuning.gravity, Some(800.0));
        match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => {
                assert_eq!(tuning.max_speed, Some(320.0));
                assert_eq!(tuning.friction, Some(4.0));
                assert_eq!(tuning.accelerate, Some(10.0));
                assert_eq!(tuning.water_accelerate, Some(10.0));
                assert_eq!(tuning.water_friction, Some(4.0));
            }
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease | RuleSetId::Quake3 => {
                assert_eq!(tuning.friction, None);
                assert_eq!(tuning.accelerate, None);
                assert_eq!(tuning.water_accelerate, None);
                assert_eq!(tuning.water_friction, None);
                assert_eq!(
                    tuning.max_speed,
                    (rules == RuleSetId::Quake3).then_some(320.0)
                );
            }
        }
        assert_eq!(
            tuning.stop_speed,
            matches!(
                rules,
                RuleSetId::Quake | RuleSetId::QuakeWorld | RuleSetId::Quake2Rerelease
            )
            .then_some(100.0)
        );
        assert_eq!(
            tuning.spectator_max_speed,
            (rules == RuleSetId::QuakeWorld).then_some(500.0)
        );
        assert_eq!(
            tuning.air_accelerate,
            match rules {
                RuleSetId::QuakeWorld => Some(0.7),
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => Some(0.0),
                RuleSetId::Quake | RuleSetId::Quake3 => None,
            }
        );
        assert_eq!(tuning.fixed_step_ms, 0);
    }
    assert_eq!(vars.lookup_count(), 0);
}

#[test]
fn aliases_fill_engine_defaults_and_preserve_native_player_gravity_and_speed() {
    let mut vars = Cvars::new().unwrap();
    let handles = MovementHandles::bind(&vars).unwrap();
    let context = Context {
        source: RuleSetId::Quake,
        role: Role::Game,
        ..Context::default()
    };
    let gravity = vars.bind("sv_gravity", context).unwrap();
    let speed = vars.bind("sv_maxspeed", context).unwrap();
    let base = MovementTuning {
        gravity: Some(137.0),
        max_speed: Some(416.0),
        gravity_multiplier: 0.5,
        speed_multiplier: 2.0,
        fixed_step_ms: 17,
        no_step: true,
        ..MovementTuning::default()
    };
    for rules in RuleSetId::ALL {
        let tuning = handles.resolve(&vars, rules, base);
        assert_eq!(tuning.gravity, base.gravity);
        assert_eq!(tuning.max_speed, base.max_speed);
        assert_eq!(tuning.gravity_multiplier, base.gravity_multiplier);
        assert_eq!(tuning.speed_multiplier, base.speed_multiplier);
        assert_eq!(tuning.fixed_step_ms, 17);
        assert!(tuning.no_step);
    }
    vars.force_write(gravity, "612.5").unwrap();
    vars.force_write(speed, "235.25").unwrap();
    vars.reset_lookup_count();
    for rules in RuleSetId::ALL {
        let tuning = handles.resolve(&vars, rules, base);
        assert_eq!(tuning.gravity, base.gravity);
        assert_eq!(tuning.max_speed, base.max_speed);
        assert_eq!(tuning.gravity_multiplier, 0.5);
        assert_eq!(tuning.speed_multiplier, 2.0);
        let engine = handles.resolve(&vars, rules, MovementTuning::default());
        assert_eq!(engine.gravity, Some(612.5));
        assert_eq!(engine.max_speed, Some(235.25));
    }
    assert_eq!(vars.lookup_count(), 0);
    vars.reset(gravity.canonical());
    vars.reset(speed.canonical());
    for rules in RuleSetId::ALL {
        let tuning = handles.resolve(&vars, rules, base);
        assert_eq!(tuning.gravity, base.gravity);
        assert_eq!(tuning.max_speed, base.max_speed);
        let engine = handles.resolve(&vars, rules, MovementTuning::default());
        assert_eq!(engine.gravity, Some(800.0));
        assert_eq!(
            engine.max_speed,
            matches!(
                rules,
                RuleSetId::Quake | RuleSetId::QuakeWorld | RuleSetId::Quake3
            )
            .then_some(320.0)
        );
    }
}

#[test]
fn netquake_water_inherits_current_ordinary_settings_until_overridden() {
    let mut vars = Cvars::new().unwrap();
    let handles = MovementHandles::bind(&vars).unwrap();
    let accelerate = vars.find("sv_accelerate").unwrap();
    let friction = vars.find("sv_friction").unwrap();
    let water_accelerate = vars.find("sv_wateraccelerate").unwrap();
    let water_friction = vars.find("sv_waterfriction").unwrap();
    vars.set_text(accelerate, "6.5").unwrap();
    vars.set_text(friction, "2.75").unwrap();
    let resolve = |vars: &Cvars| handles.resolve(vars, RuleSetId::Quake, MovementTuning::default());
    let inherited = resolve(&vars);
    assert_eq!(inherited.water_accelerate, Some(6.5));
    assert_eq!(inherited.water_friction, Some(2.75));
    vars.set_text(water_accelerate, "9.25").unwrap();
    vars.set_text(water_friction, "1.5").unwrap();
    let explicit = resolve(&vars);
    assert_eq!(explicit.water_accelerate, Some(9.25));
    assert_eq!(explicit.water_friction, Some(1.5));
    vars.reset(water_accelerate);
    vars.reset(water_friction);
    assert_eq!(resolve(&vars), inherited);
}

#[test]
fn unsupported_catalog_defaults_stay_absent_until_explicitly_set() {
    let mut vars = Cvars::new().unwrap();
    let handles = MovementHandles::bind(&vars).unwrap();
    let spectator = vars.find("sv_spectatormaxspeed").unwrap();
    let friction = vars.find("sv_friction").unwrap();
    let resolve =
        |vars: &Cvars| handles.resolve(vars, RuleSetId::Quake2, MovementTuning::default());
    assert_eq!(resolve(&vars).spectator_max_speed, None);
    assert_eq!(resolve(&vars).friction, None);
    vars.set_text(spectator, "740").unwrap();
    vars.set_text(friction, "3.5").unwrap();
    let explicit = resolve(&vars);
    assert_eq!(explicit.spectator_max_speed, Some(740.0));
    assert_eq!(explicit.friction, Some(3.5));
    vars.reset(spectator);
    vars.reset(friction);
    assert_eq!(resolve(&vars).spectator_max_speed, None);
    assert_eq!(resolve(&vars).friction, None);
}

#[test]
fn fixed_step_uses_cached_integers_and_native_settings_boundary() {
    let mut vars = Cvars::new().unwrap();
    let handles = MovementHandles::bind(&vars).unwrap();
    let fixed = vars.find("pmove_fixed").unwrap();
    let msec = vars.find("pmove_msec").unwrap();
    let native = MovementTuning {
        fixed_step_ms: 17,
        ..MovementTuning::default()
    };
    let resolve =
        |vars: &Cvars| handles.resolve(vars, RuleSetId::Quake3, MovementTuning::default());
    assert_eq!(resolve(&vars).fixed_step_ms, 0);
    assert_eq!(
        handles
            .resolve(&vars, RuleSetId::Quake3, native)
            .fixed_step_ms,
        17
    );
    vars.set_text(fixed, "1").unwrap();
    assert_eq!(resolve(&vars).fixed_step_ms, 8);
    for (text, expected) in [("-1", 8), ("16.9", 16), ("100", 33)] {
        vars.set_text(msec, text).unwrap();
        vars.reset_lookup_count();
        assert_eq!(resolve(&vars).fixed_step_ms, expected);
        assert_eq!(
            handles
                .resolve(&vars, RuleSetId::Quake3, native)
                .fixed_step_ms,
            17
        );
        assert_eq!(vars.lookup_count(), 0);
    }
    vars.set_text(fixed, "0").unwrap();
    assert_eq!(resolve(&vars).fixed_step_ms, 0);
    assert_eq!(
        handles
            .resolve(&vars, RuleSetId::Quake3, native)
            .fixed_step_ms,
        17
    );
    vars.reset(fixed);
    vars.reset(msec);
    assert_eq!(resolve(&vars).fixed_step_ms, 0);
}
