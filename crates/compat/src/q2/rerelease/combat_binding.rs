//! Q2 rerelease combat bindings over native armor and health stores.
//!
//! Donor: `src/compat/q2/rerelease/combat-binding.ts` — bridges the
//! declared original DLL armor rules, damage callbacks and health stores.

use std::collections::HashMap;

use qa_world::combat::{ArmorState, CombatState, ItemId, PoweredProtection, RegularArmor};
use thiserror::Error;

/// Combat binding failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CombatBindingError {
    /// Native armor item is absent.
    #[error("Native armor item is absent: {0}")]
    AbsentItem(String),
    /// Native armor item is absent.
    #[error("Native armor item is absent")]
    AbsentArmor,
    /// Native Q2 armor cannot store another game's armor record.
    #[error("Native Q2 armor cannot store another game's armor record")]
    ForeignArmor,
    /// Native power activation requires its original source equipment operation.
    #[error("Native power activation requires its original source equipment operation")]
    PowerActivation,
    /// Native non-client armor requires its own source declaration.
    #[error("Native non-client armor requires its own source declaration")]
    NonClientArmor,
    /// Unknown native armor item.
    #[error("Unknown native armor item")]
    UnknownArmor,
    /// No combat binding for this slot.
    #[error("Native target lacks a combat binding")]
    NoBinding,
}

/// Flag bits from the retail world profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatFlagBits {
    /// Godmode bit.
    pub godmode: u64,
    /// Notarget bit.
    pub notarget: u64,
    /// No-knockback bit.
    pub no_knockback: u64,
    /// Power-armor bit.
    pub power_armor: u64,
}

/// Retail flag bits.
#[must_use]
pub const fn retail_flag_bits() -> CombatFlagBits {
    CombatFlagBits {
        godmode: 16,
        notarget: 32,
        no_knockback: 2048,
        power_armor: 4096,
    }
}

/// Armor metadata from the retail world profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatArmorConfig {
    /// Regular armor items in preference order.
    pub regular: Vec<ItemId>,
    /// Empty armor item.
    pub empty: ItemId,
    /// Screen item.
    pub screen: ItemId,
    /// Shield item.
    pub shield: ItemId,
    /// Cell item.
    pub cells: ItemId,
}

/// Retail armor configuration.
#[must_use]
pub fn retail_armor_config() -> CombatArmorConfig {
    CombatArmorConfig {
        regular: vec![
            "q2:item_armor_jacket".to_string(),
            "q2:item_armor_combat".to_string(),
            "q2:item_armor_body".to_string(),
        ],
        empty: "q2:item_armor_body".to_string(),
        screen: "q2:item_power_screen".to_string(),
        shield: "q2:item_power_shield".to_string(),
        cells: "q2:ammo_cells".to_string(),
    }
}

/// Synthetic client counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientCounters {
    /// Inventory counters by item.
    pub inventory: HashMap<ItemId, i32>,
    /// Capture-turret team.
    pub ctf_team: i32,
    /// Invulnerability deadline in milliseconds.
    pub invincible_until: i64,
}

/// Synthetic combat slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatSlot {
    /// Entity flags.
    pub flags: u64,
    /// Health.
    pub health: i32,
    /// Mass.
    pub mass: i32,
    /// Takes damage.
    pub takedamage: bool,
    /// Client counters, if a client.
    pub client: Option<ClientCounters>,
    /// Server flags (monsters use bit 2 for the accumulator).
    pub svflags: u32,
    /// Monster invulnerability deadline.
    pub monster_invincible_until: i64,
    /// Fallback team id from public state.
    pub public_team_id: i32,
}

