use qa_console::{
    cvars::{Cvars, WriteError},
    views::{Context, Role, RuleSetId},
};

fn context(source: RuleSetId) -> Context {
    Context {
        source,
        role: Role::Game,
        ..Context::default()
    }
}

#[test]
fn native_reset_uses_source_and_alias_defaults_without_resetting_other_sources() {
    for source in RuleSetId::ALL {
        for name in ["sensitivity", "gamma"] {
            let mut cvars = Cvars::new().unwrap();
            let context = context(source);
            let view = cvars.bind(name, context).unwrap();
            if !cvars.default_available(view.canonical(), source) {
                continue;
            }
            let original = cvars.read(view).unwrap().as_str().to_owned();
            cvars.force_write(view, "0.8").unwrap();
            cvars.reset_view(view, true).unwrap();
            assert_eq!(cvars.read(view).unwrap().as_str(), original);
            if name == "gamma" {
                cvars.set_text(view.canonical(), "0").unwrap();
                assert!(cvars.read(view).is_err());
                cvars.reset_view(view, true).unwrap();
                assert_eq!(cvars.read(view).unwrap().as_str(), original);
            }
        }
    }
    let mut cvars = Cvars::new().unwrap();
    let q3 = cvars
        .bind("sensitivity", context(RuleSetId::Quake3))
        .unwrap();
    let q1 = cvars
        .bind("sensitivity", context(RuleSetId::Quake))
        .unwrap();
    cvars.force_write(q3, "9").unwrap();
    cvars.reset_view(q3, true).unwrap();
    assert_eq!(cvars.read(q3).unwrap().as_str(), "5");
    assert_eq!(cvars.read(q1).unwrap().as_str(), "5");
    cvars.reset_view(q1, true).unwrap();
    assert_eq!(cvars.read(q3).unwrap().as_str(), "3");
    assert_eq!(cvars.read(q1).unwrap().as_str(), "3");
}

#[test]
fn native_reset_preserves_protection_latches_and_equal_value_short_circuit() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake3);
    let readonly = cvars
        .register("_qa_reset_rom", Some("1"), 64, context)
        .unwrap();
    cvars.force_write(readonly, "2").unwrap();
    assert_eq!(cvars.reset_view(readonly, false), Err(WriteError::ReadOnly));
    assert_eq!(cvars.read(readonly).unwrap().as_str(), "2");
    cvars.reset_view(readonly, true).unwrap();
    assert_eq!(cvars.read(readonly).unwrap().as_str(), "1");
    let latched = cvars
        .register("_qa_reset_latch", Some("1"), 32, context)
        .unwrap();
    cvars.set_latch_active(latched.canonical(), true);
    cvars.write(latched, "2").unwrap();
    let generation = cvars.view_update_generation(latched);
    cvars.reset_view(latched, true).unwrap();
    assert_eq!(cvars.view_update_generation(latched), generation);
    assert_eq!(cvars.latched(latched).unwrap().unwrap().as_str(), "2");
    cvars.apply_latches().unwrap();
    cvars.reset_view(latched, false).unwrap();
    assert_eq!(cvars.read(latched).unwrap().as_str(), "2");
    assert_eq!(cvars.latched(latched).unwrap().unwrap().as_str(), "1");
    cvars.reset_view(latched, true).unwrap();
    assert_eq!(cvars.read(latched).unwrap().as_str(), "1");
    assert!(cvars.latched(latched).unwrap().is_none());
}

#[test]
fn native_registration_fills_an_unresolved_source_default_without_overwriting_values() {
    let mut cvars = Cvars::new().unwrap();
    let rr = context(RuleSetId::Quake2Rerelease);
    let classic = context(RuleSetId::Quake2);
    let unresolved = cvars.bind("maxentities", rr).unwrap();
    assert!(!cvars.default_available(unresolved.canonical(), rr.source));
    let query = cvars.register("maxentities", None, 32, rr).unwrap();
    assert_eq!(query.canonical(), unresolved.canonical());
    assert!(!cvars.default_available(query.canonical(), rr.source));
    assert!(matches!(
        cvars.register("_qa_missing_default", None, 0, rr),
        Err(WriteError::MissingDefault)
    ));
    let old_count = cvars.entries().count();
    let view = cvars.register("maxentities", Some("8192"), 32, rr).unwrap();
    assert_eq!(view.canonical(), unresolved.canonical());
    assert_eq!(cvars.entries().count(), old_count);
    assert_eq!(cvars.read(view).unwrap().as_str(), "8192");
    assert_eq!(cvars.numeric(view).unwrap(), 8192.0);
    assert!(cvars.native_default_available(view.canonical(), rr.source));
    let other = cvars.bind("maxentities", classic).unwrap();
    assert_eq!(cvars.read(other).unwrap().as_str(), "1024");
    let generation = cvars.view_generation(view);
    cvars.register("MAXENTITIES", Some("4096"), 32, rr).unwrap();
    assert_eq!(cvars.view_generation(view), generation);
    assert_eq!(cvars.read(view).unwrap().as_str(), "8192");
    cvars.force_write(view, "2048").unwrap();
    cvars.reset(view.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), "8192");
    let mut cvars = Cvars::new().unwrap();
    let user = cvars.bind("maxentities", rr).unwrap();
    cvars.force_write(user, "4096").unwrap();
    cvars.register("maxentities", Some("8192"), 32, rr).unwrap();
    assert_eq!(cvars.read(user).unwrap().as_str(), "4096");
    cvars.reset(user.canonical());
    assert_eq!(cvars.read(user).unwrap().as_str(), "8192");
    let empty = cvars
        .register("_qa_explicit_empty", Some(""), 0, rr)
        .unwrap();
    assert!(cvars.default_available(empty.canonical(), rr.source));
    assert_eq!(cvars.read(empty).unwrap().as_str(), "");
}

