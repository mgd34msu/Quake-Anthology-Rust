//! Borrowed QC rows projecting canonical actors owned by other modules.
//!
//! Ported from donor `src/compat/qc/borrowed-actors.ts` (`QcBorrowedActors`).
//!
//! Local mirrors: [`BorrowedHost`] mirrors the `SessionActorRegistry` /
//! `BodyTable` / `GameplayAuthority` services consumed by the donor;
//! [`BorrowedSlotPool`] mirrors the `SourceActorSlots` storage/lifetime
//! pair; [`BorrowedCheckpoint`] mirrors the save rows. Slot references
//! reuse `super::entity_host`; entity words live in
//! [`FieldTable`](crate::fields::FieldTable).

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Bounds, Vec3};

use super::entity_host::{reference_for_slot, slot_for_reference};
use crate::error::GuestError;
use crate::fields::{FieldLayout, FieldTable, FieldValue};

/// Vector projection fields, in word order.
const VECTORS: &[&str] = &["origin", "angles", "velocity", "mins", "maxs", "absmin", "absmax", "size"];
/// Private writable fields (QC-owned scratch on borrowed rows).
const PRIVATE_FIELDS: &[&str] = &["chain", "invincible_sound"];
/// Scalar projection fields.
const SCALARS: &[&str] = &["health", "takedamage", "classname", "solid"];

/// Field layout for borrowed rows: full projection surface.
#[must_use]
pub fn borrowed_layout() -> FieldLayout {
    let mut layout = FieldLayout::new();
    for name in VECTORS {
        layout = layout.field(name, "vector");
    }
    for name in PRIVATE_FIELDS {
        layout = layout.field(name, "float");
    }
    layout = layout.field("health", "float").field("takedamage", "float").field("classname", "string").field(
        "solid",
        "float",
    );
    layout
}

/// Canonical body snapshot behind borrowed projection.
#[derive(Debug, Clone, PartialEq)]
pub struct BorrowedBody {
    /// World-space origin.
    pub origin: Vec3,
    /// World-space angles.
    pub angles: Vec3,
    /// Linear velocity.
    pub velocity: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
    /// Linked absolute bounds, if linked.
    pub linked: Option<Bounds>,
}

/// Canonical combat snapshot behind borrowed projection.
#[derive(Debug, Clone, PartialEq)]
pub struct BorrowedCombat {
    /// Current health.
    pub health: f32,
    /// Whether the actor can take damage.
    pub can_take_damage: bool,
}

/// Row reuse policy after a release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReusePolicy {
    /// Rows are reusable immediately.
    Immediate,
    /// Rows are reusable after a delay in seconds.
    AfterSeconds(f32),
}

/// Host services behind borrowed actors.
pub trait BorrowedHost {
    /// Whether an actor is live and owned.
    fn is_owned(&self, actor: &ActorId) -> bool;
    /// Resolve to the canonical owned handle.
    fn resolve_owned(&self, actor: &ActorId) -> Option<ActorId>;
    /// Source provider of an actor, if it owns a source row.
    fn source_provider(&self, actor: &ActorId) -> Option<String>;
    /// This pool's provider name.
    fn provider(&self) -> &str;
    /// Read the canonical body.
    fn body(&self, actor: &ActorId) -> Option<BorrowedBody>;
    /// Read the canonical combat state.
    fn combat(&self, actor: &ActorId) -> Option<BorrowedCombat>;
    /// Read the canonical classname.
    fn classname(&self, actor: &ActorId) -> String;
    /// Read the canonical solidity (0-4).
    fn solid(&self, actor: &ActorId) -> u8;
    /// Current time in seconds.
    fn now_seconds(&self) -> f32;
}

#[derive(Debug, Clone, PartialEq)]
enum SlotUse {
    Free { freed_at: f32 },
    Source,
    Borrowed(ActorId),
}

