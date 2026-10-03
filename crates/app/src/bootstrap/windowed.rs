//! Windowed game path: native SDL window, GL renderer, and startup driver.
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
use qa_client::input::router::{InputRouter, RouterError, RouterWindow, Seat, SeatInputEvent, SeatRoute, UiCallback};
use qa_client::render::dynamic_texture::resolve_draw_textures;
use qa_client::render::gl::platform::PlatformGlContext;
use qa_client::render::gl::renderer::GlRenderer;
use qa_client::render::gl::{GlContext, DEPTH_BITS, MAX_TEXTURE_COORDS, MAX_TEXTURE_IMAGE_UNITS, MAX_TEXTURE_SIZE};
use qa_client::render::scene::resources::SceneImageRegistry;
use qa_client::render::types::{
    DrawBatch, DrawBuffer, ImageResourceOperation, OrderedBackend, Rect as ClientRect, RenderOperation,
    RenderView as ClientRenderView, RenderViewState, ResourceOwner, SourceTime, ViewClear, ViewTarget,
};
use qa_client::view::{perspective_projection, CameraClip, Rect as ViewRect, SceneCamera};
use qa_content::catalog::{
    discover_installed_content, BehaviorMounts, CatalogError, DiscoverContentOptions, InstalledCatalog, LaunchPreset,
    LaunchQvmCompatibility, LaunchWeaponSources, QvmCompatRole,
};
use qa_content::contract::{
    ContentDigest, ExecutableRecipe, GameFamily, ModDescription, ModSelection, ProviderReference, ProviderTiming,
    QvmAbiProfile, ResourceRequest,
};
use qa_content::mounts::MountPreparationScope;
use qa_core::cmd::Dialect;
use qa_core::identity::{IdentityOwner, ProviderId, SeatId};
use qa_core::math::{angles_to_axis, vec3, vec4, Vec4};
use qa_platform::controller::ControllerSelection;
use qa_platform::native_libraries::NativeLibraryOptions;
use qa_platform::sdl::{
    SdlBackend, SdlDisplayMode, SdlEvent, SdlGlOptions, SdlInputLease, SdlWindow, SdlWindowOptions,
    SdlWindowPresentation,
};
use qa_platform::sdl_render_context::SdlWorkerRenderContext;

use super::input::NullRegistry;
use super::renderer::{
    CaptureId, ImageLevel, NativeBackendFactory, NativeRenderBackend, NativeRenderWindow, NativeRenderer,
    NativeRendererOptions, NativeWindowFactory, RenderBackendKind, RenderCommand, RenderDriverInfo, RenderFrame,
    RenderGlConfig, RenderImageRegistry, RenderWindowOptions, RenderWindowPresentation, RendererResourceOwner,
};
use super::startup::{StartupApplication, StartupBackend, StartupEntry, StartupError, StartupFrame};
use super::startup_selection::{
    PreparedQ3Catalog, PreparedTeamArena, QvmGrappleStyle, StartupArenaSelection, StartupPlayerProducts,
    StartupSelectionCollaborators, StartupSelectionModel,
};
use super::windowed_menu::WindowedMenu;
use super::windowed_scene::WindowedPresentation;
use super::windowed_world::{load_windowed_world, WindowedWorld};
use crate::options::{ApplicationOptions, Network, Renderer};
use crate::startup::StartupConfig;

/// SDL window-event id for close (donor `event.event === 14` quit check).
const SDL_WINDOWEVENT_CLOSE: u8 = 14;

