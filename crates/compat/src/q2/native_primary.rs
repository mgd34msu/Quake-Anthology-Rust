//! Port of `src/compat/q2/native-primary.ts`.
//! Bridges builtin primary profiles: service tables, world profiles and declarations.

use std::collections::HashSet;

use qa_guest::core::contracts::{
    GuestCallSignature, GuestFieldLayout, GuestLayout, GuestRegister, GuestStorage, GuestValueLayout, NativeAbi,
    NativeCallAbi,
};
use qa_world::combat::ItemId;

use super::native_combat_call::{
    read_native_combat_call, stock_native_combat_call, validate_native_combat_call, CombatOperation, NativeCombatCall,
};
use super::native_primary_command_profile::native_primary_command_profile;
use super::native_primary_commands::NativePrimaryCommandProfile;
use super::native_primary_drop::NativePrimaryDropProfile;
use super::native_primary_drop_profile::native_primary_drop_profile;
use super::native_primary_inventory::NativePrimaryInventoryProfile;
use super::native_primary_inventory_profile::native_primary_inventory_profile;
use super::native_primary_pickups::{
    AmmoSupply, NativePickupGrant, NativePickupProfile, PickupConsumer, PickupEntity, PickupItems, PickupResource,
    PickupSupply, PickupSupplyProfile, PickupTime, ProtectionChannel, TimeStorage,
};
use super::native_primary_player::NativePrimaryPlayerProfile;
use super::native_primary_player_profile::native_primary_player_profile;
use super::native_primary_reader::{
    namespaced, native_offset, read_guest_layout, NativeRegion, Reader, CLASSIC_DIGEST, RETAIL_DIGEST,
};
use super::native_primary_weapon_profile::native_primary_weapon_profile;
use super::native_primary_weapons::NativePrimaryWeaponProfile;

// ---------------------------------------------------------------------------
// Classic world
// ---------------------------------------------------------------------------

/// Classic combat calls.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicWorldCalls {
    /// Pain call.
    pub pain: NativeCombatCall,
    /// Death call.
    pub death: NativeCombatCall,
    /// Damage call.
    pub damage: NativeCombatCall,
    /// Regular armor call.
    pub regular_armor: NativeCombatCall,
    /// Power armor call.
    pub power_armor: NativeCombatCall,
}

/// Classic game tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassicGame {
    /// Base game.
    Base,
    /// Xatrix mission pack.
    Xatrix,
    /// Rogue mission pack.
    Rogue,
    /// Capture the flag.
    Ctf,
}

impl ClassicGame {
    /// Profile label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Xatrix => "xatrix",
            Self::Rogue => "rogue",
            Self::Ctf => "ctf",
        }
    }
}

/// Classic combat entity fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWorldFields {
    /// Health offset.
    pub health: u32,
    /// Damageable offset.
    pub damageable: u32,
    /// Flags offset.
    pub flags: u32,
    /// Mass offset.
    pub mass: u32,
    /// Velocity offset.
    pub velocity: u32,
    /// Pain callback offset.
    pub pain: u32,
    /// Die callback offset.
    pub die: u32,
}

/// Classic combat client fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWorldClient {
    /// Inventory offset.
    pub inventory: u32,
    /// Inventory slot count.
    pub inventory_count: u32,
    /// Max grenades offset.
    pub max_grenades: u32,
    /// Invincible frame offset.
    pub invincible_frame: u32,
    /// Userinfo offset.
    pub userinfo: u32,
    /// Userinfo length.
    pub userinfo_bytes: u32,
    /// View angles offset.
    pub view_angles: u32,
}

/// Classic combat entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWorldEntries {
    /// Damage entry RVA.
    pub damage: u32,
    /// Power armor entry RVA.
    pub power_armor: u32,
    /// Regular armor entry RVA.
    pub regular_armor: u32,
    /// Spawn entry RVA.
    pub spawn: u32,
    /// Free entry RVA.
    pub free: u32,
}

/// Classic combat globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWorldGlobals {
    /// Level frame RVA.
    pub level_frame: u32,
    /// Item list RVA.
    pub item_list: u32,
    /// Item stride.
    pub item_bytes: u32,
}

/// Classic combat item slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWorldItems {
    /// Jacket armor slot.
    pub jacket: u32,
    /// Combat armor slot.
    pub combat: u32,
    /// Body armor slot.
    pub body: u32,
    /// Screen slot.
    pub screen: u32,
    /// Shield slot.
    pub shield: u32,
    /// Cells slot.
    pub cells: u32,
    /// Grenades slot.
    pub grenades: u32,
}

/// Classic armor item fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicItemFields {
    /// Class name offset.
    pub class_name: u32,
    /// Armor info offset.
    pub armor_info: u32,
}

/// Classic armor info fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicArmorInfo {
    /// Normal protection offset.
    pub normal_protection: u32,
    /// Energy protection offset.
    pub energy_protection: u32,
}

/// Classic regular armor priorities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicArmor {
    /// Priority slots.
    pub regular: Vec<u32>,
    /// Empty tier slot.
    pub empty: u32,
}

/// Classic flag masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicFlags {
    /// Invulnerable mask.
    pub invulnerable: u32,
    /// Notarget mask.
    pub notarget: u32,
    /// No-knockback mask.
    pub no_knockback: u32,
    /// Power armor mask.
    pub power_armor: u32,
}

/// Classic team skin offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicTeams {
    /// Model offset.
    pub model: u32,
    /// Skin offset.
    pub skin: u32,
}

/// Unnamed source item identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnnamedItem {
    /// Table index.
    pub index: u32,
    /// Display label.
    pub label: String,
    /// Canonical item.
    pub item: ItemId,
}

/// Classic source inventory table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicInventoryTable {
    /// Row count.
    pub count: u32,
    /// Class name offset.
    pub class_name: u32,
    /// Label offset.
    pub label: u32,
    /// Flags offset.
    pub flags: u32,
    /// Ammo flag bit.
    pub ammo_flag: u32,
    /// Tag offset.
    pub tag: u32,
    /// Capacity offsets.
    pub capacities: Vec<u32>,
    /// Unnamed identities.
    pub unnamed: Vec<UnnamedItem>,
    /// Empty slot index.
    pub empty_index: u32,
    /// Whether slot zero is the sentinel.
    pub sentinel: bool,
}

/// Classic primary world profile: combat profile plus the source item table.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicPrimaryWorldProfile {
    /// Combat calls.
    pub calls: ClassicWorldCalls,
    /// Artifact digest.
    pub digest: String,
    /// Game tag.
    pub game: ClassicGame,
    /// Entity record length.
    pub entity_bytes: u32,
    /// Combat entity fields.
    pub fields: ClassicWorldFields,
    /// Combat client fields.
    pub client: ClassicWorldClient,
    /// Combat entries.
    pub entries: ClassicWorldEntries,
    /// Combat globals.
    pub globals: ClassicWorldGlobals,
    /// Combat item slots.
    pub items: ClassicWorldItems,
    /// Armor item fields.
    pub item_fields: ClassicItemFields,
    /// Armor info fields.
    pub armor_info: ClassicArmorInfo,
    /// Regular armor.
    pub armor: ClassicArmor,
    /// Flag masks.
    pub flags: ClassicFlags,
    /// Team offsets.
    pub teams: ClassicTeams,
    /// Source inventory table.
    pub inventory_table: ClassicInventoryTable,
}

/// Builtin classic world profile for a digest, or `None` when unknown.
#[must_use]
pub fn classic_primary_world_profile(digest: &str) -> Option<ClassicPrimaryWorldProfile> {
    if digest != CLASSIC_DIGEST {
        return None;
    }
    Some(ClassicPrimaryWorldProfile {
        calls: ClassicWorldCalls {
            pain: stock_native_combat_call(CombatOperation::Pain, Some(NativeAbi::WindowsI386)),
            death: stock_native_combat_call(CombatOperation::Death, Some(NativeAbi::WindowsI386)),
            damage: stock_native_combat_call(CombatOperation::Damage, None),
            regular_armor: stock_native_combat_call(CombatOperation::RegularArmor, None),
            power_armor: stock_native_combat_call(CombatOperation::PowerArmor, None),
        },
        digest: CLASSIC_DIGEST.to_string(),
        game: ClassicGame::Xatrix,
        entity_bytes: 896,
        fields: ClassicWorldFields {
            health: 480,
            damageable: 512,
            flags: 264,
            mass: 400,
            velocity: 376,
            pain: 452,
            die: 456,
        },
        client: ClassicWorldClient {
            inventory: 740,
            inventory_count: 256,
            max_grenades: 1776,
            invincible_frame: 3728,
            userinfo: 188,
            userinfo_bytes: 512,
            view_angles: 3652,
        },
        entries: ClassicWorldEntries {
            damage: 0x5050,
            power_armor: 0x5580,
            regular_armor: 0x5760,
            spawn: 0x19090,
            free: 0x19140,
        },
        globals: ClassicWorldGlobals {
            level_frame: 0x76800,
            item_list: 0x4b828,
            item_bytes: 76,
        },
        items: ClassicWorldItems {
            jacket: 3,
            combat: 2,
            body: 1,
            screen: 5,
            shield: 6,
            cells: 23,
            grenades: 12,
        },
        item_fields: ClassicItemFields {
            class_name: 0,
            armor_info: 64,
        },
        armor_info: ClassicArmorInfo {
            normal_protection: 8,
            energy_protection: 12,
        },
        armor: ClassicArmor {
            regular: vec![3, 2, 1],
            empty: 1,
        },
        flags: ClassicFlags {
            invulnerable: 16,
            notarget: 32,
            no_knockback: 2048,
            power_armor: 4096,
        },
        teams: ClassicTeams { model: 64, skin: 128 },
        inventory_table: ClassicInventoryTable {
            count: 48,
            class_name: 0,
            label: 40,
            flags: 56,
            ammo_flag: 2,
            tag: 68,
            capacities: vec![0x6e4, 0x6e8, 0x6ec, 0x6f0, 0x6f4, 0x6f8, 0x6fc, 0x700],
            unnamed: vec![
                UnnamedItem {
                    index: 0,
                    label: String::new(),
                    item: "q2:none".to_string(),
                },
                UnnamedItem {
                    index: 47,
                    label: "Health".to_string(),
                    item: "q2:item_health".to_string(),
                },
            ],
            empty_index: 0,
            sentinel: true,
        },
    })
}

