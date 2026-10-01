//! Ordered remote-seat publication pump with failure capture.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/remote-seat-pump.ts`
//! (`RemoteSeatPump`). The network poll and source close (`./remote-seat-source.ts`, out
//! of scope) arrive through the [`RemoteSeatPumpSource`] seam. The donor's promise
//! microtasks are sync: `poll` runs the network poll inline, and `enqueue` reports
//! completion through a callback instead of a promise. Interior mutability keeps the
//! donor's reentrancy patterns (a publication enqueueing another, a poll during a poll)
//! working under sync calls. Three documented folds: unknown donor errors become
//! [`PumpError::Failed`] messages, enqueueing after close fails immediately instead of
//! through the callback, and a reentrant `close` joins the in-flight close while an
//! in-flight drain error after close started is dropped.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use thiserror::Error;

/// Pump failure, with the donor retirement message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PumpError {
    /// The publication was retired (donor `retirementError`).
    #[error("Remote seat publication was retired")]
    Retired,
    /// A poll or publication failed (donor `unknown` error, messaged).
    #[error("Remote seat pump failed: {0}")]
    Failed(String),
}

/// Network poll and source close (donor `RemoteSeatSource` subset).
pub trait RemoteSeatPumpSource {
    /// Poll the network.
    fn poll(&mut self, now_ms: u64) -> Result<(), PumpError>;
    /// Close the source.
    fn close(&mut self) -> Result<(), PumpError>;
}

/// One queued publication (donor `Publication`).
struct Publication {
    ready: Box<dyn Fn() -> bool>,
    run: Option<Box<dyn FnMut() -> Result<(), PumpError>>>,
    fail: Option<Box<dyn FnMut(PumpError)>>,
}

/// Remote seat pump (donor `RemoteSeatPump`).
pub struct RemoteSeatPump<S> {
    source: RefCell<S>,
    pending_poll: Cell<bool>,
    pending_drain: Cell<bool>,
    failure: RefCell<Option<PumpError>>,
    publications: RefCell<VecDeque<Publication>>,
    closing: Cell<bool>,
    closed: Cell<bool>,
    close_result: RefCell<Option<Result<(), PumpError>>>,
}

impl<S> RemoteSeatPump<S> {
    /// Build the pump over a source.
    pub fn new(source: S) -> Self {
        Self {
            source: RefCell::new(source),
            pending_poll: Cell::new(false),
            pending_drain: Cell::new(false),
            failure: RefCell::new(None),
            publications: RefCell::new(VecDeque::new()),
            closing: Cell::new(false),
            closed: Cell::new(false),
            close_result: RefCell::new(None),
        }
    }
}

impl<S: RemoteSeatPumpSource> RemoteSeatPump<S> {
    /// Poll the network unless closed, busy, or failed (donor `poll`).
    pub fn poll(&self, now_ms: u64) {
        if self.closed.get() || self.pending_poll.get() || self.failure.borrow().is_some() {
            return;
        }
        self.pending_poll.set(true);
        let outcome = self.source.borrow_mut().poll(now_ms);
        self.pending_poll.set(false);
        if let Err(error) = outcome {
            if error != PumpError::Retired {
                *self.failure.borrow_mut() = Some(error);
            }
        }
    }

