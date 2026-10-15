//! Real x86-64 imports exercised only by the owned child, with both ABIs.
use super::{BASE, REGIONS, image_child};
use crate::native::{
    NativeAbi, NativeError, NativeFormat, NativeImport, NativeProcess, NativeScalar,
    runtime::{FIRST, FUNCTIONS},
};
use crate::native::{NativeAction, NativeRequest};
use std::time::Duration;

#[derive(Clone, Copy)]
enum Value {
    Word(u64),
    Double(f64),
}
fn move_word(code: &mut Vec<u8>, register: &[u8], word: u64) {
    code.extend(register);
    code.extend(word.to_le_bytes());
}
fn call_code(abi: NativeAbi, pointer: u64, values: &[Value]) -> Vec<u8> {
    let registers: &[&[u8]] = if abi == NativeAbi::SystemV {
        &[
            &[0x48, 0xbf],
            &[0x48, 0xbe],
            &[0x48, 0xba],
            &[0x48, 0xb9],
            &[0x49, 0xb8],
            &[0x49, 0xb9],
        ]
    } else {
        &[&[0x48, 0xb9], &[0x48, 0xba], &[0x49, 0xb8], &[0x49, 0xb9]]
    };
    let mut gp = 0;
    let mut fp = 0;
    let mut stack = Vec::new();
    let mut body = Vec::new();
    for &value in values {
        let word = match value {
            Value::Word(word) => word,
            Value::Double(value) => value.to_bits(),
        };
        if abi == NativeAbi::SystemV && matches!(value, Value::Double(_)) {
            if fp < 8 {
                move_word(&mut body, &[0x48, 0xb8], word);
                body.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0 | (fp as u8) << 3]);
                fp += 1;
            } else {
                stack.push(word);
            }
        } else if gp < registers.len() {
            move_word(&mut body, registers[gp], word);
            if abi == NativeAbi::Microsoft && matches!(value, Value::Double(_)) {
                move_word(&mut body, &[0x48, 0xb8], word);
                body.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0 | (gp as u8) << 3]);
            }
            gp += 1;
        } else {
            stack.push(word);
        }
    }
    let shadow = if abi == NativeAbi::Microsoft { 32 } else { 0 };
    // Reserve enough for the generic import gate's initial raw capture too.
    let minimum = if abi == NativeAbi::Microsoft { 104 } else { 56 };
    let reserve = (shadow + stack.len() * 8).max(minimum).div_ceil(16) * 16 + 8;
    let mut code = vec![0x48, 0x81, 0xec];
    code.extend((reserve as u32).to_le_bytes());
    for (index, word) in stack.iter().enumerate() {
        move_word(&mut code, &[0x48, 0xb8], *word);
        code.extend([0x48, 0x89, 0x84, 0x24]);
        code.extend(((shadow + index * 8) as u32).to_le_bytes());
    }
    code.extend(body);
    move_word(&mut code, &[0x49, 0xbb], pointer);
    code.push(0xb8);
    code.extend((fp as u32).to_le_bytes());
    code.extend([0x41, 0xff, 0xd3, 0x48, 0x81, 0xc4]);
    code.extend((reserve as u32).to_le_bytes());
    code.push(0xc3);
    code
}
fn fixture(imports: &[NativeImport<'_>]) -> NativeProcess {
    let bytes = vec![0; 8192];
    image_child(&bytes, &REGIONS, imports, Duration::from_secs(3)).unwrap()
}
fn run(
    process: &mut NativeProcess,
    abi: NativeAbi,
    ordinal: usize,
    values: &[Value],
    callback: impl FnMut(NativeRequest<()>, u64, &mut [u8]) -> Result<NativeAction<()>, NativeError>,
) -> Result<u64, NativeError> {
    let code = call_code(abi, process.import_pointer(ordinal).unwrap(), values);
    process.memory_mut().unwrap()[..code.len()].copy_from_slice(&code);
    let entry = process.bind(BASE, abi, &[], NativeScalar::I32).unwrap();
    process.invoke::<()>(entry, [0; 13], callback)
}
fn store(process: &mut NativeProcess, offset: usize, bytes: &[u8]) -> u64 {
    process.memory_mut().unwrap()[offset..offset + bytes.len()].copy_from_slice(bytes);
    BASE + offset as u64
}
fn output(process: &NativeProcess, offset: usize) -> &[u8] {
    qa_core::text::terminated(&process.memory().unwrap()[offset..]).unwrap()
}

#[test]
fn classic_print_adapters_capture_raw_text_promoted_doubles_and_named_arguments() {
    use NativeScalar::{I32, Word};
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        for (number, parameters, named) in [
            (0, &[I32, Word][..], vec![Value::Word(2)]),
            (1, &[Word][..], vec![]),
            (
                2,
                &[Word, I32, Word][..],
                vec![Value::Word(BASE + 7000), Value::Word(3)],
            ),
            (3, &[Word, Word][..], vec![Value::Word(BASE + 7000)]),
            (7, &[Word][..], vec![]),
        ] {
            let format_index = parameters.len() - 1;
            let imports = [NativeImport {
                trap: false,
                number,
                abi,
                parameters,
                result: NativeScalar::Void,
                filter: None,
                format: Some(NativeFormat {
                    buffer: BASE + 6144,
                    capacity: if number == 0 { 2048 } else { 1024 },
                    format: format_index as u8,
                }),
            }];
            let mut process = fixture(&imports);
            let raw = store(&mut process, 5000, b"\xffbrown\0");
            let format = store(
                &mut process,
                4096,
                b"raw=%s %*.*f %d %lld %lu %zu %#x %.0d %%\0",
            );
            let mut values = named;
            values.push(Value::Word(format));
            values.extend([
                Value::Word(raw),
                Value::Word(8),
                Value::Word(2),
                Value::Double(1.25),
                Value::Word((-7i32) as u64),
                Value::Word(i64::MAX as u64),
                Value::Word(0x1_0000_0001),
                Value::Word(u64::MAX),
                Value::Word(42),
                Value::Word(0),
            ]);
            let expected = if abi == NativeAbi::SystemV {
                b"raw=\xffbrown     1.25 -7 9223372036854775807 4294967297 18446744073709551615 0x2a  %".as_slice()
            } else {
                b"raw=\xffbrown     1.25 -7 9223372036854775807 1 18446744073709551615 0x2a  %"
            };
            let mut callbacks = 0;
            run(&mut process, abi, 0, &values, |call, base, memory| {
                let NativeRequest::Import(call) = call else {
                    panic!("scalar callback cannot reenter")
                };

                callbacks += 1;
                assert_eq!(call.number, number);
                assert_eq!(call.arguments[format_index], base + 6144);
                assert_eq!(
                    qa_core::text::terminated(&memory[6144..]).unwrap(),
                    expected
                );
                if number == 2 {
                    assert_eq!(call.arguments[..2], [BASE + 7000, 3]);
                }
                Ok(NativeAction::<()>::Return(0))
            })
            .unwrap();
            assert_eq!(callbacks, 1, "{abi:?} ordinal {number}");
        }
    }
}
#[test]
fn format_arguments_continue_beyond_the_thirteen_word_packet_and_eight_fp_registers() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let imports = [NativeImport {
            trap: false,
            number: 1,
            abi,
            parameters: &[NativeScalar::Word],
            result: NativeScalar::Void,
            filter: None,
            format: Some(NativeFormat {
                buffer: BASE + 6144,
                capacity: 1024,
                format: 0,
            }),
        }];
        let mut process = fixture(&imports);
        let format = store(
            &mut process,
            4096,
            b"%d %d %d %d %d %d %d %d %d %d %d %d %d %d %d\0",
        );
        let mut values = vec![Value::Word(format)];
        values.extend((1..=15).map(Value::Word));
        run(&mut process, abi, 0, &values, |_, _, _| {
            Ok(NativeAction::<()>::Return(0))
        })
        .unwrap();
        assert_eq!(
            output(&process, 6144),
            b"1 2 3 4 5 6 7 8 9 10 11 12 13 14 15"
        );
        let format = store(
            &mut process,
            4096,
            b"%.1f %.1f %.1f %.1f %.1f %.1f %.1f %.1f %.1f %d\0",
        );
        let mut values = vec![Value::Word(format)];
        values.extend((1..=9).map(|v| Value::Double(v as f64 + 0.5)));
        values.push(Value::Word(19));
        run(&mut process, abi, 0, &values, |_, _, _| {
            Ok(NativeAction::<()>::Return(0))
        })
        .unwrap();
        assert_eq!(
            output(&process, 6144),
            b"1.5 2.5 3.5 4.5 5.5 6.5 7.5 8.5 9.5 19"
        );
    }
}
#[test]
fn format_output_is_bounded_and_bad_guest_reads_trap_before_publication() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        for (format, argument) in [
            (b"%1024d\0".as_slice(), Value::Word(1)),
            (b"%s\0", Value::Word(BASE + 12288)),
            (b"%ls\0", Value::Word(BASE + 5000)),
            (b"%Lf\0", Value::Double(1.0)),
            (b"%2$d\0", Value::Word(2)),
        ] {
            let imports = [NativeImport {
                trap: false,
                number: 1,
                abi,
                parameters: &[NativeScalar::Word],
                result: NativeScalar::Void,
                filter: None,
                format: Some(NativeFormat {
                    buffer: BASE + 6144,
                    capacity: 1024,
                    format: 0,
                }),
            }];
            let mut process = fixture(&imports);
            let pointer = store(&mut process, 4096, format);
            let mut calls = 0;
            let result = run(
                &mut process,
                abi,
                0,
                &[Value::Word(pointer), argument],
                |_, _, _| {
                    calls += 1;
                    Ok(NativeAction::<()>::Return(0))
                },
            );
            assert!(
                matches!(result, Err(NativeError::ImportTrap { ordinal: 0, .. })),
                "{abi:?} {format:?}: {result:?}"
            );
            assert_eq!(calls, 0);
        }
    }
}
fn crt_import(abi: NativeAbi, number: u32) -> NativeImport<'static> {
    let function = FUNCTIONS.iter().find(|f| f.number == number).unwrap();
    NativeImport {
        trap: false,
        number,
        abi,
        parameters: function.parameters,
        result: function.result,
        filter: None,
        format: None,
    }
}
#[test]
fn sprintf_and_snprintf_reuse_the_child_formatter_without_engine_calls() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let imports = [crt_import(abi, FIRST + 18), crt_import(abi, FIRST + 19)];
        let mut process = fixture(&imports);
        let format = store(&mut process, 4096, b"%+08.2f %#o %.3g %.1e\0");
        let args = [
            Value::Word(BASE + 6144),
            Value::Word(format),
            Value::Double(1.25),
            Value::Word(8),
            Value::Double(12345.0),
            Value::Double(0.125),
        ];
        let length = run(&mut process, abi, 0, &args, |_, _, _| {
            panic!("CRT formatting must stay in child")
        })
        .unwrap();
        assert_eq!(output(&process, 6144), b"+0001.25 010 1.23e+04 1.2e-01");
        assert_eq!(length, output(&process, 6144).len() as u64);
        let args = [
            Value::Word(BASE + 6144),
            Value::Word(5),
            Value::Word(format),
            Value::Double(1.25),
            Value::Word(8),
            Value::Double(12345.0),
            Value::Double(0.125),
        ];
        let length = run(&mut process, abi, 1, &args, |_, _, _| {
            panic!("CRT formatting must stay in child")
        })
        .unwrap();
        assert_eq!(length, 29);
        assert_eq!(output(&process, 6144), b"+000");
        let mut args = args;
        args[0] = Value::Word(u64::MAX);
        args[1] = Value::Word(0);
        assert_eq!(
            run(&mut process, abi, 1, &args, |_, _, _| panic!(
                "CRT formatting must stay in child"
            ))
            .unwrap(),
            29
        );
    }
}
#[test]
fn vsprintf_reads_the_native_va_list_in_each_abi() {
    for abi in [NativeAbi::SystemV, NativeAbi::Microsoft] {
        let imports = [crt_import(abi, FIRST + 20)];
        let mut process = fixture(&imports);
        let format = store(&mut process, 4096, b"%.2f %s %d%n\0");
        let string = store(&mut process, 5000, b"\xffok\0");
        let list = BASE + 5504;
        if abi == NativeAbi::SystemV {
            let mut header = Vec::new();
            header.extend(16u32.to_le_bytes());
            header.extend(48u32.to_le_bytes());
            header.extend((BASE + 5800).to_le_bytes());
            header.extend((BASE + 5600).to_le_bytes());
            store(&mut process, 5504, &header);
            store(&mut process, 5600 + 16, &string.to_le_bytes());
            store(&mut process, 5600 + 24, &((-9i32) as u64).to_le_bytes());
            store(&mut process, 5600 + 32, &(BASE + 6000).to_le_bytes());
            store(&mut process, 5600 + 48, &2.5f64.to_bits().to_le_bytes());
        } else {
            let mut slots = Vec::new();
            for word in [2.5f64.to_bits(), string, (-9i32) as u64, BASE + 6000] {
                slots.extend(word.to_le_bytes());
            }
            store(&mut process, 5504, &slots);
        }
        let args = [
            Value::Word(BASE + 6144),
            Value::Word(format),
            Value::Word(list),
        ];
        assert_eq!(
            run(&mut process, abi, 0, &args, |_, _, _| panic!(
                "va_list is child-local"
            ))
            .unwrap(),
            11
        );
        assert_eq!(output(&process, 6144), b"2.50 \xffok -9");
        assert_eq!(&process.memory().unwrap()[6000..6004], &11u32.to_le_bytes());
    }
}
