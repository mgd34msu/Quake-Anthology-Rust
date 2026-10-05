//! NetQuake remote presentation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/remote-q1.ts`
//! (`Q1RemotePresentation`). Decoded NetQuake records become presentation
//! state; this owner has no `Simulation` or combat table. The donor's
//! asynchronous content load resolves inline, preserving the donor's
//! receive order.
//!
//! Scoreboard rows mirror the out-of-scope donor
//! `../q1-service-presentation.ts` (`Q1ClientRow`,
//! `updateQ1ClientMetadata`); only the fields this presentation reads and
//! publishes are carried. Loaded content and the scene surface arrive
//! through [`Q1RemoteHost`]; the recipe handle is the shared
//! [`ExecutableRecipe`]. Published snapshots use
//! [`SimulationOutput`](qa_world::session::SimulationOutput); the donor's
//! `configurations` and `scene` blocks have no `qa-world` home, so they
//! travel beside the output as [`Q1RemoteSceneOutput`].
//!
//! The lane traits ([`Q1ApplicationClientHost`],
//! [`RemotePresentationAccess`]) are infallible, so donor `throw` paths
//! panic with the donor's message.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use qa_content::contract::{
    ArmorState, ContentId, InventoryEntry, PoweredProtectionState, ProviderReference, RegularArmorState,
    ResolvedResourceReference,
};
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_core::cmd::{ascii_fold, command_separator_offset, tokenize_command, Dialect, TextMode};
use qa_core::identity::{ActorId, IdentityOwner, ProviderId, SavedActorId};
use qa_core::math::{Bounds, Vec3};
use qa_core::time::{FrameContext, FramePhase, SourceTime};
use qa_net::common::commands::{ActorCommand, UserCommand};
use qa_net::q1_net::{
    NetQuakeClientData, NetQuakeMessage, NqNamedSlot, NqNumberedSlot, NqText, NqUnit, Q1WireEntity, TemporaryEntity,
};
use qa_world::body::BodyState;
use qa_world::movement::types::Q1UserCommand;
use qa_world::save::shared::{CharacterSelection, ProviderRef};
use qa_world::session::{
    EngineSession, SessionClient, SimEvent, SimEventPayload, SimulationOutput, SnapshotActor, SnapshotBody,
    SnapshotInventory,
};

use super::q1_client::Q1ApplicationClientHost;
use super::q1_demo::NetQuakeDemoRemote;
use super::remote_world::{
    lerp_angles, lerp_vec, provider_id, snapshot_entry, vec3, RemoteWorldContent, RemoteWorldError, ZERO,
};
use super::types::{
    ApplicationNetworkPlayer, ArsenalWarning, PlayerAmmo, PlayerArsenalItem, PlayerArsenalKind, PlayerUi, PlayerView,
    PresentationFamily, PresentationModel, PresentationPlayerColors, RemotePresentationAccess, WeaponStatus, WorldText,
};
use super::unified_event_codec::{Q1BeamStyle, Q1EffectKind, Q1Event, Q1NamedChannel, Q1SoundChannel};
use crate::persistence::recipe::{
    ExecutableRecipe, ExecutionImplementation, ResolvedResourceReference as RecipeResource,
};

/// Remote world description for content loading (donor `Q1RemoteWorld`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1RemoteWorld {
    /// World model path.
    pub map: String,
    /// Precached model paths.
    pub models: Vec<String>,
    /// Precached sound paths.
    pub sounds: Vec<String>,
}

/// Loaded-content surface the Q1 presentation reads.
pub trait Q1RemoteContent {
    /// Recipe the content was loaded from.
    fn recipe(&self) -> &ExecutableRecipe;
    /// World geometry identity (donor `content.world`).
    fn world_geometry(&self) -> &str;
}

/// Host callbacks behind [`Q1RemotePresentationOptions`].
pub trait Q1RemoteHost {
    /// Loaded content handle.
    type Content: Q1RemoteContent;
    /// Scene queries handle.
    type Scene;
    /// Load content for an admitted world (donor async; resolves inline).
    fn load_content(&mut self, world: &Q1RemoteWorld) -> Self::Content;
    /// Build scene queries for loaded content.
    fn build_scene(content: &Self::Content) -> Self::Scene;
    /// Send a console command to the server.
    fn send_command(&mut self, text: &str);
    /// Print server text.
    fn print(&mut self, text: &str);
    /// Publish a sampled output.
    fn publish(&mut self, output: &SimulationOutput);
    /// Handle a disconnect.
    fn disconnected(&mut self, reason: &str);
    /// Mint the next actor generation for a slot.
    fn next_generation(&self, slot: u32) -> u32;
    /// Presentation clock override, when the host drives time.
    fn presentation_time(&self) -> Option<u64> {
        None
    }
}

/// NetQuake remote presentation options (donor `Q1RemotePresentationOptions`).
pub struct Q1RemotePresentationOptions<H: Q1RemoteHost> {
    /// Identity authority.
    pub identity: IdentityOwner,
    /// Engine session.
    pub session: EngineSession,
    /// Bound client.
    pub client: SessionClient,
    /// Preloaded content, if any.
    pub content: Option<H::Content>,
    /// Host callbacks.
    pub host: H,
}

/// Scoreboard row (mirrors donor `Q1ClientRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ClientRow {
    /// Source slot.
    pub slot: u32,
    /// Player name.
    pub name: String,
    /// Packed colors.
    pub colors: i32,
    /// Frag count.
    pub frags: i32,
    /// Ping, when reported.
    pub ping: Option<i32>,
    /// Social handle, when reported.
    pub social: Option<String>,
    /// Player info, when reported.
    pub player_info: Option<String>,
}

/// Scoreboard metadata event (donor `Q1ClientMetadataEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1ClientMetadataEvent {
    /// Name update.
    Name {
        /// Source slot.
        slot: u32,
        /// Value.
        value: String,
    },
    /// Colors update.
    Colors {
        /// Source slot.
        slot: u32,
        /// Value.
        value: i32,
    },
    /// Frags update.
    Frags {
        /// Source slot.
        slot: u32,
        /// Value.
        value: i32,
    },
    /// Ping update.
    Ping {
        /// Source slot.
        slot: u32,
        /// Value.
        value: i32,
    },
    /// Social update.
    Social {
        /// Source slot.
        slot: u32,
        /// Value.
        value: String,
    },
    /// Player-info update.
    PlayerInfo {
        /// Source slot.
        slot: u32,
        /// Value.
        value: String,
    },
}

/// Update one scoreboard row (donor `updateQ1ClientMetadata`).
fn update_client_metadata(table: &mut HashMap<u32, Q1ClientRow>, event: &Q1ClientMetadataEvent) {
    let slot = match event {
        Q1ClientMetadataEvent::Name { slot, .. }
        | Q1ClientMetadataEvent::Colors { slot, .. }
        | Q1ClientMetadataEvent::Frags { slot, .. }
        | Q1ClientMetadataEvent::Ping { slot, .. }
        | Q1ClientMetadataEvent::Social { slot, .. }
        | Q1ClientMetadataEvent::PlayerInfo { slot, .. } => *slot,
    };
    let row = table.entry(slot).or_insert_with(|| Q1ClientRow {
        slot,
        name: String::new(),
        colors: 0,
        frags: 0,
        ping: None,
        social: None,
        player_info: None,
    });
    match event {
        Q1ClientMetadataEvent::Name { value, .. } => row.name = value.clone(),
        Q1ClientMetadataEvent::Colors { value, .. } => row.colors = *value,
        Q1ClientMetadataEvent::Frags { value, .. } => row.frags = *value,
        Q1ClientMetadataEvent::Ping { value, .. } => row.ping = Some(*value),
        Q1ClientMetadataEvent::Social { value, .. } => row.social = Some(value.clone()),
        Q1ClientMetadataEvent::PlayerInfo { value, .. } => row.player_info = Some(value.clone()),
    }
}

/// Presentation event payload (donor `SimulationPresentationEvent` kinds).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1RemoteEvent {
    /// Q1 source event.
    Q1(Q1Event),
    /// Scoreboard metadata event.
    ClientMetadata(Q1ClientMetadataEvent),
    /// Skybox selection.
    Sky {
        /// Sky name.
        name: String,
    },
    /// Music track.
    Music {
        /// CD track.
        track: u8,
    },
    /// View reset.
    ViewReset {
        /// Player actor.
        actor: ActorId,
        /// Reset angles.
        angles: Vec3,
    },
}

