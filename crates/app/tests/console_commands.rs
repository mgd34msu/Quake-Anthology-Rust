use qa_app::{
    Runtime,
    host::FrameHost,
    modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
};
use qa_compat::{
    abi::{CallTable, Q3_CLIENT, Q3_SERVER, Q3_UI},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{CallbackId, ClientId, ModuleId, RuleSetId, ThinkTime},
    sys_events::SeatId,
};
use qa_formats::program::qvm::Opcode::{self, *};
use qa_session::timing::TickRate;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;
#[path = "support/visibility_world.rs"]
mod visibility_world;

type Code = Vec<(Opcode, i32)>;
#[derive(Default)]
struct Data(Vec<u8>);
impl Data {
    fn text(&mut self, text: &[u8]) -> i32 {
        if self.0.is_empty() {
            self.0.resize(128, 0);
        }
        let at = self.0.len() as i32;
        self.0.extend_from_slice(text);
        self.0.push(0);
        at
    }
}
fn call(code: &mut Code, number: i32, arguments: &[i32]) {
    for (index, &word) in arguments.iter().enumerate() {
        code.extend([(Const, word), (Arg, 8 + index as i32 * 4)]);
    }
    code.extend([(Const, -number - 1), (Call, 0)]);
}
fn syscall(code: &mut Code, number: i32, arguments: &[i32]) {
    call(code, number, arguments);
    code.push((Pop, 0));
}
fn vm(bodies: &[(i32, Code)], mut data: Data) -> qa_compat::qvm::Vm {
    let mut code = vec![(Enter, 64)];
    let mut jumps = Vec::new();
    for (entry, _) in bodies {
        code.extend([(Local, 72), (Load4, 0), (Const, *entry)]);
        jumps.push(code.len());
        code.push((Eq, 0));
    }
    code.extend([(Const, 0), (Leave, 64)]);
    for ((_, body), jump) in bodies.iter().zip(jumps) {
        code[jump].1 = code.len() as i32;
        code.extend_from_slice(body);
    }
    data.0.resize((data.0.len() + 3) & !3, 0);
    service_program::program(&code, &data.0)
}
fn request(
    runtime: &mut Runtime,
    module: u16,
    imports: &'static CallTable,
    seat: SeatId,
    vm: qa_compat::qvm::Vm,
) -> ModuleRequest {
    let module = ModuleId(module);
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
        .unwrap()
        .id;
    ModuleRequest {
        search: None,
        q3_image: None,
        context: CallContext {
            module,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::Quake3,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
            server_frame: 0,
            clock: ThinkTime::Milliseconds(0),
            allocation: AllocationPolicy::q3(0),
            link_order: LinkOrder::Head,
            console: Context {
                seat,
                ..Context::default()
            },
        },
        timing_rules: RuleSetId::Quake3,
        phase: if std::ptr::eq(imports, &Q3_SERVER) {
            Phase::Server
        } else {
            Phase::Client
        },
        rate: TickRate::FrameDriven,
        anchor,
        program: Program::Qvm {
            vm: Box::new(vm),
            imports,
        },
        // Console export admission must resolve/append its native ordinal at load.
        entries: vec![0, 10],
        frame: Export::words(CallbackId(0), &[]),
        prepare: Vec::new(),
        initialize: Vec::new(),
        api: None,
        shutdown: Vec::new(),
        instruction_budget: 10000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    }
}
fn host(runtime: Runtime, requests: Vec<ModuleRequest>) -> FrameHost {
    FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap()
}
fn value(host: &FrameHost, name: &str) -> Option<f32> {
    host.console
        .cvars
        .find(name)
        .map(|handle| host.console.cvars.value(handle))
}
fn invoke(host: &mut FrameHost, module: u16) {
    host.call_module_export(
        ModuleId(module),
        Export::words(CallbackId(1), &[]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
}

fn connect(runtime: &mut Runtime, protocol: qa_network::commands::packet::Protocol) -> ClientId {
    runtime
        .connect_local(
            SeatId::FIRST,
            qa_app::map::SpawnAnchor {
                position: Default::default(),
                angles: Default::default(),
                entity: 0,
                fixture_fallback: false,
            },
            qa_app::client_policy::ClientPolicy {
                input: RuleSetId::Quake3,
                client: RuleSetId::Quake3,
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake3,
            },
            protocol,
            0,
        )
        .unwrap()
}

#[test]
fn ordinary_console_executes_without_modules_through_the_owned_driver() {
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        Runtime::load(1, []).unwrap(),
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.console
        .append("set _qa_root 9\n", Context::default())
        .unwrap();
    host.execute_commands();
    assert_eq!(value(&host, "_qa_root"), Some(9.0));
    assert!(host.console.idle());
}

#[test]
fn command_export_cannot_legitimize_an_invalid_original_callback_index() {
    for lifecycle in [false, true] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let mut request = request(
            &mut runtime,
            1,
            &Q3_SERVER,
            SeatId::FIRST,
            vm(&[], Data::default()),
        );
        let invalid = CallbackId(request.entries.len() as u32);
        if lifecycle {
            request.prepare = vec![Export::words(invalid, &[])];
        } else {
            request.frame.callback = invalid;
        }
        assert!(
            FrameHost::load_modules(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::FrameDriven,
                vec![request]
            )
            .is_err()
        );
    }
}

#[test]
fn quit_inside_exec_now_clears_pending_work_and_post_quit_shutdown_still_runs() {
    let mut data = Data::default();
    let quit = data.text(b"quit");
    let marker = data.text(b"_qa_shutdown");
    let one = data.text(b"1");
    let mut body = Code::new();
    syscall(&mut body, 14, &[0, quit]);
    body.extend([(Const, 0), (Leave, 64)]);
    let mut shutdown = Code::new();
    syscall(&mut shutdown, 5, &[marker, one]);
    shutdown.extend([(Const, 0), (Leave, 64)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut request = request(
        &mut runtime,
        1,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(10, body), (11, shutdown)], data),
    );
    request.entries.push(11);
    request.shutdown.push(Export::words(CallbackId(2), &[]));
    let mut host = host(runtime, vec![request]);
    host.console
        .append("set _qa_after_quit 1\n", Context::default())
        .unwrap();
    invoke(&mut host, 1);
    assert!(host.runtime.quit);
    assert!(host.console.idle());
    assert_eq!(value(&host, "_qa_after_quit"), None);
    host.shutdown_modules();
    assert_eq!(value(&host, "_qa_shutdown"), Some(1.0));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
}

#[test]
fn server_exec_now_is_immediate_without_draining_pending_or_derived_commands() {
    let mut data = Data::default();
    let alias = data.text(b"alias native_route \"set _qa_derived 1\"");
    let route = data.text(b"native_route");
    let command = data.text(b"set _qa_immediate 7");
    let name = data.text(b"_qa_immediate");
    let mut body = Code::new();
    syscall(&mut body, 14, &[0, alias]);
    syscall(&mut body, 14, &[0, route]);
    syscall(&mut body, 14, &[0, command]);
    call(&mut body, 6, &[name]);
    body.push((Leave, 64));
    let mut runtime = Runtime::load(1, []).unwrap();
    let request = request(
        &mut runtime,
        1,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(10, body)], data),
    );
    let mut host = host(runtime, vec![request]);
    host.console
        .append("set _qa_queued 3\n", Context::default())
        .unwrap();
    invoke(&mut host, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(7))
    );
    assert_eq!(value(&host, "_qa_immediate"), Some(7.0));
    assert_eq!(value(&host, "_qa_queued"), None);
    assert_eq!(value(&host, "_qa_derived"), None);
    host.execute_commands();
    assert_eq!(value(&host, "_qa_derived"), Some(1.0));
    assert_eq!(value(&host, "_qa_queued"), Some(3.0));
}

