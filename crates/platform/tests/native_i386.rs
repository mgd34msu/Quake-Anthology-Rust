//! Focused cross-width backend fixtures. Set QA_NATIVE_I386_HELPER to the
//! independently built ELF32 companion; ordinary host tests report a skip when
//! the host has no packaged i386 toolchain. No module ABI admission is implied.
use qa_platform::native::{
    NativeAbi, NativeError, NativeImage, NativeImport, NativeProcess, NativeRegion, NativeScalar,
};
use qa_platform::native::{NativeAction, NativeRequest, NativeStop};
use std::{path::PathBuf, time::Duration};

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
fn helper() -> Option<PathBuf> {
    let helper = std::env::var_os("QA_NATIVE_I386_HELPER").map(PathBuf::from);
    if helper.is_none() {
        eprintln!("i386 backend fixture skipped: QA_NATIVE_I386_HELPER is not configured");
    }
    helper
}

fn nested_imports() -> [NativeImport<'static>; 2] {
    std::array::from_fn(|index| NativeImport {
        trap: false,
        number: 10 + index as u32,
        abi: NativeAbi::SystemV,
        parameters: &[],
        result: NativeScalar::Word,
        filter: None,
        format: None,
    })
}

fn cdecl_call(import: u32, after: &[u8]) -> Vec<u8> {
    let mut code = vec![0x53, 0x83, 0xec, 112, 0xbb];
    code.extend(0x1122_3344u32.to_le_bytes());
    code.push(0xb8);
    code.extend(import.to_le_bytes());
    code.extend([0xff, 0xd0]);
    code.extend(after);
    code.extend([0x83, 0xc4, 112, 0x5b, 0xc3]);
    code
}

