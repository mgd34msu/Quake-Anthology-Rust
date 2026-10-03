//! Native renderer ownership: window, backend, journal, and frame captures.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/renderer.ts`
//! (`RendererDiagnostics`, `PreparedRendererRestart`, `NativeRenderer`).
//! Owns the native surface, image identity, and ordered serial or worker
//! execution. Windows ([`SdlWindow`](qa_platform::sdl::SdlWindow)), backends
//! ([`GlRenderer`](qa_client::render::gl::renderer::GlRenderer),
//! [`CpuRenderer`](qa_client::render::cpu::rasterizer::CpuRenderer),
//! [`RenderExecutor`](qa_client::render::execution::RenderExecutor); the worker
//! side is ported ([`worker`](qa_client::render::worker)) and the backend
//! union stays absorbed behind [`NativeRenderBackend`]), and the image registry
//! ([`SceneImageRegistry`](qa_client::render::scene::resources::SceneImageRegistry))
//! arrive as injected traits; the image journal, display modes, and math reuse
//! `qa_client`, `qa_platform`, and `qa_core`. Sync port: the donor's async
//! open/restart/close become sync
//! factory calls with the same aggregate cleanup texts; frame-capture
//! promises become one-shot callbacks; the restart `publish`/`discard`
//! closures become id-checked renderer methods; worker context parking is a
//! backend-internal no-op.

use std::collections::HashMap;

use qa_client::render::image_journal::ImageDescription;
use qa_client::render::image_journal::RenderImageJournal;
use qa_client::render::types::{
    DrawBuffer as ClientDrawBuffer, ImageResourceOperation, RenderView as ClientRenderView,
};
use qa_core::math::Vec4;
use qa_platform::sdl::SdlDisplayMode;
use thiserror::Error;

/// Native renderer failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RendererError {
    /// Operation on a closed renderer.
    #[error("Native renderer is closed")]
    Closed,
    /// Mutation while a restart is prepared.
    #[error("Native renderer restart is prepared")]
    RestartPrepared,
    /// Image registry belongs to another owner.
    #[error("Renderer image registry belongs to another owner")]
    ForeignRegistry,
    /// Frame belongs to another renderer lifetime.
    #[error("Frame belongs to another renderer lifetime")]
    ForeignFrame,
    /// Unsupported swap interval.
    #[error("SDL returned an unsupported swap interval")]
    BadSwapInterval,
    /// Prepared drawable dimensions changed.
    #[error("Prepared renderer drawable dimensions changed")]
    DimensionsChanged,
    /// Closed during restart preparation.
    #[error("Renderer closed during restart preparation")]
    ClosedDuringRestart,
    /// Publish of a stale restart.
    #[error("Renderer restart is no longer prepared")]
    StaleRestart,
    /// Worker swap on the wrong thread.
    #[error("Worker swaps execute on their owning thread")]
    WorkerSwap,
    /// CPU presentation without pixels.
    #[error("CPU renderer presentation has no pixels")]
    NoCpuPixels,
    /// Capture completion without pixels.
    #[error("Renderer capture has no pixels")]
    NoCapturePixels,
    /// GL pixel read without a capture request.
    #[error("GL captures must be requested before presentation with captureNextFrame()")]
    GlCaptureOrder,
    /// CPU backend unavailable.
    #[error("CPU renderer backend is unavailable")]
    NoCpuBackend,
    /// Frame capture aborted.
    #[error("Frame capture aborted")]
    CaptureAborted,
    /// Renderer closed before presentation.
    #[error("Renderer closed before the requested frame was presented")]
    ClosedBeforePresent,
    /// Capture failure.
    #[error("{0}")]
    Capture(String),
    /// Opening cleanup failure (donor `AggregateError` member texts).
    #[error("Renderer opening and cleanup failed")]
    OpenCleanup {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Restart preparation cleanup failure.
    #[error("Renderer restart preparation and cleanup failed")]
    RestartCleanup {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Restart publication cleanup failure.
    #[error("Renderer restart publication and restoration failed")]
    PublishCleanup {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Resource retirement failure.
    #[error("Renderer resource retirement failed")]
    RetireCleanup {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Native cleanup failure.
    #[error("Native renderer cleanup failed")]
    CloseCleanup {
        /// Member failure messages.
        errors: Vec<String>,
    },
    /// Window failure.
    #[error("{0}")]
    Window(String),
    /// Backend failure.
    #[error("{0}")]
    Backend(String),
    /// Image registry failure.
    #[error("{0}")]
    Images(String),
}

/// Window backend kind (donor `"cpu" | "gl"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderBackendKind {
    /// Software rasterizer.
    Cpu,
    /// OpenGL.
    Gl,
}

/// Renderer resource owner identity (donor `RendererResourceOwner`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererResourceOwner {
    /// Owner identity.
    pub identity: String,
    /// Owner session.
    pub session: u64,
    /// Owner generation.
    pub generation: u64,
}

/// One captured image level (donor `ImageLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageLevel {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel bytes.
    pub pixels: Vec<u8>,
}

/// Frame capture id.
pub type CaptureId = u64;

/// One-shot frame capture completion.
pub type CaptureHandler = Box<dyn FnOnce(Result<ImageLevel, String>)>;

/// Pending frame captures (donor `FrameCaptures`, sync callbacks).
#[derive(Default)]
pub struct FrameCaptures {
    next_id: CaptureId,
    armed: Vec<CaptureId>,
    failure: Option<String>,
    pending: HashMap<CaptureId, CaptureHandler>,
}

impl FrameCaptures {
    /// Create empty captures.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drain armed capture ids.
    pub fn take(&mut self) -> Vec<CaptureId> {
        std::mem::take(&mut self.armed)
    }

