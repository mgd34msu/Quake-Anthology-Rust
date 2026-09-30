//! Mission-pack messages (src/content/q1/missionpacks/messages.ts).

use qa_core::identity::ActorId;

use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::types::{Q1Edition, Q1MessageArg};

/// Classic mission-pack strings (`classic`).
const CLASSIC: &[(&str, &str)] = &[
    ("$qc_wetsuit", "Wetsuit"),
    ("$qc_empathy_shields", "Empathy Shields"),
    ("$qc_mjolnir", "Mjolnir"),
    ("$qc_laser_cannon", "Laser Cannon"),
    ("$qc_prox_gun", "Proximity Gun"),
    ("$qc_power_shield", "Power Shield"),
    ("$qc_anti_grav_belt", "Anti-Grav Belt"),
    ("$qc_vengeance_sphere", "Vengeance Sphere"),
    ("$qc_lava_nails", "lava nails"),
    ("$qc_multi_rockets", "multi rockets"),
    ("$qc_quad_damage", "Quad Damage"),
    ("$qc_pentagram_of_protection", "Pentagram of Protection"),
    ("$qc_ring_of_shadows", "Ring of Shadows"),
    ("$qc_wetsuit_fade", "Air supply in Wetsuit is running out\n"),
    ("$qc_empathy_fade", "Empathy Shields are running out\n"),
    ("$qc_shield_failing", "Shield failing...\n"),
    ("$qc_shield_lost", "Shield Lost.\n"),
    ("$qc_antigrav_failing", "Antigrav failing...\n"),
    ("$qc_antigrav_lost", "Antigrav Lost.\n"),
    ("$qc_vengeance_lost", "Vengeance Sphere Lost\n"),
    ("$qc_you_are_denied_vengeance", "You are denied Vengeance"),
    ("$qc_lava_enabled", "Lava Enabled\n"),
    ("$qc_super_lava_enabled", "Super Lava Enabled\n"),
    ("$qc_multi_gl_enabled", "Multi Grenades Enabled\n"),
    ("$qc_multi_rl_enabled", "Multi Rockets Enabled\n"),
    ("$qc_plasma_enabled", "Plasma Gun Enabled\n"),
    ("$qc_no_weapon", "no weapon.\n"),
    ("$qc_not_enough_ammo", "not enough ammo.\n"),
    ("$qc_got_horn", "You got the Horn of Conjuring\n"),
    ("$qc_normal_nails", "Normal Nails\n"),
    ("$qc_normal_grenades", "Normal Grenades\n"),
    ("$qc_normal_rockets", "Normal Rockets\n"),
    ("$qc_lightning_gun", "Lightning Gun\n"),
    ("$qc_multi_gl", "Multi Grenades\n"),
    ("$qc_multi_rl", "Multi Rockets\n"),
    ("$qc_plasma_gun", "Plasma Gun\n"),
    ("$qc_no_ammo_available", "No ammo available!\n"),
    ("$qc_quad_cheat", "quad cheat\n"),
    ("$qc_wetsuit_cheat", "wetsuit cheat\n"),
    ("$qc_empathy_cheat", "empathy shields cheat\n"),
    ("$qc_genocide_cheat", "Genocide!\n"),
    ("$qc_dump_player_loc", "Dumping Player Location\n"),
    ("$qc_double_shotgun", "Double-barrelled Shotgun"),
    ("$qc_nailgun", "Nailgun"),
    ("$qc_super_nailgun", "Super Nailgun"),
    ("$qc_grenade_launcher", "Grenade Launcher"),
    ("$qc_rocket_launcher", "Rocket Launcher"),
    ("$qc_thunderbolt", "Thunderbolt"),
];

/// Look up a classic string, passing unknown keys through.
fn classic(key: &str) -> &str {
    CLASSIC
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, text)| *text)
        .unwrap_or(key)
}

/// Send a mission-pack message (`missionMessage`).
pub fn mission_message(game: &mut Q1EntityServices, player: Option<&ActorId>, key: &str) {
    let text = if game.options().edition == Q1Edition::Classic {
        classic(key).to_string()
    } else {
        key.to_string()
    };
    game.message(player, &text, false, Vec::new());
}

/// Send a mission-pack pickup message (`missionPickupMessage`).
pub fn mission_pickup_message(game: &mut Q1EntityServices, player: &ActorId, key: &str) {
    if game.options().edition == Q1Edition::Classic {
        game.message(
            player,
            &format!("You got the {}\n", classic(key)),
            false,
            Vec::new(),
        );
    } else {
        game.message(
            Some(player),
            "$qc_got_item",
            false,
            vec![Q1MessageArg::Text(key.to_string())],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;

    #[test]
    fn classic_strings_match_donor() {
        assert_eq!(classic("$qc_mjolnir"), "Mjolnir");
        assert_eq!(classic("$qc_no_weapon"), "no weapon.\n");
        assert_eq!(classic("$qc_missing"), "$qc_missing");
    }

    #[test]
    fn messages_send_without_players() {
        let mut game = test_game();
        mission_message(&mut game, None, "$qc_genocide_cheat");
        let player = game.create("player", None, None).expect("player");
        mission_pickup_message(&mut game, &player, "$qc_thunderbolt");
    }
}
