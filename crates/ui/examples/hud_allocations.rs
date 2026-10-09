#[path = "../../../tools/probes/allocation_counter.rs"]
mod allocation_counter;
use qa_core::{names::NameTable, primitives::*};
use qa_gameplay::registry::Registry;
use qa_ui::hud::{HudBindings, expire_messages, print};

fn main() -> Result<(), &'static str> {
    let names = NameTable::load(Registry::names_needed()).map_err(|_| "names")?;
    let registry = Registry::load(&names).map_err(|_| "registry")?;
    let bindings = HudBindings::load(&registry, &[]).ok_or("HUD layout")?;
    let mut players: [PlayerState; 3] = std::array::from_fn(|_| {
        PlayerState::with_capacity(registry.items.len() + 1, 16, bindings.values.capacity())
    });
    let mut states = std::array::from_fn::<_, 3, _>(|_| bindings.state(16, NameId(1)));
    for (index, player) in players.iter_mut().enumerate() {
        let weapon = registry
            .weapons
            .iter()
            .find(|weapon| weapon.module == ModuleId(index as u16 + 1) && weapon.ammo.is_some())
            .ok_or("weapon")?;
        player.weapon = weapon.id;
        player.inventory[weapon.item.0 as usize] = 1;
        player.inventory[weapon.ammo.ok_or("ammo")?.0 as usize] = 17;
    }
    let mut texts = qa_core::events::TextStore::load(3, 32).map_err(|_| "text")?;
    let leases: [_; 3] = std::array::from_fn(|_| texts.insert(b"message").expect("load rows"));
    allocation_counter::start();
    let mut snapshots = 0;
    for frame in 0..10_000 {
        for (index, (player, state)) in players.iter_mut().zip(&mut states).enumerate() {
            player.health = 100 - frame % 20;
            bindings.update(player, state);
            print(
                state,
                &mut texts,
                PrintEvent {
                    client: Some(ClientId(index as u32)),
                    kind: PrintKind::Notify,
                    level: 1,
                    text: leases[index].id(),
                },
                frame as f64,
                3.0,
                2.0,
            );
            expire_messages(state, &mut texts, frame as f64);
            if std::hint::black_box(state.health) != player.health || state.ammo != 17 {
                return Err("snapshot differs");
            }
            snapshots += 1;
        }
    }
    let allocations = allocation_counter::stop();
    println!(
        "{{\"scope\":\"headless HUD snapshots, not HUD drawing\",\"frames\":10000,\"seats\":3,\"snapshots\":{snapshots},\"allocations_after_load\":{allocations}}}"
    );
    if allocations == 0 {
        Ok(())
    } else {
        Err("HUD allocated during update")
    }
}
