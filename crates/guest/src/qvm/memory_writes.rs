//! Committed-store observation for one QVM allocation.
//!
//! Port of `src/compat/qvm/memory-writes.ts` (`QvmMemoryWrites`, `QvmWritableView`
//! and the write-event records). One allocation owns publication: watched ranges
//! capture before/after bytes around each committed store and deliver one event
//! per watch. Publication permits bookkeeping only, so stores inside a `publish`
//! callback fail; `after_publication` hooks run after delivery, optionally routed
//! through an effect callback (default: inline).
//!
//! Sync-port notes: the donor returns an unsubscribe closure from `observe` and
//! lets callbacks throw; here `observe` returns a watch id removed with
//! [`QvmMemoryWrites::unobserve`], and callbacks report
//! [`GuestError`](crate::error::GuestError). A single failing watch re-raises its
//! error; several failures join into one callback error (the donor raises an
//! `AggregateError`, which has no `GuestError` equivalent).

use std::collections::HashMap;

use crate::error::GuestError;

/// One watched byte range within the allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmWriteRange {
    /// Start offset in bytes.
    pub byte_offset: usize,
    /// Length in bytes.
    pub byte_length: usize,
}

/// Committed bytes for one watched overlap: before and after images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCommittedBytes {
    /// Start offset in bytes.
    pub byte_offset: usize,
    /// Bytes before the store.
    pub before: Vec<u8>,
    /// Bytes after the store.
    pub after: Vec<u8>,
}

/// One publication: a sequence number plus the committed ranges of one watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmCommittedWrite {
    /// Publication sequence (starts at 1).
    pub sequence: u64,
    /// Committed ranges for the receiving watch.
    pub ranges: Vec<QvmCommittedBytes>,
}

/// Captured pre-store bytes for one watch, held across the mutation.
#[derive(Debug)]
pub struct QvmWriteCapture {
    watch_id: u64,
    ranges: Vec<(usize, Vec<u8>)>,
}

type PublishCallback = Box<dyn FnMut(&QvmCommittedWrite) -> Result<(), GuestError>>;
type AfterCallback = Box<dyn FnMut(&QvmCommittedWrite) -> Result<(), GuestError>>;

struct Watch {
    ranges: Vec<QvmWriteRange>,
    publish: PublishCallback,
    after_publication: Option<AfterCallback>,
    active: bool,
}

/// Publication state for one allocation. The byte slice is supplied per call so
/// the owner keeps its own borrow of the allocation.
pub struct QvmMemoryWrites {
    watches: HashMap<u64, Watch>,
    next_id: u64,
    publishing: bool,
    closed: bool,
    sequence: u64,
    effect: Option<Box<dyn FnMut(&mut dyn FnMut())>>,
}

impl std::fmt::Debug for QvmMemoryWrites {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QvmMemoryWrites")
            .field("watches", &self.watches.len())
            .field("publishing", &self.publishing)
            .field("closed", &self.closed)
            .field("sequence", &self.sequence)
            .finish()
    }
}

impl Default for QvmMemoryWrites {
    fn default() -> Self {
        Self::new()
    }
}

impl QvmMemoryWrites {
    /// Fresh publication state with no watches.
    #[must_use]
    pub fn new() -> Self {
        Self {
            watches: HashMap::new(),
            next_id: 1,
            publishing: false,
            closed: false,
            sequence: 0,
            effect: None,
        }
    }

    /// Route `after_publication` hooks through `effect` (default: run inline).
    pub fn set_effect(&mut self, effect: Option<Box<dyn FnMut(&mut dyn FnMut())>>) {
        self.effect = effect;
    }

    /// Whether stores must capture (a watch exists, publication runs, or retired).
    #[must_use]
    pub fn intercepts(&self) -> bool {
        !self.watches.is_empty() || self.publishing || self.closed
    }

    /// Fail while a publication is in flight.
    pub fn assert_not_publishing(&self) -> Result<(), GuestError> {
        if self.publishing {
            return Err(GuestError::callback("QVM store publication permits bookkeeping only"));
        }
        Ok(())
    }

    /// Fail once the allocation is retired.
    pub fn assert_live(&self) -> Result<(), GuestError> {
        if self.closed {
            return Err(GuestError::callback("QVM memory has been retired"));
        }
        Ok(())
    }

    /// Fail while publishing or after retirement.
    pub fn assert_writable(&self) -> Result<(), GuestError> {
        self.assert_not_publishing()?;
        self.assert_live()
    }

    /// Fail when `offset..offset+len` exceeds a `total`-byte allocation.
    pub fn check_range(&self, total: usize, offset: usize, len: usize) -> Result<(), GuestError> {
        if offset > total || len > total - offset {
            return Err(GuestError::memory_fault(
                "out-of-bounds",
                offset as u64,
                len,
                "access",
                "QVM raw memory range exceeds allocation",
            ));
        }
        Ok(())
    }

