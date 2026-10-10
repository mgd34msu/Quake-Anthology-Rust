use super::{
    NativeAbi, NativeError, NativeImage, NativeImport, NativeRegion, NativeScalar,
    implementation::{NativeProcess, child_main, executable_offset, transfer},
};
use std::{os::unix::process::ExitStatusExt, process::Command, time::Duration};

const BASE: u64 = 0x2000_0000;
const REGIONS: [NativeRegion; 2] = [
    NativeRegion {
        offset: 0,
        length: 4096,
        permissions: 5,
    },
    NativeRegion {
        offset: 4096,
        length: 4096,
        permissions: 3,
    },
];

#[test]
fn a_partial_packet_cannot_restart_its_absolute_deadline() {
    use std::{io::Write, os::unix::net::UnixStream, time::Instant};
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        if writer.write_all(&[1]).is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
        let _ = writer.write_all(&[2; 199]);
    });
    let mut bytes = [0; 200];
    let result = transfer(
        &mut reader,
        &mut bytes,
        false,
        Some(Instant::now() + Duration::from_millis(100)),
    );
    assert!(matches!(result, Err(NativeError::Timeout)), "{result:?}");
    assert_eq!(bytes[0], 1);
    drop(reader);
    writer.join().unwrap();
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    let mut bytes = [3; 200];
    transfer(
        &mut writer,
        &mut bytes,
        true,
        Some(Instant::now() + Duration::from_secs(1)),
    )
    .unwrap();
    let mut actual = [0; 200];
    transfer(
        &mut reader,
        &mut actual,
        false,
        Some(Instant::now() + Duration::from_secs(1)),
    )
    .unwrap();
    assert_eq!(actual, bytes);
}

#[test]
fn child_entry() {
    if std::env::var_os("QA_NATIVE_TEST_CHILD").is_some() {
        std::process::exit(if child_main().is_ok() { 0 } else { 125 });
    }
}

fn child(code: &[u8], timeout: Duration) -> Result<NativeProcess, NativeError> {
    let mut bytes = vec![0; 8192];
    bytes[..code.len()].copy_from_slice(code);
    image_child(&bytes, &REGIONS, &[], timeout)
}

fn image_child(
    bytes: &[u8],
    regions: &[NativeRegion],
    imports: &[NativeImport<'_>],
    timeout: Duration,
) -> Result<NativeProcess, NativeError> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(["--exact", "native::tests::child_entry", "--nocapture"])
        .env("QA_NATIVE_TEST_CHILD", "1");
    NativeProcess::launch(
        command,
        NativeImage {
            imports,
            base: BASE,
            pointer_bytes: 8,
            bytes,
            regions,
            timeout,
        },
    )
}

#[test]
fn byte_ranges_share_page_rights_without_expanding_callable_entries() {
    let mut bytes = vec![0; 5000];
    // mov [rdi],rsi; mov rax,rsi; ret
    let code = [0x48, 0x89, 0x37, 0x48, 0x89, 0xf0, 0xc3];
    bytes[512..512 + code.len()].copy_from_slice(&code);
    let regions = [
        NativeRegion {
            offset: 0,
            length: 512,
            permissions: 1,
        },
        NativeRegion {
            offset: 512,
            length: code.len(),
            permissions: 5,
        },
        NativeRegion {
            offset: 1000,
            length: 24,
            permissions: 3,
        },
        NativeRegion {
            offset: 4096,
            length: 904,
            permissions: 1,
        },
    ];
    let mut process = image_child(&bytes, &regions, &[], Duration::from_secs(3)).unwrap();
    assert!(process.executable(BASE + 512));
    for offset in [0, 511, 519, 1000, 4096, 5000, 8191] {
        assert!(!process.executable(BASE + offset));
    }
    assert_eq!(
        process.memory().unwrap().len(),
        8192 + 4096 + 8 * 1024 * 1024
    );
    assert!(process.memory().unwrap()[5000..].iter().all(|&b| b == 0));
    let mut words = [0; 13];
    words[0] = BASE + 1000;
    words[1] = 0xabcdef;
    assert_eq!(
        process
            .invoke(
                process
                    .bind(
                        BASE + 512,
                        NativeAbi::SystemV,
                        &[NativeScalar::Word; 13],
                        NativeScalar::Word
                    )
                    .unwrap(),
                words,
                |_, _, _| { Err(NativeError::Callback) }
            )
            .unwrap(),
        words[1]
    );
    assert_eq!(
        &process.memory().unwrap()[1000..1008],
        &words[1].to_le_bytes()
    );
    assert!(matches!(
        process.bind(
            BASE + 519,
            NativeAbi::SystemV,
            &[NativeScalar::Word; 13],
            NativeScalar::Word
        ),
        Err(NativeError::Extent)
    ));

    for (offset, length, permissions) in [(4999, 2, 1), (0, 0, 1), (0, 1, 8)] {
        assert!(matches!(
            image_child(
                &bytes,
                &[NativeRegion {
                    offset,
                    length,
                    permissions
                }],
                &[],
                Duration::from_secs(3)
            ),
            Err(NativeError::Extent)
        ));
    }
}

