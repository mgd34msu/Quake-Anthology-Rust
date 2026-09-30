//! QVM private item-table layout and cached catalog reads.
//!
//! Provenance: `src/compat/qvm/item-catalog.ts`.
//!
//! Offsets describe the mod's private item table; public weapon numbers stay
//! source-owned. [`QvmSourceItemCatalog`] caches reads until an observed
//! store touches its locator, records, or strings.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use super::game_data::{ProfileReader, QvmSharedMemory, QvmWriteRange};
use crate::error::GuestError;

/// Item-table address: direct offset or live global holding one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmItemAddress {
    /// Direct table offset.
    Direct(usize),
    /// Global holding the table offset.
    Global(usize),
}

/// Item-table count: direct count or live global with a maximum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmItemCount {
    /// Direct record count.
    Direct(usize),
    /// Global holding the count plus its maximum.
    Global {
        /// Global offset.
        global: usize,
        /// Maximum count.
        maximum: usize,
    },
}

/// Private item-table field offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmItemFields {
    /// Class-name pointer offset.
    pub class_name: usize,
    /// Pickup-name pointer offset.
    pub pickup_name: usize,
    /// Type word offset.
    pub item_type: usize,
    /// Tag word offset.
    pub tag: usize,
}

/// Declared private item-table layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmItemLayout {
    /// Table address.
    pub address: QvmItemAddress,
    /// Table count.
    pub count: QvmItemCount,
    /// Whether the table lives in live source storage.
    pub live_source: bool,
    /// Record stride.
    pub stride: usize,
    /// Field offsets.
    pub fields: QvmItemFields,
    /// Weapon type tag.
    pub weapon_type: i32,
    /// Ammo type tag.
    pub ammo_type: i32,
}

/// Parse an item-table layout declaration.
pub fn parse_qvm_item_layout(reader: &ProfileReader<'_>) -> Result<QvmItemLayout, GuestError> {
    let fields = reader.field("fields")?;
    let stride = reader.field("stride")?.integer(4)? as usize;
    let offset = |name: &str| -> Result<usize, GuestError> {
        let value = fields.field(name)?.integer(0)? as usize;
        if !value.is_multiple_of(4) || value + 4 > stride {
            return fields
                .field(name)?
                .fail("item field exceeds its record or is unaligned");
        }
        Ok(value)
    };
    let pointer = reader.field("address")?;
    let address = match pointer.value() {
        super::game_data::ProfileValue::Int(_) | super::game_data::ProfileValue::Float(_) => {
            QvmItemAddress::Direct(pointer.integer(4)? as usize)
        }
        _ => QvmItemAddress::Global(pointer.field("global")?.integer(0)? as usize),
    };
    let count_reader = reader.field("count")?;
    let count = match count_reader.value() {
        super::game_data::ProfileValue::Int(_) | super::game_data::ProfileValue::Float(_) => {
            QvmItemCount::Direct(count_reader.integer(1)? as usize)
        }
        _ => QvmItemCount::Global {
            global: count_reader.field("global")?.integer(0)? as usize,
            maximum: count_reader.field("maximum")?.integer(1)? as usize,
        },
    };
    let live_source = if reader.field("source")?.is_undefined() {
        false
    } else {
        reader.field("source")?.literal_str("live")?;
        true
    };
    let layout = QvmItemLayout {
        address,
        count,
        live_source,
        stride,
        fields: QvmItemFields {
            class_name: offset("className")?,
            pickup_name: offset("pickupName")?,
            item_type: offset("type")?,
            tag: offset("tag")?,
        },
        weapon_type: reader.field("weaponType")?.integer(0)? as i32,
        ammo_type: reader.field("ammoType")?.integer(0)? as i32,
    };
    let address_word = match layout.address {
        QvmItemAddress::Direct(address) => address,
        QvmItemAddress::Global(global) => global,
    };
    let count_aligned = match layout.count {
        QvmItemCount::Direct(_) => true,
        QvmItemCount::Global { global, .. } => global % 4 == 0,
    };
    if !address_word.is_multiple_of(4)
        || !count_aligned
        || !stride.is_multiple_of(4)
        || layout.weapon_type == layout.ammo_type
    {
        return reader.fail("invalid item table layout");
    }
    if !live_source
        && (!matches!(layout.address, QvmItemAddress::Direct(_)) || !matches!(layout.count, QvmItemCount::Direct(_)))
    {
        return reader.fail("runtime item table locations require live source storage");
    }
    Ok(layout)
}

