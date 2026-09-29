//! QVM inventory bindings over public player words and private storage.
//!
//! Provenance: `src/compat/qvm/game-inventory.ts`.
//!
//! Storage words reuse [`super::item_storage`]; the binding shape mirrors the
//! used surface of `src/world/gameplay/inventory.ts`
//! (`InventoryStateBinding`: `read`, `write`, and the private-only `entry`
//! and `mutableCapacity`).
//!
//! [`QvmGameData`]: super::game_data::QvmGameData

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_world::combat::ItemId;
use qa_world::inventory::{CountArithmetic, CountPolicy, InventoryEntry};

use super::game_data::{
    AbiProfile, ModuleIdentity, QvmGameData, QvmImage, QvmMemoryWindow, QvmModule, QvmSharedMemory,
};
use super::item_storage::{
    read_qvm_item_storage, write_qvm_item_storage, QvmItemCapacity, QvmItemField, QvmItemStorage, QvmItemStorageAccess,
};
use crate::error::GuestError;

/// Public weapon slot: word bit plus item and ammo ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmInventoryWeapon {
    /// Weapon slot (1 through 15).
    pub weapon: i32,
    /// Weapon item.
    pub item: ItemId,
    /// Ammo item, if any.
    pub ammo: Option<ItemId>,
}

/// Weapon definitions: fixed or refreshed per view.
#[derive(Clone)]
pub enum QvmInventoryWeapons {
    /// Fixed definitions, validated eagerly.
    Fixed(Vec<QvmInventoryWeapon>),
    /// Refreshed definitions, validated per view.
    Dynamic(Rc<dyn Fn() -> Vec<QvmInventoryWeapon>>),
}

/// Ammo capacity context.
#[derive(Debug, Clone)]
pub struct QvmInventoryCapacityContext {
    /// Source module.
    pub module: QvmModule,
    /// Client record address.
    pub client: usize,
    /// Entity record address.
    pub entity: usize,
    /// Client slot.
    pub client_number: usize,
}

/// Ammo capacity callback.
pub type QvmAmmoCapacity = Rc<dyn Fn(&QvmSharedMemory, i32, &QvmInventoryCapacityContext) -> Result<i32, GuestError>>;

/// Public inventory profile over player-state words.
#[derive(Clone)]
pub struct QvmPublicInventoryProfile {
    /// Owning module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Weapon-bits offset.
    pub weapons_offset: usize,
    /// Ammo-counters offset.
    pub ammo_offset: usize,
    /// Ammo capacity callback.
    pub capacity: QvmAmmoCapacity,
}

/// Private inventory profile over declared storage.
#[derive(Debug, Clone)]
pub struct QvmPrivateInventoryProfile {
    /// Owning module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Entity stride in bytes.
    pub entity_stride: usize,
    /// Client stride in bytes.
    pub client_stride: usize,
    /// Source image.
    pub image: QvmImage,
    /// Declared storage.
    pub storage: Vec<QvmItemStorage>,
}

/// Inventory profile: public words or private storage.
#[derive(Clone)]
pub enum QvmInventoryProfile {
    /// Public player-state words.
    Public(QvmPublicInventoryProfile),
    /// Private declared storage.
    Private(QvmPrivateInventoryProfile),
}

/// Inventory binding options.
#[derive(Clone)]
pub struct QvmInventoryOptions {
    /// Source module.
    pub module: QvmModule,
    /// Located game data.
    pub data: QvmGameData,
    /// Inventory profile.
    pub profile: QvmInventoryProfile,
    /// Weapon definitions (public profiles).
    pub weapons: QvmInventoryWeapons,
    /// Resolve the current client slot; never retained across restore.
    pub client: Rc<dyn Fn() -> usize>,
}

/// Borrowed source word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmInventoryWord {
    /// Word address.
    pub address: usize,
    /// Word value.
    pub value: i32,
}

/// Bound public field.
#[derive(Debug, Clone, PartialEq, Eq)]
struct QvmInventoryField {
    /// Item id.
    item: ItemId,
    /// Weapon slot.
    weapon: i32,
    /// Field kind.
    kind: QvmInventoryFieldKind,
}

/// Public field kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QvmInventoryFieldKind {
    /// Weapon ownership bit.
    Weapon,
    /// Ammo counter.
    Ammo,
}

/// Check that a profile belongs to the bound module and data.
fn validate_profile(options: &QvmInventoryOptions) -> Result<(), GuestError> {
    let (module, abi) = match &options.profile {
        QvmInventoryProfile::Public(profile) => (&profile.module, profile.abi_profile),
        QvmInventoryProfile::Private(profile) => (&profile.module, profile.abi_profile),
    };
    if !options.module.module_id().same_module(module)
        || options.module.abi_profile() != abi
        || options.data.abi_profile() != abi
    {
        return Err(GuestError::invalid(
            "QVM inventory profile belongs to another source module",
        ));
    }
    Ok(())
}

