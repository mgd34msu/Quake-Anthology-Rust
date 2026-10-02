//! Unified presentation frame codec.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-frame-codec.ts`
//! (`encodeUnifiedFrame`, `readUnifiedFrame`, `decodeUnifiedFrame`,
//! `UnifiedResourceKey`, `UnifiedFrameDecoder`).
//!
//! Only public presentation state enters the wire; server model bytes and
//! filesystem provenance never do. The envelope carries a schema literal and
//! a version so an old epoch can be discarded before resolving any
//! resources. Asynchronous donor resolution runs synchronously with order
//! preserved. Scene and snapshot shapes reuse
//! [`super::unified_types`]; value readers reuse
//! [`super::unified_frame_values`]; identity uses
//! [`UnifiedIdentityDecoder`].

use qa_content::contract::ContentId;
use qa_world::save::records::{read_inventory_entry as read_world_inventory_entry, write_inventory_entry};
use qa_world::save::shared::{
    read_character, read_content_id, read_digest, read_frame, read_provider_ref, read_time, write_character,
    write_frame, write_provider_ref, write_time,
};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str as json_str,
    SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::unified_components::{
    read_component_frames, write_component_frames, write_component_owner, UnifiedComponentError, UnifiedComponentFrames,
};
use super::unified_event_codec::{read_unified_simulation_event, write_unified_simulation_event, UnifiedEventError};
use super::unified_frame_values::{
    read_actor, read_axis, read_character_view, read_color, read_model, read_native_camera_view, read_player_ui,
    read_player_view, read_vector, read_world_text, wire_actor, write_character_view, write_color,
    write_native_camera_view, write_player_ui, write_player_view, write_presentation_model, write_vector,
    write_world_text,
};
use super::unified_prediction::{decode_unified_prediction, encode_unified_prediction, UnifiedPredictionError};
use super::unified_types::{
    UnifiedActorConfiguration, UnifiedBodyState, UnifiedDecodedModel, UnifiedEntityFlags, UnifiedEntityTransform,
    UnifiedFramePlayer, UnifiedIdentityDecoder, UnifiedModelPose, UnifiedNativeCamera, UnifiedOutput,
    UnifiedPresentationFrame, UnifiedSceneAttachment, UnifiedSceneEntity, UnifiedSceneFamily, UnifiedSceneLight,
    UnifiedSceneLightStyle, UnifiedSceneParticle, UnifiedSceneSnapshot, UnifiedSkeletonJoint, UnifiedSnapshot,
    UnifiedSnapshotActor, UnifiedSnapshotBody, UnifiedSnapshotInventory,
};
use crate::persistence::mods::{read_mod_identity, write_mod_identity};
use crate::persistence::recipe::{mount_identity, provenance_mount, ResolvedResourceReference};
use crate::persistence::PersistenceError;

/// Maximum decompressed frame payload in bytes (donor 32 MiB limit).
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
/// Maximum compressed frame payload in bytes (donor 4 MiB channel limit).
pub const MAX_CHANNEL_BYTES: usize = 4 * 1024 * 1024;

/// Unified frame codec failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedFrameError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Persistence failure.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Component failure.
    #[error(transparent)]
    Component(#[from] UnifiedComponentError),
    /// Event failure.
    #[error(transparent)]
    Event(#[from] UnifiedEventError),
    /// Prediction failure.
    #[error(transparent)]
    Prediction(#[from] UnifiedPredictionError),
    /// Frame failure.
    #[error("{0}")]
    Frame(String),
}

/// Resource key carried on the wire (donor `UnifiedResourceKey`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedResourceKey {
    /// Content identity.
    pub content: ContentId,
    /// Resource path.
    pub path: String,
    /// Byte digest.
    pub digest: String,
    /// Byte length.
    pub byte_length: u64,
}

/// Frame decoder (donor `UnifiedFrameDecoder`).
pub trait UnifiedFrameDecoder: UnifiedIdentityDecoder {
    /// Negotiated world, when locally loaded.
    fn world(&self) -> Option<super::unified_types::UnifiedSceneWorld>;
    /// Resolve a resource key.
    fn resource(&self, key: &UnifiedResourceKey) -> Result<ResolvedResourceReference, UnifiedFrameError>;
    /// Decode a model for a key and optional brush number.
    fn model(
        &self,
        key: &UnifiedResourceKey,
        brush_model: Option<i64>,
    ) -> Result<UnifiedDecodedModel, UnifiedFrameError>;
}

fn resource_key(resource: &ResolvedResourceReference) -> UnifiedResourceKey {
    UnifiedResourceKey {
        content: ContentId(mount_identity(provenance_mount(&resource.provenance)).content.clone()),
        path: resource.requested_path.clone(),
        digest: resource.digest.clone(),
        byte_length: resource.byte_length,
    }
}

fn valid_resource_path(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') || path.starts_with('/') || path.contains('\0') {
        return false;
    }
    let mut chars = path.chars();
    if matches!(chars.next(), Some(first) if first.is_ascii_alphabetic()) && chars.next() == Some(':') {
        return false;
    }
    !path
        .split('/')
        .any(|part| part == ".." || part == "." || part.is_empty())
}

fn read_key(reader: SaveReader) -> Result<UnifiedResourceKey, WorldError> {
    let path = reader.field("path").string()?;
    if !valid_resource_path(&path) {
        return Err(reader.fail("invalid relative resource path"));
    }
    Ok(UnifiedResourceKey {
        content: ContentId(read_content_id(reader.field("content"))?),
        path,
        digest: read_digest(reader.field("digest"))?,
        byte_length: u64::try_from(reader.field("byteLength").integer(0)?)
            .map_err(|_| reader.field("byteLength").fail("resource length exceeds its range"))?,
    })
}

