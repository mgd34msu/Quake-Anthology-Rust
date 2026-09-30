//! AAS and bot-movement traps over the shared navigation owner.
//!
//! Provenance: `src/compat/qvm/bot-navigation-syscalls.ts` (Q3 AAS and bot
//! movement imports). [`AasHost`] and [`BotMoveHost`] are local mirrors of
//! the `SourceBotNavigation`/`AasBspEntities`/move-state surfaces the donor
//! consumes; the donor's `qvmBotMovementPrediction` provider projection is
//! host-owned behind [`AasHost::predict_client_movement`], which returns the
//! already-projected record. Donor `float32ToBits` results pass through as
//! `i32` bit patterns.

use qa_core::math::Vec3;

use super::bot_navigation_records::{
    read_bot_goal, read_bot_init_move, write_aas_entity_info, write_bot_move_result, AasEntityInfo, BotGoal,
    BotInitMove, BotMoveResult, QVM_AAS_ENTITY_INFO_BYTES, QVM_BOT_GOAL_BYTES, QVM_BOT_INIT_MOVE_BYTES,
    QVM_BOT_MOVE_RESULT_BYTES,
};
use super::client_state::{AbiProfile, CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::{
    BOTLIB_AAS_ALTERNATIVE_ROUTE_GOAL, BOTLIB_AAS_AREA_INFO, BOTLIB_AAS_AREA_REACHABILITY,
    BOTLIB_AAS_AREA_TRAVEL_TIME_TO_GOAL_AREA, BOTLIB_AAS_BBOX_AREAS, BOTLIB_AAS_ENABLE_ROUTING_AREA,
    BOTLIB_AAS_ENTITY_INFO, BOTLIB_AAS_FLOAT_FOR_BSP_EPAIR_KEY, BOTLIB_AAS_INITIALIZED,
    BOTLIB_AAS_INT_FOR_BSP_EPAIR_KEY, BOTLIB_AAS_NEXT_BSP_ENTITY, BOTLIB_AAS_POINT_AREA_NUM, BOTLIB_AAS_POINT_CONTENTS,
    BOTLIB_AAS_POINT_REACHABILITY_AREA_INDEX, BOTLIB_AAS_PREDICT_CLIENT_MOVEMENT, BOTLIB_AAS_PREDICT_ROUTE,
    BOTLIB_AAS_PRESENCE_TYPE_BOUNDING_BOX, BOTLIB_AAS_SWIMMING, BOTLIB_AAS_TIME, BOTLIB_AAS_TRACE_AREAS,
    BOTLIB_AAS_VALUE_FOR_BSP_EPAIR_KEY, BOTLIB_AAS_VECTOR_FOR_BSP_EPAIR_KEY, BOTLIB_AI_ADD_AVOID_SPOT,
    BOTLIB_AI_ALLOC_MOVE_STATE, BOTLIB_AI_FREE_MOVE_STATE, BOTLIB_AI_INIT_MOVE_STATE, BOTLIB_AI_MOVEMENT_VIEW_TARGET,
    BOTLIB_AI_MOVE_IN_DIRECTION, BOTLIB_AI_MOVE_TO_GOAL, BOTLIB_AI_PREDICT_VISIBLE_POSITION,
    BOTLIB_AI_REACHABILITY_AREA, BOTLIB_AI_RESET_AVOID_REACH, BOTLIB_AI_RESET_LAST_AVOID_REACH,
    BOTLIB_AI_RESET_MOVE_STATE,
};
use crate::error::GuestError;

/// Area crossing: area number plus crossing point.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaCrossing {
    /// Area number.
    pub area: i32,
    /// Crossing point.
    pub point: Vec3,
}

/// AAS area info plus bounds and origin.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaInfo {
    /// Contents.
    pub contents: i32,
    /// Flags.
    pub flags: i32,
    /// Presence type.
    pub presence_type: i32,
    /// Cluster.
    pub cluster: i32,
    /// Bounds minimum.
    pub min: Vec3,
    /// Bounds maximum.
    pub max: Vec3,
    /// Origin.
    pub origin: Vec3,
    /// Enabled override from the routing checkpoint, if pinned.
    pub enabled: Option<bool>,
    /// Reachable-area count.
    pub reachable_area_count: i32,
}

/// Client-movement prediction query.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementPredictionQuery {
    /// Entity number.
    pub entity_num: i32,
    /// Start origin.
    pub origin: Vec3,
    /// Presence type (2 or 4).
    pub presence: i32,
    /// On ground.
    pub on_ground: bool,
    /// Start velocity.
    pub velocity: Vec3,
    /// Command move.
    pub command_move: Vec3,
    /// Command frames.
    pub command_frames: i32,
    /// Maximum frames.
    pub max_frames: i32,
    /// Frame time.
    pub frame_time: f32,
    /// Stop events mask.
    pub stop_events: i32,
    /// Stop area.
    pub stop_area: i32,
    /// Visualize the prediction.
    pub visualize: bool,
}

/// Projected movement-prediction trace.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementTrace {
    /// Started inside solid.
    pub start_solid: bool,
    /// Completed fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Last area.
    pub last_area: i32,
    /// Hit area.
    pub area: i32,
    /// Plane index.
    pub plane: i32,
}

/// Projected client-movement prediction.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementPrediction {
    /// Prediction succeeded.
    pub success: bool,
    /// End position.
    pub end: Vec3,
    /// End area.
    pub end_area: i32,
    /// End velocity.
    pub velocity: Vec3,
    /// Trace record.
    pub trace: MovementTrace,
    /// Presence type.
    pub presence: i32,
    /// Stop event.
    pub stop_event: i32,
    /// End contents.
    pub end_contents: i32,
    /// Elapsed time.
    pub time: f32,
    /// Simulated frames.
    pub frames: i32,
}

