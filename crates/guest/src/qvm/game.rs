//! QVM server-game handle (`sv_game.c`, `sv_client.c`, `g_public.h`).
//!
//! Provenance: `src/compat/qvm/game.ts` (Quake III Arena, GPL-2.0-or-later).
//!
//! [`QvmGame`] owns its [`QvmModule`] and shares located [`QvmGameData`]
//! with the `G_LOCATE_GAME_DATA` trap. Async variants mirror the donor
//! `*Async` surface; the synchronous mirror backing means they never
//! suspend.

use super::game_data::{
    AbiProfile, CallKind, QvmArtifact, QvmGameData, QvmGameExport, QvmGameImport, QvmHostCall, QvmHostFn, QvmModule,
    QvmRole,
};
use crate::error::GuestError;

/// Server-game API identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmGameApi {
    /// API version: 8 for modern, 7 for legacy.
    pub version: i32,
}

impl QvmGameApi {
    /// API kind tag.
    #[must_use]
    pub const fn kind() -> &'static str {
        "q3-qagame"
    }
}

/// Server-game module with located game data.
#[derive(Debug, Clone)]
pub struct QvmGame {
    /// Guest module.
    pub module: QvmModule,
    /// Located game data (shared with the locate trap).
    pub data: QvmGameData,
}

impl QvmGame {
    /// Build a game over a qagame artifact, chaining `host` behind the
    /// locate-data trap.
    pub fn new(artifact: QvmArtifact, host: QvmHostFn) -> Result<Self, GuestError> {
        if artifact.role != QvmRole::Qagame {
            return Err(GuestError::invalid("QvmGame requires a qagame artifact"));
        }
        let abi = artifact.abi_profile.unwrap_or(AbiProfile::Modern);
        let module = QvmModule::new(artifact, None, None)?;
        let data = QvmGameData::new(module.memory(), abi);
        let located = data.clone();
        let chained: QvmHostFn = std::rc::Rc::new(move |call: &QvmHostCall| {
            if call.kind == CallKind::Engine
                && call.role == QvmRole::Qagame
                && call.code == QvmGameImport::G_LOCATE_GAME_DATA
            {
                let words = (call.int(1)?, call.int(2)?, call.int(3)?, call.int(4)?, call.int(5)?);
                let (_, num_entities, entity_stride, _, client_stride) = words;
                let as_usize = |value: i32| {
                    usize::try_from(value).map_err(|_| GuestError::invalid("Game-data trap word is negative"))
                };
                located.locate(
                    words.0,
                    as_usize(num_entities)?,
                    as_usize(entity_stride)?,
                    words.3,
                    as_usize(client_stride)?,
                )?;
                return Ok(Some(0));
            }
            host(call)
        });
        module.set_host(Some(chained));
        Ok(Self { module, data })
    }

    /// API identity.
    #[must_use]
    pub fn api(&self) -> QvmGameApi {
        QvmGameApi {
            version: if self.module.abi_profile().is_modern() { 8 } else { 7 },
        }
    }

    /// Initialize the game.
    pub fn initialize(&self, level_time: i32, random_seed: i32, restart: bool) -> Result<(), GuestError> {
        self.module.call(
            &[QvmGameExport::GAME_INIT, level_time, random_seed, i32::from(restart)],
            0,
        )?;
        Ok(())
    }

    /// Shut the game down.
    pub fn shutdown(&self, restart: bool) -> Result<(), GuestError> {
        self.module
            .call(&[QvmGameExport::GAME_SHUTDOWN, i32::from(restart)], 0)?;
        Ok(())
    }

    /// Connect a client; returns the denial message, if any.
    pub fn client_connect(&self, client: i32, first_time: bool, is_bot: bool) -> Result<Option<String>, GuestError> {
        let denied = self.module.call(
            &[
                QvmGameExport::GAME_CLIENT_CONNECT,
                client,
                i32::from(first_time),
                i32::from(is_bot),
            ],
            0,
        )?;
        if denied == 0 {
            Ok(None)
        } else {
            self.module.memory().read_string(denied).map(Some)
        }
    }

