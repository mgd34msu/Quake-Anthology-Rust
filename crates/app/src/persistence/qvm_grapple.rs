//! QVM grapple definition ported from `src/persistence/qvm-grapple.ts`.
//!
//! Author-declared grapple layout for a Q3 QVM artifact; the loader
//! subsequently validates the declaration against the mounted
//! executable, so this module owns wire shape only.

use qa_core::math::Vec3;
use qa_guest::checkpoint::{read_module, write_module, ModuleIdentity};
use qa_world::save::shared::{read_vector, write_vector};
use qa_world::save::value::{arr, int, num, obj, str, SaveJson, SaveReader};

use super::PersistenceError;

/// Grapple entity fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrappleFields {
    /// In-use offset.
    pub inuse: i64,
    /// Client offset.
    pub client: i64,
    /// Parent offset.
    pub parent: i64,
    /// Target offset.
    pub target: i64,
    /// Mover offset.
    pub mover: Option<i64>,
    /// Hook offset.
    pub hook: i64,
    /// Health offset.
    pub health: i64,
    /// Take-damage offset.
    pub takedamage: i64,
    /// Event-time offset.
    pub event_time: i64,
    /// Free-after-event offset.
    pub free_after_event: i64,
}

/// Grapple globals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrappleGlobals {
    /// Time.
    pub time: i64,
    /// Frame.
    pub frame: i64,
    /// Movement.
    pub movement: i64,
    /// Forward.
    pub forward: i64,
    /// Ground plane.
    pub ground_plane: i64,
}

/// Grapple callbacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmGrappleCallbacks {
    /// Allocate.
    pub allocate: i64,
    /// Free.
    pub free: i64,
    /// Fire.
    pub fire: i64,
    /// Release.
    pub release: i64,
    /// Force release.
    pub force_release: i64,
    /// Missile.
    pub missile: i64,
    /// Follow.
    pub follow: Option<i64>,
    /// Think.
    pub think: i64,
    /// Pull.
    pub pull: i64,
    /// Move mover hooks.
    pub move_mover_hooks: Option<i64>,
    /// Damage.
    pub damage: i64,
    /// Same team.
    pub same_team: i64,
    /// Player move.
    pub player_move: i64,
}

/// Grapple cable presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QvmGrappleCable {
    /// Shader cable.
    Shader {
        /// Path.
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

/// Grapple view anchor.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleAnchor {
    /// Path.
    pub path: String,
    /// Tag.
    pub tag: String,
    /// Offset.
    pub offset: Vec3,
    /// Above.
    pub above: i64,
    /// Scale.
    pub scale: f64,
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
    pub view_anchor: QvmGrappleAnchor,
    /// View attachments.
    pub view_attachments: Vec<(String, String)>,
    /// Cable.
    pub cable: QvmGrappleCable,
    /// Fire sound.
    pub fire_sound: Option<String>,
    /// Attach sound.
    pub attach_sound: Option<String>,
    /// Release sound.
    pub release_sound: Option<String>,
    /// Pull sound.
    pub pull_sound: Option<String>,
    /// Hang sound.
    pub hang_sound: Option<String>,
}

/// QVM grapple definition.
#[derive(Debug, Clone, PartialEq)]
pub struct QvmGrappleDefinition {
    /// Id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Module.
    pub module: ModuleIdentity,
    /// ABI profile.
    pub abi_profile: String,
    /// Entity stride.
    pub entity_stride: i64,
    /// Client stride.
    pub client_stride: i64,
    /// Pulling flag.
    pub pulling_flag: i64,
    /// Fields.
    pub fields: QvmGrappleFields,
    /// Globals.
    pub globals: QvmGrappleGlobals,
    /// Callbacks.
    pub callbacks: QvmGrappleCallbacks,
    /// Fire arguments.
    pub fire_arguments: Vec<i64>,
    /// Movement byte length.
    pub movement_byte_length: i64,
    /// Movement words.
    pub movement_words: Vec<(i64, i64)>,
    /// Initial cvars.
    pub initial_cvars: Vec<(String, String)>,
    /// Event lifetime milliseconds.
    pub event_lifetime_milliseconds: i64,
    /// Grapple damage method.
    pub grapple_damage_method: i64,
    /// Presentation.
    pub presentation: QvmGrapplePresentation,
}

fn integer(reader: SaveReader, minimum: i64) -> Result<i64, PersistenceError> {
    Ok(reader.integer(minimum)?)
}

