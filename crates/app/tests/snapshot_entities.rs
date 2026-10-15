use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
    snapshots::EntitySource,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{
        Bounds, ClientId, EntityAppearance, EntityId, IndexSpan, ModelRef, ModuleId, NativeEntity,
        Plane, ResourceNamespace, RuleSetId, SoundRef, ThinkTime, Vec3,
    },
    sys_events::{EventKind, EventTime, SeatId, SysEvent, SysEventQueue},
};
use qa_network::{commands::packet::Protocol, projection::EntityRecord, snapshots::ReceivedFrame};
use qa_session::timing::TickRate;
use qa_world::{
    entities::AllocationPolicy,
    leaves::LeafScratch,
    portals::{AreaPortals, Portal},
    visibility::{PvsRows, VisLeaf, VisNode, VisibilityWorld},
};
use std::time::Duration;

const MODULE: ModuleId = ModuleId(7);

#[derive(Default)]
struct Clock(u64);
impl FrameSource for Clock {
    fn begin_frame(&mut self) -> Duration {
        self.0 += 10_000_000;
        Duration::from_nanos(self.0)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime(self.0),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::from_nanos(self.0)
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

fn host(protocol: Protocol, clients: usize) -> Result<(FrameHost, ClientId), String> {
    let mut runtime = Runtime::load(clients, [])?;
    let client = runtime.connect_local(
        SeatId::FIRST,
        qa_app::map::SpawnAnchor {
            position: Vec3([32., 0., 0.]),
            angles: Vec3::default(),
            entity: 0,
            fixture_fallback: false,
        },
        qa_app::client_policy::ClientPolicy::select(
            Some(RuleSetId::QuakeWorld),
            None,
            None,
            None,
            None,
        )?,
        protocol,
        0,
    )?;
    assert!(runtime.server.bind_client_native(
        client,
        NativeEntity {
            module: MODULE,
            slot: if protocol == Protocol::Quake3_68 {
                0
            } else {
                1
            },
        }
    ));
    Ok((
        FrameHost::load(
            Console::new(Context::default()).map_err(|error| error.to_string())?,
            runtime,
            TickRate::FrameDriven,
            vec![],
        )?,
        client,
    ))
}

// These fixtures choose the exact encoded snapshot and optional visibility
// inputs. The shared queued consumer imports it without scheduling another
// SERVER publication; full automatic frames are covered by snapshot_connections.
fn dispatch(host: &mut FrameHost, time: EventTime) -> qa_app::host::FrameResult {
    host.queue
        .push(SysEvent {
            time,
            kind: EventKind::Time,
        })
        .unwrap();
    let mut result = qa_app::host::FrameResult::default();
    host.drain(&mut result);
    result
}

fn add(
    host: &mut FrameHost,
    native: Option<NativeEntity>,
    source: RuleSetId,
    position: Vec3,
) -> EntityId {
    let entity = host
        .runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.), MODULE, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let columns = &mut host.runtime.server.entities.columns;
    let slot = entity.slot as usize;
    columns.native_entity[slot] = native;
    columns.position[slot] = position;
    columns.mins[slot] = Vec3([-1.; 3]);
    columns.maxs[slot] = Vec3([1.; 3]);
    columns.angles[slot] = Vec3([0., 45., 0.]);
    columns.appearance[slot] = EntityAppearance {
        source,
        models: [
            ModelRef::native(ResourceNamespace::Module(MODULE), 5),
            None,
            None,
            None,
        ],
        frame: 17,
        ..Default::default()
    };
    entity
}

fn transmit(
    host: &mut FrameHost,
    client: ClientId,
    time: EventTime,
    delta: Option<u32>,
    visibility: Option<(&VisibilityWorld, &mut LeafScratch, &mut [u8])>,
    portals: Option<&AreaPortals>,
) -> Result<u32, String> {
    let runtime = &mut host.runtime;
    let player = &runtime.server.clients[client.0 as usize];
    let binding = runtime.snapshots[client.0 as usize]
        .as_mut()
        .ok_or("binding")?;
    let channel = &mut runtime
        .network
        .get_mut(client, Endpoint::Server)
        .ok_or("server channel")?
        .channel;
    let sequence = binding.sequence(channel, time);
    let mut bytes = [0; 1400];
    let mut resolve = |model: ModelRef| {
        (model.namespace == ResourceNamespace::Module(MODULE)).then_some(model.ordinal)
    };
    let source = EntitySource {
        receiver: client,
        entities: &runtime.server.entities,
        players: &runtime.server.clients,
        registry: &runtime.catalog.registry,
        native_module: Some(MODULE),
        self_entity: player.entity,
        visibility,
        portals,
        resolve_sound: &mut |sound| {
            (sound == SoundRef::native(ResourceNamespace::Module(ModuleId(8)), 5).unwrap())
                .then_some(6)
        },
        resolve_model: &mut resolve,
    };
    let length = binding
        .encode(
            channel,
            &player.player,
            source,
            &mut runtime.native_numbers[client.0 as usize],
            sequence,
            time,
            delta,
            &mut bytes,
        )
        .map_err(|error| error.to_string())?;
    channel
        .prepare_snapshot(sequence, None, &bytes[..length], time)
        .map_err(|error| error.to_string())?
        .ok_or("prepared snapshot")?;
    assert!(
        channel
            .submit_with(time, |packet| runtime
                .loopback
                .send(Endpoint::Server, client, packet)
                .is_ok())
            .map_err(|error| error.to_string())?
            .is_some()
    );
    while channel.pending_fragments() {
        channel
            .prepare_output(time)
            .map_err(|error| error.to_string())?
            .ok_or("prepared fragment")?;
        assert!(
            channel
                .submit_with(time, |packet| runtime
                    .loopback
                    .send(Endpoint::Server, client, packet)
                    .is_ok())
                .map_err(|error| error.to_string())?
                .is_some()
        );
    }
    binding.submitted(sequence);
    Ok(sequence)
}

fn records(host: &FrameHost, client: ClientId) -> Result<Vec<(u32, EntityRecord)>, String> {
    let binding = host.runtime.snapshots[client.0 as usize]
        .as_ref()
        .ok_or("binding")?;
    let frame = host
        .runtime
        .network
        .get(client, Endpoint::Client)
        .ok_or("CLIENT channel")?
        .channel
        .current_snapshot()
        .ok_or("CLIENT frame")?;
    let mut rows = Vec::new();
    macro_rules! decode {
        ($frame:expr) => {
            for entity in $frame.entities {
                let mut record = EntityRecord::default();
                assert!(binding.entity_projection().apply(
                    &entity.words,
                    &mut record,
                    ResourceNamespace::Client(client)
                ));
                rows.push((entity.number, record));
            }
        };
    }
    match frame {
        ReceivedFrame::NetQuake(frame) => decode!(frame),
        ReceivedFrame::QuakeWorld(frame) => decode!(frame),
        ReceivedFrame::Quake2(frame) => decode!(frame),
        ReceivedFrame::Quake2Repro(frame) => decode!(frame),
        ReceivedFrame::Quake3(frame) => decode!(frame),
    }
    Ok(rows)
}

#[test]
fn primary_players_cross_native_world_slot_conventions_without_losing_peers() -> Result<(), String>
{
    for (protocol, source, source_first) in [
        (Protocol::Quake2_34, RuleSetId::Quake3, 0),
        (Protocol::Quake3_68, RuleSetId::Quake2, 1),
    ] {
        let (mut host, client) = host(protocol, 2)?;
        assert!(host.runtime.server.bind_client_native(
            client,
            NativeEntity {
                module: MODULE,
                slot: source_first
            }
        ));
        host.runtime.native_numbers[client.0 as usize].set_native_clients(2, source_first as u32);
        let peer = host
            .runtime
            .server
            .connect(
                qa_session::clients::Connection::Remote,
                MODULE,
                Some(NativeEntity {
                    module: MODULE,
                    slot: source_first + 1,
                }),
            )
            .ok_or("peer")?;
        let entity = host.runtime.server.clients[peer.0 as usize].entity;
        let columns = &mut host.runtime.server.entities.columns;
        columns.position[entity.slot as usize] = Vec3([48., 0., 0.]);
        columns.mins[entity.slot as usize] = Vec3([-8.; 3]);
        columns.maxs[entity.slot as usize] = Vec3([8.; 3]);
        columns.appearance[entity.slot as usize] = EntityAppearance {
            source,
            models: [
                ModelRef::native(ResourceNamespace::Module(MODULE), 5),
                None,
                None,
                None,
            ],
            ..Default::default()
        };
        let ordinary = add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: source_first + 2,
            }),
            source,
            Vec3([100., 0., 0.]),
        );
        transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            None,
            None,
        )?;
        dispatch(&mut host, EventTime(1_000_000_000));
        let rows = records(&host, client)?;
        let first = qa_network::snapshots::snapshot_limits(protocol).first_entity;
        assert!(
            rows.iter()
                .any(|(number, record)| *number == first + 1 && record.position.0[0] == 48.)
        );
        let mapped = host.runtime.native_numbers[client.0 as usize]
            .number(ordinary)
            .ok_or("ordinary projection")?;
        assert!(mapped >= first + 2);
        assert!(
            rows.iter()
                .any(|(number, record)| *number == mapped && record.position.0[0] == 100.)
        );
        assert_eq!(host.runtime.network.command_errors, 0);
    }
    Ok(())
}

