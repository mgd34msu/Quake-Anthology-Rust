use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    client_resources::ClientResources,
    map::SpawnAnchor,
    modules::{Q3Role, Q3Spec, complete_q3_clients},
};
use qa_console::{cvars::Cvars, views::Context};
use qa_core::{primitives::*, sys_events::SeatId};
use qa_network::commands::packet::Protocol;

#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod support;

fn connect(runtime: &mut Runtime, seat: SeatId, rules: RuleSetId) -> ClientId {
    runtime
        .connect_local(
            seat,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                client: rules,
                input: RuleSetId::QuakeWorld,
                movement: if rules == RuleSetId::Quake3 {
                    RuleSetId::Quake
                } else {
                    RuleSetId::Quake3
                },
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            seat.index() as u32,
        )
        .unwrap()
}
fn set(cvars: &mut Cvars, name: &str, text: &str) {
    let view = cvars.bind(name, Context::default()).unwrap();
    cvars.force_write(view, text).unwrap();
}

#[test]
fn defaults_follow_client_roles_and_selected_files_without_replacing_explicit_modules() {
    let mut fixture = support::Fixture::new(&[("vm/cgame.qvm", vec![1]), ("vm/ui.qvm", vec![2])]);
    let other = support::Fixture::new(&[("vm/cgame.qvm", vec![3]), ("vm/ui.qvm", vec![4])]);
    let selected = fixture
        .vfs
        .mounts()
        .map(|mount| mount.id)
        .collect::<Vec<_>>();
    let search = fixture.vfs.search_view(selected).unwrap();
    let other_mount = other.vfs.mounts().next().unwrap().id;
    fixture
        .vfs
        .mount_directory(other.vfs.directory_path(other_mount).unwrap(), 100)
        .unwrap();
    let mut runtime = Runtime::load(4, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    runtime
        .server
        .connect(qa_session::clients::Connection::Bot, ModuleId(0), None)
        .unwrap();
    let first = connect(&mut runtime, SeatId::FIRST, RuleSetId::Quake);
    let seat = SeatId::new(2).unwrap();
    let id = connect(&mut runtime, seat, RuleSetId::Quake3);
    runtime.client_resources[id.0 as usize] = ClientResources::load(
        id,
        Protocol::Quake3_68,
        RuleSetId::Quake3,
        &runtime.vfs,
        Some(&search),
    )
    .unwrap();
    let mut clients = [None; SeatId::COUNT];
    clients[0] = Some(first);
    clients[seat.index()] = Some(id);
    let cvars = Cvars::new().unwrap();
    let mut specs = vec![
        Q3Spec::parse("game:custom-server.qvm").unwrap(),
        Q3Spec::parse("cgame:2:custom-client.qvm").unwrap(),
    ];
    let events = runtime.server.events.len();
    complete_q3_clients(&runtime, &cvars, &mut specs, &clients).unwrap();
    assert_eq!(specs.len(), 3);
    assert_eq!(specs[0].path, "custom-server.qvm");
    assert_eq!(specs[1].path, "custom-client.qvm");
    assert_eq!(specs[2].role, Q3Role::Ui(seat));
    assert_eq!(specs[2].path, "vm/ui.qvm");
    assert!(specs[2].search.as_ref().unwrap().same_selection(&search));
    let view = runtime.vfs.read_view(specs[2].search.as_ref());
    let file = view.open(specs[2].path.as_bytes()).unwrap();
    assert_eq!(
        view.read_to_vec(&file, 16, &mut Default::default())
            .unwrap(),
        [2]
    );
    complete_q3_clients(&runtime, &cvars, &mut specs, &clients).unwrap();
    assert_eq!(specs.len(), 3);
    assert_eq!(
        runtime.server.events.len(),
        events,
        "selection never drives a frame or publishes output"
    );
    assert_eq!(
        runtime.server.clients[id.0 as usize].player.movement_rules,
        RuleSetId::Quake
    );
}

#[test]
fn native_mode_prefers_a_native_file_and_restricted_or_missing_native_uses_qvm() {
    let mut fixture = support::Fixture::new(&[
        ("cgamex86_64.so", vec![1]),
        ("vm/cgame.qvm", vec![2]),
        ("vm/ui.qvm", vec![3]),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let id = connect(&mut runtime, SeatId::FIRST, RuleSetId::Quake3);
    let clients = [Some(id), None, None, None];
    let mut cvars = Cvars::new().unwrap();
    for (mode, restricted, path) in [
        (0, 0, "cgamex86_64.so"),
        (0, 1, "vm/cgame.qvm"),
        (1, 0, "vm/cgame.qvm"),
        (2, 0, "vm/cgame.qvm"),
    ] {
        set(&mut cvars, "vm_cgame", &mode.to_string());
        set(&mut cvars, "vm_ui", "0");
        set(&mut cvars, "fs_restrict", &restricted.to_string());
        let mut specs = Vec::new();
        complete_q3_clients(&runtime, &cvars, &mut specs, &clients).unwrap();
        assert_eq!(specs[0].path, path);
        assert_eq!(specs[1].path, "vm/ui.qvm");
    }
}

#[test]
fn a_missing_default_does_not_leave_a_partially_completed_role_list() {
    let mut fixture = support::Fixture::new(&[("vm/cgame.qvm", vec![1])]);
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let id = connect(&mut runtime, SeatId::FIRST, RuleSetId::Quake3);
    let mut specs = vec![Q3Spec::parse("game:custom.qvm").unwrap()];
    let error = complete_q3_clients(
        &runtime,
        &Cvars::new().unwrap(),
        &mut specs,
        &[Some(id), None, None, None],
    )
    .unwrap_err();
    assert!(error.contains("vm/ui.qvm"));
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].path, "custom.qvm");
}
