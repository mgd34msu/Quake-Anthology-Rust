//! Windowed game path: native SDL window, GL or CPU renderer, and startup driver.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts`
//! (`StartupApplication.open` graphics tail),
//! `src/app/bootstrap/renderer.ts` (`NativeRenderer.open`, `openBackend`),
//! and `src/render/commands/frame.ts` (`SceneFrameBuilder`) plus the client
//! view submission (`RenderView` with scene entities and camera state).
//! The window factory opens an [`SdlWindow`] with GL options (the
//! `SDL_GL_CreateContext` path); the backend factory adopts that context
//! through the SDL render-context transfer protocol and builds a
//! [`GlRenderer`] over [`PlatformGlContext`]; both compose through
//! [`NativeRenderer::open`]; [`WindowedStartupBackend`] drives
//! [`StartupApplication::open`] for `--windowed` runs, submitting one
//! draw-buffer plus scene view plus swap frame per tick from its live world
//! state and degrading to clear plus swap when no scene is loaded.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use qa_client::audio::engine::{SdlDeviceFactory, UnifiedAudio, UnifiedAudioOptions};
use qa_client::audio::types::AudioAudience;
use qa_client::input::router::{InputRouter, RouterError, RouterWindow, Seat, SeatInputEvent, SeatRoute, UiCallback};
use qa_client::input::{FrameContext as ClientFrameContext, InputCommandBuilder};
use qa_client::render::dynamic_texture::resolve_draw_textures;
use qa_client::render::gl::platform::PlatformGlContext;
use qa_client::render::gl::renderer::GlRenderer;
use qa_client::render::gl::{GlContext, DEPTH_BITS, MAX_TEXTURE_COORDS, MAX_TEXTURE_IMAGE_UNITS, MAX_TEXTURE_SIZE};
use qa_client::render::scene::resources::SceneImageRegistry;
use qa_client::render::stage_timings::{StageSample, StageTimer, StageTotals};
use qa_client::render::types::{
    DrawBatch, DrawBuffer, ImageResourceOperation, OrderedBackend, Rect as ClientRect, RenderOperation,
    RenderView as ClientRenderView, RenderViewState, ResourceOwner, SourceTime, ViewClear, ViewTarget,
};
use qa_client::view::{perspective_projection, CameraClip, Rect as ViewRect, SceneCamera};
use qa_content::catalog::{discover_installed_content, DiscoverContentOptions};
use qa_core::cmd::Dialect;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{IdentityOwner, SeatId};
use qa_core::math::{angles_to_axis, vec3, vec4, Vec3, Vec4};
use qa_platform::controller::ControllerEvent;
use qa_platform::controller::ControllerSelection;
use qa_platform::controller::SdlControllers;
use qa_platform::native_libraries::NativeLibraryOptions;
use qa_platform::sdl::{
    SdlBackend, SdlDisplayMode, SdlEvent, SdlGlOptions, SdlInputLease, SdlWindow, SdlWindowOptions,
    SdlWindowPresentation,
};
use qa_platform::sdl_render_context::SdlWorkerRenderContext;
use qa_world::session::SessionSeat;

use super::audio_bridge::{AudioBridge, MountsSoundContent};
use super::frame_time::{
    nq_frame_due, qw_fps, qw_frame_due, read_frame_time_controls, register_frame_time_cvars, source_frame_milliseconds,
    FrameTimeControls, FrameTimeHost,
};
use super::input::{dialect_family, seat_sample, user_command, LocalPlayer, NullRegistry};
use super::play_world::{load_play_world, PlayWorld};
use super::renderer::{
    CaptureId, ImageLevel, NativeBackendFactory, NativeRenderBackend, NativeRenderWindow, NativeRenderer,
    NativeRendererOptions, NativeWindowFactory, RenderBackendKind, RenderCommand, RenderDriverInfo, RenderFrame,
    RenderGlConfig, RenderImageRegistry, RenderWindowOptions, RenderWindowPresentation, RendererResourceOwner,
};
use super::simulation::players::net_to_world;
use super::startup::{StartupAction, StartupApplication, StartupBackend, StartupEntry, StartupError, StartupFrame};
use super::startup_selection::StartupSelectionModel;
use super::windowed_cpu::NativeCpuBackend;
use super::windowed_menu::WindowedMenu;
use super::windowed_menu_launch::{launch_options, pump_windowed_controllers, MenuLaunchQueue};
use super::windowed_pacer::WindowedPacer;
use super::windowed_preset::WindowedPresetCollaborators;
use super::windowed_scene::PlayPresentation;
use crate::options::{ApplicationOptions, Renderer};
use crate::startup::StartupConfig;

/// SDL window-event id for close (donor `event.event === 14` quit check).
const SDL_WINDOWEVENT_CLOSE: u8 = 14;

/// Native renderer over the windowed adapters.
type WindowedRenderer = NativeRenderer<NativeSdlWindow, NativeWindowedBackend, NativeImages>;

/// State shared between the windowed factories and adapters.
#[derive(Default)]
struct WindowedShare {
    /// Open window, shared so the backend factory can adopt GL.
    window: Option<Rc<RefCell<SdlWindow>>>,
    /// Whether the GL context was adopted away from the window.
    detached: bool,
}

/// Best-effort window restoration after a failed backend open.
fn restore_window(share: &Rc<RefCell<WindowedShare>>) {
    if let Some(window) = share.borrow().window.as_ref() {
        let _ignored = window.borrow_mut().restore_render_context();
    }
}

/// [`NativeRenderWindow`] over a shared [`SdlWindow`].
struct NativeSdlWindow {
    window: Rc<RefCell<SdlWindow>>,
    share: Rc<RefCell<WindowedShare>>,
    kind: RenderBackendKind,
    presentation: SdlWindowPresentation,
}

impl NativeSdlWindow {
    /// Pump SDL events through the shared window.
    fn poll_events(&self) -> Result<Vec<SdlEvent>, String> {
        self.window
            .borrow_mut()
            .poll_events()
            .map_err(|error| error.to_string())
    }

    /// Whether the GL context was adopted away from the window.
    fn detached(&self) -> bool {
        self.share.borrow().detached
    }
}

impl NativeRenderWindow for NativeSdlWindow {
    type Error = String;

    fn backend_kind(&self) -> RenderBackendKind {
        self.kind
    }

    fn width(&self) -> i32 {
        self.window.borrow().logical_size().map(|size| size.0).unwrap_or(0)
    }

    fn height(&self) -> i32 {
        self.window.borrow().logical_size().map(|size| size.1).unwrap_or(0)
    }

    fn drawable_size(&self) -> (i32, i32) {
        self.window.borrow().drawable_size_signed().unwrap_or((0, 0))
    }

    fn flags(&self) -> u32 {
        self.window.borrow().flags().unwrap_or(0)
    }

    fn display_modes(&self) -> Vec<SdlDisplayMode> {
        self.window.borrow().display_modes().unwrap_or_default()
    }

    fn swap_interval(&self) -> i32 {
        self.window.borrow_mut().swap_interval().unwrap_or(0)
    }

    fn set_swap_interval(&mut self, interval: i32) -> Result<(), String> {
        self.window
            .borrow_mut()
            .set_swap_interval(interval)
            .map_err(|error| error.to_string())
    }

    fn set_visible(&mut self, visible: bool) -> Result<(), String> {
        self.window
            .borrow()
            .set_visible(visible)
            .map_err(|error| error.to_string())
    }

    fn make_current(&mut self) -> Result<(), String> {
        if self.detached() {
            // The adopted worker context selects itself before every GL call,
            // so the window-level make-current is redundant after detach.
            return Ok(());
        }
        self.window
            .borrow_mut()
            .make_current()
            .map_err(|error| error.to_string())
    }

    fn present_pixels(&mut self, pixels: &[u8]) -> Result<(), String> {
        if self.kind != RenderBackendKind::Cpu {
            return Err("present requires a CPU window".to_string());
        }
        self.window
            .borrow_mut()
            .present(pixels)
            .map_err(|error| error.to_string())
    }

    fn capture_presentation(&self) -> RenderWindowPresentation {
        let live = self.window.borrow().capture_presentation().ok();
        RenderWindowPresentation {
            width: live.as_ref().map_or(self.presentation.size.0, |state| state.size.0),
            height: live.as_ref().map_or(self.presentation.size.1, |state| state.size.1),
            display_index: live
                .as_ref()
                .map_or(self.presentation.display_index, |state| state.display_index),
            position: live.as_ref().map_or(self.presentation.position, |state| state.position),
        }
    }

    fn restore_presentation(&mut self, presentation: &RenderWindowPresentation) -> Result<(), String> {
        self.presentation.size = (presentation.width, presentation.height);
        self.presentation.position = presentation.position;
        self.presentation.display_index = presentation.display_index;
        self.window
            .borrow_mut()
            .restore_presentation(&self.presentation)
            .map_err(|error| error.to_string())
    }

    fn close_window(&mut self) -> Result<(), String> {
        self.window.borrow_mut().close().map_err(|error| error.to_string())
    }
}

/// [`NativeWindowFactory`] opening a GL [`SdlWindow`] (donor
/// `SdlWindow.open` in `NativeRenderer.open`).
struct NativeSdlWindowFactory {
    share: Rc<RefCell<WindowedShare>>,
}

impl NativeWindowFactory for NativeSdlWindowFactory {
    type Window = NativeSdlWindow;
    type Error = String;

    fn open_window(&mut self, options: &RenderWindowOptions) -> Result<NativeSdlWindow, String> {
        let backend = match options.backend {
            RenderBackendKind::Gl => SdlBackend::Gl(SdlGlOptions {
                stereo: false,
                stencil_bits: 0,
                color_bits: 0.0,
                depth_bits: 0.0,
                driver: None,
                allow_software_gl: true,
            }),
            RenderBackendKind::Cpu => SdlBackend::Cpu,
        };
        let window_options = SdlWindowOptions {
            title: "Quake".to_string(),
            width: options.width,
            height: options.height,
            hidden: options.hidden,
            resizable: true,
            fullscreen: false,
            display_refresh: 0,
            display_index: options.display_index,
            min_display_refresh: 0,
            max_display_refresh: 0,
            position: None,
            backend,
        };
        let window = SdlWindow::open(&window_options).map_err(|error| error.to_string())?;
        let presentation = window.capture_presentation().map_err(|error| error.to_string())?;
        let shared = Rc::new(RefCell::new(window));
        self.share.borrow_mut().window = Some(Rc::clone(&shared));
        Ok(NativeSdlWindow {
            window: shared,
            share: Rc::clone(&self.share),
            kind: options.backend,
            presentation,
        })
    }
}

/// Owned GL parts: the renderer plus the adopted context behind a raw pointer.
struct GlParts {
    renderer: GlRenderer<PlatformGlContext<'static>>,
    /// Adopted worker context backing the renderer borrow; freed exactly
    /// once in [`NativeGlBackend::release_parts`] after the renderer drops.
    worker: *mut SdlWorkerRenderContext,
}

/// [`NativeRenderBackend`] over [`GlRenderer`].
struct NativeGlBackend {
    parts: Option<GlParts>,
    width: i32,
    height: i32,
    driver: Option<RenderDriverInfo>,
    worker_interval: i32,
    color: Vec4,
}

/// Execute one ordered operation against the live [`GlRenderer`], resolving
/// dynamic textures before drawing (donor `RenderExecutor` draw path).
fn execute_windowed_operation(renderer: &mut GlRenderer<PlatformGlContext<'static>>, operation: &RenderOperation) {
    match operation {
        RenderOperation::Draw(batches) => {
            for batch in batches {
                let mut pending = Vec::new();
                let resolved = resolve_draw_textures(batch, &mut |operation| pending.push(operation));
                for operation in pending {
                    renderer.apply_image_resource(&operation);
                }
                renderer.draw_batch(&resolved);
            }
        }
        RenderOperation::ObjectOpacity { opacity, batches } => {
            if *opacity <= 0.0 {
                return;
            }
            if *opacity >= 1.0 {
                execute_windowed_operation(renderer, &RenderOperation::Draw(batches.clone()));
                return;
            }
            let mut pending = Vec::new();
            let mut resolved_batches = Vec::with_capacity(batches.len());
            for batch in batches {
                let mut operations = Vec::new();
                let resolved = resolve_draw_textures(batch, &mut |operation| operations.push(operation));
                pending.extend(operations);
                resolved_batches.push(resolved);
            }
            for operation in pending {
                renderer.apply_image_resource(&operation);
            }
            renderer.with_object_opacity(*opacity, |renderer| {
                for batch in &resolved_batches {
                    renderer.draw_batch(batch);
                }
            });
        }
        other => renderer.draw_immediate(other),
    }
}

/// Execute one ordered view: before-view operations, the view state
/// (viewport, clear, clip), then the view operations (donor
/// `RenderExecutor` view path).
fn execute_windowed_view(renderer: &mut GlRenderer<PlatformGlContext<'static>>, view: &ClientRenderView) {
    for operation in &view.before_view {
        execute_windowed_operation(renderer, operation);
    }
    renderer.begin_view(&view.state);
    for operation in &view.operations {
        execute_windowed_operation(renderer, operation);
    }
}

impl NativeGlBackend {
    /// Reconstruct and drop an adopted worker box from its raw pointer.
    ///
    /// # Safety
    ///
    /// `raw` must come from `Box::into_raw`, be live, have no live borrows,
    /// and be freed exactly once.
    unsafe fn free_worker(raw: *mut SdlWorkerRenderContext) {
        drop(unsafe { Box::from_raw(raw) });
    }

    /// Drop GL parts in order: the renderer first (releasing procedure
    /// leases), then the adopted worker context (releasing to the window).
    fn release_parts(parts: GlParts) {
        let GlParts { renderer, worker } = parts;
        drop(renderer);
        // SAFETY: `worker` came from `Box::into_raw` in `open_backend`; the
        // renderer borrow is dead after the drop above, and parts are taken
        // exactly once (explicit close or the drop backstop, never both).
        unsafe {
            Self::free_worker(worker);
        }
    }
}

impl Drop for NativeGlBackend {
    fn drop(&mut self) {
        // Backstop for error paths that skip `close_backend`; the explicit
        // close takes parts first, so this only fires once. Only drops run
        // here (no GL calls), so a half-torn-down window cannot panic.
        if let Some(parts) = self.parts.take() {
            Self::release_parts(parts);
        }
    }
}

