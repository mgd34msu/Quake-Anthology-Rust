//! Q2 rerelease foreign actor projections and staged damage.
//!
//! Donor: `src/compat/q2/rerelease/foreign-actors.ts` — bridges shared
//! actors into native slots with intercepted damage and armor stages.

use std::collections::HashMap;

use qa_core::identity::SavedActorId;
use qa_core::math::Vec3;
use qa_guest::core::contracts::{GuestAddress, GuestAllocationOptions};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use qa_world::combat::{Delivery, Reaction};
use thiserror::Error;

use super::layouts::{edict_layout, field_offset};

/// Foreign actor failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ForeignActorError {
    /// Cannot project a stale foreign actor.
    #[error("Cannot project a stale foreign actor")]
    StaleActor,
    /// Native G_Spawn returned null.
    #[error("Native G_Spawn returned null")]
    SpawnFailed,
    /// Foreign native projection requires an existing body binding.
    #[error("Foreign native projection requires an existing body binding")]
    NoBody,
    /// Native damage requires the API2023 mod_t ABI.
    #[error("Native damage requires the API2023 mod_t ABI")]
    BadMod,
    /// Missing native damage ID.
    #[error("Missing native damage ID")]
    MissingDamageId,
    /// Unclassified native damage cause.
    #[error("Unclassified native damage cause")]
    UnclassifiedCause,
    /// Native damage references a stale edict.
    #[error("Native damage references a stale edict")]
    StaleEdict,
    /// Native armor stage has no live actor.
    #[error("Native armor stage has no live actor")]
    StageWithoutActor,
    /// Native armor stage already has an owner.
    #[error("Native armor stage already has an owner")]
    StageOwned,
    /// Native power stage has no active source damage request.
    #[error("Native power stage has no active source damage request")]
    PowerWithoutRequest,
    /// Native regular stage has no active source damage request.
    #[error("Native regular stage has no active source damage request")]
    RegularWithoutRequest,
    /// Native target lacks a combat binding.
    #[error("Native target lacks a combat binding")]
    NoCombatBinding,
    /// Native damage execution requires a native target.
    #[error("Native damage execution requires a native target")]
    ForeignTarget,
    /// Damage cause has no native representation.
    #[error("Damage cause has no native representation")]
    NoNativeCause,
    /// Saved foreign projection actor is no longer live.
    #[error("Saved foreign projection actor is no longer live")]
    RestoreNotLive,
    /// Native projection restore is already active.
    #[error("Native projection restore is already active")]
    RestoreActive,
    /// Foreign native projections are closed.
    #[error("Foreign native projections are closed")]
    Closed,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Protection channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered protection.
    Powered,
}

/// Classified damage cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignCause {
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

/// One damage request over local actor keys.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignDamageRequest {
    /// Target actor key.
    pub target: u32,
    /// Attacker actor key.
    pub attacker: u32,
    /// Inflictor actor key.
    pub inflictor: u32,
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
    pub delivery: Delivery,
    /// Cause.
    pub cause: ForeignCause,
}

/// Native damage arguments at the intercepted entry.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignDamageArgs {
    /// Target slot.
    pub target_slot: u32,
    /// Attacker slot.
    pub attacker_slot: u32,
    /// Inflictor slot.
    pub inflictor_slot: u32,
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

/// Damage outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageOutcome {
    /// Reaction.
    pub reaction: Reaction,
    /// Applied damage.
    pub applied_damage: i32,
}

/// Body snapshot for projection sync.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectionBody {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
    /// Health.
    pub health: i32,
    /// Mass.
    pub mass: i32,
    /// Takes damage.
    pub takedamage: bool,
}

/// One native projection of a shared actor.
#[derive(Debug, Clone, PartialEq)]
struct Projection {
    actor: u32,
    slot: u32,
    generation: i32,
    address: GuestAddress,
    body: ProjectionBody,
    releasing: bool,
    syncing: bool,
}

struct DamageFrame {
    request: ForeignDamageRequest,
    entered: bool,
    stack: Option<u64>,
    observing: bool,
}

struct InventoryPublication {
    frame: usize,
    actor: u32,
    committed: bool,
}

/// Armor intercept: (request, amount, original) -> saved.
pub type ArmorIntercept = Box<dyn FnMut(&ForeignDamageRequest, f32, &dyn Fn() -> f32) -> f32>;

