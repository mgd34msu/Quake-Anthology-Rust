//! Native Q2 mod presentation: messages, loops, fog, and client frames.
//!
//! Port of donor `src/app/bootstrap/simulation/native-mod-presentation.ts`
//! (`NativeModPresentation`, `readNativeModPresentation`).

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::ui::hud::q2_native::NativeQ2HudFrame;
use qa_client::ui::types::Q2ProtocolFamily;
use qa_content::contract::{
    ContentId, GameFamily, NativeModClientPresentation, NativeModHudPresentation, NativeModViewPresentation,
};
use qa_content::q2::rerelease::types::{create_q2_fog, Q2FogState};
use qa_core::identity::{ActorId, ProviderId, SavedActorId};
use qa_core::math::Vec3;
use qa_core::time::SourceTime;
use qa_net::protocol::ProtocolIdentity;
use qa_net::q2_adapters::Q2Player;
use qa_net::q2_net::{Q2ReadMode, Q2ServerMessageOptions, Q2ServerMessageReader, Q2ServerRecord};
use qa_net::q2_variants::FogData;
use qa_world::save::value::SaveReader;

use super::classic_guest_services::{ClassicGuestAudience, ClassicGuestMessage};
use super::native_mod_camera::{native_mod_camera, NativeModCameraView};
use super::native_mod_host::{
    ModCommandText, ModEngineEvent, ModHostServices, ModSoundEvent, NativeModHostContext, NativeModProjection,
};
use super::types::{SimulationPresentation, SimulationPresentationEvent, SourcePresentationEvent};
use crate::bootstrap::network::q2_layout::q2_application_layout;

/// Native mod presentation error.
#[derive(Debug, thiserror::Error)]
pub enum NativeModPresentationError {
    /// Invalid presentation state or input.
    #[error("invalid native mod presentation: {0}")]
    Invalid(String),
}

impl NativeModPresentationError {
    /// Invalid-data error.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl From<qa_world::WorldError> for NativeModPresentationError {
    fn from(error: qa_world::WorldError) -> Self {
        Self::invalid(error.to_string())
    }
}

fn mapped(error: impl ToString) -> NativeModPresentationError {
    NativeModPresentationError::invalid(error.to_string())
}

/// Mirror of `NativeModAppearance` from donor
/// `src/app/bootstrap/simulation/native-mod-presentation.ts` (canonical
/// home: this module).
#[derive(Debug, Clone)]
pub struct NativeModAppearance {
    /// Model path.
    pub path: String,
    /// Skin index.
    pub skin: i32,
    /// Skin path.
    pub skin_path: Option<String>,
    /// Attached models.
    pub attached_models: Vec<String>,
    /// Frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Effects.
    pub effects: i32,
    /// Render flags.
    pub render_flags: i32,
    /// Scale.
    pub scale: f64,
    /// Alpha.
    pub alpha: f64,
    /// Visible.
    pub visible: bool,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Mirror of the presentation source clock (donor
/// `NativeModPresentationSource.clock`).
#[derive(Debug, Clone, Copy)]
pub struct NativeQ2PresentationClock {
    /// Server frame.
    pub server_frame: i32,
    /// Time in milliseconds.
    pub time_milliseconds: i64,
    /// Frame time in milliseconds.
    pub frame_time_milliseconds: Option<i64>,
}

/// Mirror of the presentation entity state (donor
/// `NativeModPresentationSource.state`).
#[derive(Debug, Clone)]
pub struct NativeModEntityState {
    /// Active.
    pub active: bool,
    /// Visible.
    pub visible: bool,
    /// Looped sound index.
    pub sound: u32,
    /// Entity event.
    pub event: u32,
    /// Origin.
    pub origin: Vec3,
    /// Loop volume.
    pub volume: f64,
    /// Loop attenuation.
    pub attenuation: f64,
}

/// Native mod presentation source (donor `NativeModPresentationSource`).
pub trait NativeModPresentationSource {
    /// Source edition.
    fn edition(&self) -> ModEdition;
    /// Current configstrings.
    fn configstrings(&mut self) -> HashMap<i32, String>;
    /// Drain queued messages.
    fn drain_messages(&mut self) -> Vec<ClassicGuestMessage>;
    /// Entity appearance.
    fn appearance(&mut self, slot: u32) -> Result<NativeModAppearance, NativeModPresentationError>;
    /// Entity signature.
    fn signature(&mut self, slot: u32) -> Result<String, NativeModPresentationError>;
    /// Client player state.
    fn player_state(&mut self, slot: u32) -> Result<Q2Player, NativeModPresentationError>;
    /// Presentation clock.
    fn clock(&mut self) -> NativeQ2PresentationClock;
    /// Entity presentation state.
    fn entity_state(&mut self, slot: u32) -> Result<NativeModEntityState, NativeModPresentationError>;
}

/// Mod source edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModEdition {
    /// Classic API 3.
    Classic,
    /// Rerelease API 2023.
    Rerelease,
}

/// Mirror of `ModClientPresentationFrame` from donor
/// `src/world/session/mod-client-presentation.ts` (canonical home:
/// session lane; unify post-merge).
#[derive(Debug, Clone)]
pub enum ModClientPresentationFrame {
    /// QVM frame.
    Qvm {
        /// HUD mode.
        hud_overlay: bool,
    },
    /// QuakeC frame.
    Quakec {
        /// Health and armor.
        hud: Option<(i32, i32)>,
        /// Player view.
        view: Option<super::types::PlayerView>,
    },
    /// Native frame.
    Native {
        /// HUD overlay.
        hud: Option<NativeHudFrame>,
        /// Camera view.
        view: Option<NativeModCameraView>,
    },
}

/// Native HUD overlay frame.
#[derive(Debug, Clone)]
pub struct NativeHudFrame {
    /// Overlay mode.
    pub mode: NativeHudMode,
    /// HUD frame.
    pub frame: NativeQ2HudFrame,
}

/// Native HUD overlay mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeHudMode {
    /// Layout overlay.
    LayoutOverlay,
    /// Replace status.
    ReplaceStatus,
}

/// Saved fog entry.
#[derive(Debug, Clone)]
pub struct NativeFogEntry {
    /// Recipient.
    pub actor: SavedActorId,
    /// Fog state.
    pub value: Q2FogState,
}

