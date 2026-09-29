//! Entity, grapple and weapon-jump reachability from id Software's
//! `code/botlib/be_aas_reach.c`, `AAS_TravelFlagsForTeam` through
//! `AAS_Reachability_WeaponJump`, ported from
//! `src/bots/navigation/aas-reachability-special.ts`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use qa_core::math::{add3, angle_vectors, dot3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};
use qa_core::numeric::{float32_to_bits, native_atoi};

use crate::aas::AasPlane;
use crate::aas_reachability::AasReachabilityContext;
use crate::aas_reachability_geometry::{
    aas_at, aas_closest_edge_points, aas_face_center, aas_fall_damage_distance, aas_max_jump_distance, axis_get, ma,
    v3, with_z, AasClosestEdgeState, AXES, DOWN, UP, ZERO,
};
use crate::aas_reachability_types::AasStopEvent;
use crate::behavior::TravelType;
use crate::error::BotsError;

const MAX_EPAIRKEY: usize = 128;
const FACE_SOLID: i32 = 1;
const FACE_GROUND: i32 = 4;
const AREA_WEAPONJUMP: i32 = 8192;
const LAND_EVENTS: i32 = AasStopEvent::HIT_GROUND
    | AasStopEvent::ENTER_WATER
    | AasStopEvent::ENTER_SLIME
    | AasStopEvent::ENTER_LAVA
    | AasStopEvent::HIT_GROUND_DAMAGE
    | AasStopEvent::TOUCH_JUMP_PAD
    | AasStopEvent::TOUCH_TELEPORTER;
const HAZARD_EVENTS: i32 = AasStopEvent::ENTER_SLIME | AasStopEvent::ENTER_LAVA | AasStopEvent::HIT_GROUND_DAMAGE;
const WEAPON_JUMP_ITEMS: [&str; 12] = [
    "item_armor_body",
    "item_armor_combat",
    "item_health_mega",
    "weapon_grenadelauncher",
    "weapon_rocketlauncher",
    "weapon_lightning",
    "weapon_plasmagun",
    "weapon_railgun",
    "weapon_bfg",
    "item_quad",
    "item_regen",
    "item_invulnerability",
];

fn middle(a: Vec3, b: Vec3) -> Vec3 {
    scale3(add3(a, b), 0.5)
}

fn epair_text(bytes: &[u8]) -> String {
    bytes.iter().take_while(|b| **b != 0).map(|b| char::from(*b)).collect()
}

fn source_int(value: f64) -> Result<i32, BotsError> {
    let integer = value.trunc();
    if !integer.is_finite() || integer < -2_147_483_648.0 || integer > 2_147_483_647.0 {
        return Err(BotsError::IntRange(
            "AAS special reachability float-to-int conversion is undefined".to_string(),
        ));
    }
    Ok(integer as i32)
}

// C printf promotes stored float coordinates to double and rounds ties
// to even. Used for diagnostics only; Rust fixed-precision formatting is
// likewise correctly rounded.
fn fixed(value: f32, digits: usize) -> String {
    let bits = float32_to_bits(value);
    if bits & 0x7fff_ffff == 0x7f80_0000 {
        return if bits >> 31 == 0 {
            "inf".to_string()
        } else {
            "-inf".to_string()
        };
    }
    if value.is_nan() {
        return "nan".to_string();
    }
    format!("{:.digits$}", f64::from(value), digits = digits)
}

fn perimeter(min: Vec3, mid: Vec3, max: Vec3) -> [Vec3; 8] {
    [
        vec3(min.x, mid.y, 0.0),
        vec3(mid.x, max.y, 0.0),
        vec3(max.x, mid.y, 0.0),
        vec3(mid.x, min.y, 0.0),
        vec3(min.x, max.y, 0.0),
        vec3(max.x, max.y, 0.0),
        vec3(max.x, min.y, 0.0),
        vec3(min.x, min.y, 0.0),
    ]
}

impl<'a> AasReachabilityContext<'a> {
    fn teleport_area(&self, area: i32) -> Result<bool, BotsError> {
        Ok((self.world.setting(area)?.contents & 64) != 0)
    }

    fn jump_pad_area(&self, area: i32) -> Result<bool, BotsError> {
        Ok((self.world.setting(area)?.contents & 128) != 0)
    }

    /// Team travel flags from a `bot_notteam` epair.
    pub(crate) fn travel_flags_for_team(&self, entity: i32) -> Result<i32, BotsError> {
        let (found, value) = self.bsp_entities.int(entity, "bot_notteam")?;
        if !found {
            return Ok(0);
        }
        Ok(if value == 1 {
            TravelType::NOTTEAM1
        } else if value == 2 {
            TravelType::NOTTEAM2
        } else {
            0
        })
    }

