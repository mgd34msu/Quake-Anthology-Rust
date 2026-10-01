//! Rerelease native weapon checkpoint reader.
//!
//! Port of donor `src/app/bootstrap/simulation/rerelease-weapon-checkpoint.ts`
//! (`readRereleaseWeaponBehaviorCheckpoint`).

use std::rc::Rc;

use std::collections::HashMap;

use qa_compat::q2::rerelease::native_weapon_declaration::{
    read_native_weapon_declaration, same_native_weapon_declaration, DeclValue, NativeWeaponBehaviorDeclaration,
};
use qa_content::contract::WeaponBehaviorDefinition;
use qa_world::save::records::read_saved_actor;
use qa_world::save::shared::read_vector;
use qa_world::save::value::{SaveJson, SaveReader};

use super::classic_guest_world::ClassicGuestMap;
use super::native_q2_rerelease_save::read_rerelease_source_save;
use super::rerelease_weapon_behavior::{
    RereleaseWeaponBehaviorCheckpoint, WeaponBindingEntry, WeaponBindingKind, WeaponConfigstringEntry,
    WeaponRetiredEntry,
};

/// Weapon checkpoint read error.
#[derive(Debug, thiserror::Error)]
pub enum RereleaseWeaponCheckpointError {
    /// Invalid checkpoint data.
    #[error("invalid API2023 weapon checkpoint: {0}")]
    Invalid(String),
}

impl RereleaseWeaponCheckpointError {
    /// Invalid-data error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl From<qa_world::WorldError> for RereleaseWeaponCheckpointError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::invalid(error.to_string())
    }
}

fn mapped(error: impl ToString) -> RereleaseWeaponCheckpointError {
    RereleaseWeaponCheckpointError::invalid(error.to_string())
}

/// Read a host save bundle. Deferred damage and projections have no
/// compat host counterpart, so saves carrying whole-world native state
/// are rejected (donor `native-mod-host.ts` checkpoint guard).
fn source_save(
    reader: SaveReader,
) -> Result<qa_compat::q2::rerelease::host::SourceSave, RereleaseWeaponCheckpointError> {
    let saved = read_rerelease_source_save(reader).map_err(mapped)?;
    if !saved.deferred_damage.is_empty() || !saved.projections.is_empty() {
        return Err(RereleaseWeaponCheckpointError::invalid(
            "weapon checkpoint carries whole-world native save state",
        ));
    }
    Ok(qa_compat::q2::rerelease::host::SourceSave {
        native: saved.native,
        deferred: Vec::new(),
        projections: Vec::new(),
    })
}

/// Bridge a saved declaration subtree into the loader's declaration value.
fn declaration_value(json: &SaveJson) -> Result<DeclValue, RereleaseWeaponCheckpointError> {
    match json {
        SaveJson::Null => Ok(DeclValue::Null),
        SaveJson::Bool(value) => Ok(DeclValue::Bool(*value)),
        SaveJson::Number(value) => {
            if value.is_finite() && value.fract() == 0.0 {
                #[allow(clippy::cast_possible_truncation)]
                Ok(DeclValue::Int(*value as i64))
            } else {
                Err(RereleaseWeaponCheckpointError::invalid(
                    "native weapon declaration needs integer numbers",
                ))
            }
        }
        SaveJson::BigInt(value) => i64::try_from(*value)
            .map(DeclValue::Int)
            .map_err(|_| RereleaseWeaponCheckpointError::invalid("native weapon declaration integer exceeds i64")),
        SaveJson::Bytes(_) => Err(RereleaseWeaponCheckpointError::invalid(
            "native weapon declaration has no byte fields",
        )),
        SaveJson::String(value) => Ok(DeclValue::Str(value.clone())),
        SaveJson::Array(items) => items
            .iter()
            .map(declaration_value)
            .collect::<Result<Vec<_>, _>>()
            .map(DeclValue::List),
        SaveJson::Object(members) => {
            let mut map = HashMap::with_capacity(members.len());
            for (key, value) in members {
                map.insert(key.clone(), declaration_value(value)?);
            }
            Ok(DeclValue::Map(map))
        }
    }
}

/// Saved definition reader seam (donor `readWeaponBehaviorDefinition`
/// from donor `src/world/gameplay/weapon-behaviors.ts`, canonical home:
/// gameplay lane; unify post-merge).
pub type WeaponDefinitionReadFn = Rc<
    dyn Fn(SaveReader, &WeaponBehaviorDefinition) -> Result<WeaponBehaviorDefinition, RereleaseWeaponCheckpointError>,
>;

