//! Port of `src/compat/q2/native-primary-commands.ts`.
//! Bridges original give/drop commands: item tables, grants and death-drop projection.

use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue, GuestRegister, NativeAbi};
use qa_world::combat::ItemId;

use super::native_primary_reader::NativeRegion;
use super::native_primary_weapons::{HostResult, NativeActorId, NativeHostError, SyntheticHost};

/// Ammo grant region with its descriptor register and grant kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmmoGrant {
    /// Grant region entry RVA.
    pub entry: u32,
    /// Grant region join RVA.
    pub join: u32,
    /// Register holding the item descriptor.
    pub descriptor: GuestRegister,
    /// Grant kind.
    pub kind: GrantKind,
}

/// Ammo grant kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantKind {
    /// Counter is set.
    Set,
    /// Counter is incremented.
    Add,
}

/// Give command declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GiveProfile {
    /// Give entry RVA.
    pub entry: u32,
    /// Give-weapons observation RVA.
    pub weapons: u32,
    /// Give-ammo observation RVA.
    pub ammo: u32,
    /// Unknown-item region.
    pub unknown: NativeRegion,
    /// Ammo grant regions.
    pub ammo_grants: Vec<AmmoGrant>,
    /// Imported argc RVA.
    pub argc: u32,
    /// Imported argv RVA.
    pub argv: u32,
}

/// Drop command declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropCommand {
    /// Drop entry RVA.
    pub entry: u32,
    /// Eligibility region.
    pub eligibility: NativeRegion,
}

/// Client offsets used by commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandClient {
    /// Client pointer offset within the entity.
    pub pointer: u32,
    /// Current weapon offset within the client.
    pub weapon: u32,
    /// Ammo index offset within the client, when stored.
    pub ammo_index: Option<u32>,
    /// Inventory offset within the client.
    pub inventory: u32,
}

/// Weapon ammo reference inside the item table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemAmmo {
    /// Ammo referenced by display name.
    Name {
        /// Ammo name pointer offset.
        offset: u32,
        /// Item label pointer offset used for resolution.
        label: u32,
    },
    /// Ammo referenced by table index.
    Index {
        /// Ammo index offset.
        offset: u32,
    },
}

/// Source item table declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandItems {
    /// Table base RVA.
    pub table: u32,
    /// Row stride in bytes.
    pub stride: u32,
    /// Row count.
    pub count: u32,
    /// Classname pointer offset.
    pub classname: u32,
    /// Flags offset.
    pub flags: u32,
    /// Weapon flag bit.
    pub weapon_flag: u32,
    /// Ammunition flag bit.
    pub ammunition_flag: u32,
    /// Icon pointer offset.
    pub icon: u32,
    /// Weapon ammo reference.
    pub ammo: ItemAmmo,
}

/// Native primary command profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePrimaryCommandProfile {
    /// Artifact digest.
    pub digest: String,
    /// Executable ABI.
    pub abi: NativeAbi,
    /// Give command.
    pub give: GiveProfile,
    /// Drop command.
    pub drop: DropCommand,
    /// Client offsets.
    pub client: CommandClient,
    /// Item table.
    pub items: CommandItems,
}

/// Give observation category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveCategory {
    /// Give-all-weapons path.
    Weapons,
    /// Give-all-ammo path.
    Ammo,
}

/// Observed ammo counter change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmmoChange {
    /// Grant kind.
    pub kind: GrantKind,
    /// Counter delta (or absolute value for `Set`).
    pub amount: i32,
}

/// Selected death-drop projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeathDrop {
    /// Projected weapon, or bare-hands drop.
    pub item: Option<ItemId>,
    /// Projected ammo counter.
    pub ammo: i32,
}

