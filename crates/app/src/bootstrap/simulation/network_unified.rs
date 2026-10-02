//! Unified native application server host.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/network-unified.ts`
//! (`UnifiedApplicationPlayer`, `UnifiedApplicationServerHost`,
//! `unifiedPresentationFor`, `unifiedModelPresentations`,
//! `createUnifiedApplicationServerHost`).
//!
//! # Missing siblings
//!
//! - `simulation/runtime.ts` (`SharedSimulation`): [`UnifiedHostSimulation`]
//!   is the narrow seam.
//! - `app/bootstrap/content.ts` (`LoadedApplicationContent`) and
//!   `world/session/session.ts` (`EngineSession`): [`UnifiedHostContent`]
//!   and [`UnifiedHostSession`] are the narrow seams.
//! - `app/bootstrap/network/unified-types.ts`,
//!   `unified-components.ts`, `unified-native-components.ts`,
//!   `unified-frame-codec.ts`: [`UnifiedPresentationFrame`],
//!   [`UnifiedComponentPublication`], [`UnifiedNativePublication`],
//!   [`UnifiedResourceKey`], and [`UnifiedPrediction`] are local mirrors;
//!   unify post-merge.
//! - `app/bootstrap/component-scene.ts` (`selectComponentScene`) and
//!   `app/bootstrap/network/unified-prediction.ts`
//!   (`projectUnifiedPrediction`): selection and projection live behind
//!   the [`UnifiedModSource`] and [`UnifiedHostSimulation`] seams.
//! - `content/q2/base/player/index.ts` (`q2Userinfo`): parsed locally by
//!   [`parse_q2_userinfo`].

use std::collections::HashMap;

use qa_content::contract::{ContentId, ExecutableRecipe, PresentationOwner, ResolvedResourceReference, ResourceId};
use qa_core::identity::SavedActorId;
use qa_core::identity::{ActorId, ClientId};
use qa_core::math::Vec3;
use qa_net::common::commands::MovementDialect;
use qa_net::common::commands::{ActorCommand, UserCommand};
use qa_net::common::endpoint::NetworkAddress;
use qa_world::session::SimulationOutput;
use thiserror::Error;

use qa_content::q1::composition::types::Q1CompositionEvent;
use qa_content::q1::foundation::types::Q1Event;
use qa_content::q2::base::player::types::Q2PlayerEvent;
use qa_content::q2::composition::types::Q2CompositionEvent;
use qa_content::q2::foundation::host::Q2PresentationEvent;
use qa_content::q2::foundation::weapons::types::Q2WeaponEvent;
use qa_content::q2::rerelease::types::Q2RereleaseEvent;
use qa_content::q3::foundation::presentation::Q3CharacterView;

use super::types::{
    PlayerUi, PlayerView, SimulationPresentation, SimulationPresentationEvent, SourcePresentationEvent,
};
use qa_client::text::ui_world::WorldText;

/// Unified application player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedApplicationPlayer {
    /// Owning client.
    pub client: ClientId,
    /// Owning actor.
    pub actor: ActorId,
    /// Source entity number.
    pub source_entity: i32,
}

/// Unified admission verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedAdmission {
    /// Accepted.
    Accepted {
        /// Player.
        player: UnifiedApplicationPlayer,
    },
    /// Rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
}

/// Mirror of `UnifiedResourceKey` (canonical home: `unified-frame-codec`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UnifiedResourceKey {
    /// Content id.
    pub content: ContentId,
    /// Resource path.
    pub path: String,
    /// Resource revision.
    pub revision: String,
}

/// Mirror of `UnifiedPrediction` (canonical home: `unified-prediction`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPrediction {
    /// Acknowledged input sequence.
    pub acknowledged: i64,
}

/// Mirror of `UnifiedComponentPublication` (canonical home: `unified-components`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedComponentPublication {
    /// Owner.
    pub owner: PresentationOwner,
    /// Identity.
    pub identity: String,
    /// Generation.
    pub generation: u64,
    /// ABI profile.
    pub abi: String,
    /// Runtime name.
    pub runtime: String,
    /// Viewer.
    pub viewer: ActorId,
    /// Bindings.
    pub bindings: Vec<(String, String)>,
    /// Context handle.
    pub context: String,
}

/// Mirror of `UnifiedNativePublication` (canonical home: `unified-native-components`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedNativePublication {
    /// Owner.
    pub owner: PresentationOwner,
    /// Identity.
    pub identity: String,
    /// Generation.
    pub generation: u64,
    /// Viewer.
    pub viewer: ActorId,
    /// Frame handle.
    pub frame: String,
}