fn standard(code: &[u8]) -> NativeProcess {
    match child(code, Duration::from_secs(3)) {
        Ok(child) => child,
        Err(error) => panic!("native child failed: {error:?}"),
    }
}

#[test]
fn native_system_v_and_microsoft_calls_publish_the_same_owned_memory() {
    // mov rax,[rdi]; add rax,rsi; mov [rdi],rax; ret
    let mut system_v = standard(&[0x48, 0x8b, 0x07, 0x48, 0x01, 0xf0, 0x48, 0x89, 0x07, 0xc3]);
    assert_ne!(system_v.pid(), std::process::id());
    for task in std::fs::read_dir(format!("/proc/{}/task", system_v.pid())).expect("owned tasks") {
        let status = std::fs::read_to_string(task.expect("owned task").path().join("status"))
            .expect("kernel status");
        assert!(status.lines().any(|line| line.starts_with("State:\tT")));
        assert!(status.lines().any(|line| line == "Seccomp:\t2"));
    }
    system_v.memory_mut().expect("parked")[4096..4104].copy_from_slice(&11u64.to_le_bytes());
    let mut arguments = [0; 13];
    arguments[0] = BASE + 4096;
    arguments[1] = 7;
    for wanted in [18u64, 25, 32] {
        assert_eq!(
            system_v
                .invoke(
                    system_v
                        .bind(
                            BASE,
                            NativeAbi::SystemV,
                            &[NativeScalar::Word; 13],
                            NativeScalar::Word
                        )
                        .unwrap(),
                    arguments,
                    |_, _, _| Err(NativeError::Callback)
                )
                .expect("native return"),
            wanted
        );
        assert_eq!(
            &system_v.memory().expect("parked")[4096..4104],
            &wanted.to_le_bytes()
        );
    }
    // mov rax,[rcx]; add rax,rdx; mov [rcx],rax; ret
    let mut microsoft = standard(&[0x48, 0x8b, 0x01, 0x48, 0x01, 0xd0, 0x48, 0x89, 0x01, 0xc3]);
    assert_eq!(
        microsoft
            .invoke(
                microsoft
                    .bind(
                        BASE,
                        NativeAbi::Microsoft,
                        &[NativeScalar::Word; 13],
                        NativeScalar::Word
                    )
                    .unwrap(),
                arguments,
                |_, _, _| Err(NativeError::Callback)
            )
            .expect("native return"),
        7
    );
    assert_eq!(
        &microsoft.memory().expect("parked")[4096..4104],
        &7u64.to_le_bytes()
    );
}

#[test]
fn native_import_stops_before_engine_access_and_resumes_with_the_reply() {
    // sub rsp,8; mov rax,rdi; mov edi,37; mov rsi,rdx; call rax; add rsp,8; ret
    let mut process = standard(&[
        0x48, 0x83, 0xec, 0x08, 0x48, 0x89, 0xf8, 0xbf, 37, 0, 0, 0, 0x48, 0x89, 0xd6, 0xff, 0xd0,
        0x48, 0x83, 0xc4, 0x08, 0xc3,
    ]);
    let mut arguments = [0; 13];
    arguments[0] = process.callback(NativeAbi::SystemV);
    arguments[2] = BASE + 4096;
    let mut calls = 0;
    let result = process
        .invoke(
            process
                .bind(
                    BASE,
                    NativeAbi::SystemV,
                    &[NativeScalar::Word; 13],
                    NativeScalar::Word,
                )
                .unwrap(),
            arguments,
            |call, base, memory| {
                calls += 1;
                assert_eq!(call.number, 37);
                assert_eq!(call.arguments[0], base + 4096);
                memory[4096..4104].copy_from_slice(&123u64.to_le_bytes());
                Ok(91)
            },
        )
        .expect("import return");
    assert_eq!(result, 91);
    assert_eq!(calls, 1);
    assert_eq!(
        &process.memory().expect("parked")[4096..4104],
        &123u64.to_le_bytes()
    );
}

