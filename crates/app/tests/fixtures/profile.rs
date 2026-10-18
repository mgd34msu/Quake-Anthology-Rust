use super::*;
use qa_console::{cvars::Cvars, input_settings::InputHandles, views::Role};
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
        None,
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
    put(&mut console, q1.source, "gamma", "0.5", None, &mut report);
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
        None,
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
        None,
        &mut report,
    );
    assert_eq!((report.cvars, report.unsupported), (1, 2));
    assert_eq!(console.cvars.entries().count(), count);
    assert!(console.cvars.bind("_qa_profile_overflow", q3).is_none());
    put(&mut console, q1.source, "FOV", "110", None, &mut report);
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

struct ProfileFiles(std::path::PathBuf);
impl ProfileFiles {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("qa-profile-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, value: Value) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn import(
        &self,
        console: &mut Console<Runtime>,
        runtime: &mut Runtime,
        product: &str,
        policies: &[ClientPolicy],
    ) -> Result<Import, String> {
        let mount = runtime
            .vfs
            .mount_settings_directory(&self.0, -1_000_000)
            .unwrap();
        let files: Vec<_> = runtime
            .vfs
            .files_in_mount(mount)
            .map(|(file, name)| (file, name.to_vec()))
            .collect();
        let result = import_files(console, runtime, product, policies, &files);
        assert!(runtime.vfs.unmount(mount));
        result
    }
}
impl Drop for ProfileFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn canonical_player_settings_target_their_own_client_columns() {
    use qa_core::primitives::ModuleId;
    use qa_session::clients::Connection;
    let files = ProfileFiles::new("canonical-seats");
    files.write(
        "cvars/shared/canonical.json",
        json!({
            "version": 1, "dialect": "q3",
            "entries": [{"name":"sensitivity", "value":"4"}],
            "players": [
                {"seat":0, "entries":[{"name":"cg_fov", "value":"111"}]},
                {"seat":1, "entries":[{"name":"cg_fov", "value":"122"}]}
            ]
        }),
    );
    let mut runtime = Runtime::load(2, []).unwrap();
    let mut console = Console::<Runtime>::new(Context::default()).unwrap();
    let owners: Vec<_> = (0..2)
        .map(|index| {
            let id = runtime
                .server
                .connect(Connection::Local, ModuleId(1), None)
                .unwrap();
            let owner = (id, runtime.server.clients[id.0 as usize].entity);
            console
                .cvars
                .load_client_values(owner, Some((SeatId::ALL[index], RuleSetId::Quake3)))
                .unwrap();
            owner
        })
        .collect();
    let report = files
        .import(
            &mut console,
            &mut runtime,
            "q3",
            &[ClientPolicy::select(None, Some(RuleSetId::Quake3), None, None, None).unwrap(); 2],
        )
        .unwrap();
    assert_eq!(report.seats, [SeatId::FIRST, SeatId::ALL[1]]);
    for (owner, expected) in owners.into_iter().zip(["111", "122"]) {
        let context = Context {
            client_lifetime: Some(owner),
            ..Context::default()
        };
        let fov = console.cvars.bind("cg_fov", context).unwrap();
        assert_eq!(console.cvars.read(fov).unwrap().as_str(), expected);
    }
    let global = console.cvars.bind("cg_fov", Context::default()).unwrap();
    assert_eq!(console.cvars.read(global).unwrap().as_str(), "90");
}

