use qa_core::{
    primitives::{CommandIntent, ModuleId, MovementRules, Plane, PlayerTail, UserCmd, Vec3},
    sys_events::EventTime,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};
use qa_world::{
    area::{LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionWorld, Contents, EntityTraceRules, TraceQuery, TraceRules, WorldTrace,
        brushes::{Brush, BrushMap},
    },
};
fn floor() -> CollisionWorld {
    CollisionWorld::Brushes(
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
    let mut server = Server::load(15, 64, 1, 0, 0).unwrap();
    let world = floor();
    let mut scratch = world.scratch();
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
                None,
            )
            .unwrap();
        let client = &mut server.clients[slot];
        client.player.movement_rules = rules[slot % 5];
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([0.0, slot as f32 * 128.0, 24.125]);
        client.player.movement.grounded = true;
        client.intent = CommandIntent {
            movement: [127, 0, 0],
            ..Default::default()
        };
        prediction.apply_snapshot(&client.player);
        let entity = client.entity;
        server
            .entities
            .columns
            .set_body(entity.slot as usize, client.player.body);
        assert!(server.area.link(
            &server.entities,
            entity,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit,
        ));
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
    assert_eq!(server.move_pending_clients(&world, &mut scratch), 15);
    for (client, prediction) in server.clients[..15].iter().zip(&mut predictions) {
        let mut trace = WorldTrace::new(
            &world,
            &server.entities,
            &server.area,
            &mut scratch,
            Some(client.entity),
        );
        prediction.advance(client.command, &mut trace);
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
    assert_eq!(server.move_pending_clients(&world, &mut scratch), 0);
}

#[test]
fn authoritative_movement_skips_self_hits_another_client_and_unlinks_disconnects() {
    let mut server = Server::load(2, 16, 1, 0, 0).unwrap();
    let world = floor();
    let mut scratch = world.scratch();
    let moving = server
        .connect(Connection::Local, ModuleId(2), PlayerTail::None, None)
        .unwrap();
    let obstacle = server
        .connect(Connection::Remote, ModuleId(1), PlayerTail::None, None)
        .unwrap();
    for (id, x, rules, order) in [
        (moving, 0.0, MovementRules::Quake3, LinkOrder::Tail),
        (obstacle, 48.0, MovementRules::Quake2, LinkOrder::Head),
    ] {
        let client = &mut server.clients[id.0 as usize];
        client.player.movement_rules = rules;
        client.link_order = order;
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([x, 0.0, 24.125]);
        client.player.health = 100;
        client.player.movement.grounded = true;
        server
            .entities
            .columns
            .set_body(client.entity.slot as usize, client.player.body);
        assert!(server.area.link(
            &server.entities,
            client.entity,
            LinkFlags::SOLID,
            order,
            LinkIntent::Explicit,
        ));
    }
    let mover_entity = server.clients[moving.0 as usize].entity;
    let obstacle_entity = server.clients[obstacle.0 as usize].entity;
    let body = server.clients[moving.0 as usize].player.body;
    let query = TraceQuery {
        mins: body.mins,
        maxs: body.maxs,
        mask: Contents::SOLID | Contents::BODY,
        ..TraceQuery::point(
            body.position,
            Vec3([96.0, 0.0, 24.125]),
            TraceRules::ARENA,
            EntityTraceRules::ARENA,
        )
    };
    {
        let mut trace = WorldTrace::new(
            &world,
            &server.entities,
            &server.area,
            &mut scratch,
            Some(mover_entity),
        );
        let own_position = trace.trace(TraceQuery {
            end: query.start,
            ..query
        });
        assert!(!own_position.start_solid && !own_position.all_solid);
        assert_eq!(own_position.fraction, 1.0);
        let other = trace.trace(query);
        assert_eq!(other.entity, Some(obstacle_entity));
        assert!(other.fraction > 0.0 && other.fraction < 1.0);
    }
    server.submit_command(
        moving,
        UserCmd {
            duration_ms: 200,
            server_time_ms: 200,
            movement: [127, 0, 0],
            ..Default::default()
        },
    );
    assert!(server.move_pending_clients(&world, &mut scratch) > 0);
    let moved = server.clients[moving.0 as usize].player.body.position;
    assert!(moved.0[0] > 0.0 && moved.0[0] < 18.0);
    assert_eq!(
        server.clients[obstacle.0 as usize].player.body.position.0[0],
        48.0
    );
    assert_eq!(
        server.clients[obstacle.0 as usize].link_order,
        LinkOrder::Head
    );
    assert_eq!(
        server.entities.columns.position[mover_entity.slot as usize],
        moved
    );
    assert!(server.disconnect(obstacle));
    let mut trace = WorldTrace::new(
        &world,
        &server.entities,
        &server.area,
        &mut scratch,
        Some(mover_entity),
    );
    let after_disconnect = trace.trace(TraceQuery {
        start: moved,
        ..query
    });
    assert_eq!(after_disconnect.fraction, 1.0);
    assert!(!after_disconnect.start_solid && !after_disconnect.all_solid);
}