/// Mirror of `UnifiedPresentationFrame` (canonical home: `unified-types`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPresentationFrame {
    /// Epoch.
    pub epoch: u64,
    /// Acknowledged input.
    pub acknowledged_input: i64,
    /// Prediction.
    pub prediction: UnifiedPrediction,
    /// Filtered output.
    pub output: SimulationOutput,
    /// Model presentations.
    pub models: Vec<SimulationPresentation>,
    /// Character views.
    pub characters: Vec<Q3CharacterView>,
    /// World text.
    pub world_text: Vec<WorldText>,
    /// Player view.
    pub player_view: PlayerView,
    /// Player UI.
    pub player_ui: PlayerUi,
}

/// Narrow engine session seam.
pub trait UnifiedHostSession {
    /// Create a client.
    fn create_client(&mut self, slot: u32) -> Result<ClientId, String>;
    /// Connect a client.
    fn connect_client(&mut self, client: &ClientId, origin: UnifiedClientOrigin);
    /// Close a client.
    fn close_client(&mut self, client: &ClientId) -> Result<(), String>;
}

/// Client origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedClientOrigin {
    /// Loopback.
    Loopback,
    /// Remote.
    Remote,
}

/// Narrow content seam.
pub trait UnifiedHostContent {}

/// Mod presentation source view.
pub trait UnifiedModSource {
    /// Owner.
    fn owner(&self) -> PresentationOwner;
    /// Identity.
    fn identity(&self) -> String;
    /// Generation.
    fn generation(&self) -> u64;
    /// ABI profile.
    fn abi(&self) -> String;
    /// Runtime name.
    fn runtime(&self) -> String;
    /// Assert current.
    fn assert_current(&self);
    /// Whether live for a viewer.
    fn live(&self, viewer: &ActorId) -> bool;
    /// Context for a viewer, scene already selected.
    fn context(&self, viewer: &ActorId) -> Option<String>;
    /// Bindings.
    fn bindings(&self) -> Vec<(String, String)>;
    /// Client command; [`None`] when the source has no handler.
    fn client_command(&self, viewer: &ActorId, args: &[String]) -> Option<bool>;
}

/// Native mod source view.
pub trait UnifiedNativeModSource {
    /// Owner.
    fn owner(&self) -> PresentationOwner;
    /// Identity.
    fn identity(&self) -> String;
    /// Generation.
    fn generation(&self) -> u64;
    /// Assert current.
    fn assert_current(&self);
    /// Native frame for a viewer.
    fn native_frame(&self, viewer: &ActorId) -> Option<String>;
}

/// Q1 source view for unified hosting.
pub trait UnifiedQ1Source {
    /// Set composition userinfo.
    fn set_userinfo(&mut self, actor: &ActorId, values: &HashMap<String, String>);
    /// Set composition colors.
    fn set_colors(&mut self, actor: &ActorId, shirt: i32, pants: i32);
}

/// Q2 source view for unified hosting.
pub trait UnifiedQ2Source {
    /// Gate a connection; returns the rewritten userinfo.
    fn connect(&mut self, userinfo: &str) -> Result<String, String>;
    /// Set player userinfo.
    fn set_userinfo(&mut self, actor: &ActorId, userinfo: &str);
}

/// Q3 source view for unified hosting.
pub trait UnifiedQ3Source {
    /// Set server userinfo.
    fn set_userinfo(&mut self, slot: u32, userinfo: &str);
    /// Notify admission of a userinfo change.
    fn userinfo_changed(&mut self, slot: u32);
}

/// Unified simulation seam.
pub trait UnifiedHostSimulation {
    /// Executable recipe.
    fn recipe(&self) -> ExecutableRecipe;
    /// Maximum clients.
    fn max_clients(&self) -> u32;
    /// Simulation mode.
    fn mode(&self) -> UnifiedSimulationMode;
    /// Live players.
    fn players(&self) -> Vec<ActorId>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Source slot for an actor.
    fn source_slot(&self, actor: &ActorId) -> Option<i32>;
    /// Movement dialect of an actor's selected movement.
    fn movement_dialect(&self, actor: &ActorId) -> Option<MovementDialect>;
    /// Client of an actor's movement player.
    fn movement_client(&self, actor: &ActorId) -> Option<ClientId>;
    /// Admit a player.
    fn admit_player(&mut self, client: &ClientId) -> Result<UnifiedApplicationPlayer, UnifiedAdmissionError>;
    /// Disconnect a player.
    fn disconnect_player(&mut self, actor: &ActorId) -> Result<(), String>;
    /// Run a player command.
    fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]);
    /// Notify a client event.
    fn notify_client_event(&mut self, actor: &ActorId);
    /// Mod presentation sources.
    fn mod_sources(&self) -> Vec<std::rc::Rc<dyn UnifiedModSource>>;
    /// Native mod sources.
    fn native_mod_sources(&self) -> Vec<std::rc::Rc<dyn UnifiedNativeModSource>>;
    /// Presentations.
    fn presentations(&self) -> Vec<SimulationPresentation>;
    /// Character views.
    fn character_views(&self) -> Vec<Q3CharacterView>;
    /// World text.
    fn world_text(&self) -> Vec<WorldText>;
    /// Player view.
    fn player_view(&self, actor: &ActorId) -> PlayerView;
    /// Player UI.
    fn player_ui(&self, actor: &ActorId) -> PlayerUi;
    /// Unified prediction.
    fn unified_prediction(&self, actor: &ActorId, acknowledged: i64) -> UnifiedPrediction;
    /// Persistent presentation events.
    fn persistent_presentation(&self) -> Vec<SimulationPresentationEvent>;
    /// Declared content resources.
    fn declared_resources(&self) -> Vec<ResolvedResourceReference>;
    /// Event resource lookup.
    fn event_resource(&self, id: &ResourceId) -> Option<ResolvedResourceReference>;
}

