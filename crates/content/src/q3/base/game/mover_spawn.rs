//! Quake III base/game: mover spawn.
//!
//! Donor provenance: `src/content/q3/base/game/mover-spawn.ts`.

use qa_core::math::{add3, add_point_to_bounds, dot3, length3, scale3, sub3, vec3, vector_to_angles, Bounds, Vec3};
use std::collections::HashSet;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::item_motion::*;
use crate::q3::base::game::mirrors_game_items::*;
use crate::q3::base::game::misc::*;

// ---------------------------------------------------------------------------
// mover-spawn.ts: g_mover.c class spawn and trigger handlers
// ---------------------------------------------------------------------------

pub(crate) const FRAMETIME: i32 = 100;

/// Door trigger touch name.
pub const MOVER_DOOR_TRIGGER_TOUCH: &str = "q3.base.game.mover-spawn.spawnDoorTrigger.touch";

/// Door blocked name.
pub const MOVER_DOOR_BLOCKED: &str = "q3.base.game.mover-spawn.door.blocked";

/// Door match-team think name.
pub const MOVER_DOOR_MATCH_TEAM: &str = "q3.base.game.mover-spawn.door.matchTeam";

/// Door spawn-trigger think name.
pub const MOVER_DOOR_SPAWN_TRIGGER: &str = "q3.base.game.mover-spawn.door.spawnTrigger";

/// Plat trigger touch name.
pub const MOVER_PLAT_TRIGGER_TOUCH: &str = "q3.base.game.mover-spawn.spawnPlatTrigger.touch";

/// Plat touch name.
pub const MOVER_PLAT_TOUCH: &str = "q3.base.game.mover-spawn.plat.touch";

/// Button touch name.
pub const MOVER_BUTTON_TOUCH: &str = "q3.base.game.mover-spawn.button.touch";

/// Reached-train think name.
pub const MOVER_REACHED_TRAIN_THINK: &str = "q3.base.game.mover-spawn.reachedTrain.think";

/// Train reached name.
pub const MOVER_TRAIN_REACHED: &str = "q3.base.game.mover-spawn.train.reached";

/// Train think name.
pub const MOVER_TRAIN_THINK: &str = "q3.base.game.mover-spawn.train.think";

/// Mover spawn host (`MoverSpawnHost`).
pub trait MoverSpawnHost {
    /// Mover core.
    fn movers(&mut self) -> &mut dyn MoverCore;
    /// Combat and world pair.
    fn combat_and_world(&mut self) -> (&mut dyn CombatOps, &mut dyn WorldOps);
    /// Gravity.
    fn gravity(&self) -> f32;
    /// Set a brush model (`SV_SetBrushModel`).
    fn set_brush_model(&mut self, pool: &mut EntityPool, slot: Slot, name: Option<&str>) -> Q3GameItemsResult<()>;
    /// Remap a shader.
    fn remap_shader(&mut self, old_name: &str, new_name: &str, time_seconds: f32);
    /// Use targets (`useTargets`).
    fn use_targets(
        &mut self,
        pool: &mut EntityPool,
        entity: Slot,
        activator: Option<DamageParticipant>,
    ) -> Q3GameItemsResult<()>;
    /// Warning sink.
    fn warn(&mut self, message: &str);
}

pub(crate) fn float_int(value: f32) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) {
        value.trunc() as i32
    } else {
        i32::MIN
    }
}

pub(crate) fn axis_component(value: Vec3, axis: i32) -> Q3GameItemsResult<f32> {
    match axis {
        0 => Ok(value.x),
        1 => Ok(value.y),
        2 => Ok(value.z),
        _ => Err(invalid("door trigger axis is not a source vector index")),
    }
}

pub(crate) fn with_axis_component(value: Vec3, axis: i32, amount: f32) -> Q3GameItemsResult<Vec3> {
    match axis {
        0 => Ok(vec3(amount, value.y, value.z)),
        1 => Ok(vec3(value.x, amount, value.z)),
        2 => Ok(vec3(value.x, value.y, amount)),
        _ => Err(invalid("door trigger axis is not a source vector index")),
    }
}