/// Public inventory over player-state words.
pub struct QvmPublicInventory {
    /// Source module.
    module: QvmModule,
    /// Located game data.
    data: QvmGameData,
    /// Public profile.
    profile: QvmPublicInventoryProfile,
    /// Weapon definitions.
    weapons: QvmInventoryWeapons,
    /// Client slot resolver.
    client: Rc<dyn Fn() -> usize>,
    /// Bound fields by item.
    fields: RefCell<HashMap<ItemId, QvmInventoryField>>,
    /// Previously validated definitions.
    previous: RefCell<Option<Vec<QvmInventoryWeapon>>>,
}

impl QvmPublicInventory {
    /// Rebuild bound fields when definitions change.
    fn refresh(&self) -> Result<(), GuestError> {
        let definitions = match &self.weapons {
            QvmInventoryWeapons::Fixed(definitions) => definitions.clone(),
            QvmInventoryWeapons::Dynamic(definitions) => definitions(),
        };
        if self.previous.borrow().as_ref() == Some(&definitions) {
            return Ok(());
        }
        let mut fields = HashMap::new();
        let mut slots = Vec::new();
        for definition in &definitions {
            if definition.weapon < 1 || definition.weapon >= 16 || slots.contains(&definition.weapon) {
                return Err(GuestError::invalid(
                    "QVM inventory requires distinct public weapon slots",
                ));
            }
            slots.push(definition.weapon);
            let mut bound = vec![(&definition.item, QvmInventoryFieldKind::Weapon)];
            if let Some(ammo) = &definition.ammo {
                bound.push((ammo, QvmInventoryFieldKind::Ammo));
            }
            for (item, kind) in bound {
                if fields.contains_key(item) {
                    return Err(GuestError::invalid(format!(
                        "QVM inventory item {item} aliases another source field"
                    )));
                }
                fields.insert(
                    item.clone(),
                    QvmInventoryField {
                        item: item.clone(),
                        weapon: definition.weapon,
                        kind,
                    },
                );
            }
        }
        *self.fields.borrow_mut() = fields;
        *self.previous.borrow_mut() = Some(definitions);
        Ok(())
    }

    /// Open the live public player record.
    fn view(&self) -> Result<QvmMemoryWindow, GuestError> {
        self.module.memory().assert_live()?;
        self.refresh()?;
        let record = self.data.public_player_bytes((self.client)())?;
        let exceeds = |offset: usize, len: usize| offset.checked_add(len).map_or(true, |end| end > record.len);
        if exceeds(self.profile.weapons_offset, 4) || exceeds(self.profile.ammo_offset, 64) {
            return Err(GuestError::invalid(
                "QVM inventory fields exceed the public player record",
            ));
        }
        Ok(record)
    }

    /// Resolve a field capacity.
    fn capacity(&self, field: &QvmInventoryField) -> Result<i32, GuestError> {
        if field.kind == QvmInventoryFieldKind::Weapon {
            return Ok(1);
        }
        let slot = (self.client)();
        let context = QvmInventoryCapacityContext {
            module: self.module.clone(),
            client: self.data.client_bytes(slot)?.offset,
            entity: self.data.entity_bytes(slot)?.offset,
            client_number: slot,
        };
        let value = (self.profile.capacity)(&self.module.memory(), field.weapon, &context)?;
        if value < 0 {
            return Err(GuestError::invalid(
                "QVM source ammo capacity is not a nonnegative int32",
            ));
        }
        Ok(value)
    }

    /// Read all bound entries.
    fn read(&self) -> Result<Vec<InventoryEntry>, GuestError> {
        let source = self.view()?;
        let mut entries = Vec::new();
        let fields: Vec<QvmInventoryField> = self.fields.borrow().values().cloned().collect();
        for field in &fields {
            if field.kind == QvmInventoryFieldKind::Weapon {
                let owned = source.get_i32(self.profile.weapons_offset)? & (1 << field.weapon) != 0;
                entries.push(InventoryEntry {
                    item: field.item.clone(),
                    count: f64::from(i32::from(owned)),
                    capacity: 1.0,
                    count_policy: None,
                });
            } else {
                entries.push(InventoryEntry {
                    item: field.item.clone(),
                    count: f64::from(source.get_i32(self.profile.ammo_offset + field.weapon as usize * 4)?),
                    capacity: f64::from(self.capacity(field)?),
                    count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
                });
            }
        }
        Ok(entries)
    }