/// Simulation mode mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedSimulationMode {
    /// New game.
    New,
    /// Restore.
    Restore,
}

/// Admission errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnifiedAdmissionError {
    /// Denied (mirrors `Q3ClientAdmissionDenied` until `q3/runtime` lands).
    #[error("{reason}")]
    Denied {
        /// Reason.
        reason: String,
    },
    /// Failed.
    #[error("{message}")]
    Failed {
        /// Message.
        message: String,
    },
}

/// Unified hosting errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnifiedServerError {
    /// No source game.
    #[error("Unified hosting currently requires a TypeScript Q1, Q2 or Q3 source game")]
    MissingSource,
    /// Retired player.
    #[error("Unified client belongs to a retired source player")]
    RetiredPlayer,
    /// Lost source address.
    #[error("Unified player lost its authoritative source address")]
    LostAddress,
    /// Lost Q2 entity.
    #[error("Unified Q2 player lost its source entity")]
    LostQ2Entity,
    /// Invalid sequence.
    #[error("Invalid unified input sequence")]
    InvalidSequence,
    /// Command mismatch.
    #[error("Unified command does not match selected movement")]
    CommandMismatch,
    /// Missing sound resource.
    #[error("Unified sound has no declared content resource")]
    MissingSound,
    /// Admission cleanup failed.
    #[error("Unified admission cleanup failed: {0}")]
    AdmissionCleanup(String),
    /// Disconnect failed.
    #[error("Unified player disconnect failed: {0}")]
    DisconnectFailed(String),
}

