//! Port of `src/compat/q2/native-mod-armor.ts`.
//! Bridges source armor words: decodes declared selection/points/cells storage
//! into canonical armor while the original damage body owns absorption.

use std::collections::HashMap;

use qa_guest::abi::values::{decode_value, encode_value};
use qa_guest::core::contracts::{GuestAddress, GuestCallValue, GuestStorage, GuestValueLayout};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::error::GuestError;
use qa_world::combat::{ArmorState, PoweredProtection, RegularArmor};
use thiserror::Error;

/// Failures decoding or storing native armor.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ArmorError {
    /// A field names no declared entity or public client record.
    #[error("native armor field requires a declared entity or public client record")]
    BadRecord,
    /// Overlapping fields disagree on storage.
    #[error("overlapping native armor fields have incompatible storage")]
    OverlappingFields,
    /// Two items share one selection identity.
    #[error("native armor selection is ambiguous")]
    AmbiguousSelection,
    /// An enum selection equals its inactive value or is inexact.
    #[error("native armor selection is invalid")]
    BadSelection,
    /// A Q2 protection fraction is not finite.
    #[error("native armor protection must be finite")]
    BadProtection,
    /// An enabled mask is not a positive integer.
    #[error("native power armor requires an integer enabled mask")]
    BadEnabledMask,
    /// A value exceeds its declared storage.
    #[error("native armor value exceeds its declared storage")]
    ValueRange,
    /// The selected armor has no source storage.
    #[error("selected native armor has no source storage")]
    MissingStorage,
    /// A write needs a record the slot does not project.
    #[error("native armor write requires its source record")]
    MissingWriteRecord,
    /// A write crosses armor families or representations.
    #[error("native armor declaration cannot represent this armor state")]
    Unrepresentable,
    /// Source selection cannot represent the armor without wider changes.
    #[error("native source selection cannot represent this armor without changing other ownership")]
    SelectionConflict,
    /// Underlying guest failure.
    #[error("native armor guest failure: {0}")]
    Guest(String),
}

impl From<GuestError> for ArmorError {
    fn from(error: GuestError) -> Self {
        Self::Guest(error.to_string())
    }
}

/// One scalar armor field inside a record.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArmorField {
    /// Owning record id.
    pub record: String,
    /// Byte offset inside the record.
    pub offset: usize,
    /// Lane storage.
    pub storage: GuestStorage,
}

impl ArmorField {
    fn width(&self, pointer_bytes: usize) -> usize {
        self.storage.byte_length(pointer_bytes)
    }
}

/// Declared record layout for validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmorRecordLayout {
    /// Record id.
    pub id: String,
    /// Record stride in bytes.
    pub stride: usize,
    /// Whether this is a public client record.
    pub is_client: bool,
}

/// How an armor item is selected in source storage.
#[derive(Debug, Clone, PartialEq)]
pub enum ArmorSelection {
    /// Selected while the field is positive.
    Positive {
        /// Selection field.
        field: ArmorField,
    },
    /// Selected when the field holds exactly `value` (`none` otherwise).
    Enum {
        /// Selection field.
        field: ArmorField,
        /// Active value.
        value: f64,
        /// Inactive value.
        none: f64,
    },
}

impl ArmorSelection {
    fn field(&self) -> &ArmorField {
        match self {
            Self::Positive { field } | Self::Enum { field, .. } => field,
        }
    }

    fn is_selected(&self, value: Option<f64>) -> bool {
        let Some(value) = value else { return false };
        match self {
            Self::Positive { .. } => value > 0.0,
            Self::Enum { value: active, .. } => value == *active,
        }
    }
}

/// Regular armor item declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularArmorItem {
    /// Canonical item id.
    pub item: String,
    /// Selection rule.
    pub selection: ArmorSelection,
    /// Points field.
    pub points: ArmorField,
    /// Q2 normal protection (Q2 family only).
    pub normal_protection: f64,
    /// Q2 energy protection (Q2 family only).
    pub energy_protection: f64,
}

/// Powered protection kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PowerKind {
    /// Screen.
    Screen,
    /// Shield.
    Shield,
}

/// Power armor item declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerArmorItem {
    /// Protection kind.
    pub kind: PowerKind,
    /// Canonical item id (legacy placeholder).
    pub item: String,
    /// Selection rule.
    pub selection: ArmorSelection,
    /// Cells field.
    pub cells: ArmorField,
    /// Optional enabled bitmask.
    pub enabled: Option<EnabledMask>,
}

/// Enabled bitmask for power armor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnabledMask {
    /// Mask field.
    pub field: ArmorField,
    /// Bitmask.
    pub mask: i64,
}