#[test]
fn netquake_and_quakeworld_project_entities_through_the_same_source_path() -> Result<(), String> {
    for protocol in [Protocol::NetQuake15, Protocol::QuakeWorld28] {
        let (mut host, client) = host(protocol, 1)?;
        add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: 42,
            }),
            protocol.rule_set(),
            Vec3([12.375, -24.625, 48.125]),
        );
        if protocol == Protocol::QuakeWorld28 {
            assert!(
                host.runtime
                    .send_local_command(client, &Default::default(), EventTime(0))
            );
            dispatch(&mut host, EventTime(1_000_000_000));
        }
        transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            None,
            None,
        )?;
        assert!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_none()
        );
        dispatch(&mut host, EventTime(1_000_000_000));
        let rows = records(&host, client)?;
        let record = rows.iter().find(|(number, _)| *number == 42).unwrap().1;
        assert_eq!(record.position, Vec3([12.375, -24.625, 48.125]));
        assert_eq!(record.appearance.frame, 17);
        assert_eq!(
            record.appearance.models[0],
            ModelRef::native(ResourceNamespace::Client(client), 5)
        );
        assert_eq!(host.runtime.network.command_errors, 0);
    }
    Ok(())
}

#[test]
fn sound_only_entity_projects_its_explicit_foreign_resource_namespace() -> Result<(), String> {
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol, 1)?;
        let entity = add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: 42,
            }),
            RuleSetId::Quake2,
            Vec3([64., 0., 0.]),
        );
        let appearance = &mut host.runtime.server.entities.columns.appearance[entity.slot as usize];
        appearance.models = [None; 4];
        appearance.loop_sound = SoundRef::native(ResourceNamespace::Module(ModuleId(8)), 5);
        // Player and entity resources use the same resolver even when their
        // resource owner differs from the native entity namespace.
        host.runtime.server.clients[client.0 as usize]
            .player
            .loop_sound = SoundRef::native(ResourceNamespace::Module(ModuleId(8)), 5);
        transmit(
            &mut host,
            client,
            EventTime::from_milliseconds(1000),
            None,
            None,
            None,
        )?;
        dispatch(&mut host, EventTime::from_milliseconds(1000));
        if protocol == Protocol::Quake3_68 {
            assert_eq!(
                host.runtime.prediction[0].player.loop_sound,
                SoundRef::native(ResourceNamespace::Client(client), 6)
            );
        }
        let rows = records(&host, client)?;
        let record = rows
            .iter()
            .find(|(number, _)| *number == 42)
            .ok_or("sound-only row")?
            .1;
        assert_eq!(
            record.appearance.loop_sound,
            SoundRef::native(ResourceNamespace::Client(client), 6)
        );
        assert_eq!(record.appearance.models, [None; 4]);
        assert_eq!(record.position, Vec3([64., 0., 0.]));
        assert_eq!(
            host.runtime.server.entities.columns.appearance[entity.slot as usize].loop_sound,
            SoundRef::native(ResourceNamespace::Module(ModuleId(8)), 5)
        );
    }
    Ok(())
}

