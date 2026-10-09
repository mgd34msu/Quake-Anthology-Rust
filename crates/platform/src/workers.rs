//! Load-owned workers with bounded exclusive claims over borrowed job slices.
//!
//! Jobs and the erased batch descriptor remain on the dispatcher's stack. The
//! completion barrier finishes every worker before that stack can be released,
//! including during unwinding. The caller and workers claim each job once.
//! No jobs are boxed or queued, and dispatch never creates a thread.
//!
//! Job panics are scoped errors in unwind builds. The workspace's release
//! panic=abort profile terminates the process before Rust can catch a panic.

use std::{
    any::Any,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
use crate::allocations::{self, Counts};

/// Cold capacity bound for persistent pools, including 1/2/4/8 raster bands.
pub const MAX_WORKERS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    InvalidCount,
    LoadFailed,
    JobPanicked,
    WorkerStopped,
    CountBuffer,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCount => "invalid worker count",
            Self::LoadFailed => "worker startup failed",
            Self::JobPanicked => "a dispatched job panicked",
            Self::WorkerStopped => "a worker stopped unexpectedly",
            Self::CountBuffer => "worker allocation count buffer has the wrong length",
        })
    }
}

impl std::error::Error for WorkerError {}

#[derive(Clone, Copy)]
struct Dispatch {
    context: *const (),
    run: unsafe fn(*const ()) -> bool,
}

// SAFETY: dispatch_scoped installs this descriptor only while its stack Batch
// and borrowed jobs are alive. The function accepts only Send jobs and grants
// each participant an exclusive atomic job claim. InFlight drains all workers
// before the borrow returns, then clears the descriptor under the same mutex.
unsafe impl Send for Dispatch {}

struct State {
    epoch: u64,
    dispatch: Option<Dispatch>,
    remaining: usize,
    panicked: bool,
    worker_failed: bool,
    stopping: bool,
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    counts: Box<[Counts]>,
}

struct Shared {
    state: Mutex<State>,
    ready: Box<[Condvar]>,
    complete: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wait<'a>(&self, condition: &Condvar, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        condition
            .wait(state)
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub struct Workers {
    shared: Arc<Shared>,
    threads: Vec<JoinHandle<()>>,
}

impl Workers {
    pub fn load(count: usize) -> Result<Self, WorkerError> {
        if count > MAX_WORKERS {
            return Err(WorkerError::InvalidCount);
        }
        let mut threads = Vec::new();
        threads
            .try_reserve_exact(count)
            .map_err(|_| WorkerError::LoadFailed)?;
        let ready = {
            let mut ready = Vec::new();
            ready
                .try_reserve_exact(count)
                .map_err(|_| WorkerError::LoadFailed)?;
            ready.resize_with(count, Condvar::new);
            ready.into_boxed_slice()
        };
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        let counts = {
            let mut counts = Vec::new();
            counts
                .try_reserve_exact(count)
                .map_err(|_| WorkerError::LoadFailed)?;
            counts.resize(count, Counts::default());
            counts.into_boxed_slice()
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                epoch: 0,
                dispatch: None,
                remaining: 0,
                panicked: false,
                worker_failed: false,
                stopping: false,
                #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
                counts,
            }),
            ready,
            complete: Condvar::new(),
        });
        let mut workers = Self { shared, threads };
        for index in 0..count {
            let shared = Arc::clone(&workers.shared);
            let thread = thread::Builder::new()
                .name(format!("qa-worker-{index}"))
                .spawn(move || worker_main(shared, index))
                .map_err(|_| WorkerError::LoadFailed)?;
            workers.threads.push(thread);
        }
        Ok(workers)
    }

    pub fn count(&self) -> usize {
        self.threads.len()
    }

    /// The caller and persistent workers claim exclusive job indices until the
    /// borrowed batch is exhausted. Every job is attempted even when another
    /// job unwinds. With zero background workers the caller runs the whole batch.
    pub fn dispatch_scoped<J: Send>(
        &mut self,
        jobs: &mut [J],
        work: fn(&mut J),
    ) -> Result<(), WorkerError> {
        let batch = Batch {
            jobs: jobs.as_mut_ptr(),
            len: jobs.len(),
            work,
            next: AtomicUsize::new(0),
        };
        // Declaration order matters: the guard drains workers before Batch is
        // dropped. Any state mutex guard is newer and unlocks before draining.
        let mut scope = InFlight {
            shared: &self.shared,
            active: false,
        };
        {
            let mut state = self.shared.lock();
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            state.counts.fill(Counts::default());
            if state.worker_failed || state.stopping {
                return Err(WorkerError::WorkerStopped);
            }
            if jobs.is_empty() {
                return Ok(());
            }
            state.dispatch = Some(Dispatch {
                context: (&batch as *const Batch<J>).cast(),
                run: run_batch::<J>,
            });
            state.remaining = self.count();
            state.panicked = false;
            state.epoch = state.epoch.wrapping_add(1);
            scope.active = true;
            for ready in &self.shared.ready {
                ready.notify_one();
            }
        }
        // SAFETY: the caller holds the same scoped Batch alive as the workers.
        // Its atomic claims grant exclusive job access through run_batch too.
        let panicked = unsafe { run_batch::<J>((&batch as *const Batch<J>).cast()) };
        self.shared.lock().panicked |= panicked;
        scope.finish()
    }

    /// Development counters cover each worker's wait/wake, job execution and
    /// completion-barrier update. The caller must install CountingAllocator;
    /// measure the dispatching thread separately. Native heap work is excluded.
    /// Only the latest dispatch is retained. Empty dispatches and dispatches
    /// rejected before execution clear it.
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    pub fn allocation_counts(&self, output: &mut [Counts]) -> Result<(), WorkerError> {
        if output.len() != self.count() {
            return Err(WorkerError::CountBuffer);
        }
        output.copy_from_slice(&self.shared.lock().counts);
        Ok(())
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        {
            let mut state = self.shared.lock();
            state.stopping = true;
            for ready in &self.shared.ready {
                ready.notify_one();
            }
        }
        for thread in self.threads.drain(..) {
            if let Err(payload) = thread.join() {
                discard_panic(payload);
            }
        }
    }
}

