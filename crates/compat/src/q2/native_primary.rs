//! Port of `src/compat/q2/native-primary.ts`.
//! Bridges builtin primary profiles: service tables, world profiles and declarations.

use std::collections::HashSet;

use qa_guest::core::contracts::{
    GuestCallSignature, GuestFieldLayout, GuestLayout, GuestRegister, GuestStorage, GuestValueLayout,
    NativeAbi, NativeCallAbi,
};
use qa_world::combat::ItemId;

use super::native_combat_call::{
    CombatOperation, NativeCombatCall, read_native_combat_call, stock_native_combat_call,
    validate_native_combat_call,
};
use super::native_primary_command_profile::native_primary_command_profile;
use super::native_primary_commands::NativePrimaryCommandProfile;
use super::native_primary_drop_profile::native_primary_drop_profile;
use super::native_primary_drop::NativePrimaryDropProfile;
use super::native_primary_inventory_profile::native_primary_inventory_profile;
use super::native_primary_inventory::NativePrimaryInventoryProfile;
use super::native_primary_pickups::{
    AmmoSupply, NativePickupGrant, NativePickupProfile, PickupConsumer, PickupEntity, PickupItems,
    PickupResource, PickupSupply, PickupSupplyProfile, PickupTime, ProtectionChannel, TimeStorage,
};
use super::native_primary_player_profile::native_primary_player_profile;
use super::native_primary_player::NativePrimaryPlayerProfile;
use super::native_primary_reader::{
    CLASSIC_DIGEST, RETAIL_DIGEST, NativeRegion, Reader, native_offset, native_signature,
    namespaced, read_guest_layout,
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
    if profile.entity_bytes < 260 || profile.globals.item_bytes < 4 || profile.client.inventory_count < 1
    {
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
        return Err(
            "classic regular armor requires distinct source priorities and an admitted empty tier"
                .to_string(),
        );
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
            return Err(
                "classic combat inventory index exceeds its declared source storage".to_string(),
            );
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
            regular_armor: read_native_combat_call(
                &calls.field("regularArmor"),
                CombatOperation::RegularArmor,
                abi,
            ),
            power_armor: read_native_combat_call(
                &calls.field("powerArmor"),
                CombatOperation::PowerArmor,
                abi,
            ),
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