/// Validate the classic combat surface: calls, record sizes and field bounds.
pub fn validate_classic_combat(profile: &ClassicPrimaryWorldProfile) -> Result<(), String> {
    let abi = NativeAbi::WindowsI386;
    validate_native_combat_call(&profile.calls.pain, CombatOperation::Pain, abi)?;
    validate_native_combat_call(&profile.calls.death, CombatOperation::Death, abi)?;
    validate_native_combat_call(&profile.calls.damage, CombatOperation::Damage, abi)?;
    validate_native_combat_call(&profile.calls.regular_armor, CombatOperation::RegularArmor, abi)?;
    validate_native_combat_call(&profile.calls.power_armor, CombatOperation::PowerArmor, abi)?;
    if profile.entity_bytes < 260 || profile.globals.item_bytes < 4 || profile.client.inventory_count < 1 {
        return Err("classic combat source record sizes are invalid".to_string());
    }
    for (offset, extra) in [
        (profile.fields.health, 4),
        (profile.fields.damageable, 4),
        (profile.fields.flags, 4),
        (profile.fields.mass, 4),
        (profile.fields.velocity, 12),
        (profile.fields.pain, 4),
        (profile.fields.die, 4),
    ] {
        if offset % 4 != 0 || offset + extra > profile.entity_bytes {
            return Err("classic combat field is outside its source edict".to_string());
        }
    }
    for offset in [
        profile.client.inventory,
        profile.client.max_grenades,
        profile.client.invincible_frame,
        profile.client.view_angles,
    ] {
        if offset % 4 != 0 {
            return Err("classic combat client field is unaligned".to_string());
        }
    }
    if profile.client.userinfo_bytes < 1 {
        return Err("classic combat userinfo requires a bounded source string".to_string());
    }
    if profile.client.inventory as u64 + profile.client.inventory_count as u64 * 4 > u64::from(u32::MAX) {
        return Err("classic combat source field exceeds its address range".to_string());
    }
    for offset in [profile.item_fields.class_name, profile.item_fields.armor_info] {
        if offset % 4 != 0 || offset + 4 > profile.globals.item_bytes {
            return Err("classic armor item field exceeds its original item record".to_string());
        }
    }
    if profile.armor_info.normal_protection % 4 != 0 || profile.armor_info.energy_protection % 4 != 0 {
        return Err("classic armor information is unaligned".to_string());
    }
    if profile.armor_info.normal_protection == profile.armor_info.energy_protection
        || profile.item_fields.class_name == profile.item_fields.armor_info
    {
        return Err("classic armor fields overlap".to_string());
    }
    if profile.armor.regular.is_empty()
        || profile.armor.regular.iter().collect::<HashSet<_>>().len() != profile.armor.regular.len()
        || !profile.armor.regular.contains(&profile.armor.empty)
    {
        return Err("classic regular armor requires distinct source priorities and an admitted empty tier".to_string());
    }
    for index in [
        profile.items.jacket,
        profile.items.combat,
        profile.items.body,
        profile.items.screen,
        profile.items.shield,
        profile.items.cells,
        profile.items.grenades,
    ]
    .into_iter()
    .chain(profile.armor.regular.iter().copied())
    {
        if index < 1 || index >= profile.client.inventory_count {
            return Err("classic combat inventory index exceeds its declared source storage".to_string());
        }
    }
    Ok(())
}

/// Read a classic world profile subtree and validate it.
pub fn read_classic_world_profile(reader: &Reader, digest: String) -> ClassicPrimaryWorldProfile {
    let fields = reader.field("fields");
    let client = reader.field("client");
    let entries = reader.field("entries");
    let globals = reader.field("globals");
    let items = reader.field("items");
    let item_fields = reader.field("itemFields");
    let armor_info = reader.field("armorInfo");
    let flags = reader.field("flags");
    let teams = reader.field("teams");
    let armor = reader.field("armor");
    let table = reader.field("inventoryTable");
    let calls = reader.field("calls");
    let abi = NativeAbi::WindowsI386;
    let game = match reader.field("game").choice_index(&["base", "xatrix", "rogue", "ctf"]) {
        0 => ClassicGame::Base,
        1 => ClassicGame::Xatrix,
        2 => ClassicGame::Rogue,
        _ => ClassicGame::Ctf,
    };
    let profile = ClassicPrimaryWorldProfile {
        calls: ClassicWorldCalls {
            pain: read_native_combat_call(&calls.field("pain"), CombatOperation::Pain, abi),
            death: read_native_combat_call(&calls.field("death"), CombatOperation::Death, abi),
            damage: read_native_combat_call(&calls.field("damage"), CombatOperation::Damage, abi),
            regular_armor: read_native_combat_call(&calls.field("regularArmor"), CombatOperation::RegularArmor, abi),
            power_armor: read_native_combat_call(&calls.field("powerArmor"), CombatOperation::PowerArmor, abi),
        },
        digest,
        game,
        entity_bytes: reader.field("entityBytes").integer_u32(1),
        fields: ClassicWorldFields {
            health: native_offset(&fields.field("health")),
            damageable: native_offset(&fields.field("damageable")),
            flags: native_offset(&fields.field("flags")),
            mass: native_offset(&fields.field("mass")),
            velocity: native_offset(&fields.field("velocity")),
            pain: native_offset(&fields.field("pain")),
            die: native_offset(&fields.field("die")),
        },
        client: ClassicWorldClient {
            inventory: native_offset(&client.field("inventory")),
            inventory_count: client.field("inventoryCount").integer_u32(1),
            max_grenades: native_offset(&client.field("maxGrenades")),
            invincible_frame: native_offset(&client.field("invincibleFrame")),
            userinfo: native_offset(&client.field("userinfo")),
            userinfo_bytes: client.field("userinfoBytes").integer_u32(1),
            view_angles: native_offset(&client.field("viewAngles")),
        },
        entries: ClassicWorldEntries {
            damage: native_offset(&entries.field("damage")),
            power_armor: native_offset(&entries.field("powerArmor")),
            regular_armor: native_offset(&entries.field("regularArmor")),
            spawn: native_offset(&entries.field("spawn")),
            free: native_offset(&entries.field("free")),
        },
        globals: ClassicWorldGlobals {
            level_frame: native_offset(&globals.field("levelFrame")),
            item_list: native_offset(&globals.field("itemList")),
            item_bytes: globals.field("itemBytes").integer_u32(1),
        },
        items: ClassicWorldItems {
            jacket: native_offset(&items.field("jacket")),
            combat: native_offset(&items.field("combat")),
            body: native_offset(&items.field("body")),
            screen: native_offset(&items.field("screen")),
            shield: native_offset(&items.field("shield")),
            cells: native_offset(&items.field("cells")),
            grenades: native_offset(&items.field("grenades")),
        },
        item_fields: ClassicItemFields {
            class_name: native_offset(&item_fields.field("className")),
            armor_info: native_offset(&item_fields.field("armorInfo")),
        },
        armor_info: ClassicArmorInfo {
            normal_protection: native_offset(&armor_info.field("normalProtection")),
            energy_protection: native_offset(&armor_info.field("energyProtection")),
        },
        flags: ClassicFlags {
            invulnerable: flags.field("invulnerable").integer_u32(0),
            notarget: flags.field("notarget").integer_u32(0),
            no_knockback: flags.field("noKnockback").integer_u32(0),
            power_armor: flags.field("powerArmor").integer_u32(0),
        },
        teams: ClassicTeams {
            model: teams.field("model").integer_u32(0),
            skin: teams.field("skin").integer_u32(0),
        },
        armor: ClassicArmor {
            regular: armor.field("regular").list(native_offset),
            empty: native_offset(&armor.field("empty")),
        },
        inventory_table: ClassicInventoryTable {
            count: table.field("count").integer_u32(1),
            class_name: native_offset(&table.field("className")),
            label: native_offset(&table.field("label")),
            flags: native_offset(&table.field("flags")),
            ammo_flag: table.field("ammoFlag").integer_u32(1),
            tag: native_offset(&table.field("tag")),
            capacities: table.field("capacities").list(native_offset),
            unnamed: table.field("unnamed").list(|value| UnnamedItem {
                index: native_offset(&value.field("index")),
                label: value.field("label").string(),
                item: namespaced(&value.field("item")),
            }),
            empty_index: table.field("emptyIndex").integer_u32(0),
            sentinel: table.field("sentinel").boolean(),
        },
    };
    if let Err(message) = validate_classic_combat(&profile) {
        reader.fail(&message);
    }
    if profile.inventory_table.count > profile.client.inventory_count
        || profile.inventory_table.empty_index >= profile.inventory_table.count
    {
        table.fail("item table exceeds original inventory storage");
    }
    for index in [
        profile.items.jacket,
        profile.items.combat,
        profile.items.body,
        profile.items.screen,
        profile.items.shield,
        profile.items.cells,
        profile.items.grenades,
    ]
    .into_iter()
    .chain(profile.armor.regular.iter().copied())
    {
        if index >= profile.inventory_table.count {
            table.fail("combat item is outside the original source table");
        }
    }
    for field in [
        profile.inventory_table.class_name,
        profile.inventory_table.label,
        profile.inventory_table.flags,
        profile.inventory_table.tag,
    ] {
        if field + 4 > profile.globals.item_bytes {
            table.fail("item field exceeds declared stride");
        }
    }
    let mut indices = HashSet::new();
    let mut names = HashSet::new();
    for entry in &profile.inventory_table.unnamed {
        if entry.index >= profile.inventory_table.count
            || !indices.insert(entry.index)
            || !names.insert(entry.item.clone())
        {
            table.fail("unnamed item identities must be unique source slots");
        }
    }
    profile
}