/// Alternative-route goal.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteGoal {
    /// Goal origin.
    pub origin: Vec3,
    /// Goal area.
    pub area: i32,
    /// Start travel time.
    pub start_travel_time: u16,
    /// Goal travel time.
    pub goal_travel_time: u16,
    /// Extra travel time.
    pub extra_travel_time: u16,
}

/// Route prediction.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutePrediction {
    /// Whether routing succeeded.
    pub succeeded: bool,
    /// End position.
    pub end_position: Vec3,
    /// End area.
    pub end_area: i32,
    /// Stop event.
    pub stop_event: i32,
    /// End contents.
    pub end_contents: i32,
    /// End travel flags.
    pub end_travel_flags: i32,
    /// Travel time.
    pub time: i32,
}

/// Alternative-route query.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteQuery {
    /// Start point.
    pub start: Vec3,
    /// Start area.
    pub start_area: i32,
    /// Goal point.
    pub goal: Vec3,
    /// Goal area.
    pub goal_area: i32,
    /// Travel flags.
    pub travel_flags: i32,
    /// Maximum goals.
    pub maximum_goals: i32,
    /// Route type.
    pub route_type: i32,
}

/// Predict-route query.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictRouteQuery {
    /// Start area.
    pub area: i32,
    /// Start origin.
    pub origin: Vec3,
    /// Goal area.
    pub goal_area: i32,
    /// Travel flags.
    pub travel_flags: i32,
    /// Maximum areas.
    pub maximum_areas: i32,
    /// Maximum time.
    pub maximum_time: i32,
    /// Stop event.
    pub stop_event: i32,
    /// Stop contents.
    pub stop_contents: i32,
    /// Stop travel flags.
    pub stop_travel_flags: i32,
    /// Stop area.
    pub stop_area: i32,
}

/// Host AAS/navigation surface used by traps 300-318 and 575-577.
pub trait AasHost {
    /// Whether navigation data is loaded.
    fn initialized(&mut self) -> bool;
    /// Navigation time.
    fn time(&mut self) -> f32;
    /// Diagnostic print.
    fn print(&mut self, text: &str);
    /// Routing-area enable state; `set` pins a new state.
    fn routing_area(&mut self, area: i32, set: Option<bool>) -> Option<bool>;
    /// Areas overlapping a bounding box.
    fn bbox_areas(&mut self, min: Vec3, max: Vec3, maximum: i32) -> Vec<i32>;
    /// Area info by number.
    fn area(&mut self, area: i32) -> AreaInfo;
    /// Entity info by number.
    fn entity_info(&mut self, number: i32) -> Option<AasEntityInfo>;
    /// Presence-type bounds.
    fn presence_bounds(&mut self, presence: i32) -> (Vec3, Vec3);
    /// Area number containing a point.
    fn point_area(&mut self, point: Vec3) -> i32;
    /// Areas crossed by a segment.
    fn trace_areas(&mut self, start: Vec3, end: Vec3, maximum: i32) -> Vec<AreaCrossing>;
    /// Contents at a point.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Next BSP entity number.
    fn next_entity(&mut self, entity: i32) -> i32;
    /// BSP epair string value.
    fn epair_value(&mut self, entity: i32, key: &str) -> Option<String>;
    /// BSP epair vector value.
    fn epair_vector(&mut self, entity: i32, key: &str) -> (Vec3, bool);
    /// BSP epair float value.
    fn epair_float(&mut self, entity: i32, key: &str) -> (f32, bool);
    /// BSP epair int value.
    fn epair_int(&mut self, entity: i32, key: &str) -> (i32, bool);
    /// Travel time from an area to a goal area.
    fn area_travel_time(&mut self, area: i32, origin: Option<Vec3>, goal_area: i32, travel_flags: i32) -> i32;
    /// Whether a point is in a swimming area.
    fn swimming(&mut self, point: Vec3) -> bool;
    /// Projected client-movement prediction.
    fn predict_client_movement(&mut self, query: &MovementPredictionQuery) -> MovementPrediction;
    /// Alternative-route goals.
    fn alternative_route_goals(&mut self, query: &RouteQuery) -> Vec<RouteGoal>;
    /// Predicted route.
    fn predict_route(&mut self, query: &PredictRouteQuery) -> RoutePrediction;
    /// Reachability-area index for an optional origin.
    fn reachability_index(&mut self, origin: Option<Vec3>) -> i32;
}

/// Host bot move-state surface used by traps 548-557, 572, and 574.
pub trait BotMoveHost {
    /// Reset a move state.
    fn reset(&mut self, handle: i32);
    /// Reset avoid-reach data.
    fn reset_avoid_reach(&mut self, handle: i32);
    /// Reset last-avoid-reach data.
    fn reset_last_avoid_reach(&mut self, handle: i32);
    /// Allocate a move state.
    fn allocate(&mut self) -> i32;
    /// Free a move state.
    fn free(&mut self, handle: i32);
    /// Initialize a move state.
    fn initialize(&mut self, handle: i32, init: &BotInitMove);
    /// Move toward a goal, filling the result.
    fn move_to_goal(&mut self, result: &mut BotMoveResult, handle: i32, goal: &BotGoal, flags: i32);
    /// Move in a direction.
    fn move_in_direction(&mut self, handle: i32, direction: Vec3, speed: f32, flags: i32) -> bool;
    /// Reachability area for a point.
    fn reachability_area(&mut self, point: Vec3, flags: i32) -> i32;
    /// Movement view target, writing the target on success.
    fn movement_view_target(&mut self, handle: i32, goal: &BotGoal, flags: i32, range: f32, target: &mut Vec3) -> bool;
    /// Predict a visible position, writing the target on success.
    fn predict_visible_position(
        &mut self,
        origin: Vec3,
        entity: i32,
        goal: &BotGoal,
        flags: i32,
        target: &mut Vec3,
    ) -> bool;
    /// Add an avoid spot.
    fn add_avoid_spot(&mut self, handle: i32, point: Vec3, radius: f32, flags: i32);
}

