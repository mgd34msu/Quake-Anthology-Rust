//! Donor: `src/compat/q2/classic/pmove.ts` — the API 3 `Pmove` import.
//!
//! Bridges the 240-byte guest `pmove_t` block to the shared classic Q2
//! movement runner: guest words load into `ClassicPmove`, synthetic trace
//! and contents callbacks answer in decoded vectors, and the result commits
//! back to guest words. Native callbacks in the donor read and write
//! `pmove_t` directly, so the donor commits before and reloads after every
//! callback; synthetic callbacks receive decoded vectors and return decoded
//! traces, which makes that marshal round-trip vacuous.

use std::cell::RefCell;

use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{float_to_wrapped_i32, NumericOps};
use qa_guest::core::contracts::GuestAddress;
use qa_guest::core::memory::SparseGuestMemory;
use qa_world::movement::q2::types::{
    ClassicPmove, ClassicPmoveCmd, ClassicPmoveState, MovementEntity, SrcVec3, TraceT,
};
use qa_world::movement::q2::{pm_flags, pm_type, pmove_classic, Q2_PLAYER_BOUNDS};

use super::layout::{ClassicQ2Error, ClassicResult, CLASSIC_Q2_PMOVE_BYTES};

/// Requested movement body bounds around one pmove.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassicMovementBody {
    /// Currently accepted bounds.
    pub current: Bounds,
    /// Requested bounds, if any.
    pub requested: Option<Bounds>,
}

/// Synthetic trace and contents callbacks answering in decoded vectors.
pub trait PmoveTrace {
    /// Sweep `mins`/`maxs` from `start` to `end`.
    fn trace(&mut self, start: SrcVec3, mins: SrcVec3, maxs: SrcVec3, end: SrcVec3) -> TraceT;
    /// Contents at a point.
    fn point_contents(&mut self, point: SrcVec3) -> i32;
}

/// Synthetic edict/entity mapping for touches and ground.
pub trait PmoveEntities {
    /// Map a guest edict pointer to a movement entity.
    fn entity(&mut self, address: Option<GuestAddress>) -> ClassicResult<Option<MovementEntity>>;
    /// Map a movement entity back to its guest edict pointer.
    fn pointer(&mut self, hit: &MovementEntity) -> ClassicResult<Option<GuestAddress>>;
}

/// Equipment adjustments applied around the movement run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquipmentPose {
    /// Forced view height.
    pub view_height: f64,
    /// Forced hull bounds.
    pub bounds: Bounds,
    /// Whether the pose crouches.
    pub crouched: bool,
}

/// Equipment movement adjustments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EquipmentMovement {
    /// Gravity scale.
    pub gravity_scale: f64,
    /// Whether prediction output is suppressed (guest flag bit 6).
    pub prediction_suppressed: bool,
    /// Velocity override in units per second.
    pub velocity: Option<Vec3>,
    /// Speed multiplier.
    pub speed_multiplier: f64,
    /// Frozen pose override.
    pub pose: Option<EquipmentPose>,
}

/// Options for one guest pmove run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClassicGuestPmoveOptions {
    /// Character bounds override.
    pub character_bounds: Option<Bounds>,
    /// Air acceleration override.
    pub air_accelerate: f64,
    /// Equipment adjustments.
    pub equipment: Option<EquipmentMovement>,
}

fn read_i16(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64) -> ClassicResult<i16> {
    Ok(memory.read_i16(memory.offset(address, offset)?)?)
}

fn read_u8(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64) -> ClassicResult<u8> {
    Ok(memory.read_u8(memory.offset(address, offset)?)?)
}

fn read_f32(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64) -> ClassicResult<f32> {
    Ok(memory.read_f32(memory.offset(address, offset)?)?)
}

fn write_i16(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64, value: i16) -> ClassicResult<()> {
    memory.write_i16(memory.offset(address, offset)?, value)?;
    Ok(())
}

fn write_u8(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64, value: u8) -> ClassicResult<()> {
    memory.write_u8(memory.offset(address, offset)?, value)?;
    Ok(())
}

fn write_i32(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64, value: i32) -> ClassicResult<()> {
    memory.write_i32(memory.offset(address, offset)?, value)?;
    Ok(())
}

fn write_f32(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64, value: f32) -> ClassicResult<()> {
    memory.write_f32(memory.offset(address, offset)?, value)?;
    Ok(())
}

fn read_short3(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64) -> ClassicResult<[i32; 3]> {
    Ok([
        i32::from(read_i16(memory, address, offset)?),
        i32::from(read_i16(memory, address, offset + 2)?),
        i32::from(read_i16(memory, address, offset + 4)?),
    ])
}

