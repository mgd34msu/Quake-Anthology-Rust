//! Typed failures for the simulation crate. Donor `RangeError` sites become
//! named variants so callers match on meaning instead of message text.

use thiserror::Error;

/// Failure of a world, physics, gameplay, or session operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorldError {
    /// Source time must be finite.
    #[error("Source time must be finite")]
    NonFiniteTime,
    /// Source clocks need an explicit unit conversion.
    #[error("Source clocks need an explicit unit conversion")]
    TimeUnit,
    /// Elapsed source time cannot be negative.
    #[error("Elapsed source time cannot be negative")]
    NegativeTime,
    /// Frame counter exhausted.
    #[error("Frame counter exhausted")]
    ClockOverflow,
    /// Think clock has the wrong source unit.
    #[error("Think clock has the wrong source unit")]
    ThinkUnit,
    /// Actor registry is full.
    #[error("Actor registry is full")]
    RegistryFull,
    /// Actor generation is exhausted.
    #[error("Actor generation is exhausted")]
    GenerationExhausted,
    /// Stale or foreign actor authority.
    #[error("Stale or foreign actor authority")]
    StaleActor,
    /// Actor belongs to another session.
    #[error("Actor belongs to another session")]
    ForeignActor,
    /// Source slot must be a nonnegative safe integer.
    #[error("Source slot must be a nonnegative safe integer")]
    BadSourceSlot,
    /// Source slot is occupied or has no pending checkpoint reconstruction.
    #[error("{0}")]
    SourceBinding(String),
    /// Actor capacity must be a positive safe integer.
    #[error("Actor capacity must be a positive safe integer")]
    BadCapacity,
    /// Actor registry is closed.
    #[error("Actor registry is closed")]
    RegistryClosed,
    /// Actor already has a body binding.
    #[error("Actor already has a body binding")]
    BodyExists,
    /// Cannot link an actor without a body.
    #[error("Actor has no body")]
    BodyMissing,
    /// Cannot attach to a missing anchor body.
    #[error("Body attachment anchor is missing")]
    AnchorMissing,
    /// Body attachment cycle.
    #[error("Body attachment cycle")]
    AttachCycle,
    /// Invalid saved body link count.
    #[error("Invalid saved body link count")]
    BadLinkCount,
    /// Frame scheduler is closed.
    #[error("Frame scheduler is closed")]
    SchedulerClosed,
    /// Scheduler advance is already running.
    #[error("Scheduler advance is already running")]
    SchedulerReentrant,
    /// Unknown think callback.
    #[error("Unknown think callback: {0}")]
    UnknownCallback(String),
    /// Provider is absent from mixed frame ordering.
    #[error("Provider is absent from mixed frame ordering")]
    ProviderAbsent,
    /// Missing scheduler clock.
    #[error("Missing scheduler clock: {0}")]
    MissingClock(String),
    /// Missing source frame for provider.
    #[error("Missing source frame for provider: {0}")]
    MissingFrame(String),
    /// Think execution provider disagrees with the actor's registered continuation.
    #[error("Think execution provider disagrees with the actor's registered continuation")]
    ExecutionProvider,
    /// Think order must name its owning actor and provider.
    #[error("Think order must name its owning actor and provider")]
    ThinkOrder,
    /// Think invocation sequence must be a nonnegative safe integer.
    #[error("Think invocation sequence must be a nonnegative safe integer")]
    ThinkSequence,
    /// Cannot cancel another provider's think.
    #[error("Cannot cancel another provider's think")]
    ThinkOwner,
    /// Duplicate scheduler clock.
    #[error("Duplicate scheduler clock: {0}")]
    DuplicateClock(String),
    /// Duplicate provider in frame order.
    #[error("Duplicate provider in frame order: {0}")]
    DuplicateProvider(String),
    /// Invalid Quake hull node or plane.
    #[error("{0}")]
    Hull(String),
    /// Cycle in Quake hull.
    #[error("Cycle in Quake hull")]
    HullCycle,
    /// Inventory quantity must be finite and nonnegative.
    #[error("Inventory quantity must be finite and nonnegative")]
    BadQuantity,
    /// Inventory counter must be finite.
    #[error("Inventory counter must be finite")]
    BadCounter,
    /// Inventory counter exceeds binary32 range.
    #[error("Inventory counter exceeds binary32 range")]
    CounterRange,
    /// Duplicate inventory item.
    #[error("Duplicate inventory item {0}")]
    DuplicateItem(String),
    /// Actor has no inventory binding.
    #[error("Actor has no inventory binding")]
    InventoryMissing,
    /// Item is not a signed source counter.
    #[error("Item is not a signed source counter")]
    NotSourceCounter,
    /// Pickup destination was not admitted.
    #[error("Pickup destination {0} was not admitted")]
    PickupAdmission(String),
    /// Invalid pickup grant plan.
    #[error("{0}")]
    PickupPlan(String),
    /// Q2 victim armor requires an explicit classic or rerelease source profile.
    #[error("Q2 victim armor requires an explicit classic or rerelease source profile")]
    ArmorProfile,
    /// Source regular armor requires its original absorption binding.
    #[error("Source regular armor requires its original absorption binding")]
    SourceArmor,
    /// Unknown server command.
    #[error("Unknown command: {0}")]
    UnknownCommand(String),
    /// Server command usage error.
    #[error("{0}")]
    CommandUsage(String),
    /// A level transition is already committing.
    #[error("A level transition is already committing")]
    TransitionBusy,
    /// Transition was not resolved here or was already committed.
    #[error("Transition was not resolved here or was already committed")]
    TransitionStale,
    /// Simulation is closed.
    #[error("Simulation is closed")]
    SimulationClosed,
    /// Invalid save image.
    #[error("{0}")]
    BadSave(String),
    /// Movement impact trace requires a collision plane.
    #[error("Movement impact trace requires a collision plane")]
    ImpactPlane,
    /// Unknown spawn classname.
    #[error("Unknown spawn classname: {0}")]
    UnknownSpawnClass(String),
    /// Invalid spawn field value.
    #[error("{0}")]
    BadSpawnFields(String),
    /// Invalid client command value.
    #[error("{0}")]
    BadClientCommand(String),
    /// Timer clock has the wrong source unit.
    #[error("Timer clock has the wrong source unit")]
    TimerUnit,
    /// Fixed-step plan needs a positive step.
    #[error("Fixed-step plan needs a positive step")]
    BadFixedStep,
    /// Server is closed.
    #[error("Server is closed")]
    ServerClosed,
    /// Resource scope is closed.
    #[error("{0} is closed")]
    ResourceClosed(String),
    /// Closing a resource scope failed; member messages are kept in `errors`.
    #[error("Failed to close {name}")]
    CloseFailed {
        /// Scope name.
        name: String,
        /// Member failure messages, in reverse-acquisition order.
        errors: Vec<String>,
    },
    /// Client slot is occupied.
    #[error("Client slot {0} is occupied")]
    ClientSlotOccupied(u32),
    /// Client slot generation is exhausted.
    #[error("Client slot generation is exhausted")]
    ClientGenerationExhausted,
    /// Cycle in Quake clip-space BSP.
    #[error("Cycle in Quake clip-space BSP")]
    ClipCycle,
    /// Invalid Quake clip-space node.
    #[error("Invalid Quake clip-space node")]
    BadClipNode,
    /// Unknown Quake model.
    #[error("Unknown Quake model {0}")]
    UnknownQ1Model(i32),
    /// Quake model has no drawing hull.
    #[error("Quake model has no drawing hull")]
    MissingDrawingHull,
    /// Unknown Quake solid-space leaf.
    #[error("Unknown Quake solid-space leaf")]
    UnknownSolidLeaf,
    /// Cycle in Quake solid-space BSP.
    #[error("Cycle in Quake solid-space BSP")]
    SolidCycle,
    /// Unknown Quake solid-space node/plane.
    #[error("Unknown Quake solid-space node/plane")]
    BadSolidNode,
    /// Quake solid-cell capsule sweep did not converge.
    #[error("Quake solid-cell capsule sweep did not converge")]
    SweepDiverged,
    /// Invalid mod operation registration or dispatch.
    #[error("{0}")]
    BadModOperation(String),
    /// Invalid original pickup rule, selection, or consumption.
    #[error("{0}")]
    BadPickup(String),
    /// Invalid weapon behavior attachment or checkpoint.
    #[error("{0}")]
    BadWeaponBehavior(String),
}
