//! Q2 rerelease client body-shape guard for Pmove.
//!
//! Donor: `src/compat/q2/rerelease/body-shape.ts` — bridges original
//! dimension decisions and trace policy around native movement.

use qa_core::math::{Bounds, Vec3};
use qa_guest::GuestError;
use qa_guest::core::contracts::{GuestAddress, GuestCallResult, GuestCallValue};
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

use super::layouts::{field_offset, pmove_layout};

/// Body-shape failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BodyShapeError {
    /// Native body shape requires an admitted boundary.
    #[error("Native body shape requires an admitted original dimensions and trace boundary")]
    MissingBoundary,
    /// Original Pmove expansion returned an invalid trace.
    #[error("Original Pmove expansion returned an invalid trace")]
    BadTrace,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Requested body hull with its currently accepted shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementBodyShape {
    /// Currently accepted hull.
    pub current: Bounds,
    /// Requested hull, if any.
    pub requested: Option<Bounds>,
}

/// Admitted original dimensions/trace boundary RVAs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyBoundary {
    /// Dimensions entry RVA.
    pub dimensions: u64,
    /// Trace entry RVA.
    pub trace: u64,
    /// Movement global RVA.
    pub movement_global: u64,
    /// Game API entry RVA.
    pub game_api: u64,
}

/// A requested local hull expands only after the selected source collision
/// query accepts it (`movement/body-shape.ts`).
#[must_use]
pub fn movement_bounds(
    previous: &Bounds,
    requested: &Bounds,
    clear: &dyn Fn(&Bounds) -> bool,
) -> Bounds {
    let expands = requested.min.x < previous.min.x
        || requested.min.y < previous.min.y
        || requested.min.z < previous.min.z
        || requested.max.x > previous.max.x
        || requested.max.y > previous.max.y
        || requested.max.z > previous.max.z;
    if expands && !clear(requested) {
        *previous
    } else {
        *requested
    }
}

fn read_vec3(
    memory: &mut SparseGuestMemory,
    base: GuestAddress,
    offset: i64,
) -> Result<Vec3, GuestError> {
    let at = memory.offset(base, offset)?;
    Ok(Vec3 {
        x: memory.read_f32(at)?,
        y: memory.read_f32(memory.offset(at, 4)?)?,
        z: memory.read_f32(memory.offset(at, 8)?)?,
    })
}

fn write_vec3(
    memory: &mut SparseGuestMemory,
    base: GuestAddress,
    offset: i64,
    value: Vec3,
) -> Result<(), GuestError> {
    let at = memory.offset(base, offset)?;
    memory.write_f32(at, value.x)?;
    memory.write_f32(memory.offset(at, 4)?, value.y)?;
    memory.write_f32(memory.offset(at, 8)?, value.z)?;
    Ok(())
}

/// Cached `pmove_t` field offsets for one guard scope.
#[derive(Debug, Clone, Copy)]
struct PmoveFields {
    /// Origin offset.
    origin: i64,
    /// Mins offset.
    mins: i64,
    /// Maxs offset.
    maxs: i64,
    /// Flags offset.
    flags: i64,
    /// View height offset.
    viewheight: i64,
}

fn pmove_fields() -> Result<PmoveFields, BodyShapeError> {
    let layout = pmove_layout();
    let at = |name: &str| {
        field_offset(&layout, name)
            .map(|offset| offset as i64)
            .map_err(|_| BodyShapeError::BadTrace)
    };
    Ok(PmoveFields {
        origin: at("s.origin")?,
        mins: at("mins")?,
        maxs: at("maxs")?,
        flags: at("s.pm_flags")?,
        viewheight: at("s.viewheight")?,
    })
}

/// Apply one body frame: expand the guarded hull when the trace probe
/// accepts it, restoring the pre-run duck posture otherwise.
pub fn apply_body_frame(
    memory: &mut SparseGuestMemory,
    pmove: GuestAddress,
    accepted: &Bounds,
    desired: &Bounds,
    previous_duck: u16,
    previous_height: i8,
    probe: &dyn Fn(Vec3, &Bounds) -> bool,
) -> Result<Bounds, BodyShapeError> {
    let fields = pmove_fields()?;
    let origin = read_vec3(memory, pmove, fields.origin)?;
    let next = movement_bounds(accepted, desired, &|bounds| probe(origin, bounds));
    if next != *desired {
        let flags = memory.read_u16(memory.offset(pmove, fields.flags)?)?;
        memory.write_u16(
            memory.offset(pmove, fields.flags)?,
            (flags & !1) | (previous_duck & 1),
        )?;
        memory.write_i8(
            memory.offset(pmove, fields.viewheight)?,
            previous_height,
        )?;
    }
    write_vec3(memory, pmove, fields.mins, next.min)?;
    write_vec3(memory, pmove, fields.maxs, next.max)?;
    Ok(next)
}

