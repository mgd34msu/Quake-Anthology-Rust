//! Actor host construction over the shared scene queries.
//!
//! Port of donor `src/app/bootstrap/simulation/source-hosts.ts`
//! (`ActorHostRuntime`, `createQ1ActorHost`, `createQ2ActorHost`). The
//! donor's `SharedSceneQueries` class lives with the world lane; this
//! module extends the real [`SceneQueries`](qa_bots::scene::SceneQueries)
//! trait with the members the guest lane reads.

use std::rc::Rc;

use qa_bots::scene::{
    BspPlane as BotsBspPlane, Q2SecondaryImpact as BotsSecondaryImpact, Q2SurfaceInfo as BotsSurfaceInfo,
    TraceContact as BotsTraceContact,
};
use qa_bots::scene::{
    LeafContents, PointContentsQuery, PointContentsResult, Q1MoveRule, QueryTarget, SceneQueries, TraceDetail,
    TraceHit as BotsTraceHit, TracePolicy, TraceQuery, TraceResult as BotsTraceResult, TraceShape, VisibilityKind,
};
use qa_content::monsters::MonsterTargetObservation;
use qa_content::q1::foundation::gameplay::SourceDamageModifier;
use qa_content::q1::foundation::host::{
    Q1ActorCallbackTable, Q1CancelThinkHook, Q1ChangeYawHook, Q1CheckBottomHook, Q1CheckClientHook, Q1ClassnameHook,
    Q1Contents, Q1ContentsHook, Q1ControlPlayerHook, Q1EmitHook, Q1FoundationHost, Q1GameplayAuthority,
    Q1MonsterTargetHook, Q1MoveToGoalHook, Q1OriginalPickupPort, Q1PlayersHook, Q1PowerupExpiresHook, Q1PowerupHook,
    Q1PunchAngles, Q1RandomHook, Q1RegisterEntity, Q1ScheduleThinkHook, Q1SessionActorRegistry, Q1SetGravityHook,
    Q1SharedBodyTable, Q1SharedInventoryTable, Q1SourceDamageMultiplierHook, Q1SourceTargetHook, Q1StepPusherHook,
    Q1TraceHook, Q1TransitionHook, Q1WalkMoveHook, Q1WeaponBehaviorPort, Q1WeaponImpactHook, Q1WeaponVolumeHook,
};
use qa_content::q1::foundation::types::{Q1Trace, Q1TraceRequest};
use qa_content::q2::foundation::host::{
    Q2FoundationHost, Q2LandmarkCarry, Q2Motion, Q2OriginalPickups, Q2PlayerViewState, Q2Solid, Q2TraceRequest,
    Q2WeaponTarget,
};
use qa_content::q2::support::contracts::{
    Q2BspPlane, Q2SecondaryPlane, Q2SurfaceInfo, Q2TraceFields, TraceContact, TraceFamily, TraceHit, TraceResult,
    TransitionIntent, WeaponBehaviorProjectilePort,
};
use qa_content::q2::support::misc::Q2RereleaseRandomSource;
use qa_content::q2::support::tables::{
    Q2ActorRegistry, Q2BodyTable, Q2CallbackTable, Q2CombatAuthority, Q2InventoryTable,
};
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericProfile;
use qa_world::spatial::SpatialActor;

use super::random::SourceRandom;

/// Source clock reading in seconds.
pub type ActorHostClock = Box<dyn Fn() -> f64>;
/// Actor think scheduler.
pub type ActorHostSchedule = Box<dyn FnMut(&OwnedActor, Option<f64>)>;
/// Source-stable actor ordering, as a donor comparator.
pub type ActorHostOrder = Rc<dyn Fn(&ActorId, &ActorId) -> i32>;

/// Actor host runtime clock, random, and scheduling.
pub struct ActorHostRuntime {
    /// Selected numeric profile.
    pub numeric: NumericProfile,
    /// Source random stream.
    pub random: SourceRandom,
    /// Current source time in seconds.
    pub now: ActorHostClock,
    /// Frame time in seconds.
    pub frame_seconds: ActorHostClock,
    /// Schedule an actor think.
    pub schedule: ActorHostSchedule,
}

