//! Retained presentation events with owner lifecycle and media queues.
//!
//! Port of donor `src/app/bootstrap/presentation-state.ts`
//! (`LocalPresentationMedia`, `PresentationState`). Ownership, resource references,
//! save readers, vectors, shader names, and the Q1 fog shapes are the ported contract,
//! content, persistence, material, and time helpers; the Q1 fog simulation runs
//! directly on the canonical
//! [`SimulationQ1Fog`](super::simulation::q1_fog::SimulationQ1Fog), with boundary
//! converters between the local generic events and the canonical
//! [`Q1FogContext`](super::simulation::q1_fog::Q1FogContext) plus canonical addon
//! events. The event unions (`SimulationPresentationEvent`, `SourcePresentationEvent`
//! from `./simulation/types.ts`, out of scope) are mirrored locally as
//! [`SimulationPresentationEvent`] with a generic foreign payload `F` for variants this
//! module carries but never inspects; the media request mirrors the contract request
//! whose fields are private. Reference-identity checks on queued media use shared
//! ownership ([`Rc`] pointer equality). Documented folds: absent and null source
//! entities both restore to [`None`]; save keys are recomputed on restore exactly like
//! the donor; addressed Quake II lightstyles apply live but rebuild broadcast-only,
//! exactly like the donor.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use super::simulation::q1_fog::{Q1FogContext, SimulationQ1Fog, SimulationQ1FogOptions};
use qa_client::materials::fog::Q1FogTransition;
use qa_client::materials::material::{normalize_shader_name, strip_shader_extension};
use qa_content::contract::{
    presentation_owner_key, same_presentation_owner, ContentId, PresentationOwner, ResolvedResourceReference,
    ResourceId,
};
use qa_content::value::{arr, boolean, int, namespaced, num, obj, read_vector, str, SaveJson, SaveReader, ValueError};
use qa_core::identity::{ActorId, ProviderId, SavedActorId};
use qa_core::math::{vec3, Vec3};
use qa_core::time::SourceTime;
use qa_world::save::shared::validate_content_id;
use thiserror::Error;

/// Maximum safe presentation owner generation (donor safe integer).
const MAX_OWNER_GENERATION: u64 = 9_007_199_254_740_991;

/// Failure of a presentation state operation, with donor messages.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresentationStateError {
    /// A presentation owner is already active.
    #[error("Presentation owner is already active: {0}")]
    OwnerActive(String),
    /// A restored presentation owner differs.
    #[error("Restored presentation owner differs: {0}")]
    OwnerDiffers(String),
    /// A legacy save hides persistent presentation without ownership.
    #[error("Legacy save has persistent presentation in component {0}'s content without recorded ownership; primary and component output cannot be distinguished")]
    LegacyOwnership(String),
    /// The presentation owner generation is exhausted.
    #[error("Presentation owner generation exhausted")]
    GenerationExhausted,
    /// A presentation owner is retired.
    #[error("Presentation owner retired: {0}")]
    OwnerRetired(String),
    /// A saved presentation owner was not restored.
    #[error("Saved presentation owner was not restored: {0}")]
    OwnerUnrestored(String),
    /// A presentation owner is not active.
    #[error("Presentation owner is not active: {0}")]
    OwnerInactive(String),
    /// A component source emitted an owner lifecycle event.
    #[error("Component source cannot emit owner lifecycle events")]
    OwnerLifecycle,
    /// Local media requires its active owner and content.
    #[error("Local media requires its active presentation owner and content")]
    MediaOwnership,
    /// Shader replay requires a local material cue.
    #[error("Shader replay requires a local material cue")]
    ShaderReplay,
    /// A save requires completed local media.
    #[error("Save requires completed local presentation media")]
    MediaPending,
    /// A registered resource path does not match its request.
    #[error("Registered resource path does not match its source request")]
    ResourcePath,
    /// A replicated presentation owner changed identity.
    #[error("Replicated presentation owner changed identity")]
    ReplicatedIdentity,
    /// A replicated presentation activation moved backward.
    #[error("Replicated presentation activation moved backward")]
    ReplicatedBackward,
    /// An event cannot persist.
    #[error("Unsupported persistent source event")]
    UnsupportedPersistent,
    /// A save requires consumed source output.
    #[error("Save requires consumed source output")]
    OutputPending,
    /// Save value failure.
    #[error(transparent)]
    Value(#[from] ValueError),
    /// Q1 fog construction or restore failed.
    #[error("Q1 fog failed: {0}")]
    Fog(String),
}

/// Owner lifecycle (donor `presentation-owner` event).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerLifecycle {
    /// Owner retired.
    Retired {
        /// Retired owner.
        owner: PresentationOwner,
    },
    /// Owner refreshed.
    Refreshed {
        /// Refreshed owner.
        owner: PresentationOwner,
    },
}

/// View reset reason (donor `view-reset` reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewResetReason {
    /// Spawn.
    Spawn,
    /// Teleport.
    Teleport,
    /// Freeze.
    Freeze,
    /// Source.
    Source,
}

/// Quake music event (donor `music` event).
#[derive(Debug, Clone, PartialEq)]
pub enum MusicEvent {
    /// CD track.
    CdTrack {
        /// Track number.
        track: i32,
    },
    /// Pause.
    Pause {
        /// Whether paused.
        paused: bool,
    },
}

/// Quake client metadata (donor `Q1ClientMetadataEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1ClientMetadata {
    /// Name.
    Name {
        /// Client slot.
        slot: i32,
        /// Value.
        value: String,
    },
    /// Social.
    Social {
        /// Client slot.
        slot: i32,
        /// Value.
        value: String,
    },
    /// Player info.
    PlayerInfo {
        /// Client slot.
        slot: i32,
        /// Value.
        value: String,
    },
    /// Colors.
    Colors {
        /// Client slot.
        slot: i32,
        /// Value.
        value: i32,
    },
    /// Frags.
    Frags {
        /// Client slot.
        slot: i32,
        /// Value.
        value: i32,
    },
    /// Ping.
    Ping {
        /// Client slot.
        slot: i32,
        /// Value.
        value: i32,
    },
}

impl Q1ClientMetadata {
    /// Metadata kind.
    fn kind(&self) -> &'static str {
        match self {
            Q1ClientMetadata::Name { .. } => "name",
            Q1ClientMetadata::Social { .. } => "social",
            Q1ClientMetadata::PlayerInfo { .. } => "player-info",
            Q1ClientMetadata::Colors { .. } => "colors",
            Q1ClientMetadata::Frags { .. } => "frags",
            Q1ClientMetadata::Ping { .. } => "ping",
        }
    }

    /// Client slot.
    fn slot(&self) -> i32 {
        match self {
            Q1ClientMetadata::Name { slot, .. }
            | Q1ClientMetadata::Social { slot, .. }
            | Q1ClientMetadata::PlayerInfo { slot, .. }
            | Q1ClientMetadata::Colors { slot, .. }
            | Q1ClientMetadata::Frags { slot, .. }
            | Q1ClientMetadata::Ping { slot, .. } => *slot,
        }
    }
}

/// Quake event (donor `Q1Event` subset with foreign carriage).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1PresentationEvent<F> {
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
    /// Finale.
    Finale {
        /// Text.
        text: String,
        /// Stage.
        stage: i32,
    },
    /// Lightstyle.
    Lightstyle {
        /// Style number.
        style: i32,
        /// Pattern.
        pattern: String,
    },
    /// Any other Quake event, carried opaquely.
    Other {
        /// Event kind.
        kind: String,
        /// Event actor, when the donor shape carries one.
        actor: Option<ActorId>,
        /// Opaque payload.
        payload: F,
    },
}

impl<F> Q1PresentationEvent<F> {
    /// Event actor for source entity resolution (donor `"actor" in event`).
    fn actor(&self) -> Option<&ActorId> {
        match self {
            Q1PresentationEvent::Other { actor, .. } => actor.as_ref(),
            _ => None,
        }
    }
}

/// Quake intermission event (donor `Q1IntermissionResult` subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1LevelEvent<F> {
    /// Finale.
    Finale {
        /// Text.
        text: String,
        /// Track number.
        track: i32,
    },
    /// Any other intermission event, carried opaquely.
    Other {
        /// Event kind.
        kind: String,
        /// Opaque payload.
        payload: F,
    },
}

/// Quake addon fog fields (donor fog `Q1AddonEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FogFields {
    /// Fog player.
    pub player: Option<ActorId>,
    /// Density.
    pub density: f64,
    /// Color.
    pub color: Vec3,
    /// Sky factor.
    pub sky_factor: f64,
    /// Duration.
    pub duration: f64,
}

/// Quake addon event (donor `Q1AddonEvent` subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1AddonEvent<F> {
    /// Fog.
    Fog(Q1FogFields),
    /// Any other addon event, carried opaquely.
    Other {
        /// Event kind.
        kind: String,
        /// Opaque payload.
        payload: F,
    },
}

/// Quake composition event (donor `Q1CompositionEvent` subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1CompositionEvent<F> {
    /// Addon event.
    Addon(Q1AddonEvent<F>),
    /// Any other composition event, carried opaquely.
    Other {
        /// Event kind.
        kind: String,
        /// Event actor, when the donor shape carries one.
        actor: Option<ActorId>,
        /// Opaque payload.
        payload: F,
    },
}

impl<F> Q1CompositionEvent<F> {
    /// Event actor for source entity resolution (donor `"actor" in event`).
    fn actor(&self) -> Option<&ActorId> {
        match self {
            Q1CompositionEvent::Other { actor, .. } => actor.as_ref(),
            _ => None,
        }
    }
}

/// Quake II sound loop (donor `loop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2SoundLoop {
    /// Start.
    Start,
    /// Stop.
    Stop,
    /// Once.
    Once,
}

impl Q2SoundLoop {
    /// Donor loop text.
    fn as_str(&self) -> &'static str {
        match self {
            Q2SoundLoop::Start => "start",
            Q2SoundLoop::Stop => "stop",
            Q2SoundLoop::Once => "once",
        }
    }
}

/// Quake II presentation event (donor `Q2PresentationEvent` subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2PresentationEvent<F> {
    /// Sound.
    Sound {
        /// Sound actor.
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
        /// Whether reliable.
        reliable: bool,
        /// Loop mode.
        loop_mode: Q2SoundLoop,
        /// Loop owner.
        loop_owner: Option<ProviderId>,
    },
    /// Music.
    Music {
        /// Track.
        track: String,
    },
    /// Lightstyle.
    Lightstyle {
        /// Style number.
        style: i32,
        /// Pattern.
        pattern: String,
    },
    /// Any other Quake II event, carried opaquely.
    Other {
        /// Event kind.
        kind: String,
        /// Event actor, when the donor shape carries one.
        actor: Option<ActorId>,
        /// Opaque payload.
        payload: F,
    },
}

impl<F> Q2PresentationEvent<F> {
    /// Event actor for source entity resolution (donor `"actor" in event`).
    fn actor(&self) -> Option<&ActorId> {
        match self {
            Q2PresentationEvent::Sound { actor, .. } => actor.as_ref(),
            Q2PresentationEvent::Other { actor, .. } => actor.as_ref(),
            _ => None,
        }
    }
}

/// Quake II composition event (donor `Q2CompositionEvent`, carried opaquely with its
/// unwrapped actor).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CompositionEvent<F> {
    /// Event kind.
    pub kind: String,
    /// Unwrapped event actor, when the donor shape carries one.
    pub actor: Option<ActorId>,
    /// Opaque payload.
    pub payload: F,
}

/// Source presentation event (donor `SourcePresentationEvent` subset with foreign
/// carriage for the families this module never inspects).
#[derive(Debug, Clone, PartialEq)]
pub enum SourcePresentationEvent<F> {
    /// Owner lifecycle.
    PresentationOwner {
        /// Lifecycle.
        event: OwnerLifecycle,
    },
    /// Quake.
    Q1(Q1PresentationEvent<F>),
    /// Quake sky.
    Q1Sky {
        /// Skybox name.
        name: String,
    },
    /// Quake client metadata.
    Q1Client(Q1ClientMetadata),
    /// Quake intermission.
    Q1Level(Q1LevelEvent<F>),
    /// Quake composition.
    Q1Composition(Q1CompositionEvent<F>),
    /// Quake fog transition (canonical fog output).
    Q1Fog {
        /// Fog player.
        player: Option<ActorId>,
        /// Sky factor.
        sky_factor: f64,
        /// Fog transition capture.
        transition: Q1FogTransition,
    },
    /// Quake II.
    Q2(Q2PresentationEvent<F>),
    /// Quake II composition.
    Q2Composition(Q2CompositionEvent<F>),
    /// Music.
    Music(MusicEvent),
    /// View reset.
    ViewReset {
        /// Reason.
        reason: ViewResetReason,
        /// Actor.
        actor: ActorId,
        /// Angles.
        angles: Vec3,
    },
    /// Any other family, carried opaquely.
    Foreign {
        /// Family kind.
        kind: String,
        /// Event actor, when the donor shape carries one.
        actor: Option<ActorId>,
        /// Opaque payload.
        payload: F,
    },
}