/// Armor storage family declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum ArmorDefinition {
    /// No armor storage.
    None,
    /// Q2 armor with protection fractions.
    Q2 {
        /// Regular items.
        regular: Vec<RegularArmorItem>,
        /// Power items.
        power: Vec<PowerArmorItem>,
    },
    /// Source-family armor without protection fractions.
    Source {
        /// Regular items.
        regular: Vec<RegularArmorItem>,
        /// Power items.
        power: Vec<PowerArmorItem>,
    },
}

impl ArmorDefinition {
    fn regular(&self) -> &[RegularArmorItem] {
        match self {
            Self::None => &[],
            Self::Q2 { regular, .. } | Self::Source { regular, .. } => regular,
        }
    }

    fn power(&self) -> &[PowerArmorItem] {
        match self {
            Self::None => &[],
            Self::Q2 { power, .. } | Self::Source { power, .. } => power,
        }
    }
}

fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.trunc() == value && value.abs() <= 9_007_199_254_740_992.0
}

fn validate_value(field: &ArmorField, value: f64, pointer_bytes: usize) -> Result<(), ArmorError> {
    let bits = field.width(pointer_bytes) * 8;
    let unsigned = matches!(
        field.storage,
        GuestStorage::Uint8 | GuestStorage::Uint16 | GuestStorage::Uint32 | GuestStorage::Uint64
    );
    if !value.is_finite() {
        return Err(ArmorError::ValueRange);
    }
    if field.storage == GuestStorage::Float32 {
        if !(value as f32).is_finite() {
            return Err(ArmorError::ValueRange);
        }
        return Ok(());
    }
    if field.storage.is_float() {
        return Ok(());
    }
    if !is_safe_integer(value) {
        return Err(ArmorError::ValueRange);
    }
    let bound = 2f64.powi(bits as i32 - i32::from(!unsigned));
    let minimum = if unsigned { 0.0 } else { -bound };
    let maximum = 2f64.powi(bits as i32 - i32::from(!unsigned));
    if value < minimum || value >= maximum {
        return Err(ArmorError::ValueRange);
    }
    Ok(())
}

/// Decodes source armor storage; absorption stays with the damage body.
#[derive(Debug)]
pub struct NativeModArmorState {
    definition: ArmorDefinition,
    entity_record: String,
    fields: Vec<ArmorField>,
}

impl NativeModArmorState {
    /// Validate the declaration against record layouts.
    pub fn new(
        definition: ArmorDefinition,
        entity_record: &str,
        records: &[ArmorRecordLayout],
        pointer_bytes: usize,
    ) -> Result<Self, ArmorError> {
        let fields: Vec<ArmorField> = definition
            .regular()
            .iter()
            .flat_map(|item| [item.selection.field().clone(), item.points.clone()])
            .chain(definition.power().iter().flat_map(|item| {
                let mut fields = vec![item.selection.field().clone(), item.cells.clone()];
                if let Some(enabled) = &item.enabled {
                    fields.push(enabled.field.clone());
                }
                fields
            }))
            .collect();
        let layout = |id: &str| records.iter().find(|record| record.id == id);
        for field in &fields {
            let Some(record) = layout(&field.record) else {
                return Err(ArmorError::BadRecord);
            };
            if record.id != entity_record && !record.is_client
                || field.offset.saturating_add(field.width(pointer_bytes)) > record.stride
            {
                return Err(ArmorError::BadRecord);
            }
        }
        for (index, field) in fields.iter().enumerate() {
            for previous in &fields[..index] {
                if field.record == previous.record
                    && field.offset
                        < previous.offset + previous.width(pointer_bytes)
                    && previous.offset < field.offset + field.width(pointer_bytes)
                    && (field.offset != previous.offset || field.storage != previous.storage)
                {
                    return Err(ArmorError::OverlappingFields);
                }
            }
        }
        if matches!(definition, ArmorDefinition::None) {
            return Ok(Self {
                definition,
                entity_record: entity_record.to_string(),
                fields,
            });
        }
        let mut regular_ids = std::collections::HashSet::new();
        for item in definition.regular() {
            if !regular_ids.insert(item.item.clone()) {
                return Err(ArmorError::AmbiguousSelection);
            }
        }
        let mut power_kinds = std::collections::HashSet::new();
        for item in definition.power() {
            if !power_kinds.insert(item.kind) {
                return Err(ArmorError::AmbiguousSelection);
            }
        }
        for item in definition
            .regular()
            .iter()
            .map(|item| &item.selection)
            .chain(definition.power().iter().map(|item| &item.selection))
        {
            if let ArmorSelection::Enum { field, value, none } = item {
                validate_value(field, *value, pointer_bytes)?;
                validate_value(field, *none, pointer_bytes)?;
                if value == none {
                    return Err(ArmorError::BadSelection);
                }
                if field.storage == GuestStorage::Float32
                    && (f64::from(*value as f32) != *value || f64::from(*none as f32) != *none)
                {
                    return Err(ArmorError::BadSelection);
                }
            }
        }
        if let ArmorDefinition::Q2 { regular, .. } = &definition {
            for item in regular {
                if ![item.normal_protection, item.energy_protection]
                    .iter()
                    .all(|value| value.is_finite())
                {
                    return Err(ArmorError::BadProtection);
                }
            }
        }
        for item in definition.power() {
            if let Some(enabled) = &item.enabled {
                if enabled.field.storage.is_float() || enabled.mask <= 0 {
                    return Err(ArmorError::BadEnabledMask);
                }
                validate_value(&enabled.field, enabled.mask as f64, pointer_bytes)?;
            }
        }
        Ok(Self {
            definition,
            entity_record: entity_record.to_string(),
            fields,
        })
    }

