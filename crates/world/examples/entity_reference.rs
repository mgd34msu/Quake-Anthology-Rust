//! Headless schedules compared with extracted original C by the developer tool.
//! Generation observations describe this engine's handles, not a native C ABI.
use qa_core::primitives::{EntityId, ModuleId};
use qa_world::entities::{AllocationPolicy, EntityTable, EntityTime, MAX_ENTITIES};
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counter;
static TRACKING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if TRACKING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(pointer, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counter = Counter;

#[derive(Clone, Copy)]
enum Family {
    Q1,
    Qw,
    Q2,
    Q3,
}

impl Family {
    fn parse(text: &str) -> Result<Self, &'static str> {
        match text {
            "q1" => Ok(Self::Q1),
            "qw" => Ok(Self::Qw),
            "q2" => Ok(Self::Q2),
            "q3" => Ok(Self::Q3),
            _ => Err("unknown allocation boundary"),
        }
    }

    fn time(self, text: &str) -> Result<EntityTime, Box<dyn std::error::Error>> {
        Ok(match self {
            Self::Q3 => EntityTime::Milliseconds(text.parse()?),
            // Q2's level.time is float, whereas Q1/QW's sv.time is double.
            Self::Q2 => EntityTime::Seconds(text.parse::<f32>()? as f64),
            Self::Q1 | Self::Qw => EntityTime::Seconds(text.parse()?),
        })
    }

    fn policy(self, start: i64) -> AllocationPolicy {
        match self {
            Self::Q1 | Self::Q2 => AllocationPolicy::EDICT,
            Self::Qw => AllocationPolicy::QUAKEWORLD,
            Self::Q3 => AllocationPolicy::q3(start),
        }
    }
}

struct Case<'a> {
    name: &'a str,
    table: EntityTable,
    handles: Vec<Option<EntityId>>,
    generations: Vec<u32>,
    policy: AllocationPolicy,
    step: usize,
}

impl<'a> Case<'a> {
    fn new(
        name: &'a str,
        capacity: usize,
        reserved: usize,
        policy: AllocationPolicy,
    ) -> Result<Self, &'static str> {
        let table = EntityTable::new(capacity, reserved).map_err(|_| "invalid table boundary")?;
        let handles = (0..capacity).map(|slot| table.id_at(slot)).collect();
        Ok(Self {
            name,
            table,
            handles,
            generations: vec![1; capacity],
            policy,
            step: 0,
        })
    }

    fn row(
        &mut self,
        op: &str,
        slot: i64,
        generation: u32,
        accepted: bool,
        displaced: Option<EntityId>,
    ) {
        let (old_slot, old_generation) =
            displaced.map_or((-1, 0), |id| (i64::from(id.slot), id.generation));
        println!(
            "{} {} {op} {slot} {generation} {} {old_slot} {old_generation} {}",
            self.name,
            self.step,
            u8::from(accepted),
            self.table.len()
        );
        self.step += 1;
    }

    fn allocate(&mut self, now: EntityTime) -> Result<(), &'static str> {
        let result = self.table.allocate(now, ModuleId(1), self.policy);
        if let Some(result) = result {
            let slot = result.id.slot as usize;
            if result.displaced.is_some() {
                self.generations[slot] += 1;
            }
            if result.id.generation != self.generations[slot]
                || result
                    .displaced
                    .is_some_and(|id| self.table.resolve(id).is_some())
            {
                return Err("allocation generation transition differs");
            }
            self.handles[slot] = Some(result.id);
            self.row(
                "alloc",
                slot as i64,
                result.id.generation,
                true,
                result.displaced,
            );
        } else {
            self.row("alloc", -1, 0, false, None);
        }
        Ok(())
    }

    fn release(&mut self, now: EntityTime, slot: usize) -> Result<(), &'static str> {
        let id = self
            .handles
            .get(slot)
            .copied()
            .flatten()
            .ok_or("free of never-allocated slot")?;
        let accepted = self.table.release(id, now);
        if accepted {
            self.generations[slot] += 1;
            if self.table.resolve(id).is_some() {
                return Err("freed handle still resolves");
            }
        }
        self.row("free", slot as i64, id.generation, accepted, None);
        Ok(())
    }

    fn protect(&mut self, slot: usize, value: bool) -> Result<(), &'static str> {
        let id = self
            .handles
            .get(slot)
            .copied()
            .flatten()
            .ok_or("protect of never-allocated slot")?;
        let accepted = self.table.set_never_free(id, value);
        self.row("protect", slot as i64, id.generation, accepted, None);
        Ok(())
    }

    fn check(&mut self, slot: u32, generation: u32) {
        let accepted = self.table.resolve(EntityId { slot, generation }).is_some();
        self.row("check", i64::from(slot), generation, accepted, None);
    }

    fn snapshot(&mut self) -> Result<(), &'static str> {
        for slot in 0..self.table.capacity() {
            let generation = self.generations[slot];
            let active = self.table.id_at(slot);
            if active.is_some_and(|id| id.generation != generation) {
                return Err("final live generation differs");
            }
            self.row("final", slot as i64, generation, active.is_some(), None);
        }
        Ok(())
    }
}

