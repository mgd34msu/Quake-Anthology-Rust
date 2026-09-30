//! Quake III base/game: item motion.
//!
//! Donor provenance: `src/content/q3/base/game/item-motion.ts`.

use qa_core::math::{add3, angle_vectors, dot3, scale3, vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::items_core::*;
use crate::q3::base::game::state::GameFlags;
use crate::q3::base::shared::definitions::{EntityType, GameType, ItemType, Product};
use crate::q3::base::shared::player_state::ENTITYNUM_WORLD;
use crate::q3::base::shared::trajectory::{evaluate_trajectory, evaluate_trajectory_delta, Trajectory, TrajectoryType};
use crate::q3::base::world::{TraceShape, TraceSolidity};

// ---------------------------------------------------------------------------
// item-motion.ts: G_BounceItem, G_RunItem, LaunchItem, Drop_Item
// ---------------------------------------------------------------------------

pub(crate) const ITEM_RADIUS: f32 = 15.0;

pub(crate) const CONTENTS_SOLID: i32 = 0x1;

pub(crate) const CONTENTS_PLAYERCLIP: i32 = 0x10000;

pub(crate) const CONTENTS_TRIGGER: i32 = 0x4000_0000;

pub(crate) const CONTENTS_NODROP: i32 = i32::MIN;

pub(crate) const EF_BOUNCE_HALF: i32 = 0x20;

pub(crate) const DROPPED_ITEM_LIFETIME: i32 = 30_000;

/// Item touch callback name (`q3.item.touch`).
pub const ITEM_TOUCH: &str = "q3.item.touch";

/// Dropped-flag think name (`q3.item.droppedFlag`).
pub const DROPPED_FLAG_THINK: &str = "q3.item.droppedFlag";

/// Dropped-item expiry think name.
pub const LAUNCH_ITEM_THINK: &str = "q3.base.game.item-motion.launchItem.think";

/// Frame times (`ItemFrameTime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemFrameTime {
    /// Current time.
    pub time: i32,
    /// Previous time.
    pub previous_time: i32,
}

/// Run-item context (`RunItemContext`).
pub struct RunItemContext<'a> {
    /// Current time.
    pub time: i32,
    /// Previous time.
    pub previous_time: i32,
    /// World host.
    pub world: &'a mut dyn WorldOps,
    /// Team-item free (`Team_FreeEntity`).
    pub free_team_entity: &'a mut dyn FnMut(&mut EntityPool, Slot) -> Q3GameItemsResult<()>,
    /// Think dispatch.
    pub think: &'a mut dyn FnMut(&mut EntityPool, Slot, CallbackName) -> Q3GameItemsResult<()>,
}

/// Launch-item context (`LaunchItemContext`).
pub struct LaunchItemContext<'a> {
    /// Product.
    pub product: Product,
    /// Game type.
    pub game_type: i32,
    /// Current time.
    pub time: i32,
    /// Item table.
    pub items: &'a dyn ItemTable,
    /// Dropped team-item check (`Team_CheckDroppedItem`).
    pub check_dropped_team_item: &'a mut dyn FnMut(&mut EntityPool, Slot) -> Q3GameItemsResult<()>,
}

/// Drop-item context (`DropItemContext`).
pub struct DropItemContext<'a> {
    /// Launch context.
    pub launch: LaunchItemContext<'a>,
    /// Game random in [0, 1].
    pub random: &'a mut dyn FnMut() -> f32,
}