/// Read a QVM grapple definition.
pub fn read_qvm_grapple_definition(reader: SaveReader) -> Result<QvmGrappleDefinition, PersistenceError> {
    let fields = reader.field("fields");
    let globals = reader.field("globals");
    let callbacks = reader.field("callbacks");
    let movement = reader.field("movement");
    let cvars = reader.field("initialCvars");
    let presentation = reader.field("presentation");
    let cable = presentation.field("cable");
    let anchor = presentation.field("viewAnchor");
    let cable_kind = cable.field("kind").choice_str(&["shader", "model"])?;
    let cvar_value = cvars.value;
    let initial_cvars = match &cvar_value {
        Some(SaveJson::Object(members)) => members
            .iter()
            .map(|(name, _)| Ok((name.clone(), cvars.field(name).string()?)))
            .collect::<Result<Vec<_>, PersistenceError>>()?,
        _ => return Err(PersistenceError::from(cvars.fail("Expected source settings"))),
    };
    let sound = |name: &str| {
        presentation
            .field(name)
            .nullable(|value| value.string().map_err(PersistenceError::from))
    };
    Ok(QvmGrappleDefinition {
        id: reader.field("id").string()?,
        title: reader.field("title").string()?,
        module: read_module(reader.field("module"))?,
        abi_profile: reader.field("abiProfile").choice_str(&["q3-modern", "q3-1.16n-base"])?,
        entity_stride: integer(reader.field("entityStride"), 1)?,
        client_stride: integer(reader.field("clientStride"), 1)?,
        pulling_flag: integer(reader.field("pullingFlag"), 1)?,
        fields: QvmGrappleFields {
            inuse: integer(fields.field("inuse"), 0)?,
            client: integer(fields.field("client"), 0)?,
            parent: integer(fields.field("parent"), 0)?,
            target: integer(fields.field("target"), 0)?,
            mover: fields
                .field("mover")
                .nullable(|value| value.integer(0).map_err(PersistenceError::from))?,
            hook: integer(fields.field("hook"), 0)?,
            health: integer(fields.field("health"), 0)?,
            takedamage: integer(fields.field("takedamage"), 0)?,
            event_time: integer(fields.field("eventTime"), 0)?,
            free_after_event: integer(fields.field("freeAfterEvent"), 0)?,
        },
        globals: QvmGrappleGlobals {
            time: integer(globals.field("time"), 0)?,
            frame: integer(globals.field("frame"), 0)?,
            movement: integer(globals.field("movement"), 0)?,
            forward: integer(globals.field("forward"), 0)?,
            ground_plane: integer(globals.field("groundPlane"), 0)?,
        },
        callbacks: QvmGrappleCallbacks {
            allocate: integer(callbacks.field("allocate"), 1)?,
            free: integer(callbacks.field("free"), 1)?,
            fire: integer(callbacks.field("fire"), 1)?,
            release: integer(callbacks.field("release"), 1)?,
            force_release: integer(callbacks.field("forceRelease"), 1)?,
            missile: integer(callbacks.field("missile"), 1)?,
            follow: callbacks
                .field("follow")
                .nullable(|value| value.integer(1).map_err(PersistenceError::from))?,
            think: integer(callbacks.field("think"), 1)?,
            pull: integer(callbacks.field("pull"), 1)?,
            move_mover_hooks: callbacks
                .field("moveMoverHooks")
                .nullable(|value| value.integer(1).map_err(PersistenceError::from))?,
            damage: integer(callbacks.field("damage"), 1)?,
            same_team: integer(callbacks.field("sameTeam"), 1)?,
            player_move: integer(callbacks.field("playerMove"), 1)?,
        },
        fire_arguments: reader
            .field("fireArguments")
            .list(|value| value.integer(i64::MIN).map_err(PersistenceError::from))?,
        movement_byte_length: integer(movement.field("byteLength"), 4)?,
        movement_words: movement
            .field("words")
            .list(|value| -> Result<(i64, i64), PersistenceError> {
                Ok((
                    value.field("offset").integer(4)?,
                    value.field("value").integer(i64::MIN)?,
                ))
            })?,
        initial_cvars,
        event_lifetime_milliseconds: integer(reader.field("eventLifetimeMilliseconds"), 1)?,
        grapple_damage_method: integer(reader.field("grappleDamageMethod"), 0)?,
        presentation: QvmGrapplePresentation {
            projectile_model: presentation.field("projectileModel").string()?,
            view_model: presentation.field("viewModel").string()?,
            weapon_index: integer(presentation.field("weaponIndex"), 1)?,
            view_anchor: QvmGrappleAnchor {
                path: anchor.field("path").string()?,
                tag: anchor.field("tag").string()?,
                offset: read_vector(anchor.field("offset"))?,
                above: anchor.field("fovOffset").field("above").integer(1)?,
                scale: anchor.field("fovOffset").field("scale").finite()?,
            },
            view_attachments: presentation.field("viewAttachments").list(
                |entry| -> Result<(String, String), PersistenceError> {
                    Ok((entry.field("path").string()?, entry.field("tag").string()?))
                },
            )?,
            cable: if cable_kind == "shader" {
                QvmGrappleCable::Shader {
                    path: cable.field("path").string()?,
                    width: cable.field("width").integer(1)?,
                }
            } else {
                QvmGrappleCable::Model {
                    flight: cable.field("flight").string()?,
                    pull: cable.field("pull").string()?,
                    hold: cable.field("hold").string()?,
                    segment_length: cable.field("segmentLength").integer(1)?,
                }
            },
            fire_sound: sound("fireSound")?,
            attach_sound: sound("attachSound")?,
            release_sound: sound("releaseSound")?,
            pull_sound: sound("pullSound")?,
            hang_sound: sound("hangSound")?,
        },
    })
}

