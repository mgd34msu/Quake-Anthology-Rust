//! Rerelease Quake weapon-wheel selection and layout.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/q1-wheel.ts`
//! (`q1WheelSlotItem`, `ApplicationQ1Wheel`). The `wwheel.txt` parser and the
//! `q1WheelItems` layout are the canonical [`qa_client::ui::hud::q1_wheel`] port, called
//! directly like the donor; the [`Q1WheelAssets`] seam keeps only the product gate, the
//! wheel file, and icon loading. Slots are stored as the local [`Q1WheelSlot`] projection
//! (the canonical `ammoIcon`/`unknown` fields are never read here) and converted at the
//! canonical call boundary. Player state
//! ([`simulation::types`](super::simulation::types), `./simulation/types.ts` port) is
//! shimmed to the fields this module reads.

use std::collections::HashMap;
use std::rc::Rc;

use qa_client::ui::hud::q1_wheel::{
    parse_q1_weapon_wheel, q1_wheel_items, Q1WheelSlot as HudQ1WheelSlot, Q1WheelState,
};
use qa_content::contract::{ContentId, ResourceId};
use thiserror::Error;

/// Base weapon items by impulse 1..=8 (donor `baseItems`).
const BASE_ITEMS: &[&str] = &[
    "q1:weapon/axe",
    "q1:weapon/shotgun",
    "q1:weapon/supershotgun",
    "q1:weapon/nailgun",
    "q1:weapon/supernailgun",
    "q1:weapon/grenadelauncher",
    "q1:weapon/rocketlauncher",
    "q1:weapon/lightning",
];

/// Map a wheel slot impulse to its item (donor `q1WheelSlotItem`).
#[must_use]
pub fn q1_wheel_slot_item(impulse: Option<i32>, campaign: &str) -> Option<String> {
    let impulse = impulse?;
    if (1..=8).contains(&impulse) {
        return BASE_ITEMS.get((impulse - 1) as usize).map(ToString::to_string);
    }
    if campaign == "hipnotic" {
        return match impulse {
            225 => Some("q1:weapon/hipnotic:laser".to_string()),
            226 => Some("q1:weapon/hipnotic:mjolnir".to_string()),
            227 => Some("q1:weapon/hipnotic:proximity".to_string()),
            228 => Some("q1:weapon/grenadelauncher".to_string()),
            _ => None,
        };
    }
    if campaign == "mg3" {
        return if impulse == 225 {
            Some("q1:weapon/mg3:laser".to_string())
        } else {
            None
        };
    }
    if campaign == "ctf" {
        return if impulse == 22 {
            Some("q1:weapon/ctf:grapple".to_string())
        } else {
            None
        };
    }
    if campaign == "rogue" {
        if impulse == 22 {
            return Some("q1:weapon/rogue:grapple".to_string());
        }
        const POWERED: &[&str] = &[
            "q1:weapon/rogue:lava-nailgun",
            "q1:weapon/rogue:lava-supernailgun",
            "q1:weapon/rogue:multi-grenade",
            "q1:weapon/rogue:multi-rocket",
            "q1:weapon/rogue:plasma",
        ];
        if (60..=64).contains(&impulse) {
            return POWERED.get((impulse - 60) as usize).map(ToString::to_string);
        }
        if (65..=68).contains(&impulse) {
            return BASE_ITEMS.get((impulse - 62) as usize).map(ToString::to_string);
        }
    }
    None
}

/// One parsed wheel slot (donor `Q1WheelSlot` fields this module reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1WheelSlot {
    /// Wheel ordinal.
    pub slot: i32,
    /// Weapon impulse, when the slot selects by impulse.
    pub impulse: Option<i32>,
    /// Icon path, when authored.
    pub icon: Option<String>,
    /// Selected-icon path, when authored.
    pub selected_icon: Option<String>,
    /// Progs entity byte offset for ammo counts, when authored.
    pub entity_variable_byte_offset: Option<i32>,
    /// Owned-bits mask, when authored.
    pub weapon_bits: Option<i32>,
}

