use super::*;

pub(super) fn verify(fail_shutdown: bool) {
    let (_fixture, mut host, mut source, ids, _) = setup(qvm_role(false, fail_shutdown));
    queue_gamestate(&mut host, ids[0], "a", 11);
    queue_gamestate(&mut host, ids[1], "a", 22);
    until_running(&mut host, &mut source);
    let old = host.runtime.server.clients[ids[0].0 as usize].entity;
    let peer = host.runtime.server.clients[ids[1].0 as usize].entity;
    let peer_calls = host.module_counts(ModuleId(2)).unwrap().calls;
    assert_eq!(marker_count(&host, b"restart init"), 2);
    host.console
        .append(
            "disconnect\n",
            Context {
                seat: SeatId::FIRST,
                ..Context::default()
            },
        )
        .unwrap();
    host.execute_commands_with_driver(&mut source.driver());
    assert!(
        host.runtime.server.clients[ids[0].0 as usize]
            .connection
            .is_none()
    );
    assert!(host.local_clients[0].is_none());
    for _ in 0..4 {
        source.step(&mut host);
        if host.runtime.server.available_client_slot().is_some() {
            break;
        }
    }
    assert_eq!(marker_count(&host, b"restart shutdown"), 1);
    assert_eq!(host.module_state(ModuleId(2)), Some(State::Running));
    assert_eq!(host.runtime.server.clients[ids[1].0 as usize].entity, peer);
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert!(host.module_counts(ModuleId(2)).unwrap().calls > peer_calls);
    if fail_shutdown {
        assert_eq!(host.module_state(ModuleId(1)), Some(State::Failed));
        return;
    }
    assert_eq!(host.module_state(ModuleId(1)), Some(State::Pending));
    let id = host
        .runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3([32., 0., 0.]),
                angles: Vec3([0., 180., 0.]),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::QuakeWorld,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake,
                trace: RuleSetId::Quake2,
            },
            Protocol::Quake3_68,
            0,
        )
        .unwrap();
    assert_ne!(host.runtime.server.clients[id.0 as usize].entity, old);
    host.local_clients[0] = Some(id);
    queue_gamestate(&mut host, id, "b", 33);
    until_running(&mut host, &mut source);
    assert_eq!(query(&mut host, ModuleId(1), 9), Ok(33));
    assert_eq!(query(&mut host, ModuleId(2), 9), Ok(22));
    assert_eq!(marker_count(&host, b"restart init"), 3);
    assert_eq!(marker_count(&host, b"restart shutdown"), 1);
    source.step(&mut host);
    assert_eq!(marker_count(&host, b"restart init"), 3);
    assert_eq!(marker_count(&host, b"restart shutdown"), 1);
}
