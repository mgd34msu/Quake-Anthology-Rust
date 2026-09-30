//! Q2 rerelease deferred monster damage accumulation.
//!
//! Donor: `src/compat/q2/rerelease/deferred-damage.ts` — bridges the DLL's
//! damage accumulator and deferred pain/death reactions over synthetic
//! edict records.

use std::collections::HashMap;

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestAllocationOptions};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

/// Deferred damage failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DeferredDamageError {
    /// Native monster accumulation has no damage call provenance.
    #[error("Native monster accumulation has no damage call provenance")]
    NoProvenance,
    /// Invalid native monster mod_t.
    #[error("Invalid native monster mod_t")]
    BadMod,
    /// Missing native monster MOD.
    #[error("Missing native monster MOD")]
    MissingMod,
    /// Unclassified native monster damage cause.
    #[error("Unclassified native monster damage cause")]
    UnclassifiedCause,
    /// Native damage references a stale actor.
    #[error("Native damage references a stale actor")]
    StaleActor,
    /// Native monster accumulation has a null edict pointer.
    #[error("Native monster accumulation has a null edict pointer")]
    NullEdict,
    /// Saved native damage actor generation changed.
    #[error("Saved native damage actor generation changed")]
    GenerationChanged,
    /// Saved native damage actor is not live.
    #[error("Saved native damage actor is not live")]
    ActorNotLive,
    /// Saved monster has no native source slot.
    #[error("Saved monster has no native source slot")]
    NoSourceSlot,
    /// Invalid saved native monster mod_t.
    #[error("Invalid saved native monster mod_t")]
    BadSavedMod,
    /// Saved pending damage target is not a monster.
    #[error("Saved pending damage target is not a monster")]
    NotAMonster,
    /// Missing saved native damage reference.
    #[error("Missing saved native damage reference")]
    MissingReference,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Monster accumulator offsets (retail world profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonsterAccumulator {
    /// Attacker pointer.
    pub attacker: usize,
    /// Inflictor pointer.
    pub inflictor: usize,
    /// Blood counter.
    pub blood: usize,
    /// Knockback counter.
    pub knockback: usize,
    /// Damage point.
    pub point: usize,
    /// `mod_t` bytes.
    pub modem: usize,
}

/// Retail accumulator offsets.
#[must_use]
pub const fn retail_accumulator() -> MonsterAccumulator {
    MonsterAccumulator {
        attacker: 3120,
        inflictor: 3128,
        blood: 3136,
        knockback: 3140,
        point: 3144,
        modem: 3156,
    }
}

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    /// Direct hit.
    Direct,
    /// Radius damage.
    Radius,
}

/// Classified damage cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageCause {
    /// Canonical means of death.
    pub means_of_death: i32,
    /// Damage flags.
    pub damage_flags: i32,
    /// Native ordinal.
    pub native_id: u8,
    /// Friendly fire.
    pub friendly_fire: bool,
    /// No point loss.
    pub no_point_loss: bool,
}

/// Classify a rerelease native cause id.
#[must_use]
pub fn canonical_cause_from_native(id: u8, friendly_fire: bool) -> Option<i32> {
    let id = i32::from(id);
    if id > 58 {
        return None;
    }
    let canonical = if id < 22 {
        id
    } else if id == 22 {
        57
    } else if id <= 56 {
        id - 1
    } else if id == 57 {
        56
    } else {
        58
    };
    Some(canonical + if friendly_fire { 0x800_0000 } else { 0 })
}

/// Attack provenance over local actor keys.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence number.
    pub sequence: u64,
    /// Time in milliseconds.
    pub time_ms: i64,
    /// Attacker actor, if any.
    pub attacker: Option<u32>,
    /// Inflictor actor, if any.
    pub inflictor: Option<u32>,
    /// Originating projectile, if any.
    pub originating_projectile: Option<u32>,
    /// Weapon item, if any.
    pub weapon: Option<String>,
    /// Damage cause.
    pub cause: DamageCause,
}

/// One damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Target actor key.
    pub target: u32,
    /// Amount.
    pub amount: f32,
    /// Knockback.
    pub knockback: f32,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
    /// Attack provenance.
    pub attack: AttackProvenance,
}

