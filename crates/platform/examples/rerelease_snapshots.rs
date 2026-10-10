//! Heap-only native KEX frame reception through the shared Ring and merge.
use qa_core::primitives::ThinkTime;
use qa_network::{
    commands::packet::Error,
    message::{Encoding, Reader, Writer},
    snapshots::{self, Q2Header, Q2KexContext, Q2KexRing, Q2ReproRing, Ring},
    states::{self, Q2_RERELEASE_ENTITY_WORDS, Q2KexPlayer, Q2ReproPlayer},
};
use qa_platform::allocations;
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn main() -> Result<(), String> {
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("heap positive control".into());
    }
    let args: Vec<_> = std::env::args().collect();
    if let [_, option, path] = &args[..]
        && option == "--compare"
    {
        return compare(path).map_err(|e| e.to_string());
    }
    if args.len() != 1 {
        return Err("usage: rerelease_snapshots [--compare fixture]".into());
    }
    let mut rings = [
        Q2KexRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?,
        Q2KexRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?,
    ];
    let mut contexts = [
        Q2KexContext::load(&rings[0], false).map_err(|e| e.to_string())?,
        Q2KexContext::load(&rings[1], true).map_err(|e| e.to_string())?,
    ];
    let mut packets = [[0; 1400]; 2];
    let mut lengths = [0; 2];
    let mut player = Q2KexPlayer::default();
    player.words[1] = 123.25f32.to_bits();
    player.words[8] = 0x8000;
    player.words[23] = 511;
    player.stats[63] = (-32768i32) as u32;
    let mut entity = [0; Q2_RERELEASE_ENTITY_WORDS];
    entity[0] = 65000;
    entity[8] = 1.25f32.to_bits();
    entity[14] = (-5.125f32).to_bits();
    entity[20] = 0xfedc_ba98;
    entity[21] = 127;
    for mode in 0..2 {
        let mut writer = Writer::new(&mut packets[mode], Encoding::Bytes);
        Q2Header {
            sequence: 5,
            delta: -1,
            flags: 6,
            player_flags: 0,
        }
        .write::<false>(&mut writer, &[0xff; 32])
        .map_err(|e| e.to_string())?;
        states::write_q2_kex_player(&mut writer, &Q2KexPlayer::default(), &player)
            .map_err(|e| e.to_string())?;
        writer.write_bits(18, 8).map_err(|e| e.to_string())?;
        states::write_q2_kex_entity(
            &mut writer,
            1,
            &[0; Q2_RERELEASE_ENTITY_WORDS],
            Some(&entity),
            true,
            mode == 1,
            &mut states::Q2KexWire {
                nonzero_solid: (&[0; Q2_RERELEASE_ENTITY_WORDS])[19] != 0,
                baseline_solid: false,
            },
        )
        .map_err(|e| e.to_string())?;
        writer.write_bits(0, 16).map_err(|e| e.to_string())?;
        lengths[mode] = writer.size();
    }
    let mut repro_ring = Q2ReproRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?;
    let mut repro_player = Q2ReproPlayer::default();
    repro_player.words[..42].copy_from_slice(&player.words);
    repro_player.words[42] = 255;
    repro_player.stats = player.stats;
    let mut player_bytes = [0; 1400];
    let mut player_writer = Writer::new(&mut player_bytes, Encoding::Bytes);
    let player_flags =
        states::write_q2_repro_player(&mut player_writer, &Q2ReproPlayer::default(), &repro_player)
            .map_err(|e| e.to_string())?;
    let mut repro_packet = [0; 1400];
    let mut writer = Writer::new(&mut repro_packet, Encoding::Bytes);
    Q2Header {
        sequence: 5,
        delta: -1,
        flags: 6,
        player_flags,
    }
    .write::<true>(&mut writer, &[0xff; 32])
    .map_err(|e| e.to_string())?;
    for &byte in player_writer.bytes() {
        writer
            .write_bits(u32::from(byte), 8)
            .map_err(|e| e.to_string())?;
    }
    states::write_q2_repro_entity(&mut writer, 1, &[0; 25], Some(&entity), true)
        .map_err(|e| e.to_string())?;
    writer.write_bits(0, 16).map_err(|e| e.to_string())?;
    let repro_length = writer.size();
    let mut total = allocations::Counts::default();
    let mut checks = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        let result: Result<(), Error> = (|| {
            for mode in 0..2 {
                let mut reader = Reader::new(&packets[mode][..lengths[mode]], Encoding::Bytes);
                if reader.read_bits(8)? != 20 {
                    return Err(Error::Opcode);
                }
                if !snapshots::read_q2_kex(
                    &mut reader,
                    &mut rings[mode],
                    &mut contexts[mode],
                    |n| ThinkTime::Milliseconds(i64::from(n) * 25),
                )? {
                    return Err(Error::Context);
                }
                let current = rings[mode].frame(5).ok_or(Error::Context)?;
                if reader.byte_position() != lengths[mode]
                    || current.entities.len() != 1
                    || current.entities[0].words != entity
                    || current.player[..42] != player.words
                    || current.player[42..] != player.stats
                    || current.flags != 6
                    || current.areas != [0xff; 32]
                    || current.time != ThinkTime::Milliseconds(125)
                {
                    return Err(Error::Context);
                }
                checks += 1;
            }
            let mut reader = Reader::new(&repro_packet[..repro_length], Encoding::Bytes);
            if reader.read_bits(8)? != 20
                || !snapshots::read_q2_repro(&mut reader, &mut repro_ring, native_time)?
            {
                return Err(Error::Context);
            }
            let current = repro_ring.current().ok_or(Error::Context)?;
            if reader.byte_position() != repro_length
                || current.entities.len() != 1
                || current.entities[0].words != entity
                || current.player[..43] != repro_player.words
                || current.player[43..] != repro_player.stats
                || current.flags != 6
                || current.areas != [0xff; 32]
                || current.time != ThinkTime::Milliseconds(125)
            {
                return Err(Error::Context);
            }
            checks += 1;
            Ok(())
        })();
        let heap = allocations::end_frame();
        result.map_err(|e| e.to_string())?;
        if frame >= 60 {
            total.allocations += heap.allocations;
            total.reallocations += heap.reallocations;
            total.requested_bytes += heap.requested_bytes;
        }
    }
    if total != allocations::Counts::default() {
        return Err(format!("enhanced Q2 receive heap {total:?}"));
    }
    println!(
        "{{\"scope\":\"KEX2023/2022 and protocol1038 full-frame reception, Ring and entity merge; caller heap only, no channel/OS/app/native host/gameplay\",\"warmup\":60,\"measured_iterations\":600,\"checks_including_warmup\":{checks},\"checks_per_format_including_warmup\":[660,660,660],\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"positive_control_allocations\":1,\"timing_run\":false}}"
    );
    Ok(())
}