#[test]
fn q3_event_only_stop_survives_the_server_presence_filter_and_queued_snapshot() -> Result<(), String>
{
    for target in [0, 12] {
        let (mut host, client) = host(Protocol::Quake3_68, 1)?;
        if target != 0 {
            add(
                &mut host,
                Some(NativeEntity {
                    module: MODULE,
                    slot: target,
                }),
                RuleSetId::Quake3,
                Vec3([96., 0., 0.]),
            );
        }
        let entity = add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: 42,
            }),
            RuleSetId::Quake3,
            Vec3([64., 0., 0.]),
        );
        let appearance = &mut host.runtime.server.entities.columns.appearance[entity.slot as usize];
        appearance.models = [None; 4];
        appearance.native.entity_type = 13 + 75;
        appearance.native.other_entity = Some(qa_core::primitives::NativeEntityRef::entity(
            ResourceNamespace::Module(MODULE),
            target as u32,
        ));
        appearance.native.render_flags = 0x10;
        assert_eq!(appearance.native.event, 0);
        assert_eq!(appearance.loop_sound, None);
        transmit(
            &mut host,
            client,
            EventTime::from_milliseconds(1000),
            None,
            None,
            None,
        )?;
        dispatch(&mut host, EventTime::from_milliseconds(1000));
        let rows = records(&host, client)?;
        let appearance = rows
            .iter()
            .find(|(number, _)| *number == 42)
            .ok_or("event-only row")?
            .1
            .appearance;
        assert_eq!(appearance.native.entity_type, 88);
        assert_eq!(
            appearance.native.other_entity,
            Some(qa_core::primitives::NativeEntityRef::entity(
                ResourceNamespace::Client(client),
                target as u32
            ))
        );
        assert_eq!(appearance.native.render_flags, 0x10);
        assert_eq!(appearance.native.event, 0);
        assert_eq!(appearance.loop_sound, None);
    }
    Ok(())
}

#[test]
fn q3_secondary_module_stop_target_uses_the_connection_entity_namespace() -> Result<(), String> {
    let (mut host, client) = host(Protocol::Quake3_68, 1)?;
    let secondary = ModuleId(8);
    // Put the event before its target in common slot order. The two modules
    // both own native entity twelve, but only the primary keeps wire twelve.
    let event = add(
        &mut host,
        Some(NativeEntity {
            module: secondary,
            slot: 42,
        }),
        RuleSetId::Quake3,
        Vec3([64., 0., 0.]),
    );
    let appearance = &mut host.runtime.server.entities.columns.appearance[event.slot as usize];
    appearance.models = [None; 4];
    appearance.native.entity_type = 13 + 75;
    appearance.native.other_entity = Some(qa_core::primitives::NativeEntityRef::entity(
        ResourceNamespace::Module(ModuleId(8)),
        12,
    ));
    appearance.native.render_flags = 0x10;
    add(
        &mut host,
        Some(NativeEntity {
            module: MODULE,
            slot: 12,
        }),
        RuleSetId::Quake3,
        Vec3([96., 0., 0.]),
    );
    add(
        &mut host,
        Some(NativeEntity {
            module: secondary,
            slot: 12,
        }),
        RuleSetId::Quake3,
        Vec3([128., 0., 0.]),
    );
    transmit(
        &mut host,
        client,
        EventTime::from_milliseconds(1000),
        None,
        None,
        None,
    )?;
    dispatch(&mut host, EventTime::from_milliseconds(1000));
    let rows = records(&host, client)?;
    assert_eq!(
        rows.iter()
            .find(|(number, _)| *number == 12)
            .ok_or("primary target")?
            .1
            .position,
        Vec3([96., 0., 0.])
    );
    let target = rows
        .iter()
        .find(|(_, record)| record.position == Vec3([128., 0., 0.]))
        .ok_or("secondary target")?
        .0;
    assert_ne!(target, 12);
    let record = rows
        .iter()
        .find(|(_, record)| record.appearance.native.entity_type == 88)
        .ok_or("secondary stop event")?
        .1;
    assert_eq!(
        record.appearance.native.other_entity,
        Some(qa_core::primitives::NativeEntityRef::entity(
            ResourceNamespace::Client(client),
            target
        ))
    );
    assert_eq!(
        host.runtime.server.entities.columns.appearance[event.slot as usize]
            .native
            .other_entity,
        Some(qa_core::primitives::NativeEntityRef::entity(
            ResourceNamespace::Module(secondary),
            12
        ))
    );
    Ok(())
}