struct ArmorBinding {
    regular: Option<ArmorIntercept>,
    powered: Option<ArmorIntercept>,
}

/// Armor stage token for explicit unbind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorStageToken {
    /// Actor key.
    pub actor: u32,
    /// Channel.
    pub channel: ProtectionChannel,
}

/// Saved projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionSave {
    /// Native slot.
    pub slot: u32,
    /// Shared actor.
    pub actor: SavedActorId,
}

/// Native slots are ABI projections. Existing shared actors retain their
/// bodies, inventories and combat bindings. Headless port: interception
/// points become explicit notification methods.
pub struct RereleaseForeignActors {
    /// Guest memory backing projection records.
    pub memory: SparseGuestMemory,
    /// Record stride.
    pub stride: usize,
    projections: HashMap<u32, Projection>,
    slots: HashMap<u32, u32>,
    live: HashMap<u32, bool>,
    bodies: HashMap<u32, ProjectionBody>,
    damage_frames: Vec<DamageFrame>,
    inventory_publications: Vec<InventoryPublication>,
    armor_bindings: HashMap<u32, ArmorBinding>,
    /// Requests routed to the shared engine.
    pub shared_log: Vec<ForeignDamageRequest>,
    /// Requests executed natively.
    pub native_log: Vec<ForeignDamageRequest>,
    /// Scripted health deltas by actor for native execution.
    pub scripted_damage: HashMap<u32, i32>,
    /// Scripted reactions by actor for native execution.
    pub scripted_reaction: HashMap<u32, Reaction>,
    deferred: HashMap<u32, ForeignDamageRequest>,
    restoring: Option<HashMap<u32, SavedActorId>>,
    next_slot: u32,
    next_actor: u32,
    closed: bool,
    power_bypass: Option<u64>,
    power_pending: bool,
    offsets: ProjectionOffsets,
}

#[derive(Debug, Clone, Copy)]
struct ProjectionOffsets {
    origin: i64,
    velocity: i64,
    min: i64,
    max: i64,
}

impl RereleaseForeignActors {
    /// Create over guest memory.
    #[must_use]
    pub fn new(memory: SparseGuestMemory) -> Self {
        let edict = edict_layout();
        let at = |name: &str| field_offset(&edict, name).unwrap_or(0) as i64;
        Self {
            memory,
            stride: 3688,
            projections: HashMap::new(),
            slots: HashMap::new(),
            live: HashMap::new(),
            bodies: HashMap::new(),
            damage_frames: Vec::new(),
            inventory_publications: Vec::new(),
            armor_bindings: HashMap::new(),
            shared_log: Vec::new(),
            native_log: Vec::new(),
            scripted_damage: HashMap::new(),
            scripted_reaction: HashMap::new(),
            deferred: HashMap::new(),
            restoring: None,
            next_slot: 1000,
            next_actor: 1,
            closed: false,
            power_bypass: None,
            power_pending: false,
            offsets: ProjectionOffsets {
                origin: at("s.origin"),
                velocity: at("sv.velocity"),
                min: at("mins"),
                max: at("maxs"),
            },
        }
    }

    /// Register a shared actor, returning its key.
    pub fn register_actor(&mut self, body: ProjectionBody) -> u32 {
        let actor = self.next_actor;
        self.next_actor += 1;
        self.live.insert(actor, true);
        self.bodies.insert(actor, body);
        actor
    }

    /// Set actor liveness.
    pub fn set_live(&mut self, actor: u32, live: bool) {
        self.live.insert(actor, live);
    }

    /// Whether an actor is live.
    #[must_use]
    pub fn is_live(&self, actor: u32) -> bool {
        self.live.get(&actor).copied().unwrap_or(false)
    }

    /// Resolve an actor key from a native slot.
    pub fn actor_at_slot(
        &mut self,
        slot: u32,
        resolve: &dyn Fn(SavedActorId) -> Option<u32>,
    ) -> Result<Option<u32>, ForeignActorError> {
        if let Some(actor) = self.slots.get(&slot).copied() {
            return Ok(Some(actor));
        }
        if let Some(restoring) = &self.restoring {
            if let Some(saved) = restoring.get(&slot) {
                let actor = resolve(*saved).ok_or(ForeignActorError::RestoreNotLive)?;
                return Ok(Some(actor));
            }
        }
        Ok(None)
    }

