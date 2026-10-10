use super::{BASE, REGIONS, configured_child};
use crate::native::{
    NativeAbi, NativeImport, NativeProcess, NativeRegion, NativeScalar,
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
    let mut bytes = vec![0; 12288];
    let gate = NativeProcess::import_address(BASE, bytes.len(), imports.len() - 1).unwrap();
    // Real native callbacks import their tags through the kernel-stop boundary.
    for (at, tag) in [(256, 2u32), (320, 7), (384, 9)] {
        bytes[at] = 0xb9;
        bytes[at + 1..at + 5].copy_from_slice(&tag.to_le_bytes());
        bytes[at + 5..at + 11].copy_from_slice(&[0xff, 0x25, 0, 0, 0, 0]);
        bytes[at + 11..at + 19].copy_from_slice(&gate.to_le_bytes());
    }
    bytes[512..517].copy_from_slice(&[0x8b, 0x01, 0x2b, 0x02, 0xc3]);
    let config = RuntimeConfig {
        base: BASE + 4096,
        heap_bytes: 4096,
    };
    config.prepare_crt(&mut bytes[4096..8192]).unwrap();
    configured_child(
        &bytes,
        &[
            REGIONS[0],
            NativeRegion {
                length: 8192,
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