impl<F> SourcePresentationEvent<F> {
    /// Family kind.
    fn kind(&self) -> &str {
        match self {
            SourcePresentationEvent::PresentationOwner { .. } => "presentation-owner",
            SourcePresentationEvent::Q1(_) => "q1",
            SourcePresentationEvent::Q1Sky { .. } => "q1-sky",
            SourcePresentationEvent::Q1Client(_) => "q1-client",
            SourcePresentationEvent::Q1Level(_) => "q1-level",
            SourcePresentationEvent::Q1Composition(_) => "q1-composition",
            SourcePresentationEvent::Q1Fog { .. } => "q1-fog",
            SourcePresentationEvent::Q2(_) => "q2",
            SourcePresentationEvent::Q2Composition(_) => "q2-composition",
            SourcePresentationEvent::Music(_) => "music",
            SourcePresentationEvent::ViewReset { .. } => "view-reset",
            SourcePresentationEvent::Foreign { kind, .. } => kind,
        }
    }

    /// Event actor for source entity resolution (donor `"actor" in event`, including
    /// the view-reset and composition unwraps).
    fn actor(&self) -> Option<&ActorId> {
        match self {
            SourcePresentationEvent::Q1(event) => event.actor(),
            SourcePresentationEvent::Q1Composition(event) => event.actor(),
            SourcePresentationEvent::Q2(event) => event.actor(),
            SourcePresentationEvent::Q2Composition(event) => event.actor.as_ref(),
            SourcePresentationEvent::ViewReset { actor, .. } => Some(actor),
            SourcePresentationEvent::Foreign { actor, .. } => actor.as_ref(),
            _ => None,
        }
    }
}

/// Simulation presentation event (donor `SimulationPresentationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct SimulationPresentationEvent<F> {
    /// Source event.
    pub source: SourcePresentationEvent<F>,
    /// Presenting owner.
    pub owner: Option<PresentationOwner>,
    /// Per-actor recipient.
    pub recipient: Option<ActorId>,
    /// Presentation sequence.
    pub sequence: i64,
    /// Content.
    pub content: ContentId,
    /// Seconds.
    pub seconds: f64,
    /// Source entity slot.
    pub source_entity: Option<i32>,
}

/// Local media request (donor `ComponentPresentationMediaRequest` mirror; the contract
/// fields are private).
#[derive(Debug, Clone, PartialEq)]
pub enum MediaRequest {
    /// Start music with an intro leading into a loop.
    Music {
        /// Intro track.
        intro: String,
        /// Loop track.
        loop_track: String,
    },
    /// Stop music.
    MusicStop,
    /// Remap a shader with a time offset.
    ShaderRemap {
        /// Original shader.
        original: String,
        /// Replacement shader.
        replacement: String,
        /// Time offset.
        time_offset: f64,
    },
}

/// Local presentation media (donor `LocalPresentationMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPresentationMedia {
    /// Media request.
    pub event: MediaRequest,
    /// Presenting owner.
    pub owner: Option<PresentationOwner>,
    /// Content.
    pub content: ContentId,
    /// Presentation sequence.
    pub sequence: i64,
    /// Seconds.
    pub seconds: f64,
}

/// Retained presentation (donor source-or-local union).
#[derive(Debug, Clone, PartialEq)]
pub enum RetainedPresentation<F> {
    /// Source event.
    Source(SimulationPresentationEvent<F>),
    /// Local media.
    Local(LocalPresentationMedia),
}

impl<F> RetainedPresentation<F> {
    /// Presenting owner.
    fn owner(&self) -> Option<&PresentationOwner> {
        match self {
            RetainedPresentation::Source(event) => event.owner.as_ref(),
            RetainedPresentation::Local(event) => event.owner.as_ref(),
        }
    }

    /// Presentation sequence.
    fn sequence(&self) -> i64 {
        match self {
            RetainedPresentation::Source(event) => event.sequence,
            RetainedPresentation::Local(event) => event.sequence,
        }
    }

    /// Content.
    fn content(&self) -> &ContentId {
        match self {
            RetainedPresentation::Source(event) => &event.content,
            RetainedPresentation::Local(event) => &event.content,
        }
    }
}

/// Scene light style (donor `SceneLightStyle`).
#[derive(Debug, Clone, PartialEq)]
pub enum SceneLightStyle {
    /// Quake style.
    Q1 {
        /// Style number.
        style: i32,
        /// Value.
        value: i32,
    },
    /// Quake II style.
    Q2 {
        /// Style number.
        style: i32,
        /// Color.
        rgb: Vec3,
        /// White.
        white: f64,
    },
}

/// Reference a saved actor id (donor `readSavedActor`).
fn read_saved_actor(reader: &SaveReader) -> Result<SavedActorId, ValueError> {
    Ok(SavedActorId {
        slot: u32::try_from(reader.field("slot").integer(0)?).unwrap_or(u32::MAX),
        generation: u32::try_from(reader.field("generation").integer(0)?).unwrap_or(u32::MAX),
    })
}

/// Write a saved actor id (donor `savedActorId`).
fn write_saved_actor(actor: &ActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot()))),
        ("generation", int(i64::from(actor.generation()))),
    ])
}

/// Write a provider id (donor namespaced identity).
fn write_provider(provider: &ProviderId) -> SaveJson {
    str(&format!("{}:{}", provider.namespace, provider.name))
}

/// Read a content id (donor `readContentId`).
fn read_content_id(reader: &SaveReader) -> Result<ContentId, PresentationStateError> {
    let value = reader.string()?;
    validate_content_id(&value).map_err(|_| reader.fail("expected a content identity"))?;
    Ok(ContentId(value))
}

/// Read a provider id (donor `namespaced`).
fn read_provider(reader: &SaveReader) -> Result<ProviderId, ValueError> {
    let value = namespaced(reader.clone())?;
    let (namespace, name) = value.split_once(':').unwrap_or(("", ""));
    Ok(ProviderId::new(namespace, name))
}

/// Write a vector (donor `{x, y, z}`).
fn write_vector(value: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
    ])
}

/// Format a float the way `JSON.stringify` does for the loop key.
fn js_number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e21 {
        format!("{}", value.trunc() as i64)
    } else if value.is_finite() {
        format!("{value}")
    } else {
        "null".to_string()
    }
}

/// Escape a JSON string.
fn json_escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => out.push(ch),
        }
    }
}

/// Quake II loop key (donor `q2LoopKey`).
fn q2_loop_key(
    actor: Option<&ActorId>,
    channel: f64,
    path: &str,
    loop_owner: Option<&ProviderId>,
    recipient: Option<&ActorId>,
) -> String {
    let mut key = String::from("[\"sound\",");
    match loop_owner {
        Some(owner) => {
            key.push('"');
            json_escape(&format!("{}:{}", owner.namespace, owner.name), &mut key);
            key.push_str("\",");
        }
        None => key.push_str("null,"),
    }
    key.push_str(&format!(
        "{},{},{},\"",
        actor.map_or(-1, |actor| actor.slot() as i64),
        actor.map_or(-1, |actor| actor.generation() as i64),
        js_number(channel),
    ));
    json_escape(path, &mut key);
    key.push_str("\",");
    match recipient {
        Some(recipient) => key.push_str(&format!("[{},{}]", recipient.slot(), recipient.generation())),
        None => key.push_str("null"),
    }
    key.push(']');
    key
}

/// Shader domain (donor `shaderDomain`).
fn shader_domain(original: &str) -> String {
    format!("shader:{}", normalize_shader_name(strip_shader_extension(original)))
}

/// Persistent domain (donor `persistentDomain`).
fn persistent_domain<F>(source: &RetainedPresentation<F>) -> Option<String> {
    match source {
        RetainedPresentation::Local(event) => Some(match &event.event {
            MediaRequest::ShaderRemap { original, .. } => shader_domain(original),
            _ => "music:track".to_string(),
        }),
        RetainedPresentation::Source(event) => match &event.source {
            SourcePresentationEvent::Q1Sky { .. } => Some("sky".to_string()),
            SourcePresentationEvent::Q1Client(metadata) => Some(format!(
                "client:{}:{}:{}",
                event.content,
                metadata.slot(),
                metadata.kind()
            )),
            SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle { style, .. })
            | SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle { style, .. }) => {
                Some(format!("style:{style}"))
            }
            SourcePresentationEvent::Q1(Q1PresentationEvent::Finale { .. })
            | SourcePresentationEvent::Q1Level(Q1LevelEvent::Finale { .. }) => Some("finale".to_string()),
            SourcePresentationEvent::Music(MusicEvent::Pause { .. }) => Some("music:pause".to_string()),
            SourcePresentationEvent::Music(_) => Some("music:track".to_string()),
            SourcePresentationEvent::Q2(Q2PresentationEvent::Music { .. }) => Some("music:track".to_string()),
            _ => None,
        },
    }
}

/// Persistent slot (donor `persistentSlot`).
fn persistent_slot<F>(source: &RetainedPresentation<F>) -> Option<String> {
    let domain = persistent_domain(source)?;
    let recipient = match source {
        RetainedPresentation::Local(_) => "world".to_string(),
        RetainedPresentation::Source(event) => event.recipient.as_ref().map_or_else(
            || "world".to_string(),
            |recipient| format!("{}:{}", recipient.slot(), recipient.generation()),
        ),
    };
    Some(format!("{domain}:{recipient}"))
}

/// Stored Q1 fog options (donor `fogOptions`).
///
/// The donor rebuilds a [`SimulationQ1Fog`] per owner from the stored
/// options object; the liveness closure is shared by reference so every
/// rebuild accepts the same players.
struct FogSeed {
    /// Map content owning the global transition.
    content: ContentId,
    /// Extra accepted source contents for component owners.
    accepted_contents: HashSet<ContentId>,
    /// Entity lump text seeding worldspawn fog.
    entities: String,
    /// Liveness probe for fogged players.
    alive: Rc<dyn Fn(&ActorId) -> bool>,
}

impl FogSeed {
    /// Rebuild canonical fog options, accepting one more content.
    fn options(&self, extra: Option<&ContentId>) -> SimulationQ1FogOptions {
        let mut accepted_contents = self.accepted_contents.clone();
        if let Some(extra) = extra {
            accepted_contents.insert(extra.clone());
        }
        let alive = Rc::clone(&self.alive);
        SimulationQ1FogOptions {
            content: self.content.clone(),
            accepted_contents: Some(accepted_contents),
            entities: self.entities.clone(),
            alive: Box::new(move |actor| alive(actor)),
        }
    }
}

/// Fog context for a local presentation event (donor `update` context).
fn fog_context<F>(presentation: &SimulationPresentationEvent<F>) -> Q1FogContext {
    Q1FogContext {
        content: presentation.content.clone(),
        sequence: presentation.sequence.max(0) as u64,
        seconds: presentation.seconds,
        source_entity: presentation.source_entity,
    }
}

/// Canonical addon event for local fog fields.
fn fog_addon_event(fog: &Q1FogFields) -> qa_content::q1::addons::context::Q1AddonEvent {
    qa_content::q1::addons::context::Q1AddonEvent::Fog {
        player: fog.player.clone(),
        density: fog.density,
        color: fog.color,
        sky_factor: fog.sky_factor,
        duration: fog.duration,
    }
}

/// Local presentation event for a canonical fog output event.
fn local_fog_event<F>(event: super::simulation::types::SimulationPresentationEvent) -> SimulationPresentationEvent<F> {
    let source = match event.event {
        super::simulation::types::SourcePresentationEvent::Q1Fog {
            player,
            transition,
            sky_factor,
        } => SourcePresentationEvent::Q1Fog {
            player,
            sky_factor,
            transition,
        },
        other => panic!("Q1 fog emitted a non-fog event: {other:?}"),
    };
    SimulationPresentationEvent {
        source,
        owner: event.owner,
        recipient: event.recipient,
        sequence: event.sequence as i64,
        content: event.content,
        seconds: event.seconds,
        source_entity: event.source_entity,
    }
}

