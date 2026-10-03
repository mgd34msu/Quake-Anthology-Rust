//! Selected-monster runtime across Q1 and Q2 map programs.
//!
//! Absolute donor:
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/monster-runtime.ts`
//!
//! Admission, patrol/combat mission state, triggered-monster activation, and
//! release tracking for selected monsters. The map program is the real
//! content services: donor `Q1Foundation`/`Q2Foundation` map state merged
//! into [`Q1EntityServices`]/[`Q2GameServices`] during the content ports, so
//! [`MonsterMap`] holds those services plus the Q2 item module. Two donor
//! mechanisms adapt to Rust ownership:
//!
//! * The donor registers an actor-release callback in its constructor; the
//!   content registries expose no release subscription, so the session
//!   forwards releases to [`SelectedMonsters::note_released`] (the same
//!   pattern as `Q2GameServices::on_actor_released`).
//! * The donor hands its behavior a retained mission object at attach time;
//!   retained `&mut` borrows cannot work that way, so the behavior seam
//!   drops the mission parameter and the session dispatches content-AI
//!   callbacks through the public `monster_*` methods (or a short-lived
//!   [`SelectedMonsterMission`] implementing [`MonsterMission`]).
//! * The donor emits `monster-killed` on the Q1 host, but the content
//!   `Q1Event` has no such variant, so [`MonsterMap::Q1`] carries a
//!   [`Q1MonsterKillSink`] (value seam citing the donor mission killed
//!   branch) instead of touching content event types.

use std::collections::{BTreeMap, HashMap};

use qa_content::contract::{EnemySelection, MonsterDefinitionReference, MonsterSelectionTarget, ProviderReference};
use qa_content::monsters::{
    provider_text, AuthoredMonster, AuthoredTarget, CombatRoute, MonsterActivation, MonsterBarrier, MonsterMission,
    MonsterPlacement,
};
use qa_content::q1::addons::context::Q1AddonContext;
use qa_content::q1::foundation::entity_services::Q1EntityServices;
use qa_content::q1::foundation::gameplay::CombatState as Q1CombatState;
use qa_content::q1::foundation::types::Q1Edition;
use qa_content::q2::foundation::host::{Q2Edition, Q2GameServices, Q2SpawnFields};
use qa_content::q2::foundation::items::Q2ItemModule;
use qa_content::q2::foundation::monsters::place_triggered_monster;
use qa_content::q2::support::contracts::CombatTraitChanges;
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;
use qa_core::time::FrameContext;
use qa_world::WorldError;

use super::monster_checkpoint::{
    SavedAuthoredMonster, SavedMonsterActivation, SavedMonsterBarrier, SavedMonsterPlacement,
};
use super::q2_monster_sources::SelectedQ2MonsterPack;
use super::random::SourceRandom;
use qa_content::bsp::Q1Entity;

/// Per-source frame clock plus its advance latch.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceClock {
    /// Current source frame.
    pub frame: FrameContext,
    /// Whether the frame already advanced.
    pub advanced: bool,
}

/// Selected-monster source (donor `SelectedMonsterSource`).
///
/// The variants hold whole games; they move rarely (once per source), so the
/// size difference is accepted rather than boxed.
#[allow(clippy::large_enum_variant)]
pub enum SelectedMonsterSource {
    /// Quake source over its own entity services.
    Q1 {
        /// Defining provider.
        reference: ProviderReference,
        /// Source RNG stream.
        random: SourceRandom,
        /// Source frame clock.
        clock: SourceClock,
        /// Source entity services.
        game: Q1EntityServices,
        /// Addon context, for addon programs.
        addon: Option<Q1AddonContext>,
    },
    /// Quake II source over its own game services.
    Q2 {
        /// Defining provider.
        reference: ProviderReference,
        /// Source RNG stream.
        random: SourceRandom,
        /// Source frame clock.
        clock: SourceClock,
        /// Source game services.
        game: Q2GameServices,
        /// Registered packs.
        packs: Vec<SelectedQ2MonsterPack>,
    },
}

impl SelectedMonsterSource {
    /// Q1 frame parts for the simulation step tail (C6).
    pub fn q1_frame_parts(
        &mut self,
    ) -> Option<(
        &ProviderReference,
        &mut SourceClock,
        &mut Q1EntityServices,
        &Option<Q1AddonContext>,
    )> {
        match self {
            SelectedMonsterSource::Q1 {
                reference,
                clock,
                game,
                addon,
                ..
            } => Some((reference, clock, game, addon)),
            _ => None,
        }
    }

    /// Q2 frame parts for the simulation step tail (C6).
    pub fn q2_frame_parts(&mut self) -> Option<(&mut SourceClock, &mut Q2GameServices)> {
        match self {
            SelectedMonsterSource::Q2 { clock, game, .. } => Some((clock, game)),
            _ => None,
        }
    }

    /// Monster clock for execution-frame reads (C6).
    pub fn clock(&self) -> &SourceClock {
        match self {
            SelectedMonsterSource::Q1 { clock, .. } => clock,
            SelectedMonsterSource::Q2 { clock, .. } => clock,
        }
    }
}

/// Session bridge for monster AI behavior (donor `SelectedMonsterBehavior`).
///
/// The donor passes a retained mission object to `attach`; this seam drops
/// that parameter (see the module docs) and the session dispatches AI
/// callbacks through [`SelectedMonsters`] instead.
pub trait SelectedMonsterBehavior {
    /// Attach AI state for an admitted monster.
    fn attach(&mut self, actor: &OwnedActor, definition: &MonsterDefinitionReference);
    /// Whether the session accepts the monster's placement.
    fn validate_placement(
        &mut self,
        entry: &AuthoredMonster,
        definition: &MonsterDefinitionReference,
        relocated: Option<Vec3>,
    ) -> bool;
    /// Whether a dormant monster may wake.
    fn placement_ready(&mut self, entry: &AuthoredMonster) -> bool;
    /// Current enemy, if any.
    fn enemy(&mut self, actor: &ActorId) -> Option<ActorId>;
    /// Previous enemy, if any.
    fn old_enemy(&mut self, actor: &ActorId) -> Option<ActorId>;
    /// Retarget a patrol route.
    fn set_route(&mut self, actor: &ActorId, goal: Option<ActorId>, pause_until: f64);
    /// Wake a dormant monster.
    fn resume(&mut self, actor: &ActorId, activator: Option<ActorId>);
}

/// Q1 monster-kill notification sink.
///
/// Value seam for the donor `host.emit({ kind: "monster-killed", ... })` call
/// in the mission `killed` branch of
/// `src/app/bootstrap/simulation/monster-runtime.ts`;
/// the content `Q1Event` has no such variant, so the session implements this
/// instead of duplicating presentation routing.
pub trait Q1MonsterKillSink {
    /// Report a counted Q1 monster death.
    fn monster_killed(&mut self, actor: &ActorId, total: i32, found: i32);
}

/// Map program running selected monsters (donor `MonsterMap`).
///
/// See [`SelectedMonsterSource`] for why the game-holding variants are not
/// boxed.
#[allow(clippy::large_enum_variant)]
pub enum MonsterMap {
    /// Quake map program.
    Q1 {
        /// Map entity services.
        game: Q1EntityServices,
        /// Kill notification sink.
        kills: Box<dyn Q1MonsterKillSink>,
    },
    /// Quake II map program.
    Q2 {
        /// Map game services.
        game: Q2GameServices,
        /// Item module for monster drops.
        items: Q2ItemModule,
    },
}

/// Ordinary Q1 selection classnames (donor `q1Ordinary`).
const Q1_ORDINARY: &[&str] = &[
    "monster_army",
    "monster_boss",
    "monster_demon1",
    "monster_dog",
    "monster_enforcer",
    "monster_fish",
    "monster_hell_knight",
    "monster_knight",
    "monster_ogre",
    "monster_ogre_marksman",
    "monster_oldone",
    "monster_shalrath",
    "monster_shambler",
    "monster_tarbaby",
    "monster_wizard",
    "monster_zombie",
    "monster_gremlin",
    "monster_scourge",
    "monster_dragon",
    "monster_eel",
    "monster_golem_bronze",
    "monster_golem_gold",
    "monster_golem_iron",
    "monster_golem_stone",
    "monster_guard",
    "monster_spike_mine",
    "monster_were-bear",
    "monster_were-jaguar",
];

/// Ordinary Q2 selection classnames (donor `q2Ordinary`).
const Q2_ORDINARY: &[&str] = &[
    "monster_actor",
    "monster_berserk",
    "monster_brain",
    "monster_chick",
    "monster_flipper",
    "monster_floater",
    "monster_flyer",
    "monster_gladiator",
    "monster_gunner",
    "monster_hover",
    "monster_infantry",
    "monster_insane",
    "monster_medic",
    "monster_mutant",
    "monster_parasite",
    "monster_soldier",
    "monster_supertank",
    "monster_tank",
    "monster_arachnid",
    "monster_carrier",
    "monster_widow",
    "monster_widow2",
    "monster_stalker",
    "monster_daedalus",
];

/// Mirror of `Q2PathFollower` from donor
/// `src/content/q2/foundation/monsters/index.ts` (canonical home:
/// `qa_content::q2::foundation::monsters`; unify post-merge).
pub trait Q2PathFollower {
    /// Following actor.
    fn actor(&self) -> &OwnedActor;
    /// Resolved patrol goal snapshot.
    fn move_target(&self) -> Option<ActorId>;
    /// Current enemy snapshot.
    fn enemy(&self) -> Option<ActorId>;
    /// Retarget the patrol route.
    fn advance(&mut self, name: &str, goal: Option<ActorId>, pause_until: f64);
}

/// Mirror of `Q2CombatFollower` from donor
/// `src/content/q2/foundation/monsters/index.ts` (canonical home:
/// `qa_content::q2::foundation::monsters`; unify post-merge).
pub trait Q2CombatFollower {
    /// Live combat goal.
    fn move_target(&self) -> Option<ActorId>;
    /// Current enemy snapshot.
    fn enemy(&self) -> Option<ActorId>;
    /// Previous enemy snapshot.
    fn old_enemy(&self) -> Option<ActorId>;
    /// Scheduling activator snapshot.
    fn activator(&self) -> Option<ActorId>;
    /// Combat followers always walk.
    fn walking(&self) -> bool;
    /// Advance the combat target.
    fn advance(&mut self, target: &str, goal: Option<ActorId>, move_target: Option<ActorId>);
    /// Hold position.
    fn hold(&mut self);
    /// Finish the combat run.
    fn finish(&mut self);
}

/// Selected monsters over one map program (donor `SelectedMonsters`).
pub struct SelectedMonsters {
    /// Default replacement.
    pub selection_default: MonsterSelectionTarget,
    /// Per-classname replacements.
    pub selection_by_classname: HashMap<String, MonsterSelectionTarget>,
    /// Admitted monsters by actor.
    pub authored: HashMap<ActorId, AuthoredMonster>,
    /// Selected definitions by actor.
    pub definitions: HashMap<ActorId, MonsterDefinitionReference>,
}

fn saved_id(actor: &ActorId) -> SavedActorId {
    SavedActorId {
        slot: actor.slot(),
        generation: actor.generation(),
    }
}

/// Parse a donor `Number(...)` field: decimal text, missing or invalid reads
/// as zero.
fn number_field(fields: &BTreeMap<String, String>, key: &str) -> f64 {
    fields
        .get(key)
        .map(|value| value.trim().parse::<f64>().unwrap_or(0.0))
        .unwrap_or(0.0)
}

fn bad_save(message: impl Into<String>) -> WorldError {
    WorldError::BadSave(message.into())
}

/// Flip Q1 damage handling, preserving the rest of the combat traits.
fn set_q1_can_take_damage(
    game: &mut Q1EntityServices,
    actor: &OwnedActor,
    can_take_damage: bool,
) -> Result<(), WorldError> {
    let state = q1_combat(game, actor.id())?;
    game.host
        .combat
        .set_traits(
            actor,
            qa_content::q1::foundation::gameplay::CombatTraits {
                can_take_damage,
                mass: state.mass,
                invulnerable: state.invulnerable,
                team: state.team.clone(),
                no_knockback: state.no_knockback,
            },
        )
        .map_err(|error| bad_save(error.to_string()))
}

/// Flip Q2 damage handling through the partial trait update.
fn set_q2_can_take_damage(game: &mut Q2GameServices, actor: &OwnedActor, can_take_damage: bool) {
    game.host.combat().set_traits(
        actor,
        &CombatTraitChanges {
            can_take_damage: Some(can_take_damage),
            ..Default::default()
        },
    );
}

/// Read Q1 combat state for an admitted monster.
fn q1_combat(game: &Q1EntityServices, actor: &ActorId) -> Result<Q1CombatState, WorldError> {
    game.host.combat.read(actor).ok_or(WorldError::BodyMissing)
}

impl SelectedMonsters {
    /// Select replacement monsters (donor constructor; the map and behavior
    /// arrive per call because the runtime cannot retain their borrows).
    pub fn new(selection: &EnemySelection) -> Result<Self, WorldError> {
        match selection {
            EnemySelection::Replace { default, by_classname } => Ok(Self {
                selection_default: default.clone(),
                selection_by_classname: by_classname.clone(),
                authored: HashMap::new(),
                definitions: HashMap::new(),
            }),
            EnemySelection::MapDefined => Err(WorldError::BadSpawnFields(
                "enemy selection must replace monsters".to_string(),
            )),
        }
    }

    /// Resolve the selected definition for a classname, or `None` when the
    /// map keeps its authored monster.
    pub fn resolve(
        &self,
        map: &MonsterMap,
        classname: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<Option<MonsterDefinitionReference>, WorldError> {
        let target = self
            .selection_by_classname
            .get(classname)
            .unwrap_or(&self.selection_default);
        let MonsterSelectionTarget::Defined(definition) = target else {
            return Ok(None);
        };
        if !classname.starts_with("monster_") {
            return Ok(None);
        }
        let ordinary = match map {
            MonsterMap::Q1 { .. } => Q1_ORDINARY.contains(&classname),
            MonsterMap::Q2 { .. } => Q2_ORDINARY.contains(&classname),
        };
        if !ordinary {
            return Err(WorldError::UnknownSpawnClass(format!(
                "Selected monster admission does not yet preserve authored {classname} obligations"
            )));
        }
        let flags = number_field(fields, "spawnflags") as i32;
        let inhibition = 0xf00
            | 0xff00
            | (if matches!(map, MonsterMap::Q2 { .. }) {
                0x1f00
            } else {
                0
            });
        if flags & !inhibition & !(if classname == "monster_zombie" { 1 } else { 3 }) != 0 {
            return Err(WorldError::BadSpawnFields(format!(
                "Selected monster admission does not yet preserve {classname} spawn flags {}",
                number_field(fields, "spawnflags")
            )));
        }
        Ok(Some(definition.clone()))
    }

    /// Whether an actor is an admitted monster.
    #[must_use]
    pub fn active(&self, actor: &ActorId) -> bool {
        self.authored.contains_key(actor)
    }
}

impl SelectedMonsters {
    /// Admit a Q1 monster from its authored source entity.
    pub fn admit_q1<B: SelectedMonsterBehavior>(
        &mut self,
        map: &mut MonsterMap,
        behavior: &mut B,
        actor: OwnedActor,
        source: &Q1Entity,
        ordinal: i32,
        definition: &MonsterDefinitionReference,
    ) -> Result<(), WorldError> {
        let fields: BTreeMap<String, String> = source.properties.iter().cloned().collect();
        let MonsterMap::Q1 { game, .. } = map else {
            return Err(bad_save("Q1 monster admission requires its Q1 map program"));
        };
        self.admit(game, behavior, actor, &fields, ordinal, definition)
    }

    /// Admit a Q2 monster from its authored spawn fields.
    pub fn admit_q2<B: SelectedMonsterBehavior>(
        &mut self,
        map: &mut MonsterMap,
        behavior: &mut B,
        actor: OwnedActor,
        source: &Q2SpawnFields,
        definition: &MonsterDefinitionReference,
    ) -> Result<(), WorldError> {
        let MonsterMap::Q2 { game, .. } = map else {
            return Err(bad_save("Q2 monster admission requires its Q2 map program"));
        };
        self.admit_q2_inner(game, behavior, actor, source, definition)
    }

    /// Shared admission: build the authored entry, pin dormant monsters, and
    /// attach behavior.
    fn admit<B: SelectedMonsterBehavior>(
        &mut self,
        game: &mut Q1EntityServices,
        behavior: &mut B,
        actor: OwnedActor,
        fields: &BTreeMap<String, String>,
        ordinal: i32,
        definition: &MonsterDefinitionReference,
    ) -> Result<(), WorldError> {
        let entry = admitted_entry(&actor, fields, ordinal);
        if !self.authored.contains_key(actor.id()) {
            set_q1_can_take_damage(game, &actor, false)?;
            game.host
                .bodies
                .link(&actor)
                .map_err(|error| bad_save(error.to_string()))?;
        }
        behavior.attach(&actor, definition);
        game.authored_targets.insert(actor.id().clone(), entry.target.clone());
        self.authored.insert(actor.id().clone(), entry);
        self.definitions.insert(actor.id().clone(), definition.clone());
        Ok(())
    }

    /// Q2 admission over game services.
    fn admit_q2_inner<B: SelectedMonsterBehavior>(
        &mut self,
        game: &mut Q2GameServices,
        behavior: &mut B,
        actor: OwnedActor,
        source: &Q2SpawnFields,
        definition: &MonsterDefinitionReference,
    ) -> Result<(), WorldError> {
        let entry = admitted_entry(&actor, &source.values, source.ordinal);
        if !self.authored.contains_key(actor.id()) {
            set_q2_can_take_damage(game, &actor, false);
            game.host.bodies().link(&actor, None);
        }
        behavior.attach(&actor, definition);
        game.authored_targets.insert(actor.id().clone(), entry.target.clone());
        self.authored.insert(actor.id().clone(), entry);
        self.definitions.insert(actor.id().clone(), definition.clone());
        Ok(())
    }

    /// Capture authored monsters into checkpoint records.
    pub fn capture(&self) -> Result<Vec<SavedAuthoredMonster>, WorldError> {
        let mut saved = Vec::with_capacity(self.authored.len());
        for (id, entry) in &self.authored {
            let Some(definition) = self.definitions.get(id) else {
                return Err(bad_save("Authored monster has no selected definition"));
            };
            saved.push(SavedAuthoredMonster {
                target: super::monster_checkpoint::SavedAuthoredTarget {
                    actor: saved_id(entry.target.actor.id()),
                    classname: entry.target.classname.clone(),
                    targetname: entry.target.targetname.clone(),
                    target: entry.target.target.clone(),
                    killtarget: entry.target.killtarget.clone(),
                    message: entry.target.message.clone(),
                    delay: entry.target.delay,
                },
                definition: definition.clone(),
                source_ordinal: entry.source_ordinal,
                spawnflags: entry.spawnflags,
                death_target: entry.death_target.clone(),
                drop_item: entry.drop_item.clone(),
                route: entry.route.clone(),
                route_goal: entry.route_goal.as_ref().map(saved_id),
                route_resolved: entry.route_resolved,
                counted_death: entry.counted_death,
                combat_target: entry.combat_target.clone(),
                combat_goal: entry.combat_goal.as_ref().map(saved_id),
                stand_ground: entry.stand_ground,
                placement: match &entry.placement {
                    MonsterPlacement::Ready => SavedMonsterPlacement::Ready,
                    MonsterPlacement::Teleport { origin } => SavedMonsterPlacement::Teleport { origin: *origin },
                    MonsterPlacement::Waiting { barriers, activator } => SavedMonsterPlacement::Waiting {
                        barriers: barriers
                            .iter()
                            .map(|barrier| SavedMonsterBarrier {
                                actor: saved_id(&barrier.actor),
                                origin: barrier.origin,
                            })
                            .collect(),
                        activator: activator.as_ref().map(saved_id),
                    },
                },
                activation: match &entry.activation {
                    MonsterActivation::Active => SavedMonsterActivation::Active,
                    MonsterActivation::Dormant => SavedMonsterActivation::Dormant,
                    MonsterActivation::Scheduled { at, activator } => SavedMonsterActivation::Scheduled {
                        at: *at,
                        activator: activator.as_ref().map(saved_id),
                    },
                },
            });
        }
        Ok(saved)
    }

    /// Restore authored monsters from checkpoint records.
    pub fn restore(&mut self, map: &mut MonsterMap, entries: &[SavedAuthoredMonster]) -> Result<(), WorldError> {
        for saved in entries {
            let actor = self.restore_actor(map, saved)?;
            let placement = match &saved.placement {
                SavedMonsterPlacement::Ready => MonsterPlacement::Ready,
                SavedMonsterPlacement::Teleport { origin } => MonsterPlacement::Teleport { origin: *origin },
                SavedMonsterPlacement::Waiting { barriers, activator } => {
                    if saved.target.targetname.is_empty() || barriers.is_empty() {
                        return Err(bad_save("Saved waiting monster has no authored door encounter"));
                    }
                    let live_barriers: Vec<(ActorId, qa_core::math::Vec3)> = barriers
                        .iter()
                        .map(|barrier| (restore_live_id(map, &barrier.actor), barrier.origin))
                        .collect();
                    {
                        let MonsterMap::Q2 { game, .. } = map else {
                            return Err(bad_save("Saved waiting monster has no authored door encounter"));
                        };
                        for (actor, _) in &live_barriers {
                            match game.entities.get(actor) {
                                Some(door)
                                    if door.classname == "func_door" && door.targetname == saved.target.targetname => {}
                                Some(_) => {
                                    return Err(bad_save(
                                        "Saved waiting monster barrier differs from its authored encounter",
                                    ));
                                }
                                None => {
                                    let live = game.host.actors().is_live(actor);
                                    if !live {
                                        return Err(bad_save(
                                            "Saved waiting monster barrier differs from its authored encounter",
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    MonsterPlacement::Waiting {
                        barriers: live_barriers
                            .into_iter()
                            .map(|(actor, origin)| MonsterBarrier { actor, origin })
                            .collect(),
                        activator: activator.as_ref().map(|id| restore_live_id(map, id)),
                    }
                }
            };
            let entry = AuthoredMonster {
                target: AuthoredTarget {
                    actor: actor.clone(),
                    classname: saved.target.classname.clone(),
                    targetname: saved.target.targetname.clone(),
                    target: saved.target.target.clone(),
                    killtarget: saved.target.killtarget.clone(),
                    message: saved.target.message.clone(),
                    delay: saved.target.delay,
                },
                source_ordinal: saved.source_ordinal,
                spawnflags: saved.spawnflags,
                death_target: saved.death_target.clone(),
                drop_item: saved.drop_item.clone(),
                route: saved.route.clone(),
                route_goal: saved.route_goal.map(|id| restore_live_id(map, &id)),
                route_resolved: saved.route_resolved,
                counted_death: saved.counted_death,
                combat_target: saved.combat_target.clone(),
                combat_goal: saved.combat_goal.map(|id| restore_live_id(map, &id)),
                stand_ground: saved.stand_ground,
                placement,
                activation: match &saved.activation {
                    SavedMonsterActivation::Active => MonsterActivation::Active,
                    SavedMonsterActivation::Dormant => MonsterActivation::Dormant,
                    SavedMonsterActivation::Scheduled { at, activator } => MonsterActivation::Scheduled {
                        at: *at,
                        activator: activator.as_ref().map(|id| restore_live_id(map, id)),
                    },
                },
            };
            match map {
                MonsterMap::Q1 { game, .. } => {
                    game.authored_targets.insert(actor.id().clone(), entry.target.clone());
                }
                MonsterMap::Q2 { game, .. } => {
                    game.authored_targets.insert(actor.id().clone(), entry.target.clone());
                }
            }
            self.definitions.insert(actor.id().clone(), saved.definition.clone());
            self.authored.insert(actor.id().clone(), entry);
        }
        Ok(())
    }

    /// Resolve and validate one saved monster actor.
    fn restore_actor(&self, map: &mut MonsterMap, saved: &SavedAuthoredMonster) -> Result<OwnedActor, WorldError> {
        let missing_owner = || bad_save("Missing authored monster map owner");
        let mismatch = || bad_save("Saved monster definition differs from selected actor");
        let classname = saved.target.classname.as_str();
        let (actor, provider) = match map {
            MonsterMap::Q1 { game, .. } => {
                let actor = game
                    .host
                    .actors
                    .resolve_saved(&saved.target.actor)
                    .ok_or_else(missing_owner)?;
                (actor, game.provider())
            }
            MonsterMap::Q2 { game, .. } => {
                let actor = game
                    .host
                    .actors()
                    .resolve_saved(saved.target.actor)
                    .ok_or_else(missing_owner)?;
                (actor, game.options.provider.clone())
            }
        };
        if actor.owner() != &provider {
            return Err(missing_owner());
        }
        let expected = self
            .selection_by_classname
            .get(classname)
            .unwrap_or(&self.selection_default);
        let MonsterSelectionTarget::Defined(expected) = expected else {
            return Err(mismatch());
        };
        if expected.classname != saved.definition.classname
            || expected.source.provider != saved.definition.source.provider
            || expected.source.content != saved.definition.source.content
        {
            return Err(mismatch());
        }
        let observed = match map {
            MonsterMap::Q1 { game, .. } => game.host.actors.observations().iter().any(|observation| {
                &observation.id == actor.id()
                    && observation.definition == format!("{}/{}", provider_text(&expected.source.provider), classname)
            }),
            MonsterMap::Q2 { game, .. } => game.host.actors().observations().iter().any(|observation| {
                &observation.id == actor.id()
                    && observation.definition == format!("{}/{}", provider_text(&expected.source.provider), classname)
            }),
        };
        if !observed {
            return Err(mismatch());
        }
        Ok(actor)
    }

    /// Forget a released actor (the session forwards engine releases here).
    pub fn note_released(&mut self, actor: &ActorId) {
        self.authored.remove(actor);
        self.definitions.remove(actor);
    }

    /// Run pre-turn placement and activation for one monster.
    pub fn before_turn<B: SelectedMonsterBehavior>(
        &mut self,
        map: &mut MonsterMap,
        behavior: &mut B,
        actor: &ActorId,
    ) -> Result<bool, WorldError> {
        let Some(placement) = self.authored.get(actor).map(|entry| entry.placement.clone()) else {
            return Ok(true);
        };
        match placement {
            MonsterPlacement::Teleport { origin } => {
                let body = read_body(map, actor);
                if body.as_ref().is_some_and(|state| state.origin != origin) {
                    let ready = behavior.placement_ready(self.authored.get(actor).expect("entry"));
                    if !ready {
                        self.authored.get_mut(actor).expect("entry").placement = MonsterPlacement::Teleport { origin };
                        return Ok(false);
                    }
                    let definition = self
                        .definitions
                        .get(actor)
                        .cloned()
                        .ok_or_else(|| bad_save("Teleporting monster has no selected definition"))?;
                    let at = body.as_ref().map(|state| state.origin);
                    let placed = behavior.validate_placement(self.authored.get(actor).expect("entry"), &definition, at);
                    if !placed {
                        return Ok(false);
                    }
                    self.authored.get_mut(actor).expect("entry").placement = MonsterPlacement::Ready;
                }
            }
            MonsterPlacement::Waiting { activator, .. } => {
                let ready = behavior.placement_ready(self.authored.get(actor).expect("entry"));
                if !ready {
                    return Ok(false);
                }
                let definition = self
                    .definitions
                    .get(actor)
                    .cloned()
                    .ok_or_else(|| bad_save("Waiting monster has no selected definition"))?;
                let placed = behavior.validate_placement(self.authored.get(actor).expect("entry"), &definition, None);
                if !placed {
                    return Ok(false);
                }
                let owned = self.authored.get(actor).expect("entry").target.actor.clone();
                wake_monster(map, &owned)?;
                self.authored.get_mut(actor).expect("entry").placement = MonsterPlacement::Ready;
                behavior.resume(actor, activator);
            }
            MonsterPlacement::Ready => {}
        }
        let activation = self.authored.get(actor).expect("entry").activation.clone();
        match activation {
            MonsterActivation::Active => Ok(self.active(actor)),
            MonsterActivation::Dormant => {
                if !matches!(map, MonsterMap::Q2 { .. }) {
                    return Err(bad_save("Triggered monster activation requires its Q2 map program"));
                }
                Ok(false)
            }
            MonsterActivation::Scheduled { at, activator } => {
                let MonsterMap::Q2 { game, .. } = map else {
                    return Err(bad_save("Triggered monster activation requires its Q2 map program"));
                };
                if at > game.host.now() {
                    return Ok(false);
                }
                let owned = self.authored.get(actor).expect("entry").target.actor.clone();
                let spawnflags = self.authored.get(actor).expect("entry").spawnflags;
                place_triggered_monster(game, owned.clone());
                if !game.host.actors().is_live(actor) {
                    return Ok(false);
                }
                self.authored.get_mut(actor).expect("entry").activation = MonsterActivation::Active;
                wake_monster(map, &owned)?;
                behavior.resume(actor, if spawnflags & 1 == 0 { activator } else { None });
                let live = match map {
                    MonsterMap::Q1 { game, .. } => game.host.actors.is_live(actor),
                    MonsterMap::Q2 { game, .. } => game.host.actors().is_live(actor),
                };
                Ok(live && self.active(actor))
            }
        }
    }
}

/// Live body snapshot for the teleport check.
struct BodySnapshot {
    origin: Vec3,
}

fn read_body(map: &mut MonsterMap, actor: &ActorId) -> Option<BodySnapshot> {
    match map {
        MonsterMap::Q1 { game, .. } => game
            .host
            .bodies
            .read(actor)
            .map(|state| BodySnapshot { origin: state.origin }),
        MonsterMap::Q2 { game, .. } => game
            .host
            .bodies()
            .read(actor)
            .map(|state| BodySnapshot { origin: state.origin }),
    }
}

/// Make a waking monster damageable and link its body.
fn wake_monster(map: &mut MonsterMap, owned: &OwnedActor) -> Result<(), WorldError> {
    match map {
        MonsterMap::Q1 { game, .. } => {
            set_q1_can_take_damage(game, owned, true)?;
            game.host
                .bodies
                .link(owned)
                .map_err(|error| bad_save(error.to_string()))
        }
        MonsterMap::Q2 { game, .. } => {
            set_q2_can_take_damage(game, owned, true);
            game.host.bodies().link(owned, None);
            Ok(())
        }
    }
}

/// Reference a saved goal actor in the live session.
fn restore_live_id(map: &mut MonsterMap, saved: &SavedActorId) -> ActorId {
    match map {
        MonsterMap::Q1 { game, .. } => game.host.actors.reference_saved(saved),
        MonsterMap::Q2 { game, .. } => game.host.actors().reference_saved(*saved),
    }
}

/// Build the authored entry shared by both admission paths.
fn admitted_entry(actor: &OwnedActor, fields: &BTreeMap<String, String>, ordinal: i32) -> AuthoredMonster {
    let text = |key: &str| fields.get(key).cloned().unwrap_or_default();
    AuthoredMonster {
        target: AuthoredTarget {
            actor: actor.clone(),
            classname: text("classname"),
            targetname: text("targetname"),
            target: text("target"),
            killtarget: text("killtarget"),
            message: text("message"),
            delay: number_field(fields, "delay"),
        },
        source_ordinal: u32::try_from(ordinal).unwrap_or(0),
        spawnflags: number_field(fields, "spawnflags") as i32 as u32,
        death_target: text("death_target"),
        drop_item: text("drop_item"),
        route: text("target"),
        route_goal: None,
        route_resolved: false,
        counted_death: false,
        combat_target: text("combattarget"),
        combat_goal: None,
        stand_ground: false,
        placement: MonsterPlacement::Ready,
        activation: MonsterActivation::Active,
    }
}

impl SelectedMonsters {
    /// Note a monster entering the world (donor mission `started`).
    pub fn monster_started<B: SelectedMonsterBehavior>(
        &mut self,
        map: &mut MonsterMap,
        behavior: &mut B,
        actor: &ActorId,
    ) -> Result<(), WorldError> {
        let _ = map;
        let Some(entry) = self.authored.get(actor) else {
            return Ok(());
        };
        let entry = entry.clone();
        let Some(definition) = self.definitions.get(actor).cloned() else {
            return Err(bad_save("Started monster has no selected definition"));
        };
        behavior.validate_placement(&entry, &definition, None);
        Ok(())
    }

    /// Resolve the patrol goal, caching it on the entry (donor mission
    /// `route`).
    pub fn monster_route(&mut self, map: &mut MonsterMap, actor: &ActorId) -> Option<ActorId> {
        let entry = self.authored.get(actor)?;
        if entry.route_resolved {
            return entry.route_goal.clone();
        }
        let route = entry.route.clone();
        let goal = match map {
            MonsterMap::Q1 { game, .. } => {
                let found = game.find(&route);
                let first = found.first()?;
                let target = game.entities.get(first)?;
                if target.classname == "path_corner" && game.host.actors.is_live(first) {
                    Some(first.clone())
                } else {
                    None
                }
            }
            MonsterMap::Q2 { game, .. } => {
                let found = game.pick_target(&route);
                if game
                    .targets(&route)
                    .iter()
                    .filter_map(|id| game.entities.get(id))
                    .any(|target| target.classname == "point_combat")
                {
                    let entry = self.authored.get_mut(actor)?;
                    entry.combat_target.clone_from(&entry.route.clone());
                    entry.target.target.clone_from(&entry.route.clone());
                    entry.route = String::new();
                }
                found
            }
        };
        let entry = self.authored.get_mut(actor)?;
        entry.route_goal = goal.clone();
        entry.route_resolved = true;
        goal
    }

    /// Cached patrol goal without resolving (for the shared mission handle;
    /// [`SelectedMonsters::mission`] resolves eagerly first).
    fn route_cached(&self, actor: &ActorId) -> Option<ActorId> {
        self.authored.get(actor).and_then(|entry| entry.route_goal.clone())
    }

    /// Note a monster acquiring its combat target (donor mission
    /// `foundTarget`).
    pub fn monster_found_target<B: SelectedMonsterBehavior>(
        &mut self,
        map: &mut MonsterMap,
        behavior: &mut B,
        actor: &ActorId,
    ) {
        let take = match self.authored.get(actor) {
            Some(entry) if !entry.combat_target.is_empty() => behavior.enemy(actor).is_some(),
            _ => false,
        };
        if !take {
            return;
        }
        let MonsterMap::Q2 { game, .. } = map else { return };
        let target = self.authored.get(actor).expect("entry").combat_target.clone();
        let Some(goal) = game.pick_target(&target) else { return };
        let entry = self.authored.get_mut(actor).expect("entry");
        entry.combat_target = String::new();
        entry.combat_goal = Some(goal.clone());
        if game.options.edition == Q2Edition::Classic {
            if let Some(target) = game.entities.get_mut(&goal) {
                target.targetname = String::new();
            }
        }
    }

    /// Use a dormant monster (donor mission `use`).
    pub fn monster_use(
        &mut self,
        map: &mut MonsterMap,
        actor: &ActorId,
        activator: Option<ActorId>,
    ) -> Result<bool, WorldError> {
        let Some(entry) = self.authored.get_mut(actor) else {
            return Ok(false);
        };
        if matches!(entry.placement, MonsterPlacement::Waiting { .. }) {
            if let MonsterPlacement::Waiting { activator: slot, .. } = &mut entry.placement {
                *slot = activator;
            }
            return Ok(true);
        }
        if matches!(entry.activation, MonsterActivation::Active) {
            return Ok(false);
        }
        let MonsterMap::Q2 { game, .. } = map else {
            return Err(bad_save("Triggered monster use requires its Q2 map program"));
        };
        let delay = if game.options.edition == Q2Edition::Rerelease {
            game.host.frame_seconds()
        } else {
            0.1
        };
        let at = game.host.now() + delay;
        let entry = self.authored.get_mut(actor).expect("entry");
        if matches!(entry.activation, MonsterActivation::Dormant) {
            entry.activation = MonsterActivation::Scheduled { at, activator };
        }
        Ok(true)
    }

    /// Count a spawned monster (donor mission `spawned`).
    pub fn monster_spawned(&mut self, map: &mut MonsterMap, actor: &ActorId) {
        let Some(entry) = self.authored.get(actor) else { return };
        let fish = entry.target.classname == "monster_fish";
        match map {
            MonsterMap::Q1 { game, .. } => {
                game.total_monsters += 1;
                if fish && game.options().edition == Q1Edition::Classic {
                    game.total_monsters += 1;
                }
            }
            MonsterMap::Q2 { game, .. } => {
                game.counters.total_monsters += 1;
            }
        }
    }

    /// Count a monster death, drop its item, and fire its death target
    /// (donor mission `killed`).
    pub fn monster_killed(
        &mut self,
        map: &mut MonsterMap,
        actor: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), WorldError> {
        let Some(entry) = self.authored.get_mut(actor) else {
            return Ok(());
        };
        if entry.counted_death {
            return Ok(());
        }
        entry.counted_death = true;
        let owned = entry.target.actor.clone();
        let drop_item = entry.drop_item.clone();
        let death_target = entry.death_target.clone();
        match map {
            MonsterMap::Q1 { game, kills } => {
                game.killed_monsters += 1;
                let total = game.total_monsters;
                let found = game.killed_monsters;
                game.use_targets(actor, attacker)
                    .map_err(|error| bad_save(error.to_string()))?;
                kills.monster_killed(actor, total, found);
            }
            MonsterMap::Q2 { game, items } => {
                game.counters.killed_monsters += 1;
                if !drop_item.is_empty() {
                    items.drop_monster(&owned, game, &drop_item);
                }
                if !death_target.is_empty() {
                    self.authored
                        .get_mut(actor)
                        .expect("entry")
                        .target
                        .target
                        .clone_from(&death_target);
                }
                let target = self.authored.get(actor).expect("entry").target.clone();
                game.use_targets(&target, attacker, false);
            }
        }
        Ok(())
    }

    /// Read the combat route (donor mission `combatRoute`).
    #[must_use]
    pub fn monster_combat_route(&self, actor: &ActorId) -> CombatRoute {
        self.authored.get(actor).map_or(
            CombatRoute {
                goal: None,
                stand_ground: false,
            },
            |entry| CombatRoute {
                goal: entry.combat_goal.clone(),
                stand_ground: entry.stand_ground,
            },
        )
    }

    /// Borrow a shared mission handle (donor `mission`). The patrol route
    /// resolves eagerly so the handle's borrowed `route` matches donor
    /// semantics; see the module docs for the ownership rationale.
    pub fn mission<'m, B: SelectedMonsterBehavior>(
        &'m mut self,
        map: &'m mut MonsterMap,
        behavior: &'m mut B,
        actor: &ActorId,
    ) -> Option<SelectedMonsterMission<'m, B>> {
        if !self.authored.contains_key(actor) {
            return None;
        }
        let id = actor.clone();
        self.monster_route(map, &id);
        Some(SelectedMonsterMission {
            monsters: self,
            map,
            behavior,
            actor: id,
        })
    }

    /// Borrow a Q1 patrol follower (donor `q1PathFollower`).
    pub fn q1_path_follower<'m, B: SelectedMonsterBehavior>(
        &'m mut self,
        behavior: &'m mut B,
        actor: &ActorId,
    ) -> Option<SelectedQ1Follower<'m, B>> {
        if !self.authored.contains_key(actor) {
            return None;
        }
        let targetname = self.authored.get(actor).expect("entry").route.clone();
        let enemy = behavior.enemy(actor);
        Some(SelectedQ1Follower {
            targetname,
            enemy,
            monsters: self,
            behavior,
            actor: actor.clone(),
        })
    }

    /// Borrow a Q2 patrol follower (donor `q2PathFollower`).
    pub fn q2_path_follower<'m, B: SelectedMonsterBehavior>(
        &'m mut self,
        behavior: &'m mut B,
        actor: &ActorId,
    ) -> Option<SelectedQ2PathFollower<'m, B>> {
        let entry = self.authored.get(actor)?;
        let follower = SelectedQ2PathFollower {
            actor: entry.target.actor.clone(),
            move_target: entry.route_goal.clone(),
            enemy: behavior.enemy(actor),
            monsters: self,
            behavior,
            actor_id: actor.clone(),
        };
        Some(follower)
    }

    /// Borrow a Q2 combat follower (donor `q2CombatFollower`).
    pub fn q2_combat_follower<'m, B: SelectedMonsterBehavior>(
        &'m mut self,
        behavior: &'m mut B,
        actor: &ActorId,
    ) -> Option<SelectedQ2CombatFollower<'m>> {
        let entry = self.authored.get(actor)?;
        let activator = match &entry.activation {
            MonsterActivation::Scheduled { activator, .. } => activator.clone(),
            _ => None,
        };
        let follower = SelectedQ2CombatFollower {
            enemy: behavior.enemy(actor),
            old_enemy: behavior.old_enemy(actor),
            activator,
            monsters: self,
            actor_id: actor.clone(),
        };
        Some(follower)
    }
}

/// Shared mission handle implementing [`MonsterMission`] over borrowed
/// runtime state (donor `mission` return).
pub struct SelectedMonsterMission<'m, B> {
    monsters: &'m mut SelectedMonsters,
    map: &'m mut MonsterMap,
    behavior: &'m mut B,
    actor: ActorId,
}

impl<B: SelectedMonsterBehavior> MonsterMission for SelectedMonsterMission<'_, B> {
    fn spawned(&mut self) {
        self.monsters.monster_spawned(self.map, &self.actor);
    }

    fn started(&mut self) {
        self.monsters
            .monster_started(self.map, self.behavior, &self.actor)
            .expect("Started monster has no selected definition");
    }

    fn killed(&mut self, attacker: Option<&ActorId>) {
        self.monsters
            .monster_killed(self.map, &self.actor, attacker)
            .expect("monster kill failed");
    }

    fn route(&self) -> Option<ActorId> {
        self.monsters.route_cached(&self.actor)
    }

    fn r#use(&mut self, activator: Option<&ActorId>) -> bool {
        self.monsters
            .monster_use(self.map, &self.actor, activator.cloned())
            .expect("Triggered monster use requires its Q2 map program")
    }

    fn combat_route(&self) -> CombatRoute {
        self.monsters.monster_combat_route(&self.actor)
    }

    fn found_target(&mut self) {
        self.monsters.monster_found_target(self.map, self.behavior, &self.actor);
    }
}

/// Q1 patrol follower over borrowed runtime state (donor `q1PathFollower`
/// return).
pub struct SelectedQ1Follower<'m, B> {
    targetname: String,
    enemy: Option<ActorId>,
    monsters: &'m mut SelectedMonsters,
    behavior: &'m mut B,
    actor: ActorId,
}

impl<B: SelectedMonsterBehavior> SelectedQ1Follower<'_, B> {
    /// Route name snapshot.
    #[must_use]
    pub fn targetname(&self) -> &str {
        &self.targetname
    }

    /// Current enemy snapshot.
    #[must_use]
    pub fn enemy(&self) -> Option<ActorId> {
        self.enemy.clone()
    }

    /// Retarget the patrol route.
    pub fn advance(&mut self, name: &str, goal: Option<ActorId>, pause_until: f64) {
        let entry = self.monsters.authored.get_mut(&self.actor).expect("entry");
        entry.route = if goal.is_none() {
            String::new()
        } else {
            name.to_string()
        };
        entry.route_goal.clone_from(&goal);
        entry.route_resolved = true;
        self.behavior.set_route(&self.actor, goal, pause_until);
    }
}

/// Q2 patrol follower over borrowed runtime state (donor `q2PathFollower`
/// return).
pub struct SelectedQ2PathFollower<'m, B> {
    actor: OwnedActor,
    move_target: Option<ActorId>,
    enemy: Option<ActorId>,
    monsters: &'m mut SelectedMonsters,
    behavior: &'m mut B,
    actor_id: ActorId,
}

impl<B: SelectedMonsterBehavior> Q2PathFollower for SelectedQ2PathFollower<'_, B> {
    fn actor(&self) -> &OwnedActor {
        &self.actor
    }

    fn move_target(&self) -> Option<ActorId> {
        self.move_target.clone()
    }

    fn enemy(&self) -> Option<ActorId> {
        self.enemy.clone()
    }

    fn advance(&mut self, name: &str, goal: Option<ActorId>, pause_until: f64) {
        let entry = self.monsters.authored.get_mut(&self.actor_id).expect("entry");
        entry.route = name.to_string();
        entry.route_goal.clone_from(&goal);
        entry.route_resolved = true;
        self.behavior.set_route(&self.actor_id, goal, pause_until);
    }
}

/// Q2 combat follower over borrowed runtime state (donor `q2CombatFollower`
/// return).
pub struct SelectedQ2CombatFollower<'m> {
    enemy: Option<ActorId>,
    old_enemy: Option<ActorId>,
    activator: Option<ActorId>,
    monsters: &'m mut SelectedMonsters,
    actor_id: ActorId,
}

impl Q2CombatFollower for SelectedQ2CombatFollower<'_> {
    fn move_target(&self) -> Option<ActorId> {
        self.monsters
            .authored
            .get(&self.actor_id)
            .and_then(|entry| entry.combat_goal.clone())
    }

    fn enemy(&self) -> Option<ActorId> {
        self.enemy.clone()
    }

    fn old_enemy(&self) -> Option<ActorId> {
        self.old_enemy.clone()
    }

    fn activator(&self) -> Option<ActorId> {
        self.activator.clone()
    }

    fn walking(&self) -> bool {
        true
    }

    fn advance(&mut self, target: &str, _goal: Option<ActorId>, move_target: Option<ActorId>) {
        let entry = self.monsters.authored.get_mut(&self.actor_id).expect("entry");
        entry.target.target = target.to_string();
        entry.combat_goal = move_target;
        entry.combat_target = String::new();
    }

    fn hold(&mut self) {
        let entry = self.monsters.authored.get_mut(&self.actor_id).expect("entry");
        entry.combat_goal = None;
        entry.stand_ground = true;
    }

    fn finish(&mut self) {
        self.monsters
            .authored
            .get_mut(&self.actor_id)
            .expect("entry")
            .stand_ground = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{ArmorState, ContentId, PoweredProtectionState, RegularArmorState};
    use qa_content::q1::foundation::gameplay::CombatState as Q1CombatState;
    use qa_content::q1::foundation::host::Q1GameplayAuthority;
    use qa_content::q2::foundation::items::{create_q2_item_module, Q2ItemHooks};
    use qa_content::q2::support::contracts::{BodyState as Q2BodyState, CombatState as Q2CombatState};
    use qa_content::q2::support::tables::{Q2BodyTable, Q2CombatAuthority};
    use qa_core::identity::ProviderId;
    use qa_core::math::Bounds;

    use super::super::test_hosts::{test_q1_game, test_q2_game, Q1Handles, Q2Handles};

    struct FakeBehavior {
        attached: Vec<ActorId>,
        validate: bool,
        ready: bool,
        enemy: Option<ActorId>,
        old_enemy: Option<ActorId>,
        routes: Vec<(ActorId, Option<ActorId>, f64)>,
        resumed: Vec<(ActorId, Option<ActorId>)>,
    }

    impl FakeBehavior {
        fn new() -> Self {
            Self {
                attached: Vec::new(),
                validate: true,
                ready: true,
                enemy: None,
                old_enemy: None,
                routes: Vec::new(),
                resumed: Vec::new(),
            }
        }
    }

    impl SelectedMonsterBehavior for FakeBehavior {
        fn attach(&mut self, actor: &OwnedActor, _definition: &MonsterDefinitionReference) {
            self.attached.push(actor.id().clone());
        }

        fn validate_placement(
            &mut self,
            _entry: &AuthoredMonster,
            _definition: &MonsterDefinitionReference,
            _relocated: Option<Vec3>,
        ) -> bool {
            self.validate
        }

        fn placement_ready(&mut self, _entry: &AuthoredMonster) -> bool {
            self.ready
        }

        fn enemy(&mut self, _actor: &ActorId) -> Option<ActorId> {
            self.enemy.clone()
        }

        fn old_enemy(&mut self, _actor: &ActorId) -> Option<ActorId> {
            self.old_enemy.clone()
        }

        fn set_route(&mut self, actor: &ActorId, goal: Option<ActorId>, pause_until: f64) {
            self.routes.push((actor.clone(), goal, pause_until));
        }

        fn resume(&mut self, actor: &ActorId, activator: Option<ActorId>) {
            self.resumed.push((actor.clone(), activator));
        }
    }

    struct FakeKills {
        kills: std::rc::Rc<std::cell::RefCell<Vec<(ActorId, i32, i32)>>>,
    }

    impl Q1MonsterKillSink for FakeKills {
        fn monster_killed(&mut self, actor: &ActorId, total: i32, found: i32) {
            self.kills.borrow_mut().push((actor.clone(), total, found));
        }
    }

    fn knight_ref() -> MonsterDefinitionReference {
        MonsterDefinitionReference {
            source: ProviderReference {
                provider: ProviderId::new("q1", "monsters/classic/id1"),
                content: ContentId("q1:id1:e1m1:1".to_string()),
            },
            classname: "monster_knight".to_string(),
        }
    }

    fn berserk_ref() -> MonsterDefinitionReference {
        MonsterDefinitionReference {
            source: ProviderReference {
                provider: ProviderId::new("q2", "monsters/classic/baseq2"),
                content: ContentId("q2:baseq2:base1:1".to_string()),
            },
            classname: "monster_berserk".to_string(),
        }
    }

    fn q1_selection() -> EnemySelection {
        EnemySelection::Replace {
            default: MonsterSelectionTarget::Defined(knight_ref()),
            by_classname: HashMap::from([("monster_boss".to_string(), MonsterSelectionTarget::MapDefined)]),
        }
    }

    fn q2_selection() -> EnemySelection {
        EnemySelection::Replace {
            default: MonsterSelectionTarget::Defined(berserk_ref()),
            by_classname: HashMap::new(),
        }
    }

    fn q1_map(game: Q1EntityServices) -> MonsterMap {
        MonsterMap::Q1 {
            game,
            kills: Box::new(FakeKills {
                kills: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            }),
        }
    }

    fn q2_map(game: Q2GameServices) -> MonsterMap {
        fn weapon_picked(_a: ActorId, _g: &mut Q2GameServices, _i: qa_content::contract::ItemId, _b: bool) {}
        fn silencer(_a: ActorId, _v: f64) {}
        fn power_armor(_a: ActorId, _k: qa_content::q2::foundation::items::Q2PowerArmorKind) {}
        MonsterMap::Q2 {
            game,
            items: create_q2_item_module(Q2ItemHooks {
                weapon_picked,
                silencer,
                power_armor,
                ammo_pack: None,
                random_respawn: None,
                weapon_respawn_seconds: None,
            }),
        }
    }

    fn zero_vec() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn test_bounds() -> Bounds {
        Bounds {
            min: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
            max: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        }
    }

    fn test_armor() -> ArmorState {
        ArmorState {
            regular: RegularArmorState::None,
            powered: PoweredProtectionState::None,
        }
    }

    fn admit_q1_knight(
        monsters: &mut SelectedMonsters,
        map: &mut MonsterMap,
        behavior: &mut FakeBehavior,
        handles: &Q1Handles,
        target: &str,
    ) -> OwnedActor {
        let owned = handles
            .actors
            .mint(&ProviderId::new("q1", "test"), "q1:official/monster_knight");
        handles.bodies.admit_linked(owned.id(), zero_vec(), test_bounds());
        {
            let mut combat = handles.combat.clone();
            Q1GameplayAuthority::create(
                &mut combat,
                &owned,
                &Q1CombatState {
                    health: 100.0,
                    armor: test_armor(),
                    mass: 100.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: None,
                    team: None,
                },
            )
            .expect("combat");
        }
        let source = Q1Entity {
            properties: vec![
                ("classname".to_string(), "monster_knight".to_string()),
                ("target".to_string(), target.to_string()),
                ("spawnflags".to_string(), "0".to_string()),
            ],
        };
        monsters
            .admit_q1(map, behavior, owned.clone(), &source, 3, &knight_ref())
            .expect("admit");
        owned
    }

    #[test]
    fn selection_must_replace() {
        assert!(SelectedMonsters::new(&EnemySelection::MapDefined).is_err());
        assert!(SelectedMonsters::new(&q1_selection()).is_ok());
    }

    #[test]
    fn resolve_routes_ordinary_map_defined_and_unknown() {
        let (game, _handles) = test_q1_game();
        let map = q1_map(game);
        let monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let fields = BTreeMap::new();
        assert_eq!(
            monsters.resolve(&map, "monster_knight", &fields).expect("resolve"),
            Some(knight_ref())
        );
        assert_eq!(monsters.resolve(&map, "monster_boss", &fields).expect("boss"), None);
        assert_eq!(monsters.resolve(&map, "light", &fields).expect("light"), None);
        assert!(monsters.resolve(&map, "monster_bob", &fields).is_err());
    }

    #[test]
    fn resolve_masks_spawn_flags() {
        let (game, _handles) = test_q1_game();
        let map = q1_map(game);
        let monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let flagged = |flags: &str| BTreeMap::from([("spawnflags".to_string(), flags.to_string())]);
        assert!(monsters
            .resolve(&map, "monster_knight", &flagged("3"))
            .expect("low")
            .is_some());
        assert!(monsters
            .resolve(&map, "monster_knight", &flagged("256"))
            .expect("inhibit")
            .is_some());
        assert!(monsters.resolve(&map, "monster_knight", &flagged("4")).is_err());
        assert!(monsters
            .resolve(&map, "monster_zombie", &flagged("1"))
            .expect("zombie")
            .is_some());
        assert!(monsters.resolve(&map, "monster_zombie", &flagged("2")).is_err());
    }

    #[test]
    fn admit_q1_pins_and_attaches() {
        let (game, handles) = test_q1_game();
        let mut map = q1_map(game);
        let mut monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q1_knight(&mut monsters, &mut map, &mut behavior, &handles, "p1");
        assert!(monsters.active(owned.id()));
        assert_eq!(behavior.attached, vec![owned.id().clone()]);
        let entry = &monsters.authored[owned.id()];
        assert_eq!(entry.route, "p1");
        assert_eq!(entry.source_ordinal, 3);
        let MonsterMap::Q1 { game, .. } = &map else {
            panic!("q1 map")
        };
        assert!(game.authored_targets.contains_key(owned.id()));
        assert!(
            !Q1GameplayAuthority::read(&handles.combat, owned.id())
                .expect("combat")
                .can_take_damage
        );
    }

    #[test]
    fn q1_route_resolves_path_corner() {
        let (mut game, handles) = test_q1_game();
        let corner = game.create("path_corner", None, None).expect("corner");
        game.entities.get_mut(&corner).expect("entity").targetname = "p1".to_string();
        let mut map = q1_map(game);
        let mut monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q1_knight(&mut monsters, &mut map, &mut behavior, &handles, "p1");
        assert_eq!(monsters.monster_route(&mut map, owned.id()), Some(corner.clone()));
        assert!(monsters.authored[owned.id()].route_resolved);
        assert_eq!(monsters.monster_route(&mut map, owned.id()), Some(corner));
    }

    #[test]
    fn q1_kill_counts_once_and_reports() {
        let (game, handles) = test_q1_game();
        let mut map = q1_map(game);
        let mut monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q1_knight(&mut monsters, &mut map, &mut behavior, &handles, "");
        monsters.monster_spawned(&mut map, owned.id());
        monsters.monster_killed(&mut map, owned.id(), None).expect("kill");
        monsters.monster_killed(&mut map, owned.id(), None).expect("kill");
        let MonsterMap::Q1 { game, kills } = &mut map else {
            panic!("q1 map")
        };
        assert_eq!(game.total_monsters, 1);
        assert_eq!(game.killed_monsters, 1);
        assert!(monsters.authored[owned.id()].counted_death);
        kills.monster_killed(owned.id(), game.total_monsters, game.killed_monsters);
    }

    #[test]
    fn q1_kill_sink_reports_counts() {
        let (game, handles) = test_q1_game();
        let recorded = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut map = MonsterMap::Q1 {
            game,
            kills: Box::new(FakeKills {
                kills: recorded.clone(),
            }),
        };
        let mut monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q1_knight(&mut monsters, &mut map, &mut behavior, &handles, "");
        monsters.monster_spawned(&mut map, owned.id());
        monsters.monster_killed(&mut map, owned.id(), None).expect("kill");
        assert_eq!(*recorded.borrow(), vec![(owned.id().clone(), 1, 1)]);
    }

    #[test]
    fn mission_handle_delegates() {
        let (mut game, handles) = test_q1_game();
        let corner = game.create("path_corner", None, None).expect("corner");
        game.entities.get_mut(&corner).expect("entity").targetname = "p1".to_string();
        let mut map = q1_map(game);
        let mut monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q1_knight(&mut monsters, &mut map, &mut behavior, &handles, "p1");
        let mut mission = monsters.mission(&mut map, &mut behavior, owned.id()).expect("mission");
        assert_eq!(MonsterMission::route(&mission), Some(corner));
        MonsterMission::spawned(&mut mission);
        assert_eq!(mission.monsters.monster_combat_route(owned.id()).goal, None);
        MonsterMission::started(&mut mission);
        assert!(monsters.definitions.contains_key(owned.id()));
    }

    #[test]
    fn q1_follower_advances_route() {
        let (game, handles) = test_q1_game();
        let mut map = q1_map(game);
        let mut monsters = SelectedMonsters::new(&q1_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q1_knight(&mut monsters, &mut map, &mut behavior, &handles, "p1");
        let goal = handles
            .actors
            .mint(&ProviderId::new("q1", "test"), "path_corner")
            .id()
            .clone();
        let mut follower = monsters.q1_path_follower(&mut behavior, owned.id()).expect("follower");
        assert_eq!(follower.targetname(), "p1");
        follower.advance("p2", Some(goal.clone()), 4.0);
        let entry = &monsters.authored[owned.id()];
        assert_eq!(entry.route, "p2");
        assert_eq!(entry.route_goal, Some(goal.clone()));
        assert!(entry.route_resolved);
        assert_eq!(behavior.routes, vec![(owned.id().clone(), Some(goal.clone()), 4.0)]);
        assert!(monsters.q1_path_follower(&mut behavior, &goal).is_none());
    }

    fn admit_q2_berserk(
        monsters: &mut SelectedMonsters,
        map: &mut MonsterMap,
        behavior: &mut FakeBehavior,
        handles: &Q2Handles,
    ) -> OwnedActor {
        let owned = handles.actors.mint(
            &ProviderId::new("q2", "test"),
            "q2:monsters/classic/baseq2/monster_berserk",
        );
        {
            let mut bodies = handles.bodies.clone();
            Q2BodyTable::create(
                &mut bodies,
                &owned,
                &Q2BodyState {
                    origin: zero_vec(),
                    angles: zero_vec(),
                    velocity: zero_vec(),
                    bounds: test_bounds(),
                    ground: None,
                },
            );
            Q2BodyTable::link(&mut bodies, &owned, None);
            let mut combat = handles.combat.clone();
            Q2CombatAuthority::create(
                &mut combat,
                &owned,
                &Q2CombatState {
                    health: 100.0,
                    armor: test_armor(),
                    mass: 100.0,
                    can_take_damage: true,
                    invulnerable: false,
                    no_knockback: false,
                    team: None,
                },
            );
        }
        let source = Q2SpawnFields {
            ordinal: 5,
            classname: "monster_berserk".to_string(),
            values: BTreeMap::from([
                ("classname".to_string(), "monster_berserk".to_string()),
                ("spawnflags".to_string(), "0".to_string()),
            ]),
        };
        monsters
            .admit_q2(map, behavior, owned.clone(), &source, &berserk_ref())
            .expect("admit");
        owned
    }

    #[test]
    fn q2_capture_restore_round_trips() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        let saved = monsters.capture().expect("capture");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].source_ordinal, 5);
        let mut revived = SelectedMonsters::new(&q2_selection()).expect("select");
        revived.restore(&mut map, &saved).expect("restore");
        assert!(revived.active(owned.id()));
        assert_eq!(revived.authored[owned.id()].source_ordinal, 5);
        assert_eq!(revived.capture().expect("recapture").len(), 1);
    }

    #[test]
    fn restore_rejects_definition_mismatch() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        let mut saved = monsters.capture().expect("capture");
        saved[0].definition.classname = "monster_boss".to_string();
        let mut revived = SelectedMonsters::new(&q2_selection()).expect("select");
        assert!(revived.restore(&mut map, &saved).is_err());
    }

    #[test]
    fn before_turn_wakes_waiting_monster() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        monsters.authored.get_mut(owned.id()).expect("entry").placement = MonsterPlacement::Waiting {
            barriers: Vec::new(),
            activator: None,
        };
        assert!(monsters.before_turn(&mut map, &mut behavior, owned.id()).expect("turn"));
        assert!(matches!(
            monsters.authored[owned.id()].placement,
            MonsterPlacement::Ready
        ));
        assert!(
            Q2CombatAuthority::read(&handles.combat, owned.id())
                .expect("combat")
                .can_take_damage
        );
        assert_eq!(behavior.resumed.len(), 1);
        behavior.ready = false;
        monsters.authored.get_mut(owned.id()).expect("entry").placement = MonsterPlacement::Waiting {
            barriers: Vec::new(),
            activator: None,
        };
        assert!(!monsters.before_turn(&mut map, &mut behavior, owned.id()).expect("turn"));
    }

    #[test]
    fn before_turn_holds_dormant_and_future() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        monsters.authored.get_mut(owned.id()).expect("entry").activation = MonsterActivation::Dormant;
        assert!(!monsters.before_turn(&mut map, &mut behavior, owned.id()).expect("turn"));
        monsters.authored.get_mut(owned.id()).expect("entry").activation = MonsterActivation::Scheduled {
            at: 60.0,
            activator: None,
        };
        assert!(!monsters.before_turn(&mut map, &mut behavior, owned.id()).expect("turn"));
        let stranger = handles
            .actors
            .mint(&ProviderId::new("q2", "test"), "player")
            .id()
            .clone();
        assert!(!monsters.active(&stranger));
        assert!(monsters.before_turn(&mut map, &mut behavior, &stranger).expect("turn"));
    }

    #[test]
    fn monster_use_schedules_dormant() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        assert!(!monsters.monster_use(&mut map, owned.id(), None).expect("active"));
        monsters.authored.get_mut(owned.id()).expect("entry").activation = MonsterActivation::Dormant;
        assert!(monsters.monster_use(&mut map, owned.id(), None).expect("use"));
        assert!(matches!(
            monsters.authored[owned.id()].activation,
            MonsterActivation::Scheduled { at, .. } if at == 0.1
        ));
        assert!(monsters.monster_use(&mut map, owned.id(), None).expect("again"));
    }

    #[test]
    fn q2_combat_follower_tracks_combat() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        let enemy = handles
            .actors
            .mint(&ProviderId::new("q2", "test"), "player")
            .id()
            .clone();
        behavior.enemy = Some(enemy.clone());
        let goal = handles
            .actors
            .mint(&ProviderId::new("q2", "test"), "point_combat")
            .id()
            .clone();
        let follower = monsters
            .q2_combat_follower(&mut behavior, owned.id())
            .expect("follower");
        assert_eq!(Q2CombatFollower::enemy(&follower), Some(enemy));
        assert!(Q2CombatFollower::walking(&follower));
        assert_eq!(Q2CombatFollower::move_target(&follower), None);
        {
            let mut held = follower;
            held.advance("p1", None, Some(goal.clone()));
            assert_eq!(Q2CombatFollower::move_target(&held), Some(goal));
            held.hold();
        }
        assert!(monsters.authored[owned.id()].stand_ground);
        {
            let mut held = monsters
                .q2_combat_follower(&mut behavior, owned.id())
                .expect("follower");
            held.finish();
        }
        assert!(!monsters.authored[owned.id()].stand_ground);
    }

    #[test]
    fn releases_forget_monsters() {
        let (game, handles) = test_q2_game();
        let mut map = q2_map(game);
        let mut monsters = SelectedMonsters::new(&q2_selection()).expect("select");
        let mut behavior = FakeBehavior::new();
        let owned = admit_q2_berserk(&mut monsters, &mut map, &mut behavior, &handles);
        monsters.note_released(owned.id());
        assert!(!monsters.active(owned.id()));
        assert!(monsters.capture().expect("capture").is_empty());
    }
}
