//! Grapple profile: exact-bytes declaration parsing.
//!
//! Provenance: `src/compat/qvm/grapple-profile.ts`.
//!
//! Absorbs the pure-Rust types of `src/contracts/qvm-grapple.ts`
//! ([`QvmGrappleProfile`] and its field types). Record sizes reuse
//! [`super::player_record::qvm_player_state_bytes`] and
//! [`super::shared_entity_record::qvm_shared_entity_bytes`].

use std::collections::BTreeMap;

use qa_core::math::{vec3, Vec3};

use super::game_data::{AbiProfile, ModuleIdentity, ProfileReader, ProfileValue, QvmArtifact, QvmOpcode, QvmRole};
use super::player_record::qvm_player_state_bytes;
use super::shared_entity_record::qvm_shared_entity_bytes;
use crate::error::GuestError;

/// Grapple entity fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGrappleFields {
    /// In-use offset.
    pub inuse: usize,
    /// Client offset.
    pub client: usize,
    /// Parent offset.
    pub parent: usize,
    /// Target offset.
    pub target: usize,
    /// Mover offset, if any.
    pub mover: Option<usize>,
    /// Hook offset in the player record.
    pub hook: usize,
    /// Health offset.
    pub health: usize,
    /// Take-damage offset.
    pub takedamage: usize,
    /// Event-time offset.
    pub event_time: usize,
    /// Free-after-event offset.
    pub free_after_event: usize,
}

/// Grapple globals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGrappleGlobals {
    /// Time global.
    pub time: usize,
    /// Frame global.
    pub frame: usize,
    /// Movement global.
    pub movement: usize,
    /// Forward vector global.
    pub forward: usize,
    /// Ground-plane global.
    pub ground_plane: usize,
}

/// Grapple callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGrappleCallbacks {
    /// Allocate entry.
    pub allocate: usize,
    /// Free entry.
    pub free: usize,
    /// Fire entry.
    pub fire: usize,
    /// Release entry.
    pub release: usize,
    /// Force-release entry.
    pub force_release: usize,
    /// Missile entry.
    pub missile: usize,
    /// Follow entry, if any.
    pub follow: Option<usize>,
    /// Think entry.
    pub think: usize,
    /// Pull entry.
    pub pull: usize,
    /// Move-mover-hooks entry, if any.
    pub move_mover_hooks: Option<usize>,
    /// Damage entry.
    pub damage: usize,
    /// Same-team entry.
    pub same_team: usize,
    /// Player-move entry.
    pub player_move: usize,
}

/// Movement projection word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGrappleWord {
    /// Word offset.
    pub offset: usize,
    /// Word value.
    pub value: i32,
}

/// Movement projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrappleMovement {
    /// Record byte length.
    pub byte_length: usize,
    /// Projection words.
    pub words: Vec<QvmGrappleWord>,
}

/// View anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmViewAnchor {
    /// Model path.
    pub path: String,
    /// Tag name.
    pub tag: String,
    /// Offset.
    pub offset: Vec3,
    /// FOV above offset.
    pub fov_above: i64,
    /// FOV scale.
    pub fov_scale: f64,
}

/// View attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmViewAttachment {
    /// Model path.
    pub path: String,
    /// Tag name.
    pub tag: String,
}

/// Cable presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmCable {
    /// Shader cable.
    Shader {
        /// Shader path.
        path: String,
        /// Width.
        width: i64,
    },
    /// Model cable.
    Model {
        /// Flight model.
        flight: String,
        /// Pull model.
        pull: String,
        /// Hold model.
        hold: String,
        /// Segment length.
        segment_length: i64,
    },
}

/// Grapple presentation.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrapplePresentation {
    /// Projectile model.
    pub projectile_model: String,
    /// View model.
    pub view_model: String,
    /// Weapon index.
    pub weapon_index: i64,
    /// View anchor.
    pub view_anchor: QvmViewAnchor,
    /// View attachments.
    pub view_attachments: Vec<QvmViewAttachment>,
    /// Cable presentation.
    pub cable: QvmCable,
    /// Fire sound, if any.
    pub fire_sound: Option<String>,
    /// Attach sound, if any.
    pub attach_sound: Option<String>,
    /// Release sound, if any.
    pub release_sound: Option<String>,
    /// Pull sound, if any.
    pub pull_sound: Option<String>,
    /// Hang sound, if any.
    pub hang_sound: Option<String>,
}