/// Saved client state entry.
#[derive(Debug, Clone)]
pub struct NativeClientEntry {
    /// Recipient.
    pub actor: SavedActorId,
    /// Layout program.
    pub layout: String,
    /// Inventory counts.
    pub inventory: Vec<i64>,
}

/// Mirror of `NativeModPresentationCheckpoint` from donor
/// `src/app/bootstrap/simulation/native-mod-presentation.ts`
/// (canonical home: this module).
#[derive(Debug, Clone)]
pub struct NativeModPresentationCheckpoint {
    /// Fog recipients.
    pub fog: Vec<NativeFogEntry>,
    /// Client states.
    pub clients: Vec<NativeClientEntry>,
}

/// Fog read seam (donor `readQ2FogState` from donor
/// `src/persistence/q2-rerelease-state.ts`, canonical home: persistence
/// lane; unify post-merge).
pub type ReadQ2FogStateFn = Rc<dyn Fn(SaveReader) -> Result<Q2FogState, NativeModPresentationError>>;

/// Read a presentation checkpoint.
pub fn read_native_mod_presentation(
    reader: SaveReader,
    read_fog: &ReadQ2FogStateFn,
) -> Result<NativeModPresentationCheckpoint, NativeModPresentationError> {
    let fog: Vec<NativeFogEntry> = reader
        .field("fog")
        .list(|entry| {
            Ok::<_, NativeModPresentationError>(NativeFogEntry {
                actor: SavedActorId {
                    slot: u32::try_from(entry.field("actor").field("slot").integer(0).map_err(mapped)?)
                        .map_err(mapped)?,
                    generation: u32::try_from(entry.field("actor").field("generation").integer(0).map_err(mapped)?)
                        .map_err(mapped)?,
                },
                value: read_fog(entry.field("value"))?,
            })
        })
        .map_err(mapped)?;
    let mut seen = HashSet::new();
    if fog
        .iter()
        .any(|entry| !seen.insert((entry.actor.slot, entry.actor.generation)))
    {
        return Err(NativeModPresentationError::invalid(
            "Duplicate native mod presentation recipient",
        ));
    }
    let clients_field = reader.field("clients");
    let clients: Vec<NativeClientEntry> = if clients_field.is_missing() {
        Vec::new()
    } else {
        clients_field
            .list(|entry| {
                Ok::<_, NativeModPresentationError>(NativeClientEntry {
                    actor: SavedActorId {
                        slot: u32::try_from(entry.field("actor").field("slot").integer(0).map_err(mapped)?)
                            .map_err(mapped)?,
                        generation: u32::try_from(entry.field("actor").field("generation").integer(0).map_err(mapped)?)
                            .map_err(mapped)?,
                    },
                    layout: entry.field("layout").string().map_err(mapped)?,
                    inventory: entry
                        .field("inventory")
                        .list(|value| value.integer(i64::MIN).map_err(mapped))
                        .map_err(mapped)?,
                })
            })
            .map_err(mapped)?
    };
    let mut seen = HashSet::new();
    if clients
        .iter()
        .any(|entry| !seen.insert((entry.actor.slot, entry.actor.generation)))
    {
        return Err(NativeModPresentationError::invalid(
            "Duplicate native client presentation",
        ));
    }
    Ok(NativeModPresentationCheckpoint { fog, clients })
}

/// Service translation host (donor `Q2ServicePresentationHost` from
/// donor `src/app/bootstrap/network/q2-service-presentation.ts`,
/// canonical home: network lane; unify post-merge).
pub trait Q2ServicePresentationHost {
    /// Source edition.
    fn edition(&self) -> ModEdition;
    /// Presentation content.
    fn content(&self) -> ContentId;
    /// Time in seconds.
    fn seconds(&self) -> f64;
    /// Next event sequence.
    fn next_sequence(&mut self) -> u64;
    /// Destination player.
    fn player(&self) -> Option<(ActorId, u32)>;
    /// Actor bound to a source entity.
    fn actor(&mut self, source_entity: u32) -> Option<ActorId>;
    /// Entity origin and angles.
    fn entity(&mut self, source_entity: u32) -> Option<(Vec3, Vec3)>;
    /// Sound configstring offset.
    fn sound_config_offset(&self) -> u32;
    /// Image configstring offset.
    fn image_config_offset(&self) -> u32;
    /// Player-skin configstring offset.
    fn player_skin_config_offset(&self) -> u32;
    /// Apply a fog update.
    fn fog(&mut self, value: &FogData) -> Result<Q2FogState, NativeModPresentationError>;
    /// Read a configstring.
    fn config_string(&self, index: i32) -> Option<String>;
    /// Write a configstring.
    fn set_config_string(&mut self, index: i32, value: String);
    /// Write the client inventory.
    fn set_inventory(&mut self, counts: Vec<i32>) -> Result<(), NativeModPresentationError>;
    /// Write the client layout.
    fn set_layout(&mut self, program: String) -> Result<(), NativeModPresentationError>;
    /// Emit a presentation event.
    fn emit(&mut self, event: SimulationPresentationEvent);
}

/// Service record translation seam (donor `translateQ2ServiceRecords`
/// from donor `src/app/bootstrap/network/q2-service-presentation.ts`,
/// canonical home: network lane; unify post-merge).
pub type TranslateQ2ServiceRecordsFn = Rc<
    dyn Fn(
        &[Q2ServerRecord],
        &mut dyn Q2ServicePresentationHost,
    ) -> Result<Vec<Q2ServerRecord>, NativeModPresentationError>,
>;

/// Wire fog seam (donor `q2FogFromWire` from donor
/// `src/app/bootstrap/rerelease-presentation/fog.ts`, canonical home:
/// rerelease presentation lane; unify post-merge).
pub type Q2FogFromWireFn = Rc<dyn Fn(&Q2FogState, &FogData) -> Q2FogState>;

/// Native loop record.
#[derive(Debug, Clone)]
struct NativeLoop {
    path: String,
    volume: f64,
    attenuation: f64,
    origin: Vec3,
}

/// Client message state.
#[derive(Debug, Clone, Default)]
struct ClientMessages {
    layout: String,
    inventory: Vec<i32>,
}

