//! QVM weapon behavior profiles: exact-artifact projectile declarations.
//!
//! Provenance: `src/compat/qvm/weapon-behavior-profile.ts`.
//!
//! Absorbs the QVM-relevant parts of `src/contracts/weapon-behavior.ts`
//! (definitions, callbacks, [`QvmWeaponBehaviorLayout`],
//! [`same_weapon_behavior`], [`same_qvm_weapon_layout`]).
//! `src/contracts/native-weapon-behavior.ts` declares native
//! (`windows-x86-64`) artifacts with no QVM-relevant content; only the
//! equality projection of its ABI call shape is mirrored here for
//! [`same_weapon_behavior`], and the native declaration itself stays with the
//! native-weapon worker. Runtime stepping (`WeaponBehaviorInstance`) is
//! interpreter-owned; this port covers declarations, reads, and validation.

use std::collections::HashSet;

use super::mod_provider::{
    qvm_shared_entity_bytes, ModuleId, ProfileReader, ProfileValue, QvmAbi, QvmArtifact, QvmOpcode,
};
use crate::error::GuestError;

/// Projectile role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponBehaviorRole {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Nail.
    Nail,
    /// Bolt.
    Bolt,
    /// Plasma.
    Plasma,
    /// Energy.
    Energy,
    /// Grapple.
    Grapple,
}

impl WeaponBehaviorRole {
    /// Declaration name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rocket => "rocket",
            Self::Grenade => "grenade",
            Self::Nail => "nail",
            Self::Bolt => "bolt",
            Self::Plasma => "plasma",
            Self::Energy => "energy",
            Self::Grapple => "grapple",
        }
    }

    /// Parse a declaration name.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "rocket" => Some(Self::Rocket),
            "grenade" => Some(Self::Grenade),
            "nail" => Some(Self::Nail),
            "bolt" => Some(Self::Bolt),
            "plasma" => Some(Self::Plasma),
            "energy" => Some(Self::Energy),
            "grapple" => Some(Self::Grapple),
            _ => None,
        }
    }
}

/// Equality projection of a native ABI call shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NativeArtifactAbi {
    /// ABI kind.
    pub kind: String,
    /// Call convention.
    pub call: String,
    /// Image format.
    pub image: String,
    /// Pointer bytes.
    pub pointer_bytes: u32,
}

/// Weapon behavior callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponBehaviorCallback {
    /// QuakeC function.
    QuakeC {
        /// Module identity.
        module: ModuleId,
        /// Function index.
        function_index: usize,
    },
    /// QVM instruction.
    Qvm {
        /// Module identity.
        module: ModuleId,
        /// Instruction index.
        instruction_index: usize,
    },
    /// Native artifact offset.
    NativeArtifact {
        /// Module identity.
        module: ModuleId,
        /// Image offset.
        image_offset: usize,
        /// ABI.
        abi: NativeArtifactAbi,
    },
}

/// Weapon behavior definition (mirror of `WeaponBehaviorDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorDefinition {
    /// Behavior id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Role.
    pub role: WeaponBehaviorRole,
    /// Aspect (`trajectory`).
    pub aspect: String,
    /// Module identity.
    pub module: ModuleId,
    /// Fire callback.
    pub fire: WeaponBehaviorCallback,
    /// Activation callback, if any.
    pub activate: Option<WeaponBehaviorCallback>,
}

