//! Port of `src/compat/qc/profile.ts`: program search order, mounted
//! program loading, classic edict geometry, and host-profile description.
//!
//! Contracts absorption (pure Rust types): `QcWeaponStageDeclaration` /
//! `QcPrimaryWeaponStageDeclaration` (`src/contracts/qc-weapon-stage.ts`)
//! and `QcPickupCallerDeclaration` (`src/contracts/qc-pickup-callers.ts`),
//! plus the declaration-to-program binding of
//! `content/q1/quakec/pickup-callers.ts` (`qc_declared_pickup_stages`).
//! JSON readers for these declarations live in [`super::compatibility`].
//! Local mirrors: [`QcProgramContent`] stands in for the content-store
//! `MountedContent` owned by the content crate; [`QuakeCHostProfile`]
//! mirrors the execution contract with `crate::core::contracts` layouts.

use std::collections::HashSet;

use super::builtins::QcHostKind;
use super::machine::{QcBuiltinRegistry, QcInlineRegion};
use super::memory::QcEntityLayout;
use super::program::{load_qc_program, QuakeCApi, QcOpcode, QcProgram, QcValueType};
use super::source_call::ModSourceCall;
use crate::core::contracts::{CallbackId, GuestFieldLayout, GuestLayout, GuestStorage};
use crate::error::GuestError;

/// Program search order for a host kind.
pub fn qc_program_search_order(kind: QcHostKind) -> &'static [&'static str] {
    match kind {
        QcHostKind::Quakeworld => &["qwprogs.dat", "progs.dat"],
        QcHostKind::Netquake | QcHostKind::Rerelease => &["progs.dat"],
    }
}

/// One opened program candidate from the content store.
#[derive(Debug, Clone)]
pub struct QcOpenedProgram {
    /// Raw image bytes.
    pub bytes: Vec<u8>,
    /// Resolved resource reference (digest/path label).
    pub reference: String,
}

/// Minimal content-store surface needed to find a QuakeC program.
pub trait QcProgramContent {
    /// Open `path`, or return `None` when absent.
    fn open(&self, path: &str) -> Result<Option<QcOpenedProgram>, GuestError>;
}

/// A loaded program with its resolved reference.
#[derive(Debug, Clone)]
pub struct QcLoadedProgram {
    /// Loaded program.
    pub program: QcProgram,
    /// Resolved resource reference.
    pub reference: String,
}

/// Load the first program in search order, pinning the expected API.
pub fn load_mounted_qc_program(
    content: &dyn QcProgramContent,
    kind: QcHostKind,
) -> Result<QcLoadedProgram, GuestError> {
    let api = match kind {
        QcHostKind::Quakeworld => QuakeCApi::Quakeworld,
        QcHostKind::Netquake | QcHostKind::Rerelease => QuakeCApi::Netquake,
    };
    for path in qc_program_search_order(kind) {
        if let Some(opened) = content.open(path)? {
            let program = load_qc_program(&opened.bytes, Some(api), path)?;
            return Ok(QcLoadedProgram {
                program,
                reference: opened.reference,
            });
        }
    }
    Err(GuestError::bad_image(
        "progs.dat",
        format!("missing {}", qc_program_search_order(kind).join(" or ")),
    ))
}

/// WinQuake/QW i386 `edict_t` geometry: the prefix covers free, links,
/// leaves, baseline, and freetime (104 bytes on QuakeWorld, 96 otherwise).
#[must_use]
pub fn classic_qc_entity_layout(program: &QcProgram) -> QcEntityLayout {
    let variables_offset_bytes = if program.api.is_quakeworld() { 104 } else { 96 };
    QcEntityLayout {
        stride_bytes: variables_offset_bytes + program.entity_field_words * 4,
        variables_offset_bytes,
        field_words: program.entity_field_words,
    }
}

/// Builtin binding kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcBuiltinKind {
    /// Numbered builtin.
    Numbered,
    /// Named (`ex_*`) builtin.
    Named,
}

