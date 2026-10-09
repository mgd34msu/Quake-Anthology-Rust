//! Production attachment transport: C-port comparison or fixed hot workload.
//! No game, native module, OS input, worker or renderer is started.
use qa_core::primitives::{BodyAttachment, BodyFollow, Bounds, Vec3};
use qa_platform::{Stopwatch, allocations};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    entities::EntityTable,
};
use std::hint::black_box;

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;
const ROWS: usize = 32;
const INPUT_WORDS: usize = 18;
const OUTPUT_WORDS: usize = ROWS * 12 + 2;

fn loaded(capacity: usize) -> Result<(EntityTable, AreaGrid), String> {
    Ok((
        EntityTable::new(capacity, capacity).map_err(|e| format!("{e:?}"))?,
        AreaGrid::load(
            capacity,
            Bounds {
                mins: Vec3([-65536.0; 3]),
                maxs: Vec3([65536.0; 3]),
            },
        )
        .map_err(|e| format!("{e:?}"))?,
    ))
}

fn compare(input: &str, output: &str) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() % (ROWS * INPUT_WORDS * 4) != 0 {
        return Err("fixture length".into());
    }
    let cases = bytes.len() / (ROWS * INPUT_WORDS * 4);
    let mut results = Vec::with_capacity(cases * OUTPUT_WORDS * 4);
    let mut calls = 0;
    let mut reallocations = 0;
    let mut requested = 0;
    for case in bytes.as_chunks::<{ ROWS * INPUT_WORDS * 4 }>().0 {
        let rows: [[u32; INPUT_WORDS]; ROWS] = std::array::from_fn(|slot| {
            std::array::from_fn(|word| {
                let at = (slot * INPUT_WORDS + word) * 4;
                u32::from_le_bytes([case[at], case[at + 1], case[at + 2], case[at + 3]])
            })
        });
        let (mut table, mut grid) = loaded(ROWS)?;
        for (slot, row) in rows.iter().enumerate() {
            let vector = |start| {
                Vec3(std::array::from_fn(|axis| {
                    f32::from_bits(row[start + axis])
                }))
            };
            table.columns.position[slot] = vector(0);
            table.columns.velocity[slot] = vector(3);
            table.columns.mins[slot] = vector(6);
            table.columns.maxs[slot] = vector(9);
            if slot != 0 {
                grid.link(
                    &table,
                    table.id_at(slot).ok_or("entity")?,
                    LinkFlags::SOLID,
                    LinkOrder::Tail,
                    LinkIntent::Explicit,
                );
            }
        }
        for order in 1..=ROWS as u32 {
            for (slot, row) in rows.iter().enumerate().filter(|(_, row)| row[17] == order) {
                let follow = match row[16] {
                    0 => BodyFollow::Translation,
                    1 => BodyFollow::Center,
                    2 => BodyFollow::BoundsMin,
                    _ => return Err("follow mode".into()),
                };
                table
                    .attach(
                        table.id_at(slot).ok_or("child")?,
                        BodyAttachment {
                            anchor: table.id_at(row[15] as usize).ok_or("anchor")?,
                            follow,
                            offset: Vec3(std::array::from_fn(|axis| {
                                f32::from_bits(row[12 + axis])
                            })),
                        },
                    )
                    .map_err(|e| format!("{e:?}"))?;
            }
        }
        allocations::begin_frame();
        let first = grid.transport_attachments(&mut table);
        let stable = grid.transport_attachments(&mut table);
        let count = allocations::end_frame();
        calls += count.allocations;
        reallocations += count.reallocations;
        requested += count.requested_bytes;
        if first.rejected != 0 || stable.rejected != 0 {
            return Err("rejected fixture".into());
        }
        for slot in 0..ROWS {
            for vector in [
                table.columns.position[slot],
                table.columns.velocity[slot],
                table.columns.mins[slot],
                table.columns.maxs[slot],
            ] {
                for value in vector.0 {
                    results.extend_from_slice(&value.to_bits().to_le_bytes());
                }
            }
        }
        results.extend_from_slice(&first.relinked.to_le_bytes());
        results.extend_from_slice(&stable.relinked.to_le_bytes());
    }
    std::fs::write(output, results).map_err(|e| e.to_string())?;
    if calls + reallocations + requested != 0 {
        return Err("comparison transport allocated".into());
    }
    println!(
        "{{\"scope\":\"headless C-port attachment fixtures; no gameplay/native touch callbacks\",\"cases\":{cases},\"rows\":{ROWS},\"rust_allocations\":{calls},\"rust_reallocations\":{reallocations},\"rust_requested_bytes\":{requested}}}"
    );
    Ok(())
}

