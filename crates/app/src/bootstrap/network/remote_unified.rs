//! Unified remote presentation.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/remote-unified.ts`
//! (`UnifiedRemotePresentation`). A read-only replica: identities and
//! presentation state, never an authoritative `Simulation`. The donor's
//! asynchronous loads resolve inline, preserving the donor's order.
//!
//! Media (`PresentationState`), user files, movement prediction, and the
//! scene surface live outside this shard. Media and prediction arrive
//! through [`UnifiedRemoteMedia`] and [`UnifiedRemotePredictor`]; the
//! scene behind [`UnifiedRemoteScene`] must share collision state with
//! the host predictor (host-internal, for example through `Rc`), because
//! the presentation links prediction bodies into its own scene while the
//! predictor replays against them. Loaded content extends
//! [`UnifiedLoadedContent`] with the recipe, geometry, prepared-mod, and
//! mount queries the replica and the component consumers read.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_content::contract::{GameFamily, ModUserFiles, PresentationOwner};
use qa_content::mounts::MountedContent;
use qa_core::identity::{ActorId, ClientId, IdentityOwner, OwnedActor, SeatId};
use qa_core::math::{Bounds, Vec3};
use qa_net::common::commands::UserCommand;
use qa_world::WorldError;
use thiserror::Error;

use super::remote_world::{RemoteWorldContent, RemoteWorldError};
use super::types::{PlayerUi, PlayerView, PresentationModel};
use super::unified_component_consumer::{
    PreparedComponentMod, UnifiedComponentClientSource, UnifiedComponentConsumerError, UnifiedComponentConsumers,
    UnifiedComponentHost, UnifiedComponentSource,
};
use super::unified_components::{UnifiedComponentFrames, UnifiedComponentUpdate};
use super::unified_content::{
    resolve_unified_resource, unified_resource_id, unified_resource_id_for_reference, UnifiedCompositionIdentity,
    UnifiedContentError, UnifiedLoadedContent,
};
use super::unified_control::{UnifiedArsenalIntent, UnifiedControl, UnifiedServerMode};
use super::unified_event_codec::{
    decode_unified_presentation_events, read_unified_simulation_event, OwnerLifecycleEvent, UnifiedEventError,
    UnifiedPresentationBody, UnifiedPresentationEvent, UnifiedSimulationEvent, UnifiedSimulationPayload,
};
use super::unified_frame_codec::{read_unified_frame, UnifiedFrameDecoder, UnifiedFrameError, UnifiedResourceKey};
use super::unified_native_consumer::{
    PreparedNativeMod, UnifiedNativeConsumerError, UnifiedNativeConsumers, UnifiedNativeHost, UnifiedNativeSource,
};
use super::unified_prediction::{
    Q1NetquakeMovement, Q1QuakeworldMovement, Q2ClassicMovement, Q2RereleaseMovement, Q3Movement,
    UnifiedCollisionRecord, UnifiedLinkedBody, UnifiedMovementState, UnifiedPredictionProjection,
};
use super::unified_types::UnifiedCharacterView;
use super::unified_types::{
    UnifiedDecodedModel, UnifiedIdentityDecoder, UnifiedOutput, UnifiedPresentationFrame, UnifiedSceneWorld,
};
use crate::persistence::recipe::{ExecutableRecipe, ResolvedResourceReference};
use qa_world::save::value::{decode_checkpoint_value, SaveReader};
use qa_world::session::SessionClient;

