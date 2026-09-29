//! Q2 rerelease debug-shape drawing imports.
//!
//! Donor: `src/compat/q2/rerelease/debug-shapes.ts` — bridges the `game.h`
//! `Draw_*` imports into tessellated debug lines with source lifetimes.

use qa_core::math::{Vec3, Vec4};
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use qa_guest::GuestError;
use thiserror::Error;

/// Debug-shape import failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DebugShapeError {
    /// Shape requires a finite source float.
    #[error("Q2 debug shape requires a finite source float")]
    NonFinite,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// One tessellated debug line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DebugLine {
    /// Start point.
    pub start: Vec3,
    /// End point.
    pub end: Vec3,
    /// Color.
    pub color: Vec4,
    /// Depth test flag.
    pub depth_test: bool,
}

/// Debug shape from a `Draw_*` import.
#[derive(Debug, Clone, PartialEq)]
pub enum DebugShape {
    /// Line segment.
    Line {
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
    },
    /// Point cross.
    Point {
        /// Origin.
        origin: Vec3,
        /// Cross size.
        size: f32,
    },
    /// Flat circle.
    Circle {
        /// Origin.
        origin: Vec3,
        /// Radius.
        radius: f32,
    },
    /// Sphere.
    Sphere {
        /// Origin.
        origin: Vec3,
        /// Radius.
        radius: f32,
    },
    /// Bounding box.
    Bounds {
        /// Minimum corner.
        min: Vec3,
        /// Maximum corner.
        max: Vec3,
    },
    /// Cylinder.
    Cylinder {
        /// Origin.
        origin: Vec3,
        /// Half height.
        half_height: f32,
        /// Radius.
        radius: f32,
    },
    /// Arrow with a cap color.
    Arrow {
        /// Start point.
        start: Vec3,
        /// End point.
        end: Vec3,
        /// Head size.
        size: f32,
        /// Cap color.
        cap_color: Vec4,
    },
    /// Ray from an origin along a direction.
    Ray {
        /// Origin.
        origin: Vec3,
        /// Direction.
        direction: Vec3,
        /// Length.
        length: f32,
        /// Head size.
        size: f32,
    },
}

/// One debug-shape event: tessellated lines plus lifetime.
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseDebugShapesEvent {
    /// Tessellated lines.
    pub lines: Vec<DebugLine>,
    /// Lifetime in milliseconds.
    pub lifetime_milliseconds: u32,
}

fn add3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

fn sub3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

fn scale3(value: Vec3, scale: f32) -> Vec3 {
    Vec3 {
        x: value.x * scale,
        y: value.y * scale,
        z: value.z * scale,
    }
}

fn dot3(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn length3(value: Vec3) -> f32 {
    dot3(value, value).sqrt()
}

fn normalize3(value: Vec3) -> Vec3 {
    let length = length3(value);
    if length == 0.0 {
        value
    } else {
        scale3(value, 1.0 / length)
    }
}

/// Tessellate a shape into lines (`debug/shapes.ts`, adapted from
/// `q2repro` `refresh/debug.c`).
#[must_use]
pub fn debug_shape_lines(shape: &DebugShape, color: Vec4, depth_test: bool) -> Vec<DebugLine> {
    let mut lines = Vec::new();
    let mut line = |start: Vec3, end: Vec3, tint: Vec4| {
        lines.push(DebugLine {
            start,
            end,
            color: tint,
            depth_test,
        });
    };
    let mut arrow = |start: Vec3, end: Vec3, size: f32, cap_color: Vec4, color: Vec4| {
        let delta = sub3(end, start);
        let length = length3(delta);
        let dir = normalize3(delta);
        let apex = if length > size {
            add3(start, scale3(dir, length - size))
        } else {
            end
        };
        if length > size {
            line(start, apex, color);
        }
        let extent = if length > size { size } else { length };
        let tip = add3(apex, scale3(dir, extent));
        let rotated = Vec3 {
            x: dir.z,
            y: -dir.x,
            z: dir.y,
        };
        let right = normalize3(sub3(rotated, scale3(dir, dot3(rotated, dir))));
        line(apex, tip, cap_color);
        line(add3(apex, scale3(right, extent)), tip, cap_color);
        line(add3(apex, scale3(right, -extent)), tip, cap_color);
    };
    match shape {
        DebugShape::Line { start, end } => line(*start, *end, color),
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
                line(sub3(*origin, axis), add3(*origin, axis), color);
            }
        }
        DebugShape::Bounds { min, max } => {
            let corner = |i: usize, z: f32| Vec3 {
                x: if i > 1 { min.x } else { max.x },
                y: if (i + 1) % 4 > 1 { min.y } else { max.y },
                z,
            };
            for i in 0..4 {
                line(corner(i, min.z), corner(i, max.z), color);
                for z in [min.z, max.z] {
                    line(corner(i, z), corner((i + 1) % 4, z), color);
                }
            }
        }
        DebugShape::Circle { origin, radius } => {
            circle_lines(*origin, *radius, &mut line, color);
        }
        DebugShape::Cylinder {
            origin,
            half_height,
            radius,
        } => {
            cylinder_lines(*origin, *half_height, *radius, &mut line, color);
        }
        DebugShape::Sphere { origin, radius } => {
            sphere_lines(*origin, *radius, &mut line, color);
        }
        DebugShape::Arrow {
            start,
            end,
            size,
            cap_color,
        } => arrow(*start, *end, *size, *cap_color, color),
        DebugShape::Ray {
            origin,
            direction,
            length,
            size,
        } => arrow(*origin, add3(*origin, scale3(*direction, *length)), *size, color, color),
    }
    lines
}

