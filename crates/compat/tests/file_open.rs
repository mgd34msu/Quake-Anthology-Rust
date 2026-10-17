use qa_compat::{
    abi::{Addresses, CallTable, Invocation, Q3_CLIENT, Q3_SERVER, Q3_UI, QvmCalls, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, EngineServices, ServiceStorage},
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    names::NameTable,
    primitives::{ModuleId, RuleSetId, ThinkTime},
    sys_events::EventTime,
};
use qa_formats::program::qvm::{Image, Opcode};
use qa_platform::native::NativeAbi;
use qa_world::{
    collision::{CollisionStore, TraceScratch},
    entities::AllocationPolicy,
};

struct Fixture {
    root: std::path::PathBuf,
    server: qa_session::clients::Server,
    input: qa_input::Input,
    cvars: Cvars,
    commands: CommandBuffer,
    vfs: qa_content::vfs::Vfs,
    localization: qa_content::localization::Catalog,
    registry: qa_gameplay::registry::Registry,
    names: NameTable,
    storage: ServiceStorage,
    geometry: CollisionStore,
    scratch: TraceScratch,
}
impl Fixture {
    fn new(test: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("qa-compat-file-open-{test}-{}", std::process::id()));
        qa_platform::files::create_dir_all(&root).unwrap();
        qa_platform::files::write(root.join("empty.dat"), b"").unwrap();
        qa_platform::files::write(root.join("data.dat"), b"abc").unwrap();
        let mut vfs = qa_content::vfs::Vfs::default();
        vfs.mount_directory(&root, 0).unwrap();
        let names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
        let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
        let cvars = Cvars::new().unwrap();
        let storage =
            ServiceStorage::load_native(&[(ModuleId(1), 0, RuleSetId::Quake3)], 1, &cvars).unwrap();
        let geometry = CollisionStore::new();
        let scratch = geometry.scratch();
        Self {
            root,
            server: qa_session::clients::Server::load(1, 4, 1, 1, 1, 0).unwrap(),
            input: qa_input::Input::load(),
            cvars,
            commands: CommandBuffer::new(),
            vfs,
            localization: Default::default(),
            registry,
            names,
            storage,
            geometry,
            scratch,
        }
    }
    fn engine(&mut self) -> EngineServices<'_> {
        EngineServices {
            server: &mut self.server,
            callbacks: None,
            client_disconnect: None,
            input: &mut self.input,
            editor: None,
            cvars: &mut self.cvars,
            commands: &mut self.commands,
            client_connection: None,
            client_collision: None,
            client_audio: None,
            renderer: None,
            scene: None,
            vfs: &self.vfs,
            model_loader: None,
            model_lookup: None,
            sound_loader: None,
            sound_bank: None,
            localization: &self.localization,
            registry: &self.registry,
            names: &mut self.names,
            storage: &mut self.storage,
            geometry: &self.geometry,
            world: None,
            native_geometry: None,
            foreign_trace: None,
            visibility: None,
            portals: None,
            scratch: &mut self.scratch,
        }
    }
    fn context() -> CallContext {
        CallContext {
            module: ModuleId(1),
            clock: ThinkTime::Milliseconds(9000),
            server_frame: 0,
            native_rules: RuleSetId::Quake3,
            trace_rules: RuleSetId::QuakeWorld,
            sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake2),
            console: Context::default(),
            allocation: AllocationPolicy::EDICT,
            link_order: qa_gameplay::rules::link_order(RuleSetId::Quake),
        }
    }
    fn call(
        &mut self,
        table: &CallTable,
        mode: Addresses,
        memory: &mut ModuleMemory<'_>,
        number: u32,
        arguments: &[u64],
    ) -> Result<u64, CallError> {
        let mut unknown = UnknownCalls::load(2).unwrap();
        let result = table.invoke(
            number,
            &mut Invocation {
                services: &mut self.engine(),
                memory,
                native_cvars: None,
                native_resources: None,
                native_entities: None,
                native_surfaces: None,
                native_command: None,
                native_configs: None,
                native_pmove: None,
                native_player: None,
                context: Self::context(),
                platform_time: EventTime(42),
                command: Default::default(),
                argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
                addresses: mode,
                arguments,
            },
            &mut unknown,
        );
        assert_eq!(unknown.calls, 0);
        result.map(|step| match step {
            qa_compat::abi::ImportStep::Return(value) => value,
            qa_compat::abi::ImportStep::Host { .. } => panic!("unexpected host request"),
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.vfs = Default::default();
        qa_platform::files::remove_dir_all(&self.root).unwrap();
    }
}

fn modes() -> [(Addresses, u64); 4] {
    [
        (Addresses::Qvm { mask: 511 }, 0),
        (
            Addresses::Native {
                abi: NativeAbi::SystemV,
                pointer_bytes: 4,
            },
            0x2000_0000,
        ),
        (
            Addresses::Native {
                abi: NativeAbi::SystemV,
                pointer_bytes: 8,
            },
            1 << 40,
        ),
        (
            Addresses::Native {
                abi: NativeAbi::Microsoft,
                pointer_bytes: 8,
            },
            1 << 40,
        ),
    ]
}
fn paths(memory: &mut ModuleMemory<'_>, base: u64) {
    for (offset, path) in [
        (128, b"empty.dat\0".as_slice()),
        (160, b"data.dat\0"),
        (192, b"missing.dat\0"),
    ] {
        memory.write(base + offset, path).unwrap();
    }
}

#[test]
fn null_output_queries_existence_without_writing_memory_or_consuming_handles() {
    let mut f = Fixture::new("address-modes");
    for (mode, base) in modes() {
        for (table, open, close) in [(&Q3_SERVER, 10, 13), (&Q3_CLIENT, 10, 13), (&Q3_UI, 13, 16)] {
            let mut bytes = [0xa5; 512];
            let mut memory = ModuleMemory::borrow(base, &mut bytes).unwrap();
            paths(&mut memory, base);
            let before = memory.read(base, 512).unwrap().to_vec();
            // Original files.c returns qtrue even for a zero-byte file.
            for (offset, expected) in [(128, 1), (160, 1), (192, 0)] {
                assert_eq!(
                    f.call(table, mode, &mut memory, open, &[base + offset, 0, 0]),
                    Ok(expected)
                );
                assert_eq!(memory.read(base, 512).unwrap(), before);
            }
            // The ordinary output is still a four-byte handle and the return
            // remains file length, including zero and native -1 on failure.
            for (offset, length, handle) in [(128, 0, 1), (160, 3, 1), (192, u32::MAX as u64, 0)] {
                memory.write(base + 256, &[0xa5; 8]).unwrap();
                assert_eq!(
                    f.call(
                        table,
                        mode,
                        &mut memory,
                        open,
                        &[base + offset, base + 256, 0]
                    ),
                    Ok(length)
                );
                assert_eq!(memory.read_word(base + 256).unwrap() as u32, handle);
                assert_eq!(memory.read(base + 260, 4).unwrap(), &[0xa5; 4]);
                if handle != 0 {
                    // One file slot is already occupied: the existence path
                    // must still succeed without attempting another open.
                    let occupied = memory.read(base, 512).unwrap().to_vec();
                    assert_eq!(
                        f.call(table, mode, &mut memory, open, &[base + 160, 0, 0]),
                        Ok(1)
                    );
                    assert_eq!(memory.read(base, 512).unwrap(), occupied);
                    f.call(table, mode, &mut memory, close, &[u64::from(handle)])
                        .unwrap();
                }
            }
        }
    }
}

#[test]
fn nonzero_qvm_output_wrapping_to_zero_still_opens_and_writes_the_handle() {
    let mut f = Fixture::new("masked-output");
    let mut bytes = [0xa5; 512];
    let mut memory = ModuleMemory::borrow(0, &mut bytes).unwrap();
    paths(&mut memory, 0);
    let mode = Addresses::Qvm { mask: 511 };
    assert_eq!(
        f.call(&Q3_SERVER, mode, &mut memory, 10, &[160, 512, 0]),
        Ok(3)
    );
    assert_eq!(memory.read_word(0).unwrap(), 1);
    assert_eq!(memory.read(4, 124).unwrap(), &[0xa5; 124]);
    f.call(&Q3_SERVER, mode, &mut memory, 13, &[1]).unwrap();
}

#[test]
fn executed_qvm_observes_boolean_existence_and_preserves_guest_data() {
    use Opcode::*;
    let mut f = Fixture::new("executed-qvm");
    let mut operations = vec![(Enter, 64)];
    for (path, result) in [(128, 256), (160, 260), (192, 264)] {
        operations.push((Const, result));
        for (index, argument) in [path, 0, 0].into_iter().enumerate() {
            operations.extend([(Const, argument), (Arg, 8 + index as i32 * 4)]);
        }
        operations.extend([(Const, -11), (Call, 0), (Store4, 0)]);
    }
    operations.extend([(Const, 73), (Leave, 64)]);
    let mut code = Vec::new();
    for &(op, argument) in &operations {
        code.push(op as u8);
        match op.operand_bytes() {
            1 => code.push(argument as u8),
            4 => code.extend(argument.to_le_bytes()),
            _ => {}
        }
    }
    let mut data = [0xa5; 512];
    paths(&mut ModuleMemory::borrow(0, &mut data).unwrap(), 0);
    let original = data;
    let mut bytes = Vec::new();
    for word in [
        0x12721444,
        operations.len() as i32,
        32,
        code.len() as i32,
        32 + code.len() as i32,
        512,
        0,
        65536 - 512,
    ] {
        bytes.extend(word.to_le_bytes());
    }
    bytes.extend(code);
    bytes.extend(data);
    let mut vm = qa_compat::qvm::Vm::load(Image::parse(&bytes).unwrap()).unwrap();
    let mut unknown = UnknownCalls::load(2).unwrap();
    {
        let mut engine = f.engine();
        let mut calls = QvmCalls {
            services: &mut engine,
            table: &Q3_SERVER,
            context: Fixture::context(),
            platform_time: EventTime(42),
            command: Default::default(),
            argument_selection: &mut qa_compat::abi::ArgumentSelection::RetainedClient,
            unknown: &mut unknown,
        };
        assert_eq!(vm.call(&mut calls, [0; 10], 1000, false), Ok(73));
    }
    assert_eq!(unknown.calls, 0);
    for (at, expected) in [(256, 1), (260, 1), (264, 0)] {
        assert_eq!(vm.memory.read_word(at).unwrap(), expected);
    }
    assert_eq!(vm.memory.read(0, 256).unwrap(), &original[..256]);
    assert_eq!(vm.memory.read(268, 244).unwrap(), &original[268..]);
}

#[test]
fn resetting_one_instance_closes_its_files_and_restarts_only_its_service_cursors() {
    use qa_compat::services::ENGINE_CALLS;
    use qa_core::primitives::{NativeDelivery, NativeTarget, Vec3};
    let mut fixture = Fixture::new("instance-reset");
    let first = ModuleId(1);
    let peer = ModuleId(2);
    fixture.storage = ServiceStorage::load_modules(
        &[
            (first, 0, RuleSetId::Quake3, 1),
            (peer, 0, RuleSetId::Quake3, 1),
        ],
        &fixture.cvars,
    )
    .unwrap();
    for module in [first, peer] {
        fixture
            .storage
            .bind_entity_text(
                module,
                std::sync::Arc::from(b"alpha beta".as_slice()),
                qa_formats::entities::EntitySyntax::Quake3,
            )
            .unwrap();
    }
    let mut engine = fixture.engine();
    let first_handle = engine.file_open(first, b"data.dat").unwrap().0;
    let peer_handle = engine.file_open(peer, b"data.dat").unwrap().0;
    assert_eq!(first_handle, peer_handle);
    let mut byte = [0];
    for module in [first, peer] {
        assert_eq!(engine.file_read(module, 1, &mut byte), Ok(1));
        assert_eq!(byte, *b"a");
        assert_eq!(
            (ENGINE_CALLS.entity_token)(&mut engine, module).unwrap(),
            Some(b"alpha".as_slice())
        );
        (ENGINE_CALLS.message_bits)(&mut engine, module, 0xff, 8).unwrap();
    }
    engine.storage.reset_instance(first).unwrap();
    assert_eq!(
        engine.file_read(first, first_handle, &mut byte),
        Err(CallError::File)
    );
    assert_eq!(engine.file_read(peer, peer_handle, &mut byte), Ok(1));
    assert_eq!(byte, *b"b");
    assert_eq!(
        (ENGINE_CALLS.entity_token)(&mut engine, first).unwrap(),
        Some(b"alpha".as_slice())
    );
    assert_eq!(
        (ENGINE_CALLS.entity_token)(&mut engine, peer).unwrap(),
        Some(b"beta".as_slice())
    );
    let delivery = |module| NativeDelivery {
        module,
        source: RuleSetId::Quake3,
        target: NativeTarget::All,
        origin: Vec3::default(),
        reliable: true,
        dupe_key: 0,
        component: 0,
        emission: None,
        check_portals: false,
    };
    assert_eq!(
        (ENGINE_CALLS.native_message)(&mut engine, delivery(first), Default::default()),
        Ok(None)
    );
    assert!(
        (ENGINE_CALLS.native_message)(&mut engine, delivery(peer), Default::default())
            .unwrap()
            .is_some()
    );
    assert_eq!(
        engine.file_open(first, b"data.dat").unwrap().0,
        first_handle
    );
    assert_eq!(engine.file_read(first, first_handle, &mut byte), Ok(1));
    assert_eq!(byte, *b"a");
    assert_eq!(engine.file_read(peer, peer_handle, &mut byte), Ok(1));
    assert_eq!(byte, *b"c");
}

#[test]
fn directory_refresh_keeps_open_module_files_on_their_accepted_source() {
    let mut fixture = Fixture::new("directory-refresh");
    let owner = ModuleId(1);
    fixture.storage =
        ServiceStorage::load_native(&[(owner, 0, RuleSetId::Quake3)], 2, &fixture.cvars).unwrap();
    let mount = fixture.vfs.mounts().next().unwrap().id;
    let (old, length) = fixture.engine().file_open(owner, b"data.dat").unwrap();
    assert_eq!(length, 3);
    let mut byte = [0];
    assert_eq!(fixture.engine().file_read(owner, old, &mut byte), Ok(1));
    assert_eq!(byte, *b"a");

    qa_platform::files::write(fixture.root.join("replacement"), b"replacement").unwrap();
    qa_platform::files::rename(
        fixture.root.join("replacement"),
        fixture.root.join("data.dat"),
    )
    .unwrap();
    fixture.vfs.refresh_directory_prefix(mount, b"").unwrap();
    let (new, length) = fixture.engine().file_open(owner, b"data.dat").unwrap();
    assert_eq!(length, 11);
    assert_ne!(old, new);

    let mut bytes = [0; 16];
    assert_eq!(fixture.engine().file_read(owner, old, &mut bytes), Ok(2));
    assert_eq!(&bytes[..2], b"bc");
    assert_eq!(fixture.engine().file_read(owner, old, &mut bytes), Ok(0));
    assert_eq!(
        fixture
            .engine()
            .file_seek(owner, old, std::io::SeekFrom::End(-1)),
        Ok(true)
    );
    assert_eq!(fixture.engine().file_read(owner, old, &mut byte), Ok(1));
    assert_eq!(byte, *b"c");
    assert_eq!(fixture.engine().file_read(owner, new, &mut bytes), Ok(11));
    assert_eq!(&bytes[..11], b"replacement");
}

#[test]
fn module_search_changes_scope_open_and_probe_without_changing_peer_handles() {
    let mut f = Fixture::new("module-search");
    let first = ModuleId(1);
    let peer = ModuleId(2);
    f.storage = ServiceStorage::load_modules(
        &[
            (first, 0, RuleSetId::Quake3, 2),
            (peer, 0, RuleSetId::Quake2, 2),
        ],
        &f.cvars,
    )
    .unwrap();
    let original = f.vfs.mounts().next().unwrap().id;
    let later = f.root.join("later");
    qa_platform::files::create_dir_all(&later).unwrap();
    qa_platform::files::write(later.join("data.dat"), b"later bytes").unwrap();
    qa_platform::files::write(later.join("only.dat"), b"private").unwrap();
    let mount = f.vfs.mount_directory(&later, 10).unwrap();
    let peer_handle = f.engine().file_open(peer, b"data.dat").unwrap().0;
    let previous = f.engine().file_open(first, b"data.dat").unwrap().0;
    f.storage
        .set_file_search(first, Some(f.vfs.search_view([original]).unwrap()), &f.vfs)
        .unwrap();
    assert_eq!(
        f.engine().file_read(first, previous, &mut [0; 8]),
        Err(CallError::File)
    );
    assert_eq!(f.engine().file_length(first, b"data.dat"), Ok(3));
    assert_eq!(f.engine().file_length(peer, b"data.dat"), Ok(11));
    assert_eq!(
        f.engine().file_length(first, b"only.dat"),
        Err(CallError::File)
    );
    let (handle, length) = f.engine().file_open(first, b"data.dat").unwrap();
    assert_eq!(length, 3);
    let mut bytes = [0; 32];
    let count = f.engine().file_read(first, handle, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], b"abc");
    let count = f.engine().file_read(peer, peer_handle, &mut bytes).unwrap();
    assert_eq!(&bytes[..count], b"later bytes");
    // Native FS_FOpenFile's null-handle probe and ordinary open share the
    // module owner for every existing Q3 mapping and address width.
    for (mode, base) in modes() {
        for (table, open) in [(&Q3_SERVER, 10), (&Q3_CLIENT, 10), (&Q3_UI, 13)] {
            let mut data = [0; 512];
            let mut memory = ModuleMemory::borrow(base, &mut data).unwrap();
            memory.write(base + 160, b"only.dat\0").unwrap();
            assert_eq!(
                f.call(table, mode, &mut memory, open, &[base + 160, 0, 0]),
                Ok(0)
            );
            assert_eq!(
                f.call(table, mode, &mut memory, open, &[base + 160, base + 256, 0]),
                Ok(u64::from(u32::MAX))
            );
            assert_eq!(memory.read_word(base + 256), Ok(0));
        }
    }
    f.storage
        .set_file_search(first, Some(f.vfs.search_view([mount]).unwrap()), &f.vfs)
        .unwrap();
    assert_eq!(f.engine().file_length(first, b"only.dat"), Ok(7));
    let foreign = qa_content::vfs::Vfs::default().search_view([]).unwrap();
    assert_eq!(
        f.storage.set_file_search(first, Some(foreign), &f.vfs),
        Err(CallError::File)
    );
    assert_eq!(f.engine().file_length(first, b"only.dat"), Ok(7));
    f.storage.set_file_search(first, None, &f.vfs).unwrap();
    assert_eq!(f.engine().file_length(first, b"data.dat"), Ok(11));
}
