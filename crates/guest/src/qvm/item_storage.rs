//! QVM item storage: counters and packed bits over declared fields.
//!
//! Provenance: `src/compat/qvm/item-storage.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-mod-items.ts`
//! ([`QvmItemField`], [`QvmItemTest`], [`QvmItemCapacity`],
//! [`QvmItemStorage`], [`QvmWeaponActor`], [`QvmWeaponStage`],
//! [`QvmModItems`]) plus the declaration parsers of
//! `src/content/mods/qvm-items.ts` ([`parse_qvm_item_storage`],
//! [`parse_qvm_mod_items`]).
//!
//! [`QvmModSourceCall`] and [`QvmModInputPointer`] come from
//! [`super::mod_actors`] (which absorbs `qvm-mod-callbacks.ts`).
//! [`HeldWeaponDeclaration`] is a local mirror of
//! `src/contracts/held-weapon.ts` (model path, reference frame, grip
//! transform, optional fallback/part); no values cross into other modules.
//! Entries reuse [`qa_world::inventory::InventoryEntry`].

use std::collections::HashSet;

use qa_core::math::Vec3;
use qa_world::combat::ItemId;
use qa_world::inventory::{CountArithmetic, CountPolicy, InventoryEntry};

use super::game_data::{namespaced_id, ProfileReader, QvmImage, QvmOpcode};
use super::mod_actors::{QvmModInputPointer, QvmModInputPointerBase, QvmModSourceCall};
use crate::error::GuestError;

/// Model grip transform (mirror of `ModelTransform` in `scene.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWeaponGrip {
    /// Grip origin.
    pub origin: Vec3,
    /// Grip axis.
    pub axis: [Vec3; 3],
    /// Grip scale.
    pub scale: Vec3,
}

/// Held-weapon model (mirror of `HeldWeaponModel` in `held-weapon.ts`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWeaponModel {
    /// Content digest, if pinned.
    pub digest: Option<String>,
    /// Model path.
    pub path: String,
    /// Reference frame.
    pub reference_frame: i32,
    /// Grip transform.
    pub grip: HeldWeaponGrip,
    /// Fallback model path.
    pub fallback: Option<String>,
    /// Optional part restriction (digests plus vertices).
    pub part: Option<HeldWeaponPart>,
}

/// Held-weapon part restriction.
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWeaponPart {
    /// Part digests.
    pub digests: Vec<String>,
    /// Part vertices.
    pub vertices: Vec<f64>,
}

/// Held-weapon presentation declaration (mirror of `held-weapon.ts`).
#[derive(Debug, Clone, PartialEq)]
pub enum HeldWeaponDeclaration {
    /// No held weapon.
    None,
    /// Held model.
    Model(HeldWeaponModel),
}

/// Declared record field: record id plus byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmItemField {
    /// Record id.
    pub record: String,
    /// Byte offset.
    pub offset: usize,
}

/// Item field test comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmItemTestComparison {
    /// Equality.
    Equals,
    /// At most.
    AtMost,
}

/// Declared field test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmItemTest {
    /// Tested field.
    pub field: QvmItemField,
    /// Bit mask, if any.
    pub mask: Option<u32>,
    /// Comparison.
    pub comparison: QvmItemTestComparison,
    /// Compared value.
    pub value: i64,
}

/// Capacity selector comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmCapacityComparison {
    /// Equality.
    Equals,
    /// Inequality.
    NotEquals,
}

/// Capacity selector override.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCapacityOverride {
    /// Selector global address.
    pub address: usize,
    /// Comparison.
    pub comparison: QvmCapacityComparison,
    /// Compared value.
    pub value: i64,
    /// Constant instruction selected on match.
    pub instruction: usize,
}

/// Declared item capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmItemCapacity {
    /// Constant capacity.
    Constant {
        /// Capacity value.
        value: i32,
    },
    /// Field capacity.
    Field {
        /// Capacity field.
        field: QvmItemField,
    },
    /// Source-selected capacity.
    Source {
        /// Default constant instruction.
        instruction: usize,
        /// Selector overrides.
        overrides: Vec<QvmCapacityOverride>,
    },
}

/// Packed bit item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmPackedItem {
    /// Item id.
    pub item: ItemId,
    /// Bit mask.
    pub mask: u32,
}

