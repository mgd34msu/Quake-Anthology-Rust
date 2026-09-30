//! Scalar math traps shared by the game, client-game, and UI modules.
//!
//! Port of `src/compat/qvm/math-syscalls.ts` (scalar traps from Quake III
//! Arena's `cl_cgame.c`, `cl_ui.c` and `sv_game.c`, `Q_acos` from
//! `qcommon/common.c`; Copyright (C) 1999-2005 Id Software, Inc.,
//! GPL-2.0-or-later).
//!
//! `None` means another service owns the trap, not a successful zero result.

use qa_core::numeric::float32_to_bits;

use crate::error::GuestError;

use super::memory::QvmWritableView;
use super::syscalls::QvmSyscallRole;

/// Handle scalar traps 103-111. Returns `None` when another service owns `trap`.
pub fn qvm_math_syscall(role: QvmSyscallRole, words: &QvmWritableView) -> Result<Option<i32>, GuestError> {
    let trap = words.get_i32(0)?;
    // Transcendentals evaluate in binary64, then round once to binary32,
    // matching the donor's Math.* + fround composition.
    let bit64 = |value: f64| float32_to_bits(value as f32) as i32;
    let bit = |value: f32| float32_to_bits(value) as i32;
    match trap {
        103 => Ok(Some(bit64(f64::from(words.get_f32(4)?).sin()))),
        104 => Ok(Some(bit64(f64::from(words.get_f32(4)?).cos()))),
        105 => Ok(Some(bit64(
            f64::from(words.get_f32(4)?).atan2(f64::from(words.get_f32(8)?)),
        ))),
        106 => Ok(Some(bit64(f64::from(words.get_f32(4)?).sqrt()))),
        107 => {
            if role == QvmSyscallRole::Game {
                Ok(None)
            } else {
                Ok(Some(bit(words.get_f32(4)?.floor())))
            }
        }
        108 => {
            if role == QvmSyscallRole::Game {
                Ok(None)
            } else {
                Ok(Some(bit(words.get_f32(4)?.ceil())))
            }
        }
        110 => {
            if role == QvmSyscallRole::Game {
                Ok(Some(bit(words.get_f32(4)?.floor())))
            } else {
                Ok(None)
            }
        }
        111 => {
            if role == QvmSyscallRole::Ui {
                Ok(None)
            } else if role == QvmSyscallRole::Game {
                Ok(Some(bit(words.get_f32(4)?.ceil())))
            } else {
                let angle = f64::from(words.get_f32(4)?).acos() as f32;
                // Source clamps the result, not the input, and returns +PI for
                // either end. NaN (out-of-range input) compares false to both
                // bounds and passes through, matching the donor.
                let clamped = if angle > std::f32::consts::PI || angle < -std::f32::consts::PI {
                    std::f32::consts::PI
                } else {
                    angle
                };
                Ok(Some(bit(clamped)))
            }
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qvm::memory::QvmMemory;

    fn words(trap: i32, args: &[f32]) -> QvmWritableView {
        let memory = QvmMemory::new(vec![0; 64]).unwrap();
        let view = memory.data_view(0, 64).unwrap();
        view.set_i32(0, trap).unwrap();
        for (index, value) in args.iter().enumerate() {
            view.set_f32(4 + index * 4, *value).unwrap();
        }
        // Views share the allocation; the local memory handle can drop.
        view
    }

    fn result(role: QvmSyscallRole, trap: i32, args: &[f32]) -> Option<f32> {
        let view = words(trap, args);
        qvm_math_syscall(role, &view)
            .unwrap()
            .map(|bits| f32::from_bits(bits as u32))
    }

    #[test]
    fn trig_and_sqrt_traps() {
        assert_eq!(result(QvmSyscallRole::Cgame, 103, &[0.0]), Some(0.0));
        assert_eq!(result(QvmSyscallRole::Ui, 104, &[0.0]), Some(1.0));
        assert_eq!(result(QvmSyscallRole::Game, 106, &[9.0]), Some(3.0));
        let atan = result(QvmSyscallRole::Game, 105, &[1.0, 1.0]).unwrap();
        assert!((atan - std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    }

    #[test]
    fn floor_ceil_move_by_role() {
        assert_eq!(result(QvmSyscallRole::Game, 107, &[1.5]), None);
        assert_eq!(result(QvmSyscallRole::Cgame, 107, &[1.5]), Some(1.0));
        assert_eq!(result(QvmSyscallRole::Ui, 108, &[1.5]), Some(2.0));
        assert_eq!(result(QvmSyscallRole::Game, 110, &[1.5]), Some(1.0));
        assert_eq!(result(QvmSyscallRole::Cgame, 110, &[1.5]), None);
        assert_eq!(result(QvmSyscallRole::Game, 111, &[1.5]), Some(2.0));
        assert_eq!(result(QvmSyscallRole::Ui, 111, &[1.5]), None);
    }

    #[test]
    fn cgame_acos_clamps_the_result() {
        assert_eq!(result(QvmSyscallRole::Cgame, 111, &[1.0]), Some(0.0));
        assert!(result(QvmSyscallRole::Cgame, 111, &[2.0]).unwrap().is_nan());
        assert_eq!(result(QvmSyscallRole::Cgame, 99, &[]), None);
    }
}
