//! Client-owned QuakeC service state and local message presentation.
//!
//! Provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/quakec-local-messages.ts`.

use std::collections::HashMap;

use qa_content::q1::composition::types::{Q1CompositionEvent, Q1CompositionPromptChoice};
use qa_content::q1::foundation::types::{Q1Effect, Q1Event, Q1SoundChannel};
use qa_core::identity::ActorId;
use qa_core::math::{vec3, Vec3};
use qa_net::q1_net::{NetQuakeMessage, NqNamedSlot, NqNumberedSlot, NqText, NqUnit, NqValued};

use super::types::{PlayerView, Q1ClientMetadataEvent, Q1ClientNumericKind, Q1ClientStringKind};

/// Mirror of `NetworkEvent` from donor `src/contracts/protocol.ts`
/// (canonical home: net-lane protocol port); unify post-merge.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkEvent {
    /// Print a line.
    Print {
        /// Print level.
        level: i32,
        /// Text.
        text: String,
    },
    /// Center-print text.
    CenterPrint {
        /// Text.
        text: String,
    },
    /// Client command text.
    CommandText {
        /// Text.
        text: String,
    },
    /// Config string.
    ConfigString {
        /// Index.
        index: i32,
        /// Value.
        value: String,
    },
    /// Positioned sound.
    Sound {
        /// Entity number.
        entity_number: i32,
        /// Channel.
        channel: i32,
        /// Sound index.
        sound_index: i32,
        /// Explicit origin.
        origin: Option<Vec3>,
        /// Volume.
        volume: f64,
        /// Attenuation.
        attenuation: f64,
        /// Delay in seconds.
        delay_seconds: f64,
    },
    /// Quake damage feedback.
    Q1Damage {
        /// Armor damage.
        armor: i32,
        /// Blood damage.
        blood: i32,
        /// Damage source.
        source: Vec3,
    },
    /// Quake particle burst.
    Q1Particle {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Count.
        count: i32,
        /// Color.
        color: i32,
    },
    /// Quake II layout program.
    Q2Layout {
        /// Program.
        program: String,
    },
    /// Quake II inventory counts.
    Q2Inventory {
        /// Counts.
        counts: Vec<f64>,
    },
    /// Quake II muzzle flash.
    Q2MuzzleFlash {
        /// Entity number.
        entity_number: i32,
        /// Flash.
        flash: i32,
        /// Monster flag.
        monster: bool,
    },
    /// Quake III server command.
    Q3ServerCommand {
        /// Sequence.
        sequence: i32,
        /// Text.
        text: String,
    },
    /// Disconnect.
    Disconnect {
        /// Reason.
        reason: String,
    },
}

/// Local QuakeC service failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuakeCLocalMessageError {
    /// Client stat index is out of range.
    #[error("Invalid NetQuake client stat")]
    InvalidStat,
    /// Client state is missing.
    #[error("Missing local QC client state")]
    MissingClient,
    /// Restored view recipient is invalid.
    #[error("Invalid retained QC view recipient")]
    InvalidViewRecipient,
    /// Restored client id is duplicated.
    #[error("Duplicate restored local QC client")]
    DuplicateClient,
    /// Sound channel is out of range.
    #[error("Invalid local QC sound channel")]
    InvalidSoundChannel,
    /// Message kind has no local presentation.
    #[error("Unsupported local QuakeC service {0}")]
    UnsupportedService(String),
    /// Service entity slot is not live.
    #[error("QC service actor {0} is not live")]
    ServiceActor(u16),
    /// Service sound index is unknown.
    #[error("Unknown QC service sound {0}")]
    ServiceSound(u16),
    /// Service model index is unknown.
    #[error("Unknown QC service model {0}")]
    ServiceModel(u16),
    /// Service camera actor is missing.
    #[error("Missing QC camera actor")]
    ServiceCamera,
}
fn as_vec3(value: [f64; 3]) -> Vec3 {
    vec3(value[0] as f32, value[1] as f32, value[2] as f32)
}

fn message_key(message: &NetQuakeMessage) -> Option<String> {
    match message {
        NetQuakeMessage::Text { kind, .. } => match kind {
            NqText::ServerVars => Some("server-vars".to_string()),
            NqText::Skybox => Some("skybox".to_string()),
            NqText::Finale | NqText::Cutscene => Some("intermission".to_string()),
            _ => None,
        },
        NetQuakeMessage::Valued { kind, .. } => match kind {
            NqValued::SetViews => Some("set-views".to_string()),
            NqValued::Sequence => Some("sequence".to_string()),
            NqValued::SpawnedMonster => None,
        },
        NetQuakeMessage::Unit(kind) => match kind {
            NqUnit::LevelCompleted => Some("level-completed".to_string()),
            NqUnit::BackToLobby => Some("back-to-lobby".to_string()),
            NqUnit::Intermission => Some("intermission".to_string()),
            _ => None,
        },
        NetQuakeMessage::Stat { index, .. } => Some(format!("stat:{index}")),
        NetQuakeMessage::LightStyle { index, .. } => Some(format!("light-style:{index}")),
        NetQuakeMessage::NamedSlot { kind, slot, .. } => {
            let kind = match kind {
                NqNamedSlot::Name => "name",
                NqNamedSlot::Social => "social",
                NqNamedSlot::PlayerInfo => "player-info",
            };
            Some(format!("{kind}:{slot}"))
        }
        NetQuakeMessage::NumberedSlot { kind, slot, .. } => {
            let kind = match kind {
                NqNumberedSlot::Frags => "frags",
                NqNumberedSlot::Colors => "colors",
                NqNumberedSlot::Ping => "ping",
            };
            Some(format!("{kind}:{slot}"))
        }
        NetQuakeMessage::SetView { .. } => Some("set-view".to_string()),
        NetQuakeMessage::SetAngle { .. } => Some("set-angle".to_string()),
        NetQuakeMessage::Pause { .. } => Some("pause".to_string()),
        NetQuakeMessage::CdTrack { .. } => Some("cd-track".to_string()),
        NetQuakeMessage::Fog { .. } => Some("fog".to_string()),
        _ => None,
    }
}

