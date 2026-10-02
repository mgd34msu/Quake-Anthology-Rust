//! Client renderer restarts between source frames on the retained client.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/video-restart.ts`
//! (`PreparedVideoPresentation`, `VideoRestartHost`, `ApplicationVideoRestart`,
//! `VideoGuestSeat`, `prepareVideoGuests`). Video requests run between
//! source frames on the retained client. The renderer reuses
//! [`super::renderer`]; commands reuse `qa_core`; capture, input, guests,
//! and windows (unported siblings) arrive as injected traits. Sync port:
//! the donor's async drain/guest reopen become sync host calls with the
//! same texts, ordering, and aggregate cleanup.

use qa_core::cmd_buffer::CommandBuffer;
use qa_core::cmd_buffer::CommandContext;
use qa_core::cmd_buffer::CommandDocumentation;
use qa_core::cmd_buffer::CommandOrigin;
use qa_core::cmd_buffer::Invocation;
use qa_core::cvar::CvarRegistry;
use thiserror::Error;

use super::renderer::NativeBackendFactory;
use super::renderer::NativeRenderBackend;
use super::renderer::NativeRenderWindow;
use super::renderer::NativeRenderer;
use super::renderer::NativeWindowFactory;
use super::renderer::RenderBackendKind;
use super::renderer::RenderImageRegistry;
use super::renderer::RendererError;

/// Video restart failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VideoRestartError {
    /// Request on a retired owner.
    #[error("Video restart owner has retired")]
    Retired,
    /// Double command registration.
    #[error("Video restart command already registered")]
    AlreadyRegistered,
    /// Command belongs to another owner.
    #[error("Video restart command belongs to another owner")]
    ForeignCommand,
    /// Reentrant drain.
    #[error("Video restart is already preparing")]
    AlreadyPreparing,
    /// Cancelled during preparation.
    #[error("Video restart was cancelled during preparation")]
    CancelledPreparing,
    /// Cancelled after publication.
    #[error("Video restart was cancelled after publication")]
    CancelledPublished,
    /// Publication failure (donor `AggregateError` member texts).
    #[error("Video restart publication failed")]
    PublicationFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Guest shutdown failure.
    #[error("Previous guest video shutdown failed")]
    GuestShutdownFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Guest reopen failure.
    #[error("Guest video reopen and cleanup failed")]
    GuestReopenFailed {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Host failure.
    #[error("{0}")]
    Host(String),
    /// Renderer failure.
    #[error("{0}")]
    Renderer(String),
}

impl From<RendererError> for VideoRestartError {
    fn from(error: RendererError) -> Self {
        Self::Renderer(error.to_string())
    }
}

/// Prepared video presentation (donor `PreparedVideoPresentation`).
pub trait PreparedVideoPresentation {
    /// Validate before publication.
    fn validate_publication(&self) -> Result<(), String>;
    /// Restart after publication.
    fn restart(&mut self) -> Result<(), String>;
}

/// Frame capture drain (absorbed `ApplicationCapture` subset).
pub trait VideoCapture {
    /// Whether a readback is pending.
    fn pending_readback(&self) -> bool;
    /// Drain pending captures.
    fn drain(&mut self);
}

/// Video restart host (donor `VideoRestartHost`).
pub trait VideoRestartHost<Window> {
    /// Frame capture, if any.
    fn capture(&mut self) -> Option<&mut dyn VideoCapture>;
    /// Worker execution override.
    fn render_worker(&self) -> Option<bool> {
        None
    }
    /// Prepare the video presentation.
    fn prepare(&mut self) -> Result<Option<Box<dyn PreparedVideoPresentation>>, String>;
    /// Publish a window to input.
    fn publish_window(&mut self, window: &Window);
    /// Run after publication.
    fn published(&mut self, kind: RenderBackendKind);
    /// Run after settling.
    fn settled(&mut self);
    /// Print diagnostics.
    fn print(&mut self, text: &str, source: Option<&CommandContext>);
}

/// Requested restart (donor `requested` row).
#[derive(Debug, Clone, PartialEq, Eq)]
struct VideoRequest {
    kind: RenderBackendKind,
    render_worker: bool,
    source: Option<CommandContext>,
}

/// Video requests between source frames on the retained client.
pub struct ApplicationVideoRestart<Window, Backend, Images, Host> {
    renderer: NativeRenderer<Window, Backend, Images>,
    host: Host,
    requested: Option<VideoRequest>,
    running: bool,
    closed: bool,
    registered: bool,
    completed: u64,
}

