//! Remote seat component-client collection view.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/remote-components.ts`
//! (`RemoteComponentView`). The component collection resolves to the canonical
//! [`ApplicationModPresentations`](super::mod_presentations::ApplicationModPresentations)
//! (checkpoint mirror at
//! [`mod_presentation_checkpoint`](qa_guest::qvm::mod_presentation_checkpoint)),
//! which implements [`RemoteComponentClients`] below; the media preparation
//! ([`component_media`](super::component_media)) and the unified remote side
//! ([`network::remote_unified`](super::network::remote_unified)) stay behind the
//! [`RemoteComponentMedia`] seam for the host to wire; the seat presentation
//! ([`presentation`](super::presentation)) and the mod presentation sources stay generic.
//! Documented folds:
//! the donor's async dispatch/prepare/media calls are sync through the host; the
//! `queueCommand` closure the donor installs on the collection options is a shared
//! [`RemoteComponentQueue`] handle the host clones into its collection before building
//! the view; the dispatch outcome (`"handled" | "retired"`) is ignored like the donor
//! ignores it; collection cleanup failures are discarded because the seam
//! close returns `()` (the donor propagates them).

use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use thiserror::Error;

use super::mod_presentation::{ModPresentationBackend, ModPresentationSource, ModSceneRendererOps};
use super::mod_presentations::{
    ApplicationModPresentations, ComponentClientCommandRequest, ModCollectionSeat, ModPresentationsError,
    ModQ3EventPayload,
};
use super::presentation_state::{SimulationPresentationEvent, SourcePresentationEvent};

/// Component-client collection (donor `ApplicationModPresentations` subset).
pub trait RemoteComponentClients<F> {
    /// Queued command request (donor `ComponentClientCommandRequest`).
    type Command;
    /// Failure.
    type Error: std::error::Error;
    /// Viewing seat (donor `ViewingSeat`).
    type Presentation;
    /// Active mod presentation (donor `ActiveModPresentation`).
    type Source;

    /// Whether the collection holds queued commands (donor `pendingCommands`).
    fn pending_commands(&self) -> bool;
    /// Dispatch one queued command (donor `dispatchCommand`).
    fn dispatch_command(&mut self, request: Self::Command) -> Result<(), Self::Error>;
    /// Prepare the collection (donor `prepare`).
    fn prepare(
        &mut self,
        presentations: &[Self::Presentation],
        sources: &[Self::Source],
        events: &[SimulationPresentationEvent<F>],
        frame: i32,
    ) -> Result<(), Self::Error>;
    /// Close the collection (donor `close`).
    fn close(&mut self);
}

/// Remote media preparation (donor `UnifiedRemotePresentation` media surface plus
/// `preparePresentationAudio`/`preparePresentationShaders`).
pub trait RemoteComponentMedia<F> {
    /// Failure.
    type Error: std::error::Error;
    /// Active mod presentation (donor `ActiveModPresentation`).
    type Source;

    /// Prepare component audio over `events` (donor `preparePresentationAudio`).
    fn prepare_audio(&mut self, events: &[SimulationPresentationEvent<F>]) -> Result<(), Self::Error>;
    /// Prepare component shaders (donor `preparePresentationShaders`).
    fn prepare_shaders(&mut self) -> Result<(), Self::Error>;
    /// Current mod presentation sources (donor `modPresentationSources`).
    fn mod_presentation_sources(&self) -> Vec<Self::Source>;
}

/// Canonical component collection behind the clients seam (donor
/// `ApplicationModPresentations` as `RemoteComponentView["clients"]`).
impl<F: ModQ3EventPayload, B: ModPresentationBackend + 'static, S: ModCollectionSeat + 'static>
    RemoteComponentClients<F> for ApplicationModPresentations<B, S>