/// Declared item storage: a counter or packed bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmItemStorage {
    /// Counter storage.
    Counter {
        /// Count field.
        field: QvmItemField,
        /// Item id.
        item: ItemId,
        /// Capacity.
        capacity: QvmItemCapacity,
    },
    /// Packed-bits storage.
    Bits {
        /// Word field.
        field: QvmItemField,
        /// Private mask bits.
        private_mask: u32,
        /// Packed items.
        items: Vec<QvmPackedItem>,
    },
}

/// Weapon actor: record plus caller-storage pointer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponActor {
    /// Record id.
    pub record: String,
    /// Actor pointer.
    pub pointer: QvmModInputPointer,
}

/// Weapon dispatcher predicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmWeaponPredicate {
    /// Conditional instruction.
    pub instruction: usize,
    /// Forced direction when unselected.
    pub unselected: bool,
}

/// Weapon selection value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmSelectionValue {
    /// Source value.
    pub value: i64,
    /// Item id.
    pub item: ItemId,
}

/// Weapon stage call.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmStageCall {
    /// Call-site instruction.
    pub instruction: usize,
    /// Source call.
    pub call: QvmModSourceCall,
}

/// Weapon continuation projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmProjection {
    /// Movement pointer.
    pub movement: QvmModInputPointer,
    /// Projection byte length.
    pub byte_length: usize,
    /// Minimum value.
    pub minimum: i64,
    /// Maximum value.
    pub maximum: i64,
    /// View-height field.
    pub view_height: QvmItemField,
    /// Ground field.
    pub ground: QvmItemField,
}

/// Weapon dispatcher definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponStage {
    /// Dispatcher entry and actor.
    pub dispatcher: QvmWeaponDispatcher,
    /// Dispatcher predicates.
    pub predicates: Vec<QvmWeaponPredicate>,
    /// Settled-action tests.
    pub settled: Vec<QvmItemTest>,
    /// Selection field and values.
    pub selection: QvmWeaponSelection,
    /// Attack-request entry, argument, and accepted tests.
    pub request: QvmWeaponRequest,
    /// Continuation.
    pub continuation: QvmWeaponContinuation,
}

/// Weapon dispatcher entry and actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponDispatcher {
    /// Dispatcher entry.
    pub entry: usize,
    /// Dispatcher actor.
    pub actor: QvmWeaponActor,
}

/// Weapon selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponSelection {
    /// Selection field.
    pub field: QvmItemField,
    /// Selection values.
    pub values: Vec<QvmSelectionValue>,
}

/// Weapon attack request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponRequest {
    /// Request entry.
    pub entry: usize,
    /// Request argument.
    pub argument: usize,
    /// Accepted tests.
    pub accepted: Vec<QvmItemTest>,
}

/// Weapon continuation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmWeaponContinuation {
    /// Continuation entry.
    pub entry: usize,
    /// Continuation actor.
    pub actor: QvmWeaponActor,
    /// Continuation instruction.
    pub instruction: usize,
    /// Original taken direction.
    pub original_taken: bool,
    /// Guard tests.
    pub when: Vec<QvmItemTest>,
    /// Predicates.
    pub predicates: Vec<QvmWeaponPredicate>,
    /// Projection.
    pub projection: QvmProjection,
    /// Calls.
    pub calls: Vec<QvmStageCall>,
}

/// Item icon declaration (mirror of `SourceItemIconDeclaration`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmItemIcon {
    /// Image path.
    Image {
        /// Image path.
        path: String,
    },
    /// WAD picture.
    WadPicture {
        /// WAD path.
        path: String,
        /// Lump name.
        lump: String,
    },
    /// Shader name.
    Shader {
        /// Shader name.
        name: String,
    },
}

/// Item action calls (mirror of `SourceItemActionCalls`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QvmItemActions {
    /// Use action.
    pub use_call: Option<QvmModSourceCall>,
    /// Drop action.
    pub drop_call: Option<QvmModSourceCall>,
}

impl QvmItemActions {
    /// Declared action names in use/drop order.
    #[must_use]
    pub fn action_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.use_call.is_some() {
            names.push("use");
        }
        if self.drop_call.is_some() {
            names.push("drop");
        }
        names
    }
}

/// Item admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmItemAdmission {
    /// Add the item.
    Add,
    /// Replace the primary item.
    ReplacePrimary,
}

/// Mod item kind.
#[derive(Debug, Clone, PartialEq)]
pub enum QvmModItemKind {
    /// Counter item.
    Counter,
    /// Weapon item.
    Weapon {
        /// Ammo item, if any.
        ammo: Option<ItemId>,
        /// Held-weapon presentation, if any.
        held: Option<HeldWeaponDeclaration>,
    },
}

