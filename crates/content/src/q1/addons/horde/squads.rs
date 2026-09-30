//! Q1 horde squads (`src/content/q1/addons/horde/squads.ts`).
//!
//! `quakec_mg1/horde.qc` `SpawnSquad2` and `SpawnWave2`.
//! GPL-2.0-or-later.

use qa_core::math::Vec3;

/// Horde monster (`HordeMonster`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HordeMonster {
    /// Knight.
    Knight,
    /// Hell knight.
    Hellknight,
    /// Dog.
    Dog,
    /// Demon.
    Demon,
    /// Ogre.
    Ogre,
    /// Grunt.
    Grunt,
    /// Enforcer.
    Enforcer,
    /// Shambler.
    Shambler,
    /// Shalrath.
    Shalrath,
    /// Wizard.
    Wizard,
    /// Zombie.
    Zombie,
}

/// Horde squad (`HordeSquad`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HordeSquad {
    /// Three grunts.
    ThreeGrunts,
    /// Two grunts and a dog.
    TwoGruntsOneDog,
    /// Two dogs.
    TwoDogs,
    /// One enforcer.
    OneEnforcer,
    /// Two enforcers.
    TwoEnforcers,
    /// One ogre.
    OneOgre,
    /// Two knights.
    TwoKnights,
    /// Two zombies.
    TwoZombies,
    /// One wizard.
    OneWizard,
    /// Two hell knights.
    TwoHellknights,
    /// Two knights and a hell knight.
    TwoKnightsOneHellknight,
    /// Three wizards.
    ThreeWizards,
    /// Shambler.
    Shambler,
    /// Double demon.
    DoubleDemon,
    /// Shalrath.
    Shalrath,
}

/// Horde squad type (`HordeSquadType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HordeSquadType {
    /// Normal.
    Normal,
    /// Ranged.
    Ranged,
    /// Flying.
    Flying,
    /// Boss.
    Boss,
}

/// Horde squad spawn entry (`HordeSquadSpawn`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HordeSquadSpawn {
    /// Monster kind.
    pub monster: HordeMonster,
    /// Spawn offset.
    pub offset: Vec3,
}

fn spawn(monster: HordeMonster, x: f32, y: f32, z: f32) -> HordeSquadSpawn {
    HordeSquadSpawn {
        monster,
        offset: Vec3 { x, y, z },
    }
}

/// Resolves squad spawns (`hordeSquad`).
pub fn horde_squad(name: HordeSquad, skill: i32, random: &mut dyn FnMut() -> f64) -> Vec<HordeSquadSpawn> {
    use HordeMonster as M;
    use HordeSquad as S;
    match name {
        S::ThreeGrunts if skill > 0 => vec![
            spawn(M::Grunt, 0.0, -40.0, 0.0),
            spawn(M::Grunt, 40.0, 40.0, 0.0),
            spawn(M::Grunt, -40.0, 40.0, 0.0),
        ],
        S::ThreeGrunts => vec![spawn(M::Grunt, -40.0, 0.0, 0.0), spawn(M::Grunt, 40.0, 0.0, 0.0)],
        S::TwoGruntsOneDog => vec![
            spawn(M::Dog, 44.0, 0.0, 0.0),
            spawn(M::Grunt, -40.0, -40.0, 0.0),
            spawn(M::Grunt, -40.0, 40.0, 0.0),
        ],
        S::TwoDogs if skill > 0 => vec![spawn(M::Dog, 0.0, -44.0, 0.0), spawn(M::Dog, 0.0, 44.0, 0.0)],
        S::TwoDogs => vec![spawn(M::Dog, 0.0, -44.0, 0.0)],
        S::OneEnforcer => vec![spawn(M::Enforcer, 0.0, 0.0, 0.0)],
        S::TwoEnforcers if skill > 0 => vec![spawn(M::Enforcer, 40.0, 0.0, 0.0), spawn(M::Enforcer, -40.0, 0.0, 0.0)],
        S::TwoEnforcers => vec![spawn(M::Enforcer, 0.0, 0.0, 0.0)],
        S::OneOgre => vec![spawn(M::Ogre, 0.0, 0.0, 0.0)],
        S::TwoKnights if skill > 0 => vec![spawn(M::Knight, 40.0, 0.0, 0.0), spawn(M::Knight, -40.0, 0.0, 0.0)],
        S::TwoKnights => vec![spawn(M::Knight, 40.0, 0.0, 0.0)],
        S::TwoZombies => vec![spawn(M::Zombie, 40.0, 0.0, 0.0), spawn(M::Zombie, -40.0, 0.0, 0.0)],
        S::OneWizard => vec![spawn(M::Wizard, 0.0, 0.0, 0.0)],
        S::TwoHellknights if skill > 0 => vec![
            spawn(M::Hellknight, 0.0, 40.0, 0.0),
            spawn(M::Hellknight, 0.0, -40.0, 0.0),
        ],
        S::TwoHellknights => vec![spawn(M::Hellknight, 0.0, 40.0, 0.0)],
        S::TwoKnightsOneHellknight => vec![
            spawn(M::Hellknight, 40.0, 0.0, 0.0),
            spawn(M::Knight, -40.0, 40.0, 0.0),
            spawn(M::Knight, -40.0, -40.0, 0.0),
        ],
        S::ThreeWizards if skill > 0 => vec![
            spawn(M::Wizard, 40.0, 40.0, 40.0),
            spawn(M::Wizard, -40.0, 40.0, 40.0),
            spawn(M::Wizard, -40.0, -40.0, 40.0),
        ],
        S::ThreeWizards => vec![spawn(M::Wizard, 40.0, 40.0, 40.0), spawn(M::Wizard, -40.0, 40.0, 40.0)],
        S::Shambler => vec![spawn(M::Shambler, 0.0, 0.0, 0.0)],
        S::DoubleDemon if skill >= 3 && random() > 0.8 => vec![
            spawn(M::Shambler, 40.0, 40.0, 0.0),
            spawn(M::Shambler, -40.0, -40.0, 0.0),
        ],
        S::DoubleDemon if skill >= 1 => vec![spawn(M::Demon, 40.0, 40.0, 0.0), spawn(M::Demon, -40.0, -40.0, 0.0)],
        S::DoubleDemon => vec![spawn(M::Demon, 0.0, 0.0, 0.0)],
        S::Shalrath => vec![spawn(M::Shalrath, 0.0, 0.0, 0.0)],
    }
}

