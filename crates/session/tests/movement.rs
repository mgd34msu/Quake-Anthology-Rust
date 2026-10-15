use qa_core::{
    primitives::{
        Bounds, CollisionOwner, CommandIntent, GeometryId, ModuleId, Plane, RuleSetId,
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
        CollisionStore, Contents, TraceQuery, TraceRules, WorldTrace,
        brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    },
};
fn floor() -> (CollisionStore, GeometryId) {
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            vec![Plane {
                encoding: None,
                normal: Vec3([0.0, 0.0, 1.0]),
                distance: 0.0,
                axis: None,
            }],
            vec![Brush {
                sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                contents: Contents::SOLID,
            }],
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags(0)]),
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
fn shared_server_movement_transports_attached_clients_before_snapshot_copy() {
    use qa_core::primitives::{BodyAttachment, BodyFollow};
    let mut server = Server::load(2, 16, 1, 0, 0, 0).unwrap();
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
    let moving = server
        .connect(Connection::Local, ModuleId(2), None)
        .unwrap();
    let attached = server.connect(Connection::Bot, ModuleId(1), None).unwrap();
    for (slot, rules, order) in [
        (0, RuleSetId::Quake3, LinkOrder::Tail),
        (1, RuleSetId::Quake, LinkOrder::Head),
    ] {
        let client = &mut server.clients[slot];
        client.input_rules = rules;
        client.player.movement_rules = rules;
        client.player.trace_rules = rules;
        client.link_order = order;
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([0.0, slot as f32 * 128.0, 24.125]);
        client.player.movement.grounded = true;
        server
            .entities
            .columns
            .set_body(client.entity.slot as usize, client.player.body);
        server.area.link(
            &server.entities,
            client.entity,
            LinkFlags::SOLID,
            order,
            LinkIntent::Explicit,
        );
    }
    let root = server.clients[moving.0 as usize].entity;
    let child = server.clients[attached.0 as usize].entity;
    server
        .entities
        .attach(
            child,
            BodyAttachment {
                anchor: root,
                follow: BodyFollow::Translation,
                offset: Vec3([0.0, 128.0, 0.0]),
            },
        )
        .unwrap();
    server.submit_command(
        moving,
        UserCmd {
            duration_ms: 16,
            server_time_ms: 16,
            movement: [127.0, 0.0, 0.0],
            units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
            ..Default::default()
        },
        qa_core::primitives::CommandExecution::PerCommand,
        None,
    );
    assert_eq!(
        server.move_pending_clients(
            &store,
            geometry,
            0,
            &mut scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            reject_move
        ),
        1
    );
    let origin = server.clients[0].player.body.position;
    assert!(origin.0[0] > 0.0);
    assert_eq!(
        server.clients[1].player.body.position,
        origin + Vec3([0.0, 128.0, 0.0])
    );
    assert_eq!(
        server.entities.columns.position[child.slot as usize],
        server.clients[1].player.body.position
    );
    assert_eq!(server.clients[1].player.movement_rules, RuleSetId::Quake);
    let mut prediction = Prediction::default();
    prediction.apply_snapshot(&server.clients[1].player);
    assert_eq!(
        prediction.player.body.position,
        server.clients[1].player.body.position
    );
}
#[test]
fn all_clients_and_prediction_use_identical_movement_on_foreign_geometry() {
    let cvars = qa_console::cvars::Cvars::new().unwrap();
    let handles = qa_console::input_settings::InputHandles::load(&cvars).unwrap();
    let mut server = Server::load(15, 64, 1, 0, 0, 0).unwrap();
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
    let rules = [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ];
    let mut predictions: [Prediction; 15] = std::array::from_fn(|_| Prediction::default());
    for (slot, prediction) in predictions.iter_mut().enumerate() {
        let connection = [Connection::Local, Connection::Remote, Connection::Bot][slot / 5];
        let id = server.connect(connection, ModuleId(2), None).unwrap();
        let client = &mut server.clients[slot];
        client.input_rules = rules[slot % 5];
        client.player.movement_rules = rules[slot % 5];
        client.player.weapon_state = Some(qa_core::primitives::WeaponState {
            source: RuleSetId::Quake,
            namespace: qa_core::primitives::ResourceNamespace::Module(ModuleId(1)),
            readiness: qa_core::primitives::WeaponReadiness::DeadlineSeconds(7.0),
        });
        client.player.trace_rules = rules[slot % 5];
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([0.0, slot as f32 * 128.0, 24.125]);
        client.player.movement.grounded = true;
        client.intent = CommandIntent::moving([1.0, 0.0, 0.0]);
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
                    movement: [127.0, 0.0, 0.0],
                    units: qa_gameplay::rules::command_units(rules[slot % 5]),
                    ..Default::default()
                },
                qa_core::primitives::CommandExecution::PerCommand,
                None,
            );
        }
    }
    server.build_bot_commands(EventTime(0), EventTime(16_000_000), |rules| {
        handles.policy(&cvars, rules)
    });
    assert_eq!(
        server.move_pending_clients(
            &store,
            geometry,
            0,
            &mut scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            reject_move
        ),
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
            CollisionOwner::Lifetime(client.entity),
        );
        prediction
            .advance(client.command, &mut trace)
            .expect("valid movement query");
        assert_eq!(client.player.body.position, prediction.player.body.position);
        assert_eq!(client.player.body.velocity, prediction.player.body.velocity);
        assert_eq!(client.player.movement, prediction.player.movement);
        assert_eq!(
            client.player.weapon_state,
            Some(qa_core::primitives::WeaponState {
                source: qa_core::primitives::RuleSetId::Quake,
                namespace: qa_core::primitives::ResourceNamespace::Module(
                    qa_core::primitives::ModuleId(1)
                ),
                readiness: qa_core::primitives::WeaponReadiness::DeadlineSeconds(7.0)
            })
        );
        assert!(client.player.body.position.0[0] > 0.0);
        assert_eq!(
            server.entities.columns.position[client.entity.slot as usize],
            client.player.body.position
        );
    }
    assert_eq!(
        server.move_pending_clients(
            &store,
            geometry,
            0,
            &mut scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            reject_move
        ),
        0
    );
}

