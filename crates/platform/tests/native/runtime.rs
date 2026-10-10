use super::{BASE, REGIONS, configured_child};
use crate::native::{
    NativeAbi, NativeError, NativeImport, NativeProcess, NativeRegion, NativeScalar,
    runtime::{FIRST, FUNCTIONS, RuntimeConfig},
};
use std::time::Duration;

fn runtime_child() -> NativeProcess {
    let abi = NativeAbi::Microsoft;
    let mut imports: Vec<_> = FUNCTIONS
        .iter()
        .map(|f| NativeImport {
            number: f.number,
            abi,
            parameters: f.parameters,
            result: f.result,
        })
        .collect();
    imports.push(NativeImport {
        number: 777,
        abi,
        parameters: &[NativeScalar::I32],
        result: NativeScalar::I32,
    });
    let mut bytes = vec![0; 12288 + crate::native::runtime::THREAD_BYTES];
    let gate = NativeProcess::import_address(BASE, bytes.len(), imports.len() - 1).unwrap();
    // Real native callbacks import their tags through the kernel-stop boundary.
    for (at, tag) in [(256, 2u32), (320, 7), (384, 9)] {
        bytes[at] = 0xb9;
        bytes[at + 1..at + 5].copy_from_slice(&tag.to_le_bytes());
        bytes[at + 5..at + 11].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
        bytes[at + 11..at + 19].copy_from_slice(&gate.to_le_bytes());
    }
    bytes[512..517].copy_from_slice(&[0x8b, 0x01, 0x2b, 0x02, 0xc3]);
    let alloc = NativeProcess::import_address(
        BASE,
        bytes.len(),
        FUNCTIONS
            .iter()
            .position(|f| f.number == FIRST + 418)
            .unwrap(),
    )
    .unwrap();
    // A destructor reenters FlsAlloc, then reports the returned index through
    // the engine import. Reusing its own slot proves retirement preceded it.
    let mut destructor = vec![0x48, 0x83, 0xec, 40, 0x31, 0xc9, 0x48, 0xb8];
    destructor.extend(alloc.to_le_bytes());
    destructor.extend([0xff, 0xd0, 0x89, 0xc1, 0x48, 0x83, 0xc4, 40, 0x48, 0xb8]);
    destructor.extend(gate.to_le_bytes());
    destructor.extend([0xff, 0xe0]);
    bytes[768..768 + destructor.len()].copy_from_slice(&destructor);
    let config = RuntimeConfig {
        base: BASE + 4096,
        heap_bytes: 4096,
        teb: Some(BASE + 12288),
    };
    config.prepare_crt(&mut bytes[4096..8192]).unwrap();
    configured_child(
        &bytes,
        &[
            REGIONS[0],
            NativeRegion {
                length: bytes.len() - 4096,
                ..REGIONS[1]
            },
        ],
        &imports,
        Duration::from_secs(3),
        Some(config),
    )
    .unwrap()
}

fn call(process: &mut NativeProcess, trace: &mut Vec<i32>, number: u32, args: &[u64]) -> u64 {
    let ordinal = FUNCTIONS
        .iter()
        .position(|f| f.number == FIRST + number)
        .unwrap();
    let function = &FUNCTIONS[ordinal];
    let entry = process
        .bind(
            process.import_pointer(ordinal).unwrap(),
            NativeAbi::Microsoft,
            function.parameters,
            function.result,
        )
        .unwrap();
    let mut words = [0; 13];
    words[..args.len()].copy_from_slice(args);
    process
        .invoke(entry, words, |request, _, _| {
            assert_eq!(request.number, 777);
            let tag = request.arguments[0] as i32;
            trace.push(tag);
            Ok(if tag == 7 { 7 } else { 0 })
        })
        .unwrap()
}