    /// Complete captures with a pixel copy.
    pub fn complete(&mut self, image: Option<&ImageLevel>, ids: &[CaptureId]) -> Result<(), RendererError> {
        if !ids.is_empty() && image.is_none() {
            return Err(RendererError::NoCapturePixels);
        }
        let Some(image) = image else {
            return Ok(());
        };
        for id in ids {
            if let Some(handler) = self.pending.remove(id) {
                handler(Ok(image.clone()));
            }
        }
        Ok(())
    }

    /// Request a capture; fails fast after `fail`.
    pub fn request(&mut self, handler: CaptureHandler) -> Result<CaptureId, RendererError> {
        if let Some(failure) = &self.failure {
            return Err(RendererError::Capture(failure.clone()));
        }
        self.next_id += 1;
        let id = self.next_id;
        self.pending.insert(id, handler);
        self.armed.push(id);
        Ok(id)
    }

    /// Cancel a capture (donor `AbortSignal`).
    pub fn cancel(&mut self, id: CaptureId) -> bool {
        let handler = self.pending.remove(&id);
        self.armed.retain(|armed| *armed != id);
        if let Some(handler) = handler {
            handler(Err(RendererError::CaptureAborted.to_string()));
            true
        } else {
            false
        }
    }

    /// Fail all pending captures.
    pub fn fail(&mut self, reason: &str) {
        if self.failure.is_none() {
            self.failure = Some(reason.to_string());
        }
        for (_, handler) in self.pending.drain() {
            handler(Err(self.failure.clone().expect("just set")));
        }
        self.armed.clear();
    }

    /// Fail captures for renderer close.
    pub fn close(&mut self) {
        self.fail(&RendererError::ClosedBeforePresent.to_string());
    }
}

/// Window presentation snapshot (donor `capturePresentation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderWindowPresentation {
    /// Drawable width.
    pub width: i32,
    /// Drawable height.
    pub height: i32,
    /// Display index.
    pub display_index: i32,
    /// Window position.
    pub position: (i32, i32),
}

/// Window open options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderWindowOptions {
    /// Backend kind.
    pub backend: RenderBackendKind,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Hidden window.
    pub hidden: bool,
    /// Resizable window.
    pub resizable: bool,
    /// Display index.
    pub display_index: i32,
    /// Window position.
    pub position: (i32, i32),
}

/// Native window (absorbed `SdlWindow` surface).
pub trait NativeRenderWindow {
    /// Backend failure.
    type Error: std::fmt::Display;
    /// Backend kind.
    fn backend_kind(&self) -> RenderBackendKind;
    /// Window width.
    fn width(&self) -> i32;
    /// Window height.
    fn height(&self) -> i32;
    /// Drawable size.
    fn drawable_size(&self) -> (i32, i32);
    /// Window flags.
    fn flags(&self) -> u32;
    /// Display modes.
    fn display_modes(&self) -> Vec<SdlDisplayMode>;
    /// Swap interval.
    fn swap_interval(&self) -> i32;
    /// Set the swap interval.
    fn set_swap_interval(&mut self, interval: i32) -> Result<(), Self::Error>;
    /// Show or hide the window.
    fn set_visible(&mut self, visible: bool) -> Result<(), Self::Error>;
    /// Make the GL context current.
    fn make_current(&mut self) -> Result<(), Self::Error>;
    /// Present CPU pixels.
    fn present_pixels(&mut self, pixels: &[u8]) -> Result<(), Self::Error>;
    /// Capture the presentation snapshot.
    fn capture_presentation(&self) -> RenderWindowPresentation;
    /// Restore a presentation snapshot.
    fn restore_presentation(&mut self, presentation: &RenderWindowPresentation) -> Result<(), Self::Error>;
    /// Close the window.
    fn close_window(&mut self) -> Result<(), Self::Error>;
}

/// Native window factory (absorbed `SdlWindow.open`).
pub trait NativeWindowFactory {
    /// Window handle.
    type Window: NativeRenderWindow;
    /// Factory failure.
    type Error: std::fmt::Display;
    /// Open a window.
    fn open_window(&mut self, options: &RenderWindowOptions) -> Result<Self::Window, Self::Error>;
}

/// GL driver description (donor `GlRenderer["driver"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderDriverInfo {
    /// Vendor string.
    pub vendor: String,
    /// Renderer string.
    pub renderer: String,
    /// Version string.
    pub version: String,
    /// Shading language string.
    pub shading_language: String,
}

/// GL configuration (donor `glConfig` pick).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderGlConfig {
    /// Maximum texture size.
    pub max_texture_size: i32,
    /// Texture units.
    pub texture_units: i32,
    /// Color bits.
    pub color_bits: i32,
    /// Depth bits.
    pub depth_bits: i32,
    /// Stereo enabled.
    pub stereo_enabled: bool,
}

/// One render command (donor `RenderFrame["commands"]`).
#[derive(Debug, Clone, PartialEq)]
pub enum RenderCommand {
    /// Set the 2D drawing color.
    SetColor {
        /// Drawing color.
        color: Vec4,
    },
    /// Swap buffers.
    SwapBuffers,
    /// Opaque draw command for the backend.
    Draw,
    /// Select the draw buffer (donor `draw-buffer`).
    DrawBuffer {
        /// Target buffer.
        buffer: ClientDrawBuffer,
        /// Clear after selecting.
        clear: bool,
    },
    /// Render one ordered view: scene entities plus view/camera state
    /// (donor `view`).
    View(ClientRenderView),
}

/// One render frame (donor `RenderFrame`).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderFrame {
    /// Frame owner lifetime.
    pub owner: RendererResourceOwner,
    /// Frame commands.
    pub commands: Vec<RenderCommand>,
}

