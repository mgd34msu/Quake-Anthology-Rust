//! Source execution backends for simulated actors.
//!
//! Object-safe identity surfaces for the three server-game execution
//! backends (native library, QVM bytecode, QuakeC program). The application
//! simulation tags each source with one backend
//! (`SourceExecution` in `qa-app`) and dispatches per-actor execution
//! through these trait objects; the free matching helpers there compare a
//! recipe execution module against prepared guest state using the same
//! discriminating fields exposed here (owner, role, API, ABI, artifact).

/// Execution backend discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorExecutionKind {
    /// Native shared-library server game.
    Native,
    /// QVM bytecode server game.
    Qvm,
    /// QuakeC program server game.
    QuakeC,
}

/// Identity of a native server-game execution.
pub trait NativeActorExecution {
    /// Backend discriminator.
    fn kind(&self) -> ActorExecutionKind;
    /// Owning provider (`namespace:name` text).
    fn owner_provider(&self) -> String;
    /// Owning content identity text.
    fn owner_content(&self) -> String;
    /// Module role text (`server-game` for actor execution).
    fn role(&self) -> String;
    /// Native module API text.
    fn api(&self) -> String;
    /// Native ABI profile text.
    fn abi(&self) -> String;
    /// Requested artifact path.
    fn artifact_path(&self) -> String;
}

/// Identity of a QVM server-game execution.
pub trait QvmActorExecution {
    /// Backend discriminator.
    fn kind(&self) -> ActorExecutionKind;
    /// Owning provider (`namespace:name` text).
    fn owner_provider(&self) -> String;
    /// Owning content identity text.
    fn owner_content(&self) -> String;
    /// Module role text (`server-game` for actor execution).
    fn role(&self) -> String;
    /// QVM API identity text.
    fn api(&self) -> String;
    /// Requested artifact path.
    fn artifact_path(&self) -> String;
}

/// Identity of a QuakeC server-game execution.
pub trait QuakeCActorExecution {
    /// Backend discriminator.
    fn kind(&self) -> ActorExecutionKind;
    /// Owning provider (`namespace:name` text).
    fn owner_provider(&self) -> String;
    /// Owning content identity text.
    fn owner_content(&self) -> String;
    /// QuakeC API identity text.
    fn api(&self) -> String;
    /// Requested artifact path.
    fn artifact_path(&self) -> String;
}
