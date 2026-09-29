//! Source AAS sampling and entity queries over shared assets, ported
//! from `src/bots/navigation/aas-reachability-spatial.ts`.
//! Copyright (C) 1999-2005 Id Software, Inc.

use std::collections::HashMap;

use qa_core::math::{add3, angle_vectors, cross3, dot3, length3, normalize3, scale3, sub3, vec3, Bounds, Vec3};
use qa_core::numeric::{native_atof, native_atoi};
use qa_world::collision::{convert_contents, convert_surface_flags};
use qa_world::spatial::CollisionFamily;

use crate::aas::{aas_bbox_areas, aas_trace_areas, AasAreaCrossing};
use crate::aas_reachability::AasReachabilityContext;
use crate::aas_reachability_geometry::{aas_at, ma, v3, with_z, ZERO};
use crate::aas_reachability_types::AasClientMove;
use crate::behavior::BotMovementPrediction;
use crate::entities::parse_entities;
use crate::error::BotsError;
use crate::scene::{PointContentsQuery, PointContentsResult, QueryTarget, TraceDetail, TraceQuery, TraceShape};

fn decimal_integer(text: &str) -> Result<i32, BotsError> {
    let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
    let mut cursor = 0;
    while cursor < bytes.len() && matches!(bytes[cursor], b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ') {
        cursor += 1;
    }
    let negative = bytes.get(cursor) == Some(&b'-');
    if matches!(bytes.get(cursor), Some(b'+') | Some(b'-')) {
        cursor += 1;
    }
    let start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    if cursor > start {
        let magnitude: Vec<u8> = bytes[start..cursor].to_vec();
        // Compare against the int32 bounds without big integers: strip
        // leading zeros, then compare length and lexicographic order.
        let mut significant = magnitude.as_slice();
        while significant.len() > 1 && significant[0] == b'0' {
            significant = &significant[1..];
        }
        let limit: &[u8] = if negative { b"2147483648" } else { b"2147483647" };
        if significant.len() > limit.len() || (significant.len() == limit.len() && significant > limit) {
            return Err(BotsError::IntRange(
                "AAS integer epair conversion exceeds the source-defined int32 range".to_string(),
            ));
        }
    }
    Ok(native_atoi(text)?)
}