/// Routed presentation event (donor `SimulationPresentationEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1RemotePresentationEvent {
    /// Event sequence.
    pub sequence: u64,
    /// Source seconds.
    pub seconds: f64,
    /// Source content.
    pub content: ContentId,
    /// Source entity, when the event names one.
    pub source_entity: Option<u32>,
    /// Payload.
    pub event: Q1RemoteEvent,
}

/// Actor configuration block (donor snapshot `configurations` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1RemoteConfiguration {
    /// Configured actor.
    pub actor: ActorId,
    /// Movement provider.
    pub movement: ProviderRef,
    /// Character selection.
    pub character: CharacterSelection,
    /// Weapon providers.
    pub weapons: Vec<ProviderRef>,
    /// Inventory provider.
    pub inventory: ProviderRef,
}

/// Animated light style (donor scene `lightStyles` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1RemoteLightStyle {
    /// Style index.
    pub style: u8,
    /// Animated value.
    pub value: f64,
}

/// Donor snapshot blocks with no `qa-world` home.
#[derive(Debug, Clone, PartialEq)]
pub struct Q1RemoteSceneOutput {
    /// World geometry resource.
    pub world_resource: RecipeResource,
    /// World geometry identity.
    pub world_geometry: String,
    /// Actor configurations.
    pub configurations: Vec<Q1RemoteConfiguration>,
    /// Animated light styles by style index.
    pub light_styles: Vec<Q1RemoteLightStyle>,
}

/// NetQuake remote presentation (donor `Q1RemotePresentation`).
pub struct Q1RemotePresentation<H: Q1RemoteHost> {
    options: Q1RemotePresentationOptions<H>,
    world: RemoteWorldContent<H::Content, H::Scene, ()>,
    ordinal: Cell<u32>,
    sequence: u64,
    frame_number: i32,
    pending_impulse: Cell<i32>,
    seconds: f64,
    previous_seconds: f64,
    received_at: u64,
    fraction: f64,
    max_clients: u32,
    view_entity: u32,
    view_angles: Cell<Vec3>,
    demo_seconds: Option<f64>,
    demo_angles: Option<(Vec3, Vec3)>,
    data: Option<NetQuakeClientData>,
    weapon_alpha: u8,
    actors: RefCell<HashMap<u32, ActorId>>,
    current: HashMap<u32, Q1WireEntity>,
    previous: HashMap<u32, Q1WireEntity>,
    statics: Vec<Q1WireEntity>,
    models: Vec<String>,
    sounds: Vec<String>,
    resources: HashMap<String, ResolvedResourceReference>,
    styles: HashMap<u8, String>,
    events: Vec<Q1RemotePresentationEvent>,
    sounds_pending: Vec<SimEvent>,
    published: Option<SimulationOutput>,
    published_scene: Option<Q1RemoteSceneOutput>,
    scoreboard: HashMap<u32, Q1ClientRow>,
    records: Vec<NetQuakeMessage>,
}

/// Native weapon table: (selection bit, name, ammunition or `None`).
const WEAPONS: [(u32, &str, Option<&str>); 8] = [
    (4096, "axe", None),
    (1, "shotgun", Some("shells")),
    (2, "supershotgun", Some("shells")),
    (4, "nailgun", Some("nails")),
    (8, "supernailgun", Some("nails")),
    (16, "grenadelauncher", Some("rockets")),
    (32, "rocketlauncher", Some("rockets")),
    (64, "lightning", Some("cells")),
];

/// Decode an entity alpha byte (donor `ENTALPHA_DECODE`).
fn decode_alpha(alpha: u8) -> f64 {
    if alpha == 0 {
        1.0
    } else {
        f64::from(alpha - 1) / 254.0
    }
}

/// Decode an entity scale byte (donor `ENTSCALE_DECODE`).
fn decode_scale(scale: u8) -> f64 {
    f64::from(scale) / 16.0
}

fn sound_channel(value: u8) -> Q1SoundChannel {
    match value {
        0 => Q1SoundChannel::Named(Q1NamedChannel::Auto),
        1 => Q1SoundChannel::Named(Q1NamedChannel::Weapon),
        2 => Q1SoundChannel::Named(Q1NamedChannel::Voice),
        3 => Q1SoundChannel::Named(Q1NamedChannel::Item),
        4 => Q1SoundChannel::Named(Q1NamedChannel::Body),
        5..=7 => Q1SoundChannel::Number(i64::from(value)),
        _ => panic!("Invalid NetQuake sound channel"),
    }
}

