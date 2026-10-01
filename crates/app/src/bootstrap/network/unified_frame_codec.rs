//! Unified frame header and frame codecs.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-frame-codec.ts`
//! (`readUnifiedFrameHeader`, `readUnifiedFrame`, `encodeUnifiedFrame`,
//! `UnifiedResourceKey`, `UnifiedFrameResource`, `UnifiedFrameHeader`).
//!
//! Headers pin the acknowledged frame, world geometry, epoch, resources,
//! and per-family session snapshots; frames carry the compressed snapshot,
//! admitted-player projection, presentation rows, and events. Snapshot,
//! scene, and event shapes decode into the [`super::unified_types`]
//! mirrors with full validation; resource identities reuse the app recipe
//! codec; prediction, components, and events reuse their shard codecs.
//!
//! Per-family session snapshots (ack/input/profile/applied) thread through
//! as raw checkpoint values: their interpretation belongs to the session
//! lane, so the [`UnifiedFrameSessionHost`] interprets on decode and
//! serializes on encode while the codec pins envelope presence and field
//! order. [`UnifiedFrameModelHost`] decodes content models for the
//! brush-model world check, and [`UnifiedFrameContentHost`] supplies the
//! catalog layer snapshot on encode.

use qa_content::contract::ContentId;
use qa_world::save::records::{read_inventory_entry, write_inventory_entry};
use qa_world::save::shared::{
    read_bounds, read_character, read_content_id, read_frame, read_provider_ref, read_time, write_bounds,
    write_character, write_frame, write_provider_ref, write_time,
};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str as json_str,
    SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::unified_components::{
    read_component_frames, write_component_frames, write_component_update, UnifiedComponentFrames,
    UnifiedComponentUpdate,
};
use super::unified_event_codec::{read_unified_simulation_event, write_unified_simulation_event, UnifiedEventError};
use super::unified_frame_values::{
    read_actor, read_axis, read_character_view, read_color, read_model, read_native_camera_view, read_player_ui,
    read_player_view, read_vector, wire_actor, write_character_view, write_color, write_native_camera_view,
    write_player_ui, write_presentation_model, write_vector, write_world_text,
};
use super::unified_prediction::{decode_unified_prediction, encode_unified_prediction, UnifiedPredictionError};
use super::unified_types::{
    UnifiedActorConfiguration, UnifiedBodyState, UnifiedDecodedModel, UnifiedEntityFlags, UnifiedEntityTransform,
    UnifiedFramePlayer, UnifiedIdentityDecoder, UnifiedLightCone, UnifiedLightProfile, UnifiedLightShadow,
    UnifiedModelPose, UnifiedNativeCamera, UnifiedOutput, UnifiedPresentationFrame, UnifiedSceneAttachment,
    UnifiedSceneEntity, UnifiedSceneFamily, UnifiedSceneLight, UnifiedSceneLightStyle, UnifiedSceneParticle,
    UnifiedSceneSnapshot, UnifiedSceneWorld, UnifiedSkeletonJoint, UnifiedSnapshot, UnifiedSnapshotActor,
    UnifiedSnapshotBody, UnifiedSnapshotInventory,
};
use crate::persistence::mods::{read_mod_identity, write_mod_identity};
use crate::persistence::recipe::{read_resource, write_resource, ResolvedResourceReference};
use crate::persistence::PersistenceError;

/// Maximum decompressed frame payload in bytes (donor `MAX_FRAME_BYTES`).
pub const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
/// Maximum list items in one frame payload.
pub const MAX_FRAME_LIST: usize = 65536;

/// Unified frame codec failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedFrameError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Persistence failure.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Event failure.
    #[error(transparent)]
    Event(#[from] UnifiedEventError),
    /// Prediction failure.
    #[error(transparent)]
    Prediction(#[from] UnifiedPredictionError),
    /// Component failure.
    #[error(transparent)]
    Component(#[from] super::unified_components::UnifiedComponentError),
    /// Frame failure.
    #[error("{0}")]
    Frame(String),
}

/// Unified resource key (donor `UnifiedResourceKey`).
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

/// Unified resource identity (donor `UnifiedResourceIdentity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedResourceIdentity {
    /// Content identity.
    pub content: ContentId,
    /// Negotiated resource id.
    pub resource: String,
    /// Parsed reference.
    pub parsed: ResolvedResourceReference,
}

/// Frame resource row (donor `UnifiedFrameResource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedFrameResource {
    /// Resource identity.
    pub identity: UnifiedResourceIdentity,
    /// Byte digest.
    pub digest: String,
    /// Display label.
    pub label: String,
}

