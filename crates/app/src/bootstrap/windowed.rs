//! Windowed game path: native SDL window, GL renderer, and startup driver.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts`
//! (`StartupApplication.open` graphics tail) and
//! `src/app/bootstrap/renderer.ts` (`NativeRenderer.open`, `openBackend`).
//! The window factory opens an [`SdlWindow`] with GL options (the
//! `SDL_GL_CreateContext` path); the backend factory adopts that context
//! through the SDL render-context transfer protocol and builds a
//! [`GlRenderer`] over [`PlatformGlContext`]; both compose through
//! [`NativeRenderer::open`]; [`WindowedStartupBackend`] drives
//! [`StartupApplication::open`] for `--windowed` runs.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use qa_client::input::router::SeatInputEvent;
use qa_client::render::gl::platform::PlatformGlContext;
use qa_client::render::gl::renderer::GlRenderer;
use qa_client::render::gl::{GlContext, DEPTH_BITS, MAX_TEXTURE_COORDS, MAX_TEXTURE_IMAGE_UNITS, MAX_TEXTURE_SIZE};
use qa_client::render::scene::resources::SceneImageRegistry;
use qa_client::render::types::{DrawBuffer, ImageResourceOperation, OrderedBackend, ResourceOwner};
use qa_content::catalog::{
    discover_installed_content, BehaviorMounts, CatalogError, DiscoverContentOptions, InstalledCatalog, LaunchPreset,
    LaunchQvmCompatibility, LaunchWeaponSources, QvmCompatRole,
};
use qa_content::contract::{
    ContentDigest, ExecutableRecipe, GameFamily, ModDescription, ModSelection, ProviderReference, ProviderTiming,
    QvmAbiProfile, ResourceRequest,
};
use qa_content::mounts::MountPreparationScope;
use qa_core::identity::{IdentityOwner, ProviderId, SeatId};
use qa_core::math::vec4;
use qa_platform::native_libraries::NativeLibraryOptions;
use qa_platform::sdl::{
    SdlBackend, SdlDisplayMode, SdlEvent, SdlGlOptions, SdlWindow, SdlWindowOptions, SdlWindowPresentation,
};
use qa_platform::sdl_render_context::SdlWorkerRenderContext;

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
        // The smoke composition carries no draws, so SetColor paints the
        // donor magenta clear directly; painting at command time (rather than
        // at presentation) keeps armed captures truthful, because captures
        // read before the swap.
        if matches!(command, RenderCommand::SetColor { .. }) {
            if let Some(parts) = self.parts.as_mut() {
                parts.renderer.select_draw_buffer(DrawBuffer::Back, true);
            }
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
struct WindowedCollaborators;

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

/// Production [`StartupBackend`] presenting frames on a native GL window.
pub struct WindowedStartupBackend {
    width: u32,
    height: u32,
    hidden: bool,
    gamma: f64,
    owner: Option<RendererResourceOwner>,
    renderer: Option<WindowedRenderer>,
    backends: Option<NativeGlBackendFactory>,
    share: Rc<RefCell<WindowedShare>>,
    quit: Rc<Cell<bool>>,
    start: Instant,
}

impl WindowedStartupBackend {
    /// Build an unopened backend over a resolved config.
    fn new(config: &StartupConfig, hidden: bool, gamma: f64, quit: Rc<Cell<bool>>) -> Self {
        Self {
            width: config.width,
            height: config.height,
            hidden,
            gamma,
            owner: None,
            renderer: None,
            backends: None,
            share: Rc::new(RefCell::new(WindowedShare::default())),
            quit,
            start: Instant::now(),
        }
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

    /// Arm a capture, present one frame, and return its pixels.
    fn present_and_capture(&mut self) -> Result<Vec<u8>, String> {
        let (renderer, backends, owner) = self.live_parts()?;
        let slot: Rc<RefCell<Option<Result<ImageLevel, String>>>> = Rc::new(RefCell::new(None));
        let writer = Rc::clone(&slot);
        renderer
            .capture_next_frame(Box::new(move |result| {
                *writer.borrow_mut() = Some(result);
            }))
            .map_err(|error| error.to_string())?;
        let frame = RenderFrame {
            owner,
            commands: vec![
                RenderCommand::SetColor {
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderCommand::SwapBuffers,
            ],
        };
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
        let identity = IdentityOwner::create("windowed").map_err(|error| error.to_string())?;
        let resource_owner = ResourceOwner::new(7, identity.session().clone(), 0);
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
        Ok(())
    }

    fn poll(&mut self) -> Result<(), String> {
        let Some(renderer) = self.renderer.as_mut() else {
            return Ok(());
        };
        for event in renderer.window().poll_events()? {
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
        }
        Ok(())
    }

    fn frame(&mut self, _ctx: &mut StartupFrame<'_>) -> Result<(), String> {
        let (renderer, backends, owner) = self.live_parts()?;
        let frame = RenderFrame {
            owner,
            commands: vec![
                RenderCommand::SetColor {
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                },
                RenderCommand::SwapBuffers,
            ],
        };
        renderer.execute(&frame, backends).map_err(|error| error.to_string())
    }

    fn input(&mut self, _event: &SeatInputEvent) -> bool {
        false
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
        false
    }

    fn input_seat(&self) -> Option<SeatId> {
        None
    }

    fn close(&mut self) -> Vec<String> {
        let mut failures = Vec::new();
        if let Some(renderer) = self.renderer.as_mut() {
            if let Err(error) = renderer.close() {
                failures.push(error.to_string());
            }
        }
        self.renderer = None;
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
    let backend = WindowedStartupBackend::new(&config, options.hidden, options.gamma, Rc::clone(&quit));
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

#[cfg(test)]
mod tests {
    use qa_content::catalog::{CatalogProduct, ProductAvailability, ProductExpectation};
    use qa_content::contract::ContentId;

    use super::*;

    fn windowed_options() -> ApplicationOptions {
        ApplicationOptions {
            windowed: true,
            width: 64,
            height: 64,
            frame_limit: Some(3),
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
        let mut backend = WindowedStartupBackend::new(&config, false, 1.0, Rc::new(Cell::new(false)));
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

    #[test]
    fn live_windowed_smoke_runs_frames() {
        // Passes with or without a display: a real windowed run is exercised
        // when GL is available, otherwise the honest open failure is required.
        let options = windowed_options();
        match open_windowed_application(&options, StartupEntry::Run) {
            Ok(composed) => {
                let mut composed = composed;
                let pixels = composed.app.capture_next_frame().expect("windowed capture works");
                assert_eq!(pixels.len(), 64 * 64 * 4);
                for pixel in pixels.as_chunks::<4>().0.iter().step_by(1024) {
                    assert_eq!(pixel[0], 255, "red channel");
                    assert_eq!(pixel[1], 0, "green channel");
                    assert!(pixel[2] == 127 || pixel[2] == 128, "blue channel: {}", pixel[2]);
                    assert_eq!(pixel[3], 255, "alpha channel");
                }
                let frames = drive_windowed_application(&mut composed.app, &composed.quit, Some(3))
                    .expect("windowed drive works");
                assert_eq!(frames, 3);
                assert!(composed.app.is_closed());
            }
            Err(error) => assert!(!error.is_empty(), "honest open failure"),
        }
    }
}