#[test]
fn q3_stop_with_an_unresolved_native_target_does_not_invent_client_zero() -> Result<(), String> {
    let (mut host, client) = host(Protocol::Quake3_68, 1)?;
    let event = add(
        &mut host,
        Some(NativeEntity {
            module: MODULE,
            slot: 42,
        }),
        RuleSetId::Quake3,
        Vec3([64., 0., 0.]),
    );
    let appearance = &mut host.runtime.server.entities.columns.appearance[event.slot as usize];
    appearance.models = [None; 4];
    appearance.native.entity_type = 13 + 75;
    appearance.native.other_entity = Some(qa_core::primitives::NativeEntityRef::entity(
        ResourceNamespace::Module(ModuleId(8)),
        12,
    ));
    appearance.native.render_flags = 0x10;
    transmit(
        &mut host,
        client,
        EventTime::from_milliseconds(1000),
        None,
        None,
        None,
    )?;
    dispatch(&mut host, EventTime::from_milliseconds(1000));
    let rows = records(&host, client)?;
    // Original Q3 omits the viewed player row; CLIENT retains it through PS.
    assert!(rows.iter().all(|(number, _)| *number != 0));
    assert_eq!(
        host.runtime.prediction[0]
            .player
            .native
            .and_then(|native| native.client_number),
        Some(0)
    );
    assert_eq!(host.runtime.prediction[0].player.loop_sound, None);
    assert!(rows.iter().all(|(number, _)| *number != 42));
    Ok(())
}

#[test]
fn entity_updates_and_removals_reach_client_only_through_queued_native_snapshots()
-> Result<(), String> {
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol, 1)?;
        let entity = add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: 42,
            }),
            protocol.rule_set(),
            Vec3([12.375, -24.625, 48.125]),
        );
        let first = transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            None,
            None,
        )?;
        assert!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .is_none()
        );
        let result = dispatch(&mut host, EventTime(1_000_000_000));
        assert_eq!(result.drains, 1);
        let initial = records(&host, client)?;
        let record = initial.iter().find(|(number, _)| *number == 42).unwrap().1;
        assert_eq!(record.position, Vec3([12.375, -24.625, 48.125]));
        assert_eq!(record.appearance.frame, 17);
        assert_eq!(
            record.appearance.models[0],
            ModelRef::native(ResourceNamespace::Client(client), 5)
        );
        host.runtime.server.entities.columns.position[entity.slot as usize] = Vec3([21., 22., 23.]);
        host.runtime.server.entities.columns.appearance[entity.slot as usize].frame = 18;
        let second = transmit(
            &mut host,
            client,
            EventTime(1_100_000_000),
            Some(first),
            None,
            None,
        )?;
        assert_eq!(records(&host, client)?, initial);
        dispatch(&mut host, EventTime(1_100_000_000));
        let updated = records(&host, client)?;
        let record = updated.iter().find(|(number, _)| *number == 42).unwrap().1;
        assert_eq!(
            (record.position, record.appearance.frame),
            (Vec3([21., 22., 23.]), 18)
        );
        assert!(
            host.runtime
                .server
                .entities
                .release(entity, ThinkTime::Seconds(0.))
        );
        transmit(
            &mut host,
            client,
            EventTime(1_200_000_000),
            Some(second),
            None,
            None,
        )?;
        dispatch(&mut host, EventTime(1_200_000_000));
        assert!(
            records(&host, client)?
                .iter()
                .all(|(number, _)| *number != 42)
        );
        let channel = &host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel;
        assert_eq!(
            channel.previous_snapshot().map(|frame| frame.sequence()),
            Some(second)
        );
        assert_eq!(host.runtime.network.command_errors, 0);
    }
    Ok(())
}

