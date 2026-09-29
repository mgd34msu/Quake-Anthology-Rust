//! Q2 rerelease source-private edict and client bindings.
//!
//! Donor: `src/compat/q2/rerelease/source-state.ts` — bridges
//! `rerelease/g_local.h` source-private state into body, combat,
//! inventory and callback bindings over synthetic guests.

use std::collections::{HashMap, VecDeque};

use qa_core::math::Vec3;
use qa_guest::GuestError;
use qa_guest::core::contracts::{
    GuestAccess, GuestAddress, GuestAllocationOptions, GuestCallResult, GuestCallValue, GuestLayout,
};
use qa_guest::core::memory::SparseGuestMemory;
use qa_world::combat::{ArmorState, CombatState, ItemId};
use qa_world::inventory::{CountArithmetic, CountPolicy, InventoryEntry};
use thiserror::Error;

use super::layouts::{field_offset, private_edict_prefix_layout};

/// Source-state failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SourceStateError {
    /// Inventory index outside the selected `IT_TOTAL`.
    #[error("Rerelease inventory index is outside the selected IT_TOTAL")]
    BadItemIndex,
    /// Capacity index outside the selected `AMMO_MAX`.
    #[error("Rerelease capacity index is outside the selected AMMO_MAX")]
    BadAmmoIndex,
    /// Duplicate semantic or source inventory item.
    #[error("Duplicate semantic or source inventory item")]
    DuplicateItem,
    /// Item has no native inventory binding.
    #[error("Item has no native inventory binding")]
    UnboundItem,
    /// Native inventory count exceeds int32.
    #[error("Native inventory count exceeds int32")]
    CountOutOfRange,
    /// Native item has a fixed source capacity.
    #[error("Native item has a fixed source capacity")]
    FixedCapacity,
    /// Native ammo capacity exceeds int16.
    #[error("Native ammo capacity exceeds int16")]
    CapacityOutOfRange,
    /// Native power armor cell count exceeds int32.
    #[error("Native power armor cell count exceeds int32")]
    CellsOutOfRange,
    /// Rerelease edict does not contain the selected source-private prefix.
    #[error("Rerelease edict does not contain the selected source-private prefix")]
    ShortEdict,
    /// Rerelease private client profile belongs to another artifact.
    #[error("Rerelease private client profile belongs to another artifact")]
    ForeignClient,
    /// Native rerelease callback requires a classified Q2 damage cause.
    #[error("Native rerelease callback requires a classified Q2 damage cause")]
    UnclassifiedCause,
    /// Damage cause has no rerelease mod_t representation.
    #[error("Damage cause has no rerelease mod_t representation")]
    NoModRepresentation,
    /// Native rerelease touch requires its source trace.
    #[error("Native rerelease touch requires its source trace")]
    MissingTouchTrace,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Friendly-fire bit in canonical cause ids.
pub const CAUSE_FRIENDLY_FIRE: i32 = 0x800_0000;
/// Canonical grapple cause.
pub const CAUSE_GRAPPLE: i32 = 56;
/// Canonical telefrag-spawn cause.
pub const CAUSE_TELEFRAG_SPAWN: i32 = 57;
/// Canonical blue-blaster cause.
pub const CAUSE_BLUE_BLASTER: i32 = 58;

/// Classify a rerelease native cause (`missionpacks/damage.ts`).
#[must_use]
pub fn canonical_cause_from_native(id: u8, friendly_fire: bool) -> Option<i32> {
    let id = i32::from(id);
    if id > 58 {
        return None;
    }
    let canonical = if id < 22 {
        id
    } else if id == 22 {
        CAUSE_TELEFRAG_SPAWN
    } else if id <= 56 {
        id - 1
    } else if id == 57 {
        CAUSE_GRAPPLE
    } else {
        CAUSE_BLUE_BLASTER
    };
    Some(canonical + if friendly_fire { CAUSE_FRIENDLY_FIRE } else { 0 })
}

