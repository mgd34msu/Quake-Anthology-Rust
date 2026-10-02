//! QuakeWorld remote presentation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/remote-qw.ts`
//! (`QwRemotePresentation`). Decoded QuakeWorld protocol state is
//! translated into NetQuake messages and shared with a
//! [`Q1RemotePresentation`]; the QuakeWorld layer adds player-entity mapping,
//! nail projectiles, userinfo scoreboard rows, spectator camera control, skin
//! selection, client prediction, and brush linking.
//!
//! The donor is async (`Promise` with `assertCurrent` retirement guards);
//! this sync port resolves every host call inline in program order. The
//! donor passes `this` into its camera closures; the `'static` camera
//! closures here capture shared handles instead: the wrapped Q1
//! presentation lives behind `Rc<RefCell<..>>` so the trace closure can read
//! the current scene and player, and the host lives behind a second shared
//! handle so `send_command`/`print` stay reachable. Camera entry points are
//! only invoked while no shared borrow is held, so the cells never reenter.
//! Render-facing borrows that the donor returns by reference (`scene`,
//! `client`, `output`) are exposed through closures or owned clones for the
//! same reason.
//!
//! Session-lane pieces the donor imports stay behind host traits:
//! [`QwRemoteScene`] for camera traces and brush links, [`QwPredictor`] for
//! the QuakeWorld movement predictor, and [`QwRemoteHost`] for content,
//! checksums, clocks, and downloads.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_client::render::scene::models::types::IndexedModelSkin;
use qa_content::contract::{ContentId, InventoryEntry, ResolvedResourceReference};
use qa_content::q3::foundation::presentation::Q3CharacterView;
use qa_core::identity::{ActorId, IdentityOwner, OwnedActor, ProviderId, SavedActorId, SeatId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::native_atoi;
use qa_net::common::commands::{ActorCommand, UserCommand};
use qa_net::q1_net::{
    NetQuakeClientData, NetQuakeMessage, NqNamedSlot, NqNumberedSlot, NqText, NqUnit, Q1WireEntity, QuakeWorldMessage,
    QwMoveVariables, QwPlayerState, QwSlotStat, QwSlotValue,
};
use qa_net::q1_wide::{NqProfile, WideEntityState};
use qa_net::qw::QwUsercmd;
use qa_world::body::BodyState;
use qa_world::session::{EngineSession, SessionClient, SimulationOutput};

use super::qw_camera::{QwCameraFrame, QwCameraOptions, QwCameraPlayer, QwCameraTrace, QwSpectatorCamera};
use super::qw_skins::{skin_name, QwPlayerSkins, QwSkinOptions};
use super::qw_types::{
    QwApplicationClientHost, QwApplicationDownloads, QwApplicationPrediction, QwApplicationSkins, QwServerData,
};
use super::remote_q1::{
    Q1ClientRow, Q1RemoteContent, Q1RemoteHost, Q1RemotePresentation, Q1RemotePresentationEvent,
    Q1RemotePresentationOptions, Q1RemoteWorld,
};
use super::types::{
    ApplicationNetworkPlayer, PlayerPitchDrift, PlayerUi, PlayerView, PresentationIndexedSkin, PresentationModel,
    WorldText,
};

/// Stored intermission state (donor `intermission` message).
#[derive(Debug, Clone, PartialEq)]
struct QwIntermission {
    /// Intermission origin.
    origin: [f64; 3],
    /// Intermission angles.
    angles: [f64; 3],
}

/// Scene queries the QuakeWorld layer needs (donor `scene` uses).
pub trait QwRemoteScene {
    /// Trace a spectator-camera segment, ignoring `pass_actor`.
    fn trace_spectator(&self, start: Vec3, end: Vec3, pass_actor: Option<&ActorId>) -> QwCameraTrace;
    /// Unlink a previously linked actor.
    fn unlink_actor(&mut self, actor: &ActorId);
    /// Collision bounds of a brush model.
    fn model_bounds(&self, model: u32) -> Bounds;
    /// Link a solid body.
    fn link_solid(&mut self, link: QwSolidLink);
}

/// One linked solid (donor `scene.link` call in `linkSolids`).
#[derive(Debug, Clone, PartialEq)]
pub struct QwSolidLink {
    /// Linked actor.
    pub actor: ActorId,
    /// Linked body state.
    pub state: BodyState,
    /// Absolute bounds (local bounds expanded by one unit per side).
    pub absolute_bounds: Bounds,
    /// Brush model number, when the solid is a brush.
    pub model: Option<u32>,
    /// Whether the solid follows a player.
    pub monster: bool,
}

/// Client-prediction snapshot (donor `MovementPredictionSnapshot` base).
#[derive(Debug, Clone, PartialEq)]
pub struct QwPredictionBase {
    /// Acknowledged command sequence.
    pub sequence: u32,
    /// Command time in milliseconds.
    pub command_time_ms: f64,
    /// Player origin.
    pub origin: [f64; 3],
    /// Player velocity.
    pub velocity: [i16; 3],
    /// View angles.
    pub angles: Vec3,
    /// Health.
    pub health: f64,
    /// Spectator flag (1 when spectating).
    pub spectator: u8,
    /// Weapon frame.
    pub weapon_frame: u8,
    /// Animation frame.
    pub frame: u8,
    /// Active weapon item id.
    pub active_weapon: Option<String>,
    /// Inventory entries.
    pub inventory: Vec<InventoryEntry>,
    /// Server weapon ordinal.
    pub source_weapon: i32,
}

/// Predicted player state (donor `MovementPredictionResult` projection).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QwPredictedPlayer {
    /// Predicted origin.
    pub origin: Vec3,
    /// Whether the prediction is grounded.
    pub grounded: bool,
}

/// Predictor construction seed (donor `QuakeWorldPrediction` options).
#[derive(Debug, Clone, PartialEq)]
pub struct QwPredictorSeed {
    /// Owned prediction actor.
    pub actor: OwnedActor,
    /// Local seat.
    pub seat: SeatId,
    /// Inventory provider.
    pub inventory_provider: String,
    /// Character provider.
    pub character_provider: String,
    /// Entity provider.
    pub entities_provider: String,
    /// Initial snapshot.
    pub base: QwPredictionBase,
    /// Movement variables.
    pub variables: QwMoveVariables,
}

/// QuakeWorld movement predictor (donor `QuakeWorldPrediction`).
pub trait QwPredictor {
    /// Receive a server snapshot.
    fn receive(&mut self, base: &QwPredictionBase, own: &QwPlayerState, variables: &QwMoveVariables);
    /// Record a sent command.
    fn sent(&mut self, sequence: u32, command: &QwUsercmd, now_ms: f64);
    /// Record an acknowledged command.
    fn acknowledged(&mut self, sequence: u32, now_ms: f64);
    /// Replay pending moves; `None` means history is exhausted.
    fn replay(&mut self) -> Option<QwPredictedPlayer>;
}

/// Host callbacks behind [`QwRemotePresentationOptions`].
pub trait QwRemoteHost: Q1RemoteHost {
    /// Movement predictor.
    type Predictor: QwPredictor;
    /// Download sink.
    type Downloads: QwApplicationDownloads;
    /// Prepare server data (donor `prepareServerData`).
    fn prepare_server_data(&mut self, data: &QwServerData);
    /// Map checksum for an admitted world (donor `mapChecksum`).
    fn map_checksum(&mut self, world: &Q1RemoteWorld, game_directory: &str) -> i32;
    /// Whether a content path resolves (donor `mounts.resolve`).
    fn sound_available(content: &Self::Content, path: &str) -> bool;
    /// Current time in seconds (donor `performance.now() / 1000`).
    fn now_seconds(&self) -> f64;
    /// Build the movement predictor.
    fn build_predictor(&mut self, seed: &QwPredictorSeed) -> Self::Predictor;
}

/// Options for [`QwRemotePresentation`] (donor `QwRemotePresentationOptions`).
pub struct QwRemotePresentationOptions<H: QwRemoteHost> {
    /// Identity authority.
    pub identity: IdentityOwner,
    /// Engine session.
    pub session: EngineSession,
    /// Bound client.
    pub client: SessionClient,
    /// Local seat.
    pub seat: SeatId,
    /// Preloaded content, if any.
    pub content: Option<H::Content>,
    /// Skin selection options.
    pub skin_options: QwSkinOptions,
    /// Spectator camera options.
    pub camera_options: Option<QwCameraOptions>,
    /// Download sink, when the session lane binds one.
    pub downloads: Option<H::Downloads>,
    /// Host callbacks.
    pub host: H,
}

