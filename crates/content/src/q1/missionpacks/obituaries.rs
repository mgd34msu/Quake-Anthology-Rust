//! Mission-pack obituaries (src/content/q1/missionpacks/obituaries.ts).

use qa_core::identity::{ActorId, same_actor};

use crate::q1::base::messages::{classic_monster_obituary, classic_obituary_text};
use crate::q1::base::rules::{
    Q1DeathWater, Q1Obituary, Q1ObituaryInput, Q1ObituaryMessage, Q1ObituaryScore, q1_obituary,
};
use crate::q1::foundation::types::{Q1Edition, Q1Weapon};

use super::types::Q1MissionPack;

/// Mission-pack obituary input (`Q1MissionPackObituaryInput`).
pub type Q1MissionPackObituaryInput = Q1ObituaryInput;

/// Mission-pack obituary context (`MissionPackObituaryContext`).
pub struct MissionPackObituaryContext {
    /// Mission pack.
    pub pack: Q1MissionPack,
    /// Inflictor classname.
    pub inflictor_classname: String,
    /// Attacker death type.
    pub attacker_death_type: String,
    /// Victim team saved at spawn.
    pub victim_saved_team: i32,
    /// Session game config flags.
    pub gamecfg: i32,
    /// Rogue tag score probe.
    pub tag_score: Option<Box<dyn Fn() -> i32>>,
}

impl std::fmt::Debug for MissionPackObituaryContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MissionPackObituaryContext")
            .field("pack", &self.pack)
            .field("inflictor_classname", &self.inflictor_classname)
            .field("attacker_death_type", &self.attacker_death_type)
            .field("victim_saved_team", &self.victim_saved_team)
            .field("gamecfg", &self.gamecfg)
            .finish_non_exhaustive()
    }
}

/// Classic mission-pack death strings (`classic`).
const CLASSIC: &[(&str, &str)] = &[
    ("$qc_death_empathy1", "{0} shares {1}'s pain\n"),
    ("$qc_death_empathy2", "{0} feels {1}'s pain\n"),
    ("$qc_death_bomb1", "{0} got too friendly with {1}'s bomb\n"),
    ("$qc_death_bomb2", "{0} did the rhumba with {1}'s bomb\n"),
    ("$qc_death_laser1", "{0} was toasted by {1}'s laser\n"),
    ("$qc_death_laser2", "{0} was radiated by {1}'s laser\n"),
    ("$qc_death_hammer", "{0} was slammed by {1}'s hammer\n"),
    ("$qc_death_grappled", "{0} was grappled by {1}\n"),
    ("$qc_death_burned", "{0} was burned by {1}\n"),
    ("$qc_death_fused", "{0} was fused by {1}\n"),
    ("$qc_death_blasted", "{0} was blasted to bits by {1}\n"),
    (
        "$qc_death_vengeance",
        "{0} was purged by the Vengeance Sphere\n",
    ),
    ("$qc_death_smashed", "{0} was smashed by {1}\n"),
    ("$qc_changed_teams", "{0} changed teams\n"),
    ("$qc_tried_change_teams", "{0} tried to change teams\n"),
    ("$qc_suicide_loaded", "{0} checks if his weapon is loaded\n"),
    ("$qc_ks_dragon1", "{0} was annihilated by the Dragon\n"),
    ("$qc_ks_dragon2", "{0} was squashed by the Dragon\n"),
    ("$qc_ks_eel", "{0} was electrified by an Eel\n"),
    ("$qc_ks_wrath", "{0} was disintegrated by a Wrath\n"),
    ("$qc_ks_overlord", "{0} was obliterated by an Overlord\n"),
    (
        "$qc_ks_swordsman",
        "{0} was slit open by a Phantom Swordsman\n",
    ),
    ("$qc_ks_hephaestus", "{0} fries in Hephaestus' fury\n"),
    ("$qc_ks_guardian", "{0} was crushed by a Guardian\n"),
    ("$qc_ks_mummy", "{0} was Mummified\n"),
    ("$qc_ks_gremlin", "{0} was outsmarted by a Gremlin\n"),
    ("$qc_ks_centroid", "{0} was stung by a Centroid\n"),
    ("$qc_ks_armagon", "{0} was outgunned by Armagon\n"),
    ("$qc_ks_blew_up", "{0} blew up\n"),
    ("$qc_ks_spiked", "{0} was spiked\n"),
    ("$qc_ks_lavaball", "{0} ate a lavaball\n"),
    ("$qc_ks_tried_leave", "{0} tried to leave\n"),
    ("$qc_ks_rode_lightning", "{0} rode the lightning\n"),
    ("$qc_ks_cleaved", "{0} was cleaved in two\n"),
    ("$qc_ks_sliced", "{0} was sliced to pieces\n"),
    ("$qc_ks_plasma", "{0} was turned to plasma\n"),
];