/// Convert a canonical cause back to a rerelease native ordinal.
#[must_use]
pub fn native_cause_from_canonical(canonical: i32) -> Option<(u8, bool)> {
    if canonical < 0 || canonical > CAUSE_FRIENDLY_FIRE + 58 {
        return None;
    }
    let friendly = (canonical & CAUSE_FRIENDLY_FIRE) != 0;
    let id = canonical & !CAUSE_FRIENDLY_FIRE;
    if id > 58 {
        return None;
    }
    let raw = if id < 22 {
        id
    } else if id <= 55 {
        id + 1
    } else if id == CAUSE_GRAPPLE {
        57
    } else if id == CAUSE_TELEFRAG_SPAWN {
        22
    } else {
        58
    };
    u8::try_from(raw).ok().map(|id| (id, friendly))
}

/// Three-byte by-value `mod_t` layout.
#[must_use]
pub fn mod_layout() -> GuestLayout {
    use qa_guest::core::contracts::{GuestFieldLayout, GuestStorage};
    GuestLayout::new(
        "q2-rerelease:mod_t",
        3,
        1,
        8,
        vec![
            GuestFieldLayout {
                name: "id".to_string(),
                byte_offset: 0,
                storage: GuestStorage::Uint8,
                count: 1,
            },
            GuestFieldLayout {
                name: "friendly_fire".to_string(),
                byte_offset: 1,
                storage: GuestStorage::Uint8,
                count: 1,
            },
            GuestFieldLayout {
                name: "no_point_loss".to_string(),
                byte_offset: 2,
                storage: GuestStorage::Uint8,
                count: 1,
            },
        ],
    )
}

/// Encode a `mod_t` aggregate value.
#[must_use]
pub fn encode_mod(id: u8, friendly_fire: bool, no_point_loss: bool) -> GuestCallValue {
    GuestCallValue::Aggregate {
        layout: mod_layout(),
        bytes: vec![id, u8::from(friendly_fire), u8::from(no_point_loss)],
    }
}

/// Q2 damage cause for source callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2Cause {
    /// Canonical means of death.
    pub means_of_death: i32,
    /// Native ordinal override, if already classified.
    pub native_id: Option<u8>,
    /// Friendly fire flag.
    pub friendly_fire: bool,
    /// No point-loss flag.
    pub no_point_loss: bool,
}

impl Q2Cause {
    /// Resolve to native `mod_t` bytes.
    pub fn native(&self) -> Result<(u8, bool, bool), SourceStateError> {
        match self.native_id {
            Some(id) => Ok((id, self.friendly_fire, self.no_point_loss)),
            None => native_cause_from_canonical(self.means_of_death)
                .map(|(id, friendly)| (id, friendly, self.no_point_loss))
                .ok_or(SourceStateError::NoModRepresentation),
        }
    }
}

/// Inventory capacity rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemCapacity {
    /// Ammunition capacity slot.
    Ammo {
        /// `max_ammo` source index.
        source_index: usize,
    },
    /// Fixed capacity.
    Fixed {
        /// Count.
        count: i32,
    },
}

/// Declared inventory item with its source slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseInventoryItem {
    /// Item identity.
    pub item: ItemId,
    /// Source inventory index.
    pub source_index: usize,
    /// Capacity rule.
    pub capacity: ItemCapacity,
}

/// Retail observed client field offsets (`client-profile.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetailClientFields {
    /// Record byte length.
    pub byte_length: usize,
    /// `pers.inventory` offset.
    pub inventory: usize,
    /// `pers.max_ammo` offset.
    pub max_ammo: usize,
    /// `invincible_time` offset.
    pub invincible_time: usize,
    /// `resp.ctf_team` offset.
    pub ctf_team: usize,
}

/// Retail client fields for the 84-slot artifact profile.
#[must_use]
pub const fn retail_client_fields() -> RetailClientFields {
    RetailClientFields {
        byte_length: 7344,
        inventory: 2688,
        max_ammo: 3024,
        invincible_time: 6672,
        ctf_team: 0x17dc,
    }
}

/// Source-private client record.
pub struct RereleaseSourceClient<'m> {
    memory: &'m mut SparseGuestMemory,
    address: GuestAddress,
    fields: RetailClientFields,
    inventory_count: usize,
    ammo_count: usize,
}

impl<'m> RereleaseSourceClient<'m> {
    /// Create over a client record address.
    pub fn new(
        memory: &'m mut SparseGuestMemory,
        address: GuestAddress,
        fields: RetailClientFields,
        inventory_count: usize,
        ammo_count: usize,
    ) -> Result<Self, SourceStateError> {
        memory.check(address, fields.byte_length, GuestAccess::Read)?;
        Ok(Self {
            memory,
            address,
            fields,
            inventory_count,
            ammo_count,
        })
    }

