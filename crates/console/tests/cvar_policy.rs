use qa_console::{
    cvars::{Cvars, RuntimePolicy, WriteError},
    views::{Context, Role, RuleSetId},
};

fn context(source: RuleSetId) -> Context {
    Context {
        source,
        role: Role::Game,
        ..Context::default()
    }
}

fn running(server_active: bool) -> RuntimePolicy {
    RuntimePolicy {
        initialized: true,
        server_active,
    }
}

#[test]
fn rejected_storage_write_preserves_pending_value_and_native_generation() {
    let mut vars = Cvars::new().unwrap();
    let view = vars
        .register(
            "_qa_storage_latch",
            Some("1"),
            32,
            context(RuleSetId::Quake3),
        )
        .unwrap();
    let native = vars.native_view(view).unwrap();
    vars.set_runtime_policy(running(false));
    vars.write(view, "2").unwrap();
    let generation = vars.native_snapshot(native, view.context()).unwrap().1;
    assert_eq!(
        vars.set_text(
            view.canonical(),
            &"x".repeat(qa_console::text::MAX_TEXT + 1)
        ),
        Err(WriteError::Conversion(
            qa_console::conversion::Error::TextTooLong
        ))
    );
    assert_eq!(vars.read(view).unwrap().as_str(), "1");
    assert_eq!(vars.latched(view).unwrap().unwrap().as_str(), "2");
    assert_eq!(
        vars.native_snapshot(native, view.context()).unwrap().1,
        generation
    );
    vars.set_text(view.canonical(), "é\0raw").unwrap();
    assert_eq!(vars.read(view).unwrap().as_str(), "é\0raw");
    assert!(vars.latched(view).unwrap().is_none());
}

#[test]
fn host_policy_closes_the_initial_setup_window_without_changing_values() {
    let mut vars = Cvars::new().unwrap();
    assert_eq!(vars.runtime_policy(), RuntimePolicy::default());
    let q3 = context(RuleSetId::Quake3);
    let init = vars.register("_qa_policy_init", Some("1"), 16, q3).unwrap();
    vars.write(init, "2").unwrap();
    let revision = vars.revision();
    vars.set_runtime_policy(running(false));
    assert_eq!(vars.revision(), revision);
    assert_eq!(vars.runtime_policy(), running(false));
    assert_eq!(vars.write(init, "3"), Err(WriteError::InitOnly));
    assert_eq!(vars.read(init).unwrap().as_str(), "2");
    // Native force setters retain the same publication/conversion path.
    vars.force_write(init, "3").unwrap();
    assert_eq!(vars.read(init).unwrap().as_str(), "3");
}

#[test]
fn initialized_q3_latches_before_cheat_checks_without_an_active_server() {
    let mut vars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let cheats = vars.bind("sv_cheats", q3).unwrap();
    vars.force_write(cheats, "0").unwrap();
    let latched = vars
        .register("_qa_policy_latched_cheat", Some("1"), 32 | 512, q3)
        .unwrap();
    let native = vars.native_view(latched).unwrap();
    vars.set_runtime_policy(running(false));
    vars.write(latched, "2").unwrap();
    assert_eq!(vars.read(latched).unwrap().as_str(), "1");
    assert_eq!(vars.latched(latched).unwrap().unwrap().as_str(), "2");
    assert_eq!(
        vars.native_snapshot(native, latched.context()).unwrap().1,
        2
    );
    let generation = vars.view_update_generation(latched);
    vars.write(latched, "2").unwrap();
    assert_eq!(vars.view_update_generation(latched), generation);
    assert_eq!(
        vars.native_snapshot(native, latched.context()).unwrap().1,
        2
    );
    vars.apply_latch(latched).unwrap();
    assert_eq!(vars.read(latched).unwrap().as_str(), "2");
    assert!(vars.latched(latched).unwrap().is_none());
    assert_eq!(
        vars.native_snapshot(native, latched.context()).unwrap().1,
        3
    );
}

#[test]
fn q3_rom_and_init_precede_latch_but_an_unchanged_value_precedes_protection() {
    let mut vars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let readonly = vars
        .register("_qa_policy_readonly", Some("1"), 64 | 32 | 512, q3)
        .unwrap();
    let init = vars
        .register("_qa_policy_init_latch", Some("1"), 16 | 32 | 512, q3)
        .unwrap();
    vars.set_runtime_policy(running(false));
    assert_eq!(vars.write(readonly, "2"), Err(WriteError::ReadOnly));
    assert_eq!(vars.write(init, "2"), Err(WriteError::InitOnly));
    assert!(vars.latched(readonly).unwrap().is_none());
    assert!(vars.latched(init).unwrap().is_none());
    let revision = vars.revision();
    vars.write(readonly, "1").unwrap();
    vars.write(init, "1").unwrap();
    assert_eq!(vars.revision(), revision);
}

