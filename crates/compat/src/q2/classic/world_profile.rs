//! Donor: `src/compat/q2/classic/world-profile.ts` — the primary world
//! profile with its source item table.
//!
//! Bridges saved profile values to the verified combat profile plus the
//! native item-table declaration consumed by source inventory.

use std::collections::HashMap;

use qa_guest::core::contracts::{
    ContentDigest, GuestFieldLayout, GuestLayout, GuestStorage, GuestValueLayout, NativeCallAbi,
};
use qa_world::combat::ItemId;

use super::combat_profile::{
    classic_combat_profile, validate_classic_combat_profile, validate_native_combat_call, ClassicCombatOperation,
    ClassicCombatProfile, ClassicGame, ClassicNativeCombatArgument, ClassicNativeCombatCall, ClassicNativeCombatField,
    CombatAddressDefault, CombatArmor, CombatArmorInfo, CombatCalls, CombatClientFields, CombatEntityFields,
    CombatEntries, CombatFlags, CombatGlobals, CombatItemFields, CombatItems, CombatTeams,
};
use super::layout::{ClassicQ2Error, ClassicResult};

/// One saved profile value.
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileValue {
    /// Integer number.
    Int(i64),
    /// Floating number.
    Float(f64),
    /// String.
    Str(String),
    /// Boolean.
    Bool(bool),
    /// List.
    List(Vec<ProfileValue>),
    /// Record.
    Map(HashMap<String, ProfileValue>),
    /// Null.
    Null,
}

/// Reader over a saved profile value tree, mirroring the donor save reader.
#[derive(Debug, Clone)]
pub struct ProfileReader<'a> {
    value: &'a ProfileValue,
    path: String,
}

impl<'a> ProfileReader<'a> {
    /// Read from a root value.
    #[must_use]
    pub fn root(value: &'a ProfileValue) -> Self {
        Self {
            value,
            path: "profile".to_string(),
        }
    }

    /// Fail with the reader path attached.
    pub fn fail<T>(&self, detail: &str) -> ClassicResult<T> {
        let path = &self.path;
        Err(ClassicQ2Error::invalid(format!("{path}: {detail}")))
    }