/// Mirror of the `SharedSceneQueries` members the guest lane reads, from
/// donor `src/world/collision/index.ts` (canonical home:
/// `qa_world::collision`); unify post-merge.
pub trait ActorHostScene: SceneQueries {
    /// Sweep a body through collision, skipping excluded actors.
    fn trace_excluding(&self, query: &TraceQuery, excluded: &[ActorId]) -> BotsTraceResult;
    /// Sweep geometry only, skipping linked actors.
    fn geometry_trace(&self, query: &TraceQuery) -> BotsTraceResult;
    /// Leaf containing a point.
    fn point_leaf(&self, point: Vec3) -> i32;
    /// Cluster of a leaf.
    fn leaf_cluster(&self, leaf: i32) -> i32;
    /// Area of a leaf.
    fn leaf_area(&self, leaf: i32) -> i32;
    /// Bounds of an inline model.
    fn model_bounds(&self, model: i32) -> Bounds;
    /// Actors touching bounds. Mirrors `queryActors`.
    fn query_actors(&self, bounds: &Bounds, role: qa_world::spatial::QueryRole) -> Vec<SpatialActor>;
    /// Decoded model count. Mirrors `geometry.models.length`.
    fn model_count(&self) -> usize;
    /// Quake II texture records, when the geometry is Quake II. Mirrors
    /// `geometry.textureInfo` on `q2-bsp` worlds.
    fn q2_texture_info(&self) -> Option<Vec<SceneTextureInfo>>;
}

/// Mirror of one `q2-bsp` texture record from donor
/// `src/contracts/scene.ts` (canonical home: `qa_world::collision`);
/// unify post-merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneTextureInfo {
    /// Texture name.
    pub name: String,
    /// Surface flags.
    pub flags: i32,
    /// Surface value.
    pub value: i32,
}

/// World side of an actor host: scene queries plus identity ordering.
#[derive(Clone)]
pub struct ActorHostWorld {
    /// Shared scene queries.
    pub scene: Rc<dyn ActorHostScene>,
    /// Selected numeric profile.
    pub numeric: NumericProfile,
    /// World actor, if any.
    pub world_actor: Rc<dyn Fn() -> Option<ActorId>>,
    /// Source-stable actor ordering, as a donor comparator.
    pub source_order: ActorHostOrder,
}