#[test]
fn q2_bot_input_uses_the_human_builder_and_q3_movement_on_independent_trace_rules() {
    use qa_input::UserCmdBuilder;

    let cvars = qa_console::cvars::Cvars::new().unwrap();
    let handles = qa_console::input_settings::InputHandles::load(&cvars).unwrap();
    let mut server = Server::load(1, 8, 1, 0, 0, 0).unwrap();
    let id = server.connect(Connection::Bot, ModuleId(2), None).unwrap();
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
    let client = &mut server.clients[id.0 as usize];
    client.input_rules = RuleSetId::Quake2;
    client.player.movement_rules = RuleSetId::Quake3;
    client.player.trace_rules = RuleSetId::Quake2;
    client.player.health = 100;
    qa_movement::set_bounds(&mut client.player);
    client.player.body.position = Vec3([0.0, 0.0, 24.125]);
    client.player.movement.grounded = true;
    client.intent = CommandIntent::moving([1.0, 0.0, 0.0]);
    let entity = client.entity;
    let mut human = Prediction::default();
    human.player.health = client.player.health;
    human.apply_snapshot(&client.player);
    let mut normalized = Prediction::default();
    normalized.player.health = client.player.health;
    normalized.apply_snapshot(&client.player);
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

    let duration = EventTime::from_milliseconds(16);
    let command = UserCmdBuilder::build(
        duration,
        duration,
        server.clients[id.0 as usize].intent,
        handles.policy(&cvars, RuleSetId::Quake2),
    );
    assert_eq!(command.movement, [200.0, 0.0, 0.0]);
    assert_eq!(
        command.units,
        qa_gameplay::rules::command_units(RuleSetId::Quake2)
    );
    server.build_bot_commands(EventTime(0), duration, |rules| {
        handles.policy(&cvars, rules)
    });
    let bot = server.clients[id.0 as usize].command;
    assert_eq!(bot.movement, command.movement);
    assert_eq!(bot.units, command.units);
    assert_eq!(bot.duration_ms, command.duration_ms);
    assert_eq!(bot.duration_ns, command.duration_ns);
    assert_eq!(bot.server_time_ms, command.server_time_ms);
    assert_eq!(bot.view_angles, command.view_angles);
    assert_eq!(
        server.move_pending_clients(
            &store,
            geometry,
            0,
            &mut scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            reject_move
        ),
        1
    );
    let mut trace = WorldTrace::new(
        &store,
        geometry,
        0,
        &server.entities,
        &server.area,
        &mut scratch,
        CollisionOwner::Lifetime(entity),
    );
    human.advance(command, &mut trace).unwrap();
    let normalized_command = command.in_units(qa_gameplay::rules::command_units(RuleSetId::Quake3));
    assert_eq!(normalized_command.movement, [127.0, 0.0, 0.0]);
    normalized.advance(normalized_command, &mut trace).unwrap();
    let client = &server.clients[id.0 as usize];
    assert!(client.player.body.position.0[0] > 0.0);
    assert_eq!(client.input_rules, RuleSetId::Quake2);
    assert_eq!(client.player.movement_rules, RuleSetId::Quake3);
    assert_eq!(client.player.trace_rules, RuleSetId::Quake2);
    for prediction in [&human, &normalized] {
        assert_eq!(client.player.body.position, prediction.player.body.position);
        assert_eq!(client.player.body.velocity, prediction.player.body.velocity);
        assert_eq!(client.player.movement, prediction.player.movement);
    }
}

