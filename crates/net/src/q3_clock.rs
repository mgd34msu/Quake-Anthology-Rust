//! Port of Quake-Anthology-TS `src/network/q3/clock.ts`
//!
//! `CL_SetCGameTime` and `CL_AdjustTimeDelta` (`cl_cgame.c`).
//! `Q3ClockOptions` carries the advance flags; [`Q3ClientClock`] owns the
//! published snapshot by clone (the donor holds a reference) and otherwise
//! follows the donor step for step. Donor `RangeError` failures surface as
//! [`Q3NetError::Range`], donor `CommonError("drop", ..)` failures as
//! [`Q3NetError::Drop`].

use crate::q3_net::{Q3NetError, Snapshot};

/// Range message for clock arithmetic outside signed int32 (donor `sourceClock`).
const CLOCK_RANGE: &str = "Q3 source clock arithmetic exceeds signed int32";

/// Check an intermediate clock value (donor `sourceClock`).
fn source_clock(value: i64) -> Result<i32, Q3NetError> {
    i32::try_from(value).map_err(|_| Q3NetError::Range(CLOCK_RANGE))
}

/// Check a real-time clock value (donor `sourceClock` over a float).
fn source_clock_real(value: f64) -> Result<i32, Q3NetError> {
    if value.trunc() != value || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(Q3NetError::Range(CLOCK_RANGE));
    }
    Ok(value as i32)
}

/// Clock advance options (donor `Q3ClockOptions`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3ClockOptions {
    /// Clock paused flag.
    pub paused: bool,
    /// Time nudge in milliseconds.
    pub time_nudge: f64,
    /// Timescale multiplier.
    pub timescale: f64,
    /// Demo playback flag.
    pub demo: bool,
    /// Frozen demo flag.
    pub freeze_demo: bool,
    /// Timedemo flag.
    pub timedemo: bool,
}

/// Benchmark timing (donor `demoTiming` return).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DemoTiming {
    /// Rendered demo frames.
    pub frames: i32,
    /// Elapsed milliseconds.
    pub elapsed_milliseconds: i32,
}

/// Quake III client clock (donor `Q3ClientClock`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Q3ClientClock {
    /// Current client time in milliseconds.
    pub time: i32,
    /// Server-to-client time delta in milliseconds.
    pub delta: i32,
    old_time: i32,
    old_frame_server_time: i32,
    pending: bool,
    extrapolated: bool,
    current: Option<Snapshot>,
    active: bool,
    demo_base_time: i32,
    demo_frames: i32,
    demo_start: i32,
}

impl Q3ClientClock {
    /// Zeroed clock.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a snapshot (donor `publish`).
    pub fn publish(&mut self, snapshot: &Snapshot) {
        self.current = Some(snapshot.clone());
        self.pending = true;
    }

    /// Clear clock state (donor `clear`).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Advance the clock (donor `advance`).
    ///
    /// Call after module initialization has primed this client; returns
    /// `None` until its first active snapshot.
    pub fn advance(&mut self, real_time: f64, options: &Q3ClockOptions) -> Result<Option<i32>, Q3NetError> {
        let real_time = source_clock_real(real_time)?;
        let snapshot = self.current.clone();
        if !self.active {
            match snapshot {
                Some(ref snapshot) if self.pending && snapshot.flags & 2 == 0 => {
                    self.pending = false;
                    self.active = true;
                    self.delta = source_clock(snapshot.server_time as i64 - real_time as i64)?;
                    self.old_time = snapshot.server_time;
                    self.demo_base_time = snapshot.server_time;
                }
                _ => return Ok(None),
            }
        }
        let Some(snapshot) = snapshot else {
            return Err(Q3NetError::Drop {
                kind: "drop",
                message: "CL_SetCGameTime: !cl.snap.valid".to_string(),
            });
        };
        if options.paused {
            return Ok(Some(self.time));
        }
        if snapshot.server_time < self.old_frame_server_time {
            return Err(Q3NetError::Drop {
                kind: "drop",
                message: "cl.snap.serverTime < cl.oldFrameServerTime".to_string(),
            });
        }
        self.old_frame_server_time = snapshot.server_time;
        if !options.demo || !options.freeze_demo {
            let nudge = options.time_nudge.trunc().clamp(-30.0, 30.0);
            let shifted = source_clock(real_time as i64 + self.delta as i64)?;
            self.time = source_clock_real(f64::from(shifted) - nudge)?;
            if self.time < self.old_time {
                self.time = self.old_time;
            }
            self.old_time = self.time;
            if shifted >= source_clock(snapshot.server_time as i64 - 5)? {
                self.extrapolated = true;
            }
        }
        if self.pending {
            self.pending = false;
            if !options.demo {
                let next = source_clock(snapshot.server_time as i64 - real_time as i64)?;
                let distance = source_clock((next as i64 - self.delta as i64).abs())?;
                if distance > 500 {
                    self.delta = next;
                    self.old_time = snapshot.server_time;
                    self.time = snapshot.server_time;
                } else if distance > 100 {
                    self.delta = source_clock(self.delta as i64 + next as i64)? >> 1;
                } else if options.timescale == 0.0 || options.timescale == 1.0 {
                    if self.extrapolated {
                        self.extrapolated = false;
                        self.delta = source_clock(self.delta as i64 - 2)?;
                    } else {
                        self.delta = source_clock(self.delta as i64 + 1)?;
                    }
                }
            }
        }
        if options.demo && options.timedemo {
            if self.demo_start == 0 {
                self.demo_start = real_time;
            }
            self.demo_frames = source_clock(self.demo_frames as i64 + 1)?;
            let frames = source_clock(self.demo_frames as i64 * 50)?;
            self.time = source_clock(self.demo_base_time as i64 + frames as i64)?;
        }
        Ok(Some(self.time))
    }