    /// Look up a projection: `None` unknown, `Some(None)` releasing or
    /// dead, `Some(Some)` live.
    #[must_use]
    pub fn lookup(&self, slot: u32) -> Option<Option<u32>> {
        self.slots.get(&slot).copied().map(|actor| {
            let projection = self.projections.get(&actor).expect("projection");
            if projection.releasing || !self.is_live(actor) {
                None
            } else {
                Some(actor)
            }
        })
    }

    /// Project an actor into a native slot, syncing its body.
    pub fn address(&mut self, actor: u32) -> Result<GuestAddress, ForeignActorError> {
        if self.closed {
            return Err(ForeignActorError::Closed);
        }
        if !self.is_live(actor) {
            return Err(ForeignActorError::StaleActor);
        }
        if !self.projections.contains_key(&actor) {
            let slot = self.next_slot;
            self.next_slot += 1;
            let address = self.memory.allocate(&GuestAllocationOptions::bytes(self.stride))?;
            let body = self.bodies.get(&actor).copied().ok_or(ForeignActorError::NoBody)?;
            self.projections.insert(
                actor,
                Projection {
                    actor,
                    slot,
                    generation: 1,
                    address,
                    body,
                    releasing: false,
                    syncing: false,
                },
            );
            self.slots.insert(slot, actor);
        }
        let address = self.projections.get(&actor).expect("projection").address;
        if let Err(error) = self.sync(actor) {
            self.release_projection(actor);
            return Err(error);
        }
        Ok(address)
    }

    /// Synchronize every projection body.
    pub fn synchronize(&mut self) -> Result<(), ForeignActorError> {
        let actors: Vec<u32> = self.projections.keys().copied().collect();
        for actor in actors {
            self.sync(actor)?;
        }
        Ok(())
    }

    fn sync(&mut self, actor: u32) -> Result<(), ForeignActorError> {
        let Some(projection) = self.projections.get(&actor) else {
            return Ok(());
        };
        if projection.releasing || projection.syncing {
            return Ok(());
        }
        let body = self.bodies.get(&actor).copied().ok_or(ForeignActorError::NoBody)?;
        if let Some(projection) = self.projections.get_mut(&actor) {
            projection.syncing = true;
            projection.body = body;
        }
        let address = self.projections.get(&actor).expect("projection").address;
        let write_vec = |memory: &mut SparseGuestMemory, offset: i64, value: Vec3| {
            memory.write_f32(memory.offset(address, offset)?, value.x)?;
            memory.write_f32(memory.offset(address, offset + 4)?, value.y)?;
            memory.write_f32(memory.offset(address, offset + 8)?, value.z)?;
            Ok::<(), ForeignActorError>(())
        };
        write_vec(&mut self.memory, self.offsets.origin, body.origin)?;
        write_vec(&mut self.memory, self.offsets.velocity, body.velocity)?;
        write_vec(&mut self.memory, self.offsets.min, body.min)?;
        write_vec(&mut self.memory, self.offsets.max, body.max)?;
        if let Some(projection) = self.projections.get_mut(&actor) {
            projection.syncing = false;
        }
        Ok(())
    }

    /// Update a shared body.
    pub fn update_body(&mut self, actor: u32, body: ProjectionBody) {
        self.bodies.insert(actor, body);
    }

    /// Run an operation with inventory publication for an actor.
    pub fn with_inventory_publication<T>(
        &mut self,
        actor: u32,
        committed: bool,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let frame = self.damage_frames.len().wrapping_sub(1);
        self.inventory_publications.push(InventoryPublication {
            frame,
            actor,
            committed,
        });
        let result = operation(self);
        self.inventory_publications.pop();
        result
    }

    /// Whether damage to an actor is currently observed.
    #[must_use]
    pub fn observing_damage(&self, actor: u32) -> bool {
        self.damage_frames
            .last()
            .is_some_and(|frame| frame.observing && frame.request.target == actor)
    }

    /// Hand the current frame request to deferred tracking once.
    pub fn take_frame_request(&mut self, actor: u32) -> Option<ForeignDamageRequest> {
        let frame = self.damage_frames.last_mut()?;
        if frame.entered || frame.request.target != actor {
            return None;
        }
        frame.entered = true;
        Some(frame.request.clone())
    }

