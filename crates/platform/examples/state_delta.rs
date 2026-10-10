//! Developer-only original-C delta record fidelity and allocation/timing probe.
use qa_network::{
    commands::delta as command_delta,
    message::{Encoding, Reader, Writer},
    states::{
        self, ENTITY_WORDS, NQ_ENTITY_WORDS, NQ_PLAYER_WORDS, PLAYER_WORDS, Q2_ENTITY_WORDS,
        Q2_PLAYER_WORDS, QW_ENTITY_WORDS, QW_PLAYER_WORDS,
    },
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

struct Case {
    mode: u8,
    flags: u8,
    number: u32,
    from: [u32; PLAYER_WORDS],
    to: [u32; PLAYER_WORDS],
    expected: Encoded,
    bytes: [u8; 1400],
}
#[derive(PartialEq, Eq)]
struct Encoded {
    bits: u32,
    length: usize,
    decoded: [u32; PLAYER_WORDS],
    number: u32,
    removed: bool,
}
impl Default for Encoded {
    fn default() -> Self {
        Self {
            bits: 0,
            length: 0,
            decoded: [0; PLAYER_WORDS],
            number: 0,
            removed: false,
        }
    }
}
fn words(reader: &mut Reader<'_>) -> Result<[u32; PLAYER_WORDS], String> {
    let mut words = [0; PLAYER_WORDS];
    for word in &mut words {
        *word = reader.read_bits(32).map_err(|e| e.to_string())?;
    }
    Ok(words)
}
fn case(reader: &mut Reader<'_>) -> Result<Case, String> {
    Ok(Case {
        mode: reader.read_bits(8).map_err(|e| e.to_string())? as u8,
        flags: reader.read_bits(8).map_err(|e| e.to_string())? as u8,
        number: reader.read_bits(16).map_err(|e| e.to_string())?,
        from: words(reader)?,
        to: words(reader)?,
        expected: Encoded::default(),
        bytes: [0; 1400],
    })
}
fn encode(case: &Case, bytes: &mut [u8; 1400]) -> Result<Encoded, String> {
    let encoding = if case.mode >= 2 {
        Encoding::Bytes
    } else {
        Encoding::Q3
    };
    let mut writer = Writer::new(bytes, encoding);
    let mut result = Encoded::default();
    match case.mode {
        0 => {
            let from: &[u32; ENTITY_WORDS] = case.from[..ENTITY_WORDS]
                .try_into()
                .map_err(|_| "entity words")?;
            let to: &[u32; ENTITY_WORDS] = case.to[..ENTITY_WORDS]
                .try_into()
                .map_err(|_| "entity words")?;
            let sent = states::write_q3_entity(
                &mut writer,
                case.number,
                from,
                if case.flags & 2 != 0 { None } else { Some(to) },
                case.flags & 1 != 0,
            )
            .map_err(|e| e.to_string())?;
            result.number = case.number;
            if sent {
                let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
                let decoded =
                    states::read_q3_entity(&mut reader, from).map_err(|e| e.to_string())?;
                result.number = u32::from(decoded.number);
                result.removed = decoded.words.is_none();
                if let Some(words) = decoded.words {
                    result.decoded[..ENTITY_WORDS].copy_from_slice(&words);
                }
            } else {
                result.decoded[..ENTITY_WORDS].copy_from_slice(from);
            }
        }
        1 => {
            states::write_q3_player(&mut writer, &case.from, &case.to)
                .map_err(|e| e.to_string())?;
            let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
            result.decoded =
                states::read_q3_player(&mut reader, &case.from).map_err(|e| e.to_string())?;
        }
        2 => {
            let from: &[u32; Q2_PLAYER_WORDS] = case.from[..Q2_PLAYER_WORDS]
                .try_into()
                .map_err(|_| "Q2 player words")?;
            let to: &[u32; Q2_PLAYER_WORDS] = case.to[..Q2_PLAYER_WORDS]
                .try_into()
                .map_err(|_| "Q2 player words")?;
            // These words already project the native short/byte pmove ABI.
            states::write_q2_player(&mut writer, from, to).map_err(|e| e.to_string())?;
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            result.decoded[..Q2_PLAYER_WORDS].copy_from_slice(
                &states::read_q2_player(&mut reader, from).map_err(|e| e.to_string())?,
            );
        }
        3 => {
            let from: &[u32; QW_ENTITY_WORDS] = case.from[..QW_ENTITY_WORDS]
                .try_into()
                .map_err(|_| "QW entity words")?;
            let to: &[u32; QW_ENTITY_WORDS] = case.to[..QW_ENTITY_WORDS]
                .try_into()
                .map_err(|_| "QW entity words")?;
            result.number = case.number;
            let sent = states::write_qw_entity(
                &mut writer,
                case.number,
                from,
                if case.flags & 2 != 0 { None } else { Some(to) },
                case.flags & 1 != 0,
            )
            .map_err(|e| e.to_string())?;
            if sent {
                let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
                let decoded =
                    states::read_qw_entity(&mut reader, from).map_err(|e| e.to_string())?;
                result.number = u32::from(decoded.number);
                result.removed = decoded.words.is_none();
                if let Some(words) = decoded.words {
                    result.decoded[..QW_ENTITY_WORDS].copy_from_slice(&words);
                }
            } else {
                result.decoded[..QW_ENTITY_WORDS].copy_from_slice(from);
            }
        }
        4 => {
            let from: &[u32; Q2_ENTITY_WORDS] = case.from[..Q2_ENTITY_WORDS]
                .try_into()
                .map_err(|_| "Q2 entity words")?;
            let to: &[u32; Q2_ENTITY_WORDS] = case.to[..Q2_ENTITY_WORDS]
                .try_into()
                .map_err(|_| "Q2 entity words")?;
            result.number = case.number;
            let sent = states::write_q2_entity(
                &mut writer,
                case.number,
                from,
                if case.flags & 2 != 0 { None } else { Some(to) },
                case.flags & 1 != 0,
                case.flags & 4 != 0,
            )
            .map_err(|e| e.to_string())?;
            if sent {
                let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
                let decoded =
                    states::read_q2_entity(&mut reader, from).map_err(|e| e.to_string())?;
                result.number = u32::from(decoded.number);
                result.removed = decoded.words.is_none();
                if let Some(words) = decoded.words {
                    result.decoded[..Q2_ENTITY_WORDS].copy_from_slice(&words);
                }
            } else {
                result.decoded[..Q2_ENTITY_WORDS].copy_from_slice(from);
            }
        }
        5 => {
            let from: &[u32; NQ_ENTITY_WORDS] = case.from[..NQ_ENTITY_WORDS]
                .try_into()
                .map_err(|_| "NQ baseline words")?;
            let to: &[u32; NQ_ENTITY_WORDS] = case.to[..NQ_ENTITY_WORDS]
                .try_into()
                .map_err(|_| "NQ entity words")?;
            states::write_nq_entity(&mut writer, case.number, from, to, case.flags & 4 != 0)
                .map_err(|e| e.to_string())?;
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            let decoded = states::read_nq_entity(&mut reader, from).map_err(|e| e.to_string())?;
            result.number = u32::from(decoded.number);
            if let Some(words) = decoded.words {
                result.decoded[..NQ_ENTITY_WORDS].copy_from_slice(&words);
            }
        }
        6 => {
            let to: &[u32; NQ_PLAYER_WORDS] = case.to[..NQ_PLAYER_WORDS]
                .try_into()
                .map_err(|_| "NQ player words")?;
            states::write_nq_player(&mut writer, to).map_err(|e| e.to_string())?;
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            result.decoded[..NQ_PLAYER_WORDS].copy_from_slice(
                &states::read_nq_player(&mut reader, case.flags & 4 != 0)
                    .map_err(|e| e.to_string())?,
            );
        }
        7 => {
            let to: &[u32; QW_PLAYER_WORDS] = case.to[..QW_PLAYER_WORDS]
                .try_into()
                .map_err(|_| "QW player words")?;
            let old_command: &[u32; 11] = case.from[14..25]
                .try_into()
                .map_err(|_| "QW prior command")?;
            let new_command: &[u32; 11] = case.to[14..25]
                .try_into()
                .map_err(|_| "QW player command")?;
            states::write_qw_player(
                &mut writer,
                case.number,
                to,
                command_delta::qw_from_words(new_command),
            )
            .map_err(|e| e.to_string())?;
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            let decoded = states::read_qw_player(&mut reader, case.from[8], |_| {
                command_delta::qw_from_words(old_command)
            })
            .map_err(|e| e.to_string())?;
            result.number = u32::from(decoded.number);
            result.decoded[..QW_PLAYER_WORDS].copy_from_slice(&decoded.words);
            result.decoded[14..25].copy_from_slice(&command_delta::qw_words(decoded.command));
        }
        8 | 10 => {
            let from = case.from[..states::Q2_RR_STATS]
                .try_into()
                .map_err(|_| "RR stats")?;
            let to = case.to[..states::Q2_RR_STATS]
                .try_into()
                .map_err(|_| "RR stats")?;
            if case.mode == 8 {
                states::write_q2_rr_stats(&mut writer, from, to)
            } else {
                states::write_q2_kex_stats(&mut writer, from, to)
            }
            .map_err(|e| e.to_string())?;
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            result.decoded[..states::Q2_RR_STATS].copy_from_slice(
                &if case.mode == 8 {
                    states::read_q2_rr_stats(&mut reader, from)
                } else {
                    states::read_q2_kex_stats(&mut reader, from)
                }
                .map_err(|e| e.to_string())?,
            );
        }
        9 => {
            let from = states::Q2ReproPlayer {
                words: std::array::from_fn(|i| case.from[i]),
                stats: std::array::from_fn(|i| case.from[43 + i]),
            };
            let to = states::Q2ReproPlayer {
                words: std::array::from_fn(|i| case.to[i]),
                stats: std::array::from_fn(|i| case.to[43 + i]),
            };
            // Cold comparison envelope: native frame extra flags precede the
            // ordinary player body only in this fixture stream.
            writer.write_bits(0, 8).map_err(|e| e.to_string())?;
            let extra = states::write_q2_repro_player(&mut writer, &from, &to)
                .map_err(|e| e.to_string())?;
            let mut reader = Reader::new(&writer.bytes()[1..], Encoding::Bytes);
            let decoded = states::read_q2_repro_player(&mut reader, &from, extra)
                .map_err(|e| e.to_string())?;
            result.decoded[..43].copy_from_slice(&decoded.words);
            result.decoded[43..107].copy_from_slice(&decoded.stats);
            result.bits = writer.bit_position() as u32;
            result.length = writer.size();
            bytes[0] = extra;
            return Ok(result);
        }
        _ => return Err("state dialect".into()),
    }
    result.bits = writer.bit_position() as u32;
    result.length = writer.size();
    Ok(result)
}
fn compare() -> Result<(), String> {
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let mut reader = Reader::new(&input, Encoding::Bytes);
    let mut output = std::io::stdout().lock();
    let mut bytes = [0; 1400];
    while reader.byte_position() < input.len() {
        let result = encode(&case(&mut reader)?, &mut bytes)?;
        output
            .write_all(&result.bits.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&(result.length as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&bytes[..result.length])
            .map_err(|e| e.to_string())?;
        for word in result.decoded {
            output
                .write_all(&word.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
        output
            .write_all(&result.number.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&[u8::from(result.removed)])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn timing(fixture: &str, original: &str, heap_only: bool) -> Result<(), String> {
    let input = std::fs::read(fixture).map_err(|e| e.to_string())?;
    let oracle = std::fs::read(original).map_err(|e| e.to_string())?;
    let mut reader = Reader::new(&input, Encoding::Bytes);
    let mut expected = Reader::new(&oracle, Encoding::Bytes);
    let mut cases = Vec::new();
    while reader.byte_position() < input.len() {
        let mut case = case(&mut reader)?;
        case.expected.bits = expected.read_bits(32).map_err(|e| e.to_string())?;
        case.expected.length = expected.read_bits(32).map_err(|e| e.to_string())? as usize;
        let Some(bytes) = case.bytes.get_mut(..case.expected.length) else {
            return Err("oracle capacity".into());
        };
        expected.read_data(bytes).map_err(|e| e.to_string())?;
        case.expected.decoded = words(&mut expected)?;
        case.expected.number = expected.read_bits(32).map_err(|e| e.to_string())?;
        case.expected.removed = expected.read_bits(8).map_err(|e| e.to_string())? != 0;
        cases.push(case);
    }
    if cases.is_empty() || expected.byte_position() != oracle.len() {
        return Err("oracle boundary".into());
    }
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut bytes = [0; 1400];
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    let mut checks = 0;
    let mut wire_bytes = 0;
    let mut mode_checks = [0u64; 11];
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = (!heap_only).then(Stopwatch::start);
        for peer in 0..16 {
            let offset = frame * 16 + peer;
            let case = &cases[(if heap_only { offset * 257 } else { offset }) % cases.len()];
            let result = encode(std::hint::black_box(case), &mut bytes)?;
            if result != case.expected || bytes[..result.length] != case.bytes[..result.length] {
                return Err("native state fidelity".into());
            }
            checks += 1;
            mode_checks[case.mode as usize] += 1;
            wire_bytes += result.length;
        }
        let elapsed = watch.map(|watch| watch.elapsed().as_nanos() as u64);
        let actual = allocations::end_frame();
        if frame >= 60 {
            if let Some(elapsed) = elapsed {
                samples[frame - 60] = elapsed;
            }
            counts.allocations += actual.allocations;
            counts.reallocations += actual.reallocations;
            counts.requested_bytes += actual.requested_bytes;
        }
    }
    if counts != allocations::Counts::default() || checks != 10560 {
        return Err(format!("state allocation/count gate {counts:?}"));
    }
    if heap_only {
        for (mode, count) in mode_checks.iter().enumerate().skip(8) {
            if cases.iter().any(|case| case.mode as usize == mode) && *count == 0 {
                return Err("rerelease records not exercised".into());
            }
        }
        println!(
            "{{\"scope\":\"native state record encode/decode and original-C byte/word fidelity; caller Rust thread, no snapshot packets, workers, physical transport or gameplay\",\"warmup\":60,\"frames\":600,\"checks\":{checks},\"mode_checks_including_warmup\":{mode_checks:?},\"wire_bytes\":{wire_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"timing_run\":false}}",
            counts.allocations, counts.reallocations, counts.requested_bytes
        );
        return Ok(());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 native state records per iteration, encode/decode and original-C byte/word fidelity; no snapshot packets or gameplay\",\"warmup\":60,\"frames\":600,\"checks\":{checks},\"wire_bytes\":{wire_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        counts.allocations,
        counts.reallocations,
        counts.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) * 0.5,
        samples[593]
    );
    Ok(())
}
fn main() -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).is_some_and(|s| s == "--compare") {
        compare()
    } else if args.len() == 4 && matches!(args[1].as_str(), "--timing" | "--heap-only") {
        timing(&args[2], &args[3], args[1] == "--heap-only")
    } else {
        Err("--compare or --timing/--heap-only fixture original".into())
    }
}
