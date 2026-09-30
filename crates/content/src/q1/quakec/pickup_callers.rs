//! Declared pickup callers (`src/content/q1/quakec/pickup-callers.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/pickup-callers.ts`
//! (`readQcPickupCaller`, `qcDeclaredPickupStages`).

use std::collections::HashSet;

use crate::contract::{ItemId, PickupResource, ProtectionChannel, QcStatement};
use crate::value::{namespaced, SaveReader};

use super::pickup_stage::{
    QcPickupDescriptor, QcPickupOperation, QcPickupRegion, QcPickupScalar, QcPickupStage, QcPickupStageDescriptor,
    QcPickupValue,
};
use super::qc_view::{signed_qc_branch, QcInlineRegion, QcOpcode, QcProgramView, QcValueType};
use super::{fround, QcError};

/// Declared pickup item (donor `QcPickupCallerDeclaration` item part).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupItem {
    /// Pickup item.
    pub item: ItemId,
    /// Resource binding.
    pub resource: Option<PickupResource>,
    /// Count source.
    pub count: Option<QcPickupScalar>,
}

/// Declared pickup item value (donor descriptor value entry).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupItemValue<T> {
    /// Pickup item.
    pub item: ItemId,
    /// Resource binding.
    pub resource: Option<PickupResource>,
    /// Count source.
    pub count: Option<QcPickupScalar>,
    /// Discriminator value.
    pub value: T,
}

/// Declared pickup descriptor (donor
/// `QcPickupCallerDeclaration.descriptor`).
#[derive(Debug, Clone, PartialEq)]
pub enum QcPickupCallerDescriptor {
    /// Constant descriptor.
    Constant(QcPickupItem),
    /// String-dispatched descriptor.
    Str {
        /// Dispatch field.
        field: String,
        /// Descriptors per value.
        values: Vec<QcPickupItemValue<String>>,
    },
    /// Float-dispatched descriptor.
    Float {
        /// Dispatch field.
        field: String,
        /// Descriptors per value.
        values: Vec<QcPickupItemValue<f64>>,
    },
}

/// Declared pickup region operation (donor region `operation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QcCallerOperation {
    /// Recipient decision.
    Decision {
        /// Predicate word.
        word: u32,
        /// Accepted value.
        accepted: f64,
    },
    /// Recipient admission.
    Admission,
    /// Resource grant.
    Grant,
    /// Pickup consumption.
    Consume,
}

/// Declared pickup region (donor `QcPickupCallerDeclaration.regions`
/// entry).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupCallerRegion {
    /// Region entry.
    pub entry: u32,
    /// Region exit.
    pub exit: u32,
    /// Exact source statements.
    pub statements: Vec<QcStatement>,
    /// Region operation.
    pub operation: QcCallerOperation,
}

/// Declared pickup caller (donor `QcPickupCallerDeclaration`).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupCallerDeclaration {
    /// Touch function name.
    pub function: String,
    /// Item descriptor.
    pub descriptor: QcPickupCallerDescriptor,
    /// Dropped-pickup source.
    pub dropped: Option<QcPickupScalar>,
    /// Declared regions.
    pub regions: Vec<QcPickupCallerRegion>,
}

/// Read a scalar source (donor `scalar`).
fn read_scalar(reader: SaveReader) -> Result<QcPickupScalar, QcError> {
    if reader.field("kind").choice_str(&["field", "global"])? == "field" {
        Ok(QcPickupScalar::Field {
            name: reader.field("name").string()?,
        })
    } else {
        Ok(QcPickupScalar::Global {
            word: u32_field(&reader, "word", 0)?,
        })
    }
}

/// Read a bounded integer field as `u32`.
fn u32_field(reader: &SaveReader, name: &str, minimum: i64) -> Result<u32, QcError> {
    let field = reader.field(name);
    let value = field.integer(minimum)?;
    u32::try_from(value).map_err(|_| QcError::from(field.fail("expected an integer in range")))
}

