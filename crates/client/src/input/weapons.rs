//! Weapon catalogs, default weapon keys, and `use` resolution.
//!
//! Donor provenance: `src/input/weapon-bindings.ts`. The catalog data
//! mirrors the content tables (`q1:weapon/*`, `q2:weapon_*`,
//! `q3:weapon/*` ids and display names); a running game may supply
//! its own [`WeaponBindingItem`] list instead.

/// Weapon catalog family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponFamily {
    /// Quake I.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Selectable item kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponBindingKind {
    /// Weapon.
    Weapon,
    /// Powerup.
    Powerup,
}

/// One selectable item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBindingItem {
    /// Canonical item id.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Item kind.
    pub kind: WeaponBindingKind,
}

impl WeaponBindingItem {
    /// New weapon item.
    #[must_use]
    pub fn weapon(id: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            kind: WeaponBindingKind::Weapon,
        }
    }
}

const Q1_WEAPONS: [(&str, &str); 8] = [
    ("axe", "Axe"),
    ("shotgun", "Shotgun"),
    ("supershotgun", "Double-barrelled Shotgun"),
    ("nailgun", "Nailgun"),
    ("supernailgun", "Super Nailgun"),
    ("grenadelauncher", "Grenade Launcher"),
    ("rocketlauncher", "Rocket Launcher"),
    ("lightning", "Thunderbolt"),
];

const Q1_MISSION_WEAPONS: [(&str, &str); 8] = [
    ("hipnotic:laser", "Laser Cannon"),
    ("hipnotic:mjolnir", "Mjolnir"),
    ("hipnotic:proximity", "Proximity Gun"),
    ("rogue:lava-nailgun", "Lava Nailgun"),
    ("rogue:lava-supernailgun", "Lava Supernailgun"),
    ("rogue:multi-grenade", "Multi Grenade"),
    ("rogue:multi-rocket", "Multi Rocket"),
    ("rogue:plasma", "Plasma"),
];

