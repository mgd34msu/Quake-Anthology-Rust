use qa_platform::allocations::Counts;

#[derive(Default)]
pub struct Gate {
    pub frames: u64,
    pub failed_frames: u64,
    pub counts: Counts,
}

impl Gate {
    pub fn observe(&mut self, frame: u64, warmup: u32, counts: Counts) {
        if frame < u64::from(warmup) {
            return;
        }
        self.frames += 1;
        self.failed_frames += u64::from(counts.allocations != 0 || counts.reallocations != 0);
        self.counts.allocations += counts.allocations;
        self.counts.reallocations += counts.reallocations;
        self.counts.requested_bytes += counts.requested_bytes;
    }

    pub fn finish(self, scope: &str, worker_threads: usize) -> Result<(), String> {
        println!(
            "{{\"event\":\"allocation_gate\",\"scope\":\"{scope}_all_instrumented_rust_threads\",\"calling_threads\":1,\"worker_threads\":{worker_threads},\"native_heap_measured\":false,\"frames\":{},\"failed_frames\":{},\"allocations\":{},\"reallocations\":{},\"requested_bytes\":{},\"passed\":{}}}",
            self.frames,
            self.failed_frames,
            self.counts.allocations,
            self.counts.reallocations,
            self.counts.requested_bytes,
            self.frames != 0 && self.failed_frames == 0,
        );
        if self.frames == 0 {
            Err("allocation qualification requires measured frames".into())
        } else if self.failed_frames != 0 {
            Err("allocation qualification rejected allocating measured frames".into())
        } else {
            Ok(())
        }
    }
}