#[test]
fn all_thirteen_export_words_use_the_native_register_and_stack_locations() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let mut process = standard(&[0xc3]);
        let words = std::array::from_fn(|i| 0xfedc_ba98_7654_0000 + i as u64);
        let registers: &[_] = if abi == NativeAbi::SystemV {
            &[
                [0x48, 0x89, 0xf8],
                [0x48, 0x89, 0xf0],
                [0x48, 0x89, 0xd0],
                [0x48, 0x89, 0xc8],
                [0x4c, 0x89, 0xc0],
                [0x4c, 0x89, 0xc8],
            ]
        } else {
            &[
                [0x48, 0x89, 0xc8],
                [0x48, 0x89, 0xd0],
                [0x4c, 0x89, 0xc0],
                [0x4c, 0x89, 0xc8],
            ]
        };
        for (i, &wanted) in words.iter().enumerate() {
            let mut code = if let Some(register) = registers.get(i) {
                register.to_vec() // mov rax,<argument register>
            } else {
                let first = if abi == NativeAbi::SystemV { 8 } else { 40 };
                vec![
                    0x48,
                    0x8b,
                    0x44,
                    0x24,
                    first + (i - registers.len()) as u8 * 8,
                ]
            };
            code.push(0xc3);
            process.memory_mut().unwrap()[..code.len()].copy_from_slice(&code);
            assert_eq!(
                process
                    .invoke(
                        process
                            .bind(BASE, abi, &[NativeScalar::Word; 13], NativeScalar::Word)
                            .unwrap(),
                        words,
                        |_, _, _| Err(NativeError::Callback)
                    )
                    .unwrap(),
                wanted,
                "{abi:?} argument {i}"
            );
        }
        process.memory_mut().unwrap()[..4].copy_from_slice(&[0x48, 0x89, 0xe0, 0xc3]);
        let stack = process
            .invoke(
                process
                    .bind(BASE, abi, &[NativeScalar::Word; 13], NativeScalar::Word)
                    .unwrap(),
                words,
                |_, _, _| Err(NativeError::Callback),
            )
            .unwrap();
        assert!(stack >= BASE + 8192 + 4096);
        assert!(stack < BASE + process.memory().unwrap().len() as u64);
        assert_eq!(stack % 16, 8);
        assert!(!process.executable(stack));
    }
}

#[test]
fn raw_variadic_import_words_are_captured_without_a_rust_foreign_stack_frame() {
    let wanted: [u64; 14] = std::array::from_fn(|i| {
        if i == 0 {
            37
        } else {
            0xfedc_0000_3f80_0000 + i as u64
        }
    });
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let registers: &[_] = if abi == NativeAbi::SystemV {
            &[
                [0x48, 0xbf],
                [0x48, 0xbe],
                [0x48, 0xba],
                [0x48, 0xb9],
                [0x49, 0xb8],
                [0x49, 0xb9],
            ]
        } else {
            &[[0x48, 0xb9], [0x48, 0xba], [0x49, 0xb8], [0x49, 0xb9]]
        };
        let reserve = if abi == NativeAbi::SystemV { 72 } else { 120 };
        let mut code = vec![
            0x48,
            0x89,
            if abi == NativeAbi::SystemV {
                0xf8
            } else {
                0xc8
            }, // callback -> rax
            0x48,
            0x83,
            0xec,
            reserve,
        ];
        for (i, &word) in wanted.iter().enumerate() {
            if let Some(register) = registers.get(i) {
                code.extend_from_slice(register);
                code.extend_from_slice(&word.to_le_bytes());
            } else {
                code.extend_from_slice(&[0x49, 0xba]); // mov r10,<word>
                code.extend_from_slice(&word.to_le_bytes());
                let first = if abi == NativeAbi::SystemV { 0 } else { 32 };
                code.extend_from_slice(&[
                    0x4c,
                    0x89,
                    0x54,
                    0x24,
                    first + (i - registers.len()) as u8 * 8,
                ]);
            }
        }
        code.extend_from_slice(&[0xff, 0xd0, 0x48, 0x83, 0xc4, reserve, 0xc3]);
        let mut process = standard(&code);
        let mut words = [0; 13];
        words[0] = process.callback(abi);
        let mut calls = 0;
        assert_eq!(
            process
                .invoke(
                    process
                        .bind(BASE, abi, &[NativeScalar::Word; 13], NativeScalar::Word)
                        .unwrap(),
                    words,
                    |call, _, _| {
                        calls += 1;
                        assert_eq!(call.number, wanted[0] as u32);
                        assert_eq!(call.arguments, wanted[1..]);
                        Ok(0xabcde)
                    }
                )
                .unwrap(),
            0xabcde
        );
        assert_eq!(calls, 1);
    }
}

