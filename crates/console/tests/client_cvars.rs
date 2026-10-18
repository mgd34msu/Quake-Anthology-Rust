use qa_console::{
    catalog::Scope,
    cvars::Cvars,
    views::{Context, Role, RuleSetId},
};
use qa_core::primitives::{ClientId, EntityId};

fn client(index: u32, generation: u32) -> Context {
    Context {
        source: RuleSetId::Quake3,
        role: Role::Cgame,
        client_lifetime: Some((
            ClientId(index),
            EntityId {
                slot: index + 10,
                generation,
            },
        )),
        ..Context::default()
    }
}

#[test]
fn connection_values_inherit_global_changes_until_locally_overridden() {
    let mut vars = Cvars::new().unwrap();
    let first = client(1, 3);
    let second = client(8, 5);
    for context in [first, second] {
        vars.load_client_values(context.client_lifetime.unwrap())
            .unwrap();
    }
    let global = vars.bind("pmove_msec", Context::default()).unwrap();
    let a = vars.bind("pmove_msec", first).unwrap();
    let b = vars.bind("pmove_msec", second).unwrap();
    let native_a = vars.native_view(a).unwrap();
    let native_b = vars.native_view(b).unwrap();
    vars.force_write(global, "12").unwrap();
    for view in [global, a, b] {
        assert_eq!(vars.integer_view(view).unwrap(), 12);
    }
    vars.force_write(a, "16").unwrap();
    assert_eq!(vars.integer_view(a).unwrap(), 16);
    assert_eq!(vars.integer_view(b).unwrap(), 12);
    let b_generation = vars.native_snapshot(native_b).unwrap().1;
    vars.force_write(a, "20").unwrap();
    assert_eq!(vars.native_snapshot(native_b).unwrap().1, b_generation);
    assert_eq!(vars.native_snapshot(native_a).unwrap().1, 4);
    vars.force_write(global, "24").unwrap();
    assert_eq!(vars.integer_view(a).unwrap(), 20);
    assert_eq!(vars.integer_view(b).unwrap(), 24);
    vars.reset(a.canonical());
    assert_eq!(vars.integer_view(a).unwrap(), 24);
    let server = vars
        .bind(
            "pmove_msec",
            Context {
                side: Scope::Server,
                ..first
            },
        )
        .unwrap();
    assert_eq!(server.canonical(), global.canonical());
    assert_eq!(vars.integer_view(server).unwrap(), 24);
}

#[test]
fn module_defaults_flags_aliases_latches_and_client_slot_reuse_are_isolated() {
    let mut vars = Cvars::new().unwrap();
    let a = client(1, 3);
    let b = client(8, 5);
    for context in [a, b] {
        vars.load_client_values(context.client_lifetime.unwrap())
            .unwrap();
    }
    let av = vars.register("_client_value", Some("1"), 32, a).unwrap();
    let bv = vars.register("_client_value", Some("2"), 64, b).unwrap();
    assert_eq!(vars.read(av).unwrap().as_str(), "1");
    assert_eq!(vars.read(bv).unwrap().as_str(), "2");
    assert_eq!(vars.flags(av), 32);
    assert_eq!(vars.flags(bv), 64);
    vars.set_runtime_policy(qa_console::cvars::RuntimePolicy {
        initialized: true,
        server_active: false,
    });
    vars.write(av, "3").unwrap();
    assert_eq!(vars.latched(av).unwrap().unwrap().as_str(), "3");
    assert!(vars.latched(bv).unwrap().is_none());
    vars.apply_latch(av).unwrap();
    assert_eq!(vars.read(av).unwrap().as_str(), "3");
    assert_eq!(vars.read(bv).unwrap().as_str(), "2");
    let fov_a = vars.bind("fov", a).unwrap();
    let fov_b = vars.bind("cg_fov", b).unwrap();
    vars.force_write(fov_a, "110").unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", a).unwrap()).unwrap(),
        110.0
    );
    assert_ne!(vars.numeric(fov_b).unwrap(), 110.0);
    let native = vars.native_view(av).unwrap();
    let next = client(1, 4);
    vars.load_client_values(next.client_lifetime.unwrap())
        .unwrap();
    assert!(vars.read(av).is_err());
    assert!(vars.bind("_client_value", a).is_none());
    let fresh = vars.register("_client_value", Some("7"), 0, next).unwrap();
    assert_eq!(vars.read(fresh).unwrap().as_str(), "7");
    assert_eq!(vars.read(bv).unwrap().as_str(), "2");
    assert_eq!(vars.native_view(fresh).unwrap(), native);
}