fn write_key(key: &UnifiedResourceKey) -> SaveJson {
    obj(vec![
        ("content", json_str(key.content.as_str())),
        ("path", json_str(&key.path)),
        ("digest", json_str(&key.digest)),
        ("byteLength", int(key.byte_length as i64)),
    ])
}

fn match_resource(key: &UnifiedResourceKey, resource: &ResolvedResourceReference) -> Result<(), UnifiedFrameError> {
    let resolved = resource_key(resource);
    if resolved != *key {
        return Err(UnifiedFrameError::Frame(
            "Unified resource differs from locally resolved content".to_string(),
        ));
    }
    Ok(())
}

fn read_pose(reader: SaveReader) -> Result<UnifiedModelPose, WorldError> {
    let kind = reader.field("kind").choice_str(&["frame", "skeleton"])?;
    if kind == "frame" {
        Ok(UnifiedModelPose::Frame {
            frame: reader.field("frame").integer(i64::MIN)? as f64,
            previous_frame: reader.field("previousFrame").integer(i64::MIN)? as f64,
            back_lerp: reader.field("backLerp").finite()?,
        })
    } else {
        Ok(UnifiedModelPose::Skeleton {
            joints: reader.field("joints").list(|joint| {
                Ok(UnifiedSkeletonJoint {
                    position: read_vector(joint.field("position"))?,
                    orientation: read_color(joint.field("orientation"))?,
                    scale: joint.field("scale").finite()?,
                })
            })?,
        })
    }
}

fn write_pose(pose: &UnifiedModelPose) -> SaveJson {
    match pose {
        UnifiedModelPose::Frame {
            frame,
            previous_frame,
            back_lerp,
        } => obj(vec![
            ("kind", json_str("frame")),
            ("frame", num(*frame)),
            ("previousFrame", num(*previous_frame)),
            ("backLerp", num(*back_lerp)),
        ]),
        UnifiedModelPose::Skeleton { joints } => obj(vec![
            ("kind", json_str("skeleton")),
            (
                "joints",
                arr(joints
                    .iter()
                    .map(|joint| {
                        obj(vec![
                            ("position", write_vector(joint.position)),
                            ("orientation", write_color(joint.orientation)),
                            ("scale", num(joint.scale)),
                        ])
                    })
                    .collect()),
            ),
        ]),
    }
}

fn read_entity(
    reader: SaveReader,
    context: &impl UnifiedFrameDecoder,
    depth: u32,
) -> Result<UnifiedSceneEntity, UnifiedFrameError> {
    if depth > 32 {
        return Err(reader.fail("scene attachment nesting exceeds limit").into());
    }
    let key = read_key(reader.field("resource"))?;
    let resource = context.resource(&key)?;
    match_resource(&key, &resource)?;
    let brush_model = reader.field("brushModel").nullable(|value| value.integer(0))?;
    let model = context.model(&key, brush_model)?;
    if brush_model.is_some()
        && !matches!(&model, UnifiedDecodedModel::BrushModel { model: number, world }
            if Some(*number) == brush_model && Some(world) == context.world().as_ref().map(|world| &world.geometry))
    {
        return Err(reader.fail("brush model differs from local world").into());
    }
    if brush_model.is_none() && matches!(model, UnifiedDecodedModel::BrushModel { .. }) {
        return Err(reader.fail("unexpected brush model").into());
    }
    let transform = reader.field("transform");
    let flags = reader.field("flags");
    let opacity = reader.field("opacity");
    let attachments = reader.field("attachments").list(|attachment| {
        Ok::<_, UnifiedFrameError>(UnifiedSceneAttachment {
            tag: attachment.field("tag").string()?,
            entity: Box::new(read_entity(attachment.field("entity"), context, depth + 1)?),
        })
    })?;
    Ok(UnifiedSceneEntity {
        actor: reader.field("actor").nullable(|actor| read_actor(actor, context))?,
        resource,
        model,
        transform: UnifiedEntityTransform {
            origin: read_vector(transform.field("origin"))?,
            axis: read_axis(transform.field("axis"))?,
            scale: read_vector(transform.field("scale"))?,
        },
        previous_origin: read_vector(reader.field("previousOrigin"))?,
        pose: read_pose(reader.field("pose"))?,
        skin: reader.field("skin").integer(i64::MIN)? as f64,
        color: read_color(reader.field("color"))?,
        shader_time: read_time(reader.field("shaderTime"))?,
        flags: UnifiedEntityFlags {
            family: match flags.field("kind").choice_str(&["q1", "q2", "q3"])?.as_str() {
                "q1" => UnifiedSceneFamily::Q1,
                "q2" => UnifiedSceneFamily::Q2,
                _ => UnifiedSceneFamily::Q3,
            },
            bits: flags.field("bits").integer(i64::MIN)? as f64,
        },
        lighting_origin: read_vector(reader.field("lightingOrigin"))?,
        shadow_plane: reader.field("shadowPlane").finite()?,
        opacity: if opacity.value.is_none() {
            None
        } else {
            Some(opacity.finite()?)
        },
        attachments,
    })
}