/// Monster kill strings (`monsters`).
const MONSTERS: &[(&str, &str)] = &[
    ("monster_army", "$qc_ks_grunt"),
    ("monster_demon1", "$qc_ks_fiend"),
    ("monster_dog", "$qc_ks_rottweiler"),
    ("monster_dragon", "$qc_ks_dragon"),
    ("monster_dragon_dead", "$qc_ks_dragon2"),
    ("monster_enforcer", "$qc_ks_enforcer"),
    ("monster_fish", "$qc_ks_rotfish"),
    ("monster_hell_knight", "$qc_ks_deathknight"),
    ("monster_knight", "$qc_ks_knight"),
    ("monster_ogre", "$qc_ks_ogre"),
    ("monster_oldone", "$qc_ks_shub"),
    ("monster_shalrath", "$qc_ks_vore"),
    ("monster_shambler", "$qc_ks_shambler"),
    ("monster_tarbaby", "$qc_ks_spawn"),
    ("monster_vomit", "$qc_ks_vomitus"),
    ("monster_wizard", "$qc_ks_scrag"),
    ("monster_zombie", "$qc_ks_zombie"),
    ("monster_gremlin", "$qc_ks_gremlin"),
    ("monster_scourge", "$qc_ks_centroid"),
    ("monster_armagon", "$qc_ks_armagon"),
    ("monster_eel", "$qc_ks_eel"),
    ("monster_wrath", "$qc_ks_wrath"),
    ("monster_super_wrath", "$qc_ks_overlord"),
    ("monster_sword", "$qc_ks_swordsman"),
    ("monster_lava_man", "$qc_ks_hephaestus"),
    ("monster_morph", "$qc_ks_guardian"),
    ("monster_mummy", "$qc_ks_mummy"),
];