/// Audience projection for a presentation event.
///
/// Projection gaps (documented, not deferred): the Rust `Q3SourceEvent`
/// taxonomy has no console-command, drop-client, log, or server-command
/// variants, `Q3SharedBallisticEvent` has no rail-award kind, and
/// `Q2RereleaseEvent` has no autosave variant, so those donor denials
/// project to visible until the taxonomies unify.
#[must_use]
pub fn unified_presentation_for(actor: &ActorId, _client: &ClientId, value: &SimulationPresentationEvent) -> bool {
    if value.recipient.as_ref().is_some_and(|recipient| recipient != actor) {
        return false;
    }
    let own = |target: Option<&ActorId>| target.is_none_or(|target| target == actor);
    match &value.event {
        SourcePresentationEvent::OwnerRetired { .. }
        | SourcePresentationEvent::OwnerRefreshed { .. }
        | SourcePresentationEvent::DebugGraph { .. }
        | SourcePresentationEvent::Q3Character(_)
        | SourcePresentationEvent::Q1LevelCompleted
        | SourcePresentationEvent::Q1BackToLobby
        | SourcePresentationEvent::CdTrack { .. }
        | SourcePresentationEvent::MusicPause { .. }
        | SourcePresentationEvent::Q1Level(_)
        | SourcePresentationEvent::Q1Client(_)
        | SourcePresentationEvent::Q1Skybox { .. }
        | SourcePresentationEvent::Q3Source(_)
        | SourcePresentationEvent::Q3Ballistics(_) => true,
        SourcePresentationEvent::ViewReset { actor: target, .. } => target == actor,
        SourcePresentationEvent::Q1Fog { player, .. } => own(player.as_ref()),
        SourcePresentationEvent::Q1(event) => match event {
            Q1Event::ServerCommand { .. } => false,
            Q1Event::Message { player, .. }
            | Q1Event::Camera { player, .. }
            | Q1Event::TeleportPlayer { player, .. } => player == actor,
            Q1Event::Effect { actor: target, .. } => own(target.as_ref()),
            Q1Event::Sound { actor: target, .. } => target == actor,
            _ => true,
        },
        SourcePresentationEvent::Q1Composition(event) => match event {
            Q1CompositionEvent::SourceLog { .. } | Q1CompositionEvent::DeveloperMessage { .. } => false,
            Q1CompositionEvent::Addon(event) => q1_addon_for(actor, event),
            Q1CompositionEvent::CtfStatus { actor: target, .. }
            | Q1CompositionEvent::Prompt { actor: target, .. }
            | Q1CompositionEvent::ClearPrompt { actor: target, .. } => target == actor,
            _ => true,
        },
        SourcePresentationEvent::Q2(event) => match event {
            Q2PresentationEvent::Pickup { player, .. } => player == actor,
            Q2PresentationEvent::CenterPrint { actor: target, .. } => target == actor,
            Q2PresentationEvent::Print { actor: target, .. } => own(target.as_ref()),
            Q2PresentationEvent::DamageIndicator { actor: target, .. } => target == actor,
            _ => true,
        },
        SourcePresentationEvent::Q2Player(event) => match event {
            Q2PlayerEvent::StuffText { .. } | Q2PlayerEvent::LoadMenu { .. } | Q2PlayerEvent::Trail { .. } => false,
            Q2PlayerEvent::Userinfo { .. } => true,
            Q2PlayerEvent::Print { target, .. } => own(target.as_ref()),
            Q2PlayerEvent::View { actor: target, .. }
            | Q2PlayerEvent::Scoreboard { actor: target, .. }
            | Q2PlayerEvent::Inventory { actor: target, .. }
            | Q2PlayerEvent::Help { actor: target, .. }
            | Q2PlayerEvent::Chase { actor: target, .. } => target == actor,
        },
        SourcePresentationEvent::Q2Composition(event) => match event {
            Q2CompositionEvent::Kick { .. } => false,
            Q2CompositionEvent::MissionpackEntity(_) => true,
            Q2CompositionEvent::MissionpackPlayer(effect) => match effect {
                qa_content::q2::missionpacks::types::Q2MissionPackPlayerEffect::TrackerPain {
                    actor: target, ..
                } => target == actor,
                qa_content::q2::missionpacks::types::Q2MissionPackPlayerEffect::NukeBlind { actor: target, .. } => {
                    target == actor
                }
                _ => true,
            },
            Q2CompositionEvent::GrapplePrediction { actor: target, .. } => target == actor,
            _ => true,
        },
        SourcePresentationEvent::Q2Rerelease(event) => match event {
            Q2RereleaseEvent::RestartLevel { .. } => false,
            Q2RereleaseEvent::Alpha { .. }
            | Q2RereleaseEvent::DynamicLight { .. }
            | Q2RereleaseEvent::PlayerDogtag { .. }
            | Q2RereleaseEvent::Flashlight { .. } => true,
            Q2RereleaseEvent::LocalizedPrint { actor: target, .. } => own(target.as_ref()),
            Q2RereleaseEvent::MissionObjective { actor: target, .. } => target == actor,
            Q2RereleaseEvent::MissionStatus { actor: target, .. } => target == actor,
            Q2RereleaseEvent::ScreenBlend { actor: target, .. } => target == actor,
            Q2RereleaseEvent::HelpComputer { actor: target, .. } => target == actor,
            Q2RereleaseEvent::Fog { actor: target, .. } => target == actor,
            Q2RereleaseEvent::KeyedPoi { actor: target, .. } => target == actor,
            Q2RereleaseEvent::RemovePoi { actor: target, .. } => target == actor,
            Q2RereleaseEvent::DirectionalDamage { actor: target, .. } => target == actor,
            Q2RereleaseEvent::Poi { actor: target, .. } => target == actor,
            Q2RereleaseEvent::HelpPath { actor: target, .. } => target == actor,
            Q2RereleaseEvent::CoopRespawn { actor: target, .. } => target == actor,
            Q2RereleaseEvent::Healthbar { actor: target, .. } => target == actor,
            Q2RereleaseEvent::ItemVisibility { actor: target, .. } => target == actor,
            _ => true,
        },
        SourcePresentationEvent::Q2Weapon(event) => match event {
            Q2WeaponEvent::ViewWeapon { actor: target, .. } => target == actor,
            _ => true,
        },
    }
}

/// Q1 addon audience projection.
fn q1_addon_for(actor: &ActorId, event: &qa_content::q1::addons::context::Q1AddonEvent) -> bool {
    use qa_content::q1::addons::context::Q1AddonEvent as Addon;
    match event {
        Addon::RuneCollected { player, .. } | Addon::PunchAngle { player, .. } | Addon::ViewRoll { player, .. } => {
            player == actor
        }
        Addon::Fog { player, .. } => player.as_ref().is_none_or(|player| player == actor),
        Addon::Alpha { actor: target, .. } | Addon::Lightning { actor: target, .. } => target == actor,
        _ => true,
    }
}

/// Foreign view records carry held-model identity without a first-person model.
#[must_use]
pub fn unified_model_presentations(viewer: &ActorId, models: &[SimulationPresentation]) -> Vec<SimulationPresentation> {
    models
        .iter()
        .map(|source| {
            if !source.view_weapon || source.actor == *viewer {
                return source.clone();
            }
            let mut next = source.clone();
            next.frame = 0;
            next.old_frame = 0;
            next.skin = 0;
            next.effects = 0;
            next.render_flags = 0;
            next.origin = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
            next.angles = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
            next.scale = 1.0;
            next.visible = false;
            if let Some(weapon) = next.q3_weapon.as_mut() {
                weapon.torso_animation = 0;
                weapon.horizontal_speed = 0.0;
                weapon.bob_cycle = 0.0;
            }
            next
        })
        .collect()
}

