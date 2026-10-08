//! Shared movement boundary. Physics enters through THE-891.
use qa_core::primitives::{MovementRules, UserCmd};

/// Apply the selected movement policy, never the map or client protocol.
/// Keep the absolute server time: Q3's outer Pmove subdivides its interval.
pub fn prepare_command(rules: MovementRules, mut command: UserCmd) -> UserCmd {
    command.duration_ms = match rules {
        // qsrc WinQuake host.c Host_FilterTime; milliseconds at this boundary.
        MovementRules::Quake => command.duration_ms.clamp(1, 100),
        // QW/Q2 CL_FinishMove. QW SV_RunCmd further bisects steps above 50 ms.
        MovementRules::QuakeWorld | MovementRules::Quake2 => {
            if command.duration_ms > 250 {
                100
            } else {
                command.duration_ms
            }
        }
        // Rerelease game.h usercmd_t has a byte msec. Client policy is KEX-owned.
        MovementRules::Quake2Rerelease => command.duration_ms.min(255),
        // qsrc bg_pmove.c PmoveSingle. Outer Pmove has its own 66/fixed steps.
        MovementRules::Quake3 => command.duration_ms.clamp(1, 200),
    };
    command
}
