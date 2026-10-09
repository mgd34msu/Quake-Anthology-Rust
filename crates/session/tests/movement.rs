use qa_core::{
    primitives::{
        Bounds, CommandIntent, GeometryId, ModuleId, MovementRules, Plane, PlayerTail,
        SurfaceFlags, UserCmd, Vec3,
    },
    sys_events::EventTime,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};
use qa_world::{
    area::{LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, EntityTraceRules, TraceQuery, TraceRules, WorldTrace,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    },
};
fn floor() -> (CollisionStore, GeometryId) {
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
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
            vec![SurfaceFlags(0)],
            BrushTree::direct(1).unwrap(),
            vec![Bounds {
                mins: Vec3([-131072.0; 3]),
                maxs: Vec3([131072.0; 3]),
            }],
        )
        .unwrap();
    (store, geometry)
}
#[test]
fn all_clients_and_prediction_use_identical_movement_on_foreign_geometry() {
    let mut server = Server::load(15, 64, 1, 0, 0).unwrap();
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
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
    assert_eq!(
        server.move_pending_clients(&store, geometry, 0, &mut scratch),
        15
    );
    for (client, prediction) in server.clients[..15].iter().zip(&mut predictions) {
        let mut trace = WorldTrace::new(
            &store,
            geometry,
            0,
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
    assert_eq!(
        server.move_pending_clients(&store, geometry, 0, &mut scratch),
        0
    );
}

#[test]
fn authoritative_movement_skips_self_hits_another_client_and_unlinks_disconnects() {
    let mut server = Server::load(2, 16, 1, 0, 0).unwrap();
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
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
            &store,
            geometry,
            0,
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
    assert!(server.move_pending_clients(&store, geometry, 0, &mut scratch) > 0);
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
        &store,
        geometry,
        0,
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

#[test]
fn authoritative_and_prediction_callers_select_a_nonzero_model_in_a_second_geometry() {
    let (mut store, first_geometry) = floor();
    let bounds = Bounds {
        mins: Vec3([-131072.0; 3]),
        maxs: Vec3([131072.0; 3]),
    };
    // Model zero encloses the player; model one is the ordinary ground plane.
    // The earlier geometry has only model zero, so either implicit default
    // would produce a different trace and movement result.
    let geometry = store
        .load_brushes(
            [64.0, 0.0]
                .into_iter()
                .map(|distance| Plane {
                    normal: Vec3([0.0, 0.0, 1.0]),
                    distance,
                    axis: None,
                })
                .collect(),
            (0..2)
                .map(|first_plane| Brush {
                    first_plane,
                    plane_count: 1,
                    contents: Contents::SOLID,
                })
                .collect(),
            vec![SurfaceFlags(0); 2],
            BrushTree {
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: (0..2)
                    .map(|first_brush| CollisionLeaf {
                        stored_contents: None,
                        first_brush,
                        brush_count: 1,
                    })
                    .collect(),
                leaf_brushes: vec![0, 1],
                models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
            },
            vec![bounds; 2],
        )
        .unwrap();
    assert_ne!(geometry, first_geometry);
    let mut scratch = store.scratch();
    let mut server = Server::load(1, 8, 1, 0, 0).unwrap();
    let id = server
        .connect(Connection::Local, ModuleId(2), PlayerTail::None, None)
        .unwrap();
    let client = &mut server.clients[id.0 as usize];
    client.player.movement_rules = MovementRules::Quake3;
    client.player.health = 100;
    qa_movement::set_bounds(&mut client.player);
    client.player.body.position = Vec3([0.0, 0.0, 24.125]);
    client.player.movement.grounded = true;
    let entity = client.entity;
    let mut prediction = Prediction::default();
    prediction.apply_snapshot(&client.player);
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
    let query = TraceQuery::point(
        Vec3([0.0, 0.0, 10.0]),
        Vec3([0.0, 0.0, -10.0]),
        TraceRules::ARENA,
        EntityTraceRules::ARENA,
    );
    let selected = WorldTrace::new(
        &store,
        geometry,
        1,
        &server.entities,
        &server.area,
        &mut scratch,
        Some(entity),
    )
    .trace(query);
    assert!(!selected.start_solid && selected.fraction > 0.0 && selected.fraction < 1.0);
    assert!(
        WorldTrace::new(
            &store,
            geometry,
            0,
            &server.entities,
            &server.area,
            &mut scratch,
            Some(entity),
        )
        .trace(query)
        .all_solid
    );
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: 16,
        movement: [127, 0, 0],
        ..Default::default()
    };
    server.submit_command(id, command);
    assert_eq!(
        server.move_pending_clients(&store, geometry, 1, &mut scratch),
        1
    );
    let mut trace = WorldTrace::new(
        &store,
        geometry,
        1,
        &server.entities,
        &server.area,
        &mut scratch,
        Some(entity),
    );
    prediction.advance(command, &mut trace);
    let player = &server.clients[id.0 as usize].player;
    assert!(player.body.position.0[0] > 0.0 && player.movement.grounded);
    assert_eq!(player.body.position, prediction.player.body.position);
    assert_eq!(player.body.velocity, prediction.player.body.velocity);
    assert_eq!(player.movement, prediction.player.movement);
}
