//! Debug lines: shape tessellation and the timed line store.
//!
//! Donor provenance: `src/debug/shapes.ts` (`debugShapeLines`, geometry
//! adapted from q2repro `refresh/debug.c`, GPL-2.0-or-later) and
//! `src/debug/world.ts` (`WorldDebugLineStore`). Same tessellation counts,
//! arrow caps, expiry rules (server milliseconds own expiry, lifetime-zero
//! lines show for one presentation frame), and validation messages.
//! Trigonometry evaluates in `f64` and rounds to the workspace `f32`
//! vectors, matching the donor's `Math` inputs.

use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, Vec3, Vec4};
use thiserror::Error;

/// Failure of a debug operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DebugError {
    /// A capacity, lifetime, clock, or geometry value is invalid.
    #[error("{0}")]
    BadValue(String),
}

/// One debug line segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Line color.
    pub color: Vec4,
    /// Whether depth testing applies.
    pub depth_test: bool,
}

/// Tessellatable debug shape (donor `DebugShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DebugShape {
    /// Line segment.
    Line {
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
    },
    /// Axis cross.
    Point {
        /// Center.
        origin: Vec3,
        /// Cross size.
        size: f32,
    },
    /// Flat circle.
    Circle {
        /// Center.
        origin: Vec3,
        /// Radius.
        radius: f32,
    },
    /// Wireframe sphere.
    Sphere {
        /// Center.
        origin: Vec3,
        /// Radius.
        radius: f32,
    },
    /// Axis-aligned box edges.
    Bounds {
        /// Minimum corner.
        min: Vec3,
        /// Maximum corner.
        max: Vec3,
    },
    /// Vertical cylinder.
    Cylinder {
        /// Center.
        origin: Vec3,
        /// Half height.
        half_height: f32,
        /// Radius.
        radius: f32,
    },
    /// Arrow with a tinted cap.
    Arrow {
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Cap size.
        size: f32,
        /// Cap color.
        cap_color: Vec4,
    },
    /// Ray arrow from an origin along a direction.
    Ray {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Length.
        length: f32,
        /// Cap size.
        size: f32,
    },
}

fn arrow_lines(
    lines: &mut Vec<DebugLine>,
    start: Vec3,
    end: Vec3,
    size: f32,
    color: Vec4,
    cap_color: Vec4,
    depth_test: bool,
) {
    let delta = sub3(end, start);
    let length = length3(delta);
    let direction = normalize3(delta);
    let apex = if length > size {
        add3(start, scale3(direction, length - size))
    } else {
        end
    };
    if length > size {
        lines.push(DebugLine {
            start,
            end: apex,
            color,
            depth_test,
        });
    }
    let extent = if length > size { size } else { length };
    let tip = add3(apex, scale3(direction, extent));
    let rotated = Vec3 {
        x: direction.z,
        y: -direction.x,
        z: direction.y,
    };
    let right = normalize3(sub3(rotated, scale3(direction, dot3(rotated, direction))));
    lines.push(DebugLine {
        start: apex,
        end: tip,
        color: cap_color,
        depth_test,
    });
    lines.push(DebugLine {
        start: add3(apex, scale3(right, extent)),
        end: tip,
        color: cap_color,
        depth_test,
    });
    lines.push(DebugLine {
        start: add3(apex, scale3(right, -extent)),
        end: tip,
        color: cap_color,
        depth_test,
    });
}

