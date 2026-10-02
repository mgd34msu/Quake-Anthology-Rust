//! Native NetQuake server host binding.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/network-q1.ts`
//! (`Q1ApplicationServerBindingOptions`, `createQ1ApplicationServerHost`).
//!
//! # Missing siblings
//!
//! - `simulation/runtime.ts` (`SharedSimulation`): [`Q1NativeHostSimulation`]
//!   extends the QuakeC host simulation seam with the native Q1 surface.
//! - `app/bootstrap/content.ts` (`LoadedApplicationContent`) and
//!   `world/session/session.ts` (`EngineSession`): reuses
//!   [`Q1QuakeCHostContent`] and [`Q1QuakeCHostSession`].
//! - `app/bootstrap/network/q1-types.ts`: reuses the `network-q1-quakec`
//!   host mirrors; unify post-merge.
//! - `network/q1/profile.ts` (`createNetQuakeCodec.maxPrecache`): arrives as
//!   [`Q1ServerOptions::max_precache`].

use std::collections::HashMap;

use qa_content::contract::ContentId;
use qa_content::q1::foundation::types::WEAPONS;
use qa_core::identity::{ActorId, ClientId, OwnedActor};
use qa_core::math::Vec3;
use qa_net::common::commands::{ActorCommand, CommandSource};
use qa_net::common::endpoint::NetworkAddress;
use qa_net::protocol::q1::ENTALPHA_ZERO;
use qa_net::q1_wide::{entalpha_encode, entscale_encode, NqProfile};
use qa_world::movement::q1::types::Q1MovementState;
use qa_world::movement::types::Q1UserCommand;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use super::network_q1_quakec::{
    ModClientEventKind, Q1ApplicationAdmission, Q1ApplicationFrame, Q1ApplicationGameState, Q1ApplicationMessage,
    Q1ApplicationPlayer, Q1ClientData, Q1ExtendedEntityState, Q1NamedSlotKind, Q1NumberedSlotKind, Q1PrintFn,
    Q1QuakeCHostContent, Q1QuakeCHostSession, Q1QuakeCHostSimulation, Q1QuakeCNetGame, Q1QuakeCServerOptions,
    Q1RejectsFn, QuakeCSourceKind,
};
use super::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// Mirror of donor `Q1ProtocolIdentity` (canonical home: `qa-net` protocol).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1ProtocolIdentity {
    /// Protocol version.
    pub version: u32,
}

impl Q1ProtocolIdentity {
    /// Map to the wire profile.
    #[must_use]
    pub fn profile(&self) -> NqProfile {
        match self.version {
            15 => NqProfile::Netquake,
            666 => NqProfile::Fitzquake,
            999 => NqProfile::Rmq { flags: 0 },
            _ => NqProfile::Netquake,
        }
    }
}

/// Narrow movement view for native Q1 networking.
#[derive(Debug, Clone)]
pub struct Q1NetworkMovement {
    /// NetQuake movement state.
    pub state: Q1MovementState,
    /// View height.
    pub view_height: f64,
}

/// Narrow player UI view for native Q1 networking.
#[derive(Debug, Clone)]
pub struct Q1NetworkUi {
    /// Health.
    pub health: f64,
    /// Ammo count.
    pub ammo: f64,
    /// Armor kind and points.
    pub armor: Q1NetworkArmor,
}

/// Narrow armor view.
#[derive(Debug, Clone)]
pub struct Q1NetworkArmor {
    /// True when no armor.
    pub none: bool,
    /// True for Q1 armor.
    pub q1: bool,
    /// Absorption fraction.
    pub absorption: f64,
    /// Armor points.
    pub points: f64,
}

/// Narrow body view.
#[derive(Debug, Clone)]
pub struct Q1NetworkBody {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
}

/// Narrow presentation view.
#[derive(Debug, Clone)]
pub struct Q1NetworkPresentation {
    /// Actor.
    pub actor: ActorId,
    /// View weapon flag.
    pub view_weapon: bool,
    /// Model path override.
    pub path: Option<String>,
    /// Frame override.
    pub frame: Option<f64>,
    /// Skin override.
    pub skin: Option<f64>,
    /// Effects override.
    pub effects: Option<i32>,
    /// Previous origin.
    pub previous_origin: Option<Vec3>,
}

/// Native Q1 entity view.
#[derive(Debug, Clone)]
pub struct Q1NetworkEntity {
    /// Owning actor.
    pub actor: OwnedActor,
    /// Model path.
    pub model: String,
    /// Frame.
    pub frame: f64,
    /// Skin.
    pub skin: f64,
    /// Effects.
    pub effects: i32,
    /// Movement kind.
    pub movement: String,
    /// Alpha number field.
    pub alpha: f64,
    /// Scale number field.
    pub scale: f64,
}

/// Native Q1 player state view.
#[derive(Debug, Clone)]
pub struct Q1NetworkPlayerState {
    /// Powerup expirations by name.
    pub powerups: Vec<(String, f64)>,
    /// Current weapon name.
    pub weapon: String,
    /// Weapon frame.
    pub weapon_frame: f64,
    /// Alpha.
    pub alpha: f64,
    /// Scale.
    pub scale: f64,
}

/// Native Q1 client record view.
#[derive(Debug, Clone)]
pub struct Q1NetworkClient {
    /// Client slot.
    pub slot: u32,
    /// Player name.
    pub name: String,
    /// Shirt color.
    pub shirt: i32,
    /// Pants color.
    pub pants: i32,
    /// Frags.
    pub frags: f64,
}

/// Native Q1 simulation surface.
pub trait Q1NativeHostSimulation: Q1QuakeCHostSimulation {
    /// Donor `actors.sourceOf` slot for the map entities provider.
    fn source_slot(&self, actor: &ActorId) -> Option<i32>;
    /// Donor `bodies.read`.
    fn read_body(&self, actor: &ActorId) -> Option<Q1NetworkBody>;
    /// Donor `presentations`.
    fn network_presentations(&self) -> Vec<Q1NetworkPresentation>;
    /// Donor `movementPlayer`.
    fn network_movement(&self, actor: &ActorId) -> Option<Q1NetworkMovement>;
    /// Donor `playerUi`.
    fn network_player_ui(&self, actor: &ActorId) -> Q1NetworkUi;
    /// Donor `inventory.count`.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> f64;
    /// Donor `quakecSource()?.kind`.
    fn quakec_source_kind(&self) -> Option<QuakeCSourceKind>;
    /// Donor `movementPlayer(actor)?.client`.
    fn movement_client(&self, actor: &ActorId) -> Option<ClientId>;
}

/// Native Q1 game surface.
pub trait Q1NativeGame {
    /// Maximum clients.
    fn max_clients(&self) -> u32;
    /// Deathmatch mode.
    fn deathmatch(&self) -> i32;
    /// Edition name.
    fn edition(&self) -> String;
    /// Program selection name.
    fn program(&self) -> String;
    /// Whether id1 precaches are in use.
    fn uses_id1_precaches(&self) -> bool;
    /// Whether precaches are frozen.
    fn precaches_frozen(&self) -> bool;
    /// Model precache paths including the empty zeroth entry.
    fn precache_models(&self) -> Vec<String>;
    /// Sound precache paths including the empty zeroth entry.
    fn precache_sounds(&self) -> Vec<String>;
    /// Game time in seconds.
    fn game_time(&self) -> f64;
    /// Map name.
    fn map_name(&self) -> String;
    /// Worldspawn message.
    fn world_message(&self) -> Option<String>;
    /// Total secrets.
    fn total_secrets(&self) -> i32;
    /// Total monsters.
    fn total_monsters(&self) -> i32;
    /// Found secrets.
    fn found_secrets(&self) -> i32;
    /// Killed monsters.
    fn killed_monsters(&self) -> i32;
    /// Native entities.
    fn network_entities(&self) -> Vec<Q1NetworkEntity>;
    /// Native player state.
    fn network_player(&self, actor: &ActorId) -> Option<Q1NetworkPlayerState>;
    /// Weapon item id for a weapon name.
    fn weapon_item(&self, weapon: &str) -> String;
    /// Weapon view model for a weapon and state.
    fn weapon_model(&self, weapon: &str, state: &Q1NetworkPlayerState) -> String;
    /// Composition client records.
    fn composition_clients(&self) -> Vec<Q1NetworkClient>;
    /// Require a client record.
    fn require_client(&self, actor: &ActorId) -> Vec<(String, String)>;
    /// Set client userinfo.
    fn set_client_userinfo(&mut self, actor: &ActorId, values: Vec<(String, String)>);
    /// Set client colors.
    fn set_client_colors(&mut self, actor: &ActorId, shirt: i32, pants: i32);
}

