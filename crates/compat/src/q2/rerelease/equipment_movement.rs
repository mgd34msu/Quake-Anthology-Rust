//! Q2 rerelease equipment movement guard for Pmove.
//!
//! Donor: `src/compat/q2/rerelease/equipment-movement.ts` — bridges
//! external equipment velocity, gravity, speed and pose around native Pmove.

use qa_core::math::{Bounds, Vec3};
use qa_guest::GuestError;
use qa_guest::core::contracts::GuestAddress;
use qa_guest::core::memory::SparseGuestMemory;
use thiserror::Error;

use super::layouts::{field_offset, pmove_layout};

/// `PMF_DUCKED` (`movement/q2/types.ts`).
pub const PMF_DUCKED: u16 = 1;
/// `PMF_NO_POSITIONAL_PREDICTION` bit used for suppression.
pub const PMF_NO_POSITIONAL_PREDICTION: u16 = 64;
/// `KexPmTypeT.PM_FREEZE` (`movement/q2/types.ts`).
pub const KEX_PM_FREEZE: i32 = 6;

/// Equipment movement failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EquipmentMovementError {
    /// Equipment movement speed must be finite and positive.
    #[error("Equipment movement speed must be finite and positive")]
    BadSpeed,
    /// Speed register outside the XMM file.
    #[error("Equipment speed load targets an unknown register")]
    BadRegister,
    /// Guest memory failure.
    #[error(transparent)]
    Guest(#[from] GuestError),
}

/// Forced equipment pose applied around native movement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquipmentPose {
    /// View height.
    pub view_height: i8,
    /// Hull bounds.
    pub bounds: Bounds,
    /// Crouched flag.
    pub crouched: bool,
}

/// External equipment movement adjustment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquipmentMovement {
    /// Forced velocity, if any.
    pub velocity: Option<Vec3>,
    /// Gravity scale.
    pub gravity_scale: f32,
    /// Prediction suppression flag.
    pub prediction_suppressed: bool,
    /// Speed multiplier.
    pub speed_multiplier: f32,
    /// Forced pose, if any.
    pub pose: Option<EquipmentPose>,
}

impl Default for EquipmentMovement {
    fn default() -> Self {
        Self {
            velocity: None,
            gravity_scale: 1.0,
            prediction_suppressed: false,
            speed_multiplier: 1.0,
            pose: None,
        }
    }
}

/// One profiled speed load: XMM register lane scaled by the multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedLoad {
    /// Continuation RVA (checked by the caller profile).
    pub next: u64,
    /// XMM register index.
    pub register: usize,
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

/// External equipment velocity enters before original component input
/// callbacks.
pub fn prepare_equipment_movement(
    memory: &mut SparseGuestMemory,
    pmove: GuestAddress,
    equipment: &EquipmentMovement,
) -> Result<(), EquipmentMovementError> {
    let layout = pmove_layout();
    let at = |name: &str| {
        field_offset(&layout, name)
            .map(|offset| offset as i64)
            .map_err(|_| EquipmentMovementError::BadSpeed)
    };
    if let Some(velocity) = equipment.velocity {
        write_vec3(memory, pmove, at("s.velocity")?, velocity)?;
    }
    let gravity = at("s.gravity")?;
    let scaled = f32::from(memory.read_i16(memory.offset(pmove, gravity)?)?)
        * equipment.gravity_scale;
    memory.write_i16(memory.offset(pmove, gravity)?, scaled.trunc() as i16)?;
    let flags = at("s.pm_flags")?;
    let current = memory.read_u16(memory.offset(pmove, flags)?)?;
    memory.write_u16(
        memory.offset(pmove, flags)?,
        if equipment.prediction_suppressed {
            current | PMF_NO_POSITIONAL_PREDICTION
        } else {
            current & !PMF_NO_POSITIONAL_PREDICTION
        },
    )?;
    Ok(())
}

/// Scale the profiled XMM lanes by the equipment multiplier. Headless port
/// of the speed-load entry hooks: `lanes` holds the low float of each of
/// the 16 XMM registers.
pub fn apply_speed_loads(
    lanes: &mut [f32; 16],
    loads: &[SpeedLoad],
    multiplier: f32,
) -> Result<(), EquipmentMovementError> {
    if !multiplier.is_finite() || multiplier <= 0.0 {
        return Err(EquipmentMovementError::BadSpeed);
    }
    if multiplier == 1.0 {
        return Ok(());
    }
    for load in loads {
        let Some(lane) = lanes.get_mut(load.register) else {
            return Err(EquipmentMovementError::BadRegister);
        };
        *lane *= multiplier;
    }
    Ok(())
}