    /// Teleport reachabilities from trigger entities.
    pub(crate) fn teleport(&mut self) -> Result<(), BotsError> {
        let settings = self.movement_settings.clone();
        let mut classname_buffer = [0u8; MAX_EPAIRKEY];
        let mut model_buffer = [0u8; MAX_EPAIRKEY];
        let mut target_buffer = [0u8; MAX_EPAIRKEY];
        let mut target_name_buffer = [0u8; MAX_EPAIRKEY];
        let mut model_seen = false;
        let mut entity = self.bsp_entities.next_entity(0);
        while entity != 0 {
            let mut classname = None;
            if self.bsp_entities.value(entity, "classname", &mut classname_buffer)? {
                classname = Some(epair_text(&classname_buffer));
            }
            let bound = entity;
            entity = self.bsp_entities.next_entity(entity);
            let Some(classname) = classname else { continue };
            if classname != "trigger_multiple" && classname != "trigger_teleport" {
                continue;
            }
            let entity = bound;
            if self.bsp_entities.value(entity, "model", &mut model_buffer)? {
                model_seen = true;
            }
            self.print(1, &format!("{classname} model = \"{}\"\n", epair_text(&model_buffer)));
            if !model_seen {
                return Err(BotsError::Internal(
                    "AAS_Reachability_Teleport would read an uninitialized model suffix".to_string(),
                ));
            }
            let model_number = native_atoi(&epair_text(&model_buffer[1..]))?;
            let (model_bounds, model_origin) = self.host_model_bounds(model_number, ZERO)?;
            if !self.bsp_entities.value(entity, "target", &mut target_buffer)? {
                self.print(
                    3,
                    &format!(
                        "{classname} at {} {} {} without target\n",
                        fixed(model_origin.x, 0),
                        fixed(model_origin.y, 0),
                        fixed(model_origin.z, 0)
                    ),
                );
                continue;
            }
            if classname == "trigger_multiple" {
                let mut relay = self.bsp_entities.next_entity(0);
                loop {
                    if relay == 0 {
                        break;
                    }
                    let is_relay = self.bsp_entities.value(relay, "classname", &mut classname_buffer)?
                        && epair_text(&classname_buffer) == "target_teleporter";
                    let named = self.bsp_entities.value(relay, "targetname", &mut target_name_buffer)?
                        && epair_text(&target_name_buffer) == epair_text(&target_buffer);
                    if is_relay && named {
                        break;
                    }
                    relay = self.bsp_entities.next_entity(relay);
                }
                if relay == 0 {
                    continue;
                }
                if !self.bsp_entities.value(relay, "target", &mut target_buffer)? {
                    self.print(3, "target_teleporter without target\n");
                    continue;
                }
            }
            let target = epair_text(&target_buffer);
            let mut destination = self.bsp_entities.next_entity(0);
            while destination != 0 {
                if self
                    .bsp_entities
                    .value(destination, "targetname", &mut target_name_buffer)?
                    && epair_text(&target_name_buffer) == target
                {
                    break;
                }
                destination = self.bsp_entities.next_entity(destination);
            }
            if destination == 0 {
                self.print(3, &format!("teleporter without misc_teleporter_dest ({target})\n"));
                continue;
            }
            let (found_origin, mut destination_origin) = self.bsp_entities.vector(destination, "origin")?;
            if !found_origin {
                self.print(3, &format!("teleporter destination ({target}) without origin\n"));
                continue;
            }
            let mut destination_area = self.world.point_area(destination_origin)?;
            if !self.teleport_area(destination_area)? && !self.jump_pad_area(destination_area)? {
                let trace = self.trace_client_bbox(
                    destination_origin,
                    with_z(destination_origin, f64::from(destination_origin.z) - 64.0),
                    4,
                )?;
                if trace.start_solid {
                    self.print(3, &format!("teleporter destination ({target}) in solid\n"));
                    continue;
                }
                // The source recomputes the area from the predicted
                // end below; keep the fallible sample for its error.
                self.world.point_area(trace.end)?;
                let angle = self.bsp_entities.float(destination, "angle")?.1;
                let velocity = if angle != 0.0 {
                    scale3(angle_vectors(vec3(0.0, angle, 0.0)).forward, 400.0)
                } else {
                    ZERO
                };
                let movement = self.movement_predict_client(crate::behavior::BotMovementPrediction {
                    entity_num: -1,
                    origin: destination_origin,
                    presence: 2,
                    on_ground: false,
                    velocity,
                    command_move: ZERO,
                    command_frames: 0,
                    max_frames: 30,
                    frame_time: 0.1,
                    stop_events: LAND_EVENTS,
                    stop_area: 0,
                    visualize: false,
                })?;
                destination_area = self.world.point_area(movement.end)?;
                if (movement.stop_event & (AasStopEvent::ENTER_SLIME | AasStopEvent::ENTER_LAVA)) != 0 {
                    self.print(2, &format!("teleported into slime or lava at dest {target}\n"));
                }
                destination_origin = movement.end;
            }
            let bounds = Bounds {
                min: add3(model_origin, model_bounds.min),
                max: add3(model_origin, model_bounds.max),
            };
            let midpoint = middle(bounds.min, bounds.max);
            let areas = self.link_client_bounds(bounds, 4)?;
            if areas.is_empty() {
                self.print(1, "trigger_multiple not in any area\n");
            }
            for link in areas {
                if !self.teleport_area(link)? {
                    continue;
                }
                let Some(id) = self.allocate() else {
                    break;
                };
                let team = self.travel_flags_for_team(entity)?;
                {
                    let slot = self.slot_mut(id)?;
                    slot.link.area = destination_area;
                    slot.link.face = 0;
                    slot.link.edge = 0;
                    slot.link.start = midpoint;
                    slot.link.end = destination_origin;
                    slot.link.travel_type = TravelType::TELEPORT | team;
                    slot.link.set_travel_time(settings.teleport_time)?;
                }
                self.link_area(link, id)?;
                self.debug_state.count("teleport");
            }
        }
        Ok(())
    }