impl<Window, Backend, Images, Host> ApplicationVideoRestart<Window, Backend, Images, Host>
where
    Window: NativeRenderWindow + 'static,
    Backend: NativeRenderBackend + 'static,
    Images: RenderImageRegistry + 'static,
    Host: VideoRestartHost<Window> + 'static,
{
    /// Create a restart owner over a renderer and host.
    #[must_use]
    pub fn new(renderer: NativeRenderer<Window, Backend, Images>, host: Host) -> Self {
        Self {
            renderer,
            host,
            requested: None,
            running: false,
            closed: false,
            registered: false,
            completed: 0,
        }
    }

    /// Borrow the renderer.
    #[must_use]
    pub fn renderer(&self) -> &NativeRenderer<Window, Backend, Images> {
        &self.renderer
    }

    /// Whether a request is pending or running.
    #[must_use]
    pub fn pending(&self) -> bool {
        self.requested.is_some() || self.running
    }

    /// Completed drain count.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.completed
    }

    /// Request a restart (`None` keeps the current backend).
    pub fn request(
        &mut self,
        kind: Option<RenderBackendKind>,
        source: Option<CommandContext>,
    ) -> Result<(), VideoRestartError> {
        if self.closed {
            return Err(VideoRestartError::Retired);
        }
        let render_worker = self
            .host
            .render_worker()
            .unwrap_or_else(|| self.renderer.render_worker());
        self.requested = Some(VideoRequest {
            kind: kind.unwrap_or_else(|| self.renderer.window().backend_kind()),
            render_worker,
            source,
        });
        Ok(())
    }

    /// Register `vid_restart` through shared ownership.
    pub fn register(
        &mut self,
        commands: &mut CommandBuffer,
        cvars: &CvarRegistry,
        shared: &std::rc::Rc<std::cell::RefCell<Self>>,
    ) -> Result<(), VideoRestartError> {
        if self.registered {
            return Err(VideoRestartError::AlreadyRegistered);
        }
        let owner = std::rc::Rc::clone(shared);
        let registered = commands
            .register(
                "vid_restart",
                Some(std::rc::Rc::new(move |invocation: &mut Invocation| {
                    let mut this = owner.borrow_mut();
                    let mut origin = &invocation.source.origin;
                    while let CommandOrigin::Script { caller, .. } = origin {
                        origin = caller;
                    }
                    if matches!(origin, CommandOrigin::RemoteClient { .. }) {
                        this.host
                            .print("vid_restart is a local client command.\n", Some(&invocation.source));
                        return;
                    }
                    let args = invocation.args();
                    let kind = args.first().map(String::as_str);
                    if args.len() > 1 || kind.is_some_and(|kind| kind != "cpu" && kind != "gl") {
                        this.host.print("vid_restart [cpu|gl]\n", Some(&invocation.source));
                        return;
                    }
                    let kind = match kind {
                        Some("cpu") => Some(RenderBackendKind::Cpu),
                        Some("gl") => Some(RenderBackendKind::Gl),
                        _ => None,
                    };
                    let _ = this.request(kind, Some(invocation.source.clone()));
                })),
                Some(CommandDocumentation {
                    summary: "Restart the client renderer while retaining the current game.".to_string(),
                    usage: "vid_restart [cpu|gl]".to_string(),
                    examples: vec![
                        "vid_restart".to_string(),
                        "vid_restart cpu".to_string(),
                        "vid_restart gl".to_string(),
                    ],
                    allowed_values: None,
                }),
                cvars,
            )
            .map_err(|error| VideoRestartError::Host(error.to_string()))?;
        if !registered {
            return Err(VideoRestartError::ForeignCommand);
        }
        self.registered = true;
        Ok(())
    }

    /// Drain one pending request; true means a restart published.
    pub fn drain<WindowFactory, BackendFactory>(
        &mut self,
        windows: &mut WindowFactory,
        backends: &mut BackendFactory,
    ) -> Result<bool, VideoRestartError>
    where
        WindowFactory: NativeWindowFactory<Window = Window>,
        BackendFactory: NativeBackendFactory<Backend = Backend>,
    {
        if self.running {
            return Err(VideoRestartError::AlreadyPreparing);
        }
        let Some(request) = self.requested.clone() else {
            return Ok(false);
        };
        if self.closed || self.host.capture().is_some_and(|capture| capture.pending_readback()) {
            return Ok(false);
        }
        let VideoRequest {
            kind,
            render_worker,
            source,
        } = request;
        self.requested = None;
        self.running = true;
        let outcome = self.drain_inner(kind, render_worker, source.as_ref(), windows, backends);
        self.completed += 1;
        self.running = false;
        self.host.settled();
        outcome
    }

    #[allow(clippy::too_many_lines)]
    fn drain_inner<WindowFactory, BackendFactory>(
        &mut self,
        kind: RenderBackendKind,
        render_worker: bool,
        source: Option<&CommandContext>,
        windows: &mut WindowFactory,
        backends: &mut BackendFactory,
    ) -> Result<bool, VideoRestartError>
    where
        WindowFactory: NativeWindowFactory<Window = Window>,
        BackendFactory: NativeBackendFactory<Backend = Backend>,
    {
        if let Some(capture) = self.host.capture() {
            capture.drain();
        }
        if let Err(error) = self.renderer.prepare_restart(kind, render_worker, windows, backends) {
            let error = VideoRestartError::from(error);
            self.reject(&error, source);
            return Ok(false);
        }
        let mut presentation = match self.host.prepare() {
            Err(error) => {
                let error = VideoRestartError::Host(error);
                self.reject(&error, source);
                return Ok(false);
            }
            Ok(presentation) => presentation,
        };
        if self.closed {
            let error = VideoRestartError::CancelledPreparing;
            self.reject(&error, source);
            return Ok(false);
        }
        let mut published = false;
        let mut retirement: Option<usize> = None;
        let mut input_published = false;
        let mut failed: Option<VideoRestartError> = None;
        if let Some(presentation) = presentation.as_ref() {
            if let Err(error) = presentation.validate_publication().map_err(VideoRestartError::Host) {
                failed = Some(error);
            }
        }
        if failed.is_none() {
            // Publish the prepared window to input before the renderer swap.
            let window = self.renderer.prepared_window().expect("prepared above");
            let host = &mut self.host;
            host.publish_window(window);
            input_published = true;
        }
        if failed.is_none() {
            match self.renderer.publish_prepared_restart() {
                Ok(id) => {
                    published = true;
                    retirement = Some(id);
                }
                Err(error) => failed = Some(error.into()),
            }
        }
        if failed.is_none() {
            if let Some(presentation) = presentation.as_mut() {
                if let Err(error) = presentation.restart().map_err(VideoRestartError::Host) {
                    failed = Some(error);
                }
            }
        }
        if failed.is_none() {
            if self.closed {
                failed = Some(VideoRestartError::CancelledPublished);
            } else {
                self.host.published(kind);
                if let Some(id) = retirement {
                    if let Err(error) = self.renderer.retire_resources(id) {
                        failed = Some(error.into());
                    } else {
                        retirement = None;
                    }
                }
            }
        }
        match failed {
            None => {
                self.host.print(
                    &format!(
                        "Renderer restarted ({}).\n",
                        match kind {
                            RenderBackendKind::Cpu => "cpu",
                            RenderBackendKind::Gl => "gl",
                        }
                    ),
                    source,
                );
                Ok(true)
            }
            Some(error) => {
                let mut failures = vec![error.to_string()];
                if !published {
                    if input_published {
                        // The renderer still owns the previous window; republish it.
                        self.host.publish_window(self.renderer.window());
                    }
                    if let Err(cleanup) = self.renderer.discard_prepared_restart() {
                        failures.push(cleanup.to_string());
                    }
                } else if let Some(id) = retirement {
                    if let Err(cleanup) = self.renderer.retire_resources(id) {
                        failures.push(cleanup.to_string());
                    }
                }
                if published || failures.len() > 1 {
                    return Err(VideoRestartError::PublicationFailed { errors: failures });
                }
                self.host.print(&format!("Video restart rejected: {error}\n"), source);
                Ok(false)
            }
        }
    }

    fn reject(&mut self, error: &VideoRestartError, source: Option<&CommandContext>) {
        // Best-effort discard mirrors the donor's unpublished cleanup.
        let _ = self.renderer.discard_prepared_restart();
        self.host.print(&format!("Video restart rejected: {error}\n"), source);
    }

    /// Close the owner.
    pub fn close(&mut self, commands: &mut CommandBuffer) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.requested = None;
        if self.registered {
            commands.unregister("vid_restart");
            self.registered = false;
        }
    }
}

