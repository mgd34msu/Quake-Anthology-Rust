//! Primary equipment presentation: original cgame HUD/weapon-shell interfaces.
//!
//! Provenance: `src/compat/qvm/primary-presentation-profile.ts`.
//!
//! Absorbs the QVM-relevant parts of `src/contracts/held-weapon.ts`
//! ([`HeldWeaponDeclaration`], shared with item definitions via
//! [`super::mod_weapon_stage`]) plus its reader (`content/held-weapon.ts`,
//! with digest/path/grip mirrors). Local mirror:
//! [`QvmEquipmentPresentationProfile`] (`content/q3/equipment/cgame-weapon-hud.ts`).
//! Compatibility-declaration loading, mounts, and the q3 fallback belong to
//! the content/compatibility owners: the caller supplies the loaded
//! [`CompatibilityEquipment`] section (or `None`) plus a q3 fallback.

use qa_core::math::{vec3, Vec3};

use super::mod_provider::{
    qualify_qvm_region, ProfileReader, QvmArtifact, QvmOpcode, QVM_MAX_PRIVATE_ARGUMENT_WORDS, QVM_REF_ENTITY_BYTES,
};
use super::primary_player_profile::RegionRef;
use crate::error::GuestError;

// ---------------------------------------------------------------------------
// Held-weapon contract types (`src/contracts/held-weapon.ts`).
// ---------------------------------------------------------------------------

/// Model grip transform (mirror of `ModelTransform`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelGrip {
    /// Origin.
    pub origin: Vec3,
    /// Rotation axis.
    pub axis: [Vec3; 3],
    /// Scale.
    pub scale: Vec3,
}

/// Held-weapon model subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldWeaponPart {
    /// Source digests.
    pub digests: Vec<String>,
    /// Vertices.
    pub vertices: Vec<usize>,
}

/// Held-weapon model (mirror of `HeldWeaponModel`).
#[derive(Debug, Clone, PartialEq)]
pub struct HeldWeaponModel {
    /// Content digest, if any.
    pub digest: Option<String>,
    /// Resource path.
    pub path: String,
    /// Reference frame.
    pub reference_frame: usize,
    /// Grip transform.
    pub grip: ModelGrip,
    /// Fallback path, if any.
    pub fallback: Option<String>,
    /// Subset, if any.
    pub part: Option<HeldWeaponPart>,
}

/// Held-weapon declaration (mirror of `HeldWeaponDeclaration`).
#[derive(Debug, Clone, PartialEq)]
pub enum HeldWeaponDeclaration {
    /// No held weapon.
    None,
    /// Held model.
    Model(HeldWeaponModel),
}

fn is_content_digest(value: &str) -> bool {
    value.len() == 71
        && value.as_bytes()[..7] == *b"sha256:"
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn normalize_resource_path(reader: &ProfileReader<'_>, path: &str) -> Result<String, GuestError> {
    let normalized = path.replace('\\', "/");
    let bad_drive =
        normalized.len() >= 2 && normalized.as_bytes()[0].is_ascii_alphabetic() && normalized.as_bytes()[1] == b':';
    if normalized.is_empty()
        || normalized.contains('\0')
        || bad_drive
        || normalized
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return reader.fail(&format!("Invalid relative resource path: {path}"));
    }
    Ok(normalized)
}

fn dot_f32(left: Vec3, right: Vec3) -> f32 {
    left.x * right.x + (left.y * right.y + left.z * right.z)
}

fn cross_f32(left: Vec3, right: Vec3) -> Vec3 {
    vec3(
        left.y * right.z - left.z * right.y,
        left.z * right.x - left.x * right.z,
        left.x * right.y - left.y * right.x,
    )
}

/// Read a model grip (mirror of `readModelGrip`).
pub fn read_model_grip(reader: &ProfileReader<'_>) -> Result<ModelGrip, GuestError> {
    let vector = |value: &ProfileReader<'_>| -> Result<Vec3, GuestError> {
        Ok(vec3(
            value.field("x")?.finite()? as f32,
            value.field("y")?.finite()? as f32,
            value.field("z")?.finite()? as f32,
        ))
    };
    let axis = reader.field("axis")?.list(vector)?;
    if axis.len() != 3 {
        return reader.fail("Model grip requires three source axes");
    }
    let (first, second, third) = (axis[0], axis[1], axis[2]);
    if axis.iter().any(|value| (dot_f32(*value, *value) - 1.0).abs() > 0.001)
        || dot_f32(first, second).abs() > 0.001
        || dot_f32(first, third).abs() > 0.001
        || dot_f32(second, third).abs() > 0.001
        || dot_f32(cross_f32(first, second), third) < 0.999
    {
        return reader.fail("Model grip axes must form a rotation");
    }
    let scale = if reader.field("scale")?.is_undefined() {
        vec3(1.0, 1.0, 1.0)
    } else {
        vector(&reader.field("scale")?)?
    };
    if scale.x == 0.0 || scale.y == 0.0 || scale.z == 0.0 {
        return reader.fail("Model grip scale must be invertible");
    }
    Ok(ModelGrip {
        origin: vector(&reader.field("origin")?)?,
        axis: [first, second, third],
        scale,
    })
}