/// One described builtin binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcBuiltinBinding {
    /// Binding kind.
    pub kind: QcBuiltinKind,
    /// Builtin number (0 for named builtins).
    pub number: i32,
    /// Builtin name.
    pub name: String,
    /// Stable callback id.
    pub callback: CallbackId,
}

/// Host profile describing a loaded program to the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeCHostProfile {
    /// Program API.
    pub api: QuakeCApi,
    /// Program search order.
    pub program_search_order: Vec<String>,
    /// Global-words layout.
    pub globals_layout: GuestLayout,
    /// Entity-variable layout.
    pub entity_variables_layout: GuestLayout,
    /// Bound builtins.
    pub builtins: Vec<QcBuiltinBinding>,
    /// Advertised extensions.
    pub extensions: Vec<String>,
}

fn field_layout(
    name: &str,
    offset_words: usize,
    value_type: QcValueType,
    base: usize,
) -> GuestFieldLayout {
    GuestFieldLayout {
        name: name.to_string(),
        byte_offset: base + offset_words * 4,
        storage: match value_type {
            QcValueType::Vector | QcValueType::Float => GuestStorage::Float32,
            _ => GuestStorage::Int32,
        },
        count: if value_type == QcValueType::Vector { 3 } else { 1 },
    }
}

/// Describe a program/host pair for the execution profile.
pub fn describe_qc_host(
    program: &QcProgram,
    kind: QcHostKind,
    entities: &QcEntityLayout,
    registry: &QcBuiltinRegistry,
    extensions: &[String],
) -> Result<QuakeCHostProfile, GuestError> {
    if (kind == QcHostKind::Quakeworld) != program.api.is_quakeworld() {
        return Err(GuestError::bad_image(
            "progs.dat",
            "host kind disagrees with program API",
        ));
    }
    let globals_layout = GuestLayout::new(
        &format!("quakec:{}-globals", program.api.system_crc()),
        program.initial_globals.len(),
        4,
        4,
        program
            .globals
            .iter()
            .map(|definition| {
                field_layout(&definition.name, definition.offset, definition.value_type, 0)
            })
            .collect(),
    );
    let entity_variables_layout = GuestLayout::new(
        &format!("quakec:{}-entity-variables", program.api.system_crc()),
        entities.field_words * 4,
        4,
        4,
        program
            .fields
            .iter()
            .map(|definition| {
                field_layout(&definition.name, definition.offset, definition.value_type, 0)
            })
            .collect(),
    );
    let mut builtins: Vec<QcBuiltinBinding> = registry
        .numbered
        .keys()
        .map(|number| {
            let name = program
                .functions
                .iter()
                .find(|function| function.first_statement == -*number)
                .map(|function| function.name.clone())
                .unwrap_or_else(|| format!("builtin_{number}"));
            QcBuiltinBinding {
                kind: QcBuiltinKind::Numbered,
                number: *number,
                name,
                callback: CallbackId::new("quakec", &format!("numbered-{number}")),
            }
        })
        .collect();
    builtins.extend(registry.named.keys().map(|name| QcBuiltinBinding {
        kind: QcBuiltinKind::Named,
        number: 0,
        name: name.clone(),
        callback: CallbackId::new("quakec", &format!("named-{name}")),
    }));
    builtins.sort_by(|left, right| {
        (left.kind as u8, left.number, &left.name).cmp(&(right.kind as u8, right.number, &right.name))
    });
    Ok(QuakeCHostProfile {
        api: program.api,
        program_search_order: qc_program_search_order(kind)
            .iter()
            .map(|path| (*path).to_string())
            .collect(),
        globals_layout,
        entity_variables_layout,
        builtins,
        extensions: extensions.to_vec(),
    })
}