    /// Resolve a slot base address for a record.
    pub fn location(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        field: &ArmorField,
    ) -> Result<Option<GuestAddress>, ArmorError> {
        let Some(base) = base_for(slot, &field.record) else {
            return Ok(None);
        };
        let address = memory.offset(base, field.offset as i64)?;
        memory.check(
            address,
            field.width(memory.pointer_bytes()),
            qa_guest::core::contracts::GuestAccess::Read,
        )?;
        Ok(Some(address))
    }

    fn read_scalar(
        &self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
        field: &ArmorField,
    ) -> Result<f64, ArmorError> {
        let width = field.width(memory.pointer_bytes());
        let bytes = memory.copy(address, width)?;
        let layout = GuestValueLayout::Scalar(field.storage);
        match decode_value(&layout, &bytes, memory)? {
            GuestCallValue::Int32(value) => Ok(f64::from(value)),
            GuestCallValue::Uint32(value) => Ok(f64::from(value)),
            GuestCallValue::Int64(value) => Ok(value as f64),
            GuestCallValue::Uint64(value) => Ok(value as f64),
            GuestCallValue::Float32(value) => Ok(f64::from(value)),
            GuestCallValue::Float64(value) => Ok(value),
            GuestCallValue::Pointer(_) => Err(ArmorError::ValueRange),
            GuestCallValue::Aggregate { .. } => Err(ArmorError::ValueRange),
        }
    }

    fn write_scalar(
        &self,
        memory: &mut SparseGuestMemory,
        address: GuestAddress,
        field: &ArmorField,
        value: f64,
    ) -> Result<(), ArmorError> {
        let stored = if field.storage == GuestStorage::Float32 {
            GuestCallValue::Float32(value as f32)
        } else if field.storage.is_float() {
            GuestCallValue::Float64(value)
        } else if matches!(
            field.storage,
            GuestStorage::Uint8
                | GuestStorage::Uint16
                | GuestStorage::Uint32
                | GuestStorage::Uint64
        ) {
            GuestCallValue::Uint64(value as u64)
        } else {
            GuestCallValue::Int64(value as i64)
        };
        let layout = GuestValueLayout::Scalar(field.storage);
        let bytes = encode_value(&layout, &stored, memory)?;
        memory.check(
            address,
            bytes.len(),
            qa_guest::core::contracts::GuestAccess::Write,
        )?;
        Ok(memory.write(address, &bytes)?)
    }

    fn value(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        field: &ArmorField,
    ) -> Result<Option<f64>, ArmorError> {
        let Some(address) = self.location(memory, base_for, slot, field)? else {
            return Ok(None);
        };
        Ok(Some(self.read_scalar(memory, address, field)?))
    }

    fn required(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        field: &ArmorField,
    ) -> Result<f64, ArmorError> {
        self.value(memory, base_for, slot, field)?
            .ok_or(ArmorError::MissingStorage)
    }

    /// Read the armor state for one slot.
    pub fn read(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
    ) -> Result<ArmorState, ArmorError> {
        let mut cache: HashMap<*const ArmorField, Option<f64>> = HashMap::new();
        let mut read = |field: &ArmorField| -> Result<Option<f64>, ArmorError> {
            let key = std::ptr::from_ref(field);
            if let Some(cached) = cache.get(&key) {
                return Ok(*cached);
            }
            let value = self.value(memory, base_for, slot, field)?;
            cache.insert(key, value);
            Ok(value)
        };
        self.capture(&mut read)
    }

