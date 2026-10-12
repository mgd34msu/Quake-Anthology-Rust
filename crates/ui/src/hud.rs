use qa_core::events::TextStore;
use qa_core::primitives::{
    HudChange, HudLine, HudState, ItemId, NameId, PlayerState, PrintEvent, PrintKind,
};
use qa_gameplay::registry::{NativePlayerBindings, Registry};

#[derive(Clone, Copy)]
struct WeaponBinding {
    item: ItemId,
    ammo: Option<ItemId>,
}

pub struct HudBindings {
    pub values: crate::values::ValueLayout,
    weapons: Box<[Option<WeaponBinding>]>,
    items: usize,
    native_players: Box<[Option<NativePlayerBindings>; 5]>,
}

impl HudBindings {
    /// Resolved once from the common registry, including mixed character arsenals.
    pub fn load(registry: &Registry, extensions: &[crate::values::ExtensionValue]) -> Option<Self> {
        let values = crate::values::ValueLayout::load(extensions)?;
        let mut weapons = vec![None; registry.weapons.len() + 1].into_boxed_slice();
        for weapon in &registry.weapons {
            weapons[weapon.id.0 as usize] = Some(WeaponBinding {
                item: weapon.item,
                ammo: weapon.ammo,
            });
        }
        Some(Self {
            values,
            weapons,
            items: registry.items.len() + 1,
            native_players: Box::new(
                qa_core::primitives::RuleSetId::ALL
                    .map(|rules| NativePlayerBindings::load(rules, registry)),
            ),
        })
    }

    pub fn state(&self, powerups: usize, layout: NameId) -> HudState {
        let mut state = HudState::with_capacity(
            self.items,
            powerups,
            self.weapons.len(),
            self.values.capacity(),
        );
        state.layout = layout;
        state
    }

    pub fn project_stats(&self, player: &PlayerState, state: &mut HudState) {
        state.clipped_values = state
            .clipped_values
            .saturating_add(state.values.copy_from(&player.values) as u64);
        state.health = player.health;
        state.armor = player.armor;
        state.armor_type = player.armor_type;
        state.frags = player.frags;
        state.score = player.score;
    }
    /// Queued native stat receipt updates the one CLIENT player before HUD reads.
    pub fn import_native_stat(
        &self,
        rules: qa_core::primitives::RuleSetId,
        client: qa_core::primitives::ClientId,
        index: usize,
        bits: u32,
        player: &mut PlayerState,
        time: qa_core::sys_events::EventTime,
    ) -> bool {
        let Some(binding) = self.values.native(rules).stats.get(index).copied() else {
            return false;
        };
        if let Some(native) = self.native_players[rules as usize].as_ref() {
            native.import(
                Some(binding),
                qa_core::primitives::ResourceNamespace::Client(client),
                index,
                bits,
                player,
                time,
            )
        } else {
            binding.import(&mut player.values, bits)
        }
    }
    pub fn apply(&self, state: &mut HudState, change: HudChange) -> bool {
        if !state.inventory.apply(change) {
            return false;
        }
        match change {
            HudChange::Item { item, count, .. } => {
                let index = item.0 as usize;
                if count > 0 {
                    state.owned_items.insert(index);
                } else {
                    state.owned_items.remove(index);
                }
            }
            HudChange::Weapon(_) => {}
            HudChange::Powerup { .. } | HudChange::Collectibles(_) => return true,
        }
        state.owned_weapons.clear();
        for (weapon, binding) in self.weapons.iter().enumerate() {
            if let Some(binding) = binding
                && state
                    .inventory
                    .item_counts
                    .get(binding.item.0 as usize)
                    .is_some_and(|&count| count > 0)
            {
                state.owned_weapons.insert(weapon);
            }
        }
        state.ammo = self
            .weapons
            .get(state.inventory.weapon.0 as usize)
            .and_then(|binding| *binding)
            .and_then(|binding| binding.ammo)
            .and_then(|ammo| state.inventory.item_counts.get(ammo.0 as usize))
            .copied()
            .unwrap_or(0);
        true
    }
}

/// Recipient selection belongs to the frame dispatcher; source newlines stay in
/// the bounded TextStore. Layout drawing never reads mutable PlayerState.
pub fn print(
    state: &mut HudState,
    texts: &mut TextStore,
    event: PrintEvent,
    now: f64,
    notify_time: f64,
    center_time: f64,
) -> bool {
    if event.kind == PrintKind::Console {
        return true;
    }
    let Some(text) = texts.lease(event.text) else {
        return false;
    };
    match event.kind {
        PrintKind::Notify | PrintKind::Chat => {
            state.notify.rotate_left(1);
            if let Some(line) = state.notify[3].take() {
                texts.release(line.text);
            }
            state.notify[3] = Some(HudLine {
                text,
                started_at: now,
                until: now + notify_time,
            });
        }
        PrintKind::Center => {
            if let Some(line) = state.centerprint.take() {
                texts.release(line.text);
            }
            state.centerprint = Some(HudLine {
                text,
                started_at: now,
                until: now + center_time,
            });
        }
        PrintKind::Layout => {
            if let Some(old) = state.layout_text.replace(text) {
                texts.release(old);
            }
        }
        PrintKind::Console => {}
    }
    true
}
pub fn expire_messages(state: &mut HudState, texts: &mut TextStore, now: f64) {
    for line in &mut state.notify {
        if line.as_ref().is_some_and(|line| now >= line.until)
            && let Some(line) = line.take()
        {
            texts.release(line.text);
        }
    }
    if state
        .centerprint
        .as_ref()
        .is_some_and(|line| now >= line.until)
        && let Some(line) = state.centerprint.take()
    {
        texts.release(line.text);
    }
}
