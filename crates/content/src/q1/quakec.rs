//! Quake 1 QuakeC-binding content (`src/content/q1/quakec/*`).
//!
//! Donor provenance: `armor-points.ts`, `armor-stage.ts`, `damage-call.ts`,
//! `damage-scale.ts`, `id1-attacks.ts`, `id1-damage.ts`,
//! `id1-environment.ts`, `id1-pickups.ts`, `id1-program.ts`,
//! `id1-projectiles.ts`, `pickup-callers.ts`, `pickup-stage.ts`,
//! `weapon-stage-declaration.ts`, `weapon-stage.ts`.
//!
//! The `compat/qc` machine surface this content drives is captured in
//! [`qc_view`]; the `compat` lane implements those traits for the real VM.
//! Shared gameplay/world structural types live in [`qc_gameplay`] in
//! isolation (it imports nothing from its siblings) so the file can be
//! deleted once `crate::q1::foundation::gameplay` lands.

pub mod armor_points;
pub mod armor_stage;
pub mod damage_call;
pub mod damage_scale;
pub mod id1_attacks;
pub mod id1_damage;
pub mod id1_environment;
pub mod id1_pickups;
pub mod id1_program;
pub mod id1_projectiles;
pub mod pickup_callers;
pub mod pickup_stage;
pub mod qc_gameplay;
pub mod qc_view;
pub mod weapon_stage;
pub mod weapon_stage_declaration;

use thiserror::Error;

use crate::mods::ModsError;
use crate::value::ValueError;

/// QuakeC-binding failure (donor `QcProgramError`, plus reader input).
///
/// A parent-provided `Q1Error` may replace this type at merge; every
/// function in this module returns it.
#[derive(Debug, Error)]
pub enum QcError {
    /// Malformed declaration value.
    #[error(transparent)]
    Value(#[from] ValueError),
    /// Malformed mod declaration value.
    #[error(transparent)]
    Mods(#[from] ModsError),
    /// Rejected or mismatched QuakeC program (`QcProgramError`).
    #[error("{artifact}: {message}")]
    Program {
        /// Failure detail.
        message: String,
        /// Artifact label (donor `source`, default `progs.dat`).
        artifact: String,
    },
    /// Cancelled source execution (donor `QcFunctionCancellation`).
    ///
    /// The donor `cancel` diverges; here the caller builds this error via
    /// [`QcError::cancelled`] and returns it immediately. The VM matches
    /// `owner` against the live boundary invocation.
    #[error("cancelled source execution {owner}: [{0}, {1}, {2}]", words[0], words[1], words[2])]
    Cancelled {
        /// Owning boundary invocation.
        owner: u64,
        /// Replacement return words.
        words: [i32; 3],
    },
}

impl QcError {
    /// Build a program failure for an artifact source.
    pub fn program(message: impl Into<String>, source: &str) -> Self {
        Self::Program {
            message: message.into(),
            artifact: source.to_string(),
        }
    }

    /// Build the cancellation error for a live execution owner.
    #[must_use]
    pub fn cancelled(owner: u64, words: [i32; 3]) -> Self {
        Self::Cancelled { owner, words }
    }
}

/// Round to binary32 storage (donor `Math.fround`).
#[must_use]
pub(crate) fn fround(value: f64) -> f64 {
    f64::from(qa_core::numeric::store_f32(value))
}

/// Whether two guest floats are identical (donor `Object.is`).
#[must_use]
pub(crate) fn float_identical(left: f64, right: f64) -> bool {
    left == right || (left.is_nan() && right.is_nan())
}

/// Pushes a frame that pops on drop unless defused (donor
/// `try/finally` pops, including on panic unwind).
///
/// Frames form a strict stack: nested pushes pop before their outer
/// guard defuses. RefCell borrows must never span guest or authority
/// calls, so the drop-time pop cannot observe an outstanding borrow.
pub(crate) struct FrameGuard<'s, T> {
    stack: &'s std::cell::RefCell<Vec<T>>,
    armed: bool,
}

impl<'s, T> FrameGuard<'s, T> {
    /// Push a frame, popping it when the guard drops.
    pub(crate) fn push(stack: &'s std::cell::RefCell<Vec<T>>, frame: T) -> Self {
        stack.borrow_mut().push(frame);
        Self { stack, armed: true }
    }

    /// Pop the frame, disarming the guard.
    pub(crate) fn defuse(mut self) {
        self.armed = false;
        self.stack.borrow_mut().pop();
    }
}

impl<T> Drop for FrameGuard<'_, T> {
    fn drop(&mut self) {
        if self.armed {
            self.stack.borrow_mut().pop();
        }
    }
}