#[test]
fn windows_thread_storage_has_real_stack_bounds_and_child_local_gs() {
    use crate::native::runtime::{STATIC_TLS_OFFSET, THREAD_BYTES, TLS_DATA_OFFSET};
    let teb = BASE + 12288;
    let mut bytes = vec![0; 12288 + THREAD_BYTES];
    // mov rax,gs:[rcx]; ret, and mov rax,rsp; ret. These run only in the child.
    bytes[..5].copy_from_slice(&[0x65, 0x48, 0x8b, 0x01, 0xc3]);
    bytes[32..36].copy_from_slice(&[0x48, 0x89, 0xe0, 0xc3]);
    let tls = teb + TLS_DATA_OFFSET as u64;
    let vector = 12288 + STATIC_TLS_OFFSET;
    bytes[vector..vector + 8].copy_from_slice(&tls.to_le_bytes());
    let template = 12288 + TLS_DATA_OFFSET;
    bytes[template..template + 8].copy_from_slice(b"TLS one\0");
    let config = RuntimeConfig {
        base: BASE + 4096,
        heap_bytes: 4096,
        teb: Some(teb),
    };
    config.prepare_crt(&mut bytes[4096..8192]).unwrap();
    let regions = [
        REGIONS[0],
        NativeRegion {
            length: bytes.len() - 4096,
            ..REGIONS[1]
        },
    ];
    let imports: Vec<_> = FUNCTIONS
        .iter()
        .map(|f| NativeImport {
            number: f.number,
            abi: NativeAbi::Microsoft,
            parameters: f.parameters,
            result: f.result,
        })
        .collect();
    let launch = |bytes: &[u8]| {
        configured_child(
            bytes,
            &regions,
            &imports,
            Duration::from_secs(3),
            Some(config),
        )
        .unwrap()
    };
    let mut first = launch(&bytes);
    bytes[template..template + 8].copy_from_slice(b"TLS two\0");
    let second = launch(&bytes);
    let entry = first
        .bind(
            BASE,
            NativeAbi::Microsoft,
            &[NativeScalar::Word],
            NativeScalar::Word,
        )
        .unwrap();
    let mut read_gs = |offset| {
        let mut args = [0; 13];
        args[0] = offset;
        first
            .invoke(entry, args, |_, _, _| panic!("no engine import"))
            .unwrap()
    };
    assert_eq!(read_gs(0), u64::MAX);
    assert_eq!(read_gs(0x30), teb);
    assert_eq!(read_gs(0x40), 1);
    assert_eq!(read_gs(0x48), 1);
    assert_eq!(read_gs(0x58), teb + STATIC_TLS_OFFSET as u64);
    assert_eq!(read_gs(0x60), teb + 0x2000);
    assert_eq!(read_gs(0x1780), teb + 0x5000);
    let top = read_gs(8);
    let bottom = read_gs(16);
    let stack = first
        .bind(BASE + 32, NativeAbi::Microsoft, &[], NativeScalar::Word)
        .unwrap();
    let rsp = first
        .invoke(stack, [0; 13], |_, _, _| panic!("no engine import"))
        .unwrap();
    assert!(bottom < rsp && rsp < top);
    assert_eq!(top - bottom, 8 * 1024 * 1024);
    assert_eq!(
        &first.memory().unwrap()[template..template + 8],
        b"TLS one\0"
    );
    assert_eq!(
        &second.memory().unwrap()[template..template + 8],
        b"TLS two\0"
    );
    assert_eq!(call(&mut first, &mut Vec::new(), 14, &[4096]), BASE + 8192);
    assert_eq!(call(&mut first, &mut Vec::new(), 14, &[1]), 0);
    assert_eq!(
        &first.memory().unwrap()[12288 + 0x68..12288 + 0x6c],
        &[0; 4]
    );
}

#[test]
fn rejected_child_runtime_calls_keep_the_import_name_and_isolate_the_child() {
    let mut failed = runtime_child();
    let mut healthy = runtime_child();
    let ordinal = FUNCTIONS
        .iter()
        .position(|f| f.number == FIRST + 300)
        .unwrap();
    let entry = failed
        .bind(
            failed.import_pointer(ordinal).unwrap(),
            NativeAbi::Microsoft,
            &[NativeScalar::I32],
            NativeScalar::I32,
        )
        .unwrap();
    let mut arguments = [0; 13];
    arguments[0] = 3;
    let result = failed.invoke(entry, arguments, |_, _, _| {
        panic!("runtime rejection must not dispatch an engine import")
    });
    assert!(
        matches!(
            result,
            Err(NativeError::RuntimeImport("_configure_narrow_argv"))
        ),
        "{result:?}"
    );
    assert_eq!(failed.pid(), 0);
    assert!(failed.memory_mut().is_ok());
    assert_eq!(
        call(&mut healthy, &mut Vec::new(), 6, &[0.5f64.to_bits()]),
        0.5f64.sin().to_bits()
    );
}