/// Mod item definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModItemDefinition {
    /// Item id.
    pub item: ItemId,
    /// Display label.
    pub label: String,
    /// Icon, if any.
    pub icon: Option<QvmItemIcon>,
    /// Admission.
    pub admission: QvmItemAdmission,
    /// Actions, if any.
    pub actions: Option<QvmItemActions>,
    /// Item kind.
    pub kind: QvmModItemKind,
}

/// Mod weapons block.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModItemWeapons {
    /// Input entry.
    pub input_entry: usize,
    /// Input clock field.
    pub input_clock: QvmItemField,
    /// Weapon stage.
    pub stage: QvmWeaponStage,
}

/// Mod items declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmModItems {
    /// Item definitions.
    pub definitions: Vec<QvmModItemDefinition>,
    /// Item storage.
    pub storage: Vec<QvmItemStorage>,
    /// Weapons block, if any.
    pub weapons: Option<QvmModItemWeapons>,
}

fn parse_field(reader: &ProfileReader<'_>) -> Result<QvmItemField, GuestError> {
    Ok(QvmItemField {
        record: reader.field("record")?.string()?,
        offset: reader.field("offset")?.integer(0)? as usize,
    })
}

fn parse_test(reader: &ProfileReader<'_>) -> Result<QvmItemTest, GuestError> {
    let mask = reader.field("mask")?.nullable(|value| value.integer(0))?;
    let mask = match mask {
        None => None,
        Some(mask) => Some(u32::try_from(mask).map_err(|_| GuestError::invalid("test mask exceeds 32 bits"))?),
    };
    let comparison = reader.field("comparison")?.choice(&["equals", "at-most"])?;
    Ok(QvmItemTest {
        field: parse_field(&reader.field("field")?)?,
        mask,
        comparison: if comparison == "equals" {
            QvmItemTestComparison::Equals
        } else {
            QvmItemTestComparison::AtMost
        },
        value: reader.field("value")?.integer(i64::MIN)?,
    })
}

fn parse_pointer(reader: &ProfileReader<'_>) -> Result<QvmModInputPointer, GuestError> {
    let kind = reader.field("kind")?.choice(&["argument", "global"])?;
    let indirections = reader
        .field("indirections")?
        .list(|value| value.integer(0))?
        .into_iter()
        .map(|offset| offset as usize)
        .collect();
    let offset = reader.field("offset")?.integer(0)? as usize;
    let base = if kind == "argument" {
        QvmModInputPointerBase::Argument {
            index: reader.field("index")?.integer(0)? as usize,
        }
    } else {
        QvmModInputPointerBase::Global {
            address: reader.field("address")?.integer(0)? as usize,
        }
    };
    Ok(QvmModInputPointer {
        base,
        indirections,
        offset,
    })
}

fn parse_actor(reader: &ProfileReader<'_>) -> Result<QvmWeaponActor, GuestError> {
    Ok(QvmWeaponActor {
        record: reader.field("record")?.string()?,
        pointer: parse_pointer(&reader.field("pointer")?)?,
    })
}

fn parse_capacity(reader: &ProfileReader<'_>) -> Result<QvmItemCapacity, GuestError> {
    let kind = reader.field("kind")?.choice(&["constant", "field", "source"])?;
    if kind == "constant" {
        let value = reader.field("value")?.integer(0)?;
        let value = i32::try_from(value).map_err(|_| GuestError::invalid("capacity exceeds the signed source ABI"))?;
        return Ok(QvmItemCapacity::Constant { value });
    }
    if kind == "field" {
        return Ok(QvmItemCapacity::Field {
            field: parse_field(&reader.field("field")?)?,
        });
    }
    Ok(QvmItemCapacity::Source {
        instruction: reader.field("instruction")?.integer(0)? as usize,
        overrides: reader.field("overrides")?.list(|value| {
            let comparison = value.field("comparison")?.choice(&["equals", "not-equals"])?;
            Ok(QvmCapacityOverride {
                address: value.field("address")?.integer(0)? as usize,
                comparison: if comparison == "equals" {
                    QvmCapacityComparison::Equals
                } else {
                    QvmCapacityComparison::NotEquals
                },
                value: value.field("value")?.integer(i64::MIN)?,
                instruction: value.field("instruction")?.integer(0)? as usize,
            })
        })?,
    })
}