#[test]
fn null_and_empty_exec_now_drain_the_existing_wait_once_per_invocation() {
    for null in [true, false] {
        let mut data = Data::default();
        let empty = data.text(b"");
        let tail = data.text(b"_qa_tail");
        let mut body = Code::new();
        syscall(&mut body, 14, &[0, if null { 0 } else { empty }]);
        call(&mut body, 6, &[tail]);
        body.push((Leave, 64));
        let mut runtime = Runtime::load(1, []).unwrap();
        let request = request(
            &mut runtime,
            1,
            &Q3_SERVER,
            SeatId::FIRST,
            vm(&[(10, body)], data),
        );
        let mut host = host(runtime, vec![request]);
        host.console
            .append(
                "set _qa_prefix 1; wait 2; set _qa_tail 1\n",
                Context::default(),
            )
            .unwrap();
        for expected in [0, 0, 1] {
            invoke(&mut host, 1);
            assert_eq!(
                host.module_counts(ModuleId(1)).unwrap().last_result,
                Some(ModuleResult::Qvm(expected))
            );
        }
        assert_eq!(value(&host, "_qa_prefix"), Some(1.0));
        assert_eq!(value(&host, "_qa_tail"), Some(1.0));
    }
}

#[test]
fn nonzero_qvm_pointer_wrapping_to_zero_executes_text_instead_of_draining() {
    let mut data = Data(b"set _qa_wrapped 1\0".to_vec());
    data.0.resize(128, 0);
    let mut body = Code::new();
    syscall(&mut body, 14, &[0, 65536]);
    body.extend([(Const, 0), (Leave, 64)]);
    let mut runtime = Runtime::load(1, []).unwrap();
    let request = request(
        &mut runtime,
        1,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(10, body)], data),
    );
    let mut host = host(runtime, vec![request]);
    host.console
        .append("set _qa_pending 1\n", Context::default())
        .unwrap();
    invoke(&mut host, 1);
    assert_eq!(value(&host, "_qa_wrapped"), Some(1.0));
    assert_eq!(value(&host, "_qa_pending"), None);
}