#[test]
fn windows_tls_and_fls_slots_remain_independent_and_reuse_native_indices() {
    let mut first = runtime_child();
    let mut second = runtime_child();
    let mut trace = Vec::new();
    for index in 0..1088 {
        assert_eq!(call(&mut first, &mut trace, 414, &[]), index);
        assert_eq!(call(&mut first, &mut trace, 417, &[index, index + 17]), 1);
    }
    assert_eq!(call(&mut first, &mut trace, 414, &[]), u64::from(u32::MAX));
    assert_eq!(call(&mut second, &mut trace, 414, &[]), 0);
    assert_eq!(call(&mut second, &mut trace, 416, &[0]), 0);
    for index in [0, 63, 64, 1087] {
        assert_eq!(call(&mut first, &mut trace, 416, &[index]), index + 17);
        assert_eq!(call(&mut first, &mut trace, 400, &[]), 0);
    }
    assert_eq!(call(&mut first, &mut trace, 416, &[1088]), 0);
    assert_eq!(call(&mut first, &mut trace, 400, &[]), 87);
    assert_eq!(call(&mut first, &mut trace, 415, &[64]), 1);
    assert_eq!(call(&mut first, &mut trace, 414, &[]), 64);
    assert_eq!(call(&mut first, &mut trace, 416, &[64]), 0);
    assert_eq!(call(&mut first, &mut trace, 418, &[BASE + 768]), 0);
    assert_eq!(call(&mut first, &mut trace, 421, &[0, 77]), 1);
    assert_eq!(call(&mut first, &mut trace, 420, &[0]), 77);
    assert_eq!(call(&mut first, &mut trace, 419, &[0]), 1);
    assert_eq!(trace, [0]);
    assert_eq!(call(&mut first, &mut trace, 420, &[0]), 0);
    assert_eq!(call(&mut first, &mut trace, 400, &[]), 0);
    assert_eq!(call(&mut first, &mut trace, 419, &[0]), 1);
    assert_eq!(call(&mut first, &mut trace, 420, &[0]), 0);
    assert_eq!(call(&mut first, &mut trace, 400, &[]), 87);
    for index in 0..128 {
        assert_eq!(call(&mut first, &mut trace, 418, &[0]), index);
    }
    assert_eq!(call(&mut first, &mut trace, 418, &[0]), u64::from(u32::MAX));
    assert_eq!(call(&mut first, &mut trace, 400, &[]), 8);
    assert_eq!(call(&mut second, &mut trace, 418, &[0]), 0);
    assert_eq!(call(&mut second, &mut trace, 420, &[0]), 0);
}

#[test]
fn windows_sync_and_environment_services_keep_the_c_state_changes() {
    let mut process = runtime_child();
    let mut trace = Vec::new();
    assert_eq!(call(&mut process, &mut trace, 402, &[]), 1);
    assert_eq!(call(&mut process, &mut trace, 403, &[]), 1);
    assert_eq!(call(&mut process, &mut trace, 404, &[]), u64::MAX);
    assert_eq!(call(&mut process, &mut trace, 401, &[123]), 0);
    assert_eq!(call(&mut process, &mut trace, 400, &[]), 123);
    assert_eq!(call(&mut process, &mut trace, 412, &[BASE]), 1);
    assert_eq!(call(&mut process, &mut trace, 412, &[BASE + 1]), 0);
    assert_eq!(call(&mut process, &mut trace, 400, &[]), 87);
    process.memory_mut().unwrap()[12288 + crate::native::runtime::STATIC_TLS_OFFSET
        ..12296 + crate::native::runtime::STATIC_TLS_OFFSET]
        .copy_from_slice(&(BASE + 6200).to_le_bytes());
    assert_eq!(call(&mut process, &mut trace, 412, &[BASE]), 0);
    assert_eq!(call(&mut process, &mut trace, 413, &[BASE + 256]), 0);
    assert_eq!(call(&mut process, &mut trace, 413, &[0]), BASE + 256);
    let lock = BASE + 6400;
    assert_eq!(call(&mut process, &mut trace, 405, &[lock]), 0);
    assert_eq!(&process.memory().unwrap()[6400..6408], &1u64.to_le_bytes());
    assert_eq!(call(&mut process, &mut trace, 406, &[lock]), 0);
    assert_eq!(&process.memory().unwrap()[6400..6408], &[0; 8]);
    process.memory_mut().unwrap()[6400..6416].fill(255);
    assert_eq!(call(&mut process, &mut trace, 407, &[lock]), 0);
    assert_eq!(&process.memory().unwrap()[6400..6416], &[0; 16]);
    process.memory_mut().unwrap()[6400..6408]
        .copy_from_slice(&0xffff_ffff_ffff_abcd_u64.to_le_bytes());
    process.memory_mut().unwrap()[6408..6416].copy_from_slice(&(BASE + 6603).to_le_bytes());
    assert_eq!(call(&mut process, &mut trace, 408, &[lock]), BASE + 6592);
    assert_eq!(&process.memory().unwrap()[6400..6408], &[0; 8]);
    assert_eq!(&process.memory().unwrap()[6408..6416], &11u64.to_le_bytes());
    for flag in [0, 6, 10, 40] {
        assert_eq!(
            call(&mut process, &mut trace, 411, &[flag]),
            u64::from(flag == 6 || flag == 10)
        );
    }
    assert_eq!(call(&mut process, &mut trace, 409, &[u64::MAX]), 0);
    assert_eq!(call(&mut process, &mut trace, 410, &[]), 0);
    assert!(trace.is_empty());
}