impl NativeRenderBackend for NativeGlBackend {
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
        false
    }

    fn set_output_gamma(&mut self, gamma: f64) -> Result<(), String> {
        if let Some(parts) = self.parts.as_mut() {
            parts.renderer.set_output_gamma(gamma as f32);
        }
        Ok(())
    }

    fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
        if let Some(parts) = self.parts.as_mut() {
            parts.renderer.apply_image_resource(operation);
        }
    }

    fn execute_serial_command(&mut self, command: &RenderCommand) {
        match command {
            RenderCommand::SetColor { color } => {
                self.color = *color;
            }
            RenderCommand::DrawBuffer { buffer, clear } => {
                // Painting at command time (rather than at presentation)
                // keeps armed captures truthful, because captures read
                // before the swap.
                if let Some(parts) = self.parts.as_mut() {
                    parts.renderer.select_draw_buffer(*buffer, *clear);
                }
            }
            RenderCommand::View(view) => {
                if let Some(parts) = self.parts.as_mut() {
                    execute_windowed_view(&mut parts.renderer, view);
                }
            }
            RenderCommand::Draw | RenderCommand::SwapBuffers => {}
        }
    }

    fn execute_worker(
        &mut self,
        _commands: &[RenderCommand],
        _captures: &[CaptureId],
        _operations: Vec<ImageResourceOperation>,
    ) {
    }

    fn synchronize(&mut self) {}

    fn set_worker_swap_interval(&mut self, interval: i32) {
        self.worker_interval = interval;
    }

    fn worker_swap_interval(&self) -> i32 {
        self.worker_interval
    }

    fn resize_backend(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
        if let Some(parts) = self.parts.as_mut() {
            parts
                .renderer
                .set_drawable_size(width.max(0) as u32, height.max(0) as u32);
        }
    }

    fn finish_software(&mut self) {}

    fn software_pixels(&self) -> Vec<u8> {
        Vec::new()
    }

    fn read_pixels_backend(&mut self) -> ImageLevel {
        let (width, height) = (self.width, self.height);
        match self.parts.as_mut() {
            Some(parts) => ImageLevel {
                width: width.max(0) as u32,
                height: height.max(0) as u32,
                pixels: parts.renderer.read_pixels(),
            },
            None => ImageLevel {
                width: 0,
                height: 0,
                pixels: Vec::new(),
            },
        }
    }

    fn present_backend(&mut self) {
        if let Some(parts) = self.parts.as_mut() {
            parts.renderer.present();
        }
    }

    fn driver(&self) -> Option<RenderDriverInfo> {
        self.driver.clone()
    }

    fn gl_config(&self) -> Option<RenderGlConfig> {
        // `GlRenderer` exposes no serial config getters.
        None
    }

    fn close_backend(&mut self) -> Result<(), String> {
        if let Some(mut parts) = self.parts.take() {
            parts.renderer.close();
            Self::release_parts(parts);
        }
        Ok(())
    }
}

/// Pre-check the [`GlRenderer::new`] requirements so weak GL reports an
/// error instead of panicking.
fn gl_capable(gl: &mut PlatformGlContext<'_>) -> bool {
    let mut one = [0i32; 1];
    gl.get_integerv(MAX_TEXTURE_COORDS, &mut one);
    let coords = one[0];
    gl.get_integerv(MAX_TEXTURE_IMAGE_UNITS, &mut one);
    let units = one[0];
    gl.get_integerv(MAX_TEXTURE_SIZE, &mut one);
    let max_texture = one[0];
    gl.get_integerv(DEPTH_BITS, &mut one);
    let depth = one[0];
    coords.min(units) >= 4 && max_texture >= 1 && depth >= 1
}

/// Windowed backend: GL or CPU composition behind one renderer type.
enum NativeWindowedBackend {
    Gl(Box<NativeGlBackend>),
    Cpu(Box<NativeCpuBackend>),
}

impl NativeRenderBackend for NativeWindowedBackend {
    type Error = String;

    fn width(&self) -> i32 {
        match self {
            Self::Gl(backend) => backend.width(),
            Self::Cpu(backend) => backend.width(),
        }
    }

    fn height(&self) -> i32 {
        match self {
            Self::Gl(backend) => backend.height(),
            Self::Cpu(backend) => backend.height(),
        }
    }

    fn is_worker(&self) -> bool {
        match self {
            Self::Gl(backend) => backend.is_worker(),
            Self::Cpu(backend) => backend.is_worker(),
        }
    }

    fn is_software(&self) -> bool {
        match self {
            Self::Gl(backend) => backend.is_software(),
            Self::Cpu(backend) => backend.is_software(),
        }
    }

    fn set_output_gamma(&mut self, gamma: f64) -> Result<(), String> {
        match self {
            Self::Gl(backend) => backend.set_output_gamma(gamma),
            Self::Cpu(backend) => backend.set_output_gamma(gamma),
        }
    }

    fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
        match self {
            Self::Gl(backend) => backend.apply_image_resource(operation),
            Self::Cpu(backend) => backend.apply_image_resource(operation),
        }
    }

    fn execute_serial_command(&mut self, command: &RenderCommand) {
        match self {
            Self::Gl(backend) => backend.execute_serial_command(command),
            Self::Cpu(backend) => backend.execute_serial_command(command),
        }
    }

    fn execute_serial_command_timed(&mut self, command: &RenderCommand, timer: &mut StageTimer) {
        match self {
            Self::Gl(backend) => backend.execute_serial_command_timed(command, timer),
            Self::Cpu(backend) => backend.execute_serial_command_timed(command, timer),
        }
    }

    fn execute_worker(
        &mut self,
        commands: &[RenderCommand],
        captures: &[CaptureId],
        operations: Vec<ImageResourceOperation>,
    ) {
        match self {
            Self::Gl(backend) => backend.execute_worker(commands, captures, operations),
            Self::Cpu(backend) => backend.execute_worker(commands, captures, operations),
        }
    }

    fn synchronize(&mut self) {
        match self {
            Self::Gl(backend) => backend.synchronize(),
            Self::Cpu(backend) => backend.synchronize(),
        }
    }

    fn set_worker_swap_interval(&mut self, interval: i32) {
        match self {
            Self::Gl(backend) => backend.set_worker_swap_interval(interval),
            Self::Cpu(backend) => backend.set_worker_swap_interval(interval),
        }
    }

    fn worker_swap_interval(&self) -> i32 {
        match self {
            Self::Gl(backend) => backend.worker_swap_interval(),
            Self::Cpu(backend) => backend.worker_swap_interval(),
        }
    }

    fn resize_backend(&mut self, width: i32, height: i32) {
        match self {
            Self::Gl(backend) => backend.resize_backend(width, height),
            Self::Cpu(backend) => backend.resize_backend(width, height),
        }
    }

    fn finish_software(&mut self) {
        match self {
            Self::Gl(backend) => backend.finish_software(),
            Self::Cpu(backend) => backend.finish_software(),
        }
    }

    fn software_pixels(&self) -> Vec<u8> {
        match self {
            Self::Gl(backend) => backend.software_pixels(),
            Self::Cpu(backend) => backend.software_pixels(),
        }
    }

    fn read_pixels_backend(&mut self) -> ImageLevel {
        match self {
            Self::Gl(backend) => backend.read_pixels_backend(),
            Self::Cpu(backend) => backend.read_pixels_backend(),
        }
    }

    fn present_backend(&mut self) {
        match self {
            Self::Gl(backend) => backend.present_backend(),
            Self::Cpu(backend) => backend.present_backend(),
        }
    }

    fn driver(&self) -> Option<RenderDriverInfo> {
        match self {
            Self::Gl(backend) => backend.driver(),
            Self::Cpu(backend) => backend.driver(),
        }
    }

    fn gl_config(&self) -> Option<RenderGlConfig> {
        match self {
            Self::Gl(backend) => backend.gl_config(),
            Self::Cpu(backend) => backend.gl_config(),
        }
    }

    fn close_backend(&mut self) -> Result<(), String> {
        match self {
            Self::Gl(backend) => backend.close_backend(),
            Self::Cpu(backend) => backend.close_backend(),
        }
    }
}

/// [`NativeBackendFactory`] adopting GL and building [`GlRenderer`], or
/// opening the software rasterizer (donor `openBackend`).
struct NativeWindowedBackendFactory {
    share: Rc<RefCell<WindowedShare>>,
    owner: ResourceOwner,
}

impl NativeBackendFactory for NativeWindowedBackendFactory {
    type Backend = NativeWindowedBackend;
    type Error = String;

    fn open_backend(
        &mut self,
        kind: RenderBackendKind,
        width: i32,
        height: i32,
        gamma: f64,
        worker: bool,
    ) -> Result<NativeWindowedBackend, String> {
        if worker {
            return Err("windowed worker rendering is not implemented".to_string());
        }
        if kind == RenderBackendKind::Cpu {
            let backend = NativeCpuBackend::open(width, height, gamma, self.owner.clone())?;
            return Ok(NativeWindowedBackend::Cpu(Box::new(backend)));
        }
        if width <= 0 || height <= 0 {
            return Err("windowed GL backend requires positive dimensions".to_string());
        }
        let window = self
            .share
            .borrow()
            .window
            .clone()
            .ok_or_else(|| "windowed backend has no window".to_string())?;
        let transfer = window
            .borrow_mut()
            .detach_render_context()
            .map_err(|error| error.to_string())?;
        let adopted = match SdlWorkerRenderContext::adopt(&transfer, &NativeLibraryOptions::default()) {
            Ok(context) => context,
            Err(error) => {
                restore_window(&self.share);
                return Err(error.to_string());
            }
        };
        let raw = Box::into_raw(Box::new(adopted));
        // SAFETY: `raw` is a live unique box; the derived borrow lives in the
        // returned backend and dies before `release_parts` reconstructs it.
        let mut platform = match PlatformGlContext::load(unsafe { &mut *raw }) {
            Ok(platform) => platform,
            Err(error) => {
                unsafe {
                    NativeGlBackend::free_worker(raw);
                }
                restore_window(&self.share);
                return Err(error.to_string());
            }
        };
        if !gl_capable(&mut platform) {
            drop(platform);
            unsafe {
                NativeGlBackend::free_worker(raw);
            }
            restore_window(&self.share);
            return Err("OpenGL renderer requires four texture coordinate units and a depth framebuffer".to_string());
        }
        let mut renderer = GlRenderer::new(platform, self.owner.clone(), width as u32, height as u32);
        renderer.set_output_gamma(gamma as f32);
        let driver = renderer.driver().clone();
        let backend = NativeGlBackend {
            parts: Some(GlParts { renderer, worker: raw }),
            width,
            height,
            driver: Some(RenderDriverInfo {
                vendor: driver.vendor.clone(),
                renderer: driver.renderer.clone(),
                version: driver.version.clone(),
                shading_language: driver.shading_language.clone(),
            }),
            worker_interval: 1,
            color: vec4(1.0, 1.0, 1.0, 1.0),
        };
        self.share.borrow_mut().detached = true;
        Ok(NativeWindowedBackend::Gl(Box::new(backend)))
    }
}

/// [`RenderImageRegistry`] over [`SceneImageRegistry`].
struct NativeImages {
    inner: SceneImageRegistry,
    owner: RendererResourceOwner,
}

impl RenderImageRegistry for NativeImages {
    type Error = String;

    fn owner(&self) -> &RendererResourceOwner {
        &self.owner
    }

    fn drain_pending_operations(&mut self) -> Vec<ImageResourceOperation> {
        self.inner.drain_operations()
    }

    fn close_images(&mut self) -> Result<(), String> {
        self.inner.close();
        Ok(())
    }
}

/// Cap on recorded windowed seat events (a diagnostic record; the smoke run
/// has no game client consuming them).
const WINDOWED_INPUT_LOG_CAP: usize = 256;

/// Shared queue receiving seat events forwarded from the router UI tap.
type WindowedInputQueue = Rc<RefCell<Vec<SeatInputEvent>>>;

/// [`RouterWindow`] over the shared windowed [`SdlWindow`].
///
/// The windowed GL path shares its [`SdlWindow`] between the render window
/// and the backend factory, so the window cannot move into an
/// [`SdlRouterWindow`](qa_client::input::router::SdlRouterWindow); this
/// adapter replicates the `SdlRouterWindow::attach` protocol (an SDL input
/// lease plus the router window surface) over the shared handle. The router
/// never polls through this adapter: [`WindowedStartupBackend::poll`] pumps
/// once through the render window and feeds each [`SdlEvent`] to
/// [`InputRouter::handle_platform`], so the adapter only serves sizing and
/// relative-mouse capture.
struct WindowedRouterWindow {
    window: Rc<RefCell<SdlWindow>>,
    lease: Option<SdlInputLease>,
}

impl WindowedRouterWindow {
    /// Attach to the shared window, acquiring the SDL input lease.
    fn attach(window: Rc<RefCell<SdlWindow>>) -> Result<Self, String> {
        let lease = window.borrow_mut().begin_input().map_err(|error| error.to_string())?;
        Ok(Self {
            window,
            lease: Some(lease),
        })
    }
}

impl RouterWindow for WindowedRouterWindow {
    fn logical_size(&mut self) -> Result<(i32, i32), RouterError> {
        self.window
            .borrow()
            .logical_size()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn drawable_size(&mut self) -> Result<(i32, i32), RouterError> {
        self.window
            .borrow()
            .drawable_size_signed()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn poll_events(&mut self) -> Result<Vec<SdlEvent>, RouterError> {
        self.window
            .borrow_mut()
            .poll_events()
            .map_err(|error| RouterError::Platform(error.to_string()))
    }

    fn set_relative_mouse(&mut self, enabled: bool) -> Result<(), RouterError> {
        if let Some(lease) = &self.lease {
            lease
                .set_relative_mouse(enabled)
                .map_err(|error| RouterError::Platform(error.to_string()))?;
        }
        Ok(())
    }
}

impl Drop for WindowedRouterWindow {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            let _ = lease.close();
        }
    }
}

