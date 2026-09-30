//! Quake III base/game: item motion.
//!
//! Donor provenance: `src/content/q3/base/game/item-motion.ts`.

use qa_core::math::{add3, dot3, scale3, vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_items::*;

// ---------------------------------------------------------------------------
// item-motion.ts: G_BounceItem, G_RunItem, LaunchItem, Drop_Item
// ---------------------------------------------------------------------------

pub(crate) const ITEM_RADIUS: f32 = 15.0;

pub(crate) const CONTENTS_SOLID: i32 = 0x1;

pub(crate) const CONTENTS_PLAYERCLIP: i32 = 0x10000;

pub(crate) const CONTENTS_TRIGGER: i32 = 0x4000_0000;

pub(crate) const CONTENTS_NODROP: i32 = i32::MIN;

pub(crate) const EF_BOUNCE_HALF: i32 = 0x20;

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
    if pool.entities[slot].s.ground_entity_num == -1 && pool.entities[slot].s.pos.ty != TrajectoryType::Gravity {
        let entity = &mut pool.entities[slot];
        entity.s.pos.ty = TrajectoryType::Gravity;
        entity.s.pos.time = context.time;
    }

    if pool.entities[slot].s.pos.ty == TrajectoryType::Stationary {
        run_think(pool, slot, context.time, context.think)?;
        return Ok(());
    }

    let destination = evaluate_trajectory(&pool.entities[slot].s.pos, context.time);
    let owner_num = pool.entities[slot].r.owner_num;
    let pass_actor = if owner_num < 0 || owner_num >= ENTITYNUM_WORLD as i32 {
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
            .is_some_and(|item| item.item_type() == ItemType::Team);
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
    if item.item_type() != ItemType::Team {
        return false;
    }
    game_type == GameType::Ctf as i32 || (product == Product::Missionpack && game_type == GameType::OneFctf as i32)
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
    entity.s.e_type = EntityType::Item as i32;
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
        ty: TrajectoryType::Gravity,
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
    let forward = qvm_angle_vectors(vec3(0.0, source.s.apos.base.y + angle, source.s.apos.base.z)).forward;
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