    /// Begin a client.
    pub fn client_begin(&self, client: i32) -> Result<(), GuestError> {
        self.module.call(&[QvmGameExport::GAME_CLIENT_BEGIN, client], 0)?;
        Ok(())
    }

    /// Report a client userinfo change.
    pub fn client_userinfo_changed(&self, client: i32) -> Result<(), GuestError> {
        self.module
            .call(&[QvmGameExport::GAME_CLIENT_USERINFO_CHANGED, client], 0)?;
        Ok(())
    }

    /// Disconnect a client.
    pub fn client_disconnect(&self, client: i32) -> Result<(), GuestError> {
        self.module.call(&[QvmGameExport::GAME_CLIENT_DISCONNECT, client], 0)?;
        Ok(())
    }

    /// Deliver a client command.
    pub fn client_command(&self, client: i32, arguments: &[String]) -> Result<(), GuestError> {
        self.module
            .command(&[QvmGameExport::GAME_CLIENT_COMMAND, client], arguments)?;
        Ok(())
    }

    /// Run a client think.
    pub fn client_think(&self, client: i32) -> Result<(), GuestError> {
        self.module.call(&[QvmGameExport::GAME_CLIENT_THINK, client], 0)?;
        Ok(())
    }

    /// Run a server frame.
    pub fn run_frame(&self, time: i32) -> Result<(), GuestError> {
        self.module.call(&[QvmGameExport::GAME_RUN_FRAME, time], 0)?;
        Ok(())
    }

    /// Run a console command; returns whether the game handled it.
    pub fn console_command(&self, arguments: &[String]) -> Result<bool, GuestError> {
        Ok(self.module.command(&[QvmGameExport::GAME_CONSOLE_COMMAND], arguments)? != 0)
    }

    /// Run a bot frame.
    pub fn bot_frame(&self, time: i32) -> Result<(), GuestError> {
        self.module.call(&[QvmGameExport::BOTAI_START_FRAME, time], 0)?;
        Ok(())
    }

    /// Initialize the game.
    pub async fn initialize_async(&self, level_time: i32, random_seed: i32, restart: bool) -> Result<(), GuestError> {
        self.initialize(level_time, random_seed, restart)
    }

    /// Shut the game down.
    pub async fn shutdown_async(&self, restart: bool) -> Result<(), GuestError> {
        self.shutdown(restart)
    }

    /// Connect a client; returns the denial message, if any.
    pub async fn client_connect_async(
        &self,
        client: i32,
        first_time: bool,
        is_bot: bool,
    ) -> Result<Option<String>, GuestError> {
        self.client_connect(client, first_time, is_bot)
    }

    /// Begin a client.
    pub async fn client_begin_async(&self, client: i32) -> Result<(), GuestError> {
        self.client_begin(client)
    }

    /// Report a client userinfo change.
    pub async fn client_userinfo_changed_async(&self, client: i32) -> Result<(), GuestError> {
        self.client_userinfo_changed(client)
    }

    /// Disconnect a client.
    pub async fn client_disconnect_async(&self, client: i32) -> Result<(), GuestError> {
        self.client_disconnect(client)
    }

    /// Deliver a client command.
    pub async fn client_command_async(&self, client: i32, arguments: &[String]) -> Result<(), GuestError> {
        self.client_command(client, arguments)
    }

    /// Run a client think.
    pub async fn client_think_async(&self, client: i32) -> Result<(), GuestError> {
        self.client_think(client)
    }

    /// Run a server frame.
    pub async fn run_frame_async(&self, time: i32) -> Result<(), GuestError> {
        self.run_frame(time)
    }

    /// Run a console command; returns whether the game handled it.
    pub async fn console_command_async(&self, arguments: &[String]) -> Result<bool, GuestError> {
        self.console_command(arguments)
    }

    /// Restart guest data after `GAME_SHUTDOWN`; the caller runs `GAME_INIT`
    /// after source data reload.
    pub fn restart(&self, bytes: &[u8]) -> Result<(), GuestError> {
        self.module.restart(bytes)?;
        self.data.clear();
        Ok(())
    }

