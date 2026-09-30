//! Q1 weapon display names
//! (`src/content/q1/foundation/weapon-names.ts`).

use super::types::{is_q1_base_weapon, Q1BaseWeapon, Q1Weapon};

/// Display name for a base weapon.
fn base_display_name(weapon: Q1BaseWeapon) -> &'static str {
    match weapon {
        Q1BaseWeapon::Axe => "Axe",
        Q1BaseWeapon::Shotgun => "Shotgun",
        Q1BaseWeapon::Supershotgun => "Double-barrelled Shotgun",
        Q1BaseWeapon::Nailgun => "Nailgun",
        Q1BaseWeapon::Supernailgun => "Super Nailgun",
        Q1BaseWeapon::Grenadelauncher => "Grenade Launcher",
        Q1BaseWeapon::Rocketlauncher => "Rocket Launcher",
        Q1BaseWeapon::Lightning => "Thunderbolt",
    }
}

/// Title-case a donor weapon suffix (`lace_like-this` to
/// `Lace Like This`, donor `replaceAll` plus word-boundary upper
/// casing).
fn title_case(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut boundary = true;
    for char in text.chars() {
        if char == '_' || char == '-' || char == ' ' {
            output.push(' ');
            boundary = true;
        } else if boundary && char.is_ascii_alphanumeric() {
            output.extend(char.to_uppercase());
            boundary = false;
        } else {
            output.push(char);
            boundary = !char.is_ascii_alphanumeric() && char != '_';
        }
    }
    output
}

/// Display name for a Q1 weapon (`q1WeaponDisplayName`).
#[must_use]
pub fn q1_weapon_display_name(weapon: Q1Weapon) -> String {
    if is_q1_base_weapon(weapon) {
        let base = super::types::WEAPONS
            .iter()
            .find(|candidate| Q1Weapon::from(**candidate) == weapon)
            .copied()
            .expect("base weapon roster is complete");
        return base_display_name(base).to_string();
    }
    match weapon {
        Q1Weapon::HipnoticLaser => return String::from("Laser Cannon"),
        Q1Weapon::HipnoticMjolnir => return String::from("Mjolnir"),
        Q1Weapon::HipnoticProximity => return String::from("Proximity Gun"),
        _ => {}
    }
    let name = weapon.as_str().rsplit(':').next().unwrap_or(weapon.as_str());
    title_case(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_match_donor() {
        assert_eq!(
            q1_weapon_display_name(Q1Weapon::Supershotgun),
            "Double-barrelled Shotgun"
        );
        assert_eq!(q1_weapon_display_name(Q1Weapon::HipnoticLaser), "Laser Cannon");
        assert_eq!(q1_weapon_display_name(Q1Weapon::RogueLavaNailgun), "Lava Nailgun");
        assert_eq!(q1_weapon_display_name(Q1Weapon::CtfGrapple), "Grapple");
    }
}