/// Client-owned service state, separate from the module's authoritative edict fields.
#[derive(Debug, Clone, Default)]
pub struct QuakeCLocalMessages {
    baseline: HashMap<String, NetQuakeMessage>,
    clients: HashMap<ActorId, HashMap<String, NetQuakeMessage>>,
    baseline_view: Option<ActorId>,
    views: HashMap<ActorId, ActorId>,
}

impl QuakeCLocalMessages {
    /// Create empty service state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn retained(
        state: &mut HashMap<String, NetQuakeMessage>,
        message: &NetQuakeMessage,
    ) -> Result<(), QuakeCLocalMessageError> {
        match message {
            NetQuakeMessage::PromptBegin { .. } | NetQuakeMessage::PromptClear => {
                state.retain(|key, _| !key.starts_with("prompt:"));
                state.insert("prompt:begin".to_string(), message.clone());
            }
            NetQuakeMessage::PromptChoice { .. } => {
                if matches!(state.get("prompt:begin"), Some(NetQuakeMessage::PromptBegin { text, .. }) if !text.is_empty())
                {
                    let count = state
                        .values()
                        .filter(|value| matches!(value, NetQuakeMessage::PromptChoice { .. }))
                        .count();
                    state.insert(format!("prompt:choice:{count}"), message.clone());
                }
            }
            NetQuakeMessage::Stat { index, .. } => {
                if *index >= 32 {
                    return Err(QuakeCLocalMessageError::InvalidStat);
                }
                state.insert(format!("stat:{index}"), message.clone());
            }
            NetQuakeMessage::Unit(unit @ (NqUnit::KilledMonster | NqUnit::FoundSecret)) => {
                let index = if *unit == NqUnit::KilledMonster { 14 } else { 13 };
                let old = match state.get(&format!("stat:{index}")) {
                    Some(NetQuakeMessage::Stat { value, .. }) => *value,
                    _ => 0,
                };
                state.insert(format!("stat:{index}"), NetQuakeMessage::Stat { index, value: old + 1 });
            }
            NetQuakeMessage::Valued {
                kind: NqValued::SpawnedMonster,
                value,
            } => {
                let old = match state.get("stat:12") {
                    Some(NetQuakeMessage::Stat { value, .. }) => *value,
                    _ => 0,
                };
                state.insert(
                    "stat:12".to_string(),
                    NetQuakeMessage::Stat {
                        index: 12,
                        value: old + *value,
                    },
                );
            }
            _ => {
                if let Some(key) = message_key(message) {
                    state.insert(key, message.clone());
                }
            }
        }
        Ok(())
    }

    /// Admit a client, seeding it with the baseline.
    pub fn admit(&mut self, actor: &ActorId) {
        if !self.clients.contains_key(actor) {
            self.clients.insert(actor.clone(), self.baseline.clone());
            if let Some(view) = &self.baseline_view {
                self.views.insert(actor.clone(), view.clone());
            }
        }
    }

    /// Whether a client is admitted.
    #[must_use]
    pub fn has_client(&self, actor: &ActorId) -> bool {
        self.clients.contains_key(actor)
    }

    /// Retire a client and its camera views.
    pub fn retire(&mut self, actor: &ActorId) {
        self.clients.remove(actor);
        self.views.remove(actor);
        if self.baseline_view.as_ref() == Some(actor) {
            self.baseline_view = None;
        }
        self.views.retain(|_, target| target != actor);
    }

    /// Receive messages for one client, or for the baseline and all clients.
    /// `view_target` is `None` when no camera update rides along, `Some(None)`
    /// to clear, and `Some(Some(_))` to retarget.
    pub fn receive(
        &mut self,
        messages: &[NetQuakeMessage],
        actor: Option<&ActorId>,
        view_target: Option<Option<ActorId>>,
    ) -> Result<(), QuakeCLocalMessageError> {
        if let Some(actor) = actor {
            self.admit(actor);
        }
        for message in messages {
            if matches!(message, NetQuakeMessage::SetView { .. }) {
                if let Some(target) = view_target.clone() {
                    match actor {
                        None => {
                            self.baseline_view = target.clone();
                            let recipients: Vec<ActorId> = self.clients.keys().cloned().collect();
                            for recipient in recipients {
                                match target.clone() {
                                    None => {
                                        self.views.remove(&recipient);
                                    }
                                    Some(target) => {
                                        self.views.insert(recipient, target);
                                    }
                                }
                            }
                        }
                        Some(actor) => match target {
                            None => {
                                self.views.remove(actor);
                            }
                            Some(target) => {
                                self.views.insert(actor.clone(), target);
                            }
                        },
                    }
                }
            }
        }
        match actor {
            None => {
                for message in messages {
                    Self::retained(&mut self.baseline, message)?;
                    for state in self.clients.values_mut() {
                        Self::retained(state, message)?;
                    }
                }
            }
            Some(actor) => {
                self.admit(actor);
                let state = self
                    .clients
                    .get_mut(actor)
                    .ok_or(QuakeCLocalMessageError::MissingClient)?;
                for message in messages {
                    Self::retained(state, message)?;
                }
            }
        }
        Ok(())
    }

    /// Retained client stat.
    #[must_use]
    pub fn stat(&self, actor: &ActorId, index: u8) -> Option<i32> {
        match self.clients.get(actor)?.get(&format!("stat:{index}")) {
            Some(NetQuakeMessage::Stat { value, .. }) => Some(*value),
            _ => None,
        }
    }

    /// Retained client view angles.
    #[must_use]
    pub fn angles(&self, actor: &ActorId) -> Option<[f64; 3]> {
        match self.clients.get(actor)?.get("set-angle") {
            Some(NetQuakeMessage::SetAngle { angles, .. }) => Some(*angles),
            _ => None,
        }
    }

    /// Retained client view entity.
    #[must_use]
    pub fn view(&self, actor: &ActorId) -> Option<u16> {
        match self.clients.get(actor)?.get("set-view") {
            Some(NetQuakeMessage::SetView { entity, .. }) => Some(*entity),
            _ => None,
        }
    }

    /// Retained client camera target.
    #[must_use]
    pub fn view_target(&self, actor: &ActorId) -> Option<&ActorId> {
        self.views.get(actor)
    }

    /// Capture retained camera views.
    #[must_use]
    pub fn capture_views(&self) -> QuakeCViewCapture {
        QuakeCViewCapture {
            baseline: self.baseline_view.clone(),
            clients: self
                .views
                .iter()
                .map(|(actor, target)| QuakeCViewClient {
                    actor: actor.clone(),
                    target: target.clone(),
                })
                .collect(),
        }
    }

    /// Restore retained camera views.
    pub fn restore_views(
        &mut self,
        baseline: Option<ActorId>,
        clients: &[QuakeCViewClient],
    ) -> Result<(), QuakeCLocalMessageError> {
        self.baseline_view = baseline;
        self.views.clear();
        for entry in clients {
            if !self.clients.contains_key(&entry.actor) || self.views.contains_key(&entry.actor) {
                return Err(QuakeCLocalMessageError::InvalidViewRecipient);
            }
            self.views.insert(entry.actor.clone(), entry.target.clone());
        }
        Ok(())
    }

    /// Retained client session state.
    #[must_use]
    pub fn session_state(&self, actor: &ActorId) -> QuakeCSessionState {
        let state = self.clients.get(actor);
        QuakeCSessionState {
            server_vars: match state.and_then(|state| state.get("server-vars")) {
                Some(NetQuakeMessage::Text { text, .. }) => text.clone(),
                _ => String::new(),
            },
            views: match state.and_then(|state| state.get("set-views")) {
                Some(NetQuakeMessage::Valued { value, .. }) => *value,
                _ => 1,
            },
            sequence: match state.and_then(|state| state.get("sequence")) {
                Some(NetQuakeMessage::Valued { value, .. }) => *value,
                _ => 0,
            },
            level_completed: state.is_some_and(|state| state.contains_key("level-completed")),
            back_to_lobby: state.is_some_and(|state| state.contains_key("back-to-lobby")),
        }
    }

    /// Retained client prompt.
    #[must_use]
    pub fn prompt(&self, actor: &ActorId) -> Q1CompositionEvent {
        let messages = self.clients.get(actor);
        match messages.and_then(|messages| messages.get("prompt:begin")) {
            Some(NetQuakeMessage::PromptBegin { text, .. }) if !text.is_empty() => {
                let choices = messages
                    .iter()
                    .flat_map(|messages| messages.values())
                    .filter_map(|message| match message {
                        NetQuakeMessage::PromptChoice { text, impulse } => Some(Q1CompositionPromptChoice {
                            label: text.clone(),
                            impulse: i32::from(*impulse),
                        }),
                        _ => None,
                    })
                    .collect();
                Q1CompositionEvent::Prompt {
                    actor: actor.clone(),
                    title: text.clone(),
                    choices,
                }
            }
            _ => Q1CompositionEvent::ClearPrompt { actor: actor.clone() },
        }
    }

    /// Answer a retained prompt, clearing it on a valid impulse.
    pub fn answer_prompt(&mut self, actor: &ActorId, impulse: u8) -> Result<bool, QuakeCLocalMessageError> {
        let valid = match self.prompt(actor) {
            Q1CompositionEvent::Prompt { choices, .. } => {
                choices.iter().any(|choice| choice.impulse == i32::from(impulse))
            }
            _ => false,
        };
        if !valid {
            return Ok(false);
        }
        self.receive(&[NetQuakeMessage::PromptClear], Some(actor), None)?;
        Ok(true)
    }

    /// Whether the client retains an intermission.
    #[must_use]
    pub fn intermission(&self, actor: &ActorId) -> bool {
        self.clients
            .get(actor)
            .is_some_and(|state| state.contains_key("intermission"))
    }

    /// Retained presentation messages for a client.
    #[must_use]
    pub fn presentation(&self, actor: &ActorId) -> Vec<NetQuakeMessage> {
        self.clients
            .get(actor)
            .map(|state| {
                state
                    .values()
                    .filter(|message| {
                        matches!(
                            message,
                            NetQuakeMessage::Text {
                                kind: NqText::Skybox | NqText::Finale | NqText::Cutscene,
                                ..
                            } | NetQuakeMessage::NamedSlot { .. }
                                | NetQuakeMessage::NumberedSlot { .. }
                                | NetQuakeMessage::PromptBegin { .. }
                                | NetQuakeMessage::PromptChoice { .. }
                                | NetQuakeMessage::PromptClear
                                | NetQuakeMessage::LightStyle { .. }
                                | NetQuakeMessage::Unit(NqUnit::Intermission)
                                | NetQuakeMessage::CdTrack { .. }
                                | NetQuakeMessage::Pause { .. }
                        )
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Capture retained state.
    #[must_use]
    pub fn capture(&self) -> QuakeCLocalMessageCapture {
        QuakeCLocalMessageCapture {
            baseline: self.baseline.values().cloned().collect(),
            clients: self
                .clients
                .iter()
                .map(|(actor, messages)| QuakeCLocalClientCapture {
                    actor: actor.clone(),
                    messages: messages.values().cloned().collect(),
                })
                .collect(),
        }
    }

    /// Restore retained state.
    pub fn restore(&mut self, capture: &QuakeCLocalMessageCapture) -> Result<(), QuakeCLocalMessageError> {
        self.baseline.clear();
        self.clients.clear();
        self.baseline_view = None;
        self.views.clear();
        for message in &capture.baseline {
            Self::retained(&mut self.baseline, message)?;
        }
        for entry in &capture.clients {
            if self.clients.contains_key(&entry.actor) {
                return Err(QuakeCLocalMessageError::DuplicateClient);
            }
            let mut state = HashMap::new();
            for message in &entry.messages {
                Self::retained(&mut state, message)?;
            }
            self.clients.insert(entry.actor.clone(), state);
        }
        Ok(())
    }
}

/// Retained camera views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeCViewCapture {
    /// Baseline view target.
    pub baseline: Option<ActorId>,
    /// Per-client view targets.
    pub clients: Vec<QuakeCViewClient>,
}

/// One retained client camera view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeCViewClient {
    /// Viewing actor.
    pub actor: ActorId,
    /// View target.
    pub target: ActorId,
}

/// Retained client session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuakeCSessionState {
    /// Server vars text.
    pub server_vars: String,
    /// View count.
    pub views: i32,
    /// Sequence.
    pub sequence: i32,
    /// Level completed.
    pub level_completed: bool,
    /// Back to lobby.
    pub back_to_lobby: bool,
}

/// Retained client messages.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCLocalClientCapture {
    /// Client actor.
    pub actor: ActorId,
    /// Retained messages.
    pub messages: Vec<NetQuakeMessage>,
}

