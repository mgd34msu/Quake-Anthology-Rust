//! Portable replacement for Q3's optional macOS OmniTimer stack and stamp lists.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/core/omnitimer.ts`.

use std::collections::HashMap;
use std::time::Instant;

use thiserror::Error;

/// Aggregated report row for one timer name.
#[derive(Debug, Clone, PartialEq)]
pub struct TimerReport {
    /// Timer name.
    pub name: String,
    /// Number of completed scopes.
    pub calls: u64,
    /// Total milliseconds including nested scopes.
    pub total_milliseconds: f64,
    /// Milliseconds excluding nested scopes.
    pub self_milliseconds: f64,
    /// Longest single scope in milliseconds.
    pub maximum_milliseconds: f64,
}

/// One timestamped mark in milliseconds since the epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct TimerStamp {
    /// Stamp name.
    pub name: String,
    /// Milliseconds since the timer epoch.
    pub milliseconds: f64,
}

#[derive(Debug, Clone)]
struct ActiveTimer {
    name: String,
    start: f64,
    children: f64,
}

#[derive(Debug, Clone, Default)]
struct Aggregate {
    calls: u64,
    total_milliseconds: f64,
    self_milliseconds: f64,
    maximum_milliseconds: f64,
}

/// Timer misuse errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OmniTimerError {
    /// Stamp capacity must be at least one.
    #[error("Invalid timer stamp capacity")]
    InvalidCapacity,
    /// Mode/reset changed while a scope is active.
    #[error("Cannot change timer mode inside an active scope")]
    ActiveScope,
    /// Pop without a matching push.
    #[error("Unbalanced timer pop")]
    UnbalancedPop,
}

/// Hierarchical stopwatch with stamp lists; inert until enabled.
pub struct OmniTimer {
    clock: Box<dyn Fn() -> f64>,
    stamp_capacity: usize,
    stack: Vec<ActiveTimer>,
    totals: HashMap<String, Aggregate>,
    stamps: Vec<TimerStamp>,
    epoch: f64,
    active: bool,
}

impl OmniTimer {
    /// Build a timer over `clock` (milliseconds) with `stamp_capacity` stamps.
    pub fn with_clock(clock: impl Fn() -> f64 + 'static, stamp_capacity: usize) -> Result<Self, OmniTimerError> {
        if stamp_capacity < 1 {
            return Err(OmniTimerError::InvalidCapacity);
        }
        let clock: Box<dyn Fn() -> f64> = Box::new(clock);
        let epoch = clock();
        Ok(Self {
            clock,
            stamp_capacity,
            stack: Vec::new(),
            totals: HashMap::new(),
            stamps: Vec::new(),
            epoch,
            active: false,
        })
    }

    /// Build a wall-clock timer with the donor default stamp capacity (4096).
    pub fn new() -> Self {
        let start = Instant::now();
        Self::with_clock(move || start.elapsed().as_secs_f64() * 1000.0, 4096).expect("default capacity is valid")
    }

    /// Stamp capacity.
    pub fn stamp_capacity(&self) -> usize {
        self.stamp_capacity
    }

    /// Whether measurement is enabled.
    pub fn enabled(&self) -> bool {
        self.active
    }

    /// Enable or disable measurement; fails inside an active scope.
    pub fn set_enabled(&mut self, enabled: bool) -> Result<(), OmniTimerError> {
        if !self.stack.is_empty() {
            return Err(OmniTimerError::ActiveScope);
        }
        self.active = enabled;
        Ok(())
    }

    /// Open a named scope; no-op while disabled.
    pub fn push(&mut self, name: impl Into<String>) {
        if self.active {
            let start = (self.clock)();
            self.stack.push(ActiveTimer {
                name: name.into(),
                start,
                children: 0.0,
            });
        }
    }

    /// Close the innermost scope; no-op while disabled.
    pub fn pop(&mut self) -> Result<(), OmniTimerError> {
        if !self.active {
            return Ok(());
        }
        let item = self.stack.pop().ok_or(OmniTimerError::UnbalancedPop)?;
        let elapsed = ((self.clock)() - item.start).max(0.0);
        if let Some(parent) = self.stack.last_mut() {
            parent.children += elapsed;
        }
        let total = self.totals.entry(item.name).or_default();
        total.calls += 1;
        total.total_milliseconds += elapsed;
        total.self_milliseconds += (elapsed - item.children).max(0.0);
        total.maximum_milliseconds = total.maximum_milliseconds.max(elapsed);
        Ok(())
    }

    /// Run `operation` inside a named scope; direct call while disabled.
    pub fn measure<T>(&mut self, name: impl Into<String>, operation: impl FnOnce() -> T) -> Result<T, OmniTimerError> {
        if !self.active {
            return Ok(operation());
        }
        self.push(name);
        let value = operation();
        self.pop()?;
        Ok(value)
    }

    /// Record a stamp; no-op while disabled; evicts the oldest at capacity.
    pub fn stamp(&mut self, name: impl Into<String>) {
        if !self.active {
            return;
        }
        if self.stamps.len() == self.stamp_capacity {
            self.stamps.remove(0);
        }
        let milliseconds = (self.clock)() - self.epoch;
        self.stamps.push(TimerStamp {
            name: name.into(),
            milliseconds,
        });
    }