fn schedule(family: Family, input: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut case: Option<Case<'_>> = None;
    for line in input.lines() {
        let mut words = line.split_ascii_whitespace();
        let Some(command) = words.next() else {
            continue;
        };
        if command.starts_with('#') {
            continue;
        }
        let mut next = || words.next().ok_or("missing schedule field");
        match command {
            "reset" => {
                if let Some(previous) = case.as_mut() {
                    previous.snapshot()?;
                }
                let name = next()?;
                let capacity = next()?.parse()?;
                let reserved = next()?.parse()?;
                let start = next()?.parse()?;
                case = Some(Case::new(name, capacity, reserved, family.policy(start))?);
            }
            "alloc" => {
                let now = family.time(next()?)?;
                let count: usize = next()?.parse()?;
                if count > MAX_ENTITIES {
                    return Err("allocation count outside workload boundary".into());
                }
                let current = case.as_mut().ok_or("allocation before reset")?;
                for _ in 0..count {
                    current.allocate(now)?;
                }
            }
            "free" => {
                let now = family.time(next()?)?;
                let slot = next()?.parse()?;
                case.as_mut()
                    .ok_or("free before reset")?
                    .release(now, slot)?;
            }
            "protect" => {
                let slot = next()?.parse()?;
                let value: u8 = next()?.parse()?;
                if value > 1 {
                    return Err("invalid protection flag".into());
                }
                case.as_mut()
                    .ok_or("protect before reset")?
                    .protect(slot, value != 0)?;
            }
            "check" => {
                let slot = next()?.parse()?;
                let generation = next()?.parse()?;
                case.as_mut()
                    .ok_or("check before reset")?
                    .check(slot, generation);
            }
            _ => return Err("unknown schedule command".into()),
        }
        if words.next().is_some() {
            return Err("extra schedule field".into());
        }
    }
    case.as_mut().ok_or("empty schedule")?.snapshot()?;
    Ok(())
}

fn churn_cycle(
    table: &mut EntityTable,
    handles: &mut [Option<EntityId>],
    now: f64,
) -> Result<(), &'static str> {
    for (slot, entry) in handles.iter_mut().enumerate().skip(1) {
        let allocation = table
            .allocate(now, ModuleId(1), AllocationPolicy::EDICT)
            .ok_or("churn table unexpectedly full")?;
        if allocation.id.slot as usize != slot || allocation.displaced.is_some() {
            return Err("churn ascending allocation differs");
        }
        *entry = Some(allocation.id);
    }
    if table.len() != MAX_ENTITIES
        || table
            .allocate(now, ModuleId(1), AllocationPolicy::EDICT)
            .is_some()
    {
        return Err("churn full-table boundary differs");
    }
    for entry in handles.iter_mut().skip(1) {
        let id = entry.take().ok_or("missing churn handle")?;
        if !table.release(id, now) || table.resolve(id).is_some() {
            return Err("churn stale generation remains valid");
        }
        black_box(id);
    }
    if table.len() != 1 {
        return Err("churn changed reserved world slot");
    }
    Ok(())
}

fn churn() -> Result<(), &'static str> {
    let mut table = EntityTable::new(MAX_ENTITIES, 1).map_err(|_| "churn capacity")?;
    let mut handles = vec![None; MAX_ENTITIES];
    for cycle in 0..60 {
        churn_cycle(&mut table, &mut handles, 10.0 + cycle as f64)?;
    }
    ALLOCATIONS.store(0, Ordering::Relaxed);
    TRACKING.store(true, Ordering::Relaxed);
    let mut result = Ok(());
    for cycle in 0..600 {
        result = churn_cycle(&mut table, &mut handles, 70.0 + cycle as f64);
        if result.is_err() {
            break;
        }
    }
    TRACKING.store(false, Ordering::Relaxed);
    result?;
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    println!(
        "{{\"scope\":\"headless post-load entity churn; no gameplay or timing qualification\",\"capacity\":{MAX_ENTITIES},\"reserved\":1,\"warmup_cycles\":60,\"measured_cycles\":600,\"allocate_release_pairs\":{},\"allocations_after_load\":{allocations},\"remaining_live\":{}}}",
        600 * (MAX_ENTITIES - 1),
        table.len()
    );
    if allocations != 0 {
        return Err("post-load churn allocated");
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--churn" {
        return Ok(churn()?);
    }
    if args.len() != 3 {
        return Err("expected family and load-only schedule, or --churn".into());
    }
    schedule(Family::parse(&args[1])?, &args[2])
}