/// Retained service state.
#[derive(Debug, Clone, PartialEq)]
pub struct QuakeCLocalMessageCapture {
    /// Baseline messages.
    pub baseline: Vec<NetQuakeMessage>,
    /// Per-client messages.
    pub clients: Vec<QuakeCLocalClientCapture>,
}
/// Body lookup for local camera views.
pub trait QuakeCLocalViewSource {
    /// Read an actor body origin and angles.
    fn read(&self, actor: &ActorId) -> Option<(Vec3, Vec3)>;
    /// Client view offset.
    fn offset(&self, actor: &ActorId) -> Vec3;
}

/// Source camera targets retain the generation captured by WriteEntity.
#[must_use]
pub fn quake_c_local_view(
    actor: &ActorId,
    state: &QuakeCLocalMessages,
    source: &dyn QuakeCLocalViewSource,
) -> Option<PlayerView> {
    let intermission = state.intermission(actor);
    let target = state.view_target(actor);
    if !intermission && (target.is_none() || target == Some(actor)) {
        return None;
    }
    let (origin, angles) = source.read(target.unwrap_or(actor))?;
    let offset = if intermission {
        vec3(0.0, 0.0, 0.0)
    } else {
        source.offset(actor)
    };
    Some(PlayerView {
        client_view_offset_delta: None,
        blend: None,
        damage_blend: None,
        origin: vec3(origin.x + offset.x, origin.y + offset.y, origin.z + offset.z),
        angles: state.angles(actor).map(as_vec3).unwrap_or(angles),
        view_height: f64::from(offset.z),
        kick_angles: None,
        field_of_view: None,
        foreign_character_death: false,
        pitch_drift: None,
    })
}