// ---------------------------------------------------------------------------
// Rerelease world
// ---------------------------------------------------------------------------

/// Rerelease combat calls.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseWorldCalls {
    /// Pain call.
    pub pain: NativeCombatCall,
    /// Death call.
    pub death: NativeCombatCall,
    /// Deferred pain call.
    pub process_pain: NativeCombatCall,
    /// Damage call.
    pub damage: NativeCombatCall,
    /// Power armor call.
    pub power_armor: NativeCombatCall,
}

/// Scalar storage of one armor location.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocationStorage {
    /// Guest pointer.
    Pointer,
    /// Signed 32-bit.
    Int32,
    /// Unsigned 32-bit.
    Uint32,
}

impl LocationStorage {
    /// Profile label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pointer => "pointer",
            Self::Int32 => "int32",
            Self::Uint32 => "uint32",
        }
    }

    /// Width at the rerelease 8-byte pointer width.
    #[must_use]
    pub const fn width(self) -> u32 {
        match self {
            Self::Pointer => 8,
            Self::Int32 | Self::Uint32 => 4,
        }
    }
}

/// One armor value location: register or stack slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RereleaseWorldLocation {
    /// Register location.
    Register {
        /// Register.
        register: GuestRegister,
        /// Storage.
        storage: LocationStorage,
    },
    /// Stack location.
    Stack {
        /// Stack offset.
        offset: u32,
        /// Storage.
        storage: LocationStorage,
    },
}

/// Rerelease world client profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseWorldClient {
    /// Artifact authority digest.
    pub authority_digest: String,
    /// Client record layout.
    pub layout: GuestLayout,
    /// Inventory slot count.
    pub inventory_count: u32,
    /// Ammo slot count.
    pub ammo_count: u32,
}

/// Rerelease world entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseEntries {
    /// Spawn entry RVA.
    pub spawn: u32,
    /// Free entry RVA.
    pub free: u32,
    /// Damage entry RVA.
    pub damage: u32,
    /// Power armor entry RVA.
    pub power_armor: u32,
    /// Deferred pain entry RVA.
    pub process_pain: u32,
    /// Clock RVA.
    pub time: u32,
    /// Regular armor region.
    pub regular_armor: NativeRegion,
}

/// Rerelease regular armor locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseRegularArmor {
    /// Target location.
    pub target: RereleaseWorldLocation,
    /// Amount location.
    pub amount: RereleaseWorldLocation,
    /// Point location.
    pub point: RereleaseWorldLocation,
    /// Normal location.
    pub normal: RereleaseWorldLocation,
    /// Flags location.
    pub flags: RereleaseWorldLocation,
    /// Result location.
    pub result: RereleaseWorldLocation,
    /// Register repair moves.
    pub repair: Vec<ArmorRepair>,
}

/// One armor register repair move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmorRepair {
    /// Source location.
    pub source: RereleaseWorldLocation,
    /// Target location.
    pub target: RereleaseWorldLocation,
}

/// Rerelease monster accumulators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseMonster {
    /// Attacker offset.
    pub attacker: u32,
    /// Inflictor offset.
    pub inflictor: u32,
    /// Blood offset.
    pub blood: u32,
    /// Knockback offset.
    pub knockback: u32,
    /// Point offset.
    pub point: u32,
    /// Cause offset.
    pub cause: u32,
    /// Invincible time offset.
    pub invincible_time: u32,
}

/// Source of one inventory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventorySource {
    /// Classname lookup.
    Classname {
        /// Class name.
        name: String,
    },
    /// Table index.
    Index {
        /// Index.
        index: u32,
    },
    /// Remaining unmapped row.
    Remaining,
}

/// Capacity of one inventory row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InventoryCapacity {
    /// Ammo slot capacity.
    Ammo {
        /// Ammo source index.
        source_index: u32,
    },
    /// Fixed capacity.
    Fixed {
        /// Count.
        count: u32,
    },
}

/// One rerelease inventory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseInventoryRow {
    /// Canonical item.
    pub item: ItemId,
    /// Row source.
    pub source: InventorySource,
    /// Row capacity.
    pub capacity: InventoryCapacity,
}

/// Rerelease armor metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseArmor {
    /// Armor table RVA.
    pub table: u32,
    /// Table stride.
    pub stride: u32,
    /// Normal protection offset.
    pub normal: u32,
    /// Energy protection offset.
    pub energy: u32,
    /// Regular armor items.
    pub regular: Vec<ItemId>,
    /// Empty tier item.
    pub empty: ItemId,
    /// Screen item.
    pub screen: ItemId,
    /// Shield item.
    pub shield: ItemId,
    /// Cells item.
    pub cells: ItemId,
    /// Cells index.
    pub cells_index: u32,
}

/// Rerelease flag masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseFlags {
    /// Godmode mask.
    pub godmode: u64,
    /// Notarget mask.
    pub notarget: u64,
    /// No-knockback mask.
    pub no_knockback: u64,
    /// Power armor mask.
    pub power_armor: u64,
}

/// Rerelease movement body hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleaseMovementBody {
    /// Dimensions hook RVA.
    pub dimensions: u32,
    /// Trace hook RVA.
    pub trace: u32,
    /// Movement global RVA.
    pub movement_global: u32,
}

/// One movement speed load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedLoad {
    /// Next-instruction RVA.
    pub next: u32,
    /// Register code.
    pub register: u8,
}

/// Rerelease movement metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseMovement {
    /// Optional body hooks.
    pub body: Option<RereleaseMovementBody>,
    /// Game API RVA.
    pub game_api: u32,
    /// Pmove RVA.
    pub pmove: u32,
    /// Speed loads.
    pub speed_loads: Vec<SpeedLoad>,
}

/// Rerelease primary world profile.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleasePrimaryWorldProfile {
    /// Combat calls.
    pub calls: RereleaseWorldCalls,
    /// Artifact digest.
    pub digest: String,
    /// Edict record layout.
    pub edict: GuestLayout,
    /// Client profile.
    pub client: RereleaseWorldClient,
    /// World entries.
    pub entries: RereleaseEntries,
    /// Regular armor locations.
    pub regular_armor: RereleaseRegularArmor,
    /// Monster accumulators.
    pub monster: RereleaseMonster,
    /// Inventory roster.
    pub inventory: Vec<RereleaseInventoryRow>,
    /// Armor metadata.
    pub armor: RereleaseArmor,
    /// Flag masks.
    pub flags: RereleaseFlags,
    /// Movement metadata.
    pub movement: RereleaseMovement,
}