    /// Aggregated per-name report rows.
    pub fn report(&self) -> Vec<TimerReport> {
        self.totals
            .iter()
            .map(|(name, total)| TimerReport {
                name: name.clone(),
                calls: total.calls,
                total_milliseconds: total.total_milliseconds,
                self_milliseconds: total.self_milliseconds,
                maximum_milliseconds: total.maximum_milliseconds,
            })
            .collect()
    }

    /// Copy of the stamp list.
    pub fn stamp_list(&self) -> Vec<TimerStamp> {
        self.stamps.clone()
    }

    /// Clear totals and stamps, rebase the epoch; fails inside an active scope.
    pub fn reset(&mut self) -> Result<(), OmniTimerError> {
        if !self.stack.is_empty() {
            return Err(OmniTimerError::ActiveScope);
        }
        self.totals.clear();
        self.stamps.clear();
        self.epoch = (self.clock)();
        Ok(())
    }

    /// Tab-separated report matching the donor's `format()`.
    pub fn format(&self) -> String {
        let mut out = String::from("name\tcalls\ttotal_ms\tself_ms\tmax_ms\n");
        for row in self.report() {
            out.push_str(&format!(
                "{}\t{}\t{:.3}\t{:.3}\t{:.3}\n",
                row.name, row.calls, row.total_milliseconds, row.self_milliseconds, row.maximum_milliseconds
            ));
        }
        out
    }
}

impl Default for OmniTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for OmniTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OmniTimer")
            .field("stamp_capacity", &self.stamp_capacity)
            .field("stack", &self.stack)
            .field("stamps", &self.stamps)
            .field("epoch", &self.epoch)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn scripted(times: Vec<f64>) -> (Rc<Cell<usize>>, impl Fn() -> f64) {
        let cursor = Rc::new(Cell::new(0));
        let read = cursor.clone();
        let clock = move || {
            let index = read.get().min(times.len().saturating_sub(1));
            read.set(read.get() + 1);
            times[index]
        };
        (cursor, clock)
    }

    #[test]
    fn rejects_empty_capacity() {
        assert_eq!(
            OmniTimer::with_clock(|| 0.0f64, 0).unwrap_err(),
            OmniTimerError::InvalidCapacity
        );
    }

    #[test]
    fn inert_while_disabled() {
        let mut timer = OmniTimer::with_clock(|| 0.0f64, 4).expect("capacity");
        timer.push("a");
        timer.pop().expect("inert pop");
        timer.stamp("s");
        assert!(timer.report().is_empty());
        assert!(timer.stamp_list().is_empty());
    }

    #[test]
    fn aggregates_self_time() {
        let (_cursor, clock) = scripted(vec![0.0, 10.0, 12.0, 20.0, 30.0]);
        let mut timer = OmniTimer::with_clock(clock, 8).expect("capacity");
        timer.set_enabled(true).expect("enable");
        timer.push("outer");
        timer.push("inner");
        timer.pop().expect("inner");
        timer.pop().expect("outer");
        let report: HashMap<_, _> = timer.report().into_iter().map(|row| (row.name.clone(), row)).collect();
        assert_eq!(report["inner"].calls, 1);
        assert!((report["inner"].total_milliseconds - 8.0).abs() < f64::EPSILON);
        assert!((report["outer"].self_milliseconds - 12.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unbalanced_pop_errors() {
        let mut timer = OmniTimer::with_clock(|| 0.0f64, 4).expect("capacity");
        timer.set_enabled(true).expect("enable");
        assert_eq!(timer.pop().unwrap_err(), OmniTimerError::UnbalancedPop);
    }

    #[test]
    fn mode_change_inside_scope_errors() {
        let mut timer = OmniTimer::with_clock(|| 0.0f64, 4).expect("capacity");
        timer.set_enabled(true).expect("enable");
        timer.push("a");
        assert_eq!(timer.set_enabled(false).unwrap_err(), OmniTimerError::ActiveScope);
        assert_eq!(timer.reset().unwrap_err(), OmniTimerError::ActiveScope);
    }

    #[test]
    fn stamps_evict_oldest_at_capacity() {
        let (_cursor, clock) = scripted(vec![0.0, 1.0, 2.0, 3.0]);
        let mut timer = OmniTimer::with_clock(clock, 2).expect("capacity");
        timer.set_enabled(true).expect("enable");
        timer.stamp("a");
        timer.stamp("b");
        timer.stamp("c");
        let stamps = timer.stamp_list();
        assert_eq!(
            stamps.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
    }

    #[test]
    fn measure_runs_operation_and_records() {
        let (_cursor, clock) = scripted(vec![0.0, 5.0, 9.0]);
        let mut timer = OmniTimer::with_clock(clock, 4).expect("capacity");
        timer.set_enabled(true).expect("enable");
        let value = timer.measure("op", || 42).expect("measure");
        assert_eq!(value, 42);
        assert_eq!(timer.report().len(), 1);
    }

    #[test]
    fn format_matches_donor_shape() {
        let (_cursor, clock) = scripted(vec![0.0, 1.0, 3.0]);
        let mut timer = OmniTimer::with_clock(clock, 4).expect("capacity");
        timer.set_enabled(true).expect("enable");
        timer.measure("op", || {}).expect("measure");
        let text = timer.format();
        assert!(text.starts_with("name\tcalls\ttotal_ms\tself_ms\tmax_ms\n"));
        assert!(text.contains("op\t1\t2.000"));
    }
}