/// Local save value to canonical fog save value.
fn fog_save_value(value: &SaveJson) -> qa_world::save::value::SaveJson {
    use qa_world::save::value::SaveJson as FogJson;
    match value {
        SaveJson::Null => FogJson::Null,
        SaveJson::Bool(value) => FogJson::Bool(*value),
        SaveJson::Number(value) => FogJson::Number(*value),
        SaveJson::BigInt(value) => FogJson::BigInt(*value),
        SaveJson::Bytes(value) => FogJson::Bytes(value.clone()),
        SaveJson::String(value) => FogJson::String(value.clone()),
        SaveJson::Array(values) => FogJson::Array(values.iter().map(fog_save_value).collect()),
        SaveJson::Object(members) => FogJson::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), fog_save_value(value)))
                .collect(),
        ),
    }
}

/// Canonical fog save value to local save value.
fn local_save_value(value: &qa_world::save::value::SaveJson) -> SaveJson {
    match value {
        qa_world::save::value::SaveJson::Null => SaveJson::Null,
        qa_world::save::value::SaveJson::Bool(value) => SaveJson::Bool(*value),
        qa_world::save::value::SaveJson::Number(value) => SaveJson::Number(*value),
        qa_world::save::value::SaveJson::BigInt(value) => SaveJson::BigInt(*value),
        qa_world::save::value::SaveJson::Bytes(value) => SaveJson::Bytes(value.clone()),
        qa_world::save::value::SaveJson::String(value) => SaveJson::String(value.clone()),
        qa_world::save::value::SaveJson::Array(values) => {
            SaveJson::Array(values.iter().map(local_save_value).collect())
        }
        qa_world::save::value::SaveJson::Object(members) => SaveJson::Object(
            members
                .iter()
                .map(|(key, value)| (key.clone(), local_save_value(value)))
                .collect(),
        ),
    }
}

/// Source slot resolution (donor `sourceSlot`).
pub type SourceSlotFn = Box<dyn Fn(&ActorId) -> Option<i32>>;

/// Owner status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnerStatus {
    /// Restored from a save.
    Restored,
    /// Active.
    Active,
}

/// Presentation owner entry.
struct OwnerEntry {
    token: PresentationOwner,
    content: ContentId,
    fog: Option<SimulationQ1Fog>,
    status: OwnerStatus,
}

/// Queued local media delivery.
struct LocalMediaDelivery<F> {
    request: Rc<RetainedPresentation<F>>,
    retain: bool,
    shader_replay: bool,
    current: Option<Box<dyn Fn() -> bool>>,
}

/// Lightstyle value.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LightstyleValue {
    family: String,
    pattern: String,
}

/// Format a provider id the way the donor interpolates it.
fn provider_text(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

/// Retained presentation events with owner lifecycle (donor `PresentationState`).
pub struct PresentationState<F> {
    presentation_sequence: i64,
    next_owner_generation: u64,
    legacy_persistence: bool,
    owners: HashMap<ProviderId, OwnerEntry>,
    source: Vec<SimulationPresentationEvent<F>>,
    resources: HashMap<String, ResolvedResourceReference>,
    resources_by_id: HashMap<ResourceId, ResolvedResourceReference>,
    styles: BTreeMap<i32, LightstyleValue>,
    legacy_styles: BTreeMap<i32, LightstyleValue>,
    persistent: HashMap<String, RetainedPresentation<F>>,
    local_media_output: HashMap<i64, LocalMediaDelivery<F>>,
    local_media_enabled: bool,
    media_sequence: i64,
    shader_sequences: HashMap<String, i64>,
    restored_through: i64,
    restored_owner_generation: u64,
    epoch: u64,
    now: Box<dyn Fn() -> SourceTime>,
    source_slot: SourceSlotFn,
    fog_seed: Option<FogSeed>,
    fog: Option<SimulationQ1Fog>,
}

impl<F: Clone + 'static> PresentationState<F> {
    /// Build the state over clock, source slot, and fog options closures.
    pub fn new(
        now: impl Fn() -> SourceTime + 'static,
        source_slot: impl Fn(&ActorId) -> Option<i32> + 'static,
        fog_options: Option<SimulationQ1FogOptions>,
    ) -> Result<Self, PresentationStateError> {
        let (fog_seed, fog) = match fog_options {
            None => (None, None),
            Some(options) => {
                let SimulationQ1FogOptions {
                    content,
                    accepted_contents,
                    entities,
                    alive,
                } = options;
                let shared: Rc<dyn Fn(&ActorId) -> bool> = Rc::from(alive);
                let rebuild = Rc::clone(&shared);
                let fog = SimulationQ1Fog::new(SimulationQ1FogOptions {
                    content: content.clone(),
                    accepted_contents: accepted_contents.clone(),
                    entities: entities.clone(),
                    alive: Box::new(move |actor| rebuild(actor)),
                })
                .map_err(|error| PresentationStateError::Fog(error.to_string()))?;
                let seed = FogSeed {
                    content,
                    accepted_contents: accepted_contents.unwrap_or_default(),
                    entities,
                    alive: shared,
                };
                (Some(seed), Some(fog))
            }
        };
        Ok(Self {
            presentation_sequence: 0,
            next_owner_generation: 1,
            legacy_persistence: false,
            owners: HashMap::new(),
            source: Vec::new(),
            resources: HashMap::new(),
            resources_by_id: HashMap::new(),
            styles: BTreeMap::new(),
            legacy_styles: BTreeMap::new(),
            persistent: HashMap::new(),
            local_media_output: HashMap::new(),
            local_media_enabled: false,
            media_sequence: -1,
            shader_sequences: HashMap::new(),
            restored_through: -1,
            restored_owner_generation: 0,
            epoch: 0,
            now: Box::new(now),
            source_slot: Box::new(source_slot),
            fog_seed,
            fog,
        })
    }

    /// Open an owner fog accepting content (donor `ownerFog`).
    fn owner_fog(&self, content: &ContentId) -> Result<Option<SimulationQ1Fog>, PresentationStateError> {
        let Some(seed) = self.fog_seed.as_ref() else {
            return Ok(None);
        };
        SimulationQ1Fog::new(seed.options(Some(content)))
            .map(Some)
            .map_err(|error| PresentationStateError::Fog(error.to_string()))
    }

    /// Current clock in seconds.
    fn seconds(&self) -> f64 {
        match (self.now)() {
            SourceTime::Seconds(value) => f64::from(value),
            SourceTime::Milliseconds(value) => f64::from(value) / 1000.0,
        }
    }

    /// Bind an owner (donor `bindOwner`).
    pub fn bind_owner(
        &mut self,
        provider: ProviderId,
        content: ContentId,
        restoring: bool,
    ) -> Result<BoundPresentationOwner<'_, F>, PresentationStateError> {
        if self
            .owners
            .get(&provider)
            .is_some_and(|prior| prior.status == OwnerStatus::Active)
        {
            return Err(PresentationStateError::OwnerActive(provider_text(&provider)));
        }
        if let Some(prior) = self.owners.get(&provider) {
            if !restoring || prior.content != content {
                return Err(PresentationStateError::OwnerDiffers(provider_text(&provider)));
            }
        }
        if restoring
            && self.legacy_persistence
            && (self.persistent.values().any(|event| event.content() == &content)
                || self
                    .fog
                    .as_ref()
                    .is_some_and(|fog| fog.presentation().iter().any(|event| event.content == content)))
        {
            return Err(PresentationStateError::LegacyOwnership(provider_text(&provider)));
        }
        if !self.owners.contains_key(&provider) && self.next_owner_generation >= MAX_OWNER_GENERATION {
            return Err(PresentationStateError::GenerationExhausted);
        }
        if !self.owners.contains_key(&provider) {
            let token = PresentationOwner {
                provider: provider.clone(),
                generation: self.next_owner_generation,
            };
            let fog = self.owner_fog(&content)?;
            self.owners.insert(
                provider.clone(),
                OwnerEntry {
                    token,
                    content: content.clone(),
                    fog,
                    status: OwnerStatus::Active,
                },
            );
            self.next_owner_generation += 1;
        }
        let token = self
            .owners
            .get_mut(&provider)
            .expect("inserted owner entry")
            .token
            .clone();
        self.owners.get_mut(&provider).expect("inserted owner entry").status = OwnerStatus::Active;
        let epoch = self.epoch;
        Ok(BoundPresentationOwner {
            state: self,
            provider,
            generation: token.generation,
            epoch,
            closed: false,
            token,
        })
    }

    /// Require all restored owners bound (donor `finishOwnerRestore`).
    pub fn finish_owner_restore(&mut self) -> Result<(), PresentationStateError> {
        for entry in self.owners.values() {
            if entry.status == OwnerStatus::Restored {
                return Err(PresentationStateError::OwnerUnrestored(provider_text(
                    &entry.token.provider,
                )));
            }
        }
        self.legacy_persistence = false;
        Ok(())
    }

    /// Refresh an active owner (donor `refreshOwner`).
    pub fn refresh_owner(&mut self, provider: &ProviderId) -> Result<(), PresentationStateError> {
        let entry = self.owners.get(provider);
        if entry.is_none_or(|entry| entry.status != OwnerStatus::Active) {
            return Err(PresentationStateError::OwnerInactive(provider_text(provider)));
        }
        let entry = entry.expect("checked owner entry");
        let content = entry.content.clone();
        let token = entry.token.clone();
        self.emit(
            &content,
            SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Refreshed { owner: token },
            },
            None,
            None,
        );
        Ok(())
    }

    /// Retire an owner (donor `retireOwner`).
    fn retire_owner(&mut self, owner: &PresentationOwner, content: &ContentId) {
        let mut affected = HashSet::new();
        let mut local_shaders: HashMap<String, LocalPresentationMedia> = HashMap::new();
        let mut local_music = false;
        let keys: Vec<String> = self
            .persistent
            .iter()
            .filter(|(_, event)| same_presentation_owner(event.owner(), owner))
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            if let Some(event) = self.persistent.remove(key) {
                match &event {
                    RetainedPresentation::Local(local) => match &local.event {
                        MediaRequest::ShaderRemap { original, .. } => {
                            local_shaders.insert(shader_domain(original), local.clone());
                        }
                        _ => local_music = true,
                    },
                    RetainedPresentation::Source(_) => {
                        if let Some(domain) = persistent_domain(&event) {
                            affected.insert(domain);
                        }
                    }
                }
            }
        }
        let sequences: Vec<i64> = self
            .local_media_output
            .iter()
            .filter(|(_, delivery)| same_presentation_owner(delivery.request.owner(), owner))
            .map(|(sequence, _)| *sequence)
            .collect();
        for sequence in &sequences {
            if let Some(delivery) = self.local_media_output.remove(sequence) {
                if let RetainedPresentation::Local(local) = delivery.request.as_ref() {
                    if let MediaRequest::ShaderRemap { original, .. } = &local.event {
                        local_shaders.insert(shader_domain(original), local.clone());
                    }
                }
            }
        }
        self.emit(
            content,
            SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Retired { owner: owner.clone() },
            },
            None,
            None,
        );
        let mut replacements: HashMap<String, SimulationPresentationEvent<F>> = HashMap::new();
        for event in self.persistent.values() {
            let RetainedPresentation::Source(event) = event else {
                continue;
            };
            let retained = RetainedPresentation::Source(event.clone());
            if let Some(slot) = persistent_slot(&retained) {
                if affected.contains(persistent_domain(&retained).as_deref().unwrap_or(""))
                    && replacements
                        .get(&slot)
                        .is_none_or(|prior| prior.sequence < event.sequence)
                {
                    replacements.insert(slot, event.clone());
                }
            }
        }
        let mut replacements: Vec<SimulationPresentationEvent<F>> = replacements.into_values().collect();
        replacements.sort_by_key(|event| event.sequence);
        for event in replacements {
            let sequence = self.presentation_sequence;
            self.presentation_sequence += 1;
            self.source.push(SimulationPresentationEvent { sequence, ..event });
        }
        if self.local_media_enabled
            && (local_music
                || affected.contains("music:track")
                    && self.persistent.values().any(|event| {
                        matches!(
                            event,
                            RetainedPresentation::Local(local)
                                if !matches!(local.event, MediaRequest::ShaderRemap { .. })
                        )
                    }))
        {
            let replacement = self
                .persistent
                .values()
                .filter(|event| persistent_domain(event).as_deref() == Some("music:track"))
                .max_by_key(|event| event.sequence())
                .cloned();
            if let Some(replacement) = replacement {
                let sequence = self.presentation_sequence;
                self.presentation_sequence += 1;
                let replay = match replacement {
                    RetainedPresentation::Source(mut event) => {
                        event.sequence = sequence;
                        RetainedPresentation::Source(event)
                    }
                    RetainedPresentation::Local(mut event) => {
                        event.sequence = sequence;
                        RetainedPresentation::Local(event)
                    }
                };
                self.local_media_output.insert(
                    sequence,
                    LocalMediaDelivery {
                        request: Rc::new(replay),
                        retain: false,
                        shader_replay: false,
                        current: None,
                    },
                );
            }
        }
        if self.local_media_enabled {
            let mut domains: Vec<(String, LocalPresentationMedia)> = local_shaders.into_iter().collect();
            domains.sort_by(|left, right| left.0.cmp(&right.0));
            for (domain, removed) in domains {
                let replacement = self
                    .persistent
                    .values()
                    .filter(|event| persistent_domain(event).as_deref() == Some(domain.as_str()))
                    .max_by_key(|event| event.sequence())
                    .cloned();
                if let Some(replacement) = replacement {
                    let sequence = self.presentation_sequence;
                    self.presentation_sequence += 1;
                    let request = match replacement {
                        RetainedPresentation::Source(mut event) => {
                            event.sequence = sequence;
                            RetainedPresentation::Source(event)
                        }
                        RetainedPresentation::Local(mut event) => {
                            event.sequence = sequence;
                            RetainedPresentation::Local(event)
                        }
                    };
                    self.local_media_output.insert(
                        sequence,
                        LocalMediaDelivery {
                            request: Rc::new(request),
                            retain: false,
                            shader_replay: true,
                            current: None,
                        },
                    );
                } else if let MediaRequest::ShaderRemap { original, .. } = &removed.event {
                    let sequence = self.presentation_sequence;
                    self.presentation_sequence += 1;
                    let request = RetainedPresentation::Local(LocalPresentationMedia {
                        event: MediaRequest::ShaderRemap {
                            original: original.clone(),
                            replacement: original.clone(),
                            time_offset: 0.0,
                        },
                        owner: None,
                        content: removed.content.clone(),
                        sequence,
                        seconds: removed.seconds,
                    });
                    self.local_media_output.insert(
                        sequence,
                        LocalMediaDelivery {
                            request: Rc::new(request),
                            retain: false,
                            shader_replay: true,
                            current: None,
                        },
                    );
                }
            }
        }
        let mut fog: Vec<SimulationPresentationEvent<F>> = self
            .fog
            .as_ref()
            .map(|fog| fog.presentation().into_iter().map(local_fog_event).collect())
            .unwrap_or_default();
        for entry in self.owners.values() {
            if let Some(entry_fog) = entry.fog.as_ref() {
                fog.extend(entry_fog.presentation().into_iter().map(|event| {
                    let mut event = local_fog_event(event);
                    event.owner = Some(entry.token.clone());
                    event
                }));
            }
        }
        fog.sort_by_key(|event| event.sequence);
        for event in fog {
            let sequence = self.presentation_sequence;
            self.presentation_sequence += 1;
            self.source.push(SimulationPresentationEvent { sequence, ..event });
        }
        self.rebuild_styles();
    }

    /// Rebuild lightstyles from legacy and persistent events (donor `rebuildStyles`).
    fn rebuild_styles(&mut self) {
        self.styles.clear();
        for (style, value) in &self.legacy_styles {
            self.styles.insert(*style, value.clone());
        }
        let mut sources: Vec<&RetainedPresentation<F>> = self.persistent.values().collect();
        sources.sort_by_key(|source| source.sequence());
        for source in sources {
            if let RetainedPresentation::Source(event) = source {
                let lightstyle = match &event.source {
                    SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle { style, pattern }) => {
                        Some(("q1", style, pattern))
                    }
                    SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle { style, pattern }) => {
                        Some(("q2", style, pattern))
                    }
                    _ => None,
                };
                if let Some((family, style, pattern)) = lightstyle {
                    if event.recipient.is_none() {
                        self.styles.insert(
                            *style,
                            LightstyleValue {
                                family: family.to_string(),
                                pattern: pattern.clone(),
                            },
                        );
                    }
                }
            }
        }
    }
}