fn q1_weapon_display_name(weapon: &str) -> String {
    if let Some((_, label)) = Q1_WEAPONS.iter().find(|(id, _)| *id == weapon) {
        return (*label).to_string();
    }
    if let Some((_, label)) = Q1_MISSION_WEAPONS.iter().find(|(id, _)| *id == weapon) {
        return (*label).to_string();
    }
    weapon
        .rsplit(':')
        .next()
        .unwrap_or(weapon)
        .replace(['_', '-'], " ")
        .split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const Q2_BASE_WEAPONS: [(&str, &str, &str); 11] = [
    ("blaster", "q2:weapon_blaster", "Blaster"),
    ("shotgun", "q2:weapon_shotgun", "Shotgun"),
    ("supershotgun", "q2:weapon_supershotgun", "Super Shotgun"),
    ("machinegun", "q2:weapon_machinegun", "Machinegun"),
    ("chaingun", "q2:weapon_chaingun", "Chaingun"),
    ("grenades", "q2:ammo_grenades", "Grenades"),
    ("grenadelauncher", "q2:weapon_grenadelauncher", "Grenade Launcher"),
    ("rocketlauncher", "q2:weapon_rocketlauncher", "Rocket Launcher"),
    ("hyperblaster", "q2:weapon_hyperblaster", "HyperBlaster"),
    ("railgun", "q2:weapon_railgun", "Railgun"),
    ("bfg", "q2:weapon_bfg", "BFG10K"),
];

const Q2_XATRIX_WEAPONS: [(&str, &str, &str); 3] = [
    ("trap", "q2:ammo_trap", "Trap"),
    ("ionripper", "q2:weapon_boomer", "Ionripper"),
    ("phalanx", "q2:weapon_phalanx", "Phalanx"),
];

const Q2_ROGUE_WEAPONS: [(&str, &str, &str); 5] = [
    ("tesla", "q2:ammo_tesla", "Tesla"),
    ("proxlauncher", "q2:weapon_proxlauncher", "Prox Launcher"),
    ("chainfist", "q2:weapon_chainfist", "Chainfist"),
    ("disintegrator", "q2:weapon_disintegrator", "Disruptor"),
    ("etf_rifle", "q2:weapon_etf_rifle", "ETF Rifle"),
];

const Q3_WEAPONS: [(i32, &str, &str); 13] = [
    (1, "q3:weapon/gauntlet", "Gauntlet"),
    (2, "q3:weapon/machinegun", "Machinegun"),
    (3, "q3:weapon/shotgun", "Shotgun"),
    (4, "q3:weapon/grenadelauncher", "Grenade Launcher"),
    (5, "q3:weapon/rocketlauncher", "Rocket Launcher"),
    (6, "q3:weapon/lightning", "Lightning Gun"),
    (7, "q3:weapon/railgun", "Railgun"),
    (8, "q3:weapon/plasmagun", "Plasma Gun"),
    (9, "q3:weapon/bfg", "BFG10K"),
    (10, "q3:weapon/grapple", "Grappling Hook"),
    (11, "q3:weapon/nailgun", "Nailgun"),
    (12, "q3:weapon/proxlauncher", "Prox Launcher"),
    (13, "q3:weapon/chaingun", "Chaingun"),
];

/// Official boot catalog for a family, campaign, and edition.
#[must_use]
pub fn base_weapon_binding_items(family: WeaponFamily, campaign: &str, edition: &str) -> Vec<WeaponBindingItem> {
    match family {
        WeaponFamily::Q1 => {
            let mut weapons: Vec<String> = Q1_WEAPONS.iter().map(|(id, _)| (*id).to_string()).collect();
            if campaign == "mg3" {
                weapons.push("mg3:laser".to_string());
                weapons.push("mg3:mjolnir".to_string());
            }
            if campaign == "hipnotic" || campaign == "rogue" {
                for (id, _) in Q1_MISSION_WEAPONS {
                    if id.starts_with(campaign) {
                        weapons.push(id.to_string());
                    }
                }
            }
            weapons
                .iter()
                .map(|weapon| WeaponBindingItem::weapon(&format!("q1:weapon/{weapon}"), &q1_weapon_display_name(weapon)))
                .collect()
        }
        WeaponFamily::Q2 => {
            let mut weapons: Vec<(&str, &str)> = Q2_BASE_WEAPONS.iter().map(|(_, item, label)| (*item, *label)).collect();
            let rerelease_pack = edition == "rerelease" && ["baseq2", "xatrix", "rogue", "mg2"].contains(&campaign);
            if rerelease_pack {
                weapons.extend(Q2_XATRIX_WEAPONS.iter().map(|(_, item, label)| (*item, *label)));
                weapons.extend(Q2_ROGUE_WEAPONS.iter().map(|(_, item, label)| (*item, *label)));
            } else if campaign == "xatrix" {
                weapons.extend(Q2_XATRIX_WEAPONS.iter().map(|(_, item, label)| (*item, *label)));
            } else if campaign == "rogue" {
                weapons.extend(Q2_ROGUE_WEAPONS.iter().map(|(_, item, label)| (*item, *label)));
            }
            weapons.iter().map(|(item, label)| WeaponBindingItem::weapon(item, label)).collect()
        }
        WeaponFamily::Q3 => {
            let missionpack = campaign == "missionpack";
            Q3_WEAPONS
                .iter()
                .filter(|(number, _, _)| missionpack || *number <= 10)
                .map(|(_, item, label)| WeaponBindingItem::weapon(item, label))
                .collect()
        }
    }
}

/// Default number/letter keys for catalog weapons.
#[must_use]
pub fn default_weapon_bindings(items: &[WeaponBindingItem]) -> Vec<(String, String)> {
    let mut result: Vec<(String, String)> = Vec::new();
    let mut append = |id: &str, key: &str| {
        if !result.iter().any(|(assigned, _)| assigned == key)
            && items.iter().any(|item| item.kind == WeaponBindingKind::Weapon && item.id == id)
        {
            result.push((key.to_string(), format!("use {id}")));
        }
    };
    for (index, (weapon, _)) in Q1_WEAPONS.iter().enumerate() {
        append(&format!("q1:weapon/{weapon}"), &format!("{}", index + 1));
    }
    append("q1:weapon/hipnotic:laser", "9");
    append("q1:weapon/hipnotic:mjolnir", "0");
    append("q1:weapon/mg3:laser", "9");
    let mut slot = 1;
    for (name, item, _) in Q2_BASE_WEAPONS {
        if name == "grenades" {
            append(item, "g");
        } else {
            append(item, &format!("{}", slot % 10));
            slot += 1;
        }
    }
    for (number, item, _) in Q3_WEAPONS {
        if number <= 10 {
            append(item, &format!("{}", number % 10));
        }
    }
    result
}

fn normalized(value: &str) -> String {
    value.to_lowercase().replace(' ', "")
}

fn suspicious_argument(argument: &str) -> bool {
    argument.chars().any(|value| matches!(value, ';' | '\r' | '\n' | '\\' | '"'))
        || argument.contains("//")
        || argument.contains("/*")
}

/// Resolved weapon or powerup selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWeaponSelection {
    /// Item kind.
    pub kind: WeaponBindingKind,
    /// Canonical item id.
    pub item: String,
}