/// Retail `g_items.cpp` item list classnames in table order.
const RETAIL_CLASSNAMES: [&str; 82] = [
    "item_armor_body",
    "item_armor_combat",
    "item_armor_jacket",
    "item_armor_shard",
    "item_power_screen",
    "item_power_shield",
    "weapon_grapple",
    "weapon_blaster",
    "weapon_chainfist",
    "weapon_shotgun",
    "weapon_supershotgun",
    "weapon_machinegun",
    "weapon_etf_rifle",
    "weapon_chaingun",
    "ammo_grenades",
    "ammo_trap",
    "ammo_tesla",
    "weapon_grenadelauncher",
    "weapon_proxlauncher",
    "weapon_rocketlauncher",
    "weapon_hyperblaster",
    "weapon_boomer",
    "weapon_plasmabeam",
    "weapon_railgun",
    "weapon_phalanx",
    "weapon_bfg",
    "weapon_disintegrator",
    "ammo_shells",
    "ammo_bullets",
    "ammo_cells",
    "ammo_rockets",
    "ammo_slugs",
    "ammo_magslug",
    "ammo_flechettes",
    "ammo_prox",
    "ammo_nuke",
    "ammo_disruptor",
    "item_quad",
    "item_quadfire",
    "item_invulnerability",
    "item_invisibility",
    "item_silencer",
    "item_breather",
    "item_enviro",
    "item_ancient_head",
    "item_legacy_head",
    "item_adrenaline",
    "item_bandolier",
    "item_pack",
    "item_ir_goggles",
    "item_double",
    "item_sphere_vengeance",
    "item_sphere_hunter",
    "item_sphere_defender",
    "item_doppleganger",
    "key_data_cd",
    "key_power_cube",
    "key_explosive_charges",
    "key_yellow_key",
    "key_power_core",
    "key_pyramid",
    "key_data_spinner",
    "key_pass",
    "key_blue_key",
    "key_red_key",
    "key_green_key",
    "key_commander_head",
    "key_airstrike_target",
    "key_nuke_container",
    "key_nuke",
    "item_health_small",
    "item_health",
    "item_health_large",
    "item_health_mega",
    "item_flag_team1",
    "item_flag_team2",
    "item_tech1",
    "item_tech2",
    "item_tech3",
    "item_tech4",
    "item_flashlight",
    "item_compass",
];

/// Retail ammo slot per classname.
fn retail_ammo_slot(name: &str) -> Option<u32> {
    match name {
        "ammo_bullets" => Some(0),
        "ammo_shells" => Some(1),
        "ammo_rockets" => Some(2),
        "ammo_grenades" => Some(3),
        "ammo_cells" => Some(4),
        "ammo_slugs" => Some(5),
        "ammo_magslug" => Some(6),
        "ammo_trap" => Some(7),
        "ammo_flechettes" => Some(8),
        "ammo_tesla" => Some(9),
        "ammo_disruptor" => Some(10),
        "ammo_prox" => Some(11),
        _ => None,
    }
}

fn layout_field(name: &str, byte_offset: usize, storage: GuestStorage, count: usize) -> GuestFieldLayout {
    GuestFieldLayout {
        name: name.to_string(),
        byte_offset,
        storage,
        count,
    }
}

/// Retail observed client fields: the exact explicit table from the donor
/// client profile. The `shared.*` public prefix spread belongs to the layout
/// owner lane; admission consumes only these explicit fields.
fn retail_client_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-rerelease-retail:observed-client-fields",
        7344,
        8,
        8,
        vec![
            layout_field("resp.score", 0x17c8, GuestStorage::Int32, 1),
            layout_field("resp.ctf_team", 0x17dc, GuestStorage::Int32, 1),
            layout_field("pers.selected_item", 2672, GuestStorage::Int32, 1),
            layout_field("pers.inventory", 2688, GuestStorage::Int32, 84),
            layout_field("pers.max_ammo", 3024, GuestStorage::Int16, 12),
            layout_field("pers.weapon", 3048, GuestStorage::Pointer, 1),
            layout_field("newweapon", 6296, GuestStorage::Pointer, 1),
            layout_field("v_angle", 6552, GuestStorage::Float32, 3),
            layout_field("invincible_time", 6672, GuestStorage::Int64, 1),
            layout_field("no_weapon_chains", 7016, GuestStorage::Uint8, 1),
        ],
    )
}

/// Retail edict layout with offsets verified against the donor packing rules
/// (`inuse` 1376 and `spawn_count` 1472 cross-checked through the
/// `spawnflags` and `count` offsets). Admission bounds use the byte length.
fn retail_edict_layout() -> GuestLayout {
    GuestLayout::new(
        "q2-rerelease-x64:edict_t_private_prefix",
        3688,
        8,
        8,
        vec![
            layout_field("shared.client", 120, GuestStorage::Pointer, 1),
            layout_field("shared.inuse", 1376, GuestStorage::Uint8, 1),
            layout_field("shared.linked", 1377, GuestStorage::Uint8, 1),
            layout_field("shared.linkcount", 1380, GuestStorage::Int32, 1),
            layout_field("shared.areanum", 1384, GuestStorage::Int32, 1),
            layout_field("shared.areanum2", 1388, GuestStorage::Int32, 1),
            layout_field("shared.svflags", 1392, GuestStorage::Uint32, 1),
            layout_field("shared.solid", 1456, GuestStorage::Uint8, 1),
            layout_field("shared.clipmask", 1460, GuestStorage::Uint32, 1),
            layout_field("shared.owner", 1464, GuestStorage::Pointer, 1),
            layout_field("spawn_count", 1472, GuestStorage::Int32, 1),
            layout_field("movetype", 1476, GuestStorage::Int32, 1),
            layout_field("flags", 1480, GuestStorage::Uint64, 1),
            layout_field("model", 1488, GuestStorage::Pointer, 1),
            layout_field("freetime", 1496, GuestStorage::Int64, 1),
            layout_field("message", 1504, GuestStorage::Pointer, 1),
            layout_field("classname", 1512, GuestStorage::Pointer, 1),
            layout_field("spawnflags", 1520, GuestStorage::Uint32, 1),
            layout_field("health", 1912, GuestStorage::Int32, 1),
            layout_field("max_health", 1916, GuestStorage::Int32, 1),
            layout_field("gib_health", 1920, GuestStorage::Int32, 1),
            layout_field("viewheight", 1952, GuestStorage::Int32, 1),
            layout_field("count", 1976, GuestStorage::Int32, 1),
        ],
    )
}

/// Builtin rerelease world profile for a digest, or `None` when unknown.
#[must_use]
pub fn rerelease_primary_world_profile(digest: &str) -> Option<RereleasePrimaryWorldProfile> {
    if digest != RETAIL_DIGEST {
        return None;
    }
    let abi = NativeAbi::WindowsX86_64;
    let mut inventory = Vec::with_capacity(84);
    inventory.push(RereleaseInventoryRow {
        item: "q2:none".to_string(),
        source: InventorySource::Index { index: 0 },
        capacity: InventoryCapacity::Fixed { count: 0 },
    });
    for name in RETAIL_CLASSNAMES {
        inventory.push(RereleaseInventoryRow {
            item: format!("q2:{name}"),
            source: InventorySource::Classname { name: name.to_string() },
            capacity: match retail_ammo_slot(name) {
                Some(source_index) => InventoryCapacity::Ammo { source_index },
                None => InventoryCapacity::Fixed { count: 0x7fff_ffff },
            },
        });
    }
    inventory.push(RereleaseInventoryRow {
        item: "q2:item_tag_token".to_string(),
        source: InventorySource::Remaining,
        capacity: InventoryCapacity::Fixed { count: 0x7fff_ffff },
    });
    Some(RereleasePrimaryWorldProfile {
        calls: RereleaseWorldCalls {
            pain: stock_native_combat_call(CombatOperation::Pain, Some(abi)),
            death: stock_native_combat_call(CombatOperation::Death, Some(abi)),
            process_pain: stock_native_combat_call(CombatOperation::DeferredReaction, Some(abi)),
            damage: stock_native_combat_call(CombatOperation::Damage, Some(abi)),
            power_armor: stock_native_combat_call(CombatOperation::PowerArmor, Some(abi)),
        },
        digest: RETAIL_DIGEST.to_string(),
        edict: retail_edict_layout(),
        client: RereleaseWorldClient {
            authority_digest: RETAIL_DIGEST.to_string(),
            layout: retail_client_layout(),
            inventory_count: 84,
            ammo_count: 12,
        },
        entries: RereleaseEntries {
            spawn: 0x964b0,
            free: 0x96600,
            damage: 0x5cae0,
            power_armor: 0x5c100,
            process_pain: 0x76e20,
            time: 0x241b28,
            regular_armor: NativeRegion {
                entry: 0x5d022,
                join: 0x5d154,
            },
        },
        regular_armor: RereleaseRegularArmor {
            target: RereleaseWorldLocation::Register {
                register: GuestRegister::Rdi,
                storage: LocationStorage::Pointer,
            },
            amount: RereleaseWorldLocation::Register {
                register: GuestRegister::R14,
                storage: LocationStorage::Int32,
            },
            point: RereleaseWorldLocation::Register {
                register: GuestRegister::R13,
                storage: LocationStorage::Pointer,
            },
            normal: RereleaseWorldLocation::Stack {
                offset: 0x108,
                storage: LocationStorage::Pointer,
            },
            flags: RereleaseWorldLocation::Stack {
                offset: 0x120,
                storage: LocationStorage::Int32,
            },
            result: RereleaseWorldLocation::Register {
                register: GuestRegister::R12,
                storage: LocationStorage::Int32,
            },
            repair: vec![ArmorRepair {
                source: RereleaseWorldLocation::Stack {
                    offset: 0xe0,
                    storage: LocationStorage::Uint32,
                },
                target: RereleaseWorldLocation::Register {
                    register: GuestRegister::Rbx,
                    storage: LocationStorage::Uint32,
                },
            }],
        },
        monster: RereleaseMonster {
            attacker: 3120,
            inflictor: 3128,
            blood: 3136,
            knockback: 3140,
            point: 3144,
            cause: 3156,
            invincible_time: 0xb88,
        },
        inventory,
        armor: RereleaseArmor {
            table: 0x1953a8,
            stride: 192,
            normal: 8,
            energy: 12,
            regular: vec![
                "q2:item_armor_jacket".to_string(),
                "q2:item_armor_combat".to_string(),
                "q2:item_armor_body".to_string(),
            ],
            empty: "q2:item_armor_body".to_string(),
            screen: "q2:item_power_screen".to_string(),
            shield: "q2:item_power_shield".to_string(),
            cells: "q2:ammo_cells".to_string(),
            cells_index: 30,
        },
        flags: RereleaseFlags {
            godmode: 16,
            notarget: 32,
            no_knockback: 2048,
            power_armor: 4096,
        },
        movement: RereleaseMovement {
            body: Some(RereleaseMovementBody {
                dimensions: 0xe9ff0,
                trace: 0xe71a0,
                movement_global: 0x23c9c8,
            }),
            game_api: 0x6bcd0,
            pmove: 0xea560,
            speed_loads: vec![
                SpeedLoad {
                    next: 0xe8293,
                    register: 0,
                },
                SpeedLoad {
                    next: 0xe889c,
                    register: 1,
                },
                SpeedLoad {
                    next: 0xe88ce,
                    register: 0,
                },
                SpeedLoad {
                    next: 0xe8b15,
                    register: 1,
                },
                SpeedLoad {
                    next: 0xe8b1f,
                    register: 1,
                },
                SpeedLoad {
                    next: 0xe9e3e,
                    register: 10,
                },
            ],
        },
    })
}