/// Parse one item-storage declaration.
pub fn parse_qvm_item_storage(reader: &ProfileReader<'_>) -> Result<QvmItemStorage, GuestError> {
    let field = parse_field(&reader.field("field")?)?;
    if reader.field("kind")?.choice(&["counter", "bits"])? == "counter" {
        return Ok(QvmItemStorage::Counter {
            field,
            item: namespaced_id(&reader.field("item")?)?,
            capacity: parse_capacity(&reader.field("capacity")?)?,
        });
    }
    let private_mask = reader.field("privateMask")?.integer(0)?;
    let private_mask = u32::try_from(private_mask).map_err(|_| GuestError::invalid("privateMask exceeds 32 bits"))?;
    Ok(QvmItemStorage::Bits {
        field,
        private_mask,
        items: reader.field("items")?.list(|entry| {
            let mask = entry.field("mask")?.integer(1)?;
            let mask = u32::try_from(mask).map_err(|_| GuestError::invalid("item mask exceeds 32 bits"))?;
            Ok(QvmPackedItem {
                item: namespaced_id(&entry.field("item")?)?,
                mask,
            })
        })?,
    })
}

fn parse_icon(reader: &ProfileReader<'_>) -> Result<QvmItemIcon, GuestError> {
    let kind = reader.field("kind")?.choice(&["image", "wad-picture", "shader"])?;
    if kind == "image" {
        return Ok(QvmItemIcon::Image {
            path: reader.field("path")?.string()?,
        });
    }
    if kind == "wad-picture" {
        return Ok(QvmItemIcon::WadPicture {
            path: reader.field("path")?.string()?,
            lump: reader.field("lump")?.string()?,
        });
    }
    Ok(QvmItemIcon::Shader {
        name: reader.field("name")?.string()?,
    })
}