#[test]
fn windows_time_uses_parked_event_snapshots_and_native_integer_conversions() {
    use crate::native::runtime::TIME_OFFSET;
    use qa_core::sys_events::EventTime;
    let mut process = runtime_child();
    let mut trace = Vec::new();
    let word = |process: &NativeProcess, at| {
        u64::from_le_bytes(process.memory().unwrap()[at..at + 8].try_into().unwrap())
    };
    let origin = word(&process, 4096 + TIME_OFFSET + 8) as i64;
    process.set_event_time(EventTime(2_345_999_999)).unwrap();
    assert_eq!(word(&process, 4096 + TIME_OFFSET), 2_345_999_999);
    assert_eq!(word(&process, 4096 + TIME_OFFSET + 8) as i64, origin + 2345);
    for _ in 0..3 {
        assert_eq!(call(&mut process, &mut trace, 422, &[BASE + 6400]), 1);
        assert_eq!(word(&process, 6400), 2_345_999_999);
    }
    assert_eq!(call(&mut process, &mut trace, 423, &[BASE + 6400]), 1);
    assert_eq!(word(&process, 6400), 1_000_000_000);
    for millis in [-1001i64, -1000, -1, 0, 999, 1000, 1700000000123, i64::MAX] {
        process.memory_mut().unwrap()[4096 + TIME_OFFSET + 8..4096 + TIME_OFFSET + 16]
            .copy_from_slice(&millis.to_le_bytes());
        assert_eq!(
            call(&mut process, &mut trace, 340, &[BASE + 6400]),
            millis.div_euclid(1000) as u64
        );
        assert_eq!(word(&process, 6400), millis.div_euclid(1000) as u64);
        assert_eq!(
            call(&mut process, &mut trace, 340, &[0]),
            millis.div_euclid(1000) as u64
        );
        assert_eq!(
            call(&mut process, &mut trace, 339, &[]),
            (millis as u64).wrapping_mul(10000)
        );
        assert_eq!(call(&mut process, &mut trace, 424, &[BASE + 6400]), 0);
        assert_eq!(
            word(&process, 6400),
            (millis as u64)
                .wrapping_mul(10000)
                .wrapping_add(116444736000000000)
        );
    }
    process.set_event_time(EventTime(4_500_000_000)).unwrap();
    assert_eq!(word(&process, 4096 + TIME_OFFSET + 8) as i64, origin + 4500);
    assert!(trace.is_empty());
}

#[test]
fn crt_math_matches_the_original_c_switch() {
    let mut process = runtime_child();
    let mut trace = Vec::new();
    let mut count = 0;
    for row in include_str!("crt-math.csv")
        .lines()
        .filter(|row| !row.starts_with('#'))
    {
        let values: Vec<_> = row
            .split(',')
            .map(|word| u64::from_str_radix(word, 16).unwrap())
            .collect();
        let number = values[0] as u32;
        let function = FUNCTIONS
            .iter()
            .find(|f| f.number == FIRST + number)
            .unwrap();
        let mut args = [values[1], values[2]];
        if number == 325 {
            args[1] = BASE + 7000;
        }
        let result = call(
            &mut process,
            &mut trace,
            number,
            &args[..function.parameters.len()],
        );
        let expected = if function.result == NativeScalar::I16 {
            values[3] as u16 as i16 as i64 as u64
        } else {
            values[3]
        };
        assert_eq!(result, expected, "{row}");
        if number == 325 {
            assert_eq!(
                u64::from_le_bytes(process.memory().unwrap()[7000..7008].try_into().unwrap()),
                values[4]
            );
        }
        count += 1;
    }
    assert_eq!(count, 357);
    assert!(trace.is_empty());
}

