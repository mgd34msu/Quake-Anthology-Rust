//! Navigation-visible movement contracts projected from
//! `src/contracts/movement.ts`: provider identity, prediction input/result
//! shapes, and the movement services subset that navigation threads through
//! to the selected provider. Full movement ownership lives with the
//! movement provider; this module pins the shapes navigation consumes.

use qa_core::identity::ProviderId;
use qa_core::math::Vec3;
use qa_core::numeric::NumericProfile;
use qa_core::time::FrameContext;

use crate::scene::{TraceHit, WorldKind};

/// Movement provider family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MovementKind {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
    /// Classic Quake II.
    Q2Classic,
    /// Rerelease Quake II.
    Q2Rerelease,
    /// Quake III.
    Q3,
}

impl MovementKind {
    /// Collision family selected by this movement kind.
    #[must_use]
    pub fn family(&self) -> WorldKind {
        match self {
            MovementKind::Q1Netquake | MovementKind::Q1Quakeworld => WorldKind::Q1Bsp,
            MovementKind::Q2Classic | MovementKind::Q2Rerelease => WorldKind::Q2Bsp,
            MovementKind::Q3 => WorldKind::Q3Bsp,
        }
    }
}

/// Selected movement provider: kind, identity, and numeric profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovementProfile {
    /// Provider family.
    pub kind: MovementKind,
    /// Provider identity.
    pub id: ProviderId,
    /// Selected numeric profile.
    pub numeric: NumericProfile,
}

/// Movement execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MovementExecution {
    /// Committing authoritative step.
    Authoritative,
    /// Noncommitting prediction step.
    Prediction,
}

/// Prediction input handed to the selected provider. Fixtures and drivers
/// build this from the previous result, the command index, and the
/// traversal request; navigation checks execution, kind, profile, and
/// frame advance before stepping.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementInput {
    /// Provider family.
    pub kind: MovementKind,
    /// Selected movement profile.
    pub profile: MovementProfile,
    /// Frame clock; elapsed must advance source time.
    pub frame: FrameContext,
    /// Execution mode; navigation requires prediction.
    pub execution: MovementExecution,
}

/// Predicted movement state origin. Only the classic Quake II fixed-point
/// origin differs; every other family reports a float origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MovementState {
    /// NetQuake origin.
    Q1Netquake {
        /// Predicted origin.
        origin: Vec3,
    },
    /// QuakeWorld origin.
    Q1Quakeworld {
        /// Predicted origin.
        origin: Vec3,
    },
    /// Classic Quake II origin in eighths.
    Q2Classic {
        /// Predicted origin in eighths of a unit.
        origin_eighths: [i32; 3],
    },
    /// Rerelease Quake II origin.
    Q2Rerelease {
        /// Predicted origin.
        origin: Vec3,
    },
    /// Quake III origin.
    Q3 {
        /// Predicted origin.
        origin: Vec3,
    },
}

impl MovementState {
    /// Provider family of this state.
    #[must_use]
    pub fn kind(&self) -> MovementKind {
        match self {
            MovementState::Q1Netquake { .. } => MovementKind::Q1Netquake,
            MovementState::Q1Quakeworld { .. } => MovementKind::Q1Quakeworld,
            MovementState::Q2Classic { .. } => MovementKind::Q2Classic,
            MovementState::Q2Rerelease { .. } => MovementKind::Q2Rerelease,
            MovementState::Q3 { .. } => MovementKind::Q3,
        }
    }

    /// Predicted origin in world units.
    #[must_use]
    pub fn origin(&self) -> Vec3 {
        match *self {
            MovementState::Q1Netquake { origin }
            | MovementState::Q1Quakeworld { origin }
            | MovementState::Q2Rerelease { origin }
            | MovementState::Q3 { origin } => origin,
            MovementState::Q2Classic { origin_eighths } => Vec3 {
                x: origin_eighths[0] as f32 / 8.0,
                y: origin_eighths[1] as f32 / 8.0,
                z: origin_eighths[2] as f32 / 8.0,
            },
        }
    }
}

/// Predicted movement outcome: an active state or a provider callback that
/// removed the actor mid-step.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementResult {
    /// Active predicted state.
    Active {
        /// Predicted state.
        state: MovementState,
        /// Ground contact.
        ground: TraceHit,
        /// Water level.
        water_level: i32,
    },
    /// The provider removed the predicted actor.
    ActorRemoved,
}

/// Movement services subset threaded through prediction. Providers that
/// need more (scene, touch, weapons, animation) capture it in their own
/// state; navigation only forwards this record to the selected provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementServices {
    /// Selected numeric profile.
    pub numeric: NumericProfile,
}

/// Selected movement provider invoked in prediction mode.
pub trait MovementProvider {
    /// Provider family.
    fn kind(&self) -> MovementKind;
    /// Provider identity.
    fn id(&self) -> &ProviderId;
    /// Advance one prediction command. Never mutates live actors.
    fn advance(&self, input: &MovementInput, services: &MovementServices) -> MovementResult;
}