    fn item_address(&self, index: usize) -> Result<GuestAddress, SourceStateError> {
        if index >= self.inventory_count {
            return Err(SourceStateError::BadItemIndex);
        }
        Ok(self.memory.offset(
            self.address,
            (self.fields.inventory + index * 4) as i64,
        )?)
    }

    fn ammo_address(&self, index: usize) -> Result<GuestAddress, SourceStateError> {
        if index >= self.ammo_count {
            return Err(SourceStateError::BadAmmoIndex);
        }
        Ok(self.memory.offset(
            self.address,
            (self.fields.max_ammo + index * 2) as i64,
        )?)
    }

    /// Invulnerability deadline in milliseconds.
    pub fn invincible_until_milliseconds(&mut self) -> Result<i64, SourceStateError> {
        Ok(self.memory.read_i64(
            self.memory
                .offset(self.address, self.fields.invincible_time as i64)?,
        )?)
    }

    /// Capture-turret team field.
    pub fn ctf_team(&mut self) -> Result<i32, SourceStateError> {
        Ok(self.memory.read_i32(
            self.memory.offset(self.address, self.fields.ctf_team as i64)?,
        )?)
    }

    /// Read one inventory counter.
    pub fn read_counter(&mut self, index: usize) -> Result<i32, SourceStateError> {
        let address = self.item_address(index)?;
        Ok(self.memory.read_i32(address)?)
    }

    /// Write one inventory counter.
    pub fn write_counter(&mut self, index: usize, count: i32) -> Result<(), SourceStateError> {
        let address = self.item_address(index)?;
        self.memory.write_i32(address, count)?;
        Ok(())
    }

    /// Whether an item has a mutable ammunition capacity.
    #[must_use]
    pub fn mutable_capacity(items: &[RereleaseInventoryItem], item: &str) -> bool {
        items.iter().any(|value| {
            value.item == item && matches!(value.capacity, ItemCapacity::Ammo { .. })
        })
    }

    /// Read the declared roster as shared inventory entries.
    pub fn read_inventory(
        &mut self,
        items: &[RereleaseInventoryItem],
    ) -> Result<Vec<InventoryEntry>, SourceStateError> {
        Self::validate_roster(items, self.inventory_count, self.ammo_count)?;
        let mut entries = Vec::with_capacity(items.len());
        for item in items {
            let address = self.item_address(item.source_index)?;
            let count = self.memory.read_i32(address)?;
            let capacity = match item.capacity {
                ItemCapacity::Fixed { count } => f64::from(count),
                ItemCapacity::Ammo { source_index } => {
                    let at = self.ammo_address(source_index)?;
                    f64::from(self.memory.read_i16(at)?)
                }
            };
            entries.push(InventoryEntry {
                item: item.item.clone(),
                count: f64::from(count),
                capacity,
                count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
            });
        }
        Ok(entries)
    }

    /// Write one shared inventory entry back to source counters.
    pub fn write_inventory(
        &mut self,
        items: &[RereleaseInventoryItem],
        entry: &InventoryEntry,
    ) -> Result<(), SourceStateError> {
        Self::validate_roster(items, self.inventory_count, self.ammo_count)?;
        let Some(item) = items.iter().find(|value| value.item == entry.item) else {
            return Err(SourceStateError::UnboundItem);
        };
        if !entry.count.is_finite()
            || entry.count < f64::from(i32::MIN)
            || entry.count > f64::from(i32::MAX)
        {
            return Err(SourceStateError::CountOutOfRange);
        }
        match item.capacity {
            ItemCapacity::Fixed { count } => {
                if entry.capacity != f64::from(count) {
                    return Err(SourceStateError::FixedCapacity);
                }
            }
            ItemCapacity::Ammo { source_index } => {
                if !entry.capacity.is_finite()
                    || entry.capacity < 0.0
                    || entry.capacity > f64::from(i16::MAX)
                {
                    return Err(SourceStateError::CapacityOutOfRange);
                }
                let at = self.ammo_address(source_index)?;
                self.memory.write_i16(at, entry.capacity as i16)?;
            }
        }
        let address = self.item_address(item.source_index)?;
        self.memory.write_i32(address, entry.count as i32)?;
        Ok(())
    }