/// One laid-out wheel item (donor `WheelItem`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1WheelItem {
    /// Item id (overwritten with the selected player item, else the wheel id).
    pub id: String,
    /// Source slot ordinal.
    pub source_ordinal: i32,
    /// Sort order.
    pub sort_order: i32,
    /// Display label.
    pub label: String,
    /// Whether the player owns the item.
    pub owned: bool,
    /// Whether the item has ammo.
    pub has_ammo: bool,
    /// Ammo count, when the slot reads one.
    pub count: Option<f64>,
    /// Warning count.
    pub warning_count: i32,
    /// Icon resource, when authored and loaded.
    pub icon: Option<ResourceId>,
    /// Selected-icon resource, when authored and loaded.
    pub selected_icon: Option<ResourceId>,
}

/// One player inventory item (donor `PlayerUiItem` fields this module reads).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1WheelPlayerItem {
    /// Item id.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Whether the player owns the item.
    pub owned: bool,
    /// Whether the item has ammo.
    pub has_ammo: bool,
    /// Ammo count, when tracked.
    pub count: Option<f64>,
}

/// Player state the wheel reads (donor `PlayerUi` fields this module reads).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1WheelPlayer {
    /// Weapon-status source content, when the player has weapon status.
    pub weapon_source_content: Option<ContentId>,
    /// Inventory items.
    pub items: Vec<Q1WheelPlayerItem>,
    /// Active weapon id, when armed.
    pub active_weapon: Option<String>,
}

/// Product identity the wheel reads (donor `catalog.product(content).expectation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1WheelProduct {
    /// Whether the product family is `q1`.
    pub q1_family: bool,
    /// Whether the product edition is `rerelease`.
    pub rerelease: bool,
    /// Campaign slug.
    pub campaign: String,
}

/// Product, wheel file, and icon seam (donor assets + icons). Parsing is the
/// canonical [`parse_q1_weapon_wheel`], called directly by [`ApplicationQ1Wheel::prepare`].
pub trait Q1WheelAssets {
    /// Product identity for content.
    fn product(&self, content: &ContentId) -> Q1WheelProduct;
    /// Raw `wwheel.txt` bytes, or [`None`] when the file is absent.
    fn wheel_text(&mut self, content: &ContentId) -> Option<Vec<u8>>;
    /// Load an icon image for content and path.
    fn load_icon(&mut self, content: &ContentId, path: &str) -> ResourceId;
}

/// Project a canonical wheel slot onto the fields this module reads.
fn hud_slot(slot: HudQ1WheelSlot) -> Q1WheelSlot {
    Q1WheelSlot {
        slot: slot.slot,
        impulse: slot.impulse,
        icon: slot.icon,
        selected_icon: slot.selected_icon,
        entity_variable_byte_offset: slot.entity_variable_byte_offset,
        weapon_bits: slot.weapon_bits,
    }
}

/// Failure to prepare the wheel, with donor messages.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q1WheelError {
    /// The authored wheel file failed to parse.
    #[error("Invalid authored weapon wheel: {0}")]
    InvalidWheel(String),
}

/// Rerelease weapon wheel (donor `ApplicationQ1Wheel`).
#[derive(Debug, Default)]
pub struct ApplicationQ1Wheel {
    content: Option<ContentId>,
    campaign: String,
    slots: Vec<Q1WheelSlot>,
    images: HashMap<String, ResourceId>,
}

