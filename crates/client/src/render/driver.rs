//! Headless synchronous driver over an ordered backend.
//!
//! The former `SyncDriver`/`ImageJournal` duplicate of the image journal was
//! removed: no production backend used it (every `is_worker()` returns false)
//! and its only references were its own unit tests. Ordered execution lives
//! in `execution` and the retained image journal in `image_journal`.