/// Bound presentation owner handle (donor `bindOwner` result).
pub struct BoundPresentationOwner<'a, F> {
    state: &'a mut PresentationState<F>,
    provider: ProviderId,
    generation: u64,
    epoch: u64,
    closed: bool,
    token: PresentationOwner,
}

impl<F: Clone + 'static> BoundPresentationOwner<'_, F> {
    /// Bound owner token.
    pub fn owner(&self) -> &PresentationOwner {
        &self.token
    }

    /// Require the handle current (donor `current`).
    fn current(&self) -> Result<(), PresentationStateError> {
        if self.closed
            || self.epoch != self.state.epoch
            || self
                .state
                .owners
                .get(&self.provider)
                .is_none_or(|entry| entry.token.generation != self.generation || entry.status != OwnerStatus::Active)
        {
            return Err(PresentationStateError::OwnerRetired(provider_text(&self.provider)));
        }
        Ok(())
    }

    /// Emit an owned source event (donor handle `emit`).
    pub fn emit(
        &mut self,
        content: &ContentId,
        source: SourcePresentationEvent<F>,
        time: Option<SourceTime>,
        recipient: Option<&ActorId>,
    ) -> Result<(), PresentationStateError> {
        self.current()?;
        if matches!(source, SourcePresentationEvent::PresentationOwner { .. }) {
            return Err(PresentationStateError::OwnerLifecycle);
        }
        let token = self.token.clone();
        self.state.emit_owned(Some(&token), content, source, time, recipient);
        Ok(())
    }

    /// Register a resource (donor handle `registerResource`).
    pub fn register_resource(
        &mut self,
        content: &ContentId,
        path: &str,
        resource: ResolvedResourceReference,
    ) -> Result<(), PresentationStateError> {
        self.current()?;
        self.state.register_resource(content, path, resource)
    }

    /// Close the handle, retiring the owner once (donor handle `close`).
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let retired = self
            .state
            .owners
            .get(&self.provider)
            .is_some_and(|entry| entry.token.generation == self.generation);
        if retired {
            if let Some(entry) = self.state.owners.remove(&self.provider) {
                let token = entry.token.clone();
                let content = entry.content.clone();
                self.state.retire_owner(&token, &content);
            }
        }
    }
}