/// Shared-host bridge so the wrapped Q1 presentation and the camera closures
/// reach the same host through one shared handle.
#[derive(Clone)]
struct QwSharedHost<H: QwRemoteHost> {
    /// Shared host.
    host: Rc<RefCell<H>>,
}

impl<H: QwRemoteHost> Q1RemoteHost for QwSharedHost<H> {
    type Content = H::Content;
    type Scene = H::Scene;

    fn load_content(&mut self, world: &Q1RemoteWorld) -> Self::Content {
        self.host.borrow_mut().load_content(world)
    }

    fn build_scene(content: &Self::Content) -> Self::Scene {
        H::build_scene(content)
    }

    fn send_command(&mut self, text: &str) {
        self.host.borrow_mut().send_command(text);
    }

    fn print(&mut self, text: &str) {
        self.host.borrow_mut().print(text);
    }

    fn publish(&mut self, output: &SimulationOutput) {
        self.host.borrow_mut().publish(output);
    }

    fn disconnected(&mut self, reason: &str) {
        self.host.borrow_mut().disconnected(reason);
    }

    fn next_generation(&self, slot: u32) -> u32 {
        self.host.borrow().next_generation(slot)
    }

    fn presentation_time(&self) -> Option<u64> {
        self.host.borrow().presentation_time()
    }
}

/// QuakeWorld remote presentation (donor `QwRemotePresentation`).
pub struct QwRemotePresentation<H: QwRemoteHost> {
    /// Shared host handle.
    host: Rc<RefCell<H>>,
    /// Shared NetQuake presentation.
    shared: Rc<RefCell<Q1RemotePresentation<QwSharedHost<H>>>>,
    /// Download sink.
    downloads: Option<H::Downloads>,
    /// Local seat.
    seat: SeatId,
    /// Admitted server data.
    data: Option<QwServerData>,
    /// Music track received before admission.
    pending_music_track: Option<u8>,
    /// Movement predictor.
    predictor: Option<H::Predictor>,
    /// Latest prediction.
    predicted: Option<QwPredictedPlayer>,
    /// Acknowledged command sequence.
    acknowledged_sequence: u32,
    /// Player stats by index.
    stats: HashMap<u8, i32>,
    /// Userinfo maps by slot.
    userinfos: HashMap<u8, HashMap<String, String>>,
    /// Selected skins by slot.
    selected_skins: HashMap<u8, IndexedModelSkin>,
    /// Skin selector.
    player_skins: RefCell<QwPlayerSkins>,
    /// Whether skins are loading.
    skin_loading: bool,
    /// Last prepared skin signature.
    skin_signature: String,
    /// Last prepared policy signature.
    skin_policy_signature: String,
    /// Userinfo revision counter.
    skin_revision: u64,
    /// Own player state.
    own_player: Option<QwPlayerState>,
    /// Player states by slot for the camera.
    camera_players: HashMap<i32, QwPlayerState>,
    /// Spectator camera.
    camera: QwSpectatorCamera,
    /// Last received source records.
    records: Vec<QuakeWorldMessage>,
    /// Last packet entities.
    entities: Vec<Q1WireEntity>,
    /// Precached model names.
    model_names: Vec<String>,
    /// Precached sound count.
    sound_count: usize,
    /// Resolved sound indexes (1-based).
    available_sounds: HashSet<u16>,
    /// Linked solid actors.
    linked: Vec<ActorId>,
    /// Pending view kick in degrees.
    kick: i8,
    /// Intermission state.
    intermission: Option<QwIntermission>,
    /// Movement variables.
    variables: Option<QwMoveVariables>,
}

fn provider_id(provider: &str) -> ProviderId {
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    ProviderId::new(namespace, name)
}

fn vec3(values: [f64; 3]) -> Vec3 {
    Vec3 {
        x: values[0] as f32,
        y: values[1] as f32,
        z: values[2] as f32,
    }
}

fn arr(value: Vec3) -> [f64; 3] {
    [f64::from(value.x), f64::from(value.y), f64::from(value.z)]
}

/// Map a QuakeWorld player state onto a NetQuake entity (donor `playerEntity`).
fn player_entity(state: &QwPlayerState) -> Q1WireEntity {
    Q1WireEntity {
        number: u32::from(state.number) + 1,
        state: WideEntityState {
            origin: state.origin,
            angles: [-state.command.angles[0] / 3.0, state.command.angles[1], 0.0],
            modelindex: state.model_index,
            frame: u16::from(state.frame),
            colormap: state.number.saturating_add(1),
            skin: state.skin,
            effects: state.effects,
            alpha: 0,
            scale: 16,
        },
        lerp_finish_seconds: 0.0,
        step: true,
        quakeworld_flags: 0,
    }
}

impl<H: QwRemoteHost + 'static> QwRemotePresentation<H> {
    /// Create a presentation (donor constructor).
    pub fn new(options: QwRemotePresentationOptions<H>) -> Self
    where
        H::Scene: QwRemoteScene,
    {
        let host = Rc::new(RefCell::new(options.host));
        let shared = Q1RemotePresentation::new(Q1RemotePresentationOptions {
            identity: options.identity,
            session: options.session,
            client: options.client,
            content: options.content,
            host: QwSharedHost { host: Rc::clone(&host) },
        });
        let shared = Rc::new(RefCell::new(shared));
        let camera_options = options.camera_options.unwrap_or_else(|| QwCameraOptions {
            hightrack: Box::new(|| 0),
            chasecam: Box::new(|| 0),
        });
        let send_host = Rc::clone(&host);
        let trace_shared = Rc::clone(&shared);
        let print_host = Rc::clone(&host);
        let camera = QwSpectatorCamera::new(
            camera_options,
            move |text| send_host.borrow_mut().send_command(text),
            move |start, end| {
                let mut presentation = trace_shared.borrow_mut();
                let pass = presentation.player().map(|player| player.actor);
                presentation
                    .scene()
                    .expect("QW camera needs scene queries")
                    .trace_spectator(start, end, pass.as_ref())
            },
            move |text| print_host.borrow_mut().print(text),
        );
        Self {
            host,
            shared,
            downloads: options.downloads,
            seat: options.seat,
            data: None,
            pending_music_track: None,
            predictor: None,
            predicted: None,
            acknowledged_sequence: 0,
            stats: HashMap::new(),
            userinfos: HashMap::new(),
            selected_skins: HashMap::new(),
            player_skins: RefCell::new(QwPlayerSkins::new(options.skin_options)),
            skin_loading: false,
            skin_signature: String::new(),
            skin_policy_signature: String::new(),
            skin_revision: 0,
            own_player: None,
            camera_players: HashMap::new(),
            camera,
            records: Vec::new(),
            entities: Vec::new(),
            model_names: Vec::new(),
            sound_count: 0,
            available_sounds: HashSet::new(),
            linked: Vec::new(),
            kick: 0,
            intermission: None,
            variables: None,
        }
    }
}