/// Native backend (absorbed software/GL/worker union).
pub trait NativeRenderBackend {
    /// Backend failure.
    type Error: std::fmt::Display;
    /// Backend width.
    fn width(&self) -> i32;
    /// Backend height.
    fn height(&self) -> i32;
    /// Whether the backend executes on a worker thread.
    fn is_worker(&self) -> bool;
    /// Whether the backend is the software rasterizer.
    fn is_software(&self) -> bool;
    /// Set the output gamma.
    fn set_output_gamma(&mut self, gamma: f64) -> Result<(), Self::Error>;
    /// Apply one image resource operation.
    fn apply_image_resource(&mut self, operation: &ImageResourceOperation);
    /// Execute one serial command.
    fn execute_serial_command(&mut self, command: &RenderCommand);
    /// Execute worker commands with captures and image operations.
    fn execute_worker(
        &mut self,
        commands: &[RenderCommand],
        captures: &[CaptureId],
        operations: Vec<ImageResourceOperation>,
    );
    /// Synchronize worker execution.
    fn synchronize(&mut self);
    /// Set the worker swap interval.
    fn set_worker_swap_interval(&mut self, interval: i32);
    /// Worker swap interval description.
    fn worker_swap_interval(&self) -> i32;
    /// Resize a worker backend.
    fn resize_backend(&mut self, width: i32, height: i32);
    /// Finish software rendering.
    fn finish_software(&mut self);
    /// Software pixel buffer.
    fn software_pixels(&self) -> Vec<u8>;
    /// Read back pixels.
    fn read_pixels_backend(&mut self) -> ImageLevel;
    /// Present GL output.
    fn present_backend(&mut self);
    /// GL driver description, if any.
    fn driver(&self) -> Option<RenderDriverInfo>;
    /// GL configuration, if any.
    fn gl_config(&self) -> Option<RenderGlConfig>;
    /// Close the backend.
    fn close_backend(&mut self) -> Result<(), Self::Error>;
}

/// Native backend factory (absorbed `openBackend`).
pub trait NativeBackendFactory {
    /// Backend handle.
    type Backend: NativeRenderBackend;
    /// Factory failure.
    type Error: std::fmt::Display;
    /// Open a backend for a window kind.
    fn open_backend(
        &mut self,
        kind: RenderBackendKind,
        width: i32,
        height: i32,
        gamma: f64,
        worker: bool,
    ) -> Result<Self::Backend, Self::Error>;
}

/// Image registry (absorbed `SceneImageRegistry` subset).
pub trait RenderImageRegistry {
    /// Registry failure.
    type Error: std::fmt::Display;
    /// Registry owner.
    fn owner(&self) -> &RendererResourceOwner;
    /// Drain pending image operations.
    fn drain_pending_operations(&mut self) -> Vec<ImageResourceOperation>;
    /// Close the registry.
    fn close_images(&mut self) -> Result<(), Self::Error>;
}

/// Renderer diagnostics (donor `RendererDiagnostics`).
#[derive(Debug, Clone, PartialEq)]
pub struct RendererDiagnostics {
    /// Backend kind.
    pub backend: RenderBackendKind,
    /// Backend width.
    pub width: i32,
    /// Backend height.
    pub height: i32,
    /// GL driver description, if any.
    pub driver: Option<RenderDriverInfo>,
    /// Display modes.
    pub display_modes: Vec<SdlDisplayMode>,
    /// Journaled images.
    pub images: Vec<ImageDescription>,
}

/// Renderer open options (donor `NativeRenderer.open` options).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NativeRendererOptions {
    /// Backend kind.
    pub renderer: RenderBackendKind,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Hidden window.
    pub hidden: bool,
    /// Output gamma.
    pub gamma: f64,
    /// Worker execution.
    pub render_worker: bool,
}

/// Prepared restart phase (donor `"prepared" | "published" | "discarded"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestartPhase {
    Prepared,
    Published,
    Discarded,
}

/// One prepared restart (donor `PreparedRendererRestart`).
struct PreparedRestart<Window, Backend> {
    window: Window,
    backend: Backend,
    journal: RenderImageJournal,
    phase: RestartPhase,
}

/// Retired window/backend pair awaiting disposal.
struct RetiredResources<Window, Backend> {
    window: Option<Window>,
    backend: Option<Backend>,
    retired: bool,
}

/// Owns the native surface, image identity, and ordered execution.
pub struct NativeRenderer<Window, Backend, Images> {
    window: Window,
    owner: RendererResourceOwner,
    backend: Backend,
    gamma: f64,
    images: Images,
    journal: RenderImageJournal,
    captures: FrameCaptures,
    color: Vec4,
    closed: bool,
    preparing: bool,
    pending_restart: Option<PreparedRestart<Window, Backend>>,
    retirements: Vec<RetiredResources<Window, Backend>>,
    interval: i32,
}