/// Session lifecycle signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCSessionKind {
    /// Level completed.
    LevelCompleted,
    /// Back to lobby.
    BackToLobby,
}

/// Retained fog message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuakeCLocalFog {
    /// Fog density.
    pub density: f64,
    /// Fog color.
    pub color: Vec3,
    /// Transition seconds.
    pub transition_seconds: f64,
}

/// Decoded source service host. Methods that resolve source-owned ids report
/// [`QuakeCLocalMessageError`] service failures from the host implementation.
pub trait QuakeCLocalMessageHost {
    /// Resolve a service entity slot.
    fn actor(&self, slot: u16) -> Result<ActorId, QuakeCLocalMessageError>;
    /// Broadcast recipients.
    fn recipients(&self) -> Vec<ActorId>;
    /// Source world actor.
    fn source_actor(&self) -> ActorId;
    /// Current map path.
    fn map(&self) -> String;
    /// Source time in seconds.
    fn seconds(&self) -> f64;
    /// Read a camera origin and angles.
    fn camera(&self, actor: &ActorId) -> Result<(Vec3, Vec3), QuakeCLocalMessageError>;
    /// Resolve a precached sound name.
    fn sound(&self, index: u16) -> Result<String, QuakeCLocalMessageError>;
    /// Resolve a precached model name.
    fn model(&self, index: u16) -> Result<String, QuakeCLocalMessageError>;
    /// Emit a presentation event.
    fn emit(&mut self, event: Q1Event, recipient: Option<ActorId>);
    /// Send a network event.
    fn message(&mut self, event: NetworkEvent, actor: Option<ActorId>);
    /// Play a music track.
    fn music(&mut self, track: u8);
    /// Reset client view angles.
    fn angles(&mut self, actor: &ActorId, angles: Vec3);
    /// Pause or resume.
    fn pause(&mut self, paused: bool);
    /// Set the skybox.
    fn sky(&mut self, name: &str, recipient: Option<ActorId>);
    /// Report client metadata.
    fn client_metadata(&mut self, event: Q1ClientMetadataEvent, recipient: Option<ActorId>);
    /// Report a session lifecycle signal.
    fn session(&mut self, kind: QuakeCSessionKind, recipient: Option<ActorId>);
    /// Present a prompt.
    fn prompt(&mut self, event: Q1CompositionEvent);
    /// Present fog.
    fn fog(&mut self, fog: QuakeCLocalFog, recipient: Option<ActorId>);
}