/// Catalog item names and tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCatalogItem {
    /// Class name.
    pub class_name: String,
    /// Pickup name.
    pub pickup_name: String,
    /// Item type.
    pub item_type: i32,
    /// Item tag.
    pub tag: i32,
}

/// Catalog record with its table position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCatalogRecord {
    /// Record index.
    pub index: usize,
    /// Record address.
    pub address: usize,
    /// Class name.
    pub class_name: String,
    /// Pickup name.
    pub pickup_name: String,
    /// Item type.
    pub item_type: i32,
    /// Item tag.
    pub tag: i32,
}

fn read_word(data: &[u8], address: usize) -> Result<i32, GuestError> {
    if address + 4 > data.len() {
        return Err(GuestError::invalid(
            "QVM item table exceeds initialized or live module data",
        ));
    }
    let mut word = [0u8; 4];
    word.copy_from_slice(&data[address..address + 4]);
    Ok(i32::from_le_bytes(word))
}

/// Read item records, reporting touched ranges to `range`.
pub fn read_qvm_item_records(
    data: &[u8],
    layout: &QvmItemLayout,
    types: Option<&HashSet<i32>>,
    range: Option<&mut dyn FnMut(usize, usize)>,
) -> Result<Vec<QvmCatalogRecord>, GuestError> {
    let touch = |range: &mut Option<&mut dyn FnMut(usize, usize)>, offset: usize, length: usize| {
        if let Some(range) = range {
            range(offset, length);
        }
    };
    let mut range: Option<&mut dyn FnMut(usize, usize)> = range.map(|range| &mut *range);
    let read_global = |address: usize, range: &mut Option<&mut dyn FnMut(usize, usize)>| -> Result<i32, GuestError> {
        if !address.is_multiple_of(4) || address + 4 > data.len() {
            return Err(GuestError::invalid("QVM item table locator exceeds module data"));
        }
        touch(range, address, 4);
        read_word(data, address)
    };
    let table = match layout.address {
        QvmItemAddress::Direct(address) => address as i64,
        QvmItemAddress::Global(global_address) => read_global(global_address, &mut range)? as i64,
    };
    let (count, maximum) = match layout.count {
        QvmItemCount::Direct(count) => (count as i64, None),
        QvmItemCount::Global { global, maximum } => (read_global(global, &mut range)? as i64, Some(maximum as i64)),
    };
    if count < 0
        || maximum.is_some_and(|maximum| count > maximum)
        || table < 0
        || table % 4 != 0
        || (count > 0 && table == 0)
        || table + count * layout.stride as i64 > data.len() as i64
    {
        return Err(GuestError::invalid(
            "QVM item table exceeds initialized or live module data",
        ));
    }
    let (table, count) = (table as usize, count as usize);
    if count > 0 {
        touch(&mut range, table, count * layout.stride);
    }
    let string = |pointer: i32, range: &mut Option<&mut dyn FnMut(usize, usize)>| -> Result<String, GuestError> {
        if pointer <= 0 || pointer as usize >= data.len() {
            return Err(GuestError::invalid(
                "QVM item string is outside initialized module data",
            ));
        }
        let pointer = pointer as usize;
        let end = data[pointer..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|index| pointer + index)
            .ok_or_else(|| GuestError::invalid("QVM item string is unterminated"))?;
        touch(range, pointer, end - pointer + 1);
        Ok(data[pointer..end].iter().map(|byte| *byte as char).collect())
    };
    let mut items = Vec::new();
    for index in 0..count {
        let address = table + index * layout.stride;
        let word = |offset: usize| read_word(data, address + offset);
        if word(layout.fields.class_name)? == 0 {
            continue;
        }
        let item_type = word(layout.fields.item_type)?;
        if types.is_some_and(|types| !types.contains(&item_type)) {
            continue;
        }
        let class_name = string(word(layout.fields.class_name)?, &mut range)?;
        let pickup_name = string(word(layout.fields.pickup_name)?, &mut range)?;
        if class_name.is_empty() || pickup_name.is_empty() {
            return Err(GuestError::invalid("QVM item has no source name"));
        }
        items.push(QvmCatalogRecord {
            index,
            address,
            class_name,
            pickup_name,
            item_type,
            tag: word(layout.fields.tag)?,
        });
    }
    Ok(items)
}