/// One proven source instruction (opcode plus three operands, stored as
/// `i32` so out-of-`u16` proofs compare unequal exactly like the donor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcProofStatement {
    /// Opcode discriminant.
    pub opcode: i32,
    /// First operand.
    pub a: i32,
    /// Second operand.
    pub b: i32,
    /// Third operand.
    pub c: i32,
}

/// Result predicate of a weapon-stage repeat region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QcWeaponResult {
    /// Predicate word (a float global at word >= 28).
    pub word: usize,
    /// Accepted value (0 or 1).
    pub value: bool,
}

/// One repeat region of the weapon dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcWeaponRepeat {
    /// Owning function name.
    pub function: String,
    /// Region entry statement.
    pub entry: usize,
    /// Join statement.
    pub exit: usize,
    /// Result predicate.
    pub result: QcWeaponResult,
    /// Proven instructions (region plus join).
    pub statements: Vec<QcProofStatement>,
}

/// Weapon-stage declaration: dispatcher plus verified repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcWeaponStage {
    /// Dispatcher function name.
    pub dispatcher: String,
    /// Continuation function names.
    pub continuations: Vec<String>,
    /// Verified repeat regions.
    pub repeats: Vec<QcWeaponRepeat>,
}

/// A client call: bare function name or full source call.
#[derive(Debug, Clone, PartialEq)]
pub enum QcClientCall {
    /// Bare function name.
    Function(String),
    /// Full source call.
    Call(ModSourceCall),
}

/// Client objectives declaration.
#[derive(Debug, Clone, PartialEq)]
pub enum QcWeaponObjectives {
    /// No objectives call.
    None,
    /// Bare function name.
    Function(String),
    /// Full source call.
    Call(ModSourceCall),
}

/// Client half of the primary weapon stage.
#[derive(Debug, Clone, PartialEq)]
pub struct QcWeaponClient {
    /// Spawn call.
    pub spawn: QcClientCall,
    /// Select-spawn call.
    pub select_spawn: QcClientCall,
    /// Objectives call.
    pub objectives: QcWeaponObjectives,
}

/// Primary weapon-stage declaration (stage plus client calls).
#[derive(Debug, Clone, PartialEq)]
pub struct QcPrimaryWeaponStage {
    /// Dispatcher function name.
    pub dispatcher: String,
    /// Continuation function names.
    pub continuations: Vec<String>,
    /// Verified repeat regions.
    pub repeats: Vec<QcWeaponRepeat>,
    /// Client calls.
    pub client: QcWeaponClient,
}

/// Scalar source of a pickup count/dropped flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QcPickupScalar {
    /// Entity float field.
    Field {
        /// Field name.
    pub name: String,
    },
    /// Global word.
    Global {
        /// Word offset.
    pub word: usize,
    },
}

/// Protection channel of a pickup resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QcProtectionChannel {
    /// Regular armor.
    Regular,
    /// Powered armor.
    Powered,
}

/// Resource granted by a pickup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QcPickupResource {
    /// Protection grant.
    Protection {
        /// Channel.
    pub channel: QcProtectionChannel,
    },
    /// Inventory grant.
    Inventory {
        /// Item id (`namespace:name`).
    pub item: String,
    },
}

/// One pickup item binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QcPickupItem {
    /// Item id.
    pub item: String,
    /// Granted resource, if any.
    pub resource: Option<QcPickupResource>,
    /// Count scalar, if any.
    pub count: Option<QcPickupScalar>,
}

/// One pickup item binding with its discriminator value.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupItemValue<T> {
    /// Item id.
    pub item: String,
    /// Granted resource, if any.
    pub resource: Option<QcPickupResource>,
    /// Count scalar, if any.
    pub count: Option<QcPickupScalar>,
    /// Discriminator value.
    pub value: T,
}

