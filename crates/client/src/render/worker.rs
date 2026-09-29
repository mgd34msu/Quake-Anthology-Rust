//! Frontend handle submitting frames to the render worker.
//!
//! Donor provenance: `src/render/worker.ts` (`WorkerRenderer`). The donor
//! overlaps one immutable command packet with frontend work and treats
//! synchronous calls as barriers; this port sends encoded commands in order
//! over the synchronous transport and pumps correlated acknowledgments,
//! surfacing worker failures as [`RenderError::Worker`].

use super::error::RenderError;
use super::types::{RenderCommand, RenderFrame};
use super::worker_transport::{FrontendEndpoint, TransportPair, WorkerEndpoint};

fn surfaced(error: RenderError) -> RenderError {
    match error {
        RenderError::Worker(_) => error,
        other => RenderError::Worker(other.to_string()),
    }
}

/// Frontend owner of the transport endpoint plus the acknowledgment pump.
/// `W` is the encoded command value, `A` the per-command acknowledgment.
pub struct WorkerHandle<W, A> {
    endpoint: FrontendEndpoint<W, Result<A, RenderError>>,
    next_expected: u64,
    closed: bool,
}

impl<W, A> WorkerHandle<W, A> {
    /// Wrap a frontend endpoint from [`TransportPair::linked`].
    #[must_use]
    pub fn from_endpoint(endpoint: FrontendEndpoint<W, Result<A, RenderError>>) -> Self {
        Self {
            endpoint,
            next_expected: 1,
            closed: false,
        }
    }

    /// Create a handle already linked to its worker endpoint.
    #[must_use]
    pub fn linked() -> (Self, WorkerEndpoint<W, Result<A, RenderError>>) {
        let (frontend, worker) = TransportPair::linked();
        (Self::from_endpoint(frontend), worker)
    }

    /// Encode `frame` in order, then collect the acknowledgments available
    /// so far. In the synchronous system the worker runs when pumped, so a
    /// fresh submit usually returns no acks; drain again after pumping.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] when shut down, when the transport
    /// is closed, when an acknowledgment arrives out of order, or when the
    /// worker reports a failure.
    pub fn submit_frame(
        &mut self,
        frame: &RenderFrame,
        encode: &mut dyn FnMut(&RenderCommand) -> W,
    ) -> Result<Vec<A>, RenderError> {
        if self.closed {
            return Err(RenderError::Worker("render worker handle is shut down".to_string()));
        }
        for command in &frame.commands {
            self.endpoint.send(encode(command))?;
        }
        self.drain_acknowledgments()
    }

    /// Collect queued acknowledgments in correlation order.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] on the first out-of-order or failed
    /// acknowledgment; earlier successful acks in the same drain are kept
    /// by the caller only when the whole drain succeeds.
    pub fn drain_acknowledgments(&mut self) -> Result<Vec<A>, RenderError> {
        let mut acks = Vec::new();
        for response in self.endpoint.drain() {
            if response.sequence != self.next_expected {
                return Err(RenderError::Worker(format!(
                    "worker acknowledgment out of order: expected {} got {}",
                    self.next_expected, response.sequence
                )));
            }
            self.next_expected += 1;
            match response.payload {
                Ok(ack) => acks.push(ack),
                Err(error) => return Err(surfaced(error)),
            }
        }
        Ok(acks)
    }

    /// Submitted commands still awaiting acknowledgment.
    #[must_use]
    pub fn outstanding(&self) -> u64 {
        self.endpoint.outstanding()
    }

    /// Whether [`WorkerHandle::shutdown`] has retired this handle.
    #[must_use]
    pub const fn is_shutdown(&self) -> bool {
        self.closed
    }