impl<F: Clone + 'static> PresentationState<F> {
    /// Retire an actor (donor `retire`).
    pub fn retire(&mut self, actor: &ActorId) {
        if let Some(fog) = self.fog.as_mut() {
            fog.retire(actor);
        }
        for entry in self.owners.values_mut() {
            if let Some(fog) = entry.fog.as_mut() {
                fog.retire(actor);
            }
        }
        let keys: Vec<String> = self
            .persistent
            .iter()
            .filter(|(_, event)| {
                matches!(event, RetainedPresentation::Source(event) if event.recipient.as_ref() == Some(actor))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &keys {
            self.persistent.remove(key);
        }
    }

    /// Look up a registered resource (donor `resource`).
    pub fn resource(&self, id: &ResourceId) -> Option<&ResolvedResourceReference> {
        self.resources_by_id.get(id)
    }

    /// Look up a registered resource by content and path (donor protected `resources`).
    pub fn resource_by_path(&self, content: &ContentId, path: &str) -> Option<&ResolvedResourceReference> {
        self.resources.get(&format!("{content}/{path}"))
    }

    /// Persistent source presentation (donor `persistentPresentation`).
    pub fn persistent_presentation(&self) -> Vec<SimulationPresentationEvent<F>> {
        let mut events: Vec<SimulationPresentationEvent<F>> = self
            .persistent
            .values()
            .filter_map(|event| match event {
                RetainedPresentation::Source(event) => Some(event.clone()),
                RetainedPresentation::Local(_) => None,
            })
            .collect();
        events.sort_by_key(|event| event.sequence);
        events
    }

    /// Publish local media (donor `publishLocalMedia`).
    pub fn publish_local_media(
        &mut self,
        owner: Option<&PresentationOwner>,
        content: &ContentId,
        event: MediaRequest,
        initializing: bool,
        current: Option<impl Fn() -> bool + 'static>,
    ) -> Result<(), PresentationStateError> {
        if let Some(owner) = owner {
            let active = self.owners.get(&owner.provider);
            if active.is_none_or(|active| {
                active.status != OwnerStatus::Active
                    || !same_presentation_owner(Some(&active.token), owner)
                    || active.content != *content
            }) {
                return Err(PresentationStateError::MediaOwnership);
            }
        }
        self.enable_local_media();
        let domain = match &event {
            MediaRequest::ShaderRemap { original, .. } => shader_domain(original),
            _ => "music:track".to_string(),
        };
        if initializing
            && owner.is_none_or(|owner| owner.generation < self.restored_owner_generation)
            && self.persistent.values().any(|value| {
                value.sequence() < self.restored_through && persistent_domain(value).as_deref() == Some(domain.as_str())
            })
        {
            return Ok(());
        }
        let sequence = self.presentation_sequence;
        self.presentation_sequence += 1;
        let request = RetainedPresentation::Local(LocalPresentationMedia {
            event,
            owner: owner.cloned(),
            content: content.clone(),
            sequence,
            seconds: self.seconds(),
        });
        let retain = matches!(
            request,
            RetainedPresentation::Local(LocalPresentationMedia {
                event: MediaRequest::ShaderRemap { .. },
                ..
            })
        );
        if !retain {
            if let Some(slot) = persistent_slot(&request) {
                self.persistent.insert(
                    format!("local:{}:{slot}", presentation_owner_key(request.owner())),
                    request.clone(),
                );
            }
        }
        self.local_media_output.insert(
            sequence,
            LocalMediaDelivery {
                request: Rc::new(request),
                retain,
                shader_replay: false,
                current: current.map(|current| Box::new(current) as Box<dyn Fn() -> bool>),
            },
        );
        Ok(())
    }

    /// Enable local media delivery (donor `enableLocalMedia`).
    pub fn enable_local_media(&mut self) {
        if self.local_media_enabled {
            return;
        }
        self.local_media_enabled = true;
        for event in self.persistent.values() {
            if matches!(event, RetainedPresentation::Local(_)) {
                self.local_media_output.insert(
                    event.sequence(),
                    LocalMediaDelivery {
                        request: Rc::new(event.clone()),
                        retain: false,
                        shader_replay: false,
                        current: None,
                    },
                );
            }
        }
    }

    /// Pending local media (donor `pendingLocalMedia`).
    pub fn pending_local_media(&self) -> Vec<Rc<RetainedPresentation<F>>> {
        let mut pending: Vec<Rc<RetainedPresentation<F>>> = self
            .local_media_output
            .values()
            .map(|delivery| Rc::clone(&delivery.request))
            .collect();
        pending.sort_by_key(|request| request.sequence());
        pending
    }

    /// Applied media sequence (donor `appliedMediaSequence`).
    pub fn applied_media_sequence(&self) -> i64 {
        self.media_sequence
    }

    /// Record applied media (donor `appliedMedia`).
    pub fn applied_media(&mut self, sequence: i64) {
        self.media_sequence = self.media_sequence.max(sequence);
    }

    /// Record an applied shader remap (donor `appliedShader`).
    pub fn applied_shader(&mut self, original: &str, sequence: i64) {
        let domain = shader_domain(original);
        let prior = self.shader_sequences.get(&domain).copied().unwrap_or(-1);
        self.shader_sequences.insert(domain, prior.max(sequence));
    }

    /// Resolve shader replay (donor `resolveShaderReplay`).
    pub fn resolve_shader_replay(
        &mut self,
        request: &Rc<RetainedPresentation<F>>,
    ) -> Result<Option<Rc<RetainedPresentation<F>>>, PresentationStateError> {
        let sequence = request.sequence();
        let replay = match self.local_media_output.get(&sequence) {
            Some(delivery)
                if Rc::ptr_eq(&delivery.request, request)
                    && delivery.shader_replay
                    && matches!(
                        delivery.request.as_ref(),
                        RetainedPresentation::Local(LocalPresentationMedia {
                            event: MediaRequest::ShaderRemap { .. },
                            ..
                        })
                    ) =>
            {
                true
            }
            Some(delivery) if Rc::ptr_eq(&delivery.request, request) => return Ok(Some(Rc::clone(request))),
            _ => return Ok(None),
        };
        if !replay {
            return Ok(None);
        }
        let RetainedPresentation::Local(local) = request.as_ref() else {
            return Ok(None);
        };
        let MediaRequest::ShaderRemap { original, .. } = &local.event else {
            return Ok(None);
        };
        let domain = shader_domain(original);
        let winner = self
            .persistent
            .values()
            .filter(|event| persistent_domain(event).as_deref() == Some(domain.as_str()))
            .max_by_key(|event| event.sequence())
            .cloned();
        if let Some(RetainedPresentation::Source(_)) = winner {
            return Err(PresentationStateError::ShaderReplay);
        }
        let resolved = match winner {
            None => RetainedPresentation::Local(LocalPresentationMedia {
                event: MediaRequest::ShaderRemap {
                    original: original.clone(),
                    replacement: original.clone(),
                    time_offset: 0.0,
                },
                owner: None,
                content: local.content.clone(),
                sequence,
                seconds: local.seconds,
            }),
            Some(RetainedPresentation::Local(mut winner)) => {
                winner.sequence = sequence;
                RetainedPresentation::Local(winner)
            }
            Some(RetainedPresentation::Source(_)) => unreachable!("checked shader winner"),
        };
        self.local_media_output.insert(
            sequence,
            LocalMediaDelivery {
                request: Rc::new(resolved),
                retain: false,
                shader_replay: false,
                current: None,
            },
        );
        Ok(self
            .local_media_output
            .get(&sequence)
            .map(|delivery| Rc::clone(&delivery.request)))
    }

    /// Whether local media is current (donor `localMediaCurrent`).
    pub fn local_media_current(&self, request: &Rc<RetainedPresentation<F>>) -> bool {
        let Some(delivery) = self.local_media_output.get(&request.sequence()) else {
            return false;
        };
        if !Rc::ptr_eq(&delivery.request, request) {
            return false;
        }
        if delivery.current.as_ref().is_some_and(|current| !current()) {
            return false;
        }
        if let Some(owner) = request.owner() {
            if !same_presentation_owner(self.owners.get(&owner.provider).map(|entry| &entry.token), owner) {
                return false;
            }
        }
        let RetainedPresentation::Local(local) = request.as_ref() else {
            return true;
        };
        let MediaRequest::ShaderRemap { original, .. } = &local.event else {
            return true;
        };
        local.sequence
            >= self
                .shader_sequences
                .get(&shader_domain(original))
                .copied()
                .unwrap_or(-1)
    }

    /// Acknowledge local media (donor `acknowledgeLocalMedia`).
    pub fn acknowledge_local_media(&mut self, request: &Rc<RetainedPresentation<F>>, committed: bool) {
        let sequence = request.sequence();
        let retain = match self.local_media_output.get(&sequence) {
            Some(delivery) if Rc::ptr_eq(&delivery.request, request) => delivery.retain,
            _ => return,
        };
        if committed && retain {
            if let Some(slot) = persistent_slot(request) {
                self.persistent.insert(
                    format!("local:{}:{slot}", presentation_owner_key(request.owner())),
                    (**request).clone(),
                );
            }
        }
        self.local_media_output.remove(&sequence);
    }

    /// Require consumed local media (donor `assertLocalMediaConsumed`).
    pub fn assert_local_media_consumed(&self) -> Result<(), PresentationStateError> {
        if !self.local_media_output.is_empty() {
            return Err(PresentationStateError::MediaPending);
        }
        Ok(())
    }

    /// Register a resource (donor `registerResource`).
    pub fn register_resource(
        &mut self,
        content: &ContentId,
        path: &str,
        resource: ResolvedResourceReference,
    ) -> Result<(), PresentationStateError> {
        if resource.requested_path != path {
            return Err(PresentationStateError::ResourcePath);
        }
        self.resources.insert(format!("{content}/{path}"), resource.clone());
        self.resources_by_id.insert(resource.id.clone(), resource);
        Ok(())
    }

    /// Emit a source event (donor `emit`).
    pub fn emit(
        &mut self,
        content: &ContentId,
        source: SourcePresentationEvent<F>,
        time: Option<SourceTime>,
        recipient: Option<&ActorId>,
    ) {
        self.emit_owned(None, content, source, time, recipient);
    }

    /// Emit an owned source event (donor `emitOwned`).
    fn emit_owned(
        &mut self,
        owner: Option<&PresentationOwner>,
        content: &ContentId,
        source: SourcePresentationEvent<F>,
        time: Option<SourceTime>,
        recipient: Option<&ActorId>,
    ) -> SimulationPresentationEvent<F> {
        let source = match source {
            SourcePresentationEvent::Q1(Q1PresentationEvent::StaticModel {
                path,
                frame,
                color_map,
                skin,
                origin,
                angles,
            }) => SourcePresentationEvent::Q1(Q1PresentationEvent::StaticModel {
                path,
                frame: frame.trunc(),
                color_map: color_map.trunc(),
                skin: skin.trunc(),
                origin,
                angles,
            }),
            source => source,
        };
        let seconds = match time.unwrap_or_else(|| (self.now)()) {
            SourceTime::Seconds(value) => f64::from(value),
            SourceTime::Milliseconds(value) => f64::from(value) / 1000.0,
        };
        let source_entity = source.actor().and_then(|actor| (self.source_slot)(actor));
        let sequence = self.presentation_sequence;
        self.presentation_sequence += 1;
        let presentation = SimulationPresentationEvent {
            source,
            owner: owner.cloned(),
            recipient: recipient.cloned(),
            sequence,
            content: content.clone(),
            seconds,
            source_entity,
        };
        self.record(presentation.clone());
        presentation
    }

    /// Admit a replicated owner (donor `admitReplicatedOwner`).
    pub fn admit_replicated_owner(
        &mut self,
        token: PresentationOwner,
        content: ContentId,
    ) -> Result<(), PresentationStateError> {
        if let Some(previous) = self.owners.remove(&token.provider) {
            if same_presentation_owner(Some(&previous.token), &token) {
                if previous.content != content || previous.status != OwnerStatus::Active {
                    self.owners.insert(token.provider.clone(), previous);
                    return Err(PresentationStateError::ReplicatedIdentity);
                }
                self.owners.insert(token.provider.clone(), previous);
                return Ok(());
            }
            if token.generation <= previous.token.generation {
                self.owners.insert(token.provider.clone(), previous);
                return Err(PresentationStateError::ReplicatedBackward);
            }
            let previous_token = previous.token.clone();
            let previous_content = previous.content.clone();
            self.retire_owner(&previous_token, &previous_content);
        }
        let generation = token.generation.saturating_add(1);
        self.next_owner_generation = self.next_owner_generation.max(generation);
        if self.next_owner_generation > MAX_OWNER_GENERATION {
            return Err(PresentationStateError::GenerationExhausted);
        }
        let fog = self.owner_fog(&content)?;
        self.owners.insert(
            token.provider.clone(),
            OwnerEntry {
                token,
                content,
                fog,
                status: OwnerStatus::Active,
            },
        );
        Ok(())
    }

    /// Retire a replicated owner (donor `retireReplicatedOwner`).
    pub fn retire_replicated_owner(&mut self, token: &PresentationOwner) {
        let retired = self
            .owners
            .get(&token.provider)
            .is_some_and(|entry| same_presentation_owner(Some(&entry.token), token));
        if !retired {
            return;
        }
        if let Some(entry) = self.owners.remove(&token.provider) {
            let content = entry.content.clone();
            self.retire_owner(token, &content);
        }
    }

    /// Receive a replicated presentation (donor `receivePresentation`).
    pub fn receive_presentation(&mut self, source: SimulationPresentationEvent<F>) -> i64 {
        let sequence = self.presentation_sequence;
        if let SourcePresentationEvent::PresentationOwner {
            event: OwnerLifecycle::Retired { owner },
        } = &source.source
        {
            let owner = owner.clone();
            let content = source.content.clone();
            if self
                .owners
                .get(&owner.provider)
                .is_some_and(|entry| same_presentation_owner(Some(&entry.token), &owner))
            {
                self.owners.remove(&owner.provider);
            }
            self.retire_owner(&owner, &content);
            return sequence;
        }
        let sequence = self.presentation_sequence;
        self.presentation_sequence += 1;
        self.record(SimulationPresentationEvent { sequence, ..source });
        sequence
    }

    /// Record a presentation (donor `record`).
    fn record(&mut self, presentation: SimulationPresentationEvent<F>) {
        let recipient = presentation.recipient.clone();
        let owner = presentation.owner.clone();
        self.source.push(presentation.clone());
        if let SourcePresentationEvent::Q1Composition(Q1CompositionEvent::Addon(Q1AddonEvent::Fog(fog))) =
            &presentation.source
        {
            let context = fog_context(&presentation);
            let event = fog_addon_event(fog);
            let update = match &owner {
                None => self.fog.as_mut().map(|fog_state| {
                    fog_state
                        .update(&context, &event)
                        .into_iter()
                        .map(local_fog_event)
                        .collect::<Vec<_>>()
                }),
                Some(owner) => self
                    .owners
                    .get_mut(&owner.provider)
                    .and_then(|entry| entry.fog.as_mut())
                    .map(|fog_state| {
                        fog_state
                            .update(&context, &event)
                            .into_iter()
                            .map(local_fog_event)
                            .collect::<Vec<_>>()
                    }),
            };
            if let Some(mut events) = update {
                for event in events.drain(..) {
                    let mut event = event;
                    if owner.is_some() {
                        event.owner = owner.clone();
                    }
                    self.source.push(event);
                }
            }
        }
        if let Some(slot) = persistent_slot(&RetainedPresentation::Source(presentation.clone())) {
            self.persistent.insert(
                format!("{}:{slot}", presentation_owner_key(presentation.owner.as_ref())),
                RetainedPresentation::Source(presentation.clone()),
            );
        }
        if let SourcePresentationEvent::Q1(event) = &presentation.source {
            if matches!(
                event,
                Q1PresentationEvent::Ambient { .. } | Q1PresentationEvent::StaticModel { .. }
            ) {
                let kind = match event {
                    Q1PresentationEvent::Ambient { .. } => "ambient",
                    _ => "static-model",
                };
                self.persistent.insert(
                    format!(
                        "{}:{kind}:{}",
                        presentation_owner_key(presentation.owner.as_ref()),
                        presentation.sequence
                    ),
                    RetainedPresentation::Source(presentation.clone()),
                );
            }
        }
        if let SourcePresentationEvent::Q2(Q2PresentationEvent::Sound {
            actor,
            path,
            channel,
            loop_mode,
            loop_owner,
            ..
        }) = &presentation.source
        {
            if *loop_mode != Q2SoundLoop::Once {
                let key = format!(
                    "{}:{}",
                    presentation_owner_key(presentation.owner.as_ref()),
                    q2_loop_key(actor.as_ref(), *channel, path, loop_owner.as_ref(), recipient.as_ref())
                );
                if *loop_mode == Q2SoundLoop::Stop {
                    self.persistent.remove(&key);
                } else {
                    self.persistent
                        .insert(key, RetainedPresentation::Source(presentation.clone()));
                }
            }
        }
        let lightstyle = match &presentation.source {
            SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle { style, pattern }) => {
                Some(("q1", style, pattern))
            }
            SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle { style, pattern }) => {
                Some(("q2", style, pattern))
            }
            _ => None,
        };
        if let Some((family, style, pattern)) = lightstyle {
            if matches!(presentation.source, SourcePresentationEvent::Q2(_)) || recipient.is_none() {
                self.styles.insert(
                    *style,
                    LightstyleValue {
                        family: family.to_string(),
                        pattern: pattern.clone(),
                    },
                );
            }
        }
    }

    /// Take source output (donor `takePresentation`).
    pub fn take_presentation(&mut self) -> Vec<SimulationPresentationEvent<F>> {
        std::mem::take(&mut self.source)
    }

    /// Require consumed output (donor `assertOutputConsumed`).
    pub fn assert_output_consumed(&self) -> Result<(), PresentationStateError> {
        self.assert_local_media_consumed()?;
        if !self.source.is_empty() {
            return Err(PresentationStateError::OutputPending);
        }
        Ok(())
    }
}

