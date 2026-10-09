//! One movement entry per player rule set, over shared geometry and player state.
mod physics;
mod slide;
pub use physics::{MovementResult, TraceServices, entry, pmove, set_bounds};
use qa_core::primitives::{RuleSetId, UserCmd};

/// Apply the selected movement policy, never the map or client protocol.
/// Keep the absolute server time: Q3's outer Pmove subdivides its interval.
pub fn prepare_command(rules: RuleSetId, mut command: UserCmd) -> UserCmd {
    command.duration_ms = match rules {
        // qsrc WinQuake host.c Host_FilterTime; milliseconds at this boundary.
        RuleSetId::Quake => command.duration_ms.clamp(1, 100),
        // QW/Q2 CL_FinishMove. QW SV_RunCmd further bisects steps above 50 ms.
        RuleSetId::QuakeWorld | RuleSetId::Quake2 => {
            if command.duration_ms > 250 {
                100
            } else {
                command.duration_ms
            }
        }
        // Rerelease game.h usercmd_t has a byte msec. Client policy is KEX-owned.
        RuleSetId::Quake2Rerelease => command.duration_ms.min(255),
        // qsrc bg_pmove.c PmoveSingle. Outer Pmove has its own 66/fixed steps.
        RuleSetId::Quake3 => command.duration_ms.clamp(1, 200),
    };
    command.duration_ns = if rules == RuleSetId::Quake {
        if command.duration_ns == 0 {
            u64::from(command.duration_ms) * 1_000_000
        } else {
            command.duration_ns.clamp(1_000_000, 100_000_000)
        }
    } else {
        u64::from(command.duration_ms) * 1_000_000
    };
    command
}