#[test]
fn authoritative_movement_skips_self_hits_another_client_and_unlinks_disconnects() {
    let mut server = Server::load(2, 16, 1, 0, 0, 0).unwrap();
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
    let moving = server
        .connect(Connection::Local, ModuleId(2), None)
        .unwrap();
    let obstacle = server
        .connect(Connection::Remote, ModuleId(1), None)
        .unwrap();
    for (id, x, rules, order) in [
        (moving, 0.0, RuleSetId::Quake3, LinkOrder::Tail),
        (obstacle, 48.0, RuleSetId::Quake2, LinkOrder::Head),
    ] {
        let client = &mut server.clients[id.0 as usize];
        client.input_rules = rules;
        client.player.movement_rules = rules;
        client.player.trace_rules = rules;
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
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
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
            CollisionOwner::Lifetime(mover_entity),
        );
        let own_position = trace
            .trace(TraceQuery {
                end: query.start,
                ..query
            })
            .expect("valid movement query");
        assert!(!own_position.start_solid && !own_position.all_solid);
        assert_eq!(own_position.fraction, 1.0);
        let other = trace.trace(query).expect("valid movement query");
        assert_eq!(other.entity, Some(obstacle_entity));
        assert!(other.fraction > 0.0 && other.fraction < 1.0);
    }
    server.submit_command(
        moving,
        UserCmd {
            duration_ms: 200,
            server_time_ms: 200,
            movement: [127.0, 0.0, 0.0],
            units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
            ..Default::default()
        },
        qa_core::primitives::CommandExecution::PerCommand,
        None,
    );
    assert!(
        server.move_pending_clients(
            &store,
            geometry,
            0,
            &mut scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            reject_move
        ) > 0
    );
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
        CollisionOwner::Lifetime(mover_entity),
    );
    let after_disconnect = trace
        .trace(TraceQuery {
            start: moved,
            ..query
        })
        .expect("valid movement query");
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
                    encoding: None,
                    normal: Vec3([0.0, 0.0, 1.0]),
                    distance,
                    axis: None,
                })
                .collect(),
            (0..2)
                .map(|first_plane| Brush {
                    sides: qa_core::primitives::IndexSpan {
                        first: first_plane,
                        count: 1,
                    },
                    contents: Contents::SOLID,
                })
                .collect(),
            Vec::new(),
            qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags(0); 2]),
            BrushTree {
                planes: Vec::new(),
                nodes: Vec::new(),
                leaves: (0..2)
                    .map(|first_brush| CollisionLeaf {
                        stored_contents: None,
                        brushes: qa_core::primitives::IndexSpan {
                            first: first_brush,
                            count: 1,
                        },
                        patches: Default::default(),
                    })
                    .collect(),
                leaf_brushes: vec![0, 1],
                models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                leaf_patches: Vec::new(),
            },
            vec![bounds; 2],
        )
        .unwrap();
    assert_ne!(geometry, first_geometry);
    let mut scratch = store.scratch();
    let mut server = Server::load(1, 8, 1, 0, 0, 0).unwrap();
    let id = server
        .connect(Connection::Local, ModuleId(2), None)
        .unwrap();
    let client = &mut server.clients[id.0 as usize];
    client.input_rules = RuleSetId::Quake3;
    client.player.movement_rules = RuleSetId::Quake3;
    client.player.trace_rules = RuleSetId::Quake3;
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
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
    );
    let selected = WorldTrace::new(
        &store,
        geometry,
        1,
        &server.entities,
        &server.area,
        &mut scratch,
        CollisionOwner::Lifetime(entity),
    )
    .trace(query)
    .expect("valid movement query");
    assert!(!selected.start_solid && selected.fraction > 0.0 && selected.fraction < 1.0);
    assert!(
        WorldTrace::new(
            &store,
            geometry,
            0,
            &server.entities,
            &server.area,
            &mut scratch,
            CollisionOwner::Lifetime(entity),
        )
        .trace(query)
        .expect("valid movement query")
        .all_solid
    );
    let command = UserCmd {
        duration_ms: 16,
        server_time_ms: 16,
        movement: [127.0, 0.0, 0.0],
        units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
        ..Default::default()
    };
    server.submit_command(
        id,
        command,
        qa_core::primitives::CommandExecution::PerCommand,
        None,
    );
    assert_eq!(
        server.move_pending_clients(
            &store,
            geometry,
            1,
            &mut scratch,
            qa_core::primitives::PatchClip::All,
            None,
            None,
            reject_move
        ),
        1
    );
    let mut trace = WorldTrace::new(
        &store,
        geometry,
        1,
        &server.entities,
        &server.area,
        &mut scratch,
        CollisionOwner::Lifetime(entity),
    );
    prediction
        .advance(command, &mut trace)
        .expect("valid movement query");
    let player = &server.clients[id.0 as usize].player;
    assert!(player.body.position.0[0] > 0.0 && player.movement.grounded);
    assert_eq!(player.body.position, prediction.player.body.position);
    assert_eq!(player.body.velocity, prediction.player.body.velocity);
    assert_eq!(player.movement, prediction.player.movement);
}

