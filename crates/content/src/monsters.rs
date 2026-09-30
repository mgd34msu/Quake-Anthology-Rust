//! Game-content monster catalog (`src/content/monsters`).
//!
//! Donor provenance: `src/content/monsters/authored.ts`,
//! `src/content/monsters/definitions.ts`,
//! `src/content/monsters/expansions.ts`,
//! `src/content/monsters/mg3-resources.ts`,
//! `src/content/monsters/q1.ts`,
//! `src/content/monsters/q2-expansions.ts`,
//! `src/content/monsters/q2-rerelease.ts`,
//! `src/content/monsters/q2.ts`,
//! `src/content/monsters/roster.ts`,
//! `src/content/monsters/target.ts`.
//!
//! Creature tables are built fresh by each constructor (the donor's
//! `readonly` records become [`HashMap`]s of owned strings). Resource
//! vectors keep donor order; merges that flatten a whole table iterate
//! it in classname order so output stays deterministic.

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::Vec3;
use qa_core::numeric::{Q1_DONOR_PROFILE, Q2_DONOR_PROFILE};
use qa_core::time::ClockProfile;
use thiserror::Error;

use crate::contract::{
    EnemySelection, GameFamily, MonsterDefinitionReference, MonsterSelectionTarget, ProviderReference, ProviderTiming,
    SourceEdition,
};

/// Monster catalog lookup failure (donor `RangeError` throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MonsterError {
    /// The selected classname is not implemented by the source.
    #[error("Selected monster implementation is unavailable: {provider}/{classname}")]
    Unavailable {
        /// Requested provider text.
        provider: String,
        /// Requested classname.
        classname: String,
    },
    /// No monster source carries the requested provider.
    #[error("Unknown monster source: {0}")]
    UnknownSource(String),
}

/// Build a [`ProviderId`] from donor `"namespace:name"` text.
fn provider_id(text: &str) -> ProviderId {
    match text.split_once(':') {
        Some((namespace, name)) => ProviderId::new(namespace, name),
        None => ProviderId::new("", text),
    }
}

/// Render a [`ProviderId`] as donor `"namespace:name"` text.
#[must_use]
pub fn provider_text(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

/// Build one creature entry from resource path literals.
fn creature(resources: &[&str]) -> MonsterCreature {
    MonsterCreature {
        resources: resources.iter().map(ToString::to_string).collect(),
    }
}

/// Build a creature table from `(classname, creature)` pairs.
fn catalog(entries: Vec<(&str, MonsterCreature)>) -> HashMap<String, MonsterCreature> {
    entries
        .into_iter()
        .map(|(classname, entry)| (classname.to_string(), entry))
        .collect()
}

/// Union `items` into `resources`, preserving first-occurrence order
/// (donor `new Set([...])` spread semantics).
fn union_ordered(resources: &mut Vec<String>, items: &[String]) {
    for item in items {
        if !resources.contains(item) {
            resources.push(item.clone());
        }
    }
}

// `authored.ts`.

/// Live map-script fields; source entity records can supply these fields
/// directly (`AuthoredTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredTarget {
    /// Owning actor handle.
    pub actor: OwnedActor,
    /// Entity classname.
    pub classname: String,
    /// This entity's target name.
    pub targetname: String,
    /// Trigger target.
    pub target: String,
    /// Kill target.
    pub killtarget: String,
    /// Center-print message.
    pub message: String,
    /// Trigger delay in seconds.
    pub delay: f64,
}

/// Barrier pinning a waiting monster (`placement` barrier entry).
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterBarrier {
    /// Barrier actor.
    pub actor: ActorId,
    /// Barrier origin.
    pub origin: Vec3,
}

/// Spawn placement of an authored monster (`placement` union).
#[derive(Debug, Clone, PartialEq)]
pub enum MonsterPlacement {
    /// Placed and ready.
    Ready,
    /// Teleporting to an origin.
    Teleport {
        /// Teleport destination.
        origin: Vec3,
    },
    /// Waiting on barriers.
    Waiting {
        /// Pinning barriers.
        barriers: Vec<MonsterBarrier>,
        /// Entity that will release the monster.
        activator: Option<ActorId>,
    },
}

/// Activation state of an authored monster (`activation` union).
#[derive(Debug, Clone, PartialEq)]
pub enum MonsterActivation {
    /// Active in the world.
    Active,
    /// Dormant until used.
    Dormant,
    /// Scheduled to activate at a time.
    Scheduled {
        /// Activation time in seconds.
        at: f64,
        /// Scheduling activator.
        activator: Option<ActorId>,
    },
}

/// Live authored monster entity (`AuthoredMonster`).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredMonster {
    /// Shared map-script fields.
    pub target: AuthoredTarget,
    /// Source entity ordinal.
    pub source_ordinal: u32,
    /// Spawn flags bit field.
    pub spawnflags: u32,
    /// Target fired on death.
    pub death_target: String,
    /// Item classname dropped on death.
    pub drop_item: String,
    /// Patrol route name.
    pub route: String,
    /// Resolved patrol goal.
    pub route_goal: Option<ActorId>,
    /// Whether the patrol goal resolved.
    pub route_resolved: bool,
    /// Whether the death counted toward the kill total.
    pub counted_death: bool,
    /// Combat target name.
    pub combat_target: String,
    /// Resolved combat goal.
    pub combat_goal: Option<ActorId>,
    /// Whether the monster holds its ground in combat.
    pub stand_ground: bool,
    /// Spawn placement.
    pub placement: MonsterPlacement,
    /// Activation state.
    pub activation: MonsterActivation,
}

/// Combat patrol goal plus stance (`combatRoute` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatRoute {
    /// Combat goal.
    pub goal: Option<ActorId>,
    /// Whether the monster holds its ground.
    pub stand_ground: bool,
}

/// Authored monster mission callbacks (`MonsterMission`).
pub trait MonsterMission {
    /// The monster spawned into the world.
    fn spawned(&mut self);
    /// The monster started its mission behavior.
    fn started(&mut self);
    /// The monster was killed, optionally by an attacker.
    fn killed(&mut self, attacker: Option<&ActorId>);
    /// Current patrol goal.
    fn route(&self) -> Option<ActorId>;
    /// Use the monster, reporting whether the use applied (donor `use`).
    fn r#use(&mut self, activator: Option<&ActorId>) -> bool;
    /// Current combat goal plus stance.
    fn combat_route(&self) -> CombatRoute;
    /// The monster acquired a target.
    fn found_target(&mut self);
}

// `target.ts`.

/// What a monster observes about a potential target
/// (`MonsterTargetObservation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MonsterTargetObservation {
    /// Eye height above the origin.
    pub view_height: f64,
    /// Target ignores monster targeting.
    pub notarget: bool,
    /// Target is invisible.
    pub invisible: bool,
    /// Light level at the target, when known.
    pub light_level: Option<f64>,
    /// Time until which the monster stays hostile, when set.
    pub hostile_until: Option<f64>,
}

/// Whether a living target without `notarget` can be selected
/// (`monsterTargetEligible`).
#[must_use]
pub fn monster_target_eligible(health: f64, observation: Option<&MonsterTargetObservation>) -> bool {
    health > 0.0 && observation.is_some_and(|seen| !seen.notarget)
}

// `definitions.ts`.

/// Monster game family (`"q1" | "q2"` in `MonsterSourceDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
}

impl MonsterFamily {
    /// Donor family text (`"q1"`, `"q2"`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            MonsterFamily::Q1 => "q1",
            MonsterFamily::Q2 => "q2",
        }
    }
}

/// Monster program within a family (donor `program` union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterProgram {
    /// Quake base game.
    Id1,
    /// Quake Hipnotic expansion.
    Hipnotic,
    /// Quake or Quake II rogue expansion.
    Rogue,
    /// Quake Dimension of the Past.
    Dopa,
    /// Quake Machinegames episode 1.
    Mg1,
    /// Quake Machinegames episode 3.
    Mg3,
    /// Quake II base game.
    Baseq2,
    /// Quake II Xatrix expansion.
    Xatrix,
    /// Quake II Machinegames expansion.
    Mg2,
}

impl MonsterProgram {
    /// Donor program text (`"id1"`, `"hipnotic"`, ...).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            MonsterProgram::Id1 => "id1",
            MonsterProgram::Hipnotic => "hipnotic",
            MonsterProgram::Rogue => "rogue",
            MonsterProgram::Dopa => "dopa",
            MonsterProgram::Mg1 => "mg1",
            MonsterProgram::Mg3 => "mg3",
            MonsterProgram::Baseq2 => "baseq2",
            MonsterProgram::Xatrix => "xatrix",
            MonsterProgram::Mg2 => "mg2",
        }
    }
}

/// One monster implementation plus its precached resources
/// (`creatures` entry in `MonsterSourceDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterCreature {
    /// Precached resource paths.
    pub resources: Vec<String>,
}

/// Monster implementations bound to one provider
/// (`MonsterSourceDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterSourceDefinition {
    /// Defining provider.
    pub provider: ProviderId,
    /// Source edition.
    pub edition: SourceEdition,
    /// Implementations keyed by classname.
    pub creatures: HashMap<String, MonsterCreature>,
    /// Game family.
    pub family: MonsterFamily,
    /// Program within the family.
    pub program: MonsterProgram,
}

/// Build one source definition from its parts.
fn source(
    provider: &str,
    family: MonsterFamily,
    edition: SourceEdition,
    program: MonsterProgram,
    creatures: HashMap<String, MonsterCreature>,
) -> MonsterSourceDefinition {
    MonsterSourceDefinition {
        provider: provider_id(provider),
        edition,
        creatures,
        family,
        program,
    }
}

/// Donor edition text (`"classic"`, `"rerelease"`).
fn edition_name(edition: SourceEdition) -> &'static str {
    match edition {
        SourceEdition::Classic => "classic",
        SourceEdition::Rerelease => "rerelease",
    }
}

/// These identities bind the existing source modules, including their
/// edition-specific continuations (`monsterSources`).
#[must_use]
pub fn monster_sources() -> Vec<MonsterSourceDefinition> {
    let mut sources = q1_monster_sources();
    for mut candidate in q2_monster_sources() {
        let extra = if candidate.edition == SourceEdition::Rerelease {
            q2_expanded_base_creatures()
        } else {
            q2_expanded_classic_creatures()
        };
        candidate.creatures.extend(extra);
        sources.push(candidate);
    }
    sources.extend(q1_expansion_monster_sources());
    sources.extend(q1_addon_monster_sources());
    sources.extend(q2_expansion_sources());
    sources
}

/// Resolve the source implementing a monster definition (`monsterSource`).
pub fn monster_source(definition: &MonsterDefinitionReference) -> Result<MonsterSourceDefinition, MonsterError> {
    let found = monster_sources()
        .into_iter()
        .find(|candidate| candidate.provider == definition.source.provider);
    match found {
        Some(source) if source.creatures.contains_key(&definition.classname) => Ok(source),
        _ => Err(MonsterError::Unavailable {
            provider: provider_text(&definition.source.provider),
            classname: definition.classname.clone(),
        }),
    }
}

/// Numeric plus clock profile for a monster source (`monsterTiming`).
///
/// The donor's Q2 classic `frameMilliseconds: 100` and rerelease
/// `preparation: "before-frame"` are implied by
/// [`ClockProfile::Q2Classic`] and [`ClockProfile::Q2Rerelease`].
#[must_use]
pub fn monster_timing(source: &MonsterSourceDefinition) -> ProviderTiming {
    let (numeric, clock) = match (source.family, source.edition) {
        (MonsterFamily::Q1, _) => (
            Q1_DONOR_PROFILE,
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.001,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None,
            },
        ),
        (_, SourceEdition::Classic) => (Q2_DONOR_PROFILE, ClockProfile::Q2Classic),
        (_, SourceEdition::Rerelease) => (
            Q2_DONOR_PROFILE,
            ClockProfile::Q2Rerelease {
                frame_milliseconds: 25.0,
            },
        ),
    };
    ProviderTiming {
        provider: source.provider.clone(),
        clock,
        numeric,
    }
}

// `q1.ts`.

/// Quake grunt (`army`).
fn q1_army() -> MonsterCreature {
    creature(&[
        "progs/soldier.mdl",
        "progs/h_guard.mdl",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/backpack.mdl",
        "sound/soldier/death1.wav",
        "sound/soldier/idle.wav",
        "sound/soldier/pain1.wav",
        "sound/soldier/pain2.wav",
        "sound/soldier/sattck1.wav",
        "sound/soldier/sight1.wav",
        "sound/player/udeath.wav",
        "sound/weapons/lock4.wav",
    ])
}

/// Quake attack dog (`dog`).
fn q1_dog() -> MonsterCreature {
    creature(&[
        "progs/dog.mdl",
        "progs/h_dog.mdl",
        "progs/gib3.mdl",
        "sound/dog/dattack1.wav",
        "sound/dog/ddeath.wav",
        "sound/dog/dpain1.wav",
        "sound/dog/dsight.wav",
        "sound/dog/idle.wav",
        "sound/player/udeath.wav",
    ])
}

/// Quake enforcer (`enforcer`).
fn q1_enforcer() -> MonsterCreature {
    creature(&[
        "progs/enforcer.mdl",
        "progs/h_mega.mdl",
        "progs/laser.mdl",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/backpack.mdl",
        "sound/enforcer/death1.wav",
        "sound/enforcer/enfire.wav",
        "sound/enforcer/enfstop.wav",
        "sound/enforcer/idle1.wav",
        "sound/enforcer/pain1.wav",
        "sound/enforcer/pain2.wav",
        "sound/enforcer/sight1.wav",
        "sound/enforcer/sight2.wav",
        "sound/enforcer/sight3.wav",
        "sound/enforcer/sight4.wav",
        "sound/player/udeath.wav",
        "sound/weapons/lock4.wav",
    ])
}

/// Quake knight (`knight`).
fn q1_knight() -> MonsterCreature {
    creature(&[
        "progs/knight.mdl",
        "progs/h_knight.mdl",
        "sound/knight/kdeath.wav",
        "sound/knight/khurt.wav",
        "sound/knight/ksight.wav",
        "sound/knight/sword1.wav",
        "sound/knight/sword2.wav",
        "sound/knight/idle.wav",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "sound/player/udeath.wav",
    ])
}

/// Quake fiend (`demon`).
fn q1_demon() -> MonsterCreature {
    creature(&[
        "progs/demon.mdl",
        "progs/h_demon.mdl",
        "sound/demon/ddeath.wav",
        "sound/demon/dhit2.wav",
        "sound/demon/djump.wav",
        "sound/demon/dpain1.wav",
        "sound/demon/idle1.wav",
        "sound/demon/sight2.wav",
        "progs/gib1.mdl",
        "progs/zom_gib.mdl",
        "sound/player/udeath.wav",
    ])
}

/// Quake ogre (`ogre`).
fn q1_ogre() -> MonsterCreature {
    creature(&[
        "progs/ogre.mdl",
        "progs/h_ogre.mdl",
        "progs/grenade.mdl",
        "sound/ogre/ogdrag.wav",
        "sound/ogre/ogdth.wav",
        "sound/ogre/ogidle.wav",
        "sound/ogre/ogidle2.wav",
        "sound/ogre/ogpain1.wav",
        "sound/ogre/ogsawatk.wav",
        "sound/ogre/ogwake.wav",
        "progs/gib3.mdl",
        "progs/zom_gib.mdl",
        "progs/backpack.mdl",
        "progs/s_explod.spr",
        "sound/player/udeath.wav",
        "sound/weapons/grenade.wav",
        "sound/weapons/r_exp3.wav",
        "sound/weapons/bounce.wav",
        "sound/weapons/lock4.wav",
    ])
}

/// Quake hell knight (`hellknight`).
fn q1_hellknight() -> MonsterCreature {
    creature(&[
        "progs/hknight.mdl",
        "progs/k_spike.mdl",
        "progs/h_hellkn.mdl",
        "sound/hknight/attack1.wav",
        "sound/hknight/death1.wav",
        "sound/hknight/pain1.wav",
        "sound/hknight/sight1.wav",
        "sound/hknight/hit.wav",
        "sound/hknight/slash1.wav",
        "sound/hknight/idle.wav",
        "sound/hknight/grunt.wav",
        "sound/knight/sword1.wav",
        "sound/knight/sword2.wav",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "sound/player/udeath.wav",
    ])
}

/// Quake shambler (`shambler`).
fn q1_shambler() -> MonsterCreature {
    creature(&[
        "progs/shambler.mdl",
        "progs/s_light.mdl",
        "progs/h_shams.mdl",
        "progs/bolt.mdl",
        "sound/shambler/sattck1.wav",
        "sound/shambler/sboom.wav",
        "sound/shambler/sdeath.wav",
        "sound/shambler/shurt2.wav",
        "sound/shambler/sidle.wav",
        "sound/shambler/ssight.wav",
        "sound/shambler/melee1.wav",
        "sound/shambler/melee2.wav",
        "sound/shambler/smack.wav",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/zom_gib.mdl",
        "sound/player/udeath.wav",
    ])
}

/// Quake scrag (`wizard`).
fn q1_wizard() -> MonsterCreature {
    creature(&[
        "progs/wizard.mdl",
        "progs/h_wizard.mdl",
        "progs/w_spike.mdl",
        "sound/wizard/hit.wav",
        "sound/wizard/wattack.wav",
        "sound/wizard/wdeath.wav",
        "sound/wizard/widle1.wav",
        "sound/wizard/widle2.wav",
        "sound/wizard/wpain.wav",
        "sound/wizard/wsight.wav",
        "progs/gib2.mdl",
        "sound/player/udeath.wav",
    ])
}

/// Quake shalrath (`shalrath`).
fn q1_shalrath() -> MonsterCreature {
    creature(&[
        "progs/shalrath.mdl",
        "progs/h_shal.mdl",
        "progs/v_spike.mdl",
        "sound/shalrath/attack.wav",
        "sound/shalrath/attack2.wav",
        "sound/shalrath/death.wav",
        "sound/shalrath/idle.wav",
        "sound/shalrath/pain.wav",
        "sound/shalrath/sight.wav",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/s_explod.spr",
        "sound/player/udeath.wav",
        "sound/weapons/r_exp3.wav",
    ])
}

/// Quake spawn (`tarbaby`).
fn q1_tarbaby() -> MonsterCreature {
    creature(&[
        "progs/tarbaby.mdl",
        "sound/blob/death1.wav",
        "sound/blob/hit1.wav",
        "sound/blob/land1.wav",
        "sound/blob/sight1.wav",
    ])
}

/// Quake zombie (`zombie`).
fn q1_zombie() -> MonsterCreature {
    creature(&[
        "progs/zombie.mdl",
        "progs/h_zombie.mdl",
        "progs/zom_gib.mdl",
        "sound/zombie/z_idle.wav",
        "sound/zombie/z_idle1.wav",
        "sound/zombie/z_shot1.wav",
        "sound/zombie/z_gib.wav",
        "sound/zombie/z_pain.wav",
        "sound/zombie/z_pain1.wav",
        "sound/zombie/z_fall.wav",
        "sound/zombie/z_miss.wav",
        "sound/zombie/z_hit.wav",
        "sound/zombie/idle_w2.wav",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
    ])
}

