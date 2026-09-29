//! `SnapVector` traps for the game and client-game modules.
//!
//! Port of `src/compat/qvm/snap-vector-syscalls.ts` (`Sys_SnapVector` from
//! `unix/snapvector.nasm`, reached through `G_SNAPVECTOR` in `server/sv_game.c`
//! and `CG_SNAPVECTOR` in `client/cl_cgame.c`; Copyright (C) 1999-2005
//! Id Software, Inc., GPL-2.0-or-later).
//!
//! Selected Linux i386 x87 profile: cw037F nearest-even, masked exceptions,
//! signed dword fistp/fild, then binary32 fstp. This is not the C rint profile.
//! `None` means another service owns the trap.

use crate::error::GuestError;

use super::memory::{QvmMemory, QvmWritableView};
use super::syscalls::QvmSyscallRole;

/// Snap one component: round half to even in binary64, saturate out-of-range
/// integers to `INT_MIN`, and normalize zero to `+0`.
// Exact float comparisons mirror the donor x87 profile bit for bit.
#[allow(clippy::float_cmp)]
fn snap_component(value: f64) -> i32 {
    let lower = value.floor();
    let fraction = value - lower;
    // NaN fractions (infinite inputs) fall into the parity branch exactly like
    // the donor's chained comparisons, then saturate below.
    let integer = match fraction.partial_cmp(&0.5) {
        Some(std::cmp::Ordering::Less) => lower,
        Some(std::cmp::Ordering::Greater) => lower + 1.0,
        _ => {
            if lower % 2.0 == 0.0 {
                lower
            } else {
                lower + 1.0
            }
        }
    };
    if !integer.is_finite() || integer < f64::from(i32::MIN) || integer > f64::from(i32::MAX) {
        return i32::MIN;
    }
    if integer == 0.0 {
        0
    } else {
        integer as i32
    }
}

/// Handle `G_SNAPVECTOR` (42) / `CG_SNAPVECTOR` (71). The UI has no snap trap.
pub fn qvm_snap_vector_syscall(
    role: QvmSyscallRole,
    words: &QvmWritableView,
    memory: &QvmMemory,
) -> Result<Option<i32>, GuestError> {
    if role == QvmSyscallRole::Ui {
        return Ok(None);
    }
    let trap = if role == QvmSyscallRole::Game { 42 } else { 71 };
    if words.get_i32(0)? != trap {
        return Ok(None);
    }
    let pointer = memory
        .pointer(words.get_i32(4)?)?
        .ok_or_else(|| GuestError::invalid("QVM SnapVector requires a nonnull pointer"))?;
    let vector = pointer.view();
    // Mask only the base; publish each component before reaching the next read.
    for offset in (0..12).step_by(4) {
        let snapped = snap_component(f64::from(vector.get_f32(offset)?));
        vector.set_f32(offset, snapped as f32)?;
    }
    Ok(Some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(role_trap: i32, at: i32) -> (QvmMemory, QvmWritableView) {
        let memory = QvmMemory::new(vec![0; 256]).unwrap();
        let words = memory.data_view(224, 32).unwrap();
        words.set_i32(0, role_trap).unwrap();
        words.set_i32(4, at).unwrap();
        (memory, words)
    }

    fn read_triple(memory: &QvmMemory, at: i32) -> [f32; 3] {
        let view = memory.view(at, 12, 0).unwrap();
        [
            view.get_f32(0).unwrap(),
            view.get_f32(4).unwrap(),
            view.get_f32(8).unwrap(),
        ]
    }

    #[test]
    fn snaps_half_to_even() {
        let (memory, words) = fixture(42, 64);
        let vector = memory.view(64, 12, 0).unwrap();
        vector.set_f32(0, 2.5).unwrap();
        vector.set_f32(4, 3.5).unwrap();
        vector.set_f32(8, -2.5).unwrap();
        assert_eq!(
            qvm_snap_vector_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            Some(0)
        );
        assert_eq!(read_triple(&memory, 64), [2.0, 4.0, -2.0]);
    }

    #[test]
    fn cgame_trap_and_saturation() {
        let (memory, words) = fixture(71, 64);
        let vector = memory.view(64, 12, 0).unwrap();
        vector.set_f32(0, 1e30).unwrap();
        vector.set_f32(4, -0.0).unwrap();
        vector.set_f32(8, 1.4).unwrap();
        assert_eq!(
            qvm_snap_vector_syscall(QvmSyscallRole::Cgame, &words, &memory).unwrap(),
            Some(0)
        );
        let snapped = read_triple(&memory, 64);
        assert_eq!(snapped[0], i32::MIN as f32);
        assert_eq!(snapped[1].to_bits(), 0u32);
        assert_eq!(snapped[2], 1.0);
    }

    #[test]
    fn ui_and_foreign_traps_pass_through() {
        let (memory, words) = fixture(42, 64);
        assert_eq!(
            qvm_snap_vector_syscall(QvmSyscallRole::Ui, &words, &memory).unwrap(),
            None
        );
        let (memory, words) = fixture(71, 64);
        assert_eq!(
            qvm_snap_vector_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            None
        );
        let (memory, words) = fixture(42, 0);
        assert!(qvm_snap_vector_syscall(QvmSyscallRole::Game, &words, &memory).is_err());
    }
}
