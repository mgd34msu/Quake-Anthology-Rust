//! id1 program bindings (`src/content/q1/quakec/id1-program.ts`).
//!
//! Donor provenance: `src/content/q1/quakec/id1-program.ts`
//! (`id1ProgramBinding`, `id1DamageMultiplier`,
//! `deriveNativeProgramBinding`).
//!
//! The donor memoizes derived bindings in a module-global `WeakMap`
//! keyed by program object; here the caller threads an explicit
//! [`Id1ProgramCache`] keyed by artifact digest (identical digests are
//! identical programs). Read-only consumers that only need
//! kind/attribution/attacks/environment/damage-index data use
//! [`id1_program_snapshot`], which derives fresh without a cache.

use std::collections::{HashMap, HashSet};

use crate::contract::ModSourceCall;

use super::super::foundation::gameplay::{DamageReaction, EnvironmentHazard};
use super::damage_call::{qc_damage_call_layout, standard_qc_damage_call_layout, QcDamageCallLayout};
use super::qc_view::{QcApiKind, QcMachineView, QcOpcode, QcProgramView, QcValueType};
use super::QcError;

/// Program family (donor `Id1ProgramBinding.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Id1Kind {
    /// NetQuake.
    Netquake,
    /// QuakeWorld.
    Quakeworld,
}

/// Binding attribution (donor `Id1ProgramBinding.attribution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Id1Attribution {
    /// Pinned original artifact.
    Pinned,
    /// Derived native semantics.
    Native,
}

/// Environmental damage callback context (donor
/// `EnvironmentalSite.context`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvContext {
    /// World hazard.
    World,
    /// Touch callback.
    Touch,
    /// Blocked callback.
    Blocked,
    /// Radius damage.
    Radius,
}

/// Native environmental site (donor `EnvironmentalSite.native`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeEnv {
    /// Telefrag.
    Teledeath,
    /// Exit punishment.
    Exit,
    /// Fireball.
    Fireball,
    /// Laser trap.
    Laser,
    /// Spike trap.
    Spike,
    /// Exploding barrel.
    Barrel,
}

/// Classified environmental damage site (donor `EnvironmentalSite`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentalSite {
    /// Calling function index.
    pub caller: usize,
    /// Calling function name.
    pub name: String,
    /// Damage call statement.
    pub statement: usize,
    /// Hazard kind.
    pub hazard: EnvironmentHazard,
    /// Callback context.
    pub context: EnvContext,
    /// Native classification.
    pub native: Option<NativeEnv>,
    /// Whether the attacker comes from `goalentity`.
    pub attacker_goalentity: bool,
}

/// Damage layout variant (donor `Id1ProgramBinding.damage`).
#[derive(Debug, Clone, PartialEq)]
pub enum Id1DamageKind {
    /// Pinned statement sites.
    Sites {
        /// Health store statement.
        health_store: usize,
        /// Taken-damage word.
        take: usize,
        /// Death call and reaction word.
        death: (usize, usize),
        /// Pain call and reaction word.
        pain: (usize, usize),
        /// Pinned statements.
        statements: Vec<(usize, QcOpcode, u16, u16, u16)>,
    },
    /// Derived reaction calls.
    Calls {
        /// Reaction per call statement.
        reactions: HashMap<usize, DamageReaction>,
    },
}

/// Damage function layout (donor `Id1ProgramBinding.damage`).
#[derive(Debug, Clone, PartialEq)]
pub struct Id1DamageLayout {
    /// Function index.
    pub index: usize,
    /// First statement.
    pub first_statement: i32,
    /// Frame start word.
    pub parameter_start: usize,
    /// Frame word count.
    pub local_words: usize,
    /// Damage function global.
    pub global: usize,
    /// Damage call layout.
    pub call: QcDamageCallLayout,
    /// Layout variant.
    pub kind: Id1DamageKind,
}

/// Synchronous attack layout (donor `Id1ProgramBinding.attacks`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Id1AttacksLayout {
    /// Axe function.
    pub axe: usize,
    /// Shotgun function.
    pub shotgun: usize,
    /// Super shotgun function.
    pub super_shotgun: usize,
    /// Multi-damage accumulator.
    pub add_multi: usize,
    /// Trace attack function.
    pub trace_attack: usize,
    /// Axe damage statements.
    pub axe_damage: Vec<usize>,
    /// Multi-damage caller and statement.
    pub apply_multi_damage: (usize, usize),
}

