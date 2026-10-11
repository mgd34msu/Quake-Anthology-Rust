use qa_app::Runtime;
use qa_compat::{
    abi::{Addresses, Invocation, Q3_CLIENT, Q3_SERVER, Q3_UI, UnknownCalls},
    memory::ModuleMemory,
    services::{CallContext, CallError, ServiceStorage},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{ModuleId, RuleSetId, ThinkTime},
    sys_events::EventTime,
};
use qa_platform::native::NativeAbi;
use qa_world::{area::LinkOrder, entities::AllocationPolicy};

struct Files(std::path::PathBuf);
impl Files {
    fn load(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("qa-module-seek-{name}-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("module.txt"), b"native file bytes").unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn q3_file_imports_seek_shared_handles_with_native_long_widths_and_eof() {
    let files = Files::load("abi");
    for (base, addresses, wide) in [
        (0, Addresses::Qvm { mask: 511 }, false),
        (
            1u64 << 40,
            Addresses::Native {
                abi: NativeAbi::Microsoft,
            },
            false,
        ),
        (
            1u64 << 40,
            Addresses::Native {
                abi: NativeAbi::SystemV,
            },
            true,
        ),
    ] {
        for (table, open, read, seek, close) in [
            (&Q3_SERVER, 10, 11, 45, 13),
            (&Q3_CLIENT, 10, 11, 89, 13),
            (&Q3_UI, 13, 14, 86, 16),
        ] {
            let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
            runtime.vfs.mount_directory(&files.0, 0).unwrap();
            let mut console = Console::new(Context::default()).unwrap();
            let mut storage = ServiceStorage::load(&[], 2, &console.cvars).unwrap();
            let mut scratch = runtime.geometry.scratch();
            let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
            let mut memory = ModuleMemory::load(base, 512, &[]).unwrap();
            memory.write_string(base + 16, 32, b"module.txt").unwrap();
            let mut unknown = UnknownCalls::load(1).unwrap();
            let mut context = CallContext {
                module: ModuleId(1),
                clock: ThinkTime::Milliseconds(0),
                server_frame: 0,
                console: Context {
                    source: RuleSetId::Quake3,
                    ..Context::default()
                },
                allocation: AllocationPolicy::EDICT,
                link_order: LinkOrder::Head,
            };
            macro_rules! invoke {
                ($ordinal:expr, $arguments:expr) => {{
                    let arguments = $arguments;
                    let mut call = Invocation {
                        services: &mut services,
                        memory: &mut memory,
                        native_cvars: None,
                        native_resources: None,
                        native_entities: None,
                        native_surfaces: None,
                        native_command: None,
                        native_configs: None,
                        context,
                        platform_time: EventTime(0),
                        command: &[],
                        addresses,
                        arguments,
                    };
                    table.invoke($ordinal, &mut call, &mut unknown)
                }};
            }
            assert_eq!(invoke!(open, &[base + 16, 0, 0]), Ok(17));
            if matches!(addresses, Addresses::Qvm { .. }) {
                assert_eq!(invoke!(open, &[base + 16, 1u64 << 32, 0]), Ok(17));
                assert_eq!(invoke!(open, &[base + 16, 512, 0]), Ok(17));
                let wrapped = memory.read_word(0).unwrap() as u64;
                assert_ne!(wrapped, 0);
                assert_eq!(invoke!(close, &[wrapped]), Ok(0));
            }
            assert_eq!(invoke!(open, &[base + 16, base + 64, 0]), Ok(17));
            let handle = memory.read_word(base + 64).unwrap() as u64;
            assert_eq!(invoke!(read, &[base + 96, 6, handle]), Ok(0));
            assert_eq!(invoke!(seek, &[handle, 7, 2]), Ok(0));
            assert_eq!(invoke!(read, &[base + 112, 4, handle]), Ok(0));
            assert_eq!(invoke!(seek, &[handle, (-6i64) as u64, 0]), Ok(0));
            assert_eq!(invoke!(read, &[base + 128, 2, handle]), Ok(0));
            assert_eq!(invoke!(seek, &[handle, (-5i64) as u64, 1]), Ok(0));
            assert_eq!(invoke!(read, &[base + 144, 5, handle]), Ok(0));
            assert_eq!(
                invoke!(seek, &[handle, (-18i64) as u64, 1]),
                Ok(u64::from(u32::MAX))
            );
            assert_eq!(
                invoke!(seek, &[handle, (-1i64) as u64, 2]),
                Ok(u64::from(u32::MAX))
            );
            assert_eq!(invoke!(seek, &[handle, 0, 3]), Err(CallError::File));
            memory.write(base + 152, b"!").unwrap();
            assert_eq!(invoke!(read, &[base + 152, 1, handle]), Ok(0));
            assert_eq!(memory.read(base + 152, 1).unwrap(), b"!");
            assert_eq!(invoke!(seek, &[handle, 1u64 << 32, 2]), Ok(0));
            assert_eq!(invoke!(read, &[base + 160, 1, handle]), Ok(0));
            assert_eq!(memory.read(base + 96, 6).unwrap(), b"native");
            assert_eq!(memory.read(base + 112, 4).unwrap(), b"file");
            assert_eq!(memory.read(base + 128, 2).unwrap(), b"e ");
            assert_eq!(memory.read(base + 144, 5).unwrap(), b"bytes");
            assert_eq!(
                memory.read(base + 160, 1).unwrap(),
                if wide { b"\0" } else { b"n" }
            );
            context.module = ModuleId(2);
            assert_eq!(invoke!(seek, &[handle, 0, 2]), Err(CallError::File));
            context.module = ModuleId(1);
            assert_eq!(invoke!(close, &[handle]), Ok(0));
            assert_eq!(invoke!(seek, &[handle, 0, 2]), Err(CallError::File));
            assert_eq!(unknown.calls, 0);
        }
    }
}

#[test]
fn shared_seek_reuses_pak_and_deflate_readers_across_backward_and_large_offsets() {
    use qa_compat::services::ENGINE_CALLS;
    use std::io::SeekFrom;
    let files = Files::load("archives");
    std::fs::write(
        files.0.join("pak0.pak"),
        include_bytes!("fixtures/module-files/seek.pak"),
    )
    .unwrap();
    std::fs::write(
        files.0.join("pak1.pk3"),
        include_bytes!("fixtures/module-files/seek.pk3"),
    )
    .unwrap();
    let mut runtime = Runtime::load(1, std::iter::empty()).unwrap();
    runtime.vfs.mount_product(&files.0, 0).unwrap();
    let mut console = Console::new(Context::default()).unwrap();
    let mut storage = ServiceStorage::load(&[], 2, &console.cvars).unwrap();
    let mut scratch = runtime.geometry.scratch();
    let mut services = runtime.engine_services(&mut console, &mut storage, &mut scratch);
    let owner = ModuleId(1);
    let (pak, pak_length) = (ENGINE_CALLS.file_open)(&mut services, owner, b"module.txt").unwrap();
    let (zip, zip_length) = (ENGINE_CALLS.file_open)(&mut services, owner, b"long.txt").unwrap();
    assert_eq!((pak_length, zip_length), (17, 70000));
    let mut out = [0; 6];
    assert!((ENGINE_CALLS.file_seek)(&mut services, owner, zip, SeekFrom::Start(65535)).unwrap());
    assert_eq!(
        (ENGINE_CALLS.file_read)(&mut services, owner, zip, &mut out).unwrap(),
        6
    );
    assert_eq!(&out, b"567890");
    assert!((ENGINE_CALLS.file_seek)(&mut services, owner, pak, SeekFrom::End(-5)).unwrap());
    assert_eq!(
        (ENGINE_CALLS.file_read)(&mut services, owner, pak, &mut out).unwrap(),
        5
    );
    assert_eq!(&out[..5], b"bytes");
    assert!((ENGINE_CALLS.file_seek)(&mut services, owner, zip, SeekFrom::Start(0)).unwrap());
    assert_eq!(
        (ENGINE_CALLS.file_read)(&mut services, owner, zip, &mut out).unwrap(),
        6
    );
    assert_eq!(&out, b"012345");
    assert!((ENGINE_CALLS.file_seek)(&mut services, owner, zip, SeekFrom::End(5)).unwrap());
    assert_eq!(
        (ENGINE_CALLS.file_read)(&mut services, owner, zip, &mut out).unwrap(),
        0
    );
    assert!((ENGINE_CALLS.file_seek)(&mut services, owner, zip, SeekFrom::Current(-10)).unwrap());
    assert_eq!(
        (ENGINE_CALLS.file_read)(&mut services, owner, zip, &mut out).unwrap(),
        5
    );
    assert_eq!(&out[..5], b"56789");
    assert!(
        !(ENGINE_CALLS.file_seek)(&mut services, owner, zip, SeekFrom::Current(i64::MIN)).unwrap()
    );
    assert_eq!(
        (ENGINE_CALLS.file_read)(&mut services, owner, zip, &mut out).unwrap(),
        0
    );
}