/// `Q1FoundationHost` fields except the `trace`/`contents` world queries.
/// Mirrors donor `Omit<Q1FoundationHost, "trace" | "contents">`.
pub struct Q1ActorHostBindings {
    /// Shared actor registry.
    pub actors: Box<dyn Q1SessionActorRegistry>,
    /// Shared body table.
    pub bodies: Box<dyn Q1SharedBodyTable>,
    /// Shared callback bindings.
    pub callbacks: Box<dyn Q1ActorCallbackTable>,
    /// Shared combat authority.
    pub combat: Box<dyn Q1GameplayAuthority>,
    /// Shared inventory table.
    pub inventory: Box<dyn Q1SharedInventoryTable>,
    /// Original pickup admission.
    pub original_pickups: Option<Box<dyn Q1OriginalPickupPort>>,
    /// View punch-angle store.
    pub punch_angles: Option<Box<dyn Q1PunchAngles>>,
    /// Projectile behavior port.
    pub weapon_behavior: Option<Box<dyn Q1WeaponBehaviorPort>>,
    /// Entity registration hook.
    pub register_entity: Option<Box<dyn Q1RegisterEntity>>,
    /// Source damage modifier.
    pub source_damage_modifier: Option<SourceDamageModifier>,
    /// Source damage powerup owner.
    pub source_damage_powerup_owner: Option<ProviderId>,
    /// Unit random draw.
    pub random: Q1RandomHook,
    /// Monster step move.
    pub walk_move: Q1WalkMoveHook,
    /// Monster yaw update.
    pub change_yaw: Q1ChangeYawHook,
    /// Monster goal move.
    pub move_to_goal: Q1MoveToGoalHook,
    /// Monster ground check.
    pub check_bottom: Q1CheckBottomHook,
    /// Schedule an actor think.
    pub schedule_think: Q1ScheduleThinkHook,
    /// Cancel an actor think.
    pub cancel_think: Q1CancelThinkHook,
    /// Emit a presentation event.
    pub emit: Q1EmitHook,
    /// Request a session transition.
    pub transition: Q1TransitionHook,
    /// Admitted players.
    pub players: Q1PlayersHook,
    /// Visible client cycle.
    pub check_client: Q1CheckClientHook,
    /// Gameplay classname of an actor.
    pub classname: Q1ClassnameHook,
    /// Apply a timed powerup.
    pub powerup: Q1PowerupHook,
    /// Step a pusher.
    pub step_pusher: Q1StepPusherHook,
    /// Weapon impact effect hook.
    pub weapon_impact: Option<Q1WeaponImpactHook>,
    /// Weapon volume hook.
    pub weapon_volume: Option<Q1WeaponVolumeHook>,
    /// Monster target observation hook.
    pub monster_target: Option<Q1MonsterTargetHook>,
    /// Gravity scale hook.
    pub set_gravity: Option<Q1SetGravityHook>,
    /// Player cinematic control hook.
    pub control_player: Option<Q1ControlPlayerHook>,
    /// Source target observation hook.
    pub source_target: Option<Q1SourceTargetHook>,
    /// Powerup expiry hook.
    pub powerup_expires: Option<Q1PowerupExpiresHook>,
    /// Source damage multiplier hook.
    pub source_damage_multiplier: Option<Q1SourceDamageMultiplierHook>,
}