fn write_entity(entity: &UnifiedSceneEntity) -> SaveJson {
    let mut members = vec![
        ("actor", entity.actor.as_ref().map_or(SaveJson::Null, wire_actor)),
        ("resource", write_key(&resource_key(&entity.resource))),
        (
            "brushModel",
            match &entity.model {
                UnifiedDecodedModel::BrushModel { model, .. } => int(*model),
                UnifiedDecodedModel::Other => SaveJson::Null,
            },
        ),
        (
            "transform",
            obj(vec![
                ("origin", write_vector(entity.transform.origin)),
                (
                    "axis",
                    arr(entity.transform.axis.iter().map(|axis| write_vector(*axis)).collect()),
                ),
                ("scale", write_vector(entity.transform.scale)),
            ]),
        ),
        ("previousOrigin", write_vector(entity.previous_origin)),
        ("pose", write_pose(&entity.pose)),
        ("skin", num(entity.skin)),
        ("color", write_color(entity.color)),
        ("shaderTime", write_time(entity.shader_time)),
        (
            "flags",
            obj(vec![
                (
                    "kind",
                    json_str(match entity.flags.family {
                        UnifiedSceneFamily::Q1 => "q1",
                        UnifiedSceneFamily::Q2 => "q2",
                        UnifiedSceneFamily::Q3 => "q3",
                    }),
                ),
                ("bits", num(entity.flags.bits)),
            ]),
        ),
        ("lightingOrigin", write_vector(entity.lighting_origin)),
        ("shadowPlane", num(entity.shadow_plane)),
        (
            "attachments",
            arr(entity
                .attachments
                .iter()
                .map(|attachment| {
                    obj(vec![
                        ("tag", json_str(&attachment.tag)),
                        ("entity", write_entity(&attachment.entity)),
                    ])
                })
                .collect()),
        ),
    ];
    if let Some(opacity) = entity.opacity {
        members.push(("opacity", num(opacity)));
    }
    obj(members)
}

fn read_light(reader: SaveReader) -> Result<UnifiedSceneLight, WorldError> {
    use super::unified_types::{UnifiedLightCone, UnifiedLightProfile, UnifiedLightShadow};
    let profile = reader.field("profile");
    let kind = profile.field("kind").choice_str(&["q1", "q2", "q3"])?;
    let base = UnifiedSceneLight {
        origin: read_vector(reader.field("origin"))?,
        color: read_vector(reader.field("color"))?,
        radius: reader.field("radius").finite()?,
        additive: reader.field("additive").boolean()?,
        profile: match kind.as_str() {
            "q1" => UnifiedLightProfile::Q1,
            "q3" => UnifiedLightProfile::Q3,
            _ => {
                let shadow_field = profile.field("shadow");
                let shadow_kind = shadow_field.field("kind").choice_str(&["none", "cast"])?;
                UnifiedLightProfile::Q2 {
                    scale: profile.field("scale").finite()?,
                    cone: profile.field("cone").nullable(|cone| {
                        Ok(UnifiedLightCone {
                            direction: read_vector(cone.field("direction"))?,
                            cos_half_angle: cone.field("cosHalfAngle").finite()?,
                        })
                    })?,
                    shadow: if shadow_kind == "none" {
                        UnifiedLightShadow::None
                    } else {
                        UnifiedLightShadow::Cast {
                            resolution: shadow_field.field("resolution").integer(1)? as f64,
                        }
                    },
                }
            }
        },
    };
    Ok(base)
}

fn write_light(light: &UnifiedSceneLight) -> SaveJson {
    use super::unified_types::{UnifiedLightProfile, UnifiedLightShadow};
    obj(vec![
        ("origin", write_vector(light.origin)),
        ("color", write_vector(light.color)),
        ("radius", num(light.radius)),
        ("additive", boolean(light.additive)),
        (
            "profile",
            match &light.profile {
                UnifiedLightProfile::Q1 => obj(vec![("kind", json_str("q1"))]),
                UnifiedLightProfile::Q3 => obj(vec![("kind", json_str("q3"))]),
                UnifiedLightProfile::Q2 { scale, cone, shadow } => obj(vec![
                    ("kind", json_str("q2")),
                    ("scale", num(*scale)),
                    (
                        "cone",
                        cone.as_ref().map_or(SaveJson::Null, |cone| {
                            obj(vec![
                                ("direction", write_vector(cone.direction)),
                                ("cosHalfAngle", num(cone.cos_half_angle)),
                            ])
                        }),
                    ),
                    (
                        "shadow",
                        match shadow {
                            UnifiedLightShadow::None => obj(vec![("kind", json_str("none"))]),
                            UnifiedLightShadow::Cast { resolution } => {
                                obj(vec![("kind", json_str("cast")), ("resolution", num(*resolution))])
                            }
                        },
                    ),
                ]),
            },
        ),
    ])
}

fn read_particle(reader: SaveReader) -> Result<UnifiedSceneParticle, WorldError> {
    let kind = reader.field("kind").choice_str(&["indexed", "rgba"])?;
    let origin = read_vector(reader.field("origin"))?;
    let size = reader.field("size").finite()?;
    if kind == "indexed" {
        Ok(UnifiedSceneParticle::Indexed {
            origin,
            size,
            palette_index: reader.field("paletteIndex").integer(0)? as f64,
            alpha: reader.field("alpha").finite()?,
        })
    } else {
        Ok(UnifiedSceneParticle::Rgba {
            origin,
            size,
            color: read_color(reader.field("color"))?,
            rotation: reader.field("rotation").finite()?,
        })
    }
}