/// Resolve `use`, `weapon`, or `impulse` against the item catalog.
#[must_use]
pub fn resolve_weapon_selection(command: &str, args: &[&str], items: &[WeaponBindingItem]) -> Option<ResolvedWeaponSelection> {
    let name = command.to_lowercase();
    if name != "use" && name != "weapon" && name != "impulse" {
        return None;
    }
    if args.is_empty() || args.iter().any(|argument| suspicious_argument(argument)) {
        return None;
    }
    if name == "use" {
        let requested = normalized(&args.join(""));
        if let Some(canonical) = items.iter().find(|item| item.id == requested) {
            return Some(ResolvedWeaponSelection {
                kind: canonical.kind,
                item: canonical.id.clone(),
            });
        }
        let matching: Vec<&WeaponBindingItem> = items
            .iter()
            .filter(|item| {
                if normalized(&item.label) == requested {
                    return true;
                }
                let q1 = Q1_WEAPONS
                    .iter()
                    .map(|(id, _)| (*id).to_string())
                    .chain(Q1_MISSION_WEAPONS.iter().map(|(id, _)| (*id).to_string()))
                    .find(|weapon| format!("q1:weapon/{weapon}") == item.id);
                if let Some(q1) = q1 {
                    if normalized(&q1) == requested || normalized(&q1_weapon_display_name(&q1)) == requested {
                        return true;
                    }
                }
                if let Some(q3) = Q3_WEAPONS.iter().find(|(_, item_id, _)| *item_id == item.id) {
                    if normalized(q3.1.strip_prefix("q3:weapon/").unwrap_or(q3.1)) == requested {
                        return true;
                    }
                }
                let q2 = Q2_BASE_WEAPONS
                    .iter()
                    .map(|(qname, item_id, label)| (*qname, *item_id, *label))
                    .chain(Q2_XATRIX_WEAPONS.iter().map(|(qname, item_id, label)| (*qname, *item_id, *label)))
                    .chain(Q2_ROGUE_WEAPONS.iter().map(|(qname, item_id, label)| (*qname, *item_id, *label)))
                    .find(|(_, item_id, _)| *item_id == item.id);
                if let Some((qname, _, label)) = q2 {
                    if normalized(qname) == requested || normalized(label) == requested {
                        return true;
                    }
                }
                false
            })
            .collect();
        let item = if matching.len() == 1 { matching[0] } else { return None };
        return Some(ResolvedWeaponSelection {
            kind: item.kind,
            item: item.id.clone(),
        });
    }
    if args.len() != 1 {
        return None;
    }
    let argument = args[0];
    if argument.is_empty()
        || !argument.starts_with(|value: char| ('1'..='9').contains(&value))
        || !argument.chars().all(|value| value.is_ascii_digit())
    {
        return None;
    }
    let number: usize = argument.parse().ok()?;
    if name == "impulse"
        && items.iter().any(|item| {
            item.kind == WeaponBindingKind::Weapon
                && !Q1_WEAPONS.iter().any(|(weapon, _)| item.id == format!("q1:weapon/{weapon}"))
        })
    {
        return None;
    }
    let id = if name == "weapon" {
        Q3_WEAPONS.iter().find(|(tag, _, _)| *tag as usize == number)?.1.to_string()
    } else {
        format!("q1:weapon/{}", Q1_WEAPONS.get(number - 1)?.0)
    };
    let item = items.iter().find(|item| item.kind == WeaponBindingKind::Weapon && item.id == id)?;
    Some(ResolvedWeaponSelection {
        kind: item.kind,
        item: item.id.clone(),
    })
}