/// Raw per-family session snapshots (opaque to the codec; see module docs).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSessionSnapshots {
    /// Acknowledgement state.
    pub ack: SaveJson,
    /// Pending input.
    pub input: SaveJson,
    /// Session profile.
    pub profile: SaveJson,
    /// Applied state (decode only; never re-encoded).
    pub applied: Option<SaveJson>,
}

/// Session family carried by a frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedSessionFamily {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
    /// Classic Quake II.
    Q2Classic,
    /// Rerelease Quake II.
    Q2Rerelease,
    /// Quake III.
    Q3,
}

/// Frame header (donor `UnifiedFrameHeader`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedFrameHeader<Session> {
    /// Acknowledged frame.
    pub frame: i64,
    /// World geometry handle.
    pub world: String,
    /// World epoch.
    pub epoch: u64,
    /// Frame resources.
    pub resources: Vec<UnifiedFrameResource>,
    /// Session family.
    pub family: UnifiedSessionFamily,
    /// Raw session snapshots.
    pub snapshots: UnifiedSessionSnapshots,
    /// Interpreted session.
    pub session: Session,
    /// Frame revision.
    pub frame_revision: i64,
}

/// Host interpreting per-family session snapshots.
pub trait UnifiedFrameSessionHost {
    /// Interpreted session handle.
    type Session;
    /// Error type.
    type Error: Into<UnifiedFrameError>;
    /// Interpret raw snapshots on decode.
    fn interpret_session(
        &self,
        family: UnifiedSessionFamily,
        snapshots: &UnifiedSessionSnapshots,
    ) -> Result<Self::Session, Self::Error>;
    /// Serialize a session for encode (ack/input/profile; applied is never re-encoded).
    fn consume_session(
        &self,
        family: UnifiedSessionFamily,
        session: &Self::Session,
    ) -> Result<UnifiedSessionSnapshots, Self::Error>;
}

/// Host decoding content models for the brush-model world check.
pub trait UnifiedFrameModelHost {
    /// Decode a scene model.
    fn decode_model(&self, resource: &ResolvedResourceReference) -> UnifiedDecodedModel;
}

/// Host supplying the catalog layer snapshot on encode.
pub trait UnifiedFrameContentHost {
    /// Catalog layers value.
    fn content_layers(&self) -> SaveJson;
}

fn bounded_list<T, E>(reader: SaveReader, read: impl FnMut(SaveReader) -> Result<T, E>) -> Result<Vec<T>, E>
where
    E: From<WorldError>,
{
    let items = reader.list(read)?;
    if items.len() > MAX_FRAME_LIST {
        return Err(E::from(reader.fail("invalid frame list length")));
    }
    Ok(items)
}

fn read_resource_identity(reader: SaveReader) -> Result<UnifiedResourceIdentity, UnifiedFrameError> {
    let content = ContentId(read_content_id(reader.field("content"))?);
    let resource = reader.field("resource").string()?;
    if !resource.starts_with("resource:") {
        return Err(reader.fail("expected a resource identity").into());
    }
    let parsed = read_resource(reader.field("parsed"))?;
    if parsed.id.as_str() != resource {
        return Err(reader.fail("resource identity differs from its reference").into());
    }
    if parsed.requested_path.contains('\0') {
        return Err(reader.fail("invalid resource path").into());
    }
    Ok(UnifiedResourceIdentity {
        content,
        resource,
        parsed,
    })
}

fn write_resource_identity(identity: &UnifiedResourceIdentity) -> SaveJson {
    obj(vec![
        ("content", json_str(identity.content.as_str())),
        ("resource", json_str(&identity.resource)),
        ("parsed", write_resource(&identity.parsed)),
    ])
}

fn required_value(reader: &SaveReader, name: &'static str) -> Result<SaveJson, UnifiedFrameError> {
    reader
        .field(name)
        .value
        .cloned()
        .ok_or_else(|| reader.fail("frame header requires session snapshots").into())
}