fn write_particle(particle: &UnifiedSceneParticle) -> SaveJson {
    match particle {
        UnifiedSceneParticle::Indexed {
            origin,
            size,
            palette_index,
            alpha,
        } => obj(vec![
            ("kind", json_str("indexed")),
            ("origin", write_vector(*origin)),
            ("size", num(*size)),
            ("paletteIndex", num(*palette_index)),
            ("alpha", num(*alpha)),
        ]),
        UnifiedSceneParticle::Rgba {
            origin,
            size,
            color,
            rotation,
        } => obj(vec![
            ("kind", json_str("rgba")),
            ("origin", write_vector(*origin)),
            ("size", num(*size)),
            ("color", write_color(*color)),
            ("rotation", num(*rotation)),
        ]),
    }
}

fn read_style(reader: SaveReader) -> Result<UnifiedSceneLightStyle, WorldError> {
    let kind = reader.field("kind").choice_str(&["q1", "q2"])?;
    let style = reader.field("style").integer(0)? as f64;
    if kind == "q1" {
        Ok(UnifiedSceneLightStyle::Q1 {
            style,
            value: reader.field("value").finite()?,
        })
    } else {
        Ok(UnifiedSceneLightStyle::Q2 {
            style,
            rgb: read_vector(reader.field("rgb"))?,
            white: reader.field("white").finite()?,
        })
    }
}

fn write_style(style: &UnifiedSceneLightStyle) -> SaveJson {
    match style {
        UnifiedSceneLightStyle::Q1 { style, value } => obj(vec![
            ("kind", json_str("q1")),
            ("style", num(*style)),
            ("value", num(*value)),
        ]),
        UnifiedSceneLightStyle::Q2 { style, rgb, white } => obj(vec![
            ("kind", json_str("q2")),
            ("style", num(*style)),
            ("rgb", write_vector(*rgb)),
            ("white", num(*white)),
        ]),
    }
}

/// Encode public presentation state only (donor `encodeUnifiedFrame`).
pub fn encode_unified_frame(frame: &UnifiedPresentationFrame) -> Result<Vec<u8>, UnifiedFrameError> {
    let snapshot = &frame.output.snapshot;
    let scene = &snapshot.scene;
    let mut members = vec![("schema", json_str("qts-unified-frame")), ("version", int(9))];
    if let Some(camera) = frame.native_camera.as_ref() {
        members.push((
            "nativeCamera",
            obj(vec![
                ("owner", write_component_owner(&camera.owner)),
                ("identity", write_mod_identity(&camera.identity)),
                ("generation", int(camera.generation as i64)),
                ("view", write_native_camera_view(&camera.view)),
            ]),
        ));
    }
    let components = frame.components.as_ref().map_or_else(
        || {
            write_component_frames(&UnifiedComponentFrames {
                revision: 0,
                sources: Vec::new(),
                native: None,
            })
        },
        write_component_frames,
    )?;
    members.push(("components", components));
    members.push(("epoch", int(frame.epoch as i64)));
    members.push(("acknowledgedInput", int(frame.acknowledged_input)));
    members.push((
        "prediction",
        SaveJson::Bytes(encode_unified_prediction(&frame.prediction)),
    ));
    members.push((
        "output",
        obj(vec![
            (
                "snapshot",
                obj(vec![
                    ("frame", write_frame(snapshot.frame)),
                    (
                        "actors",
                        arr(snapshot
                            .actors
                            .iter()
                            .map(|actor| {
                                obj(vec![
                                    ("id", wire_actor(&actor.id)),
                                    ("owner", json_str(&actor.owner)),
                                    ("definition", json_str(&actor.definition)),
                                ])
                            })
                            .collect()),
                    ),
                    (
                        "bodies",
                        arr(snapshot
                            .bodies
                            .iter()
                            .map(|body| {
                                obj(vec![
                                    ("actor", wire_actor(&body.actor)),
                                    (
                                        "body",
                                        obj(vec![
                                            ("origin", write_vector(body.body.origin)),
                                            ("angles", write_vector(body.body.angles)),
                                            ("velocity", write_vector(body.body.velocity)),
                                            (
                                                "bounds",
                                                obj(vec![
                                                    ("min", write_vector(body.body.bounds.min)),
                                                    ("max", write_vector(body.body.bounds.max)),
                                                ]),
                                            ),
                                            ("ground", body.body.ground.as_ref().map_or(SaveJson::Null, wire_actor)),
                                        ]),
                                    ),
                                ])
                            })
                            .collect()),
                    ),
                    (
                        "inventories",
                        arr(snapshot
                            .inventories
                            .iter()
                            .map(|inventory| {
                                obj(vec![
                                    ("actor", wire_actor(&inventory.actor)),
                                    (
                                        "entries",
                                        arr(inventory.entries.iter().map(write_inventory_entry).collect()),
                                    ),
                                ])
                            })
                            .collect()),
                    ),
                    (
                        "configurations",
                        arr(snapshot
                            .configurations
                            .iter()
                            .map(|configuration| {
                                obj(vec![
                                    ("actor", wire_actor(&configuration.actor)),
                                    ("movement", write_provider_ref(&configuration.movement)),
                                    ("character", write_character(&configuration.character)),
                                    (
                                        "weapons",
                                        arr(configuration.weapons.iter().map(write_provider_ref).collect()),
                                    ),
                                    ("inventory", write_provider_ref(&configuration.inventory)),
                                ])
                            })
                            .collect()),
                    ),
                    (
                        "scene",
                        obj(vec![
                            ("time", write_time(scene.time)),
                            (
                                "world",
                                scene
                                    .world
                                    .as_ref()
                                    .map_or(SaveJson::Null, |world| write_key(&resource_key(&world.resource))),
                            ),
                            ("entities", arr(scene.entities.iter().map(write_entity).collect())),
                            ("lights", arr(scene.lights.iter().map(write_light).collect())),
                            ("particles", arr(scene.particles.iter().map(write_particle).collect())),
                            ("lightStyles", arr(scene.light_styles.iter().map(write_style).collect())),
                            (
                                "areaBits",
                                scene
                                    .area_bits
                                    .as_ref()
                                    .map_or(SaveJson::Null, |bits| SaveJson::Bytes(bits.clone())),
                            ),
                        ]),
                    ),
                ]),
            ),
            (
                "events",
                arr(frame
                    .output
                    .events
                    .iter()
                    .map(write_unified_simulation_event)
                    .collect::<Result<_, _>>()?),
            ),
        ]),
    ));
    members.push((
        "models",
        arr(frame.models.iter().map(write_presentation_model).collect()),
    ));
    members.push((
        "characters",
        arr(frame.characters.iter().map(write_character_view).collect()),
    ));
    members.push(("worldText", arr(frame.texts.iter().map(write_world_text).collect())));
    members.push((
        "player",
        obj(vec![
            ("actor", wire_actor(&frame.player.actor)),
            ("view", write_player_view(&frame.player.view)),
            ("ui", write_player_ui(&frame.player.ui)),
        ]),
    ));
    let value = encode_checkpoint_value(&obj(members));
    if value.len() > MAX_FRAME_BYTES {
        return Err(UnifiedFrameError::Frame("Unified frame exceeds byte limit".to_string()));
    }
    Ok(deflate_frame(&value))
}