/// Resolve a lone selection command to its item id.
///
/// Returns `None` for scripts, aliases, and multi-command lines.
#[must_use]
pub fn weapon_binding_item(text: &str, items: &[WeaponBindingItem]) -> Option<String> {
    if text.chars().any(|value| matches!(value, ';' | '\r' | '\n' | '\\')) || text.contains("//") || text.contains("/*") {
        return None;
    }
    let trimmed = text.trim();
    let (name, rest) = trimmed.split_once(char::is_whitespace)?;
    if !name.eq_ignore_ascii_case("use") && !name.eq_ignore_ascii_case("weapon") && !name.eq_ignore_ascii_case("impulse") {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    let argument = if rest.starts_with('"') {
        let inner = rest.strip_prefix('"')?.strip_suffix('"')?.trim();
        if inner.is_empty() || inner.contains('"') || inner.contains('\r') || inner.contains('\n') {
            return None;
        }
        inner
    } else {
        if rest.contains('"') || rest.contains('\r') || rest.contains('\n') {
            return None;
        }
        rest
    };
    resolve_weapon_selection(name, &[argument], items).map(|selection| selection.item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_cover_families_and_campaigns() {
        let q1 = base_weapon_binding_items(WeaponFamily::Q1, "", "classic");
        assert_eq!(q1.len(), 8);
        assert_eq!(q1[2].label, "Double-barrelled Shotgun");
        let hipnotic = base_weapon_binding_items(WeaponFamily::Q1, "hipnotic", "classic");
        assert!(hipnotic.iter().any(|item| item.id == "q1:weapon/hipnotic:laser"));
        let mg3 = base_weapon_binding_items(WeaponFamily::Q1, "mg3", "classic");
        assert!(mg3.iter().any(|item| item.id == "q1:weapon/mg3:mjolnir"));
        let q2 = base_weapon_binding_items(WeaponFamily::Q2, "", "classic");
        assert_eq!(q2.len(), 11);
        let rogue = base_weapon_binding_items(WeaponFamily::Q2, "rogue", "classic");
        assert!(rogue.iter().any(|item| item.id == "q2:weapon_chainfist"));
        let rerelease = base_weapon_binding_items(WeaponFamily::Q2, "baseq2", "rerelease");
        assert!(rerelease.iter().any(|item| item.id == "q2:weapon_phalanx"));
        assert!(rerelease.iter().any(|item| item.id == "q2:weapon_etf_rifle"));
        let q3 = base_weapon_binding_items(WeaponFamily::Q3, "", "classic");
        assert_eq!(q3.len(), 10);
        let team_arena = base_weapon_binding_items(WeaponFamily::Q3, "missionpack", "classic");
        assert_eq!(team_arena.len(), 13);
    }

    #[test]
    fn default_keys_follow_source_slots() {
        let q1 = base_weapon_binding_items(WeaponFamily::Q1, "hipnotic", "classic");
        let bindings = default_weapon_bindings(&q1);
        assert!(bindings.contains(&("1".to_string(), "use q1:weapon/axe".to_string())));
        assert!(bindings.contains(&("9".to_string(), "use q1:weapon/hipnotic:laser".to_string())));
        assert!(bindings.contains(&("0".to_string(), "use q1:weapon/hipnotic:mjolnir".to_string())));
        let q2 = base_weapon_binding_items(WeaponFamily::Q2, "", "classic");
        let bindings = default_weapon_bindings(&q2);
        assert!(bindings.contains(&("g".to_string(), "use q2:ammo_grenades".to_string())));
        assert!(bindings.contains(&("1".to_string(), "use q2:weapon_blaster".to_string())));
        let q3 = base_weapon_binding_items(WeaponFamily::Q3, "", "classic");
        let bindings = default_weapon_bindings(&q3);
        assert!(bindings.contains(&("2".to_string(), "use q3:weapon/machinegun".to_string())));
        assert!(bindings.contains(&("0".to_string(), "use q3:weapon/grapple".to_string())));
    }

    #[test]
    fn selections_resolve_names_and_numbers() {
        let q1 = base_weapon_binding_items(WeaponFamily::Q1, "", "classic");
        assert_eq!(
            resolve_weapon_selection("use", &["Thunderbolt"], &q1).unwrap().item,
            "q1:weapon/lightning"
        );
        assert_eq!(resolve_weapon_selection("impulse", &["8"], &q1).unwrap().item, "q1:weapon/lightning");
        assert!(resolve_weapon_selection("impulse", &["09"], &q1).is_none());
        assert!(resolve_weapon_selection("use", &["a;b"], &q1).is_none());
        let q3 = base_weapon_binding_items(WeaponFamily::Q3, "", "classic");
        assert_eq!(
            resolve_weapon_selection("weapon", &["5"], &q3).unwrap().item,
            "q3:weapon/rocketlauncher"
        );
        assert_eq!(
            weapon_binding_item("use \"Rocket Launcher\"", &q3).unwrap(),
            "q3:weapon/rocketlauncher"
        );
        assert!(weapon_binding_item("use rl; echo hi", &q3).is_none());
        let expanded = base_weapon_binding_items(WeaponFamily::Q1, "hipnotic", "classic");
        assert!(resolve_weapon_selection("impulse", &["8"], &expanded).is_none());
    }
}
