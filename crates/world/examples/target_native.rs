//! Developer-only target-index comparison. Build this example separately and
//! run tools/check_targets.py. Native/common slot numbers coincide only inside
//! these fixtures; no VM ABI, module namespace or live game is exercised.
use qa_core::{
    names::{NameMatch, NameTable},
    primitives::{ModuleId, NameId},
};
use qa_world::{
    entities::{AllocationPolicy, EntityTable},
    targets::TargetIndex,
};
use std::hint::black_box;

#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;

const MAX_SLOTS: usize = 64;
const WORDS: usize = 3 + MAX_SLOTS;

#[derive(Clone, Copy)]
enum Boundary {
    Quake,
    Quake2,
    Quake3,
}

struct Table {
    entities: EntityTable,
    targets: TargetIndex,
}

struct Query {
    table: usize,
    start: Option<u32>,
    name: Option<NameId>,
}

fn word(data: &mut &[u8]) -> Result<u32, &'static str> {
    let (head, tail) = data
        .split_first_chunk::<4>()
        .ok_or("truncated target fixture")?;
    *data = tail;
    Ok(u32::from_le_bytes(*head))
}

fn optional_index(value: u32, limit: usize) -> Result<Option<usize>, &'static str> {
    if value == u32::MAX {
        Ok(None)
    } else if (value as usize) < limit {
        Ok(Some(value as usize))
    } else {
        Err("target fixture index exceeds its cold table")
    }
}