#[test]
fn quakeworld_native_players_use_playerinfo_and_accepted_client_entity_rows() -> Result<(), String>
{
    let (mut host, client) = host(Protocol::QuakeWorld28, 2)?;
    let peer = host
        .runtime
        .server
        .connect(
            qa_session::clients::Connection::Remote,
            MODULE,
            Some(NativeEntity {
                module: MODULE,
                slot: 2,
            }),
        )
        .ok_or("peer")?;
    host.runtime.native_numbers[0].set_native_clients(2, 1);
    let remote = &mut host.runtime.server.clients[peer.0 as usize];
    remote.player.body.position = Vec3([48., 12., 24.]);
    remote.player.body.velocity = Vec3([16., 0., 0.]);
    remote.command.view_angles = Vec3([10., 45., 0.]);
    let slot = remote.entity.slot as usize;
    host.runtime
        .server
        .entities
        .columns
        .set_body(slot, remote.player.body);
    host.runtime.server.entities.columns.angles[slot] = Vec3([0., 45., 0.]);
    host.runtime.server.entities.columns.appearance[slot] = EntityAppearance {
        source: RuleSetId::QuakeWorld,
        frame: 17,
        skin: 7,
        models: [
            ModelRef::native(ResourceNamespace::Module(MODULE), 5),
            None,
            None,
            None,
        ],
        native: qa_core::primitives::NativeAppearanceTail {
            effects: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        host.runtime
            .send_local_command(client, &Default::default(), EventTime(0))
    );
    dispatch(&mut host, EventTime(1_000_000_000));
    let first = transmit(
        &mut host,
        client,
        EventTime(1_000_000_000),
        None,
        None,
        None,
    )?;
    dispatch(&mut host, EventTime(1_000_000_000));
    let rows = records(&host, client)?;
    let row = rows.iter().find(|(number, _)| *number == 2).unwrap().1;
    assert_eq!(row.position, Vec3([48., 12., 24.]));
    assert_eq!(
        (
            row.appearance.frame,
            row.appearance.skin,
            row.appearance.native.effects
        ),
        (17, 7, 2)
    );
    assert_eq!(
        row.appearance.models[0],
        ModelRef::native(ResourceNamespace::Client(client), 5)
    );
    assert_eq!(host.runtime.network.command_errors, 0);
    host.runtime.server.clients[peer.0 as usize].connection = None;
    // QW deltas answer a submitted client request; the advisory byte in the
    // server message is not an independent choice of a CLIENT delta base.
    assert!(
        host.runtime
            .send_local_command(client, &Default::default(), EventTime(1_100_000_000))
    );
    dispatch(&mut host, EventTime(1_100_000_000));
    transmit(
        &mut host,
        client,
        EventTime(1_100_000_000),
        Some(first),
        None,
        None,
    )?;
    dispatch(&mut host, EventTime(1_100_000_000));
    assert!(
        records(&host, client)?
            .iter()
            .all(|(number, _)| *number != 2)
    );
    Ok(())
}

#[test]
fn snapshot_exclusion_is_a_shared_capability_independent_of_source_and_wire_flags()
-> Result<(), String> {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol, 1)?;
        for (index, source) in [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
            RuleSetId::Quake3,
        ]
        .into_iter()
        .enumerate()
        {
            let visible = add(
                &mut host,
                Some(NativeEntity {
                    module: MODULE,
                    slot: 42 + index as i32 * 2,
                }),
                source,
                Vec3([20.0 + index as f32, 0.0, 0.0]),
            );
            // Raw native flag words have already been converted at the module
            // boundary; a bit in that ABI column does not select visibility.
            host.runtime.server.entities.columns.flags[visible.slot as usize] = 1;
            let excluded = add(
                &mut host,
                Some(NativeEntity {
                    module: MODULE,
                    slot: 43 + index as i32 * 2,
                }),
                source,
                Vec3([40.0 + index as f32, 0.0, 0.0]),
            );
            host.runtime.server.entities.columns.appearance[excluded.slot as usize]
                .snapshot_excluded = true;
        }
        if protocol == Protocol::QuakeWorld28 {
            assert!(
                host.runtime
                    .send_local_command(client, &Default::default(), EventTime(0))
            );
            dispatch(&mut host, EventTime(1_000_000_000));
        }
        transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            None,
            None,
        )?;
        dispatch(&mut host, EventTime(1_000_000_000));
        let rows = records(&host, client)?;
        for index in 0..5 {
            assert!(
                rows.iter().any(|(number, _)| *number == 42 + index * 2),
                "{protocol:?}"
            );
            assert!(
                rows.iter().all(|(number, _)| *number != 43 + index * 2),
                "{protocol:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn native_namespace_wins_over_foreign_handles_and_generation_reuse() -> Result<(), String> {
    let (mut host, client) = host(Protocol::Quake2_34, 1)?;
    let foreign = add(
        &mut host,
        Some(NativeEntity {
            module: ModuleId(8),
            slot: 2,
        }),
        RuleSetId::Quake2,
        Vec3([20., 0., 0.]),
    );
    let native = add(
        &mut host,
        Some(NativeEntity {
            module: MODULE,
            slot: 2,
        }),
        RuleSetId::Quake2,
        Vec3([40., 0., 0.]),
    );
    let hidden = add(
        &mut host,
        Some(NativeEntity {
            module: MODULE,
            slot: 99,
        }),
        RuleSetId::Quake2,
        Vec3([60., 0., 0.]),
    );
    host.runtime.server.entities.columns.flags[hidden.slot as usize] = 1;
    host.runtime.server.entities.columns.appearance[hidden.slot as usize].snapshot_excluded = true;
    let first = transmit(
        &mut host,
        client,
        EventTime(1_000_000_000),
        None,
        None,
        None,
    )?;
    dispatch(&mut host, EventTime(1_000_000_000));
    let rows = records(&host, client)?;
    assert_eq!(
        rows.iter()
            .find(|(number, _)| *number == 2)
            .unwrap()
            .1
            .position
            .0[0],
        40.
    );
    let foreign_number = rows
        .iter()
        .find(|(_, row)| row.position.0[0] == 20.)
        .unwrap()
        .0;
    assert_ne!(foreign_number, 2);
    assert!(rows.iter().all(|(number, _)| *number != 99));
    assert_eq!(
        host.runtime.snapshots[0].as_ref().unwrap().view_entity(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
        ),
        Some(1)
    );
    assert!(
        host.runtime
            .server
            .entities
            .release(foreign, ThinkTime::Seconds(0.))
    );
    let replacement = add(
        &mut host,
        Some(NativeEntity {
            module: MODULE,
            slot: 88,
        }),
        RuleSetId::Quake2,
        Vec3([80., 0., 0.]),
    );
    assert_eq!(replacement.slot, foreign.slot);
    assert_ne!(replacement.generation, foreign.generation);
    transmit(
        &mut host,
        client,
        EventTime(1_100_000_000),
        Some(first),
        None,
        None,
    )?;
    dispatch(&mut host, EventTime(1_100_000_000));
    let rows = records(&host, client)?;
    assert!(rows.iter().all(|(number, _)| *number != foreign_number));
    assert_eq!(
        rows.iter()
            .find(|(number, _)| *number == 88)
            .unwrap()
            .1
            .position
            .0[0],
        80.
    );
    assert_eq!(
        rows.iter()
            .find(|(number, _)| *number == 2)
            .unwrap()
            .1
            .position
            .0[0],
        host.runtime.server.entities.columns.position[native.slot as usize].0[0]
    );
    Ok(())
}

#[test]
fn oversized_worlds_select_a_native_packet_without_publishing_failed_trials() -> Result<(), String>
{
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol, 1)?;
        let mut entities = Vec::new();
        for index in 0..200 {
            let entity = add(
                &mut host,
                Some(NativeEntity {
                    module: MODULE,
                    slot: index + 42,
                }),
                protocol.rule_set(),
                Vec3([
                    1024. + index as f32,
                    -512. - index as f32,
                    768. + index as f32,
                ]),
            );
            let appearance =
                &mut host.runtime.server.entities.columns.appearance[entity.slot as usize];
            appearance
                .models
                .fill(ModelRef::native(ResourceNamespace::Module(MODULE), 5));
            appearance.frame = 127;
            appearance.skin = 127;
            appearance.native.effects = 255;
            appearance.native.colormap = 7;
            host.runtime.server.entities.columns.angles[entity.slot as usize] =
                Vec3([22.5, 45., 67.5]);
            entities.push(entity);
        }
        if protocol == Protocol::QuakeWorld28 {
            assert!(
                host.runtime
                    .send_local_command(client, &Default::default(), EventTime(0))
            );
            dispatch(&mut host, EventTime(1_000_000_000));
        }
        let first = transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            None,
            None,
        )?;
        let server = host
            .runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        assert_eq!(server.sequence(), first);
        assert!(
            host.runtime
                .network
                .get(client, Endpoint::Server)
                .unwrap()
                .channel
                .previous_snapshot()
                .is_none()
        );
        dispatch(&mut host, EventTime(1_000_000_000));
        let before = records(&host, client).map_err(|error| format!("{protocol:?}: first {error}; packets={} delivered={} malformed={} command_errors={}",
            host.runtime.network.packets, host.runtime.network.delivered,
            host.runtime.network.malformed, host.runtime.network.command_errors))?;
        let visible = before.iter().filter(|(number, _)| *number >= 42).count();
        assert!(
            visible > 0 && visible < entities.len(),
            "{protocol:?}: {visible}"
        );
        assert!(
            host.runtime.snapshots[0]
                .as_ref()
                .unwrap()
                .entity_rejections()
                > 0
        );
        for entity in &entities {
            let position = &mut host.runtime.server.entities.columns.position[entity.slot as usize];
            *position = *position + Vec3([64., 32., 16.]);
        }
        if protocol == Protocol::QuakeWorld28 {
            assert!(host.runtime.send_local_command(
                client,
                &Default::default(),
                EventTime(1_100_000_000)
            ));
            dispatch(&mut host, EventTime(1_100_000_000));
        }
        let second = transmit(
            &mut host,
            client,
            EventTime(1_100_000_000),
            Some(first),
            None,
            None,
        )?;
        assert_eq!(records(&host, client)?, before);
        let channel = &host
            .runtime
            .network
            .get(client, Endpoint::Server)
            .unwrap()
            .channel;
        assert_eq!(
            channel.previous_snapshot().map(|frame| frame.sequence()),
            Some(first)
        );
        dispatch(&mut host, EventTime(1_100_000_000));
        let after = records(&host, client).map_err(|error| format!("{protocol:?}: second {error}; packets={} delivered={} malformed={} command_errors={}",
            host.runtime.network.packets, host.runtime.network.delivered,
            host.runtime.network.malformed, host.runtime.network.command_errors))?;
        let row = after.iter().find(|(number, _)| *number == 42).unwrap().1;
        assert_eq!(row.position, Vec3([1088., -480., 784.]));
        assert_eq!(
            host.runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .current_snapshot()
                .map(|frame| frame.sequence()),
            Some(second)
        );
        assert_eq!(host.runtime.network.command_errors, 0);
    }
    Ok(())
}

