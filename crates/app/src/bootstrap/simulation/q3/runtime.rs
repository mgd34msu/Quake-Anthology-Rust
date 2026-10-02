//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/q3/runtime.ts`
//!
//! Quake III source game runtime composition.
//!
//! # Missing siblings
//!
//! None: `q3/types.ts`, `q3/presentation.ts`, and `q3/server-state.ts`
//! have all landed in this directory.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use super::super::q3_ballistics::{Q3BehaviorLaunch, Q3WeaponBehaviorPort};
use qa_content::q3::base::combat_bridge::{Q3CombatBridge, Q3CombatBridgeHost, VictimArmorContext};
use qa_content::q3::base::game::ballistics_math::{
    q3_bounce_velocity, q3_bullet_endpoint, q3_missile_hit_time, q3_shotgun_endpoints, BallisticAttack,
};
use qa_content::q3::base::game::combat::{
    can_damage as combat_can_damage, damage as combat_damage, q3_admit_target_damage,
    radius_damage as combat_radius_damage, AdmitDecision, ArmorEffect as GameArmorEffect, ArmorState as GameArmorState,
    AttackCause as GameAttackCause, AttackProvenance as GameAttackProvenance, CombatActors as GameCombatActors,
    CombatAuthority, CombatContext, CombatProduct, CombatState as GameCombatState, DamageApply,
    DamageDecision as GameDamageDecision, DamageDelivery, DamageFeedback as GameDamageFeedback,
    DamageMutation as GameDamageMutation, DamageOutcome as GameDamageOutcome, DamageReaction,
    DamageRequest as GameDamageRequest, EnvHazard, PoweredProtection, Q2NativeCause as GameQ2NativeCause,
    Q3DamageCall as GameDamageCall, RegularArmor, SourceDamageModifier as GameSourceDamageModifier,
    SourceTime as GameSourceTime, SpatialQueries as GameSpatialQueries,
};
use qa_content::q3::base::game::death::{
    DeathFrame, DeathHost, DeathItemCallbacks, DeathRuntime, MissionpackDeath,
    Q3DeathAnimationCheckpoint as DeathAnimationCheckpoint, Q3DeathAnimationSequence as DeathAnimationSequence,
    SimItemDrop,
};
use qa_content::q3::base::game::entities::{
    run_think as entities_run_think, DamageParticipant as EntitiesDamageParticipant, EntityEventStatus,
    EntityPool as BaseEntityPool, EntityPoolOptions, EntityRef as GameEntityRef,
    ItemDefinition as EntitiesItemDefinition, Q3ItemTable as EntitiesItemTable, ServerWorldMirror, SharedActor,
    TouchContactMirror,
};
use qa_content::q3::base::game::entities::{GameRandomMirror, SimRandom};
use qa_content::q3::base::game::format::{game_format, GameFormatArgument};
use qa_content::q3::base::game::hitscan::{
    q3_rail_statistics, BulletEmitEvent, Q3BulletAttack, Q3BulletTarget, Q3ContactEvent, Q3RailStatistics, RailShot,
    RailStatistics, RailStatisticsOutcome,
};
use qa_content::q3::base::game::item_lifecycle::{
    bind_item_save_callbacks, finish_spawning_item as lifecycle_finish_spawning_item, observe_q3_supply,
    respawn_item as lifecycle_respawn_item, spawn_item as lifecycle_spawn_item, touch_item as lifecycle_touch_item,
    ItemLifecycleCallbacks, ItemLifecycleContext, ItemRegistry, SupplyObservation,
};
use qa_content::q3::base::game::item_motion::{
    drop_item as items_core_drop_item, launch_item as items_core_launch_item, launch_item_think, run_item,
    DropItemContext as ItemsDropItemContext, LaunchItemContext as ItemsLaunchItemContext, RunItemContext,
    DROPPED_FLAG_THINK, LAUNCH_ITEM_THINK,
};
use qa_content::q3::base::game::item_pickup::{
    pickup_item as content_pickup_item, ItemPickupContext, PowerupSightTrace,
};
use qa_content::q3::base::game::items_core::{
    AccuracyTarget as ItemsAccuracyTarget, ActorId as ItemsActorId, ActorTraceQuery as ItemsActorTraceQuery,
    ActorTraceResult as ItemsActorTraceResult, CallbackName, CombatOps as ItemsCombatOps,
    DamageParticipant as ItemsDamageParticipant, EntityPool as ItemsEntityPool, GameRandom as ItemsGameRandom,
    ItemDefinition as ItemsCoreItemDefinition, ItemKind as ItemsItemKind, ItemRegistry as ItemsItemRegistry,
    ItemTable as ItemsItemTable, MoverCore as ItemsMoverCore, OwnedActor as ItemsOwnedActor, Q3GameItemsError,
    Q3GameItemsResult, Slot, SpawnVariables as ItemsSpawnVariables, TraceContact as ItemsTraceContact,
    TraceHit as ItemsTraceHit, WorldOps as ItemsWorldOps,
};
use qa_content::q3::base::game::level::{GameLevel, TeamVoteState, VoteState};
use qa_content::q3::base::game::memory::{GameMemory, GameMemorySave};
use qa_content::q3::base::game::misc::kill_box as base_kill_box;
use qa_content::q3::base::game::misc::teleport_player as misc_teleport_player;
use qa_content::q3::base::game::misc::TeleportContext;
use qa_content::q3::base::game::misc_spawn::{misc_spawn_handlers, run_misc_spawn, MiscSpawnHost};
use qa_content::q3::base::game::missile::MissileRuntime;
use qa_content::q3::base::game::missile::{
    snap_vector, snap_vector_towards, BodyState as MissileBodyState, BodyTable, ImpactEmit as MissileImpactEmit,
    InvulnerabilityOutcome as MissileInvulnerability, MissileDirection, MissileHost, MissileLauncher, MissileSaveEntry,
    ProjectileContext, ProjectileDriver, ProjectileLaunch as MissileLaunch, ProjectilePhase as MissilePhase,
};
use qa_content::q3::base::game::mover::MoverRuntime;
use qa_content::q3::base::game::mover_spawn::{
    is_door_trigger as mover_is_door_trigger, mover_spawn_handlers, run_mover_spawn,
    MoverSpawnHost as ItemsMoverSpawnHost,
};
use qa_content::q3::base::game::numeric::game_atoi;
use qa_content::q3::base::game::personal_portal::PersonalPortalRuntime;
use qa_content::q3::base::game::projectile::q3_launch_projectile;
use qa_content::q3::base::game::save_level::{
    capture_q3_level, restore_q3_level, Q3GameLevel, Q3TeamVoteState, Q3VoteState,
};
use qa_content::q3::base::game::save_module_values::Q3CvarSnapshot;
use qa_content::q3::base::game::save_reader::{read_q3_actor, read_q3_graph};
use qa_content::q3::base::game::save_state::{
    capture_q3_graph, graph_to_json, prepare_q3_graph, restore_q3_graph, ClientBacking as SaveClientBacking,
    OwnershipEntry as SaveOwnershipEntry, Q3ActorRegistry as SaveQ3ActorRegistry,
    Q3EntityRecords as SaveQ3EntityRecords,
};
use qa_content::q3::base::game::shader_remaps::ShaderRemapRegistry;
use qa_content::q3::base::game::spawn::{
    spawn_entities, SpawnFilter, SpawnHandlerTable, SpawnOutcome, SpawnPair, SpawnReport, SpawnRoute, SpawnServices,
    SpawnVariables, WorldspawnState,
};
use qa_content::q3::base::game::state::{
    ConnectionState, GameFlags, Participant as StateParticipant, SpectatorState, TouchContact as StateTouchContact,
    MAX_CLIENTS, MAX_GENTITIES,
};
use qa_content::q3::base::game::targets::{target_spawn_handlers, TargetLocationState};
use qa_content::q3::base::game::triggers::trigger_spawn_handlers;
use qa_content::q3::base::game::utilities::{
    find_entity, use_targets as base_use_targets, ConfigStringRegistry, ConfigStringStore, EntityStringField,
};
use qa_content::q3::base::game::weapon::{
    invulnerability_effect, log_accuracy_hit, InvulnerabilityImpact as WeaponImpact, WeaponRuntime,
};
use qa_content::q3::base::game::weapon::{BulletHost, ContactHost, RailHost, ShotgunHost};
use qa_content::q3::base::map_spawns::find_q3_entity_teams;
use qa_content::q3::base::records::{
    ArmorState as RecordsArmorState, AttackCause as RecordsAttackCause, AttackProvenance as RecordsAttackProvenance,
    ClientBackingSnapshot, CombatState as RecordsCombatState, DamageAdmission, DamageDecision as RecordsDamageDecision,
    DamageFeedback as RecordsDamageFeedback, DamageMutation as RecordsDamageMutation,
    DamageOutcome as RecordsDamageOutcome, DamageParticipant as RecordsDamageParticipant, DamageRequest,
    EntityPoolRef as RecordsPoolRef, EntityRef as RecordsEntityRef, EnvironmentHazard, PoweredProtectionState,
    Q1ArmorEffect, Q2ClassicGame, Q2NativeCause as RecordsQ2NativeCause, Q3ActorCallbacks, Q3DamageCall,
    Q3EntityPool as RecordsEntityPool, Q3EntityRecords, Q3RecordHost, Q3SessionActors, Q3SessionBodies,
    Q3SessionCombat, Q3SessionInventory, RegularArmorState, SharedParticipant, SlotOwnership,
};
use qa_content::q3::base::settings::{Q3GameSettings, Q3SettingsHost};
use qa_content::q3::base::shared::definitions::{
    stat_schema, EntityEvent, EntityType, GameType, MoveType, Powerup, Product, StatSchema, Team, Weapon, WeaponState,
    EVENT_VALID_MSEC,
};
use qa_content::q3::base::shared::entity_shared::EntityCollisionModel;
use qa_content::q3::base::shared::entity_state::EntityState;
use qa_content::q3::base::shared::items::{
    find_item_for_weapon as shared_item_for_weapon, item_list, ItemKind as SharedItemKind,
};
use qa_content::q3::base::shared::player_state::{MoveFlags, UserCommand as Q3UserCommand, ENTITYNUM_WORLD};
use qa_content::q3::base::shared::trajectory::{evaluate_trajectory, evaluate_trajectory_delta, TrajectoryType};
use qa_content::q3::base::world::{
    ActorSpatialQueries, ActorTraceQuery, ActorTraceResult, LinkState, Q3ServerWorld, ServerTraceQuery,
    ServerTraceResult, ServerWorld, TraceContact as WorldTraceContact,
};
use qa_content::q3::base::world::{
    ActorTraceHit, ActorTraceResult as WorldActorTraceResult, TraceContact as BaseTraceContact, TraceSolidity,
};
use qa_content::q3::base::world_adapter::{
    ActorCollision, ActorCollisionRole, Q3TraceQuery, Q3TraceResult, Q3WorldAdapter, Q3WorldAdapterHost,
};
use qa_content::q3::foundation::weapon_behavior::q3_projectile_behavior;
use qa_content::q3::team_arena::arenas::{ArenaHost, ArenaRuntime};
use qa_content::q3::team_arena::client_admission::{
    client_info_value, ClientAdmissionHost, ClientAdmissionRuntime, ClientAdmissionSettings, ClientBotServices,
};
use qa_content::q3::team_arena::client_effects::{
    client_end_frame, ClientTimerOwnership, EffectsCore, EffectsCoreRef, EffectsHost, EffectsRef,
};
use qa_content::q3::team_arena::client_events::{client_events, ClientEvents, SpawnSelector};
use qa_content::q3::team_arena::client_policy::{
    client_inactivity_timer, client_intermission_think, spectator_client_end_frame, spectator_think, ClientPolicyHost,
};
use qa_content::q3::team_arena::client_spawn::{
    spawn_deathmatch_point, spawn_player_start, ClientSpawnFrame, ClientSpawnHost, ClientSpawnRuntime,
    ClientSpawnState, SpawnPoint, SpawnPose,
};
use qa_content::q3::team_arena::client_think::{
    ClientThinkFrame, ClientThinkHost, ClientThinkRuntime, ClientThinkSettings, TouchAccess,
};
use qa_content::q3::team_arena::commands::{CommandImports, CommandSettings, GameCommandHost, GameCommandRuntime};
use qa_content::q3::team_arena::movement_host::{ClientMovementOptions, ClientMovementResult, MovementHost};
use qa_content::q3::team_arena::r#match::{
    MatchHost, MatchModuleState, MatchRuntime, MatchSettings, MatchState, MatchStateRef, SpawnShort,
};
use qa_content::q3::team_arena::server_commands::{
    GameServerCommandHost, GameServerCommandRuntime, GameServerCommandState, ServerCommandCapability, ServerCommandCvar,
};
use qa_content::q3::team_arena::session::{GameSessionManager, SessionWorld};
use qa_content::q3::team_arena::support::{
    ClientRef as SupportClientRef, Combat as SupportCombat, CombatRef, ConfigStrings,
    CvarRegistry as SupportCvarRegistry, DamageParticipant as SupportDamageParticipant, DeathHost as SupportDeathHost,
    DieCallback, DropHost, EntityPool as SupportEntityPool, EntityRef as SupportEntityRef,
    GameRandom as TeamGameRandom, ItemHost, PoolHooks, PoolRef, PortalHost, Q3World as SupportQ3World, RankingsHost,
    SaveValue as TeamSaveValue, SessionCvarName, SessionCvarService, SessionServices, SharedSlots, SlotArray,
    TargetsHost, TeleportHost, TouchContact, WeaponHost, WorldRef,
};
use qa_content::q3::team_arena::team::{spawn_team_point, ObeliskSettings, TeamHost, TeamRuntime};
use qa_content::value::{SaveJson as ContentSaveJson, SaveReader as ContentSaveReader};
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, OwnedActor, ProviderId, SavedActorId};
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;
use qa_core::time::{FrameContext, SourceTime as CoreSourceTime};
use qa_net::common::commands::ActorCommand;
use qa_world::body::{BodyState, LinkedBody};
use qa_world::combat::{Delivery, Reaction};
use qa_world::movement::q3::weapon::{step_q3_holdable, Q3HoldableState};
use qa_world::save::value::{
    arr as engine_arr, encode_checkpoint_value, int as engine_int, obj as engine_obj, str as engine_str, SaveJson,
    SaveReader,
};

use super::super::types::SimulationPresentation;
use super::presentation::{q3_pool_models, q3_pool_presentation_state};
use super::types::{
    Q3ArsenalCategory, Q3SourceBots, Q3SourceEntityEvent, Q3SourceHost, Q3SourceOptions, Q3SourcePresentationState,
    Q3SourceScene, Q3SourceSessionCarry, Q3SourceSessionClient,
};

/// Admission rejection from [`Q3SourceRuntime::admit_player`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ClientAdmissionDenied {
    /// Rejection reason.
    pub reason: String,
}

impl fmt::Display for Q3ClientAdmissionDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Q3 client admission denied: {}", self.reason)
    }
}

impl std::error::Error for Q3ClientAdmissionDenied {}

/// Construction mode: fresh map or native-state restore.
#[derive(Debug, Clone)]
pub enum Q3SourceConstruction {
    /// Fresh map load.
    New,
    /// Restore from captured native state.
    Restore(SaveJson),
}

/// Shared runtime core behind every host adapter.
struct Q3SourceRuntimeCore {
    product: Product,
    level: Rc<RefCell<GameLevel>>,
    base_random: Rc<RefCell<GameRandomMirror>>,
    team_random: Rc<TeamGameRandom>,
    records: Q3EntityRecords,
    world: Rc<RuntimeQ3World>,
    base_pool: Rc<RefCell<BaseEntityPool>>,
    team_pool: PoolRef,
    memory: Rc<RefCell<GameMemory>>,
    config: Rc<RefCell<RuntimeConfigRegistry>>,
    registry: Rc<RefCell<ItemRegistry>>,
    remaps: Rc<RefCell<ShaderRemapRegistry>>,
    locations: Rc<RefCell<TargetLocationState>>,
    settings: Rc<Q3GameSettings>,
    bridge: Rc<Q3CombatBridge>,
    missiles: Rc<RefCell<MissileRuntime>>,
    weapons: Rc<RefCell<WeaponRuntime>>,
    item_lifecycle: Rc<RefCell<ItemLifecycleContext>>,
    team: Rc<TeamRuntime>,
    death: Rc<DeathRuntime>,
    think: Rc<ClientThinkRuntime>,
    spawns: Rc<ClientSpawnRuntime>,
    session: Rc<GameSessionManager>,
    session_world: Rc<SessionWorld>,
    match_runtime: Rc<MatchRuntime>,
    arenas: Rc<ArenaRuntime>,
    movers: Rc<RefCell<MoverRuntime>>,
    mover_spawns: Rc<RefCell<RuntimeMoverSpawnHost>>,
    portal: Option<Rc<RefCell<PersonalPortalRuntime>>>,
    admission: Rc<ClientAdmissionRuntime>,
    commands: Rc<GameCommandHostRuntime>,
    server_commands: Rc<GameServerCommandRuntime>,
    links: Rc<RefCell<CoreLinks>>,
    mover_links: Rc<RefCell<MoverSpawnLinks>>,
}

type GameCommandHostRuntime = GameCommandRuntime;

/// Published event mirror per actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PublishedEvent {
    event: i32,
    time: i32,
}

/// Configstring store bridging the host-owned store into the registry.
#[derive(Clone)]
struct SharedConfigStore {
    store: Rc<RefCell<dyn ConfigStringStore>>,
}

impl ConfigStringStore for SharedConfigStore {
    fn get(&self, index: usize) -> String {
        self.store.borrow().get(index)
    }

    fn set(&mut self, index: usize, value: &str) {
        self.store.borrow_mut().set(index, value);
    }
}

/// Configstring registry over the shared host store.
type RuntimeConfigRegistry = ConfigStringRegistry<SharedConfigStore>;

/// Mover-spawn host adapter (donor `MoverSpawnRuntime` services).
struct RuntimeMoverSpawnHost {
    links: Rc<RefCell<MoverSpawnLinks>>,
    mover_core: RuntimeMoverCore,
    combat: RuntimeCombatOps,
    world: RuntimeWorldOps,
}

/// Deferred mover-spawn links filled once the runtime exists.
#[derive(Default)]
struct MoverSpawnLinks {
    host: Option<Rc<dyn Q3SourceHost>>,
    level: Option<Rc<RefCell<GameLevel>>>,
    settings: Option<Rc<Q3GameSettings>>,
    remaps: Option<Rc<RefCell<ShaderRemapRegistry>>>,
    config: Option<Rc<RefCell<RuntimeConfigRegistry>>>,
    core: Option<Rc<RefCell<CoreLinks>>>,
    movers: Option<Rc<RefCell<MoverRuntime>>>,
    driver: Option<Rc<RefCell<RuntimeDriver>>>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    spawn_variables: HashMap<usize, SpawnVariables>,
}

impl RuntimeMoverSpawnHost {
    /// Whether an items-pool slot is a door trigger.
    fn is_door_trigger(&self, slot: usize) -> bool {
        let links = self.links.borrow();
        let pool = links.items_pool.clone().expect("items pool");
        let borrowed = pool.borrow();
        let probed = mover_is_door_trigger(&borrowed, slot).expect("door trigger probe");
        drop(borrowed);
        probed
    }
}

/// Mover core adapter routing items-pool mover calls through the driver.
struct RuntimeMoverCore {
    links: Rc<RefCell<MoverSpawnLinks>>,
}

/// Quake III source game runtime.
pub struct Q3SourceRuntime {
    /// Construction options.
    pub options: Q3SourceOptions,
    /// Session host.
    pub host: Rc<dyn Q3SourceHost>,
    core: Rc<Q3SourceRuntimeCore>,
    mode: Q3SourceConstruction,
    loaded: Cell<bool>,
    retired: Cell<bool>,
    loaded_game_type: Cell<Option<i32>>,
    map_report: RefCell<Option<SpawnReport>>,
    published_events: Rc<RefCell<HashMap<ActorId, PublishedEvent>>>,
    unobserve: RefCell<Option<Box<dyn FnOnce()>>>,
}

/// Settings host over engine cvars with runtime team remaps.
struct RuntimeSettingsHost {
    host: Rc<dyn Q3SourceHost>,
    level: Rc<RefCell<GameLevel>>,
    remaps: Rc<RefCell<ShaderRemapRegistry>>,
    product: Product,
}

impl Q3SettingsHost for RuntimeSettingsHost {
    fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
        self.host.cvars()
    }

    fn send_server_command(&self, client: i32, command: String) {
        self.host.engine().send_server_command(client, &command);
    }

    fn remap_teams(&self) {
        remap_teams(&self.host, &self.remaps, self.product, self.level.borrow().base.time);
    }

    fn format_tracked_change(&self, name: &str, value: &str) -> String {
        game_format(
            "print \"Server: %s changed to %s\\n\"",
            &[
                GameFormatArgument::Text(name.to_string()),
                GameFormatArgument::Text(value.to_string()),
            ],
        )
    }
}

/// Remap team shaders and publish the shader-state configstring.
fn remap_teams(
    host: &Rc<dyn Q3SourceHost>,
    remaps: &Rc<RefCell<ShaderRemapRegistry>>,
    product: Product,
    level_time: i32,
) {
    if product != Product::Missionpack {
        return;
    }
    let cvars = host.cvars();
    let red = cvars.borrow().variable_string("g_redteam");
    let blue = cvars.borrow().variable_string("g_blueteam");
    let seconds = f64::from(level_time as f32 * 0.001);
    for suffix in ["01", "02"] {
        remaps
            .borrow_mut()
            .add(
                &format!("textures/ctf2/redteam{suffix}"),
                &format!("team_icon/{red}_red"),
                seconds,
            )
            .expect("remap red team shader");
        remaps
            .borrow_mut()
            .add(
                &format!("textures/ctf2/blueteam{suffix}"),
                &format!("team_icon/{blue}_blue"),
                seconds,
            )
            .expect("remap blue team shader");
    }
    let state = remaps
        .borrow()
        .build_shader_state_config()
        .expect("shader state config");
    host.configstrings().borrow_mut().set(24, &state);
}

/// Cvar registry adapter for arena services.
struct RuntimeArenaCvars {
    cvars: Rc<RefCell<CvarRegistry>>,
}

impl SupportCvarRegistry for RuntimeArenaCvars {
    fn get(&self, name: &str) -> Option<Q3CvarSnapshot> {
        self.cvars.borrow().get(name).map(|current| Q3CvarSnapshot {
            name: current.name,
            value: current.value,
            reset_value: current.reset_value,
            latched_value: current.latched_value,
            flags: current.flags as i32,
            modified: current.modified,
            modification_count: current.modification_count as i32,
            numeric_value: f64::from(current.numeric_value),
            integer_value: current.integer_value,
        })
    }

    fn set(&self, name: &str, value: &str, force: bool) {
        let _ = self.cvars.borrow_mut().set(name, value, force);
    }
}

/// Configstring adapter for arena services.
struct RuntimeArenaConfig {
    store: Rc<RefCell<dyn ConfigStringStore>>,
}

impl ConfigStrings for RuntimeArenaConfig {
    fn model_index(&self, name: &str) -> i32 {
        let mut registry = RuntimeConfigRegistry::new(SharedConfigStore {
            store: self.store.clone(),
        });
        registry.model_index(Some(name)).expect("arena model index") as i32
    }
}

/// Record host over the session host plus combat bridge wiring.
struct RuntimeRecordHost {
    links: Rc<RefCell<RecordLinks>>,
}

/// Deferred record links filled once the runtime exists.
#[derive(Default)]
struct RecordLinks {
    host: Option<Rc<dyn Q3SourceHost>>,
    bridge: Option<Rc<Q3CombatBridge>>,
    pool: Option<Rc<RefCell<BaseEntityPool>>>,
    records: Option<Q3EntityRecords>,
    combat: Option<Rc<RefCell<CombatContext>>>,
    level: Option<Rc<RefCell<GameLevel>>>,
}

impl RuntimeRecordHost {
    fn host(&self) -> Rc<dyn Q3SourceHost> {
        self.links.borrow().host.clone().expect("record host")
    }
}

impl Q3RecordHost for RuntimeRecordHost {
    fn actors(&self) -> Rc<dyn Q3SessionActors> {
        self.host().actors()
    }

    fn bodies(&self) -> Rc<dyn Q3SessionBodies> {
        self.host().bodies()
    }

    fn combat(&self) -> Rc<dyn Q3SessionCombat> {
        self.host().combat()
    }

    fn inventory(&self) -> Rc<dyn Q3SessionInventory> {
        self.host().inventory()
    }

    fn callbacks(&self) -> Rc<dyn Q3ActorCallbacks> {
        self.host().callbacks()
    }

    fn ammo_timer_stored(&self, actor: &ActorId, weapon: usize, value: i32) {
        self.host().ammo_timer_stored(actor, weapon, value);
    }

    fn schedule(&self, actor: &OwnedActor, due_milliseconds: Option<i32>) {
        self.host().schedule(actor, due_milliseconds);
    }

    fn run_think(&self, actor: &OwnedActor, time_milliseconds: i32) {
        self.host().run_think(actor, time_milliseconds);
    }

    fn damage_call(&self) -> Option<Q3DamageCall> {
        self.links
            .borrow()
            .bridge
            .clone()
            .and_then(|bridge| bridge.current_call())
    }

    fn admit_damage(&self, entity: RecordsEntityRef, request: &DamageRequest) -> DamageAdmission {
        let links = self.links.borrow();
        let (Some(pool), Some(records), Some(combat), Some(level)) = (
            links.pool.clone(),
            links.records.clone(),
            links.combat.clone(),
            links.level.clone(),
        ) else {
            return DamageAdmission::Continue;
        };
        drop(links);
        combat.borrow_mut().intermission_queued = level.borrow().base.intermission_queued;
        let slot = entity.borrow().slot;
        let (target, inflictor, attacker) = {
            let borrowed = pool.borrow();
            let world = borrowed.at(1022);
            let convert = |participant: RecordsDamageParticipant| -> EntitiesDamageParticipant {
                match participant {
                    RecordsDamageParticipant::Native(native) => {
                        EntitiesDamageParticipant::Native(borrowed.at(native.borrow().slot as i32))
                    }
                    RecordsDamageParticipant::SharedActor(shared) => EntitiesDamageParticipant::Shared(SharedActor {
                        actor: shared.actor.clone(),
                        origin: shared.origin(),
                    }),
                }
            };
            let target = borrowed.at(slot as i32);
            let inflictor = match request.attack.inflictor.clone() {
                Some(actor) => records
                    .use_participant(Some(&actor))
                    .map(&convert)
                    .unwrap_or_else(|| EntitiesDamageParticipant::Native(world.clone())),
                None => EntitiesDamageParticipant::Native(world.clone()),
            };
            let attacker = match request.attack.attacker.clone() {
                Some(actor) => records
                    .use_participant(Some(&actor))
                    .map(&convert)
                    .unwrap_or_else(|| EntitiesDamageParticipant::Native(world.clone())),
                None => EntitiesDamageParticipant::Native(world),
            };
            (target, inflictor, attacker)
        };
        let context = combat.borrow();
        match q3_admit_target_damage(&context, &target, &inflictor, &attacker) {
            AdmitDecision::Continue => DamageAdmission::Continue,
            AdmitDecision::Handled => DamageAdmission::Handled,
        }
    }

    fn foreign(&self, actor: &ActorId) -> Option<RecordsEntityRef> {
        self.host().foreign(actor)
    }

    fn is_player(&self, actor: &ActorId) -> bool {
        self.host().is_player(actor)
    }
}

/// Map a session actor to its items-pool actor through the records slot.
fn items_actor_for(actor: &ActorId, links: &CoreLinks) -> Option<ItemsActorId> {
    let records = links.records.clone().expect("records");
    records
        .native_by_actor(Some(actor))
        .map(|entity| ItemsActorId::from_slot(entity.borrow().slot))
}

/// Records-backed pool view for the combat bridge.
struct RuntimeRecordsPoolView {
    records: Q3EntityRecords,
}

impl RecordsEntityPool for RuntimeRecordsPoolView {
    fn num_entities(&self) -> usize {
        MAX_GENTITIES
    }

    fn entity_at(&self, index: usize) -> RecordsEntityRef {
        self.records.get(index).expect("bridge entity slot")
    }
}

/// Apply velocity feedback mutations to a session body.
fn apply_damage_feedback(host: &Rc<dyn Q3SourceHost>, owned: &OwnedActor, decision: &RecordsDamageDecision) {
    let bodies = host.bodies();
    let Some(mut body) = bodies.read(owned.id()) else {
        return;
    };
    let mut touched = false;
    for mutation in &decision.mutations {
        match mutation {
            RecordsDamageMutation::SourceVelocity { after, .. } => {
                body.velocity = *after;
                touched = true;
            }
            RecordsDamageMutation::Impulse { impulse, .. } => {
                body.velocity = Vec3 {
                    x: body.velocity.x + impulse.x,
                    y: body.velocity.y + impulse.y,
                    z: body.velocity.z + impulse.z,
                };
                touched = true;
            }
            _ => {}
        }
    }
    if touched {
        bodies.write(owned, body);
    }
}

/// Combat bridge host over session combat plus team damage rules.
struct RuntimeBridgeHost {
    core: Rc<RefCell<CoreLinks>>,
}

#[derive(Default)]
struct CoreLinks {
    host: Option<Rc<dyn Q3SourceHost>>,
    product: Option<Product>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    records: Option<Q3EntityRecords>,
    world: Option<Rc<RuntimeQ3World>>,
    weapon_provider: Option<ProviderId>,
    combat_provider: Option<ProviderId>,
    inventory_provider: Option<ProviderId>,
    movement_provider: Option<ProviderId>,
    level: Option<Rc<RefCell<GameLevel>>>,
    settings: Option<Rc<Q3GameSettings>>,
    team: Option<Rc<TeamRuntime>>,
    team_pool: Option<PoolRef>,
    missiles: Option<Rc<RefCell<MissileRuntime>>>,
    config: Option<Rc<RefCell<RuntimeConfigRegistry>>>,
    driver: Option<Rc<RefCell<RuntimeDriver>>>,
    item_table: Option<Rc<EntitiesItemTable>>,
    weapons: Option<Rc<RefCell<WeaponRuntime>>>,
    combat: Option<Rc<RefCell<CombatContext>>>,
    random: Option<Rc<RefCell<GameRandomMirror>>>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    portal: Option<Option<Rc<RefCell<PersonalPortalRuntime>>>>,
    missile_host: Option<Rc<RefCell<RuntimeMissileHost>>>,
    death: Option<Rc<DeathRuntime>>,
    effects: Option<EffectsRef>,
    pending_targets: Vec<(usize, Option<StateParticipant>)>,
}

impl Q3CombatBridgeHost for RuntimeBridgeHost {
    fn authority(&self) -> Rc<dyn qa_content::q3::base::records::Q3SessionCombat> {
        self.core.borrow().host.clone().expect("bridge host").combat()
    }

    fn entities(&self) -> RecordsPoolRef {
        Rc::new(RuntimeRecordsPoolView {
            records: self.core.borrow().records.clone().expect("bridge records"),
        })
    }

    fn records(&self) -> Q3EntityRecords {
        self.core.borrow().records.clone().expect("bridge records")
    }

    fn world(&self) -> Rc<dyn Q3ServerWorld> {
        self.core.borrow().world.clone().expect("bridge world")
    }

    fn weapon_provider(&self) -> ProviderId {
        self.core.borrow().weapon_provider.clone().expect("weapon provider")
    }

    fn combat_provider(&self) -> ProviderId {
        self.core.borrow().combat_provider.clone().expect("combat provider")
    }

    fn inventory_provider(&self) -> ProviderId {
        self.core
            .borrow()
            .inventory_provider
            .clone()
            .expect("inventory provider")
    }

    fn movement_provider(&self) -> ProviderId {
        self.core.borrow().movement_provider.clone().expect("movement provider")
    }

    fn armor_context(&self, request: &DamageRequest) -> VictimArmorContext {
        self.core
            .borrow()
            .host
            .clone()
            .expect("bridge host")
            .armor_context(request)
    }

    fn time(&self) -> i32 {
        self.core.borrow().level.clone().expect("level").borrow().base.time
    }

    fn intermission_queued(&self) -> i32 {
        self.core
            .borrow()
            .level
            .clone()
            .expect("level")
            .borrow()
            .base
            .intermission_queued
    }

    fn game_type(&self) -> i32 {
        self.core
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_gametype")
    }

    fn friendly_fire(&self) -> bool {
        self.core
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_friendlyFire")
            != 0
    }

    fn knockback(&self) -> f32 {
        self.core
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .number("g_knockback")
    }

    fn product(&self) -> Product {
        self.core.borrow().product.expect("product")
    }

    fn check_hurt_carrier(&self, target: RecordsEntityRef, attacker: RecordsEntityRef) {
        let links = self.core.borrow();
        let team = links.team.clone().expect("team");
        let pool = links.team_pool.clone().expect("team pool");
        team.check_hurt_carrier(&pool.at(target.borrow().slot), &pool.at(attacker.borrow().slot));
    }

    fn log_accuracy_hit(&self, target: RecordsEntityRef, attacker: RecordsEntityRef) -> bool {
        let links = self.core.borrow();
        let game_type = links.settings.clone().expect("settings").integer("g_gametype");
        let target_slot = target.borrow().slot;
        let attacker_slot = attacker.borrow().slot;
        let driver = links.driver.clone().expect("accuracy driver");
        drop(links);
        let mut driver = driver.borrow_mut();
        log_accuracy_hit(game_type, &mut *driver, target_slot, attacker_slot)
    }

    fn damage_feedback(&self, call: &Q3DamageCall, decision: &RecordsDamageDecision) {
        let links = self.core.borrow();
        let host = links.host.clone().expect("feedback host");
        let owned = call.target.borrow().actor();
        apply_damage_feedback(&host, &owned, decision);
    }

    fn foreign_damage_feedback(
        &self,
        target: RecordsEntityRef,
        _owner: Option<RecordsEntityRef>,
        decision: &RecordsDamageDecision,
    ) {
        let links = self.core.borrow();
        let host = links.host.clone().expect("feedback host");
        let owned = target.borrow().actor();
        apply_damage_feedback(&host, &owned, decision);
    }

    fn projectile_parent(&self, actor: &ActorId) -> Option<ActorId> {
        let links = self.core.borrow();
        let missiles = links.missiles.clone().expect("missiles");
        let items_pool = links.items_pool.clone().expect("items pool");
        let base_pool = links.base_pool.clone().expect("base pool");
        let parent = missiles
            .borrow()
            .owner_of(&items_pool.borrow(), items_actor_for(actor, &links)?);
        parent
            .and_then(|parent| parent.slot())
            .map(|slot| base_pool.borrow().at(slot as i32).borrow().actor.id.clone())
    }

    fn check_obelisk_attack(&self, target: RecordsEntityRef, attacker: RecordsEntityRef) -> bool {
        let links = self.core.borrow();
        let team = links.team.clone().expect("team");
        let pool = links.team_pool.clone().expect("team pool");
        team.check_obelisk_attack(&pool.at(target.borrow().slot), &pool.at(attacker.borrow().slot))
    }

    fn invulnerability_effect(&self, target: RecordsEntityRef, direction: Vec3, point: Vec3) {
        let links = self.core.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("invulnerability driver");
        let slot = target.borrow().slot;
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        invulnerability_effect(&mut *driver.borrow_mut(), slot, direction, point).expect("invulnerability effect");
        reconcile_and_sync_state_to_entities(&pool, &driver, slot);
        reconcile_spawn_union(&self.core);
        drain_pending_targets(&self.core);
    }
}

/// Adapter host: scene queries with runtime-owned curve cvars.
struct RuntimeAdapterHost {
    scene: Rc<dyn Q3SourceScene>,
    cvars: Rc<RefCell<CvarRegistry>>,
}

impl Q3WorldAdapterHost for RuntimeAdapterHost {
    fn trace_scene(&self, query: &Q3TraceQuery) -> Q3TraceResult {
        self.scene.trace_scene(query)
    }

    fn point_contents_scene(&self, query: &Q3TraceQuery, point: Vec3) -> i32 {
        self.scene.point_contents_scene(query, point)
    }

    fn query_actors(&self, bounds: Bounds) -> Vec<ActorId> {
        self.scene.query_actors(bounds)
    }

    fn spatial_collision(&self, actor: &ActorId) -> Option<ActorCollision> {
        self.scene.spatial_collision(actor)
    }

    fn body_state(&self, actor: &ActorId) -> Option<BodyState> {
        self.scene.body_state(actor)
    }

    fn linked_body(&self, actor: &ActorId) -> Option<LinkedBody> {
        self.scene.linked_body(actor)
    }

    fn set_collision(&self, actor: &OwnedActor, collision: ActorCollision) {
        self.scene.set_collision(actor, collision);
    }

    fn link_body(&self, actor: &OwnedActor, origin: Option<Vec3>) {
        self.scene.link_body(actor, origin);
    }

    fn unlink_body(&self, actor: &OwnedActor) {
        self.scene.unlink_body(actor);
    }

    fn curves(&self) -> bool {
        self.cvars.borrow().variable_value("cm_noCurves") == 0.0
    }

    fn player_curve_clip(&self) -> bool {
        self.cvars.borrow().variable_value("cm_playerCurveClip") != 0.0
    }

    fn geometry_trace_start_solid(&self, query: &Q3TraceQuery, model: i32, origin: Vec3, angles: Vec3) -> bool {
        self.scene.geometry_trace_start_solid(query, model, origin, angles)
    }

    fn body_trace_start_solid(&self, query: &Q3TraceQuery, body: &BodyState, collision: &ActorCollision) -> bool {
        self.scene.body_trace_start_solid(query, body, collision)
    }
}

/// Shared world: adapter as base server world and team world.
struct RuntimeQ3World {
    adapter: Q3WorldAdapter,
    base_pool: RefCell<Option<Rc<RefCell<BaseEntityPool>>>>,
    records: RefCell<Option<Q3EntityRecords>>,
}

impl ServerWorld for RuntimeQ3World {
    fn entity_contact(&self, bounds: Bounds, entity_num: i32, capsule: bool) -> bool {
        self.adapter.entity_contact(bounds, entity_num, capsule)
    }

    fn trace(&self, query: &ServerTraceQuery) -> ServerTraceResult {
        self.adapter.trace(query)
    }

    fn area_entities(&self, bounds: Bounds, maximum: usize) -> Vec<i32> {
        self.adapter.area_entities(bounds, maximum)
    }

    fn point_contents(&self, point: Vec3, pass_entity_num: i32) -> i32 {
        self.adapter.point_contents(point, pass_entity_num)
    }

    fn link_state(&self, number: i32) -> Option<LinkState> {
        self.adapter.link_state(number)
    }

    fn link(&self, entity: RecordsEntityRef) {
        self.adapter.link(entity);
    }

    fn unlink(&self, number: i32) {
        self.adapter.unlink(number);
    }
}

impl ActorSpatialQueries for RuntimeQ3World {
    fn area_actors(&self, bounds: Bounds, maximum: usize) -> Vec<ActorId> {
        self.adapter.area_actors(bounds, maximum)
    }

    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult {
        self.adapter.trace_actor(query)
    }

    fn contact_actor(&self, bounds: Bounds, actor: &ActorId, capsule: bool) -> bool {
        self.adapter.contact_actor(bounds, actor, capsule)
    }
}

impl SupportQ3World for RuntimeQ3World {
    fn link(&self, entity: &SupportEntityRef) {
        let slot = entity.borrow().slot;
        if let Some(records) = self.records.borrow().as_ref() {
            if let Some(native) = records.get(slot) {
                self.adapter.link(native);
            }
        }
    }

    fn unlink(&self, number: i32) {
        self.adapter.unlink(number);
    }

    fn link_state(&self, number: i32) -> Option<LinkState> {
        self.adapter.link_state(number)
    }

    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32 {
        self.adapter.point_contents(point, pass_entity)
    }

    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult {
        self.adapter.trace_actor(query)
    }

    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId> {
        self.adapter.area_actors(*bounds, maximum)
    }

    fn contact_actor(&self, bounds: &Bounds, actor: &ActorId) -> bool {
        self.adapter.contact_actor(*bounds, actor, true)
    }
}

/// Think host over the shared core.
struct RuntimeThinkHost {
    think_links: Rc<RefCell<ThinkLinks>>,
}

#[derive(Default)]
struct ThinkLinks {
    team_pool: Option<PoolRef>,
    world: Option<Rc<RuntimeQ3World>>,
    effects: Option<EffectsCoreRef>,
    items: Option<Rc<dyn ItemHost>>,
    level: Option<Rc<RefCell<GameLevel>>>,
    settings: Option<Rc<Q3GameSettings>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    records: Option<Q3EntityRecords>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    missiles: Option<Rc<RefCell<MissileRuntime>>>,
    weapons: Option<Rc<RefCell<WeaponRuntime>>>,
    spawns: Option<Rc<ClientSpawnRuntime>>,
    mover_spawns: Option<Rc<RefCell<RuntimeMoverSpawnHost>>>,
    commands: Option<Rc<GameCommandRuntime>>,
    admission: Option<Rc<ClientAdmissionRuntime>>,
    combat: Option<Rc<RefCell<CombatContext>>>,
    product: Option<Product>,
    think: Option<Rc<ClientThinkRuntime>>,
    driver: Option<Rc<RefCell<RuntimeDriver>>>,
    policy: Option<Rc<RuntimePolicyHost>>,
    lifecycle: Option<Rc<RefCell<ItemLifecycleContext>>>,
    item_table: Option<Rc<EntitiesItemTable>>,
    weapon_host: Option<Rc<dyn WeaponHost>>,
    spawn_selector: Option<Rc<dyn SpawnSelector>>,
    drop_host: Option<Rc<dyn DropHost>>,
    teleport_host: Option<Rc<dyn TeleportHost>>,
    combat_handle: Option<CombatRef>,
    portal_host: Option<Rc<dyn PortalHost>>,
    core: Option<Rc<RefCell<CoreLinks>>>,
}

impl MovementHost for RuntimeThinkHost {
    fn move_client(
        &self,
        entity: &SupportEntityRef,
        command: &Q3UserCommand,
        options: &ClientMovementOptions,
    ) -> qa_content::q3::team_arena::movement_host::ClientMovementResult {
        self.think_links
            .borrow()
            .host
            .clone()
            .expect("host")
            .move_client(entity, command, options)
    }
}

impl ClientThinkHost for RuntimeThinkHost {
    fn pool(&self) -> PoolRef {
        self.think_links.borrow().team_pool.clone().expect("team pool")
    }

    fn world(&self) -> WorldRef {
        self.think_links.borrow().world.clone().expect("world")
    }

    fn touches(&self) -> Rc<dyn TouchAccess> {
        Rc::new(RuntimeTouches {
            links: self.think_links.clone(),
        })
    }

    fn effects(&self) -> EffectsCoreRef {
        self.think_links.borrow().effects.clone().expect("effects")
    }

    fn items(&self) -> Rc<dyn ItemHost> {
        self.think_links.borrow().items.clone().expect("items")
    }

    fn timer_ownership(&self, actor: &ActorId) -> Option<ClientTimerOwnership> {
        self.think_links
            .borrow()
            .host
            .clone()
            .expect("host")
            .timer_ownership(actor)
    }

    fn speed_multiplier(&self, actor: &ActorId) -> Option<f32> {
        self.think_links
            .borrow()
            .host
            .clone()
            .expect("host")
            .speed_multiplier(actor)
    }

    fn frame(&self) -> ClientThinkFrame {
        let links = self.think_links.borrow();
        let level_rc = links.level.clone().expect("level");
        let level = level_rc.borrow();
        ClientThinkFrame {
            time: level.base.time,
            intermission_time: level.base.intermission_time,
            intermission_queued: level.base.intermission_queued,
        }
    }

    fn settings(&self) -> ClientThinkSettings {
        let links = self.think_links.borrow();
        let settings = links.settings.clone().expect("settings");
        let product = links.product.expect("product");
        let single_player = product == Product::Missionpack && settings.integer("ui_singlePlayerActive") != 0;
        ClientThinkSettings {
            synchronous_clients: settings.integer("g_synchronousClients") != 0,
            pmove_fixed: settings.integer("pmove_fixed") != 0,
            debug_move: settings.integer("g_debugMove"),
            pmove_msec: settings.integer("pmove_msec"),
            gravity: settings.number("g_gravity"),
            speed: settings.number("g_speed"),
            dmflags: settings.integer("dmflags"),
            smooth_clients: settings.integer("g_smoothClients") != 0,
            force_respawn_seconds: settings.integer("g_forcerespawn"),
            single_player,
        }
    }

    fn set_pmove_msec(&self, ms: i32) {
        let host = self.think_links.borrow().host.clone().expect("host");
        let _ = host.cvars().borrow_mut().set("pmove_msec", &ms.to_string(), true);
    }

    fn intermission_think(&self, client: &SupportClientRef) {
        client_intermission_think(client);
    }

    fn spectator_think(&self, entity: &SupportEntityRef, command: &Q3UserCommand) {
        let policy = self.think_links.borrow().policy.clone().expect("policy");
        spectator_think(&*policy, entity, command);
    }

    fn check_inactivity(&self, client: &SupportClientRef) -> bool {
        let policy = self.think_links.borrow().policy.clone().expect("policy");
        client_inactivity_timer(&*policy, client)
    }

    fn free_hook(&self, hook: &SupportEntityRef) {
        let links = self.think_links.borrow();
        let missiles = links.missiles.clone().expect("missiles");
        let pool = links.items_pool.clone().expect("items pool");
        let slot = hook.borrow().slot;
        missiles
            .borrow_mut()
            .hook_free(&mut pool.borrow_mut(), slot)
            .expect("free grapple hook");
    }

    fn check_gauntlet_attack(&self, entity: &SupportEntityRef) -> bool {
        let links = self.think_links.borrow();
        let host = links.host.clone().expect("host");
        let weapons = links.weapons.clone().expect("weapons");
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("gauntlet driver");
        let settings = links.settings.clone().expect("settings");
        let slot = entity.borrow().slot;
        let actor = pool.borrow().at(slot as i32).borrow().actor.id.clone();
        if !host.primary_attack_allowed(&actor) {
            return false;
        }
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        weapons.borrow_mut().quad_factor = settings.number("g_quadfactor");
        let hit = {
            let mut driver = driver.borrow_mut();
            weapons
                .borrow()
                .check_gauntlet_attack(&mut *driver, slot)
                .expect("gauntlet attack")
        };
        let core = links.core.clone().expect("core");
        drop(links);
        reconcile_and_sync_state_to_entities(&pool, &driver, slot);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
        hit
    }

    fn client_events(&self, entity: &SupportEntityRef, old_sequence: i32) {
        run_client_events(&self.think_links, entity, old_sequence);
    }

    fn respawn(&self, entity: &SupportEntityRef) {
        self.think_links
            .borrow()
            .spawns
            .clone()
            .expect("spawns")
            .respawn(entity);
    }

    fn append_console_command(&self, command: &str) {
        self.think_links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .append_console_command(command);
    }

    fn is_door_trigger(&self, entity: &SupportEntityRef) -> bool {
        let links = self.think_links.borrow();
        let spawns = links.mover_spawns.clone().expect("mover spawns");
        let borrowed = spawns.borrow();
        let probed = borrowed.is_door_trigger(entity.borrow().slot);
        drop(borrowed);
        probed
    }

    fn bot_test_aas(&self, origin: Vec3) {
        let host = self.think_links.borrow().host.clone().expect("host");
        if let Q3SourceBots::Available { test_aas, .. } = host.bots() {
            test_aas(origin);
        }
    }
}

/// Touch dispatch over records and session callbacks.
struct RuntimeTouches {
    links: Rc<RefCell<ThinkLinks>>,
}

impl TouchAccess for RuntimeTouches {
    fn native(&self, actor: &ActorId) -> Option<SupportEntityRef> {
        let links = self.links.borrow();
        let records = links.records.clone().expect("records");
        let pool = links.team_pool.clone().expect("team pool");
        records
            .native_by_actor(Some(actor))
            .map(|base| pool.at(base.borrow().slot))
    }

    fn is_trigger(&self, actor: &ActorId) -> bool {
        let links = self.links.borrow();
        let host = links.host.clone().expect("host");
        host.scene()
            .spatial_collision(actor)
            .map(|collision| collision.role == ActorCollisionRole::Trigger)
            .unwrap_or(false)
    }

    fn touch(&self, this: &ActorId, other: &ActorId) {
        let links = self.links.borrow();
        let host = links.host.clone().expect("host");
        let records = links.records.clone().expect("records");
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("touch driver");
        let Some(native) = records.native_by_actor(Some(this)) else {
            return;
        };
        let slot = native.borrow().slot;
        if host.actors().resolve_owned(this).is_none() {
            return;
        }
        let entities_touch = pool.borrow().at(slot as i32).borrow().touch.clone();
        if let Some(touch) = entities_touch {
            let entity = pool.borrow().at(slot as i32);
            let participant = entities_participant_for_actor(&pool.borrow(), &records, other);
            let contact = TouchContactMirror {
                this_actor: entity.borrow().actor.clone(),
                other: other.clone(),
                plane: None,
                surface: None,
            };
            touch(entity, participant, contact);
        }
        let state_touch = driver.borrow().state_touch(slot);
        if let Some(touch) = state_touch {
            let participant = Q3Driver::participant(&*driver.borrow(), other);
            let contact = StateTouchContact {
                self_actor: this.clone(),
                other: other.clone(),
                plane: None,
                surface: None,
            };
            touch(&mut *driver.borrow_mut(), slot, &participant, &contact);
        }
        let core = links.core.clone().expect("core");
        drop(links);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }
}

/// Item services bridging support entities to the base lifecycle.
struct RuntimeItemHost {
    links: Rc<RefCell<CoreLinks>>,
    lifecycle: Rc<RefCell<Option<ItemLifecycleContext>>>,
}

impl ItemHost for RuntimeItemHost {
    fn item_at(&self, _product: Product, index: usize) -> EntitiesItemDefinition {
        let links = self.links.borrow();
        let table = links.item_table.clone().expect("item table");
        #[allow(clippy::cast_possible_wrap)]
        table.item_at(index as i32).clone()
    }

    fn find_item(&self, _product: Product, name: &str) -> Option<EntitiesItemDefinition> {
        let links = self.links.borrow();
        let table = links.item_table.clone().expect("item table");
        table.find_item(name).cloned()
    }

    fn find_item_for_powerup(&self, _product: Product, powerup: i32) -> Option<EntitiesItemDefinition> {
        let links = self.links.borrow();
        let table = links.item_table.clone().expect("item table");
        table.find_item_for_powerup(powerup).cloned()
    }

    fn spawn_item(
        &self,
        entity: &SupportEntityRef,
        item: &qa_content::q3::base::game::entities::ItemDefinition,
        vars: &SpawnVariables,
        disabled: bool,
    ) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let lifecycle = self.lifecycle.borrow().clone().expect("lifecycle");
        let slot = entity.borrow().slot;
        let base = pool.borrow().at(slot as i32);
        lifecycle_spawn_item(&base, item, vars, &|| disabled, &lifecycle);
    }

    fn finish_spawning_item(&self, entity: &SupportEntityRef) {
        let pool = self.links.borrow().base_pool.clone().expect("base pool");
        let lifecycle = self.lifecycle.borrow().clone().expect("lifecycle");
        let slot = entity.borrow().slot;
        let base = pool.borrow().at(slot as i32);
        lifecycle_finish_spawning_item(&base, &lifecycle);
    }

    fn touch_item(&self, entity: &SupportEntityRef, other: &SupportDamageParticipant, _contact: &TouchContact) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let lifecycle = self.lifecycle.borrow().clone().expect("lifecycle");
        let team_pool = links.team_pool.clone().expect("team pool");
        let slot = entity.borrow().slot;
        let base = pool.borrow().at(slot as i32);
        let borrowed = pool.borrow();
        let participant = match other {
            SupportDamageParticipant::Entity(entity) => {
                EntitiesDamageParticipant::Native(borrowed.at(entity.borrow().slot as i32))
            }
            SupportDamageParticipant::SharedActor(actor) => EntitiesDamageParticipant::Shared(SharedActor {
                actor: actor.clone(),
                origin: None,
            }),
        };
        let other_actor = match other {
            SupportDamageParticipant::Entity(entity) => entity.borrow().actor.clone(),
            SupportDamageParticipant::SharedActor(actor) => actor.clone(),
        };
        let contact = TouchContactMirror {
            this_actor: base.borrow().actor.clone(),
            other: other_actor,
            plane: None,
            surface: None,
        };
        drop(borrowed);
        lifecycle_touch_item(&base, participant, &contact, &lifecycle);
        sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        drop(links);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }
}

/// Client policy host over think links (donor `policy()`).
struct RuntimePolicyHost {
    links: Rc<RefCell<ThinkLinks>>,
}

impl MovementHost for RuntimePolicyHost {
    fn move_client(
        &self,
        entity: &SupportEntityRef,
        command: &Q3UserCommand,
        options: &ClientMovementOptions,
    ) -> ClientMovementResult {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .move_client(entity, command, options)
    }
}

impl ClientPolicyHost for RuntimePolicyHost {
    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    fn world(&self) -> WorldRef {
        let world: WorldRef = self.links.borrow().world.clone().expect("world");
        world
    }

    fn time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.time
    }

    fn inactivity_seconds(&self) -> i32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_inactivity")
    }

    fn follow1(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.follow1
    }

    fn follow2(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.follow2
    }

    fn touch_triggers(&self, entity: &SupportEntityRef) {
        self.links.borrow().think.clone().expect("think").touch_triggers(entity);
    }

    fn follow_cycle(&self, entity: &SupportEntityRef, direction: i32) {
        self.links
            .borrow()
            .commands
            .clone()
            .expect("commands")
            .follow_cycle(entity, direction);
    }

    fn client_begin(&self, client_num: usize) {
        #[allow(clippy::cast_possible_wrap)]
        self.links
            .borrow()
            .admission
            .clone()
            .expect("admission")
            .begin(client_num as i32);
    }

    fn drop_client(&self, client_num: usize, reason: &str) {
        let host = self.links.borrow().host.clone().expect("host");
        #[allow(clippy::cast_possible_wrap)]
        host.engine().drop_client(client_num as i32, reason);
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .send_server_command(client_num, text);
    }
}

/// Run client events with the missionpack portal when present.
fn run_client_events(links: &Rc<RefCell<ThinkLinks>>, entity: &SupportEntityRef, old_sequence: i32) {
    let borrowed = links.borrow();
    let host = borrowed.host.clone().expect("host");
    let settings = borrowed.settings.clone().expect("settings");
    let product = borrowed.product.expect("product");
    let portal = borrowed.portal_host.clone();
    if product == Product::Missionpack && portal.is_none() {
        panic!("Missionpack portal runtime is missing");
    }
    let world: WorldRef = borrowed.world.clone().expect("world");
    let context = ClientEvents {
        world,
        weapons: borrowed.weapon_host.clone().expect("weapon host"),
        spawns: borrowed.spawn_selector.clone().expect("spawn selector"),
        drops: borrowed.drop_host.clone().expect("drop host"),
        items: borrowed.items.clone().expect("items"),
        teleport: borrowed.teleport_host.clone().expect("teleport host"),
        dmflags: settings.integer("dmflags"),
        primary_attack_allowed: Some(Rc::new(move |actor: &ActorId| host.primary_attack_allowed(actor))),
        product,
        combat: borrowed.combat_handle.clone().expect("combat"),
        personal_portal: portal,
    };
    drop(borrowed);
    client_events(&context, entity, old_sequence);
}

/// Weapon services over the driver weapon runtime (donor `weapons`).
struct RuntimeWeaponHost {
    links: Rc<RefCell<CoreLinks>>,
}

impl WeaponHost for RuntimeWeaponHost {
    fn fire(&self, entity: &SupportEntityRef) {
        let links = self.links.borrow();
        let weapons = links.weapons.clone().expect("weapons");
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("fire driver");
        let settings = links.settings.clone().expect("settings");
        let slot = entity.borrow().slot;
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        weapons.borrow_mut().quad_factor = settings.number("g_quadfactor");
        weapons
            .borrow_mut()
            .fire(&mut *driver.borrow_mut(), slot)
            .expect("fire weapon");
        reconcile_and_sync_state_to_entities(&pool, &driver, slot);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }

    fn start_kamikaze(&self, entity: &SupportEntityRef) {
        let links = self.links.borrow();
        let weapons = links.weapons.clone().expect("weapons");
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("kamikaze driver");
        let slot = entity.borrow().slot;
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        weapons
            .borrow()
            .start_kamikaze(&mut *driver.borrow_mut(), slot)
            .expect("kamikaze");
        reconcile_and_sync_state_to_entities(&pool, &driver, slot);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }
}

/// Spawn selector over the client spawn runtime.
struct RuntimeSpawnSelector {
    spawns: Rc<ClientSpawnRuntime>,
}

impl SpawnSelector for RuntimeSpawnSelector {
    fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint {
        self.spawns.select_spawn_point(avoid)
    }
}

/// Item-drop services over the items pool (donor `drops`).
struct RuntimeDropHost {
    links: Rc<RefCell<CoreLinks>>,
}

impl DropHost for RuntimeDropHost {
    fn drop_item(&self, entity: &SupportEntityRef, item: &EntitiesItemDefinition, angle: i32) -> SupportEntityRef {
        let links = self.links.borrow();
        let host = links.host.clone().expect("host");
        let product = links.product.expect("product");
        let level = links.level.clone().expect("level");
        let settings = links.settings.clone().expect("settings");
        let team_pool = links.team_pool.clone().expect("team pool");
        let team = links.team.clone().expect("team");
        let items_pool = links.items_pool.clone().expect("items pool");
        let random = links.random.clone().expect("random");
        let table = RuntimeItemsTable {
            table: links.item_table.clone().expect("item table"),
        };
        let slot = entity.borrow().slot;
        let game_type = settings.integer("g_gametype");
        let time = level.borrow().base.time;
        let items_def = items_core_item(product, item);
        let mut check_dropped = |pool: &mut ItemsEntityPool, slot: Slot| -> Q3GameItemsResult<()> {
            team.check_dropped_item(&team_pool.at(slot));
            let _ = pool;
            Ok(())
        };
        let mut random_draw = || random.borrow_mut().random_value();
        let mut context = ItemsDropItemContext {
            launch: ItemsLaunchItemContext {
                product,
                game_type,
                time,
                items: &table,
                check_dropped_team_item: &mut check_dropped,
            },
            random: &mut random_draw,
        };
        let dropped = items_core_drop_item(
            &mut items_pool.borrow_mut(),
            slot,
            &mut context,
            &items_def,
            #[allow(clippy::cast_precision_loss)]
            {
                angle as f32
            },
        )
        .expect("drop item");
        drop(links);
        let _ = host;
        let base_pool = self.links.borrow().base_pool.clone().expect("base pool");
        reconcile_pools(&base_pool, &items_pool, &team_pool);
        let product = self.links.borrow().product.expect("product");
        sync_items_to_entities_team(&items_pool.borrow(), &base_pool, &team_pool, product, dropped);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
        team_pool.at(dropped)
    }
}

/// Convert an entities-pool item into the items-core item shape via the shared list.
fn items_core_item(product: Product, item: &EntitiesItemDefinition) -> ItemsCoreItemDefinition {
    let class = item.class_name.as_deref().unwrap_or_default();
    let shared = item_list(product)
        .iter()
        .find(|def| def.class_name.unwrap_or_default() == class)
        .unwrap_or_else(|| panic!("dropped item has no shared definition: {class}"));
    ItemsCoreItemDefinition {
        class_name: item.class_name.clone(),
        quantity: item.quantity,
        kind: items_item_kind(shared.kind),
    }
}

/// Map a shared item kind into the items-core kind mirror.
fn items_item_kind(kind: SharedItemKind) -> ItemsItemKind {
    match kind {
        SharedItemKind::Bad => ItemsItemKind::Bad,
        SharedItemKind::Weapon(weapon) => ItemsItemKind::Weapon(weapon),
        SharedItemKind::Ammo(weapon) => ItemsItemKind::Ammo(weapon),
        SharedItemKind::Armor => ItemsItemKind::Armor,
        SharedItemKind::Health => ItemsItemKind::Health,
        SharedItemKind::Powerup(powerup) => ItemsItemKind::Powerup(powerup),
        SharedItemKind::Holdable(holdable) => ItemsItemKind::Holdable(holdable),
        SharedItemKind::PersistantPowerup(powerup) => ItemsItemKind::PersistantPowerup(powerup),
        SharedItemKind::Team(powerup) => ItemsItemKind::Team(powerup),
    }
}

/// Teleport services over the items pool (donor `teleportPlayer`).
struct RuntimeTeleportHost {
    links: Rc<RefCell<CoreLinks>>,
}

impl TeleportHost for RuntimeTeleportHost {
    fn teleport_player(&self, entity: &SupportEntityRef, origin: Vec3, angles: Vec3) {
        let links = self.links.borrow();
        let items_pool = links.items_pool.clone().expect("items pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let base_pool = links.base_pool.clone().expect("base pool");
        let product = links.product.expect("product");
        let slot = entity.borrow().slot;
        let links_ref = self.links.clone();
        let mut combat = RuntimeCombatOps {
            links: links_ref.clone(),
        };
        let mut world = RuntimeWorldOps { links: links_ref };
        let mut context = TeleportContext {
            combat: &mut combat,
            world: &mut world,
        };
        misc_teleport_player(&mut items_pool.borrow_mut(), &mut context, slot, origin, angles)
            .expect("teleport player");
        drop(links);
        reconcile_pools(&base_pool, &items_pool, &team_pool);
        sync_items_to_entities_team(&items_pool.borrow(), &base_pool, &team_pool, product, slot);
        let max = team_pool.max_clients();
        for client in 0..max {
            sync_items_to_entities_team(&items_pool.borrow(), &base_pool, &team_pool, product, client);
        }
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }
}

/// Personal-portal services over the driver portal runtime.
struct RuntimePortalHost {
    links: Rc<RefCell<CoreLinks>>,
}

impl PortalHost for RuntimePortalHost {
    fn drop_portal_source(&self, entity: &SupportEntityRef) {
        self.drop_portal(entity.borrow().slot, true);
    }

    fn drop_portal_destination(&self, entity: &SupportEntityRef) {
        self.drop_portal(entity.borrow().slot, false);
    }
}

impl RuntimePortalHost {
    fn drop_portal(&self, slot: usize, source: bool) {
        let links = self.links.borrow();
        let portal = links.portal.clone().expect("portal").expect("missionpack portal");
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("portal driver");
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        let mut driver = driver.borrow_mut();
        if source {
            portal
                .borrow_mut()
                .drop_portal_source(&mut *driver, slot)
                .expect("portal source");
        } else {
            portal
                .borrow_mut()
                .drop_portal_destination(&mut *driver, slot)
                .expect("portal destination");
        }
        drop(driver);
        reconcile_and_sync_state_to_entities(&pool, &self.links.borrow().driver.clone().expect("driver"), slot);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }
}

/// Team combat services over the base combat module (donor `combat`).
struct RuntimeCombat {
    links: Rc<RefCell<CoreLinks>>,
}

impl SupportCombat for RuntimeCombat {
    fn product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    fn time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.time
    }

    fn game_type(&self) -> i32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_gametype")
    }

    fn intermission_queued(&self) -> i32 {
        self.links
            .borrow()
            .level
            .clone()
            .expect("level")
            .borrow()
            .base
            .intermission_queued
    }

    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    #[allow(clippy::too_many_arguments)]
    fn damage(
        &self,
        target: &SupportEntityRef,
        inflictor: Option<&SupportDamageParticipant>,
        attacker: Option<&SupportDamageParticipant>,
        direction: Option<Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    ) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let records = links.records.clone().expect("records");
        let combat = links.combat.clone().expect("combat");
        let driver = links.driver.clone().expect("damage driver");
        let target_slot = target.borrow().slot;
        refresh_combat_context(&combat, &links);
        let borrowed = pool.borrow();
        let entities_target = EntitiesDamageParticipant::Native(borrowed.at(target_slot as i32));
        let convert = |participant: &SupportDamageParticipant| -> EntitiesDamageParticipant {
            match participant {
                SupportDamageParticipant::Entity(entity) => {
                    EntitiesDamageParticipant::Native(borrowed.at(entity.borrow().slot as i32))
                }
                SupportDamageParticipant::SharedActor(actor) => EntitiesDamageParticipant::Shared(SharedActor {
                    actor: actor.clone(),
                    origin: None,
                }),
            }
        };
        let entities_inflictor = inflictor.map(&convert);
        let entities_attacker = attacker.map(&convert);
        drop(borrowed);
        let mut direction = direction;
        combat_damage(
            &combat.borrow(),
            entities_target,
            entities_inflictor,
            entities_attacker,
            direction.as_mut(),
            point,
            #[allow(clippy::cast_precision_loss)]
            {
                amount as f32
            },
            flags,
            method,
            None,
        );
        sync_state_to_entities_team(&pool, &team_pool, &driver, target_slot);
        if let Some(SupportDamageParticipant::Entity(entity)) = attacker {
            let slot = entity.borrow().slot;
            sync_state_to_entities_team(&pool, &team_pool, &driver, slot);
        }
        let _ = records;
        drop(links);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }
}

/// Items-core world operations over the shared world adapter.
struct RuntimeWorldOps {
    links: Rc<RefCell<CoreLinks>>,
}

impl ItemsWorldOps for RuntimeWorldOps {
    fn link(&mut self, pool: &mut ItemsEntityPool, slot: Slot) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let world = links.world.clone().expect("world");
        let records = links.records.clone().expect("records");
        world.adapter.link(records.get(slot).expect("link slot"));
        if let Some(entity) = pool.get_mut(slot) {
            entity.linked = true;
        }
        Ok(())
    }

    fn unlink(&mut self, _pool: &mut ItemsEntityPool, number: i32) -> Q3GameItemsResult<()> {
        self.links.borrow().world.clone().expect("world").adapter.unlink(number);
        Ok(())
    }

    fn trace_actor(&mut self, _pool: &ItemsEntityPool, query: &ItemsActorTraceQuery) -> ItemsActorTraceResult {
        let links = self.links.borrow();
        let world = links.world.clone().expect("world");
        let pass_entity_num = query
            .pass_actor
            .and_then(|actor| actor.slot())
            .map_or(-1, |slot| slot as i32);
        let server = world.adapter.trace(&ServerTraceQuery {
            start: query.start,
            end: query.end,
            shape: query.shape,
            pass_entity_num,
            mask: query.mask,
        });
        let hit = if server.entity_num < 0 || server.entity_num as usize >= MAX_GENTITIES {
            ItemsTraceHit::None
        } else if server.entity_num == ENTITYNUM_WORLD {
            ItemsTraceHit::World
        } else {
            ItemsTraceHit::Actor(ItemsActorId::from_slot(server.entity_num as usize))
        };
        ItemsActorTraceResult {
            fraction: server.fraction,
            end: server.end,
            solidity: server.solidity,
            contact: match server.contact {
                WorldTraceContact::None => ItemsTraceContact::None,
                WorldTraceContact::Plane { plane } => ItemsTraceContact::Plane { normal: plane.normal },
            },
            contents: server.contents,
            surface_flags: server.surface_flags,
            hit,
        }
    }

    fn point_contents(&mut self, _pool: &ItemsEntityPool, point: Vec3, pass_entity_num: i32) -> i32 {
        self.links
            .borrow()
            .world
            .clone()
            .expect("world")
            .adapter
            .point_contents(point, pass_entity_num)
    }

    fn area_actors(&mut self, _pool: &ItemsEntityPool, bounds: Bounds, maximum: usize) -> Vec<ItemsActorId> {
        let links = self.links.borrow();
        let world = links.world.clone().expect("world");
        let actors = world.adapter.area_actors(bounds, maximum);
        actors
            .into_iter()
            .filter_map(|actor| items_actor_for(&actor, &links))
            .collect()
    }
}

/// Items-core combat operations over the base combat module.
struct RuntimeCombatOps {
    links: Rc<RefCell<CoreLinks>>,
}

impl ItemsCombatOps for RuntimeCombatOps {
    fn time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.time
    }

    fn previous_time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().previous_time
    }

    fn game_type(&self) -> i32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_gametype")
    }

    fn product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        pool: &mut ItemsEntityPool,
        target: Slot,
        inflictor: ItemsDamageParticipant,
        attacker: ItemsDamageParticipant,
        direction: Option<Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
        projectile: Option<ItemsActorId>,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let base_pool = links.base_pool.clone().expect("base pool");
        let combat = links.combat.clone().expect("combat");
        refresh_combat_context(&combat, &links);
        let borrowed = base_pool.borrow();
        let convert = |participant: ItemsDamageParticipant| -> EntitiesDamageParticipant {
            match participant {
                ItemsDamageParticipant::Entity(slot) => EntitiesDamageParticipant::Native(borrowed.at(slot as i32)),
                ItemsDamageParticipant::SharedActor(actor) => {
                    let id = actor
                        .slot()
                        .map(|slot| borrowed.at(slot as i32).borrow().actor.id.clone())
                        .unwrap_or_else(|| borrowed.at(1022).borrow().actor.id.clone());
                    EntitiesDamageParticipant::Shared(SharedActor {
                        actor: id,
                        origin: None,
                    })
                }
            }
        };
        let entities_target = EntitiesDamageParticipant::Native(borrowed.at(target as i32));
        let entities_inflictor = convert(inflictor);
        let entities_attacker = convert(attacker);
        let entities_projectile = projectile
            .and_then(|actor| actor.slot())
            .map(|slot| borrowed.at(slot as i32).borrow().actor.id.clone());
        drop(borrowed);
        let mut direction = direction;
        combat_damage(
            &combat.borrow(),
            entities_target,
            Some(entities_inflictor),
            Some(entities_attacker),
            direction.as_mut(),
            point,
            #[allow(clippy::cast_precision_loss)]
            {
                amount as f32
            },
            flags,
            method,
            entities_projectile,
        );
        let _ = pool;
        Ok(())
    }

    fn can_damage(&mut self, pool: &mut ItemsEntityPool, target: Slot, origin: Vec3) -> bool {
        let links = self.links.borrow();
        let base_pool = links.base_pool.clone().expect("base pool");
        let combat = links.combat.clone().expect("combat");
        refresh_combat_context(&combat, &links);
        let borrowed = base_pool.borrow();
        let participant = EntitiesDamageParticipant::Native(borrowed.at(target as i32));
        let _ = pool;
        let context = combat.borrow();
        combat_can_damage(&context, &participant, origin)
    }

    #[allow(clippy::too_many_arguments)]
    fn radius_damage(
        &mut self,
        pool: &mut ItemsEntityPool,
        origin: Vec3,
        attacker: Slot,
        damage: i32,
        radius: f32,
        ignore: Option<Slot>,
        method: i32,
        projectile: Option<ItemsActorId>,
    ) -> bool {
        let links = self.links.borrow();
        let base_pool = links.base_pool.clone().expect("base pool");
        let combat = links.combat.clone().expect("combat");
        refresh_combat_context(&combat, &links);
        let borrowed = base_pool.borrow();
        let entities_attacker = EntitiesDamageParticipant::Native(borrowed.at(attacker as i32));
        let entities_ignore = ignore.map(|slot| EntitiesDamageParticipant::Native(borrowed.at(slot as i32)));
        let entities_projectile = projectile
            .and_then(|actor| actor.slot())
            .map(|slot| borrowed.at(slot as i32).borrow().actor.id.clone());
        let _ = pool;
        let context = combat.borrow();
        combat_radius_damage(
            &context,
            origin,
            &entities_attacker,
            #[allow(clippy::cast_precision_loss)]
            {
                damage as f32
            },
            radius,
            entities_ignore.as_ref(),
            method,
            entities_projectile,
        )
    }

    fn accuracy_hit(&mut self, _team_game: bool, target: &ItemsAccuracyTarget, attacker: &ItemsAccuracyTarget) -> bool {
        let links = self.links.borrow();
        let settings = links.settings.clone().expect("settings");
        let driver = links.driver.clone().expect("accuracy driver");
        let game_type = settings.integer("g_gametype");
        let (Some(target), Some(attacker)) = (target.actor.slot(), attacker.actor.slot()) else {
            return false;
        };
        drop(links);
        let hit = log_accuracy_hit(game_type, &mut *driver.borrow_mut(), target, attacker);
        hit
    }
}

/// Items-core item table over the shared item list.
struct RuntimeItemsTable {
    table: Rc<EntitiesItemTable>,
}

impl ItemsItemTable for RuntimeItemsTable {
    fn index_of(&self, _product: Product, item: &ItemsCoreItemDefinition) -> Option<usize> {
        let class = item.class_name.as_deref().unwrap_or_default();
        self.table
            .items
            .iter()
            .position(|entry| entry.class_name.as_deref().unwrap_or_default() == class)
    }

    fn item_at(&self, product: Product, index: usize) -> Q3GameItemsResult<ItemsCoreItemDefinition> {
        #[allow(clippy::cast_possible_wrap)]
        let entry = self.table.item_at(index as i32);
        Ok(items_core_item(product, entry))
    }

    fn find_item_for_weapon(&self, product: Product, weapon: Weapon) -> Q3GameItemsResult<ItemsCoreItemDefinition> {
        let shared =
            shared_item_for_weapon(product, weapon).unwrap_or_else(|_| panic!("weapon has no shared item: {weapon:?}"));
        Ok(ItemsCoreItemDefinition {
            class_name: shared.class_name.map(str::to_string),
            quantity: shared.quantity,
            kind: items_item_kind(shared.kind),
        })
    }
}

/// Refresh combat-module snapshot scalars from the live level and settings.
fn refresh_combat_context(combat: &Rc<RefCell<CombatContext>>, links: &CoreLinks) {
    let level_rc = links.level.clone().expect("level");
    let level = level_rc.borrow();
    let settings = links.settings.clone().expect("settings");
    let mut context = combat.borrow_mut();
    context.time = level.base.time;
    context.intermission_queued = level.base.intermission_queued;
    context.game_type = settings.integer("g_gametype");
    context.knockback = settings.number("g_knockback");
}

/// Render a records-side provider id in donor `namespace:name` word form.
fn game_provider_from_records(provider: &ProviderId) -> String {
    format!("{}:{}", provider.namespace, provider.name)
}

/// Parse a donor `namespace:name` provider word into its records-side twin.
fn records_provider_from_game(provider: &str) -> ProviderId {
    match provider.split_once(':') {
        Some((namespace, name)) => ProviderId::new(namespace, name),
        None => ProviderId::new("", provider),
    }
}

/// Convert a records-side attack cause into its game-side twin.
fn game_cause_from_records(cause: RecordsAttackCause) -> GameAttackCause {
    match cause {
        RecordsAttackCause::Q1 {
            death_type,
            armor_effect,
        } => GameAttackCause::Q1 {
            death_type,
            armor_effect: armor_effect.map(|effect| match effect {
                Q1ArmorEffect::Bypass => GameArmorEffect::Bypass,
                Q1ArmorEffect::HalfEffectiveness => GameArmorEffect::HalfEffectiveness,
            }),
        },
        RecordsAttackCause::Q2 {
            means_of_death,
            damage_flags,
            native,
        } => GameAttackCause::Q2 {
            means_of_death,
            damage_flags,
            native: native.map(|cause| match cause {
                RecordsQ2NativeCause::Classic { game, value } => GameQ2NativeCause::Classic {
                    game: match game {
                        Q2ClassicGame::Base => "base".to_owned(),
                        Q2ClassicGame::Xatrix => "xatrix".to_owned(),
                        Q2ClassicGame::Rogue => "rogue".to_owned(),
                        Q2ClassicGame::Ctf => "ctf".to_owned(),
                    },
                    value,
                },
                RecordsQ2NativeCause::Rerelease {
                    id,
                    friendly_fire,
                    no_point_loss,
                } => GameQ2NativeCause::Rerelease {
                    id,
                    friendly_fire,
                    no_point_loss,
                },
            }),
        },
        RecordsAttackCause::Q3 {
            means_of_death,
            damage_flags,
        } => GameAttackCause::Q3 {
            means_of_death,
            damage_flags,
        },
        RecordsAttackCause::Environment { hazard } => GameAttackCause::Environment {
            hazard: match hazard {
                EnvironmentHazard::Fall => EnvHazard::Fall,
                EnvironmentHazard::Drown => EnvHazard::Drown,
                EnvironmentHazard::Lava => EnvHazard::Lava,
                EnvironmentHazard::Slime => EnvHazard::Slime,
                EnvironmentHazard::Crush => EnvHazard::Crush,
                EnvironmentHazard::Trigger => EnvHazard::Trigger,
            },
        },
    }
}

/// Convert a game-side attack cause into its records-side twin.
fn records_cause_from_game(cause: GameAttackCause) -> RecordsAttackCause {
    match cause {
        GameAttackCause::Q1 {
            death_type,
            armor_effect,
        } => RecordsAttackCause::Q1 {
            death_type,
            armor_effect: armor_effect.map(|effect| match effect {
                GameArmorEffect::Bypass => Q1ArmorEffect::Bypass,
                GameArmorEffect::HalfEffectiveness => Q1ArmorEffect::HalfEffectiveness,
            }),
        },
        GameAttackCause::Q2 {
            means_of_death,
            damage_flags,
            native,
        } => RecordsAttackCause::Q2 {
            means_of_death,
            damage_flags,
            native: native.map(|cause| match cause {
                GameQ2NativeCause::Classic { game, value } => RecordsQ2NativeCause::Classic {
                    game: match game.as_str() {
                        "xatrix" => Q2ClassicGame::Xatrix,
                        "rogue" => Q2ClassicGame::Rogue,
                        "ctf" => Q2ClassicGame::Ctf,
                        _ => Q2ClassicGame::Base,
                    },
                    value,
                },
                GameQ2NativeCause::Rerelease {
                    id,
                    friendly_fire,
                    no_point_loss,
                } => RecordsQ2NativeCause::Rerelease {
                    id,
                    friendly_fire,
                    no_point_loss,
                },
            }),
        },
        GameAttackCause::Q3 {
            means_of_death,
            damage_flags,
        } => RecordsAttackCause::Q3 {
            means_of_death,
            damage_flags,
        },
        GameAttackCause::Environment { hazard } => RecordsAttackCause::Environment {
            hazard: match hazard {
                EnvHazard::Fall => EnvironmentHazard::Fall,
                EnvHazard::Drown => EnvironmentHazard::Drown,
                EnvHazard::Lava => EnvironmentHazard::Lava,
                EnvHazard::Slime => EnvironmentHazard::Slime,
                EnvHazard::Crush => EnvironmentHazard::Crush,
                EnvHazard::Trigger => EnvironmentHazard::Trigger,
            },
        },
    }
}

/// Convert a records-side source clock into its game-side twin.
fn game_time_from_records(time: CoreSourceTime) -> GameSourceTime {
    match time {
        CoreSourceTime::Milliseconds(value) => GameSourceTime::Milliseconds { value },
        CoreSourceTime::Seconds(value) => GameSourceTime::Milliseconds {
            value: (value * 1000.0) as i32,
        },
    }
}

/// Convert a game-side source clock into its records-side twin.
fn records_time_from_game(time: GameSourceTime) -> CoreSourceTime {
    match time {
        GameSourceTime::Milliseconds { value } => CoreSourceTime::Milliseconds(value),
    }
}

/// Convert records-side attack provenance into its game-side twin.
fn game_provenance_from_records(provenance: RecordsAttackProvenance) -> GameAttackProvenance {
    GameAttackProvenance {
        sequence: provenance.sequence,
        time: game_time_from_records(provenance.time),
        attacker: provenance.attacker,
        inflictor: provenance.inflictor,
        originating_projectile: provenance.originating_projectile,
        weapon: provenance.weapon,
        weapon_provider: game_provider_from_records(&provenance.weapon_provider),
        damage_powerup_owner: provenance.damage_powerup_owner.as_ref().map(game_provider_from_records),
        combat_provider: game_provider_from_records(&provenance.combat_provider),
        inventory_provider: game_provider_from_records(&provenance.inventory_provider),
        movement_provider: game_provider_from_records(&provenance.movement_provider),
        cause: game_cause_from_records(provenance.cause),
    }
}

/// Convert game-side attack provenance into its records-side twin.
fn records_provenance_from_game(provenance: GameAttackProvenance) -> RecordsAttackProvenance {
    RecordsAttackProvenance {
        sequence: provenance.sequence,
        time: records_time_from_game(provenance.time),
        attacker: provenance.attacker,
        inflictor: provenance.inflictor,
        originating_projectile: provenance.originating_projectile,
        weapon: provenance.weapon,
        weapon_provider: records_provider_from_game(&provenance.weapon_provider),
        damage_powerup_owner: provenance
            .damage_powerup_owner
            .as_ref()
            .map(|owner| records_provider_from_game(owner)),
        combat_provider: records_provider_from_game(&provenance.combat_provider),
        inventory_provider: records_provider_from_game(&provenance.inventory_provider),
        movement_provider: records_provider_from_game(&provenance.movement_provider),
        cause: records_cause_from_game(provenance.cause),
    }
}

/// Convert a records-side damage request into its game-side twin.
fn game_request_from_records(request: DamageRequest) -> GameDamageRequest {
    GameDamageRequest {
        attack: game_provenance_from_records(request.attack),
        target: request.target,
        amount: request.amount,
        knockback: request.knockback,
        direction: request.direction,
        point: request.point,
        normal: request.normal,
        delivery: match request.delivery {
            Delivery::Direct => DamageDelivery::Direct,
            Delivery::Radius => DamageDelivery::Radius,
        },
    }
}

/// Convert a game-side damage request into its records-side twin.
fn records_request_from_game(request: GameDamageRequest) -> DamageRequest {
    DamageRequest {
        attack: records_provenance_from_game(request.attack),
        target: request.target,
        amount: request.amount,
        knockback: request.knockback,
        direction: request.direction,
        point: request.point,
        normal: request.normal,
        delivery: match request.delivery {
            DamageDelivery::Direct => Delivery::Direct,
            DamageDelivery::Radius => Delivery::Radius,
        },
    }
}

/// Convert records-side regular armor into its game-side twin.
fn game_regular_armor_from_records(armor: RegularArmorState) -> RegularArmor {
    match armor {
        RegularArmorState::None => RegularArmor::None,
        RegularArmorState::Q1 {
            points,
            absorption,
            item,
        } => RegularArmor::Q1 {
            points,
            absorption,
            item,
        },
        RegularArmorState::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        } => RegularArmor::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        },
        RegularArmorState::Q3 { points, protection } => RegularArmor::Q3 { points, protection },
        RegularArmorState::Source { points, item } => RegularArmor::Source { points, item },
    }
}

/// Convert game-side regular armor into its records-side twin.
fn records_regular_armor_from_game(armor: RegularArmor) -> RegularArmorState {
    match armor {
        RegularArmor::None => RegularArmorState::None,
        RegularArmor::Q1 {
            points,
            absorption,
            item,
        } => RegularArmorState::Q1 {
            points,
            absorption,
            item,
        },
        RegularArmor::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        } => RegularArmorState::Q2 {
            points,
            normal_protection,
            energy_protection,
            item,
        },
        RegularArmor::Q3 { points, protection } => RegularArmorState::Q3 { points, protection },
        RegularArmor::Source { points, item } => RegularArmorState::Source { points, item },
    }
}

/// Convert records-side armor state into its game-side twin.
fn game_armor_from_records(armor: RecordsArmorState) -> GameArmorState {
    GameArmorState {
        regular: game_regular_armor_from_records(armor.regular),
        powered: match armor.powered {
            PoweredProtectionState::None => PoweredProtection::None,
            PoweredProtectionState::Screen { cells } => PoweredProtection::Screen { cells },
            PoweredProtectionState::Shield { cells } => PoweredProtection::Shield { cells },
        },
    }
}

/// Convert game-side armor state into its records-side twin.
fn records_armor_from_game(armor: GameArmorState) -> RecordsArmorState {
    RecordsArmorState {
        regular: records_regular_armor_from_game(armor.regular),
        powered: match armor.powered {
            PoweredProtection::None => PoweredProtectionState::None,
            PoweredProtection::Screen { cells } => PoweredProtectionState::Screen { cells },
            PoweredProtection::Shield { cells } => PoweredProtectionState::Shield { cells },
        },
    }
}

/// Convert a records-side combat snapshot into its game-side twin.
fn game_state_from_records(state: RecordsCombatState) -> GameCombatState {
    GameCombatState {
        health: state.health,
        armor: game_armor_from_records(state.armor),
        mass: state.mass as f32,
        can_take_damage: state.can_take_damage,
        invulnerable: state.invulnerable,
        no_knockback: state.no_knockback,
        team: state.team,
    }
}

/// Convert a records-side damage mutation into its game-side twin.
fn game_mutation_from_records(mutation: RecordsDamageMutation) -> GameDamageMutation {
    match mutation {
        RecordsDamageMutation::Health { before, after } => GameDamageMutation::Health { before, after },
        RecordsDamageMutation::Armor { before, after } => GameDamageMutation::Armor {
            before: game_armor_from_records(before),
            after: game_armor_from_records(after),
        },
        RecordsDamageMutation::SourceVelocity {
            before,
            after,
            movement_provider,
        } => GameDamageMutation::SourceVelocity {
            before,
            after,
            movement_provider: game_provider_from_records(&movement_provider),
        },
        RecordsDamageMutation::Impulse {
            impulse,
            movement_provider,
        } => GameDamageMutation::Impulse {
            impulse,
            movement_provider: game_provider_from_records(&movement_provider),
        },
    }
}

/// Convert a game-side damage mutation into its records-side twin.
fn records_mutation_from_game(mutation: GameDamageMutation) -> RecordsDamageMutation {
    match mutation {
        GameDamageMutation::Health { before, after } => RecordsDamageMutation::Health { before, after },
        GameDamageMutation::Armor { before, after } => RecordsDamageMutation::Armor {
            before: records_armor_from_game(before),
            after: records_armor_from_game(after),
        },
        GameDamageMutation::SourceVelocity {
            before,
            after,
            movement_provider,
        } => RecordsDamageMutation::SourceVelocity {
            before,
            after,
            movement_provider: records_provider_from_game(&movement_provider),
        },
        GameDamageMutation::Impulse {
            impulse,
            movement_provider,
        } => RecordsDamageMutation::Impulse {
            impulse,
            movement_provider: records_provider_from_game(&movement_provider),
        },
    }
}

/// Convert a records-side damage decision into its game-side twin.
fn game_decision_from_records(decision: RecordsDamageDecision) -> GameDamageDecision {
    GameDamageDecision {
        request: game_request_from_records(decision.request),
        mutations: decision.mutations.into_iter().map(game_mutation_from_records).collect(),
        applied_damage: decision.applied_damage,
        reaction: match decision.reaction {
            Reaction::None => DamageReaction::None,
            Reaction::Pain => DamageReaction::Pain,
            Reaction::Death => DamageReaction::Death,
        },
        feedback: decision.feedback.map(|feedback| match feedback {
            RecordsDamageFeedback::Q2 {
                power_armor,
                armor,
                blood,
                knockback,
            } => GameDamageFeedback::Q2 {
                power_armor,
                armor,
                blood,
                knockback,
            },
            RecordsDamageFeedback::Q3 { knockback, battlesuit } => GameDamageFeedback::Q3 { knockback, battlesuit },
        }),
    }
}

/// Convert a game-side damage decision into its records-side twin.
fn records_decision_from_game(decision: GameDamageDecision) -> RecordsDamageDecision {
    RecordsDamageDecision {
        request: records_request_from_game(decision.request),
        mutations: decision.mutations.into_iter().map(records_mutation_from_game).collect(),
        applied_damage: decision.applied_damage,
        reaction: match decision.reaction {
            DamageReaction::None => Reaction::None,
            DamageReaction::Pain => Reaction::Pain,
            DamageReaction::Death => Reaction::Death,
        },
        feedback: decision.feedback.map(|feedback| match feedback {
            GameDamageFeedback::Q2 {
                power_armor,
                armor,
                blood,
                knockback,
            } => RecordsDamageFeedback::Q2 {
                power_armor,
                armor,
                blood,
                knockback,
            },
            GameDamageFeedback::Q3 { knockback, battlesuit } => RecordsDamageFeedback::Q3 { knockback, battlesuit },
        }),
    }
}

/// Convert a records-side damage outcome into its game-side twin.
fn game_outcome_from_records(outcome: RecordsDamageOutcome) -> GameDamageOutcome {
    match outcome {
        RecordsDamageOutcome::StaleTarget { request } => GameDamageOutcome::StaleTarget {
            request: game_request_from_records(request),
        },
        RecordsDamageOutcome::Committed { decision, survived } => GameDamageOutcome::Committed {
            decision: game_decision_from_records(decision),
            survived,
        },
    }
}

/// Convert a game-side damage outcome into its records-side twin.
fn records_outcome_from_game(outcome: GameDamageOutcome) -> RecordsDamageOutcome {
    match outcome {
        GameDamageOutcome::StaleTarget { request } => RecordsDamageOutcome::StaleTarget {
            request: records_request_from_game(request),
        },
        GameDamageOutcome::Committed { decision, survived } => RecordsDamageOutcome::Committed {
            decision: records_decision_from_game(decision),
            survived,
        },
    }
}

/// Resolve the records-side entity twin of a game-pool entity.
fn records_entity_for_game(records: &Q3EntityRecords, entity: &GameEntityRef) -> RecordsEntityRef {
    records.get(entity.borrow().slot).expect("damage record")
}

/// Convert a game-side participant into its records-side twin.
fn records_participant_from_game(
    records: &Q3EntityRecords,
    participant: EntitiesDamageParticipant,
) -> RecordsDamageParticipant {
    match participant {
        EntitiesDamageParticipant::Native(entity) => {
            RecordsDamageParticipant::Native(records_entity_for_game(records, &entity))
        }
        EntitiesDamageParticipant::Shared(shared) => {
            let origin = shared.origin;
            RecordsDamageParticipant::SharedActor(SharedParticipant::new(shared.actor.clone(), Rc::new(move || origin)))
        }
    }
}

/// Convert a records-side participant into its game-side twin.
fn game_participant_from_records(
    pool: &BaseEntityPool,
    participant: RecordsDamageParticipant,
) -> EntitiesDamageParticipant {
    match participant {
        RecordsDamageParticipant::Native(native) =>
        {
            #[allow(clippy::cast_possible_wrap)]
            EntitiesDamageParticipant::Native(pool.at(native.borrow().slot as i32))
        }
        RecordsDamageParticipant::SharedActor(shared) => EntitiesDamageParticipant::Shared(SharedActor {
            actor: shared.actor.clone(),
            origin: shared.origin(),
        }),
    }
}

/// Convert a game-side damage call into its records-side twin.
fn records_call_from_game(records: &Q3EntityRecords, call: GameDamageCall) -> Q3DamageCall {
    Q3DamageCall {
        target: records_entity_for_game(records, &call.target),
        source: records_participant_from_game(records, call.source),
        owner: records_participant_from_game(records, call.owner),
        direction: call.direction,
        point: call.point,
        amount: call.amount,
        flags: call.flags,
        method_of_death: call.method_of_death,
    }
}

/// Build the game-side combat context over the bridge, host authority, and shared pools.
#[allow(clippy::too_many_arguments)]
fn build_game_combat_context(
    host: &Rc<dyn Q3SourceHost>,
    bridge: &Rc<Q3CombatBridge>,
    records: &Q3EntityRecords,
    base_pool: &Rc<RefCell<BaseEntityPool>>,
    world: &Rc<RuntimeQ3World>,
    level: &Rc<RefCell<GameLevel>>,
    settings: &Rc<Q3GameSettings>,
    item_table: &Rc<EntitiesItemTable>,
    product: Product,
) -> CombatContext {
    let bridge_context = bridge.context().clone();
    let authority_backend = host.combat();
    let apply_backend = authority_backend.clone();
    let authority = CombatAuthority {
        apply: Rc::new(move |request: GameDamageRequest| {
            game_outcome_from_records(apply_backend.apply(records_request_from_game(request)))
        }),
        read: Rc::new({
            let read_backend = authority_backend.clone();
            move |actor: &ActorId| read_backend.read(actor).map(game_state_from_records)
        }),
    };
    let source_damage_modifier =
        bridge_context
            .source_damage_modifier
            .clone()
            .map(|modifier| GameSourceDamageModifier {
                owner: game_provider_from_records(&modifier.owner),
                transform: Rc::new(move |attacker: Option<ActorId>, amount: f32| {
                    (modifier.transform)(attacker.as_ref(), amount)
                }),
            });
    let attack_bridge = bridge_context.attack.clone();
    let attack_records = records.clone();
    let attack = Rc::new(
        move |source: EntitiesDamageParticipant,
              owner: EntitiesDamageParticipant,
              weapon: Option<String>,
              method_of_death: i32,
              flags: i32,
              projectile: Option<ActorId>| {
            game_provenance_from_records((attack_bridge)(
                &records_participant_from_game(&attack_records, source),
                &records_participant_from_game(&attack_records, owner),
                weapon,
                method_of_death,
                flags,
                projectile,
            ))
        },
    );
    let dispatch_bridge = bridge_context.dispatch.clone();
    let dispatch_records = records.clone();
    let dispatch = Rc::new(move |call: GameDamageCall, apply: DamageApply<'_>| {
        game_outcome_from_records((dispatch_bridge)(
            records_call_from_game(&dispatch_records, call),
            &|| records_outcome_from_game(apply()),
        ))
    });
    let participant_bridge = bridge_context.actors.participant.clone();
    let participant_pool = base_pool.clone();
    let actors = GameCombatActors {
        is_live: bridge_context.actors.is_live.clone(),
        participant: Rc::new(move |actor: &ActorId| {
            let borrowed = participant_pool.borrow();
            let converted = game_participant_from_records(&borrowed, (participant_bridge)(actor));
            drop(borrowed);
            converted
        }),
        parent: bridge_context.actors.parent.clone(),
        linked_bounds: bridge_context.actors.linked_bounds.clone(),
        is_player: bridge_context.actors.is_player.clone(),
    };
    let area_world = world.clone();
    let trace_world = world.clone();
    let spatial = GameSpatialQueries {
        area_actors: Rc::new(move |bounds: &Bounds, maximum: i32| {
            ActorSpatialQueries::area_actors(area_world.as_ref(), *bounds, maximum.max(0) as usize)
        }),
        trace_actor: Rc::new(move |query: &ActorTraceQuery| {
            ActorSpatialQueries::trace_actor(trace_world.as_ref(), query)
        }),
    };
    let hurt_bridge = bridge_context.check_hurt_carrier.clone();
    let hurt_records = records.clone();
    let check_hurt_carrier = Rc::new(move |target: &GameEntityRef, owner: &GameEntityRef| {
        (hurt_bridge)(
            records_entity_for_game(&hurt_records, target),
            records_entity_for_game(&hurt_records, owner),
        );
    });
    let accuracy_bridge = bridge_context.log_accuracy_hit.clone();
    let accuracy_records = records.clone();
    let log_accuracy_hit = Rc::new(move |target: &GameEntityRef, owner: &GameEntityRef| {
        (accuracy_bridge)(
            records_entity_for_game(&accuracy_records, target),
            records_entity_for_game(&accuracy_records, owner),
        )
    });
    let product_word = if product == Product::Missionpack {
        let obelisk = bridge_context.check_obelisk_attack.clone().expect("obelisk attack");
        let effect = bridge_context
            .invulnerability_effect
            .clone()
            .expect("invulnerability effect");
        CombatProduct::Missionpack {
            check_obelisk_attack: Rc::new({
                let obelisk_records = records.clone();
                move |target: &GameEntityRef, owner: &EntitiesDamageParticipant| {
                    (obelisk)(
                        records_entity_for_game(&obelisk_records, target),
                        &records_participant_from_game(&obelisk_records, owner.clone()),
                    )
                }
            }),
            invulnerability_effect: Rc::new({
                let effect_records = records.clone();
                move |target: &GameEntityRef, origin: Vec3, angles: Vec3| {
                    (effect)(records_entity_for_game(&effect_records, target), origin, angles);
                }
            }),
        }
    } else {
        CombatProduct::Baseq3
    };
    let borrowed_level = level.borrow();
    let time = borrowed_level.base.time;
    let intermission_queued = borrowed_level.base.intermission_queued;
    drop(borrowed_level);
    CombatContext {
        authority,
        source_damage_modifier,
        attack,
        dispatch,
        time,
        intermission_queued,
        game_type: settings.integer("g_gametype"),
        friendly_fire: (bridge_context.friendly_fire)(),
        knockback: settings.number("g_knockback"),
        entities: base_pool.clone(),
        spatial,
        actors,
        debug_damage: bridge_context.debug_damage.clone(),
        check_hurt_carrier,
        log_accuracy_hit,
        product: product_word,
        item_table: item_table.clone(),
    }
}

/// Spawn host over the shared core.
struct RuntimeSpawnHost {
    links: Rc<RefCell<SpawnLinks>>,
    team_random: Rc<TeamGameRandom>,
}

#[derive(Default)]
struct SpawnLinks {
    team_pool: Option<PoolRef>,
    world: Option<Rc<RuntimeQ3World>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    records: Option<Q3EntityRecords>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    think: Option<Rc<ClientThinkRuntime>>,
    level: Option<Rc<RefCell<GameLevel>>>,
    settings: Option<Rc<Q3GameSettings>>,
    match_runtime: Option<Rc<MatchRuntime>>,
    death: Option<Rc<DeathRuntime>>,
    core: Option<Rc<RefCell<CoreLinks>>>,
    effects_host: Option<Rc<dyn EffectsHost>>,
    targets_host: Option<Rc<dyn TargetsHost>>,
    policy: Option<Rc<RuntimePolicyHost>>,
}

impl ClientSpawnHost for RuntimeSpawnHost {
    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    fn world(&self) -> WorldRef {
        let world: WorldRef = self.links.borrow().world.clone().expect("world");
        world
    }

    fn is_player(&self, actor: &ActorId) -> bool {
        self.links.borrow().host.clone().expect("host").is_player(actor)
    }

    fn random(&self) -> &TeamGameRandom {
        self.team_random.as_ref()
    }

    fn think_runtime(&self) -> Rc<ClientThinkRuntime> {
        self.links.borrow().think.clone().expect("think")
    }

    fn frame(&self) -> ClientSpawnFrame {
        let links = self.links.borrow();
        let level_rc = links.level.clone().expect("level");
        let level = level_rc.borrow();
        let settings = links.settings.clone().expect("settings");
        ClientSpawnFrame {
            time: level.base.time,
            game_type: settings.integer("g_gametype"),
            inactivity_seconds: settings.integer("g_inactivity"),
            intermission_time: level.base.intermission_time,
        }
    }

    fn user_command(&self, client_num: usize) -> Q3UserCommand {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .get_user_command(client_num as i32)
    }

    fn handicap(&self, client_num: usize) -> String {
        let host = self.links.borrow().host.clone().expect("host");
        client_info_value(&host.engine().get_userinfo(client_num as i32), "handicap")
    }

    fn find_intermission_point(&self) -> SpawnPose {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .find_intermission_point()
    }

    fn move_to_intermission(&self, entity: &SupportEntityRef) {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .move_client_to_intermission(entity);
    }

    fn kill_box(&self, entity: &SupportEntityRef) {
        let links = self.links.borrow();
        let core = links.core.clone().expect("core");
        let items_pool = links.items_pool.clone().expect("items pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let base_pool = links.base_pool.clone().expect("base pool");
        let slot = entity.borrow().slot;
        let mut combat = RuntimeCombatOps { links: core.clone() };
        let mut world = RuntimeWorldOps { links: core };
        base_kill_box(&mut items_pool.borrow_mut(), &mut combat, &mut world, slot).expect("kill box");
        let core = links.core.clone().expect("core");
        let product = core.borrow().product.expect("product");
        drop(links);
        reconcile_pools(&base_pool, &items_pool, &team_pool);
        let max = team_pool.max_clients();
        for client in 0..max {
            sync_items_to_entities_team(&items_pool.borrow(), &base_pool, &team_pool, product, client);
        }
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }

    fn player_die(&self) -> DieCallback {
        let links = self.links.clone();
        Rc::new(move |entity, inflictor, attacker, damage, method| {
            let borrowed = links.borrow();
            let death = borrowed.death.clone().expect("death");
            let pool = borrowed.base_pool.clone().expect("base pool");
            let team_pool = borrowed.team_pool.clone().expect("team pool");
            let slot = entity.borrow().slot;
            let borrowed_pool = pool.borrow();
            let base = borrowed_pool.at(slot as i32);
            let convert = |participant: SupportDamageParticipant| -> EntitiesDamageParticipant {
                match participant {
                    SupportDamageParticipant::Entity(entity) => {
                        EntitiesDamageParticipant::Native(borrowed_pool.at(entity.borrow().slot as i32))
                    }
                    SupportDamageParticipant::SharedActor(actor) => {
                        EntitiesDamageParticipant::Shared(SharedActor { actor, origin: None })
                    }
                }
            };
            let entities_inflictor = convert(inflictor);
            let entities_attacker = convert(attacker);
            drop(borrowed_pool);
            death.player_die(
                &base,
                Some(&entities_inflictor),
                Some(&entities_attacker),
                damage,
                method,
            );
            drop(borrowed);
            sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        })
    }

    fn body_die(&self) -> DieCallback {
        let links = self.links.clone();
        Rc::new(move |entity, _, _, _, _| {
            let borrowed = links.borrow();
            let death = borrowed.death.clone().expect("death");
            let pool = borrowed.base_pool.clone().expect("base pool");
            let team_pool = borrowed.team_pool.clone().expect("team pool");
            let slot = entity.borrow().slot;
            let base = pool.borrow().at(slot as i32);
            death.body_die(&base);
            drop(borrowed);
            sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        })
    }

    fn has_selected_player(&self) -> bool {
        true
    }

    fn selected_player(&self, entity: &SupportEntityRef, pose: &SpawnPose) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .spawn_player(entity, pose);
    }

    fn effects(&self) -> EffectsRef {
        self.links.borrow().effects_host.clone().expect("effects")
    }

    fn targets(&self) -> Rc<dyn TargetsHost> {
        self.links.borrow().targets_host.clone().expect("targets")
    }
}

/// Client effects host over shared level, settings, and config (donor `effects()`).
struct RuntimeEffectsHost {
    links: Rc<RefCell<EffectsLinks>>,
}

/// Deferred effects links filled once the runtime exists.
#[derive(Default)]
struct EffectsLinks {
    level: Option<Rc<RefCell<GameLevel>>>,
    settings: Option<Rc<Q3GameSettings>>,
    config: Option<Rc<RefCell<RuntimeConfigRegistry>>>,
    combat_handle: Option<CombatRef>,
    items: Option<Rc<dyn ItemHost>>,
    team_random: Option<Rc<TeamGameRandom>>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    team_pool: Option<PoolRef>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    policy: Option<Rc<RuntimePolicyHost>>,
    core: Option<Rc<RefCell<CoreLinks>>>,
}

impl EffectsCore for RuntimeEffectsHost {
    fn combat(&self) -> CombatRef {
        self.links.borrow().combat_handle.clone().expect("combat")
    }

    fn items(&self) -> Rc<dyn ItemHost> {
        self.links.borrow().items.clone().expect("items")
    }
}

impl EffectsHost for RuntimeEffectsHost {
    fn intermission_time(&self) -> i32 {
        self.links
            .borrow()
            .level
            .clone()
            .expect("level")
            .borrow()
            .base
            .intermission_time
    }

    fn smooth_clients(&self) -> bool {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_smoothClients")
            != 0
    }

    fn fry_sound(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().fry_sound
    }

    fn random_int(&self) -> i32 {
        self.links.borrow().team_random.clone().expect("random").rand()
    }

    fn sound_index(&self, path: &str) -> i32 {
        let links = self.links.borrow();
        let config = links.config.clone().expect("config");
        let mut cfg = config.borrow_mut();
        cfg.sound_index(Some(path)).expect("effect sound index") as i32
    }

    fn sound(&self, entity: &SupportEntityRef, _channel: i32, sound: i32) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let slot = entity.borrow().slot;
        let origin = pool.borrow().at(slot as i32).borrow().r.current_origin;
        let temp = pool
            .borrow_mut()
            .temp_entity(origin, EntityEvent::EvGeneralSound as i32);
        temp.borrow_mut().s.event_parm = sound;
        let items_pool = self.links.borrow().items_pool.clone().expect("items pool");
        let core = self.links.borrow().core.clone().expect("core");
        drop(links);
        reconcile_pools(&pool, &items_pool, &team_pool);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }

    fn spectator_end_frame(&self, entity: &SupportEntityRef) {
        let policy = self.links.borrow().policy.clone().expect("policy");
        spectator_client_end_frame(&*policy, entity);
    }
}

/// Target services over the driver (donor `useTargets`).
struct RuntimeTargetsHost {
    links: Rc<RefCell<CoreLinks>>,
}

impl TargetsHost for RuntimeTargetsHost {
    fn use_targets(&self, used: Option<&SupportEntityRef>, activator: Option<&SupportDamageParticipant>) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let driver = links.driver.clone().expect("targets driver");
        let Some(used) = used else {
            return;
        };
        let slot = used.borrow().slot;
        let participant = activator.map(|activator| match activator {
            SupportDamageParticipant::Entity(entity) => StateParticipant::Entity(entity.borrow().slot),
            SupportDamageParticipant::SharedActor(actor) => StateParticipant::SharedActor(actor.clone()),
        });
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        base_use_targets(&mut *driver.borrow_mut(), slot, participant).expect("use targets");
        reconcile_and_sync_state_to_entities(&pool, &driver, slot);
        reconcile_spawn_union(&self.links);
        drain_pending_targets(&self.links);
    }
}

/// Session services over engine print and command broadcast.
struct RuntimeSessionServices {
    links: Rc<RefCell<SessionLinks>>,
}

#[derive(Default)]
struct SessionLinks {
    host: Option<Rc<dyn Q3SourceHost>>,
    commands: Option<Rc<GameCommandRuntime>>,
}

impl SessionServices for RuntimeSessionServices {
    fn print(&self, message: &str) {
        self.links.borrow().host.clone().expect("host").engine().print(message);
    }

    fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
        self.links
            .borrow()
            .commands
            .clone()
            .expect("commands")
            .broadcast_team_change(client_num, old_team);
    }
}

/// Session cvars over the engine registry.
struct RuntimeSessionCvars {
    cvars: Rc<RefCell<CvarRegistry>>,
}

impl SessionCvarService for RuntimeSessionCvars {
    fn get(&self, name: &SessionCvarName) -> String {
        self.cvars.borrow().variable_string(&name.as_string())
    }

    fn set(&self, name: &SessionCvarName, value: &str) {
        let _ = self.cvars.borrow_mut().set(&name.as_string(), value, true);
    }
}

/// Match host over the shared core.
struct RuntimeMatchHost {
    links: Rc<RefCell<MatchLinks>>,
    team_state: MatchStateRef,
    team_random: Rc<TeamGameRandom>,
}

#[derive(Default)]
struct MatchLinks {
    product: Option<Product>,
    team_state: Option<MatchStateRef>,
    team_pool: Option<PoolRef>,
    team_scores: Option<SharedSlots>,
    level: Option<Rc<RefCell<GameLevel>>>,
    team_random: Option<TeamGameRandom>,
    spawns: Option<Rc<ClientSpawnRuntime>>,
    settings: Option<Rc<Q3GameSettings>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    commands: Option<Rc<GameCommandRuntime>>,
    admission: Option<Rc<ClientAdmissionRuntime>>,
    session: Option<Rc<GameSessionManager>>,
    session_world: Option<Rc<SessionWorld>>,
    arenas: Option<Rc<ArenaRuntime>>,
}

impl MatchHost for RuntimeMatchHost {
    fn product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    fn state(&self) -> &MatchStateRef {
        &self.team_state
    }

    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    fn team_scores(&self) -> SharedSlots {
        self.links.borrow().team_scores.clone().expect("team scores")
    }

    fn random(&self) -> &TeamGameRandom {
        self.team_random.as_ref()
    }

    fn spawn(&self) -> Rc<dyn SpawnShort> {
        Rc::new(RuntimeSpawnShort {
            links: self.links.clone(),
        })
    }

    fn settings(&self) -> MatchSettings {
        let links = self.links.borrow();
        let settings = links.settings.clone().expect("settings");
        MatchSettings {
            game_type: settings.integer("g_gametype"),
            time_limit: settings.integer("timelimit"),
            frag_limit: settings.integer("fraglimit"),
            capture_limit: settings.integer("capturelimit"),
            warmup_seconds: settings.integer("g_warmup"),
            warmup_modification_count: settings.snapshot("g_warmup").modification_count as i32,
            password: settings.string("g_password"),
            password_modification_count: settings.snapshot("g_password").modification_count as i32,
        }
    }

    fn set_team(&self, entity: &SupportEntityRef, team: &str) {
        self.links
            .borrow()
            .commands
            .clone()
            .expect("commands")
            .set_team(entity, team);
    }

    fn stop_following(&self, entity: &SupportEntityRef) {
        self.links
            .borrow()
            .commands
            .clone()
            .expect("commands")
            .stop_following(entity);
    }

    fn send_scoreboard(&self, entity: &SupportEntityRef) {
        self.links
            .borrow()
            .commands
            .clone()
            .expect("commands")
            .scoreboard(entity);
    }

    fn client_userinfo_changed(&self, client_num: usize) {
        self.links
            .borrow()
            .admission
            .clone()
            .expect("admission")
            .userinfo_changed(client_num as i32);
    }

    fn write_session_data(&self) {
        sync_session_world(&self.links);
        self.links.borrow().session.clone().expect("session").write_world();
    }

    fn append_console_command(&self, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .append_console_command(text);
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .send_server_command(client_num, text);
    }

    fn set_configstring(&self, index: i32, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .configstrings()
            .borrow_mut()
            .set(index as usize, text);
    }

    fn set_cvar(&self, name: &str, value: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .cvars()
            .borrow_mut()
            .set(name, value, true)
            .expect("set cvar");
    }

    fn log(&self, text: &str) {
        self.links.borrow().host.clone().expect("host").engine().log(text);
    }

    fn warn(&self, text: &str) {
        self.links.borrow().host.clone().expect("host").engine().print(text);
    }

    fn bot_interbreed_end_match(&self) {
        if let Q3SourceBots::Available {
            interbreed_end_match, ..
        } = self.links.borrow().host.clone().expect("host").bots()
        {
            interbreed_end_match();
        }
    }

    fn update_tournament_info(&self) {
        self.links
            .borrow()
            .arenas
            .clone()
            .expect("arenas")
            .update_tournament_info();
    }

    fn spawn_models_on_victory_pads(&self) {
        self.links
            .borrow()
            .arenas
            .clone()
            .expect("arenas")
            .spawn_models_on_victory_pads();
    }

    fn single_player(&self) -> bool {
        let links = self.links.borrow();
        links.product.expect("product") == Product::Missionpack
            && links
                .settings
                .clone()
                .expect("settings")
                .integer("ui_singlePlayerActive")
                != 0
    }
}

/// Spawn-short bridge to the client spawn runtime.
struct RuntimeSpawnShort {
    links: Rc<RefCell<MatchLinks>>,
}

impl SpawnShort for RuntimeSpawnShort {
    fn select_spawn_point(&self, avoid: Vec3) -> SpawnPoint {
        self.links
            .borrow()
            .spawns
            .clone()
            .expect("spawns")
            .select_spawn_point(avoid)
    }

    fn respawn(&self, entity: &SupportEntityRef) {
        self.links.borrow().spawns.clone().expect("spawns").respawn(entity);
    }
}

/// Sync live level state into the session world mirror.
fn sync_session_world(links: &Rc<RefCell<MatchLinks>>) {
    let borrowed = links.borrow();
    let _ = borrowed;
}

/// Arena host over match, world, cvars, and configstrings.
struct RuntimeArenaHost {
    match_runtime: Rc<RefCell<Option<Rc<MatchRuntime>>>>,
    world: Rc<RuntimeQ3World>,
    cvars: Rc<RuntimeArenaCvars>,
    config: Rc<RuntimeArenaConfig>,
}

impl ArenaHost for RuntimeArenaHost {
    fn match_runtime(&self) -> Rc<MatchRuntime> {
        self.match_runtime.borrow().clone().expect("match")
    }

    fn world(&self) -> WorldRef {
        self.world.clone()
    }

    fn cvars(&self) -> Rc<dyn SupportCvarRegistry> {
        self.cvars.clone()
    }

    fn config(&self) -> Rc<dyn ConfigStrings> {
        self.config.clone()
    }
}

/// Team host over the shared core.
struct RuntimeTeamHost {
    links: Rc<RefCell<TeamLinks>>,
}

#[derive(Default)]
struct TeamLinks {
    product: Option<Product>,
    team_pool: Option<PoolRef>,
    world: Option<Rc<RuntimeQ3World>>,
    level: Option<Rc<RefCell<GameLevel>>>,
    team_scores: Option<SharedSlots>,
    settings: Option<Rc<Q3GameSettings>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    locations: Option<Rc<RefCell<TargetLocationState>>>,
    commands: Option<Rc<GameCommandRuntime>>,
    match_runtime: Option<Rc<MatchRuntime>>,
    death: Option<Rc<DeathRuntime>>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    lifecycle: Option<Rc<RefCell<ItemLifecycleContext>>>,
}

impl TeamHost for RuntimeTeamHost {
    fn product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    fn world(&self) -> WorldRef {
        self.links.borrow().world.clone().expect("world")
    }

    fn game_type(&self) -> i32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_gametype")
    }

    fn time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.time
    }

    fn team_scores(&self) -> SharedSlots {
        self.links.borrow().team_scores.clone().expect("team scores")
    }

    fn sorted_clients(&self) -> Vec<i32> {
        self.links
            .borrow()
            .level
            .clone()
            .expect("level")
            .borrow()
            .base
            .sorted_clients
            .clone()
    }

    fn location_head(&self) -> Option<SupportEntityRef> {
        let links = self.links.borrow();
        let head = links.locations.clone().expect("locations").borrow().head;
        let pool = links.team_pool.clone().expect("team pool");
        head.map(|slot| pool.at(slot))
    }

    fn obelisk_settings(&self) -> Option<ObeliskSettings> {
        let links = self.links.borrow();
        if links.product.expect("product") != Product::Missionpack {
            return None;
        }
        let settings = links.settings.clone().expect("settings");
        Some(ObeliskSettings {
            health: settings.integer("g_obeliskHealth"),
            regen_period_seconds: settings.integer("g_obeliskRegenPeriod"),
            regen_amount: settings.integer("g_obeliskRegenAmount"),
            respawn_delay_seconds: settings.integer("g_obeliskRespawnDelay"),
        })
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .send_server_command(client_num, text);
    }

    fn set_configstring(&self, index: i32, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .configstrings()
            .borrow_mut()
            .set(index as usize, text);
    }

    fn warn(&self, text: &str) {
        self.links.borrow().host.clone().expect("host").engine().print(text);
    }

    fn add_score(&self, player: &SupportEntityRef, origin: Vec3, score: i32) {
        let links = self.links.borrow();
        let death = links.death.clone().expect("death");
        let pool = links.base_pool.clone().expect("base pool");
        let scores = links.team_scores.clone().expect("team scores");
        death.add_score(&pool.borrow().at(player.borrow().slot as i32), origin, score);
        sync_death_scores_to_shared(&death, &scores);
    }

    fn calculate_ranks(&self) {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .calculate_ranks();
    }

    fn respawn_item(&self, item: &SupportEntityRef) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let lifecycle = links.lifecycle.clone().expect("lifecycle");
        let team_pool = links.team_pool.clone().expect("team pool");
        let slot = item.borrow().slot;
        lifecycle_respawn_item(&pool.borrow().at(slot as i32), &lifecycle.borrow());
        sync_entities_to_team(&pool.borrow(), &team_pool, slot);
    }

    fn in_pvs(&self, first: Vec3, second: Vec3) -> bool {
        let scene = self.links.borrow().host.clone().expect("host").scene();
        let a = scene.point_leaf(first);
        let b = scene.point_leaf(second);
        scene.cluster_visible(scene.leaf_cluster(a), scene.leaf_cluster(b))
            && scene.areas_connected(scene.leaf_area(a), scene.leaf_area(b))
    }
}

/// Support death host bridging to the base death runtime.
struct RuntimeSupportDeathHost {
    links: Rc<RefCell<DeathLinks>>,
}

#[derive(Default)]
struct DeathLinks {
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    death: Option<Rc<DeathRuntime>>,
    team_pool: Option<PoolRef>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    core: Option<Rc<RefCell<CoreLinks>>>,
}

impl SupportDeathHost for RuntimeSupportDeathHost {
    fn player_die(
        &self,
        target: &SupportEntityRef,
        inflictor: Option<&SupportDamageParticipant>,
        attacker: Option<&SupportDamageParticipant>,
        damage: i32,
        method: i32,
    ) {
        let links = self.links.borrow();
        let death = links.death.clone().expect("death");
        let pool = links.base_pool.clone().expect("base pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let items_pool = links.items_pool.clone().expect("items pool");
        let slot = target.borrow().slot;
        let borrowed = pool.borrow();
        let base = borrowed.at(slot as i32);
        let convert = |participant: Option<&SupportDamageParticipant>| -> EntitiesDamageParticipant {
            match participant {
                Some(SupportDamageParticipant::Entity(entity)) => {
                    EntitiesDamageParticipant::Native(borrowed.at(entity.borrow().slot as i32))
                }
                Some(SupportDamageParticipant::SharedActor(actor)) => EntitiesDamageParticipant::Shared(SharedActor {
                    actor: actor.clone(),
                    origin: None,
                }),
                None => EntitiesDamageParticipant::Native(borrowed.at(1022)),
            }
        };
        let entities_inflictor = convert(inflictor);
        let entities_attacker = convert(attacker);
        drop(borrowed);
        death.player_die(
            &base,
            Some(&entities_inflictor),
            Some(&entities_attacker),
            damage,
            method,
        );
        let core = links.core.clone().expect("core");
        drop(links);
        reconcile_pools(&pool, &items_pool, &team_pool);
        sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }

    fn toss_client_items(&self, entity: &SupportEntityRef) {
        let links = self.links.borrow();
        let death = links.death.clone().expect("death");
        let pool = links.base_pool.clone().expect("base pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let items_pool = links.items_pool.clone().expect("items pool");
        let slot = entity.borrow().slot;
        death.toss_client_items(&pool.borrow().at(slot as i32));
        let core = links.core.clone().expect("core");
        drop(links);
        reconcile_pools(&pool, &items_pool, &team_pool);
        sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }

    fn toss_client_persistant_powerups(&self, entity: &SupportEntityRef) {
        let links = self.links.borrow();
        let death = links.death.clone().expect("death");
        let pool = links.base_pool.clone().expect("base pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let items_pool = links.items_pool.clone().expect("items pool");
        let slot = entity.borrow().slot;
        death.toss_client_persistant_powerups(&pool.borrow().at(slot as i32));
        let core = links.core.clone().expect("core");
        drop(links);
        reconcile_pools(&pool, &items_pool, &team_pool);
        sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }

    fn toss_client_cubes(&self, entity: &SupportEntityRef) {
        let links = self.links.borrow();
        let death = links.death.clone().expect("death");
        let pool = links.base_pool.clone().expect("base pool");
        let team_pool = links.team_pool.clone().expect("team pool");
        let items_pool = links.items_pool.clone().expect("items pool");
        let slot = entity.borrow().slot;
        death.toss_client_cubes(&pool.borrow().at(slot as i32));
        let core = links.core.clone().expect("core");
        drop(links);
        reconcile_pools(&pool, &items_pool, &team_pool);
        sync_entities_to_team(&pool.borrow(), &team_pool, slot);
        reconcile_spawn_union(&core);
        drain_pending_targets(&core);
    }
}

/// Command engine imports.
struct RuntimeCommandImports {
    host: Rc<dyn Q3SourceHost>,
}

impl CommandImports for RuntimeCommandImports {
    fn send_server_command(&self, client_num: i32, text: &str) {
        self.host.engine().send_server_command(client_num, text);
    }

    fn set_configstring(&self, index: i32, text: &str) {
        self.host.configstrings().borrow_mut().set(index as usize, text);
    }

    fn append_console_command(&self, text: &str) {
        self.host.engine().append_console_command(text);
    }

    fn get_cvar(&self, name: &str) -> String {
        self.host.cvars().borrow().variable_string(name)
    }

    fn get_userinfo(&self, client_num: usize) -> String {
        self.host.engine().get_userinfo(client_num as i32)
    }

    fn set_userinfo(&self, client_num: usize, text: &str) {
        self.host.engine().set_userinfo(client_num as i32, text);
    }

    fn log(&self, text: &str) {
        self.host.engine().log(text);
    }

    fn print(&self, text: &str) {
        self.host.engine().print(text);
    }
}

/// Game command host over the shared core.
struct RuntimeGameCommandHost {
    links: Rc<RefCell<CommandLinks>>,
    team_state: MatchStateRef,
}

#[derive(Default)]
struct CommandLinks {
    team_pool: Option<PoolRef>,
    team_state: Option<MatchStateRef>,
    team_scores: Option<SharedSlots>,
    settings: Option<Rc<Q3GameSettings>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    imports: Option<Rc<RuntimeCommandImports>>,
    team: Option<Rc<TeamRuntime>>,
    death_host: Option<Rc<RuntimeSupportDeathHost>>,
    spawns: Option<Rc<ClientSpawnRuntime>>,
    admission: Option<Rc<ClientAdmissionRuntime>>,
    match_runtime: Option<Rc<MatchRuntime>>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    items_host: Option<Rc<dyn ItemHost>>,
    teleport_host: Option<Rc<dyn TeleportHost>>,
}

impl GameCommandHost for RuntimeGameCommandHost {
    fn grant_selected_arsenal(
        &self,
        actor: &ActorId,
        category: qa_content::q3::team_arena::commands::ArsenalCategory,
    ) -> bool {
        let links = self.links.borrow();
        let host = links.host.clone().expect("host");
        let mapped = match category {
            qa_content::q3::team_arena::commands::ArsenalCategory::Weapons => Q3ArsenalCategory::Weapons,
            qa_content::q3::team_arena::commands::ArsenalCategory::Ammo => Q3ArsenalCategory::Ammo,
        };
        host.grant_selected_arsenal(actor, mapped)
    }

    fn give_selected_item(&self, actor: &ActorId, args: &[String]) -> bool {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .give_selected_item(actor, args)
    }

    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    fn match_state(&self) -> &MatchStateRef {
        &self.team_state
    }

    fn team_scores(&self) -> SharedSlots {
        self.links.borrow().team_scores.clone().expect("team scores")
    }

    fn settings(&self) -> CommandSettings {
        let links = self.links.borrow();
        let settings = links.settings.clone().expect("settings");
        CommandSettings {
            game_type: settings.integer("g_gametype"),
            cheats: settings.integer("sv_cheats") != 0,
            team_force_balance: settings.integer("g_teamForceBalance") != 0,
            max_game_clients: settings.integer("g_maxGameClients"),
            dedicated: settings.integer("dedicated") != 0,
            allow_vote: settings.integer("g_allowVote") != 0,
        }
    }

    fn imports(&self) -> Rc<dyn CommandImports> {
        self.links.borrow().imports.clone().expect("imports")
    }

    fn team_location_message(&self, entity: &SupportEntityRef, capacity: usize) -> Option<String> {
        self.links
            .borrow()
            .team
            .clone()
            .expect("team")
            .get_location_message(entity, capacity)
    }

    fn death(&self) -> Rc<dyn SupportDeathHost> {
        self.links.borrow().death_host.clone().expect("death host")
    }

    fn copy_to_body_queue(&self, entity: &SupportEntityRef) -> Option<SupportEntityRef> {
        self.links
            .borrow()
            .spawns
            .clone()
            .expect("spawns")
            .copy_to_body_queue(entity)
    }

    fn admission_begin(&self, client_num: usize) {
        self.links
            .borrow()
            .admission
            .clone()
            .expect("admission")
            .begin(client_num as i32);
    }

    fn admission_userinfo_changed(&self, client_num: usize) {
        self.links
            .borrow()
            .admission
            .clone()
            .expect("admission")
            .userinfo_changed(client_num as i32);
    }

    fn match_begin_intermission(&self) {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .begin_intermission();
    }

    fn match_set_leader(&self, team_code: i32, client_num: usize) {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .set_leader(team_code, client_num);
    }

    fn match_check_team_leader(&self, team_code: i32) {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .check_team_leader(team_code);
    }

    fn items(&self) -> Rc<dyn ItemHost> {
        self.links.borrow().items_host.clone().expect("items")
    }

    fn teleport(&self) -> Rc<dyn TeleportHost> {
        self.links.borrow().teleport_host.clone().expect("teleport")
    }
}

/// Admission console commands bridge.
struct RuntimeAdmissionCommands {
    commands: Rc<RefCell<Option<Rc<GameCommandRuntime>>>>,
}

impl qa_content::q3::team_arena::client_admission::ClientAdmissionCommands for RuntimeAdmissionCommands {
    fn broadcast_team_change(&self, client_num: usize, old_team: i32) {
        self.commands
            .borrow()
            .clone()
            .expect("commands")
            .broadcast_team_change(client_num, old_team);
    }

    fn stop_following(&self, entity: &SupportEntityRef) {
        self.commands.borrow().clone().expect("commands").stop_following(entity);
    }
}

/// Admission host over the shared core.
struct RuntimeAdmissionHost {
    links: Rc<RefCell<AdmissionLinks>>,
}

#[derive(Default)]
struct AdmissionLinks {
    product: Option<Product>,
    team_pool: Option<PoolRef>,
    team_scores: Option<SharedSlots>,
    world: Option<Rc<RuntimeQ3World>>,
    team_state: Option<MatchStateRef>,
    level: Option<Rc<RefCell<GameLevel>>>,
    session: Option<Rc<GameSessionManager>>,
    spawns: Option<Rc<ClientSpawnRuntime>>,
    death_host: Option<Rc<RuntimeSupportDeathHost>>,
    match_runtime: Option<Rc<MatchRuntime>>,
    commands: Option<Rc<RuntimeAdmissionCommands>>,
    bots: Option<Q3SourceBots<'static>>,
    settings: Option<Rc<Q3GameSettings>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    server_commands: Option<Rc<GameServerCommandRuntime>>,
}

impl ClientAdmissionHost for RuntimeAdmissionHost {
    fn product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    fn pool(&self) -> PoolRef {
        self.links.borrow().team_pool.clone().expect("team pool")
    }

    fn team_scores(&self) -> SharedSlots {
        self.links.borrow().team_scores.clone().expect("team scores")
    }

    fn world(&self) -> WorldRef {
        self.links.borrow().world.clone().expect("world")
    }

    fn match_state(&self) -> MatchStateRef {
        self.links.borrow().team_state.clone().expect("team state")
    }

    fn new_session(&self) -> bool {
        self.links.borrow().level.clone().expect("level").borrow().new_session
    }

    fn session(&self) -> Rc<GameSessionManager> {
        self.links.borrow().session.clone().expect("session")
    }

    fn spawn_client(&self, entity: &SupportEntityRef) {
        self.links.borrow().spawns.clone().expect("spawns").client_spawn(entity);
    }

    fn death(&self) -> Rc<dyn SupportDeathHost> {
        self.links.borrow().death_host.clone().expect("death host")
    }

    fn calculate_ranks(&self) {
        self.links
            .borrow()
            .match_runtime
            .clone()
            .expect("match")
            .calculate_ranks();
    }

    fn commands(&self) -> Rc<dyn qa_content::q3::team_arena::client_admission::ClientAdmissionCommands> {
        self.links.borrow().commands.clone().expect("commands")
    }

    fn bots(&self) -> ClientBotServices<'_> {
        self.links.borrow().bots.clone().expect("bots").client_services()
    }

    fn settings(&self) -> ClientAdmissionSettings {
        let links = self.links.borrow();
        let settings = links.settings.clone().expect("settings");
        ClientAdmissionSettings {
            game_type: settings.integer("g_gametype"),
            password: settings.string("g_password"),
        }
    }

    fn get_userinfo(&self, client_num: usize) -> String {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .get_userinfo(client_num as i32)
    }

    fn set_configstring(&self, index: i32, value: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .configstrings()
            .borrow_mut()
            .set(index as usize, value);
    }

    fn send_server_command(&self, client_num: i32, value: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .send_server_command(client_num, value);
    }

    fn log(&self, value: &str) {
        self.links.borrow().host.clone().expect("host").engine().log(value);
    }

    fn filter_packet(&self, address: &str) -> bool {
        self.links
            .borrow()
            .server_commands
            .clone()
            .expect("server commands")
            .filter_packet(address)
    }
}

/// Server command host over engine services and runtime modules.
struct RuntimeServerCommandHost {
    links: Rc<RefCell<ServerCommandLinks>>,
}

#[derive(Default)]
struct ServerCommandLinks {
    settings: Option<Rc<Q3GameSettings>>,
    host: Option<Rc<dyn Q3SourceHost>>,
    commands: Option<Rc<GameCommandRuntime>>,
    bots: Option<Q3SourceBots<'static>>,
    memory: Option<Rc<RefCell<GameMemory>>>,
    arenas: Option<Rc<ArenaRuntime>>,
}

impl GameServerCommandHost for RuntimeServerCommandHost {
    fn read_vm_cvar(&self, name: ServerCommandCvar) -> qa_content::q3::base::game::save_module_values::Q3CvarSnapshot {
        let links = self.links.borrow();
        let settings = links.settings.clone().expect("settings");
        let snapshot = settings.snapshot(name.as_str());
        Q3CvarSnapshot {
            name: snapshot.name,
            value: snapshot.value,
            reset_value: snapshot.reset_value,
            latched_value: snapshot.latched_value,
            flags: snapshot.flags as i32,
            modified: snapshot.modified,
            modification_count: snapshot.modification_count as i32,
            numeric_value: f64::from(snapshot.numeric_value),
            integer_value: snapshot.integer_value,
        }
    }

    fn print(&self, text: &str) {
        self.links.borrow().host.clone().expect("host").engine().print(text);
    }

    fn send_server_command(&self, client_num: i32, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .send_server_command(client_num, text);
    }

    fn execute_console_now(&self, text: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .execute_console_now(text);
    }

    fn set_team(&self, entity: &SupportEntityRef, team: &str) {
        self.links
            .borrow()
            .commands
            .clone()
            .expect("commands")
            .set_team(entity, team);
    }

    fn bots(&self) -> ServerCommandCapability {
        match self.links.borrow().bots.clone().expect("bots") {
            Q3SourceBots::Unavailable { reason } => ServerCommandCapability::Unavailable { reason },
            Q3SourceBots::Available { console_command, .. } => ServerCommandCapability::Available {
                run: Rc::new(move |argv: &[String]| console_command(argv)),
            },
        }
    }

    fn memory(&self) -> ServerCommandCapability {
        let memory = self.links.borrow().memory.clone().expect("memory");
        ServerCommandCapability::Available {
            run: Rc::new(move |_argv: &[String]| {
                memory.borrow_mut().status();
            }),
        }
    }

    fn podium(&self) -> ServerCommandCapability {
        let arenas = self.links.borrow().arenas.clone().expect("arenas");
        ServerCommandCapability::Available {
            run: Rc::new(move |_argv: &[String]| {
                arenas.abort_podium();
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Driver state pool and cross-pool synchronization.
// ---------------------------------------------------------------------------

/// State-pool entity record.
use qa_content::q3::base::game::mover::{MoverActorAccess, SharedBodyKind, SharedMoverBody};
use qa_content::q3::base::game::state::{
    ActorCombatState as StateActorCombatState, CombatContext as StateCombatContext, EntityPool as StateEntityPool,
    EntityTouch as StateEntityTouch, GameEntity as StateGameEntity, MoverState as StateMoverState, Q3BodyState,
    Q3Driver, Q3GameError, Q3PlayerSlots, ServerWorldOps as StateServerWorldOps, SpatialQueries as StateSpatialQueries,
};
use qa_content::q3::base::game::utilities::GameUtilityScratch;
/// State-pool client record.
type StateGameClient = qa_content::q3::base::game::state::GameClient;
/// Entities slot container.
type EntitiesPlayerSlots = qa_content::q3::base::game::entities::PlayerStateSlots;
/// State slot container.
type StatePlayerSlots = qa_content::q3::base::game::state::Q3PlayerSlots;
/// Team slot container.
type TeamSlotArray = qa_content::q3::team_arena::support::SlotArray;
/// Team-pool client record.
type TeamGameClient = qa_content::q3::team_arena::support::GameClient;
/// Entities-pool client record.
type EntitiesGameClient = qa_content::q3::base::game::entities::GameClient;
/// Items-pool network state record.
type ItemsEntityState = qa_content::q3::base::game::items_core::EntityState;

/// Decode a stored movement type, rejecting unknown values.
fn stat_max_health(schema: StatSchema) -> i32 {
    match schema {
        StatSchema::Base(raw) => raw.max_health,
        StatSchema::Missionpack(raw) => raw.max_health,
    }
}

fn stat_holdable_item(schema: StatSchema) -> i32 {
    match schema {
        StatSchema::Base(raw) => raw.holdable_item,
        StatSchema::Missionpack(raw) => raw.holdable_item,
    }
}

struct RecordsPoolView {
    records: Q3EntityRecords,
}

impl RecordsEntityPool for RecordsPoolView {
    fn num_entities(&self) -> usize {
        self.records.capture_ownership().len()
    }

    fn entity_at(&self, index: usize) -> RecordsEntityRef {
        self.records
            .get(index)
            .unwrap_or_else(|| panic!("Q3 entity {index} outside 0..1024"))
    }
}

fn move_type_from_i32(value: i32) -> MoveType {
    match value {
        0 => MoveType::PmNormal,
        1 => MoveType::PmNoclip,
        2 => MoveType::PmSpectator,
        3 => MoveType::PmDead,
        4 => MoveType::PmFreeze,
        5 => MoveType::PmIntermission,
        6 => MoveType::PmSpintermission,
        _ => panic!("Invalid movement type {value}"),
    }
}

/// Decode a stored weapon state, rejecting unknown values.
fn weapon_state_from_i32(value: i32) -> WeaponState {
    match value {
        0 => WeaponState::WeaponReady,
        1 => WeaponState::WeaponRaising,
        2 => WeaponState::WeaponDropping,
        3 => WeaponState::WeaponFiring,
        _ => panic!("Invalid weapon state {value}"),
    }
}

/// Decode a stored spectator state, rejecting unknown values.
fn spectator_state_from_i32(value: i32) -> SpectatorState {
    match value {
        0 => SpectatorState::Not,
        1 => SpectatorState::Free,
        2 => SpectatorState::Follow,
        3 => SpectatorState::Scoreboard,
        _ => panic!("Invalid spectator state {value}"),
    }
}

/// Copy the shared network fields from an items-pool state record.
fn copy_items_state_to_shared(source: &ItemsEntityState, target: &mut EntityState) {
    target.e_type = source.e_type;
    target.number = source.number;
    target.client_num = source.client_num;
    target.origin = source.origin;
    target.origin2 = source.origin2;
    target.angles = source.angles;
    target.angles2 = source.angles2;
    target.pos = source.pos;
    target.apos = source.apos;
    target.weapon = source.weapon as i32;
    target.modelindex = source.modelindex;
    target.modelindex2 = source.modelindex2;
    target.e_flags = source.e_flags;
    target.event = source.event;
    target.event_parm = source.event_parm;
    target.other_entity_num = source.other_entity_num;
    target.ground_entity_num = source.ground_entity_num;
    target.loop_sound = source.loop_sound;
    target.generic1 = source.generic1;
    target.powerups = source.powerups;
    target.frame = source.frame;
    target.legs_anim = source.legs_anim;
    target.torso_anim = source.torso_anim;
}

/// Copy the shared network fields into an items-pool state record.
fn copy_shared_state_to_items(source: &EntityState, target: &mut ItemsEntityState) {
    target.e_type = source.e_type;
    target.number = source.number;
    target.client_num = source.client_num;
    target.origin = source.origin;
    target.origin2 = source.origin2;
    target.angles = source.angles;
    target.angles2 = source.angles2;
    target.pos = source.pos;
    target.apos = source.apos;
    target.weapon = Weapon::from_i32(source.weapon).expect("items weapon");
    target.modelindex = source.modelindex;
    target.modelindex2 = source.modelindex2;
    target.e_flags = source.e_flags;
    target.event = source.event;
    target.event_parm = source.event_parm;
    target.other_entity_num = source.other_entity_num;
    target.ground_entity_num = source.ground_entity_num;
    target.loop_sound = source.loop_sound;
    target.generic1 = source.generic1;
    target.powerups = source.powerups;
    target.frame = source.frame;
    target.legs_anim = source.legs_anim;
    target.torso_anim = source.torso_anim;
}

/// Resolve a state-pool classname through the owning client netname.
fn resolve_state_classname(state_pool: &RuntimeStatePool, state: &StateGameEntity) -> Option<String> {
    let netname = state
        .client
        .and_then(|index| state_pool.clients.get(index))
        .map(|client| client.pers.netname.clone());
    state.resolve_classname(netname.as_deref())
}

/// Driver-side entity pool: fixed slots mirroring the entities pool by slot.
struct RuntimeStatePool {
    product: Product,
    max_clients: usize,
    num_entities: usize,
    entities: Vec<StateGameEntity>,
    clients: Vec<StateGameClient>,
    callbacks: qa_content::q3::base::game::save_callbacks::Q3CallbackCatalog,
    rankings: qa_content::q3::base::game::rankings::Q3RankingReports,
}

impl RuntimeStatePool {
    fn new(product: Product, max_clients: usize) -> Self {
        let clients = (0..MAX_CLIENTS).map(|_| StateGameClient::new(product)).collect();
        Self {
            product,
            max_clients,
            num_entities: MAX_CLIENTS,
            entities: Vec::new(),
            clients,
            callbacks: qa_content::q3::base::game::save_callbacks::Q3CallbackCatalog::new(),
            rankings: qa_content::q3::base::game::rankings::Q3RankingReports::new(),
        }
    }

    fn raise_mark(&mut self, slot: usize) {
        self.num_entities = self.num_entities.max(slot + 1).min(MAX_GENTITIES);
    }
}

impl StateEntityPool for RuntimeStatePool {
    fn product(&self) -> Product {
        self.product
    }

    fn num_entities(&self) -> usize {
        self.num_entities
    }

    fn max_clients(&self) -> usize {
        self.max_clients
    }

    fn entity(&self, slot: usize) -> Option<&StateGameEntity> {
        self.entities.get(slot)
    }

    fn entity_mut(&mut self, slot: usize) -> Option<&mut StateGameEntity> {
        self.entities.get_mut(slot)
    }

    fn client(&self, slot: usize) -> Option<&StateGameClient> {
        self.clients.get(slot)
    }

    fn client_mut(&mut self, slot: usize) -> Option<&mut StateGameClient> {
        self.clients.get_mut(slot)
    }

    fn spawn_entity(&mut self) -> Result<usize, Q3GameError> {
        for slot in MAX_CLIENTS..self.num_entities {
            if !self.entities[slot].inuse {
                self.entities[slot].inuse = true;
                return Ok(slot);
            }
        }
        if self.num_entities >= ENTITYNUM_WORLD as usize {
            return Err(Q3GameError::Failure("G_Spawn: no free entities".to_string()));
        }
        let slot = self.num_entities;
        self.entities[slot].inuse = true;
        self.num_entities = slot + 1;
        Ok(slot)
    }

    fn free_entity(&mut self, slot: usize) {
        if let Some(entity) = self.entities.get_mut(slot) {
            entity.inuse = false;
            entity.r.linked = false;
        }
    }

    fn temp_entity(&mut self, origin: Vec3, event: EntityEvent) -> usize {
        let slot = self.spawn_entity().expect("driver temp entity");
        if let Some(entity) = self.entities.get_mut(slot) {
            entity.s.e_type = EntityType::EtEvents as i32 + event as i32;
            entity.set_classname(Some("tempEntity".to_string()));
            entity.free_after_event = true;
            entity.s.pos.trajectory_type = TrajectoryType::TrStationary;
            entity.s.pos.base = origin;
        }
        slot
    }

    fn add_event(&mut self, slot: usize, event: EntityEvent, parm: i32) {
        if let Some(entity) = self.entities.get_mut(slot) {
            entity.s.event = event as i32;
            entity.s.event_parm = parm;
        }
    }

    fn set_nextthink(&mut self, slot: usize, time: i32) {
        if let Some(entity) = self.entities.get_mut(slot) {
            entity.nextthink = time;
        }
    }

    fn callbacks(&self) -> &qa_content::q3::base::game::save_callbacks::Q3CallbackCatalog {
        &self.callbacks
    }

    fn callbacks_mut(&mut self) -> &mut qa_content::q3::base::game::save_callbacks::Q3CallbackCatalog {
        &mut self.callbacks
    }

    fn rankings(&self) -> &qa_content::q3::base::game::rankings::Q3RankingReports {
        &self.rankings
    }

    fn restore_counts(&mut self, num_entities: usize, max_clients: usize) {
        self.num_entities = num_entities.min(MAX_GENTITIES);
        self.max_clients = max_clients.min(MAX_CLIENTS);
    }
}

/// Copy one slot from the entities pool into the driver state pool.
fn sync_entities_to_state(pool: &BaseEntityPool, state_pool: &mut RuntimeStatePool, slot: usize) {
    let borrowed = pool.at(slot as i32);
    let borrowed = borrowed.borrow();
    state_pool.raise_mark(slot);
    let Some(state) = state_pool.entities.get_mut(slot) else {
        return;
    };
    state.inuse = borrowed.inuse;
    state.s = borrowed.s.clone();
    state.r.sv_flags = borrowed.r.sv_flags;
    state.r.single_client = borrowed.r.single_client;
    state.r.contents = borrowed.r.contents;
    state.r.owner_num = borrowed.r.owner_num;
    state.r.mins = borrowed.r.mins;
    state.r.maxs = borrowed.r.maxs;
    state.r.current_origin = borrowed.r.current_origin;
    state.r.current_angles = borrowed.r.current_angles;
    state.r.linked = borrowed.r.linked;
    state.r.ground = borrowed.r.ground.clone();
    state.set_classname(borrowed.classname.clone());
    state.spawnflags = borrowed.spawnflags;
    state.never_free = borrowed.never_free;
    state.flags = borrowed.flags;
    state.freetime = borrowed.freetime;
    state.event_time = borrowed.event_time;
    state.free_after_event = borrowed.free_after_event;
    state.unlink_after_event = borrowed.unlink_after_event;
    state.physics_bounce = borrowed.physics_bounce as i32;
    state.clipmask = borrowed.clipmask;
    state.mover_state = borrowed.mover_state as i32;
    state.nextthink = borrowed.nextthink;
    state.health = borrowed.health;
    state.takedamage = borrowed.takedamage;
    state.count = borrowed.count;
    state.teamchain = borrowed.teamchain.as_ref().map(|entity| entity.borrow().slot);
    state.teammaster = borrowed.teammaster.as_ref().map(|entity| entity.borrow().slot);
    state.team = borrowed.team.clone();
    state.targetname.clone_from(&borrowed.targetname);
    state.target.clone_from(&borrowed.target);
    state.wait = borrowed.wait;
    state.random = borrowed.random;
    if let (Some(client), Some(index)) = (borrowed.client.as_ref(), state.client) {
        if let Some(state_client) = state_pool.clients.get_mut(index) {
            sync_entities_client_to_state(client, state_client);
        }
    }
}

/// Copy shared client fields from an entities client into a state client.
fn sync_entities_client_to_state(client: &EntitiesGameClient, state: &mut StateGameClient) {
    state.ps.pm_type = client.ps.pm_type as i32;
    state.ps.pm_flags = client.ps.pm_flags;
    state.ps.pm_time = client.ps.pm_time;
    state.ps.origin = client.ps.origin;
    state.ps.legs_anim = client.ps.legs_anim;
    state.ps.torso_anim = client.ps.torso_anim;
    state.ps.e_flags = client.ps.e_flags;
    state.ps.event_sequence = client.ps.event_sequence;
    copy_slots_entities_to_state(&client.ps.events, &mut state.ps.events);
    copy_slots_entities_to_state(&client.ps.event_parms, &mut state.ps.event_parms);
    state.ps.external_event = client.ps.external_event;
    state.ps.external_event_parm = client.ps.external_event_parm;
    state.ps.external_event_time = client.ps.external_event_time;
    state.ps.client_num = client.ps.client_num;
    state.ps.weapon = client.ps.weapon as i32;
    state.ps.weapon_state = client.ps.weapon_state as i32;
    state.ps.viewangles = client.ps.viewangles;
    copy_slots_entities_to_state(&client.ps.stats, &mut state.ps.stats);
    copy_slots_entities_to_state(&client.ps.persistant, &mut state.ps.persistant);
    copy_slots_entities_to_state(&client.ps.powerups, &mut state.ps.powerups);
    copy_slots_entities_to_state(&client.ps.ammo, &mut state.ps.ammo);
    state.ps.generic1 = client.ps.generic1;
    state.ps.pmove_framecount = client.ps.pmove_framecount;
    state.pers.connected = client.pers.connected as i32;
    state.pers.predict_item_pickup = client.pers.predict_item_pickup;
    state.pers.netname.clone_from(&client.pers.netname);
    state.sess.session_team = client.sess.session_team as i32;
    state.sess.spectator_state = client.sess.spectator_state as i32;
    state.sess.spectator_client = client.sess.spectator_client;
    state.noclip = client.noclip;
    state.damage_armor = client.damage_armor;
    state.damage_blood = client.damage_blood;
    state.damage_knockback = client.damage_knockback;
    state.damage_from = client.damage_from;
    state.damage_from_world = client.damage_from_world;
    state.last_killed_client = client.last_killed_client;
    state.last_hurt_client = client.last_hurt_client;
    state.last_hurt_mod = client.last_hurt_mod;
    state.respawn_time = client.respawn_time;
    state.reward_time = client.reward_time;
    state.last_kill_time = client.last_kill_time;
}

/// Copy slot words between the entities and state slot containers.
fn copy_slots_entities_to_state(from: &EntitiesPlayerSlots, to: &mut StatePlayerSlots) {
    let len = from.len().min(to.len());
    for index in 0..len {
        #[allow(clippy::cast_possible_wrap)]
        to.set(index, from.get(index as i32));
    }
}

/// Copy slot words from the state container back into the entities container.
fn copy_slots_state_to_entities(from: &StatePlayerSlots, to: &mut EntitiesPlayerSlots) {
    let len = from.len().min(to.len());
    for index in 0..len {
        #[allow(clippy::cast_possible_wrap)]
        to.set(index as i32, from.get(index));
    }
}

/// Copy one slot from the driver state pool back into the entities pool.
fn sync_state_to_entities(pool: &BaseEntityPool, state_pool: &RuntimeStatePool, slot: usize) {
    let Some(state) = state_pool.entities.get(slot) else {
        return;
    };
    let target = pool.at(slot as i32);
    let mut borrowed = target.borrow_mut();
    borrowed.inuse = state.inuse;
    borrowed.s = state.s.clone();
    borrowed.r.sv_flags = state.r.sv_flags;
    borrowed.r.single_client = state.r.single_client;
    borrowed.r.contents = state.r.contents;
    borrowed.r.owner_num = state.r.owner_num;
    borrowed.r.mins = state.r.mins;
    borrowed.r.maxs = state.r.maxs;
    borrowed.r.current_origin = state.r.current_origin;
    borrowed.r.current_angles = state.r.current_angles;
    borrowed.r.linked = state.r.linked;
    borrowed.r.ground.clone_from(&state.r.ground);
    borrowed.classname = resolve_state_classname(state_pool, state);
    borrowed.spawnflags = state.spawnflags;
    borrowed.never_free = state.never_free;
    borrowed.flags = state.flags;
    borrowed.freetime = state.freetime;
    borrowed.event_time = state.event_time;
    borrowed.free_after_event = state.free_after_event;
    borrowed.unlink_after_event = state.unlink_after_event;
    borrowed.physics_bounce = state.physics_bounce as f32;
    borrowed.clipmask = state.clipmask;
    borrowed.mover_state = StateMoverState::from_i32(state.mover_state).expect("mover state");
    borrowed.nextthink = state.nextthink;
    borrowed.health = state.health;
    borrowed.takedamage = state.takedamage;
    borrowed.count = state.count;
    borrowed.teamchain = state.teamchain.map(|slot| pool.at(slot as i32));
    borrowed.teammaster = state.teammaster.map(|slot| pool.at(slot as i32));
    borrowed.team.clone_from(&state.team);
    borrowed.targetname.clone_from(&state.targetname);
    borrowed.target.clone_from(&state.target);
    borrowed.wait = state.wait;
    borrowed.random = state.random;
    if let (Some(client), Some(index)) = (borrowed.client.as_mut(), state.client) {
        if let Some(state_client) = state_pool.clients.get(index) {
            sync_state_client_to_entities(state_client, client);
        }
    }
}

/// Copy shared client fields from a state client back into an entities client.
fn sync_state_client_to_entities(state: &StateGameClient, client: &mut EntitiesGameClient) {
    client.ps.pm_type = move_type_from_i32(state.ps.pm_type);
    client.ps.pm_flags = state.ps.pm_flags;
    client.ps.pm_time = state.ps.pm_time;
    client.ps.origin = state.ps.origin;
    client.ps.legs_anim = state.ps.legs_anim;
    client.ps.torso_anim = state.ps.torso_anim;
    client.ps.e_flags = state.ps.e_flags;
    client.ps.event_sequence = state.ps.event_sequence;
    copy_slots_state_to_entities(&state.ps.events, &mut client.ps.events);
    copy_slots_state_to_entities(&state.ps.event_parms, &mut client.ps.event_parms);
    client.ps.external_event = state.ps.external_event;
    client.ps.external_event_parm = state.ps.external_event_parm;
    client.ps.external_event_time = state.ps.external_event_time;
    client.ps.client_num = state.ps.client_num;
    client.ps.weapon = Weapon::from_i32(state.ps.weapon).expect("client weapon");
    client.ps.weapon_state = weapon_state_from_i32(state.ps.weapon_state);
    client.ps.viewangles = state.ps.viewangles;
    copy_slots_state_to_entities(&state.ps.stats, &mut client.ps.stats);
    copy_slots_state_to_entities(&state.ps.persistant, &mut client.ps.persistant);
    copy_slots_state_to_entities(&state.ps.powerups, &mut client.ps.powerups);
    copy_slots_state_to_entities(&state.ps.ammo, &mut client.ps.ammo);
    client.ps.generic1 = state.ps.generic1;
    client.ps.pmove_framecount = state.ps.pmove_framecount;
    client.pers.predict_item_pickup = state.pers.predict_item_pickup;
    client.pers.netname.clone_from(&state.pers.netname);
    client.sess.session_team = Team::from_i32(state.sess.session_team).expect("session team");
    client.sess.spectator_state = spectator_state_from_i32(state.sess.spectator_state);
    client.sess.spectator_client = state.sess.spectator_client;
    client.noclip = state.noclip;
    client.damage_armor = state.damage_armor;
    client.damage_blood = state.damage_blood;
    client.damage_knockback = state.damage_knockback;
    client.damage_from = state.damage_from;
    client.damage_from_world = state.damage_from_world;
    client.last_killed_client = state.last_killed_client;
    client.last_hurt_client = state.last_hurt_client;
    client.last_hurt_mod = state.last_hurt_mod;
    client.respawn_time = state.respawn_time;
    client.reward_time = state.reward_time;
    client.last_kill_time = state.last_kill_time;
}

/// Reconcile allocation marks, then copy one state slot back into entities.
fn reconcile_and_sync_state_to_entities(
    pool: &Rc<RefCell<BaseEntityPool>>,
    driver: &Rc<RefCell<RuntimeDriver>>,
    slot: usize,
) {
    let borrowed_driver = driver.borrow();
    sync_state_to_entities(&pool.borrow(), borrowed_driver.state(), slot);
}

/// Copy one slot from the entities pool into the team pool.
fn sync_entities_to_team(pool: &BaseEntityPool, team_pool: &PoolRef, slot: usize) {
    let borrowed = pool.at(slot as i32);
    let borrowed = borrowed.borrow();
    while team_pool.num_entities() <= slot {
        team_pool.spawn();
    }
    let target = team_pool.at(slot);
    let mut team = target.borrow_mut();
    team.inuse = borrowed.inuse;
    team.actor = borrowed.actor.id.clone();
    team.never_free = borrowed.never_free;
    team.set_classname(borrowed.classname.clone());
    team.s = borrowed.s.clone();
    team.r.sv_flags = borrowed.r.sv_flags;
    team.r.single_client = borrowed.r.single_client;
    team.r.contents = borrowed.r.contents;
    team.r.owner_num = borrowed.r.owner_num;
    team.r.mins = borrowed.r.mins;
    team.r.maxs = borrowed.r.maxs;
    team.health = borrowed.health;
    team.takedamage = borrowed.takedamage;
    team.flags = borrowed.flags;
    team.physics_bounce = borrowed.physics_bounce as i32;
    team.nextthink = borrowed.nextthink;
    team.event_time = borrowed.event_time;
    team.free_after_event = borrowed.free_after_event;
    team.freetime = borrowed.freetime;
    team.spawnflags = borrowed.spawnflags;
    team.clipmask = borrowed.clipmask;
    team.count = borrowed.count;
    team.target.clone_from(&borrowed.target);
    team.targetname.clone_from(&borrowed.targetname);
    team.item.clone_from(&borrowed.item);
    if let (Some(client), Some(team_client)) = (borrowed.client.as_ref(), team.client.as_ref()) {
        let mut team_client = team_client.borrow_mut();
        sync_entities_client_to_team(client, &mut team_client);
    }
}

/// Entities-pool touch callback.
type EntitiesTouchCallback = qa_content::q3::base::game::entities::TouchCallback;
/// Entities-pool think callback.
type EntitiesThinkCallback = qa_content::q3::base::game::entities::ThinkCallback;
/// Items-pool client record.
type ItemsGameClient = qa_content::q3::base::game::items_core::GameClient;
/// Items-pool player-state slots.
type ItemsPlayerSlots = qa_content::q3::base::game::items_core::PlayerStateSlots;

/// Copy entities-pool slots into items-pool slots elementwise.
fn copy_slots_entities_to_items(from: &EntitiesPlayerSlots, to: &mut ItemsPlayerSlots) {
    let len = from.len().min(to.len());
    for index in 0..len {
        let _ = to.set(index, from.get(index as i32));
    }
}

/// Copy items-pool slots into entities-pool slots elementwise.
fn copy_slots_items_to_entities(from: &ItemsPlayerSlots, to: &mut EntitiesPlayerSlots) {
    let len = from.len().min(to.len());
    for index in 0..len {
        if let Ok(value) = from.get(index) {
            to.set(index as i32, value);
        }
    }
}

/// Mirror an entities-pool client into an items-pool client.
///
/// Only fields present on both records are copied; items-only player state
/// (velocity, health, weapon timers) and the accuracy counters stay owned by
/// the items pool, while entities-only state (weapon state, damage totals,
/// respawn timers) stays owned by the entities pool.
fn sync_entities_client_to_items(client: &EntitiesGameClient, target: &mut ItemsGameClient) {
    target.ps.client_num = client.ps.client_num;
    target.ps.origin = client.ps.origin;
    target.ps.viewangles = client.ps.viewangles;
    target.ps.pm_time = client.ps.pm_time;
    target.ps.pm_flags = client.ps.pm_flags;
    target.ps.pm_type = client.ps.pm_type;
    target.ps.e_flags = client.ps.e_flags;
    copy_slots_entities_to_items(&client.ps.stats, &mut target.ps.stats);
    copy_slots_entities_to_items(&client.ps.persistant, &mut target.ps.persistant);
    copy_slots_entities_to_items(&client.ps.powerups, &mut target.ps.powerups);
    copy_slots_entities_to_items(&client.ps.ammo, &mut target.ps.ammo);
    target.ps.weapon = client.ps.weapon;
    target.ps.legs_anim = client.ps.legs_anim;
    target.ps.torso_anim = client.ps.torso_anim;
    target.ps.generic1 = client.ps.generic1;
    target.ps.external_event = client.ps.external_event;
    target.ps.external_event_parm = client.ps.external_event_parm;
    target.ps.external_event_time = client.ps.external_event_time;
    target.ps.event_sequence = client.ps.event_sequence;
    copy_slots_entities_to_items(&client.ps.events, &mut target.ps.events);
    copy_slots_entities_to_items(&client.ps.event_parms, &mut target.ps.event_parms);
    target.pers.connected = client.pers.connected;
    target.sess.session_team = client.sess.session_team;
    target.hook = client.hook.as_ref().map(|entity| entity.borrow().slot);
    target.persistant_powerup = client.persistant_powerup.as_ref().map(|entity| entity.borrow().slot);
    copy_slots_entities_to_items(&client.ammo_times, &mut target.ammo_times);
    target.invulnerability_time = client.invulnerability_time;
}

/// Mirror an items-pool client into an entities-pool client.
///
/// Only fields present on both records are copied; pool-owned extras are
/// preserved on their owning side (see [`sync_entities_client_to_items`]).
fn sync_items_client_to_entities(pool: &BaseEntityPool, source: &ItemsGameClient, client: &mut EntitiesGameClient) {
    client.ps.client_num = source.ps.client_num;
    client.ps.origin = source.ps.origin;
    client.ps.viewangles = source.ps.viewangles;
    client.ps.pm_time = source.ps.pm_time;
    client.ps.pm_flags = source.ps.pm_flags;
    client.ps.pm_type = source.ps.pm_type;
    client.ps.e_flags = source.ps.e_flags;
    copy_slots_items_to_entities(&source.ps.stats, &mut client.ps.stats);
    copy_slots_items_to_entities(&source.ps.persistant, &mut client.ps.persistant);
    copy_slots_items_to_entities(&source.ps.powerups, &mut client.ps.powerups);
    copy_slots_items_to_entities(&source.ps.ammo, &mut client.ps.ammo);
    client.ps.weapon = source.ps.weapon;
    client.ps.legs_anim = source.ps.legs_anim;
    client.ps.torso_anim = source.ps.torso_anim;
    client.ps.generic1 = source.ps.generic1;
    client.ps.external_event = source.ps.external_event;
    client.ps.external_event_parm = source.ps.external_event_parm;
    client.ps.external_event_time = source.ps.external_event_time;
    client.ps.event_sequence = source.ps.event_sequence;
    copy_slots_items_to_entities(&source.ps.events, &mut client.ps.events);
    copy_slots_items_to_entities(&source.ps.event_parms, &mut client.ps.event_parms);
    client.pers.connected = source.pers.connected;
    client.sess.session_team = source.sess.session_team;
    client.hook = source.hook.map(|slot| pool.at(slot as i32));
    client.persistant_powerup = source.persistant_powerup.map(|slot| pool.at(slot as i32));
    copy_slots_items_to_entities(&source.ammo_times, &mut client.ammo_times);
    client.invulnerability_time = source.invulnerability_time;
}

/// Copy shared client fields from an entities client into a team client.
fn sync_entities_client_to_team(client: &EntitiesGameClient, team: &mut TeamGameClient) {
    team.ps.pm_type = client.ps.pm_type as i32;
    team.ps.pm_flags = client.ps.pm_flags;
    team.ps.pm_time = client.ps.pm_time;
    team.ps.origin = client.ps.origin;
    team.ps.legs_anim = client.ps.legs_anim;
    team.ps.torso_anim = client.ps.torso_anim;
    team.ps.e_flags = client.ps.e_flags;
    team.ps.event_sequence = client.ps.event_sequence;
    copy_slots_entities_to_team(&client.ps.events, &team.ps.events);
    copy_slots_entities_to_team(&client.ps.event_parms, &team.ps.event_parms);
    team.ps.external_event = client.ps.external_event;
    team.ps.external_event_parm = client.ps.external_event_parm;
    team.ps.external_event_time = client.ps.external_event_time;
    team.ps.client_num = client.ps.client_num;
    team.ps.weapon = client.ps.weapon as i32;
    team.ps.weapon_state = client.ps.weapon_state as i32;
    team.ps.viewangles = client.ps.viewangles;
    copy_slots_entities_to_team(&client.ps.stats, &team.ps.stats);
    copy_slots_entities_to_team(&client.ps.persistant, &team.ps.persistant);
    copy_slots_entities_to_team(&client.ps.powerups, &team.ps.powerups);
    copy_slots_entities_to_team(&client.ps.ammo, &team.ps.ammo);
    team.ps.generic1 = client.ps.generic1;
    team.ps.pmove_framecount = client.ps.pmove_framecount;
    team.pers.predict_item_pickup = client.pers.predict_item_pickup;
    team.pers.netname.clone_from(&client.pers.netname);
    team.sess.session_team = client.sess.session_team as i32;
    team.sess.spectator_state = client.sess.spectator_state as i32;
    team.sess.spectator_client = client.sess.spectator_client;
    team.noclip = client.noclip;
    team.damage_armor = client.damage_armor;
    team.damage_blood = client.damage_blood;
    team.damage_knockback = client.damage_knockback;
    team.damage_from = client.damage_from;
    team.damage_from_world = client.damage_from_world;
    team.last_killed_client = client.last_killed_client;
    team.last_hurt_mod = client.last_hurt_mod;
    team.respawn_time = client.respawn_time;
    team.reward_time = client.reward_time;
}

/// Copy slot words from an entities container into a team container.
fn copy_slots_entities_to_team(from: &EntitiesPlayerSlots, to: &TeamSlotArray) {
    let len = from.len().min(to.len());
    for index in 0..len {
        #[allow(clippy::cast_possible_wrap)]
        to.set(index, from.get(index as i32));
    }
}

/// Copy one slot from the team pool back into the entities pool.
fn sync_team_to_entities(team_pool: &PoolRef, pool: &BaseEntityPool, slot: usize) {
    let borrowed = team_pool.at(slot);
    let borrowed = borrowed.borrow();
    let target = pool.at(slot as i32);
    let mut entity = target.borrow_mut();
    entity.inuse = borrowed.inuse;
    entity.s = borrowed.s.clone();
    entity.r.sv_flags = borrowed.r.sv_flags;
    entity.r.single_client = borrowed.r.single_client;
    entity.r.contents = borrowed.r.contents;
    entity.r.owner_num = borrowed.r.owner_num;
    entity.r.mins = borrowed.r.mins;
    entity.r.maxs = borrowed.r.maxs;
    entity.health = borrowed.health;
    entity.takedamage = borrowed.takedamage;
    entity.flags = borrowed.flags;
    entity.physics_bounce = borrowed.physics_bounce as f32;
    entity.nextthink = borrowed.nextthink;
    entity.event_time = borrowed.event_time;
    entity.free_after_event = borrowed.free_after_event;
    entity.freetime = borrowed.freetime;
    entity.spawnflags = borrowed.spawnflags;
    entity.clipmask = borrowed.clipmask;
    entity.count = borrowed.count;
    entity.target.clone_from(&borrowed.target);
    entity.targetname.clone_from(&borrowed.targetname);
    entity.item.clone_from(&borrowed.item);
    if let (Some(client), Some(team_client)) = (entity.client.as_mut(), borrowed.client.as_ref()) {
        let team_client = team_client.borrow();
        sync_team_client_to_entities(&team_client, client);
    }
}

/// Copy shared client fields from a team client back into an entities client.
fn sync_team_client_to_entities(team: &TeamGameClient, client: &mut EntitiesGameClient) {
    client.ps.pm_type = move_type_from_i32(team.ps.pm_type);
    client.ps.pm_flags = team.ps.pm_flags;
    client.ps.pm_time = team.ps.pm_time;
    client.ps.origin = team.ps.origin;
    client.ps.legs_anim = team.ps.legs_anim;
    client.ps.torso_anim = team.ps.torso_anim;
    client.ps.e_flags = team.ps.e_flags;
    client.ps.event_sequence = team.ps.event_sequence;
    copy_slots_team_to_entities(&team.ps.events, &mut client.ps.events);
    copy_slots_team_to_entities(&team.ps.event_parms, &mut client.ps.event_parms);
    client.ps.external_event = team.ps.external_event;
    client.ps.external_event_parm = team.ps.external_event_parm;
    client.ps.external_event_time = team.ps.external_event_time;
    client.ps.client_num = team.ps.client_num;
    client.ps.weapon = Weapon::from_i32(team.ps.weapon).expect("team weapon");
    client.ps.weapon_state = weapon_state_from_i32(team.ps.weapon_state);
    client.ps.viewangles = team.ps.viewangles;
    copy_slots_team_to_entities(&team.ps.stats, &mut client.ps.stats);
    copy_slots_team_to_entities(&team.ps.persistant, &mut client.ps.persistant);
    copy_slots_team_to_entities(&team.ps.powerups, &mut client.ps.powerups);
    copy_slots_team_to_entities(&team.ps.ammo, &mut client.ps.ammo);
    client.ps.generic1 = team.ps.generic1;
    client.ps.pmove_framecount = team.ps.pmove_framecount;
    client.pers.predict_item_pickup = team.pers.predict_item_pickup;
    client.pers.netname.clone_from(&team.pers.netname);
    client.sess.session_team = Team::from_i32(team.sess.session_team).expect("team session team");
    client.sess.spectator_state = spectator_state_from_i32(team.sess.spectator_state);
    client.sess.spectator_client = team.sess.spectator_client;
    client.noclip = team.noclip;
    client.damage_armor = team.damage_armor;
    client.damage_blood = team.damage_blood;
    client.damage_knockback = team.damage_knockback;
    client.damage_from = team.damage_from;
    client.damage_from_world = team.damage_from_world;
    client.last_killed_client = team.last_killed_client;
    client.last_hurt_mod = team.last_hurt_mod;
    client.respawn_time = team.respawn_time;
    client.reward_time = team.reward_time;
}

/// Copy slot words from a team container back into an entities container.
fn copy_slots_team_to_entities(from: &TeamSlotArray, to: &mut EntitiesPlayerSlots) {
    let len = from.len().min(to.len());
    for index in 0..len {
        #[allow(clippy::cast_possible_wrap)]
        to.set(index as i32, from.get(index));
    }
}

/// Copy one state slot into entities, then into the team pool.
fn sync_state_to_entities_team(
    pool: &Rc<RefCell<BaseEntityPool>>,
    team_pool: &PoolRef,
    driver: &Rc<RefCell<RuntimeDriver>>,
    slot: usize,
) {
    let borrowed_driver = driver.borrow();
    sync_state_to_entities(&pool.borrow(), borrowed_driver.state(), slot);
    drop(borrowed_driver);
    sync_entities_to_team(&pool.borrow(), team_pool, slot);
}

/// Copy one items slot into the entities pool, then into the team pool.
fn sync_items_to_entities_team(
    items_pool: &ItemsEntityPool,
    pool: &Rc<RefCell<BaseEntityPool>>,
    team_pool: &PoolRef,
    product: Product,
    slot: usize,
) {
    sync_items_to_entities(items_pool, pool, product, slot);
    sync_entities_to_team(&pool.borrow(), team_pool, slot);
}

/// Mirror an items-pool slot into the entities pool.
fn sync_items_to_entities(
    items_pool: &ItemsEntityPool,
    pool: &Rc<RefCell<BaseEntityPool>>,
    product: Product,
    slot: usize,
) {
    let Ok(source) = items_pool.at(slot) else {
        return;
    };
    let target = pool.borrow().at(slot as i32);
    let mut entity = target.borrow_mut();
    entity.inuse = source.inuse;
    copy_items_state_to_shared(&source.s, &mut entity.s);
    entity.r.sv_flags = source.r.sv_flags;
    entity.r.contents = source.r.contents;
    entity.r.owner_num = source.r.owner_num;
    entity.r.mins = source.r.mins;
    entity.r.maxs = source.r.maxs;
    entity.r.current_origin = source.r.current_origin;
    entity.r.current_angles = source.r.current_angles;
    entity.classname.clone_from(&source.classname);
    entity.spawnflags = source.spawnflags;
    entity.event_time = source.event_time;
    entity.free_after_event = source.free_after_event;
    entity.physics_bounce = source.physics_bounce;
    entity.health = source.health;
    entity.takedamage = source.takedamage;
    entity.target.clone_from(&source.target);
    entity.targetname.clone_from(&source.targetname);
    match (&source.client, &mut entity.client) {
        (Some(source), Some(client)) => sync_items_client_to_entities(&pool.borrow(), source, client),
        (Some(source), client @ None) => {
            let mut fresh = EntitiesGameClient::new(product);
            sync_items_client_to_entities(&pool.borrow(), source, &mut fresh);
            *client = Some(fresh);
        }
        (None, client) => *client = None,
    }
    drop(entity);
}

/// Mirror an entities-pool slot into the items pool.
fn sync_entities_to_items(pool: &BaseEntityPool, items_pool: &mut ItemsEntityPool, product: Product, slot: usize) {
    let source = pool.at(slot as i32);
    let borrowed = source.borrow();
    let Ok(target) = items_pool.at_mut(slot) else {
        return;
    };
    target.inuse = borrowed.inuse;
    copy_shared_state_to_items(&borrowed.s, &mut target.s);
    target.r.sv_flags = borrowed.r.sv_flags;
    target.r.contents = borrowed.r.contents;
    target.r.owner_num = borrowed.r.owner_num;
    target.r.mins = borrowed.r.mins;
    target.r.maxs = borrowed.r.maxs;
    target.r.current_origin = borrowed.r.current_origin;
    target.r.current_angles = borrowed.r.current_angles;
    target.classname.clone_from(&borrowed.classname);
    target.spawnflags = borrowed.spawnflags;
    target.event_time = borrowed.event_time;
    target.free_after_event = borrowed.free_after_event;
    target.physics_bounce = borrowed.physics_bounce;
    target.health = borrowed.health;
    target.takedamage = borrowed.takedamage;
    target.target.clone_from(&borrowed.target);
    target.targetname.clone_from(&borrowed.targetname);
    match (&borrowed.client, &mut target.client) {
        (Some(client), Some(target)) => sync_entities_client_to_items(client, target),
        (Some(client), target @ None) => {
            let mut fresh = ItemsGameClient::new(product);
            sync_entities_client_to_items(client, &mut fresh);
            *target = Some(fresh);
        }
        (None, target) => *target = None,
    }
}

/// Mirror every inuse entities-pool slot into the state pool.
fn sync_all_entities_to_state(pool: &BaseEntityPool, state_pool: &mut RuntimeStatePool) {
    for slot in 0..MAX_GENTITIES {
        if pool.at(slot as i32).borrow().inuse {
            sync_entities_to_state(pool, state_pool, slot);
        }
    }
}

/// Propagate newly allocated slots across every pool, then bind or release records.
///
/// Only slots that are free in the destination pool are touched, so shared live
/// slots keep their values; directional value flows happen at explicit boundaries.
fn reconcile_spawn_union(core: &Rc<RefCell<CoreLinks>>) {
    let (product, pool, items_pool, team_pool, driver, records) = {
        let links = core.borrow();
        (
            links.product.expect("product"),
            links.base_pool.clone().expect("base pool"),
            links.items_pool.clone().expect("items pool"),
            links.team_pool.clone().expect("team pool"),
            links.driver.clone().expect("driver"),
            links.records.clone().expect("records"),
        )
    };
    for slot in 0..MAX_GENTITIES {
        let in_state = driver
            .borrow()
            .state()
            .entities
            .get(slot)
            .is_some_and(|entity| entity.inuse);
        let in_entities = pool.borrow().at(slot as i32).borrow().inuse;
        let in_items = items_pool.borrow().get(slot).is_some_and(|entity| entity.inuse);
        let in_team = team_pool.at(slot).borrow().inuse;
        if !in_state && !in_entities && !in_items && !in_team {
            if slot >= MAX_CLIENTS {
                if let Some(record) = records.get(slot) {
                    if record.borrow().inuse() {
                        records.release(record);
                    }
                }
            }
            continue;
        }
        if in_items && !in_entities {
            sync_items_to_entities(&items_pool.borrow(), &pool, product, slot);
        }
        if pool.borrow().at(slot as i32).borrow().inuse && !in_state {
            sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), slot);
        }
        if in_state && !pool.borrow().at(slot as i32).borrow().inuse {
            sync_state_to_entities(&pool.borrow(), driver.borrow().state(), slot);
            sync_state_to_items(driver.borrow().state(), &mut items_pool.borrow_mut(), slot);
        }
        if pool.borrow().at(slot as i32).borrow().inuse {
            if !team_pool.at(slot).borrow().inuse {
                sync_entities_to_team(&pool.borrow(), &team_pool, slot);
            }
            if !items_pool.borrow().get(slot).is_some_and(|entity| entity.inuse) {
                sync_entities_to_items(&pool.borrow(), &mut items_pool.borrow_mut(), product, slot);
            }
        }
        let transient = driver
            .borrow()
            .state()
            .entities
            .get(slot)
            .is_some_and(|entity| entity.free_after_event || entity.unlink_after_event);
        if !transient && (in_state || pool.borrow().at(slot as i32).borrow().inuse) {
            let _ = records.activate(slot);
        }
    }
    let high = pool.borrow().num_entities().max(MAX_CLIENTS + 1);
    let max_clients = pool.borrow().max_clients();
    pool.borrow_mut().restore_counts(high, max_clients);
}

/// Run queued target activations once the driver borrow is free.
fn drain_pending_targets(core: &Rc<RefCell<CoreLinks>>) {
    loop {
        let next = core.borrow_mut().pending_targets.pop();
        let Some((slot, participant)) = next else {
            return;
        };
        let (pool, driver) = {
            let links = core.borrow();
            (
                links.base_pool.clone().expect("base pool"),
                links.driver.clone().expect("driver"),
            )
        };
        sync_all_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut());
        base_use_targets(&mut *driver.borrow_mut(), slot, participant).expect("drain targets");
        reconcile_and_sync_state_to_entities(&pool, &driver, slot);
        reconcile_spawn_union(core);
    }
}

/// Copy death-runtime team scores into the shared slots.
fn sync_death_scores_to_shared(death: &DeathRuntime, scores: &SharedSlots) {
    let array = death.host.team_scores.borrow();
    for (index, value) in array.iter().enumerate() {
        scores.set(index, *value);
    }
}

/// Copy one state slot into the items pool.
fn sync_state_to_items(state_pool: &RuntimeStatePool, items_pool: &mut ItemsEntityPool, slot: usize) {
    let Some(state) = state_pool.entities.get(slot) else {
        return;
    };
    let Ok(target) = items_pool.at_mut(slot) else {
        return;
    };
    target.inuse = state.inuse;
    copy_shared_state_to_items(&state.s, &mut target.s);
    target.r.current_origin = state.r.current_origin;
    target.r.current_angles = state.r.current_angles;
    target.r.mins = state.r.mins;
    target.r.maxs = state.r.maxs;
    target.r.contents = state.r.contents;
    target.r.owner_num = state.r.owner_num;
    target.r.sv_flags = state.r.sv_flags;
    target.classname = resolve_state_classname(state_pool, state);
    target.health = state.health;
    target.takedamage = state.takedamage;
    target.spawnflags = state.spawnflags;
    target.event_time = state.event_time;
    target.free_after_event = state.free_after_event;
    target.target.clone_from(&state.target);
    target.targetname.clone_from(&state.targetname);
}

/// Mirror an items-pool slot into the driver state pool.
fn sync_items_to_state(items_pool: &ItemsEntityPool, state_pool: &mut RuntimeStatePool, slot: usize) {
    let Ok(source) = items_pool.at(slot) else {
        return;
    };
    let Some(target) = state_pool.entities.get_mut(slot) else {
        return;
    };
    target.inuse = source.inuse;
    copy_items_state_to_shared(&source.s, &mut target.s);
    target.r.current_origin = source.r.current_origin;
    target.r.current_angles = source.r.current_angles;
    target.r.mins = source.r.mins;
    target.r.maxs = source.r.maxs;
    target.r.contents = source.r.contents;
    target.r.owner_num = source.r.owner_num;
    target.r.sv_flags = source.r.sv_flags;
    target.set_classname(source.classname.clone());
    target.health = source.health;
    target.takedamage = source.takedamage;
    target.spawnflags = source.spawnflags;
    target.event_time = source.event_time;
    target.free_after_event = source.free_after_event;
    target.target.clone_from(&source.target);
    target.targetname.clone_from(&source.targetname);
}

/// Mirror a driver-pool slot into the items pool through the state trait.
fn sync_dyn_state_to_items(pool: &dyn StateEntityPool, items_pool: &mut ItemsEntityPool, slot: usize) {
    let Some(state) = pool.entity(slot) else {
        return;
    };
    let Ok(target) = items_pool.at_mut(slot) else {
        return;
    };
    target.inuse = state.inuse;
    copy_shared_state_to_items(&state.s, &mut target.s);
    target.r.current_origin = state.r.current_origin;
    target.r.current_angles = state.r.current_angles;
    target.r.mins = state.r.mins;
    target.r.maxs = state.r.maxs;
    target.r.contents = state.r.contents;
    target.r.owner_num = state.r.owner_num;
    target.r.sv_flags = state.r.sv_flags;
    let netname = state
        .client
        .and_then(|index| pool.client(index))
        .map(|client| client.pers.netname.clone());
    target.classname = state.resolve_classname(netname.as_deref());
    target.health = state.health;
    target.takedamage = state.takedamage;
    target.spawnflags = state.spawnflags;
    target.event_time = state.event_time;
    target.free_after_event = state.free_after_event;
    target.target.clone_from(&state.target);
    target.targetname.clone_from(&state.targetname);
}

/// Mirror an items-pool slot into the driver pool through the state trait.
fn sync_items_to_dyn_state(items_pool: &ItemsEntityPool, pool: &mut dyn StateEntityPool, slot: usize) {
    let Ok(source) = items_pool.at(slot) else {
        return;
    };
    let Some(target) = pool.entity_mut(slot) else {
        return;
    };
    target.inuse = source.inuse;
    copy_items_state_to_shared(&source.s, &mut target.s);
    target.r.current_origin = source.r.current_origin;
    target.r.current_angles = source.r.current_angles;
    target.r.mins = source.r.mins;
    target.r.maxs = source.r.maxs;
    target.r.contents = source.r.contents;
    target.r.owner_num = source.r.owner_num;
    target.r.sv_flags = source.r.sv_flags;
    target.set_classname(source.classname.clone());
    target.health = source.health;
    target.takedamage = source.takedamage;
    target.spawnflags = source.spawnflags;
    target.event_time = source.event_time;
    target.free_after_event = source.free_after_event;
    target.target.clone_from(&source.target);
    target.targetname.clone_from(&source.targetname);
}

/// Convert an items-pool damage participant into a driver participant.
fn state_participant_for_items(records: &Q3EntityRecords, participant: &ItemsDamageParticipant) -> StateParticipant {
    match participant {
        ItemsDamageParticipant::Entity(slot) => StateParticipant::Entity(*slot),
        ItemsDamageParticipant::SharedActor(actor) => {
            let native = records.get(actor.0 as usize).expect("shared actor record");
            let id = native.borrow().binding.actor().id().clone();
            StateParticipant::SharedActor(id)
        }
    }
}

/// Add-only allocation union: mirror newly inuse slots into pools missing them.
fn reconcile_pools(pool: &Rc<RefCell<BaseEntityPool>>, items_pool: &Rc<RefCell<ItemsEntityPool>>, team_pool: &PoolRef) {
    let borrowed = pool.borrow();
    let borrowed_items = items_pool.borrow();
    let mut high = borrowed.num_entities();
    for slot in 0..MAX_GENTITIES {
        let in_entities = borrowed.at(slot as i32).borrow().inuse;
        let in_items = borrowed_items.get(slot).is_some_and(|entity| entity.inuse);
        let in_team = team_pool.at(slot).borrow().inuse;
        if !(in_entities || in_items || in_team) {
            continue;
        }
        high = high.max(slot + 1);
        // NOTE: value sync for shared slots happens at explicit directional
        // boundaries; reconcile only mirrors allocation flags for new slots.
        if in_entities && !in_team {
            drop(borrowed);
            drop(borrowed_items);
            sync_entities_to_team(&pool.borrow(), team_pool, slot);
            return reconcile_pools(pool, items_pool, team_pool);
        }
    }
    drop(borrowed);
    drop(borrowed_items);
    let capped = high.min(ENTITYNUM_WORLD as usize).max(MAX_CLIENTS);
    pool.borrow_mut().restore_counts(capped, pool.borrow().max_clients());
}

/// Map a session actor to its entities-pool damage participant.
fn entities_participant_for_actor(
    pool: &BaseEntityPool,
    records: &Q3EntityRecords,
    actor: &ActorId,
) -> EntitiesDamageParticipant {
    match records.native_by_actor(Some(actor)) {
        Some(native) => EntitiesDamageParticipant::Native(pool.at(native.borrow().slot as i32)),
        None => EntitiesDamageParticipant::Shared(SharedActor {
            actor: actor.clone(),
            origin: None,
        }),
    }
}

// ---------------------------------------------------------------------------
// Missiles.
// ---------------------------------------------------------------------------

/// Surface flag bit for no-damage surfaces (donor `SURF_NODAMAGE`).
const HITSCAN_SURF_NODAMAGE: i32 = 0x10;
/// Contents bit for solid brushes (donor `CONTENTS_SOLID`).
const HITSCAN_CONTENTS_SOLID: i32 = 0x1;

/// Game-random adapter over the shared random mirror.
struct MissileRandomAdapter {
    random: Rc<RefCell<GameRandomMirror>>,
}

impl ItemsGameRandom for MissileRandomAdapter {
    fn rand_int(&mut self) -> i32 {
        self.random.borrow_mut().rand_value()
    }

    fn random(&mut self) -> f32 {
        self.random.borrow_mut().random_value()
    }

    fn crandom(&mut self) -> f32 {
        self.random.borrow_mut().crandom_value()
    }
}

/// Missile host over runtime links with a persistent body table.
struct RuntimeMissileHost {
    links: Rc<RefCell<CoreLinks>>,
    combat: RuntimeCombatOps,
    world: RuntimeWorldOps,
    bodies: BodyTable,
    random: MissileRandomAdapter,
    behavior: Option<Rc<dyn Q3WeaponBehaviorPort>>,
}

impl RuntimeMissileHost {
    fn session_body(&self, origin: Vec3, velocity: Vec3) -> BodyState {
        BodyState {
            origin,
            angles: vec3(0.0, 0.0, 0.0),
            velocity,
            bounds: Bounds {
                min: vec3(0.0, 0.0, 0.0),
                max: vec3(0.0, 0.0, 0.0),
            },
            ground: None,
        }
    }

    fn owned_actor(&self, projectile: ItemsActorId) -> OwnedActor {
        let records = self.links.borrow().records.clone().expect("records");
        let native = records.get(projectile.0 as usize).expect("missile record");
        let owned = native.borrow().binding.actor();
        owned
    }
}

impl MissileHost for RuntimeMissileHost {
    fn combat(&mut self) -> &mut dyn ItemsCombatOps {
        &mut self.combat
    }

    fn world(&mut self) -> &mut dyn ItemsWorldOps {
        &mut self.world
    }

    fn combat_and_world(&mut self) -> (&mut dyn ItemsCombatOps, &mut dyn ItemsWorldOps) {
        (&mut self.combat, &mut self.world)
    }

    fn bodies(&mut self) -> &mut BodyTable {
        &mut self.bodies
    }

    fn random(&mut self) -> &mut dyn ItemsGameRandom {
        &mut self.random
    }

    fn has_weapon_behavior(&self) -> bool {
        self.behavior.is_some()
    }

    fn weapon_behavior_launch(
        &mut self,
        projectile: ItemsActorId,
        shooter: ItemsActorId,
        weapon: Weapon,
        time_seconds: f64,
        origin: Vec3,
        velocity: Vec3,
    ) -> Option<MissileBodyState> {
        let behavior = self.behavior.clone()?;
        let declaration = q3_projectile_behavior(weapon as i32).ok()?;
        let owned = self.owned_actor(projectile);
        let body = self.session_body(origin, velocity);
        behavior
            .launch(&Q3BehaviorLaunch {
                projectile: owned,
                shooter: self.owned_actor(shooter).id().clone(),
                weapon: declaration.weapon,
                role: declaration.role,
                time_seconds,
                body,
            })
            .map(|update| MissileBodyState {
                origin: update.origin,
                velocity: update.velocity,
            })
    }

    fn weapon_behavior_step(
        &mut self,
        projectile: ItemsActorId,
        _origin: Vec3,
        _velocity: Vec3,
        time_seconds: f64,
    ) -> Option<MissileBodyState> {
        let behavior = self.behavior.clone()?;
        let body = self.bodies.read(projectile)?;
        let owned = self.owned_actor(projectile);
        behavior
            .step(&owned, &self.session_body(body.origin, body.velocity), time_seconds)
            .map(|update| MissileBodyState {
                origin: update.origin,
                velocity: update.velocity,
            })
    }

    fn is_missionpack(&self) -> bool {
        self.links.borrow().product == Some(Product::Missionpack)
    }

    fn prox_mine_timeout(&self) -> i32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_proxMineTimeout")
    }

    fn missionpack_sound_index(&mut self, path: &str) -> i32 {
        let config = self.links.borrow().config.clone().expect("config");
        let index = config.borrow_mut().sound_index(Some(path)).expect("missionpack sound") as i32;
        index
    }

    fn invulnerability_impact(
        &mut self,
        _pool: &mut ItemsEntityPool,
        target: Slot,
        direction: Vec3,
        point: Vec3,
    ) -> MissileInvulnerability {
        let (pool, driver) = {
            let links = self.links.borrow();
            (
                links.base_pool.clone().expect("base pool"),
                links.driver.clone().expect("driver"),
            )
        };
        sync_entities_to_state(&pool.borrow(), driver.borrow_mut().state_mut(), target);
        let impact = invulnerability_effect(&mut *driver.borrow_mut(), target, direction, point)
            .expect("missile invulnerability");
        reconcile_and_sync_state_to_entities(&pool, &driver, target);
        match impact {
            WeaponImpact::Miss => MissileInvulnerability::Miss,
            WeaponImpact::Hit { bounce_direction, .. } => MissileInvulnerability::Hit { bounce_direction },
        }
    }
}

/// Contact normal of an items-pool trace.
fn items_trace_normal(trace: &ItemsActorTraceResult) -> Vec3 {
    match &trace.contact {
        ItemsTraceContact::Plane { normal } => *normal,
        ItemsTraceContact::None => vec3(0.0, 0.0, 1.0),
    }
}

/// Projectile behavior over a missile context, mirroring donor projectile flow.
struct RuntimeProjectileDriver;

impl RuntimeProjectileDriver {
    fn record_actor(context: &mut ProjectileContext<'_>) -> ItemsActorId {
        context
            .runtime
            .capture_save_state()
            .into_iter()
            .find(|entry| entry.entity == context.slot)
            .map(|entry| ItemsActorId(entry.actor))
            .unwrap_or_else(|| ItemsActorId::from_slot(context.slot))
    }
}

impl ProjectileDriver for RuntimeProjectileDriver {
    fn launch(
        &mut self,
        start: Vec3,
        direction: Vec3,
        speed: f32,
        gravity: bool,
        duration: i32,
        time: i32,
    ) -> MissileLaunch {
        let fired = q3_launch_projectile(start, direction, speed, gravity, duration, time);
        MissileLaunch {
            expires: fired.expires,
            trajectory: fired.trajectory,
        }
    }

    fn bounce(&mut self, context: &mut ProjectileContext<'_>, trace: &ItemsActorTraceResult) -> Q3GameItemsResult<()> {
        let hit_time = q3_missile_hit_time(context.previous_time(), context.time(), trace.fraction);
        let plane = items_trace_normal(trace);
        let half = context.state.flags & 0x20 != 0;
        let delta = q3_bounce_velocity(
            evaluate_trajectory_delta(&context.state.trajectory, hit_time),
            plane,
            half,
        );
        context.state.trajectory.delta = delta;
        if half && plane.z > 0.2 && length3(delta) < 40.0 {
            context.set_origin_stop(trace.end)?;
            return Ok(());
        }
        let origin = add3(context.origin()?, plane);
        context.move_body(origin, delta)?;
        context.state.trajectory.base = origin;
        context.state.trajectory.time = context.time();
        Ok(())
    }

    fn explode(&mut self, context: &mut ProjectileContext<'_>) -> Q3GameItemsResult<()> {
        let actor = Self::record_actor(context);
        let time = context.time();
        let origin = snap_vector(evaluate_trajectory(&context.state.trajectory, time));
        context.set_origin_stop(origin)?;
        context.emit(&MissileImpactEmit::Impact {
            normal: vec3(0.0, 0.0, 1.0),
            target: None,
            flesh: false,
            surface_flags: 0,
        })?;
        if !context.live()? {
            return Ok(());
        }
        context.retain()?;
        if context.spec.splash != 0 && context.radius(origin, Some(actor))? {
            context.accuracy()?;
        }
        if context.live()? {
            context.link()?;
        }
        Ok(())
    }

    fn impact(&mut self, context: &mut ProjectileContext<'_>, trace: &ItemsActorTraceResult) -> Q3GameItemsResult<()> {
        let actor = match &trace.hit {
            ItemsTraceHit::Actor(actor) => *actor,
            _ => ItemsActorId::from_slot(ENTITYNUM_WORLD as usize),
        };
        let plane = items_trace_normal(trace);
        let target = context.target(actor)?;
        let damageable = target.is_some_and(|target| target.damageable);
        if !damageable && context.state.flags & 0x30 != 0 {
            self.bounce(context, trace)?;
            context.emit(&MissileImpactEmit::Bounce { normal: plane })?;
            return Ok(());
        }
        if context.has_reflection()
            && target.is_some_and(|target| target.damageable && target.invulnerable)
            && context.spec.weapon != Weapon::WpProxLauncher
        {
            let direction = normalize3(context.state.trajectory.delta);
            let point = context.state.trajectory.base;
            let effect = context.reflection_impact(actor, direction, point)?;
            if !context.live()? {
                return Ok(());
            }
            if let MissileInvulnerability::Hit { bounce_direction } = effect {
                let half = context.state.flags & 0x20;
                context.state.flags &= !0x20;
                let mut bounced = *trace;
                bounced.contact = ItemsTraceContact::Plane {
                    normal: bounce_direction,
                };
                self.bounce(context, &bounced)?;
                context.state.flags |= half;
            }
            return Ok(());
        }
        let mut hit_client = false;
        if damageable && context.spec.direct != 0 {
            if target.is_some_and(|target| target.accuracy_eligible) {
                context.accuracy()?;
                hit_client = true;
            }
            let time = context.time();
            let mut velocity = evaluate_trajectory_delta(&context.state.trajectory, time);
            if length3(velocity) == 0.0 {
                velocity = vec3(velocity.x, velocity.y, 1.0);
            }
            context.damage(actor, Some(velocity), context.spec.damage_point)?;
            if !context.live()? {
                return Ok(());
            }
        }
        if context.has_special()? {
            if context.special_impact(trace, actor)? || !context.live()? {
                return Ok(());
            }
        } else if !context.live()? {
            return Ok(());
        }
        let current = context.target(actor)?;
        context.emit(&MissileImpactEmit::Impact {
            normal: plane,
            target: Some(actor),
            flesh: current.is_some_and(|target| target.damageable && target.player),
            surface_flags: trace.surface_flags,
        })?;
        if !context.live()? {
            return Ok(());
        }
        context.retain()?;
        let origin = snap_vector_towards(trace.end, context.state.trajectory.base);
        context.set_origin_stop(origin)?;
        if context.spec.splash != 0 && context.radius(origin, Some(actor))? && !hit_client {
            context.accuracy()?;
        }
        if context.live()? {
            context.link()?;
        }
        Ok(())
    }

    fn step(&mut self, context: &mut ProjectileContext<'_>) -> Q3GameItemsResult<()> {
        if !context.live()? {
            return Ok(());
        }
        if context.phase()? == MissilePhase::Event {
            if context.time().wrapping_sub(context.event_time()?) > EVENT_VALID_MSEC {
                context.release()?;
            }
            return Ok(());
        }
        if context.time().wrapping_sub(context.event_time()?) > EVENT_VALID_MSEC {
            context.clear_event()?;
        }
        if context.phase()? == MissilePhase::Attached {
            context.think(self)?;
            return Ok(());
        }
        let pass = context
            .runtime
            .capture_save_state()
            .into_iter()
            .find(|entry| entry.entity == context.slot)
            .and_then(|entry| entry.pass)
            .map(ItemsActorId);
        let origin = context.origin()?;
        let time = context.time();
        let destination = evaluate_trajectory(&context.state.trajectory, time);
        let mut trace = context.trace(origin, destination, pass)?;
        if trace.solidity != TraceSolidity::Clear {
            trace = context.trace(origin, origin, pass)?;
            trace.fraction = 0.0;
        } else {
            let time = context.time();
            let delta = evaluate_trajectory_delta(&context.state.trajectory, time);
            context.move_body(trace.end, delta)?;
        }
        context.link()?;
        if trace.fraction != 1.0 {
            if trace.surface_flags & HITSCAN_SURF_NODAMAGE != 0 {
                if context.has_special()? {
                    context.no_impact()?;
                }
                if context.live()? {
                    context.release()?;
                }
                return Ok(());
            }
            self.impact(context, &trace)?;
            if !context.live()? || context.phase()? != MissilePhase::Flight {
                return Ok(());
            }
        }
        if context.has_special()? {
            context.after_move()?;
        }
        if !context.live()? {
            return Ok(());
        }
        context.think(self)?;
        Ok(())
    }
}

/// Weapon missile launcher over runtime links.
struct RuntimeMissileLauncher {
    links: Rc<RefCell<CoreLinks>>,
}

impl MissileLauncher for RuntimeMissileLauncher {
    fn fire_plasma(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize {
        self.fire_directed(driver, entity, muzzle, direction, MissileWeapon::Plasma)
    }

    fn fire_grenade(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize {
        self.fire_directed(driver, entity, muzzle, direction, MissileWeapon::Grenade)
    }

    fn fire_rocket(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize {
        self.fire_directed(driver, entity, muzzle, direction, MissileWeapon::Rocket)
    }

    fn fire_bfg(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize {
        self.fire_directed(driver, entity, muzzle, direction, MissileWeapon::Bfg)
    }

    fn fire_grapple(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize {
        self.fire_directed(driver, entity, muzzle, direction, MissileWeapon::Grapple)
    }

    fn fire_nail(
        &mut self,
        _driver: &mut dyn Q3Driver,
        entity: usize,
        muzzle: Vec3,
        forward: Vec3,
        right: Vec3,
        up: Vec3,
    ) -> usize {
        let (missiles, items_pool, missile_host) = self.handles();
        let mut projectile = RuntimeProjectileDriver;
        let mut launcher = missiles.borrow_mut();
        let mut pool = items_pool.borrow_mut();
        let mut host = missile_host.borrow_mut();
        launcher
            .fire_nail(
                &mut pool,
                &mut *host,
                &mut projectile,
                entity,
                muzzle,
                forward,
                right,
                up,
            )
            .expect("fire nail")
    }

    fn fire_prox(&mut self, driver: &mut dyn Q3Driver, entity: usize, muzzle: Vec3, direction: Vec3) -> usize {
        self.fire_directed(driver, entity, muzzle, direction, MissileWeapon::Prox)
    }
}

/// Launcher weapon selector.
#[derive(Clone, Copy)]
enum MissileWeapon {
    Plasma,
    Grenade,
    Rocket,
    Bfg,
    Grapple,
    Prox,
}

/// Shared missile-launcher handles.
type MissileLauncherHandles = (
    Rc<RefCell<MissileRuntime>>,
    Rc<RefCell<ItemsEntityPool>>,
    Rc<RefCell<RuntimeMissileHost>>,
);

impl RuntimeMissileLauncher {
    fn handles(&self) -> MissileLauncherHandles {
        let links = self.links.borrow();
        (
            links.missiles.clone().expect("missiles"),
            links.items_pool.clone().expect("items pool"),
            links.missile_host.clone().expect("missile host"),
        )
    }

    fn fire_directed(
        &mut self,
        _driver: &mut dyn Q3Driver,
        entity: usize,
        muzzle: Vec3,
        direction: Vec3,
        weapon: MissileWeapon,
    ) -> usize {
        let (missiles, items_pool, missile_host) = self.handles();
        let mut fired_direction = MissileDirection {
            x: direction.x,
            y: direction.y,
            z: direction.z,
        };
        let mut projectile = RuntimeProjectileDriver;
        let mut missiles = missiles.borrow_mut();
        let mut items_pool = items_pool.borrow_mut();
        let mut missile_host = missile_host.borrow_mut();
        let bolt = match weapon {
            MissileWeapon::Plasma => missiles.fire_plasma(
                &mut items_pool,
                &mut *missile_host,
                &mut projectile,
                entity,
                muzzle,
                &mut fired_direction,
            ),
            MissileWeapon::Grenade => missiles.fire_grenade(
                &mut items_pool,
                &mut *missile_host,
                &mut projectile,
                entity,
                muzzle,
                &mut fired_direction,
            ),
            MissileWeapon::Rocket => missiles.fire_rocket(
                &mut items_pool,
                &mut *missile_host,
                &mut projectile,
                entity,
                muzzle,
                &mut fired_direction,
            ),
            MissileWeapon::Bfg => missiles.fire_bfg(
                &mut items_pool,
                &mut *missile_host,
                &mut projectile,
                entity,
                muzzle,
                &mut fired_direction,
            ),
            MissileWeapon::Grapple => missiles.fire_grapple(
                &mut items_pool,
                &mut *missile_host,
                &mut projectile,
                entity,
                muzzle,
                &mut fired_direction,
            ),
            MissileWeapon::Prox => missiles.fire_prox(
                &mut items_pool,
                &mut *missile_host,
                &mut projectile,
                entity,
                muzzle,
                &mut fired_direction,
            ),
        };
        bolt.expect("fire missile")
    }
}

// ---------------------------------------------------------------------------
// Hitscan.
// ---------------------------------------------------------------------------

/// Trace contact normal for hitscan mirrors.
fn hitscan_trace_normal(trace: &WorldActorTraceResult) -> Vec3 {
    match &trace.contact {
        BaseTraceContact::Plane { plane } => plane.normal,
        BaseTraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Quad-scaled damage for hitscan mirrors.
fn hitscan_scaled_damage(amount: f32, attack: &Q3BulletAttack) -> i32 {
    qvm_float_to_int(amount * attack.quad)
}

/// Reflected ray endpoint for hitscan mirrors.
fn hitscan_reflected_end(start: Vec3, point: Vec3, direction: Vec3) -> Vec3 {
    let incoming = sub3(point, start);
    add3(
        point,
        scale3(
            normalize3(add3(incoming, scale3(direction, -2.0 * dot3(incoming, direction)))),
            8192.0,
        ),
    )
}

/// Bullet loop over a bullet host, mirroring `q3_bullet_fire`.
#[allow(clippy::too_many_arguments)]
fn run_bullet_fire(
    driver: &mut dyn Q3Driver,
    host: &mut dyn BulletHost,
    random: &mut dyn SimRandom,
    missionpack: bool,
    shooter: &ActorId,
    attack: &mut Q3BulletAttack,
    spread: f32,
    amount: f32,
) {
    let mut end = q3_bullet_endpoint(
        &BallisticAttack {
            muzzle: attack.muzzle,
            forward: attack.forward,
            right: attack.right,
            up: attack.up,
        },
        spread,
        random,
    );
    let mut pass: Option<ActorId> = Some(shooter.clone());
    for _ in 0..10 {
        let trace = host.trace_hit(driver, attack.muzzle, end, pass.as_ref());
        if (trace.surface_flags & HITSCAN_SURF_NODAMAGE) != 0 {
            return;
        }
        if !matches!(trace.hit, ActorTraceHit::None) {
            host.impact(driver, trace.end);
        }
        let actor = match &trace.hit {
            ActorTraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        let target = actor.as_ref().and_then(|actor| host.hit_target(driver, actor));
        let point = snap_vector_towards(trace.end, attack.muzzle);
        let flesh = target.is_some_and(|record| record.damageable && record.player);
        host.emit_hit(
            driver,
            &BulletEmitEvent {
                point,
                normal: hitscan_trace_normal(&trace),
                target: actor.clone(),
                flesh,
            },
        );
        if flesh && target.is_some_and(|record| record.accuracy_eligible) {
            host.credit_accuracy(driver);
        }
        if let Some(actor) = actor {
            if target.is_some_and(|record| record.damageable) {
                let record = target.unwrap_or(Q3BulletTarget {
                    damageable: true,
                    player: false,
                    accuracy_eligible: false,
                    invulnerable: false,
                });
                if missionpack && record.player && record.invulnerable {
                    match host.invulnerability_impact(driver, &actor, attack.forward, point) {
                        WeaponImpact::Hit {
                            impact_point,
                            bounce_direction,
                        } => {
                            end = hitscan_reflected_end(attack.muzzle, impact_point, bounce_direction);
                            attack.muzzle = impact_point;
                            pass = None;
                        }
                        WeaponImpact::Miss => {
                            attack.muzzle = point;
                            pass = Some(actor);
                        }
                    }
                    continue;
                }
                host.apply_damage(
                    driver,
                    &actor,
                    attack.forward,
                    point,
                    hitscan_scaled_damage(amount, attack),
                );
            }
        }
        break;
    }
}

/// Gauntlet loop over a contact host, mirroring `q3_gauntlet_attack`.
fn run_gauntlet_attack(
    driver: &mut dyn Q3Driver,
    host: &mut dyn ContactHost,
    shooter: &ActorId,
    attack: &Q3BulletAttack,
    quad_active: bool,
) -> bool {
    let trace = host.trace_hit(
        driver,
        attack.muzzle,
        add3(attack.muzzle, scale3(attack.forward, 32.0)),
        Some(shooter),
    );
    if (trace.surface_flags & HITSCAN_SURF_NODAMAGE) != 0 {
        return false;
    }
    let ActorTraceHit::Actor { actor } = &trace.hit else {
        return false;
    };
    let target = host.hit_target(driver, actor);
    if !target.is_some_and(|record| record.damageable) {
        return false;
    }
    host.impact(driver, trace.end);
    if target.is_some_and(|record| record.player) {
        host.emit_contact(
            driver,
            &Q3ContactEvent::Hit {
                point: trace.end,
                normal: hitscan_trace_normal(&trace),
                target: actor.clone(),
            },
        );
    }
    if quad_active {
        host.emit_contact(driver, &Q3ContactEvent::GauntletQuad);
    }
    host.apply_damage(
        driver,
        actor,
        attack.forward,
        trace.end,
        hitscan_scaled_damage(50.0, attack),
    );
    true
}

/// Lightning loop over a contact host, mirroring `q3_lightning_fire`.
fn run_lightning_fire(
    driver: &mut dyn Q3Driver,
    host: &mut dyn ContactHost,
    missionpack: bool,
    shooter: &ActorId,
    attack: &mut Q3BulletAttack,
) {
    let mut pass: Option<ActorId> = Some(shooter.clone());
    for count in 0..10 {
        let trace = host.trace_hit(
            driver,
            attack.muzzle,
            add3(attack.muzzle, scale3(attack.forward, 768.0)),
            pass.as_ref(),
        );
        if missionpack && count != 0 {
            host.emit_contact(
                driver,
                &Q3ContactEvent::LightningReflection {
                    start: attack.muzzle,
                    end: snap_vector(trace.end),
                },
            );
        }
        if matches!(trace.hit, ActorTraceHit::None) {
            return;
        }
        if (trace.surface_flags & HITSCAN_SURF_NODAMAGE) == 0 {
            host.impact(driver, trace.end);
        }
        let actor = match &trace.hit {
            ActorTraceHit::Actor { actor } => Some(actor.clone()),
            _ => None,
        };
        let target = actor.as_ref().and_then(|actor| host.hit_target(driver, actor));
        if let Some(actor) = &actor {
            if target.is_some_and(|record| record.damageable) {
                let record = target.unwrap_or(Q3BulletTarget {
                    damageable: true,
                    player: false,
                    accuracy_eligible: false,
                    invulnerable: false,
                });
                if missionpack && record.player && record.invulnerable {
                    match host.invulnerability_impact(driver, actor, attack.forward, trace.end) {
                        WeaponImpact::Hit {
                            impact_point,
                            bounce_direction,
                        } => {
                            let end = hitscan_reflected_end(attack.muzzle, impact_point, bounce_direction);
                            attack.muzzle = impact_point;
                            attack.forward = normalize3(sub3(end, impact_point));
                            pass = None;
                        }
                        WeaponImpact::Miss => {
                            attack.muzzle = trace.end;
                            pass = Some(actor.clone());
                        }
                    }
                    continue;
                }
                host.apply_damage(
                    driver,
                    actor,
                    attack.forward,
                    trace.end,
                    hitscan_scaled_damage(8.0, attack),
                );
            }
        }
        let after = actor.as_ref().and_then(|actor| host.hit_target(driver, actor));
        if actor
            .as_ref()
            .is_some_and(|_| after.is_some_and(|record| record.damageable && record.player))
        {
            let actor = actor.unwrap_or_else(|| shooter.clone());
            host.emit_contact(
                driver,
                &Q3ContactEvent::Hit {
                    point: trace.end,
                    normal: hitscan_trace_normal(&trace),
                    target: actor,
                },
            );
            if after.is_some_and(|record| record.accuracy_eligible) {
                host.credit_accuracy(driver);
            }
        } else if (trace.surface_flags & HITSCAN_SURF_NODAMAGE) == 0 {
            host.emit_contact(
                driver,
                &Q3ContactEvent::Miss {
                    point: trace.end,
                    normal: hitscan_trace_normal(&trace),
                },
            );
        }
        break;
    }
}

/// One shotgun pellet over a shotgun host, mirroring `shotgun_pellet`.
fn run_shotgun_pellet(
    driver: &mut dyn Q3Driver,
    host: &mut dyn ShotgunHost,
    missionpack: bool,
    shooter: &ActorId,
    attack: &Q3BulletAttack,
    start: Vec3,
    end: Vec3,
) -> bool {
    let mut start = start;
    let mut end = end;
    let mut pass: Option<ActorId> = Some(shooter.clone());
    for _ in 0..10 {
        let trace = host.trace_hit(driver, start, end, pass.as_ref());
        if (trace.surface_flags & HITSCAN_SURF_NODAMAGE) != 0 {
            return false;
        }
        if !matches!(trace.hit, ActorTraceHit::None) {
            host.impact(driver, trace.end);
        }
        let ActorTraceHit::Actor { actor } = &trace.hit else {
            return false;
        };
        let target = host.hit_target(driver, actor);
        if !target.is_some_and(|record| record.damageable) {
            return false;
        }
        let record = target.unwrap_or(Q3BulletTarget {
            damageable: true,
            player: false,
            accuracy_eligible: false,
            invulnerable: false,
        });
        if missionpack && record.player && record.invulnerable {
            match host.invulnerability_impact(driver, actor, attack.forward, trace.end) {
                WeaponImpact::Hit {
                    impact_point,
                    bounce_direction,
                } => {
                    end = hitscan_reflected_end(start, impact_point, bounce_direction);
                    start = impact_point;
                    pass = None;
                }
                WeaponImpact::Miss => {
                    start = trace.end;
                    pass = Some(actor.clone());
                }
            }
            continue;
        }
        host.apply_damage(
            driver,
            actor,
            attack.forward,
            trace.end,
            hitscan_scaled_damage(10.0, attack),
        );
        return host
            .hit_target(driver, actor)
            .is_some_and(|record| record.accuracy_eligible);
    }
    false
}

/// Shotgun loop over a shotgun host, mirroring `q3_shotgun_fire`.
fn run_shotgun_fire(
    driver: &mut dyn Q3Driver,
    host: &mut dyn ShotgunHost,
    random: &mut dyn SimRandom,
    missionpack: bool,
    shooter: &ActorId,
    attack: &Q3BulletAttack,
) {
    let muzzle = attack.muzzle;
    let direction = snap_vector(scale3(attack.forward, 4096.0));
    let event = host.begin_shotgun(driver, muzzle, direction);
    let seed = random.rand_value() & 255;
    host.emit_shotgun_seed(driver, event, seed);
    let mut hit_client = false;
    for end in q3_shotgun_endpoints(muzzle, direction, seed) {
        if !driver.actor_live(shooter) {
            break;
        }
        if run_shotgun_pellet(driver, host, missionpack, shooter, attack, muzzle, end) && !hit_client {
            hit_client = true;
            host.credit_accuracy(driver);
        }
    }
}

/// Rail trail emitter over a rail host.
fn emit_rail_trail(
    driver: &mut dyn Q3Driver,
    host: &mut dyn RailHost,
    attack: &Q3BulletAttack,
    point: Vec3,
    normal: Option<Vec3>,
) {
    host.emit_trail(
        driver,
        &RailShot {
            start: add3(add3(attack.muzzle, scale3(attack.right, 4.0)), scale3(attack.up, -1.0)),
            end: point,
            impact_normal: normal,
        },
    );
}

/// Rail damage through the driver combat context.
fn rail_damage(
    driver: &mut dyn Q3Driver,
    shooter: &ActorId,
    actor: &ActorId,
    direction: Vec3,
    point: Vec3,
    amount: i32,
) {
    let target = driver.participant(actor);
    let attacker = driver.participant(shooter);
    let mut direction = direction;
    driver.combat().damage(
        &target,
        Some(&attacker),
        Some(&attacker),
        Some(&mut direction),
        Some(point),
        amount,
        0,
        10,
    );
}

/// Rail loop over a rail host, mirroring `q3_rail_fire`.
fn run_rail_fire(
    driver: &mut dyn Q3Driver,
    host: &mut dyn RailHost,
    missionpack: bool,
    shooter: &ActorId,
    attack: &mut Q3BulletAttack,
) -> i32 {
    let mut end = add3(attack.muzzle, scale3(attack.forward, 8192.0));
    let mut pass: Option<ActorId> = Some(shooter.clone());
    let mut hits = 0;
    let mut penetrated = 0;
    let mut restores: Vec<ActorId> = Vec::new();
    let mut trace: Option<WorldActorTraceResult> = None;
    loop {
        if !host.is_alive(driver) {
            break;
        }
        let current = host.trace_hit(driver, attack.muzzle, end, pass.as_ref());
        if !matches!(current.hit, ActorTraceHit::None) && (current.surface_flags & HITSCAN_SURF_NODAMAGE) == 0 {
            host.impact(driver, current.end);
        }
        let ActorTraceHit::Actor { actor } = &current.hit else {
            trace = Some(current);
            break;
        };
        let actor = actor.clone();
        let target = host.hit_target(driver, &actor);
        let mut recorded = false;
        if target.is_some_and(|record| record.damageable) {
            let record = target.unwrap_or(Q3BulletTarget {
                damageable: true,
                player: false,
                accuracy_eligible: false,
                invulnerable: false,
            });
            if missionpack && record.player && record.invulnerable {
                match host.invulnerability_impact(driver, &actor, attack.forward, current.end) {
                    WeaponImpact::Hit {
                        impact_point,
                        bounce_direction,
                    } => {
                        end = hitscan_reflected_end(attack.muzzle, impact_point, bounce_direction);
                        let snapped = snap_vector_towards(current.end, attack.muzzle);
                        trace = Some(WorldActorTraceResult {
                            end: snapped,
                            ..current.clone()
                        });
                        emit_rail_trail(driver, host, attack, snapped, None);
                        attack.muzzle = impact_point;
                        pass = None;
                        recorded = true;
                    }
                    WeaponImpact::Miss => {}
                }
            } else {
                if target.is_some_and(|record| record.accuracy_eligible) {
                    hits += 1;
                }
                rail_damage(
                    driver,
                    shooter,
                    &actor,
                    attack.forward,
                    current.end,
                    hitscan_scaled_damage(100.0, attack),
                );
            }
        }
        if (current.contents & HITSCAN_CONTENTS_SOLID) != 0 {
            trace = Some(current);
            break;
        }
        if !recorded {
            trace = Some(current.clone());
        }
        if let Some(restore) = host.unlink_actor(driver, &actor) {
            restores.push(restore);
        }
        penetrated += 1;
        if penetrated >= 4 {
            break;
        }
    }
    for restore in restores {
        host.restore_actor(driver, &restore);
    }
    if let Some(trace) = trace {
        let normal = if (trace.surface_flags & HITSCAN_SURF_NODAMAGE) != 0 {
            None
        } else {
            Some(hitscan_trace_normal(&trace))
        };
        emit_rail_trail(
            driver,
            host,
            attack,
            snap_vector_towards(trace.end, attack.muzzle),
            normal,
        );
    }
    hits
}

/// Rail statistics over a statistics record, reusing `q3_rail_statistics`.
fn run_rail_statistics(current: &RailStatistics, hits: i32, time: i32) -> RailStatisticsOutcome {
    let updated = q3_rail_statistics(
        &Q3RailStatistics {
            streak: current.streak,
            hits: current.hits,
            impressive_count: current.impressive_count,
            reward_until: current.reward_until,
        },
        hits,
        time,
    );
    RailStatisticsOutcome {
        streak: updated.statistics.streak,
        hits: updated.statistics.hits,
        impressive_count: updated.statistics.impressive_count,
        reward_until: updated.statistics.reward_until,
        awarded: updated.awarded,
    }
}

// ---------------------------------------------------------------------------
// Driver.
// ---------------------------------------------------------------------------

/// Deferred driver links filled once the runtime exists.
#[derive(Default)]
struct DriverLinks {
    host: Option<Rc<dyn Q3SourceHost>>,
    level: Option<Rc<RefCell<GameLevel>>>,
    settings: Option<Rc<Q3GameSettings>>,
    config: Option<Rc<RefCell<RuntimeConfigRegistry>>>,
    records: Option<Q3EntityRecords>,
    base_pool: Option<Rc<RefCell<BaseEntityPool>>>,
    items_pool: Option<Rc<RefCell<ItemsEntityPool>>>,
    team_pool: Option<PoolRef>,
    combat: Option<Rc<RefCell<CombatContext>>>,
    remaps: Option<Rc<RefCell<ShaderRemapRegistry>>>,
    missiles: Option<Rc<RefCell<MissileRuntime>>>,
    team: Option<Rc<TeamRuntime>>,
    death: Option<Rc<DeathRuntime>>,
    product: Option<Product>,
    random: Option<Rc<RefCell<GameRandomMirror>>>,
    world: Option<Rc<RuntimeQ3World>>,
    lifecycle: Option<Rc<RefCell<ItemLifecycleContext>>>,
    item_table: Option<Rc<EntitiesItemTable>>,
    drop_host: Option<Rc<dyn DropHost>>,
    team_scores: Option<SharedSlots>,
    missile_host: Option<Rc<RefCell<RuntimeMissileHost>>>,
    core: Option<Rc<RefCell<CoreLinks>>>,
    map_travel: bool,
}

/// Base-module driver: owns the state pool, shares everything else.
struct RuntimeDriver {
    state_pool: Option<RuntimeStatePool>,
    scratch: GameUtilityScratch,
    links: Rc<RefCell<DriverLinks>>,
}

impl RuntimeDriver {
    fn driver_product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    fn new(product: Product, max_clients: usize, print: Rc<dyn Fn(&str)>) -> Self {
        Self {
            state_pool: Some(RuntimeStatePool::new(product, max_clients)),
            scratch: GameUtilityScratch::new(print),
            links: Rc::new(RefCell::new(DriverLinks::default())),
        }
    }

    /// Borrow the state pool.
    fn state(&self) -> &RuntimeStatePool {
        self.state_pool.as_ref().expect("state pool")
    }

    /// Mutably borrow the state pool.
    fn state_mut(&mut self) -> &mut RuntimeStatePool {
        self.state_pool.as_mut().expect("state pool")
    }

    /// Current think callback for a state slot, if any.
    fn state_touch(&self, slot: usize) -> Option<StateEntityTouch> {
        self.state().entities.get(slot).and_then(|entity| entity.touch.clone())
    }
}

impl StateServerWorldOps for RuntimeDriver {
    fn link(&mut self, slot: usize) {
        let (world, records) = {
            let links = self.links.borrow();
            (
                links.world.clone().expect("world"),
                links.records.clone().expect("records"),
            )
        };
        if let Some(native) = records.get(slot) {
            world.adapter.link(native);
        }
        if let Some(entity) = self.state_mut().entities.get_mut(slot) {
            entity.r.linked = true;
        }
    }

    fn unlink(&mut self, slot: usize) {
        let world = self.links.borrow().world.clone().expect("world");
        world.adapter.unlink(slot as i32);
        if let Some(entity) = self.state_mut().entities.get_mut(slot) {
            entity.r.linked = false;
        }
    }

    fn link_state(&self, slot: usize) -> Option<LinkState> {
        self.links
            .borrow()
            .world
            .clone()
            .expect("world")
            .adapter
            .link_state(slot as i32)
    }

    fn area_entities(&self, bounds: &Bounds, maximum: usize) -> Vec<usize> {
        self.links
            .borrow()
            .world
            .clone()
            .expect("world")
            .adapter
            .area_entities(*bounds, maximum)
            .into_iter()
            .map(|number| number as usize)
            .collect()
    }
}

impl StateSpatialQueries for RuntimeDriver {
    fn area_actors(&self, bounds: &Bounds, maximum: usize) -> Vec<ActorId> {
        self.links
            .borrow()
            .world
            .clone()
            .expect("world")
            .adapter
            .area_actors(*bounds, maximum)
    }

    fn trace_actor(&self, query: &ActorTraceQuery) -> ActorTraceResult {
        self.links
            .borrow()
            .world
            .clone()
            .expect("world")
            .adapter
            .trace_actor(query)
    }
}

impl StateCombatContext for RuntimeDriver {
    fn product(&self) -> Product {
        self.links.borrow().product.expect("product")
    }

    fn game_type(&self) -> i32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .integer("g_gametype")
    }

    fn time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.time
    }

    #[allow(clippy::too_many_arguments)]
    fn damage(
        &mut self,
        target: &StateParticipant,
        inflictor: Option<&StateParticipant>,
        attacker: Option<&StateParticipant>,
        direction: Option<&mut Vec3>,
        point: Option<Vec3>,
        amount: i32,
        flags: i32,
        method: i32,
    ) {
        let links = self.links.borrow();
        let pool = links.base_pool.clone().expect("base pool");
        let combat = links.combat.clone().expect("combat");
        let level = links.level.clone().expect("level");
        let settings = links.settings.clone().expect("settings");
        {
            let borrowed_level = level.borrow();
            let mut context = combat.borrow_mut();
            context.time = borrowed_level.base.time;
            context.intermission_queued = borrowed_level.base.intermission_queued;
            context.game_type = settings.integer("g_gametype");
            context.knockback = settings.number("g_knockback");
        }
        let borrowed = pool.borrow();
        let convert = |participant: &StateParticipant| -> EntitiesDamageParticipant {
            match participant {
                StateParticipant::Entity(slot) => EntitiesDamageParticipant::Native(borrowed.at(*slot as i32)),
                StateParticipant::SharedActor(actor) => EntitiesDamageParticipant::Shared(SharedActor {
                    actor: actor.clone(),
                    origin: None,
                }),
            }
        };
        let entities_target = convert(target);
        let entities_inflictor = inflictor.map(&convert);
        let entities_attacker = attacker.map(&convert);
        drop(borrowed);
        combat_damage(
            &combat.borrow(),
            entities_target,
            entities_inflictor,
            entities_attacker,
            direction,
            point,
            amount as f32,
            flags,
            method,
            None,
        );
    }

    fn actor_combat_state(&self, actor: &ActorId) -> Option<StateActorCombatState> {
        let links = self.links.borrow();
        let combat = links.combat.clone().expect("combat");
        let state = (combat.borrow().authority.read)(actor)?;
        let team = Q3Driver::native_slot(self, actor)
            .and_then(|slot| self.state().entities.get(slot))
            .and_then(|entity| entity.client)
            .and_then(|client| self.state().clients.get(client))
            .map(|client| client.sess.session_team);
        Some(StateActorCombatState {
            can_take_damage: state.can_take_damage,
            health: state.health,
            team,
        })
    }
}

impl MoverActorAccess for RuntimeDriver {
    fn native_slot(&self, actor: &ActorId) -> Option<usize> {
        self.links
            .borrow()
            .records
            .clone()
            .expect("records")
            .native_by_actor(Some(actor))
            .map(|native| native.borrow().slot)
    }

    fn participant(&self, actor: &ActorId) -> StateParticipant {
        match MoverActorAccess::native_slot(self, actor) {
            Some(slot) => StateParticipant::Entity(slot),
            None => StateParticipant::SharedActor(actor.clone()),
        }
    }

    fn observe(&self, actor: &ActorId) -> Option<SharedMoverBody> {
        let links = self.links.borrow();
        let host = links.host.clone().expect("host");
        let body = host.bodies().read(actor)?;
        let entity = MoverActorAccess::native_slot(self, actor);
        Some(SharedMoverBody {
            actor: actor.clone(),
            kind: if host.is_player(actor) {
                SharedBodyKind::Player
            } else {
                SharedBodyKind::Movable
            },
            state: Q3BodyState {
                origin: body.origin,
                angles: body.angles,
                velocity: body.velocity,
                bounds: body.bounds,
                ground: body.ground.clone(),
            },
            absolute_bounds: Bounds {
                min: Vec3 {
                    x: body.bounds.min.x + body.origin.x,
                    y: body.bounds.min.y + body.origin.y,
                    z: body.bounds.min.z + body.origin.z,
                },
                max: Vec3 {
                    x: body.bounds.max.x + body.origin.x,
                    y: body.bounds.max.y + body.origin.y,
                    z: body.bounds.max.z + body.origin.z,
                },
            },
            clip_mask: entity
                .and_then(|slot| self.state().entities.get(slot))
                .map(|entity| entity.clipmask)
                .unwrap_or(0),
        })
    }

    fn write(&mut self, actor: &ActorId, origin: Vec3, ground: Option<ActorId>) {
        let host = self.links.borrow().host.clone().expect("host");
        let bodies = host.bodies();
        let Some(owned) = host.actors().resolve_owned(actor) else {
            return;
        };
        let Some(mut body) = bodies.read(actor) else {
            return;
        };
        body.origin = origin;
        body.ground = ground;
        bodies.write(&owned, body);
    }

    fn link_actor(&mut self, actor: &ActorId) {
        let links = self.links.borrow();
        let world = links.world.clone().expect("world");
        let records = links.records.clone().expect("records");
        if let Some(native) = records.native_by_actor(Some(actor)) {
            world.adapter.link(native);
        }
    }

    fn release(&mut self, actor: &ActorId) {
        let Some(slot) = MoverActorAccess::native_slot(self, actor) else {
            return;
        };
        self.links
            .borrow()
            .world
            .clone()
            .expect("world")
            .adapter
            .unlink(slot as i32);
        if let Some(entity) = self.state_mut().entities.get_mut(slot) {
            entity.r.linked = false;
        }
    }
}

impl Q3Driver for RuntimeDriver {
    fn pool(&mut self) -> &mut dyn StateEntityPool {
        self.state_mut()
    }

    fn world(&mut self) -> &mut dyn StateServerWorldOps {
        self
    }

    fn spatial(&mut self) -> &mut dyn StateSpatialQueries {
        self
    }

    fn combat(&mut self) -> &mut dyn StateCombatContext {
        self
    }

    fn scratch(&mut self) -> &mut GameUtilityScratch {
        &mut self.scratch
    }

    fn mover_actors(&mut self) -> &mut dyn MoverActorAccess {
        self
    }

    fn warn(&mut self, message: &str) {
        self.links.borrow().host.clone().expect("host").engine().print(message);
    }

    fn log(&mut self, message: &str) {
        self.links.borrow().host.clone().expect("host").engine().log(message);
    }

    fn sound_index(&mut self, path: &str) -> i32 {
        let config = self.links.borrow().config.clone().expect("config");
        let index = config.borrow_mut().sound_index(Some(path)).expect("sound index") as i32;
        index
    }

    fn model_index(&mut self, name: Option<&str>) -> i32 {
        let config = self.links.borrow().config.clone().expect("config");
        let index = config.borrow_mut().model_index(name).expect("model index") as i32;
        index
    }

    fn gravity(&self) -> f32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .number("g_gravity")
    }

    fn game_rand(&mut self) -> i32 {
        self.links
            .borrow()
            .random
            .clone()
            .expect("random")
            .borrow_mut()
            .rand_value()
    }

    fn game_random(&mut self) -> f32 {
        self.links
            .borrow()
            .random
            .clone()
            .expect("random")
            .borrow_mut()
            .random_value()
    }

    fn game_crandom(&mut self) -> f32 {
        self.links
            .borrow()
            .random
            .clone()
            .expect("random")
            .borrow_mut()
            .crandom_value()
    }

    fn remap_shader(&mut self, old: &str, new: &str, time_seconds: f32) {
        let (host, remaps) = {
            let links = self.links.borrow();
            (links.host.clone().expect("host"), links.remaps.clone().expect("remaps"))
        };
        remaps
            .borrow_mut()
            .add(old, new, time_seconds as f64)
            .expect("remap shader");
        let shader_state = remaps.borrow().build_shader_state_config().expect("shader remap");
        host.configstrings().borrow_mut().set(24, &shader_state);
    }

    fn set_configstring(&mut self, index: i32, value: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .configstrings()
            .borrow_mut()
            .set(index as usize, value);
    }

    fn set_cvar(&mut self, name: &str, value: &str) {
        let _ = self
            .links
            .borrow()
            .host
            .clone()
            .expect("host")
            .cvars()
            .borrow_mut()
            .set(name, value, true);
    }

    fn send_server_command(&mut self, client: i32, command: &str) {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .engine()
            .send_server_command(client, command);
    }

    fn use_targets(&mut self, slot: usize, activator: Option<StateParticipant>) {
        base_use_targets(self, slot, activator).expect("driver use targets");
    }

    fn adjust_area_portal(&mut self, slot: usize, open: bool) {
        let host = self.links.borrow().host.clone().expect("host");
        let actor = self.state().entities.get(slot).map(|entity| entity.actor.clone());
        let Some(actor) = actor else {
            return;
        };
        let bounds = host.bodies().linked(&actor).map(|linked| linked.absolute_bounds);
        let Some(bounds) = bounds else {
            panic!("Area portal mover must be linked");
        };
        let mut areas = host
            .scene()
            .box_leaves(bounds, 128)
            .into_iter()
            .map(|leaf| host.scene().leaf_area(leaf))
            .collect::<Vec<_>>();
        areas.sort_unstable();
        areas.dedup();
        if areas.len() >= 2 {
            host.scene().adjust_area_portal_state(areas[0], areas[1], open);
        }
    }

    fn return_dropped_flag(&mut self, slot: usize) {
        let links = self.links.borrow();
        let team = links.team.clone().expect("team");
        let team_pool = links.team_pool.clone().expect("team pool");
        team.free_entity(&team_pool.at(slot));
    }

    fn return_flag(&mut self, team: Team) {
        self.links.borrow().team.clone().expect("team").return_flag(team as i32);
    }

    fn add_score(&mut self, player: usize, origin: Vec3, points: i32) {
        let (death, pool, scores) = {
            let links = self.links.borrow();
            (
                links.death.clone().expect("death"),
                links.base_pool.clone().expect("base pool"),
                links.team_scores.clone().expect("team scores"),
            )
        };
        death.add_score(&pool.borrow().at(player as i32), origin, points);
        sync_death_scores_to_shared(&death, &scores);
    }

    fn explode_missile(&mut self, slot: usize) {
        let (missiles, items_pool, missile_host) = {
            let links = self.links.borrow();
            (
                links.missiles.clone().expect("missiles"),
                links.items_pool.clone().expect("items pool"),
                links.missile_host.clone().expect("missile host"),
            )
        };
        let mut projectile = RuntimeProjectileDriver;
        missiles
            .borrow_mut()
            .explode(
                &mut items_pool.borrow_mut(),
                &mut *missile_host.borrow_mut(),
                &mut projectile,
                slot,
            )
            .expect("explode missile");
    }

    fn teleport_player(&mut self, player: usize, origin: Vec3, angles: Vec3) {
        let (items_pool, core) = {
            let links = self.links.borrow();
            (
                links.items_pool.clone().expect("items pool"),
                links.core.clone().expect("core"),
            )
        };
        let mut combat = RuntimeCombatOps { links: core.clone() };
        let mut world = RuntimeWorldOps { links: core };
        let mut context = TeleportContext {
            combat: &mut combat,
            world: &mut world,
        };
        misc_teleport_player(&mut items_pool.borrow_mut(), &mut context, player, origin, angles)
            .expect("driver teleport");
    }

    fn map_travel_mode(&self) -> bool {
        self.links.borrow().map_travel
    }

    fn map_travel_teleport(&mut self, player: usize, origin: Vec3, angles: Vec3) {
        self.teleport_player(player, origin, angles);
    }

    fn map_travel_drop_flag(&mut self, player: usize) {
        let powerup = {
            let state = self.state();
            let Some(entity) = state.entities.get(player) else {
                return;
            };
            let Some(index) = entity.client else {
                panic!("Portal touch requires a client entity");
            };
            let client = &state.clients[index];
            if client.ps.powerups.get(Powerup::PwNeutralflag as usize) != 0 {
                Powerup::PwNeutralflag
            } else if client.ps.powerups.get(Powerup::PwRedflag as usize) != 0 {
                Powerup::PwRedflag
            } else if client.ps.powerups.get(Powerup::PwBlueflag as usize) != 0 {
                Powerup::PwBlueflag
            } else {
                return;
            }
        };
        let item = self.find_item_for_powerup(powerup as i32).expect("portal carried flag");
        self.drop_item(player, item, 0);
        let state = self.state_mut();
        let index = state
            .entities
            .get(player)
            .and_then(|entity| entity.client)
            .expect("portal client");
        state.clients[index].ps.powerups.set(powerup as usize, 0);
    }

    fn touch_item(&mut self, item: usize, player: usize, contact: &StateTouchContact) {
        let (pool, lifecycle) = {
            let links = self.links.borrow();
            (
                links.base_pool.clone().expect("base pool"),
                links.lifecycle.clone().expect("lifecycle"),
            )
        };
        let base = pool.borrow().at(item as i32);
        let participant = EntitiesDamageParticipant::Native(pool.borrow().at(player as i32));
        let mirror = TouchContactMirror {
            this_actor: base.borrow().actor.clone(),
            other: contact.other.clone(),
            plane: contact.plane,
            surface: None,
        };
        lifecycle_touch_item(&base, participant, &mirror, &lifecycle.borrow());
    }

    fn drop_item(&mut self, entity: usize, item: usize, angle: i32) -> usize {
        let (drop_host, team_pool, item_table, pool) = {
            let links = self.links.borrow();
            (
                links.drop_host.clone().expect("drop host"),
                links.team_pool.clone().expect("team pool"),
                links.item_table.clone().expect("item table"),
                links.base_pool.clone().expect("base pool"),
            )
        };
        let Some(definition) = item_table.items.get(item).cloned() else {
            panic!("Drop item {item} is outside the item table");
        };
        let dropped = drop_host
            .drop_item(&team_pool.at(entity), &definition, angle)
            .borrow()
            .slot;
        sync_team_to_entities(&team_pool, &pool.borrow(), dropped);
        sync_entities_to_state(&pool.borrow(), self.state_mut(), dropped);
        dropped
    }

    fn dropped_flag_think(&mut self, slot: usize) {
        let links = self.links.borrow();
        let team = links.team.clone().expect("team");
        let team_pool = links.team_pool.clone().expect("team pool");
        team.dropped_flag_think(&team_pool.at(slot));
    }

    fn check_dropped_team_item(&mut self, slot: usize) {
        let links = self.links.borrow();
        let team = links.team.clone().expect("team");
        let team_pool = links.team_pool.clone().expect("team pool");
        team.check_dropped_item(&team_pool.at(slot));
    }

    fn actor_live(&self, actor: &ActorId) -> bool {
        self.links.borrow().host.clone().expect("host").actors().is_live(actor)
    }

    fn actor_origin(&self, actor: &ActorId) -> Option<Vec3> {
        self.links
            .borrow()
            .host
            .clone()
            .expect("host")
            .bodies()
            .read(actor)
            .map(|body| body.origin)
    }

    fn participant(&self, actor: &ActorId) -> StateParticipant {
        MoverActorAccess::participant(self, actor)
    }

    fn actor_is_player(&self, actor: &ActorId) -> bool {
        self.links.borrow().host.clone().expect("host").is_player(actor)
    }

    fn native_slot(&self, actor: &ActorId) -> Option<usize> {
        MoverActorAccess::native_slot(self, actor)
    }

    fn actor_event(&mut self, actor: &ActorId, event: EntityEvent, parameter: i32) {
        let (host, level) = {
            let links = self.links.borrow();
            (links.host.clone().expect("host"), links.level.clone().expect("level"))
        };
        let origin = host
            .bodies()
            .read(actor)
            .map(|body| body.origin)
            .unwrap_or(vec3(0.0, 0.0, 0.0));
        let state = EntityState {
            event: event as i32,
            event_parm: parameter,
            ..EntityState::default()
        };
        host.entity_event(Q3SourceEntityEvent {
            actor: actor.clone(),
            state,
            origin,
            time: level.borrow().base.time,
        });
    }

    fn item_count(&self) -> usize {
        self.links.borrow().item_table.clone().expect("item table").items.len()
    }

    fn item_at(&self, index: usize) -> Option<EntitiesItemDefinition> {
        let table = self.links.borrow().item_table.clone().expect("item table");
        table.items.get(index).cloned()
    }

    fn find_item(&self, pickup_name: &str) -> Option<usize> {
        let table = self.links.borrow().item_table.clone().expect("item table");
        table.find_item(pickup_name).and_then(|item| table.index_of(item))
    }

    fn find_item_for_powerup(&self, powerup: i32) -> Option<usize> {
        let table = self.links.borrow().item_table.clone().expect("item table");
        table
            .find_item_for_powerup(powerup)
            .and_then(|item| table.index_of(item))
    }

    fn set_brush_model(&mut self, slot: usize, model: Option<&str>) {
        let Some(name) = model else {
            panic!("SV_SetBrushModel: None is not a brush model");
        };
        if !name.starts_with('*') {
            panic!("SV_SetBrushModel: {name} is not a brush model");
        };
        let index = game_atoi(&name[1..]).expect("brush model index");
        let host = self.links.borrow().host.clone().expect("host");
        let bounds = host.scene().model_bounds(index);
        {
            let state = self.state_mut();
            let Some(entity) = state.entities.get_mut(slot) else {
                return;
            };
            entity.s.modelindex = index;
            entity.r.mins = bounds.min;
            entity.r.maxs = bounds.max;
            entity.r.model = EntityCollisionModel::Inline { index };
            entity.r.contents = -1;
        }
        StateServerWorldOps::link(self, slot);
    }

    fn bullet_fire(
        &mut self,
        host: &mut dyn BulletHost,
        shooter: &ActorId,
        attack: &mut Q3BulletAttack,
        spread: i32,
        amount: i32,
    ) {
        let missionpack = self.driver_product() == Product::Missionpack;
        let random = self.links.borrow().random.clone().expect("random");
        let mut random = random.borrow_mut();
        run_bullet_fire(
            self,
            host,
            &mut *random,
            missionpack,
            shooter,
            attack,
            spread as f32,
            amount as f32,
        );
    }

    fn gauntlet_attack(
        &mut self,
        host: &mut dyn ContactHost,
        shooter: &ActorId,
        attack: &mut Q3BulletAttack,
        has_quad: bool,
    ) -> bool {
        run_gauntlet_attack(self, host, shooter, attack, has_quad)
    }

    fn lightning_fire(&mut self, host: &mut dyn ContactHost, shooter: &ActorId, attack: &mut Q3BulletAttack) {
        let missionpack = self.driver_product() == Product::Missionpack;
        run_lightning_fire(self, host, missionpack, shooter, attack);
    }

    fn shotgun_fire(&mut self, host: &mut dyn ShotgunHost, shooter: &ActorId, attack: &mut Q3BulletAttack) {
        let missionpack = self.driver_product() == Product::Missionpack;
        let random = self.links.borrow().random.clone().expect("random");
        let mut random = random.borrow_mut();
        run_shotgun_fire(self, host, &mut *random, missionpack, shooter, attack);
    }

    fn rail_fire(&mut self, host: &mut dyn RailHost, shooter: &ActorId, attack: &mut Q3BulletAttack) -> i32 {
        let missionpack = self.driver_product() == Product::Missionpack;
        run_rail_fire(self, host, missionpack, shooter, attack)
    }

    fn rail_statistics(&mut self, current: &RailStatistics, hits: i32, time: i32) -> RailStatisticsOutcome {
        run_rail_statistics(current, hits, time)
    }
}

impl Q3SourceRuntime {
    /// Assemble a source runtime over engine-owned services.
    pub fn new(options: Q3SourceOptions, host: Rc<dyn Q3SourceHost>, mode: Q3SourceConstruction) -> Self {
        let fresh = matches!(mode, Q3SourceConstruction::New);
        if fresh {
            if let Some(carry) = &options.session_carry {
                host.cvars()
                    .borrow_mut()
                    .set("session", &carry.world, true)
                    .expect("session carry");
                for client in &carry.clients {
                    if client.slot < 0 || client.slot as usize >= options.max_clients {
                        continue;
                    }
                    host.cvars()
                        .borrow_mut()
                        .set(&format!("session{}", client.slot), &client.session, true)
                        .expect("client carry");
                    host.engine().set_userinfo(client.slot, &client.userinfo);
                }
            }
        }
        let engine = host.engine();
        let print_engine = engine.clone();
        let print: Rc<dyn Fn(String)> = Rc::new(move |text| print_engine.print(&text));
        let remap_engine = engine.clone();
        let remaps = Rc::new(RefCell::new(ShaderRemapRegistry::new(Rc::new(move |text: &str| {
            remap_engine.print(text);
        }))));
        let level = Rc::new(RefCell::new(GameLevel::new()));
        let settings_host = Rc::new(RuntimeSettingsHost {
            host: host.clone(),
            level: level.clone(),
            remaps: remaps.clone(),
            product: options.product,
        });
        let settings = Rc::new(Q3GameSettings::new(settings_host, options.product));
        if fresh {
            host.cvars()
                .borrow_mut()
                .set("sv_maxclients", &options.max_clients.to_string(), true)
                .expect("max clients");
            settings.register(&options.build_date);
        } else {
            let Q3SourceConstruction::Restore(snapshot) = &mode else {
                unreachable!("non-fresh construction without a snapshot");
            };
            let content = native_save_content(options.product, &options.entities, snapshot);
            let reader = ContentSaveReader::new(&content);
            let server_value = reader.field("server").value.expect("native server");
            host.server_state()
                .restore_save_state(&engine_json_from_content(server_value))
                .expect("restore server");
            settings
                .restore_save_state(reader.field("settings").value.expect("native settings"))
                .expect("restore settings");
        }
        let base_random = Rc::new(RefCell::new(GameRandomMirror::new(options.seed)));
        let team_random = Rc::new(TeamGameRandom::new(options.seed));
        let record_links = Rc::new(RefCell::new(RecordLinks::default()));
        record_links.borrow_mut().host = Some(host.clone());
        record_links.borrow_mut().level = Some(level.clone());
        let record_host = Rc::new(RuntimeRecordHost {
            links: record_links.clone(),
        });
        let records = Q3EntityRecords::new(
            record_host,
            options.recipe.map.entities.provider.clone(),
            options.product,
        );
        record_links.borrow_mut().records = Some(records.clone());
        let adapter_host = Rc::new(RuntimeAdapterHost {
            scene: host.scene(),
            cvars: host.cvars(),
        });
        let world = Rc::new(RuntimeQ3World {
            adapter: Q3WorldAdapter::new(adapter_host, records.clone()),
            base_pool: RefCell::new(None),
            records: RefCell::new(Some(records.clone())),
        });
        let time_level = level.clone();
        let link_world = world.clone();
        let link_records = records.clone();
        let unlink_world = world.clone();
        let base_pool = Rc::new(RefCell::new(BaseEntityPool::open(EntityPoolOptions {
            product: options.product,
            max_clients: options.max_clients,
            map_start_time: 0,
            time: Rc::new(move || time_level.borrow().base.time),
            print: print.clone(),
            link: Rc::new(move |entity| {
                link_world
                    .adapter
                    .link(link_records.get(entity.borrow().slot).expect("link slot"));
            }),
            unlink: Rc::new(move |entity| unlink_world.adapter.unlink(entity.borrow().slot as i32)),
            event_debug: None,
        })));
        world.base_pool.borrow_mut().replace(base_pool.clone());
        let memory = Rc::new(RefCell::new(GameMemory::new(
            Box::new({
                let settings = settings.clone();
                move || settings.integer("g_debugAlloc")
            }),
            Box::new({
                let print = print.clone();
                move |text| print(text)
            }),
        )));
        let config = Rc::new(RefCell::new(ConfigStringRegistry::new(SharedConfigStore {
            store: host.configstrings(),
        })));
        Self::finish_construction(
            options,
            host,
            mode,
            fresh,
            remaps,
            level,
            settings,
            base_random,
            team_random,
            records,
            world,
            base_pool,
            memory,
            config,
            print,
            record_links,
        )
    }

    /// Build the combat, item, and pool runtimes, then assemble the shared core.
    #[allow(clippy::too_many_arguments)]
    fn finish_construction(
        options: Q3SourceOptions,
        host: Rc<dyn Q3SourceHost>,
        mode: Q3SourceConstruction,
        fresh: bool,
        remaps: Rc<RefCell<ShaderRemapRegistry>>,
        level: Rc<RefCell<GameLevel>>,
        settings: Rc<Q3GameSettings>,
        base_random: Rc<RefCell<GameRandomMirror>>,
        team_random: Rc<TeamGameRandom>,
        records: Q3EntityRecords,
        world: Rc<RuntimeQ3World>,
        base_pool: Rc<RefCell<BaseEntityPool>>,
        memory: Rc<RefCell<GameMemory>>,
        config: Rc<RefCell<RuntimeConfigRegistry>>,
        print: Rc<dyn Fn(String)>,
        record_links: Rc<RefCell<RecordLinks>>,
    ) -> Self {
        let item_table = Rc::new(EntitiesItemTable::new(
            item_list(options.product)
                .iter()
                .map(|entry| EntitiesItemDefinition {
                    class_name: entry.class_name.map(str::to_string),
                    pickup_name: entry.pickup_name.map(str::to_string),
                    quantity: entry.quantity,
                    item_type: entry.item_type(),
                    tag: entry.tag(),
                })
                .collect(),
        ));
        let registry = Rc::new(RefCell::new(ItemRegistry::new(options.product, item_table.clone())));
        let core_links = Rc::new(RefCell::new(CoreLinks::default()));
        {
            let mut links = core_links.borrow_mut();
            links.host = Some(host.clone());
            links.product = Some(options.product);
            links.base_pool = Some(base_pool.clone());
            links.records = Some(records.clone());
            links.world = Some(world.clone());
            links.weapon_provider = Some(options.weapon_provider.provider.clone());
            links.combat_provider = Some(options.recipe.combat.provider.clone());
            links.inventory_provider = Some(options.recipe.inventory.provider.clone());
            links.movement_provider = Some(options.recipe.movement.provider.clone());
            links.level = Some(level.clone());
            links.settings = Some(settings.clone());
            links.config = Some(config.clone());
            links.item_table = Some(item_table.clone());
            links.random = Some(base_random.clone());
        }
        let bridge_host = Rc::new(RuntimeBridgeHost {
            core: core_links.clone(),
        });
        let bridge = Rc::new(Q3CombatBridge::new(bridge_host));
        record_links.borrow_mut().bridge = Some(bridge.clone());
        let game_combat = build_game_combat_context(
            &host,
            &bridge,
            &records,
            &base_pool,
            &world,
            &level,
            &settings,
            &item_table,
            options.product,
        );
        core_links.borrow_mut().combat = Some(Rc::new(RefCell::new(game_combat)));
        let missiles = Rc::new(RefCell::new(MissileRuntime::new()));
        let launcher = RuntimeMissileLauncher {
            links: core_links.clone(),
        };
        let weapons = Rc::new(RefCell::new(WeaponRuntime::new(
            1.0,
            Box::new(launcher),
            Rc::new(move |driver, actor| {
                let slot = driver.native_slot(actor)?;
                let linked = driver.world().link_state(slot).is_some_and(|state| state.linked);
                if !linked {
                    return None;
                }
                driver.world().unlink(slot);
                Some(actor.clone())
            }),
            Rc::new(move |driver, actor| {
                if let Some(slot) = driver.native_slot(actor) {
                    driver.world().link(slot);
                }
            }),
        )));
        core_links.borrow_mut().missiles = Some(missiles.clone());
        core_links.borrow_mut().weapons = Some(weapons.clone());
        let hook_world = world.clone();
        let print_hook = print.clone();
        let team_pool = Rc::new(SupportEntityPool::new(
            options.product,
            options.max_clients,
            PoolHooks {
                time: Rc::new({
                    let level = level.clone();
                    move || level.borrow().base.time
                }),
                map_start_time: 0,
                link: Rc::new({
                    let world = hook_world.clone();
                    move |entity| SupportQ3World::link(world.as_ref(), entity)
                }),
                unlink: Rc::new({
                    let world = hook_world.clone();
                    move |entity| SupportQ3World::unlink(world.as_ref(), entity.borrow().slot as i32)
                }),
                print: Rc::new(move |message: &str| print_hook(message.to_owned())),
            },
            Box::new(RuntimeRankings {
                pool: base_pool.clone(),
            }),
        ));
        core_links.borrow_mut().team_pool = Some(team_pool.clone());
        let items_pool = Rc::new(RefCell::new(ItemsEntityPool::new(options.product)));
        core_links.borrow_mut().items_pool = Some(items_pool.clone());
        Self::finish_teams(
            options,
            host,
            mode,
            fresh,
            remaps,
            level,
            settings,
            base_random,
            team_random,
            records,
            world,
            base_pool,
            memory,
            config,
            print,
            item_table,
            registry,
            core_links,
            bridge,
            missiles,
            weapons,
            team_pool,
            items_pool,
        )
    }

    /// Build the lifecycle context, driver, and team runtimes, then finish assembly.
    #[allow(clippy::too_many_arguments)]
    fn finish_teams(
        options: Q3SourceOptions,
        host: Rc<dyn Q3SourceHost>,
        mode: Q3SourceConstruction,
        fresh: bool,
        remaps: Rc<RefCell<ShaderRemapRegistry>>,
        level: Rc<RefCell<GameLevel>>,
        settings: Rc<Q3GameSettings>,
        base_random: Rc<RefCell<GameRandomMirror>>,
        team_random: Rc<TeamGameRandom>,
        records: Q3EntityRecords,
        world: Rc<RuntimeQ3World>,
        base_pool: Rc<RefCell<BaseEntityPool>>,
        memory: Rc<RefCell<GameMemory>>,
        config: Rc<RefCell<RuntimeConfigRegistry>>,
        print: Rc<dyn Fn(String)>,
        item_table: Rc<EntitiesItemTable>,
        registry: Rc<RefCell<ItemRegistry>>,
        core_links: Rc<RefCell<CoreLinks>>,
        bridge: Rc<Q3CombatBridge>,
        missiles: Rc<RefCell<MissileRuntime>>,
        weapons: Rc<RefCell<WeaponRuntime>>,
        team_pool: PoolRef,
        items_pool: Rc<RefCell<ItemsEntityPool>>,
    ) -> Self {
        let team_cell: Rc<RefCell<Option<Rc<TeamRuntime>>>> = Rc::new(RefCell::new(None));
        let driver_cell: Rc<RefCell<Option<Rc<RefCell<RuntimeDriver>>>>> = Rc::new(RefCell::new(None));
        let lifecycle: Rc<RefCell<ItemLifecycleContext>> = Rc::new(RefCell::new(ItemLifecycleContext {
            original_pickups: host.original_pickups(),
            callbacks: None,
            preview_pickup: Some(Rc::new({
                let host = host.clone();
                move |item| host.preview_pickup(item)
            })),
            admit_pickup: Some(Rc::new({
                let host = host.clone();
                move |item| host.admit_pickup(item)
            })),
            entities: base_pool.clone(),
            world: server_world_mirror(&world, &records),
            product: options.product,
            game_type: settings.integer("g_gametype"),
            weapon_respawn_seconds: settings.integer("g_weaponrespawn"),
            team_weapon_respawn_seconds: settings.integer("g_weaponTeamRespawn"),
            handicap_for_client: Rc::new({
                let engine = host.engine();
                move |client| client_info_value(&engine.get_userinfo(client), "handicap")
            }),
            team_pickup: Rc::new({
                let team_cell = team_cell.clone();
                let team_pool = team_pool.clone();
                move |item, player| {
                    let team = team_cell.borrow().clone().expect("team runtime");
                    let item_slot = item.borrow().slot;
                    let player_slot = player.borrow().slot;
                    team.pickup_team(&team_pool.at(item_slot), &team_pool.at(player_slot))
                }
            }),
            use_targets: Rc::new({
                let driver_cell = driver_cell.clone();
                move |item, player| {
                    let driver = driver_cell.borrow().clone().expect("use-targets driver");
                    let item_slot = item.borrow().slot;
                    let player_slot = player.borrow().slot;
                    driver
                        .borrow_mut()
                        .use_targets(item_slot, Some(StateParticipant::Entity(player_slot)));
                }
            }),
            sound_index: Rc::new({
                let config = config.clone();
                move |path| config.borrow_mut().sound_index(Some(path)).expect("lifecycle sound") as i32
            }),
            random: base_random.clone(),
            registry: registry.clone(),
            log: Rc::new({
                let engine = host.engine();
                move |text| engine.log(&text)
            }),
            warn: print.clone(),
            item_table: item_table.clone(),
            pickup_item: Rc::new({
                let base_pool = base_pool.clone();
                let items_pool = items_pool.clone();
                let pickup_table = Rc::new(RuntimeItemsTable {
                    table: item_table.clone(),
                });
                let product = options.product;
                move |item, player, mirror| {
                    let item_slot = item.borrow().slot;
                    let player_slot = player.borrow().slot;
                    let borrowed = base_pool.borrow();
                    let mut items = items_pool.borrow_mut();
                    sync_entities_to_items(&borrowed, &mut items, product, item_slot);
                    sync_entities_to_items(&borrowed, &mut items, product, player_slot);
                    for (index, client) in mirror.clients.iter().enumerate() {
                        if let Some(entity) = items.get_mut(index) {
                            if let Some(target) = entity.client.as_mut() {
                                sync_entities_client_to_items(client, target);
                            }
                        }
                    }
                    drop(borrowed);
                    let trace = mirror.trace_solid_line.clone();
                    let mut trace = move |from: Vec3, to: Vec3| PowerupSightTrace {
                        fraction: trace(from, to),
                    };
                    let handicap = mirror.handicap_for_client.clone();
                    let mut handicap = move |client: i32| handicap(client);
                    let mut context = ItemPickupContext {
                        game_type: mirror.game_type,
                        weapon_respawn_seconds: mirror.weapon_respawn_seconds,
                        team_weapon_respawn_seconds: mirror.team_weapon_respawn_seconds,
                        time: mirror.time,
                        trace_solid_line: &mut trace,
                        handicap_for_client: &mut handicap,
                    };
                    content_pickup_item(&mut items, &*pickup_table, item_slot, player_slot, &mut context)
                        .expect("pickup item")
                }
            }),
        }));
        register_lifecycle_callbacks(&base_pool, &records, &lifecycle);
        bind_item_save_callbacks(&lifecycle.borrow());
        Self::finish_drivers(
            options,
            host,
            mode,
            fresh,
            remaps,
            level,
            settings,
            base_random,
            team_random,
            records,
            world,
            base_pool,
            memory,
            config,
            print,
            item_table,
            registry,
            core_links,
            bridge,
            missiles,
            weapons,
            team_pool,
            items_pool,
            lifecycle,
            team_cell,
            driver_cell,
        )
    }

    /// Build the driver, missile host, team, death, think, and spawn runtimes.
    #[allow(clippy::too_many_arguments)]
    fn finish_drivers(
        options: Q3SourceOptions,
        host: Rc<dyn Q3SourceHost>,
        mode: Q3SourceConstruction,
        fresh: bool,
        remaps: Rc<RefCell<ShaderRemapRegistry>>,
        level: Rc<RefCell<GameLevel>>,
        settings: Rc<Q3GameSettings>,
        base_random: Rc<RefCell<GameRandomMirror>>,
        team_random: Rc<TeamGameRandom>,
        records: Q3EntityRecords,
        world: Rc<RuntimeQ3World>,
        base_pool: Rc<RefCell<BaseEntityPool>>,
        memory: Rc<RefCell<GameMemory>>,
        config: Rc<RefCell<RuntimeConfigRegistry>>,
        print: Rc<dyn Fn(String)>,
        item_table: Rc<EntitiesItemTable>,
        registry: Rc<RefCell<ItemRegistry>>,
        core_links: Rc<RefCell<CoreLinks>>,
        bridge: Rc<Q3CombatBridge>,
        missiles: Rc<RefCell<MissileRuntime>>,
        weapons: Rc<RefCell<WeaponRuntime>>,
        team_pool: PoolRef,
        items_pool: Rc<RefCell<ItemsEntityPool>>,
        lifecycle: Rc<RefCell<ItemLifecycleContext>>,
        team_cell: Rc<RefCell<Option<Rc<TeamRuntime>>>>,
        driver_cell: Rc<RefCell<Option<Rc<RefCell<RuntimeDriver>>>>>,
    ) -> Self {
        let driver_print = print.clone();
        let driver = Rc::new(RefCell::new(RuntimeDriver::new(
            options.product,
            options.max_clients,
            Rc::new(move |text: &str| driver_print(text.to_string())),
        )));
        {
            let borrowed_driver = driver.borrow();
            let mut links = borrowed_driver.links.borrow_mut();
            links.host = Some(host.clone());
            links.level = Some(level.clone());
            links.settings = Some(settings.clone());
            links.config = Some(config.clone());
            links.records = Some(records.clone());
            links.base_pool = Some(base_pool.clone());
            links.items_pool = Some(items_pool.clone());
            links.team_pool = Some(team_pool.clone());
            links.combat = core_links.borrow().combat.clone();
            links.remaps = Some(remaps.clone());
            links.missiles = Some(missiles.clone());
            links.product = Some(options.product);
            links.random = Some(base_random.clone());
            links.world = Some(world.clone());
            links.lifecycle = Some(lifecycle.clone());
            links.item_table = Some(item_table.clone());
            links.core = Some(core_links.clone());
        }
        driver_cell.borrow_mut().replace(driver.clone());
        core_links.borrow_mut().driver = Some(driver.clone());
        let missile_host = Rc::new(RefCell::new(RuntimeMissileHost {
            links: core_links.clone(),
            combat: RuntimeCombatOps {
                links: core_links.clone(),
            },
            world: RuntimeWorldOps {
                links: core_links.clone(),
            },
            bodies: BodyTable::default(),
            random: MissileRandomAdapter {
                random: base_random.clone(),
            },
            behavior: options.weapon_behavior.clone(),
        }));
        core_links.borrow_mut().missile_host = Some(missile_host);
        driver.borrow().links.borrow_mut().missile_host = core_links.borrow().missile_host.clone();
        let team_scores: SharedSlots = Rc::new(SlotArray::new(4));
        let team_state: MatchStateRef = Rc::new(RefCell::new(MatchState::default()));
        let locations = Rc::new(RefCell::new(TargetLocationState::new()));
        let team_links = Rc::new(RefCell::new(TeamLinks::default()));
        {
            let mut links = team_links.borrow_mut();
            links.product = Some(options.product);
            links.team_pool = Some(team_pool.clone());
            links.world = Some(world.clone());
            links.level = Some(level.clone());
            links.team_scores = Some(team_scores.clone());
            links.settings = Some(settings.clone());
            links.host = Some(host.clone());
            links.locations = Some(locations.clone());
            links.base_pool = Some(base_pool.clone());
            links.lifecycle = Some(lifecycle.clone());
        }
        let team = Rc::new(TeamRuntime::new(Rc::new(RuntimeTeamHost {
            links: team_links.clone(),
        })));
        team_cell.borrow_mut().replace(team.clone());
        core_links.borrow_mut().team = Some(team.clone());
        driver.borrow().links.borrow_mut().team = Some(team.clone());
        let match_cell: Rc<RefCell<Option<Rc<MatchRuntime>>>> = Rc::new(RefCell::new(None));
        let commands_cell: Rc<RefCell<Option<Rc<GameCommandRuntime>>>> = Rc::new(RefCell::new(None));
        let death = build_death(
            &options,
            &host,
            &level,
            &settings,
            &base_pool,
            &team_pool,
            &items_pool,
            &base_random,
            &item_table,
            &lifecycle,
            &team,
            &driver,
            &world,
            &records,
            &missiles,
            &weapons,
            &core_links,
            &match_cell,
            &commands_cell,
        );
        core_links.borrow_mut().death = Some(death.clone());
        team_links.borrow_mut().death = Some(death.clone());
        driver.borrow().links.borrow_mut().death = Some(death.clone());
        driver.borrow().links.borrow_mut().team_scores = Some(team_scores.clone());
        driver.borrow().links.borrow_mut().drop_host = Some(Rc::new(RuntimeDropHost {
            links: core_links.clone(),
        }));
        Self::finish_think(
            options,
            host,
            mode,
            fresh,
            remaps,
            level,
            settings,
            base_random,
            team_random,
            records,
            world,
            base_pool,
            memory,
            config,
            item_table,
            registry,
            core_links,
            bridge,
            missiles,
            weapons,
            team_pool,
            items_pool,
            lifecycle,
            team_state,
            team_scores,
            locations,
            team,
            death,
            driver,
            match_cell,
            commands_cell,
            team_links,
        )
    }

    /// Build the think, effects, spawn, and mover-spawn runtimes.
    #[allow(clippy::too_many_arguments)]
    fn finish_think(
        options: Q3SourceOptions,
        host: Rc<dyn Q3SourceHost>,
        mode: Q3SourceConstruction,
        fresh: bool,
        remaps: Rc<RefCell<ShaderRemapRegistry>>,
        level: Rc<RefCell<GameLevel>>,
        settings: Rc<Q3GameSettings>,
        base_random: Rc<RefCell<GameRandomMirror>>,
        team_random: Rc<TeamGameRandom>,
        records: Q3EntityRecords,
        world: Rc<RuntimeQ3World>,
        base_pool: Rc<RefCell<BaseEntityPool>>,
        memory: Rc<RefCell<GameMemory>>,
        config: Rc<RefCell<RuntimeConfigRegistry>>,
        item_table: Rc<EntitiesItemTable>,
        registry: Rc<RefCell<ItemRegistry>>,
        core_links: Rc<RefCell<CoreLinks>>,
        bridge: Rc<Q3CombatBridge>,
        missiles: Rc<RefCell<MissileRuntime>>,
        weapons: Rc<RefCell<WeaponRuntime>>,
        team_pool: PoolRef,
        items_pool: Rc<RefCell<ItemsEntityPool>>,
        lifecycle: Rc<RefCell<ItemLifecycleContext>>,
        team_state: MatchStateRef,
        team_scores: SharedSlots,
        locations: Rc<RefCell<TargetLocationState>>,
        team: Rc<TeamRuntime>,
        death: Rc<DeathRuntime>,
        driver: Rc<RefCell<RuntimeDriver>>,
        match_cell: Rc<RefCell<Option<Rc<MatchRuntime>>>>,
        commands_cell: Rc<RefCell<Option<Rc<GameCommandRuntime>>>>,
        team_links: Rc<RefCell<TeamLinks>>,
    ) -> Self {
        let think_links = Rc::new(RefCell::new(ThinkLinks::default()));
        let item_host: Rc<dyn ItemHost> = Rc::new(RuntimeItemHost {
            links: core_links.clone(),
            lifecycle: Rc::new(RefCell::new(Some(lifecycle.borrow().clone()))),
        });
        let policy = Rc::new(RuntimePolicyHost {
            links: think_links.clone(),
        });
        let effects_links = Rc::new(RefCell::new(EffectsLinks::default()));
        {
            let mut links = effects_links.borrow_mut();
            links.level = Some(level.clone());
            links.settings = Some(settings.clone());
            links.config = Some(config.clone());
            links.combat_handle = Some(Rc::new(RuntimeCombat {
                links: core_links.clone(),
            }));
            links.items = Some(item_host.clone());
            links.team_random = Some(team_random.clone());
            links.base_pool = Some(base_pool.clone());
            links.team_pool = Some(team_pool.clone());
            links.items_pool = Some(items_pool.clone());
            links.policy = Some(policy.clone());
            links.core = Some(core_links.clone());
        }
        let effects_host = Rc::new(RuntimeEffectsHost {
            links: effects_links.clone(),
        });
        let effects_ref: EffectsRef = effects_host.clone();
        core_links.borrow_mut().effects = Some(effects_ref);
        {
            let mut links = think_links.borrow_mut();
            links.team_pool = Some(team_pool.clone());
            links.world = Some(world.clone());
            links.effects = Some(effects_host.clone());
            links.items = Some(item_host.clone());
            links.level = Some(level.clone());
            links.settings = Some(settings.clone());
            links.host = Some(host.clone());
            links.records = Some(records.clone());
            links.base_pool = Some(base_pool.clone());
            links.items_pool = Some(items_pool.clone());
            links.missiles = Some(missiles.clone());
            links.weapons = Some(weapons.clone());
            links.combat = core_links.borrow().combat.clone();
            links.product = Some(options.product);
            links.driver = Some(driver.clone());
            links.policy = Some(policy.clone());
            links.lifecycle = Some(lifecycle.clone());
            links.item_table = Some(item_table.clone());
            links.weapon_host = Some(Rc::new(RuntimeWeaponHost {
                links: core_links.clone(),
            }));
            links.drop_host = Some(Rc::new(RuntimeDropHost {
                links: core_links.clone(),
            }));
            links.teleport_host = Some(Rc::new(RuntimeTeleportHost {
                links: core_links.clone(),
            }));
            links.combat_handle = Some(Rc::new(RuntimeCombat {
                links: core_links.clone(),
            }));
            links.portal_host = Some(Rc::new(RuntimePortalHost {
                links: core_links.clone(),
            }));
            links.core = Some(core_links.clone());
        }
        let think = Rc::new(ClientThinkRuntime::new(Rc::new(RuntimeThinkHost {
            think_links: think_links.clone(),
        })));
        think_links.borrow_mut().think = Some(think.clone());
        let mover_spawn_links = Rc::new(RefCell::new(MoverSpawnLinks::default()));
        {
            let mut links = mover_spawn_links.borrow_mut();
            links.host = Some(host.clone());
            links.level = Some(level.clone());
            links.settings = Some(settings.clone());
            links.remaps = Some(remaps.clone());
            links.config = Some(config.clone());
            links.core = Some(core_links.clone());
            links.driver = Some(driver.clone());
            links.items_pool = Some(items_pool.clone());
        }
        let mover_spawns = Rc::new(RefCell::new(RuntimeMoverSpawnHost {
            links: mover_spawn_links.clone(),
            mover_core: RuntimeMoverCore {
                links: mover_spawn_links.clone(),
            },
            combat: RuntimeCombatOps {
                links: core_links.clone(),
            },
            world: RuntimeWorldOps {
                links: core_links.clone(),
            },
        }));
        think_links.borrow_mut().mover_spawns = Some(mover_spawns.clone());
        let targets_host: Rc<dyn TargetsHost> = Rc::new(RuntimeTargetsHost {
            links: core_links.clone(),
        });
        let spawn_links = Rc::new(RefCell::new(SpawnLinks::default()));
        {
            let mut links = spawn_links.borrow_mut();
            links.team_pool = Some(team_pool.clone());
            links.world = Some(world.clone());
            links.host = Some(host.clone());
            links.records = Some(records.clone());
            links.base_pool = Some(base_pool.clone());
            links.items_pool = Some(items_pool.clone());
            links.think = Some(think.clone());
            links.level = Some(level.clone());
            links.settings = Some(settings.clone());
            links.death = Some(death.clone());
            links.core = Some(core_links.clone());
            links.effects_host = Some(effects_host.clone());
            links.targets_host = Some(targets_host.clone());
            links.policy = Some(policy.clone());
        }
        let client_spawns = Rc::new(RefCell::new(ClientSpawnState::default()));
        let spawns = Rc::new(ClientSpawnRuntime::new(
            Rc::new(RuntimeSpawnHost {
                links: spawn_links.clone(),
                team_random: team_random.clone(),
            }),
            client_spawns.borrow().clone(),
        ));
        think_links.borrow_mut().spawns = Some(spawns.clone());
        think_links.borrow_mut().spawn_selector = Some(Rc::new(RuntimeSpawnSelector { spawns: spawns.clone() }));
        Self::finish_runtime(
            options,
            host,
            mode,
            fresh,
            remaps,
            level,
            settings,
            base_random,
            team_random,
            records,
            world,
            base_pool,
            memory,
            config,
            registry,
            core_links,
            bridge,
            missiles,
            weapons,
            team_pool,
            items_pool,
            lifecycle,
            team_state,
            team_scores,
            locations,
            team,
            death,
            match_cell,
            commands_cell,
            think,
            spawns,
            mover_spawns,
            think_links,
            spawn_links,
            team_links,
            mover_spawn_links,
        )
    }

    /// Build the session, match, arena, admission, and command runtimes, then assemble.
    #[allow(clippy::too_many_arguments)]
    fn finish_runtime(
        options: Q3SourceOptions,
        host: Rc<dyn Q3SourceHost>,
        mode: Q3SourceConstruction,
        fresh: bool,
        remaps: Rc<RefCell<ShaderRemapRegistry>>,
        level: Rc<RefCell<GameLevel>>,
        settings: Rc<Q3GameSettings>,
        base_random: Rc<RefCell<GameRandomMirror>>,
        team_random: Rc<TeamGameRandom>,
        records: Q3EntityRecords,
        world: Rc<RuntimeQ3World>,
        base_pool: Rc<RefCell<BaseEntityPool>>,
        memory: Rc<RefCell<GameMemory>>,
        config: Rc<RefCell<RuntimeConfigRegistry>>,
        registry: Rc<RefCell<ItemRegistry>>,
        core_links: Rc<RefCell<CoreLinks>>,
        bridge: Rc<Q3CombatBridge>,
        missiles: Rc<RefCell<MissileRuntime>>,
        weapons: Rc<RefCell<WeaponRuntime>>,
        team_pool: PoolRef,
        items_pool: Rc<RefCell<ItemsEntityPool>>,
        lifecycle: Rc<RefCell<ItemLifecycleContext>>,
        team_state: MatchStateRef,
        team_scores: SharedSlots,
        locations: Rc<RefCell<TargetLocationState>>,
        team: Rc<TeamRuntime>,
        death: Rc<DeathRuntime>,
        match_cell: Rc<RefCell<Option<Rc<MatchRuntime>>>>,
        commands_cell: Rc<RefCell<Option<Rc<GameCommandRuntime>>>>,
        think: Rc<ClientThinkRuntime>,
        spawns: Rc<ClientSpawnRuntime>,
        mover_spawns: Rc<RefCell<RuntimeMoverSpawnHost>>,
        think_links: Rc<RefCell<ThinkLinks>>,
        spawn_links: Rc<RefCell<SpawnLinks>>,
        team_links: Rc<RefCell<TeamLinks>>,
        mover_spawn_links: Rc<RefCell<MoverSpawnLinks>>,
    ) -> Self {
        let session_world = Rc::new(SessionWorld {
            clients: team_pool.clients().to_vec(),
            max_clients: options.max_clients,
            team_scores: team_scores.clone(),
            game_type: Cell::new(0),
            team_auto_join: Cell::new(false),
            max_game_clients: Cell::new(0),
            time: Cell::new(0),
            num_non_spectator_clients: Cell::new(0),
            new_session: Cell::new(false),
        });
        Self::refresh_session_world(&session_world, &level, &settings);
        let session_links = Rc::new(RefCell::new(SessionLinks::default()));
        session_links.borrow_mut().host = Some(host.clone());
        let session = Rc::new(GameSessionManager::new(
            session_world.clone(),
            Rc::new(RuntimeSessionServices {
                links: session_links.clone(),
            }),
            Rc::new(RuntimeSessionCvars { cvars: host.cvars() }),
        ));
        let match_links = Rc::new(RefCell::new(MatchLinks::default()));
        {
            let mut links = match_links.borrow_mut();
            links.product = Some(options.product);
            links.team_state = Some(team_state.clone());
            links.team_pool = Some(team_pool.clone());
            links.team_scores = Some(team_scores.clone());
            links.level = Some(level.clone());
            links.team_random = Some(TeamGameRandom::new(options.seed));
            links.spawns = Some(spawns.clone());
            links.settings = Some(settings.clone());
            links.host = Some(host.clone());
            links.session = Some(session.clone());
            links.session_world = Some(session_world.clone());
        }
        let arena_match: Rc<RefCell<Option<Rc<MatchRuntime>>>> = Rc::new(RefCell::new(None));
        let arenas = Rc::new(ArenaRuntime::new(Rc::new(RuntimeArenaHost {
            match_runtime: arena_match.clone(),
            world: world.clone(),
            cvars: Rc::new(RuntimeArenaCvars { cvars: host.cvars() }),
            config: Rc::new(RuntimeArenaConfig {
                store: host.configstrings(),
            }),
        })));
        match_links.borrow_mut().arenas = Some(arenas.clone());
        let game_match = Rc::new(MatchRuntime::new(
            Rc::new(RuntimeMatchHost {
                links: match_links.clone(),
                team_state: team_state.clone(),
                team_random: team_random.clone(),
            }),
            MatchModuleState::new(),
        ));
        arena_match.borrow_mut().replace(game_match.clone());
        match_cell.borrow_mut().replace(game_match.clone());
        team_links.borrow_mut().match_runtime = Some(game_match.clone());
        spawn_links.borrow_mut().match_runtime = Some(game_match.clone());
        let movers = Rc::new(RefCell::new(MoverRuntime::new(0)));
        mover_spawn_links.borrow_mut().movers = Some(movers.clone());
        let portal = if options.product == Product::Missionpack {
            Some(Rc::new(RefCell::new(PersonalPortalRuntime::new())))
        } else {
            None
        };
        core_links.borrow_mut().portal = Some(portal.clone());
        let death_links = Rc::new(RefCell::new(DeathLinks::default()));
        {
            let mut links = death_links.borrow_mut();
            links.base_pool = Some(base_pool.clone());
            links.death = Some(death.clone());
            links.team_pool = Some(team_pool.clone());
            links.items_pool = Some(items_pool.clone());
            links.core = Some(core_links.clone());
        }
        let death_host = Rc::new(RuntimeSupportDeathHost {
            links: death_links.clone(),
        });
        let admission_links = Rc::new(RefCell::new(AdmissionLinks::default()));
        let admission_commands = Rc::new(RuntimeAdmissionCommands {
            commands: Rc::new(RefCell::new(None)),
        });
        {
            let mut links = admission_links.borrow_mut();
            links.product = Some(options.product);
            links.team_pool = Some(team_pool.clone());
            links.team_scores = Some(team_scores.clone());
            links.world = Some(world.clone());
            links.team_state = Some(team_state.clone());
            links.level = Some(level.clone());
            links.session = Some(session.clone());
            links.spawns = Some(spawns.clone());
            links.death_host = Some(death_host.clone());
            links.match_runtime = Some(game_match.clone());
            links.commands = Some(admission_commands.clone());
            links.bots = Some(host.bots());
            links.settings = Some(settings.clone());
            links.host = Some(host.clone());
        }
        let admission = Rc::new(ClientAdmissionRuntime::new(Rc::new(RuntimeAdmissionHost {
            links: admission_links.clone(),
        })));
        let command_links = Rc::new(RefCell::new(CommandLinks::default()));
        {
            let mut links = command_links.borrow_mut();
            links.team_pool = Some(team_pool.clone());
            links.team_state = Some(team_state.clone());
            links.team_scores = Some(team_scores.clone());
            links.settings = Some(settings.clone());
            links.host = Some(host.clone());
            links.imports = Some(Rc::new(RuntimeCommandImports { host: host.clone() }));
            links.team = Some(team.clone());
            links.death_host = Some(death_host.clone());
            links.spawns = Some(spawns.clone());
            links.admission = Some(admission.clone());
            links.match_runtime = Some(game_match.clone());
            links.base_pool = Some(base_pool.clone());
            links.items_host = Some(Rc::new(RuntimeItemHost {
                links: core_links.clone(),
                lifecycle: Rc::new(RefCell::new(Some(lifecycle.borrow().clone()))),
            }));
            links.teleport_host = Some(Rc::new(RuntimeTeleportHost {
                links: core_links.clone(),
            }));
        }
        let commands = Rc::new(GameCommandRuntime::new(Rc::new(RuntimeGameCommandHost {
            links: command_links.clone(),
            team_state: team_state.clone(),
        })));
        commands_cell.borrow_mut().replace(commands.clone());
        admission_commands.commands.borrow_mut().replace(commands.clone());
        think_links.borrow_mut().commands = Some(commands.clone());
        think_links.borrow_mut().admission = Some(admission.clone());
        team_links.borrow_mut().commands = Some(commands.clone());
        session_links.borrow_mut().commands = Some(commands.clone());
        match_links.borrow_mut().commands = Some(commands.clone());
        match_links.borrow_mut().admission = Some(admission.clone());
        let server_command_links = Rc::new(RefCell::new(ServerCommandLinks::default()));
        {
            let mut links = server_command_links.borrow_mut();
            links.settings = Some(settings.clone());
            links.host = Some(host.clone());
            links.commands = Some(commands.clone());
            links.bots = Some(host.bots());
            links.memory = Some(memory.clone());
            links.arenas = Some(arenas.clone());
        }
        let server_commands = Rc::new(GameServerCommandRuntime::new(
            team_pool.clone(),
            Rc::new(RuntimeArenaCvars { cvars: host.cvars() }),
            Rc::new(RuntimeServerCommandHost {
                links: server_command_links.clone(),
            }),
            GameServerCommandState::default(),
        ));
        admission_links.borrow_mut().server_commands = Some(server_commands.clone());
        let published_events = Rc::new(RefCell::new(HashMap::new()));
        let released_published = published_events.clone();
        let unobserve = host.actors().on_release(Box::new(move |actor| {
            released_published.borrow_mut().remove(actor.id());
        }));
        let unobserve: Box<dyn FnOnce()> = unobserve;
        let core = Rc::new(Q3SourceRuntimeCore {
            product: options.product,
            level: level.clone(),
            base_random,
            team_random,
            records: records.clone(),
            world,
            base_pool,
            team_pool,
            memory,
            config,
            registry,
            remaps,
            locations,
            settings,
            bridge,
            missiles,
            weapons,
            item_lifecycle: lifecycle,
            team,
            death,
            think,
            spawns,
            session,
            session_world,
            match_runtime: game_match,
            arenas,
            movers,
            mover_spawns,
            portal,
            admission,
            commands,
            server_commands,
            links: core_links,
            mover_links: mover_spawn_links,
        });
        let runtime = Self {
            options,
            host,
            core,
            mode,
            loaded: Cell::new(false),
            retired: Cell::new(false),
            loaded_game_type: Cell::new(None),
            map_report: RefCell::new(None),
            published_events,
            unobserve: RefCell::new(Some(unobserve)),
        };
        if fresh {
            return runtime;
        }
        runtime.restore_constructed();
        runtime.spawn_handlers();
        runtime
    }

    /// Prepare restored graph state when constructed from a native snapshot.
    fn restore_constructed(&self) {
        let Q3SourceConstruction::Restore(snapshot) = &self.mode else {
            return;
        };
        let content = native_save_content(self.options.product, &self.options.entities, snapshot);
        let reader = ContentSaveReader::new(&content);
        let graph_value = reader.field("graph").value.expect("native graph");
        let graph = read_q3_graph(graph_value).expect("read native graph");
        let item_count = self
            .core
            .links
            .borrow()
            .item_table
            .clone()
            .expect("item table")
            .items
            .len();
        let mut records = SaveRecords {
            records: &self.core.records,
            actors: self.host.actors(),
            item_count,
        };
        let actors = SaveActors {
            actors: self.host.actors(),
            records: self.core.records.clone(),
            provider: self.options.recipe.map.entities.provider.clone(),
        };
        prepare_q3_graph(&mut records, &graph, &actors).expect("prepare native graph");
    }

    /// Refresh the session-world cells from live level and settings readers.
    fn refresh_session_world(world: &Rc<SessionWorld>, level: &Rc<RefCell<GameLevel>>, settings: &Rc<Q3GameSettings>) {
        let borrowed = level.borrow();
        world.game_type.set(settings.integer("g_gametype"));
        world.team_auto_join.set(settings.integer("g_teamAutoJoin") != 0);
        world.max_game_clients.set(settings.integer("g_maxGameClients"));
        world.time.set(borrowed.base.time);
        world
            .num_non_spectator_clients
            .set(borrowed.base.num_non_spectator_clients);
        world.new_session.set(borrowed.new_session);
    }
}

/// Round compatibility after a map restart (`restartCompatibility`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3RestartCompatibility {
    /// Same game type and client capacity.
    Compatible,
    /// Game type changed.
    GameTypeChanged,
    /// Client capacity changed.
    ClientCapacityChanged,
}

impl Q3SourceRuntime {
    /// Current game type.
    #[must_use]
    pub fn game_type(&self) -> i32 {
        self.core.settings.integer("g_gametype")
    }

    /// Integer setting.
    fn integer(&self, name: &str) -> i32 {
        self.core.settings.integer(name)
    }

    /// Numeric setting.
    fn number(&self, name: &str) -> f32 {
        self.core.settings.number(name)
    }

    /// String setting.
    fn string(&self, name: &str) -> String {
        self.core.settings.string(name)
    }

    /// Engine cvar value.
    fn engine_cvar(&self, name: &str) -> String {
        self.host.cvars().borrow().variable_string(name)
    }

    /// Set an engine cvar.
    fn set_cvar(&self, name: &str, value: &str) {
        self.host.cvars().borrow_mut().set(name, value, true).expect("set cvar");
    }

    /// Copied source state for cgame/network consumers.
    #[must_use]
    pub fn source_state(&self) -> Q3SourcePresentationState {
        let pool = self.core.base_pool.borrow();
        let strings = self.host.configstrings();
        let borrowed = strings.borrow();
        q3_pool_presentation_state(&pool, self.options.product, self.host.now(), &*borrowed)
    }

    /// Model presentations for the shared renderer.
    #[must_use]
    pub fn presentations(&self) -> Vec<SimulationPresentation> {
        let pool = self.core.base_pool.borrow();
        let strings = self.host.configstrings();
        let borrowed = strings.borrow();
        q3_pool_models(
            &pool,
            self.options.product,
            self.core.level.borrow().base.time,
            &self.options.recipe.map.entities.content,
            &*borrowed,
        )
    }

    /// Spawn report from the loaded map.
    pub fn spawn_report(&self) -> SpawnReport {
        self.map_report.borrow().clone().expect("Q3 map not loaded")
    }

    /// Observe a supply pickup from a recipient.
    pub fn observe_supply(&self, pickup: &ActorId, recipient: &ActorId) -> Option<SupplyObservation> {
        let entity = self.core.records.native_by_actor(Some(pickup))?;
        let player = self.core.records.native_by_actor(Some(recipient))?;
        let pool = self.core.base_pool.borrow();
        #[allow(clippy::cast_possible_wrap)]
        let entity = pool.at(entity.borrow().slot as i32);
        #[allow(clippy::cast_possible_wrap)]
        let player = pool.at(player.borrow().slot as i32);
        drop(pool);
        observe_q3_supply(&entity, &player, &self.core.item_lifecycle.borrow())
    }

    /// Quad damage factor.
    #[must_use]
    pub fn quad_damage_factor(&self) -> f32 {
        self.number("g_quadfactor")
    }

    /// Step the selected holdable; true blocks the primary for this command.
    pub fn step_holdable(&self, actor: &ActorId, pressed: bool) -> bool {
        let entity = self.core.records.native_by_actor(Some(actor));
        let player = entity.as_ref().map(|native| {
            let slot = native.borrow().slot;
            #[allow(clippy::cast_possible_wrap)]
            self.core.base_pool.borrow().at(slot as i32)
        });
        let (Some(native), Some(player)) = (entity, player) else {
            return true;
        };
        let mut borrowed = player.borrow_mut();
        let Some(client) = borrowed.client.as_mut() else {
            return true;
        };
        let ps = &mut client.ps;
        let schema = stat_schema(self.options.product);
        if ps.pm_flags & MoveFlags::Respawned as i32 != 0
            || ps.pm_type == MoveType::PmSpectator
            || native.borrow().health() <= 0
        {
            return true;
        }
        let item = ps.stats.get(stat_holdable_item(schema));
        let mut state = Q3HoldableState {
            pm_flags: ps.pm_flags,
            holdable_item: item,
            holdable_tag: self
                .core
                .links
                .borrow()
                .item_table
                .clone()
                .expect("item table")
                .item_at(item)
                .tag,
            health: f64::from(native.borrow().health()),
            max_health: f64::from(ps.stats.get(stat_max_health(schema))),
        };
        let consumed = step_q3_holdable(&mut state, pressed, &mut |event| {
            ps.add_event(event, 0);
        });
        ps.pm_flags = state.pm_flags;
        ps.stats.set(stat_holdable_item(schema), state.holdable_item);
        consumed
    }

    /// Capture the native save image.
    pub fn capture_native_state(&self) -> SaveJson {
        if !self.loaded.get() || self.map_report.borrow().is_none() {
            panic!("Q3 native save requires a loaded source");
        }
        let links = self.core.links.borrow();
        let driver = links.driver.clone().expect("driver");
        let borrowed_driver = driver.borrow();
        let state_pool = borrowed_driver.state_pool.as_ref().expect("state pool");
        let item_count = links.item_table.clone().expect("item table").items.len();
        let records = SaveRecords {
            records: &self.core.records,
            actors: self.host.actors(),
            item_count,
        };
        let graph = capture_q3_graph(&records, state_pool).expect("capture graph");
        drop(borrowed_driver);
        let level = save_level_from_game(&self.core.level.borrow());
        let published: Vec<SaveJson> = self
            .published_events
            .borrow()
            .iter()
            .map(|(actor, event)| {
                engine_obj(vec![
                    ("actor", engine_json_from_saved_actor(&SavedActorId::from(actor))),
                    ("event", engine_int(i64::from(event.event))),
                    ("time", engine_int(i64::from(event.time))),
                ])
            })
            .collect();
        let portal = self.core.portal.as_ref().map_or(SaveJson::Null, |portal| {
            engine_json_from_content(&portal.borrow().capture_save_state())
        });
        engine_obj(vec![
            ("schema", engine_str("q3:native")),
            ("version", engine_int(1)),
            ("product", engine_str(self.options.product.as_str())),
            ("entityText", engine_str(&self.options.entities)),
            ("server", self.host.server_state().capture_save_state()),
            ("graph", engine_json_from_content(&graph_to_json(&graph))),
            ("level", engine_json_from_content(&capture_q3_level(&level))),
            ("random", engine_int(i64::from(self.core.base_random.borrow().seed()))),
            (
                "locations",
                engine_json_from_content(&self.core.locations.borrow().capture_save_state()),
            ),
            ("spawns", save_json_from_value(&self.core.spawns.capture_save_state())),
            (
                "match",
                save_json_from_value(&self.core.match_runtime.capture_save_state()),
            ),
            (
                "remaps",
                engine_json_from_content(&self.core.remaps.borrow().capture_save_state()),
            ),
            (
                "settings",
                engine_json_from_content(&self.core.settings.capture_save_state()),
            ),
            (
                "memory",
                engine_json_from_memory(&self.core.memory.borrow().capture_save_state()),
            ),
            (
                "registeredItems",
                SaveJson::Bytes(self.core.registry.borrow().capture_save_state()),
            ),
            (
                "bridge",
                engine_json_from_content(&self.core.bridge.capture_save_state()),
            ),
            (
                "missiles",
                engine_json_from_missiles(&self.core.missiles.borrow().capture_save_state()),
            ),
            ("team", save_json_from_value(&self.core.team.capture_save_state())),
            ("arenas", save_json_from_value(&self.core.arenas.capture_save_state())),
            ("personalPortal", portal),
            (
                "serverCommands",
                save_json_from_value(&self.core.server_commands.capture_save_state()),
            ),
            ("publishedEvents", engine_arr(published)),
            (
                "report",
                engine_json_from_report(&self.map_report.borrow().clone().expect("map report")),
            ),
        ])
    }

    /// Capture the native save image as checkpoint bytes.
    pub fn capture_native_bytes(&self) -> Vec<u8> {
        encode_checkpoint_value(&self.capture_native_state())
    }

    /// Finish hydrating a constructed-from-snapshot runtime.
    pub fn finish_native_restore(&self) {
        if self.loaded.get() || !matches!(self.mode, Q3SourceConstruction::Restore(_)) {
            panic!("Q3 native hydration requires a prepared restore source");
        }
        let Q3SourceConstruction::Restore(snapshot) = &self.mode else {
            panic!("Q3 native hydration requires a prepared restore source");
        };
        let content = native_save_content(self.options.product, &self.options.entities, snapshot);
        let reader = ContentSaveReader::new(&content);
        let level_value = reader.field("level").value.expect("native level").clone();
        let mut level = save_level_from_game(&self.core.level.borrow());
        restore_q3_level(&mut level, &level_value).expect("restore level");
        apply_save_level(&mut self.core.level.borrow_mut(), &level);
        let random_value = reader.field("random").value.expect("native random").clone();
        let seed = ContentSaveReader::new(&random_value)
            .integer(i64::from(i32::MIN))
            .expect("seed");
        #[allow(clippy::cast_possible_wrap)]
        let seed = seed as i32;
        self.core.base_random.borrow_mut().reset(seed);
        self.core.team_random.reset(seed);
        let graph_value = reader.field("graph").value.expect("native graph").clone();
        let graph = read_q3_graph(&graph_value).expect("read native graph");
        let links = self.core.links.borrow();
        let item_count = links.item_table.clone().expect("item table").items.len();
        let driver = links.driver.clone().expect("driver");
        let items_pool = links.items_pool.clone().expect("items pool");
        drop(links);
        let records = SaveRecords {
            records: &self.core.records,
            actors: self.host.actors(),
            item_count,
        };
        let actors = SaveActors {
            actors: self.host.actors(),
            records: self.core.records.clone(),
            provider: self.options.recipe.map.entities.provider.clone(),
        };
        {
            let mut borrowed_driver = driver.borrow_mut();
            let state_pool = borrowed_driver.state_mut();
            restore_q3_graph(&records, state_pool, &graph, &actors).expect("restore graph");
        }
        {
            let borrowed_driver = driver.borrow();
            let state_pool = borrowed_driver.state_pool.as_ref().expect("state pool");
            let mut items = items_pool.borrow_mut();
            let base_pool = self.core.base_pool.clone();
            let team_pool = self.core.team_pool.clone();
            for slot in 0..MAX_GENTITIES {
                sync_dyn_state_to_items(state_pool, &mut items, slot);
            }
            for slot in 0..MAX_GENTITIES {
                sync_items_to_entities_team(&items, &base_pool, &team_pool, self.options.product, slot);
            }
        }
        self.core
            .locations
            .borrow_mut()
            .restore_save_state(
                reader.field("locations").value.expect("native locations"),
                driver.borrow().state_pool.as_ref().expect("state pool"),
            )
            .expect("restore locations");
        let engine = SaveReader::new(snapshot);
        self.core.spawns.restore_save_state(&save_value_from_json(
            engine.field("spawns").value.expect("native spawns"),
        ));
        self.core.match_runtime.restore_save_state(&save_value_from_json(
            engine.field("match").value.expect("native match"),
        ));
        self.core
            .remaps
            .borrow_mut()
            .restore_save_state(reader.field("remaps").value.expect("native remaps"))
            .expect("restore remaps");
        self.core
            .memory
            .borrow_mut()
            .restore_save_state(&memory_from_engine_json(engine.field("memory")))
            .expect("restore memory");
        if let SaveJson::Bytes(bytes) = engine.field("registeredItems").value.expect("native items") {
            self.core.registry.borrow_mut().restore_save_state(bytes);
        } else {
            panic!("native items must be bytes");
        }
        self.core
            .bridge
            .restore_save_state(reader.field("bridge").value.expect("native bridge"))
            .expect("restore bridge");
        let missile_entries = missiles_from_engine_json(engine.field("missiles"));
        {
            let resolve =
                |handle: u32| -> Q3GameItemsResult<ItemsActorId> { Ok(ItemsActorId::from_slot(handle as usize)) };
            self.core
                .missiles
                .borrow_mut()
                .restore_save_state(&items_pool.borrow(), &missile_entries, &resolve)
                .expect("restore missiles");
        }
        self.core
            .team
            .restore_save_state(&save_value_from_json(engine.field("team").value.expect("native team")));
        self.core.arenas.restore_save_state(&save_value_from_json(
            engine.field("arenas").value.expect("native arenas"),
        ));
        match &self.core.portal {
            None => {
                if engine.field("personalPortal").value != Some(&SaveJson::Null) {
                    panic!("baseq3 has no personal portal state");
                }
            }
            Some(portal) => {
                portal
                    .borrow_mut()
                    .restore_save_state(reader.field("personalPortal").value.expect("native portal"))
                    .expect("restore portal");
            }
        }
        self.core.server_commands.restore_save_state(&save_value_from_json(
            engine.field("serverCommands").value.expect("native server commands"),
        ));
        self.published_events.borrow_mut().clear();
        let published: Vec<(ActorId, PublishedEvent)> = engine
            .field("publishedEvents")
            .list(|entry| {
                let actor_value = content_json_from_engine(entry.field("actor").value.expect("event actor"));
                let actor_reader = ContentSaveReader::new(&actor_value);
                let saved = read_q3_actor(&actor_reader).expect("event actor");
                let actor = resolve_saved_actor(&self.core.records, &saved).expect("live actor");
                Ok::<_, qa_world::WorldError>((
                    actor,
                    PublishedEvent {
                        event: entry.field("event").integer(0)? as i32,
                        time: entry.field("time").integer(0)? as i32,
                    },
                ))
            })
            .expect("published events");
        for (actor, event) in published {
            if self.published_events.borrow().contains_key(&actor) {
                panic!("Invalid published Q3 event actor");
            }
            self.published_events.borrow_mut().insert(actor, event);
        }
        *self.map_report.borrow_mut() = Some(report_from_engine_json(engine.field("report")));
        Self::refresh_session_world(&self.core.session_world, &self.core.level, &self.core.settings);
        self.loaded.set(true);
        self.loaded_game_type.set(Some(self.game_type()));
    }

    /// Remap CTF team shaders on missionpack maps.
    fn remap_teams(&self) {
        self.remap_teams_at(self.core.level.borrow().base.time);
    }

    /// Remap CTF team shaders at an explicit level time.
    fn remap_teams_at(&self, level_time: i32) {
        if self.options.product != Product::Missionpack {
            return;
        }
        let time = f64::from(level_time as f32 * 0.001);
        for suffix in ["01", "02"] {
            self.core
                .remaps
                .borrow_mut()
                .add(
                    &format!("textures/ctf2/redteam{suffix}"),
                    &format!("team_icon/{}_red", self.string("g_redteam")),
                    time,
                )
                .expect("remap red team");
        }
        for suffix in ["01", "02"] {
            self.core
                .remaps
                .borrow_mut()
                .add(
                    &format!("textures/ctf2/blueteam{suffix}"),
                    &format!("team_icon/{}_blue", self.string("g_blueteam")),
                    time,
                )
                .expect("remap blue team");
        }
        let built = self
            .core
            .remaps
            .borrow()
            .build_shader_state_config()
            .expect("shader state");
        self.host.configstrings().borrow_mut().set(24, &built);
    }

    /// Warn about missing team items and obelisks.
    fn check_team_items(&self) {
        self.core.team.init_game();
        let flags: &[&str] = if self.game_type() == GameType::GtCtf as i32 {
            &["Red", "Blue"]
        } else if self.options.product == Product::Missionpack && self.game_type() == GameType::Gt1fctf as i32 {
            &["Red", "Blue", "Neutral"]
        } else {
            &[]
        };
        let table = self.core.links.borrow().item_table.clone().expect("item table");
        for name in flags {
            let item = table.find_item(&format!("{name} Flag"));
            let registered = item.is_some_and(|item| self.core.registry.borrow().is_registered(item));
            if !registered {
                self.host.engine().print(&format!(
                    "^3WARNING: No team_CTF_{}flag in map",
                    name.to_ascii_lowercase()
                ));
            }
        }
        let obelisks: &[&str] = if self.options.product != Product::Missionpack {
            &[]
        } else if self.game_type() == GameType::GtObelisk as i32 {
            &["team_redobelisk", "team_blueobelisk"]
        } else if self.game_type() == GameType::GtHarvester as i32 {
            &["team_redobelisk", "team_blueobelisk", "team_neutralobelisk"]
        } else {
            &[]
        };
        let driver = self.core.links.borrow().driver.clone().expect("driver");
        let borrowed_driver = driver.borrow();
        let state_pool = borrowed_driver.state_pool.as_ref().expect("state pool");
        for name in obelisks {
            if find_entity(state_pool, None, EntityStringField::Classname, Some(name)).is_none() {
                self.host.engine().print(&format!("^3WARNING: No {name} in map"));
            }
        }
    }

    /// Round compatibility after a map restart.
    pub fn restart_compatibility(&self) -> Q3RestartCompatibility {
        if !self.loaded.get() || self.retired.get() {
            panic!("Q3 round is not active");
        }
        let cvars = self.host.cvars();
        let borrowed = cvars.borrow();
        if borrowed.get("sv_maxclients").is_some_and(|cvar| cvar.modified)
            || borrowed.variable_value("sv_maxclients") as i32 != self.options.max_clients as i32
        {
            return Q3RestartCompatibility::ClientCapacityChanged;
        }
        if borrowed.get("g_gametype").is_some_and(|cvar| cvar.modified)
            || borrowed.variable_value("g_gametype") as i32 != self.loaded_game_type.get().unwrap_or(-1)
        {
            return Q3RestartCompatibility::GameTypeChanged;
        }
        Q3RestartCompatibility::Compatible
    }

    /// Retire the runtime and release native state.
    pub fn close(&self) {
        if self.retired.get() {
            return;
        }
        self.retired.set(true);
        self.loaded.set(false);
        self.core.missiles.borrow_mut().close();
        self.core.records.close();
        if let Some(unobserve) = self.unobserve.borrow_mut().take() {
            unobserve();
        }
        self.published_events.borrow_mut().clear();
    }

    /// Spawn the map entities.
    pub fn load(&self) -> SpawnReport {
        if self.retired.get() || self.loaded.get() || matches!(self.mode, Q3SourceConstruction::Restore(_)) {
            panic!("Q3 source map cannot spawn in its current construction mode");
        }
        {
            let mut level = self.core.level.borrow_mut();
            level.base.time = self.host.now();
            level.base.start_time = level.base.time;
            level.base.warmup_modification_count = self.core.settings.snapshot("g_warmup").modification_count as i32;
        }
        self.core.memory.borrow_mut().initialize();
        self.core
            .base_pool
            .borrow_mut()
            .initialize_clients(self.options.max_clients);
        let _ = self.core.records.activate(1022);
        self.core.level.borrow_mut().fry_sound = self
            .core
            .config
            .borrow_mut()
            .sound_index(Some("sound/player/fry.wav"))
            .expect("fry sound") as i32;
        self.core.session.initialize_world();
        self.core.server_commands.process_ip_bans();
        self.core.spawns.init_body_queue();
        self.core.registry.borrow_mut().clear(self.game_type());
        let driver = self.core.links.borrow().driver.clone().expect("driver");
        let item_host = Rc::new(RuntimeItemHost {
            links: self.core.links.clone(),
            lifecycle: Rc::new(RefCell::new(Some(self.core.item_lifecycle.borrow().clone()))),
        });
        let staging = GameMemory::new(Box::new(|| 0), Box::new(|_| {}));
        let taken = std::mem::replace(&mut *self.core.memory.borrow_mut(), staging);
        let mut services = RuntimeSpawnServices {
            memory: taken,
            product: self.options.product,
            game_type: self.game_type(),
            handlers: self.spawn_handlers(),
            host: self.host.clone(),
            item_host,
            team_pool: self.core.team_pool.clone(),
        };
        let mut world = WorldspawnState {
            start_time: self.core.level.borrow().base.start_time,
            motd: self.string("g_motd"),
            restarted: self.integer("g_restarted"),
            do_warmup: self.integer("g_doWarmup"),
            warmup_time: self.core.level.borrow().base.warmup_time,
        };
        let mut borrowed_driver = driver.borrow_mut();
        let report = spawn_entities(
            &self.options.entities,
            &mut *borrowed_driver,
            &mut services,
            &mut world,
            "q3:spawn",
        )
        .expect("spawn entities");
        drop(borrowed_driver);
        self.core.level.borrow_mut().base.warmup_time = world.warmup_time;
        *self.core.memory.borrow_mut() = services.memory;
        let teams_view = RecordsPoolView {
            records: self.core.records.clone(),
        };
        find_q3_entity_teams(&teams_view);
        if self.game_type() >= GameType::GtTeam as i32 {
            self.check_team_items();
        }
        {
            let configstrings = self.host.configstrings();
            let engine = self.host.engine();
            self.core.registry.borrow().save(
                &|index, value| {
                    configstrings
                        .borrow_mut()
                        .set(usize::try_from(index).unwrap_or(0), &value);
                },
                &|text| engine.print(&text),
            );
        }
        if self.game_type() == GameType::GtSinglePlayer as i32
            || game_atoi(&self.engine_cvar("com_buildScript")).unwrap_or(0) != 0
        {
            self.core
                .config
                .borrow_mut()
                .model_index(Some("models/mapobjects/podium/podium4.md3"))
                .expect("podium");
            self.core
                .config
                .borrow_mut()
                .sound_index(Some("sound/player/gurp1.wav"))
                .expect("gurp1");
            self.core
                .config
                .borrow_mut()
                .sound_index(Some("sound/player/gurp2.wav"))
                .expect("gurp2");
        }
        self.remap_teams();
        self.core.settings.update();
        self.loaded.set(true);
        self.loaded_game_type.set(Some(self.game_type()));
        let unknown: Vec<String> = report
            .outcomes
            .iter()
            .filter_map(|outcome| match outcome {
                SpawnOutcome::Unknown { classname, .. } => {
                    Some(classname.clone().unwrap_or_else(|| "<unnamed>".to_owned()))
                }
                _ => None,
            })
            .collect();
        if !unknown.is_empty() {
            panic!("Unimplemented authored Q3 spawns: {}", unknown.join(", "));
        }
        self.host
            .configstrings()
            .borrow_mut()
            .set(0, &self.host.server_state().server_info());
        *self.map_report.borrow_mut() = Some(report.clone());
        report
    }

    /// Advance the level clock to a new frame.
    pub fn begin_frame(&self, frame: &FrameContext) {
        let mut level = self.core.level.borrow_mut();
        level.frame_num = frame.frame;
        level.previous_time = level.base.time;
        level.base.time = frame.time.as_milliseconds_truncated();
        drop(level);
        self.core.settings.update();
    }

    /// Run one actor for the current frame.
    pub fn run_actor(&self, actor: &OwnedActor) {
        let records = &self.core.records;
        let entity = records.by_actor(Some(actor.id()));
        let links = self.core.links.borrow();
        let missiles = self.core.missiles.clone();
        let missile_host = links.missile_host.clone().expect("missile host");
        let items_pool = links.items_pool.clone().expect("items pool");
        drop(links);
        {
            let mut projectiles = RuntimeProjectileDriver;
            let mut borrowed_host = missile_host.borrow_mut();
            let owned = ItemsOwnedActor {
                id: ItemsActorId::from_slot(actor.id().slot() as usize),
            };
            let ran = missiles
                .borrow_mut()
                .run_owned(
                    &mut items_pool.borrow_mut(),
                    &mut *borrowed_host,
                    &mut projectiles,
                    owned,
                )
                .expect("run owned");
            if ran {
                return;
            }
        }
        let Some(entity) = entity else {
            return;
        };
        let slot = entity.borrow().slot;
        if slot == 1022 {
            return;
        }
        let base_pool = self.core.base_pool.borrow();
        #[allow(clippy::cast_possible_wrap)]
        let base = base_pool.at(slot as i32);
        if base_pool.expire_events(&base) != EntityEventStatus::Active {
            return;
        }
        let borrowed = base.borrow();
        if borrowed.never_free
            && self
                .core
                .world
                .adapter
                .link_state(slot as i32)
                .is_none_or(|state| !state.linked)
        {
            return;
        }
        let e_type = borrowed.s.e_type;
        drop(borrowed);
        drop(base_pool);
        if e_type == EntityType::EtMissile as i32 {
            let mut projectiles = RuntimeProjectileDriver;
            let mut borrowed_host = missile_host.borrow_mut();
            missiles
                .borrow_mut()
                .run(
                    &mut items_pool.borrow_mut(),
                    &mut *borrowed_host,
                    &mut projectiles,
                    slot,
                )
                .expect("run missile");
            sync_items_to_entities_team(
                &items_pool.borrow(),
                &self.core.base_pool,
                &self.core.team_pool,
                self.options.product,
                slot,
            );
            return;
        }
        let physics_object = self.core.team_pool.at(slot).borrow().physics_object;
        if e_type == EntityType::EtItem as i32 || physics_object {
            let time = self.core.level.borrow().base.time;
            let previous_time = self.core.level.borrow().previous_time;
            let team = self.core.team.clone();
            let team_pool = self.core.team_pool.clone();
            let mut free_team_entity = |pool: &mut ItemsEntityPool, item: Slot| {
                team.free_entity(&team_pool.at(item));
                let _ = pool;
                Ok(())
            };
            let think_team = team.clone();
            let think_pool = team_pool.clone();
            let mut think = |pool: &mut ItemsEntityPool, item: Slot, name: CallbackName| {
                if name.0 == LAUNCH_ITEM_THINK {
                    return launch_item_think(pool, item);
                }
                if name.0 == DROPPED_FLAG_THINK {
                    think_team.dropped_flag_think(&think_pool.at(item));
                }
                Ok(())
            };
            let missile_world = missile_host.clone();
            let mut borrowed_host = missile_world.borrow_mut();
            let world_ops: &mut dyn ItemsWorldOps = &mut borrowed_host.world;
            let mut context = RunItemContext {
                time,
                previous_time,
                world: world_ops,
                free_team_entity: &mut free_team_entity,
                think: &mut think,
            };
            run_item(&mut items_pool.borrow_mut(), slot, &mut context).expect("run item");
            sync_items_to_entities_team(
                &items_pool.borrow(),
                &self.core.base_pool,
                &self.core.team_pool,
                self.options.product,
                slot,
            );
            return;
        }
        if e_type == EntityType::EtMover as i32 {
            self.run_mover(slot);
            return;
        }
        if slot < MAX_CLIENTS {
            self.core.think.run_client(&self.core.team_pool.at(slot));
        } else {
            let base_pool = self.core.base_pool.borrow();
            #[allow(clippy::cast_possible_wrap)]
            let base = base_pool.at(slot as i32);
            let time = self.core.level.borrow().base.time;
            entities_run_think(&base, time);
        }
    }

    /// Run a mover chain and mirror body velocities.
    fn run_mover(&self, slot: usize) {
        let base_pool = self.core.base_pool.borrow();
        #[allow(clippy::cast_possible_wrap)]
        let base = base_pool.at(slot as i32);
        if base.borrow().flags & GameFlags::TEAMSLAVE != 0 {
            return;
        }
        let mut before: Vec<(OwnedActor, Vec3)> = Vec::new();
        let mut part: Option<GameEntityRef> = Some(base.clone());
        while let Some(current) = part {
            let borrowed = current.borrow();
            let slot = borrowed.slot;
            let origin = borrowed.r.current_origin;
            part = borrowed.teamchain.clone();
            drop(borrowed);
            if let Some(record) = self.core.records.get(slot) {
                before.push((record.borrow().binding.actor().clone(), origin));
            }
        }
        drop(base_pool);
        let links = self.core.links.borrow();
        let driver = links.driver.clone().expect("driver");
        drop(links);
        let mut borrowed_driver = driver.borrow_mut();
        self.core
            .movers
            .borrow()
            .run(&mut *borrowed_driver, slot)
            .expect("run mover");
        let level = self.core.level.borrow();
        let elapsed = (level.base.time - level.previous_time) as f32 * 0.001;
        drop(level);
        for (actor, origin) in before {
            let Some(body) = self.host.bodies().read(actor.id()) else {
                continue;
            };
            let component = |after: f32, start: f32| {
                if elapsed <= 0.0 {
                    0.0
                } else {
                    (after - start) / elapsed
                }
            };
            self.host.bodies().write(
                &actor,
                BodyState {
                    velocity: vec3(
                        component(body.origin.x, origin.x),
                        component(body.origin.y, origin.y),
                        component(body.origin.z, origin.z),
                    ),
                    ..body
                },
            );
        }
    }

    /// Run end-of-frame systems and publish entity events.
    pub fn end_frame(&self) {
        let effects = self.core.links.borrow().effects.clone().expect("effects");
        let max_clients = self.core.base_pool.borrow().max_clients();
        for index in 0..max_clients {
            let pool = self.core.base_pool.borrow();
            #[allow(clippy::cast_possible_wrap)]
            let entity = pool.at(index as i32);
            if !entity.borrow().inuse {
                continue;
            }
            drop(pool);
            client_end_frame(&*effects, &self.core.team_pool.at(index));
        }
        self.core.match_runtime.check_tournament();
        self.core.match_runtime.check_exit_rules();
        self.core.team.check_team_status();
        self.core.match_runtime.check_vote();
        self.core.match_runtime.check_team_vote(Team::TeamRed as i32);
        self.core.match_runtime.check_team_vote(Team::TeamBlue as i32);
        self.core.match_runtime.check_cvars();
        if self.integer("g_listEntity") != 0 {
            for index in 0..MAX_GENTITIES {
                #[allow(clippy::cast_possible_wrap)]
                let entity = self.core.base_pool.borrow().at(index as i32);
                let classname = entity.borrow().classname.clone().unwrap_or_default();
                self.host.engine().print(&game_format(
                    "%4i: %s\n",
                    &[
                        GameFormatArgument::Int(index as i32),
                        GameFormatArgument::Text(classname),
                    ],
                ));
            }
            self.set_cvar("g_listEntity", "0");
        }
        self.host
            .configstrings()
            .borrow_mut()
            .set(0, &self.host.server_state().server_info());
        self.publish_events();
    }

    /// Publish fresh entity events to the host.
    fn publish_events(&self) {
        let pool = self.core.base_pool.borrow();
        for slot in 0..pool.num_entities() {
            #[allow(clippy::cast_possible_wrap)]
            let entity = pool.at(slot as i32);
            let borrowed = entity.borrow();
            if !borrowed.inuse {
                continue;
            }
            let event = if borrowed.s.e_type >= EntityType::EtEvents as i32 {
                borrowed.s.e_type - EntityType::EtEvents as i32
            } else {
                borrowed.s.event
            };
            if event == 0 {
                continue;
            }
            let id = {
                let slot = borrowed.slot;
                let Some(record) = self.core.records.get(slot) else {
                    continue;
                };
                let borrowed_record = record.borrow();
                borrowed_record.binding.actor().id().clone()
            };
            let fresh = match self.published_events.borrow().get(&id) {
                Some(previous) => previous.event != event || previous.time != borrowed.event_time,
                None => true,
            };
            if !fresh {
                continue;
            }
            let state = borrowed.s.copy();
            let origin = borrowed.r.current_origin;
            let time = borrowed.event_time;
            drop(borrowed);
            self.published_events
                .borrow_mut()
                .insert(id.clone(), PublishedEvent { event, time });
            self.host.entity_event(Q3SourceEntityEvent {
                actor: id,
                state,
                origin,
                time: self.core.level.borrow().base.time,
            });
        }
    }

    /// Attach an actor to a client slot.
    pub fn prepare_client(&self, actor: &OwnedActor, client: i32) -> RecordsEntityRef {
        if !self.loaded.get() {
            panic!("Q3 source map must be loaded before admitting players");
        }
        #[allow(clippy::cast_sign_loss)]
        self.core
            .records
            .attach(client as usize, actor.clone(), true)
            .expect("attach client")
    }

    /// Admit a player into a client slot.
    pub fn admit_player(&self, actor: &OwnedActor, client: i32) -> RecordsEntityRef {
        let entity = self.prepare_client(actor, client);
        #[allow(clippy::cast_sign_loss)]
        let slot = client as usize;
        let restored = self
            .options
            .session_carry
            .as_ref()
            .is_some_and(|carry| carry.clients.iter().any(|saved| saved.slot as usize == slot));
        if let Some(rejected) = self.core.admission.connect(client, !restored, false) {
            panic!("{}", Q3ClientAdmissionDenied { reason: rejected });
        }
        self.core.admission.begin(client);
        entity
    }

    /// Capture the session carry for the next map.
    pub fn capture_session(&self) -> Q3SourceSessionCarry {
        self.core.session.write_world();
        let mut clients = Vec::new();
        for slot in 0..self.core.base_pool.borrow().max_clients() {
            #[allow(clippy::cast_possible_wrap)]
            let number = slot as i32;
            let client = self.core.base_pool.borrow().client_at(number);
            if client.pers.connected == ConnectionState::Disconnected {
                continue;
            }
            self.core.session.write_client(number);
            clients.push(Q3SourceSessionClient {
                slot: number,
                session: self.host.cvars().borrow().variable_string(&format!("session{slot}")),
                userinfo: self.host.engine().get_userinfo(number),
            });
        }
        Q3SourceSessionCarry {
            world: self.host.cvars().borrow().variable_string("session"),
            clients,
        }
    }

    /// Disconnect a player actor.
    pub fn disconnect_player(&self, actor: &ActorId) {
        let entity = self.core.records.by_actor(Some(actor));
        if entity.as_ref().is_some_and(|native| native.borrow().client.is_some()) {
            #[allow(clippy::cast_possible_wrap)]
            self.core
                .admission
                .disconnect(entity.expect("player").borrow().slot as i32);
        }
    }

    /// Run a client movement command.
    pub fn player_think(&self, input: &ActorCommand) {
        let entity = self.core.records.by_actor(Some(&input.actor));
        if entity.as_ref().is_none_or(|native| native.borrow().client.is_none()) {
            panic!("Q3 client command has no admitted actor");
        }
        let slot = entity.expect("player").borrow().slot;
        self.core.think.client_think(slot, &self.host.source_command(input));
    }

    /// Dispatch a client console command.
    pub fn player_command(&self, actor: &ActorId, name: &str, args: &[String]) {
        let entity = self.core.records.by_actor(Some(actor));
        if entity.as_ref().is_none_or(|native| native.borrow().client.is_none()) {
            panic!("Q3 client command has no admitted actor");
        }
        let slot = entity.expect("player").borrow().slot;
        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push(name.to_owned());
        argv.extend(args.iter().cloned());
        self.core.commands.dispatch(slot, &argv);
    }

    /// Fire a player weapon.
    pub fn player_weapon(&self, actor: &ActorId) {
        let entity = self.core.records.by_actor(Some(actor));
        if entity.as_ref().is_none_or(|native| native.borrow().client.is_none()) {
            panic!("Q3 weapon fire has no admitted actor");
        }
        let slot = entity.expect("player").borrow().slot;
        {
            let pool = self.core.base_pool.borrow();
            #[allow(clippy::cast_possible_wrap)]
            let base = pool.at(slot as i32);
            let mut borrowed = base.borrow_mut();
            let weapon = borrowed.client.as_ref().expect("player client").ps.weapon;
            borrowed.s.weapon = weapon as i32;
        }
        let driver = self.core.links.borrow().driver.clone().expect("driver");
        let mut borrowed_driver = driver.borrow_mut();
        self.core
            .weapons
            .borrow_mut()
            .fire(&mut *borrowed_driver, slot)
            .expect("fire weapon");
    }

    /// Run pre-reaction hooks for a damage decision.
    pub fn before_reaction(&self, actor: &OwnedActor, decision: &RecordsDamageDecision) {
        self.core.bridge.before_reaction(decision);
        let entity = self.core.records.native_by_actor(Some(actor.id()));
        if decision.reaction == Reaction::Death
            && entity.as_ref().is_some_and(|native| native.borrow().client.is_some())
        {
            let native = entity.expect("victim");
            let slot = native.borrow().slot;
            let pool = self.core.base_pool.borrow();
            #[allow(clippy::cast_possible_wrap)]
            let base = pool.at(slot as i32);
            let inflictor = self
                .core
                .records
                .damage_inflictor(decision.request.attack.inflictor.as_ref());
            let attacker = self
                .core
                .records
                .damage_inflictor(decision.request.attack.attacker.as_ref());
            let game_inflictor = game_participant_from_records(&pool, inflictor);
            let game_attacker = game_participant_from_records(&pool, attacker);
            let means_of_death = match &decision.request.attack.cause {
                RecordsAttackCause::Q3 { means_of_death, .. } => *means_of_death,
                _ => 0,
            };
            self.core.death.player_die(
                &base,
                Some(&game_inflictor),
                Some(&game_attacker),
                decision.applied_damage,
                means_of_death,
            );
        }
    }
}

impl ItemsMoverCore for RuntimeMoverCore {
    fn time(&self) -> i32 {
        self.links.borrow().level.clone().expect("level").borrow().base.time
    }

    fn sound_index(&mut self, path: &str) -> i32 {
        let links = self.links.borrow();
        let config = links.config.clone().expect("config");
        let mut borrowed = config.borrow_mut();
        let indexed = borrowed.sound_index(Some(path)).expect("mover sound") as i32;
        drop(borrowed);
        indexed
    }

    fn use_binary(
        &mut self,
        pool: &mut ItemsEntityPool,
        entity: Slot,
        other: Option<ItemsDamageParticipant>,
        activator: Option<ItemsDamageParticipant>,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let movers = links.movers.clone().expect("movers");
        let driver = links.driver.clone().expect("driver");
        let records = links
            .core
            .clone()
            .expect("core")
            .borrow()
            .records
            .clone()
            .expect("records");
        let other = other
            .as_ref()
            .map(|participant| state_participant_for_items(&records, participant));
        let activator = activator
            .as_ref()
            .map(|participant| state_participant_for_items(&records, participant));
        sync_items_to_state(pool, driver.borrow_mut().state_mut(), entity);
        movers
            .borrow()
            .use_binary(&mut *driver.borrow_mut(), entity, other, activator)
            .map_err(|error| Q3GameItemsError::Invalid(error.to_string()))?;
        sync_state_to_items(driver.borrow().state(), pool, entity);
        Ok(())
    }

    fn match_team(
        &mut self,
        pool: &mut ItemsEntityPool,
        leader: Slot,
        state: StateMoverState,
        time: i32,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let movers = links.movers.clone().expect("movers");
        let driver = links.driver.clone().expect("driver");
        sync_items_to_state(pool, driver.borrow_mut().state_mut(), leader);
        movers
            .borrow()
            .match_team(&mut *driver.borrow_mut(), leader, state, time)
            .map_err(|error| Q3GameItemsError::Invalid(error.to_string()))?;
        sync_state_to_items(driver.borrow().state(), pool, leader);
        Ok(())
    }

    fn blocked_door(
        &mut self,
        pool: &mut ItemsEntityPool,
        entity: Slot,
        other: ItemsDamageParticipant,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let movers = links.movers.clone().expect("movers");
        let driver = links.driver.clone().expect("driver");
        let records = links
            .core
            .clone()
            .expect("core")
            .borrow()
            .records
            .clone()
            .expect("records");
        let other = state_participant_for_items(&records, &other);
        sync_items_to_state(pool, driver.borrow_mut().state_mut(), entity);
        movers
            .borrow()
            .blocked_door(&mut *driver.borrow_mut(), entity, &other)
            .map_err(|error| Q3GameItemsError::Invalid(error.to_string()))?;
        sync_state_to_items(driver.borrow().state(), pool, entity);
        Ok(())
    }

    fn initialize_binary(
        &mut self,
        pool: &mut ItemsEntityPool,
        entity: Slot,
        _variables: &ItemsSpawnVariables,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let movers = links.movers.clone().expect("movers");
        let driver = links.driver.clone().expect("driver");
        let variables = links.spawn_variables.get(&entity).expect("spawn variables").clone();
        sync_items_to_state(pool, driver.borrow_mut().state_mut(), entity);
        movers
            .borrow()
            .initialize_binary(&mut *driver.borrow_mut(), entity, &variables)
            .map_err(|error| Q3GameItemsError::Invalid(error.to_string()))?;
        sync_state_to_items(driver.borrow().state(), pool, entity);
        Ok(())
    }

    fn set_state(
        &mut self,
        pool: &mut ItemsEntityPool,
        entity: Slot,
        state: StateMoverState,
        time: i32,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let movers = links.movers.clone().expect("movers");
        let driver = links.driver.clone().expect("driver");
        sync_items_to_state(pool, driver.borrow_mut().state_mut(), entity);
        movers
            .borrow()
            .set_state(&mut *driver.borrow_mut(), entity, state, time)
            .map_err(|error| Q3GameItemsError::Invalid(error.to_string()))?;
        sync_state_to_items(driver.borrow().state(), pool, entity);
        Ok(())
    }
}

impl ItemsMoverSpawnHost for RuntimeMoverSpawnHost {
    fn movers(&mut self) -> &mut dyn ItemsMoverCore {
        &mut self.mover_core
    }

    fn combat_and_world(&mut self) -> (&mut dyn ItemsCombatOps, &mut dyn ItemsWorldOps) {
        (&mut self.combat, &mut self.world)
    }

    fn gravity(&self) -> f32 {
        self.links
            .borrow()
            .settings
            .clone()
            .expect("settings")
            .number("g_gravity")
    }

    fn set_brush_model(&mut self, pool: &mut ItemsEntityPool, slot: Slot, name: Option<&str>) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let host = links.host.clone().expect("host");
        let name = name.ok_or_else(|| Q3GameItemsError::Invalid("brush model missing".to_string()))?;
        if !name.starts_with('*') {
            return Err(Q3GameItemsError::Invalid(format!("{name} is not a brush model")));
        }
        let index = game_atoi(&name[1..]).map_err(|error| Q3GameItemsError::Invalid(error.to_string()))?;
        let entity = pool.at_mut(slot)?;
        entity.s.modelindex = index;
        let bounds = host.scene().model_bounds(index);
        entity.r.mins = bounds.min;
        entity.r.maxs = bounds.max;
        entity.r.contents = -1;
        drop(links);
        self.world.link(pool, slot)?;
        Ok(())
    }

    fn remap_shader(&mut self, old_name: &str, new_name: &str, time_seconds: f32) {
        let links = self.links.borrow();
        let remaps = links.remaps.clone().expect("remaps");
        let host = links.host.clone().expect("host");
        remaps
            .borrow_mut()
            .add(old_name, new_name, f64::from(time_seconds))
            .expect("remap shader");
        let state = remaps.borrow().build_shader_state_config().expect("shader remap");
        host.configstrings().borrow_mut().set(24, &state);
    }

    fn use_targets(
        &mut self,
        pool: &mut ItemsEntityPool,
        entity: Slot,
        activator: Option<ItemsDamageParticipant>,
    ) -> Q3GameItemsResult<()> {
        let links = self.links.borrow();
        let driver = links.driver.clone().expect("driver");
        let records = links
            .core
            .clone()
            .expect("core")
            .borrow()
            .records
            .clone()
            .expect("records");
        let activator = activator
            .as_ref()
            .map(|participant| state_participant_for_items(&records, participant));
        sync_items_to_state(pool, driver.borrow_mut().state_mut(), entity);
        driver.borrow_mut().use_targets(entity, activator);
        Ok(())
    }

    fn warn(&mut self, message: &str) {
        self.links.borrow().host.clone().expect("host").engine().print(message);
    }
}

/// Spawn services over runtime-owned memory, handlers, and engine sinks.
///
/// Memory is moved out of the shared cell for the duration of the spawn pass
/// (the services trait needs `&mut` ownership) and moved back afterwards.
struct RuntimeSpawnServices {
    memory: GameMemory,
    product: Product,
    game_type: i32,
    handlers: SpawnHandlerTable,
    host: Rc<dyn Q3SourceHost>,
    item_host: Rc<RuntimeItemHost>,
    team_pool: PoolRef,
}

impl SpawnServices for RuntimeSpawnServices {
    fn memory(&mut self) -> &mut GameMemory {
        &mut self.memory
    }

    fn product(&self) -> Product {
        self.product
    }

    fn game_type(&self) -> i32 {
        self.game_type
    }

    fn handlers(&self) -> &SpawnHandlerTable {
        &self.handlers
    }

    fn spawn_item(
        &mut self,
        _driver: &mut dyn Q3Driver,
        slot: usize,
        item: usize,
        variables: &SpawnVariables,
    ) -> Result<(), Q3GameError> {
        let entity = self.team_pool.at(slot);
        let table = self.item_host.links.borrow().item_table.clone().expect("item table");
        #[allow(clippy::cast_possible_wrap)]
        let definition = table.item_at(item as i32).clone();
        let cvar = format!("disable_{}", definition.class_name.as_deref().unwrap_or_default());
        let off = game_atoi(&self.host.cvars().borrow().variable_string(&cvar)).unwrap_or(0) != 0;
        self.item_host.spawn_item(&entity, &definition, variables, off);
        Ok(())
    }

    fn warn(&mut self, message: &str) {
        self.host.engine().print(message);
    }
}

/// Items-registry adapter over the shared item registry.
struct RuntimeItemRegistry {
    registry: Rc<RefCell<ItemRegistry>>,
    table: Rc<EntitiesItemTable>,
}

impl ItemsItemRegistry for RuntimeItemRegistry {
    fn product(&self) -> Product {
        self.registry.borrow().product
    }

    fn register(&mut self, item: &ItemsCoreItemDefinition) {
        let entry = self
            .table
            .items
            .iter()
            .find(|entry| entry.class_name == item.class_name)
            .cloned()
            .unwrap_or(EntitiesItemDefinition {
                class_name: item.class_name.clone(),
                pickup_name: None,
                quantity: item.quantity,
                item_type: item.item_type(),
                tag: 0,
            });
        self.registry.borrow_mut().register(&entry);
    }
}

/// Trigger-spawn classnames merged from the content trigger table.
const TRIGGER_SPAWN_CLASSES: [&str; 6] = [
    "trigger_multiple",
    "trigger_always",
    "trigger_push",
    "trigger_teleport",
    "trigger_hurt",
    "func_timer",
];

/// Target-spawn classnames merged from the content target table.
const TARGET_SPAWN_CLASSES: [&str; 13] = [
    "target_give",
    "target_remove_powerups",
    "target_delay",
    "target_score",
    "target_print",
    "target_speaker",
    "target_laser",
    "target_teleporter",
    "target_relay",
    "target_position",
    "target_push",
    "target_kill",
    "target_location",
];

impl Q3SourceRuntime {
    /// Assemble the spawn-handler table over shared pools and runtimes.
    fn spawn_handlers(&self) -> SpawnHandlerTable {
        let mut handlers = SpawnHandlerTable::new();
        let team_pool = self.core.team_pool.clone();
        for classname in ["info_player_start", "info_player_deathmatch"] {
            let pool = team_pool.clone();
            let start = classname == "info_player_start";
            handlers.insert(
                classname,
                Rc::new(
                    move |_driver: &mut dyn Q3Driver,
                          _services: &mut dyn SpawnServices,
                          slot: usize,
                          variables: &SpawnVariables| {
                        let entity = pool.at(slot);
                        if start {
                            spawn_player_start(&entity, variables);
                        } else {
                            spawn_deathmatch_point(&entity, variables);
                        }
                        Ok(())
                    },
                ),
            );
        }
        for classname in ["info_player_intermission", "item_botroam"] {
            handlers.insert(
                classname,
                Rc::new(
                    move |_driver: &mut dyn Q3Driver,
                          _services: &mut dyn SpawnServices,
                          _slot: usize,
                          _variables: &SpawnVariables| Ok(()),
                ),
            );
        }
        for classname in [
            "team_CTF_redplayer",
            "team_CTF_blueplayer",
            "team_CTF_redspawn",
            "team_CTF_bluespawn",
        ] {
            let pool = team_pool.clone();
            handlers.insert(
                classname,
                Rc::new(
                    move |_driver: &mut dyn Q3Driver,
                          _services: &mut dyn SpawnServices,
                          slot: usize,
                          _variables: &SpawnVariables| {
                        spawn_team_point(&pool.at(slot));
                        Ok(())
                    },
                ),
            );
        }
        self.insert_mover_spawns(&mut handlers);
        self.insert_misc_spawns(&mut handlers);
        let triggers = trigger_spawn_handlers();
        for classname in TRIGGER_SPAWN_CLASSES {
            let handler = triggers.get(classname).expect("trigger handler");
            handlers.insert(classname, handler);
        }
        let targets = target_spawn_handlers(&self.core.locations);
        for classname in TARGET_SPAWN_CLASSES {
            let handler = targets.get(classname).expect("target handler");
            handlers.insert(classname, handler);
        }
        let remove = handlers
            .get("info_null")
            .expect("Source info_null handler is unavailable");
        handlers.insert("func_group", remove);
        if self.core.product == Product::Missionpack {
            let team = self.core.team.clone();
            let pool = team_pool.clone();
            for (classname, code) in [
                ("team_redobelisk", Team::TeamRed as i32),
                ("team_blueobelisk", Team::TeamBlue as i32),
            ] {
                let team = team.clone();
                let pool = pool.clone();
                handlers.insert(
                    classname,
                    Rc::new(
                        move |_driver: &mut dyn Q3Driver,
                              _services: &mut dyn SpawnServices,
                              slot: usize,
                              _variables: &SpawnVariables| {
                            team.spawn_team_obelisk(&pool.at(slot), code);
                            Ok(())
                        },
                    ),
                );
            }
            let team = self.core.team.clone();
            let pool = team_pool.clone();
            handlers.insert(
                "team_neutralobelisk",
                Rc::new(
                    move |_driver: &mut dyn Q3Driver,
                          _services: &mut dyn SpawnServices,
                          slot: usize,
                          _variables: &SpawnVariables| {
                        team.spawn_neutral_obelisk(&pool.at(slot));
                        Ok(())
                    },
                ),
            );
        }
        handlers
    }

    /// Insert mover-spawn adapters bridging driver slots through the items pool.
    fn insert_mover_spawns(&self, handlers: &mut SpawnHandlerTable) {
        for (classname, handler) in mover_spawn_handlers() {
            let links = self.core.links.clone();
            let mover_links = self.core.mover_links.clone();
            let mover_spawns = self.core.mover_spawns.clone();
            handlers.insert(
                classname,
                Rc::new(
                    move |driver: &mut dyn Q3Driver,
                          _services: &mut dyn SpawnServices,
                          slot: usize,
                          variables: &SpawnVariables| {
                        let items_pool = links.borrow().items_pool.clone().expect("items pool");
                        mover_links.borrow_mut().spawn_variables.insert(slot, variables.clone());
                        sync_dyn_state_to_items(driver.pool(), &mut items_pool.borrow_mut(), slot);
                        let items_variables = ItemsSpawnVariables::new(
                            variables
                                .entries
                                .iter()
                                .map(|pair| (pair.key.clone(), pair.value.clone()))
                                .collect(),
                        );
                        let outcome = run_mover_spawn(
                            &mut items_pool.borrow_mut(),
                            &mut *mover_spawns.borrow_mut(),
                            &handler,
                            slot,
                            &items_variables,
                        );
                        let outcome = outcome.map_err(failure_from_items);
                        sync_items_to_dyn_state(&items_pool.borrow(), driver.pool(), slot);
                        outcome
                    },
                ),
            );
        }
    }

    /// Insert misc-spawn adapters bridging driver slots through the items pool.
    fn insert_misc_spawns(&self, handlers: &mut SpawnHandlerTable) {
        for (classname, handler) in misc_spawn_handlers() {
            let links = self.core.links.clone();
            let registry = self.core.registry.clone();
            handlers.insert(
                classname,
                Rc::new(
                    move |driver: &mut dyn Q3Driver,
                          _services: &mut dyn SpawnServices,
                          slot: usize,
                          variables: &SpawnVariables| {
                        let borrowed = links.borrow();
                        let items_pool = borrowed.items_pool.clone().expect("items pool");
                        let missiles = borrowed.missiles.clone().expect("missiles");
                        let missile_host = borrowed.missile_host.clone().expect("missile host");
                        let random = borrowed.random.clone().expect("random");
                        let table = borrowed.item_table.clone().expect("item table");
                        let level = borrowed.level.clone().expect("level");
                        let host = borrowed.host.clone().expect("host");
                        drop(borrowed);
                        sync_dyn_state_to_items(driver.pool(), &mut items_pool.borrow_mut(), slot);
                        let items_variables = ItemsSpawnVariables::new(
                            variables
                                .entries
                                .iter()
                                .map(|pair| (pair.key.clone(), pair.value.clone()))
                                .collect(),
                        );
                        let table = RuntimeItemsTable { table };
                        let mut registries = RuntimeItemRegistry {
                            registry: registry.clone(),
                            table: table.table.clone(),
                        };
                        let mut projectile = RuntimeProjectileDriver;
                        let mut random = MissileRandomAdapter { random };
                        let mut world = RuntimeWorldOps { links: links.clone() };
                        let warn_host = host.clone();
                        let mut warn = |message: &str| warn_host.engine().print(message);
                        let mut spawn_host = MiscSpawnHost {
                            missiles: &mut *missiles.borrow_mut(),
                            missile_host: &mut *missile_host.borrow_mut(),
                            missile_driver: &mut projectile,
                            item_registry: &mut registries,
                            items: &table,
                            random: &mut random,
                            world: &mut world,
                            time: level.borrow().base.time,
                            warn: &mut warn,
                        };
                        let outcome = run_misc_spawn(
                            &mut items_pool.borrow_mut(),
                            &mut spawn_host,
                            &handler,
                            slot,
                            &items_variables,
                        );
                        let outcome = outcome.map_err(failure_from_items);
                        sync_items_to_dyn_state(&items_pool.borrow(), driver.pool(), slot);
                        outcome
                    },
                ),
            );
        }
    }
}

/// Map an items-pool failure into a driver failure.
fn failure_from_items(error: Q3GameItemsError) -> Q3GameError {
    Q3GameError::Failure(error.to_string())
}

/// Convert a team-arena save value into checkpoint JSON (lossless via big integers).
fn save_json_from_value(value: &TeamSaveValue) -> SaveJson {
    match value {
        TeamSaveValue::Null => SaveJson::Null,
        TeamSaveValue::Bool(flag) => SaveJson::Bool(*flag),
        TeamSaveValue::Int(number) => SaveJson::BigInt(i128::from(*number)),
        TeamSaveValue::Str(text) => SaveJson::String(text.clone()),
        TeamSaveValue::List(items) => SaveJson::Array(items.iter().map(save_json_from_value).collect()),
        TeamSaveValue::Map(entries) => SaveJson::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), save_json_from_value(entry)))
                .collect(),
        ),
    }
}

/// Convert checkpoint JSON back into a team-arena save value.
fn save_value_from_json(value: &SaveJson) -> TeamSaveValue {
    match value {
        SaveJson::Null => TeamSaveValue::Null,
        SaveJson::Bool(flag) => TeamSaveValue::Bool(*flag),
        SaveJson::Number(number) => TeamSaveValue::Int(*number as i64),
        SaveJson::BigInt(number) => TeamSaveValue::Int(*number as i64),
        SaveJson::Bytes(bytes) => {
            TeamSaveValue::Str(bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
        }
        SaveJson::String(text) => TeamSaveValue::Str(text.clone()),
        SaveJson::Array(items) => TeamSaveValue::List(items.iter().map(save_value_from_json).collect()),
        SaveJson::Object(entries) => TeamSaveValue::Map(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), save_value_from_json(entry)))
                .collect(),
        ),
    }
}

/// Convert content checkpoint JSON into engine checkpoint JSON.
fn engine_json_from_content(value: &ContentSaveJson) -> SaveJson {
    match value {
        ContentSaveJson::Null => SaveJson::Null,
        ContentSaveJson::Bool(flag) => SaveJson::Bool(*flag),
        ContentSaveJson::Number(number) => SaveJson::Number(*number),
        ContentSaveJson::BigInt(number) => SaveJson::BigInt(*number),
        ContentSaveJson::Bytes(bytes) => SaveJson::Bytes(bytes.clone()),
        ContentSaveJson::String(text) => SaveJson::String(text.clone()),
        ContentSaveJson::Array(items) => SaveJson::Array(items.iter().map(engine_json_from_content).collect()),
        ContentSaveJson::Object(entries) => SaveJson::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), engine_json_from_content(entry)))
                .collect(),
        ),
    }
}

/// Convert engine checkpoint JSON back into content checkpoint JSON.
fn content_json_from_engine(value: &SaveJson) -> ContentSaveJson {
    match value {
        SaveJson::Null => ContentSaveJson::Null,
        SaveJson::Bool(flag) => ContentSaveJson::Bool(*flag),
        SaveJson::Number(number) => ContentSaveJson::Number(*number),
        SaveJson::BigInt(number) => ContentSaveJson::BigInt(*number),
        SaveJson::Bytes(bytes) => ContentSaveJson::Bytes(bytes.clone()),
        SaveJson::String(text) => ContentSaveJson::String(text.clone()),
        SaveJson::Array(items) => ContentSaveJson::Array(items.iter().map(content_json_from_engine).collect()),
        SaveJson::Object(entries) => ContentSaveJson::Object(
            entries
                .iter()
                .map(|(key, entry)| (key.clone(), content_json_from_engine(entry)))
                .collect(),
        ),
    }
}

/// Validate a native snapshot envelope and return its content JSON.
fn native_save_content(product: Product, entities: &str, snapshot: &SaveJson) -> ContentSaveJson {
    let content = content_json_from_engine(snapshot);
    let reader = ContentSaveReader::new(&content);
    reader.field("schema").literal_str("q3:native").expect("native schema");
    reader.field("version").literal_i64(1).expect("native version");
    reader
        .field("product")
        .literal_str(product.as_str())
        .expect("native product");
    reader
        .field("entityText")
        .literal_str(entities)
        .expect("native entities");
    content
}

/// Convert a live level into its save-shape twin.
fn save_level_from_game(level: &GameLevel) -> Q3GameLevel {
    let mut team_scores = Q3PlayerSlots::new(level.team_scores.len());
    for index in 0..level.team_scores.len() {
        team_scores.set(index, level.team_scores.get(index).expect("team score"));
    }
    let vote = Q3VoteState {
        time: level.base.vote.time,
        yes: level.base.vote.yes,
        no: level.base.vote.no,
        string: level.base.vote.string.clone(),
        display_string: level.base.vote.display_string.clone(),
        execute_time: level.base.vote.execute_time,
    };
    let team_votes = [
        Q3TeamVoteState {
            time: level.base.team_votes[0].time,
            yes: level.base.team_votes[0].yes,
            no: level.base.team_votes[0].no,
            string: level.base.team_votes[0].string.clone(),
        },
        Q3TeamVoteState {
            time: level.base.team_votes[1].time,
            yes: level.base.team_votes[1].yes,
            no: level.base.team_votes[1].no,
            string: level.base.team_votes[1].string.clone(),
        },
    ];
    Q3GameLevel {
        time: level.base.time,
        start_time: level.base.start_time,
        warmup_time: level.base.warmup_time,
        warmup_modification_count: level.base.warmup_modification_count,
        restarted: level.base.restarted,
        num_connected_clients: level.base.num_connected_clients,
        num_non_spectator_clients: level.base.num_non_spectator_clients,
        num_playing_clients: level.base.num_playing_clients,
        num_voting_clients: level.base.num_voting_clients,
        num_team_voting_clients: level.base.num_team_voting_clients,
        sorted_clients: level.base.sorted_clients.clone(),
        follow1: level.base.follow1,
        follow2: level.base.follow2,
        intermission_time: level.base.intermission_time,
        intermission_queued: level.base.intermission_queued,
        intermission_origin: level.base.intermission_origin,
        intermission_angle: level.base.intermission_angle,
        changemap: level.base.changemap.clone(),
        ready_to_exit: level.base.ready_to_exit,
        exit_time: level.base.exit_time,
        frame_num: level.frame_num,
        previous_time: level.previous_time,
        new_session: level.new_session,
        fry_sound: level.fry_sound,
        team_scores,
        vote,
        team_votes,
    }
}

/// Apply a restored save-shape level onto the live level.
fn apply_save_level(level: &mut GameLevel, saved: &Q3GameLevel) {
    level.base.time = saved.time;
    level.base.start_time = saved.start_time;
    level.base.warmup_time = saved.warmup_time;
    level.base.warmup_modification_count = saved.warmup_modification_count;
    level.base.restarted = saved.restarted;
    level.base.num_connected_clients = saved.num_connected_clients;
    level.base.num_non_spectator_clients = saved.num_non_spectator_clients;
    level.base.num_playing_clients = saved.num_playing_clients;
    level.base.num_voting_clients = saved.num_voting_clients;
    level.base.num_team_voting_clients = saved.num_team_voting_clients;
    level.base.sorted_clients = saved.sorted_clients.clone();
    level.base.follow1 = saved.follow1;
    level.base.follow2 = saved.follow2;
    level.base.intermission_time = saved.intermission_time;
    level.base.intermission_queued = saved.intermission_queued;
    level.base.intermission_origin = saved.intermission_origin;
    level.base.intermission_angle = saved.intermission_angle;
    level.base.changemap = saved.changemap.clone();
    level.base.ready_to_exit = saved.ready_to_exit;
    level.base.exit_time = saved.exit_time;
    level.base.vote = VoteState {
        time: saved.vote.time,
        yes: saved.vote.yes,
        no: saved.vote.no,
        string: saved.vote.string.clone(),
        display_string: saved.vote.display_string.clone(),
        execute_time: saved.vote.execute_time,
    };
    level.base.team_votes = [
        TeamVoteState {
            time: saved.team_votes[0].time,
            yes: saved.team_votes[0].yes,
            no: saved.team_votes[0].no,
            string: saved.team_votes[0].string.clone(),
        },
        TeamVoteState {
            time: saved.team_votes[1].time,
            yes: saved.team_votes[1].yes,
            no: saved.team_votes[1].no,
            string: saved.team_votes[1].string.clone(),
        },
    ];
    level.frame_num = saved.frame_num;
    level.previous_time = saved.previous_time;
    level.new_session = saved.new_session;
    level.fry_sound = saved.fry_sound;
    for index in 0..saved.team_scores.len() {
        level
            .team_scores
            .set(index, saved.team_scores.get(index))
            .expect("team score");
    }
}

/// Encode a game-memory image into engine checkpoint JSON.
fn engine_json_from_memory(save: &GameMemorySave) -> SaveJson {
    engine_obj(vec![
        ("pool", SaveJson::Bytes(save.pool.clone())),
        ("allocPoint", engine_int(save.alloc_point as i64)),
    ])
}

/// Decode a game-memory image from engine checkpoint JSON.
fn memory_from_engine_json(reader: SaveReader<'_>) -> GameMemorySave {
    let pool = match reader.field("pool").value.expect("memory pool") {
        SaveJson::Bytes(bytes) => bytes.clone(),
        _ => panic!("memory pool must be bytes"),
    };
    let alloc_point = reader.field("allocPoint").integer(0).expect("memory point");
    GameMemorySave {
        pool,
        alloc_point: alloc_point as usize,
    }
}

/// Encode missile save entries into engine checkpoint JSON.
fn engine_json_from_missiles(entries: &[MissileSaveEntry]) -> SaveJson {
    engine_arr(
        entries
            .iter()
            .map(|entry| {
                let optional =
                    |value: Option<u32>| value.map_or(SaveJson::Null, |handle| engine_int(i64::from(handle)));
                engine_obj(vec![
                    ("entity", engine_int(entry.entity as i64)),
                    ("actor", engine_int(i64::from(entry.actor))),
                    ("owner", engine_int(i64::from(entry.owner))),
                    ("pass", optional(entry.pass)),
                    ("trigger", optional(entry.trigger)),
                    ("attachment", optional(entry.attachment)),
                ])
            })
            .collect(),
    )
}

/// Decode missile save entries from engine checkpoint JSON.
fn missiles_from_engine_json(reader: SaveReader<'_>) -> Vec<MissileSaveEntry> {
    reader
        .list(|entry| {
            let optional = |field: &str| -> Result<Option<u32>, qa_world::WorldError> {
                entry.field(field).nullable(|value| Ok(value.integer(0)? as u32))
            };
            Ok::<_, qa_world::WorldError>(MissileSaveEntry {
                entity: entry.field("entity").integer(0)? as usize,
                actor: entry.field("actor").integer(0)? as u32,
                owner: entry.field("owner").integer(0)? as u32,
                pass: optional("pass")?,
                trigger: optional("trigger")?,
                attachment: optional("attachment")?,
            })
        })
        .expect("missile entries")
}

/// Encode a saved actor id into engine checkpoint JSON.
fn engine_json_from_saved_actor(actor: &SavedActorId) -> SaveJson {
    engine_obj(vec![
        ("slot", engine_int(i64::from(actor.slot))),
        ("generation", engine_int(i64::from(actor.generation))),
    ])
}

/// Resolve a saved actor id against live records by slot and generation.
fn resolve_saved_actor(records: &Q3EntityRecords, saved: &SavedActorId) -> Option<ActorId> {
    for slot in 0..MAX_GENTITIES {
        let native = records.get(slot)?;
        let id = native.borrow().binding.actor().id().clone();
        if id.slot() == saved.slot && id.generation() == saved.generation {
            return Some(id);
        }
    }
    None
}

/// Encode a spawn report into engine checkpoint JSON.
fn engine_json_from_report(report: &SpawnReport) -> SaveJson {
    let outcomes = report
        .outcomes
        .iter()
        .map(|outcome| match outcome {
            SpawnOutcome::Dispatched { route, slot, classname } => engine_obj(vec![
                ("kind", engine_str("dispatched")),
                (
                    "route",
                    engine_str(match route {
                        SpawnRoute::Item => "item",
                        SpawnRoute::Handler => "handler",
                    }),
                ),
                ("slot", engine_int(*slot as i64)),
                ("classname", engine_str(classname)),
            ]),
            SpawnOutcome::Filtered { slot, reason } => engine_obj(vec![
                ("kind", engine_str("filtered")),
                ("slot", engine_int(*slot as i64)),
                (
                    "reason",
                    engine_str(match reason {
                        SpawnFilter::Notsingle => "notsingle",
                        SpawnFilter::Notteam => "notteam",
                        SpawnFilter::Notfree => "notfree",
                        SpawnFilter::Notta => "notta",
                        SpawnFilter::Notq3a => "notq3a",
                        SpawnFilter::Gametype => "gametype",
                    }),
                ),
            ]),
            SpawnOutcome::Unknown { slot, classname } => engine_obj(vec![
                ("kind", engine_str("unknown")),
                ("slot", engine_int(*slot as i64)),
                (
                    "classname",
                    classname.as_ref().map_or(SaveJson::Null, |name| engine_str(name)),
                ),
            ]),
        })
        .collect();
    engine_obj(vec![
        (
            "worldVariables",
            engine_arr(
                report
                    .world_variables
                    .entries
                    .iter()
                    .map(|pair| engine_obj(vec![("key", engine_str(&pair.key)), ("value", engine_str(&pair.value))]))
                    .collect(),
            ),
        ),
        ("outcomes", engine_arr(outcomes)),
    ])
}

/// Decode a spawn report from engine checkpoint JSON.
fn report_from_engine_json(reader: SaveReader<'_>) -> SpawnReport {
    let pairs: Vec<SpawnPair> = reader
        .field("worldVariables")
        .list(|pair| {
            Ok::<_, qa_world::WorldError>(SpawnPair {
                key: pair.field("key").string()?,
                value: pair.field("value").string()?,
            })
        })
        .expect("report variables");
    let character_count: usize = pairs.iter().map(|pair| pair.key.len() + pair.value.len() + 2).sum();
    let outcomes: Vec<SpawnOutcome> = reader
        .field("outcomes")
        .list(|entry| {
            let kind = entry.field("kind").choice_str(&["dispatched", "filtered", "unknown"])?;
            let slot = entry.field("slot").integer(0)? as usize;
            if kind == "dispatched" {
                let route = entry.field("route").choice_str(&["item", "handler"])?;
                return Ok::<_, qa_world::WorldError>(SpawnOutcome::Dispatched {
                    route: if route == "item" {
                        SpawnRoute::Item
                    } else {
                        SpawnRoute::Handler
                    },
                    slot,
                    classname: entry.field("classname").string()?,
                });
            }
            if kind == "filtered" {
                let reason = entry.field("reason").choice_str(&[
                    "notsingle",
                    "notteam",
                    "notfree",
                    "notta",
                    "notq3a",
                    "gametype",
                ])?;
                return Ok::<_, qa_world::WorldError>(SpawnOutcome::Filtered {
                    slot,
                    reason: match reason.as_str() {
                        "notsingle" => SpawnFilter::Notsingle,
                        "notteam" => SpawnFilter::Notteam,
                        "notfree" => SpawnFilter::Notfree,
                        "notta" => SpawnFilter::Notta,
                        "notq3a" => SpawnFilter::Notq3a,
                        _ => SpawnFilter::Gametype,
                    },
                });
            }
            Ok::<_, qa_world::WorldError>(SpawnOutcome::Unknown {
                slot,
                classname: entry.field("classname").nullable(|value| value.string())?,
            })
        })
        .expect("report outcomes");
    SpawnReport {
        world_variables: SpawnVariables {
            entries: pairs,
            character_count,
        },
        outcomes,
    }
}

/// Records adapter implementing the save-state records surface.
struct SaveRecords<'q> {
    records: &'q Q3EntityRecords,
    actors: Rc<dyn Q3SessionActors>,
    item_count: usize,
}

impl SaveQ3EntityRecords for SaveRecords<'_> {
    fn product(&self) -> Product {
        self.records.product()
    }

    fn item_count(&self) -> usize {
        self.item_count
    }

    fn capture_ownership(&self) -> Vec<SaveOwnershipEntry> {
        self.records
            .capture_ownership()
            .into_iter()
            .map(|entry| SaveOwnershipEntry {
                actor: entry.actor.map(|owned| owned.id().clone()),
                active: entry.active,
                borrowed: entry.borrowed,
            })
            .collect()
    }

    fn restore_ownership(&mut self, entries: Vec<SaveOwnershipEntry>) {
        let states: Vec<SlotOwnership> = entries
            .into_iter()
            .map(|entry| SlotOwnership {
                actor: entry
                    .actor
                    .map(|id| self.actors.resolve_owned(&id).expect("restored actor")),
                active: entry.active,
                borrowed: entry.borrowed,
            })
            .collect();
        self.records.restore_ownership(&states).expect("restore ownership");
    }

    fn capture_client_backing(&self, slot: usize) -> SaveClientBacking {
        let snapshot = self.records.capture_client_backing(slot);
        SaveClientBacking {
            source_stats: snapshot.source_stats.to_vec(),
            special_ammo: snapshot.special_ammo.to_vec(),
        }
    }

    fn restore_client_backing(&mut self, slot: usize, backing: &SaveClientBacking) {
        let mut source_stats = [0; 16];
        let mut special_ammo = [0; 16];
        for (index, value) in backing.source_stats.iter().take(16).enumerate() {
            source_stats[index] = *value;
        }
        for (index, value) in backing.special_ammo.iter().take(16).enumerate() {
            special_ammo[index] = *value;
        }
        self.records
            .restore_client_backing(
                slot,
                &ClientBackingSnapshot {
                    source_stats,
                    special_ammo,
                },
            )
            .expect("restore client backing");
    }

    fn damage_inflictor(&self, actor: &ActorId) -> StateParticipant {
        match self.records.damage_inflictor(Some(actor)) {
            RecordsDamageParticipant::Native(native) => StateParticipant::Entity(native.borrow().slot),
            RecordsDamageParticipant::SharedActor(shared) => StateParticipant::SharedActor(shared.actor.clone()),
        }
    }

    fn restore_callbacks(&mut self) {
        self.records.restore_callbacks();
    }
}

/// Actor-registry adapter implementing the save-state actor surface.
struct SaveActors {
    actors: Rc<dyn Q3SessionActors>,
    records: Q3EntityRecords,
    provider: ProviderId,
}

impl SaveQ3ActorRegistry for SaveActors {
    fn resolve_saved(&self, saved: &SavedActorId) -> Option<ActorId> {
        for slot in 0..MAX_GENTITIES {
            let Some(native) = self.records.get(slot) else {
                continue;
            };
            let id = native.borrow().binding.actor().id().clone();
            if id.slot() == saved.slot && id.generation() == saved.generation {
                return Some(id);
            }
        }
        None
    }

    fn reference_saved(&self, saved: &SavedActorId) -> ActorId {
        if let Some(id) = self.resolve_saved(saved) {
            return id;
        }
        self.actors
            .allocate_at_source(&self.provider, saved.slot as usize, "q3:restored")
            .id()
            .clone()
    }
}

/// Build the death runtime over shared pools and deferred match/commands links.
#[allow(clippy::too_many_arguments)]
fn build_death(
    options: &Q3SourceOptions,
    host: &Rc<dyn Q3SourceHost>,
    level: &Rc<RefCell<GameLevel>>,
    settings: &Rc<Q3GameSettings>,
    base_pool: &Rc<RefCell<BaseEntityPool>>,
    team_pool: &PoolRef,
    items_pool: &Rc<RefCell<ItemsEntityPool>>,
    base_random: &Rc<RefCell<GameRandomMirror>>,
    item_table: &Rc<EntitiesItemTable>,
    lifecycle: &Rc<RefCell<ItemLifecycleContext>>,
    team: &Rc<TeamRuntime>,
    driver: &Rc<RefCell<RuntimeDriver>>,
    world: &Rc<RuntimeQ3World>,
    records: &Q3EntityRecords,
    missiles: &Rc<RefCell<MissileRuntime>>,
    weapons: &Rc<RefCell<WeaponRuntime>>,
    core_links: &Rc<RefCell<CoreLinks>>,
    match_cell: &Rc<RefCell<Option<Rc<MatchRuntime>>>>,
    commands_cell: &Rc<RefCell<Option<Rc<GameCommandRuntime>>>>,
) -> Rc<DeathRuntime> {
    let death_scores = Rc::new(RefCell::new([0, 0, 0, 0]));
    let callbacks = lifecycle.borrow().callbacks.clone().expect("lifecycle callbacks");
    let flag_team = team.clone();
    let flag_pool = team_pool.clone();
    let check_team = team.clone();
    let check_pool = team_pool.clone();
    let frame_level = level.clone();
    let frame_settings = settings.clone();
    let ranks_match = match_cell.clone();
    let board_commands = commands_cell.clone();
    let board_pool = team_pool.clone();
    let bonus_team = team.clone();
    let bonus_pool = team_pool.clone();
    let flag_return = team.clone();
    let drop_pools = (base_pool.clone(), team_pool.clone(), items_pool.clone());
    let drop_table = item_table.clone();
    let drop_product = options.product;
    let drop_settings = settings.clone();
    let drop_team = team.clone();
    let drop_random = base_random.clone();
    let launch_pools = (base_pool.clone(), team_pool.clone(), items_pool.clone());
    let launch_table = item_table.clone();
    let launch_product = options.product;
    let launch_settings = settings.clone();
    let launch_team = team.clone();
    let missionpack = options.product == Product::Missionpack;
    let obelisk_pool = team_pool.clone();
    let obelisk_base = base_pool.clone();
    let hook_missiles = missiles.clone();
    let hook_items = items_pool.clone();
    let hook_base = base_pool.clone();
    let hook_team = team_pool.clone();
    let hook_product = options.product;
    let kamikaze_weapons = weapons.clone();
    let kamikaze_pool = base_pool.clone();
    let kamikaze_driver = driver.clone();
    let kamikaze_links = core_links.clone();
    let cube_settings = settings.clone();
    let host_checkpoint = host.death_animations().capture();
    #[allow(clippy::cast_possible_wrap)]
    let death_checkpoint = DeathAnimationCheckpoint {
        version: host_checkpoint.version as i32,
        index: host_checkpoint.index as i32,
    };
    let mut death_animations = DeathAnimationSequence::new();
    death_animations.restore(death_checkpoint);
    let death_animations = Rc::new(RefCell::new(death_animations));
    Rc::new(DeathRuntime::new(DeathHost {
        character_death_selected: true,
        death_animations,
        pool: base_pool.clone(),
        world: server_world_mirror(world, records),
        random: base_random.clone(),
        team_scores: death_scores,
        missiles_hook_free: Rc::new(move |entity| {
            let slot = entity.borrow().slot;
            sync_entities_to_items(&hook_base.borrow(), &mut hook_items.borrow_mut(), hook_product, slot);
            hook_missiles
                .borrow_mut()
                .hook_free(&mut hook_items.borrow_mut(), slot)
                .expect("hook free");
            sync_items_to_entities_team(&hook_items.borrow(), &hook_base, &hook_team, hook_product, slot);
        }),
        items: DeathItemCallbacks {
            touch_item: callbacks.touch,
            dropped_flag_think: Rc::new(move |entity| {
                flag_team.dropped_flag_think(&flag_pool.at(entity.borrow().slot));
            }),
            check_dropped_team_item: Rc::new(move |entity| {
                check_team.check_dropped_item(&check_pool.at(entity.borrow().slot));
            }),
        },
        frame: Rc::new(move || {
            let borrowed = frame_level.borrow();
            DeathFrame {
                time: borrowed.base.time,
                game_type: frame_settings.integer("g_gametype"),
                warmup_time: borrowed.base.warmup_time,
                intermission_time: borrowed.base.intermission_time,
                blood: frame_settings.integer("com_blood") != 0,
            }
        }),
        calculate_ranks: Rc::new(move || {
            ranks_match.borrow().clone().expect("match runtime").calculate_ranks();
        }),
        send_scoreboard: Rc::new(move |entity| {
            board_commands
                .borrow()
                .clone()
                .expect("commands runtime")
                .scoreboard(&board_pool.at(entity.borrow().slot));
        }),
        log: Rc::new({
            let engine = host.engine();
            move |text| engine.log(&text)
        }),
        team_frag_bonuses: Rc::new(move |target, attacker| {
            bonus_team.frag_bonuses(
                &bonus_pool.at(target.borrow().slot),
                attacker
                    .as_ref()
                    .map(|entity| bonus_pool.at(entity.borrow().slot))
                    .as_ref(),
            );
        }),
        return_flag: Rc::new(move |team_code| flag_return.return_flag(team_code)),
        product: options.product,
        missionpack: missionpack.then(|| MissionpackDeath {
            neutral_obelisk: Rc::new(move || {
                for slot in 0..obelisk_pool.num_entities() {
                    let entity = obelisk_pool.at(slot);
                    if entity.borrow().classname().as_deref() == Some("team_neutralobelisk") {
                        return Some(obelisk_base.borrow().at(slot as i32));
                    }
                }
                None
            }),
            cube_timeout_seconds: Rc::new(move || cube_settings.integer("g_cubeTimeout")),
            start_kamikaze: Rc::new(move |entity| {
                let slot = entity.borrow().slot;
                sync_entities_to_state(&kamikaze_pool.borrow(), kamikaze_driver.borrow_mut().state_mut(), slot);
                kamikaze_weapons
                    .borrow()
                    .start_kamikaze(&mut *kamikaze_driver.borrow_mut(), slot)
                    .expect("kamikaze");
                reconcile_and_sync_state_to_entities(&kamikaze_pool, &kamikaze_driver, slot);
                reconcile_spawn_union(&kamikaze_links);
                drain_pending_targets(&kamikaze_links);
            }),
        }),
        drop_item: Rc::new(
            move |item_drop: &SimItemDrop, entity, item: EntitiesItemDefinition, angle: f32| {
                let (base, team_entities, items) = &drop_pools;
                let borrowed = base.borrow();
                let slot = entity.borrow().slot;
                let table = RuntimeItemsTable {
                    table: drop_table.clone(),
                };
                let mut check_dropped = |pool: &mut ItemsEntityPool, slot: Slot| -> Q3GameItemsResult<()> {
                    drop_team.check_dropped_item(&team_entities.at(slot));
                    let _ = pool;
                    Ok(())
                };
                let mut random_draw = || drop_random.borrow_mut().random_value();
                let mut context = ItemsDropItemContext {
                    launch: ItemsLaunchItemContext {
                        product: drop_product,
                        game_type: drop_settings.integer("g_gametype"),
                        time: item_drop.time,
                        items: &table,
                        check_dropped_team_item: &mut check_dropped,
                    },
                    random: &mut random_draw,
                };
                let items_def = items_core_item(drop_product, &item);
                let dropped = items_core_drop_item(&mut items.borrow_mut(), slot, &mut context, &items_def, angle)
                    .expect("death drop");
                drop(borrowed);
                reconcile_pools(base, items, team_entities);
                sync_items_to_entities_team(&items.borrow(), base, team_entities, drop_product, dropped);
                base.borrow().at(dropped as i32)
            },
        ),
        launch_item: Rc::new(
            move |drop: &SimItemDrop, item: EntitiesItemDefinition, origin, velocity| {
                let (base, team_entities, items) = &launch_pools;
                let table = RuntimeItemsTable {
                    table: launch_table.clone(),
                };
                let mut check_dropped = |pool: &mut ItemsEntityPool, slot: Slot| -> Q3GameItemsResult<()> {
                    launch_team.check_dropped_item(&team_entities.at(slot));
                    let _ = pool;
                    Ok(())
                };
                let mut context = ItemsLaunchItemContext {
                    product: launch_product,
                    game_type: launch_settings.integer("g_gametype"),
                    time: drop.time,
                    items: &table,
                    check_dropped_team_item: &mut check_dropped,
                };
                let items_def = items_core_item(launch_product, &item);
                let launched =
                    items_core_launch_item(&mut items.borrow_mut(), &mut context, &items_def, origin, velocity)
                        .expect("death launch");
                reconcile_pools(base, items, team_entities);
                sync_items_to_entities_team(&items.borrow(), base, team_entities, launch_product, launched);
                base.borrow().at(launched as i32)
            },
        ),
        item_table: item_table.clone(),
    }))
}

/// Server-world function mirror over the shared world adapter.
fn server_world_mirror(world: &Rc<RuntimeQ3World>, records: &Q3EntityRecords) -> ServerWorldMirror {
    let trace_world = world.clone();
    let actor_world = world.clone();
    let contents_world = world.clone();
    let link_world = world.clone();
    let link_records = records.clone();
    let unlink_world = world.clone();
    ServerWorldMirror {
        trace: Rc::new(move |query| ServerWorld::trace(&*trace_world, query)),
        trace_actor: Rc::new(move |query| ActorSpatialQueries::trace_actor(actor_world.as_ref(), query)),
        point_contents: Rc::new(move |point, pass| ServerWorld::point_contents(&*contents_world, point, pass)),
        link: Rc::new(move |entity| {
            link_world
                .adapter
                .link(link_records.get(entity.borrow().slot).expect("mirror link"));
        }),
        unlink: Rc::new(move |slot| ServerWorld::unlink(&*unlink_world, slot)),
    }
}

/// Register the item touch/respawn callbacks under their save-stable names.
fn register_lifecycle_callbacks(
    base_pool: &Rc<RefCell<BaseEntityPool>>,
    _records: &Q3EntityRecords,
    lifecycle: &Rc<RefCell<ItemLifecycleContext>>,
) {
    let touch_lifecycle = lifecycle.clone();
    let touch: EntitiesTouchCallback = Rc::new(move |entity, other, contact| {
        lifecycle_touch_item(&entity, other, &contact, &touch_lifecycle.borrow());
    });
    let respawn_lifecycle = lifecycle.clone();
    let respawn: EntitiesThinkCallback = Rc::new(move |entity| {
        lifecycle_respawn_item(&entity, &respawn_lifecycle.borrow());
    });
    lifecycle.borrow_mut().callbacks = Some(ItemLifecycleCallbacks {
        touch: touch.clone(),
        respawn: respawn.clone(),
    });
    base_pool
        .borrow()
        .callbacks
        .borrow_mut()
        .touch
        .register("q3.item.touch", touch);
    base_pool
        .borrow()
        .callbacks
        .borrow_mut()
        .think
        .register("q3.item.respawn", respawn);
}

/// Team-pool rankings sink reporting into the base-pool ranking reports.
struct RuntimeRankings {
    pool: Rc<RefCell<BaseEntityPool>>,
}

impl RankingsHost for RuntimeRankings {
    fn use_holdable(&self, slot: usize, holdable: i32) {
        #[allow(clippy::cast_possible_wrap)]
        self.pool.borrow().rankings.borrow().use_holdable(slot as i32, holdable);
    }

    fn capture(&self, slot: usize) {
        #[allow(clippy::cast_possible_wrap)]
        self.pool.borrow().rankings.borrow().capture(slot as i32);
    }

    fn pickup_powerup(&self, slot: usize, powerup: i32) {
        #[allow(clippy::cast_possible_wrap)]
        self.pool
            .borrow()
            .rankings
            .borrow()
            .pickup_powerup(slot as i32, powerup);
    }
}
