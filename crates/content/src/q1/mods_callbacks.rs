//! Mod callback declarations (`src/content/mods/callbacks.ts`).
//!
//! Donor provenance: `src/content/mods/callbacks.ts`
//! (`readModCallbacks`).

use thiserror::Error;

use qa_core::identity::ProviderId;

use crate::contract::{
    CallbackId, DamageTransformResult, InventoryTransformOperation, ModActorField, ModActorFieldBinding,
    ModActorOperation, ModCallback, ModCallbackBinding, ModCallbackBindingKind, ModCallbackDeclaration,
    ModCallbackGlobal, ModCallbackOperation, ModCallbackString, ModCallbackValue, ModClientInput, ModClientInputUpdate,
    ModClientScalarInput, ModConsoleArgumentType, ModConsoleCommand, ModConsoleGlobal, ModConsoleValue, ModCvar,
    ModProgram, ModQcArmorSelection, ModQcArmorSelectionValue, ModQcArmorStage, ModQcClients, ModQcCombat,
    ModQcDamageFlags, ModQcDamageScale, ModQcDamageScaleKind, ModQcEmptyArmor, ModQcEmptyArmorItem, ModQcInputOutput,
    ModQcItemCapacity, ModQcItemDefinition, ModQcItemKind, ModQcItemStorage, ModQcItems, ModQcMaskedItem,
    ModQcProtection, ModQcRegularScale, ModQcWeaponMapping, ModQcWeaponModel, ModQcWeaponSelect, ModQcWeaponStage,
    ModQcWeaponValue, ModRuntimeConstant, ModSourceCall, PoweredProtectionKind, QcModClientPresentation,
    QcModHudPresentation, QcModObjectiveStorage, QcModViewPresentation, QcStatement, SourceItemAdmissionMode,
    SourceMatchField, SourceWeaponItem,
};
use crate::held_weapon::{read_held_weapon_declaration, HeldWeaponError};
use crate::item_icon::{read_item_icon_declaration, ItemIconError};
use crate::mods::{
    read_client_output_declarations, read_item_actions, read_mod_client_input, read_mod_pickup_rule,
    read_mod_source_call, read_mod_source_value, read_source_objectives, read_source_team_values, ModsError,
};
use crate::paths::{normalize_resource_path, PathError};
use crate::value::{namespaced, parse_save_json, read_digest, read_vector, SaveJson, SaveReader, ValueError};

use super::quakec::weapon_stage_declaration::read_qc_weapon_stage_declaration;
use super::quakec::QcError;