fn retire_producer(
    _: &mut Console<Runtime>,
    runtime: &mut Runtime,
    _: &qa_console::command_text::Arguments<'_>,
    _: Context,
) -> Result<(), qa_console::commands::CommandError> {
    runtime
        .server
        .retire_module_entities(ModuleId(2), ThinkTime::Milliseconds(0));
    Ok(())
}

#[test]
fn retiring_the_command_producer_during_fallback_never_forwards_its_text() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = connect(
        &mut runtime,
        qa_network::commands::packet::Protocol::Quake3_68,
    );
    runtime.server.clients[client.0 as usize].module = ModuleId(1);
    let mut data = Data::default();
    let command = data.text(b"retire_producer");
    let mut callback = Code::new();
    syscall(&mut callback, 14, &[0, command]);
    callback.extend([(Const, 0), (Leave, 64)]);
    let server = request(
        &mut runtime,
        1,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(9, callback)], data),
    );
    let ui = request(
        &mut runtime,
        2,
        &Q3_UI,
        SeatId::FIRST,
        vm(&[], Data::default()),
    );
    let producer = qa_console::views::CommandProducer {
        module: ModuleId(2),
        anchor: ui.anchor,
    };
    let mut host = host(runtime, vec![server, ui]);
    host.local_clients[SeatId::FIRST.index()] = Some(client);
    assert!(host.console.register("retire_producer", retire_producer));
    host.console
        .append(
            "native_command; set _qa_stale 1\n",
            Context {
                producer: Some(producer),
                ..Context::default()
            },
        )
        .unwrap();
    host.console
        .append("set _qa_survivor 1\n", Context::default())
        .unwrap();
    host.execute_commands();
    assert_eq!(value(&host, "_qa_stale"), None);
    assert_eq!(value(&host, "_qa_survivor"), Some(1.0));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().calls, 1);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 0);
    assert_eq!(
        host.runtime
            .network
            .get(client, qa_core::loopback::Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        0
    );
}

#[test]
fn unclaimed_console_text_reaches_the_selected_connections_native_channel() {
    use qa_core::{loopback::Endpoint, sys_events::EventTime};
    use qa_network::commands::packet::Protocol;
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let client = connect(&mut runtime, protocol);
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        host.local_clients[SeatId::FIRST.index()] = Some(client);
        host.console
            .append_line_bytes(b"native_command \"raw bytes\"", Context::default())
            .unwrap();
        host.execute_commands();
        let connection = host
            .runtime
            .network
            .get_mut(client, Endpoint::Client)
            .unwrap();
        let mut control = [0; 1400];
        let length = connection
            .commands
            .as_mut()
            .unwrap()
            .encode_control(&connection.channel, &mut control)
            .unwrap();
        let body = (protocol == Protocol::Quake3_68).then_some(&control[..length]);
        assert!(
            connection
                .channel
                .prepare(body, EventTime(1))
                .unwrap()
                .is_some()
        );
    }
}

fn forwarded(
    host: &mut FrameHost,
    client: ClientId,
    protocol: qa_network::commands::packet::Protocol,
) -> Vec<Vec<u8>> {
    use qa_core::{loopback::Endpoint, sys_events::EventTime};
    use qa_network::{
        channel::{Channel, Delivery},
        commands::{connection::Commands, packet::Protocol},
        ingress::Incoming,
    };
    let connection = host
        .runtime
        .network
        .get_mut(client, Endpoint::Client)
        .unwrap();
    let mut control = [0; 1400];
    let length = connection
        .commands
        .as_ref()
        .unwrap()
        .encode_control(&connection.channel, &mut control)
        .unwrap();
    let body = (protocol == Protocol::Quake3_68).then_some(&control[..length]);
    let Some(packet) = connection.channel.prepare(body, EventTime(1)).unwrap() else {
        return vec![];
    };
    let bytes = packet.bytes.to_vec();
    connection.channel.submitted(EventTime(1)).unwrap();
    let mut server = Channel::load(protocol.channel(), Endpoint::Server, 8192, 8).unwrap();
    let received = server.receive(&bytes, EventTime(2)).unwrap();
    let sequence = received.header.sequence;
    let Delivery::Payload(payload) = received.delivery else {
        panic!("complete native packet");
    };
    let payload = payload.to_vec();
    let mut commands = Commands::load(protocol, 8192);
    let length = commands.stage(&payload).unwrap();
    let mut result = Vec::new();
    commands
        .decode(length, sequence, 0, &mut server, None, |record| {
            if let Incoming::CommandString { text, .. } = record {
                result.push(text.to_vec());
            }
            std::ops::ControlFlow::Continue(())
        })
        .unwrap();
    result
}

