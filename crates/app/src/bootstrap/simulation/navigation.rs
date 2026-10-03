//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/navigation.ts`.
//!
//! Application bot navigation: per-client [`NavigationRuntime`]s over a
//! shared loaded graph, with per-actor worlds whose prediction driver
//! replays detached player movement (`createPlayerMovementPrediction`)
//! and whose entity/hazard readers resolve live mover state.
//!
//! Donor behavior preserved:
//!
//! * [`bot_navigation_profile`] admits walk/crouch/jump/drop/swim/
//!   water-jump/ladder/mover with the standing box, the crouched box, and
//!   the player trace policy (crouch only for q2-classic, q2-rerelease,
//!   and q3 movement).
//! * [`create_application_bot_navigation`] rejects content whose recipe
//!   differs from the simulation recipe, snapshots live players (Q3-guest
//!   projections when the guest arm is active, simulation players
//!   otherwise), loads navigation for the map geometry, and caches one
//!   runtime per client keyed by actor identity plus locomotion.
//! * [`ApplicationBotNavigation::restart_round`] rebuilds the base runtime
//!   and clears per-client runtimes; [`checkpoint`](ApplicationBotNavigation::checkpoint)
//!   captures `{ version: 1, base, clients[] }` with per-client reusable
//!   flags; [`restore_checkpoint`](ApplicationBotNavigation::restore_checkpoint)
//!   validates the version, the client mapping, and every nested runtime
//!   checkpoint before committing.
//! * [`ApplicationBotNavigation::predict_client_movement`] ports donor
//!   `predictApplicationBotMovement` (`bot-prediction.ts`): detached
//!   prediction with crouched-presence input, AAS area-stop checks, and
//!   the ground/liquid/stop-area/fall stop flags.
//! * Train connections port donor `applicationTrainConnections`
//!   (`bot-mover-connections.ts`): Q2 corner routes become mover edges
//!   with riding endpoints and per-stop travel seconds.
//! * [`ApplicationBotNavigation::travel_weapon`] returns `None`, matching
//!   the donor.
//!
//! Rust adaptations (all donor-equivalent):
//!
//! * The donor is async over its mount store; this port is synchronous
//!   over the [`ApplicationNavigationContent`] seam, matching the
//!   `NavigationResources` seam style of the `qa_bots` loader.
//! * `NavigationRuntime` borrows its world, so runtimes cannot be owned
//!   beside their worlds. This port owns the graph, the worlds, and the
//!   persisted [`NavigationRuntimeCheckpoint`] per runtime, and
//!   materializes a runtime per access
//!   ([`with_runtime`](ApplicationBotNavigation::with_runtime),
//!   [`with_client_runtime`](ApplicationBotNavigation::with_client_runtime)),
//!   persisting the checkpoint back afterwards. The checkpoint carries
//!   the complete mutable runtime state, so observed behavior matches
//!   the donor's long-lived runtimes.
//! * The donor caches guest projections by object identity and compares
//!   cached players with `===`. Snapshots here are owned values rebuilt
//!   per call, so identity is (client slot, actor id) plus
//!   [`player_locomotion_matches`]; reusable flags and cache hits follow
//!   the same rule.
//! * The donor driver override passes the admitted profile's movement
//!   into prediction; each world stores its own application movement
//!   profile instead (callers always admit with the world's own graph
//!   profile, which was built from that same application profile).
//! * Prediction input for client movement goes through the public
//!   [`NavigationPrediction`] input with a synthesized traversal request:
//!   the endpoint is offset by command/20 so the relocated driver math
//!   reproduces the donor wish vector exactly, and crouched presence
//!   selects [`TravelMode::Crouch`] for the donor's -400 z command.
//! * Checkpoint client lists are sorted by slot (the donor keeps map
//!   insertion order; the cache here is a `HashMap`).
//! * `SelectedBotNavigation` (donor `Pick<SourceBotNavigationHost,
//!   "runtime" | "forClient" | "crouchedBounds" | "predictClientMovement"
//!   | "travelWeapon">`, donor navigation.ts:39-43) has no worktree home:
//!   the five members live here as inherent methods with Rust-adapted
//!   shapes (borrowing
//!   [`with_runtime`](ApplicationBotNavigation::with_runtime) and
//!   [`with_client_runtime`](ApplicationBotNavigation::with_client_runtime)
//!   instead of `runtime`/`forClient`, plus `crouched_bounds`,
//!   `predict_client_movement`, and `travel_weapon`).
//!
//! Sibling homes:
//!
//! * [`LoadedApplicationContent`](crate::bootstrap::content::LoadedApplicationContent)
//!   (donor `src/app/bootstrap/content.ts`) behind
//!   [`ApplicationNavigationContent`]; decoded-geometry supply arrives
//!   through the seam (the donor reads `simulation.options.world`,
//!   which has no worktree accessor).
//! * Keyed Q1 doors read the bound holder's live key count through the
//!   Q1 source view
//!   ([`Q1SharedInventoryTable::count`](qa_content::q1::foundation::host::Q1SharedInventoryTable::count))
//!   in `keyed_door_needs_key`; the shared table home is
//!   [`count`](qa_world::inventory::InventoryTable::count).
//! * Q2 expansion movers read the assembled product from the game arena
//!   (`Q2GameServices::composition.product`, donor `q2.product`)
//!   with Rogue platform state
//!   (`Q2RogueMovers::platform_state`, donor
//!   `expansion.entities.movers?.platformState`); the `SharedSimulation`
//!   product seam stays identity-only.
//!
//! Scene actor rows flow from the shared tables: world construction
//! installs a rows provider over the live actor/body handles, so
//! `queryActors` answers linked bodies and the hazard loop below reads
//! live `trigger_hurt` state. Rows carry box shapes (collision shapes
//! live in the physics lane's spatial table, which has no accessor),
//! so entity/train model matching stays gated on `Model` rows.
//!
//! Missing siblings (no Rust home in this worktree; unify post-merge):
//!
//! * Scene collision shapes: entity/train matching needs `Model` rows
//!   from the physics lane's spatial table; box rows keep those loops
//!   inert while hazard reads are live.
//! * Q3 native entity records (donor `records.nativeByActor`, mover
//!   state, door triggers, `trigger_hurt`): the Q3 entity/hazard arms
//!   land with the records home.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_bots::aas_prediction_stop::aas_prediction_stop;
use qa_bots::behavior::prediction::{project_bot_movement, BotMovementProjection};
use qa_bots::behavior::{BotMovementPrediction, BotMovementStop, BotTravelPredictionResult};
use qa_bots::construct::{NavigationConnection, NavigationConstruction};
use qa_bots::content::{ContentDigest, ContentId as BotsContentId, NavigationResources, OpenedResource};
use qa_bots::entity_binding::source_mover_bounds_match;
use qa_bots::error::BotsError;
use qa_bots::load::{load_prepared_navigation, preload_navigation, PreloadOptions};
use qa_bots::movement::{
    create_movement_admission, create_movement_route_admission, NavigationPrediction, NavigationPredictionDriver,
    NavigationPredictionLimits,
};
use qa_bots::movement_contract::MovementProfile as BotsMovementProfile;
use qa_bots::movement_contract::{MovementInput, MovementKind, MovementProvider, MovementResult, MovementServices};
use qa_bots::runtime::{NavigationRuntime, NavigationRuntimeCheckpoint};
use qa_bots::save::{SaveError, SaveValue};
use qa_bots::scene::{
    BodyShape, DecodedWorld, LeafQueryResult, PointContentsQuery, PointContentsResult, SceneQueries, TraceContact,
    TraceDetail, TraceHit, TraceQuery, TraceResult, VisibilityKind,
};
use qa_bots::types::{
    ElevatorPhase, ElevatorState, NavigationAsset, NavigationEntityBinding, NavigationEntityState, NavigationGraph,
    NavigationMapIdentity, NavigationProfile, NavigationRoutePrediction, NavigationWorld, TrainState, TrainStop,
    TravelMode, TraversalAdmission, TraversalRequest,
};
use qa_bots::{ResourceProvenance as BotsProvenance, ResourceReference as BotsResourceReference};
use qa_content::contract::{ContentId, ExecutableRecipe, ResolvedResourceReference};
use qa_content::mounts::MountedContent;
use qa_content::q1::foundation::entity::{Q1Actor, Q1MoverState};
use qa_content::q1::foundation::types::Q1Solid;
use qa_content::q2::base::entities::movers::{mover_platform_state, mover_traversal, Q2PlatformPhase, Q2PlatformState};
use qa_content::q2::foundation::host::{Q2Edition, Q2Entity, Q2GameServices, Q2Solid};
use qa_content::q2::foundation::movers::{create_q2_mover_module, Q2MoverModule, Q2TrainRoute};
use qa_content::q2::missionpacks::entities::movers::{Q2Plat2Phase, Q2RogueMovers};
use qa_content::q2::missionpacks::types::Q2MissionPack;
use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericProfile;
use thiserror::Error;

