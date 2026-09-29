//! Native weapon behavior persistence ported from `src/persistence/native-weapon.ts`.
//!
//! Structural port of the `q2-api2023-trajectory` declaration plus
//! [`same_weapon_behavior`] (donor `src/contracts/weapon-behavior.ts`).
//! Identity keys (artifact, id, title, role, aspect, call RVAs, class
//! list) are validated and the remaining loader-owned layout tables are
//! retained verbatim; the compat rerelease loader owns layout
//! cross-checks against the mounted executable, and the caller supplies
//! the built-in legacy declaration old saves omit.

use std::collections::HashSet;

use qa_guest::checkpoint::{
    read_module, read_native_call_abi, write_module, write_native_call_abi, ModuleIdentity, NativeCallAbi,
};
use qa_world::save::shared::validate_digest;
use qa_world::save::source_items::normalize_resource_path;
use qa_world::save::value::{int, namespaced, obj, str, SaveJson, SaveReader};

use super::PersistenceError;

/// Weapon behavior callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponBehaviorCallback {
    /// QuakeC function.
    Quakec {
        /// Module.
        module: ModuleIdentity,
        /// Function index.
        function_index: u64,
    },
    /// QVM instruction.
    Qvm {
        /// Module.
        module: ModuleIdentity,
        /// Instruction index.
        instruction_index: u64,
    },
    /// Native artifact offset.
    NativeArtifact {
        /// Module.
        module: ModuleIdentity,
        /// Image offset.
        image_offset: i128,
        /// ABI.
        abi: NativeCallAbi,
    },
}

/// Weapon behavior definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehaviorDefinition {
    /// Id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Module.
    pub module: ModuleIdentity,
    /// Role.
    pub role: String,
    /// Aspect.
    pub aspect: String,
    /// Activate callback.
    pub activate: Option<WeaponBehaviorCallback>,
    /// Fire callback.
    pub fire: WeaponBehaviorCallback,
}

/// Compare behavior definitions by value (title excluded by the donor).
#[must_use]
pub fn same_weapon_behavior(left: &WeaponBehaviorDefinition, right: &WeaponBehaviorDefinition) -> bool {
    fn same_module(left: &ModuleIdentity, right: &ModuleIdentity) -> bool {
        left.id == right.id
            && left.digest == right.digest
            && left.artifact_path == right.artifact_path
            && left.revision == right.revision
    }
    fn same_callback(left: &Option<WeaponBehaviorCallback>, right: &Option<WeaponBehaviorCallback>) -> bool {
        match (left, right) {
            (None, None) => true,
            (
                Some(WeaponBehaviorCallback::Quakec {
                    module: left_module,
                    function_index: left_index,
                }),
                Some(WeaponBehaviorCallback::Quakec {
                    module: right_module,
                    function_index: right_index,
                }),
            ) => same_module(left_module, right_module) && left_index == right_index,
            (
                Some(WeaponBehaviorCallback::Qvm {
                    module: left_module,
                    instruction_index: left_index,
                }),
                Some(WeaponBehaviorCallback::Qvm {
                    module: right_module,
                    instruction_index: right_index,
                }),
            ) => same_module(left_module, right_module) && left_index == right_index,
            (
                Some(WeaponBehaviorCallback::NativeArtifact {
                    module: left_module,
                    image_offset: left_offset,
                    abi: left_abi,
                }),
                Some(WeaponBehaviorCallback::NativeArtifact {
                    module: right_module,
                    image_offset: right_offset,
                    abi: right_abi,
                }),
            ) => {
                same_module(left_module, right_module)
                    && left_offset == right_offset
                    && write_native_call_abi(left_abi) == write_native_call_abi(right_abi)
            }
            _ => false,
        }
    }
    left.id == right.id
        && left.role == right.role
        && left.aspect == right.aspect
        && same_module(&left.module, &right.module)
        && same_callback(&Some(left.fire.clone()), &Some(right.fire.clone()))
        && same_callback(&left.activate, &right.activate)
}

