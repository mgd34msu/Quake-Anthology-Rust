//! Real child imports yield all service/byte loans before host reentry.
use super::{Fixture, context, native_image};
use qa_compat::{
    abi::{HostRequest, Q3_UI, UnknownCalls},
    native::{Error, NamedExport, NativeCalls, PendingImport, Step, Vm},
    services::{CallContext, CallError},
};
use qa_core::{
    primitives::ThinkTime,
    sys_events::{EventTime, SeatId},
};
use qa_platform::native::NativeScalar;

fn with_calls<R>(
    fixture: &mut Fixture,
    unknown: &mut UnknownCalls,
    context: CallContext,
    time: EventTime,
    operation: impl FnOnce(&mut NativeCalls<'_, '_>) -> R,
) -> R {
    let mut services = fixture.engine();
    operation(&mut NativeCalls {
        services: &mut services,
        table: &Q3_UI,
        context,
        platform_time: time,
        command: qa_console::command_text::Arguments::from_command(
            qa_core::primitives::CommandArgs {
                argv: &[b"original", b"payload"],
                raw: b"payload",
            },
        ),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown,
        native_player: None,
    })
}
fn mov_rax(code: &mut Vec<u8>, address: u64) {
    code.extend([0x48, 0xb8]);
    code.extend(address.to_le_bytes());
}
fn syscall(code: &mut Vec<u8>, callback: u64, number: u32) {
    code.push(0xb9);
    code.extend(number.to_le_bytes());
    mov_rax(code, callback);
    code.extend([0xff, 0xd0]);
}
fn body(code: &mut Vec<u8>, callback: u64, destination: u64, counter: u64, result: u32) {
    code.extend([0x48, 0x81, 0xec, 136, 0, 0, 0]);
    syscall(code, callback, 39); // UI ClearStates: must suspend before returning.
    mov_rax(code, counter);
    code.extend([0xff, 0x00]);
    syscall(code, callback, 2);
    code.extend([0x48, 0xba]);
    code.extend(destination.to_le_bytes());
    code.extend([0x89, 0x02]);
    syscall(code, callback, 10); // argc, still supplied by the outer owner.
    code.extend([0x48, 0xba]);
    code.extend((destination + 4).to_le_bytes());
    code.extend([0x89, 0x02]);
    code.extend([0x48, 0x81, 0xc4, 136, 0, 0, 0, 0xb8]);
    code.extend(result.to_le_bytes());
    code.push(0xc3);
}
fn mapped_image() -> Vm {
    let mut file = native_image::pe_export_fixture(64);
    native_image::put(&mut file, 208, 12288, 4);
    native_image::put(&mut file, 392 + 8, 8192, 4);
    native_image::put(&mut file, 392 + 16, 8192, 4);
    native_image::pe_rva(&mut file, 0x1140, 0x1200, 4);
    file.resize(8704, 0);
    native_image::pe_text(&mut file, 0x1080, &[0xb8, 1, 0, 0, 0, 0xc3]);
    native_image::pe_text(&mut file, 0x1200, &[0xc3]);
    let named = [0, 1, 2].map(|command| NamedExport {
        name: b"GetGameAPI",
        command: Some(command),
        parameters: &[NativeScalar::I32; 10],
        result: NativeScalar::Word,
    });
    Vm::map_image(
        qa_formats::program::native::Image::parse(
            &file,
            None,
            qa_formats::program::native::LoadRole::Library,
        )
        .unwrap(),
        &named,
        &[],
        std::time::Duration::from_secs(3),
    )
    .unwrap()
}
fn image() -> Vm {
    let mut vm = mapped_image();
    let base = vm.process.base();
    let mut code = vec![
        0x83, 0xf9, 2, 0x0f, 0x84, 0, 0, 0, 0, 0x83, 0xf9, 1, 0x0f, 0x84, 0, 0, 0, 0,
    ];
    body(
        &mut code,
        vm.import_callback(),
        base + 0x1800,
        base + 0x1810,
        77,
    );
    let nested = code.len();
    body(
        &mut code,
        vm.import_callback(),
        base + 0x1808,
        base + 0x1810,
        11,
    );
    let healthy = code.len();
    code.extend([0xb8, 99, 0, 0, 0, 0xc3]);
    code[5..9].copy_from_slice(&((healthy as i32) - 9).to_le_bytes());
    code[14..18].copy_from_slice(&((nested as i32) - 18).to_le_bytes());
    qa_compat::memory::process_memory(&mut vm.process)
        .unwrap()
        .write(base + 0x1200, &code)
        .unwrap();
    vm
}
fn host(step: Step, seat: SeatId, time: EventTime) -> PendingImport {
    let Step::Host { request, pending } = step else {
        panic!("host stop");
    };
    let HostRequest::ClearKeys {
        seat: actual,
        context,
        time: actual_time,
    } = request
    else {
        panic!("UI ClearStates");
    };
    assert_eq!(actual, seat);
    assert_eq!(context.seat, seat);
    assert_eq!(actual_time, time);
    pending
}

