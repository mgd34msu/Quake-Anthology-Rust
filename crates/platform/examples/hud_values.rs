//! Native HUD-field boundary fixtures and the actual shared snapshot path.
use qa_core::primitives::*;
use std::hint::black_box;
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("bench") {
        return bench();
    }
    let mut runtime = qa_app::Runtime::load(64, std::iter::empty())?;
    for rules in RuleSetId::ALL {
        let table = runtime.catalog.hud.values.native(rules);
        for (bank, fields) in [&table.stats, &table.persistent].iter().enumerate() {
            for (ordinal, field) in fields.iter().enumerate() {
                for value in [i32::MIN, -32768, -1, 0, 1234, 32767, i32::MAX] {
                    if !field.import(&mut runtime.server.clients[0].player.values, value as u32) {
                        return Err("native field admission".into());
                    }
                    let common = runtime.server.clients[0]
                        .player
                        .values
                        .get(field.id)
                        .ok_or("value handle")?;
                    let projected = field
                        .export(&runtime.server.clients[0].player.values)
                        .ok_or("native field projection")?;
                    println!(
                        "{} {bank} {ordinal} {value} {} {projected}",
                        rules as u8,
                        common.as_integer()
                    );
                }
            }
        }
    }
    Ok(())
}
#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    use qa_platform::{
        Stopwatch,
        allocations::{begin_frame, end_frame},
    };
    let mut runtime = qa_app::Runtime::load(64, std::iter::empty())?;
    for client in &mut runtime.server.clients {
        client.player.health = 100;
        for rules in RuleSetId::ALL {
            let table = runtime.catalog.hud.values.native(rules);
            for field in table.stats.iter().chain(&table.persistent) {
                field.import(&mut client.player.values, 0x8123_4567);
            }
        }
    }
    begin_frame();
    let control = black_box(vec![0u8; 128]);
    let positive = end_frame();
    drop(control);
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut allocations = 0;
    let mut bytes = 0;
    let mut checksum = 0u64;
    let field = runtime.catalog.hud.values.native(RuleSetId::Quake2).stats[18];
    for frame in 0..660 {
        begin_frame();
        let timer = Stopwatch::start();
        for (index, client) in runtime.server.clients.iter_mut().enumerate() {
            field.import(
                &mut client.player.values,
                (frame as u32) * 64 + index as u32,
            );
            runtime.catalog.hud.update(&client.player, &mut client.hud);
            let value = field.export(&client.hud.values).ok_or("snapshot value")?;
            let expected = ((frame as u32) * 64 + index as u32) & 65535;
            if value != expected || client.hud.clipped_values != 0 {
                return Err("snapshot differs".into());
            }
            checksum = checksum.wrapping_add(u64::from(black_box(value)));
        }
        let ns = timer.elapsed().as_nanos() as u64;
        let counts = end_frame();
        if frame >= 60 {
            samples[frame - 60] = ns;
            allocations += counts.allocations + counts.reallocations;
            bytes += counts.requested_bytes;
        }
    }
    if allocations != 0 || bytes != 0 {
        return Err("HUD numeric allocation gate".into());
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"64 shared HUD snapshots with192 numeric values each; no native module or HUD drawing\",\"warmup\":60,\"frames\":600,\"allocations_or_reallocations\":{allocations},\"requested_bytes\":{bytes},\"allocation_positive_control\":1,\"checksum\":{checksum},\"median_ns\":{},\"p99_ns\":{}}}",
        (samples[299] + samples[300]) / 2,
        samples[593]
    );
    Ok(())
}
#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn bench() -> Result<(), Box<dyn std::error::Error>> {
    Err("bench requires allocation-tracking".into())
}