/// Program binding (donor `Id1ProgramBinding`).
#[derive(Debug, Clone, PartialEq)]
pub struct Id1ProgramBinding {
    /// Program family.
    pub kind: Id1Kind,
    /// Artifact digest.
    pub digest: String,
    /// Binding attribution.
    pub attribution: Id1Attribution,
    /// Armor inventory field.
    pub armor_field: String,
    /// Armor inventory masks (green, yellow, red).
    pub armor_masks: [i32; 3],
    /// Damage layout.
    pub damage: Id1DamageLayout,
    /// Synchronous attack layout.
    pub attacks: Option<Id1AttacksLayout>,
    /// Environmental sites.
    pub environment: Vec<EnvironmentalSite>,
}

/// Pinned NetQuake binding (donor `netquake`).
fn netquake_binding() -> Id1ProgramBinding {
    Id1ProgramBinding {
        kind: Id1Kind::Netquake,
        attribution: Id1Attribution::Pinned,
        digest: "sha256:f2619787f9aa0f057246eea1665b622b4691b5c5a800b1a46133d1fe8b771580".to_string(),
        armor_field: "items".to_string(),
        armor_masks: [8192, 16384, 32768],
        damage: Id1DamageLayout {
            call: standard_qc_damage_call_layout(117, 1580),
            index: 117,
            first_statement: 1421,
            parameter_start: 1580,
            local_words: 10,
            global: 520,
            kind: Id1DamageKind::Sites {
                health_store: 1526,
                take: 1589,
                death: (1532, 1559),
                pain: (1568, 1701),
                statements: vec![
                    (1421, QcOpcode::LoadF, 1580, 163, 1590),
                    (1442, QcOpcode::StorePF, 213, 1600, 0),
                    (1450, QcOpcode::StorePF, 1607, 1601, 0),
                    (1454, QcOpcode::StorePF, 1610, 1608, 0),
                    (1492, QcOpcode::StorePV, 1655, 1643, 0),
                    (1526, QcOpcode::StorePF, 1677, 1675, 0),
                    (1532, QcOpcode::Call2, 1559, 0, 0),
                    (1568, QcOpcode::Call2, 1701, 0, 0),
                ],
            },
        },
        attacks: Some(Id1AttacksLayout {
            axe: 163,
            shotgun: 174,
            super_shotgun: 175,
            add_multi: 171,
            trace_attack: 172,
            axe_damage: vec![3460],
            apply_multi_damage: (170, 3580),
        }),
        environment: vec![
            EnvironmentalSite {
                caller: 239,
                name: "WaterMove".to_string(),
                statement: 6446,
                hazard: EnvironmentHazard::Drown,
                context: EnvContext::World,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 239,
                name: "WaterMove".to_string(),
                statement: 6489,
                hazard: EnvironmentHazard::Lava,
                context: EnvContext::World,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 239,
                name: "WaterMove".to_string(),
                statement: 6509,
                hazard: EnvironmentHazard::Slime,
                context: EnvContext::World,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 243,
                name: "PlayerPostThink".to_string(),
                statement: 6935,
                hazard: EnvironmentHazard::Fall,
                context: EnvContext::World,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 434,
                name: "hurt_touch".to_string(),
                statement: 10462,
                hazard: EnvironmentHazard::Trigger,
                context: EnvContext::Touch,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 375,
                name: "door_blocked".to_string(),
                statement: 8690,
                hazard: EnvironmentHazard::Crush,
                context: EnvContext::Blocked,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 397,
                name: "secret_blocked".to_string(),
                statement: 9589,
                hazard: EnvironmentHazard::Crush,
                context: EnvContext::Blocked,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 448,
                name: "plat_crush".to_string(),
                statement: 10736,
                hazard: EnvironmentHazard::Crush,
                context: EnvContext::Blocked,
                native: None,
                attacker_goalentity: false,
            },
            EnvironmentalSite {
                caller: 451,
                name: "train_blocked".to_string(),
                statement: 10877,
                hazard: EnvironmentHazard::Crush,
                context: EnvContext::Blocked,
                native: None,
                attacker_goalentity: false,
            },
        ],
    }
}