impl<Window, Backend, Images> NativeRenderer<Window, Backend, Images>
where
    Window: NativeRenderWindow,
    Backend: NativeRenderBackend,
    Images: RenderImageRegistry,
{
    /// Open a renderer over factory-built resources.
    pub fn open<WindowFactory, BackendFactory>(
        options: &NativeRendererOptions,
        owner: RendererResourceOwner,
        images: Images,
        windows: &mut WindowFactory,
        backends: &mut BackendFactory,
    ) -> Result<Self, RendererError>
    where
        WindowFactory: NativeWindowFactory<Window = Window>,
        BackendFactory: NativeBackendFactory<Backend = Backend>,
    {
        if images.owner() != &owner {
            return Err(RendererError::ForeignRegistry);
        }
        let mut window = windows
            .open_window(&RenderWindowOptions {
                backend: options.renderer,
                width: options.width,
                height: options.height,
                hidden: options.hidden,
                resizable: true,
                display_index: 0,
                position: (0, 0),
            })
            .map_err(|error| RendererError::Window(error.to_string()))?;
        let journal = RenderImageJournal::default();
        let captures = FrameCaptures::new();
        let opened: Result<Backend, RendererError> = (|| {
            if options.renderer == RenderBackendKind::Gl {
                window
                    .set_swap_interval(1)
                    .map_err(|error| RendererError::Window(error.to_string()))?;
            }
            backends
                .open_backend(
                    options.renderer,
                    options.width,
                    options.height,
                    options.gamma,
                    options.render_worker,
                )
                .map_err(|error| RendererError::Backend(error.to_string()))
        })();
        let backend = match opened {
            Ok(opened) => opened,
            Err(error) => {
                let mut failures = vec![error.to_string()];
                if let Err(cleanup) = window.close_window() {
                    failures.push(cleanup.to_string());
                }
                if failures.len() > 1 {
                    return Err(RendererError::OpenCleanup { errors: failures });
                }
                return Err(error);
            }
        };
        Ok(Self {
            window,
            owner,
            backend,
            gamma: options.gamma,
            images,
            journal,
            captures,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            closed: false,
            preparing: false,
            pending_restart: None,
            retirements: Vec::new(),
            interval: 1,
        })
    }

    /// Current window.
    #[must_use]
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Current backend.
    #[must_use]
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    /// Resource owner.
    #[must_use]
    pub fn owner(&self) -> &RendererResourceOwner {
        &self.owner
    }

    /// Image registry.
    #[must_use]
    pub fn images(&self) -> &Images {
        &self.images
    }

    /// Whether the backend executes on a worker thread.
    #[must_use]
    pub fn render_worker(&self) -> bool {
        self.backend.is_worker()
    }

    /// GL driver description, if any.
    #[must_use]
    pub fn driver(&self) -> Option<RenderDriverInfo> {
        self.backend.driver()
    }

    /// GL configuration, if any.
    #[must_use]
    pub fn gl_config(&self) -> Option<RenderGlConfig> {
        self.backend.gl_config()
    }

    /// Swap interval.
    pub fn swap_interval(&self) -> Result<i32, RendererError> {
        if self.backend.is_worker() {
            return Ok(self.backend.worker_swap_interval());
        }
        if self.window.backend_kind() != RenderBackendKind::Gl {
            return Ok(0);
        }
        let interval = self.window.swap_interval();
        if interval != -1 && interval != 0 && interval != 1 {
            return Err(RendererError::BadSwapInterval);
        }
        Ok(interval)
    }

    /// Set the swap interval.
    pub fn set_swap_interval(&mut self, interval: i32) -> Result<(), RendererError> {
        self.writable()?;
        if self.backend.is_worker() {
            self.backend.set_worker_swap_interval(interval);
        } else if self.window.backend_kind() == RenderBackendKind::Gl {
            self.window
                .set_swap_interval(interval)
                .map_err(|error| RendererError::Window(error.to_string()))?;
        }
        self.interval = interval;
        Ok(())
    }

    /// Run a window mutation with parked worker contexts.
    pub fn mutate_window(&mut self, operation: impl FnOnce()) -> Result<(), RendererError> {
        self.writable()?;
        operation();
        Ok(())
    }

    /// Output gamma.
    #[must_use]
    pub fn output_gamma(&self) -> f64 {
        self.gamma
    }

    /// Clear color.
    #[must_use]
    pub fn color(&self) -> Vec4 {
        self.color
    }

    /// Synchronize worker execution.
    pub fn synchronize(&mut self) {
        if self.backend.is_worker() {
            self.backend.synchronize();
        }
    }

    /// Whether the renderer is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Whether a restart is prepared or published.
    #[must_use]
    pub fn has_pending_restart(&self) -> bool {
        self.pending_restart.is_some()
    }

    /// Prepared restart window, if any.
    #[must_use]
    pub fn prepared_window(&self) -> Option<&Window> {
        self.pending_restart.as_ref().map(|restart| &restart.window)
    }

    /// Retirement count.
    #[must_use]
    pub fn retirement_count(&self) -> usize {
        self.retirements.len()
    }

    fn writable(&self) -> Result<(), RendererError> {
        if self.closed {
            return Err(RendererError::Closed);
        }
        if self.preparing || self.pending_restart.is_some() {
            return Err(RendererError::RestartPrepared);
        }
        Ok(())
    }

    fn apply_image(&mut self, operation: &ImageResourceOperation) {
        if self.backend.is_worker() {
            self.backend.apply_image_resource(operation);
        } else {
            let _ = self.journal.record(operation);
            self.backend.apply_image_resource(operation);
        }
    }

    fn restore_context(&mut self) {
        if !self.closed && !self.backend.is_worker() && self.window.backend_kind() == RenderBackendKind::Gl {
            let _ = self.window.make_current();
        }
    }

    /// Prepare a restart onto a fresh window and backend.
    pub fn prepare_restart<WindowFactory, BackendFactory>(
        &mut self,
        kind: RenderBackendKind,
        worker: bool,
        windows: &mut WindowFactory,
        backends: &mut BackendFactory,
    ) -> Result<(), RendererError>
    where
        WindowFactory: NativeWindowFactory<Window = Window>,
        BackendFactory: NativeBackendFactory<Backend = Backend>,
    {
        self.writable()?;
        self.synchronize();
        for operation in self.images.drain_pending_operations() {
            self.apply_image(&operation);
        }
        let presentation = self.window.capture_presentation();
        let dimensions = self.window.drawable_size();
        if self.window.backend_kind() == RenderBackendKind::Gl {
            self.interval = self.swap_interval()?;
        }
        self.preparing = true;
        let mut journal = RenderImageJournal::default();
        let prepared: Result<(Window, Backend, RenderImageJournal), RendererError> = (|| {
            let mut window = windows
                .open_window(&RenderWindowOptions {
                    backend: kind,
                    width: presentation.width,
                    height: presentation.height,
                    hidden: true,
                    resizable: (self.window.flags() & 0x20) != 0,
                    display_index: presentation.display_index,
                    position: presentation.position,
                })
                .map_err(|error| RendererError::Window(error.to_string()))?;
            if kind == RenderBackendKind::Gl {
                window
                    .set_swap_interval(self.interval)
                    .map_err(|error| RendererError::Window(error.to_string()))?;
            }
            let mut backend = backends
                .open_backend(kind, presentation.width, presentation.height, self.gamma, worker)
                .map_err(|error| RendererError::Backend(error.to_string()))?;
            let mut replay_error: Option<String> = None;
            self.journal
                .replay(&mut |operation| backend.apply_image_resource(operation));
            if !backend.is_worker() {
                self.journal.replay(&mut |operation| {
                    if replay_error.is_none() {
                        if let Err(error) = journal.record(operation) {
                            replay_error = Some(error.to_string());
                        }
                    }
                });
            } else {
                backend.execute_worker(&[RenderCommand::SetColor { color: self.color }], &[], Vec::new());
                backend.synchronize();
            }
            let failure = if let Some(error) = replay_error {
                Some(RendererError::Backend(error))
            } else if window.width() != dimensions.0 || window.height() != dimensions.1 {
                Some(RendererError::DimensionsChanged)
            } else if self.closed {
                Some(RendererError::ClosedDuringRestart)
            } else {
                None
            };
            if let Some(error) = failure {
                let mut failures = vec![error.to_string()];
                if let Err(cleanup) = backend.close_backend() {
                    failures.push(cleanup.to_string());
                }
                if let Err(cleanup) = window.close_window() {
                    failures.push(cleanup.to_string());
                }
                if failures.len() > 1 {
                    return Err(RendererError::RestartCleanup { errors: failures });
                }
                return Err(error);
            }
            Ok((window, backend, journal))
        })();
        match prepared {
            Ok((window, backend, journal)) => {
                self.restore_context();
                self.preparing = false;
                self.pending_restart = Some(PreparedRestart {
                    window,
                    backend,
                    journal,
                    phase: RestartPhase::Prepared,
                });
                Ok(())
            }
            Err(error) => {
                self.preparing = false;
                self.restore_context();
                Err(error)
            }
        }
    }

    /// Publish the prepared restart, returning its retirement id.
    pub fn publish_prepared_restart(&mut self) -> Result<usize, RendererError> {
        let prepared = matches!(
            self.pending_restart.as_ref(),
            Some(restart) if restart.phase == RestartPhase::Prepared
        );
        if !prepared || self.closed {
            return Err(RendererError::StaleRestart);
        }
        let presentation = self.window.capture_presentation();
        let (kind, is_worker) = {
            let restart = self.pending_restart.as_ref().expect("checked above");
            (restart.window.backend_kind(), restart.backend.is_worker())
        };
        let published = (|| {
            let restart = self.pending_restart.as_mut().expect("checked above");
            restart
                .window
                .restore_presentation(&presentation)
                .map_err(|error| RendererError::Window(error.to_string()))?;
            if kind == RenderBackendKind::Gl && !is_worker {
                restart
                    .window
                    .make_current()
                    .map_err(|error| RendererError::Window(error.to_string()))?;
            }
            Ok::<(), RendererError>(())
        })()
        .and_then(|()| {
            self.window
                .set_visible(false)
                .map_err(|error| RendererError::Window(error.to_string()))
        });
        if let Err(error) = published {
            let mut failures = vec![error.to_string()];
            if let Some(restart) = self.pending_restart.as_mut() {
                if let Err(failure) = restart.window.set_visible(false) {
                    failures.push(failure.to_string());
                }
            }
            let presentation = self.window.capture_presentation();
            if let Err(failure) = self.window.restore_presentation(&presentation) {
                failures.push(failure.to_string());
            }
            self.restore_context();
            if failures.len() > 1 {
                return Err(RendererError::PublishCleanup { errors: failures });
            }
            return Err(error);
        }
        let mut restart = self.pending_restart.take().expect("checked above");
        restart.phase = RestartPhase::Published;
        let retired_window = std::mem::replace(&mut self.window, restart.window);
        let retired_backend = std::mem::replace(&mut self.backend, restart.backend);
        self.journal = restart.journal;
        let color = self.color;
        self.backend.execute_serial_command(&RenderCommand::SetColor { color });
        self.retirements.push(RetiredResources {
            window: Some(retired_window),
            backend: Some(retired_backend),
            retired: false,
        });
        Ok(self.retirements.len() - 1)
    }

    /// Discard the prepared restart.
    pub fn discard_prepared_restart(&mut self) -> Result<(), RendererError> {
        let Some(mut restart) = self.pending_restart.take() else {
            return Ok(());
        };
        if restart.phase != RestartPhase::Prepared {
            self.pending_restart = Some(restart);
            return Ok(());
        }
        restart.phase = RestartPhase::Discarded;
        self.dispose(&mut restart.window, &mut restart.backend)
    }

    /// Retire published resources exactly once.
    pub fn retire_resources(&mut self, id: usize) -> Result<(), RendererError> {
        let pair = {
            let Some(retired) = self.retirements.get_mut(id) else {
                return Ok(());
            };
            if retired.retired {
                return Ok(());
            }
            retired.retired = true;
            (
                retired.window.take().expect("retired once"),
                retired.backend.take().expect("retired once"),
            )
        };
        let (mut window, mut backend) = pair;
        self.dispose(&mut window, &mut backend)
    }

    fn dispose<DisposeWindow: NativeRenderWindow, DisposeBackend: NativeRenderBackend>(
        &mut self,
        window: &mut DisposeWindow,
        backend: &mut DisposeBackend,
    ) -> Result<(), RendererError> {
        let mut failures = Vec::new();
        if window.backend_kind() == RenderBackendKind::Gl && !backend.is_worker() {
            if let Err(error) = window.make_current() {
                failures.push(error.to_string());
            }
        }
        if let Err(error) = backend.close_backend() {
            failures.push(error.to_string());
        }
        if let Err(error) = window.close_window() {
            failures.push(error.to_string());
        }
        self.restore_context();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RendererError::RetireCleanup { errors: failures })
        }
    }

    /// Renderer diagnostics.
    pub fn diagnostics(&mut self) -> Result<RendererDiagnostics, RendererError> {
        if self.closed {
            return Err(RendererError::Closed);
        }
        self.synchronize();
        Ok(RendererDiagnostics {
            backend: self.window.backend_kind(),
            width: self.backend.width(),
            height: self.backend.height(),
            driver: self.backend.driver(),
            display_modes: self.window.display_modes(),
            images: self.journal.describe(),
        })
    }

    /// Set the output gamma.
    pub fn set_output_gamma(&mut self, gamma: f64) -> Result<(), RendererError> {
        self.writable()?;
        self.backend
            .set_output_gamma(gamma)
            .map_err(|error| RendererError::Backend(error.to_string()))?;
        self.gamma = gamma;
        Ok(())
    }

    fn resize<BackendFactory>(&mut self, backends: &mut BackendFactory) -> Result<(), RendererError>
    where
        BackendFactory: NativeBackendFactory<Backend = Backend>,
    {
        if !self.backend.is_worker() && !self.backend.is_software() {
            return Ok(());
        }
        let (width, height) = self.window.drawable_size();
        if self.backend.is_worker() {
            self.backend.resize_backend(width, height);
            return Ok(());
        }
        if width == self.backend.width() && height == self.backend.height() {
            return Ok(());
        }
        let mut replacement = backends
            .open_backend(RenderBackendKind::Cpu, width, height, self.gamma, false)
            .map_err(|error| RendererError::Backend(error.to_string()))?;
        replacement
            .set_output_gamma(self.gamma)
            .map_err(|error| RendererError::Backend(error.to_string()))?;
        self.journal
            .replay(&mut |operation| replacement.apply_image_resource(operation));
        let mut old = std::mem::replace(&mut self.backend, replacement);
        old.close_backend()
            .map_err(|error| RendererError::Backend(error.to_string()))?;
        Ok(())
    }
}

