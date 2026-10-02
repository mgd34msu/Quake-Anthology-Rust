//! Rerelease Quake weapon-wheel selection and layout.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q1-wheel.ts`
//! (`q1WheelSlotItem`, `ApplicationQ1Wheel`). The `wwheel.txt` parser
//! (`src/ui/hud/q1-wheel.ts`, out of scope) stays host-owned through the [`Q1WheelAssets`]
//! seam, which also provides the product gate, the wheel file, and icon loading; the
//! eight-line `q1WheelItems` layout it calls is inlined below with citation and must
//! delegate to the real port when `ui/hud/q1-wheel.ts` lands. Player state
//! (`./simulation/types.ts`, out of scope) is shimmed to the fields this module reads.

use std::collections::HashMap;

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

/// Product, wheel file, parser, and icon seam (donor assets + icons + parser).
pub trait Q1WheelAssets {
    /// Product identity for content.
    fn product(&self, content: &ContentId) -> Q1WheelProduct;
    /// Raw `wwheel.txt` bytes, or [`None`] when the file is absent.
    fn wheel_text(&mut self, content: &ContentId) -> Option<Vec<u8>>;
    /// Parse wheel text with the out-of-scope parser; [`Err`] carries its error list.
    fn parse_wheel(&mut self, content: &ContentId, text: &str) -> Result<Vec<Q1WheelSlot>, Vec<String>>;
    /// Load an icon image for content and path.
    fn load_icon(&mut self, content: &ContentId, path: &str) -> ResourceId;
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
        let slots = assets
            .parse_wheel(source, &String::from_utf8_lossy(&bytes))
            .map_err(|errors| Q1WheelError::InvalidWheel(errors.join("; ")))?;
        self.slots = slots;
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
        let entity_float = |offset: i32| {
            self.slots
                .iter()
                .find(|slot| slot.entity_variable_byte_offset == Some(offset))
                .map_or(0.0, |slot| {
                    self.player_item(slot, player)
                        .and_then(|item| item.count)
                        .unwrap_or(0.0)
                })
        };
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
        // Inlined `q1WheelItems` (src/ui/hud/q1-wheel.ts, out of scope): lay out one
        // WheelItem per slot from owned bits, entity counts, labels, and images.
        let values: Vec<Q1WheelItem> = self
            .slots
            .iter()
            .map(|slot| {
                let item = self.player_item(slot, player);
                let count = slot.entity_variable_byte_offset.map(entity_float);
                Q1WheelItem {
                    id: format!("q1-wheel:{}", slot.slot),
                    source_ordinal: slot.slot,
                    sort_order: slot.slot,
                    label: item.map_or_else(
                        || format!("Slot {}", slot.slot.saturating_add(1)),
                        |item| item.label.clone(),
                    ),
                    owned: slot.weapon_bits.is_some_and(|mask| bits & mask != 0),
                    has_ammo: count.is_none_or(|count| count > 0.0),
                    count,
                    warning_count: 0,
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
        slots: Vec<Q1WheelSlot>,
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
            Some(b"wheel".to_vec())
        }
        fn parse_wheel(&mut self, _content: &ContentId, _text: &str) -> Result<Vec<Q1WheelSlot>, Vec<String>> {
            Ok(self.slots.clone())
        }
        fn load_icon(&mut self, _content: &ContentId, path: &str) -> ResourceId {
            self.icons += 1;
            ResourceId(format!("icon:{}:{path}", self.icons))
        }
    }

    fn slot(ordinal: i32, impulse: Option<i32>) -> Q1WheelSlot {
        Q1WheelSlot {
            slot: ordinal,
            impulse,
            icon: Some(format!("icon{ordinal}")),
            selected_icon: None,
            entity_variable_byte_offset: None,
            weapon_bits: Some(1 << ordinal),
        }
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
            slots: vec![slot(0, Some(1)), slot(1, Some(2))],
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
            slots: vec![slot(0, Some(1)), slot(1, Some(2))],
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
                Some(Vec::new())
            }
            fn parse_wheel(&mut self, _content: &ContentId, _text: &str) -> Result<Vec<Q1WheelSlot>, Vec<String>> {
                Err(vec!["bad header".to_string()])
            }
            fn load_icon(&mut self, _content: &ContentId, _path: &str) -> ResourceId {
                ResourceId("icon".to_string())
            }
        }
        let mut wheel = ApplicationQ1Wheel::new();
        let error = wheel.prepare(&mut Bad, &player()).unwrap_err();
        assert_eq!(error, Q1WheelError::InvalidWheel("bad header".to_string()));
    }
}