    /// Elevator reachabilities from func_plat entities.
    pub(crate) fn elevator(&mut self) -> Result<(), BotsError> {
        let settings = self.movement_settings.clone();
        if self.debug {
            self.log("AAS_Reachability_Elevator\r\n");
        }
        let mut classname_buffer = [0u8; MAX_EPAIRKEY];
        let mut model_buffer = [0u8; MAX_EPAIRKEY];
        let mut entity = self.bsp_entities.next_entity(0);
        while entity != 0 {
            let current = entity;
            entity = self.bsp_entities.next_entity(entity);
            if !self.bsp_entities.value(current, "classname", &mut classname_buffer)?
                || epair_text(&classname_buffer) != "func_plat"
            {
                continue;
            }
            let entity = current;
            if self.debug {
                self.log("found func plat\r\n");
            }
            if !self.bsp_entities.value(entity, "model", &mut model_buffer)? {
                self.print(3, "func_plat without model\n");
                continue;
            }
            let model_number = native_atoi(epair_text(&model_buffer).get(1..).unwrap_or(""))?;
            if model_number <= 0 {
                self.print(3, "func_plat with invalid model number\n");
                continue;
            }
            let (model_bounds, _) = self.host_model_bounds(model_number, ZERO)?;
            let origin = self.bsp_entities.vector(entity, "origin")?.1;
            let mut min = model_bounds.min;
            let mut max = model_bounds.max;
            let mut lip = self.bsp_entities.float(entity, "lip")?.1;
            if lip == 0.0 {
                lip = 8.0;
            }
            let mut height = self.bsp_entities.float(entity, "height")?.1;
            if height == 0.0 {
                height = (f64::from(max.z) - f64::from(min.z)) as f32 - lip;
            }
            let mut speed = self.bsp_entities.float(entity, "speed")?.1;
            if speed == 0.0 {
                speed = 200.0;
            }
            let pos2 = with_z(origin, f64::from(origin.z) - f64::from(height));
            let sum = add3(min, max);
            let center = v3(
                f64::from(pos2.x) + 0.5 * f64::from(sum.x),
                f64::from(pos2.y) + 0.5 * f64::from(sum.y),
                f64::from(pos2.z) + 0.5 * f64::from(sum.z),
            );
            let drop = (f64::from(origin.z) - f64::from(pos2.z)) as f32;
            let platform_bottom = with_z(center, f64::from((f64::from(max.z) - f64::from(drop)) as f32) + 2.0);
            let platform_top = with_z(center, f64::from(max.z) + 2.0);
            min = sub3(min, vec3(1.0, 1.0, 1.0));
            max = add3(max, vec3(1.0, 1.0, 1.0));
            let mid = middle(min, max);
            let bottom_sides = perimeter(min, mid, max);
            for offset in bottom_sides.iter().map(Some).chain([None]) {
                let (bottom_origin, area1) = if let Some(offset) = offset {
                    let mut bottom_origin = v3(
                        f64::from(origin.x) + f64::from(offset.x),
                        f64::from(origin.y) + f64::from(offset.y),
                        f64::from(platform_bottom.z) + 16.0,
                    );
                    let mut area1 = self.world.point_area(bottom_origin)?;
                    let mut k = 0;
                    while k < 16 {
                        if area1 != 0 && (self.grounded(area1)? || self.swim_area(area1)?) {
                            break;
                        }
                        bottom_origin = with_z(bottom_origin, f64::from(bottom_origin.z) + 4.0);
                        area1 = self.world.point_area(bottom_origin)?;
                        k += 1;
                    }
                    if k >= 16 {
                        continue;
                    }
                    (bottom_origin, area1)
                } else {
                    let area1 = self
                        .world
                        .point_area(with_z(platform_top, f64::from(platform_top.z) + 24.0))?;
                    if area1 == 0 {
                        continue;
                    }
                    (with_z(platform_bottom, f64::from(platform_bottom.z) + 24.0), area1)
                };
                let mut n = 0;
                while n < 3 {
                    min = sub3(min, vec3(4.0, 4.0, 4.0));
                    max = add3(max, vec3(4.0, 4.0, 4.0));
                    let top_sides = perimeter(min, mid, max);
                    for offset in top_sides {
                        let mut top_origin = v3(
                            f64::from(origin.x) + f64::from(offset.x),
                            f64::from(origin.y) + f64::from(offset.y),
                            f64::from(platform_top.z) + 16.0,
                        );
                        let mut area2 = self.world.point_area(top_origin)?;
                        let mut l = 0;
                        while l < 16 {
                            if area2 != 0 && (self.grounded(area2)? || self.swim_area(area2)?) {
                                let trace = self.trace_client_bbox(
                                    with_z(platform_top, f64::from(platform_top.z) + 32.0),
                                    with_z(top_origin, f64::from(top_origin.z) + 1.0),
                                    4,
                                )?;
                                if trace.fraction >= 1.0 {
                                    break;
                                }
                            }
                            top_origin = with_z(top_origin, f64::from(top_origin.z) + 4.0);
                            area2 = self.world.point_area(top_origin)?;
                            l += 1;
                        }
                        if l >= 16 || area2 == area1 || !self.grounded(area2)? || self.exists(area1, area2)? {
                            continue;
                        }
                        let outward = normalize3(sub3(bottom_origin, platform_bottom));
                        let start = v3(
                            f64::from(bottom_origin.x) + f64::from((24.0 * f64::from(outward.x)) as f32),
                            f64::from(bottom_origin.y) + f64::from((24.0 * f64::from(outward.y)) as f32),
                            f64::from(bottom_origin.z),
                        );
                        if !AXES.iter().any(|&axis| {
                            axis_get(start, axis) < axis_get(origin, axis) + axis_get(min, axis)
                                || axis_get(start, axis) > axis_get(origin, axis) + axis_get(max, axis)
                        }) {
                            continue;
                        }
                        let Some(id) = self.allocate() else {
                            continue;
                        };
                        let team = self.travel_flags_for_team(entity)?;
                        {
                            let slot = self.slot_mut(id)?;
                            slot.link.area = area2;
                            slot.link.face = model_number;
                            slot.link.edge = source_int(f64::from(height))?;
                            slot.link.start = start;
                            slot.link.end = top_origin;
                            slot.link.travel_type = TravelType::ELEVATOR | team;
                            let time = (settings.start_elevator_time
                                + f64::from(((f64::from(height) * 100.0) / f64::from(speed)) as f32))
                                as f32;
                            slot.link.set_travel_time(f64::from(time))?;
                        }
                        self.link_area(area1, id)?;
                        n = 9999;
                        if self.debug {
                            self.log(&format!("elevator reach from {area1} to {area2}\r\n"));
                        }
                        self.debug_state.count("elevator");
                    }
                    n += 1;
                }
            }
        }
        Ok(())
    }