/// Horde squad category for selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HordeCategory {
    /// Fodder.
    Fodder,
    /// Elites.
    Elites,
    /// Bosses.
    Bosses,
}

/// Chooses a horde squad (`chooseHordeSquad`).
pub fn choose_horde_squad(
    army: bool,
    category: HordeCategory,
    random: &mut dyn FnMut() -> f64,
) -> Option<(HordeSquad, HordeSquadType)> {
    use HordeCategory as C;
    use HordeSquad as S;
    use HordeSquadType as T;
    if army {
        if category == C::Bosses {
            return None;
        }
        if category == C::Elites {
            return Some((
                if random() * 2.0 < 1.5 {
                    S::TwoEnforcers
                } else {
                    S::OneOgre
                },
                T::Ranged,
            ));
        }
        let roll = random() * 4.0;
        return Some((
            if roll < 1.0 {
                S::ThreeGrunts
            } else if roll < 2.0 {
                S::TwoGruntsOneDog
            } else if roll < 3.5 {
                S::TwoDogs
            } else {
                S::OneEnforcer
            },
            if roll < 3.5 { T::Normal } else { T::Ranged },
        ));
    }
    if category == C::Fodder {
        let roll = random() * 4.0;
        return Some((
            if roll < 2.0 {
                S::TwoKnights
            } else if roll < 3.0 {
                S::TwoZombies
            } else {
                S::OneWizard
            },
            if roll < 3.0 { T::Normal } else { T::Flying },
        ));
    }
    if category == C::Elites {
        let roll = random() * 4.0;
        return Some((
            if roll < 1.0 {
                S::TwoHellknights
            } else if roll < 2.0 {
                S::TwoKnightsOneHellknight
            } else if roll < 3.0 {
                S::OneOgre
            } else {
                S::ThreeWizards
            },
            if roll < 2.0 {
                T::Normal
            } else if roll < 3.0 {
                T::Ranged
            } else {
                T::Flying
            },
        ));
    }
    let roll = random() * 3.0;
    Some((
        if roll < 1.0 {
            S::Shambler
        } else if roll < 2.5 {
            S::DoubleDemon
        } else {
            S::Shalrath
        },
        if (1.0..2.5).contains(&roll) { T::Normal } else { T::Boss },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squads_scale_with_skill() {
        let skilled = horde_squad(HordeSquad::ThreeGrunts, 1, &mut || 0.0);
        assert_eq!(skilled.len(), 3);
        let easy = horde_squad(HordeSquad::ThreeGrunts, 0, &mut || 0.0);
        assert_eq!(easy.len(), 2);
        let shamblers = horde_squad(HordeSquad::DoubleDemon, 3, &mut || 0.9);
        assert!(shamblers.iter().all(|spawn| spawn.monster == HordeMonster::Shambler));
    }

    #[test]
    fn selection_matches_rolls() {
        let picked = choose_horde_squad(false, HordeCategory::Fodder, &mut || 0.1).expect("squad");
        assert_eq!(picked, (HordeSquad::TwoKnights, HordeSquadType::Normal));
        assert_eq!(choose_horde_squad(true, HordeCategory::Bosses, &mut || 0.0), None);
    }
}
