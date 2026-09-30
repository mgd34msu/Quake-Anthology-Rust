//! Base monster spawn defaults (`src/content/q1/base/species.ts`).
//!
//! QuakeC monster spawn defaults. Copyright (C) 1996-2022 id Software
//! LLC. GPL-2.0-or-later.

use qa_core::math::{Bounds, Vec3};

use crate::q1::foundation::entity::Q1MonsterSpecies;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::{q1_error, Q1Error};

/// Base-campaign species (`BaseSpecies`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BaseSpecies {
    /// Knight.
    Knight,
    /// Enforcer.
    Enforcer,
    /// Fiend.
    Demon,
    /// Ogre.
    Ogre,
    /// Death knight.
    Hellknight,
    /// Shambler.
    Shambler,
    /// Scrag.
    Wizard,
    /// Vore.
    Shalrath,
    /// Spawn.
    Tarbaby,
    /// Rotfish.
    Fish,
    /// Zombie.
    Zombie,
    /// Chthon.
    Boss,
    /// Shub-Niggurath.
    Oldone,
}

impl BaseSpecies {
    /// Donor species text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        Q1MonsterSpecies::from(*self).as_str()
    }

    /// Parse donor species text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match Q1MonsterSpecies::parse(text)? {
            Q1MonsterSpecies::Knight => Ok(BaseSpecies::Knight),
            Q1MonsterSpecies::Enforcer => Ok(BaseSpecies::Enforcer),
            Q1MonsterSpecies::Demon => Ok(BaseSpecies::Demon),
            Q1MonsterSpecies::Ogre => Ok(BaseSpecies::Ogre),
            Q1MonsterSpecies::Hellknight => Ok(BaseSpecies::Hellknight),
            Q1MonsterSpecies::Shambler => Ok(BaseSpecies::Shambler),
            Q1MonsterSpecies::Wizard => Ok(BaseSpecies::Wizard),
            Q1MonsterSpecies::Shalrath => Ok(BaseSpecies::Shalrath),
            Q1MonsterSpecies::Tarbaby => Ok(BaseSpecies::Tarbaby),
            Q1MonsterSpecies::Fish => Ok(BaseSpecies::Fish),
            Q1MonsterSpecies::Zombie => Ok(BaseSpecies::Zombie),
            Q1MonsterSpecies::Boss => Ok(BaseSpecies::Boss),
            Q1MonsterSpecies::Oldone => Ok(BaseSpecies::Oldone),
            species => Err(q1_error(format!("Not a Q1 base monster species: {}", species.as_str()))),
        }
    }
}

impl From<BaseSpecies> for Q1MonsterSpecies {
    fn from(species: BaseSpecies) -> Self {
        match species {
            BaseSpecies::Knight => Q1MonsterSpecies::Knight,
            BaseSpecies::Enforcer => Q1MonsterSpecies::Enforcer,
            BaseSpecies::Demon => Q1MonsterSpecies::Demon,
            BaseSpecies::Ogre => Q1MonsterSpecies::Ogre,
            BaseSpecies::Hellknight => Q1MonsterSpecies::Hellknight,
            BaseSpecies::Shambler => Q1MonsterSpecies::Shambler,
            BaseSpecies::Wizard => Q1MonsterSpecies::Wizard,
            BaseSpecies::Shalrath => Q1MonsterSpecies::Shalrath,
            BaseSpecies::Tarbaby => Q1MonsterSpecies::Tarbaby,
            BaseSpecies::Fish => Q1MonsterSpecies::Fish,
            BaseSpecies::Zombie => Q1MonsterSpecies::Zombie,
            BaseSpecies::Boss => Q1MonsterSpecies::Boss,
            BaseSpecies::Oldone => Q1MonsterSpecies::Oldone,
        }
    }
}

/// Monster locomotion (`MonsterSpecies["movement"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterMovement {
    /// Walk.
    Walk,
    /// Fly.
    Fly,
    /// Swim.
    Swim,
    /// Stationary boss.
    Boss,
}