impl<H: QwRemoteHost + 'static> QwRemotePresentation<H> {
    /// Movement variables, when admitted (donor `moveVariables`).
    #[must_use]
    pub fn move_variables(&self) -> Option<QwMoveVariables> {
        self.variables.clone()
    }

    /// Run a query against the bound client (donor `client`).
    pub fn with_client<R>(&self, query: impl FnOnce(&SessionClient) -> R) -> R {
        query(self.shared.borrow().client())
    }

    /// Current player, if any (donor `player`).
    #[must_use]
    pub fn player(&self) -> Option<ApplicationNetworkPlayer> {
        self.shared.borrow().player()
    }

    /// Published output, if any (donor `output`).
    #[must_use]
    pub fn output(&self) -> Option<SimulationOutput> {
        self.shared.borrow().output().cloned()
    }

    /// Run a query against the scene (donor `scene`).
    pub fn with_scene<R>(&mut self, query: impl FnOnce(&H::Scene) -> R) -> R {
        let mut shared = self.shared.borrow_mut();
        let scene = shared.scene().expect("QW scene is loaded");
        query(scene)
    }

    /// Last received source records (donor `sourceRecords`).
    #[must_use]
    pub fn source_records(&self) -> Vec<QuakeWorldMessage> {
        self.records.clone()
    }

    /// Scoreboard rows by slot (donor `scoreboard`).
    #[must_use]
    pub fn scoreboard(&self) -> HashMap<u32, Q1ClientRow> {
        self.shared.borrow().scoreboard().clone()
    }

    /// Take a pending spectator teleport (donor `takeSpectatorTeleport`).
    pub fn take_spectator_teleport(&mut self) -> Option<Vec3> {
        self.camera.take_teleport()
    }

    /// Receive server data (donor `serverData`).
    pub fn server_data(&mut self, data: &QwServerData) {
        self.data = None;
        self.pending_music_track = None;
        self.host.borrow_mut().prepare_server_data(data);
    }

    /// Receive game state; returns the map checksum (donor `gameState`).
    pub fn game_state(&mut self, data: &QwServerData, models: &[String], sounds: &[String]) -> i32 {
        self.camera.reset();
        self.camera_players.clear();
        self.player_skins.borrow_mut().clear();
        self.userinfos.clear();
        self.selected_skins.clear();
        self.skin_signature.clear();
        self.skin_loading = false;
        self.skin_revision += 1;
        self.predictor = None;
        self.predicted = None;
        self.model_names = models.to_vec();
        self.linked.clear();
        self.data = Some(data.clone());
        self.variables = Some(data.move_variables.clone());
        self.stats.clear();
        self.own_player = None;
        self.entities.clear();
        self.kick = 0;
        self.intermission = None;
        let map = models.first().expect("QW has no world model").clone();
        self.shared.borrow_mut().receive(
            &[
                NetQuakeMessage::ServerInfo {
                    protocol: NqProfile::Netquake,
                    max_clients: 32,
                    game_type: 1,
                    level: data.level.clone(),
                    models: models.to_vec(),
                    sounds: sounds.to_vec(),
                },
                NetQuakeMessage::SetView {
                    entity: u16::from(data.player_slot) + 1,
                },
            ],
            0,
        );
        if let Some(track) = self.pending_music_track.take() {
            self.shared.borrow_mut().receive(
                &[NetQuakeMessage::CdTrack {
                    track,
                    loop_track: track,
                }],
                0,
            );
        }
        self.sound_count = sounds.len();
        self.available_sounds.clear();
        {
            let shared = self.shared.borrow();
            let content = shared.content();
            for (index, sound) in sounds.iter().enumerate() {
                if H::sound_available(content, &format!("sound/{sound}")) {
                    self.available_sounds.insert(index as u16 + 1);
                }
            }
        }
        self.host.borrow_mut().map_checksum(
            &Q1RemoteWorld {
                map,
                models: models.to_vec(),
                sounds: sounds.to_vec(),
            },
            &data.game_directory,
        )
    }

    /// Record userinfo as scoreboard rows (donor `scoreboardInfo`).
    fn scoreboard_info(&mut self, slot: u8, info: HashMap<String, String>, translated: &mut Vec<NetQuakeMessage>) {
        if slot >= 32 {
            panic!("Invalid QW userinfo slot");
        }
        let color = |info: &HashMap<String, String>, key: &str| {
            let value = native_atoi(info.get(key).map_or("0", String::as_str)).unwrap_or(0);
            if !(0..=13).contains(&value) {
                13
            } else {
                value
            }
        };
        let name: String = info
            .get("name")
            .map_or(String::new(), |name| name.chars().take(15).collect());
        let packed = color(&info, "topcolor") * 16 + color(&info, "bottomcolor");
        self.userinfos.insert(slot, info);
        self.skin_revision += 1;
        translated.push(NetQuakeMessage::NamedSlot {
            kind: NqNamedSlot::Name,
            slot,
            value: name,
        });
        translated.push(NetQuakeMessage::NumberedSlot {
            kind: NqNumberedSlot::Colors,
            slot,
            value: packed as i16,
        });
    }

    /// Skin resource names (donor `skins.names`).
    pub fn skin_names(&self) -> Vec<String> {
        let (noskins, baseskin, allskins) = self.player_skins.borrow().policy();
        if noskins != 0 {
            return Vec::new();
        }
        let mut names = HashSet::new();
        for info in self.userinfos.values() {
            if info.get("name").is_none_or(String::is_empty) {
                continue;
            }
            let resolved = if allskins.is_empty() {
                let skin = info.get("skin").map_or("", String::as_str);
                if skin.is_empty() {
                    skin_name(&baseskin)
                } else {
                    skin_name(skin)
                }
            } else {
                skin_name(&allskins)
            };
            names.insert(format!("skins/{resolved}.pcx"));
        }
        let mut names: Vec<String> = names.into_iter().collect();
        names.sort();
        names
    }

    /// Mark skins loading or idle (donor `skins.loading`).
    pub fn set_skin_loading(&mut self, value: bool) {
        self.skin_loading = value;
        self.selected_skins.clear();
        if !value {
            self.player_skins.borrow_mut().clear();
            self.skin_signature.clear();
        }
    }

    /// Select skins for all known players (donor `prepareSkins`).
    pub fn prepare_skins(&mut self) {
        if self.skin_loading {
            return;
        }
        let (noskins, baseskin, allskins) = self.player_skins.borrow().policy();
        let policy_signature = format!("{noskins}\0{baseskin}\0{allskins}");
        let signature = format!("{policy_signature}\0{}", self.skin_revision);
        if signature == self.skin_signature {
            return;
        }
        if policy_signature != self.skin_policy_signature {
            self.player_skins.borrow_mut().clear();
            self.skin_policy_signature = policy_signature;
        }
        self.selected_skins.clear();
        let slots: Vec<(u8, String)> = self
            .userinfos
            .iter()
            .filter(|(_, info)| !info.get("name").is_none_or(String::is_empty))
            .map(|(slot, info)| (*slot, info.get("skin").cloned().unwrap_or_default()))
            .collect();
        for (slot, skin) in slots {
            if let Some(selected) = self.player_skins.borrow_mut().select(&skin) {
                self.selected_skins.insert(slot, selected);
            }
        }
        self.skin_signature = signature;
    }

    /// Stat value by index (donor `stat`).
    fn stat(&self, index: u8) -> i32 {
        self.stats.get(&index).copied().unwrap_or(0)
    }
}