#[test]
fn module_names_share_handles_and_listing_in_every_source() {
    let mut cvars = Cvars::new().unwrap();
    let count = cvars.entries().count();
    let first = cvars
        .register(
            "_QA_ModuleSpeed",
            Some("12.5"),
            0,
            context(RuleSetId::Quake2Rerelease),
        )
        .unwrap();
    for source in RuleSetId::ALL {
        let view = cvars.bind("_qa_modulespeed", context(source)).unwrap();
        assert_eq!(view.canonical(), first.canonical());
        assert_eq!(cvars.read(view).unwrap().as_str(), "12.5");
        assert_eq!(cvars.numeric(view).unwrap().to_bits(), 12.5f32.to_bits());
        assert!(cvars.native_default_available(view.canonical(), source));
    }
    cvars.write(first, "27.25").unwrap();
    for source in RuleSetId::ALL {
        assert_eq!(
            cvars.value_in(first.canonical(), source).to_bits(),
            27.25f32.to_bits()
        );
        assert_eq!(cvars.integer_in(first.canonical(), source), 27);
    }
    let rows = cvars
        .entries()
        .filter(|(handle, _, _, _, _)| *handle == first.canonical())
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "_QA_ModuleSpeed");
    assert_eq!(rows[0].2, "27.25");
    assert!(rows[0].3.is_none());
    assert_eq!(cvars.entries().count(), count + 1);
}

#[test]
fn registration_preserves_catalog_aliases_defaults_and_written_values() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake2);
    let before = cvars.bind("fov", context).unwrap();
    let initial = cvars.read(before).unwrap().as_str().to_owned();
    let view = cvars.register("FOV", Some("13"), 0, context).unwrap();
    assert_eq!(view.canonical(), before.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), initial);
    cvars.write(view, "117").unwrap();
    let generation = cvars.view_generation(view);
    let after = cvars.register("fov", Some("14"), 0, context).unwrap();
    assert_eq!(cvars.read(after).unwrap().as_str(), "117");
    assert_eq!(cvars.view_generation(after), generation);
    let count = cvars.entries().count();
    assert_eq!(
        cvars
            .register("cg_fov", Some("90"), 0, context)
            .unwrap()
            .canonical(),
        view.canonical()
    );
    assert_eq!(cvars.entries().count(), count);
}

#[test]
fn module_values_use_the_existing_latch_flag_and_reset_paths() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::Quake3);
    let view = cvars
        .register("_qa_latched", Some("10"), 32, context)
        .unwrap();
    let generation = cvars.view_generation(view);
    cvars.server_active = true;
    cvars.write(view, "20").unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "10");
    assert_eq!(cvars.view_generation(view), generation);
    cvars.apply_latches().unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "20");
    assert!(cvars.view_generation(view) > generation);
    cvars.reset(view.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), "10");
    cvars.full_set(view, "30", 64).unwrap();
    assert_eq!(cvars.write(view, "40"), Err(WriteError::ReadOnly));
    let generation = cvars.view_generation(view);
    let merged = cvars
        .register("_QA_LATCHED", Some("99"), 1, context)
        .unwrap();
    assert_eq!(cvars.flags(merged), 65);
    assert_eq!(cvars.read(merged).unwrap().as_str(), "30");
    assert!(cvars.view_generation(merged) > generation);
    let generation = cvars.view_generation(merged);
    cvars
        .register("_qa_latched", Some("88"), 1, context)
        .unwrap();
    assert_eq!(cvars.view_generation(merged), generation);
    cvars.full_set(view, "50", 0).unwrap();
    cvars.write(view, "60").unwrap();
    assert_eq!(cvars.read(view).unwrap().as_str(), "60");
}

#[test]
fn bad_info_and_capacity_failures_keep_live_rows_and_existing_bindings() {
    let mut cvars = Cvars::new().unwrap();
    let context = context(RuleSetId::QuakeWorld);
    let initial = cvars.entries().count();
    for (name, default) in [("", "1"), ("bad;name", "1"), ("_qa_bad", "bad\\info")] {
        assert!(cvars.register(name, Some(default), 6, context).is_err());
        assert_eq!(cvars.entries().count(), initial);
    }
    let first = cvars
        .register("_qa_module_0", Some("7"), 0, context)
        .unwrap();
    for index in 1..1024 {
        cvars
            .register(&format!("_qa_module_{index}"), Some("0"), 0, context)
            .unwrap();
    }
    assert!(matches!(
        cvars.register("_qa_capacity", Some("0"), 0, context),
        Err(WriteError::Capacity)
    ));
    assert_eq!(cvars.entries().count(), initial + 1024);
    assert!(cvars.bind("_qa_capacity", context).is_none());
    assert_eq!(
        cvars
            .register("_QA_MODULE_0", Some("9"), 0, context)
            .unwrap()
            .canonical(),
        first.canonical()
    );
    assert_eq!(cvars.read(first).unwrap().as_str(), "7");
}