    /// Whether the current frame publishes inventory writes.
    #[must_use]
    pub fn current_publishes_inventory(&self) -> bool {
        let top = self.damage_frames.len().wrapping_sub(1);
        if self.damage_frames.get(top).is_none() {
            return true;
        }
        for scope in self.inventory_publications.iter().rev() {
            if scope.frame == top
                && self
                    .damage_frames
                    .get(top)
                    .is_some_and(|frame| frame.request.target == scope.actor)
            {
                return scope.committed;
            }
        }
        true
    }

    /// Emulate the damage continuation observer: stamp the frame stack.
    pub fn observe_damage_continuation(&mut self, stack: u64) {
        if let Some(frame) = self.damage_frames.last_mut() {
            if frame.stack.is_none() {
                frame.stack = Some(stack);
            }
        }
    }

    /// Emulate the power-armor entry observer for bypass tracking.
    pub fn observe_power_entry(&mut self, stack: u64) {
        if self.power_pending {
            self.power_bypass = Some(stack);
            self.power_pending = false;
        }
    }

    /// Incoming native damage at the intercepted entry.
    pub fn incoming_damage(&mut self, args: &ForeignDamageArgs) -> Result<(), ForeignActorError> {
        let target = self
            .slots
            .get(&args.target_slot)
            .copied()
            .ok_or(ForeignActorError::StaleEdict)?;
        let attacker = self
            .slots
            .get(&args.attacker_slot)
            .copied()
            .ok_or(ForeignActorError::StaleEdict)?;
        let inflictor = self
            .slots
            .get(&args.inflictor_slot)
            .copied()
            .ok_or(ForeignActorError::StaleEdict)?;
        let cause_id = canonical_cause_from_native(args.modem[0], args.modem[1] != 0)
            .ok_or(ForeignActorError::UnclassifiedCause)?;
        let request = ForeignDamageRequest {
            target,
            attacker,
            inflictor,
            amount: args.amount,
            knockback: args.knockback,
            direction: args.direction,
            point: args.point,
            normal: args.normal,
            delivery: if (args.damage_flags & 1) != 0 {
                Delivery::Radius
            } else {
                Delivery::Direct
            },
            cause: ForeignCause {
                means_of_death: cause_id,
                damage_flags: args.damage_flags,
                native_id: args.modem[0],
                friendly_fire: args.modem[1] != 0,
                no_point_loss: args.modem[2] != 0,
            },
        };
        if self.projections.contains_key(&target) {
            self.shared_log.push(request);
        } else {
            self.native_log.push(request.clone());
            self.damage_native(&request)?;
        }
        if self.projections.contains_key(&target) {
            self.sync(target)?;
        }
        Ok(())
    }

    /// Execute damage natively with staged armor interception.
    pub fn damage_native(&mut self, request: &ForeignDamageRequest) -> Result<DamageOutcome, ForeignActorError> {
        if !self.projections.contains_key(&request.target) {
            return Err(ForeignActorError::ForeignTarget);
        }
        let body = self
            .bodies
            .get(&request.target)
            .copied()
            .ok_or(ForeignActorError::NoCombatBinding)?;
        self.damage_frames.push(DamageFrame {
            request: request.clone(),
            entered: false,
            stack: None,
            observing: true,
        });
        let outcome = self.execute_native(request.target, body, request.amount);
        self.damage_frames.pop();
        outcome
    }