/// Pinned QuakeWorld binding (donor `quakeworld`).
fn quakeworld_binding() -> Id1ProgramBinding {
    let site = |caller: usize,
                name: &str,
                statement: usize,
                hazard: EnvironmentHazard,
                context: EnvContext,
                native: Option<NativeEnv>,
                attacker_goalentity: bool| {
        EnvironmentalSite {
            caller,
            name: name.to_string(),
            statement,
            hazard,
            context,
            native,
            attacker_goalentity,
        }
    };
    Id1ProgramBinding {
        kind: Id1Kind::Quakeworld,
        attribution: Id1Attribution::Pinned,
        digest: "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830".to_string(),
        armor_field: "items".to_string(),
        armor_masks: [8192, 16384, 32768],
        damage: Id1DamageLayout {
            call: standard_qc_damage_call_layout(83, 855),
            index: 83,
            first_statement: 359,
            parameter_start: 855,
            local_words: 13,
            global: 542,
            kind: Id1DamageKind::Sites {
                health_store: 517,
                take: 864,
                death: (523, 833),
                pain: (532, 1008),
                statements: vec![
                    (359, QcOpcode::LoadF, 855, 158, 868),
                    (388, QcOpcode::StorePF, 207, 884, 0),
                    (396, QcOpcode::StorePF, 891, 885, 0),
                    (400, QcOpcode::StorePF, 894, 892, 0),
                    (439, QcOpcode::StorePV, 939, 927, 0),
                    (457, QcOpcode::StorePV, 965, 953, 0),
                    (517, QcOpcode::StorePF, 1004, 1002, 0),
                    (523, QcOpcode::Call2, 833, 0, 0),
                    (532, QcOpcode::Call2, 1008, 0, 0),
                ],
            },
        },
        attacks: Some(Id1AttacksLayout {
            axe: 134,
            shotgun: 145,
            super_shotgun: 146,
            add_multi: 141,
            trace_attack: 143,
            axe_damage: vec![2887, 2893],
            apply_multi_damage: (140, 3027),
        }),
        environment: vec![
            site(
                383,
                "tdeath_touch",
                9808,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Teledeath),
                false,
            ),
            site(
                383,
                "tdeath_touch",
                9817,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Teledeath),
                false,
            ),
            site(
                383,
                "tdeath_touch",
                9828,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Teledeath),
                false,
            ),
            site(
                383,
                "tdeath_touch",
                9836,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Teledeath),
                false,
            ),
            site(
                186,
                "changelevel_touch",
                5410,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Exit),
                false,
            ),
            site(
                431,
                "fire_touch",
                10926,
                EnvironmentHazard::Lava,
                EnvContext::Touch,
                Some(NativeEnv::Fireball),
                false,
            ),
            site(
                435,
                "Laser_Touch",
                11091,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Laser),
                false,
            ),
            site(
                158,
                "spike_touch",
                3900,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Spike),
                false,
            ),
            site(
                159,
                "superspike_touch",
                3973,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                Some(NativeEnv::Spike),
                false,
            ),
            site(
                84,
                "T_RadiusDamage",
                580,
                EnvironmentHazard::Trigger,
                EnvContext::Radius,
                Some(NativeEnv::Barrel),
                false,
            ),
            site(
                202,
                "WaterMove",
                6020,
                EnvironmentHazard::Drown,
                EnvContext::World,
                None,
                false,
            ),
            site(
                202,
                "WaterMove",
                6063,
                EnvironmentHazard::Lava,
                EnvContext::World,
                None,
                false,
            ),
            site(
                202,
                "WaterMove",
                6083,
                EnvironmentHazard::Slime,
                EnvContext::World,
                None,
                false,
            ),
            site(
                206,
                "PlayerPostThink",
                6533,
                EnvironmentHazard::Fall,
                EnvContext::World,
                None,
                false,
            ),
            site(
                393,
                "hurt_touch",
                10071,
                EnvironmentHazard::Trigger,
                EnvContext::Touch,
                None,
                false,
            ),
            site(
                335,
                "door_blocked",
                8275,
                EnvironmentHazard::Crush,
                EnvContext::Blocked,
                None,
                true,
            ),
            site(
                357,
                "secret_blocked",
                9183,
                EnvironmentHazard::Crush,
                EnvContext::Blocked,
                None,
                false,
            ),
            site(
                407,
                "plat_crush",
                10349,
                EnvironmentHazard::Crush,
                EnvContext::Blocked,
                None,
                false,
            ),
            site(
                410,
                "train_blocked",
                10492,
                EnvironmentHazard::Crush,
                EnvContext::Blocked,
                None,
                false,
            ),
        ],
    }
}