fn write_short3(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    offset: i64,
    value: [i32; 3],
) -> ClassicResult<()> {
    for (index, word) in value.iter().enumerate() {
        write_i16(memory, address, offset + index as i64 * 2, *word as i16)?;
    }
    Ok(())
}

fn read_float3(memory: &mut SparseGuestMemory, address: GuestAddress, offset: i64) -> ClassicResult<SrcVec3> {
    Ok([
        f64::from(read_f32(memory, address, offset)?),
        f64::from(read_f32(memory, address, offset + 4)?),
        f64::from(read_f32(memory, address, offset + 8)?),
    ])
}

fn write_float3(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    offset: i64,
    value: SrcVec3,
) -> ClassicResult<()> {
    for (index, word) in value.iter().enumerate() {
        write_f32(memory, address, offset + index as i64 * 4, *word as f32)?;
    }
    Ok(())
}

fn apply_equipment(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    equipment: &EquipmentMovement,
    numeric: NumericOps,
) -> ClassicResult<()> {
    let scaled = numeric.mul(f64::from(read_i16(memory, address, 18)?), equipment.gravity_scale);
    let gravity = numeric
        .to_int32(scaled)
        .map_err(|error| ClassicQ2Error::invalid(format!("API 3 Pmove gravity overflow: {error}")))?;
    write_i16(memory, address, 18, gravity as i16)?;
    let flags = read_u8(memory, address, 16)?;
    write_u8(
        memory,
        address,
        16,
        if equipment.prediction_suppressed {
            flags | 64
        } else {
            flags & !64
        },
    )?;
    if let Some(velocity) = equipment.velocity {
        for (index, value) in [velocity.x, velocity.y, velocity.z].iter().enumerate() {
            let scaled = numeric.mul(f64::from(*value), 8.0);
            let word = numeric
                .to_int32(scaled)
                .map_err(|error| ClassicQ2Error::invalid(format!("API 3 Pmove velocity overflow: {error}")))?;
            write_i16(memory, address, 10 + index as i64 * 2, word as i16)?;
        }
    }
    Ok(())
}

fn commit(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    pm: &ClassicPmove<'_>,
    entities: &mut dyn PmoveEntities,
) -> ClassicResult<()> {
    write_i32(memory, address, 0, pm.s.pm_type)?;
    write_short3(memory, address, 4, pm.s.origin)?;
    write_short3(memory, address, 10, pm.s.velocity)?;
    write_u8(memory, address, 16, pm.s.pm_flags as u8)?;
    write_u8(memory, address, 17, pm.s.pm_time as u8)?;
    write_i16(memory, address, 18, float_to_wrapped_i32(pm.s.gravity) as i16)?;
    write_short3(memory, address, 20, pm.s.delta_angles)?;
    write_i32(memory, address, 48, pm.numtouch as i32)?;
    if pm.numtouch > 32 || pm.numtouch > pm.touchents.len() {
        return Err(ClassicQ2Error::invalid("Pmove exceeded MAXTOUCH"));
    }
    for (index, hit) in pm.touchents.iter().take(pm.numtouch).enumerate() {
        memory.write_pointer(memory.offset(address, 52 + index as i64 * 4)?, entities.pointer(hit)?)?;
    }
    write_float3(memory, address, 180, pm.viewangles)?;
    write_f32(memory, address, 192, pm.viewheight as f32)?;
    write_float3(memory, address, 196, pm.mins)?;
    write_float3(memory, address, 208, pm.maxs)?;
    let ground = match &pm.groundentity {
        Some(hit) => entities.pointer(hit)?,
        None => None,
    };
    memory.write_pointer(memory.offset(address, 220)?, ground)?;
    write_i32(memory, address, 224, pm.watertype)?;
    write_i32(memory, address, 228, pm.waterlevel)?;
    Ok(())
}