#[test]
fn crt_conversions_match_original_c_values_and_end_pointers() {
    let mut process = runtime_child();
    let mut trace = Vec::new();
    let mut count = 0;
    for row in include_str!("crt-convert.csv")
        .lines()
        .filter(|row| !row.starts_with('#'))
    {
        let fields: Vec<_> = row.split(',').collect();
        let number = u32::from_str_radix(fields[0], 16).unwrap();
        let radix = fields[1].parse::<i32>().unwrap();
        let text: Vec<_> = fields[2]
            .as_bytes()
            .chunks_exact(2)
            .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
            .collect();
        let expected = u64::from_str_radix(fields[3], 16).unwrap();
        let stop = u64::from_str_radix(fields[4], 16).unwrap();
        let error = u32::from_str_radix(fields[5], 16).unwrap();
        let memory = process.memory_mut().unwrap();
        memory[6400..6401 + text.len()].fill(0);
        memory[6400..6400 + text.len()].copy_from_slice(&text);
        memory[4728..4732].fill(0);
        let args = if number == 335 {
            vec![BASE + 6400, BASE + 6300, radix as i64 as u64]
        } else {
            vec![BASE + 6400]
        };
        assert_eq!(
            call(&mut process, &mut trace, number, &args),
            expected,
            "{row}"
        );
        if number == 335 {
            assert_eq!(
                u64::from_le_bytes(process.memory().unwrap()[6300..6308].try_into().unwrap()),
                BASE + 6400 + stop,
                "{row}"
            );
        }
        assert_eq!(
            u32::from_le_bytes(process.memory().unwrap()[4728..4732].try_into().unwrap()),
            error,
            "{row}"
        );
        count += 1;
    }
    assert_eq!(count, 163);
    // Outputs may alias the original string. Parsing must release its native
    // byte borrow before writing the end pointer, exactly as the C copy does.
    process.memory_mut().unwrap()[6400..6407].copy_from_slice(b"123456\0");
    assert_eq!(
        call(
            &mut process,
            &mut trace,
            335,
            &[BASE + 6400, BASE + 6400, 10]
        ),
        123456
    );
    assert_eq!(
        u64::from_le_bytes(process.memory().unwrap()[6400..6408].try_into().unwrap()),
        BASE + 6406
    );
    assert!(trace.is_empty());
}

#[test]
fn crt_search_and_bounded_compare_keep_native_terminator_rules() {
    let mut process = runtime_child();
    let mut trace = Vec::new();
    process.memory_mut().unwrap()[6400..6406].copy_from_slice(b"ababa\0");
    process.memory_mut().unwrap()[6500..6504].copy_from_slice(b"aba\0");
    process.memory_mut().unwrap()[6510..6514].copy_from_slice(b"ba\xff\0");
    assert_eq!(
        call(&mut process, &mut trace, 334, &[BASE + 6400, BASE + 6500]),
        BASE + 6400
    );
    assert_eq!(
        call(&mut process, &mut trace, 334, &[BASE + 6400, BASE + 6501]),
        BASE + 6401
    );
    assert_eq!(
        call(&mut process, &mut trace, 334, &[BASE + 6400, BASE + 6510]),
        0
    );
    assert_eq!(
        call(&mut process, &mut trace, 334, &[BASE + 6400, BASE + 6503]),
        BASE + 6400
    );
    assert_eq!(
        call(&mut process, &mut trace, 334, &[BASE + 6503, BASE + 6400]),
        0
    );
    assert_eq!(
        call(&mut process, &mut trace, 333, &[BASE + 6400, b'b' as u64]),
        BASE + 6401
    );
    assert_eq!(
        call(&mut process, &mut trace, 333, &[BASE + 6400, 0]),
        BASE + 6405
    );
    assert_eq!(
        call(&mut process, &mut trace, 333, &[BASE + 6400, b'x' as u64]),
        0
    );
    assert_eq!(
        call(&mut process, &mut trace, 331, &[BASE + 6510, 0x1ff, 3]),
        BASE + 6512
    );
    assert_eq!(call(&mut process, &mut trace, 331, &[u64::MAX, 0, 0]), 0);
    assert_eq!(
        call(&mut process, &mut trace, 332, &[u64::MAX, u64::MAX, 0]),
        0
    );
    assert_eq!(
        call(
            &mut process,
            &mut trace,
            332,
            &[BASE + 6400, BASE + 6500, 3]
        ),
        0
    );
    assert_eq!(
        call(
            &mut process,
            &mut trace,
            332,
            &[BASE + 6400, BASE + 6500, 4]
        ),
        b'b' as u64
    );
    assert_eq!(
        call(
            &mut process,
            &mut trace,
            332,
            &[BASE + 6503, BASE + 6513, 100000]
        ),
        0
    );
    assert!(trace.is_empty());
}

