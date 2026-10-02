//! Unified presentation and simulation event codecs.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-event-codec.ts`
//! (`writeUnifiedSimulationEvent`, `readUnifiedSimulationEvent`,
//! `encodeUnifiedPresentationEvents`, `decodeUnifiedPresentationEvents`).
//!
//! The generated donor codec reads and writes every presentation and
//! simulation event through checkpoint values; identities resolve through
//! the client's [`UnifiedIdentityDecoder`](super::unified_types::UnifiedIdentityDecoder).
//! Presentation batches travel as plain JSON (the checkpoint codec renders
//! and parses plain values identically), capped at 65536 events and 16 MiB;
//! simulation events embed as values inside the frame checkpoint. Event
//! shapes are local mirrors carrying exactly the fields the codec reads and
//! writes. QVM player records reuse [`qa_guest`]; armor and inventory rows
//! reuse [`qa_world::save::records`].

use std::collections::BTreeMap;

use qa_core::identity::{ActorId, ClientId, SeatId};
use qa_core::math::{Vec3, Vec4};
use qa_core::time::SourceTime;
use qa_guest::error::GuestError;
use qa_guest::qvm::game_data::AbiProfile;
use qa_guest::qvm::player_record::{
    qvm_player_state_bytes, read_source_qvm_player_state, write_qvm_player_state, QvmPlayerState,
};
use qa_world::combat::ArmorState;
use qa_world::save::records::{read_armor, read_inventory_entry, write_armor, write_inventory_entry};
use qa_world::save::shared::{read_digest, read_time, write_time};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str as json_str,
    SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::unified_frame_values::{read_actor, read_color, read_vector, wire_actor, write_color, write_vector};
use super::unified_types::UnifiedIdentityDecoder;

/// Maximum presentation-event payload in bytes (donor `MAX_EVENT_BYTES`).
pub const MAX_EVENT_BYTES: usize = 16 * 1024 * 1024;
/// Maximum events or list items in one payload.
pub const MAX_EVENT_LIST: usize = 65536;

/// Unified event codec failure.
#[derive(Debug, thiserror::Error)]
pub enum UnifiedEventError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// QVM record failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
    /// Payload exceeds its protocol limit.
    #[error("{0}")]
    Limit(String),
}

fn bounded_list<T, E>(reader: SaveReader, read: impl FnMut(SaveReader) -> Result<T, E>) -> Result<Vec<T>, E>
where
    E: From<WorldError>,
{
    let items = reader.list(read)?;
    if items.len() > MAX_EVENT_LIST {
        return Err(E::from(reader.fail("invalid event list length")));
    }
    Ok(items)
}

fn read_client(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<ClientId, WorldError> {
    let slot = reader.field("slot").integer(0)?;
    let generation = reader.field("generation").integer(0)?;
    let (Ok(slot), Ok(generation)) = (u32::try_from(slot), u32::try_from(generation)) else {
        return Err(reader.fail("client reference exceeds its range"));
    };
    Ok(identity.client(slot, generation))
}

fn write_client(value: &ClientId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(value.slot()))),
        ("generation", int(i64::from(value.generation()))),
    ])
}

fn read_seat(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<SeatId, WorldError> {
    let index = reader.field("index").integer(0)?;
    let Ok(index) = u32::try_from(index) else {
        return Err(reader.fail("seat reference exceeds its range"));
    };
    Ok(identity.seat(index))
}

fn write_seat(value: &SeatId) -> SaveJson {
    obj(vec![("index", int(i64::from(value.index())))])
}

fn read_opt_actor(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Option<ActorId>, WorldError> {
    if reader.value == Some(&SaveJson::Null) {
        Ok(None)
    } else {
        read_actor(reader, identity).map(Some)
    }
}

fn write_opt_actor(value: Option<&ActorId>) -> SaveJson {
    value.map_or(SaveJson::Null, wire_actor)
}

fn read_resource(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<String, WorldError> {
    let value = reader.string()?;
    let valid = value.len() == "resource:unified:".len() + 64
        && value.starts_with("resource:unified:")
        && value["resource:unified:".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    if !valid {
        return Err(reader.fail("expected negotiated unified resource identity"));
    }
    Ok(identity.resource_id(&format!("resource:{}", &value["resource:".len()..])))
}

fn write_resource(value: &str) -> Result<SaveJson, WorldError> {
    let valid = value.len() == "resource:unified:".len() + 64
        && value.starts_with("resource:unified:")
        && value["resource:unified:".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit());
    if !valid {
        return Err(WorldError::BadSave(
            "Unified events require negotiated resource identities".to_string(),
        ));
    }
    Ok(json_str(value))
}

fn read_content(reader: SaveReader) -> Result<String, WorldError> {
    let value = reader.string()?;
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() != 4 || !matches!(parts[0], "q1" | "q2" | "q3") {
        return Err(reader.fail("invalid content identity"));
    }
    Ok(value)
}

fn read_module(reader: SaveReader) -> Result<UnifiedModuleIdentity, WorldError> {
    Ok(UnifiedModuleIdentity {
        id: namespaced(reader.field("id"))?,
        artifact_path: reader.field("artifactPath").string()?,
        digest: read_digest(reader.field("digest"))?,
        revision: reader.field("revision").string()?,
    })
}

fn write_module(module: &UnifiedModuleIdentity) -> SaveJson {
    obj(vec![
        ("id", json_str(&module.id)),
        ("artifactPath", json_str(&module.artifact_path)),
        ("digest", json_str(&module.digest)),
        ("revision", json_str(&module.revision)),
    ])
}

/// Module identity (donor `ModuleIdentity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedModuleIdentity {
    /// Module id.
    pub id: String,
    /// Artifact path.
    pub artifact_path: String,
    /// Artifact digest.
    pub digest: String,
    /// Revision.
    pub revision: String,
}

/// Q3 trajectory (donor `Trajectory`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedTrajectory {
    /// Trajectory type.
    pub trajectory_type: f64,
    /// Start time.
    pub time: f64,
    /// Duration.
    pub duration: f64,
    /// Base point.
    pub base: Vec3,
    /// Delta.
    pub delta: Vec3,
}

fn read_trajectory(reader: SaveReader) -> Result<UnifiedTrajectory, WorldError> {
    Ok(UnifiedTrajectory {
        trajectory_type: reader.field("type").finite()?,
        time: reader.field("time").finite()?,
        duration: reader.field("duration").finite()?,
        base: read_vector(reader.field("base"))?,
        delta: read_vector(reader.field("delta"))?,
    })
}

fn write_trajectory(value: &UnifiedTrajectory) -> SaveJson {
    obj(vec![
        ("type", num(value.trajectory_type)),
        ("time", num(value.time)),
        ("duration", num(value.duration)),
        ("base", write_vector(value.base)),
        ("delta", write_vector(value.delta)),
    ])
}

/// Q3 entity state (donor `EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedQ3EntityState {
    /// Position trajectory.
    pub pos: UnifiedTrajectory,
    /// Angular trajectory.
    pub apos: UnifiedTrajectory,
    /// Origin.
    pub origin: Vec3,
    /// Secondary origin.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Entity number.
    pub number: f64,
    /// Entity type.
    pub e_type: f64,
    /// Entity flags.
    pub e_flags: f64,
    /// Time.
    pub time: f64,
    /// Secondary time.
    pub time2: f64,
    /// Other entity number.
    pub other_entity_num: f64,
    /// Second other entity number.
    pub other_entity_num2: f64,
    /// Ground entity number.
    pub ground_entity_num: f64,
    /// Constant light.
    pub constant_light: f64,
    /// Loop sound.
    pub loop_sound: f64,
    /// Model index.
    pub modelindex: f64,
    /// Second model index.
    pub modelindex2: f64,
    /// Client number.
    pub client_num: f64,
    /// Frame.
    pub frame: f64,
    /// Solid encoding.
    pub solid: f64,
    /// Event.
    pub event: f64,
    /// Event parameter.
    pub event_parm: f64,
    /// Powerups.
    pub powerups: f64,
    /// Weapon.
    pub weapon: f64,
    /// Legs animation.
    pub legs_anim: f64,
    /// Torso animation.
    pub torso_anim: f64,
    /// Generic value.
    pub generic1: f64,
}

fn read_entity_state(reader: SaveReader) -> Result<UnifiedQ3EntityState, WorldError> {
    Ok(UnifiedQ3EntityState {
        pos: read_trajectory(reader.field("pos"))?,
        apos: read_trajectory(reader.field("apos"))?,
        origin: read_vector(reader.field("origin"))?,
        origin2: read_vector(reader.field("origin2"))?,
        angles: read_vector(reader.field("angles"))?,
        angles2: read_vector(reader.field("angles2"))?,
        number: reader.field("number").finite()?,
        e_type: reader.field("eType").finite()?,
        e_flags: reader.field("eFlags").finite()?,
        time: reader.field("time").finite()?,
        time2: reader.field("time2").finite()?,
        other_entity_num: reader.field("otherEntityNum").finite()?,
        other_entity_num2: reader.field("otherEntityNum2").finite()?,
        ground_entity_num: reader.field("groundEntityNum").finite()?,
        constant_light: reader.field("constantLight").finite()?,
        loop_sound: reader.field("loopSound").finite()?,
        modelindex: reader.field("modelindex").finite()?,
        modelindex2: reader.field("modelindex2").finite()?,
        client_num: reader.field("clientNum").finite()?,
        frame: reader.field("frame").finite()?,
        solid: reader.field("solid").finite()?,
        event: reader.field("event").finite()?,
        event_parm: reader.field("eventParm").finite()?,
        powerups: reader.field("powerups").finite()?,
        weapon: reader.field("weapon").finite()?,
        legs_anim: reader.field("legsAnim").finite()?,
        torso_anim: reader.field("torsoAnim").finite()?,
        generic1: reader.field("generic1").finite()?,
    })
}

#[allow(clippy::too_many_lines)]
fn write_entity_state(value: &UnifiedQ3EntityState) -> SaveJson {
    obj(vec![
        ("number", num(value.number)),
        ("eType", num(value.e_type)),
        ("eFlags", num(value.e_flags)),
        ("pos", write_trajectory(&value.pos)),
        ("apos", write_trajectory(&value.apos)),
        ("time", num(value.time)),
        ("time2", num(value.time2)),
        ("origin", write_vector(value.origin)),
        ("origin2", write_vector(value.origin2)),
        ("angles", write_vector(value.angles)),
        ("angles2", write_vector(value.angles2)),
        ("otherEntityNum", num(value.other_entity_num)),
        ("otherEntityNum2", num(value.other_entity_num2)),
        ("groundEntityNum", num(value.ground_entity_num)),
        ("constantLight", num(value.constant_light)),
        ("loopSound", num(value.loop_sound)),
        ("modelindex", num(value.modelindex)),
        ("modelindex2", num(value.modelindex2)),
        ("clientNum", num(value.client_num)),
        ("frame", num(value.frame)),
        ("solid", num(value.solid)),
        ("event", num(value.event)),
        ("eventParm", num(value.event_parm)),
        ("powerups", num(value.powerups)),
        ("weapon", num(value.weapon)),
        ("legsAnim", num(value.legs_anim)),
        ("torsoAnim", num(value.torso_anim)),
        ("generic1", num(value.generic1)),
    ])
}

/// Music event.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedMusicEvent {
    /// CD track selection.
    CdTrack {
        /// Track number.
        track: f64,
    },
    /// Pause state.
    Pause {
        /// Paused flag.
        paused: bool,
    },
}

fn read_music_event(reader: SaveReader) -> Result<UnifiedMusicEvent, WorldError> {
    match reader.field("kind").choice_str(&["cd-track", "pause"])?.as_str() {
        "cd-track" => Ok(UnifiedMusicEvent::CdTrack {
            track: reader.field("track").finite()?,
        }),
        _ => Ok(UnifiedMusicEvent::Pause {
            paused: reader.field("paused").boolean()?,
        }),
    }
}

fn write_music_event(event: &UnifiedMusicEvent) -> SaveJson {
    match event {
        UnifiedMusicEvent::CdTrack { track } => obj(vec![("kind", json_str("cd-track")), ("track", num(*track))]),
        UnifiedMusicEvent::Pause { paused } => obj(vec![("kind", json_str("pause")), ("paused", boolean(*paused))]),
    }
}

/// Q1 named sound channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1NamedChannel {
    /// Automatic channel.
    Auto,
    /// Weapon channel.
    Weapon,
    /// Voice channel.
    Voice,
    /// Item channel.
    Item,
    /// Body channel.
    Body,
}

/// Q1 sound channel (named or numbered -1/5/6/7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1SoundChannel {
    /// Named channel.
    Named(Q1NamedChannel),
    /// Numbered channel.
    Number(i64),
}

fn read_sound_channel(reader: SaveReader) -> Result<Q1SoundChannel, WorldError> {
    match reader.value {
        Some(SaveJson::String(_)) => {
            let channel = reader.choice_str(&["auto", "weapon", "voice", "item", "body"])?;
            Ok(Q1SoundChannel::Named(match channel.as_str() {
                "auto" => Q1NamedChannel::Auto,
                "weapon" => Q1NamedChannel::Weapon,
                "voice" => Q1NamedChannel::Voice,
                "item" => Q1NamedChannel::Item,
                _ => Q1NamedChannel::Body,
            }))
        }
        _ => {
            let channel = reader.choice_i64(&[-1, 5, 6, 7])?;
            Ok(Q1SoundChannel::Number(channel))
        }
    }
}

fn write_sound_channel(channel: Q1SoundChannel) -> SaveJson {
    match channel {
        Q1SoundChannel::Named(named) => json_str(match named {
            Q1NamedChannel::Auto => "auto",
            Q1NamedChannel::Weapon => "weapon",
            Q1NamedChannel::Voice => "voice",
            Q1NamedChannel::Item => "item",
            Q1NamedChannel::Body => "body",
        }),
        Q1SoundChannel::Number(number) => int(number),
    }
}

/// Q1 message argument (string or number).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1MessageArg {
    /// Text argument.
    Text(String),
    /// Numeric argument.
    Number(f64),
}

fn read_message_arg(reader: SaveReader) -> Result<Q1MessageArg, WorldError> {
    match reader.value {
        Some(SaveJson::String(_)) => Ok(Q1MessageArg::Text(reader.string()?)),
        _ => Ok(Q1MessageArg::Number(reader.finite()?)),
    }
}

fn write_message_arg(arg: &Q1MessageArg) -> SaveJson {
    match arg {
        Q1MessageArg::Text(text) => json_str(text),
        Q1MessageArg::Number(number) => num(*number),
    }
}

/// Q1 message part.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MessagePart {
    /// Text.
    pub text: String,
    /// Arguments.
    pub args: Option<Vec<Q1MessageArg>>,
}

fn read_message_part(reader: SaveReader, _identity: &dyn UnifiedIdentityDecoder) -> Result<Q1MessagePart, WorldError> {
    let args = reader.field("args");
    Ok(Q1MessagePart {
        text: reader.field("text").string()?,
        args: if args.value.is_none() {
            None
        } else {
            Some(bounded_list(args, read_message_arg)?)
        },
    })
}

fn write_message_part(part: &Q1MessagePart) -> SaveJson {
    let mut members = vec![("text", json_str(&part.text))];
    if let Some(args) = &part.args {
        members.push(("args", arr(args.iter().map(write_message_arg).collect())));
    }
    obj(members)
}

/// Q1 effect kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1EffectKind {
    /// Blood.
    Blood,
    /// Gunshot.
    Gunshot,
    /// Spike.
    Spike,
    /// Super spike.
    Superspike,
    /// Explosion.
    Explosion,
    /// Teleport.
    Teleport,
    /// Muzzle flash.
    Muzzleflash,
    /// Pickup.
    Pickup,
    /// Lava splash.
    LavaSplash,
    /// Tar explosion.
    TarExplosion,
    /// Meat spray.
    MeatSpray,
    /// Wizard spike.
    WizardSpike,
    /// Knight spike.
    KnightSpike,
}

impl Q1EffectKind {
    fn text(self) -> &'static str {
        match self {
            Self::Blood => "blood",
            Self::Gunshot => "gunshot",
            Self::Spike => "spike",
            Self::Superspike => "superspike",
            Self::Explosion => "explosion",
            Self::Teleport => "teleport",
            Self::Muzzleflash => "muzzleflash",
            Self::Pickup => "pickup",
            Self::LavaSplash => "lava-splash",
            Self::TarExplosion => "tar-explosion",
            Self::MeatSpray => "meat-spray",
            Self::WizardSpike => "wizard-spike",
            Self::KnightSpike => "knight-spike",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "blood" => Self::Blood,
            "gunshot" => Self::Gunshot,
            "spike" => Self::Spike,
            "superspike" => Self::Superspike,
            "explosion" => Self::Explosion,
            "teleport" => Self::Teleport,
            "muzzleflash" => Self::Muzzleflash,
            "pickup" => Self::Pickup,
            "lava-splash" => Self::LavaSplash,
            "tar-explosion" => Self::TarExplosion,
            "meat-spray" => Self::MeatSpray,
            "wizard-spike" => Self::WizardSpike,
            "knight-spike" => Self::KnightSpike,
            _ => return None,
        })
    }
}

/// Q1 weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Weapon {
    /// Axe.
    Axe,
    /// Shotgun.
    Shotgun,
    /// Super shotgun.
    Supershotgun,
    /// Nailgun.
    Nailgun,
    /// Super nailgun.
    Supernailgun,
    /// Grenade launcher.
    Grenadelauncher,
    /// Rocket launcher.
    Rocketlauncher,
    /// Lightning gun.
    Lightning,
    /// Hipnotic laser.
    HipnoticLaser,
    /// Hipnotic mjolnir.
    HipnoticMjolnir,
    /// Hipnotic proximity gun.
    HipnoticProximity,
    /// Rogue lava nailgun.
    RogueLavaNailgun,
    /// Rogue lava super nailgun.
    RogueLavaSupernailgun,
    /// Rogue multi grenade.
    RogueMultiGrenade,
    /// Rogue multi rocket.
    RogueMultiRocket,
    /// Rogue plasma.
    RoguePlasma,
    /// Rogue grapple.
    RogueGrapple,
    /// Mission pack 3 laser.
    Mg3Laser,
    /// Mission pack 3 mjolnir.
    Mg3Mjolnir,
    /// Capture-the-flag grapple.
    CtfGrapple,
}

const Q1_WEAPONS: &[&str] = &[
    "axe",
    "shotgun",
    "supershotgun",
    "nailgun",
    "supernailgun",
    "grenadelauncher",
    "rocketlauncher",
    "lightning",
    "hipnotic:laser",
    "hipnotic:mjolnir",
    "hipnotic:proximity",
    "rogue:lava-nailgun",
    "rogue:lava-supernailgun",
    "rogue:multi-grenade",
    "rogue:multi-rocket",
    "rogue:plasma",
    "rogue:grapple",
    "mg3:laser",
    "mg3:mjolnir",
    "ctf:grapple",
];

impl Q1Weapon {
    fn text(self) -> &'static str {
        match self {
            Self::Axe => "axe",
            Self::Shotgun => "shotgun",
            Self::Supershotgun => "supershotgun",
            Self::Nailgun => "nailgun",
            Self::Supernailgun => "supernailgun",
            Self::Grenadelauncher => "grenadelauncher",
            Self::Rocketlauncher => "rocketlauncher",
            Self::Lightning => "lightning",
            Self::HipnoticLaser => "hipnotic:laser",
            Self::HipnoticMjolnir => "hipnotic:mjolnir",
            Self::HipnoticProximity => "hipnotic:proximity",
            Self::RogueLavaNailgun => "rogue:lava-nailgun",
            Self::RogueLavaSupernailgun => "rogue:lava-supernailgun",
            Self::RogueMultiGrenade => "rogue:multi-grenade",
            Self::RogueMultiRocket => "rogue:multi-rocket",
            Self::RoguePlasma => "rogue:plasma",
            Self::RogueGrapple => "rogue:grapple",
            Self::Mg3Laser => "mg3:laser",
            Self::Mg3Mjolnir => "mg3:mjolnir",
            Self::CtfGrapple => "ctf:grapple",
        }
    }
}

/// Q1 character attack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1CharacterAttack {
    /// Axe swing.
    Axe {
        /// Swing variant.
        variant: i64,
    },
    /// Shotgun blast.
    Shotgun,
    /// Rocket launch.
    Rocket,
    /// Nail burst.
    Nail,
    /// Lightning discharge.
    Lightning,
}

fn read_character_attack(reader: SaveReader) -> Result<Q1CharacterAttack, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "axe" => Ok(Q1CharacterAttack::Axe {
            variant: reader.field("variant").choice_i64(&[0, 1, 2, 3])?,
        }),
        "shotgun" => Ok(Q1CharacterAttack::Shotgun),
        "rocket" => Ok(Q1CharacterAttack::Rocket),
        "nail" => Ok(Q1CharacterAttack::Nail),
        "lightning" => Ok(Q1CharacterAttack::Lightning),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_character_attack(attack: Q1CharacterAttack) -> SaveJson {
    match attack {
        Q1CharacterAttack::Axe { variant } => obj(vec![("kind", json_str("axe")), ("variant", int(variant))]),
        Q1CharacterAttack::Shotgun => obj(vec![("kind", json_str("shotgun"))]),
        Q1CharacterAttack::Rocket => obj(vec![("kind", json_str("rocket"))]),
        Q1CharacterAttack::Nail => obj(vec![("kind", json_str("nail"))]),
        Q1CharacterAttack::Lightning => obj(vec![("kind", json_str("lightning"))]),
    }
}

/// Q1 beam style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1BeamStyle {
    /// Lightning 1.
    Lightning1,
    /// Lightning 2.
    Lightning2,
    /// Lightning 3.
    Lightning3,
    /// Grapple.
    Grapple,
}

/// Q1 powerup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Powerup {
    /// Quad damage.
    Quad,
    /// Invulnerability.
    Invulnerability,
    /// Invisibility.
    Invisibility,
    /// Biosuit.
    Suit,
    /// Hipnotic wetsuit.
    HipnoticWetsuit,
    /// Hipnotic empathy shield.
    HipnoticEmpathy,
    /// Rogue shield.
    RogueShield,
    /// Rogue antigrav.
    RogueAntigrav,
    /// Mission pack 3 lava suit.
    Mg3Lavasuit,
}

/// Q1 event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1Event {
    /// Stop a looping sound.
    StopSound {
        /// Actor handle.
        actor: ActorId,
        /// Channel.
        channel: f64,
    },
    /// Positional sound.
    Sound {
        /// Origin override.
        origin: Option<Vec3>,
        /// Actor handle.
        actor: ActorId,
        /// Sound path.
        path: String,
        /// Channel.
        channel: Q1SoundChannel,
        /// Attenuation.
        attenuation: f64,
        /// Volume.
        volume: f64,
    },
    /// Ambient sound.
    Ambient {
        /// Origin.
        origin: Vec3,
        /// Sound path.
        path: String,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
    },
    /// Player message.
    Message {
        /// Player actor.
        player: ActorId,
        /// Text.
        text: String,
        /// Center-print flag.
        center: bool,
        /// Arguments.
        args: Option<Vec<Q1MessageArg>>,
        /// Message parts.
        parts: Option<Vec<Q1MessagePart>>,
    },
    /// Visual effect.
    Effect {
        /// Effect kind.
        effect: Q1EffectKind,
        /// Actor handle.
        actor: Option<ActorId>,
        /// Origin.
        origin: Vec3,
        /// Amount.
        amount: f64,
        /// Muzzle flash placement.
        muzzle: Option<Q1Muzzle>,
    },
    /// Colored explosion.
    ColoredExplosion {
        /// Origin.
        origin: Vec3,
        /// First color.
        color_start: f64,
        /// Color count.
        color_length: f64,
    },
    /// Static model.
    StaticModel {
        /// Model path.
        path: String,
        /// Frame.
        frame: f64,
        /// Color map.
        color_map: f64,
        /// Skin.
        skin: f64,
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
    },
    /// Particle burst.
    Particles {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Color.
        color: f64,
        /// Count.
        count: f64,
    },
    /// Server command text.
    ServerCommand {
        /// Text.
        text: String,
    },
    /// Camera placement.
    Camera {
        /// Player actor.
        player: ActorId,
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
        /// View offset.
        view_offset: Option<Vec3>,
    },
    /// Beam.
    Beam {
        /// Beam style.
        style: Q1BeamStyle,
        /// Actor handle.
        actor: ActorId,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
    },
    /// Light style.
    Lightstyle {
        /// Style index.
        style: f64,
        /// Pattern.
        pattern: String,
    },
    /// Monster total.
    MonsterTotal {
        /// Total.
        total: f64,
    },
    /// Secret found.
    Secret {
        /// Actor handle.
        actor: ActorId,
        /// Total.
        total: f64,
        /// Found count.
        found: f64,
    },
    /// Monster killed.
    MonsterKilled {
        /// Actor handle.
        actor: ActorId,
        /// Total.
        total: f64,
        /// Found count.
        found: f64,
    },
    /// Weapon state.
    Weapon {
        /// Player actor.
        player: ActorId,
        /// Weapon.
        weapon: Q1Weapon,
        /// View model path.
        view_model: String,
        /// Frame.
        frame: f64,
        /// Punch angle.
        punch: f64,
        /// Attack.
        attack: Option<Q1CharacterAttack>,
    },
    /// Player teleport.
    TeleportPlayer {
        /// Player actor.
        player: ActorId,
        /// Angles.
        angles: Vec3,
        /// Input lock time.
        lock_until: f64,
    },
    /// Powerup pickup.
    Powerup {
        /// Player actor.
        player: ActorId,
        /// Powerup.
        powerup: Q1Powerup,
        /// Expiry time.
        expires: f64,
    },
    /// Intermission camera.
    Intermission {
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
        /// Map name.
        map: String,
        /// Exit delay.
        exit_after: f64,
        /// Music track.
        track: f64,
    },
    /// Finale text.
    Finale {
        /// Text.
        text: String,
        /// Stage.
        stage: i64,
    },
    /// Achievement.
    Achievement {
        /// Player actor.
        player: Option<ActorId>,
        /// Achievement id.
        id: String,
    },
}

/// Q1 muzzle flash placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1Muzzle {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

fn read_colored_explosion(reader: SaveReader, _identity: &dyn UnifiedIdentityDecoder) -> Result<Q1Event, WorldError> {
    reader.field("kind").literal_str("colored-explosion")?;
    Ok(Q1Event::ColoredExplosion {
        origin: read_vector(reader.field("origin"))?,
        color_start: reader.field("colorStart").finite()?,
        color_length: reader.field("colorLength").finite()?,
    })
}

fn write_colored_explosion(origin: Vec3, color_start: f64, color_length: f64) -> SaveJson {
    obj(vec![
        ("kind", json_str("colored-explosion")),
        ("origin", write_vector(origin)),
        ("colorStart", num(color_start)),
        ("colorLength", num(color_length)),
    ])
}

fn read_lightstyle(reader: SaveReader) -> Result<Q1Event, WorldError> {
    reader.field("kind").literal_str("lightstyle")?;
    Ok(Q1Event::Lightstyle {
        style: reader.field("style").finite()?,
        pattern: reader.field("pattern").string()?,
    })
}

fn write_lightstyle(style: f64, pattern: &str) -> SaveJson {
    obj(vec![
        ("kind", json_str("lightstyle")),
        ("style", num(style)),
        ("pattern", json_str(pattern)),
    ])
}

