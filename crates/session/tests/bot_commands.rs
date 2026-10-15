use qa_core::{
    primitives::{CommandIntent, ModuleId, RuleSetId, Vec3, WeaponId, buttons},
    sys_events::EventTime,
};
use qa_input::UserCmdBuilder;
use qa_session::clients::{Connection, Server};

#[test]
fn all_64_bot_slots_build_in_server_time_without_local_seat_state() {
    let mut server = Server::load(64, 128, 1, 0, 0, 0).unwrap();
    let cvars = qa_console::cvars::Cvars::new().unwrap();
    let handles = qa_console::input_settings::InputHandles::load(&cvars).unwrap();
    let rules = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ];
    for slot in 0..64 {
        let id = server
            .connect(Connection::Bot, ModuleId((slot % 3) as u16), None)
            .unwrap();
        assert_eq!(id.0 as usize, slot);
        server.clients[slot].input_rules = rules[slot % rules.len()];
        server.clients[slot].player.movement_rules = rules[slot % rules.len()];
        server.clients[slot].player.trace_rules = rules[slot % rules.len()];
        server.clients[slot].intent = CommandIntent {
            view_angles: Vec3([1.0, slot as f32, 3.0]),
            buttons: buttons::ATTACK,
            impulse: slot as u8,
            weapon: Some(WeaponId(3)),
            light_level: 127,
            ..CommandIntent::moving([slot as f32 / 64.0, -(slot as f32) / 64.0, 17.0 / 64.0])
        };
    }
    server.build_bot_commands(
        EventTime(5_000_000_000),
        EventTime(5_050_000_000),
        |rules| handles.policy(&cvars, rules),
    );
    for slot in 0..64 {
        let client = &server.clients[slot];
        let local = UserCmdBuilder::build(
            EventTime(50_000_000),
            EventTime(5_050_000_000),
            client.intent,
            handles.policy(&cvars, client.input_rules),
        );
        assert_eq!(client.command.duration_ms, 50);
        assert_eq!(client.command.server_time_ms, 5050);
        assert_eq!(client.command.movement, local.movement);
        assert_eq!(client.command.units, local.units);
        assert_eq!(client.command.view_angles, local.view_angles);
        assert_eq!(client.command.buttons, local.buttons);
        assert_eq!(client.command.impulse, local.impulse);
        assert_eq!(client.command.weapon, local.weapon);
        assert_eq!(client.command.light_level, local.light_level);
    }
    let id = qa_core::primitives::ClientId(63);
    assert!(server.disconnect(id));
    server.clients[63].command.duration_ms = 77;
    server.build_bot_commands(
        EventTime(5_050_000_000),
        EventTime(5_100_000_000),
        |rules| handles.policy(&cvars, rules),
    );
    assert_eq!(server.clients[63].command.duration_ms, 77); // disconnected slot excluded
    let reused = server.connect(Connection::Bot, ModuleId(2), None).unwrap();
    assert_eq!(reused, id);
    assert_eq!(server.clients[63].input_rules, RuleSetId::default());
    server.build_bot_commands(
        EventTime(5_100_000_000),
        EventTime(5_150_000_000),
        |rules| handles.policy(&cvars, rules),
    );
    assert_eq!(server.clients[63].command.movement, [0.0; 3]);
    assert_eq!(server.clients[63].command.weapon, None);
    assert_eq!(server.clients[63].command.duration_ms, 50);
}

#[test]
fn bot_duration_uses_input_finish_and_movement_admission_instead_of_module_family() {
    let mut server = Server::load(4, 16, 1, 0, 0, 0).unwrap();
    let cvars = qa_console::cvars::Cvars::new().unwrap();
    let handles = qa_console::input_settings::InputHandles::load(&cvars).unwrap();
    for (slot, (connection, rule)) in [
        (Connection::Bot, RuleSetId::Quake),
        (Connection::Bot, RuleSetId::Quake2),
        (Connection::Bot, RuleSetId::Quake3),
        (Connection::Remote, RuleSetId::Quake3),
    ]
    .into_iter()
    .enumerate()
    {
        server.connect(connection, ModuleId(2), None).unwrap();
        server.clients[slot].input_rules = rule;
        server.clients[slot].player.movement_rules = rule;
        server.clients[slot].player.trace_rules = rule;
        server.clients[slot].command.duration_ms = 7;
    }
    server.build_bot_commands(EventTime(0), EventTime(300_000_000), |rules| {
        handles.policy(&cvars, rules)
    });
    assert_eq!(
        std::array::from_fn::<_, 4, _>(|slot| server.clients[slot].command.duration_ms),
        [100, 100, 200, 7]
    );
}