/// Write a QVM grapple definition.
#[must_use]
pub fn write_qvm_grapple_definition(definition: &QvmGrappleDefinition) -> SaveJson {
    let sound = |value: &Option<String>| value.as_ref().map_or(SaveJson::Null, |text| str(text));
    obj(vec![
        ("id", str(&definition.id)),
        ("title", str(&definition.title)),
        ("module", write_module(&definition.module)),
        ("abiProfile", str(&definition.abi_profile)),
        ("entityStride", int(definition.entity_stride)),
        ("clientStride", int(definition.client_stride)),
        ("pullingFlag", int(definition.pulling_flag)),
        (
            "fields",
            obj(vec![
                ("inuse", int(definition.fields.inuse)),
                ("client", int(definition.fields.client)),
                ("parent", int(definition.fields.parent)),
                ("target", int(definition.fields.target)),
                ("mover", definition.fields.mover.map_or(SaveJson::Null, int)),
                ("hook", int(definition.fields.hook)),
                ("health", int(definition.fields.health)),
                ("takedamage", int(definition.fields.takedamage)),
                ("eventTime", int(definition.fields.event_time)),
                ("freeAfterEvent", int(definition.fields.free_after_event)),
            ]),
        ),
        (
            "globals",
            obj(vec![
                ("time", int(definition.globals.time)),
                ("frame", int(definition.globals.frame)),
                ("movement", int(definition.globals.movement)),
                ("forward", int(definition.globals.forward)),
                ("groundPlane", int(definition.globals.ground_plane)),
            ]),
        ),
        (
            "callbacks",
            obj(vec![
                ("allocate", int(definition.callbacks.allocate)),
                ("free", int(definition.callbacks.free)),
                ("fire", int(definition.callbacks.fire)),
                ("release", int(definition.callbacks.release)),
                ("forceRelease", int(definition.callbacks.force_release)),
                ("missile", int(definition.callbacks.missile)),
                ("follow", definition.callbacks.follow.map_or(SaveJson::Null, int)),
                ("think", int(definition.callbacks.think)),
                ("pull", int(definition.callbacks.pull)),
                (
                    "moveMoverHooks",
                    definition.callbacks.move_mover_hooks.map_or(SaveJson::Null, int),
                ),
                ("damage", int(definition.callbacks.damage)),
                ("sameTeam", int(definition.callbacks.same_team)),
                ("playerMove", int(definition.callbacks.player_move)),
            ]),
        ),
        (
            "fireArguments",
            arr(definition.fire_arguments.iter().map(|value| int(*value)).collect()),
        ),
        (
            "movement",
            obj(vec![
                ("byteLength", int(definition.movement_byte_length)),
                (
                    "words",
                    arr(definition
                        .movement_words
                        .iter()
                        .map(|(offset, value)| obj(vec![("offset", int(*offset)), ("value", int(*value))]))
                        .collect()),
                ),
            ]),
        ),
        (
            "initialCvars",
            obj(definition
                .initial_cvars
                .iter()
                .map(|(name, value)| (name.as_str(), str(value)))
                .collect()),
        ),
        ("eventLifetimeMilliseconds", int(definition.event_lifetime_milliseconds)),
        ("grappleDamageMethod", int(definition.grapple_damage_method)),
        (
            "presentation",
            obj(vec![
                ("projectileModel", str(&definition.presentation.projectile_model)),
                ("viewModel", str(&definition.presentation.view_model)),
                ("weaponIndex", int(definition.presentation.weapon_index)),
                (
                    "viewAnchor",
                    obj(vec![
                        ("path", str(&definition.presentation.view_anchor.path)),
                        ("tag", str(&definition.presentation.view_anchor.tag)),
                        ("offset", write_vector(definition.presentation.view_anchor.offset)),
                        (
                            "fovOffset",
                            obj(vec![
                                ("above", int(definition.presentation.view_anchor.above)),
                                ("scale", num(definition.presentation.view_anchor.scale)),
                            ]),
                        ),
                    ]),
                ),
                (
                    "viewAttachments",
                    arr(definition
                        .presentation
                        .view_attachments
                        .iter()
                        .map(|(path, tag)| obj(vec![("path", str(path)), ("tag", str(tag))]))
                        .collect()),
                ),
                (
                    "cable",
                    match &definition.presentation.cable {
                        QvmGrappleCable::Shader { path, width } => obj(vec![
                            ("kind", str("shader")),
                            ("path", str(path)),
                            ("width", int(*width)),
                        ]),
                        QvmGrappleCable::Model {
                            flight,
                            pull,
                            hold,
                            segment_length,
                        } => obj(vec![
                            ("kind", str("model")),
                            ("flight", str(flight)),
                            ("pull", str(pull)),
                            ("hold", str(hold)),
                            ("segmentLength", int(*segment_length)),
                        ]),
                    },
                ),
                ("fireSound", sound(&definition.presentation.fire_sound)),
                ("attachSound", sound(&definition.presentation.attach_sound)),
                ("releaseSound", sound(&definition.presentation.release_sound)),
                ("pullSound", sound(&definition.presentation.pull_sound)),
                ("hangSound", sound(&definition.presentation.hang_sound)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::Vec3;
    use qa_guest::checkpoint::ModuleIdentity;
    use qa_world::save::value::{decode_checkpoint_value, encode_checkpoint_value};

    fn module() -> ModuleIdentity {
        ModuleIdentity {
            id: "q3:grapple".to_string(),
            artifact_path: "grapple.qvm".to_string(),
            digest: format!("sha256:{}", "0".repeat(64)),
            revision: "1".to_string(),
        }
    }

    fn sample() -> QvmGrappleDefinition {
        QvmGrappleDefinition {
            id: "q3:grapple".to_string(),
            title: "Grapple".to_string(),
            module: module(),
            abi_profile: "q3-modern".to_string(),
            entity_stride: 1024,
            client_stride: 512,
            pulling_flag: 1,
            fields: QvmGrappleFields {
                inuse: 0,
                client: 8,
                parent: 16,
                target: 24,
                mover: None,
                hook: 32,
                health: 40,
                takedamage: 48,
                event_time: 56,
                free_after_event: 64,
            },
            globals: QvmGrappleGlobals {
                time: 0,
                frame: 4,
                movement: 8,
                forward: 12,
                ground_plane: 16,
            },
            callbacks: QvmGrappleCallbacks {
                allocate: 1,
                free: 2,
                fire: 3,
                release: 4,
                force_release: 5,
                missile: 6,
                follow: None,
                think: 7,
                pull: 8,
                move_mover_hooks: None,
                damage: 9,
                same_team: 10,
                player_move: 11,
            },
            fire_arguments: vec![0, 1],
            movement_byte_length: 64,
            movement_words: vec![(4, 1)],
            initial_cvars: vec![("g_grapple".to_string(), "1".to_string())],
            event_lifetime_milliseconds: 2000,
            grapple_damage_method: 0,
            presentation: QvmGrapplePresentation {
                projectile_model: "models/grapple.md3".to_string(),
                view_model: "models/v_grapple.md3".to_string(),
                weapon_index: 10,
                view_anchor: QvmGrappleAnchor {
                    path: "models/v_grapple.md3".to_string(),
                    tag: "tag_hook".to_string(),
                    offset: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    above: 8,
                    scale: 1.0,
                },
                view_attachments: Vec::new(),
                cable: QvmGrappleCable::Shader {
                    path: "gfx/cable".to_string(),
                    width: 2,
                },
                fire_sound: Some("sound/grapple/fire.wav".to_string()),
                attach_sound: None,
                release_sound: None,
                pull_sound: None,
                hang_sound: None,
            },
        }
    }

    #[test]
    fn grapple_round_trip() {
        let definition = sample();
        let json = write_qvm_grapple_definition(&definition);
        let back = decode_checkpoint_value(&encode_checkpoint_value(&json)).unwrap();
        assert_eq!(
            read_qvm_grapple_definition(SaveReader::at(&back, "g")).unwrap(),
            definition
        );
    }
}
