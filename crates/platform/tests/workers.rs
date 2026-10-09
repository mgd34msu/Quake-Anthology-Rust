use qa_platform::{MAX_WORKERS, WorkerError, Workers};
use std::{
    cell::RefCell,
    sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, ThreadId},
};

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Row<'a> {
    output: &'a mut [u32],
    value: u32,
}

fn fill_row(job: &mut Row<'_>) {
    job.output.fill(job.value);
}

#[test]
fn short_lived_disjoint_rows_can_be_borrowed_and_reused_after_dispatch() {
    let mut workers = Workers::load(3).unwrap();
    for frame in 1..=4 {
        let mut pixels = [[0u32; 9]; 7];
        let mut jobs: Vec<_> = pixels
            .iter_mut()
            .enumerate()
            .map(|(index, output)| Row {
                output,
                value: frame * 100 + index as u32,
            })
            .collect();
        workers.dispatch_scoped(&mut jobs, fill_row).unwrap();
        drop(jobs);
        for (index, row) in pixels.iter().enumerate() {
            assert_eq!(row, &[frame * 100 + index as u32; 9]);
        }
        pixels.fill([0; 9]);
        assert_eq!(pixels, [[0; 9]; 7]);
    }
}

struct ThreadJob<'a> {
    output: &'a mut Option<ThreadId>,
    visits: &'a AtomicUsize,
    gate: Option<&'a Barrier>,
}

fn identify_worker(job: &mut ThreadJob<'_>) {
    *job.output = Some(thread::current().id());
    job.visits.fetch_add(1, Ordering::SeqCst);
    if let Some(gate) = job.gate {
        gate.wait();
    }
}

#[test]
fn caller_and_every_worker_claim_once_with_more_and_fewer_jobs_than_participants() {
    let mut workers = Workers::load(3).unwrap();
    assert_eq!(workers.count(), 3);
    let caller = thread::current().id();
    let gate = Barrier::new(4);
    let visits: [AtomicUsize; 8] = std::array::from_fn(|_| AtomicUsize::new(0));
    let mut large = [None; 8];
    {
        let mut jobs: Vec<_> = large
            .iter_mut()
            .enumerate()
            .map(|(index, output)| ThreadJob {
                output,
                visits: &visits[index],
                gate: (index < 4).then_some(&gate),
            })
            .collect();
        workers.dispatch_scoped(&mut jobs, identify_worker).unwrap();
    }
    assert!(large.iter().all(Option::is_some));
    for (index, id) in large[..4].iter().enumerate() {
        assert!(!large[..index].contains(id));
    }
    assert!(large[..4].contains(&Some(caller)));
    assert!(large[4..].iter().all(|id| large[..4].contains(id)));
    assert!(visits.iter().all(|count| count.load(Ordering::SeqCst) == 1));
    let small_visits: [AtomicUsize; 2] = std::array::from_fn(|_| AtomicUsize::new(0));
    let mut small = [None; 2];
    {
        let mut jobs: Vec<_> = small
            .iter_mut()
            .enumerate()
            .map(|(index, output)| ThreadJob {
                output,
                visits: &small_visits[index],
                gate: None,
            })
            .collect();
        workers.dispatch_scoped(&mut jobs, identify_worker).unwrap();
    }
    assert!(small.iter().all(|id| large[..4].contains(id)));
    assert!(
        small_visits
            .iter()
            .all(|count| count.load(Ordering::SeqCst) == 1)
    );
    let mut again = [None; 8];
    {
        let mut jobs: Vec<_> = again
            .iter_mut()
            .enumerate()
            .map(|(index, output)| ThreadJob {
                output,
                visits: &visits[index],
                gate: (index < 4).then_some(&gate),
            })
            .collect();
        workers.dispatch_scoped(&mut jobs, identify_worker).unwrap();
    }
    assert!(again.iter().all(|id| large[..4].contains(id)));
    assert!(visits.iter().all(|count| count.load(Ordering::SeqCst) == 2));
    workers
        .dispatch_scoped::<u8>(&mut [], |value| *value += 1)
        .unwrap();
}

#[test]
fn zero_background_workers_run_the_same_borrowed_batch_on_the_caller() {
    let mut workers = Workers::load(0).unwrap();
    assert_eq!(workers.count(), 0);
    let caller = thread::current().id();
    let visits: [AtomicUsize; 3] = std::array::from_fn(|_| AtomicUsize::new(0));
    let mut outputs = [None; 3];
    {
        let mut jobs: Vec<_> = outputs
            .iter_mut()
            .enumerate()
            .map(|(index, output)| ThreadJob {
                output,
                visits: &visits[index],
                gate: None,
            })
            .collect();
        workers.dispatch_scoped(&mut jobs, identify_worker).unwrap();
    }
    assert_eq!(outputs, [Some(caller); 3]);
    assert!(visits.iter().all(|count| count.load(Ordering::SeqCst) == 1));
    workers
        .dispatch_scoped::<u8>(&mut [], |job| *job += 1)
        .unwrap();
}