/// Slot pool backing borrowed rows.
#[derive(Debug, Clone)]
pub struct BorrowedSlotPool {
    slots: Vec<SlotUse>,
    first_dynamic: usize,
    capacity: usize,
    reuse: ReusePolicy,
}

impl BorrowedSlotPool {
    /// Build a pool. `count` rows start free; source rows are marked via
    /// [`BorrowedSlotPool::mark_source`].
    #[must_use]
    pub fn new(count: usize, first_dynamic: usize, capacity: usize, reuse: ReusePolicy) -> Self {
        Self {
            slots: vec![SlotUse::Free { freed_at: 0.0 }; count],
            first_dynamic,
            capacity,
            reuse,
        }
    }

    /// First dynamic slot.
    #[must_use]
    pub const fn first_dynamic(&self) -> usize {
        self.first_dynamic
    }

    /// Maximum row count.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current row count.
    #[must_use]
    pub fn count(&self) -> usize {
        self.slots.len()
    }

    /// Mark a row as owned by a current source actor.
    pub fn mark_source(&mut self, slot: usize) -> Result<(), GuestError> {
        let entry = self.entry_mut(slot)?;
        *entry = SlotUse::Source;
        Ok(())
    }

    /// Whether a row holds a current source actor.
    #[must_use]
    pub fn is_source(&self, slot: usize) -> bool {
        matches!(self.slots.get(slot), Some(SlotUse::Source))
    }

    fn entry_mut(&mut self, slot: usize) -> Result<&mut SlotUse, GuestError> {
        if slot >= self.slots.len() {
            return Err(GuestError::invalid(format!("borrowed slot {slot} is out of range")));
        }
        Ok(&mut self.slots[slot])
    }
}

/// Borrowed-row access kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    /// Field read: refreshes the projection first.
    Read,
    /// Field write: rejected outside private scratch fields.
    Write,
}

/// Saved borrowed-row mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowedCheckpoint {
    /// Borrowed actor.
    pub actor: SavedActorId,
    /// Borrowed row.
    pub slot: usize,
}

#[derive(Debug, Clone)]
struct BorrowedRow {
    actor: ActorId,
    slot: usize,
    classname: Option<String>,
}

/// Borrowed rows retain original entity pointers while the canonical actor
/// owns its state and lifetime.
pub struct QcBorrowedActors<H> {
    host: H,
    pool: BorrowedSlotPool,
    by_actor: HashMap<ActorId, BorrowedRow>,
    by_slot: HashMap<usize, ActorId>,
}

impl<H: BorrowedHost> QcBorrowedActors<H> {
    /// Build the borrowed-actor table over a slot pool.
    pub fn new(host: H, pool: BorrowedSlotPool) -> Self {
        Self { host, pool, by_actor: HashMap::new(), by_slot: HashMap::new() }
    }

    /// Borrow the host.
    #[must_use]
    pub fn host(&self) -> &H {
        &self.host
    }

    /// Borrow the slot pool.
    #[must_use]
    pub fn pool(&self) -> &BorrowedSlotPool {
        &self.pool
    }

    /// Borrow the slot pool mutably.
    pub fn pool_mut(&mut self) -> &mut BorrowedSlotPool {
        &mut self.pool
    }

    /// Actor borrowed into a slot, if any.
    pub fn actor(&self, slot: usize) -> Result<Option<ActorId>, GuestError> {
        let Some(actor) = self.by_slot.get(&slot) else {
            return Ok(None);
        };
        if !self.host.is_owned(actor) {
            return Err(GuestError::invalid("borrowed actor lost its canonical owner"));
        }
        Ok(Some(actor.clone()))
    }