#[test]
fn canonical_profiles_load_before_connections_with_distinct_binds_and_cli_override_priority() {
    let files = ProfileFiles::new("offline-seats");
    files.write(
        "cvars/shared/canonical.json",
        json!({"version":1,"dialect":"q3","entries":[],"players":[
            {"seat":0,"entries":[{"name":"cg_fov","value":"111"}]},
            {"seat":1,"entries":[{"name":"cg_fov","value":"122"}]}
        ]}),
    );
    for (index, action) in [(1, "forward"), (2, "back")] {
        files.write(&format!("mixed/input/seat-{index}.json"),json!({"version":1,"bindings":[{"input":{"kind":"key","code":119},"target":{"kind":"action","action":action}}]}));
    }
    let mut runtime = Runtime::load(2, []).unwrap();
    let mut console = Console::<Runtime>::new(Context::default()).unwrap();
    let policies =
        [ClientPolicy::select(None, Some(RuleSetId::Quake3), None, None, None).unwrap(); 2];
    let report = files
        .import(&mut console, &mut runtime, "mixed", &policies)
        .unwrap();
    assert!(report.consumed());
    assert_eq!(
        runtime
            .server
            .clients
            .iter()
            .filter(|c| c.connection.is_some())
            .count(),
        0
    );
    for (seat, expected, binding) in [
        (SeatId::FIRST, "111", b"+forward".as_slice()),
        (SeatId::ALL[1], "122", b"+back".as_slice()),
    ] {
        let context = Context {
            seat,
            cvar_seat: Some(seat),
            ..Context::default()
        };
        assert_eq!(
            console
                .cvars
                .read(console.cvars.bind("cg_fov", context).unwrap())
                .unwrap()
                .as_str(),
            expected
        );
        assert_eq!(runtime.input.binding(seat, 26).unwrap().bytes(), binding);
    }
    apply_overrides(
        &mut console,
        &[("fov".into(), "117".into(), Context::default())],
    )
    .unwrap();
    for seat in [SeatId::FIRST, SeatId::ALL[1]] {
        let context = Context {
            seat,
            cvar_seat: Some(seat),
            ..Context::default()
        };
        assert_eq!(
            console
                .cvars
                .read(console.cvars.bind("cg_fov", context).unwrap())
                .unwrap()
                .as_str(),
            "117"
        );
    }
    assert_eq!(
        console
            .cvars
            .read(console.cvars.bind("cg_fov", Context::default()).unwrap())
            .unwrap()
            .as_str(),
        "117"
    );
}

#[test]
fn fallback_profiles_select_each_seats_independent_client_and_input_roles() {
    let files = ProfileFiles::new("role-seats");
    for (path, source, name, value) in [
        ("mixed/cvars/client/q2/0.json", "q2", "name", "alpha"),
        ("mixed/cvars/client/q3/1.json", "q3", "name", "beta"),
        ("mixed/cvars/client/q3/0.json", "q3", "name", "wrong role"),
        ("mixed/cvars/input/qw/0.json", "qw", "sensitivity", "6"),
        ("mixed/cvars/input/q1/1.json", "q1", "sensitivity", "9"),
    ] {
        files.write(
            path,
            json!({"version":1,"dialect":source,"entries":[{"name":name,"value":value}]}),
        );
    }
    let policies = [
        ClientPolicy {
            client: RuleSetId::Quake2,
            input: RuleSetId::QuakeWorld,
            movement: RuleSetId::Quake3,
            trace: RuleSetId::Quake,
        },
        ClientPolicy {
            client: RuleSetId::Quake3,
            input: RuleSetId::Quake,
            movement: RuleSetId::Quake2,
            trace: RuleSetId::QuakeWorld,
        },
    ];
    let mut runtime = Runtime::load(2, []).unwrap();
    let mut console = Console::<Runtime>::new(Context::default()).unwrap();
    let report = files
        .import(&mut console, &mut runtime, "mixed", &policies)
        .unwrap();
    assert!(
        !report
            .files
            .iter()
            .any(|path| path.ends_with("client/q3/0.json"))
    );
    for (index, name, sensitivity) in [(0, "alpha", 6.0), (1, "beta", 9.0)] {
        let seat = SeatId::ALL[index];
        let context = Context {
            seat,
            cvar_seat: Some(seat),
            source: policies[index].client,
            ..Context::default()
        };
        assert_eq!(
            console
                .cvars
                .read(console.cvars.bind("name", context).unwrap())
                .unwrap()
                .as_str(),
            name
        );
        let input = Context {
            source: policies[index].input,
            ..context
        };
        assert_eq!(
            console
                .cvars
                .numeric(console.cvars.bind("sensitivity", input).unwrap())
                .unwrap(),
            sensitivity
        );
    }
}
