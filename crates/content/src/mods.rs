//! Mod declaration readers ported from `src/content/mods/*`.
//!
//! Donors: `client-input.ts`, `client-outputs.ts`, `item-actions.ts`,
//! `match.ts`, `native-callbacks.ts`, `pickups.ts`, `qvm-items.ts`,
//! `selection.ts`, `source-call.ts`. The QuakeC/QVM declaration readers
//! (`callbacks.ts`, `qvm-callbacks.ts`, `qvm-presentation.ts`,
//! `declaration.ts`, `catalog.ts`) are out of scope.

use std::collections::{HashMap, HashSet};

use qa_core::identity::ProviderId;
use thiserror::Error;

use crate::contract::{
    CallbackId, ContractError, GrantAcceptance, ItemId, ItemTestComparison, ModActorInput, ModActorOperation,
    ModCallbackBinding, ModCallbackBindingKind, ModCallbackGlobal, ModCallbackInput, ModCallbackOperation,
    ModCallbackString, ModCallbackValue, ModClientInput, ModClientInputBinding, ModClientInputPhase,
    ModClientInputScope, ModClientMovementMode, ModClientOutputDeclaration, ModClientScalarInput, ModDescription,
    ModPickupRule, ModProtectionAdmission, ModPurpose, ModSelection, ModSourceCall, ModTimeInput, ModTimeUnits,
    NativeItemCapacity, NativeItemDefinition, NativeItemKind, NativeItemPointer, NativeItemStorage, NativeItemTest,
    NativeModAcceptance, NativeModActorField, NativeModActorRecord, NativeModAddress, NativeModAdmissionCall,
    NativeModArmor, NativeModArmorField, NativeModArmorSelection, NativeModCallback, NativeModClientInputField,
    NativeModClientInputValue, NativeModClients, NativeModCombat, NativeModCombatFlags, NativeModDamageCauses,
    NativeModDamageEntry, NativeModDeclaration, NativeModDeferredDamage, NativeModEntry, NativeModGlobal,
    NativeModInputOutput, NativeModItems, NativeModMaskedField, NativeModNextthink, NativeModObjectiveStorage,
    NativeModPickup, NativeModPickupContext, NativeModPickupValue, NativeModPose, NativeModPowerArmorItem,
    NativeModProtectionAbsorb, NativeModProtectionChannel, NativeModProtectionDefinition, NativeModProtectionRegion,
    NativeModQ2ArmorCall, NativeModQ2ArmorCheck, NativeModRecordBase, NativeModRegionFrame, NativeModRegionInput,
    NativeModRegionLocation, NativeModRegionRegister, NativeModRegionStorage, NativeModRegularAbsorb,
    NativeModRegularArmorItem, NativeModReturn, NativeModScalar, NativeModScalarField, NativeModScalarValueKind,
    NativeModSharedActorBinding, NativeModSharedActorField, NativeModSkip, NativeModSourceActorCallbacks,
    NativeModSourceActorFields, NativeModSourceActors, NativeModSourceCall, NativeModTarget, NativeModUpdate,
    NativeModValue, NativeModValueKind, NativePowerArmorKind, NativeWeaponClearedField, NativeWeaponDecision,
    NativeWeaponDispatcher, NativeWeaponSelection, NativeWeaponStage, NativeWeaponValue, ObjectiveId,
    OriginalPickupOperation, PickupWrite, PickupWriteFields, ProtectionChannel, Q2CallbackAbi, QvmItemCapacity,
    QvmItemCapacityOverride, QvmItemDefinition, QvmItemField, QvmItemKind, QvmItemStorage, QvmItemTest,
    QvmItemsWeaponInput, QvmItemsWeaponStage, QvmMaskedItem, QvmModInputPointer, QvmModInputPointerBase,
    QvmWeaponActor, QvmWeaponCall, QvmWeaponContinuation, QvmWeaponDispatcher, QvmWeaponPredicate, QvmWeaponProjection,
    QvmWeaponRequest, QvmWeaponSelection, QvmWeaponStage, QvmWeaponValue, SourceItemActionCalls,
    SourceItemAdmissionMode, SourceMatchField, SourceObjectiveDeclaration, SourceObjectiveRole, SourceObjectiveValue,
    SourceTeamValue, SourceWeaponItem,
};
use crate::contract::{
    ItemCapacityComparison, ModAvailability, ModCvar, ModProgram, NativeClassicGame, NativeModClock,
};
use crate::held_weapon::{read_held_weapon_declaration, HeldWeaponError};
use crate::item_icon::{read_item_icon_declaration, ItemIconError};
use crate::paths::{normalize_resource_path, PathError};
use crate::value::{namespaced, parse_save_json, read_digest, read_vector, SaveJson, SaveReader, ValueError};