fn sound_channel(channel: u8) -> Result<Q1SoundChannel, QuakeCLocalMessageError> {
    match channel {
        0 => Ok(Q1SoundChannel::Auto),
        1 => Ok(Q1SoundChannel::Weapon),
        2 => Ok(Q1SoundChannel::Voice),
        3 => Ok(Q1SoundChannel::Item),
        4 => Ok(Q1SoundChannel::Body),
        5..=7 => Ok(Q1SoundChannel::Raw(i32::from(channel))),
        _ => Err(QuakeCLocalMessageError::InvalidSoundChannel),
    }
}

fn client_metadata(message: &NetQuakeMessage) -> Option<Q1ClientMetadataEvent> {
    match message {
        NetQuakeMessage::NamedSlot { kind, slot, value } => {
            let kind = match kind {
                NqNamedSlot::Name => Q1ClientStringKind::Name,
                NqNamedSlot::Social => Q1ClientStringKind::Social,
                NqNamedSlot::PlayerInfo => Q1ClientStringKind::PlayerInfo,
            };
            Some(Q1ClientMetadataEvent::String {
                kind,
                slot: u32::from(*slot),
                value: value.clone(),
            })
        }
        NetQuakeMessage::NumberedSlot { kind, slot, value } => {
            let kind = match kind {
                NqNumberedSlot::Frags => Q1ClientNumericKind::Frags,
                NqNumberedSlot::Colors => Q1ClientNumericKind::Colors,
                NqNumberedSlot::Ping => Q1ClientNumericKind::Ping,
            };
            Some(Q1ClientMetadataEvent::Numeric {
                kind,
                slot: u32::from(*slot),
                value: i32::from(*value),
            })
        }
        _ => None,
    }
}