pub fn run() {
    let mut vm = image();
    let mut external = image();
    let mut fixture = Fixture::new();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let outer_context = context();
    let outer_time = EventTime::from_milliseconds(1234);
    let mut outer = with_calls(
        &mut fixture,
        &mut unknown,
        outer_context,
        outer_time,
        |calls| vm.begin(calls, 0, &[]),
    )
    .unwrap();
    let step = with_calls(
        &mut fixture,
        &mut unknown,
        outer_context,
        outer_time,
        |calls| vm.advance(calls, &mut outer),
    )
    .unwrap();
    let mut outer_pending = Some(host(step, outer_context.console.seat, outer_time));
    assert!(vm.is_active());
    assert_eq!(
        vm.activation_capacity(),
        qa_platform::native::NATIVE_REENTRY_LIMIT + 1
    );
    assert!(vm.process.memory().is_err());
    // No service or A-mapping borrow survives this independent module entry.
    assert_eq!(
        with_calls(
            &mut fixture,
            &mut unknown,
            outer_context,
            outer_time,
            |calls| external.call(calls, 2, &[])
        )
        .unwrap(),
        99
    );
    assert!(matches!(
        with_calls(
            &mut fixture,
            &mut unknown,
            outer_context,
            outer_time,
            |calls| vm.begin(calls, 99, &[])
        ),
        Err(Error::Export)
    ));
    let mut nested_context = outer_context;
    nested_context.clock = ThinkTime::Milliseconds(5678);
    nested_context.console.seat = SeatId::ALL[1];
    let nested_time = EventTime::from_milliseconds(5678);
    let mut nested = with_calls(
        &mut fixture,
        &mut unknown,
        nested_context,
        nested_time,
        |calls| vm.begin(calls, 1, &[]),
    )
    .unwrap();
    assert!(matches!(
        with_calls(
            &mut fixture,
            &mut unknown,
            outer_context,
            outer_time,
            |calls| vm.advance(calls, &mut outer)
        ),
        Err(Error::Export)
    ));
    assert!(matches!(
        with_calls(
            &mut fixture,
            &mut unknown,
            outer_context,
            outer_time,
            |calls| vm.resume(calls, &mut outer, &mut outer_pending, 0)
        ),
        Err(Error::Export)
    ));
    assert!(outer_pending.is_some());
    let step = with_calls(
        &mut fixture,
        &mut unknown,
        nested_context,
        nested_time,
        |calls| vm.advance(calls, &mut nested),
    )
    .unwrap();
    let mut nested_pending = Some(host(step, nested_context.console.seat, nested_time));
    let mut poisoned = outer_context;
    poisoned.clock = ThinkTime::Milliseconds(9999);
    poisoned.console.seat = SeatId::ALL[3];
    let step = with_calls(
        &mut fixture,
        &mut unknown,
        poisoned,
        EventTime::from_milliseconds(9999),
        |calls| vm.resume(calls, &mut nested, &mut nested_pending, 0),
    )
    .unwrap();
    assert!(matches!(step, Step::Returned(11)));
    assert!(vm.is_active());
    let step = with_calls(
        &mut fixture,
        &mut unknown,
        poisoned,
        EventTime::from_milliseconds(9999),
        |calls| vm.resume(calls, &mut outer, &mut outer_pending, 11),
    )
    .unwrap();
    assert!(matches!(step, Step::Returned(77)));
    assert!(!vm.is_active());
    let base = vm.process.base();
    let memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
    for (offset, expected) in [(0, 1234), (4, 2), (8, 5678), (12, 2), (16, 2)] {
        assert_eq!(memory.read_word(base + 0x1800 + offset).unwrap(), expected);
    }
    drop(memory);
    assert_eq!(unknown.calls, 0);
    let mut canceled = with_calls(
        &mut fixture,
        &mut unknown,
        outer_context,
        outer_time,
        |calls| vm.begin(calls, 0, &[]),
    )
    .unwrap();
    let step = with_calls(
        &mut fixture,
        &mut unknown,
        outer_context,
        outer_time,
        |calls| vm.advance(calls, &mut canceled),
    )
    .unwrap();
    let _pending = host(step, outer_context.console.seat, outer_time);
    assert!(matches!(
        with_calls(
            &mut fixture,
            &mut unknown,
            outer_context,
            outer_time,
            |calls| vm.abort(calls, &mut canceled, CallError::Aborted)
        ),
        Error::Service(CallError::Aborted)
    ));
    assert!(!vm.is_active());
    assert_eq!(vm.process.pid(), 0);
    assert_eq!(
        with_calls(
            &mut fixture,
            &mut unknown,
            outer_context,
            outer_time,
            |calls| external.call(calls, 2, &[])
        )
        .unwrap(),
        99
    );
    execute_now_command_copy();
    foreign_runs();
    nested_reject_zero_unwinds_its_suspended_parent();
}

