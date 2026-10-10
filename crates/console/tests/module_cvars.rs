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
fn module_names_share_handles_and_listing_in_every_source() {
    let mut cvars = Cvars::new().unwrap();
    let count = cvars.entries().count();
    let first = cvars
        .register(
            "_QA_ModuleSpeed",
            "12.5",
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
    let view = cvars.register("FOV", "13", 0, context).unwrap();
    assert_eq!(view.canonical(), before.canonical());
    assert_eq!(cvars.read(view).unwrap().as_str(), initial);
    cvars.write(view, "117").unwrap();
    let generation = cvars.view_generation(view);
    let after = cvars.register("fov", "14", 0, context).unwrap();
    assert_eq!(cvars.read(after).unwrap().as_str(), "117");
    assert_eq!(cvars.view_generation(after), generation);
    let count = cvars.entries().count();
    assert_eq!(
        cvars
            .register("cg_fov", "90", 0, context)
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
    let view = cvars.register("_qa_latched", "10", 32, context).unwrap();
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
    let merged = cvars.register("_QA_LATCHED", "99", 1, context).unwrap();
    assert_eq!(cvars.flags(merged), 65);
    assert_eq!(cvars.read(merged).unwrap().as_str(), "30");
    assert!(cvars.view_generation(merged) > generation);
    let generation = cvars.view_generation(merged);
    cvars.register("_qa_latched", "88", 1, context).unwrap();
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
        assert!(cvars.register(name, default, 6, context).is_err());
        assert_eq!(cvars.entries().count(), initial);
    }
    let first = cvars.register("_qa_module_0", "7", 0, context).unwrap();
    for index in 1..1024 {
        cvars
            .register(&format!("_qa_module_{index}"), "0", 0, context)
            .unwrap();
    }
    assert!(matches!(
        cvars.register("_qa_capacity", "0", 0, context),
        Err(WriteError::Capacity)
    ));
    assert_eq!(cvars.entries().count(), initial + 1024);
    assert!(cvars.bind("_qa_capacity", context).is_none());
    assert_eq!(
        cvars
            .register("_QA_MODULE_0", "9", 0, context)
            .unwrap()
            .canonical(),
        first.canonical()
    );
    assert_eq!(cvars.read(first).unwrap().as_str(), "7");
}
