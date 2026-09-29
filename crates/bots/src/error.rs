//! Typed failures for bot navigation. Donor `RangeError`/`TypeError` sites
//! become named variants so callers match on meaning instead of message
//! text; donor `BinaryError` sites reuse [`qa_core::binary::BinaryError`].

use qa_core::binary::BinaryError;
use qa_core::numeric::NumericError;
use thiserror::Error;

use crate::entities::EntityError;
use crate::save::SaveError;

/// Failure of a navigation parse, build, estimate, or route operation.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BotsError {
    /// Byte-level parse failure.
    #[error(transparent)]
    Binary(#[from] BinaryError),
    /// Native number conversion failure.
    #[error(transparent)]
    Numeric(#[from] NumericError),
    /// Entity text parse failure.
    #[error(transparent)]
    Entities(#[from] EntityError),
    /// Checkpoint decode failure.
    #[error(transparent)]
    Save(#[from] SaveError),
    /// Indexed record access outside its table.
    #[error("{what} {index} exceeds {length}")]
    OutOfRange {
        /// Which table was addressed.
        what: &'static str,
        /// Requested index.
        index: i64,
        /// Table length.
        length: usize,
    },
    /// AAS BSP walk revisited more nodes than exist.
    #[error("Cyclic AAS BSP tree")]
    CyclicTree,
    /// AAS BSP walk reached a missing node or plane.
    #[error("Missing AAS BSP node/plane")]
    MissingBspNode,
    /// Area enumeration limit is not a nonnegative integer.
    #[error("Invalid navigation area limit")]
    InvalidAreaLimit,
    /// Estimate travel flags are not a 32-bit mask.
    #[error("Estimate travel flags must be a 32-bit mask")]
    InvalidTravelFlags,
    /// Estimate origin is not finite float32.
    #[error("Estimate origin must contain finite float32 coordinates")]
    InvalidEstimateOrigin,
    /// Navigation profile carries a non-finite scalar.
    #[error("Navigation profile must be finite")]
    NonFiniteProfile,
    /// Navigation body or traversal envelope is inverted or out of range.
    #[error("Invalid navigation body/traversal envelope")]
    BadProfileEnvelope,
    /// Collision policy family does not follow the selected movement.
    #[error("Navigation collision policy must follow selected movement, independently of BSP format")]
    PolicyMismatch,
    /// Map identity and decoded geometry disagree.
    #[error("Navigation map identity and decoded geometry disagree")]
    MapMismatch,
    /// Construction spacing, link distance, or node cap is invalid.
    #[error("Invalid navigation construction limits")]
    ConstructionLimits,
    /// Construction exceeded its node cap; no partial graph was published.
    #[error("Navigation construction exceeds its node limit")]
    NodeLimit {
        /// Configured cap.
        maximum: usize,
    },
    /// Construction needs a world model at index zero.
    #[error("Navigation construction requires a world model")]
    MissingWorldModel,
    /// Graph carries a duplicate node id.
    #[error("Duplicate navigation node")]
    DuplicateNode {
        /// Repeated id.
        id: i32,
    },
    /// Edge identity is duplicated or references a missing node.
    #[error("Invalid navigation edge identity/reference")]
    BadEdge,
    /// Edge travel cost is negative or non-finite.
    #[error("Invalid navigation edge cost")]
    BadEdgeCost,
    /// Unknown navigation area id.
    #[error("Unknown navigation area")]
    UnknownArea {
        /// Requested id.
        id: i32,
    },
    /// Unknown navigation edge id.
    #[error("Unknown navigation edge")]
    UnknownEdge {
        /// Requested id.
        id: i32,
    },
    /// A failed prediction session was reused instead of discarded.
    #[error("Discard a failed navigation prediction before trying another route")]
    PredictionPoisoned,
    /// Predictor is not the selected movement provider.
    #[error("Navigation predictor is not the selected movement provider")]
    PredictorMismatch,
    /// Movement input and provider kinds disagree.
    #[error("Navigation movement input/provider disagree")]
    InputProviderMismatch,
    /// Prediction changed the selected movement profile.
    #[error("Navigation prediction changed the selected movement profile")]
    PredictionProfileChanged,
    /// Prediction frame did not advance source time.
    #[error("Navigation prediction must advance source time")]
    PredictionStalled,
    /// Prediction second, command, or tolerance limit is invalid.
    #[error("Invalid navigation prediction limits")]
    PredictionLimits,
    /// Navigation must run movement in noncommitting prediction mode.
    #[error("Navigation must use noncommitting movement prediction")]
    PredictionMode,
    /// Movement admission returned no trajectory or an invalid duration.
    #[error("Movement admission returned no trajectory or invalid duration")]
    BadAdmissionTrajectory,
    /// Source float-to-int conversion left the defined range.
    #[error("{0}")]
    IntRange(String),
    /// AAS estimate distance exceeds the source int range.
    #[error("AAS estimate distance exceeds source int range")]
    EstimateRange,
    /// AAS trace point is non-finite.
    #[error("Non-finite AAS trace point")]
    NonFiniteTrace,
    /// Presence bounds are missing for both the AAS bboxes and the profile.
    #[error("Missing AAS presence bounds")]
    MissingPresenceBounds {
        /// Requested presence.
        presence: i32,
    },
    /// AAS generation prediction requires an AAS end area.
    #[error("AAS generation prediction requires an AAS end area")]
    PredictionArea,
    /// Too many BSP entities for AAS generation.
    #[error("Too many BSP entities for AAS generation")]
    EntityLimit,
    /// BSP epair output buffer is empty.
    #[error("Empty BSP epair output")]
    EmptyEpair,
    /// AAS generation requires a selected prediction client.
    #[error("AAS generation requires a selected prediction client")]
    BadPredictionClient,
    /// Non-finite intermediate in a source computation.
    #[error("{0}")]
    NonFinite(String),
    /// Portal-area scratch write exceeds its source allocation.
    #[error("AAS portal-area write exceeds its source allocation")]
    PortalAreaOverflow {
        /// Which scratch allocation overflowed.
        allocation: &'static str,
    },
    /// Navigation map resource path is invalid.
    #[error("Invalid navigation map resource path")]
    BadResourcePath,
    /// MD4 input exceeds the source unsigned-int length.
    #[error("MD4 update length exceeds unsigned int")]
    Md4Length,
    /// Internal invariant violation (donor `throw new Error` sites).
    #[error("{0}")]
    Internal(String),
}

/// Borrow a record by index, matching the donor's throwing accessors.
pub(crate) fn indexed<'a, T>(values: &'a [T], index: i64, what: &'static str) -> Result<&'a T, BotsError> {
    if index < 0 || index >= values.len() as i64 {
        return Err(BotsError::OutOfRange {
            what,
            index,
            length: values.len(),
        });
    }
    Ok(&values[index as usize])
}

/// Mutably borrow a record by index.
pub(crate) fn indexed_mut<'a, T>(values: &'a mut [T], index: i64, what: &'static str) -> Result<&'a mut T, BotsError> {
    if index < 0 || index >= values.len() as i64 {
        return Err(BotsError::OutOfRange {
            what,
            index,
            length: values.len(),
        });
    }
    Ok(&mut values[index as usize])
}