/// Run source movement over one guest `pmove_t` block.
pub fn run_classic_guest_pmove(
    memory: &mut SparseGuestMemory,
    address: GuestAddress,
    options: &ClassicGuestPmoveOptions,
    numeric: NumericOps,
    body: Option<&ClassicMovementBody>,
    trace: &mut dyn PmoveTrace,
    entities: &mut dyn PmoveEntities,
) -> ClassicResult<()> {
    memory.check(
        address,
        CLASSIC_Q2_PMOVE_BYTES,
        qa_guest::core::contracts::GuestAccess::Read,
    )?;
    if memory.read_pointer(memory.offset(address, 232)?)?.is_none()
        || memory.read_pointer(memory.offset(address, 236)?)?.is_none()
    {
        return Err(ClassicQ2Error::invalid("API 3 Pmove callback pointer is null"));
    }
    if let Some(equipment) = &options.equipment {
        apply_equipment(memory, address, equipment, numeric)?;
    }
    let ground = entities.entity(memory.read_pointer(memory.offset(address, 220)?)?)?;
    let trace = RefCell::new(trace);
    let mut pm = ClassicPmove {
        s: ClassicPmoveState {
            pm_type: memory.read_i32(memory.offset(address, 0)?)?,
            origin: read_short3(memory, address, 4)?,
            velocity: read_short3(memory, address, 10)?,
            pm_flags: i32::from(read_u8(memory, address, 16)?),
            pm_time: i32::from(read_u8(memory, address, 17)?),
            gravity: f64::from(read_i16(memory, address, 18)?),
            delta_angles: read_short3(memory, address, 20)?,
        },
        cmd: ClassicPmoveCmd {
            msec: i32::from(read_u8(memory, address, 28)?),
            angles: read_short3(memory, address, 30)?,
            forwardmove: f64::from(read_i16(memory, address, 36)?),
            sidemove: f64::from(read_i16(memory, address, 38)?),
            upmove: f64::from(read_i16(memory, address, 40)?),
            buttons: i32::from(read_u8(memory, address, 29)?),
            impulse: i32::from(read_u8(memory, address, 42)?),
            lightlevel: i32::from(read_u8(memory, address, 43)?),
        },
        snapinitial: memory.read_i32(memory.offset(address, 44)?)? != 0,
        numtouch: 0,
        touchents: Vec::new(),
        touchtraces: Vec::new(),
        viewangles: read_float3(memory, address, 180)?,
        viewheight: f64::from(read_f32(memory, address, 192)?),
        mins: read_float3(memory, address, 196)?,
        maxs: read_float3(memory, address, 208)?,
        groundentity: ground,
        watertype: memory.read_i32(memory.offset(address, 224)?)?,
        waterlevel: memory.read_i32(memory.offset(address, 228)?)?,
        trace: Box::new(|start, mins, maxs, end| trace.borrow_mut().trace(start, mins, maxs, end)),
        pointcontents: Box::new(|point| trace.borrow_mut().point_contents(point)),
        character_bounds: options.character_bounds.unwrap_or(Q2_PLAYER_BOUNDS),
        body_bounds: body.and_then(|body| body.requested),
        previous_bounds: body.map(|body| body.current),
    };
    let pose = options.equipment.and_then(|equipment| equipment.pose);
    let pm_type_saved = pm.s.pm_type;
    if pose.is_some() {
        pm.s.pm_type = pm_type::FREEZE;
        pm.s.velocity = [0, 0, 0];
    }
    pmove_classic(
        &mut pm,
        numeric,
        options.air_accelerate,
        false,
        false,
        options.equipment.map_or(1.0, |equipment| equipment.speed_multiplier),
    );
    if let Some(pose) = pose {
        pm.s.pm_type = pm_type_saved;
        pm.viewheight = pose.view_height;
        pm.mins = [
            f64::from(pose.bounds.min.x),
            f64::from(pose.bounds.min.y),
            f64::from(pose.bounds.min.z),
        ];
        pm.maxs = [
            f64::from(pose.bounds.max.x),
            f64::from(pose.bounds.max.y),
            f64::from(pose.bounds.max.z),
        ];
        if pose.crouched {
            pm.s.pm_flags |= pm_flags::DUCKED;
        } else {
            pm.s.pm_flags &= !pm_flags::DUCKED;
        }
    }
    commit(memory, address, &pm, entities)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::ProviderId;
    use qa_core::numeric::Q2_DONOR_PROFILE;
    use qa_guest::core::contracts::{ContentDigest, GuestAllocationOptions, ModuleIdentity};
    use qa_world::movement::q2::types::plane;

    fn test_memory() -> SparseGuestMemory {
        SparseGuestMemory::new(
            ModuleIdentity::new(
                ProviderId::new("q2", "pmove-test"),
                "pmove",
                ContentDigest::new("sha256", "0"),
                "test",
            ),
            4,
            0x10000,
        )
        .unwrap()
    }

    fn numeric() -> NumericOps {
        NumericOps::select(Q2_DONOR_PROFILE).unwrap()
    }

    struct OpenTrace {
        calls: usize,
    }

    impl PmoveTrace for OpenTrace {
        fn trace(&mut self, _start: SrcVec3, _mins: SrcVec3, _maxs: SrcVec3, end: SrcVec3) -> TraceT {
            self.calls += 1;
            TraceT {
                allsolid: false,
                startsolid: false,
                fraction: 1.0,
                endpos: end,
                plane: plane(),
                surface: None,
                contents: 0,
                ent: None,
                plane2: plane(),
                surface2: None,
                native: None,
            }
        }

        fn point_contents(&mut self, _point: SrcVec3) -> i32 {
            0
        }
    }

    struct NullEntities;

    impl PmoveEntities for NullEntities {
        fn entity(&mut self, address: Option<GuestAddress>) -> ClassicResult<Option<MovementEntity>> {
            Ok(address.map(|_| MovementEntity::World { model: 0 }))
        }

        fn pointer(&mut self, hit: &MovementEntity) -> ClassicResult<Option<GuestAddress>> {
            if *hit == MovementEntity::None {
                Ok(None)
            } else {
                Err(ClassicQ2Error::invalid("unexpected pmove touch entity"))
            }
        }
    }

    fn pmove_block(memory: &mut SparseGuestMemory) -> GuestAddress {
        let address = memory
            .allocate(&GuestAllocationOptions::bytes(CLASSIC_Q2_PMOVE_BYTES))
            .unwrap();
        let code = memory.allocate(&GuestAllocationOptions::bytes(8)).unwrap();
        memory
            .write_pointer(memory.offset(address, 232).unwrap(), Some(code))
            .unwrap();
        memory
            .write_pointer(memory.offset(address, 236).unwrap(), Some(code))
            .unwrap();
        memory.write_i32(address, pm_type::NORMAL).unwrap();
        address
    }

    #[test]
    fn open_air_fall_moves_and_commits() {
        let mut memory = test_memory();
        let address = pmove_block(&mut memory);
        memory.write_i16(memory.offset(address, 18).unwrap(), 800).unwrap();
        memory.write_u8(memory.offset(address, 28).unwrap(), 100).unwrap();
        memory.write_i16(memory.offset(address, 4 + 4).unwrap(), 800).unwrap();
        let mut trace = OpenTrace { calls: 0 };
        let mut entities = NullEntities;
        run_classic_guest_pmove(
            &mut memory,
            address,
            &ClassicGuestPmoveOptions::default(),
            numeric(),
            None,
            &mut trace,
            &mut entities,
        )
        .unwrap();
        assert!(trace.calls > 0);
        assert_eq!(memory.read_i32(address).unwrap(), pm_type::NORMAL);
        assert_eq!(memory.read_i32(memory.offset(address, 48).unwrap()).unwrap(), 0);
        let gravity = memory.read_i16(memory.offset(address, 18).unwrap()).unwrap();
        assert_eq!(gravity, 800);
    }

    #[test]
    fn equipment_and_pose_adjust_guest_words() {
        let mut memory = test_memory();
        let address = pmove_block(&mut memory);
        memory.write_i16(memory.offset(address, 18).unwrap(), 800).unwrap();
        memory.write_u8(memory.offset(address, 28).unwrap(), 50).unwrap();
        let options = ClassicGuestPmoveOptions {
            character_bounds: None,
            air_accelerate: 0.0,
            equipment: Some(EquipmentMovement {
                gravity_scale: 0.5,
                prediction_suppressed: true,
                velocity: Some(Vec3 { x: 8.0, y: 0.0, z: 0.0 }),
                speed_multiplier: 1.0,
                pose: Some(EquipmentPose {
                    view_height: 22.0,
                    bounds: Q2_PLAYER_BOUNDS,
                    crouched: true,
                }),
            }),
        };
        let mut trace = OpenTrace { calls: 0 };
        let mut entities = NullEntities;
        run_classic_guest_pmove(
            &mut memory,
            address,
            &options,
            numeric(),
            None,
            &mut trace,
            &mut entities,
        )
        .unwrap();
        assert_eq!(memory.read_i16(memory.offset(address, 18).unwrap()).unwrap(), 400);
        assert_eq!(memory.read_u8(memory.offset(address, 16).unwrap()).unwrap() & 64, 64);
        assert_eq!(memory.read_f32(memory.offset(address, 192).unwrap()).unwrap(), 22.0);
        assert_eq!(memory.read_i32(address).unwrap(), pm_type::NORMAL);
        let null = pmove_block(&mut memory);
        memory.write_pointer(memory.offset(null, 232).unwrap(), None).unwrap();
        assert!(run_classic_guest_pmove(
            &mut memory,
            null,
            &ClassicGuestPmoveOptions::default(),
            numeric(),
            None,
            &mut trace,
            &mut entities
        )
        .is_err());
    }
}
