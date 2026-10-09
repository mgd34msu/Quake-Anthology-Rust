#![cfg(any(debug_assertions, feature = "allocation-tracking"))]

use qa_app::renderer::CpuDispatch;
#[cfg(panic = "unwind")]
use qa_platform::WorkerError;
use qa_platform::allocations::{Counts, begin_frame, end_frame};
use qa_render::cpu::RasterBands;

// This test executable instruments the calling thread and its owned workers.
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;

struct Job<'a> {
    output: &'a mut u32,
    bytes: usize,
    panic: bool,
}

fn work(job: &mut Job<'_>) {
    if job.bytes != 0 {
        std::hint::black_box(vec![19u8; job.bytes]);
    }
    if job.panic {
        panic!("worker allocation positive control");
    }
    *job.output += 1;
}

#[test]
#[expect(
    clippy::drop_non_drop,
    reason = "The fixture explicitly ends borrowed job storage before inspecting completed outputs and counts"
)]
fn each_frame_merges_all_batches_once_and_terminal_frame_keeps_last_batch() {
    for bands in [RasterBands::One, RasterBands::Two] {
        let mut dispatch = CpuDispatch::load(bands).unwrap();
        assert_eq!(dispatch.worker_count(), bands.count() - 1);
        let mut output = [0u32; 2];
        let mut jobs = output.each_mut().map(|output| Job {
            output,
            bytes: 0,
            panic: false,
        });
        for _ in 0..8 {
            dispatch.dispatch(&mut jobs, work).unwrap();
        }
        let _ = dispatch.merge_counts(Counts::default());

        // Startup is measured here as a positive control, then discarded.
        begin_frame();
        for job in &mut jobs {
            job.bytes = 32;
        }
        dispatch.dispatch(&mut jobs, work).unwrap();
        let startup = dispatch.merge_counts(end_frame());
        assert_eq!(
            startup,
            Counts {
                allocations: 2,
                reallocations: 0,
                requested_bytes: 64
            }
        );
        assert_eq!(dispatch.merge_counts(Counts::default()), Counts::default());

        begin_frame();
        std::hint::black_box(vec![7u8; 96]);
        for bytes in [64, 128] {
            for job in &mut jobs {
                job.bytes = bytes;
            }
            dispatch.dispatch(&mut jobs, work).unwrap();
        }
        let frame = dispatch.merge_counts(end_frame());
        assert_eq!(
            frame,
            Counts {
                allocations: 5,
                reallocations: 0,
                requested_bytes: 480
            }
        );
        assert_eq!(dispatch.merge_counts(Counts::default()), Counts::default());

        // The terminal frame consumes its final dispatch before pool teardown.
        begin_frame();
        for job in &mut jobs {
            job.bytes = 40;
        }
        dispatch.dispatch(&mut jobs, work).unwrap();
        let terminal = dispatch.merge_counts(end_frame());
        assert_eq!(
            terminal,
            Counts {
                allocations: 2,
                reallocations: 0,
                requested_bytes: 80
            }
        );
        assert_eq!(dispatch.merge_counts(Counts::default()), Counts::default());
        assert_eq!(dispatch.failure(), None);
        drop(jobs);
        assert_eq!(output, [12; 2]);
    }
}

#[cfg(panic = "unwind")]
#[test]
#[expect(
    clippy::drop_non_drop,
    reason = "The fixture explicitly ends borrowed job storage before inspecting completed outputs and counts"
)]
fn failed_batch_counts_are_captured_before_error_and_empty_batch_adds_nothing() {
    let mut dispatch = CpuDispatch::load(RasterBands::Two).unwrap();
    let mut output = [0u32; 2];
    let mut jobs = output.each_mut().map(|output| Job {
        output,
        bytes: 0,
        panic: false,
    });
    for _ in 0..8 {
        dispatch.dispatch(&mut jobs, work).unwrap();
    }
    let _ = dispatch.merge_counts(Counts::default());
    for job in &mut jobs {
        job.bytes = 64;
    }
    begin_frame();
    dispatch.dispatch(&mut jobs, work).unwrap();
    jobs[0].panic = true;
    assert_eq!(
        dispatch.dispatch(&mut jobs, work),
        Err(WorkerError::JobPanicked)
    );
    dispatch
        .dispatch::<u8>(&mut [], |value| *value += 1)
        .unwrap();
    let total = dispatch.merge_counts(end_frame());
    // Both batches allocate twice; panic reporting may add further allocations.
    assert!(total.allocations >= 4);
    assert!(total.requested_bytes >= 256);
    assert_eq!(dispatch.merge_counts(Counts::default()), Counts::default());
    assert_eq!(dispatch.failure(), Some(WorkerError::JobPanicked));
    drop(jobs);
    assert_eq!(output, [9, 10]);
}
