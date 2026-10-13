//! Native player values enter the same inventory and value bank.
use super::Registry;
use qa_core::{
    primitives::{
        HudChange, ItemId, ModelRef, NQ_PACKET_STATS, PlayerState, PlayerValueBindings,
        Q3StatLayout, ResourceNamespace, RuleSetId, ValueBinding, WeaponId,
    },
    sys_events::EventTime,
};

#[derive(Clone, Copy)]
pub struct NativePlayerBindings {
    source: RuleSetId,
    weapons: [Option<(u32, WeaponId, ItemId)>; 16],
    items: [Option<(u32, ItemId)>; 10],
    ammo: [Option<ItemId>; 16],
    armor: [Option<ItemId>; 3],
    items_stat: bool,
}
impl NativePlayerBindings {
    /// Native bit positions are ABI data; registry identities remain independent.
    pub fn load(source: RuleSetId, registry: &Registry) -> Option<Self> {
        if !matches!(
            source,
            RuleSetId::Quake | RuleSetId::QuakeWorld | RuleSetId::Quake3
        ) {
            return None;
        }
        let mut weapons = [None; 16];
        let mut ammo = [None; 16];
        if source == RuleSetId::Quake3 {
            for (ordinal, entry) in weapons.iter_mut().enumerate() {
                if let Some(weapon) = registry.native_weapon(source, ordinal as u16)
                    && let Some(definition) = registry.weapon(weapon)
                {
                    *entry = Some((1 << ordinal, weapon, definition.item));
                    ammo[ordinal] = definition.ammo;
                }
            }
        } else {
            for (entry, native) in weapons.iter_mut().zip([1, 2, 4, 8, 16, 32, 64, 4096]) {
                *entry = registry.native_weapon(source, native).and_then(|weapon| {
                    registry
                        .weapon(weapon)
                        .map(|entry| (u32::from(native), weapon, entry.item))
                });
            }
            for (entry, native) in ammo.iter_mut().zip([11, 12, 13, 14]) {
                *entry = registry.native_item(source, native);
            }
        }
        Some(Self {
            source,
            weapons,
            // Original bothdefs.h IT_* values, paired with catalog ordinals.
            items: if source == RuleSetId::Quake3 {
                [None; 10]
            } else {
                [
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
                })
            },
            ammo,
            armor: if source == RuleSetId::Quake3 {
                [None; 3]
            } else {
                [2, 3, 4].map(|native| registry.native_item(source, native))
            },
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
    /// Admission precedes projection and inventory changes on both SERVER and
    /// CLIENT. Even semantically unknown rows must fit the bound value bank.
    pub fn packet_fits(
        &self,
        values: PlayerValueBindings<'_>,
        layout: Q3StatLayout,
        player: &PlayerState,
    ) -> bool {
        if !values.fits(&player.values) {
            return false;
        }
        match self.source {
            RuleSetId::Quake => {
                self.fits(player)
                    && values.items.is_some()
                    && NQ_PACKET_STATS
                        .into_iter()
                        .all(|index| index < values.arrays[0].len())
            }
            RuleSetId::Quake3 => {
                self.fits(player)
                    && values.arrays[0].len() == 16
                    && values.arrays[2].len() == 16
                    && layout.fits(values.arrays[0].len())
            }
            // QW's incremental STAT_* messages, not playerinfo, own inventory.
            _ => true,
        }
    }
    /// The native projection has already populated its sole bank. This one
    /// semantic importer is used after both native-memory and queued receipt.
    pub fn import_packet(
        &self,
        values: PlayerValueBindings<'_>,
        layout: Q3StatLayout,
        namespace: ResourceNamespace,
        player: &mut PlayerState,
        time: EventTime,
    ) -> bool {
        if !self.packet_fits(values, layout, player) {
            return false;
        }
        match self.source {
            RuleSetId::Quake => {
                for index in NQ_PACKET_STATS {
                    let Some(bits) = values.arrays[0][index].export(&player.values) else {
                        return false;
                    };
                    if !self.import(None, namespace, index, bits, player, time) {
                        return false;
                    }
                }
                let Some(bits) = values
                    .items
                    .and_then(|binding| binding.export(&player.values))
                else {
                    return false;
                };
                self.import_items(None, bits, player, time)
            }
            RuleSetId::Quake3 => {
                let Some(mask) = player
                    .values
                    .get(values.arrays[0][usize::from(layout.weapons)].id)
                else {
                    return false;
                };
                // NumericValue carries the signed bank value; wire narrowing
                // belongs to the native encoder, never this inventory import.
                for (ordinal, item) in self.ammo.into_iter().enumerate() {
                    if let Some(item) = item {
                        let Some(value) = player.values.get(values.arrays[2][ordinal].id) else {
                            return false;
                        };
                        set_item(player, item, value.as_integer(), time);
                    }
                }
                self.import_weapon_mask(mask.0, player, time);
                true
            }
            _ => true,
        }
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
        if self.source == RuleSetId::Quake3 {
            return binding.is_none_or(|binding| binding.import(&mut player.values, bits));
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
        if self.source == RuleSetId::Quake3 || !self.fits(player) {
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
        self.import_weapon_mask(bits, player, time);
        player.armor_type = [32768, 16384, 8192]
            .into_iter()
            .zip(self.armor.into_iter().rev())
            .find_map(|(mask, item)| (bits & mask != 0).then_some(item).flatten())
            .unwrap_or_default();
        player.inventory.collectibles = u64::from(bits >> 28);
        true
    }
    fn import_weapon_mask(&self, bits: u32, player: &mut PlayerState, time: EventTime) {
        for (mask, _, item) in self.weapons.into_iter().flatten() {
            set_item(player, item, i32::from(bits & mask != 0), time);
        }
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
