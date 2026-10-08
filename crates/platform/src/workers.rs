//! Load-owned workers with deterministic partitions of borrowed job slices.
//!
//! Jobs and the erased batch descriptor remain on the dispatcher's stack. The
//! completion barrier finishes every worker before that stack can be released,
//! including during unwinding. Each job belongs exclusively to one partition.
//! No jobs are boxed or queued, and dispatch never creates a thread.
//!
//! Job panics are scoped errors in unwind builds. The workspace's release
//! panic=abort profile terminates the process before Rust can catch a panic.

use std::{
    any::Any,
    fmt,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Condvar, Mutex, MutexGuard},
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
            Self::JobPanicked => "a worker job panicked",
            Self::WorkerStopped => "a worker stopped unexpectedly",
            Self::CountBuffer => "worker allocation count buffer has the wrong length",
        })
    }
}

impl std::error::Error for WorkerError {}

#[derive(Clone, Copy)]
struct Dispatch {
    context: *const (),
    run: unsafe fn(*const (), usize, usize) -> bool,
}

// SAFETY: dispatch_scoped installs this descriptor only while its stack Batch
// and borrowed jobs are alive. The function accepts only Send jobs and grants
// each worker disjoint indices. InFlight drains all workers before the borrow
// returns, and clears the descriptor under the same publication mutex.
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
    ready: Condvar,
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
        if count == 0 || count > MAX_WORKERS {
            return Err(WorkerError::InvalidCount);
        }
        let mut threads = Vec::new();
        threads
            .try_reserve_exact(count)
            .map_err(|_| WorkerError::LoadFailed)?;
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
            ready: Condvar::new(),
            complete: Condvar::new(),
        });
        let mut workers = Self { shared, threads };
        for index in 0..count {
            let shared = Arc::clone(&workers.shared);
            let thread = thread::Builder::new()
                .name(format!("qa-worker-{index}"))
                .spawn(move || worker_main(shared, index, count))
                .map_err(|_| WorkerError::LoadFailed)?;
            workers.threads.push(thread);
        }
        Ok(workers)
    }

    pub fn count(&self) -> usize {
        self.threads.len()
    }

    /// Partition jobs into contiguous ranges by worker index. Remainder jobs
    /// belong to the first workers; completion order never changes assignment.
    /// Every job is attempted even when another job unwinds.
    pub fn dispatch_scoped<J: Send>(
        &mut self,
        jobs: &mut [J],
        work: fn(&mut J),
    ) -> Result<(), WorkerError> {
        let batch = Batch {
            jobs: jobs.as_mut_ptr(),
            len: jobs.len(),
            work,
        };
        // Declaration order matters: the guard drains workers before Batch is
        // dropped. Any state mutex guard is newer and unlocks before draining.
        let mut scope = InFlight {
            shared: &self.shared,
            active: false,
        };
        {
            let mut state = self.shared.lock();
            if state.worker_failed || state.stopping {
                return Err(WorkerError::WorkerStopped);
            }
            if jobs.is_empty() {
                #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
                state.counts.fill(Counts::default());
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
            self.shared.ready.notify_all();
        }
        scope.finish()
    }

    /// Development counters cover each worker's wait/wake, job execution and
    /// completion-barrier update. The caller must install CountingAllocator;
    /// measure the dispatching thread separately. Native heap work is excluded.
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
            self.shared.ready.notify_all();
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
}

unsafe fn run_batch<J: Send>(context: *const (), index: usize, count: usize) -> bool {
    // SAFETY: publication is protected by the state mutex. The dispatcher's
    // InFlight guard keeps this immutable Batch and all J values alive until
    // every worker has completed; only individual job values are mutated.
    let batch = unsafe { &*context.cast::<Batch<J>>() };
    let quotient = batch.len / count;
    let remainder = batch.len % count;
    let start = quotient * index + index.min(remainder);
    let end = start + quotient + usize::from(index < remainder);
    let mut panicked = false;
    for job in start..end {
        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: these ranges are disjoint, within len, and visited once.
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

fn worker_main(shared: Arc<Shared>, index: usize, count: usize) {
    let mut seen_epoch = 0;
    loop {
        #[cfg(any(debug_assertions, feature = "allocation-tracking"))]
        allocations::begin_frame();
        let dispatch = {
            let mut state = shared.lock();
            while !state.stopping && state.epoch == seen_epoch {
                state = shared.wait(&shared.ready, state);
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
            completion.panicked = unsafe { (dispatch.run)(dispatch.context, index, count) };
        }
    }
}