fn execute(
    table: &mut Table,
    query: &Query,
    names: &NameTable,
    boundary: Boundary,
) -> Result<[u32; WORDS], &'static str> {
    let mut result = [u32::MAX; WORDS];
    result[0] = 0;
    result[1] = match boundary {
        Boundary::Quake => 0,
        _ => u32::MAX,
    };
    result[2] = 0;
    let Some(name) = query.name else {
        // Native PF_Find faults on NULL match. The fixture's typed boundary
        // rejects that invalid request; it does not claim fatal-runtime parity.
        if matches!(boundary, Boundary::Quake) {
            result[2] = 1;
        }
        return Ok(result);
    };
    table.targets.refresh(&mut table.entities, names);
    let mode = match boundary {
        Boundary::Quake => NameMatch::Exact,
        Boundary::Quake2 | Boundary::Quake3 => NameMatch::Folded,
    };
    let mut previous = query.start;
    for id in table.targets.find(black_box(name), mode, names) {
        if query.start.is_some_and(|start| id.slot <= start) {
            continue;
        }
        if result[0] as usize >= MAX_SLOTS
            || id.slot as usize >= table.entities.capacity()
            || previous.is_some_and(|slot| id.slot <= slot)
            || table.entities.resolve(id).is_none()
        {
            return Err("target index returned invalid lifetime or source order");
        }
        result[3 + result[0] as usize] = id.slot;
        result[0] += 1;
        previous = Some(id.slot);
    }
    Ok(result)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 4 {
        return Err("expected q1|qw|q2|q3, target fixture and result file".into());
    }
    let boundary = match args[1].as_str() {
        "q1" | "qw" => Boundary::Quake,
        "q2" => Boundary::Quake2,
        "q3" => Boundary::Quake3,
        _ => return Err("invalid native target boundary".into()),
    };
    let payload = std::fs::read(&args[2])?;
    let mut data = payload.as_slice();
    if word(&mut data)? != 0x47524154 || word(&mut data)? != 1 {
        return Err("unsupported native target fixture".into());
    }
    let name_count = word(&mut data)? as usize;
    let table_count = word(&mut data)? as usize;
    let query_count = word(&mut data)? as usize;
    if !(1..=64).contains(&name_count)
        || !(1..=16).contains(&table_count)
        || !(1..=50000).contains(&query_count)
    {
        return Err("target fixture load capacity exceeds bounds".into());
    }
    let mut raw_names = Vec::with_capacity(name_count);
    for _ in 0..name_count {
        let size = word(&mut data)? as usize;
        if size > 63 || size > data.len() {
            return Err("target fixture name length exceeds bounds".into());
        }
        let (name, tail) = data.split_at(size);
        if name.contains(&0) {
            return Err("embedded NUL is outside the native string fixture contract".into());
        }
        raw_names.push(name.to_vec());
        data = tail;
    }
    let names = NameTable::load(raw_names.iter().map(Vec::as_slice))
        .map_err(|_| "target fixture name registration rejected")?;
    let ids: Vec<_> = raw_names.iter().map(|name| names.find(name)).collect();
    if ids.iter().any(Option::is_none) {
        return Err("registered exact native target name missing".into());
    }
    let mut tables = Vec::with_capacity(table_count);
    for _ in 0..table_count {
        let size = word(&mut data)? as usize;
        if !(1..=MAX_SLOTS).contains(&size) {
            return Err("target fixture entity capacity exceeds bounds".into());
        }
        let mut entities =
            EntityTable::new(size, 1).map_err(|_| "target fixture entity table rejected")?;
        let mut inactive = [false; MAX_SLOTS];
        for (slot, inactive_slot) in inactive.iter_mut().enumerate().take(size) {
            let active = word(&mut data)?;
            let name = optional_index(word(&mut data)?, name_count)?;
            if active > 1 || (slot == 0 && active == 0) {
                return Err("invalid native target fixture active flag".into());
            }
            let id = if slot == 0 {
                entities.id_at(0).ok_or("fixture world entity missing")?
            } else {
                entities
                    .allocate(1.0, ModuleId(1), AllocationPolicy::EDICT)
                    .ok_or("fixture native entity allocation rejected")?
                    .id
            };
            if id.slot as usize != slot
                || !entities.set_targetname(id, name.and_then(|index| ids[index]))
            {
                return Err("fixture native/common slot or name assignment differs".into());
            }
            *inactive_slot = active == 0;
        }
        for (slot, &inactive_slot) in inactive.iter().enumerate().take(size) {
            if inactive_slot {
                let id = entities
                    .id_at(slot)
                    .ok_or("fixture inactive row lifetime missing")?;
                if !entities.release(id, 0.0) {
                    return Err("fixture inactive row release rejected".into());
                }
            }
        }
        let mut targets = TargetIndex::new(&entities);
        targets.refresh(&mut entities, &names);
        tables.push(Table { entities, targets });
    }
    let mut queries = Vec::with_capacity(query_count);
    for _ in 0..query_count {
        let table = word(&mut data)? as usize;
        if table >= tables.len() {
            return Err("target query table exceeds bounds".into());
        }
        let start = optional_index(word(&mut data)?, tables[table].entities.capacity())?
            .map(|value| value as u32);
        let name = optional_index(word(&mut data)?, name_count)?;
        if (matches!(boundary, Boundary::Quake) && start.is_none())
            || (matches!(boundary, Boundary::Quake2) && name.is_none())
        {
            return Err("target query is outside its native start/NULL-match contract".into());
        }
        let name = match name {
            Some(index) => Some(
                match boundary {
                    Boundary::Quake => names.find(&raw_names[index]),
                    Boundary::Quake2 | Boundary::Quake3 => names.find_folded(&raw_names[index]),
                }
                .ok_or("registered native target query name missing")?,
            ),
            None => None,
        };
        queries.push(Query { table, start, name });
    }
    if !data.is_empty() {
        return Err("trailing native target fixture bytes".into());
    }
    let mut results = vec![[0; WORDS]; query_count];
    allocation_counter::start();
    let positive = black_box(Vec::<u8>::with_capacity(black_box(128)));
    let positive_allocations = allocation_counter::stop();
    drop(positive);
    if positive_allocations == 0 {
        return Err("target allocation positive control did not detect an allocation".into());
    }
    allocation_counter::start();
    let outcome = queries
        .iter()
        .zip(&mut results)
        .try_for_each(|(query, output)| {
            *output = execute(&mut tables[query.table], black_box(query), &names, boundary)?;
            Ok::<_, &'static str>(())
        });
    let allocations = allocation_counter::stop();
    outcome?;
    let found: u64 = results.iter().map(|row| u64::from(row[0])).sum();
    let rejections: u64 = results.iter().map(|row| u64::from(row[2])).sum();
    let mut output = Vec::with_capacity(query_count * WORDS * 4);
    for row in results {
        for value in row {
            output.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(&args[3], output)?;
    println!(
        "{{\"scope\":\"native target lookup fixture; not VM/module ABI/gameplay/performance\",\"rule\":\"{}\",\"rows\":{query_count},\"found_slots\":{found},\"scoped_invalid_query_rejections\":{rejections},\"rust_calling_thread_alloc_or_realloc\":{allocations},\"allocation_positive_control\":{positive_allocations}}}",
        args[1]
    );
    if allocations != 0 {
        return Err("target lookup allocation gate failed".into());
    }
    Ok(())
}