    #[allow(clippy::type_complexity)]
    fn capture(
        &self,
        read: &mut dyn FnMut(&ArmorField) -> Result<Option<f64>, ArmorError>,
    ) -> Result<ArmorState, ArmorError> {
        if matches!(self.definition, ArmorDefinition::None) {
            return Ok(ArmorState {
                regular: RegularArmor::None,
                powered: PoweredProtection::None,
            });
        }
        let mut required = |field: &ArmorField| -> Result<f64, ArmorError> {
            read(field)?.ok_or(ArmorError::MissingStorage)
        };
        let regular = match &self.definition {
            ArmorDefinition::Q2 { regular, .. } => {
                let mut found = None;
                for item in regular {
                    if item.selection.is_selected(read(item.selection.field())?) {
                        found = Some(item);
                        break;
                    }
                }
                match found {
                    None => RegularArmor::None,
                    Some(item) => RegularArmor::Q2 {
                        points: required(&item.points)?,
                        normal_protection: item.normal_protection,
                        energy_protection: item.energy_protection,
                        item: item.item.clone(),
                    },
                }
            }
            ArmorDefinition::Source { regular, .. } => {
                let mut found = None;
                for item in regular {
                    if item.selection.is_selected(read(item.selection.field())?) {
                        found = Some(item);
                        break;
                    }
                }
                match found {
                    None => RegularArmor::None,
                    Some(item) => RegularArmor::Source {
                        points: required(&item.points)?,
                        item: Some(item.item.clone()),
                    },
                }
            }
            ArmorDefinition::None => RegularArmor::None,
        };
        let mut power_found = None;
        for item in self.definition.power() {
            let selected = item.selection.is_selected(read(item.selection.field())?);
            let enabled = match &item.enabled {
                None => true,
                Some(mask) => required(&mask.field)? as i64 & mask.mask != 0,
            };
            if selected && enabled {
                power_found = Some(item);
                break;
            }
        }
        let powered = match power_found {
            None => PoweredProtection::None,
            Some(item) => {
                let cells = required(&item.cells)? as i32;
                match item.kind {
                    PowerKind::Screen => PoweredProtection::Screen { cells },
                    PowerKind::Shield => PoweredProtection::Shield { cells },
                }
            }
        };
        Ok(ArmorState { regular, powered })
    }

    /// Read one count field.
    pub fn read_count(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        field: &ArmorField,
    ) -> Result<f64, ArmorError> {
        self.required(memory, base_for, slot, field)
    }

    /// Write one count field.
    pub fn write_count(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        field: &ArmorField,
        value: f64,
    ) -> Result<(), ArmorError> {
        validate_value(field, value, memory.pointer_bytes())?;
        let Some(address) = self.location(memory, base_for, slot, field)? else {
            return Err(ArmorError::MissingWriteRecord);
        };
        self.write_scalar(memory, address, field, value)
    }

    /// Normalize a legacy power-only armor view against current storage.
    pub fn normalize_legacy_armor(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        armor: ArmorState,
    ) -> Result<ArmorState, ArmorError> {
        let kind = match &armor.powered {
            PoweredProtection::Screen { .. } => PowerKind::Screen,
            PoweredProtection::Shield { .. } => PowerKind::Shield,
            PoweredProtection::None => return Ok(armor),
        };
        let Some(item) = self.definition.power().iter().find(|item| item.kind == kind) else {
            return Ok(armor);
        };
        let current = self.read(memory, base_for, slot)?;
        Ok(normalize_legacy_power_only_armor(&armor, &current, &item.item))
    }

    /// Validate a write without committing it.
    pub fn validate_write(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        armor: &ArmorState,
    ) -> Result<(), ArmorError> {
        self.stores(memory, base_for, slot, armor)?;
        Ok(())
    }