impl MonsterMovement {
    /// Donor movement text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            MonsterMovement::Walk => "walk",
            MonsterMovement::Fly => "fly",
            MonsterMovement::Swim => "swim",
            MonsterMovement::Boss => "boss",
        }
    }
}

/// Base monster spawn defaults (`MonsterSpecies`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonsterSpecies {
    /// Monster species.
    pub species: Q1MonsterSpecies,
    /// Obituary kill string, if any.
    pub kill_string: Option<&'static str>,
    /// Spawn classnames.
    pub classnames: &'static [&'static str],
    /// Model name without `progs/` or `.mdl`.
    pub model: &'static str,
    /// Head gib model name, if any.
    pub head: Option<&'static str>,
    /// Spawn health.
    pub health: f64,
    /// Health below which the monster gibs.
    pub gib_health: f64,
    /// Gib model names.
    pub gibs: &'static [&'static str],
    /// Collision bounds.
    pub bounds: Bounds,
    /// Stand frame.
    pub stand: &'static str,
    /// Walk frame.
    pub walk: &'static str,
    /// Run frame.
    pub run: &'static str,
    /// Sight sound.
    pub sight: &'static str,
    /// Missile frame, if any.
    pub missile: Option<&'static str>,
    /// Whether the monster has a melee attack.
    pub melee: bool,
    /// Locomotion.
    pub movement: MonsterMovement,
}

const HUMAN: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 40.0,
    },
};
const LARGE: Bounds = Bounds {
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
};
const GIBS: &[&str] = &["gib1", "gib2", "gib3"];