#[test]
fn prediction_copies_independent_trace_rules_and_matches_authoritative_contact()
-> Result<(), &'static str> {
    let (store, geometry) = floor();
    let mut scratch = store.scratch();
    for (trace_rules, expected_height) in
        [(RuleSetId::Quake3, 24.125), (RuleSetId::Quake2, 24.03125)]
    {
        let mut server = Server::load(1, 8, 1, 0, 0, 0).map_err(|_| "server capacity")?;
        let id = server
            .connect(Connection::Local, ModuleId(2), None)
            .ok_or("client slot")?;
        let client = &mut server.clients[id.0 as usize];
        client.player.view_weapon.frame = 29;
        client.input_rules = RuleSetId::Quake3;
        client.player.movement_rules = RuleSetId::Quake3;
        client.player.trace_rules = trace_rules;
        client.player.health = 100;
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([0.0, 0.0, 25.0]);
        client.player.body.velocity = Vec3([0.0, 0.0, -100.0]);
        client.player.movement.tuning.no_step = true;
        let entity = client.entity;
        let mut prediction = Prediction::default();
        prediction.player.trace_rules = RuleSetId::QuakeWorld;
        prediction.apply_snapshot(&client.player);
        assert_eq!(prediction.player.movement_rules, RuleSetId::Quake3);
        assert_eq!(prediction.player.trace_rules, trace_rules);
        if trace_rules == RuleSetId::Quake2 {
            assert_ne!(
                prediction.player.movement_rules,
                prediction.player.trace_rules
            );
        }
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
        let command = UserCmd {
            duration_ms: 16,
            server_time_ms: 16,
            ..Default::default()
        };
        server.submit_command(
            id,
            command,
            qa_core::primitives::CommandExecution::PerCommand,
            None,
        );
        assert_eq!(
            server.move_pending_clients(
                &store,
                geometry,
                0,
                &mut scratch,
                qa_core::primitives::PatchClip::All,
                None,
                None,
                reject_move
            ),
            1
        );
        let mut trace = WorldTrace::new(
            &store,
            geometry,
            0,
            &server.entities,
            &server.area,
            &mut scratch,
            CollisionOwner::Lifetime(entity),
        );
        assert!(
            prediction
                .advance(command, &mut trace)
                .expect("valid movement query")
                .traces
                > 0
        );
        let player = &server.clients[id.0 as usize].player;
        // Native Q2 DIST_EPSILON is 1/32; Q3 SURFACE_CLIP_EPSILON is 1/8.
        // Standing mins.z=-24 makes the distinct caller bias observable here.
        assert!((player.body.position.0[2] - expected_height).abs() < 0.00001);
        assert!(player.movement.grounded);
        assert_eq!(player.body.position, prediction.player.body.position);
        assert_eq!(player.body.velocity, prediction.player.body.velocity);
        assert_eq!(player.movement, prediction.player.movement);
        assert_eq!(player.trace_rules, prediction.player.trace_rules);
        assert_eq!(player.weapon_state, None);
        assert_eq!(player.view_weapon.frame, 29);
        assert_eq!(prediction.player.weapon_state, None);
    }
    Ok(())
}

fn reject_move(client: qa_core::primitives::ClientId, error: qa_world::collision::TraceError) {
    panic!("unexpected movement rejection for {client:?}: {error:?}");
}

