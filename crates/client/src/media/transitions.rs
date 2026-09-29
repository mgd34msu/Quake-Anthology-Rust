//! Cinematic transitions between servers and maps.
//!
//! Donor provenance: `src/media/transitions.ts`
//! (`cinematicTransition`).

use qa_core::identity::SeatId;

use super::types::CinematicEndReason;

/// A cinematic transition (`CinematicTransition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CinematicTransition {
    /// Q2 next-server transition.
    Q2NextServer {
        /// Server count.
        server_count: i32,
    },
    /// Q3 next-map transition.
    Q3NextMap {
        /// Command.
        command: String,
    },
    /// Return without a follow-up command.
    Return,
}

/// A transition host (`CinematicTransitionHost`).
pub trait CinematicTransitionHost {
    /// Send a client command.
    fn send_client_command(&mut self, seat: &SeatId, command: &str);
    /// Append a command.
    fn append_command(&mut self, seat: &SeatId, command: &str);
    /// Leave the cinematic.
    fn leave_cinematic(&mut self, seat: &SeatId);
}

/// Snapshot the transition at movie admission; an unrelated seat
/// cannot consume it (`cinematicTransition`).
pub fn cinematic_transition<'a>(
    seat: &SeatId,
    transition: &CinematicTransition,
    host: &'a mut dyn CinematicTransitionHost,
) -> TransitionCompletion<'a> {
    TransitionCompletion {
        seat: seat.clone(),
        transition: transition.clone(),
        host,
        completed: false,
    }
}

/// A one-shot transition completion.
pub struct TransitionCompletion<'a> {
    seat: SeatId,
    transition: CinematicTransition,
    host: &'a mut dyn CinematicTransitionHost,
    completed: bool,
}

impl TransitionCompletion<'_> {
    /// Complete the transition (later calls are no-ops).
    pub fn complete(&mut self, reason: CinematicEndReason) {
        if self.completed {
            return;
        }
        self.completed = true;
        self.host.leave_cinematic(&self.seat);
        if reason == CinematicEndReason::Stopped {
            return;
        }
        match &self.transition {
            CinematicTransition::Q2NextServer { server_count } => {
                self.host
                    .send_client_command(&self.seat, &format!("nextserver {server_count}\n"));
            }
            CinematicTransition::Q3NextMap { command } => {
                if !command.is_empty() {
                    self.host.append_command(&self.seat, &format!("{command}\n"));
                }
            }
            CinematicTransition::Return => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FixedHost {
        client: Vec<String>,
        appended: Vec<String>,
        left: usize,
    }

    impl CinematicTransitionHost for FixedHost {
        fn send_client_command(&mut self, _seat: &SeatId, command: &str) {
            self.client.push(command.to_string());
        }

        fn append_command(&mut self, _seat: &SeatId, command: &str) {
            self.appended.push(command.to_string());
        }

        fn leave_cinematic(&mut self, _seat: &SeatId) {
            self.left += 1;
        }
    }

    fn new_host() -> FixedHost {
        FixedHost {
            client: Vec::new(),
            appended: Vec::new(),
            left: 0,
        }
    }

    #[test]
    fn admission_has_no_side_effects() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = new_host();
        let seat = owner.seat(0);
        let _completion =
            cinematic_transition(&seat, &CinematicTransition::Q2NextServer { server_count: 1 }, &mut host);
        assert!(host.client.is_empty());
        assert_eq!(host.left, 0);
    }

    #[test]
    fn q2_sends_nextserver_once() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = new_host();
        let seat = owner.seat(0);
        let mut completion =
            cinematic_transition(&seat, &CinematicTransition::Q2NextServer { server_count: 3 }, &mut host);
        completion.complete(CinematicEndReason::Finished);
        completion.complete(CinematicEndReason::Finished);
        drop(completion);
        assert_eq!(host.client, vec!["nextserver 3\n"]);
        assert_eq!(host.left, 1);
    }

    #[test]
    fn stopped_leaves_without_commands() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = new_host();
        let seat = owner.seat(0);
        let mut completion = cinematic_transition(
            &seat,
            &CinematicTransition::Q3NextMap {
                command: "map intro".to_string(),
            },
            &mut host,
        );
        completion.complete(CinematicEndReason::Stopped);
        drop(completion);
        assert!(host.appended.is_empty());
        assert_eq!(host.left, 1);
    }

    #[test]
    fn q3_appends_and_return_leaves() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut host = new_host();
        let seat = owner.seat(0);
        let mut completion = cinematic_transition(
            &seat,
            &CinematicTransition::Q3NextMap {
                command: "map intro".to_string(),
            },
            &mut host,
        );
        completion.complete(CinematicEndReason::Skipped);
        drop(completion);
        assert_eq!(host.appended, vec!["map intro\n"]);

        let mut host = new_host();
        let mut completion = cinematic_transition(
            &seat,
            &CinematicTransition::Q3NextMap { command: String::new() },
            &mut host,
        );
        completion.complete(CinematicEndReason::Finished);
        drop(completion);
        assert!(host.appended.is_empty());

        let mut host = new_host();
        let mut completion = cinematic_transition(&seat, &CinematicTransition::Return, &mut host);
        completion.complete(CinematicEndReason::Finished);
        drop(completion);
        assert!(host.client.is_empty());
        assert_eq!(host.left, 1);
    }
}
