//! Developer-only comparison of production per-entity thinks with unchanged
//! qsrc functions. tools/check_thinks.py supplies cold fixture files. This
//! example consumes no terminal input and launches no game or platform loop.
use qa_core::primitives::{CallbackCall, CallbackId, EntityId, ModuleId, ThinkTime};
use qa_session::dispatch::{
    FunctionBinding, FunctionTable, ModuleBinding, RuleSetId, ThinkFrame, ThinkWorld, run_think,
};
use qa_world::entities::{AllocationPolicy, EntityTable};
use std::hint::black_box;

#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;

const ENTITIES: usize = 3;
const MAX_CALLS: usize = 16;
const OUTPUT_WORDS: usize = 100;

#[derive(Clone, Copy)]
struct Input {
    frame: ThinkFrame,
    due: ThinkTime,
    callback: Option<CallbackId>,
    mode: u64,
    reschedule: ThinkTime,
    repeats: u64,
}

struct Case {
    order: bool,
    entities: [Input; ENTITIES],
}

struct World {
    entities: EntityTable,
    input: [Input; ENTITIES],
    remaining: [u64; ENTITIES],
    calls: [[u64; 5]; MAX_CALLS],
    call_count: usize,
    invalid: bool,
    bindings: [ModuleBinding; ENTITIES],
}

impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}

fn raw(time: ThinkTime) -> u64 {
    match time {
        ThinkTime::Seconds(value) => value.to_bits(),
        ThinkTime::Milliseconds(value) => value as u64,
    }
}

fn deadline(time: ThinkTime) -> Option<ThinkTime> {
    match time {
        ThinkTime::Seconds(0.0) | ThinkTime::Milliseconds(0) => None,
        _ => Some(time),
    }
}

fn callback(world: &mut World, _: ModuleId, function: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        world.invalid = true;
        return false;
    };
    let Some(slot) = world.entities.resolve(entity) else {
        world.invalid = true;
        return false;
    };
    if !(1..=ENTITIES).contains(&slot) || world.call_count >= MAX_CALLS {
        world.invalid = true;
        return false;
    }
    let index = slot - 1;
    world.calls[world.call_count] = [
        slot as u64,
        u64::from(function),
        raw(time),
        world.entities.columns.next_think[slot].map_or(0, raw),
        world
            .entities
            .columns
            .think_function(slot)
            .map_or(0, |value| u64::from(value.0)),
    ];
    world.call_count += 1;
    match world.input[index].mode {
        3 => {
            // This fixture removes the current lifetime and never reuses the
            // slot during run_think. Native teardown uses the same clear rule.
            if !world.entities.release(entity, ThinkTime::Seconds(0.0)) {
                world.invalid = true;
                return false;
            }
        }
        1 | 2 => {
            if world.input[index].mode == 2 {
                world
                    .entities
                    .columns
                    .set_think_function(slot, world.bindings[index].think(Some(CallbackId(2))));
            }
            if world.remaining[index] != 0 {
                world.remaining[index] -= 1;
                world.entities.columns.next_think[slot] = deadline(world.input[index].reschedule);
            }
        }
        _ => {}
    }
    true
}

fn word(data: &mut &[u8]) -> Result<u64, &'static str> {
    let (head, tail) = data.split_first_chunk::<8>().ok_or("truncated think row")?;
    *data = tail;
    Ok(u64::from_le_bytes(*head))
}

fn header_word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data
        .split_first_chunk::<4>()
        .ok_or("truncated think header")?;
    *data = tail;
    Ok(u32::from_le_bytes(*head))
}

