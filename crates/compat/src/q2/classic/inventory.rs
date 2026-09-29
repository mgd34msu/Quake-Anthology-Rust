//! Donor: `src/compat/q2/classic/inventory.ts` — source inventory over the
//! original item table.
//!
//! Bridges the native `itemlist` array plus live client words to shared
//! inventory entries: ammo capacities stay mutable source words while plain
//! counters keep fixed capacities.

use std::collections::HashSet;

use qa_guest::core::contracts::GuestAddress;
use qa_world::combat::{item_id, ItemId};
use qa_world::inventory::{CountArithmetic, CountPolicy, InventoryEntry};

use super::host::ClassicQ2GuestHost;
use super::layout::{ClassicQ2Error, ClassicResult};
use super::records::read_classic_string;
use super::world_profile::{classic_primary_world_profile, ClassicPrimaryWorldProfile};

/// Capacity class of one native item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCapacity {
    /// Fixed-capacity counter.
    Counter,
    /// Ammo with a mutable client capacity word.
    Ammo {
        /// Client capacity field offset.
        offset: usize,
    },
}

/// One resolved native item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicNativeItem {
    /// Item identity.
    pub item: ItemId,
    /// Table index.
    pub index: usize,
    /// Capacity class.
    pub capacity: NativeCapacity,
}

/// The original item table plus per-client count bindings.
#[derive(Debug)]
pub struct ClassicSourceInventory {
    profile: ClassicPrimaryWorldProfile,
    items: Vec<ClassicNativeItem>,
}