    /// Read a named record field.
    pub fn field(&self, name: &'static str) -> ClassicResult<ProfileReader<'a>> {
        match self.value {
            ProfileValue::Map(map) => map.get(name).map_or_else(
                || self.fail(&format!("missing field {name}")),
                |value| {
                    let parent = &self.path;
                    Ok(ProfileReader {
                        value,
                        path: format!("{parent}.{name}"),
                    })
                },
            ),
            _ => self.fail("expected a record"),
        }
    }

    /// Read an integer at least `minimum`.
    pub fn integer(&self, minimum: i64) -> ClassicResult<i64> {
        let value = match self.value {
            ProfileValue::Int(value) => *value,
            ProfileValue::Float(value) if value.fract() == 0.0 && value.abs() < 9.007_199_254_740_992.0 => {
                *value as i64
            }
            _ => return self.fail("expected an integer in range"),
        };
        if value < minimum {
            return self.fail("expected an integer in range");
        }
        Ok(value)
    }

    /// Read a string.
    pub fn string(&self) -> ClassicResult<String> {
        match self.value {
            ProfileValue::Str(value) => Ok(value.clone()),
            _ => self.fail("expected a string"),
        }
    }

    /// Read a boolean.
    pub fn boolean(&self) -> ClassicResult<bool> {
        match self.value {
            ProfileValue::Bool(value) => Ok(*value),
            _ => self.fail("expected a boolean"),
        }
    }

    /// Read one of the allowed string choices.
    pub fn choice(&self, options: &[&str]) -> ClassicResult<String> {
        let value = self.string()?;
        if options.contains(&value.as_str()) {
            Ok(value)
        } else {
            self.fail("unexpected choice")
        }
    }

    /// Read a list with an element reader.
    pub fn list<T>(&self, each: impl Fn(&ProfileReader<'a>) -> ClassicResult<T>) -> ClassicResult<Vec<T>> {
        match self.value {
            ProfileValue::List(values) => values
                .iter()
                .map(|value| {
                    let parent = &self.path;
                    each(&ProfileReader {
                        value,
                        path: format!("{parent}[]"),
                    })
                })
                .collect(),
            _ => self.fail("expected a list"),
        }
    }

    /// Read a nullable value.
    pub fn nullable<T>(&self, each: impl Fn(&ProfileReader<'a>) -> ClassicResult<T>) -> ClassicResult<Option<T>> {
        match self.value {
            ProfileValue::Null => Ok(None),
            _ => Ok(Some(each(self)?)),
        }
    }
}

/// Read a native offset bounded by the 32-bit address space.
pub fn native_offset(reader: &ProfileReader) -> ClassicResult<u32> {
    let value = reader.integer(0)?;
    if value > 0xffff_ffff {
        return reader.fail("native offset exceeds PE address space");
    }
    Ok(value as u32)
}

/// Read a native scalar storage name.
pub fn native_scalar(reader: &ProfileReader) -> ClassicResult<GuestStorage> {
    match reader
        .choice(&[
            "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64",
        ])?
        .as_str()
    {
        "int8" => Ok(GuestStorage::Int8),
        "uint8" => Ok(GuestStorage::Uint8),
        "int16" => Ok(GuestStorage::Int16),
        "uint16" => Ok(GuestStorage::Uint16),
        "int32" => Ok(GuestStorage::Int32),
        "uint32" => Ok(GuestStorage::Uint32),
        "int64" => Ok(GuestStorage::Int64),
        "uint64" => Ok(GuestStorage::Uint64),
        "float32" => Ok(GuestStorage::Float32),
        _ => Ok(GuestStorage::Float64),
    }
}

/// Read a `namespace:name` identity.
pub fn namespaced(reader: &ProfileReader) -> ClassicResult<String> {
    let value = reader.string()?;
    match value.find(':') {
        Some(colon) if colon > 0 && colon + 1 < value.len() => Ok(value),
        _ => reader.fail("expected a namespaced identity"),
    }
}

fn read_layout(reader: &ProfileReader) -> ClassicResult<GuestLayout> {
    Ok(GuestLayout::new(
        &reader.field("id")?.string()?,
        reader.field("byteLength")?.integer(1)? as usize,
        reader.field("alignment")?.integer(1)? as usize,
        reader.field("pointerBytes")?.integer(1)? as usize,
        reader.field("fields")?.list(|field| {
            Ok(GuestFieldLayout {
                name: field.field("name")?.string()?,
                byte_offset: native_offset(&field.field("byteOffset")?)? as usize,
                storage: native_scalar(&field.field("storage")?)?,
                count: field.field("count")?.integer(1)? as usize,
            })
        })?,
    ))
}

fn read_convention(reader: &ProfileReader) -> ClassicResult<NativeCallAbi> {
    match reader
        .choice(&["cdecl", "stdcall", "fastcall", "thiscall", "microsoft-x64"])?
        .as_str()
    {
        "cdecl" => Ok(NativeCallAbi::Cdecl),
        "stdcall" => Ok(NativeCallAbi::Stdcall),
        "fastcall" => Ok(NativeCallAbi::Fastcall),
        "thiscall" => Ok(NativeCallAbi::Thiscall),
        _ => Ok(NativeCallAbi::MicrosoftX64),
    }
}

/// Read a combat call declaration, validating it against its operation.
pub fn read_native_combat_call(
    reader: &ProfileReader,
    operation: ClassicCombatOperation,
) -> ClassicResult<ClassicNativeCombatCall> {
    let result = ClassicNativeCombatCall {
        convention: read_convention(&reader.field("convention")?)?,
        arguments: reader.field("arguments")?.list(|value| {
            match value.field("kind")?.choice(&["field", "value", "address"])?.as_str() {
                "field" => {
                    let name = value.field("field")?.choice(&[
                        "target",
                        "inflictor",
                        "attacker",
                        "direction",
                        "point",
                        "normal",
                        "amount",
                        "knockback",
                        "flags",
                        "cause",
                        "sparks",
                        "kick",
                    ])?;
                    ClassicNativeCombatField::parse(&name)
                        .map(ClassicNativeCombatArgument::Field)
                        .ok_or_else(|| ClassicQ2Error::invalid("unknown native combat field"))
                }
                "address" => Ok(ClassicNativeCombatArgument::Address {
                    target: value.field("address")?.nullable(|address| {
                        Ok(CombatAddressDefault {
                            rva: native_offset(&address.field("rva")?)?,
                            indirections: address.field("indirections")?.list(native_offset)?,
                        })
                    })?,
                }),
                _ => {
                    let layout = value.field("layout")?;
                    let resolved = if layout.field("kind")?.choice(&["scalar", "aggregate"])? == "scalar" {
                        GuestValueLayout::Scalar(native_scalar(&layout.field("storage")?)?)
                    } else {
                        GuestValueLayout::Aggregate(read_layout(&layout.field("layout")?)?)
                    };
                    Ok(ClassicNativeCombatArgument::Value {
                        layout: resolved,
                        bytes: value
                            .field("bytes")?
                            .list(|byte| byte.integer(0))?
                            .into_iter()
                            .map(|byte| {
                                u8::try_from(byte)
                                    .map_err(|_| ClassicQ2Error::invalid("combat default byte out of range"))
                            })
                            .collect::<ClassicResult<Vec<u8>>>()?,
                    })
                }
            }
        })?,
    };
    validate_native_combat_call(&result, operation)?;
    Ok(result)
}

/// Unnamed source item identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnnamedItem {
    /// Table index.
    pub index: usize,
    /// Pickup label.
    pub label: String,
    /// Item identity.
    pub item: ItemId,
}

/// Source item table declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassicInventoryTableProfile {
    /// Table entry count.
    pub count: usize,
    /// Classname pointer field.
    pub class_name: usize,
    /// Label pointer field.
    pub label: usize,
    /// Flags field.
    pub flags: usize,
    /// Ammo flag mask.
    pub ammo_flag: i64,
    /// Ammo tag field.
    pub tag: usize,
    /// Per-tag client capacity fields.
    pub capacities: Vec<usize>,
    /// Unnamed slot identities.
    pub unnamed: Vec<UnnamedItem>,
    /// Empty-slot index.
    pub empty_index: usize,
    /// Whether a zero sentinel follows the table.
    pub sentinel: bool,
}

/// Combat profile plus the source item table.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicPrimaryWorldProfile {
    /// Combat profile.
    pub combat: ClassicCombatProfile,
    /// Item table.
    pub inventory_table: ClassicInventoryTableProfile,
}