impl<F: Clone + 'static> PresentationState<F> {
    /// Capture the state (donor `capture`).
    pub fn capture(&self) -> Result<SaveJson, PresentationStateError> {
        let mut owners: Vec<(&ProviderId, &OwnerEntry)> = self.owners.iter().collect();
        owners.sort_by(|left, right| {
            left.1
                .token
                .generation
                .cmp(&right.1.token.generation)
                .then_with(|| provider_text(left.0).cmp(&provider_text(right.0)))
        });
        let ownership = obj(vec![
            ("nextGeneration", int(self.next_owner_generation as i64)),
            (
                "owners",
                arr(owners
                    .into_iter()
                    .map(|(_, entry)| {
                        obj(vec![
                            ("provider", write_provider(&entry.token.provider)),
                            ("generation", int(entry.token.generation as i64)),
                            ("content", str(entry.content.as_str())),
                            (
                                "fog",
                                entry
                                    .fog
                                    .as_ref()
                                    .map(|fog| local_save_value(&fog.capture()))
                                    .unwrap_or(SaveJson::Null),
                            ),
                        ])
                    })
                    .collect()),
            ),
        ]);
        let base_styles = arr(self
            .legacy_styles
            .iter()
            .map(|(style, value)| {
                obj(vec![
                    ("style", int(i64::from(*style))),
                    ("family", str(&value.family)),
                    ("pattern", str(&value.pattern)),
                ])
            })
            .collect());
        let styles = arr(self
            .styles
            .iter()
            .map(|(style, value)| {
                obj(vec![
                    ("style", int(i64::from(*style))),
                    ("family", str(&value.family)),
                    ("pattern", str(&value.pattern)),
                ])
            })
            .collect());
        let mut persistent: Vec<(&String, &RetainedPresentation<F>)> = self.persistent.iter().collect();
        persistent.sort_by_key(|(_, event)| event.sequence());
        let mut saved = Vec::new();
        for (key, event) in persistent {
            saved.push(self.capture_retained(key, event)?);
        }
        Ok(obj(vec![
            ("ownership", ownership),
            ("baseStyles", base_styles),
            (
                "q1Fog",
                self.fog
                    .as_ref()
                    .map(|fog| local_save_value(&fog.capture()))
                    .unwrap_or(SaveJson::Null),
            ),
            ("presentationSequence", int(self.presentation_sequence)),
            ("styles", styles),
            ("persistent", arr(saved)),
        ]))
    }

    /// Capture one retained event (donor persistent map).
    fn capture_retained(&self, key: &str, event: &RetainedPresentation<F>) -> Result<SaveJson, PresentationStateError> {
        if let RetainedPresentation::Local(local) = event {
            let mut members: Vec<(&str, SaveJson)> = vec![("key", str(key)), ("kind", str("local-media"))];
            if let Some(owner) = &local.owner {
                members.push(("owner", write_owner(owner)));
            }
            members.extend([
                ("content", str(local.content.as_str())),
                ("sequence", int(local.sequence)),
                ("seconds", num(local.seconds)),
                ("event", write_media_request(&local.event)),
            ]);
            return Ok(obj(members));
        }
        let RetainedPresentation::Source(event) = event else {
            return Err(PresentationStateError::UnsupportedPersistent);
        };
        let mut members: Vec<(&str, SaveJson)> = vec![("key", str(key)), ("kind", str(event.source.kind()))];
        if let Some(owner) = &event.owner {
            members.push(("owner", write_owner(owner)));
        }
        members.extend([
            (
                "recipient",
                event
                    .recipient
                    .as_ref()
                    .map(write_saved_actor)
                    .unwrap_or(SaveJson::Null),
            ),
            ("content", str(event.content.as_str())),
            ("sequence", int(event.sequence)),
            ("seconds", num(event.seconds)),
            (
                "sourceEntity",
                event.source_entity.map_or(SaveJson::Null, |slot| int(i64::from(slot))),
            ),
        ]);
        let inner = match &event.source {
            SourcePresentationEvent::Q1(Q1PresentationEvent::Ambient {
                origin,
                path,
                volume,
                attenuation,
            }) => obj(vec![
                ("kind", str("ambient")),
                ("origin", write_vector(*origin)),
                ("path", str(path)),
                ("volume", num(*volume)),
                ("attenuation", num(*attenuation)),
            ]),
            SourcePresentationEvent::Q1(Q1PresentationEvent::StaticModel {
                path,
                frame,
                color_map,
                skin,
                origin,
                angles,
            }) => obj(vec![
                ("kind", str("static-model")),
                ("path", str(path)),
                ("frame", num(*frame)),
                ("colorMap", num(*color_map)),
                ("skin", num(*skin)),
                ("origin", write_vector(*origin)),
                ("angles", write_vector(*angles)),
            ]),
            SourcePresentationEvent::Q1(Q1PresentationEvent::Finale { text, stage }) => obj(vec![
                ("kind", str("finale")),
                ("text", str(text)),
                ("stage", int(i64::from(*stage))),
            ]),
            SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle { style, pattern })
            | SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle { style, pattern }) => obj(vec![
                ("kind", str("lightstyle")),
                ("style", int(i64::from(*style))),
                ("pattern", str(pattern)),
            ]),
            SourcePresentationEvent::Q1Sky { name } => obj(vec![("kind", str("skybox")), ("name", str(name))]),
            SourcePresentationEvent::Q1Client(metadata) => write_client_metadata(metadata),
            SourcePresentationEvent::Q1Level(Q1LevelEvent::Finale { text, track }) => obj(vec![
                ("kind", str("finale")),
                ("text", str(text)),
                ("track", int(i64::from(*track))),
            ]),
            SourcePresentationEvent::Q2(Q2PresentationEvent::Sound {
                actor,
                origin,
                path,
                channel,
                volume,
                attenuation,
                reliable,
                loop_mode,
                loop_owner,
            }) => {
                let mut sound: Vec<(&str, SaveJson)> = vec![
                    ("kind", str("sound")),
                    ("actor", actor.as_ref().map(write_saved_actor).unwrap_or(SaveJson::Null)),
                    ("origin", write_vector(*origin)),
                    ("path", str(path)),
                    ("channel", num(*channel)),
                    ("volume", num(*volume)),
                    ("attenuation", num(*attenuation)),
                    ("reliable", boolean(*reliable)),
                    ("loop", str(loop_mode.as_str())),
                ];
                if let Some(owner) = loop_owner {
                    sound.push(("loopOwner", write_provider(owner)));
                }
                obj(sound)
            }
            SourcePresentationEvent::Q2(Q2PresentationEvent::Music { track }) => {
                obj(vec![("kind", str("music")), ("track", str(track))])
            }
            SourcePresentationEvent::Music(MusicEvent::CdTrack { track }) => {
                obj(vec![("kind", str("cd-track")), ("track", int(i64::from(*track)))])
            }
            SourcePresentationEvent::Music(MusicEvent::Pause { paused }) => {
                obj(vec![("kind", str("pause")), ("paused", boolean(*paused))])
            }
            _ => return Err(PresentationStateError::UnsupportedPersistent),
        };
        members.push(("event", inner));
        Ok(obj(members))
    }

    /// Restore the state (donor `restore`).
    pub fn restore(
        &mut self,
        reader: SaveReader,
        reference: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<(), PresentationStateError> {
        self.presentation_sequence = reader.field("presentationSequence").integer(0)?;
        self.restored_through = self.presentation_sequence;
        self.local_media_output.clear();
        self.local_media_enabled = false;
        self.media_sequence = -1;
        self.shader_sequences.clear();
        self.source.clear();
        self.styles.clear();
        self.legacy_styles.clear();
        self.persistent.clear();
        self.owners.clear();
        self.epoch += 1;
        let ownership = reader.field("ownership");
        self.legacy_persistence = ownership.value.is_none();
        self.next_owner_generation = if ownership.value.is_none() {
            1
        } else {
            u64::try_from(ownership.field("nextGeneration").integer(1)?)
                .map_err(|_| ownership.fail("Invalid presentation generation counter"))?
        };
        if self.next_owner_generation > MAX_OWNER_GENERATION {
            return Err(ownership.fail("Invalid presentation generation counter").into());
        }
        self.restored_owner_generation = self.next_owner_generation;
        if ownership.value.is_some() {
            ownership
                .field("owners")
                .list(|value| -> Result<(), PresentationStateError> {
                    let token = read_presentation_owner(&value)?;
                    let content = read_content_id(&value.field("content"))?;
                    if token.generation >= self.next_owner_generation || self.owners.contains_key(&token.provider) {
                        return Err(value.fail("Invalid saved presentation owner").into());
                    }
                    let mut fog = self.owner_fog(&content)?;
                    let field = value.field("fog");
                    if !matches!(field.value, Some(SaveJson::Null)) {
                        let Some(fog_state) = fog.as_mut() else {
                            return Err(value.fail("Owned fog requires a Q1 map").into());
                        };
                        let owned = fog_save_value(field.value.expect("fog value checked"));
                        let reader = qa_world::save::value::SaveReader::new(&owned);
                        let restored = fog_state
                            .restore(&reader, &|saved| reference(&saved))
                            .map_err(|error| PresentationStateError::Fog(error.to_string()))?;
                        for event in restored {
                            let mut event = local_fog_event(event);
                            event.owner = Some(token.clone());
                            self.source.push(event);
                        }
                    }
                    self.owners.insert(
                        token.provider.clone(),
                        OwnerEntry {
                            token,
                            content,
                            fog,
                            status: OwnerStatus::Restored,
                        },
                    );
                    Ok(())
                })?;
        }
        let fog = reader.field("q1Fog");
        match fog.value {
            Some(value) if !matches!(value, SaveJson::Null) => {
                let Some(fog_state) = self.fog.as_mut() else {
                    return Err(fog.fail("Q1 fog state requires a Q1 map").into());
                };
                let owned = fog_save_value(value);
                let reader = qa_world::save::value::SaveReader::new(&owned);
                let restored = fog_state
                    .restore(&reader, &|saved| reference(&saved))
                    .map_err(|error| PresentationStateError::Fog(error.to_string()))?;
                self.source.extend(restored.into_iter().map(local_fog_event));
            }
            _ => {
                if let Some(fog_state) = self.fog.as_mut() {
                    fog_state.reset();
                }
            }
        }
        reader
            .field("styles")
            .list(|value| -> Result<(), PresentationStateError> {
                self.styles.insert(
                    i32::try_from(value.field("style").integer(0)?)
                        .map_err(|_| value.fail("expected an integer in range"))?,
                    LightstyleValue {
                        family: value.field("family").choice_str(&["q1", "q2"])?,
                        pattern: value.field("pattern").string()?,
                    },
                );
                Ok(())
            })?;
        if reader.field("baseStyles").value.is_some() {
            reader
                .field("baseStyles")
                .list(|value| -> Result<(), PresentationStateError> {
                    self.legacy_styles.insert(
                        i32::try_from(value.field("style").integer(0)?)
                            .map_err(|_| value.fail("expected an integer in range"))?,
                        LightstyleValue {
                            family: value.field("family").choice_str(&["q1", "q2"])?,
                            pattern: value.field("pattern").string()?,
                        },
                    );
                    Ok(())
                })?;
        }
        if self.legacy_persistence {
            for (style, value) in self.styles.clone() {
                self.legacy_styles.insert(style, value);
            }
        }
        reader
            .field("persistent")
            .list(|value| -> Result<(), PresentationStateError> { self.restore_persistent(&value, reference) })?;
        self.source.sort_by_key(|event| event.sequence);
        Ok(())
    }

    /// Restore one persistent event (donor persistent list).
    fn restore_persistent(
        &mut self,
        value: &SaveReader,
        reference: &dyn Fn(&SavedActorId) -> ActorId,
    ) -> Result<(), PresentationStateError> {
        let event = value.field("event");
        let family =
            value
                .field("kind")
                .choice_str(&["q1", "q2", "music", "q1-sky", "q1-client", "q1-level", "local-media"])?;
        let kind = event.field("kind").choice_str(&[
            "ambient",
            "music",
            "sound",
            "static-model",
            "finale",
            "cd-track",
            "pause",
            "lightstyle",
            "skybox",
            "name",
            "social",
            "player-info",
            "colors",
            "frags",
            "ping",
            "music-stop",
            "shader-remap",
        ])?;
        let recipient = value.field("recipient");
        let owner = if value.field("owner").value.is_none() {
            None
        } else {
            Some(read_presentation_owner(&value.field("owner"))?)
        };
        if let Some(owner) = &owner {
            if !same_presentation_owner(self.owners.get(&owner.provider).map(|entry| &entry.token), owner) {
                return Err(value.fail("Persistent event has no saved presentation owner").into());
            }
        }
        let recipient = if recipient.value.is_none() || matches!(recipient.value, Some(SaveJson::Null)) {
            None
        } else {
            Some(reference(&read_saved_actor(&recipient)?))
        };
        let content = read_content_id(&value.field("content"))?;
        let sequence = value.field("sequence").integer(0)?;
        let seconds = value.field("seconds").number()?;
        if family == "local-media" {
            if recipient.is_some() || (kind != "music" && kind != "music-stop" && kind != "shader-remap") {
                return Err(value.fail("Invalid local presentation media request").into());
            }
            let media = match kind.as_str() {
                "music-stop" => MediaRequest::MusicStop,
                "shader-remap" => MediaRequest::ShaderRemap {
                    original: event.field("original").string()?,
                    replacement: event.field("replacement").string()?,
                    time_offset: event.field("timeOffset").number()?,
                },
                _ => MediaRequest::Music {
                    intro: event.field("intro").string()?,
                    loop_track: event.field("loop").string()?,
                },
            };
            let restored = RetainedPresentation::Local(LocalPresentationMedia {
                event: media,
                owner: owner.clone(),
                content: content.clone(),
                sequence,
                seconds,
            });
            if owner
                .as_ref()
                .is_some_and(|owner| self.owners.get(&owner.provider).map(|entry| &entry.content) != Some(&content))
                || sequence >= self.presentation_sequence
            {
                return Err(value
                    .fail("Invalid local presentation media content or sequence")
                    .into());
            }
            if let Some(slot) = persistent_slot(&restored) {
                self.persistent.insert(
                    format!("local:{}:{slot}", presentation_owner_key(restored.owner())),
                    restored,
                );
            }
            return Ok(());
        }
        let source_entity = value
            .field("sourceEntity")
            .nullable(|slot| -> Result<i32, PresentationStateError> {
                Ok(i32::try_from(slot.integer(0)?).map_err(|_| slot.fail("expected an integer in range"))?)
            })?;
        let base = |source: SourcePresentationEvent<F>| SimulationPresentationEvent {
            source,
            owner: owner.clone(),
            recipient: recipient.clone(),
            sequence,
            content: content.clone(),
            seconds,
            source_entity,
        };
        let restored = if family == "music" && kind == "cd-track" {
            base(SourcePresentationEvent::Music(MusicEvent::CdTrack {
                track: i32::try_from(event.field("track").integer(0)?)
                    .map_err(|_| event.fail("expected an integer in range"))?,
            }))
        } else if family == "q1-sky" && kind == "skybox" {
            base(SourcePresentationEvent::Q1Sky {
                name: event.field("name").string()?,
            })
        } else if family == "q1-client" && (kind == "name" || kind == "social" || kind == "player-info") {
            let slot = source_client_integer(&event.field("slot"), 0, 255)?;
            let value = event.field("value").string()?;
            base(SourcePresentationEvent::Q1Client(match kind.as_str() {
                "name" => Q1ClientMetadata::Name { slot, value },
                "social" => Q1ClientMetadata::Social { slot, value },
                _ => Q1ClientMetadata::PlayerInfo { slot, value },
            }))
        } else if family == "q1-client" && (kind == "colors" || kind == "frags" || kind == "ping") {
            let slot = source_client_integer(&event.field("slot"), 0, 255)?;
            let value = source_client_integer(&event.field("value"), -32768, 32767)?;
            base(SourcePresentationEvent::Q1Client(match kind.as_str() {
                "colors" => Q1ClientMetadata::Colors { slot, value },
                "frags" => Q1ClientMetadata::Frags { slot, value },
                _ => Q1ClientMetadata::Ping { slot, value },
            }))
        } else if family == "music" && kind == "pause" {
            base(SourcePresentationEvent::Music(MusicEvent::Pause {
                paused: event.field("paused").boolean()?,
            }))
        } else if (family == "q1" || family == "q2") && kind == "lightstyle" {
            let style = i32::try_from(event.field("style").integer(0)?)
                .map_err(|_| event.fail("expected an integer in range"))?;
            let pattern = event.field("pattern").string()?;
            base(if family == "q1" {
                SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle { style, pattern })
            } else {
                SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle { style, pattern })
            })
        } else if family == "q1-level" && kind == "finale" {
            base(SourcePresentationEvent::Q1Level(Q1LevelEvent::Finale {
                text: event.field("text").string()?,
                track: i32::try_from(event.field("track").integer(0)?)
                    .map_err(|_| event.fail("expected an integer in range"))?,
            }))
        } else if family == "q1" && kind == "finale" {
            base(SourcePresentationEvent::Q1(Q1PresentationEvent::Finale {
                text: event.field("text").string()?,
                stage: i32::try_from(event.field("stage").choice_i64(&[1, 2, 3, 4, 5, 6])?)
                    .map_err(|_| event.fail("expected an integer in range"))?,
            }))
        } else if family == "q1" && kind == "ambient" {
            base(SourcePresentationEvent::Q1(Q1PresentationEvent::Ambient {
                origin: read_vector(event.field("origin"))?,
                path: event.field("path").string()?,
                volume: event.field("volume").number()?,
                attenuation: event.field("attenuation").number()?,
            }))
        } else if family == "q1" && kind == "static-model" {
            base(SourcePresentationEvent::Q1(Q1PresentationEvent::StaticModel {
                path: event.field("path").string()?,
                frame: event.field("frame").integer(i64::MIN)? as f64,
                color_map: event.field("colorMap").integer(i64::MIN)? as f64,
                skin: event.field("skin").integer(i64::MIN)? as f64,
                origin: read_vector(event.field("origin"))?,
                angles: read_vector(event.field("angles"))?,
            }))
        } else if family == "q2" && kind == "music" {
            base(SourcePresentationEvent::Q2(Q2PresentationEvent::Music {
                track: event.field("track").string()?,
            }))
        } else if family == "q2" && kind == "sound" {
            base(SourcePresentationEvent::Q2(Q2PresentationEvent::Sound {
                actor: event
                    .field("actor")
                    .nullable(|actor| -> Result<ActorId, PresentationStateError> {
                        Ok(reference(&read_saved_actor(&actor)?))
                    })?,
                origin: read_vector(event.field("origin"))?,
                path: event.field("path").string()?,
                channel: event.field("channel").number()?,
                volume: event.field("volume").number()?,
                attenuation: event.field("attenuation").number()?,
                reliable: event.field("reliable").boolean()?,
                loop_mode: {
                    event.field("loop").literal_str("start")?;
                    Q2SoundLoop::Start
                },
                loop_owner: if event.field("loopOwner").value.is_none() {
                    None
                } else {
                    Some(read_provider(&event.field("loopOwner"))?)
                },
            }))
        } else {
            return Err(event.fail("Invalid persistent source event family").into());
        };
        let retained = RetainedPresentation::Source(restored.clone());
        let key = match persistent_slot(&retained) {
            Some(slot) => format!("{}:{slot}", presentation_owner_key(owner.as_ref())),
            None => format!(
                "{}:{}",
                presentation_owner_key(owner.as_ref()),
                match &restored.source {
                    SourcePresentationEvent::Q2(Q2PresentationEvent::Sound {
                        actor,
                        channel,
                        path,
                        loop_owner,
                        ..
                    }) => q2_loop_key(
                        actor.as_ref(),
                        *channel,
                        path,
                        loop_owner.as_ref(),
                        restored.recipient.as_ref(),
                    ),
                    _ => format!("{kind}:{sequence}"),
                }
            ),
        };
        self.persistent.insert(key, retained);
        self.source.push(restored);
        Ok(())
    }

    /// Read a lightstyle pattern (donor `lightStyle`).
    pub fn light_style(&self, style: i32) -> &str {
        self.styles
            .get(&style)
            .map(|value| value.pattern.as_str())
            .unwrap_or("")
    }

    /// Evaluate lightstyles (donor `lightStyles`).
    pub fn light_styles(&self, seconds: f64) -> Vec<SceneLightStyle> {
        self.styles
            .iter()
            .map(|(style, value)| {
                let letter = if value.pattern.is_empty() {
                    12
                } else {
                    i32::from(value.pattern.as_bytes()[(seconds * 10.0).floor() as usize % value.pattern.len()]) - 97
                };
                let scale = f64::from(letter) / 12.0;
                if value.family == "q1" {
                    SceneLightStyle::Q1 {
                        style: *style,
                        value: if value.pattern.is_empty() { 256 } else { letter * 22 },
                    }
                } else {
                    SceneLightStyle::Q2 {
                        style: *style,
                        rgb: vec3(scale as f32, scale as f32, scale as f32),
                        white: scale * 3.0,
                    }
                }
            })
            .collect()
    }
}