/// Read a weapon behavior checkpoint, validating its definition and
/// declaration against the live component.
pub fn read_rerelease_weapon_behavior_checkpoint(
    reader: SaveReader,
    definition: &WeaponBehaviorDefinition,
    expected_declaration: &NativeWeaponBehaviorDeclaration,
    read_definition: &WeaponDefinitionReadFn,
) -> Result<RereleaseWeaponBehaviorCheckpoint, RereleaseWeaponCheckpointError> {
    let saved_definition = read_definition(reader.field("definition"), definition)?;
    let declaration_json = reader
        .field("declaration")
        .value
        .cloned()
        .ok_or_else(|| RereleaseWeaponCheckpointError::invalid("native weapon save has no declaration"))?;
    let declaration_value = declaration_value(&declaration_json)?;
    let declaration = read_native_weapon_declaration(
        &declaration_value,
        Some((
            expected_declaration.artifact_digest.as_str(),
            expected_declaration.artifact_path.as_str(),
        )),
    )
    .map_err(mapped)?;
    if !same_native_weapon_declaration(&declaration, expected_declaration) {
        return Err(RereleaseWeaponCheckpointError::invalid(
            "native trajectory checkpoint declaration changed",
        ));
    }
    let map_field = reader.field("map");
    let mut bindings = Vec::new();
    for entry in reader
        .field("bindings")
        .list(|value| {
            let slot = value.field("slot").integer(1).map_err(mapped)?;
            let kind = match value
                .field("kind")
                .choice_str(&["client", "target", "projectile"])
                .map_err(mapped)?
                .as_str()
            {
                "client" => WeaponBindingKind::Client,
                "target" => WeaponBindingKind::Target,
                _ => WeaponBindingKind::Projectile,
            };
            Ok::<_, RereleaseWeaponCheckpointError>(WeaponBindingEntry {
                slot: u32::try_from(slot).map_err(mapped)?,
                actor: read_saved_actor(value.field("actor")).map_err(mapped)?,
                kind,
                generation: i32::try_from(value.field("generation").integer(0).map_err(mapped)?).map_err(mapped)?,
            })
        })
        .map_err(mapped)?
    {
        bindings.push(entry);
    }
    let mut retired = Vec::new();
    for entry in reader
        .field("retired")
        .list(|value| {
            let trajectory_field = value.field("trajectory");
            Ok::<_, RereleaseWeaponCheckpointError>(WeaponRetiredEntry {
                actor: read_saved_actor(value.field("actor")).map_err(mapped)?,
                trajectory: qa_content::q2::support::contracts::WeaponTrajectoryUpdate {
                    origin: read_vector(trajectory_field.field("origin")).map_err(mapped)?,
                    angles: read_vector(trajectory_field.field("angles")).map_err(mapped)?,
                    velocity: read_vector(trajectory_field.field("velocity")).map_err(mapped)?,
                },
            })
        })
        .map_err(mapped)?
    {
        retired.push(entry);
    }
    let mut configstrings = Vec::new();
    for entry in reader
        .field("configstrings")
        .list(|value| {
            Ok::<_, RereleaseWeaponCheckpointError>(WeaponConfigstringEntry {
                index: i32::try_from(value.field("index").integer(0).map_err(mapped)?).map_err(mapped)?,
                value: value.field("value").string().map_err(mapped)?,
            })
        })
        .map_err(mapped)?
    {
        configstrings.push(entry);
    }
    Ok(RereleaseWeaponBehaviorCheckpoint {
        version: u32::try_from(reader.field("version").literal_i64(1).map_err(mapped)?).map_err(mapped)?,
        declaration,
        definition: saved_definition,
        map: ClassicGuestMap {
            map: map_field.field("map").string().map_err(mapped)?,
            entities: map_field.field("entities").string().map_err(mapped)?,
            spawn_point: map_field.field("spawnPoint").string().map_err(mapped)?,
        },
        time: reader.field("time").finite().map_err(mapped)?,
        cvars: reader.field("cvars").bytes().map_err(mapped)?,
        game: source_save(reader.field("game"))?,
        level: source_save(reader.field("level"))?,
        configstrings,
        retired,
        bindings,
    })
}

