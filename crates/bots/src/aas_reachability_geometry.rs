//! Area geometry and ordinary reachabilities from id Software's
//! `code/botlib/be_aas_reach.c`, ported from
//! `src/bots/navigation/aas-reachability-geometry.ts`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use std::cmp::Ordering;

use qa_core::math::{add3, cross3, dot3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};

use crate::aas::{AasAsset, AasFace, AasPlane};
use crate::aas_reachability::AasReachabilityContext;
use crate::aas_reachability_types::AasMovementSettings;
use crate::aas_reachability_types::AasStopEvent;
use crate::behavior::TravelType;
use crate::error::{indexed, BotsError};

pub(crate) const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
pub(crate) const UP: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };
pub(crate) const DOWN: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: -1.0,
};
pub(crate) const AXES: [usize; 3] = [0, 1, 2];

/// Checked record access for reachability construction.
pub fn aas_at<T>(values: &[T], index: i32) -> Result<&T, BotsError> {
    indexed(values, i64::from(index), "AAS reachability index")
}

/// Build a vector from binary64 components, narrowing like the donor's
/// `vec3` fround stores.
pub(crate) fn v3(x: f64, y: f64, z: f64) -> Vec3 {
    vec3(x as f32, y as f32, z as f32)
}

/// Multiply-add in binary32.
pub(crate) fn ma(start: Vec3, scale: f32, direction: Vec3) -> Vec3 {
    add3(start, scale3(direction, scale))
}

/// Multiply-add in binary64, narrowing the stored result.
pub(crate) fn ma_double(start: Vec3, scale: f64, direction: Vec3) -> Vec3 {
    v3(
        f64::from(start.x) + scale * f64::from(direction.x),
        f64::from(start.y) + scale * f64::from(direction.y),
        f64::from(start.z) + scale * f64::from(direction.z),
    )
}

/// Replace the Z coordinate.
pub(crate) fn with_z(point: Vec3, height: f64) -> Vec3 {
    Vec3 {
        x: point.x,
        y: point.y,
        z: height as f32,
    }
}

/// Read a vector component by axis (0 = x, 1 = y, 2 = z).
pub(crate) fn axis_get(point: Vec3, axis: usize) -> f32 {
    match axis {
        0 => point.x,
        1 => point.y,
        _ => point.z,
    }
}

/// Source float-to-int conversion with a defined-range check.
pub(crate) fn source_integer(value: f64) -> Result<i32, BotsError> {
    let integer = value.trunc();
    if !integer.is_finite() || integer < -2_147_483_648.0 || integer > 2_147_483_647.0 {
        return Err(BotsError::IntRange(
            "AAS reachability numeric conversion exceeds source signed integer range".to_string(),
        ));
    }
    Ok(integer as i32)
}

/// Source integer absolute value; `abs(INT_MIN)` is undefined.
pub(crate) fn integer_abs(value: f64) -> Result<i32, BotsError> {
    let integer = source_integer(value)?;
    if integer == i32::MIN {
        return Err(BotsError::IntRange(
            "AAS reachability abs of INT_MIN is undefined".to_string(),
        ));
    }
    Ok(integer.abs())
}

fn close_bounds(a: Bounds, b: Bounds, vertical: bool) -> bool {
    for axis in AXES {
        if !vertical && axis == 2 {
            continue;
        }
        let (a_min, a_max, b_min, b_max) = match axis {
            0 => (a.min.x, a.max.x, b.min.x, b.max.x),
            1 => (a.min.y, a.max.y, b.min.y, b.max.y),
            _ => (a.min.z, a.max.z, b.min.z, b.max.z),
        };
        if a_min > b_max + 10.0 || a_max < b_min - 10.0 {
            return false;
        }
    }
    true
}

/// Surface area of a face.
pub fn aas_face_area(world: &AasAsset, face: &AasFace) -> Result<f32, BotsError> {
    let first = aas_at(&world.edge_indexes, face.first_edge)?;
    let edge = aas_at(&world.edges, first.unsigned_abs() as i32)?;
    let origin = aas_at(&world.vertices, edge.vertices[if *first < 0 { 1 } else { 0 }])?;
    let mut total = 0.0f32;
    for i in 1..face.edge_count - 1 {
        let number = aas_at(&world.edge_indexes, face.first_edge + i)?;
        let current = aas_at(&world.edges, number.unsigned_abs() as i32)?;
        let first_vertex = aas_at(&world.vertices, current.vertices[if *number < 0 { 1 } else { 0 }])?;
        let last_vertex = aas_at(&world.vertices, current.vertices[if *number < 0 { 0 } else { 1 }])?;
        total = (f64::from(total)
            + 0.5
                * f64::from(length3(cross3(
                    sub3(*first_vertex, *origin),
                    sub3(*last_vertex, *origin),
                )))) as f32;
    }
    Ok(total)
}

/// Volume of an area.
pub fn aas_area_volume(world: &AasAsset, area_number: i32) -> Result<f32, BotsError> {
    let area = aas_at(&world.areas, area_number)?;
    let first_face = aas_at(
        &world.faces,
        aas_at(&world.face_indexes, area.first_face)?.unsigned_abs() as i32,
    )?;
    let edge = aas_at(
        &world.edges,
        aas_at(&world.edge_indexes, first_face.first_edge)?.unsigned_abs() as i32,
    )?;
    let corner = aas_at(&world.vertices, edge.vertices[0])?;
    let mut volume = 0.0f32;
    for i in 0..area.face_count {
        let face = aas_at(
            &world.faces,
            aas_at(&world.face_indexes, area.first_face + i)?.unsigned_abs() as i32,
        )?;
        let plane = aas_at(&world.planes, face.plane ^ i32::from(face.back_area != area_number))?;
        let distance = -((f64::from(dot3(*corner, plane.normal)) - f64::from(plane.distance)) as f32);
        volume = (volume + distance * aas_face_area(world, face)?) as f32;
    }
    Ok((f64::from(volume) / 3.0) as f32)
}

/// Ground-face surface area of an area.
pub fn aas_area_ground_face_area(world: &AasAsset, area_number: i32) -> Result<f32, BotsError> {
    let area = aas_at(&world.areas, area_number)?;
    let mut total = 0.0f32;
    for i in 0..area.face_count {
        let face = aas_at(
            &world.faces,
            aas_at(&world.face_indexes, area.first_face + i)?.unsigned_abs() as i32,
        )?;
        if (face.flags & 4) != 0 {
            total = (f64::from(total) + f64::from(aas_face_area(world, face)?)) as f32;
        }
    }
    Ok(total)
}

/// Centroid of a face's edge endpoints.
pub fn aas_face_center(world: &AasAsset, face_number: i32) -> Result<Vec3, BotsError> {
    let face = aas_at(&world.faces, face_number)?;
    let mut center = ZERO;
    for i in 0..face.edge_count {
        let edge = aas_at(
            &world.edges,
            aas_at(&world.edge_indexes, face.first_edge + i)?.unsigned_abs() as i32,
        )?;
        center = add3(center, *aas_at(&world.vertices, edge.vertices[0])?);
        center = add3(center, *aas_at(&world.vertices, edge.vertices[1])?);
    }
    Ok(scale3(center, (0.5 / f64::from(face.edge_count)) as f32))
}