    /// Commit an armor write to source storage.
    pub fn write(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        armor: &ArmorState,
    ) -> Result<(), ArmorError> {
        for (address, field, value) in self.stores(memory, base_for, slot, armor)? {
            self.write_scalar(memory, address, &field, value)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn stores(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        armor: &ArmorState,
    ) -> Result<Vec<(GuestAddress, ArmorField, f64)>, ArmorError> {
        if matches!(self.definition, ArmorDefinition::None) {
            if !matches!(armor.regular, RegularArmor::None)
                || !matches!(armor.powered, PoweredProtection::None)
            {
                return Err(ArmorError::Unrepresentable);
            }
            return Ok(Vec::new());
        }
        let family = match &self.definition {
            ArmorDefinition::Q2 { .. } => "q2",
            ArmorDefinition::Source { .. } => "source",
            ArmorDefinition::None => unreachable!("handled above"),
        };
        match (&armor.regular, family) {
            (RegularArmor::None, _) => {}
            (RegularArmor::Q2 { .. }, "q2") | (RegularArmor::Source { .. }, "source") => {}
            _ => return Err(ArmorError::Unrepresentable),
        }
        let requested_item = match &armor.regular {
            RegularArmor::Q2 { item, .. } => Some(item),
            RegularArmor::Source { item, .. } => item.as_ref(),
            _ => None,
        };
        let regular = requested_item.and_then(|item| {
            self.definition
                .regular()
                .iter()
                .find(|candidate| &candidate.item == item)
        });
        if requested_item.is_some() && regular.is_none() {
            return Err(ArmorError::Unrepresentable);
        }
        if let (RegularArmor::Q2 { normal_protection, energy_protection, .. }, Some(declared)) =
            (&armor.regular, regular)
        {
            if declared.normal_protection != *normal_protection
                || declared.energy_protection != *energy_protection
            {
                return Err(ArmorError::Unrepresentable);
            }
        }
        let requested_power = match &armor.powered {
            PoweredProtection::None => None,
            PoweredProtection::Screen { .. } => Some(PowerKind::Screen),
            PoweredProtection::Shield { .. } => Some(PowerKind::Shield),
        };
        let power = requested_power.and_then(|kind| {
            self.definition
                .power()
                .iter()
                .find(|candidate| candidate.kind == kind)
        });
        if requested_power.is_some() && power.is_none() {
            return Err(ArmorError::Unrepresentable);
        }

        let mut stores: HashMap<(u64, usize), (GuestAddress, ArmorField, f64)> = HashMap::new();

        // Deselect current regular armor when clearing.
        if matches!(armor.regular, RegularArmor::None) {
            let mut current = None;
            for item in self.definition.regular() {
                let value = Self::pending_value(
                    memory,
                    base_for,
                    self,
                    slot,
                    &stores,
                    item.selection.field(),
                )?;
                if item.selection.is_selected(value) {
                    current = Some(item);
                    break;
                }
            }
            if let Some(item) = current {
                Self::select_store(
                    memory, base_for, self, slot, &mut stores, &item.selection, false,
                )?;
                Self::field_store(
                    memory, base_for, self, slot, &mut stores, &item.points, 0.0,
                )?;
            }
        }
        let current_power = self.read(memory, base_for, slot)?.powered;
        let current_power_kind = match &current_power {
            PoweredProtection::None => None,
            PoweredProtection::Screen { .. } => Some(PowerKind::Screen),
            PoweredProtection::Shield { .. } => Some(PowerKind::Shield),
        };
        if current_power_kind != requested_power {
            for item in self.definition.power() {
                Self::enable_store(
                    memory, base_for, self, slot, &mut stores, item, false,
                )?;
            }
        }
        if let (Some(item), RegularArmor::Q2 { points, .. } | RegularArmor::Source { points, .. }) =
            (regular, &armor.regular)
        {
            Self::select_store(memory, base_for, self, slot, &mut stores, &item.selection, true)?;
            Self::field_store(memory, base_for, self, slot, &mut stores, &item.points, *points)?;
        }
        if let Some(item) = power {
            let cells = match &armor.powered {
                PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells } => {
                    f64::from(*cells)
                }
                PoweredProtection::None => 0.0,
            };
            if current_power_kind != requested_power {
                Self::enable_store(memory, base_for, self, slot, &mut stores, item, true)?;
            }
            let current_cells = match &current_power {
                PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells } => {
                    f64::from(*cells)
                }
                PoweredProtection::None => 0.0,
            };
            if current_power_kind != requested_power || current_cells != cells {
                Self::field_store(memory, base_for, self, slot, &mut stores, &item.cells, cells)?;
            }
        }

        let mut capture = |field: &ArmorField| -> Result<Option<f64>, ArmorError> {
            Self::pending_value(memory, base_for, self, slot, &stores, field)
        };
        let result = self.capture(&mut capture)?;
        let requested_points = match (&armor.regular, regular) {
            (RegularArmor::Q2 { points, .. } | RegularArmor::Source { points, .. }, Some(item)) => {
                if item.points.storage == GuestStorage::Float32 {
                    f64::from(*points as f32)
                } else {
                    *points
                }
            }
            _ => 0.0,
        };
        let requested_cells = match (&armor.powered, power) {
            (
                PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells },
                Some(item),
            ) => {
                let cells = f64::from(*cells);
                if item.cells.storage == GuestStorage::Float32 {
                    f64::from(cells as f32)
                } else {
                    cells
                }
            }
            _ => 0.0,
        };
        let actual_points = match &result.regular {
            RegularArmor::Q2 { points, .. } | RegularArmor::Source { points, .. } => *points,
            _ => 0.0,
        };
        let actual_cells = match &result.powered {
            PoweredProtection::Screen { cells } | PoweredProtection::Shield { cells } => {
                f64::from(*cells)
            }
            PoweredProtection::None => 0.0,
        };
        let empty_regular = matches!(armor.regular, RegularArmor::None)
            || requested_points == 0.0
                && regular.is_some_and(|item| {
                    matches!(item.selection, ArmorSelection::Positive { .. })
                        && item.selection.field().record == item.points.record
                        && item.selection.field().offset == item.points.offset
                });
        let power_matches = match (&armor.powered, &result.powered) {
            (PoweredProtection::None, PoweredProtection::None) => true,
            (PoweredProtection::Screen { .. }, PoweredProtection::Screen { .. })
            | (PoweredProtection::Shield { .. }, PoweredProtection::Shield { .. }) => true,
            _ => false,
        };
        let regular_item_matches = match (&armor.regular, &result.regular) {
            (RegularArmor::None, RegularArmor::None) => true,
            (
                RegularArmor::Q2 { item: left, .. } | RegularArmor::Source { item: Some(left), .. },
                RegularArmor::Q2 { item: right, .. } | RegularArmor::Source { item: Some(right), .. },
            ) => left == right,
            _ => false,
        };
        if !power_matches
            || requested_points != actual_points
            || requested_cells != actual_cells
            || (empty_regular && !matches!(result.regular, RegularArmor::None))
            || (!empty_regular && !regular_item_matches)
        {
            return Err(ArmorError::SelectionConflict);
        }
        Ok(stores.into_values().collect())
    }

    #[allow(clippy::too_many_arguments)]
    fn pending_value(
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        this: &Self,
        slot: usize,
        stores: &HashMap<(u64, usize), (GuestAddress, ArmorField, f64)>,
        field: &ArmorField,
    ) -> Result<Option<f64>, ArmorError> {
        let Some(base) = base_for(slot, &field.record) else {
            return Ok(None);
        };
        if let Some((_, _, value)) = stores.get(&(base.offset, field.offset)) {
            return Ok(Some(*value));
        }
        this.value(memory, base_for, slot, field)
    }

    #[allow(clippy::too_many_arguments)]
    fn field_store(
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        this: &Self,
        slot: usize,
        stores: &mut HashMap<(u64, usize), (GuestAddress, ArmorField, f64)>,
        field: &ArmorField,
        value: f64,
    ) -> Result<(), ArmorError> {
        validate_value(field, value, memory.pointer_bytes())?;
        let Some(address) = this.location(memory, base_for, slot, field)? else {
            if value != 0.0 {
                return Err(ArmorError::MissingWriteRecord);
            }
            return Ok(());
        };
        let stored = if field.storage == GuestStorage::Float32 {
            f64::from(value as f32)
        } else {
            value
        };
        let base = base_for(slot, &field.record).ok_or(ArmorError::MissingWriteRecord)?;
        stores.insert((base.offset, field.offset), (address, field.clone(), stored));
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn select_store(
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        this: &Self,
        slot: usize,
        stores: &mut HashMap<(u64, usize), (GuestAddress, ArmorField, f64)>,
        selection: &ArmorSelection,
        active: bool,
    ) -> Result<(), ArmorError> {
        let value = match selection {
            ArmorSelection::Enum { value, none, .. } => {
                if active {
                    *value
                } else {
                    *none
                }
            }
            ArmorSelection::Positive { field } => {
                if active {
                    this.required(memory, base_for, slot, field)?.max(1.0)
                } else {
                    0.0
                }
            }
        };
        Self::field_store(memory, base_for, this, slot, stores, selection.field(), value)
    }

    #[allow(clippy::too_many_arguments)]
    fn enable_store(
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        this: &Self,
        slot: usize,
        stores: &mut HashMap<(u64, usize), (GuestAddress, ArmorField, f64)>,
        item: &PowerArmorItem,
        active: bool,
    ) -> Result<(), ArmorError> {
        let Some(mask) = &item.enabled else {
            return Self::select_store(
                memory,
                base_for,
                this,
                slot,
                stores,
                &item.selection,
                active,
            );
        };
        let base = base_for(slot, &mask.field.record);
        let current = match base {
            None => None,
            Some(base) => match stores.get(&(base.offset, mask.field.offset)) {
                Some((_, _, value)) => Some(*value),
                None => this.value(memory, base_for, slot, &mask.field)?,
            },
        };
        let Some(current) = current else {
            if active {
                return Err(ArmorError::MissingWriteRecord);
            }
            return Ok(());
        };
        let mask_bits = u64::from_ne_bytes(mask.mask.to_ne_bytes());
        let current_bits = u64::from_ne_bytes((current as i64).to_ne_bytes());
        let next = if active {
            current_bits | mask_bits
        } else {
            current_bits & !mask_bits
        };
        Self::field_store(
            memory,
            base_for,
            this,
            slot,
            stores,
            &mask.field,
            i64::from_ne_bytes(next.to_ne_bytes()) as f64,
        )?;
        if active {
            Self::select_store(memory, base_for, this, slot, stores, &item.selection, true)?;
        }
        Ok(())
    }

    /// Watch armor words; `poll` publishes committed changes exactly once.
    pub fn observe(
        &self,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
        changed: impl Fn(ArmorState, ArmorState) + 'static,
    ) -> Result<ArmorWatch, ArmorError> {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut groups: HashMap<u64, (GuestAddress, Vec<ArmorField>)> = HashMap::new();
        for field in &self.fields {
            let Some(base) = base_for(slot, &field.record) else {
                continue;
            };
            groups
                .entry(base.offset)
                .or_insert_with(|| (base, Vec::new()))
                .1
                .push(field.clone());
        }
        let previous = self.read(memory, base_for, slot)?;
        let dirty = Rc::new(RefCell::new(false));
        let mut ids = Vec::new();
        for (base, fields) in groups.values() {
            let pointer_bytes = memory.pointer_bytes();
            let start = fields.iter().map(|field| field.offset).min().unwrap_or(0);
            let end = fields
                .iter()
                .map(|field| field.offset + field.width(pointer_bytes))
                .max()
                .unwrap_or(0);
            let address = memory.offset(*base, start as i64)?;
            let watched = fields.clone();
            let flag = dirty.clone();
            let id = memory.observe_writes(
                address,
                end.saturating_sub(start),
                Box::new(move |ranges| {
                    for range in ranges {
                        let absolute = start + range.byte_offset;
                        if watched.iter().any(|field| {
                            let width = field.width(4);
                            absolute < field.offset + width
                                && field.offset < absolute + range.byte_length
                        }) {
                            *flag.borrow_mut() = true;
                            break;
                        }
                    }
                }),
            )?;
            ids.push(id);
        }
        Ok(ArmorWatch {
            ids,
            previous,
            dirty,
            changed: Rc::new(changed),
        })
    }
}