/// Pinned binding for a digest.
fn pinned_binding(digest: &str) -> Option<Id1ProgramBinding> {
    match digest {
        "sha256:f2619787f9aa0f057246eea1665b622b4691b5c5a800b1a46133d1fe8b771580" => Some(netquake_binding()),
        "sha256:ff51cb5e77360d72b93487d89198dcf94629b92f8bae100fc6ea48a6c12a7830" => Some(quakeworld_binding()),
        _ => None,
    }
}

/// Derived-binding cache (donor `derivedBindings`).
#[derive(Debug, Default)]
pub struct Id1ProgramCache {
    /// Binding per artifact digest.
    entries: HashMap<String, Id1ProgramBinding>,
}

/// Pinned annotations remain artifact-specific; other layouts require
/// original operation proofs (donor `id1ProgramBinding`).
pub fn id1_program_binding(
    cache: &mut Id1ProgramCache,
    program: &QcProgramView,
    declared_damage: Option<&ModSourceCall>,
) -> Result<Id1ProgramBinding, QcError> {
    let Some(binding) = pinned_binding(program.digest) else {
        return derive_native_binding(cache, program, declared_damage);
    };
    if let Some(declared) = declared_damage {
        let call = qc_damage_call_layout(program, Some(declared))?;
        if call.function_index != binding.damage.index || call.roles != binding.damage.call.roles {
            return Err(QcError::program(
                "Pinned QC damage declaration differs from its original ABI",
                program.source,
            ));
        }
        let cached = cache
            .entries
            .get(program.digest)
            .and_then(|binding| binding.damage.call.declaration.as_ref());
        if cached.is_some_and(|cached| cached != declared) {
            return Err(QcError::program(
                "QC program already has a different qualified combat call",
                program.source,
            ));
        }
        let mut qualified = binding.clone();
        qualified.damage.call = call;
        cache.entries.insert(program.digest.to_string(), qualified);
    }
    if (program.api == QcApiKind::Quakeworld) != (binding.kind == Id1Kind::Quakeworld) {
        return Err(QcError::program(
            "QuakeC source requires a verified classic id1 or native QuakeWorld artifact",
            program.source,
        ));
    }
    Ok(cache.entries.get(program.digest).cloned().unwrap_or(binding))
}

/// Fresh binding without cache consultation, for read-only consumers.
/// QuakeWorld programs still require their artifact-qualified damage
/// declaration (donor `deriveNativeBinding`).
pub(crate) fn id1_program_snapshot(
    program: &QcProgramView,
    declared_damage: Option<&ModSourceCall>,
) -> Result<Id1ProgramBinding, QcError> {
    let mut cache = Id1ProgramCache::default();
    id1_program_binding(&mut cache, program, declared_damage)
}

/// Source damage multiplier for the pinned programs (donor
/// `id1DamageMultiplier`; the program view is passed explicitly because
/// it borrows).
pub fn id1_damage_multiplier(
    program: &QcProgramView,
    machine: &dyn QcMachineView,
    attacker: i32,
    inflictor: i32,
) -> Result<f64, QcError> {
    // Pinned-only consumer: pinned programs skip derivation, native ones
    // reject below; QuakeWorld natives fail derivation first either way.
    let binding = id1_program_snapshot(program, None)?;
    if binding.attribution == Id1Attribution::Native {
        return Err(QcError::program(
            "Native mod damage multiplier belongs to its bytecode",
            program.source,
        ));
    }
    let field = |name: &str| -> Result<usize, QcError> {
        machine
            .field_offset(name)
            .map_err(|_| QcError::program(format!("Missing source damage field {name}"), machine.program_source()))
    };
    let attacker_slot = machine.entity_slot(attacker)?;
    if machine.entity_float(attacker_slot, field("super_damage_finished")?)?
        <= machine.global_float(machine.global_offset("time")?)?
    {
        return Ok(1.0);
    }
    if binding.kind == Id1Kind::Netquake {
        return Ok(4.0);
    }
    let inflictor_slot = machine.entity_slot(inflictor)?;
    if machine.strings_get(machine.entity_int(inflictor_slot, field("classname")?)?)? == "door" {
        return Ok(1.0);
    }
    Ok(if machine.global_float(machine.global_offset("deathmatch")?)? == 4.0 {
        8.0
    } else {
        4.0
    })
}