fn visibility() -> VisibilityWorld {
    let whole = Bounds {
        mins: Vec3([-1024.; 3]),
        maxs: Vec3([1024.; 3]),
    };
    VisibilityWorld::load(
        vec![Plane::oriented(Vec3::axis(0, 1.), 0.)],
        vec![VisNode {
            plane: 0,
            children: [-1, -2],
            bounds: whole,
            surfaces: IndexSpan::default(),
        }],
        vec![
            VisLeaf {
                selector: Some(0),
                area: Some(1),
                solid: false,
                bounds: whole,
                surfaces: IndexSpan::default(),
            },
            VisLeaf {
                selector: Some(1),
                area: Some(2),
                solid: false,
                bounds: whole,
                surfaces: IndexSpan::default(),
            },
        ],
        vec![],
        0,
        0,
        PvsRows::load(vec![Some(0), Some(1)], vec![1, 2], None).unwrap(),
    )
    .unwrap()
}

#[test]
fn snapshot_viewer_on_a_plane_uses_native_clip_ties_independently_of_movement() -> Result<(), String>
{
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol, 1)?;
        let player = &mut host.runtime.server.clients[client.0 as usize].player;
        assert_eq!(player.movement_rules, RuleSetId::QuakeWorld);
        player.body.position = Vec3::default();
        player.view_offset = Vec3::default();
        let world = visibility();
        let mut scratch = LeafScratch::new(world.node_count() + 1);
        let mut row = vec![0; world.pvs().row_bytes()];
        host.runtime.snapshots[client.0 as usize]
            .as_mut()
            .unwrap()
            .load_visibility(&world);
        for (number, x) in [(42, 32.0), (43, -32.0)] {
            add(
                &mut host,
                Some(NativeEntity {
                    module: MODULE,
                    slot: number,
                }),
                protocol.rule_set(),
                Vec3([x, 0.0, 0.0]),
            );
        }
        let portals = AreaPortals::load(
            3,
            1,
            vec![Portal {
                number: 0,
                first: 1,
                second: 2,
            }],
        )
        .unwrap();
        transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            Some((&world, &mut scratch, &mut row)),
            Some(&portals),
        )?;
        dispatch(&mut host, EventTime(1_000_000_000));
        let rows = records(&host, client)?;
        assert!(
            rows.iter().any(|(number, _)| *number == 42),
            "{protocol:?}: front entity"
        );
        assert!(
            rows.iter().all(|(number, _)| *number != 43),
            "{protocol:?}: back entity"
        );
        let frame = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        let areas = match frame {
            ReceivedFrame::Quake2(frame) => frame.areas,
            ReceivedFrame::Quake2Repro(frame) => frame.areas,
            ReceivedFrame::Quake3(frame) => frame.areas,
            _ => unreachable!(),
        };
        assert_eq!(!areas[0] & 6, 2, "{protocol:?}: viewer front area");
    }
    Ok(())
}