fn scan_float(text: &[u8], start: usize) -> Result<Option<(f64, usize)>, BotsError> {
    let mut cursor = start;
    while cursor < text.len() && matches!(text[cursor], b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ') {
        cursor += 1;
    }
    let beginning = cursor;
    if matches!(text.get(cursor), Some(b'+') | Some(b'-')) {
        cursor += 1;
    }
    let lower = |index: usize| text.get(index).map(|b| b.to_ascii_lowercase());
    let starts_with = |word: &[u8]| word.iter().enumerate().all(|(i, b)| lower(cursor + i) == Some(*b));
    if starts_with(b"inf") {
        if lower(cursor + 3) == Some(b'i') {
            if !starts_with(b"infinity") {
                return Ok(None);
            }
            cursor += 8;
        } else {
            cursor += 3;
        }
    } else if starts_with(b"nan") {
        cursor += 3;
        if text.get(cursor) == Some(&b'(') {
            cursor += 1;
            while cursor < text.len() && (text[cursor].is_ascii_alphanumeric() || text[cursor] == b'_') {
                cursor += 1;
            }
            if text.get(cursor) != Some(&b')') {
                return Ok(None);
            }
            cursor += 1;
        }
    } else {
        let hexadecimal = starts_with(b"0x");
        if hexadecimal {
            cursor += 2;
        }
        let is_digit = |byte: Option<&u8>| {
            byte.is_some_and(|b| {
                if hexadecimal {
                    b.is_ascii_hexdigit()
                } else {
                    b.is_ascii_digit()
                }
            })
        };
        let mut digits = 0;
        while is_digit(text.get(cursor)) {
            cursor += 1;
            digits += 1;
        }
        if text.get(cursor) == Some(&b'.') {
            cursor += 1;
            while is_digit(text.get(cursor)) {
                cursor += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return Ok(None);
        }
        let exponent = lower(cursor);
        if exponent == Some(if hexadecimal { b'p' } else { b'e' }) {
            cursor += 1;
            if matches!(text.get(cursor), Some(b'+') | Some(b'-')) {
                cursor += 1;
            }
            let exponent_start = cursor;
            while text.get(cursor).is_some_and(|b| b.is_ascii_digit()) {
                cursor += 1;
            }
            if cursor == exponent_start {
                return Ok(None);
            }
        }
    }
    let slice: String = text[beginning..cursor].iter().map(|b| char::from(*b)).collect();
    Ok(Some((native_atof(&slice)?, cursor)))
}

/// BSP entity records with source epair accessors.
#[derive(Debug, Clone)]
pub struct AasReachabilityEntities {
    records: Vec<HashMap<String, String>>,
}

impl AasReachabilityEntities {
    /// Parse entity text.
    pub fn new(text: &str) -> Result<Self, BotsError> {
        let records = parse_entities(text, "<entities>")?;
        if records.len() >= 2048 {
            return Err(BotsError::EntityLimit);
        }
        Ok(Self { records })
    }

    /// Entity number after a previous one, or zero at the end.
    #[must_use]
    pub fn next_entity(&self, previous: i32) -> i32 {
        let next = previous + 1;
        if next > 0 && next as usize <= self.records.len() {
            next
        } else {
            0
        }
    }

    /// Copy an epair value into a NUL-terminated output buffer.
    pub fn value(&self, entity: i32, key: &str, output: &mut [u8]) -> Result<bool, BotsError> {
        if output.is_empty() {
            return Err(BotsError::EmptyEpair);
        }
        output[0] = 0;
        let Some(value) = entity
            .checked_sub(1)
            .and_then(|index| self.records.get(index as usize))
            .and_then(|record| record.get(key))
        else {
            return Ok(false);
        };
        output.fill(0);
        let capacity = output.len() - 1;
        for (slot, ch) in output.iter_mut().take(capacity).zip(value.chars()) {
            *slot = ch as u8;
        }
        Ok(true)
    }

    fn text(&self, entity: i32, key: &str) -> Option<String> {
        entity
            .checked_sub(1)
            .and_then(|index| self.records.get(index as usize))
            .and_then(|record| record.get(key))
            .map(|value| value.split('\0').next().unwrap_or("").chars().take(127).collect())
    }

    /// Integer epair with a found flag.
    pub fn int(&self, entity: i32, key: &str) -> Result<(bool, i32), BotsError> {
        match self.text(entity, key) {
            None => Ok((false, 0)),
            Some(text) => Ok((true, decimal_integer(&text)?)),
        }
    }

    /// Float epair with a found flag.
    pub fn float(&self, entity: i32, key: &str) -> Result<(bool, f32), BotsError> {
        match self.text(entity, key) {
            None => Ok((false, 0.0)),
            Some(text) => Ok((true, native_atof(&text)? as f32)),
        }
    }

    /// Vector epair with a found flag.
    pub fn vector(&self, entity: i32, key: &str) -> Result<(bool, Vec3), BotsError> {
        let Some(text) = self.text(entity, key) else {
            return Ok((false, Vec3 { x: 0.0, y: 0.0, z: 0.0 }));
        };
        let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
        let mut value = [0.0f32; 3];
        let mut cursor = 0;
        for slot in &mut value {
            let Some((scanned, end)) = scan_float(&bytes, cursor)? else {
                break;
            };
            *slot = scanned as f32;
            cursor = end;
        }
        Ok((
            true,
            Vec3 {
                x: value[0],
                y: value[1],
                z: value[2],
            },
        ))
    }
}

/// AAS bounding-box trace outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasTrace {
    /// Start position was solid.
    pub start_solid: bool,
    /// Fraction completed.
    pub fraction: f64,
    /// End position.
    pub end: Vec3,
    /// Hit entity number.
    pub entity_num: i32,
    /// Last traversed area.
    pub last_area: i32,
    /// Blocking area.
    pub area: i32,
    /// Blocking plane.
    pub plane: i32,
}

struct TracePiece {
    start: Vec3,
    end: Vec3,
    node: i32,
    plane: i32,
}

/// Host trace outcome with Q3-mapped surface flags.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostTrace {
    /// End position.
    pub end: Vec3,
    /// Start position was solid.
    pub start_solid: bool,
    /// Fraction completed.
    pub fraction: f64,
    /// Surface flags mapped to Q3.
    pub surface_flags: i32,
}

/// Jump-pad source record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpPadInfo {
    /// Pad start point.
    pub start: Vec3,
    /// Pad bounds.
    pub bounds: Bounds,
    /// Launch velocity.
    pub velocity: Vec3,
}