impl ClassicSourceInventory {
    /// Resolve the item table at `image`, unless the digest is unadmitted.
    pub fn create(
        host: &mut ClassicQ2GuestHost,
        image: GuestAddress,
        declared: Option<ClassicPrimaryWorldProfile>,
    ) -> ClassicResult<Option<Self>> {
        let profile = declared.or_else(|| classic_primary_world_profile(&host.memory.module().digest));
        let Some(profile) = profile else {
            return Ok(None);
        };
        if profile.combat.digest != host.memory.module().digest {
            return Err(ClassicQ2Error::invalid(
                "Classic inventory profile belongs to another artifact",
            ));
        }
        let table = &profile.inventory_table;
        let mut items = Vec::with_capacity(table.count);
        let mut names = HashSet::with_capacity(table.count);
        for index in 0..table.count {
            let address = host.memory.offset(
                image,
                (profile.combat.globals.item_list as usize + index * profile.combat.globals.item_bytes) as i64,
            )?;
            let class_name_at = host.memory.offset(address, table.class_name as i64)?;
            let class_name = host.memory.read_pointer(class_name_at)?;
            let classname = read_classic_string(&mut host.memory, class_name, 65536)?;
            let label_at = host.memory.offset(address, table.label as i64)?;
            let label_ptr = host.memory.read_pointer(label_at)?;
            let label = read_classic_string(&mut host.memory, label_ptr, 65536)?;
            let mut item = None;
            if classname.is_empty() {
                item = table
                    .unnamed
                    .iter()
                    .find(|value| value.index == index && value.label == label)
                    .map(|entry| entry.item.clone());
            }
            let item = match item {
                Some(item) => item,
                None => {
                    if !is_classname(&classname) {
                        return Err(ClassicQ2Error::invalid("Native item has no qualified classname"));
                    }
                    item_id("q2", &classname)
                }
            };
            if !names.insert(item.clone()) {
                return Err(ClassicQ2Error::invalid("Duplicate native inventory item"));
            }
            let flags = host.memory.read_i32(host.memory.offset(address, table.flags as i64)?)?;
            if (i64::from(flags) & table.ammo_flag) as i32 == 0 {
                items.push(ClassicNativeItem {
                    item,
                    index,
                    capacity: NativeCapacity::Counter,
                });
            } else {
                let tag = host.memory.read_i32(host.memory.offset(address, table.tag as i64)?)?;
                let offset = table
                    .capacities
                    .get(tag as usize)
                    .copied()
                    .ok_or_else(|| ClassicQ2Error::invalid("Native ammo tag has no qualified capacity field"))?;
                items.push(ClassicNativeItem {
                    item,
                    index,
                    capacity: NativeCapacity::Ammo { offset },
                });
            }
        }
        if table.sentinel {
            let sentinel = host.memory.offset(
                image,
                (profile.combat.globals.item_list as usize + table.count * profile.combat.globals.item_bytes) as i64,
            )?;
            if host
                .memory
                .copy(sentinel, profile.combat.globals.item_bytes)?
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(ClassicQ2Error::invalid(
                    "Native item table exceeds its qualified roster",
                ));
            }
        }
        Ok(Some(Self { profile, items }))
    }

    /// Resolved items in table order.
    #[must_use]
    pub fn items(&self) -> &[ClassicNativeItem] {
        &self.items
    }

    /// Whether an item has a mutable ammo capacity.
    #[must_use]
    pub fn mutable_capacity(&self, item: &str) -> bool {
        self.items
            .iter()
            .any(|value| value.item == item && matches!(value.capacity, NativeCapacity::Ammo { .. }))
    }

    /// Bind one live client slot.
    pub fn bind(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<ClassicInventoryBinding<'_>> {
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.at(&mut host.memory, slot)?;
        if edicts.current(&mut host.memory, &host.registry, &record)?.is_none() {
            return Err(ClassicQ2Error::invalid("Native inventory requires a live source actor"));
        }
        Ok(ClassicInventoryBinding { inventory: self, slot })
    }

    fn client(&self, host: &mut ClassicQ2GuestHost, slot: u32) -> ClassicResult<GuestAddress> {
        let edicts = host
            .edicts
            .as_mut()
            .ok_or_else(|| ClassicQ2Error::invalid("GetGameAPI has not returned its export table"))?;
        let record = edicts.at(&mut host.memory, slot)?;
        if edicts.current(&mut host.memory, &host.registry, &record)?.is_none() {
            return Err(ClassicQ2Error::invalid("Native inventory owner was released"));
        }
        host.memory
            .read_pointer(host.memory.offset(record.address, 84)?)?
            .ok_or_else(|| ClassicQ2Error::invalid("Native inventory requires a source client"))
    }

    fn count_offset(&self, item: &ClassicNativeItem) -> i64 {
        (self.profile.combat.client.inventory + item.index * 4) as i64
    }
}

/// Client binding reading and writing live source counters.
#[derive(Debug)]
pub struct ClassicInventoryBinding<'i> {
    inventory: &'i ClassicSourceInventory,
    slot: u32,
}