    fn validate_roster(
        items: &[RereleaseInventoryItem],
        inventory_count: usize,
        ammo_count: usize,
    ) -> Result<(), SourceStateError> {
        let mut ids = std::collections::HashSet::new();
        let mut slots = std::collections::HashSet::new();
        for item in items {
            if item.source_index >= inventory_count {
                return Err(SourceStateError::BadItemIndex);
            }
            if !ids.insert(item.item.clone()) || !slots.insert(item.source_index) {
                return Err(SourceStateError::DuplicateItem);
            }
            if let ItemCapacity::Ammo { source_index } = item.capacity {
                if source_index >= ammo_count {
                    return Err(SourceStateError::BadAmmoIndex);
                }
            }
        }
        Ok(())
    }

    /// Read power-armor cells (armor and ammunition bind the same counter).
    pub fn read_cells(&mut self, cells_index: usize) -> Result<i32, SourceStateError> {
        self.read_counter(cells_index)
    }

    /// Write power-armor cells.
    pub fn write_cells(
        &mut self,
        cells_index: usize,
        count: f64,
    ) -> Result<(), SourceStateError> {
        if !count.is_finite() || count < f64::from(i32::MIN) || count > f64::from(i32::MAX) {
            return Err(SourceStateError::CellsOutOfRange);
        }
        self.write_counter(cells_index, count as i32)
    }
}

/// Combat traits resolved through the selected source semantic policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CombatTraits {
    /// Invulnerable.
    pub invulnerable: bool,
    /// Team name, if any.
    pub team: Option<String>,
    /// Immune to knockback.
    pub no_knockback: bool,
}

/// Body state over source records with local actor ids.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceBodyState {
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
    /// Ground actor, if any.
    pub ground: Option<u32>,
}

/// Actor address resolution for body sync.
pub struct BodyAddresses<'a> {
    /// Resolve an actor id from a guest address.
    pub actor_of: &'a dyn Fn(GuestAddress) -> Option<u32>,
    /// Resolve a guest address from an actor id.
    pub address_of: &'a dyn Fn(u32) -> GuestAddress,
}

/// One recorded source callback invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct CallbackInvocation {
    /// Callback name.
    pub name: String,
    /// Arguments including the entity self pointer.
    pub arguments: Vec<GuestCallValue>,
}

/// Explicitly selected source layout; no inference from public mirrors.
pub struct RereleaseSourceEdict<'m> {
    memory: &'m mut SparseGuestMemory,
    address: GuestAddress,
    stride_bytes: usize,
    prefix: GuestLayout,
    /// Recorded invocations in order.
    pub invocations: Vec<CallbackInvocation>,
    scripted: HashMap<String, VecDeque<GuestCallResult>>,
}

impl<'m> RereleaseSourceEdict<'m> {
    /// Create over an edict record.
    pub fn new(
        memory: &'m mut SparseGuestMemory,
        address: GuestAddress,
        stride_bytes: usize,
    ) -> Result<Self, SourceStateError> {
        let prefix = private_edict_prefix_layout();
        if stride_bytes < prefix.byte_length {
            return Err(SourceStateError::ShortEdict);
        }
        Ok(Self {
            memory,
            address,
            stride_bytes,
            prefix,
            invocations: Vec::new(),
            scripted: HashMap::new(),
        })
    }

    /// Queue a scripted callback result.
    pub fn script(&mut self, name: &str, result: GuestCallResult) {
        self.scripted
            .entry(name.to_string())
            .or_default()
            .push_back(result);
    }

    /// Record address.
    #[must_use]
    pub const fn address(&self) -> GuestAddress {
        self.address
    }

    /// Record stride.
    #[must_use]
    pub const fn stride_bytes(&self) -> usize {
        self.stride_bytes
    }

    /// Address of a named source field.
    pub fn at(&self, name: &str) -> Result<GuestAddress, SourceStateError> {
        let offset = field_offset(&self.prefix, name)
            .map(|offset| offset as i64)
            .map_err(|_| SourceStateError::ShortEdict)?;
        Ok(self.memory.offset(self.address, offset)?)
    }