/// Unified replica failure.
#[derive(Debug, Error)]
pub enum UnifiedRemoteError {
    /// Frame failure.
    #[error(transparent)]
    Frame(#[from] UnifiedFrameError),
    /// Event failure.
    #[error(transparent)]
    Event(#[from] UnifiedEventError),
    /// Content failure.
    #[error(transparent)]
    Content(#[from] UnifiedContentError),
    /// Component failure.
    #[error(transparent)]
    Component(#[from] UnifiedComponentConsumerError),
    /// Native failure.
    #[error(transparent)]
    Native(#[from] UnifiedNativeConsumerError),
    /// World failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Remote world failure.
    #[error(transparent)]
    RemoteWorld(#[from] RemoteWorldError),
    /// Replica failure.
    #[error("{0}")]
    Remote(String),
}

/// Loaded-content surface the unified replica reads.
pub trait UnifiedRemoteContent: UnifiedLoadedContent {
    /// World geometry identity (donor `content.world`).
    fn world_geometry(&self) -> &str;
    /// Whether a prepared mod carries a component presentation.
    fn has_component_presentation(&self) -> bool;
    /// Locally prepared gameplay presentations.
    fn prepared_component_mods(&self) -> Vec<PreparedComponentMod>;
    /// Locally prepared native presentations.
    fn prepared_native_mods(&self) -> Vec<PreparedNativeMod>;
    /// Installed content mounts.
    fn mounted_content(&self, content: &str) -> Result<MountedContent, UnifiedRemoteError>;
    /// Product family and edition for content.
    fn product_edition(&self, content: &str) -> Result<(GameFamily, String), UnifiedRemoteError>;
}

/// Media surface behind the replica (donor `PresentationState`).
pub trait UnifiedRemoteMedia {
    /// Drained media event.
    type Event;
    /// Receive a presentation event, returning its local sequence.
    fn receive_presentation(&mut self, event: UnifiedPresentationEvent) -> f64;
    /// Drain received media events.
    fn take_presentation(&mut self) -> Vec<Self::Event>;
    /// Admit a replicated owner.
    fn admit_replicated_owner(&mut self, owner: &PresentationOwner, content: &str);
    /// Retire a replicated owner.
    fn retire_replicated_owner(&mut self, owner: &PresentationOwner);
}

/// Scene surface the replica links prediction bodies through.
pub trait UnifiedRemoteScene {
    /// Link a prediction body.
    fn link(&mut self, body: &UnifiedLinkedBody, collision: &UnifiedCollisionRecord);
    /// Unlink an actor.
    fn unlink(&mut self, actor: &ActorId);
}

/// Predicted player sample (donor `MovementPredictionResult['player']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPredictedPlayer {
    /// Predicted origin.
    pub origin: Vec3,
    /// Predicted velocity.
    pub velocity: Vec3,
    /// Predicted view angles.
    pub view_angles: Vec3,
    /// Predicted view height.
    pub view_height: f64,
    /// Predicted bounds.
    pub bounds: Bounds,
    /// Ground actor, when standing on one.
    pub ground_actor: Option<ActorId>,
}

/// Prediction command (donor `PredictionCommand`, absolute angle space).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedRemotePredictionCommand {
    /// Input sequence.
    pub sequence: i64,
    /// Command time in milliseconds.
    pub time_milliseconds: f64,
    /// Movement command.
    pub command: UserCommand,
    /// Arsenal intent, if any.
    pub arsenal: Option<UnifiedArsenalIntent>,
}

/// Movement predictor behind the replica (donor `SelectedMovementPrediction`).
///
/// `replay` returns `None` when the prediction is unusable (donor
/// `history-exhausted` and `unchanged` statuses); otherwise it returns the
/// predicted player sample.
pub trait UnifiedRemotePredictor {
    /// Receive an authoritative projection.
    fn receive(&mut self, projection: &UnifiedPredictionProjection);
    /// Submit a prediction command.
    fn submit(&mut self, command: &UnifiedRemotePredictionCommand);
    /// Replay pending commands against the latest projection.
    fn replay(&mut self) -> Option<UnifiedPredictedPlayer>;
}

/// Session offer (donor `Extract<UnifiedControl, {kind:'offer'}>`).
#[derive(Debug, Clone)]
pub struct UnifiedRemoteOffer<'a> {
    /// World epoch.
    pub epoch: u64,
    /// Composition identity.
    pub composition: &'a UnifiedCompositionIdentity,
    /// Server mode.
    pub mode: UnifiedServerMode,
    /// Maximum clients.
    pub max_clients: i64,
}

/// Host callbacks behind [`UnifiedRemoteOptions`].
pub trait UnifiedRemoteHost {
    /// Loaded content handle.
    type Content: UnifiedRemoteContent;
    /// Scene queries handle.
    type Scene: UnifiedRemoteScene;
    /// Media handle.
    type Media: UnifiedRemoteMedia;
    /// Predictor handle.
    type Predictor: UnifiedRemotePredictor;
    /// Load content for an offered world (donor async; resolves inline).
    fn load_content(&mut self, offer: &UnifiedRemoteOffer) -> Result<Self::Content, UnifiedRemoteError>;
    /// Build scene queries for loaded content.
    fn build_scene(content: &Self::Content) -> Self::Scene;
    /// Create media for an offered world.
    fn create_media(&mut self) -> Self::Media;
    /// Create a predictor for an admitted projection.
    fn create_predictor(
        &mut self,
        actor: OwnedActor,
        seat: &SeatId,
        initial: &UnifiedPredictionProjection,
        content: &Self::Content,
    ) -> Self::Predictor;
    /// Decode a model for a key and optional brush number.
    fn model(
        &self,
        key: &UnifiedResourceKey,
        brush_model: Option<i64>,
    ) -> Result<UnifiedDecodedModel, UnifiedRemoteError>;
    /// Publish a received output.
    fn publish(&mut self, output: &UnifiedOutput);
    /// Run an operation inside the component-publish transaction.
    fn publish_components(
        &mut self,
        operation: &mut dyn FnMut() -> Result<(), UnifiedRemoteError>,
    ) -> Result<(), UnifiedRemoteError> {
        operation()
    }
    /// Send a player command.
    fn send_command(&mut self, name: &str, args: &[String]);
    /// Send a component command.
    fn send_component_command(
        &mut self,
        owner: &PresentationOwner,
        generation: i64,
        args: &[String],
    ) -> Result<(), UnifiedRemoteError>;
    /// Handle a disconnect.
    fn disconnected(&mut self, reason: &str);
    /// Print server text.
    fn print(&mut self, text: &str);
    /// Mint the next actor generation for a slot.
    fn next_generation(&self, slot: u32) -> u32;
}

/// Unified remote presentation options (donor `UnifiedRemoteOptions`).
pub struct UnifiedRemoteOptions<H: UnifiedRemoteHost> {
    /// Identity authority.
    pub identity: IdentityOwner,
    /// Bound client.
    pub client: SessionClient,
    /// Local seat.
    pub seat: SeatId,
    /// Host callbacks.
    pub host: H,
    /// Client-owned writable storage for component presentations.
    pub mod_files: Option<ModUserFiles>,
}

/// Admitted binding (donor `Extract<UnifiedControl, {kind:'admitted'}>`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnifiedBinding {
    client_slot: u32,
    client_generation: u32,
    actor_slot: u32,
    actor_generation: u32,
    source_entity: i64,
}

/// Queued component command (donor `command` callback).
#[derive(Debug, Clone, PartialEq, Eq)]
struct QueuedComponentCommand {
    owner: PresentationOwner,
    generation: i64,
    args: Vec<String>,
}

/// State shared with the component consumer bridges.
struct ConsumerBridgeCore<C, M> {
    content: Option<Rc<C>>,
    media: M,
    generation: u64,
    closed: bool,
    viewer: Option<ActorId>,
    queue: Vec<QueuedComponentCommand>,
}

/// Bridge from the consumer hosts to the replica core.
struct ConsumerBridge<C, M> {
    core: Rc<RefCell<ConsumerBridgeCore<C, M>>>,
    expected_generation: u64,
    files: Option<ModUserFiles>,
}

impl<C: UnifiedRemoteContent, M: UnifiedRemoteMedia> ConsumerBridge<C, M> {
    fn current_check(&self) -> Result<(), String> {
        let core = self.core.borrow();
        if core.closed || core.generation != self.expected_generation {
            return Err("Unified replica load was retired".to_string());
        }
        Ok(())
    }
}

impl<C: UnifiedRemoteContent + 'static, M: UnifiedRemoteMedia + 'static> UnifiedComponentHost for ConsumerBridge<C, M> {
    fn assert_current(&self) -> Result<(), UnifiedComponentConsumerError> {
        self.current_check().map_err(UnifiedComponentConsumerError::Consumer)
    }

    fn viewer(&self) -> Option<ActorId> {
        self.core.borrow().viewer.clone()
    }

    fn send_command(
        &self,
        owner: &PresentationOwner,
        generation: i64,
        args: &[String],
    ) -> Result<(), UnifiedComponentConsumerError> {
        self.core.borrow_mut().queue.push(QueuedComponentCommand {
            owner: owner.clone(),
            generation,
            args: args.to_vec(),
        });
        Ok(())
    }

    fn admit_replicated_owner(
        &mut self,
        owner: &PresentationOwner,
        content: &str,
    ) -> Result<(), UnifiedComponentConsumerError> {
        self.current_check().map_err(UnifiedComponentConsumerError::Consumer)?;
        self.core.borrow_mut().media.admit_replicated_owner(owner, content);
        Ok(())
    }

    fn retire_replicated_owner(&mut self, owner: &PresentationOwner) -> Result<(), UnifiedComponentConsumerError> {
        self.current_check().map_err(UnifiedComponentConsumerError::Consumer)?;
        self.core.borrow_mut().media.retire_replicated_owner(owner);
        Ok(())
    }

    fn prepared_mods(&self) -> Vec<PreparedComponentMod> {
        self.core
            .borrow()
            .content
            .as_ref()
            .map_or_else(Vec::new, |content| content.prepared_component_mods())
    }

    fn for_content(&self, content: &str) -> Result<MountedContent, UnifiedComponentConsumerError> {
        let core = self.core.borrow();
        let content = core
            .content
            .as_ref()
            .ok_or_else(|| UnifiedComponentConsumerError::Consumer("Unified replica has no content".to_string()))?
            .mounted_content(content)
            .map_err(|error| UnifiedComponentConsumerError::Consumer(error.to_string()))?;
        Ok(content)
    }

    fn user_files(&mut self) -> &mut ModUserFiles {
        self.files.as_mut().expect("component files are present")
    }

    fn take_files(&mut self) -> Option<ModUserFiles> {
        self.files.take()
    }
}

impl<C: UnifiedRemoteContent + 'static, M: UnifiedRemoteMedia + 'static> UnifiedNativeHost for ConsumerBridge<C, M> {
    fn assert_current(&self) -> Result<(), UnifiedNativeConsumerError> {
        self.current_check().map_err(UnifiedNativeConsumerError::Consumer)
    }

    fn viewer(&self) -> Option<ActorId> {
        self.core.borrow().viewer.clone()
    }

    fn admit_replicated_owner(
        &mut self,
        owner: &PresentationOwner,
        content: &str,
    ) -> Result<(), UnifiedNativeConsumerError> {
        self.current_check().map_err(UnifiedNativeConsumerError::Consumer)?;
        self.core.borrow_mut().media.admit_replicated_owner(owner, content);
        Ok(())
    }

    fn retire_replicated_owner(&mut self, owner: &PresentationOwner) -> Result<(), UnifiedNativeConsumerError> {
        self.current_check().map_err(UnifiedNativeConsumerError::Consumer)?;
        self.core.borrow_mut().media.retire_replicated_owner(owner);
        Ok(())
    }

    fn prepared_mods(&self) -> Vec<PreparedNativeMod> {
        self.core
            .borrow()
            .content
            .as_ref()
            .map_or_else(Vec::new, |content| content.prepared_native_mods())
    }

    fn product_edition(&self, content: &str) -> Result<(GameFamily, String), UnifiedNativeConsumerError> {
        let core = self.core.borrow();
        let edition = core
            .content
            .as_ref()
            .ok_or_else(|| UnifiedNativeConsumerError::Consumer("Unified replica has no content".to_string()))?
            .product_edition(content)
            .map_err(|error| UnifiedNativeConsumerError::Consumer(error.to_string()))?;
        Ok(edition)
    }
}

/// Movement origin (donor `movementOrigin`).
fn movement_origin(state: &UnifiedMovementState) -> Vec3 {
    match state {
        UnifiedMovementState::Q1Netquake(Q1NetquakeMovement { origin, .. })
        | UnifiedMovementState::Q1Quakeworld(Q1QuakeworldMovement { origin, .. })
        | UnifiedMovementState::Q2Rerelease(Q2RereleaseMovement { origin, .. })
        | UnifiedMovementState::Q3(Q3Movement { origin, .. }) => *origin,
        UnifiedMovementState::Q2Classic(Q2ClassicMovement { origin_eighths, .. }) => Vec3 {
            x: (origin_eighths[0] / 8.0) as f32,
            y: (origin_eighths[1] / 8.0) as f32,
            z: (origin_eighths[2] / 8.0) as f32,
        },
    }
}

/// Admitted replica player (donor `player`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedRemotePlayer {
    /// Player actor.
    pub actor: ActorId,
    /// Owning client.
    pub client: ClientId,
    /// Source entity number.
    pub source_entity: i64,
}

/// Mod client presentation source (donor `ActiveModClientPresentation`).
#[derive(Debug, Clone)]
pub enum UnifiedRemoteClientSource {
    /// Gameplay component source.
    Component(UnifiedComponentClientSource),
    /// Native component source.
    Native(UnifiedNativeSource),
}

/// Pending event batch (donor `pendingEvents` entry).
struct PendingEvents {
    frame: i32,
    events: Vec<UnifiedPresentationEvent>,
    simulation: Vec<UnifiedSimulationEvent>,
}

/// Unified remote presentation (donor `UnifiedRemotePresentation`).
pub struct UnifiedRemotePresentation<H: UnifiedRemoteHost> {
    options: UnifiedRemoteOptions<H>,
    world: RemoteWorldContent<Rc<H::Content>, H::Scene, ()>,
    core: Rc<RefCell<ConsumerBridgeCore<H::Content, H::Media>>>,
    actors: RefCell<HashMap<(u32, u32), ActorId>>,
    clients: RefCell<HashMap<(u32, u32), ClientId>>,
    resources: RefCell<HashMap<qa_content::contract::ResourceId, ResolvedResourceReference>>,
    pending_events: Vec<PendingEvents>,
    source_sequences: HashMap<i64, f64>,
    components: Option<UnifiedComponentConsumers>,
    native: Option<UnifiedNativeConsumers>,
    simulation_events: Vec<UnifiedSimulationEvent>,
    current: Option<UnifiedPresentationFrame>,
    binding: Option<UnifiedBinding>,
    predictor: Option<H::Predictor>,
    predicted: Option<UnifiedPredictedPlayer>,
    projection: Option<UnifiedPredictionProjection>,
    linked: HashSet<ActorId>,
    generation: u64,
    world_epoch: u64,
    content_epoch: u64,
    offered_digest: Option<String>,
    last_event_sequence: f64,
    last_simulation_sequence: f64,
    files: Option<ModUserFiles>,
}

fn sequence_key(sequence: f64) -> i64 {
    sequence as i64
}

fn empty_component_frames() -> UnifiedComponentFrames {
    UnifiedComponentFrames {
        revision: 0,
        sources: Vec::new(),
        native: None,
    }
}

impl<H: UnifiedRemoteHost + 'static> UnifiedRemotePresentation<H>
where
    H::Content: 'static,
    H::Media: 'static,
{
    /// Wrap presentation options.
    pub fn new(mut options: UnifiedRemoteOptions<H>) -> Self {
        let files = options.mod_files.take();
        let world = RemoteWorldContent::new(None, |content: &Rc<H::Content>, _| H::build_scene(content));
        // The initial media handle is replaced on the first offer; create a
        // placeholder through the host so drains before admission behave.
        let media = options.host.create_media();
        Self {
            options,
            world,
            core: Rc::new(RefCell::new(ConsumerBridgeCore {
                content: None,
                media,
                generation: 0,
                closed: false,
                viewer: None,
                queue: Vec::new(),
            })),
            actors: RefCell::new(HashMap::new()),
            clients: RefCell::new(HashMap::new()),
            resources: RefCell::new(HashMap::new()),
            pending_events: Vec::new(),
            source_sequences: HashMap::new(),
            components: None,
            native: None,
            simulation_events: Vec::new(),
            current: None,
            binding: None,
            predictor: None,
            predicted: None,
            projection: None,
            linked: HashSet::new(),
            generation: 0,
            world_epoch: 0,
            content_epoch: 0,
            offered_digest: None,
            last_event_sequence: -1.0,
            last_simulation_sequence: -1.0,
            files,
        }
    }

    /// Borrow the bound client.
    #[must_use]
    pub fn client(&self) -> &SessionClient {
        &self.options.client
    }

    /// Offered composition digest, if any.
    #[must_use]
    pub fn composition_digest(&self) -> Option<&str> {
        self.offered_digest.as_deref()
    }

    /// World epoch.
    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.world_epoch
    }

