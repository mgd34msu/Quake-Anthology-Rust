//! Scoped generic native reads keep the existing decoder and memory authority.
use super::{ENTITIES, Fixture, NATIVE, PLAYER_STRIDE, PLAYERS, context, entity, player, qvm};
use qa_compat::services::CallError;
use qa_core::primitives::{Body, CollisionOwner, CollisionShape, RuleSetId, Vec3};
use qa_world::collision::{CollisionStore, NativeTraceEntities, NativeTraceTarget};

fn origin(
    view: Option<&dyn NativeTraceEntities>,
    store: &CollisionStore,
) -> Result<Vec3, CallError> {
    let view = view.ok_or(CallError::Entity)?;
    let target = NativeTraceTarget {
        body: Body::default(),
        angles: Vec3::default(),
        shape: CollisionShape::None,
        rules: qa_gameplay::rules::model_rules(RuleSetId::Quake3),
        contents_pose: None,
    };
    Ok(view
        .target(NATIVE, target, false, store, None)
        .ok_or(CallError::Geometry)?
        .body
        .position)
}

pub fn run() {
    use qa_formats::program::qvm::Opcode;

    let mut fixture = Fixture::new();
    let mut vm = qvm(
        &[(Opcode::Enter, 16), (Opcode::Const, 0), (Opcode::Leave, 16)],
        &fixture.registry,
    );
    assert_eq!(
        vm.with_trace_view(context(), None, |view| Ok(view.is_none())),
        Ok(true)
    );
    entity(&mut vm.memory, ENTITIES);
    player(&mut vm.memory, PLAYERS);
    fixture.register(&mut vm, ENTITIES, PLAYERS, PLAYER_STRIDE);
    vm.memory
        .write_vec3(ENTITIES + 488, Vec3([321.0, 4.0, 5.0]))
        .unwrap();
    assert_eq!(
        vm.with_trace_view(context(), None, |view| origin(view, &fixture.geometry)),
        Ok(Vec3([321.0, 4.0, 5.0]))
    );
    vm.memory.write_word(ENTITIES + 512, -1).unwrap();
    let rejected: Result<(), _> = vm.with_trace_view(context(), None, |view| {
        let view = view.unwrap();
        assert_eq!(view.owner(NATIVE), None);
        assert!(view.failed());
        Err(CallError::Unsupported)
    });
    assert_eq!(rejected, Err(CallError::Entity));
    vm.memory.write_word(ENTITIES + 512, 1023).unwrap();
    assert_eq!(
        vm.with_trace_view(context(), None, |view| Ok(view.unwrap().owner(NATIVE))),
        Ok(Some(CollisionOwner::None))
    );
    assert_eq!(
        vm.with_trace_view(context(), None, |_| Err::<(), _>(CallError::Unsupported)),
        Err(CallError::Unsupported)
    );

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    owned_pending_reads();
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn owned_pending_reads() {
    use qa_compat::{
        abi::{HostRequest, Q3_SERVER, UnknownCalls},
        native::{Error, NativeCalls, PendingImport, Step},
    };
    use qa_core::{primitives::Q3StatLayout, sys_events::EventTime};

    fn with_calls<T>(
        fixture: &mut Fixture,
        unknown: &mut UnknownCalls,
        read: impl FnOnce(&mut NativeCalls<'_, '_>) -> T,
    ) -> T {
        read(&mut NativeCalls {
            services: &mut fixture.engine(),
            table: &Q3_SERVER,
            context: context(),
            platform_time: EventTime(0),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown,
            native_player: None,
        })
    }
    fn mov_rax(code: &mut Vec<u8>, value: u64) {
        code.extend([0x48, 0xb8]);
        code.extend(value.to_le_bytes());
    }
    fn store(code: &mut Vec<u8>, address: u64, value: u32) {
        mov_rax(code, address);
        code.extend([0xc7, 0x00]);
        code.extend(value.to_le_bytes());
    }
    fn returned(code: &mut Vec<u8>, value: u32) {
        code.extend([0x48, 0x83, 0xc4, 56, 0xb8]);
        code.extend(value.to_le_bytes());
        code.push(0xc3);
    }

    let mut vm = super::native_continuation::mapped_image();
    let mut fixture = Fixture::new();
    let mut unknown = UnknownCalls::load(1).unwrap();
    vm.configure_q3_game(Q3StatLayout::BASE);
    vm.bind_player_values(
        super::value_bindings(&super::player_bindings()),
        &fixture.registry,
    )
    .unwrap();
    let base = vm.process.base();
    let entities = base + 0x1800;
    let players = base + 0x2400;
    // Existing fixture exports select this one function with commands 0/1/2.
    let mut code = vec![
        0x48, 0x83, 0xec, 56, 0x83, 0xf9, 1, 0x0f, 0x84, 0, 0, 0, 0, 0x83, 0xf9, 2, 0x0f, 0x84, 0,
        0, 0, 0,
    ];
    store(&mut code, entities + 488, 321.0f32.to_bits());
    store(&mut code, entities + 512, u32::MAX);
    code.extend([0xb9, 14, 0, 0, 0, 0x31, 0xd2, 0x49, 0xb8]);
    code.extend((base + 0x1b00).to_le_bytes());
    mov_rax(&mut code, vm.import_callback());
    code.extend([0xff, 0xd0]);
    returned(&mut code, 77);
    let nested_at = code.len();
    store(&mut code, entities + 488, 456.0f32.to_bits());
    store(&mut code, entities + 512, 1023);
    returned(&mut code, 11);
    let locate_at = code.len();
    code.extend([0xb9, 15, 0, 0, 0, 0x48, 0xba]);
    code.extend(entities.to_le_bytes());
    code.extend([0x41, 0xb8, 1, 0, 0, 0, 0x41, 0xb9]);
    code.extend((super::ENTITY_STRIDE as u32).to_le_bytes());
    mov_rax(&mut code, players);
    code.extend([0x48, 0x89, 0x44, 0x24, 32, 0x48, 0xc7, 0x44, 0x24, 40]);
    code.extend((PLAYER_STRIDE as u32).to_le_bytes());
    mov_rax(&mut code, vm.import_callback());
    code.extend([0xff, 0xd0]);
    returned(&mut code, 0);
    code[9..13].copy_from_slice(&((nested_at as i32) - 13).to_le_bytes());
    code[18..22].copy_from_slice(&((locate_at as i32) - 22).to_le_bytes());
    {
        let mut memory = qa_compat::memory::process_memory(&mut vm.process).unwrap();
        entity(&mut memory, entities);
        player(&mut memory, players);
        memory.write(base + 0x1200, &code).unwrap();
        memory.write_string(base + 0x1b00, 32, b"probe").unwrap();
    }
    assert!(
        vm.with_trace_view(context(), None, None, |view| Ok(view.is_none()))
            .unwrap()
    );
    assert_eq!(
        with_calls(&mut fixture, &mut unknown, |calls| vm.call(calls, 2, &[])).unwrap(),
        0
    );
    assert_eq!(
        vm.with_trace_view(context(), None, None, |view| origin(
            view,
            &fixture.geometry
        ))
        .unwrap(),
        Vec3([-900.0, 2.0, 3.0])
    );
    let before =
        fixture.server.entities.columns.position[fixture.server.clients[1].entity.slot as usize];
    let mut outer =
        with_calls(&mut fixture, &mut unknown, |calls| vm.begin(calls, 0, &[])).unwrap();
    let Step::Host {
        request: HostRequest::ExecuteNow {
            text: Some(pointer),
            ..
        },
        pending,
    } = with_calls(&mut fixture, &mut unknown, |calls| {
        vm.advance(calls, &mut outer)
    })
    .unwrap()
    else {
        panic!("native EXEC_NOW stop")
    };
    assert_eq!(pointer, base + 0x1b00);
    let mut pending: Option<PendingImport> = Some(pending);
    assert!(vm.process.memory().is_err());
    assert!(matches!(
        vm.with_trace_view(context(), None, None, |_| Ok(())),
        Err(Error::Export)
    ));
    assert_eq!(
        vm.with_trace_view(
            context(),
            None,
            Some((&outer, pending.as_ref().unwrap())),
            |view| origin(view, &fixture.geometry)
        )
        .unwrap(),
        Vec3([321.0, 2.0, 3.0])
    );
    let pid = vm.process.pid();
    let rejected = vm.with_trace_view(
        context(),
        None,
        Some((&outer, pending.as_ref().unwrap())),
        |view| {
            let view = view.unwrap();
            assert_eq!(view.owner(NATIVE), None);
            assert!(view.failed());
            Err::<(), _>(CallError::Unsupported)
        },
    );
    assert!(matches!(rejected, Err(Error::Service(CallError::Entity))));
    assert_eq!(vm.process.pid(), pid);
    assert_ne!(pid, 0);
    assert_eq!(
        fixture.server.entities.columns.position[fixture.server.clients[1].entity.slot as usize],
        before
    );
    let mut nested =
        with_calls(&mut fixture, &mut unknown, |calls| vm.begin(calls, 1, &[])).unwrap();
    assert!(matches!(
        vm.with_trace_view(
            context(),
            None,
            Some((&outer, pending.as_ref().unwrap())),
            |_| Ok(())
        ),
        Err(Error::Export)
    ));
    assert!(matches!(
        with_calls(&mut fixture, &mut unknown, |calls| vm
            .advance(calls, &mut nested))
        .unwrap(),
        Step::Returned(11)
    ));
    assert_eq!(
        vm.with_trace_view(
            context(),
            None,
            Some((&outer, pending.as_ref().unwrap())),
            |view| origin(view, &fixture.geometry)
        )
        .unwrap(),
        Vec3([456.0, 2.0, 3.0])
    );
    assert_eq!(
        vm.with_trace_view(
            context(),
            None,
            Some((&outer, pending.as_ref().unwrap())),
            |view| Ok(view.unwrap().owner(NATIVE))
        )
        .unwrap(),
        Some(CollisionOwner::None)
    );
    assert_eq!(
        fixture.server.entities.columns.position[fixture.server.clients[1].entity.slot as usize],
        before
    );
    assert!(matches!(
        with_calls(&mut fixture, &mut unknown, |calls| vm.resume(
            calls,
            &mut outer,
            &mut pending,
            0
        ))
        .unwrap(),
        Step::Returned(77)
    ));
    assert!(pending.is_none());
    assert_eq!(
        vm.with_trace_view(context(), None, None, |view| origin(
            view,
            &fixture.geometry
        ))
        .unwrap(),
        Vec3([456.0, 2.0, 3.0])
    );
    assert_eq!(unknown.calls, 0);
}