where
    B::Error: std::error::Error + 'static,
    B::Renderer: ModSceneRendererOps<B::Error>,
{
    /// Queued command request (donor `ComponentClientCommandRequest`).
    type Command = ComponentClientCommandRequest;
    /// Failure.
    type Error = ModPresentationsError<B::Error>;
    /// Viewing seat handle.
    type Presentation = Rc<RefCell<S>>;
    /// Active mod presentation.
    type Source = Rc<dyn ModPresentationSource>;

    fn pending_commands(&self) -> bool {
        self.pending_commands()
    }

    fn dispatch_command(&mut self, request: Self::Command) -> Result<(), Self::Error> {
        self.dispatch_command(&request).map(|_| ())
    }

    fn prepare(
        &mut self,
        presentations: &[Self::Presentation],
        sources: &[Self::Source],
        events: &[SimulationPresentationEvent<F>],
        frame: i32,
    ) -> Result<(), Self::Error> {
        self.prepare(presentations, sources, events, i64::from(frame))
    }

    fn close(&mut self) {
        let _ = self.close();
    }
}

/// View operation error.
#[derive(Debug, Error)]
pub enum RemoteComponentError<C, M> {
    /// The view is retired.
    #[error("Remote component view is retired")]
    Retired,
    /// Component collection failure.
    #[error(transparent)]
    Clients(C),
    /// Remote media failure.
    #[error(transparent)]
    Media(M),
}

/// Retired-view error for the queue handle, which knows no failure types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("Remote component view is retired")]
pub struct RetiredViewError;

/// Shared queued-command state (donor `commands` plus `closed`).
#[derive(Debug)]
struct SharedQueue<Q> {
    commands: Vec<Q>,
    closed: bool,
}

/// Shared command queue handle (donor `queueCommand` closure).
///
/// The host clones this into its [`RemoteComponentClients`] before building the view,
/// mirroring the closure the donor installs on the collection options.
#[derive(Debug)]
pub struct RemoteComponentQueue<Q> {
    shared: Rc<RefCell<SharedQueue<Q>>>,
}

impl<Q> Clone for RemoteComponentQueue<Q> {
    fn clone(&self) -> Self {
        Self {
            shared: Rc::clone(&self.shared),
        }
    }
}

impl<Q> Default for RemoteComponentQueue<Q> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Q> RemoteComponentQueue<Q> {
    /// Empty open queue.
    pub fn new() -> Self {
        Self {
            shared: Rc::new(RefCell::new(SharedQueue {
                commands: Vec::new(),
                closed: false,
            })),
        }
    }

    /// Queue a component command (donor `queueCommand`); fails once the view retires.
    pub fn queue_command(&self, request: Q) -> Result<(), RetiredViewError> {
        let mut shared = self.shared.borrow_mut();
        if shared.closed {
            return Err(RetiredViewError);
        }
        shared.commands.push(request);
        Ok(())
    }
}

/// One remote seat's component view (donor `RemoteComponentView`).
#[derive(Debug)]
pub struct RemoteComponentView<C, M, F, Q> {
    clients: C,
    media: M,
    queue: RemoteComponentQueue<Q>,
    marker: PhantomData<F>,
}