/// Build a Q1 foundation host whose world queries read the shared scene.
#[must_use]
pub fn create_q1_actor_host(bindings: Q1ActorHostBindings, world: ActorHostWorld) -> Q1FoundationHost {
    let trace_world = world.clone();
    let trace: Q1TraceHook = Box::new(move |request: &Q1TraceRequest| {
        let scene = &trace_world.scene;
        let result = scene.trace(&TraceQuery {
            start: request.start,
            end: request.end,
            shape: TraceShape::Box { bounds: request.bounds },
            target: QueryTarget::World,
            policy: TracePolicy::Q1 {
                move_rule: if request.missile {
                    Q1MoveRule::Missile
                } else if request.monsters {
                    Q1MoveRule::Normal
                } else {
                    Q1MoveRule::NoMonsters
                },
                hull: None,
            },
            numeric: trace_world.numeric,
            pass_actor: request.ignore.clone(),
        });
        let TraceDetail::Q1 {
            in_open,
            in_water,
            source_plane,
            surface_flags,
            ..
        } = result.detail
        else {
            panic!("Q1 actor trace returned another source representation");
        };
        let contents = scene.point_contents(&PointContentsQuery {
            point: result.end,
            target: QueryTarget::World,
            policy: TracePolicy::Q1 {
                move_rule: Q1MoveRule::Normal,
                hull: None,
            },
            numeric: trace_world.numeric,
            pass_actor: request.ignore.clone(),
        });
        Q1Trace {
            fraction: result.fraction,
            end: result.end,
            normal: source_plane.normal,
            actor: match result.hit {
                BotsTraceHit::Actor { actor } => Some(actor),
                BotsTraceHit::World { .. } => (trace_world.world_actor)(),
                BotsTraceHit::None => None,
            },
            start_solid: result.start_solid,
            all_solid: result.all_solid,
            sky: surface_flags.unwrap_or(0) & 4 != 0 || matches!(contents, PointContentsResult::Q1 { contents: -6 }),
            in_open,
            in_water,
        }
    });
    let contents_world = world.clone();
    let contents: Q1ContentsHook = Box::new(move |point: Vec3| {
        let result = contents_world.scene.point_contents(&PointContentsQuery {
            point,
            target: QueryTarget::World,
            policy: TracePolicy::Q1 {
                move_rule: Q1MoveRule::Normal,
                hull: None,
            },
            numeric: contents_world.numeric,
            pass_actor: None,
        });
        let PointContentsResult::Q1 { contents } = result else {
            panic!("Q1 actor contents returned another source representation");
        };
        match contents {
            -2 => Q1Contents::Solid,
            -3 => Q1Contents::Water,
            -4 => Q1Contents::Slime,
            -5 => Q1Contents::Lava,
            -6 => Q1Contents::Sky,
            _ => Q1Contents::Empty,
        }
    });
    Q1FoundationHost {
        actors: bindings.actors,
        bodies: bindings.bodies,
        callbacks: bindings.callbacks,
        combat: bindings.combat,
        inventory: bindings.inventory,
        original_pickups: bindings.original_pickups,
        punch_angles: bindings.punch_angles,
        weapon_behavior: bindings.weapon_behavior,
        register_entity: bindings.register_entity,
        source_damage_modifier: bindings.source_damage_modifier,
        source_damage_powerup_owner: bindings.source_damage_powerup_owner,
        random: bindings.random,
        trace,
        contents,
        walk_move: bindings.walk_move,
        change_yaw: bindings.change_yaw,
        move_to_goal: bindings.move_to_goal,
        check_bottom: bindings.check_bottom,
        schedule_think: bindings.schedule_think,
        cancel_think: bindings.cancel_think,
        emit: bindings.emit,
        transition: bindings.transition,
        players: bindings.players,
        check_client: bindings.check_client,
        classname: bindings.classname,
        powerup: bindings.powerup,
        step_pusher: bindings.step_pusher,
        weapon_impact: bindings.weapon_impact,
        weapon_volume: bindings.weapon_volume,
        monster_target: bindings.monster_target,
        set_gravity: bindings.set_gravity,
        control_player: bindings.control_player,
        source_target: bindings.source_target,
        powerup_expires: bindings.powerup_expires,
        source_damage_multiplier: bindings.source_damage_multiplier,
    }
}

