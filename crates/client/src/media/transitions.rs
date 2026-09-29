//! Cinematic transitions between servers and maps.
//!
//! Donor provenance: `src/media/transitions.ts`
//! (`cinematicTransition`).

use qa_core::identity::SeatId;

use super::types::CinematicEndReason;

/// A cinematic transition (`CinematicTransition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CinematicTransition {
    /// Q2 next-server cinematic.
    Q2NextServer {
        /// Target.
        target: String,
    },
    /// Q3 next-map cinematic.
    Q3NextMap {
        /// Target.
        target: String,
    },
    /// Return to the previous map.
    Return,
}

/// A transition host (`CinematicTransitionHost`).
pub trait CinematicTransitionHost {
    /// Send a client command.
    fn send_client_command(&mut self, seat: &SeatId, command: &str);
    /// Start a cinematic; returns false when it cannot start.
    fn start_cinematic(&mut self, seat: &SeatId, target: &str) -> bool;
}

/// A one-shot transition completion (`complete` closure).
pub struct TransitionCompletion<'a> {
    seat: SeatId,
    host: &'a mut dyn CinematicTransitionHost,
    done: bool,
}

impl<'a> TransitionCompletion<'a> {
    /// Complete the transition (later calls are no-ops).
    pub fn complete(&mut self, reason: CinematicEndReason) {
        if self.done {
            return;
        }
        self.done = true;
        if reason == CinematicEndReason::Stopped {
            self.host.send_client_command(&self.seat, "disconnect\n");
        }
    }
}

/// Run a transition (`cinematicTransition`).
pub fn cinematic_transition<'a>(
    seat: &SeatId,
    transition: &CinematicTransition,
    host: &'a mut dyn CinematicTransitionHost,
) -> TransitionCompletion<'a> {
    match transition {
        CinematicTransition::Q2NextServer { target } | CinematicTransition::Q3NextMap { target } => {
            host.send_client_command(seat, &format!("cinematic {target}\n"));
            if !host.start_cinematic(seat, target) {
                host.send_client_command(seat, "disconnect\n");
            }
        }
        CinematicTransition::Return => {
            host.send_client_command(seat, "disconnect\n");
        }
    }
    TransitionCompletion {
        seat: seat.clone(),
        host,
        done: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FixedHost {
        commands: Vec<String>,
        start: bool,
    }

    impl CinematicTransitionHost for FixedHost {
        fn send_client_command(&mut self, _seat: &SeatId, command: &str) {
            self.commands.push(command.to_string());
        }

        fn start_cinematic(&mut self, _seat: &SeatId, _target: &str) -> bool {
            self.start
        }
    }

    #[test]
    fn failed_start_disconnects() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = FixedHost {
            commands: Vec::new(),
            start: false,
        };
        let seat = owner.seat(0);
        let mut completion = cinematic_transition(
            &seat,
            &CinematicTransition::Q3NextMap {
                target: "intro".to_string(),
            },
            &mut host,
        );
        completion.complete(CinematicEndReason::Finished);
        completion.complete(CinematicEndReason::Stopped);
        drop(completion);
        assert_eq!(host.commands, vec!["cinematic intro\n", "disconnect\n"]);
    }

    #[test]
    fn return_disconnects() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = FixedHost {
            commands: Vec::new(),
            start: true,
        };
        let seat = owner.seat(0);
        let _completion = cinematic_transition(&seat, &CinematicTransition::Return, &mut host);
        assert_eq!(host.commands, vec!["disconnect\n"]);
    }
}
