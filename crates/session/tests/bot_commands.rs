use qa_core::{
    primitives::{CommandIntent, ModuleId, MovementRules, PlayerTail, Vec3, WeaponId, buttons},
    sys_events::EventTime,
};
use qa_input::UserCmdBuilder;
use qa_session::clients::{Connection, Server};
use std::time::Duration;

#[test]
fn all_64_bot_slots_build_in_server_time_without_local_seat_state() {
    let mut server = Server::load(64, 128, 1, 0).unwrap();
    let rules = [
        MovementRules::Quake,
        MovementRules::QuakeWorld,
        MovementRules::Quake2,
        MovementRules::Quake2Rerelease,
        MovementRules::Quake3,
    ];
    for slot in 0..64 {
        let id = server
            .connect(
                Connection::Bot,
                ModuleId((slot % 3) as u16),
                PlayerTail::default(),
            )
            .unwrap();
        assert_eq!(id.0 as usize, slot);
        server.clients[slot].player.movement_rules = rules[slot % rules.len()];
        server.clients[slot].intent = CommandIntent {
            movement: [slot as i16, -(slot as i16), 17],
            view_angles: Vec3([1.0, slot as f32, 3.0]),
            buttons: buttons::ATTACK,
            impulse: slot as u8,
            weapon: Some(WeaponId(3)),
            light_level: 127,
        };
    }
    server.build_bot_commands(EventTime(5_000_000_000), EventTime(5_050_000_000));
    for slot in 0..64 {
        let client = &server.clients[slot];
        let local = UserCmdBuilder::build(
            Duration::from_millis(50),
            EventTime(5_050_000_000),
            client.intent,
        );
        assert_eq!(client.command.duration_ms, 50);
        assert_eq!(client.command.server_time_ms, 5050);
        assert_eq!(client.command.movement, local.movement);
        assert_eq!(client.command.view_angles, local.view_angles);
        assert_eq!(client.command.buttons, local.buttons);
        assert_eq!(client.command.impulse, local.impulse);
        assert_eq!(client.command.weapon, local.weapon);
        assert_eq!(client.command.light_level, local.light_level);
    }
    let id = qa_core::primitives::ClientId(63);
    assert!(server.disconnect(id));
    server.clients[63].command.duration_ms = 77;
    server.build_bot_commands(EventTime(5_050_000_000), EventTime(5_100_000_000));
    assert_eq!(server.clients[63].command.duration_ms, 77); // disconnected slot excluded
    let reused = server
        .connect(Connection::Bot, ModuleId(2), PlayerTail::default())
        .unwrap();
    assert_eq!(reused, id);
    server.build_bot_commands(EventTime(5_100_000_000), EventTime(5_150_000_000));
    assert_eq!(server.clients[63].command.movement, [0; 3]);
    assert_eq!(server.clients[63].command.weapon, None);
    assert_eq!(server.clients[63].command.duration_ms, 50);
}

#[test]
fn bot_duration_uses_movement_policy_instead_of_module_family() {
    let mut server = Server::load(4, 16, 1, 0).unwrap();
    for (slot, (connection, rule)) in [
        (Connection::Bot, MovementRules::Quake),
        (Connection::Bot, MovementRules::Quake2),
        (Connection::Bot, MovementRules::Quake3),
        (Connection::Remote, MovementRules::Quake3),
    ]
    .into_iter()
    .enumerate()
    {
        server
            .connect(
                connection,
                ModuleId(2),
                PlayerTail::Q2 {
                    weapon_frame: 0,
                    movement_time: 0,
                },
            )
            .unwrap();
        server.clients[slot].player.movement_rules = rule;
        server.clients[slot].command.duration_ms = 7;
    }
    server.build_bot_commands(EventTime(0), EventTime(300_000_000));
    assert_eq!(
        server.clients.map(|c| c.command.duration_ms)[..4],
        [100, 100, 200, 7]
    );
}