fn validate_layout_shape(layout: &GuestLayout) -> Result<(), String> {
    if layout.pointer_bytes != 8 {
        return Err("source world layout requires the 8-byte pointer width".to_string());
    }
    let mut names = HashSet::new();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for field in &layout.fields {
        let end = field.byte_offset + field.storage.byte_length(8) * field.count;
        if field.count == 0 || !names.insert(field.name.clone()) {
            return Err("overlapping or empty native source fields".to_string());
        }
        if end > layout.byte_length
            || ranges
                .iter()
                .any(|(start, stop)| field.byte_offset < *stop && end > *start)
        {
            return Err("overlapping or empty native source fields".to_string());
        }
        ranges.push((field.byte_offset, end));
    }
    Ok(())
}

fn require_client_field(layout: &GuestLayout, name: &str, storage: GuestStorage, count: usize) -> Result<(), String> {
    match layout.fields.iter().find(|field| field.name == name) {
        Some(field) if field.storage == storage && field.count == count => Ok(()),
        _ => Err(format!("missing typed source client field {name}")),
    }
}

/// Validate a rerelease world profile. Layout self-consistency and every
/// metadata range is checked; the full public-prefix byte identity against
/// the shared layout tables lives with the layout owner lane, so this port
/// requires the consumed typed fields instead.
pub fn validate_rerelease_world(profile: &RereleasePrimaryWorldProfile) -> Result<(), String> {
    let abi = NativeAbi::WindowsX86_64;
    validate_native_combat_call(&profile.calls.pain, CombatOperation::Pain, abi)?;
    validate_native_combat_call(&profile.calls.death, CombatOperation::Death, abi)?;
    validate_native_combat_call(&profile.calls.process_pain, CombatOperation::DeferredReaction, abi)?;
    validate_native_combat_call(&profile.calls.damage, CombatOperation::Damage, abi)?;
    validate_native_combat_call(&profile.calls.power_armor, CombatOperation::PowerArmor, abi)?;
    if profile.digest != RETAIL_DIGEST || profile.client.authority_digest != RETAIL_DIGEST {
        return Err("rerelease source world profile belongs to another artifact".to_string());
    }
    for value in [
        profile.entries.spawn,
        profile.entries.free,
        profile.entries.damage,
        profile.entries.power_armor,
        profile.entries.process_pain,
        profile.entries.time,
        profile.entries.regular_armor.entry,
        profile.entries.regular_armor.join,
        profile.armor.table,
        profile.movement.game_api,
        profile.movement.pmove,
    ] {
        if value == 0 {
            return Err("source world metadata is outside its declared range".to_string());
        }
    }
    if profile.client.inventory_count == 0
        || profile.client.inventory_count > 65536
        || profile.client.ammo_count == 0
        || profile.client.ammo_count > 65536
        || profile.armor.stride < 8
        || profile.armor.stride > 65536
        || profile.armor.normal > 65532
        || profile.armor.energy > 65532
        || profile.armor.cells_index >= profile.client.inventory_count
    {
        return Err("source world metadata is outside its declared range".to_string());
    }
    for value in [
        profile.flags.godmode,
        profile.flags.notarget,
        profile.flags.no_knockback,
        profile.flags.power_armor,
    ] {
        if value == 0 {
            return Err("source world metadata is outside its declared range".to_string());
        }
    }
    if let Some(body) = &profile.movement.body {
        for address in [body.dimensions, body.trace, body.movement_global] {
            if address == 0 {
                return Err("source world metadata is outside its declared range".to_string());
            }
        }
    }
    let mut loads = HashSet::new();
    for load in &profile.movement.speed_loads {
        if load.next == 0 || load.register > 15 || !loads.insert(load.next) {
            return Err("repeated source movement load would apply equipment twice".to_string());
        }
    }
    let locations = [
        profile.regular_armor.target,
        profile.regular_armor.amount,
        profile.regular_armor.point,
        profile.regular_armor.normal,
        profile.regular_armor.flags,
        profile.regular_armor.result,
    ];
    for location in locations.into_iter().chain(
        profile
            .regular_armor
            .repair
            .iter()
            .flat_map(|repair| [repair.source, repair.target]),
    ) {
        if let RereleaseWorldLocation::Stack { offset, storage } = location {
            if offset > 1_048_576 - storage.width() {
                return Err("source world metadata is outside its declared range".to_string());
            }
        }
    }
    validate_layout_shape(&profile.edict)?;
    validate_layout_shape(&profile.client.layout)?;
    require_client_field(
        &profile.client.layout,
        "pers.inventory",
        GuestStorage::Int32,
        profile.client.inventory_count as usize,
    )?;
    require_client_field(
        &profile.client.layout,
        "pers.max_ammo",
        GuestStorage::Int16,
        profile.client.ammo_count as usize,
    )?;
    require_client_field(&profile.client.layout, "v_angle", GuestStorage::Float32, 3)?;
    require_client_field(&profile.client.layout, "invincible_time", GuestStorage::Int64, 1)?;
    for (offset, bytes) in [
        (profile.monster.attacker, 8),
        (profile.monster.inflictor, 8),
        (profile.monster.blood, 4),
        (profile.monster.knockback, 4),
        (profile.monster.point, 12),
        (profile.monster.cause, 3),
        (profile.monster.invincible_time, 8),
    ] {
        if offset + bytes > profile.edict.byte_length as u32 {
            return Err("source monster accumulator exceeds its edict".to_string());
        }
    }
    for location in [
        profile.regular_armor.target,
        profile.regular_armor.point,
        profile.regular_armor.normal,
    ] {
        if !matches!(
            location,
            RereleaseWorldLocation::Register {
                storage: LocationStorage::Pointer,
                ..
            } | RereleaseWorldLocation::Stack {
                storage: LocationStorage::Pointer,
                ..
            }
        ) {
            return Err("source armor geometry requires pointer locations".to_string());
        }
    }
    for location in [
        profile.regular_armor.amount,
        profile.regular_armor.flags,
        profile.regular_armor.result,
    ] {
        if !matches!(
            location,
            RereleaseWorldLocation::Register {
                storage: LocationStorage::Int32,
                ..
            } | RereleaseWorldLocation::Stack {
                storage: LocationStorage::Int32,
                ..
            }
        ) {
            return Err("source armor values require int32 locations".to_string());
        }
    }
    for repair in &profile.regular_armor.repair {
        let storage = |location: RereleaseWorldLocation| match location {
            RereleaseWorldLocation::Register { storage, .. } | RereleaseWorldLocation::Stack { storage, .. } => storage,
        };
        if storage(repair.source) != storage(repair.target) {
            return Err("source armor repair changes scalar representation".to_string());
        }
    }
    if profile.entries.regular_armor.entry == profile.entries.regular_armor.join {
        return Err("source armor region is empty".to_string());
    }
    let items: HashSet<&ItemId> = profile.inventory.iter().map(|row| &row.item).collect();
    if items.len() != profile.inventory.len()
        || profile.inventory.len() != profile.client.inventory_count as usize
        || profile
            .inventory
            .iter()
            .filter(|row| matches!(row.source, InventorySource::Remaining))
            .count()
            > 1
    {
        return Err("source inventory requires one complete unique roster".to_string());
    }
    for row in &profile.inventory {
        match &row.source {
            InventorySource::Index { index } if *index >= profile.client.inventory_count => {
                return Err("source inventory mapping exceeds its private storage".to_string());
            }
            InventorySource::Classname { name } if name.is_empty() || name.contains('\0') => {
                return Err("source inventory mapping exceeds its private storage".to_string());
            }
            _ => {}
        }
        match &row.capacity {
            InventoryCapacity::Ammo { source_index } if *source_index >= profile.client.ammo_count => {
                return Err("source inventory mapping exceeds its private storage".to_string());
            }
            InventoryCapacity::Fixed { count } if *count > 0x7fff_ffff => {
                return Err("source inventory mapping exceeds its private storage".to_string());
            }
            _ => {}
        }
    }
    if profile.armor.regular.iter().collect::<HashSet<_>>().len() != profile.armor.regular.len()
        || !profile.armor.regular.contains(&profile.armor.empty)
        || profile.armor.cells_index >= profile.client.inventory_count
        || [
            &profile.armor.empty,
            &profile.armor.screen,
            &profile.armor.shield,
            &profile.armor.cells,
        ]
        .into_iter()
        .chain(profile.armor.regular.iter())
        .any(|item| !items.contains(item))
    {
        return Err("source armor metadata has no declared inventory item".to_string());
    }
    if profile.armor.normal + 4 > 65536 || profile.armor.energy + 4 > 65536 || profile.movement.speed_loads.is_empty() {
        return Err("invalid source armor or movement metadata".to_string());
    }
    Ok(())
}

