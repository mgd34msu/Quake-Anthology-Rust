use qa_core::{
    events::{EventRing, FrameEvent, OutputTarget},
    primitives::{ClientId, HudChange, HudEvent, Inventory, ItemId, PlayerState},
};

/// Producer-owned baseline, independent of the HUD consumer's progress.
pub struct HudPublication {
    inventory: Inventory,
}

impl HudPublication {
    pub fn with_capacity(items: usize, powerups: usize) -> Self {
        Self {
            inventory: Inventory::with_capacity(items, powerups),
        }
    }

    pub fn reset(&mut self) {
        self.inventory.reset();
    }

    pub fn publish_changes(
        &mut self,
        player: &PlayerState,
        events: &mut EventRing,
        client: ClientId,
    ) {
        let mut publish = |change| {
            events
                .push_to(
                    OutputTarget::Client(client),
                    FrameEvent::Hud(HudEvent { client, change }),
                )
                .is_ok()
        };
        for item in 0..self.inventory.item_counts.len() {
            let count = player.inventory.item_counts.get(item).copied().unwrap_or(0);
            let acquired_at = player
                .inventory
                .item_acquired_at
                .get(item)
                .copied()
                .unwrap_or(0.0);
            let change = HudChange::Item {
                item: ItemId(item as u32),
                count,
                acquired_at,
            };
            if (self.inventory.item_counts[item] != count
                || self.inventory.item_acquired_at[item].to_bits() != acquired_at.to_bits())
                && publish(change)
            {
                self.inventory.apply(change);
            }
        }
        let change = HudChange::Weapon(player.inventory.weapon);
        if self.inventory.weapon != player.inventory.weapon && publish(change) {
            self.inventory.apply(change);
        }
        for slot in 0..self.inventory.powerup_until.len() {
            let until = player
                .inventory
                .powerup_until
                .get(slot)
                .copied()
                .unwrap_or(0.0);
            let change = HudChange::Powerup {
                slot: slot as u32,
                until,
            };
            if self.inventory.powerup_until[slot].to_bits() != until.to_bits() && publish(change) {
                self.inventory.apply(change);
            }
        }
        let change = HudChange::Collectibles(player.inventory.collectibles);
        if self.inventory.collectibles != player.inventory.collectibles && publish(change) {
            self.inventory.apply(change);
        }
    }
}