struct PanicJob<'a> {
    output: &'a mut u32,
    gate: Option<&'a Barrier>,
    started: &'a AtomicUsize,
    finished: &'a AtomicUsize,
    panic: bool,
}

fn sometimes_panic(job: &mut PanicJob<'_>) {
    job.started.fetch_add(1, Ordering::SeqCst);
    if let Some(gate) = job.gate {
        gate.wait();
    }
    if job.panic {
        panic!("scoped worker fixture");
    }
    *job.output += 1;
    job.finished.fetch_add(1, Ordering::SeqCst);
}

#[test]
#[expect(
    clippy::drop_non_drop,
    reason = "The fixture explicitly ends borrowed job storage before inspecting completed outputs and counts"
)]
fn a_panicking_job_still_waits_for_other_workers_and_attempts_later_jobs() {
    let mut workers = Workers::load(2).unwrap();
    let gate = Barrier::new(3);
    let started = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let mut output = [0u32; 4];
    {
        let mut jobs: Vec<_> = output
            .iter_mut()
            .enumerate()
            .map(|(index, output)| PanicJob {
                output,
                // Caller and both workers start a job before one can panic.
                gate: (index < 3).then_some(&gate),
                started: &started,
                finished: &finished,
                panic: index == 0,
            })
            .collect();
        assert_eq!(
            workers.dispatch_scoped(&mut jobs, sometimes_panic),
            Err(WorkerError::JobPanicked)
        );
    }
    assert_eq!(started.load(Ordering::SeqCst), 4);
    assert_eq!(finished.load(Ordering::SeqCst), 3);
    assert_eq!(output, [0, 1, 1, 1]);
    // Reuse the now-returned borrow and the same persistent pool immediately.
    let mut next = output.map(|_| [0u32; 1]);
    let mut jobs = next.each_mut().map(|output| Row { output, value: 7 });
    workers.dispatch_scoped(&mut jobs, fill_row).unwrap();
    drop(jobs);
    assert_eq!(next, [[7]; 4]);
}

#[test]
fn a_serial_panicking_job_returns_the_borrow_and_attempts_later_jobs() {
    let mut workers = Workers::load(0).unwrap();
    let started = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let mut output = [0u32; 3];
    {
        let mut jobs: Vec<_> = output
            .iter_mut()
            .enumerate()
            .map(|(index, output)| PanicJob {
                output,
                gate: None,
                started: &started,
                finished: &finished,
                panic: index == 0,
            })
            .collect();
        assert_eq!(
            workers.dispatch_scoped(&mut jobs, sometimes_panic),
            Err(WorkerError::JobPanicked)
        );
    }
    assert_eq!(started.load(Ordering::SeqCst), 3);
    assert_eq!(finished.load(Ordering::SeqCst), 2);
    assert_eq!(output, [0, 1, 1]);
    workers
        .dispatch_scoped(&mut output, |value| *value += 1)
        .unwrap();
    assert_eq!(output, [1, 2, 2]);
}

struct ExitMark(Arc<AtomicUsize>);

impl Drop for ExitMark {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

thread_local! {
    static EXIT_MARK: RefCell<Option<ExitMark>> = const { RefCell::new(None) };
}

struct ExitJob<'a> {
    counter: &'a Arc<AtomicUsize>,
    gate: &'a Barrier,
    caller: ThreadId,
}

fn mark_owned_thread_exit(job: &mut ExitJob<'_>) {
    if thread::current().id() != job.caller {
        EXIT_MARK.with(|mark| *mark.borrow_mut() = Some(ExitMark(Arc::clone(job.counter))));
    }
    job.gate.wait();
}

#[test]
fn dropping_pool_joins_all_owned_workers_before_thread_local_teardown_returns() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut workers = Workers::load(3).unwrap();
    let gate = Barrier::new(4);
    let mut jobs = std::array::from_fn::<_, 4, _>(|_| ExitJob {
        counter: &counter,
        gate: &gate,
        caller: thread::current().id(),
    });
    workers
        .dispatch_scoped(&mut jobs, mark_owned_thread_exit)
        .unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    drop(workers);
    assert_eq!(counter.load(Ordering::SeqCst), 3);
}