/// Whether an entity is a door trigger.
pub fn is_door_trigger(pool: &EntityPool, slot: Slot) -> Q3GameItemsResult<bool> {
    pool.require_owned(slot)?;
    Ok(pool.at(slot)?.touch == Some(CallbackName(MOVER_DOOR_TRIGGER_TOUCH)))
}

pub(crate) fn mover_parent(pool: &EntityPool, slot: Slot) -> Q3GameItemsResult<Slot> {
    pool.at(slot)?
        .parent
        .ok_or_else(|| invalid("mover trigger has no parent"))
}

/// Door trigger touch.
pub fn door_touch(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    other: DamageParticipant,
) -> Q3GameItemsResult<()> {
    let parent = mover_parent(pool, slot)?;
    if let DamageParticipant::Entity(other_slot) = other {
        let spectator = pool
            .at(other_slot)?
            .client
            .as_ref()
            .is_some_and(|client| client.sess.session_team == Team::Spectator);
        if spectator {
            let state = pool.at(parent)?.mover_state;
            if state == MoverState::OneToTwo || state == MoverState::Pos2 {
                return Ok(());
            }
            let axis = pool.at(slot)?.count;
            let entity = pool.at(slot)?;
            let bounds = Bounds {
                min: entity.r.absmin,
                max: entity.r.absmax,
            };
            let min = axis_component(bounds.min, axis)?;
            let max = axis_component(bounds.max, axis)?;
            let position = axis_component(pool.at(other_slot)?.s.origin, axis)?;
            let toward_min = (position - max).abs() < (position - min).abs();
            let direction = with_axis_component(vec3(0.0, 0.0, 0.0), axis, if toward_min { -1.0 } else { 1.0 })?;
            let center = scale3(add3(bounds.min, bounds.max), 0.5);
            let origin = with_axis_component(center, axis, if toward_min { min - 10.0 } else { max + 10.0 })?;
            let (combat, world) = host.combat_and_world();
            let mut context = TeleportContext {
                combat: &mut *combat,
                world: &mut *world,
            };
            teleport_player(pool, &mut context, other_slot, origin, vector_to_angles(direction))?;
            return Ok(());
        }
    }
    if pool.at(parent)?.mover_state != MoverState::OneToTwo {
        host.movers()
            .use_binary(pool, parent, Some(DamageParticipant::Entity(slot)), Some(other))?;
    }
    Ok(())
}

/// Spawn a door trigger.
pub fn spawn_door_trigger(pool: &mut EntityPool, host: &mut dyn MoverSpawnHost, slot: Slot) -> Q3GameItemsResult<()> {
    let mut part = Some(slot);
    while let Some(current) = part {
        pool.at_mut(current)?.takedamage = true;
        part = pool.at(current)?.teamchain;
    }
    let entity = pool.at(slot)?;
    let mut bounds = Bounds {
        min: entity.r.absmin,
        max: entity.r.absmax,
    };
    let mut part = pool.at(slot)?.teamchain;
    while let Some(current) = part {
        let record = pool.at(current)?;
        bounds = add_point_to_bounds(add_point_to_bounds(bounds, record.r.absmin), record.r.absmax);
        part = record.teamchain;
    }
    let size = sub3(bounds.max, bounds.min);
    let mut axis = 0;
    for index in 1..3 {
        if axis_component(size, index)? < axis_component(size, axis)? {
            axis = index;
        }
    }
    let trigger = pool.spawn()?;
    pool.at_mut(trigger)?.classname = Some("door_trigger".to_string());
    pool.at_mut(trigger)?.r.mins = with_axis_component(bounds.min, axis, axis_component(bounds.min, axis)? - 120.0)?;
    pool.at_mut(trigger)?.r.maxs = with_axis_component(bounds.max, axis, axis_component(bounds.max, axis)? + 120.0)?;
    pool.at_mut(trigger)?.parent = Some(slot);
    pool.at_mut(trigger)?.r.contents = CONTENTS_TRIGGER;
    pool.at_mut(trigger)?.count = axis;
    let touch = pool.touch_cbs.resolve(MOVER_DOOR_TRIGGER_TOUCH)?;
    pool.at_mut(trigger)?.touch = Some(touch);
    host.combat_and_world().1.link(pool, trigger)?;
    let state = pool.at(slot)?.mover_state;
    let time = host.movers().time();
    host.movers().match_team(pool, slot, state, time)?;
    Ok(())
}