impl<H: Q1RemoteHost + 'static> Q1RemotePresentation<H> {
    /// Wrap presentation options.
    pub fn new(mut options: Q1RemotePresentationOptions<H>) -> Self {
        let world = RemoteWorldContent::new(options.content.take(), |content, _| H::build_scene(content));
        Self {
            options,
            world,
            ordinal: Cell::new(0),
            sequence: 0,
            frame_number: 0,
            pending_impulse: Cell::new(0),
            seconds: 0.0,
            previous_seconds: 0.0,
            received_at: 0,
            fraction: 1.0,
            max_clients: 0,
            view_entity: 0,
            view_angles: Cell::new(ZERO),
            demo_seconds: None,
            demo_angles: None,
            data: None,
            weapon_alpha: 0,
            actors: RefCell::new(HashMap::new()),
            current: HashMap::new(),
            previous: HashMap::new(),
            statics: Vec::new(),
            models: Vec::new(),
            sounds: Vec::new(),
            resources: HashMap::new(),
            styles: HashMap::new(),
            events: Vec::new(),
            sounds_pending: Vec::new(),
            published: None,
            published_scene: None,
            scoreboard: HashMap::new(),
            records: Vec::new(),
        }
    }

    /// Borrow the bound client.
    #[must_use]
    pub fn client(&self) -> &SessionClient {
        &self.options.client
    }

    /// Borrow the session name.
    #[must_use]
    pub fn session_name(&self) -> &str {
        self.options.session.session().name()
    }

    /// Borrow the scene queries, building them lazily from the content.
    pub fn scene(&mut self) -> Result<&H::Scene, RemoteWorldError> {
        self.world.scene()
    }

    /// Mutably borrow the scene queries (QuakeWorld brush linking).
    pub fn scene_mut(&mut self) -> Result<&mut H::Scene, RemoteWorldError> {
        self.world.scene_mut()
    }

    /// Borrow the identity authority.
    #[must_use]
    pub fn identity(&self) -> &IdentityOwner {
        &self.options.identity
    }

    /// Borrow the published output, if any.
    #[must_use]
    pub fn output(&self) -> Option<&SimulationOutput> {
        self.published.as_ref()
    }

    /// Borrow the published scene blocks, if any.
    #[must_use]
    pub fn scene_output(&self) -> Option<&Q1RemoteSceneOutput> {
        self.published_scene.as_ref()
    }

    /// Borrow the retained source records.
    #[must_use]
    pub fn source_records(&self) -> &[NetQuakeMessage] {
        &self.records
    }

    /// Borrow the scoreboard table.
    #[must_use]
    pub fn scoreboard(&self) -> &HashMap<u32, Q1ClientRow> {
        &self.scoreboard
    }

    /// Borrow the view player, if a view entity is set.
    #[must_use]
    pub fn player(&self) -> Option<ApplicationNetworkPlayer> {
        if self.view_entity == 0 {
            return None;
        }
        Some(ApplicationNetworkPlayer {
            client: self.options.client.id().clone(),
            actor: self.actor(self.view_entity),
            source_entity: self.view_entity,
        })
    }

    /// Borrow the loaded content.
    pub fn content(&self) -> &H::Content {
        self.world.content().expect("remote content is loaded")
    }

    fn map_content(&self) -> ContentId {
        ContentId(self.content().recipe().map.entities.content.clone())
    }

    fn source_content(&self) -> ContentId {
        let recipe = self.content().recipe();
        let quakec = recipe
            .execution
            .iter()
            .find(|execution| matches!(execution.implementation, ExecutionImplementation::Quakec { .. }))
            .map(|execution| execution.owner.content.clone());
        ContentId(quakec.unwrap_or_else(|| recipe.map.entities.content.clone()))
    }

    /// Borrow a registered actor without creating it.
    #[must_use]
    pub fn actor_at(&self, number: u32) -> Option<ActorId> {
        self.actors.borrow().get(&number).cloned()
    }

    fn actor(&self, number: u32) -> ActorId {
        if let Some(found) = self.actor_at(number) {
            return found;
        }
        if self.ordinal.get() >= 65536 {
            panic!("Remote actor registry is full");
        }
        let slot = self.ordinal.get();
        self.ordinal.set(slot + 1);
        let value = self
            .options
            .identity
            .actor(slot, self.options.host.next_generation(slot));
        self.actors.borrow_mut().insert(number, value.clone());
        value
    }

    /// Resolve the player slot behind an actor.
    #[must_use]
    pub fn player_slot(&self, actor: &ActorId) -> Option<u32> {
        for number in 1..=self.max_clients {
            if self.actors.borrow().get(&number) == Some(actor) {
                return Some(number - 1);
            }
        }
        None
    }

    /// Whether an actor is a player.
    #[must_use]
    pub fn is_player(&self, actor: &ActorId) -> bool {
        self.player_slot(actor).is_some()
    }

    fn push_event(&mut self, event: Q1RemoteEvent, source_entity: Option<u32>) {
        let sequence = self.sequence;
        self.sequence += 1;
        self.events.push(Q1RemotePresentationEvent {
            sequence,
            seconds: self.seconds,
            content: self.map_content(),
            source_entity,
            event,
        });
    }

    fn emit(&mut self, event: Q1Event, source_entity: Option<u32>) {
        self.push_event(Q1RemoteEvent::Q1(event), source_entity);
    }

    fn bonus_flash(&mut self) {
        let actor = self.actor(self.view_entity);
        let origin = self
            .current
            .get(&self.view_entity)
            .map(|state| vec3(state.state.origin))
            .unwrap_or(ZERO);
        self.emit(
            Q1Event::Effect {
                effect: Q1EffectKind::Pickup,
                actor: Some(actor),
                origin,
                amount: 1.0,
                muzzle: None,
            },
            Some(self.view_entity),
        );
    }

    fn sampled(&self, state: &Q1WireEntity) -> Q1WireEntity {
        let old = self.previous.get(&state.number);
        let teleported = old.is_some_and(|old| {
            [0, 1, 2]
                .iter()
                .any(|&index| (state.state.origin[index] - old.state.origin[index]).abs() > 100.0)
        });
        if state.step || old.is_none() || teleported {
            return state.clone();
        }
        let old = old.expect("previous entity is present");
        let mut sampled = state.clone();
        sampled.state.origin = {
            let origin = lerp_vec(vec3(old.state.origin), vec3(state.state.origin), self.fraction);
            [f64::from(origin.x), f64::from(origin.y), f64::from(origin.z)]
        };
        sampled.state.angles = {
            let angles = lerp_angles(vec3(old.state.angles), vec3(state.state.angles), self.fraction);
            [f64::from(angles.x), f64::from(angles.y), f64::from(angles.z)]
        };
        sampled
    }

    /// Receive decoded server messages (donor `receive`).
    pub fn receive(&mut self, messages: &[NetQuakeMessage], now_milliseconds: u64) {
        self.records = messages.to_vec();
        for message in messages {
            match message {
                NetQuakeMessage::ServerInfo {
                    models,
                    sounds,
                    max_clients,
                    ..
                } => {
                    let map = models.first();
                    let valid = map.is_some_and(|map| map.starts_with("maps/") && map.ends_with(".bsp"));
                    if !valid {
                        panic!("NetQuake server has no world model");
                    }
                    let map = map.expect("world model is present").clone();
                    let loaded = self.options.host.load_content(&Q1RemoteWorld {
                        map,
                        models: models.clone(),
                        sounds: sounds.clone(),
                    });
                    self.world.set_content(loaded);
                    self.actors.borrow_mut().clear();
                    self.ordinal.set(0);
                    self.current.clear();
                    self.previous.clear();
                    self.statics.clear();
                    self.styles.clear();
                    self.scoreboard.clear();
                    self.events.clear();
                    self.sounds_pending.clear();
                    self.models.clone_from(models);
                    self.sounds.clone_from(sounds);
                    self.max_clients = u32::from(*max_clients);
                    self.view_entity = 0;
                    self.pending_impulse.set(0);
                    self.view_angles.set(ZERO);
                    self.demo_angles = None;
                    self.demo_seconds = None;
                    self.data = None;
                    self.weapon_alpha = 0;
                    self.published = None;
                    self.published_scene = None;
                    self.seconds = 0.0;
                    self.previous_seconds = 0.0;
                }
                NetQuakeMessage::Time { seconds } => {
                    self.previous_seconds = self.seconds;
                    self.seconds = f64::from(*seconds);
                    self.previous = std::mem::take(&mut self.current);
                    self.received_at = self.options.host.presentation_time().unwrap_or(now_milliseconds);
                    self.fraction = 1.0;
                    self.frame_number += 1;
                }
                NetQuakeMessage::Entity { state } => {
                    self.current.insert(state.number, state.clone());
                }
                NetQuakeMessage::Static { state } => {
                    self.statics.push(state.clone());
                }
                NetQuakeMessage::SetView { entity } => {
                    self.view_entity = u32::from(*entity);
                }
                NetQuakeMessage::SetAngle { angles } => {
                    let angles = vec3(*angles);
                    self.view_angles.set(angles);
                    if let Some(player) = self.player() {
                        let sequence = self.sequence;
                        self.sequence += 1;
                        let content = self.map_content();
                        self.events.push(Q1RemotePresentationEvent {
                            sequence,
                            seconds: self.seconds,
                            content,
                            source_entity: None,
                            event: Q1RemoteEvent::ViewReset {
                                actor: player.actor,
                                angles,
                            },
                        });
                    }
                }
                NetQuakeMessage::ClientData { data, weapon_alpha } => {
                    self.data = Some(data.clone());
                    self.weapon_alpha = *weapon_alpha;
                }
                NetQuakeMessage::LightStyle { index, value } => {
                    self.styles.insert(*index, value.clone());
                }
                NetQuakeMessage::CdTrack { track, .. } => {
                    let sequence = self.sequence;
                    self.sequence += 1;
                    let content = self.map_content();
                    self.events.push(Q1RemotePresentationEvent {
                        sequence,
                        seconds: self.seconds,
                        content,
                        source_entity: None,
                        event: Q1RemoteEvent::Music { track: *track },
                    });
                }
                NetQuakeMessage::NamedSlot { kind, slot, value } => {
                    let event = match kind {
                        NqNamedSlot::Name => Q1ClientMetadataEvent::Name {
                            slot: u32::from(*slot),
                            value: value.clone(),
                        },
                        NqNamedSlot::Social => Q1ClientMetadataEvent::Social {
                            slot: u32::from(*slot),
                            value: value.clone(),
                        },
                        NqNamedSlot::PlayerInfo => Q1ClientMetadataEvent::PlayerInfo {
                            slot: u32::from(*slot),
                            value: value.clone(),
                        },
                    };
                    update_client_metadata(&mut self.scoreboard, &event);
                    let sequence = self.sequence;
                    self.sequence += 1;
                    let content = self.source_content();
                    self.events.push(Q1RemotePresentationEvent {
                        sequence,
                        seconds: self.seconds,
                        content,
                        source_entity: None,
                        event: Q1RemoteEvent::ClientMetadata(event),
                    });
                }
                NetQuakeMessage::NumberedSlot { kind, slot, value } => {
                    let event = match kind {
                        NqNumberedSlot::Frags => Q1ClientMetadataEvent::Frags {
                            slot: u32::from(*slot),
                            value: i32::from(*value),
                        },
                        NqNumberedSlot::Colors => Q1ClientMetadataEvent::Colors {
                            slot: u32::from(*slot),
                            value: i32::from(*value),
                        },
                        NqNumberedSlot::Ping => Q1ClientMetadataEvent::Ping {
                            slot: u32::from(*slot),
                            value: i32::from(*value),
                        },
                    };
                    update_client_metadata(&mut self.scoreboard, &event);
                    let sequence = self.sequence;
                    self.sequence += 1;
                    let content = self.source_content();
                    self.events.push(Q1RemotePresentationEvent {
                        sequence,
                        seconds: self.seconds,
                        content,
                        source_entity: None,
                        event: Q1RemoteEvent::ClientMetadata(event),
                    });
                }
                NetQuakeMessage::Text { kind, text } => match kind {
                    NqText::Print => self.options.host.print(text),
                    NqText::CenterPrint => {
                        if let Some(player) = self.player() {
                            self.emit(
                                Q1Event::Message {
                                    player: player.actor,
                                    text: text.clone(),
                                    center: true,
                                    args: None,
                                    parts: None,
                                },
                                None,
                            );
                        }
                    }
                    NqText::Skybox => {
                        let sequence = self.sequence;
                        self.sequence += 1;
                        let content = self.source_content();
                        self.events.push(Q1RemotePresentationEvent {
                            sequence,
                            seconds: self.seconds,
                            content,
                            source_entity: None,
                            event: Q1RemoteEvent::Sky { name: text.clone() },
                        });
                    }
                    NqText::Stufftext => self.receive_stufftext(text),
                    _ => {}
                },
                NetQuakeMessage::Sound {
                    entity,
                    channel,
                    index,
                    volume,
                    attenuation,
                    origin,
                } => {
                    let path = self
                        .sounds
                        .get(usize::from(index.saturating_sub(1)))
                        .cloned()
                        .unwrap_or_else(|| panic!("Unknown NetQuake sound index {index}"));
                    if *index == 0 {
                        panic!("Unknown NetQuake sound index {index}");
                    }
                    self.emit(
                        Q1Event::Sound {
                            origin: Some(vec3(*origin)),
                            actor: self.actor(u32::from(*entity)),
                            path: path.clone(),
                            channel: sound_channel(*channel),
                            attenuation: *attenuation,
                            volume: f64::from(*volume) / 255.0,
                        },
                        Some(u32::from(*entity)),
                    );
                    let resource = self
                        .resources
                        .get(&format!("sound/{path}"))
                        .unwrap_or_else(|| panic!("Unresolved NetQuake sound {path}"));
                    let sequence = self.sequence;
                    self.sequence += 1;
                    self.sounds_pending.push(SimEvent {
                        sequence,
                        time: SourceTime::Seconds(self.seconds as f32),
                        payload: SimEventPayload::Sound {
                            resource: resource.id.as_str().to_string(),
                            actor: (*entity != 0).then(|| SavedActorId::from(&self.actor(u32::from(*entity)))),
                            origin: vec3(*origin),
                            channel: *channel,
                            volume: f64::from(*volume) / 255.0,
                            attenuation: *attenuation,
                        },
                    });
                }
                NetQuakeMessage::StaticSound {
                    index,
                    volume,
                    attenuation,
                    origin,
                } => {
                    let path = self
                        .sounds
                        .get(usize::from(index.saturating_sub(1)))
                        .cloned()
                        .unwrap_or_else(|| panic!("Unknown NetQuake sound index {index}"));
                    if *index == 0 {
                        panic!("Unknown NetQuake sound index {index}");
                    }
                    self.emit(
                        Q1Event::Ambient {
                            origin: vec3(*origin),
                            path,
                            volume: f64::from(*volume) / 255.0,
                            attenuation: *attenuation,
                        },
                        None,
                    );
                }
                NetQuakeMessage::StopSound { entity, channel } => {
                    self.emit(
                        Q1Event::StopSound {
                            actor: self.actor(u32::from(*entity)),
                            channel: f64::from(*channel),
                        },
                        Some(u32::from(*entity)),
                    );
                }
                NetQuakeMessage::Particle {
                    origin,
                    direction,
                    count,
                    color,
                } => {
                    self.emit(
                        Q1Event::Particles {
                            origin: vec3(*origin),
                            direction: vec3(*direction),
                            color: f64::from(*color),
                            count: f64::from(*count),
                        },
                        None,
                    );
                }
                NetQuakeMessage::TemporaryEntity { effect } => self.receive_effect(effect),
                NetQuakeMessage::Unit(NqUnit::BonusFlash) => self.bonus_flash(),
                NetQuakeMessage::Unit(_) => {}
                _ => {}
            }
        }
        self.publish();
    }

    fn receive_effect(&mut self, effect: &TemporaryEntity) {
        match effect {
            TemporaryEntity::ExplosionColors {
                origin,
                color_start,
                color_length,
            } => {
                self.emit(
                    Q1Event::ColoredExplosion {
                        origin: vec3(*origin),
                        color_start: f64::from(*color_start),
                        color_length: f64::from(*color_length),
                    },
                    None,
                );
            }
            TemporaryEntity::Beam {
                effect_type,
                entity,
                start,
                end,
            } => {
                let style = match effect_type {
                    5 => Q1BeamStyle::Lightning1,
                    6 => Q1BeamStyle::Lightning2,
                    9 => Q1BeamStyle::Lightning3,
                    _ => Q1BeamStyle::Grapple,
                };
                self.emit(
                    Q1Event::Beam {
                        style,
                        actor: self.actor(u32::from(*entity)),
                        start: vec3(*start),
                        end: vec3(*end),
                    },
                    None,
                );
            }
            TemporaryEntity::Point {
                effect_type,
                origin,
                count,
            } => {
                let kind = match effect_type {
                    0 => Q1EffectKind::Spike,
                    1 => Q1EffectKind::Superspike,
                    2 => Q1EffectKind::Gunshot,
                    3 => Q1EffectKind::Explosion,
                    4 => Q1EffectKind::TarExplosion,
                    7 => Q1EffectKind::WizardSpike,
                    8 => Q1EffectKind::KnightSpike,
                    10 => Q1EffectKind::LavaSplash,
                    11 => Q1EffectKind::Teleport,
                    _ => panic!("Unsupported NetQuake temporary entity {effect_type}"),
                };
                self.emit(
                    Q1Event::Effect {
                        effect: kind,
                        actor: None,
                        origin: vec3(*origin),
                        amount: f64::from(*count),
                        muzzle: None,
                    },
                    None,
                );
            }
        }
    }

    fn receive_stufftext(&mut self, text: &str) {
        let mut pending = text;
        while !pending.is_empty() {
            let offset = command_separator_offset(pending, Dialect::Q1Netquake);
            let line = &pending[..offset];
            pending = if offset < pending.len() {
                &pending[offset + 1..]
            } else {
                ""
            };
            let name = tokenize_command(line, Dialect::Q1Netquake, TextMode::Source)
                .map(|tokens| tokens.argv.into_iter().next().unwrap_or_default())
                .unwrap_or_default();
            if ascii_fold(&name) == "bf" {
                self.bonus_flash();
            } else if line.trim() == "reconnect" {
                self.published = None;
                self.published_scene = None;
            } else if !name.is_empty() {
                self.options.host.print(&format!("Unhandled server command: {line}\n"));
            }
        }
    }

    fn require_data(&self, actor: &ActorId) -> (ApplicationNetworkPlayer, NetQuakeClientData) {
        let player = self.player();
        let data = self.data.clone();
        match (player, data) {
            (Some(player), Some(data)) if player.actor == *actor => (player, data),
            _ => panic!("Remote Q1 player has no clientdata"),
        }
    }

    /// Camera view for an actor (donor `playerView`).
    pub fn player_view(&self, actor: &ActorId) -> PlayerView {
        let (player, data) = self.require_data(actor);
        let origin = self
            .current
            .get(&player.source_entity)
            .map(|state| vec3(self.sampled(state).state.origin))
            .unwrap_or(ZERO);
        PlayerView {
            origin,
            angles: self.view_angles.get(),
            view_height: f64::from(data.view_height),
            blend: None,
            damage_blend: None,
            kick_angles: Some(Vec3 {
                x: f32::from(data.punch_angles[0]),
                y: f32::from(data.punch_angles[1]),
                z: f32::from(data.punch_angles[2]),
            }),
            field_of_view: None,
            client_view_offset_delta: None,
            foreign_character_death: None,
            pitch_drift: Some(super::types::PlayerPitchDrift {
                grounded: data.on_ground,
                ideal_pitch: f64::from(data.ideal_pitch),
                disabled: self.demo_seconds.is_some(),
            }),
        }
    }

    fn selected_weapon(&self, data: &NetQuakeClientData) -> Option<(u32, &'static str, Option<&'static str>)> {
        let axe_model = data
            .weapon_model
            .checked_sub(1)
            .and_then(|index| self.models.get(usize::from(index)));
        WEAPONS.into_iter().find(|(bit, name, _)| {
            *bit == data.active_weapon
                || (*name == "axe"
                    && data.active_weapon == 0
                    && axe_model.is_some_and(|path| path == "progs/v_axe.mdl"))
        })
    }

    /// HUD state for an actor (donor `playerUi`).
    #[must_use]
    pub fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        let (_, data) = self.require_data(actor);
        let weapon = self.selected_weapon(&data);
        let source = self.content().recipe().weapons.first().cloned();
        let counts = [
            ("shells", f64::from(data.shells)),
            ("nails", f64::from(data.nails)),
            ("rockets", f64::from(data.rockets)),
            ("cells", f64::from(data.cells)),
        ];
        let count_of = |name: &str| {
            counts
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, count)| *count)
                .unwrap_or(0.0)
        };
        let item = weapon.map(|(_, name, _)| format!("q1:weapon/{name}"));
        let ammo_name = weapon.and_then(|(_, _, ammo)| ammo);
        let regular = if data.armor == 0 {
            RegularArmorState::None
        } else {
            RegularArmorState::Q1 {
                points: f64::from(data.armor),
                absorption: if data.items & 32768 != 0 {
                    0.8
                } else if data.items & 16384 != 0 {
                    0.6
                } else {
                    0.3
                },
                item: "q1:armor".to_string(),
            }
        };
        let weapon_status = match (&weapon, &source, &item) {
            (Some((_, name, _)), Some(source), Some(item)) => {
                let (namespace, provider) = source.provider.split_once(':').unwrap_or(("", ""));
                Some(WeaponStatus {
                    source: ProviderReference {
                        provider: ProviderId::new(namespace, provider),
                        content: ContentId(source.content.clone()),
                    },
                    item: item.clone(),
                    label: (*name).to_string(),
                    ammo: match ammo_name {
                        None => super::types::WeaponAmmoStatus::Unmetered,
                        Some(ammo) => super::types::WeaponAmmoStatus::finite(
                            format!("q1:ammo/{ammo}"),
                            f64::from(data.ammo),
                            data.ammo > 0,
                            false,
                        ),
                    },
                })
            }
            _ => None,
        };
        PlayerUi {
            selected_arsenal: None,
            native_inventory: None,
            health: f64::from(data.health),
            armor: ArmorState {
                regular,
                powered: PoweredProtectionState::None,
            },
            active_weapon: item,
            ammo: ammo_name.map(|ammo| PlayerAmmo {
                item: format!("q1:ammo/{ammo}"),
                count: f64::from(data.ammo),
            }),
            inventory: counts
                .iter()
                .map(|(name, count)| InventoryEntry {
                    item: format!("q1:ammo/{name}"),
                    count: *count,
                    capacity: *count,
                    count_policy: None,
                })
                .collect(),
            arsenal_warning: ArsenalWarning::None,
            powerups: Vec::new(),
            items: WEAPONS
                .into_iter()
                .enumerate()
                .map(|(ordinal, (bit, name, ammo))| {
                    let owned = data.items as u32 & bit != 0;
                    PlayerArsenalItem {
                        id: format!("q1:weapon/{name}"),
                        label: name.to_string(),
                        kind: PlayerArsenalKind::Weapon,
                        source_ordinal: (ordinal + 1) as f64,
                        owned,
                        has_ammo: ammo.is_none_or(|ammo| count_of(ammo) > 0.0),
                        count: ammo.map(count_of),
                        warning_count: 0.0,
                    }
                })
                .collect(),
            weapon_status,
        }
    }

    /// Visible presentations (donor `presentations`).
    #[must_use]
    pub fn presentations(&self) -> Vec<PresentationModel> {
        let mut result = Vec::new();
        let mut append = |state: &Q1WireEntity, number: u32, view_entity: u32| {
            if state.state.modelindex == 0 {
                return;
            }
            let path = self
                .models
                .get(usize::from(state.state.modelindex - 1))
                .unwrap_or_else(|| panic!("Unknown NetQuake model {}", state.state.modelindex))
                .clone();
            let colors = if state.state.colormap > 0 && u32::from(state.state.colormap) <= self.max_clients {
                self.scoreboard
                    .get(&(u32::from(state.state.colormap) - 1))
                    .map(|row| row.colors)
            } else {
                None
            };
            result.push(PresentationModel {
                actor: self.actor(number),
                content: self.map_content(),
                family: PresentationFamily::Q1,
                path,
                frame: i64::from(state.state.frame),
                old_frame: i64::from(state.state.frame),
                skin: i64::from(state.state.skin),
                effects: i64::from(state.state.effects),
                render_flags: 0,
                origin: vec3(state.state.origin),
                angles: vec3(state.state.angles),
                scale: decode_scale(state.state.scale),
                visible: number != view_entity,
                view_weapon: false,
                replaces_body: None,
                render_owner: None,
                held_weapon: None,
                native_held_weapon: None,
                weapon_item: None,
                flare: None,
                back_lerp: None,
                skin_path: None,
                indexed_skin: None,
                player_colors: colors.map(|colors| PresentationPlayerColors {
                    top: i64::from((colors >> 4) & 15).min(13),
                    bottom: i64::from(colors & 15).min(13),
                }),
                previous_origin: None,
                model_beam: None,
                shader_beam: None,
                model_attachments: None,
                model_anchor: None,
                q3_grapple_cable: None,
                alpha: Some(decode_alpha(state.state.alpha)),
                q3_weapon: None,
            });
        };
        let states: Vec<(Q1WireEntity, u32)> = self
            .current
            .values()
            .map(|state| (self.sampled(state), state.number))
            .collect();
        for (state, number) in &states {
            append(state, *number, self.view_entity);
        }
        for (index, state) in self.statics.clone().into_iter().enumerate() {
            append(&state, 65536 + index as u32, self.view_entity);
        }
        if let (Some(player), Some(data)) = (self.player(), self.data.clone()) {
            if data.weapon_model != 0 {
                let path = self
                    .models
                    .get(usize::from(data.weapon_model - 1))
                    .unwrap_or_else(|| panic!("Unknown Q1 weapon model"))
                    .clone();
                let view = self.player_view(&player.actor);
                result.push(PresentationModel {
                    actor: player.actor,
                    content: self.map_content(),
                    family: PresentationFamily::Q1,
                    path,
                    frame: i64::from(data.weapon_frame),
                    old_frame: i64::from(data.weapon_frame),
                    skin: 0,
                    effects: 0,
                    render_flags: 0,
                    origin: Vec3 {
                        x: view.origin.x,
                        y: view.origin.y,
                        z: view.origin.z + view.view_height as f32,
                    },
                    angles: view.angles,
                    scale: 1.0,
                    visible: data.health > 0 && data.items as u32 & 524288 == 0,
                    view_weapon: true,
                    replaces_body: None,
                    render_owner: None,
                    held_weapon: None,
                    native_held_weapon: None,
                    weapon_item: None,
                    flare: None,
                    back_lerp: None,
                    skin_path: None,
                    indexed_skin: None,
                    player_colors: None,
                    previous_origin: None,
                    model_beam: None,
                    shader_beam: None,
                    model_attachments: None,
                    model_anchor: None,
                    q3_grapple_cable: None,
                    alpha: Some(decode_alpha(self.weapon_alpha)),
                    q3_weapon: None,
                });
            }
        }
        result
    }

    /// Convert an actor command into a wire command (donor `command`).
    #[must_use]
    pub fn command(&self, command: &ActorCommand) -> Q1UserCommand {
        let UserCommand::Q1Netquake {
            acknowledged_server_time_seconds,
            view_angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } = &command.command
        else {
            panic!("Native Q1 requires NetQuake input");
        };
        self.player_view(&command.actor);
        self.view_angles.set(vec3(*view_angles));
        if let Some(requested) = command.arsenal.as_ref().and_then(|arsenal| arsenal.weapon.clone()) {
            if Some(requested.clone()) != self.player_ui(&command.actor).active_weapon {
                self.select_weapon(&command.actor, &requested);
            }
        }
        let impulse = if *impulse != 0.0 {
            *impulse as i32
        } else {
            self.pending_impulse.get()
        };
        self.pending_impulse.set(0);
        Q1UserCommand {
            acknowledged_server_time_seconds: *acknowledged_server_time_seconds,
            view_angles: vec3(*view_angles),
            forward_move: *forward_move,
            side_move: *side_move,
            up_move: *up_move,
            buttons: *buttons as i32,
            impulse,
        }
    }

    fn select_weapon(&self, actor: &ActorId, requested: &str) {
        let selected = self
            .player_ui(actor)
            .items
            .into_iter()
            .find(|item| item.owned && (item.id == requested || item.label == requested));
        match selected {
            Some(item) => self.pending_impulse.set(item.source_ordinal as i32),
            None => panic!("Native Q1 weapon is not owned: {requested}"),
        }
    }

    /// Run a player command (donor `playerCommand`).
    pub fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        self.player_view(actor);
        if std::iter::once(name)
            .chain(args.iter().map(String::as_str))
            .any(|value| value.contains(['"', '\n', '\r', ';']))
        {
            panic!("Invalid native command delimiter");
        }
        if name == "use" {
            let requested = args.concat().to_lowercase().replace(' ', "");
            self.select_weapon(actor, &requested);
            return;
        }
        if name == "weapnext" || name == "weapprev" {
            let ui = self.player_ui(actor);
            let owned: Vec<_> = ui.items.iter().filter(|item| item.owned && item.has_ammo).collect();
            let current = owned
                .iter()
                .position(|item| Some(item.id.clone()) == ui.active_weapon)
                .unwrap_or(0);
            if !owned.is_empty() {
                let step = if name == "weapnext" { 1 } else { owned.len() - 1 };
                if let Some(next) = owned.get((current + step) % owned.len()) {
                    self.pending_impulse.set(next.source_ordinal as i32);
                }
            }
            return;
        }
        let mut text = name.to_string();
        for arg in args {
            text.push_str(&format!(r#" "{arg}""#));
        }
        self.options.host.send_command(&text);
    }

    /// Register a resolved resource (donor `registerResource`).
    pub fn register_resource(&mut self, _content: &ContentId, path: &str, resource: &ResolvedResourceReference) {
        self.resources.insert(path.to_string(), resource.clone());
    }

    fn publish(&mut self) {
        let (Some(player), Some(data)) = (self.player(), self.data.clone()) else {
            return;
        };
        if !self.current.contains_key(&self.view_entity) {
            return;
        }
        let recipe = self.content().recipe().clone();
        let time = self.previous_seconds + (self.seconds - self.previous_seconds) * self.fraction;
        let velocity = Vec3 {
            x: f32::from(data.velocity[0]),
            y: f32::from(data.velocity[1]),
            z: f32::from(data.velocity[2]),
        };
        let bodies: Vec<SnapshotBody> = self
            .current
            .values()
            .map(|state| {
                let sampled = self.sampled(state);
                SnapshotBody {
                    id: SavedActorId::from(&self.actor(state.number)),
                    state: BodyState {
                        origin: vec3(sampled.state.origin),
                        angles: vec3(sampled.state.angles),
                        velocity: if state.number == self.view_entity {
                            velocity
                        } else {
                            ZERO
                        },
                        bounds: Bounds {
                            min: Vec3 {
                                x: -16.0,
                                y: -16.0,
                                z: -24.0,
                            },
                            max: Vec3 {
                                x: 16.0,
                                y: 16.0,
                                z: 32.0,
                            },
                        },
                        ground: None,
                    },
                }
            })
            .collect();
        let owner = provider_id(&recipe.map.entities.provider);
        let published = SimulationOutput {
            snapshot: qa_world::session::WorldSnapshot {
                frame: FrameContext {
                    frame: self.frame_number,
                    time: SourceTime::Seconds(time as f32),
                    elapsed: SourceTime::Seconds(0.0f64.max(self.seconds - self.previous_seconds) as f32),
                    phase: FramePhase::FrameExit,
                },
                actors: bodies
                    .iter()
                    .map(|body| SnapshotActor {
                        id: body.id,
                        owner: owner.clone(),
                        definition: "q1:remote-entity".to_string(),
                    })
                    .collect(),
                bodies,
                inventories: vec![SnapshotInventory {
                    id: SavedActorId::from(&player.actor),
                    entries: self
                        .player_ui(&player.actor)
                        .inventory
                        .iter()
                        .map(snapshot_entry)
                        .collect(),
                }],
            },
            events: self.sounds_pending.clone(),
        };
        let mut light_styles: Vec<Q1RemoteLightStyle> = self
            .styles
            .iter()
            .map(|(style, pattern)| {
                let value = if pattern.is_empty() {
                    256.0
                } else {
                    let at = (time * 10.0).floor().max(0.0) as usize % pattern.len();
                    (f64::from(pattern.as_bytes()[at]) - 97.0) * 22.0
                };
                Q1RemoteLightStyle { style: *style, value }
            })
            .collect();
        light_styles.sort_by_key(|style| style.style);
        self.published_scene = Some(Q1RemoteSceneOutput {
            world_resource: recipe.map.geometry.clone(),
            world_geometry: self.content().world_geometry().to_string(),
            configurations: vec![Q1RemoteConfiguration {
                actor: player.actor.clone(),
                movement: recipe.movement.clone(),
                character: recipe.character.clone(),
                weapons: recipe.weapons.clone(),
                inventory: recipe.inventory.clone(),
            }],
            light_styles,
        });
        self.published = Some(published);
        if let Some(published) = self.published.clone() {
            self.options.host.publish(&SimulationOutput {
                snapshot: published.snapshot,
                events: Vec::new(),
            });
        }
    }

    /// Sample presentation time (donor `samplePresentation`).
    pub fn sample_presentation(&mut self, now_milliseconds: u64) -> Option<&SimulationOutput> {
        if let Some(demo) = self.demo_seconds {
            return self.sample_demo(demo);
        }
        let now = self.options.host.presentation_time().unwrap_or(now_milliseconds);
        if self.seconds - self.previous_seconds > 0.1 {
            self.previous_seconds = self.seconds - 0.1;
        }
        let duration = 0.0f64.max(self.seconds - self.previous_seconds);
        self.fraction = if duration == 0.0 {
            1.0
        } else {
            0.0f64.max(1.0f64.min((now.saturating_sub(self.received_at)) as f64 / (duration * 1000.0)))
        };
        self.publish();
        self.published.as_ref()
    }

    /// Recorded source seconds (donor `recordedSeconds`).
    #[must_use]
    pub fn recorded_seconds(&self) -> f64 {
        self.seconds
    }

    /// Set demo view angles (donor `setDemoViewAngles`).
    pub fn set_demo_view_angles(&mut self, current: Vec3, interpolate: bool) {
        let previous = if interpolate {
            self.demo_angles.map(|(_, current)| current).unwrap_or(current)
        } else {
            current
        };
        self.demo_angles = Some((previous, current));
    }

    /// Sample a recorded timestamp (donor `sampleDemo`).
    pub fn sample_demo(&mut self, seconds: f64) -> Option<&SimulationOutput> {
        self.demo_seconds = Some(seconds);
        if self.seconds - self.previous_seconds > 0.1 {
            self.previous_seconds = self.seconds - 0.1;
        }
        let duration = self.seconds - self.previous_seconds;
        self.fraction = if duration <= 0.0 {
            1.0
        } else {
            0.0f64.max(1.0f64.min((seconds - self.previous_seconds) / duration))
        };
        if let Some((previous, current)) = self.demo_angles {
            self.view_angles.set(lerp_angles(previous, current, self.fraction));
        }
        self.publish();
        self.published.as_ref()
    }

    /// Drain presentation events, emitting pending sounds first.
    pub fn drain_presentation_events(&mut self) -> Vec<Q1RemotePresentationEvent> {
        if self.published.is_none() {
            return Vec::new();
        }
        if let Some(published) = self.published.clone() {
            self.options.host.publish(&SimulationOutput {
                snapshot: published.snapshot,
                events: self.sounds_pending.clone(),
            });
        }
        self.sounds_pending.clear();
        std::mem::take(&mut self.events)
    }

    /// Handle a disconnect (donor `disconnected`).
    pub fn disconnected(&mut self, reason: &str) {
        self.options.host.disconnected(reason);
    }
}