/// Structurally validated native weapon declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeWeaponBehaviorDeclaration {
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub artifact_digest: String,
    /// Id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Role.
    pub role: String,
    /// Activate RVA.
    pub activate_rva: Option<i128>,
    /// Fire RVA.
    pub fire_rva: i128,
    /// Retained record (loader-owned layout tables preserved verbatim).
    pub retained: SaveJson,
}

fn bad(message: &str) -> PersistenceError {
    PersistenceError::BadSave(message.to_string())
}

fn exact_keys(reader: &SaveReader, expected: &[&str]) -> Result<(), PersistenceError> {
    let keys: HashSet<String> = match &reader.value {
        Some(SaveJson::Object(members)) => members.iter().map(|(key, _)| key.clone()).collect(),
        _ => return Err(PersistenceError::from(reader.fail("expected an object"))),
    };
    let wanted: HashSet<String> = expected.iter().map(|key| key.to_string()).collect();
    if keys == wanted {
        Ok(())
    } else {
        Err(PersistenceError::from(reader.fail("unexpected declaration keys")))
    }
}

fn nonzero_uint(reader: SaveReader) -> Result<i128, PersistenceError> {
    let value = reader.bigint()?;
    if value < 1 {
        return Err(PersistenceError::from(reader.fail("expected a nonzero address")));
    }
    Ok(value)
}

fn call_rvas(reader: SaveReader) -> Result<Vec<i128>, PersistenceError> {
    reader
        .field("calls")
        .list(|call| call.field("rva").bigint().map_err(PersistenceError::from))
}

/// Read a native weapon declaration structurally, binding the artifact to a module.
pub fn read_native_weapon_declaration(
    reader: SaveReader,
    module: Option<&ModuleIdentity>,
) -> Result<NativeWeaponBehaviorDeclaration, PersistenceError> {
    exact_keys(
        &reader,
        &[
            "version",
            "kind",
            "abi",
            "artifactPath",
            "artifactDigest",
            "id",
            "title",
            "role",
            "aspect",
            "entity",
            "client",
            "equippedWeapon",
            "time",
            "think",
            "allocate",
            "free",
            "projectileTouch",
            "equip",
            "launch",
            "activateRva",
            "fireRva",
            "initializationClasses",
            "equipment",
            "ammunition",
            "initialCvars",
            "provisioningCvars",
        ],
    )?;
    exact_keys(
        &reader.field("entity"),
        &[
            "byteLength",
            "origin",
            "angles",
            "velocity",
            "client",
            "owner",
            "viewHeight",
            "generation",
            "nextThink",
            "thinkCallback",
            "thinkRegistration",
            "touchCallback",
        ],
    )?;
    exact_keys(
        &reader.field("client"),
        &["byteLength", "weapon", "viewAngles", "forward"],
    )?;
    exact_keys(&reader.field("equippedWeapon"), &["byteLength", "callback", "expected"])?;
    exact_keys(&reader.field("time"), &["storage", "rva"])?;
    exact_keys(&reader.field("think"), &["signature", "tag", "registration"])?;
    exact_keys(&reader.field("allocate"), &["signature", "entry"])?;
    exact_keys(&reader.field("free"), &["signature", "entry"])?;
    reader.field("version").literal_i64(1)?;
    reader.field("kind").literal_str("q2-api2023-trajectory")?;
    reader.field("abi").literal_str("windows-x86-64")?;
    let artifact_path = normalize_resource_path(&reader.field("artifactPath").string()?)?;
    let digest = reader.field("artifactDigest").string()?;
    validate_digest(&digest).map_err(|_| {
        PersistenceError::from(
            reader
                .field("artifactDigest")
                .fail("expected canonical SHA256 identity"),
        )
    })?;
    if let Some(module) = module {
        if module.digest != digest || module.artifact_path != artifact_path {
            return Err(PersistenceError::from(
                reader.fail("native profile artifact identity differs"),
            ));
        }
    }
    let activate_rva = reader.field("activateRva").nullable(nonzero_uint)?;
    let fire_rva = nonzero_uint(reader.field("fireRva"))?;
    let equip_rvas = call_rvas(reader.field("equip"))?;
    let launch_rvas = call_rvas(reader.field("launch"))?;
    if activate_rva.is_some_and(|rva| !equip_rvas.contains(&rva)) {
        return Err(PersistenceError::from(
            reader.fail("activation must identify a declared equip call"),
        ));
    }
    if !launch_rvas.contains(&fire_rva) {
        return Err(PersistenceError::from(
            reader.fail("fire must identify a declared launch call"),
        ));
    }
    let classes = reader
        .field("initializationClasses")
        .list(|value| value.string().map_err(PersistenceError::from))?;
    if classes.iter().filter(|name| name.as_str() == "worldspawn").count() != 1
        || classes.iter().collect::<HashSet<_>>().len() != classes.len()
    {
        return Err(PersistenceError::from(
            reader.fail("initialization classes require one worldspawn and unique classes"),
        ));
    }
    Ok(NativeWeaponBehaviorDeclaration {
        artifact_path,
        artifact_digest: digest,
        id: namespaced(reader.field("id"))?,
        title: reader.field("title").string()?,
        role: reader
            .field("role")
            .choice_str(&["rocket", "grenade", "nail", "bolt", "plasma", "energy", "grapple"])?,
        activate_rva,
        fire_rva,
        retained: reader.value.cloned().unwrap_or(SaveJson::Null),
    })
}