/// Mod reader failure.
#[derive(Debug, Error)]
pub enum ModsError {
    /// Malformed declaration value.
    #[error(transparent)]
    Value(#[from] ValueError),
    /// Invalid contract identity.
    #[error(transparent)]
    Contract(#[from] ContractError),
    /// Invalid resource path.
    #[error(transparent)]
    Path(#[from] PathError),
    /// Invalid held weapon declaration.
    #[error(transparent)]
    HeldWeapon(#[from] HeldWeaponError),
    /// Invalid item icon declaration.
    #[error(transparent)]
    ItemIcon(#[from] ItemIconError),
    /// Invalid declaration.
    #[error("{0}")]
    Invalid(String),
}

/// Read a bounded integer field as `u64`.
fn u64_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<u64, ModsError> {
    let field = reader.field(name);
    let value = field.integer(minimum)?;
    u64::try_from(value).map_err(|_| ModsError::from(field.fail("expected an integer in range")))
}

/// Read a bounded integer field as `u32`.
fn u32_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<u32, ModsError> {
    let field = reader.field(name);
    let value = field.integer(minimum)?;
    u32::try_from(value).map_err(|_| ModsError::from(field.fail("expected an integer in range")))
}

/// Read a bounded integer field as `f64` (donor `number` semantics).
fn int_number(reader: &SaveReader, name: &str, minimum: i64) -> Result<f64, ModsError> {
    #[allow(clippy::cast_precision_loss)]
    Ok(reader.field(name).integer(minimum)? as f64)
}

/// Convert a checked integer to `u64`.
fn as_u64(reader: &SaveReader, value: i64) -> Result<u64, ModsError> {
    u64::try_from(value).map_err(|_| ModsError::from(reader.fail("expected an integer in range")))
}

/// Convert a checked integer to `u32`.
fn as_u32(reader: &SaveReader, value: i64) -> Result<u32, ModsError> {
    u32::try_from(value).map_err(|_| ModsError::from(reader.fail("expected an integer in range")))
}

/// Convert a checked integer to `f64` (donor `number` semantics).
fn as_int_number(value: i64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    {
        value as f64
    }
}

/// Split a validated `namespace:name` identity into a provider id.
fn provider_id(text: &str) -> ProviderId {
    match text.find(':') {
        Some(colon) => ProviderId::new(&text[..colon], &text[colon + 1..]),
        None => ProviderId::new(text, ""),
    }
}

// `source-call.ts`.

const CALLBACK_INPUTS: &[&str] = &[
    "self",
    "other",
    "activator",
    "attacker",
    "inflictor",
    "amount",
    "damage-flags",
    "regular-protection-scale",
    "knockback",
    "point",
    "direction",
    "normal",
    "item",
    "time",
    "elapsed",
    "result",
    "view-angles",
    "attack",
    "jump",
    "impulse",
    "forward-move",
    "side-move",
    "up-move",
    "pickup-count",
    "pickup-has-count",
    "pickup-dropped",
];

/// Read a callback input name.
fn callback_input(reader: SaveReader) -> Result<ModCallbackInput, ModsError> {
    Ok(match reader.choice_str(CALLBACK_INPUTS)?.as_str() {
        "view-angles" => ModCallbackInput::Client(ModClientInput::ViewAngles),
        "attack" => ModCallbackInput::Client(ModClientInput::Attack),
        "jump" => ModCallbackInput::Client(ModClientInput::Jump),
        "impulse" => ModCallbackInput::Client(ModClientInput::Impulse),
        "forward-move" => ModCallbackInput::Client(ModClientInput::ForwardMove),
        "side-move" => ModCallbackInput::Client(ModClientInput::SideMove),
        "up-move" => ModCallbackInput::Client(ModClientInput::UpMove),
        "self" => ModCallbackInput::Slf,
        "other" => ModCallbackInput::Other,
        "activator" => ModCallbackInput::Activator,
        "attacker" => ModCallbackInput::Attacker,
        "inflictor" => ModCallbackInput::Inflictor,
        "amount" => ModCallbackInput::Amount,
        "damage-flags" => ModCallbackInput::DamageFlags,
        "regular-protection-scale" => ModCallbackInput::RegularProtectionScale,
        "knockback" => ModCallbackInput::Knockback,
        "point" => ModCallbackInput::Point,
        "direction" => ModCallbackInput::Direction,
        "normal" => ModCallbackInput::Normal,
        "item" => ModCallbackInput::Item,
        "time" => ModCallbackInput::Time,
        "elapsed" => ModCallbackInput::Elapsed,
        "result" => ModCallbackInput::Result,
        "pickup-count" => ModCallbackInput::PickupCount,
        "pickup-has-count" => ModCallbackInput::PickupHasCount,
        _ => ModCallbackInput::PickupDropped,
    })
}

/// Read a callback value (shared by the QuakeC and native readers).
fn mod_callback_value(reader: SaveReader) -> Result<ModCallbackValue, ModsError> {
    match reader
        .field("kind")
        .choice_str(&["input", "float", "string", "vector"])?
        .as_str()
    {
        "input" => Ok(ModCallbackValue::Input(callback_input(reader.field("name"))?)),
        "float" => Ok(ModCallbackValue::Float(reader.field("value").number()?)),
        "string" => Ok(ModCallbackValue::Str(ModCallbackString(
            reader.field("value").string()?,
        ))),
        _ => Ok(ModCallbackValue::Vector(read_vector(reader.field("value"))?)),
    }
}

/// Read a QuakeC source call.
pub fn read_mod_source_call(reader: SaveReader) -> Result<ModSourceCall, ModsError> {
    Ok(ModSourceCall {
        function: reader.field("function").string()?,
        arguments: reader.field("arguments").list(mod_callback_value)?,
        globals: reader.field("globals").list(|entry| -> Result<_, ModsError> {
            Ok(ModCallbackGlobal {
                name: entry.field("name").string()?,
                value: mod_callback_value(entry.field("value"))?,
            })
        })?,
    })
}

/// Read a QuakeC source value.
pub fn read_mod_source_value(reader: SaveReader) -> Result<ModCallbackValue, ModsError> {
    mod_callback_value(reader)
}

// `client-input.ts`.

/// Read client input bindings.
pub fn read_mod_client_input<Call, Output, R, O>(
    reader: SaveReader,
    mut read_call: R,
    mut read_output: Option<O>,
) -> Result<Vec<ModClientInputBinding<Call, Output>>, ModsError>
where
    R: FnMut(SaveReader) -> Result<Call, ModsError>,
    O: FnMut(SaveReader) -> Result<Output, ModsError>,
{
    reader.list(|entry| -> Result<_, ModsError> {
        let scope = match entry
            .field("scope")
            .choice_str(&["client-command", "movement-slice"])?
            .as_str()
        {
            "client-command" => ModClientInputScope::ClientCommand,
            _ => ModClientInputScope::MovementSlice,
        };
        let before = entry.field("phase").choice_str(&["before", "after"])? == "before";
        let calls = entry.field("calls").list(&mut read_call)?;
        if entry.field("outputs").is_missing() {
            return Ok(ModClientInputBinding {
                scope,
                calls,
                phase: if before {
                    ModClientInputPhase::Before { outputs: Vec::new() }
                } else {
                    ModClientInputPhase::After
                },
            });
        }
        if !before {
            return Err(entry
                .fail("Input outputs require a declared before source adapter")
                .into());
        }
        let Some(outputs) = read_output.as_mut() else {
            return Err(entry
                .fail("Input outputs require a declared before source adapter")
                .into());
        };
        Ok(ModClientInputBinding {
            scope,
            calls,
            phase: ModClientInputPhase::Before {
                outputs: entry.field("outputs").list(&mut *outputs)?,
            },
        })
    })
}

// `client-outputs.ts`.

/// Read client output declarations.
pub fn read_client_output_declarations<Scalar, Vector, S, V>(
    reader: SaveReader,
    mut scalar: S,
    mut vector: V,
) -> Result<Vec<ModClientOutputDeclaration<Scalar, Vector>>, ModsError>
where
    S: FnMut(SaveReader) -> Result<Scalar, ModsError>,
    V: FnMut(SaveReader) -> Result<Vector, ModsError>,
{
    reader.list(|entry| -> Result<_, ModsError> {
        match entry
            .field("kind")
            .choice_str(&["view-offset", "movement-mode", "stance", "body-shape"])?
            .as_str()
        {
            "body-shape" => Ok(ModClientOutputDeclaration::BodyShape {
                min: vector(entry.field("min"))?,
                max: vector(entry.field("max"))?,
            }),
            "view-offset" => {
                if entry.field("height").is_missing() {
                    return Ok(ModClientOutputDeclaration::ViewOffsetField {
                        field: vector(entry.field("field"))?,
                    });
                }
                if !entry.field("field").is_missing() {
                    return Err(entry
                        .fail("view offset must declare a vector or a scalar height, not both")
                        .into());
                }
                Ok(ModClientOutputDeclaration::ViewHeight {
                    height: scalar(entry.field("height"))?,
                })
            }
            kind => {
                let field = scalar(entry.field("field"))?;
                let mask = if entry.field("mask").is_missing() {
                    None
                } else {
                    Some(int_number(&entry, "mask", 1)?)
                };
                if kind == "movement-mode" {
                    Ok(ModClientOutputDeclaration::MovementMode {
                        field,
                        mask,
                        values: entry.field("values").list(|value| -> Result<_, ModsError> {
                            Ok(crate::contract::ModClientMovementModeValue {
                                value: value.field("value").finite()?,
                                mode: match value
                                    .field("mode")
                                    .choice_str(&["normal", "noclip", "freeze"])?
                                    .as_str()
                                {
                                    "normal" => ModClientMovementMode::Normal,
                                    "noclip" => ModClientMovementMode::Noclip,
                                    _ => ModClientMovementMode::Freeze,
                                },
                            })
                        })?,
                    })
                } else {
                    Ok(ModClientOutputDeclaration::Stance {
                        field,
                        mask,
                        values: entry.field("values").list(|value| -> Result<_, ModsError> {
                            Ok(crate::contract::ModClientStanceValue {
                                value: value.field("value").finite()?,
                                crouched: value.field("crouched").boolean()?,
                            })
                        })?,
                    })
                }
            }
        }
    })
}

// `item-actions.ts`.

/// Read source item action calls.
pub fn read_item_actions<Call>(
    reader: SaveReader,
    mut read_call: impl FnMut(SaveReader) -> Result<Call, ModsError>,
) -> Result<SourceItemActionCalls<Call>, ModsError> {
    let use_call = reader.field("use");
    let drop_call = reader.field("drop");
    if use_call.is_missing() && drop_call.is_missing() {
        return Err(reader.fail("Item actions require an original use or drop call").into());
    }
    Ok(SourceItemActionCalls {
        use_call: if use_call.is_missing() {
            None
        } else {
            Some(read_call(use_call)?)
        },
        drop_call: if drop_call.is_missing() {
            None
        } else {
            Some(read_call(drop_call)?)
        },
    })
}

// `match.ts` (boundary `source-match.ts` shapes used by these readers).

/// Source team alias (donor `source-match.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceTeamAlias {
    /// Original identity.
    pub source: Option<String>,
    /// Shared identity.
    pub team: Option<String>,
}

/// Source team command (donor `source-match.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceTeamCommand {
    /// Original identity.
    pub source: Option<String>,
    /// Shared identity.
    pub team: Option<String>,
    /// Command arguments.
    pub arguments: Vec<String>,
}

/// Source primary match (donor `source-match.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourcePrimaryMatch {
    /// Score offset.
    pub score: i64,
    /// Team commands.
    pub teams: Vec<SourceTeamCommand>,
}

/// Validate a source match field (donor `validateSourceMatchField`).
pub fn validate_source_match_field(field: &SourceMatchField) -> Result<(), ModsError> {
    let SourceMatchField::Team { values } = field else {
        return Ok(());
    };
    let distinct_values: HashSet<u64> = values.iter().map(|value| value.value.to_bits()).collect();
    let distinct_teams: HashSet<&Option<String>> = values.iter().map(|value| &value.team).collect();
    if values.is_empty()
        || distinct_values.len() != values.len()
        || distinct_teams.len() != values.len()
        || values
            .iter()
            .any(|value| !value.value.is_finite() || value.team.as_deref() == Some(""))
    {
        return Err(ModsError::Invalid(
            "Team projection requires distinct original values and shared identities".to_string(),
        ));
    }
    Ok(())
}

/// Read source team values.
pub fn read_source_team_values(reader: SaveReader) -> Result<Vec<SourceTeamValue>, ModsError> {
    let values: Vec<SourceTeamValue> = reader.list(|entry| -> Result<_, ModsError> {
        Ok(SourceTeamValue {
            value: entry.field("value").finite()?,
            team: entry.field("team").nullable(|value| value.string())?,
        })
    })?;
    if let Err(error) = validate_source_match_field(&SourceMatchField::Team { values: values.clone() }) {
        return Err(reader.fail(&error.to_string()).into());
    }
    Ok(values)
}

/// Read source team aliases.
pub fn read_source_team_aliases(reader: SaveReader) -> Result<Vec<SourceTeamAlias>, ModsError> {
    let values: Vec<SourceTeamAlias> = reader.list(|value| -> Result<_, ModsError> {
        Ok(SourceTeamAlias {
            source: value.field("source").nullable(|value| value.string())?,
            team: value.field("team").nullable(|value| value.string())?,
        })
    })?;
    let distinct_source: HashSet<&Option<String>> = values.iter().map(|value| &value.source).collect();
    let distinct_team: HashSet<&Option<String>> = values.iter().map(|value| &value.team).collect();
    if distinct_source.len() != values.len()
        || distinct_team.len() != values.len()
        || values
            .iter()
            .any(|value| value.source.as_deref() == Some("") || value.team.as_deref() == Some(""))
    {
        return Err(reader
            .fail("team aliases require distinct original and shared identities")
            .into());
    }
    Ok(values)
}

/// Read a source primary match.
pub fn read_source_primary_match(reader: SaveReader, client_bytes: u64) -> Result<SourcePrimaryMatch, ModsError> {
    let score = reader.field("score").integer(0)?;
    let teams: Vec<SourceTeamCommand> = reader.field("teams").list(|value| -> Result<_, ModsError> {
        Ok(SourceTeamCommand {
            source: value.field("source").nullable(|value| value.string())?,
            team: value.field("team").nullable(|value| value.string())?,
            arguments: value.field("arguments").list(|value| value.string())?,
        })
    })?;
    let limit = i64::try_from(client_bytes).unwrap_or(i64::MAX);
    if score % 4 != 0 || score + 4 > limit {
        return Err(reader
            .field("score")
            .fail("score is outside the original client record")
            .into());
    }
    let distinct_source: HashSet<&Option<String>> = teams.iter().map(|team| &team.source).collect();
    let distinct_team: HashSet<&Option<String>> = teams.iter().map(|team| &team.team).collect();
    if distinct_source.len() != teams.len()
        || distinct_team.len() != teams.len()
        || teams.iter().any(|team| {
            team.arguments.is_empty()
                || team.arguments.iter().any(|argument| {
                    argument.is_empty() || argument.bytes().any(|byte| byte == 0 || byte == b'\r' || byte == b'\n')
                })
        })
    {
        return Err(reader
            .field("teams")
            .fail("team changes require distinct identities and complete original command arguments")
            .into());
    }
    Ok(SourcePrimaryMatch { score, teams })
}

/// Read source objective declarations.
pub fn read_source_objectives<Scalar, Reference, Call, S, R, C>(
    reader: SaveReader,
    mut scalar: S,
    mut reference: R,
    mut call: C,
) -> Result<Vec<SourceObjectiveDeclaration<Scalar, Reference, Call>>, ModsError>
where
    S: FnMut(SaveReader) -> Result<Scalar, ModsError>,
    R: FnMut(SaveReader) -> Result<Reference, ModsError>,
    C: FnMut(SaveReader) -> Result<Call, ModsError>,
{
    let declarations: Vec<SourceObjectiveDeclaration<Scalar, Reference, Call>> =
        reader.list(|value| -> Result<_, ModsError> {
            let state = value.field("state");
            let values: Vec<SourceObjectiveValue> = state.field("values").list(|entry| -> Result<_, ModsError> {
                Ok(SourceObjectiveValue {
                    value: entry.field("value").finite()?,
                    stage: entry.field("stage").string()?,
                    complete: entry.field("complete").boolean()?,
                })
            })?;
            let distinct_values: HashSet<u64> = values.iter().map(|value| value.value.to_bits()).collect();
            let distinct_stages: HashSet<&String> = values.iter().map(|value| &value.stage).collect();
            if values.is_empty()
                || distinct_values.len() != values.len()
                || distinct_stages.len() != values.len()
                || values.iter().any(|value| value.stage.is_empty())
            {
                return Err(state
                    .fail("objective stages require distinct source values and shared names")
                    .into());
            }
            let role = if value.field("role").choice_str(&["owned", "borrowed"])? == "owned" {
                SourceObjectiveRole::Owned {
                    campaign_gate: value.field("campaignGate").boolean()?,
                    bot_goal: value.field("botGoal").boolean()?,
                    change: value.field("change").nullable(&mut call)?,
                }
            } else {
                SourceObjectiveRole::Borrowed {
                    writable: value.field("writable").boolean()?,
                }
            };
            Ok(SourceObjectiveDeclaration {
                id: namespaced(value.field("id"))?,
                storage: scalar(state.field("storage"))?,
                values,
                carrier: value.field("carrier").nullable(&mut reference)?,
                target: value.field("target").nullable(&mut reference)?,
                role,
            })
        })?;
    let distinct: HashSet<&ObjectiveId> = declarations.iter().map(|value| &value.id).collect();
    if distinct.len() != declarations.len() {
        return Err(reader.fail("duplicate objective channel").into());
    }
    Ok(declarations)
}

// `pickups.ts`.

/// Read a pickup write binding.
fn pickup_write(value: SaveReader, legacy: bool) -> Result<PickupWrite, ModsError> {
    if value.field("kind").choice_str(&["protection", "inventory"])? == "protection" {
        Ok(PickupWrite::Protection {
            channel: match value.field("channel").choice_str(&["regular", "powered"])?.as_str() {
                "regular" => ProtectionChannel::Regular,
                _ => ProtectionChannel::Powered,
            },
        })
    } else {
        Ok(PickupWrite::Inventory {
            item: namespaced(value.field("item"))?,
            fields: if legacy {
                PickupWriteFields::Count
            } else {
                match value
                    .field("fields")
                    .choice_str(&["count", "capacity", "count-and-capacity"])?
                    .as_str()
                {
                    "count" => PickupWriteFields::Count,
                    "capacity" => PickupWriteFields::Capacity,
                    _ => PickupWriteFields::CountAndCapacity,
                }
            },
        })
    }
}

/// Read a mod pickup rule.
pub fn read_mod_pickup_rule<Call>(
    reader: SaveReader,
    mut read_call: impl FnMut(SaveReader) -> Result<Call, ModsError>,
) -> Result<ModPickupRule<Call>, ModsError> {
    let id = reader.field("id").string()?;
    let offered: Vec<ItemId> = reader.field("offered").list(namespaced)?;
    let distinct_offered: HashSet<&ItemId> = offered.iter().collect();
    if id.is_empty() || offered.is_empty() || distinct_offered.len() != offered.len() {
        return Err(reader
            .fail("Pickup rule requires an id and distinct offered items")
            .into());
    }
    let resource = reader.field("resource");
    let operation = reader.field("operation");
    if !resource.is_missing() && !reader.field("writes").is_missing() {
        return Err(reader.fail("Pickup rule has both resource and writes").into());
    }
    let writes: Vec<PickupWrite> = if reader.field("writes").is_missing() {
        vec![pickup_write(resource, true)?]
    } else {
        reader.field("writes").list(|value| pickup_write(value, false))?
    };
    if writes.is_empty() {
        return Err(reader.fail("Pickup rule requires a nonempty write set").into());
    }
    let keys: Vec<String> = writes
        .iter()
        .map(|write| match write {
            PickupWrite::Protection { channel } => format!(
                "protection:{}",
                match channel {
                    ProtectionChannel::Regular => "regular",
                    ProtectionChannel::Powered => "powered",
                }
            ),
            PickupWrite::Inventory { item, .. } => format!("inventory:{item}"),
        })
        .collect();
    let distinct_keys: HashSet<&String> = keys.iter().collect();
    if distinct_keys.len() != keys.len() {
        return Err(reader.fail("Pickup rule has duplicate resource writes").into());
    }
    let grant = read_call(operation.field("grant"))?;
    let call = if operation
        .field("kind")
        .choice_str(&["boolean-grant", "gate-then-grant"])?
        == "boolean-grant"
    {
        OriginalPickupOperation::BooleanGrant { grant }
    } else {
        OriginalPickupOperation::GateThenGrant {
            gate: read_call(operation.field("gate"))?,
            grant,
            grant_accepts: match operation
                .field("grantAccepts")
                .choice_str(&["nonzero", "always"])?
                .as_str()
            {
                "nonzero" => GrantAcceptance::Nonzero,
                _ => GrantAcceptance::Always,
            },
        }
    };
    Ok(ModPickupRule {
        id,
        writes,
        offered,
        operation: call,
    })
}

// `qvm-items.ts`.

/// Read a QVM item field.
fn qvm_field(reader: SaveReader) -> Result<QvmItemField, ModsError> {
    Ok(QvmItemField {
        record: reader.field("record").string()?,
        offset: u32_field(&reader, "offset", 0)?,
    })
}

/// Read a QVM item test.
fn qvm_test(reader: SaveReader) -> Result<QvmItemTest, ModsError> {
    Ok(QvmItemTest {
        field: qvm_field(reader.field("field"))?,
        mask: reader
            .field("mask")
            .nullable(|value| value.integer(0))?
            .map(as_int_number),
        comparison: match reader.field("comparison").choice_str(&["equals", "at-most"])?.as_str() {
            "equals" => ItemTestComparison::Equals,
            _ => ItemTestComparison::AtMost,
        },
        value: as_int_number(reader.field("value").integer(i64::MIN)?),
    })
}

/// Read a QVM input pointer.
fn qvm_pointer(reader: SaveReader) -> Result<QvmModInputPointer, ModsError> {
    let base = match reader.field("kind").choice_str(&["argument", "global"])?.as_str() {
        "argument" => QvmModInputPointerBase::Argument {
            index: u32_field(&reader, "index", 0)?,
        },
        _ => QvmModInputPointerBase::Global {
            address: u32_field(&reader, "address", 0)?,
        },
    };
    Ok(QvmModInputPointer {
        base,
        indirections: reader.field("indirections").list(|value| -> Result<_, ModsError> {
            let raw = value.integer(0)?;
            as_u32(&value, raw)
        })?,
        offset: u32_field(&reader, "offset", 0)?,
    })
}

/// Read a QVM weapon actor.
fn qvm_actor(reader: SaveReader) -> Result<QvmWeaponActor, ModsError> {
    Ok(QvmWeaponActor {
        record: reader.field("record").string()?,
        pointer: qvm_pointer(reader.field("pointer"))?,
    })
}

/// Read a QVM item capacity.
fn qvm_capacity(reader: SaveReader) -> Result<QvmItemCapacity, ModsError> {
    match reader
        .field("kind")
        .choice_str(&["constant", "field", "source"])?
        .as_str()
    {
        "constant" => Ok(QvmItemCapacity::Constant {
            value: int_number(&reader, "value", 0)?,
        }),
        "field" => Ok(QvmItemCapacity::Field {
            field: qvm_field(reader.field("field"))?,
        }),
        _ => Ok(QvmItemCapacity::Source {
            instruction: u32_field(&reader, "instruction", 0)?,
            overrides: reader.field("overrides").list(|value| -> Result<_, ModsError> {
                Ok(QvmItemCapacityOverride {
                    address: u32_field(&value, "address", 0)?,
                    comparison: match value
                        .field("comparison")
                        .choice_str(&["equals", "not-equals"])?
                        .as_str()
                    {
                        "equals" => ItemCapacityComparison::Equals,
                        _ => ItemCapacityComparison::NotEquals,
                    },
                    value: as_int_number(value.field("value").integer(i64::MIN)?),
                    instruction: u32_field(&value, "instruction", 0)?,
                })
            })?,
        }),
    }
}

/// Read QVM item storage.
pub fn read_qvm_item_storage(reader: SaveReader) -> Result<QvmItemStorage, ModsError> {
    let field = qvm_field(reader.field("field"))?;
    if reader.field("kind").choice_str(&["counter", "bits"])? == "counter" {
        Ok(QvmItemStorage::Counter {
            field,
            item: namespaced(reader.field("item"))?,
            capacity: qvm_capacity(reader.field("capacity"))?,
        })
    } else {
        Ok(QvmItemStorage::Bits {
            field,
            private_mask: int_number(&reader, "privateMask", 0)?,
            items: reader.field("items").list(|entry| -> Result<_, ModsError> {
                Ok(QvmMaskedItem {
                    item: namespaced(entry.field("item"))?,
                    mask: int_number(&entry, "mask", 1)?,
                })
            })?,
        })
    }
}

/// Read a QVM weapon predicate.
fn qvm_predicate(reader: SaveReader) -> Result<QvmWeaponPredicate, ModsError> {
    Ok(QvmWeaponPredicate {
        instruction: u32_field(&reader, "instruction", 0)?,
        unselected: reader.field("unselected").boolean()?,
    })
}

/// Read QVM mod items.
pub fn read_qvm_mod_items(
    reader: SaveReader,
    mut read_call: impl FnMut(SaveReader) -> Result<crate::contract::QvmModSourceCall, ModsError>,
) -> Result<crate::contract::QvmModItems, ModsError> {
    let definitions = reader.field("definitions").list(|value| -> Result<_, ModsError> {
        let icon = if value.field("icon").is_missing() {
            None
        } else {
            value
                .field("icon")
                .nullable(|entry| read_item_icon_declaration(&entry))?
        };
        let actions = if value.field("actions").is_missing() {
            None
        } else {
            Some(read_item_actions(value.field("actions"), &mut read_call)?)
        };
        let admission = match value
            .field("admission")
            .choice_str(&["add", "replace-primary"])?
            .as_str()
        {
            "add" => SourceItemAdmissionMode::Add,
            _ => SourceItemAdmissionMode::ReplacePrimary,
        };
        let item = namespaced(value.field("item"))?;
        let label = value.field("label").string()?;
        if value.field("kind").choice_str(&["counter", "weapon"])? == "counter" {
            Ok(QvmItemDefinition {
                item,
                label,
                icon,
                admission,
                actions,
                kind: QvmItemKind::Counter,
            })
        } else {
            let held = if value.field("held").is_missing() {
                None
            } else {
                Some(read_held_weapon_declaration(&value.field("held"))?)
            };
            Ok(QvmItemDefinition {
                item,
                label,
                icon,
                admission,
                actions,
                kind: QvmItemKind::Weapon(SourceWeaponItem {
                    ammo: value.field("ammo").nullable(namespaced)?,
                    held,
                }),
            })
        }
    })?;
    let storage = reader.field("storage").list(read_qvm_item_storage)?;
    let weapons = reader.field("weapons");
    let staged = if weapons.is_missing() {
        None
    } else {
        let stage = weapons.field("stage");
        let input = weapons.field("input");
        let dispatcher = stage.field("dispatcher");
        let selection = stage.field("selection");
        let request = stage.field("request");
        let continuation = stage.field("continuation");
        let projection = continuation.field("projection");
        Some(QvmItemsWeaponStage {
            input: QvmItemsWeaponInput {
                entry: u32_field(&input, "entry", 0)?,
                clock: qvm_field(input.field("clock"))?,
            },
            stage: QvmWeaponStage {
                dispatcher: QvmWeaponDispatcher {
                    entry: u32_field(&dispatcher, "entry", 0)?,
                    actor: qvm_actor(dispatcher.field("actor"))?,
                },
                predicates: stage.field("predicates").list(qvm_predicate)?,
                settled: stage.field("settled").list(qvm_test)?,
                selection: QvmWeaponSelection {
                    field: qvm_field(selection.field("field"))?,
                    values: selection.field("values").list(|value| -> Result<_, ModsError> {
                        Ok(QvmWeaponValue {
                            value: as_int_number(value.field("value").integer(i64::MIN)?),
                            item: namespaced(value.field("item"))?,
                        })
                    })?,
                },
                request: QvmWeaponRequest {
                    entry: u32_field(&request, "entry", 0)?,
                    argument: u32_field(&request, "argument", 0)?,
                    accepted: request.field("accepted").list(qvm_test)?,
                },
                continuation: QvmWeaponContinuation {
                    entry: u32_field(&continuation, "entry", 0)?,
                    actor: qvm_actor(continuation.field("actor"))?,
                    instruction: u32_field(&continuation, "instruction", 0)?,
                    original_taken: continuation.field("originalTaken").boolean()?,
                    when: continuation.field("when").list(qvm_test)?,
                    predicates: continuation.field("predicates").list(qvm_predicate)?,
                    projection: QvmWeaponProjection {
                        movement: qvm_pointer(projection.field("movement"))?,
                        byte_length: u64_field(&projection, "byteLength", 1)?,
                        minimum: u32_field(&projection, "minimum", 0)?,
                        maximum: u32_field(&projection, "maximum", 0)?,
                        view_height: qvm_field(projection.field("viewHeight"))?,
                        ground: qvm_field(projection.field("ground"))?,
                    },
                    calls: continuation.field("calls").list(|value| -> Result<_, ModsError> {
                        Ok(QvmWeaponCall {
                            instruction: u32_field(&value, "instruction", 0)?,
                            call: read_call(value.field("call"))?,
                        })
                    })?,
                },
            },
        })
    };
    Ok(crate::contract::QvmModItems {
        definitions,
        storage,
        weapons: staged,
    })
}

// `selection.ts`.

/// A successful edit publishes one complete, dependency-ordered selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ModSelectionSet {
    descriptions: HashMap<String, ModDescription>,
    selected: Vec<ModSelection>,
}

impl ModSelectionSet {
    /// Build a selection set, resolving the initial selections.
    pub fn new(descriptions: Vec<ModDescription>, initial: Vec<ModSelection>) -> Result<Self, ModsError> {
        let mut set = Self {
            descriptions: HashMap::new(),
            selected: Vec::new(),
        };
        set.refresh(descriptions)?;
        set.restore(initial)?;
        Ok(set)
    }