    /// Watch `ranges`, returning a watch id for [`Self::unobserve`].
    /// Empty ranges drop out; overlapping ranges merge.
    pub fn observe(
        &mut self,
        total: usize,
        ranges: &[QvmWriteRange],
        publish: PublishCallback,
        after_publication: Option<AfterCallback>,
    ) -> Result<u64, GuestError> {
        self.assert_live()?;
        let mut sorted: Vec<QvmWriteRange> = Vec::with_capacity(ranges.len());
        for range in ranges {
            self.check_range(total, range.byte_offset, range.byte_length)?;
            if range.byte_length != 0 {
                sorted.push(*range);
            }
        }
        sorted.sort_by_key(|range| range.byte_offset);
        let mut merged: Vec<QvmWriteRange> = Vec::with_capacity(sorted.len());
        for range in sorted {
            if let Some(previous) = merged.last_mut() {
                if previous.byte_offset + previous.byte_length >= range.byte_offset {
                    let end = (previous.byte_offset + previous.byte_length).max(range.byte_offset + range.byte_length);
                    previous.byte_length = end - previous.byte_offset;
                    continue;
                }
            }
            merged.push(range);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.watches.insert(
            id,
            Watch {
                ranges: merged,
                publish,
                after_publication,
                active: true,
            },
        );
        Ok(id)
    }

    /// Remove the watch installed by [`Self::observe`]. Unknown ids are a no-op.
    pub fn unobserve(&mut self, id: u64) {
        if let Some(watch) = self.watches.get_mut(&id) {
            watch.active = false;
        }
        self.watches.remove(&id);
    }

    /// Capture pre-store bytes for every watch overlapping `offset..offset+len`.
    /// Returns `None` without allocation when no watch overlaps.
    pub fn before(&self, bytes: &[u8], offset: usize, len: usize) -> Result<Option<Vec<QvmWriteCapture>>, GuestError> {
        self.assert_writable()?;
        self.check_range(bytes.len(), offset, len)?;
        let mut captures: Option<Vec<QvmWriteCapture>> = None;
        let mut ids: Vec<u64> = self.watches.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let watch = &self.watches[&id];
            let mut ranges = Vec::new();
            for range in &watch.ranges {
                let start = offset.max(range.byte_offset);
                let end = (offset + len).min(range.byte_offset + range.byte_length);
                if start < end {
                    ranges.push((start, bytes[start..end].to_vec()));
                }
            }
            if !ranges.is_empty() {
                captures
                    .get_or_insert_with(Vec::new)
                    .push(QvmWriteCapture { watch_id: id, ranges });
            }
        }
        Ok(captures)
    }

    /// Publish one event per captured watch, then run `after_publication` hooks.
    pub fn after(&mut self, bytes: &[u8], captures: Option<Vec<QvmWriteCapture>>) -> Result<(), GuestError> {
        let Some(captures) = captures else {
            return Ok(());
        };
        self.sequence += 1;
        let sequence = self.sequence;
        let mut deliveries: Vec<(u64, QvmCommittedWrite)> = Vec::with_capacity(captures.len());
        for capture in &captures {
            let mut ranges = Vec::with_capacity(capture.ranges.len());
            for (start, before) in &capture.ranges {
                let end = start + before.len();
                ranges.push(QvmCommittedBytes {
                    byte_offset: *start,
                    before: before.clone(),
                    after: bytes[*start..end].to_vec(),
                });
            }
            deliveries.push((capture.watch_id, QvmCommittedWrite { sequence, ranges }));
        }
        let mut errors: Vec<GuestError> = Vec::new();
        self.publishing = true;
        for (id, event) in &deliveries {
            if let Some(watch) = self.watches.get_mut(id) {
                if watch.active {
                    if let Err(error) = (watch.publish)(event) {
                        errors.push(error);
                    }
                }
            }
        }
        self.publishing = false;
        if errors.len() == 1 {
            return Err(errors
                .pop()
                .unwrap_or_else(|| GuestError::callback("QVM store publication failed")));
        }
        if !errors.is_empty() {
            let detail = errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ");
            return Err(GuestError::callback(format!(
                "QVM committed store publication failed: {detail}"
            )));
        }
        for (id, event) in &deliveries {
            let active = self
                .watches
                .get(id)
                .is_some_and(|watch| watch.active && watch.after_publication.is_some());
            if active && !self.closed {
                let mut run = |after: &mut AfterCallback| -> Result<(), GuestError> {
                    if let Some(effect) = self.effect.as_mut() {
                        let mut pending: Option<GuestError> = None;
                        let mut perform = || {
                            if let Err(error) = after(event) {
                                pending = Some(error);
                            }
                        };
                        effect(&mut perform);
                        if let Some(error) = pending {
                            return Err(error);
                        }
                        Ok(())
                    } else {
                        after(event)
                    }
                };
                // Borrow dance: take the callback out, run it, put it back.
                let mut taken: Option<AfterCallback> = None;
                if let Some(watch) = self.watches.get_mut(id) {
                    taken = watch.after_publication.take();
                }
                if let Some(mut after) = taken {
                    let result = run(&mut after);
                    if let Some(watch) = self.watches.get_mut(id) {
                        watch.after_publication = Some(after);
                    }
                    result?;
                }
            }
        }
        Ok(())
    }

