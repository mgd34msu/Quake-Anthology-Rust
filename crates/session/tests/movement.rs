use qa_core::{
    primitives::{CommandIntent, ModuleId, MovementRules, Plane, PlayerTail, UserCmd, Vec3},
    sys_events::EventTime,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};
use qa_world::collision::{
    CollisionWorld, Contents,
    brushes::{Brush, BrushMap},
};
fn floor() -> CollisionWorld {
    CollisionWorld::Q2Brushes(
        BrushMap::load(
            vec![Plane {
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: 0.0,
                axis: None,
            }],
            vec![Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            }],
        )
        .unwrap(),
    )
}
#[test]
fn all_clients_and_prediction_use_identical_movement_on_foreign_geometry() {
    let mut server = Server::load(15, 64, 1, 0).unwrap();
    let mut world = floor();
    let rules = [
        MovementRules::Quake,
        MovementRules::QuakeWorld,
        MovementRules::Quake2,
        MovementRules::Quake2Rerelease,
        MovementRules::Quake3,
    ];
    let mut predictions: [Prediction; 15] = std::array::from_fn(|_| Prediction::default());
    for (slot, prediction) in predictions.iter_mut().enumerate() {
        let connection = [Connection::Local, Connection::Remote, Connection::Bot][slot / 5];
        let id = server
            .connect(
                connection,
                ModuleId(2),
                PlayerTail::Q1 {
                    attack_finished: 7.0,
                },
            )
            .unwrap();
        let client = &mut server.clients[slot];
        client.player.movement_rules = rules[slot % 5];
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([0.0, 0.0, 24.125]);
        client.player.movement.grounded = true;
        client.intent = CommandIntent {
            movement: [127, 0, 0],
            ..Default::default()
        };
        prediction.apply_snapshot(&client.player);
        if connection != Connection::Bot {
            server.submit_command(
                id,
                UserCmd {
                    duration_ms: 16,
                    server_time_ms: 16,
                    movement: [127, 0, 0],
                    ..Default::default()
                },
            );
        }
    }
    server.build_bot_commands(EventTime(0), EventTime(16_000_000));
    assert_eq!(server.move_pending_clients(&mut world), 15);
    for (client, prediction) in server.clients[..15].iter().zip(&mut predictions) {
        prediction.advance(client.command, &mut world);
        assert_eq!(client.player.body.position, prediction.player.body.position);
        assert_eq!(client.player.body.velocity, prediction.player.body.velocity);
        assert_eq!(client.player.movement, prediction.player.movement);
        assert_eq!(
            client.player.tail,
            PlayerTail::Q1 {
                attack_finished: 7.0
            }
        );
        assert!(client.player.body.position.0[0] > 0.0);
        assert_eq!(
            server.entities.columns.position[client.entity.slot as usize],
            client.player.body.position
        );
    }
    assert_eq!(server.move_pending_clients(&mut world), 0);
}
