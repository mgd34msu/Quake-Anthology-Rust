//! Source bot library from `src/bots/behavior/q3/library.ts`
//! (`be_interface.c`: `BotLibSetup`, `BotLibShutdown`, `BotLibStartFrame`,
//! `BotLibLoadMap`, `BotLibUpdateEntity`, `BotLibTest`).
//!
//! `BotLibrary` owns the botlib service bundle: memory, libvars,
//! characters, actions, chat, weights, weapons, goals, log, and move
//! states. Setup allocates per-map state; `start_frame` advances the
//! library clock; shutdown frees map state.

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::library::actions::BotActionBuffer;
use crate::behavior::library::character::BotCharacterLibrary;
use crate::behavior::library::chat::BotChatLibrary;
use crate::behavior::library::goals::BotGoalLibrary;
use crate::behavior::library::libvars::BotLibVars;
use crate::behavior::library::log::{BotLog, BotLogSink};
use crate::behavior::library::memory::BotMemory;
use crate::behavior::library::weapons::WeaponAi;
use crate::behavior::library::weights::WeightConfigStore;
use crate::behavior::q3::movement_state::BotMoveStateStore;
use crate::error::BotsError;

/// Bot library print severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotPrintSeverity {
    /// Message.
    Message = 1,
    /// Warning.
    Warning = 2,
    /// Error.
    Error = 3,
    /// Fatal.
    Fatal = 4,
}

/// Bot library services bundle.
pub struct BotLibrary<'a> {
    /// Bot memory manager.
    pub memory: BotMemory,
    /// Library variables.
    pub variables: BotLibVars,
    /// Character profiles.
    pub characters: BotCharacterLibrary,
    /// Per-client action cells.
    pub actions: BotActionBuffer,
    /// Chat runtime.
    pub chat: BotChatLibrary<'a>,
    /// Weight configs.
    pub weights: WeightConfigStore<'a>,
    /// Weapon knowledge.
    pub weapons: WeaponAi<'a>,
    /// Goal library.
    pub goals: BotGoalLibrary<'a>,
    /// Log.
    pub log: BotLog,
    /// Movement states.
    pub move_states: BotMoveStateStore,
    /// Debug build flag.
    pub debug_build: bool,
    initialized: bool,
    closed: bool,
    time: f32,
}

impl<'a> BotLibrary<'a> {
    /// New library over prepared files with `clients` action slots.
    pub fn new(files: &'a dyn BotSourceFiles, clients: usize, debug_build: bool) -> Self {
        Self {
            memory: BotMemory::new(),
            variables: BotLibVars::new(),
            characters: BotCharacterLibrary::new(),
            actions: BotActionBuffer::new(clients),
            chat: BotChatLibrary::new(files),
            weights: WeightConfigStore::new(files),
            weapons: WeaponAi::new(files),
            goals: BotGoalLibrary::new(files),
            log: BotLog::new(),
            move_states: BotMoveStateStore::new(),
            debug_build,
            initialized: false,
            closed: false,
            time: 0.0,
        }
    }

    /// Whether the library is set up.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Library clock seconds.
    #[must_use]
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Set up the library (`BotLibSetup`).
    pub fn setup(&mut self) -> Result<(), BotsError> {
        if self.closed || self.initialized {
            return Err(BotsError::BotLifetime(
                "bot library setup outside its lifetime".to_owned(),
            ));
        }
        self.initialized = true;
        Ok(())
    }

    /// Load map state (`BotLibLoadMap`): item config plus level reset.
    pub fn load_map(&mut self, item_config: &str) -> Result<(), BotsError> {
        if !self.initialized || self.closed {
            return Err(BotsError::BotLifetime(
                "bot library map load outside its lifetime".to_owned(),
            ));
        }
        self.goals.clear_level_items();
        let error = self.goals.load_item_config(item_config);
        if error != crate::behavior::library::goals::GoalError::NONE {
            return Err(BotsError::BotScript(format!("cannot load item config {item_config}")));
        }
        Ok(())
    }

    /// Start a library frame (`BotLibStartFrame`).
    pub fn start_frame(&mut self, time_seconds: f32) -> Result<(), BotsError> {
        if !self.initialized || self.closed {
            return Err(BotsError::BotLifetime(
                "bot library frame outside its lifetime".to_owned(),
            ));
        }
        if !time_seconds.is_finite() {
            return Err(BotsError::BotLifetime("bot observation time must be finite".to_owned()));
        }
        self.time = time_seconds;
        Ok(())
    }

    /// Shut down the library (`BotLibShutdown`).
    pub fn shutdown(&mut self, sink: &mut dyn BotLogSink) {
        if self.closed {
            return;
        }
        self.log.shutdown(sink);
        self.goals.clear_level_items();
        self.variables.clear();
        self.memory.dispose();
        self.initialized = false;
        self.closed = true;
    }
}
