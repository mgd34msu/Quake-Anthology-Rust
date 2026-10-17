//! The ordinary session services handle file lists from parked ELF/PE children.
use super::*;

pub(super) fn run() {
    let files =
        Files(std::env::temp_dir().join(format!("qa-native-file-listing-{}", std::process::id())));
    qa_platform::files::create_dir_all(files.0.join("maps")).unwrap();
    for name in ["b.bsp", "a.bsp"] {
        qa_platform::files::write(files.0.join("maps").join(name), b"loaded").unwrap();
    }
    for encoding in [Encoding::Elf, Encoding::Pe] {
        for (table, ordinal) in [(&Q3_SERVER, 38), (&qa_compat::abi::Q3_UI, 17)] {
            // Forward vmMain's five words to the numbered import callback.
            let mut code = vec![0x48, 0x83, 0xec, 56];
            code.extend([0x48, 0x8b, 0x05]);
            code.extend((0x700i32 - (code.len() as i32 + 4)).to_le_bytes());
            if encoding == Encoding::Pe {
                // Windows' fifth argument follows the caller's shadow space.
                code.extend([0x4c, 0x8b, 0x54, 0x24, 96]);
                code.extend([0x4c, 0x89, 0x54, 0x24, 32]);
            }
            code.extend([0xff, 0xd0]);
            code.extend([0x80, 0x3d]);
            code.extend((0x740i32 - (code.len() as i32 + 5)).to_le_bytes());
            code.extend([b'a', 0x74, 2, 0x0f, 0x0b]);
            code.extend([0x48, 0x83, 0xc4, 56, 0xc3]);
            assert!(code.len() <= 64);
            let image = function_image(encoding, &code);
            let name = if encoding == Encoding::Elf {
                b"vmMain".as_slice()
            } else {
                b"GetGameAPI".as_slice()
            };
            let entry = image.symbol(name).unwrap().address;
            let offset = (entry - image.base) as usize;
            let mut vm = Vm::map_image(
                image,
                &[
                    NamedExport {
                        name,
                        command: None,
                        parameters: &[NativeScalar::Word; 5],
                        result: NativeScalar::Word,
                    },
                    NamedExport {
                        name: b"dllEntry",
                        command: None,
                        parameters: &[NativeScalar::Word],
                        result: NativeScalar::Void,
                    },
                ],
                &[],
                Duration::from_secs(3),
            )
            .unwrap();
            let callback = vm.import_callback();
            let memory = vm.process.memory_mut().unwrap();
            memory[offset + 0x720..offset + 0x72a].copy_from_slice(b"maps\0.bsp\0");
            memory[offset + 0x740..offset + 0x780].fill(0xa5);
            let mut runtime = Runtime::load(1, []).unwrap();
            runtime.vfs.mount_directory(&files.0, 0).unwrap();
            let mut request = request(
                &mut runtime,
                ModuleId(1),
                RuleSetId::Quake3,
                TickRate::fixed(50).unwrap(),
                vm,
            );
            let Program::Native { imports, .. } = &mut request.program else {
                unreachable!()
            };
            *imports = table;
            request.entries = vec![0, 1];
            request.prepare = vec![Export::words(CallbackId(1), &[callback])];
            request.frame = Export::words(
                CallbackId(0),
                &[ordinal, entry + 0x720, entry + 0x725, entry + 0x740, 64],
            );
            let mut host = FrameHost::load_modules(
                Console::new(Context::default()).unwrap(),
                runtime,
                TickRate::FrameDriven,
                vec![request],
            )
            .unwrap();
            let mut source = Source {
                time: EventTime(0),
                polls: 0,
            };
            host.frame(&mut source, true);
            source.time = EventTime(50_000_000);
            host.frame(&mut source, true);
            let counts = host.module_counts(ModuleId(1)).unwrap();
            assert_eq!(counts.traps, 0, "{encoding:?} ordinal {ordinal}");
            assert_eq!(counts.last_result, Some(ModuleResult::Native(2)));
            host.shutdown_modules();
        }
    }
    println!("native file listing passed for ELF/PE and game/UI imports");
}