/// Fall distance that deals damage.
pub fn aas_fall_damage_distance(settings: &AasMovementSettings) -> Result<i32, BotsError> {
    let velocity = (30.0f64 * 10000.0).sqrt() as f32;
    let time = (f64::from(velocity) / settings.gravity) as f32;
    source_integer(0.5 * settings.gravity * f64::from(time) * f64::from(time))
}

/// Fall delta for a drop distance.
pub fn aas_fall_delta(settings: &AasMovementSettings, distance: f32) -> f32 {
    let time = ((f64::from(distance).abs() * 2.0 / settings.gravity).sqrt()) as f32;
    let delta = (f64::from(time) * settings.gravity) as f32;
    (f64::from(delta) * f64::from(delta) * 0.0001) as f32
}

/// Maximum jump height for a vertical velocity.
pub fn aas_max_jump_height(settings: &AasMovementSettings, velocity: f64) -> f32 {
    let time = (velocity / settings.gravity) as f32;
    (0.5 * settings.gravity * f64::from(time) * f64::from(time)) as f32
}

/// Maximum jump distance for a vertical velocity.
pub fn aas_max_jump_distance(settings: &AasMovementSettings, velocity: f64) -> f32 {
    let time = (settings.max_jump_fall_height / (0.5 * settings.gravity)).sqrt() as f32;
    let ascent = (velocity / settings.gravity) as f32;
    let total = (f64::from(time) + f64::from(ascent)) as f32;
    (settings.max_velocity * f64::from(total)) as f32
}

/// Barrier-jump travel time.
pub fn aas_barrier_jump_travel_time(settings: &AasMovementSettings) -> Result<i32, BotsError> {
    Ok(source_integer(settings.jump_velocity / (settings.gravity * 0.1))? & 0xffff)
}

/// Closest-edge-point range between two edges.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasClosestEdgeRange {
    /// First range start.
    pub start1: Vec3,
    /// Second range start.
    pub start2: Vec3,
    /// First range end.
    pub end1: Vec3,
    /// Second range end.
    pub end2: Vec3,
}

/// Mutable closest-edge-point search state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasClosestEdgeState {
    /// Active range.
    pub range: Option<AasClosestEdgeRange>,
}

/// Active `AAS_ClosestEdgePoints`, including its original horizontal
/// projection formula. The eight parameters mirror the donor signature.
#[allow(clippy::too_many_arguments)]
pub fn aas_closest_edge_points(
    v1: Vec3,
    v2: Vec3,
    v3: Vec3,
    v4: Vec3,
    plane1: &AasPlane,
    plane2: &AasPlane,
    state: &mut AasClosestEdgeState,
    mut best_distance: f64,
) -> Result<f64, BotsError> {
    let direction1 = with_z(sub3(v2, v1), 0.0);
    let direction2 = with_z(sub3(v4, v3), 0.0);
    fn projection(point: Vec3, edge_start: Vec3, direction: Vec3, plane: &AasPlane) -> Vec3 {
        let flat = if direction.x != 0.0 {
            let a = (f64::from(direction.y) / f64::from(direction.x)) as f32;
            let b = (f64::from(edge_start.y) - f64::from((f64::from(a) * f64::from(edge_start.x)) as f32)) as f32;
            let x = ((f64::from(dot3(point, direction))
                - f64::from(
                    ((f64::from(a) * f64::from(direction.x)) as f32 as f64
                        + f64::from((f64::from(b) * f64::from(direction.y)) as f32)) as f32,
                ))
                / f64::from(direction.x)) as f32;
            Vec3 {
                x,
                y: (f64::from((f64::from(a) * f64::from(x)) as f32) + f64::from(b)) as f32,
                z: 0.0,
            }
        } else {
            Vec3 {
                x: edge_start.x,
                y: point.y,
                z: 0.0,
            }
        };
        with_z(
            flat,
            f64::from(
                ((f64::from(plane.distance) - f64::from(dot3(plane.normal, flat))) as f32 as f64
                    / f64::from(plane.normal.z)) as f32,
            ),
        )
    }
    let p1 = projection(v1, v3, direction2, plane2);
    let p2 = projection(v2, v3, direction2, plane2);
    let p3 = projection(v3, v1, direction1, plane1);
    let p4 = projection(v4, v1, direction1, plane1);
    let distance = |a: Vec3, b: Vec3| length3(sub3(b, a));
    let between = |point: Vec3, a: Vec3, b: Vec3| dot3(sub3(point, a), sub3(point, b)) <= 0.0;
    let mut update = |start: Vec3, end: Vec3, allow_range: bool| -> Result<(), BotsError> {
        let current = distance(start, end);
        if allow_range && f64::from(current) > best_distance - 0.5 && f64::from(current) < best_distance + 0.5 {
            let range = state.range.ok_or_else(|| {
                BotsError::Internal("AAS_ClosestEdgePoints reads an uninitialized closest-point range".to_string())
            })?;
            let (mut start1, mut start2, mut end1, mut end2) = (range.start1, range.start2, range.end1, range.end2);
            let first_start = distance(start1, start);
            let second_start = distance(start2, start);
            if first_start > second_start {
                if first_start > distance(start1, start2) {
                    start2 = start;
                }
            } else if second_start > distance(start1, start2) {
                start1 = start;
            }
            let first_end = distance(end1, end);
            let second_end = distance(end2, end);
            if first_end > second_end {
                if first_end > distance(end1, end2) {
                    end2 = end;
                }
            } else if second_end > distance(end1, end2) {
                end1 = end;
            }
            state.range = Some(AasClosestEdgeRange {
                start1,
                start2,
                end1,
                end2,
            });
        } else if f64::from(current) < best_distance {
            best_distance = f64::from(current);
            state.range = Some(AasClosestEdgeRange {
                start1: start,
                start2: start,
                end1: end,
                end2: end,
            });
        }
        Ok(())
    };
    let mut found = false;
    if between(p1, v3, v4) {
        update(v1, p1, true)?;
        found = true;
    }
    if between(p2, v3, v4) {
        update(v2, p2, true)?;
        found = true;
    }
    if between(p3, v1, v2) {
        update(p3, v3, true)?;
        found = true;
    }
    if between(p4, v1, v2) {
        update(p4, v4, true)?;
        found = true;
    }
    if !found {
        update(v1, v3, false)?;
        update(v1, v4, false)?;
        update(v2, v3, false)?;
        update(v2, v4, false)?;
    }
    Ok(best_distance)
}

struct StepCandidate {
    distance: f32,
    length: f32,
    edge: i32,
    start: Vec3,
    end: Vec3,
    normal: Vec3,
}

impl<'a> AasReachabilityContext<'a> {
    pub(crate) fn grounded(&self, area: i32) -> Result<bool, BotsError> {
        Ok((self.world.setting(area)?.flags & 1) != 0)
    }

    pub(crate) fn swim_area(&self, area: i32) -> Result<bool, BotsError> {
        Ok((self.world.setting(area)?.flags & 4) != 0)
    }

    pub(crate) fn crouch(&self, area: i32) -> Result<bool, BotsError> {
        Ok((self.world.setting(area)?.presence & 2) == 0)
    }

    pub(crate) fn ladder_area(&self, area: i32) -> Result<bool, BotsError> {
        Ok((self.world.setting(area)?.flags & 2) != 0)
    }