/// Command hooks owned by the caller.
pub struct CommandHooks {
    /// Give-path observation.
    pub give: Box<dyn FnMut(NativeActorId, GiveCategory)>,
    /// Unknown-item give; returns true to skip the original path.
    pub give_item: Box<dyn FnMut(NativeActorId, &[String]) -> bool>,
    /// Observed ammo grant.
    pub give_ammo: Box<dyn FnMut(NativeActorId, ItemId, AmmoChange)>,
    /// Death-drop projection, or `None` to run the original.
    pub drop: Box<dyn FnMut(NativeActorId) -> Option<DeathDrop>>,
}

/// One resolved source item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandItem {
    /// Canonical item id.
    pub item: ItemId,
    /// Table index.
    pub index: u32,
    /// Descriptor address.
    pub address: GuestAddress,
    /// Weapon row.
    pub weapon: bool,
    /// Ammunition row.
    pub ammunition: bool,
    /// HUD icon string.
    pub icon: String,
    /// Linked ammo item for weapons.
    pub ammo: Option<ItemId>,
}

/// Inventory slot view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventorySlot {
    /// Canonical item id.
    pub item: ItemId,
    /// Table index.
    pub index: u32,
    /// Weapon row.
    pub weapon: bool,
    /// Ammunition row.
    pub ammunition: bool,
    /// HUD icon string.
    pub icon: String,
}

/// Weapon view with linked ammo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponView {
    /// Canonical item id.
    pub item: ItemId,
    /// Linked ammo item.
    pub ammo: Option<ItemId>,
}

/// Original commands retain cheat checks, grants and map-owned drop decisions.
pub struct NativePrimaryCommands {
    profile: NativePrimaryCommandProfile,
    hooks: CommandHooks,
    items: Vec<CommandItem>,
    giving: Vec<Option<NativeActorId>>,
    dropping: Vec<Option<NativeActorId>>,
    closed: bool,
}

fn is_item_name(classname: &str) -> bool {
    let mut chars = classname.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '_')
}

impl NativePrimaryCommands {
    /// Build the service and resolve the source item table.
    pub fn new(
        host: &mut SyntheticHost,
        profile: NativePrimaryCommandProfile,
        hooks: CommandHooks,
    ) -> HostResult<Self> {
        if host.core.digest != profile.digest {
            return Err(NativeHostError::Fault(
                "native command profile belongs to another artifact".to_string(),
            ));
        }
        let mut service = Self {
            profile,
            hooks,
            items: Vec::new(),
            giving: Vec::new(),
            dropping: Vec::new(),
            closed: false,
        };
        service.items = service.read_items(host)?;
        Ok(service)
    }

    /// Borrow the profile.
    #[must_use]
    pub fn profile(&self) -> &NativePrimaryCommandProfile {
        &self.profile
    }

    fn at(&self, host: &SyntheticHost, offset: u32) -> HostResult<GuestAddress> {
        host.core.at(offset)
    }

    fn current(&self, host: &SyntheticHost, actor: NativeActorId) -> bool {
        !self.closed && host.core.is_live(actor)
    }

    fn read_string_at(&self, host: &mut SyntheticHost, base: GuestAddress, offset: u32) -> HostResult<String> {
        let slot = host.core.memory.offset(base, i64::from(offset))?;
        match host.core.memory.read_pointer(slot)? {
            Some(address) => host.core.read_c_string(address, 65536),
            None => Ok(String::new()),
        }
    }