/// Binding options for the native NetQuake server host.
pub struct Q1ServerOptions<S, C, E, G, Q> {
    /// Engine session.
    pub session: E,
    /// Address rejection predicate.
    pub rejects: Option<Q1RejectsFn>,
    /// Shared simulation.
    pub simulation: S,
    /// Loaded content.
    pub content: C,
    /// Protocol identity.
    pub protocol: Q1ProtocolIdentity,
    /// Donor `createNetQuakeCodec(protocol, reader).maxPrecache`.
    pub max_precache: usize,
    /// Native game handle.
    pub game: Option<G>,
    /// QuakeC game handle for the QuakeC branch.
    pub quakec_game: Option<Q>,
    /// Print sink.
    pub print: Q1PrintFn,
}

/// Native host creation errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q1ServerError {
    /// Missing native game.
    #[error("Q1 network requires the Q1 source game")]
    MissingGame,
    /// Non-id1 composition.
    #[error("Native ordered Q1 precaches currently require classic id1 source declarations")]
    NonId1Composition,
    /// Precaches still loading.
    #[error("Q1 source precaches are still loading")]
    PrecachesLoading,
    /// Precache overflow.
    #[error("NetQuake precache overflow")]
    PrecacheOverflow,
    /// Missing model resource.
    #[error("Q1 model precache resource is missing: {0}")]
    MissingModel(String),
    /// Missing sound resource.
    #[error("Q1 sound precache resource is missing: {0}")]
    MissingSound(String),
    /// Content failure.
    #[error("Q1 content failure: {0}")]
    Content(String),
    /// Unprecached resource.
    #[error("Q1 resource was not precached before signon: {0}")]
    Unprecached(String),
    /// Missing source address.
    #[error("Q1 actor has no source address")]
    MissingAddress,
    /// Missing QuakeC delegate.
    #[error("Q1 QuakeC branch has no QuakeC game handle")]
    MissingQuakeC,
}

/// Routed message with its audience.
#[derive(Debug, Clone, PartialEq)]
struct RoutedMessage {
    /// Recipient actor, or broadcast when [`None`].
    recipient: Option<ActorId>,
    /// Reliable delivery.
    reliable: bool,
    /// Message.
    message: Q1ApplicationMessage,
}

/// Scoreboard row.
#[derive(Debug, Clone, PartialEq)]
struct BoardEntry {
    /// Player name.
    name: String,
    /// Packed colors.
    colors: i32,
    /// Frags.
    frags: f64,
}

/// Native NetQuake server host.
pub struct Q1NativeHost<S, E, G> {
    /// Engine session.
    session: E,
    /// Address rejection predicate.
    rejects: Option<Q1RejectsFn>,
    /// Shared simulation.
    simulation: S,
    /// Native game.
    game: G,
    /// Protocol identity.
    protocol: Q1ProtocolIdentity,
    /// Wide protocol (anything but 15).
    wide: bool,
    /// Maximum clients.
    max_clients: u32,
    /// Model precache indexes by path.
    models: HashMap<String, i32>,
    /// Sound precache indexes by path.
    sounds: HashMap<String, i32>,
    /// Sound resource ids by path.
    sound_resources: HashMap<String, qa_content::contract::ResourceId>,
    /// Admitted players by client slot.
    clients: HashMap<u32, Q1ApplicationPlayer>,
    /// Muzzle flash actors for the current frame.
    muzzle_flashes: std::collections::HashSet<ActorId>,
    /// Routed messages awaiting frame publication.
    routed: Vec<RoutedMessage>,
    /// Scoreboard rows by client slot.
    board: HashMap<u32, BoardEntry>,
    /// Pause flag at the previous observation.
    previous_pause: bool,
    /// Operator messages queued for the next observation.
    operator_messages: Vec<RoutedMessage>,
    /// Print sink.
    print: Q1PrintFn,
}

/// Unified server host across the native and QuakeC branches.
pub enum Q1ServerHost<S, E, G, Qs, Qe, Qg> {
    /// Native source host.
    Native(Q1NativeHost<S, E, G>),
    /// QuakeC source host.
    QuakeC(super::network_q1_quakec::Q1QuakeCNetHost<Qs, Qe, Qg>),
}

/// Create the Q1 application server host.
pub fn create_q1_application_server_host<S, C, E, G, Q>(
    options: Q1ServerOptions<S, C, E, G, Q>,
) -> Result<Q1ServerHost<S, E, G, S, E, Q>, Q1ServerError>
where
    S: Q1NativeHostSimulation,
    C: Q1QuakeCHostContent,
    E: Q1QuakeCHostSession,
    G: Q1NativeGame,
    Q: Q1QuakeCNetGame,
{
    if options.simulation.quakec_source_kind() == Some(QuakeCSourceKind::Netquake) {
        let Some(game) = options.quakec_game else {
            return Err(Q1ServerError::MissingQuakeC);
        };
        let host = super::network_q1_quakec::create_quakec_netquake_host(Q1QuakeCServerOptions {
            session: options.session,
            rejects: options.rejects,
            simulation: options.simulation,
            content: options.content,
            protocol: options.protocol.profile(),
            max_precache: options.max_precache,
            print: options.print,
            game: Some(game),
        })
        .map_err(|error| Q1ServerError::Content(error.to_string()))?;
        return Ok(Q1ServerHost::QuakeC(host));
    }
    let Some(game) = options.game else {
        return Err(Q1ServerError::MissingGame);
    };
    if !game.uses_id1_precaches() || game.edition() != "classic" || game.program() != "id1" {
        return Err(Q1ServerError::NonId1Composition);
    }
    if !game.precaches_frozen() {
        return Err(Q1ServerError::PrecachesLoading);
    }
    let mut simulation = options.simulation;
    if game.precache_models().len() > options.max_precache || game.precache_sounds().len() > options.max_precache {
        return Err(Q1ServerError::PrecacheOverflow);
    }
    let models: HashMap<String, i32> = game.precache_models()[1..]
        .iter()
        .enumerate()
        .map(|(ordinal, path)| (path.clone(), ordinal as i32 + 1))
        .collect();
    let sounds: HashMap<String, i32> = game.precache_sounds()[1..]
        .iter()
        .enumerate()
        .map(|(ordinal, path)| (path.clone(), ordinal as i32 + 1))
        .collect();
    let mounts = options
        .content
        .for_content(&simulation.recipe().map_entities_content)
        .map_err(|error| Q1ServerError::Content(error.to_string()))?;
    for path in models.keys() {
        if !path.starts_with('*')
            && mounts
                .resolve(path)
                .map_err(|error| Q1ServerError::Content(error.to_string()))?
                .is_none()
        {
            return Err(Q1ServerError::MissingModel(path.clone()));
        }
    }
    let mut sound_resources = HashMap::new();
    for path in sounds.keys() {
        let resolved = format!("sound/{path}");
        match mounts
            .resolve(&resolved)
            .map_err(|error| Q1ServerError::Content(error.to_string()))?
        {
            None => return Err(Q1ServerError::MissingSound(path.clone())),
            Some(resource) => {
                sound_resources.insert(path.clone(), resource.id.clone());
                simulation.register_resource(&simulation.recipe().map_entities_content, &resolved, &resource);
            }
        }
    }
    let max_clients = game.max_clients();
    let previous_pause = simulation.q1_paused();
    Ok(Q1ServerHost::Native(Q1NativeHost {
        session: options.session,
        rejects: options.rejects,
        simulation,
        game,
        protocol: options.protocol,
        wide: options.protocol.version != 15,
        max_clients,
        models,
        sounds,
        sound_resources,
        clients: HashMap::new(),
        muzzle_flashes: std::collections::HashSet::new(),
        routed: Vec::new(),
        board: HashMap::new(),
        previous_pause,
        operator_messages: Vec::new(),
        print: options.print,
    }))
}