impl<Window, Backend, Images> NativeRenderer<Window, Backend, Images>
where
    Window: NativeRenderWindow,
    Backend: NativeRenderBackend,
    Images: RenderImageRegistry,
{
    /// Execute one frame.
    pub fn execute<BackendFactory>(
        &mut self,
        frame: &RenderFrame,
        backends: &mut BackendFactory,
    ) -> Result<(), RendererError>
    where
        BackendFactory: NativeBackendFactory<Backend = Backend>,
    {
        self.writable()?;
        if frame.owner != self.owner {
            return Err(RendererError::ForeignFrame);
        }
        self.resize(backends)?;
        if self.backend.is_worker() {
            let captures = if frame
                .commands
                .iter()
                .any(|command| matches!(command, RenderCommand::SwapBuffers))
            {
                self.captures.take()
            } else {
                Vec::new()
            };
            let operations = self.images.drain_pending_operations();
            self.backend.execute_worker(&frame.commands, &captures, operations);
            for command in &frame.commands {
                if let RenderCommand::SetColor { color } = command {
                    self.color = *color;
                }
            }
        } else {
            let mut index = 0;
            while index < frame.commands.len() {
                match &frame.commands[index] {
                    RenderCommand::SetColor { color } => {
                        self.color = *color;
                        self.backend
                            .execute_serial_command(&RenderCommand::SetColor { color: *color });
                    }
                    RenderCommand::SwapBuffers => {
                        let captures = self.captures.take();
                        self.swap(&captures)?;
                    }
                    RenderCommand::Draw => {
                        self.backend.execute_serial_command(&RenderCommand::Draw);
                    }
                    RenderCommand::DrawBuffer { buffer, clear } => {
                        self.backend.execute_serial_command(&RenderCommand::DrawBuffer {
                            buffer: *buffer,
                            clear: *clear,
                        });
                    }
                    RenderCommand::View(view) => {
                        self.backend.execute_serial_command(&RenderCommand::View(view.clone()));
                    }
                }
                index += 1;
            }
            for operation in self.images.drain_pending_operations() {
                self.apply_image(&operation);
            }
        }
        Ok(())
    }

    fn swap(&mut self, captures: &[CaptureId]) -> Result<(), RendererError> {
        if self.backend.is_worker() {
            return Err(RendererError::WorkerSwap);
        }
        if self.backend.is_software() {
            self.backend.finish_software();
        }
        if !captures.is_empty() {
            let image = if self.backend.is_software() {
                ImageLevel {
                    width: self.backend.width() as u32,
                    height: self.backend.height() as u32,
                    pixels: self.backend.software_pixels(),
                }
            } else {
                self.backend.read_pixels_backend()
            };
            self.captures.complete(Some(&image), captures)?;
        }
        if self.backend.is_software() {
            let pixels = self.backend.software_pixels();
            self.window
                .present_pixels(&pixels)
                .map_err(|error| RendererError::Window(error.to_string()))?;
        } else {
            self.backend.present_backend();
        }
        Ok(())
    }

    /// Read back pixels (CPU windows only).
    pub fn read_pixels(&mut self) -> Result<Vec<u8>, RendererError> {
        if self.window.backend_kind() != RenderBackendKind::Cpu {
            return Err(RendererError::GlCaptureOrder);
        }
        if self.backend.is_worker() {
            return Ok(self.backend.read_pixels_backend().pixels);
        }
        if !self.backend.is_software() {
            return Err(RendererError::NoCpuBackend);
        }
        self.backend.finish_software();
        Ok(self.backend.software_pixels())
    }

    /// Capture the next presented frame.
    pub fn capture_next_frame(&mut self, handler: CaptureHandler) -> Result<CaptureId, RendererError> {
        if self.closed {
            return Err(RendererError::Closed);
        }
        self.captures.request(handler)
    }

    /// Cancel a pending capture.
    pub fn cancel_capture(&mut self, id: CaptureId) -> bool {
        self.captures.cancel(id)
    }

    /// Close the renderer, aggregating cleanup failures.
    pub fn close(&mut self) -> Result<(), RendererError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut failures = Vec::new();
        if let Err(error) = self.discard_prepared_restart() {
            failures.push(error.to_string());
        }
        self.captures.close();
        if self.window.backend_kind() == RenderBackendKind::Gl && !self.backend.is_worker() {
            if let Err(error) = self.window.make_current() {
                failures.push(error.to_string());
            }
        }
        if let Err(error) = self.backend.close_backend() {
            failures.push(error.to_string());
        }
        let closed = self.images.close_images().map_err(|error| error.to_string());
        if closed.is_ok() {
            self.images.drain_pending_operations();
        }
        if let Err(error) = closed {
            failures.push(error);
        }
        self.journal.clear();
        if let Err(error) = self.window.close_window() {
            failures.push(error.to_string());
        }
        for index in 0..self.retirements.len() {
            if let Err(error) = self.retire_resources(index) {
                failures.push(error.to_string());
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(RendererError::CloseCleanup { errors: failures })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct WindowError;

    impl std::fmt::Display for WindowError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "window failed")
        }
    }

    struct FakeWindow {
        kind: RenderBackendKind,
        width: i32,
        height: i32,
        swap: i32,
        presented: Vec<u8>,
        closed: bool,
    }

    impl NativeRenderWindow for FakeWindow {
        type Error = WindowError;

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
            0x20
        }

        fn display_modes(&self) -> Vec<SdlDisplayMode> {
            Vec::new()
        }

        fn swap_interval(&self) -> i32 {
            self.swap
        }

        fn set_swap_interval(&mut self, interval: i32) -> Result<(), WindowError> {
            self.swap = interval;
            Ok(())
        }

        fn set_visible(&mut self, _visible: bool) -> Result<(), WindowError> {
            Ok(())
        }

        fn make_current(&mut self) -> Result<(), WindowError> {
            Ok(())
        }

        fn present_pixels(&mut self, pixels: &[u8]) -> Result<(), WindowError> {
            self.presented = pixels.to_vec();
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

        fn restore_presentation(&mut self, presentation: &RenderWindowPresentation) -> Result<(), WindowError> {
            self.width = presentation.width;
            self.height = presentation.height;
            Ok(())
        }

        fn close_window(&mut self) -> Result<(), WindowError> {
            self.closed = true;
            Ok(())
        }
    }

    struct FakeBackend {
        width: i32,
        height: i32,
        worker: bool,
        software: bool,
        gamma: f64,
        pixels: Vec<u8>,
        executed: Vec<RenderCommand>,
        closed: bool,
    }

    impl NativeRenderBackend for FakeBackend {
        type Error = WindowError;

        fn width(&self) -> i32 {
            self.width
        }

        fn height(&self) -> i32 {
            self.height
        }

        fn is_worker(&self) -> bool {
            self.worker
        }

        fn is_software(&self) -> bool {
            self.software
        }

        fn set_output_gamma(&mut self, gamma: f64) -> Result<(), WindowError> {
            self.gamma = gamma;
            Ok(())
        }

        fn apply_image_resource(&mut self, _operation: &ImageResourceOperation) {}

        fn execute_serial_command(&mut self, command: &RenderCommand) {
            self.executed.push(command.clone());
        }

        fn execute_worker(
            &mut self,
            commands: &[RenderCommand],
            _captures: &[CaptureId],
            _operations: Vec<ImageResourceOperation>,
        ) {
            self.executed.extend(commands.iter().cloned());
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
            self.pixels.clone()
        }

        fn read_pixels_backend(&mut self) -> ImageLevel {
            ImageLevel {
                width: 2,
                height: 2,
                pixels: vec![9, 9, 9, 9],
            }
        }

        fn present_backend(&mut self) {}

        fn driver(&self) -> Option<RenderDriverInfo> {
            None
        }

        fn gl_config(&self) -> Option<RenderGlConfig> {
            None
        }

        fn close_backend(&mut self) -> Result<(), WindowError> {
            self.closed = true;
            Ok(())
        }
    }

    struct FakeImages {
        owner: RendererResourceOwner,
    }

    impl RenderImageRegistry for FakeImages {
        type Error = WindowError;

        fn owner(&self) -> &RendererResourceOwner {
            &self.owner
        }

        fn drain_pending_operations(&mut self) -> Vec<ImageResourceOperation> {
            Vec::new()
        }

        fn close_images(&mut self) -> Result<(), WindowError> {
            Ok(())
        }
    }

    struct FakeWindows;

    struct FakeBackends {
        backend_kind: RenderBackendKind,
        worker: bool,
    }

    impl NativeWindowFactory for FakeWindows {
        type Window = FakeWindow;
        type Error = WindowError;

        fn open_window(&mut self, options: &RenderWindowOptions) -> Result<FakeWindow, WindowError> {
            Ok(FakeWindow {
                kind: options.backend,
                width: options.width,
                height: options.height,
                swap: 1,
                presented: Vec::new(),
                closed: false,
            })
        }
    }

    impl NativeBackendFactory for FakeBackends {
        type Backend = FakeBackend;
        type Error = WindowError;

        fn open_backend(
            &mut self,
            kind: RenderBackendKind,
            width: i32,
            height: i32,
            gamma: f64,
            worker: bool,
        ) -> Result<FakeBackend, WindowError> {
            self.backend_kind = kind;
            self.worker = worker;
            Ok(FakeBackend {
                width,
                height,
                worker,
                software: kind == RenderBackendKind::Cpu,
                gamma,
                pixels: vec![1, 2, 3, 4],
                executed: Vec::new(),
                closed: false,
            })
        }
    }

    fn owner() -> RendererResourceOwner {
        RendererResourceOwner {
            identity: "test".to_string(),
            session: 1,
            generation: 1,
        }
    }

    fn options() -> NativeRendererOptions {
        NativeRendererOptions {
            renderer: RenderBackendKind::Cpu,
            width: 640,
            height: 480,
            hidden: false,
            gamma: 1.0,
            render_worker: false,
        }
    }

    fn renderer() -> (
        NativeRenderer<FakeWindow, FakeBackend, FakeImages>,
        FakeWindows,
        FakeBackends,
    ) {
        let mut windows = FakeWindows;
        let mut backends = FakeBackends {
            backend_kind: RenderBackendKind::Cpu,
            worker: false,
        };
        let renderer = NativeRenderer::open(
            &options(),
            owner(),
            FakeImages { owner: owner() },
            &mut windows,
            &mut backends,
        )
        .unwrap();
        (renderer, windows, backends)
    }

    #[test]
    fn opens_rejects_foreign_registries() {
        let mut windows = FakeWindows;
        let mut backends = FakeBackends {
            backend_kind: RenderBackendKind::Cpu,
            worker: false,
        };
        let foreign = RendererResourceOwner {
            identity: "other".to_string(),
            session: 1,
            generation: 1,
        };
        assert_eq!(
            NativeRenderer::open(
                &options(),
                owner(),
                FakeImages { owner: foreign },
                &mut windows,
                &mut backends
            )
            .err()
            .unwrap(),
            RendererError::ForeignRegistry
        );
    }

    #[test]
    fn executes_frames_and_completes_captures() {
        let (mut renderer, _, mut backends) = renderer();
        let captured: Rc<RefCell<Vec<ImageLevel>>> = Rc::new(RefCell::new(Vec::new()));
        let stored = Rc::clone(&captured);
        let id = renderer
            .capture_next_frame(Box::new(move |result| {
                stored.borrow_mut().push(result.unwrap());
            }))
            .unwrap();
        assert_eq!(id, 1);
        renderer
            .execute(
                &RenderFrame {
                    owner: owner(),
                    commands: vec![RenderCommand::SwapBuffers, RenderCommand::Draw],
                },
                &mut backends,
            )
            .unwrap();
        assert_eq!(captured.borrow().len(), 1);
        assert_eq!(captured.borrow()[0].pixels, vec![1, 2, 3, 4]);
        assert_eq!(renderer.read_pixels().unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(
            renderer
                .execute(
                    &RenderFrame {
                        owner: RendererResourceOwner {
                            identity: "x".to_string(),
                            session: 1,
                            generation: 1
                        },
                        commands: Vec::new(),
                    },
                    &mut backends,
                )
                .err()
                .unwrap(),
            RendererError::ForeignFrame
        );
    }

    #[test]
    fn captures_cancel_and_fail_on_close() {
        let (mut renderer, _, _) = renderer();
        let aborted: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let stored = Rc::clone(&aborted);
        let id = renderer
            .capture_next_frame(Box::new(move |result| {
                stored.borrow_mut().push(result.unwrap_err());
            }))
            .unwrap();
        assert!(renderer.cancel_capture(id));
        assert!(!renderer.cancel_capture(id));
        assert_eq!(*aborted.borrow(), vec![RendererError::CaptureAborted.to_string()]);
        renderer.close().unwrap();
        assert!(renderer.is_closed());
        assert_eq!(
            renderer.capture_next_frame(Box::new(|_| {})).err().unwrap(),
            RendererError::Closed
        );
        renderer.close().unwrap();
    }

    #[test]
    fn diagnostics_and_gamma_round_trip() {
        let (mut renderer, _, _) = renderer();
        let diagnostics = renderer.diagnostics().unwrap();
        assert_eq!(diagnostics.backend, RenderBackendKind::Cpu);
        assert_eq!((diagnostics.width, diagnostics.height), (640, 480));
        renderer.set_output_gamma(2.2).unwrap();
        assert_eq!(renderer.output_gamma(), 2.2);
        renderer.set_swap_interval(0).unwrap();
        renderer.mutate_window(|| {}).unwrap();
    }

    #[test]
    fn restart_publish_discard_cycle() {
        let (mut renderer, mut windows, mut backends) = renderer();
        renderer
            .prepare_restart(RenderBackendKind::Cpu, false, &mut windows, &mut backends)
            .unwrap();
        assert!(renderer.has_pending_restart());
        assert_eq!(
            renderer.set_output_gamma(1.0).err().unwrap(),
            RendererError::RestartPrepared
        );
        let id = renderer.publish_prepared_restart().unwrap();
        assert!(!renderer.has_pending_restart());
        renderer.retire_resources(id).unwrap();
        renderer.retire_resources(id).unwrap();
        renderer
            .prepare_restart(RenderBackendKind::Cpu, false, &mut windows, &mut backends)
            .unwrap();
        renderer.discard_prepared_restart().unwrap();
        assert!(!renderer.has_pending_restart());
        renderer.discard_prepared_restart().unwrap();
        assert_eq!(
            renderer.publish_prepared_restart().err().unwrap(),
            RendererError::StaleRestart
        );
    }
}
