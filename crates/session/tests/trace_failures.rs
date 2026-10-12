use qa_core::primitives::{
    Body, Bounds, CollisionOwner, CollisionShape, CommandExecution, ModuleId, Plane, PlayerTail,
    RuleSetId, SurfaceFlags, ThinkTime, UserCmd, Vec3,
};
use qa_session::{
    clients::{Connection, Server},
    prediction::Prediction,
};
use qa_world::{
    area::{LinkFlags, LinkIntent, LinkOrder},
    collision::{
        CollisionStore, Contents, TraceError, WorldTrace,
        brushes::{Brush, BrushTree},
    },
    entities::AllocationPolicy,
};

fn floor(store: &mut CollisionStore) -> qa_core::primitives::GeometryId {
    store
        .load_brushes(
            vec![Plane::oriented(Vec3([0.0, 0.0, 1.0]), 0.0)],
            vec![Brush {
                sides: qa_core::primitives::IndexSpan { first: 0, count: 1 },
                contents: Contents::SOLID,
            }],
            qa_world::collision::surfaces::SurfaceTable::flags(vec![SurfaceFlags::default()]),
            BrushTree::direct(1).unwrap(),
            vec![Bounds {
                mins: Vec3([-65536.0; 3]),
                maxs: Vec3([65536.0; 3]),
            }],
        )
        .unwrap()
}
fn body_bits(body: Body) -> [[u32; 3]; 4] {
    [body.position, body.velocity, body.mins, body.maxs].map(|row| row.0.map(f32::to_bits))
}

#[test]
fn failed_client_is_consumed_once_and_healthy_client_commits() {
    let mut server = Server::load(2, 16, 1, 0, 0, 0).unwrap();
    let mut store = CollisionStore::new();
    let geometry = floor(&mut store);
    let stale = floor(&mut store);
    let mut scratch = store.scratch();
    assert!(store.remove(stale));
    for slot in 0..2 {
        let id = server
            .connect(Connection::Local, ModuleId(2), PlayerTail::None, None)
            .unwrap();
        assert_eq!(id.0 as usize, slot);
        let client = &mut server.clients[slot];
        client.player.movement_rules = RuleSetId::Quake3;
        client.player.trace_rules = RuleSetId::Quake3;
        qa_movement::set_bounds(&mut client.player);
        client.player.body.position = Vec3([slot as f32 * 1024.0, 0.0, 64.0]);
        server
            .entities
            .columns
            .set_body(client.entity.slot as usize, client.player.body);
        assert!(server.area.link(
            &server.entities,
            client.entity,
            LinkFlags::SOLID,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
        server.submit_command(
            id,
            UserCmd {
                duration_ms: 16,
                server_time_ms: 16,
                movement: [127.0, 0.0, 0.0],
                ..Default::default()
            },
            CommandExecution::PerCommand,
            None,
        );
    }
    let bad = server.clients[0].entity;
    let good = server.clients[1].entity;
    let old_bad = body_bits(server.clients[0].player.body);
    let old_bad_link = server.area.bounds(bad);
    let old_good = server.clients[1].player.body.position;
    let obstacle = server
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(3),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = obstacle.slot as usize;
    server.entities.columns.set_body(
        slot,
        Body {
            position: Vec3([0.0, 0.0, 64.0]),
            mins: Vec3([-32.0; 3]),
            maxs: Vec3([32.0; 3]),
            ..Default::default()
        },
    );
    server.entities.columns.collision_shape[slot] = CollisionShape::Model {
        geometry: stale,
        index: 0,
    };
    server.entities.columns.collision_contents[slot] = Contents::BODY.0;
    assert!(server.area.link(
        &server.entities,
        obstacle,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    let mut rejected = Vec::new();
    assert_eq!(
        server.move_pending_clients(&store, geometry, 0, &mut scratch, |id, error| rejected
            .push((id, error))),
        1
    );
    assert_eq!(
        rejected,
        vec![(
            qa_core::primitives::ClientId(0),
            TraceError::Geometry {
                geometry: stale,
                model: 0
            }
        )]
    );
    assert_eq!(body_bits(server.clients[0].player.body), old_bad);
    assert_eq!(
        body_bits(server.entities.columns.body(bad.slot as usize)),
        old_bad
    );
    assert_eq!(server.area.bounds(bad), old_bad_link);
    assert!(server.clients[1].player.body.position.0[0] > old_good.0[0]);
    assert_eq!(
        body_bits(server.entities.columns.body(good.slot as usize)),
        body_bits(server.clients[1].player.body)
    );
    assert_eq!(
        server.move_pending_clients(&store, geometry, 0, &mut scratch, |id, error| rejected
            .push((id, error))),
        0
    );
    assert_eq!(rejected.len(), 1);
}

#[test]
fn prediction_rejects_stale_world_without_changing_its_hot_state() {
    let mut server = Server::load(1, 8, 1, 0, 0, 0).unwrap();
    let mut store = CollisionStore::new();
    let geometry = floor(&mut store);
    let mut scratch = store.scratch();
    assert!(store.remove(geometry));
    let id = server
        .connect(Connection::Local, ModuleId(2), PlayerTail::None, None)
        .unwrap();
    let source = &mut server.clients[id.0 as usize].player;
    source.movement_rules = RuleSetId::Quake3;
    source.trace_rules = RuleSetId::Quake3;
    qa_movement::set_bounds(source);
    source.body.position = Vec3([0.0, 0.0, 64.0]);
    let mut prediction = Prediction::default();
    prediction.apply_snapshot(source);
    let body = body_bits(prediction.player.body);
    let movement = prediction.player.movement;
    let angles = prediction.player.view_angles;
    let offset = prediction.player.view_offset;
    let mut trace = WorldTrace::new(
        &store,
        geometry,
        0,
        &server.entities,
        &server.area,
        &mut scratch,
        CollisionOwner::None,
    );
    assert_eq!(
        prediction
            .advance(
                UserCmd {
                    duration_ms: 16,
                    server_time_ms: 16,
                    view_angles: Vec3([10.0, 20.0, 30.0]),
                    ..Default::default()
                },
                &mut trace
            )
            .unwrap_err(),
        TraceError::Geometry { geometry, model: 0 }
    );
    assert_eq!(body_bits(prediction.player.body), body);
    assert_eq!(prediction.player.movement, movement);
    assert_eq!(prediction.player.view_angles, angles);
    assert_eq!(prediction.player.view_offset, offset);
}
