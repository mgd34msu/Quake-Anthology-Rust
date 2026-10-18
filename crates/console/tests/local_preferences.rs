use qa_console::{cvars::Cvars, views::Context};
use qa_core::{
    primitives::{ClientId, EntityId},
    sys_events::SeatId,
};

fn client(index: u32, generation: u32, seat: u8) -> Context {
    Context {
        seat: SeatId::new(seat).unwrap(),
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
fn local_archived_preferences_survive_connection_replacement() {
    let mut vars = Cvars::new().unwrap();
    let first = client(0, 1, 0);
    vars.load_client_values(
        first.client_lifetime.unwrap(),
        Some((first.seat, first.source)),
    )
    .unwrap();
    let fov = vars.bind("cg_fov", first).unwrap();
    vars.force_write(fov, "111").unwrap();
    let next = client(0, 2, 0);
    vars.load_client_values(
        next.client_lifetime.unwrap(),
        Some((next.seat, next.source)),
    )
    .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", next).unwrap()).unwrap(),
        111.0
    );
}

#[test]
fn seat_preferences_survive_other_clients_using_the_previous_native_slot() {
    let mut vars = Cvars::new().unwrap();
    let first = client(0, 1, 0);
    vars.load_client_values(
        first.client_lifetime.unwrap(),
        Some((first.seat, first.source)),
    )
    .unwrap();
    let old = vars.bind("cg_fov", first).unwrap();
    vars.force_write(old, "111").unwrap();
    let native = vars.native_view(old).unwrap();
    let server_id = vars.bind("sv_serverid", first).unwrap();
    vars.force_write(server_id, "77").unwrap();
    let ephemeral = vars
        .register("_connection_only", Some("1"), 0, first)
        .unwrap();
    vars.force_write(ephemeral, "19").unwrap();
    let latched = vars
        .register("_local_latch", Some("1"), 1 | 32, first)
        .unwrap();
    vars.set_runtime_policy(qa_console::cvars::RuntimePolicy {
        initialized: true,
        server_active: false,
    });
    vars.write(latched, "23").unwrap();

    let peer = client(0, 2, 1);
    vars.load_client_values(
        peer.client_lifetime.unwrap(),
        Some((peer.seat, peer.source)),
    )
    .unwrap();
    let peer_fov = vars.bind("cg_fov", peer).unwrap();
    assert_eq!(vars.numeric(peer_fov).unwrap(), 90.0);
    vars.force_write(peer_fov, "99").unwrap();
    assert!(vars.read(old).is_err());
    assert!(vars.native_snapshot(native, first).is_none());

    let next = client(7, 1, 0);
    vars.load_client_values(
        next.client_lifetime.unwrap(),
        Some((next.seat, next.source)),
    )
    .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", next).unwrap()).unwrap(),
        111.0
    );
    assert_ne!(
        vars.integer_view(vars.bind("sv_serverid", next).unwrap())
            .unwrap(),
        77
    );
    let fresh = vars
        .register("_connection_only", Some("7"), 0, next)
        .unwrap();
    assert_eq!(vars.read(fresh).unwrap().as_str(), "7");
    let restored = vars
        .register("_local_latch", Some("2"), 1 | 32, next)
        .unwrap();
    assert_eq!(vars.read(restored).unwrap().as_str(), "23");
    assert!(vars.latched(restored).unwrap().is_none());
    assert_eq!(vars.numeric(peer_fov).unwrap(), 99.0);
    let root = vars.bind("cg_fov", Context::default()).unwrap();
    assert_eq!(vars.numeric(root).unwrap(), 90.0);

    let remote = client(7, 2, 2);
    vars.load_client_values(remote.client_lifetime.unwrap(), None)
        .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", remote).unwrap()).unwrap(),
        90.0
    );
    assert!(vars.bind("cg_fov", next).is_none());
    let reconnect = client(9, 1, 0);
    vars.load_client_values(
        reconnect.client_lifetime.unwrap(),
        Some((reconnect.seat, reconnect.source)),
    )
    .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", reconnect).unwrap())
            .unwrap(),
        111.0
    );
}

#[test]
fn local_archive_preserves_alias_spelling_and_converts_through_each_native_view() {
    for source in qa_core::primitives::RuleSetId::ALL {
        let mut vars = Cvars::new().unwrap();
        let first = Context {
            source,
            ..client(0, 1, 0)
        };
        vars.load_client_values(first.client_lifetime.unwrap(), Some((first.seat, source)))
            .unwrap();
        let alias = vars.bind("qts_weapon_autoswitch", first).unwrap();
        vars.write(alias, "NEVER").unwrap();
        let next = Context {
            source,
            ..client(1, 1, 0)
        };
        vars.load_client_values(next.client_lifetime.unwrap(), Some((next.seat, source)))
            .unwrap();
        let alias = vars.bind("qts_weapon_autoswitch", next).unwrap();
        assert_eq!(vars.read(alias).unwrap().as_str(), "NEVER");
        assert_eq!(
            vars.numeric(vars.bind("cg_autoswitch", next).unwrap())
                .unwrap(),
            0.0
        );
    }
}