/// Read a held-weapon declaration (mirror of `readHeldWeaponDeclaration`).
pub fn read_held_weapon_declaration(reader: &ProfileReader<'_>) -> Result<HeldWeaponDeclaration, GuestError> {
    if reader.field("kind")?.choice(&["none", "model"])? == "none" {
        return Ok(HeldWeaponDeclaration::None);
    }
    let model = reader.field("model")?;
    let part = model.field("part")?;
    let read_digest = |value: &ProfileReader<'_>| -> Result<String, GuestError> {
        let parsed = value.string()?;
        if !is_content_digest(&parsed) {
            return value.fail("held model requires a SHA256 digest");
        }
        Ok(parsed)
    };
    let subset = if part.is_undefined() {
        None
    } else {
        Some(HeldWeaponPart {
            digests: part.field("digests")?.list(read_digest)?,
            vertices: part
                .field("vertices")?
                .list(|value| value.integer(0).map(|vertex| vertex as usize))?,
        })
    };
    if let Some(subset) = subset.as_ref() {
        let distinct: std::collections::HashSet<usize> = subset.vertices.iter().copied().collect();
        if subset.digests.is_empty() || subset.vertices.is_empty() || distinct.len() != subset.vertices.len() {
            return part.fail("held model subset requires source digests and distinct vertices");
        }
    }
    let digest = model.field("digest")?;
    let fallback = model.field("fallback")?;
    let path = model.field("path")?;
    let path_text = path.string()?;
    Ok(HeldWeaponDeclaration::Model(HeldWeaponModel {
        path: normalize_resource_path(&path, &path_text)?,
        reference_frame: model.field("referenceFrame")?.integer(0)? as usize,
        grip: read_model_grip(&model.field("grip")?)?,
        digest: if digest.is_undefined() {
            None
        } else {
            Some(read_digest(&digest)?)
        },
        fallback: if fallback.is_undefined() {
            None
        } else {
            let text = fallback.string()?;
            Some(normalize_resource_path(&fallback, &text)?)
        },
        part: subset,
    }))
}

// ---------------------------------------------------------------------------
// Equipment presentation mirror (`cgame-weapon-hud.ts`).
// ---------------------------------------------------------------------------

/// View decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresentationView {
    /// Entry instruction.
    pub entry: usize,
    /// Decision instruction.
    pub decision: usize,
    /// Taken direction.
    pub taken: bool,
}

/// Warning states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WarningStates {
    /// None state.
    pub none: i32,
    /// Low state.
    pub low: i32,
    /// Empty state.
    pub empty: i32,
}

/// Warning profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresentationWarning {
    /// Entry instruction.
    pub entry: usize,
    /// State address.
    pub state: usize,
    /// States.
    pub states: WarningStates,
}

/// Held-weapon hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresentationHeld {
    /// Entry instruction.
    pub entry: usize,
    /// Ref-entity local.
    pub gun: usize,
    /// Parent argument.
    pub parent_argument: usize,
    /// State argument.
    pub state_argument: usize,
    /// Entity argument.
    pub entity_argument: usize,
    /// Entity-number offset.
    pub entity_number_offset: usize,
}

/// Status region entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRegion {
    /// Entry instruction.
    pub entry: usize,
    /// Decision instruction.
    pub decision: usize,
    /// Taken direction.
    pub taken: bool,
    /// Ammo regions.
    pub ammo: Vec<RegionRef>,
}

/// Status profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationStatus {
    /// Function entries.
    Functions {
        /// Entries.
        entries: Vec<usize>,
    },
    /// Region entries.
    Regions {
        /// Entries.
        entries: Vec<StatusRegion>,
    },
}

/// Equipment presentation profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmEquipmentPresentationProfile {
    /// HUD entry.
    pub hud: usize,
    /// View decision.
    pub view: PresentationView,
    /// Warning profile.
    pub warning: PresentationWarning,
    /// Held-weapon hook.
    pub held: PresentationHeld,
    /// Status profile.
    pub status: PresentationStatus,
}