/// Guest video seat (donor `VideoGuestSeat`).
pub trait VideoGuestSeat {
    /// Shut down the guest client.
    fn shutdown_guest(&mut self) -> Result<(), String>;
    /// Close the guest client.
    fn close_guest(&mut self) -> Result<(), String>;
    /// Reopen the guest on the current window.
    fn reopen_guest(&mut self) -> Result<(), String>;
    /// Validate before publication.
    fn assert_current(&self) -> Result<(), String>;
}

/// Guest video presentation over owned seats.
pub struct VideoGuestPresentation {
    seats: Vec<Box<dyn VideoGuestSeat>>,
}

impl VideoGuestPresentation {
    /// Reclaim the seats.
    #[must_use]
    pub fn into_seats(self) -> Vec<Box<dyn VideoGuestSeat>> {
        self.seats
    }
}

impl PreparedVideoPresentation for VideoGuestPresentation {
    fn validate_publication(&self) -> Result<(), String> {
        for seat in &self.seats {
            seat.assert_current()?;
        }
        Ok(())
    }

    fn restart(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        for seat in &mut self.seats {
            if let Err(error) = seat.shutdown_guest() {
                failures.push(error);
            }
            if let Err(error) = seat.close_guest() {
                failures.push(error);
            }
        }
        if !failures.is_empty() {
            return Err(VideoRestartError::GuestShutdownFailed { errors: failures }.to_string());
        }
        for seat in &mut self.seats {
            if let Err(error) = seat.reopen_guest() {
                return Err(VideoRestartError::GuestReopenFailed { errors: vec![error] }.to_string());
            }
        }
        Ok(())
    }
}