fn read_q1_event(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q1Event, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "stop-sound" => Ok(Q1Event::StopSound {
            actor: read_actor(reader.field("actor"), identity)?,
            channel: reader.field("channel").finite()?,
        }),
        "sound" => {
            let origin = reader.field("origin");
            Ok(Q1Event::Sound {
                origin: if origin.value.is_none() {
                    None
                } else {
                    Some(read_vector(origin)?)
                },
                actor: read_actor(reader.field("actor"), identity)?,
                path: reader.field("path").string()?,
                channel: read_sound_channel(reader.field("channel"))?,
                attenuation: reader.field("attenuation").finite()?,
                volume: reader.field("volume").finite()?,
            })
        }
        "ambient" => Ok(Q1Event::Ambient {
            origin: read_vector(reader.field("origin"))?,
            path: reader.field("path").string()?,
            volume: reader.field("volume").finite()?,
            attenuation: reader.field("attenuation").finite()?,
        }),
        "message" => {
            let args = reader.field("args");
            let parts = reader.field("parts");
            Ok(Q1Event::Message {
                player: read_actor(reader.field("player"), identity)?,
                text: reader.field("text").string()?,
                center: reader.field("center").boolean()?,
                args: if args.value.is_none() {
                    None
                } else {
                    Some(bounded_list(args, read_message_arg)?)
                },
                parts: if parts.value.is_none() {
                    None
                } else {
                    Some(bounded_list(parts, |part| read_message_part(part, identity))?)
                },
            })
        }
        "effect" => {
            let effect = reader.field("effect").choice_str(&[
                "blood",
                "gunshot",
                "spike",
                "superspike",
                "explosion",
                "teleport",
                "muzzleflash",
                "pickup",
                "lava-splash",
                "tar-explosion",
                "meat-spray",
                "wizard-spike",
                "knight-spike",
            ])?;
            let muzzle = reader.field("muzzle");
            Ok(Q1Event::Effect {
                effect: Q1EffectKind::parse(&effect).expect("choice validated"),
                actor: read_opt_actor(reader.field("actor"), identity)?,
                origin: read_vector(reader.field("origin"))?,
                amount: reader.field("amount").finite()?,
                muzzle: if muzzle.value.is_none() {
                    None
                } else {
                    Some(Q1Muzzle {
                        origin: read_vector(muzzle.field("origin"))?,
                        angles: read_vector(muzzle.field("angles"))?,
                    })
                },
            })
        }
        "colored-explosion" => read_colored_explosion(reader, identity),
        "static-model" => Ok(Q1Event::StaticModel {
            path: reader.field("path").string()?,
            frame: reader.field("frame").finite()?,
            color_map: reader.field("colorMap").finite()?,
            skin: reader.field("skin").finite()?,
            origin: read_vector(reader.field("origin"))?,
            angles: read_vector(reader.field("angles"))?,
        }),
        "particles" => Ok(Q1Event::Particles {
            origin: read_vector(reader.field("origin"))?,
            direction: read_vector(reader.field("direction"))?,
            color: reader.field("color").finite()?,
            count: reader.field("count").finite()?,
        }),
        "server-command" => Ok(Q1Event::ServerCommand {
            text: reader.field("text").string()?,
        }),
        "camera" => {
            let view_offset = reader.field("viewOffset");
            Ok(Q1Event::Camera {
                player: read_actor(reader.field("player"), identity)?,
                origin: read_vector(reader.field("origin"))?,
                angles: read_vector(reader.field("angles"))?,
                view_offset: if view_offset.value.is_none() {
                    None
                } else {
                    Some(read_vector(view_offset)?)
                },
            })
        }
        "beam" => {
            let style = reader
                .field("style")
                .choice_str(&["lightning1", "lightning2", "lightning3", "grapple"])?;
            Ok(Q1Event::Beam {
                style: match style.as_str() {
                    "lightning1" => Q1BeamStyle::Lightning1,
                    "lightning2" => Q1BeamStyle::Lightning2,
                    "lightning3" => Q1BeamStyle::Lightning3,
                    _ => Q1BeamStyle::Grapple,
                },
                actor: read_actor(reader.field("actor"), identity)?,
                start: read_vector(reader.field("start"))?,
                end: read_vector(reader.field("end"))?,
            })
        }
        "lightstyle" => read_lightstyle(reader),
        "monster-total" => Ok(Q1Event::MonsterTotal {
            total: reader.field("total").finite()?,
        }),
        "secret" => Ok(Q1Event::Secret {
            actor: read_actor(reader.field("actor"), identity)?,
            total: reader.field("total").finite()?,
            found: reader.field("found").finite()?,
        }),
        "monster-killed" => Ok(Q1Event::MonsterKilled {
            actor: read_actor(reader.field("actor"), identity)?,
            total: reader.field("total").finite()?,
            found: reader.field("found").finite()?,
        }),
        "weapon" => {
            let weapon = reader.field("weapon").choice_str(Q1_WEAPONS)?;
            let attack = reader.field("attack");
            Ok(Q1Event::Weapon {
                player: read_actor(reader.field("player"), identity)?,
                weapon: Q1Weapon::parse(&weapon).expect("choice validated"),
                view_model: reader.field("viewModel").string()?,
                frame: reader.field("frame").finite()?,
                punch: reader.field("punch").finite()?,
                attack: if attack.value.is_none() {
                    None
                } else {
                    Some(read_character_attack(attack)?)
                },
            })
        }
        "teleport-player" => Ok(Q1Event::TeleportPlayer {
            player: read_actor(reader.field("player"), identity)?,
            angles: read_vector(reader.field("angles"))?,
            lock_until: reader.field("lockUntil").finite()?,
        }),
        "powerup" => {
            let powerup = reader.field("powerup").choice_str(&[
                "quad",
                "invulnerability",
                "invisibility",
                "suit",
                "hipnotic:wetsuit",
                "hipnotic:empathy",
                "rogue:shield",
                "rogue:antigrav",
                "mg3:lavasuit",
            ])?;
            Ok(Q1Event::Powerup {
                player: read_actor(reader.field("player"), identity)?,
                powerup: Q1Powerup::parse(&powerup).expect("choice validated"),
                expires: reader.field("expires").finite()?,
            })
        }
        "intermission" => Ok(Q1Event::Intermission {
            origin: read_vector(reader.field("origin"))?,
            angles: read_vector(reader.field("angles"))?,
            map: reader.field("map").string()?,
            exit_after: reader.field("exitAfter").finite()?,
            track: reader.field("track").finite()?,
        }),
        "finale" => Ok(Q1Event::Finale {
            text: reader.field("text").string()?,
            stage: reader.field("stage").choice_i64(&[1, 2, 3, 4, 5, 6])?,
        }),
        "achievement" => Ok(Q1Event::Achievement {
            player: read_opt_actor(reader.field("player"), identity)?,
            id: reader.field("id").string()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

impl Q1Weapon {
    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "axe" => Self::Axe,
            "shotgun" => Self::Shotgun,
            "supershotgun" => Self::Supershotgun,
            "nailgun" => Self::Nailgun,
            "supernailgun" => Self::Supernailgun,
            "grenadelauncher" => Self::Grenadelauncher,
            "rocketlauncher" => Self::Rocketlauncher,
            "lightning" => Self::Lightning,
            "hipnotic:laser" => Self::HipnoticLaser,
            "hipnotic:mjolnir" => Self::HipnoticMjolnir,
            "hipnotic:proximity" => Self::HipnoticProximity,
            "rogue:lava-nailgun" => Self::RogueLavaNailgun,
            "rogue:lava-supernailgun" => Self::RogueLavaSupernailgun,
            "rogue:multi-grenade" => Self::RogueMultiGrenade,
            "rogue:multi-rocket" => Self::RogueMultiRocket,
            "rogue:plasma" => Self::RoguePlasma,
            "rogue:grapple" => Self::RogueGrapple,
            "mg3:laser" => Self::Mg3Laser,
            "mg3:mjolnir" => Self::Mg3Mjolnir,
            "ctf:grapple" => Self::CtfGrapple,
            _ => return None,
        })
    }
}

impl Q1Powerup {
    fn text(self) -> &'static str {
        match self {
            Self::Quad => "quad",
            Self::Invulnerability => "invulnerability",
            Self::Invisibility => "invisibility",
            Self::Suit => "suit",
            Self::HipnoticWetsuit => "hipnotic:wetsuit",
            Self::HipnoticEmpathy => "hipnotic:empathy",
            Self::RogueShield => "rogue:shield",
            Self::RogueAntigrav => "rogue:antigrav",
            Self::Mg3Lavasuit => "mg3:lavasuit",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "quad" => Self::Quad,
            "invulnerability" => Self::Invulnerability,
            "invisibility" => Self::Invisibility,
            "suit" => Self::Suit,
            "hipnotic:wetsuit" => Self::HipnoticWetsuit,
            "hipnotic:empathy" => Self::HipnoticEmpathy,
            "rogue:shield" => Self::RogueShield,
            "rogue:antigrav" => Self::RogueAntigrav,
            "mg3:lavasuit" => Self::Mg3Lavasuit,
            _ => return None,
        })
    }
}

fn write_q1_event(event: &Q1Event) -> SaveJson {
    match event {
        Q1Event::StopSound { actor, channel } => obj(vec![
            ("kind", json_str("stop-sound")),
            ("actor", wire_actor(actor)),
            ("channel", num(*channel)),
        ]),
        Q1Event::Sound {
            origin,
            actor,
            path,
            channel,
            attenuation,
            volume,
        } => {
            let mut members = vec![("kind", json_str("sound"))];
            if let Some(origin) = origin {
                members.push(("origin", write_vector(*origin)));
            }
            members.push(("actor", wire_actor(actor)));
            members.push(("path", json_str(path)));
            members.push(("channel", write_sound_channel(*channel)));
            members.push(("attenuation", num(*attenuation)));
            members.push(("volume", num(*volume)));
            obj(members)
        }
        Q1Event::Ambient {
            origin,
            path,
            volume,
            attenuation,
        } => obj(vec![
            ("kind", json_str("ambient")),
            ("origin", write_vector(*origin)),
            ("path", json_str(path)),
            ("volume", num(*volume)),
            ("attenuation", num(*attenuation)),
        ]),
        Q1Event::Message {
            player,
            text,
            center,
            args,
            parts,
        } => {
            let mut members = vec![
                ("kind", json_str("message")),
                ("player", wire_actor(player)),
                ("text", json_str(text)),
                ("center", boolean(*center)),
            ];
            if let Some(args) = args {
                members.push(("args", arr(args.iter().map(write_message_arg).collect())));
            }
            if let Some(parts) = parts {
                members.push(("parts", arr(parts.iter().map(write_message_part).collect())));
            }
            obj(members)
        }
        Q1Event::Effect {
            effect,
            actor,
            origin,
            amount,
            muzzle,
        } => {
            let mut members = vec![
                ("kind", json_str("effect")),
                ("effect", json_str(effect.text())),
                ("actor", write_opt_actor(actor.as_ref())),
                ("origin", write_vector(*origin)),
                ("amount", num(*amount)),
            ];
            if let Some(muzzle) = muzzle {
                members.push((
                    "muzzle",
                    obj(vec![
                        ("origin", write_vector(muzzle.origin)),
                        ("angles", write_vector(muzzle.angles)),
                    ]),
                ));
            }
            obj(members)
        }
        Q1Event::ColoredExplosion {
            origin,
            color_start,
            color_length,
        } => write_colored_explosion(*origin, *color_start, *color_length),
        Q1Event::StaticModel {
            path,
            frame,
            color_map,
            skin,
            origin,
            angles,
        } => obj(vec![
            ("kind", json_str("static-model")),
            ("path", json_str(path)),
            ("frame", num(*frame)),
            ("colorMap", num(*color_map)),
            ("skin", num(*skin)),
            ("origin", write_vector(*origin)),
            ("angles", write_vector(*angles)),
        ]),
        Q1Event::Particles {
            origin,
            direction,
            color,
            count,
        } => obj(vec![
            ("kind", json_str("particles")),
            ("origin", write_vector(*origin)),
            ("direction", write_vector(*direction)),
            ("color", num(*color)),
            ("count", num(*count)),
        ]),
        Q1Event::ServerCommand { text } => obj(vec![("kind", json_str("server-command")), ("text", json_str(text))]),
        Q1Event::Camera {
            player,
            origin,
            angles,
            view_offset,
        } => {
            let mut members = vec![
                ("kind", json_str("camera")),
                ("player", wire_actor(player)),
                ("origin", write_vector(*origin)),
                ("angles", write_vector(*angles)),
            ];
            if let Some(offset) = view_offset {
                members.push(("viewOffset", write_vector(*offset)));
            }
            obj(members)
        }
        Q1Event::Beam {
            style,
            actor,
            start,
            end,
        } => obj(vec![
            ("kind", json_str("beam")),
            (
                "style",
                json_str(match style {
                    Q1BeamStyle::Lightning1 => "lightning1",
                    Q1BeamStyle::Lightning2 => "lightning2",
                    Q1BeamStyle::Lightning3 => "lightning3",
                    Q1BeamStyle::Grapple => "grapple",
                }),
            ),
            ("actor", wire_actor(actor)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
        ]),
        Q1Event::Lightstyle { style, pattern } => write_lightstyle(*style, pattern),
        Q1Event::MonsterTotal { total } => obj(vec![("kind", json_str("monster-total")), ("total", num(*total))]),
        Q1Event::Secret { actor, total, found } => obj(vec![
            ("kind", json_str("secret")),
            ("actor", wire_actor(actor)),
            ("total", num(*total)),
            ("found", num(*found)),
        ]),
        Q1Event::MonsterKilled { actor, total, found } => obj(vec![
            ("kind", json_str("monster-killed")),
            ("actor", wire_actor(actor)),
            ("total", num(*total)),
            ("found", num(*found)),
        ]),
        Q1Event::Weapon {
            player,
            weapon,
            view_model,
            frame,
            punch,
            attack,
        } => {
            let mut members = vec![
                ("kind", json_str("weapon")),
                ("player", wire_actor(player)),
                ("weapon", json_str(weapon.text())),
                ("viewModel", json_str(view_model)),
                ("frame", num(*frame)),
                ("punch", num(*punch)),
            ];
            if let Some(attack) = attack {
                members.push(("attack", write_character_attack(*attack)));
            }
            obj(members)
        }
        Q1Event::TeleportPlayer {
            player,
            angles,
            lock_until,
        } => obj(vec![
            ("kind", json_str("teleport-player")),
            ("player", wire_actor(player)),
            ("angles", write_vector(*angles)),
            ("lockUntil", num(*lock_until)),
        ]),
        Q1Event::Powerup {
            player,
            powerup,
            expires,
        } => obj(vec![
            ("kind", json_str("powerup")),
            ("player", wire_actor(player)),
            ("powerup", json_str(powerup.text())),
            ("expires", num(*expires)),
        ]),
        Q1Event::Intermission {
            origin,
            angles,
            map,
            exit_after,
            track,
        } => obj(vec![
            ("kind", json_str("intermission")),
            ("origin", write_vector(*origin)),
            ("angles", write_vector(*angles)),
            ("map", json_str(map)),
            ("exitAfter", num(*exit_after)),
            ("track", num(*track)),
        ]),
        Q1Event::Finale { text, stage } => obj(vec![
            ("kind", json_str("finale")),
            ("text", json_str(text)),
            ("stage", int(*stage)),
        ]),
        Q1Event::Achievement { player, id } => obj(vec![
            ("kind", json_str("achievement")),
            ("player", write_opt_actor(player.as_ref())),
            ("id", json_str(id)),
        ]),
    }
}

/// Q1 fog parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Fog {
    /// Density.
    pub density: f64,
    /// Color.
    pub color: Vec3,
}

fn read_q1_fog(reader: SaveReader) -> Result<Q1Fog, WorldError> {
    Ok(Q1Fog {
        density: reader.field("density").finite()?,
        color: read_vector(reader.field("color"))?,
    })
}

fn write_q1_fog(fog: &Q1Fog) -> SaveJson {
    obj(vec![("density", num(fog.density)), ("color", write_vector(fog.color))])
}

/// Q1 fog transition.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FogTransition {
    /// Previous fog.
    pub previous: Q1Fog,
    /// Target fog.
    pub target: Q1Fog,
    /// Start time.
    pub start: f64,
    /// Duration.
    pub duration: f64,
}

fn read_q1_fog_transition(reader: SaveReader) -> Result<Q1FogTransition, WorldError> {
    Ok(Q1FogTransition {
        previous: read_q1_fog(reader.field("previous"))?,
        target: read_q1_fog(reader.field("target"))?,
        start: reader.field("start").finite()?,
        duration: reader.field("duration").finite()?,
    })
}

fn write_q1_fog_transition(transition: &Q1FogTransition) -> SaveJson {
    obj(vec![
        ("previous", write_q1_fog(&transition.previous)),
        ("target", write_q1_fog(&transition.target)),
        ("start", num(transition.start)),
        ("duration", num(transition.duration)),
    ])
}

/// Q1 fog event body.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FogEvent {
    /// Player actor.
    pub player: Option<ActorId>,
    /// Transition.
    pub transition: Q1FogTransition,
    /// Sky factor.
    pub sky_factor: f64,
}

/// Rune program.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1RuneProgram {
    /// Dopa.
    Dopa,
    /// Mission pack 1.
    Mg1,
    /// Mission pack 3.
    Mg3,
    /// Capture the flag.
    Ctf,
}

/// Q1 addon event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1AddonEvent {
    /// Music selection.
    Music {
        /// Track.
        track: f64,
        /// Loop track.
        loop_track: f64,
    },
    /// Sell screen.
    SellScreen,
    /// Entity alpha.
    Alpha {
        /// Actor handle.
        actor: ActorId,
        /// Alpha.
        alpha: f64,
    },
    /// Rune collected.
    RuneCollected {
        /// Player actor.
        player: ActorId,
        /// Rune bits.
        bits: f64,
        /// Program.
        program: Q1RuneProgram,
    },
    /// Cutscene camera.
    Cutscene {
        /// Camera origin.
        camera: Vec3,
        /// Camera angles.
        angles: Vec3,
    },
    /// Fog override.
    Fog {
        /// Player actor.
        player: Option<ActorId>,
        /// Density.
        density: f64,
        /// Color.
        color: Vec3,
        /// Sky factor.
        sky_factor: f64,
        /// Duration.
        duration: f64,
    },
    /// Punch angle.
    PunchAngle {
        /// Player actor.
        player: ActorId,
        /// Angles.
        angles: Vec3,
    },
    /// View roll.
    ViewRoll {
        /// Player actor.
        player: ActorId,
        /// Roll.
        roll: f64,
    },
    /// Lightning beam.
    Lightning {
        /// Actor handle.
        actor: ActorId,
        /// Style.
        style: i64,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
    },
    /// Colored explosion.
    ColoredExplosion {
        /// Origin.
        origin: Vec3,
        /// First color.
        color_start: f64,
        /// Color count.
        color_length: f64,
    },
    /// Monster count.
    MonsterCount {
        /// Count.
        count: f64,
    },
    /// Developer message.
    DeveloperMessage {
        /// Text.
        text: String,
    },
    /// Actor effects.
    ActorEffects {
        /// Actor handle.
        actor: ActorId,
        /// Effects bitmask.
        effects: f64,
    },
    /// Debug bounds.
    DebugBounds {
        /// Minimum corner.
        min: Vec3,
        /// Maximum corner.
        max: Vec3,
        /// Color.
        color: f64,
        /// Lifetime.
        lifetime: f64,
        /// Depth-test flag.
        depth_test: bool,
    },
}

fn read_addon_alpha(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q1AddonEvent, WorldError> {
    reader.field("kind").literal_str("alpha")?;
    Ok(Q1AddonEvent::Alpha {
        actor: read_actor(reader.field("actor"), identity)?,
        alpha: reader.field("alpha").finite()?,
    })
}

fn write_addon_alpha(actor: &ActorId, alpha: f64) -> SaveJson {
    obj(vec![
        ("kind", json_str("alpha")),
        ("actor", wire_actor(actor)),
        ("alpha", num(alpha)),
    ])
}

fn read_addon_event(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q1AddonEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "music" => Ok(Q1AddonEvent::Music {
            track: reader.field("track").finite()?,
            loop_track: reader.field("loopTrack").finite()?,
        }),
        "sell-screen" => Ok(Q1AddonEvent::SellScreen),
        "alpha" => read_addon_alpha(reader, identity),
        "rune-collected" => {
            let program = reader.field("program").choice_str(&["dopa", "mg1", "mg3", "ctf"])?;
            Ok(Q1AddonEvent::RuneCollected {
                player: read_actor(reader.field("player"), identity)?,
                bits: reader.field("bits").finite()?,
                program: match program.as_str() {
                    "dopa" => Q1RuneProgram::Dopa,
                    "mg1" => Q1RuneProgram::Mg1,
                    "mg3" => Q1RuneProgram::Mg3,
                    _ => Q1RuneProgram::Ctf,
                },
            })
        }
        "cutscene" => Ok(Q1AddonEvent::Cutscene {
            camera: read_vector(reader.field("camera"))?,
            angles: read_vector(reader.field("angles"))?,
        }),
        "fog" => Ok(Q1AddonEvent::Fog {
            player: read_opt_actor(reader.field("player"), identity)?,
            density: reader.field("density").finite()?,
            color: read_vector(reader.field("color"))?,
            sky_factor: reader.field("skyFactor").finite()?,
            duration: reader.field("duration").finite()?,
        }),
        "punch-angle" => Ok(Q1AddonEvent::PunchAngle {
            player: read_actor(reader.field("player"), identity)?,
            angles: read_vector(reader.field("angles"))?,
        }),
        "view-roll" => Ok(Q1AddonEvent::ViewRoll {
            player: read_actor(reader.field("player"), identity)?,
            roll: reader.field("roll").finite()?,
        }),
        "lightning" => Ok(Q1AddonEvent::Lightning {
            actor: read_actor(reader.field("actor"), identity)?,
            style: reader.field("style").choice_i64(&[1, 2, 3])?,
            start: read_vector(reader.field("start"))?,
            end: read_vector(reader.field("end"))?,
        }),
        "colored-explosion" => {
            reader.field("kind").literal_str("colored-explosion")?;
            Ok(Q1AddonEvent::ColoredExplosion {
                origin: read_vector(reader.field("origin"))?,
                color_start: reader.field("colorStart").finite()?,
                color_length: reader.field("colorLength").finite()?,
            })
        }
        "monster-count" => Ok(Q1AddonEvent::MonsterCount {
            count: reader.field("count").finite()?,
        }),
        "developer-message" => Ok(Q1AddonEvent::DeveloperMessage {
            text: reader.field("text").string()?,
        }),
        "actor-effects" => Ok(Q1AddonEvent::ActorEffects {
            actor: read_actor(reader.field("actor"), identity)?,
            effects: reader.field("effects").finite()?,
        }),
        "debug-bounds" => Ok(Q1AddonEvent::DebugBounds {
            min: read_vector(reader.field("min"))?,
            max: read_vector(reader.field("max"))?,
            color: reader.field("color").finite()?,
            lifetime: reader.field("lifetime").finite()?,
            depth_test: reader.field("depthTest").boolean()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_addon_event(event: &Q1AddonEvent) -> SaveJson {
    match event {
        Q1AddonEvent::Music { track, loop_track } => obj(vec![
            ("kind", json_str("music")),
            ("track", num(*track)),
            ("loopTrack", num(*loop_track)),
        ]),
        Q1AddonEvent::SellScreen => obj(vec![("kind", json_str("sell-screen"))]),
        Q1AddonEvent::Alpha { actor, alpha } => write_addon_alpha(actor, *alpha),
        Q1AddonEvent::RuneCollected { player, bits, program } => obj(vec![
            ("kind", json_str("rune-collected")),
            ("player", wire_actor(player)),
            ("bits", num(*bits)),
            (
                "program",
                json_str(match program {
                    Q1RuneProgram::Dopa => "dopa",
                    Q1RuneProgram::Mg1 => "mg1",
                    Q1RuneProgram::Mg3 => "mg3",
                    Q1RuneProgram::Ctf => "ctf",
                }),
            ),
        ]),
        Q1AddonEvent::Cutscene { camera, angles } => obj(vec![
            ("kind", json_str("cutscene")),
            ("camera", write_vector(*camera)),
            ("angles", write_vector(*angles)),
        ]),
        Q1AddonEvent::Fog {
            player,
            density,
            color,
            sky_factor,
            duration,
        } => obj(vec![
            ("kind", json_str("fog")),
            ("player", write_opt_actor(player.as_ref())),
            ("density", num(*density)),
            ("color", write_vector(*color)),
            ("skyFactor", num(*sky_factor)),
            ("duration", num(*duration)),
        ]),
        Q1AddonEvent::PunchAngle { player, angles } => obj(vec![
            ("kind", json_str("punch-angle")),
            ("player", wire_actor(player)),
            ("angles", write_vector(*angles)),
        ]),
        Q1AddonEvent::ViewRoll { player, roll } => obj(vec![
            ("kind", json_str("view-roll")),
            ("player", wire_actor(player)),
            ("roll", num(*roll)),
        ]),
        Q1AddonEvent::Lightning {
            actor,
            style,
            start,
            end,
        } => obj(vec![
            ("kind", json_str("lightning")),
            ("actor", wire_actor(actor)),
            ("style", int(*style)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
        ]),
        Q1AddonEvent::ColoredExplosion {
            origin,
            color_start,
            color_length,
        } => write_colored_explosion(*origin, *color_start, *color_length),
        Q1AddonEvent::MonsterCount { count } => obj(vec![("kind", json_str("monster-count")), ("count", num(*count))]),
        Q1AddonEvent::DeveloperMessage { text } => {
            obj(vec![("kind", json_str("developer-message")), ("text", json_str(text))])
        }
        Q1AddonEvent::ActorEffects { actor, effects } => obj(vec![
            ("kind", json_str("actor-effects")),
            ("actor", wire_actor(actor)),
            ("effects", num(*effects)),
        ]),
        Q1AddonEvent::DebugBounds {
            min,
            max,
            color,
            lifetime,
            depth_test,
        } => obj(vec![
            ("kind", json_str("debug-bounds")),
            ("min", write_vector(*min)),
            ("max", write_vector(*max)),
            ("color", num(*color)),
            ("lifetime", num(*lifetime)),
            ("depthTest", boolean(*depth_test)),
        ]),
    }
}

/// Q1 client userinfo row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1UserinfoItem {
    /// Key.
    pub key: String,
    /// Value.
    pub value: String,
}

/// Q1 client snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ClientSnapshot {
    /// Actor handle.
    pub actor: ActorId,
    /// Slot.
    pub slot: f64,
    /// Name.
    pub name: String,
    /// Frags.
    pub frags: f64,
    /// Shirt color.
    pub shirt: f64,
    /// Pants color.
    pub pants: f64,
    /// Team.
    pub team: f64,
    /// Observer flag.
    pub observer: bool,
    /// No-target flag.
    pub no_target: bool,
    /// Userinfo rows.
    pub userinfo: Vec<Q1UserinfoItem>,
}

fn read_client_snapshot(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q1ClientSnapshot, WorldError> {
    Ok(Q1ClientSnapshot {
        actor: read_actor(reader.field("actor"), identity)?,
        slot: reader.field("slot").finite()?,
        name: reader.field("name").string()?,
        frags: reader.field("frags").finite()?,
        shirt: reader.field("shirt").finite()?,
        pants: reader.field("pants").finite()?,
        team: reader.field("team").finite()?,
        observer: reader.field("observer").boolean()?,
        no_target: reader.field("noTarget").boolean()?,
        userinfo: bounded_list(reader.field("userinfo"), |item| {
            Ok(Q1UserinfoItem {
                key: item.field("key").string()?,
                value: item.field("value").string()?,
            })
        })?,
    })
}

fn write_client_snapshot(snapshot: &Q1ClientSnapshot) -> SaveJson {
    obj(vec![
        ("actor", wire_actor(&snapshot.actor)),
        ("slot", num(snapshot.slot)),
        ("name", json_str(&snapshot.name)),
        ("frags", num(snapshot.frags)),
        ("shirt", num(snapshot.shirt)),
        ("pants", num(snapshot.pants)),
        ("team", num(snapshot.team)),
        ("observer", boolean(snapshot.observer)),
        ("noTarget", boolean(snapshot.no_target)),
        (
            "userinfo",
            arr(snapshot
                .userinfo
                .iter()
                .map(|item| obj(vec![("key", json_str(&item.key)), ("value", json_str(&item.value))]))
                .collect()),
        ),
    ])
}

/// Capture-the-flag status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1CtfStatus {
    /// Red score.
    pub red: f64,
    /// Blue score.
    pub blue: f64,
    /// Flag state.
    pub flags: f64,
    /// Rune items.
    pub rune_items: f64,
}

fn read_ctf_status(reader: SaveReader) -> Result<Q1CtfStatus, WorldError> {
    Ok(Q1CtfStatus {
        red: reader.field("red").finite()?,
        blue: reader.field("blue").finite()?,
        flags: reader.field("flags").finite()?,
        rune_items: reader.field("runeItems").finite()?,
    })
}

fn write_ctf_status(status: Q1CtfStatus) -> SaveJson {
    obj(vec![
        ("red", num(status.red)),
        ("blue", num(status.blue)),
        ("flags", num(status.flags)),
        ("runeItems", num(status.rune_items)),
    ])
}

/// Prompt choice.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PromptChoice {
    /// Label.
    pub label: String,
    /// Impulse.
    pub impulse: f64,
}

/// Q1 source finale.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1SourceFinale {
    /// Finale text.
    Finale {
        /// Text.
        text: String,
        /// Music track.
        track: f64,
    },
    /// Sell screen.
    SellScreen,
}

fn read_source_finale(reader: SaveReader) -> Result<Q1SourceFinale, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "finale" => Ok(Q1SourceFinale::Finale {
            text: reader.field("text").string()?,
            track: reader.field("track").finite()?,
        }),
        "sell-screen" => Ok(Q1SourceFinale::SellScreen),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_source_finale(finale: &Q1SourceFinale) -> SaveJson {
    match finale {
        Q1SourceFinale::Finale { text, track } => obj(vec![
            ("kind", json_str("finale")),
            ("text", json_str(text)),
            ("track", num(*track)),
        ]),
        Q1SourceFinale::SellScreen => obj(vec![("kind", json_str("sell-screen"))]),
    }
}

/// Q1 composition event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1CompositionEvent {
    /// Addon event.
    Addon {
        /// Event.
        event: Q1AddonEvent,
    },
    /// Client snapshot.
    Client {
        /// Snapshot.
        client: Q1ClientSnapshot,
    },
    /// Client left.
    ClientLeft {
        /// Actor handle.
        actor: ActorId,
        /// Slot.
        slot: f64,
    },
    /// CTF status.
    CtfStatus {
        /// Actor handle.
        actor: ActorId,
        /// Status.
        status: Q1CtfStatus,
    },
    /// CTF capture.
    CtfCapture {
        /// Team.
        team: Q1CtfTeam,
        /// Total.
        total: f64,
    },
    /// Prompt.
    Prompt {
        /// Actor handle.
        actor: ActorId,
        /// Title.
        title: String,
        /// Choices.
        choices: Vec<Q1PromptChoice>,
    },
    /// Clear prompt.
    ClearPrompt {
        /// Actor handle.
        actor: ActorId,
    },
    /// Source log.
    SourceLog {
        /// Actor handle.
        actor: ActorId,
        /// Action.
        action: String,
    },
    /// Developer message.
    DeveloperMessage {
        /// Text.
        text: String,
    },
    /// Level presentation.
    LevelPresentation {
        /// Event.
        event: Q1SourceFinale,
    },
}

/// Q1 CTF team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1CtfTeam {
    /// Red team.
    Red,
    /// Blue team.
    Blue,
}