#[test]
fn native_local_buffers_are_shared_and_live_across_the_import_reply() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let (reserve, local, callback, number, argument) = if abi == NativeAbi::SystemV {
            (24, 0, 0xf8, 0xbf, 0x74)
        } else {
            (56, 32, 0xc8, 0xb9, 0x54)
        };
        let mut code = vec![0x48, 0x89, callback, 0x48, 0x83, 0xec, reserve, 0x49, 0xba];
        code.extend_from_slice(b"stack\0\0\0");
        code.extend_from_slice(&[
            0x4c, 0x89, 0x54, 0x24, local, // mov [rsp+local],r10
            0x48, 0x8d, argument, 0x24, local, number, 37, 0, 0, 0, 0xff, 0xd0, 0x48, 0x8b, 0x44,
            0x24, local, // read the parent's write to this local
            0x48, 0x83, 0xc4, reserve, 0xc3,
        ]);
        let mut process = standard(&code);
        let pid = process.pid();
        let mut words = [0; 13];
        words[0] = process.callback(abi);
        let result = process
            .invoke(
                process
                    .bind(BASE, abi, &[NativeScalar::Word; 13], NativeScalar::Word)
                    .unwrap(),
                words,
                |call, base, memory| {
                    assert_eq!(call.number, 37);
                    let at = (call.arguments[0] - base) as usize;
                    assert!(at >= 8192 + 4096);
                    assert_eq!(&memory[at..at + 8], b"stack\0\0\0");
                    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
                    assert!(status.lines().any(|line| line.starts_with("State:\tT")));
                    memory[at..at + 8].copy_from_slice(&123u64.to_le_bytes());
                    Ok(91)
                },
            )
            .unwrap();
        assert_eq!(result, 123);
    }
}

#[test]
fn shared_native_stack_guard_fault_is_local() {
    // mov rsp,rdi; push rax: touching the guard below the stack must fault.
    let mut process = standard(&[0x48, 0x89, 0xfc, 0x50, 0xc3]);
    let mut words = [0; 13];
    words[0] = BASE + 8192 + 4096;
    let result = process.invoke(
        process
            .bind(
                BASE,
                NativeAbi::SystemV,
                &[NativeScalar::Word; 13],
                NativeScalar::Word,
            )
            .unwrap(),
        words,
        |_, _, _| Err(NativeError::Callback),
    );
    assert!(
        matches!(result, Err(NativeError::Exited(status)) if status.signal() == Some(11)),
        "{result:?}"
    );
    assert_eq!(process.pid(), 0);
}

#[test]
fn native_import_restores_integer_simd_and_x87_state() {
    // Save r12/xmm6; set r12 and xmm6, push integer 123 on the x87 stack;
    // invoke import 37; sum all three afterward, restore the native callee
    // registers and return. The same body uses the appropriate syscall ABI.
    let template = [
        0x41, 0x54, 0x48, 0x81, 0xec, 0x80, 0x00, 0x00, 0x00, 0x48, 0x89, 0xf8, 0xf3, 0x0f, 0x7f,
        0x74, 0x24, 0x50, 0x49, 0xbc, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x49, 0xba,
        0x67, 0x45, 0x23, 0x01, 0xef, 0xcd, 0xab, 0x89, 0x66, 0x49, 0x0f, 0x6e, 0xf2, 0xc7, 0x44,
        0x24, 0x40, 0x7b, 0x00, 0x00, 0x00, 0xdb, 0x44, 0x24, 0x40, 0xbf, 0x25, 0x00, 0x00, 0x00,
        0xff, 0xd0, 0xdb, 0x5c, 0x24, 0x40, 0x66, 0x48, 0x0f, 0x7e, 0xf0, 0x4c, 0x01, 0xe0, 0x8b,
        0x54, 0x24, 0x40, 0x48, 0x01, 0xd0, 0xf3, 0x0f, 0x6f, 0x74, 0x24, 0x50, 0x48, 0x81, 0xc4,
        0x80, 0x00, 0x00, 0x00, 0x41, 0x5c, 0xc3,
    ];
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let mut code = template;
        if abi == NativeAbi::Microsoft {
            code[11] = 0xc8; // callback from rcx
            code[55] = 0xb9; // syscall number to ecx
        }
        let mut process = standard(&code);
        let mut words = [0; 13];
        words[0] = process.callback(abi);
        assert_eq!(
            process
                .invoke(
                    process
                        .bind(BASE, abi, &[NativeScalar::Word; 13], NativeScalar::Word)
                        .unwrap(),
                    words,
                    |call, _, _| {
                        assert_eq!(call.number, 37);
                        Ok(0)
                    }
                )
                .unwrap(),
            0x1122_3344_5566_7788 + 0x89ab_cdef_0123_4567 + 123
        );
    }
}

#[test]
fn region_lookup_handles_the_full_sorted_table_and_gaps() {
    let regions: Vec<_> = (0..65536)
        .map(|i| NativeRegion {
            offset: 8 + i * 16,
            length: 8,
            permissions: if i % 2 == 0 { 5 } else { 3 },
        })
        .collect();
    assert!(!executable_offset(&regions, 0));
    for (i, region) in regions.iter().enumerate() {
        assert_eq!(executable_offset(&regions, region.offset), i % 2 == 0);
        assert_eq!(executable_offset(&regions, region.offset + 7), i % 2 == 0);
        assert!(!executable_offset(&regions, region.offset + 8));
    }
    assert!(!executable_offset(&regions, usize::MAX));
}