/// Build the windowed seat router: one keyboard seat whose UI tap forwards
/// every [`SeatInputEvent`] into `queue` for
/// [`WindowedStartupBackend::poll`] to deliver through
/// [`StartupBackend::input`](super::startup::StartupBackend::input). The
/// seat claims the first gamepad (donor controller slot zero); the backend
/// pumps [`SdlControllers`] into the router alongside window events.
fn open_windowed_input(seat: SeatId, queue: WindowedInputQueue, start: Instant) -> Result<InputRouter, String> {
    let tap = Rc::clone(&queue);
    let ui_event: UiCallback = Box::new(move |event, _focus| {
        tap.borrow_mut().push(event.clone());
        false
    });
    let routes = vec![SeatRoute {
        seat: Seat::new(seat.clone(), Dialect::Q2Classic, ui_event),
        controller: ControllerSelection::Automatic,
    }];
    InputRouter::new(
        routes,
        Some(seat),
        None,
        Box::new(NullRegistry),
        Box::new(move || start.elapsed().as_secs_f64() * 1000.0),
        Box::new(move || start.elapsed().as_millis() as u32),
        false,
        Box::new(|_| {}),
        false,
        None,
    )
    .map_err(|error| error.to_string())
}

/// Open the windowed controller pump, claiming the first gamepad for slot
/// zero (donor controller slot). Returns `None` when the controller
/// subsystem is unavailable — keyboard input still works — so a second
/// owner or a pad-less SDL never fails the window.
fn open_windowed_controllers() -> Option<SdlControllers> {
    let mut controllers = match SdlControllers::open() {
        Ok(controllers) => controllers,
        Err(error) => {
            eprintln!("windowed: no gamepad input ({error})");
            return None;
        }
    };
    if let Err(error) = controllers.set_assignments(&[ControllerSelection::Automatic]) {
        eprintln!("windowed: no gamepad input ({error})");
        controllers.close();
        return None;
    }
    Some(controllers)
}

/// Seat id carried by a seat input event.
fn seat_event_seat(event: &SeatInputEvent) -> &SeatId {
    match event {
        SeatInputEvent::Focus { seat, .. }
        | SeatInputEvent::Key { seat, .. }
        | SeatInputEvent::Text { seat, .. }
        | SeatInputEvent::MouseMotion { seat, .. }
        | SeatInputEvent::MouseButton { seat, .. }
        | SeatInputEvent::MouseWheel { seat, .. }
        | SeatInputEvent::ControllerButton { seat, .. }
        | SeatInputEvent::ControllerAxis { seat, .. } => seat,
    }
}

/// Open windowed audio over the SDL device factory (donor `SNDDMA_Init`
/// path through [`UnifiedAudio`] + [`SdlDeviceFactory`]). Returns `None`
/// when muted or when no usable device exists; the windowed run stays
/// silent instead of failing.
fn open_windowed_audio_on(device_name: Option<&str>, muted: bool) -> Option<UnifiedAudio> {
    if muted {
        return None;
    }
    let start = Instant::now();
    let mut random_state = 0x1234_5678_i64;
    let mut audio = UnifiedAudio::new(UnifiedAudioOptions {
        sample_rate: None,
        output_format: None,
        milliseconds: Box::new(move || start.elapsed().as_millis() as i64),
        random: Box::new(move || {
            random_state = random_state.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (random_state >> 16) & 0x7fff
        }),
        max_actors: None,
        on_sound: None,
        device_factory: Some(Rc::new(SdlDeviceFactory)),
    })
    .ok()?;
    audio.open_device(device_name, None).ok()?;
    Some(audio)
}

/// Open windowed audio on the default device; `None` is the silent
/// mute/no-device fallback, never an open failure.
fn open_windowed_audio() -> Option<UnifiedAudio> {
    open_windowed_audio_on(None, false)
}

/// Refresh windowed game audio once per frame: set the local seat/eye
/// listener, start the world's stashed map speakers once per install,
/// then drain sim sound events into the engine (donor
/// `ApplicationAudio.frame` order: listeners, receive, music, pump). The
/// windowed sim produces no presentation sound events yet, so the receive
/// drains an empty batch; the music advance and pump stay in
/// [`refresh_windowed_audio`]. Never fails; engine errors log and the run
/// stays silent.
fn refresh_windowed_bridge_audio(
    audio: &mut Option<UnifiedAudio>,
    bridge: &mut Option<AudioBridge<MountsSoundContent>>,
    seat: Option<&SeatId>,
    world: Option<&PlayWorld>,
    speakers_started: &mut bool,
) {
    let (Some(engine), Some(bridge), Some(seat), Some(world)) = (audio.as_mut(), bridge.as_mut(), seat, world) else {
        return;
    };
    bridge.update_listeners(engine, seat, world.player_eye(), world.player_actor());
    // The engine opens after the direct-launch world installs, so the
    // first frame with audio present starts the speakers; the latch keeps
    // it exactly once per install (`set_world` resets it).
    if !*speakers_started {
        *speakers_started = true;
        bridge.start_map_speakers(engine, world.map_speakers(), world.sound_family());
    }
    bridge.receive(engine, &[], &AudioAudience::World);
}

/// Refresh windowed audio once per frame: advance music decoding and pump
/// queued PCM. Never fails; a lost device stays silent.
fn refresh_windowed_audio(audio: &mut Option<UnifiedAudio>, work_ms: f64) {
    if let Some(engine) = audio.as_mut() {
        engine.update_music();
        let work = if work_ms.is_finite() && work_ms >= 0.0 {
            work_ms
        } else {
            0.0
        };
        let _ignored = engine.pump(None, work);
    }
}

/// Live world state for one windowed frame: the map presentation (world
/// geometry plus model-bearing entities) plus the clear color. The
/// view/camera (viewport, projection, target, time) is derived per frame
/// from the backend's live drawable size, seat, and clock, so the stored
/// presentation stays valid across resizes. `batches` covers the
/// presentation-less unit-test path; production scenes always carry a
/// presentation.
struct WindowedScene {
    /// Static batches submitted inside the view when no presentation loaded.
    batches: Vec<DrawBatch>,
    /// Live map presentation, prepared fresh every frame.
    presentation: Option<PlayPresentation>,
    /// View clear color (donor world-view clear).
    clear_color: Vec4,
}

impl std::fmt::Debug for WindowedScene {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowedScene")
            .field("batches", &self.batches.len())
            .field("presentation", &self.presentation)
            .field("clear_color", &self.clear_color)
            .finish()
    }
}

impl WindowedScene {
    /// Scene over explicit batches and clear color.
    #[cfg(test)]
    fn new(batches: Vec<DrawBatch>, clear_color: Vec4) -> Self {
        Self {
            batches,
            presentation: None,
            clear_color,
        }
    }
}

/// Horizontal field of view for the windowed camera (donor default).
const WINDOWED_FOV_X: f32 = 90.0;

/// Vertical field of view for a horizontal one (donor `fovY` form).
fn windowed_vertical_fov(fov_x: f32, width: i32, height: i32) -> f32 {
    ((f64::from(height) / f64::from(width) * (f64::from(fov_x) * std::f64::consts::PI / 360.0).tan()).atan() * 360.0
        / std::f64::consts::PI) as f32
}

/// Windowed camera over the live drawable size at `origin`/`angles`.
/// Returns `None` when the dimensions cannot produce a valid projection,
/// in which case the frame degrades to clear plus swap.
fn windowed_camera_for(width: i32, height: i32, origin: [f32; 3], angles: [f32; 3]) -> Option<SceneCamera> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let fov_y = windowed_vertical_fov(WINDOWED_FOV_X, width, height);
    if !fov_y.is_finite() || fov_y <= 0.0 || fov_y >= 180.0 {
        return None;
    }
    let projection = perspective_projection(WINDOWED_FOV_X, fov_y, 16384.0, 4.0).ok()?;
    Some(SceneCamera {
        origin: vec3(origin[0], origin[1], origin[2]),
        axis: angles_to_axis(vec3(angles[0], angles[1], angles[2])),
        viewport: ViewRect {
            x: 0,
            y: 0,
            width,
            height,
        },
        projection,
        clip: CameraClip::None,
    })
}

/// Default windowed camera over the live drawable size. Returns `None`
/// when the dimensions cannot produce a valid projection, in which case
/// the frame degrades to clear plus swap.
fn windowed_camera(width: i32, height: i32) -> Option<SceneCamera> {
    windowed_camera_for(width, height, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0])
}

/// Ordered view for a loaded scene plus the image uploads the backend
/// must apply before executing it. A presentation prepares the world view
/// (world plus inline-model plus entity batches) at the spawn camera;
/// without one, the static batches submit at the default camera. Returns
/// `None` when the live dimensions cannot produce a camera.
fn windowed_scene_view(
    width: i32,
    height: i32,
    seat: Option<&SeatId>,
    time_ms: f64,
    scene: &mut WindowedScene,
    player: Option<(Vec3, Vec3)>,
) -> Option<(ClientRenderView, Vec<ImageResourceOperation>)> {
    let target = match seat {
        Some(seat) => ViewTarget::Seat(seat.clone()),
        None => ViewTarget::Preview("windowed".to_string()),
    };
    if let Some(presentation) = scene.presentation.as_mut() {
        let (origin, angles) = player.map_or_else(
            || {
                presentation
                    .spawn()
                    .map_or(([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]), |spawn| {
                        (
                            [spawn.origin.x, spawn.origin.y, spawn.origin.z],
                            [spawn.angles.x, spawn.angles.y, spawn.angles.z],
                        )
                    })
            },
            |(eye, angles)| ([eye.x, eye.y, eye.z], [angles.x, angles.y, angles.z]),
        );
        let camera = windowed_camera_for(width, height, origin, angles)?;
        let clear_color = scene.clear_color;
        let (mut view, image_operations) = presentation
            .prepare_frame_view(camera, target, SourceTime::Milliseconds(time_ms))
            .ok()?;
        view.state.clear = Some(ViewClear {
            depth: 1.0,
            color: Some(clear_color),
            stencil: false,
        });
        return Some((view, image_operations));
    }
    let camera = windowed_camera(width, height)?;
    let viewport = ClientRect {
        x: 0.0,
        y: 0.0,
        width: f64::from(camera.viewport.width) as f32,
        height: f64::from(camera.viewport.height) as f32,
    };
    let operations = if scene.batches.is_empty() {
        Vec::new()
    } else {
        vec![RenderOperation::Draw(scene.batches.clone())]
    };
    Some((
        ClientRenderView {
            state: RenderViewState {
                viewport,
                clear: Some(ViewClear {
                    depth: 1.0,
                    color: Some(scene.clear_color),
                    stencil: false,
                }),
                clip_plane: None,
            },
            target,
            time: SourceTime::Milliseconds(time_ms),
            before_view: Vec::new(),
            operations,
        },
        Vec::new(),
    ))
}

/// Faithful frame commands (donor `SceneFrameBuilder` shape): draw-buffer
/// selection, one scene view when a scene is loaded, then the buffer swap,
/// plus the image uploads the backend must apply before executing the
/// view. No scene (or an unusable live size) degrades to clear plus swap.
fn build_windowed_commands(
    width: i32,
    height: i32,
    seat: Option<&SeatId>,
    time_ms: f64,
    scene: Option<&mut WindowedScene>,
    player: Option<(Vec3, Vec3)>,
) -> (Vec<RenderCommand>, Vec<ImageResourceOperation>) {
    match scene {
        None => (
            vec![
                RenderCommand::DrawBuffer {
                    buffer: DrawBuffer::Back,
                    clear: true,
                },
                RenderCommand::SwapBuffers,
            ],
            Vec::new(),
        ),
        Some(scene) => match windowed_scene_view(width, height, seat, time_ms, scene, player) {
            Some((view, image_operations)) => (
                vec![
                    RenderCommand::DrawBuffer {
                        buffer: DrawBuffer::Back,
                        clear: false,
                    },
                    RenderCommand::View(view),
                    RenderCommand::SwapBuffers,
                ],
                image_operations,
            ),
            None => (
                vec![
                    RenderCommand::DrawBuffer {
                        buffer: DrawBuffer::Back,
                        clear: true,
                    },
                    RenderCommand::SwapBuffers,
                ],
                Vec::new(),
            ),
        },
    }
}

/// Count captured pixels that differ from pure black (any nonzero RGB
/// channel), for the capture-diff proof that the running game presents
/// world geometry instead of a black frame.
#[cfg(test)]
fn count_non_black(pixels: &[u8]) -> usize {
    pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
        .count()
}

/// Stock host-frame clock (`realtime`/`oldrealtime`) for the windowed
/// play loop. `realtime` advances every display frame; `oldrealtime`
/// updates to `realtime` after a run frame, exactly like
/// `Host_FilterTime`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostFrameGate {
    realtime_s: f64,
    oldrealtime_s: f64,
    /// Raw seconds since the last run frame, stashed when [`sim_due`]
    /// passes: stock `host_frametime` before the engine clamp
    /// (`host.c` for NetQuake, `cl_main.c:1325` for QuakeWorld).
    ///
    /// [`sim_due`]: HostFrameGate::sim_due
    due_frame_s: f64,
    /// NetQuake `host_framerate` override in seconds (stock default 0).
    pub nq_host_framerate: f64,
    /// QuakeWorld `cl_maxfps` (stock default 0).
    pub qw_maxfps: f64,
    /// QuakeWorld `rate` (stock default 2500).
    pub qw_rate: f64,
    /// Timedemo exemption (`cls.timedemo`).
    pub timedemo: bool,
}

