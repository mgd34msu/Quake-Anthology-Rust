//! Native Q1/QW player stats enter the same inventory and value bank.
use super::Registry;
use qa_core::{
    primitives::{
        HudChange, ItemId, ModelRef, PlayerState, ResourceNamespace, RuleSetId, ValueBinding,
        WeaponId,
    },
    sys_events::EventTime,
};

#[derive(Clone, Copy)]
pub struct NativePlayerBindings {
    weapons: [Option<(u32, WeaponId, ItemId)>; 8],
    items: [Option<(u32, ItemId)>; 10],
    ammo: [Option<ItemId>; 4],
    armor: [Option<ItemId>; 3],
    items_stat: bool,
}
impl NativePlayerBindings {
    /// Native bit positions are ABI data; registry identities remain independent.
    pub fn load(source: RuleSetId, registry: &Registry) -> Option<Self> {
        if !matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            return None;
        }
        Some(Self {
            weapons: [1, 2, 4, 8, 16, 32, 64, 4096].map(|native| {
                registry.native_weapon(source, native).and_then(|weapon| {
                    registry
                        .weapon(weapon)
                        .map(|entry| (u32::from(native), weapon, entry.item))
                })
            }),
            // Original bothdefs.h IT_* values, paired with catalog ordinals.
            items: [
                (8192, 2),
                (16384, 3),
                (32768, 4),
                (131072, 16),
                (262144, 17),
                (524288, 21),
                (1048576, 19),
                (2097152, 20),
                (4194304, 22),
                (65536, 1),
            ]
            .map(|(mask, native)| {
                registry
                    .native_item(source, native)
                    .map(|item| (mask, item))
            }),
            ammo: [11, 12, 13, 14].map(|native| registry.native_item(source, native)),
            armor: [2, 3, 4].map(|native| registry.native_item(source, native)),
            items_stat: source == RuleSetId::QuakeWorld,
        })
    }
    pub fn fits(&self, player: &PlayerState) -> bool {
        self.ammo
            .into_iter()
            .flatten()
            .chain(self.items.into_iter().flatten().map(|(_, item)| item))
            .chain(self.weapons.into_iter().flatten().map(|(_, _, item)| item))
            .all(|item| {
                (item.0 as usize) < player.inventory.item_counts.len()
                    && (item.0 as usize) < player.inventory.item_acquired_at.len()
            })
    }
    /// Unknown/reserved ordinals retain their raw bits without inventing fields.
    /// Scheduling, movement, trace and weapon readiness are never changed here.
    pub fn import(
        &self,
        binding: Option<ValueBinding>,
        namespace: ResourceNamespace,
        index: usize,
        bits: u32,
        player: &mut PlayerState,
        time: EventTime,
    ) -> bool {
        if index >= 32 {
            return false;
        }
        if index == 15 && self.items_stat {
            return self.import_items(binding, bits, player, time);
        }
        if (6..=9).contains(&index)
            && self.ammo[index - 6].is_some_and(|item| {
                item.0 as usize >= player.inventory.item_counts.len()
                    || item.0 as usize >= player.inventory.item_acquired_at.len()
            })
        {
            return false;
        }
        if let Some(binding) = binding
            && !binding.import(&mut player.values, bits)
        {
            return false;
        }
        let integer = bits as i32;
        match index {
            0 => player.health = integer,
            1 => {
                player.frags = integer;
                player.score = integer;
            }
            2 => player.view_weapon.model = ModelRef::native(namespace, bits),
            4 => player.armor = integer,
            5 => player.view_weapon.frame = integer,
            6..=9 => {
                if let Some(item) = self.ammo[index - 6] {
                    set_item(player, item, integer, time);
                }
            }
            10 => {
                player.inventory.weapon = self
                    .weapons
                    .into_iter()
                    .flatten()
                    .find_map(|(native, weapon, _)| (native == bits).then_some(weapon))
                    .unwrap_or_default()
            }
            _ => {}
        }
        true
    }
    /// NQ clientdata carries its mask separately from the stat array. QW's
    /// STAT_ITEMS enters here too; unknown bits remain in the supplied binding.
    pub fn import_items(
        &self,
        binding: Option<ValueBinding>,
        bits: u32,
        player: &mut PlayerState,
        time: EventTime,
    ) -> bool {
        if !self.fits(player) {
            return false;
        }
        if let Some(binding) = binding
            && !binding.import(&mut player.values, bits)
        {
            return false;
        }
        for (mask, item) in self.items.into_iter().flatten() {
            set_item(player, item, i32::from(bits & mask != 0), time);
        }
        for (mask, _, item) in self.weapons.into_iter().flatten() {
            set_item(player, item, i32::from(bits & mask != 0), time);
        }
        player.armor_type = [32768, 16384, 8192]
            .into_iter()
            .zip(self.armor.into_iter().rev())
            .find_map(|(mask, item)| (bits & mask != 0).then_some(item).flatten())
            .unwrap_or_default();
        player.inventory.collectibles = u64::from(bits >> 28);
        true
    }
}
fn set_item(player: &mut PlayerState, item: ItemId, count: i32, time: EventTime) {
    let at = item.0 as usize;
    let acquired_at = if player.inventory.item_counts[at] <= 0 && count > 0 {
        time.seconds()
    } else {
        player.inventory.item_acquired_at[at]
    };
    player.inventory.apply(HudChange::Item {
        item,
        count,
        acquired_at,
    });
}