    fn execute_native(
        &mut self,
        target: u32,
        body: ProjectionBody,
        amount: f32,
    ) -> Result<DamageOutcome, ForeignActorError> {
        let request = self.damage_frames.last().expect("frame").request.clone();
        let mut remaining = amount;
        for channel in [ProtectionChannel::Powered, ProtectionChannel::Regular] {
            let has = self.armor_bindings.get(&target).is_some_and(|binding| match channel {
                ProtectionChannel::Regular => binding.regular.is_some(),
                ProtectionChannel::Powered => binding.powered.is_some(),
            });
            if !has {
                continue;
            }
            if channel == ProtectionChannel::Powered && !self.is_live(target) {
                return Err(ForeignActorError::StaleActor);
            }
            let saved = {
                let binding = self.armor_bindings.get_mut(&target).expect("binding");
                let intercept = match channel {
                    ProtectionChannel::Regular => binding.regular.as_mut(),
                    ProtectionChannel::Powered => binding.powered.as_mut(),
                }
                .expect("intercept");
                intercept(&request, remaining, &|| remaining * 0.5)
            };
            remaining -= saved;
            if !self.is_live(target) {
                return Err(ForeignActorError::StaleActor);
            }
        }
        let delta = self.scripted_damage.get(&target).copied().unwrap_or(remaining as i32);
        let reaction = self.scripted_reaction.get(&target).copied().unwrap_or_else(|| {
            if body.health - delta <= 0 {
                Reaction::Death
            } else if delta > 0 {
                Reaction::Pain
            } else {
                Reaction::None
            }
        });
        if let Some(body) = self.bodies.get_mut(&target) {
            body.health -= delta;
        }
        self.native_log.push(request);
        Ok(DamageOutcome {
            reaction,
            applied_damage: delta,
        })
    }

    /// Bind an armor stage intercept, returning an explicit unbind token.
    pub fn bind_armor_stage(
        &mut self,
        actor: u32,
        channel: ProtectionChannel,
        intercept: ArmorIntercept,
    ) -> Result<ArmorStageToken, ForeignActorError> {
        if !self.is_live(actor) {
            return Err(ForeignActorError::StageWithoutActor);
        }
        let binding = self.armor_bindings.entry(actor).or_insert_with(|| ArmorBinding {
            regular: None,
            powered: None,
        });
        let slot = match channel {
            ProtectionChannel::Regular => &mut binding.regular,
            ProtectionChannel::Powered => &mut binding.powered,
        };
        if slot.is_some() {
            return Err(ForeignActorError::StageOwned);
        }
        *slot = Some(intercept);
        Ok(ArmorStageToken { actor, channel })
    }

    /// Unbind an armor stage.
    pub fn unbind_armor_stage(&mut self, token: ArmorStageToken) {
        if let Some(binding) = self.armor_bindings.get_mut(&token.actor) {
            match token.channel {
                ProtectionChannel::Regular => binding.regular = None,
                ProtectionChannel::Powered => binding.powered = None,
            }
            if binding.regular.is_none() && binding.powered.is_none() {
                self.armor_bindings.remove(&token.actor);
            }
        }
    }

    /// Run the original power stage with bypass tracking.
    pub fn run_power_original(&mut self, stack: u64, original: &dyn Fn() -> f32) -> f32 {
        self.power_pending = true;
        let saved = original();
        self.observe_power_entry(stack);
        saved
    }

    /// Whether the power stage at a stack is bypassed.
    #[must_use]
    pub fn power_bypassed(&self, stack: u64) -> bool {
        self.power_bypass == Some(stack)
    }

    /// Release an actor's projection and deferred damage.
    pub fn released(&mut self, actor: u32) {
        self.deferred.remove(&actor);
        if self.projections.contains_key(&actor) {
            self.release_projection(actor);
        }
        self.live.insert(actor, false);
    }

    fn release_projection(&mut self, actor: u32) {
        if let Some(projection) = self.projections.get_mut(&actor) {
            projection.releasing = true;
        }
        if let Some(projection) = self.projections.remove(&actor) {
            self.slots.remove(&projection.slot);
        }
    }

    /// Clear every projection, collecting cleanup failures.
    pub fn clear(&mut self) -> Result<(), ForeignActorError> {
        self.deferred.clear();
        let actors: Vec<u32> = self.projections.keys().copied().collect();
        for actor in actors {
            self.release_projection(actor);
        }
        Ok(())
    }

    /// Save live projections.
    #[must_use]
    pub fn save_projections(&self) -> Vec<ProjectionSave> {
        self.projections
            .values()
            .filter(|projection| {
                !projection.releasing
                    && self.is_live(projection.actor)
                    && self.slots.get(&projection.slot) == Some(&projection.actor)
            })
            .map(|projection| ProjectionSave {
                slot: projection.slot,
                actor: SavedActorId {
                    slot: projection.actor,
                    generation: 1,
                },
            })
            .collect()
    }

