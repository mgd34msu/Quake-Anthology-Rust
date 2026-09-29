//! Source clocks ported from `src/contracts/time.ts`. Source time keeps its
//! units; conversion happens explicitly at a provider boundary. Simulation
//! code never reads the host clock.

/// Per-family frame clock profiles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockProfile {
    /// NetQuake variable frame with clamps and an optional fixed step.
    Q1Netquake {
        /// Minimum frame length in seconds.
        minimum_frame_seconds: f64,
        /// Maximum frame length in seconds.
        maximum_frame_seconds: f64,
        /// Fixed frame length, when forced.
        fixed_frame_seconds: Option<f64>,
    },
    /// QuakeWorld command-millisecond clamp.
    Q1Quakeworld {
        /// Maximum command length in milliseconds.
        maximum_command_milliseconds: f64,
    },
    /// Classic Quake II 100ms server frame.
    Q2Classic,
    /// Rerelease Quake II variable frame, prepared before the frame.
    Q2Rerelease {
        /// Frame length in milliseconds.
        frame_milliseconds: f64,
    },
    /// Quake III server frame with optional fixed movement step.
    Q3 {
        /// Server frame length in milliseconds.
        server_frame_milliseconds: f64,
        /// Fixed movement step in milliseconds, when forced.
        fixed_movement_milliseconds: Option<f64>,
    },
}

/// Source time in its native units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceTime {
    /// Seconds as binary32 (Q1/Q2 server time).
    Seconds(f32),
    /// Milliseconds as `i32` (Q2/Q3 frame clocks).
    Milliseconds(i32),
}

impl SourceTime {
    /// Seconds value as `f64` for boundary conversion.
    #[must_use]
    pub fn as_seconds_f64(&self) -> f64 {
        match *self {
            SourceTime::Seconds(value) => f64::from(value),
            SourceTime::Milliseconds(value) => f64::from(value) / 1000.0,
        }
    }

    /// Milliseconds value, truncating toward zero for seconds input.
    #[must_use]
    pub fn as_milliseconds_truncated(&self) -> i32 {
        match *self {
            SourceTime::Seconds(value) => (f64::from(value) * 1000.0).trunc() as i32,
            SourceTime::Milliseconds(value) => value,
        }
    }
}

/// Simulation phase within a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePhase {
    /// Frame entry.
    FrameEntry,
    /// Client command execution.
    ClientCommand,
    /// Entity prethink.
    EntityPrethink,
    /// Entity physics.
    EntityPhysics,
    /// Entity think.
    EntityThink,
    /// Client end-of-frame.
    ClientEndFrame,
    /// Frame exit.
    FrameExit,
}

/// Clock reading for one simulation frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameContext {
    /// Frame number.
    pub frame: i32,
    /// Current source time.
    pub time: SourceTime,
    /// Elapsed source time since the previous frame.
    pub elapsed: SourceTime,
    /// Current phase.
    pub phase: FramePhase,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_time_converts_at_boundaries() {
        assert_eq!(SourceTime::Seconds(1.5).as_milliseconds_truncated(), 1500);
        assert_eq!(SourceTime::Milliseconds(2500).as_seconds_f64(), 2.5);
        assert_eq!(SourceTime::Seconds(-0.5).as_milliseconds_truncated(), -500);
    }
}