fn read_q1_composition_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q1CompositionEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "addon" => Ok(Q1CompositionEvent::Addon {
            event: read_addon_event(reader.field("event"), identity)?,
        }),
        "client" => Ok(Q1CompositionEvent::Client {
            client: read_client_snapshot(reader.field("client"), identity)?,
        }),
        "client-left" => Ok(Q1CompositionEvent::ClientLeft {
            actor: read_actor(reader.field("actor"), identity)?,
            slot: reader.field("slot").finite()?,
        }),
        "ctf-status" => Ok(Q1CompositionEvent::CtfStatus {
            actor: read_actor(reader.field("actor"), identity)?,
            status: read_ctf_status(reader.field("status"))?,
        }),
        "ctf-capture" => {
            let team = reader.field("team").choice_str(&["red", "blue"])?;
            Ok(Q1CompositionEvent::CtfCapture {
                team: if team == "red" { Q1CtfTeam::Red } else { Q1CtfTeam::Blue },
                total: reader.field("total").finite()?,
            })
        }
        "prompt" => Ok(Q1CompositionEvent::Prompt {
            actor: read_actor(reader.field("actor"), identity)?,
            title: reader.field("title").string()?,
            choices: bounded_list(reader.field("choices"), |item| {
                Ok(Q1PromptChoice {
                    label: item.field("label").string()?,
                    impulse: item.field("impulse").finite()?,
                })
            })?,
        }),
        "clear-prompt" => Ok(Q1CompositionEvent::ClearPrompt {
            actor: read_actor(reader.field("actor"), identity)?,
        }),
        "source-log" => Ok(Q1CompositionEvent::SourceLog {
            actor: read_actor(reader.field("actor"), identity)?,
            action: reader.field("action").string()?,
        }),
        "developer-message" => Ok(Q1CompositionEvent::DeveloperMessage {
            text: reader.field("text").string()?,
        }),
        "level-presentation" => Ok(Q1CompositionEvent::LevelPresentation {
            event: read_source_finale(reader.field("event"))?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q1_composition_event(event: &Q1CompositionEvent) -> SaveJson {
    match event {
        Q1CompositionEvent::Addon { event } => {
            obj(vec![("kind", json_str("addon")), ("event", write_addon_event(event))])
        }
        Q1CompositionEvent::Client { client } => obj(vec![
            ("kind", json_str("client")),
            ("client", write_client_snapshot(client)),
        ]),
        Q1CompositionEvent::ClientLeft { actor, slot } => obj(vec![
            ("kind", json_str("client-left")),
            ("actor", wire_actor(actor)),
            ("slot", num(*slot)),
        ]),
        Q1CompositionEvent::CtfStatus { actor, status } => obj(vec![
            ("kind", json_str("ctf-status")),
            ("actor", wire_actor(actor)),
            ("status", write_ctf_status(*status)),
        ]),
        Q1CompositionEvent::CtfCapture { team, total } => obj(vec![
            ("kind", json_str("ctf-capture")),
            (
                "team",
                json_str(match team {
                    Q1CtfTeam::Red => "red",
                    Q1CtfTeam::Blue => "blue",
                }),
            ),
            ("total", num(*total)),
        ]),
        Q1CompositionEvent::Prompt { actor, title, choices } => obj(vec![
            ("kind", json_str("prompt")),
            ("actor", wire_actor(actor)),
            ("title", json_str(title)),
            (
                "choices",
                arr(choices
                    .iter()
                    .map(|choice| {
                        obj(vec![
                            ("label", json_str(&choice.label)),
                            ("impulse", num(choice.impulse)),
                        ])
                    })
                    .collect()),
            ),
        ]),
        Q1CompositionEvent::ClearPrompt { actor } => {
            obj(vec![("kind", json_str("clear-prompt")), ("actor", wire_actor(actor))])
        }
        Q1CompositionEvent::SourceLog { actor, action } => obj(vec![
            ("kind", json_str("source-log")),
            ("actor", wire_actor(actor)),
            ("action", json_str(action)),
        ]),
        Q1CompositionEvent::DeveloperMessage { text } => {
            obj(vec![("kind", json_str("developer-message")), ("text", json_str(text))])
        }
        Q1CompositionEvent::LevelPresentation { event } => obj(vec![
            ("kind", json_str("level-presentation")),
            ("event", write_source_finale(event)),
        ]),
    }
}

/// Q1 intermission result.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1IntermissionResult {
    /// Waiting for players.
    Waiting,
    /// Travel to a map.
    Travel {
        /// Map name.
        map: String,
    },
    /// Finale text.
    Finale {
        /// Text.
        text: String,
        /// Music track.
        track: f64,
    },
    /// Sell screen.
    SellScreen,
}

fn read_q1_intermission_result(reader: SaveReader) -> Result<Q1IntermissionResult, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "waiting" => Ok(Q1IntermissionResult::Waiting),
        "travel" => Ok(Q1IntermissionResult::Travel {
            map: reader.field("map").string()?,
        }),
        "finale" => Ok(Q1IntermissionResult::Finale {
            text: reader.field("text").string()?,
            track: reader.field("track").finite()?,
        }),
        "sell-screen" => Ok(Q1IntermissionResult::SellScreen),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q1_intermission_result(result: &Q1IntermissionResult) -> SaveJson {
    match result {
        Q1IntermissionResult::Waiting => obj(vec![("kind", json_str("waiting"))]),
        Q1IntermissionResult::Travel { map } => obj(vec![("kind", json_str("travel")), ("map", json_str(map))]),
        Q1IntermissionResult::Finale { text, track } => obj(vec![
            ("kind", json_str("finale")),
            ("text", json_str(text)),
            ("track", num(*track)),
        ]),
        Q1IntermissionResult::SellScreen => obj(vec![("kind", json_str("sell-screen"))]),
    }
}

/// Print level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2PrintLevel {
    /// Low priority.
    Low,
    /// Medium priority.
    Medium,
    /// High priority.
    High,
    /// Chat.
    Chat,
}

fn read_print_level(reader: SaveReader) -> Result<Q2PrintLevel, WorldError> {
    match reader.choice_str(&["low", "medium", "high", "chat"])?.as_str() {
        "low" => Ok(Q2PrintLevel::Low),
        "medium" => Ok(Q2PrintLevel::Medium),
        "high" => Ok(Q2PrintLevel::High),
        _ => Ok(Q2PrintLevel::Chat),
    }
}

fn write_print_level(level: Q2PrintLevel) -> SaveJson {
    json_str(match level {
        Q2PrintLevel::Low => "low",
        Q2PrintLevel::Medium => "medium",
        Q2PrintLevel::High => "high",
        Q2PrintLevel::Chat => "chat",
    })
}

/// Sound loop mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2SoundLoop {
    /// Start looping.
    Start,
    /// Stop looping.
    Stop,
    /// Play once.
    Once,
}

/// Dynamic-light cone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2LightCone {
    /// Direction.
    pub direction: Vec3,
    /// Cosine half-angle.
    pub cos_half_angle: f64,
}

/// Q2 presentation event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PresentationEvent {
    /// Model update.
    Model {
        /// Actor handle.
        actor: ActorId,
        /// Model path.
        path: String,
        /// Attached models.
        attached_models: Vec<String>,
        /// Frame.
        frame: f64,
        /// Previous frame.
        old_frame: f64,
        /// Scale.
        scale: f64,
        /// Alpha.
        alpha: f64,
        /// Skin.
        skin: f64,
        /// Effects.
        effects: f64,
        /// Render flags.
        render_flags: f64,
    },
    /// Visibility update.
    Visibility {
        /// Actor handle.
        actor: ActorId,
        /// Visible flag.
        visible: bool,
    },
    /// Sound.
    Sound {
        /// Actor handle.
        actor: Option<ActorId>,
        /// Origin.
        origin: Vec3,
        /// Sound path.
        path: String,
        /// Channel.
        channel: f64,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
        /// Reliable flag.
        reliable: bool,
        /// Loop mode.
        loop_mode: Q2SoundLoop,
        /// Loop owner.
        loop_owner: Option<String>,
    },
    /// Center print.
    Centerprint {
        /// Actor handle.
        actor: ActorId,
        /// Text.
        text: String,
        /// Instant flag.
        instant: Option<bool>,
        /// Duration in seconds.
        duration_seconds: Option<f64>,
    },
    /// Print.
    Print {
        /// Actor handle.
        actor: Option<ActorId>,
        /// Level.
        level: Q2PrintLevel,
        /// Text.
        text: String,
    },
    /// Help text.
    Help {
        /// Slot.
        slot: i64,
        /// Text.
        text: String,
    },
    /// Light style.
    Lightstyle {
        /// Style index.
        style: f64,
        /// Pattern.
        pattern: String,
    },
    /// Music track.
    Music {
        /// Track path.
        track: String,
    },
    /// Particle effect.
    Effect {
        /// Effect name.
        effect: String,
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Count.
        count: f64,
        /// Color.
        color: f64,
    },
    /// Damage indicator.
    DamageIndicator {
        /// Actor handle.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Amount.
        amount: f64,
    },
    /// Item pickup.
    Pickup {
        /// Player actor.
        player: ActorId,
        /// Item id.
        item: String,
        /// Icon path.
        icon: String,
        /// Display name.
        name: String,
    },
    /// Point of interest.
    Poi {
        /// Origin.
        origin: Vec3,
        /// Message.
        message: String,
        /// Fields.
        fields: BTreeMap<String, String>,
    },
    /// Dynamic light.
    DynamicLight {
        /// Actor handle.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Color.
        color: Vec3,
        /// Visible flag.
        visible: bool,
        /// Radius.
        radius: f64,
        /// Intensity.
        intensity: f64,
        /// Resolution.
        resolution: f64,
        /// Fade start.
        fade_start: f64,
        /// Fade end.
        fade_end: f64,
        /// Light style.
        lightstyle: f64,
        /// Spot cone.
        cone: Option<Q2LightCone>,
    },
    /// Beam.
    Beam {
        /// Actor handle.
        actor: ActorId,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Width.
        width: f64,
        /// Color.
        color: f64,
        /// Visible flag.
        visible: bool,
    },
    /// Monster beam.
    MonsterBeam {
        /// Beam effect.
        effect: Q2MonsterBeam,
        /// Actor handle.
        actor: ActorId,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
    },
    /// Monster muzzle flash.
    MonsterMuzzleflash {
        /// Actor handle.
        actor: ActorId,
        /// Flash number.
        flash: f64,
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
    },
    /// Entity event.
    EntityEvent {
        /// Actor handle.
        actor: ActorId,
        /// Event number.
        event: f64,
    },
}

/// Monster beam effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2MonsterBeam {
    /// Parasite beam.
    Parasite,
    /// Medic beam.
    Medic,
}

fn read_q2_presentation_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2PresentationEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "model" => Ok(Q2PresentationEvent::Model {
            actor: read_actor(reader.field("actor"), identity)?,
            path: reader.field("path").string()?,
            attached_models: bounded_list(reader.field("attachedModels"), |item| item.string())?,
            frame: reader.field("frame").finite()?,
            old_frame: reader.field("oldFrame").finite()?,
            scale: reader.field("scale").finite()?,
            alpha: reader.field("alpha").finite()?,
            skin: reader.field("skin").finite()?,
            effects: reader.field("effects").finite()?,
            render_flags: reader.field("renderFlags").finite()?,
        }),
        "visibility" => Ok(Q2PresentationEvent::Visibility {
            actor: read_actor(reader.field("actor"), identity)?,
            visible: reader.field("visible").boolean()?,
        }),
        "sound" => {
            let loop_owner = reader.field("loopOwner");
            let loop_mode = reader.field("loop").choice_str(&["start", "stop", "once"])?;
            Ok(Q2PresentationEvent::Sound {
                actor: read_opt_actor(reader.field("actor"), identity)?,
                origin: read_vector(reader.field("origin"))?,
                path: reader.field("path").string()?,
                channel: reader.field("channel").finite()?,
                volume: reader.field("volume").finite()?,
                attenuation: reader.field("attenuation").finite()?,
                reliable: reader.field("reliable").boolean()?,
                loop_mode: match loop_mode.as_str() {
                    "start" => Q2SoundLoop::Start,
                    "stop" => Q2SoundLoop::Stop,
                    _ => Q2SoundLoop::Once,
                },
                loop_owner: if loop_owner.value.is_none() {
                    None
                } else {
                    Some(namespaced(loop_owner)?)
                },
            })
        }
        "centerprint" => {
            let instant = reader.field("instant");
            let duration = reader.field("durationSeconds");
            Ok(Q2PresentationEvent::Centerprint {
                actor: read_actor(reader.field("actor"), identity)?,
                text: reader.field("text").string()?,
                instant: if instant.value.is_none() {
                    None
                } else {
                    Some(instant.boolean()?)
                },
                duration_seconds: if duration.value.is_none() {
                    None
                } else {
                    Some(duration.finite()?)
                },
            })
        }
        "print" => Ok(Q2PresentationEvent::Print {
            actor: read_opt_actor(reader.field("actor"), identity)?,
            level: read_print_level(reader.field("level"))?,
            text: reader.field("text").string()?,
        }),
        "help" => Ok(Q2PresentationEvent::Help {
            slot: reader.field("slot").choice_i64(&[1, 2])?,
            text: reader.field("text").string()?,
        }),
        "lightstyle" => {
            reader.field("kind").literal_str("lightstyle")?;
            Ok(Q2PresentationEvent::Lightstyle {
                style: reader.field("style").finite()?,
                pattern: reader.field("pattern").string()?,
            })
        }
        "music" => Ok(Q2PresentationEvent::Music {
            track: reader.field("track").string()?,
        }),
        "effect" => Ok(Q2PresentationEvent::Effect {
            effect: reader.field("effect").string()?,
            origin: read_vector(reader.field("origin"))?,
            direction: read_vector(reader.field("direction"))?,
            count: reader.field("count").finite()?,
            color: reader.field("color").finite()?,
        }),
        "damage-indicator" => Ok(Q2PresentationEvent::DamageIndicator {
            actor: read_actor(reader.field("actor"), identity)?,
            origin: read_vector(reader.field("origin"))?,
            amount: reader.field("amount").finite()?,
        }),
        "pickup" => Ok(Q2PresentationEvent::Pickup {
            player: read_actor(reader.field("player"), identity)?,
            item: namespaced(reader.field("item"))?,
            icon: reader.field("icon").string()?,
            name: reader.field("name").string()?,
        }),
        "poi" => {
            let mut fields = BTreeMap::new();
            bounded_list(reader.field("fields"), |item| {
                let key = item.field("key").string()?;
                if fields.contains_key(&key) {
                    return Err(item.fail("duplicate map key"));
                }
                fields.insert(key, item.field("value").string()?);
                Ok(())
            })?;
            Ok(Q2PresentationEvent::Poi {
                origin: read_vector(reader.field("origin"))?,
                message: reader.field("message").string()?,
                fields,
            })
        }
        "dynamic-light" => {
            let cone = reader.field("cone");
            Ok(Q2PresentationEvent::DynamicLight {
                actor: read_actor(reader.field("actor"), identity)?,
                origin: read_vector(reader.field("origin"))?,
                color: read_vector(reader.field("color"))?,
                visible: reader.field("visible").boolean()?,
                radius: reader.field("radius").finite()?,
                intensity: reader.field("intensity").finite()?,
                resolution: reader.field("resolution").finite()?,
                fade_start: reader.field("fadeStart").finite()?,
                fade_end: reader.field("fadeEnd").finite()?,
                lightstyle: reader.field("lightstyle").finite()?,
                cone: if cone.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(Q2LightCone {
                        direction: read_vector(cone.field("direction"))?,
                        cos_half_angle: cone.field("cosHalfAngle").finite()?,
                    })
                },
            })
        }
        "beam" => Ok(Q2PresentationEvent::Beam {
            actor: read_actor(reader.field("actor"), identity)?,
            start: read_vector(reader.field("start"))?,
            end: read_vector(reader.field("end"))?,
            width: reader.field("width").finite()?,
            color: reader.field("color").finite()?,
            visible: reader.field("visible").boolean()?,
        }),
        "monster-beam" => {
            let effect = reader.field("effect").choice_str(&["parasite", "medic"])?;
            Ok(Q2PresentationEvent::MonsterBeam {
                effect: if effect == "parasite" {
                    Q2MonsterBeam::Parasite
                } else {
                    Q2MonsterBeam::Medic
                },
                actor: read_actor(reader.field("actor"), identity)?,
                start: read_vector(reader.field("start"))?,
                end: read_vector(reader.field("end"))?,
            })
        }
        "monster-muzzleflash" => Ok(Q2PresentationEvent::MonsterMuzzleflash {
            actor: read_actor(reader.field("actor"), identity)?,
            flash: reader.field("flash").finite()?,
            origin: read_vector(reader.field("origin"))?,
            direction: read_vector(reader.field("direction"))?,
        }),
        "entity-event" => Ok(Q2PresentationEvent::EntityEvent {
            actor: read_actor(reader.field("actor"), identity)?,
            event: reader.field("event").finite()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q2_presentation_event(event: &Q2PresentationEvent) -> SaveJson {
    match event {
        Q2PresentationEvent::Model {
            actor,
            path,
            attached_models,
            frame,
            old_frame,
            scale,
            alpha,
            skin,
            effects,
            render_flags,
        } => obj(vec![
            ("kind", json_str("model")),
            ("actor", wire_actor(actor)),
            ("path", json_str(path)),
            (
                "attachedModels",
                arr(attached_models.iter().map(|model| json_str(model)).collect()),
            ),
            ("frame", num(*frame)),
            ("oldFrame", num(*old_frame)),
            ("scale", num(*scale)),
            ("alpha", num(*alpha)),
            ("skin", num(*skin)),
            ("effects", num(*effects)),
            ("renderFlags", num(*render_flags)),
        ]),
        Q2PresentationEvent::Visibility { actor, visible } => obj(vec![
            ("kind", json_str("visibility")),
            ("actor", wire_actor(actor)),
            ("visible", boolean(*visible)),
        ]),
        Q2PresentationEvent::Sound {
            actor,
            origin,
            path,
            channel,
            volume,
            attenuation,
            reliable,
            loop_mode,
            loop_owner,
        } => {
            let mut members = vec![
                ("kind", json_str("sound")),
                ("actor", write_opt_actor(actor.as_ref())),
                ("origin", write_vector(*origin)),
                ("path", json_str(path)),
                ("channel", num(*channel)),
                ("volume", num(*volume)),
                ("attenuation", num(*attenuation)),
                ("reliable", boolean(*reliable)),
                (
                    "loop",
                    json_str(match loop_mode {
                        Q2SoundLoop::Start => "start",
                        Q2SoundLoop::Stop => "stop",
                        Q2SoundLoop::Once => "once",
                    }),
                ),
            ];
            if let Some(owner) = loop_owner {
                members.push(("loopOwner", json_str(owner)));
            }
            obj(members)
        }
        Q2PresentationEvent::Centerprint {
            actor,
            text,
            instant,
            duration_seconds,
        } => {
            let mut members = vec![
                ("kind", json_str("centerprint")),
                ("actor", wire_actor(actor)),
                ("text", json_str(text)),
            ];
            if let Some(instant) = instant {
                members.push(("instant", boolean(*instant)));
            }
            if let Some(duration) = duration_seconds {
                members.push(("durationSeconds", num(*duration)));
            }
            obj(members)
        }
        Q2PresentationEvent::Print { actor, level, text } => obj(vec![
            ("kind", json_str("print")),
            ("actor", write_opt_actor(actor.as_ref())),
            ("level", write_print_level(*level)),
            ("text", json_str(text)),
        ]),
        Q2PresentationEvent::Help { slot, text } => obj(vec![
            ("kind", json_str("help")),
            ("slot", int(*slot)),
            ("text", json_str(text)),
        ]),
        Q2PresentationEvent::Lightstyle { style, pattern } => write_lightstyle(*style, pattern),
        Q2PresentationEvent::Music { track } => obj(vec![("kind", json_str("music")), ("track", json_str(track))]),
        Q2PresentationEvent::Effect {
            effect,
            origin,
            direction,
            count,
            color,
        } => obj(vec![
            ("kind", json_str("effect")),
            ("effect", json_str(effect)),
            ("origin", write_vector(*origin)),
            ("direction", write_vector(*direction)),
            ("count", num(*count)),
            ("color", num(*color)),
        ]),
        Q2PresentationEvent::DamageIndicator { actor, origin, amount } => obj(vec![
            ("kind", json_str("damage-indicator")),
            ("actor", wire_actor(actor)),
            ("origin", write_vector(*origin)),
            ("amount", num(*amount)),
        ]),
        Q2PresentationEvent::Pickup {
            player,
            item,
            icon,
            name,
        } => obj(vec![
            ("kind", json_str("pickup")),
            ("player", wire_actor(player)),
            ("item", json_str(item)),
            ("icon", json_str(icon)),
            ("name", json_str(name)),
        ]),
        Q2PresentationEvent::Poi {
            origin,
            message,
            fields,
        } => obj(vec![
            ("kind", json_str("poi")),
            ("origin", write_vector(*origin)),
            ("message", json_str(message)),
            (
                "fields",
                arr(fields
                    .iter()
                    .map(|(key, value)| obj(vec![("key", json_str(key)), ("value", json_str(value))]))
                    .collect()),
            ),
        ]),
        Q2PresentationEvent::DynamicLight {
            actor,
            origin,
            color,
            visible,
            radius,
            intensity,
            resolution,
            fade_start,
            fade_end,
            lightstyle,
            cone,
        } => obj(vec![
            ("kind", json_str("dynamic-light")),
            ("actor", wire_actor(actor)),
            ("origin", write_vector(*origin)),
            ("color", write_vector(*color)),
            ("visible", boolean(*visible)),
            ("radius", num(*radius)),
            ("intensity", num(*intensity)),
            ("resolution", num(*resolution)),
            ("fadeStart", num(*fade_start)),
            ("fadeEnd", num(*fade_end)),
            ("lightstyle", num(*lightstyle)),
            (
                "cone",
                cone.map_or(SaveJson::Null, |cone| {
                    obj(vec![
                        ("direction", write_vector(cone.direction)),
                        ("cosHalfAngle", num(cone.cos_half_angle)),
                    ])
                }),
            ),
        ]),
        Q2PresentationEvent::Beam {
            actor,
            start,
            end,
            width,
            color,
            visible,
        } => obj(vec![
            ("kind", json_str("beam")),
            ("actor", wire_actor(actor)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
            ("width", num(*width)),
            ("color", num(*color)),
            ("visible", boolean(*visible)),
        ]),
        Q2PresentationEvent::MonsterBeam {
            effect,
            actor,
            start,
            end,
        } => obj(vec![
            ("kind", json_str("monster-beam")),
            (
                "effect",
                json_str(match effect {
                    Q2MonsterBeam::Parasite => "parasite",
                    Q2MonsterBeam::Medic => "medic",
                }),
            ),
            ("actor", wire_actor(actor)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
        ]),
        Q2PresentationEvent::MonsterMuzzleflash {
            actor,
            flash,
            origin,
            direction,
        } => obj(vec![
            ("kind", json_str("monster-muzzleflash")),
            ("actor", wire_actor(actor)),
            ("flash", num(*flash)),
            ("origin", write_vector(*origin)),
            ("direction", write_vector(*direction)),
        ]),
        Q2PresentationEvent::EntityEvent { actor, event } => obj(vec![
            ("kind", json_str("entity-event")),
            ("actor", wire_actor(actor)),
            ("event", num(*event)),
        ]),
    }
}

/// Q2 beam effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2BeamEffect {
    /// Railgun trail.
    Rail,
    /// Railgun water trail.
    RailWater,
    /// BFG laser.
    BfgLaser,
    /// BFG zap.
    BfgZap,
    /// Bubble trail.
    BubbleTrail,
    /// BFG lightning.
    BfgLightning,
    /// Heat beam.
    Heatbeam,
    /// Monster heat beam.
    MonsterHeatbeam,
}

/// Player animation priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2AnimationPriority {
    /// Attack.
    Attack,
    /// Pain.
    Pain,
    /// Reverse.
    Reverse,
}

/// Q2 weapon event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2WeaponEvent {
    /// Muzzle flash.
    Muzzleflash {
        /// Actor handle.
        actor: ActorId,
        /// Flash number.
        flash: f64,
        /// Silenced flag.
        silenced: bool,
    },
    /// Beam.
    Beam {
        /// Beam effect.
        effect: Q2BeamEffect,
        /// Actor handle.
        actor: Option<ActorId>,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Duration.
        duration: f64,
    },
    /// View weapon.
    ViewWeapon {
        /// Actor handle.
        actor: ActorId,
        /// Weapon name.
        weapon: Option<String>,
        /// Model path.
        model: String,
        /// Player model.
        player_model: f64,
        /// Frame.
        frame: f64,
        /// Skin.
        skin: f64,
        /// Rate.
        rate: f64,
        /// Kick origin.
        kick_origin: Vec3,
        /// Kick angles.
        kick_angles: Vec3,
    },
    /// Player animation.
    PlayerAnimation {
        /// Actor handle.
        actor: ActorId,
        /// Priority.
        priority: Q2AnimationPriority,
        /// First frame.
        first: f64,
        /// Last frame.
        last: f64,
        /// Reset time.
        reset_time: bool,
    },
    /// Invisibility reveal.
    InvisibilityReveal {
        /// Actor handle.
        actor: ActorId,
        /// Reveal time.
        until: f64,
    },
}

fn read_q2_weapon_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2WeaponEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "muzzleflash" => Ok(Q2WeaponEvent::Muzzleflash {
            actor: read_actor(reader.field("actor"), identity)?,
            flash: reader.field("flash").finite()?,
            silenced: reader.field("silenced").boolean()?,
        }),
        "beam" => {
            let effect = reader.field("effect").choice_str(&[
                "rail",
                "rail-water",
                "bfg-laser",
                "bfg-zap",
                "bubble-trail",
                "bfg-lightning",
                "heatbeam",
                "monster-heatbeam",
            ])?;
            Ok(Q2WeaponEvent::Beam {
                effect: match effect.as_str() {
                    "rail" => Q2BeamEffect::Rail,
                    "rail-water" => Q2BeamEffect::RailWater,
                    "bfg-laser" => Q2BeamEffect::BfgLaser,
                    "bfg-zap" => Q2BeamEffect::BfgZap,
                    "bubble-trail" => Q2BeamEffect::BubbleTrail,
                    "bfg-lightning" => Q2BeamEffect::BfgLightning,
                    "heatbeam" => Q2BeamEffect::Heatbeam,
                    _ => Q2BeamEffect::MonsterHeatbeam,
                },
                actor: reader.field("actor").nullable(|actor| read_actor(actor, identity))?,
                start: read_vector(reader.field("start"))?,
                end: read_vector(reader.field("end"))?,
                duration: reader.field("duration").finite()?,
            })
        }
        "view-weapon" => {
            let weapon = reader.field("weapon");
            Ok(Q2WeaponEvent::ViewWeapon {
                actor: read_actor(reader.field("actor"), identity)?,
                weapon: if weapon.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(weapon.string()?)
                },
                model: reader.field("model").string()?,
                player_model: reader.field("playerModel").finite()?,
                frame: reader.field("frame").finite()?,
                skin: reader.field("skin").finite()?,
                rate: reader.field("rate").finite()?,
                kick_origin: read_vector(reader.field("kickOrigin"))?,
                kick_angles: read_vector(reader.field("kickAngles"))?,
            })
        }
        "player-animation" => {
            let priority = reader.field("priority").choice_str(&["attack", "pain", "reverse"])?;
            Ok(Q2WeaponEvent::PlayerAnimation {
                actor: read_actor(reader.field("actor"), identity)?,
                priority: match priority.as_str() {
                    "attack" => Q2AnimationPriority::Attack,
                    "pain" => Q2AnimationPriority::Pain,
                    _ => Q2AnimationPriority::Reverse,
                },
                first: reader.field("first").finite()?,
                last: reader.field("last").finite()?,
                reset_time: reader.field("resetTime").boolean()?,
            })
        }
        "invisibility-reveal" => Ok(Q2WeaponEvent::InvisibilityReveal {
            actor: read_actor(reader.field("actor"), identity)?,
            until: reader.field("until").finite()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q2_weapon_event(event: &Q2WeaponEvent) -> SaveJson {
    match event {
        Q2WeaponEvent::Muzzleflash { actor, flash, silenced } => obj(vec![
            ("kind", json_str("muzzleflash")),
            ("actor", wire_actor(actor)),
            ("flash", num(*flash)),
            ("silenced", boolean(*silenced)),
        ]),
        Q2WeaponEvent::Beam {
            effect,
            actor,
            start,
            end,
            duration,
        } => obj(vec![
            ("kind", json_str("beam")),
            (
                "effect",
                json_str(match effect {
                    Q2BeamEffect::Rail => "rail",
                    Q2BeamEffect::RailWater => "rail-water",
                    Q2BeamEffect::BfgLaser => "bfg-laser",
                    Q2BeamEffect::BfgZap => "bfg-zap",
                    Q2BeamEffect::BubbleTrail => "bubble-trail",
                    Q2BeamEffect::BfgLightning => "bfg-lightning",
                    Q2BeamEffect::Heatbeam => "heatbeam",
                    Q2BeamEffect::MonsterHeatbeam => "monster-heatbeam",
                }),
            ),
            ("actor", actor.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
            ("duration", num(*duration)),
        ]),
        Q2WeaponEvent::ViewWeapon {
            actor,
            weapon,
            model,
            player_model,
            frame,
            skin,
            rate,
            kick_origin,
            kick_angles,
        } => obj(vec![
            ("kind", json_str("view-weapon")),
            ("actor", wire_actor(actor)),
            (
                "weapon",
                weapon.as_ref().map_or(SaveJson::Null, |weapon| json_str(weapon)),
            ),
            ("model", json_str(model)),
            ("playerModel", num(*player_model)),
            ("frame", num(*frame)),
            ("skin", num(*skin)),
            ("rate", num(*rate)),
            ("kickOrigin", write_vector(*kick_origin)),
            ("kickAngles", write_vector(*kick_angles)),
        ]),
        Q2WeaponEvent::PlayerAnimation {
            actor,
            priority,
            first,
            last,
            reset_time,
        } => obj(vec![
            ("kind", json_str("player-animation")),
            ("actor", wire_actor(actor)),
            (
                "priority",
                json_str(match priority {
                    Q2AnimationPriority::Attack => "attack",
                    Q2AnimationPriority::Pain => "pain",
                    Q2AnimationPriority::Reverse => "reverse",
                }),
            ),
            ("first", num(*first)),
            ("last", num(*last)),
            ("resetTime", boolean(*reset_time)),
        ]),
        Q2WeaponEvent::InvisibilityReveal { actor, until } => obj(vec![
            ("kind", json_str("invisibility-reveal")),
            ("actor", wire_actor(actor)),
            ("until", num(*until)),
        ]),
    }
}

/// CTF scoreboard row.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfScoreRow {
    /// Actor handle.
    pub actor: ActorId,
    /// Slot.
    pub slot: f64,
    /// Name.
    pub name: String,
    /// Score.
    pub score: f64,
    /// Ping.
    pub ping: f64,
}

fn read_ctf_score_row(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q2CtfScoreRow, WorldError> {
    Ok(Q2CtfScoreRow {
        actor: read_actor(reader.field("actor"), identity)?,
        slot: reader.field("slot").finite()?,
        name: reader.field("name").string()?,
        score: reader.field("score").finite()?,
        ping: reader.field("ping").finite()?,
    })
}

fn write_ctf_score_row(row: &Q2CtfScoreRow) -> SaveJson {
    obj(vec![
        ("actor", wire_actor(&row.actor)),
        ("slot", num(row.slot)),
        ("name", json_str(&row.name)),
        ("score", num(row.score)),
        ("ping", num(row.ping)),
    ])
}

/// CTF flag state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2FlagState {
    /// At home.
    Home,
    /// Carried.
    Carried,
    /// Dropped.
    Dropped,
}

fn read_flag_state(reader: SaveReader) -> Result<Q2FlagState, WorldError> {
    match reader.choice_str(&["home", "carried", "dropped"])?.as_str() {
        "home" => Ok(Q2FlagState::Home),
        "carried" => Ok(Q2FlagState::Carried),
        _ => Ok(Q2FlagState::Dropped),
    }
}

fn write_flag_state(state: Q2FlagState) -> SaveJson {
    json_str(match state {
        Q2FlagState::Home => "home",
        Q2FlagState::Carried => "carried",
        Q2FlagState::Dropped => "dropped",
    })
}

/// CTF technology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2CtfTech {
    /// Resistance.
    Resistance,
    /// Strength.
    Strength,
    /// Haste.
    Haste,
    /// Regeneration.
    Regeneration,
}

/// CTF menu action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2CtfMenuAction {
    /// Join red.
    JoinRed,
    /// Join blue.
    JoinBlue,
    /// Join game.
    JoinGame,
    /// Observe.
    Observe,
    /// Chase camera.
    ChaseCam,
    /// Player list.
    PlayerList,
    /// Statistics.
    Stats,
    /// MOTD.
    Motd,
    /// Settings.
    Settings,
    /// Leave.
    Leave,
    /// Credits.
    Credits,
    /// Close menu.
    Close,
}