/// Spawn a door (`func_door`).
pub fn mover_door(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let start = host.movers().sound_index("sound/movers/doors/dr1_strt.wav");
    let end = host.movers().sound_index("sound/movers/doors/dr1_end.wav");
    pool.at_mut(slot)?.sound1to2 = start;
    pool.at_mut(slot)?.sound2to1 = start;
    pool.at_mut(slot)?.sound_pos1 = end;
    pool.at_mut(slot)?.sound_pos2 = end;
    let blocked = pool.blocked_cbs.resolve(MOVER_DOOR_BLOCKED)?;
    pool.at_mut(slot)?.blocked = Some(blocked);
    if pool.at(slot)?.speed == 0.0 {
        pool.at_mut(slot)?.speed = 400.0;
    }
    if pool.at(slot)?.wait == 0.0 {
        pool.at_mut(slot)?.wait = 2.0;
    }
    pool.at_mut(slot)?.wait = pool.at(slot)?.wait * 1000.0;
    let lip = variables.float("lip", "8").value;
    let damage = variables.int("dmg", "2").value;
    pool.at_mut(slot)?.damage = damage;
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.pos1 = origin;
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    let angles = pool.at(slot)?.s.angles;
    let (direction, cleared) = move_direction(angles);
    pool.at_mut(slot)?.s.angles = cleared;
    pool.at_mut(slot)?.movedir = direction;
    let entity = pool.at(slot)?;
    let absolute = vec3(entity.movedir.x.abs(), entity.movedir.y.abs(), entity.movedir.z.abs());
    let distance = dot3(absolute, sub3(entity.r.maxs, entity.r.mins)) - lip;
    let pos2 = add3(pool.at(slot)?.pos1, scale3(pool.at(slot)?.movedir, distance));
    pool.at_mut(slot)?.pos2 = pos2;
    if pool.at(slot)?.spawnflags & 1 != 0 {
        let pos2 = pool.at(slot)?.pos2;
        let origin = pool.at(slot)?.s.origin;
        pool.at_mut(slot)?.pos1 = pos2;
        pool.at_mut(slot)?.pos2 = origin;
    }
    host.movers().initialize_binary(pool, slot, variables)?;
    let time = host.movers().time();
    pool.at_mut(slot)?.nextthink = time.wrapping_add(FRAMETIME);
    if pool.at(slot)?.flags & GameFlags::TEAMSLAVE == 0 {
        let health = variables.int("health", "0").value;
        if health != 0 {
            pool.at_mut(slot)?.takedamage = true;
        }
        let name = if pool.at(slot)?.targetname.is_some() || health != 0 {
            MOVER_DOOR_MATCH_TEAM
        } else {
            MOVER_DOOR_SPAWN_TRIGGER
        };
        let think = pool.think_cbs.resolve(name)?;
        pool.at_mut(slot)?.think = Some(think);
    }
    Ok(())
}

pub(crate) fn spawn_plat_trigger(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
) -> Q3GameItemsResult<()> {
    let trigger = pool.spawn()?;
    pool.at_mut(trigger)?.classname = Some("plat_trigger".to_string());
    let touch = pool.touch_cbs.resolve(MOVER_PLAT_TRIGGER_TOUCH)?;
    pool.at_mut(trigger)?.touch = Some(touch);
    pool.at_mut(trigger)?.r.contents = CONTENTS_TRIGGER;
    pool.at_mut(trigger)?.parent = Some(slot);
    let entity = pool.at(slot)?;
    let mut min = add3(add3(entity.pos1, entity.r.mins), vec3(33.0, 33.0, 0.0));
    let mut max = add3(add3(entity.pos1, entity.r.maxs), vec3(-33.0, -33.0, 8.0));
    for axis in [0, 1] {
        if axis_component(max, axis)? > axis_component(min, axis)? {
            continue;
        }
        let entity = pool.at(slot)?;
        let center = axis_component(entity.pos1, axis)?
            + (axis_component(entity.r.mins, axis)? + axis_component(entity.r.maxs, axis)?) * 0.5;
        min = with_axis_component(min, axis, center)?;
        max = with_axis_component(max, axis, center + 1.0)?;
    }
    pool.at_mut(trigger)?.r.mins = min;
    pool.at_mut(trigger)?.r.maxs = max;
    host.combat_and_world().1.link(pool, trigger)?;
    Ok(())
}

