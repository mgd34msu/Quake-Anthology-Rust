#[path = "model_assets/support.rs"]
mod support;

use qa_app::{
    client_collision::{ClientCollision, ClientCollisionError},
    client_scene::ClientModelBindings,
    model_assets::{ModelAssets, ModelLimits, ModelRequest},
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
    projection::{EntityContext, EntityProjection, EntityRecord},
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
    vfs: qa_content::vfs::Vfs,
    store: CollisionStore,
    geometry: GeometryId,
    models: ModelAssets,
    configs: ConfigStrings,
    bindings: ClientModelBindings,
    client: ClientId,
    protocol: Protocol,
    world: qa_render::world::WorldId,
    presentation: WorldPresentationId,
}
impl Fixture {
    fn new(protocol: Protocol, client: ClientId) -> Self {
        let vfs = qa_content::vfs::Vfs::default();
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
                Vec::new(),
                SurfaceTable::flags(vec![SurfaceFlags::default(); 6]),
                BrushTree {
                    planes: vec![],
                    nodes: vec![],
                    leaves: vec![
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan::default(),
                            patches: Default::default(),
                        },
                        CollisionLeaf {
                            stored_contents: None,
                            brushes: IndexSpan { first: 0, count: 1 },
                            patches: Default::default(),
                        },
                    ],
                    leaf_brushes: vec![0],
                    models: vec![ModelRoot::Leaf(0), ModelRoot::Leaf(1)],
                    leaf_patches: Vec::new(),
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
        let presentation = assets.world(world).unwrap().presentations()[0];
        let profile = ModelRequest::native("", protocol.rule_set(), None).profile();
        let mut builder =
            ModelAssets::new(&vfs, assets, [profile], [world], ModelLimits::default()).unwrap();
        builder
            .bind_loaded_world(
                &vfs,
                "maps/test.bsp",
                world,
                Some(geometry),
                store.model_bounds(geometry, 0).unwrap(),
                None,
                None,
            )
            .unwrap();
        builder.end_initial_registration();
        let catalog = builder.catalog();
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
            bindings.bind_all(
                &configs,
                catalog,
                profile,
                Some((world, presentation)),
                None
            ),
            0
        );
        Self {
            vfs,
            store,
            geometry,
            models: builder,
            configs,
            bindings,
            client,
            protocol,
            world,
            presentation,
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
            &EntityContext::default(),
            &mut words,
            |model, _usage| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal())
        ));
        Entity { number, words }
    }
    fn trace(&self, view: &mut ClientCollision) -> Trace {
        let policy = qa_world::collision::trace_policy(self.protocol.rule_set());
        view.trace(&self.store, qa_core::primitives::PatchClip::All)
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

    fn resources(&self, client: ClientId) -> qa_app::client_resources::ClientResources {
        let mut resources = qa_app::client_resources::ClientResources::load(
            client,
            self.protocol,
            self.protocol.rule_set(),
            &self.vfs,
            None,
        )
        .unwrap()
        .unwrap();
        let state = &mut resources.state;
        for ordinal in [1, 2] {
            let index = state.layout.resources[0].get(ordinal).unwrap();
            state
                .configs
                .set(index, self.configs.config(index).unwrap().text)
                .unwrap();
        }
        assert_eq!(
            state.models.bind_all(
                &state.configs,
                self.models.catalog(),
                ModelRequest::native("", self.protocol.rule_set(), None).profile(),
                Some((self.world, self.presentation)),
                None
            ),
            0
        );
        resources
    }
}