impl<H: QwRemoteHost + 'static> QwRemotePresentation<H> {
    /// Receive QuakeWorld messages (donor `receive`).
    pub fn receive(&mut self, messages: &[QuakeWorldMessage], now_ms: f64)
    where
        H::Scene: QwRemoteScene,
    {
        self.records = messages.to_vec();
        let Some(data) = self.data.clone() else {
            for message in messages {
                if let QuakeWorldMessage::CdTrack { track } = message {
                    self.pending_music_track = Some(*track);
                }
            }
            return;
        };
        let mut translated = Vec::new();
        let mut players = Vec::new();
        let mut nails = Vec::new();
        let mut frame = false;
        for message in messages {
            match message {
                QuakeWorldMessage::Player { state } => {
                    players.push(state.clone());
                    self.camera_players.insert(i32::from(state.number), state.clone());
                    if state.number == data.player_slot {
                        self.own_player = Some(state.clone());
                    }
                }
                QuakeWorldMessage::PacketEntities { entities, .. } => {
                    self.entities = entities.clone();
                    frame = true;
                }
                QuakeWorldMessage::InvalidDelta { .. } => {}
                QuakeWorldMessage::Nails { projectiles } => {
                    let model_index = self
                        .model_names
                        .iter()
                        .position(|name| name == "progs/spike.mdl")
                        .map_or(0, |index| index + 1);
                    if model_index != 0 {
                        for (index, nail) in projectiles.iter().enumerate() {
                            nails.push(Q1WireEntity {
                                number: 131_072 + index as u32,
                                state: WideEntityState {
                                    origin: [
                                        f64::from(nail.origin[0]),
                                        f64::from(nail.origin[1]),
                                        f64::from(nail.origin[2]),
                                    ],
                                    angles: [f64::from(nail.pitch), f64::from(nail.yaw), 0.0],
                                    modelindex: model_index as u16,
                                    frame: 0,
                                    colormap: 0,
                                    skin: 0,
                                    effects: 0,
                                    alpha: 0,
                                    scale: 16,
                                },
                                lerp_finish_seconds: 0.0,
                                step: true,
                                quakeworld_flags: 0,
                            });
                        }
                    }
                }
                QuakeWorldMessage::Stat { index, value } => {
                    self.stats.insert(*index, *value);
                }
                QuakeWorldMessage::Kick { degrees } => {
                    self.kick = *degrees;
                }
                QuakeWorldMessage::Speed { entity_gravity, value } => {
                    if let Some(variables) = self.variables.as_mut() {
                        if *entity_gravity {
                            variables.entity_gravity = *value;
                        } else {
                            variables.max_speed = *value;
                        }
                    }
                }
                QuakeWorldMessage::Userinfo { slot, value, .. } => {
                    let info = qa_net::q1_net::quake_world_info(value).into_iter().collect();
                    self.scoreboard_info(*slot, info, &mut translated);
                }
                QuakeWorldMessage::SetInfo { slot, key, value } => {
                    let mut info = self.userinfos.get(slot).cloned().unwrap_or_default();
                    info.insert(key.clone(), value.clone());
                    self.scoreboard_info(*slot, info, &mut translated);
                }
                QuakeWorldMessage::Print { text, .. } => {
                    translated.push(NetQuakeMessage::Text {
                        kind: NqText::Print,
                        text: text.clone(),
                    });
                }
                QuakeWorldMessage::Intermission { origin, angles } => {
                    self.intermission = Some(QwIntermission {
                        origin: *origin,
                        angles: *angles,
                    });
                    translated.push(NetQuakeMessage::SetAngle { angles: *angles });
                    translated.push(NetQuakeMessage::Unit(NqUnit::Intermission));
                }
                QuakeWorldMessage::CdTrack { track } => {
                    translated.push(NetQuakeMessage::CdTrack {
                        track: *track,
                        loop_track: *track,
                    });
                }
                QuakeWorldMessage::Sound {
                    entity,
                    channel,
                    index,
                    origin,
                    volume,
                    attenuation,
                } => {
                    if *index < 1 || usize::from(*index) > self.sound_count {
                        panic!("Invalid QW sound index");
                    }
                    if self.available_sounds.contains(index) {
                        translated.push(NetQuakeMessage::Sound {
                            entity: *entity,
                            channel: *channel,
                            index: *index,
                            volume: *volume,
                            attenuation: *attenuation,
                            origin: *origin,
                        });
                    }
                }
                QuakeWorldMessage::StaticSound {
                    index,
                    origin,
                    volume,
                    attenuation,
                } => {
                    if *index < 1 || usize::from(*index) > self.sound_count {
                        panic!("Invalid QW sound index");
                    }
                    if self.available_sounds.contains(index) {
                        translated.push(NetQuakeMessage::StaticSound {
                            index: *index,
                            volume: *volume,
                            attenuation: *attenuation,
                            origin: *origin,
                        });
                    }
                }
                QuakeWorldMessage::ViewEntity { muzzle_flash, entity } => {
                    if !muzzle_flash {
                        translated.push(NetQuakeMessage::SetView { entity: *entity });
                    }
                }
                QuakeWorldMessage::SlotStat { kind, slot, value } => {
                    if *kind == QwSlotStat::Frags {
                        let number = match value {
                            QwSlotValue::Integer(number) => *number,
                            QwSlotValue::Float(number) => *number as i32,
                        };
                        translated.push(NetQuakeMessage::NumberedSlot {
                            kind: NqNumberedSlot::Frags,
                            slot: *slot,
                            value: number as i16,
                        });
                    }
                }
                QuakeWorldMessage::Baseline { state } => {
                    translated.push(NetQuakeMessage::Baseline { state: state.clone() });
                }
                QuakeWorldMessage::Static { state } => {
                    translated.push(NetQuakeMessage::Static { state: state.clone() });
                }
                QuakeWorldMessage::SetAngle { angles } => {
                    translated.push(NetQuakeMessage::SetAngle { angles: *angles });
                }
                QuakeWorldMessage::LightStyle { index, value } => {
                    translated.push(NetQuakeMessage::LightStyle {
                        index: *index,
                        value: value.clone(),
                    });
                }
                QuakeWorldMessage::StopSound { entity, channel } => {
                    translated.push(NetQuakeMessage::StopSound {
                        entity: *entity,
                        channel: *channel,
                    });
                }
                QuakeWorldMessage::Damage { armor, blood, source } => {
                    translated.push(NetQuakeMessage::Damage {
                        armor: *armor,
                        blood: *blood,
                        source: *source,
                    });
                }
                QuakeWorldMessage::TemporaryEntity { effect } => {
                    translated.push(NetQuakeMessage::TemporaryEntity { effect: effect.clone() });
                }
                QuakeWorldMessage::Pause { paused } => {
                    translated.push(NetQuakeMessage::Pause { paused: *paused });
                }
                QuakeWorldMessage::Text { kind, text } => {
                    let mapped = match kind {
                        qa_net::q1_net::QwText::CenterPrint => Some(NqText::CenterPrint),
                        qa_net::q1_net::QwText::Finale => Some(NqText::Finale),
                        qa_net::q1_net::QwText::Stufftext => None,
                    };
                    if let Some(kind) = mapped {
                        translated.push(NetQuakeMessage::Text {
                            kind,
                            text: text.clone(),
                        });
                    }
                }
                QuakeWorldMessage::Unit(_)
                | QuakeWorldMessage::ServerData { .. }
                | QuakeWorldMessage::ServerInfo { .. }
                | QuakeWorldMessage::Download { .. }
                | QuakeWorldMessage::ChokeCount { .. }
                | QuakeWorldMessage::ModelList { .. }
                | QuakeWorldMessage::SoundList { .. } => {}
            }
        }
        if frame {
            if let Some(own) = self.own_player.clone() {
                let view_height = if own.flags & 1024 != 0 {
                    8
                } else if own.flags & 512 != 0 {
                    -16
                } else {
                    22
                };
                let mut headed = vec![NetQuakeMessage::Time {
                    seconds: (now_ms / 1000.0) as f32,
                }];
                headed.append(&mut translated);
                translated = headed;
                for entity in &self.entities {
                    let mut state = entity.clone();
                    state.step = true;
                    translated.push(NetQuakeMessage::Entity { state });
                }
                for nail in &nails {
                    translated.push(NetQuakeMessage::Entity { state: nail.clone() });
                }
                for player in &players {
                    translated.push(NetQuakeMessage::Entity {
                        state: player_entity(player),
                    });
                }
                translated.push(NetQuakeMessage::ClientData {
                    weapon_alpha: 0,
                    data: NetQuakeClientData {
                        view_height,
                        ideal_pitch: 0,
                        punch_angles: [self.kick, 0, 0],
                        velocity: own.velocity,
                        items: self.stat(15),
                        on_ground: false,
                        in_water: false,
                        weapon_frame: u16::from(own.weapon_frame),
                        armor: self.stat(4) as u16,
                        weapon_model: self.stat(2) as u16,
                        health: self.stat(0) as i16,
                        ammo: self.stat(3) as u16,
                        shells: self.stat(6) as u16,
                        nails: self.stat(7) as u16,
                        rockets: self.stat(8) as u16,
                        cells: self.stat(9) as u16,
                        active_weapon: self.stat(10) as u32,
                    },
                });
                self.kick = 0;
            }
        }
        self.shared.borrow_mut().receive(&translated, now_ms as u64);
        self.prepare_skins();
        if frame {
            self.link_solids(&players, &nails);
        }
        if players.iter().any(|player| player.number == data.player_slot) {
            self.receive_prediction(now_ms);
        }
    }

    /// Feed a server snapshot to the predictor (donor `receivePrediction`).
    fn receive_prediction(&mut self, now_ms: f64) {
        let own = self.own_player.clone();
        let player = self.shared.borrow().player();
        let variables = self.variables.clone();
        let data = self.data.clone();
        let published = self.shared.borrow().output().is_some();
        let (Some(own), Some(player), Some(variables), Some(data)) = (own, player, variables, data) else {
            return;
        };
        if !published {
            return;
        }
        let view = self.shared.borrow().player_view(&player.actor);
        let ui = self.shared.borrow().player_ui(&player.actor);
        let (inventory_provider, character_provider, entities_provider) = {
            let shared = self.shared.borrow();
            let recipe = shared.content().recipe();
            (
                recipe.inventory.provider.clone(),
                recipe.character.definition.provider.clone(),
                recipe.map.entities.provider.clone(),
            )
        };
        let base = QwPredictionBase {
            sequence: self.acknowledged_sequence,
            command_time_ms: now_ms,
            origin: own.origin,
            velocity: own.velocity,
            angles: view.angles,
            health: ui.health,
            spectator: u8::from(data.spectator),
            weapon_frame: own.weapon_frame,
            frame: own.frame,
            active_weapon: ui.active_weapon.clone(),
            inventory: ui.inventory.clone(),
            source_weapon: self.stat(10),
        };
        if self.predictor.is_none() {
            let actor = {
                let shared = self.shared.borrow();
                shared
                    .identity()
                    .owned_actor(&player.actor, provider_id(&entities_provider))
                    .expect("QW predictor needs a locally owned actor")
            };
            let seed = QwPredictorSeed {
                actor,
                seat: self.seat.clone(),
                inventory_provider,
                character_provider,
                entities_provider,
                base: base.clone(),
                variables: variables.clone(),
            };
            self.predictor = Some(self.host.borrow_mut().build_predictor(&seed));
        }
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.receive(&base, &own, &variables);
            self.predicted = predictor.replay();
        }
    }

    /// Link frame solids into the scene (donor `linkSolids`).
    fn link_solids(&mut self, players: &[QwPlayerState], nails: &[Q1WireEntity])
    where
        H::Scene: QwRemoteScene,
    {
        let mut states: HashMap<u32, Q1WireEntity> = HashMap::new();
        for entity in self.entities.iter().chain(nails.iter()) {
            states.insert(entity.number, entity.clone());
        }
        for player in players {
            let entity = player_entity(player);
            states.insert(entity.number, entity);
        }
        let mut shared = self.shared.borrow_mut();
        let Some(output) = shared.output().cloned() else {
            return;
        };
        let actors: HashMap<u32, ActorId> = states
            .keys()
            .filter_map(|number| shared.actor_at(*number).map(|actor| (*number, actor)))
            .collect();
        let scene = shared.scene_mut().expect("QW scene is loaded");
        for actor in &self.linked {
            scene.unlink_actor(actor);
        }
        self.linked.clear();
        let player_slot = self.data.as_ref().map_or(u8::MAX, |data| data.player_slot);
        for (number, state) in &states {
            if *number == u32::from(player_slot) + 1 {
                continue;
            }
            let Some(actor) = actors.get(number) else {
                continue;
            };
            let target = SavedActorId::from(actor);
            let Some(body) = output.snapshot.bodies.iter().find(|body| body.id == target) else {
                continue;
            };
            let player = players.iter().find(|player| u32::from(player.number) + 1 == *number);
            let path = state
                .state
                .modelindex
                .checked_sub(1)
                .and_then(|index| self.model_names.get(usize::from(index)));
            let model = path
                .and_then(|path| path.strip_prefix('*'))
                .and_then(|number| number.parse::<u32>().ok());
            if player.is_none() && model.is_none() {
                continue;
            }
            if player.is_some_and(|player| player.flags & 512 != 0) {
                continue;
            }
            let bounds = model.map_or(body.state.bounds, |model| scene.model_bounds(model));
            let origin = vec3(state.state.origin);
            let absolute = Bounds {
                min: Vec3 {
                    x: origin.x + bounds.min.x - 1.0,
                    y: origin.y + bounds.min.y - 1.0,
                    z: origin.z + bounds.min.z - 1.0,
                },
                max: Vec3 {
                    x: origin.x + bounds.max.x + 1.0,
                    y: origin.y + bounds.max.y + 1.0,
                    z: origin.z + bounds.max.z + 1.0,
                },
            };
            scene.link_solid(QwSolidLink {
                actor: actor.clone(),
                state: BodyState {
                    origin,
                    angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    velocity: body.state.velocity,
                    bounds,
                    ground: body.state.ground.clone(),
                },
                absolute_bounds: absolute,
                model,
                monster: player.is_some(),
            });
            self.linked.push(actor.clone());
        }
    }

    /// Record a sent command (donor `prediction.sent`).
    pub fn prediction_sent(&mut self, sequence: u32, command: &QwUsercmd, now_ms: f64) {
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.sent(sequence, command, now_ms);
            self.predicted = predictor.replay();
        }
    }

    /// Record an acknowledged command (donor `prediction.acknowledged`).
    pub fn prediction_acknowledged(&mut self, sequence: u32, now_ms: f64) {
        self.acknowledged_sequence = sequence;
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.acknowledged(sequence, now_ms);
        }
    }
}