#[test]
fn q3_final_forwarding_uses_parsed_names_and_preserves_multiple_argument_text() {
    use qa_network::commands::packet::Protocol;
    for protocol in [Protocol::Quake3_68, Protocol::Quake2_34] {
        for (text, expected, diagnostic) in [
            (b"  \"-qa_unregistered\"  ".as_slice(), None, false),
            (b"  \"+qa_unregistered\"  ".as_slice(), None, true),
            (
                b"  \"qa_single\"  ".as_slice(),
                Some(b"qa_single".as_slice()),
                false,
            ),
            (
                b"   qa_many   \"two words\" ".as_slice(),
                Some(b"   qa_many   \"two words\" ".as_slice()),
                false,
            ),
        ] {
            let mut runtime = Runtime::load(1, []).unwrap();
            let client = connect(&mut runtime, protocol);
            let mut host = FrameHost::load(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::FrameDriven,
                vec![],
            )
            .unwrap();
            host.local_clients[SeatId::FIRST.index()] = Some(client);
            let before = host.runtime.server.events.sequence_range().end;
            host.console
                .append_line_bytes(
                    text,
                    Context {
                        source: RuleSetId::Quake3,
                        ..Context::default()
                    },
                )
                .unwrap();
            host.execute_commands();
            assert_eq!(
                host.runtime.server.events.sequence_range().end > before,
                diagnostic
            );
            assert_eq!(
                forwarded(&mut host, client, protocol),
                expected.into_iter().map(<[u8]>::to_vec).collect::<Vec<_>>()
            );
        }
    }
    // The console's source controls its policy even on a Q3 wire connection.
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = connect(&mut runtime, Protocol::Quake3_68);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[SeatId::FIRST.index()] = Some(client);
    host.console
        .append_line_bytes(
            b"  \"+qa_unregistered\"  ",
            Context {
                source: RuleSetId::Quake2,
                ..Context::default()
            },
        )
        .unwrap();
    host.execute_commands();
    assert_eq!(
        forwarded(&mut host, client, Protocol::Quake3_68),
        [b"  \"+qa_unregistered\"  ".to_vec()]
    );
}

#[test]
fn ui_exec_now_reenters_server_console_export_and_restores_outer_arguments() {
    let mut data = Data::default();
    let outer = data.text(b"native_command \"two words\" tail");
    let variable = data.text(b"_qa_nested");
    let mut caller = Code::new();
    syscall(&mut caller, 12, &[0, outer]);
    call(&mut caller, 4, &[variable]); // UI returns float cvar value.
    caller.push((Leave, 64));
    let caller = vm(&[(10, caller)], data);

    let mut server_data = Data::default();
    let nested_server = server_data.text(b"set _qa_nested 1");
    let mut callback = Code::new();
    syscall(&mut callback, 14, &[0, nested_server]);
    call(&mut callback, 8, &[]);
    callback.push((Leave, 64));
    let mut runtime = Runtime::load(1, []).unwrap();
    let ui = request(&mut runtime, 2, &Q3_UI, SeatId::FIRST, caller);
    let server = request(
        &mut runtime,
        1,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(9, callback)], server_data),
    );
    let mut host = host(runtime, vec![ui, server]);
    invoke(&mut host, 2);
    assert_eq!(value(&host, "_qa_nested"), Some(1.0));
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(3))
    );
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::Qvm(1f32.to_bits() as i32))
    );
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

#[test]
fn fallback_zero_and_trap_continue_to_the_same_seats_ui_and_nonzero_claim_stops() {
    for trapped in [false, true] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let mut server_body = Code::new();
        if trapped {
            server_body.extend([(Const, 1), (Const, 0), (DivI, 0), (Pop, 0)]);
        }
        server_body.extend([(Const, 0), (Leave, 64)]);
        let server = request(
            &mut runtime,
            1,
            &Q3_SERVER,
            SeatId::FIRST,
            vm(&[(9, server_body)], Data::default()),
        );
        let mut data = Data::default();
        let marker = data.text(b"_qa_ui_claimed");
        let one = data.text(b"1");
        let mut body = Code::new();
        syscall(&mut body, 3, &[marker, one]);
        body.extend([(Local, 76), (Load4, 0), (Leave, 64)]); // UI receives current platform milliseconds.
        let ui = request(
            &mut runtime,
            2,
            &Q3_UI,
            SeatId::FIRST,
            vm(&[(8, body)], data),
        );
        let other_seat = request(
            &mut runtime,
            3,
            &Q3_UI,
            SeatId::ALL[1],
            vm(&[(8, vec![(Const, 1), (Leave, 64)])], Data::default()),
        );
        let mut host = host(runtime, vec![server, ui, other_seat]);
        host.time = qa_core::sys_events::EventTime::from_milliseconds(42);
        host.console
            .append("native_command a\n", Context::default())
            .unwrap();
        host.execute_commands();
        assert_eq!(value(&host, "_qa_ui_claimed"), Some(1.0));
        assert_eq!(
            host.module_counts(ModuleId(2)).unwrap().last_result,
            Some(ModuleResult::Qvm(42))
        );
        assert_eq!(host.module_counts(ModuleId(3)).unwrap().calls, 0);
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().traps,
            u64::from(trapped)
        );
    }
}