fn input(data: &mut &[u8], timing: RuleSetId) -> Result<Input, &'static str> {
    let now = word(data)?;
    let step = f64::from_bits(word(data)?);
    let due = word(data)?;
    let function = word(data)?;
    let mode = word(data)?;
    let reschedule = word(data)?;
    let repeats = word(data)?;
    if function > 2 || mode > 3 || repeats > 3 || !step.is_finite() || step < 0.0 {
        return Err("invalid think fixture callback or frame duration");
    }
    let (frame, due, reschedule) = match timing {
        RuleSetId::Quake | RuleSetId::QuakeWorld | RuleSetId::Quake2 => {
            let now = f64::from_bits(now);
            let due = f64::from_bits(due);
            let reschedule = f64::from_bits(reschedule);
            if !now.is_finite()
                || !due.is_finite()
                || !reschedule.is_finite()
                || due != f64::from(due as f32)
                || reschedule != f64::from(reschedule as f32)
                || (timing == RuleSetId::Quake2 && now != f64::from(now as f32))
            {
                return Err("think seconds fixture violates its native float boundary");
            }
            (
                ThinkFrame::Seconds { now, step },
                ThinkTime::Seconds(due),
                ThinkTime::Seconds(reschedule),
            )
        }
        RuleSetId::Quake2Rerelease | RuleSetId::Quake3 => {
            let (now, due, reschedule) = (now as i64, due as i64, reschedule as i64);
            if timing == RuleSetId::Quake3
                && [now, due, reschedule]
                    .iter()
                    .any(|&value| i32::try_from(value).is_err())
            {
                return Err("Q3 fixture exceeds the native signed-int boundary");
            }
            (
                ThinkFrame::Milliseconds { now },
                ThinkTime::Milliseconds(due),
                ThinkTime::Milliseconds(reschedule),
            )
        }
    };
    Ok(Input {
        frame,
        due,
        callback: (function != 0).then_some(CallbackId(function as u32)),
        mode,
        reschedule,
        repeats,
    })
}

#[expect(
    clippy::collapsible_if,
    reason = "The original-C comparison fixture resolves each native-slot lifetime before its fallible release."
)]
fn reset(world: &mut World, case: &Case) -> Result<[EntityId; ENTITIES], &'static str> {
    for slot in 1..=ENTITIES {
        if let Some(id) = world.entities.id_at(slot) {
            if !world.entities.release(id, ThinkTime::Seconds(0.0)) {
                return Err("fixture lifetime reset rejected");
            }
        }
    }
    world.input = case.entities;
    world.remaining = case.entities.map(|value| value.repeats);
    world.calls.fill([0; 5]);
    world.call_count = 0;
    world.invalid = false;
    let mut ids = [EntityId {
        slot: 0,
        generation: 0,
    }; ENTITIES];
    for (index, id) in ids.iter_mut().enumerate() {
        // Reset frees occur at fixture time zero, in the native reuse grace
        // period. Allocation is a fixture boundary, not the module clock.
        *id = world
            .entities
            .allocate(
                ThinkTime::Seconds(1.0),
                ModuleId((index + 1) as u16),
                AllocationPolicy::EDICT,
            )
            .ok_or("fixture entity allocation rejected")?
            .id;
        if id.slot as usize != index + 1 {
            return Err("fixture native slot mapping differs");
        }
        world.entities.columns.next_think[index + 1] = deadline(case.entities[index].due);
        world.entities.columns.set_think_function(
            index + 1,
            world.bindings[index].think(case.entities[index].callback),
        );
    }
    Ok(ids)
}