    /// Borrow a canonical actor into a QC row; returns its reference.
    pub fn reference(&mut self, fields: &mut FieldTable, actor: &ActorId) -> Result<i32, GuestError> {
        let Some(owned) = self.host.resolve_owned(actor) else {
            return Err(GuestError::invalid("Cannot borrow a retired canonical actor"));
        };
        if self.host.source_provider(&owned).as_deref() == Some(self.host.provider()) {
            return Err(GuestError::invalid("Source actors must use their original QC row"));
        }
        if let Some(existing) = self.by_actor.get(&owned) {
            return Ok(reference_for_slot(existing.slot));
        }
        let now = self.host.now_seconds();
        let count = self.pool.slots.len();
        let mut selected = count;
        for slot in self.pool.first_dynamic..count {
            if let SlotUse::Free { freed_at } = self.pool.slots[slot] {
                let reusable = match self.pool.reuse {
                    ReusePolicy::Immediate => true,
                    ReusePolicy::AfterSeconds(delay) => now - freed_at >= delay,
                };
                if reusable {
                    selected = slot;
                    break;
                }
            }
        }
        if selected >= self.pool.capacity {
            return Err(GuestError::invalid("No free QC row for a borrowed actor"));
        }
        if self.pool.is_source(selected) || self.by_slot.contains_key(&selected) {
            return Err(GuestError::invalid("Borrowed QC row collides with a current source actor"));
        }
        if selected == count {
            self.pool.slots.push(SlotUse::Free { freed_at: now });
        }
        self.pool.slots[selected] = SlotUse::Borrowed(owned.clone());
        let row = BorrowedRow { actor: owned.clone(), slot: selected, classname: None };
        self.by_actor.insert(owned.clone(), row);
        self.by_slot.insert(selected, owned.clone());
        if let Err(error) = self.refresh(fields, &owned, None) {
            self.by_actor.remove(&owned);
            self.by_slot.remove(&selected);
            self.pool.slots[selected] = SlotUse::Free { freed_at: now };
            return Err(error);
        }
        Ok(reference_for_slot(selected))
    }

    /// Notify the table that a canonical actor was released.
    pub fn released(&mut self, actor: &ActorId) {
        if let Some(row) = self.by_actor.remove(actor) {
            self.by_slot.remove(&row.slot);
            let now = self.host.now_seconds();
            if let Ok(entry) = self.pool.entry_mut(row.slot) {
                *entry = SlotUse::Free { freed_at: now };
            }
        }
    }

    /// Guard a word access against a borrowed row. Reads refresh the
    /// projection; writes outside private scratch are rejected.
    pub fn access(
        &mut self,
        fields: &mut FieldTable,
        reference: i32,
        word: usize,
        words: usize,
        kind: AccessKind,
    ) -> Result<(), GuestError> {
        if words != 1 && words != 3 {
            return Err(GuestError::invalid(format!("borrowed access width {words} must be 1 or 3")));
        }
        let slot = slot_for_reference(reference, self.pool.slots.len())?;
        let Some(actor) = self.by_slot.get(&slot).cloned() else {
            return Ok(());
        };
        if !self.host.is_owned(&actor) {
            return Err(GuestError::invalid("borrowed actor lost its canonical owner"));
        }
        if kind == AccessKind::Write {
            if words == 1 && PRIVATE_FIELDS.contains(&field_at(word).unwrap_or("")) {
                return Ok(());
            }
            return Err(GuestError::invalid(
                "Original QC cannot mutate a borrowed actor outside its source owner",
            ));
        }
        let mut requested: Vec<&str> = Vec::new();
        for current in word..word + words {
            let Some(name) = field_at(current) else {
                return Err(GuestError::invalid(format!(
                    "Borrowed QC actor has no canonical field projection at {current}"
                )));
            };
            if !requested.contains(&name) {
                requested.push(name);
            }
        }
        self.refresh(fields, &actor, Some(&requested))
    }

