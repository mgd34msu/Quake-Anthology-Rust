//! Pinned headless native timing and live-bitset dispatch, without game modules.
use qa_core::primitives::{CallbackCall, CallbackId, ModuleId, RuleSetId, ThinkTime};
use qa_platform::{Stopwatch, allocations};
use qa_session::dispatch::{FunctionBinding, FunctionTable, ThinkFrame, ThinkWorld, run_thinks};
use qa_world::entities::{AllocationPolicy, EntityTable};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

struct World {
    entities: EntityTable,
    calls: [u64; 5],
    last_time: [Option<ThinkTime>; 5],
}
impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

fn think(world: &mut World, module: ModuleId, _: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        return false;
    };
    let Some(slot) = world.entities.resolve(entity) else {
        return false;
    };
    world.calls[module.0 as usize] += 1;
    world.last_time[module.0 as usize] = Some(time);
    world.entities.columns.next_think[slot] = Some(match time {
        ThinkTime::Seconds(now) => ThinkTime::Seconds(now + 1.0),
        ThinkTime::Milliseconds(now) => ThinkTime::Milliseconds(now + 1000),
    });
    true
}

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let parameter = |name, default| -> Result<usize, String> {
        args.windows(2)
            .find(|pair| pair[0] == name)
            .map_or(Ok(default), |pair| {
                pair[1].parse().map_err(|_| format!("invalid {name}"))
            })
    };
    let capacity = parameter("--capacity", 8192)?;
    let live = parameter("--live", 2048)?;
    if live == 0 || live >= capacity {
        return Err("require 0 < live < capacity".into());
    }
    let mut world = World {
        entities: EntityTable::new(capacity, 1).map_err(|e| format!("{e:?}"))?,
        calls: [0; 5],
        last_time: [None; 5],
    };
    let table = FunctionTable::load(RuleSetId::ALL.map(|rules| {
        (
            ModuleId(rules as u16),
            rules,
            vec![FunctionBinding {
                entry: 0,
                call: think,
            }],
        )
    }))
    .map_err(|e| format!("{e:?}"))?;
    let mut ids = Vec::with_capacity(capacity - 1);
    let mut expected_per_rule = [0u64; 5];
    for index in 0..capacity - 1 {
        let id = world
            .entities
            .allocate(1.0, ModuleId((index % 5) as u16), AllocationPolicy::EDICT)
            .ok_or("entity capacity")?
            .id;
        ids.push(id);
    }
    for (index, &id) in ids.iter().enumerate() {
        // Spread sparse lifetimes across the complete slot range.
        if index * live / (capacity - 1) == (index + 1) * live / (capacity - 1) {
            world.entities.release(id, 1.0);
            continue;
        }
        expected_per_rule[index % 5] += 1;
        world.entities.columns.next_think[id.slot as usize] = Some(if index % 5 < 3 {
            ThinkTime::Seconds(1.0)
        } else {
            ThinkTime::Milliseconds(1000)
        });
        if !table.bind_think(&mut world.entities, id, Some(CallbackId(0))) {
            return Err("think binding lifetime".into());
        }
    }
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    let positive = allocations::end_frame();
    if positive.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut samples = [0u64; 600];
    let mut allocated = allocations::Counts::default();
    for frame in 1u32..=660 {
        allocations::begin_frame();
        let watch = Stopwatch::start();
        let result = run_thinks(&mut world, &table, |module| {
            Some(if module.0 < 3 {
                ThinkFrame::Seconds {
                    now: f64::from(frame),
                    step: 0.01,
                }
            } else {
                ThinkFrame::Milliseconds {
                    now: i64::from(frame) * 1000,
                }
            })
        });
        let elapsed = watch.elapsed().as_nanos() as u64;
        let counts = allocations::end_frame();
        if result.called as usize != live || result.rejected != 0 {
            return Err("dispatch result differs".into());
        }
        if frame > 60 {
            samples[frame as usize - 61] = elapsed;
            allocated.allocations += counts.allocations;
            allocated.reallocations += counts.reallocations;
            allocated.requested_bytes += counts.requested_bytes;
        }
    }
    for (rule, &expected) in expected_per_rule.iter().enumerate() {
        let time = if rule < 3 {
            ThinkTime::Seconds(660.0)
        } else {
            ThinkTime::Milliseconds(660000)
        };
        if world.calls[rule] != expected * 660
            || (expected != 0 && world.last_time[rule] != Some(time))
        {
            return Err("per-rule callback count or timestamp differs".into());
        }
    }
    samples.sort_unstable();
    println!(
        "{{\"scope\":\"headless mixed native think rules and live-bitset scan; no gameplay, workers or native modules\",\"capacity\":{capacity},\"live\":{live},\"warmup\":60,\"frames\":600,\"calls_per_rule\":{:?},\"fidelity_mismatches\":0,\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"allocation_positive_control\":1,\"median_ns\":{},\"p99_ns\":{}}}",
        world.calls,
        allocated.allocations,
        allocated.reallocations,
        allocated.requested_bytes,
        (samples[299] as f64 + samples[300] as f64) / 2.0,
        samples[593]
    );
    if allocated.allocations != 0 || allocated.reallocations != 0 || allocated.requested_bytes != 0
    {
        return Err("measured dispatch heap activity".into());
    }
    Ok(())
}
