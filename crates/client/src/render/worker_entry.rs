//! Synchronous worker system: frontend handle plus a pumped runtime.
//!
//! Donor provenance: `src/render/worker-entry.ts` (`serveRenderWorker` +
//! `createRenderWorkerRuntime`). The donor boots a thread behind a message
//! port; this port links the frontend handle and the runtime through the
//! in-memory transport and drives both sides with explicit pumps, so
//! headless builds keep identical command ordering with no threads.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::mem::take;
use std::rc::Rc;

use super::error::RenderError;
use super::types::{ImageResourceOperation, OrderedBackend, RenderCommand};
use super::worker::WorkerHandle;
use super::worker_runtime::{DecodedCommand, WorkerRuntime};
use super::worker_transport::{TransportPair, WireCodec, WorkerEndpoint};

/// Per-command acknowledgment collected from the runtime callbacks.
/// Successful image operations journal on the frontend; swaps deliver
/// their armed capture ids.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkerAck {
    /// An image operation applied; the frontend journals `0`.
    ImageApplied(ImageResourceOperation),
    /// A swap executed with these capture ids.
    Swapped(Vec<u32>),
}

/// Closure-pair adapter implementing [`WireCodec`].
///
/// This is the single injection point for the real wire format owned by
/// `crate::render::worker_protocol`: pass `WireEncoder::command` as the
/// encode closure and `WireDecoder::command` as the decode closure.
/// Concretely, once that module lands, wiring it looks like this:
///
/// ```text
/// let mut encoder = worker_protocol::WireEncoder::new(owner);
/// let mut decoder = worker_protocol::WireDecoder::new(owner, palette_source);
/// let adapter = ProtocolAdapter::new(
///     |command: &RenderCommand| encoder.command(command),
///     |wire: &WireValue| decoder.command(wire),
/// );
/// let mut system = WorkerSystem::new(backend, adapter);
/// ```
///
/// Any deviation between those exact `WireEncoder`/`WireDecoder` APIs and
/// [`WireCodec`] is absorbed by these two closures, in this file only.
pub struct ProtocolAdapter<W, E, D> {
    encode: E,
    decode: D,
    wire: PhantomData<W>,
}

impl<W, E, D> ProtocolAdapter<W, E, D>
where
    E: FnMut(&RenderCommand) -> W,
    D: FnMut(&W) -> Result<DecodedCommand, RenderError>,
{
    /// Build an adapter from an encode and a decode closure.
    pub fn new(encode: E, decode: D) -> Self {
        Self {
            encode,
            decode,
            wire: PhantomData,
        }
    }
}

impl<W, E, D> WireCodec for ProtocolAdapter<W, E, D>
where
    E: FnMut(&RenderCommand) -> W,
    D: FnMut(&W) -> Result<DecodedCommand, RenderError>,
{
    type Wire = W;

    fn encode_command(&mut self, command: &RenderCommand) -> Self::Wire {
        (self.encode)(command)
    }

    fn decode_command(&mut self, wire: &Self::Wire) -> Result<DecodedCommand, RenderError> {
        (self.decode)(wire)
    }
}

/// Linked frontend handle and worker runtime sharing one transport.
pub struct WorkerSystem<B: OrderedBackend, C: WireCodec> {
    /// Frontend submission handle.
    pub handle: WorkerHandle<C::Wire, Vec<WorkerAck>>,
    endpoint: WorkerEndpoint<C::Wire, Result<Vec<WorkerAck>, RenderError>>,
    runtime: WorkerRuntime<B>,
    codec: C,
    pending: Rc<RefCell<Vec<WorkerAck>>>,
    terminal: Option<RenderError>,
}

impl<B: OrderedBackend, C: WireCodec> WorkerSystem<B, C> {
    /// Link `backend` and `codec` through a fresh transport pair.
    pub fn new(backend: B, codec: C) -> Self {
        let (frontend, endpoint) = TransportPair::linked();
        let pending: Rc<RefCell<Vec<WorkerAck>>> = Rc::new(RefCell::new(Vec::new()));
        let applied_sink = Rc::clone(&pending);
        let swap_sink = Rc::clone(&pending);
        let runtime = WorkerRuntime::new(
            backend,
            move |operation: &ImageResourceOperation| {
                applied_sink
                    .borrow_mut()
                    .push(WorkerAck::ImageApplied(operation.clone()));
            },
            move |captures: &[u32]| {
                swap_sink.borrow_mut().push(WorkerAck::Swapped(captures.to_vec()));
            },
        );
        Self {
            handle: WorkerHandle::from_endpoint(frontend),
            endpoint,
            runtime,
            codec,
            pending,
            terminal: None,
        }
    }

    /// Borrow the runtime backend.
    #[must_use]
    pub fn backend(&self) -> &B {
        self.runtime.backend()
    }

    /// Mutably borrow the runtime backend.
    pub fn backend_mut(&mut self) -> &mut B {
        self.runtime.backend_mut()
    }

    /// Mutably borrow the codec (for encode-side state).
    pub fn codec_mut(&mut self) -> &mut C {
        &mut self.codec
    }

    /// Whether the worker has no queued dispatch.
    #[must_use]
    pub fn is_worker_idle(&self) -> bool {
        self.endpoint.is_idle()
    }