    fn read_items(&self, host: &mut SyntheticHost) -> HostResult<Vec<CommandItem>> {
        let table = &self.profile.items;
        struct Row {
            item: Option<ItemId>,
            index: u32,
            address: GuestAddress,
            icon: String,
            ammunition: bool,
            weapon: bool,
            label: String,
        }
        let mut source = Vec::with_capacity(table.count as usize);
        for index in 0..table.count {
            let base = u64::from(table.table) + u64::from(index) * u64::from(table.stride);
            let base = u32::try_from(base)
                .map_err(|_| NativeHostError::Fault("original item table exceeds its image".to_string()))?;
            let address = self.at(host, base)?;
            let classname = self.read_string_at(host, address, table.classname)?;
            let item = if is_item_name(&classname) {
                Some(format!("q2:{classname}"))
            } else {
                None
            };
            let icon = self.read_string_at(host, address, table.icon)?;
            let flags = host
                .core
                .memory
                .read_u32(host.core.memory.offset(address, i64::from(table.flags))?)?;
            let label = match table.ammo {
                ItemAmmo::Name { label, .. } => self.read_string_at(host, address, label)?,
                ItemAmmo::Index { .. } => String::new(),
            };
            source.push(Row {
                item,
                index,
                address,
                icon,
                ammunition: flags & table.ammunition_flag != 0,
                weapon: flags & table.weapon_flag != 0,
                label,
            });
        }
        let mut items = Vec::new();
        let mut names = std::collections::HashSet::new();
        for row in &source {
            let item = match &row.item {
                Some(item) => item,
                None => {
                    if row.weapon {
                        return Err(NativeHostError::Fault(
                            "original weapon has no qualified classname".to_string(),
                        ));
                    }
                    continue;
                }
            };
            if !names.insert(item.clone()) {
                return Err(NativeHostError::Fault(
                    "original item table has duplicate classnames".to_string(),
                ));
            }
            let mut ammo = None;
            if row.weapon {
                let ammo_field = match table.ammo {
                    ItemAmmo::Name { offset, .. } | ItemAmmo::Index { offset } => offset,
                };
                let slot = host.core.memory.offset(row.address, i64::from(ammo_field))?;
                match table.ammo {
                    ItemAmmo::Name { .. } => {
                        let name = match host.core.memory.read_pointer(slot)? {
                            Some(address) => host.core.read_c_string(address, 65536)?,
                            None => String::new(),
                        };
                        if !name.is_empty() {
                            let found = source.iter().find(|row| row.label.eq_ignore_ascii_case(&name));
                            match found.and_then(|row| row.item.clone()) {
                                Some(item) => ammo = Some(item),
                                None => {
                                    return Err(NativeHostError::Fault(
                                        "original weapon ammo name has no source item".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                    ItemAmmo::Index { .. } => {
                        let index = host.core.memory.read_i32(slot)?;
                        if index != 0 {
                            let found = usize::try_from(index).ok().and_then(|index| source.get(index));
                            match found.and_then(|row| row.item.clone()) {
                                Some(item) => ammo = Some(item),
                                None => {
                                    return Err(NativeHostError::Fault(
                                        "original weapon ammo index has no source item".to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            items.push(CommandItem {
                item: item.clone(),
                index: row.index,
                address: row.address,
                weapon: row.weapon,
                ammunition: row.ammunition,
                icon: row.icon.clone(),
                ammo,
            });
        }
        Ok(items)
    }

    /// Resolve a canonical item to its table slot.
    #[must_use]
    pub fn source_item(&self, item: &ItemId) -> Option<&CommandItem> {
        self.items.iter().find(|row| &row.item == item)
    }

    /// Resolve a descriptor address to its canonical item.
    #[must_use]
    pub fn item_at(&self, address: GuestAddress) -> Option<ItemId> {
        self.items
            .iter()
            .find(|row| row.address == address)
            .map(|row| row.item.clone())
    }

    /// Inventory slot views in table order.
    #[must_use]
    pub fn inventory_slots(&self) -> Vec<InventorySlot> {
        self.items
            .iter()
            .map(|row| InventorySlot {
                item: row.item.clone(),
                index: row.index,
                weapon: row.weapon,
                ammunition: row.ammunition,
                icon: row.icon.clone(),
            })
            .collect()
    }

    /// Weapon views with linked ammo.
    #[must_use]
    pub fn weapons(&self) -> Vec<WeaponView> {
        self.items
            .iter()
            .filter(|row| row.weapon)
            .map(|row| WeaponView {
                item: row.item.clone(),
                ammo: row.ammo.clone(),
            })
            .collect()
    }

    /// Enter a give scope for an actor (headless entry wrapper).
    pub fn begin_give(&mut self, actor: Option<NativeActorId>) {
        self.giving.push(actor);
    }

    /// Leave a give scope.
    pub fn end_give(&mut self) {
        self.giving.pop();
    }

    /// Enter a drop scope for an actor (headless entry wrapper).
    pub fn begin_drop(&mut self, actor: Option<NativeActorId>) {
        self.dropping.push(actor);
    }

    /// Leave a drop scope.
    pub fn end_drop(&mut self) {
        self.dropping.pop();
    }

    /// Observe a give-all path for the scoped actor.
    pub fn observe_give_category(&mut self, host: &SyntheticHost, category: GiveCategory) {
        if let Some(Some(actor)) = self.giving.last().copied() {
            if self.current(host, actor) {
                (self.hooks.give)(actor, category);
            }
        }
    }

    /// Run the unknown-item region; returns true when the original path skips.
    pub fn unknown_give(&mut self, host: &mut SyntheticHost) -> HostResult<bool> {
        let actor = match self.giving.last().copied().flatten() {
            Some(actor) if self.current(host, actor) => actor,
            _ => return Ok(false),
        };
        let arguments = self.command_arguments(host)?;
        Ok((self.hooks.give_item)(actor, &arguments))
    }

    /// Run one ammo grant region around the original increment.
    pub fn ammo_grant(
        &mut self,
        host: &mut SyntheticHost,
        grant_index: usize,
        execute: impl FnOnce(&mut SyntheticHost) -> HostResult<()>,
    ) -> HostResult<()> {
        let actor = match self.giving.last().copied().flatten() {
            Some(actor) if self.current(host, actor) => actor,
            _ => return execute(&mut *host),
        };
        let grant = self
            .profile
            .give
            .ammo_grants
            .get(grant_index)
            .ok_or_else(|| NativeHostError::Fault("unknown original ammo grant".to_string()))?;
        let descriptor = host.core.register_read(grant.descriptor);
        let descriptor = if host.core.pointer_bytes() == 4 {
            descriptor & 0xFFFF_FFFF
        } else {
            descriptor
        };
        let row = self
            .items
            .iter()
            .find(|row| {
                let offset = if host.core.pointer_bytes() == 4 {
                    row.address.offset & 0xFFFF_FFFF
                } else {
                    row.address.offset
                };
                offset == descriptor
            })
            .ok_or_else(|| NativeHostError::Fault("original ammo command lacks its source item".to_string()))?;
        let entity = host
            .core
            .entity_of(actor)
            .map_err(|_| NativeHostError::Fault("original ammo command lacks its source item".to_string()))?;
        let client = host.core.memory.read_pointer(
            host.core
                .memory
                .offset(entity, i64::from(self.profile.client.pointer))?,
        )?;
        let client =
            client.ok_or_else(|| NativeHostError::Fault("original ammo command lost its client".to_string()))?;
        let counter = host.core.memory.offset(
            client,
            i64::from(self.profile.client.inventory) + i64::from(row.index) * 4,
        )?;
        let before = host.core.memory.read_i32(counter)?;
        execute(&mut *host)?;
        if self.current(host, actor) {
            let after = host.core.memory.read_i32(counter)?;
            let amount = match grant.kind {
                GrantKind::Set => after,
                GrantKind::Add => after.wrapping_sub(before),
            };
            (self.hooks.give_ammo)(
                actor,
                row.item.clone(),
                AmmoChange {
                    kind: grant.kind,
                    amount,
                },
            );
        }
        Ok(())
    }

    /// Run the death-drop eligibility region; returns true when projected.
    pub fn death_drop(
        &mut self,
        host: &mut SyntheticHost,
        execute: impl FnOnce(&mut SyntheticHost) -> HostResult<()>,
    ) -> HostResult<bool> {
        let actor = match self.dropping.last().copied().flatten() {
            Some(actor) if self.current(host, actor) => actor,
            _ => {
                execute(&mut *host)?;
                return Ok(false);
            }
        };
        let projection = (self.hooks.drop)(actor);
        let projection = match projection {
            Some(projection) => projection,
            None => {
                execute(&mut *host)?;
                return Ok(false);
            }
        };
        let entity = host
            .core
            .entity_of(actor)
            .map_err(|_| NativeHostError::Fault("original death drop lost its actor".to_string()))?;
        let client = host.core.memory.read_pointer(
            host.core
                .memory
                .offset(entity, i64::from(self.profile.client.pointer))?,
        )?;
        let client = client.ok_or_else(|| NativeHostError::Fault("original death drop lost its client".to_string()))?;
        let item = match &projection.item {
            None => None,
            Some(item) => match self.items.iter().find(|row| &row.item == item && row.weapon) {
                Some(row) => Some(row.clone()),
                None => {
                    return Err(NativeHostError::Fault(
                        "selected death drop lacks an original weapon or int32 ammunition".to_string(),
                    ));
                }
            },
        };
        let ammo = match item.as_ref().and_then(|row| row.ammo.clone()) {
            None => None,
            Some(ammo) => Some(self.items.iter().find(|row| row.item == ammo).cloned().ok_or_else(|| {
                NativeHostError::Fault("selected death drop lacks an original weapon or int32 ammunition".to_string())
            })?),
        };
        let weapon = host.core.memory.offset(client, i64::from(self.profile.client.weapon))?;
        let mut words = Vec::new();
        if let Some(offset) = self.profile.client.ammo_index {
            let address = host.core.memory.offset(client, i64::from(offset))?;
            words.push((address, ammo.as_ref().map_or(0, |row| row.index as i32)));
        }
        if let Some(ammo) = &ammo {
            let address = host.core.memory.offset(
                client,
                i64::from(self.profile.client.inventory) + i64::from(ammo.index) * 4,
            )?;
            words.push((address, projection.ammo));
        }
        let saved_weapon = host.core.memory.copy(weapon, host.core.pointer_bytes())?;
        let mut saved_words = Vec::with_capacity(words.len());
        for (address, _) in &words {
            saved_words.push((*address, host.core.memory.copy(*address, 4)?));
        }
        host.core
            .memory
            .write_pointer(weapon, item.as_ref().map(|row| row.address))?;
        for (address, value) in &words {
            host.core.memory.write_i32(*address, *value)?;
        }
        let outcome = execute(&mut *host);
        if self.current(host, actor) && (host.core.entity_of(actor) == Ok(entity)) {
            for (address, bytes) in saved_words.into_iter().rev() {
                host.core.memory.write(address, &bytes)?;
            }
            host.core.memory.write(weapon, &saved_weapon)?;
        }
        outcome?;
        Ok(true)
    }

    /// Project a weapon descriptor around a closure, restoring afterwards.
    pub fn with_weapon<R>(
        &self,
        host: &mut SyntheticHost,
        actor: NativeActorId,
        item: Option<&ItemId>,
        run: impl FnOnce(&mut SyntheticHost) -> HostResult<R>,
    ) -> HostResult<R> {
        if !self.current(host, actor) {
            return Err(NativeHostError::Fault(
                "original weapon projection lost its actor".to_string(),
            ));
        }
        let item = match item {
            None => return run(&mut *host),
            Some(item) => item,
        };
        let descriptor = self
            .items
            .iter()
            .find(|row| &row.item == item && row.weapon)
            .ok_or_else(|| NativeHostError::Fault("selected weapon has no original descriptor".to_string()))?;
        let entity = host
            .core
            .entity_of(actor)
            .map_err(|_| NativeHostError::Fault("selected weapon has no original descriptor".to_string()))?;
        let client = host.core.memory.read_pointer(
            host.core
                .memory
                .offset(entity, i64::from(self.profile.client.pointer))?,
        )?;
        let client =
            client.ok_or_else(|| NativeHostError::Fault("original weapon projection lost its client".to_string()))?;
        let address = host.core.memory.offset(client, i64::from(self.profile.client.weapon))?;
        let previous = host.core.memory.read_pointer(address)?;
        host.core.memory.write_pointer(address, Some(descriptor.address))?;
        let outcome = run(&mut *host);
        if self.current(host, actor) && (host.core.entity_of(actor) == Ok(entity)) {
            host.core.memory.write_pointer(address, previous)?;
        }
        outcome
    }

    fn imported(
        &self,
        host: &mut SyntheticHost,
        offset: u32,
        values: &[GuestCallValue],
    ) -> HostResult<GuestCallResult> {
        let address = host.core.memory.read_pointer(host.core.at(offset)?)?;
        match address {
            Some(address) => host.invoke(address, values),
            None => Err(NativeHostError::Fault("original command import is null".to_string())),
        }
    }

    /// Read give arguments through the imported argc/argv routines.
    pub fn command_arguments(&self, host: &mut SyntheticHost) -> HostResult<Vec<String>> {
        let count = self.imported(host, self.profile.give.argc, &[])?;
        let count = match count {
            GuestCallResult::Value(GuestCallValue::Int32(value)) => value,
            _ => {
                return Err(NativeHostError::Fault("original command argc is invalid".to_string()));
            }
        };
        if !(0..=1024).contains(&count) {
            return Err(NativeHostError::Fault("original command argc is invalid".to_string()));
        }
        let mut arguments = Vec::with_capacity(count.max(1) as usize - 1);
        for index in 1..count {
            let value = self.imported(host, self.profile.give.argv, &[GuestCallValue::Int32(index)])?;
            match value {
                GuestCallResult::Value(GuestCallValue::Pointer(Some(address))) => {
                    arguments.push(host.core.read_c_string(address, 65536)?);
                }
                _ => {
                    return Err(NativeHostError::Fault("original command argv is null".to_string()));
                }
            }
        }
        Ok(arguments)
    }

    /// Close the service.
    pub fn close(&mut self) {
        self.closed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::CLASSIC_DIGEST;
    use super::*;

    fn profile() -> NativePrimaryCommandProfile {
        NativePrimaryCommandProfile {
            digest: CLASSIC_DIGEST.to_string(),
            abi: NativeAbi::WindowsI386,
            give: GiveProfile {
                entry: 0x100,
                weapons: 0x110,
                ammo: 0x120,
                unknown: NativeRegion {
                    entry: 0x130,
                    join: 0x140,
                },
                ammo_grants: vec![AmmoGrant {
                    entry: 0x150,
                    join: 0x160,
                    descriptor: GuestRegister::Rsi,
                    kind: GrantKind::Add,
                }],
                argc: 0x700,
                argv: 0x704,
            },
            drop: DropCommand {
                entry: 0x200,
                eligibility: NativeRegion {
                    entry: 0x210,
                    join: 0x220,
                },
            },
            client: CommandClient {
                pointer: 84,
                weapon: 0x80,
                ammo_index: Some(0x84),
                inventory: 0x100,
            },
            items: CommandItems {
                table: 0x400,
                stride: 32,
                count: 3,
                classname: 0,
                flags: 8,
                weapon_flag: 1,
                ammunition_flag: 2,
                icon: 12,
                ammo: ItemAmmo::Name { offset: 16, label: 20 },
            },
        }
    }

    fn hooks() -> CommandHooks {
        CommandHooks {
            give: Box::new(|_, _| {}),
            give_item: Box::new(|_, _| false),
            give_ammo: Box::new(|_, _, _| {}),
            drop: Box::new(|_| None),
        }
    }

    fn write_row(
        host: &mut SyntheticHost,
        profile: &NativePrimaryCommandProfile,
        index: u32,
        classname: &str,
        flags: u32,
        icon: &str,
        ammo_name: &str,
        label: &str,
    ) {
        let base = host
            .core
            .at(profile.items.table + index * profile.items.stride)
            .expect("row");
        let name = host.core.allocate_string(classname).expect("name");
        let icon = host.core.allocate_string(icon).expect("icon");
        let ammo = host.core.allocate_string(ammo_name).expect("ammo");
        let label = host.core.allocate_string(label).expect("label");
        host.core.memory.write_pointer(base, Some(name)).expect("write");
        host.core
            .memory
            .write_u32(host.core.memory.offset(base, 8).expect("f"), flags)
            .expect("flags");
        host.core
            .memory
            .write_pointer(host.core.memory.offset(base, 12).expect("i"), Some(icon))
            .expect("icon");
        host.core
            .memory
            .write_pointer(host.core.memory.offset(base, 16).expect("a"), Some(ammo))
            .expect("ammo");
        host.core
            .memory
            .write_pointer(host.core.memory.offset(base, 20).expect("l"), Some(label))
            .expect("label");
    }

    fn table_host(profile: &NativePrimaryCommandProfile) -> SyntheticHost {
        let mut host = SyntheticHost::synthetic(CLASSIC_DIGEST, 4, 0x2000).expect("host");
        write_row(&mut host, profile, 0, "", 0, "", "", "");
        write_row(
            &mut host,
            profile,
            1,
            "weapon_blaster",
            1,
            "w_blaster",
            "Shells",
            "Blaster",
        );
        write_row(&mut host, profile, 2, "ammo_shells", 2, "a_shells", "", "Shells");
        host
    }

    fn linked(host: &mut SyntheticHost) -> NativeActorId {
        let actor = host.core.spawn_actor(896, 512).expect("actor");
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host.core.client_of(actor).expect("client");
        host.core.set_client(entity, 84, client).expect("link");
        actor
    }

    #[test]
    fn resolves_item_tables_with_ammo_links() {
        let profile = profile();
        let mut host = table_host(&profile);
        let commands = NativePrimaryCommands::new(&mut host, profile, hooks()).expect("commands");
        assert_eq!(commands.inventory_slots().len(), 2);
        let blaster = commands.source_item(&"q2:weapon_blaster".to_string()).expect("blaster");
        assert_eq!(blaster.index, 1);
        assert_eq!(blaster.ammo.as_deref(), Some("q2:ammo_shells"));
        assert_eq!(commands.weapons().len(), 1);
        assert_eq!(commands.item_at(blaster.address).as_deref(), Some("q2:weapon_blaster"));
    }

    #[test]
    fn accounts_ammo_grants_around_originals() {
        use std::sync::{Arc, Mutex};
        let profile = profile();
        let mut host = table_host(&profile);
        let seen: Arc<Mutex<Vec<AmmoChange>>> = Arc::new(Mutex::new(Vec::new()));
        let capture = seen.clone();
        let commands_hooks = CommandHooks {
            give: Box::new(|_, _| {}),
            give_item: Box::new(|_, _| false),
            give_ammo: Box::new(move |_, _, change| capture.lock().expect("lock").push(change)),
            drop: Box::new(|_| None),
        };
        let mut commands = NativePrimaryCommands::new(&mut host, profile, commands_hooks).expect("commands");
        let actor = linked(&mut host);
        let shells = commands
            .source_item(&"q2:ammo_shells".to_string())
            .expect("shells")
            .clone();
        host.core.register_write(GuestRegister::Rsi, shells.address.offset);
        commands.begin_give(Some(actor));
        commands
            .ammo_grant(&mut host, 0, |host| {
                let entity = host.core.entity_of(actor).expect("entity");
                let client = host
                    .core
                    .memory
                    .read_pointer(host.core.memory.offset(entity, 84).expect("link"))
                    .expect("client")
                    .expect("client");
                let counter = host
                    .core
                    .memory
                    .offset(client, 0x100 + i64::from(shells.index) * 4)
                    .expect("counter");
                let before = host.core.memory.read_i32(counter).expect("before");
                host.core.memory.write_i32(counter, before + 10).expect("grant");
                Ok(())
            })
            .expect("grant");
        commands.end_give();
        let seen = seen.lock().expect("lock").clone();
        assert_eq!(
            seen,
            vec![AmmoChange {
                kind: GrantKind::Add,
                amount: 10
            }]
        );
    }

    #[test]
    fn projects_weapons_and_death_drops() {
        let profile = profile();
        let mut host = table_host(&profile);
        let commands_hooks = CommandHooks {
            give: Box::new(|_, _| {}),
            give_item: Box::new(|_, _| false),
            give_ammo: Box::new(|_, _, _| {}),
            drop: Box::new(|_| {
                Some(DeathDrop {
                    item: Some("q2:weapon_blaster".to_string()),
                    ammo: 5,
                })
            }),
        };
        let mut commands = NativePrimaryCommands::new(&mut host, profile, commands_hooks).expect("commands");
        let actor = linked(&mut host);
        let blaster = "q2:weapon_blaster".to_string();
        let entity = host.core.entity_of(actor).expect("entity");
        let client = host
            .core
            .memory
            .read_pointer(host.core.memory.offset(entity, 84).expect("link"))
            .expect("client")
            .expect("client");
        let weapon_slot = host.core.memory.offset(client, 0x80).expect("weapon");
        commands
            .with_weapon(&mut host, actor, Some(&blaster), |host| {
                let current = host.core.memory.read_pointer(weapon_slot).expect("read");
                assert!(current.is_some());
                Ok(())
            })
            .expect("project");
        assert!(host.core.memory.read_pointer(weapon_slot).expect("read").is_none());
        commands.begin_drop(Some(actor));
        let projected = commands
            .death_drop(&mut host, |host| {
                let current = host.core.memory.read_pointer(weapon_slot).expect("read");
                assert!(current.is_some());
                let ammo_counter = host.core.memory.offset(client, 0x100 + 2 * 4).expect("ammo");
                assert_eq!(host.core.memory.read_i32(ammo_counter).expect("count"), 5);
                let ammo_index = host.core.memory.offset(client, 0x84).expect("index");
                assert_eq!(host.core.memory.read_i32(ammo_index).expect("index"), 2);
                Ok(())
            })
            .expect("drop");
        commands.end_drop();
        assert!(projected);
        assert!(host.core.memory.read_pointer(weapon_slot).expect("read").is_none());
    }

    #[test]
    fn reads_give_arguments_through_imports() {
        let profile = profile();
        let mut host = table_host(&profile);
        let first = host.core.allocate_string("blaster").expect("arg");
        let argc = host.core.at(0x700).expect("argc");
        let argv = host.core.at(0x704).expect("argv");
        let argc_impl = host.core.at(0x900).expect("impl");
        let argv_impl = host.core.at(0x910).expect("impl");
        host.core.memory.write_pointer(argc, Some(argc_impl)).expect("write");
        host.core.memory.write_pointer(argv, Some(argv_impl)).expect("write");
        host.on_invoke(
            argc_impl,
            Box::new(|_, _| Ok(GuestCallResult::Value(GuestCallValue::Int32(2)))),
        );
        host.on_invoke(
            argv_impl,
            Box::new(move |_, _| Ok(GuestCallResult::Value(GuestCallValue::Pointer(Some(first))))),
        );
        let mut commands = NativePrimaryCommands::new(&mut host, profile, hooks()).expect("commands");
        let actor = linked(&mut host);
        commands.begin_give(Some(actor));
        let arguments = commands.command_arguments(&mut host).expect("args");
        assert_eq!(arguments, vec!["blaster".to_string()]);
        assert!(!commands.unknown_give(&mut host).expect("unknown"));
        commands.end_give();
        commands.close();
    }
}