    /// Remove every watch. Fails while publishing.
    pub fn clear(&mut self) -> Result<(), GuestError> {
        self.assert_not_publishing()?;
        self.watches.clear();
        Ok(())
    }

    /// Retire the allocation: clear watches and reject later stores.
    pub fn close(&mut self) {
        self.watches.clear();
        self.closed = true;
    }
}

/// Scalar view over a live allocation range with committed-store publication.
///
/// The donor defines this beside the publication state; here the struct lives
/// in [`crate::qvm::memory`] (it borrows the shared backing) and is
/// re-exported so the export surface matches the donor barrel.
pub use super::memory::QvmWritableView;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn before_returns_none_without_watches() {
        let writes = QvmMemoryWrites::new();
        assert!(!writes.intercepts());
        let bytes = vec![0u8; 16];
        assert!(writes.before(&bytes, 0, 16).unwrap().is_none());
    }

    #[test]
    fn overlapping_ranges_merge_and_publish() {
        let mut writes = QvmMemoryWrites::new();
        let seen: Rc<RefCell<Vec<QvmCommittedWrite>>> = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = Rc::clone(&seen);
        writes
            .observe(
                16,
                &[
                    QvmWriteRange {
                        byte_offset: 0,
                        byte_length: 8,
                    },
                    QvmWriteRange {
                        byte_offset: 4,
                        byte_length: 8,
                    },
                ],
                Box::new(move |event| {
                    seen_clone.borrow_mut().push(event.clone());
                    Ok(())
                }),
                None,
            )
            .unwrap();
        let mut bytes = vec![0u8; 16];
        let captures = writes.before(&bytes, 2, 4).unwrap();
        bytes[2..6].copy_from_slice(&[1, 2, 3, 4]);
        writes.after(&bytes, captures).unwrap();
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].sequence, 1);
        assert_eq!(seen[0].ranges.len(), 1);
        assert_eq!(seen[0].ranges[0].byte_offset, 2);
        assert_eq!(seen[0].ranges[0].before, vec![0, 0, 0, 0]);
        assert_eq!(seen[0].ranges[0].after, vec![1, 2, 3, 4]);
    }

    #[test]
    fn unobserved_stores_skip_watches() {
        let mut writes = QvmMemoryWrites::new();
        let id = writes
            .observe(
                16,
                &[QvmWriteRange {
                    byte_offset: 0,
                    byte_length: 4,
                }],
                Box::new(|_| Ok(())),
                None,
            )
            .unwrap();
        writes.unobserve(id);
        assert!(!writes.intercepts());
    }

    #[test]
    fn stores_reject_closed_allocations() {
        let mut writes = QvmMemoryWrites::new();
        writes.close();
        let bytes = vec![0u8; 8];
        assert!(writes.before(&bytes, 0, 1).is_err());
        assert!(writes.observe(8, &[], Box::new(|_| Ok(())), None).is_err());
    }

    #[test]
    fn single_publish_failure_reraises() {
        let mut writes = QvmMemoryWrites::new();
        writes
            .observe(
                8,
                &[QvmWriteRange {
                    byte_offset: 0,
                    byte_length: 8,
                }],
                Box::new(|_| Err(GuestError::callback("boom"))),
                None,
            )
            .unwrap();
        let bytes = vec![0u8; 8];
        let captures = writes.before(&bytes, 0, 8).unwrap();
        let error = writes.after(&bytes, captures).unwrap_err();
        assert_eq!(error.to_string(), "boom");
    }

    #[test]
    fn after_publication_runs_through_effect() {
        let mut writes = QvmMemoryWrites::new();
        let order: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let order_publish = Rc::clone(&order);
        let order_after = Rc::clone(&order);
        let order_effect = Rc::clone(&order);
        writes
            .observe(
                8,
                &[QvmWriteRange {
                    byte_offset: 0,
                    byte_length: 8,
                }],
                Box::new(move |_| {
                    order_publish.borrow_mut().push("publish");
                    Ok(())
                }),
                Some(Box::new(move |_| {
                    order_after.borrow_mut().push("after");
                    Ok(())
                })),
            )
            .unwrap();
        writes.set_effect(Some(Box::new(move |perform| {
            order_effect.borrow_mut().push("effect");
            perform();
        })));
        let bytes = vec![0u8; 8];
        let captures = writes.before(&bytes, 0, 8).unwrap();
        writes.after(&bytes, captures).unwrap();
        assert_eq!(*order.borrow(), vec!["publish", "effect", "after"]);
    }
}
