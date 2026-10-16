use super::*;
use crate::{Runtime, client_policy::ClientPolicy, map::SpawnAnchor};
use qa_compat::{
    abi::{Addresses, ArgumentSelection, ImportStep, Invocation, Q3_CLIENT, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ClientConnection, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    loopback::Endpoint,
    primitives::{ThinkTime, Vec3},
    sys_events::{EventTime, Peer, SeatId},
};
use qa_network::ingress::{ConnectionPhase, Incoming};
use std::ops::ControlFlow;

fn connect(runtime: &mut Runtime, seat: SeatId, protocol: Protocol) -> (ClientId, EntityId) {
    let client = runtime
        .connect_local(
            seat,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake3,
            },
            protocol,
            0,
        )
        .unwrap();
    (client, runtime.server.clients[client.0 as usize].entity)
}

fn import(runtime: &mut Runtime, client: ClientId, lifetime: EntityId, text: &[u8]) {
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage =
        ServiceStorage::load_native(&[(ModuleId(9), 0, RuleSetId::Quake3)], 0, &console.cvars)
            .unwrap();
    let mut scratch = runtime.geometry.scratch();
    let (mut services, mut connection, _, _) = runtime.engine_service_parts(
        &mut console,
        &mut storage,
        &mut scratch,
        Some((client, lifetime)),
    );
    services.client_connection = connection
        .as_mut()
        .map(|view| view as &mut dyn ClientConnection);
    let mut memory = ModuleMemory::load(0, 512, &[]).unwrap();
    memory.write_string(16, 496, text).unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut call = Invocation {
        services: &mut services,
        memory: &mut memory,
        native_cvars: None,
        native_resources: None,
        native_entities: None,
        native_surfaces: None,
        native_command: None,
        native_configs: None,
        native_pmove: None,
        native_player: None,
        context: CallContext {
            module: ModuleId(9),
            clock: ThinkTime::Milliseconds(0),
            server_frame: 0,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake3,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
            console: Context::default(),
            allocation: qa_world::entities::AllocationPolicy::q3(0),
            link_order: qa_world::area::LinkOrder::Head,
        },
        platform_time: EventTime(0),
        command: Default::default(),
        argument_selection: &mut ArgumentSelection::Caller,
        addresses: Addresses::Qvm { mask: 511 },
        arguments: &[16],
    };
    assert!(matches!(
        Q3_CLIENT.invoke(16, &mut call, &mut unknown).unwrap(),
        ImportStep::Return(0)
    ));
    assert_eq!(unknown.calls, 0);
}

fn receive(runtime: &mut Runtime, client: ClientId) -> Vec<Vec<u8>> {
    let connection = runtime.network.get_mut(client, Endpoint::Client).unwrap();
    let protocol = connection.commands.as_ref().unwrap().protocol;
    let mut control = [0; 1400];
    let length = connection
        .commands
        .as_ref()
        .unwrap()
        .encode_control(&connection.channel, &mut control)
        .unwrap();
    let body = (protocol == Protocol::Quake3_68).then_some(&control[..length]);
    let packet = connection
        .channel
        .prepare(body, EventTime(1))
        .unwrap()
        .unwrap();
    let bytes = packet.bytes.to_vec();
    connection.channel.submitted(EventTime(1)).unwrap();
    let mut received = Vec::new();
    runtime.network.receive(
        Endpoint::Server.socket(),
        Peer::Loopback(client),
        &bytes,
        EventTime(2),
        |id, endpoint, record| {
            assert_eq!((id, endpoint), (client, Endpoint::Server));
            if let Incoming::CommandString { text, .. } = record {
                received.push(text.to_vec());
            }
            ControlFlow::Continue(())
        },
    );
    received
}

#[test]
fn cgame_send_uses_the_receiving_protocol_and_preserves_raw_command_text() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let (client, lifetime) = connect(&mut runtime, SeatId::FIRST, protocol);
        let text = b" -raw_command  \"quoted value\" ";
        import(&mut runtime, client, lifetime, text);
        assert_eq!(
            receive(&mut runtime, client),
            [text.to_vec()],
            "{protocol:?}"
        );
        assert_eq!(
            runtime
                .network
                .get(client, Endpoint::Client)
                .unwrap()
                .channel
                .counts()
                .client_command_rejected,
            0,
        );
    }
}

#[test]
fn client_command_rejects_stale_or_unconnected_bindings_before_channel_admission() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let (client, lifetime) = connect(&mut runtime, SeatId::FIRST, Protocol::Quake3_68);
    let stale = EntityId {
        generation: lifetime.generation.wrapping_add(1),
        ..lifetime
    };
    assert_eq!(
        send_client_command(&mut runtime, client, stale, b"stale"),
        Err(CallError::Entity)
    );
    runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap()
        .phase = ConnectionPhase::Rejected;
    assert_eq!(
        send_client_command(&mut runtime, client, lifetime, b"rejected"),
        Err(CallError::Entity)
    );
    let channel = &runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel;
    assert_eq!(channel.command_state().unwrap().queued, 0);
    assert_eq!(channel.counts().client_command_rejected, 0);
    runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap()
        .phase = ConnectionPhase::Connected;
    assert_eq!(
        send_client_command(&mut runtime, client, lifetime, b"healthy"),
        Ok(true)
    );
    assert_eq!(receive(&mut runtime, client), [b"healthy".to_vec()]);
}

#[test]
fn full_client_command_queue_keeps_void_import_and_healthy_peer_usable() {
    let mut runtime = Runtime::load(2, []).unwrap();
    let (client, lifetime) = connect(&mut runtime, SeatId::FIRST, Protocol::Quake3_68);
    let (peer, peer_lifetime) = connect(&mut runtime, SeatId::ALL[1], Protocol::Quake3_68);
    let before_ring = runtime.server.events.rejected_publications();
    let mut admitted = 0;
    for _ in 0..128 {
        if !send_client_command(&mut runtime, client, lifetime, b"a").unwrap() {
            break;
        }
        admitted += 1;
    }
    assert!(admitted > 0 && admitted < 128);
    import(&mut runtime, client, lifetime, b"full");
    assert_eq!(
        send_client_command(&mut runtime, client, lifetime, &[b'x'; 2048]),
        Ok(false)
    );
    let channel = &runtime
        .network
        .get(client, Endpoint::Client)
        .unwrap()
        .channel;
    assert_eq!(channel.command_state().unwrap().queued, admitted);
    assert_eq!(channel.counts().client_command_rejected, 3);
    assert_eq!(runtime.server.events.rejected_publications(), before_ring);
    assert!(runtime.server.clients[client.0 as usize].is_active());
    import(&mut runtime, peer, peer_lifetime, b"peer");
    assert_eq!(receive(&mut runtime, peer), [b"peer".to_vec()]);
    assert_eq!(
        runtime
            .network
            .get(peer, Endpoint::Client)
            .unwrap()
            .channel
            .counts()
            .client_command_rejected,
        0
    );
}
