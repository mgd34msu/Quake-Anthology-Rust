//! QVM client-game handle (`cl_cgame.c`, `cg_public.h`).
//!
//! Provenance: `src/compat/qvm/cgame.ts` (Quake III Arena, GPL-2.0-or-later).
//!
//! Local mirrors: `src/contracts/identity.ts` ([`SeatId]),
//! `src/contracts/ui.ts` ([`StereoView`], [`QvmCgameEventHandling`]).
//! Async export variants mirror the donor surface; the synchronous mirror
//! backing means they never suspend.

use std::cell::RefCell;
use std::rc::Rc;

use super::game_data::{QvmArtifact, QvmCgameExport, QvmModule, QvmRole};
use crate::error::GuestError;

/// Client seat id (mirror of `SeatId`).
pub type SeatId = u32;

/// Stereo view selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StereoView {
    /// Center view.
    Center,
    /// Left view.
    Left,
    /// Right view.
    Right,
}

/// Cgame event-handling mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QvmCgameEventHandling {
    /// No handling.
    None,
    /// Team menu.
    TeamMenu,
    /// Scoreboard.
    Scoreboard,
    /// Edit HUD.
    EditHud,
}

/// Client-game API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCgameApi {
    /// API version: 4 for modern, 3 for legacy.
    pub version: i32,
}

impl QvmCgameApi {
    /// API kind tag.
    #[must_use]
    pub const fn kind() -> &'static str {
        "q3-cgame"
    }
}

/// Engine gamestate observed by the lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCgameState {
    /// Engine generation.
    pub generation: i32,
    /// Active server message number.
    pub server_message_number: i32,
    /// Drop reason, if the connection dropped.
    pub dropped: Option<String>,
}

/// Cgame lifetime callbacks (mirror of `QvmCgameLifetime`).
pub trait QvmCgameLifetime {
    /// Fail when not on the current engine operation.
    fn assert_current_operation(&self) -> Result<(), GuestError>;
    /// Read the current engine gamestate.
    fn current(&self) -> QvmCgameState;
    /// Begin the loading transition.
    fn begin_loading(&self);
    /// Prime a generation after `CG_Init`.
    fn prime(&self, generation: i32);
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum QvmCgamePhase {
    Created,
    Retired,
    Initializing { generation: i32 },
    Initialized { generation: i32 },
}

/// Client-game module with lifecycle checks.
pub struct QvmCgame {
    /// Owning seat.
    pub seat: SeatId,
    /// Guest module.
    pub module: QvmModule,
    /// API identity.
    pub api: QvmCgameApi,
    lifetime: Rc<dyn QvmCgameLifetime>,
    phase: RefCell<QvmCgamePhase>,
}

impl std::fmt::Debug for QvmCgame {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QvmCgame")
            .field("seat", &self.seat)
            .field("api", &self.api)
            .field("phase", &self.phase.borrow())
            .finish()
    }
}

impl QvmCgame {
    /// Build a cgame over a cgame artifact.
    pub fn new(seat: SeatId, artifact: QvmArtifact, lifetime: Rc<dyn QvmCgameLifetime>) -> Result<Self, GuestError> {
        if artifact.role != QvmRole::Cgame {
            return Err(GuestError::invalid("QvmCgame requires a cgame artifact"));
        }
        let module = QvmModule::new(artifact, None, None)?;
        let api = QvmCgameApi {
            version: if module.abi_profile().is_modern() { 4 } else { 3 },
        };
        Ok(Self {
            seat,
            module,
            api,
            lifetime,
            phase: RefCell::new(QvmCgamePhase::Created),
        })
    }

    /// Whether input-event exports are supported.
    #[must_use]
    pub fn supports_input_events(&self) -> bool {
        self.api.version >= 4
    }

    fn current(&self, command: i32) -> Result<(), GuestError> {
        self.lifetime.assert_current_operation()?;
        let phase = self.phase.borrow().clone();
        if phase == QvmCgamePhase::Retired {
            return Err(GuestError::invalid("Cgame module has been retired"));
        }
        if command == QvmCgameExport::CG_SHUTDOWN || command == QvmCgameExport::CG_CONSOLE_COMMAND {
            return Ok(());
        }
        if phase == QvmCgamePhase::Created {
            return Err(GuestError::invalid("Cgame module has not initialized"));
        }
        let state = self.lifetime.current();
        if let Some(dropped) = state.dropped {
            return Err(GuestError::invalid(dropped));
        }
        let generation = match phase {
            QvmCgamePhase::Initializing { generation } | QvmCgamePhase::Initialized { generation } => generation,
            _ => 0,
        };
        if state.generation != generation {
            return Err(GuestError::invalid("Cgame module belongs to a stale engine gamestate"));
        }
        Ok(())
    }

    async fn call(&self, command: i32, values: &[i32]) -> Result<i32, GuestError> {
        self.current(command)?;
        if !self.module.abi_profile().is_modern() && command > QvmCgameExport::CG_LAST_ATTACKER {
            return Err(GuestError::invalid(format!("Legacy cgame does not export {command}")));
        }
        let mut words = vec![command];
        words.extend_from_slice(values);
        let result = self.module.call(&words, 0)?;
        self.current(command)?;
        Ok(result)
    }