impl<H: Q1RemoteHost + 'static> Q1ApplicationClientHost for Q1RemotePresentation<H> {
    fn receive(&mut self, messages: &[NetQuakeMessage], now_milliseconds: u64) {
        Q1RemotePresentation::receive(self, messages, now_milliseconds);
    }

    fn command(&self, command: &ActorCommand) -> Q1UserCommand {
        Q1RemotePresentation::command(self, command)
    }

    fn disconnected(&mut self, reason: &str) {
        Q1RemotePresentation::disconnected(self, reason);
    }
}

impl<H: Q1RemoteHost + 'static> RemotePresentationAccess for Q1RemotePresentation<H> {
    fn world_text(&self) -> Vec<WorldText> {
        Vec::new()
    }

    fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        Q1RemotePresentation::player_ui(self, actor)
    }

    fn character_views(&self) -> Vec<Q3CharacterView> {
        Vec::new()
    }

    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        Q1RemotePresentation::player_command(self, actor, name, args);
    }

    fn presentations(&self) -> Vec<PresentationModel> {
        Q1RemotePresentation::presentations(self)
    }

    fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference) {
        Q1RemotePresentation::register_resource(self, content, path, resource);
    }

    fn player_view(&self, actor: &ActorId) -> PlayerView {
        Q1RemotePresentation::player_view(self, actor)
    }
}

