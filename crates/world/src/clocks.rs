//! Source clocks ported from `src/world/session/clocks.ts`. The caller
//! supplies its source timestep; no renderer tick or protocol chooses it.

use qa_core::time::{FrameContext, FramePhase, SourceTime};

use crate::WorldError;

/// Source time must be finite.
pub fn validate_time(time: SourceTime) -> Result<(), WorldError> {
    let finite = match time {
        SourceTime::Seconds(value) => value.is_finite(),
        SourceTime::Milliseconds(_) => true,
    };
    if finite {
        Ok(())
    } else {
        Err(WorldError::NonFiniteTime)
    }
}

/// Both times must share their source unit; conversion is explicit.
pub fn same_time_unit(left: SourceTime, right: SourceTime) -> Result<(), WorldError> {
    let same = matches!(
        (left, right),
        (SourceTime::Seconds(_), SourceTime::Seconds(_)) | (SourceTime::Milliseconds(_), SourceTime::Milliseconds(_))
    );
    if same {
        Ok(())
    } else {
        Err(WorldError::TimeUnit)
    }
}

/// Elapsed source time; negative steps are rejected.
fn validate_elapsed(elapsed: SourceTime) -> Result<(), WorldError> {
    validate_time(elapsed)?;
    let negative = match elapsed {
        SourceTime::Seconds(value) => value < 0.0,
        SourceTime::Milliseconds(value) => value < 0,
    };
    if negative {
        return Err(WorldError::NegativeTime);
    }
    Ok(())
}

/// One source clock. `FrameContext` is `Copy`, so no copy helpers are needed.
#[derive(Debug, Clone)]
pub struct SourceClock {
    current: FrameContext,
}

impl SourceClock {
    /// Create a clock at its initial source time and frame zero.
    pub fn new(initial: SourceTime) -> Result<Self, WorldError> {
        validate_time(initial)?;
        Ok(Self {
            current: FrameContext {
                frame: 0,
                time: initial,
                elapsed: match initial {
                    SourceTime::Seconds(_) => SourceTime::Seconds(0.0),
                    SourceTime::Milliseconds(_) => SourceTime::Milliseconds(0),
                },
                phase: FramePhase::FrameEntry,
            },
        })
    }

    /// Current frame context.
    #[must_use]
    pub fn frame(&self) -> FrameContext {
        self.current
    }

    /// Advance by the caller's source timestep.
    pub fn advance(&mut self, elapsed: SourceTime, phase: FramePhase) -> Result<FrameContext, WorldError> {
        validate_elapsed(elapsed)?;
        same_time_unit(self.current.time, elapsed)?;
        let time = match (self.current.time, elapsed) {
            (SourceTime::Seconds(now), SourceTime::Seconds(step)) => SourceTime::Seconds(now + step),
            (SourceTime::Milliseconds(now), SourceTime::Milliseconds(step)) => {
                SourceTime::Milliseconds(now.checked_add(step).ok_or(WorldError::ClockOverflow)?)
            }
            _ => return Err(WorldError::TimeUnit),
        };
        validate_time(time)?;
        let frame = self.current.frame.checked_add(1).ok_or(WorldError::ClockOverflow)?;
        self.current = FrameContext {
            frame,
            time,
            elapsed,
            phase,
        };
        Ok(self.current)
    }

    /// Enter a new phase without advancing time.
    pub fn enter(&mut self, phase: FramePhase) -> FrameContext {
        self.current.phase = phase;
        self.current
    }

    /// Restore a clock from a saved frame context.
    pub fn restore(frame: FrameContext) -> Result<Self, WorldError> {
        validate_time(frame.time)?;
        validate_time(frame.elapsed)?;
        same_time_unit(frame.time, frame.elapsed)?;
        if frame.frame < 0 {
            return Err(WorldError::ClockOverflow);
        }
        Ok(Self { current: frame })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_clock_advances_and_enters_phases() {
        let mut clock = SourceClock::new(SourceTime::Seconds(1.0)).unwrap();
        assert_eq!(clock.frame().frame, 0);
        let frame = clock
            .advance(SourceTime::Seconds(0.5), FramePhase::EntityPhysics)
            .unwrap();
        assert_eq!(frame.frame, 1);
        assert_eq!(frame.time, SourceTime::Seconds(1.5));
        assert_eq!(frame.elapsed, SourceTime::Seconds(0.5));
        assert_eq!(frame.phase, FramePhase::EntityPhysics);
        let entered = clock.enter(FramePhase::EntityThink);
        assert_eq!(entered.frame, 1);
        assert_eq!(entered.time, SourceTime::Seconds(1.5));
    }

    #[test]
    fn milliseconds_clock_rejects_mixed_units_and_negative_steps() {
        let mut clock = SourceClock::new(SourceTime::Milliseconds(100)).unwrap();
        let frame = clock
            .advance(SourceTime::Milliseconds(50), FramePhase::FrameEntry)
            .unwrap();
        assert_eq!(frame.time, SourceTime::Milliseconds(150));
        assert_eq!(
            clock.advance(SourceTime::Seconds(1.0), FramePhase::FrameEntry),
            Err(WorldError::TimeUnit)
        );
        assert_eq!(
            clock.advance(SourceTime::Milliseconds(-1), FramePhase::FrameEntry),
            Err(WorldError::NegativeTime)
        );
        assert!(matches!(
            SourceClock::new(SourceTime::Seconds(f32::INFINITY)),
            Err(WorldError::NonFiniteTime)
        ));
    }
}