/// Tessellate a shape into line segments (donor `debugShapeLines`).
#[must_use]
pub fn debug_shape_lines(shape: &DebugShape, color: Vec4, depth_test: bool) -> Vec<DebugLine> {
    let mut lines: Vec<DebugLine> = Vec::new();
    match *shape {
        DebugShape::Line { start, end } => {
            lines.push(DebugLine {
                start,
                end,
                color,
                depth_test,
            });
        }
        DebugShape::Point { origin, size } => {
            let half = size * 0.5;
            for axis in [
                Vec3 {
                    x: half,
                    y: 0.0,
                    z: 0.0,
                },
                Vec3 {
                    x: 0.0,
                    y: half,
                    z: 0.0,
                },
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: half,
                },
            ] {
                lines.push(DebugLine {
                    start: sub3(origin, axis),
                    end: add3(origin, axis),
                    color,
                    depth_test,
                });
            }
        }
        DebugShape::Bounds { min, max } => {
            let corner = |index: usize, z: f32| Vec3 {
                x: if index > 1 { min.x } else { max.x },
                y: if (index + 1) % 4 > 1 { min.y } else { max.y },
                z,
            };
            for index in 0..4 {
                lines.push(DebugLine {
                    start: corner(index, min.z),
                    end: corner(index, max.z),
                    color,
                    depth_test,
                });
                for z in [min.z, max.z] {
                    lines.push(DebugLine {
                        start: corner(index, z),
                        end: corner((index + 1) % 4, z),
                        color,
                        depth_test,
                    });
                }
            }
        }
        DebugShape::Circle { origin, radius } => {
            let count = (5.0 + f64::from(radius) / 8.0).min(16.0).trunc() as usize;
            let point = |index: usize| {
                let angle = f64::from(index as u32) * std::f64::consts::PI * 2.0 / f64::from(count as u32);
                Vec3 {
                    x: origin.x + (angle.cos() * f64::from(radius)) as f32,
                    y: origin.y + (angle.sin() * f64::from(radius)) as f32,
                    z: origin.z,
                }
            };
            for index in 0..count {
                lines.push(DebugLine {
                    start: point(index),
                    end: point((index + 1) % count),
                    color,
                    depth_test,
                });
            }
        }
        DebugShape::Cylinder {
            origin,
            half_height,
            radius,
        } => {
            let count = (5.0 + f64::from(radius) / 8.0).min(16.0).trunc() as usize;
            let point = |index: usize, z: f32| {
                let angle = f64::from(index as u32) * std::f64::consts::PI * 2.0 / f64::from(count as u32);
                Vec3 {
                    x: origin.x + (angle.cos() * f64::from(radius)) as f32,
                    y: origin.y + (angle.sin() * f64::from(radius)) as f32,
                    z,
                }
            };
            let bottom = origin.z - half_height;
            let top = origin.z + half_height;
            for index in 0..count {
                lines.push(DebugLine {
                    start: point(index, bottom),
                    end: point((index + 1) % count, bottom),
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: point(index, top),
                    end: point((index + 1) % count, top),
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: point(index, bottom),
                    end: point(index, top),
                    color,
                    depth_test,
                });
            }
        }
        DebugShape::Sphere { origin, radius } => {
            let stacks = (4.0 + f64::from(radius) / 32.0).min(10.0).trunc() as usize;
            let slices = (6.0 + f64::from(radius) / 32.0).min(16.0).trunc() as usize;
            let ring = |stack: usize, slice: usize| {
                let phi = std::f64::consts::PI * f64::from(stack as u32 + 1) / f64::from(stacks as u32);
                let theta = std::f64::consts::PI * 2.0 * f64::from(slice as u32) / f64::from(slices as u32);
                add3(
                    origin,
                    scale3(
                        Vec3 {
                            x: (phi.sin() * theta.cos()) as f32,
                            y: (phi.sin() * theta.sin()) as f32,
                            z: phi.cos() as f32,
                        },
                        radius,
                    ),
                )
            };
            let north = add3(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: radius,
                },
            );
            let south = sub3(
                origin,
                Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: radius,
                },
            );
            for index in 0..slices {
                let next = (index + 1) % slices;
                lines.push(DebugLine {
                    start: north,
                    end: ring(0, next),
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: ring(0, next),
                    end: ring(0, index),
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: ring(0, index),
                    end: north,
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: south,
                    end: ring(stacks - 2, index),
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: ring(stacks - 2, index),
                    end: ring(stacks - 2, next),
                    color,
                    depth_test,
                });
                lines.push(DebugLine {
                    start: ring(stacks - 2, next),
                    end: south,
                    color,
                    depth_test,
                });
            }
            for stack in 0..stacks - 2 {
                for index in 0..slices {
                    let next = (index + 1) % slices;
                    lines.push(DebugLine {
                        start: ring(stack, index),
                        end: ring(stack, next),
                        color,
                        depth_test,
                    });
                    lines.push(DebugLine {
                        start: ring(stack, next),
                        end: ring(stack + 1, next),
                        color,
                        depth_test,
                    });
                    lines.push(DebugLine {
                        start: ring(stack + 1, next),
                        end: ring(stack + 1, index),
                        color,
                        depth_test,
                    });
                    lines.push(DebugLine {
                        start: ring(stack + 1, index),
                        end: ring(stack, index),
                        color,
                        depth_test,
                    });
                }
            }
        }
        DebugShape::Arrow {
            start,
            end,
            size,
            cap_color,
        } => {
            arrow_lines(&mut lines, start, end, size, color, cap_color, depth_test);
        }
        DebugShape::Ray {
            origin,
            direction,
            length,
            size,
        } => {
            arrow_lines(
                &mut lines,
                origin,
                add3(origin, scale3(direction, length)),
                size,
                color,
                color,
                depth_test,
            );
        }
    }
    lines
}

/// Default timed-store capacity in lines.
pub const DEFAULT_DEBUG_CAPACITY: usize = 9216;

#[derive(Debug, Clone, Copy)]
struct TimedLine {
    line: DebugLine,
    expires: Option<u32>,
    first_frame: Option<u64>,
}

/// Timed debug-line store (donor `WorldDebugLineStore`).
///
/// Server milliseconds own expiry; all seats share one
/// presentation-frame snapshot.
#[derive(Debug)]
pub struct WorldDebugLineStore {
    entries: Vec<TimedLine>,
    /// Maximum retained lines.
    pub capacity: usize,
}

