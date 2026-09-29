//! Synchronous in-memory transport between the render frontend and worker.
//!
//! Donor provenance: `src/render/worker-transport.ts`
//! (`RenderWorkerTransport`, `serveRenderWorker`). The donor runs the worker
//! on another thread behind sequence-numbered envelopes, parent correlation,
//! and a close handshake; this port keeps the same framing and FIFO ordering
//! over two in-memory queues so headless builds pump the worker
//! synchronously with no threads.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use super::error::RenderError;
use super::types::RenderCommand;
use super::worker_runtime::DecodedCommand;

/// Request/response correlation id. Numbering starts at 1 like the donor.
pub type Sequence = u64;

/// A frontend-to-worker request envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request<W> {
    /// Correlation id, unique per request.
    pub sequence: Sequence,
    /// Encoded command payload.
    pub payload: W,
}

/// A worker-to-frontend reply envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response<R> {
    /// Correlation id copied from the answered request.
    pub sequence: Sequence,
    /// Reply payload.
    pub payload: R,
}

/// Frontend-to-worker envelope: ordered dispatches plus the close marker.
#[derive(Debug)]
enum WorkerBound<W> {
    /// One ordered command dispatch.
    Dispatch(Request<W>),
    /// Frontend retirement; the worker observes it once drained.
    Close,
}

#[derive(Debug)]
struct Channel<W, R> {
    next_sequence: Sequence,
    sent_requests: u64,
    to_worker: VecDeque<WorkerBound<W>>,
    to_frontend: VecDeque<Response<R>>,
    frontend_closed: bool,
    worker_closed: bool,
}

/// Constructor for a linked frontend/worker endpoint pair.
pub struct TransportPair;

impl TransportPair {
    /// Create a linked pair sharing two FIFO queues.
    #[must_use]
    pub fn linked<W, R>() -> (FrontendEndpoint<W, R>, WorkerEndpoint<W, R>) {
        let channel = Rc::new(RefCell::new(Channel {
            next_sequence: 1,
            sent_requests: 0,
            to_worker: VecDeque::new(),
            to_frontend: VecDeque::new(),
            frontend_closed: false,
            worker_closed: false,
        }));
        (
            FrontendEndpoint {
                channel: Rc::clone(&channel),
                completed: 0,
            },
            WorkerEndpoint { channel },
        )
    }
}

/// Frontend half: sends dispatches in order, collects correlated replies.
#[derive(Debug)]
pub struct FrontendEndpoint<W, R> {
    channel: Rc<RefCell<Channel<W, R>>>,
    completed: u64,
}

impl<W, R> FrontendEndpoint<W, R> {
    /// Enqueue one dispatch, returning its correlation id.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] once the endpoint is closed.
    pub fn send(&mut self, payload: W) -> Result<Sequence, RenderError> {
        let mut channel = self.channel.borrow_mut();
        if channel.frontend_closed {
            return Err(RenderError::Worker("render transport is closed".to_string()));
        }
        let sequence = channel.next_sequence;
        channel.next_sequence += 1;
        channel.sent_requests += 1;
        channel
            .to_worker
            .push_back(WorkerBound::Dispatch(Request { sequence, payload }));
        Ok(sequence)
    }

    /// Pop the oldest available reply.
    pub fn try_recv(&mut self) -> Option<Response<R>> {
        let response = self.channel.borrow_mut().to_frontend.pop_front()?;
        self.completed += 1;
        Some(response)
    }

    /// Drain every available reply in FIFO order.
    pub fn drain(&mut self) -> Vec<Response<R>> {
        let mut out = Vec::new();
        while let Some(response) = self.try_recv() {
            out.push(response);
        }
        out
    }

    /// Requests sent minus replies collected.
    #[must_use]
    pub fn outstanding(&self) -> u64 {
        self.channel.borrow().sent_requests - self.completed
    }

    /// Retire the frontend half; the worker observes the marker once it
    /// drains the queued dispatches. Idempotent like the donor close.
    pub fn close(&mut self) {
        let mut channel = self.channel.borrow_mut();
        if channel.frontend_closed {
            return;
        }
        channel.frontend_closed = true;
        channel.to_worker.push_back(WorkerBound::Close);
    }

    /// Whether `close` has retired this half.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.channel.borrow().frontend_closed
    }

    /// Whether no request is outstanding and no reply is queued.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        let channel = self.channel.borrow();
        channel.sent_requests == self.completed && channel.to_frontend.is_empty()
    }
}

/// Worker half: receives dispatches in order, posts correlated replies.
#[derive(Debug)]
pub struct WorkerEndpoint<W, R> {
    channel: Rc<RefCell<Channel<W, R>>>,
}

impl<W, R> WorkerEndpoint<W, R> {
    /// Pop the oldest dispatch, or `None` once closed and drained.
    pub fn recv(&mut self) -> Option<Request<W>> {
        let envelope = self.channel.borrow_mut().to_worker.pop_front()?;
        match envelope {
            WorkerBound::Dispatch(request) => Some(request),
            WorkerBound::Close => {
                self.channel.borrow_mut().worker_closed = true;
                None
            }
        }
    }