    /// Write one entry.
    fn write(&self, entry: &InventoryEntry) -> Result<(), GuestError> {
        let source = self.view()?;
        let field = self
            .fields
            .borrow()
            .get(&entry.item)
            .cloned()
            .ok_or_else(|| GuestError::invalid(format!("QVM inventory did not admit {}", entry.item)))?;
        if entry.capacity != f64::from(self.capacity(&field)?) {
            return Err(GuestError::invalid("QVM inventory cannot change source capacity"));
        }
        if field.kind == QvmInventoryFieldKind::Weapon {
            if (entry.count != 0.0 && entry.count != 1.0)
                || entry.count_policy.is_some_and(|policy| policy != CountPolicy::Stack)
            {
                return Err(GuestError::invalid("QVM weapon ownership requires a zero or one stack"));
            }
            let before = source.get_i32(self.profile.weapons_offset)?;
            let bit = 1 << field.weapon;
            source.set_i32(
                self.profile.weapons_offset,
                if entry.count == 0.0 {
                    before & !bit
                } else {
                    before | bit
                },
            )?;
        } else {
            if entry.count.fract() != 0.0
                || entry.count < f64::from(i32::MIN)
                || entry.count > f64::from(i32::MAX)
                || entry.count_policy != Some(CountPolicy::SourceCounter(CountArithmetic::Int32))
            {
                return Err(GuestError::invalid("QVM ammo requires its signed int32 source counter"));
            }
            source.set_i32(self.profile.ammo_offset + field.weapon as usize * 4, entry.count as i32)?;
        }
        Ok(())
    }
}

/// Private inventory over declared storage.
pub struct QvmPrivateInventory {
    /// Source module.
    module: QvmModule,
    /// Located game data.
    data: QvmGameData,
    /// Private profile.
    profile: QvmPrivateInventoryProfile,
    /// Client slot resolver.
    client: Rc<dyn Fn() -> usize>,
    /// Storage by item.
    fields: HashMap<ItemId, QvmItemStorage>,
}

/// Live storage access over located records.
struct QvmPrivateAccess<'a> {
    /// Owning binding.
    binding: &'a QvmPrivateInventory,
}

impl QvmPrivateInventory {
    /// Resolve a field address.
    fn address(&self, field: &QvmItemField) -> Result<usize, GuestError> {
        self.module.memory().assert_live()?;
        let slot = (self.client)();
        if self.data.entity_stride_bytes() != self.profile.entity_stride
            || self.data.client_stride_bytes() != self.profile.client_stride
        {
            return Err(GuestError::invalid("Private QVM inventory source records changed"));
        }
        let record = if field.record == "client" {
            Some(self.data.client_bytes(slot)?)
        } else if field.record == "entity" {
            Some(self.data.entity_bytes(slot)?)
        } else {
            None
        };
        let Some(record) = record else {
            return Err(GuestError::invalid(
                "Private QVM inventory field exceeds its source record",
            ));
        };
        if field.offset % 4 != 0 || field.offset.checked_add(4).map_or(true, |end| end > record.len) {
            return Err(GuestError::invalid(
                "Private QVM inventory field exceeds its source record",
            ));
        }
        record
            .offset
            .checked_add(field.offset)
            .ok_or_else(|| GuestError::invalid("Private QVM inventory field exceeds its source record"))
    }

    /// Live access.
    fn access(&self) -> QvmPrivateAccess<'_> {
        QvmPrivateAccess { binding: self }
    }

    /// Read all storage entries.
    fn read(&self) -> Result<Vec<InventoryEntry>, GuestError> {
        let access = self.access();
        let mut entries = Vec::new();
        for storage in &self.profile.storage {
            entries.extend(read_qvm_item_storage(&self.profile.image, storage, &access)?);
        }
        Ok(entries)
    }

    /// Read one item entry.
    fn entry(&self, item: &ItemId) -> Result<Option<InventoryEntry>, GuestError> {
        let Some(storage) = self.fields.get(item) else {
            return Ok(None);
        };
        let access = self.access();
        Ok(read_qvm_item_storage(&self.profile.image, storage, &access)?
            .into_iter()
            .find(|entry| &entry.item == item))
    }

    /// Whether an item capacity is a mutable field.
    fn mutable_capacity(&self, item: &ItemId) -> bool {
        matches!(
            self.fields.get(item),
            Some(QvmItemStorage::Counter {
                capacity: QvmItemCapacity::Field { .. },
                ..
            })
        )
    }

    /// Write one entry.
    fn write(&self, entry: &InventoryEntry) -> Result<(), GuestError> {
        let Some(storage) = self.fields.get(&entry.item) else {
            return Err(GuestError::invalid(format!(
                "QVM inventory did not admit {}",
                entry.item
            )));
        };
        write_qvm_item_storage(&self.profile.image, storage, entry, &self.access())
    }
}

