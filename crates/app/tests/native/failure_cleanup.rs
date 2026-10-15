use super::*;

fn program(encoding: Encoding, code: &[u8]) -> Vm {
    let name = if encoding == Encoding::Elf {
        b"vmMain".as_slice()
    } else {
        b"GetGameAPI".as_slice()
    };
    Vm::map_image(
        function_image(encoding, code),
        &[NamedExport {
            name,
            command: None,
            parameters: &[qa_platform::native::NativeScalar::Word; 13],
            result: qa_platform::native::NativeScalar::Word,
        }],
        &[],
        Duration::from_secs(3),
    )
    .unwrap()
}

pub(super) fn run() {
    use qa_session::clients::Connection;
    for (encoding, closing) in [Encoding::Elf, Encoding::Pe]
        .into_iter()
        .flat_map(|encoding| [false, true].map(|closing| (encoding, closing)))
    {
        let mut runtime = Runtime::load(2, []).unwrap();
        let failed = ModuleId(7);
        let healthy = ModuleId(8);
        let clients = [failed, healthy].map(|module| {
            runtime
                .server
                .connect(
                    Connection::Remote,
                    module,
                    Some(qa_core::primitives::NativeEntity { module, slot: 0 }),
                )
                .unwrap()
        });
        let lifetimes = clients.map(|id| runtime.server.clients[id.0 as usize].entity);
        let bad = request(
            &mut runtime,
            failed,
            RuleSetId::Quake3,
            TickRate::FrameDriven,
            program(encoding, &[0x0f, 0x0b]),
        );
        let good = request(
            &mut runtime,
            healthy,
            RuleSetId::Quake3,
            TickRate::FrameDriven,
            program(encoding, &[0xb8, 77, 0, 0, 0, 0xc3]),
        );
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![bad, good],
        )
        .unwrap();
        host.initialize_modules(Phase::Server);
        if closing {
            assert!(host.runtime.server.begin_close(
                clients[0],
                lifetimes[0],
                EventTime::from_milliseconds(1),
            ));
        }
        assert!(
            host.call_module_export(
                failed,
                Export::words(CallbackId(0), &[]),
                ThinkTime::Milliseconds(10),
            )
            .is_err()
        );
        assert_eq!(
            host.module_state(failed),
            Some(qa_app::modules::State::Failed)
        );
        assert!(
            host.runtime.server.clients[clients[0].0 as usize]
                .connection
                .is_none(),
            "failed native owner retains a connected client: {encoding:?}, closing={closing}"
        );
        assert!(host.runtime.server.entities.resolve(lifetimes[0]).is_none());
        assert!(
            host.runtime
                .server
                .entities
                .resolve(host.runtime.server.clients[clients[0].0 as usize].entity)
                .is_some()
        );
        assert!(host.runtime.server.entities.resolve(lifetimes[1]).is_some());
        assert_eq!(
            host.runtime.server.clients[clients[1].0 as usize].connection,
            Some(Connection::Remote)
        );
        host.call_module_export(
            healthy,
            Export::words(CallbackId(0), &[]),
            ThinkTime::Milliseconds(11),
        )
        .unwrap();
        assert_eq!(host.module_counts(failed).unwrap().calls, 1);
        assert_eq!(host.module_counts(failed).unwrap().traps, 1);
        assert_eq!(host.module_counts(healthy).unwrap().traps, 0);
        assert_eq!(
            host.module_counts(healthy).unwrap().last_result,
            Some(ModuleResult::Native(77))
        );
    }
    println!("failed native owner connection cleanup: ELF and PE passed");
}