pub(crate) fn collision_normal(trace: &ActorTraceResult) -> Vec3 {
    match trace.contact {
        TraceContact::Plane { normal } => normal,
        TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

/// Reflect one item trajectory at a trace impact (`bounceItem`).
pub fn bounce_item(
    pool: &mut EntityPool,
    slot: Slot,
    trace: &ActorTraceResult,
    frame: ItemFrameTime,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let elapsed = frame.time.wrapping_sub(frame.previous_time);
    let hit_time = qvm_float_to_int(frame.previous_time as f32 + elapsed as f32 * trace.fraction);
    let velocity = evaluate_trajectory_delta(&pool.entities[slot].s.pos, hit_time);
    let normal = collision_normal(trace);
    let dot = dot3(velocity, normal);
    let reflected = add3(velocity, scale3(normal, -2.0 * dot));
    let delta = scale3(reflected, pool.entities[slot].physics_bounce);
    pool.entities[slot].s.pos.delta = delta;

    if normal.z > 0.0 && delta.z < 40.0 {
        let stopped = vec3(
            qvm_float_to_int(trace.end.x) as f32,
            qvm_float_to_int(trace.end.y) as f32,
            qvm_float_to_int(trace.end.z + 1.0) as f32,
        );
        set_origin(pool, slot, stopped)?;
        trace_ground(pool, slot, trace.hit)?;
        return Ok(());
    }

    let origin = add3(pool.entities[slot].r.current_origin, normal);
    let entity = &mut pool.entities[slot];
    entity.r.current_origin = origin;
    entity.s.pos.base = origin;
    entity.s.pos.time = frame.time;
    Ok(())
}

/// Advance, link, think, remove, or bounce an item (`runItem`).
pub fn run_item(pool: &mut EntityPool, slot: Slot, context: &mut RunItemContext<'_>) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    if pool.entities[slot].s.ground_entity_num == -1
        && pool.entities[slot].s.pos.trajectory_type != TrajectoryType::TrGravity
    {
        let entity = &mut pool.entities[slot];
        entity.s.pos.trajectory_type = TrajectoryType::TrGravity;
        entity.s.pos.time = context.time;
    }

    if pool.entities[slot].s.pos.trajectory_type == TrajectoryType::TrStationary {
        run_think(pool, slot, context.time, context.think)?;
        return Ok(());
    }

    let destination = evaluate_trajectory(&pool.entities[slot].s.pos, context.time);
    let owner_num = pool.entities[slot].r.owner_num;
    let pass_actor = if !(0..ENTITYNUM_WORLD).contains(&owner_num) {
        None
    } else {
        pool.get(owner_num as usize).map(|owner| ActorId::from_slot(owner.slot))
    };
    let entity = &pool.entities[slot];
    let mask = if entity.clipmask != 0 {
        entity.clipmask
    } else {
        CONTENTS_SOLID | CONTENTS_PLAYERCLIP
    };
    let trace = context.world.trace_actor(
        pool,
        &ActorTraceQuery {
            start: entity.r.current_origin,
            end: destination,
            shape: TraceShape::Box {
                mins: entity.r.mins,
                maxs: entity.r.maxs,
            },
            pass_actor,
            mask,
        },
    );
    pool.entities[slot].r.current_origin = trace.end;
    pool.link(slot)?;
    run_think(pool, slot, context.time, context.think)?;

    let fraction = if trace.solidity == TraceSolidity::Clear {
        trace.fraction
    } else {
        0.0
    };
    if fraction == 1.0 {
        return Ok(());
    }
    let origin = pool.entities[slot].r.current_origin;
    if context.world.point_contents(pool, origin, -1) & CONTENTS_NODROP != 0 {
        let is_team = pool.entities[slot]
            .item
            .as_ref()
            .is_some_and(|item| item.item_type() == ItemType::ItTeam);
        if is_team {
            (context.free_team_entity)(pool, slot)?;
        } else {
            pool.free(slot)?;
        }
        return Ok(());
    }
    let mut trace = trace;
    trace.fraction = fraction;
    bounce_item(
        pool,
        slot,
        &trace,
        ItemFrameTime {
            time: context.time,
            previous_time: context.previous_time,
        },
    )
}

pub(crate) fn is_special_team_drop(product: Product, game_type: i32, item: &ItemDefinition) -> bool {
    if item.item_type() != ItemType::ItTeam {
        return false;
    }
    game_type == GameType::GtCtf as i32 || (product == Product::Missionpack && game_type == GameType::Gt1fctf as i32)
}

/// Create and link one dropped item (`launchItem`).
pub fn launch_item(
    pool: &mut EntityPool,
    context: &mut LaunchItemContext<'_>,
    item: &ItemDefinition,
    origin: Vec3,
    velocity: Vec3,
) -> Q3GameItemsResult<Slot> {
    bind_launch_save_callbacks(pool);
    if pool.product() != context.product {
        return Err(invalid("item motion product does not match its entity pool"));
    }
    let index = context.items.index_of(context.product, item).unwrap_or(0);
    if index < 1 {
        return Err(range("launched item does not belong to the selected product table"));
    }
    let slot = pool.spawn()?;
    let touch = pool.touch_cbs.resolve(ITEM_TOUCH)?;
    let entity = &mut pool.entities[slot];
    entity.s.e_type = EntityType::EtItem as i32;
    entity.s.modelindex = index as i32;
    entity.s.modelindex2 = 1;
    entity.classname = item.class_name.clone();
    entity.item = Some(item.clone());
    entity.r.mins = vec3(-ITEM_RADIUS, -ITEM_RADIUS, -ITEM_RADIUS);
    entity.r.maxs = vec3(ITEM_RADIUS, ITEM_RADIUS, ITEM_RADIUS);
    entity.r.contents = CONTENTS_TRIGGER;
    entity.touch = Some(touch);
    entity.s.e_flags |= EF_BOUNCE_HALF;
    entity.nextthink = context.time.wrapping_add(DROPPED_ITEM_LIFETIME);
    if is_special_team_drop(context.product, context.game_type, item) {
        let think = pool.think_cbs.resolve(DROPPED_FLAG_THINK)?;
        pool.entities[slot].think = Some(think);
        (context.check_dropped_team_item)(pool, slot)?;
    } else {
        let think = pool.think_cbs.resolve(LAUNCH_ITEM_THINK)?;
        pool.entities[slot].think = Some(think);
    }
    pool.entities[slot].flags = GameFlags::DROPPED_ITEM;
    set_origin(pool, slot, origin)?;
    pool.entities[slot].s.pos = Trajectory {
        trajectory_type: TrajectoryType::TrGravity,
        time: context.time,
        duration: 0,
        base: origin,
        delta: velocity,
    };
    pool.link(slot)?;
    Ok(slot)
}

/// Toss an item forward from an entity (`dropItem`).
pub fn drop_item(
    pool: &mut EntityPool,
    entity: Slot,
    context: &mut DropItemContext<'_>,
    item: &ItemDefinition,
    angle: f32,
) -> Q3GameItemsResult<Slot> {
    pool.require_owned(entity)?;
    let source = &pool.entities[entity];
    let forward = angle_vectors(vec3(0.0, source.s.apos.base.y + angle, source.s.apos.base.z)).forward;
    let horizontal = scale3(forward, 150.0);
    let random = (context.random)();
    if !random.is_finite() || random < 0.0 || random > 1.0 {
        return Err(range("Drop_Item random value must be within [0, 1]"));
    }
    let crandom = 2.0 * (random - 0.5);
    let lift = 200.0 + crandom * 50.0;
    let velocity = vec3(horizontal.x, horizontal.y, horizontal.z + lift);
    let base = pool.entities[entity].s.pos.base;
    launch_item(pool, &mut context.launch, item, base, velocity)
}

/// Register launch save callbacks (`bindLaunchSaveCallbacks`).
pub fn bind_launch_save_callbacks(pool: &mut EntityPool) {
    pool.touch_cbs.intern(ITEM_TOUCH);
    pool.think_cbs.intern(DROPPED_FLAG_THINK);
    pool.think_cbs.intern(LAUNCH_ITEM_THINK);
}

/// Dropped-item expiry think: free the item (host dispatches [`LAUNCH_ITEM_THINK`]).
pub fn launch_item_think(pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
    pool.free(slot)
}

#[cfg(test)]
mod tests {
    use qa_core::math::vec3;

    use super::*;
    use crate::q3::base::game::items_core::test_support::*;
    use crate::q3::base::shared::definitions::*;
    use crate::q3::base::shared::trajectory::{
        evaluate_trajectory, evaluate_trajectory_delta, Trajectory, TrajectoryType,
    };
    use crate::q3::base::world::TraceSolidity;

    #[test]
    fn trajectory_evaluation_matches_donor() {
        let linear = Trajectory {
            trajectory_type: TrajectoryType::TrLinear,
            time: 1000,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(100.0, 0.0, 0.0),
        };
        assert_eq!(evaluate_trajectory(&linear, 1500), vec3(50.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&linear, 9999), vec3(100.0, 0.0, 0.0));
        let gravity = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            ..linear
        };
        let at = evaluate_trajectory(&gravity, 2000);
        assert!((at.x - 100.0).abs() < 0.001);
        assert!((at.z + 400.0).abs() < 0.5);
        let delta = evaluate_trajectory_delta(&gravity, 2000);
        assert!((delta.z + 800.0).abs() < 0.5);
        let stop = Trajectory {
            trajectory_type: TrajectoryType::TrLinearStop,
            duration: 500,
            ..linear
        };
        assert_eq!(evaluate_trajectory(&stop, 2000), vec3(50.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&stop, 2000), vec3(0.0, 0.0, 0.0));
        let sine = Trajectory {
            trajectory_type: TrajectoryType::TrSine,
            duration: 1000,
            delta: vec3(0.0, 0.0, 10.0),
            ..linear
        };
        let mid = evaluate_trajectory(&sine, 1250);
        assert!((mid.z - 10.0).abs() < 0.01, "{mid:?}");
        let still = Trajectory::zero(TrajectoryType::TrStationary);
        assert_eq!(evaluate_trajectory(&still, 4242), vec3(0.0, 0.0, 0.0));
        assert_eq!(evaluate_trajectory_delta(&still, 4242), vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn angle_vectors_match_donor() {
        let forward = angle_vectors(vec3(0.0, 90.0, 0.0)).forward;
        assert!(forward.x.abs() < 1e-5);
        assert!((forward.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn launch_and_drop_items() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Baseq3);
        let mut checked = Vec::new();
        let mut check = |pool: &mut EntityPool, slot: Slot| {
            checked.push(slot);
            let _ = pool;
            Ok(())
        };
        let mut ctx = LaunchItemContext {
            product: Product::Baseq3,
            game_type: GameType::GtFfa as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        let slot = launch_item(
            &mut pool,
            &mut ctx,
            &items.list[4],
            vec3(1.0, 2.0, 3.0),
            vec3(0.0, 0.0, 10.0),
        )
        .unwrap();
        let entity = pool.at(slot).unwrap();
        assert_eq!(entity.s.e_type, EntityType::EtItem as i32);
        assert_eq!(entity.s.modelindex, 4);
        assert_eq!(entity.think, Some(CallbackName(LAUNCH_ITEM_THINK)));
        assert_eq!(entity.nextthink, 31000);
        assert!(checked.is_empty());
        launch_item_think(&mut pool, slot).unwrap();
        assert!(pool.get(slot).is_none());
        // Team drop in CTF uses the flag think and runs the check.
        let mut checked = Vec::new();
        let mut check = |_pool: &mut EntityPool, slot: Slot| {
            checked.push(slot);
            Ok(())
        };
        let mut ctx = LaunchItemContext {
            product: Product::Baseq3,
            game_type: GameType::GtCtf as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        let flag = launch_item(
            &mut pool,
            &mut ctx,
            &items.list[11],
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0),
        )
        .unwrap();
        assert_eq!(pool.at(flag).unwrap().think, Some(CallbackName(DROPPED_FLAG_THINK)));
        assert_eq!(checked, vec![flag]);
        // Drop forward from an entity.
        let owner = player_slot(&mut pool, Product::Baseq3);
        pool.at_mut(owner).unwrap().s.apos.base = vec3(0.0, 0.0, 0.0);
        pool.at_mut(owner).unwrap().s.pos.base = vec3(5.0, 5.0, 5.0);
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut random = || 0.5f32;
        let mut drop = DropItemContext {
            launch: LaunchItemContext {
                product: Product::Baseq3,
                game_type: GameType::GtFfa as i32,
                time: 2000,
                items: &items,
                check_dropped_team_item: &mut check,
            },
            random: &mut random,
        };
        let tossed = drop_item(&mut pool, owner, &mut drop, &items.list[4], 0.0).unwrap();
        assert_eq!(pool.at(tossed).unwrap().s.pos.base, vec3(5.0, 5.0, 5.0));
        assert_eq!(pool.at(tossed).unwrap().s.pos.delta.z, 200.0);
        // Bad random is rejected.
        let mut bad_random = || f32::NAN;
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut bad = DropItemContext {
            launch: LaunchItemContext {
                product: Product::Baseq3,
                game_type: GameType::GtFfa as i32,
                time: 2000,
                items: &items,
                check_dropped_team_item: &mut check,
            },
            random: &mut bad_random,
        };
        assert!(drop_item(&mut pool, owner, &mut bad, &items.list[4], 0.0).is_err());
    }

    #[test]
    fn run_item_frames_and_bounce() {
        let mut pool = EntityPool::new(Product::Baseq3);
        // Stationary item runs think only.
        let still = pool.spawn().unwrap();
        pool.at_mut(still).unwrap().s.pos.trajectory_type = TrajectoryType::TrStationary;
        pool.at_mut(still).unwrap().nextthink = 500;
        let mut world = TestWorld::new();
        let mut freed = Vec::new();
        let mut free_team = |_pool: &mut EntityPool, slot: Slot| {
            freed.push(slot);
            Ok(())
        };
        let mut ran = Vec::new();
        let mut think = |_pool: &mut EntityPool, slot: Slot, _name: CallbackName| {
            ran.push(slot);
            Ok(())
        };
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        bind_launch_save_callbacks(&mut pool);
        pool.at_mut(still).unwrap().think = Some(CallbackName(LAUNCH_ITEM_THINK));
        run_item(&mut pool, still, &mut ctx).unwrap();
        assert_eq!(ran, vec![still]);
        // Lost support converts to gravity.
        let falling = pool.spawn().unwrap();
        pool.at_mut(falling).unwrap().s.ground_entity_num = -1;
        pool.at_mut(falling).unwrap().s.pos.trajectory_type = TrajectoryType::TrLinear;
        pool.at_mut(falling).unwrap().s.pos.delta = vec3(0.0, 0.0, 0.0);
        let mut world = TestWorld::new();
        let mut free_team = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut think = |_pool: &mut EntityPool, _slot: Slot, _name: CallbackName| Ok(());
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        run_item(&mut pool, falling, &mut ctx).unwrap();
        assert_eq!(
            pool.at(falling).unwrap().s.pos.trajectory_type,
            TrajectoryType::TrGravity
        );
        // Nodrop frees the item after a partial trace.
        let mut world = TestWorld::new();
        world.trace_result.fraction = 0.5;
        world.trace_result.end = vec3(1.0, 1.0, 1.0);
        world.contents = i32::MIN;
        let dropping = pool.spawn().unwrap();
        pool.at_mut(dropping).unwrap().s.pos.trajectory_type = TrajectoryType::TrLinear;
        let mut free_team = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut think = |_pool: &mut EntityPool, _slot: Slot, _name: CallbackName| Ok(());
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        run_item(&mut pool, dropping, &mut ctx).unwrap();
        assert!(pool.get(dropping).is_none());
    }

    #[test]
    fn bounce_settles_or_reflects() {
        let mut pool = EntityPool::new(Product::Baseq3);
        let slot = pool.spawn().unwrap();
        pool.at_mut(slot).unwrap().s.pos = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            time: 900,
            duration: 0,
            base: vec3(0.0, 0.0, 100.0),
            delta: vec3(0.0, 0.0, -10.0),
        };
        pool.at_mut(slot).unwrap().physics_bounce = 0.5;
        let trace = ActorTraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 5.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                normal: vec3(0.0, 0.0, 1.0),
            },
            contents: 1,
            surface_flags: 0,
            hit: TraceHit::World,
        };
        bounce_item(
            &mut pool,
            slot,
            &trace,
            ItemFrameTime {
                time: 1000,
                previous_time: 900,
            },
        )
        .unwrap();
        // Reflected upward velocity is small, so the item settles.
        assert_eq!(
            pool.at(slot).unwrap().s.pos.trajectory_type,
            TrajectoryType::TrStationary
        );
        assert_eq!(pool.at(slot).unwrap().r.current_origin, vec3(0.0, 0.0, 6.0));
        // A wall bounce keeps flying.
        let wall = pool.spawn().unwrap();
        pool.at_mut(wall).unwrap().s.pos = Trajectory {
            trajectory_type: TrajectoryType::TrGravity,
            time: 900,
            duration: 0,
            base: vec3(0.0, 0.0, 100.0),
            delta: vec3(100.0, 0.0, 0.0),
        };
        pool.at_mut(wall).unwrap().physics_bounce = 0.5;
        pool.at_mut(wall).unwrap().r.current_origin = vec3(10.0, 0.0, 100.0);
        let trace = ActorTraceResult {
            contact: TraceContact::Plane {
                normal: vec3(-1.0, 0.0, 0.0),
            },
            hit: TraceHit::World,
            ..trace
        };
        bounce_item(
            &mut pool,
            wall,
            &trace,
            ItemFrameTime {
                time: 1000,
                previous_time: 900,
            },
        )
        .unwrap();
        assert_eq!(pool.at(wall).unwrap().r.current_origin, vec3(9.0, 0.0, 100.0));
    }

    #[test]
    fn item_motion_errors_and_team_nodrop() {
        let items = test_items();
        let mut pool = EntityPool::new(Product::Baseq3);
        // Wrong product and foreign items are rejected.
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut ctx = LaunchItemContext {
            product: Product::Missionpack,
            game_type: GameType::GtFfa as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        assert!(launch_item(
            &mut pool,
            &mut ctx,
            &items.list[4],
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 0.0)
        )
        .is_err());
        let foreign = item_def("foreign", 1, ItemKind::Armor);
        let mut check = |_pool: &mut EntityPool, _slot: Slot| Ok(());
        let mut ctx = LaunchItemContext {
            product: Product::Baseq3,
            game_type: GameType::GtFfa as i32,
            time: 1000,
            items: &items,
            check_dropped_team_item: &mut check,
        };
        assert!(launch_item(&mut pool, &mut ctx, &foreign, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)).is_err());
        // Team items in nodrop route to the team free instead of pool free.
        let flag = pool.spawn().unwrap();
        pool.at_mut(flag).unwrap().s.pos.trajectory_type = TrajectoryType::TrLinear;
        pool.at_mut(flag).unwrap().item = Some(items.list[11].clone());
        let mut world = TestWorld::new();
        world.trace_result.fraction = 0.25;
        world.contents = i32::MIN;
        let mut freed = Vec::new();
        let mut free_team = |pool: &mut EntityPool, slot: Slot| {
            freed.push(slot);
            pool.free(slot)
        };
        let mut think = |_pool: &mut EntityPool, _slot: Slot, _name: CallbackName| Ok(());
        let mut ctx = RunItemContext {
            time: 1000,
            previous_time: 900,
            world: &mut world,
            free_team_entity: &mut free_team,
            think: &mut think,
        };
        run_item(&mut pool, flag, &mut ctx).unwrap();
        assert_eq!(freed, vec![flag]);
    }
}
