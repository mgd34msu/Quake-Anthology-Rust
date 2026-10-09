use qa_core::events::TextStore;
use qa_core::primitives::{HudLine, HudState, ItemId, NameId, PlayerState, PrintEvent, PrintKind};
use qa_gameplay::registry::Registry;

#[derive(Clone, Copy)]
struct WeaponBinding {
    item: ItemId,
    ammo: Option<ItemId>,
}

pub struct HudBindings {
    pub values: crate::values::ValueLayout,
    weapons: Box<[Option<WeaponBinding>]>,
    items: usize,
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

    pub fn update(&self, player: &PlayerState, state: &mut HudState) {
        state.clipped_values = state
            .clipped_values
            .saturating_add(state.values.copy_from(&player.values) as u64);
        state.health = player.health;
        state.armor = player.armor;
        state.armor_type = player.armor_type;
        state.weapon = player.weapon;
        state.frags = player.frags;
        state.score = player.score;
        state.collectibles = player.collectibles;
        state.item_counts.fill(0);
        state.item_acquired_at.fill(0.0);
        state.powerup_until.fill(0.0);
        for (out, &count) in state.item_counts.iter_mut().zip(&player.inventory) {
            *out = count;
        }
        for (out, &time) in state
            .item_acquired_at
            .iter_mut()
            .zip(&player.item_acquired_at)
        {
            *out = time;
        }
        for (out, &time) in state.powerup_until.iter_mut().zip(&player.powerup_until) {
            *out = time;
        }
        state.owned_items.fill(0);
        for (item, &count) in state.item_counts.iter().enumerate() {
            if count > 0 {
                state.owned_items[item / 64] |= 1 << (item % 64);
            }
        }
        state.owned_weapons.fill(0);
        for (weapon, binding) in self.weapons.iter().enumerate() {
            if let Some(binding) = binding
                && player
                    .inventory
                    .get(binding.item.0 as usize)
                    .is_some_and(|&count| count > 0)
                && let Some(word) = state.owned_weapons.get_mut(weapon / 64)
            {
                *word |= 1 << (weapon % 64);
            }
        }
        state.ammo = self
            .weapons
            .get(player.weapon.0 as usize)
            .and_then(|binding| *binding)
            .and_then(|binding| binding.ammo)
            .and_then(|ammo| player.inventory.get(ammo.0 as usize))
            .copied()
            .unwrap_or(0);
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