/// Read a frame header (donor `readUnifiedFrameHeader`).
pub fn read_unified_frame_header<Host: UnifiedFrameSessionHost>(
    reader: SaveReader,
    host: &Host,
) -> Result<UnifiedFrameHeader<Host::Session>, UnifiedFrameError> {
    let session = reader.field("session");
    let kind =
        session
            .field("kind")
            .choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?;
    let family = match kind.as_str() {
        "q1-netquake" => UnifiedSessionFamily::Q1Netquake,
        "q1-quakeworld" => UnifiedSessionFamily::Q1Quakeworld,
        "q2-classic" => UnifiedSessionFamily::Q2Classic,
        "q2-rerelease" => UnifiedSessionFamily::Q2Rerelease,
        _ => UnifiedSessionFamily::Q3,
    };
    let snapshots = UnifiedSessionSnapshots {
        ack: required_value(&session, "ack")?,
        input: required_value(&session, "input")?,
        profile: required_value(&session, "profile")?,
        applied: session.field("applied").value.cloned(),
    };
    if snapshots.applied.is_none() {
        return Err(session.fail("frame header requires session snapshots").into());
    }
    let interpreted = host.interpret_session(family, &snapshots).map_err(Into::into)?;
    let resources = bounded_list(reader.field("resources"), |resource| {
        Ok::<_, UnifiedFrameError>(UnifiedFrameResource {
            identity: read_resource_identity(resource.field("identity"))?,
            digest: resource.field("digest").string()?,
            label: resource.field("label").string()?,
        })
    })?;
    let mut keys = std::collections::HashSet::new();
    for resource in &resources {
        if !keys.insert(resource.identity.resource.clone()) {
            return Err(reader.fail("duplicate frame resource").into());
        }
    }
    Ok(UnifiedFrameHeader {
        frame: reader.field("frame").integer(0)?,
        world: reader.field("world").string()?,
        epoch: reader.field("epoch").integer(0).and_then(|epoch| {
            u64::try_from(epoch).map_err(|_| reader.field("epoch").fail("frame epoch exceeds its range"))
        })?,
        resources,
        family,
        snapshots,
        session: interpreted,
        frame_revision: reader.field("frameRevision").integer(0)?,
    })
}

fn read_scene_family(reader: SaveReader) -> Result<UnifiedSceneFamily, WorldError> {
    match reader.choice_str(&["q1", "q2", "q3"])?.as_str() {
        "q1" => Ok(UnifiedSceneFamily::Q1),
        "q2" => Ok(UnifiedSceneFamily::Q2),
        _ => Ok(UnifiedSceneFamily::Q3),
    }
}

fn write_scene_family(family: UnifiedSceneFamily) -> SaveJson {
    json_str(match family {
        UnifiedSceneFamily::Q1 => "q1",
        UnifiedSceneFamily::Q2 => "q2",
        UnifiedSceneFamily::Q3 => "q3",
    })
}

fn read_scene_entity(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
    models: &ModelLookup,
) -> Result<UnifiedSceneEntity, UnifiedFrameError> {
    let model_ref = reader.field("model");
    let resource = read_resource(model_ref.field("resource"))?;
    let model = models.decode(&resource)?;
    let transform = reader.field("transform");
    let pose = reader.field("pose");
    let pose_kind = pose.field("kind").choice_str(&["frame", "skeleton"])?;
    let flags = reader.field("flags");
    let opacity = reader.field("opacity");
    Ok(UnifiedSceneEntity {
        actor: reader.field("actor").nullable(|actor| read_actor(actor, identity))?,
        resource,
        model,
        transform: UnifiedEntityTransform {
            origin: read_vector(transform.field("origin"))?,
            axis: read_axis(transform.field("axis"))?,
            scale: read_vector(transform.field("scale"))?,
        },
        previous_origin: read_vector(reader.field("previousOrigin"))?,
        pose: if pose_kind == "frame" {
            UnifiedModelPose::Frame {
                frame: pose.field("frame").finite()?,
                previous_frame: pose.field("previousFrame").finite()?,
                back_lerp: pose.field("backLerp").finite()?,
            }
        } else {
            UnifiedModelPose::Skeleton {
                joints: bounded_list(pose.field("joints"), |joint| {
                    Ok::<_, UnifiedFrameError>(UnifiedSkeletonJoint {
                        position: read_vector(joint.field("position"))?,
                        orientation: read_color(joint.field("orientation"))?,
                        scale: joint.field("scale").finite()?,
                    })
                })?,
            }
        },
        skin: reader.field("skin").finite()?,
        color: read_color(reader.field("color"))?,
        shader_time: read_time(reader.field("shaderTime"))?,
        flags: UnifiedEntityFlags {
            family: read_scene_family(flags.field("kind"))?,
            bits: flags.field("bits").finite()?,
        },
        lighting_origin: read_vector(reader.field("lightingOrigin"))?,
        shadow_plane: reader.field("shadowPlane").finite()?,
        opacity: if opacity.value.is_none() {
            None
        } else {
            Some(opacity.finite()?)
        },
        attachments: bounded_list(reader.field("attachments"), |attachment| {
            Ok::<_, UnifiedFrameError>(UnifiedSceneAttachment {
                tag: attachment.field("tag").string()?,
                entity: Box::new(read_scene_entity(attachment.field("entity"), identity, models)?),
            })
        })?,
    })
}