impl QvmItemStorageAccess for QvmPrivateAccess<'_> {
    fn read(&self, field: &QvmItemField) -> Result<i32, GuestError> {
        let address = self.binding.address(field)?;
        self.binding.module.memory().read_i32(address)
    }

    fn global(&self, address: usize) -> Result<i32, GuestError> {
        self.binding.module.memory().read_i32(address)
    }

    fn write(&self, field: &QvmItemField, value: i32) -> Result<(), GuestError> {
        let address = self.binding.address(field)?;
        self.binding.module.memory().write_i32(address, value)
    }
}

/// Canonical inventory binding borrowing original source words.
pub enum QvmInventoryBinding {
    /// Public player-state words.
    Public(QvmPublicInventory),
    /// Private declared storage.
    Private(QvmPrivateInventory),
}

impl QvmInventoryBinding {
    /// Read all bound entries.
    pub fn read(&self) -> Result<Vec<InventoryEntry>, GuestError> {
        match self {
            Self::Public(binding) => binding.read(),
            Self::Private(binding) => binding.read(),
        }
    }

    /// Write one entry.
    pub fn write(&self, entry: &InventoryEntry) -> Result<(), GuestError> {
        match self {
            Self::Public(binding) => binding.write(entry),
            Self::Private(binding) => binding.write(entry),
        }
    }

    /// Read one item entry (`None` when absent; public bindings have no entry).
    pub fn entry(&self, item: &ItemId) -> Result<Option<InventoryEntry>, GuestError> {
        match self {
            Self::Public(_) => Ok(None),
            Self::Private(binding) => binding.entry(item),
        }
    }

    /// Whether an item capacity is a mutable field.
    #[must_use]
    pub fn mutable_capacity(&self, item: &ItemId) -> bool {
        match self {
            Self::Public(_) => false,
            Self::Private(binding) => binding.mutable_capacity(item),
        }
    }
}

/// Bind canonical inventory to original source words.
pub fn qvm_inventory_binding(options: QvmInventoryOptions) -> Result<QvmInventoryBinding, GuestError> {
    validate_profile(&options)?;
    match options.profile {
        QvmInventoryProfile::Public(profile) => {
            for offset in [profile.weapons_offset, profile.ammo_offset] {
                if offset % 4 != 0 {
                    return Err(GuestError::invalid(
                        "QVM inventory fields require aligned source offsets",
                    ));
                }
            }
            if profile.weapons_offset >= profile.ammo_offset && profile.weapons_offset < profile.ammo_offset + 64 {
                return Err(GuestError::invalid("QVM inventory weapon bits overlap ammo counters"));
            }
            let binding = QvmPublicInventory {
                module: options.module,
                data: options.data,
                profile,
                weapons: options.weapons,
                client: options.client,
                fields: RefCell::new(HashMap::new()),
                previous: RefCell::new(None),
            };
            if matches!(binding.weapons, QvmInventoryWeapons::Fixed(_)) {
                binding.refresh()?;
            }
            Ok(QvmInventoryBinding::Public(binding))
        }
        QvmInventoryProfile::Private(profile) => {
            let mut fields = HashMap::new();
            for storage in &profile.storage {
                match storage {
                    QvmItemStorage::Counter { item, .. } => {
                        fields.insert(item.clone(), storage.clone());
                    }
                    QvmItemStorage::Bits { items, .. } => {
                        for value in items {
                            fields.insert(value.item.clone(), storage.clone());
                        }
                    }
                }
            }
            Ok(QvmInventoryBinding::Private(QvmPrivateInventory {
                module: options.module,
                data: options.data,
                profile,
                client: options.client,
                fields,
            }))
        }
    }
}

