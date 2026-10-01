//! Legacy Q3 ballistics checkpoint import for the selected source.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/arsenal/q3-source-legacy.ts`
//! (`readLegacyQ3Source`, `LegacyQ3Source`, `migrateLegacyQ3Arsenal`,
//! `restoreLegacyQ3Source`).
//!
//! Validation, slot picking, records adoption, and reference projection
//! below are real. The entity surgery (classes, trajectories, think
//! callbacks, hook wiring, impact events, statistics, hook latches)
//! executes in the runtime backend through [`Q3SourceGame::import_legacy`]
//! (see `q3_source.rs`), because the Rust game models are
//! self-contained mirrors.

use std::collections::HashSet;

use qa_content::composition::{expansion_source_supply, q1_q3_supply_profile, q2_q3_supply_profile};
use qa_content::q3::base::game::state::MAX_CLIENTS;
use qa_content::q3::base::shared::definitions::Product;
use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_world::save::value::SaveReader;
use qa_world::WorldError;

use super::super::q3_ballistics::{
    read_q3_projectile_states, read_q3_weapon_statistics, Q3ProjectilePhase, Q3ProjectileState, Q3WeaponStatistics,
};
use super::q3::Q3SelectedArsenalCheckpoint;
use super::q3_source::{LegacyImport, LegacyProjectileImport, Q3SelectedSource, Q3SourceError};

/// Legacy Q3 source save words (donor `LegacyQ3Source`).
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyQ3Source {
    /// Save time in milliseconds.
    pub milliseconds: i32,
    /// Random seed.
    pub random_seed: i32,
    /// Projectile states.
    pub projectiles: Vec<Q3ProjectileState>,
    /// Weapon statistics.
    pub statistics: Vec<Q3WeaponStatistics>,
    /// Hook-latch actors.
    pub hook_held: Vec<OwnedActor>,
}

/// Legacy Q3 client for `restore_legacy` (donor `{ actor, state }`
/// element).
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyQ3Client {
    /// Client actor.
    pub actor: OwnedActor,
    /// Arsenal checkpoint.
    pub state: Q3SelectedArsenalCheckpoint,
}

/// Read legacy Q3 source save words.
pub fn read_legacy_q3_source<'a>(
    reader: SaveReader<'a>,
    owner: &dyn Fn(SaveReader<'a>) -> Result<OwnedActor, WorldError>,
    reference: &dyn Fn(SavedActorId) -> Result<ActorId, WorldError>,
) -> Result<LegacyQ3Source, WorldError> {
    let milliseconds = reader.field("milliseconds").finite()? as i32;
    let random_seed = reader.field("randomSeed").integer(i64::MIN)? as i32;
    let projectiles = read_q3_projectile_states(reader.field("projectiles"), owner, reference)?;
    let statistics = read_q3_weapon_statistics(reader.field("weaponStatistics"), owner)?;
    let hook_field = reader.field("hookHeld");
    let hook_held = if hook_field.is_missing() {
        Vec::new()
    } else {
        hook_field.list(owner)?
    };
    Ok(LegacyQ3Source {
        milliseconds,
        random_seed,
        projectiles,
        statistics,
        hook_held,
    })
}

/// Legacy map for the arsenal migration (donor `"q1" | "q2"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMap {
    /// Quake map.
    Q1,
    /// Quake II map.
    Q2,
}

/// Migrate a legacy Q3 arsenal checkpoint onto its expansion supply
/// profile.
pub fn migrate_legacy_q3_arsenal(
    checkpoint: &Q3SelectedArsenalCheckpoint,
    map: LegacyMap,
) -> Result<Q3SelectedArsenalCheckpoint, Q3SourceError> {
    let profile = match map {
        LegacyMap::Q1 => q1_q3_supply_profile(),
        LegacyMap::Q2 => q2_q3_supply_profile(),
    };
    if checkpoint.runtime.product != Product::Baseq3
        || checkpoint.supply_profile.as_deref() != Some(profile.id.as_str())
    {
        return Err(Q3SourceError::LegacyProfileMismatch);
    }
    let mut migrated = checkpoint.clone();
    migrated.supply_profile = Some(expansion_source_supply(&profile).id);
    Ok(migrated)
}