#[test]
fn a_publication_without_self_stop_is_parked_before_engine_access() {
    // Send an IMPORT packet directly, then loop without entering the trusted
    // child callback. The controller must impose the stop before borrowing.
    // mov rsi,rdi; mov eax,1; xor edi,edi; mov edx,200; syscall; jmp $
    let mut process = child(
        &[
            0x48, 0x89, 0xfe, 0xb8, 1, 0, 0, 0, 0x31, 0xff, 0xba, 200, 0, 0, 0, 0x0f, 0x05, 0xeb,
            0xfe,
        ],
        Duration::from_millis(200),
    )
    .unwrap();
    let memory = process.memory_mut().unwrap();
    let packet = &mut memory[4096..4096 + 200];
    packet[..4].copy_from_slice(b"QARN");
    packet[4] = 2;
    packet[5] = 4;
    packet[8..16].copy_from_slice(&1u64.to_le_bytes());
    packet[16..24].copy_from_slice(&37u64.to_le_bytes());
    let mut arguments = [0; 13];
    arguments[0] = BASE + 4096;
    let pid = process.pid();
    let mut calls = 0;
    let result = process.invoke(
        process
            .bind(
                BASE,
                NativeAbi::SystemV,
                &[NativeScalar::Word; 13],
                NativeScalar::Word,
            )
            .unwrap(),
        arguments,
        |call, _, memory| {
            calls += 1;
            assert_eq!(call.number, 37);
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
            assert!(status.lines().any(|line| line.starts_with("State:\tT")));
            memory[5000] = 9;
            Ok(0)
        },
    );
    assert!(matches!(result, Err(NativeError::Timeout)), "{result:?}");
    assert_eq!(calls, 1);
    assert_eq!(process.pid(), 0);
}

#[test]
fn child_fault_and_timeout_leave_other_native_owners_running() {
    let mut healthy = standard(&[0x48, 0x89, 0xf8, 0xc3]); // mov rax,rdi; ret
    let mut fault = standard(&[0x48, 0x31, 0xc0, 0x48, 0x8b, 0x00, 0xc3]); // null read
    let result = fault.invoke(
        fault
            .bind(
                BASE,
                NativeAbi::SystemV,
                &[NativeScalar::Word; 13],
                NativeScalar::Word,
            )
            .unwrap(),
        [0; 13],
        |_, _, _| Err(NativeError::Callback),
    );
    assert!(
        matches!(result, Err(NativeError::Exited(status)) if status.signal() == Some(11)),
        "{result:?}"
    );
    assert_eq!(fault.pid(), 0);
    let mut timeout = child(&[0xeb, 0xfe], Duration::from_millis(200)).expect("timeout child");
    assert!(matches!(
        timeout.invoke(
            timeout
                .bind(
                    BASE,
                    NativeAbi::SystemV,
                    &[NativeScalar::Word; 13],
                    NativeScalar::Word
                )
                .unwrap(),
            [0; 13],
            |_, _, _| Err(NativeError::Callback)
        ),
        Err(NativeError::Timeout)
    ));
    assert_eq!(timeout.pid(), 0);
    let mut arguments = [0; 13];
    arguments[0] = 54;
    assert_eq!(
        healthy
            .invoke(
                healthy
                    .bind(
                        BASE,
                        NativeAbi::SystemV,
                        &[NativeScalar::Word; 13],
                        NativeScalar::Word
                    )
                    .unwrap(),
                arguments,
                |_, _, _| Err(NativeError::Callback)
            )
            .expect("healthy still runs"),
        54
    );
}

#[test]
fn repeated_imports_cannot_restart_the_export_deadline() {
    let mut healthy = standard(&[0x48, 0x89, 0xf8, 0xc3]);
    // push rbx; mov rbx,rdi; repeatedly call syscall 37 through rbx.
    let mut looping = child(
        &[
            0x53, 0x48, 0x89, 0xfb, 0xbf, 37, 0, 0, 0, 0xff, 0xd3, 0xeb, 0xf7,
        ],
        Duration::from_millis(400),
    )
    .unwrap();
    let mut words = [0; 13];
    words[0] = looping.callback(NativeAbi::SystemV);
    let mut calls = 0;
    let result = looping.invoke(
        looping
            .bind(
                BASE,
                NativeAbi::SystemV,
                &[NativeScalar::Word; 13],
                NativeScalar::Word,
            )
            .unwrap(),
        words,
        |call, _, _| {
            assert_eq!(call.number, 37);
            calls += 1;
            // Bound this fixture even if a regression lets the native export renew
            // its budget indefinitely; the expected result is Timeout, not Callback.
            if calls > 20 {
                return Err(NativeError::Callback);
            }
            std::thread::sleep(Duration::from_millis(40));
            Ok(0)
        },
    );
    assert!(matches!(result, Err(NativeError::Timeout)), "{result:?}");
    assert!(calls > 1 && calls <= 20, "imports: {calls}");
    assert_eq!(looping.pid(), 0);
    words[0] = 54;
    assert_eq!(
        healthy
            .invoke(
                healthy
                    .bind(
                        BASE,
                        NativeAbi::SystemV,
                        &[NativeScalar::Word; 13],
                        NativeScalar::Word
                    )
                    .unwrap(),
                words,
                |_, _, _| { Err(NativeError::Callback) }
            )
            .unwrap(),
        54
    );
}

