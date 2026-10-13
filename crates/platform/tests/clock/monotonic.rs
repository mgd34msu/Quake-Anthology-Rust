use super::{Clock, Stopwatch};

#[test]
fn independent_event_clocks_remain_monotonic_during_concurrent_startup() {
    let start = std::sync::Barrier::new(16);
    std::thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                start.wait();
                let clock = Clock::new();
                let watch = Stopwatch::start();
                let mut previous = clock.now();
                let mut elapsed = watch.elapsed();
                for _ in 0..2000 {
                    let current = clock.now();
                    assert!(current >= previous);
                    previous = current;
                    let next = watch.elapsed();
                    assert!(next >= elapsed);
                    elapsed = next;
                }
            });
        }
    });
}