/// Q2 CTF event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2CtfEvent {
    /// Scoreboard.
    Scoreboard {
        /// Actor handle.
        actor: ActorId,
        /// Red rows.
        red: Vec<Q2CtfScoreRow>,
        /// Blue rows.
        blue: Vec<Q2CtfScoreRow>,
        /// Spectator rows.
        spectators: Vec<Q2CtfScoreRow>,
        /// Captures.
        captures: (f64, f64),
        /// Totals.
        totals: (f64, f64),
        /// Layout.
        layout: Vec<String>,
    },
    /// HUD state.
    Hud {
        /// Actor handle.
        actor: ActorId,
        /// Captures.
        captures: (f64, f64),
        /// Flag states.
        flag_states: (Q2FlagState, Q2FlagState),
        /// Team.
        team: f64,
        /// Carried flag.
        carried_flag: Option<i64>,
        /// Technology.
        tech: Option<Q2CtfTech>,
        /// ID target.
        id_target: Option<ActorId>,
        /// Blinking team.
        blink_team: Option<i64>,
        /// Match state.
        match_state: String,
    },
    /// Menu.
    Menu {
        /// Actor handle.
        actor: ActorId,
        /// Title.
        title: String,
        /// Entries.
        entries: Vec<Q2CtfMenuEntry>,
    },
    /// Match status.
    MatchStatus {
        /// Text.
        text: String,
    },
    /// Admin settings.
    AdminSettings {
        /// Actor handle.
        actor: ActorId,
        /// Settings.
        settings: Q2CtfAdminSettings,
    },
    /// Grapple cable.
    GrappleCable {
        /// Actor handle.
        actor: ActorId,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Offset.
        offset: Vec3,
    },
}

/// CTF menu entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2CtfMenuEntry {
    /// Label.
    pub label: String,
    /// Action.
    pub action: Option<Q2CtfMenuAction>,
}

/// CTF admin settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2CtfAdminSettings {
    /// Match time.
    pub match_time: String,
    /// Match setup.
    pub match_setup: String,
    /// Match start time.
    pub match_start_time: String,
    /// Capture limit.
    pub capture_limit: String,
    /// Use 3D target.
    pub use_3d_target: String,
    /// Allow ghost.
    pub allow_ghost: String,
    /// Allow grapple.
    pub allow_grapple: String,
    /// Allow tech.
    pub allow_tech: String,
    /// Use spawn farthest.
    pub use_spawn_farthest: String,
}

fn read_ctf_event(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q2CtfEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "scoreboard" => {
            let captures = reader.field("captures");
            let totals = reader.field("totals");
            Ok(Q2CtfEvent::Scoreboard {
                actor: read_actor(reader.field("actor"), identity)?,
                red: bounded_list(reader.field("red"), |row| read_ctf_score_row(row, identity))?,
                blue: bounded_list(reader.field("blue"), |row| read_ctf_score_row(row, identity))?,
                spectators: bounded_list(reader.field("spectators"), |row| read_ctf_score_row(row, identity))?,
                captures: (captures.field("red").finite()?, captures.field("blue").finite()?),
                totals: (totals.field("red").finite()?, totals.field("blue").finite()?),
                layout: bounded_list(reader.field("layout"), |item| item.string())?,
            })
        }
        "hud" => {
            let captures = reader.field("captures");
            let flag_states = reader.field("flagStates");
            let carried = reader.field("carriedFlag");
            let tech = reader.field("tech");
            let id_target = reader.field("idTarget");
            let blink_team = reader.field("blinkTeam");
            Ok(Q2CtfEvent::Hud {
                actor: read_actor(reader.field("actor"), identity)?,
                captures: (captures.field("red").finite()?, captures.field("blue").finite()?),
                flag_states: (
                    read_flag_state(flag_states.field("red"))?,
                    read_flag_state(flag_states.field("blue"))?,
                ),
                team: reader.field("team").finite()?,
                carried_flag: if carried.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(carried.choice_i64(&[1, 2])?)
                },
                tech: if tech.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(
                        match tech
                            .choice_str(&["resistance", "strength", "haste", "regeneration"])?
                            .as_str()
                        {
                            "resistance" => Q2CtfTech::Resistance,
                            "strength" => Q2CtfTech::Strength,
                            "haste" => Q2CtfTech::Haste,
                            _ => Q2CtfTech::Regeneration,
                        },
                    )
                },
                id_target: if id_target.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(read_actor(id_target, identity)?)
                },
                blink_team: if blink_team.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(blink_team.choice_i64(&[1, 2])?)
                },
                match_state: reader.field("match").string()?,
            })
        }
        "menu" => Ok(Q2CtfEvent::Menu {
            actor: read_actor(reader.field("actor"), identity)?,
            title: reader.field("title").string()?,
            entries: bounded_list(reader.field("entries"), |entry| {
                let action = entry.field("action");
                Ok(Q2CtfMenuEntry {
                    label: entry.field("label").string()?,
                    action: if action.value == Some(&SaveJson::Null) {
                        None
                    } else {
                        Some(
                            match action
                                .choice_str(&[
                                    "join-red",
                                    "join-blue",
                                    "join-game",
                                    "observe",
                                    "chase-cam",
                                    "player-list",
                                    "stats",
                                    "motd",
                                    "settings",
                                    "leave",
                                    "credits",
                                    "close",
                                ])?
                                .as_str()
                            {
                                "join-red" => Q2CtfMenuAction::JoinRed,
                                "join-blue" => Q2CtfMenuAction::JoinBlue,
                                "join-game" => Q2CtfMenuAction::JoinGame,
                                "observe" => Q2CtfMenuAction::Observe,
                                "chase-cam" => Q2CtfMenuAction::ChaseCam,
                                "player-list" => Q2CtfMenuAction::PlayerList,
                                "stats" => Q2CtfMenuAction::Stats,
                                "motd" => Q2CtfMenuAction::Motd,
                                "settings" => Q2CtfMenuAction::Settings,
                                "leave" => Q2CtfMenuAction::Leave,
                                "credits" => Q2CtfMenuAction::Credits,
                                _ => Q2CtfMenuAction::Close,
                            },
                        )
                    },
                })
            })?,
        }),
        "match-status" => Ok(Q2CtfEvent::MatchStatus {
            text: reader.field("text").string()?,
        }),
        "admin-settings" => {
            let settings = reader.field("settings");
            Ok(Q2CtfEvent::AdminSettings {
                actor: read_actor(reader.field("actor"), identity)?,
                settings: Q2CtfAdminSettings {
                    match_time: settings.field("matchTime").string()?,
                    match_setup: settings.field("matchSetup").string()?,
                    match_start_time: settings.field("matchStartTime").string()?,
                    capture_limit: settings.field("captureLimit").string()?,
                    use_3d_target: settings.field("use3dTarget").string()?,
                    allow_ghost: settings.field("allowGhost").string()?,
                    allow_grapple: settings.field("allowGrapple").string()?,
                    allow_tech: settings.field("allowTech").string()?,
                    use_spawn_farthest: settings.field("useSpawnFarthest").string()?,
                },
            })
        }
        "grapple-cable" => Ok(Q2CtfEvent::GrappleCable {
            actor: read_actor(reader.field("actor"), identity)?,
            start: read_vector(reader.field("start"))?,
            end: read_vector(reader.field("end"))?,
            offset: read_vector(reader.field("offset"))?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_ctf_menu_action(action: Q2CtfMenuAction) -> SaveJson {
    json_str(match action {
        Q2CtfMenuAction::JoinRed => "join-red",
        Q2CtfMenuAction::JoinBlue => "join-blue",
        Q2CtfMenuAction::JoinGame => "join-game",
        Q2CtfMenuAction::Observe => "observe",
        Q2CtfMenuAction::ChaseCam => "chase-cam",
        Q2CtfMenuAction::PlayerList => "player-list",
        Q2CtfMenuAction::Stats => "stats",
        Q2CtfMenuAction::Motd => "motd",
        Q2CtfMenuAction::Settings => "settings",
        Q2CtfMenuAction::Leave => "leave",
        Q2CtfMenuAction::Credits => "credits",
        Q2CtfMenuAction::Close => "close",
    })
}

fn write_ctf_event(event: &Q2CtfEvent) -> SaveJson {
    match event {
        Q2CtfEvent::Scoreboard {
            actor,
            red,
            blue,
            spectators,
            captures,
            totals,
            layout,
        } => obj(vec![
            ("kind", json_str("scoreboard")),
            ("actor", wire_actor(actor)),
            ("red", arr(red.iter().map(write_ctf_score_row).collect())),
            ("blue", arr(blue.iter().map(write_ctf_score_row).collect())),
            ("spectators", arr(spectators.iter().map(write_ctf_score_row).collect())),
            (
                "captures",
                obj(vec![("red", num(captures.0)), ("blue", num(captures.1))]),
            ),
            ("totals", obj(vec![("red", num(totals.0)), ("blue", num(totals.1))])),
            ("layout", arr(layout.iter().map(|line| json_str(line)).collect())),
        ]),
        Q2CtfEvent::Hud {
            actor,
            captures,
            flag_states,
            team,
            carried_flag,
            tech,
            id_target,
            blink_team,
            match_state,
        } => obj(vec![
            ("kind", json_str("hud")),
            ("actor", wire_actor(actor)),
            (
                "captures",
                obj(vec![("red", num(captures.0)), ("blue", num(captures.1))]),
            ),
            (
                "flagStates",
                obj(vec![
                    ("red", write_flag_state(flag_states.0)),
                    ("blue", write_flag_state(flag_states.1)),
                ]),
            ),
            ("team", num(*team)),
            ("carriedFlag", carried_flag.map_or(SaveJson::Null, int)),
            (
                "tech",
                tech.map_or(SaveJson::Null, |tech| {
                    json_str(match tech {
                        Q2CtfTech::Resistance => "resistance",
                        Q2CtfTech::Strength => "strength",
                        Q2CtfTech::Haste => "haste",
                        Q2CtfTech::Regeneration => "regeneration",
                    })
                }),
            ),
            ("idTarget", id_target.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("blinkTeam", blink_team.map_or(SaveJson::Null, int)),
            ("match", json_str(match_state)),
        ]),
        Q2CtfEvent::Menu { actor, title, entries } => obj(vec![
            ("kind", json_str("menu")),
            ("actor", wire_actor(actor)),
            ("title", json_str(title)),
            (
                "entries",
                arr(entries
                    .iter()
                    .map(|entry| {
                        obj(vec![
                            ("label", json_str(&entry.label)),
                            ("action", entry.action.map_or(SaveJson::Null, write_ctf_menu_action)),
                        ])
                    })
                    .collect()),
            ),
        ]),
        Q2CtfEvent::MatchStatus { text } => obj(vec![("kind", json_str("match-status")), ("text", json_str(text))]),
        Q2CtfEvent::AdminSettings { actor, settings } => obj(vec![
            ("kind", json_str("admin-settings")),
            ("actor", wire_actor(actor)),
            (
                "settings",
                obj(vec![
                    ("matchTime", json_str(&settings.match_time)),
                    ("matchSetup", json_str(&settings.match_setup)),
                    ("matchStartTime", json_str(&settings.match_start_time)),
                    ("captureLimit", json_str(&settings.capture_limit)),
                    ("use3dTarget", json_str(&settings.use_3d_target)),
                    ("allowGhost", json_str(&settings.allow_ghost)),
                    ("allowGrapple", json_str(&settings.allow_grapple)),
                    ("allowTech", json_str(&settings.allow_tech)),
                    ("useSpawnFarthest", json_str(&settings.use_spawn_farthest)),
                ]),
            ),
        ]),
        Q2CtfEvent::GrappleCable {
            actor,
            start,
            end,
            offset,
        } => obj(vec![
            ("kind", json_str("grapple-cable")),
            ("actor", wire_actor(actor)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
            ("offset", write_vector(*offset)),
        ]),
    }
}

/// LMCTF scoreboard row.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LmctfScoreRow {
    /// Actor handle.
    pub actor: ActorId,
    /// Slot.
    pub slot: f64,
    /// Name.
    pub name: String,
    /// Team.
    pub team: f64,
    /// Score.
    pub score: f64,
    /// Ping.
    pub ping: f64,
}

/// LMCTF rune.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2LmctfRune {
    /// Strength.
    Strength,
    /// Haste.
    Haste,
    /// Regeneration.
    Regeneration,
    /// Resistance.
    Resistance,
    /// Invisibility.
    Invisibility,
}

/// LMCTF event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2LmctfEvent {
    /// Grapple cable.
    GrappleCable {
        /// Actor handle.
        actor: ActorId,
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Offset.
        offset: Vec3,
    },
    /// Menu.
    Menu {
        /// Actor handle.
        actor: ActorId,
        /// Title.
        title: String,
        /// Entries.
        entries: Vec<Q2LmctfMenuEntry>,
    },
    /// Scoreboard.
    Scoreboard {
        /// Actor handle.
        actor: ActorId,
        /// Rows.
        rows: Vec<Q2LmctfScoreRow>,
        /// Layout.
        layout: Vec<String>,
    },
    /// HUD state.
    Hud {
        /// Actor handle.
        actor: ActorId,
        /// Team.
        team: f64,
        /// Carried flag.
        carried_flag: bool,
        /// Rune.
        rune: Option<Q2LmctfRune>,
        /// Layout.
        layout: Vec<String>,
    },
    /// Score log.
    ScoreLog {
        /// Actor handle.
        actor: ActorId,
        /// Victim actor.
        victim: Option<ActorId>,
        /// Name.
        name: String,
        /// Amount.
        amount: f64,
        /// Time in seconds.
        seconds: f64,
    },
}

/// LMCTF menu entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2LmctfMenuEntry {
    /// Label.
    pub label: String,
    /// Command.
    pub command: Option<String>,
}

fn read_lmctf_event(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q2LmctfEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "grapple-cable" => Ok(Q2LmctfEvent::GrappleCable {
            actor: read_actor(reader.field("actor"), identity)?,
            start: read_vector(reader.field("start"))?,
            end: read_vector(reader.field("end"))?,
            offset: read_vector(reader.field("offset"))?,
        }),
        "menu" => Ok(Q2LmctfEvent::Menu {
            actor: read_actor(reader.field("actor"), identity)?,
            title: reader.field("title").string()?,
            entries: bounded_list(reader.field("entries"), |entry| {
                Ok(Q2LmctfMenuEntry {
                    label: entry.field("label").string()?,
                    command: entry.field("command").nullable(|command| command.string())?,
                })
            })?,
        }),
        "scoreboard" => Ok(Q2LmctfEvent::Scoreboard {
            actor: read_actor(reader.field("actor"), identity)?,
            rows: bounded_list(reader.field("rows"), |row| {
                Ok(Q2LmctfScoreRow {
                    actor: read_actor(row.field("actor"), identity)?,
                    slot: row.field("slot").finite()?,
                    name: row.field("name").string()?,
                    team: row.field("team").finite()?,
                    score: row.field("score").finite()?,
                    ping: row.field("ping").finite()?,
                })
            })?,
            layout: bounded_list(reader.field("layout"), |item| item.string())?,
        }),
        "hud" => {
            let rune = reader.field("rune");
            Ok(Q2LmctfEvent::Hud {
                actor: read_actor(reader.field("actor"), identity)?,
                team: reader.field("team").finite()?,
                carried_flag: reader.field("carriedFlag").boolean()?,
                rune: if rune.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(
                        match rune
                            .choice_str(&["strength", "haste", "regeneration", "resistance", "invisibility"])?
                            .as_str()
                        {
                            "strength" => Q2LmctfRune::Strength,
                            "haste" => Q2LmctfRune::Haste,
                            "regeneration" => Q2LmctfRune::Regeneration,
                            "resistance" => Q2LmctfRune::Resistance,
                            _ => Q2LmctfRune::Invisibility,
                        },
                    )
                },
                layout: bounded_list(reader.field("layout"), |item| item.string())?,
            })
        }
        "score-log" => Ok(Q2LmctfEvent::ScoreLog {
            actor: read_actor(reader.field("actor"), identity)?,
            victim: reader.field("victim").nullable(|victim| read_actor(victim, identity))?,
            name: reader.field("name").string()?,
            amount: reader.field("amount").finite()?,
            seconds: reader.field("seconds").finite()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_lmctf_event(event: &Q2LmctfEvent) -> SaveJson {
    match event {
        Q2LmctfEvent::GrappleCable {
            actor,
            start,
            end,
            offset,
        } => obj(vec![
            ("kind", json_str("grapple-cable")),
            ("actor", wire_actor(actor)),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
            ("offset", write_vector(*offset)),
        ]),
        Q2LmctfEvent::Menu { actor, title, entries } => obj(vec![
            ("kind", json_str("menu")),
            ("actor", wire_actor(actor)),
            ("title", json_str(title)),
            (
                "entries",
                arr(entries
                    .iter()
                    .map(|entry| {
                        obj(vec![
                            ("label", json_str(&entry.label)),
                            (
                                "command",
                                entry
                                    .command
                                    .as_ref()
                                    .map_or(SaveJson::Null, |command| json_str(command)),
                            ),
                        ])
                    })
                    .collect()),
            ),
        ]),
        Q2LmctfEvent::Scoreboard { actor, rows, layout } => obj(vec![
            ("kind", json_str("scoreboard")),
            ("actor", wire_actor(actor)),
            (
                "rows",
                arr(rows
                    .iter()
                    .map(|row| {
                        obj(vec![
                            ("actor", wire_actor(&row.actor)),
                            ("slot", num(row.slot)),
                            ("name", json_str(&row.name)),
                            ("team", num(row.team)),
                            ("score", num(row.score)),
                            ("ping", num(row.ping)),
                        ])
                    })
                    .collect()),
            ),
            ("layout", arr(layout.iter().map(|line| json_str(line)).collect())),
        ]),
        Q2LmctfEvent::Hud {
            actor,
            team,
            carried_flag,
            rune,
            layout,
        } => obj(vec![
            ("kind", json_str("hud")),
            ("actor", wire_actor(actor)),
            ("team", num(*team)),
            ("carriedFlag", boolean(*carried_flag)),
            (
                "rune",
                rune.map_or(SaveJson::Null, |rune| {
                    json_str(match rune {
                        Q2LmctfRune::Strength => "strength",
                        Q2LmctfRune::Haste => "haste",
                        Q2LmctfRune::Regeneration => "regeneration",
                        Q2LmctfRune::Resistance => "resistance",
                        Q2LmctfRune::Invisibility => "invisibility",
                    })
                }),
            ),
            ("layout", arr(layout.iter().map(|line| json_str(line)).collect())),
        ]),
        Q2LmctfEvent::ScoreLog {
            actor,
            victim,
            name,
            amount,
            seconds,
        } => obj(vec![
            ("kind", json_str("score-log")),
            ("actor", wire_actor(actor)),
            ("victim", victim.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("name", json_str(name)),
            ("amount", num(*amount)),
            ("seconds", num(*seconds)),
        ]),
    }
}

/// Mission-pack player effect.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2MissionPackPlayerEffect {
    /// Tracker pain.
    TrackerPain {
        /// Actor handle.
        actor: ActorId,
        /// Effect time.
        until: f64,
    },
    /// Nuke blindness.
    NukeBlind {
        /// Actor handle.
        actor: ActorId,
        /// Effect time.
        until: f64,
    },
    /// IR vision.
    Ir {
        /// Actor handle.
        actor: ActorId,
        /// Effect time.
        until: f64,
    },
    /// Sphere camera.
    SphereCamera {
        /// Actor handle.
        actor: ActorId,
        /// Sphere actor.
        sphere: Option<ActorId>,
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
    },
}

fn read_mission_pack_player_effect(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2MissionPackPlayerEffect, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "tracker-pain" => Ok(Q2MissionPackPlayerEffect::TrackerPain {
            actor: read_actor(reader.field("actor"), identity)?,
            until: reader.field("until").finite()?,
        }),
        "nuke-blind" => Ok(Q2MissionPackPlayerEffect::NukeBlind {
            actor: read_actor(reader.field("actor"), identity)?,
            until: reader.field("until").finite()?,
        }),
        "ir" => Ok(Q2MissionPackPlayerEffect::Ir {
            actor: read_actor(reader.field("actor"), identity)?,
            until: reader.field("until").finite()?,
        }),
        "sphere-camera" => Ok(Q2MissionPackPlayerEffect::SphereCamera {
            actor: read_actor(reader.field("actor"), identity)?,
            sphere: reader.field("sphere").nullable(|sphere| read_actor(sphere, identity))?,
            origin: read_vector(reader.field("origin"))?,
            angles: read_vector(reader.field("angles"))?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_mission_pack_player_effect(event: &Q2MissionPackPlayerEffect) -> SaveJson {
    match event {
        Q2MissionPackPlayerEffect::TrackerPain { actor, until } => obj(vec![
            ("kind", json_str("tracker-pain")),
            ("actor", wire_actor(actor)),
            ("until", num(*until)),
        ]),
        Q2MissionPackPlayerEffect::NukeBlind { actor, until } => obj(vec![
            ("kind", json_str("nuke-blind")),
            ("actor", wire_actor(actor)),
            ("until", num(*until)),
        ]),
        Q2MissionPackPlayerEffect::Ir { actor, until } => obj(vec![
            ("kind", json_str("ir")),
            ("actor", wire_actor(actor)),
            ("until", num(*until)),
        ]),
        Q2MissionPackPlayerEffect::SphereCamera {
            actor,
            sphere,
            origin,
            angles,
        } => obj(vec![
            ("kind", json_str("sphere-camera")),
            ("actor", wire_actor(actor)),
            ("sphere", sphere.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("origin", write_vector(*origin)),
            ("angles", write_vector(*angles)),
        ]),
    }
}

/// Mission-pack entity event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2MissionPackEntityEvent {
    /// Steam jet.
    Steam {
        /// Effect id.
        id: f64,
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Count.
        count: f64,
        /// Color.
        color: f64,
        /// Speed.
        speed: f64,
        /// Duration in milliseconds.
        milliseconds: f64,
    },
    /// Force wall.
    ForceWall {
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Color.
        color: f64,
    },
}

fn read_mission_pack_entity_event(
    reader: SaveReader,
    _identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2MissionPackEntityEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "steam" => Ok(Q2MissionPackEntityEvent::Steam {
            id: reader.field("id").finite()?,
            origin: read_vector(reader.field("origin"))?,
            direction: read_vector(reader.field("direction"))?,
            count: reader.field("count").finite()?,
            color: reader.field("color").finite()?,
            speed: reader.field("speed").finite()?,
            milliseconds: reader.field("milliseconds").finite()?,
        }),
        "force-wall" => Ok(Q2MissionPackEntityEvent::ForceWall {
            start: read_vector(reader.field("start"))?,
            end: read_vector(reader.field("end"))?,
            color: reader.field("color").finite()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_mission_pack_entity_event(event: &Q2MissionPackEntityEvent) -> SaveJson {
    match event {
        Q2MissionPackEntityEvent::Steam {
            id,
            origin,
            direction,
            count,
            color,
            speed,
            milliseconds,
        } => obj(vec![
            ("kind", json_str("steam")),
            ("id", num(*id)),
            ("origin", write_vector(*origin)),
            ("direction", write_vector(*direction)),
            ("count", num(*count)),
            ("color", num(*color)),
            ("speed", num(*speed)),
            ("milliseconds", num(*milliseconds)),
        ]),
        Q2MissionPackEntityEvent::ForceWall { start, end, color } => obj(vec![
            ("kind", json_str("force-wall")),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
            ("color", num(*color)),
        ]),
    }
}

/// Q2 composition event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2CompositionEvent {
    /// CTF event.
    Ctf {
        /// Event.
        event: Q2CtfEvent,
    },
    /// LMCTF event.
    Lmctf {
        /// Event.
        event: Q2LmctfEvent,
    },
    /// Grapple prediction.
    GrapplePrediction {
        /// Actor handle.
        actor: ActorId,
        /// Suppressed flag.
        suppressed: bool,
    },
    /// Kick.
    Kick {
        /// Actor handle.
        actor: ActorId,
    },
    /// Mission-pack player effect.
    MissionpackPlayer {
        /// Event.
        event: Q2MissionPackPlayerEffect,
    },
    /// Mission-pack entity event.
    MissionpackEntity {
        /// Event.
        event: Q2MissionPackEntityEvent,
    },
}

fn read_q2_composition_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2CompositionEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "ctf" => Ok(Q2CompositionEvent::Ctf {
            event: read_ctf_event(reader.field("event"), identity)?,
        }),
        "lmctf" => Ok(Q2CompositionEvent::Lmctf {
            event: read_lmctf_event(reader.field("event"), identity)?,
        }),
        "grapple-prediction" => Ok(Q2CompositionEvent::GrapplePrediction {
            actor: read_actor(reader.field("actor"), identity)?,
            suppressed: reader.field("suppressed").boolean()?,
        }),
        "kick" => Ok(Q2CompositionEvent::Kick {
            actor: read_actor(reader.field("actor"), identity)?,
        }),
        "missionpack-player" => Ok(Q2CompositionEvent::MissionpackPlayer {
            event: read_mission_pack_player_effect(reader.field("event"), identity)?,
        }),
        "missionpack-entity" => Ok(Q2CompositionEvent::MissionpackEntity {
            event: read_mission_pack_entity_event(reader.field("event"), identity)?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q2_composition_event(event: &Q2CompositionEvent) -> SaveJson {
    match event {
        Q2CompositionEvent::Ctf { event } => obj(vec![("kind", json_str("ctf")), ("event", write_ctf_event(event))]),
        Q2CompositionEvent::Lmctf { event } => {
            obj(vec![("kind", json_str("lmctf")), ("event", write_lmctf_event(event))])
        }
        Q2CompositionEvent::GrapplePrediction { actor, suppressed } => obj(vec![
            ("kind", json_str("grapple-prediction")),
            ("actor", wire_actor(actor)),
            ("suppressed", boolean(*suppressed)),
        ]),
        Q2CompositionEvent::Kick { actor } => obj(vec![("kind", json_str("kick")), ("actor", wire_actor(actor))]),
        Q2CompositionEvent::MissionpackPlayer { event } => obj(vec![
            ("kind", json_str("missionpack-player")),
            ("event", write_mission_pack_player_effect(event)),
        ]),
        Q2CompositionEvent::MissionpackEntity { event } => obj(vec![
            ("kind", json_str("missionpack-entity")),
            ("event", write_mission_pack_entity_event(event)),
        ]),
    }
}

/// Rerelease debug line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2DebugLine {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Color.
    pub color: Vec4,
    /// Depth-test flag.
    pub depth_test: bool,
}

fn read_debug_line(reader: SaveReader) -> Result<Q2DebugLine, WorldError> {
    Ok(Q2DebugLine {
        start: read_vector(reader.field("start"))?,
        end: read_vector(reader.field("end"))?,
        color: read_color(reader.field("color"))?,
        depth_test: reader.field("depthTest").boolean()?,
    })
}

fn write_debug_line(line: Q2DebugLine) -> SaveJson {
    obj(vec![
        ("start", write_vector(line.start)),
        ("end", write_vector(line.end)),
        ("color", write_color(line.color)),
        ("depthTest", boolean(line.depth_test)),
    ])
}

/// Rerelease world-text orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q2TextOrientation {
    /// Billboard.
    Billboard,
    /// Fixed angles.
    Fixed {
        /// Angles.
        angles: Vec3,
    },
}

/// Rerelease world-text font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2TextFont {
    /// Classic font.
    Classic,
    /// Selected font.
    Selected,
}

/// Rerelease world text.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2WorldText {
    /// Text.
    pub text: String,
    /// Origin.
    pub origin: Vec3,
    /// Color.
    pub color: Vec4,
    /// Cell size.
    pub cell_size: f64,
    /// Distance cull factor.
    pub distance_cull_factor: Option<f64>,
    /// Orientation.
    pub orientation: Q2TextOrientation,
    /// Depth-test flag.
    pub depth_test: bool,
    /// Font.
    pub font: Q2TextFont,
}

fn read_world_text_value(reader: SaveReader) -> Result<Q2WorldText, WorldError> {
    let orientation = reader.field("orientation");
    let kind = orientation.field("kind").choice_str(&["billboard", "fixed"])?;
    let font = reader.field("font").choice_str(&["classic", "selected"])?;
    let distance = reader.field("distanceCullFactor");
    Ok(Q2WorldText {
        text: reader.field("text").string()?,
        origin: read_vector(reader.field("origin"))?,
        color: read_color(reader.field("color"))?,
        cell_size: reader.field("cellSize").finite()?,
        distance_cull_factor: if distance.value.is_none() {
            None
        } else {
            Some(distance.finite()?)
        },
        orientation: if kind == "billboard" {
            Q2TextOrientation::Billboard
        } else {
            Q2TextOrientation::Fixed {
                angles: read_vector(orientation.field("angles"))?,
            }
        },
        depth_test: reader.field("depthTest").boolean()?,
        font: if font == "classic" {
            Q2TextFont::Classic
        } else {
            Q2TextFont::Selected
        },
    })
}

fn write_world_text_value(text: &Q2WorldText) -> SaveJson {
    let mut members = vec![
        ("text", json_str(&text.text)),
        ("origin", write_vector(text.origin)),
        ("color", write_color(text.color)),
        ("cellSize", num(text.cell_size)),
    ];
    if let Some(factor) = text.distance_cull_factor {
        members.push(("distanceCullFactor", num(factor)));
    }
    members.push((
        "orientation",
        match text.orientation {
            Q2TextOrientation::Billboard => obj(vec![("kind", json_str("billboard"))]),
            Q2TextOrientation::Fixed { angles } => {
                obj(vec![("kind", json_str("fixed")), ("angles", write_vector(angles))])
            }
        },
    ));
    members.push(("depthTest", boolean(text.depth_test)));
    members.push((
        "font",
        json_str(match text.font {
            Q2TextFont::Classic => "classic",
            Q2TextFont::Selected => "selected",
        }),
    ));
    obj(members)
}

/// Rerelease fog value.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FogValue {
    /// Density.
    pub density: f64,
    /// Color.
    pub color: Vec3,
    /// Sky factor.
    pub sky_factor: f64,
}

fn read_fog_value(reader: SaveReader) -> Result<Q2FogValue, WorldError> {
    Ok(Q2FogValue {
        density: reader.field("density").finite()?,
        color: read_vector(reader.field("color"))?,
        sky_factor: reader.field("skyFactor").finite()?,
    })
}

fn write_fog_value(value: &Q2FogValue) -> SaveJson {
    obj(vec![
        ("density", num(value.density)),
        ("color", write_vector(value.color)),
        ("skyFactor", num(value.sky_factor)),
    ])
}

/// Rerelease height-fog value.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2HeightFogValue {
    /// Start color.
    pub start_color: Vec3,
    /// Start distance.
    pub start_distance: f64,
    /// End color.
    pub end_color: Vec3,
    /// End distance.
    pub end_distance: f64,
    /// Falloff.
    pub falloff: f64,
    /// Density.
    pub density: f64,
}

fn read_height_fog_value(reader: SaveReader) -> Result<Q2HeightFogValue, WorldError> {
    Ok(Q2HeightFogValue {
        start_color: read_vector(reader.field("startColor"))?,
        start_distance: reader.field("startDistance").finite()?,
        end_color: read_vector(reader.field("endColor"))?,
        end_distance: reader.field("endDistance").finite()?,
        falloff: reader.field("falloff").finite()?,
        density: reader.field("density").finite()?,
    })
}

fn write_height_fog_value(value: &Q2HeightFogValue) -> SaveJson {
    obj(vec![
        ("startColor", write_vector(value.start_color)),
        ("startDistance", num(value.start_distance)),
        ("endColor", write_vector(value.end_color)),
        ("endDistance", num(value.end_distance)),
        ("falloff", num(value.falloff)),
        ("density", num(value.density)),
    ])
}

/// Rerelease fog state.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2FogState {
    /// Fog value.
    pub fog: Q2FogValue,
    /// Height fog.
    pub height_fog: Q2HeightFogValue,
}

fn read_fog_state(reader: SaveReader) -> Result<Q2FogState, WorldError> {
    Ok(Q2FogState {
        fog: read_fog_value(reader.field("fog"))?,
        height_fog: read_height_fog_value(reader.field("heightFog"))?,
    })
}

fn write_fog_state(state: &Q2FogState) -> SaveJson {
    obj(vec![
        ("fog", write_fog_value(&state.fog)),
        ("heightFog", write_height_fog_value(&state.height_fog)),
    ])
}

/// Flashlight hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2FlashlightHand {
    /// Right hand.
    Right,
    /// Left hand.
    Left,
    /// Center.
    Center,
}

/// Coop respawn state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2CoopRespawnState {
    /// In progress.
    InProgress,
    /// Countdown.
    Countdown,
    /// You.
    You,
    /// Friend.
    Friend,
    /// All.
    All,
    /// None.
    None,
}