/// Write a presentation owner (donor token spread).
fn write_owner(owner: &PresentationOwner) -> SaveJson {
    obj(vec![
        ("provider", write_provider(&owner.provider)),
        ("generation", int(owner.generation as i64)),
    ])
}

/// Write a media request (donor local `event`).
fn write_media_request(event: &MediaRequest) -> SaveJson {
    match event {
        MediaRequest::Music { intro, loop_track } => obj(vec![
            ("kind", str("music")),
            ("intro", str(intro)),
            ("loop", str(loop_track)),
        ]),
        MediaRequest::MusicStop => obj(vec![("kind", str("music-stop"))]),
        MediaRequest::ShaderRemap {
            original,
            replacement,
            time_offset,
        } => obj(vec![
            ("kind", str("shader-remap")),
            ("original", str(original)),
            ("replacement", str(replacement)),
            ("timeOffset", num(*time_offset)),
        ]),
    }
}

/// Write client metadata (donor `q1-client` event).
fn write_client_metadata(metadata: &Q1ClientMetadata) -> SaveJson {
    match metadata {
        Q1ClientMetadata::Name { slot, value }
        | Q1ClientMetadata::Social { slot, value }
        | Q1ClientMetadata::PlayerInfo { slot, value } => obj(vec![
            ("kind", str(metadata.kind())),
            ("slot", int(i64::from(*slot))),
            ("value", str(value)),
        ]),
        Q1ClientMetadata::Colors { slot, value }
        | Q1ClientMetadata::Frags { slot, value }
        | Q1ClientMetadata::Ping { slot, value } => obj(vec![
            ("kind", str(metadata.kind())),
            ("slot", int(i64::from(*slot))),
            ("value", int(i64::from(*value))),
        ]),
    }
}

/// Read a bounded source client integer (donor `sourceClientInteger`).
fn source_client_integer(reader: &SaveReader, minimum: i64, maximum: i64) -> Result<i32, PresentationStateError> {
    let value = reader.integer(minimum)?;
    if value > maximum {
        return Err(reader.fail("source client value out of range").into());
    }
    Ok(i32::try_from(value).map_err(|_| reader.fail("source client value out of range"))?)
}