/// Grapple profile (the `QvmGrappleDefinition` contract).
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleProfile {
    /// Profile id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Owning module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: AbiProfile,
    /// Entity stride in bytes.
    pub entity_stride: usize,
    /// Client stride in bytes.
    pub client_stride: usize,
    /// Entity fields.
    pub fields: QvmGrappleFields,
    /// Globals.
    pub globals: QvmGrappleGlobals,
    /// Callbacks.
    pub callbacks: QvmGrappleCallbacks,
    /// Fire arguments.
    pub fire_arguments: Vec<i32>,
    /// Movement projection.
    pub movement: QvmGrappleMovement,
    /// Initial cvars.
    pub initial_cvars: BTreeMap<String, String>,
    /// Event lifetime in milliseconds.
    pub event_lifetime_ms: i64,
    /// Grapple damage method.
    pub grapple_damage_method: i64,
    /// Presentation.
    pub presentation: QvmGrapplePresentation,
    /// Pulling flag (one movement bit).
    pub pulling_flag: i32,
}

/// ABI profile name.
fn abi_name(profile: AbiProfile) -> &'static str {
    if profile.is_modern() {
        "q3-modern"
    } else {
        "q3-legacy"
    }
}

/// Read a grapple profile against exact executable bytes.
pub fn read_qvm_grapple_profile(
    reader: &ProfileReader<'_>,
    artifact: &QvmArtifact,
) -> Result<QvmGrappleProfile, GuestError> {
    reader.field("version")?.literal_int(1)?;
    reader.field("artifactDigest")?.literal_str(&artifact.module.digest)?;
    reader
        .field("artifactPath")?
        .literal_str(&artifact.module.artifact_path)?;
    let abi_name = reader
        .field("abiProfile")?
        .literal_str(abi_name(artifact.abi_profile.unwrap_or(AbiProfile::Modern)))?;
    let abi_profile = if abi_name == "q3-modern" {
        AbiProfile::Modern
    } else {
        AbiProfile::Legacy
    };
    if artifact.role != QvmRole::Qagame {
        return reader.fail("grapple callbacks require a qagame module");
    }
    let entity_stride = reader
        .field("entityStride")?
        .integer(qvm_shared_entity_bytes(abi_profile) as i64)? as usize;
    let client_stride = reader
        .field("clientStride")?
        .integer(qvm_player_state_bytes(abi_profile) as i64)? as usize;
    if entity_stride % 4 != 0
        || client_stride % 4 != 0
        || entity_stride.max(client_stride) > artifact.image.allocated_data_length
    {
        return reader.fail("unaligned or oversized source records");
    }
    let data_end = artifact.image.data_length + artifact.image.literal_length + artifact.image.bss_length;
    let offset = |name: &str, minimum: usize, maximum: usize| -> Result<usize, GuestError> {
        let field = reader.field("fields")?.field(name)?;
        let result = field.integer(minimum as i64)? as usize;
        if result % 4 != 0 || result > maximum - 4 {
            return field.fail("field is outside its source record");
        }
        Ok(result)
    };
    let global = |name: &str, bytes: usize| -> Result<usize, GuestError> {
        let field = reader.field("globals")?.field(name)?;
        let result = field.integer(4)? as usize;
        if result % 4 != 0 || result.checked_add(bytes).map_or(true, |end| end > data_end) {
            return field.fail("global is outside declared source memory");
        }
        Ok(result)
    };
    let callback = |name: &str| -> Result<usize, GuestError> {
        let field = reader.field("callbacks")?.field(name)?;
        let entry = field.integer(1)? as usize;
        if artifact
            .image
            .instructions
            .get(entry)
            .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
        {
            return field.fail("callback is not a source function entry");
        }
        Ok(entry)
    };
    let entity_field = |name: &str| offset(name, qvm_shared_entity_bytes(abi_profile), entity_stride);
    let nullable_callback = |name: &str| -> Result<Option<usize>, GuestError> {
        if matches!(reader.field("callbacks")?.field(name)?.value(), ProfileValue::Null) {
            return Ok(None);
        }
        callback(name).map(Some)
    };
    let pulling_flag = reader.field("pullingFlag")?.integer(1)?;
    if pulling_flag > 0x4000_0000 || !pulling_flag.is_power_of_two() {
        return reader.field("pullingFlag")?.fail("expected one player movement flag");
    }
    let cvars = reader.field("initialCvars")?;
    let ProfileValue::Record(entries) = cvars.value() else {
        return cvars.fail("Expected initial source settings");
    };
    let mut initial_cvars = BTreeMap::new();
    for (name, _) in entries {
        initial_cvars.insert(name.clone(), cvars.field(name)?.string()?);
    }
    let presentation = reader.field("presentation")?;
    let cable = presentation.field("cable")?;
    let cable_kind = cable.field("kind")?.choice(&["shader", "model"])?;
    let anchor = presentation.field("viewAnchor")?;
    let offset_vec = anchor.field("offset")?;
    let movement = reader.field("movement")?;
    let byte_length = movement.field("byteLength")?.integer(4)? as usize;
    let id = reader.field("id")?.string()?;
    let title = reader.field("title")?.string()?;
    Ok(QvmGrappleProfile {
        id,
        title,
        module: artifact.module.clone(),
        abi_profile,
        entity_stride,
        client_stride,
        fields: QvmGrappleFields {
            inuse: entity_field("inuse")?,
            client: entity_field("client")?,
            parent: entity_field("parent")?,
            target: entity_field("target")?,
            mover: if matches!(reader.field("fields")?.field("mover")?.value(), ProfileValue::Null) {
                None
            } else {
                Some(entity_field("mover")?)
            },
            hook: offset("hook", qvm_player_state_bytes(abi_profile), client_stride)?,
            health: entity_field("health")?,
            takedamage: entity_field("takedamage")?,
            event_time: entity_field("eventTime")?,
            free_after_event: entity_field("freeAfterEvent")?,
        },
        globals: QvmGrappleGlobals {
            time: global("time", 4)?,
            frame: global("frame", 4)?,
            movement: global("movement", 4)?,
            forward: global("forward", 12)?,
            ground_plane: global("groundPlane", 4)?,
        },
        callbacks: QvmGrappleCallbacks {
            allocate: callback("allocate")?,
            free: callback("free")?,
            fire: callback("fire")?,
            release: callback("release")?,
            force_release: callback("forceRelease")?,
            missile: callback("missile")?,
            follow: nullable_callback("follow")?,
            think: callback("think")?,
            pull: callback("pull")?,
            move_mover_hooks: nullable_callback("moveMoverHooks")?,
            damage: callback("damage")?,
            same_team: callback("sameTeam")?,
            player_move: callback("playerMove")?,
        },
        pulling_flag: pulling_flag as i32,
        fire_arguments: reader
            .field("fireArguments")?
            .list(|entry| entry.integer(i64::MIN))?
            .into_iter()
            .map(|value| value as i32)
            .collect(),
        movement: QvmGrappleMovement {
            byte_length,
            words: movement.field("words")?.list(|entry| {
                let offset = entry.field("offset")?.integer(4)? as usize;
                let value = entry.field("value")?.integer(i64::MIN)? as i32;
                if offset % 4 != 0 || offset + 4 > byte_length {
                    return entry.fail("Movement field is outside the declared source record");
                }
                Ok(QvmGrappleWord { offset, value })
            })?,
        },
        initial_cvars,
        event_lifetime_ms: reader.field("eventLifetimeMilliseconds")?.integer(1)?,
        grapple_damage_method: reader.field("grappleDamageMethod")?.integer(0)?,
        presentation: QvmGrapplePresentation {
            projectile_model: presentation.field("projectileModel")?.string()?,
            view_model: presentation.field("viewModel")?.string()?,
            weapon_index: presentation.field("weaponIndex")?.integer(1)?,
            view_anchor: QvmViewAnchor {
                path: anchor.field("path")?.string()?,
                tag: anchor.field("tag")?.string()?,
                offset: vec3(
                    offset_vec.field("x")?.finite()? as f32,
                    offset_vec.field("y")?.finite()? as f32,
                    offset_vec.field("z")?.finite()? as f32,
                ),
                fov_above: anchor.field("fovOffset")?.field("above")?.integer(1)?,
                fov_scale: anchor.field("fovOffset")?.field("scale")?.finite()?,
            },
            view_attachments: presentation.field("viewAttachments")?.list(|entry| {
                Ok(QvmViewAttachment {
                    path: entry.field("path")?.string()?,
                    tag: entry.field("tag")?.string()?,
                })
            })?,
            cable: if cable_kind == "shader" {
                QvmCable::Shader {
                    path: cable.field("path")?.string()?,
                    width: cable.field("width")?.integer(1)?,
                }
            } else {
                QvmCable::Model {
                    flight: cable.field("flight")?.string()?,
                    pull: cable.field("pull")?.string()?,
                    hold: cable.field("hold")?.string()?,
                    segment_length: cable.field("segmentLength")?.integer(1)?,
                }
            },
            fire_sound: presentation.field("fireSound")?.nullable(|entry| entry.string())?,
            attach_sound: presentation.field("attachSound")?.nullable(|entry| entry.string())?,
            release_sound: presentation.field("releaseSound")?.nullable(|entry| entry.string())?,
            pull_sound: presentation.field("pullSound")?.nullable(|entry| entry.string())?,
            hang_sound: presentation.field("hangSound")?.nullable(|entry| entry.string())?,
        },
    })
}