impl ApplicationQ1Wheel {
    /// Build an empty wheel.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Prepare slots and icons for the player's content (donor `prepare`).
    pub fn prepare(&mut self, assets: &mut impl Q1WheelAssets, player: &Q1WheelPlayer) -> Result<(), Q1WheelError> {
        let Some(source) = &player.weapon_source_content else {
            self.slots.clear();
            self.content = None;
            return Ok(());
        };
        if Some(source) == self.content.as_ref() {
            return Ok(());
        }
        self.content = Some(source.clone());
        self.slots.clear();
        self.images.clear();
        let product = assets.product(source);
        if !product.q1_family || !product.rerelease {
            return Ok(());
        }
        self.campaign = product.campaign;
        let Some(bytes) = assets.wheel_text(source) else {
            return Ok(());
        };
        let parsed = parse_q1_weapon_wheel(&String::from_utf8_lossy(&bytes));
        if !parsed.errors.is_empty() {
            return Err(Q1WheelError::InvalidWheel(parsed.errors.join("; ")));
        }
        self.slots = parsed.slots.into_iter().map(hud_slot).collect();
        let mut icons = Vec::new();
        for slot in &self.slots {
            for path in [&slot.icon, &slot.selected_icon].into_iter().flatten() {
                if !self.images.contains_key(path) {
                    icons.push(path.clone());
                }
            }
        }
        for path in icons {
            let id = assets.load_icon(source, &path);
            self.images.insert(path, id);
        }
        Ok(())
    }

    /// Resolve one slot to its item for a player (donor `slotItem`).
    fn slot_item(&self, slot: &Q1WheelSlot, player: &Q1WheelPlayer) -> Option<String> {
        if self.campaign == "mg3"
            && slot.impulse == Some(1)
            && player
                .items
                .iter()
                .any(|item| item.id == "q1:weapon/mg3:mjolnir" && item.owned)
        {
            return Some("q1:weapon/mg3:mjolnir".to_string());
        }
        q1_wheel_slot_item(slot.impulse, &self.campaign)
    }