/// Spawn a plat (`func_plat`).
pub fn mover_plat(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let start = host.movers().sound_index("sound/movers/plats/pt1_strt.wav");
    let end = host.movers().sound_index("sound/movers/plats/pt1_end.wav");
    pool.at_mut(slot)?.sound1to2 = start;
    pool.at_mut(slot)?.sound2to1 = start;
    pool.at_mut(slot)?.sound_pos1 = end;
    pool.at_mut(slot)?.sound_pos2 = end;
    pool.at_mut(slot)?.s.angles = vec3(0.0, 0.0, 0.0);
    pool.at_mut(slot)?.speed = variables.float("speed", "200").value;
    pool.at_mut(slot)?.damage = variables.int("dmg", "2").value;
    pool.at_mut(slot)?.wait = 1000.0;
    let lip = variables.float("lip", "8").value;
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    let height = variables.float("height", "0");
    let distance = if height.present {
        height.value
    } else {
        pool.at(slot)?.r.maxs.z - pool.at(slot)?.r.mins.z - lip
    };
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.pos2 = origin;
    pool.at_mut(slot)?.pos1 = vec3(origin.x, origin.y, origin.z - distance);
    host.movers().initialize_binary(pool, slot, variables)?;
    let touch = pool.touch_cbs.resolve(MOVER_PLAT_TOUCH)?;
    let blocked = pool.blocked_cbs.resolve(MOVER_DOOR_BLOCKED)?;
    pool.at_mut(slot)?.touch = Some(touch);
    pool.at_mut(slot)?.blocked = Some(blocked);
    pool.at_mut(slot)?.parent = Some(slot);
    if pool.at(slot)?.targetname.is_none() {
        spawn_plat_trigger(pool, host, slot)?;
    }
    Ok(())
}

/// Spawn a button (`func_button`).
pub fn mover_button(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let sound = host.movers().sound_index("sound/movers/switches/butn2.wav");
    pool.at_mut(slot)?.sound1to2 = sound;
    if pool.at(slot)?.speed == 0.0 {
        pool.at_mut(slot)?.speed = 40.0;
    }
    if pool.at(slot)?.wait == 0.0 {
        pool.at_mut(slot)?.wait = 1.0;
    }
    pool.at_mut(slot)?.wait = pool.at(slot)?.wait * 1000.0;
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.pos1 = origin;
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    let angles = pool.at(slot)?.s.angles;
    let (direction, cleared) = move_direction(angles);
    pool.at_mut(slot)?.s.angles = cleared;
    pool.at_mut(slot)?.movedir = direction;
    let lip = variables.float("lip", "4").value;
    let entity = pool.at(slot)?;
    let absolute = vec3(entity.movedir.x.abs(), entity.movedir.y.abs(), entity.movedir.z.abs());
    let distance = dot3(absolute, sub3(entity.r.maxs, entity.r.mins)) - lip;
    let pos2 = add3(pool.at(slot)?.pos1, scale3(pool.at(slot)?.movedir, distance));
    pool.at_mut(slot)?.pos2 = pos2;
    if pool.at(slot)?.health != 0 {
        pool.at_mut(slot)?.takedamage = true;
    } else {
        let touch = pool.touch_cbs.resolve(MOVER_BUTTON_TOUCH)?;
        pool.at_mut(slot)?.touch = Some(touch);
    }
    host.movers().initialize_binary(pool, slot, variables)?;
    Ok(())
}