/// Write a native weapon declaration (retained record, loader tables verbatim).
#[must_use]
pub fn write_native_weapon_declaration(declaration: &NativeWeaponBehaviorDeclaration) -> SaveJson {
    declaration.retained.clone()
}

fn behavior_entry(definition: &WeaponBehaviorDefinition, rva: Option<i128>) -> Option<WeaponBehaviorCallback> {
    rva.map(|image_offset| WeaponBehaviorCallback::NativeArtifact {
        module: definition.module.clone(),
        image_offset,
        abi: NativeCallAbi::WindowsX8664,
    })
}

/// Read a retained native declaration, defaulting to the caller-supplied legacy profile.
pub fn read_saved_native_weapon_declaration(
    reader: SaveReader,
    definition: &WeaponBehaviorDefinition,
    legacy: Option<NativeWeaponBehaviorDeclaration>,
) -> Result<NativeWeaponBehaviorDeclaration, PersistenceError> {
    let declaration = if reader.is_missing() {
        legacy.ok_or_else(|| bad("native weapon save has no retained declaration or matching legacy profile"))?
    } else {
        read_native_weapon_declaration(reader.clone(), Some(&definition.module))?
    };
    let expected = WeaponBehaviorDefinition {
        id: declaration.id.clone(),
        title: declaration.title.clone(),
        module: definition.module.clone(),
        role: declaration.role.clone(),
        aspect: "trajectory".to_string(),
        activate: behavior_entry(definition, declaration.activate_rva),
        fire: behavior_entry(definition, Some(declaration.fire_rva)).expect("fire RVA present"),
    };
    if definition.title != declaration.title
        || definition.artifact_identity() != declaration.artifact_identity()
        || !same_weapon_behavior(definition, &expected)
    {
        return Err(bad("native weapon declaration differs from saved behavior identity"));
    }
    Ok(declaration)
}

trait ArtifactIdentity {
    /// Artifact path plus digest.
    fn artifact_identity(&self) -> (String, String);
}

impl ArtifactIdentity for WeaponBehaviorDefinition {
    fn artifact_identity(&self) -> (String, String) {
        (self.module.artifact_path.clone(), self.module.digest.clone())
    }
}

