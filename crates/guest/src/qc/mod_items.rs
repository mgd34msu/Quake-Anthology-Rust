//! QuakeC source items over original inventory words.
//!
//! Ported from `src/compat/qc/mod-items.ts`.
//!
//! Local mirrors (owned by other workers' modules, noted here):
//! `QcItemMachine` mirrors the entity-word surface of `QcMachine` from
//! `src/compat/qc/machine.ts`; `QcItemServices` mirrors the actor/client/
//! inventory/weapon surface of `ModHostServices` from
//! `src/world/session/mods.ts`; inventory entries mirror
//! `src/contracts/gameplay.ts`; weapon presentation mirrors
//! `src/contracts/source-items.ts`; `validate_weapon_stage` mirrors
//! `qcDeclaredWeaponStage` from `src/content/q1/quakec/weapon-stage.ts`.
//! Declaration mirrors live in `super::mod_provider`.
//!
//! Adaptation: the donor diffs entity-store observations against their
//! before-image; this port caches last-read words per actor and diffs
//! `observe` against that cache, since the before-image lives with the
//! machine owner.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId};
use qa_core::time::SourceTime;

use super::mod_provider::{
    ContentId, ItemId, ModActorBinding, ModCallbackDeclaration, ModCallbackInput, ModItemCapacity, ModItemDefinition,
    ModItemKind, ModItemStorage, ModQcItems, ModRuntimeValue, ModSourceCall, ModWeaponSelect, ModWeaponSelector,
    QcModInputs, QcModMedia, QcProgramView, QcValueType, QcWeaponStageDeclaration,
};
use crate::error::GuestError;

/// Inventory entries plus their backing `(field, word)` pairs.
type InventoryWords = (Vec<QcInventoryEntry>, Vec<(String, f64)>);

/// Validate item declarations against a program.
pub fn validate_qc_items(program: &dyn QcProgramView, declaration: &ModCallbackDeclaration) -> Result<(), GuestError> {
    let items = match declaration.items.as_ref() {
        Some(items) => items,
        None => return Ok(()),
    };
    if declaration.clients.is_none() || items.definitions.is_empty() {
        return Err(GuestError::invalid(
            "QC source items require canonical clients and definitions",
        ));
    }
    let mut definitions = HashMap::new();
    for definition in &items.definitions {
        if definition.label.is_empty() || definitions.insert(definition.item.clone(), definition).is_some() {
            return Err(GuestError::invalid("QC item definitions are empty or duplicated"));
        }
    }
    let field = |name: &str, expected: QcValueType, input: bool| -> Result<(), GuestError> {
        let bound = declaration.actor_fields.iter().any(|field| {
            field.field == name
                && (matches!(field.binding, ModActorBinding::Private)
                    || (input && matches!(field.binding, ModActorBinding::ClientInput { .. })))
        });
        if program.field_type(name) != Some(expected) || !bound {
            return Err(GuestError::invalid(format!(
                "QC item storage {name} requires declared original storage"
            )));
        }
        Ok(())
    };
    let mut bound = HashSet::new();
    let mut words: HashMap<&str, &str> = HashMap::new();
    for storage in &items.storage {
        match storage {
            ModItemStorage::Counter {
                field: name,
                item,
                capacity,
            } => {
                field(name, QcValueType::Float, false)?;
                if words.insert(name.as_str(), "count").is_some() {
                    return Err(GuestError::invalid("QC item storage fields overlap"));
                }
                if !definitions.contains_key(item) || !bound.insert(item.clone()) {
                    return Err(GuestError::invalid(format!(
                        "QC item {item} lacks distinct declared storage"
                    )));
                }
                match capacity {
                    ModItemCapacity::Field { field: capacity } => {
                        field(capacity, QcValueType::Float, false)?;
                        if words.get(capacity.as_str()) == Some(&"count") {
                            return Err(GuestError::invalid("QC item capacity overlaps source storage"));
                        }
                        words.insert(capacity.as_str(), "capacity");
                    }
                    ModItemCapacity::Constant { value } => {
                        if !value.is_finite() || *value < 0.0 || f64::from(*value as f32) != *value {
                            return Err(GuestError::invalid("QC item capacity exceeds its source ABI"));
                        }
                    }
                }
            }
            ModItemStorage::Bits {
                field: name,
                private_mask,
                items: packed,
            } => {
                field(name, QcValueType::Float, false)?;
                if words.insert(name.as_str(), "count").is_some() {
                    return Err(GuestError::invalid("QC item storage fields overlap"));
                }
                if *private_mask < 0 || *private_mask > 0xff_ffff || packed.is_empty() {
                    return Err(GuestError::invalid(
                        "QC packed inventory requires an exact binary32 mask",
                    ));
                }
                let mut mask = *private_mask;
                for entry in packed {
                    if !definitions.contains_key(&entry.item) || !bound.insert(entry.item.clone()) {
                        return Err(GuestError::invalid(format!(
                            "QC item {} lacks distinct declared storage",
                            entry.item
                        )));
                    }
                    if entry.mask < 1
                        || entry.mask > 0x80_0000
                        || (entry.mask & (entry.mask - 1)) != 0
                        || (mask & entry.mask) != 0
                    {
                        return Err(GuestError::invalid(
                            "QC packed inventory masks overlap or exceed source precision",
                        ));
                    }
                    mask |= entry.mask;
                }
            }
        }
    }
    if bound.len() != definitions.len() {
        return Err(GuestError::invalid("QC item definition has no source storage"));
    }
    let weapons: Vec<_> = items
        .definitions
        .iter()
        .filter(|definition| matches!(definition.kind, ModItemKind::Weapon { .. }))
        .collect();
    if weapons.is_empty() != items.weapons.is_none() {
        return Err(GuestError::invalid(
            "QC weapon definitions require their original source consumer",
        ));
    }
    if let Some(consumer) = items.weapons.as_ref() {
        validate_weapon_stage(program, &consumer.stage)?;
        for name in ["think", "nextthink"] {
            let bound = declaration.actor_fields.iter().any(|field| {
                field.field == name && matches!(&field.binding, ModActorBinding::Think | ModActorBinding::Nextthink)
            });
            if !bound {
                return Err(GuestError::invalid(
                    "QC weapons require continuing source think ownership",
                ));
            }
        }
        field(&consumer.selected.field, QcValueType::Float, false)?;
        field(&consumer.select.field, QcValueType::Float, true)?;
        validate_weapon_selector(&consumer.selected.values, &weapons)?;
        validate_weapon_selector(&consumer.select.values, &weapons)?;
        field(&consumer.model.field, QcValueType::String, false)?;
        field(&consumer.model.frame, QcValueType::Float, false)?;
        for weapon in &weapons {
            if let ModItemKind::Weapon { ammo: Some(ammo) } = &weapon.kind {
                let declared = definitions.contains_key(ammo)
                    || declaration
                        .actor_fields
                        .iter()
                        .any(|field| matches!(&field.binding, ModActorBinding::Inventory { item } if item == ammo));
                if !declared {
                    return Err(GuestError::invalid("QC weapon has no declared ammo source"));
                }
            }
        }
    }
    Ok(())
}