    /// Known descriptions.
    #[must_use]
    pub fn entries(&self) -> Vec<ModDescription> {
        self.descriptions.values().cloned().collect()
    }

    /// Enabled selections.
    #[must_use]
    pub fn enabled(&self) -> &[ModSelection] {
        &self.selected
    }

    /// Whether a selection is enabled.
    pub fn has(&self, selection: &ModSelection) -> Result<bool, ModsError> {
        let key = crate::contract::mod_selection_key(selection)?;
        Ok(self.selected.iter().any(|value| {
            crate::contract::mod_selection_key(value)
                .map(|other| other == key)
                .unwrap_or(false)
        }))
    }

    /// Refresh known descriptions.
    pub fn refresh(&mut self, descriptions: Vec<ModDescription>) -> Result<(), ModsError> {
        let mut next = HashMap::new();
        for description in descriptions {
            let key = crate::contract::mod_selection_key(&description.selection)?;
            if next.contains_key(&key) {
                return Err(ModsError::Invalid(format!("Duplicate mod declaration: {key}")));
            }
            next.insert(key, description);
        }
        // Missing enabled packages stay visible so users can disable them.
        for selection in &self.selected {
            let key = crate::contract::mod_selection_key(selection)?;
            if let std::collections::hash_map::Entry::Vacant(slot) = next.entry(key) {
                if let Some(previous) = self.descriptions.get(slot.key()) {
                    let mut retained = previous.clone();
                    retained.availability = ModAvailability::Unavailable {
                        reason: "This enabled mod is no longer installed".to_string(),
                    };
                    slot.insert(retained);
                }
            }
        }
        self.descriptions = next;
        Ok(())
    }

    /// Restore selections.
    pub fn restore(&mut self, selections: Vec<ModSelection>) -> Result<(), ModsError> {
        self.selected = self.resolve(&selections)?;
        Ok(())
    }