impl<S, E, G> Q1NativeHost<S, E, G>
where
    S: Q1NativeHostSimulation,
    E: Q1QuakeCHostSession,
    G: Q1NativeGame,
{
    /// Precached index or zero for the empty path.
    fn index(&self, path: &str, table: &HashMap<String, i32>) -> Result<i32, Q1ServerError> {
        if path.is_empty() {
            return Ok(0);
        }
        table
            .get(path)
            .copied()
            .ok_or_else(|| Q1ServerError::Unprecached(path.to_string()))
    }

    /// Source entity number for an actor.
    fn number(&self, actor: &ActorId) -> Result<i32, Q1ServerError> {
        self.simulation.source_slot(actor).ok_or(Q1ServerError::MissingAddress)
    }

    /// Encoded visual bytes.
    fn visual(&self, alpha: f64, scale: f64) -> (u8, u8) {
        if self.wide {
            (entalpha_encode(alpha), (entscale_encode(scale) as i32 & 255) as u8)
        } else {
            (0, 16)
        }
    }

    /// Full entity states for the frame.
    fn entity_states(&mut self) -> Result<Vec<Q1ExtendedEntityState>, Q1ServerError> {
        let presentations: HashMap<ActorId, Q1NetworkPresentation> = self
            .simulation
            .network_presentations()
            .into_iter()
            .filter(|value| !value.view_weapon)
            .map(|value| (value.actor.clone(), value))
            .collect();
        let mut states = Vec::new();
        for entity in self.game.network_entities() {
            let actor = entity.actor.id().clone();
            let body = self.simulation.read_body(&actor);
            let presentation = presentations.get(&actor);
            let path = presentation
                .and_then(|presentation| presentation.path.clone())
                .unwrap_or_else(|| entity.model.clone());
            let number = self.number(&actor)?;
            let Some(body) = body else { continue };
            if path.is_empty() || number == 0 {
                continue;
            }
            let player = self.game.network_player(&actor).is_some();
            let (alpha, scale) = self.visual(entity.alpha, entity.scale);
            let frame = presentation.and_then(|value| value.frame).unwrap_or(entity.frame) as i32;
            states.push(Q1ExtendedEntityState {
                number,
                origin: body.origin,
                angles: body.angles,
                model_index: self.index(&path, &self.models)?,
                frame,
                color_map: if player { number } else { 0 },
                skin: presentation.and_then(|value| value.skin).unwrap_or(entity.skin) as i32,
                effects: presentation.and_then(|value| value.effects).unwrap_or(entity.effects)
                    | (i32::from(self.muzzle_flashes.contains(&actor)) * 2),
                alpha,
                scale,
                lerp_finish_seconds: 0.0,
                step: entity.movement == "step",
            });
        }
        for actor in self.simulation.players() {
            let body = self.simulation.read_body(&actor);
            let presentation = presentations.get(&actor);
            let Some(body) = body else { continue };
            let native = self.game.network_player(&actor);
            let (alpha, scale) = self.visual(
                native.as_ref().map_or(0.0, |player| player.alpha),
                native.as_ref().map_or(0.0, |player| player.scale),
            );
            states.push(Q1ExtendedEntityState {
                number: self.number(&actor)?,
                origin: body.origin,
                angles: body.angles,
                model_index: self.index(
                    &presentation
                        .and_then(|value| value.path.clone())
                        .unwrap_or_else(|| "progs/player.mdl".to_string()),
                    &self.models,
                )?,
                frame: presentation.and_then(|value| value.frame).unwrap_or(0.0) as i32,
                color_map: self.number(&actor)?,
                skin: presentation.and_then(|value| value.skin).unwrap_or(0.0) as i32,
                effects: presentation.and_then(|value| value.effects).unwrap_or(0)
                    | (i32::from(self.muzzle_flashes.contains(&actor)) * 2),
                alpha,
                scale,
                lerp_finish_seconds: 0.0,
                step: false,
            });
        }
        states.sort_by_key(|state| state.number);
        Ok(states)
    }

    /// Baseline for an entity state.
    fn baseline(&self, state: &Q1ExtendedEntityState) -> Result<Q1ExtendedEntityState, Q1ServerError> {
        let player = state.number > 0 && state.number <= self.max_clients as i32;
        let mut model_index = if player {
            self.index("progs/player.mdl", &self.models)?
        } else {
            state.model_index
        };
        let mut frame = state.frame;
        if !self.wide {
            if model_index & 0xff00 != 0 {
                model_index = 0;
            }
            if frame & 0xff00 != 0 {
                frame = 0;
            }
        }
        Ok(Q1ExtendedEntityState {
            number: state.number,
            origin: state.origin,
            angles: state.angles,
            model_index,
            frame,
            color_map: if player { state.number } else { 0 },
            skin: state.skin,
            effects: 0,
            alpha: if player { 0 } else { state.alpha },
            scale: if player || self.protocol.version != 999 {
                16
            } else {
                state.scale
            },
            lerp_finish_seconds: 0.0,
            step: false,
        })
    }
}