    /// Find the player's entry for a slot (donor `items` item lookup).
    fn player_item<'p>(&self, slot: &Q1WheelSlot, player: &'p Q1WheelPlayer) -> Option<&'p Q1WheelPlayerItem> {
        let id = self.slot_item(slot, player)?;
        player.items.iter().find(|item| item.id == id)
    }

    /// Toggle between two wheel ordinals (donor `switchWeapon`).
    #[must_use]
    pub fn switch_weapon(&self, player: &Q1WheelPlayer, first: i32, second: i32) -> Option<String> {
        let resolve = |ordinal: i32| {
            if ordinal < 0 {
                return None;
            }
            let id = match self.slots.iter().find(|slot| slot.slot == ordinal) {
                Some(slot) => self.slot_item(slot, player),
                None => {
                    let index = if ordinal == 7 { 0 } else { ordinal.checked_add(1)? };
                    BASE_ITEMS.get(usize::try_from(index).ok()?).map(ToString::to_string)
                }
            };
            let id = id?;
            player.items.iter().find(|item| item.id == id && item.owned)
        };
        let a = resolve(first);
        let b = resolve(second);
        let selected = if a.is_some_and(|item| Some(&item.id) == player.active_weapon.as_ref()) {
            b.or(a)
        } else {
            a.or(b)
        };
        selected.map(|item| item.id.clone())
    }

    /// Lay out the wheel for a player (donor `items`).
    #[must_use]
    pub fn items(&self, player: &Q1WheelPlayer) -> Option<Vec<Q1WheelItem>> {
        if self.slots.is_empty() {
            return None;
        }
        let bits =
            self.slots.iter().fold(0, |value, slot| {
                value
                    | self.player_item(slot, player).map_or(0, |item| {
                        if item.owned {
                            slot.weapon_bits.unwrap_or(0)
                        } else {
                            0
                        }
                    })
            });
        // The canonical layout takes `'static` closures, so entity counts and
        // labels are precomputed. A label is fixed by (ordinal, impulse); an
        // offset keeps its first slot's count, like the donor `find`.
        let mut counts: HashMap<i32, f64> = HashMap::new();
        for slot in &self.slots {
            if let Some(offset) = slot.entity_variable_byte_offset {
                counts.entry(offset).or_insert_with(|| {
                    self.player_item(slot, player)
                        .and_then(|item| item.count)
                        .unwrap_or(0.0)
                });
            }
        }
        let mut labels: HashMap<(i32, Option<i32>), String> = HashMap::new();
        for slot in &self.slots {
            labels.entry((slot.slot, slot.impulse)).or_insert_with(|| {
                self.player_item(slot, player).map_or_else(
                    || format!("Slot {}", slot.slot.saturating_add(1)),
                    |item| item.label.clone(),
                )
            });
        }
        let rows: Vec<HudQ1WheelSlot> = self
            .slots
            .iter()
            .map(|slot| HudQ1WheelSlot {
                slot: slot.slot,
                impulse: slot.impulse,
                icon: slot.icon.clone(),
                selected_icon: slot.selected_icon.clone(),
                ammo_icon: None,
                entity_variable_byte_offset: slot.entity_variable_byte_offset,
                weapon_bits: slot.weapon_bits,
                unknown: Vec::new(),
            })
            .collect();
        let counts_f32: HashMap<i32, f32> = counts.iter().map(|(offset, count)| (*offset, *count as f32)).collect();
        let state = Q1WheelState {
            item_bits: bits as u32,
            entity_float: Rc::new(move |offset| counts_f32.get(&offset).copied().unwrap_or(0.0)),
            label: Rc::new(move |slot: &HudQ1WheelSlot| {
                labels
                    .get(&(slot.slot, slot.impulse))
                    .cloned()
                    .unwrap_or_else(|| format!("Slot {}", slot.slot.saturating_add(1)))
            }),
            // Host icons are content ids, not client resources, so they are
            // filled from `images` after the canonical layout returns.
            image: Rc::new(|_| None),
        };
        let values: Vec<Q1WheelItem> = rows
            .iter()
            .zip(q1_wheel_items(&rows, &state))
            .map(|(slot, row)| {
                let count = slot
                    .entity_variable_byte_offset
                    .map(|offset| counts.get(&offset).copied().unwrap_or(0.0));
                Q1WheelItem {
                    id: row.id,
                    source_ordinal: row.source_ordinal,
                    sort_order: row.sort_order,
                    label: row.label,
                    owned: row.owned,
                    has_ammo: count.is_none_or(|count| count > 0.0),
                    count,
                    warning_count: row.warning_count,
                    icon: slot.icon.as_ref().and_then(|path| self.images.get(path).cloned()),
                    selected_icon: slot
                        .selected_icon
                        .as_ref()
                        .and_then(|path| self.images.get(path).cloned()),
                }
            })
            .collect();
        Some(
            values
                .into_iter()
                .map(|value| {
                    let selected = self
                        .slots
                        .iter()
                        .find(|slot| slot.slot == value.source_ordinal)
                        .and_then(|slot| self.player_item(slot, player));
                    Q1WheelItem {
                        id: selected.map_or(value.id.clone(), |item| item.id.clone()),
                        owned: selected.is_some_and(|item| item.owned),
                        has_ammo: selected.is_some_and(|item| item.has_ammo),
                        ..value
                    }
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        text: String,
        icons: u32,
    }

    impl Q1WheelAssets for Stub {
        fn product(&self, _content: &ContentId) -> Q1WheelProduct {
            Q1WheelProduct {
                q1_family: true,
                rerelease: true,
                campaign: "base".to_string(),
            }
        }
        fn wheel_text(&mut self, _content: &ContentId) -> Option<Vec<u8>> {
            Some(self.text.clone().into_bytes())
        }
        fn load_icon(&mut self, _content: &ContentId, path: &str) -> ResourceId {
            self.icons += 1;
            ResourceId(format!("icon:{}:{path}", self.icons))
        }
    }

    fn wheel_text() -> String {
        "slot 0 {\nimpulse 1\nicon icon0\nweaponnum 1\n}\nslot 1 {\nimpulse 2\nicon icon1\nweaponnum 2\n}\n".to_string()
    }

    fn player() -> Q1WheelPlayer {
        Q1WheelPlayer {
            weapon_source_content: Some(ContentId("q1:rerelease:base:1".to_string())),
            items: vec![
                Q1WheelPlayerItem {
                    id: "q1:weapon/shotgun".to_string(),
                    label: "Shotgun".to_string(),
                    owned: true,
                    has_ammo: true,
                    count: Some(12.0),
                },
                Q1WheelPlayerItem {
                    id: "q1:weapon/axe".to_string(),
                    label: "Axe".to_string(),
                    owned: true,
                    has_ammo: true,
                    count: None,
                },
            ],
            active_weapon: Some("q1:weapon/shotgun".to_string()),
        }
    }

    #[test]
    fn impulses_map_per_campaign() {
        assert_eq!(
            q1_wheel_slot_item(Some(2), "base").as_deref(),
            Some("q1:weapon/shotgun")
        );
        assert_eq!(q1_wheel_slot_item(None, "base"), None);
        assert_eq!(
            q1_wheel_slot_item(Some(226), "hipnotic").as_deref(),
            Some("q1:weapon/hipnotic:mjolnir")
        );
        assert_eq!(
            q1_wheel_slot_item(Some(22), "rogue").as_deref(),
            Some("q1:weapon/rogue:grapple")
        );
        assert_eq!(
            q1_wheel_slot_item(Some(62), "rogue").as_deref(),
            Some("q1:weapon/rogue:multi-grenade")
        );
        assert_eq!(
            q1_wheel_slot_item(Some(66), "rogue").as_deref(),
            Some("q1:weapon/supernailgun")
        );
        assert_eq!(q1_wheel_slot_item(Some(9), "base"), None);
    }

    #[test]
    fn prepare_loads_icons_once_per_path() {
        let mut wheel = ApplicationQ1Wheel::new();
        let mut assets = Stub {
            text: wheel_text(),
            icons: 0,
        };
        wheel.prepare(&mut assets, &player()).unwrap();
        assert_eq!(assets.icons, 2);
        let items = wheel.items(&player()).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].id, "q1:weapon/shotgun");
        assert!(items[1].owned);
        assert!(items[0].icon.is_some());
    }

    #[test]
    fn empty_wheel_has_no_items() {
        let wheel = ApplicationQ1Wheel::new();
        assert!(wheel.items(&player()).is_none());
    }

    #[test]
    fn switch_toggles_off_active() {
        let mut wheel = ApplicationQ1Wheel::new();
        let mut assets = Stub {
            text: wheel_text(),
            icons: 0,
        };
        wheel.prepare(&mut assets, &player()).unwrap();
        assert_eq!(wheel.switch_weapon(&player(), 1, 0).as_deref(), Some("q1:weapon/axe"));
        assert_eq!(wheel.switch_weapon(&player(), -1, -2), None);
    }

    #[test]
    fn invalid_wheel_reports_errors() {
        struct Bad;
        impl Q1WheelAssets for Bad {
            fn product(&self, _content: &ContentId) -> Q1WheelProduct {
                Q1WheelProduct {
                    q1_family: true,
                    rerelease: true,
                    campaign: "base".to_string(),
                }
            }
            fn wheel_text(&mut self, _content: &ContentId) -> Option<Vec<u8>> {
                Some(b"nope".to_vec())
            }
            fn load_icon(&mut self, _content: &ContentId, _path: &str) -> ResourceId {
                ResourceId("icon".to_string())
            }
        }
        let mut wheel = ApplicationQ1Wheel::new();
        let error = wheel.prepare(&mut Bad, &player()).unwrap_err();
        assert_eq!(
            error,
            Q1WheelError::InvalidWheel("line 1: expected slot N {".to_string())
        );
    }
}