/// Prepare guest video restarts across seats (`None` when empty).
#[must_use]
pub fn prepare_video_guests(seats: Vec<Box<dyn VideoGuestSeat>>) -> Option<VideoGuestPresentation> {
    if seats.is_empty() {
        None
    } else {
        Some(VideoGuestPresentation { seats })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::cmd::Dialect;
    use qa_core::cmd_buffer::BufferOptions;
    use qa_core::identity::IdentityOwner;

    use super::super::renderer::ImageLevel;
    use super::super::renderer::NativeBackendFactory;
    use super::super::renderer::NativeRendererOptions;
    use super::super::renderer::NativeWindowFactory;
    use super::super::renderer::RenderImageRegistry;
    use super::super::renderer::RenderWindowOptions;
    use super::super::renderer::RenderWindowPresentation;
    use super::super::renderer::RendererResourceOwner;
    use super::*;

    struct FakeWindow {
        kind: RenderBackendKind,
        width: i32,
        height: i32,
    }

    impl NativeRenderWindow for FakeWindow {
        type Error = String;

        fn backend_kind(&self) -> RenderBackendKind {
            self.kind
        }

        fn width(&self) -> i32 {
            self.width
        }

        fn height(&self) -> i32 {
            self.height
        }

        fn drawable_size(&self) -> (i32, i32) {
            (self.width, self.height)
        }

        fn flags(&self) -> u32 {
            0
        }

        fn display_modes(&self) -> Vec<qa_platform::sdl::SdlDisplayMode> {
            Vec::new()
        }

        fn swap_interval(&self) -> i32 {
            1
        }

        fn set_swap_interval(&mut self, _interval: i32) -> Result<(), String> {
            Ok(())
        }

        fn set_visible(&mut self, _visible: bool) -> Result<(), String> {
            Ok(())
        }

        fn make_current(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn present_pixels(&mut self, _pixels: &[u8]) -> Result<(), String> {
            Ok(())
        }

        fn capture_presentation(&self) -> RenderWindowPresentation {
            RenderWindowPresentation {
                width: self.width,
                height: self.height,
                display_index: 0,
                position: (0, 0),
            }
        }

        fn restore_presentation(&mut self, presentation: &RenderWindowPresentation) -> Result<(), String> {
            self.width = presentation.width;
            self.height = presentation.height;
            Ok(())
        }

        fn close_window(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeBackend {
        width: i32,
        height: i32,
        software: bool,
    }

    impl NativeRenderBackend for FakeBackend {
        type Error = String;

        fn width(&self) -> i32 {
            self.width
        }

        fn height(&self) -> i32 {
            self.height
        }

        fn is_worker(&self) -> bool {
            false
        }

        fn is_software(&self) -> bool {
            self.software
        }

        fn set_output_gamma(&mut self, _gamma: f64) -> Result<(), String> {
            Ok(())
        }

        fn apply_image_resource(&mut self, _operation: &qa_client::render::types::ImageResourceOperation) {}

        fn execute_serial_command(&mut self, _command: &super::super::renderer::RenderCommand) {}

        fn execute_worker(
            &mut self,
            _commands: &[super::super::renderer::RenderCommand],
            _captures: &[super::super::renderer::CaptureId],
            _operations: Vec<qa_client::render::types::ImageResourceOperation>,
        ) {
        }

        fn synchronize(&mut self) {}

        fn set_worker_swap_interval(&mut self, _interval: i32) {}

        fn worker_swap_interval(&self) -> i32 {
            1
        }

        fn resize_backend(&mut self, width: i32, height: i32) {
            self.width = width;
            self.height = height;
        }

        fn finish_software(&mut self) {}

        fn software_pixels(&self) -> Vec<u8> {
            Vec::new()
        }

        fn read_pixels_backend(&mut self) -> ImageLevel {
            ImageLevel {
                width: 1,
                height: 1,
                pixels: vec![0],
            }
        }

        fn present_backend(&mut self) {}

        fn driver(&self) -> Option<super::super::renderer::RenderDriverInfo> {
            None
        }

        fn gl_config(&self) -> Option<super::super::renderer::RenderGlConfig> {
            None
        }

        fn close_backend(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeImages;

    impl RenderImageRegistry for FakeImages {
        type Error = String;

        fn owner(&self) -> &RendererResourceOwner {
            static OWNER: std::sync::OnceLock<RendererResourceOwner> = std::sync::OnceLock::new();
            OWNER.get_or_init(|| RendererResourceOwner {
                identity: "video".to_string(),
                session: 1,
                generation: 1,
            })
        }

        fn drain_pending_operations(&mut self) -> Vec<qa_client::render::types::ImageResourceOperation> {
            Vec::new()
        }

        fn close_images(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    struct FakeWindows;

    struct FakeBackends;

    impl NativeWindowFactory for FakeWindows {
        type Window = FakeWindow;
        type Error = String;

        fn open_window(&mut self, options: &RenderWindowOptions) -> Result<FakeWindow, String> {
            Ok(FakeWindow {
                kind: options.backend,
                width: options.width,
                height: options.height,
            })
        }
    }

    impl NativeBackendFactory for FakeBackends {
        type Backend = FakeBackend;
        type Error = String;

        fn open_backend(
            &mut self,
            kind: RenderBackendKind,
            width: i32,
            height: i32,
            _gamma: f64,
            _worker: bool,
        ) -> Result<FakeBackend, String> {
            Ok(FakeBackend {
                width,
                height,
                software: kind == RenderBackendKind::Cpu,
            })
        }
    }

    struct FakePresentation {
        fail_restart: bool,
    }

    impl PreparedVideoPresentation for FakePresentation {
        fn validate_publication(&self) -> Result<(), String> {
            Ok(())
        }

        fn restart(&mut self) -> Result<(), String> {
            if self.fail_restart {
                return Err("guest restart failed".to_string());
            }
            Ok(())
        }
    }

    struct FakeHost {
        printed: Vec<String>,
        published: Vec<RenderBackendKind>,
        settled: usize,
        fail_prepare: bool,
        fail_restart: bool,
        worker: Option<bool>,
    }

    impl VideoRestartHost<FakeWindow> for FakeHost {
        fn capture(&mut self) -> Option<&mut dyn VideoCapture> {
            None
        }

        fn render_worker(&self) -> Option<bool> {
            self.worker
        }

        fn prepare(&mut self) -> Result<Option<Box<dyn PreparedVideoPresentation>>, String> {
            if self.fail_prepare {
                return Err("prepare failed".to_string());
            }
            Ok(Some(Box::new(FakePresentation {
                fail_restart: self.fail_restart,
            })))
        }

        fn publish_window(&mut self, window: &FakeWindow) {
            self.published.push(window.backend_kind());
        }

        fn published(&mut self, _kind: RenderBackendKind) {}

        fn settled(&mut self) {
            self.settled += 1;
        }

        fn print(&mut self, text: &str, _source: Option<&CommandContext>) {
            self.printed.push(text.to_string());
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("video-restart-test").unwrap()
    }

    fn restart(
        fail_prepare: bool,
        fail_restart: bool,
    ) -> (
        ApplicationVideoRestart<FakeWindow, FakeBackend, FakeImages, FakeHost>,
        FakeWindows,
        FakeBackends,
    ) {
        let mut windows = FakeWindows;
        let mut backends = FakeBackends;
        let renderer = NativeRenderer::open(
            &NativeRendererOptions {
                renderer: RenderBackendKind::Cpu,
                width: 640,
                height: 480,
                hidden: false,
                gamma: 1.0,
                render_worker: false,
            },
            RendererResourceOwner {
                identity: "video".to_string(),
                session: 1,
                generation: 1,
            },
            FakeImages,
            &mut windows,
            &mut backends,
        )
        .unwrap();
        let host = FakeHost {
            printed: Vec::new(),
            published: Vec::new(),
            settled: 0,
            fail_prepare,
            fail_restart,
            worker: None,
        };
        (ApplicationVideoRestart::new(renderer, host), windows, backends)
    }

    #[test]
    fn drains_a_request_and_reports() {
        let (mut restart, mut windows, mut backends) = restart(false, false);
        assert!(!restart.pending());
        assert_eq!(restart.generation(), 0);
        assert!(!restart.drain(&mut windows, &mut backends).unwrap());
        restart.request(Some(RenderBackendKind::Cpu), None).unwrap();
        assert!(restart.pending());
        assert!(restart.drain(&mut windows, &mut backends).unwrap());
        assert_eq!(restart.generation(), 1);
        assert!(!restart.pending());
    }

    #[test]
    fn unpublished_failures_reject_without_throwing() {
        let (mut restart, mut windows, mut backends) = restart(true, false);
        restart.request(None, None).unwrap();
        assert!(!restart.drain(&mut windows, &mut backends).unwrap());
        assert_eq!(restart.generation(), 1);
    }

    #[test]
    fn published_failures_aggregate() {
        let (mut restart, mut windows, mut backends) = restart(false, true);
        restart.request(None, None).unwrap();
        let error = restart.drain(&mut windows, &mut backends).err().unwrap();
        assert!(matches!(error, VideoRestartError::PublicationFailed { .. }));
        assert_eq!(restart.generation(), 1);
    }

    #[test]
    fn registers_and_closes_the_command() {
        let (restart, _, _) = restart(false, false);
        let shared = Rc::new(RefCell::new(restart));
        let command_owner = owner();
        let context = CommandContext::new(command_owner.session().clone(), CommandOrigin::LocalConsole);
        let mut commands = CommandBuffer::new(Dialect::Q3, context, BufferOptions::new()).unwrap();
        let cvars = CvarRegistry::new(Dialect::Q3);
        shared.borrow_mut().register(&mut commands, &cvars, &shared).unwrap();
        assert_eq!(
            shared.borrow_mut().register(&mut commands, &cvars, &shared),
            Err(VideoRestartError::AlreadyRegistered)
        );
        shared.borrow_mut().close(&mut commands);
        assert_eq!(shared.borrow_mut().request(None, None), Err(VideoRestartError::Retired));
        let _ = owner();
    }

    struct FakeSeat {
        fail_shutdown: bool,
        fail_reopen: bool,
    }

    impl VideoGuestSeat for FakeSeat {
        fn shutdown_guest(&mut self) -> Result<(), String> {
            if self.fail_shutdown {
                return Err("shutdown failed".to_string());
            }
            Ok(())
        }

        fn close_guest(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn reopen_guest(&mut self) -> Result<(), String> {
            if self.fail_reopen {
                return Err("reopen failed".to_string());
            }
            Ok(())
        }

        fn assert_current(&self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn guests_restart_and_reclaim() {
        assert!(prepare_video_guests(Vec::new()).is_none());
        let mut presentation = prepare_video_guests(vec![Box::new(FakeSeat {
            fail_shutdown: false,
            fail_reopen: false,
        }) as Box<dyn VideoGuestSeat>])
        .unwrap();
        presentation.validate_publication().unwrap();
        presentation.restart().unwrap();
        assert_eq!(presentation.into_seats().len(), 1);

        let mut presentation = prepare_video_guests(vec![Box::new(FakeSeat {
            fail_shutdown: true,
            fail_reopen: false,
        }) as Box<dyn VideoGuestSeat>])
        .unwrap();
        assert!(presentation.restart().is_err());

        let mut presentation = prepare_video_guests(vec![Box::new(FakeSeat {
            fail_shutdown: false,
            fail_reopen: true,
        }) as Box<dyn VideoGuestSeat>])
        .unwrap();
        assert!(presentation.restart().is_err());
    }
}