impl ArtifactIdentity for NativeWeaponBehaviorDeclaration {
    fn artifact_identity(&self) -> (String, String) {
        (self.artifact_path.clone(), self.artifact_digest.clone())
    }
}

/// Read a weapon behavior callback bound to the selected module.
pub fn read_weapon_behavior_callback(
    reader: SaveReader,
    module: &ModuleIdentity,
) -> Result<WeaponBehaviorCallback, PersistenceError> {
    let kind = reader.field("kind").choice_str(&["quakec", "qvm", "native-artifact"])?;
    let owner = read_module(reader.field("module"))?;
    if owner != *module {
        return Err(PersistenceError::from(
            reader.fail("weapon behavior callback differs from selected module"),
        ));
    }
    match kind.as_str() {
        "quakec" => {
            let index = reader.field("functionIndex").integer(1)?;
            Ok(WeaponBehaviorCallback::Quakec {
                module: owner,
                function_index: u64::try_from(index).map_err(|_| {
                    PersistenceError::from(reader.field("functionIndex").fail("expected an integer in range"))
                })?,
            })
        }
        "qvm" => {
            let index = reader.field("instructionIndex").integer(0)?;
            Ok(WeaponBehaviorCallback::Qvm {
                module: owner,
                instruction_index: u64::try_from(index).map_err(|_| {
                    PersistenceError::from(reader.field("instructionIndex").fail("expected an integer in range"))
                })?,
            })
        }
        _ => {
            let abi = read_native_call_abi(reader.field("abi"))?;
            let image_offset = reader.field("imageOffset").bigint()?;
            let maximum = match abi {
                NativeCallAbi::WindowsI386 { .. } | NativeCallAbi::LinuxI386 => 0xffff_ffffi128,
                _ => 0xffff_ffff_ffff_ffffi128,
            };
            if image_offset < 0 || image_offset > maximum {
                return Err(PersistenceError::from(
                    reader.fail("native behavior image offset is outside its ABI address width"),
                ));
            }
            Ok(WeaponBehaviorCallback::NativeArtifact {
                module: owner,
                image_offset,
                abi,
            })
        }
    }
}