    /// Checkpoint all borrowed mappings.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<BorrowedCheckpoint> {
        let mut rows: Vec<BorrowedCheckpoint> = self
            .by_actor
            .values()
            .map(|row| BorrowedCheckpoint { actor: SavedActorId::from(&row.actor), slot: row.slot })
            .collect();
        rows.sort_by_key(|row| row.slot);
        rows
    }

    /// Restore borrowed mappings. `resolve` maps saved actors to live ones;
    /// `source` reports current source occupancy per slot.
    pub fn restore(
        &mut self,
        saved: &[BorrowedCheckpoint],
        resolve: &dyn Fn(&SavedActorId) -> Option<ActorId>,
        source: &dyn Fn(usize) -> bool,
    ) -> Result<(), GuestError> {
        let mut by_actor = HashMap::new();
        let mut by_slot = HashMap::new();
        for entry in saved {
            let actor = resolve(&entry.actor);
            let valid = actor.as_ref().is_some_and(|actor| {
                entry.slot >= self.pool.first_dynamic
                    && entry.slot < self.pool.slots.len()
                    && !source(entry.slot)
                    && matches!(self.pool.slots[entry.slot], SlotUse::Free { .. })
                    && !by_actor.contains_key(actor)
                    && !by_slot.contains_key(&entry.slot)
                    && self.host.source_provider(actor).as_deref() != Some(self.host.provider())
            });
            if !valid {
                return Err(GuestError::invalid("invalid borrowed QC actor mapping"));
            }
            let actor = actor.expect("validated above");
            by_actor.insert(actor.clone(), BorrowedRow { actor: actor.clone(), slot: entry.slot, classname: None });
            by_slot.insert(entry.slot, actor);
        }
        for (slot, actor) in &by_slot {
            self.pool.slots[*slot] = SlotUse::Borrowed(actor.clone());
        }
        self.by_actor = by_actor;
        self.by_slot = by_slot;
        Ok(())
    }

    fn refresh(
        &mut self,
        fields: &mut FieldTable,
        actor: &ActorId,
        requested: Option<&[&str]>,
    ) -> Result<(), GuestError> {
        let all: Vec<&str> = VECTORS.iter().copied().chain(["health", "takedamage", "classname", "solid"]).collect();
        let requested = requested.unwrap_or(&all);
        if !fields.is_allocated(actor) {
            fields.allocate(actor, &borrowed_layout())?;
        }
        let wants_vector = requested.iter().any(|name| VECTORS.contains(name));
        if wants_vector {
            let Some(body) = self.host.body(actor) else {
                return Err(GuestError::invalid("Borrowed QC actor has no canonical body"));
            };
            let linked = if requested.contains(&"absmin") || requested.contains(&"absmax") {
                body.linked
            } else {
                None
            };
            for name in requested {
                let value = match *name {
                    "origin" => body.origin,
                    "angles" => body.angles,
                    "velocity" => body.velocity,
                    "mins" => body.bounds.min,
                    "maxs" => body.bounds.max,
                    "absmin" => linked.map(|bounds| bounds.min).unwrap_or(Vec3 {
                        x: body.origin.x + body.bounds.min.x,
                        y: body.origin.y + body.bounds.min.y,
                        z: body.origin.z + body.bounds.min.z,
                    }),
                    "absmax" => linked.map(|bounds| bounds.max).unwrap_or(Vec3 {
                        x: body.origin.x + body.bounds.max.x,
                        y: body.origin.y + body.bounds.max.y,
                        z: body.origin.z + body.bounds.max.z,
                    }),
                    "size" => Vec3 {
                        x: body.bounds.max.x - body.bounds.min.x,
                        y: body.bounds.max.y - body.bounds.min.y,
                        z: body.bounds.max.z - body.bounds.min.z,
                    },
                    _ => continue,
                };
                if ![value.x, value.y, value.z].iter().all(|component| component.is_finite()) {
                    return Err(GuestError::invalid("Borrowed actor geometry exceeds QC binary32 range"));
                }
                fields.set(actor, name, FieldValue::Vector(value))?;
            }
        }
        if requested.contains(&"health") || requested.contains(&"takedamage") {
            let combat = self.host.combat(actor);
            if requested.contains(&"health") {
                let health = combat.as_ref().map_or(0.0, |combat| combat.health);
                if !health.is_finite() {
                    return Err(GuestError::invalid("Borrowed actor health exceeds QC binary32 range"));
                }
                fields.set(actor, "health", FieldValue::Float(health))?;
            }
            if requested.contains(&"takedamage") {
                let damageable = combat.as_ref().is_some_and(|combat| combat.can_take_damage);
                fields.set(actor, "takedamage", FieldValue::Float(f32::from(damageable)))?;
            }
        }
        if requested.contains(&"classname") {
            let classname = self.host.classname(actor);
            let row = self.by_actor.get_mut(actor).ok_or_else(|| GuestError::invalid("borrowed row missing"))?;
            if row.classname.as_deref() != Some(classname.as_str()) {
                fields.set(actor, "classname", FieldValue::Text(classname.clone()))?;
                row.classname = Some(classname);
            }
        }
        if requested.contains(&"solid") {
            let solid = self.host.solid(actor);
            if solid > 4 {
                return Err(GuestError::invalid(format!("borrowed solidity {solid} out of range")));
            }
            fields.set(actor, "solid", FieldValue::Float(f32::from(solid)))?;
        }
        Ok(())
    }
}