/// Unit level entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2LevelEntry {
    /// Map name.
    pub map: String,
    /// Display name.
    pub name: String,
    /// Visit order.
    pub visit_order: f64,
    /// Total secrets.
    pub total_secrets: f64,
    /// Found secrets.
    pub found_secrets: f64,
    /// Total monsters.
    pub total_monsters: f64,
    /// Killed monsters.
    pub killed_monsters: f64,
    /// Completion time.
    pub time: f64,
}

fn read_level_entry(reader: SaveReader) -> Result<Q2LevelEntry, WorldError> {
    Ok(Q2LevelEntry {
        map: reader.field("map").string()?,
        name: reader.field("name").string()?,
        visit_order: reader.field("visitOrder").finite()?,
        total_secrets: reader.field("totalSecrets").finite()?,
        found_secrets: reader.field("foundSecrets").finite()?,
        total_monsters: reader.field("totalMonsters").finite()?,
        killed_monsters: reader.field("killedMonsters").finite()?,
        time: reader.field("time").finite()?,
    })
}

fn write_level_entry(entry: &Q2LevelEntry) -> SaveJson {
    obj(vec![
        ("map", json_str(&entry.map)),
        ("name", json_str(&entry.name)),
        ("visitOrder", num(entry.visit_order)),
        ("totalSecrets", num(entry.total_secrets)),
        ("foundSecrets", num(entry.found_secrets)),
        ("totalMonsters", num(entry.total_monsters)),
        ("killedMonsters", num(entry.killed_monsters)),
        ("time", num(entry.time)),
    ])
}

/// Q2 rerelease event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2RereleaseEvent {
    /// Debug shapes.
    DebugShapes {
        /// Lines.
        lines: Vec<Q2DebugLine>,
        /// Lifetime in milliseconds.
        lifetime_milliseconds: f64,
    },
    /// World text.
    WorldText {
        /// Text.
        text: Q2WorldText,
        /// Lifetime.
        lifetime: f64,
    },
    /// Localized print.
    LocalizedPrint {
        /// Actor handle.
        actor: Option<ActorId>,
        /// Level.
        level: Q2PrintLevel,
        /// Text.
        text: String,
        /// Arguments.
        args: Vec<String>,
    },
    /// Mission objective.
    MissionObjective {
        /// Actor handle.
        actor: ActorId,
        /// Text.
        text: String,
        /// Arguments.
        args: Vec<String>,
        /// Talk sound.
        talk_sound: String,
    },
    /// Mission status.
    MissionStatus {
        /// Actor handle.
        actor: ActorId,
        /// Icon visible.
        icon_visible: bool,
    },
    /// Screen blend.
    ScreenBlend {
        /// Actor handle.
        actor: ActorId,
        /// Blend color.
        blend: Vec4,
    },
    /// Help computer.
    HelpComputer {
        /// Actor handle.
        actor: ActorId,
        /// Visible flag.
        visible: bool,
        /// Primary text.
        primary: String,
        /// Secondary text.
        secondary: String,
        /// Slow time.
        slow_time: f64,
    },
    /// Fog state.
    Fog {
        /// Actor handle.
        actor: ActorId,
        /// Fog state.
        value: Q2FogState,
        /// Transition in milliseconds.
        transition_milliseconds: f64,
    },
    /// Flashlight.
    Flashlight {
        /// Actor handle.
        actor: ActorId,
        /// Enabled flag.
        enabled: bool,
        /// Hand.
        hand: Q2FlashlightHand,
    },
    /// Point of interest.
    Poi {
        /// Actor handle.
        actor: ActorId,
        /// Position.
        position: Vec3,
        /// Image.
        image: String,
        /// Duration.
        duration: f64,
        /// Color.
        color: Vec4,
    },
    /// Remove POI.
    RemovePoi {
        /// Actor handle.
        actor: ActorId,
        /// Key.
        key: String,
    },
    /// Keyed POI.
    KeyedPoi {
        /// Actor handle.
        actor: ActorId,
        /// Key.
        key: String,
        /// Position.
        position: Vec3,
        /// Image.
        image: String,
        /// Duration.
        duration: f64,
        /// Color.
        color: Vec4,
        /// Flags.
        flags: f64,
    },
    /// Directional damage.
    DirectionalDamage {
        /// Actor handle.
        actor: ActorId,
        /// Direction.
        direction: Vec3,
        /// Damage.
        damage: f64,
        /// Health.
        health: f64,
        /// Armor.
        armor: f64,
        /// Shield.
        shield: f64,
    },
    /// Help path.
    HelpPath {
        /// Actor handle.
        actor: ActorId,
        /// First node.
        first: Vec3,
        /// Position.
        position: Vec3,
        /// Direction.
        direction: Vec3,
    },
    /// Coop respawn.
    CoopRespawn {
        /// Actor handle.
        actor: ActorId,
        /// State.
        state: Q2CoopRespawnState,
        /// Lives.
        lives: f64,
    },
    /// Autosave.
    Autosave,
    /// Entity alpha.
    Alpha {
        /// Actor handle.
        actor: ActorId,
        /// Alpha.
        alpha: f64,
    },
    /// End of unit.
    EndOfUnit {
        /// Levels.
        levels: Vec<Q2LevelEntry>,
        /// Button time.
        button_time: f64,
    },
    /// Player dogtag.
    PlayerDogtag {
        /// Actor handle.
        actor: ActorId,
        /// Value.
        value: f64,
    },
    /// Dynamic light.
    DynamicLight {
        /// Actor handle.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Radius.
        radius: f64,
        /// Color.
        color: Vec3,
        /// Visible flag.
        visible: bool,
    },
    /// Restart level.
    RestartLevel {
        /// Map name.
        map: String,
    },
    /// Story text.
    Story {
        /// Text.
        text: String,
    },
    /// Achievement.
    Achievement {
        /// Id.
        id: String,
    },
    /// Sky.
    Sky {
        /// Name.
        name: String,
        /// Rotation.
        rotation: f64,
        /// Auto-rotate flag.
        auto_rotate: bool,
        /// Axis.
        axis: Vec3,
    },
    /// Health bar.
    Healthbar {
        /// Actor handle.
        actor: ActorId,
        /// Slot.
        slot: f64,
        /// Target actor.
        target: ActorId,
        /// Name.
        name: String,
        /// Fraction.
        fraction: f64,
        /// Visible flag.
        visible: bool,
    },
    /// Item visibility.
    ItemVisibility {
        /// Actor handle.
        actor: ActorId,
        /// Item id.
        item: String,
        /// Visible flag.
        visible: bool,
    },
}

fn read_q2_rerelease_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2RereleaseEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "debug-shapes" => Ok(Q2RereleaseEvent::DebugShapes {
            lines: bounded_list(reader.field("lines"), read_debug_line)?,
            lifetime_milliseconds: reader.field("lifetimeMilliseconds").finite()?,
        }),
        "world-text" => Ok(Q2RereleaseEvent::WorldText {
            text: read_world_text_value(reader.field("text"))?,
            lifetime: reader.field("lifetime").finite()?,
        }),
        "localized-print" => Ok(Q2RereleaseEvent::LocalizedPrint {
            actor: read_opt_actor(reader.field("actor"), identity)?,
            level: read_print_level(reader.field("level"))?,
            text: reader.field("text").string()?,
            args: bounded_list(reader.field("args"), |item| item.string())?,
        }),
        "mission-objective" => Ok(Q2RereleaseEvent::MissionObjective {
            actor: read_actor(reader.field("actor"), identity)?,
            text: reader.field("text").string()?,
            args: bounded_list(reader.field("args"), |item| item.string())?,
            talk_sound: reader.field("talkSound").string()?,
        }),
        "mission-status" => Ok(Q2RereleaseEvent::MissionStatus {
            actor: read_actor(reader.field("actor"), identity)?,
            icon_visible: reader.field("iconVisible").boolean()?,
        }),
        "screen-blend" => Ok(Q2RereleaseEvent::ScreenBlend {
            actor: read_actor(reader.field("actor"), identity)?,
            blend: read_color(reader.field("blend"))?,
        }),
        "help-computer" => Ok(Q2RereleaseEvent::HelpComputer {
            actor: read_actor(reader.field("actor"), identity)?,
            visible: reader.field("visible").boolean()?,
            primary: reader.field("primary").string()?,
            secondary: reader.field("secondary").string()?,
            slow_time: reader.field("slowTime").finite()?,
        }),
        "fog" => Ok(Q2RereleaseEvent::Fog {
            actor: read_actor(reader.field("actor"), identity)?,
            value: read_fog_state(reader.field("value"))?,
            transition_milliseconds: reader.field("transitionMilliseconds").finite()?,
        }),
        "flashlight" => {
            let hand = reader.field("hand").choice_str(&["right", "left", "center"])?;
            Ok(Q2RereleaseEvent::Flashlight {
                actor: read_actor(reader.field("actor"), identity)?,
                enabled: reader.field("enabled").boolean()?,
                hand: match hand.as_str() {
                    "right" => Q2FlashlightHand::Right,
                    "left" => Q2FlashlightHand::Left,
                    _ => Q2FlashlightHand::Center,
                },
            })
        }
        "poi" => Ok(Q2RereleaseEvent::Poi {
            actor: read_actor(reader.field("actor"), identity)?,
            position: read_vector(reader.field("position"))?,
            image: reader.field("image").string()?,
            duration: reader.field("duration").finite()?,
            color: read_color(reader.field("color"))?,
        }),
        "remove-poi" => Ok(Q2RereleaseEvent::RemovePoi {
            actor: read_actor(reader.field("actor"), identity)?,
            key: reader.field("key").string()?,
        }),
        "keyed-poi" => Ok(Q2RereleaseEvent::KeyedPoi {
            actor: read_actor(reader.field("actor"), identity)?,
            key: reader.field("key").string()?,
            position: read_vector(reader.field("position"))?,
            image: reader.field("image").string()?,
            duration: reader.field("duration").finite()?,
            color: read_color(reader.field("color"))?,
            flags: reader.field("flags").finite()?,
        }),
        "directional-damage" => Ok(Q2RereleaseEvent::DirectionalDamage {
            actor: read_actor(reader.field("actor"), identity)?,
            direction: read_vector(reader.field("direction"))?,
            damage: reader.field("damage").finite()?,
            health: reader.field("health").finite()?,
            armor: reader.field("armor").finite()?,
            shield: reader.field("shield").finite()?,
        }),
        "help-path" => Ok(Q2RereleaseEvent::HelpPath {
            actor: read_actor(reader.field("actor"), identity)?,
            first: read_vector(reader.field("first"))?,
            position: read_vector(reader.field("position"))?,
            direction: read_vector(reader.field("direction"))?,
        }),
        "coop-respawn" => {
            let state =
                reader
                    .field("state")
                    .choice_str(&["in-progress", "countdown", "you", "friend", "all", "none"])?;
            Ok(Q2RereleaseEvent::CoopRespawn {
                actor: read_actor(reader.field("actor"), identity)?,
                state: match state.as_str() {
                    "in-progress" => Q2CoopRespawnState::InProgress,
                    "countdown" => Q2CoopRespawnState::Countdown,
                    "you" => Q2CoopRespawnState::You,
                    "friend" => Q2CoopRespawnState::Friend,
                    "all" => Q2CoopRespawnState::All,
                    _ => Q2CoopRespawnState::None,
                },
                lives: reader.field("lives").finite()?,
            })
        }
        "autosave" => Ok(Q2RereleaseEvent::Autosave),
        "alpha" => {
            reader.field("kind").literal_str("alpha")?;
            Ok(Q2RereleaseEvent::Alpha {
                actor: read_actor(reader.field("actor"), identity)?,
                alpha: reader.field("alpha").finite()?,
            })
        }
        "end-of-unit" => Ok(Q2RereleaseEvent::EndOfUnit {
            levels: bounded_list(reader.field("levels"), read_level_entry)?,
            button_time: reader.field("buttonTime").finite()?,
        }),
        "player-dogtag" => Ok(Q2RereleaseEvent::PlayerDogtag {
            actor: read_actor(reader.field("actor"), identity)?,
            value: reader.field("value").finite()?,
        }),
        "dynamic-light" => Ok(Q2RereleaseEvent::DynamicLight {
            actor: read_actor(reader.field("actor"), identity)?,
            origin: read_vector(reader.field("origin"))?,
            radius: reader.field("radius").finite()?,
            color: read_vector(reader.field("color"))?,
            visible: reader.field("visible").boolean()?,
        }),
        "restart-level" => Ok(Q2RereleaseEvent::RestartLevel {
            map: reader.field("map").string()?,
        }),
        "story" => Ok(Q2RereleaseEvent::Story {
            text: reader.field("text").string()?,
        }),
        "achievement" => Ok(Q2RereleaseEvent::Achievement {
            id: reader.field("id").string()?,
        }),
        "sky" => Ok(Q2RereleaseEvent::Sky {
            name: reader.field("name").string()?,
            rotation: reader.field("rotation").finite()?,
            auto_rotate: reader.field("autoRotate").boolean()?,
            axis: read_vector(reader.field("axis"))?,
        }),
        "healthbar" => Ok(Q2RereleaseEvent::Healthbar {
            actor: read_actor(reader.field("actor"), identity)?,
            slot: reader.field("slot").finite()?,
            target: read_actor(reader.field("target"), identity)?,
            name: reader.field("name").string()?,
            fraction: reader.field("fraction").finite()?,
            visible: reader.field("visible").boolean()?,
        }),
        "item-visibility" => Ok(Q2RereleaseEvent::ItemVisibility {
            actor: read_actor(reader.field("actor"), identity)?,
            item: namespaced(reader.field("item"))?,
            visible: reader.field("visible").boolean()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q2_rerelease_event(event: &Q2RereleaseEvent) -> SaveJson {
    match event {
        Q2RereleaseEvent::DebugShapes {
            lines,
            lifetime_milliseconds,
        } => obj(vec![
            ("kind", json_str("debug-shapes")),
            ("lines", arr(lines.iter().copied().map(write_debug_line).collect())),
            ("lifetimeMilliseconds", num(*lifetime_milliseconds)),
        ]),
        Q2RereleaseEvent::WorldText { text, lifetime } => obj(vec![
            ("kind", json_str("world-text")),
            ("text", write_world_text_value(text)),
            ("lifetime", num(*lifetime)),
        ]),
        Q2RereleaseEvent::LocalizedPrint {
            actor,
            level,
            text,
            args,
        } => obj(vec![
            ("kind", json_str("localized-print")),
            ("actor", write_opt_actor(actor.as_ref())),
            ("level", write_print_level(*level)),
            ("text", json_str(text)),
            ("args", arr(args.iter().map(|arg| json_str(arg)).collect())),
        ]),
        Q2RereleaseEvent::MissionObjective {
            actor,
            text,
            args,
            talk_sound,
        } => obj(vec![
            ("kind", json_str("mission-objective")),
            ("actor", wire_actor(actor)),
            ("text", json_str(text)),
            ("args", arr(args.iter().map(|arg| json_str(arg)).collect())),
            ("talkSound", json_str(talk_sound)),
        ]),
        Q2RereleaseEvent::MissionStatus { actor, icon_visible } => obj(vec![
            ("kind", json_str("mission-status")),
            ("actor", wire_actor(actor)),
            ("iconVisible", boolean(*icon_visible)),
        ]),
        Q2RereleaseEvent::ScreenBlend { actor, blend } => obj(vec![
            ("kind", json_str("screen-blend")),
            ("actor", wire_actor(actor)),
            ("blend", write_color(*blend)),
        ]),
        Q2RereleaseEvent::HelpComputer {
            actor,
            visible,
            primary,
            secondary,
            slow_time,
        } => obj(vec![
            ("kind", json_str("help-computer")),
            ("actor", wire_actor(actor)),
            ("visible", boolean(*visible)),
            ("primary", json_str(primary)),
            ("secondary", json_str(secondary)),
            ("slowTime", num(*slow_time)),
        ]),
        Q2RereleaseEvent::Fog {
            actor,
            value,
            transition_milliseconds,
        } => obj(vec![
            ("kind", json_str("fog")),
            ("actor", wire_actor(actor)),
            ("value", write_fog_state(value)),
            ("transitionMilliseconds", num(*transition_milliseconds)),
        ]),
        Q2RereleaseEvent::Flashlight { actor, enabled, hand } => obj(vec![
            ("kind", json_str("flashlight")),
            ("actor", wire_actor(actor)),
            ("enabled", boolean(*enabled)),
            (
                "hand",
                json_str(match hand {
                    Q2FlashlightHand::Right => "right",
                    Q2FlashlightHand::Left => "left",
                    Q2FlashlightHand::Center => "center",
                }),
            ),
        ]),
        Q2RereleaseEvent::Poi {
            actor,
            position,
            image,
            duration,
            color,
        } => obj(vec![
            ("kind", json_str("poi")),
            ("actor", wire_actor(actor)),
            ("position", write_vector(*position)),
            ("image", json_str(image)),
            ("duration", num(*duration)),
            ("color", write_color(*color)),
        ]),
        Q2RereleaseEvent::RemovePoi { actor, key } => obj(vec![
            ("kind", json_str("remove-poi")),
            ("actor", wire_actor(actor)),
            ("key", json_str(key)),
        ]),
        Q2RereleaseEvent::KeyedPoi {
            actor,
            key,
            position,
            image,
            duration,
            color,
            flags,
        } => obj(vec![
            ("kind", json_str("keyed-poi")),
            ("actor", wire_actor(actor)),
            ("key", json_str(key)),
            ("position", write_vector(*position)),
            ("image", json_str(image)),
            ("duration", num(*duration)),
            ("color", write_color(*color)),
            ("flags", num(*flags)),
        ]),
        Q2RereleaseEvent::DirectionalDamage {
            actor,
            direction,
            damage,
            health,
            armor,
            shield,
        } => obj(vec![
            ("kind", json_str("directional-damage")),
            ("actor", wire_actor(actor)),
            ("direction", write_vector(*direction)),
            ("damage", num(*damage)),
            ("health", num(*health)),
            ("armor", num(*armor)),
            ("shield", num(*shield)),
        ]),
        Q2RereleaseEvent::HelpPath {
            actor,
            first,
            position,
            direction,
        } => obj(vec![
            ("kind", json_str("help-path")),
            ("actor", wire_actor(actor)),
            ("first", write_vector(*first)),
            ("position", write_vector(*position)),
            ("direction", write_vector(*direction)),
        ]),
        Q2RereleaseEvent::CoopRespawn { actor, state, lives } => obj(vec![
            ("kind", json_str("coop-respawn")),
            ("actor", wire_actor(actor)),
            (
                "state",
                json_str(match state {
                    Q2CoopRespawnState::InProgress => "in-progress",
                    Q2CoopRespawnState::Countdown => "countdown",
                    Q2CoopRespawnState::You => "you",
                    Q2CoopRespawnState::Friend => "friend",
                    Q2CoopRespawnState::All => "all",
                    Q2CoopRespawnState::None => "none",
                }),
            ),
            ("lives", num(*lives)),
        ]),
        Q2RereleaseEvent::Autosave => obj(vec![("kind", json_str("autosave"))]),
        Q2RereleaseEvent::Alpha { actor, alpha } => write_addon_alpha(actor, *alpha),
        Q2RereleaseEvent::EndOfUnit { levels, button_time } => obj(vec![
            ("kind", json_str("end-of-unit")),
            ("levels", arr(levels.iter().map(write_level_entry).collect())),
            ("buttonTime", num(*button_time)),
        ]),
        Q2RereleaseEvent::PlayerDogtag { actor, value } => obj(vec![
            ("kind", json_str("player-dogtag")),
            ("actor", wire_actor(actor)),
            ("value", num(*value)),
        ]),
        Q2RereleaseEvent::DynamicLight {
            actor,
            origin,
            radius,
            color,
            visible,
        } => obj(vec![
            ("kind", json_str("dynamic-light")),
            ("actor", wire_actor(actor)),
            ("origin", write_vector(*origin)),
            ("radius", num(*radius)),
            ("color", write_vector(*color)),
            ("visible", boolean(*visible)),
        ]),
        Q2RereleaseEvent::RestartLevel { map } => {
            obj(vec![("kind", json_str("restart-level")), ("map", json_str(map))])
        }
        Q2RereleaseEvent::Story { text } => obj(vec![("kind", json_str("story")), ("text", json_str(text))]),
        Q2RereleaseEvent::Achievement { id } => obj(vec![("kind", json_str("achievement")), ("id", json_str(id))]),
        Q2RereleaseEvent::Sky {
            name,
            rotation,
            auto_rotate,
            axis,
        } => obj(vec![
            ("kind", json_str("sky")),
            ("name", json_str(name)),
            ("rotation", num(*rotation)),
            ("autoRotate", boolean(*auto_rotate)),
            ("axis", write_vector(*axis)),
        ]),
        Q2RereleaseEvent::Healthbar {
            actor,
            slot,
            target,
            name,
            fraction,
            visible,
        } => obj(vec![
            ("kind", json_str("healthbar")),
            ("actor", wire_actor(actor)),
            ("slot", num(*slot)),
            ("target", wire_actor(target)),
            ("name", json_str(name)),
            ("fraction", num(*fraction)),
            ("visible", boolean(*visible)),
        ]),
        Q2RereleaseEvent::ItemVisibility { actor, item, visible } => obj(vec![
            ("kind", json_str("item-visibility")),
            ("actor", wire_actor(actor)),
            ("item", json_str(item)),
            ("visible", boolean(*visible)),
        ]),
    }
}

/// Q2 player view.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerView {
    /// View angles.
    pub angles: Vec3,
    /// View offset.
    pub offset: Vec3,
    /// Kick angles.
    pub kick_angles: Vec3,
    /// Gun angles.
    pub gun_angles: Vec3,
    /// Gun offset.
    pub gun_offset: Vec3,
    /// Blend color.
    pub blend: Vec4,
    /// Field of view.
    pub fov: f64,
    /// Underwater flag.
    pub underwater: bool,
    /// Muzzle flashes.
    pub flashes: Vec3,
    /// Health.
    pub health: f64,
    /// Armor.
    pub armor: f64,
    /// Ammo.
    pub ammo: f64,
    /// Score.
    pub score: f64,
    /// Selected item.
    pub selected_item: Option<String>,
    /// Timer.
    pub timer: Option<Q2ViewTimerFull>,
    /// Spectator flag.
    pub spectator: bool,
    /// Layouts.
    pub layouts: Vec<String>,
}

/// Q2 player view timer.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ViewTimerFull {
    /// Item id.
    pub item: String,
    /// Seconds.
    pub seconds: f64,
}

fn read_q2_player_view(reader: SaveReader, _identity: &dyn UnifiedIdentityDecoder) -> Result<Q2PlayerView, WorldError> {
    let selected = reader.field("selectedItem");
    let timer = reader.field("timer");
    Ok(Q2PlayerView {
        angles: read_vector(reader.field("angles"))?,
        offset: read_vector(reader.field("offset"))?,
        kick_angles: read_vector(reader.field("kickAngles"))?,
        gun_angles: read_vector(reader.field("gunAngles"))?,
        gun_offset: read_vector(reader.field("gunOffset"))?,
        blend: read_color(reader.field("blend"))?,
        fov: reader.field("fov").finite()?,
        underwater: reader.field("underwater").boolean()?,
        flashes: read_vector(reader.field("flashes"))?,
        health: reader.field("health").finite()?,
        armor: reader.field("armor").finite()?,
        ammo: reader.field("ammo").finite()?,
        score: reader.field("score").finite()?,
        selected_item: if selected.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(namespaced(selected.field("item"))?)
        },
        timer: if timer.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(Q2ViewTimerFull {
                item: namespaced(timer.field("item"))?,
                seconds: timer.field("seconds").finite()?,
            })
        },
        spectator: reader.field("spectator").boolean()?,
        layouts: bounded_list(reader.field("layouts"), |item| item.string())?,
    })
}

