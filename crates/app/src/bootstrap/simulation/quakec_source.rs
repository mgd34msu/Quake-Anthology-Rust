//! QuakeC source preparation and synchronous source runtime.
//!
//! Provenance: `src/app/bootstrap/simulation/quakec-source.ts`
//! (donor `prepareQuakeCSource`, `prepareQuakeCResources`,
//! `preparedQuakeCDamageScaling`, `preparedQuakeCWeaponStage`, `QuakeCSource`).
//!
//! This file owns the canonical [`PreparedQuakeCSource`] (re-exported
//! through `super::types`, which holds no separate placeholder). The donor takes a
//! contracts execution plus a contracts callback declaration; the worktree
//! splits declarations into persisted and runtime shapes, so the combat
//! declaration here is the runtime guest shape and contract conversions
//! happen at the id1 boundary.

use std::collections::HashMap;

use qa_bots::entities::parse_entities;
use qa_client::audio::wav::decode_quake_wav;
use qa_content::bsp::{read_q1_bsp, Q1BspOptions};
use qa_content::contract::QuakeCApiIdentity;
use qa_content::contract::ResolvedResourceReference;
use qa_content::mdl::parse_mdl;
use qa_content::mounts::MountedContent;
use qa_content::mounts::ResourceRef;
use qa_content::q1::quakec::id1_program::id1_damage_multiplier;
use qa_content::q1::quakec::id1_program::id1_program_binding;
use qa_content::q1::quakec::id1_program::Id1Attribution;
use qa_content::q1::quakec::id1_program::Id1ProgramCache;
use qa_content::q1::quakec::pickup_callers::qc_declared_pickup_stages;
use qa_content::q1::quakec::pickup_callers::read_qc_pickup_caller;
use qa_content::q1::quakec::pickup_stage::QcPickupStage;
use qa_content::q1::quakec::qc_view::QcApiKind as ContentApi;
use qa_content::q1::quakec::qc_view::QcDefinitionView;
use qa_content::q1::quakec::qc_view::QcFunctionView as ContentFunction;
use qa_content::q1::quakec::qc_view::QcOpcode as ContentOpcode;
use qa_content::q1::quakec::qc_view::QcProgramView as ContentView;
use qa_content::q1::quakec::qc_view::QcStatementView;
use qa_content::q1::quakec::qc_view::QcValueType as ContentValue;
use qa_content::q1::quakec::weapon_stage::qc_weapon_stage;
use qa_content::q1::quakec::weapon_stage::QcWeaponStage;
use qa_content::q1::quakec::weapon_stage_declaration::read_qc_primary_weapon_stage;
use qa_content::q1::quakec::weapon_stage_declaration::QcPrimaryWeaponStageDeclaration;
use qa_content::q1::quakec::QcError;
use qa_content::spr::parse_spr;
use qa_content::value::parse_save_json;
use qa_content::value::SaveReader as ContentReader;
use qa_core::math::Bounds;
use qa_guest::error::GuestError;
use qa_guest::qc::compatibility::read_quake_c_compatibility;
use qa_guest::qc::compatibility::QcArmorFlags;
use qa_guest::qc::compatibility::QcCombatDeclaration;
use qa_guest::qc::compatibility::QcDamageScaleKind;
use qa_guest::qc::compatibility::SourceTeamAlias;
use qa_guest::qc::mod_combat::validate_qc_mod_combat;
use qa_guest::qc::mod_provider::ModCombatDeclaration;
use qa_guest::qc::mod_provider::ModQcArmorStage;
use qa_guest::qc::mod_provider::ModQcArmorStageFlags;
use qa_guest::qc::mod_provider::ModQcDamageScale;
use qa_guest::qc::mod_provider::ModQcDamageScaleKind;
use qa_guest::qc::mod_provider::ModQcEmptyArmor;
use qa_guest::qc::mod_provider::ModQcRegularScale;
use qa_guest::qc::mod_provider::QcApiKind as GuestApiKind;
use qa_guest::qc::mod_provider::QcFunctionView as GuestFunction;
use qa_guest::qc::mod_provider::QcProgramView as GuestView;
use qa_guest::qc::mod_provider::QcStatement as RuntimeStatement;
use qa_guest::qc::mod_provider::QcValueType as GuestValue;
use qa_guest::qc::profile::QcProofStatement;
use qa_guest::qc::program::load_qc_program;
use qa_guest::qc::program::qc_byte_string;
use qa_guest::qc::program::QcOpcode as GuestOpcode;
use qa_guest::qc::program::QcProgram;
use qa_guest::qc::program::QcValueType as ProgramValue;
use qa_guest::qc::source_call::ModCallbackInput as CallInput;
use qa_guest::qc::source_call::ModCallbackValue as CallValue;
use qa_guest::qc::source_call::ModSourceCall as CallSite;
use qa_net::q1_net::RereleaseMessages;

/// QuakeC source failure.
#[derive(Debug, thiserror::Error)]
pub enum QuakeCSourceError {
    /// Guest failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Mount failure.
    #[error(transparent)]
    Mount(#[from] qa_content::mounts::MountError),
    /// Entity-text failure.
    #[error(transparent)]
    Entities(#[from] qa_bots::entities::EntityError),

    /// Binary decode failure.
    #[error(transparent)]
    Binary(#[from] qa_core::binary::BinaryError),
    /// Binding failure.
    #[error(transparent)]
    Bindings(#[from] QcError),
    /// Checkpoint value failure.
    #[error(transparent)]
    CheckpointValue(#[from] qa_content::q1::Q1Error),
    /// Saved message decode failure.
    #[error(transparent)]
    SavedMessages(#[from] qa_net::q1_net::Q1NetError),
    /// Engine cvar failure.
    #[error(transparent)]
    EngineCvar(#[from] qa_core::cvar::CvarError),
    /// Local message restore failure.
    #[error(transparent)]
    LocalMessages(#[from] QuakeCLocalMessageError),
    /// Selected artifact API differs from the execution.
    #[error("Shared QuakeC artifact API differs from the selected execution")]
    ExecutionMismatch,
    /// Private messages selected for QuakeWorld.
    #[error("Private NetQuake messages cannot be selected for QuakeWorld")]
    QuakeWorldPrivate,
    /// BSP without a world model.
    #[error("No world model in {0}")]
    NoWorldModel(String),
    /// Synchronous source runtime failure (carries the donor `throw` text).
    #[error("{0}")]
    Invalid(String),
}

impl From<ValueError> for QuakeCSourceError {
    fn from(error: ValueError) -> Self {
        Self::Invalid(error.to_string())
    }
}

/// Mirror of `QuakeCExecution` from donor `ExecutableRecipe["execution"]`
/// (`src/contracts/content.ts`: `{ kind: "quakec", owner, role: "server-game", artifact, api }`)
/// (canonical home: execution-contracts lane); unify post-merge.
#[derive(Debug, Clone)]
pub struct QuakeCExecution {
    /// Execution owner.
    pub owner: QuakeCExecutionOwner,
    /// Program artifact.
    pub artifact: ResolvedResourceReference,
    /// Selected API identity.
    pub api: QuakeCApiIdentity,
}

/// Mirror of the donor execution `owner: ProviderReference`
/// (`src/contracts/content.ts`: `{ provider, content }`)
/// (canonical home: execution-contracts lane); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeCExecutionOwner {
    /// Owning provider.
    pub provider: qa_core::identity::ProviderId,
    /// Owning content.
    pub content: String,
}

/// Prepared source resource.
#[derive(Debug, Clone)]
pub struct QuakeCSourceResource {
    /// Resolved asset.
    pub resource: ResolvedResourceReference,
    /// Decoded model bounds, if a model.
    pub model_bounds: Option<Bounds>,
}

/// Prepared QuakeC source: the canonical home of donor
/// `PreparedQuakeCSource` (re-exported through `super::types`; no separate
/// placeholder remains).
#[derive(Debug, Clone)]
pub struct PreparedQuakeCSource {
    /// Team aliases, if declared.
    pub teams: Option<Vec<SourceTeamAlias>>,
    /// Combat declaration, if declared.
    pub combat_declaration: Option<QcCombatDeclaration>,
    /// Message dialect.
    pub message_dialect: RereleaseMessages,
    /// Weapon-stage declaration, if declared.
    pub weapon_declaration: Option<QcPrimaryWeaponStageDeclaration>,
    /// Resolved weapon stage.
    pub weapon_stage: Option<QcWeaponStage>,
    /// Declared pickup stages.
    pub declared_pickups: Vec<QcPickupStage>,
    /// Selected execution.
    pub execution: QuakeCExecution,
    /// Loaded program.
    pub program: QcProgram,
    /// Prepared resources by asset name.
    pub resources: HashMap<String, QuakeCSourceResource>,
}

macro_rules! guest_opcodes {
    ($($variant:ident,)*) => {
        fn guest_opcode(opcode: GuestOpcode) -> ContentOpcode {
            match opcode {
                $(GuestOpcode::$variant => ContentOpcode::$variant,)*
            }
        }
    };
}

guest_opcodes! {
    Done, MulF, MulV, MulFV, MulVF, DivF, AddF, AddV, SubF, SubV,
    EqF, EqV, EqS, EqE, EqFn, NeF, NeV, NeS, NeE, NeFn,
    Le, Ge, Lt, Gt, LoadF, LoadV, LoadS, LoadEnt, LoadFld, LoadFn,
    Address, StoreF, StoreV, StoreS, StoreEnt, StoreFld, StoreFn,
    StorePF, StorePV, StorePS, StorePEnt, StorePFld, StorePFn,
    Return, NotF, NotV, NotS, NotEnt, NotFn, If, IfNot,
    Call0, Call1, Call2, Call3, Call4, Call5, Call6, Call7, Call8,
    State, Goto, And, Or, BitAnd, BitOr,
}

fn content_value(value: ProgramValue) -> ContentValue {
    match value {
        ProgramValue::Void => ContentValue::Void,
        ProgramValue::String => ContentValue::Str,
        ProgramValue::Float => ContentValue::Float,
        ProgramValue::Vector => ContentValue::Vector,
        ProgramValue::Entity => ContentValue::Entity,
        ProgramValue::Field => ContentValue::Field,
        ProgramValue::Function => ContentValue::Function,
        ProgramValue::Pointer | ProgramValue::Opaque => ContentValue::Void,
    }
}

fn guest_value(value: ProgramValue) -> GuestValue {
    match value {
        ProgramValue::Void => GuestValue::Void,
        ProgramValue::String => GuestValue::String,
        ProgramValue::Float => GuestValue::Float,
        ProgramValue::Vector => GuestValue::Vector,
        ProgramValue::Entity => GuestValue::Entity,
        ProgramValue::Field => GuestValue::Field,
        ProgramValue::Function => GuestValue::Function,
        ProgramValue::Pointer => GuestValue::Pointer,
        ProgramValue::Opaque => GuestValue::Opaque,
    }
}

/// Owned content views backing a borrowed [`ContentView`].
#[derive(Debug, Default)]
pub(crate) struct ContentProgramViews {
    statements: Vec<QcStatementView>,
    globals: Vec<QcDefinitionView>,
    fields: Vec<QcDefinitionView>,
    functions: Vec<ContentFunction>,
    digest: String,
}

impl ContentProgramViews {
    pub(crate) fn build(program: &QcProgram) -> Self {
        let statements = program
            .statements
            .iter()
            .map(|statement| QcStatementView {
                opcode: guest_opcode(statement.opcode),
                a: statement.a,
                b: statement.b,
                c: statement.c,
            })
            .collect();
        let definitions = |definitions: &[qa_guest::qc::program::QcDefinition]| {
            definitions
                .iter()
                .map(|definition| QcDefinitionView {
                    def_type: content_value(definition.value_type),
                    offset: definition.offset,
                    name: definition.name.clone(),
                })
                .collect()
        };
        Self {
            statements,
            globals: definitions(&program.globals),
            fields: definitions(&program.fields),
            functions: program
                .functions
                .iter()
                .map(|function| ContentFunction {
                    index: function.index,
                    first_statement: function.first_statement,
                    parameter_start: function.parameter_start,
                    local_words: function.local_words,
                    name: function.name.clone(),
                    parameter_sizes: function.parameter_sizes.iter().map(|size| usize::from(*size)).collect(),
                    named_builtin: function.named_builtin,
                })
                .collect(),
            digest: format!("{}:{}", program.digest.algorithm, program.digest.value),
        }
    }

    pub(crate) fn view<'v>(&'v self, program: &'v QcProgram) -> ContentView<'v> {
        let api = match program.api {
            qa_guest::qc::program::QuakeCApi::Netquake => ContentApi::Netquake,
            qa_guest::qc::program::QuakeCApi::Quakeworld => ContentApi::Quakeworld,
        };
        ContentView::new(
            &program.source,
            api,
            &self.statements,
            &self.globals,
            &self.fields,
            &self.functions,
            &program.initial_globals,
            &self.digest,
        )
    }

    /// View over separately owned backing stores (the leaked runtime view core keeps the
    /// program text, initial globals, and digest alive apart from the tables).
    pub(crate) fn view_split<'v>(
        &'v self,
        source: &'v str,
        api: ContentApi,
        initial_globals: &'v [u8],
        digest: &'v str,
    ) -> ContentView<'v> {
        ContentView::new(
            source,
            api,
            &self.statements,
            &self.globals,
            &self.fields,
            &self.functions,
            initial_globals,
            digest,
        )
    }
}

/// Guest validation view over a loaded program. Shared with
/// `super::quakec_mod`, which validates mod declarations the same way.
pub(crate) struct GuestProgramView<'p> {
    program: &'p QcProgram,
    digest: String,
}

impl<'p> GuestProgramView<'p> {
    pub(crate) fn new(program: &'p QcProgram) -> Self {
        Self {
            program,
            digest: format!("{}:{}", program.digest.algorithm, program.digest.value),
        }
    }

    fn function_view(function: &qa_guest::qc::program::QcFunction) -> GuestFunction {
        GuestFunction {
            index: function.index as i32,
            name: function.name.clone(),
            first_statement: function.first_statement,
            parameter_start: function.parameter_start as i32,
            parameter_sizes: function.parameter_sizes.iter().map(|size| i32::from(*size)).collect(),
            named_builtin: function.named_builtin,
        }
    }
}

impl GuestView for GuestProgramView<'_> {
    fn digest(&self) -> &str {
        &self.digest
    }

    fn api_kind(&self) -> GuestApiKind {
        match self.program.api {
            qa_guest::qc::program::QuakeCApi::Netquake => GuestApiKind::Q1Netquake,
            qa_guest::qc::program::QuakeCApi::Quakeworld => GuestApiKind::Q1Quakeworld,
        }
    }

    fn field_type(&self, name: &str) -> Option<GuestValue> {
        self.program
            .field_named(name)
            .map(|definition| guest_value(definition.value_type))
    }

    fn global_type(&self, name: &str) -> Option<GuestValue> {
        self.program
            .global_named(name)
            .map(|definition| guest_value(definition.value_type))
    }

    fn function_named(&self, name: &str) -> Option<GuestFunction> {
        self.program.function_named(name).ok().map(Self::function_view)
    }

    fn function_at(&self, index: i32) -> Option<GuestFunction> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.program.function_at(index).ok())
            .map(Self::function_view)
    }

    fn functions(&self) -> Vec<GuestFunction> {
        self.program.functions.iter().map(Self::function_view).collect()
    }
}

fn guest_input(input: CallInput) -> qa_guest::qc::mod_provider::ModCallbackInput {
    use qa_guest::qc::mod_provider::ModCallbackInput as Target;
    match input {
        CallInput::ViewAngles => Target::ViewAngles,
        CallInput::Attack => Target::Attack,
        CallInput::Jump => Target::Jump,
        CallInput::Impulse => Target::Impulse,
        CallInput::ForwardMove => Target::ForwardMove,
        CallInput::SideMove => Target::SideMove,
        CallInput::UpMove => Target::UpMove,
        CallInput::Self_ => Target::Self_,
        CallInput::Other => Target::Other,
        CallInput::Activator => Target::Activator,
        CallInput::Attacker => Target::Attacker,
        CallInput::Inflictor => Target::Inflictor,
        CallInput::Amount => Target::Amount,
        CallInput::DamageFlags => Target::DamageFlags,
        CallInput::RegularProtectionScale => Target::RegularProtectionScale,
        CallInput::Knockback => Target::Knockback,
        CallInput::Point => Target::Point,
        CallInput::Direction => Target::Direction,
        CallInput::Normal => Target::Normal,
        CallInput::Item => Target::Item,
        CallInput::Time => Target::Time,
        CallInput::Elapsed => Target::Elapsed,
        CallInput::Result => Target::Result,
        CallInput::PickupCount => Target::PickupCount,
        CallInput::PickupHasCount => Target::PickupHasCount,
        CallInput::PickupDropped => Target::PickupDropped,
    }
}

fn guest_value_of(value: &CallValue) -> qa_guest::qc::mod_provider::ModCallbackValue {
    use qa_guest::qc::mod_provider::ModCallbackValue as Target;
    match value {
        CallValue::Input(input) => Target::Input(guest_input(*input)),
        CallValue::Float(value) => Target::Float(*value),
        CallValue::String(text) => Target::String(text.clone()),
        CallValue::Vector(vector) => Target::Vector(*vector),
    }
}

fn guest_call(call: &CallSite) -> qa_guest::qc::mod_provider::ModSourceCall {
    use qa_guest::qc::mod_provider::ModSourceCall as Target;
    use qa_guest::qc::mod_provider::ModSourceGlobal as Global;
    Target {
        function: call.function.clone(),
        arguments: call.arguments.iter().map(guest_value_of).collect(),
        globals: call
            .globals
            .iter()
            .map(|global| Global {
                name: global.name.clone(),
                value: guest_value_of(&global.value),
            })
            .collect(),
    }
}

fn offset_word(value: usize) -> Result<i32, GuestError> {
    i32::try_from(value).map_err(|_| GuestError::invalid("combat stage offset exceeds i32"))
}

fn flag_word(value: i64) -> Result<i32, GuestError> {
    i32::try_from(value).map_err(|_| GuestError::invalid("combat stage flag exceeds i32"))
}

fn offset_u32(value: usize) -> Result<u32, GuestError> {
    u32::try_from(value).map_err(|_| GuestError::invalid("combat stage offset exceeds u32"))
}

fn flag_u32(value: i64) -> Result<u32, GuestError> {
    u32::try_from(value).map_err(|_| GuestError::invalid("combat stage flag exceeds u32"))
}

fn runtime_statement(statement: &QcProofStatement) -> RuntimeStatement {
    RuntimeStatement {
        opcode: statement.opcode,
        a: statement.a,
        b: statement.b,
        c: statement.c,
    }
}

/// Project a compatibility combat declaration onto the runtime validation
/// shape. Unspecified damage-scale kinds default to the multiplier stage.
pub(crate) fn runtime_combat(combat: &QcCombatDeclaration) -> Result<ModCombatDeclaration, GuestError> {
    let damage_scale = combat
        .damage_scale
        .as_ref()
        .map(|stage| -> Result<ModQcDamageScale, GuestError> {
            let kind = stage.kind.map_or(ModQcDamageScaleKind::Multiplier, |kind| match kind {
                QcDamageScaleKind::Multiplier => ModQcDamageScaleKind::Multiplier,
                QcDamageScaleKind::Identity => ModQcDamageScaleKind::Identity,
                QcDamageScaleKind::Transform => ModQcDamageScaleKind::Transform,
            });
            Ok(ModQcDamageScale {
                kind,
                function: stage.function.clone(),
                entry: offset_word(stage.entry)?,
                exit: offset_word(stage.exit)?,
                damage: offset_word(stage.damage)?,
                statements: stage.statements.iter().map(runtime_statement).collect(),
            })
        })
        .transpose()?;
    let armor_stage = combat
        .armor_stage
        .as_ref()
        .map(|stage| -> Result<ModQcArmorStage, GuestError> {
            let mut regular_scale = Vec::new();
            for site in &stage.regular_scale {
                regular_scale.push(ModQcRegularScale {
                    caller: site.caller.clone(),
                    statement: offset_word(site.statement)?,
                    scale: site.scale,
                });
            }
            let flags = match stage.flags {
                QcArmorFlags::None => ModQcArmorStageFlags::None,
                QcArmorFlags::Bits {
                    word,
                    no_armor,
                    no_power_armor,
                    no_regular_armor,
                    energy,
                } => ModQcArmorStageFlags::Bits {
                    word: offset_word(word)?,
                    no_armor: flag_word(no_armor)?,
                    no_power_armor: flag_word(no_power_armor)?,
                    no_regular_armor: flag_word(no_regular_armor)?,
                    energy: flag_word(energy)?,
                },
            };
            Ok(ModQcArmorStage {
                function: stage.function.clone(),
                entry: offset_word(stage.entry)?,
                exit: offset_word(stage.exit)?,
                target: offset_word(stage.target)?,
                damage: offset_word(stage.damage)?,
                saved: offset_word(stage.saved)?,
                regular_scale,
                flags,
                statements: stage.statements.iter().map(runtime_statement).collect(),
            })
        })
        .transpose()?;
    Ok(ModCombatDeclaration {
        damage: guest_call(&combat.damage),
        damage_scale,
        armor_stage,
        empty_armor: combat.empty_armor.as_ref().map(|armor| ModQcEmptyArmor {
            item: armor.item.clone(),
            absorption: armor.absorption,
        }),
    })
}

/// Whether the prepared source scales damage: a declared scale stage or a
/// pinned original program.
pub fn prepared_quake_c_damage_scaling(prepared: &PreparedQuakeCSource) -> Result<bool, QcError> {
    if prepared
        .combat_declaration
        .as_ref()
        .is_some_and(|combat| combat.damage_scale.is_some())
    {
        return Ok(true);
    }
    let views = ContentProgramViews::build(&prepared.program);
    let view = views.view(&prepared.program);
    let mut cache = Id1ProgramCache::default();
    let binding = id1_program_binding(&mut cache, &view, None)?;
    Ok(binding.attribution == Id1Attribution::Pinned)
}

/// Resolve the prepared weapon stage, computing it when absent. The
/// computation is deterministic, so resolving an explicit null is stable.
pub fn prepared_quake_c_weapon_stage(prepared: &PreparedQuakeCSource) -> Result<Option<QcWeaponStage>, QcError> {
    if let Some(stage) = &prepared.weapon_stage {
        return Ok(Some(stage.clone()));
    }
    let views = ContentProgramViews::build(&prepared.program);
    let view = views.view(&prepared.program);
    qc_weapon_stage(&view, prepared.weapon_declaration.as_ref())
}

fn api_matches(selected: QuakeCApiIdentity, program: qa_guest::qc::program::QuakeCApi) -> bool {
    matches!(
        (selected, program),
        (QuakeCApiIdentity::Netquake, qa_guest::qc::program::QuakeCApi::Netquake)
            | (
                QuakeCApiIdentity::Quakeworld,
                qa_guest::qc::program::QuakeCApi::Quakeworld
            )
    )
}

/// Decode the selected artifact and genuine assets before synchronous
/// source precaching.
pub fn prepare_quake_c_source(
    execution: &QuakeCExecution,
    mounts: &MountedContent,
    entity_text: &str,
    resource_mounts: &MountedContent,
) -> Result<PreparedQuakeCSource, QuakeCSourceError> {
    let bytes = mounts.read(ResourceRef::Resolved(&execution.artifact))?;
    let program = load_qc_program(&bytes, None, &execution.artifact.requested_path)?;
    if !api_matches(execution.api, program.api) {
        return Err(QuakeCSourceError::ExecutionMismatch);
    }
    let compatibility_file = mounts.open("quakec-compatibility.json", |_| true)?;
    let compatibility_bytes = compatibility_file.as_ref().map(|found| found.bytes.as_slice());
    let digest = format!("{}:{}", program.digest.algorithm, program.digest.value);
    let compatibility = read_quake_c_compatibility(compatibility_bytes, &digest)?;
    let views = ContentProgramViews::build(&program);
    let view = views.view(&program);
    if let Some(combat) = &compatibility.combat {
        validate_qc_mod_combat(&GuestProgramView::new(&program), &runtime_combat(combat)?)?;
    } else {
        let mut cache = Id1ProgramCache::default();
        id1_program_binding(&mut cache, &view, None)?;
    }
    // The guest compatibility read above owns teams/combat/dialect; the
    // weapon and pickup declarations are read with the content readers so the
    // binding shapes need no profile conversion.
    let document = compatibility_bytes
        .map(|bytes| {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                QcError::Value(qa_content::value::ValueError(
                    "quakec-compatibility.json: invalid UTF-8".to_string(),
                ))
            })?;
            parse_save_json(text).map_err(QcError::Value)
        })
        .transpose()?;
    let (weapon_declaration, declared_pickups, weapon_stage) = match &document {
        None => (None, Vec::new(), qc_weapon_stage(&view, None)?),
        Some(document) => {
            let root = ContentReader::new(document);
            let weapon = root.field("weaponStage");
            let declaration = if weapon.is_missing() {
                None
            } else {
                Some(read_qc_primary_weapon_stage(weapon)?)
            };
            let callers = root.field("pickupCallers");
            let callers = if callers.is_missing() {
                Vec::new()
            } else {
                callers.list(read_qc_pickup_caller)?
            };
            let declared = qc_declared_pickup_stages(&view, &callers)?;
            let stage = qc_weapon_stage(&view, declaration.as_ref())?;
            (declaration, declared, stage)
        }
    };
    let message_dialect = match compatibility.message_dialect {
        qa_guest::qc::compatibility::QcMessageDialect::KnownRetail => RereleaseMessages::KnownRetail,
        qa_guest::qc::compatibility::QcMessageDialect::ReTsPrivate => RereleaseMessages::Quake1ReTsPrivate,
    };
    if program.api.is_quakeworld() && message_dialect != RereleaseMessages::KnownRetail {
        return Err(QuakeCSourceError::QuakeWorldPrivate);
    }
    let resources = prepare_quake_c_resources(&program, resource_mounts, entity_text)?;
    Ok(PreparedQuakeCSource {
        teams: compatibility.teams,
        combat_declaration: compatibility.combat,
        message_dialect,
        weapon_declaration,
        weapon_stage,
        declared_pickups,
        execution: execution.clone(),
        program,
        resources,
    })
}

fn core_bounds(bounds: qa_content::common::Bounds) -> Bounds {
    Bounds {
        min: qa_core::math::Vec3 {
            x: bounds.min[0],
            y: bounds.min[1],
            z: bounds.min[2],
        },
        max: qa_core::math::Vec3 {
            x: bounds.max[0],
            y: bounds.max[1],
            z: bounds.max[2],
        },
    }
}

fn resource_name(name: &str) -> bool {
    name.ends_with(".mdl") || name.ends_with(".spr") || name.ends_with(".bsp") || name.ends_with(".wav")
}

/// Scan program strings plus entity text for genuine assets.
pub fn prepare_quake_c_resources(
    program: &QcProgram,
    mounts: &MountedContent,
    entity_text: &str,
) -> Result<HashMap<String, QuakeCSourceResource>, QuakeCSourceError> {
    let mut names: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut consider = |name: String| {
        if resource_name(&name) && seen.insert(name.clone()) {
            names.push(name);
        }
    };
    let mut offset = 0i64;
    while offset < program.strings.len() as i64 {
        let name = qc_byte_string(&program.strings, offset)?;
        offset += name.len() as i64 + 1;
        consider(name);
    }
    for entity in parse_entities(entity_text, "quakec-source")? {
        for name in entity.values() {
            consider(name.clone());
        }
    }
    let mut resources = HashMap::new();
    for name in names {
        let path = if name.ends_with(".wav") {
            format!("sound/{name}")
        } else {
            name.clone()
        };
        let Some(asset) = mounts.open(&path, |_| true)? else {
            continue;
        };
        let mut model_bounds = None;
        if name.ends_with(".mdl") {
            let _ = parse_mdl(&asset.bytes, &name)?;
            model_bounds = Some(Bounds {
                min: qa_core::math::Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -16.0,
                },
                max: qa_core::math::Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 16.0,
                },
            });
        } else if name.ends_with(".spr") {
            model_bounds = Some(core_bounds(parse_spr(&asset.bytes, &name)?.bounds));
        } else if name.ends_with(".bsp") {
            let model = read_q1_bsp(&asset.bytes, &name, Q1BspOptions::default())?
                .models
                .into_iter()
                .next()
                .ok_or_else(|| QuakeCSourceError::NoWorldModel(name.clone()))?;
            model_bounds = Some(core_bounds(model.bounds));
        } else {
            decode_quake_wav(&asset.bytes, &name)
                .map_err(|error| GuestError::invalid(format!("invalid wav {name}: {error}")))?;
        }
        resources.insert(
            name,
            QuakeCSourceResource {
                resource: asset.reference,
                model_bounds,
            },
        );
    }
    Ok(resources)
}

// ---------------------------------------------------------------------------
// Synchronous source runtime (`QuakeCSource`)
// ---------------------------------------------------------------------------

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use qa_content::contract::{
    ArmorState, ItemId, ModCallbackInput, ModRuntimeValue, OriginalPickupOffer, PoweredProtectionState,
    ProtectionChannel, RegularArmorState,
};
use qa_content::q1::composition::types::Q1CompositionEvent;
use qa_content::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use qa_content::q1::foundation::gameplay::{DamageOutcome, DamageRequest};
use qa_content::q1::foundation::types::{Q1Event, Q1Powerup};
use qa_content::q1::foundation::types::{Q1Weapon, WEAPONS};
use qa_content::q1::foundation::weapon_names::q1_weapon_display_name;
use qa_content::q1::quakec::armor_points::qc_empty_armor;
use qa_content::q1::quakec::id1_attacks::Id1SynchronousAttacks;
use qa_content::q1::quakec::id1_damage::{Id1DamageBinding, Id1DamageCall};
use qa_content::q1::quakec::id1_environment::{EnvCallbackKind, Id1Environment, Id1PhysicsCallback};
use qa_content::q1::quakec::id1_pickups::{Id1PickupBinding, QcPickupPolicy, QcPickupSupplyOffer};
use qa_content::q1::quakec::id1_projectiles::Id1ProjectileAttacks;
use qa_content::q1::quakec::qc_gameplay::{ActorSource, QcActorRegistry, QcActorSlots};
use qa_content::q1::quakec::qc_view::{
    with_qc_source_call, GameplayAuthority, MachineFn, QcHostSource, QcMachineView, QcWordsBuf,
};
use qa_content::q1::quakec::weapon_stage::{
    invoke_qc_client_stage, qc_client_stage_self, QcClientStageCall, QcWeaponObjectives, QcWeaponStageBinding,
};
use qa_content::q2::foundation::host::{Q2Motion, Q2MotionKind};
use qa_content::value::{arr, boolean, int, namespaced, num, obj, str, SaveJson, ValueError};
use qa_core::cmd::Dialect;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, ClientId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::donor_angle_vectors;
use qa_core::math::Vec3;
use qa_core::numeric::{native_atoi, NumericOps, Q1_DONOR_PROFILE};
use qa_core::time::ClockProfile;
use qa_core::time::{FrameContext, SourceTime};
use qa_guest::core::contracts::ModuleIdentity;
use qa_guest::fields::{FieldLayout, FieldTable, FieldValue};
use qa_guest::qc::actor_state::{
    BodyState, Motion as GuestMotion, MotionKind as GuestMotionKind, PhysicsFlagChanges, QcActorState,
    SharedPhysicsFlags as GuestPhysicsFlags, SharedSolid,
};
use qa_guest::qc::borrowed_actors::{BorrowedCheckpoint, BorrowedSlotPool, QcBorrowedActors, ReusePolicy};
use qa_guest::qc::builtins::{create_qc_builtins, QcBuiltinServices, QcHostBuiltinName, QcHostKind, QcSharedRandom};
use qa_guest::qc::client_host::{AimScene, ClientSlots, QcClientHost, VisibilityScene};
use qa_guest::qc::entity_host::{create_qc_source_slot_storage, SourceSlotStorage};
use qa_guest::qc::executor::{
    capture_qc_checkpoint, restore_qc_checkpoint, GuestPrivateState, QcExecutorHost, QcHostSavedState, QuakeCCheckpoint,
};
use qa_guest::qc::machine::{
    QcAccessKind, QcBoundaryAction, QcBuiltin, QcBuiltinRegistry, QcCallSite, QcEntityStoreObservation,
    QcFunctionBoundary as GuestFunctionBoundary, QcInlineAction, QcInlineBoundary as GuestInlineBoundary,
    QcInlineRegion, QcMachine, QcMachineOptions,
};
use qa_guest::qc::memory::QcEntityMemory;
use qa_guest::qc::message_effects::{QcBroadcastEffect, TempEntityEffect};
use qa_guest::qc::movement_host::{MonsterBodySnapshot, MovementBindings, MovementBodies, MovementWorld, RandomSource};
use qa_guest::qc::presentation_host::{
    capture_netquake_messages, capture_qc_destination, read_qc_destination, restore_netquake_messages, ApiKind,
    ClientMessage, NqMessage, PrecacheKind, QcBroadcastMessages, QcMessageCheckpoint, QcMessageDestination,
    QcMessageRouter, QcPrecachedResource, QcPresentationEvent, QcRoutedMessage, QwEntriesCheckpoint, QwEntryCheckpoint,
    RoutedCheckpoint, SavedDestination, VisibilityScope,
};
use qa_guest::qc::profile::classic_qc_entity_layout;
use qa_guest::qc::save::{
    apply_qc_entity_pairs, apply_qc_global_pairs, save_qc_entity_pairs, save_qc_global_pairs,
    QcTextPair as GuestTextPair,
};
use qa_guest::qc::world_host::QcWorldHost;
use qa_net::common::commands::UserCommand;
use qa_net::msg::MsgWriter;
use qa_net::q1_net::{write_net_quake_message, NetQuakeDecoder, NetQuakeMessage, NqText, TemporaryEntity};
use qa_net::q1_wide::NqProfile;
use qa_world::body::BodyState as WorldBodyState;
use qa_world::movement::q1::types::{Q1MovementState, QwMovementProfile, QwMovementState};
use qa_world::movement::q1::water_transition::q1_water_transition;
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, InventoryEntry as MovementInventoryEntry, Q1UserCommand,
    QwUserCommand, TraceHit, WeaponState,
};
use qa_world::movement::Q1MovementParameters;
use qa_world::scheduler::think_callback_time;

use super::actor_execution::QuakeCSource as ExecutionQuakeCSource;
use super::physics::{
    PhysicsFamily, SharedPhysicsFlags as ExecutionPhysicsFlags, SharedSolid as ExecutionSharedSolid,
    SolidKind as ExecutionSolidKind,
};
use super::powerup_timers::{q1_powerup_timers, ActivePowerupTimer};
use super::quakec_client_adapter::{
    consume_quake_c_jump, quake_c_client_command, quake_c_source_jump, QuakeCClientMovement, QuakeCTransitionState,
};
use super::quakec_local_messages::{
    present_quake_c_local_message, quake_c_local_view, NetworkEvent as LocalNetworkEvent, QuakeCLocalClientCapture,
    QuakeCLocalFog, QuakeCLocalMessageCapture, QuakeCLocalMessageError, QuakeCLocalMessageHost, QuakeCLocalMessages,
    QuakeCLocalViewSource, QuakeCSessionKind, QuakeCViewClient,
};
use super::quakec_player_ui::{quake_c_weapon_ui, QuakeCWeaponUi, QuakeCWeaponUiBinding};
use super::quakeworld_cvars::register_quake_world_engine_cvars;
use super::types::{
    CvarNameValue, PlayerView, Q1ClientMetadataEvent, QuakeCClientRole, QuakeCSourceClient, QuakeCSourceKind,
    QuakeCSourceTravel,
};
use crate::persistence::q1::quakec::{Q1AppliedEntity, Q1QuakeCMachine, Q1SaveHeader, Q1UnknownSaveFields};
use crate::persistence::q1::source::{capture_q1_source_save, restore_q1_source_save, Q1Source, Q1SourceStaging};
use crate::persistence::q1::source_text::{Q1SaveData, Q1SaveFormat, QcTextPair as SavedTextPair};

/// Synchronous source physics callback.
///
/// Donor `QuakeCPhysicsCallback` from `quakec-source.ts` (line 163), itself an alias of
/// `Id1PhysicsCallback` from `src/content/q1/quakec/id1-environment.ts`.
pub type QuakeCPhysicsCallback = Id1PhysicsCallback;

/// Source game mode.
///
/// Donor `QuakeCSourceOptions["mode"]` string union from `quakec-source.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCSourceMode {
    /// Singleplayer.
    Singleplayer,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// Slot definition kind.
///
/// Donor `${string}:${string}` slot definitions from `src/world/actors/source-slots.ts`
/// (`quakec:worldspawn`, `quakec:reserved-client`, `quakec:authored`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCSlotKind {
    /// Worldspawn slot.
    Worldspawn,
    /// Reserved client slot.
    ReservedClient,
    /// Authored map entity slot.
    Authored,
    /// Restored save-game edict slot.
    Edict,
}

impl QuakeCSlotKind {
    /// Donor definition string.
    #[must_use]
    pub fn definition(self) -> &'static str {
        match self {
            QuakeCSlotKind::Worldspawn => "quakec:worldspawn",
            QuakeCSlotKind::ReservedClient => "quakec:reserved-client",
            QuakeCSlotKind::Authored => "quakec:authored",
            QuakeCSlotKind::Edict => "quakec:edict",
        }
    }
}

/// Native weapon row.
///
/// Donor-local `NativeWeapon` interface from `quakec-source.ts` (line 88).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeWeapon {
    /// Display label.
    pub label: String,
    /// Weapon item.
    pub item: ItemId,
    /// Inventory bit.
    pub bit: i32,
    /// Selection impulse.
    pub impulse: i32,
    /// Intermediate weapon bit for two-stage selection (`via`).
    pub via: Option<i32>,
}

/// Native weapon roster for one program.
///
/// Donor-local `nativeWeapons` from `quakec-source.ts` (line 95), including the Hipnotic
/// artifact's extra laser/mjolnir/proximity rows keyed by program digest.
#[must_use]
pub fn native_weapons(program: &QcProgram) -> Vec<NativeWeapon> {
    let mut base: Vec<NativeWeapon> = WEAPONS
        .iter()
        .enumerate()
        .map(|(index, weapon)| {
            let resolved = Q1Weapon::from(*weapon);
            NativeWeapon {
                label: q1_weapon_display_name(resolved),
                item: qa_content::q1::foundation::types::weapon_item(resolved),
                bit: if index == 0 { 4096 } else { 1 << (index - 1) },
                impulse: index as i32 + 1,
                via: None,
            }
        })
        .collect();
    let digest = format!("{}:{}", program.digest.algorithm, program.digest.value);
    if digest != "sha256:35a2fdc3acb04bdafe8d0269f5327cd1d5b47971f1ef024429f1572d3201dc82" {
        return base;
    }
    // This artifact's W_ChangeWeapon uses 225/226 and toggles grenade/proximity with impulse 6.
    for (weapon, bit, impulse, via) in [
        (Q1Weapon::HipnoticLaser, 8388608, 225, None),
        (Q1Weapon::HipnoticMjolnir, 128, 226, None),
        (Q1Weapon::HipnoticProximity, 65536, 6, Some(16)),
    ] {
        base.push(NativeWeapon {
            label: q1_weapon_display_name(weapon),
            item: qa_content::q1::foundation::types::weapon_item(weapon),
            bit,
            impulse,
            via,
        });
    }
    base
}

/// Actor-release hook.
///
/// Donor `SessionActorRegistry.onRelease` callback from `src/world/actors/registry.ts`.
pub type ReleaseHook = Box<dyn FnMut(&OwnedActor)>;

/// Session actor registry plus source-slot surface.
///
/// Seam for the missing `SessionActorRegistry` / `SourceActorSlots` pair from donor
/// `src/world/actors/index.ts`: a host-provided value, injected as a trait and never duplicated.
/// Method shapes follow the donor registry (`atSource`, `allocateAtSource`, `release`, `isLive`,
/// `resolveOwned`, `referenceSaved`, `sourceOf`, `onRelease`).
pub trait QuakeCSourceActors {
    /// Actor bound at a source slot, if any (donor `atSource`).
    fn at_source(&self, provider: &ProviderId, slot: usize) -> Option<OwnedActor>;
    /// Allocate an actor at a source slot (donor `allocateAtSource`).
    fn allocate_at_source(&mut self, provider: &ProviderId, slot: usize, definition: &str) -> OwnedActor;
    /// Allocate an actor at a free source slot (donor `allocate`).
    fn allocate(&mut self, provider: &ProviderId, definition: &str) -> OwnedActor;
    /// Release an actor (donor `release`).
    fn release(&mut self, actor: &OwnedActor);
    /// Whether an actor is live (donor `isLive`).
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Resolve an owned actor (donor `resolveOwned`).
    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor>;
    /// Resolve a saved actor in the checkpoint domain (donor `referenceSaved`).
    fn reference_saved(&self, saved: SavedActorId) -> ActorId;
    /// Source provider and slot of an actor (donor `sourceOf`).
    fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, usize)>;
    /// Subscribe to actor release (donor `onRelease`).
    fn on_release(&mut self, hook: ReleaseHook);
}

/// Session physics surface.
///
/// Seam for the missing `SharedPhysics` value from donor
/// `src/app/bootstrap/simulation/physics.ts`: a host-provided value, injected as a trait and
/// never duplicated. Only the methods the source touches are exposed.
pub trait QuakeCSourcePhysics {
    /// Unlink a body (donor `physics.bodies.unlink`).
    fn unlink_body(&mut self, actor: &OwnedActor);
    /// Read a body (donor `physics.bodies.read`).
    fn read_body(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body (donor `physics.bodies.write`).
    fn write_body(&mut self, actor: &OwnedActor, body: BodyState);
    /// Bind a body (donor `physics.bodies.bind`).
    fn bind_body(&mut self, actor: &ActorId, slot: usize, binding: qa_guest::qc::actor_state::QcBodyBinding);
    /// Link a body (donor `physics.bodies.link`).
    fn link_body(&mut self, actor: &ActorId);
    /// Read a foreign pusher entity (donor `physics.readQ1Pusher`).
    fn read_q1_pusher(&self, actor: &ActorId) -> Option<qa_world::movement::q1::types::Q1PhysicsEntity>;
    /// Write a foreign pusher entity (donor `physics.writeQ1Pusher`).
    fn write_q1_pusher(&mut self, entity: &qa_world::movement::q1::types::Q1PhysicsEntity);
    /// Build host pusher services over the source projection
    /// (donor `physics.q1PusherServices(projection)`).
    fn q1_pusher_services(
        &mut self,
        projection: Rc<dyn QuakeCSourcePusherProjection>,
    ) -> Rc<RefCell<dyn qa_world::movement::q1::types::Q1PusherServices>>;
    /// Enable or disable collision for an actor (donor `physical.collisionEnabled`).
    fn set_collision_enabled(&mut self, actor: &OwnedActor, enabled: bool);
    /// Collision of an actor (donor `physics.solidOf`).
    fn solid_of(&self, actor: &ActorId) -> Option<SharedSolid>;
    /// Touch triggers for an actor (donor `physics.touchTriggers`).
    fn touch_triggers(&mut self, actor: &OwnedActor);
    /// Set world gravity (donor `physics.setWorldGravity`).
    fn set_world_gravity(&mut self, gravity: f32);
}

/// Session event sink.
///
/// Seam for the missing `SimulationEvents` value from donor
/// `src/app/bootstrap/simulation/events.ts`: a host-provided value, injected as a trait and
/// never duplicated.
pub trait QuakeCSourceEvents {
    /// Deliver a client message event (donor `events.message`).
    ///
    /// The actor is `None` for baseline (signon) presentation, matching the
    /// donor `null` recipient.
    fn message(&mut self, event: ClientMessage, actor: Option<&ActorId>);
    /// Emit a content event (donor `events.emit`).
    fn emit(&mut self, content: &str, event: QcPresentationEvent, recipient: Option<&ActorId>);
    /// Emit one decoded local-service signal (donor `events.emit` arms of
    /// `receiveLocalMessages` that carry no [`QcPresentationEvent`]).
    fn emit_local(&mut self, content: &str, event: QuakeCLocalSinkEvent, recipient: Option<&ActorId>);
    /// Light-style pattern by index, empty when unset (donor
    /// `events.lightStyle`).
    fn light_style(&self, index: usize) -> String;
}

/// Decoded local-service signal for [`QuakeCSourceEvents::emit_local`].
///
/// Each variant carries one donor `events.emit` call from
/// `receiveLocalMessages` in `quakec-source.ts` that is not a
/// [`QcPresentationEvent`].
#[derive(Debug, Clone, PartialEq)]
pub enum QuakeCLocalSinkEvent {
    /// Q1 presentation event (donor `{ kind: "q1", event }`).
    Q1(Q1Event),
    /// CD audio track (donor `{ kind: "music", event: { kind: "cd-track" } }`).
    Music {
        /// Track number.
        track: u8,
    },
    /// Client view-angle reset (donor `{ kind: "view-reset", reason: "source" }`).
    ViewReset {
        /// Viewing actor.
        actor: ActorId,
        /// Reset angles.
        angles: Vec3,
    },
    /// Music pause switch (donor `{ kind: "music", event: { kind: "pause" } }`).
    Pause {
        /// Paused flag.
        paused: bool,
    },
    /// Skybox switch (donor `{ kind: "q1-sky", event: { kind: "skybox" } }`).
    Sky {
        /// Skybox name.
        name: String,
    },
    /// Client metadata (donor `{ kind: "q1-client", event }`).
    ClientMetadata(Q1ClientMetadataEvent),
    /// Session lifecycle signal (donor `{ kind: "q1-session", event }`).
    Session(QuakeCSessionKind),
    /// Composition prompt (donor `{ kind: "q1-composition", event }`).
    Prompt(Q1CompositionEvent),
    /// Fog addon (donor `{ kind: "q1-composition", event: { kind: "addon",
    /// event: { kind: "fog" } } }`).
    Fog {
        /// Fog density.
        density: f64,
        /// Fog color.
        color: Vec3,
        /// Transition duration in seconds.
        duration_seconds: f64,
    },
}

/// Scene query surface.
///
/// Seam for the missing `SharedSceneQueries` value from donor `src/world/collision/index.ts`.
pub trait QuakeCSourceScene {
    /// Model bounds by model index (donor `scene.modelBounds`).
    fn model_bounds(&self, index: usize) -> Bounds;
    /// Line-of-sight predicate for client visibility.
    fn visible(&self, from: Vec3, to: Vec3) -> bool;
    /// Trace a hitscan ray (donor aim trace).
    fn trace_hit_actor(&self, start: Vec3, end: Vec3, pass: &ActorId) -> Option<ActorId>;
    /// Read an actor's body origin.
    fn body_origin(&self, actor: &ActorId) -> Option<Vec3>;
    /// Run a Q1 trace (donor `scene` traceline backing the spatial builtins).
    fn trace(
        &self,
        params: &qa_guest::qc::spatial_host::TraceParams,
    ) -> Result<qa_guest::qc::spatial_host::TraceResult, GuestError>;
    /// Read Q1 contents at a point (donor `scene.pointContents` Q1 branch).
    fn point_contents(&self, point: Vec3) -> Result<f32, GuestError>;
}

/// Decoded world surface.
///
/// Seam for the missing `DecodedWorld` value from donor `src/contracts/scene.ts`: only the
/// model table the source touches is exposed.
pub trait QuakeCSourceWorld {
    /// Model count, including the world model (donor `world.models.length`).
    fn model_count(&self) -> usize;
    /// Map entity text for one mode (donor `quakeCMapEntities(world, mode)` input).
    fn map_entities(&self, mode: QuakeCSourceMode) -> String;
}

/// Shared inventory surface.
///
/// Seam for the missing `SharedInventoryTable` value from donor
/// `src/world/gameplay/inventory.ts`.
pub trait QuakeCSourceInventory {
    /// Bind per-actor inventory storage (donor `inventory.bind`).
    fn bind(&mut self, actor: &OwnedActor, binding: QuakeCInventoryBinding);
    /// Count one item (donor `inventory.count`).
    fn count(&self, actor: &ActorId, item: &ItemId) -> f64;
    /// All entries (donor `inventory.entries`).
    fn entries(&self, actor: &ActorId) -> Vec<QuakeCInventoryEntry>;
}

/// Per-actor inventory binding.
///
/// Mirror of `InventoryStateBinding` from donor `src/world/gameplay/inventory.ts`
/// (canonical home: world gameplay lane); unify post-merge.
pub struct QuakeCInventoryBinding {
    /// Read all entries.
    pub read: Rc<dyn Fn() -> Vec<QuakeCInventoryEntry>>,
    /// Write one entry.
    pub write: Rc<dyn Fn(QuakeCInventoryEntry)>,
}

/// Inventory entry.
///
/// Mirror of `InventoryEntry` from donor `src/contracts/gameplay.ts`
/// (canonical home: gameplay-contracts lane); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCInventoryEntry {
    /// Item identifier.
    pub item: ItemId,
    /// Entry count.
    pub count: f64,
    /// Entry capacity.
    pub capacity: f64,
    /// Source-counter policy (donor `countPolicy`, present on ammo entries).
    pub source_counter: bool,
}

/// Actor touch contact.
///
/// Mirror of `TouchContact` from donor `src/contracts/world.ts`
/// (canonical home: world-contracts lane); unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeCTouchContact {
    /// Touching actor.
    pub touch_self: OwnedActor,
    /// Other actor.
    pub other: ActorId,
}

/// Think hook (donor `ActorCallbacks.think`).
pub type QuakeCThinkHook = Rc<dyn Fn(&OwnedActor, &FrameContext)>;
/// Primary-weapon selection predicate (donor `primaryWeaponSelected`).
pub type QuakeCPrimaryWeaponSelected = Rc<dyn Fn(&ActorId) -> bool>;
/// Inventory bind override (donor `bindInventory`).
pub type QuakeCBindInventory = Rc<dyn Fn(&OwnedActor, QuakeCInventoryBinding)>;
/// Inventory give override (donor `giveInventory`).
pub type QuakeCGiveInventory = Rc<dyn Fn(&ActorId, &[String]) -> bool>;
/// Client spawn hook (donor `clientSpawned`).
pub type QuakeCClientSpawned = Rc<dyn Fn(&OwnedActor)>;
/// Foreign classname hook (donor `foreignClassname`).
pub type QuakeCForeignClassname = Rc<dyn Fn(&ActorId) -> String>;
/// Damage admission hook (donor `damageAllowed`).
pub type QuakeCDamageAllowed = Rc<dyn Fn(&DamageRequest) -> bool>;
/// Weapon ownership hook (donor `ownsWeapon`).
pub type QuakeCOwnsWeapon = Rc<dyn Fn(&ActorId, &ItemId) -> bool>;
/// Admission hook (donor `admit`).
pub type QuakeCAdmit<P> = Rc<dyn Fn(&OwnedActor, usize, &QuakeCSource<P>)>;

/// Actor callback hooks bound by admission.
///
/// Donor `ActorCallbacks` bind payload from `src/world/actors/callbacks.ts`
/// (`think` plus `touch`; pain/die/use bind null).
pub struct QuakeCActorHooks {
    /// Think hook.
    pub think: QuakeCThinkHook,
    /// Touch hook.
    pub touch: Rc<dyn Fn(&QuakeCTouchContact)>,
}

/// Actor callback table surface.
///
/// Seam for the missing `ActorCallbackTable` value from donor
/// `src/world/actors/callbacks.ts`.
pub trait QuakeCSourceCallbacks {
    /// Bind per-actor callbacks (donor `callbacks.bind`).
    fn bind_actor(&mut self, actor: &OwnedActor, hooks: QuakeCActorHooks);
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::create_mount_id;
    use qa_content::contract::create_mount_identity;
    use qa_content::contract::create_mount_plan_id;
    use qa_content::contract::ContentId;
    use qa_content::contract::ContentMount;
    use qa_content::contract::LooseMount;
    use qa_content::contract::ResolvedMountPlan;
    use qa_content::mounts::open_mount_plan;
    use qa_content::mounts::OpenMountOptions;
    use qa_guest::qc::program::QcOpcode;

    /// Minimal progs.dat image: two Done statements, two functions, the
    /// globals/fields combat validation needs, plus extra strings.
    fn encode_progs(extra_strings: &[&str]) -> Vec<u8> {
        let globals: Vec<(u16, u16, &str)> = vec![(2, 0, "time"), (2, 1, "self_time"), (4, 2, "self")];
        let fields: Vec<(u16, u16, &str)> = vec![
            (2, 0, "health"),
            (2, 1, "takedamage"),
            (2, 2, "flags"),
            (2, 3, "invincible_finished"),
            (2, 4, "armorvalue"),
            (2, 5, "armortype"),
        ];
        let mut strings = vec![0u8];
        let mut offsets = HashMap::new();
        for name in ["main", "test.qc", "T_Damage"]
            .into_iter()
            .chain(extra_strings.iter().copied())
            .chain(globals.iter().map(|(_, _, name)| *name))
            .chain(fields.iter().map(|(_, _, name)| *name))
        {
            if offsets.contains_key(name) {
                continue;
            }
            offsets.insert(name, strings.len() as i32);
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }
        let statements = vec![
            (QcOpcode::Done as u16, 0u16, 0u16, 0u16),
            (QcOpcode::Done as u16, 0u16, 0u16, 0u16),
        ];
        let functions = [
            (0i32, 0i32, 0i32, 0i32, offsets["main"], offsets["test.qc"], 0i32),
            (1, 0, 0, 0, offsets["T_Damage"], offsets["test.qc"], 0),
        ];
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut statement_blob = Vec::new();
        for (op, a, b, c) in &statements {
            for word in [*op, *a, *b, *c] {
                statement_blob.extend_from_slice(&word.to_le_bytes());
            }
        }
        blobs.push(statement_blob);
        let definitions = |entries: &[(u16, u16, &str)]| {
            let mut blob = Vec::new();
            for (raw, at, name) in entries {
                blob.extend_from_slice(&raw.to_le_bytes());
                blob.extend_from_slice(&at.to_le_bytes());
                blob.extend_from_slice(&offsets[*name].to_le_bytes());
            }
            blob
        };
        blobs.push(definitions(&globals));
        blobs.push(definitions(&fields));
        let mut function_blob = Vec::new();
        for (first, params, locals, profile, name, file, count) in &functions {
            for word in [*first, *params, *locals, *profile, *name, *file, *count] {
                function_blob.extend_from_slice(&word.to_le_bytes());
            }
            function_blob.extend_from_slice(&[0u8; 8]);
        }
        blobs.push(function_blob);
        blobs.push(strings.clone());
        blobs.push(vec![0u8; 28 * 4]);
        let counts = [
            statements.len() as i32,
            globals.len() as i32,
            fields.len() as i32,
            functions.len() as i32,
            strings.len() as i32,
            28,
        ];
        let mut image = Vec::new();
        image.extend_from_slice(&6i32.to_le_bytes());
        image.extend_from_slice(&5927i32.to_le_bytes());
        let mut offset = 60i32;
        for (blob, count) in blobs.iter().zip(counts) {
            image.extend_from_slice(&offset.to_le_bytes());
            image.extend_from_slice(&count.to_le_bytes());
            offset += blob.len() as i32;
        }
        image.extend_from_slice(&27i32.to_le_bytes());
        for blob in &blobs {
            image.extend_from_slice(blob);
        }
        image
    }

    fn fixture_program(extra_strings: &[&str]) -> QcProgram {
        let image = encode_progs(extra_strings);
        load_qc_program(&image, None, "test.dat").unwrap()
    }

    fn fixture_bytes(extra_strings: &[&str]) -> Vec<u8> {
        encode_progs(extra_strings)
    }

    fn spr_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"IDSP");
        for word in [1i32, 0] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
        for word in [1i32, 1, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.extend_from_slice(&0.0f32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        for word in [0i32, 0, 1, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(7);
        bytes
    }

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40i32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16i32.to_le_bytes());
        bytes.extend_from_slice(&1i16.to_le_bytes());
        bytes.extend_from_slice(&1i16.to_le_bytes());
        bytes.extend_from_slice(&11025i32.to_le_bytes());
        bytes.extend_from_slice(&11025i32.to_le_bytes());
        bytes.extend_from_slice(&1i16.to_le_bytes());
        bytes.extend_from_slice(&8i16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(&[128, 130, 126, 128]);
        bytes
    }

    fn empty_bsp_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&29i32.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0u8, 15 * 8));
        bytes
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn loose_mounts(dir: &std::path::Path) -> MountedContent {
        let mount_id = create_mount_id("test", "loose").unwrap();
        let plan = ResolvedMountPlan {
            id: create_mount_plan_id("test", "r1").unwrap(),
            mounts: vec![ContentMount::Loose(LooseMount {
                identity: create_mount_identity(mount_id.clone(), ContentId("q1:classic:test:v1".to_string()), 0)
                    .unwrap(),
                root_path: dir.to_string_lossy().into_owned(),
            })],
            default_order: vec![mount_id],
            prefix_orders: Vec::new(),
        };
        open_mount_plan(&plan, OpenMountOptions::default()).unwrap()
    }

    #[test]
    fn resources_scan_strings_and_entities() {
        let dir = scratch_dir("qc-source-test-resources");
        std::fs::write(dir.join("x.spr"), spr_bytes()).unwrap();
        std::fs::create_dir_all(dir.join("sound")).unwrap();
        std::fs::write(dir.join("sound").join("y.wav"), wav_bytes()).unwrap();
        let mounts = loose_mounts(&dir);
        let program = fixture_program(&["x.spr", "missing.mdl"]);
        let resources = prepare_quake_c_resources(&program, &mounts, "{\n\"noise\" \"y.wav\"\n}\n").unwrap();
        assert!(resources.contains_key("x.spr"));
        assert!(resources.contains_key("y.wav"));
        assert!(!resources.contains_key("missing.mdl"));
        let bounds = resources["x.spr"].model_bounds.as_ref().unwrap();
        assert_eq!(bounds.min.x, -0.5);
        assert_eq!(bounds.max.z, 0.5);
        assert!(resources["y.wav"].model_bounds.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resources_reject_bad_assets() {
        let dir = scratch_dir("qc-source-test-bad");
        std::fs::create_dir_all(dir.join("sound")).unwrap();
        std::fs::write(dir.join("sound").join("bad.wav"), b"not a wav").unwrap();
        std::fs::write(dir.join("empty.bsp"), empty_bsp_bytes()).unwrap();
        std::fs::write(dir.join("junk.mdl"), b"not a model").unwrap();
        let mounts = loose_mounts(&dir);
        let program = fixture_program(&["bad.wav"]);
        assert!(prepare_quake_c_resources(&program, &mounts, "").is_err());
        let program = fixture_program(&["empty.bsp"]);
        assert!(matches!(
            prepare_quake_c_resources(&program, &mounts, "").unwrap_err(),
            QuakeCSourceError::NoWorldModel(_)
        ));
        let program = fixture_program(&["junk.mdl"]);
        assert!(prepare_quake_c_resources(&program, &mounts, "").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_loads_minimal_source() {
        let prepared = prepared_fixture("qc-source-test-prepare", false);
        assert!(prepared.teams.is_none());
        assert!(prepared.combat_declaration.is_some());
        assert_eq!(prepared.message_dialect, RereleaseMessages::KnownRetail);
        assert!(prepared.declared_pickups.is_empty());
        assert!(prepared.weapon_declaration.is_none());
        assert!(prepared.resources.is_empty());
    }

    #[test]
    fn prepare_rejects_underivable_program() {
        let dir = scratch_dir("qc-source-test-underivable");
        std::fs::write(dir.join("progs.dat"), fixture_bytes(&[])).unwrap();
        let mounts = loose_mounts(&dir);
        let found = mounts.open("progs.dat", |_| true).unwrap().unwrap();
        let execution = QuakeCExecution {
            owner: test_owner(),
            artifact: found.reference,
            api: QuakeCApiIdentity::Netquake,
        };
        // No combat declaration: the id1 derivation rejects the fixture.
        assert!(prepare_quake_c_source(&execution, &mounts, "", &mounts).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_rejects_api_mismatch() {
        let dir = scratch_dir("qc-source-test-mismatch");
        std::fs::write(dir.join("progs.dat"), fixture_bytes(&[])).unwrap();
        let mounts = loose_mounts(&dir);
        let found = mounts.open("progs.dat", |_| true).unwrap().unwrap();
        let execution = QuakeCExecution {
            owner: test_owner(),
            artifact: found.reference,
            api: QuakeCApiIdentity::Quakeworld,
        };
        assert!(matches!(
            prepare_quake_c_source(&execution, &mounts, "", &mounts).unwrap_err(),
            QuakeCSourceError::ExecutionMismatch
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn compat_json(digest: &str, scale: bool) -> Vec<u8> {
        let scale = if scale {
            r#","damageScale":{"function":"T_Damage","entry":0,"exit":1,"damage":28,"statements":[]}"#
        } else {
            ""
        };
        format!(
            r#"{{"version":1,"artifactDigest":"{digest}","combat":{{"damage":{{"function":"T_Damage","arguments":[],"globals":[]}}{scale}}}}}"#
        )
        .into_bytes()
    }

    fn test_owner() -> QuakeCExecutionOwner {
        QuakeCExecutionOwner {
            provider: qa_core::identity::ProviderId::new("q1", "test"),
            content: "test-content".to_string(),
        }
    }

    fn prepared_fixture(name: &str, scale: bool) -> PreparedQuakeCSource {
        let dir = scratch_dir(name);
        std::fs::write(dir.join("progs.dat"), fixture_bytes(&[])).unwrap();
        let program = fixture_program(&[]);
        let digest = format!("{}:{}", program.digest.algorithm, program.digest.value);
        std::fs::write(dir.join("quakec-compatibility.json"), compat_json(&digest, scale)).unwrap();
        let mounts = loose_mounts(&dir);
        let found = mounts.open("progs.dat", |_| true).unwrap().unwrap();
        let execution = QuakeCExecution {
            owner: test_owner(),
            artifact: found.reference,
            api: QuakeCApiIdentity::Netquake,
        };
        let prepared = prepare_quake_c_source(&execution, &mounts, "", &mounts).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        prepared
    }

    #[test]
    fn damage_scaling_reads_declaration() {
        let scaled = prepared_fixture("qc-source-test-scaled", true);
        assert!(prepared_quake_c_damage_scaling(&scaled).unwrap());
        // Without a declared scale the fixture program is not derivable, so
        // the id1 fallback reports it instead of guessing.
        let plain = prepared_fixture("qc-source-test-plain", false);
        assert!(prepared_quake_c_damage_scaling(&plain).is_err());
    }

    #[test]
    fn weapon_stage_resolves_absent() {
        let prepared = prepared_fixture("qc-source-test-stage", false);
        assert!(prepared.weapon_stage.is_none());
        assert!(prepared_quake_c_weapon_stage(&prepared).unwrap().is_none());
        let mut explicit = prepared;
        explicit.weapon_stage = Some(QcWeaponStage {
            dispatcher: 1,
            client: None,
            continuations: std::collections::HashSet::new(),
            repeats: Vec::new(),
        });
        assert!(prepared_quake_c_weapon_stage(&explicit).unwrap().is_some());
    }

    #[test]
    fn guest_view_resolves_program() {
        let program = fixture_program(&[]);
        let view = GuestProgramView::new(&program);
        assert!(view.digest().starts_with("sha256:"));
        assert_eq!(view.api_kind(), GuestApiKind::Q1Netquake);
        assert_eq!(view.field_type("health"), Some(GuestValue::Float));
        assert_eq!(view.field_type("bogus"), None);
        assert_eq!(view.global_type("time"), Some(GuestValue::Float));
        assert_eq!(view.global_type("self"), Some(GuestValue::Entity));
        let function = view.function_named("T_Damage").unwrap();
        assert_eq!(function.first_statement, 1);
        assert!(view.function_named("bogus").is_none());
        assert_eq!(view.function_at(1).unwrap().name, "T_Damage");
        assert!(view.function_at(99).is_none());
        assert!(view.function_at(-1).is_none());
        assert_eq!(view.functions().len(), 2);
    }

    #[test]
    fn content_views_mirror_program() {
        let program = fixture_program(&[]);
        let views = ContentProgramViews::build(&program);
        let view = views.view(&program);
        assert_eq!(view.statements.len(), 2);
        assert_eq!(view.globals.len(), 3);
        assert_eq!(view.fields.len(), 6);
        assert_eq!(view.functions.len(), 2);
        assert!(view.digest.starts_with("sha256:"));
        assert_eq!(view.api, ContentApi::Netquake);
    }

    #[test]
    fn combat_projection_maps_stages() {
        use qa_guest::qc::compatibility::QcArmorFlags;
        use qa_guest::qc::compatibility::QcArmorStage;
        use qa_guest::qc::compatibility::QcDamageScale;
        use qa_guest::qc::compatibility::QcEmptyArmor;
        let combat = QcCombatDeclaration {
            damage: CallSite {
                function: "T_Damage".to_string(),
                arguments: Vec::new(),
                globals: Vec::new(),
            },
            damage_scale: Some(QcDamageScale {
                kind: None,
                function: "T_Damage".to_string(),
                entry: 0,
                exit: 1,
                damage: 4,
                statements: Vec::new(),
            }),
            armor_stage: Some(QcArmorStage {
                function: "T_Damage".to_string(),
                entry: 0,
                exit: 1,
                target: 2,
                damage: 3,
                saved: 5,
                regular_scale: Vec::new(),
                flags: QcArmorFlags::None,
                statements: Vec::new(),
            }),
            empty_armor: Some(QcEmptyArmor {
                item: "q1:item_armor2".to_string(),
                absorption: 0.6,
            }),
        };
        let runtime = runtime_combat(&combat).unwrap();
        assert_eq!(runtime.damage.function, "T_Damage");
        let scale = runtime.damage_scale.as_ref().unwrap();
        assert_eq!(scale.kind, ModQcDamageScaleKind::Multiplier);
        assert_eq!(scale.entry, 0);
        let armor = runtime.armor_stage.as_ref().unwrap();
        assert_eq!(armor.target, 2);
        assert_eq!(armor.damage, 3);
        let program = fixture_program(&[]);
        validate_qc_mod_combat(&GuestProgramView::new(&program), &runtime).unwrap();
    }

    use crate::bootstrap::simulation::random::SourceRandom;
    use crate::persistence::recipe::ExecutableRecipe;
    use qa_content::contract::{
        OriginalPickupAdmission, OriginalPickupContinuation, OriginalPickupOutcome, SourcePickupLifetime,
        SourcePickupSelection,
    };
    use qa_content::q1::foundation::gameplay::{AttackProvenance, DamageDelivery};
    use qa_content::q1::quakec::qc_view::SourceDamageExecute;
    use qa_core::identity::IdentityOwner;
    use qa_core::time::FramePhase;
    use qa_guest::qc::actor_state::QcBodyBinding;
    use qa_guest::qc::actor_state::{BodyState, SharedSolid};
    use qa_guest::qc::message_effects::{BeamStyle, PointEffect};
    use qa_guest::qc::spatial_host::TraceResult as GuestTraceResult;
    use qa_world::movement::q1::types::{Q1PhysicsEntity, Q1PusherServices, Q1Trace};

    fn surface_fields() -> Vec<(u16, &'static str)> {
        let vectors = [
            "origin",
            "angles",
            "velocity",
            "mins",
            "v_angle",
            "punchangle",
            "movedir",
            "view_ofs",
            "avelocity",
            "oldorigin",
        ];
        let floats = [
            "movetype",
            "flags",
            "waterlevel",
            "watertype",
            "teleport_time",
            "idealpitch",
            "fixangle",
            "health",
            "team",
            "colormap",
            "items",
            "weapon",
            "currentammo",
            "weaponframe",
            "attack_finished",
            "frame",
            "nextthink",
            "modelindex",
            "solid",
            "takedamage",
            "invincible_finished",
            "armorvalue",
            "armortype",
            "max_health",
            "super_damage_finished",
            "invisible_finished",
            "radsuit_finished",
            "button0",
            "button2",
            "impulse",
            "spawnflags",
            "maxspeed",
            "gravity",
            "ammo_shells",
            "ammo_nails",
            "ammo_rockets",
            "ammo_cells",
        ];
        let strings = [
            "model",
            "classname",
            "netname",
            "deathtype",
            "target",
            "targetname",
            "killtarget",
        ];
        let mut fields: Vec<(u16, &'static str)> = Vec::new();
        for name in vectors {
            fields.push((3, name));
        }
        for name in floats {
            fields.push((2, name));
        }
        for name in strings {
            fields.push((1, name));
        }
        fields.push((4, "groundentity"));
        fields.push((4, "enemy"));
        fields.push((4, "goalentity"));
        fields.push((4, "aiment"));
        fields.push((4, "owner"));
        fields.push((6, "think"));
        fields.push((6, "touch"));
        fields.push((6, "th_pain"));
        fields.push((6, "th_die"));
        fields.push((2, "items2"));
        fields.push((2, "frags"));
        fields.push((2, "lastruntime"));
        fields
    }

    fn surface_globals() -> Vec<(u16, &'static str)> {
        let mut globals = vec![
            (2u16, "time"),
            (2, "frametime"),
            (4, "self"),
            (4, "other"),
            (1, "mapname"),
            (2, "force_retouch"),
            (4, "newmis"),
            (2, "serverflags"),
            (2, "deathmatch"),
            (2, "skill"),
            (2, "coop"),
            (2, "teamplay"),
        ];
        for (raw, name) in [(4u16, "pd0"), (4, "pd1"), (4, "pd2"), (2, "pd3")] {
            globals.push((raw, name));
        }
        for (raw, name) in [
            (6u16, "T_Damage"),
            (2, "IT2_ARMOR1"),
            (2, "IT2_ARMOR2"),
            (2, "IT2_ARMOR3"),
            (5, "fld_th_pain"),
            (2, "tmp0"),
        ] {
            globals.push((raw, name));
        }
        for ordinal in 1..=16 {
            let name: &'static str = match ordinal {
                1 => "parm1",
                2 => "parm2",
                3 => "parm3",
                4 => "parm4",
                5 => "parm5",
                6 => "parm6",
                7 => "parm7",
                8 => "parm8",
                9 => "parm9",
                10 => "parm10",
                11 => "parm11",
                12 => "parm12",
                13 => "parm13",
                14 => "parm14",
                15 => "parm15",
                _ => "parm16",
            };
            globals.push((2, name));
        }
        globals
    }

    fn surface_functions() -> Vec<&'static str> {
        vec![
            "main",
            "T_Damage",
            "StartFrame",
            "ClientConnect",
            "PutClientInServer",
            "ClientKill",
            "ClientDisconnect",
            "PlayerPreThink",
            "PlayerPostThink",
            "SetNewParms",
            "SetChangeParms",
            "W_SetCurrentAmmo",
            "spawn",
            "worldspawn",
        ]
    }

    fn surface_bytes_with_crc(crc: i32) -> Vec<u8> {
        let functions = surface_functions();
        let mut statements = vec![0u16; (functions.len() + 2) * 4];
        statements[4] = 29;
        statements[5] = 0;
        statements[6] = 20;
        statements[7] = 21;
        statements[8] = 52;
        statements[9] = 21;
        let statement_blob: Vec<u8> = statements.iter().flat_map(|word| word.to_le_bytes()).collect();
        let mut strings = vec![0u8];
        let mut intern = |text: &str| {
            let offset = strings.len() as i32;
            strings.extend_from_slice(text.as_bytes());
            strings.push(0);
            offset
        };
        let mut globals_blob = Vec::new();
        for (index, (raw, name)) in surface_globals().into_iter().enumerate() {
            let offset = index as u16;
            globals_blob.extend_from_slice(&raw.to_le_bytes());
            globals_blob.extend_from_slice(&offset.to_le_bytes());
            globals_blob.extend_from_slice(&intern(name).to_le_bytes());
        }
        let mut fields_blob = Vec::new();
        let mut field_offset = 0u16;
        for (raw, name) in surface_fields() {
            fields_blob.extend_from_slice(&raw.to_le_bytes());
            fields_blob.extend_from_slice(&field_offset.to_le_bytes());
            fields_blob.extend_from_slice(&intern(name).to_le_bytes());
            field_offset += if raw == 3 { 3 } else { 1 };
        }
        let mut function_blob = Vec::new();
        for (index, name) in functions.iter().enumerate() {
            let first = if index == 0 {
                0
            } else if index == 1 {
                1
            } else {
                index as i32 + 2
            };
            let damage = *name == "T_Damage";
            let words = if damage {
                [first, 12, 0, 0, intern(name), intern("surface.qc"), 4]
            } else {
                [first, 0, 0, 0, intern(name), intern("surface.qc"), 0]
            };
            for word in words {
                function_blob.extend_from_slice(&word.to_le_bytes());
            }
            if damage {
                function_blob.extend_from_slice(&[1u8, 1, 1, 1, 0, 0, 0, 0]);
            } else {
                function_blob.extend_from_slice(&[0u8; 8]);
            }
        }
        let mut values = vec![0u8; 96 * 4];
        values[16 * 4..17 * 4].copy_from_slice(&1i32.to_le_bytes());
        values[17 * 4..18 * 4].copy_from_slice(&1f32.to_le_bytes());
        values[18 * 4..19 * 4].copy_from_slice(&2f32.to_le_bytes());
        values[19 * 4..20 * 4].copy_from_slice(&4f32.to_le_bytes());
        values[20 * 4..21 * 4].copy_from_slice(&81i32.to_le_bytes());
        let mut blobs = vec![statement_blob, globals_blob, fields_blob, function_blob];
        blobs.push(strings.clone());
        blobs.push(values);
        let counts = [
            functions.len() as i32 + 2,
            surface_globals().len() as i32,
            surface_fields().len() as i32,
            functions.len() as i32,
            strings.len() as i32,
            96,
        ];
        let mut image = Vec::new();
        image.extend_from_slice(&6i32.to_le_bytes());
        image.extend_from_slice(&crc.to_le_bytes());
        let mut at = 60i32;
        for (blob, count) in blobs.iter().zip(counts) {
            image.extend_from_slice(&at.to_le_bytes());
            image.extend_from_slice(&count.to_le_bytes());
            at += blob.len() as i32;
        }
        image.extend_from_slice(&96i32.to_le_bytes());
        for blob in &blobs {
            image.extend_from_slice(blob);
        }
        image
    }

    fn surface_prepared_with(name: &str, crc: i32, api: QuakeCApiIdentity) -> PreparedQuakeCSource {
        let dir = scratch_dir(name);
        std::fs::write(dir.join("progs.dat"), surface_bytes_with_crc(crc)).unwrap();
        let program = load_qc_program(&surface_bytes_with_crc(crc), None, "surface.dat").unwrap();
        let digest = format!("{}:{}", program.digest.algorithm, program.digest.value);
        let compatibility = if api == QuakeCApiIdentity::Quakeworld {
            // The surface T_Damage takes (targ, inflictor, attacker, damage).
            let args = r#"{"kind":"input","name":"self"},{"kind":"input","name":"inflictor"},{"kind":"input","name":"attacker"},{"kind":"input","name":"amount"}"#;
            format!(
                r#"{{"version":1,"artifactDigest":"{digest}","combat":{{"damage":{{"function":"T_Damage","arguments":[{args}],"globals":[]}}}}}}"#
            )
            .into_bytes()
        } else {
            format!(r#"{{"version":1,"artifactDigest":"{digest}"}}"#).into_bytes()
        };
        std::fs::write(dir.join("quakec-compatibility.json"), compatibility).unwrap();
        let mounts = loose_mounts(&dir);
        let found = mounts.open("progs.dat", |_| true).unwrap().unwrap();
        let execution = QuakeCExecution {
            owner: test_owner(),
            artifact: found.reference,
            api,
        };
        let prepared = prepare_quake_c_source(&execution, &mounts, "", &mounts).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        prepared
    }

    struct FakeActors {
        owner: IdentityOwner,
        provider: ProviderId,
        slots: std::collections::HashMap<usize, OwnedActor>,
        live: std::collections::HashSet<ActorId>,
        hooks: Vec<ReleaseHook>,
        next_generation: u32,
    }

    impl FakeActors {
        fn new(provider: ProviderId) -> Self {
            Self {
                owner: IdentityOwner::create("surface").unwrap(),
                provider,
                slots: std::collections::HashMap::new(),
                live: std::collections::HashSet::new(),
                hooks: Vec::new(),
                next_generation: 1,
            }
        }

        fn mint(&mut self, slot: usize) -> OwnedActor {
            let id = self.owner.actor(slot as u32, self.next_generation);
            self.next_generation += 1;
            self.owner.owned_actor(&id, self.provider.clone()).unwrap()
        }
    }

    impl QuakeCSourceActors for FakeActors {
        fn at_source(&self, provider: &ProviderId, slot: usize) -> Option<OwnedActor> {
            if provider != &self.provider {
                return None;
            }
            self.slots.get(&slot).cloned()
        }

        fn allocate_at_source(&mut self, _provider: &ProviderId, slot: usize, _definition: &str) -> OwnedActor {
            let actor = self.mint(slot);
            self.live.insert(actor.id().clone());
            self.slots.insert(slot, actor.clone());
            actor
        }

        fn allocate(&mut self, provider: &ProviderId, definition: &str) -> OwnedActor {
            let slot = self.slots.keys().max().map_or(1, |slot| slot + 1);
            self.allocate_at_source(provider, slot, definition)
        }

        fn release(&mut self, actor: &OwnedActor) {
            self.live.remove(actor.id());
            self.slots.retain(|_, owned| owned.id() != actor.id());
            for hook in &mut self.hooks {
                hook(actor);
            }
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.live.contains(actor)
        }

        fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
            if !self.live.contains(actor) {
                return None;
            }
            self.slots.values().find(|owned| owned.id() == actor).cloned()
        }

        fn reference_saved(&self, _saved: SavedActorId) -> ActorId {
            self.owner.actor(0, 0)
        }

        fn source_of(&self, actor: &ActorId) -> Option<(ProviderId, usize)> {
            self.slots
                .iter()
                .find(|(_, owned)| owned.id() == actor)
                .map(|(slot, _)| (self.provider.clone(), *slot))
        }

        fn on_release(&mut self, hook: ReleaseHook) {
            self.hooks.push(hook);
        }
    }

    struct FakePhysics {
        bodies: std::collections::HashMap<ActorId, BodyState>,
        gravity: f32,
    }

    impl FakePhysics {
        fn new() -> Self {
            Self {
                bodies: std::collections::HashMap::new(),
                gravity: 0.0,
            }
        }

        fn default_body() -> BodyState {
            BodyState {
                origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                bounds: Bounds {
                    min: Vec3 {
                        x: -16.0,
                        y: -16.0,
                        z: -24.0,
                    },
                    max: Vec3 {
                        x: 16.0,
                        y: 16.0,
                        z: 32.0,
                    },
                },
                ground: None,
            }
        }
    }

    struct FakePusher;

    impl Q1PusherServices for FakePusher {
        fn numeric(&self) -> NumericOps {
            NumericOps::select(Q1_DONOR_PROFILE).unwrap()
        }

        fn read(&mut self, _actor: &ActorId) -> Option<Q1PhysicsEntity> {
            None
        }

        fn candidates(&mut self) -> Vec<ActorId> {
            Vec::new()
        }

        fn write(&mut self, _entity: Q1PhysicsEntity) {}

        fn link(&mut self, _actor: &OwnedActor, _touch_triggers: bool) {}

        fn collision_enabled(&mut self, _actor: &OwnedActor, _enabled: bool) {}

        fn test_position(&mut self, _entity: &Q1PhysicsEntity) -> TraceHit {
            TraceHit::None
        }

        fn push(&mut self, _entity: &Q1PhysicsEntity, _displacement: Vec3) -> (Option<Q1PhysicsEntity>, Q1Trace) {
            (
                None,
                Q1Trace {
                    fraction: 1.0,
                    end: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    start_solid: false,
                    all_solid: false,
                    contact: qa_world::movement::types::TraceContact::None,
                    hit: TraceHit::None,
                    in_open: true,
                    in_water: false,
                    source_plane: qa_core::math::Plane {
                        normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                        distance: 0.0,
                    },
                    surface_flags: None,
                },
            )
        }

        fn blocked(&mut self, _pusher: &OwnedActor, _obstacle: &ActorId) {}
    }

    impl QuakeCSourcePhysics for FakePhysics {
        fn unlink_body(&mut self, actor: &OwnedActor) {
            self.bodies.remove(actor.id());
        }

        fn read_body(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.get(actor).cloned()
        }

        fn write_body(&mut self, actor: &OwnedActor, body: BodyState) {
            self.bodies.insert(actor.id().clone(), body);
        }

        fn bind_body(&mut self, actor: &ActorId, _slot: usize, _binding: QcBodyBinding) {
            self.bodies.insert(actor.clone(), Self::default_body());
        }

        fn link_body(&mut self, actor: &ActorId) {
            self.bodies.entry(actor.clone()).or_insert_with(Self::default_body);
        }

        fn read_q1_pusher(&self, _actor: &ActorId) -> Option<Q1PhysicsEntity> {
            None
        }

        fn write_q1_pusher(&mut self, _entity: &Q1PhysicsEntity) {}

        fn q1_pusher_services(
            &mut self,
            _projection: Rc<dyn QuakeCSourcePusherProjection>,
        ) -> Rc<RefCell<dyn Q1PusherServices>> {
            Rc::new(RefCell::new(FakePusher))
        }

        fn set_collision_enabled(&mut self, _actor: &OwnedActor, _enabled: bool) {}

        fn solid_of(&self, _actor: &ActorId) -> Option<SharedSolid> {
            None
        }

        fn touch_triggers(&mut self, _actor: &OwnedActor) {}

        fn set_world_gravity(&mut self, gravity: f32) {
            self.gravity = gravity;
        }
    }

    struct FakeCombat;

    impl GameplayAuthority for FakeCombat {
        fn apply(
            &self,
            request: DamageRequest,
            _source_damage: Option<&mut dyn FnMut(DamageRequest) -> Result<DamageOutcome, QcError>>,
        ) -> Result<DamageOutcome, QcError> {
            Ok(DamageOutcome::StaleTarget { request })
        }

        fn run_source_damage(
            &self,
            request: DamageRequest,
            _execute: SourceDamageExecute<'_>,
        ) -> Result<DamageOutcome, QcError> {
            Ok(DamageOutcome::StaleTarget { request })
        }

        fn damage_operation_active(&self) -> bool {
            false
        }
    }

    impl QuakeCSourceCombat for FakeCombat {
        fn bind_actor(&self, _actor: &OwnedActor, _binding: QuakeCCombatBinding) {}
    }

    struct FakeInventory;

    impl QuakeCSourceInventory for FakeInventory {
        fn bind(&mut self, _actor: &OwnedActor, _binding: QuakeCInventoryBinding) {}

        fn count(&self, _actor: &ActorId, _item: &ItemId) -> f64 {
            0.0
        }

        fn entries(&self, _actor: &ActorId) -> Vec<QuakeCInventoryEntry> {
            Vec::new()
        }
    }

    struct FakeCallbacks;

    impl QuakeCSourceCallbacks for FakeCallbacks {
        fn bind_actor(&mut self, _actor: &OwnedActor, _hooks: QuakeCActorHooks) {}
    }

    struct FakeWorld {
        entities: String,
    }

    impl QuakeCSourceWorld for FakeWorld {
        fn model_count(&self) -> usize {
            1
        }

        fn map_entities(&self, _mode: QuakeCSourceMode) -> String {
            self.entities.clone()
        }
    }

    struct FakeScene;

    impl QuakeCSourceScene for FakeScene {
        fn model_bounds(&self, _index: usize) -> Bounds {
            Bounds {
                min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            }
        }

        fn visible(&self, _from: Vec3, _to: Vec3) -> bool {
            true
        }

        fn trace_hit_actor(&self, _start: Vec3, _end: Vec3, _pass: &ActorId) -> Option<ActorId> {
            None
        }

        fn body_origin(&self, _actor: &ActorId) -> Option<Vec3> {
            None
        }

        fn trace(&self, params: &qa_guest::qc::spatial_host::TraceParams) -> Result<GuestTraceResult, GuestError> {
            Ok(GuestTraceResult {
                fraction: 1.0,
                all_solid: false,
                start_solid: false,
                in_water: false,
                in_open: true,
                end: params.end,
                plane: qa_core::math::Plane {
                    normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                    distance: 0.0,
                },
                hit: qa_guest::qc::spatial_host::TraceHit::None,
            })
        }

        fn point_contents(&self, _point: Vec3) -> Result<f32, GuestError> {
            Ok(-1.0)
        }
    }

    #[derive(Default)]
    struct FakeEvents {
        messages: Vec<(ClientMessage, Option<ActorId>)>,
        emitted: Vec<(String, QcPresentationEvent, Option<ActorId>)>,
        local: Vec<(String, QuakeCLocalSinkEvent, Option<ActorId>)>,
    }

    impl QuakeCSourceEvents for FakeEvents {
        fn message(&mut self, event: ClientMessage, actor: Option<&ActorId>) {
            self.messages.push((event, actor.cloned()));
        }

        fn emit(&mut self, content: &str, event: QcPresentationEvent, recipient: Option<&ActorId>) {
            self.emitted.push((content.to_string(), event, recipient.cloned()));
        }

        fn emit_local(&mut self, content: &str, event: QuakeCLocalSinkEvent, recipient: Option<&ActorId>) {
            self.local.push((content.to_string(), event, recipient.cloned()));
        }

        fn light_style(&self, _index: usize) -> String {
            String::new()
        }
    }

    struct FakePickups;

    struct FakeLifetime;

    impl SourcePickupLifetime for FakeLifetime {
        fn consume_pickup(&self, _remove: Box<dyn FnOnce()>) {}
    }

    impl OriginalPickupAdmission for FakePickups {
        fn run_source<R>(
            &self,
            _offer: &OriginalPickupOffer,
            execute: &mut dyn FnMut(SourcePickupSelection<'_>, &dyn SourcePickupLifetime) -> R,
        ) -> R {
            execute(SourcePickupSelection::Original, &FakeLifetime)
        }

        fn touch(
            &self,
            _offer: &OriginalPickupOffer,
            _continuation: &dyn OriginalPickupContinuation,
        ) -> OriginalPickupOutcome {
            OriginalPickupOutcome::Accepted
        }
    }

    use crate::persistence::recipe::MapSelection;
    use crate::persistence::recipe::{
        CampaignSelection, EnemySelection, EnvironmentSelection, EquipmentSelection, ExecutionImplementation,
        GrappleSelection, HandGrenadeSelection, PresentationSelection, ResolvedExecutionModule,
    };
    use qa_guest::checkpoint::GameApi;
    use qa_world::save::shared::{CharacterSelection, ProviderRef};

    fn test_provider_ref(provider: &str) -> ProviderRef {
        ProviderRef {
            provider: provider.to_string(),
            content: "test-content".to_string(),
        }
    }

    fn recipe_reference(requested_path: &str, identity: &str) -> crate::persistence::recipe::ResolvedResourceReference {
        crate::persistence::recipe::ResolvedResourceReference {
            id: format!("test:{requested_path}"),
            requested_path: requested_path.to_string(),
            provenance: crate::persistence::recipe::ResourceProvenance::Loose {
                mount: Box::new(crate::persistence::recipe::ContentMount::Loose {
                    identity: crate::persistence::recipe::MountIdentity {
                        id: "test:loose".to_string(),
                        content: "test-content".to_string(),
                        generation: 0,
                    },
                    root_path: "/tmp".to_string(),
                }),
                member_path: requested_path.to_string(),
            },
            identity: identity.to_string(),
            byte_length: 0,
            resolution: crate::persistence::recipe::ResourceResolution::DefaultOrder {
                plan: "test:plan".to_string(),
                rank: 0,
            },
        }
    }

    fn surface_recipe(prepared: &PreparedQuakeCSource) -> ExecutableRecipe {
        let identity = prepared.execution.artifact.identity.canonical();
        let geometry = recipe_reference("maps/test.bsp", &identity);
        ExecutableRecipe {
            mods: Vec::new(),
            weapon_behaviors: Vec::new(),
            id: "surface".to_string(),
            preset: "test".to_string(),
            map: MapSelection {
                geometry_content: String::new(),
                geometry,
                entities: test_provider_ref("test:entities"),
            },
            campaign: CampaignSelection::None,
            movement: test_provider_ref("test:movement"),
            combat: test_provider_ref("test:combat"),
            inventory: test_provider_ref("test:inventory"),
            match_provider: test_provider_ref("test:match"),
            transition: test_provider_ref("test:transition"),
            engine_behavior: test_provider_ref("test:engine"),
            character: CharacterSelection {
                definition: test_provider_ref("q1:test-character"),
                appearance: test_provider_ref("q1:test-appearance"),
            },
            weapons: Vec::new(),
            equipment: EquipmentSelection {
                grapple: GrappleSelection::Disabled,
                hand_grenades: HandGrenadeSelection::Disabled,
            },
            enemies: EnemySelection::MapDefined,
            presentation: PresentationSelection {
                doppler: String::new(),
                environment: EnvironmentSelection::Disabled,
                assets: String::new(),
                hud: test_provider_ref("test:hud"),
                effects: test_provider_ref("test:effects"),
                audio: test_provider_ref("test:audio"),
            },
            execution: vec![ResolvedExecutionModule {
                owner: test_provider_ref("q1:test"),
                role: "game".to_string(),
                api: GameApi::Q1Netquake,
                implementation: ExecutionImplementation::Quakec {
                    artifact: recipe_reference("progs.dat", &identity),
                },
            }],
            mounts: crate::persistence::recipe::ResolvedMountPlan {
                id: "test:plan".to_string(),
                mounts: Vec::new(),
                default_order: Vec::new(),
                prefix_orders: Vec::new(),
            },
            resources: Vec::new(),
            timing: Vec::new(),
            ordering: qa_world::scheduler::FrameOrdering::Native {
                clock: qa_core::time::ClockProfile::Q1Netquake {
                    minimum_frame_seconds: 0.0,
                    maximum_frame_seconds: 0.1,
                    fixed_frame_seconds: None,
                },
            },
        }
    }

    type SurfaceSourceHandles = (
        Rc<QuakeCSource<FakePickups>>,
        Rc<RefCell<FakeActors>>,
        Rc<RefCell<FakePhysics>>,
        Rc<RefCell<FakeEvents>>,
        IdentityOwner,
    );

    fn surface_source(name: &str, max_clients: usize, entities: &str) -> SurfaceSourceHandles {
        surface_source_with(name, max_clients, entities, 5927, QuakeCApiIdentity::Netquake)
    }

    fn surface_source_with(
        name: &str,
        max_clients: usize,
        entities: &str,
        crc: i32,
        api: QuakeCApiIdentity,
    ) -> SurfaceSourceHandles {
        let prepared = surface_prepared_with(name, crc, api);
        let provider = prepared.execution.owner.provider.clone();
        let actors = Rc::new(RefCell::new(FakeActors::new(provider)));
        let physics = Rc::new(RefCell::new(FakePhysics::new()));
        let events = Rc::new(RefCell::new(FakeEvents::default()));
        let identities = IdentityOwner::create("surface-clients").unwrap();
        let options = QuakeCSourceOptions {
            recipe: surface_recipe(&prepared),
            world: Rc::new(RefCell::new(FakeWorld {
                entities: entities.to_string(),
            })),
            scene: Rc::new(RefCell::new(FakeScene)),
            actors: actors.clone(),
            callbacks: Rc::new(RefCell::new(FakeCallbacks)),
            physics: physics.clone(),
            combat: Rc::new(FakeCombat),
            inventory: Rc::new(RefCell::new(FakeInventory)),
            pickups: Rc::new(FakePickups),
            events: events.clone(),
            random: SourceRandom::new(1),
            admit: Rc::new(|_, _, _| {}),
            damage_request: Rc::new(|call| DamageRequest {
                attack: AttackProvenance {
                    sequence: 0,
                    time: SourceTime::Seconds(0.0),
                    attacker: None,
                    inflictor: None,
                    originating_projectile: None,
                    weapon: None,
                    weapon_provider: ProviderId::new("test", "weapon"),
                    damage_powerup_owner: None,
                    combat_provider: ProviderId::new("test", "combat"),
                    inventory_provider: ProviderId::new("test", "inventory"),
                    movement_provider: ProviderId::new("test", "movement"),
                    cause: qa_content::q1::foundation::gameplay::AttackCause::Q1 {
                        death_type: String::new(),
                        armor_effect: None,
                    },
                },
                target: call.target.clone(),
                amount: call.amount,
                knockback: 0.0,
                direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                point: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                delivery: DamageDelivery::Direct,
            }),
            print: Rc::new(|_| {}),
            change_level: Rc::new(|_| {}),
            max_clients,
            skill: 2,
            mode: QuakeCSourceMode::Singleplayer,
            initial_source_time_seconds: 0.0,
            source_registry: None,
            restore: None,
            original_save_candidate: false,
            give_inventory: None,
            primary_weapon_selected: None,
            owns_weapon: None,
            client_spawned: None,
            bind_inventory: None,
            foreign_classname: None,
            damage_allowed: None,
            pickup_policy: None,
        };
        let source = QuakeCSource::new(prepared, options).unwrap();
        (source, actors, physics, events, identities)
    }

    fn admit_surface_client(source: &QuakeCSource<FakePickups>, identities: &IdentityOwner, slot: u32) -> OwnedActor {
        if source.loading() {
            source.spawn_map().unwrap();
        }
        let client = identities.client(slot, 1);
        source.reserved_client(&client).unwrap();
        if source.kind() == QuakeCSourceKind::Quakeworld {
            source.prepare_client_spawn(&client).unwrap();
        }
        source.admit_client(&client).unwrap()
    }

    #[test]
    fn collision_reports_unknown_actors_as_absent() {
        let (source, _, _, _, identities) = surface_source("qc-collision", 2, "");
        let actor = admit_surface_client(&source, &identities, 0);
        // The fixture client has no spawned map entity, so its entity words
        // are absent and the projection fails like the donor throw.
        assert!(source.collision(&actor).is_err());
        let (other, _, _, _, other_identities) = surface_source("qc-collision-other", 2, "");
        let stranger = admit_surface_client(&other, &other_identities, 0);
        assert!(source.collision(&stranger).unwrap().is_none());
    }

    #[test]
    fn surface_constructs_and_binds_reserved_slots() {
        let (source, _, _, _, _) = surface_source("qc-surface-construct", 2, "");
        assert_eq!(source.kind(), QuakeCSourceKind::Netquake);
        assert!(source.loading());
        assert_eq!(source.time_seconds(), 0.0);
        let world = source.world_actor().unwrap();
        assert_eq!(source.source_slot(world.id()), Some(0));
        assert!(!source.is_reserved_client(world.id()));
        assert!(!source.is_active_client(world.id()));
        assert!(!source.has_client(&IdentityOwner::create("probe").unwrap().client(0, 1)));
        assert!(source.connected_client_identities().is_empty());
        assert!(source
            .client_actor(&IdentityOwner::create("probe").unwrap().client(0, 1))
            .is_none());
        assert!(source.local_client_intermission(world.id()).is_none());
        assert!(!source.is_spectator_client(world.id()));
        assert_eq!(source.classname(world.id()).unwrap(), "");
        assert_eq!(source.read_move_type(world.id()).unwrap(), Some(0));
        assert_eq!(source.notarget(world.id()).unwrap(), Some(false));
    }

    #[test]
    fn surface_admits_and_runs_client_commands() {
        let (source, _, _, _, identities) = surface_source("qc-surface-admit", 2, "");
        let actor = admit_surface_client(&source, &identities, 0);
        assert!(source.is_active_client(actor.id()));
        let client = identities.client(0, 1);
        assert!(source.has_client(&client));
        assert_eq!(source.connected_client_identities(), vec![client.clone()]);
        assert_eq!(source.client_actor(&client), Some(actor.id().clone()));
        assert!(source.is_reserved_client(actor.id()));
        assert_eq!(source.classname(actor.id()).unwrap(), "");

        source.set_match_score(actor.id(), 7.0).unwrap();
        assert_eq!(source.match_score(actor.id()).unwrap(), 7.0);
        assert!(source.set_match_score(actor.id(), f64::NAN).is_err());

        source.set_client_max_health(actor.id(), 150.0).unwrap();
        let equipment = source.client_equipment(actor.id()).unwrap();
        assert_eq!(equipment.max_health, 150.0);
        assert_eq!(equipment.quad_until, 0.0);
        assert!(source.set_client_max_health(actor.id(), f64::INFINITY).is_err());

        assert_eq!(
            source.client_powerup_expires(actor.id(), QuakeCPowerup::Quad).unwrap(),
            0.0
        );
        assert_eq!(source.death_type(actor.id()).unwrap(), "");

        let target = source.weapon_target(actor.id()).unwrap().unwrap();
        assert!(!target.monster);
        assert!(!target.aimed_damage);

        source.host_cheat(actor.id(), QuakeCCheat::God, &[]).unwrap();
        assert_eq!(source.notarget(actor.id()).unwrap(), Some(false));

        let punch = source.client_punch_angles(actor.id()).unwrap();
        assert_eq!(punch, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        source
            .set_client_punch_angles(actor.id(), Vec3 { x: 1.0, y: 2.0, z: 3.0 })
            .unwrap();
        assert_eq!(
            source.client_punch_angles(actor.id()).unwrap(),
            Vec3 { x: 1.0, y: 2.0, z: 3.0 }
        );
        assert!(!source.client_punch_advances(actor.id()).unwrap());
        assert!(source.consume_client_view_reset(actor.id()).unwrap().is_none());

        assert!(!source.client_kill(actor.id()).unwrap());
        source
            .host_cheat(actor.id(), QuakeCCheat::Give, &["health".to_string()])
            .unwrap();
        assert!(source.client_kill(actor.id()).unwrap());

        let mut info = std::collections::HashMap::new();
        info.insert("name".to_string(), "surface".to_string());
        source.set_client_info(&client, &info).unwrap();
        assert_eq!(source.client_info(&client).get("name").unwrap(), "surface");
        source.set_match_team(actor.id(), Some("3")).unwrap();
        assert_eq!(source.match_team(actor.id()).unwrap(), Some("3".to_string()));
        assert!(source.set_match_team(actor.id(), Some("nope")).is_err());
        assert!(source.set_match_team(actor.id(), None).is_err());

        source.disconnect_client(&actor).unwrap();
        assert!(!source.is_active_client(actor.id()));
        assert!(!source.has_client(&client));
    }

    #[test]
    fn surface_runs_frames_and_spawns_empty_map() {
        let (source, _, _, _, identities) = surface_source("qc-surface-frame", 2, "");
        source.spawn_map().unwrap();
        assert!(!source.loading());
        assert!(source.spawn_map().is_err());
        let frame = FrameContext {
            frame: 1,
            time: SourceTime::Seconds(0.1),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        };
        source.begin_frame(&frame).unwrap();
        assert!((source.time_seconds() - 0.1f64).abs() < 1e-6);
        let world = source.world_actor().unwrap();
        source.before_actor(&world).unwrap();
        source.end_frame().unwrap();
        let bad = FrameContext {
            frame: 2,
            time: SourceTime::Milliseconds(100),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        };
        assert!(source.begin_frame(&bad).is_err());
        assert!(source.take_new_missile().unwrap().is_none());

        let actor = admit_surface_client(&source, &identities, 0);
        source.check_water_transition(&actor).unwrap();
        source
            .write_angular_velocity(&actor, Vec3 { x: 0.0, y: 0.0, z: 0.0 })
            .unwrap();
        assert!(!source
            .request_client_weapon(actor.id(), &"q1:weapon:axe".to_string())
            .unwrap());
        source.client_pre_think(&actor).unwrap();
        source.client_post_think(&actor).unwrap();
        let ui = source.client_ui(actor.id()).unwrap();
        assert!(ui.powerups.is_empty());
        assert!(source.client_arsenal(actor.id()).is_err());
        let animation = source.client_animation(actor.id()).unwrap();
        assert_eq!(animation.provider, ProviderId::new("q1", "test-character"));
        assert!(source.local_client_view(actor.id()).is_none());
        assert_eq!(
            source.client_view_offset(actor.id()).unwrap(),
            Vec3 { x: 0.0, y: 0.0, z: 0.0 }
        );
    }

    #[test]
    fn surface_round_trips_movement_state() {
        let (source, _, _, _, identities) = surface_source("qc-surface-move", 2, "");
        let actor = admit_surface_client(&source, &identities, 0);
        let state = Q1MovementState {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            velocity: Vec3 { x: 4.0, y: 5.0, z: 6.0 },
            angles: Vec3 { x: 7.0, y: 8.0, z: 9.0 },
            old_origin: Vec3 {
                x: 10.0,
                y: 11.0,
                z: 12.0,
            },
            angular_velocity: Vec3 {
                x: 13.0,
                y: 14.0,
                z: 15.0,
            },
            view_angles: Vec3 {
                x: 16.0,
                y: 17.0,
                z: 18.0,
            },
            punch_angles: Vec3 {
                x: 19.0,
                y: 20.0,
                z: 21.0,
            },
            move_type: 3,
            flags: 1,
            water_level: 2,
            water_type: -3,
            teleport_time_seconds: 4.0,
            water_jump_direction: Vec3 {
                x: 22.0,
                y: 23.0,
                z: 24.0,
            },
            ideal_pitch: 25.0,
            fix_angle: false,
            health: 100.0,
            ground: qa_world::movement::types::TraceHit::None,
        };
        source.write_client_state(actor.id(), &state).unwrap();
        source
            .host_cheat(actor.id(), QuakeCCheat::Give, &["health".to_string()])
            .unwrap();
        let read = source.read_client_state(actor.id(), &state).unwrap();
        assert_eq!(read.view_angles, state.view_angles);
        assert_eq!(read.punch_angles, state.punch_angles);
        assert_eq!(read.move_type, 3);
        assert_eq!(read.flags, 1);
        assert_eq!(read.water_level, 2);
        assert_eq!(read.water_type, -3);
        assert_eq!(read.teleport_time_seconds, 4.0);
        assert_eq!(read.water_jump_direction, state.water_jump_direction);
        assert_eq!(read.ideal_pitch, 25.0);
        assert!(!read.fix_angle);
        assert_eq!(read.health, 100.0);
        assert_eq!(source.read_move_type(actor.id()).unwrap(), Some(3));
        source
            .client_input(
                actor.id(),
                &Q1UserCommand {
                    acknowledged_server_time_seconds: 0.0,
                    view_angles: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
                    forward_move: 0.0,
                    side_move: 0.0,
                    up_move: 0.0,
                    buttons: 3,
                    impulse: 0,
                },
            )
            .unwrap();
    }

    #[test]
    fn surface_travel_round_trip() {
        let (source, _, _, _, identities) = surface_source("qc-surface-travel", 2, "");
        assert!(source.capture_travel().is_err());
        let actor = admit_surface_client(&source, &identities, 0);
        let travel = source.capture_travel().unwrap();
        assert_eq!(travel.kind, QuakeCSourceKind::Netquake);
        assert_eq!(travel.clients.len(), 1);
        assert_eq!(travel.clients[0].parameters.len(), 16);
        assert_eq!(travel.server_flags, 0);
        source.disconnect_client(&actor).unwrap();

        let (fresh, _, _, _, _) = surface_source("qc-surface-travel-fresh", 2, "");
        fresh.restore_travel(&travel).unwrap();
        let clients = fresh.connected_client_identities();
        assert_eq!(clients.len(), 1);
        assert!(fresh.restore_travel(&travel).is_err());
        let mut wrong = travel.clone();
        wrong.kind = QuakeCSourceKind::Quakeworld;
        let (other, _, _, _, _) = surface_source("qc-surface-travel-other", 2, "");
        assert!(other.restore_travel(&wrong).is_err());
    }

    #[test]
    fn surface_converts_routed_messages() {
        let origin = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        let print = netquake_message_from_nq(&NqMessage::Print { text: "hi".to_string() });
        assert_eq!(
            print,
            NetQuakeMessage::Text {
                kind: NqText::Print,
                text: "hi".to_string()
            }
        );
        let center = netquake_message_from_nq(&NqMessage::CenterPrint { text: "c".to_string() });
        assert_eq!(
            center,
            NetQuakeMessage::Text {
                kind: NqText::CenterPrint,
                text: "c".to_string()
            }
        );
        let stuff = netquake_message_from_nq(&NqMessage::StuffText { text: "s".to_string() });
        assert_eq!(
            stuff,
            NetQuakeMessage::Text {
                kind: NqText::Stufftext,
                text: "s".to_string()
            }
        );
        let view = netquake_message_from_nq(&NqMessage::SetView { entity: 4 });
        assert_eq!(view, NetQuakeMessage::SetView { entity: 4 });
        let sound = netquake_message_from_nq(&NqMessage::Sound {
            entity: 2,
            channel: 1,
            index: 3,
            origin,
            volume: 9,
            attenuation: 0.5,
        });
        assert_eq!(
            sound,
            NetQuakeMessage::Sound {
                entity: 2,
                channel: 1,
                index: 3,
                volume: 9,
                attenuation: 0.5,
                origin: [1.0, 2.0, 3.0],
            }
        );
        let temp = netquake_message_from_nq(&NqMessage::TempEntity {
            effect: TempEntityEffect::Point {
                effect_type: 7,
                origin,
                count: 5,
            },
        });
        assert_eq!(
            temp,
            NetQuakeMessage::TemporaryEntity {
                effect: TemporaryEntity::Point {
                    effect_type: 7,
                    origin: [1.0, 2.0, 3.0],
                    count: 5
                },
            }
        );
        let beam = netquake_message_from_nq(&NqMessage::TempEntity {
            effect: TempEntityEffect::Beam {
                entity: 6,
                beam_type: 1,
                start: origin,
                end: origin,
            },
        });
        assert_eq!(
            beam,
            NetQuakeMessage::TemporaryEntity {
                effect: TemporaryEntity::Beam {
                    effect_type: 1,
                    entity: 6,
                    start: [1.0, 2.0, 3.0],
                    end: [1.0, 2.0, 3.0],
                },
            }
        );
        let boom = netquake_message_from_nq(&NqMessage::TempEntity {
            effect: TempEntityEffect::ExplosionColors {
                origin,
                color_start: 1,
                color_length: 2,
            },
        });
        assert_eq!(
            boom,
            NetQuakeMessage::TemporaryEntity {
                effect: TemporaryEntity::ExplosionColors {
                    origin: [1.0, 2.0, 3.0],
                    color_start: 1,
                    color_length: 2,
                },
            }
        );
    }

    #[test]
    fn surface_maps_broadcast_effects() {
        let origin = Vec3 { x: 1.0, y: 2.0, z: 3.0 };
        let actor = IdentityOwner::create("fx").unwrap().actor(1, 1);
        let effect = map_broadcast_effect(&QcBroadcastEffect::Effect {
            effect: PointEffect::Explosion,
            actor: Some(actor.clone()),
            origin,
            amount: 2,
        });
        assert_eq!(
            effect,
            QcPresentationEvent::Effect {
                effect: PointEffect::Explosion,
                actor: Some(actor.clone()),
                origin,
                amount: 2,
            }
        );
        let beam = map_broadcast_effect(&QcBroadcastEffect::Beam {
            style: BeamStyle::Lightning1,
            actor: actor.clone(),
            start: origin,
            end: origin,
        });
        assert_eq!(
            beam,
            QcPresentationEvent::Beam {
                style: BeamStyle::Lightning1,
                actor: actor.clone(),
                start: origin,
                end: origin,
            }
        );
        let boom = map_broadcast_effect(&QcBroadcastEffect::ColoredExplosion {
            origin,
            color_start: 1,
            color_length: 2,
        });
        assert_eq!(
            boom,
            QcPresentationEvent::ColoredExplosion {
                origin,
                color_start: 1,
                color_length: 2
            }
        );
        let particles = map_broadcast_effect(&QcBroadcastEffect::Particles {
            origin,
            direction: origin,
            color: 3,
            count: 4,
        });
        assert_eq!(
            particles,
            QcPresentationEvent::Particles {
                origin,
                direction: origin,
                color: 3,
                count: 4
            }
        );
        assert_eq!(QuakeCPostThink::default(), QuakeCPostThink::Immediate);
    }

    #[test]
    fn flush_messages_drains_broadcast_temp_entities() {
        let (source, _, _, events, _) = surface_source("qc-flush", 2, "");
        let slots = |_: usize| None;
        {
            let mut shared = source.shared.borrow_mut();
            // MSG_BROADCAST svc_temp_entity point (presentation-host recipe).
            shared.messages.write_byte(0, None, &slots, 4).unwrap();
            shared.messages.write_byte(0, None, &slots, 3).unwrap();
            shared.messages.write_coord(0, None, &slots, 1.0).unwrap();
            shared.messages.write_coord(0, None, &slots, 2.0).unwrap();
            shared.messages.write_coord(0, None, &slots, 3.0).unwrap();
            shared.messages.write_byte(0, None, &slots, 9).unwrap();
        }
        source.flush_messages().unwrap();
        assert_eq!(events.borrow().emitted.len(), 1);
        source.flush_messages().unwrap();
        assert_eq!(events.borrow().emitted.len(), 1);
    }

    #[test]
    fn execution_projections_read_absent_for_unknown_actors() {
        use qa_core::math::Bounds;
        let (source, _, _, _, identities) = surface_source("qc-exec-absent", 2, "");
        let id = identities.actor(9, 1);
        let owned = identities.owned_actor(&id, ProviderId::new("q1", "test")).unwrap();
        let body = WorldBodyState {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            bounds: Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            },
            ground: None,
        };
        assert!(source.motion(&owned, &body).unwrap().is_none());
        let flags = source.execution_flags(&owned).unwrap();
        assert!(!flags.fly && !flags.swim && !flags.partial_ground && !flags.player && !flags.dead);
        source
            .write_execution_flags(&owned, &PhysicsFlagChanges::default())
            .unwrap();
        let mut exec = QuakeCExecutionSource::new(source.clone());
        assert!(exec.motion(&owned, &body).is_none());
        assert!(exec.collision(&owned).is_none());
        assert_eq!(exec.flags(&owned).fly, Some(false));
        exec.write_flags(&owned, &ExecutionPhysicsFlags::default());
        assert!(exec.step_pusher(&id, 0.05).is_ok());
    }

    #[test]
    fn prepare_client_spawn_gates_quakeworld_admission() {
        let (nq, _, _, _, nq_identities) = surface_source("qc-prep-nq", 2, "");
        nq.spawn_map().unwrap();
        let nq_client = nq_identities.client(0, 1);
        nq.reserved_client(&nq_client).unwrap();
        assert!(nq.prepare_client_spawn(&nq_client).is_err());
        let (qw, _, _, _, qw_identities) =
            surface_source_with("qc-prep-qw", 2, "", 54730, QuakeCApiIdentity::Quakeworld);
        qw.spawn_map().unwrap();
        let qw_client = qw_identities.client(0, 1);
        qw.reserved_client(&qw_client).unwrap();
        assert!(qw.admit_client(&qw_client).is_err());
        qw.prepare_client_spawn(&qw_client).unwrap();
        let actor = qw.admit_client(&qw_client).unwrap();
        assert!(qw.is_active_client(actor.id()));
        assert!(qw.prepare_client_spawn(&qw_client).is_err());
    }

    #[test]
    fn take_new_missile_drains_quakeworld_word() {
        let (source, _, _, _, identities) =
            surface_source_with("qc-newmis", 2, "", 54730, QuakeCApiIdentity::Quakeworld);
        let actor = admit_surface_client(&source, &identities, 0);
        assert!(source.take_new_missile().unwrap().is_none());
        let offset = source.machine_read(|machine| machine.global_offset("newmis")).unwrap();
        let reference = source.reference(actor.id()).unwrap();
        source
            .machine_write(|machine| machine.globals_mut().set_int(offset, reference))
            .unwrap();
        let taken = source.take_new_missile().unwrap().expect("drained missile");
        assert_eq!(taken.id(), actor.id());
        assert!(source.take_new_missile().unwrap().is_none());
    }

    #[test]
    fn run_actor_once_passes_netquake_through() {
        use qa_core::time::FramePhase;
        let (source, _, _, _, identities) = surface_source("qc-runonce-nq", 2, "");
        let actor = admit_surface_client(&source, &identities, 0);
        let frame = FrameContext {
            frame: 1,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        };
        assert!(source.run_actor_once(actor.id(), &frame).unwrap());
        assert!(source.run_actor_once(actor.id(), &frame).unwrap());
    }

    #[test]
    fn run_actor_once_claims_quakeworld_frames() {
        use qa_core::time::FramePhase;
        let (source, _, _, _, identities) =
            surface_source_with("qc-runonce-qw", 2, "", 54730, QuakeCApiIdentity::Quakeworld);
        assert_eq!(source.kind(), QuakeCSourceKind::Quakeworld);
        let actor = admit_surface_client(&source, &identities, 0);
        let frame = FrameContext {
            frame: 1,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        };
        assert!(source.run_actor_once(actor.id(), &frame).unwrap());
        assert!(!source.run_actor_once(actor.id(), &frame).unwrap());
        let millis = FrameContext {
            frame: 2,
            time: SourceTime::Milliseconds(1000),
            elapsed: SourceTime::Milliseconds(100),
            phase: FramePhase::FrameEntry,
        };
        assert!(!source.run_actor_once(actor.id(), &millis).unwrap());
        let next = FrameContext {
            frame: 3,
            time: SourceTime::Seconds(2.0),
            elapsed: SourceTime::Seconds(0.1),
            phase: FramePhase::FrameEntry,
        };
        assert!(source.run_actor_once(actor.id(), &next).unwrap());
    }
}

/// Per-actor combat vitals.
///
/// Donor `combat.bind` read payload from `quakec-source.ts` (`admit`).
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCCombatVitals {
    /// Health.
    pub health: f64,
    /// Armor state.
    pub armor: qa_content::contract::ArmorState,
    /// Mass.
    pub mass: f64,
    /// Whether the actor can take damage.
    pub can_take_damage: bool,
    /// Whether the actor is invulnerable.
    pub invulnerable: bool,
    /// Match team.
    pub team: Option<String>,
}

/// Per-actor combat binding.
///
/// Donor `combat.bind` payload from `quakec-source.ts` (`admit`).
pub struct QuakeCCombatBinding {
    /// Points-only armor grant.
    pub empty_regular_armor: Option<qa_content::q1::quakec::armor_points::QcEmptyArmorGrant>,
    /// Run source damage for one request.
    pub source_damage: Rc<
        dyn Fn(
            qa_content::q1::foundation::gameplay::DamageRequest,
        ) -> qa_content::q1::foundation::gameplay::DamageOutcome,
    >,
    /// Regular protection owner.
    pub regular_owner: ProviderId,
    /// Regular protection stage.
    pub regular_stage: Option<qa_content::q1::quakec::id1_damage::QcArmorStageHandle<'static>>,
    /// Powered protection owner.
    pub powered_owner: Option<ProviderId>,
    /// Powered protection stage.
    pub powered_stage: Option<qa_content::q1::quakec::id1_damage::QcArmorStageHandle<'static>>,
    /// Read vitals.
    pub read: Rc<dyn Fn() -> QuakeCCombatVitals>,
    /// Write health.
    pub write_health: Rc<dyn Fn(f64)>,
    /// Validate armor storage.
    pub validate_armor: Rc<dyn Fn(&qa_content::contract::ArmorState)>,
    /// Write armor storage.
    pub write_armor: Rc<dyn Fn(&qa_content::contract::ArmorState)>,
}

/// Session combat authority surface.
///
/// Extends the content [`GameplayAuthority`] with the per-actor bind the source performs at
/// admission (donor `options.combat.bind` from `quakec-source.ts`).
pub trait QuakeCSourceCombat: GameplayAuthority {
    /// Bind per-actor damage handling (donor `combat.bind`).
    fn bind_actor(&self, actor: &OwnedActor, binding: QuakeCCombatBinding);
}

/// Restore bundle.
///
/// Donor `QuakeCSourceOptions["restore"]` from `quakec-source.ts`.
pub struct QuakeCSourceRestore {
    /// Saved checkpoint.
    pub checkpoint: QuakeCCheckpoint,
    /// Restored clients.
    pub clients: Vec<ClientId>,
}

/// Synchronous source options.
///
/// Donor `QuakeCSourceOptions` from `quakec-source.ts`. Host-provided values arrive as `Rc`
/// handles; missing cross-partition values arrive as the seam traits defined above. The admission
/// type stays generic because foundation's [`Id1PickupBinding`] is generic over it (donor
/// `OriginalPickupAdmission` has a generic method and is not `dyn`-compatible).
pub struct QuakeCSourceOptions<P: qa_content::contract::OriginalPickupAdmission + 'static> {
    /// Fresh staged original-save candidate (donor `originalSaveCandidate`).
    pub original_save_candidate: bool,
    /// Injected source cvar registry (donor `sourceRegistry`).
    pub source_registry: Option<CvarRegistry>,
    /// Restore bundle (donor `restore`).
    pub restore: Option<QuakeCSourceRestore>,
    /// Executable recipe (donor `recipe`).
    pub recipe: crate::persistence::recipe::ExecutableRecipe,
    /// Decoded world (donor `world`).
    pub world: Rc<RefCell<dyn QuakeCSourceWorld>>,
    /// Scene queries (donor `scene`).
    pub scene: Rc<RefCell<dyn QuakeCSourceScene>>,
    /// Actor registry (donor `actors`).
    pub actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    /// Actor callbacks (donor `callbacks`).
    pub callbacks: Rc<RefCell<dyn QuakeCSourceCallbacks>>,
    /// Session physics (donor `physics`).
    pub physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    /// Combat authority (donor `combat`).
    pub combat: Rc<dyn QuakeCSourceCombat>,
    /// Shared inventory (donor `inventory`).
    pub inventory: Rc<RefCell<dyn QuakeCSourceInventory>>,
    /// Pickup admission (donor `pickups`).
    pub pickups: Rc<P>,
    /// Primary-weapon selection predicate (donor `primaryWeaponSelected`).
    pub primary_weapon_selected: Option<QuakeCPrimaryWeaponSelected>,
    /// Inventory bind override (donor `bindInventory`).
    pub bind_inventory: Option<QuakeCBindInventory>,
    /// Inventory give override (donor `giveInventory`).
    pub give_inventory: Option<QuakeCGiveInventory>,
    /// Client spawn hook (donor `clientSpawned`).
    pub client_spawned: Option<QuakeCClientSpawned>,
    /// Foreign classname hook (donor `foreignClassname`).
    pub foreign_classname: Option<QuakeCForeignClassname>,
    /// Damage admission hook (donor `damageAllowed`).
    pub damage_allowed: Option<QuakeCDamageAllowed>,
    /// Pickup policy hook (donor `pickupPolicy`).
    pub pickup_policy: Option<Rc<dyn QcPickupPolicy>>,
    /// Weapon ownership hook (donor `ownsWeapon`).
    pub owns_weapon: Option<QuakeCOwnsWeapon>,
    /// Session events (donor `events`).
    pub events: Rc<RefCell<dyn QuakeCSourceEvents>>,
    /// Source RNG (donor `random`).
    pub random: super::random::SourceRandom,
    /// Skill level 0-3 (donor `skill`).
    pub skill: u8,
    /// Game mode (donor `mode`).
    pub mode: QuakeCSourceMode,
    /// Maximum clients (donor `maxClients`).
    pub max_clients: usize,
    /// Initial source time in seconds (donor `initialSourceTimeSeconds`).
    pub initial_source_time_seconds: f64,
    /// Admission hook (donor `admit`).
    pub admit: QuakeCAdmit<P>,
    /// Damage request projection (donor `damageRequest`).
    pub damage_request: Rc<dyn Fn(&Id1DamageCall) -> qa_content::q1::foundation::gameplay::DamageRequest>,
    /// Print sink (donor `print`).
    pub print: Rc<dyn Fn(&str)>,
    /// Level-change sink (donor `changeLevel`).
    pub change_level: Rc<dyn Fn(&str)>,
}

/// Pending weapon selection.
///
/// Donor `pendingWeapons` entry from `quakec-source.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWeapon {
    weapon: NativeWeapon,
    following: bool,
}

/// Incoming damage frame.
///
/// Donor `incomingDamage` entry from `quakec-source.ts`.
struct IncomingDamage {
    request: qa_content::q1::foundation::gameplay::DamageRequest,
    entered: bool,
    outcome: Option<qa_content::q1::foundation::gameplay::DamageOutcome>,
}

/// Model registry row.
///
/// Donor `models` entry from `quakec-source.ts`.
#[derive(Debug, Clone, PartialEq)]
struct SourceModel {
    index: i32,
    bounds: Bounds,
}

/// Queued QuakeWorld routed messages.
///
/// Donor `routed` entry from `quakec-source.ts`.
#[derive(Debug, Clone, PartialEq)]
struct RoutedQuakeWorldMessages {
    entries: Vec<QcRoutedMessage>,
    destination: QcMessageDestination,
}

/// Queued NetQuake routed messages.
///
/// Donor `netQuakeRouted` entry from `quakec-source.ts`.
#[derive(Debug, Clone, PartialEq)]
struct RoutedNetQuakeMessages {
    messages: Vec<NqMessage>,
    destination: QcMessageDestination,
    view_targets: Vec<(usize, Option<ActorId>)>,
}

/// Router-owned message state.
///
/// Lives behind its own [`Rc`] so [`QuakeCMessageRouter`] methods never borrow [`Shared`]:
/// builtins hold `Shared` borrows while the message bindings route, so sharing one [`RefCell`]
/// would panic on reentry.
struct RouterState {
    kind: QuakeCSourceKind,
    spawning: bool,
    local_messages_started: bool,
    netquake_wire_attached: bool,
    // Pending donor `attachNetQuakeWire` capacity checks.
    #[allow(dead_code)]
    max_clients: usize,
    overflow_bytes: usize,
    signon: Vec<QcRoutedMessage>,
    routed: Vec<RoutedQuakeWorldMessages>,
    netquake_signon: Vec<NqMessage>,
    netquake_signon_views: Vec<(usize, Option<ActorId>)>,
    netquake_routed: Vec<RoutedNetQuakeMessages>,
    local_messages: QuakeCLocalMessages,
}

impl RouterState {
    fn route_qw(&mut self, entries: &[QcRoutedMessage], destination: &QcMessageDestination) {
        if destination == &QcMessageDestination::Signon {
            self.signon.extend(entries.iter().cloned());
        } else {
            self.routed.push(RoutedQuakeWorldMessages {
                entries: entries.to_vec(),
                destination: destination.clone(),
            });
        }
    }
}

/// Message router.
///
/// Implements the guest [`QcMessageRouter`] over [`RouterState`], following the donor `qw` /
/// `nq` service objects from the `QuakeCSource` constructor.
struct QuakeCMessageRouter {
    state: Rc<RefCell<RouterState>>,
    cvar_phs: Rc<dyn Fn() -> bool>,
    is_reserved_client: Rc<dyn Fn(&ActorId) -> bool>,
}

impl QcMessageRouter for QuakeCMessageRouter {
    fn api(&self) -> ApiKind {
        match self.state.borrow().kind {
            QuakeCSourceKind::Netquake => ApiKind::NetQuake,
            QuakeCSourceKind::Quakeworld => ApiKind::QuakeWorld,
        }
    }

    fn is_client(&self, actor: &ActorId) -> bool {
        (self.is_reserved_client)(actor)
    }

    fn loading(&self) -> bool {
        self.state.borrow().spawning
    }

    fn native(&self) -> bool {
        self.state.borrow().netquake_wire_attached
    }

    fn local(&self) -> bool {
        self.state.borrow().local_messages_started
    }

    fn phs(&self) -> bool {
        (self.cvar_phs)()
    }

    fn route_nq(
        &mut self,
        messages: &[NqMessage],
        destination: &QcMessageDestination,
        view_targets: &[(usize, Option<ActorId>)],
    ) {
        let mut state = self.state.borrow_mut();
        if state.kind != QuakeCSourceKind::Netquake {
            return;
        }
        if destination == &QcMessageDestination::Signon {
            let base = state.netquake_signon.len();
            for (index, actor) in view_targets {
                state.netquake_signon_views.push((base + index, actor.clone()));
            }
            state.netquake_signon.extend(messages.iter().cloned());
        } else if state.netquake_wire_attached || state.spawning || !state.local_messages_started {
            let bytes = capture_netquake_messages(messages)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            let mut queued = 0usize;
            for entry in &state.netquake_routed {
                queued += capture_netquake_messages(&entry.messages)
                    .map(|bytes| bytes.len())
                    .unwrap_or(0);
            }
            state.overflow_bytes = queued + bytes;
            state.netquake_routed.push(RoutedNetQuakeMessages {
                messages: messages.to_vec(),
                destination: destination.clone(),
                view_targets: view_targets.to_vec(),
            });
        }
    }

    fn route_qw(&mut self, entries: &[QcRoutedMessage], destination: &QcMessageDestination) {
        self.state.borrow_mut().route_qw(entries, destination);
    }
}

/// Shared source state behind [`RefCell`], following the lane's established interior-mutability
/// pattern: every [`QuakeCSource`] method takes `&self`.
#[allow(dead_code)]
struct Shared<P: qa_content::contract::OriginalPickupAdmission + 'static> {
    options: QuakeCSourceOptions<P>,
    kind: QuakeCSourceKind,
    reserved_client_slots: usize,
    current_time: f64,
    change_level_issued: bool,
    active_clients: Rc<RefCell<std::collections::HashSet<ActorId>>>,
    weapons: Vec<NativeWeapon>,
    pending_weapons: Rc<RefCell<std::collections::HashMap<ActorId, PendingWeapon>>>,
    user_info: std::collections::HashMap<usize, std::collections::HashMap<String, String>>,
    spawn_parameters: std::collections::HashMap<usize, Vec<f64>>,
    client_identities: std::collections::HashMap<usize, ClientId>,
    original_save_restored: bool,
    original_save_extension_text: String,
    spectator_slots: std::collections::HashSet<usize>,
    prepared_clients: std::collections::HashSet<usize>,
    frag_records: Vec<(ActorId, ActorId)>,
    router: Rc<RefCell<RouterState>>,
    physics_callback: Rc<RefCell<Option<QuakeCPhysicsCallback>>>,
    spawning: Rc<RefCell<bool>>,
    incoming_damage: Rc<RefCell<Vec<IncomingDamage>>>,
    models: Rc<RefCell<std::collections::HashMap<String, SourceModel>>>,
    precached: std::collections::HashMap<String, qa_guest::qc::presentation_host::QcPrecachedResource>,
    model_count: i32,
    sound_count: i32,
    storage: Rc<RefCell<SourceSlotStorage>>,
    fields: Rc<RefCell<FieldTable>>,
    cvars: Rc<RefCell<CvarRegistry>>,
    attacks: &'static Id1SynchronousAttacks<'static>,
    projectiles: &'static Id1ProjectileAttacks<'static>,
    environment: &'static Id1Environment<'static>,
    damage: &'static Id1DamageBinding<'static>,
    pickups: &'static Id1PickupBinding<'static, P>,
    weapon_stage: Option<&'static QcWeaponStageBinding<'static>>,
    messages: QcBroadcastMessages<QuakeCMessageRouter>,
    world: QcWorldHost<WorldSlots, WorldBodies>,
    clients: QcClientHost<VisibilityView>,
    borrowed: Rc<RefCell<QcBorrowedActors<BorrowedHostView>>>,
    actor_state: QcActorState<ActorLookupView>,
    movement: MovementBindings<MovementWorldView, SceneView, MovementBodiesView, RandomView>,
    spatial: qa_guest::qc::spatial_host::SpatialBindings<SpatialWorldView, SceneView, SpatialModelsView>,
    pusher: qa_guest::qc::pusher_host::QcPusherServices<
        PusherWorldView,
        PusherBodiesView,
        ForeignPusherView,
        PusherPhysicsView,
        PusherInvokerView,
    >,
    numeric: NumericOps,
    random: QcSharedRandom,
    physical: Rc<RefCell<dyn qa_world::movement::q1::types::Q1PusherServices>>,
    layout: FieldLayout,
    hook_error: Rc<RefCell<Option<GuestError>>>,
    admissions: Rc<RefCell<Vec<(ActorId, usize)>>>,
    program_view: &'static ContentView<'static>,
    machine_view: &'static MachineView,
    source: Weak<QuakeCSource<P>>,
}

/// Convert a guest failure into a content binding failure.
fn qc_error(error: GuestError) -> QcError {
    QcError::program(error.to_string(), "progs.dat")
}

/// Session RNG handle for builtin 7.
fn random_handle<P: qa_content::contract::OriginalPickupAdmission + 'static>(
    shared: &Rc<RefCell<Shared<P>>>,
) -> QcSharedRandom {
    Rc::clone(&shared.borrow().random)
}

/// Convert a guest entity-store observation into the content shape.
fn content_observation(store: &QcEntityStoreObservation) -> qa_content::q1::quakec::qc_view::QcEntityStoreObservation {
    qa_content::q1::quakec::qc_view::QcEntityStoreObservation {
        function_index: store.function_index,
        statement: store.statement,
        reference: store.reference,
        word: store.word,
        before: store.before.clone(),
        after: store.after.clone(),
    }
}

/// Host builtins over the shared source state.
fn builtin_host<P: qa_content::contract::OriginalPickupAdmission + 'static>(
    shared: Weak<RefCell<Shared<P>>>,
) -> HashMap<QcHostBuiltinName, QcBuiltin> {
    let _ = shared;
    HashMap::new()
}

/// Content machine view over a shared guest machine.
///
/// Outside execution the view borrows the shared handle; while the guest executor runs (hooks
/// and boundaries) the shared handle is already mutably borrowed, so the view reaches the
/// published executing machine instead (see `EXECUTING`).
struct MachineView {
    machine: Rc<RefCell<QcMachine>>,
    source: String,
    digest: String,
    numeric: NumericOps,
    hook_error: Rc<RefCell<Option<GuestError>>>,
}

impl MachineView {
    fn read<R>(&self, op: impl FnOnce(&QcMachine) -> R) -> R {
        let executing = EXECUTING.with(|slot| slot.borrow().is_some());
        if executing {
            ExecutionGuard::with(op).expect("executing machine is published")
        } else {
            op(&self.machine.borrow())
        }
    }

    fn write<R>(&self, op: impl FnOnce(&mut QcMachine) -> R) -> R {
        let executing = EXECUTING.with(|slot| slot.borrow().is_some());
        if executing {
            ExecutionGuard::with_mut(op).expect("executing machine is published")
        } else {
            op(&mut self.machine.borrow_mut())
        }
    }
}

impl QcMachineView for MachineView {
    fn field_offset(&self, name: &str) -> Result<usize, QcError> {
        self.read(|machine| machine.field_offset(name)).map_err(qc_error)
    }

    fn arg_int(&self, index: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.arg_int(index)).map_err(qc_error)
    }

    fn arg_float(&self, index: usize) -> Result<f64, QcError> {
        self.read(|machine| machine.arg_float(index))
            .map(f64::from)
            .map_err(qc_error)
    }

    fn strings_get(&self, offset: i32) -> Result<String, QcError> {
        self.read(|machine| machine.strings().get(offset)).map_err(qc_error)
    }

    fn set_engine_string(&self, name: &str, value: &str, capacity: usize) -> Result<i32, QcError> {
        self.write(|machine| machine.strings_mut().set_engine(name, value, capacity))
            .map_err(qc_error)
    }

    fn entity_slot(&self, reference: i32) -> Result<usize, QcError> {
        self.read(|machine| machine.entities().slot(reference))
            .map(|slot| slot as usize)
            .map_err(qc_error)
    }

    fn entity_reference(&self, slot: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.entities().reference(slot as u32))
            .map_err(qc_error)
    }

    fn entity_int(&self, slot: usize, word: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.entities().slot_int(slot as u32, word))
            .map_err(qc_error)
    }

    fn entity_float(&self, slot: usize, word: usize) -> Result<f64, QcError> {
        self.read(|machine| machine.entities().slot_float(slot as u32, word))
            .map(f64::from)
            .map_err(qc_error)
    }

    fn entity_vector(&self, slot: usize, word: usize) -> Result<Vec3, QcError> {
        self.read(|machine| machine.entities().slot_vector(slot as u32, word))
            .map_err(qc_error)
    }

    fn set_entity_int(&self, slot: usize, word: usize, value: i32) -> Result<(), QcError> {
        self.write(|machine| machine.entities_mut().set_slot_int(slot as u32, word, value))
            .map_err(qc_error)
    }

    fn set_entity_float(&self, slot: usize, word: usize, value: f64) -> Result<(), QcError> {
        self.write(|machine| machine.entities_mut().set_slot_float(slot as u32, word, value as f32))
            .map_err(qc_error)
    }

    fn entity_snapshot(&self, slot: usize) -> Result<Vec<u8>, QcError> {
        self.read(|machine| machine.entities().record_bytes(slot as u32).map(<[u8]>::to_vec))
            .map(|bytes| bytes.to_vec())
            .map_err(qc_error)
    }

    fn staging_snapshot(&self) -> Vec<u8> {
        self.read(|machine| {
            let bytes = machine.globals().bytes();
            let end = 112.min(bytes.len());
            bytes[4.min(end)..end].to_vec()
        })
    }

    fn restore_staging(&self, bytes: &[u8]) {
        self.write(|machine| {
            let globals = machine.globals_mut().bytes_mut();
            let end = (4 + bytes.len()).min(globals.len()).min(112);
            if 4 < end {
                globals[4..end].copy_from_slice(&bytes[..end - 4]);
            }
        });
    }

    fn global_range(&self, offset: usize, words: usize) -> Result<Vec<u8>, QcError> {
        self.read(|machine| {
            let bytes = machine.globals().bytes();
            let start = offset * 4;
            let end = start + words * 4;
            if end > bytes.len() {
                return Err(QcError::program("global range out of bounds", "progs.dat"));
            }
            Ok(bytes[start..end].to_vec())
        })
    }

    fn set_global_range(&self, offset: usize, bytes: &[u8]) {
        self.write(|machine| {
            let globals = machine.globals_mut().bytes_mut();
            let start = offset * 4;
            let end = (start + bytes.len()).min(globals.len());
            if start < globals.len() {
                globals[start..end].copy_from_slice(&bytes[..end - start]);
            }
        });
    }

    fn execute(&self, function: usize, argc: usize) -> Result<(), QcError> {
        let outcome = self.write(|machine| {
            let _guard = ExecutionGuard::enter(&mut *machine);
            machine.execute(function, argc)
        });
        if let Some(error) = self.hook_error.borrow_mut().take() {
            return Err(qc_error(error));
        }
        outcome.map_err(qc_error)
    }

    fn execute_region(
        &self,
        region: &qa_content::q1::quakec::qc_view::QcInlineRegion,
        argc: usize,
    ) -> Result<f64, QcError> {
        let guest = content_inline_region(region);
        let outcome = self.write(|machine| {
            let _guard = ExecutionGuard::enter(&mut *machine);
            machine.execute_region(&guest, argc)
        });
        if let Some(error) = self.hook_error.borrow_mut().take() {
            return Err(qc_error(error));
        }
        outcome.map(f64::from).map_err(qc_error)
    }

    fn program_source(&self) -> &str {
        &self.source
    }

    fn program_digest(&self) -> &str {
        &self.digest
    }

    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn global_int(&self, word: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.globals().int(word)).map_err(qc_error)
    }

    fn global_float(&self, word: usize) -> Result<f64, QcError> {
        self.read(|machine| machine.globals().float(word))
            .map(f64::from)
            .map_err(qc_error)
    }

    fn set_global_int(&self, word: usize, value: i32) -> Result<(), QcError> {
        self.write(|machine| machine.globals_mut().set_int(word, value))
            .map_err(qc_error)
    }

    fn set_global_float(&self, word: usize, value: f64) -> Result<(), QcError> {
        self.write(|machine| machine.globals_mut().set_float(word, value as f32))
            .map_err(qc_error)
    }

    fn global_vector(&self, word: usize) -> Result<Vec3, QcError> {
        self.read(|machine| machine.globals().vector(word)).map_err(qc_error)
    }

    fn set_global_vector(&self, word: usize, value: Vec3) -> Result<(), QcError> {
        self.write(|machine| machine.globals_mut().set_vector(word, value))
            .map_err(qc_error)
    }

    fn global_definition(&self, name: &str) -> Result<qa_content::q1::quakec::qc_view::QcGlobalInfo, QcError> {
        self.read(|machine| {
            let program = machine.program();
            let definition = program
                .globals
                .iter()
                .find(|definition| definition.name == name)
                .ok_or_else(|| QcError::program(format!("missing global {name}"), "progs.dat"))?;
            Ok(qa_content::q1::quakec::qc_view::QcGlobalInfo {
                offset: definition.offset,
                def_type: content_value(definition.value_type),
            })
        })
    }
}

/// Content registry view over the session actors seam.
struct RegistryView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
}

impl QcActorRegistry for RegistryView {
    fn is_live(&self, actor: &ActorId) -> bool {
        self.actors.borrow().is_live(actor)
    }

    fn source_of(&self, actor: &ActorId) -> Option<qa_content::q1::quakec::qc_gameplay::ActorSource> {
        self.actors
            .borrow()
            .source_of(actor)
            .map(|(provider, slot)| ActorSource { provider, slot })
    }

    fn resolve_owned(&self, actor: &ActorId) -> Option<OwnedActor> {
        self.actors.borrow().resolve_owned(actor)
    }

    fn is_owned(&self, actor: &OwnedActor) -> bool {
        self.actors.borrow().resolve_owned(actor.id()).as_ref() == Some(actor)
    }

    fn reference_saved(&self, saved: SavedActorId) -> ActorId {
        self.actors.borrow().reference_saved(saved)
    }
}

/// Content slots view over the session actors seam.
struct SlotsView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    storage: Rc<RefCell<SourceSlotStorage>>,
}

impl QcActorSlots for SlotsView {
    fn at(&self, slot: usize) -> Option<OwnedActor> {
        self.actors.borrow().at_source(&self.provider, slot)
    }

    fn provider(&self) -> ProviderId {
        self.provider.clone()
    }

    fn is_free(&self, slot: usize) -> bool {
        self.storage.borrow().is_free(slot)
    }
}

/// Guest world-slots adapter over the session actors seam plus source storage.
struct WorldSlots {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    storage: Rc<RefCell<SourceSlotStorage>>,
    provider: ProviderId,
    borrowed: Rc<RefCell<QcBorrowedActors<BorrowedHostView>>>,
    fields: Rc<RefCell<FieldTable>>,
    layout: FieldLayout,
    reserved: usize,
    foreign: Rc<dyn Fn(&ActorId) -> i32>,
}

impl qa_guest::qc::world_host::WorldSlots for WorldSlots {
    fn is_live(&self, actor: &ActorId) -> bool {
        self.actors.borrow().is_live(actor)
    }

    fn is_free(&self, slot: usize) -> bool {
        self.storage.borrow().is_free(slot)
    }

    fn source_slot(&self, actor: &ActorId) -> Option<usize> {
        let (provider, slot) = self.actors.borrow().source_of(actor)?;
        (provider == self.provider).then_some(slot)
    }

    fn at(&self, slot: usize) -> Option<ActorId> {
        self.actors
            .borrow()
            .at_source(&self.provider, slot)
            .map(|actor| actor.id().clone())
    }

    fn bind_existing(&mut self, slot: usize) -> ActorId {
        let definition = if slot == 0 {
            QuakeCSlotKind::Worldspawn
        } else if slot <= self.reserved {
            QuakeCSlotKind::ReservedClient
        } else {
            QuakeCSlotKind::Authored
        };
        let owned = self
            .actors
            .borrow_mut()
            .allocate_at_source(&self.provider, slot, definition.definition());
        let fields_allocated = self.fields.borrow().is_allocated(owned.id());
        if !fields_allocated {
            self.fields
                .borrow_mut()
                .allocate(owned.id(), &self.layout)
                .expect("field row fits the source layout");
        }
        self.storage
            .borrow_mut()
            .initialize(slot, &mut self.fields.borrow_mut(), owned.id())
            .expect("bound QC slot fits storage");
        owned.id().clone()
    }

    fn foreign_actor(&self, slot: usize) -> Option<ActorId> {
        self.borrowed.borrow().actor(slot).ok().flatten()
    }

    fn foreign_reference(&self, actor: &ActorId) -> i32 {
        (self.foreign)(actor)
    }
}

/// Guest world-bodies adapter over the session physics seam.
struct WorldBodies {
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
}

impl qa_guest::qc::world_host::WorldBodies for WorldBodies {
    fn read(&self, actor: &ActorId) -> Option<BodyState> {
        self.physics.borrow().read_body(actor)
    }

    fn bind(&mut self, actor: &ActorId, slot: usize, binding: qa_guest::qc::actor_state::QcBodyBinding) {
        self.physics.borrow_mut().bind_body(actor, slot, binding);
    }

    fn link(&mut self, actor: &ActorId) {
        self.physics.borrow_mut().link_body(actor);
    }
}

/// Guest client-slots adapter over the session actors seam.
struct ClientSlotsView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    entity_count: usize,
}

impl ClientSlots for ClientSlotsView {
    fn entity_count(&self) -> usize {
        self.entity_count
    }

    fn at(&self, slot: usize) -> Option<ActorId> {
        self.actors
            .borrow()
            .at_source(&self.provider, slot)
            .map(|actor| actor.id().clone())
    }

    fn is_live(&self, actor: &ActorId) -> bool {
        self.actors.borrow().is_live(actor)
    }

    fn is_free(&self, slot: usize) -> bool {
        self.actors.borrow().at_source(&self.provider, slot).is_none()
    }
}

/// Guest visibility adapter over the session scene seam.
struct VisibilityView {
    scene: Rc<RefCell<dyn QuakeCSourceScene>>,
}

impl VisibilityScene for VisibilityView {
    fn visible(&self, from: Vec3, to: Vec3) -> bool {
        self.scene.borrow().visible(from, to)
    }
}

/// Guest aim adapter over the session scene seam.
// Pending donor aim-scene wiring (`clientWeaponSettled`, spawn selection).
#[allow(dead_code)]
struct AimView {
    scene: Rc<RefCell<dyn QuakeCSourceScene>>,
}

impl AimScene for AimView {
    fn trace_hit_actor(&self, start: Vec3, end: Vec3, pass: &ActorId) -> Option<ActorId> {
        self.scene.borrow().trace_hit_actor(start, end, pass)
    }

    fn body_origin(&self, actor: &ActorId) -> Option<Vec3> {
        self.scene.borrow().body_origin(actor)
    }
}

thread_local! {
    /// Machine currently executing QuakeC on this thread.
    ///
    /// Guest hooks (`observe_call`, boundaries) fire while the guest executor holds `&mut QcMachine`
    /// but receive no machine handle, while content bindings legitimately touch machine state from
    /// those hooks. Every [`QuakeCSource`] execution entry publishes its machine here so binding
    /// views can reach it without borrowing the (already mutably borrowed) shared handle.
    ///
    /// Soundness: the source graph is single-threaded (`Rc`, never `Arc`), so the pointer never
    /// crosses threads. The guard that publishes it borrows the machine for a strictly longer
    /// lifetime, view operations only create short-lived references that never outlive the call, and
    /// nested execution saves and restores the previous pointer (it always names the same machine).
    static EXECUTING: RefCell<Option<*mut QcMachine>> = const { RefCell::new(None) };
}

/// Guard publishing the executing machine to binding views.
struct ExecutionGuard {
    previous: Option<*mut QcMachine>,
}

impl ExecutionGuard {
    fn enter(machine: &mut QcMachine) -> Self {
        let previous = EXECUTING.with(|slot| slot.borrow_mut().replace(machine as *mut QcMachine));
        Self { previous }
    }

    /// Borrow the executing machine shared.
    fn with<R>(op: impl FnOnce(&QcMachine) -> R) -> Option<R> {
        let ptr = EXECUTING.with(|slot| *slot.borrow())?;
        // SAFETY: published by a live guard (see `EXECUTING`); the reference dies with the call.
        Some(op(unsafe { &*ptr }))
    }

    /// Borrow the executing machine exclusively.
    fn with_mut<R>(op: impl FnOnce(&mut QcMachine) -> R) -> Option<R> {
        let ptr = EXECUTING.with(|slot| *slot.borrow())?;
        // SAFETY: published by a live guard (see `EXECUTING`); the reference dies with the call.
        Some(op(unsafe { &mut *ptr }))
    }
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        EXECUTING.with(|slot| *slot.borrow_mut() = self.previous);
    }
}

/// Pull one entity's words into the fields table.
///
/// Guest bindings read entity state through [`FieldTable`]; the machine owns the words, so the
/// session syncs rows across the boundary. Strings resolve through the machine string table and
/// entity references resolve through `resolve`.
/// Build the field layout behind one source program (`FieldLayout`).
///
/// Starts from the guest entity layout and extends it with every declared
/// program field so `pull_fields` can stage rows the guest services read.
fn source_field_layout(program: &QcProgram) -> FieldLayout {
    let mut layout = FieldLayout::qc_entity();
    for definition in &program.fields {
        let kind = match definition.value_type {
            qa_guest::qc::program::QcValueType::Float => "float",
            qa_guest::qc::program::QcValueType::Vector => "vector",
            qa_guest::qc::program::QcValueType::String => "string",
            qa_guest::qc::program::QcValueType::Entity => "entity",
            _ => continue,
        };
        layout = layout.field(&definition.name, kind);
    }
    layout
}

// Pending donor entity-state readers (`readClientState`, `readQuakeWorldState`).
#[allow(dead_code)]
fn pull_fields(
    machine: &QcMachine,
    fields: &mut FieldTable,
    layout: &FieldLayout,
    actor: &ActorId,
    slot: u32,
    resolve: &dyn Fn(u32) -> Option<ActorId>,
) -> Result<(), GuestError> {
    if !fields.is_allocated(actor) {
        fields.allocate(actor, layout)?;
    }
    let program = machine.program();
    for definition in &program.fields {
        let value = match definition.value_type {
            qa_guest::qc::program::QcValueType::Float => {
                FieldValue::Float(machine.entities().slot_float(slot, definition.offset)?)
            }
            qa_guest::qc::program::QcValueType::Vector => {
                FieldValue::Vector(machine.entities().slot_vector(slot, definition.offset)?)
            }
            qa_guest::qc::program::QcValueType::String => {
                let reference = machine.entities().slot_int(slot, definition.offset)?;
                FieldValue::Text(machine.strings().get(reference)?)
            }
            qa_guest::qc::program::QcValueType::Entity => {
                let reference = machine.entities().slot_int(slot, definition.offset)?;
                let saved = if reference == 0 {
                    None
                } else {
                    let target = machine.entities().slot(reference)?;
                    resolve(target).map(|actor| SavedActorId::from(&actor))
                };
                FieldValue::Entity(saved)
            }
            _ => continue,
        };
        fields.set(actor, &definition.name, value)?;
    }
    Ok(())
}

/// Push one entity's fields-table row back into words.
///
/// Only fields present in the row are written; strings allocate fresh indices.
/// Pending donor entity-state writers (`writeClientState`, `writeQuakeWorldState`).
#[allow(dead_code)]
fn push_fields(machine: &mut QcMachine, fields: &FieldTable, actor: &ActorId, slot: u32) -> Result<(), GuestError> {
    let names: Vec<(String, usize, qa_guest::qc::program::QcValueType)> = machine
        .program()
        .fields
        .iter()
        .map(|definition| (definition.name.clone(), definition.offset, definition.value_type))
        .collect();
    for (name, offset, value_type) in &names {
        let Ok(value) = fields.get(actor, name) else {
            continue;
        };
        match (value_type, value) {
            (qa_guest::qc::program::QcValueType::Float, FieldValue::Float(value)) => {
                machine.entities_mut().set_slot_float(slot, *offset, *value)?;
            }
            (qa_guest::qc::program::QcValueType::Vector, FieldValue::Vector(value)) => {
                machine.entities_mut().set_slot_vector(slot, *offset, *value)?;
            }
            (qa_guest::qc::program::QcValueType::String, FieldValue::Text(value)) => {
                let reference = machine.strings_mut().allocate(value)?;
                machine.entities_mut().set_slot_int(slot, *offset, reference)?;
            }
            (qa_guest::qc::program::QcValueType::Entity, FieldValue::Entity(saved)) => {
                let reference = match saved {
                    None => 0,
                    Some(saved) => machine.entities().reference(saved.slot)?,
                };
                machine.entities_mut().set_slot_int(slot, *offset, reference)?;
            }
            _ => continue,
        }
    }
    Ok(())
}

/// View core backing the `'static` content views.
///
/// Content bindings borrow the machine view, registry, slots, and program view with one
/// lifetime. The core owns every backing store plus the leaked program data the content view
/// borrows; exactly one core is published per source (see `Box::leak` precedent in
/// `qa_content::q1::addons`), which also fixes the self-reference between the machine views and
/// the machine: all views reach it through shared handles, never borrows.
struct SourceViewCore {
    machine_view: MachineView,
    registry: RegistryView,
    slots: SlotsView,
    views: ContentProgramViews,
    program_source: String,
    initial_globals: Vec<u8>,
    digest: String,
}

/// Owner token for bridge-driven source cancellation.
const BRIDGE_OWNER: u64 = 0x5143_4252_4944_4745;

/// Content machine view over a directly borrowed guest machine.
///
/// Used while the guest executor lends `&mut QcMachine` to boundary dispatch: the view reaches
/// the machine through that borrow instead of the shared handle, so no [`RefCell`] is touched.
struct DirectMachineView<'m> {
    machine: RefCell<&'m mut QcMachine>,
    source: Rc<String>,
    digest: Rc<String>,
    numeric: NumericOps,
}

impl DirectMachineView<'_> {
    fn read<R>(&self, op: impl FnOnce(&QcMachine) -> R) -> R {
        op(&self.machine.borrow())
    }

    fn write<R>(&self, op: impl FnOnce(&mut QcMachine) -> R) -> R {
        op(&mut self.machine.borrow_mut())
    }
}

impl QcMachineView for DirectMachineView<'_> {
    fn program_source(&self) -> &str {
        &self.source
    }

    fn program_digest(&self) -> &str {
        &self.digest
    }

    fn numeric(&self) -> NumericOps {
        self.numeric
    }

    fn global_int(&self, word: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.globals().int(word)).map_err(qc_error)
    }

    fn global_float(&self, word: usize) -> Result<f64, QcError> {
        self.read(|machine| machine.globals().float(word))
            .map(f64::from)
            .map_err(qc_error)
    }

    fn set_global_int(&self, word: usize, value: i32) -> Result<(), QcError> {
        self.write(|machine| machine.globals_mut().set_int(word, value))
            .map_err(qc_error)
    }

    fn set_global_float(&self, word: usize, value: f64) -> Result<(), QcError> {
        self.write(|machine| machine.globals_mut().set_float(word, value as f32))
            .map_err(qc_error)
    }

    fn global_vector(&self, word: usize) -> Result<Vec3, QcError> {
        self.read(|machine| machine.globals().vector(word)).map_err(qc_error)
    }

    fn set_global_vector(&self, word: usize, value: Vec3) -> Result<(), QcError> {
        self.write(|machine| machine.globals_mut().set_vector(word, value))
            .map_err(qc_error)
    }

    fn global_definition(&self, name: &str) -> Result<qa_content::q1::quakec::qc_view::QcGlobalInfo, QcError> {
        self.read(|machine| {
            let program = machine.program();
            let definition = program
                .globals
                .iter()
                .find(|definition| definition.name == name)
                .ok_or_else(|| QcError::program(format!("missing global {name}"), "progs.dat"))?;
            Ok(qa_content::q1::quakec::qc_view::QcGlobalInfo {
                offset: definition.offset,
                def_type: content_value(definition.value_type),
            })
        })
    }

    fn field_offset(&self, name: &str) -> Result<usize, QcError> {
        self.read(|machine| machine.field_offset(name)).map_err(qc_error)
    }

    fn arg_int(&self, index: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.arg_int(index)).map_err(qc_error)
    }

    fn arg_float(&self, index: usize) -> Result<f64, QcError> {
        self.read(|machine| machine.arg_float(index))
            .map(f64::from)
            .map_err(qc_error)
    }

    fn strings_get(&self, offset: i32) -> Result<String, QcError> {
        self.read(|machine| machine.strings().get(offset)).map_err(qc_error)
    }

    fn set_engine_string(&self, name: &str, value: &str, capacity: usize) -> Result<i32, QcError> {
        self.write(|machine| machine.strings_mut().set_engine(name, value, capacity))
            .map_err(qc_error)
    }

    fn entity_slot(&self, reference: i32) -> Result<usize, QcError> {
        self.read(|machine| machine.entities().slot(reference))
            .map(|slot| slot as usize)
            .map_err(qc_error)
    }

    fn entity_reference(&self, slot: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.entities().reference(slot as u32))
            .map_err(qc_error)
    }

    fn entity_int(&self, slot: usize, word: usize) -> Result<i32, QcError> {
        self.read(|machine| machine.entities().slot_int(slot as u32, word))
            .map_err(qc_error)
    }

    fn entity_float(&self, slot: usize, word: usize) -> Result<f64, QcError> {
        self.read(|machine| machine.entities().slot_float(slot as u32, word))
            .map(f64::from)
            .map_err(qc_error)
    }

    fn entity_vector(&self, slot: usize, word: usize) -> Result<Vec3, QcError> {
        self.read(|machine| machine.entities().slot_vector(slot as u32, word))
            .map_err(qc_error)
    }

    fn set_entity_int(&self, slot: usize, word: usize, value: i32) -> Result<(), QcError> {
        self.write(|machine| machine.entities_mut().set_slot_int(slot as u32, word, value))
            .map_err(qc_error)
    }

    fn set_entity_float(&self, slot: usize, word: usize, value: f64) -> Result<(), QcError> {
        self.write(|machine| machine.entities_mut().set_slot_float(slot as u32, word, value as f32))
            .map_err(qc_error)
    }

    fn entity_snapshot(&self, slot: usize) -> Result<Vec<u8>, QcError> {
        self.read(|machine| machine.entities().record_bytes(slot as u32).map(<[u8]>::to_vec))
            .map(|bytes| bytes.to_vec())
            .map_err(qc_error)
    }

    fn staging_snapshot(&self) -> Vec<u8> {
        self.read(|machine| {
            let bytes = machine.globals().bytes();
            let end = 112.min(bytes.len());
            bytes[4.min(end)..end].to_vec()
        })
    }

    fn restore_staging(&self, bytes: &[u8]) {
        self.write(|machine| {
            let globals = machine.globals_mut().bytes_mut();
            let end = (4 + bytes.len()).min(globals.len()).min(112);
            if 4 < end {
                globals[4..end].copy_from_slice(&bytes[..end - 4]);
            }
        });
    }

    fn global_range(&self, offset: usize, words: usize) -> Result<Vec<u8>, QcError> {
        self.read(|machine| {
            let bytes = machine.globals().bytes();
            let start = offset * 4;
            let end = start + words * 4;
            if end > bytes.len() {
                return Err(QcError::program("global range out of bounds", "progs.dat"));
            }
            Ok(bytes[start..end].to_vec())
        })
    }

    fn set_global_range(&self, offset: usize, bytes: &[u8]) {
        self.write(|machine| {
            let globals = machine.globals_mut().bytes_mut();
            let start = offset * 4;
            let end = (start + bytes.len()).min(globals.len());
            if start < globals.len() {
                globals[start..end].copy_from_slice(&bytes[..end - start]);
            }
        });
    }

    fn execute(&self, function: usize, argc: usize) -> Result<(), QcError> {
        self.write(|machine| {
            let _guard = ExecutionGuard::enter(&mut *machine);
            machine.execute(function, argc)
        })
        .map_err(qc_error)
    }

    fn execute_region(
        &self,
        region: &qa_content::q1::quakec::qc_view::QcInlineRegion,
        argc: usize,
    ) -> Result<f64, QcError> {
        let guest = content_inline_region(region);
        self.write(|machine| {
            let _guard = ExecutionGuard::enter(&mut *machine);
            machine.execute_region(&guest, argc)
        })
        .map(f64::from)
        .map_err(qc_error)
    }
}

/// Convert a content inline region into the guest shape.
fn content_inline_region(region: &qa_content::q1::quakec::qc_view::QcInlineRegion) -> QcInlineRegion {
    let standalone = region
        .standalone
        .as_ref()
        .map(|standalone| qa_guest::qc::machine::QcInlineStandalone {
            saved: standalone.saved,
            scope: match standalone.scope {
                qa_content::q1::quakec::qc_view::StandaloneScope::Frame => qa_guest::qc::machine::QcInlineScope::Frame,
                qa_content::q1::quakec::qc_view::StandaloneScope::Global => {
                    qa_guest::qc::machine::QcInlineScope::Global
                }
            },
        });
    QcInlineRegion {
        function_index: region.function_index,
        entry: region.entry,
        exit: region.exit,
        replaceable: region.replaceable,
        standalone,
    }
}

/// Content function execution over a directly borrowed guest machine.
struct BridgeExecution<'m> {
    machine: RefCell<&'m mut QcMachine>,
    source: Rc<String>,
    digest: Rc<String>,
    numeric: NumericOps,
    decision: RefCell<Option<QcBoundaryAction>>,
}

impl<'m> BridgeExecution<'m> {
    fn new(machine: &'m mut QcMachine, source: Rc<String>, digest: Rc<String>, numeric: NumericOps) -> Self {
        Self {
            machine: RefCell::new(machine),
            source,
            digest,
            numeric,
            decision: RefCell::new(None),
        }
    }

    fn decision(&self) -> QcBoundaryAction {
        self.decision.borrow_mut().take().unwrap_or(QcBoundaryAction::Enter)
    }
}

impl qa_content::q1::quakec::qc_view::QcFunctionExecution for BridgeExecution<'_> {
    fn run(&self, prepare: Option<qa_content::q1::quakec::qc_view::QcPrepareHook<'_>>) -> Result<(), QcError> {
        if let Some(prepare) = prepare {
            let mut machine = self.machine.borrow_mut();
            let view = DirectMachineView {
                machine: RefCell::new(&mut **machine),
                source: Rc::clone(&self.source),
                digest: Rc::clone(&self.digest),
                numeric: self.numeric,
            };
            prepare(&view)?;
        }
        *self.decision.borrow_mut() = Some(QcBoundaryAction::Enter);
        Ok(())
    }

    fn skip(&self, words: [i32; 3]) {
        *self.decision.borrow_mut() = Some(QcBoundaryAction::Skip { return_words: words });
    }

    fn cancel_owner(&self) -> u64 {
        BRIDGE_OWNER
    }
}

/// Guest function boundary over one composed content boundary.
struct FunctionBridge {
    content: qa_content::q1::quakec::qc_view::QcFunctionBoundary<'static>,
    source: Rc<String>,
    digest: Rc<String>,
    numeric: NumericOps,
    hook_error: Rc<RefCell<Option<GuestError>>>,
}

impl GuestFunctionBoundary for FunctionBridge {
    fn contains(&self, function_index: usize) -> bool {
        self.content.functions.contains(&function_index)
    }

    fn run(&self, machine: &mut QcMachine, call: &QcCallSite) -> Result<QcBoundaryAction, GuestError> {
        let content_call = qa_content::q1::quakec::qc_view::QcCallSite {
            function_index: call.function_index,
            caller: call.caller,
            statement: call.statement,
        };
        let (result, decision) = {
            let execution = BridgeExecution::new(
                &mut *machine,
                Rc::clone(&self.source),
                Rc::clone(&self.digest),
                self.numeric,
            );
            let result = (self.content.run)(&content_call, &execution);
            (result, execution.decision())
        };
        if let Some(error) = self.hook_error.borrow_mut().take() {
            return Err(machine.fail(error.to_string()));
        }
        match result {
            Ok(()) => Ok(decision),
            Err(QcError::Cancelled { owner, words }) if owner == BRIDGE_OWNER => {
                machine.cancel_source_function(words)?;
                Ok(QcBoundaryAction::Skip { return_words: words })
            }
            Err(error) => Err(machine.fail(error.to_string())),
        }
    }
}

/// Content inline continuation recording the guest decision.
struct BridgeContinuation {
    decision: RefCell<Option<QcInlineAction>>,
}

impl qa_content::q1::quakec::qc_view::QcInlineContinuation for BridgeContinuation {
    fn run(&self) -> Result<(), QcError> {
        *self.decision.borrow_mut() = Some(QcInlineAction::Run);
        Ok(())
    }

    fn skip_to_join(&self) -> Result<(), QcError> {
        *self.decision.borrow_mut() = Some(QcInlineAction::SkipToJoin);
        Ok(())
    }
}

/// Guest inline boundary over one composed content boundary.
struct InlineBridge {
    content: qa_content::q1::quakec::qc_view::QcInlineBoundary<'static>,
    regions: Vec<QcInlineRegion>,
    hook_error: Rc<RefCell<Option<GuestError>>>,
}

impl InlineBridge {
    fn new(
        content: qa_content::q1::quakec::qc_view::QcInlineBoundary<'static>,
        hook_error: Rc<RefCell<Option<GuestError>>>,
    ) -> Self {
        let regions = content.regions.iter().map(content_inline_region).collect();
        Self {
            content,
            regions,
            hook_error,
        }
    }
}

impl GuestInlineBoundary for InlineBridge {
    fn regions(&self) -> &[QcInlineRegion] {
        &self.regions
    }

    fn run(&self, machine: &mut QcMachine, region: &QcInlineRegion) -> Result<QcInlineAction, GuestError> {
        let content = self
            .content
            .regions
            .iter()
            .find(|candidate| {
                candidate.function_index == region.function_index
                    && candidate.entry == region.entry
                    && candidate.exit == region.exit
            })
            .ok_or_else(|| machine.fail("unknown QC inline region"))?;
        let continuation = BridgeContinuation {
            decision: RefCell::new(None),
        };
        (self.content.run)(content, &continuation).map_err(|error| machine.fail(error.to_string()))?;
        if let Some(error) = self.hook_error.borrow_mut().take() {
            return Err(machine.fail(error.to_string()));
        }
        let decision = continuation.decision.borrow_mut().take().unwrap_or(QcInlineAction::Run);
        Ok(decision)
    }
}

/// Narrow pusher projection handed to the host physics.
///
/// Donor `Projection = Pick<Q1PusherServices, "read" | "write" | "link" | "blocked">` from
/// `src/compat/qc/pusher-host.ts`.
/// Host cheat name (donor `hostCheat` name union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCCheat {
    /// God mode toggle.
    God,
    /// Notarget toggle.
    Notarget,
    /// Noclip toggle.
    Noclip,
    /// Fly toggle.
    Fly,
    /// Give items.
    Give,
}

/// Client powerup timer (donor `clientPowerupExpires` powerup union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCPowerup {
    /// Quad damage.
    Quad,
    /// Pentagram of protection.
    Invulnerability,
    /// Ring of shadows.
    Invisibility,
    /// Biosuit.
    Suit,
}

/// Mixed-client post-think phase (donor `mixedClientPostThink` postThink union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QuakeCPostThink {
    /// Run `PlayerPostThink` immediately.
    #[default]
    Immediate,
    /// Defer `PlayerPostThink` to the native command loop.
    Deferred,
}

/// Movement profile family (donor `MovementProfile["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCMovementKind {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
    /// Quake II classic.
    Q2Classic,
    /// Quake II rerelease.
    Q2Rerelease,
    /// Quake III.
    Q3,
}

/// Client equipment snapshot (donor `clientEquipment` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuakeCEquipment {
    /// Maximum health.
    pub max_health: f64,
    /// Quad expiry in source seconds.
    pub quad_until: f64,
}

/// Combat target flags (donor `weaponTarget` return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuakeCWeaponTarget {
    /// Monster flag bit is set.
    pub monster: bool,
    /// Takes aimed damage.
    pub aimed_damage: bool,
}

/// Selected spawn point (donor `clientSpawnPoint` return).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuakeCSpawnPoint {
    /// Spawn origin.
    pub origin: Vec3,
    /// Spawn angles.
    pub angles: Vec3,
}

/// Client HUD snapshot (donor `clientUi` return).
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCClientUi {
    /// Weapon HUD.
    pub weapon: QuakeCWeaponUi,
    /// Active powerup timers.
    pub powerups: Vec<ActivePowerupTimer>,
}

pub trait QuakeCSourcePusherProjection {
    /// Read one pusher entity.
    fn read(&self, actor: &ActorId) -> Option<qa_world::movement::q1::types::Q1PhysicsEntity>;
    /// Write one pusher entity.
    fn write(&mut self, entity: &qa_world::movement::q1::types::Q1PhysicsEntity);
    /// Link one pusher.
    fn link(&mut self, actor: &OwnedActor, touch_triggers: bool);
    /// Run one pusher blocked callback.
    fn blocked(&mut self, pusher: &OwnedActor, obstacle: &ActorId);
}

/// Synchronous QuakeC source.
///
/// Donor `QuakeCSource` from `quakec-source.ts`: a prepared program plus session hosts driving
/// one synchronous id1 world. Construction returns `Rc<Self>` because machine-time admission
/// (`options.admit(actor, slot, source)`) hands the host a borrow of the source itself; every
/// method takes `&self` and mutates through the shared state.
pub struct QuakeCSource<P: qa_content::contract::OriginalPickupAdmission + 'static> {
    /// Prepared program and resources.
    prepared: PreparedQuakeCSource,
    /// Shared mutable state (hosts live here so `&self` methods can reach them).
    shared: Rc<RefCell<Shared<P>>>,
    /// Guest machine.
    machine: Rc<RefCell<QcMachine>>,
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Open a synchronous source.
    pub fn new(prepared: PreparedQuakeCSource, options: QuakeCSourceOptions<P>) -> Result<Rc<Self>, QuakeCSourceError> {
        let mut cache = Id1ProgramCache::default();
        let probe_views = ContentProgramViews::build(&prepared.program);
        let probe = probe_views.view(&prepared.program);
        let declared_probe = declared_damage_call(&prepared)?;
        let binding = id1_program_binding(&mut cache, &probe, declared_probe.as_ref())?;
        let quakeworld = matches!(binding.kind, qa_content::q1::quakec::id1_program::Id1Kind::Quakeworld);
        let kind = if quakeworld {
            QuakeCSourceKind::Quakeworld
        } else {
            QuakeCSourceKind::Netquake
        };
        let weapons = native_weapons(&prepared.program);
        let wanted = prepared.execution.artifact.identity.canonical();
        if !options.recipe.execution.iter().any(|entry| {
            matches!(
                entry.implementation,
                crate::persistence::recipe::ExecutionImplementation::Quakec { .. }
            ) && entry.owner.provider
                == format!(
                    "{}:{}",
                    prepared.execution.owner.provider.namespace,
                    prepared.execution.owner.provider.name
                )
                && entry.owner.content == prepared.execution.owner.content
                && matches!(&entry.implementation, crate::persistence::recipe::ExecutionImplementation::Quakec { artifact } if artifact.identity == wanted)
        }) {
            return Err(QuakeCSourceError::Invalid(
                "QC source differs from selected execution".to_string(),
            ));
        }
        if !options.initial_source_time_seconds.is_finite()
            || options.initial_source_time_seconds < 0.0
            || options.max_clients < 1
            || options.max_clients >= 2048
        {
            return Err(QuakeCSourceError::Invalid(
                "Invalid dedicated QC source timing or reserved clients".to_string(),
            ));
        }
        if quakeworld && options.max_clients > 32 {
            return Err(QuakeCSourceError::Invalid(
                "Native QuakeWorld supports at most 32 clients".to_string(),
            ));
        }
        let reserved_client_slots = if quakeworld { 32 } else { options.max_clients };
        let current_time = options.initial_source_time_seconds;
        let spawning: Rc<RefCell<bool>> = Rc::new(RefCell::new(true));
        let hook_error: Rc<RefCell<Option<GuestError>>> = Rc::new(RefCell::new(None));
        let fields = Rc::new(RefCell::new(FieldTable::new()));
        let provider = prepared.execution.owner.provider.clone();

        let layout = classic_qc_entity_layout(&prepared.program);
        let capacity = if quakeworld { 768 } else { 2048 };
        let entities = QcEntityMemory::new(layout, capacity, reserved_client_slots + 1)?;
        let storage = Rc::new(RefCell::new(create_qc_source_slot_storage(
            qa_guest::qc::entity_host::QcEdictMetadataLayout {
                free_offset_bytes: 0,
                free_time_offset_bytes: if quakeworld { 100 } else { 92 },
            },
            layout.variables_offset_bytes,
            &format!("{}:{}", provider.namespace, provider.name),
            capacity,
        )?));

        if options.restore.is_none() {
            let mut actors = options.actors.borrow_mut();
            for slot in 0..=reserved_client_slots {
                let kind = if slot == 0 {
                    QuakeCSlotKind::Worldspawn
                } else {
                    QuakeCSlotKind::ReservedClient
                };
                actors.allocate_at_source(&provider, slot, kind.definition());
            }
        }

        let numeric =
            NumericOps::select(Q1_DONOR_PROFILE).map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
        let server_spawning = Rc::clone(&spawning);
        let machine = Rc::new(RefCell::new(QcMachine::new(QcMachineOptions::new(
            prepared.program.clone(),
            numeric,
            entities.clone(),
            QcBuiltinRegistry {
                numbered: std::collections::HashMap::new(),
                named: std::collections::HashMap::new(),
            },
            Rc::new(move || !*server_spawning.borrow()),
        ))?));

        let core: &'static SourceViewCore = Box::leak(Box::new(SourceViewCore {
            machine_view: MachineView {
                machine: Rc::clone(&machine),
                source: prepared.program.source.clone(),
                digest: format!(
                    "{}:{}",
                    prepared.program.digest.algorithm, prepared.program.digest.value
                ),
                numeric,
                hook_error: Rc::clone(&hook_error),
            },
            registry: RegistryView {
                actors: Rc::clone(&options.actors),
            },
            slots: SlotsView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                storage: Rc::clone(&storage),
            },
            views: ContentProgramViews::build(&prepared.program),
            program_source: prepared.program.source.clone(),
            initial_globals: prepared.program.initial_globals.clone(),
            digest: format!(
                "{}:{}",
                prepared.program.digest.algorithm, prepared.program.digest.value
            ),
        }));
        let program_api = match prepared.program.api {
            qa_guest::qc::program::QuakeCApi::Netquake => ContentApi::Netquake,
            qa_guest::qc::program::QuakeCApi::Quakeworld => ContentApi::Quakeworld,
        };
        let program_view: &'static qa_content::q1::quakec::qc_view::QcProgramView<'static> = Box::leak(Box::new(
            core.views
                .view_split(&core.program_source, program_api, &core.initial_globals, &core.digest),
        ));
        let host_source = || QcHostSource {
            program: program_view,
            actors: &core.registry as &dyn QcActorRegistry,
            slots: &core.slots as &dyn QcActorSlots,
        };
        let machine_fn =
            || -> MachineFn<'static> { Box::new(move || &core.machine_view as &'static (dyn QcMachineView + 'static)) };

        let attacks: &'static Id1SynchronousAttacks<'static> = Box::leak(Box::new(Id1SynchronousAttacks::new(
            host_source(),
            machine_fn(),
            declared_probe.as_ref(),
        )?));
        let projectiles: &'static Id1ProjectileAttacks<'static> = Box::leak(Box::new(Id1ProjectileAttacks::new(
            host_source(),
            machine_fn(),
            declared_probe.as_ref(),
        )?));
        let environment: &'static Id1Environment<'static> = Box::leak(Box::new(Id1Environment::new(
            host_source(),
            machine_fn(),
            declared_probe.as_ref(),
        )?));

        if let Some(combat) = &prepared.combat_declaration {
            validate_qc_mod_combat(&GuestProgramView::new(&prepared.program), &runtime_combat(combat)?)?;
        }

        let machine_view: &'static MachineView = &core.machine_view;
        let mut options = options;
        let restore = options.restore.take();
        let random: QcSharedRandom = Rc::new(RefCell::new(std::mem::replace(
            &mut options.random,
            super::random::SourceRandom::new(0),
        )));
        let incoming_damage = Rc::new(RefCell::new(Vec::<IncomingDamage>::new()));
        let active_clients = Rc::new(RefCell::new(std::collections::HashSet::new()));
        let pending_weapons = Rc::new(RefCell::new(std::collections::HashMap::new()));
        let admissions: Rc<RefCell<Vec<(ActorId, usize)>>> = Rc::new(RefCell::new(Vec::new()));
        let physics_callback = Rc::new(RefCell::new(None));
        let models = Rc::new(RefCell::new(std::collections::HashMap::<String, SourceModel>::new()));
        let model_total = options.world.borrow().model_count();
        {
            let mut models = models.borrow_mut();
            models.insert(
                options.recipe.map.geometry.requested_path.clone(),
                SourceModel {
                    index: 1,
                    bounds: options.scene.borrow().model_bounds(0),
                },
            );
            for model in 1..model_total {
                models.insert(
                    format!("*{model}"),
                    SourceModel {
                        index: model as i32 + 1,
                        bounds: options.scene.borrow().model_bounds(model),
                    },
                );
            }
        }

        let dialect = if quakeworld {
            Dialect::Q1Quakeworld
        } else {
            Dialect::Q1Netquake
        };
        let fresh_registry = options.source_registry.is_none();
        let mut cvars = options
            .source_registry
            .take()
            .unwrap_or_else(|| CvarRegistry::new(dialect));
        if fresh_registry {
            let deathmatch = options.mode == QuakeCSourceMode::Deathmatch;
            let coop = options.mode == QuakeCSourceMode::Coop;
            let entries = [
                ("skill", options.skill.to_string()),
                ("deathmatch", if deathmatch { "1" } else { "0" }.to_string()),
                ("coop", if coop { "1" } else { "0" }.to_string()),
                ("teamplay", "0".to_string()),
                ("sv_cheats", "0".to_string()),
                ("sv_aim", if quakeworld { "2" } else { "0.93" }.to_string()),
                ("sv_gravity", "800".to_string()),
                ("sv_maxspeed", "320".to_string()),
                ("samelevel", "0".to_string()),
                ("timelimit", "0".to_string()),
                ("fraglimit", "0".to_string()),
                ("gamecfg", "0".to_string()),
                ("registered", "1".to_string()),
            ];
            for (name, value) in entries {
                cvars
                    .register(name, &value, 0)
                    .map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
            }
        }
        if cvars.get("developer").is_none() {
            cvars
                .register("developer", "0", 0)
                .map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
        }
        if quakeworld {
            register_quake_world_engine_cvars(&mut cvars)
                .map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
        }
        let cvars = Rc::new(RefCell::new(cvars));

        let router_state = Rc::new(RefCell::new(RouterState {
            kind,
            spawning: true,
            local_messages_started: false,
            netquake_wire_attached: false,
            max_clients: options.max_clients,
            overflow_bytes: 0,
            signon: Vec::new(),
            routed: Vec::new(),
            netquake_signon: Vec::new(),
            netquake_signon_views: Vec::new(),
            netquake_routed: Vec::new(),
            local_messages: QuakeCLocalMessages::new(),
        }));
        let router = {
            let cvars = Rc::clone(&cvars);
            let actors = Rc::clone(&options.actors);
            let provider = provider.clone();
            QuakeCMessageRouter {
                state: Rc::clone(&router_state),
                cvar_phs: Rc::new(move || cvars.borrow().variable_value("sv_phs") != 0.0),
                is_reserved_client: Rc::new(move |actor: &ActorId| {
                    actors
                        .borrow()
                        .source_of(actor)
                        .is_some_and(|(found, slot)| found == provider && slot >= 1 && slot <= reserved_client_slots)
                }),
            }
        };
        let messages = QcBroadcastMessages::new(Some(router), quakeworld, if quakeworld { 28 } else { 15 }, 0)?;

        let provider_name: &'static str =
            Box::leak(format!("{}:{}", provider.namespace, provider.name).into_boxed_str());
        let borrowed = Rc::new(RefCell::new(QcBorrowedActors::new(
            BorrowedHostView {
                actors: Rc::clone(&options.actors),
                physics: Rc::clone(&options.physics),
                machine: machine_view,
                provider: provider.clone(),
                provider_name,
                foreign_classname: options.foreign_classname.clone(),
            },
            BorrowedSlotPool::new(
                capacity,
                reserved_client_slots + 1,
                capacity,
                ReusePolicy::AfterSeconds(0.5),
            ),
        )));
        for slot in 0..=reserved_client_slots {
            borrowed.borrow_mut().pool_mut().mark_source(slot)?;
        }
        options.actors.borrow_mut().on_release({
            let active_clients = Rc::clone(&active_clients);
            let router = Rc::clone(&router_state);
            let pending_weapons = Rc::clone(&pending_weapons);
            let borrowed = Rc::clone(&borrowed);
            Box::new(move |actor: &OwnedActor| {
                active_clients.borrow_mut().remove(actor.id());
                router.borrow_mut().local_messages.retire(actor.id());
                pending_weapons.borrow_mut().remove(actor.id());
                borrowed.borrow_mut().released(actor.id());
            })
        });

        let combat_authority: &'static dyn GameplayAuthority = &**Box::leak(Box::new(Rc::clone(&options.combat)));
        let damage_request_hook = Rc::clone(&options.damage_request);
        let resolve_request = {
            let incoming = Rc::clone(&incoming_damage);
            Box::new(move |call: &Id1DamageCall| {
                if let Some(pending) = incoming.borrow_mut().last_mut() {
                    if !pending.entered {
                        pending.entered = true;
                        return pending.request.clone();
                    }
                }
                damage_request_hook(call)
            }) as Box<dyn Fn(&Id1DamageCall) -> qa_content::q1::foundation::gameplay::DamageRequest>
        };
        let projection: &'static DamageProjectionView = Box::leak(Box::new(DamageProjectionView {
            actors: Rc::clone(&options.actors),
            provider: provider.clone(),
            machine: machine_view,
            borrowed: Rc::clone(&borrowed),
            fields: Rc::clone(&fields),
            damage_allowed: options.damage_allowed.clone(),
            incoming: Rc::clone(&incoming_damage),
        }));
        let declared_armor = prepared
            .combat_declaration
            .as_ref()
            .and_then(|combat| combat.armor_stage.as_ref())
            .map(content_armor_stage)
            .transpose()?;
        let declared_damage = declared_damage_call(&prepared)?;
        let declared_scale_region = prepared
            .combat_declaration
            .as_ref()
            .and_then(|combat| combat.damage_scale.as_ref())
            .map(content_damage_scale)
            .transpose()?;
        let declared_scale = declared_damage
            .as_ref()
            .zip(declared_scale_region.as_ref())
            .map(|(call, scale)| (call as &_, scale as &_));
        let damage: &'static Id1DamageBinding<'static> = Box::leak(Box::new(Id1DamageBinding::new(
            host_source(),
            combat_authority,
            machine_fn(),
            resolve_request,
            Some(projection),
            declared_armor.as_ref(),
            declared_damage.as_ref(),
            declared_scale,
            &mut cache,
        )?));

        let admission_holder: &'static Rc<P> = Box::leak(Box::new(Rc::clone(&options.pickups)));
        let admission: &'static P = admission_holder;
        let primary_selected = options.primary_weapon_selected.clone().map(|hook| {
            Box::new(move |owned: &OwnedActor| hook(owned.id()))
                as qa_content::q1::quakec::id1_pickups::PrimaryWeaponSelectedFn
        });
        let owns_weapon = options.owns_weapon.clone().map(|hook| {
            Box::new(move |owned: &OwnedActor, item: &ItemId| hook(owned.id(), item))
                as qa_content::q1::quakec::id1_pickups::OwnsWeaponFn
        });
        let policy = options.pickup_policy.clone().map(|policy| {
            let leaked: &'static Rc<dyn QcPickupPolicy> = Box::leak(Box::new(policy));
            Box::new(move || Some(&**leaked as &dyn QcPickupPolicy))
                as qa_content::q1::quakec::id1_pickups::PolicyFn<'static>
        });
        let pickups: &'static Id1PickupBinding<'static, P> = Box::leak(Box::new(Id1PickupBinding::new(
            host_source(),
            admission,
            machine_fn(),
            primary_selected,
            owns_weapon,
            policy,
            prepared.declared_pickups.clone(),
        )?));

        let weapon_stage_declared = prepared_quake_c_weapon_stage(&prepared)?;
        if options.primary_weapon_selected.is_some() && weapon_stage_declared.is_none() {
            return Err(QuakeCSourceError::Invalid(
                "QC artifact has no qualified primary weapon stage".to_string(),
            ));
        }
        let spawn_call = weapon_stage_declared
            .as_ref()
            .and_then(|stage| Some(stage.client.as_ref()?.spawn.clone()));
        let spawn_function = spawn_call.as_ref().map(|call| call.function_index);
        if options.client_spawned.is_some() && spawn_function.is_none() {
            return Err(QuakeCSourceError::Invalid(
                "QC artifact has no qualified client spawn stage".to_string(),
            ));
        }
        let weapon_stage: Option<&'static QcWeaponStageBinding<'static>> = weapon_stage_declared
            .map(|stage| {
                let actors = Rc::clone(&options.actors);
                let active = Rc::clone(&active_clients);
                let primary = options.primary_weapon_selected.clone();
                let provider = provider.clone();
                let selected = Box::new(move |reference: i32| {
                    let Ok(slot) = machine_view.entity_slot(reference) else {
                        return true;
                    };
                    let Some(actor) = actors.borrow().at_source(&provider, slot) else {
                        return true;
                    };
                    if !active.borrow().contains(actor.id()) {
                        return true;
                    }
                    primary.as_ref().is_none_or(|hook| hook(actor.id()))
                });
                Ok::<_, QuakeCSourceError>(&*Box::leak(Box::new(QcWeaponStageBinding::new(
                    stage,
                    machine_fn(),
                    selected,
                ))))
            })
            .transpose()?;

        let source_functions =
            pickups.compose_functions(projectiles.compose(attacks.compose(damage.function_boundary())))?;
        let regions = pickups.compose_regions(damage.inline_boundary());
        let functions = match (options.client_spawned.clone(), spawn_call, spawn_function) {
            (Some(spawned), Some(spawn_call), Some(function)) => {
                let mut combined = source_functions.functions.clone();
                combined.insert(function);
                let inner_set = Rc::new(source_functions.functions.clone());
                let inner_run = source_functions.run;
                let actors = Rc::clone(&options.actors);
                let active = Rc::clone(&active_clients);
                let provider = provider.clone();
                qa_content::q1::quakec::qc_view::QcFunctionBoundary {
                    functions: combined,
                    run: Box::new(move |call, execute| {
                        if call.function_index != function {
                            return inner_run(call, execute);
                        }
                        let reference = qc_client_stage_self(machine_view, &spawn_call)?;
                        let slot = machine_view.entity_slot(reference)?;
                        let actor = actors.borrow().at_source(&provider, slot);
                        if inner_set.contains(&call.function_index) {
                            inner_run(call, execute)?;
                        } else {
                            execute.run(None)?;
                        }
                        if let Some(actor) = actor {
                            if actors.borrow().resolve_owned(actor.id()).as_ref() == Some(&actor)
                                && active.borrow().contains(actor.id())
                            {
                                spawned(&actor);
                            }
                        }
                        Ok(())
                    }),
                }
            }
            _ => source_functions,
        };

        let storage_count = reserved_client_slots + 1;
        storage.borrow_mut().set_count(storage_count)?;
        let layout = source_field_layout(&prepared.program);
        let world = QcWorldHost::new(
            WorldSlots {
                actors: Rc::clone(&options.actors),
                storage: Rc::clone(&storage),
                provider: provider.clone(),
                borrowed: Rc::clone(&borrowed),
                fields: Rc::clone(&fields),
                layout: layout.clone(),
                reserved: reserved_client_slots,
                foreign: {
                    let borrowed = Rc::clone(&borrowed);
                    let fields = Rc::clone(&fields);
                    Rc::new(move |actor: &ActorId| {
                        borrowed
                            .borrow_mut()
                            .reference(&mut fields.borrow_mut(), actor)
                            .unwrap_or(0)
                    })
                },
            },
            WorldBodies {
                physics: Rc::clone(&options.physics),
            },
            Rc::new({
                let actors = Rc::clone(&options.actors);
                move |saved: &SavedActorId| Some(actors.borrow().reference_saved(*saved))
            }),
        );
        let client_slots = ClientSlotsView {
            actors: Rc::clone(&options.actors),
            provider: provider.clone(),
            entity_count: storage_count,
        };
        let clients = QcClientHost::new(
            &client_slots,
            VisibilityView {
                scene: Rc::clone(&options.scene),
            },
            reserved_client_slots,
        )?;
        let actor_state = QcActorState::new(
            ActorLookupView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                machine: machine_view,
                reserved_client_slots,
            },
            false,
        );
        let movement = MovementBindings::new(
            MovementWorldView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                storage: Rc::clone(&storage),
                physics: Rc::clone(&options.physics),
                machine: machine_view,
            },
            SceneView {
                scene: Rc::clone(&options.scene),
            },
            MovementBodiesView {
                physics: Rc::clone(&options.physics),
            },
            RandomView {
                random: Rc::clone(&random),
            },
        );
        let spatial = qa_guest::qc::spatial_host::SpatialBindings::new(
            SpatialWorldView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                storage: Rc::clone(&storage),
                physics: Rc::clone(&options.physics),
                machine: machine_view,
            },
            SceneView {
                scene: Rc::clone(&options.scene),
            },
            SpatialModelsView {
                models: Rc::clone(&models),
            },
        );
        let pusher = qa_guest::qc::pusher_host::QcPusherServices::new(
            PusherWorldView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                physics: Rc::clone(&options.physics),
                machine: machine_view,
            },
            PusherBodiesView {
                physics: Rc::clone(&options.physics),
            },
            ForeignPusherView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                physics: Rc::clone(&options.physics),
            },
            PusherPhysicsView {
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
                physics: Rc::clone(&options.physics),
                machine: machine_view,
                callback: Rc::clone(&physics_callback),
            },
            PusherInvokerView {
                machine: machine_view,
                actors: Rc::clone(&options.actors),
                provider: provider.clone(),
            },
            if quakeworld {
                ApiKind::QuakeWorld
            } else {
                ApiKind::NetQuake
            },
            current_time as f32,
        );
        let projection = Rc::new(SourcePusherProjection {
            actors: Rc::clone(&options.actors),
            provider: provider.clone(),
            physics: Rc::clone(&options.physics),
            machine: machine_view,
            callback: Rc::clone(&physics_callback),
            hook_error: Rc::clone(&hook_error),
            quakeworld,
        });
        let physical = options.physics.borrow_mut().q1_pusher_services(projection);

        let shared = Rc::new(RefCell::new(Shared {
            options,
            kind,
            reserved_client_slots,
            current_time,
            change_level_issued: false,
            active_clients,
            weapons,
            pending_weapons,
            user_info: std::collections::HashMap::new(),
            spawn_parameters: std::collections::HashMap::new(),
            client_identities: std::collections::HashMap::new(),
            original_save_restored: false,
            original_save_extension_text: String::new(),
            spectator_slots: std::collections::HashSet::new(),
            prepared_clients: std::collections::HashSet::new(),
            frag_records: Vec::new(),
            router: router_state,
            physics_callback,
            spawning,
            incoming_damage,
            models,
            precached: std::collections::HashMap::new(),
            model_count: model_total as i32 + 1,
            sound_count: 1,
            storage,
            fields,
            cvars,
            attacks,
            projectiles,
            environment,
            damage,
            pickups,
            weapon_stage,
            messages,
            world,
            clients,
            borrowed,
            actor_state,
            movement,
            spatial,
            pusher,
            numeric,
            random,
            physical,
            layout,
            hook_error: Rc::clone(&hook_error),
            admissions: Rc::clone(&admissions),
            program_view,
            machine_view,
            source: Weak::new(),
        }));

        let host = builtin_host(Rc::downgrade(&shared));
        let storage_free = Rc::clone(&shared.borrow().storage);
        let mut machine_options = QcMachineOptions::new(
            prepared.program.clone(),
            numeric,
            entities,
            create_qc_builtins(QcBuiltinServices {
                kind: if quakeworld {
                    QcHostKind::Quakeworld
                } else {
                    QcHostKind::Netquake
                },
                random: Some(random_handle(&shared)),
                is_free_entity: Some(Rc::new(move |slot: u32| storage_free.borrow().is_free(slot as usize))),
                prepare_entities: None,
                host: Some(host),
                extensions: std::collections::HashSet::new(),
            }),
            Rc::new({
                let spawning = Rc::clone(&shared.borrow().spawning);
                move || !*spawning.borrow()
            }),
        );
        let weapon_stage = shared.borrow().weapon_stage;
        let functions = match weapon_stage {
            Some(stage) => stage.compose_functions(functions)?,
            None => functions,
        };
        let regions = match weapon_stage {
            Some(stage) => stage.compose_regions(regions)?,
            None => regions,
        };
        machine_options.function_boundary = Some(Rc::new(FunctionBridge {
            content: functions,
            source: Rc::new(core.program_source.clone()),
            digest: Rc::new(core.digest.clone()),
            numeric: shared.borrow().numeric,
            hook_error: Rc::clone(&hook_error),
        }));
        machine_options.inline_boundary = Some(Rc::new(InlineBridge::new(regions, Rc::clone(&hook_error))));
        machine_options.observe_call = Some({
            let hook_error = Rc::clone(&hook_error);
            Rc::new(move |call: &QcCallSite| {
                let content_call = qa_content::q1::quakec::qc_view::QcCallSite {
                    function_index: call.function_index,
                    caller: call.caller,
                    statement: call.statement,
                };
                if let Err(error) = pickups.validate() {
                    *hook_error.borrow_mut() = Some(GuestError::invalid(error.to_string()));
                    return;
                }
                if let Err(error) = damage.observe_call(&content_call) {
                    *hook_error.borrow_mut() = Some(GuestError::invalid(error.to_string()));
                }
            })
        });
        machine_options.validate_entity_access = Some({
            let borrowed = Rc::clone(&shared.borrow().borrowed);
            let fields = Rc::clone(&shared.borrow().fields);
            Rc::new(move |reference: i32, word: usize, words: u8, access: QcAccessKind| {
                let kind = match access {
                    QcAccessKind::Read => qa_guest::qc::borrowed_actors::AccessKind::Read,
                    QcAccessKind::Write => qa_guest::qc::borrowed_actors::AccessKind::Write,
                };
                borrowed
                    .borrow_mut()
                    .access(&mut fields.borrow_mut(), reference, word, words as usize, kind)
            })
        });
        machine_options.observe_entity_store = Some({
            let hook_error = Rc::clone(&hook_error);
            Rc::new(move |store: &QcEntityStoreObservation| {
                let content_store = content_observation(store);
                for outcome in [
                    pickups.observe_store(&content_store),
                    projectiles.observe_store(&content_store),
                    damage.observe_entity_store(&content_store),
                ] {
                    if let Err(error) = outcome {
                        *hook_error.borrow_mut() = Some(GuestError::invalid(error.to_string()));
                        return;
                    }
                }
            })
        });
        *machine.borrow_mut() = QcMachine::new(machine_options)?;

        let source = Rc::new(Self {
            prepared,
            shared,
            machine,
        });
        source.shared.borrow_mut().source = Rc::downgrade(&source);
        let admissions = Rc::clone(&source.shared.borrow().admissions);
        source
            .shared
            .borrow_mut()
            .world
            .set_admit_hook(move |actor: &ActorId, slot: usize| {
                admissions.borrow_mut().push((actor.clone(), slot));
            });
        if let Some(restore) = restore {
            source.restore_host(&restore.checkpoint, &restore.clients)?;
        } else {
            let reserved_slots = source.shared.borrow().reserved_client_slots;
            for slot in 0..=reserved_slots {
                source.bind_reserved_slot(slot)?;
            }
        }
        Ok(source)
    }
}

/// Map a value/codec failure into a guest error.
fn value_error(error: impl ToString) -> GuestError {
    GuestError::invalid(error.to_string())
}

/// Format a source kind for checkpoints (`kind`).
fn checkpoint_kind(kind: QuakeCSourceKind) -> &'static str {
    match kind {
        QuakeCSourceKind::Netquake => "netquake",
        QuakeCSourceKind::Quakeworld => "quakeworld",
    }
}

/// Format a message dialect for checkpoints (`messageDialect`).
fn checkpoint_dialect(dialect: RereleaseMessages) -> &'static str {
    match dialect {
        RereleaseMessages::KnownRetail => "known-retail",
        RereleaseMessages::Quake1ReTsPrivate => "quake-1-re-ts-private",
    }
}

/// Parse a message dialect from a checkpoint (`messageDialect`).
fn restore_dialect(text: &str) -> Result<RereleaseMessages, QuakeCSourceError> {
    match text {
        "known-retail" => Ok(RereleaseMessages::KnownRetail),
        "quake-1-re-ts-private" => Ok(RereleaseMessages::Quake1ReTsPrivate),
        _ => Err(QuakeCSourceError::Invalid("unknown saved message dialect".to_string())),
    }
}

/// Encode a saved actor identity (`{ slot, generation }`).
fn checkpoint_actor(actor: &SavedActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot))),
        ("generation", int(i64::from(actor.generation))),
    ])
}

/// Decode a saved actor identity.
fn restore_saved_actor(reader: ContentReader) -> Result<SavedActorId, QuakeCSourceError> {
    let slot = u32::try_from(reader.field("slot").integer(0)?)
        .map_err(|_| ValueError("saved actor slot exceeds u32".to_string()))?;
    let generation = u32::try_from(reader.field("generation").integer(0)?)
        .map_err(|_| ValueError("saved actor generation exceeds u32".to_string()))?;
    Ok(SavedActorId { slot, generation })
}

/// Encode one staged QuakeWorld entry.
fn checkpoint_entry(entry: &QwEntryCheckpoint) -> SaveJson {
    obj(vec![
        ("bytes", SaveJson::Bytes(entry.bytes.clone())),
        ("actor", entry.actor.as_ref().map_or(SaveJson::Null, checkpoint_actor)),
    ])
}

/// Decode one staged QuakeWorld entry.
fn restore_entry(reader: ContentReader) -> Result<QwEntryCheckpoint, QuakeCSourceError> {
    Ok(QwEntryCheckpoint {
        bytes: reader.field("bytes").bytes()?,
        actor: reader.field("actor").nullable(restore_saved_actor)?,
    })
}

/// Encode staged QuakeWorld entries (`QwEntriesCheckpoint`).
fn checkpoint_entries(entries: &QwEntriesCheckpoint) -> SaveJson {
    obj(vec![
        ("version", int(i64::from(entries.version))),
        ("flags", int(i64::from(entries.flags))),
        ("entries", arr(entries.entries.iter().map(checkpoint_entry).collect())),
    ])
}

/// Decode staged QuakeWorld entries.
fn restore_entries_checkpoint(reader: ContentReader) -> Result<QwEntriesCheckpoint, QuakeCSourceError> {
    Ok(QwEntriesCheckpoint {
        version: u32::try_from(reader.field("version").integer(0)?)
            .map_err(|_| ValueError("saved entries version exceeds u32".to_string()))?,
        flags: u32::try_from(reader.field("flags").integer(0)?)
            .map_err(|_| ValueError("saved entries flags exceed u32".to_string()))?,
        entries: reader.field("entries").list(restore_entry)?,
    })
}

/// Encode a vector (`{ x, y, z }`).
fn checkpoint_vector(value: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
    ])
}

/// Decode a vector.
fn restore_vector(reader: ContentReader) -> Result<Vec3, QuakeCSourceError> {
    Ok(Vec3 {
        x: reader.field("x").finite()? as f32,
        y: reader.field("y").finite()? as f32,
        z: reader.field("z").finite()? as f32,
    })
}

/// Encode a saved destination (`SavedDestination`).
fn checkpoint_destination(destination: &SavedDestination) -> SaveJson {
    match destination {
        SavedDestination::Broadcast { reliable } => {
            obj(vec![("kind", str("broadcast")), ("reliable", boolean(*reliable))])
        }
        SavedDestination::Client { actor } => obj(vec![("kind", str("client")), ("actor", checkpoint_actor(actor))]),
        SavedDestination::Signon => obj(vec![("kind", str("signon"))]),
        SavedDestination::Multicast {
            origin,
            visibility,
            reliable,
        } => obj(vec![
            ("kind", str("multicast")),
            ("origin", checkpoint_vector(*origin)),
            (
                "visibility",
                str(match visibility {
                    VisibilityScope::All => "all",
                    VisibilityScope::Pvs => "pvs",
                    VisibilityScope::Phs => "phs",
                }),
            ),
            ("reliable", boolean(*reliable)),
        ]),
    }
}

/// Decode a saved destination.
fn restore_destination(reader: ContentReader) -> Result<SavedDestination, QuakeCSourceError> {
    let kind = reader.field("kind").string()?;
    match kind.as_str() {
        "broadcast" => Ok(SavedDestination::Broadcast {
            reliable: reader.field("reliable").boolean()?,
        }),
        "client" => Ok(SavedDestination::Client {
            actor: restore_saved_actor(reader.field("actor"))?,
        }),
        "signon" => Ok(SavedDestination::Signon),
        "multicast" => {
            let visibility = reader.field("visibility").string()?;
            Ok(SavedDestination::Multicast {
                origin: restore_vector(reader.field("origin"))?,
                visibility: match visibility.as_str() {
                    "all" => VisibilityScope::All,
                    "pvs" => VisibilityScope::Pvs,
                    "phs" => VisibilityScope::Phs,
                    _ => return Err(QuakeCSourceError::Invalid("unknown saved visibility scope".to_string())),
                },
                reliable: reader.field("reliable").boolean()?,
            })
        }
        _ => Err(QuakeCSourceError::Invalid("unknown saved destination".to_string())),
    }
}

/// Encode one routed buffer (`RoutedCheckpoint`).
fn checkpoint_routed(entry: &RoutedCheckpoint) -> SaveJson {
    obj(vec![
        ("key", str(&entry.key)),
        ("bytes", SaveJson::Bytes(entry.bytes.clone())),
        ("maxsize", int(entry.maxsize as i64)),
        ("allowoverflow", boolean(entry.allowoverflow)),
        ("overflowed", boolean(entry.overflowed)),
        (
            "owners",
            arr(entry
                .owners
                .iter()
                .map(|(offset, actor)| {
                    obj(vec![
                        ("offset", int(i64::from(*offset))),
                        ("actor", actor.as_ref().map_or(SaveJson::Null, checkpoint_actor)),
                    ])
                })
                .collect()),
        ),
        (
            "destination",
            entry
                .destination
                .as_ref()
                .map_or(SaveJson::Null, checkpoint_destination),
        ),
    ])
}

/// Decode one routed buffer.
fn restore_routed(reader: ContentReader) -> Result<RoutedCheckpoint, QuakeCSourceError> {
    Ok(RoutedCheckpoint {
        key: reader.field("key").string()?,
        bytes: reader.field("bytes").bytes()?,
        maxsize: usize::try_from(reader.field("maxsize").integer(0)?)
            .map_err(|_| ValueError("saved routed capacity exceeds usize".to_string()))?,
        allowoverflow: reader.field("allowoverflow").boolean()?,
        overflowed: reader.field("overflowed").boolean()?,
        owners: reader.field("owners").list(|entry| {
            Ok::<_, QuakeCSourceError>((
                entry.field("offset").integer(0)? as i32,
                entry.field("actor").nullable(restore_saved_actor)?,
            ))
        })?,
        destination: reader.field("destination").nullable(restore_destination)?,
    })
}

/// Encode broadcast-message state (`QcMessageCheckpoint`).
fn checkpoint_messages(checkpoint: &QcMessageCheckpoint) -> SaveJson {
    obj(vec![
        ("buffer", SaveJson::Bytes(checkpoint.buffer.clone())),
        ("overflowed", boolean(checkpoint.overflowed)),
        (
            "owners",
            arr(checkpoint
                .owners
                .iter()
                .map(|(offset, actor)| {
                    obj(vec![
                        ("offset", int(i64::from(*offset))),
                        ("actor", actor.as_ref().map_or(SaveJson::Null, checkpoint_actor)),
                    ])
                })
                .collect()),
        ),
        ("signonBuffers", int(i64::from(checkpoint.signon_buffers))),
        ("protocolVersion", int(i64::from(checkpoint.protocol_version))),
        ("protocolFlags", int(i64::from(checkpoint.protocol_flags))),
        ("routed", arr(checkpoint.routed.iter().map(checkpoint_routed).collect())),
    ])
}

/// Decode broadcast-message state.
fn restore_messages_checkpoint(reader: ContentReader) -> Result<QcMessageCheckpoint, QuakeCSourceError> {
    Ok(QcMessageCheckpoint {
        buffer: reader.field("buffer").bytes()?,
        overflowed: reader.field("overflowed").boolean()?,
        owners: reader.field("owners").list(|entry| {
            Ok::<_, QuakeCSourceError>((
                entry.field("offset").integer(0)? as i32,
                entry.field("actor").nullable(restore_saved_actor)?,
            ))
        })?,
        signon_buffers: u32::try_from(reader.field("signonBuffers").integer(0)?)
            .map_err(|_| ValueError("saved signon buffer count exceeds u32".to_string()))?,
        protocol_version: u32::try_from(reader.field("protocolVersion").integer(0)?)
            .map_err(|_| ValueError("saved protocol version exceeds u32".to_string()))?,
        protocol_flags: u32::try_from(reader.field("protocolFlags").integer(0)?)
            .map_err(|_| ValueError("saved protocol flags exceed u32".to_string()))?,
        routed: reader.field("routed").list(restore_routed)?,
    })
}

/// Executor host behind checkpoint capture/restore (`checkpointHost`).
struct CheckpointHost<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: &'s QuakeCSource<P>,
    clients: &'s [ClientId],
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QcExecutorHost for CheckpointHost<'_, P> {
    fn checkpoint(&mut self) -> Result<QcHostSavedState, GuestError> {
        self.source
            .capture_host_state()
            .map_err(|error| GuestError::invalid(error.to_string()))
    }

    fn restore(&mut self, saved: QcHostSavedState) -> Result<(), GuestError> {
        self.source
            .restore_host_state(&saved, self.clients)
            .map_err(|error| GuestError::invalid(error.to_string()))
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Borrow the machine for reads, honoring the executing-machine guard.
    fn machine_read<R>(&self, op: impl FnOnce(&QcMachine) -> R) -> R {
        if EXECUTING.with(|slot| slot.borrow().is_some()) {
            ExecutionGuard::with(op).expect("executing machine is published")
        } else {
            op(&self.machine.borrow())
        }
    }

    /// Borrow the machine for writes, honoring the executing-machine guard.
    fn machine_write<R>(&self, op: impl FnOnce(&mut QcMachine) -> R) -> R {
        if EXECUTING.with(|slot| slot.borrow().is_some()) {
            ExecutionGuard::with_mut(op).expect("executing machine is published")
        } else {
            op(&mut self.machine.borrow_mut())
        }
    }

    /// Run one machine function under the executing-machine guard.
    ///
    /// Failures recorded by the pickup/damage/entity hooks surface here so a
    /// faulted boundary fails the entry that ran it.
    fn execute_guarded(&self, function: usize, argc: usize) -> Result<(), QuakeCSourceError> {
        if let Some(error) = self.shared.borrow().hook_error.borrow_mut().take() {
            return Err(QuakeCSourceError::Guest(error));
        }
        self.machine_write(|machine| {
            let _guard = ExecutionGuard::enter(&mut *machine);
            machine.execute(function, argc)
        })?;
        if let Some(error) = self.shared.borrow().hook_error.borrow_mut().take() {
            return Err(QuakeCSourceError::Guest(error));
        }
        Ok(())
    }

    /// Entity field word (`field`).
    fn field(&self, name: &str) -> Result<usize, QuakeCSourceError> {
        self.prepared
            .program
            .field_named(name)
            .map(|definition| definition.offset)
            .ok_or_else(|| QuakeCSourceError::Invalid(format!("Missing id1 field {name}")))
    }

    /// Entity field word for closures that cannot fail.
    fn must_field(&self, name: &str) -> usize {
        self.field(name).expect("Missing id1 field")
    }

    /// Entity float word for closures that cannot fail.
    fn must_float(&self, slot: usize, name: &str) -> f64 {
        let word = self.must_field(name);
        f64::from(self.machine_read(|machine| {
            machine
                .entities()
                .slot_float(slot as u32, word)
                .expect("admitted QC row is readable")
        }))
    }

    /// Entity integer word for closures that cannot fail.
    fn must_int(&self, slot: usize, name: &str) -> i32 {
        let word = self.must_field(name);
        self.machine_read(|machine| {
            machine
                .entities()
                .slot_int(slot as u32, word)
                .expect("admitted QC row is readable")
        })
    }

    /// Write one entity float word for closures that cannot fail.
    fn must_set_float(&self, slot: usize, name: &str, value: f32) {
        let word = self.must_field(name);
        self.machine_write(|machine| {
            machine
                .entities_mut()
                .set_slot_float(slot as u32, word, value)
                .expect("admitted QC row is writable");
        });
    }

    /// Encode an actor as a QuakeC entity reference (`reference`).
    ///
    /// The donor's reference is a byte offset into QuakeC entity memory, not
    /// the physics world's handle, so this resolves the source slot first and
    /// encodes it through the machine layout.
    fn reference(&self, actor: &ActorId) -> Result<i32, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("QC entity has no source slot".to_string()))?;
        self.machine_read(|machine| machine.entities().reference(slot as u32))
            .map_err(QuakeCSourceError::from)
    }

    /// Source slot owned by an actor (`sourceSlot`).
    pub fn source_slot(&self, actor: &ActorId) -> Option<usize> {
        let actors = self.shared.borrow().options.actors.clone();
        if !actors.borrow().is_live(actor) {
            return None;
        }
        let (provider, slot) = actors.borrow().source_of(actor)?;
        (provider == self.prepared.execution.owner.provider).then_some(slot)
    }

    /// Whether an actor is a reserved client row (`isReservedClient`).
    pub fn is_reserved_client(&self, actor: &ActorId) -> bool {
        let Some(slot) = self.source_slot(actor) else {
            return false;
        };
        slot > 0 && slot <= self.shared.borrow().reserved_client_slots
    }

    /// Whether an actor is an admitted client (`isActiveClient`).
    pub fn is_active_client(&self, actor: &ActorId) -> bool {
        self.shared.borrow().active_clients.borrow().contains(actor)
    }

    /// Current server time in seconds.
    fn current_time(&self) -> f64 {
        self.shared.borrow().current_time
    }

    /// Source family (`kind`).
    pub fn kind(&self) -> QuakeCSourceKind {
        self.shared.borrow().kind
    }

    /// Prepared source declaration (`game.prepared`; C7 needs the team aliases).
    pub fn prepared(&self) -> &PreparedQuakeCSource {
        &self.prepared
    }

    /// Shared source cvars (`game.cvars`; C7 needs pause/server-settings reads).
    pub fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
        self.shared.borrow().cvars.clone()
    }

    /// Current server time in seconds (`timeSeconds`).
    pub fn time_seconds(&self) -> f64 {
        self.current_time()
    }

    /// Whether the source is still spawning map entities (`loading`).
    pub fn loading(&self) -> bool {
        *self.shared.borrow().spawning.borrow()
    }

    /// Physics callback currently running, if any (`currentPhysicsCallback`).
    pub fn current_physics_callback(&self) -> Option<QuakeCPhysicsCallback> {
        self.shared.borrow().physics_callback.borrow().clone()
    }

    /// Worldspawn actor (`worldActor`).
    pub fn world_actor(&self) -> Result<OwnedActor, QuakeCSourceError> {
        let shared = self.shared.borrow();
        let actor = shared
            .options
            .actors
            .borrow()
            .at_source(&self.prepared.execution.owner.provider, 0)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC worldspawn actor".to_string()));
        actor
    }

    /// Team tag for match scoring (`matchTeam`).
    pub fn match_team(&self, actor: &ActorId) -> Result<Option<String>, QuakeCSourceError> {
        if self.shared.borrow().kind == QuakeCSourceKind::Quakeworld {
            let value = self
                .source_slot(actor)
                .and_then(|slot| self.shared.borrow().user_info.get(&slot)?.get("team").cloned());
            return Ok(value.filter(|value| !value.is_empty()));
        }
        let reference = self.reference(actor)?;
        let team = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities().slot_float(slot, self.must_field("team"))
        })?;
        Ok(if team > 0.0 { Some(format!("{team}")) } else { None })
    }

    /// Read id1 armor state for a slot (`armor`).
    fn armor(&self, slot: usize) -> Result<ArmorState, QuakeCSourceError> {
        let bytes = self.machine_read(|machine| machine.entities().record_bytes(slot as u32).map(<[u8]>::to_vec))?;
        let words = QcWordsBuf::from_bytes(bytes)?;
        Ok(self.shared.borrow().damage.read_armor(&words)?)
    }

    /// Admit one bound actor (`admit`).
    ///
    /// The world admit hook only queues admissions (it fires while `Shared`
    /// is mutably borrowed); [`drain_admissions`](Self::drain_admissions) runs
    /// them once the borrow releases, so this method borrows freely. Closures
    /// handed to the session reach back through a weak source handle; a
    /// session that calls them after the source was released is a programming
    /// error and fails loudly, matching the donor's captured `this`.
    fn admit(&self, actor: &OwnedActor, slot: usize) -> Result<(), QuakeCSourceError> {
        let (damage, program_view, combat, callbacks, admit_hook, weak) = {
            let shared = self.shared.borrow();
            (
                shared.damage,
                shared.program_view,
                Rc::clone(&shared.options.combat),
                Rc::clone(&shared.options.callbacks),
                Rc::clone(&shared.options.admit),
                shared.source.clone(),
            )
        };
        let mut cache = Id1ProgramCache::default();
        let declared = declared_damage_call(&self.prepared)?;
        let binding = id1_program_binding(&mut cache, program_view, declared.as_ref())?;
        let declared_empty = self
            .prepared
            .combat_declaration
            .as_ref()
            .and_then(|combat| combat.empty_armor.as_ref())
            .map(content_empty_armor)
            .transpose()?;
        let empty_regular_armor = qc_empty_armor(program_view, declared_empty.as_ref(), declared.as_ref())?;
        let powered = Id1DamageBinding::protection_stage(damage, actor, ProtectionChannel::Powered);
        let regular = Id1DamageBinding::protection_stage(damage, actor, ProtectionChannel::Regular);
        let actor_id = actor.id().clone();
        let armor_field = binding.armor_field.clone();
        let armor_masks = binding.armor_masks;
        combat.bind_actor(
            actor,
            QuakeCCombatBinding {
                empty_regular_armor,
                source_damage: {
                    let weak = weak.clone();
                    Rc::new(move |request| {
                        weak.upgrade()
                            .expect("QuakeC source was released")
                            .apply_source_damage(request)
                            .expect("source damage completed")
                    })
                },
                regular_owner: actor.owner().clone(),
                regular_stage: regular,
                powered_owner: None,
                powered_stage: powered,
                read: {
                    let weak = weak.clone();
                    let actor_id = actor_id.clone();
                    Rc::new(move || {
                        let source = weak.upgrade().expect("QuakeC source was released");
                        let slot = source.source_slot(&actor_id).expect("admitted QC actor lost its slot");
                        QuakeCCombatVitals {
                            health: source.must_float(slot, "health"),
                            armor: source.armor(slot).expect("admitted QC armor is readable"),
                            mass: 200.0,
                            can_take_damage: source.must_float(slot, "takedamage") != 0.0,
                            invulnerable: source.must_float(slot, "invincible_finished") >= source.current_time(),
                            team: source.match_team(&actor_id).expect("admitted QC team is readable"),
                        }
                    })
                },
                write_health: {
                    let weak = weak.clone();
                    let actor_id = actor_id.clone();
                    Rc::new(move |health| {
                        let source = weak.upgrade().expect("QuakeC source was released");
                        let slot = source.source_slot(&actor_id).expect("admitted QC actor lost its slot");
                        source.must_set_float(slot, "health", health as f32);
                    })
                },
                validate_armor: Rc::new(|armor| {
                    assert!(
                        armor.powered == PoweredProtectionState::None
                            && (armor.regular == RegularArmorState::None
                                || matches!(armor.regular, RegularArmorState::Q1 { .. })),
                        "Cannot store foreign armor in native id1 fields"
                    );
                }),
                write_armor: {
                    let weak = weak.clone();
                    let actor_id = actor_id.clone();
                    Rc::new(move |armor| {
                        assert!(
                            armor.powered == PoweredProtectionState::None
                                && (armor.regular == RegularArmorState::None
                                    || matches!(armor.regular, RegularArmorState::Q1 { .. })),
                            "Cannot store foreign armor in native id1 fields"
                        );
                        let source = weak.upgrade().expect("QuakeC source was released");
                        let slot = source.source_slot(&actor_id).expect("admitted QC actor lost its slot");
                        let (points, absorption) = match &armor.regular {
                            RegularArmorState::None => (0.0, 0.0),
                            RegularArmorState::Q1 { points, absorption, .. } => (*points, *absorption),
                            _ => (0.0, 0.0),
                        };
                        source.must_set_float(slot, "armorvalue", points as f32);
                        source.must_set_float(slot, "armortype", absorption as f32);
                        let [green, yellow, red] = armor_masks;
                        let mask = green | yellow | red;
                        let bit = match &armor.regular {
                            RegularArmorState::None => 0,
                            RegularArmorState::Q1 { item, .. } if item == "q1:item_armorInv" => red,
                            RegularArmorState::Q1 { item, .. } if item == "q1:item_armor2" => yellow,
                            RegularArmorState::Q1 { .. } => green,
                            _ => 0,
                        };
                        let field = armor_field.clone();
                        let kept = source.must_int(slot, &field) & !mask | bit;
                        source.must_set_float(slot, &field, kept as f32);
                    })
                },
            },
        );
        callbacks.borrow_mut().bind_actor(
            actor,
            QuakeCActorHooks {
                think: {
                    let weak = weak.clone();
                    let actor = actor.clone();
                    Rc::new(move |_actor: &OwnedActor, frame: &FrameContext| {
                        weak.upgrade()
                            .expect("QuakeC source was released")
                            .run_think(&actor, frame)
                            .expect("QC think completed");
                    })
                },
                touch: {
                    let weak = weak.clone();
                    Rc::new(move |contact: &QuakeCTouchContact| {
                        let source = weak.upgrade().expect("QuakeC source was released");
                        let Some(current) = source.source_slot(contact.touch_self.id()) else {
                            return;
                        };
                        let callback = source.must_int(current, "touch");
                        if callback != 0 && source.must_float(current, "solid") != 0.0 {
                            let other = source.reference(&contact.other).expect("touch target has a reference");
                            let prior =
                                source
                                    .shared
                                    .borrow()
                                    .physics_callback
                                    .borrow_mut()
                                    .replace(QuakeCPhysicsCallback {
                                        kind: EnvCallbackKind::Touch,
                                        actor: contact.touch_self.id().clone(),
                                        other: contact.other.clone(),
                                        function_index: callback.max(0) as usize,
                                    });
                            let outcome = source.invoke(callback, current, other, source.current_time());
                            *source.shared.borrow().physics_callback.borrow_mut() = prior;
                            outcome.expect("QC touch completed");
                        }
                    })
                },
            },
        );
        admit_hook(actor, slot, self);
        Ok(())
    }

    /// Run queued admissions (`world` admit-hook queue).
    fn drain_admissions(&self) -> Result<(), QuakeCSourceError> {
        loop {
            let next = self.shared.borrow().admissions.borrow_mut().pop();
            let Some((actor, slot)) = next else {
                return Ok(());
            };
            let actors = self.shared.borrow().options.actors.clone();
            let Some(owned) = actors.borrow().resolve_owned(&actor) else {
                continue;
            };
            self.admit(&owned, slot)?;
        }
    }

    /// Bind one reserved row (`worldHost.actor` for map/client rows).
    ///
    /// Mirrors the checkpoint-restore binding order: field row, storage row,
    /// then the world body binding that queues admission. Re-entrant: rows
    /// that are already bound keep their words.
    fn bind_reserved_slot(&self, slot: usize) -> Result<(), QuakeCSourceError> {
        self.bind_slot_inner(slot, None)?;
        Ok(())
    }

    /// Bind any live row (map/client/save rows share this order).
    fn bind_slot(&self, slot: usize, actor: &OwnedActor) -> Result<(), QuakeCSourceError> {
        self.bind_slot_inner(slot, Some(actor))?;
        Ok(())
    }

    /// Bind one row and resolve its actor (donor `worldHost.actor`).
    fn slot_actor(&self, slot: usize) -> Result<OwnedActor, QuakeCSourceError> {
        self.bind_slot_inner(slot, None)?;
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, slot);
        actor.ok_or_else(|| QuakeCSourceError::Invalid("Missing QC bound actor".to_string()))
    }

    /// Shared row binder behind [`bind_reserved_slot`](Self::bind_reserved_slot),
    /// [`bind_slot`](Self::bind_slot) and [`slot_actor`](Self::slot_actor).
    fn bind_slot_inner(&self, slot: usize, actor: Option<&OwnedActor>) -> Result<(), QuakeCSourceError> {
        let slot_free = self.shared.borrow().storage.borrow().is_free(slot);
        if slot_free {
            let provider = self.prepared.execution.owner.provider.clone();
            let actors = self.shared.borrow().options.actors.clone();
            let resolved;
            let actor = match actor {
                Some(actor) => actor,
                None => {
                    resolved = actors
                        .borrow()
                        .at_source(&provider, slot)
                        .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC reserved actor".to_string()))?;
                    &resolved
                }
            };
            {
                let shared = self.shared.borrow();
                let allocated = shared.fields.borrow().is_allocated(actor.id());
                if !allocated {
                    shared.fields.borrow_mut().allocate(actor.id(), &shared.layout)?;
                }
                shared
                    .storage
                    .borrow_mut()
                    .initialize(slot, &mut shared.fields.borrow_mut(), actor.id())?;
            }
        }
        self.shared.borrow_mut().world.actor(slot)?;
        self.drain_admissions()?;
        if let Some(error) = self.shared.borrow().hook_error.borrow_mut().take() {
            return Err(QuakeCSourceError::Guest(error));
        }
        Ok(())
    }

    /// Release one bound row (donor `slots.free`).
    fn free_slot(&self, actor: &OwnedActor, slot: usize) -> Result<(), QuakeCSourceError> {
        let actors = self.shared.borrow().options.actors.clone();
        actors.borrow_mut().release(actor);
        let now = self.current_time() as f32;
        let shared = self.shared.borrow();
        shared
            .storage
            .borrow_mut()
            .clear_freed(slot, &mut shared.fields.borrow_mut(), actor.id(), now)?;
        Ok(())
    }

    /// Bind session inventory rows to one client slot (`bindClientInventory`).
    fn bind_client_inventory(&self, actor: &OwnedActor, slot: usize) -> Result<(), QuakeCSourceError> {
        let (inventory, bind_inventory, weapons) = {
            let shared = self.shared.borrow();
            (
                Rc::clone(&shared.options.inventory),
                shared.options.bind_inventory.clone(),
                shared.weapons.clone(),
            )
        };
        let items = self.field("items")?;
        let ammo_rows: Vec<(String, f64, usize)> = [
            ("q1:ammo/shells", "ammo_shells", 100.0),
            ("q1:ammo/nails", "ammo_nails", 200.0),
            ("q1:ammo/rockets", "ammo_rockets", 100.0),
            ("q1:ammo/cells", "ammo_cells", 100.0),
        ]
        .into_iter()
        .map(|(item, field, capacity)| Ok::<_, QuakeCSourceError>((item.to_string(), capacity, self.field(field)?)))
        .collect::<Result<_, _>>()?;
        let machine = Rc::clone(&self.machine);
        let read_machine = Rc::clone(&machine);
        let read_weapons = weapons.clone();
        let read_rows = ammo_rows.clone();
        let binding = QuakeCInventoryBinding {
            read: Rc::new(move || {
                let machine = read_machine.borrow();
                let entities = machine.entities();
                let held = entities.slot_int(slot as u32, items).unwrap_or(0);
                let mut entries: Vec<QuakeCInventoryEntry> = read_weapons
                    .iter()
                    .map(|weapon| QuakeCInventoryEntry {
                        item: weapon.item.clone(),
                        count: if held & weapon.bit == 0 { 0.0 } else { 1.0 },
                        capacity: 1.0,
                        source_counter: false,
                    })
                    .collect();
                for (item, capacity, word) in &read_rows {
                    entries.push(QuakeCInventoryEntry {
                        item: item.clone(),
                        count: f64::from(entities.slot_float(slot as u32, *word).unwrap_or(0.0)),
                        capacity: *capacity,
                        source_counter: true,
                    });
                }
                entries
            }),
            write: Rc::new(move |entry: QuakeCInventoryEntry| {
                if let Some(weapon) = weapons.iter().find(|weapon| weapon.item == entry.item) {
                    let mut machine = machine.borrow_mut();
                    let bits = machine.entities().slot_int(slot as u32, items).unwrap_or(0);
                    let held = if entry.count > 0.0 {
                        bits | weapon.bit
                    } else {
                        bits & !weapon.bit
                    };
                    machine
                        .entities_mut()
                        .set_slot_float(slot as u32, items, held as f32)
                        .expect("admitted QC row is writable");
                    return;
                }
                if let Some((_, _, word)) = ammo_rows.iter().find(|(item, _, _)| *item == entry.item) {
                    machine
                        .borrow_mut()
                        .entities_mut()
                        .set_slot_float(slot as u32, *word, entry.count as f32)
                        .expect("admitted QC row is writable");
                    return;
                }
                panic!("Unsupported QC inventory field {}", entry.item);
            }),
        };
        match bind_inventory {
            Some(hook) => hook(actor, binding),
            None => inventory.borrow_mut().bind(actor, binding),
        }
        Ok(())
    }
    /// Invoke one id1 callback with self/other/time globals (`invoke`).
    fn invoke(&self, callback: i32, slot: usize, other: i32, time: f64) -> Result<(), QuakeCSourceError> {
        if callback == 0 {
            return Ok(());
        }
        let reference = self.machine_read(|machine| machine.entities().reference(slot as u32))?;
        let (this, saved_other) = self.machine_write(|machine| {
            let self_offset = machine.global_offset("self")?;
            let other_offset = machine.global_offset("other")?;
            let saved = (
                machine.globals().int(self_offset)?,
                machine.globals().int(other_offset)?,
            );
            machine.globals_mut().set_int(self_offset, reference)?;
            machine.globals_mut().set_int(other_offset, other)?;
            let time_offset = machine.global_offset("time")?;
            machine.globals_mut().set_float(time_offset, time as f32)?;
            Ok::<_, GuestError>(saved)
        })?;
        let outcome = self.execute_guarded(callback.max(0) as usize, 0);
        self.machine_write(|machine| {
            if let (Ok(self_offset), Ok(other_offset)) = (machine.global_offset("self"), machine.global_offset("other"))
            {
                machine.globals_mut().set_int(self_offset, this).ok();
                machine.globals_mut().set_int(other_offset, saved_other).ok();
            }
        });
        outcome
    }

    /// Invoke a declared client stage for one client (`invokeClientStage`).
    fn invoke_client_stage(&self, call: &QcClientStageCall, actor: &ActorId) -> Result<i32, QuakeCSourceError> {
        let actors = self.shared.borrow().options.actors.clone();
        let owner = actors
            .borrow()
            .resolve_owned(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing live QC client stage actor".to_string()))?;
        if self.source_slot(actor).is_none() || !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid(
                "Missing live QC client stage actor".to_string(),
            ));
        }
        let machine_view = self.shared.borrow().machine_view;
        let result = invoke_qc_client_stage(machine_view, call, actor, self.current_time(), &|actor| {
            Ok(match actor {
                None => machine_view.entity_reference(0)?,
                Some(actor) => {
                    let slot = self
                        .source_slot(actor)
                        .ok_or_else(|| qc_error(GuestError::invalid("QC stage actor has no source slot")))?;
                    machine_view.entity_reference(slot)?
                }
            })
        })?;
        if actors.borrow().resolve_owned(actor) != Some(owner) || !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid(
                "Original QC client stage retired its client".to_string(),
            ));
        }
        Ok(result)
    }

    /// Run one think callback when due (`runThink`).
    pub fn run_think(&self, actor: &OwnedActor, frame: &FrameContext) -> Result<(), QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor.id()) else {
            return Ok(());
        };
        let next_word = self.field("nextthink")?;
        let think_word = self.field("think")?;
        let due = self.machine_read(|machine| machine.entities().slot_float(slot as u32, next_word))?;
        let profile = ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        };
        let time = think_callback_time(&profile, SourceTime::Seconds(due), *frame).map_err(GuestError::from)?;
        let Some(time) = time else {
            return Ok(());
        };
        self.machine_write(|machine| machine.entities_mut().set_slot_float(slot as u32, next_word, 0.0))?;
        let think = self.machine_read(|machine| machine.entities().slot_int(slot as u32, think_word))?;
        self.invoke(think, slot, 0, time.as_seconds_f64())
    }

    /// Run incoming damage through the damage ABI (`applySourceDamage`).
    fn apply_source_damage(&self, request: DamageRequest) -> Result<DamageOutcome, QuakeCSourceError> {
        let (program_view, machine_view, incoming) = {
            let shared = self.shared.borrow();
            (
                shared.program_view,
                shared.machine_view,
                Rc::clone(&shared.incoming_damage),
            )
        };
        let mut cache = Id1ProgramCache::default();
        let declared = declared_damage_call(&self.prepared)?;
        let damage_abi = id1_program_binding(&mut cache, program_view, declared.as_ref())?.damage;
        if damage_abi.call.declaration.is_none() && damage_abi.call.parameters.len() != 4 {
            return Err(QuakeCSourceError::Invalid(
                "QC incoming damage requires its qualified four-argument source ABI".to_string(),
            ));
        }
        if !(request.amount as f32).is_finite() {
            return Err(QuakeCSourceError::Invalid(
                "Incoming QC damage must fit its source binary32 ABI".to_string(),
            ));
        }
        let target = self.reference(&request.target)?;
        let attacker = request
            .attack
            .attacker
            .as_ref()
            .map(|attacker| self.reference(attacker))
            .transpose()?
            .unwrap_or(0);
        let inflictor = request
            .attack
            .inflictor
            .as_ref()
            .map(|inflictor| self.reference(inflictor))
            .transpose()?
            .unwrap_or(0);
        let amount = request.amount;
        let knockback = request.knockback;
        let point = request.point;
        let direction = request.direction;
        let normal = request.normal;
        let target_actor = request.target.clone();
        let attacker_actor = request.attack.attacker.clone();
        let inflictor_actor = request.attack.inflictor.clone();
        let current_time = self.current_time();
        incoming.borrow_mut().push(IncomingDamage {
            request,
            entered: false,
            outcome: None,
        });
        let (saved_arguments, self_offset, other_offset, time_offset, previous) = self.machine_write(|machine| {
            let saved = machine.globals().bytes()[4..28 * 4].to_vec();
            let self_offset = machine.global_offset("self")?;
            let other_offset = machine.global_offset("other")?;
            let time_offset = machine.global_offset("time")?;
            let previous = (
                machine.globals().int(self_offset)?,
                machine.globals().int(other_offset)?,
                machine.globals().int(time_offset)?,
            );
            machine.globals_mut().set_int(self_offset, inflictor)?;
            machine.globals_mut().set_int(other_offset, target)?;
            machine.globals_mut().set_float(time_offset, current_time as f32)?;
            Ok::<_, GuestError>((saved, self_offset, other_offset, time_offset, previous))
        })?;
        let outcome = self.machine_write(|machine| {
            let _guard = ExecutionGuard::enter(&mut *machine);
            if let Some(declaration) = &damage_abi.call.declaration {
                let mut inputs = HashMap::new();
                inputs.insert(ModCallbackInput::Slf, ModRuntimeValue::Actor(Some(target_actor)));
                inputs.insert(ModCallbackInput::Attacker, ModRuntimeValue::Actor(attacker_actor));
                inputs.insert(ModCallbackInput::Inflictor, ModRuntimeValue::Actor(inflictor_actor));
                inputs.insert(ModCallbackInput::Amount, ModRuntimeValue::Float(amount));
                inputs.insert(ModCallbackInput::Knockback, ModRuntimeValue::Float(knockback));
                inputs.insert(ModCallbackInput::Point, ModRuntimeValue::Vector(point));
                inputs.insert(ModCallbackInput::Direction, ModRuntimeValue::Vector(direction));
                inputs.insert(ModCallbackInput::Normal, ModRuntimeValue::Vector(normal));
                inputs.insert(ModCallbackInput::Time, ModRuntimeValue::Float(current_time));
                with_qc_source_call(
                    machine_view,
                    declaration,
                    &inputs,
                    &|actor| {
                        Ok(match actor {
                            None => machine_view.entity_reference(0)?,
                            Some(actor) => {
                                let slot = self.source_slot(actor).ok_or_else(|| {
                                    qc_error(GuestError::invalid("QC stage actor has no source slot"))
                                })?;
                                machine_view.entity_reference(slot)?
                            }
                        })
                    },
                    &mut |count| {
                        machine.execute(damage_abi.index, count).map_err(qc_error)?;
                        machine_view.global_float(1)
                    },
                )?;
            } else {
                machine.globals_mut().set_int(4, target)?;
                machine.globals_mut().set_int(7, inflictor)?;
                machine.globals_mut().set_int(10, attacker)?;
                machine.globals_mut().set_float(13, amount as f32)?;
                machine.execute(damage_abi.index, 4)?;
            }
            if let Some(error) = self.shared.borrow().hook_error.borrow_mut().take() {
                return Err(QuakeCSourceError::Guest(error));
            }
            Ok::<_, QuakeCSourceError>(())
        });
        let completed = incoming.borrow().last().and_then(|entry| entry.outcome.clone());
        incoming.borrow_mut().pop();
        self.machine_write(|machine| {
            machine.globals_mut().bytes_mut()[4..28 * 4].copy_from_slice(&saved_arguments);
            machine.globals_mut().set_int(self_offset, previous.0).ok();
            machine.globals_mut().set_int(other_offset, previous.1).ok();
            machine.globals_mut().set_int(time_offset, previous.2).ok();
        });
        outcome?;
        completed.ok_or_else(|| {
            QuakeCSourceError::Invalid(
                "Incoming original QC damage did not complete its authority boundary".to_string(),
            )
        })
    }
    /// Owning module identity (`module`).
    fn module(&self) -> ModuleIdentity {
        let execution = &self.prepared.execution;
        let identity = execution.artifact.identity.canonical();
        let (algorithm, value) = identity.split_once(':').unwrap_or(("", identity.as_str()));
        ModuleIdentity {
            id: execution.owner.provider.clone(),
            artifact_path: execution.artifact.requested_path.clone(),
            digest: qa_guest::core::contracts::ContentDigest::new(algorithm, value),
            revision: identity,
        }
    }

    /// Capture a full checkpoint (`checkpoint`).
    pub fn checkpoint(&self) -> Result<QuakeCCheckpoint, QuakeCSourceError> {
        if *self.shared.borrow().spawning.borrow() || self.shared.borrow().physics_callback.borrow().is_some() {
            return Err(QuakeCSourceError::Invalid(
                "QC save requires a completed source frame".to_string(),
            ));
        }
        for slot in self.shared.borrow().client_identities.keys() {
            let actors = self.shared.borrow().options.actors.clone();
            let actor = actors
                .borrow()
                .at_source(&self.prepared.execution.owner.provider, *slot);
            let live = actor.as_ref().is_some_and(|actor| self.is_active_client(actor.id()));
            if !live {
                return Err(QuakeCSourceError::Invalid(
                    "QC save requires pending client handshakes to finish".to_string(),
                ));
            }
        }
        let (attacks, pickups) = {
            let shared = self.shared.borrow();
            (shared.attacks, shared.pickups)
        };
        attacks.assert_idle()?;
        pickups.assert_idle()?;
        let module = self.module();
        let mut host = CheckpointHost {
            source: self,
            clients: &[],
        };
        Ok(self.machine_read(|machine| capture_qc_checkpoint(machine, &module, &mut host))?)
    }

    /// Encode one NetQuake wire message for checkpoints.
    fn checkpoint_wire_message(&self, writer: &mut MsgWriter, message: &NetQuakeMessage) -> Result<(), GuestError> {
        write_net_quake_message(
            writer,
            NqProfile::Netquake,
            message,
            self.prepared.message_dialect,
            true,
        )
        .map_err(value_error)
    }

    /// Encode local messages into checkpoint bytes.
    fn checkpoint_local_bytes(&self, messages: &[NetQuakeMessage]) -> Result<Vec<u8>, GuestError> {
        let mut writer = MsgWriter::new(1 << 20, false);
        for message in messages {
            self.checkpoint_wire_message(&mut writer, message)?;
        }
        Ok(writer.bytes().to_vec())
    }

    /// Capture host state (`checkpointHost.checkpoint`).
    ///
    /// The client `visibility` table has no Rust counterpart (the guest client
    /// host keeps no persistent visibility state), so it is omitted; the
    /// cvar `latched` values likewise have no restore API and are omitted.
    fn capture_host_state(&self) -> Result<QcHostSavedState, QuakeCSourceError> {
        let shared = self.shared.borrow();
        let router = shared.router.borrow();
        let cvars = shared.cvars.borrow();
        let fate = |error: QuakeCSourceError| GuestError::invalid(error.to_string());
        let mut members: Vec<(&str, SaveJson)> = vec![
            ("kind", str(checkpoint_kind(shared.kind))),
            ("teams", str(&format!("{:?}", self.prepared.teams))),
            ("messageDialect", str(checkpoint_dialect(self.prepared.message_dialect))),
            ("pickupCallers", str(&format!("{:?}", self.prepared.declared_pickups))),
            (
                "weaponStage",
                self.prepared
                    .weapon_declaration
                    .as_ref()
                    .map_or(SaveJson::Null, |stage| str(&format!("{stage:?}"))),
            ),
            (
                "combat",
                self.prepared
                    .combat_declaration
                    .as_ref()
                    .map_or(SaveJson::Null, |combat| str(&format!("{combat:?}"))),
            ),
            ("maxClients", int(shared.options.max_clients as i64)),
            ("reservedClientSlots", int(shared.reserved_client_slots as i64)),
            ("currentTime", num(shared.current_time)),
            ("changeLevelIssued", boolean(shared.change_level_issued)),
            ("spawning", boolean(*shared.spawning.borrow())),
            (
                "activeClients",
                arr(shared
                    .active_clients
                    .borrow()
                    .iter()
                    .map(|actor| checkpoint_actor(&SavedActorId::from(actor)))
                    .collect()),
            ),
            (
                "borrowedActors",
                arr(shared
                    .borrowed
                    .borrow()
                    .checkpoint()
                    .iter()
                    .map(|entry| {
                        obj(vec![
                            ("actor", checkpoint_actor(&entry.actor)),
                            ("slot", int(entry.slot as i64)),
                        ])
                    })
                    .collect()),
            ),
            (
                "pendingWeapons",
                arr(shared
                    .pending_weapons
                    .borrow()
                    .iter()
                    .map(|(actor, pending)| {
                        obj(vec![
                            ("actor", checkpoint_actor(&SavedActorId::from(actor))),
                            ("weapon", str(&pending.weapon.item)),
                            ("following", boolean(pending.following)),
                        ])
                    })
                    .collect()),
            ),
            (
                "userInfo",
                arr(shared
                    .user_info
                    .iter()
                    .map(|(slot, values)| {
                        obj(vec![
                            ("slot", int(*slot as i64)),
                            (
                                "values",
                                arr(values
                                    .iter()
                                    .map(|(key, value)| obj(vec![("key", str(key)), ("value", str(value))]))
                                    .collect()),
                            ),
                        ])
                    })
                    .collect()),
            ),
            (
                "spectatorSlots",
                arr(shared.spectator_slots.iter().map(|slot| int(*slot as i64)).collect()),
            ),
            ("originalSaveExtensionText", str(&shared.original_save_extension_text)),
            (
                "spawnParameters",
                arr(shared
                    .spawn_parameters
                    .iter()
                    .map(|(slot, values)| {
                        obj(vec![
                            ("slot", int(*slot as i64)),
                            ("values", arr(values.iter().map(|value| num(*value)).collect())),
                        ])
                    })
                    .collect()),
            ),
            (
                "clientIdentities",
                arr(shared
                    .client_identities
                    .iter()
                    .map(|(slot, client)| {
                        obj(vec![
                            ("slot", int(*slot as i64)),
                            ("clientSlot", int(i64::from(client.slot()))),
                        ])
                    })
                    .collect()),
            ),
            (
                "preparedClients",
                arr(shared.prepared_clients.iter().map(|slot| int(*slot as i64)).collect()),
            ),
            (
                "fragRecords",
                arr(shared
                    .frag_records
                    .iter()
                    .map(|(killer, victim)| {
                        obj(vec![
                            ("killer", checkpoint_actor(&SavedActorId::from(killer))),
                            ("victim", checkpoint_actor(&SavedActorId::from(victim))),
                        ])
                    })
                    .collect()),
            ),
        ];
        let mut routed = Vec::with_capacity(router.routed.len());
        for entry in &router.routed {
            routed.push(obj(vec![
                (
                    "entries",
                    checkpoint_entries(&shared.messages.capture_entries(&entry.entries)?),
                ),
                (
                    "destination",
                    checkpoint_destination(&capture_qc_destination(&entry.destination)),
                ),
            ]));
        }
        members.push(("routed", arr(routed)));
        let local = router.local_messages.capture();
        let mut local_clients = Vec::with_capacity(local.clients.len());
        for entry in &local.clients {
            local_clients.push(obj(vec![
                ("actor", checkpoint_actor(&SavedActorId::from(&entry.actor))),
                ("bytes", SaveJson::Bytes(self.checkpoint_local_bytes(&entry.messages)?)),
            ]));
        }
        let views = router.local_messages.capture_views();
        members.push((
            "localMessages",
            obj(vec![
                ("started", boolean(router.local_messages_started)),
                (
                    "baseline",
                    SaveJson::Bytes(self.checkpoint_local_bytes(&local.baseline)?),
                ),
                ("clients", arr(local_clients)),
                (
                    "views",
                    obj(vec![
                        (
                            "baseline",
                            views
                                .baseline
                                .as_ref()
                                .map_or(SaveJson::Null, |actor| checkpoint_actor(&SavedActorId::from(actor))),
                        ),
                        (
                            "clients",
                            arr(views
                                .clients
                                .iter()
                                .map(|entry| {
                                    obj(vec![
                                        ("actor", checkpoint_actor(&SavedActorId::from(&entry.actor))),
                                        ("target", checkpoint_actor(&SavedActorId::from(&entry.target))),
                                    ])
                                })
                                .collect()),
                        ),
                    ]),
                ),
            ]),
        ));
        members.push((
            "netQuakeSignon",
            SaveJson::Bytes(capture_netquake_messages(&router.netquake_signon)?),
        ));
        members.push((
            "netQuakeSignonViews",
            arr(router
                .netquake_signon_views
                .iter()
                .map(|(index, actor)| {
                    obj(vec![
                        ("index", int(*index as i64)),
                        (
                            "actor",
                            actor
                                .as_ref()
                                .map_or(SaveJson::Null, |actor| checkpoint_actor(&SavedActorId::from(actor))),
                        ),
                    ])
                })
                .collect()),
        ));
        let mut nq_routed = Vec::with_capacity(router.netquake_routed.len());
        for entry in &router.netquake_routed {
            nq_routed.push(obj(vec![
                ("bytes", SaveJson::Bytes(capture_netquake_messages(&entry.messages)?)),
                (
                    "destination",
                    checkpoint_destination(&capture_qc_destination(&entry.destination)),
                ),
                (
                    "views",
                    arr(entry
                        .view_targets
                        .iter()
                        .map(|(index, actor)| {
                            obj(vec![
                                ("index", int(*index as i64)),
                                (
                                    "actor",
                                    actor
                                        .as_ref()
                                        .map_or(SaveJson::Null, |actor| checkpoint_actor(&SavedActorId::from(actor))),
                                ),
                            ])
                        })
                        .collect()),
                ),
            ]));
        }
        members.push(("netQuakeRouted", arr(nq_routed)));
        members.push((
            "signon",
            checkpoint_entries(&shared.messages.capture_entries(&router.signon)?),
        ));
        members.push((
            "models",
            arr(shared
                .models
                .borrow()
                .iter()
                .map(|(name, model)| {
                    obj(vec![
                        ("name", str(name)),
                        ("index", int(i64::from(model.index))),
                        (
                            "bounds",
                            obj(vec![
                                ("min", checkpoint_vector(model.bounds.min)),
                                ("max", checkpoint_vector(model.bounds.max)),
                            ]),
                        ),
                    ])
                })
                .collect()),
        ));
        let mut precached = Vec::with_capacity(shared.precached.len());
        for (key, entry) in shared.precached.iter() {
            let name = key.split_once(':').map_or(key.as_str(), |(_, name)| name);
            let (id, identity) = if key.starts_with("model:") && self.map_model_index(name).map_err(fate)?.is_some() {
                let geometry = &shared.options.recipe.map.geometry;
                (geometry.id.clone(), geometry.identity.clone())
            } else if let Some(resource) = self.prepared.resources.get(name) {
                (
                    resource.resource.id.as_str().to_string(),
                    resource.resource.identity.canonical(),
                )
            } else {
                return Err(QuakeCSourceError::Invalid(format!(
                    "saved precache resource has no mounted content {name}"
                )));
            };
            precached.push(obj(vec![
                ("key", str(key)),
                ("index", int(i64::from(entry.index))),
                ("id", str(&id)),
                ("identity", str(&identity)),
            ]));
        }
        members.push(("precached", arr(precached)));
        members.push(("modelCount", int(i64::from(shared.model_count))));
        members.push(("soundCount", int(i64::from(shared.sound_count))));
        members.push((
            "cvars",
            arr(cvars
                .snapshots(0)
                .iter()
                .map(|snapshot| {
                    obj(vec![
                        ("name", str(&snapshot.name)),
                        ("value", str(&snapshot.value)),
                        ("reset", str(&snapshot.reset_value)),
                        ("flags", int(i64::from(snapshot.flags))),
                    ])
                })
                .collect()),
        ));
        members.push(("messages", checkpoint_messages(&shared.messages.capture())));
        let emissions = shared.projectiles.capture()?;
        members.push((
            "projectiles",
            arr(emissions
                .iter()
                .map(|emission| {
                    obj(vec![
                        ("actor", checkpoint_actor(&emission.actor)),
                        ("owner", checkpoint_actor(&emission.owner)),
                        ("emittedAt", num(emission.emitted_at)),
                        ("weapon", str(&emission.weapon)),
                    ])
                })
                .collect()),
        ));
        Ok(QcHostSavedState {
            state: GuestPrivateState {
                module: self.module(),
                format: "quakec:source-v1".to_string(),
                bytes: encode_checkpoint_value(&obj(members)),
            },
            random: Vec::new(),
            callbacks: Vec::new(),
        })
    }
    /// Restore host state over a saved checkpoint (`restoreHost` machine half).
    fn restore_host(&self, checkpoint: &QuakeCCheckpoint, clients: &[ClientId]) -> Result<(), QuakeCSourceError> {
        let module = self.module();
        let mut host = CheckpointHost { source: self, clients };
        self.machine_write(|machine| restore_qc_checkpoint(machine, &module, &mut host, checkpoint))?;
        let count = self.machine_read(|machine| machine.entities().count());
        self.shared.borrow().storage.borrow_mut().set_count(count)?;
        let provider = self.prepared.execution.owner.provider.clone();
        let reserved = self.shared.borrow().reserved_client_slots;
        for slot in 0..count {
            let actors = self.shared.borrow().options.actors.clone();
            let actor = actors.borrow().at_source(&provider, slot);
            match actor {
                None => {
                    let borrowed = self.shared.borrow().borrowed.borrow().actor(slot)?;
                    if borrowed.is_some() {
                        continue;
                    }
                    let free = self
                        .shared
                        .borrow()
                        .storage
                        .borrow()
                        .read(slot)
                        .map(|state| state.free)
                        .unwrap_or(true);
                    if slot <= reserved || !free {
                        return Err(QuakeCSourceError::Invalid(
                            "Saved QC edict has no restored actor".to_string(),
                        ));
                    }
                }
                Some(actor) => {
                    let free = self
                        .shared
                        .borrow()
                        .storage
                        .borrow()
                        .read(slot)
                        .map(|state| state.free)
                        .unwrap_or(true);
                    if slot > reserved && free {
                        return Err(QuakeCSourceError::Invalid(
                            "Saved free QC edict has a live actor".to_string(),
                        ));
                    }
                    {
                        let shared = self.shared.borrow();
                        let row_allocated = shared.fields.borrow().is_allocated(actor.id());
                        if !row_allocated {
                            shared.fields.borrow_mut().allocate(actor.id(), &shared.layout)?;
                        }
                        shared
                            .storage
                            .borrow_mut()
                            .initialize(slot, &mut shared.fields.borrow_mut(), actor.id())?;
                    }
                    self.shared.borrow_mut().world.actor(slot)?;
                    self.drain_admissions()?;
                    if self.is_active_client(actor.id()) {
                        self.bind_client_inventory(&actor, slot)?;
                    }
                }
            }
        }
        if let Some(error) = self.shared.borrow().hook_error.borrow_mut().take() {
            return Err(QuakeCSourceError::Guest(error));
        }
        Ok(())
    }

    /// Restore host state from a checkpoint value (`restoreHost` value half).
    fn restore_host_state(&self, saved: &QcHostSavedState, clients: &[ClientId]) -> Result<(), QuakeCSourceError> {
        if saved.state.format != "quakec:source-v1" || !saved.random.is_empty() || !saved.callbacks.is_empty() {
            return Err(QuakeCSourceError::Invalid(
                "Unsupported QC source host checkpoint".to_string(),
            ));
        }
        let value = decode_checkpoint_value(&saved.state.bytes)?;
        let reader = ContentReader::new(&value);
        let dialect = reader.field("messageDialect");
        let dialect_text = if dialect.is_missing() {
            "known-retail".to_string()
        } else {
            dialect.choice_str(&["known-retail", "quake-1-re-ts-private"])?
        };
        if restore_dialect(&dialect_text)? != self.prepared.message_dialect {
            return Err(QuakeCSourceError::Invalid("QuakeC message dialect changed".to_string()));
        }
        let teams = reader.field("teams").string()?;
        if teams != format!("{:?}", self.prepared.teams) {
            return Err(QuakeCSourceError::Invalid(
                "QuakeC team declarations changed".to_string(),
            ));
        }
        let combat = reader.field("combat").nullable(|entry| entry.string())?;
        let expected_combat = self
            .prepared
            .combat_declaration
            .as_ref()
            .map(|combat| format!("{combat:?}"));
        if combat != expected_combat {
            return Err(QuakeCSourceError::Invalid(
                "QuakeC combat declaration changed".to_string(),
            ));
        }
        let weapon_stage = reader.field("weaponStage").nullable(|entry| entry.string())?;
        let expected_stage = self
            .prepared
            .weapon_declaration
            .as_ref()
            .map(|stage| format!("{stage:?}"));
        if weapon_stage != expected_stage {
            return Err(QuakeCSourceError::Invalid(
                "QuakeC weapon stage declaration changed".to_string(),
            ));
        }
        let pickups = reader.field("pickupCallers").string()?;
        if pickups != format!("{:?}", self.prepared.declared_pickups) {
            return Err(QuakeCSourceError::Invalid(
                "QuakeC pickup caller declarations changed".to_string(),
            ));
        }
        reader.field("kind").literal_str(checkpoint_kind(self.kind()))?;
        reader
            .field("maxClients")
            .literal_i64(self.shared.borrow().options.max_clients as i64)?;
        reader
            .field("reservedClientSlots")
            .literal_i64(self.shared.borrow().reserved_client_slots as i64)?;
        reader.field("spawning").literal_bool(false)?;
        let current_time = reader.field("currentTime").finite()?;
        if current_time < 0.0 {
            return Err(QuakeCSourceError::Invalid(
                "negative source frame-entry time".to_string(),
            ));
        }
        let change_level_issued = reader.field("changeLevelIssued").boolean()?;
        {
            let shared = self.shared.borrow();
            *shared.spawning.borrow_mut() = false;
        }
        {
            let mut shared = self.shared.borrow_mut();
            shared.current_time = current_time;
            shared.change_level_issued = change_level_issued;
        }
        {
            let shared = self.shared.borrow();
            let saved_borrowed: Vec<BorrowedCheckpoint> = reader.field("borrowedActors").list(|entry| {
                Ok::<_, QuakeCSourceError>(BorrowedCheckpoint {
                    actor: restore_saved_actor(entry.field("actor"))?,
                    slot: usize::try_from(entry.field("slot").integer(0)?)
                        .map_err(|_| ValueError("saved borrowed slot exceeds usize".to_string()))?,
                })
            })?;
            let actors = Rc::clone(&shared.options.actors);
            let reserved = shared.reserved_client_slots;
            shared.borrowed.borrow_mut().restore(
                &saved_borrowed,
                &|saved| Some(actors.borrow().reference_saved(*saved)),
                &|slot| slot <= reserved,
            )?;
        }
        {
            let mut shared = self.shared.borrow_mut();
            let extension = reader.field("originalSaveExtensionText");
            shared.original_save_extension_text = if extension.is_missing() {
                String::new()
            } else {
                extension.string()?
            };
        }
        let actors = self.shared.borrow().options.actors.clone();
        let reference = |entry: ContentReader| -> Result<ActorId, QuakeCSourceError> {
            Ok(actors.borrow().reference_saved(restore_saved_actor(entry)?))
        };
        let live = |entry: ContentReader| -> Result<ActorId, QuakeCSourceError> {
            let saved = restore_saved_actor(entry.clone())?;
            let actor = actors.borrow().reference_saved(saved);
            if self.source_slot(&actor).is_none() {
                return Err(QuakeCSourceError::Invalid(
                    entry.fail("missing live QC actor").to_string(),
                ));
            }
            Ok(actor)
        };
        let reserved = self.shared.borrow().reserved_client_slots;
        let client_slot = |entry: ContentReader| -> Result<usize, QuakeCSourceError> {
            let slot = usize::try_from(entry.integer(1)?)
                .map_err(|_| ValueError("invalid reserved client slot".to_string()))?;
            if slot > reserved {
                return Err(QuakeCSourceError::Invalid(
                    entry.fail("invalid reserved client slot").to_string(),
                ));
            }
            Ok(slot)
        };
        let active: Vec<ActorId> = reader.field("activeClients").list(live)?;
        {
            let shared = self.shared.borrow();
            let mut live_clients = shared.active_clients.borrow_mut();
            live_clients.clear();
            for actor in active {
                let slot = actors
                    .borrow()
                    .source_of(&actor)
                    .and_then(|(provider, slot)| (provider == self.prepared.execution.owner.provider).then_some(slot));
                let reserved_row = matches!(slot, Some(slot) if slot > 0 && slot <= shared.reserved_client_slots);
                if !reserved_row || live_clients.contains(&actor) {
                    return Err(QuakeCSourceError::Invalid("invalid active QC client".to_string()));
                }
                live_clients.insert(actor);
            }
        }
        let pending: Vec<(ActorId, String, bool)> = reader.field("pendingWeapons").list(|entry| {
            Ok::<_, QuakeCSourceError>((
                live(entry.field("actor"))?,
                namespaced(entry.field("weapon"))?,
                entry.field("following").boolean()?,
            ))
        })?;
        {
            let shared = self.shared.borrow();
            let mut pending_weapons = shared.pending_weapons.borrow_mut();
            pending_weapons.clear();
            for (actor, item, following) in pending {
                let weapon = shared.weapons.iter().find(|weapon| weapon.item == item).cloned();
                let Some(weapon) = weapon else {
                    return Err(QuakeCSourceError::Invalid("invalid pending weapon".to_string()));
                };
                if !shared.active_clients.borrow().contains(&actor) || pending_weapons.contains_key(&actor) {
                    return Err(QuakeCSourceError::Invalid("invalid pending weapon".to_string()));
                }
                pending_weapons.insert(actor, PendingWeapon { weapon, following });
            }
        }
        let user_info: Vec<(usize, Vec<(String, String)>)> = reader.field("userInfo").list(|entry| {
            Ok::<_, QuakeCSourceError>((
                client_slot(entry.field("slot"))?,
                entry.field("values").list(|item| {
                    Ok::<_, QuakeCSourceError>((item.field("key").string()?, item.field("value").string()?))
                })?,
            ))
        })?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.user_info.clear();
            for (slot, values) in user_info {
                if shared.user_info.contains_key(&slot) {
                    return Err(QuakeCSourceError::Invalid("duplicate userinfo slot".to_string()));
                }
                shared.user_info.insert(slot, values.into_iter().collect());
            }
        }
        let spawn_parameters: Vec<(usize, Vec<f64>)> = reader.field("spawnParameters").list(|entry| {
            Ok::<_, QuakeCSourceError>((
                client_slot(entry.field("slot"))?,
                entry.field("values").list(|item| item.finite())?,
            ))
        })?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.spawn_parameters.clear();
            for (slot, values) in spawn_parameters {
                if values.len() != 16 || shared.spawn_parameters.contains_key(&slot) {
                    return Err(QuakeCSourceError::Invalid("invalid spawn parameters".to_string()));
                }
                shared.spawn_parameters.insert(slot, values);
            }
        }
        let identities: Vec<(usize, i64)> = reader.field("clientIdentities").list(|entry| {
            Ok::<_, QuakeCSourceError>((client_slot(entry.field("slot"))?, entry.field("clientSlot").integer(0)?))
        })?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.client_identities.clear();
            for (slot, client_slot_value) in identities {
                let client = clients
                    .iter()
                    .find(|client| i64::from(client.slot()) == client_slot_value);
                let slot_matches = usize::try_from(client_slot_value).is_ok_and(|value| slot == value + 1);
                let Some(client) = client else {
                    return Err(QuakeCSourceError::Invalid(
                        "missing restored QC client identity".to_string(),
                    ));
                };
                if !slot_matches || shared.client_identities.contains_key(&slot) {
                    return Err(QuakeCSourceError::Invalid(
                        "missing restored QC client identity".to_string(),
                    ));
                }
                shared.client_identities.insert(slot, client.clone());
            }
        }
        {
            let mut shared = self.shared.borrow_mut();
            shared.spectator_slots.clear();
            let spectators = reader.field("spectatorSlots");
            if !spectators.is_missing() {
                for slot in spectators.list(client_slot)? {
                    if shared.kind != QuakeCSourceKind::Quakeworld
                        || !shared.client_identities.contains_key(&slot)
                        || shared.spectator_slots.contains(&slot)
                    {
                        return Err(QuakeCSourceError::Invalid("invalid spectator slot".to_string()));
                    }
                    shared.spectator_slots.insert(slot);
                }
            }
        }
        let prepared: Vec<usize> = reader.field("preparedClients").list(client_slot)?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.prepared_clients.clear();
            for slot in prepared {
                if shared.prepared_clients.contains(&slot) || !shared.client_identities.contains_key(&slot) {
                    return Err(QuakeCSourceError::Invalid("invalid prepared client phase".to_string()));
                }
                shared.prepared_clients.insert(slot);
            }
            if shared.active_clients.borrow().len() != shared.client_identities.len() {
                return Err(QuakeCSourceError::Invalid("client phase identity mismatch".to_string()));
            }
        }
        {
            let shared = self.shared.borrow();
            for actor in shared.active_clients.borrow().iter() {
                let slot = actors
                    .borrow()
                    .source_of(actor)
                    .and_then(|(provider, slot)| (provider == self.prepared.execution.owner.provider).then_some(slot));
                let complete = slot.is_some_and(|slot| {
                    shared.client_identities.contains_key(&slot)
                        && (shared.kind != QuakeCSourceKind::Quakeworld || shared.prepared_clients.contains(&slot))
                });
                if !complete {
                    return Err(QuakeCSourceError::Invalid("incomplete saved client phase".to_string()));
                }
            }
        }
        let frags: Vec<(ActorId, ActorId)> = reader.field("fragRecords").list(|entry| {
            Ok::<_, QuakeCSourceError>((reference(entry.field("killer"))?, reference(entry.field("victim"))?))
        })?;
        self.shared.borrow_mut().frag_records = frags;
        let resolve = |saved: &SavedActorId| Some(actors.borrow().reference_saved(*saved));
        let routed: Vec<RoutedQuakeWorldMessages> = reader.field("routed").list(|entry| {
            let saved = restore_entries_checkpoint(entry.field("entries"))?;
            Ok::<_, QuakeCSourceError>(RoutedQuakeWorldMessages {
                entries: self.shared.borrow().messages.restore_entries(&saved, &resolve)?,
                destination: read_qc_destination(&restore_destination(entry.field("destination"))?, &resolve)?,
            })
        })?;
        let signon = restore_entries_checkpoint(reader.field("signon"))?;
        let signon = self.shared.borrow().messages.restore_entries(&signon, &resolve)?;
        {
            let shared = self.shared.borrow();
            let mut router = shared.router.borrow_mut();
            router.routed = routed;
            router.signon = signon;
            router.netquake_signon.clear();
            let nq_signon = reader.field("netQuakeSignon");
            if !nq_signon.is_missing() {
                router.netquake_signon = restore_netquake_messages(&nq_signon.bytes()?)?;
            }
            let camera_views = |reader: ContentReader,
                                messages: &[NqMessage]|
             -> Result<Vec<(usize, Option<ActorId>)>, QuakeCSourceError> {
                let mut result = Vec::new();
                if !reader.is_missing() {
                    for (index, actor) in reader.list(|entry| {
                        let index = usize::try_from(entry.field("index").integer(0)?)
                            .map_err(|_| ValueError("camera view index exceeds usize".to_string()))?;
                        let actor = entry
                            .field("actor")
                            .nullable(restore_saved_actor)?
                            .map(|saved| actors.borrow().reference_saved(saved));
                        Ok::<_, QuakeCSourceError>((index, actor))
                    })? {
                        let is_view = messages
                            .get(index)
                            .is_some_and(|message| matches!(message, NqMessage::SetView { .. }));
                        if !is_view || result.iter().any(|(known, _)| *known == index) {
                            return Err(QuakeCSourceError::Invalid(
                                "Invalid retained QC camera message".to_string(),
                            ));
                        }
                        result.push((index, actor));
                    }
                }
                if messages.iter().enumerate().any(|(index, message)| {
                    matches!(message, NqMessage::SetView { .. }) && !result.iter().any(|(known, _)| *known == index)
                }) {
                    return Err(QuakeCSourceError::Invalid(
                        reader
                            .fail("Legacy QC camera message has no captured actor identity")
                            .to_string(),
                    ));
                }
                Ok(result)
            };
            router.netquake_signon_views =
                camera_views(reader.field("netQuakeSignonViews"), &router.netquake_signon.clone())?;
            let nq_routed: Vec<RoutedNetQuakeMessages> = reader.field("netQuakeRouted").list(|entry| {
                let messages = restore_netquake_messages(&entry.field("bytes").bytes()?)?;
                let destination = read_qc_destination(&restore_destination(entry.field("destination"))?, &resolve)?;
                let view_targets = camera_views(entry.field("views"), &messages)?;
                Ok::<_, QuakeCSourceError>(RoutedNetQuakeMessages {
                    messages,
                    destination,
                    view_targets,
                })
            })?;
            router.netquake_routed = nq_routed;
        }
        let models: Vec<(String, i32, Bounds)> = reader.field("models").list(|entry| {
            let name = entry.field("name").string()?;
            let index = entry.field("index").integer(1)?;
            let bounds = entry.field("bounds");
            let min = restore_vector(bounds.field("min"))?;
            let max = restore_vector(bounds.field("max"))?;
            Ok::<_, QuakeCSourceError>((
                name,
                i32::try_from(index).map_err(|_| value_error(entry.fail("model index exceeds i32")))?,
                Bounds { min, max },
            ))
        })?;
        {
            let shared = self.shared.borrow();
            let mut known = shared.models.borrow_mut();
            known.clear();
            for (name, index, bounds) in models {
                if known.contains_key(&name) {
                    return Err(QuakeCSourceError::Invalid("duplicate model".to_string()));
                }
                let map_index = self
                    .map_model_index(&name)
                    .map_err(|error| ValueError(error.to_string()))?;
                if map_index.is_some_and(|map_index| index != map_index) {
                    return Err(QuakeCSourceError::Invalid(
                        "saved map model index differs from current world".to_string(),
                    ));
                }
                known.insert(name, SourceModel { index, bounds });
            }
        }
        let precached: Vec<(String, i32, String, String, Option<i32>)> = reader.field("precached").list(|entry| {
            let key = entry.field("key").string()?;
            let name = key.split_once(':').map_or(key.as_str(), |(_, name)| name);
            let map_index = if key.starts_with("model:") {
                self.map_model_index(name)
                    .map_err(|error| ValueError(error.to_string()))?
            } else {
                None
            };
            Ok::<_, QuakeCSourceError>((
                key,
                i32::try_from(entry.field("index").integer(1)?)
                    .map_err(|_| value_error(entry.fail("precache index exceeds i32")))?,
                entry.field("id").string()?,
                entry.field("identity").string()?,
                map_index,
            ))
        })?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.precached.clear();
            for (key, index, id, identity, map_index) in precached {
                let name = key
                    .split_once(':')
                    .map_or_else(|| key.clone(), |(_, name)| name.to_string());
                let (expected_id, expected_identity) = if map_index.is_some() {
                    let geometry = &shared.options.recipe.map.geometry;
                    (geometry.id.clone(), geometry.identity.clone())
                } else if let Some(resource) = self.prepared.resources.get(name.as_str()) {
                    (
                        resource.resource.id.as_str().to_string(),
                        resource.resource.identity.canonical(),
                    )
                } else {
                    (String::new(), String::new())
                };
                if key.split_once(':').is_none_or(|(kind, _)| kind.is_empty())
                    || expected_id != id
                    || expected_identity != identity
                    || shared.precached.contains_key(&key)
                {
                    return Err(QuakeCSourceError::Invalid(
                        "saved precache resource differs from mounted content".to_string(),
                    ));
                }
                if map_index.is_some_and(|map_index| index != map_index) {
                    return Err(QuakeCSourceError::Invalid(
                        "saved inline model index differs from current map".to_string(),
                    ));
                }
                shared.precached.insert(key, QcPrecachedResource { index, path: name });
            }
        }
        {
            let shared = self.shared.borrow();
            let mut router = shared.router.borrow_mut();
            let local = reader.field("localMessages");
            if !local.is_missing() {
                router.local_messages_started = local.field("started").boolean()?;
                let mut decoder = NetQuakeDecoder::new(NqProfile::Netquake, self.prepared.message_dialect, true);
                let baseline = decoder.decode(&local.field("baseline").bytes()?)?;
                let mut clients = Vec::new();
                for entry in local.field("clients").list(|entry| {
                    Ok::<_, QuakeCSourceError>((
                        restore_saved_actor(entry.field("actor"))?,
                        entry.field("bytes").bytes()?,
                    ))
                })? {
                    clients.push(QuakeCLocalClientCapture {
                        actor: actors.borrow().reference_saved(entry.0),
                        messages: decoder.decode(&entry.1)?,
                    });
                }
                router
                    .local_messages
                    .restore(&QuakeCLocalMessageCapture { baseline, clients })?;
                let views = local.field("views");
                if !views.is_missing() {
                    let baseline = views
                        .field("baseline")
                        .nullable(restore_saved_actor)?
                        .map(|saved| actors.borrow().reference_saved(saved));
                    let clients: Vec<QuakeCViewClient> = views.field("clients").list(|entry| {
                        Ok::<_, QuakeCSourceError>(QuakeCViewClient {
                            actor: actors
                                .borrow()
                                .reference_saved(restore_saved_actor(entry.field("actor"))?),
                            target: actors
                                .borrow()
                                .reference_saved(restore_saved_actor(entry.field("target"))?),
                        })
                    })?;
                    router.local_messages.restore_views(baseline, &clients)?;
                } else {
                    let retained = router.local_messages.capture();
                    let has_camera = retained
                        .baseline
                        .iter()
                        .chain(retained.clients.iter().flat_map(|client| client.messages.iter()))
                        .any(|message| matches!(message, NetQuakeMessage::SetView { .. }));
                    if has_camera {
                        return Err(QuakeCSourceError::Invalid(
                            views
                                .fail("Legacy QC camera state has no captured actor identity")
                                .to_string(),
                        ));
                    }
                }
            }
        }
        let model_count = reader.field("modelCount").integer(1)?;
        let sound_count = reader.field("soundCount").integer(1)?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.model_count =
                i32::try_from(model_count).map_err(|_| ValueError("saved model count exceeds i32".to_string()))?;
            shared.sound_count =
                i32::try_from(sound_count).map_err(|_| ValueError("saved sound count exceeds i32".to_string()))?;
        }
        let cvars: Vec<(String, String, String, u32)> = reader.field("cvars").list(|entry| {
            Ok::<_, QuakeCSourceError>((
                entry.field("name").string()?,
                entry.field("value").string()?,
                entry.field("reset").string()?,
                u32::try_from(entry.field("flags").integer(0)?)
                    .map_err(|_| ValueError("saved cvar flags exceed u32".to_string()))?,
            ))
        })?;
        {
            let shared = self.shared.borrow();
            let mut cvars_state = shared.cvars.borrow_mut();
            for (name, value, reset, flags) in cvars {
                if cvars_state.get(&name).is_none() {
                    cvars_state.register(&name, &reset, flags)?;
                }
                cvars_state.set(&name, &value, true)?;
            }
        }
        let messages = restore_messages_checkpoint(reader.field("messages"))?;
        {
            let mut shared = self.shared.borrow_mut();
            shared.messages.restore(&messages, &resolve)?;
        }
        self.shared
            .borrow()
            .projectiles
            .restore(reader.field("projectiles"))
            .map_err(|error| ValueError(error.to_string()))?;
        Ok(())
    }

    /// Inline model index behind one model name (`mapModelIndex`).
    fn map_model_index(&self, name: &str) -> Result<Option<i32>, QuakeCSourceError> {
        let shared = self.shared.borrow();
        if name == shared.options.recipe.map.geometry.requested_path {
            return Ok(Some(1));
        }
        if !name.starts_with('*') {
            return Ok(None);
        }
        let digits = name.strip_prefix('*').unwrap_or("");
        let model = if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            digits.parse::<i64>().unwrap_or(-1)
        } else {
            -1
        };
        let models = shared.options.world.borrow().model_count();
        if model < 1 || model as usize >= models {
            return Err(QuakeCSourceError::Invalid(format!(
                "Invalid actual QC inline model {name}"
            )));
        }
        Ok(Some(model as i32 + 1))
    }

    /// Precache one model or sound (`precache`).
    ///
    /// Wired to the message-router builtins once that lane lands; the
    /// game-surface methods below only read [`precached`](Shared::precached).
    #[allow(dead_code)]
    fn precache(&self, kind: PrecacheKind, name: &str) -> Result<QcPrecachedResource, QuakeCSourceError> {
        let key = format!(
            "{}:{name}",
            match kind {
                PrecacheKind::Model => "model",
                PrecacheKind::Sound => "sound",
            }
        );
        if let Some(prior) = self.shared.borrow().precached.get(&key) {
            return Ok(prior.clone());
        }
        if kind == PrecacheKind::Model {
            if let Some(map_index) = self.map_model_index(name)? {
                let value = QcPrecachedResource {
                    index: map_index,
                    path: name.to_string(),
                };
                self.shared.borrow_mut().precached.insert(key, value.clone());
                return Ok(value);
            }
        }
        let resource = self.prepared.resources.get(name).ok_or_else(|| {
            QuakeCSourceError::Invalid(format!(
                "Unprepared actual QC {} resource {name}",
                match kind {
                    PrecacheKind::Model => "model",
                    PrecacheKind::Sound => "sound",
                }
            ))
        })?;
        let index = match kind {
            PrecacheKind::Model => {
                let mut shared = self.shared.borrow_mut();
                let index = shared.model_count;
                shared.model_count += 1;
                index
            }
            PrecacheKind::Sound => {
                let mut shared = self.shared.borrow_mut();
                let index = shared.sound_count;
                shared.sound_count += 1;
                index
            }
        };
        if index >= 256 {
            return Err(QuakeCSourceError::Invalid(format!(
                "NetQuake {} precache limit exceeded",
                match kind {
                    PrecacheKind::Model => "model",
                    PrecacheKind::Sound => "sound",
                }
            )));
        }
        if kind == PrecacheKind::Model {
            let bounds = resource
                .model_bounds
                .ok_or_else(|| QuakeCSourceError::Invalid(format!("Missing actual model bounds {name}")))?;
            self.shared
                .borrow_mut()
                .models
                .borrow_mut()
                .insert(name.to_string(), SourceModel { index, bounds });
        }
        let value = QcPrecachedResource {
            index,
            path: name.to_string(),
        };
        self.shared.borrow_mut().precached.insert(key, value.clone());
        Ok(value)
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Set one client's match team (`setMatchTeam`).
    pub fn set_match_team(&self, actor: &ActorId, team: Option<&str>) -> Result<(), QuakeCSourceError> {
        let slot = self.source_slot(actor);
        let client = slot.and_then(|slot| self.shared.borrow().client_identities.get(&slot).cloned());
        let (Some(slot), Some(client)) = (slot, client) else {
            return Err(QuakeCSourceError::Invalid(
                "Original team command requires an admitted client".to_string(),
            ));
        };
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid(
                "Original team command requires an admitted client".to_string(),
            ));
        }
        let mut info = self.client_info(&client);
        if self.kind() == QuakeCSourceKind::Quakeworld {
            if let Some(team) = team {
                if team
                    .chars()
                    .any(|char| char == '\\' || char == '"' || char == '\n' || char == '\r')
                {
                    return Err(QuakeCSourceError::Invalid(
                        "Original QW team userinfo is invalid".to_string(),
                    ));
                }
                info.insert("team".to_string(), team.to_string());
            } else {
                info.remove("team");
            }
        } else {
            let value: f64 = match team {
                None => f64::NAN,
                Some(team) => team.trim().parse().unwrap_or(f64::NAN),
            };
            if !value.is_finite() || value.fract() != 0.0 || value < 1.0 || value > 14.0 {
                return Err(QuakeCSourceError::Invalid(
                    "Team has no original Quake color command".to_string(),
                ));
            }
            info.insert("bottomcolor".to_string(), format!("{}", value as i64 - 1));
            let team_word = self.field("team")?;
            self.machine_write(|machine| {
                machine
                    .entities_mut()
                    .set_slot_float(slot as u32, team_word, value as f32)
            })?;
        }
        self.set_client_info(&client, &info)?;
        Ok(())
    }

    /// Match score for one actor (`matchScore`).
    pub fn match_score(&self, actor: &ActorId) -> Result<f64, QuakeCSourceError> {
        let reference = self.reference(actor)?;
        let frags = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities().slot_float(slot, self.must_field("frags"))
        })?;
        Ok(f64::from(frags))
    }

    /// Set one actor's match score (`setMatchScore`).
    pub fn set_match_score(&self, actor: &ActorId, score: f64) -> Result<(), QuakeCSourceError> {
        if !score.is_finite() {
            return Err(QuakeCSourceError::Invalid("QuakeC score must be finite".to_string()));
        }
        let reference = self.reference(actor)?;
        let frags = self.field("frags")?;
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities_mut().set_slot_float(slot, frags, score as f32)
        })?;
        Ok(())
    }

    /// Death-type string for one actor (`deathType`).
    pub fn death_type(&self, actor: &ActorId) -> Result<String, QuakeCSourceError> {
        let Some(definition) = self.prepared.program.field_named("deathtype") else {
            return Ok(String::new());
        };
        let offset = definition.offset;
        let reference = self.reference(actor)?;
        self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let text = machine.entities().slot_int(slot, offset)?;
            machine.strings().get(text)
        })
        .map_err(QuakeCSourceError::from)
    }

    /// Set one client's userinfo, republishing its netname (`setClientInfo`).
    pub fn set_client_info(
        &self,
        client: &ClientId,
        values: &std::collections::HashMap<String, String>,
    ) -> Result<(), QuakeCSourceError> {
        self.set_client_info_storage(client, values)?;
        let slot = client.slot() as usize + 1;
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, slot);
        let current = actor.as_ref().is_some_and(|actor| {
            self.is_active_client(actor.id()) || self.shared.borrow().prepared_clients.contains(&slot)
        });
        if !current {
            return Ok(());
        }
        let name = values.get("name").cloned().unwrap_or_else(|| "unnamed".to_string());
        let netname = self.field("netname")?;
        if self.kind() == QuakeCSourceKind::Quakeworld {
            let engine = format!("qw-name:{slot}");
            self.machine_write(|machine| {
                let offset = machine.strings_mut().set_engine(&engine, &name, 32)?;
                machine.entities_mut().set_slot_int(slot as u32, netname, offset)
            })?;
        } else {
            self.machine_write(|machine| {
                let offset = machine.strings_mut().allocate(&name)?;
                machine.entities_mut().set_slot_int(slot as u32, netname, offset)
            })?;
        }
        Ok(())
    }

    /// Store one client's userinfo without republishing (`setClientInfoStorage`).
    pub fn set_client_info_storage(
        &self,
        client: &ClientId,
        values: &std::collections::HashMap<String, String>,
    ) -> Result<(), QuakeCSourceError> {
        if client.slot() as usize >= self.shared.borrow().options.max_clients {
            return Err(QuakeCSourceError::Invalid(
                "QC userinfo slot is unavailable".to_string(),
            ));
        }
        self.shared
            .borrow_mut()
            .user_info
            .insert(client.slot() as usize + 1, values.clone());
        Ok(())
    }

    /// Whether an actor is a QuakeWorld spectator (`isSpectatorClient`).
    pub fn is_spectator_client(&self, actor: &ActorId) -> bool {
        let Some(slot) = self.source_slot(actor) else {
            return false;
        };
        self.kind() == QuakeCSourceKind::Quakeworld && self.shared.borrow().spectator_slots.contains(&slot)
    }

    /// Run one spectator callback when the program defines it (`spectatorCallback`).
    fn spectator_callback(&self, name: &str, slot: usize) -> Result<(), QuakeCSourceError> {
        let callback = self
            .prepared
            .program
            .function_named(name)
            .ok()
            .map(|function| function.index);
        if let Some(index) = callback {
            if index != 0 {
                let time = self.current_time();
                self.invoke(index as i32, slot, 0, time)?;
            }
        }
        Ok(())
    }

    /// Reserve (or re-reserve) one client's source row (`reservedClient`).
    pub fn reserved_client(&self, client: &ClientId) -> Result<OwnedActor, QuakeCSourceError> {
        let base = client.slot() as usize;
        if base >= self.shared.borrow().options.max_clients || self.loading() {
            return Err(QuakeCSourceError::Invalid(
                "QC reserved client is unavailable".to_string(),
            ));
        }
        let slot = base + 1;
        let existing = self.shared.borrow().client_identities.get(&slot).cloned();
        if let Some(existing) = &existing {
            if existing != client {
                return Err(QuakeCSourceError::Invalid(
                    "QC client slot still belongs to an earlier connection".to_string(),
                ));
            }
        }
        if existing.is_none() && self.kind() == QuakeCSourceKind::Netquake {
            let classname_word = self.field("classname")?;
            let classname = self.machine_read(|machine| {
                let text = machine.entities().slot_int(slot as u32, classname_word)?;
                machine.strings().get(text)
            })?;
            if classname == "player" {
                let provider = self.prepared.execution.owner.provider.clone();
                let actors = self.shared.borrow().options.actors.clone();
                if let Some(previous) = actors.borrow().at_source(&provider, slot) {
                    actors.borrow_mut().release(&previous);
                }
                self.machine_write(|machine| machine.entities_mut().set_slot_int(slot as u32, classname_word, 0))?;
            }
        }
        self.shared.borrow_mut().client_identities.insert(slot, client.clone());
        if !self.shared.borrow().spawn_parameters.contains_key(&slot) {
            let index = self.prepared.program.function_named("SetNewParms")?.index;
            let time = self.current_time();
            self.invoke(index as i32, 0, 0, time)?;
            let mut parameters = Vec::with_capacity(16);
            for ordinal in 1..=16 {
                let name = format!("parm{ordinal}");
                let value = self.machine_read(|machine| {
                    let offset = machine.global_offset(&name)?;
                    machine.globals().float(offset)
                })?;
                parameters.push(f64::from(value));
            }
            self.shared.borrow_mut().spawn_parameters.insert(slot, parameters);
        }
        self.slot_actor(slot)
    }

    /// Stored userinfo for one client (`clientInfo`).
    pub fn client_info(&self, client: &ClientId) -> std::collections::HashMap<String, String> {
        self.shared
            .borrow()
            .user_info
            .get(&(client.slot() as usize + 1))
            .cloned()
            .unwrap_or_default()
    }

    /// Capture level-travel state (`captureTravel`).
    pub fn capture_travel(&self) -> Result<QuakeCSourceTravel, QuakeCSourceError> {
        if self.loading() {
            return Err(QuakeCSourceError::Invalid(
                "Native QuakeC travel requires a loaded source world".to_string(),
            ));
        }
        let server_flags = self.machine_read(|machine| {
            let offset = machine.global_offset("serverflags")?;
            machine.globals().float(offset)
        })?;
        let mut slots: Vec<usize> = self.shared.borrow().client_identities.keys().copied().collect();
        slots.sort_unstable();
        let mut clients = Vec::with_capacity(slots.len());
        for slot in slots {
            let client = self
                .shared
                .borrow()
                .client_identities
                .get(&slot)
                .cloned()
                .ok_or_else(|| {
                    QuakeCSourceError::Invalid("Native QuakeC connection has no spawn parameters".to_string())
                })?;
            let provider = self.prepared.execution.owner.provider.clone();
            let actors = self.shared.borrow().options.actors.clone();
            let actor = actors.borrow().at_source(&provider, slot);
            if let Some(actor) = &actor {
                if self.is_active_client(actor.id()) {
                    let index = self.prepared.program.function_named("SetChangeParms")?.index;
                    let time = self.current_time();
                    self.invoke(index as i32, slot, 0, time)?;
                    let mut parameters = Vec::with_capacity(16);
                    for ordinal in 1..=16 {
                        let name = format!("parm{ordinal}");
                        let value = self.machine_read(|machine| {
                            let offset = machine.global_offset(&name)?;
                            machine.globals().float(offset)
                        })?;
                        parameters.push(f64::from(value));
                    }
                    self.shared.borrow_mut().spawn_parameters.insert(slot, parameters);
                }
            }
            let parameters = self
                .shared
                .borrow()
                .spawn_parameters
                .get(&slot)
                .cloned()
                .ok_or_else(|| {
                    QuakeCSourceError::Invalid("Native QuakeC connection has no spawn parameters".to_string())
                })?;
            let role = if self.shared.borrow().spectator_slots.contains(&slot) {
                QuakeCClientRole::Spectator
            } else {
                QuakeCClientRole::Player
            };
            let user_info = self.shared.borrow().user_info.get(&slot).cloned().unwrap_or_default();
            clients.push(QuakeCSourceClient {
                client,
                parameters,
                user_info,
                role: Some(role),
            });
        }
        let cvars = self
            .shared
            .borrow()
            .cvars
            .borrow()
            .snapshots(0)
            .into_iter()
            .map(|variable| CvarNameValue {
                name: variable.name.clone(),
                value: variable.latched_value.clone().unwrap_or(variable.value.clone()),
            })
            .collect();
        Ok(QuakeCSourceTravel {
            kind: self.kind(),
            server_flags: server_flags as i32,
            cvars,
            clients,
        })
    }

    /// Restore level-travel state into a fresh world (`restoreTravel`).
    pub fn restore_travel(&self, travel: &QuakeCSourceTravel) -> Result<(), QuakeCSourceError> {
        if travel.kind != self.kind() || !self.loading() || !self.shared.borrow().client_identities.is_empty() {
            return Err(QuakeCSourceError::Invalid(
                "Native QuakeC travel requires a fresh source world with the same ABI".to_string(),
            ));
        }
        self.machine_write(|machine| {
            let offset = machine.global_offset("serverflags")?;
            machine.globals_mut().set_float(offset, travel.server_flags as f32)
        })?;
        {
            let cvars = self.shared.borrow().cvars.clone();
            let mut cvars = cvars.borrow_mut();
            for variable in &travel.cvars {
                if cvars.get(&variable.name).is_none() {
                    cvars.register(&variable.name, &variable.value, 0)?;
                }
                cvars.set(&variable.name, &variable.value, true)?;
            }
        }
        for record in &travel.clients {
            let slot = record.client.slot() as usize + 1;
            let reserved = self.shared.borrow().reserved_client_slots;
            if slot < 1
                || slot > reserved
                || self.shared.borrow().client_identities.contains_key(&slot)
                || record.parameters.len() != 16
                || record.parameters.iter().any(|value| !value.is_finite())
            {
                return Err(QuakeCSourceError::Invalid(
                    "Invalid native QuakeC travel client parameters".to_string(),
                ));
            }
            if record.role == Some(QuakeCClientRole::Spectator) {
                if self.kind() != QuakeCSourceKind::Quakeworld {
                    return Err(QuakeCSourceError::Invalid(
                        "NetQuake travel cannot contain a spectator".to_string(),
                    ));
                }
                self.shared.borrow_mut().spectator_slots.insert(slot);
            }
            let mut shared = self.shared.borrow_mut();
            shared.client_identities.insert(slot, record.client.clone());
            shared.spawn_parameters.insert(slot, record.parameters.clone());
            shared.user_info.insert(slot, record.user_info.clone());
        }
        Ok(())
    }

    /// Connected client identities in slot order (`connectedClientIdentities`).
    pub fn connected_client_identities(&self) -> Vec<ClientId> {
        let mut clients: Vec<ClientId> = self.shared.borrow().client_identities.values().cloned().collect();
        clients.sort_by_key(|client| client.slot());
        clients
    }

    /// Live actor for one connected client (`clientActor`).
    pub fn client_actor(&self, client: &ClientId) -> Option<ActorId> {
        if !self.has_client(client) {
            return None;
        }
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, client.slot() as usize + 1);
        actor.map(|actor| actor.id().clone())
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Run one host cheat for an admitted client (`hostCheat`).
    pub fn host_cheat(&self, actor: &ActorId, name: QuakeCCheat, args: &[String]) -> Result<(), QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("QC host command requires an admitted client".to_string()))?;
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid(
                "QC host command requires an admitted client".to_string(),
            ));
        }
        let message = |text: String| {
            let events = self.shared.borrow().options.events.clone();
            events
                .borrow_mut()
                .message(ClientMessage::Print { level: 2, text }, Some(actor));
        };
        let denied = if self.kind() == QuakeCSourceKind::Quakeworld {
            self.shared.borrow().cvars.borrow().variable_value("sv_cheats") == 0.0
        } else {
            self.machine_read(|machine| {
                let offset = machine.global_offset("deathmatch")?;
                machine.globals().float(offset)
            })? != 0.0
        };
        if denied {
            message("Cheats are disabled on this server.\n".to_string());
            return Ok(());
        }
        if name == QuakeCCheat::Give {
            self.host_give(actor, slot, args)?;
            if self.prepared.program.function_named("W_SetCurrentAmmo").is_ok() {
                let selected = self
                    .shared
                    .borrow()
                    .options
                    .primary_weapon_selected
                    .clone()
                    .is_none_or(|predicate| predicate(actor));
                if selected {
                    let index = self.prepared.program.function_named("W_SetCurrentAmmo")?.index;
                    let time = self.current_time();
                    self.invoke(index as i32, slot, 0, time)?;
                }
            }
            return Ok(());
        }
        let (label, enabled) = match name {
            QuakeCCheat::Noclip | QuakeCCheat::Fly => {
                let move_type = if name == QuakeCCheat::Fly { 5.0 } else { 8.0 };
                let word = self.field("movetype")?;
                let enabled =
                    self.machine_read(|machine| machine.entities().slot_float(slot as u32, word))? != move_type;
                self.machine_write(|machine| {
                    machine
                        .entities_mut()
                        .set_slot_float(slot as u32, word, if enabled { move_type } else { 3.0 })
                })?;
                (if name == QuakeCCheat::Fly { "fly" } else { "noclip" }, enabled)
            }
            _ => {
                let bit = if name == QuakeCCheat::God { 64 } else { 128 };
                let word = self.field("flags")?;
                let flags = self.machine_read(|machine| machine.entities().slot_float(slot as u32, word))? as i32 ^ bit;
                self.machine_write(|machine| machine.entities_mut().set_slot_float(slot as u32, word, flags as f32))?;
                (
                    if name == QuakeCCheat::God {
                        "godmode"
                    } else {
                        "notarget"
                    },
                    flags & bit != 0,
                )
            }
        };
        message(format!("{label} {}\n", if enabled { "ON" } else { "OFF" }));
        Ok(())
    }

    /// Grant cheat items to one client slot (`hostGive`).
    fn host_give(&self, actor: &ActorId, slot: usize, args: &[String]) -> Result<(), QuakeCSourceError> {
        let Some(input) = args.first().map(|arg| arg.to_lowercase()) else {
            return Err(QuakeCSourceError::Invalid(
                "Usage: give <all|weapons|ammo|health|armor|keys|item> [amount]".to_string(),
            ));
        };
        if input != "all" {
            let give = self.shared.borrow().options.give_inventory.clone();
            if give.is_some_and(|give| give(actor, args)) {
                return Ok(());
            }
        }
        let amount = match args.get(1) {
            None => None,
            Some(text) => Some(native_atoi(text).map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?),
        };
        let all = input == "all";
        let write = |machine: &mut QcMachine, name: &str, value: f64| -> Result<(), QuakeCSourceError> {
            let word = self.field(name)?;
            machine.entities_mut().set_slot_float(slot as u32, word, value as f32)?;
            Ok(())
        };
        if all || input == "health" || input == "h" {
            let value = amount.map_or(if input == "h" { 0.0 } else { 100.0 }, f64::from);
            self.machine_write(|machine| write(machine, "health", value))?;
            if !all {
                return Ok(());
            }
        }
        if all || input == "armor" || input == "a" {
            let points = amount.map_or(if input == "a" { 0.0 } else { 200.0 }, f64::from);
            let armor_bit = if points > 150.0 {
                32768
            } else if points > 100.0 {
                16384
            } else if points > 0.0 {
                8192
            } else {
                0
            };
            let items = self.field("items")?;
            self.machine_write(|machine| {
                let held = machine.entities().slot_float(slot as u32, items)? as i32;
                machine.entities_mut().set_slot_float(
                    slot as u32,
                    items,
                    ((held & !(8192 | 16384 | 32768)) | armor_bit) as f32,
                )?;
                write(machine, "armorvalue", points)?;
                write(
                    machine,
                    "armortype",
                    if points > 150.0 {
                        0.8
                    } else if points > 100.0 {
                        0.6
                    } else if points > 0.0 {
                        0.3
                    } else {
                        0.0
                    },
                )
            })?;
            if !all {
                return Ok(());
            }
        }
        if all || input == "weapons" {
            let give = self.shared.borrow().options.give_inventory.clone();
            let handled = give.is_some_and(|give| give(actor, &["weapons".to_string()]));
            if !handled {
                let weapons = self.shared.borrow().weapons.clone();
                let items = self.field("items")?;
                self.machine_write(|machine| {
                    let mut held = machine.entities().slot_float(slot as u32, items)? as i32;
                    for weapon in &weapons {
                        held |= weapon.bit;
                    }
                    machine.entities_mut().set_slot_float(slot as u32, items, held as f32)
                })?;
            }
            if !all {
                return Ok(());
            }
        }
        if all || input == "ammo" {
            let give = self.shared.borrow().options.give_inventory.clone();
            let handled = give.is_some_and(|give| give(actor, &["ammo".to_string()]));
            if !handled {
                self.machine_write(|machine| {
                    for (field, value) in [
                        ("ammo_shells", 100.0),
                        ("ammo_nails", 200.0),
                        ("ammo_rockets", 100.0),
                        ("ammo_cells", 100.0),
                    ] {
                        write(machine, field, value)?;
                    }
                    for field in [
                        "ammo_shells1",
                        "ammo_nails1",
                        "ammo_rockets1",
                        "ammo_cells1",
                        "ammo_lava_nails",
                        "ammo_multi_rockets",
                        "ammo_plasma",
                    ] {
                        if let Some(definition) = self.prepared.program.field_named(field) {
                            let value = if field.contains("nails") { 200.0 } else { 100.0 };
                            machine
                                .entities_mut()
                                .set_slot_float(slot as u32, definition.offset, value)?;
                        }
                    }
                    Ok::<_, QuakeCSourceError>(())
                })?;
            }
            if !all {
                return Ok(());
            }
        }
        if all || input == "keys" {
            let items = self.field("items")?;
            self.machine_write(|machine| {
                let held = machine.entities().slot_float(slot as u32, items)? as i32;
                machine
                    .entities_mut()
                    .set_slot_float(slot as u32, items, (held | (131072 | 262144)) as f32)
            })?;
            return Ok(());
        }
        if input == "items" {
            for item in ["quad", "pent", "ring", "suit"] {
                self.host_give(actor, slot, &[item.to_string()])?;
            }
            return Ok(());
        }
        let hipnotic = self
            .shared
            .borrow()
            .weapons
            .iter()
            .any(|weapon| weapon.item == "q1:weapon/hipnotic:laser");
        let named: Option<String> = if hipnotic && input == "6a" {
            Some("q1:weapon/hipnotic:proximity".to_string())
        } else if hipnotic && input == "9" {
            Some("q1:weapon/hipnotic:laser".to_string())
        } else if hipnotic && input == "0" {
            Some("q1:weapon/hipnotic:mjolnir".to_string())
        } else if input.len() == 1 && matches!(input.as_bytes()[0], b'2'..=b'8') {
            let ordinal: usize = input.parse().unwrap_or(0);
            self.shared
                .borrow()
                .weapons
                .get(ordinal.wrapping_sub(1))
                .map(|weapon| weapon.item.clone())
        } else {
            None
        };
        let weapons = self.shared.borrow().weapons.clone();
        if let Some(weapon) = weapons.iter().find(|weapon| {
            Some(weapon.item.as_str()) == named.as_deref()
                || weapon.item == input
                || weapon.item.rsplit('/').next() == Some(input.as_str())
        }) {
            let bit = weapon.bit;
            let items = self.field("items")?;
            self.machine_write(|machine| {
                let held = machine.entities().slot_float(slot as u32, items)? as i32;
                machine
                    .entities_mut()
                    .set_slot_float(slot as u32, items, (held | bit) as f32)
            })?;
            return Ok(());
        }
        let ammo = match input.as_str() {
            "s" | "shells" => Some("ammo_shells"),
            "n" | "nails" => Some("ammo_nails"),
            "r" | "rockets" => Some("ammo_rockets"),
            "c" | "cells" => Some("ammo_cells"),
            "l" => Some("ammo_lava_nails"),
            "m" => Some("ammo_multi_rockets"),
            "p" => Some("ammo_plasma"),
            _ => None,
        };
        if let Some(ammo) = ammo {
            let value = amount.map_or(0.0, f64::from);
            let alternate = self
                .prepared
                .program
                .field_named(&format!("{ammo}1"))
                .map(|definition| definition.offset);
            let weapon_word = self.field("weapon")?;
            self.machine_write(|machine| {
                if let Some(alternate) = alternate {
                    machine
                        .entities_mut()
                        .set_slot_float(slot as u32, alternate, value as f32)?;
                }
                let current = machine.entities().slot_float(slot as u32, weapon_word)?;
                if alternate.is_none() || ammo == "ammo_shells" || current <= 64.0 {
                    write(machine, ammo, value)?;
                }
                let native = match ammo {
                    "ammo_lava_nails" => Some("ammo_nails"),
                    "ammo_multi_rockets" => Some("ammo_rockets"),
                    "ammo_plasma" => Some("ammo_cells"),
                    _ => None,
                };
                if let Some(native) = native {
                    let current = machine.entities().slot_float(slot as u32, weapon_word)?;
                    if current > 64.0 {
                        write(machine, native, value)?;
                    }
                }
                Ok::<_, QuakeCSourceError>(())
            })?;
            return Ok(());
        }
        let classname = match input.as_str() {
            "quad" => "item_artifact_super_damage",
            "pent" => "item_artifact_invulnerability",
            "ring" => "item_artifact_invisibility",
            "suit" => "item_artifact_envirosuit",
            _ => input.as_str(),
        };
        if !classname.starts_with("item_") && !classname.starts_with("weapon_") {
            return Err(QuakeCSourceError::Invalid(format!("Unknown QuakeC item: {input}")));
        }
        let mut template: Option<Vec<u8>> = None;
        let reserved = self.shared.borrow().reserved_client_slots;
        let count = self.machine_read(|machine| machine.entities().count());
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let touch_word = self.field("touch")?;
        let classname_word = self.field("classname")?;
        for candidate in reserved + 1..count {
            let owner = actors.borrow().at_source(&provider, candidate);
            let live = owner.as_ref().is_some_and(|owner| actors.borrow().is_live(owner.id()));
            if !live {
                continue;
            }
            let found = self.machine_read(|machine| {
                let touched = machine.entities().slot_int(candidate as u32, touch_word)?;
                if touched == 0 {
                    return Ok::<_, GuestError>(false);
                }
                let text = machine.entities().slot_int(candidate as u32, classname_word)?;
                Ok(machine.strings().get(text).is_ok_and(|name| name == classname))
            })?;
            if found {
                template =
                    Some(self.machine_read(|machine| {
                        machine.entities().field_bytes(candidate as u32).map(<[u8]>::to_vec)
                    })?);
                break;
            }
        }
        let Some(template) = template else {
            return Err(QuakeCSourceError::Invalid(format!(
                "Cannot give {classname}: this map has no source item template"
            )));
        };
        let spawn = self.prepared.program.function_named("spawn")?.index;
        self.execute_guarded(spawn, 0)?;
        let reference = self.machine_read(|machine| machine.globals().int(1))?;
        let item_slot = self.machine_read(|machine| machine.entities().slot(reference))? as usize;
        let item = self.slot_actor(item_slot)?;
        let origin_word = self.field("origin")?;
        let solid_word = self.field("solid")?;
        let nextthink_word = self.field("nextthink")?;
        let target_words: Vec<usize> = ["target", "targetname", "killtarget"]
            .into_iter()
            .filter_map(|name| {
                self.prepared
                    .program
                    .field_named(name)
                    .map(|definition| definition.offset)
            })
            .collect();
        let origin = self.machine_read(|machine| machine.entities().slot_vector(slot as u32, origin_word))?;
        self.machine_write(|machine| {
            machine
                .entities_mut()
                .field_bytes_mut(item_slot as u32)?
                .copy_from_slice(&template);
            machine
                .entities_mut()
                .set_slot_vector(item_slot as u32, origin_word, origin)?;
            machine
                .entities_mut()
                .set_slot_float(item_slot as u32, solid_word, 1.0)?;
            machine
                .entities_mut()
                .set_slot_float(item_slot as u32, nextthink_word, 0.0)?;
            for word in &target_words {
                machine.entities_mut().set_slot_int(item_slot as u32, *word, 0)?;
            }
            Ok::<_, GuestError>(())
        })?;
        let touch = self.machine_read(|machine| machine.entities().slot_int(item_slot as u32, touch_word))?;
        let target = self.reference(actor)?;
        let time = self.current_time();
        let outcome = self.invoke(touch, item_slot, target, time);
        let actors = self.shared.borrow().options.actors.clone();
        if actors.borrow().is_live(item.id()) {
            actors.borrow_mut().release(&item);
        }
        outcome
    }

    /// Prepare one QuakeWorld client's spawn entity (`prepareClientSpawn`,
    /// donor `quakec-source.ts` 720-731).
    pub fn prepare_client_spawn(&self, client: &ClientId) -> Result<(), QuakeCSourceError> {
        let actor = self.reserved_client(client)?;
        let slot = client.slot() as usize + 1;
        if self.kind() != QuakeCSourceKind::Quakeworld || self.is_active_client(actor.id()) {
            return Err(QuakeCSourceError::Invalid(
                "QW spawn requires an inactive reserved client".to_string(),
            ));
        }
        let colormap = self.field("colormap")?;
        let team = self.field("team")?;
        let netname = self.field("netname")?;
        let name = self
            .shared
            .borrow()
            .user_info
            .get(&slot)
            .and_then(|info| info.get("name").cloned())
            .unwrap_or_else(|| "unnamed".to_string());
        let maxspeed = self.shared.borrow().cvars.borrow().variable_value("sv_maxspeed");
        let gravity = self
            .prepared
            .program
            .field_named("gravity")
            .map(|definition| definition.offset);
        let maxspeed_field = self
            .prepared
            .program
            .field_named("maxspeed")
            .map(|definition| definition.offset);
        let engine = format!("qw-name:{slot}");
        self.machine_write(|machine| {
            machine.entities_mut().clear_slot(slot as u32)?;
            machine
                .entities_mut()
                .set_slot_float(slot as u32, colormap, slot as f32)?;
            machine.entities_mut().set_slot_float(slot as u32, team, 0.0)?;
            let offset = machine.strings_mut().set_engine(&engine, &name, 32)?;
            machine.entities_mut().set_slot_int(slot as u32, netname, offset)?;
            if let Some(word) = gravity {
                machine.entities_mut().set_slot_float(slot as u32, word, 1.0)?;
            }
            if let Some(word) = maxspeed_field {
                machine.entities_mut().set_slot_float(slot as u32, word, maxspeed)?;
            }
            Ok::<_, GuestError>(())
        })?;
        self.shared.borrow_mut().prepared_clients.insert(slot);
        Ok(())
    }

    /// Admit one reserved client into the server (`admitClient`).
    pub fn admit_client(&self, client: &ClientId) -> Result<OwnedActor, QuakeCSourceError> {
        let slot = client.slot() as usize + 1;
        if slot > self.shared.borrow().options.max_clients || self.loading() {
            return Err(QuakeCSourceError::Invalid("QC client slot is unavailable".to_string()));
        }
        let reserved = self.reserved_client(client)?;
        if self.is_active_client(reserved.id()) {
            return Err(QuakeCSourceError::Invalid("QC reserved client is active".to_string()));
        }
        if self.classname(reserved.id())? == "player" {
            self.shared.borrow().options.actors.borrow_mut().release(&reserved);
        }
        let actor = self.slot_actor(slot)?;
        let parameters = self
            .shared
            .borrow()
            .spawn_parameters
            .get(&slot)
            .cloned()
            .ok_or_else(|| {
                QuakeCSourceError::Invalid("Native QuakeC client has no source spawn parameters".to_string())
            })?;
        if self.kind() == QuakeCSourceKind::Quakeworld {
            if !self.shared.borrow().prepared_clients.contains(&slot) {
                return Err(QuakeCSourceError::Invalid(
                    "QW begin requires completed source spawn preparation".to_string(),
                ));
            }
        } else {
            self.machine_write(|machine| machine.entities_mut().clear_slot(slot as u32))?;
            let colormap = self.field("colormap")?;
            let team = self.field("team")?;
            let netname = self.field("netname")?;
            let name = self
                .shared
                .borrow()
                .user_info
                .get(&slot)
                .and_then(|info| info.get("name").cloned())
                .unwrap_or_else(|| format!("Player {slot}"));
            self.machine_write(|machine| {
                machine
                    .entities_mut()
                    .set_slot_float(slot as u32, colormap, slot as f32)?;
                machine.entities_mut().set_slot_float(slot as u32, team, 1.0)?;
                let offset = machine.strings_mut().allocate(&name)?;
                machine.entities_mut().set_slot_int(slot as u32, netname, offset)
            })?;
        }
        let spectator = self.shared.borrow().spectator_slots.contains(&slot);
        let spectator_connect = self.prepared.program.function_named("SpectatorConnect").ok();
        if !spectator || spectator_connect.is_some_and(|function| function.index != 0) {
            self.machine_write(|machine| {
                for (index, value) in parameters.iter().enumerate() {
                    let offset = machine.global_offset(&format!("parm{}", index + 1))?;
                    machine.globals_mut().set_float(offset, *value as f32)?;
                }
                Ok::<_, GuestError>(())
            })?;
        }
        let replay_local_presentation = self.shared.borrow().router.borrow().local_messages_started;
        self.shared
            .borrow()
            .active_clients
            .borrow_mut()
            .insert(actor.id().clone());
        if self.kind() == QuakeCSourceKind::Netquake {
            let router = self.shared.borrow().router.clone();
            let start = {
                let state = router.borrow();
                !state.netquake_wire_attached || state.local_messages_started
            };
            if start {
                let fresh = !router.borrow().local_messages_started;
                if fresh {
                    router.borrow_mut().local_messages_started = true;
                    router.borrow_mut().local_messages.admit(actor.id());
                    let (signon, views): (Vec<NetQuakeMessage>, Vec<(usize, Option<ActorId>)>) = {
                        let state = router.borrow();
                        (
                            state.netquake_signon.iter().map(netquake_message_from_nq).collect(),
                            state.netquake_signon_views.clone(),
                        )
                    };
                    let views: std::collections::HashMap<usize, Option<ActorId>> = views.into_iter().collect();
                    self.receive_local_messages(&signon, &QcMessageDestination::Signon, &views)?;
                    type RoutedSignonBatch = (
                        Vec<NetQuakeMessage>,
                        QcMessageDestination,
                        Vec<(usize, Option<ActorId>)>,
                    );
                    let routed: Vec<RoutedSignonBatch> = {
                        let state = router.borrow();
                        state
                            .netquake_routed
                            .iter()
                            .map(|entry| {
                                (
                                    entry.messages.iter().map(netquake_message_from_nq).collect(),
                                    entry.destination.clone(),
                                    entry.view_targets.clone(),
                                )
                            })
                            .collect()
                    };
                    for (messages, destination, views) in &routed {
                        let views: std::collections::HashMap<usize, Option<ActorId>> = views.iter().cloned().collect();
                        self.receive_local_messages(messages, destination, &views)?;
                    }
                }
                router.borrow_mut().local_messages.admit(actor.id());
            }
        }
        self.bind_client_inventory(&actor, slot)?;
        if spectator {
            let origin_word = self.field("origin")?;
            let view_word = self.field("view_ofs")?;
            let classname_word = self.field("classname")?;
            self.machine_write(|machine| {
                machine
                    .entities_mut()
                    .set_slot_vector(slot as u32, origin_word, Vec3 { x: 0.0, y: 0.0, z: 0.0 })?;
                machine.entities_mut().set_slot_vector(
                    slot as u32,
                    view_word,
                    Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 22.0,
                    },
                )
            })?;
            let reserved_slots = self.shared.borrow().reserved_client_slots;
            let count = self.machine_read(|machine| machine.entities().count());
            for candidate in reserved_slots - 1..count {
                let info = self.machine_read(|machine| {
                    let text = machine.entities().slot_int(candidate as u32, classname_word)?;
                    let name = machine.strings().get(text)?;
                    let origin = machine.entities().slot_vector(candidate as u32, origin_word)?;
                    Ok::<_, GuestError>((name, origin))
                })?;
                if info.0 != "info_player_start" {
                    continue;
                }
                self.machine_write(|machine| machine.entities_mut().set_slot_vector(slot as u32, origin_word, info.1))?;
                break;
            }
            self.spectator_callback("SpectatorConnect", slot)?;
        } else {
            let connect = self.prepared.program.function_named("ClientConnect")?.index;
            let time = self.current_time();
            self.invoke(connect as i32, slot, 0, time)?;
            let spawn = self
                .shared
                .borrow()
                .weapon_stage
                .and_then(|stage| stage.stage.client.as_ref().map(|client| client.spawn.clone()));
            match spawn {
                None => {
                    let put = self.prepared.program.function_named("PutClientInServer")?.index;
                    self.invoke(put as i32, slot, 0, time)?;
                }
                Some(spawn) => {
                    self.invoke_client_stage(&spawn, actor.id())?;
                }
            }
        }
        if replay_local_presentation {
            let presentation = self
                .shared
                .borrow()
                .router
                .borrow()
                .local_messages
                .presentation(actor.id());
            for message in &presentation {
                self.receive_local_messages(
                    std::slice::from_ref(message),
                    &QcMessageDestination::Client {
                        actor: actor.id().clone(),
                    },
                    &std::collections::HashMap::new(),
                )?;
            }
        }
        Ok(actor)
    }

    /// Run the `ClientKill` callback for one client (`clientKill`).
    pub fn client_kill(&self, actor: &ActorId) -> Result<bool, QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor) else {
            return Ok(false);
        };
        if !self.is_active_client(actor) || self.is_spectator_client(actor) {
            return Ok(false);
        }
        let health = self.field("health")?;
        let alive = self.machine_read(|machine| machine.entities().slot_float(slot as u32, health))? > 0.0;
        if !alive {
            return Ok(false);
        }
        let index = self.prepared.program.function_named("ClientKill")?.index;
        let time = self.current_time();
        self.invoke(index as i32, slot, 0, time)?;
        Ok(true)
    }

    /// Whether a client identity is connected (`hasClient`).
    pub fn has_client(&self, client: &ClientId) -> bool {
        self.shared
            .borrow()
            .client_identities
            .get(&(client.slot() as usize + 1))
            == Some(client)
    }

    /// Disconnect one reserved client (`disconnectClient`).
    pub fn disconnect_client(&self, actor: &OwnedActor) -> Result<(), QuakeCSourceError> {
        let slot = match self.source_slot(actor.id()) {
            Some(slot) if self.is_reserved_client(actor.id()) => slot,
            _ => {
                return Err(QuakeCSourceError::Invalid(
                    "QC disconnect requires a reserved client".to_string(),
                ));
            }
        };
        if self.is_active_client(actor.id()) {
            if self.is_spectator_client(actor.id()) {
                self.spectator_callback("SpectatorDisconnect", slot)?;
            } else {
                let index = self.prepared.program.function_named("ClientDisconnect")?.index;
                let time = self.current_time();
                self.invoke(index as i32, slot, 0, time)?;
            }
        }
        let mut shared = self.shared.borrow_mut();
        shared.spectator_slots.remove(&slot);
        shared.active_clients.borrow_mut().remove(actor.id());
        shared.pending_weapons.borrow_mut().remove(actor.id());
        shared.prepared_clients.remove(&slot);
        shared.spawn_parameters.remove(&slot);
        shared.user_info.remove(&slot);
        shared.client_identities.remove(&slot);
        drop(shared);
        self.shared
            .borrow()
            .router
            .borrow_mut()
            .local_messages
            .retire(actor.id());
        Ok(())
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Whether one client's staged weapon left continuations (`clientWeaponSettled`).
    pub fn client_weapon_settled(&self, actor: &ActorId) -> Result<bool, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC weapon client".to_string()))?;
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid("Missing QC weapon client".to_string()));
        }
        let stage = self
            .shared
            .borrow()
            .weapon_stage
            .ok_or_else(|| QuakeCSourceError::Invalid("QC artifact has no qualified weapon stage".to_string()))?;
        let reference = self.machine_read(|machine| machine.entities().reference(slot as u32))?;
        stage.settled(reference).map_err(QuakeCSourceError::from)
    }

    /// Equipment snapshot for one client (`clientEquipment`).
    pub fn client_equipment(&self, actor: &ActorId) -> Result<QuakeCEquipment, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC equipment client".to_string()))?;
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid("Missing QC equipment client".to_string()));
        }
        let max_health = self.field("max_health")?;
        let quad_until = self.field("super_damage_finished")?;
        let (max_health, quad_until) = self.machine_read(|machine| {
            let max = machine.entities().slot_float(slot as u32, max_health)?;
            let quad = machine.entities().slot_float(slot as u32, quad_until)?;
            Ok::<_, GuestError>((f64::from(max), f64::from(quad)))
        })?;
        Ok(QuakeCEquipment { max_health, quad_until })
    }

    /// Scaled damage amount for one attacker (`damageAmount`).
    pub fn damage_amount(&self, actor: Option<&ActorId>, amount: f64) -> Result<f64, QuakeCSourceError> {
        let world;
        let owner = match actor {
            Some(actor) => actor,
            None => {
                world = self.world_actor()?;
                world.id()
            }
        };
        let reference = self.reference(owner)?;
        let damage = self.shared.borrow().damage;
        let time = self.current_time();
        if let Some(scaled) = damage.damage_amount(owner, reference, time, amount)? {
            return Ok(scaled);
        }
        let machine_view = self.shared.borrow().machine_view;
        let program_view = self.shared.borrow().program_view;
        let multiplier = id1_damage_multiplier(program_view, machine_view, reference, reference)?;
        let scaled = f64::from(amount as f32) * f64::from(multiplier as f32);
        Ok(f64::from(scaled as f32))
    }

    /// Powerup expiry for one client (`clientPowerupExpires`).
    pub fn client_powerup_expires(&self, actor: &ActorId, powerup: QuakeCPowerup) -> Result<f64, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC powerup client".to_string()))?;
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid("Missing QC powerup client".to_string()));
        }
        let field = match powerup {
            QuakeCPowerup::Quad => "super_damage_finished",
            QuakeCPowerup::Invulnerability => "invincible_finished",
            QuakeCPowerup::Invisibility => "invisible_finished",
            QuakeCPowerup::Suit => "radsuit_finished",
        };
        let word = self.field(field)?;
        let expires = self.machine_read(|machine| machine.entities().slot_float(slot as u32, word))?;
        Ok(f64::from(expires))
    }

    /// Combat target flags for one actor (`weaponTarget`).
    pub fn weapon_target(&self, actor: &ActorId) -> Result<Option<QuakeCWeaponTarget>, QuakeCSourceError> {
        let actors = self.shared.borrow().options.actors.clone();
        let Some(slot) = self.source_slot(actor) else {
            return Ok(None);
        };
        if !actors.borrow().is_live(actor) {
            return Ok(None);
        }
        let flags = self.field("flags")?;
        let takedamage = self.field("takedamage")?;
        let (flags, takedamage) = self.machine_read(|machine| {
            let flags = machine.entities().slot_float(slot as u32, flags)?;
            let takedamage = machine.entities().slot_float(slot as u32, takedamage)?;
            Ok::<_, GuestError>((flags, takedamage))
        })?;
        Ok(Some(QuakeCWeaponTarget {
            monster: flags as i32 & 32 != 0,
            aimed_damage: takedamage == 2.0,
        }))
    }

    /// Set one client's maximum health (`setClientMaxHealth`).
    pub fn set_client_max_health(&self, actor: &ActorId, value: f64) -> Result<(), QuakeCSourceError> {
        if !(value as f32).is_finite() {
            return Err(QuakeCSourceError::Invalid(
                "QC max health exceeds binary32 range".to_string(),
            ));
        }
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC equipment client".to_string()))?;
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid("Missing QC equipment client".to_string()));
        }
        let word = self.field("max_health")?;
        self.machine_write(|machine| machine.entities_mut().set_slot_float(slot as u32, word, value as f32))?;
        Ok(())
    }

    /// Drop one client's objectives (`dropClientObjectives`).
    pub fn drop_client_objectives(&self, actor: &ActorId) -> Result<(), QuakeCSourceError> {
        let slot = self.source_slot(actor);
        if !self.is_active_client(actor) || slot.is_none() {
            return Err(QuakeCSourceError::Invalid("Missing QC equipment client".to_string()));
        }
        let client = self
            .shared
            .borrow()
            .weapon_stage
            .and_then(|stage| stage.stage.client.clone());
        let Some(client) = client else {
            return Err(QuakeCSourceError::Invalid(
                "QC artifact has no qualified objective contract".to_string(),
            ));
        };
        if let QcWeaponObjectives::Call(call) = &client.objectives {
            self.invoke_client_stage(call, actor)?;
        }
        Ok(())
    }

    /// Selected spawn point for one client (`clientSpawnPoint`).
    pub fn client_spawn_point(&self, actor: &ActorId) -> Result<QuakeCSpawnPoint, QuakeCSourceError> {
        if self.source_slot(actor).is_none() {
            return Err(QuakeCSourceError::Invalid("Missing QC equipment client".to_string()));
        }
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid("Missing QC equipment client".to_string()));
        }
        let client = self
            .shared
            .borrow()
            .weapon_stage
            .and_then(|stage| stage.stage.client.clone());
        let Some(client) = client else {
            return Err(QuakeCSourceError::Invalid(
                "QC artifact has no qualified spawn selection".to_string(),
            ));
        };
        let reference = self.invoke_client_stage(&client.select_spawn, actor)?;
        let spawn_slot = self.machine_read(|machine| machine.entities().slot(reference))? as usize;
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let spawn = actors.borrow().at_source(&provider, spawn_slot);
        let world = self.world_actor()?;
        let live = spawn
            .as_ref()
            .is_some_and(|spawn| actors.borrow().is_live(spawn.id()) && spawn.id() != world.id());
        if !live {
            return Err(QuakeCSourceError::Invalid(
                "Original QC source selected no spawn point".to_string(),
            ));
        }
        let origin_word = self.field("origin")?;
        let angles_word = self.field("angles")?;
        let (origin, angles) = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let origin = machine.entities().slot_vector(slot, origin_word)?;
            let angles = machine.entities().slot_vector(slot, angles_word)?;
            Ok::<_, GuestError>((origin, angles))
        })?;
        Ok(QuakeCSpawnPoint { origin, angles })
    }

    /// Whether one pickup defers selection to the source (`pickupSelectionDeferred`).
    pub fn pickup_selection_deferred(&self, offer: &OriginalPickupOffer) -> Result<bool, QuakeCSourceError> {
        self.shared
            .borrow()
            .pickups
            .selection_deferred(offer)
            .map_err(QuakeCSourceError::from)
    }

    /// Selected supply for one pickup (`pickupSupply`).
    pub fn pickup_supply(&self, offer: &OriginalPickupOffer) -> Result<QcPickupSupplyOffer, QuakeCSourceError> {
        self.shared
            .borrow()
            .pickups
            .supply(offer)
            .map_err(QuakeCSourceError::from)
    }

    /// Run mixed-client pre-think for one NetQuake command (`mixedClientPreThink`).
    pub fn mixed_client_pre_think(
        &self,
        actor: &OwnedActor,
        command: &UserCommand,
        frame: &FrameContext,
    ) -> Result<QuakeCClientMovement, QuakeCSourceError> {
        let input = quake_c_client_command(command);
        let converted = match &input {
            UserCommand::Q1Netquake {
                view_angles,
                buttons,
                impulse,
                forward_move,
                side_move,
                up_move,
                acknowledged_server_time_seconds,
            } => Q1UserCommand {
                acknowledged_server_time_seconds: *acknowledged_server_time_seconds,
                view_angles: Vec3 {
                    x: view_angles[0] as f32,
                    y: view_angles[1] as f32,
                    z: view_angles[2] as f32,
                },
                forward_move: *forward_move,
                side_move: *side_move,
                up_move: *up_move,
                buttons: *buttons as i32,
                impulse: *impulse as i32,
            },
            _ => {
                return Err(QuakeCSourceError::Invalid(
                    "Mixed QC pre-think requires a NetQuake command".to_string(),
                ));
            }
        };
        self.client_input(actor.id(), &converted)?;
        let input_buttons = converted.buttons;
        let reference = self.reference(actor.id())?;
        let flags_word = self.field("flags")?;
        let velocity_word = self.field("velocity")?;
        let before = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let flags = machine.entities().slot_float(slot, flags_word)? as i32;
            let velocity = machine.entities().slot_vector(slot, velocity_word)?;
            Ok::<_, GuestError>(QuakeCTransitionState {
                flags,
                velocity: [f64::from(velocity.x), f64::from(velocity.y), f64::from(velocity.z)],
            })
        })?;
        self.machine_write(|machine| {
            let offset = machine.global_offset("frametime")?;
            machine
                .globals_mut()
                .set_float(offset, frame.elapsed.as_seconds_f64() as f32)
        })?;
        self.client_pre_think(actor)?;
        let after = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let flags = machine.entities().slot_float(slot, flags_word)? as i32;
            let velocity = machine.entities().slot_vector(slot, velocity_word)?;
            Ok::<_, GuestError>(QuakeCTransitionState {
                flags,
                velocity: [f64::from(velocity.x), f64::from(velocity.y), f64::from(velocity.z)],
            })
        })?;
        let accepted_jump = quake_c_source_jump(command, &before, &after);
        let source_jump = accepted_jump || input_buttons & 2 != 0 && before.flags & 4096 == 0;
        if accepted_jump {
            let physics = self.shared.borrow().options.physics.clone();
            let current = physics.borrow().read_body(actor.id());
            if let Some(current) = current {
                physics.borrow_mut().write_body(
                    actor,
                    BodyState {
                        ground: None,
                        ..current
                    },
                );
            }
        }
        let mut think_frame = *frame;
        think_frame.time = qa_core::time::SourceTime::Seconds(self.current_time() as f32);
        think_frame.elapsed = qa_core::time::SourceTime::Seconds(frame.elapsed.as_seconds_f64() as f32);
        self.run_think(actor, &think_frame)?;
        Ok(QuakeCClientMovement {
            command: if source_jump {
                consume_quake_c_jump(command)
            } else {
                command.clone()
            },
            source_jump,
        })
    }

    /// Run mixed-client pre-think for one QuakeWorld command (`mixedQuakeWorldPreThink`).
    pub fn mixed_quake_world_pre_think(
        &self,
        actor: &OwnedActor,
        source_command: &QwUserCommand,
        command: &UserCommand,
        frame: &FrameContext,
    ) -> Result<QuakeCClientMovement, QuakeCSourceError> {
        let reference = self.reference(actor.id())?;
        let flags_word = self.field("flags")?;
        let velocity_word = self.field("velocity")?;
        let (before_flags, before_velocity) = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let flags = machine.entities().slot_float(slot, flags_word)? as i32;
            let velocity = machine.entities().slot_vector(slot, velocity_word)?;
            Ok::<_, GuestError>((flags, velocity))
        })?;
        self.quake_world_pre_think(actor, source_command, frame)?;
        let (after_flags, after_velocity) = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let flags = machine.entities().slot_float(slot, flags_word)? as i32;
            let velocity = machine.entities().slot_vector(slot, velocity_word)?;
            Ok::<_, GuestError>((flags, velocity))
        })?;
        let accepted =
            source_command.buttons & 2 != 0 && before_flags & (512 | 4096) == (512 | 4096) && after_flags & 4096 == 0;
        let impulse = accepted && after_velocity.z > before_velocity.z;
        if impulse {
            let physics = self.shared.borrow().options.physics.clone();
            let current = physics.borrow().read_body(actor.id());
            if let Some(current) = current {
                physics.borrow_mut().write_body(
                    actor,
                    BodyState {
                        ground: None,
                        ..current
                    },
                );
            }
        }
        Ok(QuakeCClientMovement {
            command: if impulse {
                consume_quake_c_jump(command)
            } else {
                command.clone()
            },
            source_jump: accepted,
        })
    }

    /// Run mixed-client post-think after movement (`mixedClientPostThink`).
    pub fn mixed_client_post_think(
        &self,
        actor: &OwnedActor,
        ground: &TraceHit,
        water_level: i32,
        water_type: i32,
        movement: QuakeCMovementKind,
        post_think: QuakeCPostThink,
    ) -> Result<(), QuakeCSourceError> {
        let reference = self.reference(actor.id())?;
        let flags_word = self.field("flags")?;
        let ground_word = self.field("groundentity")?;
        let level_word = self.field("waterlevel")?;
        let type_word = self.field("watertype")?;
        let ground_reference = match ground {
            TraceHit::Actor { actor } => self.reference(actor)?,
            _ => 0,
        };
        let grounded = !matches!(ground, TraceHit::None);
        let source_water = if water_level == 0 {
            -1
        } else if movement == QuakeCMovementKind::Q1Netquake || movement == QuakeCMovementKind::Q1Quakeworld {
            water_type
        } else if water_type & 16 != 0 {
            -4
        } else if water_type & 8 != 0 {
            -5
        } else {
            -3
        };
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            let flags = machine.entities().slot_float(slot, flags_word)? as i32;
            machine.entities_mut().set_slot_float(
                slot,
                flags_word,
                ((flags & !512) | if grounded { 512 } else { 0 }) as f32,
            )?;
            machine
                .entities_mut()
                .set_slot_int(slot, ground_word, ground_reference)?;
            machine
                .entities_mut()
                .set_slot_float(slot, level_word, water_level as f32)?;
            machine
                .entities_mut()
                .set_slot_float(slot, type_word, source_water as f32)
        })?;
        if post_think == QuakeCPostThink::Immediate {
            self.client_post_think(actor)?;
        }
        Ok(())
    }

    /// Feed one NetQuake command into the source ABI (`clientInput`).
    pub fn client_input(&self, actor: &ActorId, command: &Q1UserCommand) -> Result<(), QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("QC input requires an admitted client".to_string()))?;
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid(
                "QC input requires an admitted client".to_string(),
            ));
        }
        let view_word = self.field("v_angle")?;
        let button0 = self.field("button0")?;
        let button2 = self.field("button2")?;
        let impulse_word = self.field("impulse")?;
        self.machine_write(|machine| {
            machine
                .entities_mut()
                .set_slot_vector(slot as u32, view_word, command.view_angles)?;
            machine
                .entities_mut()
                .set_slot_float(slot as u32, button0, (command.buttons & 1) as f32)?;
            machine
                .entities_mut()
                .set_slot_float(slot as u32, button2, ((command.buttons >> 1) & 1) as f32)
        })?;
        if command.impulse != 0 {
            let router = self.shared.borrow().router.clone();
            if router
                .borrow_mut()
                .local_messages
                .answer_prompt(actor, command.impulse as u8)?
            {
                let content = self.prepared.execution.owner.content.clone();
                let events = self.shared.borrow().options.events.clone();
                events.borrow_mut().emit_local(
                    &content,
                    QuakeCLocalSinkEvent::Prompt(Q1CompositionEvent::ClearPrompt { actor: actor.clone() }),
                    Some(actor),
                );
            }
            self.shared.borrow().pending_weapons.borrow_mut().remove(actor);
            self.machine_write(|machine| {
                machine
                    .entities_mut()
                    .set_slot_float(slot as u32, impulse_word, command.impulse as f32)
            })?;
        }
        Ok(())
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Read one client's QuakeWorld movement state (`readQuakeWorldState`).
    pub fn read_quake_world_state(
        &self,
        actor: &ActorId,
        state: &QwMovementState,
    ) -> Result<QwMovementState, QuakeCSourceError> {
        let reference = self.reference(actor)?;
        let origin_word = self.field("origin")?;
        let mins_word = self.field("mins")?;
        let velocity_word = self.field("velocity")?;
        let angle_word = self.field("v_angle")?;
        let teleport_word = self.field("teleport_time")?;
        let health_word = self.field("health")?;
        let (origin, mins, velocity, angles, teleport, health) = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let origin = machine.entities().slot_vector(slot, origin_word)?;
            let mins = machine.entities().slot_vector(slot, mins_word)?;
            let velocity = machine.entities().slot_vector(slot, velocity_word)?;
            let angles = machine.entities().slot_vector(slot, angle_word)?;
            let teleport = machine.entities().slot_float(slot, teleport_word)?;
            let health = machine.entities().slot_float(slot, health_word)?;
            Ok::<_, GuestError>((origin, mins, velocity, angles, teleport, health))
        })?;
        Ok(QwMovementState {
            origin: Vec3 {
                x: origin.x + mins.x + 16.0,
                y: origin.y + mins.y + 16.0,
                z: origin.z + mins.z + 24.0,
            },
            velocity,
            angles,
            water_jump_time_seconds: f64::from(teleport),
            dead: health <= 0.0,
            spectator: if self.is_spectator_client(actor) { 1 } else { 0 },
            ..state.clone()
        })
    }

    /// Write one client's QuakeWorld movement state (`writeQuakeWorldState`).
    pub fn write_quake_world_state(&self, actor: &ActorId, state: &QwMovementState) -> Result<(), QuakeCSourceError> {
        let reference = self.reference(actor)?;
        let origin_word = self.field("origin")?;
        let mins_word = self.field("mins")?;
        let velocity_word = self.field("velocity")?;
        let angle_word = self.field("v_angle")?;
        let teleport_word = self.field("teleport_time")?;
        let flags_word = self.field("flags")?;
        let ground_word = self.field("groundentity")?;
        let ground_reference = match &state.ground {
            TraceHit::Actor { actor } => self.reference(actor)?,
            _ => 0,
        };
        let grounded = !matches!(state.ground, TraceHit::None);
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            let mins = machine.entities().slot_vector(slot, mins_word)?;
            machine.entities_mut().set_slot_vector(
                slot,
                origin_word,
                Vec3 {
                    x: state.origin.x - mins.x - 16.0,
                    y: state.origin.y - mins.y - 16.0,
                    z: state.origin.z - mins.z - 24.0,
                },
            )?;
            machine
                .entities_mut()
                .set_slot_vector(slot, velocity_word, state.velocity)?;
            machine.entities_mut().set_slot_vector(slot, angle_word, state.angles)?;
            machine
                .entities_mut()
                .set_slot_float(slot, teleport_word, state.water_jump_time_seconds as f32)?;
            let flags = machine.entities().slot_float(slot, flags_word)? as i32;
            machine.entities_mut().set_slot_float(
                slot,
                flags_word,
                ((flags & !512) | if grounded { 512 } else { 0 }) as f32,
            )?;
            if grounded {
                machine
                    .entities_mut()
                    .set_slot_int(slot, ground_word, ground_reference)?;
            }
            Ok::<_, GuestError>(())
        })?;
        Ok(())
    }

    /// Write one client's QuakeWorld water state (`quakeWorldWater`).
    pub fn quake_world_water(&self, actor: &ActorId, level: i32, type_: i32) -> Result<(), QuakeCSourceError> {
        let reference = self.reference(actor)?;
        let level_word = self.field("waterlevel")?;
        let type_word = self.field("watertype")?;
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities_mut().set_slot_float(slot, level_word, level as f32)?;
            machine.entities_mut().set_slot_float(slot, type_word, type_ as f32)
        })?;
        Ok(())
    }

    /// Resolve one client's QuakeWorld movement profile (`quakeWorldProfile`).
    pub fn quake_world_profile(
        &self,
        actor: &ActorId,
        profile: &QwMovementProfile,
    ) -> Result<QwMovementProfile, QuakeCSourceError> {
        let reference = self.reference(actor)?;
        let cvars = self.shared.borrow().cvars.clone();
        let cvars = cvars.borrow();
        let mut resolved = profile.clone();
        resolved.parameters = Q1MovementParameters {
            gravity: f64::from(cvars.variable_value("sv_gravity")),
            stop_speed: f64::from(cvars.variable_value("sv_stopspeed")),
            max_speed: resolved.parameters.max_speed,
            spectator_max_speed: f64::from(cvars.variable_value("sv_spectatormaxspeed")),
            accelerate: f64::from(cvars.variable_value("sv_accelerate")),
            air_accelerate: f64::from(cvars.variable_value("sv_airaccelerate")),
            water_accelerate: f64::from(cvars.variable_value("sv_wateraccelerate")),
            friction: f64::from(cvars.variable_value("sv_friction")),
            water_friction: f64::from(cvars.variable_value("sv_waterfriction")),
            entity_gravity: resolved.parameters.entity_gravity,
        };
        let maxspeed = self
            .prepared
            .program
            .field_named("maxspeed")
            .map(|definition| definition.offset);
        let gravity = self
            .prepared
            .program
            .field_named("gravity")
            .map(|definition| definition.offset);
        let fallback_max = f64::from(cvars.variable_value("sv_maxspeed"));
        drop(cvars);
        let (max_speed, entity_gravity) = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            let max = match maxspeed {
                Some(word) => f64::from(machine.entities().slot_float(slot, word)?),
                None => fallback_max,
            };
            let gravity = match gravity {
                Some(word) => f64::from(machine.entities().slot_float(slot, word)?),
                None => 1.0,
            };
            Ok::<_, GuestError>((max, gravity))
        })?;
        resolved.parameters.max_speed = max_speed;
        resolved.parameters.entity_gravity = entity_gravity;
        Ok(resolved)
    }

    /// Run QuakeWorld pre-think for one client (`quakeWorldPreThink`).
    pub fn quake_world_pre_think(
        &self,
        actor: &OwnedActor,
        command: &QwUserCommand,
        frame: &FrameContext,
    ) -> Result<(), QuakeCSourceError> {
        if self.kind() != QuakeCSourceKind::Quakeworld || !self.is_active_client(actor.id()) {
            return Err(QuakeCSourceError::Invalid(
                "QW movement requires a begun native client".to_string(),
            ));
        }
        let reference = self.reference(actor.id())?;
        let fixangle = self.field("fixangle")?;
        let v_angle = self.field("v_angle")?;
        let button0 = self.field("button0")?;
        let button2 = self.field("button2")?;
        let impulse = self.field("impulse")?;
        let health = self.field("health")?;
        let angles = self.field("angles")?;
        let velocity = self.field("velocity")?;
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            if machine.entities().slot_float(slot, fixangle)? == 0.0 {
                machine.entities_mut().set_slot_vector(slot, v_angle, command.angles)?;
            }
            machine
                .entities_mut()
                .set_slot_float(slot, button0, (command.buttons & 1) as f32)?;
            machine
                .entities_mut()
                .set_slot_float(slot, button2, ((command.buttons >> 1) & 1) as f32)?;
            if command.impulse != 0 {
                machine
                    .entities_mut()
                    .set_slot_float(slot, impulse, command.impulse as f32)?;
            }
            if machine.entities().slot_float(slot, health)? > 0.0 {
                let mut posed = machine.entities().slot_vector(slot, angles)?;
                if machine.entities().slot_float(slot, fixangle)? == 0.0 {
                    posed.x = -command.angles.x / 3.0;
                    posed.y = command.angles.y;
                }
                let mut right = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
                let moved = machine.entities().slot_vector(slot, velocity)?;
                donor_angle_vectors(posed, None, Some(&mut right), None);
                let side = moved.x * right.x + moved.y * right.y + moved.z * right.z;
                posed.z = (if side.abs() < 200.0 {
                    side.abs() * 2.0 / 200.0
                } else {
                    2.0
                }) * (if side < 0.0 { -1.0 } else { 1.0 })
                    * 4.0;
                machine.entities_mut().set_slot_vector(slot, angles, posed)?;
            }
            Ok::<_, GuestError>(())
        })?;
        if self.is_spectator_client(actor.id()) {
            return Ok(());
        }
        self.machine_write(|machine| {
            let offset = machine.global_offset("frametime")?;
            machine
                .globals_mut()
                .set_float(offset, command.milliseconds as f32 * 0.001)
        })?;
        self.client_pre_think(actor)?;
        let mut think_frame = *frame;
        think_frame.time = qa_core::time::SourceTime::Seconds(self.current_time() as f32);
        think_frame.elapsed = qa_core::time::SourceTime::Seconds(command.milliseconds as f32 * 0.001);
        self.run_think(actor, &think_frame)
    }

    /// Take the pending QuakeWorld missile, if any (`takeNewMissile`).
    pub fn take_new_missile(&self) -> Result<Option<OwnedActor>, QuakeCSourceError> {
        if self.kind() != QuakeCSourceKind::Quakeworld {
            return Ok(None);
        }
        let offset = self.machine_read(|machine| machine.global_offset("newmis"))?;
        let reference = self.machine_read(|machine| machine.globals().int(offset))?;
        if reference == 0 {
            return Ok(None);
        }
        self.machine_write(|machine| machine.globals_mut().set_int(offset, 0))?;
        let slot = self.machine_read(|machine| machine.entities().slot(reference))? as usize;
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, slot);
        Ok(actor)
    }

    /// Read one client's NetQuake movement state (`readClientState`).
    pub fn read_client_state(
        &self,
        actor: &ActorId,
        state: &Q1MovementState,
    ) -> Result<Q1MovementState, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("QC player has no source slot".to_string()))?;
        let words = [
            "origin",
            "velocity",
            "angles",
            "oldorigin",
            "avelocity",
            "v_angle",
            "punchangle",
            "movetype",
            "flags",
            "waterlevel",
            "watertype",
            "teleport_time",
            "movedir",
            "idealpitch",
            "fixangle",
            "health",
        ]
        .into_iter()
        .map(|name| self.field(name))
        .collect::<Result<Vec<_>, _>>()?;
        let values = self.machine_read(|machine| {
            let entities = machine.entities();
            Ok::<_, GuestError>((
                entities.slot_vector(slot as u32, words[0])?,
                entities.slot_vector(slot as u32, words[1])?,
                entities.slot_vector(slot as u32, words[2])?,
                entities.slot_vector(slot as u32, words[3])?,
                entities.slot_vector(slot as u32, words[4])?,
                entities.slot_vector(slot as u32, words[5])?,
                entities.slot_vector(slot as u32, words[6])?,
                entities.slot_float(slot as u32, words[7])?,
                entities.slot_float(slot as u32, words[8])?,
                entities.slot_float(slot as u32, words[9])?,
                entities.slot_float(slot as u32, words[10])?,
                entities.slot_float(slot as u32, words[11])?,
                entities.slot_vector(slot as u32, words[12])?,
                entities.slot_float(slot as u32, words[13])?,
                entities.slot_float(slot as u32, words[14])?,
                entities.slot_float(slot as u32, words[15])?,
            ))
        })?;
        let (
            origin,
            velocity,
            angles,
            old_origin,
            angular_velocity,
            view_angles,
            punch_angles,
            move_type,
            flags,
            water_level,
            water_type,
            teleport,
            movedir,
            ideal_pitch,
            fixangle,
            health,
        ) = values;
        Ok(Q1MovementState {
            origin,
            velocity,
            angles,
            old_origin,
            angular_velocity,
            view_angles: if fixangle != 0.0 { angles } else { view_angles },
            punch_angles,
            move_type: move_type as i32,
            flags: flags as i32,
            water_level: water_level as i32,
            water_type: water_type as i32,
            teleport_time_seconds: f64::from(teleport),
            water_jump_direction: movedir,
            ideal_pitch: f64::from(ideal_pitch),
            fix_angle: fixangle != 0.0,
            health: f64::from(health),
            ..state.clone()
        })
    }

    /// Read one client's NetQuake punch vector (`clientPunchAngles`).
    pub fn client_punch_angles(&self, actor: &ActorId) -> Result<Vec3, QuakeCSourceError> {
        if self.kind() != QuakeCSourceKind::Netquake {
            return Err(QuakeCSourceError::Invalid(
                "QuakeWorld does not own a NetQuake punch vector".to_string(),
            ));
        }
        let reference = self.reference(actor)?;
        let word = self.field("punchangle")?;
        self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities().slot_vector(slot, word)
        })
        .map_err(QuakeCSourceError::from)
    }

    /// Write one client's view roll (`setClientViewRoll`).
    pub fn set_client_view_roll(&self, actor: &ActorId, roll: f64) -> Result<(), QuakeCSourceError> {
        if !self.is_active_client(actor) {
            return Err(QuakeCSourceError::Invalid(
                "QC source view requires an admitted client".to_string(),
            ));
        }
        let reference = self.reference(actor)?;
        let word = self.field("v_angle")?;
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            let mut angles = machine.entities().slot_vector(slot, word)?;
            angles.z = roll as f32;
            machine.entities_mut().set_slot_vector(slot, word, angles)
        })?;
        Ok(())
    }

    /// Whether one client's punch vector advances (`clientPunchAdvances`).
    pub fn client_punch_advances(&self, actor: &ActorId) -> Result<bool, QuakeCSourceError> {
        let reference = self.reference(actor)?;
        let word = self.field("movetype")?;
        let move_type = self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities().slot_float(slot, word)
        })?;
        Ok(move_type != 0.0)
    }

    /// Live collision record for an actor (`collision`, donor `quakec-source.ts` 1443).
    pub fn collision(&self, actor: &OwnedActor) -> Result<Option<SharedSolid>, QuakeCSourceError> {
        let shared = self.shared.borrow();
        let fields = shared.fields.clone();
        let borrowed = fields.borrow();
        let solid = shared.actor_state.collision(&borrowed, actor.id())?;
        Ok(solid)
    }

    /// Live motion record for an actor (`motion`, donor `quakec-source.ts` 1444).
    pub fn motion(&self, actor: &OwnedActor, body: &WorldBodyState) -> Result<Option<GuestMotion>, QuakeCSourceError> {
        let guest_body = BodyState {
            origin: body.origin,
            angles: body.angles,
            velocity: body.velocity,
            bounds: body.bounds,
            ground: body.ground.clone(),
        };
        let shared = self.shared.borrow();
        let fields = shared.fields.clone();
        let borrowed = fields.borrow();
        Ok(shared.actor_state.motion(&borrowed, actor.id(), &guest_body)?)
    }

    /// Live physics flags for an actor (`flags`, donor `quakec-source.ts` 1445).
    pub fn execution_flags(&self, actor: &OwnedActor) -> Result<GuestPhysicsFlags, QuakeCSourceError> {
        let shared = self.shared.borrow();
        let fields = shared.fields.clone();
        let borrowed = fields.borrow();
        Ok(shared.actor_state.flags(&borrowed, actor.id())?)
    }

    /// Store partial physics-flag changes (`writeFlags`, donor `quakec-source.ts` 1446).
    pub fn write_execution_flags(
        &self,
        actor: &OwnedActor,
        changes: &PhysicsFlagChanges,
    ) -> Result<(), QuakeCSourceError> {
        let shared = self.shared.borrow();
        let fields = shared.fields.clone();
        let mut borrowed = fields.borrow_mut();
        shared.actor_state.write_flags(&mut borrowed, actor.id(), changes)?;
        Ok(())
    }

    /// Claim one QuakeWorld execution of an actor for this frame (`runActorOnce`,
    /// donor `quakec-source.ts` 1107-1114).
    pub fn run_actor_once(&self, actor: &ActorId, frame: &FrameContext) -> Result<bool, QuakeCSourceError> {
        if self.kind() != QuakeCSourceKind::Quakeworld {
            return Ok(true);
        }
        let time = match frame.time {
            SourceTime::Seconds(value) => value,
            SourceTime::Milliseconds(value) => value as f32 / 1000.0,
        };
        let reference = self.reference(actor)?;
        let word = self.field("lastruntime")?;
        let seen = self.machine_read(|machine| {
            machine
                .entities()
                .slot(reference)
                .and_then(|slot| machine.entities().slot_float(slot, word))
        })?;
        if seen == time {
            return Ok(false);
        }
        self.machine_write(|machine| {
            machine
                .entities()
                .slot(reference)
                .and_then(|slot| machine.entities_mut().set_slot_float(slot, word, time))
        })?;
        Ok(true)
    }

    /// Write one client's NetQuake punch vector (`setClientPunchAngles`).
    pub fn set_client_punch_angles(&self, actor: &ActorId, angles: Vec3) -> Result<(), QuakeCSourceError> {
        if self.kind() != QuakeCSourceKind::Netquake {
            return Err(QuakeCSourceError::Invalid(
                "QuakeWorld does not own a NetQuake punch vector".to_string(),
            ));
        }
        let reference = self.reference(actor)?;
        let word = self.field("punchangle")?;
        self.machine_write(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities_mut().set_slot_vector(slot, word, angles)
        })?;
        Ok(())
    }

    /// Write one client's NetQuake movement state (`writeClientState`).
    pub fn write_client_state(&self, actor: &ActorId, state: &Q1MovementState) -> Result<(), QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("QC player has no source slot".to_string()))?;
        let vectors = [
            ("oldorigin", state.old_origin),
            ("avelocity", state.angular_velocity),
            ("v_angle", state.view_angles),
            ("punchangle", state.punch_angles),
            ("movedir", state.water_jump_direction),
        ];
        let scalars = [
            ("movetype", state.move_type as f64),
            ("flags", state.flags as f64),
            ("waterlevel", state.water_level as f64),
            ("watertype", state.water_type as f64),
            ("teleport_time", state.teleport_time_seconds),
            ("idealpitch", state.ideal_pitch),
            ("fixangle", f64::from(u8::from(state.fix_angle))),
        ];
        let vector_words = vectors
            .iter()
            .map(|(name, _)| self.field(name))
            .collect::<Result<Vec<_>, _>>()?;
        let scalar_words = scalars
            .iter()
            .map(|(name, _)| self.field(name))
            .collect::<Result<Vec<_>, _>>()?;
        self.machine_write(|machine| {
            for (word, (_, value)) in vector_words.iter().zip(vectors.iter()) {
                machine.entities_mut().set_slot_vector(slot as u32, *word, *value)?;
            }
            for (word, (_, value)) in scalar_words.iter().zip(scalars.iter()) {
                machine
                    .entities_mut()
                    .set_slot_float(slot as u32, *word, *value as f32)?;
            }
            Ok::<_, GuestError>(())
        })?;
        Ok(())
    }

    /// Consume one client's pending view reset (`consumeClientViewReset`).
    pub fn consume_client_view_reset(&self, actor: &ActorId) -> Result<Option<Vec3>, QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor) else {
            return Ok(None);
        };
        let fixangle = self.field("fixangle")?;
        let angles = self.field("angles")?;
        let fixed = self.machine_read(|machine| machine.entities().slot_float(slot as u32, fixangle))?;
        if fixed == 0.0 {
            return Ok(None);
        }
        self.machine_write(|machine| machine.entities_mut().set_slot_float(slot as u32, fixangle, 0.0))?;
        let reset = self.machine_read(|machine| machine.entities().slot_vector(slot as u32, angles))?;
        Ok(Some(reset))
    }

    /// Request one client's weapon (`requestClientWeapon`).
    pub fn request_client_weapon(&self, actor: &ActorId, item: &ItemId) -> Result<bool, QuakeCSourceError> {
        let slot = self.source_slot(actor);
        let weapon = self
            .shared
            .borrow()
            .weapons
            .iter()
            .find(|weapon| &weapon.item == item)
            .cloned();
        let (Some(slot), Some(weapon)) = (slot, weapon) else {
            return Ok(false);
        };
        let inventory = self.shared.borrow().options.inventory.clone();
        if inventory.borrow().count(actor, item) <= 0.0 {
            return Ok(false);
        }
        let weapon_word = self.field("weapon")?;
        let items_word = self.field("items")?;
        let impulse_word = self.field("impulse")?;
        self.shared.borrow().pending_weapons.borrow_mut().remove(actor);
        let current = self.machine_read(|machine| machine.entities().slot_float(slot as u32, weapon_word))?;
        if current as i32 == weapon.bit {
            self.machine_write(|machine| machine.entities_mut().set_slot_float(slot as u32, impulse_word, 0.0))?;
            return Ok(true);
        }
        if let Some(via) = weapon.via {
            let held = self.machine_read(|machine| machine.entities().slot_float(slot as u32, items_word))? as i32;
            if current as i32 != via && held & via != 0 {
                self.shared.borrow().pending_weapons.borrow_mut().insert(
                    actor.clone(),
                    PendingWeapon {
                        weapon: weapon.clone(),
                        following: false,
                    },
                );
            }
        }
        self.machine_write(|machine| {
            machine
                .entities_mut()
                .set_slot_float(slot as u32, impulse_word, weapon.impulse as f32)
        })?;
        Ok(true)
    }

    /// Run `PlayerPreThink` for one client (`clientPreThink`).
    pub fn client_pre_think(&self, actor: &OwnedActor) -> Result<(), QuakeCSourceError> {
        if self.is_spectator_client(actor.id()) {
            return Ok(());
        }
        let slot = self
            .source_slot(actor.id())
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC client".to_string()))?;
        let index = self.prepared.program.function_named("PlayerPreThink")?.index;
        let time = self.current_time();
        self.invoke(index as i32, slot, 0, time)
    }

    /// Run `PlayerPostThink` for one client (`clientPostThink`).
    pub fn client_post_think(&self, actor: &OwnedActor) -> Result<(), QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor.id()) else {
            return Ok(());
        };
        if self.is_spectator_client(actor.id()) {
            return self.spectator_callback("SpectatorThink", slot);
        }
        let impulse_word = self.field("impulse")?;
        let health_word = self.field("health")?;
        let weapon_word = self.field("weapon")?;
        let pending = self.shared.borrow().pending_weapons.borrow().get(actor.id()).cloned();
        let impulse = self.machine_read(|machine| machine.entities().slot_float(slot as u32, impulse_word))?;
        if let Some(pending) = &pending {
            let inventory = self.shared.borrow().options.inventory.clone();
            let held = inventory.borrow().count(actor.id(), &pending.weapon.item);
            let health = self.machine_read(|machine| machine.entities().slot_float(slot as u32, health_word))?;
            if held <= 0.0 || health <= 0.0 {
                self.shared.borrow().pending_weapons.borrow_mut().remove(actor.id());
                if impulse as i32 == pending.weapon.impulse {
                    self.machine_write(|machine| {
                        machine.entities_mut().set_slot_float(slot as u32, impulse_word, 0.0)
                    })?;
                }
            } else if impulse != 0.0 && impulse as i32 != pending.weapon.impulse {
                self.shared.borrow().pending_weapons.borrow_mut().remove(actor.id());
            }
        }
        let index = self.prepared.program.function_named("PlayerPostThink")?.index;
        let time = self.current_time();
        self.invoke(index as i32, slot, 0, time)?;
        let selection = self.shared.borrow().pending_weapons.borrow().get(actor.id()).cloned();
        if let Some(selection) = selection {
            let impulse = self.machine_read(|machine| machine.entities().slot_float(slot as u32, impulse_word))?;
            if impulse == 0.0 {
                self.shared.borrow().pending_weapons.borrow_mut().remove(actor.id());
                let current = self.machine_read(|machine| machine.entities().slot_float(slot as u32, weapon_word))?;
                let inventory = self.shared.borrow().options.inventory.clone();
                if !selection.following
                    && selection.weapon.via.is_some_and(|via| current as i32 == via)
                    && inventory.borrow().count(actor.id(), &selection.weapon.item) > 0.0
                {
                    self.shared.borrow().pending_weapons.borrow_mut().insert(
                        actor.id().clone(),
                        PendingWeapon {
                            weapon: selection.weapon.clone(),
                            following: true,
                        },
                    );
                    self.machine_write(|machine| {
                        machine.entities_mut().set_slot_float(
                            slot as u32,
                            impulse_word,
                            selection.weapon.impulse as f32,
                        )
                    })?;
                }
            }
        }
        Ok(())
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// HUD snapshot for one client (`clientUi`).
    pub fn client_ui(&self, actor: &ActorId) -> Result<QuakeCClientUi, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC UI actor".to_string()))?;
        let items_word = self.field("items")?;
        let weapon_word = self.field("weapon")?;
        let current_word = self.field("currentammo")?;
        let (items, weapon, current) = self.machine_read(|machine| {
            let items = machine.entities().slot_float(slot as u32, items_word)? as i32;
            let weapon = machine.entities().slot_float(slot as u32, weapon_word)? as i32;
            let current = machine.entities().slot_float(slot as u32, current_word)?;
            Ok::<_, GuestError>((items, weapon, f64::from(current)))
        })?;
        let timers = [
            (Q1Powerup::Quad, "super_damage_finished"),
            (Q1Powerup::Invulnerability, "invincible_finished"),
            (Q1Powerup::Invisibility, "invisible_finished"),
            (Q1Powerup::Suit, "radsuit_finished"),
        ];
        let mut powerups = std::collections::HashMap::new();
        for (kind, field) in timers {
            if let Some(definition) = self.prepared.program.field_named(field) {
                if definition.value_type == qa_guest::qc::program::QcValueType::Float {
                    let expires =
                        self.machine_read(|machine| machine.entities().slot_float(slot as u32, definition.offset))?;
                    powerups.insert(kind, f64::from(expires));
                }
            }
        }
        let stat = self.shared.borrow().router.borrow().local_messages.stat(actor, 3);
        let bindings: Vec<QuakeCWeaponUiBinding> = self
            .shared
            .borrow()
            .weapons
            .iter()
            .map(|weapon| QuakeCWeaponUiBinding {
                item: weapon.item.clone(),
                label: weapon.label.clone(),
                bit: weapon.bit,
                impulse: weapon.impulse,
            })
            .collect();
        let weapon_ui = quake_c_weapon_ui(items, weapon, stat.map_or(current, f64::from), &bindings);
        let timers = q1_powerup_timers(&powerups, self.current_time());
        Ok(QuakeCClientUi {
            weapon: weapon_ui,
            powerups: timers,
        })
    }

    /// Arsenal snapshot for one client (`clientArsenal`).
    pub fn client_arsenal(&self, actor: &ActorId) -> Result<ArsenalState, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC arsenal actor".to_string()))?;
        let weapon_word = self.field("weapon")?;
        let frame_word = self.field("weaponframe")?;
        let attack_word = self.field("attack_finished")?;
        let (value, frame, attack) = self.machine_read(|machine| {
            let value = machine.entities().slot_float(slot as u32, weapon_word)?;
            let frame = machine.entities().slot_float(slot as u32, frame_word)? as i32;
            let attack = machine.entities().slot_float(slot as u32, attack_word)?;
            Ok::<_, GuestError>((value, frame, f64::from(attack)))
        })?;
        let weapons = self.shared.borrow().weapons.clone();
        let weapon = weapons.iter().find(|weapon| f64::from(weapon.bit) == f64::from(value));
        if weapon.is_none() && !(value == 0.0 && self.is_spectator_client(actor)) {
            return Err(QuakeCSourceError::Invalid(format!(
                "Unsupported actual QC weapon {value}"
            )));
        }
        let inventory = self.shared.borrow().options.inventory.clone();
        let ammo = inventory
            .borrow()
            .entries(actor)
            .into_iter()
            .map(|entry| MovementInventoryEntry {
                item: entry.item,
                count: entry.count,
            })
            .collect();
        Ok(ArsenalState {
            provider: self.prepared.execution.owner.provider.clone(),
            active_weapon: weapon.map(|weapon| weapon.item.clone()),
            state: WeaponState::Q1 {
                frame,
                attack_finished_seconds: attack,
                source_weapon: value as i32,
            },
            ammo,
        })
    }

    /// Animation snapshot for one client (`clientAnimation`).
    pub fn client_animation(&self, actor: &ActorId) -> Result<ActorAnimationState, QuakeCSourceError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| QuakeCSourceError::Invalid("Missing QC animation actor".to_string()))?;
        let frame_word = self.field("frame")?;
        let think_word = self.field("nextthink")?;
        let (frame, next) = self.machine_read(|machine| {
            let frame = machine.entities().slot_float(slot as u32, frame_word)? as i32;
            let next = machine.entities().slot_float(slot as u32, think_word)?;
            Ok::<_, GuestError>((frame, f64::from(next)))
        })?;
        let definition = self
            .shared
            .borrow()
            .options
            .recipe
            .character
            .definition
            .provider
            .clone();
        let (namespace, name) = definition.split_once(':').unwrap_or(("", definition.as_str()));
        Ok(ActorAnimationState {
            provider: ProviderId::new(namespace, name),
            state: AnimationState::Q1 {
                frame,
                next_frame_seconds: next,
            },
        })
    }

    /// Local intermission flag for one client (`localClientIntermission`).
    pub fn local_client_intermission(&self, actor: &ActorId) -> Option<bool> {
        let router = self.shared.borrow().router.clone();
        let state = router.borrow();
        if !state.local_messages.has_client(actor) {
            return None;
        }
        Some(state.local_messages.intermission(actor))
    }

    /// Local camera view for one client (`localClientView`).
    pub fn local_client_view(&self, actor: &ActorId) -> Option<PlayerView> {
        let router = self.shared.borrow().router.clone();
        let state = router.borrow();
        let source = LocalViewSource { source: self };
        quake_c_local_view(actor, &state.local_messages, &source)
    }

    /// Feed decoded NetQuake services into local presentation (`receiveLocalMessages`).
    fn receive_local_messages(
        &self,
        messages: &[NetQuakeMessage],
        destination: &QcMessageDestination,
        view_targets: &std::collections::HashMap<usize, Option<ActorId>>,
    ) -> Result<(), QuakeCSourceError> {
        if matches!(destination, QcMessageDestination::Multicast { .. }) {
            return Err(QuakeCSourceError::Invalid(
                "NetQuake has no multicast destination".to_string(),
            ));
        }
        let target = match destination {
            QcMessageDestination::Client { actor } => Some(actor.clone()),
            _ => None,
        };
        let router = self.shared.borrow().router.clone();
        let content = self.prepared.execution.owner.content.clone();
        for (index, message) in messages.iter().enumerate() {
            if matches!(message, NetQuakeMessage::SetView { .. }) && !view_targets.contains_key(&index) {
                return Err(QuakeCSourceError::Invalid(
                    "QC camera message has no captured source actor".to_string(),
                ));
            }
            router.borrow_mut().local_messages.receive(
                std::slice::from_ref(message),
                target.as_ref(),
                view_targets.get(&index).cloned(),
            )?;
            let state = router.borrow();
            let mut host = LocalMessageHost {
                source: self,
                target: target.clone(),
                content: content.clone(),
            };
            present_quake_c_local_message(message, target.as_ref(), &state.local_messages, &mut host)?;
        }
        Ok(())
    }

    /// Precached names by kind, in precache order (`precacheNames`).
    fn precache_names(&self, kind: PrecacheKind) -> Vec<String> {
        if kind == PrecacheKind::Model {
            let mut models: Vec<(String, i32)> = self
                .shared
                .borrow()
                .models
                .borrow()
                .iter()
                .map(|(name, model)| (name.clone(), model.index))
                .collect();
            models.sort_by_key(|(_, index)| *index);
            return models.into_iter().map(|(name, _)| name).collect();
        }
        let mut sounds: Vec<(String, i32)> = self
            .shared
            .borrow()
            .precached
            .iter()
            .filter(|(key, _)| key.starts_with("sound:"))
            .map(|(key, entry)| (key.clone(), entry.index))
            .collect();
        sounds.sort_by_key(|(_, index)| *index);
        sounds
            .into_iter()
            .map(|(key, _)| key["sound:".len()..].to_string())
            .collect()
    }

    /// Client view offset for one actor (`clientViewOffset`).
    pub fn client_view_offset(&self, actor: &ActorId) -> Result<Vec3, QuakeCSourceError> {
        let reference = self.reference(actor)?;
        if self.kind() == QuakeCSourceKind::Quakeworld {
            let mins_word = self.field("mins")?;
            let health_word = self.field("health")?;
            let (mins, health) = self.machine_read(|machine| {
                let slot = machine.entities().slot(reference)?;
                let mins = machine.entities().slot_vector(slot, mins_word)?;
                let health = machine.entities().slot_float(slot, health_word)?;
                Ok::<_, GuestError>((mins, health))
            })?;
            let height = if mins.z != -24.0 {
                8.0
            } else if health <= 0.0 {
                -16.0
            } else {
                22.0
            };
            return Ok(Vec3 {
                x: 0.0,
                y: 0.0,
                z: height,
            });
        }
        let word = self.field("view_ofs")?;
        self.machine_read(|machine| {
            let slot = machine.entities().slot(reference)?;
            machine.entities().slot_vector(slot, word)
        })
        .map_err(QuakeCSourceError::from)
    }
}

/// Actor-execution handle over a shared source (donor `QuakeCSource` arm of
/// `executeActor`, donor actor-execution.ts 119-123).
///
/// The session holds sources behind [`Rc`](std::rc::Rc) with interior
/// mutability; the wrapper keeps the execution method names off the shared
/// handle. Machine failures panic like the session's other execution
/// adapters: they signal a corrupt program, not a missable lookup (unknown
/// actors already read back absent through the `Option` returns).
pub struct QuakeCExecutionSource<P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: std::rc::Rc<QuakeCSource<P>>,
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCExecutionSource<P> {
    /// Borrow the execution surface of a shared source.
    pub fn new(source: std::rc::Rc<QuakeCSource<P>>) -> Self {
        Self { source }
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> ExecutionQuakeCSource for QuakeCExecutionSource<P> {
    fn is_reserved_client(&self, actor: &ActorId) -> bool {
        self.source.is_reserved_client(actor)
    }

    fn run_actor_once(&mut self, actor: &ActorId, frame: &FrameContext) -> bool {
        self.source
            .run_actor_once(actor, frame)
            .expect("qc run-once claim failed")
    }

    fn read_move_type(&self, actor: &ActorId) -> Option<i32> {
        self.source.read_move_type(actor).expect("qc movetype read failed")
    }

    fn run_think(&mut self, actor: &OwnedActor, frame: &FrameContext) {
        self.source.run_think(actor, frame).expect("qc think failed");
    }

    fn check_water_transition(&mut self, actor: &OwnedActor) {
        self.source
            .check_water_transition(actor)
            .expect("qc water transition failed");
    }

    fn motion(&self, actor: &OwnedActor, body: &WorldBodyState) -> Option<Q2Motion> {
        let motion = self.source.motion(actor, body).expect("qc motion read failed")?;
        Some(Q2Motion {
            actor: actor.clone(),
            velocity: motion.velocity,
            angular_velocity: motion.angular_velocity,
            kind: match motion.kind {
                GuestMotionKind::Stationary => Q2MotionKind::Stationary,
                GuestMotionKind::Step => Q2MotionKind::Step,
                GuestMotionKind::Fly => Q2MotionKind::Fly,
                GuestMotionKind::Toss => Q2MotionKind::Toss,
                GuestMotionKind::Push => Q2MotionKind::Push,
                GuestMotionKind::FlyMissile => Q2MotionKind::FlyMissile,
                GuestMotionKind::Bounce => Q2MotionKind::Bounce,
            },
            gravity: f64::from(motion.gravity),
            gravity_vector: motion.gravity_vector,
            clip_mask: motion.clip_mask as i32,
            owner: motion.owner,
        })
    }

    fn collision(&self, actor: &OwnedActor) -> Option<ExecutionSharedSolid> {
        let solid = self.source.collision(actor).expect("qc collision read failed")?;
        Some(ExecutionSharedSolid {
            solid: match solid.solid {
                qa_guest::qc::actor_state::SolidKind::None => ExecutionSolidKind::None,
                qa_guest::qc::actor_state::SolidKind::Trigger => ExecutionSolidKind::Trigger,
                qa_guest::qc::actor_state::SolidKind::Brush => ExecutionSolidKind::Brush,
                qa_guest::qc::actor_state::SolidKind::Box => ExecutionSolidKind::Box,
            },
            model: solid.model.and_then(|model| u32::try_from(model).ok()),
            family: PhysicsFamily::Q1,
            owner: solid.owner,
            monster: Some(solid.monster),
            dead_monster: None,
            q1_corpse: solid.q1_corpse,
            item: Some(solid.item),
        })
    }

    fn flags(&self, actor: &OwnedActor) -> ExecutionPhysicsFlags {
        let flags = self.source.execution_flags(actor).expect("qc flags read failed");
        ExecutionPhysicsFlags {
            fly: Some(flags.fly),
            swim: Some(flags.swim),
            partial_ground: Some(flags.partial_ground),
            player: Some(flags.player),
            water_level: Some(flags.water_level as i32),
            water_type: Some(flags.water_type as i32),
            dead: Some(flags.dead),
            ..ExecutionPhysicsFlags::default()
        }
    }

    fn write_flags(&mut self, actor: &OwnedActor, changes: &ExecutionPhysicsFlags) {
        let changes = PhysicsFlagChanges {
            fly: changes.fly,
            swim: changes.swim,
            partial_ground: changes.partial_ground,
            water_level: changes.water_level.map(|level| level as f32),
            water_type: changes.water_type.map(|kind| kind as f32),
        };
        self.source
            .write_execution_flags(actor, &changes)
            .expect("qc flags write failed");
    }

    fn step_pusher(
        &mut self,
        actor: &ActorId,
        elapsed_seconds: f64,
    ) -> Result<(), qa_world::movement::types::MovementError> {
        // Missing siblings: stepping runs `stepQ1Pusher` over
        // `physics.q1PusherServices(projection)` (donor quakec-source.ts
        // 403-406), and `SharedPhysics` exposes no projection-based
        // builder (physics lane). Pushers hold still until it lands,
        // matching the native Q1 host pusher stub.
        let _ = (actor, elapsed_seconds);
        Ok(())
    }
}

/// Local camera source over one [`QuakeCSource`].
struct LocalViewSource<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: &'s QuakeCSource<P>,
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCLocalViewSource for LocalViewSource<'s, P> {
    fn read(&self, actor: &ActorId) -> Option<(Vec3, Vec3)> {
        let actors = self.source.shared.borrow().options.actors.clone();
        if !actors.borrow().is_live(actor) {
            return None;
        }
        let slot = self.source.source_slot(actor);
        let Some(slot) = slot else {
            let physics = self.source.shared.borrow().options.physics.clone();
            let body = physics.borrow().read_body(actor)?;
            return Some((body.origin, body.angles));
        };
        let origin = self.source.field("origin").ok()?;
        let angles = self.source.field("angles").ok()?;
        self.source.machine_read(|machine| {
            let origin = machine.entities().slot_vector(slot as u32, origin).ok()?;
            let angles = machine.entities().slot_vector(slot as u32, angles).ok()?;
            Some((origin, angles))
        })
    }

    fn offset(&self, actor: &ActorId) -> Vec3 {
        self.source
            .client_view_offset(actor)
            .unwrap_or(Vec3 { x: 0.0, y: 0.0, z: 0.0 })
    }
}

/// Local-service host over one [`QuakeCSource`].
struct LocalMessageHost<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: &'s QuakeCSource<P>,
    target: Option<ActorId>,
    content: String,
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> LocalMessageHost<'s, P> {
    fn events(&self) -> Rc<RefCell<dyn QuakeCSourceEvents>> {
        self.source.shared.borrow().options.events.clone()
    }
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCLocalMessageHost
    for LocalMessageHost<'s, P>
{
    fn actor(&self, slot: u16) -> Result<ActorId, QuakeCLocalMessageError> {
        let provider = self.source.prepared.execution.owner.provider.clone();
        let actors = self.source.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, slot as usize);
        actor
            .map(|actor| actor.id().clone())
            .ok_or(QuakeCLocalMessageError::ServiceActor(slot))
    }

    fn recipients(&self) -> Vec<ActorId> {
        self.source
            .shared
            .borrow()
            .active_clients
            .borrow()
            .iter()
            .cloned()
            .collect()
    }

    fn source_actor(&self) -> ActorId {
        self.source.world_actor().expect("QC worldspawn is bound").id().clone()
    }

    fn map(&self) -> String {
        self.source
            .shared
            .borrow()
            .options
            .recipe
            .map
            .geometry
            .requested_path
            .clone()
    }

    fn seconds(&self) -> f64 {
        self.source.current_time()
    }

    fn camera(&self, actor: &ActorId) -> Result<(Vec3, Vec3), QuakeCLocalMessageError> {
        let slot = self
            .source
            .source_slot(actor)
            .ok_or(QuakeCLocalMessageError::ServiceCamera)?;
        let origin = self
            .source
            .field("origin")
            .map_err(|_| QuakeCLocalMessageError::ServiceCamera)?;
        let angles = self
            .source
            .field("angles")
            .map_err(|_| QuakeCLocalMessageError::ServiceCamera)?;
        self.source
            .machine_read(|machine| {
                let origin = machine.entities().slot_vector(slot as u32, origin).ok()?;
                let angles = machine.entities().slot_vector(slot as u32, angles).ok()?;
                Some((origin, angles))
            })
            .ok_or(QuakeCLocalMessageError::ServiceCamera)
    }

    fn sound(&self, index: u16) -> Result<String, QuakeCLocalMessageError> {
        self.source
            .precache_names(PrecacheKind::Sound)
            .get(usize::from(index).wrapping_sub(1))
            .cloned()
            .ok_or(QuakeCLocalMessageError::ServiceSound(index))
    }

    fn model(&self, index: u16) -> Result<String, QuakeCLocalMessageError> {
        self.source
            .precache_names(PrecacheKind::Model)
            .get(usize::from(index).wrapping_sub(1))
            .cloned()
            .ok_or(QuakeCLocalMessageError::ServiceModel(index))
    }

    fn emit(&mut self, event: Q1Event, recipient: Option<ActorId>) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::Q1(event),
            recipient.as_ref().or(self.target.as_ref()),
        );
    }

    fn message(&mut self, event: LocalNetworkEvent, actor: Option<ActorId>) {
        let mapped = match event {
            LocalNetworkEvent::Print { level, text } => ClientMessage::Print { level, text },
            LocalNetworkEvent::CenterPrint { text } => ClientMessage::CenterPrint { text },
            LocalNetworkEvent::CommandText { text } => ClientMessage::CommandText { text },
            LocalNetworkEvent::Disconnect { reason } => ClientMessage::Disconnect { reason },
            _ => return,
        };
        self.events()
            .borrow_mut()
            .message(mapped, actor.as_ref().or(self.target.as_ref()));
    }

    fn music(&mut self, track: u8) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::Music { track },
            self.target.as_ref(),
        );
    }

    fn angles(&mut self, actor: &ActorId, angles: Vec3) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::ViewReset {
                actor: actor.clone(),
                angles,
            },
            self.target.as_ref(),
        );
    }

    fn pause(&mut self, paused: bool) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::Pause { paused },
            self.target.as_ref(),
        );
    }

    fn sky(&mut self, name: &str, recipient: Option<ActorId>) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::Sky { name: name.to_string() },
            recipient.as_ref(),
        );
    }

    fn client_metadata(&mut self, event: Q1ClientMetadataEvent, recipient: Option<ActorId>) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::ClientMetadata(event),
            recipient.as_ref(),
        );
    }

    fn session(&mut self, kind: QuakeCSessionKind, recipient: Option<ActorId>) {
        self.events()
            .borrow_mut()
            .emit_local(&self.content, QuakeCLocalSinkEvent::Session(kind), recipient.as_ref());
    }

    fn prompt(&mut self, event: Q1CompositionEvent) {
        let recipient = match &event {
            Q1CompositionEvent::Prompt { actor, .. } | Q1CompositionEvent::ClearPrompt { actor } => Some(actor.clone()),
            _ => None,
        };
        self.events()
            .borrow_mut()
            .emit_local(&self.content, QuakeCLocalSinkEvent::Prompt(event), recipient.as_ref());
    }

    fn fog(&mut self, fog: QuakeCLocalFog, recipient: Option<ActorId>) {
        self.events().borrow_mut().emit_local(
            &self.content,
            QuakeCLocalSinkEvent::Fog {
                density: fog.density,
                color: fog.color,
                duration_seconds: fog.transition_seconds.max(0.0),
            },
            recipient.as_ref(),
        );
    }
}

/// Project one routed NetQuake message into its wire form.
fn netquake_message_from_nq(message: &NqMessage) -> NetQuakeMessage {
    match message {
        NqMessage::Print { text } => NetQuakeMessage::Text {
            kind: NqText::Print,
            text: text.clone(),
        },
        NqMessage::CenterPrint { text } => NetQuakeMessage::Text {
            kind: NqText::CenterPrint,
            text: text.clone(),
        },
        NqMessage::StuffText { text } => NetQuakeMessage::Text {
            kind: NqText::Stufftext,
            text: text.clone(),
        },
        NqMessage::SetView { entity } => NetQuakeMessage::SetView { entity: *entity },
        NqMessage::Sound {
            entity,
            channel,
            index,
            origin,
            volume,
            attenuation,
        } => NetQuakeMessage::Sound {
            entity: *entity,
            channel: *channel,
            index: u16::from(*index),
            volume: *volume,
            attenuation: f64::from(*attenuation),
            origin: [f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)],
        },
        NqMessage::TempEntity { effect } => NetQuakeMessage::TemporaryEntity {
            effect: match effect {
                TempEntityEffect::ExplosionColors {
                    origin,
                    color_start,
                    color_length,
                } => TemporaryEntity::ExplosionColors {
                    origin: [f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)],
                    color_start: *color_start as u8,
                    color_length: *color_length as u8,
                },
                TempEntityEffect::Beam {
                    entity,
                    beam_type,
                    start,
                    end,
                } => TemporaryEntity::Beam {
                    effect_type: *beam_type,
                    entity: *entity,
                    start: [f64::from(start.x), f64::from(start.y), f64::from(start.z)],
                    end: [f64::from(end.x), f64::from(end.y), f64::from(end.z)],
                },
                TempEntityEffect::Point {
                    effect_type,
                    origin,
                    count,
                } => TemporaryEntity::Point {
                    effect_type: *effect_type,
                    origin: [f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)],
                    count: *count as u8,
                },
            },
        },
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Drain broadcast presentation into the session sink (`messages.flush`,
    /// donor runtime.ts 4745).
    pub fn flush_messages(&self) -> Result<(), QuakeCSourceError> {
        let content = self.prepared.execution.owner.content.clone();
        let events = self.shared.borrow().options.events.clone();
        self.shared.borrow_mut().messages.flush(&mut |effect, recipient| {
            events
                .borrow_mut()
                .emit(&content, map_broadcast_effect(effect), recipient);
        })?;
        Ok(())
    }

    /// Drain signon presentation into the session sink (`messages.flushSignon`).
    fn flush_signon_messages(&self) -> Result<(), QuakeCSourceError> {
        self.shared.borrow_mut().messages.flush_signon()?;
        Ok(())
    }

    /// Spawn the source map (`spawnMap`).
    pub fn spawn_map(&self) -> Result<(), QuakeCSourceError> {
        if !self.loading() {
            return Err(QuakeCSourceError::Invalid("QC map was already spawned".to_string()));
        }
        let map = self.shared.borrow().options.recipe.map.geometry.requested_path.clone();
        let model_word = self.field("model")?;
        let modelindex_word = self.field("modelindex")?;
        let solid_word = self.field("solid")?;
        let movetype_word = self.field("movetype")?;
        self.machine_write(|machine| {
            let model = machine.strings_mut().allocate(&map)?;
            machine.entities_mut().set_slot_int(0, model_word, model)?;
            machine.entities_mut().set_slot_float(0, modelindex_word, 1.0)?;
            machine.entities_mut().set_slot_float(0, solid_word, 4.0)?;
            machine.entities_mut().set_slot_float(0, movetype_word, 7.0)
        })?;
        let time = self.current_time();
        self.machine_write(|machine| {
            let offset = machine.global_offset("time")?;
            machine.globals_mut().set_float(offset, time as f32)
        })?;
        for name in ["skill", "deathmatch", "coop", "teamplay"] {
            let value = self.shared.borrow().cvars.borrow().variable_value(name);
            if let Some(definition) = self.prepared.program.global_named(name) {
                let offset = definition.offset;
                self.machine_write(|machine| machine.globals_mut().set_float(offset, value))?;
            }
        }
        let stripped = map.strip_prefix("maps/").unwrap_or(map.as_str());
        let stripped = stripped.strip_suffix(".bsp").unwrap_or(stripped);
        self.machine_write(|machine| {
            let offset = machine.global_offset("mapname")?;
            let text = machine.strings_mut().allocate(stripped)?;
            machine.globals_mut().set_int(offset, text)
        })?;
        let mode = self.shared.borrow().options.mode;
        let world = self.shared.borrow().options.world.clone();
        let text = world.borrow().map_entities(mode);
        let entities = parse_entities(&text, &map).map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
        let provider = self.prepared.execution.owner.provider.clone();
        let skill = self.shared.borrow().options.skill;
        let spawnflags_word = self.field("spawnflags")?;
        let classname_word = self.field("classname")?;
        for (ordinal, pairs) in entities.iter().enumerate() {
            let actor = if ordinal == 0 {
                self.world_actor()?
            } else {
                self.shared
                    .borrow()
                    .options
                    .actors
                    .borrow_mut()
                    .allocate(&provider, QuakeCSlotKind::Authored.definition())
            };
            let slot = self
                .source_slot(actor.id())
                .ok_or_else(|| QuakeCSourceError::Invalid("Missing authored QC source slot".to_string()))?;
            self.bind_slot(slot, &actor)?;
            let mut ordered: Vec<GuestTextPair> = pairs
                .iter()
                .map(|(key, value)| GuestTextPair {
                    key: key.clone(),
                    value: value.clone(),
                })
                .collect();
            ordered.sort_by(|left, right| left.key.cmp(&right.key));
            self.machine_write(|machine| apply_qc_entity_pairs(machine, slot as u32, &ordered))?;
            let excluded = if mode == QuakeCSourceMode::Deathmatch {
                2048
            } else if skill == 0 {
                256
            } else if skill == 1 {
                512
            } else {
                1024
            };
            let flags =
                self.machine_read(|machine| machine.entities().slot_float(slot as u32, spawnflags_word))? as i32;
            if flags & excluded != 0 {
                self.free_slot(&actor, slot)?;
                continue;
            }
            let classname = self.machine_read(|machine| {
                let text = machine.entities().slot_int(slot as u32, classname_word)?;
                machine.strings().get(text)
            })?;
            let spawn = self
                .prepared
                .program
                .function_named(&classname)
                .ok()
                .map(|function| function.index);
            if classname.is_empty() || spawn.is_none() {
                let print = self.shared.borrow().options.print.clone();
                if classname.is_empty() {
                    print(&format!("No classname for: {classname}\n"));
                } else {
                    print(&format!("No spawn function for: {classname}\n"));
                }
                self.free_slot(&actor, slot)?;
                continue;
            }
            let time = self.current_time();
            self.invoke(spawn.unwrap_or(0) as i32, slot, 0, time)?;
            self.flush_signon_messages()?;
        }
        *self.shared.borrow_mut().spawning.borrow_mut() = false;
        let gravity = self.shared.borrow().cvars.borrow().variable_value("sv_gravity");
        self.shared
            .borrow()
            .options
            .physics
            .borrow_mut()
            .set_world_gravity(gravity);
        self.flush_messages()
    }

    /// Start one native frame (`beginFrame`).
    pub fn begin_frame(&self, frame: &FrameContext) -> Result<(), QuakeCSourceError> {
        let (qa_core::time::SourceTime::Seconds(time), qa_core::time::SourceTime::Seconds(elapsed)) =
            (frame.time, frame.elapsed)
        else {
            return Err(QuakeCSourceError::Invalid(
                "QC frame requires source seconds".to_string(),
            ));
        };
        if !time.is_finite() {
            return Err(QuakeCSourceError::Invalid(
                "QC frame requires source seconds".to_string(),
            ));
        }
        if self.kind() == QuakeCSourceKind::Netquake {
            let router = self.shared.borrow().router.clone();
            let mut router = router.borrow_mut();
            if !router.netquake_wire_attached {
                if !router.netquake_signon.is_empty() {
                    return Err(QuakeCSourceError::Invalid(
                        "QuakeC MSG_INIT requires a native NetQuake wire consumer".to_string(),
                    ));
                }
                router.netquake_routed.clear();
            }
        }
        self.shared.borrow_mut().current_time = f64::from(time);
        self.machine_write(|machine| {
            let offset = machine.global_offset("frametime")?;
            machine.globals_mut().set_float(offset, elapsed)
        })?;
        let gravity = self.shared.borrow().cvars.borrow().variable_value("sv_gravity");
        self.shared
            .borrow()
            .options
            .physics
            .borrow_mut()
            .set_world_gravity(gravity);
        let index = self.prepared.program.function_named("StartFrame")?.index;
        self.invoke(index as i32, 0, 0, f64::from(time))
    }

    /// Link one actor before reaction (`beforeActor`).
    pub fn before_actor(&self, actor: &OwnedActor) -> Result<(), QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor.id()) else {
            return Ok(());
        };
        let force = self.machine_read(|machine| {
            let offset = machine.global_offset("force_retouch")?;
            machine.globals().float(offset)
        })?;
        if force == 0.0 {
            return Ok(());
        }
        self.shared.borrow_mut().world.link(slot)?;
        self.drain_admissions()?;
        if let Some(error) = self.shared.borrow().hook_error.borrow_mut().take() {
            return Err(QuakeCSourceError::Guest(error));
        }
        self.shared.borrow().options.physics.borrow_mut().touch_triggers(actor);
        Ok(())
    }

    /// End one native frame (`endFrame`).
    pub fn end_frame(&self) -> Result<(), QuakeCSourceError> {
        self.flush_messages()?;
        {
            let router = self.shared.borrow().router.clone();
            let mut router = router.borrow_mut();
            if router.local_messages_started && !router.netquake_wire_attached {
                router.netquake_routed.clear();
            }
        }
        let offset = self.machine_read(|machine| machine.global_offset("force_retouch"))?;
        let current = self.machine_read(|machine| machine.globals().float(offset))?;
        if current != 0.0 {
            let numeric = self.shared.borrow().numeric;
            self.machine_write(|machine| {
                machine
                    .globals_mut()
                    .set_float(offset, numeric.sub(f64::from(current), 1.0) as f32)
            })?;
        }
        Ok(())
    }

    /// Movement type for one actor (`readMoveType`).
    pub fn read_move_type(&self, actor: &ActorId) -> Result<Option<i32>, QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor) else {
            return Ok(None);
        };
        let word = self.field("movetype")?;
        let move_type = self.machine_read(|machine| machine.entities().slot_float(slot as u32, word))?;
        Ok(Some(move_type as i32))
    }

    /// Notarget flag for one actor (`notarget`).
    pub fn notarget(&self, actor: &ActorId) -> Result<Option<bool>, QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor) else {
            return Ok(None);
        };
        let word = self.field("flags")?;
        let flags = self.machine_read(|machine| machine.entities().slot_float(slot as u32, word))?;
        Ok(Some(flags as i32 & 128 != 0))
    }

    /// Classname for one actor (`classname`).
    pub fn classname(&self, actor: &ActorId) -> Result<String, QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor) else {
            return Ok(String::new());
        };
        let word = self.field("classname")?;
        self.machine_read(|machine| {
            let text = machine.entities().slot_int(slot as u32, word)?;
            machine.strings().get(text)
        })
        .map_err(QuakeCSourceError::from)
    }

    /// Apply one actor's water transition (`checkWaterTransition`).
    pub fn check_water_transition(&self, actor: &OwnedActor) -> Result<(), QuakeCSourceError> {
        let Some(slot) = self.source_slot(actor.id()) else {
            return Ok(());
        };
        let origin_word = self.field("origin")?;
        let type_word = self.field("watertype")?;
        let level_word = self.field("waterlevel")?;
        let (origin, previous) = self.machine_read(|machine| {
            let origin = machine.entities().slot_vector(slot as u32, origin_word)?;
            let previous = machine.entities().slot_float(slot as u32, type_word)?;
            Ok::<_, GuestError>((origin, previous))
        })?;
        let scene = self.shared.borrow().options.scene.clone();
        let contents = scene.borrow().point_contents(origin)?;
        let transition = q1_water_transition(previous as i32, contents as i32);
        if transition.splash {
            let path = "misc/h2ohit1.wav";
            let key = format!("sound:{path}");
            if !self.shared.borrow().precached.contains_key(&key) {
                let print = self.shared.borrow().options.print.clone();
                print(&format!("SV_StartSound: {path} not precacheed\n"));
            } else {
                let content = self.prepared.execution.owner.content.clone();
                let events = self.shared.borrow().options.events.clone();
                events.borrow_mut().emit(
                    &content,
                    QcPresentationEvent::Sound {
                        actor: actor.id().clone(),
                        channel: qa_guest::qc::presentation_host::Q1SoundChannel::Auto,
                        path: path.to_string(),
                        volume: 1.0,
                        attenuation: 1.0,
                    },
                    None,
                );
            }
        }
        self.machine_write(|machine| {
            machine
                .entities_mut()
                .set_slot_float(slot as u32, type_word, transition.water_type as f32)?;
            machine
                .entities_mut()
                .set_slot_float(slot as u32, level_word, transition.water_level as f32)
        })?;
        Ok(())
    }

    /// Write one actor's angular velocity (`writeAngularVelocity`).
    pub fn write_angular_velocity(&self, actor: &OwnedActor, value: Vec3) -> Result<(), QuakeCSourceError> {
        let shared = self.shared.borrow();
        let fields = shared.fields.clone();
        shared
            .actor_state
            .write_angular_velocity(&mut fields.borrow_mut(), actor.id(), value)?;
        Ok(())
    }
}

/// Project one broadcast effect into a session presentation event.
fn map_broadcast_effect(effect: &QcBroadcastEffect) -> QcPresentationEvent {
    match effect {
        QcBroadcastEffect::Effect {
            effect,
            actor,
            origin,
            amount,
        } => QcPresentationEvent::Effect {
            effect: *effect,
            actor: actor.clone(),
            origin: *origin,
            amount: *amount,
        },
        QcBroadcastEffect::Beam {
            style,
            actor,
            start,
            end,
        } => QcPresentationEvent::Beam {
            style: *style,
            actor: actor.clone(),
            start: *start,
            end: *end,
        },
        QcBroadcastEffect::ColoredExplosion {
            origin,
            color_start,
            color_length,
        } => QcPresentationEvent::ColoredExplosion {
            origin: *origin,
            color_start: *color_start,
            color_length: *color_length,
        },
        QcBroadcastEffect::Particles {
            origin,
            direction,
            color,
            count,
        } => QcPresentationEvent::Particles {
            origin: *origin,
            direction: *direction,
            color: *color,
            count: *count,
        },
    }
}

impl<P: qa_content::contract::OriginalPickupAdmission + 'static> QuakeCSource<P> {
    /// Capture an original Quake save (`captureOriginalSave`).
    pub fn capture_original_save(&self, format: Q1SaveFormat, comment: &str) -> Result<Q1SaveData, QuakeCSourceError> {
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, 1);
        let live = actor.as_ref().is_some_and(|actor| self.is_active_client(actor.id()));
        let parameters = self.shared.borrow().spawn_parameters.get(&1).cloned();
        let Some(parameters) = parameters.filter(|_| live) else {
            return Err(QuakeCSourceError::Invalid(
                "Cannot capture an original save for an idle source".to_string(),
            ));
        };
        let extension = self.shared.borrow().original_save_extension_text.clone();
        let mut source = OriginalSaveSource::new(self, 0.0);
        let save = capture_q1_source_save(&mut source, format, comment, parameters, &extension)
            .map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
        if let Some(error) = source.error.borrow_mut().take() {
            return Err(error);
        }
        Ok(save)
    }

    /// Restore an original Quake save (`restoreOriginalSave`).
    pub fn restore_original_save(&self, save: &Q1SaveData) -> Result<Q1UnknownSaveFields, QuakeCSourceError> {
        let pending =
            self.shared.borrow().options.original_save_candidate && !self.shared.borrow().original_save_restored;
        if !pending {
            return Err(QuakeCSourceError::Invalid(
                "QuakeC source has no pending original save".to_string(),
            ));
        }
        let provider = self.prepared.execution.owner.provider.clone();
        let actors = self.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&provider, 1);
        let live = actor.as_ref().is_some_and(|actor| self.is_active_client(actor.id()));
        let parameters = self.shared.borrow().spawn_parameters.contains_key(&1);
        if !live || !parameters {
            return Err(QuakeCSourceError::Invalid(
                "Cannot restore an original save for an idle source".to_string(),
            ));
        }
        let mut source = OriginalSaveSource::new(self, save.time as f32);
        let extension = save.extension_text.clone();
        let unknowns = restore_q1_source_save(&mut source, save, |header: Q1SaveHeader| {
            let mut shared = self.shared.borrow_mut();
            shared.current_time = header.time;
            shared.spawn_parameters.insert(1, header.spawn_parameters.clone());
            shared.change_level_issued = false;
            shared.pending_weapons.borrow_mut().clear();
            shared.original_save_extension_text = extension.clone();
            shared.original_save_restored = true;
        })
        .map_err(|error| QuakeCSourceError::Invalid(error.to_string()))?;
        if let Some(error) = source.error.borrow_mut().take() {
            return Err(error);
        }
        Ok(unknowns)
    }
}

/// Original-save adapter over one [`QuakeCSource`].
struct OriginalSaveSource<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: &'s QuakeCSource<P>,
    machine: OriginalSaveMachine<'s, P>,
    staging: OriginalSaveStaging<'s, P>,
    error: Rc<RefCell<Option<QuakeCSourceError>>>,
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> OriginalSaveSource<'s, P> {
    fn new(source: &'s QuakeCSource<P>, now_seconds: f32) -> Self {
        let error = Rc::new(RefCell::new(None));
        Self {
            source,
            machine: OriginalSaveMachine {
                source,
                error: error.clone(),
            },
            staging: OriginalSaveStaging {
                source,
                error: error.clone(),
                now_seconds,
            },
            error,
        }
    }

    fn record(&self, error: QuakeCSourceError) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> Q1Source for OriginalSaveSource<'s, P> {
    type Machine = OriginalSaveMachine<'s, P>;
    type Staging = OriginalSaveStaging<'s, P>;

    fn checkpoint(&mut self) {
        if let Err(error) = self.source.checkpoint() {
            self.record(error);
        }
    }

    fn kind(&self) -> String {
        checkpoint_kind(self.source.kind()).to_string()
    }

    fn max_clients(&self) -> u32 {
        self.source.shared.borrow().options.max_clients as u32
    }

    fn mode(&self) -> String {
        match self.source.shared.borrow().options.mode {
            QuakeCSourceMode::Singleplayer => "singleplayer",
            QuakeCSourceMode::Coop => "coop",
            QuakeCSourceMode::Deathmatch => "deathmatch",
        }
        .to_string()
    }

    fn skill(&self) -> i32 {
        i32::from(self.source.shared.borrow().options.skill)
    }

    fn map_geometry_path(&self) -> String {
        self.source
            .shared
            .borrow()
            .options
            .recipe
            .map
            .geometry
            .requested_path
            .clone()
    }

    fn time_seconds(&self) -> f64 {
        self.source.current_time()
    }

    fn light_style(&self, index: usize) -> String {
        self.source.shared.borrow().options.events.borrow().light_style(index)
    }

    fn loading(&self) -> bool {
        self.source.loading()
    }

    fn split(&mut self) -> (&mut Self::Machine, &mut Self::Staging) {
        (&mut self.machine, &mut self.staging)
    }
}

/// Original-save machine adapter over one [`QuakeCSource`].
struct OriginalSaveMachine<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: &'s QuakeCSource<P>,
    error: Rc<RefCell<Option<QuakeCSourceError>>>,
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> OriginalSaveMachine<'s, P> {
    fn record(&self, error: QuakeCSourceError) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> Q1QuakeCMachine for OriginalSaveMachine<'s, P> {
    fn snapshot(&mut self) {
        let executing = EXECUTING.with(|executing| executing.borrow().is_some());
        if executing {
            self.record(QuakeCSourceError::Invalid(
                "Original save requires an idle source machine".to_string(),
            ));
        }
    }

    fn entity_count(&self) -> usize {
        self.source.machine_read(|machine| machine.entities().count())
    }

    fn entity_capacity(&self) -> usize {
        self.source.machine_read(|machine| machine.entities().capacity())
    }

    fn set_entity_count(&mut self, count: usize) {
        if let Err(error) = self
            .source
            .machine_write(|machine| machine.entities_mut().set_count(count))
        {
            self.record(QuakeCSourceError::from(error));
        }
    }

    fn clear_entity(&mut self, slot: usize) {
        if let Err(error) = self
            .source
            .machine_write(|machine| machine.entities_mut().clear_slot(slot as u32))
        {
            self.record(QuakeCSourceError::from(error));
        }
    }

    fn save_global_pairs(&self) -> Vec<SavedTextPair> {
        match self.source.machine_read(save_qc_global_pairs) {
            Ok(pairs) => pairs
                .into_iter()
                .map(|pair| SavedTextPair {
                    key: pair.key,
                    value: pair.value,
                })
                .collect(),
            Err(error) => {
                self.record(QuakeCSourceError::from(error));
                Vec::new()
            }
        }
    }

    fn save_entity_pairs(&self, slot: usize, free: bool) -> Vec<SavedTextPair> {
        match self
            .source
            .machine_read(|machine| save_qc_entity_pairs(machine, slot as u32, free))
        {
            Ok(pairs) => pairs
                .into_iter()
                .map(|pair| SavedTextPair {
                    key: pair.key,
                    value: pair.value,
                })
                .collect(),
            Err(error) => {
                self.record(QuakeCSourceError::from(error));
                Vec::new()
            }
        }
    }

    fn apply_global_pairs(&mut self, pairs: &[SavedTextPair]) -> Vec<SavedTextPair> {
        let guest: Vec<GuestTextPair> = pairs
            .iter()
            .map(|pair| GuestTextPair {
                key: pair.key.clone(),
                value: pair.value.clone(),
            })
            .collect();
        match self
            .source
            .machine_write(|machine| apply_qc_global_pairs(machine, &guest))
        {
            Ok(unknowns) => unknowns
                .into_iter()
                .map(|pair| SavedTextPair {
                    key: pair.key,
                    value: pair.value,
                })
                .collect(),
            Err(error) => {
                self.record(QuakeCSourceError::from(error));
                Vec::new()
            }
        }
    }

    fn apply_entity_pairs(&mut self, slot: usize, pairs: &[SavedTextPair]) -> Q1AppliedEntity {
        let guest: Vec<GuestTextPair> = pairs
            .iter()
            .map(|pair| GuestTextPair {
                key: pair.key.clone(),
                value: pair.value.clone(),
            })
            .collect();
        match self
            .source
            .machine_write(|machine| apply_qc_entity_pairs(machine, slot as u32, &guest))
        {
            Ok(applied) => Q1AppliedEntity {
                empty: applied.empty,
                unknown: applied
                    .unknown
                    .into_iter()
                    .map(|pair| SavedTextPair {
                        key: pair.key,
                        value: pair.value,
                    })
                    .collect(),
            },
            Err(error) => {
                self.record(QuakeCSourceError::from(error));
                Q1AppliedEntity {
                    empty: false,
                    unknown: Vec::new(),
                }
            }
        }
    }
}

/// Original-save staging adapter over one [`QuakeCSource`].
struct OriginalSaveStaging<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> {
    source: &'s QuakeCSource<P>,
    error: Rc<RefCell<Option<QuakeCSourceError>>>,
    now_seconds: f32,
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> OriginalSaveStaging<'s, P> {
    fn record(&self, error: QuakeCSourceError) {
        let mut slot = self.error.borrow_mut();
        if slot.is_none() {
            *slot = Some(error);
        }
    }

    fn provider(&self) -> ProviderId {
        self.source.prepared.execution.owner.provider.clone()
    }
}

impl<'s, P: qa_content::contract::OriginalPickupAdmission + 'static> Q1SourceStaging for OriginalSaveStaging<'s, P> {
    fn is_free(&self, slot: usize) -> bool {
        self.source.shared.borrow().storage.borrow().is_free(slot)
    }

    fn staged_entity_count(&self) -> usize {
        self.source.machine_read(|machine| machine.entities().count())
    }

    fn has_actor(&self, slot: usize) -> bool {
        let actors = self.source.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&self.provider(), slot);
        actor.is_some()
    }

    fn unlink_body(&mut self, slot: usize) {
        let actors = self.source.shared.borrow().options.actors.clone();
        let physics = self.source.shared.borrow().options.physics.clone();
        let actor = actors.borrow().at_source(&self.provider(), slot);
        if let Some(actor) = actor {
            physics.borrow_mut().unlink_body(&actor);
        }
    }

    fn release_actor(&mut self, slot: usize) {
        let actors = self.source.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&self.provider(), slot);
        if let Some(actor) = actor {
            actors.borrow_mut().release(&actor);
        }
    }

    fn reserved_client_slots(&self) -> usize {
        self.source.shared.borrow().reserved_client_slots
    }

    fn bind_existing(&mut self, slot: usize) {
        let actors = self.source.shared.borrow().options.actors.clone();
        if actors.borrow().at_source(&self.provider(), slot).is_none() {
            actors
                .borrow_mut()
                .allocate_at_source(&self.provider(), slot, QuakeCSlotKind::Edict.definition());
        }
    }

    fn initialize(&mut self, slot: usize) {
        let actors = self.source.shared.borrow().options.actors.clone();
        let actor = actors.borrow().at_source(&self.provider(), slot);
        let Some(actor) = actor else {
            self.record(QuakeCSourceError::Invalid("Missing QC staged actor".to_string()));
            return;
        };
        let shared = self.source.shared.borrow();
        if shared.storage.borrow().count() <= slot {
            if let Err(error) = shared.storage.borrow_mut().set_count(slot + 1) {
                self.record(QuakeCSourceError::from(error));
                return;
            }
        }
        if !shared.fields.borrow().is_allocated(actor.id()) {
            if let Err(error) = shared.fields.borrow_mut().allocate(actor.id(), &shared.layout) {
                self.record(QuakeCSourceError::from(error));
                return;
            }
        }
        let outcome = shared
            .storage
            .borrow_mut()
            .initialize(slot, &mut shared.fields.borrow_mut(), actor.id());
        if let Err(error) = outcome {
            self.record(QuakeCSourceError::from(error));
        }
    }

    fn clear_freed(&mut self, slot: usize) {
        let shared = self.source.shared.borrow();
        if shared.storage.borrow().count() <= slot {
            if let Err(error) = shared.storage.borrow_mut().set_count(slot + 1) {
                self.record(QuakeCSourceError::from(error));
            }
            return;
        }
        let occupant = shared.storage.borrow().at(slot).cloned();
        let actor = match occupant {
            Some(actor) => Some(actor),
            None => {
                let actors = shared.options.actors.clone();
                let actor = actors.borrow().at_source(&self.provider(), slot);
                actor.map(|actor| actor.id().clone())
            }
        };
        if let Some(actor) = actor {
            if shared.fields.borrow().is_allocated(&actor) {
                if let Err(error) = shared.storage.borrow_mut().clear_freed(
                    slot,
                    &mut shared.fields.borrow_mut(),
                    &actor,
                    self.now_seconds,
                ) {
                    self.record(QuakeCSourceError::from(error));
                }
            }
        }
    }

    fn link(&mut self, slot: usize) {
        if let Err(error) = self.source.shared.borrow_mut().world.link(slot) {
            self.record(QuakeCSourceError::from(error));
            return;
        }
        if let Err(error) = self.source.drain_admissions() {
            self.record(error);
            return;
        }
        if let Some(error) = self.source.shared.borrow().hook_error.borrow_mut().take() {
            self.record(QuakeCSourceError::from(error));
        }
    }

    fn emit_lightstyle(&mut self, style: usize, pattern: &str) {
        let content = self.source.prepared.execution.owner.content.clone();
        let events = self.source.shared.borrow().options.events.clone();
        events.borrow_mut().emit(
            &content,
            QcPresentationEvent::Lightstyle {
                style: style as i32,
                pattern: pattern.to_string(),
            },
            None,
        );
    }
}

/// Guest borrowed-host adapter over the session seams.
struct BorrowedHostView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    machine: &'static MachineView,
    provider: ProviderId,
    provider_name: &'static str,
    foreign_classname: Option<QuakeCForeignClassname>,
}

impl qa_guest::qc::borrowed_actors::BorrowedHost for BorrowedHostView {
    fn is_owned(&self, actor: &ActorId) -> bool {
        self.actors.borrow().resolve_owned(actor).is_some()
    }

    fn resolve_owned(&self, actor: &ActorId) -> Option<ActorId> {
        self.actors
            .borrow()
            .resolve_owned(actor)
            .map(|owned| owned.id().clone())
    }

    fn source_provider(&self, actor: &ActorId) -> Option<String> {
        self.actors
            .borrow()
            .source_of(actor)
            .map(|(provider, _)| format!("{}:{}", provider.namespace, provider.name))
    }

    fn provider(&self) -> &str {
        self.provider_name
    }

    fn body(&self, actor: &ActorId) -> Option<qa_guest::qc::borrowed_actors::BorrowedBody> {
        let body = self.physics.borrow().read_body(actor)?;
        Some(qa_guest::qc::borrowed_actors::BorrowedBody {
            origin: body.origin,
            velocity: body.velocity,
            angles: body.angles,
            bounds: body.bounds,
            linked: None,
        })
    }

    fn combat(&self, actor: &ActorId) -> Option<qa_guest::qc::borrowed_actors::BorrowedCombat> {
        let (provider, slot) = self.actors.borrow().source_of(actor)?;
        if provider != self.provider {
            return None;
        }
        let health = self.machine.entity_float(slot, health_word(self.machine)?).ok()? as f32;
        let takedamage = self.machine.entity_float(slot, takedamage_word(self.machine)?).ok()?;
        Some(qa_guest::qc::borrowed_actors::BorrowedCombat {
            health,
            can_take_damage: takedamage != 0.0,
        })
    }

    fn classname(&self, actor: &ActorId) -> String {
        if let Some(foreign) = &self.foreign_classname {
            return foreign(actor);
        }
        String::new()
    }

    fn solid(&self, actor: &ActorId) -> u8 {
        let Some(collision) = self.physics.borrow().solid_of(actor) else {
            return 0;
        };
        solid_word(&collision)
    }

    fn now_seconds(&self) -> f32 {
        self.machine
            .global_float(time_word(self.machine).unwrap_or(0))
            .unwrap_or(0.0) as f32
    }
}

/// Map a collision snapshot to the donor solid word (0 none, 1 trigger, 4 brush, 3 monster, 2 box).
fn solid_word(collision: &SharedSolid) -> u8 {
    if collision.solid == qa_guest::qc::actor_state::SolidKind::None {
        0
    } else if collision.solid == qa_guest::qc::actor_state::SolidKind::Trigger {
        1
    } else if collision.solid == qa_guest::qc::actor_state::SolidKind::Brush {
        4
    } else if collision.monster {
        3
    } else {
        2
    }
}

fn health_word(machine: &MachineView) -> Option<usize> {
    machine.field_offset("health").ok()
}

fn takedamage_word(machine: &MachineView) -> Option<usize> {
    machine.field_offset("takedamage").ok()
}

fn time_word(machine: &MachineView) -> Option<usize> {
    machine
        .global_definition("time")
        .ok()
        .map(|definition| definition.offset)
}

/// Guest actor-lookup adapter over the session actors seam.
struct ActorLookupView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    machine: &'static MachineView,
    reserved_client_slots: usize,
}

impl qa_guest::qc::actor_state::ActorLookup for ActorLookupView {
    fn source_slot(&self, actor: &ActorId) -> Option<u32> {
        let (provider, slot) = self.actors.borrow().source_of(actor)?;
        (provider == self.provider).then_some(slot as u32)
    }

    fn reference(&self, reference: i32) -> Option<ActorId> {
        if reference < 1 {
            return None;
        }
        let slot = self.machine.entity_slot(reference).ok()?;
        self.actors
            .borrow()
            .at_source(&self.provider, slot)
            .map(|owned| owned.id().clone())
    }

    fn is_client(&self, actor: &ActorId) -> bool {
        let Some(slot) = self.source_slot(actor) else {
            return false;
        };
        (slot as usize) >= 1 && (slot as usize) <= self.reserved_client_slots
    }
}

/// Guest movement-world adapter over the session seams.
struct MovementWorldView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    storage: Rc<RefCell<SourceSlotStorage>>,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    machine: &'static MachineView,
}

impl MovementWorld for MovementWorldView {
    fn slot_actor(&self, slot: usize) -> Result<ActorId, GuestError> {
        self.actors
            .borrow()
            .at_source(&self.provider, slot)
            .map(|owned| owned.id().clone())
            .ok_or_else(|| GuestError::invalid("movement slot has no actor"))
    }

    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
        let (provider, slot) = self
            .actors
            .borrow()
            .source_of(actor)
            .ok_or_else(|| GuestError::invalid("movement actor has no source slot"))?;
        if provider != self.provider {
            return Err(GuestError::invalid("movement actor belongs to another source"));
        }
        self.machine
            .entity_reference(slot)
            .map_err(|error| GuestError::invalid(error.to_string()))
    }

    fn link(&mut self, slot: usize) {
        if let Some(owned) = self.actors.borrow().at_source(&self.provider, slot) {
            self.physics.borrow_mut().link_body(owned.id());
        }
    }

    fn entity_count(&self) -> usize {
        self.storage.borrow().count()
    }
}

/// Guest scene adapter over the session scene seam (shared by movement and spatial bindings).
struct SceneView {
    scene: Rc<RefCell<dyn QuakeCSourceScene>>,
}

impl qa_guest::qc::spatial_host::Scene for SceneView {
    fn trace(
        &self,
        params: &qa_guest::qc::spatial_host::TraceParams,
    ) -> Result<qa_guest::qc::spatial_host::TraceResult, GuestError> {
        self.scene.borrow().trace(params)
    }

    fn point_contents(&self, point: Vec3) -> Result<f32, GuestError> {
        self.scene.borrow().point_contents(point)
    }
}

/// Guest movement-bodies adapter over the session seams.
struct MovementBodiesView {
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
}

impl MovementBodies for MovementBodiesView {
    fn read(&self, actor: &ActorId) -> Option<MonsterBodySnapshot> {
        let body = self.physics.borrow().read_body(actor)?;
        Some(MonsterBodySnapshot {
            origin: body.origin,
            angles: body.angles,
            velocity: body.velocity,
            bounds: body.bounds,
        })
    }

    fn linked(&self, _actor: &ActorId) -> Option<Bounds> {
        None
    }
}

/// Guest random adapter over the source RNG.
struct RandomView {
    random: QcSharedRandom,
}

impl RandomSource for RandomView {
    fn next_integer(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let draw = self.random.borrow_mut().next_integer();
        draw.rem_euclid(bound)
    }
}

/// Guest spatial-world adapter over the session seams.
struct SpatialWorldView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    storage: Rc<RefCell<SourceSlotStorage>>,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    machine: &'static MachineView,
}

impl qa_guest::qc::spatial_host::SpatialWorld for SpatialWorldView {
    fn entity_count(&self) -> usize {
        self.storage.borrow().count()
    }

    fn is_free(&self, slot: usize) -> bool {
        self.storage.borrow().is_free(slot)
    }

    fn slot_actor(&self, slot: usize) -> Result<ActorId, GuestError> {
        self.actors
            .borrow()
            .at_source(&self.provider, slot)
            .map(|owned| owned.id().clone())
            .ok_or_else(|| GuestError::invalid("spatial slot has no actor"))
    }

    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
        let (provider, slot) = self
            .actors
            .borrow()
            .source_of(actor)
            .ok_or_else(|| GuestError::invalid("spatial actor has no source slot"))?;
        if provider != self.provider {
            return Err(GuestError::invalid("spatial actor belongs to another source"));
        }
        self.machine
            .entity_reference(slot)
            .map_err(|error| GuestError::invalid(error.to_string()))
    }

    fn link(&mut self, slot: usize) {
        if let Some(owned) = self.actors.borrow().at_source(&self.provider, slot) {
            self.physics.borrow_mut().link_body(owned.id());
        }
    }
}

/// Guest model-table adapter over the source model registry.
struct SpatialModelsView {
    models: Rc<RefCell<std::collections::HashMap<String, SourceModel>>>,
}

impl qa_guest::qc::spatial_host::ModelTable for SpatialModelsView {
    fn model(&self, name: &str) -> Option<qa_guest::qc::spatial_host::ModelInfo> {
        if name.is_empty() {
            return Some(qa_guest::qc::spatial_host::ModelInfo {
                index: 0,
                bounds: qa_core::math::Bounds {
                    min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    max: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                },
            });
        }
        self.models
            .borrow()
            .get(name)
            .map(|model| qa_guest::qc::spatial_host::ModelInfo {
                index: model.index,
                bounds: model.bounds,
            })
    }
}

/// Guest pusher-world adapter over the session seams.
struct PusherWorldView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    machine: &'static MachineView,
}

impl qa_guest::qc::pusher_host::PusherWorld for PusherWorldView {
    fn source_slot(&self, actor: &ActorId) -> Option<usize> {
        if !self.actors.borrow().is_live(actor) {
            return None;
        }
        let (provider, slot) = self.actors.borrow().source_of(actor)?;
        (provider == self.provider).then_some(slot)
    }

    fn is_live(&self, actor: &ActorId) -> bool {
        self.actors.borrow().is_live(actor)
    }

    fn reference(&self, actor: &ActorId) -> Result<i32, GuestError> {
        let slot = self
            .source_slot(actor)
            .ok_or_else(|| GuestError::invalid("pusher actor has no source slot"))?;
        self.machine
            .entity_reference(slot)
            .map_err(|error| GuestError::invalid(error.to_string()))
    }

    fn link_slot(&mut self, slot: usize) {
        if let Some(owned) = self.actors.borrow().at_source(&self.provider, slot) {
            self.physics.borrow_mut().link_body(owned.id());
        }
    }
}

/// Guest pusher-bodies adapter over the session physics seam.
struct PusherBodiesView {
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
}

impl qa_guest::qc::pusher_host::PusherBodies for PusherBodiesView {
    fn read(&self, actor: &ActorId) -> Option<qa_guest::qc::pusher_host::PusherBodySnapshot> {
        let body = self.physics.borrow().read_body(actor)?;
        Some(qa_guest::qc::pusher_host::PusherBodySnapshot {
            origin: body.origin,
            velocity: body.velocity,
            angles: body.angles,
            bounds: body.bounds,
        })
    }

    fn ground(&self, actor: &ActorId) -> Option<ActorId> {
        self.physics.borrow().read_body(actor)?.ground.clone()
    }

    fn link_foreign(&mut self, actor: &ActorId) {
        self.physics.borrow_mut().link_body(actor);
    }
}

/// Guest foreign-pusher adapter: source slots project locally, foreign actors delegate.
struct ForeignPusherView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
}

/// Convert a world solidity into the guest pusher shape.
///
/// The guest folds corpse rows into boxes, mirroring its own solid-word
/// projection in `qa_guest::qc::pusher_host`.
fn guest_pusher_solid(solid: qa_world::movement::q1::types::Q1Solid) -> qa_guest::qc::pusher_host::PusherSolid {
    use qa_guest::qc::pusher_host::PusherSolid as Guest;
    use qa_world::movement::q1::types::Q1Solid as World;
    match solid {
        World::Not => Guest::Not,
        World::Trigger => Guest::Trigger,
        World::Box => Guest::Box,
        World::SlideBox => Guest::SlideBox,
        World::Bsp => Guest::Bsp,
        World::Corpse => Guest::Box,
    }
}

/// Convert a guest pusher solidity into the world shape.
fn world_pusher_solid(solid: qa_guest::qc::pusher_host::PusherSolid) -> qa_world::movement::q1::types::Q1Solid {
    use qa_guest::qc::pusher_host::PusherSolid as Guest;
    use qa_world::movement::q1::types::Q1Solid as World;
    match solid {
        Guest::Not => World::Not,
        Guest::Trigger => World::Trigger,
        Guest::Bsp => World::Bsp,
        Guest::SlideBox => World::SlideBox,
        Guest::Box => World::Box,
    }
}

/// Convert a world trace hit into the guest shape.
fn guest_trace_hit(hit: &qa_world::movement::types::TraceHit) -> qa_guest::qc::spatial_host::TraceHit {
    use qa_guest::qc::spatial_host::TraceHit as Guest;
    use qa_world::movement::types::TraceHit as World;
    match hit {
        World::None => Guest::None,
        World::World { model } => Guest::World { model: *model },
        World::Actor { actor } => Guest::Actor { actor: actor.clone() },
    }
}

/// Convert a guest trace hit into the world shape.
fn world_trace_hit(hit: &qa_guest::qc::spatial_host::TraceHit) -> qa_world::movement::types::TraceHit {
    use qa_guest::qc::spatial_host::TraceHit as Guest;
    use qa_world::movement::types::TraceHit as World;
    match hit {
        Guest::None => World::None,
        Guest::World { model } => World::World { model: *model },
        Guest::Actor { actor } => World::Actor { actor: actor.clone() },
    }
}

/// Convert a world movement state into the guest pusher state shape.
fn guest_pusher_state(
    state: &qa_world::movement::q1::types::Q1MovementState,
) -> qa_guest::qc::pusher_host::PusherState {
    qa_guest::qc::pusher_host::PusherState {
        origin: state.origin,
        velocity: state.velocity,
        angles: state.angles,
        old_origin: state.old_origin,
        angular_velocity: state.angular_velocity,
        view_angles: state.view_angles,
        punch_angles: state.punch_angles,
        move_type: state.move_type as f32,
        flags: state.flags,
        ground: guest_trace_hit(&state.ground),
        water_level: state.water_level as f32,
        water_type: state.water_type as f32,
        teleport_time_seconds: state.teleport_time_seconds as f32,
        water_jump_direction: state.water_jump_direction,
        ideal_pitch: state.ideal_pitch as f32,
        fix_angle: state.fix_angle,
        health: state.health as f32,
    }
}

/// Convert a guest pusher state into the world movement state shape.
fn world_pusher_state(
    state: &qa_guest::qc::pusher_host::PusherState,
) -> qa_world::movement::q1::types::Q1MovementState {
    qa_world::movement::q1::types::Q1MovementState {
        origin: state.origin,
        velocity: state.velocity,
        angles: state.angles,
        old_origin: state.old_origin,
        angular_velocity: state.angular_velocity,
        view_angles: state.view_angles,
        punch_angles: state.punch_angles,
        move_type: state.move_type as i32,
        flags: state.flags,
        ground: world_trace_hit(&state.ground),
        water_level: state.water_level as i32,
        water_type: state.water_type as i32,
        teleport_time_seconds: f64::from(state.teleport_time_seconds),
        water_jump_direction: state.water_jump_direction,
        ideal_pitch: f64::from(state.ideal_pitch),
        fix_angle: state.fix_angle,
        health: f64::from(state.health),
    }
}

/// Convert a world physics entity into the guest pusher shape.
fn guest_pusher_entity(
    entity: &qa_world::movement::q1::types::Q1PhysicsEntity,
) -> qa_guest::qc::pusher_host::Q1PhysicsEntity {
    qa_guest::qc::pusher_host::Q1PhysicsEntity {
        actor: entity.actor.id().clone(),
        bounds: entity.bounds,
        absolute_bounds: entity.absolute_bounds,
        solid: guest_pusher_solid(entity.solid),
        local_time_seconds: entity.local_time_seconds as f32,
        next_think_seconds: entity.next_think_seconds as f32,
        state: guest_pusher_state(&entity.state),
    }
}

/// Convert a guest pusher entity into the world shape.
fn world_pusher_entity(
    entity: &qa_guest::qc::pusher_host::Q1PhysicsEntity,
    owned: OwnedActor,
) -> qa_world::movement::q1::types::Q1PhysicsEntity {
    qa_world::movement::q1::types::Q1PhysicsEntity {
        actor: owned,
        state: world_pusher_state(&entity.state),
        bounds: entity.bounds,
        absolute_bounds: entity.absolute_bounds,
        solid: world_pusher_solid(entity.solid),
        local_time_seconds: f64::from(entity.local_time_seconds),
        next_think_seconds: f64::from(entity.next_think_seconds),
    }
}

impl qa_guest::qc::pusher_host::ForeignPusher for ForeignPusherView {
    fn read(&self, actor: &ActorId) -> Option<qa_guest::qc::pusher_host::Q1PhysicsEntity> {
        if self
            .actors
            .borrow()
            .source_of(actor)
            .is_some_and(|(provider, _)| provider == self.provider)
        {
            return None;
        }
        self.physics
            .borrow()
            .read_q1_pusher(actor)
            .as_ref()
            .map(guest_pusher_entity)
    }

    fn write(&mut self, entity: &qa_guest::qc::pusher_host::Q1PhysicsEntity) {
        if self
            .actors
            .borrow()
            .source_of(&entity.actor)
            .is_some_and(|(provider, _)| provider == self.provider)
        {
            return;
        }
        let Some(owned) = self.actors.borrow().resolve_owned(&entity.actor) else {
            return;
        };
        self.physics
            .borrow_mut()
            .write_q1_pusher(&world_pusher_entity(entity, owned));
    }
}

/// Guest pusher-physics adapter over the session seams.
struct PusherPhysicsView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    machine: &'static MachineView,
    callback: Rc<RefCell<Option<QuakeCPhysicsCallback>>>,
}

impl qa_guest::qc::pusher_host::PusherPhysics for PusherPhysicsView {
    fn on_link(&mut self, actor: &ActorId, touch: bool) {
        if !touch {
            return;
        }
        if let Some(owned) = self.actors.borrow().resolve_owned(actor) {
            self.physics.borrow_mut().touch_triggers(&owned);
        }
    }

    fn on_blocked(&mut self, actor: &ActorId, other: Option<&ActorId>) {
        let function_index = self
            .actors
            .borrow()
            .source_of(actor)
            .filter(|(provider, _)| *provider == self.provider)
            .and_then(|(_, slot)| {
                self.machine
                    .field_offset("blocked")
                    .ok()
                    .and_then(|word| self.machine.entity_int(slot, word).ok())
            })
            .unwrap_or(0)
            .max(0) as usize;
        let Some(other) = other.cloned() else {
            return;
        };
        *self.callback.borrow_mut() = Some(QuakeCPhysicsCallback {
            kind: qa_content::q1::quakec::id1_environment::EnvCallbackKind::Blocked,
            actor: actor.clone(),
            other,
            function_index,
        });
    }

    fn set_collision_enabled(&mut self, actor: &ActorId, enabled: bool) {
        if let Some(owned) = self.actors.borrow().resolve_owned(actor) {
            self.physics.borrow_mut().set_collision_enabled(&owned, enabled);
        }
    }
}

/// Guest pusher-invoker adapter running QC think/blocked callbacks.
struct PusherInvokerView {
    machine: &'static MachineView,
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
}

impl qa_guest::qc::pusher_host::PusherInvoker for PusherInvokerView {
    fn invoke(
        &mut self,
        actor: &ActorId,
        callback: qa_guest::qc::pusher_host::PusherCallback,
        callback_word: i32,
        other: Option<&ActorId>,
        server_time_seconds: f32,
    ) -> Result<(), GuestError> {
        let _ = callback;
        if callback_word == 0 {
            return Ok(());
        }
        let (provider, slot) = self
            .actors
            .borrow()
            .source_of(actor)
            .filter(|(provider, _)| *provider == self.provider)
            .ok_or_else(|| GuestError::invalid("pusher callback requires a live QC actor"))?;
        let _ = provider;
        let other_reference = other
            .map(|other| {
                self.actors
                    .borrow()
                    .source_of(other)
                    .filter(|(provider, _)| *provider == self.provider)
                    .and_then(|(_, slot)| self.machine.entity_reference(slot).ok())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        let reference = self
            .machine
            .entity_reference(slot)
            .map_err(|error| GuestError::invalid(error.to_string()))?;
        run_pusher_callback(
            self.machine,
            reference,
            other_reference,
            callback_word,
            server_time_seconds,
        )
        .map_err(|error| GuestError::invalid(error.to_string()))
    }
}

/// Run one pusher think/blocked callback with saved globals.
///
/// Donor pusher-host `invoke` from `src/compat/qc/pusher-host.ts`.
fn run_pusher_callback(
    machine: &MachineView,
    reference: i32,
    other_reference: i32,
    callback_word: i32,
    server_time_seconds: f32,
) -> Result<(), QcError> {
    let this = machine.global_definition("self")?.offset;
    let other = machine.global_definition("other")?.offset;
    let time = machine.global_definition("time")?.offset;
    let saved_this = machine.global_int(this)?;
    let saved_other = machine.global_int(other)?;
    machine.set_global_int(this, reference)?;
    machine.set_global_int(other, other_reference)?;
    machine.set_global_float(time, f64::from(server_time_seconds))?;
    let outcome = machine.execute(callback_word as usize, 0);
    machine.set_global_int(this, saved_this)?;
    machine.set_global_int(other, saved_other)?;
    outcome
}

/// Source pusher projection over the machine words and session physics.
///
/// Donor `physical` projection from `createQcPusherServices` in `quakec-source.ts`: the host
/// driver reads and writes source entities through this projection, so words are reached through
/// the shared machine view and bodies through physics. Field names mirror the guest pusher
/// projection in `qa_guest::qc::pusher_host`.
struct SourcePusherProjection {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    physics: Rc<RefCell<dyn QuakeCSourcePhysics>>,
    machine: &'static MachineView,
    callback: Rc<RefCell<Option<QuakeCPhysicsCallback>>>,
    hook_error: Rc<RefCell<Option<GuestError>>>,
    quakeworld: bool,
}

impl SourcePusherProjection {
    fn slot(&self, actor: &ActorId) -> Option<usize> {
        self.actors
            .borrow()
            .source_of(actor)
            .filter(|(provider, _)| *provider == self.provider)
            .map(|(_, slot)| slot)
    }

    fn word(&self, name: &str) -> Option<usize> {
        self.machine.field_offset(name).ok()
    }

    fn float(&self, slot: usize, name: &str) -> f32 {
        self.word(name)
            .and_then(|word| self.machine.entity_float(slot, word).ok())
            .unwrap_or(0.0) as f32
    }

    fn vector(&self, slot: usize, name: &str) -> Vec3 {
        const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        self.word(name)
            .and_then(|word| self.machine.entity_vector(slot, word).ok())
            .unwrap_or(ZERO)
    }

    fn set_float(&self, slot: usize, name: &str, value: f32) {
        if let Ok(word) = self.machine.field_offset(name) {
            let machine = self.machine;
            machine.write(|m| {
                m.entities_mut().set_slot_float(slot as u32, word, value).ok();
            });
        }
    }

    fn set_vector(&self, slot: usize, name: &str, value: Vec3) {
        if let Ok(word) = self.machine.field_offset(name) {
            let machine = self.machine;
            machine.write(|m| {
                m.entities_mut().set_slot_vector(slot as u32, word, value).ok();
            });
        }
    }
}

impl QuakeCSourcePusherProjection for SourcePusherProjection {
    fn read(&self, actor: &ActorId) -> Option<qa_world::movement::q1::types::Q1PhysicsEntity> {
        let slot = self.slot(actor)?;
        let owned = self.actors.borrow().resolve_owned(actor)?;
        let body = self.physics.borrow().read_body(actor)?;
        let flags = self.float(slot, "flags") as i32;
        let ground_word = self.float(slot, "groundentity") as i32;
        let ground = if flags & qa_guest::qc::actor_state::FLAG_ONGROUND == 0 {
            qa_world::movement::types::TraceHit::None
        } else if ground_word == 0 {
            qa_world::movement::types::TraceHit::World { model: 0 }
        } else if let Some(ground) = body.ground.clone() {
            qa_world::movement::types::TraceHit::Actor { actor: ground }
        } else {
            qa_world::movement::types::TraceHit::None
        };
        let solid_word = self.float(slot, "solid");
        let solid = if solid_word == 0.0 {
            qa_world::movement::q1::types::Q1Solid::Not
        } else if solid_word == 1.0 {
            qa_world::movement::q1::types::Q1Solid::Trigger
        } else if solid_word == 4.0 {
            qa_world::movement::q1::types::Q1Solid::Bsp
        } else if solid_word == 3.0 {
            qa_world::movement::q1::types::Q1Solid::SlideBox
        } else if solid_word == 5.0 {
            qa_world::movement::q1::types::Q1Solid::Corpse
        } else {
            qa_world::movement::q1::types::Q1Solid::Box
        };
        Some(qa_world::movement::q1::types::Q1PhysicsEntity {
            actor: owned,
            state: qa_world::movement::q1::types::Q1MovementState {
                origin: body.origin,
                velocity: body.velocity,
                angles: body.angles,
                old_origin: self.vector(slot, "oldorigin"),
                angular_velocity: self.vector(slot, "avelocity"),
                view_angles: self.vector(slot, "v_angle"),
                punch_angles: if self.quakeworld {
                    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
                } else {
                    self.vector(slot, "punchangle")
                },
                move_type: self.float(slot, "movetype") as i32,
                flags,
                ground,
                water_level: self.float(slot, "waterlevel") as i32,
                water_type: self.float(slot, "watertype") as i32,
                teleport_time_seconds: f64::from(self.float(slot, "teleport_time")),
                water_jump_direction: self.vector(slot, "movedir"),
                ideal_pitch: if self.quakeworld {
                    0.0
                } else {
                    f64::from(self.float(slot, "idealpitch"))
                },
                fix_angle: self.float(slot, "fixangle") != 0.0,
                health: f64::from(self.float(slot, "health")),
            },
            bounds: body.bounds,
            absolute_bounds: Bounds {
                min: self.vector(slot, "absmin"),
                max: self.vector(slot, "absmax"),
            },
            solid,
            local_time_seconds: f64::from(self.float(slot, "ltime")),
            next_think_seconds: f64::from(self.float(slot, "nextthink")),
        })
    }

    fn write(&mut self, entity: &qa_world::movement::q1::types::Q1PhysicsEntity) {
        let Some(slot) = self.slot(entity.actor.id()) else {
            self.physics.borrow_mut().write_q1_pusher(entity);
            return;
        };
        let state = &entity.state;
        self.set_vector(slot, "origin", state.origin);
        self.set_vector(slot, "angles", state.angles);
        self.set_float(slot, "flags", state.flags as f32);
        self.set_vector(slot, "mins", entity.bounds.min);
        self.set_vector(slot, "maxs", entity.bounds.max);
        self.set_float(slot, "ltime", entity.local_time_seconds as f32);
        self.set_float(slot, "nextthink", entity.next_think_seconds as f32);
        if let Some(mut body) = self.physics.borrow().read_body(entity.actor.id()) {
            body.origin = state.origin;
            body.angles = state.angles;
            body.bounds = entity.bounds;
            self.physics.borrow_mut().write_body(&entity.actor, body);
        }
    }

    fn link(&mut self, actor: &OwnedActor, touch_triggers: bool) {
        self.physics.borrow_mut().link_body(actor.id());
        if touch_triggers {
            self.physics.borrow_mut().touch_triggers(actor);
        }
    }

    fn blocked(&mut self, pusher: &OwnedActor, obstacle: &ActorId) {
        let Some(slot) = self.slot(pusher.id()) else {
            return;
        };
        let callback_word = self
            .word("blocked")
            .and_then(|word| self.machine.entity_int(slot, word).ok())
            .unwrap_or(0);
        if callback_word == 0 {
            return;
        }
        let reference = self.machine.entity_reference(slot).unwrap_or(0);
        let other_reference = self
            .slot(obstacle)
            .and_then(|slot| self.machine.entity_reference(slot).ok())
            .unwrap_or(0);
        let time = self
            .machine
            .global_definition("time")
            .ok()
            .and_then(|definition| self.machine.global_float(definition.offset).ok())
            .unwrap_or(0.0) as f32;
        let prior = self.callback.borrow_mut().replace(QuakeCPhysicsCallback {
            kind: qa_content::q1::quakec::id1_environment::EnvCallbackKind::Blocked,
            actor: pusher.id().clone(),
            other: obstacle.clone(),
            function_index: callback_word.max(0) as usize,
        });
        let outcome = run_pusher_callback(self.machine, reference, other_reference, callback_word, time);
        *self.callback.borrow_mut() = prior;
        if let Err(error) = outcome {
            *self.hook_error.borrow_mut() = Some(GuestError::invalid(error.to_string()));
        }
    }
}

/// Content damage projection over the session seams.
///
/// Donor damage `admit` / `actor` / `reference` / `completed` closures from the `QuakeCSource`
/// constructor. Reaction routing has no donor counterpart and stays inert.
struct DamageProjectionView {
    actors: Rc<RefCell<dyn QuakeCSourceActors>>,
    provider: ProviderId,
    machine: &'static MachineView,
    borrowed: Rc<RefCell<QcBorrowedActors<BorrowedHostView>>>,
    fields: Rc<RefCell<FieldTable>>,
    damage_allowed: Option<QuakeCDamageAllowed>,
    incoming: Rc<RefCell<Vec<IncomingDamage>>>,
}

impl qa_content::q1::quakec::id1_damage::Id1DamageProjection for DamageProjectionView {
    fn admit(&self, request: &qa_content::q1::foundation::gameplay::DamageRequest) -> bool {
        match &self.damage_allowed {
            None => true,
            Some(allowed) => allowed(request),
        }
    }

    fn actor(&self, reference: i32) -> Result<ActorId, QcError> {
        let slot = self.machine.entity_slot(reference)?;
        let borrowed = self.borrowed.borrow().actor(slot).map_err(qc_error)?;
        let actor = match borrowed {
            Some(actor) => actor,
            None => self
                .actors
                .borrow()
                .at_source(&self.provider, slot)
                .map(|owned| owned.id().clone())
                .ok_or_else(|| QcError::program("QC damage references a free source actor", "progs.dat"))?,
        };
        if !self.actors.borrow().is_live(&actor) {
            return Err(QcError::program(
                "QC damage references a free source actor",
                "progs.dat",
            ));
        }
        Ok(actor)
    }

    fn reference(&self, actor: Option<&ActorId>) -> Result<i32, QcError> {
        match actor {
            None => self.machine.entity_reference(0),
            Some(actor) => {
                if let Some((provider, slot)) = self.actors.borrow().source_of(actor) {
                    if provider == self.provider {
                        return self.machine.entity_reference(slot);
                    }
                }
                self.borrowed
                    .borrow_mut()
                    .reference(&mut self.fields.borrow_mut(), actor)
                    .map_err(qc_error)
            }
        }
    }

    fn completed(
        &self,
        request: &qa_content::q1::foundation::gameplay::DamageRequest,
        outcome: &qa_content::q1::foundation::gameplay::DamageOutcome,
    ) {
        if let Some(pending) = self.incoming.borrow_mut().last_mut() {
            if pending.request == *request {
                pending.outcome = Some(outcome.clone());
            }
        }
    }
}

/// Convert a guest callback value into the content shape (both mirror donor `ModCallbackValue`).
///
/// Guest-only client inputs are invalid in combat calls and rejected.
fn content_callback_value(
    value: &qa_guest::qc::mod_provider::ModCallbackValue,
) -> Result<qa_content::contract::ModCallbackValue, GuestError> {
    use qa_content::contract::{ModCallbackInput as ContentInput, ModCallbackValue as Content};
    use qa_guest::qc::mod_provider::{ModCallbackInput as GuestInput, ModCallbackValue as Guest};
    match value {
        Guest::Input(input) => {
            let mapped = match input {
                GuestInput::Self_ => ContentInput::Slf,
                GuestInput::Other => ContentInput::Other,
                GuestInput::Activator => ContentInput::Activator,
                GuestInput::Attacker => ContentInput::Attacker,
                GuestInput::Inflictor => ContentInput::Inflictor,
                GuestInput::Amount => ContentInput::Amount,
                GuestInput::DamageFlags => ContentInput::DamageFlags,
                GuestInput::RegularProtectionScale => ContentInput::RegularProtectionScale,
                GuestInput::Knockback => ContentInput::Knockback,
                GuestInput::Point => ContentInput::Point,
                GuestInput::Direction => ContentInput::Direction,
                GuestInput::Normal => ContentInput::Normal,
                GuestInput::Item => ContentInput::Item,
                GuestInput::Time => ContentInput::Time,
                GuestInput::Elapsed => ContentInput::Elapsed,
                unsupported => {
                    return Err(GuestError::invalid(format!(
                        "combat source call uses unsupported input {unsupported:?}"
                    )));
                }
            };
            Ok(Content::Input(mapped))
        }
        Guest::Float(value) => Ok(Content::Float(*value)),
        Guest::String(value) => Ok(Content::Str(qa_content::contract::ModCallbackString(value.clone()))),
        Guest::Vector(value) => Ok(Content::Vector(*value)),
    }
}

/// Declared damage call for program binding derivation (donor
/// `deriveNativeBinding` `declaredDamage`; QuakeWorld requires the
/// artifact-qualified declaration).
fn declared_damage_call(
    prepared: &PreparedQuakeCSource,
) -> Result<Option<qa_content::contract::ModSourceCall>, GuestError> {
    prepared
        .combat_declaration
        .as_ref()
        .map(|combat| content_source_call(&guest_call(&combat.damage)))
        .transpose()
}

/// Convert a guest source call into the content shape.
fn content_source_call(
    call: &qa_guest::qc::mod_provider::ModSourceCall,
) -> Result<qa_content::contract::ModSourceCall, GuestError> {
    Ok(qa_content::contract::ModSourceCall {
        function: call.function.clone(),
        arguments: call
            .arguments
            .iter()
            .map(content_callback_value)
            .collect::<Result<Vec<_>, _>>()?,
        globals: call
            .globals
            .iter()
            .map(|global| -> Result<_, GuestError> {
                Ok(qa_content::contract::ModCallbackGlobal {
                    name: global.name.clone(),
                    value: content_callback_value(&global.value)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// Convert a compat armor stage into the content declaration shape.
fn content_armor_stage(
    stage: &qa_guest::qc::compatibility::QcArmorStage,
) -> Result<qa_content::contract::ModQcArmorStage, GuestError> {
    use qa_content::contract::{ModQcArmorStage, ModQcRegularScale};
    Ok(ModQcArmorStage {
        function: stage.function.clone(),
        entry: offset_u32(stage.entry)?,
        exit: offset_u32(stage.exit)?,
        target: offset_u32(stage.target)?,
        damage: offset_u32(stage.damage)?,
        saved: offset_u32(stage.saved)?,
        regular_scale: stage
            .regular_scale
            .iter()
            .map(|site| -> Result<ModQcRegularScale, GuestError> {
                Ok(ModQcRegularScale {
                    caller: site.caller.clone(),
                    statement: offset_u32(site.statement)?,
                    scale: site.scale,
                })
            })
            .collect::<Result<Vec<_>, _>>()?,
        flags: content_damage_flags(&stage.flags)?,
        statements: stage
            .statements
            .iter()
            .map(content_statement)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// Convert compat armor flags into the content shape.
fn content_damage_flags(
    flags: &qa_guest::qc::compatibility::QcArmorFlags,
) -> Result<qa_content::contract::ModQcDamageFlags, GuestError> {
    use qa_content::contract::ModQcDamageFlags;
    use qa_guest::qc::compatibility::QcArmorFlags;
    match flags {
        QcArmorFlags::None => Ok(ModQcDamageFlags::None),
        QcArmorFlags::Bits {
            word,
            no_armor,
            no_power_armor,
            no_regular_armor,
            energy,
        } => Ok(ModQcDamageFlags::Bits {
            word: offset_u32(*word)?,
            no_armor: flag_u32(*no_armor)?,
            no_power_armor: flag_u32(*no_power_armor)?,
            no_regular_armor: flag_u32(*no_regular_armor)?,
            energy: flag_u32(*energy)?,
        }),
    }
}

/// Convert a compat proof statement into the content shape.
fn content_statement(statement: &QcProofStatement) -> Result<qa_content::contract::QcStatement, GuestError> {
    let word = |value: i32| u32::try_from(value).map_err(|_| GuestError::invalid("combat statement word exceeds u32"));
    Ok(qa_content::contract::QcStatement {
        opcode: word(statement.opcode)?,
        a: word(statement.a)?,
        b: word(statement.b)?,
        c: word(statement.c)?,
    })
}

/// Convert a compat damage scale into the content declaration shape.
fn content_damage_scale(
    scale: &qa_guest::qc::compatibility::QcDamageScale,
) -> Result<qa_content::contract::ModQcDamageScale, GuestError> {
    use qa_content::contract::{ModQcDamageScale, ModQcDamageScaleKind};
    use qa_guest::qc::compatibility::QcDamageScaleKind;
    Ok(ModQcDamageScale {
        kind: scale.kind.map_or(ModQcDamageScaleKind::Multiplier, |kind| match kind {
            QcDamageScaleKind::Multiplier => ModQcDamageScaleKind::Multiplier,
            QcDamageScaleKind::Identity => ModQcDamageScaleKind::Identity,
            QcDamageScaleKind::Transform => ModQcDamageScaleKind::Transform,
        }),
        function: scale.function.clone(),
        entry: offset_u32(scale.entry)?,
        exit: offset_u32(scale.exit)?,
        damage: offset_u32(scale.damage)?,
        statements: scale
            .statements
            .iter()
            .map(content_statement)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// Convert a compat empty-armor fallback into the content declaration shape.
fn content_empty_armor(
    empty: &qa_guest::qc::compatibility::QcEmptyArmor,
) -> Result<qa_content::contract::ModQcEmptyArmor, GuestError> {
    use qa_content::contract::{ModQcEmptyArmor, ModQcEmptyArmorItem};
    let item = match empty.item.as_str() {
        "q1:item_armor1" => ModQcEmptyArmorItem::Armor1,
        "q1:item_armor2" => ModQcEmptyArmorItem::Armor2,
        _ => ModQcEmptyArmorItem::ArmorInv,
    };
    Ok(ModQcEmptyArmor {
        item,
        absorption: empty.absorption,
    })
}
