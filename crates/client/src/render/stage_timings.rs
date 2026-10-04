//! Per-stage frame timings for renderer optimization.
//!
//! New tooling (no donor provenance): [`StageTimer`] records named,
//! non-overlapping wall-clock sections inside one frame, and
//! [`StageTotals`] accumulates snapshots across a run for the
//! `--frame-timings` report. Disabled timers cost one branch per call.
//!
//! Stages must not nest: starting a section stops the open one, so the
//! per-frame samples always partition the measured region.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// One measured stage inside a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageSample {
    /// Stage name (a stable `&'static str` per call site).
    pub name: &'static str,
    /// Wall time spent in the stage.
    pub elapsed: Duration,
}

/// Wall-clock section recorder for one frame.
#[derive(Debug)]
pub struct StageTimer {
    enabled: bool,
    open: Option<(&'static str, Instant)>,
    frame: Vec<StageSample>,
}

impl StageTimer {
    /// Create a timer; disabled timers record nothing.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            open: None,
            frame: Vec::new(),
        }
    }

    /// Whether samples are recorded.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Stop the open section, if any, and start `name`.
    pub fn section(&mut self, name: &'static str) {
        if !self.enabled {
            return;
        }
        self.stop();
        self.open = Some((name, Instant::now()));
    }

    /// Stop the open section, if any, recording its elapsed time.
    pub fn stop(&mut self) {
        if !self.enabled {
            return;
        }
        if let Some((name, start)) = self.open.take() {
            self.frame.push(StageSample {
                name,
                elapsed: start.elapsed(),
            });
        }
    }

    /// Record one externally measured span.
    pub fn record(&mut self, name: &'static str, elapsed: Duration) {
        if !self.enabled {
            return;
        }
        self.frame.push(StageSample { name, elapsed });
    }

    /// Stop the open section and drain this frame's samples.
    pub fn take_frame(&mut self) -> Vec<StageSample> {
        self.stop();
        std::mem::take(&mut self.frame)
    }
}

/// Run-wide totals over framed stage samples.
#[derive(Debug, Default)]
pub struct StageTotals {
    frames: u64,
    order: Vec<&'static str>,
    totals: HashMap<&'static str, (Duration, u64, Duration)>,
}

impl StageTotals {
    /// Empty totals.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Frames accumulated so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Accumulate one frame's samples.
    pub fn add_frame(&mut self, samples: &[StageSample]) {
        self.frames += 1;
        for sample in samples {
            let entry = self.totals.entry(sample.name).or_insert_with(|| {
                self.order.push(sample.name);
                (Duration::ZERO, 0, Duration::ZERO)
            });
            entry.0 += sample.elapsed;
            entry.1 += 1;
            entry.2 = entry.2.max(sample.elapsed);
        }
    }

    /// Accumulate one externally measured span.
    pub fn record(&mut self, name: &'static str, elapsed: Duration) {
        self.add_frame(&[StageSample { name, elapsed }]);
    }

    /// Render the totals as an aligned text table (stages in first-seen
    /// order; shares are fractions of the summed stage time).
    #[must_use]
    pub fn render(&self) -> String {
        let grand: Duration = self.totals.values().map(|entry| entry.0).sum();
        let grand_secs = grand.as_secs_f64();
        let mut out = format!(
            "frame timings over {} frames (summed stage time {:.3} ms):\n{:>24} {:>8} {:>12} {:>12} {:>12} {:>6}\n",
            self.frames,
            grand_secs * 1000.0,
            "stage",
            "calls",
            "total ms",
            "avg us",
            "max us",
            "share",
        );
        for name in &self.order {
            let Some((total, calls, max)) = self.totals.get(name) else {
                continue;
            };
            let total_ms = total.as_secs_f64() * 1000.0;
            let avg_us = if *calls == 0 {
                0.0
            } else {
                total_ms * 1000.0 / (*calls as f64)
            };
            let share = if grand_secs > 0.0 {
                total.as_secs_f64() / grand_secs * 100.0
            } else {
                0.0
            };
            out.push_str(&format!(
                "{:>24} {:>8} {:>12.3} {:>12.1} {:>12.1} {:>5.1}%\n",
                name,
                calls,
                total_ms,
                avg_us,
                max.as_secs_f64() * 1_000_000.0,
                share,
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_timer_records_nothing() {
        let mut timer = StageTimer::new(false);
        timer.section("draw");
        timer.record("setup", Duration::from_micros(3));
        timer.stop();
        assert!(timer.take_frame().is_empty());
    }

    #[test]
    fn sections_partition_one_frame() {
        let mut timer = StageTimer::new(true);
        timer.section("setup");
        timer.section("draw");
        timer.stop();
        let frame = timer.take_frame();
        assert_eq!(frame.len(), 2);
        assert_eq!(frame[0].name, "setup");
        assert_eq!(frame[1].name, "draw");
        assert!(timer.take_frame().is_empty());
    }

    #[test]
    fn totals_render_first_seen_order_with_shares() {
        let mut totals = StageTotals::new();
        totals.add_frame(&[
            StageSample {
                name: "draw",
                elapsed: Duration::from_micros(300),
            },
            StageSample {
                name: "setup",
                elapsed: Duration::from_micros(100),
            },
        ]);
        totals.add_frame(&[StageSample {
            name: "draw",
            elapsed: Duration::from_micros(100),
        }]);
        assert_eq!(totals.frames(), 2);
        let table = totals.render();
        assert!(table.contains("over 2 frames"), "{table}");
        let draw_at = table.find("draw").expect("draw row");
        let setup_at = table.find("setup").expect("setup row");
        assert!(draw_at < setup_at, "{table}");
        assert!(table.contains("80.0%"), "{table}");
        assert!(table.contains("20.0%"), "{table}");
    }
}