    /// Initialize the cgame against the active engine message.
    pub async fn init(
        &self,
        server_message_number: i32,
        server_command_sequence: i32,
        client_number: i32,
    ) -> Result<(), GuestError> {
        self.lifetime.assert_current_operation()?;
        {
            let phase = self.phase.borrow();
            if *phase == QvmCgamePhase::Retired || matches!(*phase, QvmCgamePhase::Initializing { .. }) {
                return Err(GuestError::invalid("Cgame cannot initialize in its current lifecycle"));
            }
        }
        let state = self.lifetime.current();
        if let Some(dropped) = state.dropped {
            return Err(GuestError::invalid(dropped));
        }
        if state.generation == 0 {
            return Err(GuestError::invalid("CG_Init requires a live engine gamestate"));
        }
        if state.server_message_number != server_message_number {
            return Err(GuestError::invalid(
                "CG_Init message differs from the active engine message",
            ));
        }
        *self.phase.borrow_mut() = QvmCgamePhase::Initializing {
            generation: state.generation,
        };
        self.lifetime.begin_loading();
        let outcome = self
            .call(
                QvmCgameExport::CG_INIT,
                &[server_message_number, server_command_sequence, client_number],
            )
            .await;
        match outcome {
            Ok(_) => {
                if self.lifetime.current().server_message_number != server_message_number {
                    self.retire();
                    return Err(GuestError::invalid(
                        "Engine server message parsing must serialize behind CG_Init",
                    ));
                }
                self.lifetime.prime(state.generation);
                *self.phase.borrow_mut() = QvmCgamePhase::Initialized {
                    generation: state.generation,
                };
                Ok(())
            }
            Err(error) => {
                self.retire();
                Err(error)
            }
        }
    }

    /// Shut the cgame down.
    pub async fn shutdown(&self) -> Result<(), GuestError> {
        self.call(QvmCgameExport::CG_SHUTDOWN, &[]).await?;
        Ok(())
    }

    /// Retire the module.
    pub fn retire(&self) {
        *self.phase.borrow_mut() = QvmCgamePhase::Retired;
        self.module.retire();
    }

    /// Run a console command; returns whether the cgame handled it.
    pub async fn console_command(&self, arguments: &[String]) -> Result<bool, GuestError> {
        self.current(QvmCgameExport::CG_CONSOLE_COMMAND)?;
        let handled = self.module.command(&[QvmCgameExport::CG_CONSOLE_COMMAND], arguments)? != 0;
        self.current(QvmCgameExport::CG_CONSOLE_COMMAND)?;
        Ok(handled)
    }

    /// Draw the active frame.
    pub async fn draw_active_frame(
        &self,
        time: i32,
        stereo: StereoView,
        demo_playback: bool,
    ) -> Result<(), GuestError> {
        let view = match stereo {
            StereoView::Center => 0,
            StereoView::Left => 1,
            StereoView::Right => 2,
        };
        self.call(
            QvmCgameExport::CG_DRAW_ACTIVE_FRAME,
            &[time, view, i32::from(demo_playback)],
        )
        .await?;
        Ok(())
    }

    /// Read the crosshair player, if any.
    pub async fn crosshair_player(&self) -> Result<Option<i32>, GuestError> {
        let value = self.call(QvmCgameExport::CG_CROSSHAIR_PLAYER, &[]).await?;
        Ok(if value < 0 { None } else { Some(value) })
    }

    /// Read the last attacker, if any.
    pub async fn last_attacker(&self) -> Result<Option<i32>, GuestError> {
        let value = self.call(QvmCgameExport::CG_LAST_ATTACKER, &[]).await?;
        Ok(if value < 0 { None } else { Some(value) })
    }

    /// Deliver a key event.
    pub async fn key_event(&self, key: i32, down: bool) -> Result<(), GuestError> {
        self.call(QvmCgameExport::CG_KEY_EVENT, &[key, i32::from(down)]).await?;
        Ok(())
    }

    /// Deliver a mouse event.
    pub async fn mouse_event(&self, dx: i32, dy: i32) -> Result<(), GuestError> {
        self.call(QvmCgameExport::CG_MOUSE_EVENT, &[dx, dy]).await?;
        Ok(())
    }