struct QueuedSource(qa_core::sys_events::EventTime);
impl qa_app::host::FrameSource for QueuedSource {
    fn begin_frame(&mut self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.0.0)
    }
    fn poll_events(&mut self, queue: &mut qa_core::sys_events::SysEventQueue) {
        queue
            .push(qa_core::sys_events::SysEvent {
                time: self.0,
                kind: qa_core::sys_events::EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: std::time::Duration) -> std::time::Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
    fn present(&mut self) {}
}

fn accept_local_frames(host: &mut qa_app::host::FrameHost) {
    for milliseconds in [100, 125, 150] {
        host.frame(
            &mut QueuedSource(qa_core::sys_events::EventTime::from_milliseconds(
                milliseconds,
            )),
            true,
        );
    }
}

fn player_mask_trace(view: &mut ClientCollision, store: &CollisionStore, players: bool) -> Trace {
    let policy = qa_world::collision::trace_policy(RuleSetId::Quake2Rerelease);
    view.trace(store, qa_core::primitives::PatchClip::All)
        .unwrap()
        .trace(TraceQuery {
            mask: Contents::SOLID
                | Contents::BODY
                | if players {
                    Contents::PLAYER
                } else {
                    Contents::default()
                },
            ..TraceQuery::point(
                Vec3([-64., 0., 0.]),
                Vec3([64., 0., 0.]),
                policy.0,
                policy.1,
            )
        })
        .unwrap()
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
        0,
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
            0,
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
        0,
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
        0,
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
        0,
    )
    .unwrap();
    assert_eq!(view.entity(7), None);
    view.synchronize(
        ReceivedFrame::Quake2(frame(3, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
        0,
    )
    .unwrap();
    assert_ne!(view.entity(7), Some(old));
    let new = view.entity(7).unwrap();
    view.reset();
    assert_eq!(view.entity(7), None);
    assert!(matches!(
        view.trace(&fixture.store, qa_core::primitives::PatchClip::All),
        Err(ClientCollisionError::Waiting)
    ));
    view.synchronize(
        ReceivedFrame::Quake2(frame(1, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
        0,
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
            0,
        )
        .unwrap();
    let unknown = ClientModelBindings::load(ClientId(1), ConfigLayout::QUAKE2);
    assert_eq!(
        rejected.synchronize(
            ReceivedFrame::Quake2(frame(1, &player, &rows)),
            &projection,
            &unknown,
            &fixture.store,
            None,
            0,
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
            None,
            0,
        ),
        Err(ClientCollisionError::Geometry)
    );
    assert!(matches!(
        healthy.trace(&fixture.store, qa_core::primitives::PatchClip::All),
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
            0,
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
        0,
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
        0,
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
        0,
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
        0,
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
        0,
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
            ClientPolicy::select(Some(RuleSetId::Quake2), None, None, None, None).unwrap(),
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
    runtime.model_assets = Some(fixture.models);
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
        &EntityContext::default(),
        &mut words,
        |model, _usage| Some(model.ordinal),
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
            .trace(&host.runtime.geometry, qa_core::primitives::PatchClip::All),
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
        .trace(&host.runtime.geometry, qa_core::primitives::PatchClip::All)
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
        .trace(&host.runtime.geometry, qa_core::primitives::PatchClip::All)
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
        0,
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

fn native_player_range<const P: usize, const E: usize>(
    protocol: Protocol,
    wrap: for<'a> fn(Frame<'a, P, E>) -> ReceivedFrame<'a>,
) {
    let fixture = Fixture::new(protocol, ClientId(0));
    let mut resources = fixture.resources(fixture.client);
    let index = resources.state.layout.max_clients().unwrap();
    let projection = EntityProjection::load(protocol);
    let bounds = Bounds {
        mins: Vec3([-8.; 3]),
        maxs: Vec3([8.; 3]),
    };
    let solid = SolidCodec::load(protocol.rule_set())
        .unwrap()
        .pack_bounds(bounds);
    let rows = [
        fixture.row::<E>(2, fixture.record(-24., solid)),
        fixture.row(3, fixture.record(24., solid)),
    ];
    let player = [0; P];
    let mut view = ClientCollision::load(fixture.client, protocol, &fixture.store).unwrap();
    // The accepted snapshot does not change while only CLIENT's numeric row does.
    for (text, expected, player_skipped) in [
        (b"".as_slice(), 0, false),
        (b"2suffix", 2, true),
        (b"0", 0, false),
        (b"-1", 0, false),
        (b"257", 0, false),
        (b"2", 2, true),
    ] {
        assert!(resources.apply(index, text, Some(fixture.models.catalog()), None));
        assert_eq!(resources.state.max_clients(), expected);
        view.synchronize(
            wrap(frame(1, &player, &rows)),
            &projection,
            &resources.state.models,
            &fixture.store,
            None,
            resources.state.max_clients(),
        )
        .unwrap();
        let first = view.entity(2).unwrap();
        let monster = view.entity(3).unwrap();
        assert_eq!(
            player_mask_trace(&mut view, &fixture.store, true).entity,
            Some(first)
        );
        assert_eq!(
            player_mask_trace(&mut view, &fixture.store, false).entity,
            Some(if player_skipped { monster } else { first }),
            "protocol={protocol:?}, maxclients={text:?}",
        );
    }
}

#[test]
fn client_integer_maxclients_retags_same_snapshot_without_ignoring_monsters() {
    fn classic(
        frame: Frame<'_, { states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>,
    ) -> ReceivedFrame<'_> {
        ReceivedFrame::Quake2(frame)
    }
    fn rerelease(
        frame: Frame<'_, { states::Q2_REPRO_PLAYER_WORDS }, { states::Q2_RERELEASE_ENTITY_WORDS }>,
    ) -> ReceivedFrame<'_> {
        ReceivedFrame::Quake2Repro(frame)
    }
    native_player_range::<{ states::Q2_PLAYER_WORDS }, { states::Q2_ENTITY_WORDS }>(
        Protocol::Quake2_34,
        classic,
    );
    native_player_range::<{ states::Q2_REPRO_PLAYER_WORDS }, { states::Q2_RERELEASE_ENTITY_WORDS }>(
        Protocol::Quake2Repro1038,
        rerelease,
    );
}

#[test]
fn q3_player_tags_use_existing_entity_role_instead_of_native_number() {
    let fixture = Fixture::new(Protocol::Quake3_68, ClientId(0));
    let projection = EntityProjection::load(fixture.protocol);
    let solid = SolidCodec::QUAKE3.pack_bounds(Bounds {
        mins: Vec3([-8.; 3]),
        maxs: Vec3([8.; 3]),
    });
    let mut player_record = fixture.record(-24., solid);
    player_record.appearance.native.entity_type = 1;
    let rows = [
        fixture.row::<{ states::ENTITY_WORDS }>(63, player_record),
        fixture.row(2, fixture.record(24., solid)),
    ];
    let player = [0; states::PLAYER_WORDS];
    let mut view = ClientCollision::load(fixture.client, fixture.protocol, &fixture.store).unwrap();
    view.synchronize(
        ReceivedFrame::Quake3(frame(1, &player, &rows)),
        &projection,
        &fixture.bindings,
        &fixture.store,
        None,
        0,
    )
    .unwrap();
    let native_player = view.entity(63);
    let monster = view.entity(2);
    assert_eq!(
        player_mask_trace(&mut view, &fixture.store, true).entity,
        native_player
    );
    assert_eq!(
        player_mask_trace(&mut view, &fixture.store, false).entity,
        monster
    );
}

#[test]
fn queued_client_maxclients_and_snapshots_classify_each_split_client_independently() {
    use qa_app::{Runtime, client_policy::ClientPolicy, host::FrameHost, map::SpawnAnchor};
    use qa_console::{commands::Console, views::Context};
    use qa_core::{
        primitives::{Body, ModuleId},
        sys_events::SeatId,
    };
    use qa_session::timing::TickRate;
    let protocol = Protocol::Quake2Repro1038;
    let fixture = Fixture::new(protocol, ClientId(0));
    let resources = [
        fixture.resources(ClientId(0)),
        fixture.resources(ClientId(1)),
    ];
    let solid = SolidCodec::QUAKE2_RERELEASE.pack_bounds(Bounds {
        mins: Vec3([-8.; 3]),
        maxs: Vec3([8.; 3]),
    });
    let mut appearance = fixture.record(0., solid).appearance;
    appearance.models[0] = ModelRef::native(ResourceNamespace::Module(ModuleId::default()), 2);
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.geometry = fixture.store;
    runtime.model_assets = Some(fixture.models);
    for (seat, resources) in resources.into_iter().enumerate() {
        let id = runtime
            .connect_local(
                SeatId::ALL[seat],
                SpawnAnchor {
                    position: Vec3([-24., 0., 0.]),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(Some(RuleSetId::Quake2Rerelease), None, None, None, None)
                    .unwrap(),
                protocol,
                seat as u32,
            )
            .unwrap();
        assert_eq!(id, ClientId(seat as u32));
        runtime.client_resources[seat] = Some(resources);
        // Connection metadata has no native source configstring here: there are
        // no loaded modules. Each connection must publish its own bounded row.
        runtime.native_numbers[seat].set_native_clients(
            [3, 2][seat],
            qa_network::snapshots::snapshot_limits(protocol).first_entity,
        );
        let entity = runtime.server.clients[seat].entity;
        runtime.server.entities.columns.appearance[entity.slot as usize] = appearance;
    }
    let monster = runtime
        .server
        .entities
        .allocate(
            ThinkTime::Milliseconds(0),
            ModuleId(7),
            qa_world::entities::AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    runtime.server.entities.columns.set_body(
        monster.slot as usize,
        Body {
            position: Vec3([24., 0., 0.]),
            mins: Vec3([-8.; 3]),
            maxs: Vec3([8.; 3]),
            ..Default::default()
        },
    );
    runtime.server.entities.columns.appearance[monster.slot as usize] = appearance;
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients = [Some(ClientId(0)), Some(ClientId(1)), None, None];
    // The ordinary host publishes the derived config row into EventRing, sends
    // its native packet through loopback, then accepts it through SysEventQueue.
    accept_local_frames(&mut host);
    assert_eq!(host.runtime.network.malformed, 0);
    assert_eq!(host.runtime.server.clients.len(), 2);
    for (seat, max_clients, monster_number) in [(0, 3, 4), (1, 2, 3)] {
        let resources = host.runtime.client_resources[seat].as_ref().unwrap();
        assert_eq!(resources.state.max_clients(), max_clients);
        assert_eq!(
            host.runtime.native_numbers[seat].player_count(),
            max_clients
        );
        assert_eq!(
            host.runtime.native_numbers[seat].number(monster),
            Some(monster_number)
        );
        let view = host.runtime.client_collision[seat].as_mut().unwrap();
        let expected = view.entity(monster_number).unwrap();
        let other_player_number = if seat == 0 { 2 } else { 1 };
        assert_eq!(
            player_mask_trace(view, &host.runtime.geometry, true).entity,
            view.entity(other_player_number),
        );
        assert_eq!(
            player_mask_trace(view, &host.runtime.geometry, false).entity,
            Some(expected),
        );
    }
}

#[test]
fn queued_q2_player_role_reaches_q3_collision_without_a_maxclients_row() {
    use qa_app::{Runtime, client_policy::ClientPolicy, host::FrameHost, map::SpawnAnchor};
    use qa_console::{commands::Console, views::Context};
    use qa_core::{
        loopback::Endpoint,
        primitives::{Body, CollisionTags, ModuleId, NativeEntity},
        sys_events::SeatId,
    };
    use qa_session::{clients::Connection, timing::TickRate};

    let protocol = Protocol::Quake3_68;
    let source_protocol = Protocol::Quake2_34;
    let module = ModuleId(7);
    let fixture = Fixture::new(protocol, ClientId(0));
    let resources = fixture.resources(ClientId(0));
    assert_eq!(resources.state.layout.max_clients(), None);
    let bounds = Bounds {
        mins: Vec3([-8.; 3]),
        maxs: Vec3([8.; 3]),
    };
    let appearance = EntityAppearance {
        source: RuleSetId::Quake2,
        models: [
            ModelRef::native(ResourceNamespace::Module(module), 2),
            None,
            None,
            None,
        ],
        native: qa_core::primitives::NativeAppearanceTail {
            solid: SolidCodec::QUAKE2.pack_bounds(bounds),
            ..Default::default()
        },
        ..Default::default()
    };
    // No hand-authored Q3 eType: the publishing player is a real common client
    // whose appearance and native edict namespace both remain Quake II.
    assert_eq!(appearance.native.entity_type, 0);
    let mut runtime = Runtime::load(2, []).unwrap();
    runtime.geometry = fixture.store;
    runtime.model_assets = Some(fixture.models);
    let receiver = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy::select(
                Some(RuleSetId::Quake3),
                None,
                Some(RuleSetId::Quake2Rerelease),
                Some(RuleSetId::Quake2Rerelease),
                None,
            )
            .unwrap(),
            protocol,
            5,
        )
        .unwrap();
    assert_eq!(receiver, ClientId(0));
    assert!(
        runtime
            .server
            .bind_client_native(receiver, NativeEntity { module, slot: 6 })
    );
    runtime.client_resources[receiver.0 as usize] = Some(resources);
    runtime.native_numbers[receiver.0 as usize].set_native_clients(
        8,
        qa_network::snapshots::snapshot_limits(source_protocol).first_entity,
    );
    let peer = runtime
        .server
        .connect(
            Connection::Bot,
            module,
            Some(NativeEntity { module, slot: 8 }),
        )
        .unwrap();
    assert_eq!(peer, ClientId(1));
    let peer_entity = runtime.server.clients[peer.0 as usize].entity;
    runtime.server.entities.columns.set_body(
        peer_entity.slot as usize,
        Body {
            position: Vec3([-24., 0., 0.]),
            mins: bounds.mins,
            maxs: bounds.maxs,
            ..Default::default()
        },
    );
    runtime.server.entities.columns.appearance[peer_entity.slot as usize] = appearance;
    assert_eq!(
        runtime.server.entities.columns.collision_tags[peer_entity.slot as usize],
        CollisionTags::PLAYER
    );
    let monster = runtime
        .server
        .entities
        .allocate(
            ThinkTime::Milliseconds(0),
            module,
            qa_world::entities::AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[monster.slot as usize] =
        Some(NativeEntity { module, slot: 13 });
    runtime.server.entities.columns.set_body(
        monster.slot as usize,
        Body {
            position: Vec3([24., 0., 0.]),
            mins: bounds.mins,
            maxs: bounds.maxs,
            ..Default::default()
        },
    );
    runtime.server.entities.columns.appearance[monster.slot as usize] = appearance;
    assert_eq!(
        runtime.server.entities.columns.collision_tags[monster.slot as usize],
        CollisionTags::default()
    );
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[SeatId::FIRST.index()] = Some(receiver);
    accept_local_frames(&mut host);
    assert_eq!(host.runtime.network.malformed, 0);
    assert_eq!(
        host.runtime.native_numbers[receiver.0 as usize].number(peer_entity),
        Some(7)
    );
    assert_ne!(peer.0, 7);
    let accepted = host
        .runtime
        .network
        .get(receiver, Endpoint::Client)
        .unwrap()
        .channel
        .current_snapshot()
        .unwrap();
    let ReceivedFrame::Quake3(accepted) = accepted else {
        panic!("Q3 wire frame");
    };
    let peer_row = accepted
        .entities
        .iter()
        .find(|row| row.number == 7)
        .unwrap();
    let monster_row = accepted
        .entities
        .iter()
        .find(|row| row.number == 13)
        .unwrap();
    let roles = qa_gameplay::rules::entities::entity_roles(RuleSetId::Quake3);
    assert!(roles.is_player(peer_row.words[11]));
    assert_eq!(
        peer_row.words[20], 7,
        "Q3 clientNum is destination-native, not common ClientId"
    );
    assert!(!roles.is_player(monster_row.words[11]));
    assert_eq!(
        host.runtime.client_resources[receiver.0 as usize]
            .as_ref()
            .unwrap()
            .state
            .max_clients(),
        0
    );
    let view = host.runtime.client_collision[receiver.0 as usize]
        .as_mut()
        .unwrap();
    let peer_target = view.entity(7).unwrap();
    let monster_target = view.entity(13).unwrap();
    assert_eq!(
        player_mask_trace(view, &host.runtime.geometry, true).entity,
        Some(peer_target)
    );
    assert_eq!(
        player_mask_trace(view, &host.runtime.geometry, false).entity,
        Some(monster_target)
    );
}