fn circle_lines(origin: Vec3, radius: f32, line: &mut dyn FnMut(Vec3, Vec3, Vec4), color: Vec4) {
    let count = (5.0 + radius / 8.0).min(16.0).trunc() as usize;
    let point = |i: usize| {
        let angle = i as f32 * std::f32::consts::PI * 2.0 / count as f32;
        Vec3 {
            x: origin.x + angle.cos() * radius,
            y: origin.y + angle.sin() * radius,
            z: origin.z,
        }
    };
    for i in 0..count {
        line(point(i), point((i + 1) % count), color);
    }
}

fn cylinder_lines(origin: Vec3, half_height: f32, radius: f32, line: &mut dyn FnMut(Vec3, Vec3, Vec4), color: Vec4) {
    let count = (5.0 + radius / 8.0).min(16.0).trunc() as usize;
    let point = |i: usize, z: f32| Vec3 {
        x: origin.x + (i as f32 * std::f32::consts::PI * 2.0 / count as f32).cos() * radius,
        y: origin.y + (i as f32 * std::f32::consts::PI * 2.0 / count as f32).sin() * radius,
        z,
    };
    for i in 0..count {
        let bottom = origin.z - half_height;
        let top = origin.z + half_height;
        line(point(i, bottom), point((i + 1) % count, bottom), color);
        line(point(i, top), point((i + 1) % count, top), color);
        line(point(i, bottom), point(i, top), color);
    }
}