/// Native mod presentation options.
pub struct NativeModPresentationOptions {
    /// Client admission, when the host presents clients.
    pub admission: Option<NativeModClientPresentation>,
    /// Presentation content.
    pub content: ContentId,
    /// Slot projection.
    pub projection: Rc<dyn NativeModProjection>,
    /// Mod services.
    pub services: ModHostServices,
    /// Host context.
    pub context: NativeModHostContext,
    /// Loop owner.
    pub owner: ProviderId,
    /// Service translation seam.
    pub translate: TranslateQ2ServiceRecordsFn,
    /// Wire fog seam.
    pub fog_from_wire: Q2FogFromWireFn,
}

/// Native mod presentation.
pub struct NativeModPresentation {
    reader: Q2ServerMessageReader,
    layout: super::super::network::q2_layout::Q2ApplicationLayout,
    edition: ModEdition,
    fog: HashMap<ActorId, Q2FogState>,
    configs: HashMap<i32, String>,
    loops: HashMap<ActorId, NativeLoop>,
    entity_events: HashMap<ActorId, u32>,
    clients: HashMap<ActorId, ClientMessages>,
    client_frames: HashMap<ActorId, ModClientPresentationFrame>,
    revision: u32,
    content: ContentId,
    projection: Rc<dyn NativeModProjection>,
    services: ModHostServices,
    context: NativeModHostContext,
    owner: ProviderId,
    admission: Option<NativeModClientPresentation>,
    translate: TranslateQ2ServiceRecordsFn,
    fog_from_wire: Q2FogFromWireFn,
}

impl NativeModPresentation {
    /// Create a presentation over a source edition.
    pub fn new(edition: ModEdition, options: NativeModPresentationOptions) -> Result<Self, NativeModPresentationError> {
        let protocol = match edition {
            ModEdition::Classic => ProtocolIdentity::Q2Classic,
            ModEdition::Rerelease => ProtocolIdentity::Q2Rerelease,
        };
        let layout = q2_application_layout(protocol).map_err(mapped)?;
        let reader = Q2ServerMessageReader::new(
            protocol,
            Q2ServerMessageOptions {
                read_mode: Q2ReadMode::Network,
                max_config_strings: u16::try_from(layout.max_config_strings).map_err(mapped)?,
                inventory_slots: 256,
                q2pro_extended_temp_entities: None,
            },
            std::collections::HashSet::new(),
            None,
        )
        .map_err(mapped)?;
        Ok(Self {
            reader,
            layout,
            edition,
            fog: HashMap::new(),
            configs: HashMap::new(),
            loops: HashMap::new(),
            entity_events: HashMap::new(),
            clients: HashMap::new(),
            client_frames: HashMap::new(),
            revision: 0,
            content: options.content,
            projection: options.projection,
            services: options.services,
            context: options.context,
            owner: options.owner,
            admission: options.admission,
            translate: options.translate,
            fog_from_wire: options.fog_from_wire,
        })
    }