impl<S, E, G> Q1NativeHost<S, E, G>
where
    S: Q1NativeHostSimulation,
    E: Q1QuakeCHostSession,
    G: Q1NativeGame,
{
    /// Client data message for a player.
    fn client_data(&self, player: &Q1ApplicationPlayer) -> Result<Q1ApplicationMessage, Q1ServerError> {
        let movement = self.simulation.network_movement(&player.actor);
        let native = self.game.network_player(&player.actor);
        let ui = self.simulation.network_player_ui(&player.actor);
        let (Some(movement), Some(native)) = (movement, native) else {
            return Err(Q1ServerError::MissingGame);
        };
        let state = &movement.state;
        let mut items = 0;
        let weapon_bits = [4096, 1, 2, 4, 8, 16, 32, 64];
        for (ordinal, weapon) in WEAPONS.iter().enumerate() {
            if self
                .simulation
                .inventory_count(&player.actor, &self.game.weapon_item(weapon.as_str()))
                > 0.0
            {
                items |= weapon_bits.get(ordinal).copied().unwrap_or(0);
            }
        }
        if !ui.armor.none {
            items |= if ui.armor.q1 && ui.armor.absorption >= 0.8 {
                32768
            } else if ui.armor.q1 && ui.armor.absorption >= 0.6 {
                16384
            } else {
                8192
            };
        }
        let time = self.game.game_time();
        for (powerup, expires) in &native.powerups {
            if *expires > time {
                items |= match powerup.as_str() {
                    "quad" => 4194304,
                    "invulnerability" => 1048576,
                    "invisibility" => 524288,
                    "suit" => 2097152,
                    _ => 0,
                };
            }
        }
        Ok(Q1ApplicationMessage::ClientData {
            weapon_alpha: if self.wide { entalpha_encode(native.alpha) } else { 0 },
            data: Q1ClientData {
                view_height: movement.view_height,
                ideal_pitch: state.ideal_pitch,
                punch_angles: state.punch_angles,
                velocity: state.velocity,
                items,
                on_ground: state.flags & 512 != 0,
                in_water: state.water_level >= 2,
                weapon_frame: native.weapon_frame,
                armor: if ui.armor.none { 0.0 } else { ui.armor.points },
                weapon_model: self.index(&self.game.weapon_model(&native.weapon, &native), &self.models)?,
                health: ui.health,
                ammo: ui.ammo,
                shells: self.simulation.inventory_count(&player.actor, "q1:ammo/shells"),
                nails: self.simulation.inventory_count(&player.actor, "q1:ammo/nails"),
                rockets: self.simulation.inventory_count(&player.actor, "q1:ammo/rockets"),
                cells: self.simulation.inventory_count(&player.actor, "q1:ammo/cells"),
                active_weapon: WEAPONS
                    .iter()
                    .position(|weapon| weapon.as_str() == native.weapon)
                    .and_then(|ordinal| weapon_bits.get(ordinal).copied())
                    .unwrap_or(0),
            },
        })
    }

    /// Persistent signon messages.
    fn persistent_signon(&self) -> Result<Vec<Q1ApplicationMessage>, Q1ServerError> {
        let mut out = Vec::new();
        for record in self.simulation.capture_persistent() {
            let SourcePresentationEvent::Q1(event) = &record.event else {
                continue;
            };
            match event {
                qa_content::q1::foundation::types::Q1Event::Ambient {
                    path,
                    volume,
                    attenuation,
                    origin,
                    ..
                } => {
                    out.push(Q1ApplicationMessage::Sound {
                        kind: super::network_q1_quakec::Q1SoundKind::StaticSound,
                        entity: 0,
                        channel: 0,
                        index: self.index(path, &self.sounds)?,
                        volume: (*volume * 255.0) as i32,
                        attenuation: *attenuation,
                        origin: *origin,
                    });
                }
                qa_content::q1::foundation::types::Q1Event::StaticModel {
                    path,
                    frame,
                    color_map,
                    skin,
                    origin,
                    angles,
                    ..
                } => {
                    out.push(Q1ApplicationMessage::Static {
                        state: Q1ExtendedEntityState {
                            number: 0,
                            origin: *origin,
                            angles: *angles,
                            model_index: self.index(path, &self.models)?,
                            frame: *frame,
                            color_map: *color_map,
                            skin: *skin,
                            effects: 0,
                            alpha: 0,
                            scale: 16,
                            lerp_finish_seconds: 0.0,
                            step: false,
                        },
                    });
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Observe a simulation step.
    pub fn observe(
        &mut self,
        output: &SimulationOutput,
        events: &[SimulationPresentationEvent],
    ) -> Result<(), Q1ServerError> {
        self.routed = std::mem::take(&mut self.operator_messages);
        if self.previous_pause != self.simulation.q1_paused() {
            self.previous_pause = self.simulation.q1_paused();
            self.routed.push(RoutedMessage {
                recipient: None,
                reliable: true,
                message: Q1ApplicationMessage::Pause {
                    paused: self.previous_pause,
                },
            });
        }
        self.muzzle_flashes.clear();
        let mut live_slots = std::collections::HashSet::new();
        for client in self.game.composition_clients() {
            live_slots.insert(client.slot);
            let previous = self.board.get(&client.slot);
            let colors = client.shirt * 16 + client.pants;
            if previous.is_none_or(|previous| previous.name != client.name) {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::NamedSlot {
                        kind: Q1NamedSlotKind::Name,
                        slot: client.slot as i32,
                        value: client.name.clone(),
                    },
                });
            }
            if previous.is_none_or(|previous| previous.colors != colors) {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::NumberedSlot {
                        kind: Q1NumberedSlotKind::Colors,
                        slot: client.slot as i32,
                        value: colors as f64,
                    },
                });
            }
            if previous.is_none_or(|previous| previous.frags != client.frags) {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::NumberedSlot {
                        kind: Q1NumberedSlotKind::Frags,
                        slot: client.slot as i32,
                        value: client.frags,
                    },
                });
            }
            self.board.insert(
                client.slot,
                BoardEntry {
                    name: client.name,
                    colors,
                    frags: client.frags,
                },
            );
        }
        let dead: Vec<u32> = self
            .board
            .keys()
            .copied()
            .filter(|slot| !live_slots.contains(slot))
            .collect();
        for slot in dead {
            self.routed.push(RoutedMessage {
                recipient: None,
                reliable: true,
                message: Q1ApplicationMessage::NamedSlot {
                    kind: Q1NamedSlotKind::Name,
                    slot: slot as i32,
                    value: String::new(),
                },
            });
            self.routed.push(RoutedMessage {
                recipient: None,
                reliable: true,
                message: Q1ApplicationMessage::NumberedSlot {
                    kind: Q1NumberedSlotKind::Colors,
                    slot: slot as i32,
                    value: 0.0,
                },
            });
            self.routed.push(RoutedMessage {
                recipient: None,
                reliable: true,
                message: Q1ApplicationMessage::NumberedSlot {
                    kind: Q1NumberedSlotKind::Frags,
                    slot: slot as i32,
                    value: 0.0,
                },
            });
            self.board.remove(&slot);
        }
        let recipe_content = self.simulation.recipe().map_entities_content.clone();
        for record in events {
            match &record.event {
                SourcePresentationEvent::ViewReset { actor, angles, .. } => {
                    self.routed.push(RoutedMessage {
                        recipient: Some(actor.clone()),
                        reliable: true,
                        message: Q1ApplicationMessage::SetAngle { angles: *angles },
                    });
                }
                SourcePresentationEvent::Q1(event) => {
                    self.observe_q1(record, event, &recipe_content, output)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

use qa_content::q1::foundation::types::{Q1BeamStyle, Q1Effect, Q1Event, Q1SoundChannel};

impl<S, E, G> Q1NativeHost<S, E, G>
where
    S: Q1NativeHostSimulation,
    E: Q1QuakeCHostSession,
    G: Q1NativeGame,
{
    /// Route one Q1 presentation event.
    fn observe_q1(
        &mut self,
        record: &SimulationPresentationEvent,
        event: &Q1Event,
        recipe_content: &ContentId,
        _output: &SimulationOutput,
    ) -> Result<(), Q1ServerError> {
        match event {
            Q1Event::Sound {
                actor,
                path,
                channel,
                attenuation,
                volume,
                origin,
            } => {
                if &record.content != recipe_content {
                    panic!("Native Q1 sound belongs to another content provider");
                }
                let Some(index) = self.sounds.get(path).copied() else {
                    (self.print)(&format!("SV_StartSound: {path} not precached\n"));
                    return Ok(());
                };
                let channel = match channel {
                    Q1SoundChannel::Auto => 0,
                    Q1SoundChannel::Weapon => 1,
                    Q1SoundChannel::Voice => 2,
                    Q1SoundChannel::Item => 3,
                    Q1SoundChannel::Body => 4,
                    Q1SoundChannel::Raw(value) => *value,
                };
                let source_entity = record.source_entity.or_else(|| self.simulation.source_slot(actor));
                let (Some(source_entity), Some(origin)) = (source_entity, origin) else {
                    (self.print)(&format!("Q1 sound has no emission-time origin: {path}\n"));
                    return Ok(());
                };
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: false,
                    message: Q1ApplicationMessage::Sound {
                        kind: super::network_q1_quakec::Q1SoundKind::Sound,
                        entity: source_entity,
                        channel,
                        index,
                        volume: (*volume * 255.0) as i32,
                        attenuation: *attenuation,
                        origin: *origin,
                    },
                });
            }
            Q1Event::Ambient { .. } => {}
            Q1Event::Message {
                player, text, center, ..
            } => {
                self.routed.push(RoutedMessage {
                    recipient: Some(player.clone()),
                    reliable: true,
                    message: Q1ApplicationMessage::Text {
                        kind: if *center {
                            super::network_q1_quakec::Q1TextMessageKind::CenterPrint
                        } else {
                            super::network_q1_quakec::Q1TextMessageKind::Print
                        },
                        text: text.clone(),
                    },
                });
            }
            Q1Event::Lightstyle { style, pattern } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::LightStyle {
                        index: *style,
                        value: pattern.clone(),
                    },
                });
            }
            Q1Event::Particles {
                origin,
                direction,
                color,
                count,
                ..
            } => {
                let clamp = |value: f32| (value * 16.0).trunc().clamp(-128.0, 127.0) / 16.0;
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: false,
                    message: Q1ApplicationMessage::Particle {
                        origin: *origin,
                        direction: Vec3 {
                            x: clamp(direction.x),
                            y: clamp(direction.y),
                            z: clamp(direction.z),
                        },
                        count: *count,
                        color: *color,
                    },
                });
            }
            Q1Event::ColoredExplosion {
                origin,
                color_start,
                color_length,
                ..
            } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: false,
                    message: Q1ApplicationMessage::TemporaryEntity {
                        effect: super::network_q1_quakec::Q1TemporaryEntity::ExplosionColors {
                            effect_type: 12,
                            origin: *origin,
                            color_start: *color_start,
                            color_length: *color_length,
                        },
                    },
                });
            }
            Q1Event::Beam {
                style,
                actor,
                start,
                end,
                ..
            } => {
                let effect_type = match style {
                    Q1BeamStyle::Lightning1 => 5,
                    Q1BeamStyle::Lightning2 => 6,
                    Q1BeamStyle::Lightning3 => 9,
                    Q1BeamStyle::Grapple => 13,
                };
                let entity = record.source_entity.unwrap_or_else(|| self.number(actor).unwrap_or(0));
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: false,
                    message: Q1ApplicationMessage::TemporaryEntity {
                        effect: super::network_q1_quakec::Q1TemporaryEntity::Beam {
                            effect_type,
                            entity,
                            start: *start,
                            end: *end,
                        },
                    },
                });
            }
            Q1Event::Effect {
                effect,
                actor,
                origin,
                amount,
                ..
            } => match effect {
                Q1Effect::Muzzleflash => {
                    if let Some(actor) = actor {
                        self.muzzle_flashes.insert(actor.clone());
                    }
                }
                Q1Effect::Pickup => {
                    if let Some(actor) = actor {
                        self.routed.push(RoutedMessage {
                            recipient: Some(actor.clone()),
                            reliable: true,
                            message: Q1ApplicationMessage::Text {
                                kind: super::network_q1_quakec::Q1TextMessageKind::Stufftext,
                                text: "bf\n".to_string(),
                            },
                        });
                    }
                }
                Q1Effect::Blood => {
                    self.routed.push(RoutedMessage {
                        recipient: None,
                        reliable: false,
                        message: Q1ApplicationMessage::Particle {
                            origin: *origin,
                            direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                            count: amount * 2,
                            color: 73,
                        },
                    });
                }
                Q1Effect::MeatSpray => {
                    (self.print)("Q1 meat spray requires a source entity; no temporary-entity substitute emitted\n");
                }
                other => {
                    let effect_type = match other {
                        Q1Effect::Gunshot => 2,
                        Q1Effect::Spike => 0,
                        Q1Effect::Superspike => 1,
                        Q1Effect::Explosion => 3,
                        Q1Effect::Teleport => 11,
                        Q1Effect::LavaSplash => 10,
                        Q1Effect::TarExplosion => 4,
                        Q1Effect::WizardSpike => 7,
                        Q1Effect::KnightSpike => 8,
                        _ => return Ok(()),
                    };
                    self.routed.push(RoutedMessage {
                        recipient: None,
                        reliable: false,
                        message: Q1ApplicationMessage::TemporaryEntity {
                            effect: super::network_q1_quakec::Q1TemporaryEntity::Point {
                                effect_type,
                                origin: *origin,
                                count: 1,
                            },
                        },
                    });
                }
            },
            Q1Event::TeleportPlayer { player, angles, .. } => {
                self.routed.push(RoutedMessage {
                    recipient: Some(player.clone()),
                    reliable: true,
                    message: Q1ApplicationMessage::SetAngle { angles: *angles },
                });
            }
            Q1Event::Secret { .. } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::Unit(super::network_q1_quakec::Q1UnitMessageKind::FoundSecret),
                });
            }
            Q1Event::MonsterKilled { .. } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::Unit(super::network_q1_quakec::Q1UnitMessageKind::KilledMonster),
                });
            }
            Q1Event::MonsterTotal { total, .. } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::Stat {
                        index: 12,
                        value: *total as f64,
                    },
                });
            }
            Q1Event::Intermission { .. } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::Unit(super::network_q1_quakec::Q1UnitMessageKind::Intermission),
                });
            }
            Q1Event::Finale { text, .. } => {
                self.routed.push(RoutedMessage {
                    recipient: None,
                    reliable: true,
                    message: Q1ApplicationMessage::Text {
                        kind: super::network_q1_quakec::Q1TextMessageKind::Finale,
                        text: text.clone(),
                    },
                });
            }
            Q1Event::Camera {
                player,
                origin: _,
                angles,
                ..
            } => {
                self.routed.push(RoutedMessage {
                    recipient: Some(player.clone()),
                    reliable: true,
                    message: Q1ApplicationMessage::SetAngle { angles: *angles },
                });
            }
            _ => {}
        }
        Ok(())
    }
}