impl<H: QwRemoteHost + 'static> QwRemotePresentation<H> {
    /// Convert an actor command (donor `command`).
    pub fn command(&mut self, input: &ActorCommand) -> QwUsercmd
    where
        H::Scene: QwRemoteScene,
    {
        let UserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } = &input.command
        else {
            panic!("QW requires QuakeWorld input");
        };
        let mut command = QwUsercmd {
            msec: *milliseconds as u8,
            angles: *angles,
            forwardmove: *forward_move as i16,
            sidemove: *side_move as i16,
            upmove: *up_move as i16,
            buttons: *buttons as u8,
            impulse: *impulse as u8,
        };
        let spectator = self.data.as_ref().is_some_and(|data| data.spectator) && self.intermission.is_none();
        if spectator {
            if let Some(own) = self.own_player.clone() {
                let scoreboard = self.shared.borrow().scoreboard().clone();
                let mut users = HashMap::new();
                for (slot, info) in &self.userinfos {
                    users.insert(
                        i32::from(*slot),
                        QwCameraPlayer {
                            name: info.get("name").cloned().unwrap_or_default(),
                            spectator: info.get("*spectator").is_some_and(|value| !value.is_empty()),
                            frags: scoreboard.get(&u32::from(*slot)).map_or(0, |row| row.frags),
                        },
                    );
                }
                let previous = self.camera.view().cloned();
                let frame = QwCameraFrame {
                    viewer: own.clone(),
                    players: self.camera_players.clone(),
                    users,
                    seconds: self.host.borrow().now_seconds(),
                };
                command = self.camera.command(&command, &frame);
                if let Some(view) = self.camera.view().cloned().or(previous) {
                    let weapon_frame = if view.chase {
                        view.target.weapon_frame
                    } else {
                        own.weapon_frame
                    };
                    self.own_player = Some(QwPlayerState {
                        origin: arr(view.origin),
                        weapon_frame,
                        ..own
                    });
                    let now_ms = self.host.borrow().now_seconds() * 1000.0;
                    self.receive_prediction(now_ms);
                }
            }
        }
        let translated = ActorCommand {
            actor: input.actor.clone(),
            source: input.source.clone(),
            sequence: input.sequence,
            command: UserCommand::Q1Netquake {
                acknowledged_server_time_seconds: 0.0,
                view_angles: command.angles,
                forward_move: f64::from(command.forwardmove),
                side_move: f64::from(command.sidemove),
                up_move: f64::from(command.upmove),
                buttons: f64::from(command.buttons),
                impulse: f64::from(command.impulse),
            },
            arsenal: input.arsenal.clone(),
        };
        let selected = self.shared.borrow().command(&translated);
        command.impulse = selected.impulse as u8;
        command
    }

    /// Whether an actor is a player (donor `isPlayer`).
    #[must_use]
    pub fn is_player(&self, actor: &ActorId) -> bool {
        self.shared.borrow().is_player(actor)
    }

    /// World text (donor `worldText`).
    #[must_use]
    pub fn world_text(&self) -> Vec<WorldText> {
        Vec::new()
    }

    /// Player HUD state (donor `playerUi`).
    #[must_use]
    pub fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        self.shared.borrow().player_ui(actor)
    }

    /// Player view (donor `playerView`).
    #[must_use]
    pub fn player_view(&self, actor: &ActorId) -> PlayerView {
        let mut view = self.shared.borrow().player_view(actor);
        if let Some(intermission) = &self.intermission {
            view.origin = vec3(intermission.origin);
            view.angles = vec3(intermission.angles);
            view.view_height = 0.0;
            view.kick_angles = Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
            view.pitch_drift = Some(PlayerPitchDrift {
                grounded: false,
                ideal_pitch: 0.0,
                disabled: true,
            });
            return view;
        }
        if let Some(camera) = self.camera.view().cloned() {
            if self.data.as_ref().is_some_and(|data| data.spectator) {
                view.origin = camera.origin;
                view.angles = camera.angles;
                view.view_height = if camera.target.flags & 512 != 0 && camera.chase {
                    -16.0
                } else {
                    22.0
                };
                view.kick_angles = Some(Vec3 { x: 0.0, y: 0.0, z: 0.0 });
                view.pitch_drift = Some(PlayerPitchDrift {
                    grounded: false,
                    ideal_pitch: 0.0,
                    disabled: true,
                });
                return view;
            }
        }
        if let Some(predicted) = &self.predicted {
            view.origin = predicted.origin;
        }
        let disabled = view.pitch_drift.is_some_and(|drift| drift.disabled)
            || self.data.as_ref().is_some_and(|data| data.spectator);
        view.pitch_drift = Some(PlayerPitchDrift {
            grounded: self.predicted.is_some_and(|predicted| predicted.grounded),
            ideal_pitch: 0.0,
            disabled,
        });
        if self.own_player.as_ref().is_some_and(|own| own.flags & 512 != 0) {
            view.angles.z = 80.0;
        }
        view
    }

    /// Run a player command (donor `playerCommand`).
    pub fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        self.shared.borrow_mut().player_command(actor, name, args);
    }

    /// Character views (donor `characterViews`).
    #[must_use]
    pub fn character_views(&self) -> Vec<Q3CharacterView> {
        Vec::new()
    }

    /// Presentation models (donor `presentations`).
    #[must_use]
    pub fn presentations(&self) -> Vec<PresentationModel> {
        let shared = self.shared.borrow();
        let mut models = shared.presentations();
        for model in &mut models {
            let slot = shared.player_slot(&model.actor);
            let skin = slot.and_then(|slot| self.selected_skins.get(&(slot as u8)));
            if !self.skin_loading && model.path == "progs/player.mdl" {
                if let Some(skin) = skin {
                    model.indexed_skin = Some(PresentationIndexedSkin {
                        name: skin.name.clone(),
                        width: i64::from(skin.width),
                        height: i64::from(skin.height),
                        pixels: skin.pixels.clone(),
                    });
                }
            }
        }
        if self.intermission.is_some() {
            return models.into_iter().filter(|model| !model.view_weapon).collect();
        }
        let Some(player) = shared.player() else {
            return models;
        };
        drop(shared);
        let view = self.player_view(&player.actor);
        let camera = self.camera.view().cloned();
        let spectator = self.data.as_ref().is_some_and(|data| data.spectator);
        models
            .into_iter()
            .map(|mut model| {
                if model.view_weapon {
                    model.origin = Vec3 {
                        x: view.origin.x,
                        y: view.origin.y,
                        z: view.origin.z + view.view_height as f32,
                    };
                    model.angles = view.angles;
                    if camera.as_ref().is_some_and(|camera| camera.chase) {
                        let frame = i64::from(camera.as_ref().expect("chase camera").target.weapon_frame);
                        model.frame = frame;
                        model.old_frame = frame;
                    }
                    model.visible = model.visible && (!spectator || camera.as_ref().is_some_and(|camera| camera.chase));
                    return model;
                }
                if camera.as_ref().is_some_and(|camera| camera.chase)
                    && self.shared.borrow().player_slot(&model.actor)
                        == Some(u32::from(camera.as_ref().expect("chase camera").target.number))
                {
                    model.visible = false;
                }
                model
            })
            .collect()
    }

    /// Register a resolved resource (donor `registerResource`).
    pub fn register_resource(&mut self, content: &ContentId, path: &str, resource: &ResolvedResourceReference) {
        self.shared.borrow_mut().register_resource(content, path, resource);
    }

    /// Sample the current presentation (donor `samplePresentation`).
    pub fn sample_presentation(&mut self, now_ms: f64) -> Option<SimulationOutput> {
        let output = self.shared.borrow_mut().sample_presentation(now_ms as u64).cloned()?;
        let player = self.shared.borrow().player()?;
        let view = self.player_view(&player.actor);
        let target = SavedActorId::from(&player.actor);
        let mut sampled = output;
        for body in &mut sampled.snapshot.bodies {
            if body.id == target {
                body.state.origin = view.origin;
                body.state.angles = view.angles;
            }
        }
        sampled.events.clear();
        self.host.borrow_mut().publish(&sampled);
        Some(sampled)
    }

    /// Drain presentation events (donor `drainPresentationEvents`).
    pub fn drain_presentation_events(&mut self) -> Vec<Q1RemotePresentationEvent> {
        self.shared.borrow_mut().drain_presentation_events()
    }

    /// Handle a disconnect (donor `disconnected`).
    pub fn disconnected(&mut self, reason: &str) {
        self.shared.borrow_mut().disconnected(reason);
    }

    /// Print text (donor `print`).
    pub fn print(&mut self, text: &str) {
        self.host.borrow_mut().print(text);
    }
}