/// Native damage arguments at the intercepted entry.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeDamageArgs {
    /// Attacker actor key.
    pub attacker: u32,
    /// Inflictor actor key.
    pub inflictor: u32,
    /// Direction.
    pub direction: Vec3,
    /// Point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Amount.
    pub amount: f32,
    /// Knockback.
    pub knockback: f32,
    /// Damage flags.
    pub damage_flags: i32,
    /// `mod_t` bytes.
    pub modem: [u8; 3],
}

/// Deferred reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReaction {
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// One deferred source reaction.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceReaction {
    /// Effective request with accumulator knockback and point.
    pub request: DamageRequest,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Applied damage (accumulated blood).
    pub applied_damage: i32,
}

/// Saved actor reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedDamageActor {
    /// Native slot with generation.
    Native {
        /// Edict slot.
        slot: u32,
        /// Generation.
        generation: i32,
    },
    /// Shared actor.
    Shared(SavedActorId),
}

/// Saved attack checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackCheckpoint {
    /// Sequence number.
    pub sequence: u64,
    /// Time in milliseconds.
    pub time_ms: i64,
    /// Weapon item.
    pub weapon: Option<String>,
    /// Damage cause.
    pub cause: DamageCause,
    /// Attacker checkpoint.
    pub attacker: Option<SavedActorId>,
    /// Inflictor checkpoint.
    pub inflictor: Option<SavedActorId>,
    /// Projectile checkpoint.
    pub originating_projectile: Option<SavedActorId>,
}

/// One saved deferred damage record.
#[derive(Debug, Clone, PartialEq)]
pub struct DeferredDamageSave {
    /// Target actor.
    pub target: SavedDamageActor,
    /// Attack checkpoint.
    pub attack: AttackCheckpoint,
    /// Actor references.
    pub references: Vec<(SavedActorId, SavedDamageActor)>,
    /// Amount.
    pub amount: f32,
    /// Knockback.
    pub knockback: f32,
    /// Direction.
    pub direction: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
    /// Accumulated blood.
    pub blood: i32,
    /// Accumulated knockback.
    pub acc_knockback: i32,
    /// Accumulated point.
    pub point: Vec3,
    /// `mod_t` bytes.
    pub modem: [u8; 3],
    /// Attacker slot.
    pub attacker_slot: u32,
    /// Inflictor slot.
    pub inflictor_slot: u32,
}

struct Tracked {
    generation: i32,
    pending: Option<DamageRequest>,
}

struct CallFrame {
    stack: u64,
    request: DamageRequest,
}

/// Synthetic edict record backing.
struct EdictRecord {
    address: GuestAddress,
    generation: i32,
    svflags: u32,
    health: i32,
    inuse: bool,
    live: bool,
}

/// Observes the DLL's accumulator and callback boundary; the DLL retains
/// all monster logic and stores. Headless port: entry observers become
/// explicit notification methods.
pub struct RereleaseDeferredDamage {
    /// Guest memory backing synthetic edict records.
    pub memory: SparseGuestMemory,
    /// Accumulator offsets.
    pub accumulator: MonsterAccumulator,
    /// Record stride.
    pub stride: usize,
    records: HashMap<u32, EdictRecord>,
    actors: HashMap<u32, u32>,
    tracked: HashMap<u32, Tracked>,
    calls: Vec<CallFrame>,
    next_actor: u32,
    sequence: u64,
}

impl RereleaseDeferredDamage {
    /// Create over guest memory.
    #[must_use]
    pub fn new(memory: SparseGuestMemory) -> Self {
        Self {
            memory,
            accumulator: retail_accumulator(),
            stride: 3688,
            records: HashMap::new(),
            actors: HashMap::new(),
            tracked: HashMap::new(),
            calls: Vec::new(),
            next_actor: 1,
            sequence: 1,
        }
    }