    /// Post a reply correlated to `sequence`.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] when `sequence` names no request
    /// this transport issued.
    pub fn reply(&mut self, sequence: Sequence, payload: R) -> Result<(), RenderError> {
        let sent = self.channel.borrow().sent_requests;
        if sequence == 0 || sequence > sent {
            return Err(RenderError::Worker(format!("reply to unknown sequence {sequence}")));
        }
        self.channel
            .borrow_mut()
            .to_frontend
            .push_back(Response { sequence, payload });
        Ok(())
    }

    /// Queued dispatches not yet received.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.channel.borrow().to_worker.len()
    }

    /// Whether the frontend close marker has been observed.
    #[must_use]
    pub fn observed_close(&self) -> bool {
        self.channel.borrow().worker_closed
    }

    /// Whether no dispatch is queued.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.channel.borrow().to_worker.is_empty()
    }
}

/// Codec between frontend [`RenderCommand`]s and runtime
/// [`DecodedCommand`]s.
///
/// The `worker_protocol` module owns the real wire format; the runtime and
/// entry implement against this trait so the transport shape never depends
/// on that module's exact `WireEncoder`/`WireDecoder` API.
pub trait WireCodec {
    /// Encoded command value crossing the transport.
    type Wire;

    /// Encode one command for the worker.
    fn encode_command(&mut self, command: &RenderCommand) -> Self::Wire;

    /// Decode one received value into an executable command.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::BadWire`] when the value is malformed.
    fn decode_command(&mut self, wire: &Self::Wire) -> Result<DecodedCommand, RenderError>;
}

#[cfg(test)]
mod tests {
    use super::super::types::RenderCommand;
    use super::*;

    struct Passthrough;

    impl WireCodec for Passthrough {
        type Wire = RenderCommand;

        fn encode_command(&mut self, command: &RenderCommand) -> Self::Wire {
            command.clone()
        }

        fn decode_command(&mut self, wire: &Self::Wire) -> Result<DecodedCommand, RenderError> {
            Ok(DecodedCommand::from(wire))
        }
    }

    #[test]
    fn dispatches_arrive_in_order_with_sequences_from_one() {
        let (mut frontend, mut worker) = TransportPair::linked();
        assert_eq!(frontend.send("a").unwrap(), 1);
        assert_eq!(frontend.send("b").unwrap(), 2);
        assert_eq!(frontend.send("c").unwrap(), 3);
        assert_eq!(frontend.outstanding(), 3);
        assert_eq!(worker.pending(), 3);

        let first = worker.recv().unwrap();
        assert_eq!(
            first,
            Request {
                sequence: 1,
                payload: "a"
            }
        );
        worker.reply(first.sequence, "ra").unwrap();
        assert_eq!(worker.recv().unwrap().sequence, 2);
        assert_eq!(worker.recv().unwrap().payload, "c");
        assert!(worker.recv().is_none());
        assert!(worker.is_idle());

        let replies = frontend.drain();
        assert_eq!(
            replies,
            [Response {
                sequence: 1,
                payload: "ra"
            }]
        );
        assert_eq!(frontend.outstanding(), 2);
        assert!(!frontend.is_idle());
    }

    #[test]
    fn replies_to_unknown_sequences_fail() {
        let (mut frontend, mut worker) = TransportPair::linked();
        frontend.send("a").unwrap();
        assert!(worker.reply(0, "x").is_err());
        assert!(worker.reply(2, "x").is_err());
        assert!(worker.reply(1, "ok").is_ok());
        assert_eq!(frontend.drain().len(), 1);
        assert!(frontend.is_idle());
    }

    #[test]
    fn close_retires_sender_and_drains_receiver() {
        let (mut frontend, mut worker) = TransportPair::linked::<&str, ()>();
        frontend.send("a").unwrap();
        frontend.close();
        frontend.close();
        assert!(frontend.is_closed());
        assert!(frontend.send("late").is_err());

        assert_eq!(worker.recv().unwrap().payload, "a");
        assert!(worker.recv().is_none());
        assert!(worker.observed_close());
        assert!(worker.recv().is_none());
    }

    #[test]
    fn codec_round_trip_preserves_command_order() {
        let mut codec = Passthrough;
        let commands = [
            RenderCommand::SwapBuffers,
            RenderCommand::SetColor(qa_core::math::vec4(1.0, 0.5, 0.25, 1.0)),
        ];
        let (mut frontend, mut worker) = TransportPair::linked();
        for command in &commands {
            frontend.send(codec.encode_command(command)).unwrap();
        }
        let mut decoded = Vec::new();
        while let Some(request) = worker.recv() {
            decoded.push(codec.decode_command(&request.payload).unwrap());
            worker.reply(request.sequence, true).unwrap();
        }
        assert_eq!(decoded.len(), 2);
        assert!(matches!(decoded[0], DecodedCommand::SwapBuffers { .. }));
        assert!(matches!(decoded[1], DecodedCommand::SetColor(_)));
        assert_eq!(frontend.drain().len(), 2);
        assert!(frontend.is_idle());
    }
}