impl<H: QwRemoteHost + 'static> QwApplicationSkins for QwRemotePresentation<H> {
    fn names(&self) -> Vec<String> {
        self.skin_names()
    }

    fn loading(&mut self, value: bool) {
        self.set_skin_loading(value);
    }

    fn prepare(&mut self) {
        self.prepare_skins();
    }
}

impl<H: QwRemoteHost + 'static> QwApplicationPrediction for QwRemotePresentation<H> {
    fn sent(&mut self, sequence: u32, command: &QwUsercmd, now_ms: f64) {
        self.prediction_sent(sequence, command, now_ms);
    }

    fn acknowledged(&mut self, sequence: u32, now_ms: f64) {
        self.prediction_acknowledged(sequence, now_ms);
    }
}

impl<H: QwRemoteHost + 'static> QwApplicationClientHost for QwRemotePresentation<H>
where
    H::Scene: QwRemoteScene,
{
    fn downloads(&mut self) -> Option<&mut dyn QwApplicationDownloads> {
        self.downloads
            .as_mut()
            .map(|downloads| downloads as &mut dyn QwApplicationDownloads)
    }

    fn skins(&mut self) -> Option<&mut dyn QwApplicationSkins> {
        Some(self)
    }

    fn prediction(&mut self) -> Option<&mut dyn QwApplicationPrediction> {
        Some(self)
    }

    fn server_data(&mut self, data: &QwServerData) {
        QwRemotePresentation::server_data(self, data);
    }

    fn game_state(&mut self, data: &QwServerData, models: &[String], sounds: &[String]) -> i32 {
        QwRemotePresentation::game_state(self, data, models, sounds)
    }

    fn receive(&mut self, messages: &[QuakeWorldMessage], now_ms: f64) {
        QwRemotePresentation::receive(self, messages, now_ms);
    }

    fn command(&mut self, command: &ActorCommand) -> QwUsercmd {
        QwRemotePresentation::command(self, command)
    }

    fn take_spectator_teleport(&mut self) -> Option<Vec3> {
        self.camera.take_teleport()
    }

    fn disconnected(&mut self, reason: &str) {
        QwRemotePresentation::disconnected(self, reason);
    }

    fn print(&mut self, text: &str) {
        QwRemotePresentation::print(self, text);
    }
}

#[cfg(test)]
mod tests {
    use super::super::qw_types::{QwDownloadCategory, QwDownloadReceive, QwDownloadRequest};
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_net::q1_net::{QwDownload, QwProjectile};
    use qa_net::q1_wide::QwProfile;
    use qa_world::session::SessionMode;

    use crate::persistence::recipe::fixture_recipe;

    #[derive(Debug, Default)]
    struct TestHostState {
        prepared: Vec<String>,
        checksums: Vec<(String, String)>,
        printed: Vec<String>,
        sent: Vec<String>,
        published: usize,
        disconnected: Vec<String>,
        generation: u32,
        seeds: Vec<QwPredictorSeed>,
        predictor_calls: Vec<String>,
        now_seconds: f64,
        replay: Option<QwPredictedPlayer>,
    }

    struct TestContent {
        recipe: crate::persistence::recipe::ExecutableRecipe,
    }

    impl Q1RemoteContent for TestContent {
        fn recipe(&self) -> &crate::persistence::recipe::ExecutableRecipe {
            &self.recipe
        }

        fn world_geometry(&self) -> &str {
            "test-world"
        }
    }

    #[derive(Debug)]
    struct TestScene {
        unlinked: Vec<ActorId>,
        links: Vec<QwSolidLink>,
        bounds: Bounds,
    }

    impl Default for TestScene {
        fn default() -> Self {
            Self {
                unlinked: Vec::new(),
                links: Vec::new(),
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
            }
        }
    }