/// Wire support verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q1WireSupport {
    /// Native wire supported.
    Supported,
    /// Native wire unsupported.
    Unsupported {
        /// Reasons.
        reasons: Vec<String>,
    },
}

impl<S, E, G> Q1NativeHost<S, E, G>
where
    S: Q1NativeHostSimulation,
    E: Q1QuakeCHostSession,
    G: Q1NativeGame,
{
    /// Address rejection predicate for the transport.
    #[must_use]
    pub fn rejects(&self) -> Option<Q1RejectsFn> {
        self.rejects.clone()
    }

    /// Sound resource id for a precached path.
    #[must_use]
    pub fn sound_resource(&self, path: &str) -> Option<qa_content::contract::ResourceId> {
        self.sound_resources.get(path).cloned()
    }

    /// Native wire support verdict.
    pub fn supports_source_wire(&self) -> Q1WireSupport {
        let mut reasons = Vec::new();
        if self.game.program() != "id1" {
            reasons.push("Native NetQuake application item serialization currently binds id1".to_string());
        }
        let recipe = self.simulation.recipe();
        if !recipe.movement.starts_with("q1:")
            || !recipe.character_definition.starts_with("q1:")
            || !recipe.inventory.starts_with("q1:")
            || recipe.weapons.iter().any(|value| !value.starts_with("q1:"))
        {
            reasons.push("Mixed composition requires unified serialization".to_string());
        }
        if reasons.is_empty() {
            Q1WireSupport::Supported
        } else {
            Q1WireSupport::Unsupported { reasons }
        }
    }

    /// Admit a client.
    pub fn admit(&mut self, from: &NetworkAddress) -> Result<Q1ApplicationAdmission, Q1ServerError> {
        let mut slot = 0;
        for candidate in 0..self.max_clients {
            let taken = self.clients.contains_key(&candidate)
                || self.simulation.players().iter().any(|actor| {
                    self.simulation
                        .movement_client(actor)
                        .is_some_and(|client| client.slot() == candidate)
                });
            if !taken {
                slot = candidate;
                break;
            }
            slot = self.max_clients;
        }
        if slot >= self.max_clients {
            return Ok(Q1ApplicationAdmission::Rejected {
                reason: "Server is full".to_string(),
            });
        }
        let client = self
            .session
            .create_client(slot)
            .map_err(|error| Q1ServerError::Content(error.to_string()))?;
        self.session.connect_client(
            &client,
            if matches!(from, NetworkAddress::Loopback { .. }) {
                super::network_q1_quakec::NetClientOrigin::Loopback
            } else {
                super::network_q1_quakec::NetClientOrigin::Remote
            },
        );
        let admitted = self.simulation.admit_player(&client);
        let number = self.number(&admitted.actor);
        let Ok(number) = number else {
            self.simulation.disconnect_player(&admitted.actor);
            let _ = self.session.close_client(&client);
            return Err(Q1ServerError::MissingAddress);
        };
        let player = Q1ApplicationPlayer {
            client,
            actor: admitted.actor,
            source_entity: number,
        };
        self.clients.insert(slot, player.clone());
        Ok(Q1ApplicationAdmission::Accepted { player })
    }

    /// Carry an admitted player.
    pub fn carried_player(&mut self, client: &ClientId) -> Result<Q1ApplicationPlayer, Q1ServerError> {
        let actor = self
            .simulation
            .players()
            .into_iter()
            .find(|actor| self.simulation.movement_client(actor).as_ref() == Some(client))
            .ok_or(Q1ServerError::MissingAddress)?;
        let player = Q1ApplicationPlayer {
            client: client.clone(),
            actor: actor.clone(),
            source_entity: self.number(&actor)?,
        };
        self.clients.insert(client.slot(), player.clone());
        Ok(player)
    }

    /// Disconnect a player.
    pub fn disconnect(&mut self, player: &Q1ApplicationPlayer) {
        self.simulation.disconnect_player(&player.actor);
        let _ = self.session.close_client(&player.client);
        self.clients.remove(&player.client.slot());
    }

    /// Game state for signon.
    pub fn game_state(&mut self, player: &Q1ApplicationPlayer) -> Result<Q1ApplicationGameState, Q1ServerError> {
        let states = self.entity_states()?;
        self.client_data(player)?;
        let mut baselines = HashMap::new();
        for state in &states {
            baselines.insert(state.number, self.baseline(state)?);
        }
        Ok(Q1ApplicationGameState {
            info: super::network_q1_quakec::Q1ServerInfo {
                protocol: self.protocol.profile(),
                max_clients: self.max_clients,
                game_type: if self.game.deathmatch() == 0 { 0 } else { 1 },
                level: self.game.world_message().unwrap_or_else(|| self.game.map_name()),
                models: self.models.keys().cloned().collect(),
                sounds: self.sounds.keys().cloned().collect(),
            },
            baselines,
            signon: self.persistent_signon()?,
        })
    }

    /// Spawn messages for a player.
    pub fn spawn(&mut self, player: &Q1ApplicationPlayer) -> Result<Vec<Q1ApplicationMessage>, Q1ServerError> {
        let mut messages = vec![
            Q1ApplicationMessage::Pause {
                paused: self.simulation.q1_paused(),
            },
            Q1ApplicationMessage::Time {
                seconds: self.game.game_time(),
            },
        ];
        for index in 0..64 {
            messages.push(Q1ApplicationMessage::LightStyle {
                index,
                value: self.simulation.light_style(index as u32),
            });
        }
        for client in self.game.composition_clients() {
            messages.push(Q1ApplicationMessage::NamedSlot {
                kind: Q1NamedSlotKind::Name,
                slot: client.slot as i32,
                value: client.name,
            });
            messages.push(Q1ApplicationMessage::NumberedSlot {
                kind: Q1NumberedSlotKind::Colors,
                slot: client.slot as i32,
                value: (client.shirt * 16 + client.pants) as f64,
            });
            messages.push(Q1ApplicationMessage::NumberedSlot {
                kind: Q1NumberedSlotKind::Frags,
                slot: client.slot as i32,
                value: client.frags,
            });
        }
        messages.push(Q1ApplicationMessage::Stat {
            index: 11,
            value: self.game.total_secrets() as f64,
        });
        messages.push(Q1ApplicationMessage::Stat {
            index: 12,
            value: self.game.total_monsters() as f64,
        });
        messages.push(Q1ApplicationMessage::Stat {
            index: 13,
            value: self.game.found_secrets() as f64,
        });
        messages.push(Q1ApplicationMessage::Stat {
            index: 14,
            value: self.game.killed_monsters() as f64,
        });
        messages.push(Q1ApplicationMessage::SetAngle {
            angles: self.simulation.player_view(&player.actor).angles,
        });
        messages.push(self.client_data(player)?);
        Ok(messages)
    }

    /// Frame for a player.
    pub fn frame(&mut self, player: &Q1ApplicationPlayer) -> Result<Q1ApplicationFrame, Q1ServerError> {
        let origin = self.simulation.player_view(&player.actor).origin;
        let states = self.entity_states()?;
        let scene = self.simulation.scene();
        let cluster = scene.leaf_cluster(scene.point_leaf(&origin));
        let mut entities = Vec::new();
        for state in states {
            if self.wide && state.alpha == ENTALPHA_ZERO && state.effects == 0 {
                continue;
            }
            if state.number == player.source_entity {
                entities.push(state);
                continue;
            }
            let target = scene.leaf_cluster(scene.point_leaf(&state.origin));
            if scene.cluster_visible_pvs(cluster, target) {
                entities.push(state);
            }
        }
        let reliable = self
            .routed
            .iter()
            .filter(|event| event.reliable && event.recipient.as_ref().is_none_or(|actor| actor == &player.actor))
            .map(|event| event.message.clone())
            .collect();
        let datagram = self
            .routed
            .iter()
            .filter(|event| !event.reliable && event.recipient.as_ref().is_none_or(|actor| actor == &player.actor))
            .map(|event| event.message.clone())
            .collect();
        Ok(Q1ApplicationFrame {
            seconds: self.game.game_time(),
            messages: vec![self.client_data(player)?],
            reliable,
            datagram,
            entities,
        })
    }

    /// Convert a client command to an actor command.
    pub fn input(&self, player: &Q1ApplicationPlayer, command: Q1UserCommand, sequence: u64) -> ActorCommand {
        ActorCommand {
            actor: player.actor.clone(),
            source: CommandSource::Remote {
                client: player.client.clone(),
            },
            sequence,
            command: qa_net::common::commands::UserCommand::Q1Netquake {
                acknowledged_server_time_seconds: command.acknowledged_server_time_seconds,
                view_angles: [
                    command.view_angles.x as f64,
                    command.view_angles.y as f64,
                    command.view_angles.z as f64,
                ],
                forward_move: command.forward_move,
                side_move: command.side_move,
                up_move: command.up_move,
                buttons: command.buttons as f64,
                impulse: command.impulse as f64,
            },
            arsenal: None,
        }
    }

    /// Run a client command.
    pub fn command(&mut self, player: &Q1ApplicationPlayer, name: &str, args: &[String]) {
        if name == "pause" {
            let previous = self.simulation.q1_paused();
            let text = self.simulation.toggle_q1_pause(&player.actor);
            self.operator_messages.push(RoutedMessage {
                recipient: if previous == self.simulation.q1_paused() {
                    Some(player.actor.clone())
                } else {
                    None
                },
                reliable: true,
                message: Q1ApplicationMessage::Text {
                    kind: super::network_q1_quakec::Q1TextMessageKind::Print,
                    text,
                },
            });
        } else if name == "name" {
            let mut values: HashMap<String, String> = self.game.require_client(&player.actor).into_iter().collect();
            let name_value: String = args.first().cloned().unwrap_or_else(|| "unconnected".to_string());
            values.insert("name".to_string(), name_value.chars().take(15).collect());
            self.game
                .set_client_userinfo(&player.actor, values.into_iter().collect());
            self.simulation
                .notify_client_event(ModClientEventKind::Userinfo, &player.actor);
        } else if name == "color" {
            let shirt: i32 = args.first().and_then(|value| value.parse().ok()).unwrap_or(0);
            let pants: i32 = args
                .get(1)
                .or_else(|| args.first())
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            self.game.set_client_colors(&player.actor, shirt, pants);
            self.simulation
                .notify_client_event(ModClientEventKind::Userinfo, &player.actor);
        } else if matches!(
            name,
            "use" | "weapnext" | "weapprev" | "give" | "god" | "notarget" | "noclip" | "fly" | "kill"
        ) {
            self.simulation.player_command(&player.actor, name, args);
        } else {
            (self.print)(&format!("Unsupported native Q1 client command: {name}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use qa_content::contract::{
        ContentId, ContentMount, LooseMount, MountId, MountIdentity, MountPlanId, ResolvedMountPlan,
    };
    use qa_content::mounts::{MountedContent, OpenMountOptions};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;
    use qa_net::common::endpoint::NetworkAddress;
    use qa_world::WorldError;
    use std::rc::Rc;

    use super::super::network_q1_quakec::Q1HostRecipe;
    use super::super::types::PlayerView;

    struct StubScene;

    impl super::super::network_q1_quakec::Q1HostScene for StubScene {
        fn point_leaf(&self, _point: &Vec3) -> i32 {
            0
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            0
        }

        fn cluster_visible_pvs(&self, _from: i32, _to: i32) -> bool {
            true
        }
    }

    struct StubSim {
        actor: ActorId,
        client: ClientId,
        paused: bool,
    }

    impl Q1QuakeCHostSimulation for StubSim {
        fn recipe(&self) -> Q1HostRecipe {
            Q1HostRecipe {
                movement: "q1:netquake".to_string(),
                character_definition: "q1:player".to_string(),
                inventory: "q1:items".to_string(),
                weapons: vec!["q1:weapons".to_string()],
                map_entities_content: ContentId("test:content".to_string()),
            }
        }

        fn scene(&self) -> &dyn super::super::network_q1_quakec::Q1HostScene {
            &StubScene
        }

        fn register_resource(
            &mut self,
            _content: &ContentId,
            _path: &str,
            _resource: &qa_content::contract::ResolvedResourceReference,
        ) {
        }

        fn q1_paused(&self) -> bool {
            self.paused
        }

        fn players(&self) -> Vec<ActorId> {
            vec![self.actor.clone()]
        }

        fn reserve_netquake_client(&mut self, _client: &ClientId) -> Result<ActorId, String> {
            Ok(self.actor.clone())
        }

        fn disconnect_player(&mut self, _actor: &ActorId) {}

        fn admit_player(&mut self, _client: &ClientId) -> super::super::types::PlayerAdmission {
            super::super::types::PlayerAdmission {
                actor: self.actor.clone(),
                view_height: 22.0,
            }
        }

        fn capture_persistent(&self) -> Vec<SimulationPresentationEvent> {
            Vec::new()
        }

        fn light_style(&self, _index: u32) -> String {
            "m".to_string()
        }

        fn toggle_q1_pause(&mut self, _actor: &ActorId) -> String {
            self.paused = !self.paused;
            "paused".to_string()
        }

        fn notify_client_event(&mut self, _kind: ModClientEventKind, _actor: &ActorId) {}

        fn player_view(&self, _actor: &ActorId) -> PlayerView {
            PlayerView {
                client_view_offset_delta: None,
                blend: None,
                damage_blend: None,
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
                view_height: 22.0,
                kick_angles: None,
                field_of_view: None,
                foreign_character_death: false,
                pitch_drift: None,
            }
        }

        fn player_command(&mut self, _actor: &ActorId, _name: &str, _args: &[String]) {}
    }

    impl Q1NativeHostSimulation for StubSim {
        fn source_slot(&self, _actor: &ActorId) -> Option<i32> {
            Some(1)
        }

        fn read_body(&self, _actor: &ActorId) -> Option<Q1NetworkBody> {
            Some(Q1NetworkBody {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 0.0, 0.0),
            })
        }

        fn network_presentations(&self) -> Vec<Q1NetworkPresentation> {
            Vec::new()
        }

        fn network_movement(&self, _actor: &ActorId) -> Option<Q1NetworkMovement> {
            Some(Q1NetworkMovement {
                state: qa_world::movement::q1::types::Q1MovementState {
                    origin: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    angles: vec3(0.0, 0.0, 0.0),
                    old_origin: vec3(0.0, 0.0, 0.0),
                    angular_velocity: vec3(0.0, 0.0, 0.0),
                    view_angles: vec3(0.0, 0.0, 0.0),
                    punch_angles: vec3(0.0, 0.0, 0.0),
                    move_type: 3,
                    flags: 512,
                    ground: qa_world::movement::types::TraceHit::None,
                    water_level: 0,
                    water_type: -1,
                    teleport_time_seconds: 0.0,
                    water_jump_direction: vec3(0.0, 0.0, 0.0),
                    ideal_pitch: 0.0,
                    fix_angle: false,
                    health: 100.0,
                },
                view_height: 22.0,
            })
        }

        fn network_player_ui(&self, _actor: &ActorId) -> Q1NetworkUi {
            Q1NetworkUi {
                health: 100.0,
                ammo: 0.0,
                armor: Q1NetworkArmor {
                    none: true,
                    q1: false,
                    absorption: 0.0,
                    points: 0.0,
                },
            }
        }

        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> f64 {
            0.0
        }

        fn quakec_source_kind(&self) -> Option<QuakeCSourceKind> {
            None
        }

        fn movement_client(&self, actor: &ActorId) -> Option<ClientId> {
            if actor == &self.actor {
                Some(self.client.clone())
            } else {
                None
            }
        }
    }

    struct StubContent {
        plan: ResolvedMountPlan,
    }

    impl Q1QuakeCHostContent for StubContent {
        fn for_content(&self, _content: &ContentId) -> Result<MountedContent, qa_content::mounts::MountError> {
            qa_content::mounts::open_mount_plan(&self.plan, OpenMountOptions::default())
        }
    }

    struct StubSession {
        clients: HashMap<u32, ClientId>,
    }

    impl Q1QuakeCHostSession for StubSession {
        fn create_client(&mut self, slot: u32) -> Result<ClientId, WorldError> {
            Ok(self
                .clients
                .get(&slot)
                .cloned()
                .unwrap_or_else(|| self.clients[&0].clone()))
        }

        fn connect_client(&mut self, _client: &ClientId, _origin: super::super::network_q1_quakec::NetClientOrigin) {}

        fn close_client(&mut self, _client: &ClientId) -> Result<(), String> {
            Ok(())
        }
    }

    struct StubGame;

    impl Q1NativeGame for StubGame {
        fn max_clients(&self) -> u32 {
            4
        }

        fn deathmatch(&self) -> i32 {
            0
        }

        fn edition(&self) -> String {
            "classic".to_string()
        }

        fn program(&self) -> String {
            "id1".to_string()
        }

        fn uses_id1_precaches(&self) -> bool {
            true
        }

        fn precaches_frozen(&self) -> bool {
            true
        }

        fn precache_models(&self) -> Vec<String> {
            vec!["".to_string(), "progs/player.mdl".to_string()]
        }

        fn precache_sounds(&self) -> Vec<String> {
            vec!["".to_string()]
        }

        fn game_time(&self) -> f64 {
            1.0
        }

        fn map_name(&self) -> String {
            "start".to_string()
        }

        fn world_message(&self) -> Option<String> {
            None
        }

        fn total_secrets(&self) -> i32 {
            0
        }

        fn total_monsters(&self) -> i32 {
            0
        }

        fn found_secrets(&self) -> i32 {
            0
        }

        fn killed_monsters(&self) -> i32 {
            0
        }

        fn network_entities(&self) -> Vec<Q1NetworkEntity> {
            Vec::new()
        }

        fn network_player(&self, _actor: &ActorId) -> Option<Q1NetworkPlayerState> {
            Some(Q1NetworkPlayerState {
                powerups: Vec::new(),
                weapon: "axe".to_string(),
                weapon_frame: 0.0,
                alpha: 0.0,
                scale: 0.0,
            })
        }

        fn weapon_item(&self, weapon: &str) -> String {
            format!("q1:weapon/{weapon}")
        }

        fn weapon_model(&self, _weapon: &str, _state: &Q1NetworkPlayerState) -> String {
            String::new()
        }

        fn composition_clients(&self) -> Vec<Q1NetworkClient> {
            Vec::new()
        }

        fn require_client(&self, _actor: &ActorId) -> Vec<(String, String)> {
            Vec::new()
        }

        fn set_client_userinfo(&mut self, _actor: &ActorId, _values: Vec<(String, String)>) {}

        fn set_client_colors(&mut self, _actor: &ActorId, _shirt: i32, _pants: i32) {}
    }

    struct StubQuakeGame {
        content: ContentId,
        cvars: qa_core::cvar::CvarRegistry,
    }

    impl Q1QuakeCNetGame for StubQuakeGame {
        fn kind(&self) -> QuakeCSourceKind {
            QuakeCSourceKind::Netquake
        }

        fn max_clients(&self) -> u32 {
            4
        }

        fn map_entities_content(&self) -> &ContentId {
            &self.content
        }

        fn execution_owner_content(&self) -> &ContentId {
            &self.content
        }

        fn precache_names(&self, _kind: super::super::network_q1_quakec::QuakeCPrecacheKind) -> Vec<String> {
            Vec::new()
        }

        fn field_offset(&self, _name: &str) -> Option<i32> {
            None
        }

        fn entity_count(&self) -> i32 {
            0
        }

        fn slot_occupied(&self, _slot: i32) -> bool {
            false
        }

        fn entity_float(&self, _slot: i32, _offset: i32) -> f64 {
            0.0
        }

        fn entity_int(&self, _slot: i32, _offset: i32) -> i32 {
            0
        }

        fn entity_vector(&self, _slot: i32, _offset: i32) -> qa_core::math::Vec3 {
            vec3(0.0, 0.0, 0.0)
        }

        fn entity_set_float(&mut self, _slot: i32, _offset: i32, _value: f64) {}

        fn string(&self, _index: i32) -> String {
            String::new()
        }

        fn global_float(&self, _offset: i32) -> f64 {
            0.0
        }

        fn global_int(&self, _offset: i32) -> i32 {
            0
        }

        fn global_offset(&self, _name: &str) -> i32 {
            0
        }

        fn source_slot(&self, _actor: &ActorId) -> Option<i32> {
            None
        }

        fn time_seconds(&self) -> f64 {
            0.0
        }

        fn cvars(&self) -> &qa_core::cvar::CvarRegistry {
            &self.cvars
        }

        fn attach_netquake_wire(&mut self) {}

        fn signon_messages(&self) -> Vec<super::super::network_q1_quakec::Q1NetQuakeMessage> {
            Vec::new()
        }

        fn drain_messages(&mut self) -> Vec<super::super::network_q1_quakec::Q1RoutedBatch> {
            Vec::new()
        }

        fn consume_damage(&mut self, _actor: &ActorId) -> Option<super::super::network_q1_quakec::Q1NetDamage> {
            None
        }

        fn client_info(&self, _client: &ClientId) -> HashMap<String, String> {
            HashMap::new()
        }

        fn set_client_info(&mut self, _client: &ClientId, _info: &HashMap<String, String>) {}

        fn reserved_client(&mut self, _client: &ClientId) -> OwnedActor {
            panic!("no reserved client")
        }

        fn client_actor(&self, _client: &ClientId) -> Option<ActorId> {
            None
        }

        fn disconnect_client(&mut self, _actor: &OwnedActor) {}

        fn is_active_client(&self, _actor: &ActorId) -> bool {
            false
        }
    }

    fn fixture_dir(files: &[&str]) -> ResolvedMountPlan {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("qa-sim-net-q1n-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            let path = dir.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"stub").unwrap();
        }
        let mount = ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: MountId("mount:test:loose".to_string()),
                content: ContentId("test:content".to_string()),
                generation: 1,
            },
            root_path: dir.to_string_lossy().into_owned(),
        });
        ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:1".to_string()),
            mounts: vec![mount],
            default_order: vec![MountId("mount:test:loose".to_string())],
            prefix_orders: Vec::new(),
        }
    }

    fn host() -> Q1NativeHost<StubSim, StubSession, StubGame> {
        let owner = IdentityOwner::create("q1-native-test").unwrap();
        let actor = owner.actor(1, 1);
        let client = owner.client(0, 0);
        let mut clients = HashMap::new();
        clients.insert(0, client.clone());
        let plan = fixture_dir(&["progs/player.mdl"]);
        let created = create_q1_application_server_host(Q1ServerOptions {
            session: StubSession { clients },
            rejects: None,
            simulation: StubSim {
                actor: actor.clone(),
                client,
                paused: false,
            },
            content: StubContent { plan },
            protocol: Q1ProtocolIdentity { version: 15 },
            max_precache: 512,
            game: Some(StubGame),
            quakec_game: Some(StubQuakeGame {
                content: ContentId("test:content".to_string()),
                cvars: qa_core::cvar::CvarRegistry::new(qa_core::cmd::Dialect::Q1Netquake),
            }),
            print: Rc::new(|_| {}),
        })
        .unwrap();
        match created {
            Q1ServerHost::Native(host) => host,
            Q1ServerHost::QuakeC(_) => panic!("expected native host"),
        }
    }

    #[test]
    fn native_wire_supported_for_id1() {
        let host = host();
        assert_eq!(host.supports_source_wire(), Q1WireSupport::Supported);
    }

    #[test]
    fn admit_registers_player() {
        let mut host = host();
        let admitted = host
            .admit(&NetworkAddress::Loopback { id: "test".to_string() })
            .unwrap();
        match admitted {
            Q1ApplicationAdmission::Accepted { player } => {
                assert_eq!(player.source_entity, 1);
            }
            Q1ApplicationAdmission::Rejected { .. } => panic!("expected admission"),
        }
    }

    #[test]
    fn spawn_opens_with_pause() {
        let mut host = host();
        let admitted = host
            .admit(&NetworkAddress::Loopback { id: "test".to_string() })
            .unwrap();
        let Q1ApplicationAdmission::Accepted { player } = admitted else {
            panic!("expected admission");
        };
        let _ = host.game_state(&player);
        let messages = host.spawn(&player).unwrap();
        assert!(matches!(messages[0], Q1ApplicationMessage::Pause { paused: false }));
    }
}