#[test]
fn received_systeminfo_and_cheat_resets_do_not_change_server_or_another_client() {
    let mut vars = Cvars::new().unwrap();
    let a = client(0, 1);
    let b = client(2, 9);
    for context in [a, b] {
        vars.load_client_values(context.client_lifetime.unwrap())
            .unwrap();
    }
    let allow = vars.bind("cl_allowDownload", a).unwrap();
    vars.force_write(allow, "0").unwrap();
    let global = Context::default();
    let cheats = vars.bind("sv_cheats", global).unwrap();
    vars.force_write(cheats, "1").unwrap();
    let local = vars
        .register("_cheat_pref", Some("1"), 512, global)
        .unwrap();
    vars.force_write(local, "3").unwrap();
    let root = vars.bind("pmove_msec", global).unwrap();
    vars.force_write(root, "8").unwrap();
    vars.apply_client_system_info(
        a.client_lifetime.unwrap(),
        b"\\sv_serverid\\42\\sv_cheats\\0\\pmove_msec\\16\\cl_allowDownload\\1\\_mod_setting\\a\\fs_game\\missionpack",
    )
    .unwrap();
    vars.apply_client_system_info(
        b.client_lifetime.unwrap(),
        b"\\sv_serverid\\77\\sv_cheats\\1\\pmove_msec\\12\\_mod_setting\\b",
    )
    .unwrap();
    for (context, msec, id, preference, game) in
        [(a, 16, 42, "1", "missionpack"), (b, 12, 77, "3", "")]
    {
        assert_eq!(
            vars.integer_view(vars.bind("pmove_msec", context).unwrap())
                .unwrap(),
            msec
        );
        assert_eq!(
            vars.integer_view(vars.bind("sv_serverid", context).unwrap())
                .unwrap(),
            id
        );
        assert_eq!(
            vars.read(vars.bind("_cheat_pref", context).unwrap())
                .unwrap()
                .as_str(),
            preference
        );
        assert_eq!(
            vars.read(vars.bind("fs_game", context).unwrap())
                .unwrap()
                .as_str(),
            game
        );
    }
    assert_eq!(vars.read(local).unwrap().as_str(), "3");
    assert_eq!(vars.integer_view(root).unwrap(), 8);
    assert_eq!(vars.integer_view(cheats).unwrap(), 1);
    assert_eq!(
        vars.read(vars.bind("_mod_setting", a).unwrap())
            .unwrap()
            .as_str(),
        "a"
    );
    assert_eq!(
        vars.read(vars.bind("_mod_setting", b).unwrap())
            .unwrap()
            .as_str(),
        "b"
    );
    let handle = vars.find("pmove_msec").unwrap();
    let scoped = vars
        .client_handle(handle, a.client_lifetime.unwrap())
        .unwrap();
    assert_eq!(vars.integer_in(scoped, RuleSetId::Quake3), 16);
    assert_eq!(vars.integer_in(handle, RuleSetId::Quake3), 8);
    assert_eq!(vars.integer_view(allow).unwrap(), 0);
}

#[test]
fn inherited_latches_and_physical_controls_use_their_declared_owner() {
    let mut vars = Cvars::new().unwrap();
    let global = vars
        .register("_shared_latch", Some("1"), 32, Context::default())
        .unwrap();
    let volume = vars.find("s_volume").unwrap();
    vars.share_with_clients(volume);
    vars.set_text(volume, "0.75").unwrap();
    let a = client(0, 1);
    vars.load_client_values(a.client_lifetime.unwrap()).unwrap();
    let local = vars.bind("_shared_latch", a).unwrap();
    let native = vars.native_view(local).unwrap();
    vars.set_runtime_policy(qa_console::cvars::RuntimePolicy {
        initialized: true,
        server_active: false,
    });
    vars.write(global, "2").unwrap();
    assert_eq!(vars.latched(local).unwrap().unwrap().as_str(), "2");
    assert_eq!(vars.native_snapshot(native).unwrap().1, 2);
    vars.apply_latch(global).unwrap();
    assert_eq!(vars.read(local).unwrap().as_str(), "2");
    assert_eq!(vars.native_snapshot(native).unwrap().1, 3);
    let local_volume = vars.bind("s_volume", a).unwrap();
    assert_eq!(local_volume.canonical(), volume);
    let server_info = vars.info_generation(RuleSetId::Quake3, 4 | 8);
    vars.apply_client_system_info(
        a.client_lifetime.unwrap(),
        b"\\sv_cheats\\1\\pmove_msec\\16\\s_volume\\0",
    )
    .unwrap();
    assert_eq!(vars.text(volume), "0.75");
    assert_eq!(vars.info_generation(RuleSetId::Quake3, 4 | 8), server_info);
    vars.force_write(local_volume, "0.5").unwrap();
    assert_eq!(vars.text(volume), "0.5");
}