/// Parse Q2 userinfo text into pairs.
#[must_use]
pub fn parse_q2_userinfo(value: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut parts = value.split('\\');
    let _ = parts.next();
    while let (Some(key), Some(val)) = (parts.next(), parts.next()) {
        out.push((key.to_string(), val.to_string()));
    }
    out
}

/// Unified application server host.
pub struct UnifiedApplicationServerHost<S, E, A, B, C> {
    /// Engine session.
    session: E,
    /// Shared simulation.
    simulation: S,
    /// Q1 source.
    q1: Option<A>,
    /// Q2 source.
    q2: Option<B>,
    /// Q3 source.
    q3: Option<C>,
    /// Admitted players by slot.
    players: HashMap<u32, UnifiedApplicationPlayer>,
    /// Userinfo by slot.
    userinfos: HashMap<u32, HashMap<String, String>>,
}

/// Host options.
pub struct UnifiedServerOptions<S, E, A, B, C> {
    /// Engine session.
    pub session: E,
    /// Shared simulation.
    pub simulation: S,
    /// Q1 source.
    pub q1: Option<A>,
    /// Q2 source.
    pub q2: Option<B>,
    /// Q3 source.
    pub q3: Option<C>,
}

/// Create the unified application server host.
pub fn create_unified_application_server_host<S, E, A, B, C>(
    options: UnifiedServerOptions<S, E, A, B, C>,
) -> Result<UnifiedApplicationServerHost<S, E, A, B, C>, UnifiedServerError>
where
    S: UnifiedHostSimulation,
    E: UnifiedHostSession,
    A: UnifiedQ1Source,
    B: UnifiedQ2Source,
    C: UnifiedQ3Source,
{
    if options.q1.is_none() && options.q2.is_none() && options.q3.is_none() {
        return Err(UnifiedServerError::MissingSource);
    }
    Ok(UnifiedApplicationServerHost {
        session: options.session,
        simulation: options.simulation,
        q1: options.q1,
        q2: options.q2,
        q3: options.q3,
        players: HashMap::new(),
        userinfos: HashMap::new(),
    })
}