    /// Enable or disable a selection.
    pub fn set_enabled(&mut self, selection: &ModSelection, enabled: bool) -> Result<(), ModsError> {
        if enabled {
            let mut selections = self.selected.clone();
            selections.push(selection.clone());
            self.selected = self.resolve(&selections)?;
            return Ok(());
        }
        let mut removed = HashSet::new();
        removed.insert(crate::contract::mod_selection_key(selection)?);
        let mut changed = true;
        while changed {
            changed = false;
            for active in &self.selected {
                let key = crate::contract::mod_selection_key(active)?;
                if removed.contains(&key) {
                    continue;
                }
                if let Some(description) = self.descriptions.get(&key) {
                    let mut depends = false;
                    for dependency in &description.requires {
                        if removed.contains(&crate::contract::mod_selection_key(dependency)?) {
                            depends = true;
                            break;
                        }
                    }
                    if depends {
                        removed.insert(key);
                        changed = true;
                    }
                }
            }
        }
        let kept: Vec<ModSelection> = self
            .selected
            .iter()
            .filter(|active| {
                crate::contract::mod_selection_key(active)
                    .map(|key| !removed.contains(&key))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        self.selected = kept;
        Ok(())
    }

    /// Validate the current selection.
    pub fn validate(&self) -> Result<(), ModsError> {
        self.resolve(&self.selected)?;
        Ok(())
    }

    /// Resolve selections into dependency order.
    fn resolve(&self, selections: &[ModSelection]) -> Result<Vec<ModSelection>, ModsError> {
        let mut active: HashMap<String, &ModDescription> = HashMap::new();
        let mut visiting: Vec<String> = Vec::new();
        fn visit<'a>(
            set: &'a ModSelectionSet,
            selection: &ModSelection,
            active: &mut HashMap<String, &'a ModDescription>,
            visiting: &mut Vec<String>,
        ) -> Result<(), ModsError> {
            let key = crate::contract::mod_selection_key(selection)?;
            if active.contains_key(&key) {
                return Ok(());
            }
            if visiting.contains(&key) {
                visiting.push(key);
                return Err(ModsError::Invalid(format!(
                    "Mod dependency cycle: {}",
                    visiting.join(" -> ")
                )));
            }
            let Some(description) = set.descriptions.get(&key) else {
                return Err(ModsError::Invalid(format!("Required mod is not installed: {key}")));
            };
            if description.purpose != ModPurpose::Addition {
                return Err(ModsError::Invalid(format!(
                    "{} defines a game type; select it as the game's rules",
                    description.title
                )));
            }
            if let ModAvailability::Unavailable { reason } = &description.availability {
                return Err(ModsError::Invalid(format!("{}: {reason}", description.title)));
            }
            visiting.push(key.clone());
            for dependency in &description.requires {
                visit(set, dependency, active, visiting)?;
            }
            visiting.pop();
            active.insert(key, description);
            Ok(())
        }
        for selection in selections {
            visit(self, selection, &mut active, &mut visiting)?;
        }
        for description in active.values() {
            for conflict in &description.conflicts {
                let other = active.get(&crate::contract::mod_selection_key(conflict)?);
                if let Some(other) = other {
                    return Err(ModsError::Invalid(format!(
                        "{} conflicts with {}",
                        description.title, other.title
                    )));
                }
            }
        }
        // HashMap iteration order is nondeterministic; restore visit order.
        let mut ordered: Vec<(usize, ModSelection)> = Vec::new();
        let mut order = HashMap::new();
        fn order_visit(
            set: &ModSelectionSet,
            selection: &ModSelection,
            order: &mut HashMap<String, usize>,
        ) -> Result<(), ModsError> {
            let key = crate::contract::mod_selection_key(selection)?;
            if order.contains_key(&key) {
                return Ok(());
            }
            if let Some(description) = set.descriptions.get(&key) {
                for dependency in &description.requires {
                    order_visit(set, dependency, order)?;
                }
            }
            let rank = order.len();
            order.insert(key, rank);
            Ok(())
        }
        for selection in selections {
            order_visit(self, selection, &mut order)?;
        }
        for (key, description) in &active {
            if let Some(rank) = order.get(key) {
                ordered.push((*rank, description.selection.clone()));
            }
        }
        ordered.sort_by_key(|(rank, _)| *rank);
        Ok(ordered.into_iter().map(|(_, selection)| selection).collect())
    }
}

// `native-callbacks.ts`.

const SCALAR_ENCODINGS: &[&str] = &[
    "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64",
];

/// Read a native scalar encoding.
fn scalar_encoding(reader: SaveReader) -> Result<NativeModScalar, ModsError> {
    Ok(match reader.choice_str(SCALAR_ENCODINGS)?.as_str() {
        "int8" => NativeModScalar::Int8,
        "uint8" => NativeModScalar::Uint8,
        "int16" => NativeModScalar::Int16,
        "uint16" => NativeModScalar::Uint16,
        "int32" => NativeModScalar::Int32,
        "uint32" => NativeModScalar::Uint32,
        "int64" => NativeModScalar::Int64,
        "uint64" => NativeModScalar::Uint64,
        "float32" => NativeModScalar::Float32,
        _ => NativeModScalar::Float64,
    })
}

/// Read a mod actor input.
fn actor_input(reader: SaveReader) -> Result<ModActorInput, ModsError> {
    Ok(
        match reader
            .choice_str(&["self", "other", "activator", "attacker", "inflictor"])?
            .as_str()
        {
            "self" => ModActorInput::Slf,
            "other" => ModActorInput::Other,
            "activator" => ModActorInput::Activator,
            "attacker" => ModActorInput::Attacker,
            _ => ModActorInput::Inflictor,
        },
    )
}

/// Read a scalar client input.
fn scalar_input(reader: SaveReader) -> Result<ModClientScalarInput, ModsError> {
    Ok(
        match reader
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
}

/// Read a native address.
fn native_address(reader: SaveReader) -> Result<NativeModAddress, ModsError> {
    Ok(NativeModAddress {
        rva: u64_field(&reader, "rva", 0)?,
        indirections: reader.field("indirections").list(|value| -> Result<_, ModsError> {
            let raw = value.integer(0)?;
            as_u64(&value, raw)
        })?,
    })
}

/// Read a native entry point.
fn native_entry(reader: SaveReader) -> Result<NativeModEntry, ModsError> {
    if reader.field("kind").choice_str(&["export", "rva"])? == "export" {
        Ok(NativeModEntry::Export {
            name: reader.field("name").string()?,
        })
    } else {
        Ok(NativeModEntry::Rva {
            rva: u64_field(&reader, "rva", 0)?,
        })
    }
}

/// Read a native value.
fn native_argument(reader: SaveReader) -> Result<NativeModValue, ModsError> {
    match reader
        .field("kind")
        .choice_str(&[
            "int8",
            "uint8",
            "int16",
            "uint16",
            "int32",
            "uint32",
            "int64",
            "uint64",
            "float32",
            "float64",
            "vector",
            "string",
            "actor",
            "client",
            "userinfo",
            "time",
            "address",
            "user-command",
        ])?
        .as_str()
    {
        "user-command" => Ok(NativeModValue::UserCommand),
        "client" => Ok(NativeModValue::Client {
            input: actor_input(reader.field("input"))?,
        }),
        "userinfo" => Ok(NativeModValue::Userinfo {
            input: actor_input(reader.field("input"))?,
        }),
        "actor" => Ok(NativeModValue::Actor {
            record: reader.field("record").string()?,
            input: actor_input(reader.field("input"))?,
        }),
        "time" => Ok(NativeModValue::Time {
            input: match reader.field("input").choice_str(&["time", "elapsed"])?.as_str() {
                "time" => ModTimeInput::Time,
                _ => ModTimeInput::Elapsed,
            },
            units: match reader.field("units").choice_str(&["seconds", "milliseconds"])?.as_str() {
                "seconds" => ModTimeUnits::Seconds,
                _ => ModTimeUnits::Milliseconds,
            },
            encoding: scalar_encoding(reader.field("encoding"))?,
        }),
        "address" => Ok(NativeModValue::Address(reader.field("value").nullable(native_address)?)),
        kind => Ok(NativeModValue::Value {
            kind: match kind {
                "vector" => NativeModValueKind::Vector,
                "string" => NativeModValueKind::Str,
                _ => NativeModValueKind::Scalar(scalar_encoding_from_name(kind)),
            },
            value: mod_callback_value(reader.field("value"))?,
        }),
    }
}

/// Map a validated scalar encoding name.
fn scalar_encoding_from_name(name: &str) -> NativeModScalar {
    match name {
        "int8" => NativeModScalar::Int8,
        "uint8" => NativeModScalar::Uint8,
        "int16" => NativeModScalar::Int16,
        "uint16" => NativeModScalar::Uint16,
        "int32" => NativeModScalar::Int32,
        "uint32" => NativeModScalar::Uint32,
        "int64" => NativeModScalar::Int64,
        "uint64" => NativeModScalar::Uint64,
        "float32" => NativeModScalar::Float32,
        _ => NativeModScalar::Float64,
    }
}

/// Read a native source call.
fn native_source_call(reader: SaveReader) -> Result<NativeModSourceCall, ModsError> {
    let entry = reader.field("entry");
    let call_entry = if entry.field("kind").string()? == "game-export" {
        NativeModEntry::GameExport {
            name: entry.field("name").string()?,
        }
    } else {
        native_entry(entry)?
    };
    Ok(NativeModSourceCall {
        entry: call_entry,
        arguments: reader.field("arguments").list(native_argument)?,
        globals: reader.field("globals").list(|global| -> Result<_, ModsError> {
            Ok(NativeModGlobal {
                address: native_address(global.field("address"))?,
                value: native_argument(global.field("value"))?,
            })
        })?,
        returns: match reader
            .field("returns")
            .choice_str(&[
                "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64", "void",
            ])?
            .as_str()
        {
            "void" => NativeModReturn::Void,
            name => NativeModReturn::Scalar(scalar_encoding_from_name(name)),
        },
        skips: if reader.field("skips").is_missing() {
            Vec::new()
        } else {
            reader.field("skips").list(|region| -> Result<_, ModsError> {
                Ok(NativeModSkip {
                    entry: u64_field(&region, "entry", 0)?,
                    join: u64_field(&region, "join", 0)?,
                })
            })?
        },
    })
}

/// Read a native input/output binding.
fn native_input_output(reader: SaveReader) -> Result<NativeModInputOutput, ModsError> {
    if reader.field("kind").choice_str(&["field", "handler"])? == "field" {
        Ok(NativeModInputOutput::Field {
            record: reader.field("record").string()?,
            offset: u64_field(&reader, "offset", 0)?,
        })
    } else {
        Ok(NativeModInputOutput::Handler {
            entry: native_entry(reader.field("entry"))?,
            arguments: reader.field("arguments").list(native_argument)?,
            inputs: reader.field("inputs").list(scalar_input)?,
        })
    }
}

/// Read a callback operation.
fn callback_operation(text: &str) -> ModCallbackOperation {
    match text {
        "damage" => ModCallbackOperation::Damage,
        "inventory.give" => ModCallbackOperation::InventoryGive,
        "inventory.consume" => ModCallbackOperation::InventoryConsume,
        "actor.think" => ModCallbackOperation::Actor(ModActorOperation::Think),
        "actor.touch" => ModCallbackOperation::Actor(ModActorOperation::Touch),
        "actor.use" => ModCallbackOperation::Actor(ModActorOperation::Use),
        "actor.pain" => ModCallbackOperation::Actor(ModActorOperation::Pain),
        _ => ModCallbackOperation::Actor(ModActorOperation::Die),
    }
}

/// Read a callback binding.
fn native_binding(reader: SaveReader) -> Result<ModCallbackBinding, ModsError> {
    let id = CallbackId(namespaced(reader.field("id"))?);
    let operation = reader.field("operation").choice_str(&[
        "damage",
        "inventory.give",
        "inventory.consume",
        "actor.think",
        "actor.touch",
        "actor.use",
        "actor.pain",
        "actor.die",
    ])?;
    match reader
        .field("stage")
        .choice_str(&["observe", "transform", "replace"])?
        .as_str()
    {
        "observe" => Ok(ModCallbackBinding {
            id,
            binding: ModCallbackBindingKind::Observe {
                operation: callback_operation(&operation),
            },
        }),
        "transform" => {
            if operation == "damage" {
                return Ok(ModCallbackBinding {
                    id,
                    binding: ModCallbackBindingKind::DamageTransform {
                        result: match reader.field("result").choice_str(&["amount", "knockback"])?.as_str() {
                            "amount" => crate::contract::DamageTransformResult::Amount,
                            _ => crate::contract::DamageTransformResult::Knockback,
                        },
                    },
                });
            }
            if operation == "inventory.give" || operation == "inventory.consume" {
                reader.field("result").literal_str("amount")?;
                return Ok(ModCallbackBinding {
                    id,
                    binding: ModCallbackBindingKind::InventoryTransform {
                        operation: if operation == "inventory.give" {
                            crate::contract::InventoryTransformOperation::Give
                        } else {
                            crate::contract::InventoryTransformOperation::Consume
                        },
                    },
                });
            }
            Err(reader.fail("actor callbacks support observation or replacement").into())
        }
        _ => {
            if operation == "damage" || operation == "inventory.give" || operation == "inventory.consume" {
                return Err(reader.fail("only actor callbacks support replacement").into());
            }
            reader.field("result").literal_str("boolean")?;
            let actor = match callback_operation(&operation) {
                ModCallbackOperation::Actor(actor) => actor,
                _ => ModActorOperation::Think,
            };
            Ok(ModCallbackBinding {
                id,
                binding: ModCallbackBindingKind::ActorReplace { operation: actor },
            })
        }
    }
}

/// Read a native actor field.
fn native_field(reader: SaveReader) -> Result<NativeModActorField, ModsError> {
    let offset = u64_field(&reader, "offset", 0)?;
    match reader
        .field("binding")
        .choice_str(&[
            "health",
            "inventory",
            "inventory-capacity",
            "origin",
            "velocity",
            "angles",
            "bounds-min",
            "bounds-max",
            "record",
            "constant",
            "constant-vector",
            "private",
            "address",
            "team",
            "score",
        ])?
        .as_str()
    {
        "address" => Ok(NativeModActorField::Address {
            offset,
            value: reader.field("value").nullable(native_address)?,
        }),
        "team" => Ok(NativeModActorField::Match {
            offset,
            encoding: scalar_encoding(reader.field("encoding"))?,
            field: SourceMatchField::Team {
                values: read_source_team_values(reader.field("values"))?,
            },
        }),
        "score" => Ok(NativeModActorField::Match {
            offset,
            encoding: scalar_encoding(reader.field("encoding"))?,
            field: SourceMatchField::Score,
        }),
        "health" => Ok(NativeModActorField::Health {
            offset,
            encoding: scalar_encoding(reader.field("encoding"))?,
        }),
        "inventory" => Ok(NativeModActorField::Inventory {
            offset,
            encoding: scalar_encoding(reader.field("encoding"))?,
            item: namespaced(reader.field("item"))?,
        }),
        "inventory-capacity" => Ok(NativeModActorField::InventoryCapacity {
            offset,
            encoding: scalar_encoding(reader.field("encoding"))?,
            item: namespaced(reader.field("item"))?,
        }),
        "record" => Ok(shared_field(
            offset,
            NativeModSharedActorBinding::Record {
                record: reader.field("record").string()?,
            },
        )),
        "constant" => Ok(NativeModActorField::Constant {
            offset,
            encoding: scalar_encoding(reader.field("encoding"))?,
            value: reader.field("value").number()?,
        }),
        "constant-vector" => Ok(shared_field(
            offset,
            NativeModSharedActorBinding::ConstantVector(read_vector(reader.field("value"))?),
        )),
        "private" => Ok(shared_field(
            offset,
            NativeModSharedActorBinding::Private {
                byte_length: u64_field(&reader, "byteLength", 1)?,
            },
        )),
        binding => Ok(shared_field(
            offset,
            match binding {
                "origin" => NativeModSharedActorBinding::Origin,
                "velocity" => NativeModSharedActorBinding::Velocity,
                "angles" => NativeModSharedActorBinding::Angles,
                "bounds-min" => NativeModSharedActorBinding::BoundsMin,
                _ => NativeModSharedActorBinding::BoundsMax,
            },
        )),
    }
}

/// Wrap a shared QVM-style binding.
fn shared_field(offset: u64, binding: NativeModSharedActorBinding) -> NativeModActorField {
    NativeModActorField::Shared(NativeModSharedActorField {
        offset,
        access: None,
        binding,
    })
}

/// Read a native armor field.
fn armor_field(reader: SaveReader) -> Result<NativeModArmorField, ModsError> {
    Ok(NativeModArmorField {
        record: reader.field("record").string()?,
        offset: u64_field(&reader, "offset", 0)?,
        encoding: scalar_encoding(reader.field("encoding"))?,
    })
}

/// Read a native armor selection.
fn armor_selection(reader: SaveReader) -> Result<NativeModArmorSelection, ModsError> {
    let field = armor_field(reader.field("field"))?;
    if reader.field("kind").choice_str(&["positive", "enum"])? == "positive" {
        Ok(NativeModArmorSelection::Positive { field })
    } else {
        Ok(NativeModArmorSelection::Enum {
            field,
            value: reader.field("value").number()?,
            none: reader.field("none").number()?,
        })
    }
}

/// Read native armor.
fn native_armor(reader: SaveReader) -> Result<NativeModArmor, ModsError> {
    match reader.field("kind").choice_str(&["none", "q2", "source"])?.as_str() {
        "none" => Ok(NativeModArmor::None),
        "source" => Ok(NativeModArmor::Source {
            regular: reader.field("regular").list(regular_armor_item)?,
            power: reader.field("power").list(power_armor_item)?,
        }),
        _ => Ok(NativeModArmor::Q2 {
            regular: reader.field("regular").list(|value| -> Result<_, ModsError> {
                Ok(crate::contract::NativeModQ2RegularArmor {
                    item: namespaced(value.field("item"))?,
                    selection: armor_selection(value.field("selection"))?,
                    points: armor_field(value.field("points"))?,
                    normal_protection: value.field("normalProtection").number()?,
                    energy_protection: value.field("energyProtection").number()?,
                })
            })?,
            power: reader.field("power").list(power_armor_item)?,
        }),
    }
}

/// Read a regular armor item.
fn regular_armor_item(reader: SaveReader) -> Result<NativeModRegularArmorItem, ModsError> {
    Ok(NativeModRegularArmorItem {
        item: reader.field("item").nullable(namespaced)?,
        selection: armor_selection(reader.field("selection"))?,
        points: armor_field(reader.field("points"))?,
    })
}

/// Read a power armor item.
fn power_armor_item(reader: SaveReader) -> Result<NativeModPowerArmorItem, ModsError> {
    Ok(NativeModPowerArmorItem {
        item: namespaced(reader.field("item"))?,
        kind: match reader.field("kind").choice_str(&["screen", "shield"])?.as_str() {
            "screen" => NativePowerArmorKind::Screen,
            _ => NativePowerArmorKind::Shield,
        },
        selection: armor_selection(reader.field("selection"))?,
        cells: armor_field(reader.field("cells"))?,
        enabled: reader.field("enabled").nullable(|value| -> Result<_, ModsError> {
            Ok(NativeModMaskedField {
                field: armor_field(value.field("field"))?,
                mask: int_number(&value, "mask", 1)?,
            })
        })?,
    })
}

/// Read a region location.
fn region_location(reader: SaveReader) -> Result<NativeModRegionLocation, ModsError> {
    match reader
        .field("kind")
        .choice_str(&["register", "stack", "simd"])?
        .as_str()
    {
        "simd" => Ok(NativeModRegionLocation::Simd {
            index: u32_field(&reader, "index", 0)?,
            offset: u64_field(&reader, "offset", 0)?,
            storage: scalar_encoding(reader.field("storage"))?,
        }),
        kind => {
            let storage = if matches!(reader.field("storage").value, Some(SaveJson::String(text)) if text == "pointer")
            {
                NativeModRegionStorage::Pointer
            } else {
                NativeModRegionStorage::Scalar(scalar_encoding(reader.field("storage"))?)
            };
            if kind == "stack" {
                Ok(NativeModRegionLocation::Stack {
                    offset: u64_field(&reader, "offset", 0)?,
                    storage,
                })
            } else {
                Ok(NativeModRegionLocation::Register {
                    register: match reader
                        .field("register")
                        .choice_str(&[
                            "rax", "rcx", "rdx", "rbx", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
                            "r14", "r15",
                        ])?
                        .as_str()
                    {
                        "rax" => NativeModRegionRegister::Rax,
                        "rcx" => NativeModRegionRegister::Rcx,
                        "rdx" => NativeModRegionRegister::Rdx,
                        "rbx" => NativeModRegionRegister::Rbx,
                        "rbp" => NativeModRegionRegister::Rbp,
                        "rsi" => NativeModRegionRegister::Rsi,
                        "rdi" => NativeModRegionRegister::Rdi,
                        "r8" => NativeModRegionRegister::R8,
                        "r9" => NativeModRegionRegister::R9,
                        "r10" => NativeModRegionRegister::R10,
                        "r11" => NativeModRegionRegister::R11,
                        "r12" => NativeModRegionRegister::R12,
                        "r13" => NativeModRegionRegister::R13,
                        "r14" => NativeModRegionRegister::R14,
                        _ => NativeModRegionRegister::R15,
                    },
                    storage,
                })
            }
        }
    }
}

/// Read a protection admission.
fn protection_admission(reader: SaveReader) -> Result<ModProtectionAdmission, ModsError> {
    if reader
        .field("kind")
        .choice_str(&["claim", "replace-current-primary", "replace-primary"])?
        == "replace-primary"
    {
        Ok(ModProtectionAdmission::ReplacePrimary {
            owner: provider_id(&namespaced(reader.field("owner"))?),
        })
    } else {
        Ok(
            match reader
                .field("kind")
                .choice_str(&["claim", "replace-current-primary"])?
                .as_str()
            {
                "claim" => ModProtectionAdmission::Claim,
                _ => ModProtectionAdmission::ReplaceCurrentPrimary,
            },
        )
    }
}

/// Read a Q2 flag ABI.
fn q2_abi(reader: SaveReader) -> Result<Q2CallbackAbi, ModsError> {
    Ok(match reader.choice_str(&["q2-classic", "q2-rerelease"])?.as_str() {
        "q2-classic" => Q2CallbackAbi::Classic,
        _ => Q2CallbackAbi::Rerelease,
    })
}

/// Read a protection definition.
fn native_protection(reader: SaveReader, legacy: bool) -> Result<NativeModProtectionDefinition, ModsError> {
    let powered = legacy || reader.field("channel").choice_str(&["regular", "powered"])? == "powered";
    let absorb = reader.field("absorb");
    let admission = if reader.field("admission").is_missing() {
        None
    } else {
        Some(protection_admission(reader.field("admission"))?)
    };
    let id = reader.field("id").string()?;
    let abi =
        absorb
            .field("abi")
            .choice_str(&["q2-check-power-armor", "q2-check-armor", "source-call", "source-region"])?;
    if abi == "source-call" || abi == "source-region" {
        if legacy {
            return Err(absorb
                .fail("Legacy powered protection requires its original Q2 ABI")
                .into());
        }
        let call = native_source_call(absorb.field("call"))?;
        if abi == "source-call" {
            return Ok(NativeModProtectionDefinition {
                id,
                admission,
                channel: protection_channel(powered, &reader, NativeAbsorb::SourceCall(call))?,
            });
        }
        let frame = absorb.field("frame");
        let region = NativeModProtectionRegion {
            call,
            frame: NativeModRegionFrame {
                entry: u64_field(&frame, "entry", 0)?,
                exit: u64_field(&frame, "exit", 0)?,
                stack_bytes: u64_field(&frame, "stackBytes", 0)?,
                argument_bytes: u64_field(&frame, "argumentBytes", 0)?,
            },
            entry: u64_field(&absorb, "entry", 0)?,
            join: u64_field(&absorb, "join", 0)?,
            result: region_location(absorb.field("result"))?,
            inputs: absorb.field("inputs").list(|value| -> Result<_, ModsError> {
                Ok(NativeModRegionInput {
                    target: region_location(value.field("target"))?,
                    value: native_argument(value.field("value"))?,
                })
            })?,
        };
        return Ok(NativeModProtectionDefinition {
            id,
            admission,
            channel: protection_channel(powered, &reader, NativeAbsorb::Region(region))?,
        });
    }
    let call = NativeModQ2ArmorCall {
        entry: native_entry(absorb.field("entry"))?,
        flags: q2_abi(absorb.field("flags"))?,
        globals: if absorb.field("globals").is_missing() {
            Vec::new()
        } else {
            absorb.field("globals").list(|global| -> Result<_, ModsError> {
                Ok(NativeModGlobal {
                    address: native_address(global.field("address"))?,
                    value: native_argument(global.field("value"))?,
                })
            })?
        },
    };
    if !powered {
        if abi != "q2-check-armor" {
            return Err(absorb
                .fail("Regular protection requires its original regular armor ABI")
                .into());
        }
        return Ok(NativeModProtectionDefinition {
            id,
            admission,
            channel: NativeModProtectionChannel::Regular {
                storage: reader.field("storage").list(regular_armor_item)?,
                absorb: NativeModRegularAbsorb::Q2Armor(NativeModQ2ArmorCheck {
                    entry: call.entry,
                    flags: call.flags,
                    globals: call.globals,
                    sparks: u32_field(&absorb, "sparks", 0)?,
                }),
            },
        });
    }
    if abi != "q2-check-power-armor" {
        return Err(absorb
            .fail("Powered protection requires its original power armor ABI")
            .into());
    }
    Ok(NativeModProtectionDefinition {
        id,
        admission,
        channel: NativeModProtectionChannel::Powered {
            storage: reader.field("storage").list(power_armor_item)?,
            absorb: NativeModProtectionAbsorb::Q2PowerArmor(call),
        },
    })
}

/// Pending source absorption.
enum NativeAbsorb {
    SourceCall(NativeModSourceCall),
    Region(NativeModProtectionRegion),
}

/// Build a protection channel with source absorption.
fn protection_channel(
    powered: bool,
    reader: &SaveReader,
    absorb: NativeAbsorb,
) -> Result<NativeModProtectionChannel, ModsError> {
    if powered {
        Ok(NativeModProtectionChannel::Powered {
            storage: reader.field("storage").list(power_armor_item)?,
            absorb: match absorb {
                NativeAbsorb::SourceCall(call) => NativeModProtectionAbsorb::SourceCall(call),
                NativeAbsorb::Region(region) => NativeModProtectionAbsorb::Region(region),
            },
        })
    } else {
        Ok(NativeModProtectionChannel::Regular {
            storage: reader.field("storage").list(regular_armor_item)?,
            absorb: match absorb {
                NativeAbsorb::SourceCall(call) => NativeModRegularAbsorb::SourceCall(call),
                NativeAbsorb::Region(region) => NativeModRegularAbsorb::Region(region),
            },
        })
    }
}

/// Read protection declarations (current or legacy layout).
fn native_protections(reader: SaveReader) -> Result<Vec<NativeModProtectionDefinition>, ModsError> {
    let current = reader.field("protection");
    let legacy = reader.field("poweredProtection");
    if !current.is_missing() && !legacy.is_missing() {
        return Err(current
            .fail("Protection declarations cannot mix legacy and current layouts")
            .into());
    }
    if !current.is_missing() {
        current.list(|value| native_protection(value, false))
    } else if !legacy.is_missing() {
        Ok(vec![native_protection(legacy, true)?])
    } else {
        Ok(Vec::new())
    }
}

/// Read a scalar field.
fn scalar_field(reader: SaveReader) -> Result<NativeModScalarField, ModsError> {
    Ok(NativeModScalarField {
        offset: u64_field(&reader, "offset", 0)?,
        encoding: scalar_encoding(reader.field("encoding"))?,
    })
}

/// Read source actors.
fn native_source_actors(reader: SaveReader) -> Result<NativeModSourceActors, ModsError> {
    let fields = reader.field("fields");
    let nextthink = fields.field("nextthink");
    let update = reader.field("update");
    let callbacks = if reader.field("callbacks").is_missing() {
        None
    } else {
        let callbacks = reader.field("callbacks");
        Some(NativeModSourceActorCallbacks {
            abi: q2_abi(callbacks.field("abi"))?,
            touch: nullable_u64(callbacks.field("touch"))?,
            pain: nullable_u64(callbacks.field("pain"))?,
            die: nullable_u64(callbacks.field("die"))?,
        })
    };
    let combat = if reader.field("combat").is_missing() {
        None
    } else {
        Some(native_combat(reader.field("combat"))?)
    };
    Ok(NativeModSourceActors {
        allocate: native_entry(reader.field("allocate"))?,
        release: native_entry(reader.field("release"))?,
        update: NativeModUpdate {
            entry: native_entry(update.field("entry"))?,
            returns: match update
                .field("returns")
                .choice_str(&[
                    "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "float32", "float64",
                    "void",
                ])?
                .as_str()
            {
                "void" => NativeModReturn::Void,
                name => NativeModReturn::Scalar(scalar_encoding_from_name(name)),
            },
        },
        frame_seconds: reader.field("frameSeconds").number()?,
        clock: reader.field("clock").list(|value| -> Result<_, ModsError> {
            Ok(NativeModClock {
                address: native_address(value.field("address"))?,
                input: match value.field("input").choice_str(&["time", "frame"])?.as_str() {
                    "time" => crate::contract::NativeModClockInput::Time,
                    _ => crate::contract::NativeModClockInput::Frame,
                },
                encoding: scalar_encoding(value.field("encoding"))?,
                units: match value.field("units").choice_str(&["seconds", "milliseconds"])?.as_str() {
                    "seconds" => ModTimeUnits::Seconds,
                    _ => ModTimeUnits::Milliseconds,
                },
            })
        })?,
        fields: NativeModSourceActorFields {
            velocity: u64_field(&fields, "velocity", 0)?,
            ground: u64_field(&fields, "ground", 0)?,
            r#use: nullable_u64(fields.field("use"))?,
            think: u64_field(&fields, "think", 0)?,
            nextthink: NativeModNextthink {
                offset: u64_field(&nextthink, "offset", 0)?,
                encoding: scalar_encoding(nextthink.field("encoding"))?,
                units: match nextthink
                    .field("units")
                    .choice_str(&["seconds", "milliseconds"])?
                    .as_str()
                {
                    "seconds" => ModTimeUnits::Seconds,
                    _ => ModTimeUnits::Milliseconds,
                },
            },
        },
        callbacks,
        combat,
    })
}

/// Read a nullable nonnegative integer field.
fn nullable_u64(reader: SaveReader) -> Result<Option<u64>, ModsError> {
    reader.nullable(|value| -> Result<_, ModsError> {
        let raw = value.integer(0)?;
        as_u64(&value, raw)
    })
}

/// Read native combat.
fn native_combat(reader: SaveReader) -> Result<NativeModCombat, ModsError> {
    let causes = reader.field("causes");
    let damage = reader.field("damage");
    let flags = reader.field("flags");
    let deferred = reader.field("deferred");
    Ok(NativeModCombat {
        damage: NativeModDamageEntry {
            entry: native_entry(damage.field("entry"))?,
            abi: q2_abi(damage.field("abi"))?,
        },
        causes: if causes.field("edition").choice_str(&["classic", "rerelease"])? == "classic" {
            NativeModDamageCauses::Classic {
                game: match causes
                    .field("game")
                    .choice_str(&["base", "xatrix", "rogue", "ctf"])?
                    .as_str()
                {
                    "base" => NativeClassicGame::Base,
                    "xatrix" => NativeClassicGame::Xatrix,
                    "rogue" => NativeClassicGame::Rogue,
                    _ => NativeClassicGame::Ctf,
                },
            }
        } else {
            NativeModDamageCauses::Rerelease
        },
        health: scalar_field(reader.field("health"))?,
        mass: scalar_field(reader.field("mass"))?,
        takedamage: scalar_field(reader.field("takedamage"))?,
        flags: NativeModCombatFlags {
            offset: u64_field(&flags, "offset", 0)?,
            encoding: scalar_encoding(flags.field("encoding"))?,
            invulnerable: u64_field(&flags, "invulnerable", 0)?,
            no_knockback: u64_field(&flags, "noKnockback", 0)?,
        },
        armor: native_armor(reader.field("armor"))?,
        deferred: if deferred.is_missing() {
            None
        } else {
            Some(NativeModDeferredDamage {
                process: native_entry(deferred.field("process"))?,
                attacker: u64_field(&deferred, "attacker", 0)?,
                inflictor: u64_field(&deferred, "inflictor", 0)?,
                blood: scalar_field(deferred.field("blood"))?,
                knockback: scalar_field(deferred.field("knockback"))?,
                point: u64_field(&deferred, "point", 0)?,
                r#mod: u64_field(&deferred, "mod", 0)?,
                receipt: u64_field(&deferred, "receipt", 0)?,
            })
        },
    })
}

/// Read native items.
fn native_items(reader: SaveReader) -> Result<NativeModItems, ModsError> {
    let definitions = reader.field("definitions").list(|value| -> Result<_, ModsError> {
        let icon = if value.field("icon").is_missing() {
            None
        } else {
            value
                .field("icon")
                .nullable(|entry| read_item_icon_declaration(&entry))?
        };
        let actions = if value.field("actions").is_missing() {
            None
        } else {
            Some(read_item_actions(value.field("actions"), native_source_call)?)
        };
        let admission = match value
            .field("admission")
            .choice_str(&["add", "replace-primary"])?
            .as_str()
        {
            "add" => SourceItemAdmissionMode::Add,
            _ => SourceItemAdmissionMode::ReplacePrimary,
        };
        let item = namespaced(value.field("item"))?;
        let label = value.field("label").string()?;
        if value.field("kind").choice_str(&["counter", "weapon"])? == "counter" {
            Ok(NativeItemDefinition {
                item,
                label,
                icon,
                admission,
                actions,
                kind: NativeItemKind::Counter,
            })
        } else {
            let held = if value.field("held").is_missing() {
                None
            } else {
                Some(read_held_weapon_declaration(&value.field("held"))?)
            };
            Ok(NativeItemDefinition {
                item,
                label,
                icon,
                admission,
                actions,
                kind: NativeItemKind::Weapon(SourceWeaponItem {
                    ammo: value.field("ammo").nullable(namespaced)?,
                    held,
                }),
            })
        }
    })?;
    let storage = reader.field("storage").list(|value| -> Result<_, ModsError> {
        let field = armor_field(value.field("field"))?;
        if value.field("kind").choice_str(&["counter", "bits"])? == "bits" {
            return Ok(NativeItemStorage::Bits {
                field,
                private_mask: int_number(&value, "privateMask", 0)?,
                items: value.field("items").list(|entry| -> Result<_, ModsError> {
                    Ok(crate::contract::NativeMaskedItem {
                        item: namespaced(entry.field("item"))?,
                        mask: int_number(&entry, "mask", 1)?,
                    })
                })?,
            });
        }
        let capacity = value.field("capacity");
        let kind = capacity.field("kind").choice_str(&["constant", "field", "source"])?;
        Ok(NativeItemStorage::Counter {
            field,
            item: namespaced(value.field("item"))?,
            capacity: if kind == "constant" {
                NativeItemCapacity::Constant {
                    value: capacity.field("value").number()?,
                }
            } else if kind == "field" {
                NativeItemCapacity::Field {
                    field: armor_field(capacity.field("field"))?,
                }
            } else {
                NativeItemCapacity::Source {
                    address: native_address(capacity.field("address"))?,
                    encoding: scalar_encoding(capacity.field("encoding"))?,
                }
            },
        })
    })?;
    let weapons = if reader.field("weapons").is_missing() {
        None
    } else {
        let weapon = reader.field("weapons");
        let dispatcher = weapon.field("dispatcher");
        let selection = weapon.field("selection");
        Some(NativeWeaponStage {
            dispatcher: NativeWeaponDispatcher {
                entry: native_entry(dispatcher.field("entry"))?,
                record: dispatcher.field("record").string()?,
                argument: u64_field(&dispatcher, "argument", 0)?,
                arguments: u64_field(&dispatcher, "arguments", 1)?,
            },
            decisions: weapon.field("decisions").list(|value| -> Result<_, ModsError> {
                Ok(NativeWeaponDecision {
                    entry: u64_field(&value, "entry", 0)?,
                    join: u64_field(&value, "join", 0)?,
                    fields: value.field("fields").list(|entry| -> Result<_, ModsError> {
                        Ok(NativeWeaponClearedField {
                            field: armor_field(entry.field("field"))?,
                            clear_mask: u64_field(&entry, "clearMask", 1)?,
                        })
                    })?,
                })
            })?,
            committed_input: if weapon.field("committedInput").is_missing() {
                Vec::new()
            } else {
                weapon
                    .field("committedInput")
                    .list(|value| value.list(native_item_test))?
            },
            continuations: weapon
                .field("continuations")
                .list(|value| value.list(native_item_test))?,
            settled: weapon.field("settled").list(|value| value.list(native_item_test))?,
            selection: NativeWeaponSelection {
                active: native_item_pointer(selection.field("active"))?,
                pending: selection.field("pending").nullable(native_item_pointer)?,
                values: selection.field("values").list(|value| -> Result<_, ModsError> {
                    Ok(NativeWeaponValue {
                        item: namespaced(value.field("item"))?,
                        address: native_address(value.field("address"))?,
                        request: native_source_call(value.field("request"))?,
                    })
                })?,
            },
        })
    };
    Ok(NativeModItems {
        definitions,
        storage,
        weapons,
    })
}

/// Read a native item pointer.
fn native_item_pointer(reader: SaveReader) -> Result<NativeItemPointer, ModsError> {
    Ok(NativeItemPointer {
        record: reader.field("record").string()?,
        offset: u64_field(&reader, "offset", 0)?,
    })
}

/// Read a native item test.
fn native_item_test(reader: SaveReader) -> Result<NativeItemTest, ModsError> {
    if reader.field("kind").choice_str(&["scalar", "pointer"])? == "pointer" {
        Ok(NativeItemTest::Pointer {
            field: native_item_pointer(reader.field("field"))?,
            value: reader.field("value").nullable(native_address)?,
        })
    } else {
        Ok(NativeItemTest::Scalar {
            field: armor_field(reader.field("field"))?,
            mask: reader
                .field("mask")
                .nullable(|value| value.integer(0))?
                .map(as_int_number),
            comparison: match reader.field("comparison").choice_str(&["equals", "at-most"])?.as_str() {
                "equals" => ItemTestComparison::Equals,
                _ => ItemTestComparison::AtMost,
            },
            value: reader.field("value").number()?,
        })
    }
}

/// Read a native mod pickup context value.
fn native_pickup_value(field: SaveReader) -> Result<NativeModPickupValue, ModsError> {
    match native_argument(field.field("value"))? {
        NativeModValue::Time { input, units, encoding } => Ok(NativeModPickupValue::Time { input, units, encoding }),
        NativeModValue::Address(value) => Ok(NativeModPickupValue::Address(value)),
        NativeModValue::Value { kind, value } => match kind {
            NativeModValueKind::Scalar(scalar) => Ok(NativeModPickupValue::Value {
                kind: NativeModScalarValueKind::Scalar(scalar),
                value,
            }),
            NativeModValueKind::Vector => Ok(NativeModPickupValue::Value {
                kind: NativeModScalarValueKind::Vector,
                value,
            }),
            NativeModValueKind::Str => Err(field
                .fail("Native pickup context requires scalar, vector, time or image address values")
                .into()),
        },
        _ => Err(field
            .fail("Native pickup context requires scalar, vector, time or image address values")
            .into()),
    }
}

/// Read a native client input field value.
fn native_input_field_value(field: SaveReader) -> Result<NativeModClientInputValue, ModsError> {
    match native_argument(field.field("value"))? {
        NativeModValue::Time { input, units, encoding } => {
            Ok(NativeModClientInputValue::Time { input, units, encoding })
        }
        NativeModValue::Value { kind, value } => match kind {
            NativeModValueKind::Scalar(scalar) => Ok(NativeModClientInputValue::Value {
                kind: NativeModScalarValueKind::Scalar(scalar),
                value,
            }),
            NativeModValueKind::Vector => Ok(NativeModClientInputValue::Value {
                kind: NativeModScalarValueKind::Vector,
                value,
            }),
            NativeModValueKind::Str => Err(field
                .fail("Native input fields require scalar, vector or time values")
                .into()),
        },
        _ => Err(field
            .fail("Native input fields require scalar, vector or time values")
            .into()),
    }
}

/// Read native clients.
fn native_clients(reader: SaveReader) -> Result<NativeModClients, ModsError> {
    Ok(NativeModClients {
        outputs: if reader.field("outputs").is_missing() {
            Vec::new()
        } else {
            read_client_output_declarations(
                reader.field("outputs"),
                |value| {
                    Ok(NativeModArmorField {
                        record: value.field("record").string()?,
                        offset: u64_field(&value, "offset", 0)?,
                        encoding: scalar_encoding(value.field("encoding"))?,
                    })
                },
                native_item_pointer,
            )?
        },
        maximum: u64_field(&reader, "maximum", 1)?,
        records: reader.field("records").list(|value| value.string())?,
        admit: reader.field("admit").list(|entry| -> Result<_, ModsError> {
            Ok(NativeModAdmissionCall {
                call: native_source_call(entry.clone())?,
                accepts: match entry.field("accepts").choice_str(&["always", "nonzero"])?.as_str() {
                    "always" => NativeModAcceptance::Always,
                    _ => NativeModAcceptance::Nonzero,
                },
            })
        })?,
        userinfo: reader.field("userinfo").list(native_source_call)?,
        disconnect: reader.field("disconnect").list(native_source_call)?,
        command: reader.field("command").list(native_source_call)?,
        input: if reader.field("input").is_missing() {
            Vec::new()
        } else {
            read_mod_client_input(reader.field("input"), native_source_call, Some(native_input_output))?
        },
        frame: if reader.field("frame").is_missing() {
            Vec::new()
        } else {
            reader.field("frame").list(native_source_call)?
        },
        end_frame: if reader.field("endFrame").is_missing() {
            Vec::new()
        } else {
            reader.field("endFrame").list(native_source_call)?
        },
        input_fields: if reader.field("inputFields").is_missing() {
            Vec::new()
        } else {
            reader.field("inputFields").list(|field| -> Result<_, ModsError> {
                Ok(NativeModClientInputField {
                    record: field.field("record").string()?,
                    offset: u64_field(&field, "offset", 0)?,
                    value: native_input_field_value(field.clone())?,
                })
            })?
        },
        pose: if reader.field("pose").is_missing() {
            None
        } else {
            let pose = reader.field("pose");
            let crouched = pose.field("crouched");
            Some(NativeModPose {
                view_height: armor_field(pose.field("viewHeight"))?,
                crouched: NativeModMaskedField {
                    field: armor_field(crouched.field("field"))?,
                    mask: int_number(&crouched, "mask", 1)?,
                },
            })
        },
    })
}

/// Read a native mod declaration.
pub fn read_native_mod_declaration(reader: SaveReader) -> Result<NativeModDeclaration, ModsError> {
    let program = reader.field("program");
    let target = reader.field("target");
    let api = target.field("api");
    let abi = target.field("abi");
    let classic = api
        .field("kind")
        .choice_str(&["q2-classic-game", "q2-rerelease-game"])?
        == "q2-classic-game";
    if classic {
        api.field("version").literal_i64(3)?;
        abi.field("kind").literal_str("windows-i386")?;
        abi.field("image").literal_str("pe32")?;
        abi.field("pointerBytes").literal_i64(4)?;
        abi.field("call").literal_str("cdecl")?;
    } else {
        api.field("version").literal_i64(2023)?;
        abi.field("kind").literal_str("windows-x86-64")?;
        abi.field("image").literal_str("pe32+")?;
        abi.field("pointerBytes").literal_i64(8)?;
        abi.field("call").literal_str("microsoft-x64")?;
    }
    reader.field("version").literal_i64(1)?;
    reader.field("runtime").literal_str("native")?;
    let items = if reader.field("items").is_missing() {
        None
    } else {
        Some(native_items(reader.field("items"))?)
    };
    let client_presentation = if reader.field("clientPresentation").is_missing() {
        None
    } else {
        let presentation = reader.field("clientPresentation");
        Some(crate::contract::NativeModClientPresentation {
            hud: match presentation
                .field("hud")
                .choice_str(&["none", "layout-overlay", "replace-status"])?
                .as_str()
            {
                "none" => crate::contract::NativeModHudPresentation::None,
                "layout-overlay" => crate::contract::NativeModHudPresentation::LayoutOverlay,
                _ => crate::contract::NativeModHudPresentation::ReplaceStatus,
            },
            view: match presentation
                .field("view")
                .choice_str(&["none", "playerstate"])?
                .as_str()
            {
                "none" => crate::contract::NativeModViewPresentation::None,
                _ => crate::contract::NativeModViewPresentation::Playerstate,
            },
        })
    };
    let source_actors = if reader.field("sourceActors").is_missing() {
        None
    } else {
        Some(native_source_actors(reader.field("sourceActors"))?)
    };
    let protection = if reader.field("protection").is_missing() && reader.field("poweredProtection").is_missing() {
        Vec::new()
    } else {
        native_protections(reader.clone())?
    };
    let pickups = if reader.field("pickups").is_missing() {
        Vec::new()
    } else {
        reader.field("pickups").list(|pickup| -> Result<_, ModsError> {
            let context = if pickup.field("context").is_missing() {
                Vec::new()
            } else {
                pickup.field("context").list(|field| -> Result<_, ModsError> {
                    Ok(NativeModPickupContext {
                        record: field.field("record").string()?,
                        offset: u64_field(&field, "offset", 0)?,
                        value: native_pickup_value(field.clone())?,
                    })
                })?
            };
            Ok(NativeModPickup {
                rule: read_mod_pickup_rule(pickup.clone(), native_source_call)?,
                context,
            })
        })?
    };
    let clients = if reader.field("clients").is_missing() {
        None
    } else {
        Some(native_clients(reader.field("clients"))?)
    };
    let objectives = if reader.field("objectives").is_missing() {
        Vec::new()
    } else {
        read_source_objectives(
            reader.field("objectives"),
            |value| {
                Ok(NativeModObjectiveStorage {
                    address: native_address(value.field("address"))?,
                    encoding: scalar_encoding(value.field("encoding"))?,
                })
            },
            native_address,
            native_source_call,
        )?
    };
    Ok(NativeModDeclaration {
        objectives,
        client_presentation,
        version: 1,
        program: ModProgram {
            path: normalize_resource_path(&program.field("path").string()?)?,
            digest: read_digest(program.field("digest"))?,
        },
        target: if classic {
            NativeModTarget::ClassicWindowsI386
        } else {
            NativeModTarget::RereleaseWindowsX86_64
        },
        source_actors,
        protection,
        pickups,
        items,
        clients,
        cvars: reader.field("cvars").list(|value| -> Result<_, ModsError> {
            Ok(ModCvar {
                name: value.field("name").string()?,
                value: value.field("value").string()?,
            })
        })?,
        spawn_entities: reader.field("spawnEntities").nullable(|value| value.string())?,
        actor_records: reader.field("actorRecords").list(|record| -> Result<_, ModsError> {
            let base = record.field("base");
            Ok(NativeModActorRecord {
                id: record.field("id").string()?,
                base: match base
                    .field("kind")
                    .choice_str(&["entities", "clients", "address"])?
                    .as_str()
                {
                    "entities" => NativeModRecordBase::Entities,
                    "clients" => NativeModRecordBase::Clients,
                    _ => NativeModRecordBase::Address(native_address(base)?),
                },
                stride: u64_field(&record, "stride", 4)?,
                first_slot: u64_field(&record, "firstSlot", 0)?,
                capacity: u64_field(&record, "capacity", 1)?,
                fields: record.field("fields").list(native_field)?,
            })
        })?,
        entity_record: reader.field("entityRecord").nullable(|value| value.string())?,
        initialize: reader.field("initialize").list(native_source_call)?,
        project: reader.field("project").list(native_source_call)?,
        release: reader.field("release").list(native_source_call)?,
        callbacks: reader.field("callbacks").list(|entry| -> Result<_, ModsError> {
            Ok(NativeModCallback {
                call: native_source_call(entry.clone())?,
                binding: native_binding(entry)?,
            })
        })?,
    })
}

/// Read a native mod declaration file.
pub fn read_native_mod_callbacks(bytes: &[u8]) -> Result<NativeModDeclaration, ModsError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ModsError::Invalid("Declaration bytes are not valid UTF-8".to_string()))?;
    let value = parse_save_json(text)?;
    read_native_mod_declaration(SaveReader::new(&value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ProviderReference;
    use crate::value::{arr, boolean, num, obj, str};

    fn reader(value: &SaveJson) -> SaveReader<'_> {
        SaveReader::new(value)
    }

    fn source_call_fixture() -> SaveJson {
        obj(vec![
            ("function", str("T_Damage")),
            (
                "arguments",
                arr(vec![
                    obj(vec![("kind", str("input")), ("name", str("self"))]),
                    obj(vec![("kind", str("float")), ("value", num(2.5))]),
                    obj(vec![("kind", str("string")), ("value", str("hi"))]),
                    obj(vec![
                        ("kind", str("vector")),
                        ("value", obj(vec![("x", num(1.0)), ("y", num(0.0)), ("z", num(0.0))])),
                    ]),
                ]),
            ),
            (
                "globals",
                arr(vec![obj(vec![
                    ("name", str("self")),
                    ("value", obj(vec![("kind", str("input")), ("name", str("other"))])),
                ])]),
            ),
        ])
    }

    #[test]
    fn reads_source_calls() {
        let value = source_call_fixture();
        let call = read_mod_source_call(reader(&value)).unwrap();
        assert_eq!(call.function, "T_Damage");
        assert_eq!(call.arguments.len(), 4);
        assert!(matches!(
            call.arguments[0],
            ModCallbackValue::Input(ModCallbackInput::Slf)
        ));
        assert!(matches!(call.arguments[1], ModCallbackValue::Float(_)));
        assert_eq!(call.globals.len(), 1);
        let bad = obj(vec![("kind", str("input")), ("name", str("bogus"))]);
        assert!(read_mod_source_value(reader(&bad)).is_err());
    }

    #[test]
    fn reads_client_input() {
        let value = arr(vec![
            obj(vec![
                ("scope", str("client-command")),
                ("phase", str("before")),
                ("calls", arr(vec![str("a")])),
                ("outputs", arr(vec![str("o")])),
            ]),
            obj(vec![
                ("scope", str("movement-slice")),
                ("phase", str("after")),
                ("calls", arr(vec![])),
            ]),
        ]);
        let bindings = read_mod_client_input(
            reader(&value),
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            Some(|entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) }),
        )
        .unwrap();
        assert_eq!(bindings.len(), 2);
        assert!(matches!(bindings[0].phase, ModClientInputPhase::Before { .. }));
        assert!(matches!(bindings[1].phase, ModClientInputPhase::After));
        type NoOutput = for<'a> fn(SaveReader<'a>) -> Result<String, ModsError>;
        let error = read_mod_client_input(
            reader(&value),
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            None::<NoOutput>,
        )
        .unwrap_err();
        assert!(error.to_string().contains("declared before source adapter"));
        let after = arr(vec![obj(vec![
            ("scope", str("client-command")),
            ("phase", str("after")),
            ("calls", arr(vec![])),
            ("outputs", arr(vec![str("o")])),
        ])]);
        let error = read_mod_client_input(
            reader(&after),
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            Some(|entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("declared before source adapter"));
    }