/// `Q2FoundationHost` without the seven world queries (`trace`,
/// `pointContents`, `inPvs`, `inPhs`, `areasConnected`, `nearby`,
/// `inlineModelBounds`). Mirrors donor
/// `Omit<Q2FoundationHost, Q2WorldQuery>`.
pub trait Q2ActorHostBindings {
    /// Session actor registry.
    fn actors(&mut self) -> &mut dyn Q2ActorRegistry;
    /// Shared body table.
    fn bodies(&mut self) -> &mut dyn Q2BodyTable;
    /// Actor callback table.
    fn callbacks(&mut self) -> &mut dyn Q2CallbackTable;
    /// Gameplay combat authority.
    fn combat(&mut self) -> &mut dyn Q2CombatAuthority;
    /// Shared inventory table.
    fn inventory(&mut self) -> &mut dyn Q2InventoryTable;
    /// Original-pickup admission, when the engine provides one.
    fn original_pickups(&mut self) -> Option<&mut dyn Q2OriginalPickups> {
        None
    }
    /// Weapon behavior port, when the engine provides one.
    fn weapon_behavior(&mut self) -> Option<&mut dyn WeaponBehaviorProjectilePort> {
        None
    }
    /// Weapon-target override.
    fn weapon_target_override(&mut self, _actor: &ActorId) -> Option<Option<Q2WeaponTarget>> {
        None
    }
    /// Monster-target override.
    fn monster_target_override(&mut self, _actor: &ActorId) -> Option<Option<MonsterTargetObservation>> {
        None
    }
    /// Record an entity continuation.
    fn register_entity(&mut self, _actor: &OwnedActor) {}
    /// Current source time in seconds.
    fn now(&self) -> f64;
    /// Frame time in seconds.
    fn frame_seconds(&self) -> f64;
    /// Gravity.
    fn gravity(&self) -> f64;
    /// Draw a unit random number.
    fn random(&mut self) -> f64;
    /// Rerelease random stream; classic hosts omit it.
    fn rerelease_random(&mut self) -> Option<&mut dyn Q2RereleaseRandomSource> {
        None
    }
    /// Schedule an actor's bound think callback.
    fn schedule(&mut self, actor: &OwnedActor, due_seconds: Option<f64>);
    /// Touch triggers for an actor.
    fn touch_triggers(&mut self, actor: &OwnedActor);
    /// Player actors.
    fn players(&mut self) -> Vec<ActorId>;
    /// World actor.
    fn world_actor(&mut self) -> ActorId;
    /// Whether an actor is a player.
    fn is_player(&mut self, actor: &ActorId) -> bool;
    /// Whether an actor is a monster.
    fn is_monster(&mut self, actor: &ActorId) -> bool;
    /// Set solidity.
    fn set_solid(&mut self, actor: &OwnedActor, solid: Q2Solid, model: Option<i32>);
    /// Set motion.
    fn set_motion(&mut self, motion: &Q2Motion);
    /// Set an area portal.
    fn set_area_portal(&mut self, portal: i32, open: bool);
    /// Emit a presentation event.
    fn emit(&mut self, event: qa_content::q2::foundation::host::Q2PresentationEvent);
    /// Player view state.
    fn player_view_state(&mut self, player: &ActorId) -> Option<Q2PlayerViewState>;
    /// Consume a key for a player.
    fn key_consumed(&mut self, player: &ActorId);
    /// Capture campaign state before travel.
    fn prepare_level_change(&mut self, map: &str, landmark: Option<&Q2LandmarkCarry>, server_flags: i32);
    /// Propose a transition intent.
    fn transition(&mut self, intent: TransitionIntent);
    /// Emit a diagnostic.
    fn diagnostic(&mut self, message: &str);
}

/// Q2 foundation host whose world queries read the shared scene.
pub struct Q2ActorHost<B> {
    /// Engine bindings.
    pub bindings: B,
    /// World queries.
    pub world: ActorHostWorld,
}

/// Build a Q2 foundation host whose world queries read the shared scene.
#[must_use]
pub fn create_q2_actor_host<B>(bindings: B, world: ActorHostWorld) -> Q2ActorHost<B> {
    Q2ActorHost { bindings, world }
}

impl<B: Q2ActorHostBindings> Q2ActorHost<B> {
    fn visible(&self, first: Vec3, second: Vec3, kind: VisibilityKind) -> bool {
        let scene = &self.world.scene;
        scene.cluster_visible(
            scene.leaf_cluster(scene.point_leaf(first)),
            scene.leaf_cluster(scene.point_leaf(second)),
            kind,
        )
    }
}

impl<B: Q2ActorHostBindings> Q2FoundationHost for Q2ActorHost<B> {
    fn actors(&mut self) -> &mut dyn Q2ActorRegistry {
        self.bindings.actors()
    }

    fn bodies(&mut self) -> &mut dyn Q2BodyTable {
        self.bindings.bodies()
    }

    fn callbacks(&mut self) -> &mut dyn Q2CallbackTable {
        self.bindings.callbacks()
    }

    fn combat(&mut self) -> &mut dyn Q2CombatAuthority {
        self.bindings.combat()
    }

    fn inventory(&mut self) -> &mut dyn Q2InventoryTable {
        self.bindings.inventory()
    }

    fn original_pickups(&mut self) -> Option<&mut dyn Q2OriginalPickups> {
        self.bindings.original_pickups()
    }

    fn weapon_behavior(&mut self) -> Option<&mut dyn WeaponBehaviorProjectilePort> {
        self.bindings.weapon_behavior()
    }

