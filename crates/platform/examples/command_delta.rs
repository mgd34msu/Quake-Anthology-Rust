//! Original-C record comparisons and pinned field-table codec measurements.
use qa_core::primitives::Vec3;
use qa_network::{
    commands::{Q2Cmd, Q3Cmd, QwCmd, delta},
    message::{Encoding, Reader, Writer},
};
use qa_platform::{Stopwatch, allocations};
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn qw(v: &[u32; 11]) -> QwCmd {
    QwCmd {
        view_angles: Vec3(std::array::from_fn(|i| f32::from_bits(v[i]))),
        movement: std::array::from_fn(|i| v[3 + i] as i16),
        buttons: v[6] as u8,
        impulse: v[7] as u8,
        msec: v[8] as u8,
    }
}
fn q2(v: &[u32; 11]) -> Q2Cmd {
    Q2Cmd {
        angles: std::array::from_fn(|i| v[i] as i16),
        movement: std::array::from_fn(|i| v[3 + i] as i16),
        buttons: v[6] as u8,
        impulse: v[7] as u8,
        msec: v[8] as u8,
        light_level: v[9] as u8,
    }
}
fn q3(v: &[u32; 11]) -> Q3Cmd {
    Q3Cmd {
        angles: std::array::from_fn(|i| v[i] as i32),
        movement: std::array::from_fn(|i| v[3 + i] as i8),
        buttons: v[6],
        weapon: v[7] as u8,
        server_time: v[10] as i32,
    }
}
struct Encoded {
    bits: u32,
    length: usize,
    decoded: [u32; 11],
}
fn encode(
    mode: u8,
    key: u32,
    from: &[u32; 11],
    to: &[u32; 11],
    bytes: &mut [u8; 256],
) -> Result<Encoded, String> {
    let encoding = if mode == 2 {
        Encoding::Q3
    } else {
        Encoding::Bytes
    };
    let mut writer = Writer::new(bytes, encoding);
    match mode {
        0 => delta::write_qw(&mut writer, qw(from), qw(to)),
        1 => delta::write_q2(&mut writer, q2(from), q2(to)),
        2 => delta::write_q3(&mut writer, q3(from), q3(to), key),
        _ => return Err("command dialect".into()),
    }
    .map_err(|e| e.to_string())?;
    let mut result = Encoded {
        bits: writer.bit_position() as u32,
        length: writer.size(),
        decoded: [0; 11],
    };
    let mut reader = Reader::new(writer.bytes(), encoding);
    let v = &mut result.decoded;
    match mode {
        0 => {
            let c = delta::read_qw(&mut reader, qw(from)).map_err(|e| e.to_string())?;
            v[..3].copy_from_slice(&c.view_angles.0.map(f32::to_bits));
            v[3..6].copy_from_slice(&c.movement.map(|n| n as u32));
            v[6] = c.buttons.into();
            v[7] = c.impulse.into();
            v[8] = c.msec.into();
        }
        1 => {
            let c = delta::read_q2(&mut reader, q2(from)).map_err(|e| e.to_string())?;
            v[..3].copy_from_slice(&c.angles.map(|n| n as u32));
            v[3..6].copy_from_slice(&c.movement.map(|n| n as u32));
            v[6] = c.buttons.into();
            v[7] = c.impulse.into();
            v[8] = c.msec.into();
            v[9] = c.light_level.into();
        }
        _ => {
            let c = delta::read_q3(&mut reader, q3(from), key).map_err(|e| e.to_string())?;
            v[..3].copy_from_slice(&c.angles.map(|n| n as u32));
            v[3..6].copy_from_slice(&c.movement.map(|n| n as u32));
            v[6] = c.buttons;
            v[7] = c.weapon.into();
            v[10] = c.server_time as u32;
        }
    }
    Ok(result)
}
struct Case {
    mode: u8,
    key: u32,
    from: [u32; 11],
    to: [u32; 11],
    bits: u32,
    length: usize,
    bytes: [u8; 256],
    decoded: [u32; 11],
}
fn words(reader: &mut Reader<'_>) -> Result<[u32; 11], String> {
    let mut v = [0; 11];
    for word in &mut v {
        *word = reader.read_bits(32).map_err(|e| e.to_string())?;
    }
    Ok(v)
}
fn case(reader: &mut Reader<'_>) -> Result<Case, String> {
    Ok(Case {
        mode: reader.read_bits(8).map_err(|e| e.to_string())? as u8,
        key: reader.read_bits(32).map_err(|e| e.to_string())?,
        from: words(reader)?,
        to: words(reader)?,
        bits: 0,
        length: 0,
        bytes: [0; 256],
        decoded: [0; 11],
    })
}
fn compare() -> Result<(), String> {
    let mut data = Vec::new();
    std::io::stdin()
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    let mut reader = Reader::new(&data, Encoding::Bytes);
    let mut output = std::io::stdout().lock();
    let mut bytes = [0; 256];
    while reader.byte_position() < data.len() {
        let c = case(&mut reader)?;
        let v = encode(c.mode, c.key, &c.from, &c.to, &mut bytes)?;
        output
            .write_all(&v.bits.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&(v.length as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&bytes[..v.length])
            .map_err(|e| e.to_string())?;
        for word in v.decoded {
            output
                .write_all(&word.to_le_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
fn timing(fixture: &str, original: &str) -> Result<(), String> {
    let data = std::fs::read(fixture).map_err(|e| e.to_string())?;
    let oracle = std::fs::read(original).map_err(|e| e.to_string())?;
    let mut reader = Reader::new(&data, Encoding::Bytes);
    let mut expected = Reader::new(&oracle, Encoding::Bytes);
    let mut cases = Vec::new();
    while reader.byte_position() < data.len() {
        let mut c = case(&mut reader)?;
        c.bits = expected.read_bits(32).map_err(|e| e.to_string())?;
        c.length = expected.read_bits(32).map_err(|e| e.to_string())? as usize;
        if c.length > c.bytes.len() {
            return Err("oracle capacity".into());
        }
        expected
            .read_data(&mut c.bytes[..c.length])
            .map_err(|e| e.to_string())?;
        c.decoded = words(&mut expected)?;
        cases.push(c);
    }
    if cases.is_empty() || expected.byte_position() != oracle.len() {
        return Err("oracle boundary".into());
    }
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut bytes = [0; 256];
    let mut samples = [0u64; 600];
    let mut counts = allocations::Counts::default();
    let mut checks = 0;
    let mut wire_bytes = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        for peer in 0..16 {
            let c = &cases[(frame * 16 + peer) % cases.len()];
            let v = encode(
                c.mode,
                c.key,
                std::hint::black_box(&c.from),
                &c.to,
                &mut bytes,
            )?;
            if v.bits != c.bits
                || v.length != c.length
                || bytes[..v.length] != c.bytes[..c.length]
                || v.decoded != c.decoded
            {
                return Err("native command fidelity".into());
            }
            checks += 1;
            wire_bytes += v.length;
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
        return Err(format!("command allocation/count gate {counts:?}"));
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"16 native command delta records per iteration, encode/decode and original-C byte/word fidelity; no full packets, workers, physical transport or gameplay\",\"warmup\":60,\"frames\":600,\"checks\":{checks},\"wire_bytes\":{wire_bytes},\"positive_control_allocations\":1,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"median_ns\":{},\"p99_ns\":{}}}",
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
    } else if args.len() == 4 && args[1] == "--timing" {
        timing(&args[2], &args[3])
    } else {
        Err("--compare or --timing fixture original".into())
    }
}