impl HostFrameGate {
    /// Fresh gate at the stock cvar defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            realtime_s: 0.0,
            oldrealtime_s: 0.0,
            due_frame_s: 0.0,
            nq_host_framerate: 0.0,
            qw_maxfps: 0.0,
            qw_rate: 2500.0,
            timedemo: false,
        }
    }

    /// Advance the wall clock by one display frame.
    pub fn advance(&mut self, elapsed_s: f64) {
        self.realtime_s += elapsed_s;
    }

    /// Refresh the throttle from the live cvar registry: NetQuake
    /// `host_framerate`, QuakeWorld `cl_maxfps`/`rate`. The loop calls
    /// this every frame before [`sim_due`](Self::sim_due), so a
    /// player-set cvar changes the throttle on the next frame.
    /// `timedemo` is deliberately untouched: it stays a loop/CLI-fed
    /// flag (`cls.timedemo`), never a cvar read.
    pub fn sync(&mut self, cvars: &CvarRegistry) {
        let controls = read_frame_time_controls(cvars);
        self.nq_host_framerate = controls.host_framerate;
        self.qw_maxfps = controls.maxfps;
        self.qw_rate = controls.rate;
    }

    /// Whether the simulation runs this frame, updating `oldrealtime`
    /// on run frames. Non-Q1 dialects always run.
    pub fn sim_due(&mut self, dialect: Dialect) -> bool {
        let due = match dialect {
            Dialect::Q1Quakeworld => {
                if self.oldrealtime_s > self.realtime_s {
                    self.oldrealtime_s = 0.0;
                }
                qw_frame_due(
                    self.realtime_s,
                    self.oldrealtime_s,
                    qw_fps(self.qw_maxfps, self.qw_rate),
                    self.timedemo,
                )
            }
            Dialect::Q1Netquake => nq_frame_due(self.realtime_s, self.oldrealtime_s, self.timedemo),
            _ => true,
        };
        if due {
            self.due_frame_s = self.realtime_s - self.oldrealtime_s;
            self.oldrealtime_s = self.realtime_s;
        }
        due
    }

    /// Stock host-frametime for a due Q1 sim frame, in milliseconds:
    /// the raw `realtime - oldrealtime` span through the engine clamp
    /// (`host.c` for NetQuake, `cl_main.c:1325-1328` for QuakeWorld),
    /// via the shared [`source_frame_milliseconds`] transform. Call
    /// only after [`sim_due`](Self::sim_due) passes.
    pub fn sim_frame_ms(&self, dialect: Dialect) -> f64 {
        let raw_ms = self.due_frame_s * 1000.0;
        let controls = FrameTimeControls {
            timescale: 1.0,
            fixedtime: 0.0,
            host_framerate: self.nq_host_framerate,
            camera_mode: 0.0,
            maxfps: self.qw_maxfps,
            rate: self.qw_rate,
        };
        let host = FrameTimeHost {
            dedicated: false,
            local_server: true,
        };
        source_frame_milliseconds(dialect, raw_ms, &controls, &host).unwrap_or(raw_ms)
    }
}

impl Default for HostFrameGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Production [`StartupBackend`] presenting frames on a native window.
pub struct WindowedStartupBackend {
    width: u32,
    height: u32,
    hidden: bool,
    gamma: f64,
    backend_kind: RenderBackendKind,
    identity: IdentityOwner,
    owner: Option<RendererResourceOwner>,
    renderer: Option<WindowedRenderer>,
    backends: Option<NativeWindowedBackendFactory>,
    audio: Option<UnifiedAudio>,
    audio_bridge: Option<AudioBridge<MountsSoundContent>>,
    audio_speakers_started: bool,
    share: Rc<RefCell<WindowedShare>>,
    quit: Rc<Cell<bool>>,
    start: Instant,
    input_router: Option<InputRouter>,
    input_seat: Option<SeatId>,
    local_player: Option<LocalPlayer>,
    input_queue: WindowedInputQueue,
    input_log: VecDeque<SeatInputEvent>,
    command_builder: Option<InputCommandBuilder>,
    controllers: Option<SdlControllers>,
    launch: MenuLaunchQueue,
    scene: Option<WindowedScene>,
    world: Option<PlayWorld>,
    menu: Option<WindowedMenu>,
    timer: StageTimer,
    totals: StageTotals,
    host_gate: HostFrameGate,
    /// Live frame-time cvars for the loaded world dialect, installed at
    /// world set; the play step syncs the host gate from these every
    /// frame so player-set `cl_maxfps`/`host_framerate`/`rate` values
    /// change the throttle without a relaunch.
    cvars: Option<CvarRegistry>,
}

impl WindowedStartupBackend {
    /// Build an unopened backend over a resolved config and identity. The
    /// identity owns the GL renderer, the image registry, and the input
    /// seat; the map world (loaded before open) builds its presentation
    /// over the same identity so its image uploads apply to the backend.
    fn new(
        config: &StartupConfig,
        hidden: bool,
        gamma: f64,
        backend_kind: RenderBackendKind,
        quit: Rc<Cell<bool>>,
        identity: IdentityOwner,
    ) -> Self {
        Self {
            width: config.width,
            height: config.height,
            hidden,
            gamma,
            backend_kind,
            identity,
            owner: None,
            renderer: None,
            backends: None,
            audio: None,
            audio_bridge: None,
            audio_speakers_started: false,
            share: Rc::new(RefCell::new(WindowedShare::default())),
            quit,
            start: Instant::now(),
            input_router: None,
            input_seat: None,
            local_player: None,
            input_queue: Rc::new(RefCell::new(Vec::new())),
            input_log: VecDeque::new(),
            command_builder: None,
            controllers: None,
            launch: MenuLaunchQueue::new(),
            scene: None,
            world: None,
            menu: None,
            timer: StageTimer::new(false),
            totals: StageTotals::new(),
            host_gate: HostFrameGate::new(),
            cvars: None,
        }
    }

    /// Live frame-time cvar registry for the loaded world, when set.
    #[must_use]
    pub fn cvars(&self) -> Option<&CvarRegistry> {
        self.cvars.as_ref()
    }

    /// Mutable live frame-time cvar registry (console/player writes).
    pub fn cvars_mut(&mut self) -> Option<&mut CvarRegistry> {
        self.cvars.as_mut()
    }

    /// Feed the timedemo exemption (`cls.timedemo`) from the loop/CLI.
    pub fn set_timedemo(&mut self, timedemo: bool) {
        self.host_gate.timedemo = timedemo;
    }

    /// Enable or disable per-stage frame timing collection.
    pub fn set_frame_timings(&mut self, enabled: bool) {
        self.timer = StageTimer::new(enabled);
    }

    /// Run-wide stage totals, when timing is enabled.
    pub fn timing_totals_mut(&mut self) -> Option<&mut StageTotals> {
        if self.timer.enabled() {
            Some(&mut self.totals)
        } else {
            None
        }
    }

    /// Rendered `--frame-timings` report, when timing is enabled.
    #[must_use]
    pub fn timing_report(&self) -> Option<String> {
        if self.timer.enabled() {
            Some(self.totals.render())
        } else {
            None
        }
    }

    /// Shared menu launch queue (donor `pending`): the menu entry pushes
    /// [`StartupAction`] values from its launch buttons.
    fn launch_queue(&self) -> MenuLaunchQueue {
        self.launch.clone()
    }

    /// Active menu id behind the overlay, if the menu entry is showing.
    #[must_use]
    pub fn menu_active_menu(&self) -> Option<String> {
        self.menu.as_ref()?.active_menu().map(|id| id.as_str().to_string())
    }

    /// Focused control id on the active menu, if any.
    #[must_use]
    pub fn menu_focus_control(&self) -> Option<String> {
        self.menu.as_ref()?.focus_control().map(|id| id.as_str().to_string())
    }

    /// Whether the menu overlay is still showing (false after a launch).
    #[must_use]
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Test-only pump: feed one platform event through the router exactly
    /// as [`StartupBackend::poll`](super::startup::StartupBackend::poll)
    /// would after pumping it from the window.
    pub fn inject_platform_event(&mut self, event: SdlEvent) -> Result<(), String> {
        self.handle_window_events(vec![event])
    }

    /// Test-only pump: feed one controller event through the router exactly
    /// as the controller half of `poll` would after pumping `SdlControllers`.
    pub fn inject_controller_event(&mut self, event: ControllerEvent) -> Result<(), String> {
        if let Some(router) = self.input_router.as_mut() {
            router.handle_controller(event).map_err(|error| error.to_string())?;
        }
        self.forward_queued_input();
        Ok(())
    }

    /// Adopt the menu overlay for the menu entry (donor frontend menu).
    fn set_menu(&mut self, menu: WindowedMenu) {
        self.menu = Some(menu);
    }

    /// Adopt a loaded map world, publishing its scene view: the world
    /// presentation (world geometry plus model-bearing entities) when it
    /// built, else the legacy empty batches. The live actor count is
    /// available through [`Self::world_entity_count`].
    fn set_world(&mut self, mut world: PlayWorld) {
        let presentation = world.take_presentation();
        self.scene = Some(WindowedScene {
            batches: Vec::new(),
            presentation,
            clear_color: vec4(0.0, 0.0, 0.0, 1.0),
        });
        let mut builder = InputCommandBuilder::new(dialect_family(world.dialect()));
        if let Some((_, angles)) = world.player_eye() {
            let _ignored = builder.set_view_angles(angles);
        }
        self.command_builder = Some(builder);
        self.audio_bridge = world
            .audio_mounts()
            .map(|mounts| AudioBridge::new(MountsSoundContent::new(mounts)));
        self.audio_speakers_started = false;
        self.install_frame_time_cvars(world.dialect());
        self.world = Some(world);
        self.apply_input_profile();
        self.pair_local_player();
    }

    /// Install the live frame-time cvar registry for a world dialect
    /// (stock `host_framerate` for NetQuake, `cl_maxfps` plus `rate`
    /// for QuakeWorld). A same-dialect relaunch keeps the existing
    /// registry so player-set values survive map changes.
    fn install_frame_time_cvars(&mut self, dialect: Dialect) {
        if self.cvars.as_ref().is_some_and(|cvars| cvars.dialect() == dialect) {
            return;
        }
        let mut cvars = CvarRegistry::new(dialect);
        if register_frame_time_cvars(&mut cvars).is_err() {
            return;
        }
        if dialect == Dialect::Q1Quakeworld
            && cvars.get("rate").is_none()
            && cvars.register("rate", "2500", 0).is_err()
        {
            return;
        }
        self.cvars = Some(cvars);
    }

    /// Pair the driving seat with the admitted body as the one local
    /// player: the established [`LocalPlayer`] identity (seat plus the
    /// body's own simulation actor), not a parallel player concept. Runs
    /// at world set and at input-profile install, whichever sees both
    /// the seat and the admitted body; idempotent.
    fn pair_local_player(&mut self) {
        let (Some(seat), Some(world)) = (self.input_seat.clone(), self.world.as_ref()) else {
            return;
        };
        let Some(actor) = world.player_actor() else {
            return;
        };
        let client = self.identity.client(0, 0);
        self.local_player = Some(LocalPlayer {
            seat: SessionSeat::new(seat, client),
            actor: actor.clone(),
        });
    }

    /// The paired local player (driving seat plus admitted body actor),
    /// or `None` until a seat and a body are both present.
    #[must_use]
    pub fn local_player(&self) -> Option<&LocalPlayer> {
        self.local_player.as_ref()
    }

    /// Install the world's dialect and default key bindings on the live
    /// seat (donor startup input profile). Runs at world set and at open,
    /// whichever sees both the world and the router; direct launches set
    /// the world before the router exists, menu launches after.
    fn apply_input_profile(&mut self) {
        let (Some(world), Some(router), Some(seat)) =
            (self.world.as_ref(), self.input_router.as_mut(), self.input_seat.clone())
        else {
            return;
        };
        let dialect = world.dialect();
        let Some(seat_input) = router.seat_mut(&seat) else {
            return;
        };
        if let Err(error) = seat_input.set_profile(dialect) {
            eprintln!("windowed: keeping input profile ({error})");
        }
        seat_input.unbind_all();
        for binding in super::play::play_action_bindings(dialect) {
            seat_input.bind(binding);
        }
        self.pair_local_player();
    }

    /// Run one interactive play step: sample the seat, build the family
    /// user command, step the admitted player, and tick the server. The
    /// server ticks even without a player so the world animates; command
    /// failures keep the previous frame's stillness instead of aborting.
    fn step_play(&mut self, elapsed_ms: f64) {
        if self.menu.is_some() {
            return;
        }
        let Some(world) = self.world.as_ref() else {
            return;
        };
        // Stock host-frame gate: sync the throttle from the live
        // cvar registry, then skip simulation frames that arrive too
        // early for the launched game's rate (NetQuake 72Hz,
        // QuakeWorld fps rule).
        if let Some(cvars) = self.cvars.as_ref() {
            self.host_gate.sync(cvars);
        }
        if !self.host_gate.sim_due(world.dialect()) {
            return;
        }
        // Stock host-frametime: the span since the last run frame
        // through the engine clamp, so a display hitch never feeds a
        // multi-second step into Q1 physics. Non-Q1 dialects keep the
        // raw display elapsed.
        let dialect = world.dialect();
        let sim_ms = if dialect == Dialect::Q1Netquake || dialect == Dialect::Q1Quakeworld {
            self.host_gate.sim_frame_ms(dialect)
        } else {
            elapsed_ms
        };
        let command = if self.world.as_ref().is_some_and(PlayWorld::has_player) {
            self.sample_player_command()
        } else {
            None
        };
        let Some(world) = self.world.as_mut() else {
            return;
        };
        // Tick first: the player step below reads the fresh clock frame
        // for its step length, so a tickless step would stand still.
        if let Err(error) = world.server_mut().tick_wall_milliseconds(sim_ms) {
            eprintln!("windowed: server tick failed ({error})");
        }
        if let Some(command) = command {
            if let Err(error) = world.step_player(command) {
                eprintln!("windowed: player step failed ({error})");
            }
        }
    }

    /// Sample the live seat and build one world user command, or `None`
    /// when input is not ready (router, seat, or builder missing).
    fn sample_player_command(&mut self) -> Option<qa_world::movement::types::UserCommand> {
        let router = self.input_router.as_mut()?;
        let seat = self.input_seat.clone()?;
        let builder = self.command_builder.as_mut()?;
        let seat_input = router.seat_mut(&seat)?;
        let now_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        let frame = seat_input.sample(now_ms, 16.0).ok()?;
        let sample = seat_sample(&frame);
        let world = self.world.as_ref()?;
        let clock = world.server().simulation().frame();
        let context = match builder.family() {
            qa_world::client::ClientFamily::Q1Netquake => ClientFrameContext::Q1Netquake {
                ack_time_s: clock.time.as_seconds_f64(),
                pitch_drift: None,
            },
            qa_world::client::ClientFamily::Q1Quakeworld => ClientFrameContext::Q1Quakeworld { pitch_drift: None },
            qa_world::client::ClientFamily::Q2Classic => ClientFrameContext::Q2Classic {
                delta_angles: vec3(0.0, 0.0, 0.0),
                light_level: 0,
                attack_allowed: true,
            },
            qa_world::client::ClientFamily::Q2Rerelease => ClientFrameContext::Q2Rerelease {
                delta_angles: vec3(0.0, 0.0, 0.0),
                server_frame: clock.frame,
                attack_allowed: true,
            },
            qa_world::client::ClientFamily::Q3 => ClientFrameContext::Q3 {
                server_time_ms: clock.time.as_milliseconds_truncated(),
                weapon: 0,
                sensitivity: 1.0,
            },
        };
        let built = builder.build(&sample, &context).ok()?;
        Some(net_to_world(&user_command(&built)))
    }