#[test]
fn bounding_box_pvs_and_area_bits_keep_the_protocol_visibility_contract() -> Result<(), String> {
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (mut host, client) = host(protocol, 1)?;
        let world = visibility();
        let mut scratch = LeafScratch::new(world.node_count() + 1);
        let mut row = vec![0; world.pvs().row_bytes()];
        host.runtime.snapshots[0]
            .as_mut()
            .unwrap()
            .load_visibility(&world);
        let spanning = add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: 42,
            }),
            protocol.rule_set(),
            Vec3([-10., 0., 0.]),
        );
        host.runtime.server.entities.columns.maxs[spanning.slot as usize].0[0] = 20.;
        add(
            &mut host,
            Some(NativeEntity {
                module: MODULE,
                slot: 43,
            }),
            protocol.rule_set(),
            Vec3([-30., 0., 0.]),
        );
        let mut portals = AreaPortals::load(
            3,
            1,
            vec![Portal {
                number: 0,
                first: 1,
                second: 2,
            }],
        )
        .unwrap();
        let first = transmit(
            &mut host,
            client,
            EventTime(1_000_000_000),
            None,
            Some((&world, &mut scratch, &mut row)),
            Some(&portals),
        )?;
        dispatch(&mut host, EventTime(1_000_000_000));
        let rows = records(&host, client)?;
        assert!(rows.iter().any(|(number, _)| *number == 42));
        assert!(rows.iter().all(|(number, _)| *number != 43));
        let frame = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        let bits = match frame {
            ReceivedFrame::Quake2(f) => f.areas,
            ReceivedFrame::Quake2Repro(f) => f.areas,
            ReceivedFrame::Quake3(f) => f.areas,
            _ => unreachable!(),
        };
        let visible = !bits[0];
        assert_ne!(visible & (1 << 1), 0);
        assert_eq!(visible & (1 << 2), 0);
        portals.set(0, true).unwrap();
        transmit(
            &mut host,
            client,
            EventTime(1_100_000_000),
            Some(first),
            Some((&world, &mut scratch, &mut row)),
            Some(&portals),
        )?;
        dispatch(&mut host, EventTime(1_100_000_000));
        let frame = host
            .runtime
            .network
            .get(client, Endpoint::Client)
            .unwrap()
            .channel
            .current_snapshot()
            .unwrap();
        let bits = match frame {
            ReceivedFrame::Quake2(f) => f.areas,
            ReceivedFrame::Quake2Repro(f) => f.areas,
            ReceivedFrame::Quake3(f) => f.areas,
            _ => unreachable!(),
        };
        let visible = !bits[0];
        assert_ne!(visible & (1 << 2), 0);
        // Opening the portal does not make the hidden leaf's PVS bit visible.
        assert!(
            records(&host, client)?
                .iter()
                .all(|(number, _)| *number != 43)
        );
    }
    Ok(())
}