/// Read weapon/ammo catalog items representable by public state.
pub fn read_qvm_item_catalog(
    data: &[u8],
    layout: &QvmItemLayout,
    private_inventory: bool,
) -> Result<Vec<QvmCatalogItem>, GuestError> {
    let types: HashSet<i32> = [layout.weapon_type, layout.ammo_type].into_iter().collect();
    read_qvm_item_records(data, layout, Some(&types), None)?
        .into_iter()
        .map(|item| {
            if item.tag < 1 || (!private_inventory && item.tag > 15) {
                return Err(GuestError::invalid(
                    "QVM item cannot be represented by its public weapon/ammo state",
                ));
            }
            Ok(QvmCatalogItem {
                class_name: item.class_name,
                pickup_name: item.pickup_name,
                item_type: item.item_type,
                tag: item.tag,
            })
        })
        .collect()
}

#[derive(Default)]
struct CatalogShared {
    records: Option<Vec<QvmCatalogRecord>>,
    watch: Option<u64>,
    closed: bool,
}

/// Catalog reads cached until an original store changes its locator,
/// records, or strings.
#[derive(Clone)]
pub struct QvmSourceItemCatalog {
    memory: QvmSharedMemory,
    layout: QvmItemLayout,
    initialized: Vec<u8>,
    shared: Rc<RefCell<CatalogShared>>,
}

impl std::fmt::Debug for QvmSourceItemCatalog {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QvmSourceItemCatalog")
            .field("cached", &self.shared.borrow().records.is_some())
            .finish()
    }
}

impl QvmSourceItemCatalog {
    /// Build a catalog over guest memory and initialized data.
    #[must_use]
    pub fn new(memory: QvmSharedMemory, layout: QvmItemLayout, initialized: Vec<u8>) -> Self {
        Self {
            memory,
            layout,
            initialized,
            shared: Rc::new(RefCell::new(CatalogShared::default())),
        }
    }

    /// Drop cached records and their observer.
    pub fn reset(&self) {
        let mut shared = self.shared.borrow_mut();
        if let Some(watch) = shared.watch.take() {
            self.memory.remove_observer(watch);
        }
        shared.records = None;
    }

    /// Read cached records, observing live storage when declared.
    pub fn records(&self) -> Result<Vec<QvmCatalogRecord>, GuestError> {
        self.memory.assert_live()?;
        if self.shared.borrow().closed {
            return Err(GuestError::invalid("QVM item catalog owner is retired"));
        }
        if let Some(records) = self.shared.borrow().records.clone() {
            return Ok(records);
        }
        self.reset();
        let mut ranges: Vec<QvmWriteRange> = Vec::new();
        let data;
        let records = if self.layout.live_source {
            data = self.memory.read_bytes(0, self.memory.len())?;
            let mut collect = |offset: usize, length: usize| {
                ranges.push(QvmWriteRange {
                    byte_offset: offset,
                    byte_length: length,
                });
            };
            read_qvm_item_records(&data, &self.layout, None, Some(&mut collect))?
        } else {
            read_qvm_item_records(&self.initialized, &self.layout, None, None)?
        };
        self.shared.borrow_mut().records = Some(records.clone());
        if !ranges.is_empty() {
            let shared = Rc::clone(&self.shared);
            let watch = self.memory.observe_writes(
                ranges,
                Rc::new(move |_| {
                    shared.borrow_mut().records = None;
                }),
                None,
            );
            self.shared.borrow_mut().watch = Some(watch);
        }
        Ok(records)
    }