/// Read a pickup item (donor `item`).
fn read_item(reader: SaveReader) -> Result<QcPickupItem, QcError> {
    let resource = reader.field("resource").nullable(|value| {
        if value.field("kind").choice_str(&["protection", "inventory"])? == "protection" {
            Ok::<_, QcError>(PickupResource::Protection {
                channel: match value.field("channel").choice_str(&["regular", "powered"])?.as_str() {
                    "regular" => ProtectionChannel::Regular,
                    _ => ProtectionChannel::Powered,
                },
            })
        } else {
            Ok::<_, QcError>(PickupResource::Inventory {
                item: namespaced(value.field("item"))?,
            })
        }
    })?;
    Ok(QcPickupItem {
        item: namespaced(reader.field("item"))?,
        resource,
        count: if reader.field("count").is_missing() {
            None
        } else {
            Some(read_scalar(reader.field("count"))?)
        },
    })
}

/// Read a declared pickup caller (donor `readQcPickupCaller`).
pub fn read_qc_pickup_caller(reader: SaveReader) -> Result<QcPickupCallerDeclaration, QcError> {
    let descriptor = reader.field("descriptor");
    let kind = descriptor.field("kind").choice_str(&["constant", "string", "float"])?;
    let declared = match kind.as_str() {
        "constant" => QcPickupCallerDescriptor::Constant(read_item(descriptor.clone())?),
        "string" => QcPickupCallerDescriptor::Str {
            field: descriptor.field("field").string()?,
            values: descriptor.field("values").list(|value| {
                let item = read_item(value.clone())?;
                Ok::<_, QcError>(QcPickupItemValue {
                    item: item.item,
                    resource: item.resource,
                    count: item.count,
                    value: value.field("value").string()?,
                })
            })?,
        },
        _ => QcPickupCallerDescriptor::Float {
            field: descriptor.field("field").string()?,
            values: descriptor.field("values").list(|value| {
                let item = read_item(value.clone())?;
                Ok::<_, QcError>(QcPickupItemValue {
                    item: item.item,
                    resource: item.resource,
                    count: item.count,
                    value: value.field("value").finite()?,
                })
            })?,
        },
    };
    Ok(QcPickupCallerDeclaration {
        function: reader.field("function").string()?,
        descriptor: declared,
        dropped: if reader.field("dropped").is_missing() {
            None
        } else {
            Some(read_scalar(reader.field("dropped"))?)
        },
        regions: reader.field("regions").list(|region| {
            let operation = region.field("operation");
            let kind = operation
                .field("kind")
                .choice_str(&["decision", "admission", "grant", "consume"])?;
            Ok::<_, QcError>(QcPickupCallerRegion {
                entry: u32_field(&region, "entry", 0)?,
                exit: u32_field(&region, "exit", 0)?,
                statements: region.field("statements").list(|value| {
                    Ok::<_, QcError>(QcStatement {
                        opcode: u32_field(&value, "opcode", 0)?,
                        a: u32_field(&value, "a", 0)?,
                        b: u32_field(&value, "b", 0)?,
                        c: u32_field(&value, "c", 0)?,
                    })
                })?,
                operation: if kind == "decision" {
                    QcCallerOperation::Decision {
                        word: u32_field(&operation, "word", 0)?,
                        accepted: operation.field("accepted").finite()?,
                    }
                } else {
                    match kind.as_str() {
                        "admission" => QcCallerOperation::Admission,
                        "grant" => QcCallerOperation::Grant,
                        _ => QcCallerOperation::Consume,
                    }
                },
            })
        })?,
    })
}