    fn weapon_target_override(&mut self, actor: &ActorId) -> Option<Option<Q2WeaponTarget>> {
        self.bindings.weapon_target_override(actor)
    }

    fn monster_target_override(&mut self, actor: &ActorId) -> Option<Option<MonsterTargetObservation>> {
        self.bindings.monster_target_override(actor)
    }

    fn register_entity(&mut self, actor: &OwnedActor) {
        self.bindings.register_entity(actor);
    }

    fn now(&self) -> f64 {
        self.bindings.now()
    }

    fn frame_seconds(&self) -> f64 {
        self.bindings.frame_seconds()
    }

    fn gravity(&self) -> f64 {
        self.bindings.gravity()
    }

    fn random(&mut self) -> f64 {
        self.bindings.random()
    }

    fn rerelease_random(&mut self) -> Option<&mut dyn Q2RereleaseRandomSource> {
        self.bindings.rerelease_random()
    }

    fn schedule(&mut self, actor: &OwnedActor, due_seconds: Option<f64>) {
        self.bindings.schedule(actor, due_seconds);
    }

    fn touch_triggers(&mut self, actor: &OwnedActor) {
        self.bindings.touch_triggers(actor);
    }

    fn trace(&mut self, request: &Q2TraceRequest) -> TraceResult {
        let query = TraceQuery {
            start: request.start,
            end: request.end,
            shape: match request.bounds {
                None => TraceShape::Point,
                Some(bounds) => TraceShape::Box { bounds },
            },
            target: QueryTarget::World,
            policy: TracePolicy::Q2 {
                contents_mask: request.mask,
                leaf_contents: LeafContents::Merged,
            },
            numeric: self.world.numeric,
            pass_actor: request.ignore.clone(),
        };
        let result = if request.exclude.is_empty() {
            self.world.scene.trace(&query)
        } else {
            self.world.scene.trace_excluding(&query, &request.exclude)
        };
        convert_trace(result)
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        let result = self.world.scene.point_contents(&PointContentsQuery {
            point,
            target: QueryTarget::World,
            policy: TracePolicy::Q2 {
                contents_mask: -1,
                leaf_contents: LeafContents::Merged,
            },
            numeric: self.world.numeric,
            pass_actor: None,
        });
        match result {
            PointContentsResult::Q2 { merged, .. } => merged,
            _ => panic!("Q2 actor contents returned another source representation"),
        }
    }

    fn in_pvs(&mut self, first: Vec3, second: Vec3) -> bool {
        self.visible(first, second, VisibilityKind::Pvs)
    }

    fn in_phs(&mut self, first: Vec3, second: Vec3) -> bool {
        self.visible(first, second, VisibilityKind::Phs)
    }

    fn areas_connected(&mut self, first: Vec3, second: Vec3) -> bool {
        let scene = &self.world.scene;
        scene.areas_connected(
            scene.leaf_area(scene.point_leaf(first)),
            scene.leaf_area(scene.point_leaf(second)),
        )
    }

    fn nearby(&mut self, origin: Vec3, radius: f64) -> Vec<ActorId> {
        let mut actors: Vec<ActorId> = self
            .bindings
            .actors()
            .observations()
            .iter()
            .map(|value| value.id.clone())
            .filter(|actor| {
                let body = self.bindings.bodies().read(actor);
                match body {
                    None => false,
                    Some(body) => {
                        let dx = f64::from(body.origin.x - origin.x);
                        let dy = f64::from(body.origin.y - origin.y);
                        let dz = f64::from(body.origin.z - origin.z);
                        dx.hypot(dy).hypot(dz) <= radius
                    }
                }
            })
            .collect();
        actors.sort_by(|first, second| (self.world.source_order)(first, second).cmp(&0));
        actors
    }

    fn players(&mut self) -> Vec<ActorId> {
        self.bindings.players()
    }

    fn world_actor(&mut self) -> ActorId {
        self.bindings.world_actor()
    }

