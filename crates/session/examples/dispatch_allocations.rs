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
    world.entities.columns.next_think[entity.slot as usize] = Some(match time {
        ThinkTime::Seconds(time) => ThinkTime::Seconds(time + 1.0),
        ThinkTime::Milliseconds(time) => ThinkTime::Milliseconds(time + 1000),
    });
    true
}

fn main() -> Result<(), &'static str> {
    let mut world = World {
        entities: EntityTable::new(512, 1).map_err(|_| "entities")?,
        calls: 0,
    };
    let table = FunctionTable::load((1..=5).map(|module| {
        (
            ModuleId(module),
            match module {
                1 => RuleSetId::Quake,
                2 => RuleSetId::QuakeWorld,
                3 => RuleSetId::Quake2,
                4 => RuleSetId::Quake2Rerelease,
                _ => RuleSetId::Quake3,
            },
            vec![FunctionBinding {
                entry: 0,
                call: think,
            }],
        )
    }))
    .map_err(|_| "functions")?;
    for index in 0..400 {
        let module = ModuleId(index % 5 + 1);
        let id = world
            .entities
            .allocate(1.0, module, AllocationPolicy::EDICT)
            .ok_or("entity capacity")?
            .id;
        world.entities.columns.next_think[id.slot as usize] = Some(if module.0 <= 3 {
            ThinkTime::Seconds(1.0)
        } else {
            ThinkTime::Milliseconds(1000)
        });
        if !table.bind_think(&mut world.entities, id, Some(CallbackId(0))) {
            return Err("think binding lifetime");
        }
    }
    allocation_counter::start();
    for frame in 1..=10_000 {
        let stats = run_thinks(&mut world, &table, |module| {
            Some(if module.0 <= 3 {
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
