//! Q1 mg3 infected species (`src/content/q1/addons/monsters/infected/species.ts`).
//!
//! `quakec_mg3/monsters/mg3_*_infected.qc`. GPL-2.0-or-later.

use std::sync::OnceLock;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::q1::addons::monsters::ordinary::army::ARMY_SPEC;
use crate::q1::base::species::{species_by_classname, MonsterSpecies, BASE_SPECIES};
use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::{q1_error, Q1Error};

/// Infected variant (`InfectedKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfectedKind {
    /// Grunt.
    Army,
    /// Knight.
    Knight,
    /// Enforcer.
    Enforcer,
    /// Hell knight.
    Hellknight,
}

/// Resolve the infected variant for a spawn classname
/// (`infectedClassnames`).
pub fn infected_kind_for_classname(classname: &str) -> Option<InfectedKind> {
    match classname {
        "monster_army_infected" => Some(InfectedKind::Army),
        "monster_knight_infected" => Some(InfectedKind::Knight),
        "monster_enforcer_infected" => Some(InfectedKind::Enforcer),
        "monster_hell_knight_infected" => Some(InfectedKind::Hellknight),
        _ => None,
    }
}

/// Read the infected variant (`infectedKind`).
pub fn infected_kind(game: &Q1EntityServices, id: &ActorId) -> Result<InfectedKind, Q1Error> {
    let entity = game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
    match entity.text("infected.kind").as_str() {
        "army" => Ok(InfectedKind::Army),
        "knight" => Ok(InfectedKind::Knight),
        "enforcer" => Ok(InfectedKind::Enforcer),
        "hellknight" => Ok(InfectedKind::Hellknight),
        _ => Err(q1_error(format!(
            "Missing MG3 infected variant on {}",
            entity.classname
        ))),
    }
}

fn zombie_infected_spec() -> &'static MonsterSpecies {
    static SPEC: OnceLock<MonsterSpecies> = OnceLock::new();
    SPEC.get_or_init(|| {
        let base = species_by_classname("monster_zombie").expect("Missing base species zombie");
        MonsterSpecies {
            missile: None,
            melee: true,
            ..*base
        }
    })
}

fn enforcer_infected_spec() -> &'static MonsterSpecies {
    static SPEC: OnceLock<MonsterSpecies> = OnceLock::new();
    SPEC.get_or_init(|| {
        let base = species_by_classname("monster_enforcer").expect("Missing base species enforcer");
        MonsterSpecies {
            bounds: Bounds {
                min: Vec3 {
                    x: -32.0,
                    y: -32.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 32.0,
                    y: 32.0,
                    z: 64.0,
                },
            },
            ..*base
        }
    })
}

fn corpse_spec(corpse: u32) -> &'static MonsterSpecies {
    static CORPSE1: OnceLock<MonsterSpecies> = OnceLock::new();
    static CORPSE2: OnceLock<MonsterSpecies> = OnceLock::new();
    let slot = if corpse == 1 { &CORPSE1 } else { &CORPSE2 };
    slot.get_or_init(|| {
        let base = species_by_classname("monster_hell_knight").expect("Missing base species hellknight");
        MonsterSpecies {
            stand: if corpse == 1 {
                "hknight_corpse1"
            } else {
                "hknight_corpse2"
            },
            run: if corpse == 1 {
                "hknight_corpse1_rise0"
            } else {
                "hknight_corpse2_rise0"
            },
            ..*base
        }
    })
}

/// Resolve the infected spawn spec (`infectedSpecies`).
pub fn infected_species(game: &Q1EntityServices, id: &ActorId) -> Result<&'static MonsterSpecies, Q1Error> {
    let kind = infected_kind(game, id)?;
    let entity = game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let transformed = entity.number("infected.transformed") != 0.0;
    if !transformed {
        return match kind {
            InfectedKind::Army => Ok(&ARMY_SPEC),
            InfectedKind::Knight => BASE_SPECIES
                .iter()
                .find(|spec| spec.species == Q1MonsterSpecies::Knight)
                .ok_or_else(|| q1_error("Missing base species knight")),
            InfectedKind::Enforcer => Ok(enforcer_infected_spec()),
            InfectedKind::Hellknight => {
                if entity.number("infected.risen") != 0.0 {
                    return BASE_SPECIES
                        .iter()
                        .find(|spec| spec.species == Q1MonsterSpecies::Hellknight)
                        .ok_or_else(|| q1_error("Missing base species hellknight"));
                }
                let corpse = if entity.spawnflags & 65536 != 0 {
                    1
                } else if entity.spawnflags & 8388608 != 0 {
                    2
                } else {
                    0
                };
                if corpse == 0 {
                    return BASE_SPECIES
                        .iter()
                        .find(|spec| spec.species == Q1MonsterSpecies::Hellknight)
                        .ok_or_else(|| q1_error("Missing base species hellknight"));
                }
                Ok(corpse_spec(corpse))
            }
        };
    }
    match kind {
        InfectedKind::Army | InfectedKind::Knight => Ok(zombie_infected_spec()),
        InfectedKind::Enforcer | InfectedKind::Hellknight => BASE_SPECIES
            .iter()
            .find(|spec| spec.species == Q1MonsterSpecies::Demon)
            .ok_or_else(|| q1_error("Missing base species demon")),
    }
}