    /// Spawn generation.
    pub fn generation(&mut self) -> Result<i32, SourceStateError> {
        let at = self.at("spawn_count")?;
        Ok(self.memory.read_i32(at)?)
    }

    /// Health.
    pub fn health(&mut self) -> Result<i32, SourceStateError> {
        let at = self.at("health")?;
        Ok(self.memory.read_i32(at)?)
    }

    /// Write health.
    pub fn set_health(&mut self, value: i32) -> Result<(), SourceStateError> {
        let at = self.at("health")?;
        self.memory.write_i32(at, value)?;
        Ok(())
    }

    /// Whether the record takes damage.
    pub fn damageable(&mut self) -> Result<bool, SourceStateError> {
        let at = self.at("takedamage")?;
        Ok(self.memory.read_u8(at)? != 0)
    }

    /// Client record address, if any.
    pub fn client(&mut self) -> Result<Option<GuestAddress>, SourceStateError> {
        let at = self.at("shared.client")?;
        Ok(self.memory.read_pointer(at)?)
    }

    /// Assemble shared combat state from source stores.
    pub fn combat_state(
        &mut self,
        armor: ArmorState,
        traits: CombatTraits,
    ) -> Result<CombatState, SourceStateError> {
        let health = self.health()?;
        let mass = self.memory.read_i32(self.at("mass")?)?;
        Ok(CombatState {
            health: f64::from(health),
            armor,
            mass: f64::from(mass),
            can_take_damage: self.damageable()?,
            invulnerable: traits.invulnerable,
            no_knockback: traits.no_knockback,
            team: traits.team,
        })
    }

    /// Read a vector field.
    pub fn vector(&mut self, name: &str) -> Result<Vec3, SourceStateError> {
        let at = self.at(name)?;
        Ok(self.memory.read_f32x3(at)?)
    }

    /// Write a vector field.
    pub fn write_vector(&mut self, name: &str, value: Vec3) -> Result<(), SourceStateError> {
        let at = self.at(name)?;
        self.memory.write_f32(at, value.x)?;
        self.memory
            .write_f32(self.memory.offset(at, 4)?, value.y)?;
        self.memory
            .write_f32(self.memory.offset(at, 8)?, value.z)?;
        Ok(())
    }

    /// Read body state.
    pub fn read_body(
        &mut self,
        addresses: &BodyAddresses,
    ) -> Result<SourceBodyState, SourceStateError> {
        let ground = self.memory.read_pointer(self.at("groundentity")?)?;
        Ok(SourceBodyState {
            origin: self.vector("shared.s.origin")?,
            angles: self.vector("shared.s.angles")?,
            velocity: self.vector("velocity")?,
            min: self.vector("shared.mins")?,
            max: self.vector("shared.maxs")?,
            ground: ground.and_then(addresses.actor_of),
        })
    }

    /// Write body state.
    pub fn write_body(
        &mut self,
        addresses: &BodyAddresses,
        state: &SourceBodyState,
    ) -> Result<(), SourceStateError> {
        self.write_vector("shared.s.origin", state.origin)?;
        self.write_vector("shared.s.angles", state.angles)?;
        self.write_vector("velocity", state.velocity)?;
        self.write_vector("shared.mins", state.min)?;
        self.write_vector("shared.maxs", state.max)?;
        let ground = state.ground.map(addresses.address_of);
        let at = self.at("groundentity")?;
        self.memory.write_pointer(at, ground)?;
        Ok(())
    }

    fn present(&mut self, name: &str) -> Result<bool, SourceStateError> {
        let field = format!("{name}.value");
        let at = self.at(&field)?;
        Ok(self.memory.read_pointer(at)?.is_some())
    }

    /// Invoke a source callback by name. Values are fetched per call since
    /// native assignments can replace callbacks at runtime.
    pub fn call(
        &mut self,
        name: &str,
        arguments: &[GuestCallValue],
    ) -> Result<GuestCallResult, SourceStateError> {
        let field = format!("{name}.value");
        let at = self.at(&field)?;
        if self.memory.read_pointer(at)?.is_none() {
            return Ok(GuestCallResult::Void);
        }
        let mut full = vec![GuestCallValue::Pointer(Some(self.address))];
        full.extend_from_slice(arguments);
        self.invocations.push(CallbackInvocation {
            name: name.to_string(),
            arguments: full,
        });
        if let Some(queue) = self.scripted.get_mut(name) {
            if let Some(result) = queue.pop_front() {
                return Ok(result);
            }
        }
        Ok(GuestCallResult::Void)
    }