#[test]
fn reserved_and_disconnected_seat_assignments_still_reach_unbound_ui_commands() {
    for disconnected in [false, true] {
        let mut runtime = Runtime::load(1, []).unwrap();
        let client = if disconnected {
            let client = connect(
                &mut runtime,
                qa_network::commands::packet::Protocol::Quake3_68,
            );
            assert!(runtime.disconnect_client(client));
            client
        } else {
            ClientId(0)
        };
        let mut data = Data::default();
        let marker = data.text(b"_qa_disconnected_ui");
        let one = data.text(b"1");
        let mut body = Code::new();
        syscall(&mut body, 3, &[marker, one]);
        body.extend([(Const, 1), (Leave, 64)]);
        let ui = request(
            &mut runtime,
            1,
            &Q3_UI,
            SeatId::FIRST,
            vm(&[(8, body)], data),
        );
        let mut host = host(runtime, vec![ui]);
        host.local_clients[SeatId::FIRST.index()] = Some(client);
        host.console
            .append("native_command\n", Context::default())
            .unwrap();
        host.execute_commands();
        assert_eq!(value(&host, "_qa_disconnected_ui"), Some(1.0));
        assert_eq!(
            host.module_counts(ModuleId(1)).unwrap().last_result,
            Some(ModuleResult::Qvm(1))
        );
    }
}

#[derive(Default)]
struct RetirementOutput {
    retired: Option<(ClientId, qa_core::primitives::EntityId)>,
}
impl qa_app::host::FrameSource for RetirementOutput {
    fn begin_frame(&mut self) -> std::time::Duration {
        unreachable!()
    }
    fn poll_events(&mut self, _: &mut qa_core::sys_events::SysEventQueue) {
        unreachable!()
    }
    fn wait_time(&mut self, _: std::time::Duration) -> std::time::Duration {
        unreachable!()
    }
    fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
    fn present(&mut self) {
        unreachable!()
    }
    fn sound(
        &mut self,
        _bank: Option<&qa_audio::Bank>,
        event: qa_core::primitives::SoundEvent,
    ) -> bool {
        if event.action != qa_core::primitives::SoundAction::RetireClient {
            return false;
        }
        assert!(self.retired.is_none());
        self.retired = Some((event.client.unwrap(), event.client_lifetime.unwrap()));
        true
    }
}

fn replace_connection(
    console: &mut Console<Runtime>,
    runtime: &mut Runtime,
    _: &qa_console::command_text::Arguments<'_>,
    _: Context,
) -> Result<(), qa_console::commands::CommandError> {
    let old = runtime.server.clients[0].entity;
    assert!(runtime.disconnect_client(ClientId(0)));
    // The slot is unavailable until its existing Presentation owner consumes
    // the lifetime's retirement control, even when the fixture has no voices.
    let mut source = RetirementOutput::default();
    let local =
        std::array::from_fn(|index| (index == SeatId::FIRST.index()).then_some(ClientId(0)));
    let counts = qa_app::output::dispatch(
        runtime,
        &mut source,
        &mut console.scrollback,
        &local,
        Default::default(),
        3.0,
        2.0,
    );
    assert_eq!(source.retired, Some((ClientId(0), old)));
    assert_eq!(counts.unhandled_sounds, 0);
    assert!(runtime.server.clients[0].audio_retirement.is_none());
    assert_eq!(
        connect(runtime, qa_network::commands::packet::Protocol::Quake3_68),
        ClientId(0)
    );
    Ok(())
}

#[test]
fn reconnecting_the_selected_seat_during_fallback_cannot_retarget_the_old_command() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = connect(
        &mut runtime,
        qa_network::commands::packet::Protocol::Quake3_68,
    );
    let old = runtime.server.clients[client.0 as usize].entity;
    runtime.server.clients[client.0 as usize].module = ModuleId(1);
    let mut data = Data::default();
    let command = data.text(b"replace_connection");
    let mut body = Code::new();
    syscall(&mut body, 14, &[0, command]);
    body.extend([(Const, 0), (Leave, 64)]);
    let server = request(
        &mut runtime,
        1,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(9, body)], data),
    );
    let ui = request(
        &mut runtime,
        2,
        &Q3_UI,
        SeatId::FIRST,
        vm(&[(8, vec![(Const, 1), (Leave, 64)])], Data::default()),
    );
    let mut host = host(runtime, vec![server, ui]);
    assert!(
        host.console
            .register("replace_connection", replace_connection)
    );
    host.local_clients[SeatId::FIRST.index()] = Some(client);
    host.console
        .append(
            "native_command; set _qa_reconnect_survivor 1\n",
            Context::default(),
        )
        .unwrap();
    host.execute_commands();
    assert_ne!(host.runtime.server.clients[client.0 as usize].entity, old);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 0);
    assert_eq!(value(&host, "_qa_reconnect_survivor"), Some(1.0));
    assert_eq!(
        host.runtime
            .network
            .get(client, qa_core::loopback::Endpoint::Client)
            .unwrap()
            .channel
            .command_state()
            .unwrap()
            .queued,
        0
    );
}

