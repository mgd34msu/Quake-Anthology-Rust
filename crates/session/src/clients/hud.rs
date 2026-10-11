use qa_core::{
    events::{EventRing, FrameEvent, OutputTarget},
    primitives::{ClientId, HudChange, HudEvent, ItemId, PlayerState, WeaponId},
};

/// Producer-owned baseline, independent of the HUD consumer's progress.
pub struct HudPublication {
    item_counts: Box<[i32]>,
    item_acquired_at: Box<[f64]>,
    powerup_until: Box<[f64]>,
    weapon: WeaponId,
    collectibles: u64,
    pub rejected_pushes: u64,
}

impl HudPublication {
    pub fn with_capacity(items: usize, powerups: usize) -> Self {
        Self {
            item_counts: vec![0; items].into_boxed_slice(),
            item_acquired_at: vec![0.0; items].into_boxed_slice(),
            powerup_until: vec![0.0; powerups].into_boxed_slice(),
            weapon: WeaponId::default(),
            collectibles: 0,
            rejected_pushes: 0,
        }
    }

    pub fn reset(&mut self) {
        self.item_counts.fill(0);
        self.item_acquired_at.fill(0.0);
        self.powerup_until.fill(0.0);
        self.weapon = WeaponId::default();
        self.collectibles = 0;
        self.rejected_pushes = 0;
    }

    pub fn publish_changes(
        &mut self,
        player: &PlayerState,
        events: &mut EventRing,
        client: ClientId,
    ) {
        let mut publish = |change| match events.push_to(
            OutputTarget::Client(client),
            FrameEvent::Hud(HudEvent { client, change }),
        ) {
            Ok(_) => true,
            Err(_) => {
                self.rejected_pushes = self.rejected_pushes.saturating_add(1);
                false
            }
        };
        for (item, old_count) in self.item_counts.iter_mut().enumerate() {
            let count = player.inventory.get(item).copied().unwrap_or(0);
            let acquired_at = player.item_acquired_at.get(item).copied().unwrap_or(0.0);
            if (*old_count != count
                || self.item_acquired_at[item].to_bits() != acquired_at.to_bits())
                && publish(HudChange::Item {
                    item: ItemId(item as u32),
                    count,
                    acquired_at,
                })
            {
                *old_count = count;
                self.item_acquired_at[item] = acquired_at;
            }
        }
        if self.weapon != player.weapon && publish(HudChange::Weapon(player.weapon)) {
            self.weapon = player.weapon;
        }
        for (slot, old_until) in self.powerup_until.iter_mut().enumerate() {
            let until = player.powerup_until.get(slot).copied().unwrap_or(0.0);
            if old_until.to_bits() != until.to_bits()
                && publish(HudChange::Powerup {
                    slot: slot as u32,
                    until,
                })
            {
                *old_until = until;
            }
        }
        if self.collectibles != player.collectibles
            && publish(HudChange::Collectibles(player.collectibles))
        {
            self.collectibles = player.collectibles;
        }
    }
}