    /// Loaded content epoch.
    #[must_use]
    pub fn loaded_epoch(&self) -> u64 {
        self.content_epoch
    }

    /// Borrow the scene queries, building them lazily from the content.
    pub fn scene(&mut self) -> Result<&H::Scene, RemoteWorldError> {
        self.world.scene()
    }

    /// Borrow the current output, if any.
    #[must_use]
    pub fn output(&self) -> Option<&UnifiedOutput> {
        self.current.as_ref().map(|current| &current.output)
    }

    /// Borrow the admitted player, if any.
    #[must_use]
    pub fn player(&self) -> Option<UnifiedRemotePlayer> {
        let current = self.current.as_ref()?;
        let binding = self.binding.as_ref()?;
        Some(UnifiedRemotePlayer {
            actor: current.player.actor.clone(),
            client: self.options.client.id().clone(),
            source_entity: binding.source_entity,
        })
    }

    fn assert_current(&self, generation: u64) -> Result<(), UnifiedRemoteError> {
        if self.core.borrow().closed || generation != self.generation {
            return Err(UnifiedRemoteError::Remote(
                "Unified replica load was retired".to_string(),
            ));
        }
        Ok(())
    }

    fn sync_viewer(&self) {
        let viewer = self.player().map(|player| player.actor);
        self.core.borrow_mut().viewer = viewer;
    }

    fn content(&self) -> Result<&H::Content, UnifiedRemoteError> {
        self.world.content().map(Rc::as_ref).map_err(UnifiedRemoteError::from)
    }

    fn recipe(&self) -> Result<&ExecutableRecipe, UnifiedRemoteError> {
        self.content().map(|content| content.recipe())
    }

    fn mint_actor(&self, slot: u32) -> ActorId {
        self.options
            .identity
            .actor(slot, self.options.host.next_generation(slot))
    }

    fn decode_actor(&self, slot: u32, generation: u32) -> ActorId {
        if let Some(found) = self.actors.borrow().get(&(slot, generation)) {
            return found.clone();
        }
        let value = self.mint_actor(slot);
        self.actors.borrow_mut().insert((slot, generation), value.clone());
        value
    }

    fn decode_client(&self, slot: u32, generation: u32) -> ClientId {
        if let Some(binding) = self.binding.as_ref() {
            if binding.client_slot == slot && binding.client_generation == generation {
                return self.options.client.id().clone();
            }
        }
        if let Some(found) = self.clients.borrow().get(&(slot, generation)) {
            return found.clone();
        }
        // Remote references are not admitted session clients; their
        // generation is epoch-scoped.
        let value = self
            .options
            .identity
            .client(slot, self.options.host.next_generation(slot));
        self.clients.borrow_mut().insert((slot, generation), value.clone());
        value
    }