    fn is_player(&mut self, actor: &ActorId) -> bool {
        self.bindings.is_player(actor)
    }

    fn is_monster(&mut self, actor: &ActorId) -> bool {
        self.bindings.is_monster(actor)
    }

    fn inline_model_bounds(&mut self, model: i32) -> Bounds {
        self.world.scene.model_bounds(model)
    }

    fn set_solid(&mut self, actor: &OwnedActor, solid: Q2Solid, model: Option<i32>) {
        self.bindings.set_solid(actor, solid, model);
    }

    fn set_motion(&mut self, motion: &Q2Motion) {
        self.bindings.set_motion(motion);
    }

    fn set_area_portal(&mut self, portal: i32, open: bool) {
        self.bindings.set_area_portal(portal, open);
    }

    fn emit(&mut self, event: qa_content::q2::foundation::host::Q2PresentationEvent) {
        self.bindings.emit(event);
    }

    fn player_view_state(&mut self, player: &ActorId) -> Option<Q2PlayerViewState> {
        self.bindings.player_view_state(player)
    }

    fn key_consumed(&mut self, player: &ActorId) {
        self.bindings.key_consumed(player);
    }

    fn prepare_level_change(&mut self, map: &str, landmark: Option<&Q2LandmarkCarry>, server_flags: i32) {
        self.bindings.prepare_level_change(map, landmark, server_flags);
    }

    fn transition(&mut self, intent: TransitionIntent) {
        self.bindings.transition(intent);
    }

    fn diagnostic(&mut self, message: &str) {
        self.bindings.diagnostic(message);
    }
}

fn convert_bsp_plane(plane: BotsBspPlane) -> Q2BspPlane {
    Q2BspPlane {
        normal: plane.normal,
        distance: plane.distance,
        plane_type: plane.plane_type,
        signbits: plane.signbits,
    }
}

fn convert_surface(surface: BotsSurfaceInfo) -> Q2SurfaceInfo {
    Q2SurfaceInfo {
        name: surface.name,
        flags: surface.flags,
        value: 0,
        material: String::new(),
    }
}

fn convert_secondary(secondary: BotsSecondaryImpact) -> Q2SecondaryPlane {
    Q2SecondaryPlane {
        plane: convert_bsp_plane(secondary.plane),
        surface: secondary.surface.map(convert_surface),
    }
}

