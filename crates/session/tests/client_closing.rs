use qa_core::{
    events::{NativeReceipt, OutputSubmission, OutputTarget},
    primitives::{
        CallbackCall, CallbackId, ClientCallback, ClientId, CommandArgs, CommandExecution,
        EntityId, ModuleId, NativeEntity, RuleSetId, ThinkTime, UserCmd, Vec3,
    },
    sys_events::EventTime,
    text::{InfoBytes, InfoRules},
};
use qa_session::{
    clients::{ClientClosing, ClientPhase, Connection, Server, close_timeout_expired},
    dispatch::{FunctionBinding, FunctionTable},
};

const MODULE: ModuleId = ModuleId(3);

#[test]
fn close_timeout_uses_strict_native_integer_millisecond_boundary() {
    let started = EventTime::from_milliseconds(1234);
    assert!(!close_timeout_expired(
        started,
        EventTime::from_milliseconds(3234),
        2
    ));
    assert!(!close_timeout_expired(
        started,
        EventTime(EventTime::from_milliseconds(3235).0 - 1),
        2
    ));
    assert!(close_timeout_expired(
        started,
        EventTime::from_milliseconds(3235),
        2
    ));

    // Native clocks narrow separately, rather than narrowing elapsed duration.
    let fractional_start = EventTime(started.0 + 999_999);
    assert!(!close_timeout_expired(
        fractional_start,
        EventTime(EventTime::from_milliseconds(3235).0 - 1),
        2
    ));
    assert!(close_timeout_expired(
        fractional_start,
        EventTime::from_milliseconds(3235),
        2
    ));
}

#[test]
fn zero_close_timeout_waits_for_next_native_millisecond_and_negative_expires_immediately() {
    let started = EventTime(EventTime::from_milliseconds(1234).0 + 900_000);
    assert!(!close_timeout_expired(started, started, 0));
    assert!(!close_timeout_expired(
        started,
        EventTime(started.0 + 99_999),
        0
    ));
    assert!(close_timeout_expired(
        started,
        EventTime::from_milliseconds(1235),
        0
    ));
    for seconds in [-1, i32::MIN] {
        assert!(close_timeout_expired(started, started, seconds));
        assert!(close_timeout_expired(
            started,
            EventTime(started.0 + 1),
            seconds
        ));
    }
}

#[test]
fn backwards_raw_clock_never_expires_even_inside_same_native_millisecond() {
    let started = EventTime(EventTime::from_milliseconds(1234).0 + 900_000);
    let backwards = EventTime(started.0 - 1);
    assert_eq!(started.milliseconds(), backwards.milliseconds());
    for seconds in [i32::MIN, -1, 0, 2] {
        assert!(!close_timeout_expired(started, backwards, seconds));
        assert!(!close_timeout_expired(
            started,
            EventTime::from_milliseconds(1233),
            seconds
        ));
    }
}

fn connected(count: usize) -> (Server, ClientId) {
    let mut server = Server::load(count, count + 4, 8, 8, 8, 0).unwrap();
    let id = server
        .connect(
            Connection::Remote,
            MODULE,
            Some(NativeEntity {
                module: MODULE,
                slot: 0,
            }),
        )
        .unwrap();
    (server, id)
}

fn closing(server: &Server, id: ClientId) -> ClientClosing {
    let ClientPhase::Closing(closing) = server.clients[id.0 as usize].phase else {
        panic!("client must retain its closing phase");
    };
    closing
}