#[test]
fn q3_respawn_gate_matches_server_and_prediction_replay_for_humans_and_bots() {
    use qa_core::{
        command_history::{Acknowledgement, CommandHistory},
        primitives::{CommandExecution, PlayerState, buttons},
    };

    let (store, geometry) = floor();
    let cvars = qa_console::cvars::Cvars::new().unwrap();
    let handles = qa_console::input_settings::InputHandles::load(&cvars).unwrap();
    for connection in [Connection::Local, Connection::Remote, Connection::Bot] {
        let mut server = Server::load(1, 8, 1, 0, 0, 0).unwrap();
        let id = server.connect(connection, ModuleId(2), None).unwrap();
        let client = &mut server.clients[id.0 as usize];
        client.input_rules = RuleSetId::Quake3;
        client.player.movement_rules = RuleSetId::Quake3;
        client.player.trace_rules = RuleSetId::Quake2;
        client.player.health = 100;
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([0.0, 0.0, 24.125]);
        client.player.movement.grounded = true;
        client.player.movement.respawned = true;
        let entity = client.entity;
        // Native health has already been accepted before the movement snapshot
        // is remembered, as in the production CLIENT projection path.
        let mut prediction = Prediction::new(PlayerState {
            health: client.player.health,
            ..Default::default()
        });
        prediction.apply_snapshot(&client.player);
        prediction.remember_snapshot(Some(Acknowledgement::Number(0)));
        assert!(prediction.player.movement.respawned);
        server
            .entities
            .columns
            .set_body(entity.slot as usize, client.player.body);
        assert!(server.area.link(
            &server.entities,
            entity,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
        let mut scratch = store.scratch();
        let mut history = CommandHistory::load(4, 0).unwrap();

        for (index, held) in [buttons::ATTACK | buttons::JUMP, buttons::JUMP]
            .into_iter()
            .enumerate()
        {
            let start = EventTime::from_milliseconds(index as u64 * 16);
            let end = EventTime::from_milliseconds((index as u64 + 1) * 16);
            if connection == Connection::Bot {
                server.clients[id.0 as usize].intent = CommandIntent {
                    buttons: held,
                    ..CommandIntent::moving([0.0, 0.0, 1.0])
                };
                server.build_bot_commands(start, end, |rules| handles.policy(&cvars, rules));
            } else {
                server.submit_command(
                    id,
                    UserCmd {
                        duration_ms: 16,
                        server_time_ms: end.milliseconds() as i32,
                        movement: [0.0, 0.0, 127.0],
                        units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                        buttons: held,
                        ..Default::default()
                    },
                    CommandExecution::PerCommand,
                    None,
                );
            }
            let command = server.clients[id.0 as usize].command;
            history.append(command);
            assert_eq!(
                server.move_pending_clients(
                    &store,
                    geometry,
                    0,
                    &mut scratch,
                    qa_core::primitives::PatchClip::All,
                    None,
                    None,
                    reject_move
                ),
                1
            );
            let mut trace = WorldTrace::new(
                &store,
                geometry,
                0,
                &server.entities,
                &server.area,
                &mut scratch,
                CollisionOwner::Lifetime(entity),
            );
            assert_eq!(prediction.replay(&history, &mut trace).unwrap(), 1);
            let authoritative = &server.clients[id.0 as usize].player;
            assert_eq!(prediction.player.body.position, authoritative.body.position);
            assert_eq!(prediction.player.body.velocity, authoritative.body.velocity);
            assert_eq!(prediction.player.movement, authoritative.movement);
            assert_eq!(prediction.player.trace_rules, RuleSetId::Quake2);
            assert_eq!(prediction.player.movement.respawned, index == 0);
            if index == 0 {
                assert!(authoritative.movement.grounded);
                assert!(!authoritative.movement.jump_held);
                assert_eq!(authoritative.body.velocity.0[2], 0.0);
            } else {
                assert!(authoritative.movement.jump_held);
                assert!(authoritative.body.position.0[2] > 24.125);
                assert!(authoritative.body.velocity.0[2] > 200.0);
            }
            // The next command replays from the accepted authoritative frame,
            // including its respawn latch, rather than from a second state copy.
            prediction.apply_snapshot(authoritative);
            prediction.remember_snapshot(Some(Acknowledgement::Number(history.number())));
            assert_eq!(prediction.replay(&history, &mut trace).unwrap(), 0);
            assert_eq!(prediction.player.movement, authoritative.movement);
        }
    }
}