impl ClassicInventoryBinding<'_> {
    /// Bound slot.
    #[must_use]
    pub fn slot(&self) -> u32 {
        self.slot
    }

    /// Read every entry with live counts and capacities.
    pub fn read(&self, host: &mut ClassicQ2GuestHost) -> ClassicResult<Vec<InventoryEntry>> {
        let client = self.inventory.client(host, self.slot)?;
        let mut entries = Vec::with_capacity(self.inventory.items.len());
        for item in &self.inventory.items {
            let count = host
                .memory
                .read_i32(host.memory.offset(client, self.inventory.count_offset(item))?)?;
            let capacity = match item.capacity {
                NativeCapacity::Ammo { offset } => host.memory.read_i32(host.memory.offset(client, offset as i64)?)?,
                NativeCapacity::Counter => {
                    if item.index == self.inventory.profile.inventory_table.empty_index {
                        0
                    } else {
                        0x7fff_ffff
                    }
                }
            };
            entries.push(InventoryEntry {
                item: item.item.clone(),
                count: f64::from(count),
                capacity: f64::from(capacity),
                count_policy: Some(CountPolicy::SourceCounter(CountArithmetic::Int32)),
            });
        }
        Ok(entries)
    }

    /// Write one entry's count and capacity.
    pub fn write(&self, host: &mut ClassicQ2GuestHost, item: &str, count: i32, capacity: i32) -> ClassicResult<()> {
        let native = self
            .inventory
            .items
            .iter()
            .find(|value| value.item == item)
            .ok_or_else(|| ClassicQ2Error::invalid("Item has no native inventory binding"))?;
        if capacity < 0 {
            return Err(ClassicQ2Error::invalid("Native inventory exceeds int32"));
        }
        if matches!(native.capacity, NativeCapacity::Counter) {
            let fixed = if native.index == self.inventory.profile.inventory_table.empty_index {
                0
            } else {
                0x7fff_ffff
            };
            if capacity != fixed {
                return Err(ClassicQ2Error::invalid(
                    "Native item has a fixed source counter capacity",
                ));
            }
        }
        let client = self.inventory.client(host, self.slot)?;
        if let NativeCapacity::Ammo { offset } = native.capacity {
            host.memory
                .write_i32(host.memory.offset(client, offset as i64)?, capacity)?;
        }
        host.memory
            .write_i32(host.memory.offset(client, self.inventory.count_offset(native))?, count)?;
        Ok(())
    }
}