/// Brush-model world check helper.
struct ModelLookup<'a> {
    host: &'a dyn UnifiedFrameModelHost,
    world: &'a str,
}

impl ModelLookup<'_> {
    fn decode(&self, resource: &ResolvedResourceReference) -> Result<UnifiedDecodedModel, UnifiedFrameError> {
        let model = self.host.decode_model(resource);
        if let UnifiedDecodedModel::BrushModel { world, .. } = &model {
            if world != self.world {
                return Err(UnifiedFrameError::Frame(
                    "Brush model belongs to another world".to_string(),
                ));
            }
        }
        Ok(model)
    }
}

fn write_scene_entity(entity: &UnifiedSceneEntity, models: &ModelLookup) -> Result<SaveJson, UnifiedFrameError> {
    let decoded = models.decode(&entity.resource)?;
    let _ = decoded;
    Ok(obj(vec![
        ("actor", entity.actor.as_ref().map_or(SaveJson::Null, wire_actor)),
        ("model", obj(vec![("resource", write_resource(&entity.resource))])),
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
        (
            "pose",
            match &entity.pose {
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
            },
        ),
        ("skin", num(entity.skin)),
        ("color", write_color(entity.color)),
        ("shaderTime", write_time(entity.shader_time)),
        (
            "flags",
            obj(vec![
                ("kind", write_scene_family(entity.flags.family)),
                ("bits", num(entity.flags.bits)),
            ]),
        ),
        ("lightingOrigin", write_vector(entity.lighting_origin)),
        ("shadowPlane", num(entity.shadow_plane)),
        ("opacity", entity.opacity.map_or(SaveJson::Null, num)),
        (
            "attachments",
            arr(entity
                .attachments
                .iter()
                .map(|attachment| {
                    Ok::<_, UnifiedFrameError>(obj(vec![
                        ("tag", json_str(&attachment.tag)),
                        ("entity", write_scene_entity(&attachment.entity, models)?),
                    ]))
                })
                .collect::<Result<_, _>>()?),
        ),
    ]))
}

fn read_scene_light(reader: SaveReader) -> Result<UnifiedSceneLight, UnifiedFrameError> {
    let profile = reader.field("profile");
    let kind = profile.field("kind").choice_str(&["q1", "q2", "q3"])?;
    Ok(UnifiedSceneLight {
        origin: read_vector(reader.field("origin"))?,
        color: read_vector(reader.field("color"))?,
        radius: reader.field("radius").finite()?,
        additive: reader.field("additive").boolean()?,
        profile: match kind.as_str() {
            "q1" => UnifiedLightProfile::Q1,
            "q3" => UnifiedLightProfile::Q3,
            _ => {
                let cone = profile.field("cone");
                let shadow = profile.field("shadow");
                let shadow_kind = shadow.field("kind").choice_str(&["none", "cast"])?;
                UnifiedLightProfile::Q2 {
                    scale: profile.field("scale").finite()?,
                    cone: if cone.value == Some(&SaveJson::Null) {
                        None
                    } else {
                        Some(UnifiedLightCone {
                            direction: read_vector(cone.field("direction"))?,
                            cos_half_angle: cone.field("cosHalfAngle").finite()?,
                        })
                    },
                    shadow: if shadow_kind == "none" {
                        UnifiedLightShadow::None
                    } else {
                        UnifiedLightShadow::Cast {
                            resolution: shadow.field("resolution").finite()?,
                        }
                    },
                }
            }
        },
    })
}