impl<C, M, F, Q> RemoteComponentView<C, M, F, Q>
where
    C: RemoteComponentClients<F, Command = Q>,
    M: RemoteComponentMedia<F, Source = C::Source>,
{
    /// Build the view over a host collection, remote media, and a shared queue.
    pub fn new(clients: C, media: M, queue: RemoteComponentQueue<Q>) -> Self {
        Self {
            clients,
            media,
            queue,
            marker: PhantomData,
        }
    }

    /// Whether commands are queued (donor `pendingCommands`).
    #[must_use]
    pub fn pending_commands(&self) -> bool {
        !self.queue.shared.borrow().commands.is_empty() || self.clients.pending_commands()
    }

    /// Drain queued commands, then prepare audio and shaders (donor `prepareMedia`).
    ///
    /// Without music only `presentation-owner` events reach audio, like the donor's filter.
    pub fn prepare_media(
        &mut self,
        events: &[SimulationPresentationEvent<F>],
        music: bool,
    ) -> Result<(), RemoteComponentError<C::Error, M::Error>>
    where
        F: Clone,
    {
        if self.queue.shared.borrow().closed {
            return Err(RemoteComponentError::Retired);
        }
        let commands = std::mem::take(&mut self.queue.shared.borrow_mut().commands);
        for command in commands {
            self.clients
                .dispatch_command(command)
                .map_err(RemoteComponentError::Clients)?;
        }
        if music {
            self.media.prepare_audio(events).map_err(RemoteComponentError::Media)?;
        } else {
            let owners: Vec<SimulationPresentationEvent<F>> = events
                .iter()
                .filter(|event| matches!(event.source, SourcePresentationEvent::PresentationOwner { .. }))
                .cloned()
                .collect();
            self.media.prepare_audio(&owners).map_err(RemoteComponentError::Media)?;
        }
        self.media.prepare_shaders().map_err(RemoteComponentError::Media)?;
        Ok(())
    }

    /// Prepare the collection over one seat presentation (donor `prepare`).
    pub fn prepare(
        &mut self,
        presentation: &C::Presentation,
        events: &[SimulationPresentationEvent<F>],
        frame: i32,
    ) -> Result<(), RemoteComponentError<C::Error, M::Error>> {
        if self.queue.shared.borrow().closed {
            return Err(RemoteComponentError::Retired);
        }
        let sources = self.media.mod_presentation_sources();
        self.clients
            .prepare(std::slice::from_ref(presentation), &sources, events, frame)
            .map_err(RemoteComponentError::Clients)?;
        Ok(())
    }

    /// Retire the view (donor `close`).
    pub fn close(&mut self) {
        {
            let mut shared = self.queue.shared.borrow_mut();
            if shared.closed {
                return;
            }
            shared.closed = true;
            shared.commands.clear();
        }
        self.clients.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{ContentId, PresentationOwner};
    use qa_core::identity::ProviderId;

    use super::super::presentation_state::OwnerLifecycle;

    #[derive(Debug)]
    struct TestError(&'static str);

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for TestError {}

    type PreparedCall = (Vec<String>, Vec<String>, Vec<i64>, i32);

    #[derive(Debug, Default)]
    struct StubClients {
        dispatched: Vec<String>,
        prepared: Vec<PreparedCall>,
        collection_pending: bool,
        closes: usize,
        fail_dispatch: bool,
    }

    impl RemoteComponentClients<()> for StubClients {
        type Command = String;
        type Error = TestError;
        type Presentation = String;
        type Source = String;

        fn pending_commands(&self) -> bool {
            self.collection_pending
        }

        fn dispatch_command(&mut self, request: String) -> Result<(), TestError> {
            if self.fail_dispatch {
                return Err(TestError("dispatch"));
            }
            self.dispatched.push(request);
            Ok(())
        }

        fn prepare(
            &mut self,
            presentations: &[String],
            sources: &[String],
            events: &[SimulationPresentationEvent<()>],
            frame: i32,
        ) -> Result<(), TestError> {
            self.prepared.push((
                presentations.to_vec(),
                sources.to_vec(),
                events.iter().map(|event| event.sequence).collect(),
                frame,
            ));
            Ok(())
        }

        fn close(&mut self) {
            self.closes += 1;
        }
    }

    #[derive(Debug, Default)]
    struct StubMedia {
        audio_batches: Vec<Vec<i64>>,
        shaders: usize,
        sources: Vec<String>,
    }

    impl RemoteComponentMedia<()> for StubMedia {
        type Error = TestError;
        type Source = String;

        fn prepare_audio(&mut self, events: &[SimulationPresentationEvent<()>]) -> Result<(), TestError> {
            self.audio_batches
                .push(events.iter().map(|event| event.sequence).collect());
            Ok(())
        }

        fn prepare_shaders(&mut self) -> Result<(), TestError> {
            self.shaders += 1;
            Ok(())
        }

        fn mod_presentation_sources(&self) -> Vec<String> {
            self.sources.clone()
        }
    }

    fn owner() -> PresentationOwner {
        PresentationOwner {
            provider: ProviderId::new("test", "rc"),
            generation: 1,
        }
    }

    fn event(source: SourcePresentationEvent<()>, sequence: i64) -> SimulationPresentationEvent<()> {
        SimulationPresentationEvent {
            source,
            owner: None,
            recipient: None,
            sequence,
            content: ContentId("q1:classic:id1:1".to_string()),
            seconds: 0.0,
            source_entity: None,
        }
    }

    fn owner_event() -> SimulationPresentationEvent<()> {
        event(
            SourcePresentationEvent::PresentationOwner {
                event: OwnerLifecycle::Refreshed { owner: owner() },
            },
            7,
        )
    }

    fn sky_event() -> SimulationPresentationEvent<()> {
        event(
            SourcePresentationEvent::Q1Sky {
                name: "sky1".to_string(),
            },
            9,
        )
    }

    fn view() -> (
        RemoteComponentView<StubClients, StubMedia, (), String>,
        RemoteComponentQueue<String>,
    ) {
        let queue = RemoteComponentQueue::new();
        let view = RemoteComponentView::new(StubClients::default(), StubMedia::default(), queue.clone());
        (view, queue)
    }

    #[test]
    fn pending_commands_covers_queue_and_collection() {
        let (view, queue) = view();
        assert!(!view.pending_commands());
        queue.queue_command("a".to_string()).unwrap();
        assert!(view.pending_commands());
    }

    #[test]
    fn collection_pending_commands_surface() {
        let queue = RemoteComponentQueue::new();
        let clients = StubClients {
            collection_pending: true,
            ..StubClients::default()
        };
        let view = RemoteComponentView::new(clients, StubMedia::default(), queue);
        assert!(view.pending_commands());
    }

    #[test]
    fn prepare_media_drains_queue_then_prepares() {
        let (mut view, queue) = view();
        queue.queue_command("a".to_string()).unwrap();
        queue.queue_command("b".to_string()).unwrap();
        view.prepare_media(&[owner_event(), sky_event()], true).unwrap();
        assert_eq!(view.clients.dispatched, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(view.media.audio_batches, vec![vec![7, 9]]);
        assert_eq!(view.media.shaders, 1);
        assert!(!view.pending_commands());
    }

    #[test]
    fn prepare_media_without_music_filters_to_owner_events() {
        let (mut view, _) = view();
        view.prepare_media(&[sky_event(), owner_event()], false).unwrap();
        assert_eq!(view.media.audio_batches, vec![vec![7]]);
        assert_eq!(view.media.shaders, 1);
    }

    #[test]
    fn prepare_forwards_single_presentation_and_sources() {
        let queue = RemoteComponentQueue::new();
        let media = StubMedia {
            sources: vec!["mod".to_string()],
            ..StubMedia::default()
        };
        let mut view = RemoteComponentView::new(StubClients::default(), media, queue);
        view.prepare(&"seat".to_string(), &[sky_event()], 41).unwrap();
        assert_eq!(
            view.clients.prepared,
            vec![(vec!["seat".to_string()], vec!["mod".to_string()], vec![9], 41)]
        );
    }

    #[test]
    fn client_failure_surfaces() {
        let (mut view, queue) = view();
        view.clients.fail_dispatch = true;
        queue.queue_command("a".to_string()).unwrap();
        assert!(matches!(
            view.prepare_media(&[], true),
            Err(RemoteComponentError::Clients(_))
        ));
    }

    #[test]
    fn close_retires_view() {
        let (mut view, queue) = view();
        queue.queue_command("a".to_string()).unwrap();
        view.close();
        assert!(matches!(queue.queue_command("b".to_string()), Err(RetiredViewError)));
        assert!(matches!(
            view.prepare_media(&[], true),
            Err(RemoteComponentError::Retired)
        ));
        assert!(matches!(
            view.prepare(&"seat".to_string(), &[], 1),
            Err(RemoteComponentError::Retired)
        ));
        assert!(!view.pending_commands());
        view.close();
        assert_eq!(view.clients.closes, 1);
    }
}