    /// Register a synthetic edict record, returning its actor key.
    pub fn register(
        &mut self,
        slot: u32,
        generation: i32,
        svflags: u32,
        health: i32,
    ) -> Result<u32, DeferredDamageError> {
        let address = self.memory.allocate(&GuestAllocationOptions::bytes(self.stride))?;
        let actor = self.next_actor;
        self.next_actor += 1;
        self.records.insert(
            actor,
            EdictRecord {
                address,
                generation,
                svflags,
                health,
                inuse: true,
                live: true,
            },
        );
        self.actors.insert(actor, slot);
        Ok(actor)
    }

    /// Mark an actor dead.
    pub fn kill(&mut self, actor: u32) {
        if let Some(record) = self.records.get_mut(&actor) {
            record.live = false;
            record.inuse = false;
        }
    }

    fn at(&self, actor: u32, offset: usize) -> Result<GuestAddress, DeferredDamageError> {
        let record = self.records.get(&actor).ok_or(DeferredDamageError::StaleActor)?;
        Ok(self.memory.offset(record.address, offset as i64)?)
    }

    fn valid(&self, actor: u32, tracked: &Tracked) -> bool {
        self.records
            .get(&actor)
            .is_some_and(|record| record.live && record.generation == tracked.generation && record.inuse)
    }

    /// Whether a record is a damage-accumulating monster.
    #[must_use]
    pub fn is_monster(&self, actor: u32) -> bool {
        self.records.get(&actor).is_some_and(|record| (record.svflags & 4) != 0)
    }

    /// Track a monster record for accumulation.
    pub fn track(&mut self, actor: u32) {
        if !self.is_monster(actor) || self.tracked.contains_key(&actor) {
            return;
        }
        if let Some(record) = self.records.get(&actor) {
            self.tracked.insert(
                actor,
                Tracked {
                    generation: record.generation,
                    pending: None,
                },
            );
        }
    }

    /// Emulate the damage-entry observer.
    pub fn observe_damage_entry(
        &mut self,
        stack: u64,
        target: u32,
        current: Option<DamageRequest>,
        native: &NativeDamageArgs,
    ) -> Result<(), DeferredDamageError> {
        while self.calls.last().is_some_and(|call| call.stack <= stack) {
            self.calls.pop();
        }
        if !self.is_monster(target) {
            return Ok(());
        }
        if !self.records.get(&target).is_some_and(|record| record.live) {
            return Ok(());
        }
        self.track(target);
        let request = match current {
            Some(request) => request,
            None => self.native_request(target, native)?,
        };
        self.calls.push(CallFrame { stack, request });
        Ok(())
    }

    fn native_request(&mut self, target: u32, args: &NativeDamageArgs) -> Result<DamageRequest, DeferredDamageError> {
        for actor in [args.attacker, args.inflictor] {
            if !self.records.get(&actor).is_some_and(|record| record.live) {
                return Err(DeferredDamageError::StaleActor);
            }
        }
        let cause_id = canonical_cause_from_native(args.modem[0], args.modem[1] != 0)
            .ok_or(DeferredDamageError::UnclassifiedCause)?;
        let sequence = self.sequence;
        self.sequence += 1;
        Ok(DamageRequest {
            target,
            amount: args.amount,
            knockback: args.knockback,
            point: args.point,
            direction: args.direction,
            normal: args.normal,
            delivery: if (args.damage_flags & 1) != 0 {
                DamageDelivery::Radius
            } else {
                DamageDelivery::Direct
            },
            attack: AttackProvenance {
                sequence,
                time_ms: 0,
                attacker: Some(args.attacker),
                inflictor: Some(args.inflictor),
                originating_projectile: None,
                weapon: None,
                cause: DamageCause {
                    means_of_death: cause_id,
                    damage_flags: args.damage_flags,
                    native_id: args.modem[0],
                    friendly_fire: args.modem[1] != 0,
                    no_point_loss: args.modem[2] != 0,
                },
            },
        })
    }

    /// Emulate the `mod+2` write observer: attach call provenance.
    pub fn notify_mod_write(&mut self, stack: u64, actor: u32) -> Result<(), DeferredDamageError> {
        let Some(tracked) = self.tracked.get(&actor) else {
            return Ok(());
        };
        if !self.valid(actor, tracked) {
            self.release(actor);
            return Ok(());
        }
        let Some(call) = self
            .calls
            .iter()
            .rev()
            .find(|call| call.stack >= stack && call.request.target == actor)
        else {
            return Err(DeferredDamageError::NoProvenance);
        };
        let request = call.request.clone();
        if let Some(tracked) = self.tracked.get_mut(&actor) {
            tracked.pending = Some(request);
        }
        Ok(())
    }