    /// Whether the clock needs another demo message (donor `needsDemoMessage`).
    #[must_use]
    pub fn needs_demo_message(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|snapshot| self.time >= snapshot.server_time)
    }

    /// Benchmark timing, when the demo started (donor `demoTiming`).
    pub fn demo_timing(&self, milliseconds: f64) -> Result<Option<Q3DemoTiming>, Q3NetError> {
        let elapsed = source_clock_real(milliseconds - f64::from(self.demo_start))?;
        Ok((elapsed > 0).then_some(Q3DemoTiming {
            frames: self.demo_frames,
            elapsed_milliseconds: elapsed,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3_net::Q3Product;

    fn live_options() -> Q3ClockOptions {
        Q3ClockOptions {
            paused: false,
            time_nudge: 0.0,
            timescale: 1.0,
            demo: false,
            freeze_demo: false,
            timedemo: false,
        }
    }

    fn snapshot(server_time: i32, flags: i32) -> Snapshot {
        Snapshot {
            message_number: 2,
            server_time,
            delta_number: 0,
            flags,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: Vec::new(),
            player_state: crate::q3_net::Q3PlayerState::new(Q3Product::Base),
            entities: Vec::new(),
        }
    }

    #[test]
    fn advance_waits_for_active_snapshot() {
        let mut clock = Q3ClientClock::new();
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), None);
        clock.publish(&snapshot(50, 2));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), None);
        clock.publish(&snapshot(50, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(50));
        assert_eq!(clock.delta, -950);
    }

    #[test]
    fn advance_clamps_and_floors_time() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&snapshot(1000, 0));
        let mut options = live_options();
        options.time_nudge = 90.0;
        assert_eq!(clock.advance(1000.0, &options).unwrap(), Some(1000));
        options.time_nudge = -90.0;
        assert_eq!(clock.advance(1000.0, &options).unwrap(), Some(1030));
        assert_eq!(clock.advance(900.0, &live_options()).unwrap(), Some(1030));
    }

    #[test]
    fn paused_returns_time_without_extrapolating() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&snapshot(50, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(50));
        let mut options = live_options();
        options.paused = true;
        assert_eq!(clock.advance(2000.0, &options).unwrap(), Some(50));
    }

    #[test]
    fn large_distance_resets_delta() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&snapshot(1000, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(1000));
        clock.publish(&snapshot(2000, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(2000));
        assert_eq!(clock.delta, 1000);
    }

    #[test]
    fn small_distance_nudges_delta() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&snapshot(1000, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(1000));
        clock.publish(&snapshot(1050, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(1000));
        assert_eq!(clock.delta, -2);
    }

    #[test]
    fn timedemo_counts_frames_from_base() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&snapshot(50, 0));
        let mut options = live_options();
        options.demo = true;
        options.timedemo = true;
        assert_eq!(clock.advance(1000.0, &options).unwrap(), Some(100));
        assert_eq!(clock.advance(1000.0, &options).unwrap(), Some(150));
        assert_eq!(
            clock.demo_timing(1250.0).unwrap(),
            Some(Q3DemoTiming {
                frames: 2,
                elapsed_milliseconds: 250
            })
        );
        assert!(clock.needs_demo_message());
    }

    #[test]
    fn demo_timing_needs_positive_elapsed() {
        let clock = Q3ClientClock::new();
        assert_eq!(clock.demo_timing(0.0).unwrap(), None);
        assert_eq!(
            clock.demo_timing(30.0).unwrap(),
            Some(Q3DemoTiming {
                frames: 0,
                elapsed_milliseconds: 30
            })
        );
    }

    #[test]
    fn out_of_range_real_time_fails() {
        let mut clock = Q3ClientClock::new();
        assert!(matches!(
            clock.advance(f64::from(i32::MAX) + 1.0, &live_options()),
            Err(Q3NetError::Range(_))
        ));
        assert!(matches!(
            clock.advance(f64::NAN, &live_options()),
            Err(Q3NetError::Range(_))
        ));
    }

    #[test]
    fn backwards_snapshot_drops() {
        let mut clock = Q3ClientClock::new();
        clock.publish(&snapshot(1000, 0));
        assert_eq!(clock.advance(1000.0, &live_options()).unwrap(), Some(1000));
        clock.publish(&snapshot(900, 0));
        assert!(matches!(
            clock.advance(1000.0, &live_options()),
            Err(Q3NetError::Drop { .. })
        ));
    }
}