/// Validate a weapon selector mapping.
fn validate_weapon_selector(
    values: &[super::mod_provider::ModWeaponSelectorValue],
    weapons: &[&ModItemDefinition],
) -> Result<(), GuestError> {
    if values.len() != weapons.len() {
        return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
    }
    let mut seen_items = HashSet::new();
    let mut seen_values = Vec::new();
    for value in values {
        if !value.value.is_finite() || f64::from(value.value as f32) != value.value || value.value == 0.0 {
            return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
        }
        seen_items.insert(value.item.clone());
        if seen_values.contains(&value.value) {
            return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
        }
        seen_values.push(value.value);
    }
    if seen_items.len() != weapons.len() || weapons.iter().any(|weapon| !seen_items.contains(&weapon.item)) {
        return Err(GuestError::invalid("QC weapon selectors differ from their definitions"));
    }
    Ok(())
}

/// Validate a declared weapon stage structurally.
pub fn validate_weapon_stage(program: &dyn QcProgramView, stage: &QcWeaponStageDeclaration) -> Result<(), GuestError> {
    if program.function_named(&stage.dispatcher).is_none() {
        return Err(GuestError::invalid("QC weapon stage requires its declared dispatcher"));
    }
    for continuation in &stage.continuations {
        if program.function_named(continuation).is_none() {
            return Err(GuestError::invalid(
                "QC weapon stage requires its declared continuations",
            ));
        }
    }
    for repeat in &stage.repeats {
        if program.function_named(&repeat.function).is_none()
            || repeat.entry >= repeat.exit
            || (repeat.result.1 != 0 && repeat.result.1 != 1)
        {
            return Err(GuestError::invalid("QC weapon stage requires valid repeat regions"));
        }
    }
    Ok(())
}

/// Inventory count policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QcCountPolicy {
    /// Plain stack.
    Stack,
    /// Binary32 source counter.
    SourceCounterBinary32,
}

/// Inventory entry projection.
#[derive(Debug, Clone, PartialEq)]
pub struct QcInventoryEntry {
    /// Item identifier.
    pub item: ItemId,
    /// Count.
    pub count: f64,
    /// Capacity.
    pub capacity: f64,
    /// Count policy.
    pub count_policy: QcCountPolicy,
}

/// Committed inventory store.
#[derive(Debug, Clone, PartialEq)]
pub struct QcItemStore {
    /// Previous entry.
    pub before: QcInventoryEntry,
    /// New entry.
    pub after: QcInventoryEntry,
}

/// Weapon model presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponModel {
    /// Requested resource path.
    pub requested_path: String,
    /// Model frame.
    pub frame: f64,
}

/// Weapon presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponPresentation {
    /// Source provider.
    pub provider: ProviderId,
    /// Content identifier.
    pub content: ContentId,
    /// Active weapon.
    pub active: Option<ItemId>,
    /// Model presentation.
    pub model: Option<QcWeaponModel>,
    /// Item definitions.
    pub items: Vec<ModItemDefinition>,
}

/// Pickup observation for store validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcPickupObservation {
    /// Pickup actor.
    pub actor: ActorId,
    /// Declared writes.
    pub writes: Vec<super::mod_provider::PickupWrite>,
}

/// Machine surface for item words.
pub trait QcItemMachine {
    /// Read a float by reference and field.
    fn float_for(&self, reference: i32, field: &str) -> Result<f32, GuestError>;
    /// Write a float by reference and field.
    fn set_float_for(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError>;
    /// Read an integer by reference and field.
    fn int_for(&self, reference: i32, field: &str) -> Result<i32, GuestError>;
    /// Read a managed string.
    fn strings_get(&self, index: i32) -> Result<String, GuestError>;
}

/// Host services for items.
pub trait QcItemServices {
    /// Current source time.
    fn now(&self) -> SourceTime;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an owned actor.
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Client handle for an actor.
    fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId>;
    /// Actor for a client handle.
    fn actor_for_client(&self, client: &ClientId) -> Option<ActorId>;
    /// Entity reference for an actor.
    fn reference_for_actor(&self, actor: &ActorId) -> Option<i32>;
    /// Actor for an entity reference.
    fn actor_for_reference(&self, reference: i32) -> Option<ActorId>;
    /// Whether the destination weapon slot service exists.
    fn has_weapon_service(&self) -> bool {
        false
    }
    /// Bind admitted items; returns a lease id.
    fn bind_items(
        &mut self,
        owner: &OwnedActor,
        provider: &ProviderId,
        definitions: &[ModItemDefinition],
        content: &ContentId,
    ) -> Result<u64, GuestError>;
    /// Whether a lease is current.
    fn lease_current(&self, lease: u64) -> bool;
    /// Report committed stores.
    fn notify_stored(&mut self, lease: u64, changes: &[QcItemStore]);
    /// Close a lease.
    fn close_lease(&mut self, lease: u64);
    /// Inventory count.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> f64;
    /// Bind the weapon slot; returns a binding id.
    fn weapon_bind(&mut self, _owner: &OwnedActor) -> Result<u64, GuestError> {
        Err(GuestError::invalid("QC source weapon service is absent"))
    }
    /// Release a weapon binding.
    fn weapon_unbind(&mut self, _binding: u64) {}
    /// Whether a provider holds the weapon selection.
    fn weapon_selected(&self, _actor: &ActorId, _provider: &ProviderId) -> bool {
        false
    }
}

/// Source dispatch for items.
pub trait QcItemDispatch {
    /// Invoke a source call.
    fn invoke(&mut self, call: &ModSourceCall, inputs: &QcModInputs) -> Result<f64, GuestError>;
}

#[derive(Clone)]
struct Entry {
    owner: OwnedActor,
    reference: i32,
    lease: u64,
    weapon: Option<u64>,
}

/// Source items over original words.
pub struct QcModItems<S, M, D> {
    definition: ModQcItems,
    provider: ProviderId,
    services: S,
    machine: M,
    dispatch: D,
    media: QcModMedia,
    entries: HashMap<ActorId, Entry>,
    last: HashMap<(ActorId, String), f64>,
}

impl<S: QcItemServices, M: QcItemMachine, D: QcItemDispatch> QcModItems<S, M, D> {
    /// Build over an item declaration.
    pub fn new(
        definition: ModQcItems,
        provider: ProviderId,
        services: S,
        machine: M,
        dispatch: D,
        media: QcModMedia,
    ) -> Result<Self, GuestError> {
        if definition.weapons.is_some() && !services.has_weapon_service() {
            return Err(GuestError::invalid(
                "QC weapons require the destination weapon slot service",
            ));
        }
        Ok(Self {
            definition,
            provider,
            services,
            machine,
            dispatch,
            media,
            entries: HashMap::new(),
            last: HashMap::new(),
        })
    }

