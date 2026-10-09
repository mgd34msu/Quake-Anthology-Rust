//! Original-function codec fixture and a pinned 16-peer allocation/timing probe.
use qa_network::message::{Encoding, Reader, Writer};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

#[derive(Clone, Copy)]
struct Field {
    width: u8,
    value: u32,
}
fn fields(mode: u8, seed: u32) -> [Field; 64] {
    let mut state = seed;
    std::array::from_fn(|index| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        Field {
            width: if mode == 0 {
                [8, 16, 32][index % 3]
            } else {
                (index % 32 + 1) as u8
            },
            value: state,
        }
    })
}
fn encoding(mode: u8) -> Result<Encoding, String> {
    match mode {
        0 => Ok(Encoding::Bytes),
        1 => Ok(Encoding::Bits),
        2 => Ok(Encoding::Q3),
        _ => Err("invalid fixture encoding".into()),
    }
}
fn masked(field: Field) -> u32 {
    (u64::from(field.value) & ((1u64 << field.width) - 1)) as u32
}
fn fixture() -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    for mode in 0..3 {
        for seed in 1..=256 {
            let values = fields(mode, seed);
            stdout
                .write_all(&(values.len() as u32).to_le_bytes())
                .and_then(|_| stdout.write_all(&[mode]))
                .map_err(|e| e.to_string())?;
            for field in values {
                stdout
                    .write_all(&[field.width])
                    .and_then(|_| stdout.write_all(&field.value.to_le_bytes()))
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}
fn encode_fixture() -> Result<(), String> {
    let mut fixture = Vec::new();
    std::io::stdin()
        .read_to_end(&mut fixture)
        .map_err(|e| e.to_string())?;
    let mut input = fixture.as_slice();
    let mut out = std::io::stdout().lock();
    while !input.is_empty() {
        let Some(header) = input.get(..5) else {
            return Err("fixture header truncated".into());
        };
        let count = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
        if count > 512 {
            return Err("fixture field capacity".into());
        }
        let mode = encoding(header[4])?;
        input = &input[5..];
        let size = count * 5;
        let Some(fields) = input.get(..size) else {
            return Err("fixture fields truncated".into());
        };
        input = &input[size..];
        let mut bytes = [0; 8192];
        let mut writer = Writer::new(&mut bytes, mode);
        for chunk in fields.as_chunks::<5>().0 {
            writer
                .write_bits(
                    u32::from_le_bytes([chunk[1], chunk[2], chunk[3], chunk[4]]),
                    chunk[0],
                )
                .map_err(|e| e.to_string())?;
        }
        out.write_all(&(writer.bit_position() as u32).to_le_bytes())
            .and_then(|_| out.write_all(&(writer.size() as u32).to_le_bytes()))
            .and_then(|_| out.write_all(writer.bytes()))
            .map_err(|e| e.to_string())?;
        let mut reader = Reader::new(writer.bytes(), mode);
        for chunk in fields.as_chunks::<5>().0 {
            let value = reader.read_bits(chunk[0]).map_err(|e| e.to_string())?;
            out.write_all(&value.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn timing() -> Result<(), String> {
    let values = std::array::from_fn::<_, 16, _>(|peer| fields((peer % 3) as u8, peer as u32 + 1));
    let mut packets = [[0u8; 1400]; 16];
    let modes = std::array::from_fn::<_, 16, _>(|peer| encoding((peer % 3) as u8));
    let mut samples = [0u64; 600];
    let mut totals = allocations::Counts::default();
    let mut packet_bytes = 0u64;
    let mut decoded = 0u64;
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    let positive = allocations::end_frame();
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        let mut bytes_this_frame = 0;
        for peer in 0..16 {
            let mode = *modes[peer].as_ref().map_err(Clone::clone)?;
            let mut writer = Writer::new(&mut packets[peer], mode);
            for field in values[peer] {
                writer
                    .write_bits(field.value, field.width)
                    .map_err(|e| e.to_string())?;
            }
            bytes_this_frame += writer.size() as u64;
            let mut reader = Reader::new(writer.bytes(), mode);
            for field in values[peer] {
                let value = reader.read_bits(field.width).map_err(|e| e.to_string())?;
                if value != masked(field) {
                    return Err("decoded field differs".into());
                }
                decoded += 1;
            }
        }
        let elapsed = watch.elapsed().as_nanos() as u64;
        let count = allocations::end_frame();
        if frame == 0 {
            packet_bytes = bytes_this_frame;
        }
        if bytes_this_frame != packet_bytes {
            return Err("packet sizes differ".into());
        }
        if frame >= 60 {
            samples[frame - 60] = elapsed;
            totals.allocations += count.allocations;
            totals.reallocations += count.reallocations;
            totals.requested_bytes += count.requested_bytes;
        }
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless16mixed native message encode/decode peers; no channel, workers, gameplay or native heap\",\"warmup\":60,\"frames\":600,\"peers\":16,\"fields_per_peer\":64,\"decoded_fields\":{decoded},\"packet_bytes_per_frame\":{packet_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
        totals.allocations,
        totals.reallocations,
        totals.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) / 2.0,
        samples[593]
    );
    if totals.allocations != 0 || totals.reallocations != 0 || totals.requested_bytes != 0 {
        return Err("message frame heap activity".into());
    }
    Ok(())
}
fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref() {
        Some("--fixture") => fixture(),
        Some("--encode") => encode_fixture(),
        None => timing(),
        _ => Err("use --fixture, --encode, or no arguments for timing".into()),
    }
}