    impl QwRemoteScene for TestScene {
        fn trace_spectator(&self, _start: Vec3, end: Vec3, _pass_actor: Option<&ActorId>) -> QwCameraTrace {
            QwCameraTrace {
                fraction: 1.0,
                end,
                in_water: false,
            }
        }

        fn unlink_actor(&mut self, actor: &ActorId) {
            self.unlinked.push(actor.clone());
        }

        fn model_bounds(&self, _model: u32) -> Bounds {
            self.bounds
        }

        fn link_solid(&mut self, link: QwSolidLink) {
            self.links.push(link);
        }
    }

    struct TestPredictor {
        state: Rc<RefCell<TestHostState>>,
    }

    impl QwPredictor for TestPredictor {
        fn receive(&mut self, base: &QwPredictionBase, _own: &QwPlayerState, _variables: &QwMoveVariables) {
            self.state
                .borrow_mut()
                .predictor_calls
                .push(format!("receive:{}:{}", base.sequence, base.spectator));
        }

        fn sent(&mut self, sequence: u32, _command: &QwUsercmd, _now_ms: f64) {
            self.state.borrow_mut().predictor_calls.push(format!("sent:{sequence}"));
        }

        fn acknowledged(&mut self, sequence: u32, _now_ms: f64) {
            self.state.borrow_mut().predictor_calls.push(format!("ack:{sequence}"));
        }

        fn replay(&mut self) -> Option<QwPredictedPlayer> {
            self.state.borrow_mut().predictor_calls.push("replay".to_string());
            self.state.borrow().replay
        }
    }

    struct TestDownloads;

    impl QwApplicationDownloads for TestDownloads {
        fn request(&mut self, _path: &str, _category: QwDownloadCategory) -> QwDownloadRequest {
            QwDownloadRequest::Available
        }

        fn receive(&mut self, _result: &QwDownload) -> QwDownloadReceive {
            QwDownloadReceive::Complete
        }

        fn close(&mut self) {}
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
            TestScene::default()
        }

        fn send_command(&mut self, text: &str) {
            self.state.borrow_mut().sent.push(text.to_string());
        }

        fn print(&mut self, text: &str) {
            self.state.borrow_mut().printed.push(text.to_string());
        }