/// One-time checkpoint import; all subsequent execution belongs to the
/// original missile runtime.
pub(crate) fn restore_legacy_q3_source(source: &Q3SelectedSource, saved: &LegacyQ3Source) -> Result<(), Q3SourceError> {
    let host = source.host();
    let records = source.records();
    let game = source.game();
    if host.product() != Product::Baseq3 {
        return Err(Q3SourceError::LegacyRequiresBase);
    }
    let mut actors: HashSet<ActorId> = HashSet::new();
    let mut hooks: HashSet<ActorId> = HashSet::new();
    let mut references: HashSet<ActorId> = HashSet::new();
    for state in &saved.projectiles {
        let Some(owned) = host.actors().resolve_owned(&state.base.actor) else {
            return Err(Q3SourceError::LegacyProjectileBody);
        };
        if owned.owner() != &host.provider()
            || !actors.insert(state.base.actor.clone())
            || records.native_by_actor(Some(&state.base.actor)).is_some()
            || host.bodies().read(&state.base.actor).is_none()
        {
            return Err(Q3SourceError::LegacyProjectileBody);
        }
        references.insert(state.base.owner.clone());
        if let Some(pass) = &state.base.pass {
            references.insert(pass.clone());
        }
        if matches!(state.phase, Q3ProjectilePhase::Attached { .. }) && state.base.weapon != 10 {
            return Err(Q3SourceError::LegacyAttachedHook);
        }
        if state.base.weapon == 10 {
            if hooks.contains(&state.base.owner) || host.player(&state.base.owner).is_none() {
                return Err(Q3SourceError::LegacyHookPlayer);
            }
            hooks.insert(state.base.owner.clone());
        }
        match &state.phase {
            Q3ProjectilePhase::Attached {
                target: Some(target), ..
            } => {
                references.insert(target.clone());
            }
            Q3ProjectilePhase::Event { impact, .. } => {
                if let Some(target) = &impact.target {
                    references.insert(target.clone());
                }
            }
            _ => {}
        }
    }
    let statistics_actors: Vec<ActorId> = saved.statistics.iter().map(|value| value.actor.id().clone()).collect();
    let hook_held: Vec<ActorId> = saved.hook_held.iter().map(|actor| actor.id().clone()).collect();
    for entries in [&statistics_actors, &hook_held] {
        let unique: HashSet<&ActorId> = entries.iter().collect();
        if unique.len() != entries.len() || entries.iter().any(|actor| host.player(actor).is_none()) {
            return Err(Q3SourceError::LegacyClientContinuations);
        }
        for actor in entries.iter() {
            references.insert((*actor).clone());
        }
    }
    let owned_actors: Vec<OwnedActor> = actors
        .iter()
        .filter_map(|actor| host.actors().resolve_owned(actor))
        .collect();
    let foreign: Vec<ActorId> = references
        .into_iter()
        .filter(|actor| {
            host.actors().is_live(actor)
                && records.native_by_actor(Some(actor)).is_none()
                && !owned_actors.iter().any(|owned| owned.id() == actor)
        })
        .collect();
    if foreign.iter().any(|actor| host.bodies().read(actor).is_none()) {
        return Err(Q3SourceError::LegacyReferenceBody);
    }
    let non_players = foreign.iter().filter(|actor| host.player(actor).is_none()).count();
    let mut slots = game.free_entity_slots();
    slots.retain(|slot| *slot >= MAX_CLIENTS as i32 && *slot < 1022);
    slots.sort_unstable();
    if slots.len() < actors.len() + non_players {
        return Err(Q3SourceError::LegacyCapacity);
    }
    let mut imports = Vec::with_capacity(saved.projectiles.len());
    for (state, slot) in saved.projectiles.iter().zip(slots.iter()) {
        let Some(owned) = host.actors().resolve_owned(&state.base.actor) else {
            return Err(Q3SourceError::LegacyProjectileBody);
        };
        records.adopt(*slot as usize, owned)?;
        imports.push(LegacyProjectileImport {
            slot: *slot,
            state: state.clone(),
        });
    }
    for actor in foreign {
        source.project_actor(&actor)?;
    }
    game.import_legacy(&LegacyImport {
        projectiles: imports,
        statistics: saved.statistics.clone(),
        hook_held: saved.hook_held.clone(),
        milliseconds: saved.milliseconds,
    })?;
    source.random().borrow_mut().reset(saved.random_seed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;

    #[test]
    fn migrates_legacy_arsenal_profile() {
        let profile = q1_q3_supply_profile();
        let checkpoint = Q3SelectedArsenalCheckpoint {
            supply_profile: Some(profile.id.clone()),
            arsenal: qa_world::movement::types::ArsenalState {
                provider: qa_core::identity::ProviderId::new("q3", "base"),
                active_weapon: None,
                ammo: Vec::new(),
                state: qa_world::movement::types::WeaponState::Q3 {
                    source_weapon: 2,
                    state: 0,
                    time_milliseconds: 0,
                },
            },
            runtime: qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState {
                product: Product::Baseq3,
                max_health: 100.0,
                spectator: false,
                persistent_powerup_tag: 0,
                holdable_item: 0,
                holdable_tag: 0,
                respawned: false,
                use_item_held: false,
                event_sequence: 0,
                fractional_milliseconds: 0.0,
                external_slot: qa_world::movement::q3::weapon::Q3ExternalWeaponSlot::Active,
                requested_weapon: None,
            },
            torso_animation: 0,
            last_fire_milliseconds: None,
            pending_use: None,
        };
        let migrated = migrate_legacy_q3_arsenal(&checkpoint, LegacyMap::Q1).unwrap();
        assert_eq!(
            migrated.supply_profile.as_deref(),
            Some(expansion_source_supply(&profile).id.as_str())
        );
        assert!(migrate_legacy_q3_arsenal(&checkpoint, LegacyMap::Q2).is_err());
    }

    #[test]
    fn reads_empty_legacy_save() {
        use qa_world::save::value::{arr, int, num, obj, SaveJson, SaveReader};
        let saved = obj(vec![
            ("milliseconds", num(900.0)),
            ("randomSeed", int(77)),
            ("projectiles", arr(Vec::new())),
            ("weaponStatistics", arr(Vec::new())),
        ]);
        let owner = IdentityOwner::create("q3-legacy-read-test").unwrap();
        let actor = owner.actor(1, 1);
        let owned = owner
            .owned_actor(&actor, qa_core::identity::ProviderId::new("q3", "test"))
            .unwrap();
        let read =
            read_legacy_q3_source(SaveReader::new(&saved), &|_| Ok(owned.clone()), &|_| Ok(actor.clone())).unwrap();
        assert_eq!(read.milliseconds, 900);
        assert_eq!(read.random_seed, 77);
        assert!(read.projectiles.is_empty());
        assert!(read.statistics.is_empty());
        assert!(read.hook_held.is_empty());
        let _ = SaveJson::Null;
    }
}