    /// Presentation generation.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.revision
    }

    /// Client frame for a live actor.
    #[must_use]
    pub fn client_frame(&self, actor: &ActorId) -> Option<ModClientPresentationFrame> {
        if !(self.services.actors.is_live)(actor) {
            return None;
        }
        self.client_frames.get(actor).cloned()
    }

    /// Publish one frame for the given actor/slot pairs.
    pub fn publish(
        &mut self,
        source: &mut dyn NativeModPresentationSource,
        actors: &[(ActorId, u32)],
        events: bool,
    ) -> Result<(), NativeModPresentationError> {
        self.drain(source)?;
        self.client_frames.clear();
        let configstrings = self.admission.as_ref().map(|_| source.configstrings());
        let time = (self.services.time)();
        let active: HashSet<&ActorId> = actors.iter().map(|(actor, _)| actor).collect();
        let tracked: Vec<ActorId> = self.entity_events.keys().chain(self.loops.keys()).cloned().collect();
        for actor in tracked {
            if !active.contains(&actor) {
                self.release(&actor);
            }
        }
        for (actor, slot) in actors {
            let state = source.entity_state(*slot)?;
            if !state.active || !(self.services.actors.is_live)(actor) {
                self.release(actor);
                continue;
            }
            if let (Some(admission), Some(configstrings)) = (self.admission.as_ref(), configstrings.as_ref()) {
                if self.projection.accepts_client(*slot) {
                    let player = source.player_state(*slot)?;
                    let received = self.clients.get(actor);
                    let hud = match admission.hud {
                        NativeModHudPresentation::None => None,
                        hud => {
                            let mode = match hud {
                                NativeModHudPresentation::LayoutOverlay => NativeHudMode::LayoutOverlay,
                                _ => NativeHudMode::ReplaceStatus,
                            };
                            let (protocol, stats) = match &player {
                                Q2Player::Classic(state) => (
                                    Q2ProtocolFamily::Classic,
                                    state.view.stats.iter().map(|value| i32::from(*value)).collect(),
                                ),
                                Q2Player::Rerelease(state) => (
                                    Q2ProtocolFamily::Rerelease,
                                    state.view.stats.iter().map(|value| i32::from(*value)).collect(),
                                ),
                            };
                            let clock = source.clock();
                            Some(NativeHudFrame {
                                mode,
                                frame: NativeQ2HudFrame {
                                    protocol,
                                    stats,
                                    configstrings: configstrings
                                        .iter()
                                        .map(|(index, value)| (*index, value.clone()))
                                        .collect(),
                                    layout: received.map_or_else(String::new, |state| state.layout.clone()),
                                    inventory: received.map_or_else(Vec::new, |state| state.inventory.clone()),
                                    player_number: i32::try_from(*slot).map_err(mapped)? - 1,
                                    server_frame: clock.server_frame,
                                    time_ms: clock.time_milliseconds,
                                    frame_time_ms: clock.frame_time_milliseconds,
                                },
                            })
                        }
                    };
                    let view = match admission.view {
                        NativeModViewPresentation::None => None,
                        NativeModViewPresentation::Playerstate => Some(native_mod_camera(&player)),
                    };
                    self.client_frames
                        .insert(actor.clone(), ModClientPresentationFrame::Native { hud, view });
                }
            }
            if !state.visible {
                if let Some(loop_state) = self.loops.get(actor).cloned() {
                    self.stop(actor, &loop_state);
                }
                self.entity_events.remove(actor);
                continue;
            }
            let previous = self.entity_events.get(actor).copied();
            if events && state.event != 0 && previous != Some(state.event) {
                let engine = self.services.engine.as_ref().ok_or_else(|| {
                    NativeModPresentationError::invalid("Native mod output requires presentation services")
                })?;
                (engine.emit)(
                    ModEngineEvent::EntityEvent {
                        actor: actor.clone(),
                        event: state.event,
                    },
                    seconds(&time),
                    None,
                );
            }
            self.entity_events.insert(actor.clone(), state.event);
            if state.sound == 0 {
                if let Some(loop_state) = self.loops.get(actor).cloned() {
                    self.stop(actor, &loop_state);
                }
                continue;
            }
            let key =
                i32::try_from(self.layout.sounds).map_err(mapped)? + i32::try_from(state.sound).map_err(mapped)?;
            let path = source.configstrings().get(&key).cloned().unwrap_or_default();
            if path.is_empty() {
                return Err(NativeModPresentationError::invalid(format!(
                    "Native mod loop sound {} has no configstring",
                    state.sound
                )));
            }
            let volume = if self.edition == ModEdition::Rerelease && state.volume == 0.0 {
                1.0
            } else {
                state.volume
            };
            let attenuation = if self.edition == ModEdition::Classic {
                state.attenuation
            } else if state.attenuation == -1.0 {
                0.0
            } else if state.attenuation > 0.0 && state.attenuation != 3.0 {
                state.attenuation / 5.0
            } else {
                1.0
            };
            let origin = (self.services.bodies.read)(actor)
                .map(|body| body.origin)
                .unwrap_or(state.origin);
            if let Some(loop_state) = self.loops.get(actor).cloned() {
                if loop_state.path == path && loop_state.volume == volume && loop_state.attenuation == attenuation {
                    self.loops.insert(actor.clone(), NativeLoop { origin, ..loop_state });
                    continue;
                }
                self.stop(actor, &loop_state);
            }
            let next = NativeLoop {
                path,
                volume,
                attenuation,
                origin,
            };
            self.loops.insert(actor.clone(), next.clone());
            self.sound(actor, &next, super::native_mod_host::SoundLoopState::Start)?;
        }
        Ok(())
    }

    /// Clear entity-event memory at frame start.
    pub fn begin_frame(&mut self) {
        self.entity_events.clear();
    }

    fn sound(
        &mut self,
        actor: &ActorId,
        loop_state: &NativeLoop,
        state: super::native_mod_host::SoundLoopState,
    ) -> Result<(), NativeModPresentationError> {
        let engine =
            self.services.engine.as_ref().ok_or_else(|| {
                NativeModPresentationError::invalid("Native mod output requires presentation services")
            })?;
        (engine.emit)(
            ModEngineEvent::Sound(Box::new(ModSoundEvent {
                actor: actor.clone(),
                path: loop_state.path.clone(),
                volume: loop_state.volume,
                attenuation: loop_state.attenuation,
                origin: loop_state.origin,
                reliable: false,
                loop_state: state,
                loop_owner: self.owner.clone(),
            })),
            seconds(&(self.services.time)()),
            None,
        );
        Ok(())
    }

    fn stop(&mut self, actor: &ActorId, loop_state: &NativeLoop) {
        let _ = self.sound(actor, loop_state, super::native_mod_host::SoundLoopState::Stop);
        self.loops.remove(actor);
    }

    /// Release all state bound to an actor.
    pub fn release(&mut self, actor: &ActorId) {
        if let Some(loop_state) = self.loops.get(actor).cloned() {
            self.stop(actor, &loop_state);
        }
        self.entity_events.remove(actor);
        self.fog.remove(actor);
        self.clients.remove(actor);
        self.client_frames.remove(actor);
    }

    /// Close the presentation, releasing every actor.
    pub fn close(&mut self) {
        self.revision += 1;
        let actors: Vec<ActorId> = self.loops.keys().cloned().collect();
        for actor in actors {
            self.release(&actor);
        }
        self.entity_events.clear();
        self.clients.clear();
        self.client_frames.clear();
    }

    fn client_messages(&mut self, actor: &ActorId) -> &mut ClientMessages {
        self.clients.entry(actor.clone()).or_default()
    }

    fn recipients(
        &mut self,
        message: &ClassicGuestMessage,
    ) -> Result<Vec<Option<ActorId>>, NativeModPresentationError> {
        match &message.audience {
            ClassicGuestAudience::Unicast { slot } => {
                let actor = self.projection.actor_at(*slot);
                match actor {
                    Some(actor) if self.projection.accepts_client(*slot) => Ok(vec![Some(actor)]),
                    _ => Err(NativeModPresentationError::invalid(
                        "Native mod message targets an unavailable destination player",
                    )),
                }
            }
            ClassicGuestAudience::Multicast { origin, scope } => {
                let players = self
                    .services
                    .engine
                    .as_ref()
                    .and_then(|engine| engine.presentation_players.as_ref())
                    .map(|players| players())
                    .unwrap_or_default();
                if players.is_empty() {
                    return Ok(vec![None]);
                }
                if matches!(scope, super::classic_guest_services::MulticastScope::All) {
                    return Ok(players.into_iter().map(Some).collect());
                }
                let kind = match scope {
                    super::classic_guest_services::MulticastScope::Phs => qa_bots::scene::VisibilityKind::Phs,
                    _ => qa_bots::scene::VisibilityKind::Pvs,
                };
                let scene = Rc::clone(&self.context.scene);
                let cluster = scene.leaf_cluster(scene.point_leaf(*origin));
                Ok(players
                    .into_iter()
                    .filter(|actor| {
                        (self.services.bodies.read)(actor).is_some_and(|body| {
                            scene.cluster_visible(cluster, scene.leaf_cluster(scene.point_leaf(body.origin)), kind)
                        })
                    })
                    .map(Some)
                    .collect())
            }
        }
    }

    fn drain(&mut self, source: &mut dyn NativeModPresentationSource) -> Result<(), NativeModPresentationError> {
        let messages = source.drain_messages();
        if messages.is_empty() {
            return Ok(());
        }
        if self.services.engine.is_none() {
            return Err(NativeModPresentationError::invalid(
                "Native mod output requires presentation services",
            ));
        }
        for (index, value) in source.configstrings() {
            self.configs.insert(index, value);
        }
        let time = (self.services.time)();
        let seconds_value = seconds(&time);
        for message in &messages {
            let records = self.reader.read(&message.bytes).map_err(mapped)?;
            let recipients = self.recipients(message)?;
            for recipient in recipients {
                let mut host = DrainHost {
                    presentation: self,
                    source,
                    recipient: recipient.clone(),
                    reliable: message.reliable,
                    seconds: seconds_value,
                };
                let translate = Rc::clone(&host.presentation.translate);
                let remaining = translate(&records, &mut host)?;
                let (presentation, source) = host.parts();
                for record in &remaining {
                    match &record.event {
                        qa_net::q2_net::Q2ServerEvent::Nop => {}
                        qa_net::q2_net::Q2ServerEvent::Print { .. } if recipient.is_none() => {}
                        qa_net::q2_net::Q2ServerEvent::CommandText { text } => {
                            let engine = presentation.services.engine.as_ref().ok_or_else(|| {
                                NativeModPresentationError::invalid("Native mod output requires presentation services")
                            })?;
                            if let Some(message_sink) = engine.message.as_ref() {
                                message_sink(ModCommandText { text: text.clone() }, recipient.clone());
                            } else {
                                return Err(NativeModPresentationError::invalid(
                                    "Native mod message command-text needs a shared destination binding",
                                ));
                            }
                        }
                        _ => {
                            let _ = (presentation, source);
                            return Err(NativeModPresentationError::invalid(
                                "Native mod message needs a shared destination binding",
                            ));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Entity appearance rows for an actor.
    pub fn appearance(
        &mut self,
        source: &mut dyn NativeModPresentationSource,
        actor: &ActorId,
        slot: u32,
    ) -> Result<Vec<SimulationPresentation>, NativeModPresentationError> {
        let appearance = source.appearance(slot)?;
        let body = (self.services.bodies.read)(actor);
        let base = SimulationPresentation {
            actor: actor.clone(),
            content: self.content.clone(),
            family: GameFamily::Q2,
            path: appearance.path.clone(),
            frame: appearance.frame,
            old_frame: appearance.old_frame,
            back_lerp: None,
            skin: appearance.skin,
            skin_path: appearance.skin_path.clone(),
            indexed_skin: None,
            player_colors: None,
            effects: appearance.effects,
            render_flags: appearance.render_flags,
            origin: body.as_ref().map(|body| body.origin).unwrap_or(appearance.origin),
            previous_origin: None,
            model_beam: None,
            shader_beam: None,
            model_attachments: None,
            model_anchor: None,
            q3_grapple_cable: None,
            angles: body.as_ref().map(|body| body.angles).unwrap_or(appearance.angles),
            scale: appearance.scale,
            alpha: Some(appearance.alpha),
            visible: appearance.visible,
            view_weapon: false,
            q3_weapon: None,
            held_weapon: None,
            native_held_weapon: false,
            weapon_item: None,
            replaces_body: false,
            render_source_client: false,
            flare: None,
        };
        let mut rows = vec![base.clone()];
        for path in appearance.attached_models.iter().filter(|path| !path.is_empty()) {
            rows.push(SimulationPresentation {
                path: path.clone(),
                skin: 0,
                skin_path: None,
                ..base.clone()
            });
        }
        Ok(rows)
    }

    /// Capture the presentation checkpoint.
    pub fn checkpoint(&self) -> NativeModPresentationCheckpoint {
        let live = |actor: &ActorId| (self.services.actors.is_live)(actor);
        NativeModPresentationCheckpoint {
            fog: self
                .fog
                .iter()
                .filter(|(actor, _)| live(actor))
                .map(|(actor, value)| NativeFogEntry {
                    actor: SavedActorId::from(actor),
                    value: *value,
                })
                .collect(),
            clients: self
                .clients
                .iter()
                .filter(|(actor, _)| live(actor))
                .map(|(actor, state)| NativeClientEntry {
                    actor: SavedActorId::from(actor),
                    layout: state.layout.clone(),
                    inventory: state.inventory.iter().map(|value| i64::from(*value)).collect(),
                })
                .collect(),
        }
    }

    /// Restore a presentation checkpoint.
    pub fn restore(&mut self, saved: &NativeModPresentationCheckpoint) -> Result<(), NativeModPresentationError> {
        let resolve = |actor: SavedActorId| match self.services.reference_saved.as_ref() {
            Some(resolve) => resolve(actor),
            None => (self.services.actors.reference_saved)(actor),
        };
        let fog: Vec<(ActorId, Q2FogState)> = saved
            .fog
            .iter()
            .map(|entry| (resolve(entry.actor), entry.value))
            .collect();
        if fog.iter().any(|(actor, _)| !(self.services.actors.is_live)(actor)) {
            return Err(NativeModPresentationError::invalid(
                "Native mod fog recipient is unavailable",
            ));
        }
        let clients: Vec<(ActorId, ClientMessages)> = saved
            .clients
            .iter()
            .map(|entry| {
                (
                    resolve(entry.actor),
                    ClientMessages {
                        layout: entry.layout.clone(),
                        inventory: entry
                            .inventory
                            .iter()
                            .map(|value| i32::try_from(*value).unwrap_or(i32::MAX))
                            .collect(),
                    },
                )
            })
            .collect();
        if clients.iter().any(|(actor, _)| !(self.services.actors.is_live)(actor)) {
            return Err(NativeModPresentationError::invalid(
                "Native client presentation recipient is unavailable",
            ));
        }
        for (actor, value) in fog {
            self.fog.insert(actor, value);
        }
        for (actor, state) in clients {
            self.clients.insert(actor, state);
        }
        Ok(())
    }
}

fn seconds(time: &SourceTime) -> f64 {
    match time {
        SourceTime::Seconds(value) => f64::from(*value),
        SourceTime::Milliseconds(value) => f64::from(*value) / 1000.0,
    }
}

/// Per-recipient service host over a drain.
struct DrainHost<'a> {
    presentation: &'a mut NativeModPresentation,
    source: &'a mut dyn NativeModPresentationSource,
    recipient: Option<ActorId>,
    reliable: bool,
    seconds: f64,
}

impl DrainHost<'_> {
    fn parts(&mut self) -> (&mut NativeModPresentation, &mut dyn NativeModPresentationSource) {
        (&mut *self.presentation, &mut *self.source)
    }
}

impl Q2ServicePresentationHost for DrainHost<'_> {
    fn edition(&self) -> ModEdition {
        self.presentation.edition
    }

    fn content(&self) -> ContentId {
        self.presentation.content.clone()
    }

    fn seconds(&self) -> f64 {
        self.seconds
    }

    fn next_sequence(&mut self) -> u64 {
        0
    }

    fn player(&self) -> Option<(ActorId, u32)> {
        self.recipient.clone().map(|actor| {
            let slot = self.presentation.projection.slot_of(&actor).unwrap_or(0);
            (actor, slot)
        })
    }

    fn actor(&mut self, source_entity: u32) -> Option<ActorId> {
        self.presentation.projection.actor_at(source_entity)
    }

    fn entity(&mut self, source_entity: u32) -> Option<(Vec3, Vec3)> {
        if let Some(actor) = self.presentation.projection.actor_at(source_entity) {
            if let Some(body) = (self.presentation.services.bodies.read)(&actor) {
                return Some((body.origin, body.angles));
            }
        }
        self.source
            .appearance(source_entity)
            .ok()
            .map(|appearance| (appearance.origin, appearance.angles))
    }

    fn sound_config_offset(&self) -> u32 {
        self.presentation.layout.sounds
    }

    fn image_config_offset(&self) -> u32 {
        self.presentation.layout.images
    }

    fn player_skin_config_offset(&self) -> u32 {
        self.presentation.layout.player_skins
    }

    fn fog(&mut self, value: &FogData) -> Result<Q2FogState, NativeModPresentationError> {
        let recipient = self
            .recipient
            .clone()
            .ok_or_else(|| NativeModPresentationError::invalid("Native fog requires a destination player"))?;
        let previous = self
            .presentation
            .fog
            .get(&recipient)
            .cloned()
            .unwrap_or_else(create_q2_fog);
        let next = (self.presentation.fog_from_wire)(&previous, value);
        self.presentation.fog.insert(recipient, next);
        Ok(next)
    }

    fn config_string(&self, index: i32) -> Option<String> {
        self.presentation.configs.get(&index).cloned()
    }

    fn set_config_string(&mut self, index: i32, value: String) {
        self.presentation.configs.insert(index, value);
    }

    fn set_inventory(&mut self, counts: Vec<i32>) -> Result<(), NativeModPresentationError> {
        let actor = self.recipient.clone().ok_or_else(|| {
            NativeModPresentationError::invalid("Native client presentation requires an admitted recipient")
        })?;
        self.presentation.client_messages(&actor).inventory = counts;
        Ok(())
    }

    fn set_layout(&mut self, program: String) -> Result<(), NativeModPresentationError> {
        let actor = self.recipient.clone().ok_or_else(|| {
            NativeModPresentationError::invalid("Native client presentation requires an admitted recipient")
        })?;
        self.presentation.client_messages(&actor).layout = program;
        Ok(())
    }

    fn emit(&mut self, mut event: SimulationPresentationEvent) {
        let engine = match self.presentation.services.engine.as_ref() {
            Some(engine) => engine,
            None => return,
        };
        if let SourcePresentationEvent::Q2(qa_content::q2::foundation::host::Q2PresentationEvent::Sound(sound)) =
            &mut event.event
        {
            sound.reliable = self.reliable;
        }
        (engine.emit)(
            ModEngineEvent::Service(Box::new(event)),
            self.seconds,
            self.recipient.clone(),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use qa_bots::scene::{
        LeafQueryResult, PointContentsQuery, PointContentsResult, SceneQueries, TraceQuery,
        TraceResult as BotsTraceResult, VisibilityKind,
    };
    use qa_core::identity::{IdentityOwner, OwnedActor};
    use qa_core::math::Bounds;
    use qa_net::q2_adapters::{Q2MovementState, Q2PlayerState, Q2PlayerView, Q2Vec3, Q2Vec4};
    use qa_world::save::records::write_saved_actor;
    use qa_world::save::value::{arr, obj};
    use qa_world::spatial::QueryRole;

    use super::super::native_mod_host::SoundLoopState;
    use super::super::source_hosts::ActorHostScene;
    use super::*;

    const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

    struct FakeNav;

    impl qa_compat::q2::rerelease::navigation::NavigationServices for FakeNav {
        fn runtime(&self) -> Option<&qa_compat::q2::rerelease::navigation::NavRuntime> {
            None
        }

        fn move_to_point(
            &mut self,
            _actor: u32,
            _point: qa_core::math::Vec3,
            _tolerance: f32,
        ) -> qa_compat::q2::rerelease::navigation::GoalStatus {
            1
        }

        fn follow_actor(&mut self, _actor: u32, _target: u32) -> qa_compat::q2::rerelease::navigation::GoalStatus {
            1
        }
    }

    struct FakeScene;

    impl SceneQueries for FakeScene {
        fn trace(&self, _query: &TraceQuery) -> BotsTraceResult {
            BotsTraceResult {
                fraction: 1.0,
                end: ZERO,
                start_solid: false,
                all_solid: false,
                contact: qa_bots::scene::TraceContact::None,
                hit: qa_bots::scene::TraceHit::None,
                detail: qa_bots::scene::TraceDetail::Q1 {
                    in_open: true,
                    in_water: false,
                    source_plane: qa_core::math::Plane {
                        normal: ZERO,
                        distance: 0.0,
                    },
                    surface_flags: None,
                    contents: None,
                },
            }
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
            PointContentsResult::Q2 { stored: 0, merged: 0 }
        }

        fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
            LeafQueryResult {
                leaves: Vec::new(),
                topnode: None,
                overflow: false,
            }
        }

        fn areas_connected(&self, _first: i32, _second: i32) -> bool {
            true
        }

        fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
            true
        }
    }

    impl ActorHostScene for FakeScene {
        fn trace_excluding(&self, query: &TraceQuery, _excluded: &[ActorId]) -> BotsTraceResult {
            SceneQueries::trace(self, query)
        }

        fn geometry_trace(&self, query: &TraceQuery) -> BotsTraceResult {
            SceneQueries::trace(self, query)
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            0
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            0
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            Bounds { min: ZERO, max: ZERO }
        }

        fn query_actors(&self, _bounds: &Bounds, _role: QueryRole) -> Vec<qa_world::spatial::SpatialActor> {
            Vec::new()
        }

        fn model_count(&self) -> usize {
            0
        }

        fn q2_texture_info(&self) -> Option<Vec<super::super::source_hosts::SceneTextureInfo>> {
            None
        }
    }

    struct FakeProjection {
        slots: HashMap<u32, ActorId>,
        owner: Rc<IdentityOwner>,
    }

    impl NativeModProjection for FakeProjection {
        fn project(&self, record: &qa_compat::q2::classic::records::RawEntityView) -> Option<OwnedActor> {
            self.slots.get(&record.slot).map(|id| {
                self.owner
                    .owned_actor(
                        id,
                        ProviderId {
                            namespace: "t".to_string(),
                            name: "p".to_string(),
                        },
                    )
                    .expect("owned")
            })
        }

        fn actor_at(&self, slot: u32) -> Option<ActorId> {
            self.slots.get(&slot).cloned()
        }

        fn slot_of(&self, actor: &ActorId) -> Option<u32> {
            self.slots.iter().find_map(|(slot, id)| (id == actor).then_some(*slot))
        }

        fn accepts_client(&self, _slot: u32) -> bool {
            true
        }

        fn address(&self, _actor: &ActorId) -> Option<qa_guest::core::contracts::GuestAddress> {
            None
        }

        fn import_boundary(
            &self,
            _name: &str,
            _values: &[qa_guest::core::contracts::GuestCallValue],
            invoke: &dyn Fn() -> qa_guest::core::contracts::GuestCallResult,
        ) -> qa_guest::core::contracts::GuestCallResult {
            invoke()
        }
    }

    struct FakeSource {
        edition: ModEdition,
        configstrings: HashMap<i32, String>,
        messages: Vec<ClassicGuestMessage>,
        states: HashMap<u32, NativeModEntityState>,
        player: Q2Player,
    }

    impl NativeModPresentationSource for FakeSource {
        fn edition(&self) -> ModEdition {
            self.edition
        }

        fn configstrings(&mut self) -> HashMap<i32, String> {
            self.configstrings.clone()
        }

        fn drain_messages(&mut self) -> Vec<ClassicGuestMessage> {
            std::mem::take(&mut self.messages)
        }

        fn appearance(&mut self, slot: u32) -> Result<NativeModAppearance, NativeModPresentationError> {
            Ok(NativeModAppearance {
                path: format!("models/{slot}.md2"),
                skin: 0,
                skin_path: None,
                attached_models: vec!["models/gun.md2".to_string(), String::new()],
                frame: 1,
                old_frame: 0,
                effects: 0,
                render_flags: 0,
                scale: 1.0,
                alpha: 1.0,
                visible: true,
                origin: ZERO,
                angles: ZERO,
            })
        }

        fn signature(&mut self, slot: u32) -> Result<String, NativeModPresentationError> {
            Ok(format!("sig-{slot}"))
        }

        fn player_state(&mut self, _slot: u32) -> Result<Q2Player, NativeModPresentationError> {
            Ok(self.player.clone())
        }

        fn clock(&mut self) -> NativeQ2PresentationClock {
            NativeQ2PresentationClock {
                server_frame: 7,
                time_milliseconds: 700,
                frame_time_milliseconds: Some(100),
            }
        }

        fn entity_state(&mut self, slot: u32) -> Result<NativeModEntityState, NativeModPresentationError> {
            self.states
                .get(&slot)
                .cloned()
                .ok_or_else(|| NativeModPresentationError::invalid("missing test entity state"))
        }
    }

    fn owner() -> Rc<IdentityOwner> {
        Rc::new(IdentityOwner::create("presentation-harness").expect("owner"))
    }

    fn player() -> Q2Player {
        Q2Player::Classic(Q2PlayerState {
            view: Q2PlayerView {
                view_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                view_offset: Q2Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 22.0,
                },
                kick_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_angles: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_offset: Q2Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                gun_index: 0,
                gun_frame: 0,
                fov: 90,
                render_flags: 0,
                stats: vec![100, 50],
            },
            movement: Q2MovementState {
                move_type: 3,
                origin_eighths: [0, 0, 0],
                velocity_eighths: [0, 0, 0],
                flags: 0,
                time: 0,
                gravity: 800,
                delta_angle_shorts: [0, 0, 0],
            },
            blend: Q2Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
        })
    }

    struct Harness {
        presentation: NativeModPresentation,
        emitted: Rc<RefCell<Vec<ModEngineEvent>>>,
        owner: Rc<IdentityOwner>,
    }

    fn harness(admission: Option<NativeModClientPresentation>) -> Harness {
        let owner = owner();
        let emitted = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&emitted);
        let services = ModHostServices {
            actors: super::super::native_mod_host::ModActorServices {
                is_live: Rc::new(|_| true),
                reference_saved: {
                    let owner = Rc::clone(&owner);
                    Rc::new(move |saved| owner.actor(saved.slot, saved.generation))
                },
            },
            bodies: super::super::native_mod_host::ModBodyServices {
                read: Rc::new(|_| None),
            },
            engine: Some(super::super::native_mod_host::ModServiceEngine {
                print: Rc::new(|_| {}),
                message: None,
                emit: Rc::new(move |event, _, _| sink.borrow_mut().push(event)),
                presentation_players: None,
            }),
            time: Rc::new(|| SourceTime::Seconds(1.0)),
            seed: 7,
            commands: super::super::native_mod_host::ModCommandServices {
                bind: Rc::new(|_, _| {}),
            },
            resources: super::super::native_mod_host::ModResourceServices { own: Rc::new(|_| {}) },
            reference_saved: None,
        };
        let context = NativeModHostContext {
            scene: Rc::new(FakeScene),
            max_clients: 4,
            frame_ms: 100.0,
            skill: 2,
            mode: "deathmatch".to_string(),
            gravity: 800.0,
            clock: super::super::native_mod_host::NativeModHostClock {
                now_milliseconds: Rc::new(|| 1000),
                performance_counter: Rc::new(|| 1),
                performance_frequency: 1,
            },
            navigation: Rc::new(RefCell::new(FakeNav)),
            collision: Rc::new(|_, _| {}),
            map_path: "maps/q2dm1.bsp".to_string(),
            spawn_point: "start".to_string(),
            entities: String::new(),
            engine: Rc::new(|_, _| panic!("no engine")),
        };
        let presentation = NativeModPresentation::new(
            ModEdition::Rerelease,
            NativeModPresentationOptions {
                admission,
                content: ContentId("q2:baseq3".to_string()),
                projection: Rc::new(FakeProjection {
                    slots: HashMap::new(),
                    owner: owner.clone(),
                }),
                services,
                context,
                owner: ProviderId {
                    namespace: "mod".to_string(),
                    name: "test".to_string(),
                },
                translate: Rc::new(|records, _| Ok(records.to_vec())),
                fog_from_wire: Rc::new(|previous, _| *previous),
            },
        )
        .expect("presentation");
        Harness {
            presentation,
            emitted,
            owner,
        }
    }

    fn active_state() -> NativeModEntityState {
        NativeModEntityState {
            active: true,
            visible: true,
            sound: 0,
            event: 9,
            origin: ZERO,
            volume: 0.0,
            attenuation: 0.0,
        }
    }

    #[test]
    fn publishes_entity_events_and_loops() {
        let mut harness = harness(None);
        let actor = harness.owner.actor(3, 1);
        let mut source = FakeSource {
            edition: ModEdition::Rerelease,
            configstrings: HashMap::from([(1000, "sound/world/hum.wav".to_string())]),
            messages: Vec::new(),
            states: HashMap::from([(
                5,
                NativeModEntityState {
                    sound: 2,
                    volume: 0.0,
                    ..active_state()
                },
            )]),
            player: player(),
        };
        // Sound configstring lives at layout.sounds + index; reroute the map.
        let sounds = harness.presentation.layout.sounds;
        source.configstrings = HashMap::from([(
            i32::try_from(sounds).expect("offset") + 2,
            "sound/world/hum.wav".to_string(),
        )]);
        harness
            .presentation
            .publish(&mut source, &[(actor.clone(), 5)], true)
            .expect("publish");
        let emitted = harness.emitted.borrow();
        assert!(emitted
            .iter()
            .any(|event| matches!(event, ModEngineEvent::EntityEvent { event: 9, .. })));
        assert!(emitted.iter().any(|event| matches!(
            event,
            ModEngineEvent::Sound(sound)
                if sound.loop_state == SoundLoopState::Start && sound.volume == 1.0
        )));
        // Re-publishing with the event cleared and sound stopped ends the loop.
        let mut stopped = HashMap::new();
        stopped.insert(
            5,
            NativeModEntityState {
                event: 0,
                ..active_state()
            },
        );
        source.states = stopped;
        drop(emitted);
        harness
            .presentation
            .publish(&mut source, &[(actor, 5)], true)
            .expect("publish");
        assert!(harness.emitted.borrow().iter().any(|event| matches!(
            event,
            ModEngineEvent::Sound(sound) if sound.loop_state == SoundLoopState::Stop
        )));
    }

    #[test]
    fn publishes_client_frames_with_admission() {
        let mut harness = harness(Some(NativeModClientPresentation {
            hud: NativeModHudPresentation::ReplaceStatus,
            view: NativeModViewPresentation::Playerstate,
        }));
        let actor = harness.owner.actor(1, 1);
        let mut source = FakeSource {
            edition: ModEdition::Rerelease,
            configstrings: HashMap::new(),
            messages: Vec::new(),
            states: HashMap::from([(
                1,
                NativeModEntityState {
                    event: 0,
                    ..active_state()
                },
            )]),
            player: player(),
        };
        harness
            .presentation
            .publish(&mut source, &[(actor.clone(), 1)], false)
            .expect("publish");
        match harness.presentation.client_frame(&actor).expect("frame") {
            ModClientPresentationFrame::Native { hud, view } => {
                let hud = hud.expect("hud");
                assert_eq!(hud.mode, NativeHudMode::ReplaceStatus);
                assert_eq!(hud.frame.stats, vec![100, 50]);
                assert_eq!(hud.frame.player_number, 0);
                assert!(view.is_some());
            }
            _ => panic!("expected native frame"),
        }
    }

    #[test]
    fn appearance_expands_attached_models() {
        let mut harness = harness(None);
        let actor = harness.owner.actor(3, 1);
        let mut source = FakeSource {
            edition: ModEdition::Classic,
            configstrings: HashMap::new(),
            messages: Vec::new(),
            states: HashMap::new(),
            player: player(),
        };
        let rows = harness
            .presentation
            .appearance(&mut source, &actor, 5)
            .expect("appearance");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, "models/5.md2");
        assert_eq!(rows[1].path, "models/gun.md2");
        assert_eq!(rows[1].skin, 0);
    }

    #[test]
    fn checkpoint_restore_roundtrip() {
        let mut first = harness(None);
        let actor = first.owner.actor(3, 1);
        first.presentation.fog.insert(actor.clone(), create_q2_fog());
        first.presentation.clients.insert(
            actor.clone(),
            ClientMessages {
                layout: "xv 0".to_string(),
                inventory: vec![1, 2],
            },
        );
        let saved = first.presentation.checkpoint();
        assert_eq!(saved.fog.len(), 1);
        assert_eq!(saved.clients.len(), 1);
        let mut fresh = harness(None);
        fresh.presentation.restore(&saved).expect("restore");
        let restored = fresh.owner.actor(3, 1);
        assert!(fresh.presentation.fog.contains_key(&restored));
        assert_eq!(fresh.presentation.clients[&restored].layout, "xv 0");
    }

    #[test]
    fn read_checkpoint_rejects_duplicates() {
        let fog = create_q2_fog();
        let entry = obj(vec![
            ("actor", write_saved_actor(SavedActorId { slot: 1, generation: 1 })),
            ("value", obj(vec![])),
        ]);
        let json = obj(vec![("fog", arr(vec![entry.clone(), entry])), ("clients", arr(vec![]))]);
        let reader = SaveReader::new(&json);
        let read_fog: ReadQ2FogStateFn = Rc::new(move |_| Ok(fog));
        assert!(read_native_mod_presentation(reader, &read_fog).is_err());
    }

    #[test]
    fn release_close_and_begin_frame() {
        let mut harness = harness(None);
        let actor = harness.owner.actor(3, 1);
        harness.presentation.fog.insert(actor.clone(), create_q2_fog());
        harness.presentation.entity_events.insert(actor.clone(), 4);
        harness.presentation.begin_frame();
        assert!(harness.presentation.entity_events.is_empty());
        assert_eq!(harness.presentation.generation(), 0);
        harness.presentation.release(&actor);
        assert!(!harness.presentation.fog.contains_key(&actor));
        harness.presentation.close();
        assert_eq!(harness.presentation.generation(), 1);
    }
}