#[test]
fn moving_a_connection_to_another_seat_does_not_transfer_the_old_seats_preferences() {
    let mut vars = Cvars::new().unwrap();
    let first = client(0, 1, 0);
    vars.load_client_values(
        first.client_lifetime.unwrap(),
        Some((first.seat, first.source)),
    )
    .unwrap();
    vars.force_write(vars.bind("cg_fov", first).unwrap(), "111")
        .unwrap();
    let moved = Context {
        seat: SeatId::new(1).unwrap(),
        ..first
    };
    vars.load_client_values(
        moved.client_lifetime.unwrap(),
        Some((moved.seat, moved.source)),
    )
    .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", moved).unwrap()).unwrap(),
        90.0
    );
    let returned = client(2, 1, 0);
    vars.load_client_values(
        returned.client_lifetime.unwrap(),
        Some((returned.seat, returned.source)),
    )
    .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", returned).unwrap())
            .unwrap(),
        111.0
    );
}

#[test]
fn human_seat_selection_survives_retirement_without_weakening_native_lifetimes() {
    let mut vars = Cvars::new().unwrap();
    let first = client(0, 1, 0);
    vars.load_client_values(
        first.client_lifetime.unwrap(),
        Some((first.seat, first.source)),
    )
    .unwrap();
    let native = vars.bind("cg_fov", first).unwrap();
    let human = vars.local_context(Context {
        client_lifetime: None,
        cvar_seat: None,
        ..first
    });
    let seat_view = vars.bind("cg_fov", human).unwrap();
    vars.write(seat_view, "111").unwrap();
    assert_eq!(vars.numeric(native).unwrap(), 111.0);

    let peer = client(0, 2, 1);
    vars.load_client_values(
        peer.client_lifetime.unwrap(),
        Some((peer.seat, peer.source)),
    )
    .unwrap();
    assert!(!vars.has_client_values(first.client_lifetime.unwrap()));
    assert!(vars.bind("cg_fov", vars.local_context(first)).is_none());
    assert!(vars.read(native).is_err());
    vars.write(seat_view, "117").unwrap();
    assert_eq!(vars.numeric(seat_view).unwrap(), 117.0);
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", peer).unwrap()).unwrap(),
        90.0
    );
    let next = client(7, 1, 0);
    vars.load_client_values(
        next.client_lifetime.unwrap(),
        Some((next.seat, next.source)),
    )
    .unwrap();
    assert_eq!(
        vars.numeric(vars.bind("cg_fov", next).unwrap()).unwrap(),
        117.0
    );
    assert_eq!(vars.numeric(seat_view).unwrap(), 117.0);
}

#[test]
fn saved_seat_columns_exist_before_connections_and_first_binding_keeps_pending_values() {
    let mut vars = Cvars::new().unwrap();
    let offline = |seat| Context {
        seat,
        cvar_seat: Some(seat),
        ..Context::default()
    };
    for (seat, text) in [(SeatId::FIRST, "111"), (SeatId::ALL[1], "122")] {
        vars.load_seat_values(seat, qa_core::primitives::RuleSetId::Quake3)
            .unwrap();
        let view = vars.bind("cg_fov", offline(seat)).unwrap();
        vars.write(view, text).unwrap();
        vars.load_seat_values(seat, qa_core::primitives::RuleSetId::Quake3)
            .unwrap();
        assert_eq!(vars.read(view).unwrap().as_str(), text);
    }
    let first = offline(SeatId::FIRST);
    vars.set_user("_profile_module_setting", "42", first, None, 0)
        .unwrap();
    let pending = vars
        .register("_profile_latch", Some("1"), 1 | 32, first)
        .unwrap();
    vars.set_runtime_policy(qa_console::cvars::RuntimePolicy {
        initialized: true,
        server_active: false,
    });
    vars.write(pending, "23").unwrap();
    let owner = client(7, 44, 0);
    vars.load_client_values(
        owner.client_lifetime.unwrap(),
        Some((owner.seat, owner.source)),
    )
    .unwrap();
    assert_eq!(
        vars.read(vars.bind("cg_fov", owner).unwrap())
            .unwrap()
            .as_str(),
        "111"
    );
    assert_eq!(
        vars.read(vars.bind("_profile_module_setting", owner).unwrap())
            .unwrap()
            .as_str(),
        "42"
    );
    let rebound = vars.bind("_profile_latch", owner).unwrap();
    assert_eq!(vars.latched(rebound).unwrap().unwrap().as_str(), "23");
    let peer = vars.bind("cg_fov", offline(SeatId::ALL[1])).unwrap();
    assert_eq!(vars.read(peer).unwrap().as_str(), "122");
    assert_eq!(
        vars.read(vars.bind("cg_fov", Context::default()).unwrap())
            .unwrap()
            .as_str(),
        "90"
    );
}