fn write_q2_player_view(view: &Q2PlayerView) -> SaveJson {
    obj(vec![
        ("angles", write_vector(view.angles)),
        ("offset", write_vector(view.offset)),
        ("kickAngles", write_vector(view.kick_angles)),
        ("gunAngles", write_vector(view.gun_angles)),
        ("gunOffset", write_vector(view.gun_offset)),
        ("blend", write_color(view.blend)),
        ("fov", num(view.fov)),
        ("underwater", boolean(view.underwater)),
        ("flashes", write_vector(view.flashes)),
        ("health", num(view.health)),
        ("armor", num(view.armor)),
        ("ammo", num(view.ammo)),
        ("score", num(view.score)),
        (
            "selectedItem",
            view.selected_item
                .as_ref()
                .map_or(SaveJson::Null, |item| obj(vec![("item", json_str(item))])),
        ),
        (
            "timer",
            view.timer.as_ref().map_or(SaveJson::Null, |timer| {
                obj(vec![("item", json_str(&timer.item)), ("seconds", num(timer.seconds))])
            }),
        ),
        ("spectator", boolean(view.spectator)),
        (
            "layouts",
            arr(view.layouts.iter().map(|layout| json_str(layout)).collect()),
        ),
    ])
}

/// Q2 scoreboard row.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ScoreRow {
    /// Slot.
    pub slot: f64,
    /// Name.
    pub name: String,
    /// Score.
    pub score: f64,
    /// Ping.
    pub ping: f64,
    /// Minutes played.
    pub minutes: f64,
    /// Spectator flag.
    pub spectator: bool,
}

fn read_q2_score_row(reader: SaveReader) -> Result<Q2ScoreRow, WorldError> {
    Ok(Q2ScoreRow {
        slot: reader.field("slot").finite()?,
        name: reader.field("name").string()?,
        score: reader.field("score").finite()?,
        ping: reader.field("ping").finite()?,
        minutes: reader.field("minutes").finite()?,
        spectator: reader.field("spectator").boolean()?,
    })
}

fn write_q2_score_row(row: &Q2ScoreRow) -> SaveJson {
    obj(vec![
        ("slot", num(row.slot)),
        ("name", json_str(&row.name)),
        ("score", num(row.score)),
        ("ping", num(row.ping)),
        ("minutes", num(row.minutes)),
        ("spectator", boolean(row.spectator)),
    ])
}

/// Inventory label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2InventoryLabel {
    /// Item id.
    pub item: String,
    /// Display name.
    pub name: String,
}

/// Q2 player event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PlayerEvent {
    /// Print.
    Print {
        /// Target actor.
        target: Option<ActorId>,
        /// Level.
        level: Q2PrintLevel,
        /// Text.
        text: String,
    },
    /// Userinfo.
    Userinfo {
        /// Actor handle.
        actor: ActorId,
        /// Slot.
        slot: f64,
        /// Name.
        name: String,
        /// Skin.
        skin: String,
    },
    /// Stuff text.
    Stufftext {
        /// Actor handle.
        actor: ActorId,
        /// Text.
        text: String,
    },
    /// Player view.
    View {
        /// Actor handle.
        actor: ActorId,
        /// View.
        view: Q2PlayerView,
    },
    /// Scoreboard.
    Scoreboard {
        /// Actor handle.
        actor: ActorId,
        /// Rows.
        rows: Vec<Q2ScoreRow>,
        /// Killer actor.
        killer: Option<ActorId>,
        /// Reliable flag.
        reliable: bool,
    },
    /// Inventory.
    Inventory {
        /// Actor handle.
        actor: ActorId,
        /// Entries.
        entries: Vec<qa_world::inventory::InventoryEntry>,
        /// Visible flag.
        visible: Option<bool>,
        /// Selected item (present-but-null collapses to absent).
        selected: Option<Option<String>>,
        /// Labels.
        labels: Option<Vec<Q2InventoryLabel>>,
    },
    /// Help visibility.
    Help {
        /// Actor handle.
        actor: ActorId,
        /// Visible flag.
        visible: bool,
    },
    /// Load menu.
    LoadMenu {
        /// Actor handle.
        actor: ActorId,
    },
    /// Trail.
    Trail {
        /// Actor handle.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Time.
        time: f64,
    },
    /// Chase camera.
    Chase {
        /// Actor handle.
        actor: ActorId,
        /// Target actor.
        target: Option<ActorId>,
    },
}

fn read_q2_player_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q2PlayerEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "print" => Ok(Q2PlayerEvent::Print {
            target: reader.field("target").nullable(|target| read_actor(target, identity))?,
            level: read_print_level(reader.field("level"))?,
            text: reader.field("text").string()?,
        }),
        "userinfo" => Ok(Q2PlayerEvent::Userinfo {
            actor: read_actor(reader.field("actor"), identity)?,
            slot: reader.field("slot").finite()?,
            name: reader.field("name").string()?,
            skin: reader.field("skin").string()?,
        }),
        "stufftext" => Ok(Q2PlayerEvent::Stufftext {
            actor: read_actor(reader.field("actor"), identity)?,
            text: reader.field("text").string()?,
        }),
        "view" => Ok(Q2PlayerEvent::View {
            actor: read_actor(reader.field("actor"), identity)?,
            view: read_q2_player_view(reader.field("view"), identity)?,
        }),
        "scoreboard" => Ok(Q2PlayerEvent::Scoreboard {
            actor: read_actor(reader.field("actor"), identity)?,
            rows: bounded_list(reader.field("rows"), read_q2_score_row)?,
            killer: reader.field("killer").nullable(|killer| read_actor(killer, identity))?,
            reliable: reader.field("reliable").boolean()?,
        }),
        "inventory" => {
            let visible = reader.field("visible");
            let selected = reader.field("selected");
            let labels = reader.field("labels");
            Ok(Q2PlayerEvent::Inventory {
                actor: read_actor(reader.field("actor"), identity)?,
                entries: bounded_list(reader.field("entries"), read_inventory_entry)?,
                visible: if visible.value.is_none() {
                    None
                } else {
                    Some(visible.boolean()?)
                },
                selected: if selected.value.is_none() {
                    None
                } else {
                    Some(if selected.value == Some(&SaveJson::Null) {
                        None
                    } else {
                        Some(namespaced(selected)?)
                    })
                },
                labels: if labels.value.is_none() {
                    None
                } else {
                    Some(bounded_list(labels, |label| {
                        Ok(Q2InventoryLabel {
                            item: namespaced(label.field("item"))?,
                            name: label.field("name").string()?,
                        })
                    })?)
                },
            })
        }
        "help" => Ok(Q2PlayerEvent::Help {
            actor: read_actor(reader.field("actor"), identity)?,
            visible: reader.field("visible").boolean()?,
        }),
        "load-menu" => Ok(Q2PlayerEvent::LoadMenu {
            actor: read_actor(reader.field("actor"), identity)?,
        }),
        "trail" => Ok(Q2PlayerEvent::Trail {
            actor: read_actor(reader.field("actor"), identity)?,
            origin: read_vector(reader.field("origin"))?,
            time: reader.field("time").finite()?,
        }),
        "chase" => Ok(Q2PlayerEvent::Chase {
            actor: read_actor(reader.field("actor"), identity)?,
            target: reader.field("target").nullable(|target| read_actor(target, identity))?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q2_player_event(event: &Q2PlayerEvent) -> SaveJson {
    match event {
        Q2PlayerEvent::Print { target, level, text } => obj(vec![
            ("kind", json_str("print")),
            ("target", target.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("level", write_print_level(*level)),
            ("text", json_str(text)),
        ]),
        Q2PlayerEvent::Userinfo {
            actor,
            slot,
            name,
            skin,
        } => obj(vec![
            ("kind", json_str("userinfo")),
            ("actor", wire_actor(actor)),
            ("slot", num(*slot)),
            ("name", json_str(name)),
            ("skin", json_str(skin)),
        ]),
        Q2PlayerEvent::Stufftext { actor, text } => obj(vec![
            ("kind", json_str("stufftext")),
            ("actor", wire_actor(actor)),
            ("text", json_str(text)),
        ]),
        Q2PlayerEvent::View { actor, view } => obj(vec![
            ("kind", json_str("view")),
            ("actor", wire_actor(actor)),
            ("view", write_q2_player_view(view)),
        ]),
        Q2PlayerEvent::Scoreboard {
            actor,
            rows,
            killer,
            reliable,
        } => obj(vec![
            ("kind", json_str("scoreboard")),
            ("actor", wire_actor(actor)),
            ("rows", arr(rows.iter().map(write_q2_score_row).collect())),
            ("killer", killer.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("reliable", boolean(*reliable)),
        ]),
        Q2PlayerEvent::Inventory {
            actor,
            entries,
            visible,
            selected,
            labels,
        } => {
            let mut members = vec![
                ("kind", json_str("inventory")),
                ("actor", wire_actor(actor)),
                ("entries", arr(entries.iter().map(write_inventory_entry).collect())),
            ];
            if let Some(visible) = visible {
                members.push(("visible", boolean(*visible)));
            }
            if let Some(selected) = selected {
                members.push((
                    "selected",
                    selected.as_ref().map_or(SaveJson::Null, |item| json_str(item)),
                ));
            }
            if let Some(labels) = labels {
                members.push((
                    "labels",
                    arr(labels
                        .iter()
                        .map(|label| obj(vec![("item", json_str(&label.item)), ("name", json_str(&label.name))]))
                        .collect()),
                ));
            }
            obj(members)
        }
        Q2PlayerEvent::Help { actor, visible } => obj(vec![
            ("kind", json_str("help")),
            ("actor", wire_actor(actor)),
            ("visible", boolean(*visible)),
        ]),
        Q2PlayerEvent::LoadMenu { actor } => obj(vec![("kind", json_str("load-menu")), ("actor", wire_actor(actor))]),
        Q2PlayerEvent::Trail { actor, origin, time } => obj(vec![
            ("kind", json_str("trail")),
            ("actor", wire_actor(actor)),
            ("origin", write_vector(*origin)),
            ("time", num(*time)),
        ]),
        Q2PlayerEvent::Chase { actor, target } => obj(vec![
            ("kind", json_str("chase")),
            ("actor", wire_actor(actor)),
            ("target", target.as_ref().map_or(SaveJson::Null, wire_actor)),
        ]),
    }
}

/// Q3 character presentation event.
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterPresentationEvent {
    /// Sequence.
    pub sequence: f64,
    /// Time in milliseconds.
    pub time_milliseconds: f64,
    /// Event number.
    pub event: f64,
    /// Parameter.
    pub parameter: f64,
    /// Actor handle.
    pub actor: ActorId,
}

fn read_q3_character_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q3CharacterPresentationEvent, WorldError> {
    Ok(Q3CharacterPresentationEvent {
        sequence: reader.field("sequence").finite()?,
        time_milliseconds: reader.field("timeMilliseconds").finite()?,
        event: reader.field("event").finite()?,
        parameter: reader.field("parameter").finite()?,
        actor: read_actor(reader.field("actor"), identity)?,
    })
}

fn write_q3_character_event(event: &Q3CharacterPresentationEvent) -> SaveJson {
    obj(vec![
        ("sequence", num(event.sequence)),
        ("timeMilliseconds", num(event.time_milliseconds)),
        ("event", num(event.event)),
        ("parameter", num(event.parameter)),
        ("actor", wire_actor(&event.actor)),
    ])
}

/// Q3 contact event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ContactEvent {
    /// Hit.
    Hit {
        /// Hit point.
        point: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
    },
    /// Miss.
    Miss {
        /// End point.
        point: Vec3,
        /// Normal.
        normal: Vec3,
    },
    /// Lightning reflection.
    LightningReflection {
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
    },
    /// Gauntlet quad.
    GauntletQuad,
}

fn read_contact_event(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q3ContactEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "hit" => Ok(Q3ContactEvent::Hit {
            point: read_vector(reader.field("point"))?,
            normal: read_vector(reader.field("normal"))?,
            target: reader.field("target").nullable(|target| read_actor(target, identity))?,
        }),
        "miss" => Ok(Q3ContactEvent::Miss {
            point: read_vector(reader.field("point"))?,
            normal: read_vector(reader.field("normal"))?,
        }),
        "lightning-reflection" => Ok(Q3ContactEvent::LightningReflection {
            start: read_vector(reader.field("start"))?,
            end: read_vector(reader.field("end"))?,
        }),
        "gauntlet-quad" => Ok(Q3ContactEvent::GauntletQuad),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_contact_event(event: &Q3ContactEvent) -> SaveJson {
    match event {
        Q3ContactEvent::Hit { point, normal, target } => obj(vec![
            ("kind", json_str("hit")),
            ("point", write_vector(*point)),
            ("normal", write_vector(*normal)),
            ("target", target.as_ref().map_or(SaveJson::Null, wire_actor)),
        ]),
        Q3ContactEvent::Miss { point, normal } => obj(vec![
            ("kind", json_str("miss")),
            ("point", write_vector(*point)),
            ("normal", write_vector(*normal)),
        ]),
        Q3ContactEvent::LightningReflection { start, end } => obj(vec![
            ("kind", json_str("lightning-reflection")),
            ("start", write_vector(*start)),
            ("end", write_vector(*end)),
        ]),
        Q3ContactEvent::GauntletQuad => obj(vec![("kind", json_str("gauntlet-quad"))]),
    }
}

/// Q3 shotgun event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ShotgunEvent {
    /// Muzzle point.
    pub muzzle: Vec3,
    /// Direction.
    pub direction: Vec3,
    /// Seed.
    pub seed: f64,
}

fn read_shotgun_event(reader: SaveReader) -> Result<Q3ShotgunEvent, WorldError> {
    Ok(Q3ShotgunEvent {
        muzzle: read_vector(reader.field("muzzle"))?,
        direction: read_vector(reader.field("direction"))?,
        seed: reader.field("seed").finite()?,
    })
}

fn write_shotgun_event(event: Q3ShotgunEvent) -> SaveJson {
    obj(vec![
        ("muzzle", write_vector(event.muzzle)),
        ("direction", write_vector(event.direction)),
        ("seed", num(event.seed)),
    ])
}

/// Q3 rail-trail impact.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q3RailImpact {
    /// No impact.
    None,
    /// Surface impact.
    Surface {
        /// Normal.
        normal: Vec3,
    },
}

/// Q3 rail trail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3RailTrail {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Impact.
    pub impact: Q3RailImpact,
}

fn read_rail_trail(reader: SaveReader) -> Result<Q3RailTrail, WorldError> {
    let impact = reader.field("impact");
    Ok(Q3RailTrail {
        start: read_vector(reader.field("start"))?,
        end: read_vector(reader.field("end"))?,
        impact: if impact.value == Some(&SaveJson::Null) {
            Q3RailImpact::None
        } else {
            Q3RailImpact::Surface {
                normal: read_vector(impact.field("normal"))?,
            }
        },
    })
}

fn write_rail_trail(trail: Q3RailTrail) -> SaveJson {
    obj(vec![
        ("start", write_vector(trail.start)),
        ("end", write_vector(trail.end)),
        (
            "impact",
            match trail.impact {
                Q3RailImpact::None => SaveJson::Null,
                Q3RailImpact::Surface { normal } => obj(vec![("normal", write_vector(normal))]),
            },
        ),
    ])
}

/// Q3 shared ballistic event.
#[derive(Debug, Clone, PartialEq)]
pub enum Q3SharedBallisticEvent {
    /// Remove projectile.
    Remove {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
    },
    /// Bounce.
    Bounce {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
    },
    /// Trail.
    Trail {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
    },
    /// Fire.
    Fire {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Volume.
        volume: f64,
    },
    /// Projectile.
    Projectile {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Trajectory.
        trajectory: UnifiedTrajectory,
    },
    /// Impact.
    Impact {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Hit kind.
        hit_kind: Q3ImpactHitKind,
    },
    /// Contact.
    Contact {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Contact.
        contact: Q3ContactEvent,
    },
    /// Shotgun.
    Shotgun {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Shot.
        shot: Q3ShotgunEvent,
    },
    /// Rail trail.
    Rail {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Trail.
        trail: Q3RailTrail,
    },
    /// Rail award.
    RailAward {
        /// Actor handle.
        actor: ActorId,
        /// Weapon.
        weapon: f64,
        /// Origin.
        origin: Vec3,
        /// End point.
        end: Vec3,
        /// Normal.
        normal: Vec3,
        /// Target actor.
        target: Option<ActorId>,
        /// Surface flags.
        surface_flags: f64,
        /// Time in milliseconds.
        time_milliseconds: f64,
        /// Count.
        count: f64,
        /// Award time.
        until: f64,
    },
}

/// Q3 impact hit kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ImpactHitKind {
    /// Wall.
    Wall,
    /// Flesh.
    Flesh,
}

struct Q3BallisticBase {
    actor: ActorId,
    weapon: f64,
    origin: Vec3,
    end: Vec3,
    normal: Vec3,
    target: Option<ActorId>,
    surface_flags: f64,
    time_milliseconds: f64,
}

fn read_ballistic_base(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q3BallisticBase, WorldError> {
    Ok(Q3BallisticBase {
        actor: read_actor(reader.field("actor"), identity)?,
        weapon: reader.field("weapon").finite()?,
        origin: read_vector(reader.field("origin"))?,
        end: read_vector(reader.field("end"))?,
        normal: read_vector(reader.field("normal"))?,
        target: reader.field("target").nullable(|target| read_actor(target, identity))?,
        surface_flags: reader.field("surfaceFlags").finite()?,
        time_milliseconds: reader.field("timeMilliseconds").finite()?,
    })
}

fn write_ballistic_base<'a>(kind: &'a str, base: &Q3BallisticBase) -> Vec<(&'a str, SaveJson)> {
    vec![
        ("kind", json_str(kind)),
        ("actor", wire_actor(&base.actor)),
        ("weapon", num(base.weapon)),
        ("origin", write_vector(base.origin)),
        ("end", write_vector(base.end)),
        ("normal", write_vector(base.normal)),
        ("target", base.target.as_ref().map_or(SaveJson::Null, wire_actor)),
        ("surfaceFlags", num(base.surface_flags)),
        ("timeMilliseconds", num(base.time_milliseconds)),
    ]
}

fn read_q3_ballistic_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q3SharedBallisticEvent, WorldError> {
    let kind = reader.field("kind").string()?;
    let base = read_ballistic_base(reader.clone(), identity)?;
    match kind.as_str() {
        "remove" => Ok(Q3SharedBallisticEvent::Remove {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
        }),
        "bounce" => Ok(Q3SharedBallisticEvent::Bounce {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
        }),
        "trail" => Ok(Q3SharedBallisticEvent::Trail {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
        }),
        "fire" => Ok(Q3SharedBallisticEvent::Fire {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
            volume: reader.field("volume").finite()?,
        }),
        "projectile" => Ok(Q3SharedBallisticEvent::Projectile {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
            trajectory: read_trajectory(reader.field("trajectory"))?,
        }),
        "impact" => {
            let hit_kind = reader.field("hitKind").choice_str(&["wall", "flesh"])?;
            Ok(Q3SharedBallisticEvent::Impact {
                actor: base.actor,
                weapon: base.weapon,
                origin: base.origin,
                end: base.end,
                normal: base.normal,
                target: base.target,
                surface_flags: base.surface_flags,
                time_milliseconds: base.time_milliseconds,
                hit_kind: if hit_kind == "wall" {
                    Q3ImpactHitKind::Wall
                } else {
                    Q3ImpactHitKind::Flesh
                },
            })
        }
        "contact" => Ok(Q3SharedBallisticEvent::Contact {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
            contact: read_contact_event(reader.field("contact"), identity)?,
        }),
        "shotgun" => Ok(Q3SharedBallisticEvent::Shotgun {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
            shot: read_shotgun_event(reader.field("shot"))?,
        }),
        "rail" => Ok(Q3SharedBallisticEvent::Rail {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
            trail: read_rail_trail(reader.field("trail"))?,
        }),
        "rail-award" => Ok(Q3SharedBallisticEvent::RailAward {
            actor: base.actor,
            weapon: base.weapon,
            origin: base.origin,
            end: base.end,
            normal: base.normal,
            target: base.target,
            surface_flags: base.surface_flags,
            time_milliseconds: base.time_milliseconds,
            count: reader.field("count").finite()?,
            until: reader.field("until").finite()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_q3_ballistic_event(event: &Q3SharedBallisticEvent) -> SaveJson {
    let (kind, base, extra): (&str, Q3BallisticBase, Vec<(&str, SaveJson)>) = match event {
        Q3SharedBallisticEvent::Remove {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
        } => (
            "remove",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            Vec::new(),
        ),
        Q3SharedBallisticEvent::Bounce {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
        } => (
            "bounce",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            Vec::new(),
        ),
        Q3SharedBallisticEvent::Trail {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
        } => (
            "trail",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            Vec::new(),
        ),
        Q3SharedBallisticEvent::Fire {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            volume,
        } => (
            "fire",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![("volume", num(*volume))],
        ),
        Q3SharedBallisticEvent::Projectile {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            trajectory,
        } => (
            "projectile",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![("trajectory", write_trajectory(trajectory))],
        ),
        Q3SharedBallisticEvent::Impact {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            hit_kind,
        } => (
            "impact",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![(
                "hitKind",
                json_str(match hit_kind {
                    Q3ImpactHitKind::Wall => "wall",
                    Q3ImpactHitKind::Flesh => "flesh",
                }),
            )],
        ),
        Q3SharedBallisticEvent::Contact {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            contact,
        } => (
            "contact",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![("contact", write_contact_event(contact))],
        ),
        Q3SharedBallisticEvent::Shotgun {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            shot,
        } => (
            "shotgun",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![("shot", write_shotgun_event(*shot))],
        ),
        Q3SharedBallisticEvent::Rail {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            trail,
        } => (
            "rail",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![("trail", write_rail_trail(*trail))],
        ),
        Q3SharedBallisticEvent::RailAward {
            actor,
            weapon,
            origin,
            end,
            normal,
            target,
            surface_flags,
            time_milliseconds,
            count,
            until,
        } => (
            "rail-award",
            Q3BallisticBase {
                actor: actor.clone(),
                weapon: *weapon,
                origin: *origin,
                end: *end,
                normal: *normal,
                target: target.clone(),
                surface_flags: *surface_flags,
                time_milliseconds: *time_milliseconds,
            },
            vec![("count", num(*count)), ("until", num(*until))],
        ),
    };
    let mut members = write_ballistic_base(kind, &base);
    members.extend(extra);
    obj(members)
}

fn finite_vec3(reader: SaveReader, value: Vec3) -> Result<Vec3, WorldError> {
    if value.x.is_finite() && value.y.is_finite() && value.z.is_finite() {
        Ok(value)
    } else {
        Err(reader.fail("invalid source player vectors"))
    }
}

fn read_qvm_player_state(reader: SaveReader) -> Result<QvmPlayerState, UnifiedEventError> {
    let bytes = reader.bytes()?;
    if bytes.len() != qvm_player_state_bytes(AbiProfile::Modern) {
        return Err(reader.fail("invalid original player record extent").into());
    }
    let state = read_source_qvm_player_state(&bytes, AbiProfile::Modern)?;
    finite_vec3(reader.clone(), state.origin)?;
    finite_vec3(reader.clone(), state.velocity)?;
    finite_vec3(reader.clone(), state.grapple_point)?;
    finite_vec3(reader, state.view_angles)?;
    Ok(state)
}

fn encode_qvm_player_state(state: &QvmPlayerState) -> Result<SaveJson, GuestError> {
    let mut bytes = vec![0u8; qvm_player_state_bytes(AbiProfile::Modern)];
    write_qvm_player_state(&mut bytes, state, AbiProfile::Modern)?;
    Ok(SaveJson::Bytes(bytes))
}

/// Q3 player-event sequence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q3PlayerEventSequence {
    /// External event.
    External {
        /// Time.
        time: f64,
    },
    /// Predictable event.
    Predictable {
        /// Sequence.
        sequence: f64,
    },
}

fn read_player_event_sequence(reader: SaveReader) -> Result<Q3PlayerEventSequence, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "external" => Ok(Q3PlayerEventSequence::External {
            time: reader.field("time").finite()?,
        }),
        "predictable" => Ok(Q3PlayerEventSequence::Predictable {
            sequence: reader.field("sequence").integer(0)? as f64,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_player_event_sequence(sequence: Q3PlayerEventSequence) -> SaveJson {
    match sequence {
        Q3PlayerEventSequence::External { time } => obj(vec![("kind", json_str("external")), ("time", num(time))]),
        Q3PlayerEventSequence::Predictable { sequence } => {
            obj(vec![("kind", json_str("predictable")), ("sequence", num(sequence))])
        }
    }
}

/// Q3 ABI profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3AbiProfile {
    /// Modern ABI.
    Modern,
    /// Legacy 1.16n ABI.
    Legacy116n,
}

fn read_abi_profile(reader: SaveReader) -> Result<Q3AbiProfile, WorldError> {
    match reader.choice_str(&["q3-modern", "q3-1.16n-base"])?.as_str() {
        "q3-modern" => Ok(Q3AbiProfile::Modern),
        _ => Ok(Q3AbiProfile::Legacy116n),
    }
}

fn write_abi_profile(profile: Q3AbiProfile) -> SaveJson {
    json_str(match profile {
        Q3AbiProfile::Modern => "q3-modern",
        Q3AbiProfile::Legacy116n => "q3-1.16n-base",
    })
}

/// Q3 source event.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Q3SourceEvent {
    /// Print.
    Print {
        /// Text.
        text: String,
    },
    /// Log line.
    Log {
        /// Text.
        text: String,
    },
    /// Sound.
    Sound {
        /// Actor handle.
        actor: ActorId,
        /// Origin.
        origin: Vec3,
        /// Velocity.
        velocity: Vec3,
        /// Sound path.
        path: String,
        /// Channel.
        channel: i64,
        /// Volume.
        volume: f64,
        /// Loop flag.
        loop_sound: bool,
    },
    /// Server command.
    ServerCommand {
        /// Client number.
        client: f64,
        /// Text.
        text: String,
    },
    /// Console command.
    ConsoleCommand {
        /// Execution timing.
        execution: Q3ConsoleExecution,
        /// Text.
        text: String,
    },
    /// Drop client.
    DropClient {
        /// Client actor.
        client: ActorId,
        /// Reason.
        reason: String,
    },
    /// Configstring.
    Configstring {
        /// Index.
        index: f64,
        /// Value.
        value: String,
    },
    /// Player event.
    PlayerEvent {
        /// Actor handle.
        actor: ActorId,
        /// Module.
        module: UnifiedModuleIdentity,
        /// ABI profile.
        abi_profile: Q3AbiProfile,
        /// Player state.
        player_state: QvmPlayerState,
        /// Event number.
        event: i64,
        /// Parameter.
        parameter: i64,
        /// Sequence.
        sequence: Q3PlayerEventSequence,
        /// Origin.
        origin: Vec3,
        /// Time.
        time: f64,
    },
    /// Entity event.
    EntityEvent {
        /// Actor handle.
        actor: ActorId,
        /// Entity state.
        state: UnifiedQ3EntityState,
        /// Origin.
        origin: Vec3,
        /// Time.
        time: f64,
    },
}

/// Q3 console command execution timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ConsoleExecution {
    /// Append to buffer.
    Append,
    /// Execute now.
    Now,
}

fn read_q3_source_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q3SourceEvent, UnifiedEventError> {
    match reader.field("kind").string()?.as_str() {
        "print" => Ok(Q3SourceEvent::Print {
            text: reader.field("text").string()?,
        }),
        "log" => Ok(Q3SourceEvent::Log {
            text: reader.field("text").string()?,
        }),
        "sound" => Ok(Q3SourceEvent::Sound {
            actor: read_actor(reader.field("actor"), identity)?,
            origin: read_vector(reader.field("origin"))?,
            velocity: read_vector(reader.field("velocity"))?,
            path: reader.field("path").string()?,
            channel: reader.field("channel").integer(0)?,
            volume: reader.field("volume").finite()?,
            loop_sound: reader.field("loop").boolean()?,
        }),
        "server-command" => Ok(Q3SourceEvent::ServerCommand {
            client: reader.field("client").finite()?,
            text: reader.field("text").string()?,
        }),
        "console-command" => {
            let execution = reader.field("execution").choice_str(&["append", "now"])?;
            Ok(Q3SourceEvent::ConsoleCommand {
                execution: if execution == "append" {
                    Q3ConsoleExecution::Append
                } else {
                    Q3ConsoleExecution::Now
                },
                text: reader.field("text").string()?,
            })
        }
        "drop-client" => Ok(Q3SourceEvent::DropClient {
            client: read_actor(reader.field("client"), identity)?,
            reason: reader.field("reason").string()?,
        }),
        "configstring" => Ok(Q3SourceEvent::Configstring {
            index: reader.field("index").finite()?,
            value: reader.field("value").string()?,
        }),
        "player-event" => {
            let source = reader.field("source");
            Ok(Q3SourceEvent::PlayerEvent {
                actor: read_actor(reader.field("actor"), identity)?,
                module: read_module(source.field("module"))?,
                abi_profile: read_abi_profile(source.field("abiProfile"))?,
                player_state: read_qvm_player_state(reader.field("playerState"))?,
                event: reader.field("event").integer(0)?,
                parameter: reader.field("parameter").integer(0)?,
                sequence: read_player_event_sequence(reader.field("sequence"))?,
                origin: read_vector(reader.field("origin"))?,
                time: reader.field("time").finite()?,
            })
        }
        "entity-event" => Ok(Q3SourceEvent::EntityEvent {
            actor: read_actor(reader.field("actor"), identity)?,
            state: read_entity_state(reader.field("state"))?,
            origin: read_vector(reader.field("origin"))?,
            time: reader.field("time").finite()?,
        }),
        _ => Err(reader.fail("unknown event variant").into()),
    }
}

fn write_q3_source_event(event: &Q3SourceEvent) -> Result<SaveJson, UnifiedEventError> {
    match event {
        Q3SourceEvent::Print { text } => Ok(obj(vec![("kind", json_str("print")), ("text", json_str(text))])),
        Q3SourceEvent::Log { text } => Ok(obj(vec![("kind", json_str("log")), ("text", json_str(text))])),
        Q3SourceEvent::Sound {
            actor,
            origin,
            velocity,
            path,
            channel,
            volume,
            loop_sound,
        } => Ok(obj(vec![
            ("kind", json_str("sound")),
            ("actor", wire_actor(actor)),
            ("origin", write_vector(*origin)),
            ("velocity", write_vector(*velocity)),
            ("path", json_str(path)),
            ("channel", int(*channel)),
            ("volume", num(*volume)),
            ("loop", boolean(*loop_sound)),
        ])),
        Q3SourceEvent::ServerCommand { client, text } => Ok(obj(vec![
            ("kind", json_str("server-command")),
            ("client", num(*client)),
            ("text", json_str(text)),
        ])),
        Q3SourceEvent::ConsoleCommand { execution, text } => Ok(obj(vec![
            ("kind", json_str("console-command")),
            (
                "execution",
                json_str(match execution {
                    Q3ConsoleExecution::Append => "append",
                    Q3ConsoleExecution::Now => "now",
                }),
            ),
            ("text", json_str(text)),
        ])),
        Q3SourceEvent::DropClient { client, reason } => Ok(obj(vec![
            ("kind", json_str("drop-client")),
            ("client", wire_actor(client)),
            ("reason", json_str(reason)),
        ])),
        Q3SourceEvent::Configstring { index, value } => Ok(obj(vec![
            ("kind", json_str("configstring")),
            ("index", num(*index)),
            ("value", json_str(value)),
        ])),
        Q3SourceEvent::PlayerEvent {
            actor,
            module,
            abi_profile,
            player_state,
            event,
            parameter,
            sequence,
            origin,
            time,
        } => Ok(obj(vec![
            ("kind", json_str("player-event")),
            ("actor", wire_actor(actor)),
            (
                "source",
                obj(vec![
                    ("module", write_module(module)),
                    ("abiProfile", write_abi_profile(*abi_profile)),
                ]),
            ),
            ("playerState", encode_qvm_player_state(player_state)?),
            ("event", int(*event)),
            ("parameter", int(*parameter)),
            ("sequence", write_player_event_sequence(*sequence)),
            ("origin", write_vector(*origin)),
            ("time", num(*time)),
        ])),
        Q3SourceEvent::EntityEvent {
            actor,
            state,
            origin,
            time,
        } => Ok(obj(vec![
            ("kind", json_str("entity-event")),
            ("actor", wire_actor(actor)),
            ("state", write_entity_state(state)),
            ("origin", write_vector(*origin)),
            ("time", num(*time)),
        ])),
    }
}

/// Environment hazard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentHazard {
    /// Falling.
    Falling,
    /// Lava.
    Lava,
    /// Slime.
    Slime,
    /// Water.
    Water,
    /// Void.
    Void,
    /// Crush.
    Crush,
}

fn read_hazard(reader: SaveReader) -> Result<EnvironmentHazard, WorldError> {
    match reader
        .choice_str(&["falling", "lava", "slime", "water", "void", "crush"])?
        .as_str()
    {
        "falling" => Ok(EnvironmentHazard::Falling),
        "lava" => Ok(EnvironmentHazard::Lava),
        "slime" => Ok(EnvironmentHazard::Slime),
        "water" => Ok(EnvironmentHazard::Water),
        "void" => Ok(EnvironmentHazard::Void),
        _ => Ok(EnvironmentHazard::Crush),
    }
}

fn write_hazard(hazard: EnvironmentHazard) -> SaveJson {
    json_str(match hazard {
        EnvironmentHazard::Falling => "falling",
        EnvironmentHazard::Lava => "lava",
        EnvironmentHazard::Slime => "slime",
        EnvironmentHazard::Water => "water",
        EnvironmentHazard::Void => "void",
        EnvironmentHazard::Crush => "crush",
    })
}

/// Q2 classic native damage game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2NativeGame {
    /// Base game.
    Base,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
    /// CTF.
    Ctf,
}