fn read_goal(memory: &SyscallMemory, word: i32) -> Result<BotGoal, GuestError> {
    let range = memory.span(word, QVM_BOT_GOAL_BYTES, 0)?;
    read_bot_goal(memory.read_bytes(range.start, QVM_BOT_GOAL_BYTES)?)
}

/// Dispatch a bot-navigation trap. Returns `Ok(None)` when unhandled.
pub fn bot_navigation_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    aas: &mut dyn AasHost,
    moves: &mut dyn BotMoveHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Qagame {
        return Ok(None);
    }
    match call.code {
        BOTLIB_AAS_ENABLE_ROUTING_AREA => {
            let area = call.int(1)?;
            if area <= 0 {
                return Ok(Some(0));
            }
            let set = if call.int(2)? >= 0 {
                Some(call.int(2)? != 0)
            } else {
                None
            };
            Ok(Some(aas.routing_area(area, set).map_or(0, i32::from)))
        }
        BOTLIB_AAS_BBOX_AREAS => {
            let min = memory.read_vec3_ptr(call.int(1)?)?;
            let max = memory.read_vec3_ptr(call.int(2)?)?;
            let out_word = call.int(3)?;
            let maximum = call.int(4)?.max(0) as usize;
            let areas = aas.bbox_areas(min, max, call.int(4)?);
            let count = areas.len().min(maximum);
            if count > 0 {
                memory.span(out_word, count * 4, 0)?;
                let base = memory.pointer(out_word).expect("checked span");
                for (index, area) in areas.iter().take(count).enumerate() {
                    memory.write_i32(base + index * 4, *area)?;
                }
            }
            Ok(Some(count as i32))
        }
        BOTLIB_AAS_AREA_INFO => {
            let area = call.int(1)?;
            let out_word = call.int(2)?;
            if out_word == 0 || area <= 0 {
                return Ok(Some(0));
            }
            let info = aas.area(area);
            let flags = match info.enabled {
                None => info.flags,
                Some(true) => info.flags & !8,
                Some(false) => info.flags | 8,
            };
            memory.span(out_word, 52, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            memory.write_i32(base, info.contents)?;
            memory.write_i32(base + 4, flags)?;
            memory.write_i32(base + 8, info.presence_type)?;
            memory.write_i32(base + 12, info.cluster)?;
            memory.write_vec3(base + 16, &info.min)?;
            memory.write_vec3(base + 28, &info.max)?;
            memory.write_vec3(base + 40, &info.origin)?;
            Ok(Some(52))
        }
        BOTLIB_AAS_ENTITY_INFO => {
            let info = aas.entity_info(call.int(1)?);
            let out_word = call.int(2)?;
            memory.span(out_word, QVM_AAS_ENTITY_INFO_BYTES, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            match info {
                None => memory.fill(base, QVM_AAS_ENTITY_INFO_BYTES, 0)?,
                Some(info) => {
                    let mut record = vec![0u8; QVM_AAS_ENTITY_INFO_BYTES];
                    write_aas_entity_info(&mut record, &info)?;
                    memory.write_bytes(base, &record)?;
                }
            }
            Ok(Some(0))
        }
        BOTLIB_AAS_INITIALIZED => Ok(Some(i32::from(aas.initialized()))),
        BOTLIB_AAS_PRESENCE_TYPE_BOUNDING_BOX => {
            let presence = call.int(1)?;
            if presence != 2 && presence != 4 {
                aas.print("AAS_PresenceTypeBoundingBox: unknown presence type\n");
            }
            let (min, max) = aas.presence_bounds(if presence == 2 { 2 } else { 4 });
            memory.span(call.int(2)?, 12, 0)?;
            let base = memory.pointer(call.int(2)?).expect("checked span");
            memory.write_vec3(base, &min)?;
            memory.span(call.int(3)?, 12, 0)?;
            let base = memory.pointer(call.int(3)?).expect("checked span");
            memory.write_vec3(base, &max)?;
            Ok(Some(0))
        }
        BOTLIB_AAS_TIME => Ok(Some(aas.time().to_bits() as i32)),
        BOTLIB_AAS_POINT_AREA_NUM => {
            let point = memory.read_vec3_ptr(call.int(1)?)?;
            Ok(Some(aas.point_area(point)))
        }
        BOTLIB_AAS_TRACE_AREAS => {
            let start = memory.read_vec3_ptr(call.int(1)?)?;
            let end = memory.read_vec3_ptr(call.int(2)?)?;
            let areas_word = call.int(3)?;
            let points_word = call.int(4)?;
            let maximum = call.int(5)?;
            memory.span(areas_word, 4, 0)?;
            let areas_base = memory.pointer(areas_word).expect("checked span");
            memory.write_i32(areas_base, 0)?;
            let crossings = aas.trace_areas(start, end, maximum);
            if !crossings.is_empty() {
                memory.span(areas_word, crossings.len() * 4, 0)?;
                for (index, crossing) in crossings.iter().enumerate() {
                    memory.write_i32(areas_base + index * 4, crossing.area)?;
                }
                if points_word != 0 {
                    memory.span(points_word, crossings.len() * 12, 0)?;
                    let points_base = memory.pointer(points_word).expect("checked span");
                    for (index, crossing) in crossings.iter().enumerate() {
                        memory.write_vec3(points_base + index * 12, &crossing.point)?;
                    }
                }
            }
            Ok(Some(crossings.len() as i32))
        }
        BOTLIB_AAS_POINT_CONTENTS => {
            let point = memory.read_vec3_ptr(call.int(1)?)?;
            Ok(Some(aas.point_contents(point)))
        }
        BOTLIB_AAS_NEXT_BSP_ENTITY => Ok(Some(aas.next_entity(call.int(1)?))),
        BOTLIB_AAS_VALUE_FOR_BSP_EPAIR_KEY => {
            let entity = call.int(1)?;
            let key = memory.read_string(call.int(2)?)?;
            let out_word = call.int(3)?;
            let capacity = call.int(4)?;
            memory.span(out_word, 1, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            match aas.epair_value(entity, &key) {
                None => {
                    memory.set(base, 0)?;
                    Ok(Some(0))
                }
                Some(value) => {
                    memory.write_string(out_word, &value, capacity as usize)?;
                    Ok(Some(1))
                }
            }
        }
        BOTLIB_AAS_VECTOR_FOR_BSP_EPAIR_KEY => {
            let entity = call.int(1)?;
            let key = memory.read_string(call.int(2)?)?;
            let out_word = call.int(3)?;
            memory.span(out_word, 12, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            memory.write_vec3(base, &Vec3 { x: 0.0, y: 0.0, z: 0.0 })?;
            let (value, found) = aas.epair_vector(entity, &key);
            memory.write_vec3(base, &value)?;
            Ok(Some(i32::from(found)))
        }
        BOTLIB_AAS_FLOAT_FOR_BSP_EPAIR_KEY | BOTLIB_AAS_INT_FOR_BSP_EPAIR_KEY => {
            let entity = call.int(1)?;
            let key = memory.read_string(call.int(2)?)?;
            let out_word = call.int(3)?;
            memory.span(out_word, 4, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            memory.write_i32(base, 0)?;
            if call.code == BOTLIB_AAS_FLOAT_FOR_BSP_EPAIR_KEY {
                let (value, found) = aas.epair_float(entity, &key);
                memory.write_f32(base, value)?;
                Ok(Some(i32::from(found)))
            } else {
                let (value, found) = aas.epair_int(entity, &key);
                memory.write_i32(base, value)?;
                Ok(Some(i32::from(found)))
            }
        }
        BOTLIB_AAS_AREA_REACHABILITY => Ok(Some(aas.area(call.int(1)?).reachable_area_count)),
        BOTLIB_AAS_AREA_TRAVEL_TIME_TO_GOAL_AREA => {
            if !aas.initialized() {
                return Ok(Some(0));
            }
            let area = call.int(1)?;
            let origin_word = call.int(2)?;
            let origin = if origin_word == 0 {
                None
            } else {
                Some(memory.read_vec3_ptr(origin_word)?)
            };
            Ok(Some(aas.area_travel_time(area, origin, call.int(3)?, call.int(4)?)))
        }
        BOTLIB_AAS_SWIMMING => {
            let point = memory.read_vec3_ptr(call.int(1)?)?;
            Ok(Some(i32::from(aas.swimming(point))))
        }
        BOTLIB_AAS_PREDICT_CLIENT_MOVEMENT => {
            let presence = call.int(4)?;
            if presence != 2 && presence != 4 {
                return Err(GuestError::invalid(
                    "QVM movement prediction requires a supported presence type",
                ));
            }
            let query = MovementPredictionQuery {
                entity_num: call.int(2)?,
                origin: memory.read_vec3_ptr(call.int(3)?)?,
                presence,
                on_ground: call.int(5)? != 0,
                velocity: memory.read_vec3_ptr(call.int(6)?)?,
                command_move: memory.read_vec3_ptr(call.int(7)?)?,
                command_frames: call.int(8)?,
                max_frames: call.int(9)?,
                frame_time: call.float(10)?,
                stop_events: call.int(11)?,
                stop_area: call.int(12)?,
                visualize: call.int(13)? != 0,
            };
            let result = aas.predict_client_movement(&query);
            let out_word = call.int(1)?;
            memory.span(out_word, 84, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            memory.write_vec3(base, &result.end)?;
            memory.write_i32(base + 12, result.end_area)?;
            memory.write_vec3(base + 16, &result.velocity)?;
            memory.write_i32(base + 28, i32::from(result.trace.start_solid))?;
            memory.write_f32(base + 32, result.trace.fraction)?;
            memory.write_vec3(base + 36, &result.trace.end)?;
            memory.write_i32(base + 48, result.trace.entity_num)?;
            memory.write_i32(base + 52, result.trace.last_area)?;
            memory.write_i32(base + 56, result.trace.area)?;
            memory.write_i32(base + 60, result.trace.plane)?;
            memory.write_i32(base + 64, result.presence)?;
            memory.write_i32(base + 68, result.stop_event)?;
            if call.abi_profile == AbiProfile::Legacy {
                memory.write_f32(base + 72, result.end_contents as f32)?;
            } else {
                memory.write_i32(base + 72, result.end_contents)?;
            }
            memory.write_f32(base + 76, result.time)?;
            memory.write_i32(base + 80, result.frames)?;
            Ok(Some(i32::from(result.success)))
        }
        BOTLIB_AI_RESET_MOVE_STATE => {
            moves.reset(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_MOVE_TO_GOAL => {
            let out_word = call.int(1)?;
            let handle = call.int(2)?;
            let goal_word = call.int(3)?;
            let flags = call.int(4)?;
            memory.span(out_word, QVM_BOT_MOVE_RESULT_BYTES, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            let mut result = BotMoveResult::default();
            if goal_word == 0 {
                result.failure = true;
            } else {
                let goal = read_goal(memory, goal_word)?;
                moves.move_to_goal(&mut result, handle, &goal, flags);
            }
            let mut record = vec![0u8; QVM_BOT_MOVE_RESULT_BYTES];
            write_bot_move_result(&mut record, &result)?;
            memory.write_bytes(base, &record)?;
            Ok(Some(0))
        }
        BOTLIB_AI_MOVE_IN_DIRECTION => {
            let direction = memory.read_vec3_ptr(call.int(2)?)?;
            Ok(Some(i32::from(moves.move_in_direction(
                call.int(1)?,
                direction,
                call.float(3)?,
                call.int(4)?,
            ))))
        }
        BOTLIB_AI_RESET_AVOID_REACH => {
            moves.reset_avoid_reach(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_RESET_LAST_AVOID_REACH => {
            moves.reset_last_avoid_reach(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_REACHABILITY_AREA => {
            let point = memory.read_vec3_ptr(call.int(1)?)?;
            Ok(Some(moves.reachability_area(point, call.int(2)?)))
        }
        BOTLIB_AI_MOVEMENT_VIEW_TARGET => {
            if call.int(2)? == 0 {
                return Ok(Some(0));
            }
            let goal = read_goal(memory, call.int(2)?)?;
            let target_word = call.int(5)?;
            let mut target = if target_word == 0 {
                Vec3 { x: 0.0, y: 0.0, z: 0.0 }
            } else {
                memory.span(target_word, 12, 0)?;
                memory.read_vec3_ptr(target_word)?
            };
            let found = moves.movement_view_target(call.int(1)?, &goal, call.int(3)?, call.float(4)?, &mut target);
            if target_word != 0 {
                let base = memory.pointer(target_word).expect("checked span");
                memory.write_vec3(base, &target)?;
            }
            Ok(Some(i32::from(found)))
        }
        BOTLIB_AI_ALLOC_MOVE_STATE => Ok(Some(moves.allocate())),
        BOTLIB_AI_FREE_MOVE_STATE => {
            moves.free(call.int(1)?);
            Ok(Some(0))
        }
        BOTLIB_AI_INIT_MOVE_STATE => {
            let range = memory.span(call.int(2)?, QVM_BOT_INIT_MOVE_BYTES, 0)?;
            let init = read_bot_init_move(memory.read_bytes(range.start, QVM_BOT_INIT_MOVE_BYTES)?)?;
            moves.initialize(call.int(1)?, &init);
            Ok(Some(0))
        }
        BOTLIB_AI_PREDICT_VISIBLE_POSITION => {
            if call.int(3)? == 0 {
                return Ok(Some(0));
            }
            let origin = memory.read_vec3_ptr(call.int(1)?)?;
            let goal = read_goal(memory, call.int(3)?)?;
            let target_word = call.int(5)?;
            let mut target = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
            if target_word != 0 {
                memory.span(target_word, 12, 0)?;
                target = memory.read_vec3_ptr(target_word)?;
            }
            let found = moves.predict_visible_position(origin, call.int(2)?, &goal, call.int(4)?, &mut target);
            if target_word != 0 {
                let base = memory.pointer(target_word).expect("checked span");
                memory.write_vec3(base, &target)?;
            }
            Ok(Some(i32::from(found)))
        }
        BOTLIB_AI_ADD_AVOID_SPOT => {
            let point = memory.read_vec3_ptr(call.int(2)?)?;
            moves.add_avoid_spot(call.int(1)?, point, call.float(3)?, call.int(4)?);
            Ok(Some(0))
        }
        BOTLIB_AAS_ALTERNATIVE_ROUTE_GOAL => {
            let query = RouteQuery {
                start: memory.read_vec3_ptr(call.int(1)?)?,
                start_area: call.int(2)?,
                goal: memory.read_vec3_ptr(call.int(3)?)?,
                goal_area: call.int(4)?,
                travel_flags: call.int(5)?,
                maximum_goals: call.int(7)?,
                route_type: call.int(8)?,
            };
            let goals = aas.alternative_route_goals(&query);
            let out_word = call.int(6)?;
            if !goals.is_empty() {
                memory.span(out_word, goals.len() * 24, 0)?;
                let base = memory.pointer(out_word).expect("checked span");
                for (index, goal) in goals.iter().enumerate() {
                    let offset = base + index * 24;
                    memory.write_vec3(offset, &goal.origin)?;
                    memory.write_i32(offset + 12, goal.area)?;
                    memory.write_u16(offset + 16, goal.start_travel_time)?;
                    memory.write_u16(offset + 18, goal.goal_travel_time)?;
                    memory.write_u16(offset + 20, goal.extra_travel_time)?;
                }
            }
            Ok(Some(goals.len() as i32))
        }
        BOTLIB_AAS_PREDICT_ROUTE => {
            let query = PredictRouteQuery {
                area: call.int(2)?,
                origin: memory.read_vec3_ptr(call.int(3)?)?,
                goal_area: call.int(4)?,
                travel_flags: call.int(5)?,
                maximum_areas: call.int(6)?,
                maximum_time: call.int(7)?,
                stop_event: call.int(8)?,
                stop_contents: call.int(9)?,
                stop_travel_flags: call.int(10)?,
                stop_area: call.int(11)?,
            };
            let result = aas.predict_route(&query);
            let out_word = call.int(1)?;
            memory.span(out_word, 36, 0)?;
            let base = memory.pointer(out_word).expect("checked span");
            memory.write_vec3(base, &result.end_position)?;
            memory.write_i32(base + 12, result.end_area)?;
            memory.write_i32(base + 16, result.stop_event)?;
            memory.write_i32(base + 20, result.end_contents)?;
            memory.write_i32(base + 24, result.end_travel_flags)?;
            memory.write_i32(base + 32, result.time)?;
            Ok(Some(i32::from(result.succeeded)))
        }
        BOTLIB_AAS_POINT_REACHABILITY_AREA_INDEX => {
            let origin_word = call.int(1)?;
            let origin = if origin_word == 0 {
                None
            } else {
                Some(memory.read_vec3_ptr(origin_word)?)
            };
            Ok(Some(aas.reachability_index(origin)))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::super::bot_navigation_records::{
        read_bot_move_result, write_bot_goal, BotEntityUpdate, GoalWriteFields,
    };
    use super::*;

    struct FakeAas {
        log: Vec<String>,
        ready: bool,
    }

    fn area_info() -> AreaInfo {
        AreaInfo {
            contents: 3,
            flags: 0x10,
            presence_type: 2,
            cluster: 7,
            min: Vec3 {
                x: -8.0,
                y: -8.0,
                z: -8.0,
            },
            max: Vec3 { x: 8.0, y: 8.0, z: 8.0 },
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            enabled: None,
            reachable_area_count: 5,
        }
    }

    impl AasHost for FakeAas {
        fn initialized(&mut self) -> bool {
            self.ready
        }
        fn time(&mut self) -> f32 {
            12.5
        }
        fn print(&mut self, text: &str) {
            self.log.push(format!("print {text}"));
        }
        fn routing_area(&mut self, area: i32, set: Option<bool>) -> Option<bool> {
            self.log.push(format!("route {area} {set:?}"));
            Some(set.unwrap_or(true))
        }
        fn bbox_areas(&mut self, _min: Vec3, _max: Vec3, _maximum: i32) -> Vec<i32> {
            vec![1, 2, 3, 4]
        }
        fn area(&mut self, _area: i32) -> AreaInfo {
            area_info()
        }
        fn entity_info(&mut self, number: i32) -> Option<AasEntityInfo> {
            (number == 1).then_some(AasEntityInfo {
                valid: true,
                number: 1,
                update: BotEntityUpdate {
                    entity_type: 2,
                    flags: 0,
                    origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    old_origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    mins: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    maxs: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                    ground_entity: 0,
                    solid: 0,
                    model_index: 0,
                    model_index2: 0,
                    frame: 0,
                    event: 0,
                    event_parameter: 0,
                    powerups: 0,
                    weapon: 0,
                    legs_animation: 0,
                    torso_animation: 0,
                },
                last_visible_origin: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                last_update_time: 2.0,
                update_interval: 0.5,
            })
        }
        fn presence_bounds(&mut self, presence: i32) -> (Vec3, Vec3) {
            self.log.push(format!("presence {presence}"));
            (
                Vec3 {
                    x: -15.0,
                    y: -15.0,
                    z: -24.0,
                },
                Vec3 {
                    x: 15.0,
                    y: 15.0,
                    z: 32.0,
                },
            )
        }
        fn point_area(&mut self, _point: Vec3) -> i32 {
            9
        }
        fn trace_areas(&mut self, _start: Vec3, _end: Vec3, _maximum: i32) -> Vec<AreaCrossing> {
            vec![
                AreaCrossing {
                    area: 3,
                    point: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                },
                AreaCrossing {
                    area: 4,
                    point: Vec3 { x: 2.0, y: 0.0, z: 0.0 },
                },
            ]
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            6
        }
        fn next_entity(&mut self, entity: i32) -> i32 {
            entity + 1
        }
        fn epair_value(&mut self, _entity: i32, key: &str) -> Option<String> {
            (key == "classname").then(|| "worldspawn".to_string())
        }
        fn epair_vector(&mut self, _entity: i32, key: &str) -> (Vec3, bool) {
            if key == "origin" {
                (Vec3 { x: 1.0, y: 2.0, z: 3.0 }, true)
            } else {
                (Vec3 { x: 0.0, y: 0.0, z: 0.0 }, false)
            }
        }
        fn epair_float(&mut self, _entity: i32, key: &str) -> (f32, bool) {
            (if key == "angle" { 90.0 } else { 0.0 }, key == "angle")
        }
        fn epair_int(&mut self, _entity: i32, key: &str) -> (i32, bool) {
            (if key == "spawnflags" { 3 } else { 0 }, key == "spawnflags")
        }
        fn area_travel_time(&mut self, area: i32, origin: Option<Vec3>, goal_area: i32, travel_flags: i32) -> i32 {
            self.log
                .push(format!("travel {area} {} {goal_area} {travel_flags}", origin.is_some()));
            120
        }
        fn swimming(&mut self, _point: Vec3) -> bool {
            true
        }
        fn predict_client_movement(&mut self, _query: &MovementPredictionQuery) -> MovementPrediction {
            MovementPrediction {
                success: true,
                end: Vec3 { x: 5.0, y: 5.0, z: 0.0 },
                end_area: 8,
                velocity: Vec3 { x: 1.0, y: 0.0, z: 0.0 },
                trace: MovementTrace {
                    start_solid: false,
                    fraction: 0.75,
                    end: Vec3 { x: 4.0, y: 4.0, z: 0.0 },
                    entity_num: 2,
                    last_area: 7,
                    area: 8,
                    plane: 1,
                },
                presence: 2,
                stop_event: 0,
                end_contents: 9,
                time: 1.5,
                frames: 10,
            }
        }
        fn alternative_route_goals(&mut self, _query: &RouteQuery) -> Vec<RouteGoal> {
            vec![RouteGoal {
                origin: Vec3 { x: 9.0, y: 9.0, z: 0.0 },
                area: 12,
                start_travel_time: 10,
                goal_travel_time: 20,
                extra_travel_time: 5,
            }]
        }
        fn predict_route(&mut self, _query: &PredictRouteQuery) -> RoutePrediction {
            RoutePrediction {
                succeeded: true,
                end_position: Vec3 { x: 3.0, y: 3.0, z: 0.0 },
                end_area: 6,
                stop_event: 1,
                end_contents: 2,
                end_travel_flags: 3,
                time: 44,
            }
        }
        fn reachability_index(&mut self, origin: Option<Vec3>) -> i32 {
            i32::from(origin.is_some())
        }
    }

    struct FakeMoves {
        log: Vec<String>,
    }

    impl BotMoveHost for FakeMoves {
        fn reset(&mut self, handle: i32) {
            self.log.push(format!("reset {handle}"));
        }
        fn reset_avoid_reach(&mut self, handle: i32) {
            self.log.push(format!("avoid {handle}"));
        }
        fn reset_last_avoid_reach(&mut self, handle: i32) {
            self.log.push(format!("last {handle}"));
        }
        fn allocate(&mut self) -> i32 {
            7
        }
        fn free(&mut self, handle: i32) {
            self.log.push(format!("free {handle}"));
        }
        fn initialize(&mut self, handle: i32, init: &BotInitMove) {
            self.log.push(format!("init {handle} {}", init.entity_num));
        }
        fn move_to_goal(&mut self, result: &mut BotMoveResult, handle: i32, _goal: &BotGoal, _flags: i32) {
            self.log.push(format!("goal {handle}"));
            result.move_type = 3;
        }
        fn move_in_direction(&mut self, _handle: i32, _direction: Vec3, _speed: f32, _flags: i32) -> bool {
            true
        }
        fn reachability_area(&mut self, _point: Vec3, _flags: i32) -> i32 {
            13
        }
        fn movement_view_target(
            &mut self,
            _handle: i32,
            _goal: &BotGoal,
            _flags: i32,
            _range: f32,
            target: &mut Vec3,
        ) -> bool {
            *target = Vec3 { x: 7.0, y: 7.0, z: 7.0 };
            true
        }
        fn predict_visible_position(
            &mut self,
            _origin: Vec3,
            _entity: i32,
            _goal: &BotGoal,
            _flags: i32,
            target: &mut Vec3,
        ) -> bool {
            *target = Vec3 { x: 8.0, y: 8.0, z: 8.0 };
            true
        }
        fn add_avoid_spot(&mut self, handle: i32, _point: Vec3, radius: f32, flags: i32) {
            self.log.push(format!("spot {handle} {radius} {flags}"));
        }
    }

    fn game(code: i32, args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Qagame, code, args, AbiProfile::Modern)
    }

    fn harness() -> (SyscallMemory, FakeAas, FakeMoves) {
        (
            SyscallMemory::new(65536).unwrap(),
            FakeAas {
                log: Vec::new(),
                ready: true,
            },
            FakeMoves { log: Vec::new() },
        )
    }

    fn goal(memory: &mut SyscallMemory, at: i32) {
        let goal = BotGoal {
            origin: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            area: 2,
            mins: Vec3 {
                x: -1.0,
                y: -1.0,
                z: -1.0,
            },
            maxs: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
            entity: 0,
            number: 0,
            flags: 0,
            item_info: 0,
        };
        let mut record = vec![0u8; QVM_BOT_GOAL_BYTES];
        write_bot_goal(&mut record, &goal, GoalWriteFields::Full).unwrap();
        memory.write_bytes(at as usize, &record).unwrap();
    }

    #[test]
    fn routing_area_and_bbox() {
        let (mut memory, mut aas, mut moves) = harness();
        assert_eq!(
            bot_navigation_syscall(&game(300, &[5, 1]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_navigation_syscall(&game(300, &[0, 1]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 8.0, y: 8.0, z: 8.0 }).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(301, &[256, 512, 1024, 2]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(2)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 1);
        assert_eq!(memory.read_i32(1028).unwrap(), 2);
    }

    #[test]
    fn area_info_entity_info_and_initialized() {
        let (mut memory, mut aas, mut moves) = harness();
        assert_eq!(
            bot_navigation_syscall(&game(302, &[3, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(52)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 3);
        assert_eq!(memory.read_i32(1028).unwrap(), 0x10);
        assert_eq!(
            bot_navigation_syscall(&game(302, &[3, 0]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_navigation_syscall(&game(303, &[1, 2048]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(2048).unwrap(), 1);
        assert_eq!(memory.read_i32(2068).unwrap(), 1);
        assert_eq!(
            bot_navigation_syscall(&game(303, &[9, 2048]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(2048).unwrap(), 0);
        assert_eq!(
            bot_navigation_syscall(&game(304, &[]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn presence_time_point_and_trace() {
        let (mut memory, mut aas, mut moves) = harness();
        assert_eq!(
            bot_navigation_syscall(&game(305, &[2, 256, 512]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            memory.read_vec3(256).unwrap(),
            Vec3 {
                x: -15.0,
                y: -15.0,
                z: -24.0
            }
        );
        assert_eq!(
            bot_navigation_syscall(&game(305, &[9, 256, 512]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_navigation_syscall(&game(306, &[]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(12.5f32.to_bits() as i32)
        );
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 4.0, y: 0.0, z: 0.0 }).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(307, &[256]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(9)
        );
        assert_eq!(
            bot_navigation_syscall(
                &game(308, &[256, 512, 1024, 2048, 8]),
                &mut memory,
                &mut aas,
                &mut moves
            )
            .unwrap(),
            Some(2)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 3);
        assert_eq!(memory.read_vec3(2048).unwrap(), Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        assert_eq!(
            bot_navigation_syscall(&game(309, &[256]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(6)
        );
        assert!(aas.log.iter().any(|line| line.starts_with("print ")));
    }

    #[test]
    fn bsp_entity_traps() {
        let (mut memory, mut aas, mut moves) = harness();
        assert_eq!(
            bot_navigation_syscall(&game(310, &[4]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(5)
        );
        memory.write_string(256, "classname", 10).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(311, &[1, 256, 1024, 64]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_string(1024).unwrap(), "worldspawn");
        memory.write_string(256, "nope", 5).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(311, &[1, 256, 1024, 64]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        memory.write_string(256, "origin", 7).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(312, &[1, 256, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(1024).unwrap(), Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        memory.write_string(256, "angle", 6).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(313, &[1, 256, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_f32(1024).unwrap(), 90.0);
        memory.write_string(256, "spawnflags", 11).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(314, &[1, 256, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 3);
    }

    #[test]
    fn reachability_travel_swimming_and_prediction() {
        let (mut memory, mut aas, mut moves) = harness();
        assert_eq!(
            bot_navigation_syscall(&game(315, &[3]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(5)
        );
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(317, &[256]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_navigation_syscall(&game(316, &[1, 256, 2, 7]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(120)
        );
        aas.ready = false;
        assert_eq!(
            bot_navigation_syscall(&game(316, &[1, 256, 2, 7]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        aas.ready = true;
        for (index, word) in [256, 512, 768, 1024].iter().enumerate() {
            memory
                .write_vec3(
                    *word,
                    &Vec3 {
                        x: index as f32,
                        y: 0.0,
                        z: 0.0,
                    },
                )
                .unwrap();
        }
        let frame = 0.1f32.to_bits() as i32;
        let args = [4096, 1, 256, 2, 1, 512, 768, 3, 30, frame, 0, 0, 0];
        assert_eq!(
            bot_navigation_syscall(&game(318, &args), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(4096).unwrap(), Vec3 { x: 5.0, y: 5.0, z: 0.0 });
        assert_eq!(memory.read_i32(4108).unwrap(), 8);
        assert_eq!(memory.read_f32(4128).unwrap(), 0.75);
        assert_eq!(memory.read_i32(4168).unwrap(), 9);
        assert_eq!(memory.read_i32(4176).unwrap(), 10);
        let mut bad = args;
        bad[3] = 9;
        assert!(bot_navigation_syscall(&game(318, &bad), &mut memory, &mut aas, &mut moves).is_err());
    }

    #[test]
    fn legacy_prediction_writes_float_contents() {
        let (mut memory, mut aas, mut moves) = harness();
        for word in [256, 512, 768, 1024] {
            memory.write_vec3(word, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        }
        let frame = 0.1f32.to_bits() as i32;
        let call = HostCall::engine(
            QvmRole::Qagame,
            318,
            &[4096, 1, 256, 2, 1, 512, 768, 3, 30, frame, 0, 0, 0],
            AbiProfile::Legacy,
        );
        assert_eq!(
            bot_navigation_syscall(&call, &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_f32(4168).unwrap(), 9.0);
    }

    #[test]
    fn move_state_traps() {
        let (mut memory, mut aas, mut moves) = harness();
        goal(&mut memory, 2048);
        assert_eq!(
            bot_navigation_syscall(&game(548, &[1]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_navigation_syscall(&game(549, &[1024, 1, 2048, 0]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 0);
        assert_eq!(memory.read_i32(1028).unwrap(), 3);
        assert_eq!(
            bot_navigation_syscall(&game(549, &[1024, 1, 0, 0]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(memory.read_i32(1024).unwrap(), 1);
        memory.write_vec3(256, &Vec3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap();
        let speed = 200.0f32.to_bits() as i32;
        assert_eq!(
            bot_navigation_syscall(&game(550, &[1, 256, speed, 0]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_navigation_syscall(&game(551, &[1]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_navigation_syscall(&game(552, &[1]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_navigation_syscall(&game(553, &[256, 0]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(13)
        );
        assert_eq!(
            bot_navigation_syscall(&game(555, &[]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(7)
        );
        assert_eq!(
            bot_navigation_syscall(&game(556, &[7]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        memory.write_i32(3072 + 36, 4).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(557, &[7, 3072]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            moves.log,
            vec!["reset 1", "goal 1", "avoid 1", "last 1", "free 7", "init 7 4"]
        );
    }

    #[test]
    fn view_target_and_visible_position() {
        let (mut memory, mut aas, mut moves) = harness();
        goal(&mut memory, 2048);
        let range = 64.0f32.to_bits() as i32;
        assert_eq!(
            bot_navigation_syscall(
                &game(554, &[1, 2048, 0, range, 1024]),
                &mut memory,
                &mut aas,
                &mut moves
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(1024).unwrap(), Vec3 { x: 7.0, y: 7.0, z: 7.0 });
        assert_eq!(
            bot_navigation_syscall(&game(554, &[1, 0, 0, range, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        assert_eq!(
            bot_navigation_syscall(&game(572, &[256, 1, 2048, 0, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(1024).unwrap(), Vec3 { x: 8.0, y: 8.0, z: 8.0 });
        assert_eq!(
            bot_navigation_syscall(&game(572, &[256, 1, 0, 0, 1024]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        let radius = 32.0f32.to_bits() as i32;
        assert_eq!(
            bot_navigation_syscall(&game(574, &[1, 256, radius, 2]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(moves.log, vec!["spot 1 32 2".to_string()]);
    }

    #[test]
    fn route_goals_route_and_index() {
        let (mut memory, mut aas, mut moves) = harness();
        memory.write_vec3(256, &Vec3 { x: 0.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(512, &Vec3 { x: 8.0, y: 8.0, z: 0.0 }).unwrap();
        assert_eq!(
            bot_navigation_syscall(
                &game(575, &[256, 1, 512, 2, 7, 1024, 4, 0]),
                &mut memory,
                &mut aas,
                &mut moves
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(1024).unwrap(), Vec3 { x: 9.0, y: 9.0, z: 0.0 });
        assert_eq!(memory.read_i32(1036).unwrap(), 12);
        assert_eq!(memory.read_u16(1040).unwrap(), 10);
        assert_eq!(memory.read_u16(1042).unwrap(), 20);
        assert_eq!(memory.read_u16(1044).unwrap(), 5);
        assert_eq!(
            bot_navigation_syscall(
                &game(576, &[1024, 1, 256, 2, 7, 8, 100, 0, 0, 0, 0]),
                &mut memory,
                &mut aas,
                &mut moves
            )
            .unwrap(),
            Some(1)
        );
        assert_eq!(memory.read_vec3(1024).unwrap(), Vec3 { x: 3.0, y: 3.0, z: 0.0 });
        assert_eq!(memory.read_i32(1056).unwrap(), 44);
        assert_eq!(
            bot_navigation_syscall(&game(577, &[0]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(0)
        );
        assert_eq!(
            bot_navigation_syscall(&game(577, &[256]), &mut memory, &mut aas, &mut moves).unwrap(),
            Some(1)
        );
        assert_eq!(
            bot_navigation_syscall(&game(999, &[]), &mut memory, &mut aas, &mut moves).unwrap(),
            None
        );
    }

    #[test]
    fn move_result_readback_matches_write() {
        let result = BotMoveResult {
            failure: true,
            move_type: 1,
            ..BotMoveResult::default()
        };
        let mut record = vec![0u8; QVM_BOT_MOVE_RESULT_BYTES];
        write_bot_move_result(&mut record, &result).unwrap();
        assert_eq!(read_bot_move_result(&record).unwrap(), result);
    }
}