    /// Face reachabilities between a mover face and nearby ground.
    fn find_face_reachabilities(
        &mut self,
        points: &[Vec3],
        plane: &AasPlane,
        towards_face: bool,
    ) -> Result<Option<usize>, BotsError> {
        let mut links: Option<usize> = None;
        let mut best_face = 0;
        let mut best_plane: Option<AasPlane> = None;
        let mut state = AasClosestEdgeState { range: None };
        let area_count = self.world.asset.areas.len() as i32;
        for area_number in 1..area_count {
            let area = *aas_at(&self.world.asset.areas, area_number)?;
            let mut best_distance = 999999.0f64;
            for j in 0..area.face_count {
                let face_number = *aas_at(&self.world.asset.face_indexes, area.first_face + j)?;
                let face = *aas_at(&self.world.asset.faces, face_number.unsigned_abs() as i32)?;
                if (face.flags & FACE_GROUND) == 0 {
                    continue;
                }
                let face_plane = *aas_at(&self.world.asset.planes, face.plane)?;
                for k in 0..face.edge_count {
                    let edge = *aas_at(
                        &self.world.asset.edges,
                        aas_at(&self.world.asset.edge_indexes, face.first_edge + k)?.unsigned_abs() as i32,
                    )?;
                    let v1 = *aas_at(&self.world.asset.vertices, edge.vertices[0])?;
                    let v2 = *aas_at(&self.world.asset.vertices, edge.vertices[1])?;
                    for l in 0..points.len() {
                        let distance = aas_closest_edge_points(
                            v1,
                            v2,
                            points[l],
                            points[(l + 1) % points.len()],
                            &face_plane,
                            plane,
                            &mut state,
                            best_distance,
                        )?;
                        if distance < best_distance {
                            best_face = face_number;
                            best_plane = Some(face_plane);
                            best_distance = distance;
                        }
                    }
                }
            }
            if best_distance > 192.0 {
                continue;
            }
            let (range, plane) = match (state.range, best_plane) {
                (Some(range), Some(plane)) => (range, plane),
                _ => {
                    return Err(BotsError::Internal(
                        "AAS_FindFaceReachabilities would read uninitialized closest-edge points".to_string(),
                    ));
                }
            };
            let (mut start, mut end) = (middle(range.start1, range.start2), middle(range.end1, range.end2));
            if !towards_face {
                std::mem::swap(&mut start, &mut end);
            }
            state.range = Some(crate::aas_reachability_geometry::AasClosestEdgeRange {
                start1: start,
                end1: end,
                start2: range.start2,
                end2: range.end2,
            });
            let horizontal_distance = length3(with_z(sub3(end, start), 0.0));
            if horizontal_distance
                > (2.0
                    * f64::from(aas_max_jump_distance(
                        &self.movement_settings,
                        self.movement_settings.jump_velocity,
                    ))) as f32
            {
                continue;
            }
            if (f64::from(end.z) - 32.0) as f32 > start.z || end.z < (f64::from(start.z) - 128.0) as f32 {
                continue;
            }
            if horizontal_distance > 32.0 && !self.movement_horizontal_velocity_for_jump(0.0, start, end).success {
                continue;
            }
            start = with_z(start, f64::from(start.z) + 1.0);
            end = with_z(end, f64::from(end.z) + 1.0);
            state.range = Some(crate::aas_reachability_geometry::AasClosestEdgeRange {
                start1: start,
                end1: end,
                start2: range.start2,
                end2: range.end2,
            });
            let anchor = if towards_face { end } else { start };
            let test = with_z(anchor, 0.0);
            let test_point = with_z(
                test,
                (f64::from(plane.distance) - f64::from(dot3(plane.normal, test))) / f64::from(plane.normal.z),
            );
            if !self.point_inside_face(best_face, test_point, 0.1)? && (f64::from(end.z) - 16.0) as f32 > start.z {
                continue;
            }
            let Some(id) = self.allocate() else {
                return Ok(links);
            };
            {
                let slot = self.slot_mut(id)?;
                slot.link.area = area_number;
                slot.link.face = 0;
                slot.link.edge = 0;
                slot.link.start = start;
                slot.link.end = end;
                slot.link.travel_type = 0;
                slot.link.set_travel_time(0.0)?;
                slot.next = links;
            }
            links = Some(id);
            self.permanent_line(start, end, if towards_face { 1 } else { 2 });
        }
        Ok(links)
    }