    /// Admit a client actor.
    pub fn admit(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        if self.entries.contains_key(actor) {
            return Ok(());
        }
        let owned = self.services.resolve_owned(actor);
        let client = self.services.client_for_actor(actor);
        let (Some(owner), Some(client)) = (owned, client) else {
            return Err(GuestError::invalid("QC items require a live canonical client"));
        };
        if self.services.actor_for_client(&client).as_ref() != Some(actor) {
            return Err(GuestError::invalid("QC items require a live canonical client"));
        }
        let reference = self
            .services
            .reference_for_actor(actor)
            .ok_or_else(|| GuestError::invalid("QC items require a live canonical client"))?;
        let lease = self.services.bind_items(
            &owner,
            &self.provider,
            &self.definition.definitions,
            &self.media.content,
        )?;
        let mut entry = Entry {
            owner,
            reference,
            lease,
            weapon: None,
        };
        if self.definition.weapons.is_some() {
            match self.services.weapon_bind(&entry.owner) {
                Ok(binding) => entry.weapon = Some(binding),
                Err(error) => {
                    self.services.close_lease(lease);
                    return Err(error);
                }
            }
        }
        // Seed the observation cache.
        let (_entries, words) = self.read_all(reference)?;
        for (field, value) in words {
            self.last.insert((actor.clone(), field), value);
        }
        self.entries.insert(actor.clone(), entry);
        Ok(())
    }

    /// Whether an entry is current.
    fn current(&self, actor: &ActorId, entry: &Entry) -> bool {
        self.services.resolve_owned(actor).as_ref() == Some(&entry.owner)
            && self
                .entries
                .get(actor)
                .is_some_and(|stored| stored.lease == entry.lease)
            && self.services.lease_current(entry.lease)
            && self.services.reference_for_actor(actor) == Some(entry.reference)
    }

    /// Require a current entry.
    fn require(&self, actor: &ActorId) -> Result<&Entry, GuestError> {
        let entry = self
            .entries
            .get(actor)
            .ok_or_else(|| GuestError::invalid("QC source items are no longer current"))?;
        if !self.current(actor, entry) {
            return Err(GuestError::invalid("QC source items are no longer current"));
        }
        Ok(entry)
    }

    /// Read one float word.
    fn word(&self, reference: i32, field: &str) -> Result<f64, GuestError> {
        Ok(f64::from(self.machine.float_for(reference, field)?))
    }

    /// Read all entries plus their backing words.
    fn read_all(&self, reference: i32) -> Result<InventoryWords, GuestError> {
        let mut entries = Vec::new();
        let mut words = Vec::new();
        for storage in &self.definition.storage {
            match storage {
                ModItemStorage::Counter { field, item, capacity } => {
                    let count = self.word(reference, field)?;
                    words.push((field.clone(), count));
                    let limit = match capacity {
                        super::mod_provider::ModItemCapacity::Constant { value } => *value,
                        super::mod_provider::ModItemCapacity::Field { field } => {
                            let value = self.word(reference, field)?;
                            words.push((field.clone(), value));
                            value
                        }
                    };
                    entries.push(QcInventoryEntry {
                        item: item.clone(),
                        count,
                        capacity: limit,
                        count_policy: QcCountPolicy::SourceCounterBinary32,
                    });
                }
                ModItemStorage::Bits {
                    field,
                    private_mask,
                    items,
                } => {
                    let value = self.word(reference, field)?;
                    words.push((field.clone(), value));
                    let mask = items.iter().fold(*private_mask, |mask, entry| mask | entry.mask);
                    if value.fract() != 0.0 || value < 0.0 || value > 0xff_ffff as f64 || (value as i32 & !mask) != 0 {
                        return Err(GuestError::invalid("Original QC inventory contains undeclared bits"));
                    }
                    for entry in items {
                        entries.push(QcInventoryEntry {
                            item: entry.item.clone(),
                            count: if value as i32 & entry.mask == 0 { 0.0 } else { 1.0 },
                            capacity: 1.0,
                            count_policy: QcCountPolicy::Stack,
                        });
                    }
                }
            }
        }
        Ok((entries, words))
    }

    /// Read admitted entries.
    pub fn read(&self, actor: &ActorId) -> Result<Vec<QcInventoryEntry>, GuestError> {
        let entry = self.require(actor)?;
        Ok(self.read_all(entry.reference)?.0)
    }

    /// Read one entry.
    pub fn entry(&self, actor: &ActorId, item: &str) -> Result<Option<QcInventoryEntry>, GuestError> {
        let entry = self.require(actor)?;
        for storage in &self.definition.storage {
            match storage {
                ModItemStorage::Counter {
                    field,
                    item: stored,
                    capacity,
                } if stored == item => {
                    let count = self.word(entry.reference, field)?;
                    let limit = match capacity {
                        super::mod_provider::ModItemCapacity::Constant { value } => *value,
                        super::mod_provider::ModItemCapacity::Field { field } => self.word(entry.reference, field)?,
                    };
                    return Ok(Some(QcInventoryEntry {
                        item: item.to_string(),
                        count,
                        capacity: limit,
                        count_policy: QcCountPolicy::SourceCounterBinary32,
                    }));
                }
                ModItemStorage::Bits { field, items, .. } => {
                    if let Some(bit) = items.iter().find(|entry| entry.item == item) {
                        let count = self.word(entry.reference, field)?;
                        return Ok(Some(QcInventoryEntry {
                            item: item.to_string(),
                            count: if count as i32 & bit.mask == 0 { 0.0 } else { 1.0 },
                            capacity: 1.0,
                            count_policy: QcCountPolicy::Stack,
                        }));
                    }
                }
                _ => {}
            }
        }
        Ok(None)
    }