#[test]
fn crt_startup_teardown_and_sort_use_native_callbacks() {
    let mut process = runtime_child();
    let mut trace = Vec::new();
    let table = BASE + 6000;
    let callbacks = [BASE + 256, 0, BASE + 320, BASE + 384];
    for (index, address) in callbacks.into_iter().enumerate() {
        process.memory_mut().unwrap()[6100 + index * 8..6108 + index * 8]
            .copy_from_slice(&address.to_le_bytes());
    }
    assert_eq!(
        call(&mut process, &mut trace, 309, &[BASE + 6100, BASE + 6132]),
        7
    );
    assert_eq!(trace, [2, 7]);
    trace.clear();
    assert_eq!(
        call(&mut process, &mut trace, 308, &[BASE + 6100, BASE + 6132]),
        0
    );
    assert_eq!(trace, [2, 7, 9]);
    assert_eq!(call(&mut process, &mut trace, 300, &[2]), 0);
    assert_eq!(call(&mut process, &mut trace, 301, &[]), 0);
    assert_eq!(call(&mut process, &mut trace, 302, &[512]), 0);
    assert_eq!(call(&mut process, &mut trace, 303, &[table]), 0);
    trace.clear();
    let mut expected = Vec::new();
    for index in 0..40 {
        let tag = if index % 2 == 0 { 2 } else { 9 };
        expected.push(tag);
        assert_eq!(
            call(
                &mut process,
                &mut trace,
                304,
                &[table, if tag == 2 { BASE + 256 } else { BASE + 384 }]
            ),
            0
        );
    }
    assert_eq!(call(&mut process, &mut trace, 306, &[table]), 0);
    expected.reverse();
    assert_eq!(trace, expected);
    assert_eq!(&process.memory().unwrap()[6000..6024], &[0; 24]);
    trace.clear();
    assert_eq!(call(&mut process, &mut trace, 305, &[BASE + 384]), 0);
    assert_eq!(call(&mut process, &mut trace, 307, &[]), 0);
    assert_eq!(trace, [9]);
    assert_eq!(call(&mut process, &mut trace, 310, &[BASE + 6080]), 0);
    let input: [i32; 10] = [7, 5, -9, 7, 0, 1, -6, 44, -99, 3];
    for (index, value) in input.into_iter().enumerate() {
        process.memory_mut().unwrap()[6400 + index * 4..6404 + index * 4]
            .copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(
        call(
            &mut process,
            &mut trace,
            311,
            &[BASE + 6400, 10, 4, BASE + 512]
        ),
        0
    );
    let sorted: Vec<_> = process.memory().unwrap()[6400..6440]
        .chunks_exact(4)
        .map(|s| i32::from_le_bytes(s.try_into().unwrap()))
        .collect();
    assert_eq!(sorted, [-99, -9, -6, 0, 1, 3, 5, 7, 7, 44]);
    assert_eq!(call(&mut process, &mut trace, 329, &[]), BASE + 4096 + 632);
    assert_eq!(call(&mut process, &mut trace, 330, &[]), BASE + 4096 + 416);
    assert_eq!(
        u64::from_le_bytes(process.memory().unwrap()[4512..4520].try_into().unwrap()),
        BASE + 4700
    );
    assert_eq!(&process.memory().unwrap()[4700..4702], b".\0");
}

#[test]
fn crt_catalog_windows_provider_aliases_are_exact() {
    for (name, library) in [
        (b"memcpy".as_slice(), b"VCRUNTIME140.dll".as_slice()),
        (b"sinf", b"api-ms-win-crt-math-l1-1-0.dll"),
        (
            b"_configure_narrow_argv",
            b"api-ms-win-crt-runtime-l1-1-0.dll",
        ),
    ] {
        let function = FUNCTIONS.iter().find(|f| f.name == name).unwrap();
        assert!(function.windows_provider(library));
        assert!(function.windows_provider(b"UCRTBASE.DLL"));
        assert!(function.windows_provider(b"msvcrt.dll"));
        assert!(!function.windows_provider(b"MSVCP140.dll"));
        assert!(!function.windows_provider(b"arbitrary.dll"));
    }
}