    /// Whether the far side of a ledge step is solid or a gap.
    /// The donor defines but never calls this probe; keep the port for
    /// behavior callers.
    #[allow(dead_code)]
    pub(crate) fn nearby_solid_or_gap(&self, start: Vec3, end: Vec3) -> Result<bool, BotsError> {
        let direction = normalize3(with_z(sub3(end, start), 0.0));
        let mut point = ma(end, 48.0, direction);
        let mut area = self.world.point_area(point)?;
        if area == 0 {
            point = with_z(point, f64::from(point.z) + 16.0);
            area = self.world.point_area(point)?;
            if area == 0 {
                return Ok(true);
            }
        }
        area = self.world.point_area(ma(end, 64.0, direction))?;
        Ok(area != 0 && !self.swim_area(area)? && !self.grounded(area)?)
    }

    /// Swim reachability through a shared water face.
    pub(crate) fn swim(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        if !self.swim_area(from)? || !self.swim_area(to)? || self.crouch(to)? {
            return Ok(false);
        }
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        if !close_bounds(area1.bounds, area2.bounds, true) {
            return Ok(false);
        }
        for i in 0..area1.face_count {
            let signed_face = *aas_at(&self.world.asset.face_indexes, area1.first_face + i)?;
            let face_number = signed_face.unsigned_abs() as i32;
            for j in 0..area2.face_count {
                if face_number != aas_at(&self.world.asset.face_indexes, area2.first_face + j)?.unsigned_abs() as i32 {
                    continue;
                }
                let start = aas_face_center(&self.world.asset, face_number)?;
                if (self.host_point_contents(start) & 56) == 0 {
                    continue;
                }
                let face = *aas_at(&self.world.asset.faces, face_number)?;
                let Some(id) = self.allocate() else {
                    return Ok(false);
                };
                let plane = *aas_at(&self.world.asset.planes, face.plane ^ i32::from(signed_face < 0))?;
                let end = ma(start, -2.0, plane.normal);
                let volume = aas_area_volume(&self.world.asset, to)?;
                {
                    let slot = self.slot_mut(id)?;
                    slot.link.area = to;
                    slot.link.face = face_number;
                    slot.link.edge = 0;
                    slot.link.start = start;
                    slot.link.end = end;
                    slot.link.travel_type = TravelType::SWIM;
                    slot.link.set_travel_time(1.0)?;
                    if volume < 800.0 {
                        let time = f64::from(slot.link.travel_time) + 200.0;
                        slot.link.set_travel_time(time)?;
                    }
                }
                self.link_area(from, id)?;
                self.debug_state.count("swim");
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Walk reachability across a shared ground edge.
    pub(crate) fn equal_floor_height(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        let settings = self.movement_settings.clone();
        if !self.grounded(from)? || !self.grounded(to)? {
            return Ok(false);
        }
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        if !close_bounds(area1.bounds, area2.bounds, false) || area2.bounds.min.z > area1.bounds.max.z {
            return Ok(false);
        }
        let mut best: Option<(f32, f32, i32, Vec3, Vec3)> = None;
        for i in 0..area1.face_count {
            let face1 = *aas_at(
                &self.world.asset.faces,
                aas_at(&self.world.asset.face_indexes, area1.first_face + i)?.unsigned_abs() as i32,
            )?;
            if (face1.flags & 4) == 0 {
                continue;
            }
            for j in 0..area2.face_count {
                let face2 = *aas_at(
                    &self.world.asset.faces,
                    aas_at(&self.world.asset.face_indexes, area2.first_face + j)?.unsigned_abs() as i32,
                )?;
                if (face2.flags & 4) == 0 {
                    continue;
                }
                for first in 0..face1.edge_count {
                    for second in 0..face2.edge_count {
                        let number = *aas_at(&self.world.asset.edge_indexes, face1.first_edge + first)?;
                        if number.unsigned_abs()
                            != aas_at(&self.world.asset.edge_indexes, face2.first_edge + second)?.unsigned_abs()
                        {
                            continue;
                        }
                        let edge = *aas_at(&self.world.asset.edges, number.unsigned_abs() as i32)?;
                        let v0 = *aas_at(&self.world.asset.vertices, edge.vertices[0])?;
                        let v1 = *aas_at(&self.world.asset.vertices, edge.vertices[1])?;
                        let length = length3(sub3(v1, v0));
                        let midpoint = scale3(add3(v0, v1), 0.5);
                        let edge_vector = if number < 0 { sub3(v1, v0) } else { sub3(v0, v1) };
                        let plane2 = *aas_at(&self.world.asset.planes, face2.plane)?;
                        let normal = normalize3(cross3(edge_vector, plane2.normal));
                        let start = ma_double(midpoint, 0.1, normal);
                        let raw_end = ma(midpoint, 5.0, normal);
                        let end = with_z(raw_end, f64::from(raw_end.z) + 0.125);
                        let height = dot3(UP, start);
                        let (best_height, best_length) = best.map_or((99999.0f32, 0.0f32), |(h, l, _, _, _)| (h, l));
                        if height < best_height
                            || (height < (f64::from(best_height) + 1.0) as f32 && length > best_length)
                        {
                            best = Some((height, length, number, start, end));
                        }
                    }
                }
            }
        }
        let Some((_, _, edge, start, end)) = best else {
            return Ok(false);
        };
        let Some(id) = self.allocate() else {
            return Ok(false);
        };
        let crouch_from = self.crouch(from)?;
        let crouch_to = self.crouch(to)?;
        {
            let slot = self.slot_mut(id)?;
            slot.link.area = to;
            slot.link.face = 0;
            slot.link.edge = edge;
            slot.link.start = start;
            slot.link.end = end;
            slot.link.travel_type = TravelType::WALK;
            slot.link.set_travel_time(1.0)?;
            if !crouch_from && crouch_to {
                let time = f64::from(slot.link.travel_time) + settings.start_crouch_time;
                let time = time as f32;
                slot.link.set_travel_time(f64::from(time))?;
            }
        }
        self.link_area(from, id)?;
        self.debug_state.count("equal floor");
        Ok(true)
    }

    /// Step, barrier, water-jump, and walk-off-ledge reachabilities
    /// across shared ground edges.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn step_barrier_water_jump_walk_off_ledge(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        let settings = self.movement_settings.clone();
        if (!self.grounded(from)? && !self.swim_area(from)?) || (!self.grounded(to)? && !self.swim_area(to)?) {
            return Ok(false);
        }
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        let swim1 = self.swim_area(from)?;
        if !close_bounds(area1.bounds, area2.bounds, false) {
            return Ok(false);
        }
        let mut ground: Option<StepCandidate> = None;
        let mut water: Option<StepCandidate> = None;
        for i in 0..area1.face_count {
            let face_number = *aas_at(&self.world.asset.face_indexes, area1.first_face + i)?;
            let face_side = face_number < 0;
            let face1 = *aas_at(&self.world.asset.faces, face_number.unsigned_abs() as i32)?;
            if (face1.flags & 4) == 0 {
                if !swim1 {
                    continue;
                }
                let plane = aas_at(&self.world.asset.planes, face1.plane ^ i32::from(!face_side))?;
                if dot3(plane.normal, UP) < 0.7 {
                    continue;
                }
            }
            for k in 0..face1.edge_count {
                let signed_edge = *aas_at(&self.world.asset.edge_indexes, face1.first_edge + k)?;
                let edge_number = signed_edge.unsigned_abs() as i32;
                let mut side = signed_edge < 0;
                if (face1.flags & 4) == 0 {
                    side = side == face_side;
                }
                let edge1 = *aas_at(&self.world.asset.edges, edge_number)?;
                let mut v1 = *aas_at(&self.world.asset.vertices, edge1.vertices[usize::from(!side)])?;
                let mut v2 = *aas_at(&self.world.asset.vertices, edge1.vertices[usize::from(side)])?;
                let normal = normalize3(cross3(sub3(v2, v1), UP));
                // Source reuses dist for the vertical separation inside
                // the nested edge loop.
                let dist_plane = dot3(normal, v1);
                for j in 0..area2.face_count {
                    let face2 = *aas_at(
                        &self.world.asset.faces,
                        aas_at(&self.world.asset.face_indexes, area2.first_face + j)?.unsigned_abs() as i32,
                    )?;
                    if (face2.flags & 4) == 0 {
                        continue;
                    }
                    for l in 0..face2.edge_count {
                        let edge2 = *aas_at(
                            &self.world.asset.edges,
                            aas_at(&self.world.asset.edge_indexes, face2.first_edge + l)?.unsigned_abs() as i32,
                        )?;
                        let mut v3 = *aas_at(&self.world.asset.vertices, edge2.vertices[0])?;
                        let mut v4 = *aas_at(&self.world.asset.vertices, edge2.vertices[1])?;
                        let diff1 = (f64::from(dot3(normal, v3)) - f64::from(dist_plane)) as f32;
                        let diff2 = (f64::from(dot3(normal, v4)) - f64::from(dist_plane)) as f32;
                        if diff1 < -0.1 || diff1 > 0.1 || diff2 < -0.1 || diff2 > 0.1 {
                            continue;
                        }
                        let ort = cross3(UP, normal);
                        let ort_dot = dot3(ort, ort);
                        let mut y1 = v1.z;
                        let mut y2 = v2.z;
                        let mut y3 = v3.z;
                        let mut y4 = v4.z;
                        let mut x1 = (f64::from(dot3(v1, ort)) / f64::from(ort_dot)) as f32;
                        let mut x2 = (f64::from(dot3(v2, ort)) / f64::from(ort_dot)) as f32;
                        let mut x3 = (f64::from(dot3(v3, ort)) / f64::from(ort_dot)) as f32;
                        let mut x4 = (f64::from(dot3(v4, ort)) / f64::from(ort_dot)) as f32;
                        if x1 > x2 {
                            std::mem::swap(&mut x1, &mut x2);
                            std::mem::swap(&mut y1, &mut y2);
                            std::mem::swap(&mut v1, &mut v2);
                        }
                        if x3 > x4 {
                            std::mem::swap(&mut x3, &mut x4);
                            std::mem::swap(&mut y3, &mut y4);
                            std::mem::swap(&mut v3, &mut v4);
                        }
                        if x2 <= x3 || x4 <= x1 {
                            continue;
                        }
                        // Segment interpolation with the donor's rounding
                        // order: binary32 differences and product, one
                        // narrowed division, one binary32 sum.
                        let interp = |y0: f32, x: f32, xa: f32, xb: f32, ya: f32, yb: f32| {
                            let ratio = (f64::from((x - xa) * (yb - ya)) / f64::from(xb - xa)) as f32;
                            y0 + ratio
                        };
                        let (dist, start, end, q12, q22);
                        if x1 - 0.5 < x3 && x4 < x2 + 0.5 && x3 - 0.5 < x1 && x2 < x4 + 0.5 {
                            let dist1 = y3 - y1;
                            let dist2 = y4 - y2;
                            if dist1 > dist2 - 1.0 && dist1 < dist2 + 1.0 {
                                dist = dist1;
                                start = scale3(add3(v1, v2), 0.5);
                                end = scale3(add3(v3, v4), 0.5);
                            } else if dist1 < dist2 {
                                dist = dist1;
                                start = v1;
                                end = v3;
                            } else {
                                dist = dist2;
                                start = v2;
                                end = v4;
                            }
                            q12 = v3;
                            q22 = v4;
                        } else {
                            let (dist1, pa11, pa12) = if x1 > x3 - 0.1 && x1 < x3 + 0.1 {
                                (y3 - y1, v1, v3)
                            } else if x1 < x3 {
                                let y = interp(y1, x3, x1, x2, y1, y2);
                                (y3 - y, with_z(v3, f64::from(y)), v3)
                            } else {
                                let y = interp(y3, x1, x3, x4, y3, y4);
                                (y - y1, v1, with_z(v1, f64::from(y)))
                            };
                            let (dist2, pa21, pa22) = if x2 > x4 - 0.1 && x2 < x4 + 0.1 {
                                (y4 - y2, v2, v4)
                            } else if x2 < x4 {
                                let y = interp(y3, x2, x3, x4, y3, y4);
                                (y - y2, v2, with_z(v2, f64::from(y)))
                            } else {
                                let y = interp(y1, x4, x1, x2, y1, y2);
                                (y4 - y, with_z(v4, f64::from(y)), v4)
                            };
                            if dist1 > dist2 - 1.0 && dist1 < dist2 + 1.0 {
                                dist = dist1;
                                start = scale3(add3(pa11, pa21), 0.5);
                                end = scale3(add3(pa12, pa22), 0.5);
                            } else if dist1 < dist2 {
                                dist = dist1;
                                start = pa11;
                                end = pa12;
                            } else {
                                dist = dist2;
                                start = pa21;
                                end = pa22;
                            }
                            q12 = pa12;
                            q22 = pa22;
                        }
                        let length = length3(sub3(q22, q12));
                        let prior = if (face1.flags & 4) != 0 { &ground } else { &water };
                        let (best_dist, best_length) = prior
                            .as_ref()
                            .map_or((99999.0f32, 0.0f32), |prior| (prior.distance, prior.length));
                        if dist < best_dist || (dist < (f64::from(best_dist) + 1.0) as f32 && length > best_length) {
                            let candidate = StepCandidate {
                                distance: dist,
                                length,
                                edge: edge_number,
                                start,
                                end,
                                normal,
                            };
                            if (face1.flags & 4) != 0 {
                                ground = Some(candidate);
                            } else {
                                water = Some(candidate);
                            }
                        }
                    }
                }
            }
        }
        if let Some(candidate) = &ground {
            if candidate.distance >= 0.0 && candidate.distance < settings.max_step as f32 {
                let Some(id) = self.allocate() else {
                    return Ok(false);
                };
                let crouch_from = self.crouch(from)?;
                let crouch_to = self.crouch(to)?;
                {
                    let slot = self.slot_mut(id)?;
                    slot.link.area = to;
                    slot.link.face = 0;
                    slot.link.edge = candidate.edge;
                    slot.link.start = ma_double(candidate.start, 0.1, candidate.normal);
                    slot.link.end = ma(candidate.end, 5.0, candidate.normal);
                    slot.link.travel_type = TravelType::WALK;
                    slot.link.set_travel_time(0.0)?;
                    if !crouch_from && crouch_to {
                        let time = (f64::from(slot.link.travel_time) + settings.start_crouch_time) as f32;
                        slot.link.set_travel_time(f64::from(time))?;
                    }
                }
                self.link_area(from, id)?;
                self.debug_state.count("step");
                return Ok(true);
            }
        }
        if let Some(candidate) = &water {
            let point = ma(candidate.end, -2.0, candidate.normal);
            let test_point = with_z(point, f64::from(point.z) - settings.max_water_jump);
            let max_water_jump = settings.max_water_jump;
            if self.swim_area(self.world.point_area(test_point)?)?
                && candidate.distance < (max_water_jump + 24.0) as f32
                && !self.crouch(from)?
                && !self.crouch(to)?
            {
                let Some(id) = self.allocate() else {
                    return Ok(false);
                };
                {
                    let slot = self.slot_mut(id)?;
                    slot.link.area = to;
                    slot.link.face = 0;
                    slot.link.edge = candidate.edge;
                    slot.link.start = candidate.start;
                    slot.link.end = ma(candidate.end, 15.0, candidate.normal);
                    slot.link.travel_type = TravelType::WATERJUMP;
                    slot.link.set_travel_time(settings.water_jump_time)?;
                }
                self.link_area(from, id)?;
                self.debug_state.count("waterjump");
                return Ok(true);
            }
        }
        if let Some(candidate) = &ground {
            let max_barrier = settings.max_barrier;
            let water_ok = water
                .as_ref()
                .is_none_or(|water| ((f64::from(candidate.distance) - f64::from(water.distance)) as f32) < 16.0);
            if candidate.distance > 0.0
                && candidate.distance < max_barrier as f32
                && water_ok
                && !self.crouch(from)?
                && !self.crouch(to)?
            {
                let Some(id) = self.allocate() else {
                    return Ok(false);
                };
                {
                    let slot = self.slot_mut(id)?;
                    slot.link.area = to;
                    slot.link.face = 0;
                    slot.link.edge = candidate.edge;
                    slot.link.start = ma_double(candidate.start, 0.1, candidate.normal);
                    slot.link.end = ma(candidate.end, 5.0, candidate.normal);
                    slot.link.travel_type = TravelType::BARRIERJUMP;
                    slot.link.set_travel_time(settings.barrier_jump_time)?;
                }
                self.link_area(from, id)?;
                self.debug_state.count("barrier");
                return Ok(true);
            }
        }
        let Some(candidate) = &ground else {
            return Ok(false);
        };
        if candidate.distance >= 0.0 {
            return Ok(false);
        }
        if candidate.distance > -(settings.max_step as f32) {
            let Some(id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(id)?;
                slot.link.area = to;
                slot.link.face = 0;
                slot.link.edge = candidate.edge;
                slot.link.start = ma_double(candidate.start, 0.1, candidate.normal);
                slot.link.end = ma(candidate.end, 5.0, candidate.normal);
                slot.link.travel_type = TravelType::WALK;
                slot.link.set_travel_time(1.0)?;
            }
            self.link_area(from, id)?;
            self.debug_state.count("walk");
            return Ok(true);
        }
        // Donor negation order: an incomparable distance rejects the link.
        if settings.max_fall_height != 0.0
            && candidate.distance.abs().partial_cmp(&(settings.max_fall_height as f32)) != Some(Ordering::Less)
        {
            return Ok(false);
        }
        let ground_end = ma(candidate.end, 2.0, candidate.normal);
        let start = with_z(ground_end, f64::from(candidate.start.z));
        let end = with_z(ground_end, f64::from(ground_end.z) + 4.0);
        let trace = self.trace_client_bbox(start, end, 2)?;
        // Donor negation order: an incomparable fraction rejects the link.
        if trace.start_solid
            || !matches!(
                trace.fraction.partial_cmp(&1.0),
                Some(Ordering::Equal | Ordering::Greater)
            )
            || self.world.point_area(with_z(trace.end, f64::from(trace.end.z) + 1.0))? != to
        {
            return Ok(false);
        }
        for crossing in self.trace_areas(start, end, 10)? {
            if (self.world.setting(crossing.area)?.contents & 8) != 0 {
                return Ok(false);
            }
        }
        let distance = candidate.distance;
        let edge = candidate.edge;
        let candidate_start = candidate.start;
        let Some(id) = self.allocate() else {
            return Ok(false);
        };
        let swim_to = self.swim_area(to)?;
        let to_contents = self.world.setting(to)?.contents;
        {
            let slot = self.slot_mut(id)?;
            slot.link.area = to;
            slot.link.face = 0;
            slot.link.edge = edge;
            slot.link.start = candidate_start;
            slot.link.end = ground_end;
            slot.link.travel_type = TravelType::WALKOFFLEDGE;
            slot.link.set_travel_time(
                settings.start_walk_off_ledge_time + f64::from(distance).abs() * 50.0 / settings.gravity,
            )?;
            if !swim_to && (to_contents & 128) == 0 {
                if aas_fall_delta(&settings, distance) > settings.fall_delta5 as f32 {
                    let time = (f64::from(slot.link.travel_time) + settings.fall_damage5_time) as f32;
                    slot.link.set_travel_time(f64::from(time))?;
                }
                if aas_fall_delta(&settings, distance) > settings.fall_delta10 as f32 {
                    let time = (f64::from(slot.link.travel_time) + settings.fall_damage10_time) as f32;
                    slot.link.set_travel_time(f64::from(time))?;
                }
            }
        }
        self.link_area(from, id)?;
        self.debug_state.count("walkoffledge");
        Ok(true)
    }

    /// Jump reachability between ground faces. The source returns false
    /// even after publishing a jump reachability.
    pub(crate) fn jump(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        let settings = self.movement_settings.clone();
        if !self.grounded(from)? || !self.grounded(to)? || self.crouch(from)? || self.crouch(to)? {
            return Ok(false);
        }
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        let maximum_distance = (2.0 * f64::from(aas_max_jump_distance(&settings, settings.jump_velocity))) as f32;
        for axis in [0usize, 1usize] {
            let (a_min, a_max, b_min, b_max) = match axis {
                0 => (
                    area1.bounds.min.x,
                    area1.bounds.max.x,
                    area2.bounds.min.x,
                    area2.bounds.max.x,
                ),
                _ => (
                    area1.bounds.min.y,
                    area1.bounds.max.y,
                    area2.bounds.min.y,
                    area2.bounds.max.y,
                ),
            };
            if a_min > b_max + maximum_distance || a_max < b_min - maximum_distance {
                return Ok(false);
            }
        }
        if area2.bounds.min.z > area1.bounds.max.z + aas_max_jump_height(&settings, settings.jump_velocity) {
            return Ok(false);
        }
        let mut best_distance = 999999.0f64;
        let mut state = AasClosestEdgeState { range: None };
        for i in 0..area1.face_count {
            let face1 = *aas_at(
                &self.world.asset.faces,
                aas_at(&self.world.asset.face_indexes, area1.first_face + i)?.unsigned_abs() as i32,
            )?;
            if (face1.flags & 4) == 0 {
                continue;
            }
            for j in 0..area2.face_count {
                let face2 = *aas_at(
                    &self.world.asset.faces,
                    aas_at(&self.world.asset.face_indexes, area2.first_face + j)?.unsigned_abs() as i32,
                )?;
                if (face2.flags & 4) == 0 {
                    continue;
                }
                for k in 0..face1.edge_count {
                    let edge1 = *aas_at(
                        &self.world.asset.edges,
                        aas_at(&self.world.asset.edge_indexes, face1.first_edge + k)?.unsigned_abs() as i32,
                    )?;
                    for l in 0..face2.edge_count {
                        let edge2 = *aas_at(
                            &self.world.asset.edges,
                            aas_at(&self.world.asset.edge_indexes, face2.first_edge + l)?.unsigned_abs() as i32,
                        )?;
                        let plane1 = *aas_at(&self.world.asset.planes, face1.plane)?;
                        let plane2 = *aas_at(&self.world.asset.planes, face2.plane)?;
                        best_distance = aas_closest_edge_points(
                            *aas_at(&self.world.asset.vertices, edge1.vertices[0])?,
                            *aas_at(&self.world.asset.vertices, edge1.vertices[1])?,
                            *aas_at(&self.world.asset.vertices, edge2.vertices[0])?,
                            *aas_at(&self.world.asset.vertices, edge2.vertices[1])?,
                            &plane1,
                            &plane2,
                            &mut state,
                            best_distance,
                        )?;
                    }
                }
            }
        }
        // The source calculates unused midpoints before this gate, even
        // when optimized AAS has no ground edges.
        if !(best_distance > 4.0 && best_distance < f64::from(maximum_distance)) {
            return Ok(false);
        }
        let range = state.range.ok_or_else(|| {
            BotsError::Internal("AAS_Reachability_Jump reads uninitialized closest edge points".to_string())
        })?;
        let best_start = scale3(add3(range.start1, range.start2), 0.5);
        let best_end = scale3(add3(range.end1, range.end2), 0.5);
        let (speed, travel_type): (f32, i32) =
            if best_distance <= 48.0 && ((f64::from(best_start.z) - f64::from(best_end.z)) as f32).abs() < 8.0 {
                (400.0, TravelType::WALKOFFLEDGE)
            } else {
                let fall = self.movement_horizontal_velocity_for_jump(0.0, best_start, best_end);
                if fall.success {
                    (
                        (f64::from(fall.velocity) * f64::from(1.2f32)) as f32,
                        TravelType::WALKOFFLEDGE,
                    )
                } else {
                    let jump = self.movement_horizontal_velocity_for_jump(settings.jump_velocity, best_start, best_end);
                    if !jump.success {
                        return Ok(false);
                    }
                    if length3(with_z(sub3(best_end, best_start), 0.0)) < 10.0 {
                        return Ok(false);
                    }
                    ((f64::from(jump.velocity) * f64::from(1.05f32)) as f32, TravelType::JUMP)
                }
            };
        let direction = normalize3(sub3(best_end, best_start));
        for test_start in [ma(best_start, 1.0, direction), ma(best_end, -1.0, direction)] {
            let trace = self.trace_client_bbox(test_start, with_z(test_start, f64::from(test_start.z) - 100.0), 2)?;
            if trace.start_solid {
                return Ok(false);
            }
            let plane = *aas_at(&self.world.asset.planes, trace.plane)?;
            if trace.fraction < 1.0
                && dot3(plane.normal, UP) >= 0.7
                && (self.host_point_contents(trace.end) & 24) == 0
                && ((f64::from(test_start.z) - f64::from(trace.end.z)) as f32) <= settings.max_barrier as f32
            {
                return Ok(false);
            }
        }
        let command = Vec3 {
            x: 0.0,
            y: 0.0,
            z: if (travel_type & TravelType::MASK) == TravelType::JUMP {
                settings.jump_velocity as f32
            } else {
                0.0
            },
        };
        let direction = normalize3(with_z(sub3(best_end, best_start), 0.0));
        let sideways = cross3(direction, UP);
        let mut stop_events = AasStopEvent::HIT_GROUND
            | AasStopEvent::ENTER_WATER
            | AasStopEvent::ENTER_SLIME
            | AasStopEvent::ENTER_LAVA
            | AasStopEvent::HIT_GROUND_DAMAGE;
        if (self.world.setting(from)?.contents & 8) == 0 && (self.world.setting(to)?.contents & 8) == 0 {
            stop_events |= AasStopEvent::TOUCH_CLUSTER_PORTAL;
        }
        let mut found = false;
        for i in 0..3 {
            let test_end = if i == 1 {
                add3(best_end, sideways)
            } else if i == 2 {
                sub3(best_end, sideways)
            } else {
                best_end
            };
            let direction = normalize3(with_z(sub3(test_end, best_start), 0.0));
            let predicted = self.movement_predict_client(crate::behavior::BotMovementPrediction {
                entity_num: -1,
                origin: best_start,
                presence: 2,
                on_ground: true,
                velocity: scale3(direction, speed),
                command_move: command,
                command_frames: 3,
                max_frames: 30,
                frame_time: 0.1,
                stop_events,
                stop_area: 0,
                visualize: false,
            })?;
            let movement = predicted;
            if movement.frames >= 30
                || (movement.stop_event
                    & (AasStopEvent::ENTER_SLIME | AasStopEvent::ENTER_LAVA | AasStopEvent::TOUCH_CLUSTER_PORTAL))
                    != 0
            {
                return Ok(false);
            }
            let test_start = ma(movement.end, -64.0, direction);
            found = self
                .trace_areas(movement.end, with_z(test_start, f64::from(test_start.z) + 1.0), 10)?
                .iter()
                .any(|crossing| crossing.area == to);
            if found {
                break;
            }
        }
        if !found {
            return Ok(false);
        }
        if self.debug {
            self.log(&format!("jump reachability between {from} and {to}\r\n"));
        }
        let Some(id) = self.allocate() else {
            return Ok(false);
        };
        let to_contents = self.world.setting(to)?.contents;
        {
            let slot = self.slot_mut(id)?;
            slot.link.area = to;
            slot.link.face = 0;
            slot.link.edge = 0;
            slot.link.start = best_start;
            slot.link.end = best_end;
            slot.link.travel_type = travel_type;
            let delta = sub3(best_end, best_start);
            let height = delta.z;
            if (travel_type & TravelType::MASK) == TravelType::WALKOFFLEDGE && height > length3(with_z(delta, 0.0)) {
                let time = (settings.start_walk_off_ledge_time
                    + f64::from(((f64::from(height) * 50.0) / settings.gravity) as f32))
                    as f32;
                slot.link.set_travel_time(f64::from(time))?;
            } else {
                let time = (settings.start_jump_time
                    + f64::from(
                        ((f64::from(length3(sub3(best_start, best_end))) * 240.0) / settings.max_walk_velocity) as f32,
                    )) as f32;
                slot.link.set_travel_time(f64::from(time))?;
            }
            if (to_contents & 128) == 0 {
                let drop = (f64::from(best_start.z) - f64::from(best_end.z)) as f32;
                if aas_fall_delta(&settings, drop) > settings.fall_delta5 as f32 {
                    let time = (f64::from(slot.link.travel_time) + settings.fall_damage5_time) as f32;
                    slot.link.set_travel_time(f64::from(time))?;
                } else if aas_fall_delta(&settings, drop) > settings.fall_delta10 as f32 {
                    let time = (f64::from(slot.link.travel_time) + settings.fall_damage10_time) as f32;
                    slot.link.set_travel_time(f64::from(time))?;
                }
            }
        }
        self.link_area(from, id)?;
        self.debug_state
            .count(if (travel_type & TravelType::MASK) == TravelType::JUMP {
                "jump"
            } else {
                "walkoffledge"
            });
        // The source returns false even after publishing a jump
        // reachability.
        Ok(false)
    }

    /// Ladder reachability across a shared ladder edge.
    pub(crate) fn ladder(&mut self, from: i32, to: i32) -> Result<bool, BotsError> {
        if !self.ladder_area(from)? || !self.ladder_area(to)? {
            return Ok(false);
        }
        let maximum_height = aas_max_jump_height(&self.movement_settings, self.movement_settings.jump_velocity);
        let area1 = *aas_at(&self.world.asset.areas, from)?;
        let area2 = *aas_at(&self.world.asset.areas, to)?;
        let mut best: Option<(AasFace, AasFace, i32, i32, f32, f32, i32)> = None;
        for i in 0..area1.face_count {
            let number1 = *aas_at(&self.world.asset.face_indexes, area1.first_face + i)?;
            let face1 = *aas_at(&self.world.asset.faces, number1.unsigned_abs() as i32)?;
            if (face1.flags & 2) == 0 {
                continue;
            }
            for j in 0..area2.face_count {
                let number2 = *aas_at(&self.world.asset.face_indexes, area2.first_face + j)?;
                let face2 = *aas_at(&self.world.asset.faces, number2.unsigned_abs() as i32)?;
                if (face2.flags & 2) == 0 {
                    continue;
                }
                let mut shared = false;
                for k in 0..face1.edge_count {
                    let edge1 = *aas_at(&self.world.asset.edge_indexes, face1.first_edge + k)?;
                    for l in 0..face2.edge_count {
                        if edge1.unsigned_abs()
                            != aas_at(&self.world.asset.edge_indexes, face2.first_edge + l)?.unsigned_abs()
                        {
                            continue;
                        }
                        let surface1 = aas_face_area(&self.world.asset, &face1)?;
                        let surface2 = aas_face_area(&self.world.asset, &face2)?;
                        let (best1, best2) = best.map_or((-9999.0f32, -9999.0f32), |(_, _, _, _, a1, a2, _)| (a1, a2));
                        if surface1 > best1 && surface2 > best2 {
                            best = Some((face1, face2, number1, number2, surface1, surface2, edge1));
                        }
                        shared = true;
                        break;
                    }
                    if shared {
                        break;
                    }
                }
            }
        }
        let Some((face1, face2, number1, number2, _, _, edge_number)) = best else {
            return Ok(false);
        };
        let edge = *aas_at(&self.world.asset.edges, edge_number.unsigned_abs() as i32)?;
        let v1 = *aas_at(&self.world.asset.vertices, edge.vertices[usize::from(edge_number < 0)])?;
        let v2 = *aas_at(&self.world.asset.vertices, edge.vertices[usize::from(edge_number >= 0)])?;
        let midpoint = scale3(add3(v1, v2), 0.5);
        let shared_edge = sub3(v2, v1);
        let plane1 = *aas_at(&self.world.asset.planes, face1.plane ^ i32::from(number1 < 0))?;
        let plane2 = *aas_at(&self.world.asset.planes, face2.plane ^ i32::from(number2 < 0))?;
        let direction = normalize3(cross3(plane1.normal, shared_edge));
        let point1 = ma(midpoint, -32.0, direction);
        let point2 = ma(midpoint, 32.0, direction);
        let vertical1 = (integer_abs(f64::from(dot3(plane1.normal, UP)))? as f64) < 0.1;
        let vertical2 = (integer_abs(f64::from(dot3(plane2.normal, UP)))? as f64) < 0.1;
        if !vertical1 && !vertical2 {
            return Ok(false);
        }
        if vertical1
            && vertical2
            && dot3(plane1.normal, plane2.normal) > 0.7
            && (integer_abs(f64::from(dot3(shared_edge, UP)))? as f64) < 0.7
        {
            let Some(first_id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(first_id)?;
                slot.link.area = to;
                slot.link.face = number1;
                slot.link.edge = edge_number.unsigned_abs() as i32;
                slot.link.start = point1;
                slot.link.end = ma(point2, -3.0, plane1.normal);
                slot.link.travel_type = TravelType::LADDER;
                slot.link.set_travel_time(10.0)?;
            }
            self.link_area(from, first_id)?;
            self.debug_state.count("ladder");
            let Some(second_id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(second_id)?;
                slot.link.area = from;
                slot.link.face = number2;
                slot.link.edge = edge_number.unsigned_abs() as i32;
                slot.link.start = point2;
                slot.link.end = ma(point1, -3.0, plane1.normal);
                slot.link.travel_type = TravelType::LADDER;
                slot.link.set_travel_time(10.0)?;
            }
            self.link_area(to, second_id)?;
            self.debug_state.count("ladder");
            return Ok(true);
        }
        if vertical1 && (face2.flags & 4) != 0 {
            let Some(first_id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(first_id)?;
                slot.link.area = to;
                slot.link.face = number1;
                slot.link.edge = edge_number.unsigned_abs() as i32;
                slot.link.start = point1;
                slot.link.end = ma(with_z(point2, f64::from(point2.z) + 16.0), -15.0, plane1.normal);
                slot.link.travel_type = TravelType::LADDER;
                slot.link.set_travel_time(10.0)?;
            }
            self.link_area(from, first_id)?;
            self.debug_state.count("ladder");
            let Some(second_id) = self.allocate() else {
                return Ok(false);
            };
            {
                let slot = self.slot_mut(second_id)?;
                slot.link.area = from;
                slot.link.face = number2;
                slot.link.edge = edge_number.unsigned_abs() as i32;
                slot.link.start = point2;
                slot.link.end = point1;
                slot.link.travel_type = TravelType::WALKOFFLEDGE;
                slot.link.set_travel_time(10.0)?;
            }
            self.link_area(to, second_id)?;
            self.debug_state.count("walkoffledge");
            return Ok(true);
        }
        if !vertical1 {
            return Ok(false);
        }
        let mut lowest: Option<(Vec3, i32)> = None;
        for i in 0..face1.edge_count {
            let number = aas_at(&self.world.asset.edge_indexes, face1.first_edge + i)?.unsigned_abs() as i32;
            let current = *aas_at(&self.world.asset.edges, number)?;
            let point = scale3(
                add3(
                    *aas_at(&self.world.asset.vertices, current.vertices[0])?,
                    *aas_at(&self.world.asset.vertices, current.vertices[1])?,
                ),
                0.5,
            );
            if point.z < lowest.map_or(99999.0, |(point, _)| point.z) {
                lowest = Some((point, number));
            }
        }
        let plane1 = *aas_at(&self.world.asset.planes, face1.plane)?;
        let Some((lowest_point, lowest_edge)) = lowest else {
            return Err(BotsError::Internal(
                "AAS_Reachability_Ladder reads an uninitialized lowest point".to_string(),
            ));
        };
        let offset = ma(lowest_point, 5.0, plane1.normal);
        let start = with_z(offset, f64::from(offset.z) + 5.0);
        let end = with_z(offset, f64::from(offset.z) - 100.0);
        let trace = self.trace_client_bbox(start, end, 2)?;
        if self.debug && trace.start_solid {
            self.log(&format!("trace from area {from} started in solid\r\n"));
        }
        let trace_end = with_z(trace.end, f64::from(trace.end.z) + 1.0);
        let destination = self.world.point_area(trace_end)?;
        let destination_area = *aas_at(&self.world.asset.areas, destination)?;
        for i in 0..destination_area.face_count {
            let face = *aas_at(
                &self.world.asset.faces,
                aas_at(&self.world.asset.face_indexes, destination_area.first_face + i)?.unsigned_abs() as i32,
            )?;
            if (face.flags & 2) != 0 {
                let normal = aas_at(&self.world.asset.planes, face.plane)?.normal;
                if (integer_abs(f64::from(dot3(normal, UP)))? as f64) < 0.1 {
                    return Ok(false);
                }
            }
        }
        if destination == from || self.exists(from, destination)? || self.exists(destination, from)? {
            return Ok(false);
        }
        let rise = (f64::from(start.z) - f64::from(trace_end.z)) as f32;
        // Donor negation order: an incomparable rise rejects the link.
        if rise.partial_cmp(&maximum_height) != Some(Ordering::Less) {
            if self.debug {
                self.log(&format!("jump too high between area {destination} and {from}\r\n"));
            }
            return Ok(false);
        }
        let Some(first_id) = self.allocate() else {
            return Ok(false);
        };
        {
            let slot = self.slot_mut(first_id)?;
            slot.link.area = destination;
            slot.link.face = number1;
            slot.link.edge = lowest_edge;
            slot.link.start = lowest_point;
            slot.link.end = trace_end;
            slot.link.travel_type = TravelType::LADDER;
            slot.link.set_travel_time(10.0)?;
        }
        self.link_area(from, first_id)?;
        self.debug_state.count("ladder");
        let Some(second_id) = self.allocate() else {
            return Ok(false);
        };
        {
            let slot = self.slot_mut(second_id)?;
            slot.link.area = from;
            slot.link.face = number1;
            slot.link.edge = lowest_edge;
            slot.link.start = trace_end;
            let finish = ma(lowest_point, -5.0, plane1.normal);
            slot.link.end = with_z(finish, f64::from(finish.z) + 10.0);
            slot.link.travel_type = TravelType::JUMP;
            slot.link.set_travel_time(10.0)?;
        }
        self.link_area(destination, second_id)?;
        self.debug_state.count("jump");
        Ok(true)
    }

    /// Walk-off-ledge reachabilities over non-ground shared edges.
    pub(crate) fn walk_off_ledge(&mut self, area_number: i32) -> Result<(), BotsError> {
        let settings = self.movement_settings.clone();
        if !self.grounded(area_number)? || self.swim_area(area_number)? {
            return Ok(());
        }
        let area = *aas_at(&self.world.asset.areas, area_number)?;
        for i in 0..area.face_count {
            let face1 = *aas_at(
                &self.world.asset.faces,
                aas_at(&self.world.asset.face_indexes, area.first_face + i)?.unsigned_abs() as i32,
            )?;
            if (face1.flags & 4) == 0 {
                continue;
            }
            for k in 0..face1.edge_count {
                let edge_number = *aas_at(&self.world.asset.edge_indexes, face1.first_edge + k)?;
                for j in 0..area.face_count {
                    let number2 = *aas_at(&self.world.asset.face_indexes, area.first_face + j)?;
                    let face2 = *aas_at(&self.world.asset.faces, number2.unsigned_abs() as i32)?;
                    if (face2.flags & 4) != 0 {
                        continue;
                    }
                    for l in 0..face2.edge_count {
                        if edge_number.unsigned_abs()
                            != aas_at(&self.world.asset.edge_indexes, face2.first_edge + l)?.unsigned_abs()
                        {
                            continue;
                        }
                        let other = if face2.front_area == area_number {
                            face2.back_area
                        } else {
                            face2.front_area
                        };
                        let other_area = *aas_at(&self.world.asset.areas, other)?;
                        if self.grounded(other)? {
                            let mut gap = false;
                            let mut shared = false;
                            for n in 0..other_area.face_count {
                                let number3 = *aas_at(&self.world.asset.face_indexes, other_area.first_face + n)?;
                                if number3.unsigned_abs() == number2.unsigned_abs() {
                                    continue;
                                }
                                let face3 = *aas_at(&self.world.asset.faces, number3.unsigned_abs() as i32)?;
                                for m in 0..face3.edge_count {
                                    if aas_at(&self.world.asset.edge_indexes, face3.first_edge + m)?.unsigned_abs()
                                        != edge_number.unsigned_abs()
                                    {
                                        continue;
                                    }
                                    gap = (face3.flags & 1) == 0 || (face3.flags & 4) == 0;
                                    shared = true;
                                    break;
                                }
                                if shared {
                                    break;
                                }
                            }
                            if !gap {
                                break;
                            }
                        }
                        let edge = *aas_at(&self.world.asset.edges, edge_number.unsigned_abs() as i32)?;
                        let v1 = *aas_at(&self.world.asset.vertices, edge.vertices[usize::from(edge_number < 0)])?;
                        let v2 = *aas_at(&self.world.asset.vertices, edge.vertices[usize::from(edge_number >= 0)])?;
                        let plane1 = *aas_at(&self.world.asset.planes, face1.plane)?;
                        let direction = normalize3(cross3(plane1.normal, sub3(v2, v1)));
                        let midpoint = ma(scale3(add3(v1, v2), 0.5), 8.0, direction);
                        let test_end = with_z(midpoint, f64::from(midpoint.z) - 1000.0);
                        let trace = self.trace_client_bbox(midpoint, test_end, 4)?;
                        if trace.start_solid {
                            break;
                        }
                        let destination = self.world.point_area(trace.end)?;
                        if destination == area_number
                            || self.exists(area_number, destination)?
                            || (!self.grounded(destination)? && !self.swim_area(destination)?)
                            || (self.world.setting(destination)?.contents & 6) != 0
                        {
                            break;
                        }
                        let mut blocked = false;
                        for crossing in self.trace_areas(midpoint, test_end, 10)? {
                            if (self.world.setting(crossing.area)?.contents & 8) != 0 {
                                blocked = true;
                                break;
                            }
                        }
                        if blocked {
                            break;
                        }
                        let distance = (f64::from(midpoint.z) - f64::from(trace.end.z)) as f32;
                        if settings.max_fall_height != 0.0 && distance.abs() > settings.max_fall_height as f32 {
                            break;
                        }
                        let Some(id) = self.allocate() else {
                            break;
                        };
                        let swim_destination = self.swim_area(destination)?;
                        let destination_contents = self.world.setting(destination)?.contents;
                        {
                            let slot = self.slot_mut(id)?;
                            slot.link.area = destination;
                            slot.link.face = 0;
                            slot.link.edge = edge_number;
                            slot.link.start = midpoint;
                            slot.link.end = trace.end;
                            slot.link.travel_type = TravelType::WALKOFFLEDGE;
                            slot.link.set_travel_time(
                                settings.start_walk_off_ledge_time
                                    + f64::from(distance).abs() * 50.0 / settings.gravity,
                            )?;
                            if !swim_destination && (destination_contents & 128) == 0 {
                                if aas_fall_delta(&settings, distance) > settings.fall_delta5 as f32 {
                                    let time = (f64::from(slot.link.travel_time) + settings.fall_damage5_time) as f32;
                                    slot.link.set_travel_time(f64::from(time))?;
                                } else if aas_fall_delta(&settings, distance) > settings.fall_delta10 as f32 {
                                    let time = (f64::from(slot.link.travel_time) + settings.fall_damage10_time) as f32;
                                    slot.link.set_travel_time(f64::from(time))?;
                                }
                            }
                        }
                        self.link_area(area_number, id)?;
                        self.debug_state.count("walkoffledge");
                    }
                }
            }
        }
        Ok(())
    }
}