fn parse_actions(
    reader: &ProfileReader<'_>,
    read_call: &dyn Fn(&ProfileReader<'_>) -> Result<QvmModSourceCall, GuestError>,
) -> Result<QvmItemActions, GuestError> {
    let use_call = if reader.field("use")?.is_undefined() {
        None
    } else {
        Some(read_call(&reader.field("use")?)?)
    };
    let drop_call = if reader.field("drop")?.is_undefined() {
        None
    } else {
        Some(read_call(&reader.field("drop")?)?)
    };
    Ok(QvmItemActions { use_call, drop_call })
}

/// Parse a mod-items declaration, reading source calls through `read_call`.
pub fn parse_qvm_mod_items(
    reader: &ProfileReader<'_>,
    read_call: &dyn Fn(&ProfileReader<'_>) -> Result<QvmModSourceCall, GuestError>,
    read_held: &dyn Fn(&ProfileReader<'_>) -> Result<HeldWeaponDeclaration, GuestError>,
) -> Result<QvmModItems, GuestError> {
    let definitions = reader.field("definitions")?.list(|value| {
        let icon = if value.field("icon")?.is_undefined() {
            None
        } else {
            value.field("icon")?.nullable(parse_icon)?
        };
        let actions = if value.field("actions")?.is_undefined() {
            None
        } else {
            Some(parse_actions(&value.field("actions")?, read_call)?)
        };
        let admission = value.field("admission")?.choice(&["add", "replace-primary"])?;
        let common = QvmModItemDefinition {
            item: namespaced_id(&value.field("item")?)?,
            label: value.field("label")?.string()?,
            icon,
            admission: if admission == "add" {
                QvmItemAdmission::Add
            } else {
                QvmItemAdmission::ReplacePrimary
            },
            actions,
            kind: QvmModItemKind::Counter,
        };
        if value.field("kind")?.choice(&["counter", "weapon"])? == "counter" {
            return Ok(common);
        }
        let held = if value.field("held")?.is_undefined() {
            None
        } else {
            Some(read_held(&value.field("held")?)?)
        };
        Ok(QvmModItemDefinition {
            kind: QvmModItemKind::Weapon {
                ammo: value.field("ammo")?.nullable(|field| namespaced_id(field))?,
                held,
            },
            ..common
        })
    })?;
    let storage = reader.field("storage")?.list(parse_qvm_item_storage)?;
    let weapons_reader = reader.field("weapons")?;
    if weapons_reader.is_undefined() {
        return Ok(QvmModItems {
            definitions,
            storage,
            weapons: None,
        });
    }
    let stage = weapons_reader.field("stage")?;
    let dispatcher = stage.field("dispatcher")?;
    let selection = stage.field("selection")?;
    let request = stage.field("request")?;
    let continuation = stage.field("continuation")?;
    let projection = continuation.field("projection")?;
    let predicate = |value: &ProfileReader<'_>| -> Result<QvmWeaponPredicate, GuestError> {
        Ok(QvmWeaponPredicate {
            instruction: value.field("instruction")?.integer(0)? as usize,
            unselected: value.field("unselected")?.boolean()?,
        })
    };
    Ok(QvmModItems {
        definitions,
        storage,
        weapons: Some(QvmModItemWeapons {
            input_entry: weapons_reader.field("input")?.field("entry")?.integer(0)? as usize,
            input_clock: parse_field(&weapons_reader.field("input")?.field("clock")?)?,
            stage: QvmWeaponStage {
                dispatcher: QvmWeaponDispatcher {
                    entry: dispatcher.field("entry")?.integer(0)? as usize,
                    actor: parse_actor(&dispatcher.field("actor")?)?,
                },
                predicates: stage.field("predicates")?.list(predicate)?,
                settled: stage.field("settled")?.list(parse_test)?,
                selection: QvmWeaponSelection {
                    field: parse_field(&selection.field("field")?)?,
                    values: selection.field("values")?.list(|value| {
                        Ok(QvmSelectionValue {
                            value: value.field("value")?.integer(i64::MIN)?,
                            item: namespaced_id(&value.field("item")?)?,
                        })
                    })?,
                },
                request: QvmWeaponRequest {
                    entry: request.field("entry")?.integer(0)? as usize,
                    argument: request.field("argument")?.integer(0)? as usize,
                    accepted: request.field("accepted")?.list(parse_test)?,
                },
                continuation: QvmWeaponContinuation {
                    entry: continuation.field("entry")?.integer(0)? as usize,
                    actor: parse_actor(&continuation.field("actor")?)?,
                    instruction: continuation.field("instruction")?.integer(0)? as usize,
                    original_taken: continuation.field("originalTaken")?.boolean()?,
                    when: continuation.field("when")?.list(parse_test)?,
                    predicates: continuation.field("predicates")?.list(predicate)?,
                    projection: QvmProjection {
                        movement: parse_pointer(&projection.field("movement")?)?,
                        byte_length: projection.field("byteLength")?.integer(1)? as usize,
                        minimum: projection.field("minimum")?.integer(0)?,
                        maximum: projection.field("maximum")?.integer(0)?,
                        view_height: parse_field(&projection.field("viewHeight")?)?,
                        ground: parse_field(&projection.field("ground")?)?,
                    },
                    calls: continuation.field("calls")?.list(|value| {
                        Ok(QvmStageCall {
                            instruction: value.field("instruction")?.integer(0)? as usize,
                            call: read_call(&value.field("call")?)?,
                        })
                    })?,
                },
            },
        }),
    })
}

/// Field usage checked by [`validate_qvm_item_storage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmItemFieldUsage {
    /// Counter/bits word.
    Storage,
    /// Capacity word.
    Capacity,
}

fn capacity_constant(image: &QvmImage, instruction: usize) -> Result<i32, GuestError> {
    let value = image.instruction(instruction);
    if value.map_or(true, |instruction| {
        instruction.opcode != QvmOpcode::OpConst || instruction.operand < 0
    }) {
        return Err(GuestError::invalid(
            "QVM item capacity is not its declared original constant",
        ));
    }
    Ok(value.expect("checked capacity constant").operand)
}

/// Check storage bindings against declared items, image, and fields.
pub fn validate_qvm_item_storage(
    storage_values: &[QvmItemStorage],
    items: &HashSet<ItemId>,
    image: &QvmImage,
    field: &dyn Fn(&QvmItemField, QvmItemFieldUsage) -> Result<(), GuestError>,
) -> Result<(), GuestError> {
    let mut bound = HashSet::new();
    let mut bind = |item: &ItemId| -> Result<(), GuestError> {
        if !items.contains(item) || !bound.insert(item.clone()) {
            return Err(GuestError::invalid("QVM item lacks distinct declared storage"));
        }
        Ok(())
    };
    for storage in storage_values {
        match storage {
            QvmItemStorage::Counter {
                field: counter,
                item,
                capacity,
            } => {
                field(counter, QvmItemFieldUsage::Storage)?;
                bind(item)?;
                match capacity {
                    QvmItemCapacity::Field { field: capacity_field } => {
                        field(capacity_field, QvmItemFieldUsage::Capacity)?;
                    }
                    QvmItemCapacity::Constant { value } => {
                        if *value < 0 {
                            return Err(GuestError::invalid("QVM item capacity exceeds its source ABI"));
                        }
                    }
                    QvmItemCapacity::Source { instruction, overrides } => {
                        capacity_constant(image, *instruction)?;
                        for value in overrides {
                            capacity_constant(image, value.instruction)?;
                            if value.address % 4 != 0
                                || value.address + 4 > image.initialized_data.len() + image.bss_length
                            {
                                return Err(GuestError::invalid(
                                    "QVM capacity selector exceeds original source storage",
                                ));
                            }
                        }
                    }
                }
            }
            QvmItemStorage::Bits {
                field: bits,
                private_mask,
                items: packed,
            } => {
                field(bits, QvmItemFieldUsage::Storage)?;
                if packed.is_empty() {
                    return Err(GuestError::invalid("Invalid QVM private inventory mask"));
                }
                let mut mask = *private_mask;
                for value in packed {
                    bind(&value.item)?;
                    if value.mask < 1
                        || value.mask > 0x8000_0000
                        || value.mask & (value.mask - 1) != 0
                        || mask & value.mask != 0
                    {
                        return Err(GuestError::invalid("QVM packed item masks overlap"));
                    }
                    mask |= value.mask;
                }
            }
        }
    }
    if bound.len() != items.len() {
        return Err(GuestError::invalid("QVM item definition has no source storage"));
    }
    Ok(())
}

/// Storage field access.
pub trait QvmItemStorageAccess {
    /// Read a record field.
    fn read(&self, field: &QvmItemField) -> Result<i32, GuestError>;
    /// Read a live global.
    fn global(&self, address: usize) -> Result<i32, GuestError>;
    /// Write a record field.
    fn write(&self, field: &QvmItemField, value: i32) -> Result<(), GuestError>;
}

/// Resolve a capacity against the image and live access.
pub fn qvm_item_capacity(
    image: &QvmImage,
    capacity: &QvmItemCapacity,
    access: &dyn QvmItemStorageAccess,
) -> Result<i32, GuestError> {
    match capacity {
        QvmItemCapacity::Constant { value } => Ok(*value),
        QvmItemCapacity::Field { field } => access.read(field),
        QvmItemCapacity::Source { instruction, overrides } => {
            for value in overrides {
                let matches = i64::from(access.global(value.address)?) == value.value;
                if (value.comparison == QvmCapacityComparison::Equals) == matches {
                    return capacity_constant(image, value.instruction);
                }
            }
            capacity_constant(image, *instruction)
        }
    }
}

/// Read storage entries.
pub fn read_qvm_item_storage(
    image: &QvmImage,
    storage: &QvmItemStorage,
    access: &dyn QvmItemStorageAccess,
) -> Result<Vec<InventoryEntry>, GuestError> {
    let field = match storage {
        QvmItemStorage::Counter { field, .. } | QvmItemStorage::Bits { field, .. } => field,
    };
    let count = access.read(field)?;
    match storage {
        QvmItemStorage::Counter { item, capacity, .. } => Ok(vec![InventoryEntry {
            item: item.clone(),
            count: f64::from(count),
            capacity: f64::from(qvm_item_capacity(image, capacity, access)?),
            count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
        }]),
        QvmItemStorage::Bits {
            private_mask, items, ..
        } => {
            let mask = items.iter().fold(*private_mask, |mask, value| mask | value.mask);
            if (count as u32) & !mask != 0 {
                return Err(GuestError::invalid("Original QVM inventory contains undeclared bits"));
            }
            Ok(items
                .iter()
                .map(|value| InventoryEntry {
                    item: value.item.clone(),
                    count: f64::from(i32::from((count as u32) & value.mask != 0)),
                    capacity: 1.0,
                    count_policy: None,
                })
                .collect())
        }
    }
}

/// Write one storage entry.
pub fn write_qvm_item_storage(
    image: &QvmImage,
    storage: &QvmItemStorage,
    entry: &InventoryEntry,
    access: &dyn QvmItemStorageAccess,
) -> Result<(), GuestError> {
    match storage {
        QvmItemStorage::Counter { field, capacity, .. } => {
            if entry.count.fract() != 0.0
                || entry.count < f64::from(i32::MIN)
                || entry.count > f64::from(i32::MAX)
                || entry.capacity.fract() != 0.0
                || entry.capacity < 0.0
                || entry.capacity > f64::from(i32::MAX)
            {
                return Err(GuestError::invalid("QVM item exceeds its signed source representation"));
            }
            if !matches!(capacity, QvmItemCapacity::Field { .. })
                && entry.capacity != f64::from(qvm_item_capacity(image, capacity, access)?)
            {
                return Err(GuestError::invalid("QVM capacity is owned by its original source"));
            }
            access.write(field, entry.count as i32)?;
            if let QvmItemCapacity::Field { field } = capacity {
                access.write(field, entry.capacity as i32)?;
            }
            Ok(())
        }
        QvmItemStorage::Bits { field, items, .. } => {
            let bit = items.iter().find(|value| value.item == entry.item);
            let Some(bit) = bit else {
                return Err(GuestError::invalid("QVM packed ownership requires one admitted bit"));
            };
            if entry.capacity != 1.0 || (entry.count != 0.0 && entry.count != 1.0) {
                return Err(GuestError::invalid("QVM packed ownership requires one admitted bit"));
            }
            let previous = access.read(field)? as u32;
            access.write(
                field,
                (if entry.count == 0.0 {
                    previous & !bit.mask
                } else {
                    previous | bit.mask
                }) as i32,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::super::game_data::{ProfileValue, QvmInstruction};
    use super::*;

    struct FixtureAccess {
        fields: RefCell<HashMap<(String, usize), i32>>,
        globals: RefCell<HashMap<usize, i32>>,
    }

    impl QvmItemStorageAccess for FixtureAccess {
        fn read(&self, field: &QvmItemField) -> Result<i32, GuestError> {
            Ok(*self
                .fields
                .borrow()
                .get(&(field.record.clone(), field.offset))
                .unwrap_or(&0))
        }

        fn global(&self, address: usize) -> Result<i32, GuestError> {
            Ok(*self.globals.borrow().get(&address).unwrap_or(&0))
        }

        fn write(&self, field: &QvmItemField, value: i32) -> Result<(), GuestError> {
            self.fields
                .borrow_mut()
                .insert((field.record.clone(), field.offset), value);
            Ok(())
        }
    }

    fn image() -> QvmImage {
        let mut image = QvmImage::default();
        image.instructions = vec![
            QvmInstruction::word(QvmOpcode::OpConst, 200, 0),
            QvmInstruction::word(QvmOpcode::OpConst, 50, 5),
        ];
        image.initialized_data = vec![0u8; 64];
        image
    }

    #[test]
    fn storage_parser_reads_counters_and_bits() {
        let counter = ProfileValue::record(vec![
            (
                "field",
                ProfileValue::record(vec![
                    ("record", ProfileValue::Str("player".to_string())),
                    ("offset", ProfileValue::Int(8)),
                ]),
            ),
            ("kind", ProfileValue::Str("counter".to_string())),
            ("item", ProfileValue::Str("q3:rockets".to_string())),
            (
                "capacity",
                ProfileValue::record(vec![
                    ("kind", ProfileValue::Str("constant".to_string())),
                    ("value", ProfileValue::Int(200)),
                ]),
            ),
        ]);
        let parsed = parse_qvm_item_storage(&ProfileReader::new(&counter)).unwrap();
        assert!(matches!(parsed, QvmItemStorage::Counter { .. }));
        let bits = ProfileValue::record(vec![
            (
                "field",
                ProfileValue::record(vec![
                    ("record", ProfileValue::Str("player".to_string())),
                    ("offset", ProfileValue::Int(12)),
                ]),
            ),
            ("kind", ProfileValue::Str("bits".to_string())),
            ("privateMask", ProfileValue::Int(0)),
            (
                "items",
                ProfileValue::Array(vec![ProfileValue::record(vec![
                    ("item", ProfileValue::Str("q3:shotgun".to_string())),
                    ("mask", ProfileValue::Int(4)),
                ])]),
            ),
        ]);
        let parsed = parse_qvm_item_storage(&ProfileReader::new(&bits)).unwrap();
        assert!(matches!(parsed, QvmItemStorage::Bits { .. }));
    }

    #[test]
    fn validation_binds_distinct_storage_and_masks() {
        let items: HashSet<ItemId> = ["q3:rockets".to_string(), "q3:shotgun".to_string()]
            .into_iter()
            .collect();
        let storage = vec![
            QvmItemStorage::Counter {
                field: QvmItemField {
                    record: "player".to_string(),
                    offset: 8,
                },
                item: "q3:rockets".to_string(),
                capacity: QvmItemCapacity::Source {
                    instruction: 0,
                    overrides: vec![QvmCapacityOverride {
                        address: 16,
                        comparison: QvmCapacityComparison::Equals,
                        value: 1,
                        instruction: 1,
                    }],
                },
            },
            QvmItemStorage::Bits {
                field: QvmItemField {
                    record: "player".to_string(),
                    offset: 12,
                },
                private_mask: 1,
                items: vec![QvmPackedItem {
                    item: "q3:shotgun".to_string(),
                    mask: 4,
                }],
            },
        ];
        let check = |_: &QvmItemField, _: QvmItemFieldUsage| Ok(());
        assert!(validate_qvm_item_storage(&storage, &items, &image(), &check).is_ok());
        let overlapping = vec![QvmItemStorage::Bits {
            field: QvmItemField {
                record: "player".to_string(),
                offset: 12,
            },
            private_mask: 4,
            items: vec![QvmPackedItem {
                item: "q3:shotgun".to_string(),
                mask: 4,
            }],
        }];
        assert!(validate_qvm_item_storage(&overlapping, &items, &image(), &check).is_err());
        assert!(validate_qvm_item_storage(&storage[..1], &items, &image(), &check).is_err());
    }

    #[test]
    fn capacity_selectors_and_counters_round_trip() {
        let access = FixtureAccess {
            fields: RefCell::new(HashMap::new()),
            globals: RefCell::new([(16usize, 1i32)].into_iter().collect()),
        };
        let image = image();
        let capacity = QvmItemCapacity::Source {
            instruction: 0,
            overrides: vec![QvmCapacityOverride {
                address: 16,
                comparison: QvmCapacityComparison::Equals,
                value: 1,
                instruction: 1,
            }],
        };
        assert_eq!(qvm_item_capacity(&image, &capacity, &access).unwrap(), 50);
        access.globals.borrow_mut().insert(16, 0);
        assert_eq!(qvm_item_capacity(&image, &capacity, &access).unwrap(), 200);

        let storage = QvmItemStorage::Counter {
            field: QvmItemField {
                record: "player".to_string(),
                offset: 8,
            },
            item: "q3:rockets".to_string(),
            capacity: QvmItemCapacity::Constant { value: 200 },
        };
        access.fields.borrow_mut().insert(("player".to_string(), 8), 17);
        let entries = read_qvm_item_storage(&image, &storage, &access).unwrap();
        assert_eq!(entries[0].count, 17.0);
        assert_eq!(
            entries[0].count_policy,
            Some(CountPolicy::SourceCounter(CountArithmetic::Int32))
        );
        write_qvm_item_storage(
            &image,
            &storage,
            &InventoryEntry {
                item: "q3:rockets".to_string(),
                count: 19.0,
                capacity: 200.0,
                count_policy: None,
            },
            &access,
        )
        .unwrap();
        assert_eq!(access.fields.borrow().get(&("player".to_string(), 8)), Some(&19));
        let foreign = InventoryEntry {
            item: "q3:rockets".to_string(),
            count: 19.0,
            capacity: 100.0,
            count_policy: None,
        };
        assert!(write_qvm_item_storage(&image, &storage, &foreign, &access).is_err());
    }

    #[test]
    fn packed_bits_reject_undeclared_bits_and_foreign_items() {
        let access = FixtureAccess {
            fields: RefCell::new(HashMap::from([(("player".to_string(), 12usize), 5i32)])),
            globals: RefCell::new(HashMap::new()),
        };
        let storage = QvmItemStorage::Bits {
            field: QvmItemField {
                record: "player".to_string(),
                offset: 12,
            },
            private_mask: 1,
            items: vec![QvmPackedItem {
                item: "q3:shotgun".to_string(),
                mask: 4,
            }],
        };
        let entries = read_qvm_item_storage(&image(), &storage, &access).unwrap();
        assert_eq!(entries[0].count, 1.0);
        access.fields.borrow_mut().insert(("player".to_string(), 12), 8);
        assert!(read_qvm_item_storage(&image(), &storage, &access).is_err());
        access.fields.borrow_mut().insert(("player".to_string(), 12), 1);
        write_qvm_item_storage(
            &image(),
            &storage,
            &InventoryEntry {
                item: "q3:shotgun".to_string(),
                count: 1.0,
                capacity: 1.0,
                count_policy: None,
            },
            &access,
        )
        .unwrap();
        assert_eq!(access.fields.borrow().get(&("player".to_string(), 12)), Some(&5));
    }
}