/// Quake rotfish (`fish`).
fn q1_fish() -> MonsterCreature {
    creature(&[
        "progs/fish.mdl",
        "sound/fish/death.wav",
        "sound/fish/bite.wav",
        "sound/fish/idle.wav",
    ])
}

/// Ordinary Quake creatures shared by both editions
/// (`ordinaryCreatures`).
fn q1_ordinary_creatures() -> HashMap<String, MonsterCreature> {
    catalog(vec![
        ("monster_fish", q1_fish()),
        ("monster_army", q1_army()),
        ("monster_dog", q1_dog()),
        ("monster_enforcer", q1_enforcer()),
        ("monster_knight", q1_knight()),
        ("monster_demon1", q1_demon()),
        ("monster_ogre", q1_ogre()),
        ("monster_ogre_marksman", q1_ogre()),
        ("monster_hell_knight", q1_hellknight()),
        ("monster_shambler", q1_shambler()),
        ("monster_wizard", q1_wizard()),
        ("monster_shalrath", q1_shalrath()),
        ("monster_tarbaby", q1_tarbaby()),
        ("monster_zombie", q1_zombie()),
    ])
}

/// Quake base-game monster sources (`q1MonsterSources`).
#[must_use]
pub fn q1_monster_sources() -> Vec<MonsterSourceDefinition> {
    vec![
        source(
            "q1:monsters/classic/id1",
            MonsterFamily::Q1,
            SourceEdition::Classic,
            MonsterProgram::Id1,
            q1_ordinary_creatures(),
        ),
        source(
            "q1:monsters/rerelease/id1",
            MonsterFamily::Q1,
            SourceEdition::Rerelease,
            MonsterProgram::Id1,
            q1_ordinary_creatures(),
        ),
    ]
}

// `mg3-resources.ts`: generated from quakec_mg3/monsters source precache
// declarations.

/// Machinegames episode 3 monster resources (`mg3MonsterResources`).
#[must_use]
pub fn mg3_monster_resources() -> HashMap<String, MonsterCreature> {
    catalog(vec![
        (
            "monster_ogre_rocket",
            creature(&[
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/grenade.mdl",
                "progs/h_ogre.mdl",
                "progs/ogre.mdl",
                "progs/ogre_rocket.mdl",
                "sound/armagon/idle1.wav",
                "sound/armagon/idle2.wav",
                "sound/armagon/idle3.wav",
                "sound/armagon/idle4.wav",
                "sound/armagon/pain.wav",
                "sound/armagon/sight.wav",
                "sound/armagon/sight2.wav",
                "sound/ogre/ogdrag.wav",
                "sound/ogre/ogdth.wav",
                "sound/ogre/ogidle.wav",
                "sound/ogre/ogidle2.wav",
                "sound/ogre/ogpain1.wav",
                "sound/ogre/ogsawatk.wav",
                "sound/ogre/ogwake.wav",
                "sound/player/udeath.wav",
            ]),
        ),
        (
            "monster_demodog",
            creature(&[
                "progs/dog_explosive.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_dog.mdl",
                "sound/dog/dattack1.wav",
                "sound/dog/ddeath.wav",
                "sound/dog/dpain1.wav",
                "sound/dog/dsight.wav",
                "sound/dog/idle.wav",
                "sound/player/udeath.wav",
            ]),
        ),
        (
            "monster_army_infected",
            creature(&[
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_guard.mdl",
                "progs/h_zombie.mdl",
                "progs/soldier.mdl",
                "progs/zom_gib.mdl",
                "progs/zombie.mdl",
                "sound/player/udeath.wav",
                "sound/soldier/death1.wav",
                "sound/soldier/idle.wav",
                "sound/soldier/pain1.wav",
                "sound/soldier/pain2.wav",
                "sound/soldier/sattck1.wav",
                "sound/soldier/sight1.wav",
                "sound/zombie/idle_w2.wav",
                "sound/zombie/z_fall.wav",
                "sound/zombie/z_gib.wav",
                "sound/zombie/z_hit.wav",
                "sound/zombie/z_idle.wav",
                "sound/zombie/z_idle1.wav",
                "sound/zombie/z_miss.wav",
                "sound/zombie/z_pain.wav",
                "sound/zombie/z_pain1.wav",
                "sound/zombie/z_shot1.wav",
            ]),
        ),
        (
            "monster_knight_infected",
            creature(&[
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_knight.mdl",
                "progs/h_zombie.mdl",
                "progs/knight.mdl",
                "progs/zom_gib.mdl",
                "progs/zombie.mdl",
                "sound/knight/idle.wav",
                "sound/knight/kdeath.wav",
                "sound/knight/khurt.wav",
                "sound/knight/ksight.wav",
                "sound/knight/sword1.wav",
                "sound/knight/sword2.wav",
                "sound/player/udeath.wav",
                "sound/zombie/idle_w2.wav",
                "sound/zombie/z_fall.wav",
                "sound/zombie/z_gib.wav",
                "sound/zombie/z_hit.wav",
                "sound/zombie/z_idle.wav",
                "sound/zombie/z_idle1.wav",
                "sound/zombie/z_miss.wav",
                "sound/zombie/z_pain.wav",
                "sound/zombie/z_pain1.wav",
                "sound/zombie/z_shot1.wav",
            ]),
        ),
        (
            "monster_enforcer_infected",
            creature(&[
                "progs/demon.mdl",
                "progs/enforcer.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_demon.mdl",
                "progs/h_mega.mdl",
                "progs/laser.mdl",
                "sound/demon/ddeath.wav",
                "sound/demon/dhit2.wav",
                "sound/demon/djump.wav",
                "sound/demon/dpain1.wav",
                "sound/demon/idle1.wav",
                "sound/demon/sight2.wav",
                "sound/enforcer/death1.wav",
                "sound/enforcer/enfire.wav",
                "sound/enforcer/enfstop.wav",
                "sound/enforcer/idle1.wav",
                "sound/enforcer/pain1.wav",
                "sound/enforcer/pain2.wav",
                "sound/enforcer/sight1.wav",
                "sound/enforcer/sight2.wav",
                "sound/enforcer/sight3.wav",
                "sound/enforcer/sight4.wav",
                "sound/player/udeath.wav",
            ]),
        ),
        (
            "monster_hell_knight_infected",
            creature(&[
                "progs/demon.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_demon.mdl",
                "progs/h_hellkn.mdl",
                "progs/hknight.mdl",
                "progs/k_spike.mdl",
                "sound/demon/ddeath.wav",
                "sound/demon/dhit2.wav",
                "sound/demon/djump.wav",
                "sound/demon/dpain1.wav",
                "sound/demon/idle1.wav",
                "sound/demon/sight2.wav",
                "sound/hknight/attack1.wav",
                "sound/hknight/death1.wav",
                "sound/hknight/grunt.wav",
                "sound/hknight/hit.wav",
                "sound/hknight/idle.wav",
                "sound/hknight/pain1.wav",
                "sound/hknight/sight1.wav",
                "sound/hknight/slash1.wav",
                "sound/infected/death1_rev.wav",
                "sound/knight/sword1.wav",
                "sound/knight/sword2.wav",
                "sound/player/udeath.wav",
            ]),
        ),
        (
            "monster_ranged_knight",
            creature(&[
                "progs/diamond_trail.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_hellkn.mdl",
                "progs/rknight.mdl",
                "sound/hknight/attack1.wav",
                "sound/hknight/hit.wav",
                "sound/hknight/slash1.wav",
                "sound/knight/sword1.wav",
                "sound/knight/sword2.wav",
                "sound/player/udeath.wav",
                "sound/rknight/death_01.wav",
                "sound/rknight/death_02.wav",
                "sound/rknight/idle_02.wav",
                "sound/rknight/idle_03.wav",
                "sound/rknight/idle_05.wav",
                "sound/rknight/pain_01.wav",
                "sound/rknight/pain_02.wav",
                "sound/rknight/pain_03.wav",
                "sound/rknight/sight_01.wav",
                "sound/rknight/sight_03.wav",
            ]),
        ),
        (
            "monster_super_shambler",
            creature(&[
                "progs/bolt.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_shams.mdl",
                "progs/k_spike.mdl",
                "progs/rogue/plasma.mdl",
                "progs/s_light.mdl",
                "progs/shambler_blood.mdl",
                "progs/zom_gib.mdl",
                "sound/hknight/attack1.wav",
                "sound/player/udeath.wav",
                "sound/shambler/melee1.wav",
                "sound/shambler/melee2.wav",
                "sound/shambler/sattck1.wav",
                "sound/shambler/sboom.wav",
                "sound/shambler/sdeath.wav",
                "sound/shambler/shurt2.wav",
                "sound/shambler/sidle.wav",
                "sound/shambler/smack.wav",
                "sound/shambler/ssight.wav",
                "sound/zombie/z_shot1.wav",
            ]),
        ),
        (
            "monster_lava_man",
            creature(&[
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/lavaball.mdl",
                "progs/lavaman.mdl",
                "sound/boss1/death.wav",
                "sound/boss1/out1.wav",
                "sound/boss1/pain.wav",
                "sound/boss1/sight1.wav",
                "sound/boss1/throw.wav",
                "sound/misc/power.wav",
                "sound/player/udeath.wav",
                "sound/weapons/rocket1i.wav",
            ]),
        ),
        (
            "monster_ghost",
            creature(&[
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "sound/player/udeath.wav",
            ]),
        ),
        (
            "monster_orb",
            creature(&[
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/rogue/sphere.mdl",
                "progs/teleporter_eye_blink.mdl",
                "sound/boss2/idle.wav",
                "sound/boss2/sight.wav",
                "sound/misc/power.wav",
                "sound/orb/orb_death.wav",
                "sound/orb/orb_pain.wav",
                "sound/player/udeath.wav",
                "sound/wizard/hit.wav",
                "sound/wizard/wattack.wav",
                "sound/wizard/wdeath.wav",
                "sound/wizard/wpain.wav",
            ]),
        ),
        (
            "monster_szombie",
            creature(&[
                "progs/flame2.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/h_zombie.mdl",
                "progs/zom_gib.mdl",
                "progs/zombie.mdl",
                "sound/player/udeath.wav",
                "sound/zombie/idle_w2.wav",
                "sound/zombie/z_fall.wav",
                "sound/zombie/z_gib.wav",
                "sound/zombie/z_hit.wav",
                "sound/zombie/z_idle.wav",
                "sound/zombie/z_idle1.wav",
                "sound/zombie/z_miss.wav",
                "sound/zombie/z_pain.wav",
                "sound/zombie/z_pain1.wav",
                "sound/zombie/z_shot1.wav",
            ]),
        ),
        (
            "monster_oldone_new",
            creature(&[
                "maps/bmodel/b_splash.bsp",
                "progs/diamond.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/oldone.mdl",
                "progs/rogue/plasma.mdl",
                "progs/rogue/sphere.mdl",
                "progs/s_light.mdl",
                "progs/teleport.mdl",
                "progs/teleporter_eye.mdl",
                "sound/boss2/death.wav",
                "sound/boss2/idle.wav",
                "sound/boss2/pop2.wav",
                "sound/boss2/sight.wav",
                "sound/misc/power.wav",
                "sound/orb/orb_death.wav",
                "sound/orb/orb_pain.wav",
                "sound/player/udeath.wav",
                "sound/weapons/grenade.wav",
                "sound/weapons/lhit.wav",
                "sound/weapons/lstart.wav",
                "sound/weapons/spike2.wav",
            ]),
        ),
        (
            "monster_boss_final",
            creature(&[
                "progs/boss.mdl",
                "progs/gib1.mdl",
                "progs/gib2.mdl",
                "progs/gib3.mdl",
                "progs/k_spike.mdl",
                "progs/lavaball.mdl",
                "progs/rogue/plasma.mdl",
                "progs/rogue/rubble.mdl",
                "progs/rogue/sphere.mdl",
                "progs/teleport.mdl",
                "sound/boss1/death.wav",
                "sound/boss1/out1.wav",
                "sound/boss1/pain.wav",
                "sound/boss1/sight1.wav",
                "sound/boss1/throw.wav",
                "sound/boss2/pop2.wav",
                "sound/hknight/attack1.wav",
                "sound/misc/power.wav",
                "sound/player/udeath.wav",
                "sound/weapons/rocket1i.wav",
            ]),
        ),
    ])
}

// `expansions.ts`.

/// Hipnotic scourge (`monster_scourge`).
fn expansion_scourge() -> MonsterCreature {
    creature(&[
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/h_scourg.mdl",
        "progs/scor.mdl",
        "progs/spike.mdl",
        "sound/misc/null.wav",
        "sound/player/udeath.wav",
        "sound/scourge/idle.wav",
        "sound/scourge/pain.wav",
        "sound/scourge/pain2.wav",
        "sound/scourge/sight.wav",
        "sound/scourge/walk.wav",
        "sound/shambler/smack.wav",
        "sound/weapons/rocket1i.wav",
    ])
}

/// Hipnotic gremlin (`monster_gremlin`).
fn expansion_gremlin() -> MonsterCreature {
    creature(&[
        "progs/backpack.mdl",
        "progs/bolt.mdl",
        "progs/bolt2.mdl",
        "progs/bolt3.mdl",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/grem.mdl",
        "progs/grenade.mdl",
        "progs/h_grem.mdl",
        "progs/laser.mdl",
        "progs/missile.mdl",
        "progs/s_explod.spr",
        "progs/spike.mdl",
        "progs/zom_gib.mdl",
        "sound/demon/dhit2.wav",
        "sound/grem/attack.wav",
        "sound/grem/death.wav",
        "sound/grem/sight1.wav",
        "sound/items/protect3.wav",
        "sound/player/udeath.wav",
        "sound/weapons/bounce.wav",
        "sound/weapons/grenade.wav",
        "sound/weapons/guncock.wav",
        "sound/weapons/lstart.wav",
        "sound/weapons/r_exp3.wav",
        "sound/weapons/ric1.wav",
        "sound/weapons/ric2.wav",
        "sound/weapons/ric3.wav",
        "sound/weapons/rocket1i.wav",
        "sound/weapons/sgun1.wav",
        "sound/weapons/shotgn2.wav",
        "sound/weapons/tink1.wav",
    ])
}

/// Hipnotic final boss (`monster_armagon`).
fn expansion_armagon() -> MonsterCreature {
    creature(&[
        "progs/armabody.mdl",
        "progs/armalegs.mdl",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/laser.mdl",
        "progs/missile.mdl",
        "progs/s_explod.spr",
        "sound/armagon/death.wav",
        "sound/armagon/footfall.wav",
        "sound/armagon/pain.wav",
        "sound/armagon/repel.wav",
        "sound/armagon/servo.wav",
        "sound/armagon/sight.wav",
        "sound/misc/longexpl.wav",
        "sound/player/udeath.wav",
        "sound/weapons/r_exp3.wav",
        "sound/weapons/sgun1.wav",
    ])
}

/// Rogue gulag eel (`monster_eel`).
fn expansion_eel() -> MonsterCreature {
    creature(&[
        "progs/eel2.mdl",
        "progs/eelgib.mdl",
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "sound/eel/eatt1.wav",
        "sound/eel/edie3r.wav",
        "sound/eel/eelc5.wav",
        "sound/eel/epain3.wav",
        "sound/player/udeath.wav",
    ])
}

/// Rogue flying sword (`monster_sword`).
fn expansion_sword() -> MonsterCreature {
    creature(&[
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/sword.mdl",
        "sound/knight/ksight.wav",
        "sound/knight/sword1.wav",
        "sound/player/axhit2.wav",
        "sound/player/udeath.wav",
    ])
}

/// Rogue wrath (`monster_wrath`).
fn expansion_wrath() -> MonsterCreature {
    creature(&[
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/s_explod.spr",
        "progs/w_ball.mdl",
        "progs/wrath.mdl",
        "progs/wrthgib1.mdl",
        "progs/wrthgib2.mdl",
        "progs/wrthgib3.mdl",
        "sound/player/udeath.wav",
        "sound/weapons/r_exp3.wav",
        "sound/wrath/watt.wav",
        "sound/wrath/wpain.wav",
        "sound/wrath/wsee.wav",
    ])
}

/// Rogue mummy (`monster_mummy`).
fn expansion_mummy() -> MonsterCreature {
    creature(&[
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/h_zombie.mdl",
        "progs/mummy.mdl",
        "progs/zom_gib.mdl",
        "sound/player/udeath.wav",
        "sound/zombie/z_gib.wav",
        "sound/zombie/z_hit.wav",
        "sound/zombie/z_idle.wav",
        "sound/zombie/z_miss.wav",
        "sound/zombie/z_shot1.wav",
    ])
}

/// Rogue overlord (`monster_super_wrath`).
fn expansion_super_wrath() -> MonsterCreature {
    creature(&[
        "progs/gib1.mdl",
        "progs/gib2.mdl",
        "progs/gib3.mdl",
        "progs/s_explod.spr",
        "progs/s_wrath.mdl",
        "progs/s_wrtgb2.mdl",
        "progs/s_wrtgb3.mdl",
        "progs/w_ball.mdl",
        "progs/wrthgib1.mdl",
        "progs/wrthgib2.mdl",
        "progs/wrthgib3.mdl",
        "sound/player/udeath.wav",
        "sound/s_wrath/smash.wav",
        "sound/weapons/r_exp3.wav",
        "sound/wrath/watt.wav",
        "sound/wrath/wpain.wav",
        "sound/wrath/wsee.wav",
    ])
}

/// Rogue lava man (`monster_lava_man`).
fn expansion_lava_man() -> MonsterCreature {
    creature(&[
        "progs/lavaball.mdl",
        "progs/lavaman.mdl",
        "sound/boss1/out1.wav",
        "sound/boss1/throw.wav",
    ])
}

/// Quake Hipnotic plus rogue expansion sources
/// (`q1ExpansionMonsterSources`).
#[must_use]
pub fn q1_expansion_monster_sources() -> Vec<MonsterSourceDefinition> {
    q1_monster_sources()
        .into_iter()
        .flat_map(|base| {
            let edition = edition_name(base.edition).to_string();
            let mut hipnotic = base.creatures.clone();
            hipnotic.insert("monster_scourge".to_string(), expansion_scourge());
            hipnotic.insert("monster_gremlin".to_string(), expansion_gremlin());
            hipnotic.insert("monster_armagon".to_string(), expansion_armagon());
            let mut rogue = base.creatures.clone();
            rogue.insert("monster_eel".to_string(), expansion_eel());
            rogue.insert("monster_sword".to_string(), expansion_sword());
            rogue.insert("monster_wrath".to_string(), expansion_wrath());
            rogue.insert("monster_mummy".to_string(), expansion_mummy());
            rogue.insert("monster_super_wrath".to_string(), expansion_super_wrath());
            rogue.insert("monster_lava_man".to_string(), expansion_lava_man());
            vec![
                source(
                    &format!("q1:monsters/{edition}/hipnotic"),
                    MonsterFamily::Q1,
                    base.edition,
                    MonsterProgram::Hipnotic,
                    hipnotic,
                ),
                source(
                    &format!("q1:monsters/{edition}/rogue"),
                    MonsterFamily::Q1,
                    base.edition,
                    MonsterProgram::Rogue,
                    rogue,
                ),
            ]
        })
        .collect()
}