    /// Rethrow the captured failure (donor `throwFailure`).
    pub fn throw_failure(&self) -> Result<(), PumpError> {
        match &*self.failure.borrow() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// Enqueue an operation with its completion (donor `enqueue`).
    pub fn enqueue<T, F, D>(&self, operation: F, done: D) -> Result<(), PumpError>
    where
        T: 'static,
        F: FnOnce() -> Result<T, PumpError> + 'static,
        D: FnOnce(Result<T, PumpError>) + 'static,
    {
        self.enqueue_when(operation, || true, done)
    }

    /// Enqueue an operation gated by readiness (donor `enqueue` with `ready`).
    pub fn enqueue_when<T, F, R, D>(&self, operation: F, ready: R, done: D) -> Result<(), PumpError>
    where
        T: 'static,
        F: FnOnce() -> Result<T, PumpError> + 'static,
        R: Fn() -> bool + 'static,
        D: FnOnce(Result<T, PumpError>) + 'static,
    {
        if self.closed.get() {
            return Err(PumpError::Retired);
        }
        let done = Rc::new(RefCell::new(Some(done)));
        let mut operation = Some(operation);
        let run_done = Rc::clone(&done);
        let fail_done = Rc::clone(&done);
        self.publications.borrow_mut().push_back(Publication {
            ready: Box::new(ready),
            run: Some(Box::new(move || {
                let Some(operation) = operation.take() else {
                    return Ok(());
                };
                let result = operation();
                let outcome = match &result {
                    Ok(_) => Ok(()),
                    Err(error) => Err(error.clone()),
                };
                if let Some(done) = run_done.borrow_mut().take() {
                    done(result);
                }
                outcome
            })),
            fail: Some(Box::new(move |error| {
                if let Some(done) = fail_done.borrow_mut().take() {
                    done(Err(error));
                }
            })),
        });
        Ok(())
    }

    /// Run ready publications in order (donor `drainReady`).
    pub fn drain_ready(&self) -> Result<(), PumpError> {
        if self.pending_drain.get() {
            return Ok(());
        }
        if self.closed.get() {
            return Ok(());
        }
        self.pending_drain.set(true);
        let mut outcome = Ok(());
        while !self.closed.get() {
            let mut next = match self.publications.borrow_mut().pop_front() {
                Some(next) => next,
                None => break,
            };
            if !(next.ready)() {
                self.publications.borrow_mut().push_front(next);
                break;
            }
            if let Some(mut run) = next.run.take() {
                if let Err(error) = run() {
                    outcome = Err(error);
                    break;
                }
            }
        }
        self.pending_drain.set(false);
        outcome
    }

    /// Retire queued publications and close the source (donor `close`).
    pub fn close(&self) -> Result<(), PumpError> {
        if let Some(result) = self.close_result.borrow().clone() {
            return result;
        }
        if self.closing.get() {
            return Ok(());
        }
        self.closing.set(true);
        self.closed.set(true);
        for mut publication in self.publications.borrow_mut().drain(..) {
            if let Some(mut fail) = publication.fail.take() {
                fail(PumpError::Retired);
            }
        }
        let outcome = match self.source.borrow_mut().close() {
            Ok(()) => self.throw_failure(),
            Err(error) => Err(error),
        };
        *self.close_result.borrow_mut() = Some(outcome.clone());
        self.closing.set(false);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        polls: Rc<Cell<u32>>,
        poll_result: Result<(), PumpError>,
        closes: Rc<Cell<u32>>,
    }

    impl RemoteSeatPumpSource for Stub {
        fn poll(&mut self, _now_ms: u64) -> Result<(), PumpError> {
            self.polls.set(self.polls.get() + 1);
            self.poll_result.clone()
        }
        fn close(&mut self) -> Result<(), PumpError> {
            self.closes.set(self.closes.get() + 1);
            Ok(())
        }
    }

    fn stub(poll_result: Result<(), PumpError>) -> (RemoteSeatPump<Stub>, Rc<Cell<u32>>, Rc<Cell<u32>>) {
        let polls = Rc::new(Cell::new(0));
        let closes = Rc::new(Cell::new(0));
        let pump = RemoteSeatPump::new(Stub {
            polls: Rc::clone(&polls),
            poll_result,
            closes: Rc::clone(&closes),
        });
        (pump, polls, closes)
    }

    #[test]
    fn poll_captures_failure_and_stops() {
        let (pump, polls, _) = stub(Err(PumpError::Failed("boom".to_string())));
        pump.poll(10);
        pump.poll(20);
        assert_eq!(polls.get(), 1);
        assert_eq!(pump.throw_failure(), Err(PumpError::Failed("boom".to_string())));
    }

    #[test]
    fn poll_retirement_is_swallowed() {
        let (pump, polls, _) = stub(Err(PumpError::Retired));
        pump.poll(10);
        assert_eq!(polls.get(), 1);
        assert_eq!(pump.throw_failure(), Ok(()));
    }

    #[test]
    fn drain_runs_ready_operations_in_order() {
        let (pump, _, _) = stub(Ok(()));
        let order = Rc::new(RefCell::new(Vec::new()));
        let gate = Rc::new(Cell::new(false));
        for (name, gated) in [("first", true), ("second", false)] {
            let order = Rc::clone(&order);
            let gate = Rc::clone(&gate);
            let run = move || {
                order.borrow_mut().push(name);
                Ok(())
            };
            if gated {
                pump.enqueue_when(run, move || gate.get(), |_: Result<(), PumpError>| {})
                    .unwrap();
            } else {
                pump.enqueue(run, |_: Result<(), PumpError>| {}).unwrap();
            }
        }
        pump.drain_ready().unwrap();
        assert!(order.borrow().is_empty());
        gate.set(true);
        pump.drain_ready().unwrap();
        assert_eq!(*order.borrow(), vec!["first", "second"]);
    }

    #[test]
    fn drain_error_propagates_and_dequeues() {
        let (pump, _, _) = stub(Ok(()));
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_run = Rc::clone(&seen);
        pump.enqueue(
            || Err::<(), _>(PumpError::Failed("op".to_string())),
            move |result| seen_run.borrow_mut().push(result.is_ok()),
        )
        .unwrap();
        let seen_drain = Rc::clone(&seen);
        pump.enqueue(
            || Ok(2),
            move |result: Result<i32, PumpError>| {
                seen_drain.borrow_mut().push(result == Ok(2));
            },
        )
        .unwrap();
        assert_eq!(pump.drain_ready(), Err(PumpError::Failed("op".to_string())));
        assert_eq!(*seen.borrow(), vec![false]);
        pump.drain_ready().unwrap();
        assert_eq!(*seen.borrow(), vec![false, true]);
    }

    #[test]
    fn close_retires_queued_and_replays() {
        let (pump, _, closes) = stub(Ok(()));
        let retired = Rc::new(Cell::new(false));
        let retired_done = Rc::clone(&retired);
        pump.enqueue(
            || Ok(()),
            move |result: Result<(), PumpError>| {
                retired_done.set(result == Err(PumpError::Retired));
            },
        )
        .unwrap();
        pump.close().unwrap();
        assert!(retired.get());
        assert_eq!(closes.get(), 1);
        assert_eq!(
            pump.enqueue(|| Ok(()), |_: Result<(), PumpError>| {}),
            Err(PumpError::Retired)
        );
        pump.close().unwrap();
        assert_eq!(closes.get(), 1);
    }

    #[test]
    fn operation_can_enqueue_reentrantly() {
        let (pump, _, _) = stub(Ok(()));
        let pump = Rc::new(pump);
        let order = Rc::new(RefCell::new(Vec::new()));
        let inner_pump = Rc::clone(&pump);
        let inner_order = Rc::clone(&order);
        pump.enqueue(
            move || {
                inner_order.borrow_mut().push("outer");
                inner_pump
                    .enqueue(|| Ok(()), {
                        let inner_order = Rc::clone(&inner_order);
                        move |_: Result<(), PumpError>| inner_order.borrow_mut().push("inner")
                    })
                    .unwrap();
                Ok(())
            },
            |_: Result<(), PumpError>| {},
        )
        .unwrap();
        pump.drain_ready().unwrap();
        assert_eq!(*order.borrow(), vec!["outer", "inner"]);
    }
}