    /// Live map-entity count, or `None` when no map world loaded.
    #[must_use]
    pub fn world_entity_count(&self) -> Option<usize> {
        self.world.as_ref().map(PlayWorld::entity_count)
    }

    /// Display refresh rate in Hz hosting the window, or 0 when the
    /// window is not open or SDL reports none (the frame pacer falls
    /// back to its default rate then).
    #[must_use]
    pub fn display_refresh_hz(&self) -> i32 {
        self.share
            .borrow()
            .window
            .as_ref()
            .and_then(|window| window.borrow().display().ok())
            .map(|display| display.refresh_rate)
            .unwrap_or(0)
    }

    /// Live drawable size: the window's current drawable when valid,
    /// otherwise the backend's dimensions, otherwise the configured size.
    fn live_size(&self) -> (i32, i32) {
        if let Some(renderer) = self.renderer.as_ref() {
            let (width, height) = renderer.window().drawable_size();
            if width > 0 && height > 0 {
                return (width, height);
            }
            let (width, height) = (renderer.backend().width(), renderer.backend().height());
            if width > 0 && height > 0 {
                return (width, height);
            }
        }
        (self.width as i32, self.height as i32)
    }

    /// Borrow the live renderer, driver factory, and owner.
    fn live_parts(
        &mut self,
    ) -> Result<
        (
            &mut WindowedRenderer,
            &mut NativeWindowedBackendFactory,
            RendererResourceOwner,
        ),
        String,
    > {
        let (Some(renderer), Some(backends), Some(owner)) =
            (self.renderer.as_mut(), self.backends.as_mut(), self.owner.clone())
        else {
            return Err("windowed backend is not open".to_string());
        };
        Ok((renderer, backends, owner))
    }

    /// Feed pumped window events through the seat router, then deliver the
    /// resulting seat events through backend input.
    fn handle_window_events(&mut self, events: Vec<SdlEvent>) -> Result<(), String> {
        for event in events {
            if matches!(
                event,
                SdlEvent::Quit { .. }
                    | SdlEvent::Window {
                        event: SDL_WINDOWEVENT_CLOSE,
                        ..
                    }
            ) {
                self.quit.set(true);
            }
            if let Some(router) = self.input_router.as_mut() {
                router.handle_platform(event).map_err(|error| error.to_string())?;
            }
        }
        self.forward_queued_input();
        Ok(())
    }

    /// Deliver seat events forwarded from the router UI tap through backend
    /// input (donor `input`).
    fn forward_queued_input(&mut self) {
        let forwarded = std::mem::take(&mut *self.input_queue.borrow_mut());
        for event in forwarded {
            self.input(&event);
        }
    }

    /// Drain queued menu launches (donor `step` pending consumption):
    /// resolve each action through the menu model and swap the menu overlay
    /// for the scene path. Failures latch a status line on the menu and
    /// keep it usable, like the donor's failed-launch status.
    fn drain_menu_launch(&mut self) {
        if self.menu.is_none() {
            self.launch.drain();
            return;
        }
        for action in self.launch.drain() {
            if self.menu.is_none() {
                break;
            }
            if let Err(error) = self.launch_menu_game(&action) {
                if let Some(menu) = self.menu.as_ref() {
                    menu.set_status(&error);
                }
            }
        }
    }

    /// Launch one menu action into the game view: resolve options, load the
    /// map world, release the menu images, and drop the overlay so frames
    /// render the scene path and [`StartupBackend::has_active_game`] turns
    /// true (donor `Application.openBorrowed` tail, inlined: this build has
    /// no separate game client).
    fn launch_menu_game(&mut self, action: &StartupAction) -> Result<(), String> {
        let Some(menu) = self.menu.as_ref() else {
            return Ok(());
        };
        if let StartupAction::Load { path, .. } = action {
            eprintln!("windowed: menu load {path} launches the draft world (saves are not loaded in this build)");
        }
        let (options, catalog) = {
            let mut model = menu.model().borrow_mut();
            let options = launch_options(&mut model, action)?;
            (options, model.catalog().clone())
        };
        let config = StartupConfig::from_options(&options).map_err(|error| error.to_string())?;
        let owner = ResourceOwner::new(7, self.identity.session().clone(), 0);
        let world = load_play_world(&config, &catalog, &options, owner).map_err(|error| error.to_string())?;
        eprintln!(
            "windowed: menu launched {} of {} map entities ({} {})",
            world.spawned(),
            world.entity_records(),
            world.content(),
            world.map()
        );
        if let Some(menu) = self.menu.as_ref() {
            let releases = menu.release_images();
            if !releases.is_empty() {
                if let Some(renderer) = self.renderer.as_mut() {
                    for operation in &releases {
                        renderer.backend_mut().apply_image_resource(operation);
                    }
                }
            }
        }
        self.set_world(world);
        self.menu = None;
        Ok(())
    }

    /// Build one frame's commands plus the image uploads the backend must
    /// apply before executing them. The menu entry presents the menu
    /// overlay; anything else keeps the direct-launch scene path.
    fn frame_commands(&mut self) -> (Vec<RenderCommand>, Vec<ImageResourceOperation>) {
        let (width, height) = self.live_size();
        let time_ms = self.now_ms();
        let seat = self.input_seat.clone();
        if let Some(menu) = self.menu.as_mut() {
            if let Some((view, image_operations)) = menu.frame_view(width, height, seat.as_ref(), time_ms) {
                return (
                    vec![
                        RenderCommand::DrawBuffer {
                            buffer: DrawBuffer::Back,
                            clear: false,
                        },
                        RenderCommand::View(view),
                        RenderCommand::SwapBuffers,
                    ],
                    image_operations,
                );
            }
            return (
                vec![
                    RenderCommand::DrawBuffer {
                        buffer: DrawBuffer::Back,
                        clear: true,
                    },
                    RenderCommand::SwapBuffers,
                ],
                Vec::new(),
            );
        }
        let player = self.world.as_ref().and_then(PlayWorld::player_eye);
        build_windowed_commands(width, height, seat.as_ref(), time_ms, self.scene.as_mut(), player)
    }

    /// Apply image uploads to the live backend before executing a view.
    fn apply_frame_images(renderer: &mut WindowedRenderer, image_operations: &[ImageResourceOperation]) {
        for operation in image_operations {
            renderer.backend_mut().apply_image_resource(operation);
        }
    }

    /// Arm a capture, present one frame, and return its pixels.
    fn present_and_capture(&mut self) -> Result<Vec<u8>, String> {
        let (commands, image_operations) = self.frame_commands();
        let (renderer, backends, owner) = self.live_parts()?;
        let slot: Rc<RefCell<Option<Result<ImageLevel, String>>>> = Rc::new(RefCell::new(None));
        let writer = Rc::clone(&slot);
        renderer
            .capture_next_frame(Box::new(move |result| {
                *writer.borrow_mut() = Some(result);
            }))
            .map_err(|error| error.to_string())?;
        Self::apply_frame_images(renderer, &image_operations);
        let frame = RenderFrame { owner, commands };
        renderer.execute(&frame, backends).map_err(|error| error.to_string())?;
        let captured = slot.borrow_mut().take();
        match captured {
            Some(Ok(image)) => Ok(image.pixels),
            Some(Err(error)) => Err(error),
            None => Err("windowed frame capture did not complete".to_string()),
        }
    }
}

impl StartupBackend for WindowedStartupBackend {
    fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn sleep_ms(&self, ms: f64) {
        if ms.is_finite() && ms > 0.0 {
            thread::sleep(Duration::from_secs_f64(ms / 1000.0));
        }
    }

    fn open(&mut self) -> Result<(), String> {
        if self.renderer.is_some() {
            return Ok(());
        }
        let resource_owner = ResourceOwner::new(7, self.identity.session().clone(), 0);
        let owner = RendererResourceOwner {
            identity: "windowed".to_string(),
            session: 0,
            generation: 0,
        };
        let mut windows = NativeSdlWindowFactory {
            share: Rc::clone(&self.share),
        };
        let mut backends = NativeWindowedBackendFactory {
            share: Rc::clone(&self.share),
            owner: resource_owner.clone(),
        };
        let images = NativeImages {
            inner: SceneImageRegistry::new(resource_owner),
            owner: owner.clone(),
        };
        let options = NativeRendererOptions {
            renderer: self.backend_kind,
            width: self.width as i32,
            height: self.height as i32,
            hidden: self.hidden,
            gamma: self.gamma,
            render_worker: false,
        };
        let renderer = NativeRenderer::open(&options, owner.clone(), images, &mut windows, &mut backends)
            .map_err(|error| error.to_string())?;
        self.owner = Some(owner);
        self.renderer = Some(renderer);
        self.backends = Some(backends);
        let seat = self.identity.seat(0);
        let mut router = open_windowed_input(seat.clone(), Rc::clone(&self.input_queue), self.start)?;
        let window = self
            .share
            .borrow()
            .window
            .clone()
            .ok_or_else(|| "windowed backend has no window".to_string())?;
        router
            .attach_window(Box::new(WindowedRouterWindow::attach(window)?))
            .map_err(|error| error.to_string())?;
        self.input_router = Some(router);
        self.input_seat = Some(seat);
        self.controllers = open_windowed_controllers();
        self.audio = open_windowed_audio();
        self.apply_input_profile();
        Ok(())
    }

    fn poll(&mut self) -> Result<(), String> {
        let events = {
            let Some(renderer) = self.renderer.as_mut() else {
                return Ok(());
            };
            renderer.window().poll_events()?
        };
        self.timer.section("input");
        self.handle_window_events(events)?;
        if let (Some(controllers), Some(router)) = (self.controllers.as_mut(), self.input_router.as_mut()) {
            pump_windowed_controllers(controllers, router)?;
            self.forward_queued_input();
        }
        self.timer.stop();
        Ok(())
    }

    fn frame(&mut self, ctx: &mut StartupFrame<'_>) -> Result<(), String> {
        self.timer.section("scene");
        self.host_gate.advance(ctx.elapsed_ms / 1000.0);
        self.drain_menu_launch();
        self.step_play(ctx.elapsed_ms);
        let (commands, image_operations) = self.frame_commands();
        // Inline the live borrow (rather than `live_parts`) so the timer
        // field stays reachable for the timed execute below.
        let (Some(renderer), Some(backends), Some(owner)) =
            (self.renderer.as_mut(), self.backends.as_mut(), self.owner.clone())
        else {
            self.timer.take_frame();
            return Err("windowed backend is not open".to_string());
        };
        self.timer.section("images");
        Self::apply_frame_images(renderer, &image_operations);
        let frame = RenderFrame { owner, commands };
        if let Err(error) = renderer.execute_with_timer(&frame, backends, &mut self.timer) {
            self.timer.take_frame();
            return Err(error.to_string());
        }
        self.timer.section("audio");
        refresh_windowed_bridge_audio(
            &mut self.audio,
            &mut self.audio_bridge,
            self.input_seat.as_ref(),
            self.world.as_ref(),
            &mut self.audio_speakers_started,
        );
        if let Some(menu) = self.menu.as_mut() {
            menu.drain_menu_audio(&mut self.audio);
        }
        refresh_windowed_audio(&mut self.audio, ctx.elapsed_ms);
        let samples = self.timer.take_frame();
        self.totals.add_frame(&samples);
        Ok(())
    }

    fn input(&mut self, event: &SeatInputEvent) -> bool {
        let Some(seat) = self.input_seat.as_ref() else {
            return false;
        };
        if seat_event_seat(event) != seat {
            return false;
        }
        if self.input_log.len() >= WINDOWED_INPUT_LOG_CAP {
            self.input_log.pop_front();
        }
        self.input_log.push_back(event.clone());
        if let Some(menu) = self.menu.as_ref() {
            menu.input(&super::windowed_menu::convert_router_event(event));
        }
        true
    }

    fn request_quit(&mut self) {
        self.quit.set(true);
    }

    fn read_pixels(&self) -> Result<Vec<u8>, String> {
        // GL readback requires an armed capture; see `capture_next_frame`.
        Err("GL captures must be requested before presentation with captureNextFrame()".to_string())
    }

    fn capture_next_frame(&mut self) -> Result<Vec<u8>, String> {
        self.present_and_capture()
    }

    fn has_active_game(&self) -> bool {
        self.scene.is_some()
    }

    fn input_seat(&self) -> Option<SeatId> {
        self.input_seat.clone()
    }

    fn close(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(router) = self.input_router.as_mut() {
            if let Err(error) = router.close() {
                failures.push(error.to_string());
            }
        }
        self.input_router = None;
        if let Some(mut controllers) = self.controllers.take() {
            controllers.close();
        }
        if let Some(menu) = self.menu.as_ref() {
            let releases = menu.release_images();
            if !releases.is_empty() {
                if let Some(renderer) = self.renderer.as_mut() {
                    for operation in &releases {
                        renderer.backend_mut().apply_image_resource(operation);
                    }
                }
            }
        }
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(error) = renderer.close() {
                failures.push(error.to_string());
            }
        }
        self.renderer = None;
        self.scene = None;
        self.world = None;
        self.audio_bridge = None;
        self.local_player = None;
        self.menu = None;
        if let Some(mut audio) = self.audio.take() {
            let _ignored = audio.close();
        }
        failures
    }
}

/// Opened windowed application plus its external quit flag.
pub struct WindowedApplication {
    /// Startup application (donor `StartupApplication.open` result).
    pub app: StartupApplication<WindowedStartupBackend>,
    /// Set when the window requests quit.
    pub quit: Rc<Cell<bool>>,
}