use super::player_input_application::{MovementProfile, MovementState};
use super::player_movement::{
    capture_player_locomotion, create_player_movement_prediction, locomotion_template, movement_observation,
    player_crouched_bounds, player_locomotion_matches, player_trace_policy, selected_movement_profile,
    LocomotionPlayer, MovementMedium, MovementPredictionPlayer,
};
use super::q3::guest_movement::guest_movement_projection;
use super::runtime::{
    ClientMovementOptions, SceneActorHit, SceneBody, SceneCollision, SceneCollisionShape, SceneRowProvider,
    SharedSimulation,
};
use crate::bootstrap::simulation::q3::guest_runtime::Q3ApplicationPlayer;

/// Zero vector for prediction input origins.
const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };

/// Application navigation failure.
#[derive(Debug, Error)]
pub enum NavigationError {
    /// Content and simulation recipes differ (donor `Error("Navigation
    /// and simulation must use the same loaded recipe")`).
    #[error("Navigation and simulation must use the same loaded recipe")]
    RecipeMismatch,
    /// Navigation client is not admitted (donor `Error("Navigation
    /// client ${client} is not admitted")`).
    #[error("Navigation client {0} is not admitted")]
    UnknownClient(i32),
    /// Checkpoint value is invalid.
    #[error(transparent)]
    Checkpoint(#[from] SaveError),
    /// Navigation load, construction, or prediction failed.
    #[error(transparent)]
    Navigation(#[from] BotsError),
    /// Mounted content access failed.
    #[error("navigation content: {0}")]
    Content(String),
}

/// Loaded-content surface for application navigation.
///
/// Seam over `LoadedApplicationContent` from donor
/// `src/app/bootstrap/content.ts` (canonical home: the content
/// partition); the partition implements it post-merge. Synchronous:
/// the donor awaits its mount store, this port reads mounted views.
pub trait ApplicationNavigationContent {
    /// Loaded executable recipe (donor `content.recipe`).
    fn recipe(&self) -> ExecutableRecipe;
    /// Decoded map geometry (donor `content.world`, read from
    /// `simulation.options.world` by the donor loader call).
    fn geometry(&self) -> Rc<DecodedWorld>;
    /// Mounted resources for the geometry content (donor
    /// `content.forContent(geometry.provenance.mount.identity.content)`).
    fn geometry_resources(&self) -> Result<MountedContent, NavigationError>;
    /// Selected map bytes for AAS checksum verification (donor
    /// `content.mounts.read(content.recipe.map.geometry)`).
    fn map_bytes(&self) -> Result<Vec<u8>, NavigationError>;
}

/// Traversal profile for a locomotion slice (donor
/// `botNavigationProfile`).
#[must_use]
pub fn bot_navigation_profile(player: &LocomotionPlayer) -> NavigationProfile {
    let movement = selected_movement_profile(player, None);
    let crouches = matches!(
        movement,
        MovementProfile::Q2Classic(_) | MovementProfile::Q2Rerelease(_) | MovementProfile::Q3(_)
    );
    let mut capabilities = HashSet::from([
        TravelMode::Walk,
        TravelMode::Jump,
        TravelMode::Drop,
        TravelMode::Swim,
        TravelMode::WaterJump,
        TravelMode::Ladder,
        TravelMode::Mover,
    ]);
    if crouches {
        capabilities.insert(TravelMode::Crouch);
    }
    NavigationProfile {
        movement: bots_movement_profile(&movement),
        shape: BodyShape::Box(player.standing_bounds),
        crouched_shape: crouches.then(|| BodyShape::Box(player_crouched_bounds(player))),
        policy: player_trace_policy(player),
        capabilities,
        maximum_step: 18.0,
        minimum_floor_normal: 0.7,
        maximum_drop: 128.0,
        team: None,
        monster: false,
    }
}

/// Project the rich application movement profile onto the
/// navigation-visible movement profile (family, provider, numerics).
fn bots_movement_profile(selected: &MovementProfile) -> BotsMovementProfile {
    let (kind, id, numeric) = match selected {
        MovementProfile::Q1Netquake(profile) => (MovementKind::Q1Netquake, &profile.id, &profile.numeric),
        MovementProfile::Q1Quakeworld(profile) => (MovementKind::Q1Quakeworld, &profile.id, &profile.numeric),
        MovementProfile::Q2Classic(profile) => (MovementKind::Q2Classic, &profile.id, &profile.numeric),
        MovementProfile::Q2Rerelease(profile) => (MovementKind::Q2Rerelease, &profile.id, &profile.numeric),
        MovementProfile::Q3(profile) => (MovementKind::Q3, &profile.id, &profile.numeric),
    };
    BotsMovementProfile {
        kind,
        id: id.clone(),
        numeric: *numeric,
    }
}

/// Numeric profile carried by a rich application movement profile.
fn profile_numeric(selected: &MovementProfile) -> NumericProfile {
    match selected {
        MovementProfile::Q1Netquake(profile) => profile.numeric,
        MovementProfile::Q1Quakeworld(profile) => profile.numeric,
        MovementProfile::Q2Classic(profile) => profile.numeric,
        MovementProfile::Q2Rerelease(profile) => profile.numeric,
        MovementProfile::Q3(profile) => profile.numeric,
    }
}

/// Per-client runtime checkpoint (donor
/// `ApplicationBotNavigationCheckpoint["clients"][number]`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationBotNavigationClientCheckpoint {
    /// Client slot.
    pub client: i32,
    /// Whether the saved player still matches the live player.
    pub reusable: bool,
    /// Client runtime checkpoint.
    pub runtime: NavigationRuntimeCheckpoint,
}

/// Application navigation checkpoint (donor
/// `ApplicationBotNavigationCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationBotNavigationCheckpoint {
    /// Checkpoint version (always 1).
    pub version: i32,
    /// Base runtime checkpoint.
    pub base: NavigationRuntimeCheckpoint,
    /// Per-client runtime checkpoints by slot.
    pub clients: Vec<ApplicationBotNavigationClientCheckpoint>,
}