fn location_storage(reader: &Reader) -> LocationStorage {
    match reader.field("storage").choice_index(&["pointer", "int32", "uint32"]) {
        0 => LocationStorage::Pointer,
        1 => LocationStorage::Int32,
        _ => LocationStorage::Uint32,
    }
}

fn location(reader: &Reader) -> RereleaseWorldLocation {
    let storage = location_storage(reader);
    if reader.field("kind").choice_index(&["register", "stack"]) == 0 {
        let register = reader.field("register").string();
        let register = match register.as_str() {
            "rax" => GuestRegister::Rax,
            "rcx" => GuestRegister::Rcx,
            "rdx" => GuestRegister::Rdx,
            "rbx" => GuestRegister::Rbx,
            "rbp" => GuestRegister::Rbp,
            "rsi" => GuestRegister::Rsi,
            "rdi" => GuestRegister::Rdi,
            "r8" => GuestRegister::R8,
            "r9" => GuestRegister::R9,
            "r10" => GuestRegister::R10,
            "r11" => GuestRegister::R11,
            "r12" => GuestRegister::R12,
            "r13" => GuestRegister::R13,
            "r14" => GuestRegister::R14,
            "r15" => GuestRegister::R15,
            _ => reader.fail("unknown armor register"),
        };
        RereleaseWorldLocation::Register { register, storage }
    } else {
        let offset = reader.field("offset").integer(0);
        if offset < 0 || offset as u64 > 1_048_576 - u64::from(storage.width()) {
            reader.fail("armor stack offset exceeds its frame");
        }
        RereleaseWorldLocation::Stack {
            offset: offset as u32,
            storage,
        }
    }
}

fn bounded(reader: &Reader, min: i64, max: i64) -> u32 {
    let value = reader.integer(min);
    if value > max {
        reader.fail(&format!("expected integer at most {max}"));
    }
    value as u32
}

/// Read a rerelease world profile subtree and validate it.
pub fn read_rerelease_world_profile(reader: &Reader, digest: String) -> RereleasePrimaryWorldProfile {
    let client = reader.field("client");
    let entries = reader.field("entries");
    let regular = reader.field("regularArmor");
    let monster = reader.field("monster");
    let armor = reader.field("armor");
    let flags = reader.field("flags");
    let movement = reader.field("movement");
    let calls = reader.field("calls");
    let abi = NativeAbi::WindowsX86_64;
    let profile = RereleasePrimaryWorldProfile {
        calls: RereleaseWorldCalls {
            pain: read_native_combat_call(&calls.field("pain"), CombatOperation::Pain, abi),
            death: read_native_combat_call(&calls.field("death"), CombatOperation::Death, abi),
            process_pain: read_native_combat_call(&calls.field("processPain"), CombatOperation::DeferredReaction, abi),
            damage: read_native_combat_call(&calls.field("damage"), CombatOperation::Damage, abi),
            power_armor: read_native_combat_call(&calls.field("powerArmor"), CombatOperation::PowerArmor, abi),
        },
        digest: digest.clone(),
        edict: read_guest_layout(&reader.field("edict")),
        client: RereleaseWorldClient {
            authority_digest: digest,
            layout: read_guest_layout(&client.field("layout")),
            inventory_count: bounded(&client.field("inventoryCount"), 1, 65536),
            ammo_count: bounded(&client.field("ammoCount"), 1, 65536),
        },
        entries: RereleaseEntries {
            spawn: bounded(&entries.field("spawn"), 1, 0xffff_ffff),
            free: bounded(&entries.field("free"), 1, 0xffff_ffff),
            damage: bounded(&entries.field("damage"), 1, 0xffff_ffff),
            power_armor: bounded(&entries.field("powerArmor"), 1, 0xffff_ffff),
            process_pain: bounded(&entries.field("processPain"), 1, 0xffff_ffff),
            time: bounded(&entries.field("time"), 1, 0xffff_ffff),
            regular_armor: NativeRegion {
                entry: bounded(&entries.field("regularArmor").field("entry"), 1, 0xffff_ffff),
                join: bounded(&entries.field("regularArmor").field("join"), 1, 0xffff_ffff),
            },
        },
        regular_armor: RereleaseRegularArmor {
            target: location(&regular.field("target")),
            amount: location(&regular.field("amount")),
            point: location(&regular.field("point")),
            normal: location(&regular.field("normal")),
            flags: location(&regular.field("flags")),
            result: location(&regular.field("result")),
            repair: regular.field("repair").list(|value| ArmorRepair {
                source: location(&value.field("source")),
                target: location(&value.field("target")),
            }),
        },
        monster: RereleaseMonster {
            attacker: monster.field("attacker").integer_u32(0),
            inflictor: monster.field("inflictor").integer_u32(0),
            blood: monster.field("blood").integer_u32(0),
            knockback: monster.field("knockback").integer_u32(0),
            point: monster.field("point").integer_u32(0),
            cause: monster.field("mod").integer_u32(0),
            invincible_time: monster.field("invincibleTime").integer_u32(0),
        },
        inventory: reader.field("inventory").list(|value| {
            let source = value.field("source");
            let capacity = value.field("capacity");
            RereleaseInventoryRow {
                item: namespaced(&value.field("item")),
                source: match source.field("kind").choice_index(&["classname", "index", "remaining"]) {
                    0 => InventorySource::Classname {
                        name: source.field("name").string(),
                    },
                    1 => InventorySource::Index {
                        index: source.field("index").integer_u32(0),
                    },
                    _ => InventorySource::Remaining,
                },
                capacity: if capacity.field("kind").choice_index(&["ammo", "fixed"]) == 0 {
                    InventoryCapacity::Ammo {
                        source_index: capacity.field("sourceIndex").integer_u32(0),
                    }
                } else {
                    InventoryCapacity::Fixed {
                        count: bounded(&capacity.field("count"), 0, 0x7fff_ffff),
                    }
                },
            }
        }),
        armor: RereleaseArmor {
            table: bounded(&armor.field("table"), 1, 0xffff_ffff),
            stride: bounded(&armor.field("stride"), 8, 65536),
            normal: armor.field("normal").integer_u32(0),
            energy: armor.field("energy").integer_u32(0),
            regular: armor.field("regular").list(namespaced),
            empty: namespaced(&armor.field("empty")),
            screen: namespaced(&armor.field("screen")),
            shield: namespaced(&armor.field("shield")),
            cells: namespaced(&armor.field("cells")),
            cells_index: armor.field("cellsIndex").integer_u32(0),
        },
        flags: RereleaseFlags {
            godmode: flags.field("godmode").integer(1) as u64,
            notarget: flags.field("notarget").integer(1) as u64,
            no_knockback: flags.field("noKnockback").integer(1) as u64,
            power_armor: flags.field("powerArmor").integer(1) as u64,
        },
        movement: RereleaseMovement {
            body: if movement.has("body") && !movement.field("body").is_null() {
                let body = movement.field("body");
                Some(RereleaseMovementBody {
                    dimensions: bounded(&body.field("dimensions"), 1, 0xffff_ffff),
                    trace: bounded(&body.field("trace"), 1, 0xffff_ffff),
                    movement_global: bounded(&body.field("movementGlobal"), 1, 0xffff_ffff),
                })
            } else {
                None
            },
            game_api: bounded(&movement.field("gameApi"), 1, 0xffff_ffff),
            pmove: bounded(&movement.field("pmove"), 1, 0xffff_ffff),
            speed_loads: movement.field("speedLoads").list(|value| SpeedLoad {
                next: bounded(&value.field("next"), 1, 0xffff_ffff),
                register: bounded(&value.field("register"), 0, 15) as u8,
            }),
        },
    };
    if let Err(message) = validate_rerelease_world(&profile) {
        reader.fail(&message);
    }
    profile
}