/// Loaded compatibility equipment section.
pub struct CompatibilityEquipment<'a> {
    /// ABI profile name.
    pub profile: &'a str,
    /// Section reader.
    pub reader: ProfileReader<'a>,
}

/// Read equipment presentation, falling back to q3 content when undeclared.
pub fn read_qvm_equipment_presentation(
    artifact: &QvmArtifact,
    compatibility: Option<CompatibilityEquipment<'_>>,
    q3_fallback: &dyn Fn(&QvmArtifact) -> Option<QvmEquipmentPresentationProfile>,
) -> Result<Option<QvmEquipmentPresentationProfile>, GuestError> {
    use super::mod_provider::QvmRole;
    let Some(section) = compatibility else {
        return Ok(q3_fallback(artifact));
    };
    if artifact.role != QvmRole::Cgame || section.profile != artifact.abi().name() {
        return section
            .reader
            .fail("equipment presentation requires the declared cgame ABI");
    }
    let reader = &section.reader;
    let instructions = &artifact.image.instructions;
    let read_entry = |value: &ProfileReader<'_>| -> Result<usize, GuestError> {
        let pc = value.integer(0)? as usize;
        if instructions
            .get(pc)
            .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            return value.fail("not an original function entry");
        }
        Ok(pc)
    };
    let read_decision = |value: &ProfileReader<'_>, owner: usize| -> Result<usize, GuestError> {
        let pc = value.integer(0)? as usize;
        let mut function_entry = pc as i64;
        while function_entry >= 0
            && instructions
                .get(function_entry as usize)
                .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            function_entry -= 1;
        }
        if function_entry != owner as i64
            || instructions
                .get(pc)
                .is_none_or(|instruction| !instruction.opcode.is_branch())
        {
            return value.fail("visibility decision is outside its original function");
        }
        Ok(pc)
    };
    let read_argument = |value: &ProfileReader<'_>| -> Result<usize, GuestError> {
        let index = value.integer(0)? as usize;
        if index >= QVM_MAX_PRIVATE_ARGUMENT_WORDS {
            return value.fail("source argument exceeds the private invocation extent");
        }
        Ok(index)
    };
    let read_word = |value: &ProfileReader<'_>, bytes: usize| -> Result<usize, GuestError> {
        let offset = value.integer(0)? as usize;
        if !offset.is_multiple_of(4) || offset + 4 > bytes {
            return value.fail("source word exceeds its record or is unaligned");
        }
        Ok(offset)
    };
    let read_int32 = |value: &ProfileReader<'_>| -> Result<i32, GuestError> {
        let result = value.integer(i64::from(i32::MIN))?;
        if result > i64::from(i32::MAX) {
            return value.fail("expected original int32");
        }
        Ok(result as i32)
    };
    let view = reader.field("view")?;
    let view_entry = read_entry(&view.field("entry")?)?;
    let warning = reader.field("warning")?;
    let states = warning.field("states")?;
    let held = reader.field("held")?;
    let held_entry = read_entry(&held.field("entry")?)?;
    let instruction = instructions.get(held_entry);
    if instruction.is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter) {
        return held.fail("held weapon entry disappeared");
    }
    let frame = instruction.map_or(0, |instruction| instruction.operand.max(0) as usize);
    let gun = read_word(&held.field("gun")?, frame)?;
    if gun < 8 || gun + QVM_REF_ENTITY_BYTES > frame {
        return held.fail("held weapon refEntity exceeds the original local frame");
    }
    let status = reader.field("status")?;
    let kind = status.field("kind")?.choice(&["regions", "functions"])?;
    let data_bytes = artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length;
    let profile = QvmEquipmentPresentationProfile {
        hud: read_entry(&reader.field("hud")?)?,
        view: PresentationView {
            entry: view_entry,
            decision: read_decision(&view.field("decision")?, view_entry)?,
            taken: view.field("taken")?.boolean()?,
        },
        warning: PresentationWarning {
            entry: read_entry(&warning.field("entry")?)?,
            state: read_word(&warning.field("state")?, data_bytes)?,
            states: WarningStates {
                none: read_int32(&states.field("none")?)?,
                low: read_int32(&states.field("low")?)?,
                empty: read_int32(&states.field("empty")?)?,
            },
        },
        held: PresentationHeld {
            entry: held_entry,
            gun,
            parent_argument: read_argument(&held.field("parentArgument")?)?,
            state_argument: read_argument(&held.field("stateArgument")?)?,
            entity_argument: read_argument(&held.field("entityArgument")?)?,
            entity_number_offset: read_word(&held.field("entityNumberOffset")?, artifact.image.allocated_data_length)?,
        },
        status: if kind == "functions" {
            PresentationStatus::Functions {
                entries: status.field("entries")?.list(read_entry)?,
            }
        } else {
            PresentationStatus::Regions {
                entries: status.field("entries")?.list(|value| {
                    let owner = read_entry(&value.field("entry")?)?;
                    Ok(StatusRegion {
                        entry: owner,
                        decision: read_decision(&value.field("decision")?, owner)?,
                        taken: value.field("taken")?.boolean()?,
                        ammo: value.field("ammo")?.list(|region| {
                            let start = region.field("entry")?.integer(0)? as usize;
                            let join = region.field("join")?.integer(0)? as usize;
                            qualify_qvm_region(instructions, owner, start, join)?;
                            Ok(RegionRef { entry: start, join })
                        })?,
                    })
                })?,
            }
        },
    };
    let mut entries = vec![
        profile.hud,
        profile.warning.entry,
        profile.held.entry,
        profile.view.entry,
    ];
    match &profile.status {
        PresentationStatus::Functions { entries: status } => entries.extend(status.iter().copied()),
        PresentationStatus::Regions { entries: status } => entries.extend(status.iter().map(|value| value.entry)),
    }
    let status_len = match &profile.status {
        PresentationStatus::Functions { entries } => entries.len(),
        PresentationStatus::Regions { entries } => entries.len(),
    };
    if entries.iter().collect::<std::collections::HashSet<_>>().len() != entries.len() || status_len == 0 {
        return reader.fail("equipment interfaces overlap original function ownership or omit status");
    }
    Ok(Some(profile))
}