#[test]
fn scalar_bindings_keep_mixed_integer_float_and_double_positions() {
    let kinds = [
        NativeScalar::Word,
        NativeScalar::Float,
        NativeScalar::Word,
        NativeScalar::Double,
        NativeScalar::Float,
        NativeScalar::Word,
    ];
    let values = [
        0x1234_5678_90ab_cdef,
        0xffff_ffff_8000_0000,
        0xfedc_ba98_7654_3210,
        0x7ff8_1234_5678_abcd,
        0xdead_beef_3f80_0000,
        0x0102_0304_0506_0708,
    ];
    let codes = [
        [
            vec![0x48, 0x89, 0xf8],
            vec![0x66, 0x48, 0x0f, 0x7e, 0xc0],
            vec![0x48, 0x89, 0xf0],
            vec![0x66, 0x48, 0x0f, 0x7e, 0xc8],
            vec![0x66, 0x48, 0x0f, 0x7e, 0xd0],
            vec![0x48, 0x89, 0xd0],
        ],
        [
            vec![0x48, 0x89, 0xc8],
            vec![0x66, 0x48, 0x0f, 0x7e, 0xc8],
            vec![0x4c, 0x89, 0xc0],
            vec![0x66, 0x48, 0x0f, 0x7e, 0xd8],
            vec![0x48, 0x8b, 0x44, 0x24, 40],
            vec![0x48, 0x8b, 0x44, 0x24, 48],
        ],
    ];
    for (abi, codes) in [NativeAbi::SystemV, NativeAbi::Microsoft]
        .into_iter()
        .zip(codes)
    {
        let mut process = standard(&[0xc3]);
        let entry = process.bind(BASE, abi, &kinds, NativeScalar::Word).unwrap();
        let mut words = [0; 13];
        words[..values.len()].copy_from_slice(&values);
        for (i, mut code) in codes.into_iter().enumerate() {
            code.push(0xc3);
            process.memory_mut().unwrap()[..code.len()].copy_from_slice(&code);
            let result = process
                .invoke(entry, words, |_, _, _| Err(NativeError::Callback))
                .unwrap();
            let wanted = if kinds[i] == NativeScalar::Float {
                values[i] & u32::MAX as u64
            } else {
                values[i]
            };
            assert_eq!(result, wanted, "{abi:?} mixed argument {i}");
        }
    }
}

#[test]
fn scalar_bindings_spill_after_each_native_float_register_limit() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        for kind in [NativeScalar::Float, NativeScalar::Double] {
            let mut process = standard(&[0xc3]);
            let entry = process
                .bind(BASE, abi, &[kind; 13], NativeScalar::Word)
                .unwrap();
            let words: [u64; 13] = std::array::from_fn(|i| 0x7ff8_0000_7fc0_0000 + i as u64);
            let registers = if abi == NativeAbi::SystemV { 8 } else { 4 };
            for (i, &value) in words.iter().enumerate() {
                // Read the selected XMM register or spilled stack word into RAX.
                let mut code = if i < registers {
                    vec![0x66, 0x48, 0x0f, 0x7e, 0xc0 | (i as u8 * 8)]
                } else {
                    let first = if abi == NativeAbi::SystemV { 8 } else { 40 };
                    vec![0x48, 0x8b, 0x44, 0x24, first + (i - registers) as u8 * 8]
                };
                code.push(0xc3);
                process.memory_mut().unwrap()[..code.len()].copy_from_slice(&code);
                let result = process
                    .invoke(entry, words, |_, _, _| Err(NativeError::Callback))
                    .unwrap();
                let wanted = if kind == NativeScalar::Float {
                    value & u32::MAX as u64
                } else {
                    value
                };
                assert_eq!(result, wanted, "{abi:?} {kind:?} argument {i}");
            }
        }
    }
}