/// Quake rerelease addon sources (`q1AddonMonsterSources`).
///
/// Each Machinegames episode 3 creature unions every base resource with
/// its own precaches; the base table flattens in classname order so the
/// union stays deterministic.
#[must_use]
pub fn q1_addon_monster_sources() -> Vec<MonsterSourceDefinition> {
    q1_monster_sources()
        .into_iter()
        .filter(|base| base.edition == SourceEdition::Rerelease)
        .flat_map(|base| {
            let mut mg3 = base.creatures.clone();
            let mut ordered: Vec<(&String, &MonsterCreature)> = base.creatures.iter().collect();
            ordered.sort_by(|left, right| left.0.cmp(right.0));
            let flattened: Vec<String> = ordered
                .iter()
                .flat_map(|(_, entry)| entry.resources.iter().cloned())
                .collect();
            for (classname, definition) in mg3_monster_resources() {
                let mut resources = Vec::new();
                union_ordered(&mut resources, &flattened);
                union_ordered(&mut resources, &definition.resources);
                mg3.insert(classname, MonsterCreature { resources });
            }
            vec![
                source(
                    "q1:monsters/rerelease/dopa",
                    MonsterFamily::Q1,
                    base.edition,
                    MonsterProgram::Dopa,
                    base.creatures.clone(),
                ),
                source(
                    "q1:monsters/rerelease/mg1",
                    MonsterFamily::Q1,
                    base.edition,
                    MonsterProgram::Mg1,
                    base.creatures.clone(),
                ),
                source(
                    "q1:monsters/rerelease/mg3",
                    MonsterFamily::Q1,
                    base.edition,
                    MonsterProgram::Mg3,
                    mg3,
                ),
            ]
        })
        .collect()
}

// `q2.ts`.