/// Projection word layout: vectors occupy 3 words each in [`VECTORS`]
/// order, then one word per private/scalar field.
fn field_at(word: usize) -> Option<&'static str> {
    let mut offset = 0;
    for name in VECTORS {
        if word >= offset && word < offset + 3 {
            return Some(name);
        }
        offset += 3;
    }
    for name in PRIVATE_FIELDS.iter().chain(SCALARS.iter()) {
        if word == offset {
            return Some(name);
        }
        offset += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    struct FakeHost {
        live: Vec<ActorId>,
        source: Vec<ActorId>,
        now: f32,
    }

    impl BorrowedHost for FakeHost {
        fn is_owned(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<ActorId> {
            self.live.iter().find(|entry| *entry == actor).cloned()
        }

        fn source_provider(&self, actor: &ActorId) -> Option<String> {
            self.source.contains(actor).then(|| "quakec:edict".to_string())
        }

        fn provider(&self) -> &str {
            "quakec:edict"
        }

        fn body(&self, actor: &ActorId) -> Option<BorrowedBody> {
            self.is_owned(actor).then(|| BorrowedBody {
                origin: vec3(10.0, 20.0, 30.0),
                angles: vec3(0.0, 90.0, 0.0),
                velocity: vec3(1.0, 2.0, 3.0),
                bounds: Bounds { min: vec3(-8.0, -8.0, -8.0), max: vec3(8.0, 8.0, 8.0) },
                linked: None,
            })
        }

        fn combat(&self, actor: &ActorId) -> Option<BorrowedCombat> {
            self.is_owned(actor).then(|| BorrowedCombat { health: 75.0, can_take_damage: true })
        }

        fn classname(&self, _actor: &ActorId) -> String {
            "monster_ogre".to_string()
        }

        fn solid(&self, _actor: &ActorId) -> u8 {
            3
        }

        fn now_seconds(&self) -> f32 {
            self.now
        }
    }

    fn harness() -> (IdentityOwner, FieldTable, QcBorrowedActors<FakeHost>) {
        let owner = IdentityOwner::create("borrowed-actors").unwrap();
        let fields = FieldTable::new();
        let host = FakeHost { live: vec![owner.actor(1, 1), owner.actor(2, 1)], source: vec![owner.actor(2, 1)], now: 5.0 };
        let pool = BorrowedSlotPool::new(4, 1, 6, ReusePolicy::Immediate);
        (owner, fields, QcBorrowedActors::new(host, pool))
    }

    #[test]
    fn reference_borrows_and_projects_canonical_state() {
        let (owner, mut fields, mut borrowed) = harness();
        let actor = owner.actor(1, 1);
        let reference = borrowed.reference(&mut fields, &actor).unwrap();
        assert_eq!(reference, 1);
        assert_eq!(borrowed.actor(1).unwrap(), Some(actor.clone()));
        assert_eq!(
            fields.get(&actor, "origin").unwrap().as_vector("origin").unwrap(),
            vec3(10.0, 20.0, 30.0)
        );
        assert_eq!(
            fields.get(&actor, "size").unwrap().as_vector("size").unwrap(),
            vec3(16.0, 16.0, 16.0)
        );
        assert_eq!(fields.get(&actor, "health").unwrap(), &FieldValue::Float(75.0));
        assert_eq!(fields.get(&actor, "takedamage").unwrap(), &FieldValue::Float(1.0));
        assert_eq!(fields.get(&actor, "classname").unwrap().as_text("classname").unwrap(), "monster_ogre");
        // Second borrow reuses the row.
        assert_eq!(borrowed.reference(&mut fields, &actor).unwrap(), 1);
    }

    #[test]
    fn source_and_retired_actors_are_rejected() {
        let (owner, mut fields, mut borrowed) = harness();
        assert!(borrowed.reference(&mut fields, &owner.actor(2, 1)).is_err());
        assert!(borrowed.reference(&mut fields, &owner.actor(9, 1)).is_err());
        assert_eq!(borrowed.pool().first_dynamic(), 1);
        assert_eq!(borrowed.pool().capacity(), 6);
        assert_eq!(borrowed.pool().count(), 4);
    }

    #[test]
    fn writes_are_guarded_and_reads_refresh() {
        let (owner, mut fields, mut borrowed) = harness();
        let actor = owner.actor(1, 1);
        let reference = borrowed.reference(&mut fields, &actor).unwrap();
        // Private scratch writes are allowed.
        borrowed.access(&mut fields, reference, 24, 1, AccessKind::Write).unwrap();
        // Public writes are rejected.
        assert!(borrowed.access(&mut fields, reference, 0, 3, AccessKind::Write).is_err());
        // Unknown words are rejected on read.
        assert!(borrowed.access(&mut fields, reference, 99, 1, AccessKind::Read).is_err());
        // Aligned reads refresh the projection.
        borrowed.access(&mut fields, reference, 0, 3, AccessKind::Read).unwrap();
        assert_eq!(
            fields.get(&actor, "origin").unwrap().as_vector("origin").unwrap(),
            vec3(10.0, 20.0, 30.0)
        );
    }

    #[test]
    fn release_frees_rows_for_reuse() {
        let (owner, mut fields, mut borrowed) = harness();
        let actor = owner.actor(1, 1);
        borrowed.reference(&mut fields, &actor).unwrap();
        borrowed.released(&actor);
        assert_eq!(borrowed.actor(1).unwrap(), None);
        assert!(borrowed.checkpoint().is_empty());
        let again = borrowed.reference(&mut fields, &actor).unwrap();
        assert_eq!(again, 1);
    }

    #[test]
    fn checkpoint_restore_round_trip_validates_mappings() {
        let (owner, mut fields, mut borrowed) = harness();
        let actor = owner.actor(1, 1);
        borrowed.reference(&mut fields, &actor).unwrap();
        let saved = borrowed.checkpoint();
        assert_eq!(saved, vec![BorrowedCheckpoint { actor: SavedActorId::from(&actor), slot: 1 }]);
        let (_owner, _fields, mut restored) = harness();
        restored.pool_mut().mark_source(0).unwrap();
        let live = actor.clone();
        restored
            .restore(&saved, &|saved| (*saved == SavedActorId::from(&live)).then(|| live.clone()), &|slot| {
                slot == 0
            })
            .unwrap();
        assert_eq!(restored.actor(1).unwrap(), Some(actor));
        let bad = vec![BorrowedCheckpoint { actor: SavedActorId { slot: 9, generation: 9 }, slot: 1 }];
        assert!(restored.restore(&bad, &|_| None, &|_| false).is_err());
    }
}
