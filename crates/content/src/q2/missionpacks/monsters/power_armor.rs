//! Mission-pack monster power armor (`src/content/q2/missionpacks/monsters/power-armor.ts`).

use crate::contract::{InventoryEntry, PoweredProtectionState};
use crate::q2::foundation::monsters::types::{MonsterContext, bind_shared_power_cells};

/// Power-armor kind (`"screen" | "shield"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerArmorKind {
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Grant monster power armor (`monsterPowerArmor`).
pub fn monster_power_armor(context: &mut MonsterContext, kind: PowerArmorKind, cells: f64) {
    let actor = context.actor().clone();
    if !context.game.host.inventory().has(&actor) {
        let owned = context.game.owned_of(actor.clone());
        context.game.host.inventory().create(&owned, &[]);
    }
    let owned = context.game.owned_of(actor.clone());
    context.game.host.inventory().configure(
        &owned,
        &InventoryEntry {
            item: "q2:monster-power".to_string(),
            count: cells,
            capacity: cells,
            count_policy: None,
        },
    );
    restore_monster_power_armor(context);
    let owned = context.game.owned_of(actor);
    context.game.host.combat().set_powered_protection(
        &owned,
        &match kind {
            PowerArmorKind::Screen => PoweredProtectionState::Screen { cells },
            PowerArmorKind::Shield => PoweredProtectionState::Shield { cells },
        },
    );
}

/// Restore the power-armor binding (`restoreMonsterPowerArmor`).
pub fn restore_monster_power_armor(context: &mut MonsterContext) {
    let actor = context.actor().clone();
    let entries = context.game.host.inventory().entries(&actor);
    if !entries.iter().any(|entry| entry.item == "q2:monster-power") {
        panic!("Monster power armor is missing its shared inventory cells");
    }
    bind_shared_power_cells(context);
}