/// Decoded source services use the same common presentation events as
/// source-authored TS games.
#[allow(clippy::too_many_lines)]
pub fn present_quake_c_local_message(
    message: &NetQuakeMessage,
    target: Option<&ActorId>,
    state: &QuakeCLocalMessages,
    host: &mut dyn QuakeCLocalMessageHost,
) -> Result<(), QuakeCLocalMessageError> {
    let actors: Vec<ActorId> = match target {
        None => host.recipients(),
        Some(actor) => vec![actor.clone()],
    };
    match message {
        NetQuakeMessage::Text {
            kind: NqText::ServerVars,
            ..
        }
        | NetQuakeMessage::Valued {
            kind: NqValued::SetViews | NqValued::Sequence,
            ..
        } => Ok(()),
        NetQuakeMessage::Unit(NqUnit::LevelCompleted) => {
            host.session(QuakeCSessionKind::LevelCompleted, target.cloned());
            Ok(())
        }
        NetQuakeMessage::Unit(NqUnit::BackToLobby) => {
            host.session(QuakeCSessionKind::BackToLobby, target.cloned());
            Ok(())
        }
        NetQuakeMessage::PromptBegin { .. } | NetQuakeMessage::PromptChoice { .. } | NetQuakeMessage::PromptClear => {
            for actor in &actors {
                host.prompt(state.prompt(actor));
            }
            Ok(())
        }
        NetQuakeMessage::Stat { index, value } => {
            if *index == 12 {
                host.emit(Q1Event::MonsterTotal { total: *value }, None);
            }
            Ok(())
        }
        NetQuakeMessage::Unit(NqUnit::Nop) | NetQuakeMessage::SetView { .. } => Ok(()),
        message @ (NetQuakeMessage::NamedSlot { .. } | NetQuakeMessage::NumberedSlot { .. }) => {
            if let Some(event) = client_metadata(message) {
                host.client_metadata(event, target.cloned());
            }
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::Skybox,
            text,
        } => {
            host.sky(text, target.cloned());
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::RawPrint | NqText::Chat | NqText::Botchat,
            text,
        } => {
            host.message(
                NetworkEvent::Print {
                    level: 2,
                    text: text.clone(),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::Print,
            text,
        } => {
            host.message(
                NetworkEvent::Print {
                    level: 2,
                    text: text.clone(),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::CenterPrint,
            text,
        } => {
            host.message(NetworkEvent::CenterPrint { text: text.clone() }, target.cloned());
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::Stufftext,
            text,
        } => {
            host.message(NetworkEvent::CommandText { text: text.clone() }, target.cloned());
            Ok(())
        }
        NetQuakeMessage::Unit(NqUnit::Disconnect) => {
            host.message(
                NetworkEvent::Disconnect {
                    reason: "Source disconnected client".to_string(),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::SetAngle { angles, .. } => {
            for actor in &actors {
                host.angles(actor, as_vec3(*angles));
            }
            Ok(())
        }
        NetQuakeMessage::LightStyle { index, value } => {
            host.emit(
                Q1Event::Lightstyle {
                    style: i32::from(*index),
                    pattern: value.clone(),
                },
                None,
            );
            Ok(())
        }
        NetQuakeMessage::CdTrack { track, .. } => {
            host.music(*track);
            Ok(())
        }
        NetQuakeMessage::Pause { paused, .. } => {
            host.pause(*paused);
            Ok(())
        }
        NetQuakeMessage::Fog {
            density,
            color,
            transition_seconds,
        } => {
            host.fog(
                QuakeCLocalFog {
                    density: *density,
                    color: as_vec3(*color),
                    transition_seconds: *transition_seconds,
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Unit(NqUnit::BonusFlash) => {
            for actor in &actors {
                let (origin, _) = host.camera(actor)?;
                host.emit(
                    Q1Event::Effect {
                        effect: Q1Effect::Pickup,
                        actor: Some(actor.clone()),
                        origin,
                        amount: 1,
                        muzzle: None,
                    },
                    Some(actor.clone()),
                );
            }
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::Achievement,
            text,
        } => {
            host.emit(
                Q1Event::Achievement {
                    player: target.cloned(),
                    id: text.clone(),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Valued {
            kind: NqValued::SpawnedMonster,
            ..
        } => {
            for actor in &actors {
                host.emit(
                    Q1Event::MonsterTotal {
                        total: state.stat(actor, 12).unwrap_or(0),
                    },
                    Some(actor.clone()),
                );
            }
            Ok(())
        }
        NetQuakeMessage::LocalSound { index } => {
            for actor in &actors {
                let listener = state.view_target(actor).unwrap_or(actor).clone();
                host.emit(
                    Q1Event::Sound {
                        origin: None,
                        actor: listener,
                        path: host.sound(*index)?,
                        channel: Q1SoundChannel::Raw(-1),
                        attenuation: 1.0,
                        volume: 1.0,
                    },
                    Some(actor.clone()),
                );
            }
            Ok(())
        }
        NetQuakeMessage::Damage { armor, blood, source } => {
            host.message(
                NetworkEvent::Q1Damage {
                    armor: i32::from(*armor),
                    blood: i32::from(*blood),
                    source: as_vec3(*source),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Particle {
            origin,
            direction,
            count,
            color,
        } => {
            host.message(
                NetworkEvent::Q1Particle {
                    origin: as_vec3(*origin),
                    direction: as_vec3(*direction),
                    count: i32::from(*count),
                    color: i32::from(*color),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Unit(unit @ (NqUnit::KilledMonster | NqUnit::FoundSecret)) => {
            let secret = *unit == NqUnit::FoundSecret;
            for actor in &actors {
                let (total, found) = if secret {
                    (state.stat(actor, 11).unwrap_or(0), state.stat(actor, 13).unwrap_or(0))
                } else {
                    (state.stat(actor, 12).unwrap_or(0), state.stat(actor, 14).unwrap_or(0))
                };
                host.emit(
                    if secret {
                        Q1Event::Secret {
                            actor: actor.clone(),
                            total,
                            found,
                        }
                    } else {
                        Q1Event::MonsterKilled {
                            actor: actor.clone(),
                            total,
                            found,
                        }
                    },
                    Some(actor.clone()),
                );
            }
            Ok(())
        }
        NetQuakeMessage::Unit(NqUnit::Intermission) => {
            for actor in &actors {
                let (origin, angles) = host.camera(actor)?;
                host.emit(
                    Q1Event::Intermission {
                        origin,
                        angles,
                        map: host.map(),
                        exit_after: host.seconds() + 5.0,
                        track: 0,
                    },
                    Some(actor.clone()),
                );
            }
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::Finale,
            text,
        } => {
            host.emit(
                Q1Event::Finale {
                    text: text.clone(),
                    stage: 4,
                },
                None,
            );
            Ok(())
        }
        NetQuakeMessage::Text {
            kind: NqText::Cutscene,
            text,
        } => {
            host.emit(
                Q1Event::Finale {
                    text: text.clone(),
                    stage: 1,
                },
                None,
            );
            Ok(())
        }
        NetQuakeMessage::Unit(NqUnit::SellScreen) => {
            host.message(
                NetworkEvent::CommandText {
                    text: "help\n".to_string(),
                },
                target.cloned(),
            );
            Ok(())
        }
        NetQuakeMessage::Sound {
            entity,
            channel,
            index,
            volume,
            attenuation,
            origin,
        } => {
            let path = host.sound(*index)?;
            host.emit(
                Q1Event::Sound {
                    origin: Some(as_vec3(*origin)),
                    actor: host.actor(*entity)?,
                    path,
                    channel: sound_channel(*channel)?,
                    attenuation: *attenuation,
                    volume: f64::from(*volume) / 255.0,
                },
                None,
            );
            Ok(())
        }
        NetQuakeMessage::StaticSound {
            index,
            volume,
            attenuation,
            origin,
        } => {
            host.emit(
                Q1Event::Ambient {
                    origin: as_vec3(*origin),
                    path: host.sound(*index)?,
                    volume: f64::from(*volume) / 255.0,
                    attenuation: *attenuation,
                },
                None,
            );
            Ok(())
        }
        NetQuakeMessage::StopSound { entity, channel } => {
            host.emit(
                Q1Event::StopSound {
                    actor: host.actor(*entity)?,
                    channel: i32::from(*channel),
                },
                None,
            );
            Ok(())
        }
        NetQuakeMessage::Static { state: entity } => {
            host.emit(
                Q1Event::StaticModel {
                    path: host.model(entity.state.modelindex)?,
                    frame: i32::from(entity.state.frame),
                    color_map: i32::from(entity.state.colormap),
                    skin: i32::from(entity.state.skin),
                    origin: as_vec3(entity.state.origin),
                    angles: as_vec3(entity.state.angles),
                },
                None,
            );
            Ok(())
        }
        // Owner-aware temporary effects are translated once by QcBroadcastMessages.
        NetQuakeMessage::TemporaryEntity { .. } => Ok(()),
        NetQuakeMessage::Signon { .. } => Err(QuakeCLocalMessageError::UnsupportedService("signon".to_string())),
        NetQuakeMessage::Time { .. } => Err(QuakeCLocalMessageError::UnsupportedService("time".to_string())),
        NetQuakeMessage::Version { .. } => Err(QuakeCLocalMessageError::UnsupportedService("version".to_string())),
        NetQuakeMessage::ServerInfo { .. } => {
            Err(QuakeCLocalMessageError::UnsupportedService("server-info".to_string()))
        }
        NetQuakeMessage::ClientData { .. } => {
            Err(QuakeCLocalMessageError::UnsupportedService("client-data".to_string()))
        }
        NetQuakeMessage::Entity { .. } => Err(QuakeCLocalMessageError::UnsupportedService("entity".to_string())),
        NetQuakeMessage::Baseline { .. } => Err(QuakeCLocalMessageError::UnsupportedService("baseline".to_string())),
    }
}
#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;

    fn owner() -> IdentityOwner {
        IdentityOwner::create("local-messages").unwrap()
    }

    fn stat(index: u8, value: i32) -> NetQuakeMessage {
        NetQuakeMessage::Stat { index, value }
    }

    #[test]
    fn baseline_seeds_admitted_clients() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut state = QuakeCLocalMessages::new();
        state.receive(&[stat(3, 25)], None, None).unwrap();
        assert!(!state.has_client(&actor));
        state.admit(&actor);
        assert_eq!(state.stat(&actor, 3), Some(25));
    }

    #[test]
    fn kill_counters_fold_into_stats() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut state = QuakeCLocalMessages::new();
        state
            .receive(
                &[
                    NetQuakeMessage::Unit(NqUnit::KilledMonster),
                    NetQuakeMessage::Unit(NqUnit::FoundSecret),
                    NetQuakeMessage::Valued {
                        kind: NqValued::SpawnedMonster,
                        value: 4,
                    },
                ],
                Some(&actor),
                None,
            )
            .unwrap();
        assert_eq!(state.stat(&actor, 14), Some(1));
        assert_eq!(state.stat(&actor, 13), Some(1));
        assert_eq!(state.stat(&actor, 12), Some(4));
    }

    #[test]
    fn rejects_out_of_range_stats() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut state = QuakeCLocalMessages::new();
        assert_eq!(
            state.receive(&[stat(32, 1)], Some(&actor), None),
            Err(QuakeCLocalMessageError::InvalidStat)
        );
    }

    #[test]
    fn prompts_clear_and_answer() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut state = QuakeCLocalMessages::new();
        state
            .receive(
                &[
                    NetQuakeMessage::PromptBegin {
                        text: "Team?".to_string(),
                        choices: 2,
                    },
                    NetQuakeMessage::PromptChoice {
                        text: "Red".to_string(),
                        impulse: 5,
                    },
                ],
                Some(&actor),
                None,
            )
            .unwrap();
        match state.prompt(&actor) {
            Q1CompositionEvent::Prompt { title, choices, .. } => {
                assert_eq!(title, "Team?");
                assert_eq!(choices.len(), 1);
            }
            other => panic!("unexpected prompt: {other:?}"),
        }
        assert!(!state.answer_prompt(&actor, 6).unwrap());
        assert!(state.answer_prompt(&actor, 5).unwrap());
        assert!(matches!(state.prompt(&actor), Q1CompositionEvent::ClearPrompt { .. }));
    }

    #[test]
    fn camera_views_follow_set_view() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let target = owner.actor(2, 0);
        let mut state = QuakeCLocalMessages::new();
        state
            .receive(
                &[NetQuakeMessage::SetView { entity: 2 }],
                Some(&actor),
                Some(Some(target.clone())),
            )
            .unwrap();
        assert_eq!(state.view_target(&actor), Some(&target));
        assert_eq!(state.view(&actor), Some(2));
        state
            .receive(&[NetQuakeMessage::SetView { entity: 2 }], Some(&actor), Some(None))
            .unwrap();
        assert_eq!(state.view_target(&actor), None);
    }

    #[test]
    fn capture_round_trip() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let mut state = QuakeCLocalMessages::new();
        state.receive(&[stat(3, 9)], Some(&actor), None).unwrap();
        let capture = state.capture();
        let mut restored = QuakeCLocalMessages::new();
        restored.restore(&capture).unwrap();
        assert_eq!(restored.stat(&actor, 3), Some(9));
    }

    struct ScriptHost {
        owner: IdentityOwner,
        messages: Vec<NetworkEvent>,
        events: Vec<Q1Event>,
    }

    impl QuakeCLocalMessageHost for ScriptHost {
        fn actor(&self, slot: u16) -> Result<ActorId, QuakeCLocalMessageError> {
            Ok(self.owner.actor(u32::from(slot), 0))
        }
        fn recipients(&self) -> Vec<ActorId> {
            vec![self.owner.actor(1, 0)]
        }
        fn source_actor(&self) -> ActorId {
            self.owner.actor(0, 0)
        }
        fn map(&self) -> String {
            "maps/start.bsp".to_string()
        }
        fn seconds(&self) -> f64 {
            10.0
        }
        fn camera(&self, actor: &ActorId) -> Result<(Vec3, Vec3), QuakeCLocalMessageError> {
            let _ = actor;
            Ok((vec3(1.0, 2.0, 3.0), vec3(0.0, 90.0, 0.0)))
        }
        fn sound(&self, index: u16) -> Result<String, QuakeCLocalMessageError> {
            Ok(format!("sound{index}.wav"))
        }
        fn model(&self, index: u16) -> Result<String, QuakeCLocalMessageError> {
            Ok(format!("model{index}.mdl"))
        }
        fn emit(&mut self, event: Q1Event, _recipient: Option<ActorId>) {
            self.events.push(event);
        }
        fn message(&mut self, event: NetworkEvent, _actor: Option<ActorId>) {
            self.messages.push(event);
        }
        fn music(&mut self, _track: u8) {}
        fn angles(&mut self, _actor: &ActorId, _angles: Vec3) {}
        fn pause(&mut self, _paused: bool) {}
        fn sky(&mut self, _name: &str, _recipient: Option<ActorId>) {}
        fn client_metadata(&mut self, _event: Q1ClientMetadataEvent, _recipient: Option<ActorId>) {}
        fn session(&mut self, _kind: QuakeCSessionKind, _recipient: Option<ActorId>) {}
        fn prompt(&mut self, _event: Q1CompositionEvent) {}
        fn fog(&mut self, _fog: QuakeCLocalFog, _recipient: Option<ActorId>) {}
    }

    #[test]
    fn presents_print_and_sound() {
        let owner = owner();
        let state = QuakeCLocalMessages::new();
        let mut host = ScriptHost {
            owner,
            messages: Vec::new(),
            events: Vec::new(),
        };
        present_quake_c_local_message(
            &NetQuakeMessage::Text {
                kind: NqText::Print,
                text: "hi".to_string(),
            },
            None,
            &state,
            &mut host,
        )
        .unwrap();
        present_quake_c_local_message(
            &NetQuakeMessage::Sound {
                entity: 1,
                channel: 1,
                index: 2,
                volume: 255,
                attenuation: 1.0,
                origin: [0.0, 0.0, 0.0],
            },
            None,
            &state,
            &mut host,
        )
        .unwrap();
        assert_eq!(host.messages.len(), 1);
        assert!(matches!(
            host.events.as_slice(),
            [Q1Event::Sound {
                channel: Q1SoundChannel::Weapon,
                ..
            }]
        ));
    }

    #[test]
    fn rejects_invalid_sound_channel() {
        let owner = owner();
        let state = QuakeCLocalMessages::new();
        let mut host = ScriptHost {
            owner,
            messages: Vec::new(),
            events: Vec::new(),
        };
        assert_eq!(
            present_quake_c_local_message(
                &NetQuakeMessage::Sound {
                    entity: 1,
                    channel: 9,
                    index: 2,
                    volume: 255,
                    attenuation: 1.0,
                    origin: [0.0, 0.0, 0.0],
                },
                None,
                &state,
                &mut host,
            ),
            Err(QuakeCLocalMessageError::InvalidSoundChannel)
        );
    }

    struct FixedView;

    impl QuakeCLocalViewSource for FixedView {
        fn read(&self, _actor: &ActorId) -> Option<(Vec3, Vec3)> {
            Some((vec3(10.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)))
        }
        fn offset(&self, _actor: &ActorId) -> Vec3 {
            vec3(0.0, 0.0, 22.0)
        }
    }

    #[test]
    fn local_view_requires_retarget_or_intermission() {
        let owner = owner();
        let actor = owner.actor(1, 0);
        let state = QuakeCLocalMessages::new();
        assert!(quake_c_local_view(&actor, &state, &FixedView).is_none());
    }
}