#[test]
fn scalar_results_preserve_float_bits_and_void_discards_register_garbage() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let mut process = standard(&[0xc3]); // leaves XMM0 as supplied
        for kind in [NativeScalar::Float, NativeScalar::Double] {
            let entry = process.bind(BASE, abi, &[kind], kind).unwrap();
            for value in [
                0,
                0x8000_0000,
                0x7fc0_1234,
                0x8000_0000_0000_0000,
                0x7ff0_0000_0000_0000,
                0x7ff8_1234_5678_abcd,
            ] {
                let mut words = [0; 13];
                words[0] = value;
                let result = process
                    .invoke(entry, words, |_, _, _| Err(NativeError::Callback))
                    .unwrap();
                assert_eq!(
                    result,
                    if kind == NativeScalar::Float {
                        value & u32::MAX as u64
                    } else {
                        value
                    }
                );
            }
        }
        process.memory_mut().unwrap()[..6].copy_from_slice(&[0xb8, 99, 0, 0, 0, 0xc3]);
        let entry = process.bind(BASE, abi, &[], NativeScalar::Void).unwrap();
        assert_eq!(
            process
                .invoke(entry, [0; 13], |_, _, _| Err(NativeError::Callback))
                .unwrap(),
            0
        );
        for parameters in [&[NativeScalar::Void][..], &[NativeScalar::Word; 14][..]] {
            assert!(matches!(
                process.bind(BASE, abi, parameters, NativeScalar::Word),
                Err(NativeError::Unsupported)
            ));
        }
    }
}

#[test]
fn typed_native_function_imports_decode_scalar_arguments_and_return_in_the_native_bank() {
    let kinds = [
        NativeScalar::Word,
        NativeScalar::Float,
        NativeScalar::Double,
        NativeScalar::Word,
    ];
    let mut arguments = [0; 13];
    arguments[..4].copy_from_slice(&[
        0xfedc_ba98_7654_3210,
        0x8000_0000,
        0x7ff8_1234_5678_abcd,
        0x0123_4567_89ab_cdef,
    ]);
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        for result_kind in [
            NativeScalar::Word,
            NativeScalar::Float,
            NativeScalar::Double,
            NativeScalar::Void,
        ] {
            let reserve = if abi == NativeAbi::SystemV { 8 } else { 40 };
            // Forward these four native arguments to the load-bound function
            // pointer at BASE+4096; unlike Q3, there is no leading ordinal.
            let code = [
                0x48, 0x83, 0xec, reserve, 0x48, 0x8b, 0x05, 0xf5, 0x0f, 0, 0, 0xff, 0xd0, 0x48,
                0x83, 0xc4, reserve, 0xc3,
            ];
            let mut bytes = vec![0; 8192];
            bytes[..code.len()].copy_from_slice(&code);
            let imports = [NativeImport {
                number: 137,
                abi,
                parameters: &kinds,
                result: result_kind,
            }];
            let mut process =
                image_child(&bytes, &REGIONS, &imports, Duration::from_secs(3)).unwrap();
            let pointer = process.import_pointer(0).unwrap();
            assert!(process.import_pointer(1).is_none());
            assert!(process.executable(pointer));
            assert!(!process.executable(pointer + 24));
            process.memory_mut().unwrap()[4096..4104].copy_from_slice(&pointer.to_le_bytes());
            let entry = process.bind(BASE, abi, &kinds, result_kind).unwrap();
            let mut calls = 0;
            let result = process
                .invoke(entry, arguments, |call, _, _| {
                    calls += 1;
                    assert_eq!(call.number, 137);
                    assert_eq!(call.arguments, arguments);
                    Ok(0x7ff8_5678_8000_0000)
                })
                .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(
                result,
                match result_kind {
                    NativeScalar::Word | NativeScalar::Double => 0x7ff8_5678_8000_0000,
                    NativeScalar::Float => 0x8000_0000,
                    NativeScalar::Void => 0,
                    _ => unreachable!("this fixture selects word, floating or void results"),
                }
            );
        }
    }
}

#[test]
fn native_integer_exports_apply_declared_widths_to_arguments_and_results() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        // Return the whole incoming register, exposing the ABI conversion
        // separately from whatever narrowing a C callee might perform.
        let code = match abi {
            NativeAbi::SystemV => [0x48, 0x89, 0xf8, 0xc3],
            NativeAbi::Microsoft => [0x48, 0x89, 0xc8, 0xc3],
        };
        let mut process = standard(&code);
        for &(kind, raw, expected) in INTEGER_CASES {
            let mut arguments = [0; 13];
            arguments[0] = raw;
            for (parameters, result) in [(kind, NativeScalar::Word), (NativeScalar::Word, kind)] {
                let entry = process.bind(BASE, abi, &[parameters], result).unwrap();
                assert_eq!(
                    process
                        .invoke(entry, arguments, |_, _, _| Err(NativeError::Callback))
                        .unwrap(),
                    expected,
                    "{abi:?} {kind:?} parameter {parameters:?} result {result:?}",
                );
            }
        }
    }
}