/// Base monster roster in donor order (`baseSpecies`).
pub const BASE_SPECIES: &[MonsterSpecies] = &[
    MonsterSpecies {
        species: Q1MonsterSpecies::Knight,
        kill_string: Some("$qc_ks_knight"),
        classnames: &["monster_knight"],
        model: "knight",
        head: Some("h_knight"),
        health: 75.0,
        gib_health: -40.0,
        gibs: GIBS,
        bounds: HUMAN,
        stand: "knight_stand1",
        walk: "knight_walk1",
        run: "knight_run1",
        sight: "knight/ksight.wav",
        missile: None,
        melee: true,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Enforcer,
        kill_string: Some("$qc_ks_enforcer"),
        classnames: &["monster_enforcer"],
        model: "enforcer",
        head: Some("h_mega"),
        health: 80.0,
        gib_health: -35.0,
        gibs: GIBS,
        bounds: HUMAN,
        stand: "enf_stand1",
        walk: "enf_walk1",
        run: "enf_run1",
        sight: "enforcer/sight1.wav",
        missile: Some("enf_atk1"),
        melee: false,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Demon,
        kill_string: Some("$qc_ks_fiend"),
        classnames: &["monster_demon1"],
        model: "demon",
        head: Some("h_demon"),
        health: 300.0,
        gib_health: -80.0,
        gibs: &["gib1", "gib1", "gib1"],
        bounds: LARGE,
        stand: "demon1_stand1",
        walk: "demon1_walk1",
        run: "demon1_run1",
        sight: "demon/sight2.wav",
        missile: Some("demon1_jump1"),
        melee: true,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Ogre,
        kill_string: Some("$qc_ks_ogre"),
        classnames: &["monster_ogre", "monster_ogre_marksman"],
        model: "ogre",
        head: Some("h_ogre"),
        health: 200.0,
        gib_health: -80.0,
        gibs: &["gib3", "gib3", "gib3"],
        bounds: LARGE,
        stand: "ogre_stand1",
        walk: "ogre_walk1",
        run: "ogre_run1",
        sight: "ogre/ogwake.wav",
        missile: Some("ogre_nail1"),
        melee: true,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Hellknight,
        kill_string: Some("$qc_ks_deathknight"),
        classnames: &["monster_hell_knight"],
        model: "hknight",
        head: Some("h_hellkn"),
        health: 250.0,
        gib_health: -40.0,
        gibs: GIBS,
        bounds: HUMAN,
        stand: "hknight_stand1",
        walk: "hknight_walk1",
        run: "hknight_run1",
        sight: "hknight/sight1.wav",
        missile: Some("hknight_magicc1"),
        melee: true,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Shambler,
        kill_string: Some("$qc_ks_shambler"),
        classnames: &["monster_shambler"],
        model: "shambler",
        head: Some("h_shams"),
        health: 600.0,
        gib_health: -60.0,
        gibs: GIBS,
        bounds: LARGE,
        stand: "sham_stand1",
        walk: "sham_walk1",
        run: "sham_run1",
        sight: "shambler/ssight.wav",
        missile: Some("sham_magic1"),
        melee: true,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Wizard,
        kill_string: Some("$qc_ks_scrag"),
        classnames: &["monster_wizard"],
        model: "wizard",
        head: Some("h_wizard"),
        health: 80.0,
        gib_health: -40.0,
        gibs: &["gib2", "gib2", "gib2"],
        bounds: HUMAN,
        stand: "wiz_stand1",
        walk: "wiz_walk1",
        run: "wiz_run1",
        sight: "wizard/wsight.wav",
        missile: Some("wiz_fast1"),
        melee: false,
        movement: MonsterMovement::Fly,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Shalrath,
        kill_string: Some("$qc_ks_vore"),
        classnames: &["monster_shalrath"],
        model: "shalrath",
        head: Some("h_shal"),
        health: 400.0,
        gib_health: -90.0,
        gibs: GIBS,
        bounds: LARGE,
        stand: "shal_stand",
        walk: "shal_walk1",
        run: "shal_run1",
        sight: "shalrath/sight.wav",
        missile: Some("shal_attack1"),
        melee: false,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Tarbaby,
        kill_string: Some("$qc_ks_spawn"),
        classnames: &["monster_tarbaby"],
        model: "tarbaby",
        head: None,
        health: 80.0,
        gib_health: f64::NEG_INFINITY,
        gibs: &[],
        bounds: HUMAN,
        stand: "tbaby_stand1",
        walk: "tbaby_walk1",
        run: "tbaby_run1",
        sight: "blob/sight1.wav",
        missile: Some("tbaby_jump1"),
        melee: true,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Fish,
        kill_string: Some("$qc_ks_rotfish"),
        classnames: &["monster_fish"],
        model: "fish",
        head: None,
        health: 25.0,
        gib_health: f64::NEG_INFINITY,
        gibs: &[],
        bounds: Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 24.0,
            },
        },
        stand: "f_stand1",
        walk: "f_walk1",
        run: "f_run1",
        sight: "fish/idle.wav",
        missile: None,
        melee: true,
        movement: MonsterMovement::Swim,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Zombie,
        kill_string: Some("$qc_ks_zombie"),
        classnames: &["monster_zombie"],
        model: "zombie",
        head: Some("h_zombie"),
        health: 60.0,
        gib_health: 0.0,
        gibs: GIBS,
        bounds: HUMAN,
        stand: "zombie_stand1",
        walk: "zombie_walk1",
        run: "zombie_run1",
        sight: "zombie/z_idle.wav",
        missile: Some("zombie_atta1"),
        melee: false,
        movement: MonsterMovement::Walk,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Boss,
        kill_string: Some("$qc_ks_chthon"),
        classnames: &["monster_boss"],
        model: "boss",
        head: None,
        health: 3.0,
        gib_health: f64::NEG_INFINITY,
        gibs: &[],
        bounds: Bounds {
            min: Vec3 {
                x: -128.0,
                y: -128.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 128.0,
                y: 128.0,
                z: 256.0,
            },
        },
        stand: "boss_idle1",
        walk: "boss_idle1",
        run: "boss_missile1",
        sight: "boss1/sight1.wav",
        missile: Some("boss_missile1"),
        melee: false,
        movement: MonsterMovement::Boss,
    },
    MonsterSpecies {
        species: Q1MonsterSpecies::Oldone,
        kill_string: Some("$qc_ks_shub"),
        classnames: &["monster_oldone"],
        model: "oldone",
        head: None,
        health: 40000.0,
        gib_health: f64::NEG_INFINITY,
        gibs: &[],
        bounds: Bounds {
            min: Vec3 {
                x: -160.0,
                y: -128.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 160.0,
                y: 128.0,
                z: 256.0,
            },
        },
        stand: "old_idle1",
        walk: "old_idle1",
        run: "old_idle1",
        sight: "boss2/sight.wav",
        missile: None,
        melee: false,
        movement: MonsterMovement::Boss,
    },
];