        fn publish(&mut self, _output: &SimulationOutput) {
            self.state.borrow_mut().published += 1;
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

    impl QwRemoteHost for TestHost {
        type Predictor = TestPredictor;
        type Downloads = TestDownloads;

        fn prepare_server_data(&mut self, data: &QwServerData) {
            self.state.borrow_mut().prepared.push(data.level.clone());
        }

        fn map_checksum(&mut self, world: &Q1RemoteWorld, game_directory: &str) -> i32 {
            self.state
                .borrow_mut()
                .checksums
                .push((world.map.clone(), game_directory.to_string()));
            4242
        }

        fn sound_available(_content: &TestContent, path: &str) -> bool {
            path.ends_with("shotgun.wav")
        }

        fn now_seconds(&self) -> f64 {
            self.state.borrow().now_seconds
        }

        fn build_predictor(&mut self, seed: &QwPredictorSeed) -> TestPredictor {
            self.state.borrow_mut().seeds.push(seed.clone());
            TestPredictor {
                state: self.state.clone(),
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn harness() -> (QwRemotePresentation<TestHost>, Rc<RefCell<TestHostState>>) {
        let state = Rc::new(RefCell::new(TestHostState::default()));
        let identity = IdentityOwner::create("test").unwrap();
        let client_id = identity.client(0, 1);
        let seat = identity.seat(0);
        let session = EngineSession::new(IdentityOwner::create("test").unwrap(), SessionMode::Local);
        let presentation = QwRemotePresentation::new(QwRemotePresentationOptions {
            identity,
            session,
            client: SessionClient::new(client_id),
            seat,
            content: None,
            skin_options: QwSkinOptions {
                read: Box::new(|_| None),
                noskins: Box::new(|| 0),
                baseskin: Box::new(|| "base".to_string()),
                allskins: Box::new(String::new),
            },
            camera_options: None,
            downloads: Some(TestDownloads),
            host: TestHost { state: state.clone() },
        });
        (presentation, state)
    }

    fn server_data() -> QwServerData {
        QwServerData {
            protocol: QwProfile::Quakeworld,
            server_count: 1,
            game_directory: "qw".to_string(),
            player_slot: 0,
            spectator: false,
            level: "dm1".to_string(),
            move_variables: QwMoveVariables::default(),
        }
    }

    fn models() -> Vec<String> {
        vec![
            "maps/dm1.bsp".to_string(),
            "progs/player.mdl".to_string(),
            "progs/spike.mdl".to_string(),
            "*1".to_string(),
        ]
    }

    fn sounds() -> Vec<String> {
        vec!["weapons/shotgun.wav".to_string(), "misc/missing.wav".to_string()]
    }

    fn own_player() -> QwPlayerState {
        QwPlayerState {
            number: 0,
            flags: 0,
            origin: [10.0, 20.0, 30.0],
            velocity: [0, 0, 0],
            model_index: 2,
            frame: 1,
            skin: 0,
            effects: 0,
            weapon_frame: 3,
            milliseconds: 0,
            command: QwUsercmd {
                angles: [30.0, 90.0, 0.0],
                ..QwUsercmd::default()
            },
        }
    }

    fn brush_entity() -> Q1WireEntity {
        Q1WireEntity {
            number: 5,
            state: WideEntityState {
                origin: [1.0, 2.0, 3.0],
                modelindex: 4,
                ..WideEntityState::default()
            },
            lerp_finish_seconds: 0.0,
            step: false,
            quakeworld_flags: 0,
        }
    }

    fn frame_messages() -> Vec<QuakeWorldMessage> {
        vec![
            QuakeWorldMessage::Userinfo {
                slot: 0,
                user_id: 7,
                value: "\\name\\ska\\skin\\red\\topcolor\\1\\bottomcolor\\2".to_string(),
            },
            QuakeWorldMessage::Stat { index: 0, value: 100 },
            QuakeWorldMessage::Stat { index: 10, value: 4 },
            QuakeWorldMessage::Stat { index: 15, value: 3 },
            QuakeWorldMessage::Player { state: own_player() },
            QuakeWorldMessage::PacketEntities {
                sequence: 1,
                delta_sequence: None,
                entities: vec![brush_entity()],
            },
        ]
    }

    fn admit(presentation: &mut QwRemotePresentation<TestHost>) {
        let data = server_data();
        presentation.server_data(&data);
        let checksum = presentation.game_state(&data, &models(), &sounds());
        assert_eq!(checksum, 4242);
    }

    #[test]
    fn game_state_admits_and_returns_checksum() {
        let (mut presentation, state) = harness();
        admit(&mut presentation);
        assert_eq!(state.borrow().prepared, vec!["dm1".to_string()]);
        assert_eq!(
            state.borrow().checksums,
            vec![("maps/dm1.bsp".to_string(), "qw".to_string())]
        );
        let player = presentation.player().expect("player is admitted");
        assert_eq!(player.source_entity, 1);
        assert!(presentation.move_variables().is_some());
    }

    #[test]
    fn receive_before_admission_stashes_cd_track() {
        let (mut presentation, _) = harness();
        presentation.receive(&[QuakeWorldMessage::CdTrack { track: 5 }], 0.0);
        assert!(presentation.player().is_none());
        admit(&mut presentation);
        assert!(presentation.player().is_some());
    }

    #[test]
    fn frame_translates_player_userinfo_and_stats() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        let player = presentation.player().expect("player is admitted");
        let view = presentation.player_view(&player.actor);
        assert_eq!(arr(view.origin), [10.0, 20.0, 30.0]);
        let ui = presentation.player_ui(&player.actor);
        assert_eq!(ui.health, 100.0);
        let scoreboard = presentation.scoreboard();
        let row = scoreboard.get(&0).expect("slot zero row");
        assert_eq!(row.name, "ska");
        assert_eq!(row.colors, 18);
        assert_eq!(presentation.skin_names(), vec!["skins/red.pcx".to_string()]);
    }

    #[test]
    #[should_panic(expected = "Invalid QW sound index")]
    fn sound_index_out_of_range_panics() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[QuakeWorldMessage::Sound {
                entity: 1,
                channel: 0,
                index: 9,
                origin: [0.0, 0.0, 0.0],
                volume: 255,
                attenuation: 1.0,
            }],
            100.0,
        );
    }

    #[test]
    fn unavailable_sounds_are_dropped() {
        use super::super::remote_q1::Q1RemoteEvent;
        use super::super::unified_event_codec::Q1Event;
        use qa_content::contract::{
            ContentDigest, ContentId as ContractContentId, LooseMount, MountId, MountIdentity, MountPlanId, ResourceId,
            ResourceProvenance, ResourceResolution,
        };
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.register_resource(
            &ContentId("test".to_string()),
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
                digest: ContentDigest("sha256:test".to_string()),
                byte_length: 8,
                resolution: ResourceResolution::DefaultOrder {
                    plan: MountPlanId("plan".to_string()),
                    rank: 0,
                },
            },
        );
        presentation.receive(&frame_messages(), 50.0);
        presentation.drain_presentation_events();
        presentation.receive(
            &[
                QuakeWorldMessage::Sound {
                    entity: 1,
                    channel: 0,
                    index: 1,
                    origin: [0.0, 0.0, 0.0],
                    volume: 255,
                    attenuation: 1.0,
                },
                QuakeWorldMessage::Sound {
                    entity: 1,
                    channel: 0,
                    index: 2,
                    origin: [0.0, 0.0, 0.0],
                    volume: 255,
                    attenuation: 1.0,
                },
            ],
            100.0,
        );
        let events = presentation.drain_presentation_events();
        let sounds: Vec<&Q1Event> = events
            .iter()
            .filter_map(|event| match &event.event {
                Q1RemoteEvent::Q1(payload @ Q1Event::Sound { .. }) => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(sounds.len(), 1);
    }

    #[test]
    fn set_info_updates_scoreboard_name() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        presentation.receive(
            &[QuakeWorldMessage::SetInfo {
                slot: 0,
                key: "name".to_string(),
                value: "ska2".to_string(),
            }],
            200.0,
        );
        let scoreboard = presentation.scoreboard();
        assert_eq!(scoreboard.get(&0).expect("slot zero row").name, "ska2");
    }

    #[test]
    fn slot_stat_frags_records_scoreboard() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[QuakeWorldMessage::SlotStat {
                kind: QwSlotStat::Frags,
                slot: 1,
                value: QwSlotValue::Integer(12),
            }],
            100.0,
        );
        let scoreboard = presentation.scoreboard();
        assert_eq!(scoreboard.get(&1).expect("slot one row").frags, 12);
    }

    #[test]
    fn speed_updates_move_variables() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(
            &[
                QuakeWorldMessage::Speed {
                    entity_gravity: false,
                    value: 400.0,
                },
                QuakeWorldMessage::Speed {
                    entity_gravity: true,
                    value: 0.5,
                },
            ],
            100.0,
        );
        let variables = presentation.move_variables().expect("variables");
        assert_eq!(variables.max_speed, 400.0);
        assert_eq!(variables.entity_gravity, 0.5);
    }

    #[test]
    fn intermission_overrides_view() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        presentation.receive(
            &[QuakeWorldMessage::Intermission {
                origin: [7.0, 8.0, 9.0],
                angles: [10.0, 20.0, 30.0],
            }],
            200.0,
        );
        let player = presentation.player().expect("player is admitted");
        let view = presentation.player_view(&player.actor);
        assert_eq!(arr(view.origin), [7.0, 8.0, 9.0]);
        assert_eq!(arr(view.angles), [10.0, 20.0, 30.0]);
        assert_eq!(view.view_height, 0.0);
        assert!(view.pitch_drift.expect("drift").disabled);
    }

    #[test]
    fn prediction_builds_and_overrides_origin() {
        let (mut presentation, state) = harness();
        state.borrow_mut().replay = Some(QwPredictedPlayer {
            origin: Vec3 { x: 1.0, y: 2.0, z: 3.0 },
            grounded: true,
        });
        admit(&mut presentation);
        presentation.prediction_acknowledged(9, 50.0);
        presentation.receive(&frame_messages(), 100.0);
        assert_eq!(state.borrow().seeds.len(), 1);
        let seed = state.borrow().seeds[0].clone();
        assert_eq!(seed.base.sequence, 9);
        assert_eq!(seed.base.spectator, 0);
        assert_eq!(seed.base.source_weapon, 4);
        let player = presentation.player().expect("player is admitted");
        let view = presentation.player_view(&player.actor);
        assert_eq!(view.origin.x, 1.0);
        assert!(view.pitch_drift.expect("drift").grounded);
        assert!(state.borrow().predictor_calls.iter().any(|call| call == "receive:9:0"));
    }

    #[test]
    fn link_solids_links_brush_model() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        presentation.receive(&frame_messages(), 200.0);
        let (links, unlinked) =
            presentation.with_scene(|scene: &TestScene| (scene.links.clone(), scene.unlinked.clone()));
        assert_eq!(links.len(), 2);
        assert_eq!(unlinked.len(), 1);
        assert_eq!(links[0].model, Some(1));
        assert!(!links[0].monster);
        assert_eq!(arr(links[0].state.origin), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn command_maps_quakeworld_input() {
        use qa_core::identity::IdentityOwner;
        use qa_net::common::commands::CommandSource;
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        let player = presentation.player().expect("player is admitted");
        let output = presentation.command(&ActorCommand {
            actor: player.actor,
            source: CommandSource::LocalSeat {
                seat: IdentityOwner::create("test").unwrap().seat(0),
            },
            sequence: 3,
            command: UserCommand::Q1Quakeworld {
                milliseconds: 50.0,
                angles: [10.0, 20.0, 30.0],
                forward_move: 100.0,
                side_move: -50.0,
                up_move: 0.0,
                buttons: 3.0,
                impulse: 7.0,
            },
            arsenal: None,
        });
        assert_eq!(output.msec, 50);
        assert_eq!(output.angles, [10.0, 20.0, 30.0]);
        assert_eq!(output.forwardmove, 100);
        assert_eq!(output.sidemove, -50);
        assert_eq!(output.buttons, 3);
        assert_eq!(output.impulse, 7);
    }

    #[test]
    fn sample_presentation_patches_player_body() {
        let (mut presentation, state) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        let before = state.borrow().published;
        let sampled = presentation.sample_presentation(150.0).expect("sampled output");
        assert!(sampled.events.is_empty());
        assert_eq!(state.borrow().published, before + 2);
        let player = presentation.player().expect("player is admitted");
        let view = presentation.player_view(&player.actor);
        let target = SavedActorId::from(&player.actor);
        let body = sampled
            .snapshot
            .bodies
            .iter()
            .find(|body| body.id == target)
            .expect("player body");
        assert_eq!(body.state.origin, view.origin);
        assert_eq!(body.state.angles, view.angles);
    }

    #[test]
    fn nails_translate_without_disturbing_view() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        let mut messages = frame_messages();
        messages.push(QuakeWorldMessage::Nails {
            projectiles: vec![QwProjectile {
                origin: [1, 2, 3],
                pitch: 4,
                yaw: 5,
            }],
        });
        presentation.receive(&messages, 100.0);
        let player = presentation.player().expect("player is admitted");
        let view = presentation.player_view(&player.actor);
        assert_eq!(arr(view.origin), [10.0, 20.0, 30.0]);
    }

    #[test]
    fn host_delegation_records_calls() {
        let (mut presentation, state) = harness();
        admit(&mut presentation);
        assert!(state.borrow().generation > 0);
        presentation.print("hello");
        presentation.disconnected("bye");
        let player = presentation.player().expect("player is admitted");
        presentation.receive(&frame_messages(), 100.0);
        presentation.player_command(&player.actor, "say", &["hi".to_string()]);
        assert_eq!(state.borrow().printed, vec!["hello".to_string()]);
        assert_eq!(state.borrow().disconnected, vec!["bye".to_string()]);
        assert!(state.borrow().sent.iter().any(|command| command == "say \"hi\""));
    }

    #[test]
    fn skin_names_ignore_loading_flag() {
        let (mut presentation, _) = harness();
        admit(&mut presentation);
        presentation.receive(&frame_messages(), 100.0);
        presentation.set_skin_loading(true);
        assert_eq!(presentation.skin_names(), vec!["skins/red.pcx".to_string()]);
        presentation.set_skin_loading(false);
        assert_eq!(presentation.skin_names(), vec!["skins/red.pcx".to_string()]);
    }
}