/// Horizontal jump-velocity query outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpVelocity {
    /// A velocity within the maximum exists.
    pub success: bool,
    /// Horizontal velocity.
    pub velocity: f32,
}

fn same(a: Vec3, b: Vec3) -> bool {
    a.x == b.x && a.y == b.y && a.z == b.z
}

impl<'a> AasReachabilityContext<'a> {
    /// Sample shared collision contents mapped to Q3.
    pub(crate) fn host_point_contents(&self, point: Vec3) -> i32 {
        let value = self.options.scene.point_contents(&PointContentsQuery {
            point,
            target: QueryTarget::World,
            policy: crate::scene::TracePolicy::Q3 {
                contents_mask: -1,
                curves: true,
                player_curve_clip: true,
            },
            numeric: self.options.profile.movement.numeric,
            pass_actor: None,
        });
        match value {
            PointContentsResult::Q1 { contents } => {
                convert_contents(contents, CollisionFamily::Q1, CollisionFamily::Q3)
            }
            PointContentsResult::Q2 { merged, .. } => {
                convert_contents(merged, CollisionFamily::Q2, CollisionFamily::Q3)
            }
            PointContentsResult::Q3 { contents } => {
                convert_contents(contents, CollisionFamily::Q3, CollisionFamily::Q3)
            }
        }
    }

    /// Trace shared collision with Q3-mapped surface flags.
    pub(crate) fn host_trace(&self, start: Vec3, end: Vec3, bounds: Option<Bounds>, mask: i32) -> HostTrace {
        let trace = self.options.scene.trace(&TraceQuery {
            start,
            end,
            shape: match bounds {
                None => TraceShape::Point,
                Some(bounds) => TraceShape::Box { bounds },
            },
            target: QueryTarget::World,
            policy: crate::scene::TracePolicy::Q3 {
                contents_mask: mask,
                curves: true,
                player_curve_clip: true,
            },
            numeric: self.options.profile.movement.numeric,
            pass_actor: None,
        });
        let (flags, family) = match &trace.detail {
            TraceDetail::Q1 { surface_flags, .. } => (surface_flags.unwrap_or(0), CollisionFamily::Q1),
            TraceDetail::Q2 { surface, .. } => {
                (surface.as_ref().map_or(0, |surface| surface.flags), CollisionFamily::Q2)
            }
            TraceDetail::Q3 { surface_flags, .. } => (*surface_flags, CollisionFamily::Q3),
        };
        HostTrace {
            end: trace.end,
            start_solid: trace.start_solid,
            fraction: trace.fraction,
            surface_flags: convert_surface_flags(flags, family, CollisionFamily::Q3),
        }
    }

    /// Model bounds, spherized for rotated models, with a zero origin.
    pub(crate) fn host_model_bounds(&self, model: i32, angles: Vec3) -> Result<(Bounds, Vec3), BotsError> {
        let bounds = match &self.options.geometry {
            crate::scene::DecodedWorld::Q1(world) => {
                crate::error::indexed(&world.models, i64::from(model), "AAS reachability index")?.bounds
            }
            crate::scene::DecodedWorld::Q2(world) => {
                crate::error::indexed(&world.models, i64::from(model), "AAS reachability index")?.bounds
            }
            crate::scene::DecodedWorld::Q3(world) => {
                crate::error::indexed(&world.models, i64::from(model), "AAS reachability index")?.bounds
            }
        };
        if angles.x != 0.0 || angles.y != 0.0 || angles.z != 0.0 {
            let radius = length3(vec3(
                bounds.min.x.abs().max(bounds.max.x.abs()),
                bounds.min.y.abs().max(bounds.max.y.abs()),
                bounds.min.z.abs().max(bounds.max.z.abs()),
            ));
            return Ok((
                Bounds {
                    min: vec3(-radius, -radius, -radius),
                    max: vec3(radius, radius, radius),
                },
                ZERO,
            ));
        }
        Ok((bounds, ZERO))
    }

    /// Predict client movement through the selected provider, simulating
    /// the configured prediction client.
    pub(crate) fn movement_predict_client(&self, request: BotMovementPrediction) -> Result<AasClientMove, BotsError> {
        let result = (self.options.predict_client_movement)(BotMovementPrediction {
            entity_num: self.options.prediction_client,
            ..request
        });
        let Some(end_area) = result.end_area else {
            return Err(BotsError::PredictionArea);
        };
        let frames = if result.stop_event == 0 {
            result.frames
        } else {
            (result.frames - 1).max(0)
        };
        Ok(AasClientMove {
            end: result.end,
            velocity: result.velocity,
            frames,
            time: (f64::from(frames) * f64::from(request.frame_time)) as f32,
            stop_event: result.stop_event,
            end_area,
        })
    }