#[test]
fn cgame_caller_bytes_and_explicit_retained_selection_survive_console_fallback() {
    let mut runtime = Runtime::load(1, []).unwrap();
    let client = connect(
        &mut runtime,
        qa_network::commands::packet::Protocol::Quake3_68,
    );
    let mut data = Data::default();
    data.0.resize(512, 0);
    let mut body = Code::new();
    syscall(&mut body, 8, &[1, 128, 64]); // cgame argv.
    body.extend([
        (Const, 400),
        (Const, 128),
        (Load1, 0),
        (Store4, 0),
        (Const, 404),
    ]);
    call(&mut body, 7, &[]);
    body.push((Store4, 0));
    syscall(&mut body, 53, &[1]); // GetServerCommand explicitly selects retained CLIENT argv.
    body.extend([
        (Const, 400),
        (Load4, 0),
        (Const, 10000),
        (MulI, 0),
        (Const, 404),
        (Load4, 0),
        (Const, 100),
        (MulI, 0),
        (Add, 0),
    ]);
    call(&mut body, 7, &[]);
    body.extend([(Add, 0), (Leave, 64)]);
    let mut ambient = Code::new();
    call(&mut ambient, 7, &[]);
    ambient.push((Leave, 64));
    let mut cgame = request(
        &mut runtime,
        1,
        &Q3_CLIENT,
        SeatId::FIRST,
        vm(&[(2, body), (10, ambient)], data),
    );
    cgame.frame.client = Some(client);
    let server = request(
        &mut runtime,
        2,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(9, vec![(Const, 1), (Leave, 64)])], Data::default()),
    );
    let mut host = host(runtime, vec![cgame, server]);
    host.local_clients[SeatId::FIRST.index()] = Some(client);
    host.runtime
        .network
        .get_mut(client, qa_core::loopback::Endpoint::Client)
        .unwrap()
        .channel
        .receive_command(1, b"retained from server now")
        .unwrap();
    host.console
        .append_line_bytes(b"native_command \xff", Context::default())
        .unwrap();
    host.execute_commands();
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(2550204))
    );
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().calls, 0);
    invoke(&mut host, 1);
    assert_eq!(
        host.module_counts(ModuleId(1)).unwrap().last_result,
        Some(ModuleResult::Qvm(4))
    );
}

const TRACE_ENTITIES: i32 = 4096;
const TRACE_STRIDE: i32 = 608;
const TRACE_ACTOR: i32 = TRACE_ENTITIES + TRACE_STRIDE;
const TRACE_OUTPUT: i32 = 256;
const TRACE_CONTENTS: i32 = 0x0200_0000; // Q3 temporary boxes carry CONTENTS_BODY.

fn trace_store(code: &mut Code, address: i32, value: i32) {
    code.extend([(Const, address), (Const, value), (Store4, 0)]);
}

fn trace_export(host: &mut FrameHost, callback: u32) -> i32 {
    host.call_module_export(
        ModuleId(1),
        Export::words(CallbackId(callback), &[]),
        ThinkTime::Milliseconds(0),
    )
    .unwrap();
    let Some(ModuleResult::Qvm(word)) = host.module_counts(ModuleId(1)).unwrap().last_result else {
        unreachable!()
    };
    word
}