/// Decompress a raw-deflate frame payload.
pub fn inflate_frame(bytes: &[u8]) -> Result<Vec<u8>, UnifiedFrameError> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;
    let decoder = DeflateDecoder::new(bytes);
    let mut output = Vec::new();
    decoder
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|_| UnifiedFrameError::Frame("Unified frame is not raw deflate".to_string()))?;
    if output.len() > MAX_FRAME_BYTES {
        return Err(UnifiedFrameError::Frame("Unified frame exceeds byte limit".to_string()));
    }
    Ok(output)
}

/// Compress a frame payload with raw deflate at level 1.
pub fn deflate_frame(bytes: &[u8]) -> Vec<u8> {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(1));
    encoder.write_all(bytes).expect("deflate encoding cannot fail");
    encoder.finish().expect("deflate encoding cannot fail")
}

/// Parsed frame envelope (donor `readUnifiedFrame` output).
pub struct UnifiedFrameEnvelope {
    /// World epoch, readable before resolving resources.
    pub epoch: u64,
    value: SaveJson,
}

impl UnifiedFrameEnvelope {
    /// Resolve all resources into the candidate world (donor `decode` closure).
    pub fn decode(&self, context: &impl UnifiedFrameDecoder) -> Result<UnifiedPresentationFrame, UnifiedFrameError> {
        decode_frame_value(SaveReader::new(&self.value), context)
    }
}

/// Parse once so an old epoch can be discarded before resolving any resources.
pub fn read_unified_frame(bytes: &[u8]) -> Result<UnifiedFrameEnvelope, UnifiedFrameError> {
    if bytes.len() > MAX_CHANNEL_BYTES {
        return Err(UnifiedFrameError::Frame(
            "Unified frame exceeds channel byte limit".to_string(),
        ));
    }
    let value = decode_checkpoint_value(&inflate_frame(bytes)?)?;
    let reader = SaveReader::new(&value);
    reader.field("schema").literal_str("qts-unified-frame")?;
    let version = reader.field("version").integer(0)?;
    if !(2..=9).contains(&version) {
        return Err(reader.field("version").fail("unsupported unified frame version").into());
    }
    Ok(UnifiedFrameEnvelope {
        epoch: u64::try_from(reader.field("epoch").integer(1)?)
            .map_err(|_| reader.field("epoch").fail("frame epoch exceeds its range"))?,
        value,
    })
}

/// Decode a frame (donor `decodeUnifiedFrame`).
pub fn decode_unified_frame(
    bytes: &[u8],
    context: &impl UnifiedFrameDecoder,
) -> Result<UnifiedPresentationFrame, UnifiedFrameError> {
    let envelope = read_unified_frame(bytes)?;
    envelope.decode(context)
}