fn sphere_lines(origin: Vec3, radius: f32, line: &mut dyn FnMut(Vec3, Vec3, Vec4), color: Vec4) {
    let stacks = (4.0 + radius / 32.0).min(10.0).trunc() as usize;
    let slices = (6.0 + radius / 32.0).min(16.0).trunc() as usize;
    let ring = |stack: usize, slice: usize| {
        let phi = std::f32::consts::PI * (stack + 1) as f32 / stacks as f32;
        let theta = std::f32::consts::PI * 2.0 * slice as f32 / slices as f32;
        add3(
            origin,
            scale3(
                Vec3 {
                    x: phi.sin() * theta.cos(),
                    y: phi.sin() * theta.sin(),
                    z: phi.cos(),
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
    for i in 0..slices {
        let next = (i + 1) % slices;
        line(north, ring(0, next), color);
        line(ring(0, next), ring(0, i), color);
        line(ring(0, i), north, color);
        line(south, ring(stacks - 2, i), color);
        line(ring(stacks - 2, i), ring(stacks - 2, next), color);
        line(ring(stacks - 2, next), south, color);
    }
    for j in 0..stacks - 2 {
        for i in 0..slices {
            let next = (i + 1) % slices;
            line(ring(j, i), ring(j, next), color);
            line(ring(j, next), ring(j + 1, next), color);
            line(ring(j + 1, next), ring(j + 1, i), color);
            line(ring(j + 1, i), ring(j, i), color);
        }
    }
}

/// `PF_Draw_*` converts float seconds to unsigned milliseconds, including
/// negative wrapping.
pub fn rerelease_debug_lifetime(seconds: f32) -> Result<u32, DebugShapeError> {
    if !seconds.is_finite() {
        return Err(DebugShapeError::NonFinite);
    }
    Ok((seconds * 1000.0).trunc() as i64 as u32)
}

/// `Draw_*` import dispatch.
pub struct RereleaseDebugShapeImports {
    /// Emitted events in dispatch order.
    pub events: Vec<RereleaseDebugShapesEvent>,
}

impl RereleaseDebugShapeImports {
    /// Create an empty dispatcher.
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Dispatch one import; returns `None` for other APIs/names.
    pub fn invoke(
        &mut self,
        memory: &mut SparseGuestMemory,
        api: &str,
        name: &str,
        args: &[GuestCallValue],
    ) -> Option<Result<GuestCallResult, DebugShapeError>> {
        if api != "game" {
            return None;
        }
        match name {
            "Draw_Line" | "Draw_Point" | "Draw_Circle" | "Draw_Bounds" | "Draw_Sphere" | "Draw_Cylinder"
            | "Draw_Ray" | "Draw_Arrow" => {}
            _ => return None,
        }
        Some(self.dispatch(memory, name, args))
    }

    fn dispatch(
        &mut self,
        memory: &mut SparseGuestMemory,
        name: &str,
        args: &[GuestCallValue],
    ) -> Result<GuestCallResult, DebugShapeError> {
        let pointer = |index: usize| -> Option<GuestAddress> {
            match args.get(index) {
                Some(GuestCallValue::Pointer(address)) => *address,
                _ => None,
            }
        };
        let integer = |index: usize| -> i64 {
            match args.get(index) {
                Some(GuestCallValue::Int32(value)) => i64::from(*value),
                Some(GuestCallValue::Uint32(value)) => i64::from(*value),
                Some(GuestCallValue::Int64(value)) => *value,
                Some(GuestCallValue::Uint64(value)) => *value as i64,
                _ => 0,
            }
        };
        let mut vector = |memory: &mut SparseGuestMemory, index: usize| -> Result<Vec3, DebugShapeError> {
            let address = pointer(index).ok_or(DebugShapeError::NonFinite)?;
            Ok(memory.read_f32x3(address)?)
        };
        let float = |index: usize| -> Result<f32, DebugShapeError> {
            match args.get(index) {
                Some(GuestCallValue::Float32(value)) if value.is_finite() => Ok(*value),
                _ => Err(DebugShapeError::NonFinite),
            }
        };
        let mut color = |memory: &mut SparseGuestMemory, index: usize| -> Result<Vec4, DebugShapeError> {
            let address = pointer(index).ok_or(DebugShapeError::NonFinite)?;
            let byte = |offset: i64| -> Result<f32, DebugShapeError> {
                Ok(f32::from(memory.read_u8(memory.offset(address, offset)?)?) / 255.0)
            };
            Ok(Vec4 {
                x: byte(0)?,
                y: byte(1)?,
                z: byte(2)?,
                w: byte(3)?,
            })
        };
        let (shape, color_index, lifetime_index) = match name {
            "Draw_Line" => (
                DebugShape::Line {
                    start: vector(memory, 0)?,
                    end: vector(memory, 1)?,
                },
                2,
                3,
            ),
            "Draw_Point" => (
                DebugShape::Point {
                    origin: vector(memory, 0)?,
                    size: float(1)?,
                },
                2,
                3,
            ),
            "Draw_Circle" => (
                DebugShape::Circle {
                    origin: vector(memory, 0)?,
                    radius: float(1)?,
                },
                2,
                3,
            ),
            "Draw_Sphere" => (
                DebugShape::Sphere {
                    origin: vector(memory, 0)?,
                    radius: float(1)?,
                },
                2,
                3,
            ),
            "Draw_Bounds" => (
                DebugShape::Bounds {
                    min: vector(memory, 0)?,
                    max: vector(memory, 1)?,
                },
                2,
                3,
            ),
            "Draw_Cylinder" => (
                DebugShape::Cylinder {
                    origin: vector(memory, 0)?,
                    half_height: float(1)?,
                    radius: float(2)?,
                },
                3,
                4,
            ),
            "Draw_Ray" => (
                DebugShape::Ray {
                    origin: vector(memory, 0)?,
                    direction: vector(memory, 1)?,
                    length: float(2)?,
                    size: float(3)?,
                },
                4,
                5,
            ),
            _ => (
                DebugShape::Arrow {
                    start: vector(memory, 0)?,
                    end: vector(memory, 1)?,
                    size: float(2)?,
                    cap_color: color(memory, 4)?,
                },
                3,
                5,
            ),
        };
        let tint = color(memory, color_index)?;
        let depth_test = integer(lifetime_index + 1) != 0;
        let lifetime = rerelease_debug_lifetime(float(lifetime_index)?)?;
        self.events.push(RereleaseDebugShapesEvent {
            lines: debug_shape_lines(&shape, tint, depth_test),
            lifetime_milliseconds: lifetime,
        });
        Ok(GuestCallResult::Void)
    }
}

impl Default for RereleaseDebugShapeImports {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "debug-shapes-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn write_vec(memory: &mut SparseGuestMemory, value: Vec3) -> GuestAddress {
        let address = memory.allocate(&GuestAllocationOptions::bytes(12)).expect("alloc");
        memory.write_f32(address, value.x).expect("x");
        memory
            .write_f32(memory.offset(address, 4).expect("o"), value.y)
            .expect("y");
        memory
            .write_f32(memory.offset(address, 8).expect("o"), value.z)
            .expect("z");
        address
    }

    fn write_color(memory: &mut SparseGuestMemory) -> GuestAddress {
        let address = memory.allocate(&GuestAllocationOptions::bytes(4)).expect("alloc");
        for (index, byte) in [255u8, 0, 0, 255].into_iter().enumerate() {
            memory
                .write_u8(memory.offset(address, index as i64).expect("o"), byte)
                .expect("byte");
        }
        address
    }

    #[test]
    fn draw_line_and_point_emit_lines() {
        let mut memory = test_memory();
        let mut imports = RereleaseDebugShapeImports::new();
        let start = write_vec(&mut memory, Vec3 { x: 0.0, y: 0.0, z: 0.0 });
        let end = write_vec(&mut memory, Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        let tint = write_color(&mut memory);
        imports
            .invoke(
                &mut memory,
                "game",
                "Draw_Line",
                &[
                    GuestCallValue::Pointer(Some(start)),
                    GuestCallValue::Pointer(Some(end)),
                    GuestCallValue::Pointer(Some(tint)),
                    GuestCallValue::Float32(1.5),
                    GuestCallValue::Int32(1),
                ],
            )
            .unwrap()
            .expect("line");
        assert_eq!(imports.events.len(), 1);
        let event = &imports.events[0];
        assert_eq!(event.lines.len(), 1);
        assert_eq!(event.lifetime_milliseconds, 1500);
        assert!(event.lines[0].depth_test);
        assert_eq!(event.lines[0].color.x, 1.0);
        imports
            .invoke(
                &mut memory,
                "game",
                "Draw_Point",
                &[
                    GuestCallValue::Pointer(Some(start)),
                    GuestCallValue::Float32(4.0),
                    GuestCallValue::Pointer(Some(tint)),
                    GuestCallValue::Float32(0.5),
                    GuestCallValue::Int32(0),
                ],
            )
            .unwrap()
            .expect("point");
        assert_eq!(imports.events[1].lines.len(), 3);
        assert_eq!(imports.events[1].lifetime_milliseconds, 500);
        assert!(!imports.events[1].lines[0].depth_test);
    }

    #[test]
    fn bounds_tessellation_and_lifetime_wrap() {
        let red = Vec4 {
            x: 1.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        };
        let bounds = DebugShape::Bounds {
            min: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            max: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
        };
        assert_eq!(debug_shape_lines(&bounds, red, false).len(), 12);
        let circle = DebugShape::Circle {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            radius: 8.0,
        };
        assert_eq!(debug_shape_lines(&circle, red, true).len(), 6);
        assert_eq!(rerelease_debug_lifetime(2.0).unwrap(), 2000);
        assert_eq!(rerelease_debug_lifetime(-1.0).unwrap(), (-1000i64) as u32);
        assert_eq!(
            rerelease_debug_lifetime(f32::INFINITY).unwrap_err(),
            DebugShapeError::NonFinite
        );
        let mut imports = RereleaseDebugShapeImports::new();
        let mut memory = test_memory();
        assert!(imports.invoke(&mut memory, "game", "Com_Print", &[]).is_none());
    }
}