    /// Emulate the blood write observer: zero blood clears pending damage.
    pub fn notify_blood_write(&mut self, actor: u32) -> Result<(), DeferredDamageError> {
        let blood = self.memory.read_i32(self.at(actor, self.accumulator.blood)?)?;
        if blood == 0 {
            if let Some(tracked) = self.tracked.get_mut(&actor) {
                tracked.pending = None;
            }
        }
        Ok(())
    }

    /// Write accumulator blood directly.
    pub fn write_blood(&mut self, actor: u32, blood: i32) -> Result<(), DeferredDamageError> {
        let at = self.at(actor, self.accumulator.blood)?;
        self.memory.write_i32(at, blood)?;
        Ok(())
    }

    /// Emulate the deferred-reaction entry observer.
    pub fn observe_pain_entry(&mut self, actor: u32) -> Result<Option<SourceReaction>, DeferredDamageError> {
        let Some(tracked) = self.tracked.get(&actor) else {
            return Ok(None);
        };
        if !self.valid(actor, tracked) || tracked.pending.is_none() {
            return Ok(None);
        }
        let blood = self.memory.read_i32(self.at(actor, self.accumulator.blood)?)?;
        if blood == 0 {
            return Ok(None);
        }
        let knockback = self.memory.read_i32(self.at(actor, self.accumulator.knockback)?)?;
        let point = self.memory.read_f32x3(self.at(actor, self.accumulator.point)?)?;
        let health = self.records.get(&actor).map_or(0, |record| record.health);
        let mut pending = self
            .tracked
            .get_mut(&actor)
            .expect("tracked")
            .pending
            .take()
            .expect("pending");
        pending.knockback = knockback as f32;
        pending.point = point;
        Ok(Some(SourceReaction {
            request: pending,
            reaction: if health <= 0 {
                DamageReaction::Death
            } else {
                DamageReaction::Pain
            },
            applied_damage: blood,
        }))
    }

    /// Release tracking for an actor.
    pub fn release(&mut self, actor: u32) {
        self.tracked.remove(&actor);
    }

    /// Clear all tracking and call frames.
    pub fn clear(&mut self) {
        self.tracked.clear();
        self.calls.clear();
    }

    /// Save a live actor reference.
    #[must_use]
    pub fn save_actor(&self, actor: u32) -> SavedDamageActor {
        match (self.actors.get(&actor), self.records.get(&actor)) {
            (Some(slot), Some(record)) if record.live => SavedDamageActor::Native {
                slot: *slot,
                generation: record.generation,
            },
            _ => SavedDamageActor::Shared(SavedActorId {
                slot: actor,
                generation: 0,
            }),
        }
    }

    /// Resolve a saved actor through a domain lookup.
    pub fn restore_actor(
        &self,
        saved: &SavedDamageActor,
        resolve: &dyn Fn(u32, i32) -> Option<u32>,
    ) -> Result<u32, DeferredDamageError> {
        match saved {
            SavedDamageActor::Shared(id) => {
                resolve(id.slot, id.generation as i32).ok_or(DeferredDamageError::ActorNotLive)
            }
            SavedDamageActor::Native { slot, generation } => {
                let actor = resolve(*slot, *generation).ok_or(DeferredDamageError::ActorNotLive)?;
                let record = self.records.get(&actor).ok_or(DeferredDamageError::ActorNotLive)?;
                if record.generation != *generation {
                    return Err(DeferredDamageError::GenerationChanged);
                }
                Ok(actor)
            }
        }
    }