fn write_scene_light(light: &UnifiedSceneLight) -> SaveJson {
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

fn read_scene_particle(reader: SaveReader) -> Result<UnifiedSceneParticle, UnifiedFrameError> {
    match reader.field("kind").choice_str(&["indexed", "rgba"])?.as_str() {
        "indexed" => Ok(UnifiedSceneParticle::Indexed {
            origin: read_vector(reader.field("origin"))?,
            size: reader.field("size").finite()?,
            palette_index: reader.field("paletteIndex").finite()?,
            alpha: reader.field("alpha").finite()?,
        }),
        _ => Ok(UnifiedSceneParticle::Rgba {
            origin: read_vector(reader.field("origin"))?,
            size: reader.field("size").finite()?,
            color: read_color(reader.field("color"))?,
            rotation: reader.field("rotation").finite()?,
        }),
    }
}

fn write_scene_particle(particle: &UnifiedSceneParticle) -> SaveJson {
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

fn read_light_style(reader: SaveReader) -> Result<UnifiedSceneLightStyle, UnifiedFrameError> {
    match reader.field("kind").choice_str(&["q1", "q2"])?.as_str() {
        "q1" => Ok(UnifiedSceneLightStyle::Q1 {
            style: reader.field("style").finite()?,
            value: reader.field("value").finite()?,
        }),
        _ => Ok(UnifiedSceneLightStyle::Q2 {
            style: reader.field("style").finite()?,
            rgb: read_vector(reader.field("rgb"))?,
            white: reader.field("white").finite()?,
        }),
    }
}

fn write_light_style(style: &UnifiedSceneLightStyle) -> SaveJson {
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

fn read_scene_snapshot(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
    models: &ModelLookup,
) -> Result<UnifiedSceneSnapshot, UnifiedFrameError> {
    let world = reader.field("world");
    let area_bits = reader.field("areaBits");
    Ok(UnifiedSceneSnapshot {
        session: read_session(reader.field("session"))?,
        time: read_time(reader.field("time"))?,
        world: if world.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(UnifiedSceneWorld {
                resource: read_resource(world.field("resource"))?,
                geometry: world.field("geometry").string()?,
            })
        },
        entities: bounded_list(reader.field("entities"), |entity| {
            read_scene_entity(entity, identity, models)
        })?,
        lights: bounded_list(reader.field("lights"), read_scene_light)?,
        particles: bounded_list(reader.field("particles"), read_scene_particle)?,
        light_styles: bounded_list(reader.field("lightStyles"), read_light_style)?,
        area_bits: if area_bits.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(area_bits.bytes()?)
        },
    })
}

fn write_scene_snapshot(scene: &UnifiedSceneSnapshot, models: &ModelLookup) -> Result<SaveJson, UnifiedFrameError> {
    Ok(obj(vec![
        ("session", write_session(&scene.session)),
        ("time", write_time(scene.time)),
        (
            "world",
            scene.world.as_ref().map_or(SaveJson::Null, |world| {
                obj(vec![
                    ("resource", write_resource(&world.resource)),
                    ("geometry", json_str(&world.geometry)),
                ])
            }),
        ),
        (
            "entities",
            arr(scene
                .entities
                .iter()
                .map(|entity| write_scene_entity(entity, models))
                .collect::<Result<_, _>>()?),
        ),
        ("lights", arr(scene.lights.iter().map(write_scene_light).collect())),
        (
            "particles",
            arr(scene.particles.iter().map(write_scene_particle).collect()),
        ),
        (
            "lightStyles",
            arr(scene.light_styles.iter().map(write_light_style).collect()),
        ),
        (
            "areaBits",
            scene
                .area_bits
                .as_ref()
                .map_or(SaveJson::Null, |bits| SaveJson::Bytes(bits.clone())),
        ),
    ]))
}

fn read_session(reader: SaveReader) -> Result<String, WorldError> {
    reader.field("name").string()
}

fn write_session(session: &str) -> SaveJson {
    obj(vec![("name", json_str(session))])
}

fn read_snapshot(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
    models: &ModelLookup,
) -> Result<UnifiedSnapshot, UnifiedFrameError> {
    Ok(UnifiedSnapshot {
        session: read_session(reader.field("session"))?,
        frame: read_frame(reader.field("frame"))?,
        actors: bounded_list(reader.field("actors"), |actor| {
            Ok::<_, UnifiedFrameError>(UnifiedSnapshotActor {
                id: read_actor(actor.field("id"), identity)?,
                owner: namespaced(actor.field("owner"))?,
                definition: namespaced(actor.field("definition"))?,
            })
        })?,
        bodies: bounded_list(reader.field("bodies"), |body| {
            let state = body.field("body");
            let bounds = state.field("bounds");
            Ok::<_, UnifiedFrameError>(UnifiedSnapshotBody {
                actor: read_actor(body.field("actor"), identity)?,
                body: UnifiedBodyState {
                    origin: read_vector(state.field("origin"))?,
                    angles: read_vector(state.field("angles"))?,
                    velocity: read_vector(state.field("velocity"))?,
                    bounds: read_bounds(bounds)?,
                    ground: state.field("ground").nullable(|ground| read_actor(ground, identity))?,
                },
            })
        })?,
        inventories: bounded_list(reader.field("inventories"), |inventory| {
            Ok::<_, UnifiedFrameError>(UnifiedSnapshotInventory {
                actor: read_actor(inventory.field("actor"), identity)?,
                entries: bounded_list(inventory.field("entries"), read_inventory_entry)?,
            })
        })?,
        configurations: bounded_list(reader.field("configurations"), |configuration| {
            Ok::<_, UnifiedFrameError>(UnifiedActorConfiguration {
                actor: read_actor(configuration.field("actor"), identity)?,
                movement: read_provider_ref(configuration.field("movement"))?,
                character: read_character(configuration.field("character"))?,
                weapons: bounded_list(configuration.field("weapons"), read_provider_ref)?,
                inventory: read_provider_ref(configuration.field("inventory"))?,
            })
        })?,
        scene: read_scene_snapshot(reader.field("scene"), identity, models)?,
    })
}

fn write_snapshot(snapshot: &UnifiedSnapshot, models: &ModelLookup) -> Result<SaveJson, UnifiedFrameError> {
    Ok(obj(vec![
        ("session", write_session(&snapshot.session)),
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
                                ("bounds", write_bounds(body.body.bounds)),
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
        ("scene", write_scene_snapshot(&snapshot.scene, models)?),
    ]))
}

fn read_output(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
    models: &ModelLookup,
) -> Result<UnifiedOutput, UnifiedFrameError> {
    Ok(UnifiedOutput {
        snapshot: read_snapshot(reader.field("snapshot"), identity, models)?,
        events: bounded_list(reader.field("events"), |event| {
            read_unified_simulation_event(event, identity)
        })?,
    })
}

fn write_output(output: &UnifiedOutput, models: &ModelLookup) -> Result<SaveJson, UnifiedFrameError> {
    Ok(obj(vec![
        ("snapshot", write_snapshot(&output.snapshot, models)?),
        (
            "events",
            arr(output
                .events
                .iter()
                .map(write_unified_simulation_event)
                .collect::<Result<_, _>>()?),
        ),
    ]))
}

/// Decompress a raw-deflate frame payload (donor `deflated`).
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

/// Compress a frame payload with raw deflate.
pub fn deflate_frame(bytes: &[u8]) -> Vec<u8> {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("deflate encoding cannot fail");
    encoder.finish().expect("deflate encoding cannot fail")
}

fn read_frame_player(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedFramePlayer, UnifiedFrameError> {
    Ok(UnifiedFramePlayer {
        actor: read_actor(reader.field("actor"), identity)?,
        view: read_player_view(reader.field("view"))?,
        ui: read_player_ui(reader.field("ui"))?,
    })
}

/// Read a frame (donor `readUnifiedFrame`).
pub fn read_unified_frame<Session>(
    header: &UnifiedFrameHeader<Session>,
    bytes: &[u8],
    identity: &dyn UnifiedIdentityDecoder,
    models: &dyn UnifiedFrameModelHost,
) -> Result<UnifiedPresentationFrame, UnifiedFrameError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(UnifiedFrameError::Frame("Unified frame exceeds byte limit".to_string()));
    }
    let inflated = inflate_frame(bytes)?;
    let value = decode_checkpoint_value(&inflated)?;
    let reader = SaveReader::new(&value);
    reader.field("schema").literal_str("qts-unified-frame")?;
    reader.field("version").literal_i64(1)?;
    if reader.field("frame").integer(0)? != header.frame {
        return Err(reader.fail("frame number differs from its header").into());
    }
    let lookup = ModelLookup {
        host: models,
        world: &header.world,
    };
    let components = reader.field("components");
    let native_camera = reader.field("nativeCamera");
    let player = reader.field("player");
    Ok(UnifiedPresentationFrame {
        epoch: {
            let epoch = reader.field("epoch").integer(0)?;
            u64::try_from(epoch).map_err(|_| reader.field("epoch").fail("frame epoch exceeds its range"))?
        },
        components: if components.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(read_component_frames(components, identity)?)
        },
        native_camera: if native_camera.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(UnifiedNativeCamera {
                owner: super::unified_components::read_component_owner(native_camera.field("owner"))?,
                identity: read_mod_identity(native_camera.field("identity"))?,
                generation: {
                    let generation = native_camera.field("generation").integer(1)?;
                    u64::try_from(generation).map_err(|_| {
                        native_camera
                            .field("generation")
                            .fail("native camera generation exceeds its range")
                    })?
                },
                view: read_native_camera_view(native_camera.field("view"))?,
            })
        },
        acknowledged_input: reader.field("acknowledgedInput").integer(-1)?,
        prediction: decode_unified_prediction(&reader.field("prediction").bytes()?, identity)?,
        output: read_output(reader.field("output"), identity, &lookup)?,
        models: bounded_list(reader.field("models"), |model| read_model(model, identity))?,
        characters: bounded_list(reader.field("characters"), |character| {
            read_character_view(character, identity)
        })?,
        texts: bounded_list(reader.field("texts"), |text| {
            super::unified_frame_values::read_world_text(text)
        })?,
        player: read_frame_player(player, identity)?,
    })
}

