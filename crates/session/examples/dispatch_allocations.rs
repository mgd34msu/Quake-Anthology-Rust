use qa_world::entities::AllocationPolicy;
#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
use qa_core::primitives::*;
use qa_session::dispatch::*;
use qa_world::entities::EntityTable;

struct World {
    entities: EntityTable,
    calls: u64,
}
impl ThinkWorld for World {
    fn entities(&mut self) -> &mut EntityTable {
        &mut self.entities
    }
}
fn think(world: &mut World, _: ModuleId, _: u32, call: CallbackCall) -> bool {
    let CallbackCall::Think { entity, time } = call else {
        return false;
    };
    std::hint::black_box(entity);
    world.calls += 1;
    world.entities.columns.next_think[entity.slot as usize] = Some(Think {
        at: time + 1.0,
        callback: CallbackId(0),
    });
    true
}

fn main() -> Result<(), &'static str> {
    let mut world = World {
        entities: EntityTable::new(512, 1).map_err(|_| "entities")?,
        calls: 0,
    };
    let table = FunctionTable::load((1..=3).map(|module| {
        (
            ModuleId(module),
            ThinkTiming::Current { tolerance: 0.0 },
            vec![FunctionBinding {
                entry: 0,
                call: think,
            }],
        )
    }))
    .map_err(|_| "functions")?;
    for index in 0..400 {
        let id = world
            .entities
            .allocate(1.0, ModuleId(index % 3 + 1), AllocationPolicy::EDICT)
            .ok_or("entity capacity")?
            .id;
        world.entities.columns.next_think[id.slot as usize] = Some(Think {
            at: 1.0,
            callback: CallbackId(0),
        });
    }
    allocation_counter::start();
    for frame in 1..=10_000 {
        let stats = run_thinks(&mut world, &table, f64::from(frame), f64::from(frame));
        if stats.called != 400 || stats.rejected != 0 {
            return Err("think dispatch count");
        }
    }
    let allocations = allocation_counter::stop();
    println!(
        "{{\"scope\":\"headless numeric callback dispatch, not monster gameplay\",\"frames\":10000,\"entities\":400,\"calls\":{},\"allocations_after_load\":{allocations}}}",
        world.calls
    );
    if allocations == 0 {
        Ok(())
    } else {
        Err("think dispatch allocated")
    }
}
