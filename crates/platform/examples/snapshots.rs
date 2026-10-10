//! Developer-only native snapshot byte/word comparison and heap/timing probe.
use qa_network::{
    message::{Encoding, Reader, Writer},
    snapshots::{self, Entity, Frame, Ring},
    states::{ENTITY_WORDS, PLAYER_WORDS},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

struct Case<const P: usize, const E: usize> {
    distance: u8,
    prime: bool,
    flags: u8,
    areas: Vec<u8>,
    old_player: [u32; P],
    player: [u32; P],
    old: Vec<Entity<E>>,
    entities: Vec<Entity<E>>,
    server: Ring<P, E>,
    client: Ring<P, E>,
    expected: Vec<u8>,
}
type WriteFrame<const P: usize, const E: usize> =
    fn(
        &mut Writer<'_>,
        &Ring<P, E>,
        u32,
        Option<u32>,
    ) -> Result<(), qa_network::commands::packet::Error>;
type ReadFrame<const P: usize, const E: usize> =
    fn(
        &mut Reader<'_>,
        &mut Ring<P, E>,
        u32,
        u32,
    ) -> Result<bool, qa_network::commands::packet::Error>;
struct Codec<const P: usize, const E: usize> {
    encoding: Encoding,
    opcode: u32,
    end: u32,
    write: WriteFrame<P, E>,
    read: ReadFrame<P, E>,
}
const Q3: Codec<PLAYER_WORDS, ENTITY_WORDS> = Codec {
    encoding: Encoding::Q3,
    opcode: 7,
    end: 8,
    write: snapshots::write_q3,
    read: snapshots::read_q3,
};
const Q2: Codec<{ qa_network::states::Q2_PLAYER_WORDS }, { qa_network::states::Q2_ENTITY_WORDS }> =
    Codec {
        encoding: Encoding::Bytes,
        opcode: 20,
        end: 6,
        write: |w, r, n, d| snapshots::write_q2(w, r, n, d, 16),
        read: |r, s, _, _| snapshots::read_q2(r, s),
    };
fn words<const N: usize>(reader: &mut Reader<'_>) -> Result<[u32; N], String> {
    let mut words = [0; N];
    for word in &mut words {
        *word = reader.read_bits(32).map_err(|e| e.to_string())?;
    }
    Ok(words)
}
fn entities<const E: usize>(
    reader: &mut Reader<'_>,
    count: usize,
) -> Result<Vec<Entity<E>>, String> {
    (0..count)
        .map(|_| {
            Ok(Entity {
                number: reader.read_bits(16).map_err(|e| e.to_string())?,
                words: words(reader)?,
            })
        })
        .collect()
}
fn load<const P: usize, const E: usize>(
    input: &[u8],
    retained: u64,
) -> Result<Vec<Case<P, E>>, String> {
    let mut reader = Reader::new(input, Encoding::Bytes);
    let count = reader.read_bits(32).map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    for _ in 0..count {
        let distance = reader.read_bits(8).map_err(|e| e.to_string())? as u8;
        let prime = reader.read_bits(8).map_err(|e| e.to_string())? != 0;
        let flags = reader.read_bits(8).map_err(|e| e.to_string())? as u8;
        let mut areas = vec![0; reader.read_bits(8).map_err(|e| e.to_string())? as usize];
        let a = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let b = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let baseline_count = reader.read_bits(16).map_err(|e| e.to_string())? as usize;
        let old_player = words(&mut reader)?;
        let player = words(&mut reader)?;
        reader.read_data(&mut areas).map_err(|e| e.to_string())?;
        let baselines = entities(&mut reader, baseline_count)?;
        let old = entities(&mut reader, a)?;
        let entities = entities(&mut reader, b)?;
        let mut server = Ring::load(64, 1024, 32, None).map_err(|e| format!("{e:?}"))?;
        let mut client = Ring::load(64, 1024, 32, Some(retained)).map_err(|e| format!("{e:?}"))?;
        for baseline in baselines {
            if !server.set_baseline(baseline.number, &baseline.words)
                || !client.set_baseline(baseline.number, &baseline.words)
            {
                return Err("native baseline".into());
            }
        }
        cases.push(Case {
            distance,
            prime,
            flags,
            areas,
            old_player,
            player,
            old,
            entities,
            server,
            client,
            expected: Vec::new(),
        });
    }
    if reader.byte_position() != input.len() {
        return Err("fixture boundary".into());
    }
    Ok(cases)
}

fn run<const P: usize, const E: usize>(
    case: &mut Case<P, E>,
    codec: &Codec<P, E>,
    base: u32,
    out: &mut [u8; 32768],
) -> Result<usize, String> {
    let mut wire = [0; 8192];
    case.server
        .store(Frame {
            sequence: base,
            time: 100,
            command: 12,
            flags: case.flags,
            areas: &case.areas,
            player: &case.old_player,
            entities: &case.old,
        })
        .map_err(|e| format!("{e:?}"))?;
    if case.prime {
        let mut writer = Writer::new(&mut wire, codec.encoding);
        (codec.write)(&mut writer, &case.server, base, None).map_err(|e| format!("{e:?}"))?;
        writer.write_bits(codec.end, 8).map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), codec.encoding);
        reader.read_bits(8).map_err(|e| e.to_string())?;
        if !(codec.read)(&mut reader, &mut case.client, base, 12).map_err(|e| format!("{e:?}"))? {
            return Err("initial full snapshot".into());
        }
        if reader.read_bits(8).map_err(|e| e.to_string())? != codec.end {
            return Err("full EOF".into());
        }
    }
    let sequence = base + u32::from(case.distance.max(1));
    case.server
        .store(Frame {
            sequence,
            time: 300,
            command: 12,
            flags: case.flags,
            areas: &case.areas,
            player: &case.player,
            entities: &case.entities,
        })
        .map_err(|e| format!("{e:?}"))?;
    let mut writer = Writer::new(&mut wire, codec.encoding);
    (codec.write)(
        &mut writer,
        &case.server,
        sequence,
        (case.distance != 0).then_some(base),
    )
    .map_err(|e| format!("{e:?}"))?;
    writer.write_bits(codec.end, 8).map_err(|e| e.to_string())?;
    let bits = writer.bit_position() as u32;
    let n = writer.size();
    let mut reader = Reader::new(writer.bytes(), codec.encoding);
    if reader.read_bits(8).map_err(|e| e.to_string())? != codec.opcode {
        return Err("snapshot opcode".into());
    }
    let accepted =
        (codec.read)(&mut reader, &mut case.client, sequence, 12).map_err(|e| format!("{e:?}"))?;
    if reader.read_bits(8).map_err(|e| e.to_string())? != codec.end {
        return Err("snapshot EOF".into());
    }
    let mut output = Writer::new(out, Encoding::Bytes);
    for value in [
        bits,
        n as u32,
        u32::from(accepted),
        reader.bit_position() as u32,
    ] {
        output.write_bits(value, 32).map_err(|e| e.to_string())?;
    }
    output
        .write_data(writer.bytes())
        .map_err(|e| e.to_string())?;
    if accepted {
        let frame = case.client.frame(sequence).ok_or("accepted snapshot")?;
        for value in [
            frame.time as u32,
            frame.command,
            u32::from(frame.flags),
            frame.areas.len() as u32,
            frame.entities.len() as u32,
        ] {
            output.write_bits(value, 32).map_err(|e| e.to_string())?;
        }
        output.write_data(frame.areas).map_err(|e| e.to_string())?;
        for &word in frame.player {
            output.write_bits(word, 32).map_err(|e| e.to_string())?;
        }
        for entity in frame.entities {
            output
                .write_bits(entity.number, 32)
                .map_err(|e| e.to_string())?;
            for word in entity.words {
                output.write_bits(word, 32).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(output.size())
}

fn timing(fixture: &str, oracle: &str) -> Result<(), String> {
    let input = std::fs::read(fixture).map_err(|e| e.to_string())?;
    let oracle = std::fs::read(oracle).map_err(|e| e.to_string())?;
    let mut cases = load(&input, 1920)?;
    let mut output = [0; 32768];
    let mut offset = 0;
    for case in &mut cases {
        let n = run(case, &Q3, 1, &mut output)?;
        if oracle.get(offset..offset + n) != Some(&output[..n]) {
            return Err("native snapshot fidelity at load".into());
        }
        case.expected.extend_from_slice(&output[..n]);
        offset += n;
    }
    if cases.len() < 16 || offset != oracle.len() {
        return Err("oracle boundary".into());
    }
    cases.truncate(16);
    let storage_bytes: usize = cases
        .iter()
        .map(|case| case.server.allocated_bytes() + case.client.allocated_bytes())
        .sum();
    check_heap_counter()?;
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    let mut checks = 0;
    let mut output_bytes = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        for case in &mut cases {
            let n = run(
                std::hint::black_box(case),
                &Q3,
                (frame as u32 + 1) * 64 + 1,
                &mut output,
            )?;
            if output[..n] != case.expected {
                return Err("native snapshot fidelity".into());
            }
            checks += 1;
            output_bytes += n;
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let actual = allocations::end_frame();
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            counts.allocations += actual.allocations;
            counts.reallocations += actual.reallocations;
            counts.requested_bytes += actual.requested_bytes;
        }
    }
    if counts != allocations::Counts::default() || checks != 10560 {
        return Err(format!("snapshot allocation/count gate {counts:?}"));
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 native snapshot pairs per iteration with original-C byte/word checks; no transport/host/gameplay\",\"warmup\":60,\"frames\":600,\"checks\":{checks},\"output_bytes_including_decoded_words\":{output_bytes},\"snapshot_storage_bytes\":{storage_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
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
    if args.get(1).is_some_and(|a| a == "--timing") {
        return timing(
            args.get(2).ok_or("fixture path")?,
            args.get(3).ok_or("oracle path")?,
        );
    }
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    if args.get(1).is_some_and(|a| a == "--q2") {
        return compare(&input, &Q2, 896);
    }
    compare(&input, &Q3, 1920)
}

fn compare<const P: usize, const E: usize>(
    input: &[u8],
    codec: &Codec<P, E>,
    retained: u64,
) -> Result<(), String> {
    let mut cases = load(input, retained)?;
    check_heap_counter()?;
    let mut stdout = std::io::stdout().lock();
    let mut output = [0; 32768];
    for case in &mut cases {
        allocations::begin_frame();
        let n = run(case, codec, 1, &mut output)?;
        let heap = allocations::end_frame();
        if heap != allocations::Counts::default() {
            return Err(format!("snapshot comparison heap gate {heap:?}"));
        }
        stdout.write_all(&output[..n]).map_err(|e| e.to_string())?;
    }
    eprintln!(
        "{{\"scope\":\"caller Rust heap during native frame encode/decode; no host/workers/driver\",\"cases\":{},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0}}",
        cases.len()
    );
    Ok(())
}

fn check_heap_counter() -> Result<(), String> {
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("heap positive control".into());
    }
    Ok(())
}