/// Pending armor watch; `poll` re-reads and publishes on change.
pub struct ArmorWatch {
    ids: Vec<u64>,
    previous: ArmorState,
    dirty: Rc<std::cell::RefCell<bool>>,
    changed: Rc<dyn Fn(ArmorState, ArmorState)>,
}

impl ArmorWatch {
    /// Re-read and publish if watched words changed; releases on `release`.
    pub fn poll(
        &mut self,
        armor: &NativeModArmorState,
        memory: &mut SparseGuestMemory,
        base_for: &dyn Fn(usize, &str) -> Option<GuestAddress>,
        slot: usize,
    ) -> Result<bool, ArmorError> {
        if !self.dirty.replace(false) {
            return Ok(false);
        }
        let before = self.previous.clone();
        let after = armor.read(memory, base_for, slot)?;
        self.previous = after.clone();
        if before != after {
            (self.changed)(before, after);
            return Ok(true);
        }
        Ok(false)
    }

    /// Remove the underlying write observers.
    pub fn release(self, memory: &mut SparseGuestMemory) {
        for id in self.ids {
            memory.unobserve(id);
        }
    }
}

/// Old power-only views fabricated regular armor using this source-owned item.
pub fn normalize_legacy_power_only_armor(
    legacy: &ArmorState,
    current: &ArmorState,
    placeholder: &str,
) -> ArmorState {
    let regular = &legacy.regular;
    let matches = matches!(current.regular, RegularArmor::None)
        && matches!(
            regular,
            RegularArmor::Q2 { points, normal_protection, energy_protection, item }
            if item == placeholder && *points == 0.0 && *normal_protection == 0.0 && *energy_protection == 0.0
        )
        && match (&legacy.powered, &current.powered) {
            (
                PoweredProtection::Screen { cells: left }
                | PoweredProtection::Shield { cells: left },
                PoweredProtection::Screen { cells: right }
                | PoweredProtection::Shield { cells: right },
            ) => {
                std::mem::discriminant(&legacy.powered) == std::mem::discriminant(&current.powered)
                    && left == right
            }
            _ => false,
        };
    if matches {
        ArmorState {
            regular: RegularArmor::None,
            powered: legacy.powered.clone(),
        }
    } else {
        legacy.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestMapOptions, GuestPermissions, ModuleIdentity};
    use std::cell::RefCell;
    use std::rc::Rc;

    const STRIDE: usize = 64;

    fn fixture() -> (SparseGuestMemory, GuestAddress, NativeModArmorState) {
        let module = ModuleIdentity::new(
            ProviderId::new("test", "armor"),
            "armor.so",
            ContentDigest {
                algorithm: "none".to_string(),
                value: "0".to_string(),
            },
            "r1",
        );
        let mut memory = SparseGuestMemory::new(module, 4, 0xA0000).unwrap();
        let base = memory
            .map(&GuestMapOptions::new(0x30000, STRIDE * 4, GuestPermissions::ReadWrite))
            .unwrap();
        let definition = ArmorDefinition::Q2 {
            regular: vec![RegularArmorItem {
                item: "q2:armor-jacket".to_string(),
                selection: ArmorSelection::Positive {
                    field: ArmorField {
                        record: "client".to_string(),
                        offset: 0,
                        storage: GuestStorage::Int32,
                    },
                },
                points: ArmorField {
                    record: "client".to_string(),
                    offset: 0,
                    storage: GuestStorage::Int32,
                },
                normal_protection: 0.3,
                energy_protection: 0.0,
            }],
            power: vec![PowerArmorItem {
                kind: PowerKind::Screen,
                item: "q2:power-screen".to_string(),
                selection: ArmorSelection::Enum {
                    field: ArmorField {
                        record: "client".to_string(),
                        offset: 4,
                        storage: GuestStorage::Int32,
                    },
                    value: 1.0,
                    none: 0.0,
                },
                cells: ArmorField {
                    record: "client".to_string(),
                    offset: 8,
                    storage: GuestStorage::Int32,
                },
                enabled: None,
            }],
        };
        let armor = NativeModArmorState::new(
            definition,
            "entity",
            &[
                ArmorRecordLayout {
                    id: "entity".to_string(),
                    stride: STRIDE,
                    is_client: false,
                },
                ArmorRecordLayout {
                    id: "client".to_string(),
                    stride: STRIDE,
                    is_client: true,
                },
            ],
            4,
        )
        .unwrap();
        (memory, base, armor)
    }

    #[test]
    fn q2_select_and_write_roundtrip() {
        let (mut memory, base, armor) = fixture();
        let space = memory.address_space();
        let base_for = |slot: usize, record: &str| {
            if record != "client" {
                return None;
            }
            Some(GuestAddress::new(space, base.offset + (slot * STRIDE) as u64))
        };
        let empty = armor.read(&mut memory, &base_for, 0).unwrap();
        assert!(matches!(empty.regular, RegularArmor::None));
        armor
            .write(
                &mut memory,
                &base_for,
                0,
                &ArmorState {
                    regular: RegularArmor::Q2 {
                        points: 50.0,
                        normal_protection: 0.3,
                        energy_protection: 0.0,
                        item: "q2:armor-jacket".to_string(),
                    },
                    powered: PoweredProtection::Screen { cells: 30 },
                },
            )
            .unwrap();
        let stored = armor.read(&mut memory, &base_for, 0).unwrap();
        assert!(matches!(
            stored.regular,
            RegularArmor::Q2 { points: 50.0, .. }
        ));
        assert_eq!(stored.powered, PoweredProtection::Screen { cells: 30 });
        armor
            .write(
                &mut memory,
                &base_for,
                0,
                &ArmorState {
                    regular: RegularArmor::None,
                    powered: PoweredProtection::None,
                },
            )
            .unwrap();
        let cleared = armor.read(&mut memory, &base_for, 0).unwrap();
        assert!(matches!(cleared.regular, RegularArmor::None));
        assert!(matches!(cleared.powered, PoweredProtection::None));
    }

    #[test]
    fn cross_family_write_is_rejected_and_watch_publishes_once() {
        let (mut memory, base, armor) = fixture();
        let space = memory.address_space();
        let base_for = |slot: usize, record: &str| {
            if record != "client" {
                return None;
            }
            Some(GuestAddress::new(space, base.offset + (slot * STRIDE) as u64))
        };
        let foreign = armor.write(
            &mut memory,
            &base_for,
            0,
            &ArmorState {
                regular: RegularArmor::Source {
                    points: 10.0,
                    item: Some("q2:armor-jacket".to_string()),
                },
                powered: PoweredProtection::None,
            },
        );
        assert_eq!(foreign, Err(ArmorError::Unrepresentable));

        let events: Rc<RefCell<Vec<(ArmorState, ArmorState)>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let mut watch = armor
            .observe(&mut memory, &base_for, 0, move |before, after| {
                sink.borrow_mut().push((before, after));
            })
            .unwrap();
        assert!(!watch.poll(&armor, &mut memory, &base_for, 0).unwrap());
        armor.write_count(&mut memory, &base_for, 0, &ArmorField {
            record: "client".to_string(),
            offset: 0,
            storage: GuestStorage::Int32,
        }, 25.0).unwrap();
        assert!(watch.poll(&armor, &mut memory, &base_for, 0).unwrap());
        assert!(!watch.poll(&armor, &mut memory, &base_for, 0).unwrap());
        assert_eq!(events.borrow().len(), 1);
        // Unrelated word: no publish.
        let other = memory.offset(base, (STRIDE + 48) as i64).unwrap();
        memory.write(other, &[1, 2, 3, 4]).unwrap();
        assert!(!watch.poll(&armor, &mut memory, &base_for, 0).unwrap());
        watch.release(&mut memory);
    }

    #[test]
    fn overlapping_and_ambiguous_declarations_fail() {
        let overlapping = ArmorDefinition::Source {
            regular: vec![RegularArmorItem {
                item: "q2:armor".to_string(),
                selection: ArmorSelection::Positive {
                    field: ArmorField {
                        record: "client".to_string(),
                        offset: 0,
                        storage: GuestStorage::Int32,
                    },
                },
                points: ArmorField {
                    record: "client".to_string(),
                    offset: 2,
                    storage: GuestStorage::Int32,
                },
                normal_protection: 0.0,
                energy_protection: 0.0,
            }],
            power: vec![],
        };
        let records = [ArmorRecordLayout {
            id: "client".to_string(),
            stride: STRIDE,
            is_client: true,
        }];
        assert_eq!(
            NativeModArmorState::new(overlapping, "entity", &records, 4),
            Err(ArmorError::OverlappingFields).map(|_: NativeModArmorState| ())
                .map_err(|error| error)
        );
    }
}
