//! Heap-only native KEX frame reception through the shared Ring and merge.
use qa_core::primitives::ThinkTime;
use qa_network::{
    commands::packet::Error,
    message::{Encoding, Reader, Writer},
    snapshots::{self, Q2Header, Q2KexRing},
    states::{self, Q2_RERELEASE_ENTITY_WORDS, Q2KexPlayer},
};
use qa_platform::allocations;

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
    let mut rings = [
        Q2KexRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?,
        Q2KexRing::load(16, 8192, 32, Some(8192)).map_err(|e| e.to_string())?,
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
        )
        .map_err(|e| e.to_string())?;
        writer.write_bits(0, 16).map_err(|e| e.to_string())?;
        lengths[mode] = writer.size();
    }
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
                if !snapshots::read_q2_kex(&mut reader, &mut rings[mode], mode == 1, |n| {
                    ThinkTime::Milliseconds(i64::from(n) * 25)
                })? {
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
        return Err(format!("KEX receive heap {total:?}"));
    }
    println!(
        "{{\"scope\":\"KEX2023/2022 full-frame reception, Ring and entity merge; caller heap only, no channel/OS/app/native host/gameplay\",\"warmup\":60,\"measured_iterations\":600,\"checks_including_warmup\":{checks},\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"positive_control_allocations\":1,\"timing_run\":false}}"
    );
    Ok(())
}
