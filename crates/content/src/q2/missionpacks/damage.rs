//! Q2 mission-pack damage causes (`src/content/q2/missionpacks/damage.ts`).

use crate::q2::support::contracts::{Q2NativeCause, Q2NativeGame};

/// Friendly-fire bit.
const FRIENDLY_FIRE: i32 = 0x8000000;

/// Canonical causes (`q2CanonicalCause`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2CanonicalCause {
    /// Grapple.
    pub grapple: i32,
    /// Telefrag spawn.
    pub telefrag_spawn: i32,
    /// Blue blaster.
    pub blue_blaster: i32,
}

/// Canonical causes.
pub const Q2_CANONICAL_CAUSE: Q2CanonicalCause = Q2CanonicalCause {
    grapple: 56,
    telefrag_spawn: 57,
    blue_blaster: 58,
};

/// Native cause profile (`Q2NativeCauseProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2NativeCauseProfile {
    /// Classic profile.
    Classic {
        /// Source game.
        game: Q2NativeGame,
    },
    /// Rerelease profile.
    Rerelease {
        /// No point loss.
        no_point_loss: bool,
    },
}

/// Whether a classic cause id is valid (`validClassic`).
fn valid_classic(game: Q2NativeGame, id: i32) -> bool {
    id <= 33
        || game == Q2NativeGame::Xatrix && id <= 39
        || game == Q2NativeGame::Rogue && (40..=55).contains(&id)
        || game == Q2NativeGame::Ctf && id == 34
}

/// Convert a native cause to canonical (`canonicalCauseFromNative`).
pub fn canonical_cause_from_native(native: &Q2NativeCause) -> Option<i32> {
    match *native {
        Q2NativeCause::Classic { game, value } => {
            if value < 0 || value > FRIENDLY_FIRE + 55 {
                return None;
            }
            let friendly = value & FRIENDLY_FIRE != 0;
            let id = value & !FRIENDLY_FIRE;
            if !valid_classic(game, id) {
                return None;
            }
            Some(
                (if game == Q2NativeGame::Ctf && id == 34 {
                    Q2_CANONICAL_CAUSE.grapple
                } else {
                    id
                }) + if friendly { FRIENDLY_FIRE } else { 0 },
            )
        }
        Q2NativeCause::Rerelease { id, friendly_fire, .. } => {
            if id < 0 || id > 58 {
                return None;
            }
            let mapped = if id < 22 {
                id
            } else if id == 22 {
                Q2_CANONICAL_CAUSE.telefrag_spawn
            } else if id <= 56 {
                id - 1
            } else if id == 57 {
                Q2_CANONICAL_CAUSE.grapple
            } else {
                Q2_CANONICAL_CAUSE.blue_blaster
            };
            Some(mapped + if friendly_fire { FRIENDLY_FIRE } else { 0 })
        }
    }
}

/// Convert a canonical cause to native (`nativeCauseFromCanonical`).
pub fn native_cause_from_canonical(
    profile: &Q2NativeCauseProfile,
    canonical: i32,
) -> Option<Q2NativeCause> {
    if canonical < 0 || canonical > FRIENDLY_FIRE + 58 {
        return None;
    }
    let friendly = canonical & FRIENDLY_FIRE != 0;
    let id = canonical & !FRIENDLY_FIRE;
    match *profile {
        Q2NativeCauseProfile::Classic { game } => {
            let raw = if game == Q2NativeGame::Ctf && id == Q2_CANONICAL_CAUSE.grapple {
                34
            } else {
                id
            };
            if game == Q2NativeGame::Ctf && id == 34 || !valid_classic(game, raw) {
                return None;
            }
            Some(Q2NativeCause::Classic {
                game,
                value: raw + if friendly { FRIENDLY_FIRE } else { 0 },
            })
        }
        Q2NativeCauseProfile::Rerelease { no_point_loss } => {
            if id > 58 {
                return None;
            }
            let raw = if id < 22 {
                id
            } else if id <= 55 {
                id + 1
            } else if id == Q2_CANONICAL_CAUSE.grapple {
                57
            } else if id == Q2_CANONICAL_CAUSE.telefrag_spawn {
                22
            } else {
                58
            };
            Some(Q2NativeCause::Rerelease {
                id: raw,
                friendly_fire: friendly,
                no_point_loss,
            })
        }
    }
}