fn compare(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    let mut input = Reader::new(&bytes, Encoding::Bytes);
    let cases = input.read_bits(32)?;
    let mut output = Vec::new();
    let mut frames = 0;
    let mut total = allocations::Counts::default();
    for _ in 0..cases {
        let mode = input.read_bits(8)?;
        let result = match mode {
            0 | 1 => {
                let mut ring = Q2KexRing::load(64, 8192, 255, Some(8192))?;
                baselines(&mut input, &mut ring)?;
                let mut context = Q2KexContext::load(&ring, mode == 1)?;
                compare_frames(&mut input, &mut output, &mut ring, |reader, ring| {
                    snapshots::read_q2_kex(reader, ring, &mut context, native_time)
                })?
            }
            2 => {
                let mut ring = Q2ReproRing::load(64, 8192, 255, Some(8192))?;
                baselines(&mut input, &mut ring)?;
                compare_frames(&mut input, &mut output, &mut ring, |reader, ring| {
                    snapshots::read_q2_repro(reader, ring, native_time)
                })?
            }
            _ => return Err("fixture format".into()),
        };
        frames += result.0;
        total.allocations += result.1.allocations;
        total.reallocations += result.1.reallocations;
        total.requested_bytes += result.1.requested_bytes;
    }
    if input.byte_position() != bytes.len() || total != allocations::Counts::default() {
        return Err(format!("fixture input/heap failure: {total:?}").into());
    }
    std::io::stdout().write_all(&output)?;
    eprintln!(
        "{{\"scope\":\"enhanced Q2 native frame receive comparison; caller Rust heap only, no channel/OS/app/native ABI\",\"cases\":{cases},\"frames\":{frames},\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"positive_control_allocations\":1,\"timing_run\":false}}"
    );
    Ok(())
}

fn native_time(n: u32) -> ThinkTime {
    ThinkTime::Milliseconds(i64::from(n) * 25)
}

fn baselines<const P: usize>(
    input: &mut Reader<'_>,
    ring: &mut Ring<P, Q2_RERELEASE_ENTITY_WORDS>,
) -> Result<(), Box<dyn std::error::Error>> {
    let count = input.read_bits(16)?;
    for _ in 0..count {
        let number = input.read_bits(16)?;
        let mut words = [0; Q2_RERELEASE_ENTITY_WORDS];
        for word in &mut words {
            *word = input.read_bits(32)?;
        }
        if !ring.set_baseline(number, &words) {
            return Err("fixture baseline rejected".into());
        }
    }
    Ok(())
}

fn compare_frames<const P: usize>(
    input: &mut Reader<'_>,
    output: &mut Vec<u8>,
    ring: &mut Ring<P, Q2_RERELEASE_ENTITY_WORDS>,
    mut receive: impl FnMut(
        &mut Reader<'_>,
        &mut Ring<P, Q2_RERELEASE_ENTITY_WORDS>,
    ) -> Result<bool, Error>,
) -> Result<(u32, allocations::Counts), Box<dyn std::error::Error>> {
    let count = input.read_bits(8)?;
    let mut total = allocations::Counts::default();
    for _ in 0..count {
        let size = input.read_bits(16)? as usize;
        let mut packet = [0; 1400];
        for byte in packet.get_mut(..size).ok_or("fixture packet too large")? {
            *byte = input.read_bits(8)? as u8;
        }
        let mut reader = Reader::new(&packet[..size], Encoding::Bytes);
        allocations::begin_frame();
        let result = (|| {
            if reader.read_bits(8)? != 20 {
                return Err(Error::Opcode);
            }
            receive(&mut reader, ring)
        })();
        let heap = allocations::end_frame();
        total.allocations += heap.allocations;
        total.reallocations += heap.reallocations;
        total.requested_bytes += heap.requested_bytes;
        if !result? || reader.byte_position() != size {
            return Err("fixture frame rejected or not consumed".into());
        }
        let frame = ring.current().ok_or("missing frame")?;
        output.extend_from_slice(&(reader.byte_position() as u16).to_le_bytes());
        output.extend_from_slice(&frame.sequence.to_le_bytes());
        output.extend_from_slice(&[frame.flags, frame.areas.len() as u8]);
        output.extend_from_slice(frame.areas);
        for word in frame.player {
            output.extend_from_slice(&word.to_le_bytes());
        }
        output.extend_from_slice(&(frame.entities.len() as u16).to_le_bytes());
        for entity in frame.entities {
            output.extend_from_slice(&(entity.number as u16).to_le_bytes());
            for word in &entity.words {
                output.extend_from_slice(&word.to_le_bytes());
            }
        }
    }
    Ok((count, total))
}