fn foreign_runs() {
    let mut first = image();
    let mut second = image();
    let mut fixture = Fixture::new();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let context = context();
    let time = EventTime::from_milliseconds(1234);
    let mut begin = |vm: &mut Vm| {
        with_calls(&mut fixture, &mut unknown, context, time, |calls| {
            let mut run = vm.begin(calls, 0, &[]).unwrap();
            let step = vm.advance(calls, &mut run).unwrap();
            (run, Some(host(step, context.console.seat, time)))
        })
    };
    let (mut first_run, mut first_pending) = begin(&mut first);
    let (mut second_run, mut second_pending) = begin(&mut second);
    let first_pid = first.process.pid();
    let second_pid = second.process.pid();
    with_calls(&mut fixture, &mut unknown, context, time, |calls| {
        assert!(matches!(
            second.advance(calls, &mut first_run),
            Err(Error::Export)
        ));
        assert!(matches!(
            second.abort(calls, &mut first_run, CallError::Aborted),
            Error::Export
        ));
        assert!(matches!(
            second.resume(calls, &mut first_run, &mut first_pending, 0),
            Err(Error::Export)
        ));
        assert!(matches!(
            second.resume(calls, &mut second_run, &mut first_pending, 0),
            Err(Error::Export)
        ));
        assert!(matches!(
            first.resume(calls, &mut first_run, &mut second_pending, 0),
            Err(Error::Export)
        ));
        assert!(first_pending.is_some());
        assert!(second_pending.is_some());
        assert!(first.is_active());
        assert!(second.is_active());
        assert_eq!(first.process.pid(), first_pid);
        assert_eq!(second.process.pid(), second_pid);
        assert!(matches!(
            first.resume(calls, &mut first_run, &mut first_pending, 0),
            Ok(Step::Returned(77))
        ));
        assert!(matches!(
            second.resume(calls, &mut second_run, &mut second_pending, 0),
            Ok(Step::Returned(77))
        ));
    });
    assert!(first_pending.is_none());
    assert!(second_pending.is_none());
    assert!(!first.is_active());
    assert!(!second.is_active());
    assert_eq!(unknown.calls, 0);
}

fn nested_reject_zero_unwinds_its_suspended_parent() {
    use qa_compat::{abi::Q3_SERVER, services::ENGINE_CALLS};
    use qa_core::primitives::{CallbackId, NativeEntity, RuleSetId};
    use qa_session::dispatch::{FunctionBinding, FunctionTable};

    let mut vm = mapped_image();
    let attach = vm.initializers().last().unwrap().ordinal;
    let base = vm.process.base();
    let reason = base + 0x1800;
    let mut code = vec![0x48, 0x81, 0xec, 136, 0, 0, 0];
    code.extend([0xba, 1, 0, 0, 0, 0x49, 0xb8]); // FREE native client one.
    code.extend(reason.to_le_bytes());
    syscall(&mut code, vm.import_callback(), 16); // GAME_DROP_CLIENT.
    code.extend([0x48, 0x81, 0xc4, 136, 0, 0, 0, 0xb8, 77, 0, 0, 0, 0xc3]);
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        memory.write(base + 0x1200, &code).unwrap();
        // PE DLL_PROCESS_ATTACH is a public export whose zero result is rejected
        // after the child has returned to its parent's suspended import.
        memory.write(base + 0x1080, &[0x31, 0xc0, 0xc3]).unwrap();
        memory.write(reason, b"reason\0").unwrap();
    }
    let mut fixture = Fixture::new();
    let context = context();
    let lifetime = fixture
        .server
        .entities
        .allocate(
            context.clock,
            context.module,
            qa_world::entities::AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    fixture
        .storage
        .bind_lifetime(context.module, lifetime)
        .unwrap();
    let resolver = FunctionTable::<qa_session::clients::Server>::load([(
        context.module,
        RuleSetId::Quake3,
        vec![FunctionBinding {
            entry: attach,
            call: |_, _, _, _| panic!("native adapter owns the resolved callback"),
        }],
    )])
    .unwrap();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let mut services = fixture.engine();
    services.callbacks = Some(&resolver);
    services.client_disconnect = Some(CallbackId(0));
    let mut calls = NativeCalls {
        services: &mut services,
        table: &Q3_SERVER,
        context,
        platform_time: EventTime::from_milliseconds(5_000),
        command: Default::default(),
        argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
        unknown: &mut unknown,
        native_player: None,
    };
    assert!(matches!(vm.call(&mut calls, 0, &[]), Err(Error::DllAttach)));
    assert!(!vm.is_active());
    assert_eq!(vm.process.pid(), 0);
    let free = NativeEntity {
        module: context.module,
        slot: 1,
    };
    assert!(
        (ENGINE_CALLS.drop_client_target)(
            calls.services,
            free,
            EventTime::from_milliseconds(6_000)
        )
        .unwrap()
        .is_none()
    );
    assert!(
        (ENGINE_CALLS.drop_client_target)(
            calls.services,
            free,
            EventTime::from_milliseconds(7_001)
        )
        .unwrap()
        .is_some()
    );
    assert_eq!(unknown.calls, 0);
}