/// Look up base species by spawn classname.
#[must_use]
pub fn species_by_classname(classname: &str) -> Option<&'static MonsterSpecies> {
    BASE_SPECIES
        .iter()
        .find(|species| species.classnames.contains(&classname))
}

/// Look up base species by species id.
#[must_use]
pub fn species_by_id(species: Q1MonsterSpecies) -> Option<&'static MonsterSpecies> {
    BASE_SPECIES.iter().find(|candidate| candidate.species == species)
}

/// progs106 monster spawn declarations, before model assignment and
/// startup (`precacheId1Monster`).
pub fn precache_id1_monster(game: &mut Q1EntityServices, species: Q1MonsterSpecies) -> Result<(), Q1Error> {
    let (models, sounds): (&[&str], &[&str]) = match species {
        Q1MonsterSpecies::Knight => (
            &["progs/knight.mdl", "progs/h_knight.mdl"],
            &[
                "knight/kdeath.wav",
                "knight/khurt.wav",
                "knight/ksight.wav",
                "knight/sword1.wav",
                "knight/sword2.wav",
                "knight/idle.wav",
            ],
        ),
        Q1MonsterSpecies::Enforcer => (
            &["progs/enforcer.mdl", "progs/h_mega.mdl", "progs/laser.mdl"],
            &[
                "enforcer/death1.wav",
                "enforcer/enfire.wav",
                "enforcer/enfstop.wav",
                "enforcer/idle1.wav",
                "enforcer/pain1.wav",
                "enforcer/pain2.wav",
                "enforcer/sight1.wav",
                "enforcer/sight2.wav",
                "enforcer/sight3.wav",
                "enforcer/sight4.wav",
            ],
        ),
        Q1MonsterSpecies::Demon => (
            &["progs/demon.mdl", "progs/h_demon.mdl"],
            &[
                "demon/ddeath.wav",
                "demon/dhit2.wav",
                "demon/djump.wav",
                "demon/dpain1.wav",
                "demon/idle1.wav",
                "demon/sight2.wav",
            ],
        ),
        Q1MonsterSpecies::Ogre => (
            &["progs/ogre.mdl", "progs/h_ogre.mdl", "progs/grenade.mdl"],
            &[
                "ogre/ogdrag.wav",
                "ogre/ogdth.wav",
                "ogre/ogidle.wav",
                "ogre/ogidle2.wav",
                "ogre/ogpain1.wav",
                "ogre/ogsawatk.wav",
                "ogre/ogwake.wav",
            ],
        ),
        Q1MonsterSpecies::Hellknight => (
            &["progs/hknight.mdl", "progs/k_spike.mdl", "progs/h_hellkn.mdl"],
            &[
                "hknight/attack1.wav",
                "hknight/death1.wav",
                "hknight/pain1.wav",
                "hknight/sight1.wav",
                "hknight/hit.wav",
                "hknight/slash1.wav",
                "hknight/idle.wav",
                "hknight/grunt.wav",
                "knight/sword1.wav",
                "knight/sword2.wav",
            ],
        ),
        Q1MonsterSpecies::Shambler => (
            &[
                "progs/shambler.mdl",
                "progs/s_light.mdl",
                "progs/h_shams.mdl",
                "progs/bolt.mdl",
            ],
            &[
                "shambler/sattck1.wav",
                "shambler/sboom.wav",
                "shambler/sdeath.wav",
                "shambler/shurt2.wav",
                "shambler/sidle.wav",
                "shambler/ssight.wav",
                "shambler/melee1.wav",
                "shambler/melee2.wav",
                "shambler/smack.wav",
            ],
        ),
        Q1MonsterSpecies::Wizard => (
            &["progs/wizard.mdl", "progs/h_wizard.mdl", "progs/w_spike.mdl"],
            &[
                "wizard/hit.wav",
                "wizard/wattack.wav",
                "wizard/wdeath.wav",
                "wizard/widle1.wav",
                "wizard/widle2.wav",
                "wizard/wpain.wav",
                "wizard/wsight.wav",
            ],
        ),
        Q1MonsterSpecies::Shalrath => (
            &["progs/shalrath.mdl", "progs/h_shal.mdl", "progs/v_spike.mdl"],
            &[
                "shalrath/attack.wav",
                "shalrath/attack2.wav",
                "shalrath/death.wav",
                "shalrath/idle.wav",
                "shalrath/pain.wav",
                "shalrath/sight.wav",
            ],
        ),
        Q1MonsterSpecies::Tarbaby => (
            &["progs/tarbaby.mdl"],
            &["blob/death1.wav", "blob/hit1.wav", "blob/land1.wav", "blob/sight1.wav"],
        ),
        Q1MonsterSpecies::Fish => (
            &["progs/fish.mdl"],
            &["fish/death.wav", "fish/bite.wav", "fish/idle.wav"],
        ),
        Q1MonsterSpecies::Zombie => (
            &["progs/zombie.mdl", "progs/h_zombie.mdl", "progs/zom_gib.mdl"],
            &[
                "zombie/z_idle.wav",
                "zombie/z_idle1.wav",
                "zombie/z_shot1.wav",
                "zombie/z_gib.wav",
                "zombie/z_pain.wav",
                "zombie/z_pain1.wav",
                "zombie/z_fall.wav",
                "zombie/z_miss.wav",
                "zombie/z_hit.wav",
                "zombie/idle_w2.wav",
            ],
        ),
        Q1MonsterSpecies::Boss => (
            &["progs/boss.mdl", "progs/lavaball.mdl"],
            &[
                "weapons/rocket1i.wav",
                "boss1/out1.wav",
                "boss1/sight1.wav",
                "misc/power.wav",
                "boss1/throw.wav",
                "boss1/pain.wav",
                "boss1/death.wav",
            ],
        ),
        Q1MonsterSpecies::Oldone => (
            &["progs/oldone.mdl"],
            &["boss2/death.wav", "boss2/idle.wav", "boss2/sight.wav", "boss2/pop2.wav"],
        ),
        other => {
            return Err(q1_error(format!(
                "No id1 base monster precaches for {}",
                other.as_str()
            )))
        }
    };
    for model in models {
        game.precache_model(model)?;
    }
    for sound in sounds {
        game.precache_sound(sound)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    #[test]
    fn roster_resolves_classnames() {
        assert_eq!(BASE_SPECIES.len(), 13);
        let ogre = species_by_classname("monster_ogre_marksman").expect("marksman");
        assert_eq!(ogre.species, Q1MonsterSpecies::Ogre);
        assert_eq!(ogre.classnames.len(), 2);
        assert!(species_by_classname("monster_army").is_none());
        assert_eq!(BaseSpecies::parse("boss"), Ok(BaseSpecies::Boss));
        assert!(BaseSpecies::parse("army").is_err());
        assert_eq!(BaseSpecies::Boss.as_str(), "boss");
    }

    #[test]
    fn precaches_declare_donor_assets() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        precache_id1_monster(&mut game, Q1MonsterSpecies::Shambler).expect("precache");
        assert!(precache_id1_monster(&mut game, Q1MonsterSpecies::Army).is_err());
    }
}