/// The declared original DLL owns armor rules, damage callbacks and
/// health stores. Headless port over synthetic slots.
pub struct RereleaseCombatBindings {
    /// Combat slots by entity slot.
    pub slots: HashMap<u32, CombatSlot>,
    /// Protection fractions by armor item: (normal, energy).
    pub armor_info: HashMap<ItemId, (f32, f32)>,
    /// Armor configuration.
    pub armor: CombatArmorConfig,
    /// Flag bits.
    pub flags: CombatFlagBits,
    /// Level time in milliseconds.
    pub time_ms: i64,
    /// Whether the world profile is selected.
    pub enabled: bool,
}

impl RereleaseCombatBindings {
    /// Create over synthetic slots.
    #[must_use]
    pub fn new() -> Self {
        let mut armor_info = HashMap::new();
        armor_info.insert("q2:item_armor_jacket".to_string(), (0.3, 0.3));
        armor_info.insert("q2:item_armor_combat".to_string(), (0.6, 0.6));
        armor_info.insert("q2:item_armor_body".to_string(), (0.8, 0.8));
        Self {
            slots: HashMap::new(),
            armor_info,
            armor: retail_armor_config(),
            flags: retail_flag_bits(),
            time_ms: 0,
            enabled: true,
        }
    }

    fn slot(&self, slot: u32) -> Result<&CombatSlot, CombatBindingError> {
        self.slots.get(&slot).ok_or(CombatBindingError::NoBinding)
    }

    /// Notarget flag for a slot, or `None` without a world profile.
    #[must_use]
    pub fn notarget(&self, slot: u32) -> Option<bool> {
        if !self.enabled {
            return None;
        }
        self.slots
            .get(&slot)
            .map(|slot| (slot.flags & self.flags.notarget) != 0)
    }

    fn protection(&self, item: &str, energy: bool) -> Result<f32, CombatBindingError> {
        self.armor_info
            .get(item)
            .map(|info| if energy { info.1 } else { info.0 })
            .ok_or_else(|| CombatBindingError::AbsentItem(item.to_string()))
    }

