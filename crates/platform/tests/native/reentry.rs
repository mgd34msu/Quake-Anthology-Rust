//! Real owned-child exports suspend imports and reenter the one hardware gate.
use super::{BASE, REGIONS, image_child};
use crate::native::{
    NativeAbi, NativeAction, NativeError, NativeImport, NativeRequest, NativeScalar,
    implementation::REENTRY_LIMIT,
};
use std::{
    os::unix::process::ExitStatusExt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

const DATA: u64 = BASE + 4096;

fn word(code: &mut Vec<u8>, register: &[u8], value: u64) {
    code.extend(register);
    code.extend(value.to_le_bytes());
}

fn function(before: &[u8], import: u64, after: &[u8]) -> Vec<u8> {
    // Both x64 ABIs enter at RSP%16==8. Keep shadow/raw capture space and a
    // canary away from the call frame, preserving the ABI's RBX value.
    let mut code = vec![0x53, 0x48, 0x81, 0xec];
    code.extend(160u32.to_le_bytes());
    code.extend(before);
    word(&mut code, &[0x48, 0xb8], import);
    code.extend([0xff, 0xd0]);
    code.extend(after);
    code.extend([0x48, 0x81, 0xc4]);
    code.extend(160u32.to_le_bytes());
    code.extend([0x5b, 0xc3]);
    code
}

fn imports(abi: NativeAbi) -> [NativeImport<'static>; 3] {
    std::array::from_fn(|index| NativeImport {
        trap: false,
        number: 10 + index as u32,
        abi,
        parameters: &[],
        result: if index == 0 {
            NativeScalar::Double
        } else {
            NativeScalar::Word
        },
        filter: None,
        format: None,
    })
}

fn put(bytes: &mut [u8], offset: usize, code: &[u8]) {
    bytes[offset..offset + code.len()].copy_from_slice(code);
}

fn data_word(memory: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(memory[4096 + offset..4104 + offset].try_into().unwrap())
}

#[test]
fn nested_exports_preserve_order_import_signatures_stack_and_floating_state() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let imports = imports(abi);
        let mut bytes = vec![0; 8192];
        let pointers: [u64; 3] = std::array::from_fn(|ordinal| {
            crate::native::NativeProcess::import_address(BASE, bytes.len(), ordinal).unwrap()
        });
        const RBX: u64 = 0x1122_3344_5566_7788;
        const CANARY: u64 = 0x8877_6655_4433_2211;
        const XMM6: u64 = 0x4019_0000_0000_0000;
        const MXCSR: u32 = 0x5f80;
        bytes[4128..4132].copy_from_slice(&MXCSR.to_le_bytes());
        let mut before = vec![];
        word(&mut before, &[0x48, 0xbb], RBX);
        word(&mut before, &[0x48, 0xb8], CANARY);
        before.extend([0x48, 0x89, 0x84, 0x24, 144, 0, 0, 0]);
        word(&mut before, &[0x48, 0xb8], XMM6);
        before.extend([0x66, 0x48, 0x0f, 0x6e, 0xf0]); // movq xmm6,rax
        word(&mut before, &[0x48, 0xb8], DATA + 32);
        before.extend([0x0f, 0xae, 0x10]); // ldmxcsr [rax]
        let mut after = vec![];
        word(&mut after, &[0x48, 0xba], DATA);
        after.extend([0x66, 0x0f, 0xd6, 0x02]); // movq [rdx],xmm0
        after.extend([0x48, 0x89, 0x5a, 8]);
        after.extend([0x48, 0x8b, 0x84, 0x24, 144, 0, 0, 0]);
        after.extend([0x48, 0x89, 0x42, 16]);
        after.extend([0x0f, 0xae, 0x5a, 24]); // stmxcsr [rdx+24]
        after.extend([0x66, 0x0f, 0xd6, 0x72, 40]); // movq [rdx+40],xmm6
        word(&mut after, &[0x48, 0xb8], 7.25f64.to_bits());
        after.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0]);
        put(&mut bytes, 0, &function(&before, pointers[0], &after));

        let mut before = vec![];
        word(&mut before, &[0x48, 0xba], DATA);
        before.extend([
            0x48, 0x8b, 0x42, 64, 0x48, 0x83, 0xc0, 1, 0x48, 0x89, 0x42, 72,
        ]);
        word(&mut before, &[0x48, 0xbb], !RBX);
        before.extend([0x66, 0x0f, 0xef, 0xf6]); // pxor xmm6,xmm6
        // Deliberately change the nested FP environment; the suspended outer
        // import's hardware capture must restore its own state independently.
        word(&mut before, &[0x48, 0xb8], DATA + 36);
        before.extend([0x0f, 0xae, 0x10]);
        let mut after = vec![];
        word(&mut after, &[0x48, 0xba], DATA);
        after.extend([0x48, 0x89, 0x42, 48]);
        after.extend([0x48, 0x8b, 0x42, 80, 0x48, 0x89, 0x42, 88]);
        word(&mut after, &[0x48, 0xb8], 12.5f64.to_bits());
        after.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0]);
        bytes[4132..4136].copy_from_slice(&0x1f80u32.to_le_bytes());
        put(&mut bytes, 256, &function(&before, pointers[1], &after));
        put(
            &mut bytes,
            512,
            &function(&[], pointers[2], &[0xb8, 17, 0, 0, 0]),
        );
        let mut process = image_child(&bytes, &REGIONS, &imports, Duration::from_secs(3)).unwrap();
        let outer = process.bind(BASE, abi, &[], NativeScalar::Double).unwrap();
        let nested = process
            .bind(BASE + 256, abi, &[], NativeScalar::Double)
            .unwrap();
        let leaf = process
            .bind(BASE + 512, abi, &[], NativeScalar::Word)
            .unwrap();
        let mut order = vec![];
        let value = process
            .invoke(outer, [0; 13], |request, base, memory| {
                assert_eq!(base, BASE);
                match request {
                    NativeRequest::Import(call) => match call.number {
                        10 => {
                            order.push(10);
                            memory[4160..4168].copy_from_slice(&111u64.to_le_bytes());
                            Ok(NativeAction::Call {
                                entry: nested,
                                arguments: [0; 13],
                                continuation: 10,
                            })
                        }
                        11 => {
                            order.push(11);
                            assert_eq!(data_word(memory, 72), 112);
                            Ok(NativeAction::Call {
                                entry: leaf,
                                arguments: [0; 13],
                                continuation: 11,
                            })
                        }
                        12 => {
                            order.push(12);
                            memory[4176..4184].copy_from_slice(&222u64.to_le_bytes());
                            Ok(NativeAction::Return(40))
                        }
                        _ => panic!("unexpected import"),
                    },
                    NativeRequest::Returned {
                        continuation: 11,
                        value,
                    } => {
                        order.push(111);
                        assert_eq!(value, 17);
                        Ok(NativeAction::Return(41))
                    }
                    NativeRequest::Returned {
                        continuation: 10,
                        value,
                    } => {
                        order.push(110);
                        assert_eq!(value, 12.5f64.to_bits());
                        assert_eq!(data_word(memory, 48), 41);
                        assert_eq!(data_word(memory, 88), 222);
                        Ok(NativeAction::Return(9.75f64.to_bits()))
                    }
                    NativeRequest::Returned { .. } => panic!("unexpected continuation"),
                }
            })
            .unwrap();
        assert_eq!(value, 7.25f64.to_bits());
        assert_eq!(order, [10, 11, 12, 111, 110]);
        let memory = process.memory().unwrap();
        assert_eq!(data_word(memory, 0), 9.75f64.to_bits());
        assert_eq!(data_word(memory, 8), RBX);
        assert_eq!(data_word(memory, 16), CANARY);
        assert_eq!(
            u32::from_le_bytes(memory[4120..4124].try_into().unwrap()),
            MXCSR
        );
        assert_eq!(data_word(memory, 40), XMM6);
    }
}