    /// Write one entry.
    pub fn write(&mut self, actor: &ActorId, next: &QcInventoryEntry) -> Result<(), GuestError> {
        let entry = self
            .entries
            .get(actor)
            .cloned()
            .ok_or_else(|| GuestError::invalid("QC source items are no longer current"))?;
        if !self.current(actor, &entry) {
            return Err(GuestError::invalid("QC source items are no longer current"));
        }
        for storage in &self.definition.storage {
            match storage {
                ModItemStorage::Counter { field, item, capacity } if item == &next.item => {
                    if !is_binary32(next.count) || !is_binary32(next.capacity) {
                        return Err(GuestError::invalid("QC inventory exceeds source scalar precision"));
                    }
                    if let super::mod_provider::ModItemCapacity::Constant { value } = capacity {
                        if next.capacity != *value {
                            return Err(GuestError::invalid("QC source capacity is immutable"));
                        }
                    }
                    let field = field.clone();
                    let capacity = capacity.clone();
                    self.machine.set_float_for(entry.reference, &field, next.count as f32)?;
                    if let super::mod_provider::ModItemCapacity::Field { field } = capacity {
                        self.machine
                            .set_float_for(entry.reference, &field, next.capacity as f32)?;
                    }
                    self.last.insert((actor.clone(), field), next.count);
                    return Ok(());
                }
                ModItemStorage::Bits { field, items, .. } => {
                    if let Some(bit) = items.iter().find(|entry| entry.item == next.item) {
                        if next.capacity != 1.0 || (next.count != 0.0 && next.count != 1.0) {
                            return Err(GuestError::invalid("QC weapon ownership must be one source bit"));
                        }
                        let mask = bit.mask;
                        let field = field.clone();
                        let bits = self.word(entry.reference, &field)? as i32;
                        let value = if next.count == 0.0 { bits & !mask } else { bits | mask };
                        self.machine.set_float_for(entry.reference, &field, value as f32)?;
                        self.last.insert((actor.clone(), field), value as f64);
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        Err(GuestError::invalid("QC item write has no source field"))
    }

    /// Whether an item has a mutable capacity field.
    #[must_use]
    pub fn mutable_capacity(&self, item: &str) -> bool {
        self.definition.storage.iter().any(|storage| {
            matches!(storage, ModItemStorage::Counter { item: stored, capacity, .. }
                if stored == item && matches!(capacity, super::mod_provider::ModItemCapacity::Field { .. }))
        })
    }

    /// Invoke an item action.
    pub fn invoke_action(
        &mut self,
        actor: &ActorId,
        item: &str,
        action: super::mod_provider::SourceItemAction,
    ) -> Result<(), GuestError> {
        let entry = self
            .entries
            .get(actor)
            .cloned()
            .ok_or_else(|| GuestError::invalid("Source item action is no longer admitted"))?;
        if !self.current(actor, &entry) {
            return Err(GuestError::invalid("Source item action is no longer admitted"));
        }
        let call = self
            .definition
            .definitions
            .iter()
            .find(|definition| definition.item == item)
            .and_then(|definition| definition.actions.as_ref())
            .and_then(|actions| match action {
                super::mod_provider::SourceItemAction::Use => actions.use_call.clone(),
                super::mod_provider::SourceItemAction::Drop => actions.drop_call.clone(),
            })
            .ok_or_else(|| GuestError::invalid("Source item action is no longer admitted"))?;
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(
            ModCallbackInput::Time,
            ModRuntimeValue::Float(self.services.now().as_seconds_f64()),
        );
        self.dispatch.invoke(&call, &inputs).map(|_| ())
    }

    /// Active weapon.
    pub fn active(&self, actor: &ActorId) -> Result<Option<ItemId>, GuestError> {
        let entry = self.require(actor)?;
        let weapons = self
            .definition
            .weapons
            .as_ref()
            .ok_or_else(|| GuestError::invalid("Missing qualified QC weapon consumer"))?;
        let value = self.word(entry.reference, &weapons.selected.field)?;
        let item = weapons
            .selected
            .values
            .iter()
            .find(|entry| entry.value == value)
            .map(|entry| entry.item.clone());
        if item.is_none() && value != 0.0 {
            return Err(GuestError::invalid("Original QC selected an undeclared source weapon"));
        }
        Ok(item)
    }

    /// Whether the selection accepts an item.
    pub fn weapon_accepts(&self, actor: &ActorId, item: &str) -> Result<bool, GuestError> {
        self.require(actor)?;
        let weapons = self
            .definition
            .weapons
            .as_ref()
            .ok_or_else(|| GuestError::invalid("Missing qualified QC weapon consumer"))?;
        Ok(weapons.selected.values.iter().any(|value| value.item == item)
            && self.services.inventory_count(actor, item) > 0.0)
    }

    /// Select a weapon.
    pub fn weapon_select(&mut self, actor: &ActorId, item: &str) -> Result<bool, GuestError> {
        if !self.weapon_accepts(actor, item)? {
            return Ok(false);
        }
        let weapons = self
            .definition
            .weapons
            .clone()
            .ok_or_else(|| GuestError::invalid("Missing qualified QC weapon consumer"))?;
        let value = weapons.select.values.iter().find(|value| value.item == item);
        let Some(value) = value else {
            return Ok(false);
        };
        let value = value.value;
        let entry = self.require(actor)?;
        let previous = Entry {
            owner: entry.owner.clone(),
            reference: entry.reference,
            lease: entry.lease,
            weapon: entry.weapon,
        };
        let reference = entry.reference;
        self.machine
            .set_float_for(reference, &weapons.select.field, value as f32)?;
        let mut inputs = QcModInputs::new();
        inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
        inputs.insert(
            ModCallbackInput::Time,
            ModRuntimeValue::Float(self.services.now().as_seconds_f64()),
        );
        self.dispatch.invoke(&weapons.select.call, &inputs)?;
        if !self.current(actor, &previous) {
            return Ok(false);
        }
        Ok(self.active(actor)? == Some(item.to_string()))
    }

    /// Holster the selection.
    pub fn weapon_holster(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        self.require(actor).map(|_| ())
    }

    /// Whether the selection is settled (no requested weapon differs from active).
    pub fn weapon_settled(&self, actor: &ActorId) -> Result<bool, GuestError> {
        let entry = self.require(actor)?;
        let weapons = self
            .definition
            .weapons
            .as_ref()
            .ok_or_else(|| GuestError::invalid("Missing qualified QC weapon consumer"))?;
        let requested = self.word(entry.reference, &weapons.select.field)?;
        let selected = self.word(entry.reference, &weapons.selected.field)?;
        Ok(requested == 0.0 || requested_selected_match(&weapons.select, &weapons.selected, requested, selected))
    }

    /// Resume the selection.
    pub fn weapon_resume(&mut self, actor: &ActorId, item: Option<&str>) -> Result<bool, GuestError> {
        self.require(actor)?;
        if let Some(item) = item {
            return self.weapon_select(actor, item);
        }
        let weapons = self
            .definition
            .weapons
            .clone()
            .ok_or_else(|| GuestError::invalid("Missing qualified QC weapon consumer"))?;
        for call in &weapons.resume {
            if self.require(actor).is_err() {
                return Err(GuestError::invalid("QC source weapon retired while resuming"));
            }
            let mut inputs = QcModInputs::new();
            inputs.insert(ModCallbackInput::Self_, ModRuntimeValue::Actor(Some(actor.clone())));
            inputs.insert(
                ModCallbackInput::Time,
                ModRuntimeValue::Float(self.services.now().as_seconds_f64()),
            );
            self.dispatch.invoke(call, &inputs)?;
        }
        Ok(self.require(actor).is_ok())
    }

    /// Weapon presentation.
    pub fn weapon_presentation(&self, actor: &ActorId) -> Result<QcWeaponPresentation, GuestError> {
        let entry = self.require(actor)?;
        let weapons = self
            .definition
            .weapons
            .as_ref()
            .ok_or_else(|| GuestError::invalid("QC source has no weapon presentation"))?;
        let index = self.machine.int_for(entry.reference, &weapons.model.field)?;
        let path = self.machine.strings_get(index)?;
        let asset = self.media.resources.get(&path);
        if !path.is_empty() && asset.is_none() {
            return Err(GuestError::invalid(format!(
                "Original QC weapon model was not prepared: {path}"
            )));
        }
        let frame = self.word(entry.reference, &weapons.model.frame)?;
        Ok(QcWeaponPresentation {
            provider: self.provider.clone(),
            content: self.media.content.clone(),
            active: self.active(actor)?,
            model: asset.map(|asset| QcWeaponModel {
                requested_path: asset.requested_path.clone(),
                frame,
            }),
            items: self.definition.definitions.clone(),
        })
    }

    /// Observe an entity store for one reference.
    pub fn observe(&mut self, reference: i32, pickup: Option<&QcPickupObservation>) -> Result<(), GuestError> {
        let actor = match self.services.actor_for_reference(reference) {
            Some(actor) => actor,
            None => return Ok(()),
        };
        let entry = match self.entries.get(&actor).cloned() {
            Some(entry) => entry,
            None => return Ok(()),
        };
        if !self.current(&actor, &entry) {
            return Err(GuestError::invalid("QC source item store belongs to a retired actor"));
        }
        let (after, words) = self.read_all(entry.reference)?;
        let mut before_entries = Vec::with_capacity(after.len());
        for storage in &self.definition.storage {
            match storage {
                ModItemStorage::Counter { field, item, capacity } => {
                    let count = self.last.get(&(actor.clone(), field.clone())).copied();
                    let limit = match capacity {
                        super::mod_provider::ModItemCapacity::Constant { value } => Some(*value),
                        super::mod_provider::ModItemCapacity::Field { field } => {
                            self.last.get(&(actor.clone(), field.clone())).copied()
                        }
                    };
                    if let (Some(count), Some(limit)) = (count, limit) {
                        before_entries.push(QcInventoryEntry {
                            item: item.clone(),
                            count,
                            capacity: limit,
                            count_policy: QcCountPolicy::SourceCounterBinary32,
                        });
                    }
                }
                ModItemStorage::Bits {
                    field,
                    private_mask,
                    items,
                } => {
                    if let Some(value) = self.last.get(&(actor.clone(), field.clone())).copied() {
                        let mask = items.iter().fold(*private_mask, |mask, entry| mask | entry.mask);
                        if value.fract() == 0.0
                            && value >= 0.0
                            && value <= 0xff_ffff as f64
                            && (value as i32 & !mask) == 0
                        {
                            for item in items {
                                before_entries.push(QcInventoryEntry {
                                    item: item.item.clone(),
                                    count: if value as i32 & item.mask == 0 { 0.0 } else { 1.0 },
                                    capacity: 1.0,
                                    count_policy: QcCountPolicy::Stack,
                                });
                            }
                        }
                    }
                }
            }
        }
        // Refresh the cache before reporting.
        for (field, value) in words {
            self.last.insert((actor.clone(), field), value);
        }
        let mut changes = Vec::new();
        for value in &after {
            let previous = before_entries.iter().find(|entry| entry.item == value.item);
            let Some(previous) = previous else {
                continue;
            };
            if previous.count == value.count && previous.capacity == value.capacity {
                continue;
            }
            if let Some(pickup) = pickup {
                if pickup.actor != actor
                    || !pickup.writes.iter().any(|write| {
                        matches!(write, super::mod_provider::PickupWrite::Inventory { item, fields }
                            if item == &value.item
                                && (previous.count == value.count || *fields != super::mod_provider::PickupFields::Capacity)
                                && (previous.capacity == value.capacity || *fields != super::mod_provider::PickupFields::Count))
                    })
                {
                    return Err(GuestError::invalid("Original pickup changed an undeclared source item"));
                }
            }
            changes.push(QcItemStore {
                before: previous.clone(),
                after: value.clone(),
            });
        }
        if !changes.is_empty() {
            self.services.notify_stored(entry.lease, &changes);
        }
        Ok(())
    }

    /// Release a client actor.
    pub fn release(&mut self, actor: &ActorId) -> Result<(), GuestError> {
        let entry = match self.entries.remove(actor) {
            Some(entry) => entry,
            None => return Ok(()),
        };
        self.last.retain(|(owner, _), _| owner != actor);
        if let Some(binding) = entry.weapon {
            self.services.weapon_unbind(binding);
        }
        self.services.close_lease(entry.lease);
        Ok(())
    }

    /// Release all client actors.
    pub fn close(&mut self) -> Result<(), GuestError> {
        let actors: Vec<ActorId> = self.entries.keys().cloned().collect();
        for actor in actors {
            self.release(&actor)?;
        }
        Ok(())
    }
}

/// Whether select/selected words agree on one weapon.
fn requested_selected_match(
    select: &ModWeaponSelect,
    selected: &ModWeaponSelector,
    requested: f64,
    current: f64,
) -> bool {
    let requested_item = select
        .values
        .iter()
        .find(|value| value.value == requested)
        .map(|value| &value.item);
    let current_item = selected
        .values
        .iter()
        .find(|value| value.value == current)
        .map(|value| &value.item);
    requested_item == current_item
}

/// Whether a scalar survives the binary32 source ABI.
fn is_binary32(value: f64) -> bool {
    value.is_finite() && f64::from(value as f32) == value
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use super::super::mod_provider::{
        ItemAdmission, ModClientDeclaration, ModItemActions, ModItemCapacity, ModItemIconDeclaration, ModPackedItem,
        ModWeaponModel, ModWeaponSelect, ModWeaponSelector, ModWeaponSelectorValue, ModWeapons, PickupFields,
        PickupWrite, QcApiKind, QcFunctionView, QcWeaponStageDeclaration, SourceItemAction,
    };

    struct FakeProgram {
        fields: HashMap<String, QcValueType>,
        functions: HashSet<String>,
    }

    impl QcProgramView for FakeProgram {
        fn digest(&self) -> &str {
            "abc"
        }

        fn api_kind(&self) -> QcApiKind {
            QcApiKind::Q1Netquake
        }

        fn field_type(&self, name: &str) -> Option<QcValueType> {
            self.fields.get(name).copied()
        }

        fn global_type(&self, _name: &str) -> Option<QcValueType> {
            None
        }

        fn function_named(&self, name: &str) -> Option<QcFunctionView> {
            self.functions.get(name).map(|name| QcFunctionView {
                index: 1,
                name: name.clone(),
                first_statement: 1,
                parameter_start: 0,
                parameter_sizes: Vec::new(),
                named_builtin: false,
            })
        }

        fn function_at(&self, _index: i32) -> Option<QcFunctionView> {
            None
        }

        fn functions(&self) -> Vec<QcFunctionView> {
            Vec::new()
        }
    }

    struct FakeServices {
        owner: IdentityOwner,
        live: Vec<ActorId>,
        clients: HashMap<ActorId, ClientId>,
        leases: HashMap<u64, bool>,
        stored: Vec<(u64, Vec<QcItemStore>)>,
        counts: HashMap<(ActorId, String), f64>,
        weapons: bool,
        next: u64,
    }

    impl QcItemServices for FakeServices {
        fn now(&self) -> SourceTime {
            SourceTime::Seconds(5.0)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            self.live
                .iter()
                .find(|live| *live == actor)
                .and_then(|actor| self.owner.owned_actor(actor, ProviderId::new("mod", "test")).ok())
        }

        fn client_for_actor(&self, actor: &ActorId) -> Option<ClientId> {
            self.clients.get(actor).cloned()
        }

        fn actor_for_client(&self, client: &ClientId) -> Option<ActorId> {
            self.clients
                .iter()
                .find(|(_, bound)| *bound == client)
                .map(|(actor, _)| actor.clone())
        }

        fn reference_for_actor(&self, actor: &ActorId) -> Option<i32> {
            Some(actor.slot() as i32)
        }

        fn actor_for_reference(&self, reference: i32) -> Option<ActorId> {
            self.live.iter().find(|actor| actor.slot() as i32 == reference).cloned()
        }

        fn has_weapon_service(&self) -> bool {
            self.weapons
        }

        fn bind_items(
            &mut self,
            _owner: &OwnedActor,
            _provider: &ProviderId,
            _definitions: &[ModItemDefinition],
            _content: &ContentId,
        ) -> Result<u64, GuestError> {
            let id = self.next;
            self.next += 1;
            self.leases.insert(id, true);
            Ok(id)
        }

        fn lease_current(&self, lease: u64) -> bool {
            self.leases.get(&lease).copied().unwrap_or(false)
        }

        fn notify_stored(&mut self, lease: u64, changes: &[QcItemStore]) {
            self.stored.push((lease, changes.to_vec()));
        }

        fn close_lease(&mut self, lease: u64) {
            self.leases.insert(lease, false);
        }

        fn inventory_count(&self, actor: &ActorId, item: &str) -> f64 {
            self.counts
                .get(&(actor.clone(), item.to_string()))
                .copied()
                .unwrap_or(0.0)
        }

        fn weapon_bind(&mut self, _owner: &OwnedActor) -> Result<u64, GuestError> {
            if self.weapons {
                Ok(77)
            } else {
                Err(GuestError::invalid("QC source weapon service is absent"))
            }
        }
    }

    #[derive(Default)]
    struct FakeMachine {
        floats: HashMap<(i32, String), f32>,
        ints: HashMap<(i32, String), i32>,
        strings: Vec<String>,
    }

    impl QcItemMachine for FakeMachine {
        fn float_for(&self, reference: i32, field: &str) -> Result<f32, GuestError> {
            Ok(self.floats.get(&(reference, field.to_string())).copied().unwrap_or(0.0))
        }

        fn set_float_for(&mut self, reference: i32, field: &str, value: f32) -> Result<(), GuestError> {
            self.floats.insert((reference, field.to_string()), value);
            Ok(())
        }

        fn int_for(&self, reference: i32, field: &str) -> Result<i32, GuestError> {
            Ok(self.ints.get(&(reference, field.to_string())).copied().unwrap_or(0))
        }

        fn strings_get(&self, index: i32) -> Result<String, GuestError> {
            self.strings
                .get(index as usize)
                .cloned()
                .ok_or_else(|| GuestError::invalid("String index is out of range"))
        }
    }

    #[derive(Default)]
    struct FakeDispatch {
        calls: Vec<String>,
    }

    impl QcItemDispatch for FakeDispatch {
        fn invoke(&mut self, call: &ModSourceCall, _inputs: &QcModInputs) -> Result<f64, GuestError> {
            self.calls.push(call.function.clone());
            Ok(0.0)
        }
    }

    fn call(name: &str) -> ModSourceCall {
        ModSourceCall {
            function: name.to_string(),
            arguments: Vec::new(),
            globals: Vec::new(),
        }
    }

    fn items_definition() -> ModQcItems {
        ModQcItems {
            definitions: vec![
                ModItemDefinition {
                    item: "q1:item_shells".to_string(),
                    label: "Shells".to_string(),
                    icon: None,
                    admission: ItemAdmission::Add,
                    kind: super::super::mod_provider::ModItemKind::Counter,
                    actions: None,
                },
                ModItemDefinition {
                    item: "q1:weapon_shotgun".to_string(),
                    label: "Shotgun".to_string(),
                    icon: Some(None),
                    admission: ItemAdmission::Add,
                    kind: super::super::mod_provider::ModItemKind::Weapon {
                        ammo: Some("q1:item_shells".to_string()),
                    },
                    actions: Some(ModItemActions {
                        use_call: Some(call("use_shotgun")),
                        drop_call: None,
                    }),
                },
            ],
            storage: vec![
                ModItemStorage::Counter {
                    field: "ammo_shells".to_string(),
                    item: "q1:item_shells".to_string(),
                    capacity: ModItemCapacity::Constant { value: 100.0 },
                },
                ModItemStorage::Bits {
                    field: "items".to_string(),
                    private_mask: 0,
                    items: vec![ModPackedItem {
                        item: "q1:weapon_shotgun".to_string(),
                        mask: 1,
                    }],
                },
            ],
            weapons: Some(ModWeapons {
                stage: QcWeaponStageDeclaration {
                    dispatcher: "W_Attack".to_string(),
                    continuations: Vec::new(),
                    repeats: Vec::new(),
                },
                selected: ModWeaponSelector {
                    field: "weapon".to_string(),
                    values: vec![ModWeaponSelectorValue {
                        value: 1.0,
                        item: "q1:weapon_shotgun".to_string(),
                    }],
                },
                select: ModWeaponSelect {
                    field: "weaponmodel".to_string(),
                    values: vec![ModWeaponSelectorValue {
                        value: 1.0,
                        item: "q1:weapon_shotgun".to_string(),
                    }],
                    call: call("select_weapon"),
                },
                resume: vec![call("resume_weapon")],
                model: ModWeaponModel {
                    field: "weaponmodel_str".to_string(),
                    frame: "weaponframe".to_string(),
                },
            }),
        }
    }

    fn declaration() -> ModCallbackDeclaration {
        ModCallbackDeclaration {
            actor_fields: [
                "ammo_shells",
                "items",
                "weapon",
                "weaponmodel",
                "weaponmodel_str",
                "weaponframe",
                "think",
                "nextthink",
            ]
            .into_iter()
            .map(|field| super::super::mod_provider::ModActorField {
                field: field.to_string(),
                binding: match field {
                    "think" => ModActorBinding::Think,
                    "nextthink" => ModActorBinding::Nextthink,
                    "weaponmodel" => super::super::mod_provider::ModActorBinding::ClientInput {
                        input: super::super::mod_provider::ModClientInput::Impulse,
                        update: super::super::mod_provider::ClientInputUpdate::Always,
                        scale: None,
                    },
                    _ => ModActorBinding::Private,
                },
            })
            .collect(),
            clients: Some(ModClientDeclaration {
                maximum: 4,
                input: vec![super::super::mod_provider::ModClientInputBinding {
                    scope: super::super::mod_provider::ModInputScope::ClientCommand,
                    phase: super::super::mod_provider::ModInputPhase::After,
                    calls: Vec::new(),
                    outputs: Vec::new(),
                }],
                ..ModClientDeclaration::default()
            }),
            items: Some(items_definition()),
            ..ModCallbackDeclaration::default()
        }
    }

    fn program() -> FakeProgram {
        let mut fields = HashMap::new();
        for name in [
            "ammo_shells",
            "items",
            "weapon",
            "weaponmodel",
            "weaponframe",
            "think",
            "nextthink",
        ] {
            fields.insert(name.to_string(), QcValueType::Float);
        }
        fields.insert("weaponmodel_str".to_string(), QcValueType::String);
        FakeProgram {
            fields,
            functions: ["W_Attack".to_string()].into_iter().collect(),
        }
    }

    fn media() -> QcModMedia {
        let mut media = QcModMedia {
            content: "test-content".to_string(),
            resources: HashMap::new(),
        };
        media.resources.insert(
            "progs/shotgun.mdl".to_string(),
            super::super::mod_provider::QcMediaResource {
                requested_path: "progs/shotgun.mdl".to_string(),
                model_bounds: None,
            },
        );
        media
    }

    fn fixture() -> QcModItems<FakeServices, FakeMachine, FakeDispatch> {
        QcModItems::new(
            items_definition(),
            ProviderId::new("mod", "test"),
            FakeServices {
                owner: IdentityOwner::create("items").unwrap(),
                live: Vec::new(),
                clients: HashMap::new(),
                leases: HashMap::new(),
                stored: Vec::new(),
                counts: HashMap::new(),
                weapons: true,
                next: 1,
            },
            FakeMachine {
                floats: HashMap::new(),
                ints: HashMap::new(),
                strings: vec![String::new()],
            },
            FakeDispatch::default(),
            media(),
        )
        .unwrap()
    }

    fn join(items: &mut QcModItems<FakeServices, FakeMachine, FakeDispatch>) -> ActorId {
        let actor = items.services.owner.actor(2, 1);
        let client = items.services.owner.client(2, 1);
        items.services.live.push(actor.clone());
        items.services.clients.insert(actor.clone(), client);
        actor
    }

    #[test]
    fn validation_accepts_and_rejects() {
        assert!(validate_qc_items(&program(), &declaration()).is_ok());
        let mut sparse = program();
        sparse.fields.remove("ammo_shells");
        assert!(validate_qc_items(&sparse, &declaration()).is_err());
        let mut duplicated = declaration();
        let extra = duplicated.items.as_ref().unwrap().definitions[0].clone();
        duplicated.items.as_mut().unwrap().definitions.push(extra);
        assert!(validate_qc_items(&program(), &duplicated).is_err());
    }

    #[test]
    fn admit_read_write_counters_and_bits() {
        let mut items = fixture();
        let actor = join(&mut items);
        items.admit(&actor).unwrap();
        items.machine.floats.insert((2, "ammo_shells".to_string()), 25.0);
        items.machine.floats.insert((2, "items".to_string()), 1.0);
        let entries = items.read(&actor).unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry.item == "q1:item_shells" && entry.count == 25.0 && entry.capacity == 100.0));
        assert!(entries
            .iter()
            .any(|entry| entry.item == "q1:weapon_shotgun" && entry.count == 1.0));
        items
            .write(
                &actor,
                &QcInventoryEntry {
                    item: "q1:item_shells".to_string(),
                    count: 30.0,
                    capacity: 100.0,
                    count_policy: QcCountPolicy::SourceCounterBinary32,
                },
            )
            .unwrap();
        assert_eq!(items.entry(&actor, "q1:item_shells").unwrap().unwrap().count, 30.0);
        items
            .write(
                &actor,
                &QcInventoryEntry {
                    item: "q1:weapon_shotgun".to_string(),
                    count: 0.0,
                    capacity: 1.0,
                    count_policy: QcCountPolicy::Stack,
                },
            )
            .unwrap();
        assert_eq!(items.entry(&actor, "q1:weapon_shotgun").unwrap().unwrap().count, 0.0);
        assert!(!items.mutable_capacity("q1:item_shells"));
        items
            .invoke_action(&actor, "q1:weapon_shotgun", SourceItemAction::Use)
            .unwrap();
        assert_eq!(items.dispatch.calls, vec!["use_shotgun".to_string()]);
        items.release(&actor).unwrap();
        assert!(items.read(&actor).is_err());
    }

    #[test]
    fn write_rejects_bad_counts() {
        let mut items = fixture();
        let actor = join(&mut items);
        items.admit(&actor).unwrap();
        assert!(items
            .write(
                &actor,
                &QcInventoryEntry {
                    item: "q1:item_shells".to_string(),
                    count: 1.0,
                    capacity: 50.0,
                    count_policy: QcCountPolicy::SourceCounterBinary32
                }
            )
            .is_err());
        assert!(items
            .write(
                &actor,
                &QcInventoryEntry {
                    item: "q1:weapon_shotgun".to_string(),
                    count: 2.0,
                    capacity: 1.0,
                    count_policy: QcCountPolicy::Stack
                }
            )
            .is_err());
        assert!(items
            .write(
                &actor,
                &QcInventoryEntry {
                    item: "q1:item_nails".to_string(),
                    count: 1.0,
                    capacity: 1.0,
                    count_policy: QcCountPolicy::Stack
                }
            )
            .is_err());
    }

    #[test]
    fn weapon_select_and_presentation() {
        let mut items = fixture();
        let actor = join(&mut items);
        items.admit(&actor).unwrap();
        items
            .services
            .counts
            .insert((actor.clone(), "q1:weapon_shotgun".to_string()), 1.0);
        items.machine.floats.insert((2, "weapon".to_string()), 1.0);
        assert_eq!(items.active(&actor).unwrap(), Some("q1:weapon_shotgun".to_string()));
        // Fake dispatch does not run source; mirror the select effect on the words.
        items.machine.floats.insert((2, "weaponmodel".to_string()), 1.0);
        assert!(items.weapon_accepts(&actor, "q1:weapon_shotgun").unwrap());
        assert!(items.weapon_settled(&actor).unwrap());
        items.machine.strings.push("progs/shotgun.mdl".to_string());
        items.machine.ints.insert((2, "weaponmodel_str".to_string()), 1);
        items.machine.floats.insert((2, "weaponframe".to_string()), 3.0);
        let presentation = items.weapon_presentation(&actor).unwrap();
        assert_eq!(presentation.active, Some("q1:weapon_shotgun".to_string()));
        assert_eq!(presentation.model.as_ref().unwrap().frame, 3.0);
        assert!(items.weapon_resume(&actor, None).unwrap());
        items.weapon_holster(&actor).unwrap();
    }

    #[test]
    fn weapon_select_writes_request_word() {
        let mut items = fixture();
        let actor = join(&mut items);
        items.admit(&actor).unwrap();
        items
            .services
            .counts
            .insert((actor.clone(), "q1:weapon_shotgun".to_string()), 1.0);
        items.machine.floats.insert((2, "weapon".to_string()), 1.0);
        assert!(items.weapon_select(&actor, "q1:weapon_shotgun").unwrap());
        assert_eq!(items.machine.floats.get(&(2, "weaponmodel".to_string())), Some(&1.0));
        assert!(items.dispatch.calls.contains(&"select_weapon".to_string()));
        assert!(!items.weapon_select(&actor, "q1:weapon_nailgun").unwrap());
    }

    #[test]
    fn observe_reports_changes_and_validates_pickups() {
        let mut items = fixture();
        let actor = join(&mut items);
        items.admit(&actor).unwrap();
        items.machine.floats.insert((2, "ammo_shells".to_string()), 10.0);
        items.observe(2, None).unwrap();
        assert_eq!(items.services.stored.len(), 1);
        let changes = &items.services.stored[0].1;
        assert_eq!(changes.len(), 1);
        assert_eq!((changes[0].before.count, changes[0].after.count), (0.0, 10.0));
        // No further changes: silent.
        items.observe(2, None).unwrap();
        assert_eq!(items.services.stored.len(), 1);
        // Pickup-declared write passes; undeclared write fails.
        items.machine.floats.insert((2, "ammo_shells".to_string()), 20.0);
        let pickup = QcPickupObservation {
            actor: actor.clone(),
            writes: vec![PickupWrite::Inventory {
                item: "q1:item_shells".to_string(),
                fields: PickupFields::Count,
            }],
        };
        items.observe(2, Some(&pickup)).unwrap();
        items.machine.floats.insert((2, "items".to_string()), 1.0);
        assert!(items.observe(2, Some(&pickup)).is_err());
        items.close().unwrap();
    }

    #[test]
    fn icon_and_action_helpers() {
        let icon = ModItemIconDeclaration::Image {
            path: "gfx/shotgun.lmp".to_string(),
        };
        assert!(matches!(icon, ModItemIconDeclaration::Image { .. }));
        let actions = ModItemActions {
            use_call: Some(call("use")),
            drop_call: Some(call("drop")),
        };
        assert_eq!(
            super::super::mod_provider::source_item_action_names(&actions),
            vec![SourceItemAction::Use, SourceItemAction::Drop]
        );
    }
}