// ---------------------------------------------------------------------------
// Builtin pickups
// ---------------------------------------------------------------------------

fn cdecl_signature(parameters: Vec<GuestValueLayout>, result: Option<GuestValueLayout>) -> GuestCallSignature {
    GuestCallSignature {
        abi: NativeCallAbi::Cdecl,
        parameters,
        result,
        variadic: false,
    }
}

fn x64_signature(parameters: Vec<GuestValueLayout>, result: Option<GuestValueLayout>) -> GuestCallSignature {
    GuestCallSignature {
        abi: NativeCallAbi::MicrosoftX64,
        parameters,
        result,
        variadic: false,
    }
}

fn pointer_layout() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Pointer)
}

fn int_layout() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Int32)
}

fn bool_layout() -> GuestValueLayout {
    GuestValueLayout::Scalar(GuestStorage::Uint8)
}

/// Builtin classic pickup profile for a digest, or `None` when unknown.
///
/// Original Xatrix PE: recipient regions contain no map effects; joins retain
/// respawn tails.
#[must_use]
pub fn classic_pickup_profile(digest: &str) -> Option<NativePickupProfile> {
    if digest != CLASSIC_DIGEST {
        return None;
    }
    Some(NativePickupProfile {
        digest: CLASSIC_DIGEST.to_string(),
        abi: NativeAbi::WindowsI386,
        touch: 0xab00,
        grant_return: 0xab38,
        targets_return: 0xac90,
        touch_signature: cdecl_signature(
            vec![pointer_layout(), pointer_layout(), pointer_layout(), pointer_layout()],
            None,
        ),
        grant_signature: cdecl_signature(vec![pointer_layout(), pointer_layout()], Some(int_layout())),
        grants: vec![
            NativePickupGrant {
                entry: 0xa780,
                recipient: NativeRegion {
                    entry: 0xa795,
                    join: 0xa8cb,
                },
                resource: PickupResource::Regular,
                consumers: vec![],
                supply: None,
            },
            NativePickupGrant {
                entry: 0xa3e0,
                recipient: NativeRegion {
                    entry: 0xa3e4,
                    join: 0xa4b8,
                },
                resource: PickupResource::Inventory,
                consumers: vec![],
                supply: Some(PickupSupply::Ammo {
                    entry: 0xa41c,
                    amount: GuestRegister::Rbx,
                }),
            },
            NativePickupGrant {
                entry: 0x35ff0,
                recipient: NativeRegion {
                    entry: 0x36064,
                    join: 0x36077,
                },
                resource: PickupResource::Inventory,
                consumers: vec![],
                supply: Some(PickupSupply::Weapon {
                    ammo_return: 0x360dc,
                    settle: 0x360e5,
                    autoswitch: NativeRegion {
                        entry: 0x3614a,
                        join: 0x3619b,
                    },
                }),
            },
            NativePickupGrant {
                entry: 0x9960,
                recipient: NativeRegion {
                    entry: 0x996b,
                    join: 0x9a72,
                },
                resource: PickupResource::Inventory,
                consumers: vec![],
                supply: None,
            },
            NativePickupGrant {
                entry: 0x9ac0,
                recipient: NativeRegion {
                    entry: 0x9acb,
                    join: 0x9d98,
                },
                resource: PickupResource::Inventory,
                consumers: vec![],
                supply: None,
            },
        ],
        items: PickupItems {
            table: 0x4b828,
            stride: 76,
            count: 48,
            classname: 0,
            pickup: 4,
        },
        entity: PickupEntity {
            item: 0x288,
            count: 0x214,
            spawnflags: 0x11c,
            inuse: 88,
            inuse_bytes: 4,
            generation: None,
        },
        time: PickupTime {
            address: 0x76804,
            storage: TimeStorage::FloatSeconds,
        },
        supply: PickupSupplyProfile {
            client: 0x54,
            inventory: 0x2e4,
            flags: 0x38,
            weapon_flag: 1,
            ammo: AmmoSupply {
                entry: 0xa310,
                signature: cdecl_signature(
                    vec![pointer_layout(), pointer_layout(), int_layout()],
                    Some(int_layout()),
                ),
                stop: None,
                tag: 0x44,
                capacities: vec![0x6e4, 0x6e8, 0x6ec, 0x6f0, 0x6f4, 0x6f8, 0x6fc, 0x700],
                capacity_bytes: 4,
            },
        },
    })
}

/// Builtin rerelease pickup profile for a digest, or `None` when unknown.
///
/// Retail PE regions retain the native register saves, dropped checks and
/// respawn calls.
#[must_use]
pub fn rerelease_pickup_profile(digest: &str) -> Option<NativePickupProfile> {
    if digest != RETAIL_DIGEST {
        return None;
    }
    let consumers = vec![PickupConsumer {
        entry: 0x666c0,
        signature: x64_signature(vec![pointer_layout()], None),
        protection: ProtectionChannel::Powered,
    }];
    Some(NativePickupProfile {
        digest: RETAIL_DIGEST.to_string(),
        abi: NativeAbi::WindowsX86_64,
        touch: 0x67be0,
        grant_return: 0x67c95,
        targets_return: 0x67f27,
        touch_signature: x64_signature(
            vec![pointer_layout(), pointer_layout(), pointer_layout(), bool_layout()],
            None,
        ),
        grant_signature: x64_signature(vec![pointer_layout(), pointer_layout()], Some(bool_layout())),
        grants: vec![
            NativePickupGrant {
                entry: 0x67740,
                recipient: NativeRegion {
                    entry: 0x677d4,
                    join: 0x678e8,
                },
                resource: PickupResource::Regular,
                consumers: vec![],
                supply: None,
            },
            NativePickupGrant {
                entry: 0x671e0,
                recipient: NativeRegion {
                    entry: 0x67209,
                    join: 0x67357,
                },
                resource: PickupResource::Inventory,
                consumers: consumers.clone(),
                supply: Some(PickupSupply::Ammo {
                    entry: 0x67250,
                    amount: GuestRegister::Rcx,
                }),
            },
            NativePickupGrant {
                entry: 0xefd80,
                recipient: NativeRegion {
                    entry: 0xefe19,
                    join: 0xefe31,
                },
                resource: PickupResource::Inventory,
                consumers: consumers.clone(),
                supply: Some(PickupSupply::Weapon {
                    ammo_return: 0xefeac,
                    settle: 0xefeb3,
                    autoswitch: NativeRegion {
                        entry: 0xeff20,
                        join: 0xeff33,
                    },
                }),
            },
            NativePickupGrant {
                entry: 0x667b0,
                recipient: NativeRegion {
                    entry: 0x667c4,
                    join: 0x66918,
                },
                resource: PickupResource::Inventory,
                consumers: consumers.clone(),
                supply: None,
            },
            NativePickupGrant {
                entry: 0x66960,
                recipient: NativeRegion {
                    entry: 0x66974,
                    join: 0x66ce4,
                },
                resource: PickupResource::Inventory,
                consumers,
                supply: None,
            },
        ],
        items: PickupItems {
            table: 0x195320,
            stride: 192,
            count: 84,
            classname: 8,
            pickup: 16,
        },
        entity: PickupEntity {
            item: 0x860,
            count: 0x7b8,
            spawnflags: 0x5f0,
            inuse: 1376,
            inuse_bytes: 1,
            generation: Some(1472),
        },
        time: PickupTime {
            address: 0x241b28,
            storage: TimeStorage::Int64Milliseconds,
        },
        supply: PickupSupplyProfile {
            client: 0x78,
            inventory: 0xa80,
            flags: 0x7c,
            weapon_flag: 1,
            ammo: AmmoSupply {
                entry: 0x670e0,
                signature: x64_signature(
                    vec![pointer_layout(), pointer_layout(), int_layout()],
                    Some(bool_layout()),
                ),
                stop: Some(0x6712e),
                tag: 0x90,
                capacities: (0..12).map(|tag| 0xbd0 + tag * 2).collect(),
                capacity_bytes: 2,
            },
        },
    })
}

// ---------------------------------------------------------------------------
// Primary profile
// ---------------------------------------------------------------------------

/// Primary edition selecting the world profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrimaryEdition {
    /// Classic edition.
    Classic,
    /// Rerelease edition.
    Rerelease,
}