#[test]
fn invalid_load_counts_return_scoped_errors() {
    for count in [MAX_WORKERS + 1, usize::MAX] {
        assert!(matches!(
            Workers::load(count),
            Err(WorkerError::InvalidCount)
        ));
    }
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
struct AllocationJob<'a> {
    output: &'a mut u64,
    positive_control: bool,
    gate: Option<&'a Barrier>,
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
fn allocation_job(job: &mut AllocationJob<'_>) {
    if let Some(gate) = job.gate {
        gate.wait();
    }
    if job.positive_control {
        std::hint::black_box(vec![19u8; 64]);
    }
    *job.output += 1;
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[test]
#[expect(
    clippy::drop_non_drop,
    reason = "The fixture explicitly ends borrowed job storage before inspecting completed outputs and counts"
)]
fn dispatch_counts_the_caller_and_every_worker_with_positive_controls() {
    use qa_platform::allocations::{Counts, begin_frame, end_frame};
    let mut workers = Workers::load(4).unwrap();
    let gate = Barrier::new(5);
    let mut outputs = [0u64; 5];
    let mut jobs = outputs.each_mut().map(|output| AllocationJob {
        output,
        positive_control: false,
        gate: Some(&gate),
    });
    for _ in 0..8 {
        workers.dispatch_scoped(&mut jobs, allocation_job).unwrap();
    }
    let mut counts = [Counts::default(); 4];
    for job in &mut jobs {
        job.positive_control = true;
    }
    begin_frame();
    let positive_result = workers.dispatch_scoped(&mut jobs, allocation_job);
    let positive_caller = end_frame();
    assert_eq!(positive_result, Ok(()));
    assert_eq!(
        positive_caller,
        Counts {
            allocations: 1,
            reallocations: 0,
            requested_bytes: 64
        }
    );
    workers.allocation_counts(&mut counts).unwrap();
    for count in counts {
        assert_eq!(count.allocations, 1);
        assert_eq!(count.reallocations, 0);
        assert_eq!(count.requested_bytes, 64);
    }
    for job in &mut jobs {
        job.positive_control = false;
    }
    for _ in 0..64 {
        begin_frame();
        let result = workers.dispatch_scoped(&mut jobs, allocation_job);
        let caller = end_frame();
        assert_eq!(result, Ok(()));
        assert_eq!(caller, Counts::default());
        workers.allocation_counts(&mut counts).unwrap();
        assert_eq!(counts, [Counts::default(); 4]);
    }
    assert_eq!(
        workers.allocation_counts(&mut counts[..3]),
        Err(WorkerError::CountBuffer)
    );
    drop(jobs);
    assert_eq!(outputs, [73; 5]);
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[test]
#[expect(
    clippy::drop_non_drop,
    reason = "The fixture explicitly ends borrowed job storage before inspecting completed outputs and counts"
)]
fn sequential_dispatch_counts_are_distinct_and_empty_dispatch_clears_them() {
    use qa_platform::allocations::{Counts, begin_frame, end_frame};
    let mut workers = Workers::load(2).unwrap();
    let gate = Barrier::new(3);
    let mut outputs = [0u64; 9];
    let mut jobs = outputs.each_mut().map(|output| AllocationJob {
        output,
        positive_control: false,
        gate: Some(&gate),
    });
    for _ in 0..8 {
        workers.dispatch_scoped(&mut jobs, allocation_job).unwrap();
    }
    for job in &mut jobs {
        job.positive_control = true;
    }
    begin_frame();
    let first_result = workers.dispatch_scoped(&mut jobs, allocation_job);
    let first_caller = end_frame();
    assert_eq!(first_result, Ok(()));
    let mut first = [Counts::default(); 2];
    workers.allocation_counts(&mut first).unwrap();
    assert_eq!(
        first,
        [Counts {
            allocations: 3,
            reallocations: 0,
            requested_bytes: 192,
        }; 2]
    );
    assert_eq!(first_caller, first[0]);
    begin_frame();
    let second_result = workers.dispatch_scoped(&mut jobs[..3], allocation_job);
    let second_caller = end_frame();
    assert_eq!(second_result, Ok(()));
    let mut second = [Counts::default(); 2];
    workers.allocation_counts(&mut second).unwrap();
    assert_eq!(
        second,
        [Counts {
            allocations: 1,
            reallocations: 0,
            requested_bytes: 64,
        }; 2]
    );
    assert_eq!(second_caller, second[0]);
    assert_ne!(first, second);
    workers
        .dispatch_scoped::<u8>(&mut [], |value| *value += 1)
        .unwrap();
    workers.allocation_counts(&mut second).unwrap();
    assert_eq!(second, [Counts::default(); 2]);
    drop(jobs);
    assert_eq!(outputs, [10, 10, 10, 9, 9, 9, 9, 9, 9]);
}

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
#[test]
fn a_serial_pool_keeps_job_allocations_in_the_callers_outer_scope() {
    use qa_platform::allocations::{Counts, begin_frame, end_frame};
    let mut workers = Workers::load(0).unwrap();
    let mut outputs = [0u64; 3];
    {
        let mut jobs = outputs.each_mut().map(|output| AllocationJob {
            output,
            positive_control: true,
            gate: None,
        });
        begin_frame();
        let result = workers.dispatch_scoped(&mut jobs, allocation_job);
        let caller = end_frame();
        assert_eq!(result, Ok(()));
        assert_eq!(
            caller,
            Counts {
                allocations: 3,
                reallocations: 0,
                requested_bytes: 192
            }
        );
    }
    assert_eq!(outputs, [1; 3]);
    workers.allocation_counts(&mut []).unwrap();
    assert_eq!(
        workers.allocation_counts(&mut [Counts::default()]),
        Err(WorkerError::CountBuffer)
    );
    workers
        .dispatch_scoped::<u8>(&mut [], |job| *job += 1)
        .unwrap();
    workers.allocation_counts(&mut []).unwrap();
}