#[cfg(test)]
mod tests {
    use qa_compat::q2::rerelease::layouts::{edict_layout, field_offset};
    use qa_content::contract::{ContentDigest, ModuleIdentity, ProjectileRole};
    use qa_core::identity::ProviderId;
    use qa_core::identity::SavedActorId;
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::shared::write_vector;
    use qa_world::save::value::{arr, boolean, int, num, obj, str as json_str, SaveJson};

    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: ProviderId {
                namespace: "test".to_string(),
                name: "weapon".to_string(),
            },
            artifact_path: "q2game.dll".to_string(),
            digest: ContentDigest(DIGEST.to_string()),
            revision: DIGEST.to_string(),
        }
    }

    fn definition() -> WeaponBehaviorDefinition {
        WeaponBehaviorDefinition {
            id: "test:rl".to_string(),
            title: "Rocket Launcher".to_string(),
            module: module(),
            role: ProjectileRole::Rocket,
            activate: None,
            fire: qa_content::contract::WeaponBehaviorCallback::NativeArtifact {
                module: module(),
                image_offset: 0x300,
                abi: qa_content::contract::NativeCallAbi::Native(qa_content::contract::NativeAbi::WindowsX86_64),
            },
        }
    }

    fn entry(rva: i64) -> SaveJson {
        obj(vec![("rva", int(rva)), ("registration", SaveJson::Null)])
    }

    fn declaration_json() -> SaveJson {
        let edict = edict_layout();
        let at = |name: &str| field_offset(&edict, name).expect("offset") as i64;
        let mut cursor = at("s.angles") + 12;
        let mut place = |width: i64| {
            let align = if width == 12 { 4 } else { width.min(8) };
            cursor = (cursor + align - 1) / align * align;
            let offset = cursor;
            cursor += width;
            offset
        };
        let velocity = place(12);
        let view_height = place(4);
        let generation = place(4);
        let next_think = place(8);
        let think_callback = place(8);
        let think_registration = place(8);
        let touch_callback = place(8);
        obj(vec![
            ("version", int(1)),
            ("kind", json_str("q2-api2023-trajectory")),
            ("abi", json_str("windows-x86-64")),
            ("artifactPath", json_str("q2game.dll")),
            ("artifactDigest", json_str(DIGEST)),
            ("id", json_str("test:rl")),
            ("title", json_str("Rocket Launcher")),
            ("role", json_str("rocket")),
            ("aspect", json_str("trajectory")),
            (
                "entity",
                obj(vec![
                    ("byteLength", int(2048)),
                    ("origin", int(at("s.origin"))),
                    ("angles", int(at("s.angles"))),
                    ("velocity", int(velocity)),
                    ("client", int(at("client"))),
                    ("owner", int(at("owner"))),
                    ("viewHeight", int(view_height)),
                    ("generation", int(generation)),
                    ("nextThink", int(next_think)),
                    ("thinkCallback", int(think_callback)),
                    ("thinkRegistration", int(think_registration)),
                    ("touchCallback", int(touch_callback)),
                ]),
            ),
            (
                "client",
                obj(vec![
                    ("byteLength", int(256)),
                    ("weapon", int(0)),
                    ("viewAngles", int(8)),
                    ("forward", int(20)),
                ]),
            ),
            (
                "equippedWeapon",
                obj(vec![
                    ("byteLength", int(64)),
                    ("callback", int(0)),
                    ("expected", entry(0x500)),
                ]),
            ),
            (
                "time",
                obj(vec![("storage", json_str("int64-milliseconds")), ("rva", int(0x40))]),
            ),
            (
                "think",
                obj(vec![
                    ("signature", json_str("entity-void")),
                    ("tag", int(7)),
                    (
                        "registration",
                        obj(vec![
                            ("byteLength", int(32)),
                            ("name", int(0)),
                            ("tag", int(8)),
                            ("callback", int(16)),
                        ]),
                    ),
                ]),
            ),
            (
                "allocate",
                obj(vec![("signature", json_str("void-pointer")), ("entry", entry(0x100))]),
            ),
            (
                "free",
                obj(vec![("signature", json_str("entity-void")), ("entry", entry(0x110))]),
            ),
            ("projectileTouch", entry(0x120)),
            (
                "equip",
                obj(vec![
                    ("signature", json_str("entity-void")),
                    ("calls", arr(vec![entry(0x200)])),
                ]),
            ),
            (
                "launch",
                obj(vec![
                    ("signature", json_str("entity-void")),
                    ("calls", arr(vec![entry(0x300)])),
                ]),
            ),
            ("activateRva", SaveJson::Null),
            ("fireRva", int(0x300)),
            ("initializationClasses", arr(vec![json_str("worldspawn")])),
            (
                "equipment",
                arr(vec![obj(vec![
                    ("arguments", arr(vec![json_str("give"), json_str("rockets")])),
                    ("tail", json_str("rockets")),
                ])]),
            ),
            (
                "ammunition",
                obj(vec![
                    ("arguments", arr(vec![json_str("use"), json_str("rl")])),
                    ("tail", json_str("rl")),
                ]),
            ),
            (
                "initialCvars",
                arr(vec![obj(vec![("name", json_str("w_skill")), ("value", json_str("2"))])]),
            ),
            (
                "provisioningCvars",
                arr(vec![obj(vec![("name", json_str("w_prov")), ("value", json_str("1"))])]),
            ),
        ])
    }

    fn source_save() -> SaveJson {
        obj(vec![
            ("native", SaveJson::Bytes(b"{}".to_vec())),
            ("deferredDamage", arr(vec![])),
            ("projections", arr(vec![])),
        ])
    }

    fn checkpoint_json(declaration: SaveJson) -> SaveJson {
        let actor = SavedActorId {
            slot: 11,
            generation: 1,
        };
        let zero = write_vector(qa_core::math::Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        obj(vec![
            ("version", int(1)),
            ("definition", obj(vec![])),
            ("declaration", declaration),
            (
                "map",
                obj(vec![
                    ("map", json_str("q2dm1")),
                    ("entities", json_str("spawns")),
                    ("spawnPoint", json_str("start")),
                ]),
            ),
            ("time", num(1.5)),
            ("cvars", SaveJson::Bytes(vec![7, 8])),
            ("game", source_save()),
            ("level", source_save()),
            (
                "configstrings",
                arr(vec![obj(vec![("index", int(4)), ("value", json_str("on"))])]),
            ),
            (
                "retired",
                arr(vec![obj(vec![
                    ("actor", write_saved_actor(actor)),
                    (
                        "trajectory",
                        obj(vec![
                            ("origin", zero.clone()),
                            ("angles", zero.clone()),
                            ("velocity", zero.clone()),
                        ]),
                    ),
                ])]),
            ),
            (
                "bindings",
                arr(vec![obj(vec![
                    ("slot", int(5)),
                    ("actor", write_saved_actor(actor)),
                    ("kind", json_str("projectile")),
                    ("generation", int(0)),
                ])]),
            ),
        ])
    }

    fn expected() -> NativeWeaponBehaviorDeclaration {
        let json = declaration_json();
        let value = declaration_value(&json).expect("bridge");
        read_native_weapon_declaration(&value, Some((DIGEST, "q2game.dll"))).expect("declaration")
    }

    #[test]
    fn reads_full_checkpoint() {
        let json = checkpoint_json(declaration_json());
        let reader = SaveReader::new(&json);
        let live = definition();
        let seam: WeaponDefinitionReadFn = Rc::new(|_, live| Ok(live.clone()));
        let saved = read_rerelease_weapon_behavior_checkpoint(reader, &live, &expected(), &seam).expect("checkpoint");
        assert_eq!(saved.version, 1);
        assert_eq!(saved.map.map, "q2dm1");
        assert_eq!(saved.time, 1.5);
        assert_eq!(saved.cvars, vec![7, 8]);
        assert_eq!(saved.configstrings.len(), 1);
        assert_eq!(saved.retired.len(), 1);
        assert_eq!(saved.bindings.len(), 1);
        assert_eq!(saved.bindings[0].slot, 5);
        assert_eq!(saved.bindings[0].kind, WeaponBindingKind::Projectile);
        assert_eq!(saved.definition, live);
    }

    #[test]
    fn rejects_changed_declaration() {
        let json = checkpoint_json(declaration_json());
        let reader = SaveReader::new(&json);
        let live = definition();
        let seam: WeaponDefinitionReadFn = Rc::new(|_, live| Ok(live.clone()));
        let mut changed = expected();
        changed.title = "Different".to_string();
        assert!(read_rerelease_weapon_behavior_checkpoint(reader, &live, &changed, &seam).is_err());
    }

    #[test]
    fn rejects_bad_version() {
        let mut json = checkpoint_json(declaration_json());
        if let SaveJson::Object(members) = &mut json {
            for (key, value) in members.iter_mut() {
                if key == "version" {
                    *value = int(2);
                }
            }
        }
        let reader = SaveReader::new(&json);
        let live = definition();
        let seam: WeaponDefinitionReadFn = Rc::new(|_, live| Ok(live.clone()));
        assert!(read_rerelease_weapon_behavior_checkpoint(reader, &live, &expected(), &seam).is_err());
    }

    #[test]
    fn rejects_missing_declaration() {
        let mut json = checkpoint_json(declaration_json());
        if let SaveJson::Object(members) = &mut json {
            members.retain(|(key, _)| key != "declaration");
        }
        let reader = SaveReader::new(&json);
        let live = definition();
        let seam: WeaponDefinitionReadFn = Rc::new(|_, live| Ok(live.clone()));
        assert!(read_rerelease_weapon_behavior_checkpoint(reader, &live, &expected(), &seam).is_err());
    }

    #[test]
    fn bridge_rejects_non_integer_numbers() {
        assert!(declaration_value(&num(1.5)).is_err());
        assert!(declaration_value(&SaveJson::Bytes(vec![1])).is_err());
        assert!(matches!(
            declaration_value(&boolean(true)).expect("bool"),
            DeclValue::Bool(true)
        ));
    }
}