#[test]
fn begin_close_keeps_public_state_and_transport_available_to_disconnect_callback() {
    fn disconnect(server: &mut Server, module: ModuleId, entry: u32, call: CallbackCall) -> bool {
        let CallbackCall::Client {
            entity,
            client,
            event: ClientCallback::Disconnect,
            ..
        } = call
        else {
            return false;
        };
        assert_eq!(module, MODULE);
        assert_eq!(entry, 70_003);
        let row = &server.clients[client.0 as usize];
        assert_eq!(row.entity, entity);
        assert_eq!(server.entities.resolve(entity), Some(entity.slot as usize));
        assert_eq!(row.connection, Some(Connection::Remote));
        assert!(!row.is_active());
        assert_eq!(row.player.health, 73);
        assert_eq!(row.player.armor, 11);
        assert_eq!(row.player.inventory.item_counts[4], 5);
        assert_eq!(row.hud.inventory.item_counts[4], 9);
        assert_eq!(row.command.duration_ms, 37);
        assert_eq!(row.command.view_angles, Vec3([1.0, 2.0, 3.0]));
        assert_eq!(row.module_spawn_parms[15], 0x8000_0017);
        assert_eq!(
            server.userinfo(client),
            Some(b"\\name\\retained".as_slice())
        );
        assert!(server.events.counters(row.output.unwrap()).is_some());
        server.clients[client.0 as usize].player.health += 1;
        true
    }
    let (mut server, id) = connected(1);
    let lifetime = server.clients[id.0 as usize].entity;
    let output = server.clients[id.0 as usize].output;
    let player = &mut server.clients[id.0 as usize].player;
    player.health = 73;
    player.armor = 11;
    player.inventory.item_counts[4] = 5;
    server.clients[id.0 as usize].hud.inventory.item_counts[4] = 9;
    server.clients[id.0 as usize].module_spawn_parms[15] = 0x8000_0017;
    server
        .replace_userinfo(
            id,
            b"\\name\\retained",
            InfoRules {
                bytes: 1024,
                key_bytes: 1024,
                value_bytes: 1024,
                filter: InfoBytes::Exact,
            },
        )
        .unwrap();
    server.submit_command(
        id,
        UserCmd {
            duration_ms: 37,
            view_angles: Vec3([1.0, 2.0, 3.0]),
            ..Default::default()
        },
        CommandExecution::PerCommand,
        None,
    );
    let start = EventTime::from_milliseconds(1234);
    assert!(server.begin_close(id, lifetime, start));
    assert_eq!(
        closing(&server, id),
        ClientClosing {
            started: start,
            final_output: None,
            acknowledged: false,
        }
    );
    assert!(!server.begin_close(id, lifetime, EventTime::from_milliseconds(9999)));
    assert_eq!(closing(&server, id).started, start);
    assert_eq!(server.clients[id.0 as usize].output, output);
    assert!(server.connect(Connection::Bot, MODULE, None).is_none());
    assert!(server.take_pending_command(id).is_none());
    server.submit_command(
        id,
        UserCmd {
            duration_ms: 255,
            ..Default::default()
        },
        CommandExecution::PerCommand,
        None,
    );
    let functions = FunctionTable::load([(
        MODULE,
        RuleSetId::Quake3,
        vec![FunctionBinding {
            entry: 70_003,
            call: disconnect,
        }],
    )])
    .unwrap();
    functions
        .invoke(
            &mut server,
            CallbackId(0),
            CallbackCall::Client {
                entity: lifetime,
                client: id,
                event: ClientCallback::Disconnect,
                time: ThinkTime::Milliseconds(1234),
                arguments: [0; 9],
                command: CommandArgs::default(),
            },
        )
        .unwrap();
    assert_eq!(server.clients[id.0 as usize].player.health, 74);
    assert_eq!(server.clients[id.0 as usize].output, output);
    assert_eq!(closing(&server, id).final_output, None);
}

#[test]
fn close_claim_requires_current_lifetime_and_acknowledgement_matches_consumer_and_sequence() {
    let (mut server, id) = connected(2);
    let peer = server
        .connect(Connection::Remote, ModuleId(4), None)
        .unwrap();
    let lifetime = server.clients[id.0 as usize].entity;
    let output = server.clients[id.0 as usize].output.unwrap();
    let peer_output = server.clients[peer.0 as usize].output.unwrap();
    let stale = EntityId {
        generation: lifetime.generation.wrapping_add(1),
        ..lifetime
    };
    let invalid = ClientId(u32::MAX);
    let now = EventTime::from_milliseconds(50);
    assert!(!server.begin_close(invalid, lifetime, now));
    assert!(!server.begin_close(id, stale, now));
    assert!(!server.finish_close(id, lifetime, 7));
    assert!(!server.acknowledge_close(id, output, 7));
    assert!(server.begin_close(id, lifetime, now));
    assert!(!server.finish_close(invalid, lifetime, 7));
    assert!(!server.finish_close(id, stale, 7));
    assert!(server.finish_close(id, lifetime, 7));
    assert!(!server.finish_close(id, lifetime, 8));
    assert_eq!(closing(&server, id).final_output, Some((output, 7)));
    assert!(!server.acknowledge_close(invalid, output, 7));
    assert!(!server.acknowledge_close(id, peer_output, 7));
    assert!(!server.acknowledge_close(id, output, 6));
    assert!(!server.acknowledge_close(id, output, 8));
    assert!(!closing(&server, id).acknowledged);
    assert!(server.acknowledge_close(id, output, 7));
    assert!(!server.acknowledge_close(id, output, 7));
    assert!(closing(&server, id).acknowledged);
    assert!(server.clients[peer.0 as usize].is_active());
    assert_eq!(server.clients[peer.0 as usize].output, Some(peer_output));
}