/// Open the windowed composition: selection model over the real catalog,
/// production GL or CPU backend, and [`StartupApplication::open`]. Fails
/// before touching a window when the renderer selection is not supported.
pub fn open_windowed_application(
    options: &ApplicationOptions,
    entry: StartupEntry,
) -> Result<WindowedApplication, String> {
    if options.render_worker == Some(true) {
        return Err("windowed worker rendering is not implemented".to_string());
    }
    let kind = match options.renderer {
        Renderer::Cpu => RenderBackendKind::Cpu,
        Renderer::Gl => RenderBackendKind::Gl,
    };
    let config = StartupConfig::from_options(options).map_err(|error| error.to_string())?;
    let catalog = discover_installed_content(&DiscoverContentOptions::new(PathBuf::from(&options.corpus_root)))
        .map_err(|error| error.to_string())?;
    let model = StartupSelectionModel::new(catalog, options.clone(), Box::new(WindowedPresetCollaborators))
        .map_err(|error| error.to_string())?;
    let quit = Rc::new(Cell::new(false));
    let identity = IdentityOwner::create("windowed").map_err(|error| error.to_string())?;
    let menu_seat = identity.seat(0);
    let menu_client = identity.client(0, 0);
    let resource_owner = ResourceOwner::new(7, identity.session().clone(), 0);
    let mut backend =
        WindowedStartupBackend::new(&config, options.hidden, options.gamma, kind, Rc::clone(&quit), identity);
    backend.set_frame_timings(options.frame_timings);
    if entry == StartupEntry::Menu {
        let mut menu_model = StartupSelectionModel::new(
            model.catalog().clone(),
            options.clone(),
            Box::new(WindowedPresetCollaborators),
        )
        .map_err(|error| error.to_string())?;
        menu_model.prepare_maps().map_err(|error| error.to_string())?;
        let menu = WindowedMenu::open(
            menu_model,
            menu_seat,
            menu_client,
            resource_owner,
            Rc::clone(&quit),
            backend.launch_queue(),
        )?;
        backend.set_menu(menu);
    } else {
        match load_play_world(&config, model.catalog(), options, resource_owner) {
            Ok(world) => {
                eprintln!(
                    "windowed: spawned {} of {} map entities ({} {})",
                    world.spawned(),
                    world.entity_records(),
                    world.content(),
                    world.map()
                );
                if let Some(presentation) = world.presentation() {
                    eprintln!(
                        "windowed: presenting {} world surfaces, {} model entities, {} inline models ({} skipped models)",
                        presentation.surface_count(),
                        presentation.entities().len(),
                        presentation.inline_models().len(),
                        presentation.skipped_models().len()
                    );
                } else if let Some(reason) = world.presentation_error() {
                    eprintln!("windowed: no scene presentation ({reason})");
                }
                backend.set_world(world);
            }
            Err(error) => eprintln!("windowed: no map world ({error})"),
        }
    }
    let app = StartupApplication::open(model, backend, entry).map_err(|error| error.to_string())?;
    Ok(WindowedApplication { app, quit })
}

/// Drive frames until quit, close, or the frame limit, pacing at the
/// display rate (vsync through the real swap path where the driver
/// honors the renderer's swap interval, plus a sleep-based minimum
/// frame time fallback for drivers that ignore swap control), then
/// close and report stepped frames.
pub fn drive_windowed_application(
    app: &mut StartupApplication<WindowedStartupBackend>,
    quit: &Rc<Cell<bool>>,
    frame_limit: Option<u64>,
) -> Result<u64, StartupError> {
    let mut pacer = WindowedPacer::for_refresh_hz(app.backend().display_refresh_hz());
    loop {
        if quit.get() || app.is_stopping() || app.is_closed() {
            break;
        }
        if frame_limit.is_some_and(|limit| app.frames() >= limit) {
            break;
        }
        match app.step() {
            Ok(()) => {}
            Err(error) => {
                // Prefer the step failure; close failures on this path are superseded.
                let _close_ignored = app.close();
                return Err(error);
            }
        }
        let pace_start = Instant::now();
        pacer.end_frame();
        if let Some(totals) = app.backend_mut().timing_totals_mut() {
            totals.add_sample(&StageSample {
                name: "pacer",
                elapsed: pace_start.elapsed(),
            });
        }
    }
    app.close()?;
    Ok(app.frames())
}