fn same_callback(left: Option<&WeaponBehaviorCallback>, right: Option<&WeaponBehaviorCallback>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
        (Some(left), Some(right)) => {
            let left_module = match left {
                WeaponBehaviorCallback::QuakeC { module, .. }
                | WeaponBehaviorCallback::Qvm { module, .. }
                | WeaponBehaviorCallback::NativeArtifact { module, .. } => module,
            };
            let right_module = match right {
                WeaponBehaviorCallback::QuakeC { module, .. }
                | WeaponBehaviorCallback::Qvm { module, .. }
                | WeaponBehaviorCallback::NativeArtifact { module, .. } => module,
            };
            if !left_module.same_module(right_module) {
                return false;
            }
            match (left, right) {
                (
                    WeaponBehaviorCallback::QuakeC {
                        function_index: left, ..
                    },
                    WeaponBehaviorCallback::QuakeC {
                        function_index: right, ..
                    },
                ) => left == right,
                (
                    WeaponBehaviorCallback::Qvm {
                        instruction_index: left,
                        ..
                    },
                    WeaponBehaviorCallback::Qvm {
                        instruction_index: right,
                        ..
                    },
                ) => left == right,
                (
                    WeaponBehaviorCallback::NativeArtifact {
                        image_offset: left,
                        abi: left_abi,
                        ..
                    },
                    WeaponBehaviorCallback::NativeArtifact {
                        image_offset: right,
                        abi: right_abi,
                        ..
                    },
                ) => left == right && left_abi == right_abi,
                _ => false,
            }
        }
    }
}

/// Whether two behavior definitions are identical.
#[must_use]
pub fn same_weapon_behavior(left: &WeaponBehaviorDefinition, right: &WeaponBehaviorDefinition) -> bool {
    left.id == right.id
        && left.role == right.role
        && left.aspect == right.aspect
        && left.module.same_module(&right.module)
        && same_callback(Some(&left.fire), Some(&right.fire))
        && same_callback(left.activate.as_ref(), right.activate.as_ref())
}

/// Private entity fields of a weapon layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeaponLayoutFields {
    /// In-use offset.
    pub inuse: usize,
    /// Next-think offset.
    pub nextthink: usize,
    /// Think offset.
    pub think: usize,
    /// Health offset.
    pub health: usize,
}

/// Fire ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FireAbi {
    /// Entity pointer, start, direction.
    EntityPointerStartDirection,
}

/// Author-declared private layout, valid only for the exact QVM artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QvmWeaponBehaviorLayout {
    /// Entity stride.
    pub entity_stride: usize,
    /// Level clock address.
    pub level_time: usize,
    /// Allocate entry.
    pub allocate: usize,
    /// Free entry.
    pub free: usize,
    /// Private fields.
    pub fields: WeaponLayoutFields,
    /// Fire ABI.
    pub fire_abi: FireAbi,
}

/// Whether two weapon layouts share their identity.
#[must_use]
pub fn same_qvm_weapon_layout(left: &QvmWeaponBehaviorLayout, right: &QvmWeaponBehaviorLayout) -> bool {
    left.entity_stride == right.entity_stride
        && left.level_time == right.level_time
        && left.allocate == right.allocate
        && left.free == right.free
        && left.fields.inuse == right.fields.inuse
        && left.fields.nextthink == right.fields.nextthink
        && left.fields.think == right.fields.think
        && left.fields.health == right.fields.health
        && left.fire_abi == right.fire_abi
}

/// QVM weapon profile: layout plus behavior definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmWeaponProfile {
    /// Behavior definition.
    pub definition: WeaponBehaviorDefinition,
    /// Private layout.
    pub layout: QvmWeaponBehaviorLayout,
}