    /// Horizontal velocity reaching an endpoint with a vertical velocity.
    pub(crate) fn movement_horizontal_velocity_for_jump(
        &self,
        z_velocity: f64,
        start: Vec3,
        end: Vec3,
    ) -> JumpVelocity {
        let gravity = self.movement_settings.gravity;
        let maximum = self.movement_settings.max_velocity;
        let ascent = (z_velocity / gravity) as f32;
        let maximum_jump = (0.5 * gravity * f64::from(ascent) * f64::from(ascent)) as f32;
        let height = ((f64::from(start.z) + f64::from(maximum_jump)) as f32 as f64 - f64::from(end.z)) as f32;
        if height < 0.0 {
            return JumpVelocity {
                success: false,
                velocity: maximum as f32,
            };
        }
        let time = (f64::from(height) / (0.5 * gravity)).sqrt() as f32;
        let denominator = (f64::from(time) + f64::from(ascent)) as f32;
        if denominator == 0.0 {
            return JumpVelocity {
                success: false,
                velocity: maximum as f32,
            };
        }
        let direction = sub3(end, start);
        let planar = (f64::from((f64::from(direction.x) * f64::from(direction.x)) as f32)
            + f64::from((f64::from(direction.y) * f64::from(direction.y)) as f32)) as f32;
        let speed = (f64::from(planar).sqrt() / f64::from(denominator)) as f32;
        if f64::from(speed) > maximum {
            JumpVelocity {
                success: false,
                velocity: maximum as f32,
            }
        } else {
            JumpVelocity {
                success: true,
                velocity: speed,
            }
        }
    }

    /// Vertical rocket-jump velocity from a wall trace above the origin.
    pub(crate) fn movement_rocket_jump_z_velocity(&self, origin: Vec3) -> f32 {
        let vectors = angle_vectors(vec3(90.0, 0.0, 0.0));
        let (forward, right) = (vectors.forward, vectors.right);
        let start = vec3(
            origin.x + (forward.x * 8.0 + right.x * 8.0),
            origin.y + (forward.y * 8.0 + right.y * 8.0),
            (origin.z + 8.0) + ((forward.z * 8.0 + right.z * 8.0) - 8.0),
        );
        let trace = self.host_trace(start, ma(start, 500.0, forward), None, 1);
        let distance = length3(sub3(
            trace.end,
            Vec3 {
                x: origin.x,
                y: origin.y,
                z: origin.z + 4.0,
            },
        ));
        let points = (((120.0 - 0.5 * f64::from(distance)) as f32).max(0.0) as f64 * 0.5) as f32;
        let direction = normalize3(sub3(origin, trace.end));
        (f64::from((f64::from(direction.z) * (1600.0 * f64::from(points) / 200.0)) as f32)
            + self.movement_settings.jump_velocity) as f32
    }

    /// Areas crossed by a segment.
    pub(crate) fn trace_areas(
        &self,
        start: Vec3,
        end: Vec3,
        maximum: usize,
    ) -> Result<Vec<AasAreaCrossing>, BotsError> {
        aas_trace_areas(&self.world.asset, start, end, maximum)
    }