    /// Set event-handling mode.
    pub async fn event_handling(&self, mode: QvmCgameEventHandling) -> Result<(), GuestError> {
        let value = match mode {
            QvmCgameEventHandling::None => 0,
            QvmCgameEventHandling::TeamMenu => 1,
            QvmCgameEventHandling::Scoreboard => 2,
            QvmCgameEventHandling::EditHud => 3,
        };
        self.call(QvmCgameExport::CG_EVENT_HANDLING, &[value]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::game_data::{AbiProfile, QvmImage};
    use super::*;

    struct FixtureLifetime {
        state: RefCell<QvmCgameState>,
        primed: RefCell<Vec<i32>>,
    }

    impl QvmCgameLifetime for FixtureLifetime {
        fn assert_current_operation(&self) -> Result<(), GuestError> {
            Ok(())
        }

        fn current(&self) -> QvmCgameState {
            self.state.borrow().clone()
        }

        fn begin_loading(&self) {}

        fn prime(&self, generation: i32) {
            self.primed.borrow_mut().push(generation);
        }
    }

    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn raw() -> RawWaker {
            unsafe fn clone(_: *const ()) -> RawWaker {
                raw()
            }
            unsafe fn noop(_: *const ()) {}
            RawWaker::new(std::ptr::null(), &RawWakerVTable::new(clone, noop, noop, noop))
        }
        let waker = unsafe { Waker::from_raw(raw()) };
        let mut context = Context::from_waker(&waker);
        let mut future = Box::pin(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn cgame(generation: i32, message: i32) -> (QvmCgame, Rc<FixtureLifetime>) {
        let mut image = QvmImage::default();
        image.allocated_data_length = 4096;
        let artifact = QvmArtifact {
            module: super::super::game_data::ModuleIdentity {
                id: "q3:cgame".to_string(),
                artifact_path: "cgame.qvm".to_string(),
                digest: "d".to_string(),
                revision: "r".to_string(),
            },
            role: QvmRole::Cgame,
            abi_profile: None,
            image,
        };
        let lifetime = Rc::new(FixtureLifetime {
            state: RefCell::new(QvmCgameState {
                generation,
                server_message_number: message,
                dropped: None,
            }),
            primed: RefCell::new(Vec::new()),
        });
        let game = QvmCgame::new(0, artifact, lifetime.clone()).unwrap();
        (game, lifetime)
    }

    #[test]
    fn role_and_api_selected_by_abi() {
        let (game, _) = cgame(1, 10);
        assert_eq!(game.api.version, 4);
        assert!(game.supports_input_events());
        let mut image = QvmImage::default();
        image.allocated_data_length = 64;
        let artifact = QvmArtifact {
            module: super::super::game_data::ModuleIdentity::default(),
            role: QvmRole::Qagame,
            abi_profile: Some(AbiProfile::Legacy),
            image,
        };
        let lifetime = Rc::new(FixtureLifetime {
            state: RefCell::new(QvmCgameState {
                generation: 1,
                server_message_number: 1,
                dropped: None,
            }),
            primed: RefCell::new(Vec::new()),
        });
        assert!(QvmCgame::new(0, artifact, lifetime).is_err());
    }

    #[test]
    fn init_primes_and_locks_generation() {
        let (game, lifetime) = cgame(2, 10);
        block_on(game.init(10, 4, 0)).unwrap();
        assert_eq!(lifetime.primed.borrow().as_slice(), &[2]);
        assert_eq!(game.module.calls()[0].words[0], QvmCgameExport::CG_INIT);
        lifetime.state.borrow_mut().generation = 3;
        assert!(block_on(game.draw_active_frame(10, StereoView::Center, false)).is_err());
        lifetime.state.borrow_mut().generation = 2;
        block_on(game.draw_active_frame(10, StereoView::Left, true)).unwrap();
        let calls = game.module.calls();
        assert_eq!(calls[1].words[1..4], [10, 1, 1]);
    }

    #[test]
    fn init_rejects_message_mismatch_and_retires_on_failure() {
        let (game, lifetime) = cgame(1, 10);
        assert!(block_on(game.init(11, 4, 0)).is_err());
        lifetime.state.borrow_mut().generation = 0;
        assert!(block_on(game.init(10, 4, 0)).is_err());
        lifetime.state.borrow_mut().generation = 1;
        lifetime.state.borrow_mut().dropped = Some("dropped".to_string());
        assert!(block_on(game.init(10, 4, 0)).is_err());
    }

    #[test]
    fn negative_queries_read_as_missing() {
        let (game, _) = cgame(1, 10);
        block_on(game.init(10, 4, 0)).unwrap();
        game.module.set_default_return(-1);
        assert_eq!(block_on(game.crosshair_player()).unwrap(), None);
        assert_eq!(block_on(game.last_attacker()).unwrap(), None);
        game.module.set_default_return(3);
        assert_eq!(block_on(game.crosshair_player()).unwrap(), Some(3));
    }

    #[test]
    fn event_modes_encode_in_order() {
        let (game, _) = cgame(1, 10);
        block_on(game.init(10, 4, 0)).unwrap();
        for (mode, code) in [
            (QvmCgameEventHandling::None, 0),
            (QvmCgameEventHandling::TeamMenu, 1),
            (QvmCgameEventHandling::Scoreboard, 2),
            (QvmCgameEventHandling::EditHud, 3),
        ] {
            block_on(game.event_handling(mode)).unwrap();
            let calls = game.module.calls();
            assert_eq!(calls[calls.len() - 1].words[1], code);
        }
        game.retire();
        assert!(block_on(game.shutdown()).is_err());
    }
}
