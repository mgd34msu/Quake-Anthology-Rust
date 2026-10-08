//! Matched developer workload for sparse think dispatch and unnamed churn.
use qa_core::{
    names::{NameMatch, NameTable},
    primitives::{CallbackCall, CallbackId, EntityId, ModuleId, ThinkTime},
};
use qa_platform::{Stopwatch, allocations};
use qa_session::dispatch::{
    FunctionBinding, FunctionTable, ThinkFrame, ThinkTiming, ThinkWorld, run_thinks,
};
use qa_world::{
    entities::{AllocationPolicy, EntityTable, MAX_ENTITIES},
    targets::TargetIndex,
};
use std::hint::black_box;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

const LIVE: usize = 64;
const EMPTY: EntityId = EntityId {
    slot: u32::MAX,
    generation: 0,
};
struct World {
    entities: EntityTable,
    calls: [EntityId; LIVE],
    times: [u64; LIVE],
    count: usize,
}
impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}
fn callback(world: &mut World, _: ModuleId, _: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        return false;
    };
    let ThinkTime::Seconds(time) = time else {
        return false;
    };
    if world.count == LIVE {
        return false;
    }
    world.calls[world.count] = entity;
    world.times[world.count] = time.to_bits();
    world.count += 1;
    true
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let capacity: usize = args.get(1).ok_or("expected capacity")?.parse()?;
    if capacity < LIVE + 2 || capacity > MAX_ENTITIES {
        return Err("capacity outside fixture range".into());
    }
    let names =
        NameTable::load([b"Door".as_slice(), b"door".as_slice()]).map_err(|e| format!("{e:?}"))?;
    let door = names.find(b"Door").ok_or("name")?;
    let lower = names.find(b"door").ok_or("name")?;
    let mut entities = EntityTable::new(capacity, 1).map_err(|e| format!("{e:?}"))?;
    let mut ids = [EMPTY; LIVE];
    let mut next = 0;
    // Cold population keeps exactly 64 widely separated live rows.
    for slot in 1..capacity {
        let id = entities
            .allocate(10.0, ModuleId(1), AllocationPolicy::EDICT)
            .ok_or("cold allocation")?
            .id;
        if id.slot as usize != slot {
            return Err("cold native slot order".into());
        }
        if next < LIVE && slot == 1 + next * (capacity - 2) / (LIVE - 1) {
            ids[next] = id;
            entities.set_targetname(id, Some(if next % 2 == 0 { door } else { lower }));
            entities.columns.think_fn[slot] = Some(CallbackId(0));
            next += 1;
        }
    }
    if next != LIVE {
        return Err("cold sparse rows".into());
    }
    for slot in 1..capacity {
        if !ids.iter().any(|id| id.slot as usize == slot) {
            let id = entities.id_at(slot).ok_or("cold entity")?;
            if !entities.release(id, 10.0) {
                return Err("cold release".into());
            }
        }
    }
    let mut targets = TargetIndex::new(&entities);
    targets.refresh(&mut entities, &names);
    let mut world = World {
        entities,
        calls: [EMPTY; LIVE],
        times: [0; LIVE],
        count: 0,
    };
    let functions = FunctionTable::load([(
        ModuleId(1),
        ThinkTiming::Quake2,
        vec![FunctionBinding {
            entry: 70000,
            call: callback,
        }],
    )])
    .map_err(|e| format!("{e:?}"))?;
    let _ = Stopwatch::start().elapsed();
    allocations::begin_frame();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_count = allocations::end_frame();
    drop(positive);
    if positive_count.allocations != 1 {
        return Err("allocation positive control".into());
    }
    let mut dispatch_samples = [0u64; 600];
    let mut total_samples = [0u64; 600];
    let mut refreshed = 0;
    let mut allocations_total = 0;
    let mut reallocations_total = 0;
    let mut bytes_total = 0;
    let mut calls = 0;
    for frame in 0..660 {
        let now = 20.0 + frame as f64;
        allocations::begin_frame();
        let total = Stopwatch::start();
        let transient = world
            .entities
            .allocate(now, ModuleId(1), AllocationPolicy::EDICT)
            .ok_or("transient allocation")?
            .id;
        let claimed_refresh = targets.refresh(&mut world.entities, &names);
        if !world.entities.release(transient, now) {
            return Err("transient release".into());
        }
        let released_refresh = targets.refresh(&mut world.entities, &names);
        for id in ids {
            world.entities.columns.next_think[id.slot as usize] = Some(ThinkTime::Seconds(now));
        }
        world.count = 0;
        let dispatch = Stopwatch::start();
        let stats = run_thinks(&mut world, &functions, |_| {
            Some(ThinkFrame::Seconds { now, step: 0.1 })
        });
        let dispatch_ns = dispatch.elapsed().as_nanos() as u64;
        if stats.called != LIVE as u32
            || stats.rejected != 0
            || world.count != LIVE
            || world.calls != ids
            || world.times != [now.to_bits(); LIVE]
            || targets.find(door, NameMatch::Exact, &names).count() != LIVE / 2
            || targets.find(door, NameMatch::Folded, &names).count() != LIVE
        {
            return Err("ordered callback/target fidelity differs".into());
        }
        black_box(&world.calls);
        let total_ns = total.elapsed().as_nanos() as u64;
        let count = allocations::end_frame();
        if frame >= 60 {
            dispatch_samples[frame - 60] = dispatch_ns;
            total_samples[frame - 60] = total_ns;
            refreshed += u64::from(claimed_refresh) + u64::from(released_refresh);
            calls += world.count;
            allocations_total += count.allocations;
            reallocations_total += count.reallocations;
            bytes_total += count.requested_bytes;
        }
    }
    dispatch_samples.sort_unstable();
    total_samples.sort_unstable();
    let median = |rows: &[u64; 600]| (rows[299] as f64 + rows[300] as f64) * 0.5;
    println!(
        "{{\"scope\":\"headless sparse native-rule think, exact/folded targets and unnamed allocation/release; no retail gameplay\",\"capacity\":{capacity},\"live_callbacks\":{LIVE},\"warmup\":60,\"measured_frames\":600,\"ordered_calls\":{calls},\"dispatch_median_ns\":{},\"dispatch_p99_ns\":{},\"total_median_ns\":{},\"total_p99_ns\":{},\"unnamed_refreshes\":{refreshed},\"rust_allocations\":{allocations_total},\"rust_reallocations\":{reallocations_total},\"rust_requested_bytes\":{bytes_total},\"allocation_positive_control\":1,\"fidelity_mismatches\":0}}",
        median(&dispatch_samples),
        dispatch_samples[593],
        median(&total_samples),
        total_samples[593]
    );
    if allocations_total + reallocations_total + bytes_total != 0 {
        return Err("hot allocation gate".into());
    }
    Ok(())
}

#[cfg(not(any(debug_assertions, feature = "allocation-tracking")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("build with allocation-tracking".into())
}
