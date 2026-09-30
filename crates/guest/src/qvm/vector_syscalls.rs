//! Game-module vector traps: matrix concatenation, angle vectors, perpendicular.
//!
//! Port of `src/compat/qvm/vector-syscalls.ts` (vector traps from Quake III
//! Arena's `sv_game.c` and `game/q_math.c`; Copyright (C) 1999-2005 Id Software,
//! Inc., GPL-2.0-or-later).
//!
//! Host SSE binary32 profile with the double `M_PI` angle constant and no FMA;
//! these are engine calls, not q3lcc arithmetic. Required spans preflight
//! before writes; `None` means another service owns the trap. Vector math
//! reuses `qa_core::math`.

use qa_core::math::{angle_vectors, dot3, perpendicular_vector, vec3, Vec3};

use crate::error::GuestError;

use super::memory::{QvmMemory, QvmWritableView};
use super::syscalls::QvmSyscallRole;

fn read_vector(view: &QvmWritableView) -> Result<Vec3, GuestError> {
    Ok(vec3(view.get_f32(0)?, view.get_f32(4)?, view.get_f32(8)?))
}

fn write_vector(view: &QvmWritableView, value: Vec3) -> Result<(), GuestError> {
    view.set_f32(0, value.x)?;
    view.set_f32(4, value.y)?;
    view.set_f32(8, value.z)?;
    Ok(())
}

fn optional_vector(memory: &QvmMemory, word: i32) -> Result<Option<QvmWritableView>, GuestError> {
    if word == 0 {
        Ok(None)
    } else {
        memory.view(word, 12, 0).map(Some)
    }
}

