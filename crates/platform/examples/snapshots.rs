//! Developer-only native snapshot byte/word comparison and heap/timing probe.
use qa_network::{
    message::{Encoding, Reader, Writer},
    snapshots::{self, Entity, Frame, Q3Ring},
    states::{ENTITY_WORDS, PLAYER_WORDS},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

struct Case {
    distance: u8,
    prime: bool,
    flags: u8,
    areas: Vec<u8>,
    old_player: [u32; PLAYER_WORDS],
    player: [u32; PLAYER_WORDS],
    old: Vec<Entity<ENTITY_WORDS>>,
    entities: Vec<Entity<ENTITY_WORDS>>,
    server: Q3Ring,
    client: Q3Ring,
    expected: Vec<u8>,
}
fn words<const N: usize>(reader: &mut Reader<'_>) -> Result<[u32; N], String> {
    let mut words = [0; N];
    for word in &mut words {
        *word = reader.read_bits(32).map_err(|e| e.to_string())?;
    }
    Ok(words)
}
fn entities(reader: &mut Reader<'_>, count: usize) -> Result<Vec<Entity<ENTITY_WORDS>>, String> {
    (0..count)
        .map(|_| {
            Ok(Entity {
                number: reader.read_bits(16).map_err(|e| e.to_string())?,
                words: words(reader)?,
            })
        })
        .collect()
}
fn load(input: &[u8]) -> Result<Vec<Case>, String> {
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
        let mut server = Q3Ring::load(64, 1024, 32, None).map_err(|e| format!("{e:?}"))?;
        let mut client = Q3Ring::load(64, 1024, 32, Some(1920)).map_err(|e| format!("{e:?}"))?;
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

fn run(case: &mut Case, base: u32, out: &mut [u8; 32768]) -> Result<usize, String> {
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
        let mut writer = Writer::new(&mut wire, Encoding::Q3);
        snapshots::write_q3(&mut writer, &case.server, base, None).map_err(|e| format!("{e:?}"))?;
        writer.write_bits(8, 8).map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
        reader.read_bits(8).map_err(|e| e.to_string())?;
        if !snapshots::read_q3(&mut reader, &mut case.client, base, 12)
            .map_err(|e| format!("{e:?}"))?
        {
            return Err("initial full snapshot".into());
        }
        if reader.read_bits(8).map_err(|e| e.to_string())? != 8 {
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
    let mut writer = Writer::new(&mut wire, Encoding::Q3);
    snapshots::write_q3(
        &mut writer,
        &case.server,
        sequence,
        (case.distance != 0).then_some(base),
    )
    .map_err(|e| format!("{e:?}"))?;
    writer.write_bits(8, 8).map_err(|e| e.to_string())?;
    let bits = writer.bit_position() as u32;
    let n = writer.size();
    let mut reader = Reader::new(writer.bytes(), Encoding::Q3);
    if reader.read_bits(8).map_err(|e| e.to_string())? != 7 {
        return Err("snapshot opcode".into());
    }
    let accepted = snapshots::read_q3(&mut reader, &mut case.client, sequence, 12)
        .map_err(|e| format!("{e:?}"))?;
    if reader.read_bits(8).map_err(|e| e.to_string())? != 8 {
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
    let mut cases = load(&input)?;
    let mut output = [0; 32768];
    let mut offset = 0;
    for case in &mut cases {
        let n = run(case, 1, &mut output)?;
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
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("heap positive control".into());
    }
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
    let mut cases = load(&input)?;
    let mut stdout = std::io::stdout().lock();
    let mut output = [0; 32768];
    for case in &mut cases {
        let n = run(case, 1, &mut output)?;
        stdout.write_all(&output[..n]).map_err(|e| e.to_string())?;
    }
    Ok(())
}
