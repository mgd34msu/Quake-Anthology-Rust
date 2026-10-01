//! Selected Q3 bot weapon metadata parsed once from source files.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-selected-knowledge.ts`
//! (`nativeQ3WeaponKnowledge`).
//!
//! Parse the selected Q3 source weapon metadata once; retain values, not
//! botlib allocations. The Rust botlib consolidates script sources, weights,
//! and memory behind [`WeaponAi`], so the port loads `weapons.c` through
//! that surface instead of constructing the donor stages.

use qa_bots::behavior::assets::BotSourceFiles;
use qa_bots::behavior::library::weapons::{WeaponAi, WeaponLoadResult};
use qa_bots::behavior::rerelease::data::botdata::{BotWeaponIdentity, WeaponEntry};
use qa_bots::behavior::rerelease::data::knowledge::BotWeaponT;
use qa_content::q3::foundation::arsenal::Q3_WEAPON_ITEMS;
use thiserror::Error;

/// Selected knowledge failures.
#[derive(Debug, Error)]
pub enum BotKnowledgeError {
    /// Selected Q3 bot weapon metadata is unavailable.
    #[error("Selected Q3 bot weapon metadata is unavailable")]
    Unavailable,
}

/// Parse selected Q3 source weapon metadata once.
pub fn native_q3_weapon_knowledge(files: &dyn BotSourceFiles) -> Result<Vec<BotWeaponT>, BotKnowledgeError> {
    let mut reader = WeaponAi::new(files);
    if reader.load_weapons("weapons.c") != WeaponLoadResult::NoError {
        return Err(BotKnowledgeError::Unavailable);
    }
    let mut result = Vec::new();
    for binding in Q3_WEAPON_ITEMS.iter() {
        let weapon = binding.weapon as i32;
        let Some(info) = reader.get_weapon_info(weapon) else {
            continue;
        };
        if !info.valid || info.projectile_info.damage <= 0.0 {
            continue;
        }
        let mut entry = WeaponEntry::default();
        let number = 1 << weapon;
        entry.name = binding.item.clone();
        entry.number = number;
        entry.identity = BotWeaponIdentity::Q1Bit { bit: number };
        entry.damage = f64::from(info.projectile_info.damage) * f64::from(info.projectile_count);
        entry.speed = f64::from(info.speed);
        entry.min_range = f64::from(info.projectile_info.radius);
        entry.max_range = if weapon == 1 {
            60.0
        } else if weapon == 6 {
            768.0
        } else {
            4096.0
        };
        entry.priority = entry.damage / 0.05_f64.max(f64::from(info.reload));
        entry.ammo_name = binding.ammo.clone().unwrap_or_default();
        entry.ammo = entry.ammo_name.clone();
        entry.min_ammo = f64::from(info.ammo_amount);
        entry.max_ammo = 200.0;
        let mut flags = vec![if weapon == 1 {
            "melee"
        } else if info.speed == 0.0 {
            "hitscan"
        } else {
            "projectile"
        }
        .to_string()];
        if info.projectile_info.radius > 0.0 {
            flags.push("explosive".to_string());
        }
        if info.projectile_info.gravity > 0.0 {
            flags.push("parabolic".to_string());
        }
        entry.flags = flags;
        entry.aim_point = if info.projectile_info.radius > 0.0 {
            "feet"
        } else {
            "center"
        }
        .to_string();
        result.push(BotWeaponT {
            entry,
            is_melee: weapon == 1,
            is_electric: false,
            needs_ammo: binding.ammo.is_some(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeFiles {
        weapons: Option<String>,
    }

    impl BotSourceFiles for FakeFiles {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            if path == "weapons.c" {
                self.weapons.as_ref().map(|text| text.as_bytes().to_vec())
            } else {
                None
            }
        }

        fn list(&self, _directory: &str, _extension: &str) -> Vec<String> {
            Vec::new()
        }
    }

    fn weapons_c() -> String {
        [
            "projectileinfo { name gauntlet_hit; damage 50; radius 0; gravity 0; }",
            "projectileinfo { name rocket; damage 100; radius 120; gravity 800; }",
            "weaponinfo { number 1; name gauntlet; projectile gauntlet_hit; numprojectiles 1; speed 0; reload 0.4; ammoamount 0; }",
            "weaponinfo { number 5; name rocketlauncher; projectile rocket; numprojectiles 1; speed 900; reload 0.8; ammoamount 1; }",
            "weaponinfo { number 7; name railgun; projectile none; numprojectiles 1; speed 0; reload 1.5; ammoamount 1; }",
        ]
        .join("\n")
    }

    #[test]
    fn missing_weapons_are_unavailable() {
        let files = FakeFiles { weapons: None };
        assert!(matches!(
            native_q3_weapon_knowledge(&files),
            Err(BotKnowledgeError::Unavailable)
        ));
    }

    #[test]
    fn parses_selected_weapons_once() {
        let files = FakeFiles {
            weapons: Some(weapons_c()),
        };
        let result = native_q3_weapon_knowledge(&files).expect("knowledge");
        // Gauntlet and rocket launcher qualify; the railgun has no damage.
        assert_eq!(result.len(), 2);
        let gauntlet = &result[0];
        assert_eq!(gauntlet.entry.name, "q3:weapon/gauntlet");
        assert_eq!(gauntlet.entry.number, 2);
        assert_eq!(gauntlet.entry.damage, 50.0);
        assert_eq!(gauntlet.entry.max_range, 60.0);
        assert_eq!(gauntlet.entry.flags, vec!["melee".to_string()]);
        assert_eq!(gauntlet.entry.aim_point, "center");
        assert!(gauntlet.is_melee);
        assert!(!gauntlet.needs_ammo);
        let rocket = &result[1];
        assert_eq!(rocket.entry.name, "q3:weapon/rocketlauncher");
        assert_eq!(rocket.entry.damage, 100.0);
        assert_eq!(rocket.entry.max_range, 4096.0);
        assert_eq!(rocket.entry.min_range, 120.0);
        assert_eq!(
            rocket.entry.flags,
            vec![
                "projectile".to_string(),
                "explosive".to_string(),
                "parabolic".to_string()
            ]
        );
        assert_eq!(rocket.entry.aim_point, "feet");
        assert!(rocket.needs_ammo);
        assert!(!rocket.is_melee);
        assert!(!rocket.is_electric);
    }
}