/// Frame encode input (donor `encodeUnifiedFrame` value).
pub struct UnifiedFrameEncode<'a, Session> {
    /// Frame number.
    pub frame: i64,
    /// World geometry handle.
    pub world: &'a str,
    /// World epoch.
    pub epoch: u64,
    /// Frame resources.
    pub resources: &'a [UnifiedFrameResource],
    /// Session family.
    pub family: UnifiedSessionFamily,
    /// Session handle.
    pub session: &'a Session,
    /// Frame revision.
    pub frame_revision: i64,
    /// Presentation frame.
    pub presented: &'a UnifiedPresentationFrame,
    /// Reliable component update, when the revision advanced.
    pub update: Option<&'a UnifiedComponentUpdate>,
    /// Component frames revision.
    pub components_revision: i64,
}

/// Encode a frame header plus frame (donor `encodeUnifiedFrame`).
pub fn encode_unified_frame<Host: UnifiedFrameSessionHost>(
    frame: &UnifiedFrameEncode<'_, Host::Session>,
    host: &Host,
    models: &dyn UnifiedFrameModelHost,
    content: &dyn UnifiedFrameContentHost,
) -> Result<(Vec<u8>, Vec<u8>), UnifiedFrameError> {
    let snapshots = host.consume_session(frame.family, frame.session).map_err(Into::into)?;
    let header = obj(vec![
        ("frame", int(frame.frame)),
        ("world", json_str(frame.world)),
        ("epoch", int(frame.epoch as i64)),
        (
            "resources",
            arr(frame
                .resources
                .iter()
                .map(|resource| {
                    obj(vec![
                        ("identity", write_resource_identity(&resource.identity)),
                        ("digest", json_str(&resource.digest)),
                        ("label", json_str(&resource.label)),
                    ])
                })
                .collect()),
        ),
        (
            "session",
            obj(vec![
                (
                    "kind",
                    json_str(match frame.family {
                        UnifiedSessionFamily::Q1Netquake => "q1-netquake",
                        UnifiedSessionFamily::Q1Quakeworld => "q1-quakeworld",
                        UnifiedSessionFamily::Q2Classic => "q2-classic",
                        UnifiedSessionFamily::Q2Rerelease => "q2-rerelease",
                        UnifiedSessionFamily::Q3 => "q3",
                    }),
                ),
                ("ack", snapshots.ack),
                ("input", snapshots.input),
                ("profile", snapshots.profile),
            ]),
        ),
        ("frameRevision", int(frame.frame_revision)),
    ]);
    let lookup = ModelLookup {
        host: models,
        world: frame.world,
    };
    let presented = frame.presented;
    let body = obj(vec![
        ("schema", json_str("qts-unified-frame")),
        ("version", int(1)),
        ("frame", int(frame.frame)),
        ("content", content.content_layers()),
        ("epoch", int(frame.epoch as i64)),
        (
            "components",
            presented.components.as_ref().map_or(Ok(SaveJson::Null), |components| {
                write_component_frames(&UnifiedComponentFrames {
                    revision: frame.components_revision,
                    sources: components.sources.clone(),
                    native: components.native.clone(),
                })
                .map_err(UnifiedFrameError::from)
            })?,
        ),
        (
            "update",
            frame
                .update
                .as_ref()
                .map_or(SaveJson::Null, |update| write_component_update(update)),
        ),
        (
            "nativeCamera",
            presented.native_camera.as_ref().map_or(SaveJson::Null, |camera| {
                obj(vec![
                    (
                        "owner",
                        obj(vec![
                            (
                                "provider",
                                json_str(&format!(
                                    "{}:{}",
                                    camera.owner.provider.namespace, camera.owner.provider.name
                                )),
                            ),
                            ("generation", int(camera.owner.generation as i64)),
                        ]),
                    ),
                    ("identity", write_mod_identity(&camera.identity)),
                    ("generation", int(camera.generation as i64)),
                    ("view", write_native_camera_view(&camera.view)),
                ])
            }),
        ),
        ("acknowledgedInput", int(presented.acknowledged_input)),
        (
            "prediction",
            SaveJson::Bytes(encode_unified_prediction(&presented.prediction)),
        ),
        ("output", write_output(&presented.output, &lookup)?),
        (
            "models",
            arr(presented.models.iter().map(write_presentation_model).collect()),
        ),
        (
            "characters",
            arr(presented.characters.iter().map(write_character_view).collect()),
        ),
        ("texts", arr(presented.texts.iter().map(write_world_text).collect())),
        (
            "player",
            obj(vec![
                ("actor", wire_actor(&presented.player.actor)),
                (
                    "view",
                    super::unified_frame_values::write_player_view(&presented.player.view),
                ),
                ("ui", write_player_ui(&presented.player.ui)),
            ]),
        ),
    ]);
    Ok((
        encode_checkpoint_value(&header),
        deflate_frame(&encode_checkpoint_value(&body)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Host;

    impl UnifiedFrameSessionHost for Host {
        type Session = String;
        type Error = UnifiedFrameError;
        fn interpret_session(
            &self,
            _family: UnifiedSessionFamily,
            _snapshots: &UnifiedSessionSnapshots,
        ) -> Result<String, UnifiedFrameError> {
            Ok("session".to_string())
        }
        fn consume_session(
            &self,
            _family: UnifiedSessionFamily,
            session: &String,
        ) -> Result<UnifiedSessionSnapshots, UnifiedFrameError> {
            Ok(UnifiedSessionSnapshots {
                ack: json_str(session),
                input: SaveJson::Null,
                profile: SaveJson::Null,
                applied: None,
            })
        }
    }

    #[test]
    fn header_rejects_duplicate_resources() {
        let encoded = obj(vec![
            ("frame", int(1)),
            ("world", json_str("world")),
            ("epoch", int(1)),
            ("resources", arr(Vec::new())),
            (
                "session",
                obj(vec![
                    ("kind", json_str("q3")),
                    ("ack", SaveJson::Null),
                    ("input", SaveJson::Null),
                    ("profile", SaveJson::Null),
                    ("applied", SaveJson::Null),
                ]),
            ),
            ("frameRevision", int(1)),
        ]);
        let reader = SaveReader::new(&encoded);
        let header = read_unified_frame_header(reader, &Host).unwrap();
        assert_eq!(header.frame, 1);
        assert_eq!(header.family, UnifiedSessionFamily::Q3);
        assert_eq!(header.session, "session");
    }

    #[test]
    fn header_requires_applied_snapshots() {
        let encoded = obj(vec![
            ("frame", int(1)),
            ("world", json_str("world")),
            ("epoch", int(1)),
            ("resources", arr(Vec::new())),
            (
                "session",
                obj(vec![
                    ("kind", json_str("q3")),
                    ("ack", SaveJson::Null),
                    ("input", SaveJson::Null),
                    ("profile", SaveJson::Null),
                ]),
            ),
            ("frameRevision", int(1)),
        ]);
        let reader = SaveReader::new(&encoded);
        assert!(read_unified_frame_header(reader, &Host).is_err());
    }

    #[test]
    fn frame_deflate_round_trips() {
        let payload = vec![7u8; 1000];
        let compressed = deflate_frame(&payload);
        assert!(compressed.len() < payload.len());
        assert_eq!(inflate_frame(&compressed).unwrap(), payload);
    }
}