fn timed(capacity: usize, predict: bool) -> Result<(), String> {
    let (mut table, mut grid) = loaded(capacity)?;
    if capacity < 2 {
        return Err("capacity requires an anchor".into());
    }
    for slot in 0..capacity {
        table.columns.mins[slot] = Vec3([-16.0, -8.0, -4.0]);
        table.columns.maxs[slot] = Vec3([18.0, 10.0, 6.0]);
        table.columns.velocity[slot] = Vec3([1.0, 2.0, 3.0]);
        table.columns.angles[slot] = Vec3([4.0, 5.0, 6.0]);
        if slot != 0 {
            grid.link(
                &table,
                table.id_at(slot).ok_or("entity")?,
                LinkFlags::SOLID,
                if slot.is_multiple_of(2) {
                    LinkOrder::Head
                } else {
                    LinkOrder::Tail
                },
                LinkIntent::Explicit,
            );
        }
    }
    // Worst-depth graph, reverse insertion, all three follow modes.
    for slot in (1..capacity).rev() {
        let (follow, offset) = match slot % 3 {
            0 => (BodyFollow::Center, Vec3([999.0; 3])),
            1 => (BodyFollow::Translation, Vec3([0.25; 3])),
            _ => (BodyFollow::BoundsMin, Vec3([16.25, 8.25, 4.25])),
        };
        table
            .attach(
                table.id_at(slot).ok_or("child")?,
                BodyAttachment {
                    anchor: table.id_at(slot - 1).ok_or("anchor")?,
                    follow,
                    offset,
                },
            )
            .map_err(|e| format!("{e:?}"))?;
    }
    let _ = Stopwatch::start().elapsed();
    allocations::begin_frame();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_count = allocations::end_frame();
    drop(positive);
    if positive_count.allocations != 1 {
        return Err("positive control".into());
    }
    let mut transport = [0u64; 600];
    let mut prediction = [0u64; 600];
    let mut links = [0u64; 600];
    let mut allocated = 0;
    let mut reallocated = 0;
    let mut requested = 0;
    let mut moves = 0u64;
    for frame in 0..660 {
        allocations::begin_frame();
        table.columns.position[0] = Vec3([frame as f32 + 1.0; 3]);
        let timer = Stopwatch::start();
        let result = grid.transport_attachments(&mut table);
        let elapsed = timer.elapsed().as_nanos() as u64;
        let mut expected = frame as f32 + 1.0;
        if result.moved != capacity as u32 - 1
            || result.relinked != result.moved
            || result.rejected != 0
        {
            return Err("transport counts".into());
        }
        for slot in 1..capacity {
            expected += if slot % 3 == 0 { 1.0 } else { 0.25 };
            if table.columns.position[slot].0.map(f32::to_bits) != [expected.to_bits(); 3]
                || table.columns.velocity[slot] != Vec3([1.0, 2.0, 3.0])
                || table.columns.angles[slot] != Vec3([4.0, 5.0, 6.0])
            {
                return Err("body fidelity".into());
            }
        }
        if grid.transport_attachments(&mut table).moved != 0 {
            return Err("unchanged transport".into());
        }
        let predicted_elapsed = if predict {
            let last = table.id_at(capacity - 1).ok_or("last")?;
            let old_position = table.columns.position[capacity - 1];
            let old_relinks = grid.relinks;
            let mut poses = [
                Some((table.id_at(0).ok_or("root")?, Vec3([frame as f32 + 2.0; 3]))),
                Some((last, Vec3([-1.0; 3]))),
            ];
            let timer = Stopwatch::start();
            let result = grid.predict_attachments(&table, &mut poses);
            let elapsed = timer.elapsed().as_nanos() as u64;
            if result.rejected != 0
                || result.relinked != 0
                || result.visited != capacity as u32 - 1
                || poses[1] != Some((last, old_position + Vec3([1.0; 3])))
                || table.columns.position[capacity - 1] != old_position
                || grid.relinks != old_relinks
            {
                return Err("predicted chain fidelity".into());
            }
            elapsed
        } else {
            0
        };
        let timer = Stopwatch::start();
        for operation in 0..8192 {
            let slot = 1 + operation % (capacity - 1);
            let id = table.id_at(slot).ok_or("linked entity")?;
            if !grid.unlink(id)
                || !grid.link(
                    &table,
                    id,
                    LinkFlags::SOLID,
                    if slot.is_multiple_of(2) {
                        LinkOrder::Head
                    } else {
                        LinkOrder::Tail
                    },
                    LinkIntent::Explicit,
                )
            {
                return Err("link cycle".into());
            }
        }
        let link_elapsed = timer.elapsed().as_nanos() as u64;
        // Detach/reattach exercises capacity reuse and insertion order in play.
        let last = table.id_at(capacity - 1).ok_or("last")?;
        let follow = table.attachment(last).ok_or("attachment")?;
        if !table.detach(last) {
            return Err("detach".into());
        }
        table.attach(last, follow).map_err(|e| format!("{e:?}"))?;
        let count = allocations::end_frame();
        if frame >= 60 {
            transport[frame - 60] = elapsed;
            prediction[frame - 60] = predicted_elapsed;
            links[frame - 60] = link_elapsed;
            allocated += count.allocations;
            reallocated += count.reallocations;
            requested += count.requested_bytes;
            moves += u64::from(result.moved);
        }
    }
    transport.sort_unstable();
    prediction.sort_unstable();
    links.sort_unstable();
    let median = |samples: &[u64; 600]| (samples[299] as f64 + samples[300] as f64) * 0.5;
    println!(
        "{{\"scope\":\"headless production attachment transport and area links; no gameplay/native touch callbacks/workers\",\"capacity\":{capacity},\"warmup\":60,\"measured_frames\":600,\"followed_bodies\":{},\"moved\":{moves},\"link_unlink_cycles_per_frame\":8192,\"transport_median_ns\":{},\"transport_p99_ns\":{},\"links_median_ns\":{},\"links_p99_ns\":{},\"fidelity_mismatches\":0,\"rust_allocations\":{allocated},\"rust_reallocations\":{reallocated},\"rust_requested_bytes\":{requested},\"allocation_positive_control\":1}}",
        capacity - 1,
        median(&transport),
        transport[593],
        median(&links),
        links[593]
    );
    if predict {
        println!(
            "{{\"scope\":\"headless current-client pose transport over frozen authoritative bodies; no gameplay\",\"capacity\":{capacity},\"warmup\":60,\"measured_frames\":600,\"predicted_clients\":2,\"prediction_median_ns\":{},\"prediction_p99_ns\":{},\"physical_pose_and_link_mismatches\":0,\"rust_allocations\":{allocated},\"rust_reallocations\":{reallocated},\"rust_requested_bytes\":{requested}}}",
            median(&prediction),
            prediction[593]
        );
    }
    if allocated + reallocated + requested != 0 {
        return Err("hot allocation gate".into());
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--compare" {
        compare(&args[2], &args[3])
    } else if args.len() == 2 || (args.len() == 3 && args[2] == "--prediction") {
        timed(
            args[1].parse::<usize>().map_err(|e| e.to_string())?,
            args.len() == 3,
        )
    } else {
        Err("expected capacity or --compare input output".into())
    }
}