/// Write a weapon behavior callback.
#[must_use]
pub fn write_weapon_behavior_callback(callback: &WeaponBehaviorCallback) -> SaveJson {
    match callback {
        WeaponBehaviorCallback::Quakec { module, function_index } => obj(vec![
            ("kind", str("quakec")),
            ("module", write_module(module)),
            #[allow(clippy::cast_possible_wrap)]
            ("functionIndex", int(*function_index as i64)),
        ]),
        WeaponBehaviorCallback::Qvm {
            module,
            instruction_index,
        } => obj(vec![
            ("kind", str("qvm")),
            ("module", write_module(module)),
            #[allow(clippy::cast_possible_wrap)]
            ("instructionIndex", int(*instruction_index as i64)),
        ]),
        WeaponBehaviorCallback::NativeArtifact {
            module,
            image_offset,
            abi,
        } => obj(vec![
            ("kind", str("native-artifact")),
            ("module", write_module(module)),
            ("imageOffset", SaveJson::BigInt(*image_offset)),
            ("abi", write_native_call_abi(abi)),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_world::save::value::arr;

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "q2:weapons".to_string(),
            artifact_path: "weapons.dll".to_string(),
            digest: format!("sha256:{}", "1".repeat(64)),
            revision: "2023".to_string(),
        }
    }

    fn declaration_json() -> SaveJson {
        obj(vec![
            ("version", int(1)),
            ("kind", str("q2-api2023-trajectory")),
            ("abi", str("windows-x86-64")),
            ("artifactPath", str("weapons.dll")),
            ("artifactDigest", str(&format!("sha256:{}", "1".repeat(64)))),
            ("id", str("q2:rocket")),
            ("title", str("Rocket")),
            ("role", str("rocket")),
            ("aspect", str("trajectory")),
            (
                "entity",
                obj(vec![
                    ("byteLength", int(512)),
                    ("origin", int(0)),
                    ("angles", int(12)),
                    ("velocity", int(24)),
                    ("client", int(36)),
                    ("owner", int(44)),
                    ("viewHeight", int(52)),
                    ("generation", int(56)),
                    ("nextThink", int(64)),
                    ("thinkCallback", int(72)),
                    ("thinkRegistration", int(80)),
                    ("touchCallback", int(88)),
                ]),
            ),
            (
                "client",
                obj(vec![
                    ("byteLength", int(128)),
                    ("weapon", int(0)),
                    ("viewAngles", int(8)),
                    ("forward", int(20)),
                ]),
            ),
            (
                "equippedWeapon",
                obj(vec![
                    ("byteLength", int(32)),
                    ("callback", int(0)),
                    ("expected", int(8)),
                ]),
            ),
            (
                "time",
                obj(vec![
                    ("storage", str("int64-milliseconds")),
                    ("rva", SaveJson::BigInt(16)),
                ]),
            ),
            (
                "think",
                obj(vec![
                    ("signature", str("entity-void")),
                    ("tag", int(1)),
                    ("registration", int(2)),
                ]),
            ),
            (
                "allocate",
                obj(vec![("signature", str("void-pointer")), ("entry", int(3))]),
            ),
            ("free", obj(vec![("signature", str("entity-void")), ("entry", int(4))])),
            ("projectileTouch", int(5)),
            (
                "equip",
                obj(vec![("calls", arr(vec![obj(vec![("rva", SaveJson::BigInt(0x100))])]))]),
            ),
            (
                "launch",
                obj(vec![("calls", arr(vec![obj(vec![("rva", SaveJson::BigInt(0x200))])]))]),
            ),
            ("activateRva", SaveJson::BigInt(0x100)),
            ("fireRva", SaveJson::BigInt(0x200)),
            (
                "initializationClasses",
                arr(vec![str("worldspawn"), str("info_player_start")]),
            ),
            ("equipment", arr(Vec::new())),
            ("ammunition", obj(Vec::new())),
            ("initialCvars", obj(Vec::new())),
            ("provisioningCvars", obj(Vec::new())),
        ])
    }

    #[test]
    fn declarations_bind_and_compare() {
        let json = declaration_json();
        let declaration = read_native_weapon_declaration(SaveReader::new(&json), Some(&module())).unwrap();
        assert_eq!(declaration.role, "rocket");
        assert_eq!(write_native_weapon_declaration(&declaration), json);
        let definition = WeaponBehaviorDefinition {
            id: "q2:rocket".to_string(),
            title: "Rocket".to_string(),
            module: module(),
            role: "rocket".to_string(),
            aspect: "trajectory".to_string(),
            activate: Some(WeaponBehaviorCallback::NativeArtifact {
                module: module(),
                image_offset: 0x100,
                abi: NativeCallAbi::WindowsX8664,
            }),
            fire: WeaponBehaviorCallback::NativeArtifact {
                module: module(),
                image_offset: 0x200,
                abi: NativeCallAbi::WindowsX8664,
            },
        };
        assert!(read_saved_native_weapon_declaration(SaveReader::new(&json), &definition, None).is_ok());
        // Legacy fallback supplies omitted declarations.
        let empty = obj(Vec::new());
        assert!(read_saved_native_weapon_declaration(
            SaveReader::new(&empty).field("component"),
            &definition,
            Some(declaration.clone())
        )
        .is_ok());
        assert!(
            read_saved_native_weapon_declaration(SaveReader::new(&empty).field("component"), &definition, None)
                .is_err()
        );
        let mut other = definition.clone();
        other.role = "grenade".to_string();
        assert!(!same_weapon_behavior(&definition, &other));
    }
}