    /// Think callback.
    pub fn think(&mut self) -> Result<GuestCallResult, SourceStateError> {
        self.call("think", &[])
    }

    /// Use callback; absent callbacks are a no-op.
    pub fn use_callback(
        &mut self,
        other: Option<u32>,
        activator: Option<u32>,
        address_of: &dyn Fn(u32) -> GuestAddress,
    ) -> Result<GuestCallResult, SourceStateError> {
        if !self.present("use")? {
            return Ok(GuestCallResult::Void);
        }
        self.call(
            "use",
            &[
                GuestCallValue::Pointer(other.map(address_of)),
                GuestCallValue::Pointer(activator.map(address_of)),
            ],
        )
    }

    /// Touch callback with an encoded source trace.
    pub fn touch(
        &mut self,
        other: GuestAddress,
        trace_bytes: Option<&[u8]>,
        inverted: bool,
    ) -> Result<GuestCallResult, SourceStateError> {
        if !self.present("touch")? {
            return Ok(GuestCallResult::Void);
        }
        let Some(trace) = trace_bytes else {
            return Err(SourceStateError::MissingTouchTrace);
        };
        let address = self
            .memory
            .allocate(&GuestAllocationOptions::bytes(trace.len()))?;
        self.memory.write(address, trace)?;
        let result = self.call(
            "touch",
            &[
                GuestCallValue::Pointer(Some(other)),
                GuestCallValue::Pointer(Some(address)),
                GuestCallValue::Uint32(u32::from(inverted)),
            ],
        );
        self.memory.unmap(address, trace.len())?;
        result
    }

    /// Pain callback with a classified cause.
    pub fn pain(
        &mut self,
        attacker: Option<GuestAddress>,
        kick: f32,
        damage: i32,
        cause: &Q2Cause,
    ) -> Result<GuestCallResult, SourceStateError> {
        if !self.present("pain")? {
            return Ok(GuestCallResult::Void);
        }
        let (id, friendly, no_point_loss) = cause.native()?;
        self.call(
            "pain",
            &[
                GuestCallValue::Pointer(attacker),
                GuestCallValue::Float32(kick),
                GuestCallValue::Int32(damage),
                encode_mod(id, friendly, no_point_loss),
            ],
        )
    }