/// Item descriptor of a pickup caller.
#[derive(Debug, Clone, PartialEq)]
pub enum QcPickupDescriptor {
    /// Single constant item.
    Constant(QcPickupItem),
    /// String-field discriminator.
    String {
        /// Discriminator field.
    pub field: String,
        /// Candidate values.
    pub values: Vec<QcPickupItemValue<String>>,
    },
    /// Float-field discriminator.
    Float {
        /// Discriminator field.
    pub field: String,
        /// Candidate values.
    pub values: Vec<QcPickupItemValue<f64>>,
    },
}

/// Operation of a pickup region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QcPickupOperation {
    /// Branch decision on a global predicate word.
    Decision {
        /// Predicate word.
    pub word: usize,
        /// Accepted value.
    pub accepted: f64,
    },
    /// Recipient admission boundary.
    Admission,
    /// Inventory/protection grant.
    Grant,
    /// Pickup consumption.
    Consume,
}

/// One proven pickup region.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupRegion {
    /// Region entry statement.
    pub entry: usize,
    /// Join statement.
    pub exit: usize,
    /// Proven instructions (region plus join).
    pub statements: Vec<QcProofStatement>,
    /// Region operation.
    pub operation: QcPickupOperation,
}

/// Declared touch caller for pickups.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupCallerDeclaration {
    /// Touch function name.
    pub function: String,
    /// Item descriptor.
    pub descriptor: QcPickupDescriptor,
    /// Dropped-flag scalar, if any.
    pub dropped: Option<QcPickupScalar>,
    /// Proven regions.
    pub regions: Vec<QcPickupRegion>,
}

/// Bound pickup stage: declaration verified against program bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupStage {
    /// Touch function index.
    pub function_index: usize,
    /// Verified descriptor.
    pub descriptor: QcPickupStageDescriptor,
    /// Dropped-flag scalar, if any.
    pub dropped: Option<QcPickupScalar>,
    /// Verified regions as replaceable inline regions.
    pub regions: Vec<QcPickupStageRegion>,
}

/// Verified pickup item descriptor.
#[derive(Debug, Clone, PartialEq)]
pub enum QcPickupStageDescriptor {
    /// Constant item (discriminator value 0).
    Constant {
        /// Item binding.
    pub item: QcPickupItem,
        /// Discriminator value (always 0).
    pub value: f64,
    },
    /// String-field discriminator.
    String {
        /// Discriminator field.
    pub field: String,
        /// Candidate values.
    pub values: Vec<QcPickupItemValue<String>>,
    },
    /// Float-field discriminator.
    Float {
        /// Discriminator field.
    pub field: String,
        /// Candidate values.
    pub values: Vec<QcPickupItemValue<f64>>,
    },
}

/// One verified pickup region.
#[derive(Debug, Clone, PartialEq)]
pub struct QcPickupStageRegion {
    /// Replaceable inline region.
    pub region: QcInlineRegion,
    /// Region operation.
    pub operation: QcPickupOperation,
}

fn pickup_fail(program: &QcProgram, reason: &str) -> GuestError {
    GuestError::bad_image(
        "progs.dat",
        format!("{}: Invalid declared pickup caller: {reason}", program.source),
    )
}

fn exact_source_float(value: f64) -> bool {
    (value as f32) as f64 == value
}