/// Q2 classic native damage cause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ClassicNativeCause {
    /// Game.
    pub game: Q2NativeGame,
    /// Value.
    pub value: f64,
}

/// Q2 rerelease native damage cause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2RereleaseNativeCause {
    /// Cause id.
    pub id: f64,
    /// Friendly-fire flag.
    pub friendly_fire: bool,
    /// No-point-loss flag.
    pub no_point_loss: bool,
}

/// Q2 native damage cause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Q2NativeCause {
    /// Classic cause.
    Classic(Q2ClassicNativeCause),
    /// Rerelease cause.
    Rerelease(Q2RereleaseNativeCause),
}

fn read_native_cause(reader: SaveReader) -> Result<Q2NativeCause, WorldError> {
    match reader.field("edition").choice_str(&["classic", "rerelease"])?.as_str() {
        "classic" => {
            let game = reader.field("game").choice_str(&["base", "xatrix", "rogue", "ctf"])?;
            Ok(Q2NativeCause::Classic(Q2ClassicNativeCause {
                game: match game.as_str() {
                    "base" => Q2NativeGame::Base,
                    "xatrix" => Q2NativeGame::Xatrix,
                    "rogue" => Q2NativeGame::Rogue,
                    _ => Q2NativeGame::Ctf,
                },
                value: reader.field("value").finite()?,
            }))
        }
        _ => Ok(Q2NativeCause::Rerelease(Q2RereleaseNativeCause {
            id: reader.field("id").finite()?,
            friendly_fire: reader.field("friendlyFire").boolean()?,
            no_point_loss: reader.field("noPointLoss").boolean()?,
        })),
    }
}

fn write_native_cause(cause: Q2NativeCause) -> SaveJson {
    match cause {
        Q2NativeCause::Classic(cause) => obj(vec![
            ("edition", json_str("classic")),
            (
                "game",
                json_str(match cause.game {
                    Q2NativeGame::Base => "base",
                    Q2NativeGame::Xatrix => "xatrix",
                    Q2NativeGame::Rogue => "rogue",
                    Q2NativeGame::Ctf => "ctf",
                }),
            ),
            ("value", num(cause.value)),
        ]),
        Q2NativeCause::Rerelease(cause) => obj(vec![
            ("edition", json_str("rerelease")),
            ("id", num(cause.id)),
            ("friendlyFire", boolean(cause.friendly_fire)),
            ("noPointLoss", boolean(cause.no_point_loss)),
        ]),
    }
}

/// Attack cause.
#[derive(Debug, Clone, PartialEq)]
pub enum AttackCause {
    /// Q1 cause.
    Q1 {
        /// Death type.
        death_type: f64,
        /// Armor effect.
        armor_effect: Option<String>,
    },
    /// Q2 cause.
    Q2 {
        /// Means of death.
        means_of_death: f64,
        /// Damage flags.
        damage_flags: f64,
        /// Native cause.
        native: Option<Q2NativeCause>,
    },
    /// Q3 cause.
    Q3 {
        /// Means of death.
        means_of_death: f64,
        /// Damage flags.
        damage_flags: f64,
    },
    /// Environment cause.
    Environment {
        /// Hazard.
        hazard: EnvironmentHazard,
    },
}