/// Primary world profile for an artifact digest, if admitted.
#[must_use]
pub fn classic_primary_world_profile(digest: &ContentDigest) -> Option<ClassicPrimaryWorldProfile> {
    let combat = classic_combat_profile(digest)?;
    Some(ClassicPrimaryWorldProfile {
        combat,
        inventory_table: ClassicInventoryTableProfile {
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

/// Read a primary world profile from saved values, validating every range.
pub fn read_classic_primary_world_profile(
    reader: &ProfileReader,
    digest: ContentDigest,
) -> ClassicResult<ClassicPrimaryWorldProfile> {
    let fields = reader.field("fields")?;
    let client = reader.field("client")?;
    let entries = reader.field("entries")?;
    let globals = reader.field("globals")?;
    let items = reader.field("items")?;
    let item_fields = reader.field("itemFields")?;
    let armor_info = reader.field("armorInfo")?;
    let flags = reader.field("flags")?;
    let teams = reader.field("teams")?;
    let armor = reader.field("armor")?;
    let table = reader.field("inventoryTable")?;
    let calls = reader.field("calls")?;
    let game = reader.field("game")?.choice(&["base", "xatrix", "rogue", "ctf"])?;
    let profile = ClassicPrimaryWorldProfile {
        combat: ClassicCombatProfile {
            calls: CombatCalls {
                pain: read_native_combat_call(&calls.field("pain")?, ClassicCombatOperation::Pain)?,
                death: read_native_combat_call(&calls.field("death")?, ClassicCombatOperation::Death)?,
                damage: read_native_combat_call(&calls.field("damage")?, ClassicCombatOperation::Damage)?,
                regular_armor: read_native_combat_call(
                    &calls.field("regularArmor")?,
                    ClassicCombatOperation::RegularArmor,
                )?,
                power_armor: read_native_combat_call(&calls.field("powerArmor")?, ClassicCombatOperation::PowerArmor)?,
            },
            digest,
            game: ClassicGame::parse(&game).ok_or_else(|| ClassicQ2Error::invalid("unknown classic game"))?,
            entity_bytes: reader.field("entityBytes")?.integer(1)? as usize,
            fields: CombatEntityFields {
                health: native_offset(&fields.field("health")?)? as usize,
                damageable: native_offset(&fields.field("damageable")?)? as usize,
                flags: native_offset(&fields.field("flags")?)? as usize,
                mass: native_offset(&fields.field("mass")?)? as usize,
                velocity: native_offset(&fields.field("velocity")?)? as usize,
                pain: native_offset(&fields.field("pain")?)? as usize,
                die: native_offset(&fields.field("die")?)? as usize,
            },
            client: CombatClientFields {
                inventory: native_offset(&client.field("inventory")?)? as usize,
                inventory_count: client.field("inventoryCount")?.integer(1)? as usize,
                max_grenades: native_offset(&client.field("maxGrenades")?)? as usize,
                invincible_frame: native_offset(&client.field("invincibleFrame")?)? as usize,
                userinfo: native_offset(&client.field("userinfo")?)? as usize,
                view_angles: native_offset(&client.field("viewAngles")?)? as usize,
                userinfo_bytes: client.field("userinfoBytes")?.integer(1)? as usize,
            },
            entries: CombatEntries {
                damage: native_offset(&entries.field("damage")?)?,
                power_armor: native_offset(&entries.field("powerArmor")?)?,
                regular_armor: native_offset(&entries.field("regularArmor")?)?,
                spawn: native_offset(&entries.field("spawn")?)?,
                free: native_offset(&entries.field("free")?)?,
            },
            globals: CombatGlobals {
                level_frame: native_offset(&globals.field("levelFrame")?)?,
                item_list: native_offset(&globals.field("itemList")?)?,
                item_bytes: globals.field("itemBytes")?.integer(1)? as usize,
            },
            items: CombatItems {
                jacket: native_offset(&items.field("jacket")?)? as usize,
                combat: native_offset(&items.field("combat")?)? as usize,
                body: native_offset(&items.field("body")?)? as usize,
                screen: native_offset(&items.field("screen")?)? as usize,
                shield: native_offset(&items.field("shield")?)? as usize,
                cells: native_offset(&items.field("cells")?)? as usize,
                grenades: native_offset(&items.field("grenades")?)? as usize,
            },
            item_fields: CombatItemFields {
                class_name: native_offset(&item_fields.field("className")?)? as usize,
                armor_info: native_offset(&item_fields.field("armorInfo")?)? as usize,
            },
            armor_info: CombatArmorInfo {
                normal_protection: native_offset(&armor_info.field("normalProtection")?)? as usize,
                energy_protection: native_offset(&armor_info.field("energyProtection")?)? as usize,
            },
            flags: CombatFlags {
                invulnerable: native_offset(&flags.field("invulnerable")?)?,
                notarget: native_offset(&flags.field("notarget")?)?,
                no_knockback: native_offset(&flags.field("noKnockback")?)?,
                power_armor: native_offset(&flags.field("powerArmor")?)?,
            },
            teams: CombatTeams {
                model: native_offset(&teams.field("model")?)?,
                skin: native_offset(&teams.field("skin")?)?,
            },
            armor: CombatArmor {
                regular: armor
                    .field("regular")?
                    .list(native_offset)?
                    .into_iter()
                    .map(|value| value as usize)
                    .collect(),
                empty: native_offset(&armor.field("empty")?)? as usize,
            },
        },
        inventory_table: ClassicInventoryTableProfile {
            count: table.field("count")?.integer(1)? as usize,
            class_name: native_offset(&table.field("className")?)? as usize,
            label: native_offset(&table.field("label")?)? as usize,
            flags: native_offset(&table.field("flags")?)? as usize,
            ammo_flag: table.field("ammoFlag")?.integer(1)?,
            tag: native_offset(&table.field("tag")?)? as usize,
            capacities: table
                .field("capacities")?
                .list(native_offset)?
                .into_iter()
                .map(|value| value as usize)
                .collect(),
            unnamed: table.field("unnamed")?.list(|value| {
                Ok(UnnamedItem {
                    index: native_offset(&value.field("index")?)? as usize,
                    label: value.field("label")?.string()?,
                    item: namespaced(&value.field("item")?)?,
                })
            })?,
            empty_index: table.field("emptyIndex")?.integer(0)? as usize,
            sentinel: table.field("sentinel")?.boolean()?,
        },
    };
    validate_classic_combat_profile(&profile.combat)?;
    if profile.inventory_table.count > profile.combat.client.inventory_count
        || profile.inventory_table.empty_index >= profile.inventory_table.count
    {
        return table.fail("item table exceeds original inventory storage");
    }
    for index in [
        profile.combat.items.jacket,
        profile.combat.items.combat,
        profile.combat.items.body,
        profile.combat.items.screen,
        profile.combat.items.shield,
        profile.combat.items.cells,
        profile.combat.items.grenades,
    ]
    .into_iter()
    .chain(profile.combat.armor.regular.iter().copied())
    {
        if index >= profile.inventory_table.count {
            return table.fail("combat item is outside the original source table");
        }
    }
    for field in [
        profile.inventory_table.class_name,
        profile.inventory_table.label,
        profile.inventory_table.flags,
        profile.inventory_table.tag,
    ] {
        if field + 4 > profile.combat.globals.item_bytes {
            return table.fail("item field exceeds declared stride");
        }
    }
    let mut indices = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for entry in &profile.inventory_table.unnamed {
        if entry.index >= profile.inventory_table.count
            || !indices.insert(entry.index)
            || !names.insert(entry.item.clone())
        {
            return table.fail("unnamed item identities must be unique source slots");
        }
    }
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::super::combat_profile::XATRIX_DIGEST_VALUE;
    use super::*;

    fn int(value: i64) -> ProfileValue {
        ProfileValue::Int(value)
    }

    fn str_(value: &str) -> ProfileValue {
        ProfileValue::Str(value.to_string())
    }

    fn map(entries: Vec<(&str, ProfileValue)>) -> ProfileValue {
        ProfileValue::Map(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    fn call(operation: ClassicCombatOperation) -> ProfileValue {
        map(vec![
            ("convention", str_("cdecl")),
            (
                "arguments",
                ProfileValue::List(
                    operation
                        .fields()
                        .iter()
                        .map(|field| map(vec![("kind", str_("field")), ("field", str_(field.name()))]))
                        .collect(),
                ),
            ),
        ])
    }

    fn stock_tree() -> ProfileValue {
        let profile = classic_primary_world_profile(&ContentDigest::new("sha256", XATRIX_DIGEST_VALUE)).unwrap();
        let combat = &profile.combat;
        let table = &profile.inventory_table;
        map(vec![
            (
                "calls",
                map(vec![
                    ("pain", call(ClassicCombatOperation::Pain)),
                    ("death", call(ClassicCombatOperation::Death)),
                    ("damage", call(ClassicCombatOperation::Damage)),
                    ("regularArmor", call(ClassicCombatOperation::RegularArmor)),
                    ("powerArmor", call(ClassicCombatOperation::PowerArmor)),
                ]),
            ),
            ("game", str_(combat.game.name())),
            ("entityBytes", int(combat.entity_bytes as i64)),
            (
                "fields",
                map(vec![
                    ("health", int(combat.fields.health as i64)),
                    ("damageable", int(combat.fields.damageable as i64)),
                    ("flags", int(combat.fields.flags as i64)),
                    ("mass", int(combat.fields.mass as i64)),
                    ("velocity", int(combat.fields.velocity as i64)),
                    ("pain", int(combat.fields.pain as i64)),
                    ("die", int(combat.fields.die as i64)),
                ]),
            ),
            (
                "client",
                map(vec![
                    ("inventory", int(combat.client.inventory as i64)),
                    ("inventoryCount", int(combat.client.inventory_count as i64)),
                    ("maxGrenades", int(combat.client.max_grenades as i64)),
                    ("invincibleFrame", int(combat.client.invincible_frame as i64)),
                    ("userinfo", int(combat.client.userinfo as i64)),
                    ("viewAngles", int(combat.client.view_angles as i64)),
                    ("userinfoBytes", int(combat.client.userinfo_bytes as i64)),
                ]),
            ),
            (
                "entries",
                map(vec![
                    ("damage", int(i64::from(combat.entries.damage))),
                    ("powerArmor", int(i64::from(combat.entries.power_armor))),
                    ("regularArmor", int(i64::from(combat.entries.regular_armor))),
                    ("spawn", int(i64::from(combat.entries.spawn))),
                    ("free", int(i64::from(combat.entries.free))),
                ]),
            ),
            (
                "globals",
                map(vec![
                    ("levelFrame", int(i64::from(combat.globals.level_frame))),
                    ("itemList", int(i64::from(combat.globals.item_list))),
                    ("itemBytes", int(combat.globals.item_bytes as i64)),
                ]),
            ),
            (
                "items",
                map(vec![
                    ("jacket", int(combat.items.jacket as i64)),
                    ("combat", int(combat.items.combat as i64)),
                    ("body", int(combat.items.body as i64)),
                    ("screen", int(combat.items.screen as i64)),
                    ("shield", int(combat.items.shield as i64)),
                    ("cells", int(combat.items.cells as i64)),
                    ("grenades", int(combat.items.grenades as i64)),
                ]),
            ),
            (
                "itemFields",
                map(vec![
                    ("className", int(combat.item_fields.class_name as i64)),
                    ("armorInfo", int(combat.item_fields.armor_info as i64)),
                ]),
            ),
            (
                "armorInfo",
                map(vec![
                    ("normalProtection", int(combat.armor_info.normal_protection as i64)),
                    ("energyProtection", int(combat.armor_info.energy_protection as i64)),
                ]),
            ),
            (
                "flags",
                map(vec![
                    ("invulnerable", int(i64::from(combat.flags.invulnerable))),
                    ("notarget", int(i64::from(combat.flags.notarget))),
                    ("noKnockback", int(i64::from(combat.flags.no_knockback))),
                    ("powerArmor", int(i64::from(combat.flags.power_armor))),
                ]),
            ),
            (
                "teams",
                map(vec![
                    ("model", int(i64::from(combat.teams.model))),
                    ("skin", int(i64::from(combat.teams.skin))),
                ]),
            ),
            (
                "armor",
                map(vec![
                    (
                        "regular",
                        ProfileValue::List(combat.armor.regular.iter().map(|value| int(*value as i64)).collect()),
                    ),
                    ("empty", int(combat.armor.empty as i64)),
                ]),
            ),
            (
                "inventoryTable",
                map(vec![
                    ("count", int(table.count as i64)),
                    ("className", int(table.class_name as i64)),
                    ("label", int(table.label as i64)),
                    ("flags", int(table.flags as i64)),
                    ("ammoFlag", int(table.ammo_flag)),
                    ("tag", int(table.tag as i64)),
                    (
                        "capacities",
                        ProfileValue::List(table.capacities.iter().map(|value| int(*value as i64)).collect()),
                    ),
                    (
                        "unnamed",
                        ProfileValue::List(
                            table
                                .unnamed
                                .iter()
                                .map(|entry| {
                                    map(vec![
                                        ("index", int(entry.index as i64)),
                                        ("label", str_(&entry.label)),
                                        ("item", str_(&entry.item)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    ("emptyIndex", int(table.empty_index as i64)),
                    ("sentinel", ProfileValue::Bool(table.sentinel)),
                ]),
            ),
        ])
    }

    #[test]
    fn stock_profile_round_trips_through_saved_values() {
        let digest = ContentDigest::new("sha256", XATRIX_DIGEST_VALUE);
        let expected = classic_primary_world_profile(&digest).unwrap();
        assert_eq!(expected.inventory_table.count, 48);
        assert_eq!(expected.inventory_table.capacities.len(), 8);
        let tree = stock_tree();
        let read = read_classic_primary_world_profile(&ProfileReader::root(&tree), digest).unwrap();
        assert_eq!(read, expected);
    }

    #[test]
    fn reader_rejects_bad_tables_and_calls() {
        let digest = ContentDigest::new("sha256", XATRIX_DIGEST_VALUE);
        let mut tree = stock_tree();
        if let ProfileValue::Map(root) = &mut tree {
            if let Some(ProfileValue::Map(table)) = root.get_mut("inventoryTable") {
                table.insert("count".to_string(), int(500));
            }
        }
        assert!(read_classic_primary_world_profile(&ProfileReader::root(&tree), digest.clone()).is_err());
        let mut tree = stock_tree();
        if let ProfileValue::Map(root) = &mut tree {
            if let Some(ProfileValue::Map(calls)) = root.get_mut("calls") {
                if let Some(ProfileValue::Map(damage)) = calls.get_mut("damage") {
                    damage.insert("convention".to_string(), str_("pascal"));
                }
            }
        }
        assert!(read_classic_primary_world_profile(&ProfileReader::root(&tree), digest).is_err());
        let bad = map(vec![
            ("convention", str_("cdecl")),
            ("arguments", ProfileValue::List(vec![])),
        ]);
        assert!(read_native_combat_call(&ProfileReader::root(&bad), ClassicCombatOperation::Damage).is_err());
        let mut fields: Vec<ProfileValue> = ClassicCombatOperation::PowerArmor
            .fields()
            .iter()
            .map(|field| map(vec![("kind", str_("field")), ("field", str_(field.name()))]))
            .collect();
        fields.push(map(vec![
            ("kind", str_("value")),
            (
                "layout",
                map(vec![("kind", str_("scalar")), ("storage", str_("int32"))]),
            ),
            ("bytes", ProfileValue::List(vec![int(9), int(0), int(0), int(0)])),
        ]));
        fields.push(map(vec![
            ("kind", str_("address")),
            (
                "address",
                map(vec![
                    ("rva", int(0x100)),
                    ("indirections", ProfileValue::List(vec![int(0)])),
                ]),
            ),
        ]));
        fields.push(map(vec![("kind", str_("address")), ("address", ProfileValue::Null)]));
        let mixed = map(vec![
            ("convention", str_("stdcall")),
            ("arguments", ProfileValue::List(fields)),
        ]);
        let mixed_call =
            read_native_combat_call(&ProfileReader::root(&mixed), ClassicCombatOperation::PowerArmor).unwrap();
        assert_eq!(mixed_call.arguments.len(), 8);
        let missing = map(vec![]);
        assert!(namespaced(&ProfileReader::root(&missing)).is_err());
        assert_eq!(namespaced(&ProfileReader::root(&str_("q2:none"))).unwrap(), "q2:none");
        assert!(native_scalar(&ProfileReader::root(&str_("pointer"))).is_err());
        assert_eq!(
            native_scalar(&ProfileReader::root(&str_("float32"))).unwrap(),
            GuestStorage::Float32
        );
    }
}