/// Keep original dimension decisions, trace policy and later specialized
/// probes. Headless port: `run` executes the movement body, then one frame
/// applies the guarded hull; `probe` answers the original trace query.
pub fn with_body_shape<R>(
    memory: &mut SparseGuestMemory,
    pmove: GuestAddress,
    boundary: Option<&BodyBoundary>,
    body: Option<&MovementBodyShape>,
    probe: &dyn Fn(Vec3, &Bounds) -> bool,
    run: impl FnOnce(&mut SparseGuestMemory) -> R,
) -> Result<R, BodyShapeError> {
    let Some(body) = body else {
        return Ok(run(memory));
    };
    let Some(_boundary) = boundary else {
        if body.requested.is_some() {
            return Err(BodyShapeError::MissingBoundary);
        }
        return Ok(run(memory));
    };
    let fields = pmove_fields()?;
    let previous_duck = memory.read_u16(memory.offset(pmove, fields.flags)?)? & 1;
    let previous_height = memory.read_i8(memory.offset(pmove, fields.viewheight)?)?;
    let result = run(memory);
    let desired = match body.requested {
        Some(requested) => requested,
        None => Bounds {
            min: read_vec3(memory, pmove, fields.mins)?,
            max: read_vec3(memory, pmove, fields.maxs)?,
        },
    };
    apply_body_frame(
        memory,
        pmove,
        &body.current,
        &desired,
        previous_duck,
        previous_height,
        probe,
    )?;
    Ok(result)
}

/// Decode the single-byte trace acceptance used by the expansion probe.
pub fn trace_accepts(result: &GuestCallResult) -> Result<bool, BodyShapeError> {
    match result {
        GuestCallResult::Value(GuestCallValue::Aggregate { bytes, .. }) => {
            bytes.first().map(|byte| *byte == 0).ok_or(BodyShapeError::BadTrace)
        }
        _ => Err(BodyShapeError::BadTrace),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "body-shape-test"),
            "game.dll",
            ContentDigest::new("sha256", "00"),
            "test",
        );
        SparseGuestMemory::new(module, 8, 0x1_0000).expect("memory")
    }

    fn pmove_block(memory: &mut SparseGuestMemory) -> GuestAddress {
        let layout = pmove_layout();
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(layout.byte_length))
            .expect("alloc");
        write_vec3(
            memory,
            address,
            field_offset(&layout, "mins").expect("mins") as i64,
            Vec3 {
                x: -16.0,
                y: -16.0,
                z: -24.0,
            },
        )
        .expect("mins");
        write_vec3(
            memory,
            address,
            field_offset(&layout, "maxs").expect("maxs") as i64,
            Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        )
        .expect("maxs");
        address
    }

    fn boundary() -> BodyBoundary {
        BodyBoundary {
            dimensions: 0xe9ff0,
            trace: 0xe71a0,
            movement_global: 0x23c9c8,
            game_api: 0x6bcd0,
        }
    }

    #[test]
    fn expansion_applies_when_probe_accepts() {
        let mut memory = test_memory();
        let pmove = pmove_block(&mut memory);
        let body = MovementBodyShape {
            current: Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            },
            requested: Some(Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 48.0,
                },
            }),
        };
        let seen: std::cell::RefCell<Vec<Bounds>> = std::cell::RefCell::new(Vec::new());
        let probe = |origin: Vec3, bounds: &Bounds| {
            assert_eq!(origin, Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0
            });
            seen.borrow_mut().push(*bounds);
            true
        };
        with_body_shape(
            &mut memory,
            pmove,
            Some(&boundary()),
            Some(&body),
            &probe,
            |memory| {
                memory
                    .write_u16(
                        memory
                            .offset(pmove, field_offset(&pmove_layout(), "s.pm_flags").expect("f") as i64)
                            .expect("o"),
                        1,
                    )
                    .expect("flags");
            },
        )
        .expect("guard");
        assert_eq!(seen.borrow().len(), 1);
        let layout = pmove_layout();
        let max = read_vec3(
            &mut memory,
            pmove,
            field_offset(&layout, "maxs").expect("maxs") as i64,
        )
        .expect("read");
        assert_eq!(max.z, 48.0);
    }

    #[test]
    fn blocked_expansion_keeps_current_and_requires_boundary() {
        let mut memory = test_memory();
        let pmove = pmove_block(&mut memory);
        let body = MovementBodyShape {
            current: Bounds {
                min: Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
            },
            requested: Some(Bounds {
                min: Vec3 {
                    x: -32.0,
                    y: -32.0,
                    z: -24.0,
                },
                max: Vec3 {
                    x: 32.0,
                    y: 32.0,
                    z: 32.0,
                },
            }),
        };
        with_body_shape(
            &mut memory,
            pmove,
            Some(&boundary()),
            Some(&body),
            &|_, _| false,
            |_| {},
        )
        .expect("guard");
        let layout = pmove_layout();
        let min = read_vec3(
            &mut memory,
            pmove,
            field_offset(&layout, "mins").expect("mins") as i64,
        )
        .expect("read");
        assert_eq!(min.x, -16.0);
        let err = with_body_shape(&mut memory, pmove, None, Some(&body), &|_, _| true, |_| {})
            .unwrap_err();
        assert_eq!(err, BodyShapeError::MissingBoundary);
        with_body_shape(&mut memory, pmove, None, None, &|_, _| true, |_| {}).expect("passthrough");
        assert!(trace_accepts(&GuestCallResult::Void).is_err());
    }
}