/// Serializes tests that open real windows: SDL video init/quit is
/// process-wide, so parallel window tests tear down each other's X
/// connection.
#[cfg(test)]
pub(crate) static WINDOWED_GL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use qa_client::input::router::{SdlRouterWindow, WINDOW_FOCUS_GAINED, WINDOW_FOCUS_LOST};
    use qa_content::catalog::{CatalogProduct, InstalledCatalog, ProductAvailability, ProductExpectation};
    use qa_content::contract::ContentId;
    use qa_content::contract::GameFamily;
    use qa_content::contract::ProviderReference;
    use qa_core::identity::ProviderId;
    use qa_platform::sdl::{decode_sdl_event, encode_sdl_event, SdlInjectedEvent};

    use super::super::startup_selection::StartupSelectionCollaborators;
    use super::*;
    use crate::options::GameFamily as OptionsFamily;
    use crate::options::Network;

    fn steel_corpus_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target")
    }

    fn windowed_options() -> ApplicationOptions {
        ApplicationOptions {
            windowed: true,
            width: 64,
            height: 64,
            frame_limit: Some(3),
            corpus_root: steel_corpus_root().to_string_lossy().into_owned(),
            ..ApplicationOptions::default()
        }
    }

    fn catalog() -> InstalledCatalog {
        InstalledCatalog::new(
            "/tmp/qa-windowed-test".to_string(),
            vec![CatalogProduct {
                id: ContentId("q2-classic-baseq2".to_string()),
                expectation: ProductExpectation {
                    id: "q2-classic-baseq2".to_string(),
                    family: GameFamily::Q2,
                    edition: "classic".to_string(),
                    campaign: "baseq2".to_string(),
                    title: "Quake II".to_string(),
                    content_directory: "baseq2".to_string(),
                    base_product: None,
                    required_content_archives: Vec::new(),
                    required_programs: Vec::new(),
                    map_witness: None,
                    unresolved_reason: None,
                },
                availability: ProductAvailability::Installed,
                archives: Vec::new(),
                loose_root: None,
                user_content: None,
                maps: Vec::new(),
                diagnostics: Vec::new(),
            }],
            Vec::new(),
            0,
            None,
        )
        .unwrap()
    }

    #[test]
    fn windowed_preset_collaborators_resolve_live_preset() {
        let catalog = catalog();
        let collaborators = WindowedPresetCollaborators;
        let products = collaborators
            .player_products(
                &catalog,
                "q2-classic-baseq2",
                GameFamily::Q2,
                None,
                GameFamily::Q2,
                &Network::Offline,
            )
            .unwrap();
        assert_eq!(products.movement, "q2-classic-baseq2");
        assert_eq!(products.character, "q2-classic-baseq2");
        assert!(collaborators.mod_choices(&catalog).unwrap().is_empty());
        assert!(collaborators
            .load_team_arena(&catalog, &ApplicationOptions::default())
            .unwrap()
            .is_none());
        let options = ApplicationOptions {
            movement: OptionsFamily::Q2,
            character: OptionsFamily::Q2,
            character_model: "male".to_string(),
            ..ApplicationOptions::default()
        };
        let movement = ProviderReference {
            provider: ProviderId::new("q2", "movement"),
            content: ContentId("q2-classic-baseq2".to_string()),
        };
        let character = ProviderReference {
            provider: ProviderId::new("q2", "character"),
            content: ContentId("q2-classic-baseq2".to_string()),
        };
        let preset = collaborators
            .application_preset(&catalog, &options, Some(&movement), Some(&character))
            .unwrap();
        assert_eq!(preset.map.geometry.path, "maps/base1.bsp");
    }

    #[test]
    fn timing_report_needs_enabled_timings() {
        let config = StartupConfig::from_options(&windowed_options()).unwrap();
        let mut backend = WindowedStartupBackend::new(
            &config,
            false,
            1.0,
            RenderBackendKind::Gl,
            Rc::new(Cell::new(false)),
            IdentityOwner::create("windowed-test").unwrap(),
        );
        assert!(backend.timing_report().is_none());
        assert!(backend.timing_totals_mut().is_none());
        backend.set_frame_timings(true);
        assert!(backend.timing_totals_mut().is_some());
        let report = backend.timing_report().expect("timed report");
        assert!(report.contains("over 0 frames"), "{report}");
    }

    #[test]
    fn windowed_rejects_worker_before_opening() {
        let mut options = windowed_options();
        options.render_worker = Some(true);
        let Err(error) = open_windowed_application(&options, StartupEntry::Run) else {
            panic!("expected failure");
        };
        assert!(error.contains("worker"), "{error}");
    }

    #[test]
    fn windowed_backend_before_open() {
        let config = StartupConfig::from_options(&windowed_options()).unwrap();
        let mut backend = WindowedStartupBackend::new(
            &config,
            false,
            1.0,
            RenderBackendKind::Gl,
            Rc::new(Cell::new(false)),
            IdentityOwner::create("windowed-test").unwrap(),
        );
        assert!(backend.read_pixels().is_err());
        assert!(backend.capture_next_frame().is_err());
        assert!(backend.poll().is_ok());
        assert!(!backend.has_active_game());
        assert!(backend.input_seat().is_none());
        let owner = IdentityOwner::create("windowed-test").unwrap();
        let event = SeatInputEvent::Focus {
            seat: owner.seat(0),
            time_ms: 0.0,
            focused: true,
        };
        assert!(!backend.input(&event));
        backend.request_quit();
        let _ = backend.now_ms();
        backend.sleep_ms(0.5);
        assert!(backend.close().is_empty());
    }

    fn test_input() -> (SeatId, WindowedInputQueue, InputRouter) {
        let owner = IdentityOwner::create("windowed-input-test").unwrap();
        let seat = owner.seat(0);
        let queue: WindowedInputQueue = Rc::new(RefCell::new(Vec::new()));
        let router = open_windowed_input(seat.clone(), Rc::clone(&queue), Instant::now()).unwrap();
        (seat, queue, router)
    }

    fn drain_queue(queue: &WindowedInputQueue) -> Vec<SeatInputEvent> {
        std::mem::take(&mut *queue.borrow_mut())
    }

    fn key_event(timestamp: u32, down: bool) -> SdlEvent {
        SdlEvent::Key {
            timestamp,
            down,
            repeat: false,
            scancode: 4,
            keycode: 65,
            modifiers: 0,
        }
    }

    #[test]
    fn windowed_key_events_map_through_router() {
        let (seat, queue, mut router) = test_input();
        router.handle_platform(key_event(100, true)).unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            SeatInputEvent::Key {
                code: 97,
                down: true,
                ..
            }
        ));
        assert_eq!(seat_event_seat(&events[0]), &seat);
        router.handle_platform(key_event(101, false)).unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            SeatInputEvent::Key {
                code: 97,
                down: false,
                ..
            }
        ));
        router
            .handle_platform(SdlEvent::Key {
                timestamp: 102,
                down: true,
                repeat: false,
                scancode: 9,
                keycode: 0,
                modifiers: 0,
            })
            .unwrap();
        assert!(drain_queue(&queue).is_empty());
    }

    #[test]
    fn windowed_text_and_pointer_events_map_through_router() {
        let (_seat, queue, mut router) = test_input();
        router
            .handle_platform(SdlEvent::Text {
                timestamp: 200,
                text: "hi".to_string(),
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        let SeatInputEvent::Text { text, .. } = &events[0] else {
            panic!("expected text, got {:?}", events[0]);
        };
        assert_eq!(text, "hi");
        router
            .handle_platform(SdlEvent::MouseMotion {
                timestamp: 201,
                buttons: 0,
                x: 10,
                y: 20,
                dx: 3,
                dy: -2,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        let SeatInputEvent::MouseMotion { position, delta, .. } = &events[0] else {
            panic!("expected motion, got {:?}", events[0]);
        };
        // No window is attached in the unit harness, so positions pass through raw.
        assert_eq!(*position, (10.0, 20.0));
        assert_eq!(*delta, (3.0, -2.0));
        router
            .handle_platform(SdlEvent::MouseButton {
                timestamp: 202,
                down: true,
                button: 1,
                clicks: 1,
                x: 5,
                y: 6,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            SeatInputEvent::MouseButton {
                button: 1,
                down: true,
                ..
            }
        ));
        router
            .handle_platform(SdlEvent::MouseWheel {
                timestamp: 203,
                x: 0,
                y: 1,
                precise_x: 0.0,
                precise_y: 1.0,
                flipped: false,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        let SeatInputEvent::MouseWheel { delta, .. } = &events[0] else {
            panic!("expected wheel, got {:?}", events[0]);
        };
        assert_eq!(*delta, (0.0, 1.0));
        router
            .handle_platform(SdlEvent::MouseWheel {
                timestamp: 204,
                x: 0,
                y: 1,
                precise_x: 0.0,
                precise_y: 1.0,
                flipped: true,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        let SeatInputEvent::MouseWheel { delta, .. } = &events[0] else {
            panic!("expected wheel, got {:?}", events[0]);
        };
        assert_eq!(*delta, (0.0, -1.0));
    }

    #[test]
    fn windowed_focus_events_gate_delivery() {
        let (_seat, queue, mut router) = test_input();
        router
            .handle_platform(SdlEvent::Window {
                timestamp: 300,
                event: WINDOW_FOCUS_LOST,
                data1: 0,
                data2: 0,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SeatInputEvent::Focus { focused: false, .. }));
        router.handle_platform(key_event(301, true)).unwrap();
        assert!(drain_queue(&queue).is_empty());
        router
            .handle_platform(SdlEvent::Window {
                timestamp: 302,
                event: WINDOW_FOCUS_GAINED,
                data1: 0,
                data2: 0,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], SeatInputEvent::Focus { focused: true, .. }));
        router.handle_platform(key_event(303, true)).unwrap();
        assert_eq!(drain_queue(&queue).len(), 1);
    }

    #[test]
    fn windowed_quit_events_stay_outside_seat_delivery() {
        let (_seat, queue, mut router) = test_input();
        router.handle_platform(SdlEvent::Quit { timestamp: 400 }).unwrap();
        router
            .handle_platform(SdlEvent::Window {
                timestamp: 401,
                event: SDL_WINDOWEVENT_CLOSE,
                data1: 0,
                data2: 0,
            })
            .unwrap();
        assert!(drain_queue(&queue).is_empty());
    }

    #[test]
    fn windowed_decode_chain_maps_injected_keys() {
        let (_seat, queue, mut router) = test_input();
        let injected = SdlInjectedEvent::Key {
            timestamp: 500,
            down: true,
            repeat: false,
            scancode: 4,
            keycode: 65,
            modifiers: 0,
        };
        let bytes = encode_sdl_event(&injected, 7).unwrap();
        let decoded = decode_sdl_event(&bytes).unwrap();
        assert!(matches!(
            decoded,
            SdlEvent::Key {
                keycode: 65,
                down: true,
                ..
            }
        ));
        router.handle_platform(decoded).unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0],
            SeatInputEvent::Key {
                code: 97,
                down: true,
                ..
            }
        ));
    }

    #[test]
    fn windowed_backend_delivers_router_events_to_input() {
        let config = StartupConfig::from_options(&windowed_options()).unwrap();
        let quit = Rc::new(Cell::new(false));
        let mut backend = WindowedStartupBackend::new(
            &config,
            false,
            1.0,
            RenderBackendKind::Gl,
            Rc::clone(&quit),
            IdentityOwner::create("windowed-test").unwrap(),
        );
        let owner = IdentityOwner::create("windowed-delivery-test").unwrap();
        let seat = owner.seat(0);
        let queue: WindowedInputQueue = Rc::new(RefCell::new(Vec::new()));
        let router = open_windowed_input(seat.clone(), Rc::clone(&queue), Instant::now()).unwrap();
        backend.input_queue = Rc::clone(&queue);
        backend.input_router = Some(router);
        backend.input_seat = Some(seat.clone());
        backend
            .handle_window_events(vec![key_event(600, true), SdlEvent::Quit { timestamp: 601 }])
            .unwrap();
        assert!(quit.get());
        assert_eq!(backend.input_log.len(), 1);
        assert!(matches!(
            backend.input_log[0],
            SeatInputEvent::Key {
                code: 97,
                down: true,
                ..
            }
        ));
        assert_eq!(backend.input_seat(), Some(seat.clone()));
        let foreign = IdentityOwner::create("windowed-foreign-test").unwrap().seat(0);
        let rejected = SeatInputEvent::Key {
            seat: foreign,
            time_ms: 0.0,
            code: 97,
            down: true,
        };
        assert!(!backend.input(&rejected));
        assert_eq!(backend.input_log.len(), 1);
    }

    /// wu-16: SDL menu keys map to the Quake codes the menu controller
    /// navigates on (Up/Down/Enter/Escape).
    #[test]
    fn windowed_menu_keys_map_to_quake_codes() {
        use qa_client::input::KeyCode;

        let (_seat, queue, mut router) = test_input();
        for (scancode, keycode, expected) in [
            (81, 0x4000_0000 | 81, KeyCode::Down as i32),
            (82, 0x4000_0000 | 82, KeyCode::Up as i32),
            (40, 13, KeyCode::Enter as i32),
            (41, 27, KeyCode::Escape as i32),
        ] {
            router
                .handle_platform(SdlEvent::Key {
                    timestamp: 700,
                    down: true,
                    repeat: false,
                    scancode,
                    keycode,
                    modifiers: 0,
                })
                .unwrap();
            let events = drain_queue(&queue);
            assert_eq!(events.len(), 1, "scancode {scancode} maps");
            assert!(
                matches!(events[0], SeatInputEvent::Key { code, down: true, .. } if code == expected),
                "scancode {scancode} maps to {expected}, got {:?}",
                events[0]
            );
        }
    }

    /// wu-16: assigned gamepad buttons and sticks reach the seat queue as
    /// controller seat events (the menu controller maps them to
    /// navigation and activation).
    #[test]
    fn windowed_controller_buttons_map_through_router() {
        use qa_client::input::ControllerAxis;

        let (seat, queue, mut router) = test_input();
        router
            .handle_controller(ControllerEvent::Assignment {
                timestamp: 800,
                slot: 0,
                previous: None,
                instance: Some(7),
            })
            .unwrap();
        assert!(drain_queue(&queue).is_empty());
        router
            .handle_controller(ControllerEvent::Button {
                timestamp: 801,
                instance: 7,
                slot: Some(0),
                button: 12,
                down: true,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        assert!(
            matches!(
                events[0],
                SeatInputEvent::ControllerButton {
                    device: 7,
                    button: 12,
                    down: true,
                    ..
                }
            ),
            "got {:?}",
            events[0]
        );
        assert_eq!(seat_event_seat(&events[0]), &seat);
        router
            .handle_controller(ControllerEvent::Axis {
                timestamp: 802,
                instance: 7,
                slot: Some(0),
                axis: 1,
                value: i16::MAX,
            })
            .unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1);
        let SeatInputEvent::ControllerAxis {
            axis, value, device, ..
        } = &events[0]
        else {
            panic!("expected axis, got {:?}", events[0]);
        };
        assert_eq!(*device, 7);
        assert_eq!(*axis, ControllerAxis::LeftY);
        assert!(*value > 0.9, "stick fully deflected, got {value}");
    }

    /// wu-16: the test-only pumps deliver through backend input exactly
    /// like `poll` would, without a window.
    #[test]
    fn windowed_test_pumps_deliver_to_input() {
        use qa_client::input::KeyCode;

        let config = StartupConfig::from_options(&windowed_options()).unwrap();
        let mut backend = WindowedStartupBackend::new(
            &config,
            false,
            1.0,
            RenderBackendKind::Gl,
            Rc::new(Cell::new(false)),
            IdentityOwner::create("windowed-test").unwrap(),
        );
        assert!(!backend.menu_open());
        assert_eq!(backend.menu_active_menu(), None);
        assert_eq!(backend.menu_focus_control(), None);
        let owner = IdentityOwner::create("windowed-pump-test").unwrap();
        let seat = owner.seat(0);
        let queue: WindowedInputQueue = Rc::new(RefCell::new(Vec::new()));
        let router = open_windowed_input(seat.clone(), Rc::clone(&queue), Instant::now()).unwrap();
        backend.input_queue = Rc::clone(&queue);
        backend.input_router = Some(router);
        backend.input_seat = Some(seat);
        backend
            .inject_platform_event(SdlEvent::Key {
                timestamp: 900,
                down: true,
                repeat: false,
                scancode: 81,
                keycode: 0x4000_0000 | 81,
                modifiers: 0,
            })
            .unwrap();
        assert_eq!(backend.input_log.len(), 1);
        assert!(matches!(
            backend.input_log[0],
            SeatInputEvent::Key { code, down: true, .. } if code == KeyCode::Down as i32
        ));
        backend
            .inject_controller_event(ControllerEvent::Assignment {
                timestamp: 901,
                slot: 0,
                previous: None,
                instance: Some(7),
            })
            .unwrap();
        backend
            .inject_controller_event(ControllerEvent::Button {
                timestamp: 902,
                instance: 7,
                slot: Some(0),
                button: 0,
                down: true,
            })
            .unwrap();
        assert_eq!(backend.input_log.len(), 2);
        assert!(matches!(
            backend.input_log[1],
            SeatInputEvent::ControllerButton {
                device: 7,
                button: 0,
                down: true,
                ..
            }
        ));
    }

    #[test]
    fn windowed_audio_falls_back_to_muted() {
        assert!(open_windowed_audio_on(None, true).is_none());
        assert!(open_windowed_audio_on(Some("quake-anthology-no-such-output-device"), false).is_none());
    }

    #[test]
    fn windowed_audio_refresh_never_fails() {
        let mut muted: Option<UnifiedAudio> = None;
        refresh_windowed_audio(&mut muted, 4.0);
        refresh_windowed_audio(&mut muted, f64::NAN);
        assert!(muted.is_none());
        if let Some(audio) = open_windowed_audio() {
            let mut live = Some(audio);
            for _ in 0..5 {
                refresh_windowed_audio(&mut live, 4.0);
            }
            assert!(live.is_some());
        }
    }

    #[test]
    fn windowed_backend_audio_defaults_to_muted_and_closes_cleanly() {
        let config = StartupConfig::from_options(&windowed_options()).unwrap();
        let mut backend = WindowedStartupBackend::new(
            &config,
            false,
            1.0,
            RenderBackendKind::Gl,
            Rc::new(Cell::new(false)),
            IdentityOwner::create("windowed-test").unwrap(),
        );
        assert!(backend.audio.is_none());
        assert!(backend.close().is_empty());
    }

    #[test]
    fn live_windowed_smoke_runs_frames() {
        // Passes with or without a display: a real windowed run is exercised
        // when GL is available, otherwise the honest open failure is required.
        let _gl_guard = super::WINDOWED_GL_TEST_LOCK.lock().unwrap();
        let options = windowed_options();
        match open_windowed_application(&options, StartupEntry::Run) {
            Ok(composed) => {
                let mut composed = composed;
                assert!(composed.app.input_seat().is_some(), "windowed input seat is published");
                let seat = composed.app.input_seat().unwrap();
                let probe = SeatInputEvent::Key {
                    seat,
                    time_ms: 0.0,
                    code: 97,
                    down: true,
                };
                assert!(composed.app.input(&probe), "windowed backend accepts seat input");
                let pixels = composed.app.capture_next_frame().expect("windowed capture works");
                assert_eq!(pixels.len(), 64 * 64 * 4);
                if composed.app.active_game() {
                    // Capture-diff proof: a presented world must differ from
                    // pure black programmatically (no eyeballing).
                    let lit = count_non_black(&pixels);
                    assert!(
                        lit > 64,
                        "expected a presented world, got {lit} non-black pixels of {}",
                        64 * 64
                    );
                } else {
                    for pixel in pixels.as_chunks::<4>().0.iter().step_by(1024) {
                        assert_eq!(pixel[0], 255, "red channel");
                        assert_eq!(pixel[1], 0, "green channel");
                        assert!(pixel[2] == 127 || pixel[2] == 128, "blue channel: {}", pixel[2]);
                        assert_eq!(pixel[3], 255, "alpha channel");
                    }
                }
                let frames = drive_windowed_application(&mut composed.app, &composed.quit, Some(3))
                    .expect("windowed drive works");
                assert_eq!(frames, 3);
                assert!(composed.app.is_closed());
                windowed_sdl_router_attach_pumps_keys();
            }
            Err(error) => assert!(!error.is_empty(), "honest open failure"),
        }
    }

    #[test]
    fn run_pairs_local_player_with_admitted_body() {
        // One local player concept: the driving seat plus the admitted
        // body's own simulation actor, paired exactly when a world with
        // a body is set. Without a display the honest open failure is
        // required instead.
        let _gl_guard = super::WINDOWED_GL_TEST_LOCK.lock().unwrap();
        let options = windowed_options();
        match open_windowed_application(&options, StartupEntry::Run) {
            Ok(composed) => {
                let composed = composed;
                let seat = composed.app.input_seat().expect("run publishes a seat");
                match composed
                    .app
                    .backend()
                    .world
                    .as_ref()
                    .and_then(|world| world.player_actor().cloned())
                {
                    Some(actor) => {
                        let local = composed
                            .app
                            .backend()
                            .local_player()
                            .expect("body pairs a local player");
                        assert_eq!(local.seat_id(), &seat, "paired seat drives");
                        assert_eq!(local.actor, actor, "paired actor is the body");
                    }
                    None => assert!(
                        composed.app.backend().local_player().is_none(),
                        "no body pairs no local player"
                    ),
                }
            }
            Err(error) => assert!(!error.is_empty(), "honest open failure"),
        }
    }

    #[test]
    fn live_windowed_cpu_smoke_runs_frames() {
        // CPU composition through a real SDL software window: open, capture
        // one software frame, then drive to the frame limit. Without a
        // display the honest open failure is required instead.
        let _gl_guard = super::WINDOWED_GL_TEST_LOCK.lock().unwrap();
        let mut options = windowed_options();
        options.renderer = Renderer::Cpu;
        match open_windowed_application(&options, StartupEntry::Run) {
            Ok(composed) => {
                let mut composed = composed;
                let pixels = composed.app.capture_next_frame().expect("windowed CPU capture works");
                assert_eq!(pixels.len(), 64 * 64 * 4);
                if composed.app.active_game() {
                    let lit = count_non_black(&pixels);
                    assert!(
                        lit > 64,
                        "expected a presented world, got {lit} non-black pixels of {}",
                        64 * 64
                    );
                }
                let frames = drive_windowed_application(&mut composed.app, &composed.quit, Some(3))
                    .expect("windowed CPU drive works");
                assert_eq!(frames, 3);
                assert!(composed.app.is_closed());
            }
            Err(error) => assert!(!error.is_empty(), "honest open failure"),
        }
    }

    #[test]
    fn non_black_counter_diffs_captures_from_black() {
        assert_eq!(count_non_black(&[0, 0, 0, 255, 0, 0, 0, 255]), 0);
        assert_eq!(count_non_black(&[0, 0, 0, 255, 1, 0, 0, 255, 0, 0, 5, 0]), 2);
        assert_eq!(count_non_black(&[]), 0);
    }

    #[test]
    fn windowed_camera_tracks_live_size() {
        let camera = windowed_camera(64, 48).expect("valid camera");
        assert_eq!((camera.viewport.width, camera.viewport.height), (64, 48));
        assert!(matches!(camera.clip, CameraClip::None));
        assert!(windowed_camera(0, 64).is_none());
        assert!(windowed_camera(64, 0).is_none());
        assert!(windowed_camera(-1, 64).is_none());
    }

    fn test_batch() -> DrawBatch {
        use qa_client::render::types::{
            BatchLighting, BatchPrimitive, BatchVertices, CullFace, RenderState, RenderVertex, TextureBinding,
        };
        use qa_core::math::vec2;
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1, 2],
            texture: TextureBinding::RetainCurrentTexture,
            state: RenderState::opaque(CullFace::None),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![
                RenderVertex {
                    position: vec4(0.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderVertex {
                    position: vec4(1.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(1.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderVertex {
                    position: vec4(0.0, 1.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 1.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
            ]),
        }
    }

    #[test]
    fn windowed_commands_degrade_without_scene() {
        let (commands, image_operations) = build_windowed_commands(64, 64, None, 12.0, None, None);
        assert!(image_operations.is_empty());
        assert_eq!(commands.len(), 2);
        assert!(matches!(
            commands[0],
            RenderCommand::DrawBuffer {
                buffer: DrawBuffer::Back,
                clear: true
            }
        ));
        assert!(matches!(commands[1], RenderCommand::SwapBuffers));
    }

    #[test]
    fn windowed_commands_submit_scene_view_with_swap() {
        let owner = IdentityOwner::create("windowed-scene-test").unwrap();
        let seat = owner.seat(0);
        let mut scene = WindowedScene::new(vec![test_batch()], vec4(0.0, 0.0, 0.0, 1.0));
        let (commands, image_operations) = build_windowed_commands(64, 48, Some(&seat), 33.0, Some(&mut scene), None);
        assert!(image_operations.is_empty());
        assert_eq!(commands.len(), 3);
        assert!(matches!(
            commands[0],
            RenderCommand::DrawBuffer {
                buffer: DrawBuffer::Back,
                clear: false
            }
        ));
        let RenderCommand::View(view) = &commands[1] else {
            panic!("expected view, got {:?}", commands[1]);
        };
        assert_eq!(view.state.viewport.width, 64.0);
        assert_eq!(view.state.viewport.height, 48.0);
        assert!(matches!(view.target, ViewTarget::Seat(_)));
        assert_eq!(view.time, SourceTime::Milliseconds(33.0));
        assert_eq!(view.operations.len(), 1);
        assert!(matches!(
            view.operations[0],
            RenderOperation::Draw(ref batches) if batches.len() == 1
        ));
        assert!(matches!(commands[2], RenderCommand::SwapBuffers));
    }

    #[test]
    fn windowed_scene_view_uses_preview_without_seat_and_live_size() {
        let mut scene = WindowedScene::new(Vec::new(), vec4(0.1, 0.2, 0.3, 1.0));
        let (view, image_operations) = windowed_scene_view(128, 96, None, 7.0, &mut scene, None).expect("preview view");
        assert!(image_operations.is_empty());
        assert_eq!((view.state.viewport.width, view.state.viewport.height), (128.0, 96.0));
        assert!(matches!(view.target, ViewTarget::Preview(_)));
        assert!(view.operations.is_empty());
        let clear = view.state.clear.expect("view clears");
        assert_eq!(clear.depth, 1.0);
        assert_eq!(clear.color, Some(vec4(0.1, 0.2, 0.3, 1.0)));
        let (other, _) = windowed_scene_view(32, 32, None, 7.0, &mut scene, None).expect("other size");
        assert_eq!((other.state.viewport.width, other.state.viewport.height), (32.0, 32.0));
    }

    #[test]
    fn windowed_invalid_size_degrades_even_with_scene() {
        let mut scene = WindowedScene::new(vec![test_batch()], vec4(0.0, 0.0, 0.0, 1.0));
        assert!(windowed_scene_view(0, 64, None, 0.0, &mut scene, None).is_none());
        let (commands, _) = build_windowed_commands(0, 64, None, 0.0, Some(&mut scene), None);
        assert_eq!(commands.len(), 2);
        assert!(matches!(commands[0], RenderCommand::DrawBuffer { clear: true, .. }));
        assert!(matches!(commands[1], RenderCommand::SwapBuffers));
    }

    #[test]
    fn windowed_backend_scene_tracks_active_game() {
        let config = StartupConfig::from_options(&windowed_options()).unwrap();
        let mut backend = WindowedStartupBackend::new(
            &config,
            false,
            1.0,
            RenderBackendKind::Gl,
            Rc::new(Cell::new(false)),
            IdentityOwner::create("windowed-test").unwrap(),
        );
        assert!(!backend.has_active_game());
        backend.scene = Some(WindowedScene::new(Vec::new(), vec4(0.0, 0.0, 0.0, 1.0)));
        assert!(backend.has_active_game());
        assert!(backend.close().is_empty());
        assert!(!backend.has_active_game());
    }

    #[test]
    fn windowed_gl_backend_serial_commands_degrade_without_parts() {
        let mut backend = NativeGlBackend {
            parts: None,
            width: 64,
            height: 64,
            driver: None,
            worker_interval: 1,
            color: vec4(1.0, 1.0, 1.0, 1.0),
        };
        backend.execute_serial_command(&RenderCommand::SetColor {
            color: vec4(0.5, 0.5, 0.5, 1.0),
        });
        assert_eq!(backend.color, vec4(0.5, 0.5, 0.5, 1.0));
        backend.execute_serial_command(&RenderCommand::DrawBuffer {
            buffer: DrawBuffer::Back,
            clear: true,
        });
        let mut scene = WindowedScene::new(Vec::new(), vec4(0.0, 0.0, 0.0, 1.0));
        let (view, _) = windowed_scene_view(64, 64, None, 0.0, &mut scene, None).unwrap();
        backend.execute_serial_command(&RenderCommand::View(view));
        backend.execute_serial_command(&RenderCommand::Draw);
        backend.execute_serial_command(&RenderCommand::SwapBuffers);
    }

    /// Full-window NDC triangle over one texture binding, for the
    /// viewport-fill capture proof below.
    fn fullscreen_batch(texture: qa_client::render::types::TextureBinding) -> DrawBatch {
        use qa_client::render::types::{
            BatchLighting, BatchPrimitive, BatchVertices, CullFace, RenderState, RenderVertex,
        };
        use qa_core::math::vec2;
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1, 2],
            texture,
            state: RenderState::opaque(CullFace::None),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![
                RenderVertex {
                    position: vec4(-1.0, -1.0, 0.5, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderVertex {
                    position: vec4(3.0, -1.0, 0.5, 1.0),
                    tex_coord: vec2(1.0, 0.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderVertex {
                    position: vec4(-1.0, 3.0, 0.5, 1.0),
                    tex_coord: vec2(0.0, 1.0),
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
            ]),
        }
    }

    /// Non-black bounding box plus lit count over RGBA capture pixels.
    fn capture_bbox(pixels: &[u8], width: u32, height: u32) -> (u32, u32, u32, u32, usize) {
        let (mut minx, mut miny, mut maxx, mut maxy) = (width, height, 0u32, 0u32);
        let mut count = 0usize;
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                if pixels[i] != 0 || pixels[i + 1] != 0 || pixels[i + 2] != 0 {
                    count += 1;
                    minx = minx.min(x);
                    miny = miny.min(y);
                    maxx = maxx.max(x);
                    maxy = maxy.max(y);
                }
            }
        }
        (minx, miny, maxx, maxy, count)
    }

    #[test]
    fn windowed_capture_fills_drawable() {
        // Viewport-fill proof: a full-window NDC triangle captured through
        // the real GL backend must light every drawable pixel. Passes with
        // or without a display: without GL the honest open failure skips.
        let mut options = windowed_options();
        options.width = 160;
        options.height = 120;
        let config = StartupConfig::from_options(&options).unwrap();
        let mut backend = WindowedStartupBackend::new(
            &config,
            true,
            1.0,
            RenderBackendKind::Gl,
            Rc::new(Cell::new(false)),
            IdentityOwner::create("windowed-fill-test").unwrap(),
        );
        if backend.open().is_err() {
            return;
        }
        // Upload a white 1x1 so the triangle's vertex colors survive texturing.
        use qa_client::render::types::{ImageLevel, RenderImage, TextureFilter, TextureSampling};
        let owner = ResourceOwner::new(7, backend.identity.session().clone(), 0);
        let mut registry = SceneImageRegistry::new(owner);
        let white = registry
            .register(
                "*fillwhite",
                RenderImage::Rgba8 {
                    levels: vec![ImageLevel {
                        width: 1,
                        height: 1,
                        pixels: vec![255, 255, 255, 255],
                    }],
                    border_color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                TextureSampling {
                    repeat: true,
                    filter: TextureFilter::Nearest,
                },
            )
            .expect("white registers");
        let ops = registry.drain_operations();
        backend
            .renderer
            .as_mut()
            .expect("open renderer")
            .backend_mut()
            .apply_image_resource(&ops[0]);
        backend.scene = Some(WindowedScene::new(
            vec![fullscreen_batch(qa_client::render::types::TextureBinding::BindImage(
                white,
            ))],
            vec4(0.0, 0.0, 0.0, 1.0),
        ));
        let pixels = backend.capture_next_frame().expect("capture works");
        assert_eq!(pixels.len(), 160 * 120 * 4);
        let (minx, miny, maxx, maxy, count) = capture_bbox(&pixels, 160, 120);
        assert_eq!((minx, miny, maxx, maxy), (0, 0, 159, 119), "capture fills the drawable");
        assert_eq!(count, 160 * 120, "every captured pixel is lit");
        assert!(backend.close().is_empty());
    }

    /// Live `SdlRouterWindow::attach` protocol check: push an injected key,
    /// attach, pump, and assert the decoded seat event arrives. Runs after the
    /// smoke application closes so the SDL input lease is free.
    fn windowed_sdl_router_attach_pumps_keys() {
        let options = SdlWindowOptions {
            title: "quake-anthology-input-check".to_string(),
            width: 32,
            height: 32,
            hidden: true,
            ..SdlWindowOptions::default()
        };
        let mut scratch = SdlWindow::open(&options).expect("scratch window opens");
        scratch
            .push_event(&SdlInjectedEvent::Key {
                timestamp: 7,
                down: true,
                repeat: false,
                scancode: 4,
                keycode: 65,
                modifiers: 0,
            })
            .expect("scratch key injects");
        let attached = SdlRouterWindow::attach(scratch).expect("router window attaches");
        let owner = IdentityOwner::create("windowed-attach-test").unwrap();
        let seat = owner.seat(0);
        let queue: WindowedInputQueue = Rc::new(RefCell::new(Vec::new()));
        let mut router = open_windowed_input(seat, Rc::clone(&queue), Instant::now()).unwrap();
        router.attach_window(Box::new(attached)).unwrap();
        router.pump().unwrap();
        let events = drain_queue(&queue);
        assert_eq!(events.len(), 1, "pumped key arrives: {events:?}");
        assert!(matches!(
            events[0],
            SeatInputEvent::Key {
                code: 97,
                down: true,
                ..
            }
        ));
    }

    #[test]
    fn host_gate_skips_early_sim_frames_per_dialect() {
        use qa_core::cmd::Dialect;

        let mut gate = HostFrameGate::new();
        // First display frame at 120Hz: too early for NetQuake, and
        // the skip leaves oldrealtime behind.
        gate.advance(1.0 / 120.0);
        assert!(!gate.sim_due(Dialect::Q1Netquake));
        assert_eq!(gate.oldrealtime_s, 0.0);
        // After a full 72Hz period the sim runs and oldrealtime
        // catches up exactly (`host.c:508-509`).
        gate.advance(1.0 / 120.0);
        assert!(gate.sim_due(Dialect::Q1Netquake));
        assert_eq!(gate.oldrealtime_s, gate.realtime_s);

        // QuakeWorld at stock defaults paces at 31.25Hz.
        let mut gate = HostFrameGate::new();
        gate.advance(0.020);
        assert!(!gate.sim_due(Dialect::Q1Quakeworld));
        gate.advance(0.020);
        assert!(gate.sim_due(Dialect::Q1Quakeworld));

        // Timedemo runs every frame.
        let mut gate = HostFrameGate::new();
        gate.timedemo = true;
        gate.advance(0.001);
        assert!(gate.sim_due(Dialect::Q1Netquake));
        assert!(gate.sim_due(Dialect::Q1Quakeworld));

        // Other dialects always run.
        let mut gate = HostFrameGate::new();
        gate.advance(0.001);
        assert!(gate.sim_due(Dialect::Q2Classic));
    }

    #[test]
    fn host_gate_clamps_hitch_frametimes_per_dialect() {
        use qa_core::cmd::Dialect;

        // A 5s display hitch must not reach Q1 physics: NetQuake caps
        // at 100ms (`host.c`), QuakeWorld at 200ms (`cl_main.c:1328`).
        let mut gate = HostFrameGate::new();
        gate.advance(5.0);
        assert!(gate.sim_due(Dialect::Q1Netquake));
        assert_eq!(gate.sim_frame_ms(Dialect::Q1Netquake), 100.0);

        let mut gate = HostFrameGate::new();
        gate.advance(5.0);
        assert!(gate.sim_due(Dialect::Q1Quakeworld));
        assert_eq!(gate.sim_frame_ms(Dialect::Q1Quakeworld), 200.0);

        // NetQuake floors tiny spans at 1ms (`host.c`).
        let mut gate = HostFrameGate::new();
        gate.timedemo = true;
        gate.advance(0.0001);
        assert!(gate.sim_due(Dialect::Q1Netquake));
        assert_eq!(gate.sim_frame_ms(Dialect::Q1Netquake), 1.0);

        // NetQuake `host_framerate` overrides the measured span.
        let mut gate = HostFrameGate::new();
        gate.nq_host_framerate = 0.05;
        gate.advance(5.0);
        assert!(gate.sim_due(Dialect::Q1Netquake));
        assert_eq!(gate.sim_frame_ms(Dialect::Q1Netquake), 50.0);
    }

    #[test]
    fn host_gate_sync_reads_live_qw_throttle_cvars() {
        use qa_core::cmd::Dialect;
        use qa_core::cvar::CvarRegistry;

        // Stock QW default (`cl_maxfps` 0, `rate` 2500) paces at
        // 31.25Hz: 20ms after the epoch the sim is not due.
        let mut cvars = CvarRegistry::new(Dialect::Q1Quakeworld);
        register_frame_time_cvars(&mut cvars).expect("register");
        cvars.register("rate", "2500", 0).expect("rate");
        let mut gate = HostFrameGate::new();
        gate.sync(&cvars);
        gate.advance(0.020);
        assert!(!gate.sim_due(Dialect::Q1Quakeworld));

        // A player-set `cl_maxfps 72` on the same clock makes the
        // same frame due: the throttle follows the live registry.
        cvars.set("cl_maxfps", "72", true).expect("set maxfps");
        gate.sync(&cvars);
        assert_eq!(gate.qw_maxfps, 72.0);
        assert!(gate.sim_due(Dialect::Q1Quakeworld));

        // `sync` never touches the loop-fed timedemo flag.
        gate.timedemo = true;
        gate.sync(&cvars);
        assert!(gate.timedemo);
    }

    #[test]
    fn host_gate_sync_reads_live_nq_host_framerate() {
        use qa_core::cmd::Dialect;
        use qa_core::cvar::CvarRegistry;

        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_frame_time_cvars(&mut cvars).expect("register");
        let mut gate = HostFrameGate::new();
        gate.sync(&cvars);
        assert_eq!(gate.nq_host_framerate, 0.0);
        cvars.set("host_framerate", "0.05", true).expect("set framerate");
        gate.sync(&cvars);
        gate.advance(5.0);
        assert!(gate.sim_due(Dialect::Q1Netquake));
        // The cvar read is f32 (`variable_value`), so compare against
        // the f32 spelling of 0.05s in milliseconds.
        assert_eq!(gate.sim_frame_ms(Dialect::Q1Netquake), f64::from(0.05f32) * 1000.0);
    }
}