#[test]
fn reliable_submission_and_wrong_receipt_do_not_acknowledge_the_final_close_record() {
    let (mut server, id) = connected(1);
    let lifetime = server.clients[id.0 as usize].entity;
    let output = server.clients[id.0 as usize].output.unwrap();
    assert!(server.begin_close(id, lifetime, EventTime::from_milliseconds(10)));
    let sequence = server
        .events
        .server_command(Some(id), MODULE, RuleSetId::Quake3, b"disconnect")
        .unwrap();
    assert!(server.finish_close(id, lifetime, sequence));
    assert!(server.events.submit(
        output,
        sequence,
        OutputSubmission::Reliable(NativeReceipt(700))
    ));
    assert!(server.events.retained_for(output, sequence));
    assert!(!closing(&server, id).acknowledged);
    let mut retired = None;
    assert_eq!(
        server
            .events
            .acknowledge_with(output, NativeReceipt(699), |record| {
                retired = Some(record.sequence)
            }),
        0
    );
    assert_eq!(retired, None);
    assert!(!closing(&server, id).acknowledged);
    assert_eq!(
        server
            .events
            .acknowledge_with(output, NativeReceipt(700), |record| {
                retired = Some(record.sequence)
            }),
        1
    );
    assert_eq!(retired, Some(sequence));
    assert!(server.acknowledge_close(id, output, retired.unwrap()));
    assert!(!server.events.retained_for(output, sequence));
    assert!(closing(&server, id).acknowledged);
    assert_eq!(
        server.clients[id.0 as usize].connection,
        Some(Connection::Remote)
    );
}

#[test]
fn terminal_disconnect_resets_state_and_reused_client_rejects_old_lifetime_and_receipt() {
    let (mut server, id) = connected(1);
    let old = server.clients[id.0 as usize].entity;
    let old_output = server.clients[id.0 as usize].output.unwrap();
    server.clients[id.0 as usize].player.health = 91;
    server.clients[id.0 as usize].player.inventory.item_counts[4] = 6;
    server.clients[id.0 as usize].command.duration_ms = 37;
    server.replace_userinfo_raw(id, b"retained", 1024).unwrap();
    assert!(server.begin_close(id, old, EventTime::from_milliseconds(10)));
    assert!(server.finish_close(id, old, 7));
    assert!(server.disconnect(id));
    assert!(!server.disconnect(id));
    assert!(!server.clients[id.0 as usize].is_active());
    assert_eq!(server.clients[id.0 as usize].connection, None);
    assert_eq!(server.clients[id.0 as usize].phase, ClientPhase::Active);
    assert_eq!(server.clients[id.0 as usize].output, None);
    assert_eq!(server.clients[id.0 as usize].player.health, 0);
    assert_eq!(
        server.clients[id.0 as usize].player.inventory.item_counts[4],
        0
    );
    assert_eq!(server.clients[id.0 as usize].command.duration_ms, 0);
    assert_eq!(server.userinfo(id), None);
    assert!(server.events.counters(old_output).is_none());
    assert!(server.entities.resolve(old).is_none());

    assert_eq!(
        server.connect(Connection::Remote, ModuleId(4), None),
        Some(id)
    );
    let replacement = server.clients[id.0 as usize].entity;
    let new_output = server.clients[id.0 as usize].output.unwrap();
    assert_eq!(replacement.slot, old.slot);
    assert_ne!(replacement.generation, old.generation);
    assert_ne!(new_output, old_output);
    assert!(server.clients[id.0 as usize].is_active());
    assert_eq!(server.userinfo(id), Some(b"".as_slice()));
    assert!(!server.begin_close(id, old, EventTime::from_milliseconds(20)));
    assert!(!server.finish_close(id, old, 7));
    assert!(!server.acknowledge_close(id, old_output, 7));
    assert!(server.begin_close(id, replacement, EventTime::from_milliseconds(30)));
    assert!(server.finish_close(id, replacement, 7));
    assert!(!server.acknowledge_close(id, old_output, 7));
    assert!(!closing(&server, id).acknowledged);
    assert!(server.acknowledge_close(id, new_output, 7));
}

#[test]
fn finishing_close_captures_the_current_output_consumer_and_bot_has_no_final_transport() {
    let (mut server, id) = connected(2);
    let bot = server.connect(Connection::Bot, ModuleId(4), None).unwrap();
    let lifetime = server.clients[id.0 as usize].entity;
    let old_output = server.clients[id.0 as usize].output.unwrap();
    assert!(server.begin_close(id, lifetime, EventTime::from_milliseconds(10)));
    server.events.unbind(old_output);
    let current = server.events.bind(OutputTarget::Client(id)).unwrap();
    server.clients[id.0 as usize].output = Some(current);
    assert_ne!(current, old_output);
    assert!(server.finish_close(id, lifetime, 7));
    assert_eq!(closing(&server, id).final_output, Some((current, 7)));
    assert!(!server.acknowledge_close(id, old_output, 7));
    assert!(server.acknowledge_close(id, current, 7));
    let bot_lifetime = server.clients[bot.0 as usize].entity;
    assert!(server.begin_close(bot, bot_lifetime, EventTime::from_milliseconds(11)));
    assert!(!server.finish_close(bot, bot_lifetime, 7));
    assert_eq!(closing(&server, bot).final_output, None);
}