    /// Retire the module.
    pub fn retire(&self) {
        self.module.retire();
    }
}

#[cfg(test)]
mod tests {
    use super::super::game_data::QvmImage;
    use super::*;

    fn artifact() -> QvmArtifact {
        let mut image = QvmImage::default();
        image.allocated_data_length = 4096;
        QvmArtifact {
            module: super::super::game_data::ModuleIdentity {
                id: "q3:qagame".to_string(),
                artifact_path: "qagame.qvm".to_string(),
                digest: "d".to_string(),
                revision: "r".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        }
    }

    fn game() -> QvmGame {
        let host: QvmHostFn = std::rc::Rc::new(|_| Ok(None));
        QvmGame::new(artifact(), host).unwrap()
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

    #[test]
    fn role_and_api_version_selected_by_abi() {
        let game = game();
        assert_eq!(game.api().version, 8);
        assert_eq!(QvmGameApi::kind(), "q3-qagame");
        let mut legacy = artifact();
        legacy.abi_profile = Some(AbiProfile::Legacy);
        let host: QvmHostFn = std::rc::Rc::new(|_| Ok(None));
        assert_eq!(QvmGame::new(legacy, host).unwrap().api().version, 7);
        let mut wrong = artifact();
        wrong.role = QvmRole::Cgame;
        let host: QvmHostFn = std::rc::Rc::new(|_| Ok(None));
        assert!(QvmGame::new(wrong, host).is_err());
    }

    #[test]
    fn exports_reach_module_entries() {
        let game = game();
        game.initialize(100, 7, false).unwrap();
        game.client_think(0).unwrap();
        game.run_frame(100).unwrap();
        let calls = game.module.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].words[0], QvmGameExport::GAME_INIT);
        assert_eq!(calls[1].words[0], QvmGameExport::GAME_CLIENT_THINK);
        assert_eq!(calls[2].words[0], QvmGameExport::GAME_RUN_FRAME);
        game.client_command(1, &["say".to_string()]).unwrap();
        assert_eq!(game.module.commands().len(), 1);
    }

    #[test]
    fn client_connect_reads_denial_strings() {
        let game = game();
        assert_eq!(game.client_connect(0, true, false).unwrap(), None);
        game.module.set_default_return(64);
        game.module.memory().write_string(64, "banned", 64).unwrap();
        assert_eq!(game.client_connect(0, true, false).unwrap().as_deref(), Some("banned"));
    }

    #[test]
    fn locate_trap_shares_descriptors() {
        let game = game();
        let memory = game.module.memory();
        let call = QvmHostCall {
            kind: CallKind::Engine,
            role: QvmRole::Qagame,
            code: QvmGameImport::G_LOCATE_GAME_DATA,
            words: vec![QvmGameImport::G_LOCATE_GAME_DATA, 64, 2, 560, 1024, 480],
            guest: memory,
            abi_profile: AbiProfile::Modern,
            command_arguments: None,
        };
        assert_eq!(game.module.dispatch_host(&call).unwrap(), 0);
        assert_eq!(game.data.num_entities(), 2);
        assert_eq!(game.data.entity_stride_bytes(), 560);
    }

    #[test]
    fn async_variants_match_sync_behavior() {
        let game = game();
        block_on(game.initialize_async(10, 3, false)).unwrap();
        block_on(game.run_frame_async(10)).unwrap();
        assert_eq!(block_on(game.console_command_async(&[])).unwrap(), false);
        assert_eq!(game.module.calls().len(), 2);
        game.module.set_default_return(1);
        assert_eq!(block_on(game.console_command_async(&[])).unwrap(), true);
    }

    #[test]
    fn restart_clears_tables_and_retire_closes() {
        let game = game();
        game.data.locate(64, 1, 560, 1024, 480).unwrap();
        let bytes = vec![0u8; game.module.memory().len()];
        game.restart(&bytes).unwrap();
        assert_eq!(game.data.num_entities(), 0);
        game.retire();
        assert!(game.run_frame(1).is_err());
    }
}