    /// Death callback with a classified cause.
    pub fn die(
        &mut self,
        inflictor: Option<GuestAddress>,
        attacker: Option<GuestAddress>,
        damage: i32,
        point: Vec3,
        cause: &Q2Cause,
    ) -> Result<GuestCallResult, SourceStateError> {
        if !self.present("die")? {
            return Ok(GuestCallResult::Void);
        }
        let (id, friendly, no_point_loss) = cause.native()?;
        let address = self.memory.allocate(&GuestAllocationOptions::bytes(12))?;
        self.memory.write_f32(address, point.x)?;
        self.memory
            .write_f32(self.memory.offset(address, 4)?, point.y)?;
        self.memory
            .write_f32(self.memory.offset(address, 8)?, point.z)?;
        let result = self.call(
            "die",
            &[
                GuestCallValue::Pointer(inflictor),
                GuestCallValue::Pointer(attacker),
                GuestCallValue::Int32(damage),
                GuestCallValue::Pointer(Some(address)),
                encode_mod(id, friendly, no_point_loss),
            ],
        );
        self.memory.unmap(address, 12)?;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};
    use qa_world::combat::{PoweredProtection, RegularArmor};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "source-state-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn roster() -> Vec<RereleaseInventoryItem> {
        vec![
            RereleaseInventoryItem {
                item: "q2:ammo_cells".to_string(),
                source_index: 30,
                capacity: ItemCapacity::Ammo { source_index: 4 },
            },
            RereleaseInventoryItem {
                item: "q2:item_armor_body".to_string(),
                source_index: 3,
                capacity: ItemCapacity::Fixed { count: 200 },
            },
        ]
    }

    #[test]
    fn client_counters_capacities_and_cells() {
        let mut memory = test_memory();
        let fields = retail_client_fields();
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(fields.byte_length))
            .expect("alloc");
        let mut client =
            RereleaseSourceClient::new(&mut memory, address, fields, 84, 12).expect("client");
        assert!(!RereleaseSourceClient::mutable_capacity(&roster(), "q2:item_armor_body"));
        assert!(RereleaseSourceClient::mutable_capacity(&roster(), "q2:ammo_cells"));
        client.write_counter(30, 17).expect("cells");
        assert_eq!(client.read_cells(30).expect("read"), 17);
        let entries = client.read_inventory(&roster()).expect("read");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].count, 17.0);
        let mut updated = entries[0].clone();
        updated.count = 25.0;
        updated.capacity = 100.0;
        client
            .write_inventory(&roster(), &updated)
            .expect("write");
        assert_eq!(client.read_counter(30).expect("read"), 25);
        let mut fixed = entries[1].clone();
        fixed.capacity = 1.0;
        assert_eq!(
            client.write_inventory(&roster(), &fixed).unwrap_err(),
            SourceStateError::FixedCapacity
        );
        assert_eq!(
            client.read_counter(84).unwrap_err(),
            SourceStateError::BadItemIndex
        );
        assert_eq!(
            client.invincible_until_milliseconds().expect("time"),
            0
        );
    }

    #[test]
    fn edict_health_body_and_callbacks() {
        let mut memory = test_memory();
        let prefix = private_edict_prefix_layout();
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(prefix.byte_length))
            .expect("alloc");
        let space = memory.address_space();
        let actor_of = |address: GuestAddress| (address.offset == 0x500).then_some(9u32);
        let address_of = |actor: u32| {
            GuestAddress::new(space, if actor == 9 { 0x500 } else { 0x600 })
        };
        let addresses = BodyAddresses {
            actor_of: &actor_of,
            address_of: &address_of,
        };
        let mut edict =
            RereleaseSourceEdict::new(&mut memory, address, prefix.byte_length).expect("edict");
        edict.set_health(120).expect("health");
        assert_eq!(edict.health().expect("read"), 120);
        assert_eq!(edict.generation().expect("gen"), 0);
        assert!(!edict.damageable().expect("takedamage"));
        let state = edict
            .combat_state(
                ArmorState {
                    regular: RegularArmor::None,
                    powered: PoweredProtection::None,
                },
                CombatTraits {
                    invulnerable: true,
                    team: Some("q2:1".to_string()),
                    no_knockback: false,
                },
            )
            .expect("combat");
        assert_eq!(state.health, 120.0);
        assert!(state.invulnerable);
        let body = SourceBodyState {
            origin: Vec3 {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            angles: Vec3 {
                x: 0.0,
                y: 90.0,
                z: 0.0,
            },
            velocity: Vec3 {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            min: Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            max: Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
            ground: Some(9),
        };
        edict.write_body(&addresses, &body).expect("write");
        assert_eq!(edict.read_body(&addresses).expect("read"), body);
        assert_eq!(
            edict.use_callback(None, None, &address_of).expect("absent"),
            GuestCallResult::Void
        );
        let pain_value = edict.at("pain.value").expect("pain");
        edict
            .memory
            .write_pointer(pain_value, Some(address_of(7)))
            .expect("present");
        let cause = Q2Cause {
            means_of_death: 1,
            native_id: None,
            friendly_fire: false,
            no_point_loss: false,
        };
        edict
            .pain(Some(address_of(7)), 2.0, 10, &cause)
            .expect("pain");
        assert_eq!(edict.invocations.len(), 1);
        assert_eq!(edict.invocations[0].name, "pain");
        assert_eq!(canonical_cause_from_native(22, false), Some(57));
        assert_eq!(native_cause_from_canonical(57), Some((22, false)));
        assert_eq!(canonical_cause_from_native(59, false), None);
        let die_value = edict.at("die.value").expect("die");
        edict
            .memory
            .write_pointer(die_value, Some(address_of(7)))
            .expect("present");
        let bad = Q2Cause {
            means_of_death: -5,
            native_id: None,
            friendly_fire: false,
            no_point_loss: false,
        };
        assert_eq!(
            edict.die(None, None, 1, body.origin, &bad).unwrap_err(),
            SourceStateError::NoModRepresentation
        );
        edict
            .die(None, None, 1, body.origin, &cause)
            .expect("die");
    }
}