/// Mod callbacks reader failure.
#[derive(Debug, Error)]
pub enum ModCallbacksError {
    /// Malformed declaration value.
    #[error(transparent)]
    Value(#[from] ValueError),
    /// Malformed mod declaration value.
    #[error(transparent)]
    Mods(#[from] ModsError),
    /// Malformed resource path.
    #[error(transparent)]
    Path(#[from] PathError),
    /// Malformed held-weapon declaration.
    #[error(transparent)]
    HeldWeapon(#[from] HeldWeaponError),
    /// Malformed item icon declaration.
    #[error(transparent)]
    ItemIcon(#[from] ItemIconError),
    /// Malformed QuakeC stage declaration.
    #[error(transparent)]
    Qc(#[from] QcError),
}

/// Read a bounded integer field as `u32`.
fn u32_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<u32, ModCallbacksError> {
    let field = reader.field(name);
    let value = field.integer(minimum)?;
    u32::try_from(value).map_err(|_| ModCallbacksError::from(field.fail("expected an integer in range")))
}

/// Read a bounded integer field as `u64`.
fn u64_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<u64, ModCallbacksError> {
    let field = reader.field(name);
    let value = field.integer(minimum)?;
    u64::try_from(value).map_err(|_| ModCallbacksError::from(field.fail("expected an integer in range")))
}

/// Read a bounded integer field as `f64`.
fn f64_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<f64, ModCallbacksError> {
    Ok(reader.field(name).integer(minimum)? as f64)
}

/// Read a client input/output binding (donor `inputOutput`).
fn read_input_output(reader: SaveReader) -> Result<ModQcInputOutput, ModsError> {
    if reader.field("kind").choice_str(&["field", "handler"])? == "field" {
        Ok(ModQcInputOutput::Field {
            field: reader.field("field").string()?,
        })
    } else {
        Ok(ModQcInputOutput::Handler {
            function: reader.field("function").string()?,
            inputs: reader.field("inputs").list(|value| {
                Ok::<_, ModsError>(
                    match value
                        .choice_str(&["attack", "jump", "impulse", "forward-move", "side-move", "up-move"])?
                        .as_str()
                    {
                        "attack" => ModClientScalarInput::Attack,
                        "jump" => ModClientScalarInput::Jump,
                        "impulse" => ModClientScalarInput::Impulse,
                        "forward-move" => ModClientScalarInput::ForwardMove,
                        "side-move" => ModClientScalarInput::SideMove,
                        _ => ModClientScalarInput::UpMove,
                    },
                )
            })?,
        })
    }
}

/// Read an objective storage reference (donor `objectiveStorage`).
fn read_objective_storage(reader: SaveReader) -> Result<QcModObjectiveStorage, ModsError> {
    if matches!(reader.value, Some(SaveJson::String(_))) {
        return Ok(QcModObjectiveStorage::Named(reader.string()?));
    }
    reader.field("kind").literal_str("entity-field")?;
    Ok(QcModObjectiveStorage::EntityField {
        global: reader.field("global").string()?,
        indirections: reader.field("indirections").list(|value| value.string())?,
        field: reader.field("field").string()?,
    })
}

/// Read an actor field binding (donor `field`).
fn read_actor_field(reader: SaveReader) -> Result<ModActorField, ModCallbacksError> {
    let binding = reader.field("binding").choice_str(&[
        "health",
        "origin",
        "velocity",
        "angles",
        "bounds-min",
        "bounds-max",
        "think",
        "nextthink",
        "inventory",
        "constant",
        "private",
        "classname",
        "client-flags",
        "view-offset",
        "userinfo",
        "client-input",
        "team",
        "score",
    ])?;
    let kind = match binding.as_str() {
        "health" => ModActorFieldBinding::Health,
        "origin" => ModActorFieldBinding::Origin,
        "velocity" => ModActorFieldBinding::Velocity,
        "angles" => ModActorFieldBinding::Angles,
        "bounds-min" => ModActorFieldBinding::BoundsMin,
        "bounds-max" => ModActorFieldBinding::BoundsMax,
        "think" => ModActorFieldBinding::Think,
        "nextthink" => ModActorFieldBinding::Nextthink,
        "inventory" => ModActorFieldBinding::Inventory {
            item: namespaced(reader.field("item"))?,
        },
        "constant" => ModActorFieldBinding::Constant(
            match read_mod_source_value(reader.field("value")).map_err(ModCallbacksError::from)? {
                ModCallbackValue::Float(value) => ModRuntimeConstant::Float(value),
                ModCallbackValue::Str(value) => ModRuntimeConstant::Str(value),
                ModCallbackValue::Vector(value) => ModRuntimeConstant::Vector(value),
                ModCallbackValue::Input(_) => {
                    return Err(ModCallbacksError::from(
                        reader
                            .field("value")
                            .fail("actor field constants cannot reference callback inputs"),
                    ));
                }
            },
        ),
        "private" => ModActorFieldBinding::Private,
        "classname" => ModActorFieldBinding::Classname,
        "client-flags" => ModActorFieldBinding::ClientFlags {
            grounded: if reader.field("grounded").is_missing() {
                false
            } else {
                reader.field("grounded").literal_bool(true)?;
                true
            },
            private_mask: if reader.field("privateMask").is_missing() {
                None
            } else {
                Some(f64_field(&reader, "privateMask", 0)?)
            },
        },
        "view-offset" => ModActorFieldBinding::ViewOffset,
        "userinfo" => ModActorFieldBinding::Userinfo {
            key: reader.field("key").string()?,
        },
        "client-input" => ModActorFieldBinding::ClientInput {
            input: match reader
                .field("input")
                .choice_str(&[
                    "view-angles",
                    "attack",
                    "jump",
                    "impulse",
                    "forward-move",
                    "side-move",
                    "up-move",
                ])?
                .as_str()
            {
                "view-angles" => ModClientInput::ViewAngles,
                "attack" => ModClientInput::Attack,
                "jump" => ModClientInput::Jump,
                "impulse" => ModClientInput::Impulse,
                "forward-move" => ModClientInput::ForwardMove,
                "side-move" => ModClientInput::SideMove,
                _ => ModClientInput::UpMove,
            },
            update: match reader.field("update").choice_str(&["always", "nonzero"])?.as_str() {
                "always" => ModClientInputUpdate::Always,
                _ => ModClientInputUpdate::Nonzero,
            },
            scale: if reader.field("scale").is_missing() {
                None
            } else {
                Some(reader.field("scale").number()?)
            },
        },
        "team" => ModActorFieldBinding::Match(SourceMatchField::Team {
            values: read_source_team_values(reader.field("values")).map_err(ModCallbacksError::from)?,
        }),
        _ => ModActorFieldBinding::Match(SourceMatchField::Score),
    };
    Ok(ModActorField {
        field: reader.field("field").string()?,
        binding: kind,
    })
}

/// Read a callback (donor `callback`).
fn read_callback(reader: SaveReader) -> Result<ModCallback, ModCallbacksError> {
    let arguments: Vec<ModCallbackValue> = reader
        .field("arguments")
        .list(|value| read_mod_source_value(value).map_err(ModCallbacksError::from))?;
    if arguments.len() > 8 {
        return Err(ModCallbacksError::from(
            reader.fail("source callbacks accept at most eight arguments"),
        ));
    }
    let operation = match reader
        .field("operation")
        .choice_str(&[
            "damage",
            "inventory.give",
            "inventory.consume",
            "actor.think",
            "actor.touch",
            "actor.use",
            "actor.pain",
            "actor.die",
        ])?
        .as_str()
    {
        "damage" => ModCallbackOperation::Damage,
        "inventory.give" => ModCallbackOperation::InventoryGive,
        "inventory.consume" => ModCallbackOperation::InventoryConsume,
        "actor.think" => ModCallbackOperation::Actor(ModActorOperation::Think),
        "actor.touch" => ModCallbackOperation::Actor(ModActorOperation::Touch),
        "actor.use" => ModCallbackOperation::Actor(ModActorOperation::Use),
        "actor.pain" => ModCallbackOperation::Actor(ModActorOperation::Pain),
        _ => ModCallbackOperation::Actor(ModActorOperation::Die),
    };
    let kind = match reader
        .field("stage")
        .choice_str(&["observe", "transform", "replace"])?
        .as_str()
    {
        "observe" => ModCallbackBindingKind::Observe { operation },
        "transform" => match operation {
            ModCallbackOperation::Damage => ModCallbackBindingKind::DamageTransform {
                result: match reader.field("result").choice_str(&["amount", "knockback"])?.as_str() {
                    "amount" => DamageTransformResult::Amount,
                    _ => DamageTransformResult::Knockback,
                },
            },
            ModCallbackOperation::InventoryGive => {
                reader.field("result").literal_str("amount")?;
                ModCallbackBindingKind::InventoryTransform {
                    operation: InventoryTransformOperation::Give,
                }
            }
            ModCallbackOperation::InventoryConsume => {
                reader.field("result").literal_str("amount")?;
                ModCallbackBindingKind::InventoryTransform {
                    operation: InventoryTransformOperation::Consume,
                }
            }
            ModCallbackOperation::Actor(_) => {
                return Err(ModCallbacksError::from(
                    reader.fail("actor callbacks support observation or replacement"),
                ));
            }
        },
        _ => match operation {
            ModCallbackOperation::Actor(operation) => {
                reader.field("result").literal_str("boolean")?;
                ModCallbackBindingKind::ActorReplace { operation }
            }
            _ => {
                return Err(ModCallbacksError::from(
                    reader.fail("this source contract replaces actor callbacks only"),
                ));
            }
        },
    };
    let id = CallbackId(namespaced(reader.field("id"))?);
    Ok(ModCallback {
        call: ModSourceCall {
            function: reader.field("function").string()?,
            arguments,
            globals: reader.field("globals").list(|entry| {
                Ok::<_, ModCallbacksError>(ModCallbackGlobal {
                    name: entry.field("name").string()?,
                    value: read_mod_source_value(entry.field("value")).map_err(ModCallbacksError::from)?,
                })
            })?,
        },
        binding: ModCallbackBinding { id, binding: kind },
    })
}

/// Read a console value (donor `consoleValue`).
fn read_console_value(reader: SaveReader) -> Result<ModConsoleValue, ModCallbacksError> {
    Ok(
        match reader
            .field("kind")
            .choice_str(&[
                "float",
                "string",
                "vector",
                "argument",
                "arguments-text",
                "argument-count",
            ])?
            .as_str()
        {
            "float" => ModConsoleValue::Float(reader.field("value").number()?),
            "string" => ModConsoleValue::Str(ModCallbackString(reader.field("value").string()?)),
            "vector" => ModConsoleValue::Vector(read_vector(reader.field("value"))?),
            "argument" => ModConsoleValue::Argument {
                index: u64_field(&reader, "index", 0)?,
                r#type: match reader.field("type").choice_str(&["string", "float"])?.as_str() {
                    "string" => ModConsoleArgumentType::Str,
                    _ => ModConsoleArgumentType::Float,
                },
            },
            "arguments-text" => ModConsoleValue::ArgumentsText,
            _ => ModConsoleValue::ArgumentCount,
        },
    )
}

/// Read an item capacity (donor `capacity`).
fn read_capacity(reader: SaveReader) -> Result<ModQcItemCapacity, ModCallbacksError> {
    if reader.field("kind").choice_str(&["constant", "field"])? == "constant" {
        Ok(ModQcItemCapacity::Constant {
            value: reader.field("value").finite()?,
        })
    } else {
        Ok(ModQcItemCapacity::Field {
            field: reader.field("field").string()?,
        })
    }
}

/// Read an item definition (donor `items` definition entry).
fn read_item_definition(reader: SaveReader) -> Result<ModQcItemDefinition, ModCallbacksError> {
    let icon = reader.field("icon");
    let actions = reader.field("actions");
    let ammo = reader.field("ammo");
    let held = reader.field("held");
    Ok(ModQcItemDefinition {
        item: namespaced(reader.field("item"))?,
        label: reader.field("label").string()?,
        icon: if icon.is_missing() {
            None
        } else {
            icon.nullable(|value| read_item_icon_declaration(&value).map_err(ModCallbacksError::from))?
        },
        admission: match reader
            .field("admission")
            .choice_str(&["add", "replace-primary"])?
            .as_str()
        {
            "add" => SourceItemAdmissionMode::Add,
            _ => SourceItemAdmissionMode::ReplacePrimary,
        },
        actions: if actions.is_missing() {
            None
        } else {
            Some(read_item_actions(actions, read_mod_source_call).map_err(ModCallbacksError::from)?)
        },
        kind: if reader.field("kind").choice_str(&["counter", "weapon"])? == "counter" {
            ModQcItemKind::Counter
        } else {
            ModQcItemKind::Weapon(SourceWeaponItem {
                ammo: if matches!(ammo.value, Some(SaveJson::Null)) {
                    None
                } else {
                    Some(namespaced(ammo)?)
                },
                held: if held.is_missing() {
                    None
                } else {
                    Some(read_held_weapon_declaration(&held).map_err(ModCallbacksError::from)?)
                },
            })
        },
    })
}

/// Read an item storage entry (donor `items` storage entry).
fn read_item_storage(reader: SaveReader) -> Result<ModQcItemStorage, ModCallbacksError> {
    let field = reader.field("field").string()?;
    if reader.field("kind").choice_str(&["counter", "bits"])? == "bits" {
        Ok(ModQcItemStorage::Bits {
            field,
            private_mask: f64_field(&reader, "privateMask", 0)?,
            items: reader.field("items").list(|value| {
                Ok::<_, ModCallbacksError>(ModQcMaskedItem {
                    item: namespaced(value.field("item"))?,
                    mask: f64_field(&value, "mask", 1)?,
                })
            })?,
        })
    } else {
        let capacity = reader.field("capacity");
        Ok(ModQcItemStorage::Counter {
            field,
            item: namespaced(reader.field("item"))?,
            capacity: read_capacity(capacity)?,
        })
    }
}

/// Read mod items (donor `items`).
fn read_items(reader: SaveReader) -> Result<ModQcItems, ModCallbacksError> {
    let weapons = reader.field("weapons");
    let mapping = |reader: SaveReader| -> Result<ModQcWeaponMapping, ModCallbacksError> {
        Ok(ModQcWeaponMapping {
            field: reader.field("field").string()?,
            values: reader.field("values").list(|value| {
                Ok::<_, ModCallbacksError>(ModQcWeaponValue {
                    value: value.field("value").finite()?,
                    item: namespaced(value.field("item"))?,
                })
            })?,
        })
    };
    Ok(ModQcItems {
        definitions: reader.field("definitions").list(read_item_definition)?,
        storage: reader.field("storage").list(read_item_storage)?,
        weapons: if weapons.is_missing() {
            None
        } else {
            let select = weapons.field("select");
            let model = weapons.field("model");
            let selected = mapping(weapons.field("selected"))?;
            let select_mapping = mapping(select.clone())?;
            Some(ModQcWeaponStage {
                stage: read_qc_weapon_stage_declaration(weapons.field("stage"))?,
                selected,
                select: ModQcWeaponSelect {
                    field: select_mapping.field,
                    values: select_mapping.values,
                    call: read_mod_source_call(select.field("call")).map_err(ModCallbacksError::from)?,
                },
                resume: weapons
                    .field("resume")
                    .list(|value| read_mod_source_call(value).map_err(ModCallbacksError::from))?,
                model: ModQcWeaponModel {
                    field: model.field("field").string()?,
                    frame: model.field("frame").string()?,
                },
            })
        },
    })
}

/// Read an armor stage declaration (donor `armorStage`).
fn read_armor_stage(reader: SaveReader) -> Result<ModQcArmorStage, ModCallbacksError> {
    let flags = reader.field("flags");
    let regular_scale = reader.field("regularScale");
    Ok(ModQcArmorStage {
        function: reader.field("function").string()?,
        entry: u32_field(&reader, "entry", 0)?,
        exit: u32_field(&reader, "exit", 0)?,
        target: u32_field(&reader, "target", 0)?,
        damage: u32_field(&reader, "damage", 0)?,
        saved: u32_field(&reader, "saved", 0)?,
        regular_scale: if regular_scale.is_missing() {
            Vec::new()
        } else {
            regular_scale.list(|site| {
                Ok::<_, ModCallbacksError>(ModQcRegularScale {
                    caller: site.field("caller").string()?,
                    statement: u32_field(&site, "statement", 0)?,
                    scale: site.field("scale").number()?,
                })
            })?
        },
        flags: if flags.field("kind").choice_str(&["none", "bits"])? == "none" {
            ModQcDamageFlags::None
        } else {
            ModQcDamageFlags::Bits {
                word: u32_field(&flags, "word", 0)?,
                no_armor: u32_field(&flags, "noArmor", 0)?,
                no_power_armor: u32_field(&flags, "noPowerArmor", 0)?,
                no_regular_armor: u32_field(&flags, "noRegularArmor", 0)?,
                energy: u32_field(&flags, "energy", 0)?,
            }
        },
        statements: reader.field("statements").list(|statement| {
            Ok::<_, ModCallbacksError>(QcStatement {
                opcode: u32_field(&statement, "opcode", 0)?,
                a: u32_field(&statement, "a", 0)?,
                b: u32_field(&statement, "b", 0)?,
                c: u32_field(&statement, "c", 0)?,
            })
        })?,
    })
}

/// Read a protection admission choice (donor `readProtectionAdmission`).
fn admission_choice(reader: SaveReader) -> Result<crate::contract::ModProtectionAdmission, ModCallbacksError> {
    match reader
        .field("kind")
        .choice_str(&["claim", "replace-primary", "replace-current-primary"])?
        .as_str()
    {
        "replace-primary" => {
            let owner = namespaced(reader.field("owner"))?;
            let (namespace, name) = owner
                .split_once(':')
                .ok_or_else(|| ModCallbacksError::from(reader.field("owner").fail("expected a namespaced identity")))?;
            Ok(crate::contract::ModProtectionAdmission::ReplacePrimary {
                owner: ProviderId::new(namespace, name),
            })
        }
        "replace-current-primary" => Ok(crate::contract::ModProtectionAdmission::ReplaceCurrentPrimary),
        _ => Ok(crate::contract::ModProtectionAdmission::Claim),
    }
}

/// Read a protection binding (donor `protection`).
fn read_protection(reader: SaveReader) -> Result<ModQcProtection, ModCallbacksError> {
    let admission = reader.field("admission");
    let source = reader.field("absorb");
    let flags = reader.field("flags");
    let storage = reader.field("storage");
    let selection = storage.field("selection");
    let absorb = if source.field("kind").choice_str(&["function", "region"])? == "function" {
        crate::contract::ModQcAbsorb::Function {
            call: read_mod_source_call(source.field("call")).map_err(ModCallbacksError::from)?,
        }
    } else {
        crate::contract::ModQcAbsorb::Region {
            call: read_mod_source_call(source.field("call")).map_err(ModCallbacksError::from)?,
            stage: read_armor_stage(source.field("stage"))?,
        }
    };
    let channel = reader.field("channel").choice_str(&["regular", "powered"])?;
    let selection_values = |selection: &SaveReader| -> Result<
        Vec<ModQcArmorSelectionValue<Option<crate::contract::ItemId>>>,
        ModCallbacksError,
    > {
        selection.field("values").list(|entry| {
            let item = entry.field("item");
            Ok::<_, ModCallbacksError>(ModQcArmorSelectionValue {
                value: entry.field("value").number()?,
                selected: if matches!(item.value, Some(SaveJson::Null)) {
                    None
                } else {
                    Some(namespaced(item)?)
                },
            })
        })
    };
    let powered_values =
        |selection: &SaveReader| -> Result<Vec<ModQcArmorSelectionValue<PoweredProtectionKind>>, ModCallbacksError> {
            selection.field("values").list(|entry| {
                Ok::<_, ModCallbacksError>(ModQcArmorSelectionValue {
                    value: entry.field("value").number()?,
                    selected: match entry.field("kind").choice_str(&["none", "screen", "shield"])?.as_str() {
                        "none" => PoweredProtectionKind::None,
                        "screen" => PoweredProtectionKind::Screen,
                        _ => PoweredProtectionKind::Shield,
                    },
                })
            })
        };
    Ok(ModQcProtection {
        id: namespaced(reader.field("id"))?,
        absorb,
        admission: if admission.is_missing() {
            None
        } else {
            Some(admission_choice(admission)?)
        },
        flags: crate::contract::ModQcProtectionFlags {
            no_armor: u32_field(&flags, "noArmor", 0)?,
            no_power_armor: u32_field(&flags, "noPowerArmor", 0)?,
            no_regular_armor: u32_field(&flags, "noRegularArmor", 0)?,
            energy: u32_field(&flags, "energy", 0)?,
            radius: u32_field(&flags, "radius", 0)?,
        },
        channel: if channel == "regular" {
            crate::contract::ModQcProtectionChannel::Regular {
                points: storage.field("points").string()?,
                item: {
                    let item = storage.field("item");
                    if matches!(item.value, Some(SaveJson::Null)) {
                        None
                    } else {
                        Some(namespaced(item)?)
                    }
                },
                selection: if selection.is_missing() {
                    None
                } else {
                    let mask = selection.field("mask");
                    Some(ModQcArmorSelection {
                        field: selection.field("field").string()?,
                        mask: if mask.is_missing() {
                            None
                        } else {
                            Some(f64_field(&selection, "mask", 0)?)
                        },
                        values: selection_values(&selection)?,
                    })
                },
            }
        } else {
            crate::contract::ModQcProtectionChannel::Powered {
                cells: storage.field("cells").string()?,
                kind: match storage.field("kind").choice_str(&["screen", "shield"])?.as_str() {
                    "screen" => PoweredProtectionKind::Screen,
                    _ => PoweredProtectionKind::Shield,
                },
                selection: if selection.is_missing() {
                    None
                } else {
                    let mask = selection.field("mask");
                    Some(ModQcArmorSelection {
                        field: selection.field("field").string()?,
                        mask: if mask.is_missing() {
                            None
                        } else {
                            Some(f64_field(&selection, "mask", 0)?)
                        },
                        values: powered_values(&selection)?,
                    })
                },
            }
        },
    })
}

/// Read a combat lowering (donor `readQcCombat`).
fn read_combat(reader: SaveReader) -> Result<ModQcCombat, ModCallbacksError> {
    let damage_scale = reader.field("damageScale");
    let armor_stage = reader.field("armorStage");
    let empty_armor = reader.field("emptyArmor");
    Ok(ModQcCombat {
        damage: read_mod_source_call(reader.field("damage")).map_err(ModCallbacksError::from)?,
        damage_scale: if damage_scale.is_missing() {
            None
        } else {
            Some(ModQcDamageScale {
                kind: if damage_scale.field("kind").is_missing() {
                    ModQcDamageScaleKind::Multiplier
                } else {
                    damage_scale_kind(&damage_scale)?
                },
                function: damage_scale.field("function").string()?,
                entry: u32_field(&damage_scale, "entry", 0)?,
                exit: u32_field(&damage_scale, "exit", 0)?,
                damage: u32_field(&damage_scale, "damage", 28)?,
                statements: damage_scale.field("statements").list(|value| {
                    Ok::<_, ModCallbacksError>(QcStatement {
                        opcode: u32_field(&value, "opcode", 0)?,
                        a: u32_field(&value, "a", 0)?,
                        b: u32_field(&value, "b", 0)?,
                        c: u32_field(&value, "c", 0)?,
                    })
                })?,
            })
        },
        armor_stage: if armor_stage.is_missing() {
            None
        } else {
            Some(read_armor_stage(armor_stage)?)
        },
        empty_armor: if empty_armor.is_missing() {
            None
        } else {
            Some(ModQcEmptyArmor {
                item: match empty_armor
                    .field("item")
                    .choice_str(&["q1:item_armor1", "q1:item_armor2", "q1:item_armorInv"])?
                    .as_str()
                {
                    "q1:item_armor1" => ModQcEmptyArmorItem::Armor1,
                    "q1:item_armor2" => ModQcEmptyArmorItem::Armor2,
                    _ => ModQcEmptyArmorItem::ArmorInv,
                },
                absorption: empty_armor.field("absorption").finite()?,
            })
        },
    })
}

/// Read a damage scale kind.
fn damage_scale_kind(reader: &SaveReader) -> Result<ModQcDamageScaleKind, ModCallbacksError> {
    Ok(
        match reader
            .field("kind")
            .choice_str(&["multiplier", "identity", "transform"])?
            .as_str()
        {
            "identity" => ModQcDamageScaleKind::Identity,
            "transform" => ModQcDamageScaleKind::Transform,
            _ => ModQcDamageScaleKind::Multiplier,
        },
    )
}

/// Read a mod callback declaration file (donor `readModCallbacks`).
pub fn read_mod_callbacks(bytes: &[u8]) -> Result<ModCallbackDeclaration, ModCallbacksError> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ModCallbacksError::from(ModsError::Invalid("Declaration bytes are not valid UTF-8".to_string()))
    })?;
    let value = parse_save_json(text)?;
    let reader = SaveReader::new(&value);
    let program = reader.field("program");
    let presentation = reader.field("clientPresentation");
    let clients = reader.field("clients");
    let outputs = clients.field("outputs");
    let combat = reader.field("combat");
    let initialize = reader.field("initialize");
    let frame = reader.field("frame");
    let cvars = reader.field("cvars");
    let commands = reader.field("commands");
    reader.field("version").literal_i64(1)?;
    reader.field("runtime").literal_str("quakec")?;
    Ok(ModCallbackDeclaration {
        version: 1,
        objectives: if reader.field("objectives").is_missing() {
            Vec::new()
        } else {
            read_source_objectives(
                reader.field("objectives"),
                read_objective_storage,
                read_objective_storage,
                read_mod_source_call,
            )
            .map_err(ModCallbacksError::from)?
        },
        program: ModProgram {
            path: normalize_resource_path(&program.field("path").string()?)?,
            digest: read_digest(program.field("digest"))?,
        },
        actor_fields: reader.field("actorFields").list(read_actor_field)?,
        callbacks: reader.field("callbacks").list(read_callback)?,
        client_presentation: if presentation.is_missing() {
            None
        } else {
            Some(QcModClientPresentation {
                hud: match presentation
                    .field("hud")
                    .choice_str(&["none", "replace-vitals"])?
                    .as_str()
                {
                    "none" => QcModHudPresentation::None,
                    _ => QcModHudPresentation::ReplaceVitals,
                },
                view: match presentation.field("view").choice_str(&["none", "set-view"])?.as_str() {
                    "none" => QcModViewPresentation::None,
                    _ => QcModViewPresentation::SetView,
                },
            })
        },
        clients: if clients.is_missing() {
            None
        } else {
            let input = clients.field("input");
            Some(ModQcClients {
                maximum: u64_field(&clients, "maximum", 1)?,
                outputs: if outputs.is_missing() {
                    Vec::new()
                } else {
                    read_client_output_declarations(
                        outputs,
                        |value| value.string().map_err(ModsError::from),
                        |value| value.string().map_err(ModsError::from),
                    )
                    .map_err(ModCallbacksError::from)?
                },
                admit: clients
                    .field("admit")
                    .list(|value| read_mod_source_call(value).map_err(ModCallbacksError::from))?,
                userinfo: clients
                    .field("userinfo")
                    .list(|value| read_mod_source_call(value).map_err(ModCallbacksError::from))?,
                disconnect: clients
                    .field("disconnect")
                    .list(|value| read_mod_source_call(value).map_err(ModCallbacksError::from))?,
                frame: if clients.field("frame").is_missing() {
                    Vec::new()
                } else {
                    clients
                        .field("frame")
                        .list(|value| read_mod_source_call(value).map_err(ModCallbacksError::from))?
                },
                input: if input.is_missing() {
                    Vec::new()
                } else {
                    read_mod_client_input(input, read_mod_source_call, Some(read_input_output))
                        .map_err(ModCallbacksError::from)?
                },
            })
        },
        initialize: if initialize.is_missing() {
            Vec::new()
        } else {
            initialize.list(|value| read_mod_source_call(value).map_err(ModCallbacksError::from))?
        },
        frame: if frame.is_missing() {
            None
        } else {
            Some(read_mod_source_call(frame).map_err(ModCallbacksError::from)?)
        },
        cvars: if cvars.is_missing() {
            Vec::new()
        } else {
            cvars.list(|entry| {
                Ok::<_, ModCallbacksError>(ModCvar {
                    name: entry.field("name").string()?,
                    value: entry.field("value").string()?,
                })
            })?
        },
        commands: if commands.is_missing() {
            Vec::new()
        } else {
            commands.list(|entry| {
                Ok::<_, ModCallbacksError>(ModConsoleCommand {
                    name: entry.field("name").string()?,
                    function: entry.field("function").string()?,
                    arguments: entry.field("arguments").list(read_console_value)?,
                    globals: entry.field("globals").list(|global| {
                        Ok::<_, ModCallbacksError>(ModConsoleGlobal {
                            name: global.field("name").string()?,
                            value: read_console_value(global.field("value"))?,
                        })
                    })?,
                })
            })?
        },
        combat: if combat.is_missing() {
            None
        } else {
            Some(read_combat(combat)?)
        },
        protection: if reader.field("protection").is_missing() {
            Vec::new()
        } else {
            reader.field("protection").list(read_protection)?
        },
        pickups: if reader.field("pickups").is_missing() {
            Vec::new()
        } else {
            reader
                .field("pickups")
                .list(|entry| read_mod_pickup_rule(entry, read_mod_source_call).map_err(ModCallbacksError::from))?
        },
        items: if reader.field("items").is_missing() {
            None
        } else {
            Some(read_items(reader.field("items"))?)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_utf8_and_malformed_declarations() {
        assert!(read_mod_callbacks(&[0xFF, 0xFE]).is_err());
        assert!(read_mod_callbacks(b"{}").is_err());
        assert!(read_mod_callbacks(b"{\"program\": 1}").is_err());
    }
}