/// Train arrival (`reachedTrain`).
pub fn reached_train(pool: &mut EntityPool, host: &mut dyn MoverSpawnHost, slot: Slot) -> Q3GameItemsResult<()> {
    let next = pool.at(slot)?.next_train;
    let Some(next) = next else {
        return Ok(());
    };
    if pool.at(next)?.next_train.is_none() {
        return Ok(());
    }
    host.use_targets(pool, next, None)?;
    let destination = pool
        .at(next)?
        .next_train
        .ok_or_else(|| invalid("train path was removed during target dispatch"))?;
    pool.at_mut(slot)?.next_train = Some(destination);
    pool.at_mut(slot)?.pos1 = pool.at(next)?.s.origin;
    pool.at_mut(slot)?.pos2 = pool.at(destination)?.s.origin;
    let next_speed = pool.at(next)?.speed;
    let speed = (if next_speed != 0.0 {
        next_speed
    } else {
        pool.at(slot)?.speed
    })
    .max(1.0);
    let duration = float_int(length3(sub3(pool.at(slot)?.pos2, pool.at(slot)?.pos1)) * 1000.0 / speed);
    pool.at_mut(slot)?.s.pos.duration = duration;
    pool.at_mut(slot)?.s.loop_sound = pool.at(next)?.sound_loop;
    let time = host.movers().time();
    host.movers().set_state(pool, slot, MoverState::OneToTwo, time)?;
    if pool.at(next)?.wait != 0.0 {
        pool.at_mut(slot)?.nextthink = float_int(time as f32 + pool.at(next)?.wait * 1000.0);
        let think = pool.think_cbs.resolve(MOVER_REACHED_TRAIN_THINK)?;
        pool.at_mut(slot)?.think = Some(think);
        pool.at_mut(slot)?.s.pos.ty = TrajectoryType::Stationary;
    }
    Ok(())
}

pub(crate) fn setup_train(pool: &mut EntityPool, host: &mut dyn MoverSpawnHost, slot: Slot) -> Q3GameItemsResult<()> {
    let target = pool.at(slot)?.target.clone();
    let start = find_entity(pool, None, EntityStringField::Targetname, target.as_deref());
    let Some(start) = start else {
        let absmin = pool.at(slot)?.r.absmin;
        host.warn(&format!(
            "func_train at {} with an unfound target\n",
            EntityPool::vtos(absmin)
        ));
        return Ok(());
    };
    pool.at_mut(slot)?.next_train = Some(start);
    let mut visited = HashSet::new();
    let mut path = start;
    loop {
        if !visited.insert(path) {
            return Err(invalid("train path cycle does not return to its first corner"));
        }
        let target = pool.at(path)?.target.clone();
        let Some(target) = target else {
            let origin = pool.at(path)?.s.origin;
            host.warn(&format!(
                "Train corner at {} without a target\n",
                EntityPool::vtos(origin)
            ));
            return Ok(());
        };
        let mut next = None;
        loop {
            next = find_entity(pool, next, EntityStringField::Targetname, Some(&target));
            let Some(candidate) = next else {
                let origin = pool.at(path)?.s.origin;
                host.warn(&format!(
                    "Train corner at {} without a target path_corner\n",
                    EntityPool::vtos(origin)
                ));
                return Ok(());
            };
            if pool.at(candidate)?.classname.as_deref() == Some("path_corner") {
                next = Some(candidate);
                break;
            }
        }
        let next = next.expect("corner loop checked");
        pool.at_mut(path)?.next_train = Some(next);
        path = next;
        if path == start {
            break;
        }
    }
    reached_train(pool, host, slot)
}

/// Path corner spawn (`path_corner`).
pub fn mover_path_corner(pool: &mut EntityPool, host: &mut dyn MoverSpawnHost, slot: Slot) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    if pool.at(slot)?.targetname.is_some() {
        return Ok(());
    }
    let origin = pool.at(slot)?.s.origin;
    host.warn(&format!(
        "path_corner with no targetname at {}\n",
        EntityPool::vtos(origin)
    ));
    pool.free(slot)?;
    Ok(())
}