/// Serialize a profile back to its declaration value.
#[must_use]
pub fn qvm_grapple_profile_declaration(profile: &QvmGrappleProfile) -> ProfileValue {
    let maybe = |value: Option<usize>| match value {
        Some(value) => ProfileValue::Int(value as i64),
        None => ProfileValue::Null,
    };
    let maybe_str = |value: &Option<String>| match value {
        Some(value) => ProfileValue::Str(value.clone()),
        None => ProfileValue::Null,
    };
    ProfileValue::record(vec![
        ("version", ProfileValue::Int(1)),
        ("artifactDigest", ProfileValue::Str(profile.module.digest.clone())),
        ("artifactPath", ProfileValue::Str(profile.module.artifact_path.clone())),
        ("id", ProfileValue::Str(profile.id.clone())),
        ("title", ProfileValue::Str(profile.title.clone())),
        (
            "module",
            ProfileValue::record(vec![
                ("id", ProfileValue::Str(profile.module.id.clone())),
                ("artifactPath", ProfileValue::Str(profile.module.artifact_path.clone())),
                ("digest", ProfileValue::Str(profile.module.digest.clone())),
                ("revision", ProfileValue::Str(profile.module.revision.clone())),
            ]),
        ),
        (
            "abiProfile",
            ProfileValue::Str(abi_name(profile.abi_profile).to_string()),
        ),
        ("entityStride", ProfileValue::Int(profile.entity_stride as i64)),
        ("clientStride", ProfileValue::Int(profile.client_stride as i64)),
        (
            "fields",
            ProfileValue::record(vec![
                ("inuse", ProfileValue::Int(profile.fields.inuse as i64)),
                ("client", ProfileValue::Int(profile.fields.client as i64)),
                ("parent", ProfileValue::Int(profile.fields.parent as i64)),
                ("target", ProfileValue::Int(profile.fields.target as i64)),
                ("mover", maybe(profile.fields.mover)),
                ("hook", ProfileValue::Int(profile.fields.hook as i64)),
                ("health", ProfileValue::Int(profile.fields.health as i64)),
                ("takedamage", ProfileValue::Int(profile.fields.takedamage as i64)),
                ("eventTime", ProfileValue::Int(profile.fields.event_time as i64)),
                (
                    "freeAfterEvent",
                    ProfileValue::Int(profile.fields.free_after_event as i64),
                ),
            ]),
        ),
        (
            "globals",
            ProfileValue::record(vec![
                ("time", ProfileValue::Int(profile.globals.time as i64)),
                ("frame", ProfileValue::Int(profile.globals.frame as i64)),
                ("movement", ProfileValue::Int(profile.globals.movement as i64)),
                ("forward", ProfileValue::Int(profile.globals.forward as i64)),
                ("groundPlane", ProfileValue::Int(profile.globals.ground_plane as i64)),
            ]),
        ),
        (
            "callbacks",
            ProfileValue::record(vec![
                ("allocate", ProfileValue::Int(profile.callbacks.allocate as i64)),
                ("free", ProfileValue::Int(profile.callbacks.free as i64)),
                ("fire", ProfileValue::Int(profile.callbacks.fire as i64)),
                ("release", ProfileValue::Int(profile.callbacks.release as i64)),
                (
                    "forceRelease",
                    ProfileValue::Int(profile.callbacks.force_release as i64),
                ),
                ("missile", ProfileValue::Int(profile.callbacks.missile as i64)),
                ("follow", maybe(profile.callbacks.follow)),
                ("think", ProfileValue::Int(profile.callbacks.think as i64)),
                ("pull", ProfileValue::Int(profile.callbacks.pull as i64)),
                ("moveMoverHooks", maybe(profile.callbacks.move_mover_hooks)),
                ("damage", ProfileValue::Int(profile.callbacks.damage as i64)),
                ("sameTeam", ProfileValue::Int(profile.callbacks.same_team as i64)),
                ("playerMove", ProfileValue::Int(profile.callbacks.player_move as i64)),
            ]),
        ),
        ("pullingFlag", ProfileValue::Int(i64::from(profile.pulling_flag))),
        (
            "fireArguments",
            ProfileValue::Array(
                profile
                    .fire_arguments
                    .iter()
                    .map(|value| ProfileValue::Int(i64::from(*value)))
                    .collect(),
            ),
        ),
        (
            "movement",
            ProfileValue::record(vec![
                ("byteLength", ProfileValue::Int(profile.movement.byte_length as i64)),
                (
                    "words",
                    ProfileValue::Array(
                        profile
                            .movement
                            .words
                            .iter()
                            .map(|word| {
                                ProfileValue::record(vec![
                                    ("offset", ProfileValue::Int(word.offset as i64)),
                                    ("value", ProfileValue::Int(i64::from(word.value))),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        ),
        (
            "initialCvars",
            ProfileValue::Record(
                profile
                    .initial_cvars
                    .iter()
                    .map(|(name, value)| (name.clone(), ProfileValue::Str(value.clone())))
                    .collect(),
            ),
        ),
        (
            "eventLifetimeMilliseconds",
            ProfileValue::Int(profile.event_lifetime_ms),
        ),
        ("grappleDamageMethod", ProfileValue::Int(profile.grapple_damage_method)),
        (
            "presentation",
            ProfileValue::record(vec![
                (
                    "projectileModel",
                    ProfileValue::Str(profile.presentation.projectile_model.clone()),
                ),
                ("viewModel", ProfileValue::Str(profile.presentation.view_model.clone())),
                ("weaponIndex", ProfileValue::Int(profile.presentation.weapon_index)),
                (
                    "viewAnchor",
                    ProfileValue::record(vec![
                        ("path", ProfileValue::Str(profile.presentation.view_anchor.path.clone())),
                        ("tag", ProfileValue::Str(profile.presentation.view_anchor.tag.clone())),
                        (
                            "offset",
                            ProfileValue::record(vec![
                                (
                                    "x",
                                    ProfileValue::Float(f64::from(profile.presentation.view_anchor.offset.x)),
                                ),
                                (
                                    "y",
                                    ProfileValue::Float(f64::from(profile.presentation.view_anchor.offset.y)),
                                ),
                                (
                                    "z",
                                    ProfileValue::Float(f64::from(profile.presentation.view_anchor.offset.z)),
                                ),
                            ]),
                        ),
                        (
                            "fovOffset",
                            ProfileValue::record(vec![
                                ("above", ProfileValue::Int(profile.presentation.view_anchor.fov_above)),
                                ("scale", ProfileValue::Float(profile.presentation.view_anchor.fov_scale)),
                            ]),
                        ),
                    ]),
                ),
                (
                    "viewAttachments",
                    ProfileValue::Array(
                        profile
                            .presentation
                            .view_attachments
                            .iter()
                            .map(|attachment| {
                                ProfileValue::record(vec![
                                    ("path", ProfileValue::Str(attachment.path.clone())),
                                    ("tag", ProfileValue::Str(attachment.tag.clone())),
                                ])
                            })
                            .collect(),
                    ),
                ),
                (
                    "cable",
                    match &profile.presentation.cable {
                        QvmCable::Shader { path, width } => ProfileValue::record(vec![
                            ("kind", ProfileValue::Str("shader".to_string())),
                            ("path", ProfileValue::Str(path.clone())),
                            ("width", ProfileValue::Int(*width)),
                        ]),
                        QvmCable::Model {
                            flight,
                            pull,
                            hold,
                            segment_length,
                        } => ProfileValue::record(vec![
                            ("kind", ProfileValue::Str("model".to_string())),
                            ("flight", ProfileValue::Str(flight.clone())),
                            ("pull", ProfileValue::Str(pull.clone())),
                            ("hold", ProfileValue::Str(hold.clone())),
                            ("segmentLength", ProfileValue::Int(*segment_length)),
                        ]),
                    },
                ),
                ("fireSound", maybe_str(&profile.presentation.fire_sound)),
                ("attachSound", maybe_str(&profile.presentation.attach_sound)),
                ("releaseSound", maybe_str(&profile.presentation.release_sound)),
                ("pullSound", maybe_str(&profile.presentation.pull_sound)),
                ("hangSound", maybe_str(&profile.presentation.hang_sound)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::super::game_data::{ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole};
    use super::*;

    fn artifact() -> QvmArtifact {
        let mut image = QvmImage::default();
        image.instructions = (0..32)
            .map(|index| QvmInstruction::word(QvmOpcode::OpEnter, 0, index * 8))
            .collect();
        image.data_length = 4096;
        image.allocated_data_length = 65536;
        QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        }
    }

    fn value() -> ProfileValue {
        let entries = |base: i64| {
            [
                "allocate",
                "free",
                "fire",
                "release",
                "forceRelease",
                "missile",
                "think",
                "pull",
                "damage",
                "sameTeam",
                "playerMove",
            ]
            .into_iter()
            .enumerate()
            .map(|(index, name)| (name, ProfileValue::Int(base + index as i64)))
            .chain([("follow", ProfileValue::Null), ("moveMoverHooks", ProfileValue::Null)])
            .map(|(name, value)| (name.to_string(), value))
            .collect::<Vec<_>>()
        };
        ProfileValue::record(vec![
            ("version", ProfileValue::Int(1)),
            ("artifactDigest", ProfileValue::Str("test".to_string())),
            ("artifactPath", ProfileValue::Str("test".to_string())),
            ("abiProfile", ProfileValue::Str("q3-modern".to_string())),
            ("id", ProfileValue::Str("grapple".to_string())),
            ("title", ProfileValue::Str("Grapple".to_string())),
            ("entityStride", ProfileValue::Int(512)),
            ("clientStride", ProfileValue::Int(512)),
            (
                "fields",
                ProfileValue::record(vec![
                    ("inuse", ProfileValue::Int(208)),
                    ("client", ProfileValue::Int(212)),
                    ("parent", ProfileValue::Int(216)),
                    ("target", ProfileValue::Int(220)),
                    ("mover", ProfileValue::Null),
                    ("hook", ProfileValue::Int(468)),
                    ("health", ProfileValue::Int(224)),
                    ("takedamage", ProfileValue::Int(228)),
                    ("eventTime", ProfileValue::Int(232)),
                    ("freeAfterEvent", ProfileValue::Int(236)),
                ]),
            ),
            (
                "globals",
                ProfileValue::record(vec![
                    ("time", ProfileValue::Int(64)),
                    ("frame", ProfileValue::Int(68)),
                    ("movement", ProfileValue::Int(72)),
                    ("forward", ProfileValue::Int(76)),
                    ("groundPlane", ProfileValue::Int(88)),
                ]),
            ),
            ("callbacks", ProfileValue::Record(entries(1))),
            ("pullingFlag", ProfileValue::Int(64)),
            ("fireArguments", ProfileValue::Array(vec![ProfileValue::Int(7)])),
            (
                "movement",
                ProfileValue::record(vec![
                    ("byteLength", ProfileValue::Int(16)),
                    (
                        "words",
                        ProfileValue::Array(vec![ProfileValue::record(vec![
                            ("offset", ProfileValue::Int(4)),
                            ("value", ProfileValue::Int(9)),
                        ])]),
                    ),
                ]),
            ),
            (
                "initialCvars",
                ProfileValue::record(vec![("g_grapple", ProfileValue::Str("1".to_string()))]),
            ),
            ("eventLifetimeMilliseconds", ProfileValue::Int(500)),
            ("grappleDamageMethod", ProfileValue::Int(3)),
            (
                "presentation",
                ProfileValue::record(vec![
                    ("projectileModel", ProfileValue::Str("hook".to_string())),
                    ("viewModel", ProfileValue::Str("hands".to_string())),
                    ("weaponIndex", ProfileValue::Int(10)),
                    (
                        "viewAnchor",
                        ProfileValue::record(vec![
                            ("path", ProfileValue::Str("view".to_string())),
                            ("tag", ProfileValue::Str("tag".to_string())),
                            (
                                "offset",
                                ProfileValue::record(vec![
                                    ("x", ProfileValue::Float(1.0)),
                                    ("y", ProfileValue::Float(2.0)),
                                    ("z", ProfileValue::Float(3.0)),
                                ]),
                            ),
                            (
                                "fovOffset",
                                ProfileValue::record(vec![
                                    ("above", ProfileValue::Int(4)),
                                    ("scale", ProfileValue::Float(1.5)),
                                ]),
                            ),
                        ]),
                    ),
                    ("viewAttachments", ProfileValue::Array(Vec::new())),
                    (
                        "cable",
                        ProfileValue::record(vec![
                            ("kind", ProfileValue::Str("shader".to_string())),
                            ("path", ProfileValue::Str("cable".to_string())),
                            ("width", ProfileValue::Int(2)),
                        ]),
                    ),
                    ("fireSound", ProfileValue::Null),
                    ("attachSound", ProfileValue::Str("attach".to_string())),
                    ("releaseSound", ProfileValue::Null),
                    ("pullSound", ProfileValue::Null),
                    ("hangSound", ProfileValue::Null),
                ]),
            ),
        ])
    }

    #[test]
    fn profile_parses_and_round_trips() {
        let artifact = artifact();
        let parsed = read_qvm_grapple_profile(&ProfileReader::new(&value()), &artifact).unwrap();
        assert_eq!(parsed.id, "grapple");
        assert_eq!(parsed.fields.mover, None);
        assert_eq!(parsed.callbacks.follow, None);
        assert_eq!(parsed.fire_arguments, vec![7]);
        assert_eq!(parsed.pulling_flag, 64);
        assert_eq!(parsed.presentation.view_anchor.offset, vec3(1.0, 2.0, 3.0));
        let declaration = qvm_grapple_profile_declaration(&parsed);
        let again = read_qvm_grapple_profile(&ProfileReader::new(&declaration), &artifact).unwrap();
        assert_eq!(parsed, again);
    }

    #[test]
    fn profile_rejects_mismatches() {
        let artifact = artifact();
        let read = |mutate: &dyn Fn(&mut ProfileValue)| {
            let mut root = value();
            mutate(&mut root);
            read_qvm_grapple_profile(&ProfileReader::new(&root), &artifact).is_err()
        };
        assert!(read(&|root| {
            if let ProfileValue::Record(fields) = root {
                fields.iter_mut().find(|(name, _)| name == "version").unwrap().1 = ProfileValue::Int(2);
            }
        }));
        assert!(read(&|root| {
            if let ProfileValue::Record(fields) = root {
                fields.iter_mut().find(|(name, _)| name == "pullingFlag").unwrap().1 = ProfileValue::Int(96);
            }
        }));
        assert!(read(&|root| {
            if let ProfileValue::Record(fields) = root {
                if let Some((_, ProfileValue::Record(globals))) = fields.iter_mut().find(|(name, _)| name == "globals")
                {
                    globals.iter_mut().find(|(name, _)| name == "time").unwrap().1 = ProfileValue::Int(4096);
                }
            }
        }));
        let mut legacy = artifact.clone();
        legacy.role = QvmRole::Cgame;
        assert!(read_qvm_grapple_profile(&ProfileReader::new(&value()), &legacy).is_err());
    }
}
