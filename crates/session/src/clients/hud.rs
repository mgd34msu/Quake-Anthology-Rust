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
        for (item, old_count) in self.inventory.item_counts.iter_mut().enumerate() {
            let count = player.inventory.item_counts.get(item).copied().unwrap_or(0);
            let acquired_at = player
                .inventory
                .item_acquired_at
                .get(item)
                .copied()
                .unwrap_or(0.0);
            if (*old_count != count
                || self.inventory.item_acquired_at[item].to_bits() != acquired_at.to_bits())
                && publish(HudChange::Item {
                    item: ItemId(item as u32),
                    count,
                    acquired_at,
                })
            {
                *old_count = count;
                self.inventory.item_acquired_at[item] = acquired_at;
            }
        }
        if self.inventory.weapon != player.inventory.weapon
            && publish(HudChange::Weapon(player.inventory.weapon))
        {
            self.inventory.weapon = player.inventory.weapon;
        }
        for (slot, old_until) in self.inventory.powerup_until.iter_mut().enumerate() {
            let until = player
                .inventory
                .powerup_until
                .get(slot)
                .copied()
                .unwrap_or(0.0);
            if old_until.to_bits() != until.to_bits()
                && publish(HudChange::Powerup {
                    slot: slot as u32,
                    until,
                })
            {
                *old_until = until;
            }
        }
        if self.inventory.collectibles != player.inventory.collectibles
            && publish(HudChange::Collectibles(player.inventory.collectibles))
        {
            self.inventory.collectibles = player.inventory.collectibles;
        }
    }
}