fn qvm_foreign_trace_fixture() -> (FrameHost, qa_core::primitives::EntityId) {
    use qa_core::primitives::{Bounds, NativeEntity, Vec3};
    use qa_world::collision::{brushes::BrushTree, surfaces::SurfaceTable};
    let mut runtime = Runtime::load(1, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree::direct(0).unwrap(),
            vec![Bounds {
                mins: Vec3([-128.0; 3]),
                maxs: Vec3([128.0; 3]),
            }],
        )
        .unwrap();
    let mut world = visibility_world::collision(
        &runtime.geometry,
        [Some(0), Some(1)],
        qa_world::visibility::PvsRows::load(vec![Some(0), Some(1)], vec![3, 3], None).unwrap(),
    );
    world.geometry = geometry;
    runtime.collision = Some(world);

    let mut data = Data(vec![0; 16384]);
    for slot in 0..2 {
        let row = (TRACE_ENTITIES + slot * TRACE_STRIDE) as usize;
        for (offset, word) in [(0, slot), (460, TRACE_CONTENTS), (512, 1023)] {
            data.0[row + offset..row + offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        for (offset, vector) in [
            (436, [-1.0f32; 3]),
            (448, [1.0f32; 3]),
            (488, [if slot == 0 { 100.0 } else { 20.0 }, 0.0, 0.0]),
        ] {
            for (axis, number) in vector.into_iter().enumerate() {
                let at = row + offset + axis * 4;
                data.0[at..at + 4].copy_from_slice(&number.to_le_bytes());
            }
        }
    }
    let text = data.text(b"qa_live_foreign_trace");
    let mut setup = Code::new();
    syscall(
        &mut setup,
        15,
        &[TRACE_ENTITIES, 2, TRACE_STRIDE, 8192, 468],
    );
    syscall(&mut setup, 30, &[TRACE_ENTITIES]);
    syscall(&mut setup, 30, &[TRACE_ACTOR]);
    setup.extend([(Const, 0), (Leave, 64)]);
    let mut moved = Code::new();
    trace_store(&mut moved, TRACE_ACTOR + 488, 24.0f32.to_bits() as i32);
    syscall(&mut moved, 14, &[0, text]);
    moved.extend([(Const, 7), (Leave, 64)]);
    let mut disabled = Code::new();
    trace_store(&mut disabled, TRACE_ACTOR + 460, 0);
    disabled.extend([(Const, 0), (Leave, 64)]);
    let mut enabled = Code::new();
    trace_store(&mut enabled, TRACE_ACTOR + 460, TRACE_CONTENTS);
    enabled.extend([(Const, 0), (Leave, 64)]);
    let mut malformed = Code::new();
    trace_store(&mut malformed, TRACE_ACTOR + 436, f32::NAN.to_bits() as i32);
    syscall(&mut malformed, 14, &[0, text]);
    trace_store(
        &mut malformed,
        TRACE_ACTOR + 436,
        (-1.0f32).to_bits() as i32,
    );
    malformed.extend([(Const, 42), (Leave, 64)]);
    let mut lazy_model = Code::new();
    for (offset, word) in [(432, 1), (160, i32::MAX), (460, 0)] {
        trace_store(&mut lazy_model, TRACE_ACTOR + offset, word);
    }
    syscall(&mut lazy_model, 14, &[0, text]);
    for (offset, word) in [(432, 0), (160, 0), (460, TRACE_CONTENTS)] {
        trace_store(&mut lazy_model, TRACE_ACTOR + offset, word);
    }
    lazy_model.extend([(Const, 13), (Leave, 64)]);
    let read_link_count = vec![(Const, TRACE_ACTOR + 420), (Load4, 0), (Leave, 64)];
    let mut producer = vm(
        &[
            (9, vec![(Const, 0), (Leave, 64)]), // Own nested handler declines before B runs.
            (10, setup),
            (11, moved),
            (12, disabled),
            (13, enabled),
            (14, malformed),
            (15, lazy_model),
            (16, read_link_count),
            (
                17,
                vec![(Const, TRACE_ACTOR + 488), (Load4, 0), (Leave, 64)],
            ),
        ],
        data,
    );
    producer.configure_q3_game(qa_core::primitives::Q3StatLayout::BASE);
    let mut producer = request(&mut runtime, 1, &Q3_SERVER, SeatId::FIRST, producer);
    producer.entries.extend(11..=17);

    let mut query = Data(vec![0; 384]);
    query.0[352..356].copy_from_slice(&40.0f32.to_le_bytes());
    let mut trace = Code::new();
    syscall(
        &mut trace,
        24,
        &[TRACE_OUTPUT, 336, 0, 0, 352, 1023, TRACE_CONTENTS],
    );
    trace.extend([(Const, TRACE_OUTPUT + 12), (Load4, 0), (Leave, 64)]);
    let consumer = request(
        &mut runtime,
        2,
        &Q3_SERVER,
        SeatId::FIRST,
        vm(&[(9, trace)], query),
    );
    let mut console = Console::new(Context::default()).unwrap();
    let clients = console
        .cvars
        .bind(
            "sv_maxclients",
            Context {
                source: RuleSetId::Quake3,
                ..Context::default()
            },
        )
        .unwrap();
    console.cvars.force_write(clients, "1").unwrap();
    let mut host = FrameHost::load_modules(
        console,
        runtime,
        TickRate::FrameDriven,
        vec![producer, consumer],
    )
    .unwrap();
    assert_eq!(trace_export(&mut host, 1), 0);
    let actor = host
        .runtime
        .server
        .entities
        .find_native(
            NativeEntity {
                module: ModuleId(1),
                slot: 1,
            },
            None,
        )
        .unwrap();
    (host, actor)
}

fn qvm_foreign_probe(host: &mut FrameHost) -> f32 {
    host.console
        .append("qa_live_foreign_trace\n", Context::default())
        .unwrap();
    host.execute_commands();
    let Some(ModuleResult::Qvm(bits)) = host.module_counts(ModuleId(2)).unwrap().last_result else {
        unreachable!()
    };
    f32::from_bits(bits as u32)
}

#[test]
fn qvm_exec_now_foreign_trace_reads_live_pose_after_same_owner_handler_returns() {
    use qa_core::primitives::{Bounds, Vec3};
    use qa_world::area::LinkFlags;
    let (mut host, actor) = qvm_foreign_trace_fixture();
    let linked = host.runtime.server.area.bounds(actor).unwrap();
    let bounds = Bounds {
        mins: Vec3([-128.0; 3]),
        maxs: Vec3([128.0; 3]),
    };
    let order = |host: &FrameHost| {
        host.runtime
            .server
            .area
            .query(&host.runtime.server.entities, bounds, LinkFlags::SOLID)
            .map(|linked| linked.id)
            .collect::<Vec<_>>()
    };
    let before = order(&host);
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .body(actor.slot as usize)
            .position
            .0[0],
        20.0
    );
    assert_eq!(trace_export(&mut host, 2), 7);
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::Qvm(22.875f32.to_bits() as i32))
    );
    assert_eq!(
        host.runtime
            .server
            .entities
            .columns
            .body(actor.slot as usize)
            .position
            .0[0],
        24.0
    );
    assert_eq!(host.runtime.server.area.bounds(actor), Some(linked));
    assert_eq!(order(&host), before);
    assert_eq!(trace_export(&mut host, 7), 1); // Explicit LinkEntity count never changes.
    assert_eq!(trace_export(&mut host, 8), 24.0f32.to_bits() as i32);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
}

