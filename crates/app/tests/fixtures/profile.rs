use super::*;
use qa_console::views::Role;
use serde_json::json;

#[test]
fn saved_custom_cvars_share_user_registration_and_native_default_adoption() {
    let mut console = Console::<Runtime>::new(Context::default()).unwrap();
    let mut report = Import::default();
    let count = console.cvars.entries().count();
    entries(
        &mut console,
        &json!([
            {"name": "_QA_SavedModSetting", "value": "42"},
            {"name": "FOV", "value": "105"}
        ]),
        RuleSetId::Quake3,
        &mut report,
    )
    .unwrap();
    assert_eq!((report.cvars, report.unsupported), (2, 0));
    assert_eq!(console.cvars.entries().count(), count + 1);
    let user = console
        .cvars
        .bind("_qa_savedmodsetting", Context::default())
        .unwrap();
    assert_eq!(console.cvars.read(user).unwrap().as_str(), "42");
    assert_eq!(console.cvars.flags(user), 128);
    let native = console
        .cvars
        .register(
            "_qa_savedmodsetting",
            Some("3"),
            1,
            Context {
                role: Role::Game,
                ..Context::default()
            },
        )
        .unwrap();
    assert_eq!(native.canonical(), user.canonical());
    assert_eq!(console.cvars.read(native).unwrap().as_str(), "42");
    assert_eq!(console.cvars.flags(native), 1);
    console.cvars.reset_view(native, true).unwrap();
    assert_eq!(console.cvars.read(native).unwrap().as_str(), "3");
    let fov = console.cvars.find("cg_fov").unwrap();
    assert_eq!(console.cvars.text(fov), "105");
}

#[test]
fn profile_writes_reuse_alias_views_and_count_bounded_or_protected_failures() {
    let mut console = Console::<Runtime>::new(Context::default()).unwrap();
    let q1 = Context {
        source: RuleSetId::Quake,
        ..Context::default()
    };
    let gamma = console.cvars.bind("gamma", q1).unwrap();
    let mut report = Import::default();
    put(&mut console, q1.source, "gamma", "0.5", &mut report);
    assert_eq!(console.cvars.read(gamma).unwrap().as_str(), "0.500000");
    assert_eq!(gamma.canonical(), console.cvars.find("r_gamma").unwrap());
    let q3 = Context::default();
    let protected = console
        .cvars
        .register("_qa_profile_rom", Some("original"), 64, q3)
        .unwrap();
    put(
        &mut console,
        q3.source,
        "_qa_profile_rom",
        "replacement",
        &mut report,
    );
    assert_eq!(console.cvars.read(protected).unwrap().as_str(), "original");
    for index in 1..1024 {
        console
            .cvars
            .register(&format!("_qa_profile_{index}"), Some("0"), 0, q3)
            .unwrap();
    }
    let count = console.cvars.entries().count();
    put(
        &mut console,
        q3.source,
        "_qa_profile_overflow",
        "1",
        &mut report,
    );
    assert_eq!((report.cvars, report.unsupported), (1, 2));
    assert_eq!(console.cvars.entries().count(), count);
    assert!(console.cvars.bind("_qa_profile_overflow", q3).is_none());
    put(&mut console, q1.source, "FOV", "110", &mut report);
    assert_eq!(
        console.cvars.text(console.cvars.find("cg_fov").unwrap()),
        "110"
    );
    assert_eq!((report.cvars, report.unsupported), (2, 2));
}

#[test]
fn input_handle_native_role_defaults_and_explicit_numeric_truth_share_cvars() {
    let mut vars = Cvars::new().unwrap();
    let handles = InputHandles::load(&vars).unwrap();
    vars.reset_lookup_count();
    let q1 = handles.policy(&vars, RuleSetId::Quake);
    let q3 = handles.policy(&vars, RuleSetId::Quake3);
    assert!(!q1.always_run);
    assert!(q3.always_run);
    assert_eq!(q1.rules, RuleSetId::Quake);
    assert_eq!(q3.rules, RuleSetId::Quake3);
    assert_eq!(vars.lookup_count(), 0);
    let run = vars.find("cl_run").unwrap();
    let back = vars.find("cl_backspeed").unwrap();
    let forward = vars.find("cl_forwardspeed").unwrap();
    vars.set_text(run, "0.5").unwrap();
    vars.set_text(forward, "351.5").unwrap();
    vars.reset_lookup_count();
    assert!(handles.policy(&vars, RuleSetId::Quake).always_run);
    assert!(!handles.policy(&vars, RuleSetId::Quake3).always_run);
    assert_eq!(handles.policy(&vars, RuleSetId::Quake2).back_speed, 351.5);
    assert_eq!(vars.lookup_count(), 0);
    vars.set_text(back, "77.5").unwrap();
    assert_eq!(handles.policy(&vars, RuleSetId::Quake2).back_speed, 77.5);
}