fn is_classname(value: &str) -> bool {
    let mut bytes = value.bytes();
    if !bytes.next().is_some_and(|first| first.is_ascii_lowercase()) {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::super::combat_profile::XATRIX_DIGEST_VALUE;
    use super::super::records::{allocate_classic_string, ClassicQ2Edicts};
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_host() -> ClassicQ2GuestHost {
        ClassicQ2GuestHost::new(
            ProviderId::new("q2", "classic"),
            ModuleIdentity::new(
                ProviderId::new("q2", "classic"),
                "gamex86.dll",
                ContentDigest::new("sha256", XATRIX_DIGEST_VALUE),
                "test",
            ),
            100000,
        )
        .unwrap()
    }

    fn fixture(host: &mut ClassicQ2GuestHost) -> GuestAddress {
        let profile = classic_primary_world_profile(&ContentDigest::new("sha256", XATRIX_DIGEST_VALUE)).unwrap();
        let table = &profile.inventory_table;
        let image = host.memory.allocate(&GuestAllocationOptions::bytes(0x50000)).unwrap();
        for index in 0..table.count {
            let address = host
                .memory
                .offset(
                    image,
                    (profile.combat.globals.item_list as usize + index * profile.combat.globals.item_bytes) as i64,
                )
                .unwrap();
            let (classname, label, flags, tag): (String, &str, i32, i32) = match index {
                0 => (String::new(), "", 0, 0),
                47 => (String::new(), "Health", 0, 0),
                5 => ("ammo_shells".to_string(), "Shells", 2, 1),
                _ => (format!("item_{index}"), "Label", 0, 0),
            };
            let classname_ptr = allocate_classic_string(&mut host.memory, &classname).unwrap();
            let label_ptr = allocate_classic_string(&mut host.memory, label).unwrap();
            host.memory
                .write_pointer(
                    host.memory.offset(address, table.class_name as i64).unwrap(),
                    Some(classname_ptr),
                )
                .unwrap();
            host.memory
                .write_pointer(
                    host.memory.offset(address, table.label as i64).unwrap(),
                    Some(label_ptr),
                )
                .unwrap();
            host.memory
                .write_i32(host.memory.offset(address, table.flags as i64).unwrap(), flags)
                .unwrap();
            host.memory
                .write_i32(host.memory.offset(address, table.tag as i64).unwrap(), tag)
                .unwrap();
        }
        let client = host.memory.allocate(&GuestAllocationOptions::bytes(4096)).unwrap();
        host.memory
            .write_i32(host.memory.offset(client, 740 + 5 * 4).unwrap(), 12)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(client, 0x6e8).unwrap(), 50)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(client, 740).unwrap(), 0)
            .unwrap();
        let edicts = host.memory.allocate(&GuestAllocationOptions::bytes(896 * 4)).unwrap();
        let exports = host.memory.allocate(&GuestAllocationOptions::bytes(80)).unwrap();
        host.memory.write_i32(exports, 3).unwrap();
        host.memory
            .write_pointer(host.memory.offset(exports, 64).unwrap(), Some(edicts))
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(exports, 68).unwrap(), 896)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(exports, 72).unwrap(), 4)
            .unwrap();
        host.memory
            .write_i32(host.memory.offset(exports, 76).unwrap(), 4)
            .unwrap();
        let one = host.memory.offset(edicts, 896).unwrap();
        host.memory.write_i32(host.memory.offset(one, 88).unwrap(), 1).unwrap();
        host.memory
            .write_pointer(host.memory.offset(one, 84).unwrap(), Some(client))
            .unwrap();
        host.edicts =
            Some(ClassicQ2Edicts::new(&mut host.memory, exports, ProviderId::new("q2", "classic"), None).unwrap());
        image
    }

    #[test]
    fn table_resolves_and_reads_live_counters() {
        let mut host = test_host();
        let image = fixture(&mut host);
        let inventory = ClassicSourceInventory::create(&mut host, image, None).unwrap().unwrap();
        assert_eq!(inventory.items().len(), 48);
        assert_eq!(inventory.items()[0].item, "q2:none");
        assert_eq!(inventory.items()[47].item, "q2:item_health");
        assert_eq!(inventory.items()[5].item, "q2:ammo_shells");
        assert!(inventory.mutable_capacity("q2:ammo_shells"));
        assert!(!inventory.mutable_capacity("q2:item_1"));
        let binding = inventory.bind(&mut host, 1).unwrap();
        let entries = binding.read(&mut host).unwrap();
        let shells = entries.iter().find(|entry| entry.item == "q2:ammo_shells").unwrap();
        assert_eq!((shells.count, shells.capacity), (12.0, 50.0));
        assert_eq!(
            shells.count_policy,
            Some(CountPolicy::SourceCounter(CountArithmetic::Int32))
        );
        let empty = entries.iter().find(|entry| entry.item == "q2:none").unwrap();
        assert_eq!(empty.capacity, 0.0);
        let counter = entries.iter().find(|entry| entry.item == "q2:item_1").unwrap();
        assert_eq!(counter.capacity, f64::from(0x7fff_ffffi32));
        assert!(inventory.bind(&mut host, 2).is_err());
    }

    #[test]
    fn writes_commit_and_reject_bad_bindings() {
        let mut host = test_host();
        let image = fixture(&mut host);
        let inventory = ClassicSourceInventory::create(&mut host, image, None).unwrap().unwrap();
        let binding = inventory.bind(&mut host, 1).unwrap();
        binding.write(&mut host, "q2:ammo_shells", 20, 100).unwrap();
        let entries = binding.read(&mut host).unwrap();
        let shells = entries.iter().find(|entry| entry.item == "q2:ammo_shells").unwrap();
        assert_eq!((shells.count, shells.capacity), (20.0, 100.0));
        binding.write(&mut host, "q2:item_1", 3, 0x7fff_ffff).unwrap();
        assert!(binding.write(&mut host, "q2:item_1", 3, 10).is_err());
        assert!(binding.write(&mut host, "q2:missing", 1, 1).is_err());
        assert!(binding.write(&mut host, "q2:ammo_shells", 1, -1).is_err());
        let mut foreign = test_host();
        foreign.memory = qa_guest::core::memory::SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "classic"),
                "other.dll",
                ContentDigest::new("sha256", "other"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap();
        assert!(ClassicSourceInventory::create(&mut foreign, image, None)
            .unwrap()
            .is_none());
    }
}