    /// Trace presence bounds through the AAS BSP tree.
    pub(crate) fn trace_client_bbox(&self, start: Vec3, end: Vec3, presence: i32) -> Result<AasTrace, BotsError> {
        for point in [start, end] {
            if !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite() {
                return Err(BotsError::NonFiniteTrace);
            }
        }
        let mut stack = vec![TracePiece {
            start: Vec3 {
                x: start.x,
                y: start.y,
                z: start.z,
            },
            end: Vec3 {
                x: end.x,
                y: end.y,
                z: end.z,
            },
            node: 1,
            plane: 0,
        }];
        let mut last_area = 0;
        while let Some(piece) = stack.pop() {
            if piece.node <= 0 {
                if piece.node == 0
                    || (aas_at(&self.world.settings, piece.node.wrapping_neg())?.presence & presence) == 0
                {
                    let start_solid = same(piece.start, start);
                    let direction = if start_solid {
                        ZERO
                    } else {
                        normalize3(sub3(end, start))
                    };
                    let fraction = if start_solid {
                        0.0
                    } else {
                        (f64::from(length3(sub3(piece.start, start))) / f64::from(length3(sub3(end, start)))) as f32
                    };
                    let hit_end = if start_solid {
                        piece.start
                    } else {
                        ma(piece.start, -0.125, direction)
                    };
                    let plane = if dot3(direction, aas_at(&self.world.asset.planes, piece.plane)?.normal) > 0.0 {
                        piece.plane ^ 1
                    } else {
                        piece.plane
                    };
                    return Ok(AasTrace {
                        start_solid,
                        fraction: f64::from(fraction),
                        end: hit_end,
                        entity_num: 0,
                        last_area,
                        area: if piece.node == 0 { 0 } else { piece.node.wrapping_neg() },
                        plane,
                    });
                }
                last_area = piece.node.wrapping_neg();
                continue;
            }
            let node = *aas_at(&self.world.asset.nodes, piece.node)?;
            let plane = *aas_at(&self.world.asset.planes, node.plane)?;
            let mut front = (f64::from(dot3(piece.start, plane.normal)) - f64::from(plane.distance)) as f32;
            let back = (f64::from(dot3(piece.end, plane.normal)) - f64::from(plane.distance)) as f32;
            if front >= 0.0 && back >= 0.0 {
                stack.push(TracePiece {
                    node: node.children[0],
                    ..piece
                });
                if stack.len() >= 127 {
                    self.print(3, "AAS_TraceBoundingBox: stack overflow\n");
                    return Ok(AasTrace {
                        start_solid: false,
                        fraction: 0.0,
                        end: ZERO,
                        entity_num: 0,
                        last_area,
                        area: 0,
                        plane: 0,
                    });
                }
            } else if front < 0.0 && back < 0.0 {
                stack.push(TracePiece {
                    node: node.children[1],
                    ..piece
                });
                if stack.len() >= 127 {
                    self.print(3, "AAS_TraceBoundingBox: stack overflow\n");
                    return Ok(AasTrace {
                        start_solid: false,
                        fraction: 0.0,
                        end: ZERO,
                        entity_num: 0,
                        last_area,
                        area: 0,
                        plane: 0,
                    });
                }
            } else {
                if front == back {
                    front -= 0.001;
                }
                let numerator = if front < 0.0 { front + 0.125 } else { front - 0.125 };
                let denominator = (f64::from(front) - f64::from(back)) as f32;
                let mut fraction = (f64::from(numerator) / f64::from(denominator)) as f32;
                if fraction < 0.0 {
                    fraction = 0.001;
                } else if fraction > 1.0 {
                    fraction = 0.999;
                }
                let middle = ma(piece.start, fraction, sub3(piece.end, piece.start));
                let side = i32::from(front < 0.0);
                stack.push(TracePiece {
                    start: middle,
                    end: piece.end,
                    node: node.children[usize::from(side == 0)],
                    plane: node.plane,
                });
                if stack.len() >= 127 {
                    self.print(3, "AAS_TraceBoundingBox: stack overflow\n");
                    return Ok(AasTrace {
                        start_solid: false,
                        fraction: 0.0,
                        end: ZERO,
                        entity_num: 0,
                        last_area,
                        area: 0,
                        plane: 0,
                    });
                }
                stack.push(TracePiece {
                    start: piece.start,
                    end: middle,
                    node: node.children[side as usize],
                    plane: node.plane,
                });
                if stack.len() >= 127 {
                    self.print(3, "AAS_TraceBoundingBox: stack overflow\n");
                    return Ok(AasTrace {
                        start_solid: false,
                        fraction: 0.0,
                        end: ZERO,
                        entity_num: 0,
                        last_area,
                        area: 0,
                        plane: 0,
                    });
                }
            }
        }
        Ok(AasTrace {
            start_solid: false,
            fraction: 1.0,
            end,
            entity_num: 0,
            last_area,
            area: 0,
            plane: 0,
        })
    }