    /// Func-bobbing reachabilities between mover endpoints and ground.
    pub(crate) fn func_bobbing(&mut self) -> Result<(), BotsError> {
        let settings = self.movement_settings.clone();
        let mut classname_buffer = [0u8; MAX_EPAIRKEY];
        let mut model_buffer = [0u8; MAX_EPAIRKEY];
        let mut entity = self.bsp_entities.next_entity(0);
        while entity != 0 {
            let current = entity;
            entity = self.bsp_entities.next_entity(entity);
            if !self.bsp_entities.value(current, "classname", &mut classname_buffer)?
                || epair_text(&classname_buffer) != "func_bobbing"
            {
                continue;
            }
            let entity = current;
            let mut height = self.bsp_entities.float(entity, "height")?.1;
            if height == 0.0 {
                height = 32.0;
            }
            if !self.bsp_entities.value(entity, "model", &mut model_buffer)? {
                self.print(3, "func_bobbing without model\n");
                continue;
            }
            let model_number = native_atoi(epair_text(&model_buffer).get(1..).unwrap_or(""))?;
            if model_number <= 0 {
                self.print(3, "func_bobbing with invalid model number\n");
                continue;
            }
            let origin = self.bsp_entities.vector(entity, "origin")?.1;
            let (model_bounds, _) = self.host_model_bounds(model_number, ZERO)?;
            let min = add3(model_bounds.min, origin);
            let max = add3(model_bounds.max, origin);
            let mid = middle(min, max);
            let spawn_flags = self.bsp_entities.int(entity, "spawnflags")?.1;
            let axis = if (spawn_flags & 1) != 0 {
                0
            } else if (spawn_flags & 2) != 0 {
                1
            } else {
                2
            };
            let mut move_start = mid;
            let mut move_end = mid;
            match axis {
                0 => {
                    move_start.x = (f64::from(mid.x) - f64::from(height)) as f32;
                    move_end.x = (f64::from(mid.x) + f64::from(height)) as f32;
                }
                1 => {
                    move_start.y = (f64::from(mid.y) - f64::from(height)) as f32;
                    move_end.y = (f64::from(mid.y) + f64::from(height)) as f32;
                }
                _ => {
                    move_start.z = (f64::from(mid.z) - f64::from(height)) as f32;
                    move_end.z = (f64::from(mid.z) + f64::from(height)) as f32;
                }
            }
            self.log(&format!(
                "funcbob model {model_number}, start = {{{}, {}, {}}} end = {{{}, {}, {}}}\n",
                fixed(move_start.x, 1),
                fixed(move_start.y, 1),
                fixed(move_start.z, 1),
                fixed(move_end.x, 1),
                fixed(move_end.y, 1),
                fixed(move_end.z, 1)
            ));
            let make_face = |point: Vec3| -> [Vec3; 4] {
                let top = (f64::from(point.z) + f64::from(max.z) - f64::from(mid.z) + 24.0) as f32;
                [
                    vec3(point.x + (max.x - mid.x), point.y + (max.y - mid.y), top),
                    vec3(point.x + (max.x - mid.x), point.y + (min.y - mid.y), top),
                    vec3(point.x + (min.x - mid.x), point.y + (min.y - mid.y), top),
                    vec3(point.x + (min.x - mid.x), point.y + (max.y - mid.y), top),
                ]
            };
            let start_points = make_face(move_start);
            let end_points = make_face(move_end);
            let start_plane = AasPlane {
                normal: UP,
                distance: start_points[0].z,
                plane_type: 2,
            };
            let end_plane = AasPlane {
                normal: UP,
                distance: end_points[0].z,
                plane_type: 2,
            };
            let top_offset = ((f64::from(max.z) - f64::from(mid.z)) as f32 + 24.0) as f32;
            let start_top = with_z(move_start, f64::from(move_start.z) + f64::from(top_offset));
            let end_top = with_z(move_end, f64::from(move_end.z) + f64::from(top_offset));
            if self.world.point_area(start_top)? == 0 || self.world.point_area(end_top)? == 0 {
                continue;
            }
            for direction in 0..2 {
                let starts = self.find_face_reachabilities(
                    if direction == 0 { &start_points } else { &end_points },
                    if direction == 0 { &start_plane } else { &end_plane },
                    true,
                )?;
                let ends = self.find_face_reachabilities(
                    if direction == 0 { &end_points } else { &start_points },
                    if direction == 0 { &end_plane } else { &start_plane },
                    false,
                )?;
                let mut start_ids = Vec::new();
                let mut next = starts;
                while let Some(id) = next {
                    start_ids.push(id);
                    next = self.slot(id)?.next;
                }
                let mut end_ids = Vec::new();
                let mut next = ends;
                while let Some(id) = next {
                    end_ids.push(id);
                    next = self.slot(id)?.next;
                }
                for &start_id in &start_ids {
                    for &end_id in &end_ids {
                        let start_reach = self.slot(start_id)?.link;
                        let end_reach = self.slot(end_id)?.link;
                        self.log(&format!(
                            "funcbob reach from area {} to {}\n",
                            start_reach.area, end_reach.area
                        ));
                        let anchor = if direction == 0 { start_top } else { end_top };
                        let outward = normalize3(with_z(sub3(start_reach.start, anchor), 0.0));
                        let mut start = ma(start_reach.start, 1.0, outward);
                        let mut end = ma(start_reach.start, 16.0, outward);
                        start = with_z(start, f64::from(start.z) + 1.0);
                        end = with_z(end, f64::from(end.z) + 1.0);
                        let crossings = self.trace_areas(start, end, 10)?;
                        if crossings.is_empty() {
                            continue;
                        }
                        let resolved = if crossings.len() > 1 { crossings[1].point } else { end };
                        self.slot_mut(start_id)?.link.start = resolved;
                        if self.world.point_area(resolved)? == 0 || self.world.point_area(end_reach.end)? == 0 {
                            continue;
                        }
                        let Some(id) = self.allocate() else {
                            return Err(BotsError::Internal(
                                "AAS_Reachability_FuncBobbing would dereference a failed reachability allocation"
                                    .to_string(),
                            ));
                        };
                        let team = self.travel_flags_for_team(entity)?;
                        {
                            let slot = self.slot_mut(id)?;
                            slot.link.area = end_reach.area;
                            let (edge_high, edge_low) = if direction == 0 {
                                (axis_get(move_start, axis), axis_get(move_end, axis))
                            } else {
                                (axis_get(move_end, axis), axis_get(move_start, axis))
                            };
                            slot.link.edge = source_int(f64::from(edge_high))?.wrapping_shl(16)
                                | (source_int(f64::from(edge_low))? & 0xffff);
                            slot.link.face = spawn_flags.wrapping_shl(16) | model_number;
                            slot.link.start = resolved;
                            slot.link.end = end_reach.end;
                            slot.link.travel_type = TravelType::FUNCBOB | team;
                            slot.link.set_travel_time(settings.func_bob_time)?;
                        }
                        self.debug_state.count("funcbob");
                        self.link_area(start_reach.area, id)?;
                    }
                }
                for id in start_ids.into_iter().chain(end_ids) {
                    self.free(id)?;
                }
                if (spawn_flags & 3) == 0 {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Jump-pad reachabilities from trigger_push entities.
    pub(crate) fn jump_pad(&mut self) -> Result<(), BotsError> {
        let settings = self.movement_settings.clone();
        let visualize = source_int(self.variable("bot_visualizejumppads", "0"))? != 0;
        let mut classname_buffer = [0u8; MAX_EPAIRKEY];
        let mut entity = self.bsp_entities.next_entity(0);
        while entity != 0 {
            let current = entity;
            entity = self.bsp_entities.next_entity(entity);
            if !self.bsp_entities.value(current, "classname", &mut classname_buffer)?
                || epair_text(&classname_buffer) != "trigger_push"
            {
                continue;
            }
            let entity = current;
            let Some(pad) = self.get_jump_pad_info(entity)? else {
                continue;
            };
            let mut start = pad.start;
            let mut velocity = pad.velocity;
            let areas = self.link_client_bounds(pad.bounds, 4)?;
            let mut in_pad = false;
            for area in &areas {
                if self.jump_pad_area(*area)? {
                    in_pad = true;
                    break;
                }
            }
            if !in_pad {
                self.print(1, "trigger_push not in any jump pad area\n");
                continue;
            }
            self.print(
                1,
                &format!(
                    "found a trigger_push with velocity {} {} {}\n",
                    fixed(velocity.x, 6),
                    fixed(velocity.y, 6),
                    fixed(velocity.z, 6)
                ),
            );
            if velocity.x != 0.0 || velocity.y != 0.0 {
                let mut movement = None;
                let mut destination_area = 0;
                let mut frame = 0;
                while frame < 20 {
                    let predicted = self.movement_predict_client(crate::behavior::BotMovementPrediction {
                        entity_num: -1,
                        origin: start,
                        presence: 2,
                        on_ground: false,
                        velocity,
                        command_move: ZERO,
                        command_frames: 0,
                        max_frames: 30,
                        frame_time: 0.1,
                        stop_events: LAND_EVENTS,
                        stop_area: 0,
                        visualize,
                    })?;
                    destination_area = predicted.end_area;
                    let mut source_link = None;
                    for link in &areas {
                        if self.jump_pad_area(*link)? && *link == destination_area {
                            source_link = Some(*link);
                            break;
                        }
                    }
                    movement = Some(predicted);
                    if source_link.is_none() {
                        break;
                    }
                    start = predicted.end;
                    velocity = predicted.velocity;
                    frame += 1;
                }
                if destination_area != 0 && frame < 20 {
                    let movement = movement
                        .ok_or_else(|| BotsError::Internal("Jump-pad movement has no prediction output".to_string()))?;
                    for link in &areas {
                        if !self.jump_pad_area(*link)? || self.exists(*link, destination_area)? {
                            continue;
                        }
                        let Some(id) = self.allocate() else {
                            return Ok(());
                        };
                        let team = self.travel_flags_for_team(entity)?;
                        {
                            let slot = self.slot_mut(id)?;
                            slot.link.area = destination_area;
                            slot.link.face = source_int(f64::from(velocity.z))?;
                            let planar = (f64::from((f64::from(velocity.x) * f64::from(velocity.x)) as f32)
                                + f64::from((f64::from(velocity.y) * f64::from(velocity.y)) as f32))
                                as f32;
                            slot.link.edge = source_int(f64::from(planar).sqrt())?;
                            slot.link.start = start;
                            slot.link.end = movement.end;
                            slot.link.travel_type = TravelType::JUMPPAD | team;
                            slot.link.set_travel_time(settings.jump_pad_time)?;
                        }
                        self.link_area(*link, id)?;
                        self.debug_state.count("jumppad");
                    }
                }
            }
            // This source continue retains the actual temporary area links.
            if velocity.x.abs() > 100.0 || velocity.y.abs() > 100.0 {
                continue;
            }
            let area_count = self.world.asset.areas.len() as i32;
            for destination_area in 1..area_count {
                let mut blocked = false;
                for link in &areas {
                    if self.exists(*link, destination_area)?
                        || (self.jump_pad_area(*link)? && *link == destination_area)
                    {
                        blocked = true;
                        break;
                    }
                }
                if blocked {
                    continue;
                }
                let area = *aas_at(&self.world.asset.areas, destination_area)?;
                for i in 0..area.face_count {
                    let face_number = *aas_at(&self.world.asset.face_indexes, area.first_face + i)?;
                    let face = *aas_at(&self.world.asset.faces, face_number.unsigned_abs() as i32)?;
                    if (face.flags & FACE_GROUND) == 0 {
                        continue;
                    }
                    let face_center = aas_face_center(&self.world.asset, face_number)?;
                    if face_center.z < start.z {
                        continue;
                    }
                    let jump = self.movement_horizontal_velocity_for_jump(f64::from(velocity.z), start, face_center);
                    if !(jump.success && jump.velocity < 150.0) {
                        continue;
                    }
                    let direction = normalize3(with_z(sub3(face_center, start), 0.0));
                    let command = scale3(direction, jump.velocity);
                    let movement = self.movement_predict_client(crate::behavior::BotMovementPrediction {
                        entity_num: -1,
                        origin: start,
                        presence: 2,
                        on_ground: false,
                        velocity,
                        command_move: command,
                        command_frames: 30,
                        max_frames: 30,
                        frame_time: 0.1,
                        stop_events: (LAND_EVENTS & !AasStopEvent::HIT_GROUND) | AasStopEvent::HIT_GROUND_AREA,
                        stop_area: destination_area,
                        visualize: false,
                    })?;
                    if movement.frames >= 30
                        || (movement.stop_event & HAZARD_EVENTS) != 0
                        || (movement.stop_event
                            & (AasStopEvent::HIT_GROUND_AREA
                                | AasStopEvent::TOUCH_JUMP_PAD
                                | AasStopEvent::TOUCH_TELEPORTER))
                            == 0
                    {
                        continue;
                    }
                    if areas.contains(&movement.end_area) {
                        continue;
                    }
                    for link in &areas {
                        if !self.jump_pad_area(*link)? || self.exists(*link, destination_area)? {
                            continue;
                        }
                        let Some(id) = self.allocate() else {
                            return Ok(());
                        };
                        let team = self.travel_flags_for_team(entity)?;
                        {
                            let slot = self.slot_mut(id)?;
                            slot.link.area = movement.end_area;
                            slot.link.face = source_int(f64::from(velocity.z))?;
                            let planar = (f64::from((f64::from(command.x) * f64::from(command.x)) as f32)
                                + f64::from((f64::from(command.y) * f64::from(command.y)) as f32))
                                as f32;
                            slot.link.edge = source_int(f64::from(planar).sqrt())?;
                            slot.link.start = start;
                            slot.link.end = face_center;
                            slot.link.travel_type = TravelType::JUMPPAD | team;
                            slot.link.set_travel_time(settings.air_controlled_jump_pad_time)?;
                        }
                        self.link_area(*link, id)?;
                        self.debug_state.count("jumppad");
                    }
                }
            }
        }
        Ok(())
    }

    /// Grapple reachability toward solid faces above the area.
    pub(crate) fn grapple(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        let settings = self.movement_settings.clone();
        if (!self.grounded(from)? && !self.swim_area(from)?) || (self.world.setting(from)?.presence & 2) == 0 {
            return Ok(false);
        }
        if self.swim_area(from)? {
            return Ok(false);
        }
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        if area2.bounds.max.z < area1.bounds.min.z {
            return Ok(false);
        }
        if self.world.point_area(area1.center)? == 0 {
            self.log(&format!(
                "area {from} center {} {} {} in solid?\r\n",
                fixed(area1.center.x, 6),
                fixed(area1.center.y, 6),
                fixed(area1.center.z, 6)
            ));
        }
        let floor = self.trace_client_bbox(
            area1.center,
            with_z(area1.center, f64::from(area1.center.z) - 1000.0),
            4,
        )?;
        if floor.start_solid {
            return Ok(false);
        }
        let area_start = floor.end;
        for i in 0..area2.face_count {
            let face_number = *aas_at(&self.world.asset.face_indexes, area2.first_face + i)?;
            let face = *aas_at(&self.world.asset.faces, face_number.unsigned_abs() as i32)?;
            if (face.flags & FACE_SOLID) == 0 {
                continue;
            }
            let edge = *aas_at(
                &self.world.asset.edges,
                aas_at(&self.world.asset.edge_indexes, face.first_edge)?.unsigned_abs() as i32,
            )?;
            let vertex = *aas_at(&self.world.asset.vertices, edge.vertices[0])?;
            let plane = *aas_at(&self.world.asset.planes, face.plane)?;
            if dot3(plane.normal, sub3(vertex, area_start)) > 0.0 {
                continue;
            }
            let face_center = aas_face_center(&self.world.asset, face_number)?;
            if face_center.z < (f64::from(area_start.z) + 64.0) as f32 || dot3(plane.normal, DOWN) < 0.0 {
                continue;
            }
            let delta = sub3(face_center, area_start);
            let horizontal_distance = length3(with_z(delta, 0.0));
            if horizontal_distance == 0.0
                || horizontal_distance > 2000.0
                || (f64::from((f64::from(delta.z) / f64::from(horizontal_distance)) as f32)
                    < (2.0 * std::f64::consts::PI * 15.0 / 360.0).tan())
            {
                continue;
            }
            let bsp_trace = self.host_trace(face_center, ma(face_center, -500.0, plane.normal), None, 1);
            if (bsp_trace.surface_flags & 4) != 0 || ((bsp_trace.fraction * 500.0) as f32) > 32.0 {
                continue;
            }
            let start = ma(area_start, 4.0, normalize3(sub3(face_center, area_start)));
            let approach = self.trace_client_bbox(start, bsp_trace.end, 2)?;
            if length3(sub3(approach.end, face_center)) > 24.0 {
                continue;
            }
            let landing = self.trace_client_bbox(
                approach.end,
                with_z(
                    approach.end,
                    f64::from(approach.end.z) - f64::from(aas_fall_damage_distance(&settings)?),
                ),
                2,
            )?;
            if landing.fraction >= 1.0 {
                continue;
            }
            let destination_area = self.world.point_area(landing.end)?;
            if (self.world.setting(destination_area)?.contents & (2 | 4)) != 0
                || destination_area == from
                || self.exists(from, destination_area)?
                || !self.grounded(destination_area)?
            {
                continue;
            }
            let crossings = self.trace_areas(area_start, bsp_trace.end, 20)?;
            if crossings.len() >= 20 {
                continue;
            }
            let mut portal = false;
            for crossing in &crossings {
                if (self.world.setting(crossing.area)?.contents & 8) != 0 {
                    portal = true;
                    break;
                }
            }
            if portal {
                continue;
            }
            let Some(id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(id)?;
                slot.link.area = destination_area;
                slot.link.face = face_number;
                slot.link.edge = 0;
                slot.link.start = area_start;
                slot.link.end = bsp_trace.end;
                slot.link.travel_type = TravelType::GRAPPLEHOOK;
                let length = length3(sub3(slot.link.end, slot.link.start));
                slot.link
                    .set_travel_time(settings.start_grapple_time + f64::from(length) * 0.25)?;
            }
            self.link_area(from, id)?;
            self.debug_state.count("grapple");
        }
        Ok(false)
    }

    /// Flag weapon-jump destination areas around valuables and pads.
    pub(crate) fn set_weapon_jump_area_flags(&mut self) -> Result<(), BotsError> {
        let bounds = Bounds {
            min: vec3(-15.0, -15.0, -15.0),
            max: vec3(15.0, 15.0, 15.0),
        };
        let mut classname_buffer = [0u8; MAX_EPAIRKEY];
        let mut count = 0;
        let mut entity = self.bsp_entities.next_entity(0);
        while entity != 0 {
            let current = entity;
            entity = self.bsp_entities.next_entity(entity);
            if !self.bsp_entities.value(current, "classname", &mut classname_buffer)? {
                continue;
            }
            let classname = epair_text(&classname_buffer);
            if !WEAPON_JUMP_ITEMS.contains(&classname.as_str()) {
                continue;
            }
            let (found, parsed) = self.bsp_entities.vector(current, "origin")?;
            if !found {
                continue;
            }
            let mut origin = parsed;
            if (self.bsp_entities.int(current, "spawnflags")?.1 & 1) == 0 {
                match self.drop_to_floor(origin, bounds) {
                    Some(dropped) => origin = dropped,
                    None => self.print(
                        1,
                        &format!(
                            "{classname} in solid at ({} {} {})\n",
                            fixed(origin.x, 1),
                            fixed(origin.y, 1),
                            fixed(origin.z, 1)
                        ),
                    ),
                }
            }
            let (area, _) = self.best_reachable_area(origin, bounds)?;
            self.world.setting_mut(area)?.flags |= AREA_WEAPONJUMP;
            count += 1;
        }
        let area_count = self.world.asset.areas.len() as i32;
        for area in 1..area_count {
            if !self.jump_pad_area(area)? {
                continue;
            }
            self.world.setting_mut(area)?.flags |= AREA_WEAPONJUMP;
            count += 1;
        }
        self.print(1, &format!("{count} weapon jump areas\n"));
        Ok(())
    }

    /// Rocket-jump reachability toward flagged valuable areas. The
    /// compiled source loop permits only the rocket-jump branch.
    pub(crate) fn weapon_jump(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        let settings = self.movement_settings.clone();
        if !self.grounded(from)?
            || self.swim_area(from)?
            || !self.grounded(to)?
            || (self.world.setting(to)?.flags & AREA_WEAPONJUMP) == 0
        {
            return Ok(false);
        }
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        if area2.bounds.max.z < area1.bounds.min.z {
            return Ok(false);
        }
        if self.world.point_area(area1.center)? == 0 {
            self.log(&format!(
                "area {from} center {} {} {} in solid?\r\n",
                fixed(area1.center.x, 6),
                fixed(area1.center.y, 6),
                fixed(area1.center.z, 6)
            ));
        }
        let floor = self.trace_client_bbox(
            area1.center,
            with_z(area1.center, f64::from(area1.center.z) - 1000.0),
            4,
        )?;
        if floor.start_solid {
            return Ok(false);
        }
        let start = floor.end;
        for i in 0..area2.face_count {
            let face_number = *aas_at(&self.world.asset.face_indexes, area2.first_face + i)?;
            let face = *aas_at(&self.world.asset.faces, face_number.unsigned_abs() as i32)?;
            if (face.flags & FACE_GROUND) == 0 {
                continue;
            }
            let face_center = aas_face_center(&self.world.asset, face_number)?;
            if face_center.z < (f64::from(start.z) + 64.0) as f32 {
                continue;
            }
            let velocity = self.movement_rocket_jump_z_velocity(start);
            let jump = self.movement_horizontal_velocity_for_jump(f64::from(velocity), start, face_center);
            if !(jump.success && jump.velocity < 300.0) {
                continue;
            }
            let command = scale3(normalize3(with_z(sub3(face_center, start), 0.0)), jump.velocity);
            let movement = self.movement_predict_client(crate::behavior::BotMovementPrediction {
                entity_num: -1,
                origin: start,
                presence: 2,
                on_ground: true,
                velocity: vec3(0.0, 0.0, velocity),
                command_move: command,
                command_frames: 30,
                max_frames: 30,
                frame_time: 0.1,
                stop_events: (LAND_EVENTS & !AasStopEvent::TOUCH_TELEPORTER) | AasStopEvent::HIT_GROUND_AREA,
                stop_area: to,
                visualize: false,
            })?;
            if movement.frames >= 30
                || (movement.stop_event & HAZARD_EVENTS) != 0
                || (movement.stop_event & (AasStopEvent::HIT_GROUND_AREA | AasStopEvent::TOUCH_JUMP_PAD)) == 0
            {
                continue;
            }
            let Some(id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(id)?;
                slot.link.area = to;
                slot.link.face = 0;
                slot.link.edge = 0;
                slot.link.start = start;
                slot.link.end = face_center;
                slot.link.travel_type = TravelType::ROCKETJUMP;
                slot.link.set_travel_time(settings.rocket_jump_time)?;
            }
            self.link_area(from, id)?;
            self.debug_state.count("rocketjump");
            return Ok(true);
        }
        Ok(false)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_formats_like_printf() {
        assert_eq!(fixed(1.5, 1), "1.5");
        assert_eq!(fixed(-0.0, 6), "-0.000000");
        assert_eq!(fixed(2.0, 0), "2");
        assert_eq!(fixed(f32::INFINITY, 1), "inf");
        assert_eq!(fixed(f32::NEG_INFINITY, 1), "-inf");
        assert_eq!(fixed(f32::NAN, 1), "nan");
    }

    #[test]
    fn source_int_checks_range() {
        assert_eq!(source_int(3.9).unwrap(), 3);
        assert_eq!(source_int(-3.9).unwrap(), -3);
        assert!(source_int(3e9).is_err());
        assert!(source_int(f64::NAN).is_err());
    }

    #[test]
    fn epair_text_stops_at_nul() {
        assert_eq!(epair_text(b"ab\0cd"), "ab");
        assert_eq!(epair_text(b""), "");
    }

    #[test]
    fn perimeter_spans_sides_and_corners() {
        let min = vec3(-8.0, -8.0, 0.0);
        let mid = vec3(0.0, 0.0, 0.0);
        let max = vec3(8.0, 8.0, 0.0);
        let sides = perimeter(min, mid, max);
        assert_eq!(sides[0], vec3(-8.0, 0.0, 0.0));
        assert_eq!(sides[4], vec3(-8.0, 8.0, 0.0));
        assert_eq!(middle(min, max), vec3(0.0, 0.0, 0.0));
    }
}