    /// Save pending deferred damage.
    pub fn save(&mut self) -> Result<Vec<DeferredDamageSave>, DeferredDamageError> {
        let mut saved = Vec::new();
        let actors: Vec<u32> = self.tracked.keys().copied().collect();
        for actor in actors {
            let tracked = self.tracked.get(&actor).expect("tracked");
            let (Some(request), true) = (tracked.pending.clone(), self.valid(actor, tracked)) else {
                continue;
            };
            let mut references = Vec::new();
            for party in [
                request.attack.attacker,
                request.attack.inflictor,
                request.attack.originating_projectile,
            ]
            .into_iter()
            .flatten()
            {
                references.push((
                    SavedActorId {
                        slot: party,
                        generation: 0,
                    },
                    self.save_actor(party),
                ));
            }
            let blood = self.memory.read_i32(self.at(actor, self.accumulator.blood)?)?;
            let acc_knockback = self.memory.read_i32(self.at(actor, self.accumulator.knockback)?)?;
            let point = self.memory.read_f32x3(self.at(actor, self.accumulator.point)?)?;
            let modem = self.memory.copy(self.at(actor, self.accumulator.modem)?, 3)?;
            let attacker = self
                .memory
                .read_pointer(self.at(actor, self.accumulator.attacker)?)?
                .ok_or(DeferredDamageError::NullEdict)?;
            let inflictor = self
                .memory
                .read_pointer(self.at(actor, self.accumulator.inflictor)?)?
                .ok_or(DeferredDamageError::NullEdict)?;
            saved.push(DeferredDamageSave {
                target: self.save_actor(actor),
                attack: AttackCheckpoint {
                    sequence: request.attack.sequence,
                    time_ms: request.attack.time_ms,
                    weapon: request.attack.weapon.clone(),
                    cause: request.attack.cause,
                    attacker: request.attack.attacker.map(|slot| SavedActorId { slot, generation: 0 }),
                    inflictor: request
                        .attack
                        .inflictor
                        .map(|slot| SavedActorId { slot, generation: 0 }),
                    originating_projectile: request
                        .attack
                        .originating_projectile
                        .map(|slot| SavedActorId { slot, generation: 0 }),
                },
                references,
                amount: request.amount,
                knockback: request.knockback,
                direction: request.direction,
                normal: request.normal,
                delivery: request.delivery,
                blood,
                acc_knockback,
                point,
                modem: [modem[0], modem[1], modem[2]],
                attacker_slot: attacker.offset as u32,
                inflictor_slot: inflictor.offset as u32,
            });
        }
        Ok(saved)
    }