    /// Receive a session offer (donor `offer`).
    pub fn offer(&mut self, control: &UnifiedControl) -> Result<(), UnifiedRemoteError> {
        let UnifiedControl::Offer {
            epoch,
            composition,
            mode,
            max_clients,
        } = control
        else {
            return Err(UnifiedRemoteError::Remote(
                "Unified replica offer requires an offer control".to_string(),
            ));
        };
        if self.core.borrow().closed {
            return Err(UnifiedRemoteError::Remote("Unified replica is closed".to_string()));
        }
        let epoch = *epoch;
        let mode = *mode;
        let max_clients = *max_clients;
        // Retire inside the component-publish transaction. The closure only
        // touches presentation fields and the shared generation cell, so the
        // host borrow and the core borrow never overlap.
        let components = &mut self.components;
        let native = &mut self.native;
        let files = &mut self.files;
        let core = self.core.clone();
        let world_epoch = &mut self.world_epoch;
        let offered_digest = &mut self.offered_digest;
        let binding = &mut self.binding;
        let generation = &mut self.generation;
        self.options.host.publish_components(&mut || {
            if let Some(consumers) = components.take() {
                let mut consumers = consumers;
                consumers.close()?;
                if files.is_none() {
                    *files = consumers.host_mut().take_files();
                }
            }
            if let Some(consumers) = native.take() {
                let mut consumers = consumers;
                consumers.close();
            }
            *generation += 1;
            core.borrow_mut().generation = *generation;
            *world_epoch = epoch;
            *offered_digest = Some(composition.digest.to_string());
            *binding = None;
            Ok(())
        })?;
        let content = self.options.host.load_content(&UnifiedRemoteOffer {
            epoch,
            composition,
            mode,
            max_clients,
        })?;
        self.assert_current(self.generation)?;
        self.clear_prediction()?;
        self.world.set_content(Rc::new(content));
        let has_presentation = self.content()?.has_component_presentation();
        self.content_epoch = epoch;
        self.current = None;
        self.actors.borrow_mut().clear();
        self.clients.borrow_mut().clear();
        self.resources.borrow_mut().clear();
        self.source_sequences.clear();
        self.simulation_events.clear();
        self.pending_events.clear();
        self.last_event_sequence = -1.0;
        self.last_simulation_sequence = -1.0;
        {
            let mut core = self.core.borrow_mut();
            core.media = self.options.host.create_media();
            core.content = self.world.content().ok().cloned();
        }
        self.sync_viewer();
        let core = self.core.clone();
        self.native = Some(UnifiedNativeConsumers::new(Box::new(ConsumerBridge {
            core: core.clone(),
            expected_generation: self.generation,
            files: None,
        })));
        if has_presentation {
            if self.files.is_none() {
                return Err(UnifiedRemoteError::Remote(
                    "Remote component presentation requires client-owned writable storage".to_string(),
                ));
            }
            self.components = Some(UnifiedComponentConsumers::new(Box::new(ConsumerBridge {
                core,
                expected_generation: self.generation,
                files: self.files.take(),
            })));
        }
        Ok(())
    }

    /// Receive an admission binding (donor `admitted`).
    pub fn admitted(&mut self, control: &UnifiedControl) {
        let UnifiedControl::Admitted {
            epoch,
            client,
            actor,
            source_entity,
        } = control
        else {
            return;
        };
        if *epoch != self.world_epoch {
            return;
        }
        let binding = UnifiedBinding {
            client_slot: u32::try_from(client.slot).expect("unified client slot fits"),
            client_generation: u32::try_from(client.generation).expect("unified client generation fits"),
            actor_slot: u32::try_from(actor.slot).expect("unified actor slot fits"),
            actor_generation: u32::try_from(actor.generation).expect("unified actor generation fits"),
            source_entity: *source_entity,
        };
        self.decode_actor(binding.actor_slot, binding.actor_generation);
        self.binding = Some(binding);
        self.sync_viewer();
    }

    /// Drain queued component commands to the host.
    fn drain_command_queue(&mut self) -> Result<(), UnifiedRemoteError> {
        let queued = std::mem::take(&mut self.core.borrow_mut().queue);
        for command in queued {
            self.options
                .host
                .send_component_command(&command.owner, command.generation, &command.args)?;
        }
        Ok(())
    }

    /// Receive a component update (donor `receiveComponents`).
    pub fn receive_components(
        &mut self,
        epoch: u64,
        update: &UnifiedComponentUpdate,
    ) -> Result<(), UnifiedRemoteError> {
        let components = &mut self.components;
        let native = &mut self.native;
        let core = self.core.clone();
        let world_epoch = self.world_epoch;
        self.options.host.publish_components(&mut || {
            if epoch != world_epoch || core.borrow().closed {
                return Ok(());
            }
            let commit = match native.as_mut() {
                Some(native) => Some(native.prepare_update(update)?),
                None => None,
            };
            match components.as_mut() {
                None => {
                    if !update.sources.is_empty() {
                        return Err(UnifiedRemoteError::Remote(
                            "Server activated an unqualified remote component".to_string(),
                        ));
                    }
                }
                Some(components) => components.update(update)?,
            }
            if let (Some(native), Some(commit)) = (native.as_mut(), commit) {
                native.commit_update(commit)?;
            }
            Ok(())
        })?;
        self.drain_command_queue()
    }

    /// Admitted gameplay component sources.
    pub fn mod_presentation_sources(&self) -> Result<Vec<UnifiedComponentSource>, UnifiedRemoteError> {
        match self.components.as_ref() {
            None => Ok(Vec::new()),
            Some(components) => components.sources().map_err(UnifiedRemoteError::from),
        }
    }

    /// Active mod client presentation sources.
    pub fn mod_client_presentation_sources(&self) -> Result<Vec<UnifiedRemoteClientSource>, UnifiedRemoteError> {
        let mut sources = Vec::new();
        if let Some(components) = self.components.as_ref() {
            sources.extend(
                components
                    .client_sources()?
                    .into_iter()
                    .map(UnifiedRemoteClientSource::Component),
            );
        }
        if let Some(native) = self.native.as_ref() {
            sources.extend(native.sources()?.into_iter().map(UnifiedRemoteClientSource::Native));
        }
        Ok(sources)
    }

    /// Declare resource keys (donor `declare`).
    pub fn declare(&mut self, epoch: u64, keys: &[UnifiedResourceKey]) -> Result<(), UnifiedRemoteError> {
        if epoch != self.world_epoch || self.core.borrow().closed {
            return Ok(());
        }
        let generation = self.generation;
        for key in keys {
            let resource = {
                let content = self.content()?;
                let loaded: &dyn UnifiedLoadedContent = content;
                resolve_unified_resource(loaded, key)?
            };
            self.assert_current(generation)?;
            let id = unified_resource_id(key)?;
            self.resources.borrow_mut().insert(id, resource);
        }
        Ok(())
    }

    /// Receive event payloads (donor `receiveEvents`).
    pub fn receive_events(
        &mut self,
        epoch: u64,
        frame: i32,
        payload: &[u8],
        simulation: &[u8],
    ) -> Result<(), UnifiedRemoteError> {
        if epoch != self.world_epoch || self.core.borrow().closed {
            return Ok(());
        }
        let identity = self as &dyn UnifiedIdentityDecoder;
        let events = decode_unified_presentation_events(payload, identity)?;
        let value = decode_checkpoint_value(simulation)?;
        let reader = SaveReader::new(&value);
        let simulation = reader.list(|event| read_unified_simulation_event(event, identity))?;
        self.pending_events.push(PendingEvents {
            frame,
            events,
            simulation,
        });
        self.release_events();
        Ok(())
    }

    /// Release event batches at or below the current frame.
    fn release_events(&mut self) -> Vec<UnifiedSimulationEvent> {
        let current = match self.current.as_ref() {
            Some(current) => current.output.snapshot.frame.frame,
            None => return Vec::new(),
        };
        let mut simulation = Vec::new();
        while self.pending_events.first().is_some_and(|next| next.frame <= current) {
            let next = self.pending_events.remove(0);
            for event in next.events {
                if event.sequence > self.last_event_sequence {
                    if let UnifiedPresentationBody::PresentationOwner {
                        event: OwnerLifecycleEvent::Retired { owner },
                    } = &event.body
                    {
                        if let Some(native) = self.native.as_mut() {
                            native.retire(owner);
                        }
                    }
                    let local = self.core.borrow_mut().media.receive_presentation(event.clone());
                    self.source_sequences.insert(sequence_key(event.sequence), local);
                    self.last_event_sequence = event.sequence;
                }
            }
            for event in next.simulation {
                if event.sequence > self.last_simulation_sequence {
                    let source = match &event.payload {
                        UnifiedSimulationPayload::Message {
                            source_presentation_sequence,
                            ..
                        } => *source_presentation_sequence,
                        _ => None,
                    };
                    let local = source.and_then(|source| self.source_sequences.get(&sequence_key(source)).copied());
                    let remapped = match (&event.payload, local) {
                        (UnifiedSimulationPayload::Message { event: network, .. }, Some(local)) => {
                            Some(UnifiedSimulationPayload::Message {
                                event: network.clone(),
                                source_presentation_sequence: Some(local),
                            })
                        }
                        _ => None,
                    };
                    let mut event = event;
                    if let Some(payload) = remapped {
                        event.payload = payload;
                    }
                    simulation.push(event.clone());
                    self.last_simulation_sequence = event.sequence;
                }
            }
        }
        if self.source_sequences.len() > 4096 {
            let horizon = self.last_event_sequence as i64 - 2048;
            self.source_sequences.retain(|sequence, _| *sequence >= horizon);
        }
        self.simulation_events.extend(simulation.clone());
        simulation
    }