/// Compute borrowed source words without publishing a committed grant.
pub fn qvm_inventory_projection(
    options: &QvmInventoryOptions,
    item: &ItemId,
    count: i32,
) -> Result<Vec<QvmInventoryWord>, GuestError> {
    validate_profile(options)?;
    match &options.profile {
        QvmInventoryProfile::Private(profile) => {
            let binding = qvm_inventory_binding(options.clone())?;
            let entry = binding
                .entry(item)?
                .ok_or_else(|| GuestError::invalid("Projected QVM item has no original inventory storage"))?;
            let storage = profile.storage.iter().find(|storage| match storage {
                QvmItemStorage::Counter { item: held, .. } => held == item,
                QvmItemStorage::Bits { items, .. } => items.iter().any(|value| &value.item == item),
            });
            let Some(storage) = storage else {
                return Err(GuestError::invalid(
                    "Projected QVM item has no original inventory storage",
                ));
            };
            let words = RefCell::new(HashMap::new());
            let address = |field: &QvmItemField| -> Result<usize, GuestError> {
                let slot = (options.client)();
                let record = if field.record == "client" {
                    Some(options.data.client_bytes(slot)?)
                } else if field.record == "entity" {
                    Some(options.data.entity_bytes(slot)?)
                } else {
                    None
                };
                let Some(record) = record else {
                    return Err(GuestError::invalid(
                        "Projected QVM item field exceeds its original record",
                    ));
                };
                if field.offset.checked_add(4).map_or(true, |end| end > record.len) {
                    return Err(GuestError::invalid(
                        "Projected QVM item field exceeds its original record",
                    ));
                }
                record
                    .offset
                    .checked_add(field.offset)
                    .ok_or_else(|| GuestError::invalid("Projected QVM item field exceeds its original record"))
            };
            struct Shadow<'a> {
                options: &'a QvmInventoryOptions,
                address: &'a dyn Fn(&QvmItemField) -> Result<usize, GuestError>,
                words: &'a RefCell<HashMap<usize, i32>>,
            }
            impl QvmItemStorageAccess for Shadow<'_> {
                fn read(&self, field: &QvmItemField) -> Result<i32, GuestError> {
                    let at = (self.address)(field)?;
                    if let Some(value) = self.words.borrow().get(&at) {
                        return Ok(*value);
                    }
                    self.options.module.memory().read_i32(at)
                }

                fn global(&self, address: usize) -> Result<i32, GuestError> {
                    self.options.module.memory().read_i32(address)
                }

                fn write(&self, field: &QvmItemField, value: i32) -> Result<(), GuestError> {
                    let at = (self.address)(field)?;
                    self.words.borrow_mut().insert(at, value);
                    Ok(())
                }
            }
            let shadow = Shadow {
                options,
                address: &address,
                words: &words,
            };
            write_qvm_item_storage(
                &profile.image,
                storage,
                &InventoryEntry {
                    count: f64::from(count),
                    ..entry
                },
                &shadow,
            )?;
            let mut projected = Vec::new();
            for (address, value) in words.borrow().iter() {
                if options.module.memory().read_i32(*address)? != *value {
                    projected.push(QvmInventoryWord {
                        address: *address,
                        value: *value,
                    });
                }
            }
            Ok(projected)
        }
        QvmInventoryProfile::Public(profile) => {
            let definitions = match &options.weapons {
                QvmInventoryWeapons::Fixed(definitions) => definitions.clone(),
                QvmInventoryWeapons::Dynamic(definitions) => definitions(),
            };
            let weapon = definitions
                .iter()
                .find(|definition| &definition.item == item || definition.ammo.as_ref() == Some(item));
            let Some(weapon) = weapon else {
                return Err(GuestError::invalid("Projected QVM item has no public inventory slot"));
            };
            if weapon.weapon < 1 || weapon.weapon > 15 {
                return Err(GuestError::invalid("Projected QVM item has no public inventory slot"));
            }
            let source = options.data.public_player_bytes((options.client)())?;
            let base = source.offset;
            if &weapon.item == item {
                if count != 0 && count != 1 {
                    return Err(GuestError::invalid("Projected QVM ownership requires zero or one"));
                }
                let before = source.get_i32(profile.weapons_offset)?;
                let mask = 1 << weapon.weapon;
                return Ok(vec![QvmInventoryWord {
                    address: base + profile.weapons_offset,
                    value: if count == 0 { before & !mask } else { before | mask },
                }]);
            }
            Ok(vec![QvmInventoryWord {
                address: base + profile.ammo_offset + weapon.weapon as usize * 4,
                value: count,
            }])
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::super::game_data::{ModuleIdentity, QvmArtifact, QvmImage, QvmRole};
    use super::*;

    const ENTITIES: usize = 4096;
    const CLIENTS: usize = 8192;
    const ENTITY_STRIDE: usize = 256;
    const CLIENT_STRIDE: usize = 512;

    struct Fixture {
        options: QvmInventoryOptions,
        capacities: Rc<RefCell<Vec<(i32, usize, usize, usize)>>>,
    }

    fn module() -> (QvmModule, QvmGameData) {
        let mut image = QvmImage::default();
        image.allocated_data_length = 65536;
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact, None, None).unwrap();
        let data = QvmGameData::new(module.memory(), AbiProfile::Modern);
        data.locate(ENTITIES as i32, 4, ENTITY_STRIDE, CLIENTS as i32, CLIENT_STRIDE)
            .unwrap();
        (module, data)
    }

    fn public_fixture() -> Fixture {
        let (module, data) = module();
        let capacities = Rc::new(RefCell::new(Vec::new()));
        let recorded = Rc::clone(&capacities);
        let profile = QvmPublicInventoryProfile {
            module: module.module_id(),
            abi_profile: AbiProfile::Modern,
            weapons_offset: 64,
            ammo_offset: 128,
            capacity: Rc::new(
                move |_memory: &QvmSharedMemory, weapon: i32, context: &QvmInventoryCapacityContext| {
                    recorded
                        .borrow_mut()
                        .push((weapon, context.client, context.entity, context.client_number));
                    Ok(100 + weapon)
                },
            ),
        };
        Fixture {
            options: QvmInventoryOptions {
                module,
                data,
                profile: QvmInventoryProfile::Public(profile),
                weapons: QvmInventoryWeapons::Fixed(vec![
                    QvmInventoryWeapon {
                        weapon: 2,
                        item: "q3:shotgun".to_string(),
                        ammo: Some("q3:shells".to_string()),
                    },
                    QvmInventoryWeapon {
                        weapon: 5,
                        item: "q3:railgun".to_string(),
                        ammo: Some("q3:slugs".to_string()),
                    },
                ]),
                client: Rc::new(|| 1),
            },
            capacities,
        }
    }

    fn private_fixture() -> Fixture {
        let (module, data) = module();
        let mut image = QvmImage::default();
        image.instructions = vec![super::super::game_data::QvmInstruction::word(
            super::super::game_data::QvmOpcode::OpConst,
            200,
            0,
        )];
        image.initialized_data = vec![0u8; 64];
        let profile = QvmPrivateInventoryProfile {
            module: module.module_id(),
            abi_profile: AbiProfile::Modern,
            entity_stride: ENTITY_STRIDE,
            client_stride: CLIENT_STRIDE,
            image,
            storage: vec![
                QvmItemStorage::Counter {
                    field: QvmItemField {
                        record: "client".to_string(),
                        offset: 32,
                    },
                    item: "q3:rockets".to_string(),
                    capacity: QvmItemCapacity::Constant { value: 200 },
                },
                QvmItemStorage::Bits {
                    field: QvmItemField {
                        record: "entity".to_string(),
                        offset: 40,
                    },
                    private_mask: 1,
                    items: vec![super::super::item_storage::QvmPackedItem {
                        item: "q3:gauntlet".to_string(),
                        mask: 2,
                    }],
                },
            ],
        };
        Fixture {
            options: QvmInventoryOptions {
                module,
                data,
                profile: QvmInventoryProfile::Private(profile),
                weapons: QvmInventoryWeapons::Fixed(Vec::new()),
                client: Rc::new(|| 1),
            },
            capacities: Rc::new(RefCell::new(Vec::new())),
        }
    }

    #[test]
    fn public_inventory_reads_bits_and_counters() {
        let fixture = public_fixture();
        let binding = qvm_inventory_binding(fixture.options.clone()).unwrap();
        let base = CLIENTS + CLIENT_STRIDE;
        fixture.options.module.memory().write_i32(base + 64, 1 << 5).unwrap();
        fixture
            .options
            .module
            .memory()
            .write_i32(base + 128 + 2 * 4, 17)
            .unwrap();
        fixture
            .options
            .module
            .memory()
            .write_i32(base + 128 + 5 * 4, 3)
            .unwrap();
        let mut entries = binding.read().unwrap();
        entries.sort_by(|left, right| left.item.cmp(&right.item));
        assert_eq!(entries.len(), 4);
        let shotgun = entries.iter().find(|entry| entry.item == "q3:shotgun").unwrap();
        assert_eq!((shotgun.count, shotgun.capacity), (0.0, 1.0));
        assert_eq!(shotgun.count_policy, None);
        let railgun = entries.iter().find(|entry| entry.item == "q3:railgun").unwrap();
        assert_eq!((railgun.count, railgun.capacity), (1.0, 1.0));
        let shells = entries.iter().find(|entry| entry.item == "q3:shells").unwrap();
        assert_eq!((shells.count, shells.capacity), (17.0, 102.0));
        assert_eq!(
            shells.count_policy,
            Some(CountPolicy::SourceCounter(CountArithmetic::Int32))
        );
        let slugs = entries.iter().find(|entry| entry.item == "q3:slugs").unwrap();
        assert_eq!((slugs.count, slugs.capacity), (3.0, 105.0));
        let contexts = fixture.capacities.borrow();
        assert!(contexts.iter().all(|(_, client, entity, number)| {
            *client == base && *entity == ENTITIES + ENTITY_STRIDE && *number == 1
        }));
    }

    #[test]
    fn public_inventory_writes_bits_and_counters() {
        let fixture = public_fixture();
        let binding = qvm_inventory_binding(fixture.options.clone()).unwrap();
        binding
            .write(&InventoryEntry {
                item: "q3:shotgun".to_string(),
                count: 1.0,
                capacity: 1.0,
                count_policy: Some(CountPolicy::Stack),
            })
            .unwrap();
        let base = CLIENTS + CLIENT_STRIDE;
        assert_eq!(fixture.options.module.memory().read_i32(base + 64).unwrap(), 1 << 2);
        binding
            .write(&InventoryEntry {
                item: "q3:shotgun".to_string(),
                count: 0.0,
                capacity: 1.0,
                count_policy: None,
            })
            .unwrap();
        assert_eq!(fixture.options.module.memory().read_i32(base + 64).unwrap(), 0);
        binding
            .write(&InventoryEntry {
                item: "q3:shells".to_string(),
                count: -4.0,
                capacity: 102.0,
                count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
            })
            .unwrap();
        assert_eq!(
            fixture.options.module.memory().read_i32(base + 128 + 2 * 4).unwrap(),
            -4
        );

        let rejected = InventoryEntry {
            item: "q3:shotgun".to_string(),
            count: 2.0,
            capacity: 1.0,
            count_policy: None,
        };
        assert!(binding.write(&rejected).is_err());
        let foreign = InventoryEntry {
            item: "q3:shotgun".to_string(),
            count: 1.0,
            capacity: 1.0,
            count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
        };
        assert!(binding.write(&foreign).is_err());
        let capacity = InventoryEntry {
            item: "q3:shells".to_string(),
            count: 1.0,
            capacity: 50.0,
            count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
        };
        assert!(binding.write(&capacity).is_err());
        let stack = InventoryEntry {
            item: "q3:shells".to_string(),
            count: 1.0,
            capacity: 102.0,
            count_policy: Some(CountPolicy::Stack),
        };
        assert!(binding.write(&stack).is_err());
        let unadmitted = InventoryEntry {
            item: "q3:unknown".to_string(),
            count: 1.0,
            capacity: 1.0,
            count_policy: None,
        };
        assert!(binding.write(&unadmitted).is_err());
        assert!(!binding.mutable_capacity(&"q3:shells".to_string()));
        assert_eq!(binding.entry(&"q3:shells".to_string()).unwrap(), None);
    }

    #[test]
    fn public_definitions_validate_slots_and_offsets() {
        let fixture = public_fixture();
        let invalid = |weapons: Vec<QvmInventoryWeapon>| {
            let mut options = fixture.options.clone();
            options.weapons = QvmInventoryWeapons::Fixed(weapons);
            qvm_inventory_binding(options).is_err()
        };
        let weapon = |weapon: i32, item: &str, ammo: Option<&str>| QvmInventoryWeapon {
            weapon,
            item: item.to_string(),
            ammo: ammo.map(str::to_string),
        };
        assert!(invalid(vec![weapon(0, "q3:a", None)]));
        assert!(invalid(vec![weapon(16, "q3:a", None)]));
        assert!(invalid(vec![weapon(2, "q3:a", None), weapon(2, "q3:b", None)]));
        assert!(invalid(vec![weapon(2, "q3:a", Some("q3:a"))]));
        assert!(invalid(vec![weapon(2, "q3:a", None), weapon(3, "q3:a", None)]));

        let mut options = fixture.options.clone();
        let QvmInventoryProfile::Public(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.weapons_offset = 66;
        assert!(qvm_inventory_binding(options.clone()).is_err());
        let QvmInventoryProfile::Public(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.weapons_offset = 128;
        assert!(qvm_inventory_binding(options).is_err());
    }

    #[test]
    fn dynamic_definitions_refresh_per_view() {
        let fixture = public_fixture();
        let calls = Rc::new(RefCell::new(0));
        let recorded = Rc::clone(&calls);
        let mut options = fixture.options.clone();
        options.weapons = QvmInventoryWeapons::Dynamic(Rc::new(move || {
            *recorded.borrow_mut() += 1;
            vec![QvmInventoryWeapon {
                weapon: 2,
                item: "q3:shotgun".to_string(),
                ammo: None,
            }]
        }));
        let binding = qvm_inventory_binding(options).unwrap();
        assert_eq!(*calls.borrow(), 0);
        assert_eq!(binding.read().unwrap().len(), 1);
        assert_eq!(*calls.borrow(), 1);
        assert_eq!(binding.read().unwrap().len(), 1);
        assert_eq!(*calls.borrow(), 2);
    }

    #[test]
    fn private_inventory_binds_declared_storage() {
        let fixture = private_fixture();
        let binding = qvm_inventory_binding(fixture.options.clone()).unwrap();
        fixture
            .options
            .module
            .memory()
            .write_i32(CLIENTS + CLIENT_STRIDE + 32, 17)
            .unwrap();
        fixture
            .options
            .module
            .memory()
            .write_i32(ENTITIES + ENTITY_STRIDE + 40, 3)
            .unwrap();
        let mut entries = binding.read().unwrap();
        entries.sort_by(|left, right| left.item.cmp(&right.item));
        assert_eq!(entries.len(), 2);
        let rockets = binding.entry(&"q3:rockets".to_string()).unwrap().unwrap();
        assert_eq!((rockets.count, rockets.capacity), (17.0, 200.0));
        let gauntlet = binding.entry(&"q3:gauntlet".to_string()).unwrap().unwrap();
        assert_eq!((gauntlet.count, gauntlet.capacity), (1.0, 1.0));
        assert_eq!(binding.entry(&"q3:unknown".to_string()).unwrap(), None);
        assert!(!binding.mutable_capacity(&"q3:rockets".to_string()));

        binding
            .write(&InventoryEntry {
                item: "q3:gauntlet".to_string(),
                count: 0.0,
                capacity: 1.0,
                count_policy: None,
            })
            .unwrap();
        assert_eq!(
            fixture
                .options
                .module
                .memory()
                .read_i32(ENTITIES + ENTITY_STRIDE + 40)
                .unwrap(),
            1
        );
        let unadmitted = InventoryEntry {
            item: "q3:unknown".to_string(),
            count: 1.0,
            capacity: 1.0,
            count_policy: None,
        };
        assert!(binding.write(&unadmitted).is_err());
    }

    #[test]
    fn private_fields_validate_records_and_strides() {
        let fixture = private_fixture();
        let binding = qvm_inventory_binding(fixture.options.clone()).unwrap();
        assert!(binding.read().is_ok());

        let mut options = fixture.options.clone();
        let QvmInventoryProfile::Private(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.client_stride = 256;
        assert!(qvm_inventory_binding(options).unwrap().read().is_err());

        let mut options = fixture.options.clone();
        let QvmInventoryProfile::Private(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.storage[0] = QvmItemStorage::Counter {
            field: QvmItemField {
                record: "bogus".to_string(),
                offset: 32,
            },
            item: "q3:rockets".to_string(),
            capacity: QvmItemCapacity::Constant { value: 200 },
        };
        assert!(qvm_inventory_binding(options).unwrap().read().is_err());

        let mut options = fixture.options.clone();
        let QvmInventoryProfile::Private(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.storage[0] = QvmItemStorage::Counter {
            field: QvmItemField {
                record: "client".to_string(),
                offset: 34,
            },
            item: "q3:rockets".to_string(),
            capacity: QvmItemCapacity::Constant { value: 200 },
        };
        assert!(qvm_inventory_binding(options).unwrap().read().is_err());
    }

    #[test]
    fn profiles_reject_foreign_modules() {
        let fixture = public_fixture();
        let mut options = fixture.options.clone();
        let QvmInventoryProfile::Public(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.module.revision = "2".to_string();
        assert!(qvm_inventory_binding(options.clone()).is_err());
        assert!(qvm_inventory_projection(&options, &"q3:shotgun".to_string(), 1).is_err());

        let fixture = private_fixture();
        let mut options = fixture.options.clone();
        let QvmInventoryProfile::Private(profile) = &mut options.profile else {
            unreachable!();
        };
        profile.abi_profile = AbiProfile::Legacy;
        assert!(qvm_inventory_binding(options.clone()).is_err());
        assert!(qvm_inventory_projection(&options, &"q3:rockets".to_string(), 1).is_err());
    }

    #[test]
    fn public_projection_computes_borrowed_words() {
        let fixture = public_fixture();
        let base = CLIENTS + CLIENT_STRIDE;
        fixture.options.module.memory().write_i32(base + 64, 1 << 2).unwrap();
        let words = qvm_inventory_projection(&fixture.options, &"q3:shotgun".to_string(), 0).unwrap();
        assert_eq!(
            words,
            vec![QvmInventoryWord {
                address: base + 64,
                value: 0
            }]
        );
        let words = qvm_inventory_projection(&fixture.options, &"q3:shells".to_string(), 42).unwrap();
        assert_eq!(
            words,
            vec![QvmInventoryWord {
                address: base + 128 + 2 * 4,
                value: 42
            }]
        );
        assert_eq!(fixture.options.module.memory().read_i32(base + 64).unwrap(), 1 << 2);
        assert!(qvm_inventory_projection(&fixture.options, &"q3:shotgun".to_string(), 2).is_err());
        assert!(qvm_inventory_projection(&fixture.options, &"q3:unknown".to_string(), 1).is_err());
    }

    #[test]
    fn private_projection_returns_changed_words_only() {
        let fixture = private_fixture();
        fixture
            .options
            .module
            .memory()
            .write_i32(CLIENTS + CLIENT_STRIDE + 32, 17)
            .unwrap();
        let words = qvm_inventory_projection(&fixture.options, &"q3:rockets".to_string(), 17).unwrap();
        assert!(words.is_empty());
        let words = qvm_inventory_projection(&fixture.options, &"q3:rockets".to_string(), 19).unwrap();
        assert_eq!(
            words,
            vec![QvmInventoryWord {
                address: CLIENTS + CLIENT_STRIDE + 32,
                value: 19
            }]
        );
        assert!(qvm_inventory_projection(&fixture.options, &"q3:unknown".to_string(), 1).is_err());
    }
}