fn execute(
    world: &mut World,
    table: &FunctionTable<World>,
    case: &Case,
) -> Result<[u64; OUTPUT_WORDS], &'static str> {
    let ids = reset(world, case)?;
    let order = if case.order { [2, 0, 1] } else { [0, 1, 2] };
    let frames = case.entities.map(|value| value.frame);
    let mut output = [0; OUTPUT_WORDS];
    for index in order {
        let result = run_think(world, table, black_box(ids[index]), |module| {
            frames.get(usize::from(module.0).wrapping_sub(1)).copied()
        });
        let base = 2 + index * 6;
        let slot = index + 1;
        output[base] = world.entities.columns.next_think[slot].map_or(0, raw);
        output[base + 1] = world
            .entities
            .columns
            .think_function(slot)
            .map_or(0, |id| u64::from(id.0));
        output[base + 2] = u64::from(world.entities.resolve(ids[index]).is_some());
        output[base + 3] = u64::from(result.called);
        output[base + 4] = u64::from(result.rejected);
        output[base + 5] = u64::from(result.current_lifetime);
        output[1] += u64::from(result.rejected);
    }
    if world.invalid {
        return Err("fixture callback rejected an invalid context");
    }
    output[0] = world.call_count as u64;
    for (index, call) in world.calls.iter().enumerate() {
        output[20 + index * 5..25 + index * 5].copy_from_slice(call);
    }
    Ok(output)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected q1|qw|q2|rr|q3, fixture file and result file".into());
    }
    let timing = match args[1].as_str() {
        "q1" => RuleSetId::Quake,
        "qw" => RuleSetId::QuakeWorld,
        "q2" => RuleSetId::Quake2,
        "rr" => RuleSetId::Quake2Rerelease,
        "q3" => RuleSetId::Quake3,
        _ => return Err("invalid native think rule".into()),
    };
    let payload = std::fs::read(&args[2])?;
    let mut data = payload.as_slice();
    if header_word(&mut data)? != 0x4b4e4854 || header_word(&mut data)? != 1 {
        return Err("unsupported native think fixture".into());
    }
    let count = header_word(&mut data)? as usize;
    if !(1..=50_000).contains(&count) {
        return Err("think fixture count out of bounds".into());
    }
    let mut cases = Vec::with_capacity(count);
    for _ in 0..count {
        let order = word(&mut data)?;
        if order > 1 {
            return Err("invalid think fixture caller order".into());
        }
        cases.push(Case {
            order: order != 0,
            entities: [
                input(&mut data, timing)?,
                input(&mut data, timing)?,
                input(&mut data, timing)?,
            ],
        });
    }
    if !data.is_empty() {
        return Err("trailing native think fixture bytes".into());
    }
    let first = cases.first().ok_or("empty native think fixture")?;
    let mut world = World {
        entities: EntityTable::new(4, 1).map_err(|_| "fixture entity table rejected")?,
        input: first.entities,
        remaining: [0; ENTITIES],
        calls: [[0; 5]; MAX_CALLS],
        call_count: 0,
        invalid: false,
        bindings: [ModuleBinding::default(); ENTITIES],
    };
    let table = FunctionTable::load((1..=ENTITIES).map(|module| {
        (
            ModuleId(module as u16),
            timing,
            (0..=2)
                .map(|entry| FunctionBinding {
                    entry,
                    call: callback,
                })
                .collect(),
        )
    }))
    .map_err(|_| "fixture function table rejected")?;
    world.bindings =
        std::array::from_fn(|index| table.module_binding(ModuleId((index + 1) as u16)));
    let mut results = vec![[0u64; OUTPUT_WORDS]; count];

    allocation_counter::start();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_allocations = allocation_counter::stop();
    drop(positive);
    if positive_allocations == 0 {
        return Err("allocation counter positive control did not detect an allocation".into());
    }

    allocation_counter::start();
    let outcome = cases
        .iter()
        .zip(&mut results)
        .try_for_each(|(case, result)| {
            *result = execute(&mut world, &table, black_box(case))?;
            Ok::<_, &'static str>(())
        });
    let allocations = allocation_counter::stop();
    outcome?;
    let calls: u64 = results.iter().map(|result| result[0]).sum();
    let rejections: u64 = results.iter().map(|result| result[1]).sum();
    let mut output = Vec::with_capacity(count * OUTPUT_WORDS * 8);
    for row in results {
        for value in row {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&args[3], output)?;
    println!(
        "{{\"scope\":\"production per-entity think fixture; not gameplay/full server/performance\",\"rule\":\"{}\",\"rows\":{count},\"callback_calls\":{calls},\"scoped_rejections\":{rejections},\"rust_calling_thread_alloc_or_realloc\":{allocations},\"allocation_positive_control\":{positive_allocations}}}",
        args[1]
    );
    if allocations != 0 {
        return Err("native think allocation gate failed".into());
    }
    Ok(())
}