struct Batch<J> {
    jobs: *mut J,
    len: usize,
    work: fn(&mut J),
    next: AtomicUsize,
}

unsafe fn run_batch<J: Send>(context: *const ()) -> bool {
    // SAFETY: publication is protected by the state mutex. The dispatcher's
    // InFlight guard keeps this immutable Batch and all J values alive until
    // every worker has completed; the jobs pointer/length/function are immutable,
    // and next is atomic. Only exclusively claimed individual jobs are mutated.
    let batch = unsafe { &*context.cast::<Batch<J>>() };
    let mut panicked = false;
    // Relaxed ordering grants unique indices. The publication/completion mutex
    // orders initial job state and completed writes across the scoped borrow.
    while let Ok(job) = batch
        .next
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            if next < batch.len {
                Some(next + 1)
            } else {
                None
            }
        })
    {
        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: the bounded atomic claim grants this index once and never
            // advances past len, including at usize::MAX for zero-sized jobs.
            // J: Send permits exclusive access on another thread. No J reference
            // is stored after work returns or unwinds.
            (batch.work)(unsafe { &mut *batch.jobs.add(job) });
        }));
        if let Err(payload) = result {
            panicked = true;
            discard_panic(payload);
        }
    }
    panicked
}

fn discard_panic(payload: Box<dyn Any + Send>) {
    // Ordinary payloads are dropped. If a payload's own destructor panics,
    // discard that secondary payload without invoking another user destructor
    // so the completion barrier and remaining jobs are still reached.
    if let Err(secondary) = catch_unwind(AssertUnwindSafe(|| drop(payload))) {
        std::mem::forget(secondary);
    }
}

struct InFlight<'a> {
    shared: &'a Shared,
    active: bool,
}

impl InFlight<'_> {
    fn finish(&mut self) -> Result<(), WorkerError> {
        let mut state = self.shared.lock();
        while state.remaining != 0 {
            state = self.shared.wait(&self.shared.complete, state);
        }
        state.dispatch = None;
        self.active = false;
        if state.worker_failed {
            Err(WorkerError::WorkerStopped)
        } else if state.panicked {
            Err(WorkerError::JobPanicked)
        } else {
            Ok(())
        }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if self.active {
            let _ = self.finish();
        }
    }
}

struct Completion<'a> {
    shared: &'a Shared,
    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    index: usize,
    panicked: bool,
}

impl Drop for Completion<'_> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.panicked |= self.panicked;
        state.worker_failed |= thread::panicking();
        state.remaining -= 1;
        if state.remaining == 0 {
            self.shared.complete.notify_one();
        }
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        {
            state.counts[self.index] = allocations::end_frame();
        }
    }
}

fn worker_main(shared: Arc<Shared>, index: usize) {
    let mut seen_epoch = 0;
    loop {
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        allocations::begin_frame();
        let dispatch = {
            let mut state = shared.lock();
            while !state.stopping && state.epoch == seen_epoch {
                state = shared.wait(&shared.ready[index], state);
            }
            if state.stopping {
                #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
                let _ = allocations::end_frame();
                return;
            }
            seen_epoch = state.epoch;
            state.dispatch
        };
        let mut completion = Completion {
            shared: &shared,
            #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
            index,
            panicked: false,
        };
        if let Some(dispatch) = dispatch {
            // SAFETY: this worker has observed a newly published epoch. Its
            // Completion guard signals that it will never access this batch
            // again before InFlight lets the originating borrow return.
            completion.panicked = unsafe { (dispatch.run)(dispatch.context) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
    #[test]
    fn rejected_dispatch_clears_prior_counts_without_running_jobs() -> Result<(), WorkerError> {
        for worker_failed in [false, true] {
            let mut workers = Workers::load(1)?;
            let prior = Counts {
                allocations: 3,
                reallocations: 2,
                requested_bytes: 512,
            };
            {
                let mut state = workers.shared.lock();
                state.counts[0] = prior;
                state.worker_failed = worker_failed;
                state.stopping = !worker_failed;
            }
            let mut counts = [Counts::default()];
            workers.allocation_counts(&mut counts)?;
            assert_eq!(counts, [prior]);
            let mut jobs = [0u32];
            assert_eq!(
                workers.dispatch_scoped(&mut jobs, |job| *job += 1),
                Err(WorkerError::WorkerStopped)
            );
            workers.allocation_counts(&mut counts)?;
            assert_eq!(counts, [Counts::default()]);
            assert_eq!(jobs, [0]);
        }
        Ok(())
    }
}