/// Declarations are bound to the enclosing compatibility document's
/// exact program digest (donor `qcDeclaredPickupStages`).
pub fn qc_declared_pickup_stages(
    program: &QcProgramView,
    declarations: &[QcPickupCallerDeclaration],
) -> Result<Vec<QcPickupStage>, QcError> {
    let mut callers: HashSet<usize> = HashSet::new();
    let fail = |reason: &str| -> QcError {
        QcError::program(format!("Invalid declared pickup caller: {reason}"), program.source)
    };
    let check_global = |word: u32| -> Result<(), QcError> {
        if u64::from(word) * 4 >= program.initial_globals.len() as u64 {
            return Err(fail("global word is outside source memory"));
        }
        Ok(())
    };
    let check_field = |name: &str, expected: QcValueType| -> Result<(), QcError> {
        if program.field_named(name).is_none_or(|field| field.def_type != expected) {
            return Err(fail(&format!(
                "source field {name} is not {}",
                match expected {
                    QcValueType::Str => "string",
                    _ => "float",
                }
            )));
        }
        Ok(())
    };
    let check_value = |input: &QcPickupScalar| -> Result<QcPickupScalar, QcError> {
        match input {
            QcPickupScalar::Field { name } => check_field(name, QcValueType::Float)?,
            QcPickupScalar::Global { word } => check_global(*word)?,
        }
        Ok(input.clone())
    };
    let mut stages = Vec::with_capacity(declarations.len());
    for declaration in declarations {
        let function = program.function_named(&declaration.function)?;
        if function.first_statement <= 0
            || function.named_builtin
            || !function.parameter_sizes.is_empty()
            || callers.contains(&function.index)
        {
            return Err(fail("duplicate or non-touch source function"));
        }
        callers.insert(function.index);
        let end = program.function_end(function.first_statement);
        let first = usize::try_from(function.first_statement).unwrap_or(usize::MAX);
        match &declaration.descriptor {
            QcPickupCallerDescriptor::Constant(_) => {}
            QcPickupCallerDescriptor::Str { field, values } => {
                check_field(field, QcValueType::Str)?;
                let distinct: HashSet<&String> = values.iter().map(|value| &value.value).collect();
                if values.is_empty() || distinct.len() != values.len() {
                    return Err(fail("empty or ambiguous source item descriptors"));
                }
            }
            QcPickupCallerDescriptor::Float { field, values } => {
                check_field(field, QcValueType::Float)?;
                let distinct: HashSet<u64> = values.iter().map(|value| value.value.to_bits()).collect();
                if values.is_empty() || distinct.len() != values.len() {
                    return Err(fail("empty or ambiguous source item descriptors"));
                }
                if values.iter().any(|value| fround(value.value) != value.value) {
                    return Err(fail("item discriminator is not an exact source float"));
                }
            }
        }
        let mut regions: Vec<&QcPickupCallerRegion> = declaration.regions.iter().collect();
        regions.sort_by_key(|region| region.entry);
        if !regions.iter().any(|region| {
            matches!(
                region.operation,
                QcCallerOperation::Decision { .. } | QcCallerOperation::Admission
            )
        }) {
            return Err(fail("missing recipient admission boundary"));
        }
        let mut previous_end = first;
        for region in &regions {
            let entry = region.entry as usize;
            let exit = region.exit as usize;
            if entry < previous_end || exit <= entry || exit >= end {
                return Err(fail("overlapping or out-of-function regions"));
            }
            previous_end = exit;
            if region.statements.len() != exit.saturating_sub(entry).saturating_add(1) {
                return Err(fail("instruction proof must include the region and its join"));
            }
            for (offset, expected) in region.statements.iter().enumerate() {
                let actual = program.statements.get(entry + offset);
                if actual.is_none_or(|actual| {
                    actual.opcode.as_u32() != expected.opcode
                        || u32::from(actual.a) != expected.a
                        || u32::from(actual.b) != expected.b
                        || u32::from(actual.c) != expected.c
                }) {
                    return Err(fail("instruction proof differs from original artifact"));
                }
            }
            if let QcCallerOperation::Decision { word, accepted } = region.operation {
                check_global(word)?;
                let join = program.statements.get(exit);
                if join.is_none_or(|join| {
                    u32::from(join.a) != word || (join.opcode != QcOpcode::If && join.opcode != QcOpcode::IfNot)
                }) {
                    return Err(fail("decision join does not consume the declared source predicate"));
                }
                if fround(accepted) != accepted {
                    return Err(fail("accepted decision is not an exact source float"));
                }
            }
            for at in first..end {
                let Some(statement) = program.statements.get(at) else {
                    return Err(fail("missing source instruction"));
                };
                let target = if statement.opcode == QcOpcode::Goto {
                    Some(at as i64 + i64::from(signed_qc_branch(statement.a)))
                } else if statement.opcode == QcOpcode::If || statement.opcode == QcOpcode::IfNot {
                    Some(at as i64 + i64::from(signed_qc_branch(statement.b)))
                } else {
                    None
                };
                if target
                    .is_some_and(|target| (at < entry || at >= exit) && target > entry as i64 && target < exit as i64)
                {
                    return Err(fail("source branch enters the middle of a declared region"));
                }
            }
        }
        let describe =
            |item: &QcPickupItem| -> Result<(ItemId, Option<PickupResource>, Option<QcPickupScalar>), QcError> {
                Ok((
                    item.item.clone(),
                    item.resource.clone(),
                    item.count.as_ref().map(check_value).transpose()?,
                ))
            };
        let descriptor = match &declaration.descriptor {
            QcPickupCallerDescriptor::Constant(item) => {
                let (item_id, resource, count) = describe(item)?;
                QcPickupStageDescriptor::Constant {
                    value: QcPickupDescriptor {
                        value: QcPickupValue::Num(0.0),
                        item: item_id,
                        resource,
                        count,
                        supply: None,
                    },
                }
            }
            QcPickupCallerDescriptor::Str { field, values } => {
                let mut descriptors = Vec::with_capacity(values.len());
                for value in values {
                    let item = QcPickupItem {
                        item: value.item.clone(),
                        resource: value.resource.clone(),
                        count: value.count.clone(),
                    };
                    let (item_id, resource, count) = describe(&item)?;
                    descriptors.push(QcPickupDescriptor {
                        value: QcPickupValue::Str(value.value.clone()),
                        item: item_id,
                        resource,
                        count,
                        supply: None,
                    });
                }
                QcPickupStageDescriptor::Str {
                    field: field.clone(),
                    values: descriptors,
                }
            }
            QcPickupCallerDescriptor::Float { field, values } => {
                let mut descriptors = Vec::with_capacity(values.len());
                for value in values {
                    let item = QcPickupItem {
                        item: value.item.clone(),
                        resource: value.resource.clone(),
                        count: value.count.clone(),
                    };
                    let (item_id, resource, count) = describe(&item)?;
                    descriptors.push(QcPickupDescriptor {
                        value: QcPickupValue::Num(value.value),
                        item: item_id,
                        resource,
                        count,
                        supply: None,
                    });
                }
                QcPickupStageDescriptor::Float {
                    field: field.clone(),
                    values: descriptors,
                }
            }
        };
        stages.push(QcPickupStage {
            function_index: function.index,
            dropped: declaration.dropped.as_ref().map(check_value).transpose()?,
            descriptor,
            regions: regions
                .iter()
                .map(|region| QcPickupRegion {
                    region: QcInlineRegion {
                        function_index: function.index,
                        entry: region.entry as usize,
                        exit: region.exit as usize,
                        replaceable: true,
                        standalone: None,
                    },
                    recipient_self: false,
                    operation: match region.operation {
                        QcCallerOperation::Decision { word, accepted } => {
                            QcPickupOperation::Decision { word, accepted }
                        }
                        QcCallerOperation::Admission => QcPickupOperation::Admission,
                        QcCallerOperation::Grant => QcPickupOperation::Grant,
                        QcCallerOperation::Consume => QcPickupOperation::Consume,
                    },
                })
                .collect(),
            source_selection: None,
            source_effect: None,
        });
    }
    Ok(stages)
}