#[test]
fn equal_q3_writes_preserve_a_latch_and_forced_different_values_apply_it_once() {
    let mut vars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let view = vars
        .register("_qa_policy_equal_latch", Some("1"), 32, q3)
        .unwrap();
    vars.set_runtime_policy(running(false));
    let native = vars.native_view(view).unwrap();
    vars.write(view, "2").unwrap();
    let generation = vars.view_update_generation(view);
    vars.force_write(view, "1").unwrap();
    vars.write(view, "1").unwrap();
    assert_eq!(vars.view_update_generation(view), generation);
    assert_eq!(vars.latched(view).unwrap().unwrap().as_str(), "2");
    assert_eq!(vars.native_snapshot(native, view.context()).unwrap().1, 2);
    // Matching a pending string must not bypass a newly added ROM flag.
    vars.register("_qa_policy_equal_latch", None, 64, q3)
        .unwrap();
    assert_eq!(vars.write(view, "2"), Err(WriteError::ReadOnly));
    vars.force_write(view, "2").unwrap();
    assert_eq!(vars.read(view).unwrap().as_str(), "2");
    assert!(vars.latched(view).unwrap().is_none());
    assert_eq!(vars.native_snapshot(native, view.context()).unwrap().1, 3);
}

#[test]
fn q2_latches_depend_on_live_server_state_and_q3_uses_native_latch_semantics() {
    let mut vars = Cvars::new().unwrap();
    let q2 = vars
        .register("_qa_policy_q2", Some("1"), 32, context(RuleSetId::Quake2))
        .unwrap();
    let q3 = vars
        .register("_qa_policy_q3", Some("1"), 32, context(RuleSetId::Quake3))
        .unwrap();
    // Eager catalog admission must not latch initial configuration.
    vars.write(q3, "2").unwrap();
    assert_eq!(vars.read(q3).unwrap().as_str(), "2");
    assert!(vars.latched(q3).unwrap().is_none());
    vars.set_runtime_policy(running(false));
    vars.write(q2, "2").unwrap();
    assert_eq!(vars.read(q2).unwrap().as_str(), "2");
    vars.write(q3, "3").unwrap();
    assert_eq!(vars.read(q3).unwrap().as_str(), "2");
    assert_eq!(vars.latched(q3).unwrap().unwrap().as_str(), "3");
    vars.set_runtime_policy(running(true));
    vars.write(q2, "3").unwrap();
    assert_eq!(vars.read(q2).unwrap().as_str(), "2");
    assert_eq!(vars.latched(q2).unwrap().unwrap().as_str(), "3");
    vars.force_write(q2, "4").unwrap();
    assert_eq!(vars.read(q2).unwrap().as_str(), "4");
    assert!(vars.latched(q2).unwrap().is_none());
}

#[test]
fn cheat_permission_reads_the_current_cached_cvar_in_the_same_command_phase() {
    let mut vars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let cheats = vars.bind("sv_cheats", q3).unwrap();
    let protected = vars
        .register("_qa_policy_live_cheat", Some("1"), 512, q3)
        .unwrap();
    vars.set_runtime_policy(running(true));
    vars.force_write(cheats, "0").unwrap();
    assert_eq!(vars.write(protected, "2"), Err(WriteError::Cheats));
    // No policy refresh separates these writes: a host-side cached bool
    // would still incorrectly reject the protected write.
    vars.force_write(cheats, "1").unwrap();
    vars.write(protected, "2").unwrap();
    assert_eq!(vars.read(protected).unwrap().as_str(), "2");
    vars.force_write(cheats, "0").unwrap();
    assert_eq!(vars.write(protected, "3"), Err(WriteError::Cheats));
    // Q3 consults the cached native integer, not a nonzero fractional float.
    vars.force_write(cheats, "0.5").unwrap();
    assert_eq!(vars.write(protected, "3"), Err(WriteError::Cheats));
}

#[cfg(any(debug_assertions, feature = "lookup-tracking"))]
#[test]
fn policy_and_cheat_checks_use_cached_handles_without_name_lookup() {
    let mut vars = Cvars::new().unwrap();
    let q3 = context(RuleSetId::Quake3);
    let cheats = vars.bind("sv_cheats", q3).unwrap();
    let protected = vars
        .register("_qa_policy_cached_cheat", Some("1"), 512, q3)
        .unwrap();
    vars.reset_lookup_count();
    vars.set_runtime_policy(running(true));
    vars.force_write(cheats, "1").unwrap();
    vars.write(protected, "2").unwrap();
    vars.force_write(cheats, "0").unwrap();
    assert_eq!(vars.write(protected, "3"), Err(WriteError::Cheats));
    assert_eq!(vars.lookup_count(), 0);
}