    #[test]
    fn reads_client_outputs() {
        let scalar = |entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) };
        let vector = |entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) };
        let body = arr(vec![obj(vec![
            ("kind", str("body-shape")),
            ("min", str("a")),
            ("max", str("b")),
        ])]);
        let declarations = read_client_output_declarations(reader(&body), scalar, vector).unwrap();
        assert!(matches!(declarations[0], ModClientOutputDeclaration::BodyShape { .. }));
        let both = arr(vec![obj(vec![
            ("kind", str("view-offset")),
            ("field", str("a")),
            ("height", str("b")),
        ])]);
        let scalar = |entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) };
        let vector = |entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) };
        let error = read_client_output_declarations(reader(&both), scalar, vector).unwrap_err();
        assert!(error.to_string().contains("not both"));
        let mode = arr(vec![obj(vec![
            ("kind", str("movement-mode")),
            ("field", str("f")),
            ("mask", num(3.0)),
            (
                "values",
                arr(vec![obj(vec![("value", num(1.0)), ("mode", str("noclip"))])]),
            ),
        ])]);
        let scalar = |entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) };
        let vector = |entry: SaveReader| -> Result<String, ModsError> { Ok(entry.string()?) };
        let declarations = read_client_output_declarations(reader(&mode), scalar, vector).unwrap();
        assert!(matches!(
            declarations[0],
            ModClientOutputDeclaration::MovementMode { .. }
        ));
    }

    #[test]
    fn reads_item_actions() {
        let both = obj(vec![("use", str("u")), ("drop", str("d"))]);
        let actions = read_item_actions(reader(&both), |entry| -> Result<String, ModsError> {
            Ok(entry.string()?)
        })
        .unwrap();
        assert_eq!(actions.use_call.as_deref(), Some("u"));
        assert_eq!(actions.drop_call.as_deref(), Some("d"));
        let neither = obj(vec![]);
        let error = read_item_actions(reader(&neither), |entry| -> Result<String, ModsError> {
            Ok(entry.string()?)
        })
        .unwrap_err();
        assert!(error.to_string().contains("use or drop"));
    }

    #[test]
    fn reads_team_values() {
        let value = arr(vec![
            obj(vec![("value", num(1.0)), ("team", str("red"))]),
            obj(vec![("value", num(2.0)), ("team", SaveJson::Null)]),
        ]);
        let values = read_source_team_values(reader(&value)).unwrap();
        assert_eq!(values.len(), 2);
        let duplicate = arr(vec![
            obj(vec![("value", num(1.0)), ("team", str("red"))]),
            obj(vec![("value", num(1.0)), ("team", str("blue"))]),
        ]);
        let error = read_source_team_values(reader(&duplicate)).unwrap_err();
        assert!(error.to_string().contains("distinct original values"));
        assert!(validate_source_match_field(&SourceMatchField::Score).is_ok());
    }

    #[test]
    fn reads_team_aliases_and_primary() {
        let aliases = arr(vec![obj(vec![("source", str("a")), ("team", str("red"))])]);
        assert_eq!(read_source_team_aliases(reader(&aliases)).unwrap().len(), 1);
        let duplicate = arr(vec![
            obj(vec![("source", str("a")), ("team", str("red"))]),
            obj(vec![("source", str("a")), ("team", str("blue"))]),
        ]);
        assert!(read_source_team_aliases(reader(&duplicate)).is_err());
        let primary = obj(vec![
            ("score", num(0.0)),
            (
                "teams",
                arr(vec![obj(vec![
                    ("source", SaveJson::Null),
                    ("team", str("red")),
                    ("arguments", arr(vec![str("team"), str("red")])),
                ])]),
            ),
        ]);
        assert!(read_source_primary_match(reader(&primary), 64).is_ok());
        let bad_score = obj(vec![("score", num(2.0)), ("teams", arr(vec![]))]);
        let error = read_source_primary_match(reader(&bad_score), 64).unwrap_err();
        assert!(error.to_string().contains("outside the original client record"));
    }

    #[test]
    fn reads_objectives() {
        let value = arr(vec![
            obj(vec![
                ("id", str("mod:flag")),
                (
                    "state",
                    obj(vec![
                        ("storage", str("s")),
                        (
                            "values",
                            arr(vec![obj(vec![
                                ("value", num(1.0)),
                                ("stage", str("home")),
                                ("complete", boolean(false)),
                            ])]),
                        ),
                    ]),
                ),
                ("carrier", SaveJson::Null),
                ("target", SaveJson::Null),
                ("role", str("owned")),
                ("campaignGate", boolean(true)),
                ("botGoal", boolean(false)),
                ("change", SaveJson::Null),
            ]),
            obj(vec![
                ("id", str("mod:ball")),
                (
                    "state",
                    obj(vec![
                        ("storage", str("s")),
                        (
                            "values",
                            arr(vec![obj(vec![
                                ("value", num(1.0)),
                                ("stage", str("held")),
                                ("complete", boolean(true)),
                            ])]),
                        ),
                    ]),
                ),
                ("carrier", SaveJson::Null),
                ("target", SaveJson::Null),
                ("role", str("borrowed")),
                ("writable", boolean(true)),
            ]),
        ]);
        let declarations = read_source_objectives(
            reader(&value),
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
        )
        .unwrap();
        assert_eq!(declarations.len(), 2);
        assert!(matches!(declarations[0].role, SourceObjectiveRole::Owned { .. }));
        let duplicate = arr(vec![obj(vec![
            ("id", str("mod:flag")),
            ("state", obj(vec![("storage", str("s")), ("values", arr(vec![]))])),
            ("carrier", SaveJson::Null),
            ("target", SaveJson::Null),
            ("role", str("borrowed")),
            ("writable", boolean(false)),
        ])]);
        // Empty stage values fail before the duplicate check.
        assert!(read_source_objectives(
            reader(&duplicate),
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
            |entry| -> Result<String, ModsError> { Ok(entry.string()?) },
        )
        .is_err());
    }

    fn pickup_call() -> SaveJson {
        obj(vec![("kind", str("boolean-grant")), ("grant", str("g"))])
    }

    #[test]
    fn reads_pickup_rules() {
        let legacy = obj(vec![
            ("id", str("rule")),
            ("offered", arr(vec![str("q1:shells")])),
            (
                "resource",
                obj(vec![("kind", str("inventory")), ("item", str("q1:shells"))]),
            ),
            ("operation", pickup_call()),
        ]);
        let rule = read_mod_pickup_rule(reader(&legacy), |entry| -> Result<String, ModsError> {
            Ok(entry.string()?)
        })
        .unwrap();
        assert_eq!(rule.writes.len(), 1);
        assert!(matches!(
            rule.writes[0],
            PickupWrite::Inventory {
                fields: PickupWriteFields::Count,
                ..
            }
        ));
        let modern = obj(vec![
            ("id", str("rule")),
            ("offered", arr(vec![str("q1:shells")])),
            (
                "writes",
                arr(vec![
                    obj(vec![
                        ("kind", str("inventory")),
                        ("item", str("q1:shells")),
                        ("fields", str("count-and-capacity")),
                    ]),
                    obj(vec![("kind", str("protection")), ("channel", str("regular"))]),
                ]),
            ),
            (
                "operation",
                obj(vec![
                    ("kind", str("gate-then-grant")),
                    ("gate", str("gate")),
                    ("grant", str("grant")),
                    ("grantAccepts", str("always")),
                ]),
            ),
        ]);
        let rule = read_mod_pickup_rule(reader(&modern), |entry| -> Result<String, ModsError> {
            Ok(entry.string()?)
        })
        .unwrap();
        assert_eq!(rule.writes.len(), 2);
        assert!(matches!(rule.operation, OriginalPickupOperation::GateThenGrant { .. }));
        let both = obj(vec![
            ("id", str("rule")),
            ("offered", arr(vec![str("q1:shells")])),
            (
                "resource",
                obj(vec![("kind", str("inventory")), ("item", str("q1:shells"))]),
            ),
            ("writes", arr(vec![])),
            ("operation", pickup_call()),
        ]);
        let error = read_mod_pickup_rule(reader(&both), |entry| -> Result<String, ModsError> {
            Ok(entry.string()?)
        })
        .unwrap_err();
        assert!(error.to_string().contains("both resource and writes"));
    }

    fn qvm_call_fixture(reader: SaveReader) -> Result<crate::contract::QvmModSourceCall, ModsError> {
        Ok(crate::contract::QvmModSourceCall {
            entry: u32_field(&reader, "entry", 0)?,
            arguments: Vec::new(),
            globals: Vec::new(),
            returns: crate::contract::QvmModReturn::Void,
        })
    }

    #[test]
    fn reads_qvm_items() {
        let value = obj(vec![
            (
                "definitions",
                arr(vec![
                    obj(vec![
                        ("item", str("q3:shells")),
                        ("label", str("Shells")),
                        ("admission", str("add")),
                        ("kind", str("counter")),
                    ]),
                    obj(vec![
                        ("item", str("q3:shotgun")),
                        ("label", str("Shotgun")),
                        ("admission", str("replace-primary")),
                        (
                            "actions",
                            obj(vec![(
                                "use",
                                obj(vec![
                                    ("entry", num(4.0)),
                                    ("arguments", arr(vec![])),
                                    ("globals", arr(vec![])),
                                    ("returns", str("void")),
                                ]),
                            )]),
                        ),
                        ("kind", str("weapon")),
                        ("ammo", str("q3:shells")),
                    ]),
                ]),
            ),
            (
                "storage",
                arr(vec![
                    obj(vec![
                        ("kind", str("counter")),
                        ("field", obj(vec![("record", str("player")), ("offset", num(8.0))])),
                        ("item", str("q3:shells")),
                        ("capacity", obj(vec![("kind", str("constant")), ("value", num(200.0))])),
                    ]),
                    obj(vec![
                        ("kind", str("bits")),
                        ("field", obj(vec![("record", str("player")), ("offset", num(12.0))])),
                        ("privateMask", num(255.0)),
                        (
                            "items",
                            arr(vec![obj(vec![("item", str("q3:shotgun")), ("mask", num(2.0))])]),
                        ),
                    ]),
                ]),
            ),
        ]);
        let items = read_qvm_mod_items(reader(&value), qvm_call_fixture).unwrap();
        assert_eq!(items.definitions.len(), 2);
        assert!(matches!(items.definitions[1].kind, QvmItemKind::Weapon(_)));
        assert!(items.definitions[1].actions.as_ref().unwrap().use_call.is_some());
        assert_eq!(items.storage.len(), 2);
        assert!(items.weapons.is_none());
    }

    fn mod_description(
        product: &str,
        id: &str,
        requires: Vec<ModSelection>,
        conflicts: Vec<ModSelection>,
    ) -> ModDescription {
        ModDescription {
            selection: ModSelection {
                product: product.to_string(),
                id: id.to_string(),
            },
            source: ProviderReference {
                provider: ProviderId::new("q2", "game"),
                content: crate::contract::create_content_id(&crate::contract::ContentIdentity {
                    family: crate::contract::GameFamily::Q2,
                    edition: "classic".to_string(),
                    package: "baseq2".to_string(),
                    revision: "v1".to_string(),
                })
                .unwrap(),
            },
            title: format!("{product}/{id}"),
            source_title: "Source".to_string(),
            purpose: ModPurpose::Addition,
            requires,
            conflicts,
            availability: ModAvailability::Available,
        }
    }

    fn selection(product: &str, id: &str) -> ModSelection {
        ModSelection {
            product: product.to_string(),
            id: id.to_string(),
        }
    }

    #[test]
    fn resolves_selection_sets() {
        let a = selection("game", "a");
        let b = selection("game", "b");
        let set = ModSelectionSet::new(
            vec![
                mod_description("game", "a", vec![], vec![]),
                mod_description("game", "b", vec![a.clone()], vec![]),
            ],
            vec![b.clone()],
        )
        .unwrap();
        assert_eq!(set.enabled(), &[a.clone(), b.clone()]);
        assert!(set.has(&a).unwrap());
        assert!(!set.has(&selection("game", "missing")).unwrap());
        // Dependency cycle.
        let cyclic = ModSelectionSet::new(
            vec![
                mod_description("game", "a", vec![b.clone()], vec![]),
                mod_description("game", "b", vec![a.clone()], vec![]),
            ],
            vec![a.clone()],
        )
        .unwrap_err();
        assert!(cyclic.to_string().contains("dependency cycle"));
        // Conflict.
        let conflicting = ModSelectionSet::new(
            vec![
                mod_description("game", "a", vec![], vec![b.clone()]),
                mod_description("game", "b", vec![], vec![]),
            ],
            vec![a.clone(), b.clone()],
        )
        .unwrap_err();
        assert!(conflicting.to_string().contains("conflicts with"));
        // Disable cascades to dependents.
        let mut set = ModSelectionSet::new(
            vec![
                mod_description("game", "a", vec![], vec![]),
                mod_description("game", "b", vec![a.clone()], vec![]),
            ],
            vec![b.clone()],
        )
        .unwrap();
        set.set_enabled(&a, false).unwrap();
        assert!(set.enabled().is_empty());
        // Duplicate declarations fail refresh.
        let mut set = ModSelectionSet::new(vec![], vec![]).unwrap();
        let error = set
            .refresh(vec![
                mod_description("game", "a", vec![], vec![]),
                mod_description("game", "a", vec![], vec![]),
            ])
            .unwrap_err();
        assert!(error.to_string().contains("Duplicate mod declaration"));
    }

    fn minimal_native_declaration(extra: &str) -> String {
        format!(
            r#"{{"version": 1, "runtime": "native",
            "program": {{"path": "game.dll", "digest": "sha256:{digest}"}},
            "target": {{"api": {{"kind": "q2-classic-game", "version": 3}},
              "abi": {{"kind": "windows-i386", "image": "pe32", "pointerBytes": 4, "call": "cdecl"}}}},
            "cvars": [], "spawnEntities": null, "actorRecords": [], "entityRecord": null,
            "initialize": [], "project": [], "release": [], "callbacks": []{extra}}}"#,
            digest = "ab".repeat(32),
            extra = extra
        )
    }

    #[test]
    fn reads_native_declarations() {
        let declaration = read_native_mod_callbacks(minimal_native_declaration("").as_bytes()).unwrap();
        assert_eq!(declaration.version, 1);
        assert!(matches!(declaration.target, NativeModTarget::ClassicWindowsI386));
        assert!(declaration.protection.is_empty());
        assert!(read_native_mod_callbacks(b"\xff\xfe").is_err());
        assert!(read_native_mod_callbacks(b"{oops").is_err());
        // Mixed protection layouts fail before parsing either layout.
        let mixed = minimal_native_declaration(r#", "protection": [], "poweredProtection": {}"#);
        let error = read_native_mod_callbacks(mixed.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("cannot mix legacy and current layouts"));
        // A power-armor Q2 protection parses through the real shape.
        let armored = minimal_native_declaration(
            r#", "protection": [{"id": "armor", "channel": "powered",
              "storage": [{"item": "q2:cells", "kind": "screen",
                "selection": {"kind": "positive", "field": {"record": "client", "offset": 0, "encoding": "int32"}},
                "cells": {"record": "client", "offset": 4, "encoding": "int32"}, "enabled": null}],
              "absorb": {"abi": "q2-check-power-armor",
                "entry": {"kind": "export", "name": "CheckPowerArmor"},
                "flags": "q2-classic"}}]"#,
        );
        let declaration = read_native_mod_callbacks(armored.as_bytes()).unwrap();
        assert_eq!(declaration.protection.len(), 1);
        assert!(matches!(
            declaration.protection[0].channel,
            NativeModProtectionChannel::Powered { .. }
        ));
        // Actor/record values are rejected from pickup context.
        let rejected = minimal_native_declaration(
            r#", "pickups": [{"id": "rule", "offered": ["q2:shells"],
              "writes": [{"kind": "inventory", "item": "q2:shells", "fields": "count"}],
              "operation": {"kind": "boolean-grant",
                "grant": {"entry": {"kind": "export", "name": "Pickup"},
                  "arguments": [], "globals": [], "returns": "void"}},
              "context": [{"record": "entity", "offset": 0,
                "value": {"kind": "actor", "record": "entity", "input": "self"}}]}]"#,
        );
        let error = read_native_mod_callbacks(rejected.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("pickup context requires scalar"));
        // Addresses are rejected from client input fields.
        let clients = minimal_native_declaration(
            r#", "clients": {"maximum": 8, "records": [], "admit": [], "userinfo": [],
              "disconnect": [], "command": [],
              "inputFields": [{"record": "move", "offset": 0,
                "value": {"kind": "address", "value": null}}]}"#,
        );
        let error = read_native_mod_callbacks(clients.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("input fields require scalar"));
    }
}