/// Spawn a train (`func_train`).
pub fn mover_train(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    pool.at_mut(slot)?.s.angles = vec3(0.0, 0.0, 0.0);
    if pool.at(slot)?.spawnflags & 4 != 0 {
        pool.at_mut(slot)?.damage = 0;
    } else if pool.at(slot)?.damage == 0 {
        pool.at_mut(slot)?.damage = 2;
    }
    if pool.at(slot)?.speed == 0.0 {
        pool.at_mut(slot)?.speed = 100.0;
    }
    if pool.at(slot)?.target.is_none() {
        let absmin = pool.at(slot)?.r.absmin;
        host.warn(&format!(
            "func_train without a target at {}\n",
            EntityPool::vtos(absmin)
        ));
        pool.free(slot)?;
        return Ok(());
    }
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    host.movers().initialize_binary(pool, slot, variables)?;
    let reached = pool.reached_cbs.resolve(MOVER_TRAIN_REACHED)?;
    pool.at_mut(slot)?.reached = Some(reached);
    let time = host.movers().time();
    pool.at_mut(slot)?.nextthink = time.wrapping_add(FRAMETIME);
    let think = pool.think_cbs.resolve(MOVER_TRAIN_THINK)?;
    pool.at_mut(slot)?.think = Some(think);
    Ok(())
}

/// Spawn a static mover (`func_static`).
pub fn mover_static(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    host.movers().initialize_binary(pool, slot, variables)?;
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.s.pos.base = origin;
    pool.at_mut(slot)?.r.current_origin = origin;
    Ok(())
}

/// Spawn a rotating mover (`func_rotating`).
pub fn mover_rotating(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    if pool.at(slot)?.speed == 0.0 {
        pool.at_mut(slot)?.speed = 100.0;
    }
    let spawnflags = pool.at(slot)?.spawnflags;
    let axis = if spawnflags & 4 != 0 {
        2
    } else if spawnflags & 8 != 0 {
        0
    } else {
        1
    };
    let speed = pool.at(slot)?.speed;
    pool.at_mut(slot)?.s.apos.ty = TrajectoryType::Linear;
    let delta = with_axis_component(pool.at(slot)?.s.apos.delta, axis, speed)?;
    pool.at_mut(slot)?.s.apos.delta = delta;
    if pool.at(slot)?.damage == 0 {
        pool.at_mut(slot)?.damage = 2;
    }
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    host.movers().initialize_binary(pool, slot, variables)?;
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.s.pos.base = origin;
    pool.at_mut(slot)?.r.current_origin = pool.at(slot)?.s.pos.base;
    pool.at_mut(slot)?.r.current_angles = pool.at(slot)?.s.apos.base;
    host.combat_and_world().1.link(pool, slot)?;
    Ok(())
}

/// Spawn a bobbing mover (`func_bobbing`).
pub fn mover_bobbing(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    pool.at_mut(slot)?.speed = variables.float("speed", "4").value;
    pool.at_mut(slot)?.damage = variables.int("dmg", "2").value;
    let height = variables.float("height", "32").value;
    let phase = variables.float("phase", "0").value;
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    host.movers().initialize_binary(pool, slot, variables)?;
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.r.current_origin = origin;
    let duration = float_int(pool.at(slot)?.speed * 1000.0);
    let spawnflags = pool.at(slot)?.spawnflags;
    let axis = if spawnflags & 1 != 0 {
        0
    } else if spawnflags & 2 != 0 {
        1
    } else {
        2
    };
    let delta = with_axis_component(pool.at(slot)?.s.pos.delta, axis, height)?;
    pool.at_mut(slot)?.s.pos = Trajectory {
        ty: TrajectoryType::Sine,
        time: float_int(duration as f32 * phase),
        duration,
        base: origin,
        delta,
    };
    Ok(())
}