fn decode_frame_value(
    reader: SaveReader,
    context: &impl UnifiedFrameDecoder,
) -> Result<UnifiedPresentationFrame, UnifiedFrameError> {
    let output = reader.field("output");
    let snapshot = output.field("snapshot");
    let scene = snapshot.field("scene");
    let world_field = scene.field("world");
    let world_absent = world_field.value.is_none_or(|value| matches!(value, SaveJson::Null));
    if world_absent {
        if context.world().is_some() {
            return Err(world_field.fail("missing negotiated world").into());
        }
    } else if let Some(world) = context.world() {
        match_resource(&read_key(world_field)?, &world.resource)?;
    } else {
        return Err(world_field.fail("world is not locally loaded").into());
    }
    let entities = scene.field("entities").list(|entity| read_entity(entity, context, 0))?;
    let player = reader.field("player");
    let camera = reader.field("nativeCamera");
    let native_camera = if camera.value.is_none() {
        None
    } else {
        Some(UnifiedNativeCamera {
            owner: super::unified_components::read_component_owner(camera.field("owner"))?,
            identity: read_mod_identity(camera.field("identity"))?,
            generation: u64::try_from(camera.field("generation").integer(0)?)
                .map_err(|_| camera.field("generation").fail("camera generation exceeds its range"))?,
            view: read_native_camera_view(camera.field("view"))?,
        })
    };
    let session = context.session().name().to_string();
    let components_field = reader.field("components");
    Ok(UnifiedPresentationFrame {
        epoch: u64::try_from(reader.field("epoch").integer(0)?)
            .map_err(|_| reader.field("epoch").fail("frame epoch exceeds its range"))?,
        native_camera,
        components: if components_field.value.is_none() {
            None
        } else {
            Some(read_component_frames(components_field, context)?)
        },
        acknowledged_input: reader.field("acknowledgedInput").integer(-1)?,
        prediction: decode_unified_prediction(&reader.field("prediction").bytes()?, context)?,
        output: UnifiedOutput {
            snapshot: UnifiedSnapshot {
                session: session.clone(),
                frame: read_frame(snapshot.field("frame"))?,
                actors: snapshot.field("actors").list(|actor| {
                    Ok::<_, UnifiedFrameError>(UnifiedSnapshotActor {
                        id: read_actor(actor.field("id"), context)?,
                        owner: namespaced(actor.field("owner"))?,
                        definition: namespaced(actor.field("definition"))?,
                    })
                })?,
                bodies: snapshot.field("bodies").list(|body| {
                    let state = body.field("body");
                    let bounds = state.field("bounds");
                    Ok::<_, UnifiedFrameError>(UnifiedSnapshotBody {
                        actor: read_actor(body.field("actor"), context)?,
                        body: UnifiedBodyState {
                            origin: read_vector(state.field("origin"))?,
                            angles: read_vector(state.field("angles"))?,
                            velocity: read_vector(state.field("velocity"))?,
                            bounds: qa_core::math::Bounds {
                                min: read_vector(bounds.field("min"))?,
                                max: read_vector(bounds.field("max"))?,
                            },
                            ground: state.field("ground").nullable(|ground| read_actor(ground, context))?,
                        },
                    })
                })?,
                inventories: snapshot.field("inventories").list(|inventory| {
                    Ok::<_, UnifiedFrameError>(UnifiedSnapshotInventory {
                        actor: read_actor(inventory.field("actor"), context)?,
                        entries: inventory.field("entries").list(read_world_inventory_entry)?,
                    })
                })?,
                configurations: snapshot.field("configurations").list(|configuration| {
                    Ok::<_, UnifiedFrameError>(UnifiedActorConfiguration {
                        actor: read_actor(configuration.field("actor"), context)?,
                        movement: read_provider_ref(configuration.field("movement"))?,
                        character: read_character(configuration.field("character"))?,
                        weapons: configuration.field("weapons").list(read_provider_ref)?,
                        inventory: read_provider_ref(configuration.field("inventory"))?,
                    })
                })?,
                scene: UnifiedSceneSnapshot {
                    session,
                    time: read_time(scene.field("time"))?,
                    world: context.world(),
                    entities,
                    lights: scene.field("lights").list(read_light)?,
                    particles: scene.field("particles").list(read_particle)?,
                    light_styles: scene.field("lightStyles").list(read_style)?,
                    area_bits: scene.field("areaBits").nullable(|bits| bits.bytes())?,
                },
            },
            events: output
                .field("events")
                .list(|event| read_unified_simulation_event(event, context))?,
        },
        models: reader.field("models").list(|model| read_model(model, context))?,
        characters: reader
            .field("characters")
            .list(|character| read_character_view(character, context))?,
        texts: reader.field("worldText").list(read_world_text)?,
        player: UnifiedFramePlayer {
            actor: read_actor(player.field("actor"), context)?,
            view: read_player_view(player.field("view"))?,
            ui: read_player_ui(player.field("ui"))?,
        },
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use qa_content::contract::{ArmorState, PoweredProtectionState, RegularArmorState};
    use qa_core::identity::{ClientId, IdentityOwner, SeatId, SessionId};
    use qa_core::math::{Bounds, Vec3, Vec4};
    use qa_core::numeric::{Arithmetic, FloatToInt};
    use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};

    use super::super::types::{ArsenalWarning, PlayerView};
    use super::super::unified_prediction::{
        Q1MovementParameters, Q1NetquakeEdition, Q1NetquakeMovement, UnifiedActorAnimation, UnifiedAnimationState,
        UnifiedMovementProfile, UnifiedMovementState, UnifiedPredictionEnvironment, UnifiedPredictionProjection,
        UnifiedProfileKind, UnifiedTraceHit, UnifiedWeaponState,
    };
    use crate::persistence::recipe::fixture_recipe;
    use qa_world::save::shared::SavedNumericProfile;

    pub(crate) struct Ledger {
        owner: IdentityOwner,
        world: Option<super::super::unified_types::UnifiedSceneWorld>,
        resource: ResolvedResourceReference,
        model: UnifiedDecodedModel,
    }

    impl UnifiedIdentityDecoder for Ledger {
        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
        fn actor(&self, slot: u32, generation: u32) -> qa_core::identity::ActorId {
            self.owner.actor(slot, generation)
        }
        fn client(&self, slot: u32, generation: u32) -> ClientId {
            self.owner.client(slot, generation)
        }
        fn seat(&self, index: u32) -> SeatId {
            self.owner.seat(index)
        }
        fn resource_id(&self, id: &str) -> String {
            id.to_string()
        }
    }

    impl UnifiedFrameDecoder for Ledger {
        fn world(&self) -> Option<super::super::unified_types::UnifiedSceneWorld> {
            self.world.clone()
        }
        fn resource(&self, _key: &UnifiedResourceKey) -> Result<ResolvedResourceReference, UnifiedFrameError> {
            Ok(self.resource.clone())
        }
        fn model(
            &self,
            _key: &UnifiedResourceKey,
            _brush_model: Option<i64>,
        ) -> Result<UnifiedDecodedModel, UnifiedFrameError> {
            Ok(self.model.clone())
        }
    }

    pub(crate) fn ledger() -> Ledger {
        let recipe = fixture_recipe();
        Ledger {
            owner: IdentityOwner::create("test").unwrap(),
            world: None,
            resource: recipe.map.geometry.clone(),
            model: UnifiedDecodedModel::Other,
        }
    }

    fn zero3() -> Vec3 {
        Vec3 { x: 0.0, y: 0.0, z: 0.0 }
    }

    fn projection(actor: qa_core::identity::ActorId) -> UnifiedPredictionProjection {
        UnifiedPredictionProjection {
            actor,
            sequence: 3,
            command_time_milliseconds: 50.0,
            state: UnifiedMovementState::Q1Netquake(Q1NetquakeMovement {
                origin: zero3(),
                velocity: zero3(),
                angles: zero3(),
                old_origin: zero3(),
                angular_velocity: zero3(),
                view_angles: zero3(),
                punch_angles: zero3(),
                move_type: 0.0,
                flags: 0.0,
                ground: UnifiedTraceHit::None,
                water_level: 0.0,
                water_type: 0.0,
                teleport_time_seconds: 0.0,
                water_jump_direction: zero3(),
                ideal_pitch: 0.0,
                fix_angle: false,
                health: 100.0,
            }),
            profile: UnifiedMovementProfile {
                id: "q1:netquake".to_string(),
                clock: ClockProfile::Q1Netquake {
                    minimum_frame_seconds: 0.001,
                    maximum_frame_seconds: 0.1,
                    fixed_frame_seconds: None,
                },
                numeric: SavedNumericProfile {
                    id: "q1:numbers".to_string(),
                    arithmetic: Arithmetic::Binary32EachOp,
                    float_to_int: FloatToInt::QvmIndefinite,
                },
                kind: UnifiedProfileKind::Q1Netquake {
                    parameters: Q1MovementParameters {
                        gravity: 800.0,
                        stop_speed: 100.0,
                        max_speed: 320.0,
                        spectator_max_speed: 500.0,
                        accelerate: 10.0,
                        air_accelerate: 1.0,
                        water_accelerate: 4.0,
                        friction: 4.0,
                        water_friction: 1.0,
                        entity_gravity: 1.0,
                    },
                    edition: Q1NetquakeEdition::Classic,
                    edge_friction: 2.0,
                    no_clip_angle_hack: false,
                },
            },
            arsenal: super::super::unified_prediction::UnifiedArsenalState {
                provider: "q1:arsenal".to_string(),
                active_weapon: None,
                state: UnifiedWeaponState::Q1 {
                    frame: 0.0,
                    attack_finished_seconds: 0.0,
                    source_weapon: 1.0,
                },
                ammo: Vec::new(),
            },
            animation: UnifiedActorAnimation {
                provider: "q1:animation".to_string(),
                state: UnifiedAnimationState::Q1 {
                    frame: 0.0,
                    next_frame_seconds: 0.0,
                },
            },
            standing_bounds: Bounds {
                min: zero3(),
                max: zero3(),
            },
            standing_view_height: 22.0,
            bounds: Bounds {
                min: zero3(),
                max: zero3(),
            },
            view_angles: zero3(),
            view_height: 22.0,
            view_offset: zero3(),
            environment: UnifiedPredictionEnvironment {
                health: 100.0,
                flight: false,
                haste: false,
                invulnerable: false,
                gravity_multiplier: 1.0,
                client_outputs: None,
            },
            contact: None,
            collisions: Vec::new(),
        }
    }

    fn view() -> PlayerView {
        PlayerView {
            origin: zero3(),
            angles: zero3(),
            view_height: 22.0,
            blend: None,
            damage_blend: None,
            kick_angles: None,
            field_of_view: None,
            client_view_offset_delta: None,
            foreign_character_death: None,
            pitch_drift: None,
        }
    }

    fn ui() -> super::super::types::PlayerUi {
        super::super::types::PlayerUi {
            selected_arsenal: None,
            native_inventory: None,
            health: 100.0,
            armor: ArmorState {
                regular: RegularArmorState::None,
                powered: PoweredProtectionState::None,
            },
            active_weapon: None,
            ammo: None,
            inventory: Vec::new(),
            arsenal_warning: ArsenalWarning::None,
            powerups: Vec::new(),
            items: Vec::new(),
            weapon_status: None,
        }
    }

    fn entity(resource: &ResolvedResourceReference) -> UnifiedSceneEntity {
        UnifiedSceneEntity {
            actor: None,
            resource: resource.clone(),
            model: UnifiedDecodedModel::Other,
            transform: UnifiedEntityTransform {
                origin: zero3(),
                axis: [zero3(), zero3(), zero3()],
                scale: zero3(),
            },
            previous_origin: zero3(),
            pose: UnifiedModelPose::Frame {
                frame: 1.0,
                previous_frame: 0.0,
                back_lerp: 0.5,
            },
            skin: 0.0,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            shader_time: SourceTime::Seconds(1.0),
            flags: UnifiedEntityFlags {
                family: UnifiedSceneFamily::Q3,
                bits: 0.0,
            },
            lighting_origin: zero3(),
            shadow_plane: 0.0,
            opacity: None,
            attachments: Vec::new(),
        }
    }

    pub(crate) fn frame(ledger: &Ledger) -> UnifiedPresentationFrame {
        let actor = ledger.owner.actor(1, 0);
        let session = ledger.owner.session().name().to_string();
        UnifiedPresentationFrame {
            epoch: 2,
            components: None,
            native_camera: None,
            acknowledged_input: 1,
            prediction: projection(actor.clone()),
            output: UnifiedOutput {
                snapshot: UnifiedSnapshot {
                    session: session.clone(),
                    frame: FrameContext {
                        frame: 7,
                        time: SourceTime::Seconds(1.0),
                        elapsed: SourceTime::Seconds(0.1),
                        phase: FramePhase::FrameEntry,
                    },
                    actors: Vec::new(),
                    bodies: Vec::new(),
                    inventories: Vec::new(),
                    configurations: Vec::new(),
                    scene: UnifiedSceneSnapshot {
                        session,
                        time: SourceTime::Seconds(2.0),
                        world: None,
                        entities: vec![entity(&ledger.resource)],
                        lights: Vec::new(),
                        particles: Vec::new(),
                        light_styles: Vec::new(),
                        area_bits: None,
                    },
                },
                events: Vec::new(),
            },
            models: Vec::new(),
            characters: Vec::new(),
            texts: Vec::new(),
            player: UnifiedFramePlayer {
                actor,
                view: view(),
                ui: ui(),
            },
        }
    }

    #[test]
    fn round_trips_minimal_frame() {
        let ledger = ledger();
        let frame = frame(&ledger);
        let bytes = encode_unified_frame(&frame).unwrap();
        let envelope = read_unified_frame(&bytes).unwrap();
        assert_eq!(envelope.epoch, 2);
        let decoded = envelope.decode(&ledger).unwrap();
        let mut expected = frame;
        expected.components = Some(UnifiedComponentFrames {
            revision: 0,
            sources: Vec::new(),
            native: Some(Vec::new()),
        });
        assert_eq!(decoded, expected);
    }

    #[test]
    fn world_negotiation_rejects_mismatch() {
        let base = ledger();
        let mut frame = frame(&base);
        frame.output.snapshot.scene.world = Some(super::super::unified_types::UnifiedSceneWorld {
            resource: base.resource.clone(),
            geometry: "world".to_string(),
        });
        let bytes = encode_unified_frame(&frame).unwrap();
        assert!(decode_unified_frame(&bytes, &base).is_err());
        let mut loaded = ledger();
        let mut other_resource = base.resource.clone();
        other_resource.requested_path = "maps/q3dm2.bsp".to_string();
        other_resource.digest = format!("sha256:{}", "5".repeat(64));
        loaded.world = Some(super::super::unified_types::UnifiedSceneWorld {
            resource: other_resource,
            geometry: "world".to_string(),
        });
        assert!(decode_unified_frame(&bytes, &loaded).is_err());
        let mut matched = ledger();
        matched.world = Some(super::super::unified_types::UnifiedSceneWorld {
            resource: matched.resource.clone(),
            geometry: "world".to_string(),
        });
        decode_unified_frame(&bytes, &matched).expect("matched world decodes");
    }

    #[test]
    fn brush_checks_reject_mismatch() {
        let catalog = ledger();
        let mut decoded = frame(&catalog);
        decoded.output.snapshot.scene.entities[0].model = UnifiedDecodedModel::BrushModel {
            model: 3,
            world: "other".to_string(),
        };
        let bytes = encode_unified_frame(&decoded).unwrap();
        assert!(decode_unified_frame(&bytes, &catalog).is_err());
        let mut strict = ledger();
        strict.model = UnifiedDecodedModel::BrushModel {
            model: 1,
            world: "world".to_string(),
        };
        let plain = frame(&strict);
        let bytes = encode_unified_frame(&plain).unwrap();
        assert!(decode_unified_frame(&bytes, &strict).is_err());
    }

    #[test]
    fn nesting_limit_rejects_deep_attachments() {
        let ledger = ledger();
        let mut frame = frame(&ledger);
        let mut leaf = entity(&ledger.resource);
        for _ in 0..34 {
            leaf = UnifiedSceneEntity {
                attachments: vec![UnifiedSceneAttachment {
                    tag: "t".to_string(),
                    entity: Box::new(leaf),
                }],
                ..entity(&ledger.resource)
            };
        }
        frame.output.snapshot.scene.entities = vec![leaf];
        let bytes = encode_unified_frame(&frame).unwrap();
        assert!(decode_unified_frame(&bytes, &ledger).is_err());
    }

    #[test]
    fn envelope_rejects_bad_versions() {
        let ledger = ledger();
        let bytes = encode_unified_frame(&frame(&ledger)).unwrap();
        assert!(read_unified_frame(&vec![0u8; MAX_CHANNEL_BYTES + 1]).is_err());
        let mut raw = inflate_frame(&bytes).unwrap();
        assert!(!raw.is_empty());
        raw[0] = raw[0].wrapping_add(1);
        assert!(read_unified_frame(&deflate_frame(&raw)).is_err());
    }
}