    /// Restore saved deferred damage through a domain lookup.
    pub fn restore(
        &mut self,
        saved: &[DeferredDamageSave],
        resolve: &dyn Fn(u32, i32) -> Option<u32>,
        slot_address: &dyn Fn(u32) -> Option<GuestAddress>,
    ) -> Result<(), DeferredDamageError> {
        for state in saved {
            let target = self.restore_actor(&state.target, resolve)?;
            let attacker = slot_address(state.attacker_slot).ok_or(DeferredDamageError::NullEdict)?;
            let inflictor = slot_address(state.inflictor_slot).ok_or(DeferredDamageError::NullEdict)?;
            let at = self.at(target, self.accumulator.attacker)?;
            self.memory.write_pointer(at, Some(attacker))?;
            let at = self.at(target, self.accumulator.inflictor)?;
            self.memory.write_pointer(at, Some(inflictor))?;
            let at = self.at(target, self.accumulator.blood)?;
            self.memory.write_i32(at, state.blood)?;
            let at = self.at(target, self.accumulator.knockback)?;
            self.memory.write_i32(at, state.acc_knockback)?;
            let point = self.at(target, self.accumulator.point)?;
            self.memory.write_f32(point, state.point.x)?;
            self.memory.write_f32(self.memory.offset(point, 4)?, state.point.y)?;
            self.memory.write_f32(self.memory.offset(point, 8)?, state.point.z)?;
            let modem = self.at(target, self.accumulator.modem)?;
            self.memory.write(modem, &state.modem)?;
            self.track(target);
            if !self.tracked.contains_key(&target) {
                return Err(DeferredDamageError::NotAMonster);
            }
            let attack = |saved_id: Option<SavedActorId>| {
                saved_id
                    .map(|id| {
                        state
                            .references
                            .iter()
                            .find(|(actor, _)| *actor == id)
                            .map(|(_, reference)| self.restore_actor(reference, resolve))
                            .ok_or(DeferredDamageError::MissingReference)?
                    })
                    .transpose()
            };
            let pending = DamageRequest {
                target,
                amount: state.amount,
                knockback: state.knockback,
                direction: state.direction,
                point: state.point,
                normal: state.normal,
                delivery: state.delivery,
                attack: AttackProvenance {
                    sequence: state.attack.sequence,
                    time_ms: state.attack.time_ms,
                    attacker: attack(state.attack.attacker)?,
                    inflictor: attack(state.attack.inflictor)?,
                    originating_projectile: attack(state.attack.originating_projectile)?,
                    weapon: state.attack.weapon.clone(),
                    cause: state.attack.cause,
                },
            };
            self.tracked.get_mut(&target).expect("tracked").pending = Some(pending);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_tracker() -> RereleaseDeferredDamage {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "deferred-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        RereleaseDeferredDamage::new(memory)
    }

    fn args(attacker: u32, inflictor: u32) -> NativeDamageArgs {
        NativeDamageArgs {
            attacker,
            inflictor,
            direction: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            point: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
            amount: 25.0,
            knockback: 100.0,
            damage_flags: 0,
            modem: [3, 0, 0],
        }
    }

    #[test]
    fn accumulation_defers_until_reaction() {
        let mut tracker = test_tracker();
        let monster = tracker.register(7, 2, 4, 100).expect("monster");
        let shooter = tracker.register(1, 1, 0, 100).expect("shooter");
        tracker
            .observe_damage_entry(0x8000, monster, None, &args(shooter, shooter))
            .expect("entry");
        tracker.notify_mod_write(0x8000, monster).expect("mod");
        tracker.write_blood(monster, 25).expect("blood");
        let reaction = tracker.observe_pain_entry(monster).expect("pain").expect("reaction");
        assert_eq!(reaction.reaction, DamageReaction::Pain);
        assert_eq!(reaction.applied_damage, 25);
        assert_eq!(reaction.request.target, monster);
        assert_eq!(reaction.request.attack.cause.means_of_death, 3);
        assert!(tracker.observe_pain_entry(monster).expect("again").is_none());
        tracker.write_blood(monster, 0).expect("clear");
        tracker.notify_blood_write(monster).expect("notify");
    }

    #[test]
    fn provenance_save_and_restore_round_trip() {
        let mut tracker = test_tracker();
        let monster = tracker.register(7, 2, 4, 60).expect("monster");
        let shooter = tracker.register(1, 1, 0, 100).expect("shooter");
        assert_eq!(tracker.notify_mod_write(0x8000, monster), Ok(()));
        tracker
            .observe_damage_entry(0x8000, monster, None, &args(shooter, shooter))
            .expect("entry");
        assert_eq!(
            tracker.notify_mod_write(0x9000, monster).unwrap_err(),
            DeferredDamageError::NoProvenance
        );
        tracker.notify_mod_write(0x7000, monster).expect("mod");
        let shooter_address = tracker.at(shooter, 0).expect("shooter");
        let at = tracker.at(monster, tracker.accumulator.attacker).expect("at");
        tracker.memory.write_pointer(at, Some(shooter_address)).expect("link");
        let at = tracker.at(monster, tracker.accumulator.inflictor).expect("at");
        tracker.memory.write_pointer(at, Some(shooter_address)).expect("link");
        tracker.write_blood(monster, 12).expect("blood");
        let saved = tracker.save().expect("save");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].blood, 12);
        tracker.clear();
        assert!(tracker.save().expect("empty").is_empty());
        let resolve = |slot: u32, generation: i32| {
            (slot == 7 && generation == 2)
                .then_some(monster)
                .or_else(|| (slot == 1 && generation == 1).then_some(shooter))
        };
        let slots = |slot: u32| {
            if slot == shooter_address.offset as u32 {
                Some(shooter_address)
            } else {
                None
            }
        };
        tracker.restore(&saved, &resolve, &slots).expect("restore");
        let reaction = tracker.observe_pain_entry(monster).expect("pain").expect("reaction");
        assert_eq!(reaction.applied_damage, 12);
    }
}