    /// Begin a projection restore.
    pub fn begin_restore(&mut self, saved: &[ProjectionSave]) -> Result<(), ForeignActorError> {
        if self.restoring.is_some() {
            return Err(ForeignActorError::RestoreActive);
        }
        self.restoring = Some(saved.iter().map(|entry| (entry.slot, entry.actor)).collect());
        Ok(())
    }

    /// End a projection restore.
    pub fn end_restore(&mut self) {
        self.restoring = None;
    }

    /// Close every binding and projection.
    pub fn close(&mut self) {
        self.armor_bindings.clear();
        if self.closed {
            return;
        }
        self.closed = true;
        let _ = self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};

    fn test_actors() -> RereleaseForeignActors {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "foreign-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        let memory = SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory");
        RereleaseForeignActors::new(memory)
    }

    fn body() -> ProjectionBody {
        ProjectionBody {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
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
            health: 100,
            mass: 200,
            takedamage: true,
        }
    }

    #[test]
    fn projections_sync_bodies_and_save() {
        let mut actors = test_actors();
        let actor = actors.register_actor(body());
        let address = actors.address(actor).expect("address");
        let edict = edict_layout();
        let origin_offset = field_offset(&edict, "s.origin").expect("s.origin") as i64;
        let origin_address = actors.memory.offset(address, origin_offset).expect("origin address");
        let origin = actors.memory.read_f32x3(origin_address).expect("origin");
        assert_eq!(origin.x, 1.0);
        let slot = actors.projections.get(&actor).expect("projection").slot;
        assert_eq!(actors.lookup(slot), Some(Some(actor)));
        assert_eq!(actors.lookup(9999), None);
        actors.set_live(actor, false);
        assert_eq!(actors.lookup(slot), Some(None));
        actors.set_live(actor, true);
        let saved = actors.save_projections();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].slot, slot);
        actors.begin_restore(&saved).expect("begin");
        let found = actors.actor_at_slot(slot, &|_| Some(actor)).expect("restore");
        assert_eq!(found, Some(actor));
        actors.end_restore();
        actors.released(actor);
        assert_eq!(actors.lookup(slot), None);
    }

    #[test]
    fn staged_damage_intercepts_and_reacts() {
        let mut actors = test_actors();
        let target = actors.register_actor(body());
        let other = actors.register_actor(body());
        actors.address(target).expect("target");
        actors.address(other).expect("other");
        let target_slot = actors.projections.get(&target).expect("slot").slot;
        let other_slot = actors.projections.get(&other).expect("slot").slot;
        let token = actors
            .bind_armor_stage(
                target,
                ProtectionChannel::Regular,
                Box::new(|_, amount, _| amount * 0.5),
            )
            .expect("stage");
        assert!(actors
            .bind_armor_stage(target, ProtectionChannel::Regular, Box::new(|_, amount, _| amount))
            .is_err());
        actors
            .incoming_damage(&ForeignDamageArgs {
                target_slot,
                attacker_slot: other_slot,
                inflictor_slot: other_slot,
                direction: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                point: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                amount: 40.0,
                knockback: 10.0,
                damage_flags: 0,
                modem: [7, 0, 0],
            })
            .expect("incoming");
        assert_eq!(actors.shared_log.len(), 1);
        assert_eq!(actors.shared_log[0].cause.means_of_death, 7);
        let request = actors.shared_log[0].clone();
        actors.damage_frames.push(DamageFrame {
            request: request.clone(),
            entered: false,
            stack: None,
            observing: true,
        });
        assert!(actors.observing_damage(target));
        assert!(actors.take_frame_request(target).is_some());
        assert!(actors.take_frame_request(target).is_none());
        actors.with_inventory_publication(target, false, |actors| {
            assert!(!actors.current_publishes_inventory());
        });
        assert!(actors.current_publishes_inventory());
        actors.damage_frames.pop();
        let outcome = actors.damage_native(&request).expect("native");
        assert_eq!(outcome.reaction, Reaction::Pain);
        assert_eq!(outcome.applied_damage, 20);
        actors.unbind_armor_stage(token);
        actors.run_power_original(0x500, &|| 3.0);
        assert!(actors.power_bypassed(0x500));
        assert!(!actors.power_bypassed(0x501));
        actors.close();
        assert!(actors.address(target).is_err());
    }
}
