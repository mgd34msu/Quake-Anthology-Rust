//! QuakeWorld clear-time pacing and ordered work ported from
//! `src/network/common/scheduling.ts`.

use thiserror::Error;

/// Error for invalid pacing and scheduling parameters.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SchedulingError {
    /// Network rate must be positive.
    #[error("Network rate must be positive")]
    BadRate,
    /// Packet time is not finite.
    #[error("Invalid packet time")]
    BadTime,
}

/// Byte-rate pacing (`PacketRate`).
#[derive(Debug, Clone)]
pub struct PacketRate {
    /// Bytes per second.
    pub bytes_per_second: f64,
    backup_bytes: f64,
    clear_time: f64,
}

impl PacketRate {
    /// Create a rate limiter with the donor default of 200 backup bytes.
    pub fn new(bytes_per_second: f64) -> Result<Self, SchedulingError> {
        Self::with_backup(bytes_per_second, 200.0)
    }

    /// Create a rate limiter with `backup_bytes` of headroom.
    pub fn with_backup(bytes_per_second: f64, backup_bytes: f64) -> Result<Self, SchedulingError> {
        if !bytes_per_second.is_finite() || bytes_per_second <= 0.0 {
            return Err(SchedulingError::BadRate);
        }
        Ok(Self {
            bytes_per_second,
            backup_bytes,
            clear_time: 0.0,
        })
    }

    /// True when a packet may be sent now (`canSend`).
    #[must_use]
    pub fn can_send(&self, now_milliseconds: f64, paused: bool) -> bool {
        paused || self.clear_time < now_milliseconds + self.backup_bytes * 1000.0 / self.bytes_per_second
    }

    /// Record a send of `bytes` (`sent`).
    pub fn sent(&mut self, bytes: usize, now_milliseconds: f64, paused: bool) {
        self.clear_time = if paused {
            now_milliseconds
        } else {
            self.clear_time.max(now_milliseconds) + bytes as f64 * 1000.0 / self.bytes_per_second
        };
    }

    /// Next send time in milliseconds.
    #[must_use]
    pub fn next_send_milliseconds(&self) -> f64 {
        self.clear_time
    }

    /// Reset pacing.
    pub fn reset(&mut self) {
        self.clear_time = 0.0;
    }
}

#[derive(Debug)]
struct ScheduledPacket<T> {
    sequence: u64,
    due: f64,
    value: T,
}

/// Ordered deferred work (`PacketScheduler`).
#[derive(Debug, Default)]
pub struct PacketScheduler<T> {
    pending: Vec<ScheduledPacket<T>>,
    sequence: u64,
}

impl<T> PacketScheduler<T> {
    /// Create an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            sequence: 0,
        }
    }

    /// Schedule `value` for `due_milliseconds`; returns a cancel token.
    pub fn schedule(&mut self, due_milliseconds: f64, value: T) -> Result<u64, SchedulingError> {
        if !due_milliseconds.is_finite() {
            return Err(SchedulingError::BadTime);
        }
        let sequence = self.sequence;
        self.sequence += 1;
        self.pending.push(ScheduledPacket {
            sequence,
            due: due_milliseconds,
            value,
        });
        self.pending.sort_by(|left, right| {
            left.due
                .partial_cmp(&right.due)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.sequence.cmp(&right.sequence))
        });
        Ok(sequence)
    }

    /// Cancel a scheduled packet.
    pub fn cancel(&mut self, sequence: u64) -> bool {
        let Some(index) = self.pending.iter().position(|value| value.sequence == sequence) else {
            return false;
        };
        self.pending.remove(index);
        true
    }

    /// Deliver every packet due at `now_milliseconds` (`drain`).
    pub fn drain(&mut self, now_milliseconds: f64, mut deliver: impl FnMut(T)) {
        while self.pending.first().is_some_and(|first| first.due <= now_milliseconds) {
            let first = self.pending.remove(0);
            deliver(first.value);
        }
    }

    /// Drop every scheduled packet.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// Number of scheduled packets.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// True when no packet is scheduled.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_paces_sends() {
        let mut rate = PacketRate::new(1000.0).unwrap();
        assert!(rate.can_send(0.0, false));
        rate.sent(1000, 0.0, false);
        assert!(!rate.can_send(0.0, false));
        assert!(rate.can_send(0.0, true));
        assert!(rate.can_send(900.0, false));
    }

    #[test]
    fn scheduler_delivers_in_due_order() {
        let mut scheduler = PacketScheduler::new();
        scheduler.schedule(10.0, "b").unwrap();
        scheduler.schedule(5.0, "a").unwrap();
        let cancelled = scheduler.schedule(7.0, "c").unwrap();
        assert!(scheduler.cancel(cancelled));
        let mut delivered = Vec::new();
        scheduler.drain(10.0, |value| delivered.push(value));
        assert_eq!(delivered, vec!["a", "b"]);
        assert!(scheduler.is_empty());
    }
}