    /// Retire the handle, collecting the acknowledgments available so far.
    /// Idempotent; draining stays available after shutdown so late replies
    /// from an already-pumped worker are still observable.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Worker`] when a drained acknowledgment is out
    /// of order or carries a worker failure.
    pub fn shutdown(&mut self) -> Result<Vec<A>, RenderError> {
        if self.closed {
            return Ok(Vec::new());
        }
        self.endpoint.close();
        self.closed = true;
        self.drain_acknowledgments()
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec4;

    use super::super::types::ResourceOwner;
    use super::*;

    fn frame(commands: Vec<RenderCommand>) -> RenderFrame {
        let authority = IdentityOwner::create("worker-handle-test").unwrap();
        RenderFrame {
            owner: ResourceOwner::new(1, authority.session().clone(), 0),
            sequence: 1,
            commands,
        }
    }

    fn names(commands: &[RenderCommand]) -> Vec<&'static str> {
        commands
            .iter()
            .map(|command| match command {
                RenderCommand::SwapBuffers => "swap",
                RenderCommand::SetColor(_) => "color",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn submit_sends_commands_in_order() {
        let (mut handle, mut worker) = WorkerHandle::<String, String>::linked();
        let frame = frame(vec![
            RenderCommand::SwapBuffers,
            RenderCommand::SetColor(vec4(1.0, 1.0, 1.0, 1.0)),
            RenderCommand::SwapBuffers,
        ]);
        let mut encode = |command: &RenderCommand| names(std::slice::from_ref(command))[0].to_string();
        let acks = handle.submit_frame(&frame, &mut encode).unwrap();
        assert!(acks.is_empty());
        assert_eq!(handle.outstanding(), 3);

        let mut seen = Vec::new();
        while let Some(request) = worker.recv() {
            seen.push((request.sequence, request.payload.clone()));
            worker.reply(request.sequence, Ok(request.payload)).unwrap();
        }
        assert_eq!(
            seen,
            [
                (1, "swap".to_string()),
                (2, "color".to_string()),
                (3, "swap".to_string())
            ]
        );
        assert_eq!(handle.drain_acknowledgments().unwrap(), ["swap", "color", "swap"]);
        assert_eq!(handle.outstanding(), 0);
    }

    #[test]
    fn drain_surfaces_worker_failures() {
        let (mut handle, mut worker) = WorkerHandle::<String, String>::linked();
        let frame = frame(vec![RenderCommand::SwapBuffers, RenderCommand::SwapBuffers]);
        handle.submit_frame(&frame, &mut |_| "cmd".to_string()).unwrap();
        worker.reply(1, Ok("first".to_string())).unwrap();
        worker.reply(2, Err(RenderError::BadWire("short".to_string()))).unwrap();
        let error = handle.drain_acknowledgments().unwrap_err();
        assert_eq!(
            error,
            RenderError::Worker("invalid renderer wire value: short".to_string())
        );

        let (mut handle, mut worker) = WorkerHandle::<String, String>::linked();
        handle.submit_frame(&frame, &mut |_| "cmd".to_string()).unwrap();
        worker
            .reply(1, Err(RenderError::Worker("backend lost".to_string())))
            .unwrap();
        assert_eq!(
            handle.drain_acknowledgments().unwrap_err(),
            RenderError::Worker("backend lost".to_string())
        );
    }

    #[test]
    fn out_of_order_acknowledgment_fails() {
        let (mut handle, mut worker) = WorkerHandle::<String, String>::linked();
        let frame = frame(vec![RenderCommand::SwapBuffers, RenderCommand::SwapBuffers]);
        handle.submit_frame(&frame, &mut |_| "cmd".to_string()).unwrap();
        worker.reply(2, Ok("late".to_string())).unwrap();
        worker.reply(1, Ok("early".to_string())).unwrap();
        let error = handle.drain_acknowledgments().unwrap_err();
        assert!(matches!(error, RenderError::Worker(_)), "{error:?}");
    }

    #[test]
    fn shutdown_retires_and_rejects_resubmits() {
        let (mut handle, mut worker) = WorkerHandle::<String, String>::linked();
        let frame = frame(vec![RenderCommand::SwapBuffers]);
        handle.submit_frame(&frame, &mut |_| "cmd".to_string()).unwrap();
        assert!(!handle.is_shutdown());
        assert!(handle.shutdown().unwrap().is_empty());
        assert!(handle.is_shutdown());
        assert!(handle.shutdown().unwrap().is_empty());
        assert!(handle.submit_frame(&frame, &mut |_| "cmd".to_string()).is_err());

        assert_eq!(worker.recv().unwrap().sequence, 1);
        assert!(worker.recv().is_none());
        assert!(worker.observed_close());
    }
}