/// Native renderer over the windowed adapters.
type WindowedRenderer = NativeRenderer<NativeSdlWindow, NativeGlBackend, NativeImages>;

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

    fn present_pixels(&mut self, _pixels: &[u8]) -> Result<(), String> {
        Err("present requires a CPU window".to_string())
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
        if options.backend != RenderBackendKind::Gl {
            return Err("windowed CPU composition is not implemented".to_string());
        }
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
            backend: SdlBackend::Gl(SdlGlOptions {
                stereo: false,
                stencil_bits: 0,
                color_bits: 0.0,
                depth_bits: 0.0,
                driver: None,
                allow_software_gl: true,
            }),
        };
        let window = SdlWindow::open(&window_options).map_err(|error| error.to_string())?;
        let presentation = window.capture_presentation().map_err(|error| error.to_string())?;
        let shared = Rc::new(RefCell::new(window));
        self.share.borrow_mut().window = Some(Rc::clone(&shared));
        Ok(NativeSdlWindow {
            window: shared,
            share: Rc::clone(&self.share),
            kind: RenderBackendKind::Gl,
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

/// [`NativeBackendFactory`] adopting GL and building [`GlRenderer`] (donor
/// `openBackend`).
struct NativeGlBackendFactory {
    share: Rc<RefCell<WindowedShare>>,
    owner: ResourceOwner,
}

impl NativeBackendFactory for NativeGlBackendFactory {
    type Backend = NativeGlBackend;
    type Error = String;

    fn open_backend(
        &mut self,
        kind: RenderBackendKind,
        width: i32,
        height: i32,
        gamma: f64,
        worker: bool,
    ) -> Result<NativeGlBackend, String> {
        if kind != RenderBackendKind::Gl {
            return Err("windowed CPU composition is not implemented".to_string());
        }
        if worker {
            return Err("windowed worker rendering is not implemented".to_string());
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
        Ok(backend)
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
/// [`StartupBackend::input`](super::startup::StartupBackend::input).
fn open_windowed_input(seat: SeatId, queue: WindowedInputQueue, start: Instant) -> Result<InputRouter, String> {
    let tap = Rc::clone(&queue);
    let ui_event: UiCallback = Box::new(move |event, _focus| {
        tap.borrow_mut().push(event.clone());
        false
    });
    let routes = vec![SeatRoute {
        seat: Seat::new(seat.clone(), Dialect::Q2Classic, ui_event),
        controller: ControllerSelection::None,
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

/// Launch weapon sources for the windowed smoke run (never invoked: the
/// smoke never resolves content).
struct WindowedWeaponSources;

impl LaunchWeaponSources for WindowedWeaponSources {
    fn canonical_weapon_source(
        &self,
        _map: &ProviderReference,
        weapon: &ProviderReference,
        _catalog: &InstalledCatalog,
    ) -> Result<ProviderReference, CatalogError> {
        Ok(weapon.clone())
    }

    fn selected_weapon_resources(
        &self,
        _map: &ProviderReference,
        _weapons: &[ProviderReference],
        _catalog: &InstalledCatalog,
    ) -> Result<Vec<ResourceRequest>, CatalogError> {
        Ok(Vec::new())
    }

    fn selected_weapon_timing(
        &self,
        _map: &ProviderReference,
        _weapons: &[ProviderReference],
        _catalog: &InstalledCatalog,
    ) -> Result<Vec<ProviderTiming>, CatalogError> {
        Ok(Vec::new())
    }

    fn admit_weapon_timing(
        &self,
        _timing: &mut Vec<ProviderTiming>,
        _weapon: &ProviderTiming,
    ) -> Result<(), CatalogError> {
        Ok(())
    }

    fn weapon_provider_ids(&self) -> Vec<ProviderId> {
        Vec::new()
    }
}

/// Launch QVM compatibility for the windowed smoke run (never invoked).
struct WindowedQvmCompat;

impl LaunchQvmCompatibility for WindowedQvmCompat {
    fn read_qvm_compatibility(
        &self,
        _mounts: &dyn BehaviorMounts,
        _artifact_path: &str,
        _digest: &ContentDigest,
        _role: QvmCompatRole,
    ) -> Result<QvmAbiProfile, CatalogError> {
        Err(CatalogError::Invalid("no qvm in windowed smoke".to_string()))
    }
}

/// [`StartupSelectionCollaborators`] for the windowed smoke composition.
/// Player products echo the selected product so selection options round-trip
/// over a real (possibly content-less) catalog; launch-time collaborators
/// report honestly because the smoke run never resolves content.
pub(crate) struct WindowedCollaborators;

impl StartupSelectionCollaborators for WindowedCollaborators {
    fn duplicate(&self) -> Box<dyn StartupSelectionCollaborators> {
        Box::new(WindowedCollaborators)
    }

    fn mod_choices(&self, _catalog: &InstalledCatalog) -> Result<Vec<ModDescription>, String> {
        Ok(Vec::new())
    }

    fn apply_mods(
        &self,
        recipe: ExecutableRecipe,
        _choices: &[ModDescription],
        _mods: &[ModSelection],
    ) -> Result<ExecutableRecipe, String> {
        Ok(recipe)
    }

    fn read_arena_selection(
        &self,
        _catalog: &InstalledCatalog,
        _options: &ApplicationOptions,
    ) -> Result<StartupArenaSelection, String> {
        Ok(StartupArenaSelection::default())
    }

    fn prepare_q3_product(
        &self,
        catalog: &InstalledCatalog,
        _product_id: &str,
        _initial: &ApplicationOptions,
    ) -> Result<PreparedQ3Catalog, String> {
        Ok(PreparedQ3Catalog {
            catalog: catalog.clone(),
            q3_product: None,
        })
    }

    fn load_team_arena(
        &self,
        _catalog: &InstalledCatalog,
        _initial: &ApplicationOptions,
    ) -> Result<Option<PreparedTeamArena>, String> {
        Ok(None)
    }

    fn qvm_grapple_selection(
        &self,
        _catalog: &InstalledCatalog,
        _product_id: &str,
        _mounts: &MountPreparationScope,
    ) -> Result<Option<QvmGrappleStyle>, String> {
        Ok(None)
    }

    fn player_products(
        &self,
        _catalog: &InstalledCatalog,
        product: &str,
        _movement: GameFamily,
        _character: GameFamily,
        _network: &Network,
    ) -> Result<StartupPlayerProducts, String> {
        Ok(StartupPlayerProducts {
            movement: product.to_string(),
            character: product.to_string(),
        })
    }

    fn application_preset(
        &self,
        _catalog: &InstalledCatalog,
        _options: &ApplicationOptions,
        _movement: Option<&ProviderReference>,
        _character: Option<&ProviderReference>,
    ) -> Result<LaunchPreset, String> {
        Err("no launch preset in windowed smoke".to_string())
    }

    fn launch_weapons(&self) -> Box<dyn LaunchWeaponSources> {
        Box::new(WindowedWeaponSources)
    }

    fn launch_compat(&self) -> Box<dyn LaunchQvmCompatibility> {
        Box::new(WindowedQvmCompat)
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
    presentation: Option<WindowedPresentation>,
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
) -> Option<(ClientRenderView, Vec<ImageResourceOperation>)> {
    let target = match seat {
        Some(seat) => ViewTarget::Seat(seat.clone()),
        None => ViewTarget::Preview("windowed".to_string()),
    };
    if let Some(presentation) = scene.presentation.as_mut() {
        let (origin, angles) = presentation
            .spawn()
            .map_or(([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]), |spawn| {
                (
                    [spawn.origin.x, spawn.origin.y, spawn.origin.z],
                    [spawn.angles.x, spawn.angles.y, spawn.angles.z],
                )
            });
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
        Some(scene) => match windowed_scene_view(width, height, seat, time_ms, scene) {
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

/// Production [`StartupBackend`] presenting frames on a native GL window.
pub struct WindowedStartupBackend {
    width: u32,
    height: u32,
    hidden: bool,
    gamma: f64,
    identity: IdentityOwner,
    owner: Option<RendererResourceOwner>,
    renderer: Option<WindowedRenderer>,
    backends: Option<NativeGlBackendFactory>,
    audio: Option<UnifiedAudio>,
    share: Rc<RefCell<WindowedShare>>,
    quit: Rc<Cell<bool>>,
    start: Instant,
    input_router: Option<InputRouter>,
    input_seat: Option<SeatId>,
    input_queue: WindowedInputQueue,
    input_log: VecDeque<SeatInputEvent>,
    scene: Option<WindowedScene>,
    world: Option<WindowedWorld>,
    menu: Option<WindowedMenu>,
}

impl WindowedStartupBackend {
    /// Build an unopened backend over a resolved config and identity. The
    /// identity owns the GL renderer, the image registry, and the input
    /// seat; the map world (loaded before open) builds its presentation
    /// over the same identity so its image uploads apply to the backend.
    fn new(config: &StartupConfig, hidden: bool, gamma: f64, quit: Rc<Cell<bool>>, identity: IdentityOwner) -> Self {
        Self {
            width: config.width,
            height: config.height,
            hidden,
            gamma,
            identity,
            owner: None,
            renderer: None,
            backends: None,
            audio: None,
            share: Rc::new(RefCell::new(WindowedShare::default())),
            quit,
            start: Instant::now(),
            input_router: None,
            input_seat: None,
            input_queue: Rc::new(RefCell::new(Vec::new())),
            input_log: VecDeque::new(),
            scene: None,
            world: None,
            menu: None,
        }
    }

    /// Adopt the menu overlay for the menu entry (donor frontend menu).
    fn set_menu(&mut self, menu: WindowedMenu) {
        self.menu = Some(menu);
    }

    /// Adopt a loaded map world, publishing its scene view: the world
    /// presentation (world geometry plus model-bearing entities) when it
    /// built, else the legacy empty batches. The live actor count is
    /// available through [`Self::world_entity_count`].
    fn set_world(&mut self, mut world: WindowedWorld) {
        let presentation = world.take_presentation();
        self.scene = Some(WindowedScene {
            batches: Vec::new(),
            presentation,
            clear_color: vec4(0.0, 0.0, 0.0, 1.0),
        });
        self.world = Some(world);
    }

    /// Live map-entity count, or `None` when no map world loaded.
    #[must_use]
    pub fn world_entity_count(&self) -> Option<usize> {
        self.world.as_ref().map(WindowedWorld::entity_count)
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
            &mut NativeGlBackendFactory,
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
        let forwarded = std::mem::take(&mut *self.input_queue.borrow_mut());
        for event in forwarded {
            self.input(&event);
        }
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
        build_windowed_commands(width, height, seat.as_ref(), time_ms, self.scene.as_mut())
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
        let mut backends = NativeGlBackendFactory {
            share: Rc::clone(&self.share),
            owner: resource_owner.clone(),
        };
        let images = NativeImages {
            inner: SceneImageRegistry::new(resource_owner),
            owner: owner.clone(),
        };
        let options = NativeRendererOptions {
            renderer: RenderBackendKind::Gl,
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
        self.audio = open_windowed_audio();
        Ok(())
    }

    fn poll(&mut self) -> Result<(), String> {
        let events = {
            let Some(renderer) = self.renderer.as_mut() else {
                return Ok(());
            };
            renderer.window().poll_events()?
        };
        self.handle_window_events(events)
    }

    fn frame(&mut self, ctx: &mut StartupFrame<'_>) -> Result<(), String> {
        let (commands, image_operations) = self.frame_commands();
        let (renderer, backends, owner) = self.live_parts()?;
        Self::apply_frame_images(renderer, &image_operations);
        let frame = RenderFrame { owner, commands };
        renderer.execute(&frame, backends).map_err(|error| error.to_string())?;
        refresh_windowed_audio(&mut self.audio, ctx.elapsed_ms);
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
/// production GL backend, and [`StartupApplication::open`]. Fails before
/// touching a window when the renderer selection is not supported.
pub fn open_windowed_application(
    options: &ApplicationOptions,
    entry: StartupEntry,
) -> Result<WindowedApplication, String> {
    if options.renderer != Renderer::Gl {
        return Err("windowed mode requires --renderer gl".to_string());
    }
    if options.render_worker == Some(true) {
        return Err("windowed worker rendering is not implemented".to_string());
    }
    let config = StartupConfig::from_options(options).map_err(|error| error.to_string())?;
    let catalog = discover_installed_content(&DiscoverContentOptions::new(PathBuf::from(&options.corpus_root)))
        .map_err(|error| error.to_string())?;
    let model = StartupSelectionModel::new(catalog, options.clone(), Box::new(WindowedCollaborators))
        .map_err(|error| error.to_string())?;
    let quit = Rc::new(Cell::new(false));
    let identity = IdentityOwner::create("windowed").map_err(|error| error.to_string())?;
    let menu_seat = identity.seat(0);
    let menu_client = identity.client(0, 0);
    let resource_owner = ResourceOwner::new(7, identity.session().clone(), 0);
    let mut backend = WindowedStartupBackend::new(&config, options.hidden, options.gamma, Rc::clone(&quit), identity);
    if entry == StartupEntry::Menu {
        let menu_model = StartupSelectionModel::new(
            model.catalog().clone(),
            options.clone(),
            Box::new(WindowedCollaborators),
        )
        .map_err(|error| error.to_string())?;
        let menu = WindowedMenu::open(menu_model, menu_seat, menu_client, resource_owner, Rc::clone(&quit))?;
        backend.set_menu(menu);
    } else {
        match load_windowed_world(&config, model.catalog(), options, resource_owner) {
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

/// Drive frames until quit, close, or the frame limit, pacing like the donor
/// active-game cadence (4ms floor), then close and report stepped frames.
pub fn drive_windowed_application(
    app: &mut StartupApplication<WindowedStartupBackend>,
    quit: &Rc<Cell<bool>>,
    frame_limit: Option<u64>,
) -> Result<u64, StartupError> {
    const FRAME_FLOOR: Duration = Duration::from_millis(4);
    let mut last = Instant::now();
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
        let elapsed = last.elapsed();
        if elapsed < FRAME_FLOOR {
            thread::sleep(FRAME_FLOOR - elapsed);
        }
        last = Instant::now();
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
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::ContentId;
    use qa_platform::sdl::{decode_sdl_event, encode_sdl_event, SdlInjectedEvent};

    use super::*;

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
    fn windowed_collaborators_echo_products() {
        let catalog = catalog();
        let collaborators = WindowedCollaborators;
        let products = collaborators
            .player_products(
                &catalog,
                "q2-classic-baseq2",
                GameFamily::Q2,
                GameFamily::Q3,
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
        assert!(collaborators
            .application_preset(&catalog, &ApplicationOptions::default(), None, None)
            .is_err());
    }

    #[test]
    fn windowed_rejects_cpu_and_worker_before_opening() {
        let mut options = windowed_options();
        options.renderer = Renderer::Cpu;
        let Err(error) = open_windowed_application(&options, StartupEntry::Run) else {
            panic!("expected failure");
        };
        assert!(error.contains("--renderer gl"), "{error}");
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
        let (commands, image_operations) = build_windowed_commands(64, 64, None, 12.0, None);
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
        let (commands, image_operations) = build_windowed_commands(64, 48, Some(&seat), 33.0, Some(&mut scene));
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
        let (view, image_operations) = windowed_scene_view(128, 96, None, 7.0, &mut scene).expect("preview view");
        assert!(image_operations.is_empty());
        assert_eq!((view.state.viewport.width, view.state.viewport.height), (128.0, 96.0));
        assert!(matches!(view.target, ViewTarget::Preview(_)));
        assert!(view.operations.is_empty());
        let clear = view.state.clear.expect("view clears");
        assert_eq!(clear.depth, 1.0);
        assert_eq!(clear.color, Some(vec4(0.1, 0.2, 0.3, 1.0)));
        let (other, _) = windowed_scene_view(32, 32, None, 7.0, &mut scene).expect("other size");
        assert_eq!((other.state.viewport.width, other.state.viewport.height), (32.0, 32.0));
    }

    #[test]
    fn windowed_invalid_size_degrades_even_with_scene() {
        let mut scene = WindowedScene::new(vec![test_batch()], vec4(0.0, 0.0, 0.0, 1.0));
        assert!(windowed_scene_view(0, 64, None, 0.0, &mut scene).is_none());
        let (commands, _) = build_windowed_commands(0, 64, None, 0.0, Some(&mut scene));
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
        let (view, _) = windowed_scene_view(64, 64, None, 0.0, &mut scene).unwrap();
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
}