/// Bind declared pickup callers to the exact bytes of `program`: touch
/// functions, typed fields, instruction proofs, decision joins, and
/// branch-integrity of every region.
pub fn qc_declared_pickup_stages(
    program: &QcProgram,
    declarations: &[QcPickupCallerDeclaration],
) -> Result<Vec<QcPickupStage>, GuestError> {
    let mut callers = HashSet::new();
    let mut stages = Vec::with_capacity(declarations.len());
    for declaration in declarations {
        let function = program
            .function_named(&declaration.function)
            .map_err(|_| pickup_fail(program, "duplicate or non-touch source function"))?;
        if function.first_statement <= 0
            || function.named_builtin
            || !function.parameter_sizes.is_empty()
            || !callers.insert(function.index)
        {
            return Err(pickup_fail(program, "duplicate or non-touch source function"));
        }
        let first = function.first_statement as usize;
        let end = program.function_end(function);
        match &declaration.descriptor {
            QcPickupDescriptor::Constant(_) => {}
            QcPickupDescriptor::String { field, values } => {
                let is_string = program
                    .field_named(field)
                    .is_some_and(|definition| definition.value_type == QcValueType::String);
                if !is_string {
                    return Err(pickup_fail(program, &format!("source field {field} is not string")));
                }
                check_discriminators(program, values.iter().map(|entry| &entry.value))?;
            }
            QcPickupDescriptor::Float { field, values } => {
                let is_float = program
                    .field_named(field)
                    .is_some_and(|definition| definition.value_type == QcValueType::Float);
                if !is_float {
                    return Err(pickup_fail(program, &format!("source field {field} is not float")));
                }
                let discriminators: Vec<String> =
                    values.iter().map(|entry| format!("{:?}", entry.value)).collect();
                check_discriminators(program, discriminators.iter())?;
                if values.iter().any(|entry| !exact_source_float(entry.value)) {
                    return Err(pickup_fail(program, "item discriminator is not an exact source float"));
                }
            }
        }
        let mut regions = declaration.regions.clone();
        regions.sort_by_key(|region| region.entry);
        if !regions.iter().any(|region| {
            matches!(
                region.operation,
                QcPickupOperation::Decision { .. } | QcPickupOperation::Admission
            )
        }) {
            return Err(pickup_fail(program, "missing recipient admission boundary"));
        }
        let mut previous_end = first;
        for region in &regions {
            if region.entry < previous_end || region.exit <= region.entry || region.exit >= end {
                return Err(pickup_fail(program, "overlapping or out-of-function regions"));
            }
            previous_end = region.exit;
            if region.statements.len() != region.exit - region.entry + 1 {
                return Err(pickup_fail(program, "instruction proof must include the region and its join"));
            }
            for (offset, expected) in region.statements.iter().enumerate() {
                let actual = program.statements.get(region.entry + offset);
                let matches = actual.is_some_and(|statement| {
                    i32::from(statement.opcode as u16) == expected.opcode
                        && i32::from(statement.a) == expected.a
                        && i32::from(statement.b) == expected.b
                        && i32::from(statement.c) == expected.c
                });
                if !matches {
                    return Err(pickup_fail(program, "instruction proof differs from original artifact"));
                }
            }
            if let QcPickupOperation::Decision { word, accepted } = region.operation {
                if word * 4 >= program.initial_globals.len() {
                    return Err(pickup_fail(program, "global word is outside source memory"));
                }
                let join = program.statements.get(region.exit);
                let consumes = join.is_some_and(|statement| {
                    statement.a as usize == word
                        && (statement.opcode == QcOpcode::If || statement.opcode == QcOpcode::IfNot)
                });
                if !consumes {
                    return Err(pickup_fail(
                        program,
                        "decision join does not consume the declared source predicate",
                    ));
                }
                if !exact_source_float(accepted) {
                    return Err(pickup_fail(program, "accepted decision is not an exact source float"));
                }
            }
            check_region_branches(program, function.first_statement, end, region)?;
        }
        let check_scalar = |scalar: &QcPickupScalar| -> Result<QcPickupScalar, GuestError> {
            match scalar {
                QcPickupScalar::Field { name } => {
                    let is_float = program
                        .field_named(name)
                        .is_some_and(|definition| definition.value_type == QcValueType::Float);
                    if !is_float {
                        return Err(pickup_fail(program, &format!("source field {name} is not float")));
                    }
                }
                QcPickupScalar::Global { word } => {
                    if word * 4 >= program.initial_globals.len() {
                        return Err(pickup_fail(program, "global word is outside source memory"));
                    }
                }
            }
            Ok(scalar.clone())
        };
        let check_item = |item: &QcPickupItem| -> Result<QcPickupItem, GuestError> {
            Ok(QcPickupItem {
                item: item.item.clone(),
                resource: item.resource.clone(),
                count: item.count.as_ref().map(check_scalar).transpose()?,
            })
        };
        let descriptor = match &declaration.descriptor {
            QcPickupDescriptor::Constant(item) => QcPickupStageDescriptor::Constant {
                item: check_item(item)?,
                value: 0.0,
            },
            QcPickupDescriptor::String { field, values } => {
                let mut checked = Vec::with_capacity(values.len());
                for entry in values {
                    checked.push(QcPickupItemValue {
                        item: entry.item.clone(),
                        resource: entry.resource.clone(),
                        count: entry.count.as_ref().map(&check_scalar).transpose()?,
                        value: entry.value.clone(),
                    });
                }
                QcPickupStageDescriptor::String {
                    field: field.clone(),
                    values: checked,
                }
            }
            QcPickupDescriptor::Float { field, values } => {
                let mut checked = Vec::with_capacity(values.len());
                for entry in values {
                    checked.push(QcPickupItemValue {
                        item: entry.item.clone(),
                        resource: entry.resource.clone(),
                        count: entry.count.as_ref().map(&check_scalar).transpose()?,
                        value: entry.value,
                    });
                }
                QcPickupStageDescriptor::Float {
                    field: field.clone(),
                    values: checked,
                }
            }
        };
        stages.push(QcPickupStage {
            function_index: function.index,
            descriptor,
            dropped: declaration.dropped.as_ref().map(check_scalar).transpose()?,
            regions: regions
                .iter()
                .map(|region| QcPickupStageRegion {
                    region: QcInlineRegion {
                        function_index: function.index,
                        entry: region.entry,
                        exit: region.exit,
                        replaceable: true,
                        standalone: None,
                    },
                    operation: region.operation,
                })
                .collect(),
        });
    }
    Ok(stages)
}