    /// Areas touched by expanded client bounds, in enumeration order.
    pub(crate) fn link_client_bounds(&self, bounds: Bounds, presence: i32) -> Result<Vec<i32>, BotsError> {
        let client = match self.world.asset.bboxes.iter().find(|bbox| bbox.presence == presence) {
            Some(bbox) => Some(bbox.bounds),
            None if presence == 4 => self.options.profile.crouched_shape.map(|shape| shape.bounds()),
            None => Some(self.options.profile.shape.bounds()),
        };
        let Some(client) = client else {
            return Err(BotsError::MissingPresenceBounds { presence });
        };
        aas_bbox_areas(
            &self.world.asset,
            Bounds {
                min: sub3(bounds.min, client.max),
                max: sub3(bounds.max, client.min),
            },
            self.world.asset.areas.len(),
        )
    }

    /// Best reachable area near an origin, with its adjusted origin.
    pub(crate) fn best_reachable_area(&self, origin: Vec3, bounds: Bounds) -> Result<(i32, Vec3), BotsError> {
        let mut start = origin;
        let mut area = self.world.point_area(start)?;
        for i in 0..5 {
            if area != 0 {
                break;
            }
            for j in 0..5 {
                if area != 0 {
                    break;
                }
                for k in -1..=1 {
                    if area != 0 {
                        break;
                    }
                    for l in -1..=1 {
                        if area != 0 {
                            break;
                        }
                        start = v3(
                            f64::from(origin.x) + f64::from(j * 4 * k),
                            f64::from(origin.y) + f64::from(j * 4 * l),
                            f64::from(origin.z) + f64::from(i * 4),
                        );
                        area = self.world.point_area(start)?;
                    }
                }
            }
        }
        if area != 0 {
            let end = Vec3 {
                x: start.x,
                y: start.y,
                z: start.z - 50.0,
            };
            start = with_z(start, f64::from(start.z) + 0.25);
            let trace = self.trace_client_bbox(start, end, 4)?;
            if trace.start_solid {
                return Ok((area, start));
            }
            area = self.world.point_area(trace.end)?;
            if area != 0 {
                return Ok((area, trace.end));
            }
        }
        let head = self.link_client_bounds(
            Bounds {
                min: add3(origin, bounds.min),
                max: add3(origin, bounds.max),
            },
            4,
        )?;
        for link in &head {
            if (aas_at(&self.world.settings, *link)?.flags & 5) != 0 {
                return Ok((*link, origin));
            }
        }
        for link in &head {
            if *link != 0 {
                return Ok((*link, origin));
            }
        }
        Ok((0, origin))
    }

    /// Drop bounds to the floor, or `None` when starting solid.
    pub(crate) fn drop_to_floor(&self, origin: Vec3, bounds: Bounds) -> Option<Vec3> {
        let trace = self.host_trace(
            origin,
            Vec3 {
                x: origin.x,
                y: origin.y,
                z: origin.z - 100.0,
            },
            Some(bounds),
            1,
        );
        if trace.start_solid {
            None
        } else {
            Some(trace.end)
        }
    }