/// Native primary profile: shared services plus the edition world profile.
#[derive(Debug, Clone, PartialEq)]
pub enum NativePrimaryProfile {
    /// Classic services and world.
    Classic {
        /// Weapon service profile.
        weapons: NativePrimaryWeaponProfile,
        /// Player service profile.
        player: NativePrimaryPlayerProfile,
        /// Command service profile.
        commands: NativePrimaryCommandProfile,
        /// Inventory service profile.
        inventory: NativePrimaryInventoryProfile,
        /// Drop service profile.
        drop: NativePrimaryDropProfile,
        /// Pickup service profile.
        pickups: NativePickupProfile,
        /// Classic world profile.
        world: ClassicPrimaryWorldProfile,
    },
    /// Rerelease services and world.
    Rerelease {
        /// Weapon service profile.
        weapons: NativePrimaryWeaponProfile,
        /// Player service profile.
        player: NativePrimaryPlayerProfile,
        /// Command service profile.
        commands: NativePrimaryCommandProfile,
        /// Inventory service profile.
        inventory: NativePrimaryInventoryProfile,
        /// Drop service profile.
        drop: NativePrimaryDropProfile,
        /// Pickup service profile.
        pickups: NativePickupProfile,
        /// Rerelease world profile.
        world: RereleasePrimaryWorldProfile,
    },
}

impl NativePrimaryProfile {
    /// Profile edition.
    #[must_use]
    pub fn edition(&self) -> PrimaryEdition {
        match self {
            Self::Classic { .. } => PrimaryEdition::Classic,
            Self::Rerelease { .. } => PrimaryEdition::Rerelease,
        }
    }

    /// Weapon service profile.
    #[must_use]
    pub fn weapons(&self) -> &NativePrimaryWeaponProfile {
        match self {
            Self::Classic { weapons, .. } | Self::Rerelease { weapons, .. } => weapons,
        }
    }

    /// Player service profile.
    #[must_use]
    pub fn player(&self) -> &NativePrimaryPlayerProfile {
        match self {
            Self::Classic { player, .. } | Self::Rerelease { player, .. } => player,
        }
    }

    /// Command service profile.
    #[must_use]
    pub fn commands(&self) -> &NativePrimaryCommandProfile {
        match self {
            Self::Classic { commands, .. } | Self::Rerelease { commands, .. } => commands,
        }
    }

    /// Inventory service profile.
    #[must_use]
    pub fn inventory(&self) -> &NativePrimaryInventoryProfile {
        match self {
            Self::Classic { inventory, .. } | Self::Rerelease { inventory, .. } => inventory,
        }
    }

    /// Drop service profile.
    #[must_use]
    pub fn drop(&self) -> &NativePrimaryDropProfile {
        match self {
            Self::Classic { drop, .. } | Self::Rerelease { drop, .. } => drop,
        }
    }

    /// Pickup service profile.
    #[must_use]
    pub fn pickups(&self) -> &NativePickupProfile {
        match self {
            Self::Classic { pickups, .. } | Self::Rerelease { pickups, .. } => pickups,
        }
    }
}

/// Declared primary profile with its resource reference.
#[derive(Debug, Clone, PartialEq)]
pub struct NativePrimaryDeclaration {
    /// Declaring resource reference.
    pub declaration: String,
    /// Primary profile.
    pub profile: NativePrimaryProfile,
}

/// Builtin primary profile for a digest and edition, or `None` when any
/// service or world table is unknown.
#[must_use]
pub fn builtin_native_primary(digest: &str, edition: PrimaryEdition) -> Option<NativePrimaryProfile> {
    let weapons = native_primary_weapon_profile(digest)?;
    let player = native_primary_player_profile(digest)?;
    let commands = native_primary_command_profile(digest)?;
    let inventory = native_primary_inventory_profile(digest)?;
    let drop = native_primary_drop_profile(digest)?;
    match edition {
        PrimaryEdition::Classic => {
            let pickups = classic_pickup_profile(digest)?;
            let world = classic_primary_world_profile(digest)?;
            Some(NativePrimaryProfile::Classic {
                weapons,
                player,
                commands,
                inventory,
                drop,
                pickups,
                world,
            })
        }
        PrimaryEdition::Rerelease => {
            let pickups = rerelease_pickup_profile(digest)?;
            let world = rerelease_primary_world_profile(digest)?;
            Some(NativePrimaryProfile::Rerelease {
                weapons,
                player,
                commands,
                inventory,
                drop,
                pickups,
                world,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_primary_reader::parse_json;
    use super::*;

    #[test]
    fn assembles_classic_builtins() {
        let profile = builtin_native_primary(CLASSIC_DIGEST, PrimaryEdition::Classic).expect("classic");
        assert_eq!(profile.edition(), PrimaryEdition::Classic);
        assert_eq!(profile.inventory().client, 84);
        assert_eq!(profile.commands().client.inventory, profile.inventory().inventory);
        assert_eq!(profile.drop().client.inventory, profile.inventory().inventory);
        assert_eq!(profile.pickups().supply.inventory, profile.inventory().inventory);
        assert_eq!(profile.commands().items.table, profile.pickups().items.table);
        assert_eq!(profile.commands().client.weapon, profile.drop().client.weapon);
        match &profile {
            NativePrimaryProfile::Classic { world, .. } => {
                assert_eq!(world.entity_bytes, 896);
                assert_eq!(world.inventory_table.count, 48);
                validate_classic_combat(world).expect("valid");
            }
            NativePrimaryProfile::Rerelease { .. } => panic!("expected classic"),
        }
    }

    #[test]
    fn assembles_retail_builtins() {
        let profile = builtin_native_primary(RETAIL_DIGEST, PrimaryEdition::Rerelease).expect("retail");
        assert_eq!(profile.edition(), PrimaryEdition::Rerelease);
        assert_eq!(profile.inventory().client, 120);
        assert_eq!(profile.pickups().entity.inuse, 1376);
        assert_eq!(profile.pickups().entity.generation, Some(1472));
        match &profile {
            NativePrimaryProfile::Rerelease { world, .. } => {
                assert_eq!(world.edict.byte_length, 3688);
                assert_eq!(world.inventory.len(), 84);
                assert_eq!(world.client.inventory_count, 84);
                validate_rerelease_world(world).expect("valid");
            }
            NativePrimaryProfile::Classic { .. } => panic!("expected rerelease"),
        }
    }

    #[test]
    fn rejects_unknown_builtins() {
        assert!(builtin_native_primary("sha256:dead", PrimaryEdition::Classic).is_none());
        assert!(builtin_native_primary(CLASSIC_DIGEST, PrimaryEdition::Rerelease).is_none());
    }

    #[test]
    fn reads_classic_world_documents() {
        let pain = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"attacker"},{"kind":"field","field":"kick"},{"kind":"field","field":"amount"}]}"#;
        let death = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"inflictor"},{"kind":"field","field":"attacker"},{"kind":"field","field":"amount"},{"kind":"field","field":"point"}]}"#;
        let damage = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"inflictor"},{"kind":"field","field":"attacker"},{"kind":"field","field":"direction"},{"kind":"field","field":"point"},{"kind":"field","field":"normal"},{"kind":"field","field":"amount"},{"kind":"field","field":"knockback"},{"kind":"field","field":"flags"},{"kind":"field","field":"cause"}]}"#;
        let regular = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"point"},{"kind":"field","field":"normal"},{"kind":"field","field":"amount"},{"kind":"field","field":"sparks"},{"kind":"field","field":"flags"}]}"#;
        let power = r#"{"convention":"cdecl","arguments":[{"kind":"field","field":"target"},{"kind":"field","field":"point"},{"kind":"field","field":"normal"},{"kind":"field","field":"amount"},{"kind":"field","field":"flags"}]}"#;
        let text = format!(
            r#"{{"calls":{{"pain":{pain},"death":{death},"damage":{damage},"regularArmor":{regular},"powerArmor":{power}}},
            "game":"xatrix","entityBytes":896,
            "fields":{{"health":480,"damageable":512,"flags":264,"mass":400,"velocity":376,"pain":452,"die":456}},
            "client":{{"inventory":740,"inventoryCount":256,"maxGrenades":1776,"invincibleFrame":3728,"userinfo":188,"userinfoBytes":512,"viewAngles":3652}},
            "entries":{{"damage":20560,"powerArmor":21888,"regularArmor":22368,"spawn":102544,"free":102976}},
            "globals":{{"levelFrame":485376,"itemList":310832,"itemBytes":76}},
            "items":{{"jacket":3,"combat":2,"body":1,"screen":5,"shield":6,"cells":23,"grenades":12}},
            "itemFields":{{"className":0,"armorInfo":64}},"armorInfo":{{"normalProtection":8,"energyProtection":12}},
            "flags":{{"invulnerable":16,"notarget":32,"noKnockback":2048,"powerArmor":4096}},
            "teams":{{"model":64,"skin":128}},"armor":{{"regular":[3,2,1],"empty":1}},
            "inventoryTable":{{"count":48,"className":0,"label":40,"flags":56,"ammoFlag":2,"tag":68,
            "capacities":[1764,1768,1772,1776,1780,1784,1788,1792],
            "unnamed":[{{"index":0,"label":"","item":"q2:none"}}],"emptyIndex":0,"sentinel":true}}}}"#
        );
        let value = parse_json(&text).expect("valid json");
        let root = Reader::root(&value);
        let world = read_classic_world_profile(&root, CLASSIC_DIGEST.to_string());
        assert_eq!(world.game, ClassicGame::Xatrix);
        assert_eq!(world.inventory_table.unnamed.len(), 1);
    }
}