/// Handle game vector traps 107-109. Returns `None` for other roles or traps.
// The zero-projection check intentionally uses exact equality: like the
// source assertion, NaN is not zero and passes through to the caller.
#[allow(clippy::float_cmp)]
pub fn qvm_vector_syscall(
    role: QvmSyscallRole,
    words: &QvmWritableView,
    memory: &QvmMemory,
) -> Result<Option<i32>, GuestError> {
    if role != QvmSyscallRole::Game {
        return Ok(None);
    }
    match words.get_i32(0)? {
        107 => {
            let left = memory.view(words.get_i32(4)?, 36, 0)?;
            let right = memory.view(words.get_i32(8)?, 36, 0)?;
            let output = memory.view(words.get_i32(12)?, 36, 0)?;
            // C has no restrict here. Each assignment can change later reads.
            for row in 0..3 {
                for column in 0..3 {
                    let a = vec3(
                        left.get_f32(row * 12)?,
                        left.get_f32(row * 12 + 4)?,
                        left.get_f32(row * 12 + 8)?,
                    );
                    let b = vec3(
                        right.get_f32(column * 4)?,
                        right.get_f32(column * 4 + 12)?,
                        right.get_f32(column * 4 + 24)?,
                    );
                    output.set_f32(row * 12 + column * 4, dot3(a, b))?;
                }
            }
            Ok(Some(0))
        }
        108 => {
            let angles = memory.view(words.get_i32(4)?, 12, 0)?;
            let forward = optional_vector(memory, words.get_i32(8)?)?;
            let right = optional_vector(memory, words.get_i32(12)?)?;
            let up = optional_vector(memory, words.get_i32(16)?)?;
            // Source captures every angle before publishing forward, right, up.
            let result = angle_vectors(read_vector(&angles)?);
            if let Some(forward) = forward {
                write_vector(&forward, result.forward)?;
            }
            if let Some(right) = right {
                write_vector(&right, result.right)?;
            }
            if let Some(up) = up {
                write_vector(&up, result.up)?;
            }
            Ok(Some(0))
        }
        109 => {
            let destination = memory.view(words.get_i32(4)?, 12, 0)?;
            let source = read_vector(&memory.view(words.get_i32(8)?, 12, 0)?)?;
            // q_math.c's non-Q3_VM assertion rejects a zero projection
            // denominator. As in the source, callers supply a normalized
            // vector; NaN is not zero.
            if dot3(source, source) == 0.0 {
                return Err(GuestError::invalid(
                    "QVM PerpendicularVector has a zero projection denominator",
                ));
            }
            write_vector(&destination, perpendicular_vector(source))?;
            Ok(Some(0))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(trap: i32, args: &[i32]) -> (QvmMemory, QvmWritableView) {
        let memory = QvmMemory::new(vec![0; 512]).unwrap();
        let words = memory.data_view(480, 32).unwrap();
        words.set_i32(0, trap).unwrap();
        for (index, value) in args.iter().enumerate() {
            words.set_i32(4 + index * 4, *value).unwrap();
        }
        (memory, words)
    }

    fn write_matrix(memory: &QvmMemory, at: i32, rows: [[f32; 3]; 3]) {
        let view = memory.view(at, 36, 0).unwrap();
        for (row, values) in rows.iter().enumerate() {
            for (column, value) in values.iter().enumerate() {
                view.set_f32(row * 12 + column * 4, *value).unwrap();
            }
        }
    }

    fn read_matrix(memory: &QvmMemory, at: i32) -> [[f32; 3]; 3] {
        let view = memory.view(at, 36, 0).unwrap();
        let mut rows = [[0.0; 3]; 3];
        for row in 0..3 {
            for column in 0..3 {
                rows[row][column] = view.get_f32(row * 12 + column * 4).unwrap();
            }
        }
        rows
    }

    #[test]
    fn matrix_concatenation_multiplies_rows_by_columns() {
        let (memory, words) = fixture(107, &[64, 128, 192]);
        write_matrix(&memory, 64, [[1.0, 2.0, 3.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        write_matrix(&memory, 128, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [4.0, 5.0, 6.0]]);
        assert_eq!(
            qvm_vector_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            Some(0)
        );
        assert_eq!(
            read_matrix(&memory, 192),
            [[13.0, 17.0, 18.0], [0.0, 1.0, 0.0], [4.0, 5.0, 6.0]]
        );
    }

    #[test]
    fn angle_vectors_publish_forward_right_up() {
        let (memory, words) = fixture(108, &[64, 128, 0, 192]);
        let angles = memory.view(64, 12, 0).unwrap();
        angles.set_f32(0, 0.0).unwrap();
        angles.set_f32(4, 90.0).unwrap();
        angles.set_f32(8, 0.0).unwrap();
        assert_eq!(
            qvm_vector_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            Some(0)
        );
        let forward = memory.view(128, 12, 0).unwrap();
        assert!((forward.get_f32(0).unwrap() - 0.0).abs() < 1e-5);
        assert!((forward.get_f32(1 * 4).unwrap() - 1.0).abs() < 1e-5);
        let up = memory.view(192, 12, 0).unwrap();
        assert!((up.get_f32(8).unwrap() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn perpendicular_rejects_zero_vectors() {
        let (memory, words) = fixture(109, &[64, 128]);
        let source = memory.view(128, 12, 0).unwrap();
        source.set_f32(0, 0.0).unwrap();
        source.set_f32(4, 1.0).unwrap();
        source.set_f32(8, 0.0).unwrap();
        assert_eq!(
            qvm_vector_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(),
            Some(0)
        );
        let zero = memory.view(128, 12, 0).unwrap();
        zero.set_f32(4, 0.0).unwrap();
        assert!(qvm_vector_syscall(QvmSyscallRole::Game, &words, &memory).is_err());
    }

    #[test]
    fn other_roles_and_traps_pass_through() {
        let (memory, words) = fixture(107, &[64, 128, 192]);
        assert_eq!(
            qvm_vector_syscall(QvmSyscallRole::Cgame, &words, &memory).unwrap(),
            None
        );
        let (memory, words) = fixture(42, &[64, 128, 192]);
        assert_eq!(qvm_vector_syscall(QvmSyscallRole::Game, &words, &memory).unwrap(), None);
    }
}