/// Serialize a profile to its declaration value.
pub fn qvm_weapon_profile_declaration(
    profile: &QvmWeaponProfile,
    abi_profile: QvmAbi,
) -> Result<ProfileValue, GuestError> {
    let definition = &profile.definition;
    let fire = match &definition.fire {
        WeaponBehaviorCallback::Qvm { instruction_index, .. } => *instruction_index,
        _ => return Err(GuestError::invalid("Invalid QVM weapon callback identity")),
    };
    let activation = match definition.activate.as_ref() {
        None => None,
        Some(WeaponBehaviorCallback::Qvm { instruction_index, .. }) => Some(*instruction_index),
        Some(_) => return Err(GuestError::invalid("Invalid QVM weapon callback identity")),
    };
    if !definition.id.starts_with("qvm:") {
        return Err(GuestError::invalid("Invalid QVM weapon callback identity"));
    }
    Ok(ProfileValue::record(vec![
        ("version", ProfileValue::Int(1)),
        ("artifactDigest", ProfileValue::Str(definition.module.digest.clone())),
        (
            "artifactPath",
            ProfileValue::Str(definition.module.artifact_path.clone()),
        ),
        ("abiProfile", ProfileValue::Str(abi_profile.name().to_string())),
        ("id", ProfileValue::Str(definition.id[4..].to_string())),
        ("title", ProfileValue::Str(definition.title.clone())),
        ("role", ProfileValue::Str(definition.role.name().to_string())),
        ("aspect", ProfileValue::Str(definition.aspect.clone())),
        ("fireFunction", ProfileValue::Int(fire as i64)),
        (
            "activationFunction",
            activation.map_or(ProfileValue::Null, |index| ProfileValue::Int(index as i64)),
        ),
        ("entityStride", ProfileValue::Int(profile.layout.entity_stride as i64)),
        ("levelTime", ProfileValue::Int(profile.layout.level_time as i64)),
        ("allocateFunction", ProfileValue::Int(profile.layout.allocate as i64)),
        ("freeFunction", ProfileValue::Int(profile.layout.free as i64)),
        (
            "fields",
            ProfileValue::record(vec![
                ("inuse", ProfileValue::Int(profile.layout.fields.inuse as i64)),
                ("nextthink", ProfileValue::Int(profile.layout.fields.nextthink as i64)),
                ("think", ProfileValue::Int(profile.layout.fields.think as i64)),
                ("health", ProfileValue::Int(profile.layout.fields.health as i64)),
            ]),
        ),
        (
            "fireAbi",
            ProfileValue::Str("entity-pointer-start-direction".to_string()),
        ),
    ]))
}

fn stable_behavior_id(id: &str) -> bool {
    let mut bytes = id.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-')
}

/// Read a weapon profile from its declaration value.
pub fn read_qvm_weapon_profile(value: &ProfileValue, artifact: &QvmArtifact) -> Result<QvmWeaponProfile, GuestError> {
    use super::mod_provider::QvmRole;
    let reader = ProfileReader::new(value);
    reader.field("version")?.literal_int(1)?;
    reader.field("artifactDigest")?.literal_str(&artifact.module.digest)?;
    reader
        .field("artifactPath")?
        .literal_str(&artifact.module.artifact_path)?;
    reader.field("abiProfile")?.literal_str(artifact.abi().name())?;
    if artifact.role != QvmRole::Qagame {
        return reader.fail("weapon behavior requires a qagame artifact");
    }
    let read_entry = |at: &ProfileReader<'_>| -> Result<usize, GuestError> {
        let index = at.integer(1)? as usize;
        if artifact
            .image
            .instruction(index)
            .is_none_or(|instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            return at.fail("callback is not a QVM function entry");
        }
        Ok(index)
    };
    let id = reader.field("id")?.string()?;
    if !stable_behavior_id(&id) {
        return reader.field("id")?.fail("expected a stable behavior identifier");
    }
    let role = reader
        .field("role")?
        .choice(&["rocket", "grenade", "nail", "bolt", "plasma", "energy", "grapple"])?;
    let activation = reader.field("activationFunction")?;
    let definition = WeaponBehaviorDefinition {
        id: format!("qvm:{id}"),
        title: reader.field("title")?.string()?,
        module: artifact.module.clone(),
        role: WeaponBehaviorRole::parse(&role).ok_or_else(|| GuestError::invalid("unknown weapon role"))?,
        aspect: reader.field("aspect")?.literal_str("trajectory")?,
        fire: WeaponBehaviorCallback::Qvm {
            module: artifact.module.clone(),
            instruction_index: read_entry(&reader.field("fireFunction")?)?,
        },
        activate: match activation.value() {
            ProfileValue::Null | ProfileValue::Undefined => None,
            _ => Some(WeaponBehaviorCallback::Qvm {
                module: artifact.module.clone(),
                instruction_index: read_entry(&activation)?,
            }),
        },
    };
    let minimum = qvm_shared_entity_bytes(artifact.abi());
    let entity_stride = reader.field("entityStride")?.integer(minimum as i64)? as usize;
    if entity_stride % 4 != 0 || entity_stride > artifact.image.allocated_data_length {
        return reader.field("entityStride")?.fail("invalid entity stride");
    }
    let mut occupied = HashSet::new();
    let fields = reader.field("fields")?;
    let mut field = |name: &str| -> Result<usize, GuestError> {
        let source = fields.field(name)?;
        let offset = source.integer(minimum as i64)? as usize;
        if offset % 4 != 0 || offset > entity_stride - 4 || !occupied.insert(offset) {
            return source.fail("private field is overlapping, unaligned or outside the entity");
        }
        Ok(offset)
    };
    let level_time = reader.field("levelTime")?.integer(4)? as usize;
    if level_time % 4 != 0
        || level_time > artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length - 4
    {
        return reader
            .field("levelTime")?
            .fail("level clock is outside declared guest data");
    }
    Ok(QvmWeaponProfile {
        definition,
        layout: QvmWeaponBehaviorLayout {
            entity_stride,
            level_time,
            allocate: read_entry(&reader.field("allocateFunction")?)?,
            free: read_entry(&reader.field("freeFunction")?)?,
            fields: WeaponLayoutFields {
                inuse: field("inuse")?,
                nextthink: field("nextthink")?,
                think: field("think")?,
                health: field("health")?,
            },
            fire_abi: {
                reader.field("fireAbi")?.literal_str("entity-pointer-start-direction")?;
                FireAbi::EntityPointerStartDirection
            },
        },
    })
}