/// Source calls supply every argument; these types do not imply
/// defaults for host calls (donor `deriveNativeProgramBinding`).
pub fn derive_native_program_binding(
    cache: &mut Id1ProgramCache,
    program: &QcProgramView,
) -> Result<Id1ProgramBinding, QcError> {
    derive_native_binding(cache, program, None)
}

/// Derive native damage semantics (donor `deriveNativeBinding`).
fn derive_native_binding(
    cache: &mut Id1ProgramCache,
    program: &QcProgramView,
    declared_damage: Option<&ModSourceCall>,
) -> Result<Id1ProgramBinding, QcError> {
    let declared_call = declared_damage
        .map(|declared| qc_damage_call_layout(program, Some(declared)))
        .transpose()?;
    if let Some(cached) = cache.entries.get(program.digest) {
        if declared_damage.is_none() || cached.damage.call.declaration.as_ref() == declared_damage {
            return Ok(cached.clone());
        }
        if cached.damage.call.declaration.is_some() {
            return Err(QcError::program(
                "QC program already has a different qualified combat call",
                program.source,
            ));
        }
    }
    let reject = |reason: &str| -> QcError {
        QcError::program(format!("Unsupported native damage semantics: {reason}"), program.source)
    };
    if program.api == QcApiKind::Quakeworld && declared_damage.is_none() {
        return Err(reject("QuakeWorld requires an artifact-qualified combat declaration"));
    }
    let call = match declared_call {
        Some(call) => call,
        None => qc_damage_call_layout(program, None)?,
    };
    let damage = program.function_at(call.function_index)?.clone();
    let mut mutable: HashSet<usize> = HashSet::new();
    for statement in program.statements {
        let opcode = statement.opcode;
        if opcode >= QcOpcode::StoreF && opcode <= QcOpcode::StoreFn {
            let width = usize::from(opcode == QcOpcode::StoreV) * 2 + 1;
            mutable.extend(words_range(usize::from(statement.b), width));
        } else if opcode >= QcOpcode::MulF && opcode <= QcOpcode::Address
            || opcode >= QcOpcode::NotF && opcode <= QcOpcode::NotFn
            || opcode >= QcOpcode::And && opcode <= QcOpcode::BitOr
        {
            let width = usize::from(matches!(
                opcode,
                QcOpcode::MulFV | QcOpcode::MulVF | QcOpcode::AddV | QcOpcode::SubV | QcOpcode::LoadV
            )) * 2
                + 1;
            mutable.extend(words_range(usize::from(statement.c), width));
        }
    }
    let Some(definition) = program.global_named(&damage.name) else {
        return Err(reject("damage global references"));
    };
    if definition.def_type != QcValueType::Function || mutable.contains(&definition.offset) {
        return Err(reject("damage global references"));
    }
    if usize::try_from(program.initial_i32(definition.offset)?).ok() != Some(damage.index) {
        return Err(reject("damage global references"));
    }
    let global = definition.offset;
    for (name, expected) in [
        ("health", QcValueType::Float),
        ("armorvalue", QcValueType::Float),
        ("armortype", QcValueType::Float),
        ("velocity", QcValueType::Vector),
        ("th_pain", QcValueType::Function),
        ("th_die", QcValueType::Function),
    ] {
        if program.field_named(name).is_none_or(|field| field.def_type != expected) {
            return Err(reject(&format!("field {name}")));
        }
    }
    if program
        .global_named("self")
        .is_none_or(|definition| definition.def_type != QcValueType::Entity)
    {
        return Err(reject("self global"));
    }
    let armor_mask = |name: &str| -> Result<i32, QcError> {
        let Some(definition) = program.global_named(name) else {
            return Err(reject(&format!("armor inventory constant {name}")));
        };
        if definition.def_type != QcValueType::Float || mutable.contains(&definition.offset) {
            return Err(reject(&format!("armor inventory constant {name}")));
        }
        let mask = f64::from(program.initial_f32(definition.offset)?);
        if !mask.is_finite() || mask.fract() != 0.0 || !(1.0..=0x8000_0000u32 as f64).contains(&mask) {
            return Err(reject(&format!("armor inventory bit {name}")));
        }
        #[allow(clippy::cast_possible_truncation)]
        let bits = mask as i64;
        if bits & (bits - 1) != 0 {
            return Err(reject(&format!("armor inventory bit {name}")));
        }
        #[allow(clippy::cast_possible_wrap)]
        Ok(bits as i32)
    };
    let mut inventories = Vec::new();
    for field in program.fields {
        let Some(suffix) = field.name.strip_prefix("items") else {
            continue;
        };
        if field.def_type != QcValueType::Float || !suffix.chars().all(|char| char.is_ascii_digit()) {
            continue;
        }
        if [1, 2, 3]
            .iter()
            .all(|grade| program.global_named(&format!("IT{suffix}_ARMOR{grade}")).is_some())
        {
            inventories.push((field.name.clone(), suffix.to_string()));
        }
    }
    if inventories.len() != 1 {
        return Err(reject("ambiguous or missing armor inventory constants"));
    }
    let (armor_field, suffix) = inventories.remove(0);
    let prefix = format!("IT{suffix}_ARMOR");
    let armor_masks = [
        armor_mask(&format!("{prefix}1"))?,
        armor_mask(&format!("{prefix}2"))?,
        armor_mask(&format!("{prefix}3"))?,
    ];
    if HashSet::from(armor_masks).len() != armor_masks.len() {
        return Err(reject("overlapping armor inventory bits"));
    }
    let pain = program.field_named("th_pain").map(|field| field.offset);
    let die = program.field_named("th_die").map(|field| field.offset);
    let mut reactions: HashMap<usize, DamageReaction> = HashMap::new();
    for function in program.functions {
        if function.first_statement <= 0 {
            continue;
        }
        let end = program.function_end(function.first_statement);
        let first = usize::try_from(function.first_statement).unwrap_or(usize::MAX);
        for index in first..end {
            let Some(call) = program.statements.get(index) else {
                continue;
            };
            if !call.opcode.is_call() {
                continue;
            }
            let mut word = usize::from(call.a);
            let mut cursor = index as i64 - 1;
            while cursor >= first as i64 {
                let Some(statement) = program.statements.get(usize::try_from(cursor).unwrap_or(usize::MAX)) else {
                    break;
                };
                let opcode = statement.opcode;
                let a = usize::from(statement.a);
                let b = usize::from(statement.b);
                let c = usize::from(statement.c);
                if opcode == QcOpcode::LoadFn && c == word {
                    if !mutable.contains(&b)
                        && program
                            .globals
                            .iter()
                            .any(|value| value.offset == b && value.def_type == QcValueType::Field)
                    {
                        let field = usize::try_from(program.initial_i32(b)?).ok();
                        if field.is_some() && field == pain {
                            reactions.insert(index, DamageReaction::Pain);
                        } else if field.is_some() && field == die {
                            reactions.insert(index, DamageReaction::Death);
                        }
                    }
                    break;
                }
                if opcode == QcOpcode::StoreFn && b == word {
                    word = a;
                    cursor -= 1;
                    continue;
                }
                if opcode.is_call() || opcode == QcOpcode::If || opcode == QcOpcode::IfNot || opcode == QcOpcode::Goto {
                    break;
                }
                if opcode >= QcOpcode::StoreF
                    && opcode <= QcOpcode::StoreFn
                    && b <= word
                    && word < b + usize::from(opcode == QcOpcode::StoreV) * 2 + 1
                {
                    break;
                }
                if (opcode >= QcOpcode::MulF && opcode <= QcOpcode::Address
                    || opcode >= QcOpcode::NotF && opcode <= QcOpcode::NotFn
                    || opcode >= QcOpcode::And && opcode <= QcOpcode::BitOr)
                    && c <= word
                    && word < c + 3
                {
                    break;
                }
                cursor -= 1;
            }
        }
    }
    if reactions.is_empty() {
        return Err(reject("no typed damage reaction calls"));
    }
    let binding = Id1ProgramBinding {
        kind: if program.api == QcApiKind::Quakeworld {
            Id1Kind::Quakeworld
        } else {
            Id1Kind::Netquake
        },
        attribution: Id1Attribution::Native,
        digest: program.digest.to_string(),
        armor_field,
        armor_masks,
        damage: Id1DamageLayout {
            index: damage.index,
            first_statement: damage.first_statement,
            parameter_start: damage.parameter_start,
            local_words: damage.local_words,
            global,
            call,
            kind: Id1DamageKind::Calls { reactions },
        },
        attacks: None,
        environment: Vec::new(),
    };
    cache.entries.insert(program.digest.to_string(), binding.clone());
    Ok(binding)
}

/// Word range helper.
fn words_range(first: usize, width: usize) -> Vec<usize> {
    (first..first.saturating_add(width)).collect()
}