/// Read a presentation owner (donor `readPresentationOwner`).
fn read_presentation_owner(reader: &SaveReader) -> Result<PresentationOwner, PresentationStateError> {
    let generation = u64::try_from(reader.field("generation").integer(1)?)
        .map_err(|_| reader.fail("Invalid presentation owner generation"))?;
    if generation > MAX_OWNER_GENERATION {
        return Err(reader.fail("Invalid presentation owner generation").into());
    }
    Ok(PresentationOwner {
        provider: read_provider(&reader.field("provider"))?,
        generation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    fn harness(source_slot: bool) -> (PresentationState<()>, IdentityOwner) {
        let owner = IdentityOwner::create("presentation-state").unwrap();
        let state = PresentationState::new(
            || SourceTime::Seconds(2.0),
            move |actor: &ActorId| source_slot.then(|| actor.slot() as i32),
            None,
        )
        .unwrap();
        (state, owner)
    }

    fn content() -> ContentId {
        ContentId("q1:classic:id1:1".to_string())
    }

    fn provider() -> ProviderId {
        ProviderId::new("test", "ps")
    }

    fn lightstyle(style: i32, pattern: &str) -> SourcePresentationEvent<()> {
        SourcePresentationEvent::Q1(Q1PresentationEvent::Lightstyle {
            style,
            pattern: pattern.to_string(),
        })
    }

    #[test]
    fn emit_assigns_sequences_and_source_entities() {
        let (mut state, owner) = harness(true);
        let actor = owner.actor(3, 1);
        state.emit(&content(), lightstyle(1, "a"), None, None);
        state.emit(
            &content(),
            SourcePresentationEvent::ViewReset {
                reason: ViewResetReason::Spawn,
                actor: actor.clone(),
                angles: vec3(0.0, 0.0, 0.0),
            },
            Some(SourceTime::Milliseconds(2500)),
            Some(&actor),
        );
        let taken = state.take_presentation();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].sequence, 0);
        assert_eq!(taken[0].seconds, 2.0);
        assert_eq!(taken[0].source_entity, None);
        assert_eq!(taken[1].sequence, 1);
        assert_eq!(taken[1].seconds, 2.5);
        assert_eq!(taken[1].source_entity, Some(3));
        assert_eq!(taken[1].recipient, Some(actor));
        assert!(state.take_presentation().is_empty());
    }

    #[test]
    fn static_model_truncates() {
        let (mut state, _) = harness(false);
        state.emit(
            &content(),
            SourcePresentationEvent::Q1(Q1PresentationEvent::StaticModel {
                path: "progs/x.mdl".to_string(),
                frame: 3.9,
                color_map: 1.2,
                skin: 0.7,
                origin: vec3(1.0, 2.0, 3.0),
                angles: vec3(0.0, 0.0, 0.0),
            }),
            None,
            None,
        );
        let taken = state.take_presentation();
        assert!(matches!(
            &taken[0].source,
            SourcePresentationEvent::Q1(Q1PresentationEvent::StaticModel { frame, color_map, skin, .. })
                if *frame == 3.0 && *color_map == 1.0 && *skin == 0.0
        ));
    }

    #[test]
    fn lightstyles_track_and_evaluate() {
        let (mut state, _) = harness(false);
        state.emit(&content(), lightstyle(1, "ab"), None, None);
        state.emit(
            &content(),
            SourcePresentationEvent::Q2(Q2PresentationEvent::Lightstyle {
                style: 2,
                pattern: "m".to_string(),
            }),
            None,
            None,
        );
        assert_eq!(state.light_style(1), "ab");
        assert_eq!(state.light_style(9), "");
        let styles = state.light_styles(0.0);
        assert_eq!(styles.len(), 2);
        assert!(matches!(&styles[0], SceneLightStyle::Q1 { style: 1, value: 0 }));
        assert!(matches!(
            &styles[1],
            SceneLightStyle::Q2 { style: 2, white, .. } if (*white - 3.0).abs() < 1e-9
        ));
        let later = state.light_styles(0.15);
        assert!(matches!(&later[0], SceneLightStyle::Q1 { value: 22, .. }));
    }

    #[test]
    fn bind_owner_lifecycle() {
        let (mut state, _) = harness(false);
        let mut handle = state.bind_owner(provider(), content(), false).unwrap();
        assert_eq!(handle.owner().generation, 1);
        handle.emit(&content(), lightstyle(1, "a"), None, None).unwrap();
        assert_eq!(
            handle.emit(
                &content(),
                SourcePresentationEvent::PresentationOwner {
                    event: OwnerLifecycle::Refreshed {
                        owner: handle.owner().clone()
                    },
                },
                None,
                None,
            ),
            Err(PresentationStateError::OwnerLifecycle)
        );
        handle.close();
        assert_eq!(
            handle.emit(&content(), lightstyle(1, "a"), None, None),
            Err(PresentationStateError::OwnerRetired("test:ps".to_string()))
        );
        let taken = state.take_presentation();
        assert!(taken.iter().any(|event| matches!(
            &event.source,
            SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Retired { .. }
            }
        )));
        assert!(state.persistent_presentation().is_empty());
    }

    #[test]
    fn double_bind_rejected() {
        let (mut state, _) = harness(false);
        let handle = state.bind_owner(provider(), content(), false).unwrap();
        drop(handle);
        assert_eq!(
            state.bind_owner(provider(), content(), false).map(|_| ()),
            Err(PresentationStateError::OwnerActive("test:ps".to_string()))
        );
    }

    #[test]
    fn publish_acknowledge_local_media() {
        let (mut state, _) = harness(false);
        state
            .publish_local_media(
                None,
                &content(),
                MediaRequest::Music {
                    intro: "a".to_string(),
                    loop_track: "b".to_string(),
                },
                false,
                None::<fn() -> bool>,
            )
            .unwrap();
        state
            .publish_local_media(
                None,
                &content(),
                MediaRequest::ShaderRemap {
                    original: "x".to_string(),
                    replacement: "y".to_string(),
                    time_offset: 0.0,
                },
                false,
                None::<fn() -> bool>,
            )
            .unwrap();
        let pending = state.pending_local_media();
        assert_eq!(pending.len(), 2);
        assert!(state.local_media_current(&pending[0]));
        state.acknowledge_local_media(&pending[0], true);
        assert_eq!(state.pending_local_media().len(), 1);
        assert!(!state.local_media_current(&pending[0]));
        state.acknowledge_local_media(&pending[1], true);
        assert!(state.pending_local_media().is_empty());
        assert!(state.assert_local_media_consumed().is_ok());
        assert_eq!(state.persistent_presentation().len(), 0);
    }

    #[test]
    fn shader_replay_resolves_identity_without_winner() {
        let (mut state, _) = harness(false);
        let handle = state.bind_owner(provider(), content(), false).unwrap();
        let token = handle.owner().clone();
        drop(handle);
        state
            .publish_local_media(
                Some(&token),
                &content(),
                MediaRequest::ShaderRemap {
                    original: "maps/x".to_string(),
                    replacement: "maps/y".to_string(),
                    time_offset: 1.0,
                },
                false,
                None::<fn() -> bool>,
            )
            .unwrap();
        let pending = state.pending_local_media();
        state.acknowledge_local_media(&pending[0], true);
        state.retire_replicated_owner(&token);
        let pending = state.pending_local_media();
        assert_eq!(pending.len(), 1);
        let resolved = state.resolve_shader_replay(&pending[0]).unwrap().unwrap();
        match resolved.as_ref() {
            RetainedPresentation::Local(local) => match &local.event {
                MediaRequest::ShaderRemap {
                    original,
                    replacement,
                    time_offset,
                } => {
                    assert_eq!(original, "maps/x");
                    assert_eq!(replacement, "maps/x");
                    assert_eq!(*time_offset, 0.0);
                }
                _ => panic!("expected shader remap"),
            },
            _ => panic!("expected local media"),
        }
    }

    #[test]
    fn capture_restore_round_trip() {
        let (mut state, owner) = harness(false);
        let actor = owner.actor(0, 2);
        let mut handle = state.bind_owner(provider(), content(), false).unwrap();
        let token = handle.owner().clone();
        handle.emit(&content(), lightstyle(4, "z"), None, Some(&actor)).unwrap();
        handle.emit(&content(), lightstyle(5, "y"), None, None).unwrap();
        drop(handle);
        state
            .publish_local_media(
                None,
                &content(),
                MediaRequest::Music {
                    intro: "a".to_string(),
                    loop_track: "b".to_string(),
                },
                false,
                None::<fn() -> bool>,
            )
            .unwrap();
        let save = state.capture().unwrap();
        let (mut restored, _) = harness(false);
        let reference = |saved: &SavedActorId| {
            assert_eq!((saved.slot, saved.generation), (0, 2));
            actor.clone()
        };
        restored.restore(SaveReader::new(&save), &reference).unwrap();
        assert_eq!(
            restored.finish_owner_restore(),
            Err(PresentationStateError::OwnerUnrestored("test:ps".to_string()))
        );
        {
            let handle = restored.bind_owner(provider(), content(), true).unwrap();
            assert_eq!(handle.owner().generation, token.generation);
        }
        assert!(restored.finish_owner_restore().is_ok());
        let persistent = restored.persistent_presentation();
        assert_eq!(persistent.len(), 2);
        assert_eq!(persistent[0].recipient, Some(actor.clone()));
        assert_eq!(restored.light_style(4), "");
        assert_eq!(restored.light_style(5), "y");
        restored.enable_local_media();
        assert_eq!(restored.pending_local_media().len(), 1);
    }

    #[test]
    fn receive_presentation_retires_owner() {
        let (mut state, _) = harness(false);
        let handle = state.bind_owner(provider(), content(), false).unwrap();
        let token = handle.owner().clone();
        drop(handle);
        state.receive_presentation(SimulationPresentationEvent {
            source: SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Retired { owner: token.clone() },
            },
            owner: None,
            recipient: None,
            sequence: 99,
            content: content(),
            seconds: 0.0,
            source_entity: None,
        });
        assert_eq!(
            state.refresh_owner(&provider()),
            Err(PresentationStateError::OwnerInactive("test:ps".to_string()))
        );
        let taken = state.take_presentation();
        assert!(taken.iter().any(|event| matches!(
            &event.source,
            SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Retired { .. }
            }
        )));
    }

    #[test]
    fn q2_loop_start_stop() {
        let (mut state, owner) = harness(false);
        let actor = owner.actor(1, 1);
        let sound = |loop_mode| {
            SourcePresentationEvent::Q2(Q2PresentationEvent::Sound {
                actor: Some(actor.clone()),
                origin: vec3(0.0, 0.0, 0.0),
                path: "weapons/x.wav".to_string(),
                channel: 1.0,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_mode,
                loop_owner: None,
            })
        };
        state.emit(&content(), sound(Q2SoundLoop::Start), None, None);
        state.emit(&content(), sound(Q2SoundLoop::Start), None, None);
        assert_eq!(state.persistent_presentation().len(), 1);
        state.emit(&content(), sound(Q2SoundLoop::Stop), None, None);
        assert!(state.persistent_presentation().is_empty());
        state.emit(&content(), sound(Q2SoundLoop::Once), None, None);
        assert!(state.persistent_presentation().is_empty());
    }

    #[test]
    fn retire_actor_drops_recipient_events() {
        let (mut state, owner) = harness(false);
        let actor = owner.actor(2, 1);
        state.emit(&content(), lightstyle(7, "q"), None, Some(&actor));
        assert_eq!(state.persistent_presentation().len(), 1);
        state.retire(&actor);
        assert!(state.persistent_presentation().is_empty());
    }

    fn fog_options() -> SimulationQ1FogOptions {
        SimulationQ1FogOptions {
            content: content(),
            accepted_contents: None,
            entities: "{\n\"_fog\" \"4 0.1 0.2 0.3\"\n}\n".to_string(),
            alive: Box::new(|_| true),
        }
    }

    fn fogged() -> PresentationState<()> {
        PresentationState::new(|| SourceTime::Seconds(0.0), |_: &ActorId| None, Some(fog_options())).unwrap()
    }

    #[test]
    fn fog_update_output_flows_through_record() {
        let mut state = fogged();
        let mut handle = state.bind_owner(provider(), content(), false).unwrap();
        handle
            .emit(
                &content(),
                SourcePresentationEvent::Q1Composition(Q1CompositionEvent::Addon(Q1AddonEvent::Fog(Q1FogFields {
                    player: None,
                    density: 1.0,
                    color: vec3(0.0, 0.0, 0.0),
                    sky_factor: 0.5,
                    duration: 1.0,
                }))),
                None,
                None,
            )
            .unwrap();
        drop(handle);
        let taken = state.take_presentation();
        assert_eq!(taken.len(), 2);
        assert!(matches!(taken[1].source, SourcePresentationEvent::Q1Fog { .. }));
        assert!(taken[1].owner.is_some());
        let save = state.capture().unwrap();
        let (mut fogless, _) = harness(false);
        let actor = IdentityOwner::create("presentation-fog-restore").unwrap().actor(0, 1);
        assert!(fogless
            .restore(SaveReader::new(&save), &|_: &SavedActorId| actor.clone())
            .is_err());
        let mut restored = fogged();
        restored
            .restore(SaveReader::new(&save), &|_: &SavedActorId| actor.clone())
            .unwrap();
        let taken = restored.take_presentation();
        assert_eq!(taken.len(), 1);
        assert!(matches!(taken[0].source, SourcePresentationEvent::Q1Fog { .. }));
        assert!(taken[0].owner.is_some());
    }

    #[test]
    fn fog_retire_drops_actor_transitions() {
        let owner = IdentityOwner::create("presentation-fog-retire").unwrap();
        let actor = owner.actor(0, 1);
        let mut state = fogged();
        let mut handle = state.bind_owner(provider(), content(), false).unwrap();
        handle
            .emit(
                &content(),
                SourcePresentationEvent::Q1Composition(Q1CompositionEvent::Addon(Q1AddonEvent::Fog(Q1FogFields {
                    player: Some(actor.clone()),
                    density: 0.5,
                    color: vec3(0.1, 0.2, 0.3),
                    sky_factor: 0.7,
                    duration: 2.0,
                }))),
                None,
                None,
            )
            .unwrap();
        drop(handle);
        let taken = state.take_presentation();
        assert_eq!(taken.len(), 2);
        assert!(matches!(
            taken[1].source,
            SourcePresentationEvent::Q1Fog { player: Some(_), .. }
        ));
        state.retire(&actor);
        state.take_presentation();
        let save = state.capture().unwrap();
        let mut restored = fogged();
        let reference = IdentityOwner::create("presentation-fog-retire-restore")
            .unwrap()
            .actor(0, 1);
        restored
            .restore(SaveReader::new(&save), &|_: &SavedActorId| reference.clone())
            .unwrap();
        let taken = restored.take_presentation();
        assert!(taken
            .iter()
            .all(|event| !matches!(event.source, SourcePresentationEvent::Q1Fog { player: Some(_), .. })));
    }
}