/// Spawn a pendulum (`func_pendulum`).
pub fn mover_pendulum(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    let speed = variables.float("speed", "30").value;
    let phase = variables.float("phase", "0").value;
    pool.at_mut(slot)?.damage = variables.int("dmg", "2").value;
    let model = pool.at(slot)?.model.clone();
    host.set_brush_model(pool, slot, model.as_deref())?;
    let length = 8.0f32.max(pool.at(slot)?.r.mins.z.abs());
    let gravity = host.gravity();
    let frequency = (1.0 / (std::f32::consts::PI * 2.0)) * (f64::from(gravity / (3.0 * length)).sqrt() as f32);
    let duration = float_int(1000.0 / frequency);
    pool.at_mut(slot)?.s.pos.duration = duration;
    host.movers().initialize_binary(pool, slot, variables)?;
    let origin = pool.at(slot)?.s.origin;
    pool.at_mut(slot)?.s.pos.base = origin;
    pool.at_mut(slot)?.r.current_origin = origin;
    let angles = pool.at(slot)?.s.angles;
    let delta = with_axis_component(pool.at(slot)?.s.apos.delta, 2, speed)?;
    pool.at_mut(slot)?.s.apos = Trajectory {
        ty: TrajectoryType::Sine,
        time: float_int(duration as f32 * phase),
        duration,
        base: angles,
        delta,
    };
    Ok(())
}

/// Mover spawn handler selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MoverSpawn {
    /// `func_door`.
    Door,
    /// `func_plat`.
    Plat,
    /// `func_button`.
    Button,
    /// `func_train`.
    Train,
    /// `path_corner`.
    PathCorner,
    /// `func_static`.
    Static,
    /// `func_rotating`.
    Rotating,
    /// `func_bobbing`.
    Bobbing,
    /// `func_pendulum`.
    Pendulum,
}

/// Spawn table entries (`createMoverSpawnHandlers`).
#[must_use]
pub fn mover_spawn_handlers() -> Vec<(&'static str, MoverSpawn)> {
    vec![
        ("func_door", MoverSpawn::Door),
        ("func_plat", MoverSpawn::Plat),
        ("func_button", MoverSpawn::Button),
        ("func_train", MoverSpawn::Train),
        ("path_corner", MoverSpawn::PathCorner),
        ("func_static", MoverSpawn::Static),
        ("func_rotating", MoverSpawn::Rotating),
        ("func_bobbing", MoverSpawn::Bobbing),
        ("func_pendulum", MoverSpawn::Pendulum),
    ]
}

/// Run a mover spawn handler.
pub fn run_mover_spawn(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    handler: &MoverSpawn,
    slot: Slot,
    variables: &SpawnVariables,
) -> Q3GameItemsResult<()> {
    pool.require_owned(slot)?;
    bind_mover_spawn_callbacks(pool);
    match *handler {
        MoverSpawn::Door => mover_door(pool, host, slot, variables),
        MoverSpawn::Plat => mover_plat(pool, host, slot, variables),
        MoverSpawn::Button => mover_button(pool, host, slot, variables),
        MoverSpawn::Train => mover_train(pool, host, slot, variables),
        MoverSpawn::PathCorner => mover_path_corner(pool, host, slot),
        MoverSpawn::Static => mover_static(pool, host, slot, variables),
        MoverSpawn::Rotating => mover_rotating(pool, host, slot, variables),
        MoverSpawn::Bobbing => mover_bobbing(pool, host, slot, variables),
        MoverSpawn::Pendulum => mover_pendulum(pool, host, slot, variables),
    }
}

/// Register mover-spawn save callbacks.
pub fn bind_mover_spawn_callbacks(pool: &mut EntityPool) {
    pool.touch_cbs.intern(MOVER_DOOR_TRIGGER_TOUCH);
    pool.blocked_cbs.intern(MOVER_DOOR_BLOCKED);
    pool.think_cbs.intern(MOVER_DOOR_MATCH_TEAM);
    pool.think_cbs.intern(MOVER_DOOR_SPAWN_TRIGGER);
    pool.touch_cbs.intern(MOVER_PLAT_TRIGGER_TOUCH);
    pool.touch_cbs.intern(MOVER_PLAT_TOUCH);
    pool.touch_cbs.intern(MOVER_BUTTON_TOUCH);
    pool.think_cbs.intern(MOVER_REACHED_TRAIN_THINK);
    pool.reached_cbs.intern(MOVER_TRAIN_REACHED);
    pool.think_cbs.intern(MOVER_TRAIN_THINK);
}