impl ApplicationBotNavigationCheckpoint {
    /// Encode as a checkpoint value.
    #[must_use]
    pub fn to_save_value(&self) -> SaveValue {
        SaveValue::map(vec![
            ("version", SaveValue::Int(i64::from(self.version))),
            ("base", self.base.to_save_value()),
            (
                "clients",
                SaveValue::List(
                    self.clients
                        .iter()
                        .map(|client| {
                            SaveValue::map(vec![
                                ("client", SaveValue::Int(i64::from(client.client))),
                                ("reusable", SaveValue::Bool(client.reusable)),
                                ("runtime", client.runtime.to_save_value()),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

/// Borrow a record field, failing with a dotted path.
fn checkpoint_field<'v>(value: &'v SaveValue, path: &str, name: &str) -> Result<&'v SaveValue, SaveError> {
    let SaveValue::Map(map) = value else {
        return Err(SaveError {
            path: path.to_string(),
            message: "expected a record".to_string(),
        });
    };
    map.get(name).ok_or_else(|| SaveError {
        path: format!("{path}.{name}"),
        message: "missing field".to_string(),
    })
}

/// Read an integer field with a minimum, mirroring `SaveReader`.
fn checkpoint_integer(value: &SaveValue, path: &str, minimum: i64) -> Result<i64, SaveError> {
    let SaveValue::Int(raw) = value else {
        return Err(SaveError {
            path: path.to_string(),
            message: "expected an integer".to_string(),
        });
    };
    if *raw < minimum {
        return Err(SaveError {
            path: path.to_string(),
            message: format!("expected an integer at least {minimum}"),
        });
    }
    Ok(*raw)
}

/// Read a boolean field, mirroring `SaveReader`.
fn checkpoint_boolean(value: &SaveValue, path: &str) -> Result<bool, SaveError> {
    let SaveValue::Bool(flag) = value else {
        return Err(SaveError {
            path: path.to_string(),
            message: "expected a boolean".to_string(),
        });
    };
    Ok(*flag)
}

/// Application navigation options (donor
/// `ApplicationBotNavigationOptions`).
pub struct ApplicationBotNavigationOptions<C> {
    /// Loaded application content.
    pub content: C,
    /// Owning simulation.
    pub simulation: SharedSimulation,
}

/// Installed application bot navigation (donor
/// `ApplicationBotNavigation`).
///
/// Clone shares the installation (donor object identity); equality is
/// handle identity, so reinstalling the attached navigation is
/// idempotent while a foreign installation is rejected.
#[derive(Clone)]
pub struct ApplicationBotNavigation {
    /// Shared installation state.
    inner: Rc<RefCell<ApplicationBotNavigationInner>>,
}

impl PartialEq for ApplicationBotNavigation {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl std::fmt::Debug for ApplicationBotNavigation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("ApplicationBotNavigation")
            .field("map", &inner.map.name)
            .field("clients", &inner.clients.len())
            .finish()
    }
}

/// Shared installation state behind [`ApplicationBotNavigation`].
struct ApplicationBotNavigationInner {
    /// Owning simulation.
    simulation: SharedSimulation,
    /// Loaded recipe (equals the simulation recipe).
    recipe: ExecutableRecipe,
    /// Loaded graph template (profiles are replaced per runtime).
    graph: NavigationGraph,
    /// Loaded map identity.
    map: NavigationMapIdentity,
    /// Base traversal profile.
    base_profile: NavigationProfile,
    /// Base navigation world.
    base_world: Rc<ApplicationNavigationWorld>,
    /// Persisted base runtime state.
    base_checkpoint: NavigationRuntimeCheckpoint,
    /// Per-client cached runtimes by slot.
    clients: HashMap<i32, ApplicationNavigationClient>,
    /// Crouched bounds captured at creation (donor `crouchedBounds`).
    crouched_bounds: Bounds,
}

/// Cached per-client navigation state (donor `clients` entry).
struct ApplicationNavigationClient {
    /// Navigating actor at cache time, when the player was reusable.
    actor: Option<OwnedActor>,
    /// Locomotion snapshot at cache time.
    locomotion: LocomotionPlayer,
    /// Client traversal profile.
    profile: NavigationProfile,
    /// Client navigation world.
    world: Rc<ApplicationNavigationWorld>,
    /// Persisted client runtime state.
    checkpoint: NavigationRuntimeCheckpoint,
}

/// Create application bot navigation (donor
/// `createApplicationBotNavigation`).
pub fn create_application_bot_navigation<C: ApplicationNavigationContent>(
    options: ApplicationBotNavigationOptions<C>,
) -> Result<ApplicationBotNavigation, NavigationError> {
    let recipe = options.content.recipe();
    if recipe != options.simulation.recipe() {
        return Err(NavigationError::RecipeMismatch);
    }
    let players = live_players(&options.simulation, &recipe);
    let first = players.first();
    let template = locomotion_template(&recipe);
    let first_locomotion = first.map_or(template.clone(), |player| player.locomotion());
    let profile = bot_navigation_profile(&first_locomotion);
    let geometry = options.content.geometry();
    let map = NavigationMapIdentity {
        name: recipe.map.geometry.requested_path.clone(),
        format: geometry.kind(),
        digest: ContentDigest::new(&recipe.map.geometry.digest.to_string()),
    };
    let resources = ApplicationNavigationResources {
        mounted: options.content.geometry_resources()?,
    };
    let map_bytes = options.content.map_bytes()?;
    let navigation_content = BotsContentId::new(&geometry_content(&recipe.map.geometry).to_string());
    let prepared = preload_navigation(&PreloadOptions {
        map: &map,
        resources: &resources,
        map_bytes: &map_bytes,
        navigation_content: Some(&navigation_content),
    })?;
    let connections = train_connections(&options.simulation, &profile);
    let base_selected = first.cloned();
    let base_app_profile = base_selected
        .as_ref()
        .map_or(template.profile.clone(), |player| player.profile.clone());
    let base_world = Rc::new(ApplicationNavigationWorld::new(
        options.simulation.clone(),
        recipe.clone(),
        base_selected,
        base_app_profile,
    ));
    let prepared_map = prepared.map.clone();
    let construction = NavigationConstruction {
        geometry: &geometry,
        map: &prepared_map,
        profile: &profile,
        world: base_world.as_ref(),
        spacing: None,
        link_distance: None,
        maximum_nodes: None,
        connections: Some(&connections),
    };
    let loaded = load_prepared_navigation(&construction, prepared)?;
    let graph = loaded.runtime.graph;
    let mut graph_for_base = graph.clone();
    graph_for_base.profile = profile.clone();
    let base_checkpoint = NavigationRuntime::new(graph_for_base, base_world.as_ref())?.checkpoint();
    let crouched_bounds = profile
        .crouched_shape
        .as_ref()
        .map_or(first_locomotion.standing_bounds, BodyShape::bounds);
    Ok(ApplicationBotNavigation {
        inner: Rc::new(RefCell::new(ApplicationBotNavigationInner {
            simulation: options.simulation,
            recipe,
            graph,
            map,
            base_profile: profile,
            base_world,
            base_checkpoint,
            clients: HashMap::new(),
            crouched_bounds,
        })),
    })
}

/// Geometry provenance mount content (donor
/// `geometry.provenance.mount.identity.content`).
fn geometry_content(geometry: &ResolvedResourceReference) -> ContentId {
    match &geometry.provenance {
        qa_content::contract::ResourceProvenance::Archive { mount, .. } => mount.identity.content.clone(),
        qa_content::contract::ResourceProvenance::Loose { mount, .. } => mount.identity.content.clone(),
    }
}

/// [`NavigationResources`] over a mounted content view.
struct ApplicationNavigationResources {
    /// Mounted geometry resources.
    mounted: MountedContent,
}

impl NavigationResources for ApplicationNavigationResources {
    fn open(&self, path: &str) -> Option<OpenedResource> {
        let opened = self.mounted.open(path, |_| true).ok()??;
        let mount_content = match &opened.reference.provenance {
            qa_content::contract::ResourceProvenance::Archive { mount, .. } => mount.identity.content.to_string(),
            qa_content::contract::ResourceProvenance::Loose { mount, .. } => mount.identity.content.to_string(),
        };
        Some(OpenedResource {
            reference: BotsResourceReference {
                requested_path: opened.reference.requested_path.clone(),
                provenance: BotsProvenance {
                    mount_content: BotsContentId::new(&mount_content),
                },
                digest: ContentDigest::new(&opened.reference.digest.to_string()),
                byte_length: opened.reference.byte_length as usize,
            },
            bytes: opened.bytes,
        })
    }
}

/// Travel-weapon jump mode (donor `travelWeapon` mode union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TravelWeaponMode {
    /// Rocket jump.
    RocketJump,
    /// BFG jump.
    BfgJump,
    /// Grapple.
    Grapple,
}

impl ApplicationBotNavigation {
    /// Run a closure against the base runtime (donor `runtime`).
    ///
    /// The runtime is materialized from the persisted checkpoint and
    /// the checkpoint is persisted back afterwards; observed behavior
    /// matches the donor's long-lived base runtime.
    pub fn with_runtime<R>(&self, f: impl FnOnce(&mut NavigationRuntime) -> R) -> R {
        let mut inner = self.inner.borrow_mut();
        let (output, checkpoint) = {
            let world = Rc::clone(&inner.base_world);
            let mut runtime = materialize_runtime(&inner.graph, &inner.base_profile, &world, &inner.base_checkpoint);
            let output = f(&mut runtime);
            let checkpoint = runtime.checkpoint();
            (output, checkpoint)
        };
        inner.base_checkpoint = checkpoint;
        output
    }

    /// Run a closure against a client runtime (donor `forClient`).
    ///
    /// The cached runtime is reused when the live player matches the
    /// cached actor and locomotion; otherwise a fresh client world and
    /// runtime are installed. The checkpoint is persisted back
    /// afterwards.
    pub fn with_client_runtime<R>(
        &self,
        client: i32,
        f: impl FnOnce(&mut NavigationRuntime) -> R,
    ) -> Result<R, NavigationError> {
        let player = self.player_for(client)?;
        let mut inner = self.inner.borrow_mut();
        let hit = inner.clients.get(&client).is_some_and(|entry| {
            entry
                .actor
                .as_ref()
                .is_some_and(|actor| actor.id() == player.actor.id())
                && player_locomotion_matches(&player.locomotion(), &entry.locomotion)
        });
        if !hit {
            let profile = bot_navigation_profile(&player.locomotion());
            let world = Rc::new(ApplicationNavigationWorld::new(
                inner.simulation.clone(),
                inner.recipe.clone(),
                Some(player.clone()),
                player.profile.clone(),
            ));
            let mut graph = inner.graph.clone();
            graph.profile = profile.clone();
            let checkpoint = NavigationRuntime::new(graph, world.as_ref())?.checkpoint();
            inner.clients.insert(
                client,
                ApplicationNavigationClient {
                    actor: Some(player.actor.clone()),
                    locomotion: capture_player_locomotion(&player.locomotion()),
                    profile,
                    world,
                    checkpoint,
                },
            );
        }
        let (output, checkpoint) = {
            let entry = inner.clients.get(&client).expect("cached client navigation");
            let world = Rc::clone(&entry.world);
            let profile = entry.profile.clone();
            let checkpoint = entry.checkpoint.clone();
            let mut runtime = materialize_runtime(&inner.graph, &profile, &world, &checkpoint);
            let output = f(&mut runtime);
            let checkpoint = runtime.checkpoint();
            (output, checkpoint)
        };
        if let Some(entry) = inner.clients.get_mut(&client) {
            entry.checkpoint = checkpoint;
        }
        Ok(output)
    }

    /// Resolve an admitted navigation client (donor `playerFor`).
    fn player_for(&self, client: i32) -> Result<MovementPredictionPlayer, NavigationError> {
        let inner = self.inner.borrow();
        live_players(&inner.simulation, &inner.recipe)
            .into_iter()
            .find(|player| client >= 0 && player.client.slot() == client as u32)
            .ok_or(NavigationError::UnknownClient(client))
    }

    /// Rebuild the base runtime and clear client runtimes (donor
    /// `restartRound`).
    pub fn restart_round(&self) {
        let mut inner = self.inner.borrow_mut();
        let players = live_players(&inner.simulation, &inner.recipe);
        let template = locomotion_template(&inner.recipe);
        let selected = players.first().map_or(template.clone(), |player| player.locomotion());
        let profile = bot_navigation_profile(&selected);
        let world = Rc::new(ApplicationNavigationWorld::new(
            inner.simulation.clone(),
            inner.recipe.clone(),
            None,
            players
                .first()
                .map_or(template.profile.clone(), |player| player.profile.clone()),
        ));
        let mut graph = inner.graph.clone();
        graph.profile = profile.clone();
        let checkpoint = NavigationRuntime::new(graph, world.as_ref())
            .expect("rebuild application base navigation")
            .checkpoint();
        inner.base_profile = profile;
        inner.base_world = world;
        inner.base_checkpoint = checkpoint;
        inner.clients.clear();
    }

    /// Capture the navigation checkpoint (donor `checkpoint`).
    pub fn checkpoint(&self) -> ApplicationBotNavigationCheckpoint {
        let inner = self.inner.borrow();
        let players = live_players(&inner.simulation, &inner.recipe);
        let mut clients: Vec<ApplicationBotNavigationClientCheckpoint> = inner
            .clients
            .iter()
            .map(|(client, entry)| {
                let current = players
                    .iter()
                    .find(|player| *client >= 0 && player.client.slot() == *client as u32);
                let reusable = current.is_some_and(|player| {
                    entry
                        .actor
                        .as_ref()
                        .is_some_and(|actor| actor.id() == player.actor.id())
                        && player_locomotion_matches(&player.locomotion(), &entry.locomotion)
                });
                ApplicationBotNavigationClientCheckpoint {
                    client: *client,
                    reusable,
                    runtime: entry.checkpoint.clone(),
                }
            })
            .collect();
        clients.sort_by_key(|client| client.client);
        ApplicationBotNavigationCheckpoint {
            version: 1,
            base: inner.base_checkpoint.clone(),
            clients,
        }
    }

    /// Restore a navigation checkpoint (donor `restoreCheckpoint`).
    ///
    /// `remap_client` maps saved slots to live slots (donor
    /// `remapClient`, default identity). Nothing is committed unless
    /// every nested checkpoint validates.
    pub fn restore_checkpoint(
        &self,
        value: &SaveValue,
        remap_client: Option<&dyn Fn(i32) -> i32>,
    ) -> Result<(), NavigationError> {
        let remap = remap_client.unwrap_or(&|client| client);
        let version_value = checkpoint_field(value, "applicationNavigation", "version")?;
        let version = checkpoint_integer(version_value, "applicationNavigation.version", 0)?;
        if version != 1 {
            return Err(SaveError {
                path: "applicationNavigation.version".to_string(),
                message: "expected 1".to_string(),
            }
            .into());
        }
        let base_value = checkpoint_field(value, "applicationNavigation", "base")?;
        let clients_value = checkpoint_field(value, "applicationNavigation", "clients")?;
        let SaveValue::List(entries) = clients_value else {
            return Err(SaveError {
                path: "applicationNavigation.clients".to_string(),
                message: "expected a list".to_string(),
            }
            .into());
        };
        let mut inner = self.inner.borrow_mut();
        let mut base_graph = inner.graph.clone();
        base_graph.profile = inner.base_profile.clone();
        let base_world = Rc::clone(&inner.base_world);
        let mut base = NavigationRuntime::new(base_graph, base_world.as_ref())?;
        base.restore_checkpoint(base_value)?;
        let template = locomotion_template(&inner.recipe);
        let mut restored = HashMap::new();
        let mut saved_clients = HashSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let path = format!("applicationNavigation.clients[{index}]");
            let saved = checkpoint_field(entry, &path, "client")
                .and_then(|value| checkpoint_integer(value, &format!("{path}.client"), 0))?;
            let Ok(saved_slot) = i32::try_from(saved) else {
                return Err(SaveError {
                    path: format!("{path}.client"),
                    message: "invalid or duplicate navigation client mapping".to_string(),
                }
                .into());
            };
            let client = remap(saved_slot);
            if client < 0 || !saved_clients.insert(saved_slot) || restored.contains_key(&client) {
                return Err(SaveError {
                    path: format!("{path}.client"),
                    message: "invalid or duplicate navigation client mapping".to_string(),
                }
                .into());
            }
            let reusable = checkpoint_field(entry, &path, "reusable")
                .and_then(|value| checkpoint_boolean(value, &format!("{path}.reusable")))?;
            let player = if reusable {
                Some(
                    live_players(&inner.simulation, &inner.recipe)
                        .into_iter()
                        .find(|player| player.client.slot() == client as u32)
                        .ok_or(NavigationError::UnknownClient(client))?,
                )
            } else {
                None
            };
            let locomotion = player.as_ref().map_or(template.clone(), |player| player.locomotion());
            let profile = bot_navigation_profile(&locomotion);
            let world = Rc::new(ApplicationNavigationWorld::new(
                inner.simulation.clone(),
                inner.recipe.clone(),
                player.clone(),
                player
                    .as_ref()
                    .map_or(template.profile.clone(), |player| player.profile.clone()),
            ));
            let mut graph = inner.graph.clone();
            graph.profile = profile.clone();
            let mut runtime = NavigationRuntime::new(graph, world.as_ref())?;
            let runtime_value = checkpoint_field(entry, &path, "runtime")?;
            runtime.restore_checkpoint(runtime_value)?;
            let checkpoint = runtime.checkpoint();
            restored.insert(
                client,
                ApplicationNavigationClient {
                    actor: player.as_ref().map(|player| player.actor.clone()),
                    locomotion: capture_player_locomotion(&locomotion),
                    profile,
                    world,
                    checkpoint,
                },
            );
        }
        let base_checkpoint = base.checkpoint();
        inner.base_checkpoint = base_checkpoint;
        inner.clients = restored;
        Ok(())
    }

    /// Crouched bounds captured at creation (donor `crouchedBounds`).
    #[must_use]
    pub fn crouched_bounds(&self) -> Bounds {
        self.inner.borrow().crouched_bounds
    }

    /// Travel weapon for a jump mode (donor `travelWeapon`, always null).
    #[must_use]
    pub fn travel_weapon(&self, _client: i32, _mode: TravelWeaponMode) -> Option<i32> {
        None
    }

    /// Predict client movement (donor `predictClientMovement`, via
    /// donor `predictApplicationBotMovement` from `bot-prediction.ts`).
    pub fn predict_client_movement(
        &self,
        query: &BotMovementPrediction,
    ) -> Result<BotTravelPredictionResult, NavigationError> {
        let player = self.player_for(query.entity_num)?;
        let simulation = self.inner.borrow().simulation.clone();
        let predicted = self.with_client_runtime(query.entity_num, |runtime| {
            let prediction = create_player_movement_prediction(
                &simulation,
                &player,
                query.origin,
                query.velocity,
                (f64::from(query.frame_time) * 1000.0).round() as i32,
                query.presence == 4,
                None,
            );
            let asset = runtime.graph.asset.clone();
            let projection = ApplicationBotProjection {
                prediction: &prediction,
                query,
                asset: asset.as_ref(),
                runtime,
            };
            project_bot_movement(query, &projection)
        })?;
        Ok(predicted?)
    }
}

/// Materialize a runtime from a graph template, profile, world, and
/// persisted checkpoint.
fn materialize_runtime<'w>(
    graph: &NavigationGraph,
    profile: &NavigationProfile,
    world: &'w Rc<ApplicationNavigationWorld>,
    checkpoint: &NavigationRuntimeCheckpoint,
) -> NavigationRuntime<'w> {
    let mut graph = graph.clone();
    graph.profile = profile.clone();
    let mut runtime = NavigationRuntime::new(graph, world.as_ref()).expect("rebuild application navigation runtime");
    runtime
        .restore_checkpoint(&checkpoint.to_save_value())
        .expect("restore application navigation checkpoint");
    runtime
}

/// Live prediction players (donor `players`).
fn live_players(simulation: &SharedSimulation, recipe: &ExecutableRecipe) -> Vec<MovementPredictionPlayer> {
    if simulation.has_q3_guest() {
        return simulation
            .with_q3_guest(|guest| {
                guest
                    .players()
                    .into_iter()
                    .map(|player| guest_snapshot(guest, &player, recipe))
                    .collect()
            })
            .unwrap_or_default();
    }
    simulation
        .players()
        .into_iter()
        .filter_map(|actor| {
            let player = simulation.player(&actor)?;
            let numeric = profile_numeric(&player.profile);
            Some(MovementPredictionPlayer::from_player(&player, numeric, None))
        })
        .collect()
}

/// Snapshot a guest player for prediction (donor
/// `guestMovementProjection`, resolved to owned values).
fn guest_snapshot(
    guest: &super::q3::guest_runtime::Q3QvmServerGame,
    player: &Q3ApplicationPlayer,
    recipe: &ExecutableRecipe,
) -> MovementPredictionPlayer {
    let template = locomotion_template(recipe);
    let projection = guest_movement_projection(guest, player, recipe);
    let source_movement = projection.source_movement();
    MovementPredictionPlayer {
        client: projection.client.clone(),
        actor: projection.actor.clone(),
        profile: template.profile.clone(),
        standing_bounds: projection.standing_bounds,
        bounds: projection.bounds(),
        source_movement: Some(ClientMovementOptions {
            trace_mask: Some(source_movement.trace_mask),
            fixed_msec: source_movement.fixed_msec,
            no_footsteps: Some(source_movement.no_footsteps),
            gauntlet_hit: Some(source_movement.gauntlet_hit),
        }),
        character: projection.character,
        world_gravity: f64::from(projection.world_gravity()),
        q2_movement_config: None,
        flight: projection.flight(),
        source_environment: Some(projection.source_environment()),
        gravity_multiplier: projection.gravity_multiplier,
        movement_speed_multiplier: projection.movement_speed_multiplier,
        state: MovementState::Q3(projection.movement_state()),
        arsenal: projection.arsenal(),
        animation: projection.animation(),
        view_height: f64::from(projection.view_height()),
        numeric: profile_numeric(&template.profile),
        q3_arsenal: Some(projection.q3_arsenal()),
    }
}

/// Install the live scene-rows provider over the shared tables.
///
/// Reads linked bodies for live actors through the crate handles and
/// answers the facade's `queryActors` with box-shaped rows; model
/// shapes still belong to the physics lane, so entity/train matching
/// stays gated while hazard reads go live. Idempotent: reinstalling
/// replaces an equivalent provider.
fn install_scene_rows(simulation: &SharedSimulation) {
    let actors = simulation.actors_handle();
    let tables = simulation.bodies_handle();
    let provider: SceneRowProvider = Rc::new(move |bounds, _kind| {
        let registry = actors.borrow();
        let bodies = tables.borrow();
        registry
            .observations()
            .iter()
            .filter_map(|observation| {
                let owned = registry.resolve_owned(&observation.id)?;
                let linked = bodies.linked(registry.inner(), &observation.id)?;
                if !qa_world::spatial::bounds_intersect(&linked.absolute_bounds, &bounds) {
                    return None;
                }
                Some(SceneActorHit {
                    body: SceneBody {
                        actor: owned,
                        state: linked.state,
                        absolute_bounds: linked.absolute_bounds,
                    },
                    collision: SceneCollision {
                        shape: SceneCollisionShape::Box,
                    },
                })
            })
            .collect()
    });
    simulation.scene().set_actor_rows(provider);
}

/// Per-actor navigation world (donor `worldFor` result).
struct ApplicationNavigationWorld {
    /// Shared collision queries.
    scene: ApplicationNavigationScene,
    /// Owning simulation.
    simulation: SharedSimulation,
    /// Loaded recipe.
    recipe: ExecutableRecipe,
    /// Selected player snapshot, when the world is bound to a client.
    selected: Option<MovementPredictionPlayer>,
    /// Prediction driver over the selected player.
    driver: Rc<dyn NavigationPredictionDriver>,
    /// Resolved binding models by binding key (donor `boundModels`).
    bound_models: RefCell<HashMap<BindingKey, i32>>,
}

impl ApplicationNavigationWorld {
    /// Build a world bound to a selected player, if any.
    fn new(
        simulation: SharedSimulation,
        recipe: ExecutableRecipe,
        selected: Option<MovementPredictionPlayer>,
        app_profile: MovementProfile,
    ) -> Self {
        let driver: Rc<dyn NavigationPredictionDriver> = Rc::new(ApplicationNavigationDriver {
            simulation: simulation.clone(),
            recipe: recipe.clone(),
            selected: selected.clone(),
            app_profile: app_profile.clone(),
        });
        install_scene_rows(&simulation);
        Self {
            scene: ApplicationNavigationScene,
            simulation,
            recipe,
            selected,
            driver,
            bound_models: RefCell::new(HashMap::new()),
        }
    }

    /// Prediction player for sessions: the selected player, else the
    /// live first player (donor `selectedPlayer ?? firstPlayer()`).
    fn session_player(&self) -> Option<MovementPredictionPlayer> {
        self.selected
            .clone()
            .or_else(|| live_players(&self.simulation, &self.recipe).into_iter().next())
    }

    /// Resolve an entity binding to live mover state (donor `entity`).
    fn resolve_entity(&self, binding: &NavigationEntityBinding) -> Option<NavigationEntityState> {
        let scene = self.simulation.scene();
        let linked = scene.query_actors(scene.model_bounds(0), None);
        let mut model = binding
            .model
            .or_else(|| self.bound_models.borrow().get(&binding_key(binding)).copied());
        if model.is_none() {
            for hit in &linked {
                let SceneCollisionShape::Model { model: candidate } = &hit.collision.shape else {
                    continue;
                };
                let candidate = *candidate;
                let entity = self.q1_actor(&hit.body.actor)?;
                for endpoint in [hit.body.state.origin, entity.pos1, entity.pos2] {
                    if !source_mover_bounds_match(
                        &binding.bounds,
                        &hit.body.absolute_bounds,
                        hit.body.state.origin,
                        endpoint,
                    ) {
                        continue;
                    }
                    if model.is_some() {
                        return None;
                    }
                    model = Some(candidate);
                    break;
                }
            }
            if let Some(resolved) = model {
                self.bound_models.borrow_mut().insert(binding_key(binding), resolved);
            }
        }
        let model = model?;
        for hit in &linked {
            let SceneCollisionShape::Model { model: candidate } = &hit.collision.shape else {
                continue;
            };
            if *candidate != model {
                continue;
            }
            if let Some(state) = self.q1_entity_state(&hit.body.actor, &hit.body) {
                return Some(state);
            }
            if let Some(state) = self.q2_entity_state(&hit.body.actor, &hit.body) {
                return Some(state);
            }
            return None;
        }
        None
    }

    /// Read a Q1 actor record for a scene body.
    fn q1_actor(&self, actor: &OwnedActor) -> Option<Q1Actor> {
        self.simulation
            .with_q1_source(|view| view.services.borrow().entity(actor.id()).cloned())?
    }

    /// Q1 mover entity state (donor `entity` Q1 arm).
    fn q1_entity_state(&self, actor: &OwnedActor, body: &SceneBody) -> Option<NavigationEntityState> {
        let q1 = self.q1_actor(actor)?;
        let master = q1
            .door_group
            .first()
            .and_then(|master| {
                self.simulation
                    .with_q1_source(|view| view.services.borrow().entity(master).cloned())?
            })
            .unwrap_or_else(|| q1.clone());
        let key = if master.spawnflags & 8 != 0 {
            Some("q1:key/gold")
        } else if master.spawnflags & 16 != 0 {
            Some("q1:key/silver")
        } else {
            None
        };
        let player = self.session_player();
        let key_count = match (key, player.as_ref()) {
            (Some(key), Some(player)) => {
                let item = key.to_string();
                Some(
                    self.simulation
                        .with_q1_source(|view| view.services.borrow().host.inventory.count(player.actor.id(), &item))
                        .unwrap_or(0.0),
                )
            }
            _ => None,
        };
        let needs_key = keyed_door_needs_key(key, master.touch.is_some(), key_count);
        let elevator = (q1.classname == "func_plat").then(|| ElevatorState {
            origin: body.state.origin,
            bottom: q1.pos2,
            top: q1.pos1,
            phase: q1_elevator_phase(q1.state),
        });
        let locked = if q1.classname == "func_plat" {
            !q1.activated
        } else if q1.classname == "func_door"
            && (master.state == Q1MoverState::Bottom || master.state == Q1MoverState::Down)
        {
            !master.targetname.is_empty() || master.max_health > 0.0 || master.spawnflags & 4 != 0 || needs_key
        } else {
            false
        };
        Some(NavigationEntityState {
            actor: actor.id().clone(),
            enabled: q1.solid == Q1Solid::Bsp,
            locked,
            bounds: body.absolute_bounds,
            velocity: body.state.velocity,
            destination: q1.move_completion.as_ref().map(|moved| moved.destination),
            elevator,
            train: None,
        })
    }

    /// Q2 mover entity state (donor `entity` Q2 arm).
    fn q2_entity_state(&self, actor: &OwnedActor, body: &SceneBody) -> Option<NavigationEntityState> {
        let view = self.simulation.q2_source()?;
        let entity = {
            let game = view.game.borrow();
            game.entity(actor.id())?.clone()
        };
        let platform = {
            let game = view.game.borrow();
            mover_platform_state(&game, actor.id()).or_else(|| q2_expansion_platform(&game, actor.id()))
        };
        let module = {
            let game = view.game.borrow();
            game.movers.hooks.map(create_q2_mover_module)
        };
        let mut game = view.game.borrow_mut();
        let train = module
            .as_ref()
            .and_then(|module| module.train_route(actor.id().clone(), &mut game))
            .map(|route| q2_train_state(&route, body.state.origin));
        let (locked, destination) = q2_traversal(&module, &mut game, actor.id());
        Some(NavigationEntityState {
            actor: actor.id().clone(),
            enabled: entity.solid == Q2Solid::Brush,
            locked,
            bounds: body.absolute_bounds,
            velocity: body.state.velocity,
            destination,
            elevator: platform.map(|platform| q2_elevator_state(&platform, body.state.origin)),
            train,
        })
    }

    /// Hazard test over bounds (donor `hazard`).
    fn hazard_at(&self, bounds: &Bounds) -> bool {
        let scene = self.simulation.scene();
        for hit in scene.query_actors(*bounds, None) {
            if let Some(q1) = self.q1_actor(&hit.body.actor) {
                if q1.classname == "trigger_hurt"
                    && q1.solid == Q1Solid::Trigger
                    && q1.damage > 0.0
                    && q1.touch.is_some()
                {
                    return true;
                }
            }
            if let Some(view) = self.simulation.q2_source() {
                let game = view.game.borrow();
                if let Some(entity) = game.entity(hit.body.actor.id()) {
                    if entity.classname == "trigger_hurt"
                        && entity.solid == Q2Solid::Trigger
                        && entity.damage > 0.0
                        && entity.touch.is_some()
                        && entity.timestamp <= game.now()
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl NavigationWorld for ApplicationNavigationWorld {
    fn scene(&self) -> &dyn SceneQueries {
        &self.scene
    }

    fn pass_actor(&self) -> Option<ActorId> {
        self.selected.as_ref().map(|player| player.actor.id().clone())
    }

    fn revision(&self) -> i64 {
        (self.simulation.time_seconds() * 1000.0) as i64
    }

    fn admit(&self, request: &TraversalRequest, profile: &NavigationProfile) -> TraversalAdmission {
        let admit = create_movement_admission(Rc::clone(&self.driver), NavigationPredictionLimits::default());
        admit(request, profile).unwrap_or_else(|error| TraversalAdmission::Refused {
            reason: error.to_string(),
        })
    }

    fn begin_route(&self, profile: &NavigationProfile) -> Box<dyn NavigationRoutePrediction> {
        Box::new(
            create_movement_route_admission(Rc::clone(&self.driver), profile, &NavigationPredictionLimits::default())
                .expect("default prediction limits are valid"),
        )
    }

    fn entity(&self, binding: &NavigationEntityBinding) -> Option<NavigationEntityState> {
        self.resolve_entity(binding)
    }

    fn hazard(&self, bounds: &Bounds) -> bool {
        self.hazard_at(bounds)
    }
}

/// Hashable entity-binding key (donor `boundModels` keys by binding
/// identity; bindings here are values, so the key covers the model,
/// the bounds bits, and the raw words).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct BindingKey {
    /// Source model ordinal.
    model: Option<i32>,
    /// Bounds bits.
    bounds: [u32; 6],
    /// Raw source words.
    raw: Vec<i32>,
}

/// Key an entity binding for the bound-model cache.
fn binding_key(binding: &NavigationEntityBinding) -> BindingKey {
    BindingKey {
        model: binding.model,
        bounds: [
            binding.bounds.min.x.to_bits(),
            binding.bounds.min.y.to_bits(),
            binding.bounds.min.z.to_bits(),
            binding.bounds.max.x.to_bits(),
            binding.bounds.max.y.to_bits(),
            binding.bounds.max.z.to_bits(),
        ],
        raw: binding.raw.clone(),
    }
}

/// Whether a keyed Q1 door needs its key (donor `needsKey`).
///
/// `key_count` is `None` when no player is bound (donor
/// `player === null`, which reads locked) and the live key count from
/// [`Q1SharedInventoryTable::count`](qa_content::q1::foundation::host::Q1SharedInventoryTable::count)
/// otherwise; a keyed door with a touch function reads locked while
/// the count is zero.
fn keyed_door_needs_key(key: Option<&str>, touch: bool, key_count: Option<f64>) -> bool {
    key.is_some() && touch && key_count.is_none_or(|count| count == 0.0)
}

/// Project a Q1 mover state onto the elevator phase set.
fn q1_elevator_phase(state: Q1MoverState) -> ElevatorPhase {
    match state {
        Q1MoverState::Bottom => ElevatorPhase::Bottom,
        Q1MoverState::Up => ElevatorPhase::Up,
        Q1MoverState::Top => ElevatorPhase::Top,
        Q1MoverState::Down => ElevatorPhase::Down,
    }
}

/// Project a Q2 platform phase onto the elevator phase set.
fn q2_elevator_phase(phase: Q2PlatformPhase) -> ElevatorPhase {
    match phase {
        Q2PlatformPhase::Bottom => ElevatorPhase::Bottom,
        Q2PlatformPhase::Up => ElevatorPhase::Up,
        Q2PlatformPhase::Top => ElevatorPhase::Top,
        Q2PlatformPhase::Down => ElevatorPhase::Down,
    }
}

/// Q2 expansion platform state (donor `product.expansions` arm,
/// navigation.ts:114-115).
///
/// Reads the assembled product from the game arena; only Rogue packs
/// carry movers (`Q2RogueMovers::platform_state`, donor
/// `expansion.entities.movers?.platformState`), so other packs read as
/// absent. The caller establishes entity presence first
/// (`Q2RogueMovers::platform_state` resolves through `require_entity`).
fn q2_expansion_platform(game: &Q2GameServices, actor: &ActorId) -> Option<Q2PlatformState> {
    let product = game.composition.product.as_ref()?;
    for expansion in &product.expansions {
        if expansion.pack != Q2MissionPack::Rogue {
            continue;
        }
        let movers = Q2RogueMovers {
            hooks: expansion.entities.hooks,
        };
        if let Some((top, bottom, phase)) = movers.platform_state(actor, game) {
            return Some(q2_rogue_platform_state(top, bottom, phase));
        }
    }
    None
}

/// Rogue platform state as a base platform state (donor `{ top: pos1,
/// bottom: pos2, phase }`, movers.ts:21-24).
fn q2_rogue_platform_state(top: Vec3, bottom: Vec3, phase: Q2Plat2Phase) -> Q2PlatformState {
    Q2PlatformState {
        top,
        bottom,
        phase: q2_rogue_platform_phase(phase),
    }
}

/// Rogue platform phase as a base platform phase.
fn q2_rogue_platform_phase(phase: Q2Plat2Phase) -> Q2PlatformPhase {
    match phase {
        Q2Plat2Phase::Top => Q2PlatformPhase::Top,
        Q2Plat2Phase::Bottom => Q2PlatformPhase::Bottom,
        Q2Plat2Phase::Up => Q2PlatformPhase::Up,
        Q2Plat2Phase::Down => Q2PlatformPhase::Down,
    }
}

/// Elevator state for a Q2 platform (donor `{ ...platform, origin }`).
fn q2_elevator_state(platform: &Q2PlatformState, origin: Vec3) -> ElevatorState {
    ElevatorState {
        origin,
        bottom: platform.bottom,
        top: platform.top,
        phase: q2_elevator_phase(platform.phase),
    }
}

/// Live train state for a Q2 route (donor `train` arm).
fn q2_train_state(route: &Q2TrainRoute, origin: Vec3) -> TrainState {
    TrainState {
        origin,
        running: route.running,
        stops: route
            .stops
            .iter()
            .map(|stop| TrainStop {
                id: stop.actor.slot() as i32,
                origin: stop.origin,
                next: stop.next.as_ref().map(|next| next.slot() as i32),
                wait: stop.wait,
                teleport: stop.teleport,
            })
            .collect(),
    }
}

/// Q2 mover traversal (donor `moverTraversal ?? traversal`).
fn q2_traversal(module: &Option<Q2MoverModule>, game: &mut Q2GameServices, actor: &ActorId) -> (bool, Option<Vec3>) {
    if let Some(traversal) = mover_traversal(game, actor) {
        return (traversal.locked, traversal.destination);
    }
    module.map_or((false, None), |module| {
        let traversal = module.traversal(actor.clone(), game);
        (traversal.locked, traversal.destination)
    })
}

/// Prediction driver over the selected player (donor `driver`).
struct ApplicationNavigationDriver {
    /// Owning simulation.
    simulation: SharedSimulation,
    /// Loaded recipe.
    recipe: ExecutableRecipe,
    /// Selected player snapshot, when the world is bound to a client.
    selected: Option<MovementPredictionPlayer>,
    /// Application movement profile for prediction sessions.
    app_profile: MovementProfile,
}

impl NavigationPredictionDriver for ApplicationNavigationDriver {
    fn begin(
        &self,
        request: &TraversalRequest,
        _selected: &NavigationProfile,
    ) -> Option<Box<dyn NavigationPrediction>> {
        let player = self
            .selected
            .clone()
            .or_else(|| live_players(&self.simulation, &self.recipe).into_iter().next())?;
        Some(Box::new(create_player_movement_prediction(
            &self.simulation,
            &player,
            request.from,
            ZERO,
            16,
            false,
            Some(&self.app_profile),
        )))
    }
}

/// Empty-world collision queries. Collision-lane seam: answers as if
/// the world held no geometry until the simulation scene queries are
/// wired through (same seam as the player-movement empty-world
/// traces).
struct ApplicationNavigationScene;

impl SceneQueries for ApplicationNavigationScene {
    fn trace(&self, query: &TraceQuery) -> TraceResult {
        TraceResult {
            fraction: 1.0,
            end: query.end,
            start_solid: false,
            all_solid: false,
            contact: TraceContact::None,
            hit: TraceHit::None,
            detail: TraceDetail::Q1 {
                in_open: true,
                in_water: false,
                source_plane: qa_core::math::Plane {
                    normal: Vec3 { x: 0.0, y: 0.0, z: 1.0 },
                    distance: 0.0,
                },
                surface_flags: None,
                contents: None,
            },
        }
    }

    fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
        PointContentsResult::Q1 { contents: 0 }
    }

    fn box_leaves(&self, _bounds: &Bounds, _limit: usize) -> LeafQueryResult {
        LeafQueryResult {
            leaves: Vec::new(),
            topnode: None,
            overflow: false,
        }
    }

    fn areas_connected(&self, first: i32, second: i32) -> bool {
        first == second
    }

    fn cluster_visible(&self, _from: i32, _to: i32, _kind: VisibilityKind) -> bool {
        true
    }
}

/// Q2 train connections for navigation construction (donor
/// `applicationTrainConnections` from `bot-mover-connections.ts`).
fn train_connections(simulation: &SharedSimulation, profile: &NavigationProfile) -> Vec<NavigationConnection> {
    let Some(view) = simulation.q2_source() else {
        return Vec::new();
    };
    let scene = simulation.scene();
    let mut movers = scene.query_actors(scene.model_bounds(0), None);
    movers.sort_by_key(|hit| hit.body.actor.id().slot());
    let mut connections = Vec::new();
    for hit in &movers {
        let SceneCollisionShape::Model { model } = &hit.collision.shape else {
            continue;
        };
        let model = *model;
        let entity = {
            let game = view.game.borrow();
            let Some(entity) = game.entity(hit.body.actor.id()).cloned() else {
                continue;
            };
            entity
        };
        let module = {
            let game = view.game.borrow();
            game.movers.hooks.map(create_q2_mover_module)
        };
        let Some(module) = module else {
            continue;
        };
        let mut game = view.game.borrow_mut();
        let Some(route) = module.train_route(hit.body.actor.id().clone(), &mut game) else {
            continue;
        };
        let edition = game.options.edition;
        let local = hit.body.state.bounds;
        for stop in &route.stops {
            let next = stop
                .next
                .as_ref()
                .and_then(|next| route.stops.iter().find(|candidate| &candidate.actor == next));
            let Some(next) = next else {
                continue;
            };
            if stop.wait < 0.0 || stop.teleport || next.teleport {
                continue;
            }
            let destination = game.entities.get(&next.actor).cloned();
            let speed = if edition == Q2Edition::Rerelease {
                destination.as_ref().map_or(entity.speed, |destination| {
                    if destination.speed != 0.0 {
                        destination.speed
                    } else {
                        entity.speed
                    }
                })
            } else {
                entity.speed
            };
            if speed <= 0.0 {
                continue;
            }
            let dx = f64::from(next.origin.x - stop.origin.x);
            let dy = f64::from(next.origin.y - stop.origin.y);
            let dz = f64::from(next.origin.z - stop.origin.z);
            let travel_seconds = stop.wait + dx.hypot(dy).hypot(dz) / speed;
            connections.push(train_connection(
                &entity,
                model,
                local,
                profile.shape.bounds().min.z,
                stop,
                next,
                travel_seconds,
            ));
        }
    }
    connections
}

/// Riding origin for a train stop (donor `riding`): the stop origin
/// raised to the car roof, less the profile minimum z.
fn riding_origin(origin: Vec3, local: Bounds, shape_min_z: f32) -> Vec3 {
    Vec3 {
        x: origin.x + (local.min.x + local.max.x) / 2.0,
        y: origin.y + (local.min.y + local.max.y) / 2.0,
        z: origin.z + local.max.z - shape_min_z + 0.125,
    }
}

/// Build one mover connection for a train stop pair (donor
/// connection push).
fn train_connection(
    entity: &Q2Entity,
    model: i32,
    local: Bounds,
    shape_min_z: f32,
    stop: &qa_content::q2::foundation::movers::Q2TrainStop,
    next: &qa_content::q2::foundation::movers::Q2TrainStop,
    travel_seconds: f64,
) -> NavigationConnection {
    let at_stop = |point: Vec3| Vec3 {
        x: point.x + stop.origin.x,
        y: point.y + stop.origin.y,
        z: point.z + stop.origin.z,
    };
    NavigationConnection {
        from: riding_origin(stop.origin, local, shape_min_z),
        to: riding_origin(next.origin, local, shape_min_z),
        mode: TravelMode::Mover,
        hint: None,
        entity: Some(NavigationEntityBinding {
            model: Some(model),
            bounds: Bounds {
                min: at_stop(local.min),
                max: at_stop(local.max),
            },
            raw: vec![
                entity.actor.id().slot() as i32,
                stop.actor.slot() as i32,
                next.actor.slot() as i32,
            ],
        }),
        id: stop.actor.slot() as i32,
        source_travel_type: 7,
        travel_seconds,
    }
}

/// Bot movement projection over detached player prediction (donor
/// `predictApplicationBotMovement` projection argument).
struct ApplicationBotProjection<'p> {
    /// Detached player movement prediction.
    prediction: &'p super::player_movement::PlayerMovementPrediction,
    /// Client movement query.
    query: &'p BotMovementPrediction,
    /// Graph asset, when the graph was built from one.
    asset: Option<&'p NavigationAsset>,
    /// Navigating runtime for area reads.
    runtime: &'p NavigationRuntime<'p>,
}

impl BotMovementProjection for ApplicationBotProjection<'_> {
    fn provider(&self) -> &dyn MovementProvider {
        self.prediction.provider()
    }

    fn services(&self) -> MovementServices {
        *self.prediction.services()
    }

    fn input(&self, previous: Option<&MovementResult>, frame: i32, command_move: Vec3) -> MovementInput {
        let at = match previous {
            Some(MovementResult::Active { state, .. }) => state.origin(),
            Some(MovementResult::ActorRemoved) | None => self.query.origin,
        };
        // The relocated driver math normalizes (to - from) by
        // min(400, distance * 20) / distance; offsetting the endpoint
        // by command / 20 reproduces the donor wish vector exactly
        // (donor commands stay within the 400-unit clamp the driver
        // applies anyway).
        let to = Vec3 {
            x: at.x + command_move.x / 20.0,
            y: at.y + command_move.y / 20.0,
            z: at.z,
        };
        self.prediction.input(
            previous,
            usize::try_from(frame).unwrap_or(usize::MAX),
            &TraversalRequest {
                from: at,
                to,
                mode: if self.query.presence == 4 {
                    TravelMode::Crouch
                } else {
                    TravelMode::Walk
                },
                hint: None,
                entity: None,
            },
        )
    }

    fn stop(&self, previous: Option<&MovementResult>, _result: &MovementResult, frame: i32) -> BotMovementStop {
        // The rich observation comes from the prediction's last step
        // (the advance that produced `result` just ran on this same
        // prediction); the navigation-level result carries no medium
        // or fall detail.
        let observation = movement_observation(&self.prediction.last_step());
        if let Some(NavigationAsset::Aas(asset)) = self.asset {
            let start = match previous {
                Some(MovementResult::Active { state, .. }) => state.origin(),
                Some(MovementResult::ActorRemoved) | None => self.query.origin,
            };
            if let Ok(Some(crossing)) = aas_prediction_stop(
                asset,
                start,
                observation.origin,
                frame,
                self.query.stop_events,
                self.query.stop_area,
            ) {
                return crossing;
            }
        }
        let was_grounded = match previous {
            Some(MovementResult::Active { ground, .. }) => !matches!(ground, TraceHit::None),
            Some(MovementResult::ActorRemoved) | None => self.query.on_ground,
        };
        let mut flags = classify_ground_flags(was_grounded, observation.grounded);
        flags |= match observation.medium {
            MovementMedium::Dry => 0,
            MovementMedium::Slime => 8,
            MovementMedium::Lava => 16,
            MovementMedium::Water => 4,
        };
        let area = self.runtime.area_at(observation.origin).ok().flatten();
        if self.query.stop_area != 0 && area == Some(self.query.stop_area) {
            flags |= 512;
            if !was_grounded && observation.grounded {
                flags |= 1024;
            }
        }
        if observation.damaging_fall {
            flags |= 32;
        }
        BotMovementStop {
            events: flags,
            origin: observation.origin,
            area,
        }
    }
}

/// Ground-transition stop flags (donor landing/liftoff bits).
fn classify_ground_flags(was_grounded: bool, grounded: bool) -> i32 {
    if !was_grounded && grounded {
        1
    } else if was_grounded && !grounded {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use qa_bots::scene::{TracePolicy, WorldKind};
    use qa_content::contract::GameFamily;
    use qa_core::identity::ProviderId;
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use qa_core::time::ClockProfile;
    use qa_world::movement::q3::types::{Q3MovementProfile, Q3Product};

    use super::*;

    /// Capability set covers walk/crouch/jump/drop/swim/water-jump/ladder/mover for q3.
    #[test]
    fn profile_capabilities_cover_q3_travel() {
        let profile = bot_navigation_profile(&locomotion_fixture());
        assert!(profile.capabilities.contains(&TravelMode::Walk));
        assert!(profile.capabilities.contains(&TravelMode::Crouch));
        assert!(profile.capabilities.contains(&TravelMode::Jump));
        assert!(profile.capabilities.contains(&TravelMode::Drop));
        assert!(profile.capabilities.contains(&TravelMode::Swim));
        assert!(profile.capabilities.contains(&TravelMode::WaterJump));
        assert!(profile.capabilities.contains(&TravelMode::Ladder));
        assert!(profile.capabilities.contains(&TravelMode::Mover));
        assert!(profile.crouched_shape.is_some());
        assert_eq!(profile.maximum_step, 18.0);
        assert_eq!(profile.minimum_floor_normal, 0.7);
        assert_eq!(profile.maximum_drop, 128.0);
        assert!(!profile.monster);
    }

    /// Riding origins center on the car and clear the profile minimum z.
    #[test]
    fn riding_origin_centers_and_clears() {
        let local = Bounds {
            min: Vec3 {
                x: -32.0,
                y: -32.0,
                z: 0.0,
            },
            max: Vec3 {
                x: 32.0,
                y: 32.0,
                z: 64.0,
            },
        };
        let at = riding_origin(
            Vec3 {
                x: 100.0,
                y: 200.0,
                z: 8.0,
            },
            local,
            -24.0,
        );
        assert_eq!(at.x, 100.0);
        assert_eq!(at.y, 200.0);
        assert_eq!(at.z, 8.0 + 64.0 + 24.0 + 0.125);
    }

    /// Ground-transition flags match the donor landing/liftoff bits.
    #[test]
    fn ground_flags_match_donor_bits() {
        assert_eq!(classify_ground_flags(false, true), 1);
        assert_eq!(classify_ground_flags(true, false), 2);
        assert_eq!(classify_ground_flags(true, true), 0);
        assert_eq!(classify_ground_flags(false, false), 0);
    }

    /// Binding keys separate models, bounds, and raw words.
    #[test]
    fn binding_keys_separate_bindings() {
        let bounds = Bounds {
            min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            max: Vec3 {
                x: 64.0,
                y: 64.0,
                z: 64.0,
            },
        };
        let base = NavigationEntityBinding {
            model: Some(3),
            bounds,
            raw: vec![1, 2, 3],
        };
        let mut moved = base.clone();
        moved.model = Some(4);
        let mut shifted = base.clone();
        shifted.bounds.max.z = 65.0;
        let mut reworded = base.clone();
        reworded.raw = vec![1, 2, 4];
        assert_eq!(binding_key(&base), binding_key(&base.clone()));
        assert_ne!(binding_key(&base), binding_key(&moved));
        assert_ne!(binding_key(&base), binding_key(&shifted));
        assert_ne!(binding_key(&base), binding_key(&reworded));
    }

    /// Checkpoint values encode the donor version/base/clients shape.
    #[test]
    fn checkpoint_value_encodes_donor_shape() {
        let checkpoint = ApplicationBotNavigationCheckpoint {
            version: 1,
            base: runtime_checkpoint_fixture(),
            clients: vec![ApplicationBotNavigationClientCheckpoint {
                client: 2,
                reusable: true,
                runtime: runtime_checkpoint_fixture(),
            }],
        };
        let value = checkpoint.to_save_value();
        let version = checkpoint_field(&value, "applicationNavigation", "version").expect("version");
        assert_eq!(
            checkpoint_integer(version, "applicationNavigation.version", 0).expect("int"),
            1
        );
        let base = checkpoint_field(&value, "applicationNavigation", "base").expect("base");
        assert!(matches!(base, SaveValue::Map(_)));
        let clients = checkpoint_field(&value, "applicationNavigation", "clients").expect("clients");
        let SaveValue::List(entries) = clients else {
            panic!("clients list");
        };
        assert_eq!(entries.len(), 1);
        let client = checkpoint_field(&entries[0], "entry", "client").expect("client");
        assert_eq!(checkpoint_integer(client, "entry.client", 0).expect("slot"), 2);
        let reusable = checkpoint_field(&entries[0], "entry", "reusable").expect("reusable");
        assert!(checkpoint_boolean(reusable, "entry.reusable").expect("flag"));
    }

    /// Keyed Q1 doors with a touch function read locked without inventory.
    #[test]
    fn keyed_doors_read_locked_without_inventory() {
        assert!(keyed_door_needs_key(Some("q1:key/gold"), true, None));
        assert!(keyed_door_needs_key(Some("q1:key/gold"), true, Some(0.0)));
        assert!(!keyed_door_needs_key(Some("q1:key/gold"), true, Some(1.0)));
        assert!(!keyed_door_needs_key(None, true, None));
        assert!(!keyed_door_needs_key(Some("q1:key/gold"), false, None));
    }

    /// Rogue expansion platforms map onto base platform states verbatim.
    #[test]
    fn rogue_platforms_map_onto_base_states() {
        let top = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 64.0,
        };
        let bottom = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        for (rogue, base) in [
            (Q2Plat2Phase::Top, Q2PlatformPhase::Top),
            (Q2Plat2Phase::Bottom, Q2PlatformPhase::Bottom),
            (Q2Plat2Phase::Up, Q2PlatformPhase::Up),
            (Q2Plat2Phase::Down, Q2PlatformPhase::Down),
        ] {
            let state = q2_rogue_platform_state(top, bottom, rogue);
            assert_eq!(state.top, top);
            assert_eq!(state.bottom, bottom);
            assert_eq!(state.phase, base);
        }
    }

    /// Locomotion fixture over a q3 profile.
    fn locomotion_fixture() -> LocomotionPlayer {
        LocomotionPlayer {
            profile: MovementProfile::Q3(Q3MovementProfile {
                id: ProviderId::new("q3", "test"),
                clock: ClockProfile::Q3 {
                    server_frame_milliseconds: 100.0,
                    fixed_movement_milliseconds: None,
                },
                numeric: Q3_BINARY32_PROFILE,
                product: Q3Product::BaseQ3,
                fixed_milliseconds: None,
                no_footsteps: false,
            }),
            standing_bounds: Bounds {
                min: Vec3 {
                    x: -15.0,
                    y: -15.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 15.0,
                    y: 15.0,
                    z: 32.0,
                },
            },
            source_movement: None,
            character: GameFamily::Q3,
            world_gravity: 800.0,
            q2_movement_config: None,
            flight: false,
        }
    }

    /// Runtime checkpoint fixture.
    fn runtime_checkpoint_fixture() -> NavigationRuntimeCheckpoint {
        NavigationRuntimeCheckpoint {
            version: 1,
            map: NavigationMapIdentity {
                name: "maps/test.bsp".to_string(),
                format: WorldKind::Q3Bsp,
                digest: ContentDigest::new("sha256:test"),
            },
            enabled: Vec::new(),
            blocked: Vec::new(),
            admission_seconds: Vec::new(),
            world_revision: 0,
            generation: 0,
        }
    }

    /// Trace policy helper assertion (keeps the policy import live).
    #[test]
    fn trace_policy_matches_q3() {
        let policy = player_trace_policy(&locomotion_fixture());
        assert!(matches!(policy, TracePolicy::Q3 { .. }));
    }

    /// Numeric helper reads the profile numeric.
    #[test]
    fn numeric_helper_reads_profile() {
        assert_eq!(profile_numeric(&locomotion_fixture().profile), Q3_BINARY32_PROFILE);
    }

    /// Template character matches the q3 fixture.
    #[test]
    fn family_matches_template() {
        assert_eq!(locomotion_fixture().character, GameFamily::Q3);
    }
}