#[cfg(test)]
mod tests {
    use super::super::mod_provider::{ModuleId, ProfileValue, QvmImage, QvmInstruction, QvmRole};
    use super::*;

    fn fixture_artifact() -> QvmArtifact {
        QvmArtifact {
            module: ModuleId {
                id: "test:cgame".to_string(),
                artifact_path: "vm/cgame.qvm".to_string(),
                digest: "sha256:cgame".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Cgame,
            abi_profile: None,
            image: QvmImage {
                instructions: vec![
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 256),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpEq, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 64),
                    QvmInstruction::word(QvmOpcode::OpEq, 0),
                    QvmInstruction::word(QvmOpcode::OpConst, 1),
                    QvmInstruction::word(QvmOpcode::OpPop, 0),
                    QvmInstruction::word(QvmOpcode::OpLeave, 0),
                    QvmInstruction::word(QvmOpcode::OpEnter, 0),
                ],
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 8192,
            },
        }
    }

    fn make_declaration() -> ProfileValue {
        ProfileValue::record(vec![
            ("hud", ProfileValue::Int(0)),
            (
                "view",
                ProfileValue::record(vec![
                    ("entry", ProfileValue::Int(6)),
                    ("decision", ProfileValue::Int(7)),
                    ("taken", ProfileValue::Bool(true)),
                ]),
            ),
            (
                "warning",
                ProfileValue::record(vec![
                    ("entry", ProfileValue::Int(2)),
                    ("state", ProfileValue::Int(100)),
                    (
                        "states",
                        ProfileValue::record(vec![
                            ("none", ProfileValue::Int(0)),
                            ("low", ProfileValue::Int(1)),
                            ("empty", ProfileValue::Int(2)),
                        ]),
                    ),
                ]),
            ),
            (
                "held",
                ProfileValue::record(vec![
                    ("entry", ProfileValue::Int(4)),
                    ("gun", ProfileValue::Int(8)),
                    ("parentArgument", ProfileValue::Int(0)),
                    ("stateArgument", ProfileValue::Int(1)),
                    ("entityArgument", ProfileValue::Int(2)),
                    ("entityNumberOffset", ProfileValue::Int(200)),
                ]),
            ),
            (
                "status",
                ProfileValue::record(vec![
                    ("kind", ProfileValue::Str("regions".to_string())),
                    (
                        "entries",
                        ProfileValue::Array(vec![ProfileValue::record(vec![
                            ("entry", ProfileValue::Int(10)),
                            ("decision", ProfileValue::Int(11)),
                            ("taken", ProfileValue::Bool(false)),
                            (
                                "ammo",
                                ProfileValue::Array(vec![ProfileValue::record(vec![
                                    ("entry", ProfileValue::Int(12)),
                                    ("join", ProfileValue::Int(14)),
                                ])]),
                            ),
                        ])]),
                    ),
                ]),
            ),
        ])
    }

    #[test]
    fn equipment_presentation_reads_regions() {
        let declaration = make_declaration();
        let artifact = fixture_artifact();
        let section = CompatibilityEquipment {
            profile: "q3-modern",
            reader: ProfileReader::new(&declaration),
        };
        let profile = read_qvm_equipment_presentation(&artifact, Some(section), &|_| None)
            .unwrap()
            .unwrap();
        assert_eq!(profile.hud, 0);
        assert_eq!(profile.held.gun, 8);
        assert!(matches!(profile.status, PresentationStatus::Regions { .. }));
        let mut duplicated = make_declaration();
        if let ProfileValue::Record(fields) = &mut duplicated {
            for (name, value) in fields.iter_mut() {
                if name == "warning" {
                    if let ProfileValue::Record(warning) = value {
                        for (key, entry) in warning.iter_mut() {
                            if key == "entry" {
                                *entry = ProfileValue::Int(0);
                            }
                        }
                    }
                }
            }
        }
        let section = CompatibilityEquipment {
            profile: "q3-modern",
            reader: ProfileReader::new(&duplicated),
        };
        assert!(read_qvm_equipment_presentation(&artifact, Some(section), &|_| None).is_err());
    }

    #[test]
    fn missing_section_uses_q3_fallback() {
        let artifact = fixture_artifact();
        assert!(read_qvm_equipment_presentation(&artifact, None, &|_| None)
            .unwrap()
            .is_none());
        let mut gameplay = fixture_artifact();
        gameplay.role = QvmRole::Qagame;
        let declaration = make_declaration();
        let section = CompatibilityEquipment {
            profile: "q3-modern",
            reader: ProfileReader::new(&declaration),
        };
        assert!(read_qvm_equipment_presentation(&gameplay, Some(section), &|_| None).is_err());
    }

    fn grip_value() -> ProfileValue {
        let vector = |x: f64, y: f64, z: f64| {
            ProfileValue::record(vec![
                ("x", ProfileValue::Float(x)),
                ("y", ProfileValue::Float(y)),
                ("z", ProfileValue::Float(z)),
            ])
        };
        ProfileValue::record(vec![
            ("origin", vector(1.0, 2.0, 3.0)),
            (
                "axis",
                ProfileValue::Array(vec![
                    vector(1.0, 0.0, 0.0),
                    vector(0.0, 1.0, 0.0),
                    vector(0.0, 0.0, 1.0),
                ]),
            ),
        ])
    }

    #[test]
    fn held_weapons_read_models_and_digests() {
        let none = ProfileValue::record(vec![("kind", ProfileValue::Str("none".to_string()))]);
        assert!(matches!(
            read_held_weapon_declaration(&ProfileReader::new(&none)).unwrap(),
            HeldWeaponDeclaration::None
        ));
        let model = ProfileValue::record(vec![
            ("kind", ProfileValue::Str("model".to_string())),
            (
                "model",
                ProfileValue::record(vec![
                    ("path", ProfileValue::Str("models/weapons/mg.md3".to_string())),
                    ("referenceFrame", ProfileValue::Int(2)),
                    ("grip", grip_value()),
                ]),
            ),
        ]);
        let held = read_held_weapon_declaration(&ProfileReader::new(&model)).unwrap();
        assert!(matches!(held, HeldWeaponDeclaration::Model(_)));
        let bad_digest = ProfileValue::record(vec![
            ("kind", ProfileValue::Str("model".to_string())),
            (
                "model",
                ProfileValue::record(vec![
                    ("path", ProfileValue::Str("models/weapons/mg.md3".to_string())),
                    ("referenceFrame", ProfileValue::Int(2)),
                    ("grip", grip_value()),
                    ("digest", ProfileValue::Str("nope".to_string())),
                ]),
            ),
        ]);
        assert!(read_held_weapon_declaration(&ProfileReader::new(&bad_digest)).is_err());
        let bad_path = ProfileValue::record(vec![
            ("kind", ProfileValue::Str("model".to_string())),
            (
                "model",
                ProfileValue::record(vec![
                    ("path", ProfileValue::Str("../escape.md3".to_string())),
                    ("referenceFrame", ProfileValue::Int(2)),
                    ("grip", grip_value()),
                ]),
            ),
        ]);
        assert!(read_held_weapon_declaration(&ProfileReader::new(&bad_path)).is_err());
    }
}
