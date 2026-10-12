//! One movement entry per player rule set, over shared geometry and player state.
mod physics;
mod recovery;
mod slide;
mod special;
pub use physics::{MoveOptions, MovementResult, TraceServices, entry, pmove, set_bounds};
use qa_core::{
    primitives::{RuleSetId, UserCmd},
    sys_events::EventTime,
};

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
            EventTime::from_milliseconds(u64::from(command.duration_ms)).0
        } else {
            command.duration_ns.clamp(
                EventTime::from_milliseconds(1).0,
                EventTime::from_milliseconds(100).0,
            )
        }
    } else {
        EventTime::from_milliseconds(u64::from(command.duration_ms)).0
    };
    command
}

/// QuakeWorld's native command leaves, in SV_RunCmd/CL_PredictUsercmd order.
/// Both halves discard an odd millisecond. Impulse follows SV_RunCmd: first leaf only.
/// The absolute server time and all other command fields remain unchanged.
pub struct CommandSteps {
    command: UserCmd,
    // A u16 can halve at most 16 times, even before native duration admission.
    durations: [u16; u16::BITS as usize],
    pending: usize,
}

/// Apply QuakeWorld's native duration admission once, then yield <=50 ms leaves.
/// A zero-duration command remains one leaf, as in the original server.
pub fn quakeworld_steps(command: UserCmd) -> CommandSteps {
    let command = prepare_command(RuleSetId::QuakeWorld, command);
    let mut durations = [0; u16::BITS as usize];
    durations[0] = command.duration_ms;
    CommandSteps {
        command,
        durations,
        pending: 1,
    }
}

impl Iterator for CommandSteps {
    type Item = UserCmd;

    fn next(&mut self) -> Option<Self::Item> {
        while self.pending > 0 {
            self.pending -= 1;
            let ms = self.durations[self.pending];
            if ms > 50 {
                self.durations[self.pending] = ms / 2;
                self.durations[self.pending + 1] = ms / 2;
                self.pending += 2;
            } else {
                let command = UserCmd {
                    duration_ms: ms,
                    duration_ns: EventTime::from_milliseconds(u64::from(ms)).0,
                    ..self.command
                };
                self.command.impulse = 0;
                return Some(command);
            }
        }
        None
    }
}

impl core::iter::FusedIterator for CommandSteps {}