/// The original Pmove still owns angles, collision, contacts and source
/// state. Headless port: `lanes` emulates the XMM file for speed loads and
/// `execute` runs the native movement body.
pub fn with_equipment_movement<R>(
    memory: &mut SparseGuestMemory,
    pmove: GuestAddress,
    lanes: &mut [f32; 16],
    speed_loads: &[SpeedLoad],
    equipment: Option<&EquipmentMovement>,
    execute: impl FnOnce(&mut SparseGuestMemory) -> R,
) -> Result<R, EquipmentMovementError> {
    let layout = pmove_layout();
    let at = |name: &str| {
        field_offset(&layout, name)
            .map(|offset| offset as i64)
            .map_err(|_| EquipmentMovementError::BadSpeed)
    };
    let flags = at("s.pm_flags")?;
    let move_type = at("s.pm_type")?;
    let original_type = memory.read_i32(memory.offset(pmove, move_type)?)?;
    let multiplier = equipment.map_or(1.0, |value| value.speed_multiplier);
    if !multiplier.is_finite() || multiplier <= 0.0 {
        return Err(EquipmentMovementError::BadSpeed);
    }
    apply_speed_loads(lanes, speed_loads, multiplier)?;
    let pose = equipment.and_then(|value| value.pose);
    if pose.is_some() {
        memory.write_i32(memory.offset(pmove, move_type)?, KEX_PM_FREEZE)?;
        write_vec3(
            memory,
            pmove,
            at("s.velocity")?,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
        )?;
    }
    let result = execute(memory);
    if pose.is_some() {
        memory.write_i32(memory.offset(pmove, move_type)?, original_type)?;
    }
    if let Some(pose) = pose {
        memory.write_i8(memory.offset(pmove, at("s.viewheight")?)?, pose.view_height)?;
        write_vec3(memory, pmove, at("mins")?, pose.bounds.min)?;
        write_vec3(memory, pmove, at("maxs")?, pose.bounds.max)?;
        let current = memory.read_u16(memory.offset(pmove, flags)?)?;
        memory.write_u16(
            memory.offset(pmove, flags)?,
            if pose.crouched {
                current | PMF_DUCKED
            } else {
                current & !PMF_DUCKED
            },
        )?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};

    fn test_memory() -> SparseGuestMemory {
        let module = ModuleIdentity::new(
            ProviderId::new("q2", "equipment-test"),
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
        memory
            .write_i16(
                memory
                    .offset(
                        address,
                        field_offset(&layout, "s.gravity").expect("g") as i64,
                    )
                    .expect("o"),
                800,
            )
            .expect("gravity");
        address
    }

    #[test]
    fn prepare_enters_velocity_gravity_and_suppression() {
        let mut memory = test_memory();
        let pmove = pmove_block(&mut memory);
        let layout = pmove_layout();
        prepare_equipment_movement(
            &mut memory,
            pmove,
            &EquipmentMovement {
                velocity: Some(Vec3 {
                    x: 1.0,
                    y: 2.0,
                    z: 3.0,
                }),
                gravity_scale: 0.5,
                prediction_suppressed: true,
                ..EquipmentMovement::default()
            },
        )
        .expect("prepare");
        let velocity = memory
            .offset(
                pmove,
                field_offset(&layout, "s.velocity").expect("v") as i64,
            )
            .expect("o");
        assert_eq!(memory.read_f32(velocity).expect("x"), 1.0);
        let gravity = memory
            .offset(
                pmove,
                field_offset(&layout, "s.gravity").expect("g") as i64,
            )
            .expect("o");
        assert_eq!(memory.read_i16(gravity).expect("g"), 400);
        let flags = memory
            .offset(
                pmove,
                field_offset(&layout, "s.pm_flags").expect("f") as i64,
            )
            .expect("o");
        assert_eq!(
            memory.read_u16(flags).expect("f") & PMF_NO_POSITIONAL_PREDICTION,
            PMF_NO_POSITIONAL_PREDICTION
        );
    }

    #[test]
    fn pose_freezes_scales_and_restores() {
        let mut memory = test_memory();
        let pmove = pmove_block(&mut memory);
        let layout = pmove_layout();
        let type_at = memory
            .offset(
                pmove,
                field_offset(&layout, "s.pm_type").expect("t") as i64,
            )
            .expect("o");
        memory.write_i32(type_at, 1).expect("type");
        let mut lanes = [1.0f32; 16];
        let loads = [
            SpeedLoad {
                next: 0xe8293,
                register: 0,
            },
            SpeedLoad {
                next: 0xe889c,
                register: 1,
            },
        ];
        let equipment = EquipmentMovement {
            speed_multiplier: 2.0,
            pose: Some(EquipmentPose {
                view_height: 12,
                bounds: Bounds {
                    min: Vec3 {
                        x: -8.0,
                        y: -8.0,
                        z: -8.0,
                    },
                    max: Vec3 {
                        x: 8.0,
                        y: 8.0,
                        z: 8.0,
                    },
                },
                crouched: true,
            }),
            ..EquipmentMovement::default()
        };
        with_equipment_movement(
            &mut memory,
            pmove,
            &mut lanes,
            &loads,
            Some(&equipment),
            |memory| {
                assert_eq!(memory.read_i32(type_at).expect("frozen"), KEX_PM_FREEZE);
            },
        )
        .expect("guard");
        assert_eq!(lanes[0], 2.0);
        assert_eq!(lanes[1], 2.0);
        assert_eq!(lanes[2], 1.0);
        assert_eq!(memory.read_i32(type_at).expect("type"), 1);
        let flags = memory
            .offset(
                pmove,
                field_offset(&layout, "s.pm_flags").expect("f") as i64,
            )
            .expect("o");
        assert_eq!(memory.read_u16(flags).expect("f") & PMF_DUCKED, PMF_DUCKED);
        let bad = EquipmentMovement {
            speed_multiplier: 0.0,
            ..EquipmentMovement::default()
        };
        assert_eq!(
            with_equipment_movement(&mut memory, pmove, &mut lanes, &loads, Some(&bad), |_| {})
                .unwrap_err(),
            EquipmentMovementError::BadSpeed
        );
    }
}