    /// Receive a frame (donor `receiveFrame`).
    pub fn receive_frame(&mut self, bytes: &[u8]) -> Result<Option<i64>, UnifiedRemoteError> {
        let envelope = read_unified_frame(bytes)?;
        if self.core.borrow().closed || envelope.epoch != self.world_epoch || self.binding.is_none() {
            return Ok(None);
        }
        let generation = self.generation;
        let frame = envelope.decode(self)?;
        self.assert_current(generation)?;
        let binding = self.binding.clone().expect("binding is present");
        let expected = self.decode_actor(binding.actor_slot, binding.actor_generation);
        if frame.player.actor != expected {
            return Err(UnifiedRemoteError::Remote(
                "Unified frame changed the admitted player".to_string(),
            ));
        }
        if self
            .current
            .as_ref()
            .is_some_and(|current| frame.output.snapshot.frame.frame <= current.output.snapshot.frame.frame)
        {
            return Ok(Some(frame.acknowledged_input));
        }
        let frames = frame.components.clone().unwrap_or_else(empty_component_frames);
        let commit = match self.native.as_mut() {
            Some(native) => native.prepare(&frames, &expected, frame.native_camera.as_ref())?,
            None => None,
        };
        // A missing native collection predates the first offer; its epoch
        // gate above already returned, so reaching here with no native
        // collection means the frame carries no admittable components.
        if self.native.is_some() && commit.is_none() {
            return Ok(Some(frame.acknowledged_input));
        }
        match self.components.as_mut() {
            Some(components) => {
                if !components.accept(&frames, &expected)? {
                    return Ok(Some(frame.acknowledged_input));
                }
            }
            None => {
                if frame
                    .components
                    .as_ref()
                    .is_some_and(|frames| !frames.sources.is_empty())
                {
                    return Err(UnifiedRemoteError::Remote(
                        "Remote frame contains unadmitted components".to_string(),
                    ));
                }
            }
        }
        let acknowledged = frame.acknowledged_input;
        self.current = Some(UnifiedPresentationFrame {
            output: UnifiedOutput {
                snapshot: frame.output.snapshot.clone(),
                events: Vec::new(),
            },
            ..frame.clone()
        });
        if let (Some(native), Some(commit)) = (self.native.as_mut(), commit) {
            native.commit_frames(commit);
        }
        self.correct_prediction(&frame)?;
        self.release_events();
        if let Some(current) = self.current.as_ref() {
            let output = current.output.clone();
            self.options.host.publish(&output);
        }
        self.sync_viewer();
        self.drain_command_queue()?;
        Ok(Some(acknowledged))
    }
}

impl<H: UnifiedRemoteHost + 'static> UnifiedIdentityDecoder for UnifiedRemotePresentation<H>
where
    H::Content: 'static,
    H::Media: 'static,
{
    fn session(&self) -> qa_core::identity::SessionId {
        self.options.identity.session().clone()
    }

    fn actor(&self, slot: u32, generation: u32) -> ActorId {
        self.decode_actor(slot, generation)
    }

    fn client(&self, slot: u32, generation: u32) -> ClientId {
        self.decode_client(slot, generation)
    }

    fn seat(&self, _index: u32) -> SeatId {
        self.options.seat.clone()
    }

    fn resource_id(&self, id: &str) -> String {
        self.resources
            .borrow()
            .get(&qa_content::contract::ResourceId(id.to_string()))
            .map(|resource| resource.id.clone())
            .unwrap_or_else(|| panic!("Unified resource was not declared: {id}"))
    }
}

impl<H: UnifiedRemoteHost + 'static> UnifiedFrameDecoder for UnifiedRemotePresentation<H>
where
    H::Content: 'static,
    H::Media: 'static,
{
    fn world(&self) -> Option<UnifiedSceneWorld> {
        let content = self.content().ok()?;
        Some(UnifiedSceneWorld {
            resource: content.recipe().map.geometry.clone(),
            geometry: content.world_geometry().to_string(),
        })
    }

    fn resource(&self, key: &UnifiedResourceKey) -> Result<ResolvedResourceReference, UnifiedFrameError> {
        let id = unified_resource_id(key)
            .map_err(|error| UnifiedFrameError::Frame(format!("Unified resource id is invalid: {error}")))?;
        if let Some(existing) = self.resources.borrow().get(&id) {
            return Ok(existing.clone());
        }
        let content = self
            .content()
            .map_err(|error| UnifiedFrameError::Frame(error.to_string()))?;
        let loaded: &dyn UnifiedLoadedContent = content;
        let value =
            resolve_unified_resource(loaded, key).map_err(|error| UnifiedFrameError::Frame(error.to_string()))?;
        if self.core.borrow().closed || self.core.borrow().generation != self.generation {
            return Err(UnifiedFrameError::Frame("Unified replica load was retired".to_string()));
        }
        self.resources.borrow_mut().insert(id, value.clone());
        Ok(value)
    }

    fn model(
        &self,
        key: &UnifiedResourceKey,
        brush_model: Option<i64>,
    ) -> Result<UnifiedDecodedModel, UnifiedFrameError> {
        self.options
            .host
            .model(key, brush_model)
            .map_err(|error| UnifiedFrameError::Frame(error.to_string()))
    }
}