const INTEGER_CASES: &[(NativeScalar, u64, u64)] = &[
    (
        NativeScalar::I8,
        0x1234_5678_90ab_cd80,
        0xffff_ffff_ffff_ff80,
    ),
    (NativeScalar::I8, 0xffff_ffff_ffff_ff7f, 0x7f),
    (NativeScalar::U8, 0x1234_5678_90ab_cdff, 0xff),
    (
        NativeScalar::I16,
        0x1234_5678_90ab_8000,
        0xffff_ffff_ffff_8000,
    ),
    (NativeScalar::I16, 0xffff_ffff_ffff_7fff, 0x7fff),
    (NativeScalar::U16, 0x1234_5678_90ab_ffff, 0xffff),
    (
        NativeScalar::I32,
        0x1234_5678_8000_0000,
        0xffff_ffff_8000_0000,
    ),
    (NativeScalar::I32, 0xffff_ffff_7fff_ffff, 0x7fff_ffff),
    (NativeScalar::U32, 0x1234_5678_ffff_ffff, 0xffff_ffff),
    (
        NativeScalar::Word,
        0x1234_5678_90ab_cdef,
        0x1234_5678_90ab_cdef,
    ),
];

#[test]
fn native_integer_imports_apply_declared_widths_in_both_directions() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        for &(kind, raw, expected) in INTEGER_CASES {
            let reserve = if abi == NativeAbi::SystemV { 8 } else { 40 };
            let code = [
                0x48, 0x83, 0xec, reserve, 0x48, 0x8b, 0x05, 0xf5, 0x0f, 0, 0, 0xff, 0xd0, 0x48,
                0x83, 0xc4, reserve, 0xc3,
            ];
            let mut bytes = vec![0; 8192];
            bytes[..code.len()].copy_from_slice(&code);
            let parameters = [kind];
            let imports = [NativeImport {
                number: 141,
                abi,
                parameters: &parameters,
                result: kind,
            }];
            let mut process =
                image_child(&bytes, &REGIONS, &imports, Duration::from_secs(3)).unwrap();
            let pointer = process.import_pointer(0).unwrap();
            process.memory_mut().unwrap()[4096..4104].copy_from_slice(&pointer.to_le_bytes());
            // The surrounding export leaves upper bits untouched. The import
            // itself must apply its declared width to the captured register.
            let entry = process
                .bind(BASE, abi, &[NativeScalar::Word], NativeScalar::Word)
                .unwrap();
            let mut arguments = [0; 13];
            arguments[0] = raw;
            let mut calls = 0;
            let actual = process
                .invoke(entry, arguments, |call, _, _| {
                    calls += 1;
                    assert_eq!(call.number, 141);
                    assert_eq!(
                        call.arguments[0], expected,
                        "{abi:?} {kind:?} import argument"
                    );
                    assert!(call.arguments[1..].iter().all(|&value| value == 0));
                    Ok(raw)
                })
                .unwrap();
            assert_eq!(calls, 1);
            assert_eq!(actual, expected, "{abi:?} {kind:?} import result");
        }
    }
}

#[test]
fn native_code_cannot_spawn_an_uncontrolled_writer_or_change_page_rights() {
    // syscall clone with all-zero arguments: must fail with EPERM before a child exists.
    let mut process = standard(&[
        0xb8, 56, 0, 0, 0, 0x48, 0x31, 0xff, 0x48, 0x31, 0xf6, 0x48, 0x31, 0xd2, 0x0f, 0x05, 0xc3,
    ]);
    assert_eq!(
        process
            .invoke(
                process
                    .bind(
                        BASE,
                        NativeAbi::SystemV,
                        &[NativeScalar::Word; 13],
                        NativeScalar::Word
                    )
                    .unwrap(),
                [0; 13],
                |_, _, _| Err(NativeError::Callback)
            )
            .expect("denied clone"),
        u64::MAX
    );
    // mprotect(address,4096,7) is denied even for the child's shared map.
    process.memory_mut().expect("parked")[..8]
        .copy_from_slice(&[0xb8, 10, 0, 0, 0, 0x0f, 0x05, 0xc3]);
    let mut arguments = [0; 13];
    arguments[0] = BASE;
    arguments[1] = 4096;
    arguments[2] = 7;
    assert_eq!(
        process
            .invoke(
                process
                    .bind(
                        BASE,
                        NativeAbi::SystemV,
                        &[NativeScalar::Word; 13],
                        NativeScalar::Word
                    )
                    .unwrap(),
                arguments,
                |_, _, _| Err(NativeError::Callback)
            )
            .expect("denied rights change"),
        u64::MAX
    );
}

#[test]
fn nonexecutable_entry_and_callback_rejection_are_local() {
    let process = standard(&[0xc3]);
    assert!(matches!(
        process.bind(
            BASE + 4096,
            NativeAbi::SystemV,
            &[NativeScalar::Word; 13],
            NativeScalar::Word
        ),
        Err(NativeError::Extent)
    ));
    assert_ne!(process.pid(), 0);
    assert!(matches!(
        NativeProcess::load(NativeImage {
            imports: &[],
            base: BASE,
            pointer_bytes: 4,
            bytes: &[],
            regions: &[],
            timeout: Duration::from_secs(1)
        }),
        Err(NativeError::Unsupported)
    ));
    let pid = process.pid();
    drop(process);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}