struct Continuation(Arc<AtomicUsize>);
impl Drop for Continuation {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn a_callback_reported_import_trap_reaps_its_waiting_child_and_preserves_other_owners() {
    let abi = NativeAbi::SystemV;
    let imports = imports(abi);
    let mut bytes = vec![0; 8192];
    let import = crate::native::NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
    put(&mut bytes, 0, &function(&[], import, &[]));
    put(&mut bytes, 512, &[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut process = image_child(&bytes, &REGIONS, &imports, Duration::from_secs(3)).unwrap();
    let mut healthy = image_child(&bytes, &REGIONS, &imports, Duration::from_secs(3)).unwrap();
    let entry = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let healthy_entry = healthy
        .bind(BASE + 512, abi, &[], NativeScalar::Word)
        .unwrap();
    let mut calls = 0;
    let result = process.invoke::<()>(entry, [0; 13], |request, _, _| {
        let NativeRequest::Import(call) = request else {
            panic!("no nested call")
        };
        assert_eq!(call.number, 10);
        calls += 1;
        Err(NativeError::ImportTrap {
            ordinal: 0,
            address: 0,
        })
    });
    assert!(
        matches!(
            result,
            Err(NativeError::ImportTrap {
                ordinal: 0,
                address: 0
            })
        ),
        "{result:?}"
    );
    assert_eq!(calls, 1);
    assert_eq!(process.pid(), 0);
    assert!(process.memory_mut().is_ok());
    assert_eq!(
        healthy
            .invoke::<()>(healthy_entry, [0; 13], |_, _, _| panic!("no imports"))
            .unwrap(),
        77
    );
}

#[test]
fn nested_import_trap_unwinds_every_gate_and_continuation_before_reuse() {
    let abi = NativeAbi::Microsoft;
    let mut imports = imports(abi);
    imports[1].trap = true;
    imports[1].result = NativeScalar::Void;
    let mut bytes = vec![0; 8192];
    let outer_import = crate::native::NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
    let trap_import = crate::native::NativeProcess::import_address(BASE, bytes.len(), 1).unwrap();
    put(&mut bytes, 0, &function(&[], outer_import, &[0x0f, 0x0b]));
    put(&mut bytes, 256, &function(&[], trap_import, &[0x0f, 0x0b]));
    put(&mut bytes, 512, &[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut process = image_child(&bytes, &REGIONS, &imports, Duration::from_secs(3)).unwrap();
    let pid = process.pid();
    let outer = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
    let nested = process
        .bind(BASE + 256, abi, &[], NativeScalar::Word)
        .unwrap();
    let healthy = process
        .bind(BASE + 512, abi, &[], NativeScalar::Word)
        .unwrap();
    let dropped = Arc::new(AtomicUsize::new(0));
    for count in 1..=3 {
        let result = process.invoke(outer, [0; 13], |request, _, _| match request {
            NativeRequest::Import(call) => {
                assert_eq!(call.number, 10);
                Ok(NativeAction::Call {
                    entry: nested,
                    arguments: [0; 13],
                    continuation: Continuation(dropped.clone()),
                })
            }
            NativeRequest::Returned { .. } => panic!("a trapped nested export cannot return"),
        });
        assert!(
            matches!(result, Err(NativeError::ImportTrap { ordinal: 1, .. })),
            "{result:?}"
        );
        assert_eq!(dropped.load(Ordering::Relaxed), count);
        assert_eq!(process.pid(), pid);
        assert_eq!(
            process
                .invoke::<()>(healthy, [0; 13], |_, _, _| panic!("no imports"))
                .unwrap(),
            77
        );
    }
}

#[test]
fn nested_import_and_return_borrows_stay_kernel_parked_until_reply() {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    for fatal in [false, true] {
        let abi = NativeAbi::SystemV;
        let mut imports = imports(abi);
        imports[0].result = NativeScalar::Word;
        let mut bytes = vec![0; 8192];
        let outer_import =
            crate::native::NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
        let nested_import =
            crate::native::NativeProcess::import_address(BASE, bytes.len(), 1).unwrap();
        let mut read = vec![];
        word(&mut read, &[0x48, 0xb8], DATA);
        read.extend([0x48, 0x8b, 0x00]);
        put(&mut bytes, 0, &function(&[], outer_import, &read));
        put(&mut bytes, 256, &function(&[], nested_import, &read));
        let mut command = super::native_child_command().unwrap();
        command.env("QA_NATIVE_TEST_SIGNAL", "1");
        let mut process = crate::native::NativeProcess::launch(
            command,
            crate::native::NativeImage {
                base: BASE,
                pointer_bytes: 8,
                bytes: &bytes,
                regions: &REGIONS,
                imports: &imports,
                timeout: Duration::from_secs(3),
                runtime: None,
            },
        )
        .unwrap();
        let pid = process.pid();
        let outer = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
        let nested = process
            .bind(BASE + 256, abi, &[], NativeScalar::Word)
            .unwrap();
        let result = process.invoke(outer, [0; 13], |request, _, memory| match request {
            NativeRequest::Import(call) if call.number == 10 => Ok(NativeAction::Call {
                entry: nested,
                arguments: [0; 13],
                continuation: (),
            }),
            NativeRequest::Import(call) => {
                assert_eq!(call.number, 11);
                // SAFETY: only this recorded owned child is signaled. Its
                // received notification cannot resume on a caught signal.
                assert_eq!(unsafe { kill(pid as i32, 10) }, 0);
                crate::pause(Duration::from_millis(20));
                memory[4096..4104].copy_from_slice(&123u64.to_le_bytes());
                Ok(NativeAction::Return(0))
            }
            NativeRequest::Returned {
                continuation: (),
                value,
            } => {
                assert_eq!(value, 123);
                // The returned phase gets the same validated parked borrow.
                // Fatal death leaves no writer; ordinary signals wait for SEND.
                assert_eq!(unsafe { kill(pid as i32, if fatal { 9 } else { 10 }) }, 0);
                crate::pause(Duration::from_millis(20));
                memory[4096..4104].copy_from_slice(&456u64.to_le_bytes());
                Ok(NativeAction::Return(42))
            }
        });
        if fatal {
            assert!(
                matches!(result, Err(NativeError::Exited(status)) if status.signal() == Some(9)),
                "{result:?}"
            );
            assert_eq!(process.pid(), 0);
        } else {
            assert_eq!(result.unwrap(), 456);
            assert_eq!(process.pid(), pid);
        }
        assert!(process.memory_mut().is_ok());
    }
}

#[test]
fn nested_failure_and_depth_overflow_reap_only_the_owned_child() {
    let abi = NativeAbi::SystemV;
    for (code, limit) in [
        (&[0x0f, 0x0b][..], false),
        (&[0xeb, 0xfe][..], false),
        (&[][..], true),
    ] {
        let imports = imports(abi);
        let mut bytes = vec![0; 8192];
        let import = crate::native::NativeProcess::import_address(BASE, bytes.len(), 0).unwrap();
        put(&mut bytes, 0, &function(&[], import, &[0xc3]));
        let recursive = function(&[], import, &[0xc3]);
        put(&mut bytes, 256, if limit { &recursive } else { code });
        let timeout = if code == [0xeb, 0xfe] {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(3)
        };
        let mut process = image_child(&bytes, &REGIONS, &imports, timeout).unwrap();
        let outer = process.bind(BASE, abi, &[], NativeScalar::Word).unwrap();
        let nested = process
            .bind(BASE + 256, abi, &[], NativeScalar::Word)
            .unwrap();
        let dropped = Arc::new(AtomicUsize::new(0));
        let mut calls = 0;
        let result = process.invoke(outer, [0; 13], |request, _, _| match request {
            NativeRequest::Import(_) => {
                calls += 1;
                Ok(NativeAction::Call {
                    entry: nested,
                    arguments: [0; 13],
                    continuation: Continuation(dropped.clone()),
                })
            }
            NativeRequest::Returned { .. } => panic!("failed export cannot return"),
        });
        if limit {
            assert!(matches!(result, Err(NativeError::Extent)), "{result:?}");
            assert_eq!(calls, REENTRY_LIMIT + 1);
        } else if code == [0xeb, 0xfe] {
            assert!(matches!(result, Err(NativeError::Timeout)), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(NativeError::Exited(status)) if status.signal() == Some(4)),
                "{result:?}"
            );
        }
        assert_eq!(dropped.load(Ordering::Relaxed), calls);
        assert_eq!(process.pid(), 0);
        assert!(process.memory_mut().is_ok());
    }
}