    /// Read effective armor for a slot.
    pub fn read_armor(&self, slot: u32) -> Result<ArmorState, CombatBindingError> {
        let record = self.slot(slot)?;
        let Some(client) = &record.client else {
            return Ok(ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::None,
            });
        };
        let regular = self
            .armor
            .regular
            .iter()
            .find(|item| client.inventory.get(*item).copied().unwrap_or(0) > 0)
            .cloned();
        let powered = (record.flags & self.flags.power_armor) != 0;
        let power = if !powered {
            None
        } else if client.inventory.get(&self.armor.shield).copied().unwrap_or(0) > 0 {
            Some(PoweredProtection::Shield {
                cells: client.inventory.get(&self.armor.cells).copied().unwrap_or(0),
            })
        } else if client.inventory.get(&self.armor.screen).copied().unwrap_or(0) > 0 {
            Some(PoweredProtection::Screen {
                cells: client.inventory.get(&self.armor.cells).copied().unwrap_or(0),
            })
        } else {
            None
        };
        match (&regular, &power) {
            (None, None) => Ok(ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::None,
            }),
            _ => Ok(ArmorState {
                regular: match regular {
                    None => RegularArmor::None,
                    Some(item) => RegularArmor::Q2 {
                        points: f64::from(client.inventory.get(&item).copied().unwrap_or(0)),
                        normal_protection: f64::from(self.protection(&item, false)?),
                        energy_protection: f64::from(self.protection(&item, true)?),
                        item,
                    },
                },
                powered: power.unwrap_or(PoweredProtection::None),
            }),
        }
    }

    /// Validate armor before a source write.
    pub fn validate_armor(&self, slot: u32, state: &ArmorState) -> Result<(), CombatBindingError> {
        if !matches!(state.regular, RegularArmor::None | RegularArmor::Q2 { .. }) {
            return Err(CombatBindingError::ForeignArmor);
        }
        if std::mem::discriminant(&state.powered) != std::mem::discriminant(&self.read_armor(slot)?.powered) {
            return Err(CombatBindingError::PowerActivation);
        }
        Ok(())
    }

    /// Write armor to source counters.
    pub fn write_armor(&mut self, slot: u32, state: &ArmorState) -> Result<(), CombatBindingError> {
        self.validate_armor(slot, state)?;
        if let RegularArmor::Q2 { points, item, .. } = &state.regular {
            if *points != 0.0 && !self.armor.regular.iter().any(|known| known == item) {
                return Err(CombatBindingError::UnknownArmor);
            }
        }
        let current = self.read_armor(slot)?.regular;
        let record = self.slots.get_mut(&slot).ok_or(CombatBindingError::NoBinding)?;
        let Some(client) = record.client.as_mut() else {
            if state.regular != RegularArmor::None || state.powered != PoweredProtection::None {
                return Err(CombatBindingError::NonClientArmor);
            }
            return Ok(());
        };
        for item in self.armor.regular.clone() {
            if let (RegularArmor::Q2 { item: next, .. }, RegularArmor::Q2 { item: kept, .. }) =
                (&state.regular, &current)
            {
                if next == kept && item != *kept {
                    continue;
                }
            }
            let points = match &state.regular {
                RegularArmor::Q2 { item: next, points, .. } if next == &item => *points as i32,
                _ => 0,
            };
            client.inventory.insert(item, points);
        }
        match &state.powered {
            PoweredProtection::None => {}
            PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells } => {
                client.inventory.insert(self.armor.cells.clone(), *cells);
            }
        }
        Ok(())
    }

    /// Empty regular armor record with source protection fractions.
    pub fn empty_regular_armor(&self, points: f64) -> Result<RegularArmor, CombatBindingError> {
        Ok(RegularArmor::Q2 {
            points,
            normal_protection: f64::from(self.protection(&self.armor.empty.clone(), false)?),
            energy_protection: f64::from(self.protection(&self.armor.empty.clone(), true)?),
            item: self.armor.empty.clone(),
        })
    }

    /// Read shared combat state with source semantic traits.
    pub fn combat_state(&self, slot: u32) -> Result<CombatState, CombatBindingError> {
        let record = self.slot(slot)?;
        let armor = self.read_armor(slot)?;
        let (team, invincible_until) = match &record.client {
            Some(client) => (
                if client.ctf_team > 0 {
                    Some(format!("q2:{}", client.ctf_team))
                } else {
                    None
                },
                client.invincible_until,
            ),
            None => (
                if record.public_team_id > 0 {
                    Some(format!("q2:{}", record.public_team_id))
                } else {
                    None
                },
                if (record.svflags & 4) != 0 {
                    record.monster_invincible_until
                } else {
                    0
                },
            ),
        };
        Ok(CombatState {
            health: f64::from(record.health),
            armor,
            mass: f64::from(record.mass),
            can_take_damage: record.takedamage,
            invulnerable: (record.flags & self.flags.godmode) != 0 || invincible_until > self.time_ms,
            no_knockback: (record.flags & self.flags.no_knockback) != 0,
            team,
        })
    }

    /// Write health to the source store.
    pub fn write_health(&mut self, slot: u32, health: f64) -> Result<(), CombatBindingError> {
        let record = self.slots.get_mut(&slot).ok_or(CombatBindingError::NoBinding)?;
        record.health = health as i32;
        Ok(())
    }
}

impl Default for RereleaseCombatBindings {
    fn default() -> Self {
        Self::new()
    }
}