impl<S, E, A, B, C> UnifiedApplicationServerHost<S, E, A, B, C>
where
    S: UnifiedHostSimulation,
    E: UnifiedHostSession,
    A: UnifiedQ1Source,
    B: UnifiedQ2Source,
    C: UnifiedQ3Source,
{
    /// Recipe.
    pub fn recipe(&self) -> ExecutableRecipe {
        self.simulation.recipe()
    }

    /// Maximum clients.
    pub fn max_clients(&self) -> u32 {
        self.simulation.max_clients()
    }

    /// Mode.
    pub fn mode(&self) -> UnifiedSimulationMode {
        self.simulation.mode()
    }

    /// Require a live player.
    fn require_player(&self, player: &UnifiedApplicationPlayer) -> Result<(), UnifiedServerError> {
        let current = self.players.get(&player.client.slot());
        if current.is_none_or(|current| {
            current.client != player.client || current.actor != player.actor || !self.simulation.is_live(&player.actor)
        }) {
            return Err(UnifiedServerError::RetiredPlayer);
        }
        Ok(())
    }

    /// Authoritative source player.
    fn source_player(
        &self,
        client: &ClientId,
        actor: &ActorId,
    ) -> Result<UnifiedApplicationPlayer, UnifiedServerError> {
        let slot = self
            .simulation
            .source_slot(actor)
            .ok_or(UnifiedServerError::LostAddress)?;
        Ok(UnifiedApplicationPlayer {
            client: client.clone(),
            actor: actor.clone(),
            source_entity: slot,
        })
    }

    /// Userinfo text.
    fn userinfo_text(values: &HashMap<String, String>) -> String {
        values
            .iter()
            .map(|(key, value)| format!("\\{key}\\{value}"))
            .collect::<Vec<_>>()
            .join("")
    }

    /// Update userinfo across sources.
    fn update(&mut self, player: &UnifiedApplicationPlayer, values: HashMap<String, String>) {
        if let Some(q1) = self.q1.as_mut() {
            q1.set_userinfo(&player.actor, &values);
        } else if let Some(q2) = self.q2.as_mut() {
            q2.set_userinfo(&player.actor, &Self::userinfo_text(&values));
        } else if let Some(q3) = self.q3.as_mut() {
            q3.set_userinfo(player.client.slot(), &Self::userinfo_text(&values));
            q3.userinfo_changed(player.client.slot());
        }
        self.userinfos.insert(player.client.slot(), values);
        self.simulation.notify_client_event(&player.actor);
    }

    /// Admit a client.
    pub fn admit(&mut self, address: &NetworkAddress, userinfo: &str) -> Result<UnifiedAdmission, UnifiedServerError> {
        let mut values: HashMap<String, String> = parse_q2_userinfo(userinfo).into_iter().collect();
        let address_text = match address {
            NetworkAddress::Loopback { .. } => "localhost".to_string(),
            NetworkAddress::Ipv4 { host, port } => format!("{}.{}.{}.{}:{port}", host[0], host[1], host[2], host[3]),
            NetworkAddress::Ipv6 { host, port } => format!("{host}:{port}"),
            NetworkAddress::Ipx { .. } => "ipx".to_string(),
        };
        values.insert("ip".to_string(), address_text);
        let mut slot = 0;
        while slot < self.simulation.max_clients()
            && self.simulation.players().iter().any(|actor| {
                self.simulation
                    .movement_client(actor)
                    .is_some_and(|client| client.slot() == slot)
            })
        {
            slot += 1;
        }
        if slot >= self.simulation.max_clients() {
            return Ok(UnifiedAdmission::Rejected {
                reason: "Server is full".to_string(),
            });
        }
        if let Some(q2) = self.q2.as_mut() {
            match q2.connect(&Self::userinfo_text(&values)) {
                Err(reason) => {
                    return Ok(UnifiedAdmission::Rejected { reason });
                }
                Ok(rewritten) => {
                    values = parse_q2_userinfo(&rewritten).into_iter().collect();
                }
            }
        }
        let client = self
            .session
            .create_client(slot)
            .map_err(UnifiedServerError::AdmissionCleanup)?;
        self.session.connect_client(
            &client,
            match address {
                NetworkAddress::Loopback { .. } => UnifiedClientOrigin::Loopback,
                _ => UnifiedClientOrigin::Remote,
            },
        );
        if let Some(q3) = self.q3.as_mut() {
            q3.set_userinfo(slot, &Self::userinfo_text(&values));
        }
        let admitted = self.simulation.admit_player(&client);
        let actor = match admitted {
            Ok(player) => player.actor.clone(),
            Err(error) => {
                let actor = self
                    .simulation
                    .players()
                    .into_iter()
                    .find(|actor| self.simulation.movement_client(actor).as_ref() == Some(&client));
                let mut failures = vec![error.to_string()];
                if let Some(actor) = actor {
                    if let Err(cleanup) = self.simulation.disconnect_player(&actor) {
                        failures.push(cleanup);
                    }
                }
                if let Err(cleanup) = self.session.close_client(&client) {
                    failures.push(cleanup);
                }
                self.players.remove(&slot);
                self.userinfos.remove(&slot);
                if failures.len() > 1 {
                    return Err(UnifiedServerError::AdmissionCleanup(failures.join("; ")));
                }
                match error {
                    UnifiedAdmissionError::Denied { reason } => {
                        return Ok(UnifiedAdmission::Rejected { reason });
                    }
                    UnifiedAdmissionError::Failed { message } => {
                        return Err(UnifiedServerError::AdmissionCleanup(message));
                    }
                }
            }
        };
        let player = self.source_player(&client, &actor)?;
        self.players.insert(slot, player.clone());
        if self.q3.is_none() {
            self.update(&player, values);
        } else {
            self.userinfos.insert(slot, values);
        }
        Ok(UnifiedAdmission::Accepted { player })
    }

    /// Carry an admitted player.
    pub fn carried_player(&mut self, client: &ClientId) -> Result<UnifiedApplicationPlayer, UnifiedServerError> {
        let actor = self
            .simulation
            .players()
            .into_iter()
            .find(|actor| self.simulation.movement_client(actor).as_ref() == Some(client))
            .ok_or(UnifiedServerError::RetiredPlayer)?;
        let player = self.source_player(client, &actor)?;
        self.players.insert(client.slot(), player.clone());
        Ok(player)
    }

    /// Disconnect a player.
    pub fn disconnect(&mut self, player: &UnifiedApplicationPlayer) -> Result<(), UnifiedServerError> {
        self.require_player(player)?;
        let mut failures = Vec::new();
        if let Err(error) = self.simulation.disconnect_player(&player.actor) {
            failures.push(error);
        }
        if let Err(error) = self.session.close_client(&player.client) {
            failures.push(error);
        }
        self.players.remove(&player.client.slot());
        self.userinfos.remove(&player.client.slot());
        if failures.is_empty() {
            Ok(())
        } else {
            Err(UnifiedServerError::DisconnectFailed(failures.join("; ")))
        }
    }

    /// Update userinfo.
    pub fn userinfo(&mut self, player: &UnifiedApplicationPlayer, value: &str) -> Result<(), UnifiedServerError> {
        self.require_player(player)?;
        let address = self
            .userinfos
            .get(&player.client.slot())
            .and_then(|values| values.get("ip"))
            .cloned()
            .unwrap_or_default();
        let mut values: HashMap<String, String> = parse_q2_userinfo(value).into_iter().collect();
        values.insert("ip".to_string(), address);
        self.update(player, values);
        Ok(())
    }

    /// Component publications.
    pub fn components(
        &self,
        player: &UnifiedApplicationPlayer,
    ) -> Result<Vec<UnifiedComponentPublication>, UnifiedServerError> {
        self.require_player(player)?;
        let mut out = Vec::new();
        for source in self.simulation.mod_sources() {
            source.assert_current();
            let Some(context) = source.context(&player.actor) else {
                continue;
            };
            out.push(UnifiedComponentPublication {
                owner: source.owner(),
                identity: source.identity(),
                generation: source.generation(),
                abi: source.abi(),
                runtime: source.runtime(),
                viewer: player.actor.clone(),
                bindings: source.bindings(),
                context,
            });
        }
        Ok(out)
    }

    /// Native component publications.
    pub fn native_components(
        &self,
        player: &UnifiedApplicationPlayer,
    ) -> Result<Vec<UnifiedNativePublication>, UnifiedServerError> {
        self.require_player(player)?;
        let mut out = Vec::new();
        for source in self.simulation.native_mod_sources() {
            source.assert_current();
            let Some(frame) = source.native_frame(&player.actor) else {
                continue;
            };
            out.push(UnifiedNativePublication {
                owner: source.owner(),
                identity: source.identity(),
                generation: source.generation(),
                viewer: player.actor.clone(),
                frame,
            });
        }
        Ok(out)
    }

    /// Run a component command.
    pub fn component_command(
        &self,
        player: &UnifiedApplicationPlayer,
        owner: &PresentationOwner,
        generation: u64,
        args: &[String],
    ) -> Result<bool, UnifiedServerError> {
        self.require_player(player)?;
        for source in self.simulation.mod_sources() {
            if !qa_content::contract::same_presentation_owner(Some(&source.owner()), owner)
                || source.generation() != generation
            {
                continue;
            }
            if !source.live(&player.actor) || source.context(&player.actor).is_none() {
                return Ok(false);
            }
            source.assert_current();
            return Ok(source.client_command(&player.actor, args).unwrap_or(false));
        }
        Ok(false)
    }

    /// Run a player command.
    pub fn command(
        &mut self,
        player: &UnifiedApplicationPlayer,
        name: &str,
        args: &[String],
    ) -> Result<(), UnifiedServerError> {
        self.require_player(player)?;
        if self.q1.is_some() && name == "name" {
            let values: HashMap<String, String> =
                self.userinfos.get(&player.client.slot()).cloned().unwrap_or_default();
            let mut values = values;
            let name_value: String = args.first().cloned().unwrap_or_else(|| "unconnected".to_string());
            values.insert("name".to_string(), name_value.chars().take(15).collect());
            let player = player.clone();
            self.update(&player, values);
            return Ok(());
        }
        if let Some(q1) = self.q1.as_mut() {
            if name == "color" {
                let shirt: i32 = args.first().and_then(|value| value.parse().ok()).unwrap_or(0);
                let pants: i32 = args
                    .get(1)
                    .or_else(|| args.first())
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0);
                q1.set_colors(&player.actor, shirt, pants);
                return Ok(());
            }
        }
        self.simulation.player_command(&player.actor, name, args);
        Ok(())
    }

    /// Build an actor command.
    pub fn input(
        &self,
        player: &UnifiedApplicationPlayer,
        sequence: i64,
        command: UserCommand,
        arsenal: Option<qa_net::common::commands::ArsenalIntent>,
    ) -> Result<ActorCommand, UnifiedServerError> {
        self.require_player(player)?;
        if sequence < 0 {
            return Err(UnifiedServerError::InvalidSequence);
        }
        let dialect = self.simulation.movement_dialect(&player.actor);
        if dialect != Some(command.dialect()) {
            return Err(UnifiedServerError::CommandMismatch);
        }
        Ok(ActorCommand {
            actor: player.actor.clone(),
            source: qa_net::common::commands::CommandSource::Remote {
                client: player.client.clone(),
            },
            sequence: sequence as u64,
            command,
            arsenal,
        })
    }

    /// Resource keys for permitted sound events.
    pub fn resources(
        &self,
        player: &UnifiedApplicationPlayer,
        output: &SimulationOutput,
    ) -> Result<Vec<UnifiedResourceKey>, UnifiedServerError> {
        self.require_player(player)?;
        let _ = output;
        Ok(Vec::new())
    }

    /// Presentation frame for a player.
    pub fn frame(
        &self,
        player: &UnifiedApplicationPlayer,
        output: &SimulationOutput,
        epoch: u64,
        acknowledged_input: i64,
    ) -> Result<UnifiedPresentationFrame, UnifiedServerError> {
        self.require_player(player)?;
        let saved = SavedActorId::from(&player.actor);
        let mut snapshot = output.snapshot.clone();
        snapshot.inventories.retain(|value| value.id == saved);
        Ok(UnifiedPresentationFrame {
            epoch,
            acknowledged_input,
            prediction: self.simulation.unified_prediction(&player.actor, acknowledged_input),
            output: SimulationOutput {
                snapshot,
                events: output.events.clone(),
            },
            models: unified_model_presentations(&player.actor, &self.simulation.presentations()),
            characters: self.simulation.character_views(),
            world_text: self.simulation.world_text(),
            player_view: self.simulation.player_view(&player.actor),
            player_ui: self.simulation.player_ui(&player.actor),
        })
    }

    /// Filter presentation events for a player.
    pub fn presentation_events(
        &self,
        player: &UnifiedApplicationPlayer,
        events: &[SimulationPresentationEvent],
    ) -> Result<Vec<SimulationPresentationEvent>, UnifiedServerError> {
        self.require_player(player)?;
        Ok(events
            .iter()
            .filter(|event| unified_presentation_for(&player.actor, &player.client, event))
            .cloned()
            .collect())
    }

    /// Initial presentation events.
    pub fn initial_presentation(
        &self,
        player: &UnifiedApplicationPlayer,
    ) -> Result<Vec<SimulationPresentationEvent>, UnifiedServerError> {
        self.require_player(player)?;
        let persistent = self.simulation.persistent_presentation();
        self.presentation_events(player, &persistent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use qa_core::identity::IdentityOwner;
    
    #[test]
    fn userinfo_round_trip() {
        let values = parse_q2_userinfo("\\name\\player\\ip\\localhost");
        assert_eq!(
            values,
            vec![
                ("name".to_string(), "player".to_string()),
                ("ip".to_string(), "localhost".to_string())
            ]
        );
    }

    #[test]
    fn recipient_filtering() {
        let owner = IdentityOwner::create("unified-test").unwrap();
        let actor = owner.actor(1, 1);
        let other = owner.actor(2, 1);
        let client = owner.client(0, 0);
        let value = SimulationPresentationEvent {
            event: SourcePresentationEvent::Q1BackToLobby,
            owner: None,
            recipient: Some(other),
            sequence: 0,
            content: ContentId("test:content".to_string()),
            seconds: 0.0,
            source_entity: None,
        };
        assert!(!unified_presentation_for(&actor, &client, &value));
        let value = SimulationPresentationEvent {
            recipient: None,
            ..value
        };
        assert!(unified_presentation_for(&actor, &client, &value));
    }

    #[test]
    fn server_command_hidden() {
        let owner = IdentityOwner::create("unified-test").unwrap();
        let actor = owner.actor(1, 1);
        let client = owner.client(0, 0);
        let value = SimulationPresentationEvent {
            event: SourcePresentationEvent::Q1(Q1Event::ServerCommand {
                text: "test".to_string(),
            }),
            owner: None,
            recipient: None,
            sequence: 0,
            content: ContentId("test:content".to_string()),
            seconds: 0.0,
            source_entity: None,
        };
        assert!(!unified_presentation_for(&actor, &client, &value));
    }

    #[test]
    fn foreign_view_weapon_collapsed() {
        let owner = IdentityOwner::create("unified-test").unwrap();
        let viewer = owner.actor(1, 1);
        let other = owner.actor(2, 1);
        let base = SimulationPresentation {
            held_weapon: None,
            native_held_weapon: false,
            weapon_item: None,
            replaces_body: false,
            render_source_client: false,
            flare: None,
            actor: other.clone(),
            content: ContentId("test:content".to_string()),
            family: qa_content::contract::GameFamily::Q3,
            path: "models/weapons2/shotgun/shotgun.md3".to_string(),
            frame: 5,
            old_frame: 4,
            back_lerp: None,
            skin: 1,
            skin_path: None,
            indexed_skin: None,
            player_colors: None,
            effects: 7,
            render_flags: 3,
            origin: qa_core::math::vec3(1.0, 2.0, 3.0),
            previous_origin: None,
            model_beam: None,
            shader_beam: None,
            model_attachments: None,
            model_anchor: None,
            q3_grapple_cable: None,
            angles: qa_core::math::vec3(0.0, 90.0, 0.0),
            scale: 2.0,
            alpha: None,
            visible: true,
            view_weapon: true,
            q3_weapon: None,
        };
        let collapsed = unified_model_presentations(&viewer, &[base]);
        assert_eq!(collapsed.len(), 1);
        assert!(!collapsed[0].visible);
        assert_eq!(collapsed[0].frame, 0);
        assert_eq!(collapsed[0].scale, 1.0);
    }
} // end tests mod