    /// Whether a point sits inside a face's edge winding.
    pub(crate) fn point_inside_face(&self, number: i32, point: Vec3, epsilon: f32) -> Result<bool, BotsError> {
        let face = *aas_at(&self.world.asset.faces, number)?;
        let normal = aas_at(&self.world.asset.planes, face.plane)?.normal;
        for i in 0..face.edge_count {
            let number = *aas_at(&self.world.asset.edge_indexes, face.first_edge + i)?;
            let edge = *aas_at(&self.world.asset.edges, number.unsigned_abs() as i32)?;
            let origin = *aas_at(&self.world.asset.vertices, edge.vertices[usize::from(number < 0)])?;
            let direction = sub3(
                *aas_at(&self.world.asset.vertices, edge.vertices[usize::from(number >= 0)])?,
                origin,
            );
            if dot3(sub3(point, origin), cross3(direction, normal)) < -epsilon {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Jump-pad launch record behind a trigger_push entity.
    pub(crate) fn get_jump_pad_info(&self, entity: i32) -> Result<Option<JumpPadInfo>, BotsError> {
        let mut model_buffer = [0u8; 128];
        let mut target_buffer = [0u8; 128];
        let mut name_buffer = [0u8; 128];
        let text = |bytes: &[u8]| -> String { bytes.iter().take_while(|b| **b != 0).map(|b| char::from(*b)).collect() };
        self.bsp_entities.value(entity, "model", &mut model_buffer)?;
        let model_name = text(&model_buffer);
        let model_number = if model_name.is_empty() {
            0
        } else {
            native_atoi(model_name.get(1..).unwrap_or(""))?
        };
        let (model_bounds, model_origin) = self.host_model_bounds(model_number, ZERO)?;
        let bounds = Bounds {
            min: add3(model_origin, model_bounds.min),
            max: add3(model_origin, model_bounds.max),
        };
        let center = scale3(add3(bounds.min, bounds.max), 0.5);
        let trace = self.trace_client_bbox(
            Vec3 {
                x: center.x,
                y: center.y,
                z: center.z + 64.0,
            },
            center,
            4,
        )?;
        let bottom = if trace.start_solid { center } else { trace.end };
        let start = with_z(bottom, f64::from(bottom.z) + 0.125);
        self.bsp_entities.value(entity, "target", &mut target_buffer)?;
        let target_name = text(&target_buffer);
        let mut target = self.bsp_entities.next_entity(0);
        while target != 0 {
            if self.bsp_entities.value(target, "targetname", &mut name_buffer)? && text(&name_buffer) == target_name {
                break;
            }
            target = self.bsp_entities.next_entity(target);
        }
        if target == 0 {
            self.print(1, &format!("trigger_push without target entity {target_name}\n"));
            return Ok(None);
        }
        let destination = self.bsp_entities.vector(target, "origin")?.1;
        let height = (f64::from(destination.z) - f64::from(center.z)) as f32;
        let time = (f64::from(height) / (0.5 * f64::from(self.movement_settings.gravity as f32))).sqrt() as f32;
        if time == 0.0 {
            return Ok(None);
        }
        if !time.is_finite() {
            return Err(BotsError::NonFinite(
                "Jump-pad target produces a non-finite source flight time".to_string(),
            ));
        }
        let delta = sub3(destination, center);
        let distance = length3(delta);
        let forward = ((f64::from(distance) / f64::from(time)) as f32 as f64 * f64::from(1.1f32)) as f32;
        let push = scale3(normalize3(delta), forward);
        Ok(Some(JumpPadInfo {
            start,
            bounds,
            velocity: v3(
                f64::from(push.x),
                f64::from(push.y),
                f64::from(time) * f64::from(self.movement_settings.gravity as f32),
            ),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_integers_match_source_range() {
        assert_eq!(decimal_integer("42").unwrap(), 42);
        assert_eq!(decimal_integer("  -7 junk").unwrap(), -7);
        assert_eq!(decimal_integer("+2147483647").unwrap(), 2147483647);
        assert_eq!(decimal_integer("-2147483648").unwrap(), -2147483648);
        assert!(decimal_integer("2147483648").is_err());
        assert!(decimal_integer("-2147483649").is_err());
        assert!(decimal_integer("0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000099").is_ok());
    }

    #[test]
    fn scan_floats_match_source_grammar() {
        let scan = |text: &str| {
            let bytes: Vec<u8> = text.chars().map(|c| c as u8).collect();
            scan_float(&bytes, 0).unwrap().map(|(value, _)| value)
        };
        assert_eq!(scan("1.5"), Some(1.5));
        assert_eq!(scan("  -2e3 rest"), Some(-2000.0));
        assert_eq!(scan("inf"), Some(f64::INFINITY));
        assert_eq!(scan("0x10"), Some(16.0));
        assert!(scan("nan").unwrap().is_nan());
        assert_eq!(scan("abc"), None);
        assert_eq!(scan("1.2.3"), Some(1.2));
    }

    #[test]
    fn entity_epairs_roundtrip() {
        let entities = AasReachabilityEntities::new(
            "{ \"classname\" \"a\" \"n\" \"5\" \"f\" \"1.5\" \"v\" \"1 2 3\" }\n{ \"classname\" \"b\" }",
        )
        .unwrap();
        assert_eq!(entities.next_entity(0), 1);
        assert_eq!(entities.next_entity(1), 2);
        assert_eq!(entities.next_entity(2), 0);
        assert_eq!(entities.int(1, "n").unwrap(), (true, 5));
        assert_eq!(entities.int(2, "n").unwrap(), (false, 0));
        assert_eq!(entities.float(1, "f").unwrap(), (true, 1.5));
        let (found, vector) = entities.vector(1, "v").unwrap();
        assert!(found);
        assert_eq!(vector, Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        let mut output = [9u8; 4];
        assert!(entities.value(1, "classname", &mut output).unwrap());
        assert_eq!(&output, b"a\0\0\0");
        assert!(!entities.value(2, "n", &mut output).unwrap());
        assert_eq!(output[0], 0);
        assert!(entities.value(1, "n", &mut []).is_err());
    }
}