fn execute_now_image() -> Vm {
    let mut vm = mapped_image();
    let base = vm.process.base();
    let mut code = vec![0x48, 0x81, 0xec, 136, 0, 0, 0, 0x31, 0xd2, 0x49, 0xb8];
    code.extend((base + 0x1900).to_le_bytes());
    syscall(&mut code, vm.import_callback(), 12);
    code.extend([0x48, 0x81, 0xc4, 136, 0, 0, 0, 0xb8, 42, 0, 0, 0, 0xc3]);
    let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
    memory.write(base + 0x1200, &code).unwrap();
    memory
        .write_string(base + 0x1900, 64, b"echo \xff; tail")
        .unwrap();
    drop(memory);
    vm
}
fn execute_now_command_copy() {
    let mut vm = execute_now_image();
    let mut external = image();
    let mut fixture = Fixture::new();
    let mut unknown = UnknownCalls::load(1).unwrap();
    let context = context();
    let time = EventTime(42);
    let mut run = with_calls(&mut fixture, &mut unknown, context, time, |calls| {
        vm.begin(calls, 0, &[])
    })
    .unwrap();
    let Step::Host {
        request:
            HostRequest::ExecuteNow {
                context: found,
                text: Some(pointer),
            },
        pending,
    } = with_calls(&mut fixture, &mut unknown, context, time, |calls| {
        vm.advance(calls, &mut run)
    })
    .unwrap()
    else {
        panic!("EXEC_NOW host stop");
    };
    assert_eq!(found, context.console);
    assert_eq!(pointer, vm.process.base() + 0x1900);
    let mut command = qa_console::command_text::ParsedCommand::default();
    command.set_raw(b"kept").unwrap();
    assert!(matches!(
        external.copy_import_command(&run, &pending, pointer, &mut command),
        Err(Error::Export)
    ));
    assert_eq!(command.raw_bytes(), b"kept");
    assert!(
        vm.copy_import_command(&run, &pending, pointer, &mut command)
            .unwrap()
    );
    assert_eq!(command.raw_bytes(), b"echo \xff; tail");
    assert!(command.arguments().is_empty());
    assert!(
        !vm.copy_import_command(&run, &pending, vm.process.base() + 0x1980, &mut command)
            .unwrap()
    );
    assert!(command.raw_bytes().is_empty());
    assert!(
        vm.copy_import_command(&run, &pending, pointer, &mut command)
            .unwrap()
    );
    assert!(vm.process.memory().is_err());
    let mut external_run = with_calls(&mut fixture, &mut unknown, context, time, |calls| {
        external.begin(calls, 0, &[])
    })
    .unwrap();
    let external_pending = host(
        with_calls(&mut fixture, &mut unknown, context, time, |calls| {
            external.advance(calls, &mut external_run)
        })
        .unwrap(),
        context.console.seat,
        time,
    );
    assert!(matches!(
        vm.copy_import_command(&run, &external_pending, pointer, &mut command),
        Err(Error::Export)
    ));
    assert_eq!(command.raw_bytes(), b"echo \xff; tail");
    let mut pending = Some(pending);
    assert!(matches!(
        with_calls(&mut fixture, &mut unknown, context, time, |calls| vm
            .resume(calls, &mut run, &mut pending, 0))
        .unwrap(),
        Step::Returned(42)
    ));
    assert_eq!(command.raw_bytes(), b"echo \xff; tail");
    assert!(fixture.commands.is_empty());
    let mut standalone = execute_now_image();
    assert!(matches!(
        with_calls(&mut fixture, &mut unknown, context, time, |calls| {
            standalone.call(calls, 0, &[])
        }),
        Err(Error::Service(CallError::Unsupported))
    ));
    assert!(fixture.commands.is_empty());
    with_calls(&mut fixture, &mut unknown, context, time, |calls| {
        external.abort(calls, &mut external_run, CallError::Aborted)
    });
    assert_eq!(unknown.calls, 0);
}