#[test]
fn qvm_foreign_trace_reads_returned_collision_fields_and_keeps_filtered_models_lazy() {
    let (mut host, actor) = qvm_foreign_trace_fixture();
    let linked = host.runtime.server.area.bounds(actor);
    assert_eq!(qvm_foreign_probe(&mut host), 18.875);
    trace_export(&mut host, 3); // Normal outer publication does not relink collision contents.
    assert_eq!(
        host.runtime.server.entities.columns.collision_contents[actor.slot as usize],
        TRACE_CONTENTS as u64
    );
    assert_eq!(qvm_foreign_probe(&mut host), 40.0);
    trace_export(&mut host, 4);
    assert_eq!(qvm_foreign_probe(&mut host), 18.875);
    assert_eq!(trace_export(&mut host, 6), 13); // Invalid model is behind a contents-mask rejection.
    assert_eq!(
        host.module_counts(ModuleId(2)).unwrap().last_result,
        Some(ModuleResult::Qvm(40.0f32.to_bits() as i32))
    );
    assert_eq!(host.runtime.server.area.bounds(actor), linked);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    assert_eq!(qvm_foreign_probe(&mut host), 18.875);
}

#[test]
fn malformed_foreign_qvm_trace_rejects_the_consumer_and_preserves_producer_recovery() {
    let (mut host, actor) = qvm_foreign_trace_fixture();
    let linked = host.runtime.server.area.bounds(actor);
    assert_eq!(trace_export(&mut host, 5), 42);
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 1);
    assert_eq!(
        host.module_state(ModuleId(1)),
        Some(qa_app::modules::State::Running)
    );
    assert_eq!(host.runtime.server.area.bounds(actor), linked);
    assert_eq!(qvm_foreign_probe(&mut host), 18.875);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 1);
}

#[test]
fn queued_module_exec_and_nested_scripts_keep_the_publication_time_source() {
    struct Files(std::path::PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = qa_platform::files::remove_dir_all(&self.0);
        }
    }
    let files = Files(std::env::temp_dir().join(format!("qa-scoped-exec-{}", std::process::id())));
    let mut runtime = Runtime::load(1, []).unwrap();
    let mut searches = Vec::new();
    for (name, priority, nested) in [
        ("low", 0, b"set _qa_script_source 1\n".as_slice()),
        ("high", 10, b"set _qa_script_source 2\n"),
    ] {
        let root = files.0.join(name);
        qa_platform::files::create_dir_all(&root).unwrap();
        qa_platform::files::write(root.join("native.cfg"), b"exec nested\n").unwrap();
        qa_platform::files::write(root.join("nested.cfg"), nested).unwrap();
        let mount = runtime.vfs.mount_directory(&root, priority).unwrap();
        searches.push(runtime.vfs.search_view([mount]).unwrap());
    }
    let requests = searches
        .into_iter()
        .enumerate()
        .map(|(index, search)| {
            let mut data = Data::default();
            let command = data.text(b"exec native\n");
            let mut body = Code::new();
            syscall(&mut body, 14, &[2, command]);
            body.extend([(Const, 0), (Leave, 64)]);
            let mut request = request(
                &mut runtime,
                index as u16 + 1,
                &Q3_SERVER,
                SeatId::FIRST,
                vm(&[(10, body)], data),
            );
            request.search = Some(search);
            request
        })
        .collect();
    let mut host = host(runtime, requests);
    invoke(&mut host, 1);
    assert_eq!(value(&host, "_qa_script_source"), None);
    host.execute_commands();
    assert_eq!(value(&host, "_qa_script_source"), Some(1.0));
    invoke(&mut host, 2);
    host.execute_commands();
    assert_eq!(value(&host, "_qa_script_source"), Some(2.0));
    assert_eq!(host.module_counts(ModuleId(1)).unwrap().traps, 0);
    assert_eq!(host.module_counts(ModuleId(2)).unwrap().traps, 0);
    let foreign = qa_content::vfs::Vfs::default();
    let foreign_view = foreign.search_view([]).unwrap();
    let id = foreign.bind_search(Some(&foreign_view)).unwrap().id();
    host.console
        .append(
            "exec native\n",
            Context {
                file_search: id,
                ..Context::default()
            },
        )
        .unwrap();
    host.execute_commands();
    assert_eq!(value(&host, "_qa_script_source"), Some(2.0));
}