/// Validate a profile against its artifact through a declaration round trip.
pub fn validate_qvm_weapon_profile(
    profile: &QvmWeaponProfile,
    artifact: &QvmArtifact,
) -> Result<QvmWeaponProfile, GuestError> {
    let validated = read_qvm_weapon_profile(&qvm_weapon_profile_declaration(profile, artifact.abi())?, artifact)?;
    if !same_weapon_behavior(&profile.definition, &validated.definition) {
        return Err(GuestError::invalid(
            "QVM weapon callback identity differs from the loaded artifact",
        ));
    }
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::super::mod_provider::{
        ModuleId, ProfileValue, QvmAbi, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole,
    };
    use super::*;

    fn module() -> ModuleId {
        ModuleId {
            id: "test:game".to_string(),
            artifact_path: "vm/qagame.qvm".to_string(),
            digest: "sha256:game".to_string(),
            revision: "1".to_string(),
        }
    }

    fn enters(count: usize) -> Vec<QvmInstruction> {
        let mut instructions = Vec::new();
        for _ in 0..count {
            instructions.push(QvmInstruction::word(QvmOpcode::OpEnter, 0));
            instructions.push(QvmInstruction::word(QvmOpcode::OpLeave, 0));
        }
        instructions
    }

    fn artifact() -> QvmArtifact {
        QvmArtifact {
            module: module(),
            role: QvmRole::Qagame,
            abi_profile: None,
            image: QvmImage {
                instructions: enters(8),
                data_length: 4096,
                literal_length: 0,
                bss_length: 0,
                initialized_length: 4096,
                allocated_data_length: 8192,
            },
        }
    }

    fn profile() -> QvmWeaponProfile {
        QvmWeaponProfile {
            definition: WeaponBehaviorDefinition {
                id: "qvm:rocket".to_string(),
                title: "Rocket".to_string(),
                role: WeaponBehaviorRole::Rocket,
                aspect: "trajectory".to_string(),
                module: module(),
                fire: WeaponBehaviorCallback::Qvm {
                    module: module(),
                    instruction_index: 2,
                },
                activate: None,
            },
            layout: QvmWeaponBehaviorLayout {
                entity_stride: 1024,
                level_time: 100,
                allocate: 2,
                free: 2,
                fields: WeaponLayoutFields {
                    inuse: 600,
                    nextthink: 604,
                    think: 608,
                    health: 612,
                },
                fire_abi: FireAbi::EntityPointerStartDirection,
            },
        }
    }

    #[test]
    fn reads_a_valid_profile() {
        let profile = profile();
        let declaration = qvm_weapon_profile_declaration(&profile, QvmAbi::Modern).expect("declare");
        let read = read_qvm_weapon_profile(&ProfileReader::new(&declaration), &artifact()).expect("read");
        assert_eq!(read.definition.id, "qvm:rocket");
        assert_eq!(read.layout.entity_stride, 1024);
        assert_eq!(read.layout.fields.health, 612);
        assert!(same_weapon_behavior(&profile.definition, &read.definition));
        assert!(same_qvm_weapon_layout(&profile.layout, &read.layout));
        assert_eq!(
            validate_qvm_weapon_profile(&profile, &artifact())
                .expect("validate")
                .definition
                .id,
            "qvm:rocket"
        );
    }

    #[test]
    fn same_qvm_weapon_layout_compares_layouts() {
        let left = profile().layout;
        assert!(same_qvm_weapon_layout(&left, &left));
        let mut drifted = left;
        drifted.entity_stride = 2048;
        assert!(!same_qvm_weapon_layout(&left, &drifted));
        drifted = left;
        drifted.level_time = 104;
        assert!(!same_qvm_weapon_layout(&left, &drifted));
        drifted = left;
        drifted.fields.think = 616;
        assert!(!same_qvm_weapon_layout(&left, &drifted));
    }

    #[test]
    fn validate_rejects_foreign_digest() {
        let mut foreign = artifact();
        foreign.module.digest = "sha256:other".to_string();
        assert!(validate_qvm_weapon_profile(&profile(), &foreign).is_err());
    }

    #[test]
    fn rejects_unstable_identifiers_and_entries() {
        let mut bad_id = profile();
        bad_id.definition.id = "qvm:Bad-Id!".to_string();
        let declaration = qvm_weapon_profile_declaration(&bad_id, QvmAbi::Modern).expect("declare");
        assert!(read_qvm_weapon_profile(&ProfileReader::new(&declaration), &artifact()).is_err());
        let mut bad_entry = artifact();
        bad_entry.image.instructions[2] = QvmInstruction::word(QvmOpcode::OpConst, 0);
        let declaration = qvm_weapon_profile_declaration(&profile(), QvmAbi::Modern).expect("declare");
        assert!(read_qvm_weapon_profile(&ProfileReader::new(&declaration), &bad_entry).is_err());
    }

    #[test]
    fn same_weapon_behavior_compares_callbacks() {
        let left = profile().definition;
        assert!(same_weapon_behavior(&left, &left));
        let mut activated = left.clone();
        activated.activate = Some(WeaponBehaviorCallback::Qvm {
            module: module(),
            instruction_index: 4,
        });
        assert!(!same_weapon_behavior(&left, &activated));
        let native = |offset: usize| WeaponBehaviorCallback::NativeArtifact {
            module: module(),
            image_offset: offset,
            abi: NativeArtifactAbi {
                kind: "windows-x86-64".to_string(),
                call: "system".to_string(),
                image: "pe".to_string(),
                pointer_bytes: 8,
            },
        };
        let mut left_native = left.clone();
        left_native.fire = native(16);
        let mut right_native = left.clone();
        right_native.fire = native(16);
        assert!(same_weapon_behavior(&left_native, &right_native));
        right_native.fire = native(24);
        assert!(!same_weapon_behavior(&left_native, &right_native));
    }
}