/// Shared Quake II environment callbacks (`environmentResources`).
fn q2_environment_resources() -> Vec<String> {
    [
        "sound/infantry/inflies1.wav",
        "sound/misc/fhit3.wav",
        "sound/player/watr_in.wav",
        "sound/player/watr_out.wav",
        "sound/player/lava1.wav",
        "sound/player/lava2.wav",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// Append the shared environment callbacks to a built creature.
fn append_environment(mut entry: MonsterCreature) -> MonsterCreature {
    entry.resources.extend(q2_environment_resources());
    entry
}

/// Classic berserker without environment callbacks (`classicBerserk`).
fn q2_classic_berserk() -> MonsterCreature {
    creature(&[
        "models/monsters/berserk/tris.md2",
        "models/monsters/berserk/skin.pcx",
        "models/monsters/berserk/pain.pcx",
        "models/objects/gibs/bone/tris.md2",
        "models/objects/gibs/sm_meat/tris.md2",
        "models/objects/gibs/head2/tris.md2",
        "models/objects/gibs/bone/skin.pcx",
        "models/objects/gibs/sm_meat/skin.pcx",
        "models/objects/gibs/head2/skin.pcx",
        "models/objects/gibs/head2/player.pcx",
        "sound/berserk/sight.wav",
        "sound/berserk/bersrch1.wav",
        "sound/berserk/berpain2.wav",
        "sound/berserk/berdeth2.wav",
        "sound/berserk/beridle1.wav",
        "sound/berserk/attack.wav",
        "sound/misc/udeath.wav",
    ])
}

/// Infantry resources shared by both editions (`infantryResources`).
fn q2_infantry_resources() -> Vec<String> {
    [
        "models/objects/smoke/tris.md2",
        "models/objects/smoke/skin.pcx",
        "models/objects/flash/tris.md2",
        "models/objects/flash/skin.pcx",
        "models/monsters/infantry/tris.md2",
        "models/monsters/infantry/skin.pcx",
        "models/monsters/infantry/pain.pcx",
        "models/objects/gibs/bone/tris.md2",
        "models/objects/gibs/sm_meat/tris.md2",
        "models/objects/gibs/bone/skin.pcx",
        "models/objects/gibs/sm_meat/skin.pcx",
        "sound/infantry/infsght1.wav",
        "sound/infantry/infsrch1.wav",
        "sound/infantry/infpain1.wav",
        "sound/infantry/infpain2.wav",
        "sound/infantry/infdeth1.wav",
        "sound/infantry/infdeth2.wav",
        "sound/infantry/infatck1.wav",
        "sound/infantry/infatck2.wav",
        "sound/infantry/infatck3.wav",
        "sound/infantry/infidle1.wav",
        "sound/infantry/melee2.wav",
        "sound/misc/udeath.wav",
        "sound/misc/fhit3.wav",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// Classic infantry without environment callbacks (`classicInfantry`).
fn q2_classic_infantry() -> MonsterCreature {
    let mut resources = q2_infantry_resources();
    resources.extend(
        [
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/player.pcx",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease infantry without environment callbacks (`rereleaseInfantry`).
fn q2_rerelease_infantry() -> MonsterCreature {
    let mut resources = q2_infantry_resources();
    resources.extend(
        [
            "models/monsters/infantry/gibs/arm.pcx",
            "models/monsters/infantry/gibs/chest.pcx",
            "models/monsters/infantry/gibs/foot.pcx",
            "models/monsters/infantry/gibs/head.pcx",
            "models/monsters/infantry/gibs/gun.pcx",
            "models/monsters/infantry/gibs/arm.md2",
            "models/monsters/infantry/gibs/chest.md2",
            "models/monsters/infantry/gibs/foot.md2",
            "models/monsters/infantry/gibs/gun.md2",
            "models/monsters/infantry/gibs/head.md2",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic enlisted (`soldier`).
fn q2_soldier() -> MonsterCreature {
    let mut resources = [
        "models/objects/smoke/tris.md2",
        "models/objects/smoke/skin.pcx",
        "models/objects/flash/tris.md2",
        "models/objects/flash/skin.pcx",
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/soldier/pain.pcx",
            "models/monsters/soldier/skin.pcx",
            "models/monsters/soldier/skin_lt.pcx",
            "models/monsters/soldier/skin_ltp.pcx",
            "models/monsters/soldier/skin_ss.pcx",
            "models/monsters/soldier/skin_ssp.pcx",
            "models/monsters/soldier/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/chest/skin.pcx",
            "models/objects/gibs/chest/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/tris.md2",
            "sound/infantry/infatck3.wav",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
            "sound/soldier/solatck1.wav",
            "sound/soldier/solatck2.wav",
            "sound/soldier/solatck3.wav",
            "sound/soldier/soldeth1.wav",
            "sound/soldier/soldeth2.wav",
            "sound/soldier/soldeth3.wav",
            "sound/soldier/solidle1.wav",
            "sound/soldier/solpain1.wav",
            "sound/soldier/solpain2.wav",
            "sound/soldier/solpain3.wav",
            "sound/soldier/solsght1.wav",
            "sound/soldier/solsrch1.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic gladiator (`gladiator`).
fn q2_gladiator() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/gladiatr/pain.pcx",
            "models/monsters/gladiatr/skin.pcx",
            "models/monsters/gladiatr/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/gladiator/glddeth2.wav",
            "sound/gladiator/gldidle1.wav",
            "sound/gladiator/gldpain2.wav",
            "sound/gladiator/gldsrch1.wav",
            "sound/gladiator/melee1.wav",
            "sound/gladiator/melee2.wav",
            "sound/gladiator/melee3.wav",
            "sound/gladiator/pain.wav",
            "sound/gladiator/railgun.wav",
            "sound/gladiator/sight.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic gunner (`gunner`).
fn q2_gunner() -> MonsterCreature {
    let mut resources = [
        "models/objects/smoke/tris.md2",
        "models/objects/smoke/skin.pcx",
        "models/objects/flash/tris.md2",
        "models/objects/flash/skin.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/gunner/pain.pcx",
            "models/monsters/gunner/skin.pcx",
            "models/monsters/gunner/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/grenade/skin.pcx",
            "models/objects/grenade/tris.md2",
            "sound/gunner/death1.wav",
            "sound/gunner/gunatck1.wav",
            "sound/gunner/gunatck2.wav",
            "sound/gunner/gunatck3.wav",
            "sound/gunner/gunidle1.wav",
            "sound/gunner/gunpain1.wav",
            "sound/gunner/gunpain2.wav",
            "sound/gunner/gunsrch1.wav",
            "sound/gunner/sight1.wav",
            "sound/misc/udeath.wav",
            "sound/weapons/grenlb1b.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic parasite (`parasite`).
fn q2_parasite() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/parasite/pain.pcx",
            "models/monsters/parasite/skin.pcx",
            "models/monsters/parasite/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/misc/udeath.wav",
            "sound/parasite/paratck1.wav",
            "sound/parasite/paratck2.wav",
            "sound/parasite/paratck3.wav",
            "sound/parasite/paratck4.wav",
            "sound/parasite/pardeth1.wav",
            "sound/parasite/paridle1.wav",
            "sound/parasite/paridle2.wav",
            "sound/parasite/parpain1.wav",
            "sound/parasite/parpain2.wav",
            "sound/parasite/parsght1.wav",
            "sound/parasite/parsrch1.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic flyer (`flyer`).
fn q2_flyer() -> MonsterCreature {
    let mut resources = [
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/flyer/pain.pcx",
            "models/monsters/flyer/skin.pcx",
            "models/monsters/flyer/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/tris.md2",
            "sound/flyer/flyatck1.wav",
            "sound/flyer/flyatck2.wav",
            "sound/flyer/flyatck3.wav",
            "sound/flyer/flydeth1.wav",
            "sound/flyer/flyidle1.wav",
            "sound/flyer/flypain1.wav",
            "sound/flyer/flypain2.wav",
            "sound/flyer/flysght1.wav",
            "sound/flyer/flysrch1.wav",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic floater (`floater`).
fn q2_floater() -> MonsterCreature {
    let mut resources = [
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/float/pain.pcx",
            "models/monsters/float/skin.pcx",
            "models/monsters/float/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/tris.md2",
            "sound/floater/fltatck1.wav",
            "sound/floater/fltatck2.wav",
            "sound/floater/fltatck3.wav",
            "sound/floater/fltdeth1.wav",
            "sound/floater/fltidle1.wav",
            "sound/floater/fltpain1.wav",
            "sound/floater/fltpain2.wav",
            "sound/floater/fltsght1.wav",
            "sound/floater/fltsrch1.wav",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic hover (`hover`).
fn q2_hover() -> MonsterCreature {
    let mut resources = ["models/objects/explode/tris.md2", "models/objects/explode/skin.pcx"]
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/hover/pain.pcx",
            "models/monsters/hover/skin.pcx",
            "models/monsters/hover/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/tris.md2",
            "sound/hover/hovatck1.wav",
            "sound/hover/hovdeth1.wav",
            "sound/hover/hovdeth2.wav",
            "sound/hover/hovidle1.wav",
            "sound/hover/hovpain1.wav",
            "sound/hover/hovpain2.wav",
            "sound/hover/hovsght1.wav",
            "sound/hover/hovsrch1.wav",
            "sound/hover/hovsrch2.wav",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic mutant (`mutant`).
fn q2_mutant() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/mutant/pain.pcx",
            "models/monsters/mutant/skin.pcx",
            "models/monsters/mutant/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/misc/udeath.wav",
            "sound/mutant/mutatck1.wav",
            "sound/mutant/mutatck2.wav",
            "sound/mutant/mutatck3.wav",
            "sound/mutant/mutdeth1.wav",
            "sound/mutant/mutidle1.wav",
            "sound/mutant/mutpain1.wav",
            "sound/mutant/mutpain2.wav",
            "sound/mutant/mutsght1.wav",
            "sound/mutant/mutsrch1.wav",
            "sound/mutant/step1.wav",
            "sound/mutant/step2.wav",
            "sound/mutant/step3.wav",
            "sound/mutant/thud1.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic iron maiden (`chick`).
fn q2_chick() -> MonsterCreature {
    let mut resources = [
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/bitch/pain.pcx",
            "models/monsters/bitch/skin.pcx",
            "models/monsters/bitch/tris.md2",
            "models/objects/debris2/skin.pcx",
            "models/objects/debris2/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/rocket/skin.pcx",
            "models/objects/rocket/tris.md2",
            "sound/chick/chkatck1.wav",
            "sound/chick/chkatck2.wav",
            "sound/chick/chkatck3.wav",
            "sound/chick/chkatck4.wav",
            "sound/chick/chkatck5.wav",
            "sound/chick/chkdeth1.wav",
            "sound/chick/chkdeth2.wav",
            "sound/chick/chkfall1.wav",
            "sound/chick/chkidle1.wav",
            "sound/chick/chkidle2.wav",
            "sound/chick/chkpain1.wav",
            "sound/chick/chkpain2.wav",
            "sound/chick/chkpain3.wav",
            "sound/chick/chksght1.wav",
            "sound/chick/chksrch1.wav",
            "sound/misc/udeath.wav",
            "sound/weapons/rockfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic tank (`tank`).
fn q2_tank() -> MonsterCreature {
    let mut resources = [
        "models/objects/smoke/tris.md2",
        "models/objects/smoke/skin.pcx",
        "models/objects/flash/tris.md2",
        "models/objects/flash/skin.pcx",
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/ctank/pain.pcx",
            "models/monsters/ctank/skin.pcx",
            "models/monsters/tank/pain.pcx",
            "models/monsters/tank/skin.pcx",
            "models/monsters/tank/tris.md2",
            "models/objects/debris2/skin.pcx",
            "models/objects/debris2/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/chest/skin.pcx",
            "models/objects/gibs/chest/tris.md2",
            "models/objects/gibs/gear/skin.pcx",
            "models/objects/gibs/gear/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_metal/skin.pcx",
            "models/objects/gibs/sm_metal/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/tris.md2",
            "models/objects/rocket/skin.pcx",
            "models/objects/rocket/tris.md2",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
            "sound/tank/death.wav",
            "sound/tank/sight1.wav",
            "sound/tank/step.wav",
            "sound/tank/tnkatck1.wav",
            "sound/tank/tnkatck3.wav",
            "sound/tank/tnkatck4.wav",
            "sound/tank/tnkatck5.wav",
            "sound/tank/tnkatk2a.wav",
            "sound/tank/tnkatk2b.wav",
            "sound/tank/tnkatk2c.wav",
            "sound/tank/tnkatk2d.wav",
            "sound/tank/tnkatk2e.wav",
            "sound/tank/tnkdeth2.wav",
            "sound/tank/tnkidle1.wav",
            "sound/tank/tnkpain2.wav",
            "sound/weapons/rockfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic flipper (`flipper`).
fn q2_flipper() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/flipper/tris.md2",
            "models/monsters/flipper/skin.pcx",
            "models/monsters/flipper/pain.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "sound/flipper/flpsght1.wav",
            "sound/flipper/flppain1.wav",
            "sound/flipper/flppain2.wav",
            "sound/flipper/flpdeth1.wav",
            "sound/flipper/flpatck1.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Classic brain (`brain`).
fn q2_brain() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/brain/pain.pcx",
            "models/monsters/brain/skin.pcx",
            "models/monsters/brain/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/brain/brnatck1.wav",
            "sound/brain/brnatck3.wav",
            "sound/brain/brndeth1.wav",
            "sound/brain/brnlens1.wav",
            "sound/brain/brnpain1.wav",
            "sound/brain/brnpain2.wav",
            "sound/brain/brnsght1.wav",
            "sound/brain/brnsrch1.wav",
            "sound/brain/melee1.wav",
            "sound/brain/melee2.wav",
            "sound/brain/melee3.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Quake II base-game monster sources (`q2MonsterSources`).
#[must_use]
pub fn q2_monster_sources() -> Vec<MonsterSourceDefinition> {
    let classic_infantry = append_environment(q2_classic_infantry());
    let classic_berserk = append_environment(q2_classic_berserk());
    let rerelease_infantry = append_environment(q2_rerelease_infantry());
    let mut rerelease = q2_rerelease_creatures();
    rerelease.insert("monster_infantry".to_string(), rerelease_infantry);
    vec![
        source(
            "q2:monsters/classic/baseq2",
            MonsterFamily::Q2,
            SourceEdition::Classic,
            MonsterProgram::Baseq2,
            catalog(vec![
                ("monster_brain", q2_brain()),
                ("monster_flipper", q2_flipper()),
                ("monster_infantry", classic_infantry),
                ("monster_berserk", classic_berserk),
                ("monster_soldier", q2_soldier()),
                ("monster_gladiator", q2_gladiator()),
                ("monster_gunner", q2_gunner()),
                ("monster_parasite", q2_parasite()),
                ("monster_flyer", q2_flyer()),
                ("monster_floater", q2_floater()),
                ("monster_hover", q2_hover()),
                ("monster_mutant", q2_mutant()),
                ("monster_chick", q2_chick()),
                ("monster_tank", q2_tank()),
                ("monster_soldier_light", q2_soldier()),
                ("monster_soldier_ss", q2_soldier()),
                ("monster_tank_commander", q2_tank()),
            ]),
        ),
        source(
            "q2:monsters/rerelease/baseq2",
            MonsterFamily::Q2,
            SourceEdition::Rerelease,
            MonsterProgram::Baseq2,
            rerelease,
        ),
    ]
}

// `q2-rerelease.ts`.

/// Rerelease enlisted (`soldier`).
fn q2r_soldier() -> MonsterCreature {
    let mut resources = [
        "sound/soldier/solatck1.wav",
        "sound/soldier/solatck2.wav",
        "sound/soldier/solatck3.wav",
        "models/objects/smoke/tris.md2",
        "models/objects/smoke/skin.pcx",
        "models/objects/flash/tris.md2",
        "models/objects/flash/skin.pcx",
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/explode/rskin.pcx",
        "models/objects/explode/skin2.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/soldier/gibs/arm.md2",
            "models/monsters/soldier/gibs/arm.pcx",
            "models/monsters/soldier/gibs/arm_lt.pcx",
            "models/monsters/soldier/gibs/arm_sold01.pcx",
            "models/monsters/soldier/gibs/arm_sold02.pcx",
            "models/monsters/soldier/gibs/arm_sold03.pcx",
            "models/monsters/soldier/gibs/arm_ss.pcx",
            "models/monsters/soldier/gibs/chest.md2",
            "models/monsters/soldier/gibs/chest.pcx",
            "models/monsters/soldier/gibs/chest_lt.pcx",
            "models/monsters/soldier/gibs/chest_sold01.pcx",
            "models/monsters/soldier/gibs/chest_sold02.pcx",
            "models/monsters/soldier/gibs/chest_sold03.pcx",
            "models/monsters/soldier/gibs/chest_ss.pcx",
            "models/monsters/soldier/gibs/gun.md2",
            "models/monsters/soldier/gibs/gun.pcx",
            "models/monsters/soldier/gibs/gun_lt.pcx",
            "models/monsters/soldier/gibs/gun_sold01.pcx",
            "models/monsters/soldier/gibs/gun_sold02.pcx",
            "models/monsters/soldier/gibs/gun_sold03.pcx",
            "models/monsters/soldier/gibs/gun_ss.pcx",
            "models/monsters/soldier/gibs/head.md2",
            "models/monsters/soldier/gibs/head.pcx",
            "models/monsters/soldier/gibs/head_lt.pcx",
            "models/monsters/soldier/gibs/head_sold01.pcx",
            "models/monsters/soldier/gibs/head_sold02.pcx",
            "models/monsters/soldier/gibs/head_sold03.pcx",
            "models/monsters/soldier/gibs/head_ss.pcx",
            "models/monsters/soldier/pain.pcx",
            "models/monsters/soldier/skin.pcx",
            "models/monsters/soldier/skin_lt.pcx",
            "models/monsters/soldier/skin_ltp.pcx",
            "models/monsters/soldier/skin_ss.pcx",
            "models/monsters/soldier/skin_ssp.pcx",
            "models/monsters/soldier/sold01.pcx",
            "models/monsters/soldier/sold01_p.pcx",
            "models/monsters/soldier/sold02.pcx",
            "models/monsters/soldier/sold02_p.pcx",
            "models/monsters/soldier/sold03.pcx",
            "models/monsters/soldier/sold03_p.pcx",
            "models/monsters/soldier/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/bone2/skin.pcx",
            "models/objects/gibs/bone2/tris.md2",
            "models/objects/gibs/head2/player.pcx",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/skinb.pcx",
            "models/objects/laser/sking.pcx",
            "models/objects/laser/tris.md2",
            "sound/infantry/infatck3.wav",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
            "sound/soldier/soldeth1.wav",
            "sound/soldier/soldeth2.wav",
            "sound/soldier/soldeth3.wav",
            "sound/soldier/solidle1.wav",
            "sound/soldier/solpain1.wav",
            "sound/soldier/solpain2.wav",
            "sound/soldier/solpain3.wav",
            "sound/soldier/solsght1.wav",
            "sound/soldier/solsrch1.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease berserker (`berserk`).
fn q2r_berserk() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/berserk/gibs/chest.md2",
            "models/monsters/berserk/gibs/chest.pcx",
            "models/monsters/berserk/gibs/hammer.md2",
            "models/monsters/berserk/gibs/hammer.pcx",
            "models/monsters/berserk/gibs/head.md2",
            "models/monsters/berserk/gibs/head.pcx",
            "models/monsters/berserk/gibs/thigh.md2",
            "models/monsters/berserk/gibs/thigh.pcx",
            "models/monsters/berserk/pain.pcx",
            "models/monsters/berserk/skin.pcx",
            "models/monsters/berserk/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/gear/skin.pcx",
            "models/objects/gibs/gear/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/berserk/attack.wav",
            "sound/berserk/berdeth2.wav",
            "sound/berserk/beridle1.wav",
            "sound/berserk/berpain2.wav",
            "sound/berserk/bersrch1.wav",
            "sound/berserk/jump.wav",
            "sound/berserk/sight.wav",
            "sound/misc/udeath.wav",
            "sound/mutant/thud1.wav",
            "sound/world/explod2.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease gunner (`gunner`).
fn q2r_gunner() -> MonsterCreature {
    let mut resources = [
        "sound/gunner/gunatck2.wav",
        "sound/gunner/gunatck3.wav",
        "models/objects/smoke/tris.md2",
        "models/objects/smoke/skin.pcx",
        "models/objects/flash/tris.md2",
        "models/objects/flash/skin.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/gunner/cpain.pcx",
            "models/monsters/gunner/cskin.pcx",
            "models/monsters/gunner/gibs/cchest.pcx",
            "models/monsters/gunner/gibs/cfoot.pcx",
            "models/monsters/gunner/gibs/cgarm.pcx",
            "models/monsters/gunner/gibs/cgun.pcx",
            "models/monsters/gunner/gibs/chead.pcx",
            "models/monsters/gunner/gibs/chest.md2",
            "models/monsters/gunner/gibs/chest.pcx",
            "models/monsters/gunner/gibs/foot.md2",
            "models/monsters/gunner/gibs/foot.pcx",
            "models/monsters/gunner/gibs/garm.md2",
            "models/monsters/gunner/gibs/garm.pcx",
            "models/monsters/gunner/gibs/gun.md2",
            "models/monsters/gunner/gibs/gun.pcx",
            "models/monsters/gunner/gibs/head.md2",
            "models/monsters/gunner/gibs/head.pcx",
            "models/monsters/gunner/pain.pcx",
            "models/monsters/gunner/skin.pcx",
            "models/monsters/gunner/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/grenade/skin.pcx",
            "models/objects/grenade/tris.md2",
            "sound/gunner/death1.wav",
            "sound/gunner/gunatck1.wav",
            "sound/gunner/gunidle1.wav",
            "sound/gunner/gunpain1.wav",
            "sound/gunner/gunpain2.wav",
            "sound/gunner/gunsrch1.wav",
            "sound/gunner/sight1.wav",
            "sound/misc/udeath.wav",
            "sound/weapons/grenlb1b.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease floater (`floater`).
fn q2r_floater() -> MonsterCreature {
    let mut resources = [
        "sound/floater/fltatck1.wav",
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/explode/rskin.pcx",
        "models/objects/explode/skin2.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/float/gibs/base.md2",
            "models/monsters/float/gibs/base.pcx",
            "models/monsters/float/gibs/gun.md2",
            "models/monsters/float/gibs/gun.pcx",
            "models/monsters/float/gibs/jar.md2",
            "models/monsters/float/gibs/jar.pcx",
            "models/monsters/float/gibs/piece.md2",
            "models/monsters/float/gibs/piece.pcx",
            "models/monsters/float/pain.pcx",
            "models/monsters/float/skin.pcx",
            "models/monsters/float/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_metal/skin.pcx",
            "models/objects/gibs/sm_metal/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/skinb.pcx",
            "models/objects/laser/sking.pcx",
            "models/objects/laser/tris.md2",
            "sound/floater/fltatck2.wav",
            "sound/floater/fltatck3.wav",
            "sound/floater/fltdeth1.wav",
            "sound/floater/fltidle1.wav",
            "sound/floater/fltpain1.wav",
            "sound/floater/fltpain2.wav",
            "sound/floater/fltsght1.wav",
            "sound/floater/fltsrch1.wav",
            "sound/misc/lasfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease hover (`hover`).
fn q2r_hover() -> MonsterCreature {
    let mut resources = [
        "sound/weapons/rocklx1a.wav",
        "sound/hover/hovatck1.wav",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/explode/rskin.pcx",
        "models/objects/explode/skin2.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/hover/gibs/chest.md2",
            "models/monsters/hover/gibs/chest.pcx",
            "models/monsters/hover/gibs/chestr.pcx",
            "models/monsters/hover/gibs/foot.md2",
            "models/monsters/hover/gibs/foot.pcx",
            "models/monsters/hover/gibs/footr.pcx",
            "models/monsters/hover/gibs/head.md2",
            "models/monsters/hover/gibs/head.pcx",
            "models/monsters/hover/gibs/headr.pcx",
            "models/monsters/hover/gibs/ring.md2",
            "models/monsters/hover/gibs/ring.pcx",
            "models/monsters/hover/gibs/ringr.pcx",
            "models/monsters/hover/pain.pcx",
            "models/monsters/hover/rpain.pcx",
            "models/monsters/hover/rskin.pcx",
            "models/monsters/hover/skin.pcx",
            "models/monsters/hover/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_metal/skin.pcx",
            "models/objects/gibs/sm_metal/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/skinb.pcx",
            "models/objects/laser/sking.pcx",
            "models/objects/laser/tris.md2",
            "sound/hover/hovdeth1.wav",
            "sound/hover/hovdeth2.wav",
            "sound/hover/hovidle1.wav",
            "sound/hover/hovpain1.wav",
            "sound/hover/hovpain2.wav",
            "sound/hover/hovsght1.wav",
            "sound/hover/hovsrch1.wav",
            "sound/hover/hovsrch2.wav",
            "sound/misc/lasfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease flyer (`flyer`).
fn q2r_flyer() -> MonsterCreature {
    let mut resources = [
        "sound/flyer/flyatck3.wav",
        "models/objects/explode/tris.md2",
        "models/objects/explode/skin.pcx",
        "models/objects/explode/rskin.pcx",
        "models/objects/explode/skin2.pcx",
        "models/objects/r_explode/tris.md2",
        "models/objects/r_explode/skin1.pcx",
        "models/objects/r_explode/skin2.pcx",
        "models/objects/r_explode/skin3.pcx",
        "models/objects/r_explode/skin4.pcx",
        "models/objects/r_explode/skin5.pcx",
        "models/objects/r_explode/skin6.pcx",
        "models/objects/r_explode/skin7.pcx",
    ]
    .iter()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    resources.extend(q2_environment_resources());
    resources.extend(
        [
            "models/monsters/flyer/gibs/base.md2",
            "models/monsters/flyer/gibs/base.pcx",
            "models/monsters/flyer/gibs/gun.md2",
            "models/monsters/flyer/gibs/gun.pcx",
            "models/monsters/flyer/gibs/head.md2",
            "models/monsters/flyer/gibs/head.pcx",
            "models/monsters/flyer/gibs/wing.md2",
            "models/monsters/flyer/gibs/wing.pcx",
            "models/monsters/flyer/pain.pcx",
            "models/monsters/flyer/skin.pcx",
            "models/monsters/flyer/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_metal/skin.pcx",
            "models/objects/gibs/sm_metal/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/skinb.pcx",
            "models/objects/laser/sking.pcx",
            "models/objects/laser/tris.md2",
            "sound/flyer/flyatck1.wav",
            "sound/flyer/flyatck2.wav",
            "sound/flyer/flydeth1.wav",
            "sound/flyer/flyidle1.wav",
            "sound/flyer/flypain1.wav",
            "sound/flyer/flypain2.wav",
            "sound/flyer/flysght1.wav",
            "sound/flyer/flysrch1.wav",
            "sound/misc/lasfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease mutant (`mutant`).
fn q2r_mutant() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/mutant/gibs/chest.md2",
            "models/monsters/mutant/gibs/chest.pcx",
            "models/monsters/mutant/gibs/foot.md2",
            "models/monsters/mutant/gibs/foot.pcx",
            "models/monsters/mutant/gibs/hand.md2",
            "models/monsters/mutant/gibs/hand.pcx",
            "models/monsters/mutant/gibs/head.md2",
            "models/monsters/mutant/gibs/head.pcx",
            "models/monsters/mutant/pain.pcx",
            "models/monsters/mutant/skin.pcx",
            "models/monsters/mutant/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/misc/udeath.wav",
            "sound/mutant/mutatck1.wav",
            "sound/mutant/mutatck2.wav",
            "sound/mutant/mutatck3.wav",
            "sound/mutant/mutdeth1.wav",
            "sound/mutant/mutidle1.wav",
            "sound/mutant/mutpain1.wav",
            "sound/mutant/mutpain2.wav",
            "sound/mutant/mutsght1.wav",
            "sound/mutant/mutsrch1.wav",
            "sound/mutant/step1.wav",
            "sound/mutant/step2.wav",
            "sound/mutant/step3.wav",
            "sound/mutant/thud1.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease parasite (`parasite`).
fn q2r_parasite() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/parasite/gibs/bleg.md2",
            "models/monsters/parasite/gibs/bleg.pcx",
            "models/monsters/parasite/gibs/chest.md2",
            "models/monsters/parasite/gibs/chest.pcx",
            "models/monsters/parasite/gibs/fleg.md2",
            "models/monsters/parasite/gibs/fleg.pcx",
            "models/monsters/parasite/gibs/head.md2",
            "models/monsters/parasite/gibs/head.pcx",
            "models/monsters/parasite/pain.pcx",
            "models/monsters/parasite/segment/skin.pcx",
            "models/monsters/parasite/segment/tris.md2",
            "models/monsters/parasite/skin.pcx",
            "models/monsters/parasite/tip/base.pcx",
            "models/monsters/parasite/tip/tris.md2",
            "models/monsters/parasite/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/misc/udeath.wav",
            "sound/parasite/paratck1.wav",
            "sound/parasite/paratck2.wav",
            "sound/parasite/paratck3.wav",
            "sound/parasite/paratck4.wav",
            "sound/parasite/pardeth1.wav",
            "sound/parasite/paridle1.wav",
            "sound/parasite/paridle2.wav",
            "sound/parasite/parpain1.wav",
            "sound/parasite/parpain2.wav",
            "sound/parasite/parsght1.wav",
            "sound/parasite/parsrch1.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease iron maiden (`chick`).
fn q2r_chick() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/bitch/bi_pain.pcx",
            "models/monsters/bitch/bi_sk3.pcx",
            "models/monsters/bitch/gibs/arm.md2",
            "models/monsters/bitch/gibs/arm.pcx",
            "models/monsters/bitch/gibs/arm_bi.pcx",
            "models/monsters/bitch/gibs/chest.md2",
            "models/monsters/bitch/gibs/chest.pcx",
            "models/monsters/bitch/gibs/chest_bi.pcx",
            "models/monsters/bitch/gibs/foot.md2",
            "models/monsters/bitch/gibs/foot.pcx",
            "models/monsters/bitch/gibs/foot_bi.pcx",
            "models/monsters/bitch/gibs/head.md2",
            "models/monsters/bitch/gibs/head.pcx",
            "models/monsters/bitch/gibs/head_bi.pcx",
            "models/monsters/bitch/gibs/tube.md2",
            "models/monsters/bitch/gibs/tube.pcx",
            "models/monsters/bitch/gibs/tube_bi.pcx",
            "models/monsters/bitch/pain.pcx",
            "models/monsters/bitch/skin.pcx",
            "models/monsters/bitch/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/r_explode/skin1.pcx",
            "models/objects/r_explode/skin2.pcx",
            "models/objects/r_explode/skin3.pcx",
            "models/objects/r_explode/skin4.pcx",
            "models/objects/r_explode/skin5.pcx",
            "models/objects/r_explode/skin6.pcx",
            "models/objects/r_explode/skin7.pcx",
            "models/objects/r_explode/tris.md2",
            "models/objects/rocket/skin.pcx",
            "models/objects/rocket/tris.md2",
            "sound/chick/chkatck1.wav",
            "sound/chick/chkatck2.wav",
            "sound/chick/chkatck3.wav",
            "sound/chick/chkatck5.wav",
            "sound/chick/chkdeth1.wav",
            "sound/chick/chkdeth2.wav",
            "sound/chick/chkidle1.wav",
            "sound/chick/chkidle2.wav",
            "sound/chick/chkpain1.wav",
            "sound/chick/chkpain2.wav",
            "sound/chick/chkpain3.wav",
            "sound/chick/chksght1.wav",
            "sound/misc/udeath.wav",
            "sound/weapons/rockfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease tank (`tank`).
fn q2r_tank() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/tank/cpain.pcx",
            "models/monsters/tank/cskin.pcx",
            "models/monsters/tank/gibs/barm.md2",
            "models/monsters/tank/gibs/barm.pcx",
            "models/monsters/tank/gibs/barm_c.pcx",
            "models/monsters/tank/gibs/chest.md2",
            "models/monsters/tank/gibs/chest.pcx",
            "models/monsters/tank/gibs/chest_c.pcx",
            "models/monsters/tank/gibs/foot.md2",
            "models/monsters/tank/gibs/foot.pcx",
            "models/monsters/tank/gibs/foot_c.pcx",
            "models/monsters/tank/gibs/head.md2",
            "models/monsters/tank/gibs/head.pcx",
            "models/monsters/tank/gibs/head_c.pcx",
            "models/monsters/tank/gibs/thigh.md2",
            "models/monsters/tank/gibs/thigh.pcx",
            "models/monsters/tank/gibs/thigh_c.pcx",
            "models/monsters/tank/pain.pcx",
            "models/monsters/tank/skin.pcx",
            "models/monsters/tank/tris.md2",
            "models/objects/explode/rskin.pcx",
            "models/objects/explode/skin.pcx",
            "models/objects/explode/skin2.pcx",
            "models/objects/explode/tris.md2",
            "models/objects/flash/skin.pcx",
            "models/objects/flash/tris.md2",
            "models/objects/gibs/chest/skin.pcx",
            "models/objects/gibs/chest/tris.md2",
            "models/objects/gibs/gear/skin.pcx",
            "models/objects/gibs/gear/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_metal/skin.pcx",
            "models/objects/gibs/sm_metal/tris.md2",
            "models/objects/laser/skin.pcx",
            "models/objects/laser/skinb.pcx",
            "models/objects/laser/sking.pcx",
            "models/objects/laser/tris.md2",
            "models/objects/r_explode/skin1.pcx",
            "models/objects/r_explode/skin2.pcx",
            "models/objects/r_explode/skin3.pcx",
            "models/objects/r_explode/skin4.pcx",
            "models/objects/r_explode/skin5.pcx",
            "models/objects/r_explode/skin6.pcx",
            "models/objects/r_explode/skin7.pcx",
            "models/objects/r_explode/tris.md2",
            "models/objects/rocket/skin.pcx",
            "models/objects/rocket/tris.md2",
            "models/objects/smoke/skin.pcx",
            "models/objects/smoke/tris.md2",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
            "sound/tank/death.wav",
            "sound/tank/pain.wav",
            "sound/tank/sight1.wav",
            "sound/tank/step.wav",
            "sound/tank/tnkatck1.wav",
            "sound/tank/tnkatck3.wav",
            "sound/tank/tnkatck4.wav",
            "sound/tank/tnkatck5.wav",
            "sound/tank/tnkatk2a.wav",
            "sound/tank/tnkatk2b.wav",
            "sound/tank/tnkatk2c.wav",
            "sound/tank/tnkatk2d.wav",
            "sound/tank/tnkatk2e.wav",
            "sound/tank/tnkdeth2.wav",
            "sound/tank/tnkidle1.wav",
            "sound/tank/tnkpain2.wav",
            "sound/weapons/rockfly.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease gladiator (`gladiator`).
fn q2r_gladiator() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/gladiatr/gibs/chest.md2",
            "models/monsters/gladiatr/gibs/chest.pcx",
            "models/monsters/gladiatr/gibs/chest2.pcx",
            "models/monsters/gladiatr/gibs/head.md2",
            "models/monsters/gladiatr/gibs/head.pcx",
            "models/monsters/gladiatr/gibs/head2.pcx",
            "models/monsters/gladiatr/gibs/larm.md2",
            "models/monsters/gladiatr/gibs/larm.pcx",
            "models/monsters/gladiatr/gibs/larm2.pcx",
            "models/monsters/gladiatr/gibs/rarm.md2",
            "models/monsters/gladiatr/gibs/rarm.pcx",
            "models/monsters/gladiatr/gibs/rarm2.pcx",
            "models/monsters/gladiatr/gibs/thigh.md2",
            "models/monsters/gladiatr/gibs/thigh.pcx",
            "models/monsters/gladiatr/gibs/thigh2.pcx",
            "models/monsters/gladiatr/pain.pcx",
            "models/monsters/gladiatr/pain2.pcx",
            "models/monsters/gladiatr/skin.pcx",
            "models/monsters/gladiatr/skin2.pcx",
            "models/monsters/gladiatr/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/gladiator/death.wav",
            "sound/gladiator/glddeth2.wav",
            "sound/gladiator/gldidle1.wav",
            "sound/gladiator/gldpain2.wav",
            "sound/gladiator/gldsrch1.wav",
            "sound/gladiator/melee1.wav",
            "sound/gladiator/melee2.wav",
            "sound/gladiator/melee3.wav",
            "sound/gladiator/pain.wav",
            "sound/gladiator/railgun.wav",
            "sound/gladiator/sight.wav",
            "sound/misc/udeath.wav",
            "sound/weapons/rg_hum.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease flipper (`flipper`).
fn q2r_flipper() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/flipper/tris.md2",
            "models/monsters/flipper/skin.pcx",
            "models/monsters/flipper/pain.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "sound/flipper/flpsght1.wav",
            "sound/flipper/flppain1.wav",
            "sound/flipper/flppain2.wav",
            "sound/flipper/flpdeth1.wav",
            "sound/flipper/flpatck1.wav",
            "sound/misc/udeath.wav",
            "models/objects/gibs/head2/tris.md2",
            "models/objects/gibs/head2/skin.pcx",
            "models/objects/gibs/head2/player.pcx",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease brain (`brain`).
fn q2r_brain() -> MonsterCreature {
    let mut resources = q2_environment_resources();
    resources.extend(
        [
            "models/monsters/brain/gibs/arm.md2",
            "models/monsters/brain/gibs/arm.pcx",
            "models/monsters/brain/gibs/boot.md2",
            "models/monsters/brain/gibs/boot.pcx",
            "models/monsters/brain/gibs/chest.md2",
            "models/monsters/brain/gibs/chest.pcx",
            "models/monsters/brain/gibs/door.md2",
            "models/monsters/brain/gibs/door.pcx",
            "models/monsters/brain/gibs/head.md2",
            "models/monsters/brain/gibs/head.pcx",
            "models/monsters/brain/gibs/pelvis.md2",
            "models/monsters/brain/gibs/pelvis.pcx",
            "models/monsters/brain/pain.pcx",
            "models/monsters/brain/skin.pcx",
            "models/monsters/brain/tris.md2",
            "models/monsters/parasite/segment/skin.pcx",
            "models/monsters/parasite/segment/tris.md2",
            "models/objects/gibs/bone/skin.pcx",
            "models/objects/gibs/bone/tris.md2",
            "models/objects/gibs/sm_meat/skin.pcx",
            "models/objects/gibs/sm_meat/tris.md2",
            "sound/brain/brnatck1.wav",
            "sound/brain/brnatck3.wav",
            "sound/brain/brndeth1.wav",
            "sound/brain/brnlens1.wav",
            "sound/brain/brnpain1.wav",
            "sound/brain/brnpain2.wav",
            "sound/brain/brnsght1.wav",
            "sound/brain/brnsrch1.wav",
            "sound/brain/melee1.wav",
            "sound/brain/melee2.wav",
            "sound/brain/melee3.wav",
            "sound/misc/lasfly.wav",
            "sound/misc/udeath.wav",
        ]
        .iter()
        .map(ToString::to_string),
    );
    MonsterCreature { resources }
}

/// Rerelease base-game creatures without infantry
/// (`q2RereleaseCreatures`; infantry assembles in
/// [`q2_monster_sources`]).
#[must_use]
pub fn q2_rerelease_creatures() -> HashMap<String, MonsterCreature> {
    catalog(vec![
        ("monster_brain", q2r_brain()),
        ("monster_flipper", q2r_flipper()),
        ("monster_soldier", q2r_soldier()),
        ("monster_soldier_light", q2r_soldier()),
        ("monster_soldier_ss", q2r_soldier()),
        ("monster_berserk", q2r_berserk()),
        ("monster_gunner", q2r_gunner()),
        ("monster_floater", q2r_floater()),
        ("monster_hover", q2r_hover()),
        ("monster_flyer", q2r_flyer()),
        ("monster_mutant", q2r_mutant()),
        ("monster_parasite", q2r_parasite()),
        ("monster_chick", q2r_chick()),
        ("monster_tank", q2r_tank()),
        ("monster_tank_commander", q2r_tank()),
        ("monster_gladiator", q2r_gladiator()),
    ])
}

// `q2-expansions.ts`: source precaches from quake-2-re-ts
// `src/{xatrix,rogue,kexgame}/m_*.ts`. Shared callbacks use the same
// selected mount.

/// Precaches shared by both editions (`sharedPrecaches`).
fn q2_shared_precaches() -> HashMap<String, Vec<String>> {
    [
        (
            "monster_medic",
            &[
                "models/monsters/medic/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/medic/idle.wav",
                "sound/medic/medatck1.wav",
                "sound/medic/medatck2.wav",
                "sound/medic/medatck3.wav",
                "sound/medic/medatck4.wav",
                "sound/medic/medatck5.wav",
                "sound/medic/meddeth1.wav",
                "sound/medic/medpain1.wav",
                "sound/medic/medpain2.wav",
                "sound/medic/medsght1.wav",
                "sound/medic/medsrch1.wav",
                "sound/misc/udeath.wav",
            ][..],
        ),
        (
            "monster_supertank",
            &[
                "models/monsters/boss1/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/bosstank/btkdeth1.wav",
                "sound/bosstank/btkengn1.wav",
                "sound/bosstank/btkpain1.wav",
                "sound/bosstank/btkpain2.wav",
                "sound/bosstank/btkpain3.wav",
                "sound/bosstank/btkunqv1.wav",
                "sound/bosstank/btkunqv2.wav",
            ][..],
        ),
        (
            "monster_boss2",
            &[
                "models/monsters/boss2/tris.md2",
                "sound/bosshovr/bhvdeth1.wav",
                "sound/bosshovr/bhvengn1.wav",
                "sound/bosshovr/bhvpain1.wav",
                "sound/bosshovr/bhvpain2.wav",
                "sound/bosshovr/bhvpain3.wav",
                "sound/bosshovr/bhvunqv1.wav",
            ][..],
        ),
        (
            "monster_jorg",
            &[
                "models/monsters/boss3/jorg/tris.md2",
                "models/monsters/boss3/rider/tris.md2",
                "sound/boss3/bs3atck1.wav",
                "sound/boss3/bs3atck2.wav",
                "sound/boss3/bs3deth1.wav",
                "sound/boss3/bs3idle1.wav",
                "sound/boss3/bs3pain1.wav",
                "sound/boss3/bs3pain2.wav",
                "sound/boss3/bs3pain3.wav",
                "sound/boss3/bs3srch1.wav",
                "sound/boss3/bs3srch2.wav",
                "sound/boss3/bs3srch3.wav",
                "sound/boss3/d_hit.wav",
                "sound/boss3/step1.wav",
                "sound/boss3/step2.wav",
                "sound/boss3/w_loop.wav",
                "sound/boss3/xfire.wav",
            ][..],
        ),
        (
            "monster_makron",
            &[
                "models/monsters/boss3/rider/tris.md2",
                "models/objects/gibs/gear/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/makron/bfg_fire.wav",
                "sound/makron/bhit.wav",
                "sound/makron/brain1.wav",
                "sound/makron/death.wav",
                "sound/makron/pain1.wav",
                "sound/makron/pain2.wav",
                "sound/makron/pain3.wav",
                "sound/makron/popup.wav",
                "sound/makron/rail_up.wav",
                "sound/makron/spine.wav",
                "sound/makron/step1.wav",
                "sound/makron/step2.wav",
                "sound/makron/voice.wav",
                "sound/makron/voice3.wav",
                "sound/makron/voice4.wav",
                "sound/misc/udeath.wav",
            ][..],
        ),
        (
            "monster_gekk",
            &[
                "models/monsters/gekk/tris.md2",
                "models/objects/gekkgib/arm/tris.md2",
                "models/objects/gekkgib/claw/tris.md2",
                "models/objects/gekkgib/head/tris.md2",
                "models/objects/gekkgib/leg/tris.md2",
                "models/objects/gekkgib/pelvis/tris.md2",
                "models/objects/gekkgib/torso/tris.md2",
                "models/objects/loogy/tris.md2",
                "sound/gek/gek_high.wav",
                "sound/gek/gek_low.wav",
                "sound/gek/gek_mid.wav",
                "sound/gek/gk_atck1.wav",
                "sound/gek/gk_atck2.wav",
                "sound/gek/gk_atck3.wav",
                "sound/gek/gk_deth1.wav",
                "sound/gek/gk_idle1.wav",
                "sound/gek/gk_pain1.wav",
                "sound/gek/gk_sght1.wav",
                "sound/gek/gk_step1.wav",
                "sound/gek/gk_step2.wav",
                "sound/gek/gk_step3.wav",
                "sound/misc/udeath.wav",
                "sound/mutant/thud1.wav",
            ][..],
        ),
        (
            "monster_fixbot",
            &[
                "models/monsters/fixbot/tris.md2",
                "sound/flyer/flydeth1.wav",
                "sound/flyer/flypain1.wav",
                "sound/misc/welder1.wav",
                "sound/misc/welder2.wav",
                "sound/misc/welder3.wav",
            ][..],
        ),
        (
            "monster_gladb",
            &[
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/gladiator/glddeth2.wav",
                "sound/gladiator/gldidle1.wav",
                "sound/gladiator/gldpain2.wav",
                "sound/gladiator/gldsrch1.wav",
                "sound/gladiator/melee1.wav",
                "sound/gladiator/melee2.wav",
                "sound/gladiator/melee3.wav",
                "sound/gladiator/pain.wav",
                "sound/gladiator/sight.wav",
                "sound/misc/udeath.wav",
                "sound/weapons/plasshot.wav",
            ][..],
        ),
        (
            "monster_boss5",
            &[
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/bosstank/btkdeth1.wav",
                "sound/bosstank/btkengn1.wav",
                "sound/bosstank/btkpain1.wav",
                "sound/bosstank/btkpain2.wav",
                "sound/bosstank/btkpain3.wav",
                "sound/bosstank/btkunqv1.wav",
                "sound/bosstank/btkunqv2.wav",
            ][..],
        ),
        (
            "monster_chick_heat",
            &[
                "models/monsters/bitch/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/chick/chkatck1.wav",
                "sound/chick/chkatck2.wav",
                "sound/chick/chkatck3.wav",
                "sound/chick/chkatck4.wav",
                "sound/chick/chkatck5.wav",
                "sound/chick/chkdeth1.wav",
                "sound/chick/chkdeth2.wav",
                "sound/chick/chkfall1.wav",
                "sound/chick/chkidle1.wav",
                "sound/chick/chkidle2.wav",
                "sound/chick/chkpain1.wav",
                "sound/chick/chkpain2.wav",
                "sound/chick/chkpain3.wav",
                "sound/chick/chksght1.wav",
                "sound/chick/chksrch1.wav",
                "sound/misc/udeath.wav",
            ][..],
        ),
        (
            "monster_stalker",
            &[
                "models/monsters/stalker/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/misc/udeath.wav",
                "sound/stalker/death.wav",
                "sound/stalker/idle.wav",
                "sound/stalker/melee1.wav",
                "sound/stalker/melee2.wav",
                "sound/stalker/pain.wav",
                "sound/stalker/sight.wav",
            ][..],
        ),
        (
            "monster_kamikaze",
            &[
                "models/monsters/flyer/tris.md2",
                "sound/flyer/flyatck1.wav",
                "sound/flyer/flyatck2.wav",
                "sound/flyer/flyatck3.wav",
                "sound/flyer/flydeth1.wav",
                "sound/flyer/flyidle1.wav",
                "sound/flyer/flypain1.wav",
                "sound/flyer/flypain2.wav",
                "sound/flyer/flysght1.wav",
                "sound/flyer/flysrch1.wav",
            ][..],
        ),
        (
            "monster_daedalus",
            &[
                "models/monsters/hover/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/daedalus/daeddeth1.wav",
                "sound/daedalus/daeddeth2.wav",
                "sound/daedalus/daedidle1.wav",
                "sound/daedalus/daedpain1.wav",
                "sound/daedalus/daedpain2.wav",
                "sound/daedalus/daedsght1.wav",
                "sound/daedalus/daedsrch1.wav",
                "sound/daedalus/daedsrch2.wav",
                "sound/hover/hovatck1.wav",
                "sound/hover/hovdeth1.wav",
                "sound/hover/hovdeth2.wav",
                "sound/hover/hovidle1.wav",
                "sound/hover/hovpain1.wav",
                "sound/hover/hovpain2.wav",
                "sound/hover/hovsght1.wav",
                "sound/hover/hovsrch1.wav",
                "sound/hover/hovsrch2.wav",
                "sound/tank/tnkatck3.wav",
            ][..],
        ),
        (
            "monster_turret",
            &[
                "models/monsters/turret/tris.md2",
                "models/monsters/turretbase/tris.md2",
                "models/objects/debris1/tris.md2",
                "models/objects/laser/tris.md2",
                "models/objects/rocket/tris.md2",
                "sound/chick/chkatck2.wav",
                "sound/infantry/infatck1.wav",
                "sound/misc/lasfly.wav",
                "sound/soldier/solatck2.wav",
                "sound/weapons/rockfly.wav",
            ][..],
        ),
        (
            "monster_carrier",
            &[
                "models/monsters/carrier/tris.md2",
                "models/monsters/flyer/tris.md2",
                "models/objects/debris2/tris.md2",
                "models/objects/gibs/gear/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "models/objects/grenade/tris.md2",
                "models/objects/rocket/tris.md2",
                "sound/bosshovr/bhvengn1.wav",
                "sound/carrier/death.wav",
                "sound/carrier/pain_lg.wav",
                "sound/carrier/pain_md.wav",
                "sound/carrier/pain_sm.wav",
                "sound/carrier/sight.wav",
                "sound/flyer/flyatck1.wav",
                "sound/flyer/flyatck2.wav",
                "sound/flyer/flyatck3.wav",
                "sound/flyer/flydeth1.wav",
                "sound/flyer/flyidle1.wav",
                "sound/flyer/flypain1.wav",
                "sound/flyer/flypain2.wav",
                "sound/flyer/flysght1.wav",
                "sound/flyer/flysrch1.wav",
                "sound/gladiator/railgun.wav",
                "sound/gunner/gunatck3.wav",
                "sound/infantry/infatck1.wav",
                "sound/medic_commander/monsterspawn1.wav",
                "sound/tank/rocket.wav",
                "sound/weapons/grenlb1b.wav",
                "sound/weapons/rockfly.wav",
            ][..],
        ),
        (
            "monster_medic_commander",
            &[
                "models/monsters/medic/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/medic/idle.wav",
                "sound/medic/medatck1.wav",
                "sound/medic/medatck2.wav",
                "sound/medic/medatck3.wav",
                "sound/medic/medatck4.wav",
                "sound/medic/medatck5.wav",
                "sound/medic/meddeth1.wav",
                "sound/medic/medpain1.wav",
                "sound/medic/medpain2.wav",
                "sound/medic/medsght1.wav",
                "sound/medic/medsrch1.wav",
                "sound/medic_commander/medatck2c.wav",
                "sound/medic_commander/medatck3a.wav",
                "sound/medic_commander/medatck4a.wav",
                "sound/medic_commander/medatck5a.wav",
                "sound/medic_commander/meddeth.wav",
                "sound/medic_commander/medidle.wav",
                "sound/medic_commander/medpain1.wav",
                "sound/medic_commander/medpain2.wav",
                "sound/medic_commander/medsght.wav",
                "sound/medic_commander/medsrch.wav",
                "sound/medic_commander/monsterspawn1.wav",
                "sound/misc/udeath.wav",
                "sound/tank/tnkatck3.wav",
            ][..],
        ),
        (
            "monster_widow",
            &[
                "models/monsters/blackwidow/gib1/tris.md2",
                "models/monsters/blackwidow/gib2/tris.md2",
                "models/monsters/blackwidow/gib3/tris.md2",
                "models/monsters/blackwidow/gib4/tris.md2",
                "models/monsters/blackwidow/tris.md2",
                "models/monsters/blackwidow2/gib1/tris.md2",
                "models/monsters/blackwidow2/gib2/tris.md2",
                "models/monsters/blackwidow2/gib3/tris.md2",
                "models/monsters/blackwidow2/gib4/tris.md2",
                "models/monsters/legs/tris.md2",
                "models/monsters/stalker/tris.md2",
                "models/objects/gibs/gear/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/gladiator/railgun.wav",
                "sound/misc/bigtele.wav",
                "sound/misc/bwidowbeamout.wav",
                "sound/stalker/death.wav",
                "sound/stalker/idle.wav",
                "sound/stalker/melee1.wav",
                "sound/stalker/melee2.wav",
                "sound/stalker/pain.wav",
                "sound/stalker/sight.wav",
                "sound/tank/tnkatck3.wav",
                "sound/widow/bw1pain1.wav",
                "sound/widow/bw1pain2.wav",
                "sound/widow/bw1pain3.wav",
                "sound/widow/bwstep2.wav",
                "sound/widow/bwstep3.wav",
                "sound/widow/laugh.wav",
            ][..],
        ),
        (
            "monster_widow2",
            &[
                "models/monsters/blackwidow/gib1/tris.md2",
                "models/monsters/blackwidow/gib2/tris.md2",
                "models/monsters/blackwidow/gib3/tris.md2",
                "models/monsters/blackwidow/gib4/tris.md2",
                "models/monsters/blackwidow2/gib1/tris.md2",
                "models/monsters/blackwidow2/gib2/tris.md2",
                "models/monsters/blackwidow2/gib3/tris.md2",
                "models/monsters/blackwidow2/gib4/tris.md2",
                "models/monsters/blackwidow2/tris.md2",
                "models/monsters/stalker/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/chest/tris.md2",
                "models/objects/gibs/head2/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "models/proj/disintegrator/tris.md2",
                "sound/bosshovr/bhvunqv1.wav",
                "sound/brain/brnatck3.wav",
                "sound/infantry/melee2.wav",
                "sound/misc/fhit3.wav",
                "sound/misc/udeath.wav",
                "sound/parasite/paratck1.wav",
                "sound/parasite/pardeth1.wav",
                "sound/parasite/parpain1.wav",
                "sound/parasite/parpain2.wav",
                "sound/parasite/parsght1.wav",
                "sound/tank/tnkatck3.wav",
                "sound/weapons/disint2.wav",
                "sound/weapons/disrupt.wav",
                "sound/widow/bw2pain1.wav",
                "sound/widow/bw2pain2.wav",
                "sound/widow/bw2pain3.wav",
                "sound/widow/death.wav",
            ][..],
        ),
    ]
    .into_iter()
    .map(|(classname, paths): (&str, &[&str])| (classname.to_string(), paths.iter().map(ToString::to_string).collect()))
    .collect()
}

/// Classic-only precaches (`classicPrecaches`).
fn q2_classic_precaches() -> HashMap<String, Vec<String>> {
    [
        ("monster_medic", &["models/objects/gibs/head2/tris.md2"][..]),
        (
            "monster_supertank",
            &[
                "models/objects/gibs/chest/tris.md2",
                "models/objects/gibs/gear/tris.md2",
            ][..],
        ),
        ("monster_fixbot", &["sound/misc/lasfly.wav"][..]),
        (
            "monster_gladb",
            &["models/monsters/gladb/tris.md2", "models/objects/gibs/head2/tris.md2"][..],
        ),
        (
            "monster_boss5",
            &[
                "models/monsters/boss5/tris.md2",
                "models/objects/gibs/chest/tris.md2",
                "models/objects/gibs/gear/tris.md2",
            ][..],
        ),
        ("monster_chick_heat", &["models/objects/gibs/head2/tris.md2"][..]),
        (
            "monster_stalker",
            &[
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/head2/tris.md2",
                "models/proj/laser2/tris.md2",
            ][..],
        ),
        (
            "monster_daedalus",
            &["models/objects/gibs/bone/tris.md2", "sound/misc/udeath.wav"][..],
        ),
        ("monster_turret", &["sound/world/dr_short.wav"][..]),
        (
            "monster_carrier",
            &["models/items/spawngro/tris.md2", "models/items/spawngro2/tris.md2"][..],
        ),
        (
            "monster_medic_commander",
            &[
                "models/items/spawngro/tris.md2",
                "models/items/spawngro2/tris.md2",
                "models/objects/gibs/head2/tris.md2",
            ][..],
        ),
        (
            "monster_widow",
            &[
                "models/items/spawngro2/tris.md2",
                "models/proj/laser2/tris.md2",
                "sound/bosshovr/bhvunqv1.wav",
            ][..],
        ),
        (
            "monster_widow2",
            &["models/items/spawngro2/tris.md2", "models/proj/laser2/tris.md2"][..],
        ),
    ]
    .into_iter()
    .map(|(classname, paths): (&str, &[&str])| (classname.to_string(), paths.iter().map(ToString::to_string).collect()))
    .collect()
}

/// Rerelease-only precaches (`rereleasePrecaches`).
fn q2_rerelease_precaches() -> HashMap<String, Vec<String>> {
    [
        (
            "monster_medic",
            &[
                "models/items/spawngro3/tris.md2",
                "models/monsters/medic/gibs/chest.md2",
                "models/monsters/medic/gibs/gun.md2",
                "models/monsters/medic/gibs/head.md2",
                "models/monsters/medic/gibs/hook.md2",
                "models/monsters/medic/gibs/leg.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/medic_commander/medatck2c.wav",
                "sound/medic_commander/medatck3a.wav",
                "sound/medic_commander/medatck4a.wav",
                "sound/medic_commander/medatck5a.wav",
                "sound/medic_commander/meddeth.wav",
                "sound/medic_commander/medidle.wav",
                "sound/medic_commander/medpain1.wav",
                "sound/medic_commander/medpain2.wav",
                "sound/medic_commander/medsght.wav",
                "sound/medic_commander/medsrch.wav",
                "sound/medic_commander/monsterspawn1.wav",
                "sound/tank/tnkatck3.wav",
            ][..],
        ),
        (
            "monster_supertank",
            &[
                "models/monsters/boss1/gibs/cgun.md2",
                "models/monsters/boss1/gibs/chest.md2",
                "models/monsters/boss1/gibs/core.md2",
                "models/monsters/boss1/gibs/head.md2",
                "models/monsters/boss1/gibs/ltread.md2",
                "models/monsters/boss1/gibs/rgun.md2",
                "models/monsters/boss1/gibs/rtread.md2",
                "models/monsters/boss1/gibs/tube.md2",
                "models/objects/rocket/tris.md2",
                "sound/gunner/gunatck3.wav",
                "sound/infantry/infatck1.wav",
                "sound/tank/rocket.wav",
                "sound/weapons/railgr1a.wav",
                "sound/weapons/rockfly.wav",
            ][..],
        ),
        (
            "monster_boss2",
            &[
                "models/monsters/boss2/gibs/chaingun.md2",
                "models/monsters/boss2/gibs/chest.md2",
                "models/monsters/boss2/gibs/cpu.md2",
                "models/monsters/boss2/gibs/engine.md2",
                "models/monsters/boss2/gibs/head.md2",
                "models/monsters/boss2/gibs/larm.md2",
                "models/monsters/boss2/gibs/rarm.md2",
                "models/monsters/boss2/gibs/rocket.md2",
                "models/monsters/boss2/gibs/spine.md2",
                "models/monsters/boss2/gibs/wing.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/flyer/flyatck3.wav",
                "sound/infantry/infatck1.wav",
                "sound/tank/rocket.wav",
            ][..],
        ),
        (
            "monster_jorg",
            &[
                "models/monsters/boss3/jorg/gibs/chest.md2",
                "models/monsters/boss3/jorg/gibs/foot.md2",
                "models/monsters/boss3/jorg/gibs/gun.md2",
                "models/monsters/boss3/jorg/gibs/head.md2",
                "models/monsters/boss3/jorg/gibs/spike.md2",
                "models/monsters/boss3/jorg/gibs/spine.md2",
                "models/monsters/boss3/jorg/gibs/thigh.md2",
                "models/monsters/boss3/jorg/gibs/tube.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/boss3/bs3atck1_end.wav",
                "sound/boss3/bs3atck1_loop.wav",
                "sound/makron/bfg_fire.wav",
            ][..],
        ),
        (
            "monster_gekk",
            &["sound/gek/gk_atck4.wav", "sound/gek/loogie_hit.wav"][..],
        ),
        (
            "monster_gladb",
            &[
                "models/monsters/gladiatr/gibs/chest.md2",
                "models/monsters/gladiatr/gibs/head.md2",
                "models/monsters/gladiatr/gibs/larm.md2",
                "models/monsters/gladiatr/gibs/rarm.md2",
                "models/monsters/gladiatr/gibs/thigh.md2",
                "models/monsters/gladiatr/tris.md2",
                "sound/gladiator/death.wav",
                "sound/gladiator/railgun.wav",
                "sound/weapons/phaloop.wav",
                "sound/weapons/rg_hum.wav",
            ][..],
        ),
        (
            "monster_boss5",
            &[
                "models/monsters/boss1/gibs/cgun.md2",
                "models/monsters/boss1/gibs/chest.md2",
                "models/monsters/boss1/gibs/core.md2",
                "models/monsters/boss1/gibs/head.md2",
                "models/monsters/boss1/gibs/ltread.md2",
                "models/monsters/boss1/gibs/rgun.md2",
                "models/monsters/boss1/gibs/rtread.md2",
                "models/monsters/boss1/gibs/tube.md2",
                "models/monsters/boss1/tris.md2",
                "models/objects/rocket/tris.md2",
                "sound/gunner/gunatck3.wav",
                "sound/infantry/infatck1.wav",
                "sound/tank/rocket.wav",
                "sound/weapons/railgr1a.wav",
                "sound/weapons/rockfly.wav",
            ][..],
        ),
        (
            "monster_chick_heat",
            &[
                "models/monsters/bitch/gibs/arm.md2",
                "models/monsters/bitch/gibs/chest.md2",
                "models/monsters/bitch/gibs/foot.md2",
                "models/monsters/bitch/gibs/head.md2",
                "models/monsters/bitch/gibs/tube.md2",
                "sound/weapons/railgr1a.wav",
            ][..],
        ),
        (
            "monster_soldier_ripper",
            &[
                "models/monsters/soldier/gibs/arm.md2",
                "models/monsters/soldier/gibs/chest.md2",
                "models/monsters/soldier/gibs/gun.md2",
                "models/monsters/soldier/gibs/head.md2",
                "models/monsters/soldier/tris.md2",
                "models/objects/boomrang/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/bone2/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/laser/tris.md2",
                "sound/infantry/infatck3.wav",
                "sound/misc/lasfly.wav",
                "sound/misc/udeath.wav",
                "sound/soldier/solatck1.wav",
                "sound/soldier/solatck2.wav",
                "sound/soldier/solatck3.wav",
                "sound/soldier/soldeth1.wav",
                "sound/soldier/soldeth2.wav",
                "sound/soldier/soldeth3.wav",
                "sound/soldier/solidle1.wav",
                "sound/soldier/solpain1.wav",
                "sound/soldier/solpain2.wav",
                "sound/soldier/solpain3.wav",
                "sound/soldier/solsght1.wav",
                "sound/soldier/solsrch1.wav",
                "sound/weapons/hyprbd1a.wav",
                "sound/weapons/hyprbl1a.wav",
            ][..],
        ),
        (
            "monster_soldier_hypergun",
            &[
                "models/monsters/soldier/gibs/arm.md2",
                "models/monsters/soldier/gibs/chest.md2",
                "models/monsters/soldier/gibs/gun.md2",
                "models/monsters/soldier/gibs/head.md2",
                "models/monsters/soldier/tris.md2",
                "models/objects/boomrang/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/bone2/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/laser/tris.md2",
                "sound/infantry/infatck3.wav",
                "sound/misc/lasfly.wav",
                "sound/misc/udeath.wav",
                "sound/soldier/solatck1.wav",
                "sound/soldier/solatck2.wav",
                "sound/soldier/solatck3.wav",
                "sound/soldier/soldeth1.wav",
                "sound/soldier/soldeth2.wav",
                "sound/soldier/soldeth3.wav",
                "sound/soldier/solidle1.wav",
                "sound/soldier/solpain1.wav",
                "sound/soldier/solpain2.wav",
                "sound/soldier/solpain3.wav",
                "sound/soldier/solsght1.wav",
                "sound/soldier/solsrch1.wav",
                "sound/weapons/hyprbd1a.wav",
                "sound/weapons/hyprbl1a.wav",
            ][..],
        ),
        (
            "monster_soldier_lasergun",
            &[
                "models/monsters/soldier/gibs/arm.md2",
                "models/monsters/soldier/gibs/chest.md2",
                "models/monsters/soldier/gibs/gun.md2",
                "models/monsters/soldier/gibs/head.md2",
                "models/monsters/soldier/tris.md2",
                "models/objects/boomrang/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/bone2/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/laser/tris.md2",
                "sound/infantry/infatck3.wav",
                "sound/misc/lasfly.wav",
                "sound/misc/udeath.wav",
                "sound/soldier/solatck1.wav",
                "sound/soldier/solatck2.wav",
                "sound/soldier/solatck3.wav",
                "sound/soldier/soldeth1.wav",
                "sound/soldier/soldeth2.wav",
                "sound/soldier/soldeth3.wav",
                "sound/soldier/solidle1.wav",
                "sound/soldier/solpain1.wav",
                "sound/soldier/solpain2.wav",
                "sound/soldier/solpain3.wav",
                "sound/soldier/solsght1.wav",
                "sound/soldier/solsrch1.wav",
                "sound/weapons/hyprbd1a.wav",
                "sound/weapons/hyprbl1a.wav",
            ][..],
        ),
        (
            "monster_stalker",
            &[
                "models/monsters/stalker/gibs/bodya.md2",
                "models/monsters/stalker/gibs/bodyb.md2",
                "models/monsters/stalker/gibs/claw.md2",
                "models/monsters/stalker/gibs/foot.md2",
                "models/monsters/stalker/gibs/head.md2",
                "models/monsters/stalker/gibs/leg.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "models/objects/laser/tris.md2",
            ][..],
        ),
        (
            "monster_kamikaze",
            &[
                "models/monsters/flyer/gibs/base.md2",
                "models/monsters/flyer/gibs/gun.md2",
                "models/monsters/flyer/gibs/head.md2",
                "models/monsters/flyer/gibs/wing.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
            ][..],
        ),
        (
            "monster_daedalus",
            &[
                "models/monsters/hover/gibs/chest.md2",
                "models/monsters/hover/gibs/foot.md2",
                "models/monsters/hover/gibs/head.md2",
                "models/monsters/hover/gibs/ring.md2",
                "models/objects/gibs/sm_metal/tris.md2",
            ][..],
        ),
        (
            "monster_turret",
            &[
                "sound/turret/moved.wav",
                "sound/turret/moving.wav",
                "sound/weapons/chngnu1a.wav",
            ][..],
        ),
        (
            "monster_carrier",
            &[
                "models/items/spawngro3/tris.md2",
                "models/monsters/carrier/gibs/base.md2",
                "models/monsters/carrier/gibs/chest.md2",
                "models/monsters/carrier/gibs/gl.md2",
                "models/monsters/carrier/gibs/head.md2",
                "models/monsters/carrier/gibs/lcg.md2",
                "models/monsters/carrier/gibs/lwing.md2",
                "models/monsters/carrier/gibs/rcg.md2",
                "models/monsters/carrier/gibs/rwing.md2",
                "models/monsters/carrier/gibs/spawner.md2",
                "models/monsters/carrier/gibs/thigh.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/weapons/chngnd1a.wav",
                "sound/weapons/chngnl1a.wav",
                "sound/weapons/chngnu1a.wav",
            ][..],
        ),
        (
            "monster_medic_commander",
            &[
                "models/items/spawngro3/tris.md2",
                "models/monsters/medic/gibs/chest.md2",
                "models/monsters/medic/gibs/gun.md2",
                "models/monsters/medic/gibs/head.md2",
                "models/monsters/medic/gibs/hook.md2",
                "models/monsters/medic/gibs/leg.md2",
                "models/objects/gibs/sm_metal/tris.md2",
            ][..],
        ),
        (
            "monster_widow",
            &[
                "models/items/spawngro3/tris.md2",
                "models/objects/laser/tris.md2",
                "sound/widow/bwstep1.wav",
            ][..],
        ),
        (
            "monster_widow2",
            &[
                "models/items/spawngro3/tris.md2",
                "models/objects/laser/tris.md2",
                "sound/widow/bwstep1.wav",
            ][..],
        ),
        (
            "monster_arachnid",
            &[
                "models/monsters/arachnid/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/head2/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/arachnid/death.wav",
                "sound/arachnid/pain.wav",
                "sound/arachnid/sight.wav",
                "sound/gladiator/melee2.wav",
                "sound/gladiator/melee3.wav",
                "sound/gladiator/railgun.wav",
                "sound/insane/insane11.wav",
                "sound/misc/udeath.wav",
            ][..],
        ),
        (
            "monster_guardian",
            &[
                "models/monsters/guardian/gib1.md2",
                "models/monsters/guardian/gib2.md2",
                "models/monsters/guardian/gib3.md2",
                "models/monsters/guardian/gib4.md2",
                "models/monsters/guardian/gib5.md2",
                "models/monsters/guardian/gib6.md2",
                "models/monsters/guardian/gib7.md2",
                "models/monsters/guardian/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/objects/gibs/sm_metal/tris.md2",
                "sound/weapons/hyprbl1a.wav",
                "sound/weapons/hyprbu1a.wav",
                "sound/weapons/laser2.wav",
                "sound/zortemp/step.wav",
            ][..],
        ),
        (
            "monster_shambler",
            &[
                "models/monsters/shambler/tris.md2",
                "models/objects/gibs/chest/tris.md2",
                "models/objects/gibs/head2/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "models/proj/lightning/tris.md2",
                "sound/misc/udeath.wav",
                "sound/shambler/melee1.wav",
                "sound/shambler/melee2.wav",
                "sound/shambler/sattck1.wav",
                "sound/shambler/sboom.wav",
                "sound/shambler/sdeath.wav",
                "sound/shambler/shurt2.wav",
                "sound/shambler/sidle.wav",
                "sound/shambler/smack.wav",
                "sound/shambler/ssight.wav",
            ][..],
        ),
        (
            "monster_guncmdr",
            &[
                "models/monsters/gunner/gibs/chest.md2",
                "models/monsters/gunner/gibs/foot.md2",
                "models/monsters/gunner/gibs/garm.md2",
                "models/monsters/gunner/gibs/gun.md2",
                "models/monsters/gunner/gibs/head.md2",
                "models/monsters/gunner/tris.md2",
                "models/objects/gibs/bone/tris.md2",
                "models/objects/gibs/gear/tris.md2",
                "models/objects/gibs/sm_meat/tris.md2",
                "sound/guncmdr/gcdratck1.wav",
                "sound/guncmdr/gcdratck2.wav",
                "sound/guncmdr/gcdratck3.wav",
                "sound/guncmdr/gcdrdeath1.wav",
                "sound/guncmdr/gcdridle1.wav",
                "sound/guncmdr/gcdrpain1.wav",
                "sound/guncmdr/gcdrpain2.wav",
                "sound/guncmdr/gcdrsrch1.wav",
                "sound/guncmdr/sight1.wav",
                "sound/misc/udeath.wav",
            ][..],
        ),
    ]
    .into_iter()
    .map(|(classname, paths): (&str, &[&str])| (classname.to_string(), paths.iter().map(ToString::to_string).collect()))
    .collect()
}

/// Callbacks precached for every expansion creature (`sharedCallbacks`).
fn q2_shared_callbacks() -> Vec<String> {
    [
        "sound/misc/udeath.wav",
        "sound/infantry/inflies1.wav",
        "sound/misc/fhit3.wav",
        "sound/player/watr_in.wav",
        "sound/player/watr_out.wav",
        "sound/player/lava1.wav",
        "sound/player/lava2.wav",
        "models/objects/gibs/bone/tris.md2",
        "models/objects/gibs/sm_meat/tris.md2",
        "models/objects/gibs/head2/tris.md2",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// Expand model entries with shared, edition, and conditional precaches,
/// then merge boss dependency resources (`precached`).
fn q2_precached(
    creatures: &HashMap<String, MonsterCreature>,
    edition: SourceEdition,
) -> HashMap<String, MonsterCreature> {
    const LASER: &str = "models/objects/laser/tris.md2";
    const ROCKET: &str = "models/objects/rocket/tris.md2";
    const GRENADE: &str = "models/objects/grenade/tris.md2";
    const BFG: &str = "sprites/s_bfg1.sp2";
    let shared = q2_shared_precaches();
    let callbacks = q2_shared_callbacks();
    let edition_precaches = match edition {
        SourceEdition::Classic => q2_classic_precaches(),
        SourceEdition::Rerelease => q2_rerelease_precaches(),
    };
    let mut result: HashMap<String, MonsterCreature> = HashMap::new();
    for (classname, entry) in creatures {
        let mut resources: Vec<String> = Vec::new();
        union_ordered(&mut resources, &entry.resources);
        union_ordered(&mut resources, &callbacks);
        if let Some(extra) = shared.get(classname) {
            union_ordered(&mut resources, extra);
        }
        if let Some(extra) = edition_precaches.get(classname) {
            union_ordered(&mut resources, extra);
        }
        if edition == SourceEdition::Rerelease
            && (classname == "monster_medic" || classname == "monster_medic_commander")
        {
            for step in 1..=4 {
                union_ordered(&mut resources, &[format!("sound/player/step{step}.wav")]);
            }
        }
        if classname == "monster_turret" {
            union_ordered(&mut resources, &["sound/world/dr_short.wav".to_string()]);
        }
        if classname == "monster_makron" {
            union_ordered(&mut resources, &[BFG.to_string()]);
        }
        if resources.iter().any(|path| path == LASER) {
            union_ordered(&mut resources, &["sound/misc/lasfly.wav".to_string()]);
        }
        if resources.iter().any(|path| path == ROCKET) {
            union_ordered(
                &mut resources,
                &[
                    "sound/weapons/rockfly.wav".to_string(),
                    "models/objects/debris2/tris.md2".to_string(),
                ],
            );
        }
        if resources.iter().any(|path| path == GRENADE) {
            union_ordered(&mut resources, &["sound/weapons/grenlb1b.wav".to_string()]);
        }
        if resources.iter().any(|path| path == BFG) {
            union_ordered(
                &mut resources,
                &[
                    "sprites/s_bfg3.sp2".to_string(),
                    "sound/weapons/bfg__l1a.wav".to_string(),
                    "sound/weapons/bfg__x1b.wav".to_string(),
                ],
            );
        }
        result.insert(classname.clone(), MonsterCreature { resources });
    }
    let mut available = q2_ordinary(edition);
    available.extend(result.clone());
    for (classname, dependencies) in [
        ("monster_jorg", &["monster_makron"][..]),
        ("monster_carrier", &["monster_flyer"][..]),
        ("monster_widow", &["monster_stalker"][..]),
        ("monster_widow2", &["monster_stalker"][..]),
        (
            "monster_medic_commander",
            &[
                "monster_soldier_light",
                "monster_soldier",
                "monster_soldier_ss",
                "monster_infantry",
                "monster_gunner",
                "monster_medic",
                "monster_gladiator",
            ][..],
        ),
    ] {
        let Some(entry) = result.get(classname) else {
            continue;
        };
        let mut resources = entry.resources.clone();
        for dependency in dependencies {
            if let Some(extra) = available.get(*dependency) {
                union_ordered(&mut resources, &extra.resources);
            }
        }
        result.insert(classname.to_string(), MonsterCreature { resources });
    }
    result
}

/// One model plus its direct dependencies (`model`).
fn q2_model(path: &str, dependencies: &[&str]) -> MonsterCreature {
    let mut resources = vec![path.to_string()];
    resources.extend(dependencies.iter().map(ToString::to_string));
    MonsterCreature { resources }
}

/// Base-game boss additions shared by every expanded table
/// (`baseAdditional`).
fn q2_base_additional() -> HashMap<String, MonsterCreature> {
    const LASER: &str = "models/objects/laser/tris.md2";
    const ROCKET: &str = "models/objects/rocket/tris.md2";
    catalog(vec![
        ("monster_medic", q2_model("models/monsters/medic/tris.md2", &[LASER])),
        (
            "monster_supertank",
            q2_model("models/monsters/boss1/tris.md2", &[ROCKET]),
        ),
        ("monster_boss2", q2_model("models/monsters/boss2/tris.md2", &[ROCKET])),
        (
            "monster_jorg",
            q2_model(
                "models/monsters/boss3/rider/tris.md2",
                &["models/monsters/boss3/jorg/tris.md2", "sprites/s_bfg1.sp2"],
            ),
        ),
        (
            "monster_makron",
            q2_model("models/monsters/boss3/rider/tris.md2", &[LASER]),
        ),
    ])
}

/// Xatrix models (`xatrix`).
fn q2_xatrix_models() -> HashMap<String, MonsterCreature> {
    const LASER: &str = "models/objects/laser/tris.md2";
    catalog(vec![
        (
            "monster_gekk",
            q2_model("models/monsters/gekk/tris.md2", &["models/objects/loogy/tris.md2"]),
        ),
        ("monster_fixbot", q2_model("models/monsters/fixbot/tris.md2", &[LASER])),
        (
            "monster_gladb",
            q2_model("models/monsters/gladb/tris.md2", &["sprites/s_photon.sp2"]),
        ),
        (
            "monster_boss5",
            q2_model("models/monsters/boss5/tris.md2", &["models/objects/rocket/tris.md2"]),
        ),
        (
            "monster_chick_heat",
            q2_model("models/monsters/bitch/tris.md2", &["models/objects/rocket/tris.md2"]),
        ),
        (
            "monster_soldier_ripper",
            q2_model(
                "models/monsters/soldierh/tris.md2",
                &["models/objects/boomrang/tris.md2"],
            ),
        ),
        (
            "monster_soldier_hypergun",
            q2_model("models/monsters/soldierh/tris.md2", &[LASER]),
        ),
        (
            "monster_soldier_lasergun",
            q2_model("models/monsters/soldierh/tris.md2", &[]),
        ),
    ])
}

/// Rogue models (`rogue`).
fn q2_rogue_models() -> HashMap<String, MonsterCreature> {
    const LASER: &str = "models/objects/laser/tris.md2";
    const ROCKET: &str = "models/objects/rocket/tris.md2";
    catalog(vec![
        (
            "monster_stalker",
            q2_model("models/monsters/stalker/tris.md2", &[LASER]),
        ),
        ("monster_kamikaze", q2_model("models/monsters/flyer/tris.md2", &[])),
        ("monster_daedalus", q2_model("models/monsters/hover/tris.md2", &[LASER])),
        (
            "monster_turret",
            q2_model("models/monsters/turret/tris.md2", &[LASER, ROCKET]),
        ),
        (
            "monster_carrier",
            q2_model(
                "models/monsters/carrier/tris.md2",
                &[
                    "models/monsters/flyer/tris.md2",
                    ROCKET,
                    "models/objects/grenade/tris.md2",
                ],
            ),
        ),
        (
            "monster_medic_commander",
            q2_model("models/monsters/medic/tris.md2", &[LASER]),
        ),
        (
            "monster_widow",
            q2_model(
                "models/monsters/blackwidow/tris.md2",
                &["models/monsters/stalker/tris.md2", LASER],
            ),
        ),
        (
            "monster_widow2",
            q2_model(
                "models/monsters/blackwidow2/tris.md2",
                &["models/monsters/stalker/tris.md2", "models/proj/disintegrator/tris.md2"],
            ),
        ),
    ])
}

/// Rerelease-only models (`rerelease`).
fn q2_rerelease_models() -> HashMap<String, MonsterCreature> {
    catalog(vec![
        ("monster_arachnid", q2_model("models/monsters/arachnid/tris.md2", &[])),
        (
            "monster_guardian",
            q2_model("models/monsters/guardian/tris.md2", &["models/objects/rocket/tris.md2"]),
        ),
        ("monster_shambler", q2_model("models/monsters/shambler/tris.md2", &[])),
        (
            "monster_guncmdr",
            q2_model("models/monsters/gunner/tris.md2", &["models/objects/grenade/tris.md2"]),
        ),
    ])
}

/// Base-game creatures for one edition (`ordinary`).
///
/// Panics with the donor message when the edition has no base table,
/// which cannot happen for tables built by [`q2_monster_sources`].
fn q2_ordinary(edition: SourceEdition) -> HashMap<String, MonsterCreature> {
    q2_monster_sources()
        .into_iter()
        .find(|source| source.edition == edition)
        .unwrap_or_else(|| panic!("Missing Q2 {} base monster definitions", edition_name(edition)))
        .creatures
}

/// Classic base-game boss additions (`q2ExpandedClassicCreatures`).
#[must_use]
pub fn q2_expanded_classic_creatures() -> HashMap<String, MonsterCreature> {
    q2_precached(&q2_base_additional(), SourceEdition::Classic)
}

/// Edition-specific source models and precaches resolve through the
/// selected content mount (`q2ExpandedBaseCreatures`).
#[must_use]
pub fn q2_expanded_base_creatures() -> HashMap<String, MonsterCreature> {
    let mut creatures = q2_base_additional();
    for (classname, entry) in q2_xatrix_models() {
        if classname != "monster_soldier_ripper"
            && classname != "monster_soldier_hypergun"
            && classname != "monster_soldier_lasergun"
        {
            creatures.insert(classname, entry);
        }
    }
    creatures.extend(q2_rogue_models());
    creatures.extend(q2_rerelease_models());
    creatures.insert(
        "monster_gladb".to_string(),
        q2_model("models/monsters/gladiatr/tris.md2", &["sprites/s_photon.sp2"]),
    );
    creatures.insert(
        "monster_boss5".to_string(),
        q2_model("models/monsters/boss1/tris.md2", &["models/objects/rocket/tris.md2"]),
    );
    q2_precached(&creatures, SourceEdition::Rerelease)
}

/// Quake II expansion monster sources (`q2ExpansionSources`).
#[must_use]
pub fn q2_expansion_sources() -> Vec<MonsterSourceDefinition> {
    let mut classic_xatrix = q2_ordinary(SourceEdition::Classic);
    classic_xatrix.extend(q2_base_additional());
    classic_xatrix.extend(q2_xatrix_models());
    let mut classic_rogue = q2_ordinary(SourceEdition::Classic);
    classic_rogue.extend(q2_base_additional());
    classic_rogue.extend(q2_rogue_models());
    let expanded = q2_expanded_base_creatures();
    let mut rerelease_xatrix = q2_ordinary(SourceEdition::Rerelease);
    rerelease_xatrix.extend(expanded.clone());
    let mut rerelease_rogue = q2_ordinary(SourceEdition::Rerelease);
    rerelease_rogue.extend(expanded.clone());
    let mut rerelease_mg2 = q2_ordinary(SourceEdition::Rerelease);
    rerelease_mg2.extend(expanded);
    vec![
        source(
            "q2:monsters/classic/xatrix",
            MonsterFamily::Q2,
            SourceEdition::Classic,
            MonsterProgram::Xatrix,
            q2_precached(&classic_xatrix, SourceEdition::Classic),
        ),
        source(
            "q2:monsters/classic/rogue",
            MonsterFamily::Q2,
            SourceEdition::Classic,
            MonsterProgram::Rogue,
            q2_precached(&classic_rogue, SourceEdition::Classic),
        ),
        source(
            "q2:monsters/rerelease/xatrix",
            MonsterFamily::Q2,
            SourceEdition::Rerelease,
            MonsterProgram::Xatrix,
            rerelease_xatrix,
        ),
        source(
            "q2:monsters/rerelease/rogue",
            MonsterFamily::Q2,
            SourceEdition::Rerelease,
            MonsterProgram::Rogue,
            rerelease_rogue,
        ),
        source(
            "q2:monsters/rerelease/mg2",
            MonsterFamily::Q2,
            SourceEdition::Rerelease,
            MonsterProgram::Mg2,
            rerelease_mg2,
        ),
    ]
}

// `roster.ts`.

/// Monster replacement preference role (`MonsterRole`).
///
/// Roles describe a replacement preference, not behavioral equivalence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonsterRole {
    /// Grunt equivalent.
    Trooper,
    /// Hitscan ranged equivalent.
    Ranged,
    /// Melee equivalent.
    Melee,
    /// Leaping equivalent.
    Leaper,
    /// Grenade equivalent.
    Grenadier,
    /// Rocket equivalent.
    Rocket,
    /// Artillery equivalent.
    Artillery,
    /// Hybrid ranged/melee equivalent.
    Hybrid,
    /// Heavy equivalent.
    Heavy,
    /// Flying equivalent.
    Flying,
    /// Aquatic equivalent.
    Aquatic,
    /// Boss.
    Boss,
    /// Special-case monster.
    Special,
}

impl MonsterRole {
    /// Donor role text (`"trooper"`, ...).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            MonsterRole::Trooper => "trooper",
            MonsterRole::Ranged => "ranged",
            MonsterRole::Melee => "melee",
            MonsterRole::Leaper => "leaper",
            MonsterRole::Grenadier => "grenadier",
            MonsterRole::Rocket => "rocket",
            MonsterRole::Artillery => "artillery",
            MonsterRole::Hybrid => "hybrid",
            MonsterRole::Heavy => "heavy",
            MonsterRole::Flying => "flying",
            MonsterRole::Aquatic => "aquatic",
            MonsterRole::Boss => "boss",
            MonsterRole::Special => "special",
        }
    }
}

/// One replaceable roster entry (`MonsterRosterSlot`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterRosterSlot {
    /// Entity classname.
    pub classname: String,
    /// Replacement preference role.
    pub role: MonsterRole,
}

/// Role order used before the classname sort (`slots`).
const ROLE_ORDER: [MonsterRole; 13] = [
    MonsterRole::Trooper,
    MonsterRole::Ranged,
    MonsterRole::Melee,
    MonsterRole::Leaper,
    MonsterRole::Grenadier,
    MonsterRole::Rocket,
    MonsterRole::Artillery,
    MonsterRole::Hybrid,
    MonsterRole::Heavy,
    MonsterRole::Flying,
    MonsterRole::Aquatic,
    MonsterRole::Boss,
    MonsterRole::Special,
];

/// Authored Quake entry points by role (`q1`).
fn q1_roles() -> Vec<(MonsterRole, Vec<&'static str>)> {
    vec![
        (MonsterRole::Trooper, vec!["monster_army", "monster_army_infected"]),
        (
            MonsterRole::Ranged,
            vec!["monster_enforcer", "monster_enforcer_infected"],
        ),
        (
            MonsterRole::Melee,
            vec!["monster_dog", "monster_knight", "monster_knight_infected"],
        ),
        (MonsterRole::Leaper, vec!["monster_demon1", "monster_demodog"]),
        (MonsterRole::Grenadier, vec!["monster_ogre", "monster_ogre_marksman"]),
        (MonsterRole::Rocket, vec!["monster_ogre_rocket"]),
        (MonsterRole::Artillery, vec!["monster_shalrath", "monster_scourge"]),
        (
            MonsterRole::Hybrid,
            vec![
                "monster_hell_knight",
                "monster_hell_knight_infected",
                "monster_ranged_knight",
            ],
        ),
        (MonsterRole::Heavy, vec!["monster_shambler"]),
        (MonsterRole::Flying, vec!["monster_wizard", "monster_wrath"]),
        (MonsterRole::Aquatic, vec!["monster_fish", "monster_eel"]),
        (
            MonsterRole::Boss,
            vec![
                "monster_boss",
                "monster_oldone",
                "monster_armagon",
                "monster_dragon",
                "monster_super_wrath",
                "monster_boss_final",
                "monster_oldone_new",
                "monster_super_shambler",
            ],
        ),
        (
            MonsterRole::Special,
            vec![
                "monster_zombie",
                "monster_tarbaby",
                "monster_gremlin",
                "monster_decoy",
                "monster_spikemine",
                "monster_sword",
                "monster_lava_man",
                "monster_morph",
                "monster_mummy",
                "monster_vomit",
                "monster_dragon_dead",
                "monster_ghost",
                "monster_orb",
                "monster_szombie",
            ],
        ),
    ]
}

/// Authored Quake II entry points by role (`q2`).
fn q2_roles() -> Vec<(MonsterRole, Vec<&'static str>)> {
    vec![
        (MonsterRole::Trooper, vec!["monster_soldier", "monster_soldier_light"]),
        (
            MonsterRole::Ranged,
            vec![
                "monster_infantry",
                "monster_soldier_ss",
                "monster_soldier_hypergun",
                "monster_soldier_lasergun",
                "monster_soldier_ripper",
            ],
        ),
        (MonsterRole::Melee, vec!["monster_berserk"]),
        (MonsterRole::Leaper, vec!["monster_mutant"]),
        (MonsterRole::Grenadier, vec!["monster_gunner", "monster_guncmdr"]),
        (MonsterRole::Rocket, vec!["monster_chick", "monster_chick_heat"]),
        (
            MonsterRole::Artillery,
            vec!["monster_gladiator", "monster_gladb", "monster_arachnid"],
        ),
        (MonsterRole::Hybrid, vec!["monster_brain"]),
        (
            MonsterRole::Heavy,
            vec!["monster_tank", "monster_tank_commander", "monster_shambler"],
        ),
        (
            MonsterRole::Flying,
            vec!["monster_flyer", "monster_floater", "monster_hover", "monster_daedalus"],
        ),
        (MonsterRole::Aquatic, vec!["monster_flipper"]),
        (
            MonsterRole::Boss,
            vec![
                "monster_supertank",
                "monster_boss2",
                "monster_boss3_stand",
                "monster_jorg",
                "monster_makron",
                "monster_guardian",
                "monster_boss5",
                "monster_carrier",
                "monster_widow",
                "monster_widow2",
            ],
        ),
        (
            MonsterRole::Special,
            vec![
                "monster_parasite",
                "monster_medic",
                "monster_medic_commander",
                "monster_fixbot",
                "monster_gekk",
                "monster_stalker",
                "monster_turret",
                "monster_kamikaze",
                "monster_tank_stand",
                "monster_commander_body",
            ],
        ),
    ]
}

/// Flatten role tables in role order, then sort by classname (`slots`).
fn roster_slots(roles: &[(MonsterRole, Vec<&'static str>)]) -> Vec<MonsterRosterSlot> {
    let mut slots: Vec<MonsterRosterSlot> = Vec::new();
    for role in ROLE_ORDER {
        if let Some((_, classnames)) = roles.iter().find(|(candidate, _)| *candidate == role) {
            for classname in classnames {
                slots.push(MonsterRosterSlot {
                    classname: (*classname).to_string(),
                    role,
                });
            }
        }
    }
    slots.sort_by(|left, right| left.classname.cmp(&right.classname));
    slots
}

/// Replaceable roster slots for a campaign family
/// (`campaignMonsterSlots`).
#[must_use]
pub fn campaign_monster_slots(family: GameFamily) -> Vec<MonsterRosterSlot> {
    match family {
        GameFamily::Q1 => roster_slots(&q1_roles()),
        GameFamily::Q2 => roster_slots(&q2_roles()),
        GameFamily::Q3 => Vec::new(),
    }
}

/// Preferred same-family replacement per role (`preferred`).
fn preferred_replacement(family: MonsterFamily, role: MonsterRole) -> Option<&'static str> {
    match (family, role) {
        (MonsterFamily::Q1, MonsterRole::Trooper) => Some("monster_army"),
        (MonsterFamily::Q1, MonsterRole::Ranged) => Some("monster_enforcer"),
        (MonsterFamily::Q1, MonsterRole::Melee) => Some("monster_demon1"),
        (MonsterFamily::Q1, MonsterRole::Leaper) => Some("monster_demon1"),
        (MonsterFamily::Q1, MonsterRole::Grenadier) => Some("monster_ogre"),
        (MonsterFamily::Q1, MonsterRole::Rocket) => Some("monster_shalrath"),
        (MonsterFamily::Q1, MonsterRole::Artillery) => Some("monster_shalrath"),
        (MonsterFamily::Q1, MonsterRole::Hybrid) => Some("monster_hell_knight"),
        (MonsterFamily::Q1, MonsterRole::Heavy) => Some("monster_shambler"),
        (MonsterFamily::Q1, MonsterRole::Flying) => Some("monster_wizard"),
        (MonsterFamily::Q1, MonsterRole::Aquatic) => Some("monster_fish"),
        (MonsterFamily::Q2, MonsterRole::Trooper) => Some("monster_soldier"),
        (MonsterFamily::Q2, MonsterRole::Ranged) => Some("monster_infantry"),
        (MonsterFamily::Q2, MonsterRole::Melee) => Some("monster_berserk"),
        (MonsterFamily::Q2, MonsterRole::Leaper) => Some("monster_mutant"),
        (MonsterFamily::Q2, MonsterRole::Grenadier) => Some("monster_gunner"),
        (MonsterFamily::Q2, MonsterRole::Rocket) => Some("monster_chick"),
        (MonsterFamily::Q2, MonsterRole::Artillery) => Some("monster_gladiator"),
        (MonsterFamily::Q2, MonsterRole::Hybrid) => Some("monster_brain"),
        (MonsterFamily::Q2, MonsterRole::Heavy) => Some("monster_tank"),
        (MonsterFamily::Q2, MonsterRole::Flying) => Some("monster_flyer"),
        (MonsterFamily::Q2, MonsterRole::Aquatic) => Some("monster_flipper"),
        _ => None,
    }
}

/// Default replacement roster mapping authored slots onto a source
/// (`defaultMonsterRoster`).
///
/// Cross-family dog/parasite slots swap; boss and special slots stay
/// map-defined; explicit `overrides` win.
pub fn default_monster_roster(
    authored_family: MonsterFamily,
    target: &ProviderReference,
    overrides: &HashMap<String, MonsterSelectionTarget>,
) -> Result<EnemySelection, MonsterError> {
    let selected = monster_sources()
        .into_iter()
        .find(|source| source.provider == target.provider)
        .ok_or_else(|| MonsterError::UnknownSource(provider_text(&target.provider)))?;
    let authored = match authored_family {
        MonsterFamily::Q1 => GameFamily::Q1,
        MonsterFamily::Q2 => GameFamily::Q2,
    };
    let mut by_classname: HashMap<String, MonsterSelectionTarget> = HashMap::new();
    for slot in campaign_monster_slots(authored) {
        let replacement: Option<&str> = if authored_family == MonsterFamily::Q2
            && selected.family == MonsterFamily::Q1
            && slot.classname == "monster_parasite"
        {
            Some("monster_dog")
        } else if authored_family == MonsterFamily::Q1
            && selected.family == MonsterFamily::Q2
            && slot.classname == "monster_dog"
        {
            Some("monster_parasite")
        } else if slot.role == MonsterRole::Boss || slot.role == MonsterRole::Special {
            None
        } else if selected.family == authored_family && selected.creatures.contains_key(&slot.classname) {
            Some(&slot.classname)
        } else {
            preferred_replacement(selected.family, slot.role)
        };
        let defined = replacement.is_some_and(|classname| selected.creatures.contains_key(classname));
        by_classname.insert(
            slot.classname.clone(),
            if defined {
                MonsterSelectionTarget::Defined(MonsterDefinitionReference {
                    source: target.clone(),
                    classname: replacement.unwrap_or_default().to_string(),
                })
            } else {
                MonsterSelectionTarget::MapDefined
            },
        );
    }
    for (classname, target) in overrides {
        by_classname.insert(classname.clone(), target.clone());
    }
    Ok(EnemySelection::Replace {
        default: MonsterSelectionTarget::MapDefined,
        by_classname,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ContentId;
    use qa_core::identity::IdentityOwner;

    fn provider_reference(provider: &str) -> ProviderReference {
        ProviderReference {
            provider: provider_id(provider),
            content: ContentId("q1:classic:id1:v1".to_string()),
        }
    }

    fn roster_map(selection: &EnemySelection) -> &HashMap<String, MonsterSelectionTarget> {
        match selection {
            EnemySelection::Replace { by_classname, .. } => by_classname,
            EnemySelection::MapDefined => panic!("expected a replace roster"),
        }
    }

    #[test]
    fn target_eligibility_needs_health_and_targeting() {
        let seen = MonsterTargetObservation {
            view_height: 22.0,
            notarget: false,
            invisible: false,
            light_level: None,
            hostile_until: None,
        };
        assert!(monster_target_eligible(10.0, Some(&seen)));
        assert!(!monster_target_eligible(0.0, Some(&seen)));
        assert!(!monster_target_eligible(10.0, None));
        let hidden = MonsterTargetObservation { notarget: true, ..seen };
        assert!(!monster_target_eligible(10.0, Some(&hidden)));
    }

    #[test]
    fn sources_bind_all_programs() {
        let sources = monster_sources();
        assert_eq!(sources.len(), 16);
        let providers: Vec<String> = sources.iter().map(|source| provider_text(&source.provider)).collect();
        assert!(providers.contains(&"q1:monsters/classic/id1".to_string()));
        assert!(providers.contains(&"q1:monsters/rerelease/mg3".to_string()));
        assert!(providers.contains(&"q2:monsters/classic/xatrix".to_string()));
        assert!(providers.contains(&"q2:monsters/rerelease/mg2".to_string()));
        let q1 = sources
            .iter()
            .find(|source| provider_text(&source.provider) == "q1:monsters/classic/id1")
            .expect("q1 id1");
        assert_eq!(q1.creatures.len(), 14);
        assert_eq!(
            q1.creatures.get("monster_ogre_marksman"),
            q1.creatures.get("monster_ogre")
        );
    }

    #[test]
    fn source_lookup_reports_unavailable() {
        let target = provider_reference("q1:monsters/classic/id1");
        let found = monster_source(&MonsterDefinitionReference {
            source: target,
            classname: "monster_ogre".to_string(),
        })
        .expect("ogre resolves");
        assert_eq!(found.program, MonsterProgram::Id1);
        let missing = monster_source(&MonsterDefinitionReference {
            source: provider_reference("q1:monsters/classic/id1"),
            classname: "monster_boss".to_string(),
        });
        assert_eq!(
            missing,
            Err(MonsterError::Unavailable {
                provider: "q1:monsters/classic/id1".to_string(),
                classname: "monster_boss".to_string(),
            })
        );
        let unknown = monster_source(&MonsterDefinitionReference {
            source: provider_reference("q9:monsters/classic/id1"),
            classname: "monster_ogre".to_string(),
        });
        assert!(matches!(unknown, Err(MonsterError::Unavailable { .. })));
    }

    #[test]
    fn timing_selects_family_and_edition_clocks() {
        let q1 = q1_monster_sources().remove(0);
        let timing = monster_timing(&q1);
        assert_eq!(timing.numeric, Q1_DONOR_PROFILE);
        assert_eq!(
            timing.clock,
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.001,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None,
            }
        );
        let q2 = q2_monster_sources();
        let classic = monster_timing(&q2[0]);
        assert_eq!(classic.numeric, Q2_DONOR_PROFILE);
        assert_eq!(classic.clock, ClockProfile::Q2Classic);
        let rerelease = monster_timing(&q2[1]);
        assert_eq!(
            rerelease.clock,
            ClockProfile::Q2Rerelease {
                frame_milliseconds: 25.0
            }
        );
    }

    #[test]
    fn q2_tables_expand_with_precaches() {
        let classic = q2_expanded_classic_creatures();
        assert_eq!(classic.len(), 5);
        let medic = classic.get("monster_medic").expect("classic medic");
        assert!(medic
            .resources
            .contains(&"models/objects/gibs/head2/tris.md2".to_string()));
        let base = q2_expanded_base_creatures();
        assert_eq!(base.len(), 22);
        let rerelease_medic = base.get("monster_medic").expect("rerelease medic");
        assert!(rerelease_medic
            .resources
            .contains(&"sound/player/step1.wav".to_string()));
        assert!(rerelease_medic
            .resources
            .contains(&"sound/player/step4.wav".to_string()));
        let turret = base.get("monster_turret").expect("turret");
        assert!(turret.resources.contains(&"sound/world/dr_short.wav".to_string()));
        assert!(turret.resources.contains(&"sound/misc/lasfly.wav".to_string()));
        assert!(turret.resources.contains(&"sound/weapons/rockfly.wav".to_string()));
        let makron = base.get("monster_makron").expect("makron");
        assert!(makron.resources.contains(&"sprites/s_bfg1.sp2".to_string()));
        assert!(makron.resources.contains(&"sprites/s_bfg3.sp2".to_string()));
        let jorg = base.get("monster_jorg").expect("jorg");
        assert!(jorg.resources.iter().any(|path| path.contains("makron")));
        let commander = base.get("monster_medic_commander").expect("commander");
        assert!(commander.resources.iter().any(|path| path.contains("soldier")));
        assert!(base.contains_key("monster_arachnid"));
        assert!(!base.contains_key("monster_soldier_ripper"));
        let merged = monster_sources()
            .into_iter()
            .find(|source| provider_text(&source.provider) == "q2:monsters/rerelease/baseq2")
            .expect("merged rerelease base");
        assert_eq!(merged.creatures.len(), 39);
        let expansions = q2_expansion_sources();
        assert_eq!(expansions.len(), 5);
    }

    #[test]
    fn mg3_addon_unions_base_resources() {
        let resources = mg3_monster_resources();
        assert_eq!(resources.len(), 14);
        let addons = q1_addon_monster_sources();
        assert_eq!(addons.len(), 3);
        let mg3 = addons
            .iter()
            .find(|source| source.program == MonsterProgram::Mg3)
            .expect("mg3");
        assert_eq!(mg3.creatures.len(), 14 + 14);
        let knight = mg3.creatures.get("monster_ranged_knight").expect("ranged knight");
        assert!(knight.resources.contains(&"progs/rknight.mdl".to_string()));
        assert!(knight.resources.contains(&"progs/soldier.mdl".to_string()));
        let expansions = q1_expansion_monster_sources();
        assert_eq!(expansions.len(), 4);
        let rogue = expansions
            .iter()
            .find(|source| source.program == MonsterProgram::Rogue && source.edition == SourceEdition::Classic)
            .expect("classic rogue");
        assert_eq!(rogue.creatures.len(), 14 + 6);
        assert!(rogue.creatures.contains_key("monster_lava_man"));
    }

    #[test]
    fn literal_resource_counts_match_donor() {
        let q1 = &q1_monster_sources()[0].creatures;
        for (classname, count) in [
            ("monster_army", 14),
            ("monster_dog", 9),
            ("monster_enforcer", 19),
            ("monster_knight", 12),
            ("monster_demon1", 11),
            ("monster_ogre", 19),
            ("monster_ogre_marksman", 19),
            ("monster_hell_knight", 17),
            ("monster_shambler", 18),
            ("monster_wizard", 12),
            ("monster_shalrath", 15),
            ("monster_tarbaby", 5),
            ("monster_zombie", 16),
            ("monster_fish", 4),
        ] {
            assert_eq!(
                q1.get(classname).expect(classname).resources.len(),
                count,
                "{classname}"
            );
        }
        let mg3 = mg3_monster_resources();
        for (classname, count) in [
            ("monster_ogre_rocket", 22),
            ("monster_demodog", 11),
            ("monster_army_infected", 25),
            ("monster_knight_infected", 25),
            ("monster_enforcer_infected", 25),
            ("monster_hell_knight_infected", 26),
            ("monster_ranged_knight", 22),
            ("monster_super_shambler", 22),
            ("monster_lava_man", 13),
            ("monster_ghost", 4),
            ("monster_orb", 15),
            ("monster_szombie", 18),
            ("monster_oldone_new", 23),
            ("monster_boss_final", 20),
        ] {
            assert_eq!(
                mg3.get(classname).expect(classname).resources.len(),
                count,
                "{classname}"
            );
        }
        let expansions = q1_expansion_monster_sources();
        let hipnotic = expansions
            .iter()
            .find(|source| source.program == MonsterProgram::Hipnotic && source.edition == SourceEdition::Classic)
            .expect("classic hipnotic");
        for (classname, count) in [
            ("monster_scourge", 15),
            ("monster_gremlin", 33),
            ("monster_armagon", 18),
        ] {
            assert_eq!(
                hipnotic.creatures.get(classname).expect(classname).resources.len(),
                count,
                "{classname}"
            );
        }
        let rogue = expansions
            .iter()
            .find(|source| source.program == MonsterProgram::Rogue && source.edition == SourceEdition::Classic)
            .expect("classic rogue");
        for (classname, count) in [
            ("monster_eel", 10),
            ("monster_sword", 8),
            ("monster_wrath", 14),
            ("monster_mummy", 12),
            ("monster_super_wrath", 17),
            ("monster_lava_man", 4),
        ] {
            assert_eq!(
                rogue.creatures.get(classname).expect(classname).resources.len(),
                count,
                "{classname}"
            );
        }
    }

    #[test]
    fn campaign_slots_cover_both_families() {
        let q1 = campaign_monster_slots(GameFamily::Q1);
        assert_eq!(q1.len(), 44);
        let names: Vec<&str> = q1.iter().map(|slot| slot.classname.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        let q2 = campaign_monster_slots(GameFamily::Q2);
        assert_eq!(q2.len(), 45);
        assert!(campaign_monster_slots(GameFamily::Q3).is_empty());
    }

    #[test]
    fn default_roster_prefers_same_family_then_overrides() {
        let target = provider_reference("q1:monsters/classic/id1");
        let roster = default_monster_roster(MonsterFamily::Q1, &target, &HashMap::new()).expect("q1 roster");
        let map = roster_map(&roster);
        assert_eq!(map.len(), 44);
        assert_eq!(
            map.get("monster_army"),
            Some(&MonsterSelectionTarget::Defined(MonsterDefinitionReference {
                source: target.clone(),
                classname: "monster_army".to_string(),
            }))
        );
        assert_eq!(map.get("monster_boss"), Some(&MonsterSelectionTarget::MapDefined));
        let mut overrides = HashMap::new();
        overrides.insert("monster_army".to_string(), MonsterSelectionTarget::MapDefined);
        let overridden = default_monster_roster(MonsterFamily::Q1, &target, &overrides).expect("overridden");
        assert_eq!(
            roster_map(&overridden).get("monster_army"),
            Some(&MonsterSelectionTarget::MapDefined)
        );
        let unknown = default_monster_roster(MonsterFamily::Q1, &provider_reference("q9:x"), &HashMap::new());
        assert_eq!(unknown, Err(MonsterError::UnknownSource("q9:x".to_string())));
    }

    #[test]
    fn default_roster_swaps_dog_and_parasite_across_families() {
        let q2_target = provider_reference("q2:monsters/classic/baseq2");
        let q1_on_q2 = default_monster_roster(MonsterFamily::Q1, &q2_target, &HashMap::new()).expect("q1 on q2");
        assert_eq!(
            roster_map(&q1_on_q2).get("monster_dog"),
            Some(&MonsterSelectionTarget::Defined(MonsterDefinitionReference {
                source: q2_target.clone(),
                classname: "monster_parasite".to_string(),
            }))
        );
        assert_eq!(
            roster_map(&q1_on_q2).get("monster_army"),
            Some(&MonsterSelectionTarget::Defined(MonsterDefinitionReference {
                source: q2_target.clone(),
                classname: "monster_soldier".to_string(),
            }))
        );
        let q1_target = provider_reference("q1:monsters/classic/id1");
        let q2_on_q1 = default_monster_roster(MonsterFamily::Q2, &q1_target, &HashMap::new()).expect("q2 on q1");
        assert_eq!(
            roster_map(&q2_on_q1).get("monster_parasite"),
            Some(&MonsterSelectionTarget::Defined(MonsterDefinitionReference {
                source: q1_target.clone(),
                classname: "monster_dog".to_string(),
            }))
        );
    }

    struct RecordingMission {
        log: Vec<String>,
        goal: Option<ActorId>,
        combat: Option<ActorId>,
    }

    impl MonsterMission for RecordingMission {
        fn spawned(&mut self) {
            self.log.push("spawned".to_string());
        }

        fn started(&mut self) {
            self.log.push("started".to_string());
        }

        fn killed(&mut self, attacker: Option<&ActorId>) {
            self.log.push(format!("killed:{}", attacker.is_some()));
        }

        fn route(&self) -> Option<ActorId> {
            self.goal.clone()
        }

        fn r#use(&mut self, activator: Option<&ActorId>) -> bool {
            self.log.push(format!("use:{}", activator.is_some()));
            true
        }

        fn combat_route(&self) -> CombatRoute {
            CombatRoute {
                goal: self.combat.clone(),
                stand_ground: true,
            }
        }

        fn found_target(&mut self) {
            self.log.push("found".to_string());
        }
    }

    #[test]
    fn authored_monster_holds_mission_state() {
        let owner = IdentityOwner::create("test").expect("owner");
        let actor = owner.actor(1, 0);
        let owned = owner
            .owned_actor(&actor, ProviderId::new("q1", "gameplay"))
            .expect("owned");
        let monster = AuthoredMonster {
            target: AuthoredTarget {
                actor: owned,
                classname: "monster_ogre".to_string(),
                targetname: String::new(),
                target: String::new(),
                killtarget: String::new(),
                message: String::new(),
                delay: 0.0,
            },
            source_ordinal: 7,
            spawnflags: 3,
            death_target: String::new(),
            drop_item: String::new(),
            route: "patrol".to_string(),
            route_goal: None,
            route_resolved: false,
            counted_death: false,
            combat_target: String::new(),
            combat_goal: None,
            stand_ground: false,
            placement: MonsterPlacement::Waiting {
                barriers: vec![MonsterBarrier {
                    actor: actor.clone(),
                    origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                }],
                activator: None,
            },
            activation: MonsterActivation::Scheduled {
                at: 1.5,
                activator: Some(actor.clone()),
            },
        };
        assert_eq!(monster.source_ordinal, 7);
        let mut mission = RecordingMission {
            log: Vec::new(),
            goal: Some(actor.clone()),
            combat: None,
        };
        mission.spawned();
        mission.started();
        mission.killed(Some(&actor));
        assert!(mission.r#use(None));
        mission.found_target();
        assert_eq!(mission.route(), Some(actor));
        assert!(mission.combat_route().stand_ground);
        assert_eq!(mission.log.len(), 5);
        assert!(matches!(MonsterPlacement::Ready, MonsterPlacement::Ready));
        let teleport = MonsterPlacement::Teleport {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
        };
        assert!(matches!(teleport, MonsterPlacement::Teleport { .. }));
        assert!(matches!(MonsterActivation::Active, MonsterActivation::Active));
        assert!(matches!(MonsterActivation::Dormant, MonsterActivation::Dormant));
    }
}