impl<H: UnifiedRemoteHost + 'static> UnifiedRemotePresentation<H>
where
    H::Content: 'static,
    H::Media: 'static,
{
    /// Clear prediction state, unlinking prediction bodies.
    fn clear_prediction(&mut self) -> Result<(), UnifiedRemoteError> {
        if self.projection.is_some() {
            let linked = std::mem::take(&mut self.linked);
            let scene = self.world.scene_mut().map_err(UnifiedRemoteError::from)?;
            for actor in linked {
                scene.unlink(&actor);
            }
        }
        self.linked.clear();
        self.predictor = None;
        self.predicted = None;
        self.projection = None;
        Ok(())
    }

    /// Correct prediction against a received frame.
    fn correct_prediction(&mut self, frame: &UnifiedPresentationFrame) -> Result<(), UnifiedRemoteError> {
        let projection = &frame.prediction;
        if projection.actor != frame.player.actor || projection.sequence != frame.acknowledged_input {
            return Err(UnifiedRemoteError::Remote(
                "Unified prediction does not match admitted snapshot".to_string(),
            ));
        }
        if self
            .projection
            .as_ref()
            .is_some_and(|current| current.actor != projection.actor)
        {
            self.clear_prediction()?;
        }
        let next_linked: HashSet<ActorId> = projection
            .collisions
            .iter()
            .map(|entry| entry.body.actor.clone())
            .collect();
        let stale: Vec<ActorId> = self
            .linked
            .iter()
            .filter(|actor| !next_linked.contains(*actor))
            .cloned()
            .collect();
        {
            let scene = self.world.scene_mut().map_err(UnifiedRemoteError::from)?;
            for actor in &stale {
                scene.unlink(actor);
            }
            for entry in &projection.collisions {
                scene.link(&entry.body, &entry.collision);
            }
        }
        for actor in stale {
            self.linked.remove(&actor);
        }
        for entry in &projection.collisions {
            self.linked.insert(entry.body.actor.clone());
        }
        self.projection = Some(projection.clone());
        if self.predictor.is_none() {
            let provider = self.recipe()?.map.entities.provider.clone();
            let owned = self
                .options
                .identity
                .owned_actor(&projection.actor, provider_id(&provider))
                .map_err(|error| UnifiedRemoteError::Remote(error.to_string()))?;
            let world = &self.world;
            let options = &mut self.options;
            let content = world.content().map(Rc::as_ref).map_err(UnifiedRemoteError::from)?;
            let seat = options.seat.clone();
            let predictor = options.host.create_predictor(owned, &seat, projection, content);
            self.predictor = Some(predictor);
        } else if let Some(predictor) = self.predictor.as_mut() {
            predictor.receive(projection);
        }
        self.predicted = match self.predictor.as_mut() {
            Some(predictor) => predictor.replay(),
            None => None,
        };
        Ok(())
    }

    /// Authoritative command time, if a projection is admitted.
    #[must_use]
    pub fn command_time_milliseconds(&self) -> Option<f64> {
        self.projection
            .as_ref()
            .map(|projection| projection.command_time_milliseconds)
    }

    /// Submit a prediction command (donor `predict`).
    pub fn predict(&mut self, command: &UnifiedRemotePredictionCommand) {
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.submit(command);
            self.predicted = predictor.replay();
        }
    }

    fn predicted_player(&self) -> Option<&UnifiedPredictedPlayer> {
        self.predicted.as_ref()
    }

    /// Shift an origin by the prediction delta (donor `shift`).
    fn shift(&self, origin: Vec3) -> Vec3 {
        let (Some(player), Some(current)) = (self.predicted_player(), self.current.as_ref()) else {
            return origin;
        };
        let predicted = player.origin;
        let original = movement_origin(&current.prediction.state);
        Vec3 {
            x: origin.x + predicted.x - original.x,
            y: origin.y + predicted.y - original.y,
            z: origin.z + predicted.z - original.z,
        }
    }

    /// Sample the predicted presentation (donor `samplePresentation`).
    #[must_use]
    pub fn sample_presentation(&self, _now: u64) -> Option<UnifiedOutput> {
        let output = self.output()?.clone();
        let player = self.predicted_player()?.clone();
        let actor = self.current.as_ref()?.player.actor.clone();
        Some(UnifiedOutput {
            snapshot: super::unified_types::UnifiedSnapshot {
                bodies: output
                    .snapshot
                    .bodies
                    .into_iter()
                    .map(|entry| {
                        if entry.actor == actor {
                            super::unified_types::UnifiedSnapshotBody {
                                body: super::unified_types::UnifiedBodyState {
                                    origin: player.origin,
                                    velocity: player.velocity,
                                    bounds: player.bounds,
                                    ground: player.ground_actor.clone(),
                                    ..entry.body
                                },
                                ..entry
                            }
                        } else {
                            entry
                        }
                    })
                    .collect(),
                scene: super::unified_types::UnifiedSceneSnapshot {
                    entities: output
                        .snapshot
                        .scene
                        .entities
                        .into_iter()
                        .map(|entity| {
                            if entity.actor.as_ref() == Some(&actor) {
                                super::unified_types::UnifiedSceneEntity {
                                    transform: super::unified_types::UnifiedEntityTransform {
                                        origin: self.shift(entity.transform.origin),
                                        ..entity.transform
                                    },
                                    previous_origin: self.shift(entity.previous_origin),
                                    lighting_origin: self.shift(entity.lighting_origin),
                                    ..entity
                                }
                            } else {
                                entity
                            }
                        })
                        .collect(),
                    ..output.snapshot.scene
                },
                ..output.snapshot
            },
            events: output.events,
        })
    }

    /// HUD state for an actor (donor `playerUi`).
    #[must_use]
    pub fn player_ui(&self, actor: &ActorId) -> PlayerUi {
        match self.current.as_ref() {
            Some(current) if current.player.actor == *actor => current.player.ui.clone(),
            _ => panic!("Unified player UI belongs to another actor"),
        }
    }

    /// Camera view for an actor (donor `playerView`).
    #[must_use]
    pub fn player_view(&self, actor: &ActorId) -> PlayerView {
        let current = match self.current.as_ref() {
            Some(current) if current.player.actor == *actor => current,
            _ => panic!("Unified player view belongs to another actor"),
        };
        let view = current.player.view.clone();
        match self.predicted_player() {
            None => view,
            Some(player) => PlayerView {
                origin: self.shift(view.origin),
                angles: player.view_angles,
                view_height: if view.client_view_offset_delta.is_none() {
                    player.view_height
                } else {
                    view.view_height
                },
                ..view
            },
        }
    }

    /// Visible presentations (donor `presentations`).
    #[must_use]
    pub fn presentations(&self) -> Vec<PresentationModel> {
        let actor = self.current.as_ref().map(|current| &current.player.actor);
        self.current
            .as_ref()
            .map(|current| {
                current
                    .models
                    .iter()
                    .map(|model| {
                        if actor == Some(&model.actor) {
                            let mut shifted = model.clone();
                            shifted.origin = self.shift(model.origin);
                            if let Some(previous) = model.previous_origin {
                                shifted.previous_origin = Some(self.shift(previous));
                            }
                            shifted
                        } else {
                            model.clone()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Visible character views (donor `characterViews`).
    #[must_use]
    pub fn character_views(&self) -> Vec<UnifiedCharacterView> {
        let actor = self.current.as_ref().map(|current| &current.player.actor);
        let player = self.predicted_player().cloned();
        self.current
            .as_ref()
            .map(|current| {
                current
                    .characters
                    .iter()
                    .map(|view| match (&player, actor) {
                        (Some(player), Some(actor)) if view.actor == *actor => {
                            let mut shifted = view.clone();
                            shifted.origin = self.shift(view.origin);
                            shifted.velocity = player.velocity;
                            shifted
                        }
                        _ => view.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Visible world text (donor `worldText`).
    #[must_use]
    pub fn world_text(&self) -> Vec<super::types::WorldText> {
        self.current
            .as_ref()
            .map(|current| current.texts.clone())
            .unwrap_or_default()
    }

    /// Whether an actor is configured (donor `isPlayer`).
    #[must_use]
    pub fn is_player(&self, actor: &ActorId) -> bool {
        self.current.as_ref().is_some_and(|current| {
            current
                .output
                .snapshot
                .configurations
                .iter()
                .any(|value| value.actor == *actor)
        })
    }

    /// Resolve the source slot behind an actor (donor `numberOf`).
    #[must_use]
    pub fn number_of(&self, actor: &ActorId) -> Option<u32> {
        self.actors
            .borrow()
            .iter()
            .find(|(_, value)| *value == actor)
            .map(|(key, _)| key.0)
    }

    /// Run a player command (donor `playerCommand`).
    pub fn player_command(&mut self, actor: &ActorId, name: &str, args: &[String]) {
        match self.player() {
            Some(player) if player.actor == *actor => {}
            _ => panic!("Unified command belongs to another player"),
        }
        self.options.host.send_command(name, args);
    }

    /// Register a resolved resource (donor `registerResource`).
    pub fn register_resource(&mut self, resource: &ResolvedResourceReference) -> Result<(), UnifiedRemoteError> {
        let id = unified_resource_id_for_reference(resource)?;
        self.resources.borrow_mut().insert(id, resource.clone());
        Ok(())
    }

    /// Drain simulation events.
    pub fn drain_simulation_events(&mut self) -> Vec<UnifiedSimulationEvent> {
        std::mem::take(&mut self.simulation_events)
    }

    /// Drain media presentation events.
    pub fn drain_presentation_events(&mut self) -> Vec<<H::Media as UnifiedRemoteMedia>::Event> {
        self.core.borrow_mut().media.take_presentation()
    }

    /// Handle a disconnect (donor `disconnected`).
    pub fn disconnected(&mut self, reason: &str) {
        self.options.host.disconnected(reason);
    }

    /// Print server text (donor `print`).
    pub fn print(&mut self, text: &str) {
        self.options.host.print(text);
    }

    /// Close the replica (donor `close`).
    pub fn close(&mut self) {
        if let Some(mut consumers) = self.components.take() {
            let _ = consumers.close();
            if self.files.is_none() {
                self.files = consumers.host_mut().take_files();
            }
        }
        if let Some(mut consumers) = self.native.take() {
            consumers.close();
        }
        let _ = self.clear_prediction();
        self.content_epoch = 0;
        self.core.borrow_mut().closed = true;
        self.generation += 1;
        self.core.borrow_mut().generation = self.generation;
        self.current = None;
        self.binding = None;
        self.pending_events.clear();
        self.source_sequences.clear();
        self.core.borrow_mut().media.take_presentation();
        self.simulation_events.clear();
        self.resources.borrow_mut().clear();
        self.actors.borrow_mut().clear();
        self.clients.borrow_mut().clear();
        self.sync_viewer();
    }
}

fn provider_id(provider: &str) -> qa_core::identity::ProviderId {
    let (namespace, name) = provider.split_once(':').unwrap_or(("", ""));
    qa_core::identity::ProviderId::new(namespace, name)
}

#[cfg(test)]
pub(crate) mod support {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::cell::Cell;

    use super::super::unified_frame_codec::tests as frame_tests;
    use crate::persistence::recipe::fixture_recipe;

    #[derive(Debug)]
    pub(crate) struct TestContent {
        recipe: ExecutableRecipe,
    }

    impl UnifiedLoadedContent for TestContent {
        fn recipe(&self) -> &ExecutableRecipe {
            &self.recipe
        }

        fn map_sidecars(&self) -> Vec<super::super::unified_content::UnifiedLoadedSidecar> {
            Vec::new()
        }

        fn open_resource(
            &self,
            _content: &qa_content::contract::ContentId,
            path: &str,
        ) -> Result<Option<ResolvedResourceReference>, UnifiedContentError> {
            if path == self.recipe.map.geometry.requested_path {
                Ok(Some(self.recipe.map.geometry.clone()))
            } else {
                Ok(None)
            }
        }

        fn close(self) {}
    }

    impl UnifiedRemoteContent for TestContent {
        fn world_geometry(&self) -> &str {
            "world"
        }

        fn has_component_presentation(&self) -> bool {
            false
        }

        fn prepared_component_mods(&self) -> Vec<PreparedComponentMod> {
            Vec::new()
        }

        fn prepared_native_mods(&self) -> Vec<PreparedNativeMod> {
            Vec::new()
        }

        fn mounted_content(&self, _content: &str) -> Result<MountedContent, UnifiedRemoteError> {
            Err(UnifiedRemoteError::Remote("no mounts".to_string()))
        }

        fn product_edition(&self, _content: &str) -> Result<(GameFamily, String), UnifiedRemoteError> {
            Err(UnifiedRemoteError::Remote("no product".to_string()))
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct TestScene {
        links: Vec<ActorId>,
        unlinks: Vec<ActorId>,
    }

    impl UnifiedRemoteScene for TestScene {
        fn link(&mut self, body: &UnifiedLinkedBody, _collision: &UnifiedCollisionRecord) {
            self.links.push(body.actor.clone());
        }

        fn unlink(&mut self, actor: &ActorId) {
            self.unlinks.push(actor.clone());
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct TestMedia {
        received: Vec<UnifiedPresentationEvent>,
        local: f64,
    }

    impl UnifiedRemoteMedia for TestMedia {
        type Event = UnifiedPresentationEvent;

        fn receive_presentation(&mut self, event: UnifiedPresentationEvent) -> f64 {
            self.received.push(event);
            self.local += 1.0;
            self.local
        }

        fn take_presentation(&mut self) -> Vec<UnifiedPresentationEvent> {
            std::mem::take(&mut self.received)
        }

        fn admit_replicated_owner(&mut self, _owner: &PresentationOwner, _content: &str) {}

        fn retire_replicated_owner(&mut self, _owner: &PresentationOwner) {}
    }

    #[derive(Debug)]
    pub(crate) struct TestPredictor {
        sample: Option<UnifiedPredictedPlayer>,
        submitted: Vec<UnifiedRemotePredictionCommand>,
    }

    impl UnifiedRemotePredictor for TestPredictor {
        fn receive(&mut self, _projection: &UnifiedPredictionProjection) {}

        fn submit(&mut self, command: &UnifiedRemotePredictionCommand) {
            self.submitted.push(command.clone());
        }

        fn replay(&mut self) -> Option<UnifiedPredictedPlayer> {
            self.sample.clone()
        }
    }

    #[derive(Debug, Default)]
    pub(crate) struct TestHostState {
        pub(crate) published: Vec<UnifiedOutput>,
        pub(crate) commands: Vec<(String, Vec<String>)>,
        pub(crate) disconnected: Vec<String>,
        pub(crate) printed: Vec<String>,
    }

    pub(crate) struct TestHost {
        state: Rc<RefCell<TestHostState>>,
        generation: Cell<u32>,
        sample: Option<UnifiedPredictedPlayer>,
    }

    impl UnifiedRemoteHost for TestHost {
        type Content = TestContent;
        type Scene = TestScene;
        type Media = TestMedia;
        type Predictor = TestPredictor;

        fn load_content(&mut self, _offer: &UnifiedRemoteOffer) -> Result<TestContent, UnifiedRemoteError> {
            Ok(TestContent {
                recipe: fixture_recipe(),
            })
        }

        fn build_scene(_content: &TestContent) -> TestScene {
            TestScene::default()
        }

        fn create_media(&mut self) -> TestMedia {
            TestMedia::default()
        }

        fn create_predictor(
            &mut self,
            _actor: OwnedActor,
            _seat: &SeatId,
            _initial: &UnifiedPredictionProjection,
            _content: &TestContent,
        ) -> TestPredictor {
            TestPredictor {
                sample: self.sample.clone(),
                submitted: Vec::new(),
            }
        }

        fn model(
            &self,
            _key: &UnifiedResourceKey,
            _brush_model: Option<i64>,
        ) -> Result<UnifiedDecodedModel, UnifiedRemoteError> {
            Ok(UnifiedDecodedModel::Other)
        }

        fn publish(&mut self, output: &UnifiedOutput) {
            self.state.borrow_mut().published.push(output.clone());
        }

        fn send_command(&mut self, name: &str, args: &[String]) {
            self.state.borrow_mut().commands.push((name.to_string(), args.to_vec()));
        }

        fn send_component_command(
            &mut self,
            _owner: &PresentationOwner,
            _generation: i64,
            _args: &[String],
        ) -> Result<(), UnifiedRemoteError> {
            Err(UnifiedRemoteError::Remote(
                "Remote component command channel is unavailable".to_string(),
            ))
        }

        fn disconnected(&mut self, reason: &str) {
            self.state.borrow_mut().disconnected.push(reason.to_string());
        }

        fn print(&mut self, text: &str) {
            self.state.borrow_mut().printed.push(text.to_string());
        }

        fn next_generation(&self, _slot: u32) -> u32 {
            let next = self.generation.get() + 1;
            self.generation.set(next);
            next
        }
    }

    pub(crate) fn harness() -> (UnifiedRemotePresentation<TestHost>, Rc<RefCell<TestHostState>>) {
        let state = Rc::new(RefCell::new(TestHostState::default()));
        let identity = IdentityOwner::create("test").unwrap();
        let seat = IdentityOwner::create("seat").unwrap().seat(0);
        let client_id = identity.client(0, 1);
        let presentation = UnifiedRemotePresentation::new(UnifiedRemoteOptions {
            identity,
            client: SessionClient::new(client_id),
            seat,
            host: TestHost {
                state: state.clone(),
                generation: Cell::new(0),
                sample: None,
            },
            mod_files: None,
        });
        (presentation, state)
    }

    pub(crate) fn offer(epoch: u64) -> UnifiedControl {
        UnifiedControl::Offer {
            epoch,
            composition: super::super::unified_content::create_unified_composition(&fixture_recipe(), &[])
                .expect("fixture composition"),
            mode: UnifiedServerMode::Deathmatch,
            max_clients: 8,
        }
    }

    pub(crate) fn admitted(epoch: u64) -> UnifiedControl {
        UnifiedControl::Admitted {
            epoch,
            client: super::super::unified_control::UnifiedActorReference { slot: 0, generation: 1 },
            actor: super::super::unified_control::UnifiedActorReference { slot: 5, generation: 7 },
            source_entity: 5,
        }
    }

    pub(crate) fn frame_bytes(presentation: &UnifiedRemotePresentation<TestHost>, epoch: u64) -> Vec<u8> {
        let ledger = frame_tests::ledger();
        let mut frame = frame_tests::frame(&ledger);
        frame.epoch = epoch;
        // Encode the binding's wire reference; decode maps it back to the
        // locally minted actor, exactly like a server-issued frame.
        let actor = presentation.options.identity.actor(5, 7);
        frame.player.actor = actor.clone();
        frame.prediction.actor = actor;
        frame.prediction.sequence = frame.acknowledged_input;
        frame.output.snapshot.scene.world = Some(UnifiedSceneWorld {
            resource: fixture_recipe().map.geometry.clone(),
            geometry: "world".to_string(),
        });
        super::super::unified_frame_codec::encode_unified_frame(&frame).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::support::*;
    use super::*;
    use qa_core::identity::IdentityOwner;

    use super::super::unified_frame_codec::tests as frame_tests;
    use crate::persistence::recipe::fixture_recipe;

    #[test]
    fn offer_admits_world() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        assert_eq!(presentation.epoch(), 3);
        assert_eq!(presentation.loaded_epoch(), 3);
        let expected = super::super::unified_content::create_unified_composition(&fixture_recipe(), &[])
            .expect("fixture composition");
        assert_eq!(presentation.composition_digest(), Some(expected.digest.as_str()));
        assert!(presentation.output().is_none());
        assert!(presentation.player().is_none());
        assert!(presentation.command_time_milliseconds().is_none());
        assert!(presentation.sample_presentation(0).is_none());
    }

    #[test]
    fn second_offer_retires_first() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation.offer(&offer(4)).unwrap();
        assert_eq!(presentation.epoch(), 4);
        assert!(presentation.mod_presentation_sources().unwrap().is_empty());
    }

    #[test]
    fn declare_caches_matching_resource() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        let geometry = fixture_recipe().map.geometry.clone();
        let key = UnifiedResourceKey {
            content: qa_content::contract::ContentId("q3:classic:base:1".to_string()),
            path: geometry.requested_path.clone(),
            digest: geometry.digest.clone(),
            byte_length: geometry.byte_length,
        };
        presentation.declare(3, std::slice::from_ref(&key)).unwrap();
        presentation.declare(3, std::slice::from_ref(&key)).unwrap();
        presentation.declare(99, &[]).unwrap();
    }

    #[test]
    fn declare_rejects_unknown_resource() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        let key = UnifiedResourceKey {
            content: qa_content::contract::ContentId("q3:classic:base:1".to_string()),
            path: "maps/missing.bsp".to_string(),
            digest: "sha256:0".to_string(),
            byte_length: 8,
        };
        assert!(presentation.declare(3, &[key]).is_err());
    }

    #[test]
    fn empty_component_update_applies() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation
            .receive_components(
                3,
                &UnifiedComponentUpdate {
                    revision: 1,
                    sources: Vec::new(),
                    native: Vec::new(),
                },
            )
            .unwrap();
        presentation
            .receive_components(
                99,
                &UnifiedComponentUpdate {
                    revision: 2,
                    sources: Vec::new(),
                    native: Vec::new(),
                },
            )
            .unwrap();
        assert!(presentation.mod_client_presentation_sources().unwrap().is_empty());
    }

    #[test]
    fn frame_publishes_output_and_predicts() {
        let (mut presentation, state) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation.admitted(&admitted(3));
        let bytes = frame_bytes(&presentation, 3);
        let acknowledged = presentation.receive_frame(&bytes).unwrap();
        assert_eq!(acknowledged, Some(1));
        assert!(presentation.output().is_some());
        assert_eq!(state.borrow().published.len(), 1);
        let player = presentation.player().expect("player");
        assert_eq!(player.source_entity, 5);
        assert_eq!(presentation.number_of(&player.actor), Some(5));
        assert!(presentation.command_time_milliseconds().is_some());
        // Stale frames acknowledge without republishing.
        let acknowledged = presentation.receive_frame(&bytes).unwrap();
        assert_eq!(acknowledged, Some(1));
        assert_eq!(state.borrow().published.len(), 1);
    }

    #[test]
    fn frame_rejects_changed_player() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation.admitted(&admitted(3));
        let ledger = frame_tests::ledger();
        let mut frame = frame_tests::frame(&ledger);
        frame.epoch = 3;
        frame.output.snapshot.scene.world = Some(UnifiedSceneWorld {
            resource: fixture_recipe().map.geometry.clone(),
            geometry: "world".to_string(),
        });
        let bytes = super::super::unified_frame_codec::encode_unified_frame(&frame).unwrap();
        let error = presentation.receive_frame(&bytes).unwrap_err();
        assert_eq!(error.to_string(), "Unified frame changed the admitted player");
    }

    #[test]
    fn frame_before_binding_returns_none() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        let bytes = frame_bytes(&presentation, 3);
        assert_eq!(presentation.receive_frame(&bytes).unwrap(), None);
        let bytes = frame_bytes(&presentation, 99);
        presentation.admitted(&admitted(3));
        assert_eq!(presentation.receive_frame(&bytes).unwrap(), None);
    }

    #[test]
    fn empty_event_batches_release() {
        use qa_world::save::value::{arr, encode_checkpoint_value};
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation.admitted(&admitted(3));
        let payload = super::super::unified_event_codec::encode_unified_presentation_events(&[]).unwrap();
        let simulation = encode_checkpoint_value(&arr(vec![]));
        presentation.receive_events(3, 7, &payload, &simulation).unwrap();
        let bytes = frame_bytes(&presentation, 3);
        presentation.receive_frame(&bytes).unwrap();
        assert!(presentation.drain_simulation_events().is_empty());
        assert!(presentation.drain_presentation_events().is_empty());
    }

    #[test]
    fn player_command_routes_to_host() {
        let (mut presentation, state) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation.admitted(&admitted(3));
        let bytes = frame_bytes(&presentation, 3);
        presentation.receive_frame(&bytes).unwrap();
        let actor = presentation.player().expect("player").actor;
        presentation.player_command(&actor, "say", &["hi".to_string()]);
        assert_eq!(
            state.borrow().commands,
            vec![("say".to_string(), vec!["hi".to_string()])]
        );
        let ui = presentation.player_ui(&actor);
        assert_eq!(ui.health, 100.0);
        let view = presentation.player_view(&actor);
        assert_eq!(view.view_height, 22.0);
        // The helper frame carries scene entities but no models.
        assert!(presentation.presentations().is_empty());
        assert_eq!(presentation.output().expect("output").snapshot.scene.entities.len(), 1);
    }

    #[test]
    fn registered_resource_resolves() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        let geometry = fixture_recipe().map.geometry.clone();
        presentation.register_resource(&geometry).unwrap();
        let id = unified_resource_id_for_reference(&geometry).unwrap().to_string();
        let decoded: &dyn UnifiedIdentityDecoder = &presentation;
        assert_eq!(decoded.resource_id(&id), geometry.id);
    }

    #[test]
    fn close_resets_replica() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        presentation.admitted(&admitted(3));
        let bytes = frame_bytes(&presentation, 3);
        presentation.receive_frame(&bytes).unwrap();
        assert!(presentation.output().is_some());
        presentation.close();
        assert!(presentation.output().is_none());
        assert!(presentation.player().is_none());
        assert_eq!(presentation.loaded_epoch(), 0);
        assert!(presentation.offer(&offer(4)).is_err());
    }

    #[test]
    #[should_panic(expected = "Unified command belongs to another player")]
    fn foreign_player_command_panics() {
        let (mut presentation, _) = harness();
        presentation.offer(&offer(3)).unwrap();
        let foreign = IdentityOwner::create("foreign").unwrap().actor(1, 1);
        presentation.player_command(&foreign, "say", &[]);
    }
}