fn convert_trace(result: BotsTraceResult) -> TraceResult {
    TraceResult {
        fraction: result.fraction,
        end: result.end,
        start_solid: result.start_solid,
        all_solid: result.all_solid,
        contact: match result.contact {
            BotsTraceContact::None => TraceContact::None,
            BotsTraceContact::Plane { plane } => TraceContact::Plane { plane },
        },
        hit: match result.hit {
            BotsTraceHit::None => TraceHit::None,
            BotsTraceHit::World { model } => TraceHit::World { model },
            BotsTraceHit::Actor { actor } => TraceHit::Actor { actor },
        },
        family: match result.detail {
            TraceDetail::Q1 {
                in_open,
                in_water,
                source_plane,
                surface_flags,
                ..
            } => TraceFamily::Q1 {
                in_open,
                in_water,
                source_plane,
                surface_flags,
            },
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane,
                secondary,
            } => TraceFamily::Q2(Q2TraceFields {
                contents,
                surface: surface.map(convert_surface),
                source_plane: convert_bsp_plane(source_plane),
                secondary: secondary.map(convert_secondary),
            }),
            TraceDetail::Q3 {
                contents,
                surface_flags,
                source_plane,
            } => TraceFamily::Q3 {
                contents,
                surface_flags,
                source_plane: convert_bsp_plane(source_plane),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use qa_bots::scene::{LeafQueryResult, TraceContact as BotsContact};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Plane;
    use qa_world::spatial::QueryRole;

    use super::*;

    fn owner() -> IdentityOwner {
        IdentityOwner::create("source-hosts-test").expect("owner")
    }

    fn vec(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    struct StubScene {
        trace: BotsTraceResult,
        contents: PointContentsResult,
    }

    impl SceneQueries for StubScene {
        fn trace(&self, _query: &TraceQuery) -> BotsTraceResult {
            self.trace.clone()
        }

        fn point_contents(&self, _query: &PointContentsQuery) -> PointContentsResult {
            self.contents
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

    impl ActorHostScene for StubScene {
        fn trace_excluding(&self, query: &TraceQuery, _excluded: &[ActorId]) -> BotsTraceResult {
            self.trace(query)
        }

        fn geometry_trace(&self, query: &TraceQuery) -> BotsTraceResult {
            self.trace(query)
        }

        fn point_leaf(&self, _point: Vec3) -> i32 {
            3
        }

        fn leaf_cluster(&self, _leaf: i32) -> i32 {
            5
        }

        fn leaf_area(&self, _leaf: i32) -> i32 {
            7
        }

        fn model_bounds(&self, _model: i32) -> Bounds {
            Bounds {
                min: vec(0.0, 0.0, 0.0),
                max: vec(1.0, 1.0, 1.0),
            }
        }

        fn query_actors(&self, _bounds: &Bounds, _role: QueryRole) -> Vec<SpatialActor> {
            Vec::new()
        }

        fn model_count(&self) -> usize {
            4
        }

        fn q2_texture_info(&self) -> Option<Vec<SceneTextureInfo>> {
            None
        }
    }

    fn q1_trace_result() -> BotsTraceResult {
        BotsTraceResult {
            fraction: 0.5,
            end: vec(1.0, 2.0, 3.0),
            start_solid: false,
            all_solid: false,
            contact: BotsContact::Plane {
                plane: Plane {
                    normal: vec(0.0, 0.0, 1.0),
                    distance: 3.0,
                },
            },
            hit: BotsTraceHit::None,
            detail: TraceDetail::Q1 {
                in_open: true,
                in_water: false,
                source_plane: Plane {
                    normal: vec(0.0, 0.0, 1.0),
                    distance: 3.0,
                },
                surface_flags: Some(4),
                contents: None,
            },
        }
    }

    #[test]
    fn converts_bots_trace_to_content_trace() {
        let converted = convert_trace(q1_trace_result());
        assert_eq!(converted.fraction, 0.5);
        assert!(matches!(converted.family, TraceFamily::Q1 { .. }));
        assert!(matches!(converted.contact, TraceContact::Plane { .. }));
    }

    #[test]
    fn converts_q2_surface_without_value() {
        let mut result = q1_trace_result();
        result.detail = TraceDetail::Q2 {
            contents: 1,
            surface: Some(BotsSurfaceInfo {
                name: "rock".to_string(),
                flags: 8,
            }),
            source_plane: BotsBspPlane {
                normal: vec(1.0, 0.0, 0.0),
                distance: 0.0,
                plane_type: 0,
                signbits: 0,
            },
            secondary: None,
        };
        let converted = convert_trace(result);
        match converted.family {
            TraceFamily::Q2(fields) => {
                assert_eq!(fields.contents, 1);
                assert_eq!(fields.surface.as_ref().expect("surface").name, "rock");
            }
            other => panic!("unexpected family: {other:?}"),
        }
    }

    #[test]
    fn scene_defaults_cover_actor_host_world() {
        let scene = StubScene {
            trace: q1_trace_result(),
            contents: PointContentsResult::Q1 { contents: -6 },
        };
        assert_eq!(scene.model_count(), 4);
        assert!(scene.q2_texture_info().is_none());
        assert!(scene.areas_connected(1, 2));
        let world = ActorHostWorld {
            scene: Rc::new(scene),
            numeric: qa_core::numeric::Q2_DONOR_PROFILE,
            world_actor: Rc::new(|| None),
            source_order: Rc::new(|_, _| 0),
        };
        assert!((world.world_actor)().is_none());
        let _ = owner();
    }
}