/// Demo-input surface over the canonical presentation.
///
/// Each body resolves to the inherent presentation method of the same
/// name (donor `Q1RemotePresentation` in
/// `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/remote-q1.ts`).
/// Readiness follows the donor demo input: signon plus a published player
/// and output.
impl<H: Q1RemoteHost + 'static> NetQuakeDemoRemote for Q1RemotePresentation<H> {
    fn demo_ready(&self) -> bool {
        self.player().is_some() && self.output().is_some()
    }

    fn recorded_seconds(&self) -> f64 {
        self.recorded_seconds()
    }

    fn receive(&mut self, messages: &[NetQuakeMessage], milliseconds: f64, _assert_current: &dyn Fn()) {
        self.receive(messages, milliseconds as u64);
    }

    fn set_demo_view_angles(&mut self, angles: &Vec3, interpolate: bool) {
        self.set_demo_view_angles(*angles, interpolate);
    }

    fn sample_demo(&mut self, seconds: f64) {
        let _ = self.sample_demo(seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_net::q1_wide::{NqProfile, WideEntityState};
    use qa_world::session::SessionMode;
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::persistence::recipe::fixture_recipe;

    #[derive(Debug)]
    struct TestContent {
        recipe: ExecutableRecipe,
    }

    impl Q1RemoteContent for TestContent {
        fn recipe(&self) -> &ExecutableRecipe {
            &self.recipe
        }

        fn world_geometry(&self) -> &str {
            "test-world"
        }
    }

    #[derive(Debug)]
    struct TestScene;

    #[derive(Debug, Default)]
    struct TestHostState {
        sent: Vec<String>,
        printed: Vec<String>,
        published: Vec<SimulationOutput>,
        disconnected: Vec<String>,
        generation: u32,
    }

    struct TestHost {
        state: Rc<RefCell<TestHostState>>,
    }

    impl Q1RemoteHost for TestHost {
        type Content = TestContent;
        type Scene = TestScene;

        fn load_content(&mut self, _world: &Q1RemoteWorld) -> TestContent {
            TestContent {
                recipe: fixture_recipe(),
            }
        }

        fn build_scene(_content: &TestContent) -> TestScene {
            TestScene
        }

        fn send_command(&mut self, text: &str) {
            self.state.borrow_mut().sent.push(text.to_string());
        }

        fn print(&mut self, text: &str) {
            self.state.borrow_mut().printed.push(text.to_string());
        }

        fn publish(&mut self, output: &SimulationOutput) {
            self.state.borrow_mut().published.push(output.clone());
        }

        fn disconnected(&mut self, reason: &str) {
            self.state.borrow_mut().disconnected.push(reason.to_string());
        }

        fn next_generation(&self, _slot: u32) -> u32 {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.generation
        }
    }

    fn harness() -> (Q1RemotePresentation<TestHost>, Rc<RefCell<TestHostState>>) {
        let state = Rc::new(RefCell::new(TestHostState::default()));
        let identity = IdentityOwner::create("test").unwrap();
        let client_id = identity.client(0, 1);
        let session = EngineSession::new(IdentityOwner::create("test").unwrap(), SessionMode::Local);
        let presentation = Q1RemotePresentation::new(Q1RemotePresentationOptions {
            identity,
            session,
            client: SessionClient::new(client_id),
            content: None,
            host: TestHost { state: state.clone() },
        });
        (presentation, state)
    }

    fn server_info() -> NetQuakeMessage {
        NetQuakeMessage::ServerInfo {
            protocol: NqProfile::Netquake,
            max_clients: 4,
            game_type: 0,
            level: "test".to_string(),
            models: vec!["maps/test.bsp".to_string(), "progs/player.mdl".to_string()],
            sounds: vec!["weapons/shotgun.wav".to_string()],
        }
    }

    fn entity(number: u32, origin: [f64; 3]) -> NetQuakeMessage {
        NetQuakeMessage::Entity {
            state: Q1WireEntity {
                number,
                state: WideEntityState {
                    modelindex: 2,
                    frame: 1,
                    origin,
                    ..Default::default()
                },
                step: true,
                ..Default::default()
            },
        }
    }

    fn client_data() -> NetQuakeMessage {
        NetQuakeMessage::ClientData {
            data: NetQuakeClientData {
                view_height: 22,
                health: 100,
                items: 1,
                active_weapon: 1,
                shells: 25,
                ammo: 25,
                ..Default::default()
            },
            weapon_alpha: 0,
        }
    }

    fn admit(presentation: &mut Q1RemotePresentation<TestHost>) {
        presentation.receive(
            &[
                server_info(),
                NetQuakeMessage::SetView { entity: 1 },
                NetQuakeMessage::Time { seconds: 1.0 },
                entity(1, [0.0, 0.0, 0.0]),
                client_data(),
            ],
            1000,
        );
    }

    #[test]
    fn server_info_resets_presentation_state() {
        let (mut presentation, _) = harness();
        presentation.receive(&[server_info()], 0);
        assert_eq!(presentation.max_clients, 4);
        assert!(presentation.player().is_none());
        assert!(presentation.output().is_none());
    }

    #[test]
    fn frame_publishes_snapshot_with_scene_blocks() {
        let (mut presentation, state) = harness();
        admit(&mut presentation);
        let output = presentation.output().expect("frame publishes");
        assert_eq!(output.snapshot.frame.frame, 1);
        assert_eq!(output.snapshot.bodies.len(), 1);
        assert_eq!(output.snapshot.actors.len(), 1);
        assert_eq!(output.snapshot.inventories.len(), 1);
        assert_eq!(output.snapshot.inventories[0].entries.len(), 4);
        let scene = presentation.scene_output().expect("scene blocks publish");
        assert_eq!(scene.world_geometry, "test-world");
        assert_eq!(scene.configurations.len(), 1);
        assert_eq!(scene.light_styles.len(), 0);
        // The live publish carries no events; sounds flush on drain.
        assert_eq!(state.borrow().published.len(), 1);
        assert!(state.borrow().published[0].events.is_empty());
    }

    #[test]
    fn entity_motion_interpolates_between_frames() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        let sliding = |origin: [f64; 3]| NetQuakeMessage::Entity {
            state: Q1WireEntity {
                number: 2,
                state: WideEntityState {
                    modelindex: 2,
                    origin,
                    ..Default::default()
                },
                ..Default::default()
            },
        };
        presentation.receive(
            &[
                NetQuakeMessage::Time { seconds: 1.1 },
                sliding([10.0, 0.0, 0.0]),
                entity(1, [0.0, 0.0, 0.0]),
            ],
            1100,
        );
        presentation.receive(
            &[
                NetQuakeMessage::Time { seconds: 1.2 },
                sliding([20.0, 0.0, 0.0]),
                entity(1, [0.0, 0.0, 0.0]),
            ],
            1200,
        );
        presentation.sample_presentation(1250);
        let models = presentation.presentations();
        let moved = models
            .iter()
            .find(|model| model.origin.x > 10.0 && !model.view_weapon)
            .expect("moved entity presents");
        assert!(moved.origin.x < 20.0, "origin interpolates: {}", moved.origin.x);
    }

    #[test]
    fn teleports_skip_interpolation() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[
                NetQuakeMessage::Time { seconds: 2.0 },
                NetQuakeMessage::Entity {
                    state: Q1WireEntity {
                        number: 2,
                        state: WideEntityState {
                            modelindex: 2,
                            origin: [0.0, 0.0, 0.0],
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                },
                entity(1, [0.0, 0.0, 0.0]),
            ],
            2000,
        );
        presentation.receive(
            &[
                NetQuakeMessage::Time { seconds: 2.1 },
                NetQuakeMessage::Entity {
                    state: Q1WireEntity {
                        number: 2,
                        state: WideEntityState {
                            modelindex: 2,
                            origin: [500.0, 0.0, 0.0],
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                },
                entity(1, [0.0, 0.0, 0.0]),
            ],
            2100,
        );
        presentation.sample_presentation(2050);
        let models = presentation.presentations();
        let moved = models
            .iter()
            .find(|model| model.origin.x >= 500.0 && !model.view_weapon)
            .expect("teleported entity snaps");
        assert_eq!(moved.origin.x, 500.0);
    }

    #[test]
    fn player_hud_reports_weapon_and_ammo() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        let actor = presentation.player().expect("player").actor;
        let ui = presentation.player_ui(&actor);
        assert_eq!(ui.active_weapon.as_deref(), Some("q1:weapon/shotgun"));
        assert_eq!(ui.ammo.as_ref().map(|ammo| ammo.count), Some(25.0));
        assert_eq!(ui.items.len(), 8);
        assert!(ui.items[1].owned);
        assert_eq!(ui.items[1].source_ordinal, 2.0);
        let status = ui.weapon_status.expect("weapon status");
        assert_eq!(status.item, "q1:weapon/shotgun");
    }

    #[test]
    fn scoreboard_metadata_updates_rows_and_events() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[
                NetQuakeMessage::NamedSlot {
                    kind: NqNamedSlot::Name,
                    slot: 0,
                    value: "player".to_string(),
                },
                NetQuakeMessage::NumberedSlot {
                    kind: NqNumberedSlot::Frags,
                    slot: 0,
                    value: 3,
                },
            ],
            2000,
        );
        let row = presentation.scoreboard().get(&0).expect("row");
        assert_eq!(row.name, "player");
        assert_eq!(row.frags, 3);
        let events = presentation.drain_presentation_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0].event,
            Q1RemoteEvent::ClientMetadata(Q1ClientMetadataEvent::Name { .. })
        ));
    }

    #[test]
    fn set_angle_emits_view_reset() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[NetQuakeMessage::SetAngle {
                angles: [0.0, 90.0, 0.0],
            }],
            2000,
        );
        let events = presentation.drain_presentation_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, Q1RemoteEvent::ViewReset { .. }));
    }

    #[test]
    fn stufftext_bonus_flash_and_reconnect() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[NetQuakeMessage::Text {
                kind: NqText::Stufftext,
                text: "bf\n".to_string(),
            }],
            2000,
        );
        let events = presentation.drain_presentation_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, Q1RemoteEvent::Q1(Q1Event::Effect { .. })));
        presentation.receive(
            &[NetQuakeMessage::Text {
                kind: NqText::Stufftext,
                text: "reconnect\n".to_string(),
            }],
            2000,
        );
        // The donor clears `published` on reconnect, then the same receive's
        // trailing publish rebuilds it from the retained frame state.
        assert!(presentation.output().is_some());
    }

    #[test]
    fn beam_effect_maps_style() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[NetQuakeMessage::TemporaryEntity {
                effect: TemporaryEntity::Beam {
                    effect_type: 6,
                    entity: 1,
                    start: [0.0, 0.0, 0.0],
                    end: [1.0, 1.0, 1.0],
                },
            }],
            2000,
        );
        let events = presentation.drain_presentation_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].event,
            Q1RemoteEvent::Q1(Q1Event::Beam {
                style: Q1BeamStyle::Lightning2,
                ..
            })
        ));
    }

    #[test]
    fn sound_flushes_on_drain_after_registration() {
        use qa_content::contract::{
            ContentId as ContractContentId, LooseMount, MountId, MountIdentity, MountPlanId, ResourceId,
            ResourceIdentity, ResourceProvenance, ResourceResolution,
        };
        let (mut presentation, state) = harness();
        admit(&mut presentation);
        let actor = presentation.player().expect("player").actor;
        presentation.register_resource(
            &presentation.map_content(),
            "sound/weapons/shotgun.wav",
            &ResolvedResourceReference {
                id: ResourceId("resource:test".to_string()),
                requested_path: "sound/weapons/shotgun.wav".to_string(),
                provenance: ResourceProvenance::Loose {
                    mount: LooseMount {
                        identity: MountIdentity {
                            id: MountId("mount:test".to_string()),
                            content: ContractContentId("test".to_string()),
                            generation: 0,
                        },
                        root_path: String::new(),
                    },
                    member_path: "weapons/shotgun.wav".to_string(),
                },
                identity: ResourceIdentity::parse("identity:0:0:8:0").unwrap(),
                byte_length: 8,
                resolution: ResourceResolution::DefaultOrder {
                    plan: MountPlanId("plan".to_string()),
                    rank: 0,
                },
            },
        );
        presentation.receive(
            &[NetQuakeMessage::Sound {
                entity: 1,
                channel: 1,
                index: 1,
                volume: 255,
                attenuation: 1.0,
                origin: [0.0, 0.0, 0.0],
            }],
            2000,
        );
        assert!(state.borrow().published.iter().all(|output| output
            .events
            .iter()
            .all(|event| !matches!(event.payload, SimEventPayload::Sound { .. }))));
        let events = presentation.drain_presentation_events();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event, Q1RemoteEvent::Q1(Q1Event::Sound { .. })));
        let flushed = state.borrow_mut().published.pop();
        assert!(flushed
            .expect("drain publishes")
            .events
            .iter()
            .any(|event| matches!(event.payload, SimEventPayload::Sound { .. })));
        assert_eq!(presentation.player_slot(&actor), Some(0));
        assert!(presentation.is_player(&actor));
    }

    #[test]
    fn command_selects_owned_weapon_impulse() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        let actor = presentation.player().expect("player").actor;
        let ui = presentation.player_ui(&actor);
        assert_eq!(ui.active_weapon.as_deref(), Some("q1:weapon/shotgun"));
        let wire = presentation.command(&ActorCommand {
            actor: actor.clone(),
            source: qa_net::common::commands::CommandSource::LocalSeat {
                seat: IdentityOwner::create("seat").unwrap().seat(0),
            },
            sequence: 1,
            command: UserCommand::Q1Netquake {
                acknowledged_server_time_seconds: 0.0,
                view_angles: [0.0, 45.0, 0.0],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
            },
            arsenal: None,
        });
        assert_eq!(wire.impulse, 0);
        assert_eq!(presentation.view_angles.get().y, 45.0);
    }

    #[test]
    fn light_styles_animate_with_time() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[NetQuakeMessage::LightStyle {
                index: 0,
                value: "ab".to_string(),
            }],
            2000,
        );
        presentation.sample_presentation(2000);
        let scene = presentation.scene_output().expect("scene");
        assert_eq!(scene.light_styles.len(), 1);
        assert_eq!(scene.light_styles[0].style, 0);
    }

    #[test]
    #[should_panic(expected = "NetQuake server has no world model")]
    fn server_info_without_world_model_panics() {
        let (mut presentation, _) = harness();
        presentation.receive(
            &[NetQuakeMessage::ServerInfo {
                protocol: NqProfile::Netquake,
                max_clients: 4,
                game_type: 0,
                level: "test".to_string(),
                models: vec!["progs/player.mdl".to_string()],
                sounds: Vec::new(),
            }],
            0,
        );
    }

    #[test]
    #[should_panic(expected = "Unresolved NetQuake sound")]
    fn unregistered_sound_panics() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[NetQuakeMessage::Sound {
                entity: 1,
                channel: 1,
                index: 1,
                volume: 255,
                attenuation: 1.0,
                origin: [0.0, 0.0, 0.0],
            }],
            2000,
        );
    }
}
