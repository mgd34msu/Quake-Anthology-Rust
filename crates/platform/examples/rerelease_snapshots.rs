//! Heap-only enhanced Q2 frame transmission/reception through the shared Ring.
use qa_core::primitives::ThinkTime;
use qa_network::{
    commands::packet::Error,
    message::{Encoding, Reader, Writer},
    snapshots::{self, Entity, Frame, Q2EntityPolicy, Q2KexContext, Q2KexRing, Q2ReproRing, Ring},
    states::{Q2_RERELEASE_ENTITY_WORDS, Q2KexPlayer, Q2ReproPlayer},
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
    if let [_, option, path] = &args[..]
        && option == "--compare-transmit"
    {
        return compare_transmit(path).map_err(|e| e.to_string());
    }
    if args.len() != 1 {
        return Err(
            "usage: rerelease_snapshots [--compare fixture | --compare-transmit fixture]".into(),
        );
    }
    let mut rings = [
        Q2KexRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?,
        Q2KexRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?,
    ];
    let mut contexts = [
        Q2KexContext::load(&rings[0], false).map_err(|e| e.to_string())?,
        Q2KexContext::load(&rings[1], true).map_err(|e| e.to_string())?,
    ];
    let mut senders = [
        Q2KexRing::load(16, 8192, 32, None).map_err(|e| e.to_string())?,
        Q2KexRing::load(16, 8192, 32, None).map_err(|e| e.to_string())?,
    ];
    let mut send_contexts = [
        Q2KexContext::load(&senders[0], false).map_err(|e| e.to_string())?,
        Q2KexContext::load(&senders[1], true).map_err(|e| e.to_string())?,
    ];
    let mut repro_sender = Q2ReproRing::load(16, 8192, 32, None).map_err(|e| e.to_string())?;
    let mut repro_ring = Q2ReproRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?;
    let mut packets = [[0; 1400]; 2];
    let mut repro_packet = [0; 1400];
    let mut player = Q2KexPlayer::default();
    player.words[1] = 123.25f32.to_bits();
    player.words[8] = 0x8000;
    player.words[23] = 511;
    player.stats[63] = (-32768i32) as u32;
    let mut native_player = [0; 106];
    native_player[..42].copy_from_slice(&player.words);
    native_player[42..].copy_from_slice(&player.stats);
    let mut repro_player = Q2ReproPlayer::default();
    repro_player.words[..42].copy_from_slice(&player.words);
    repro_player.words[42] = 255;
    repro_player.stats = player.stats;
    let mut native_repro = [0; 107];
    native_repro[..43].copy_from_slice(&repro_player.words);
    native_repro[43..].copy_from_slice(&repro_player.stats);
    let mut entity = [0; Q2_RERELEASE_ENTITY_WORDS];
    entity[0] = 65000;
    entity[8] = 1.25f32.to_bits();
    entity[14] = (-5.125f32).to_bits();
    entity[20] = 0xfedc_ba98;
    entity[21] = 127;
    let policy = Q2EntityPolicy {
        native_clients: 1,
        ..Q2EntityPolicy::default()
    };
    let mut total = allocations::Counts::default();
    let mut checks = 0;
    for frame in 0..660 {
        allocations::begin_frame();
        let result: Result<(), Error> = (|| {
            let sequence = frame + 1;
            let request = (frame != 0).then_some(frame);
            entity[8] = (1.25 + (frame % 2) as f32).to_bits();
            entity[19] = frame % 2;
            let entities = [Entity {
                number: 1,
                words: entity,
            }];
            for mode in 0..2 {
                senders[mode].store(Frame {
                    sequence,
                    time: native_time(sequence),
                    command: 0,
                    flags: 6,
                    areas: &[0xff; 32],
                    player: &native_player,
                    entities: &entities,
                })?;
                let mut writer = Writer::new(&mut packets[mode], Encoding::Bytes);
                snapshots::write_q2_kex(
                    &mut writer,
                    &mut senders[mode],
                    &mut send_contexts[mode],
                    sequence,
                    request,
                    policy,
                )?;
                let length = writer.size();
                let mut reader = Reader::new(&packets[mode][..length], Encoding::Bytes);
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
                let current = rings[mode].frame(sequence).ok_or(Error::Context)?;
                if reader.byte_position() != length
                    || current.entities.len() != 1
                    || current.entities[0].words != entity
                    || current.player[..42] != player.words
                    || current.player[42..] != player.stats
                    || current.flags != 6
                    || current.areas != [0xff; 32]
                    || current.time != native_time(sequence)
                {
                    return Err(Error::Context);
                }
                checks += 1;
            }
            repro_sender.store(Frame {
                sequence,
                time: native_time(sequence),
                command: 0,
                flags: 6,
                areas: &[0xff; 32],
                player: &native_repro,
                entities: &entities,
            })?;
            let mut writer = Writer::new(&mut repro_packet, Encoding::Bytes);
            snapshots::write_q2_repro(&mut writer, &mut repro_sender, sequence, request, policy)?;
            let repro_length = writer.size();
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
                || current.time != native_time(sequence)
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
        return Err(format!("enhanced Q2 frame heap {total:?}"));
    }
    println!(
        "{{\"scope\":\"KEX2023/2022 and protocol1038 full/delta frame transmission and reception, Ring and entity merge; caller heap only, no channel/OS/app/native host/gameplay\",\"warmup\":60,\"measured_iterations\":600,\"checks_including_warmup\":{checks},\"checks_per_format_including_warmup\":[660,660,660],\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"positive_control_allocations\":1,\"timing_run\":false}}"
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

fn compare_transmit(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let mut input = Reader::new(&bytes, Encoding::Bytes);
    let cases = input.read_bits(32)?;
    let mut output = Vec::new();
    let mut frames = 0;
    let mut total = allocations::Counts::default();
    for _ in 0..cases {
        let mode = input.read_bits(8)?;
        let policy = Q2EntityPolicy {
            beam_old_origin_fix: input.read_bits(8)? != 0,
            native_clients: input.read_bits(16)?,
            first_person: match input.read_bits(16)? {
                0 => None,
                n => Some(n),
            },
        };
        let result = match mode {
            0 | 1 => {
                let mut ring = Q2KexRing::load(16, 8192, 255, None)?;
                baselines(&mut input, &mut ring)?;
                let mut context = Q2KexContext::load(&ring, mode == 1)?;
                transmit_frames(
                    &mut input,
                    &mut output,
                    &mut ring,
                    |writer, ring, sequence, request| {
                        snapshots::write_q2_kex(
                            writer,
                            ring,
                            &mut context,
                            sequence,
                            request,
                            policy,
                        )
                    },
                )?
            }
            2 => {
                let mut ring = Q2ReproRing::load(16, 8192, 255, None)?;
                baselines(&mut input, &mut ring)?;
                transmit_frames(
                    &mut input,
                    &mut output,
                    &mut ring,
                    |writer, ring, sequence, request| {
                        snapshots::write_q2_repro(writer, ring, sequence, request, policy)
                    },
                )?
            }
            _ => return Err("fixture format".into()),
        };
        frames += result.0;
        total.allocations += result.1.allocations;
        total.reallocations += result.1.reallocations;
        total.requested_bytes += result.1.requested_bytes;
    }
    if input.byte_position() != bytes.len() || total != allocations::Counts::default() {
        return Err(format!("transmit fixture input/heap failure: {total:?}").into());
    }
    std::io::stdout().write_all(&output)?;
    eprintln!(
        "{{\"scope\":\"enhanced Q2 complete native frame transmission comparison; caller Rust heap only, no channel/OS/app/native ABI\",\"cases\":{cases},\"frames\":{frames},\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"positive_control_allocations\":1,\"timing_run\":false}}"
    );
    Ok(())
}

fn transmit_frames<const P: usize>(
    input: &mut Reader<'_>,
    output: &mut Vec<u8>,
    ring: &mut Ring<P, Q2_RERELEASE_ENTITY_WORDS>,
    mut write: impl FnMut(
        &mut Writer<'_>,
        &mut Ring<P, Q2_RERELEASE_ENTITY_WORDS>,
        u32,
        Option<u32>,
    ) -> Result<(), Error>,
) -> Result<(u32, allocations::Counts), Box<dyn std::error::Error>> {
    let frames = input.read_bits(8)?;
    let mut total = allocations::Counts::default();
    let mut bytes = [0; 1400];
    let mut areas = [0; 255];
    let mut player = [0; P];
    let mut entities = [Entity {
        number: 0,
        words: [0; Q2_RERELEASE_ENTITY_WORDS],
    }; 16];
    for _ in 0..frames {
        let sequence = input.read_bits(32)?;
        let delta = input.read_signed(32)?;
        let flags = input.read_bits(8)? as u8;
        let area_count = input.read_bits(8)? as usize;
        input.read_data(&mut areas[..area_count])?;
        for word in &mut player {
            *word = input.read_bits(32)?;
        }
        let count = input.read_bits(16)? as usize;
        let target = entities.get_mut(..count).ok_or("fixture entity capacity")?;
        for entity in &mut *target {
            entity.number = input.read_bits(16)?;
            for word in &mut entity.words {
                *word = input.read_bits(32)?;
            }
        }
        allocations::begin_frame();
        let result: Result<usize, Error> = (|| {
            ring.store(Frame {
                sequence,
                time: native_time(sequence),
                command: 0,
                flags,
                areas: &areas[..area_count],
                player: &player,
                entities: target,
            })?;
            let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
            write(
                &mut writer,
                ring,
                sequence,
                (delta > 0).then_some(delta as u32),
            )?;
            Ok(writer.size())
        })();
        let heap = allocations::end_frame();
        let size = result?;
        total.allocations += heap.allocations;
        total.reallocations += heap.reallocations;
        total.requested_bytes += heap.requested_bytes;
        output.extend_from_slice(&(size as u16).to_le_bytes());
        output.extend_from_slice(&bytes[..size]);
    }
    Ok((frames, total))
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
