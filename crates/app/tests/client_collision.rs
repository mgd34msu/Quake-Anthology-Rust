#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    client_collision::{ClientCollision, ClientCollisionError},
    client_scene::ClientModelBindings,
    model_assets::{ModelCatalog, ModelCatalogBuilder, ModelLimits, ModelRequest},
};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{
        Bounds, ClientId, EntityAppearance, EntityId, GeometryId, IndexSpan, ModelRef,
        ResourceNamespace, RuleSetId, SurfaceFlags, ThinkTime, Vec3,
    },
    solids::SolidCodec,
};
use qa_network::{
    commands::packet::Protocol,
    projection::{EntityProjection, EntityRecord},
    snapshots::{Entity, Frame, ReceivedFrame},
    states,
};
use qa_render::{Assets, WorldPresentationId};
use qa_world::collision::{
    CollisionStore, Contents, Trace, TraceQuery,
    brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    surfaces::SurfaceTable,
};

struct Fixture {
    store: CollisionStore,
    geometry: GeometryId,
    catalog: ModelCatalog,
    configs: ConfigStrings,
    bindings: ClientModelBindings,
    client: ClientId,
    protocol: Protocol,
}
impl Fixture {
    fn new(protocol: Protocol, client: ClientId) -> Self {
        let mut store = CollisionStore::new();
        let bounds = Bounds {
            mins: Vec3([-8.; 3]),
            maxs: Vec3([8.; 3]),
        };
        let planes = (0..3)
            .flat_map(|axis| {
                [1., -1.].map(move |sign| {
                    let mut normal = Vec3::default();
                    normal.0[axis] = sign;
                    qa_core::primitives::Plane::oriented(normal, 8.)
                })
            })
            .collect();
        let geometry = store
            .load_brushes(
                planes,
                vec![Brush {
                    sides: IndexSpan { first: 0, count: 6 },
                    contents: Contents::SOLID,
                }],
                SurfaceTable::flags(vec![SurfaceFlags::default(); 6]),
                BrushTree {
                    planes: vec![],
                    nodes: vec![],
                    leaves: vec![
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan::default(),
                        },
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan { first: 0, count: 1 },
                        },
                    ],
                    leaf_brushes: vec![0],
                    models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                },
                vec![
                    Bounds {
                        mins: Vec3([-1024.; 3]),
                        maxs: Vec3([1024.; 3]),
                    },
                    bounds,
                ],
            )
            .unwrap();
        let mut assets = Assets::load().unwrap();
        let world = support::world(&mut assets, 0.);
        let presentation = WorldPresentationId(assets.world(world).unwrap().presentations().first);
        let profile = ModelRequest::native("", protocol.rule_set(), None).profile();
        let mut builder =
            ModelCatalogBuilder::new(assets, [profile], [world], ModelLimits::default()).unwrap();
        builder
            .bind_loaded_world(
                "maps/test.bsp",
                world,
                Some(geometry),
                store.model_bounds(geometry, 0).unwrap(),
            )
            .unwrap();
        let (_, catalog) = builder.finish();
        let layout = ConfigLayout::for_rules(protocol.rule_set());
        let mut configs =
            ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
                .unwrap();
        configs
            .set(layout.resources[0].get(1).unwrap(), b"maps/test.bsp")
            .unwrap();
        configs
            .set(layout.resources[0].get(2).unwrap(), b"*1")
            .unwrap();
        let mut bindings = ClientModelBindings::load(client, layout);
        assert_eq!(
            bindings.bind_all(&configs, &catalog, profile, Some((world, presentation))),
            0
        );
        Self {
            store,
            geometry,
            catalog,
            configs,
            bindings,
            client,
            protocol,
        }
    }
    fn record(&self, x: f32, solid: u32) -> EntityRecord {
        EntityRecord {
            position: Vec3([x, 0., 0.]),
            angles: Vec3([0., 90., 0.]),
            appearance: EntityAppearance {
                source: self.protocol.rule_set(),
                native: qa_core::primitives::NativeAppearanceTail {
                    solid,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn row<const N: usize>(&self, number: u32, record: EntityRecord) -> Entity<N> {
        let mut words = [0; N];
        assert!(EntityProjection::load(self.protocol).reduce(
            &record,
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        Entity { number, words }
    }
    fn trace(&self, view: &mut ClientCollision) -> Trace {
        let policy = qa_world::collision::trace_policy(self.protocol.rule_set());
        view.trace(&self.store)
            .unwrap()
            .trace(TraceQuery {
                mask: Contents::SOLID | Contents::BODY,
                ..TraceQuery::point(
                    Vec3([-64., 0., 0.]),
                    Vec3([64., 0., 0.]),
                    policy.0,
                    policy.1,
                )
            })
            .unwrap()
    }
}
fn frame<'a, const P: usize, const E: usize>(
    sequence: u32,
    player: &'a [u32; P],
    entities: &'a [Entity<E>],
) -> Frame<'a, P, E> {
    Frame {
        sequence,
        time: ThinkTime::Milliseconds(i64::from(sequence) * 100),
        command: 0,
        flags: 0,
        qw_peers: 0,
        qw_dead: 0,
        areas: &[],
        player,
        entities,
    }
}
fn q2_bounds() -> u32 {
    SolidCodec::QUAKE2.pack_bounds(Bounds {
        mins: Vec3([-8., -8., -8.]),
        maxs: Vec3([8., 8., 8.]),
    })
}

#[test]
fn accepted_client_pose_is_owned_and_repeated_or_late_rows_do_not_replace_it() {
    let fixture = Fixture::new(Protocol::Quake2_34, ClientId(0));
    let projection = EntityProjection::load(fixture.protocol);
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    let player = [0; states::Q2_PLAYER_WORDS];
    let rows = [fixture.row::<{ states::Q2_ENTITY_WORDS }>(7, fixture.record(0., q2_bounds()))];
    view.synchronize(
        ReceivedFrame::Quake2(frame(2, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        Some(1),
    )
    .unwrap();
    let old = view.entity(7).unwrap();
    assert_eq!(old.slot, 1);
    let original = fixture.trace(&mut view);
    assert_eq!(original.entity, Some(old));
    // This mutable source stands in for SERVER or a reused decoder scratch;
    // neither is borrowed by the accepted CLIENT collision columns.
    let mut changed = rows;
    changed[0] = fixture.row(7, fixture.record(48., q2_bounds()));
    let trace = fixture.trace(&mut view);
    assert_eq!(
        (
            trace.fraction.to_bits(),
            trace.end,
            trace.entity,
            trace.start_solid,
            trace.all_solid
        ),
        (
            original.fraction.to_bits(),
            original.end,
            original.entity,
            original.start_solid,
            original.all_solid
        )
    );
    for sequence in [2, 1] {
        view.synchronize(
            ReceivedFrame::Quake2(frame(sequence, &player, &changed)),
            &projection,
            &fixture.bindings,
            &fixture.store,
            Some(1),
        )
        .unwrap();
        let trace = fixture.trace(&mut view);
        assert_eq!(
            (
                trace.fraction.to_bits(),
                trace.end,
                trace.entity,
                trace.start_solid,
                trace.all_solid
            ),
            (
                original.fraction.to_bits(),
                original.end,
                original.entity,
                original.start_solid,
                original.all_solid
            )
        );
        assert_eq!(view.entity(7), Some(old));
    }
    view.synchronize(
        ReceivedFrame::Quake2(frame(3, &player, &changed)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        Some(1),
    )
    .unwrap();
    assert_eq!(view.entity(7), Some(old));
    assert!(fixture.trace(&mut view).fraction > original.fraction);
}

#[test]
fn removal_reset_and_ordinal_reuse_create_client_lifetimes() {
    let fixture = Fixture::new(Protocol::Quake2_34, ClientId(2));
    let projection = EntityProjection::load(fixture.protocol);
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    let player = [0; states::Q2_PLAYER_WORDS];
    let rows = [fixture.row::<{ states::Q2_ENTITY_WORDS }>(7, fixture.record(0., q2_bounds()))];
    view.synchronize(
        ReceivedFrame::Quake2(frame(1, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    let old = view.entity(7).unwrap();
    view.synchronize(
        ReceivedFrame::Quake2(frame::<
            { states::Q2_PLAYER_WORDS },
            { states::Q2_ENTITY_WORDS },
        >(2, &player, &[])),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert_eq!(view.entity(7), None);
    view.synchronize(
        ReceivedFrame::Quake2(frame(3, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert_ne!(view.entity(7), Some(old));
    let new = view.entity(7).unwrap();
    view.reset();
    assert_eq!(view.entity(7), None);
    assert!(matches!(
        view.trace(&fixture.store),
        Err(ClientCollisionError::Waiting)
    ));
    view.synchronize(
        ReceivedFrame::Quake2(frame(1, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert_ne!(view.entity(7), Some(new));
}

#[test]
fn unknown_or_stale_world_rejects_only_its_client_without_server_fallback() {
    let mut fixture = Fixture::new(Protocol::Quake2_34, ClientId(0));
    let projection = EntityProjection::load(fixture.protocol);
    let player = [0; states::Q2_PLAYER_WORDS];
    let rows = [fixture.row::<{ states::Q2_ENTITY_WORDS }>(7, fixture.record(0., q2_bounds()))];
    let mut healthy =
        ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    let mut rejected =
        ClientCollision::load(ClientId(1), fixture.protocol, &fixture.store).unwrap();
    healthy
        .synchronize(
            ReceivedFrame::Quake2(frame(1, &player, &rows)),
            &projection,
            &fixture.bindings,
            &fixture.store,
            None,
        )
        .unwrap();
    let unknown = ClientModelBindings::load(ClientId(1), ConfigLayout::QUAKE2);
    assert_eq!(
        rejected.synchronize(
            ReceivedFrame::Quake2(frame(1, &player, &rows)),
            &projection,
            &unknown,
            &fixture.store,
            None
        ),
        Err(ClientCollisionError::Waiting)
    );
    assert_eq!(rejected.rejected_snapshots, 1);
    assert!(fixture.trace(&mut healthy).fraction < 1.);
    assert!(fixture.store.remove(fixture.geometry));
    assert_eq!(
        healthy.synchronize(
            ReceivedFrame::Quake2(frame(2, &player, &rows)),
            &projection,
            &fixture.bindings,
            &fixture.store,
            None
        ),
        Err(ClientCollisionError::Geometry)
    );
    assert!(matches!(
        healthy.trace(&fixture.store),
        Err(ClientCollisionError::Waiting)
    ));
    assert_eq!(healthy.entity(7), None);
}

#[test]
fn q3_world_self_and_trigger_classification_keep_native_numbers_distinct() {
    let fixture = Fixture::new(Protocol::Quake3_68, ClientId(0));
    let projection = EntityProjection::load(fixture.protocol);
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    let player = [0; states::PLAYER_WORDS];
    let solid = SolidCodec::QUAKE3.pack_bounds(Bounds {
        mins: Vec3([-8.; 3]),
        maxs: Vec3([8.; 3]),
    });
    for (sequence, kind, blocks) in [
        (1, 2, false),
        (2, 8, false),
        (3, 9, false),
        (4, 10, true),
        (5, 11, true),
    ] {
        let mut target = fixture.record(0., solid);
        target.appearance.native.entity_type = kind;
        let rows = [
            fixture.row::<{ states::ENTITY_WORDS }>(0, fixture.record(-32., solid)),
            fixture.row(3, target),
        ];
        view.synchronize(
            ReceivedFrame::Quake3(frame(sequence, &player, &rows)),
            &projection,
            &fixture.bindings,
            &fixture.store,
            Some(0),
        )
        .unwrap();
        assert_eq!(
            view.entity(1022),
            Some(EntityId {
                slot: 0,
                generation: 1
            })
        );
        assert_ne!(view.entity(0), view.entity(1022));
        assert_eq!(view.entity(1023), None);
        assert_eq!(fixture.trace(&mut view).fraction < 1., blocks);
    }
}

#[test]
fn q2_box_angles_are_ignored_and_native_order_breaks_equal_hits() {
    let fixture = Fixture::new(Protocol::Quake2Repro1038, ClientId(0));
    let projection = EntityProjection::load(fixture.protocol);
    let player = [0; states::Q2_REPRO_PLAYER_WORDS];
    let solid = SolidCodec::QUAKE2_RERELEASE.pack_bounds(Bounds {
        mins: Vec3([-8., -24., -8.]),
        maxs: Vec3([8., 24., 8.]),
    });
    let first = fixture.record(0., solid);
    let rows = [
        fixture.row::<{ states::Q2_RERELEASE_ENTITY_WORDS }>(17, first),
        fixture.row(3, first),
    ];
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    view.synchronize(
        ReceivedFrame::Quake2Repro(frame(1, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    let trace = fixture.trace(&mut view);
    assert_eq!(trace.entity, view.entity(17));
    assert!(trace.end.0[0] > -9. && trace.end.0[0] < -8.);
    let reversed = [rows[1], rows[0]];
    view.synchronize(
        ReceivedFrame::Quake2Repro(frame(2, &player, &reversed)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert_eq!(fixture.trace(&mut view).entity, view.entity(3));
}

#[test]
fn qw_brushes_precede_living_peers_and_dead_peers_do_not_block() {
    let fixture = Fixture::new(Protocol::QuakeWorld28, ClientId(0));
    let projection = EntityProjection::load(fixture.protocol);
    let player = [0; 0];
    let mut brush = fixture.record(0., 0);
    brush.appearance.models[0] = ModelRef::native(ResourceNamespace::Client(fixture.client), 2);
    let rows = [
        fixture.row::<{ states::QW_ENTITY_WORDS }>(3, fixture.record(40., 0)),
        fixture.row(48, brush),
        fixture.row(2, fixture.record(40., 0)),
    ];
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    let mut accepted = frame(1, &player, &rows);
    accepted.qw_peers = (1 << 1) | (1 << 2);
    accepted.qw_dead = accepted.qw_peers;
    view.synchronize(
        ReceivedFrame::QuakeWorld(accepted),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert_eq!(fixture.trace(&mut view).entity, view.entity(48));
    let peers = [rows[0], rows[2]];
    let mut accepted = frame(2, &player, &peers);
    accepted.qw_peers = (1 << 1) | (1 << 2);
    accepted.qw_dead = accepted.qw_peers;
    view.synchronize(
        ReceivedFrame::QuakeWorld(accepted),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert_eq!(fixture.trace(&mut view).fraction, 1.);
    accepted.sequence = 3;
    accepted.qw_dead = 0;
    view.synchronize(
        ReceivedFrame::QuakeWorld(accepted),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
    )
    .unwrap();
    assert!(fixture.trace(&mut view).fraction < 1.);
}

#[test]
fn queued_snapshot_collision_uses_wire_pose_while_server_moves_independently() {
    use qa_app::{Runtime, client_policy::ClientPolicy, host::FrameHost, map::SpawnAnchor};
    use qa_console::{commands::Console, views::Context};
    use qa_core::{
        loopback::Endpoint,
        primitives::{ModuleId, NativeEntity},
        sys_events::{EventKind, EventTime, Peer, SeatId, SysEvent},
    };
    use qa_network::{
        channel::Channel,
        message::{Encoding, Writer},
    };
    use qa_session::timing::TickRate;
    let fixture = Fixture::new(Protocol::Quake2_34, ClientId(0));
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.geometry = fixture.store;
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(Some(RuleSetId::Quake2), None, None, None).unwrap(),
            Protocol::Quake2_34,
            0,
        )
        .unwrap();
    assert_eq!(client, fixture.client);
    let state = &mut runtime.client_resources[client.0 as usize]
        .as_mut()
        .unwrap()
        .state;
    state.models = fixture.bindings;
    state.configs = fixture.configs;
    runtime.models = Some(std::sync::Arc::new(fixture.catalog));
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let server = host
        .runtime
        .server
        .entities
        .allocate(
            ThinkTime::Milliseconds(0),
            ModuleId(7),
            qa_world::entities::AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    host.runtime.server.entities.columns.native_entity[server.slot as usize] = Some(NativeEntity {
        module: ModuleId(7),
        slot: 7,
    });
    host.runtime.server.entities.columns.position[server.slot as usize] = Vec3([1000., 0., 0.]);
    let protocol = Protocol::Quake2_34;
    let projection = EntityProjection::load(protocol);
    let mut record = EntityRecord {
        position: Vec3([19.99, 0., 0.]),
        appearance: EntityAppearance {
            source: RuleSetId::Quake2,
            ..Default::default()
        },
        ..Default::default()
    };
    record.appearance.native.solid = q2_bounds();
    let mut words = [0; states::Q2_ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal())
    ));
    let rows = [Entity { number: 7, words }];
    let player = [0; states::Q2_PLAYER_WORDS];
    let mut channel = Channel::load(protocol.channel(), Endpoint::Server, 32768, 8).unwrap();
    channel.configure_snapshots(protocol).unwrap();
    let sequence = channel.send_state().sequence;
    channel
        .publish_written_snapshot(ReceivedFrame::Quake2(frame(sequence, &player, &rows)), None)
        .unwrap();
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    channel
        .write_snapshot(&mut writer, sequence, None, 0)
        .unwrap();
    let length = writer.size();
    let queued = EventTime::from_milliseconds(100);
    let packet = channel
        .prepare_snapshot(sequence, None, &bytes[..length], queued)
        .unwrap()
        .unwrap()
        .bytes
        .to_vec();
    channel.submitted(queued).unwrap();
    assert!(matches!(
        host.runtime.client_collision[client.0 as usize]
            .as_mut()
            .unwrap()
            .trace(&host.runtime.geometry),
        Err(ClientCollisionError::Waiting)
    ));
    host.queue
        .push(SysEvent {
            time: queued,
            kind: EventKind::Packet {
                socket: Endpoint::Client.socket(),
                from: Peer::Loopback(client),
                bytes: &packet,
            },
        })
        .unwrap();
    host.queue
        .push(SysEvent {
            time: queued,
            kind: EventKind::Time,
        })
        .unwrap();
    host.drain(&mut Default::default());
    let query = || {
        let policy = qa_world::collision::trace_policy(RuleSetId::Quake2);
        TraceQuery {
            mask: Contents::BODY | Contents::SOLID,
            ..TraceQuery::point(
                Vec3([-64., 0., 0.]),
                Vec3([64., 0., 0.]),
                policy.0,
                policy.1,
            )
        }
    };
    let view = host.runtime.client_collision[client.0 as usize]
        .as_mut()
        .unwrap();
    let accepted = view.entity(7).unwrap();
    let first = view
        .trace(&host.runtime.geometry)
        .unwrap()
        .trace(query())
        .unwrap();
    assert_eq!(first.entity, Some(accepted));
    // The wire rounds19.99 down to19.875 before subtracting box8 and native epsilon1/32.
    assert_eq!(
        first.end.0[0].to_bits(),
        (19.875_f32 - 8. - 1. / 32.).to_bits()
    );
    host.runtime.server.entities.columns.position[server.slot as usize] = Vec3([-1000., 0., 0.]);
    let second = view
        .trace(&host.runtime.geometry)
        .unwrap()
        .trace(query())
        .unwrap();
    assert_eq!(second.fraction.to_bits(), first.fraction.to_bits());
    assert_eq!(second.entity, Some(accepted));
    // No new packet is published by this fixture: an unchanged physical intake
    // cannot import the SERVER body's different pose through prediction.
}

#[test]
fn rerelease_full_native_entity_capacity_reserves_only_the_world() {
    let fixture = Fixture::new(Protocol::Quake2Repro1038, ClientId(0));
    let player = [0; states::Q2_REPRO_PLAYER_WORDS];
    let rows: Vec<_> = (1..8192)
        .map(|number| {
            fixture.row::<{ states::Q2_RERELEASE_ENTITY_WORDS }>(
                number,
                fixture.record(number as f32, 0),
            )
        })
        .collect();
    let projection = EntityProjection::load(fixture.protocol);
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    view.synchronize(
        ReceivedFrame::Quake2Repro(frame(1, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        Some(1),
    )
    .unwrap();
    assert_eq!(
        view.entity(0),
        Some(EntityId {
            slot: 0,
            generation: 1
        })
    );
    assert_eq!(view.entity(8191).unwrap().slot, 8191);
    assert_eq!(view.rejected_snapshots, 0);
}
