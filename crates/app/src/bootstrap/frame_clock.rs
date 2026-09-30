//! Committed client frame clock.
//!
//! Donor provenance: `src/app/bootstrap/frame-clock.ts` (`PresentationTime`).
//! Direct port with no behavioral changes.

use thiserror::Error;

/// Failure of the presentation clock.
#[derive(Debug, Error)]
pub enum FrameClockError {
    /// No frame has been committed yet.
    #[error("Presentation clock has no committed frame")]
    NoFrame,
}

/// A committed client frame clock, separate from transport wall timestamps.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct PresentationTime {
    committed_milliseconds: Option<f64>,
}

impl PresentationTime {
    /// Create a clock with no committed frame.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the committed time by the presentation elapsed time.
    pub fn advance(
        &mut self,
        wall_milliseconds: f64,
        wall_elapsed_milliseconds: f64,
        presentation_elapsed_milliseconds: f64,
    ) {
        self.committed_milliseconds = Some(
            self.committed_milliseconds
                .unwrap_or(wall_milliseconds - wall_elapsed_milliseconds)
                + presentation_elapsed_milliseconds,
        );
    }

    /// Committed presentation time in milliseconds.
    pub fn milliseconds(&self) -> Result<f64, FrameClockError> {
        self.committed_milliseconds
            .ok_or(FrameClockError::NoFrame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncommitted_clock_errors() {
        let clock = PresentationTime::new();
        assert!(matches!(
            clock.milliseconds().unwrap_err(),
            FrameClockError::NoFrame
        ));
    }

    #[test]
    fn first_advance_anchors_to_wall_then_accumulates_presentation() {
        let mut clock = PresentationTime::new();
        clock.advance(1000.0, 16.0, 16.0);
        assert_eq!(clock.milliseconds().unwrap(), 1000.0);
        clock.advance(2000.0, 500.0, 33.0);
        assert_eq!(clock.milliseconds().unwrap(), 1033.0);
    }
}