#[test]
fn host_respatializes_once_per_client_frame_from_accepted_rows_without_a_catalog()
-> Result<(), String> {
    struct AudioClock {
        clock: Clock,
        mixer: qa_audio::Mixer,
        mixes: u32,
        pcm: [i16; 2],
    }
    impl FrameSource for AudioClock {
        fn begin_frame(&mut self) -> Duration {
            self.clock.begin_frame()
        }
        fn poll_events(&mut self, queue: &mut SysEventQueue) {
            self.clock.poll_events(queue);
        }
        fn wait_time(&mut self, remaining: Duration) -> Duration {
            self.clock.wait_time(remaining)
        }
        fn elapsed(&self) -> Duration {
            self.clock.elapsed()
        }
        fn present(&mut self) {}
        fn mix_audio(
            &mut self,
            entities: &[Option<qa_app::client_scene::ClientEntities<'_>>; SeatId::COUNT],
        ) {
            self.mixes += 1;
            qa_app::audio::respatialize(&mut self.mixer, entities);
            assert!(self.mixer.paint(&mut self.pcm));
        }
    }
    let (mut host, client) = host(Protocol::Quake2_34, 1)?;
    host.local_clients[0] = Some(client);
    assert!(host.runtime.models.is_none());
    let entity = add(
        &mut host,
        Some(NativeEntity {
            module: MODULE,
            slot: 42,
        }),
        RuleSetId::Quake2,
        Vec3([100.0, 0.0, 0.0]),
    );
    let rate = std::num::NonZeroU32::new(44100).unwrap();
    let bank = qa_audio::Bank::load(
        rate,
        vec![(
            "sound/test.wav".into(),
            qa_core::primitives::Pcm {
                rate,
                channels: qa_core::primitives::PcmChannels::Mono,
                samples: vec![1024; 8],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )?;
    let sound = bank.find("sound/test.wav").unwrap();
    let mut source = AudioClock {
        clock: Clock::default(),
        mixer: qa_audio::Mixer::load(std::sync::Arc::new(bank), 1, 1),
        mixes: 0,
        pcm: [0; 2],
    };
    source.mixer.listen(
        &[Some(qa_audio::Listener {
            negate_dot: false,
            client,
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1.0, 0.0, 0.0]),
        })],
        1.0,
    );
    let first = transmit(
        &mut host,
        client,
        EventTime(1_000_000_000),
        None,
        None,
        None,
    )?;
    // Keep the native receive/output binding valid. The fixture controls
    // snapshot publication, so mark this queued-time SERVER frame as sent.
    host.runtime.snapshots[client.0 as usize]
        .as_mut()
        .unwrap()
        .submitted(0);
    host.frame(&mut source, true);
    assert_eq!(source.mixes, 1);
    let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
    assert_eq!(binding.applied, 1);
    assert_eq!(
        binding.interpolation().unwrap().time,
        ThinkTime::Milliseconds(900)
    );
    assert!(source.mixer.sound(qa_core::primitives::SoundEvent {
        sound,
        spatial: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
        client: Some(client),
        entity: Some(qa_core::primitives::SoundEmitter::Native {
            namespace: ResourceNamespace::Client(client),
            slot: 42,
        }),
        position: Vec3([100.0, 0.0, 0.0]),
        volume: 1.0,
        attenuation: 1.0,
        ..Default::default()
    }));
    host.runtime.server.entities.columns.position[entity.slot as usize] = Vec3([-100.0, 0.0, 0.0]);
    transmit(
        &mut host,
        client,
        EventTime(1_100_000_000),
        Some(first),
        None,
        None,
    )?;
    // Native Q2 begins one 100ms interval behind the first snapshot. Advance
    // 125ms of queued time from 900ms to the new interval's 25% point, 1025ms.
    source.clock.0 = 125_000_000;
    host.runtime.snapshots[client.0 as usize]
        .as_mut()
        .unwrap()
        .submitted(1);
    host.frame(&mut source, true);
    assert_eq!(source.mixes, 2);
    let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
    assert_eq!(binding.applied, 2);
    assert_eq!(
        binding.interpolation().unwrap().time,
        ThinkTime::Milliseconds(1025)
    );
    assert_eq!(binding.interpolation().unwrap().fraction, 0.25);
    assert_eq!(
        host.client_entities_inputs()[0]
            .unwrap()
            .get(42)
            .unwrap()
            .origin,
        Vec3([50.0, 0.0, 0.0])
    );
    assert_eq!(source.pcm, [0, 1020]);
    assert_eq!(source.mixer.counts.respatialized, 1);
    // Mutating SERVER after packet acceptance cannot move the active voice.
    host.runtime.server.entities.columns.position[entity.slot as usize] = Vec3([10000.0, 0.0, 0.0]);
    source.clock.0 = 175_000_000;
    host.frame(&mut source, true);
    assert_eq!(source.mixes, 3);
    let binding = host.runtime.snapshots[client.0 as usize].as_ref().unwrap();
    assert_eq!(binding.applied, 2);
    assert_eq!(
        binding.interpolation().unwrap().time,
        ThinkTime::Milliseconds(1075)
    );
    assert_eq!(binding.interpolation().unwrap().fraction, 0.75);
    assert_eq!(
        host.client_entities_inputs()[0]
            .unwrap()
            .get(42)
            .unwrap()
            .origin,
        Vec3([-50.0, 0.0, 0.0])
    );
    assert_eq!(source.pcm, [1020, 0]);
    assert_eq!(source.mixer.counts.started, 1);
    assert_eq!(source.mixer.counts.respatialized, 2);
    assert_eq!(source.mixer.counts.missing_origins, 0);
    Ok(())
}