fn read_attack_cause(reader: SaveReader) -> Result<AttackCause, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "q1" => {
            let armor_effect = reader.field("armorEffect");
            Ok(AttackCause::Q1 {
                death_type: reader.field("deathType").finite()?,
                armor_effect: if armor_effect.value.is_none() {
                    None
                } else {
                    Some(namespaced(armor_effect)?)
                },
            })
        }
        "q2" => {
            let native = reader.field("native");
            Ok(AttackCause::Q2 {
                means_of_death: reader.field("meansOfDeath").finite()?,
                damage_flags: reader.field("damageFlags").finite()?,
                native: if native.value.is_none() {
                    None
                } else {
                    Some(read_native_cause(native)?)
                },
            })
        }
        "q3" => Ok(AttackCause::Q3 {
            means_of_death: reader.field("meansOfDeath").finite()?,
            damage_flags: reader.field("damageFlags").finite()?,
        }),
        "environment" => Ok(AttackCause::Environment {
            hazard: read_hazard(reader.field("hazard"))?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_attack_cause(cause: &AttackCause) -> SaveJson {
    match cause {
        AttackCause::Q1 {
            death_type,
            armor_effect,
        } => {
            let mut members = vec![("kind", json_str("q1")), ("deathType", num(*death_type))];
            if let Some(effect) = armor_effect {
                members.push(("armorEffect", json_str(effect)));
            }
            obj(members)
        }
        AttackCause::Q2 {
            means_of_death,
            damage_flags,
            native,
        } => {
            let mut members = vec![
                ("kind", json_str("q2")),
                ("meansOfDeath", num(*means_of_death)),
                ("damageFlags", num(*damage_flags)),
            ];
            if let Some(native) = native {
                members.push(("native", write_native_cause(*native)));
            }
            obj(members)
        }
        AttackCause::Q3 {
            means_of_death,
            damage_flags,
        } => obj(vec![
            ("kind", json_str("q3")),
            ("meansOfDeath", num(*means_of_death)),
            ("damageFlags", num(*damage_flags)),
        ]),
        AttackCause::Environment { hazard } => obj(vec![
            ("kind", json_str("environment")),
            ("hazard", write_hazard(*hazard)),
        ]),
    }
}

/// Q1 event-effect actor reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1EffectActor {
    /// Absent.
    None,
    /// World.
    World,
    /// Actor.
    Actor(ActorId),
}

fn read_effect_actor(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Q1EffectActor, WorldError> {
    if reader.value == Some(&SaveJson::Null) {
        return Ok(Q1EffectActor::None);
    }
    match reader.field("kind").string()?.as_str() {
        "world" => Ok(Q1EffectActor::World),
        "actor" => Ok(Q1EffectActor::Actor(read_actor(reader.field("actor"), identity)?)),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_effect_actor(actor: &Q1EffectActor) -> SaveJson {
    match actor {
        Q1EffectActor::None => SaveJson::Null,
        Q1EffectActor::World => obj(vec![("kind", json_str("world"))]),
        Q1EffectActor::Actor(actor) => obj(vec![("kind", json_str("actor")), ("actor", wire_actor(actor))]),
    }
}

/// Attack provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackProvenance {
    /// Sequence.
    pub sequence: f64,
    /// Time.
    pub time: SourceTime,
    /// Attacker.
    pub attacker: Q1EffectActor,
    /// Inflictor.
    pub inflictor: Q1EffectActor,
    /// Originating projectile.
    pub originating_projectile: Option<ActorId>,
    /// Weapon.
    pub weapon: Option<String>,
    /// Weapon provider.
    pub weapon_provider: String,
    /// Combat provider.
    pub combat_provider: String,
    /// Inventory provider.
    pub inventory_provider: String,
    /// Movement provider.
    pub movement_provider: String,
    /// Cause.
    pub cause: AttackCause,
}

fn read_attack_provenance(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<AttackProvenance, WorldError> {
    let projectile = reader.field("originatingProjectile");
    let weapon = reader.field("weapon");
    Ok(AttackProvenance {
        sequence: reader.field("sequence").finite()?,
        time: read_time(reader.field("time"))?,
        attacker: read_effect_actor(reader.field("attacker"), identity)?,
        inflictor: read_effect_actor(reader.field("inflictor"), identity)?,
        originating_projectile: if projectile.value.is_none() {
            None
        } else {
            Some(read_actor(projectile, identity)?)
        },
        weapon: if weapon.value == Some(&SaveJson::Null) {
            None
        } else {
            Some(namespaced(weapon)?)
        },
        weapon_provider: namespaced(reader.field("weaponProvider"))?,
        combat_provider: namespaced(reader.field("combatProvider"))?,
        inventory_provider: namespaced(reader.field("inventoryProvider"))?,
        movement_provider: namespaced(reader.field("movementProvider"))?,
        cause: read_attack_cause(reader.field("cause"))?,
    })
}

fn write_attack_provenance(attack: &AttackProvenance) -> SaveJson {
    let mut members = vec![
        ("sequence", num(attack.sequence)),
        ("time", write_time(attack.time)),
        ("attacker", write_effect_actor(&attack.attacker)),
        ("inflictor", write_effect_actor(&attack.inflictor)),
    ];
    if let Some(projectile) = &attack.originating_projectile {
        members.push(("originatingProjectile", wire_actor(projectile)));
    }
    members.push((
        "weapon",
        attack.weapon.as_ref().map_or(SaveJson::Null, |weapon| json_str(weapon)),
    ));
    members.push(("weaponProvider", json_str(&attack.weapon_provider)));
    members.push(("combatProvider", json_str(&attack.combat_provider)));
    members.push(("inventoryProvider", json_str(&attack.inventory_provider)));
    members.push(("movementProvider", json_str(&attack.movement_provider)));
    members.push(("cause", write_attack_cause(&attack.cause)));
    obj(members)
}

/// Damage delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageDelivery {
    /// Direct damage.
    Direct,
    /// Radius damage.
    Radius,
}

/// Damage request.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageRequest {
    /// Attack provenance.
    pub attack: AttackProvenance,
    /// Target actor.
    pub target: ActorId,
    /// Amount.
    pub amount: f64,
    /// Knockback.
    pub knockback: f64,
    /// Direction.
    pub direction: Vec3,
    /// Hit point.
    pub point: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Delivery.
    pub delivery: DamageDelivery,
}

fn read_damage_request(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<DamageRequest, WorldError> {
    let delivery = reader.field("delivery").choice_str(&["direct", "radius"])?;
    Ok(DamageRequest {
        attack: read_attack_provenance(reader.field("attack"), identity)?,
        target: read_actor(reader.field("target"), identity)?,
        amount: reader.field("amount").finite()?,
        knockback: reader.field("knockback").finite()?,
        direction: read_vector(reader.field("direction"))?,
        point: read_vector(reader.field("point"))?,
        normal: read_vector(reader.field("normal"))?,
        delivery: if delivery == "direct" {
            DamageDelivery::Direct
        } else {
            DamageDelivery::Radius
        },
    })
}

fn write_damage_request(request: &DamageRequest) -> SaveJson {
    obj(vec![
        ("attack", write_attack_provenance(&request.attack)),
        ("target", wire_actor(&request.target)),
        ("amount", num(request.amount)),
        ("knockback", num(request.knockback)),
        ("direction", write_vector(request.direction)),
        ("point", write_vector(request.point)),
        ("normal", write_vector(request.normal)),
        (
            "delivery",
            json_str(match request.delivery {
                DamageDelivery::Direct => "direct",
                DamageDelivery::Radius => "radius",
            }),
        ),
    ])
}

/// Damage mutation.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageMutation {
    /// Health change.
    Health {
        /// Value before.
        before: f64,
        /// Value after.
        after: f64,
    },
    /// Armor change.
    Armor {
        /// State before.
        before: ArmorState,
        /// State after.
        after: ArmorState,
    },
    /// Source velocity change.
    SourceVelocity {
        /// Value before.
        before: Vec3,
        /// Value after.
        after: Vec3,
        /// Movement provider.
        movement_provider: String,
    },
    /// Impulse.
    Impulse {
        /// Impulse vector.
        impulse: Vec3,
        /// Movement provider.
        movement_provider: String,
    },
}

fn read_damage_mutation(reader: SaveReader) -> Result<DamageMutation, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "health" => Ok(DamageMutation::Health {
            before: reader.field("before").finite()?,
            after: reader.field("after").finite()?,
        }),
        "armor" => Ok(DamageMutation::Armor {
            before: read_armor(reader.field("before"))?,
            after: read_armor(reader.field("after"))?,
        }),
        "source-velocity" => Ok(DamageMutation::SourceVelocity {
            before: read_vector(reader.field("before"))?,
            after: read_vector(reader.field("after"))?,
            movement_provider: namespaced(reader.field("movementProvider"))?,
        }),
        "impulse" => Ok(DamageMutation::Impulse {
            impulse: read_vector(reader.field("impulse"))?,
            movement_provider: namespaced(reader.field("movementProvider"))?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_damage_mutation(mutation: &DamageMutation) -> SaveJson {
    match mutation {
        DamageMutation::Health { before, after } => obj(vec![
            ("kind", json_str("health")),
            ("before", num(*before)),
            ("after", num(*after)),
        ]),
        DamageMutation::Armor { before, after } => obj(vec![
            ("kind", json_str("armor")),
            ("before", write_armor(before)),
            ("after", write_armor(after)),
        ]),
        DamageMutation::SourceVelocity {
            before,
            after,
            movement_provider,
        } => obj(vec![
            ("kind", json_str("source-velocity")),
            ("before", write_vector(*before)),
            ("after", write_vector(*after)),
            ("movementProvider", json_str(movement_provider)),
        ]),
        DamageMutation::Impulse {
            impulse,
            movement_provider,
        } => obj(vec![
            ("kind", json_str("impulse")),
            ("impulse", write_vector(*impulse)),
            ("movementProvider", json_str(movement_provider)),
        ]),
    }
}

/// Damage reaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageReaction {
    /// No reaction.
    None,
    /// Pain.
    Pain,
    /// Death.
    Death,
}

/// Damage decision feedback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DamageDecisionFeedback {
    /// Q2 feedback.
    Q2 {
        /// Power armor absorbed.
        power_armor: f64,
        /// Armor absorbed.
        armor: f64,
        /// Blood.
        blood: f64,
        /// Knockback.
        knockback: f64,
    },
    /// Q3 feedback.
    Q3 {
        /// Knockback.
        knockback: f64,
        /// Battlesuit protection.
        battlesuit: bool,
    },
}

fn read_damage_feedback(reader: SaveReader) -> Result<DamageDecisionFeedback, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "q2" => Ok(DamageDecisionFeedback::Q2 {
            power_armor: reader.field("powerArmor").finite()?,
            armor: reader.field("armor").finite()?,
            blood: reader.field("blood").finite()?,
            knockback: reader.field("knockback").finite()?,
        }),
        "q3" => Ok(DamageDecisionFeedback::Q3 {
            knockback: reader.field("knockback").finite()?,
            battlesuit: reader.field("battlesuit").boolean()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_damage_feedback(feedback: DamageDecisionFeedback) -> SaveJson {
    match feedback {
        DamageDecisionFeedback::Q2 {
            power_armor,
            armor,
            blood,
            knockback,
        } => obj(vec![
            ("kind", json_str("q2")),
            ("powerArmor", num(power_armor)),
            ("armor", num(armor)),
            ("blood", num(blood)),
            ("knockback", num(knockback)),
        ]),
        DamageDecisionFeedback::Q3 { knockback, battlesuit } => obj(vec![
            ("kind", json_str("q3")),
            ("knockback", num(knockback)),
            ("battlesuit", boolean(battlesuit)),
        ]),
    }
}

/// Damage decision.
#[derive(Debug, Clone, PartialEq)]
pub struct DamageDecision {
    /// Request.
    pub request: DamageRequest,
    /// Mutations.
    pub mutations: Vec<DamageMutation>,
    /// Applied damage.
    pub applied_damage: f64,
    /// Reaction.
    pub reaction: DamageReaction,
    /// Feedback.
    pub feedback: Option<DamageDecisionFeedback>,
}

fn read_damage_decision(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<DamageDecision, WorldError> {
    let feedback = reader.field("feedback");
    let reaction = reader.field("reaction").choice_str(&["none", "pain", "death"])?;
    Ok(DamageDecision {
        request: read_damage_request(reader.field("request"), identity)?,
        mutations: bounded_list(reader.field("mutations"), read_damage_mutation)?,
        applied_damage: reader.field("appliedDamage").finite()?,
        reaction: match reaction.as_str() {
            "none" => DamageReaction::None,
            "pain" => DamageReaction::Pain,
            _ => DamageReaction::Death,
        },
        feedback: if feedback.value.is_none() {
            None
        } else {
            Some(read_damage_feedback(feedback)?)
        },
    })
}

fn write_damage_decision(decision: &DamageDecision) -> SaveJson {
    let mut members = vec![
        ("request", write_damage_request(&decision.request)),
        (
            "mutations",
            arr(decision.mutations.iter().map(write_damage_mutation).collect()),
        ),
        ("appliedDamage", num(decision.applied_damage)),
        (
            "reaction",
            json_str(match decision.reaction {
                DamageReaction::None => "none",
                DamageReaction::Pain => "pain",
                DamageReaction::Death => "death",
            }),
        ),
    ];
    if let Some(feedback) = decision.feedback {
        members.push(("feedback", write_damage_feedback(feedback)));
    }
    obj(members)
}

/// Damage outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum DamageOutcome {
    /// Stale target.
    StaleTarget {
        /// Request.
        request: DamageRequest,
    },
    /// Committed decision.
    Committed {
        /// Decision.
        decision: DamageDecision,
        /// Survived flag.
        survived: bool,
    },
}

fn read_damage_outcome(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<DamageOutcome, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "stale-target" => Ok(DamageOutcome::StaleTarget {
            request: read_damage_request(reader.field("request"), identity)?,
        }),
        "committed" => Ok(DamageOutcome::Committed {
            decision: read_damage_decision(reader.field("decision"), identity)?,
            survived: reader.field("survived").boolean()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_damage_outcome(outcome: &DamageOutcome) -> SaveJson {
    match outcome {
        DamageOutcome::StaleTarget { request } => obj(vec![
            ("kind", json_str("stale-target")),
            ("request", write_damage_request(request)),
        ]),
        DamageOutcome::Committed { decision, survived } => obj(vec![
            ("kind", json_str("committed")),
            ("decision", write_damage_decision(decision)),
            ("survived", boolean(*survived)),
        ]),
    }
}

/// Network event.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedNetworkEvent {
    /// Print.
    Print {
        /// Level.
        level: Q2PrintLevel,
        /// Text.
        text: String,
    },
    /// Center print.
    CenterPrint {
        /// Text.
        text: String,
    },
    /// Command text.
    CommandText {
        /// Text.
        text: String,
    },
    /// Configstring.
    ConfigString {
        /// Index.
        index: f64,
        /// Value.
        value: String,
    },
    /// Sound.
    Sound {
        /// Entity number.
        entity_number: f64,
        /// Channel.
        channel: f64,
        /// Sound index.
        sound_index: f64,
        /// Origin.
        origin: Option<Vec3>,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
        /// Delay in seconds.
        delay_seconds: f64,
    },
    /// Q1 damage feedback.
    Q1Damage {
        /// Armor.
        armor: f64,
        /// Blood.
        blood: f64,
        /// Source origin.
        source: Vec3,
    },
    /// Q1 particles.
    Q1Particle {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Count.
        count: f64,
        /// Color.
        color: f64,
    },
    /// Q2 layout.
    Q2Layout {
        /// Program.
        program: String,
    },
    /// Q2 inventory.
    Q2Inventory {
        /// Counts.
        counts: Vec<f64>,
    },
    /// Q2 muzzle flash.
    Q2MuzzleFlash {
        /// Entity number.
        entity_number: f64,
        /// Flash number.
        flash: f64,
        /// Monster flag.
        monster: bool,
    },
    /// Q3 server command.
    Q3ServerCommand {
        /// Sequence.
        sequence: f64,
        /// Text.
        text: String,
    },
    /// Disconnect.
    Disconnect {
        /// Reason.
        reason: String,
    },
}

fn read_network_event(
    reader: SaveReader,
    _identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedNetworkEvent, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "print" => Ok(UnifiedNetworkEvent::Print {
            level: read_print_level(reader.field("level"))?,
            text: reader.field("text").string()?,
        }),
        "center-print" => Ok(UnifiedNetworkEvent::CenterPrint {
            text: reader.field("text").string()?,
        }),
        "command-text" => Ok(UnifiedNetworkEvent::CommandText {
            text: reader.field("text").string()?,
        }),
        "config-string" => Ok(UnifiedNetworkEvent::ConfigString {
            index: reader.field("index").finite()?,
            value: reader.field("value").string()?,
        }),
        "sound" => {
            let origin = reader.field("origin");
            Ok(UnifiedNetworkEvent::Sound {
                entity_number: reader.field("entityNumber").finite()?,
                channel: reader.field("channel").finite()?,
                sound_index: reader.field("soundIndex").finite()?,
                origin: if origin.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(read_vector(origin)?)
                },
                volume: reader.field("volume").finite()?,
                attenuation: reader.field("attenuation").finite()?,
                delay_seconds: reader.field("delaySeconds").finite()?,
            })
        }
        "q1-damage" => Ok(UnifiedNetworkEvent::Q1Damage {
            armor: reader.field("armor").finite()?,
            blood: reader.field("blood").finite()?,
            source: read_vector(reader.field("source"))?,
        }),
        "q1-particle" => Ok(UnifiedNetworkEvent::Q1Particle {
            origin: read_vector(reader.field("origin"))?,
            direction: read_vector(reader.field("direction"))?,
            count: reader.field("count").finite()?,
            color: reader.field("color").finite()?,
        }),
        "q2-layout" => Ok(UnifiedNetworkEvent::Q2Layout {
            program: reader.field("program").string()?,
        }),
        "q2-inventory" => Ok(UnifiedNetworkEvent::Q2Inventory {
            counts: bounded_list(reader.field("counts"), |item| item.finite())?,
        }),
        "q2-muzzle-flash" => Ok(UnifiedNetworkEvent::Q2MuzzleFlash {
            entity_number: reader.field("entityNumber").finite()?,
            flash: reader.field("flash").finite()?,
            monster: reader.field("monster").boolean()?,
        }),
        "q3-server-command" => Ok(UnifiedNetworkEvent::Q3ServerCommand {
            sequence: reader.field("sequence").finite()?,
            text: reader.field("text").string()?,
        }),
        "disconnect" => Ok(UnifiedNetworkEvent::Disconnect {
            reason: reader.field("reason").string()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_network_event(event: &UnifiedNetworkEvent) -> SaveJson {
    match event {
        UnifiedNetworkEvent::Print { level, text } => obj(vec![
            ("kind", json_str("print")),
            ("level", write_print_level(*level)),
            ("text", json_str(text)),
        ]),
        UnifiedNetworkEvent::CenterPrint { text } => {
            obj(vec![("kind", json_str("center-print")), ("text", json_str(text))])
        }
        UnifiedNetworkEvent::CommandText { text } => {
            obj(vec![("kind", json_str("command-text")), ("text", json_str(text))])
        }
        UnifiedNetworkEvent::ConfigString { index, value } => obj(vec![
            ("kind", json_str("config-string")),
            ("index", num(*index)),
            ("value", json_str(value)),
        ]),
        UnifiedNetworkEvent::Sound {
            entity_number,
            channel,
            sound_index,
            origin,
            volume,
            attenuation,
            delay_seconds,
        } => obj(vec![
            ("kind", json_str("sound")),
            ("entityNumber", num(*entity_number)),
            ("channel", num(*channel)),
            ("soundIndex", num(*sound_index)),
            ("origin", origin.map_or(SaveJson::Null, write_vector)),
            ("volume", num(*volume)),
            ("attenuation", num(*attenuation)),
            ("delaySeconds", num(*delay_seconds)),
        ]),
        UnifiedNetworkEvent::Q1Damage { armor, blood, source } => obj(vec![
            ("kind", json_str("q1-damage")),
            ("armor", num(*armor)),
            ("blood", num(*blood)),
            ("source", write_vector(*source)),
        ]),
        UnifiedNetworkEvent::Q1Particle {
            origin,
            direction,
            count,
            color,
        } => obj(vec![
            ("kind", json_str("q1-particle")),
            ("origin", write_vector(*origin)),
            ("direction", write_vector(*direction)),
            ("count", num(*count)),
            ("color", num(*color)),
        ]),
        UnifiedNetworkEvent::Q2Layout { program } => {
            obj(vec![("kind", json_str("q2-layout")), ("program", json_str(program))])
        }
        UnifiedNetworkEvent::Q2Inventory { counts } => obj(vec![
            ("kind", json_str("q2-inventory")),
            ("counts", arr(counts.iter().copied().map(num).collect())),
        ]),
        UnifiedNetworkEvent::Q2MuzzleFlash {
            entity_number,
            flash,
            monster,
        } => obj(vec![
            ("kind", json_str("q2-muzzle-flash")),
            ("entityNumber", num(*entity_number)),
            ("flash", num(*flash)),
            ("monster", boolean(*monster)),
        ]),
        UnifiedNetworkEvent::Q3ServerCommand { sequence, text } => obj(vec![
            ("kind", json_str("q3-server-command")),
            ("sequence", num(*sequence)),
            ("text", json_str(text)),
        ]),
        UnifiedNetworkEvent::Disconnect { reason } => {
            obj(vec![("kind", json_str("disconnect")), ("reason", json_str(reason))])
        }
    }
}

/// Simulation event audience.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedEventAudience {
    /// Whole world.
    World,
    /// One seat.
    Seat(SeatId),
    /// One client.
    Client(ClientId),
}

fn read_audience(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedEventAudience, WorldError> {
    match reader.field("kind").choice_str(&["world", "seat", "client"])?.as_str() {
        "seat" => Ok(UnifiedEventAudience::Seat(read_seat(reader.field("seat"), identity)?)),
        "client" => Ok(UnifiedEventAudience::Client(read_client(
            reader.field("client"),
            identity,
        )?)),
        _ => Ok(UnifiedEventAudience::World),
    }
}

fn write_audience(audience: &UnifiedEventAudience) -> SaveJson {
    match audience {
        UnifiedEventAudience::World => obj(vec![("kind", json_str("world"))]),
        UnifiedEventAudience::Seat(seat) => obj(vec![("kind", json_str("seat")), ("seat", write_seat(seat))]),
        UnifiedEventAudience::Client(client) => {
            obj(vec![("kind", json_str("client")), ("client", write_client(client))])
        }
    }
}

/// Transition decision.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedTransitionDecision {
    /// Stay on the current map.
    Stay {
        /// Blocked exits.
        blocked: Vec<String>,
    },
    /// Round result.
    Round {
        /// Winner.
        winner: Option<String>,
    },
    /// Travel to a map.
    Travel {
        /// Map name.
        map: String,
        /// Spawn point.
        spawn_point: String,
        /// Complete campaign.
        complete_campaign: bool,
    },
    /// Campaign complete.
    CampaignComplete {
        /// Campaign name.
        campaign: String,
    },
}

fn read_transition_decision(reader: SaveReader) -> Result<UnifiedTransitionDecision, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "stay" => Ok(UnifiedTransitionDecision::Stay {
            blocked: bounded_list(reader.field("blocked"), |item| item.string())?,
        }),
        "round" => {
            let winner = reader.field("winner");
            Ok(UnifiedTransitionDecision::Round {
                winner: if winner.value == Some(&SaveJson::Null) {
                    None
                } else {
                    Some(namespaced(winner)?)
                },
            })
        }
        "travel" => Ok(UnifiedTransitionDecision::Travel {
            map: reader.field("map").string()?,
            spawn_point: reader.field("spawnPoint").string()?,
            complete_campaign: reader.field("completeCampaign").boolean()?,
        }),
        "campaign-complete" => Ok(UnifiedTransitionDecision::CampaignComplete {
            campaign: reader.field("campaign").string()?,
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_transition_decision(decision: &UnifiedTransitionDecision) -> SaveJson {
    match decision {
        UnifiedTransitionDecision::Stay { blocked } => obj(vec![
            ("kind", json_str("stay")),
            ("blocked", arr(blocked.iter().map(|exit| json_str(exit)).collect())),
        ]),
        UnifiedTransitionDecision::Round { winner } => obj(vec![
            ("kind", json_str("round")),
            (
                "winner",
                winner.as_ref().map_or(SaveJson::Null, |winner| json_str(winner)),
            ),
        ]),
        UnifiedTransitionDecision::Travel {
            map,
            spawn_point,
            complete_campaign,
        } => obj(vec![
            ("kind", json_str("travel")),
            ("map", json_str(map)),
            ("spawnPoint", json_str(spawn_point)),
            ("completeCampaign", boolean(*complete_campaign)),
        ]),
        UnifiedTransitionDecision::CampaignComplete { campaign } => obj(vec![
            ("kind", json_str("campaign-complete")),
            ("campaign", json_str(campaign)),
        ]),
    }
}

/// Simulation event payload.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum UnifiedSimulationPayload {
    /// Sound.
    Sound {
        /// Resource id.
        resource: String,
        /// Actor handle.
        actor: Q1EffectActor,
        /// Origin.
        origin: Vec3,
        /// Channel.
        channel: f64,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
    },
    /// Damage.
    Damage {
        /// Outcome.
        outcome: DamageOutcome,
    },
    /// Transition.
    Transition {
        /// Decision.
        decision: UnifiedTransitionDecision,
    },
    /// Message.
    Message {
        /// Event.
        event: UnifiedNetworkEvent,
        /// Source presentation sequence.
        source_presentation_sequence: Option<f64>,
    },
}

fn read_simulation_payload(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedSimulationPayload, WorldError> {
    match reader.field("kind").string()?.as_str() {
        "sound" => Ok(UnifiedSimulationPayload::Sound {
            resource: read_resource(reader.field("resource"), identity)?,
            actor: read_effect_actor(reader.field("actor"), identity)?,
            origin: read_vector(reader.field("origin"))?,
            channel: reader.field("channel").finite()?,
            volume: reader.field("volume").finite()?,
            attenuation: reader.field("attenuation").finite()?,
        }),
        "damage" => Ok(UnifiedSimulationPayload::Damage {
            outcome: read_damage_outcome(reader.field("outcome"), identity)?,
        }),
        "transition" => Ok(UnifiedSimulationPayload::Transition {
            decision: read_transition_decision(reader.field("decision"))?,
        }),
        "message" => {
            let sequence = reader.field("sourcePresentationSequence");
            Ok(UnifiedSimulationPayload::Message {
                event: read_network_event(reader.field("event"), identity)?,
                source_presentation_sequence: if sequence.value.is_none() {
                    None
                } else {
                    Some(sequence.finite()?)
                },
            })
        }
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_simulation_payload(payload: &UnifiedSimulationPayload) -> Result<SaveJson, UnifiedEventError> {
    match payload {
        UnifiedSimulationPayload::Sound {
            resource,
            actor,
            origin,
            channel,
            volume,
            attenuation,
        } => Ok(obj(vec![
            ("kind", json_str("sound")),
            ("resource", write_resource(resource)?),
            ("actor", write_effect_actor(actor)),
            ("origin", write_vector(*origin)),
            ("channel", num(*channel)),
            ("volume", num(*volume)),
            ("attenuation", num(*attenuation)),
        ])),
        UnifiedSimulationPayload::Damage { outcome } => Ok(obj(vec![
            ("kind", json_str("damage")),
            ("outcome", write_damage_outcome(outcome)),
        ])),
        UnifiedSimulationPayload::Transition { decision } => Ok(obj(vec![
            ("kind", json_str("transition")),
            ("decision", write_transition_decision(decision)),
        ])),
        UnifiedSimulationPayload::Message {
            event,
            source_presentation_sequence,
        } => {
            let mut members = vec![("kind", json_str("message")), ("event", write_network_event(event))];
            if let Some(sequence) = source_presentation_sequence {
                members.push(("sourcePresentationSequence", num(*sequence)));
            }
            Ok(obj(members))
        }
    }
}

/// Simulation event (donor `SimulationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSimulationEvent {
    /// Sequence.
    pub sequence: f64,
    /// Time.
    pub time: SourceTime,
    /// Audience.
    pub audience: UnifiedEventAudience,
    /// Payload.
    pub payload: UnifiedSimulationPayload,
}

/// Write a simulation event value (donor `writeUnifiedSimulationEvent`).
pub fn write_unified_simulation_event(event: &UnifiedSimulationEvent) -> Result<SaveJson, UnifiedEventError> {
    Ok(obj(vec![
        ("sequence", num(event.sequence)),
        ("time", write_time(event.time)),
        ("audience", write_audience(&event.audience)),
        ("payload", write_simulation_payload(&event.payload)?),
    ]))
}

/// Read a simulation event value (donor `readUnifiedSimulationEvent`).
pub fn read_unified_simulation_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedSimulationEvent, UnifiedEventError> {
    Ok(UnifiedSimulationEvent {
        sequence: reader.field("sequence").finite()?,
        time: read_time(reader.field("time"))?,
        audience: read_audience(reader.field("audience"), identity)?,
        payload: read_simulation_payload(reader.field("payload"), identity)?,
    })
}

/// Debug-graph sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugGraphSample {
    /// Value.
    pub value: f64,
    /// Color.
    pub color: i64,
}

/// Presentation-owner event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLifecycleEvent {
    /// Component retired.
    Retired {
        /// Owner.
        owner: qa_content::contract::PresentationOwner,
    },
    /// Component refreshed.
    Refreshed {
        /// Owner.
        owner: qa_content::contract::PresentationOwner,
    },
}

fn read_owner(reader: SaveReader) -> Result<qa_content::contract::PresentationOwner, WorldError> {
    let generation = reader.field("generation").integer(1)?;
    let provider = namespaced(reader.field("provider"))?;
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    if namespace.is_empty() || name.is_empty() {
        return Err(reader.field("provider").fail("invalid provider reference"));
    }
    Ok(qa_content::contract::PresentationOwner {
        provider: qa_core::identity::ProviderId::new(namespace, name),
        generation: generation as u64,
    })
}

fn write_owner(owner: &qa_content::contract::PresentationOwner) -> SaveJson {
    obj(vec![
        (
            "provider",
            json_str(&format!("{}:{}", owner.provider.namespace, owner.provider.name)),
        ),
        ("generation", int(owner.generation as i64)),
    ])
}

/// View-reset reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewResetReason {
    /// Server travel.
    ServerTravel,
}

/// Q1 client metadata value.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1MetadataValue {
    /// Text value.
    Text(String),
    /// Numeric value.
    Number(i64),
}

/// Q1 client metadata event body.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ClientMetadataEvent {
    /// Update kind.
    pub kind: Q1MetadataKind,
    /// Slot.
    pub slot: i64,
    /// Value.
    pub value: Q1MetadataValue,
}

/// Q1 client metadata kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1MetadataKind {
    /// Name.
    Name,
    /// Social.
    Social,
    /// Player info.
    PlayerInfo,
    /// Colors.
    Colors,
    /// Frags.
    Frags,
    /// Ping.
    Ping,
}

fn read_client_metadata(
    reader: SaveReader,
    _identity: &dyn UnifiedIdentityDecoder,
) -> Result<Q1ClientMetadataEvent, WorldError> {
    let kind = reader.field("kind").string()?;
    let slot = reader.field("slot").integer(0)?;
    if slot > 255 {
        return Err(reader.fail("event slot exceeds its range"));
    }
    match kind.as_str() {
        "name-change" => Ok(Q1ClientMetadataEvent {
            kind: Q1MetadataKind::Name,
            slot,
            value: Q1MetadataValue::Text(reader.field("value").string()?),
        }),
        "social" => Ok(Q1ClientMetadataEvent {
            kind: Q1MetadataKind::Social,
            slot,
            value: Q1MetadataValue::Text(reader.field("value").string()?),
        }),
        "player-info" => Ok(Q1ClientMetadataEvent {
            kind: Q1MetadataKind::PlayerInfo,
            slot,
            value: Q1MetadataValue::Text(reader.field("value").string()?),
        }),
        "colors" => Ok(Q1ClientMetadataEvent {
            kind: Q1MetadataKind::Colors,
            slot,
            value: Q1MetadataValue::Number(reader.field("value").integer(0)?),
        }),
        "frags" => Ok(Q1ClientMetadataEvent {
            kind: Q1MetadataKind::Frags,
            slot,
            value: Q1MetadataValue::Number(reader.field("value").integer(0)?),
        }),
        "ping" => Ok(Q1ClientMetadataEvent {
            kind: Q1MetadataKind::Ping,
            slot,
            value: Q1MetadataValue::Number(reader.field("value").integer(0)?),
        }),
        _ => Err(reader.fail("unknown event variant")),
    }
}

fn write_client_metadata(event: &Q1ClientMetadataEvent) -> SaveJson {
    let (kind, value) = match &event.value {
        Q1MetadataValue::Text(text) => (
            match event.kind {
                Q1MetadataKind::Name => "name-change",
                Q1MetadataKind::Social => "social",
                _ => "player-info",
            },
            json_str(text),
        ),
        Q1MetadataValue::Number(number) => (
            match event.kind {
                Q1MetadataKind::Colors => "colors",
                Q1MetadataKind::Frags => "frags",
                _ => "ping",
            },
            int(*number),
        ),
    };
    obj(vec![
        ("kind", json_str(kind)),
        ("slot", int(event.slot)),
        ("value", value),
    ])
}

/// Presentation event body.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum UnifiedPresentationBody {
    /// Debug-graph sample.
    DebugGraph {
        /// Sample.
        event: DebugGraphSample,
    },
    /// Owner lifecycle.
    PresentationOwner {
        /// Event.
        event: OwnerLifecycleEvent,
    },
    /// Q1 event.
    Q1 {
        /// Event.
        event: Q1Event,
    },
    /// Q1 fog transition.
    Q1Fog {
        /// Event.
        event: Q1FogEvent,
    },
    /// Q1 skybox.
    Q1Sky {
        /// Skybox name.
        name: String,
    },
    /// Q1 client metadata.
    Q1Client {
        /// Event.
        event: Q1ClientMetadataEvent,
    },
    /// Q1 session.
    Q1Session {
        /// Event.
        event: Q1IntermissionResult,
    },
    /// Music.
    Music {
        /// Event.
        event: UnifiedMusicEvent,
    },
    /// Q1 composition.
    Q1Composition {
        /// Event.
        event: Q1CompositionEvent,
    },
    /// Q1 level result.
    Q1Level {
        /// Event.
        event: Q1IntermissionResult,
    },
    /// Q2 event.
    Q2 {
        /// Event.
        event: Q2PresentationEvent,
    },
    /// Q2 weapon event.
    Q2Weapon {
        /// Event.
        event: Q2WeaponEvent,
    },
    /// View reset.
    ViewReset {
        /// Reason.
        reason: ViewResetReason,
        /// Actor handle.
        actor: ActorId,
        /// Angles.
        angles: Vec3,
    },
    /// Q2 composition.
    Q2Composition {
        /// Event.
        event: Q2CompositionEvent,
    },
    /// Q2 rerelease.
    Q2Rerelease {
        /// Event.
        event: Q2RereleaseEvent,
    },
    /// Q2 player.
    Q2Player {
        /// Event.
        event: Q2PlayerEvent,
    },
    /// Q3 character.
    Q3Character {
        /// Event.
        event: Q3CharacterPresentationEvent,
    },
    /// Q3 ballistics.
    Q3Ballistics {
        /// Event.
        event: Q3SharedBallisticEvent,
    },
    /// Q3 source.
    Q3Source {
        /// Event.
        event: Q3SourceEvent,
    },
}

/// Presentation event (donor `SourcePresentationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPresentationEvent {
    /// Body.
    pub body: UnifiedPresentationBody,
    /// Content identity.
    pub content: String,
    /// Sequence.
    pub sequence: f64,
    /// Time in seconds.
    pub seconds: f64,
    /// Source entity (absent, null, or a number).
    pub source_entity: Option<Option<f64>>,
    /// Recipient.
    pub recipient: Option<ActorId>,
    /// Owner.
    pub owner: Option<qa_content::contract::PresentationOwner>,
}

fn read_source_entity(reader: SaveReader) -> Result<Option<Option<f64>>, WorldError> {
    let field = reader.field("sourceEntity");
    if field.value.is_none() {
        Ok(None)
    } else if field.value == Some(&SaveJson::Null) {
        Ok(Some(None))
    } else {
        Ok(Some(Some(field.finite()?)))
    }
}

fn read_recipient(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<Option<ActorId>, WorldError> {
    let field = reader.field("recipient");
    if field.value.is_none() {
        Ok(None)
    } else {
        read_actor(field, identity).map(Some)
    }
}

fn write_source_entity(source: &Option<Option<f64>>) -> Option<SaveJson> {
    source.map(|entity| entity.map_or(SaveJson::Null, num))
}

fn read_source_presentation_event(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedPresentationEvent, UnifiedEventError> {
    let kind = reader.field("kind").string()?;
    let content = read_content(reader.field("content"))?;
    let sequence = reader.field("sequence").finite()?;
    let seconds = reader.field("seconds").finite()?;
    let owner_field = reader.field("owner");
    let owner = if owner_field.value.is_none() {
        None
    } else {
        Some(read_owner(owner_field)?)
    };
    let source_entity = read_source_entity(reader.clone())?;
    let recipient = read_recipient(reader.clone(), identity)?;
    let body = match kind.as_str() {
        "debug-graph" => {
            let event = reader.field("event");
            event.field("kind").literal_str("sample")?;
            UnifiedPresentationBody::DebugGraph {
                event: DebugGraphSample {
                    value: event.field("value").finite()?,
                    color: event.field("color").integer(i64::MIN)?,
                },
            }
        }
        "presentation-owner" => {
            let event = reader.field("event");
            let owner_kind = event.field("kind").string()?;
            UnifiedPresentationBody::PresentationOwner {
                event: match owner_kind.as_str() {
                    "retired" => OwnerLifecycleEvent::Retired {
                        owner: read_owner(event.field("owner"))?,
                    },
                    "refreshed" => OwnerLifecycleEvent::Refreshed {
                        owner: read_owner(event.field("owner"))?,
                    },
                    _ => return Err(reader.fail("unknown event variant").into()),
                },
            }
        }
        "q1" => UnifiedPresentationBody::Q1 {
            event: read_q1_event(reader.field("event"), identity)?,
        },
        "q1-fog" => {
            let event = reader.field("event");
            event.field("kind").literal_str("transition")?;
            UnifiedPresentationBody::Q1Fog {
                event: Q1FogEvent {
                    player: read_opt_actor(event.field("player"), identity)?,
                    transition: read_q1_fog_transition(event.field("transition"))?,
                    sky_factor: event.field("skyFactor").finite()?,
                },
            }
        }
        "q1-sky" => {
            let event = reader.field("event");
            event.field("kind").literal_str("skybox")?;
            UnifiedPresentationBody::Q1Sky {
                name: event.field("name").string()?,
            }
        }
        "q1-client" => UnifiedPresentationBody::Q1Client {
            event: read_client_metadata(reader.field("event"), identity)?,
        },
        "q1-session" => UnifiedPresentationBody::Q1Session {
            event: read_q1_intermission_result(reader.field("event"))?,
        },
        "music" => UnifiedPresentationBody::Music {
            event: read_music_event(reader.field("event"))?,
        },
        "q1-composition" => UnifiedPresentationBody::Q1Composition {
            event: read_q1_composition_event(reader.field("event"), identity)?,
        },
        "q1-level" => UnifiedPresentationBody::Q1Level {
            event: read_q1_intermission_result(reader.field("event"))?,
        },
        "q2" => UnifiedPresentationBody::Q2 {
            event: read_q2_presentation_event(reader.field("event"), identity)?,
        },
        "q2-weapon" => UnifiedPresentationBody::Q2Weapon {
            event: read_q2_weapon_event(reader.field("event"), identity)?,
        },
        "view-reset" => {
            let reason = reader.field("reason").choice_str(&["server-travel"])?;
            debug_assert_eq!(reason, "server-travel");
            UnifiedPresentationBody::ViewReset {
                reason: ViewResetReason::ServerTravel,
                actor: read_actor(reader.field("actor"), identity)?,
                angles: read_vector(reader.field("angles"))?,
            }
        }
        "q2-composition" => UnifiedPresentationBody::Q2Composition {
            event: read_q2_composition_event(reader.field("event"), identity)?,
        },
        "q2-rerelease" => UnifiedPresentationBody::Q2Rerelease {
            event: read_q2_rerelease_event(reader.field("event"), identity)?,
        },
        "q2-player" => UnifiedPresentationBody::Q2Player {
            event: read_q2_player_event(reader.field("event"), identity)?,
        },
        "q3-character" => UnifiedPresentationBody::Q3Character {
            event: read_q3_character_event(reader.field("event"), identity)?,
        },
        "q3-ballistics" => UnifiedPresentationBody::Q3Ballistics {
            event: read_q3_ballistic_event(reader.field("event"), identity)?,
        },
        "q3-source" => UnifiedPresentationBody::Q3Source {
            event: read_q3_source_event(reader.field("event"), identity)?,
        },
        _ => return Err(reader.fail("unknown event variant").into()),
    };
    Ok(UnifiedPresentationEvent {
        body,
        content,
        sequence,
        seconds,
        source_entity,
        recipient,
        owner,
    })
}

fn write_source_presentation_event(event: &UnifiedPresentationEvent) -> Result<SaveJson, UnifiedEventError> {
    let mut members: Vec<(&str, SaveJson)> = Vec::new();
    let (kind, body): (&str, SaveJson) = match &event.body {
        UnifiedPresentationBody::DebugGraph { event } => (
            "debug-graph",
            obj(vec![
                ("kind", json_str("sample")),
                ("value", num(event.value)),
                ("color", int(event.color)),
            ]),
        ),
        UnifiedPresentationBody::PresentationOwner { event } => (
            "presentation-owner",
            match event {
                OwnerLifecycleEvent::Retired { owner } => {
                    obj(vec![("kind", json_str("retired")), ("owner", write_owner(owner))])
                }
                OwnerLifecycleEvent::Refreshed { owner } => {
                    obj(vec![("kind", json_str("refreshed")), ("owner", write_owner(owner))])
                }
            },
        ),
        UnifiedPresentationBody::Q1 { event } => ("q1", write_q1_event(event)),
        UnifiedPresentationBody::Q1Fog { event } => (
            "q1-fog",
            obj(vec![
                ("kind", json_str("transition")),
                ("player", write_opt_actor(event.player.as_ref())),
                ("transition", write_q1_fog_transition(&event.transition)),
                ("skyFactor", num(event.sky_factor)),
            ]),
        ),
        UnifiedPresentationBody::Q1Sky { name } => (
            "q1-sky",
            obj(vec![("kind", json_str("skybox")), ("name", json_str(name))]),
        ),
        UnifiedPresentationBody::Q1Client { event } => ("q1-client", write_client_metadata(event)),
        UnifiedPresentationBody::Q1Session { event } => ("q1-session", write_q1_intermission_result(event)),
        UnifiedPresentationBody::Music { event } => ("music", write_music_event(event)),
        UnifiedPresentationBody::Q1Composition { event } => ("q1-composition", write_q1_composition_event(event)),
        UnifiedPresentationBody::Q1Level { event } => ("q1-level", write_q1_intermission_result(event)),
        UnifiedPresentationBody::Q2 { event } => ("q2", write_q2_presentation_event(event)),
        UnifiedPresentationBody::Q2Weapon { event } => ("q2-weapon", write_q2_weapon_event(event)),
        UnifiedPresentationBody::ViewReset {
            reason: _,
            actor,
            angles,
        } => {
            members.push(("kind", json_str("view-reset")));
            members.push(("reason", json_str("server-travel")));
            members.push(("actor", wire_actor(actor)));
            members.push(("angles", write_vector(*angles)));
            ("view-reset", SaveJson::Null)
        }
        UnifiedPresentationBody::Q2Composition { event } => ("q2-composition", write_q2_composition_event(event)),
        UnifiedPresentationBody::Q2Rerelease { event } => ("q2-rerelease", write_q2_rerelease_event(event)),
        UnifiedPresentationBody::Q2Player { event } => ("q2-player", write_q2_player_event(event)),
        UnifiedPresentationBody::Q3Character { event } => ("q3-character", write_q3_character_event(event)),
        UnifiedPresentationBody::Q3Ballistics { event } => ("q3-ballistics", write_q3_ballistic_event(event)),
        UnifiedPresentationBody::Q3Source { event } => ("q3-source", write_q3_source_event(event)?),
    };
    if !matches!(event.body, UnifiedPresentationBody::ViewReset { .. }) {
        members.push(("kind", json_str(kind)));
        members.push(("event", body));
    }
    members.push(("sequence", num(event.sequence)));
    members.push(("content", json_str(&event.content)));
    members.push(("seconds", num(event.seconds)));
    if !matches!(
        event.body,
        UnifiedPresentationBody::DebugGraph { .. } | UnifiedPresentationBody::PresentationOwner { .. }
    ) {
        if let Some(entity) = write_source_entity(&event.source_entity) {
            members.push(("sourceEntity", entity));
        }
    }
    if let Some(recipient) = &event.recipient {
        members.push(("recipient", wire_actor(recipient)));
    }
    if let Some(owner) = &event.owner {
        members.push(("owner", write_owner(owner)));
    }
    Ok(obj(members))
}

/// Encode presentation events (donor `encodeUnifiedPresentationEvents`).
pub fn encode_unified_presentation_events(events: &[UnifiedPresentationEvent]) -> Result<Vec<u8>, UnifiedEventError> {
    if events.len() > MAX_EVENT_LIST {
        return Err(UnifiedEventError::Limit(
            "Unified presentation events exceed count limit".to_string(),
        ));
    }
    let mut encoded = Vec::with_capacity(events.len());
    for event in events {
        encoded.push(write_source_presentation_event(event)?);
    }
    let bytes = encode_checkpoint_value(&arr(encoded));
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(UnifiedEventError::Limit(
            "Unified presentation events exceed byte limit".to_string(),
        ));
    }
    Ok(bytes)
}

/// Decode presentation events (donor `decodeUnifiedPresentationEvents`).
pub fn decode_unified_presentation_events(
    bytes: &[u8],
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<Vec<UnifiedPresentationEvent>, UnifiedEventError> {
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(UnifiedEventError::Limit(
            "Unified presentation events exceed byte limit".to_string(),
        ));
    }
    let value = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::new(&value);
    let events = bounded_list(reader, |event| read_source_presentation_event(event, identity))?;
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, SessionId};

    use super::super::unified_types::UnifiedIdentityDecoder as Decoder;
    use qa_core::identity::{ClientId, SeatId};

    struct Ledger {
        owner: IdentityOwner,
    }

    impl Decoder for Ledger {
        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
        fn actor(&self, slot: u32, generation: u32) -> ActorId {
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

    fn ledger() -> Ledger {
        Ledger {
            owner: IdentityOwner::create("events").unwrap(),
        }
    }

    fn vec_json(x: f64, y: f64, z: f64) -> SaveJson {
        obj(vec![("x", num(x)), ("y", num(y)), ("z", num(z))])
    }

    fn actor_json(slot: i64, generation: i64) -> SaveJson {
        obj(vec![("slot", num(slot as f64)), ("generation", num(generation as f64))])
    }

    #[test]
    fn presentation_batch_round_trips() {
        let ledger = ledger();
        let actor = ledger.actor(1, 0);
        let events = vec![
            UnifiedPresentationEvent {
                body: UnifiedPresentationBody::DebugGraph {
                    event: DebugGraphSample { value: 3.5, color: 7 },
                },
                content: "q1:classic:base:1".to_string(),
                sequence: 1.0,
                seconds: 0.5,
                source_entity: None,
                recipient: None,
                owner: None,
            },
            UnifiedPresentationEvent {
                body: UnifiedPresentationBody::Q1 {
                    event: Q1Event::MonsterTotal { total: 42.0 },
                },
                content: "q1:classic:base:1".to_string(),
                sequence: 2.0,
                seconds: 0.6,
                source_entity: Some(None),
                recipient: Some(actor.clone()),
                owner: None,
            },
            UnifiedPresentationEvent {
                body: UnifiedPresentationBody::ViewReset {
                    reason: ViewResetReason::ServerTravel,
                    actor: actor.clone(),
                    angles: Vec3 {
                        x: 0.0,
                        y: 90.0,
                        z: 0.0,
                    },
                },
                content: "q2:classic:base:1".to_string(),
                sequence: 3.0,
                seconds: 0.7,
                source_entity: Some(Some(9.0)),
                recipient: None,
                owner: None,
            },
        ];
        let encoded = encode_unified_presentation_events(&events).unwrap();
        let decoded = decode_unified_presentation_events(&encoded, &ledger).unwrap();
        assert_eq!(decoded, events);
    }

    #[test]
    fn batch_rejects_oversize_payload() {
        let ledger = ledger();
        assert!(decode_unified_presentation_events(&vec![0u8; MAX_EVENT_BYTES + 1], &ledger).is_err());
    }

    #[test]
    fn sound_channel_accepts_numbers_and_names() {
        let named = json_str("weapon");
        let reader = SaveReader::new(&named);
        assert_eq!(
            read_sound_channel(reader).unwrap(),
            Q1SoundChannel::Named(Q1NamedChannel::Weapon)
        );
        let numbered = int(5);
        let reader = SaveReader::new(&numbered);
        assert_eq!(read_sound_channel(reader).unwrap(), Q1SoundChannel::Number(5));
        let bad = int(4);
        let reader = SaveReader::new(&bad);
        assert!(read_sound_channel(reader).is_err());
    }

    #[test]
    fn q1_sound_origin_is_optional() {
        let ledger = ledger();
        let encoded = obj(vec![
            ("kind", json_str("sound")),
            ("actor", actor_json(1, 0)),
            ("path", json_str("weapons/shotgun.wav")),
            ("channel", json_str("auto")),
            ("attenuation", num(1.0)),
            ("volume", num(1.0)),
        ]);
        let reader = SaveReader::new(&encoded);
        let event = read_q1_event(reader, &ledger).unwrap();
        assert!(matches!(event, Q1Event::Sound { origin: None, .. }));
    }

    #[test]
    fn simulation_message_round_trips() {
        let ledger = ledger();
        let event = UnifiedSimulationEvent {
            sequence: 4.0,
            time: SourceTime::Milliseconds(100),
            audience: UnifiedEventAudience::World,
            payload: UnifiedSimulationPayload::Message {
                event: UnifiedNetworkEvent::Disconnect {
                    reason: "quit".to_string(),
                },
                source_presentation_sequence: Some(2.0),
            },
        };
        let encoded = write_unified_simulation_event(&event).unwrap();
        let reader = SaveReader::new(&encoded);
        let decoded = read_unified_simulation_event(reader, &ledger).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn transition_travel_round_trips() {
        let ledger = ledger();
        let event = UnifiedSimulationEvent {
            sequence: 5.0,
            time: SourceTime::Seconds(1.5),
            audience: UnifiedEventAudience::Seat(ledger.seat(0)),
            payload: UnifiedSimulationPayload::Transition {
                decision: UnifiedTransitionDecision::Travel {
                    map: "e1m1".to_string(),
                    spawn_point: "start".to_string(),
                    complete_campaign: false,
                },
            },
        };
        let encoded = write_unified_simulation_event(&event).unwrap();
        let reader = SaveReader::new(&encoded);
        let decoded = read_unified_simulation_event(reader, &ledger).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn vec_helper_builds_vectors() {
        let _ = vec_json(1.0, 2.0, 3.0);
    }
}