/// Old power-only views fabricated regular armor using the exact
/// source-owned placeholder item.
#[must_use]
pub fn normalize_legacy_power_only_armor(legacy: &ArmorState, current: &ArmorState, placeholder: &str) -> ArmorState {
    let module_powered = |powered: &PoweredProtection| match powered {
        PoweredProtection::None => None,
        PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells } => Some(*cells),
    };
    let strip = current.regular == RegularArmor::None
        && matches!(
            &legacy.regular,
            RegularArmor::Q2 {
                points,
                normal_protection,
                energy_protection,
                item,
            } if *points == 0.0
                && *normal_protection == 0.0
                && *energy_protection == 0.0
                && item == placeholder
        )
        && std::mem::discriminant(&legacy.powered) == std::mem::discriminant(&current.powered)
        && module_powered(&legacy.powered).is_some()
        && module_powered(&legacy.powered) == module_powered(&current.powered);
    if strip {
        ArmorState {
            regular: RegularArmor::None,
            powered: legacy.powered.clone(),
        }
    } else {
        legacy.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bindings() -> RereleaseCombatBindings {
        let mut bindings = RereleaseCombatBindings::new();
        let mut inventory = HashMap::new();
        inventory.insert("q2:item_armor_combat".to_string(), 60);
        inventory.insert("q2:ammo_cells".to_string(), 40);
        inventory.insert("q2:item_power_shield".to_string(), 1);
        bindings.slots.insert(
            2,
            CombatSlot {
                flags: 4096,
                health: 90,
                mass: 200,
                takedamage: true,
                client: Some(ClientCounters {
                    inventory,
                    ctf_team: 1,
                    invincible_until: 0,
                }),
                svflags: 0,
                monster_invincible_until: 0,
                public_team_id: 0,
            },
        );
        bindings
    }

    #[test]
    fn armor_reads_counters_and_flags() {
        let bindings = bindings();
        assert_eq!(bindings.notarget(2), Some(false));
        let armor = bindings.read_armor(2).expect("armor");
        match &armor.regular {
            RegularArmor::Q2 {
                points,
                item,
                normal_protection,
                ..
            } => {
                assert_eq!(*points, 60.0);
                assert_eq!(item, "q2:item_armor_combat");
                assert_eq!(*normal_protection, f64::from(0.6f32));
            }
            other => panic!("regular: {other:?}"),
        }
        assert_eq!(armor.powered, PoweredProtection::Shield { cells: 40 });
        let state = bindings.combat_state(2).expect("state");
        assert_eq!(state.health, 90.0);
        assert_eq!(state.team, Some("q2:1".to_string()));
        assert!(!state.invulnerable);
        let empty = bindings.empty_regular_armor(0.0).expect("empty");
        assert!(matches!(empty, RegularArmor::Q2 { .. }));
    }

    #[test]
    fn armor_writes_validate_and_normalize() {
        let mut bindings = bindings();
        let armor = bindings.read_armor(2).expect("armor");
        bindings.validate_armor(2, &armor).expect("valid");
        let foreign = ArmorState {
            regular: RegularArmor::Q3 {
                points: 10.0,
                protection: 0.5,
            },
            powered: PoweredProtection::None,
        };
        assert_eq!(
            bindings.validate_armor(2, &foreign).unwrap_err(),
            CombatBindingError::ForeignArmor
        );
        let plain = ArmorState {
            regular: RegularArmor::None,
            powered: PoweredProtection::Shield { cells: 40 },
        };
        bindings.write_armor(2, &plain).expect("write");
        let back = bindings.read_armor(2).expect("read");
        assert_eq!(back.regular, RegularArmor::None);
        let legacy = ArmorState {
            regular: RegularArmor::Q2 {
                points: 0.0,
                normal_protection: 0.0,
                energy_protection: 0.0,
                item: "q2:none".to_string(),
            },
            powered: PoweredProtection::Shield { cells: 40 },
        };
        let current = ArmorState {
            regular: RegularArmor::None,
            powered: PoweredProtection::Shield { cells: 40 },
        };
        let normalized = normalize_legacy_power_only_armor(&legacy, &current, "q2:none");
        assert_eq!(normalized.regular, RegularArmor::None);
        bindings.write_health(2, 55.0).expect("health");
        assert_eq!(bindings.combat_state(2).expect("state").health, 55.0);
    }
}
