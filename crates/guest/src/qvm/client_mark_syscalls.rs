//! Cgame mark-fragment trap.
//!
//! Provenance: `src/compat/qvm/client-mark-syscalls.ts`
//! (`CG_CM_MARKFRAGMENTS` from id Software `code/client/cl_cgame.c` and
//! `cgame/cg_public.h`; output order follows `renderer/tr_marks.c`
//! `R_AddMarkFragments`). The donor's guest-memory closures become an owned
//! input/output exchange: the bridge reads the projection and input points,
//! the host returns fragments plus clipped points, and the bridge writes the
//! records back.

use qa_core::math::Vec3;

use super::client_state::{CallKind, HostCall, QvmRole, SyscallMemory};
use super::legacy_bot_abi::CG_CM_MARKFRAGMENTS;
use crate::error::GuestError;

/// One mark fragment: first output point plus point count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkFragment {
    /// Index of the first output point.
    pub first_point: i32,
    /// Number of output points.
    pub point_count: i32,
}

/// Mark projection result: fragments plus clipped output points.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkProjection {
    /// Fragments referencing `points` by index.
    pub fragments: Vec<MarkFragment>,
    /// Clipped output points.
    pub points: Vec<Vec3>,
}

/// Host mark projector (the active renderer owns the BSP and mark grids).
pub trait MarkProjectorHost {
    /// Project marks, honoring the point and fragment capacities.
    fn mark_fragments(
        &mut self,
        point_count: i32,
        projection: Vec3,
        input: &[Vec3],
        max_points: i32,
        max_fragments: i32,
    ) -> MarkProjection;
}

/// Dispatch the mark-fragments trap. Returns `Ok(None)` when unhandled.
pub fn client_mark_syscall(
    call: &HostCall,
    memory: &mut SyscallMemory,
    projector: &mut dyn MarkProjectorHost,
) -> Result<Option<i32>, GuestError> {
    if call.kind != CallKind::Engine || call.role != QvmRole::Cgame || call.code != CG_CM_MARKFRAGMENTS {
        return Ok(None);
    }
    let point_count = call.int(1)?;
    let input_word = call.int(2)?;
    let projection_word = call.int(3)?;
    let max_points = call.int(4)?;
    let points_word = call.int(5)?;
    let max_fragments = call.int(6)?;
    let fragments_word = call.int(7)?;
    if point_count < 0 || max_points < 0 || max_fragments < 0 {
        return Err(GuestError::invalid("QVM mark counts must be nonnegative"));
    }
    // Guest pointers mask once; subsequent records advance without wrapping.
    let projection = memory.read_vec3_ptr(projection_word)?;
    let input_base = memory.pointer(input_word).ok_or_else(|| {
        GuestError::invalid("QVM mark input points requires a nonnull pointer")
    })?;
    let points_base = memory.pointer(points_word).ok_or_else(|| {
        GuestError::invalid("QVM mark output points requires a nonnull pointer")
    })?;
    let fragments_base = memory.pointer(fragments_word).ok_or_else(|| {
        GuestError::invalid("QVM mark fragments requires a nonnull pointer")
    })?;
    let mut input = Vec::with_capacity(point_count as usize);
    for index in 0..point_count as usize {
        input.push(memory.read_vec3(input_base + index * 12)?);
    }
    let result = projector.mark_fragments(point_count, projection, &input, max_points, max_fragments);
    for (index, fragment) in result.fragments.iter().enumerate() {
        memory.write_i32(fragments_base + index * 8, fragment.first_point)?;
        memory.write_i32(fragments_base + index * 8 + 4, fragment.point_count)?;
    }
    for (index, point) in result.points.iter().enumerate() {
        memory.write_vec3(points_base + index * 12, point)?;
    }
    Ok(Some(result.fragments.len() as i32))
}

#[cfg(test)]
mod tests {
    use super::super::client_state::AbiProfile;
    use super::*;

    struct FakeProjector {
        log: Vec<String>,
    }

    impl MarkProjectorHost for FakeProjector {
        fn mark_fragments(
            &mut self,
            point_count: i32,
            projection: Vec3,
            input: &[Vec3],
            max_points: i32,
            max_fragments: i32,
        ) -> MarkProjection {
            self.log.push(format!("mark {point_count} {} {max_points} {max_fragments}", input.len()));
            assert_eq!(projection.x, 0.0);
            MarkProjection {
                fragments: vec![MarkFragment { first_point: 0, point_count: 2 }],
                points: input.iter().take(2).copied().collect(),
            }
        }
    }

    fn call(args: &[i32]) -> HostCall {
        HostCall::engine(QvmRole::Cgame, CG_CM_MARKFRAGMENTS, args, AbiProfile::Modern)
    }

    #[test]
    fn projects_and_writes_records() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        memory.write_vec3(512, &Vec3 { x: 0.0, y: 0.0, z: -1.0 }).unwrap();
        memory.write_vec3(1024, &Vec3 { x: 1.0, y: 0.0, z: 0.0 }).unwrap();
        memory.write_vec3(1036, &Vec3 { x: 0.0, y: 1.0, z: 0.0 }).unwrap();
        memory.write_vec3(1048, &Vec3 { x: 0.0, y: 0.0, z: 1.0 }).unwrap();
        let mut projector = FakeProjector { log: Vec::new() };
        let result = client_mark_syscall(&call(&[3, 1024, 512, 16, 2048, 4, 3072]), &mut memory, &mut projector).unwrap();
        assert_eq!(result, Some(1));
        assert_eq!(memory.read_i32(3072).unwrap(), 0);
        assert_eq!(memory.read_i32(3076).unwrap(), 2);
        assert_eq!(memory.read_vec3(2048).unwrap(), Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        assert_eq!(memory.read_vec3(2060).unwrap(), Vec3 { x: 0.0, y: 1.0, z: 0.0 });
        assert_eq!(projector.log, vec!["mark 3 3 16 4".to_string()]);
    }

    #[test]
    fn rejects_null_and_negative() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut projector = FakeProjector { log: Vec::new() };
        assert!(client_mark_syscall(&call(&[1, 0, 512, 16, 2048, 4, 3072]), &mut memory, &mut projector).is_err());
        assert!(client_mark_syscall(&call(&[-1, 1024, 512, 16, 2048, 4, 3072]), &mut memory, &mut projector).is_err());
    }

    #[test]
    fn routing() {
        let mut memory = SyscallMemory::new(4096).unwrap();
        let mut projector = FakeProjector { log: Vec::new() };
        let other = HostCall::engine(QvmRole::Ui, CG_CM_MARKFRAGMENTS, &[0, 0, 0, 0, 0, 0, 0], AbiProfile::Modern);
        assert_eq!(client_mark_syscall(&other, &mut memory, &mut projector).unwrap(), None);
        let code = HostCall::engine(QvmRole::Cgame, 28, &[0, 0, 0, 0, 0, 0, 0], AbiProfile::Modern);
        assert_eq!(client_mark_syscall(&code, &mut memory, &mut projector).unwrap(), None);
    }
}