    /// Reset and retire the catalog.
    pub fn close(&self) {
        self.reset();
        self.shared.borrow_mut().closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::game_data::ProfileValue;
    use super::*;

    fn layout() -> QvmItemLayout {
        QvmItemLayout {
            address: QvmItemAddress::Direct(64),
            count: QvmItemCount::Direct(2),
            live_source: false,
            stride: 32,
            fields: QvmItemFields {
                class_name: 0,
                pickup_name: 4,
                item_type: 8,
                tag: 12,
            },
            weapon_type: 1,
            ammo_type: 2,
        }
    }

    fn data() -> Vec<u8> {
        let mut data = vec![0u8; 512];
        let mut word = |offset: usize, value: i32| {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        word(64, 256);
        word(68, 272);
        word(72, 1);
        word(76, 3);
        word(96, 288);
        word(100, 304);
        word(104, 2);
        word(108, 5);
        data[256..270].copy_from_slice(b"weapon_rocket\0");
        data[272..280].copy_from_slice(b"Rocket\0\0");
        data[288..298].copy_from_slice(b"ammo_rock\0");
        data[304..313].copy_from_slice(b"Rockets\0\0");
        data
    }

    #[test]
    fn layout_parser_validates_geometry() {
        let value = ProfileValue::record(vec![
            ("address", ProfileValue::Int(64)),
            ("count", ProfileValue::Int(2)),
            ("stride", ProfileValue::Int(32)),
            (
                "fields",
                ProfileValue::record(vec![
                    ("className", ProfileValue::Int(0)),
                    ("pickupName", ProfileValue::Int(4)),
                    ("type", ProfileValue::Int(8)),
                    ("tag", ProfileValue::Int(12)),
                ]),
            ),
            ("weaponType", ProfileValue::Int(1)),
            ("ammoType", ProfileValue::Int(2)),
        ]);
        let parsed = parse_qvm_item_layout(&ProfileReader::new(&value)).unwrap();
        assert_eq!(parsed, layout());
        let bad = ProfileValue::record(vec![
            ("address", ProfileValue::Int(63)),
            ("count", ProfileValue::Int(2)),
            ("stride", ProfileValue::Int(32)),
            (
                "fields",
                ProfileValue::record(vec![
                    ("className", ProfileValue::Int(0)),
                    ("pickupName", ProfileValue::Int(4)),
                    ("type", ProfileValue::Int(8)),
                    ("tag", ProfileValue::Int(12)),
                ]),
            ),
            ("weaponType", ProfileValue::Int(1)),
            ("ammoType", ProfileValue::Int(1)),
        ]);
        assert!(parse_qvm_item_layout(&ProfileReader::new(&bad)).is_err());
    }

    #[test]
    fn records_skip_empty_class_names_and_filter_types() {
        let data = data();
        let records = read_qvm_item_records(&data, &layout(), None, None).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].class_name, "weapon_rocket");
        assert_eq!(records[0].pickup_name, "Rocket");
        assert_eq!((records[0].index, records[0].address), (0, 64));
        assert_eq!((records[1].item_type, records[1].tag), (2, 5));
        let types: HashSet<i32> = [1].into_iter().collect();
        let weapons = read_qvm_item_records(&data, &layout(), Some(&types), None).unwrap();
        assert_eq!(weapons.len(), 1);
        let catalog = read_qvm_item_catalog(&data, &layout(), false).unwrap();
        assert_eq!(catalog.len(), 2);
        let mut tags = layout();
        tags.count = QvmItemCount::Direct(1);
        let mut over = data.clone();
        over[76..80].copy_from_slice(&16i32.to_le_bytes());
        assert!(read_qvm_item_catalog(&over, &tags, false).is_err());
        assert!(read_qvm_item_catalog(&over, &tags, true).is_ok());
    }

    #[test]
    fn catalog_caches_until_observed_store() {
        let memory = QvmSharedMemory::new(512).unwrap();
        memory.write_bytes(0, &data()).unwrap();
        let mut live = layout();
        live.live_source = true;
        live.address = QvmItemAddress::Global(32);
        live.count = QvmItemCount::Global { global: 36, maximum: 4 };
        memory.write_i32(32, 64).unwrap();
        memory.write_i32(36, 2).unwrap();
        let catalog = QvmSourceItemCatalog::new(memory.clone(), live, vec![0u8; 512]);
        assert_eq!(catalog.records().unwrap().len(), 2);
        assert_eq!(catalog.records().unwrap().len(), 2);
        memory.write_i32(76, 9).unwrap();
        assert_eq!(catalog.records().unwrap()[0].tag, 9);
        catalog.close();
        assert!(catalog.records().is_err());
    }

    #[test]
    fn corrupt_tables_rejected() {
        let data = data();
        let mut bad = layout();
        bad.count = QvmItemCount::Direct(40);
        assert!(read_qvm_item_records(&data, &bad, None, None).is_err());
        let mut unterminated = data.clone();
        unterminated[256..].fill(b'x');
        assert!(read_qvm_item_records(&unterminated, &layout(), None, None).is_err());
    }
}