/// Substitute `{N}` arguments into a format.
fn substitute(format: &str, args: &[&str]) -> String {
    let mut output = String::new();
    let mut rest = format;
    while let Some(open) = rest.find('{') {
        output.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let index_text = &after[..close];
                match index_text.parse::<usize>() {
                    Ok(index) => output.push_str(args.get(index).copied().unwrap_or("")),
                    Err(_) => {
                        output.push('{');
                        output.push_str(index_text);
                        output.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                output.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    output.push_str(rest);
    output
}

/// Classic death text with base fallback (`classicText`).
fn classic_text(key: &str, args: &[String]) -> String {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match CLASSIC.iter().find(|(candidate, _)| *candidate == key) {
        Some((_, template)) => substitute(template, &refs),
        None => classic_obituary_text(key, &refs),
    }
}

/// Random draws with first-roll replay for base fallback (`fallback`).
struct Replay<'a> {
    first: f64,
    replay: bool,
    random: &'a mut dyn FnMut() -> f64,
}

impl Replay<'_> {
    fn draw(&mut self) -> f64 {
        if self.replay {
            self.replay = false;
            self.first
        } else {
            (self.random)()
        }
    }

    fn fresh(&mut self) -> f64 {
        (self.random)()
    }
}

/// Build a mission-pack obituary (`result`).
fn result(
    input: &Q1MissionPackObituaryInput,
    key: &str,
    actor: Option<&ActorId>,
    delta: i32,
    args: &[String],
    classic_override: Option<String>,
) -> Q1Obituary {
    Q1Obituary {
        message: if key.is_empty() {
            None
        } else if input.edition == Q1Edition::Rerelease {
            Some(Q1ObituaryMessage {
                text: key.to_string(),
                arguments: args.to_vec(),
            })
        } else {
            Some(Q1ObituaryMessage {
                text: classic_override.unwrap_or_else(|| classic_text(key, args)),
                arguments: Vec::new(),
            })
        },
        score: actor.map(|actor| Q1ObituaryScore {
            actor: actor.clone(),
            delta,
        }),
        achievement: None,
    }
}

/// Base fallback with first-roll replay (`fallback`).
fn fallback(
    input: &Q1MissionPackObituaryInput,
    replay: &mut Replay<'_>,
    clear_attacker: bool,
) -> Q1Obituary {
    if clear_attacker {
        let mut owned = input.clone();
        owned.attacker = None;
        q1_obituary(&owned, &mut || replay.draw())
    } else {
        q1_obituary(input, &mut || replay.draw())
    }
}

/// Mission-pack obituary (`missionPackObituary`).
pub fn mission_pack_obituary(
    input: &Q1MissionPackObituaryInput,
    context: &MissionPackObituaryContext,
    random: &mut dyn FnMut() -> f64,
) -> Q1Obituary {
    let first_roll = random();
    // Borrowck: route every later draw through one replaying owner.
    let mut replay = Replay {
        first: first_roll,
        replay: true,
        random,
    };
    let victim_args = vec![input.victim.name.clone()];
    let attacker_classname = input
        .attacker
        .as_ref()
        .map(|attacker| attacker.classname.as_str());
    if !input.victim.is_player || matches!(attacker_classname, Some("teledeath" | "teledeath2")) {
        return fallback(input, &mut replay, false);
    }
    if input
        .attacker
        .as_ref()
        .is_some_and(|attacker| attacker.is_player)
    {
        let attacker = input.attacker.clone().expect("attacker");
        if same_actor(&input.victim.actor, &attacker.actor) {
            if input.victim.weapon == Some(Q1Weapon::Lightning) && input.victim.water_level > 1
                || input.victim.weapon == Some(Q1Weapon::Grenadelauncher)
            {
                return fallback(input, &mut replay, false);
            }
            if context.pack == Q1MissionPack::Hipnotic {
                let key = if input.edition == Q1Edition::Classic {
                    if first_roll > 0.4 {
                        "$qc_suicide_bored"
                    } else {
                        "$qc_suicide_loaded"
                    }
                } else if first_roll != 0.0 {
                    "$qc_suicide_bored"
                } else {
                    "$qc_suicide_loaded"
                };
                return result(
                    input,
                    key,
                    Some(&input.victim.actor),
                    -1,
                    &victim_args,
                    None,
                );
            }
            if input.edition == Q1Edition::Rerelease && first_roll < 0.5 {
                return result(
                    input,
                    "$qc_suicide_bored",
                    Some(&input.victim.actor),
                    -1,
                    &victim_args,
                    None,
                );
            }
            if input.teamplay != 0 && input.victim.team != context.victim_saved_team {
                let key = if context.gamecfg & 16 != 0 {
                    "$qc_changed_teams"
                } else {
                    "$qc_tried_change_teams"
                };
                return result(
                    input,
                    key,
                    Some(&input.victim.actor),
                    -1,
                    &victim_args,
                    None,
                );
            }
            let key = if input.edition == Q1Edition::Classic {
                "$qc_suicide_bored"
            } else {
                "$qc_suicide_loaded"
            };
            return result(
                input,
                key,
                Some(&input.victim.actor),
                -1,
                &victim_args,
                None,
            );
        }
        if input.teamplay == 2 && input.victim.team > 0 && input.victim.team == attacker.team {
            return fallback(input, &mut replay, false);
        }
        let points = if context.pack == Q1MissionPack::Rogue && input.teamplay == 3 {
            context.tag_score.as_ref().map(|score| score()).unwrap_or(1)
        } else {
            1
        };
        let args = vec![input.victim.name.clone(), attacker.name.clone()];
        if context.pack == Q1MissionPack::Hipnotic {
            if input.death_type == "hipnotic:empathy" {
                let key = if replay.fresh() < 0.5 {
                    "$qc_death_empathy1"
                } else {
                    "$qc_death_empathy2"
                };
                return result(input, key, Some(&attacker.actor), points, &args, None);
            }
            if context.inflictor_classname == "proximity_grenade" {
                let key = if replay.fresh() < 0.5 {
                    "$qc_death_bomb1"
                } else {
                    "$qc_death_bomb2"
                };
                return result(input, key, Some(&attacker.actor), points, &args, None);
            }
            if attacker.weapon == Some(Q1Weapon::HipnoticLaser) {
                let key = if replay.fresh() < 0.5 {
                    "$qc_death_laser1"
                } else {
                    "$qc_death_laser2"
                };
                return result(input, key, Some(&attacker.actor), points, &args, None);
            }
            if attacker.weapon == Some(Q1Weapon::HipnoticMjolnir) {
                return result(
                    input,
                    "$qc_death_hammer",
                    Some(&attacker.actor),
                    points,
                    &args,
                    None,
                );
            }
        } else {
            if attacker.weapon == Some(Q1Weapon::RogueGrapple) {
                return result(
                    input,
                    "$qc_death_grappled",
                    Some(&attacker.actor),
                    points,
                    &args,
                    None,
                );
            }
            if matches!(
                attacker.weapon,
                Some(Q1Weapon::RogueLavaNailgun | Q1Weapon::RogueLavaSupernailgun)
            ) {
                return result(
                    input,
                    "$qc_death_burned",
                    Some(&attacker.actor),
                    points,
                    &args,
                    None,
                );
            }
            if attacker.weapon == Some(Q1Weapon::RoguePlasma) {
                return result(
                    input,
                    "$qc_death_fused",
                    Some(&attacker.actor),
                    points,
                    &args,
                    None,
                );
            }
            if matches!(
                attacker.weapon,
                Some(Q1Weapon::RogueMultiGrenade | Q1Weapon::RogueMultiRocket)
            ) {
                return result(
                    input,
                    "$qc_death_blasted",
                    Some(&attacker.actor),
                    points,
                    &args,
                    None,
                );
            }
        }
        let base = fallback(input, &mut replay, false);
        return Q1Obituary {
            score: Some(Q1ObituaryScore {
                actor: attacker.actor.clone(),
                delta: points,
            }),
            ..base
        };
    }
    if context.pack == Q1MissionPack::Hipnotic {
        if !context.attacker_death_type.is_empty() {
            let classic = format!("{} {}\n", input.victim.name, context.attacker_death_type);
            return result(
                input,
                &context.attacker_death_type.clone(),
                Some(&input.victim.actor),
                -1,
                &victim_args,
                Some(classic),
            );
        }
        if input.victim.water_type != Q1DeathWater::Empty {
            return fallback(input, &mut replay, true);
        }
    }
    if input
        .attacker
        .as_ref()
        .is_some_and(|attacker| attacker.is_monster)
    {
        let attacker = input.attacker.clone().expect("attacker");
        let key = if context.pack == Q1MissionPack::Rogue && attacker.classname == "monster_dragon"
        {
            "$qc_ks_dragon1"
        } else {
            MONSTERS
                .iter()
                .find(|(candidate, _)| *candidate == attacker.classname)
                .map(|(_, key)| *key)
                .unwrap_or("")
        };
        let old = classic_monster_obituary(&attacker.classname);
        let classic = if CLASSIC.iter().any(|(candidate, _)| *candidate == key) || old.is_none() {
            None
        } else {
            old.map(|suffix| format!("{}{suffix}", input.victim.name))
        };
        return result(
            input,
            key,
            Some(&input.victim.actor),
            -1,
            &victim_args,
            classic,
        );
    }
    if attacker_classname == Some("explo_box") {
        return result(
            input,
            "$qc_ks_blew_up",
            Some(&input.victim.actor),
            -1,
            &victim_args,
            None,
        );
    }
    if input
        .attacker
        .as_ref()
        .is_some_and(|attacker| attacker.brush)
        && attacker_classname != Some("worldspawn")
    {
        return result(
            input,
            "$qc_death_squish",
            Some(&input.victim.actor),
            -1,
            &victim_args,
            None,
        );
    }
    if context.pack == Q1MissionPack::Hipnotic && input.death_type == "falling" {
        return result(
            input,
            "$qc_death_fall",
            Some(&input.victim.actor),
            -1,
            &victim_args,
            None,
        );
    }
    let trap = attacker_classname;
    if trap == Some("trap_shooter") || trap == Some("trap_spikeshooter") {
        return result(
            input,
            "$qc_ks_spiked",
            Some(&input.victim.actor),
            -1,
            &victim_args,
            None,
        );
    }
    if trap == Some("fireball") {
        return result(
            input,
            "$qc_ks_lavaball",
            Some(&input.victim.actor),
            -1,
            &victim_args,
            None,
        );
    }
    if trap == Some("trigger_changelevel") {
        return result(
            input,
            "$qc_ks_tried_leave",
            Some(&input.victim.actor),
            -1,
            &victim_args,
            None,
        );
    }
    if context.pack == Q1MissionPack::Rogue {
        if trap == Some("ltrail_start") || trap == Some("ltrail_relay") {
            return result(
                input,
                "$qc_ks_rode_lightning",
                Some(&input.victim.actor),
                -1,
                &victim_args,
                None,
            );
        }
        if trap == Some("pendulum") || trap == Some("buzzsaw") || trap == Some("plasma") {
            let key = if trap == Some("pendulum") {
                "$qc_ks_cleaved"
            } else if trap == Some("buzzsaw") {
                "$qc_ks_sliced"
            } else {
                "$qc_ks_plasma"
            };
            return result(
                input,
                key,
                Some(&input.victim.actor),
                -1,
                &victim_args,
                None,
            );
        }
        if trap == Some("Vengeance") {
            return result(input, "$qc_death_vengeance", None, 0, &victim_args, None);
        }
        if trap == Some("power_shield") && input.telefrag_owner.is_some() {
            let owner = input.telefrag_owner.clone().expect("owner");
            let args = vec![input.victim.name.clone(), owner.name.clone()];
            return result(
                input,
                "$qc_death_smashed",
                Some(&owner.actor),
                1,
                &args,
                None,
            );
        }
    }
    fallback(input, &mut replay, false)
}

#[cfg(test)]
mod tests {
    use super::super::types::test_game;
    use super::*;
    use crate::q1::base::rules::Q1ObituaryActor;

    fn actor(
        game: &mut crate::q1::foundation::entity_services::Q1EntityServices,
        name: &str,
    ) -> ActorId {
        game.create("player", None, None).expect(name.to_string())
    }

    fn participant(actor: ActorId, name: &str) -> Q1ObituaryActor {
        Q1ObituaryActor {
            actor,
            name: name.to_string(),
            classname: "player".to_string(),
            is_player: true,
            is_monster: false,
            team: 0,
            health: 100.0,
            water_type: Q1DeathWater::Empty,
            water_level: 0,
            weapon: None,
            quad_expires: 0.0,
            invulnerable_expires: 0.0,
            brush: false,
            kill_string: String::new(),
        }
    }

    fn context(pack: Q1MissionPack) -> MissionPackObituaryContext {
        MissionPackObituaryContext {
            pack,
            inflictor_classname: String::new(),
            attacker_death_type: String::new(),
            victim_saved_team: 0,
            gamecfg: 0,
            tag_score: None,
        }
    }

    #[test]
    fn hipnotic_suicide_matches_donor_roll() {
        let mut game = test_game();
        let victim = actor(&mut game, "victim");
        let participant = participant(victim, "Vic");
        let input = Q1MissionPackObituaryInput {
            edition: Q1Edition::Classic,
            victim: participant.clone(),
            attacker: Some(participant),
            telefrag_owner: None,
            teamplay: 0,
            death_type: String::new(),
        };
        let mut random = || 0.9;
        let obituary =
            mission_pack_obituary(&input, &context(Q1MissionPack::Hipnotic), &mut random);
        assert_eq!(
            obituary.message.expect("message").text,
            "Vic becomes bored with life\n"
        );
        assert_eq!(obituary.score.expect("score").delta, -1);
    }

    #[test]
    fn rogue_lava_kill_scores_attacker() {
        let mut game = test_game();
        let victim = participant(actor(&mut game, "victim"), "Vic");
        let mut attacker = participant(actor(&mut game, "attacker"), "Att");
        attacker.weapon = Some(Q1Weapon::RogueLavaNailgun);
        let input = Q1MissionPackObituaryInput {
            edition: Q1Edition::Rerelease,
            victim,
            attacker: Some(attacker),
            telefrag_owner: None,
            teamplay: 0,
            death_type: String::new(),
        };
        let mut random = || 0.1;
        let obituary = mission_pack_obituary(&input, &context(Q1MissionPack::Rogue), &mut random);
        let message = obituary.message.expect("message");
        assert_eq!(message.text, "$qc_death_burned");
        assert_eq!(
            message.arguments,
            vec!["Vic".to_string(), "Att".to_string()]
        );
        assert_eq!(obituary.score.expect("score").delta, 1);
    }

    #[test]
    fn vengeance_trap_scores_nobody() {
        let mut game = test_game();
        let victim = participant(actor(&mut game, "victim"), "Vic");
        let mut attacker = participant(actor(&mut game, "sphere"), "Sphere");
        attacker.is_player = false;
        attacker.classname = "Vengeance".to_string();
        let input = Q1MissionPackObituaryInput {
            edition: Q1Edition::Classic,
            victim,
            attacker: Some(attacker),
            telefrag_owner: None,
            teamplay: 0,
            death_type: String::new(),
        };
        let mut random = || 0.5;
        let obituary = mission_pack_obituary(&input, &context(Q1MissionPack::Rogue), &mut random);
        assert_eq!(
            obituary.message.expect("message").text,
            "Vic was purged by the Vengeance Sphere\n"
        );
        assert!(obituary.score.is_none());
    }
}