impl WorldDebugLineStore {
    /// Open a store with the donor default capacity.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            capacity: DEFAULT_DEBUG_CAPACITY,
        }
    }

    /// Open a store with an explicit capacity.
    pub fn with_capacity(capacity: usize) -> Result<Self, DebugError> {
        if capacity == 0 {
            return Err(DebugError::BadValue("Invalid debug line capacity".to_string()));
        }
        Ok(Self {
            entries: Vec::new(),
            capacity,
        })
    }

    /// Submit lines expiring `lifetime_ms` after `now_ms` (zero: one frame).
    pub fn submit(&mut self, lines: &[DebugLine], now_ms: f64, lifetime_ms: u32) -> Result<(), DebugError> {
        if !now_ms.is_finite() {
            return Err(DebugError::BadValue("Invalid debug line lifetime".to_string()));
        }
        for line in lines {
            let values = [
                line.start.x,
                line.start.y,
                line.start.z,
                line.end.x,
                line.end.y,
                line.end.z,
                line.color.x,
                line.color.y,
                line.color.z,
                line.color.w,
            ];
            if !values.iter().all(|value| value.is_finite()) {
                return Err(DebugError::BadValue("Invalid debug line geometry".to_string()));
            }
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let now = now_ms.trunc() as i64 as u32;
        let deadline = if lifetime_ms == 0 {
            0
        } else {
            now.wrapping_add(lifetime_ms)
        };
        self.entries
            .retain(|entry| entry.expires.is_none_or(|expires| expires > now));
        for line in lines {
            self.entries.push(TimedLine {
                line: *line,
                expires: if deadline == 0 { None } else { Some(deadline) },
                first_frame: None,
            });
        }
        if self.entries.len() > self.capacity {
            let overflow = self.entries.len() - self.capacity;
            self.entries.drain(..overflow);
        }
        Ok(())
    }

    /// Snapshot live lines for a presentation frame.
    pub fn snapshot(&mut self, now_ms: f64, frame: u64) -> Result<Vec<DebugLine>, DebugError> {
        if !now_ms.is_finite() {
            return Err(DebugError::BadValue("Invalid debug line clock".to_string()));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let now = now_ms.trunc() as i64 as u32;
        self.entries.retain(|entry| match entry.expires {
            None => entry.first_frame.is_none_or(|first| first == frame),
            Some(expires) => expires > now,
        });
        for entry in self.entries.iter_mut() {
            if entry.expires.is_none() && entry.first_frame.is_none() {
                entry.first_frame = Some(frame);
            }
        }
        Ok(self.entries.iter().map(|entry| entry.line).collect())
    }

    /// Drop all lines.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Default for WorldDebugLineStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::{vec3, vec4};

    fn line() -> DebugLine {
        DebugLine {
            start: vec3(0.0, 0.0, 0.0),
            end: vec3(1.0, 0.0, 0.0),
            color: vec4(1.0, 0.0, 0.0, 1.0),
            depth_test: true,
        }
    }

    #[test]
    fn tessellates_shapes() {
        let color = vec4(0.0, 1.0, 0.0, 1.0);
        assert_eq!(
            debug_shape_lines(
                &DebugShape::Line {
                    start: vec3(0.0, 0.0, 0.0),
                    end: vec3(1.0, 1.0, 1.0)
                },
                color,
                false
            )
            .len(),
            1
        );
        assert_eq!(
            debug_shape_lines(
                &DebugShape::Point {
                    origin: vec3(0.0, 0.0, 0.0),
                    size: 2.0
                },
                color,
                false
            )
            .len(),
            3
        );
        assert_eq!(
            debug_shape_lines(
                &DebugShape::Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(1.0, 1.0, 1.0)
                },
                color,
                false
            )
            .len(),
            12
        );
        let circle = debug_shape_lines(
            &DebugShape::Circle {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 8.0,
            },
            color,
            false,
        );
        assert_eq!(circle.len(), 6);
        let arrow = debug_shape_lines(
            &DebugShape::Arrow {
                start: vec3(0.0, 0.0, 0.0),
                end: vec3(10.0, 0.0, 0.0),
                size: 1.0,
                cap_color: vec4(1.0, 1.0, 1.0, 1.0),
            },
            color,
            true,
        );
        assert_eq!(arrow.len(), 4);
        let sphere = debug_shape_lines(
            &DebugShape::Sphere {
                origin: vec3(0.0, 0.0, 0.0),
                radius: 32.0,
            },
            color,
            false,
        );
        assert!(!sphere.is_empty());
    }

    #[test]
    fn expires_and_single_frames_lines() {
        let mut store = WorldDebugLineStore::with_capacity(4).unwrap();
        store.submit(&[line()], 1000.0, 500).unwrap();
        assert_eq!(store.snapshot(1200.0, 7).unwrap().len(), 1);
        assert!(store.snapshot(1600.0, 8).unwrap().is_empty());
        store.submit(&[line()], 2000.0, 0).unwrap();
        assert_eq!(store.snapshot(2000.0, 9).unwrap().len(), 1);
        assert!(store.snapshot(2000.0, 10).unwrap().is_empty());
        assert!(store.submit(&[line()], f64::NAN, 10).is_err());
        store.clear();
        assert!(WorldDebugLineStore::with_capacity(0).is_err());
    }
}