/// Dispatch a mover-spawn think callback; returns false when unhandled.
pub fn dispatch_mover_think(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    name: CallbackName,
) -> Q3GameItemsResult<bool> {
    match name.0 {
        MOVER_DOOR_MATCH_TEAM => {
            let state = pool.at(slot)?.mover_state;
            let time = host.movers().time();
            host.movers().match_team(pool, slot, state, time)?;
            Ok(true)
        }
        MOVER_DOOR_SPAWN_TRIGGER => {
            spawn_door_trigger(pool, host, slot)?;
            Ok(true)
        }
        MOVER_REACHED_TRAIN_THINK => {
            let time = host.movers().time();
            pool.at_mut(slot)?.s.pos.time = time;
            pool.at_mut(slot)?.s.pos.ty = TrajectoryType::LinearStop;
            Ok(true)
        }
        MOVER_TRAIN_THINK => {
            setup_train(pool, host, slot)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Dispatch a mover-spawn touch callback; returns false when unhandled.
pub fn dispatch_mover_touch(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    other: DamageParticipant,
) -> Q3GameItemsResult<bool> {
    match pool.at(slot)?.touch {
        Some(CallbackName(MOVER_DOOR_TRIGGER_TOUCH)) => {
            door_touch(pool, host, slot, other)?;
            Ok(true)
        }
        Some(CallbackName(MOVER_PLAT_TRIGGER_TOUCH)) => {
            let parent = mover_parent(pool, slot)?;
            if let DamageParticipant::Entity(other_slot) = other {
                if pool.at(other_slot)?.client.is_some() && pool.at(parent)?.mover_state == MoverState::Pos1 {
                    host.movers()
                        .use_binary(pool, parent, Some(DamageParticipant::Entity(slot)), Some(other))?;
                }
            }
            Ok(true)
        }
        Some(CallbackName(MOVER_PLAT_TOUCH)) => {
            if let DamageParticipant::Entity(other_slot) = other {
                let healthy = pool
                    .at(other_slot)?
                    .client
                    .as_ref()
                    .is_some_and(|client| client.ps.health > 0);
                if pool.at(other_slot)?.client.is_some() && healthy && pool.at(slot)?.mover_state == MoverState::Pos2 {
                    let time = host.movers().time();
                    pool.at_mut(slot)?.nextthink = time.wrapping_add(1000);
                }
            }
            Ok(true)
        }
        Some(CallbackName(MOVER_BUTTON_TOUCH)) => {
            if let DamageParticipant::Entity(other_slot) = other {
                if pool.at(other_slot)?.client.is_some() && pool.at(slot)?.mover_state == MoverState::Pos1 {
                    host.movers().use_binary(
                        pool,
                        slot,
                        Some(DamageParticipant::Entity(other_slot)),
                        Some(DamageParticipant::Entity(other_slot)),
                    )?;
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Dispatch a mover-spawn reached callback; returns false when unhandled.
pub fn dispatch_mover_reached(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    name: CallbackName,
) -> Q3GameItemsResult<bool> {
    if name != CallbackName(MOVER_TRAIN_REACHED) {
        return Ok(false);
    }
    reached_train(pool, host, slot)?;
    Ok(true)
}

/// Dispatch a mover-spawn blocked callback; returns false when unhandled.
pub fn dispatch_mover_blocked(
    pool: &mut EntityPool,
    host: &mut dyn MoverSpawnHost,
    slot: Slot,
    other: DamageParticipant,
) -> Q3GameItemsResult<bool> {
    if pool.at(slot)?.blocked != Some(CallbackName(MOVER_DOOR_BLOCKED)) {
        return Ok(false);
    }
    host.movers().blocked_door(pool, slot, other)?;
    Ok(true)
}