#[test]
fn cdecl_nested_exports_resume_the_suspended_import_in_the_same_owned_child() {
    let Some(helper) = helper() else { return };
    let imports = nested_imports();
    let mut bytes = [0; 8192];
    let outer_import = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap() as u32;
    let nested_import = NativeProcess::import_address(BASE, bytes.len(), 1).unwrap() as u32;
    let mut after = vec![0xa3]; // mov [DATA],eax; mov eax,ebx
    after.extend((BASE as u32 + 4096).to_le_bytes());
    after.extend([0x89, 0xd8]);
    let outer_code = cdecl_call(outer_import, &after);
    let nested_code = cdecl_call(nested_import, &[0x83, 0xc0, 1]);
    bytes[..outer_code.len()].copy_from_slice(&outer_code);
    bytes[256..256 + nested_code.len()].copy_from_slice(&nested_code);
    let mut child = load(&helper, &bytes, &imports);
    let outer = child
        .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    let nested = child
        .bind(BASE + 256, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    let mut order = vec![];
    let result = child
        .invoke(outer, [0; 13], |request, _, _| match request {
            NativeRequest::Import(call) if call.number == 10 => {
                order.push(10);
                Ok(NativeAction::Call {
                    entry: nested,
                    arguments: [0; 13],
                    continuation: (),
                })
            }
            NativeRequest::Import(call) => {
                assert_eq!(call.number, 11);
                order.push(11);
                Ok(NativeAction::Return(40))
            }
            NativeRequest::Returned {
                continuation: (),
                value,
            } => {
                order.push(101);
                assert_eq!(value, 41);
                Ok(NativeAction::Return(42))
            }
        })
        .unwrap();
    assert_eq!(result, 0x1122_3344);
    assert_eq!(order, [10, 11, 101]);
    assert_eq!(&child.memory().unwrap()[4096..4100], &42u32.to_le_bytes());
}

#[test]
fn cdecl_raw_stops_keep_outer_import_authority_across_external_and_nested_work() {
    let Some(helper) = helper() else { return };
    let imports = nested_imports();
    let mut bytes = [0; 8192];
    let outer_import = NativeProcess::import_address(BASE, bytes.len(), 0).unwrap() as u32;
    let nested_import = NativeProcess::import_address(BASE, bytes.len(), 1).unwrap() as u32;
    let mut after = vec![0xa3]; // Store the narrowed outer import return.
    after.extend((BASE as u32 + 4096).to_le_bytes());
    after.push(0xa1); // Observe the parent's post-nested write.
    after.extend((BASE as u32 + 4100).to_le_bytes());
    after.push(0xa3);
    after.extend((BASE as u32 + 4104).to_le_bytes());
    after.extend([0x89, 0xd8]); // Return preserved EBX.
    let outer_code = cdecl_call(outer_import, &after);
    let mut after = vec![0xa3]; // Store the narrowed nested import return.
    after.extend((BASE as u32 + 4116).to_le_bytes());
    after.push(0xa1);
    after.extend((BASE as u32 + 4112).to_le_bytes());
    after.extend([0x83, 0xc0, 1, 0xa3]);
    after.extend((BASE as u32 + 4120).to_le_bytes());
    after.extend([0xb8, 41, 0, 0, 0]);
    let nested_code = cdecl_call(nested_import, &after);
    bytes[..outer_code.len()].copy_from_slice(&outer_code);
    bytes[256..256 + nested_code.len()].copy_from_slice(&nested_code);
    bytes[512..518].copy_from_slice(&[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut child = load(&helper, &bytes, &imports);
    let mut external = load(&helper, &bytes, &imports);
    let outer_entry = child
        .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    let nested_entry = child
        .bind(BASE + 256, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    let external_entry = external
        .bind(BASE + 512, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    let outer = child.enter(outer_entry, [0; 13]).unwrap();
    let NativeStop::Import {
        call,
        stopped: outer_stop,
    } = child.advance(&outer).unwrap()
    else {
        panic!("outer import");
    };
    assert_eq!(call.number, 10);
    assert!(call.function);
    assert!(matches!(child.memory(), Err(NativeError::Protocol)));
    assert!(matches!(child.memory_mut(), Err(NativeError::Protocol)));
    child
        .with_import_memory(&outer_stop, |base, memory| {
            assert_eq!(base, BASE);
            memory[4112..4116].copy_from_slice(&11u32.to_le_bytes());
            Ok(())
        })
        .unwrap();
    let host_work = external.enter(external_entry, [0; 13]).unwrap();
    assert!(matches!(
        external.advance(&host_work).unwrap(),
        NativeStop::Returned(77)
    ));
    let nested = child.enter(nested_entry, [0; 13]).unwrap();
    let NativeStop::Import {
        call,
        stopped: nested_stop,
    } = child.advance(&nested).unwrap()
    else {
        panic!("nested import");
    };
    assert_eq!(call.number, 11);
    assert!(call.function);
    child.reply(nested_stop, 0xfeed_face_89ab_cdef).unwrap();
    assert!(matches!(
        child.advance(&nested).unwrap(),
        NativeStop::Returned(41)
    ));
    // The logical outer import is current again under a newer kernel park.
    child
        .with_import_memory(&outer_stop, |_, memory| {
            assert_eq!(&memory[4116..4120], &0x89ab_cdefu32.to_le_bytes());
            assert_eq!(&memory[4120..4124], &12u32.to_le_bytes());
            memory[4100..4104].copy_from_slice(&222u32.to_le_bytes());
            Ok(())
        })
        .unwrap();
    child.reply(outer_stop, 0xdead_beef_7654_3210).unwrap();
    assert!(matches!(
        child.advance(&outer).unwrap(),
        NativeStop::Returned(0x1122_3344)
    ));
    let memory = child.memory().unwrap();
    assert_eq!(&memory[4096..4100], &0x7654_3210u32.to_le_bytes());
    assert_eq!(&memory[4104..4108], &222u32.to_le_bytes());
    assert_eq!(&memory[4116..4120], &0x89ab_cdefu32.to_le_bytes());
    assert_eq!(&memory[4120..4124], &12u32.to_le_bytes());
}
fn load(helper: &std::path::Path, bytes: &[u8], imports: &[NativeImport<'_>]) -> NativeProcess {
    NativeProcess::load_with_helper(
        NativeImage {
            base: BASE,
            pointer_bytes: 4,
            bytes,
            regions: &REGIONS,
            imports,
            timeout: Duration::from_secs(3),
            runtime: None,
        },
        helper,
    )
    .unwrap()
}

#[test]
fn cdecl_scalar_cells_and_word_width_are_selected_by_the_child() {
    let Some(helper) = helper() else {
        return;
    };
    let kinds = [
        NativeScalar::Word,
        NativeScalar::Double,
        NativeScalar::I32,
        NativeScalar::Float,
        NativeScalar::Double,
        NativeScalar::I16,
        NativeScalar::U8,
        NativeScalar::Double,
        NativeScalar::Word,
        NativeScalar::Double,
        NativeScalar::Double,
        NativeScalar::Double,
        NativeScalar::Double,
    ];
    let mut values = [0u64; 13];
    for (i, value) in values.iter_mut().enumerate() {
        *value = 0xa0b0_c000_1020_3000 + i as u64;
    }
    let mut bytes = [0u8; 8192];
    bytes[0] = 0xc3;
    let mut child = load(&helper, &bytes, &[]);
    let entry = child
        .bind(BASE, NativeAbi::SystemV, &kinds, NativeScalar::Word)
        .unwrap();
    let mut cell = 0;
    for (kind, value) in kinds.iter().zip(values) {
        let cells = if *kind == NativeScalar::Double { 2 } else { 1 };
        for half in 0..cells {
            let displacement = 4 + cell * 4;
            let code = [0x8b, 0x44, 0x24, displacement as u8, 0xc3];
            child.memory_mut().unwrap()[..code.len()].copy_from_slice(&code);
            let wanted = if half == 1 {
                value >> 32
            } else {
                match kind {
                    NativeScalar::I16 => (value as i16 as i32 as u32) as u64,
                    NativeScalar::U8 => u64::from(value as u8),
                    _ => value & u64::from(u32::MAX),
                }
            };
            assert_eq!(
                child
                    .invoke::<()>(entry, values, |_, _, _| Err(NativeError::Callback))
                    .unwrap(),
                wanted
            );
            cell += 1;
        }
    }
    assert!(matches!(
        child.bind(BASE, NativeAbi::Microsoft, &[], NativeScalar::Word),
        Err(NativeError::Unsupported)
    ));
}

#[test]
fn typed_imports_decode_packed_cdecl_and_return_through_x87() {
    let Some(helper) = helper() else {
        return;
    };
    let kinds = [
        NativeScalar::Word,
        NativeScalar::Float,
        NativeScalar::Double,
        NativeScalar::I32,
        NativeScalar::Double,
    ];
    let mut arguments = [0; 13];
    arguments[..5].copy_from_slice(&[
        0xdead_beef_1234_5678,
        u64::from((-0.0f32).to_bits()),
        2.75f64.to_bits(),
        0xffff_ffff_8000_0001,
        (-3.25f64).to_bits(),
    ]);
    let expected = [
        0x1234_5678,
        u64::from((-0.0f32).to_bits()),
        2.75f64.to_bits(),
        0xffff_ffff_8000_0001,
        (-3.25f64).to_bits(),
    ];
    for (result, value) in [
        (NativeScalar::Word, 0xcafe_babe_7654_3210),
        (NativeScalar::Float, u64::from((-1.25f32).to_bits())),
        (NativeScalar::Double, (-1234.5f64).to_bits()),
        (NativeScalar::Void, u64::MAX),
    ] {
        let imports = [NativeImport {
            trap: false,
            number: 137,
            abi: NativeAbi::SystemV,
            parameters: &kinds,
            result,
            filter: None,
            format: None,
        }];
        let bytes = [0u8; 8192];
        let mut child = load(&helper, &bytes, &imports);
        let entry = child
            .bind(
                child.import_pointer(0).unwrap(),
                NativeAbi::SystemV,
                &kinds,
                result,
            )
            .unwrap();
        let mut calls = 0;
        let returned = child
            .invoke::<()>(entry, arguments, |call, base, memory| {
                let NativeRequest::Import(call) = call else {
                    panic!("scalar callback cannot reenter")
                };

                calls += 1;
                assert_eq!(call.number, 137);
                assert!(call.function);
                assert_eq!(&call.arguments[..5], &expected);
                assert_eq!(base, BASE);
                memory[4096] = 0x5a;
                Ok(NativeAction::<()>::Return(value))
            })
            .unwrap();
        assert_eq!(
            returned,
            match result {
                NativeScalar::Void => 0,
                NativeScalar::Word => value & u64::from(u32::MAX),
                _ => value,
            }
        );
        assert_eq!(calls, 1);
        assert_eq!(child.memory().unwrap()[4096], 0x5a);
    }
}

#[test]
fn raw_syscalls_use_i386_words_and_the_same_notification_slot() {
    let Some(helper) = helper() else {
        return;
    };
    let mut bytes = [0u8; 8192];
    bytes[0] = 0xc3;
    let mut child = load(&helper, &bytes, &[]);
    let target = child.callback(NativeAbi::SystemV) as u32;
    let mut code = vec![0xb8];
    code.extend(target.to_le_bytes());
    code.extend([0xff, 0xe0]);
    child.memory_mut().unwrap()[..code.len()].copy_from_slice(&code);
    let entry = child
        .bind(
            BASE,
            NativeAbi::SystemV,
            &[NativeScalar::Word; 13],
            NativeScalar::Word,
        )
        .unwrap();
    let arguments = std::array::from_fn(|i| {
        if i == 0 {
            173
        } else {
            0xfeed_face_0000_0000 | i as u64
        }
    });
    let mut calls = 0;
    assert_eq!(
        child
            .invoke::<()>(entry, arguments, |call, _, _| {
                let NativeRequest::Import(call) = call else {
                    panic!("scalar callback cannot reenter")
                };

                calls += 1;
                assert_eq!(call.number, 173);
                assert!(!call.function);
                for i in 0..12 {
                    assert_eq!(call.arguments[i], (i + 1) as u64);
                }
                assert_eq!(call.arguments[12], 0);
                Ok(NativeAction::<()>::Return(0xabcd_ef01_9876_5432))
            })
            .unwrap(),
        0x9876_5432
    );
    assert_eq!(calls, 1);
}

#[test]
fn trap_teardown_and_address_limits_remain_owned() {
    let Some(helper) = helper() else {
        return;
    };
    let imports = [NativeImport {
        trap: true,
        number: 0,
        abi: NativeAbi::SystemV,
        parameters: &[],
        result: NativeScalar::Void,
        filter: None,
        format: None,
    }];
    let mut bytes = [0u8; 8192];
    bytes[0..6].copy_from_slice(&[0xb8, 77, 0, 0, 0, 0xc3]);
    let mut child = load(&helper, &bytes, &imports);
    let pid = child.pid();
    let trap = child
        .bind(
            child.import_pointer(0).unwrap(),
            NativeAbi::SystemV,
            &[],
            NativeScalar::Void,
        )
        .unwrap();
    let healthy = child
        .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    assert!(matches!(
        child.invoke::<()>(trap, [0; 13], |_, _, _| Err(NativeError::Callback)),
        Err(NativeError::ImportTrap { ordinal: 0, .. })
    ));
    assert_eq!(child.pid(), pid);
    assert_eq!(
        child
            .invoke::<()>(healthy, [0; 13], |_, _, _| Err(NativeError::Callback))
            .unwrap(),
        77
    );
    let result = NativeProcess::load_with_helper(
        NativeImage {
            base: 0xffff_f000,
            pointer_bytes: 4,
            bytes: &bytes,
            regions: &REGIONS,
            imports: &[],
            timeout: Duration::from_secs(3),
            runtime: None,
        },
        &helper,
    );
    assert!(matches!(result, Err(NativeError::Extent)));
    let pid = child.pid();
    drop(child);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn child_local_callbacks_reenter_the_same_gate_and_restore_outer_cdecl_state() {
    let Some(helper) = helper() else {
        return;
    };
    let sort =
        qa_platform::native::runtime::function(qa_platform::native::runtime::FIRST + 311).unwrap();
    let imports = [
        NativeImport {
            trap: false,
            number: sort.number,
            abi: NativeAbi::SystemV,
            parameters: sort.parameters,
            result: sort.result,
            filter: None,
            format: None,
        },
        NativeImport {
            trap: false,
            number: 200,
            abi: NativeAbi::SystemV,
            parameters: &[NativeScalar::Word, NativeScalar::Word],
            result: NativeScalar::I32,
            filter: None,
            format: None,
        },
    ];
    let mut bytes = [0u8; 8192];
    let callback = NativeProcess::import_address(BASE, bytes.len(), 1).unwrap() as u32;
    bytes[0] = 0xb8;
    bytes[1..5].copy_from_slice(&callback.to_le_bytes());
    bytes[5..7].copy_from_slice(&[0xff, 0xe0]); // native comparator tail call
    let mut child = load(&helper, &bytes, &imports);
    let entry = child
        .bind(
            child.import_pointer(0).unwrap(),
            NativeAbi::SystemV,
            sort.parameters,
            sort.result,
        )
        .unwrap();
    let values = [9i32, -3, 12, 0, 4];
    for _ in 0..3 {
        for (index, value) in values.iter().enumerate() {
            child.memory_mut().unwrap()[4096 + index * 4..4100 + index * 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        let mut arguments = [0; 13];
        arguments[..4].copy_from_slice(&[BASE + 4096, values.len() as u64, 4, BASE]);
        let mut comparisons = 0;
        assert_eq!(
            child
                .invoke::<()>(entry, arguments, |call, base, memory| {
                    let NativeRequest::Import(call) = call else {
                        panic!("scalar callback cannot reenter")
                    };

                    comparisons += 1;
                    assert_eq!(call.number, 200);
                    assert!(call.function);
                    let value = |address: u64| {
                        let at = (address - base) as usize;
                        i32::from_le_bytes(memory[at..at + 4].try_into().unwrap())
                    };
                    Ok(NativeAction::<()>::Return(
                        (value(call.arguments[0]) - value(call.arguments[1])) as i64 as u64,
                    ))
                })
                .unwrap(),
            0
        );
        assert!(comparisons >= 4);
        let actual: Vec<i32> = child.memory().unwrap()[4096..4116]
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(actual, [-3, 0, 4, 9, 12]);
    }
}

#[test]
fn i386_notifications_deny_clone_and_timeout_reaps_only_the_owned_child() {
    use std::os::unix::process::ExitStatusExt;
    let Some(helper) = helper() else {
        return;
    };
    // fork is denied by the same default-deny guard, then returns -EPERM.
    let mut bytes = [0u8; 8192];
    bytes[..8].copy_from_slice(&[0xb8, 2, 0, 0, 0, 0xcd, 0x80, 0xc3]);
    let mut child = load(&helper, &bytes, &[]);
    let entry = child
        .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::I32)
        .unwrap();
    assert_eq!(
        child
            .invoke::<()>(entry, [0; 13], |_, _, _| Err(NativeError::Callback))
            .unwrap(),
        u64::MAX
    );
    let pid = child.pid();
    // Hardware faults terminate and reap this child without creating a retry
    // handler. Its parked bytes remain inspectable only after the reap.
    child.memory_mut().unwrap()[..2].copy_from_slice(&[0x0f, 0x0b]);
    assert!(
        matches!(child.invoke::<()>(entry,[0;13],|_,_,_|Err(NativeError::Callback)),Err(NativeError::Exited(status)) if status.signal()==Some(4))
    );
    assert_eq!(child.pid(), 0);
    assert!(child.memory_mut().is_ok());
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    bytes[..2].copy_from_slice(&[0xeb, 0xfe]);
    let mut child = NativeProcess::load_with_helper(
        NativeImage {
            base: BASE,
            pointer_bytes: 4,
            bytes: &bytes,
            regions: &REGIONS,
            imports: &[],
            timeout: Duration::from_millis(50),
            runtime: None,
        },
        &helper,
    )
    .unwrap();
    let pid = child.pid();
    let entry = child
        .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::Word)
        .unwrap();
    assert!(matches!(
        child.invoke::<()>(entry, [0; 13], |_, _, _| Err(NativeError::Callback)),
        Err(NativeError::Timeout)
    ));
    assert_eq!(child.pid(), 0);
    assert!(child.memory().is_ok());
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[test]
fn all_thirteen_double_arguments_and_x87_results_keep_the_full_stack_extent() {
    let Some(helper) = helper() else {
        return;
    };
    let mut bytes = [0u8; 8192];
    // fld qword [esp+100] reads the last of twenty-six four-byte cells.
    bytes[..5].copy_from_slice(&[0xdd, 0x44, 0x24, 100, 0xc3]);
    let mut child = load(&helper, &bytes, &[]);
    let entry = child
        .bind(
            BASE,
            NativeAbi::SystemV,
            &[NativeScalar::Double; 13],
            NativeScalar::Double,
        )
        .unwrap();
    for value in [
        0.0f64,
        -0.0,
        1234.5,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::from_bits(0x7ff8_1234_5678_abcd),
    ] {
        let mut words = std::array::from_fn(|i| (i as f64 + 0.25).to_bits());
        words[12] = value.to_bits();
        assert_eq!(
            child
                .invoke::<()>(entry, words, |_, _, _| Err(NativeError::Callback))
                .unwrap(),
            value.to_bits()
        );
    }
}

#[test]
fn pointer_list_filters_use_four_byte_banks_and_native_callbacks() {
    use qa_platform::native::NativeListFilter;
    let Some(helper) = helper() else {
        return;
    };
    let filter = NativeListFilter {
        buffer: BASE + 4608,
        capacity: 3,
        depth: 2,
        list: 2,
        limit: 3,
        callback: 5,
        data: 6,
        keep: 0,
        end: 64,
    };
    assert_eq!(filter.byte_length_for_width(4), Some(24));
    let imports = [
        NativeImport {
            trap: false,
            number: 23,
            abi: NativeAbi::SystemV,
            parameters: &[NativeScalar::Word; 7],
            result: NativeScalar::Word,
            filter: Some(filter),
            format: None,
        },
        NativeImport {
            trap: false,
            number: 777,
            abi: NativeAbi::SystemV,
            parameters: &[NativeScalar::Word; 2],
            result: NativeScalar::U32,
            filter: None,
            format: None,
        },
    ];
    let mut bytes = [0u8; 8192];
    let observer = NativeProcess::import_address(BASE, bytes.len(), 1).unwrap() as u32;
    bytes[0] = 0xb8;
    bytes[1..5].copy_from_slice(&observer.to_le_bytes());
    bytes[5..7].copy_from_slice(&[0xff, 0xe0]);
    let mut child = load(&helper, &bytes, &imports);
    let entry = child
        .bind(
            child.import_pointer(0).unwrap(),
            NativeAbi::SystemV,
            &[NativeScalar::Word; 7],
            NativeScalar::Word,
        )
        .unwrap();
    let mut arguments = [0; 13];
    arguments[..7].copy_from_slice(&[0, 0, BASE + 4096, 3, 1, BASE, 0x1234_5678]);
    let mut visited = Vec::new();
    child.memory_mut().unwrap()[4096..4112].fill(0xcc);
    assert_eq!(
        child
            .invoke::<()>(entry, arguments, |call, base, memory| {
                let NativeRequest::Import(call) = call else {
                    panic!("scalar callback cannot reenter")
                };

                if call.number == 23 {
                    assert_eq!(call.arguments[2], filter.buffer);
                    assert_eq!(call.arguments[3], 3);
                    assert_eq!(&call.arguments[5..7], &[0, 0]);
                    let at = (filter.buffer - base) as usize;
                    for (index, value) in [101u32, 202, 303].iter().enumerate() {
                        memory[at + index * 4..at + index * 4 + 4]
                            .copy_from_slice(&value.to_le_bytes());
                    }
                    Ok(NativeAction::<()>::Return(3))
                } else {
                    assert_eq!(call.number, 777);
                    assert_eq!(call.arguments[1], 0x1234_5678);
                    visited.push(call.arguments[0]);
                    Ok(NativeAction::<()>::Return(u64::from(
                        call.arguments[0] == 202,
                    )))
                }
            })
            .unwrap(),
        2
    );
    assert_eq!(visited, [101, 202, 303]);
    assert_eq!(
        &child.memory().unwrap()[4096..4104],
        &[101u32.to_le_bytes(), 303u32.to_le_bytes()].concat()
    );
    assert_eq!(&child.memory().unwrap()[4104..4112], &[0xcc; 8]);
}

#[test]
fn cdecl_formatted_imports_read_promoted_doubles_and_long_long_once() {
    use qa_platform::native::NativeFormat;
    let Some(helper) = helper() else {
        return;
    };
    let parameters = [NativeScalar::I32, NativeScalar::Word];
    let imports = [NativeImport {
        trap: false,
        number: 24,
        abi: NativeAbi::SystemV,
        parameters: &parameters,
        result: NativeScalar::Void,
        filter: None,
        format: Some(NativeFormat {
            buffer: BASE + 4608,
            capacity: 128,
            format: 1,
        }),
    }];
    let mut bytes = [0u8; 8192];
    let text = b"v=%lld f=%.2f p=%p\0";
    bytes[4096..4096 + text.len()].copy_from_slice(text);
    let long = 0x1234_5678_9abc_def0u64;
    let float = 2.75f64.to_bits();
    let cells = [
        2u32,
        (BASE + 4096) as u32,
        long as u32,
        (long >> 32) as u32,
        float as u32,
        (float >> 32) as u32,
        0x1234_5678,
    ];
    let mut code = Vec::new();
    for cell in cells.into_iter().rev() {
        code.push(0x68);
        code.extend(cell.to_le_bytes());
    }
    code.push(0xb8);
    code.extend(
        (NativeProcess::import_address(BASE, bytes.len(), 0).unwrap() as u32).to_le_bytes(),
    );
    code.extend([0xff, 0xd0, 0x83, 0xc4, 28, 0xc3]);
    bytes[..code.len()].copy_from_slice(&code);
    let mut child = load(&helper, &bytes, &imports);
    let entry = child
        .bind(BASE, NativeAbi::SystemV, &[], NativeScalar::Void)
        .unwrap();
    let mut calls = 0;
    assert_eq!(
        child
            .invoke::<()>(entry, [0; 13], |call, base, memory| {
                let NativeRequest::Import(call) = call else {
                    panic!("scalar callback cannot reenter")
                };

                calls += 1;
                assert_eq!(call.number, 24);
                assert_eq!(call.arguments[0], 2);
                assert_eq!(call.arguments[1], BASE + 4608);
                let at = (call.arguments[1] - base) as usize;
                let end = memory[at..].iter().position(|&b| b == 0).unwrap();
                assert_eq!(
                    &memory[at..at + end],
                    b"v=1311768467463790320 f=2.75 p=0x12345678"
                );
                Ok(NativeAction::<()>::Return(0))
            })
            .unwrap(),
        0
    );
    assert_eq!(calls, 1);
}

#[test]
fn unverified_32_bit_runtime_pointer_tables_are_rejected_at_load() {
    let Some(helper) = helper() else {
        return;
    };
    let function = qa_platform::native::runtime::FUNCTIONS
        .iter()
        .find(|f| f.name == b"localeconv")
        .unwrap();
    let imports = [NativeImport {
        trap: false,
        number: function.number,
        abi: NativeAbi::SystemV,
        parameters: function.parameters,
        result: function.result,
        filter: None,
        format: None,
    }];
    let bytes = [0u8; 8192];
    let result = NativeProcess::load_with_helper(
        NativeImage {
            base: BASE,
            pointer_bytes: 4,
            bytes: &bytes,
            regions: &REGIONS,
            imports: &imports,
            timeout: Duration::from_secs(3),
            runtime: None,
        },
        &helper,
    );
    assert!(matches!(result, Err(NativeError::Unsupported)));
}

#[test]
fn atoll_requires_a_64_bit_integer_gate_while_atoi_remains_admitted() {
    let Some(helper) = helper() else {
        return;
    };
    let mut bytes = [0u8; 8192];
    bytes[4096..4107].copy_from_slice(b"4294967296\0");
    for name in [b"atoll".as_slice(), b"atoi".as_slice()] {
        let function = qa_platform::native::runtime::FUNCTIONS
            .iter()
            .find(|f| f.name == name)
            .unwrap();
        let imports = [NativeImport {
            trap: false,
            number: function.number,
            abi: NativeAbi::SystemV,
            parameters: function.parameters,
            result: function.result,
            filter: None,
            format: None,
        }];
        let result = NativeProcess::load_with_helper(
            NativeImage {
                base: BASE,
                pointer_bytes: 4,
                bytes: &bytes,
                regions: &REGIONS,
                imports: &imports,
                timeout: Duration::from_secs(3),
                runtime: None,
            },
            &helper,
        );
        if name == b"atoll" {
            // 4294967296 must be EDX:EAX = 1:0. Word is a pointer-width
            // result and cannot represent this i386 long-long return.
            assert!(matches!(result, Err(NativeError::Unsupported)));
        } else {
            let mut child = result.unwrap();
            let entry = child
                .bind(
                    child.import_pointer(0).unwrap(),
                    NativeAbi::SystemV,
                    function.parameters,
                    function.result,
                )
                .unwrap();
            child.memory_mut().unwrap()[4096..4101].copy_from_slice(b"-123\0");
            let mut arguments = [0; 13];
            arguments[0] = BASE + 4096;
            assert_eq!(
                child
                    .invoke::<()>(entry, arguments, |_, _, _| Err(NativeError::Callback))
                    .unwrap(),
                (-123i32) as i64 as u64
            );
        }
    }
}