    /// Whether a decode failure has terminally failed the worker. Like the
    /// donor, the first failure sticks and fails later dispatches.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.terminal.is_some()
    }

    /// Drive one queued dispatch through decode, execute, and reply.
    /// Returns `false` when no dispatch is queued.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] when the reply cannot correlate to
    /// its request. Decode failures are reported to the frontend through
    /// the reply instead of surfacing here.
    pub fn pump_once(&mut self) -> Result<bool, RenderError> {
        let Some(request) = self.endpoint.recv() else {
            return Ok(false);
        };
        if let Some(terminal) = self.terminal.clone() {
            self.endpoint.reply(request.sequence, Err(terminal))?;
            return Ok(true);
        }
        let outcome = match self.codec.decode_command(&request.payload) {
            Ok(command) => {
                self.runtime.execute_decoded(&command);
                Ok(take(&mut *self.pending.borrow_mut()))
            }
            Err(error) => {
                self.terminal = Some(error.clone());
                Err(error)
            }
        };
        self.endpoint.reply(request.sequence, outcome)?;
        Ok(true)
    }

    /// Drive dispatches until none remain, returning the count processed.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] when a reply cannot correlate to
    /// its request; per-command failures stay in the replies.
    pub fn pump_until_idle(&mut self) -> Result<usize, RenderError> {
        let mut processed = 0;
        while self.pump_once()? {
            processed += 1;
        }
        Ok(processed)
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::super::types::{RenderFrame, ResourceOwner, TextureFilter};
    use super::super::worker_runtime::support::FakeBackend;
    use super::super::worker_transport::WireCodec;
    use super::*;

    /// Script codec: commands cross as clones; `None` decodes as a bad wire.
    struct ScriptCodec;

    impl WireCodec for ScriptCodec {
        type Wire = Option<RenderCommand>;

        fn encode_command(&mut self, command: &RenderCommand) -> Self::Wire {
            Some(command.clone())
        }

        fn decode_command(&mut self, wire: &Self::Wire) -> Result<DecodedCommand, RenderError> {
            match wire {
                Some(command) => Ok(DecodedCommand::from(command)),
                None => Err(RenderError::BadWire("empty script value".to_string())),
            }
        }
    }

    fn frame(commands: Vec<RenderCommand>) -> RenderFrame {
        let authority = IdentityOwner::create("worker-entry-test").unwrap();
        RenderFrame {
            owner: ResourceOwner::new(1, authority.session().clone(), 0),
            sequence: 1,
            commands,
        }
    }

    fn texture_mode() -> ImageResourceOperation {
        ImageResourceOperation::TextureMode {
            filter: TextureFilter::Linear,
        }
    }

    #[test]
    fn pump_round_trip_executes_and_acknowledges() {
        let mut system = WorkerSystem::new(FakeBackend::new(), ScriptCodec);
        let frame = frame(vec![
            RenderCommand::ImageResource(texture_mode()),
            RenderCommand::SwapBuffers,
        ]);
        let submitted = system
            .handle
            .submit_frame(&frame, &mut |command| ScriptCodec.encode_command(command))
            .unwrap();
        assert!(submitted.is_empty());
        assert!(!system.is_worker_idle());

        assert_eq!(system.pump_until_idle().unwrap(), 2);
        assert!(system.is_worker_idle());
        assert!(!system.is_terminal());

        let acks = system.handle.drain_acknowledgments().unwrap();
        assert_eq!(
            acks,
            [
                vec![WorkerAck::ImageApplied(texture_mode())],
                vec![WorkerAck::Swapped(Vec::new())],
            ]
        );
        assert_eq!(system.backend().log, ["image:mode"]);
        assert!(!system.pump_once().unwrap());
    }

    #[test]
    fn decode_failure_replies_and_sticks_like_terminal() {
        let mut system = WorkerSystem::new(FakeBackend::new(), ScriptCodec);
        let frame = frame(vec![RenderCommand::SwapBuffers]);
        system.handle.submit_frame(&frame, &mut |_| None).unwrap();
        assert_eq!(system.pump_until_idle().unwrap(), 1);
        assert!(system.is_terminal());
        let error = system.handle.drain_acknowledgments().unwrap_err();
        assert_eq!(
            error,
            RenderError::Worker("invalid renderer wire value: empty script value".to_string())
        );
        assert!(system.backend().log.is_empty());

        system
            .handle
            .submit_frame(&frame, &mut |command| Some(command.clone()))
            .unwrap();
        assert_eq!(system.pump_until_idle().unwrap(), 1);
        assert!(system.backend().log.is_empty(), "terminal skips execution");
        assert!(matches!(
            system.handle.drain_acknowledgments().unwrap_err(),
            RenderError::Worker(_)
        ));
    }

    #[test]
    fn protocol_adapter_carries_closure_codec() {
        let adapter = ProtocolAdapter::new(
            |command: &RenderCommand| Some(command.clone()),
            |wire: &Option<RenderCommand>| match wire {
                Some(command) => Ok(DecodedCommand::from(command)),
                None => Err(RenderError::BadWire("empty".to_string())),
            },
        );
        let mut system = WorkerSystem::new(FakeBackend::new(), adapter);
        let frame = frame(vec![RenderCommand::SwapBuffers]);
        let wire = system.codec_mut().encode_command(&frame.commands[0]);
        system.handle.submit_frame(&frame, &mut |_| wire.clone()).unwrap();
        assert_eq!(system.pump_until_idle().unwrap(), 1);
        assert_eq!(
            system.handle.drain_acknowledgments().unwrap(),
            [vec![WorkerAck::Swapped(Vec::new())]]
        );
    }
}