fn check_discriminators<'a>(
    program: &QcProgram,
    values: impl Iterator<Item = &'a String>,
) -> Result<(), GuestError> {
    let values: Vec<&String> = values.collect();
    if values.is_empty() {
        return Err(pickup_fail(program, "empty or ambiguous source item descriptors"));
    }
    let distinct: HashSet<&String> = values.iter().copied().collect();
    if distinct.len() != values.len() {
        return Err(pickup_fail(program, "empty or ambiguous source item descriptors"));
    }
    Ok(())
}

fn check_region_branches(
    program: &QcProgram,
    first: i32,
    end: usize,
    region: &QcPickupRegion,
) -> Result<(), GuestError> {
    for at in first.max(0) as usize..end {
        let Some(statement) = program.statements.get(at) else {
            return Err(pickup_fail(program, "missing source instruction"));
        };
        let target = match statement.opcode {
            QcOpcode::Goto => Some(at as i64 + i64::from(super::program::signed_qc_branch(statement.a))),
            QcOpcode::If | QcOpcode::IfNot => {
                Some(at as i64 + i64::from(super::program::signed_qc_branch(statement.b)))
            }
            _ => None,
        };
        if let Some(target) = target {
            if (at < region.entry || at >= region.exit)
                && target > region.entry as i64
                && target < region.exit as i64
            {
                return Err(pickup_fail(
                    program,
                    "source branch enters the middle of a declared region",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::program::{test_program, QcDefinition, QcFunction, QcStatement, QcValueType};
    use std::collections::HashMap;

    fn fixture_program() -> QcProgram {
        // Touch function at 1: StoreF, If on word 30 (join), Done.
        let statements = vec![
            QcStatement { opcode: QcOpcode::Done, a: 0, b: 0, c: 0 },
            QcStatement { opcode: QcOpcode::StoreF, a: 4, b: 30, c: 0 },
            QcStatement { opcode: QcOpcode::If, a: 30, b: 1, c: 0 },
            QcStatement { opcode: QcOpcode::Done, a: 0, b: 0, c: 0 },
        ];
        let globals = vec![QcDefinition {
            value_type: QcValueType::Float,
            native_type: 2,
            save: false,
            offset: 30,
            name: "decide".to_string(),
        }];
        let fields = vec![QcDefinition {
            value_type: QcValueType::Float,
            native_type: 2,
            save: false,
            offset: 0,
            name: "health".to_string(),
        }];
        let functions = vec![
            QcFunction {
                index: 0, first_statement: 0, parameter_start: 0, local_words: 0,
                name: "<null>".to_string(), file: String::new(), parameter_sizes: vec![], named_builtin: false,
            },
            QcFunction {
                index: 1, first_statement: 1, parameter_start: 33, local_words: 0,
                name: "touch".to_string(), file: "test.qc".to_string(), parameter_sizes: vec![],
                named_builtin: false,
            },
        ];
        test_program(statements, globals, fields, functions, b"\0".to_vec(), vec![0; 36 * 4], 1)
    }

    #[test]
    fn search_order_and_layout_match_donor() {
        assert_eq!(qc_program_search_order(QcHostKind::Netquake), &["progs.dat"]);
        assert_eq!(qc_program_search_order(QcHostKind::Quakeworld), &["qwprogs.dat", "progs.dat"]);
        let program = fixture_program();
        let layout = classic_qc_entity_layout(&program);
        assert_eq!(layout.variables_offset_bytes, 96);
        assert_eq!(layout.stride_bytes, 100);
        assert_eq!(layout.field_words, 1);
    }

    #[test]
    fn describe_host_builds_layouts_and_bindings() {
        struct Store;
        impl super::QcProgramContent for Store {
            fn open(&self, _path: &str) -> Result<Option<QcOpenedProgram>, GuestError> {
                Ok(None)
            }
        }
        let program = fixture_program();
        let layout = classic_qc_entity_layout(&program);
        let mut registry = QcBuiltinRegistry::default();
        registry.numbered.insert(8, std::rc::Rc::new(|machine: &mut super::super::machine::QcMachine| {
            machine.return_float(0.0)
        }));
        registry.named.insert("ex_prompt".to_string(), std::rc::Rc::new(|machine: &mut super::super::machine::QcMachine| {
            machine.return_float(0.0)
        }));
        let profile = describe_qc_host(&program, QcHostKind::Netquake, &layout, &registry, &["DP_TEST".to_string()]).unwrap();
        assert_eq!(profile.api, QuakeCApi::Netquake);
        assert_eq!(profile.globals_layout.fields.len(), 1);
        assert_eq!(profile.entity_variables_layout.byte_length, 4);
        assert_eq!(profile.builtins.len(), 2);
        assert!(describe_qc_host(&program, QcHostKind::Quakeworld, &layout, &registry, &[]).is_err());
        let missing = load_mounted_qc_program(&Store, QcHostKind::Netquake).unwrap_err();
        assert!(missing.to_string().contains("progs.dat"));
    }

    #[test]
    fn load_mounted_searches_in_order() {
        struct Store {
            files: HashMap<String, Vec<u8>>,
        }
        impl super::QcProgramContent for Store {
            fn open(&self, path: &str) -> Result<Option<QcOpenedProgram>, GuestError> {
                Ok(self.files.get(path).map(|bytes| QcOpenedProgram {
                    bytes: bytes.clone(),
                    reference: format!("test:{path}"),
                }))
            }
        }
        // Minimal valid NetQuake image: 1 statement, 28-word globals.
        let mut image = Vec::new();
        let push_i32 = |image: &mut Vec<u8>, value: i32| image.extend_from_slice(&value.to_le_bytes());
        let statement_blob = vec![0u8; 8];
        let strings = b"\0f\0g\0".to_vec();
        let blobs: Vec<Vec<u8>> = vec![
            statement_blob,
            vec![2, 0, 27, 0, 3, 0, 0, 0],
            vec![2, 0, 0, 0, 3, 0, 0, 0],
            vec![
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0,
            ],
            strings.clone(),
            vec![0u8; 28 * 4],
        ];
        let counts = [1i32, 1, 1, 1, strings.len() as i32, 28];
        let mut offset = 56i32;
        push_i32(&mut image, 6);
        push_i32(&mut image, 5927);
        for (blob, count) in blobs.iter().zip(counts) {
            push_i32(&mut image, offset);
            push_i32(&mut image, count);
            offset += blob.len() as i32;
        }
        push_i32(&mut image, 1);
        for blob in &blobs {
            image.extend_from_slice(blob);
        }
        let mut files = HashMap::new();
        files.insert("progs.dat".to_string(), image);
        let loaded = load_mounted_qc_program(&Store { files }, QcHostKind::Netquake).unwrap();
        assert_eq!(loaded.reference, "test:progs.dat");
        assert_eq!(loaded.program.api, QuakeCApi::Netquake);
    }

    fn caller_fixture() -> QcPickupCallerDeclaration {
        QcPickupCallerDeclaration {
            function: "touch".to_string(),
            descriptor: QcPickupDescriptor::Constant(QcPickupItem {
                item: "q1:item_health".to_string(),
                resource: Some(QcPickupResource::Inventory { item: "q1:item_health".to_string() }),
                count: None,
            }),
            dropped: None,
            regions: vec![QcPickupRegion {
                entry: 1,
                exit: 2,
                statements: vec![
                    QcProofStatement { opcode: i32::from(QcOpcode::StoreF as u16), a: 4, b: 30, c: 0 },
                    QcProofStatement { opcode: i32::from(QcOpcode::If as u16), a: 30, b: 1, c: 0 },
                ],
                operation: QcPickupOperation::Decision { word: 30, accepted: 1.0 },
            }],
        }
    }

    #[test]
    fn pickup_stages_bind_to_program_bytes() {
        let program = fixture_program();
        let stages = qc_declared_pickup_stages(&program, &[caller_fixture()]).unwrap();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].function_index, 1);
        assert!(stages[0].regions[0].region.replaceable);
        assert!(matches!(stages[0].descriptor, QcPickupStageDescriptor::Constant { value, .. } if value == 0.0));
    }

    #[test]
    fn pickup_stages_reject_bad_proofs() {
        let program = fixture_program();
        let mut bad = caller_fixture();
        bad.regions[0].statements[0].a = 5;
        assert!(qc_declared_pickup_stages(&program, &[bad]).is_err());
        let mut missing_join = caller_fixture();
        missing_join.regions[0].exit = 3;
        assert!(qc_declared_pickup_stages(&program, &[missing_join]).is_err());
        let mut unknown = caller_fixture();
        unknown.function = "nope".to_string();
        assert!(qc_declared_pickup_stages(&program, &[unknown]).is_err());
        let dupe = vec![caller_fixture(), caller_fixture()];
        assert!(qc_declared_pickup_stages(&program, &dupe).is_err());
    }

    #[test]
    fn pickup_float_discriminators_must_be_exact() {
        let program = fixture_program();
        let mut caller = caller_fixture();
        caller.descriptor = QcPickupDescriptor::Float {
            field: "health".to_string(),
            values: vec![QcPickupItemValue {
                item: "q1:item_health".to_string(),
                resource: None,
                count: None,
                value: 0.1,
            }],
        };
        assert!(qc_declared_pickup_stages(&program, &[caller]).is_err());
        let mut caller = caller_fixture();
        caller.descriptor = QcPickupDescriptor::String {
            field: "health".to_string(),
            values: vec![QcPickupItemValue {
                item: "q1:item_health".to_string(),
                resource: None,
                count: None,
                value: "x".to_string(),
            }],
        };
        assert!(qc_declared_pickup_stages(&program, &[caller]).is_err());
    }
}
