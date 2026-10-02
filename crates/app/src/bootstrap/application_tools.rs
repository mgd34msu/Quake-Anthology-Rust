//! Application diagnostic tools (port of donor
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/application-tools.ts`).
//!
//! [`ApplicationTools`] binds spline-camera, diagnostic, runtime, and renderer
//! tool commands to each active command buffer, queues diagnostic operations,
//! and paces capture frames. Sync adaptations: registration is synchronous
//! with rollback on failure, queued operations run in [`ApplicationTools::drain`],
//! buffers are shared [`ToolCommands`] handles compared by identity, and
//! [`ApplicationTools::measure`] runs its operation inline.
//!
//! The spline camera (`camera/application.ts`), the timer (`core/omnitimer.ts`),
//! and the diagnostic registrars (`diagnostic-tools.ts`,
//! `renderer-diagnostics.ts`) are out-of-scope siblings, represented by the
//! [`ToolSplineCamera`], [`ToolTimer`], and [`ToolDiagnosticRegistrars`] seams.
//! The capture-frame computation is absorbed here (donor `sourceCaptureFrame`)
//! so frame pacing keeps its exact fixed-point behavior.

use qa_client::view::SceneCamera;
use qa_content::mounts::MountedContent;
use qa_core::cmd_buffer::{CommandBuffer, CommandContext};
use qa_core::cvar::CvarRegistry;
use std::cell::RefCell;
use std::rc::Rc;

/// Shared command buffer handle (donor `CommandBuffer` identity).
pub type ToolCommands = Rc<RefCell<CommandBuffer>>;
/// Release closure for one tool registration.
pub type ToolRelease = Box<dyn FnOnce()>;
/// Queued diagnostic operation; failures are printed by drain.
pub type QueuedToolOperation = Box<dyn FnOnce() -> Result<(), String>>;
/// Shared tool timer (donor `OmniTimer` reference).
pub type SharedToolTimer<T> = Rc<RefCell<T>>;

/// Tool failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationToolError {
    /// A buffer registration failed; earlier registrations were rolled back.
    Registration(String),
    /// Capture pacing inputs were not finite and non-negative.
    CaptureClock(String),
}

impl std::fmt::Display for ApplicationToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registration(message) => write!(f, "{message}"),
            Self::CaptureClock(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ApplicationToolError {}

/// Frame description for runtime diagnostics (donor `RuntimeDiagnosticServices["frame"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolFrameInfo {
    /// Current frame number.
    pub frame: u32,
    /// Frame timestamp in milliseconds.
    pub milliseconds: f64,
    /// Current map name.
    pub map: String,
    /// Renderer name.
    pub renderer: String,
    /// Connected client count.
    pub clients: u32,
}

/// Tool services (donor `ApplicationToolServices`).
///
/// `output_root` is intentionally absent: it only feeds the spline-camera file
/// wiring owned by the camera lane behind [`ToolSplineCamera`].
pub trait ApplicationToolServices {
    /// Renderer handle for renderer diagnostics.
    type Renderer;
    /// Shader handle for renderer diagnostics.
    type Shaders;
    /// Resource handle for renderer diagnostics.
    type Resources;
    /// Open content mounts.
    fn mounts(&self) -> &MountedContent;
    /// Current time in milliseconds.
    fn milliseconds(&self) -> f64;
    /// Active cvar registry, when a world is loaded.
    fn cvars(&self) -> Option<&CvarRegistry>;
    /// Current frame description.
    fn frame(&self) -> ToolFrameInfo;
    /// Renderer handle.
    fn renderer(&self) -> &Self::Renderer;
    /// Shader handle.
    fn shaders(&self) -> &Self::Shaders;
    /// Resource handle.
    fn resources(&self) -> &Self::Resources;
    /// Print a line.
    fn print(&self, text: &str, source: Option<&CommandContext>);
}

/// Tool timer (donor `OmniTimer`).
pub trait ToolTimer {
    /// Whether scopes are recorded.
    fn enabled(&self) -> bool;
    /// Enable or disable recording.
    fn set_enabled(&mut self, enabled: bool);
    /// Open a named scope.
    fn push(&mut self, name: &str);
    /// Close the innermost scope.
    fn pop(&mut self);
    /// Clear all recorded scopes.
    fn reset(&mut self);
}

/// Spline camera (donor `ApplicationSplineCamera`).
pub trait ToolSplineCamera {
    /// Register camera commands on a buffer.
    fn register(&mut self, commands: &ToolCommands) -> Result<ToolRelease, ApplicationToolError>;
    /// Finish pending camera work.
    fn drain(&mut self);
    /// Reset camera state for a world change.
    fn reset(&mut self);
    /// Apply the camera override to a scene camera.
    fn apply(&mut self, camera: &SceneCamera) -> SceneCamera;
    /// Release camera resources.
    fn close(&mut self);
}

/// Diagnostic operation queue (donor `queue` callback).
#[derive(Clone)]
pub struct ToolOperationQueue {
    pending: Rc<RefCell<Vec<QueuedToolOperation>>>,
}

impl ToolOperationQueue {
    /// Queue an operation for the next drain.
    pub fn queue(&self, operation: QueuedToolOperation) {
        self.pending.borrow_mut().push(operation);
    }
}

/// Diagnostic registrars (donors `diagnostic-tools.ts`, `renderer-diagnostics.ts`).
pub trait ToolDiagnosticRegistrars {
    /// Tool services.
    type Services: ApplicationToolServices;
    /// Tool timer.
    type Timer: ToolTimer;
    /// Register timer diagnostics (donor `registerDiagnosticTools`).
    fn register_diagnostic_tools(
        &self,
        commands: &ToolCommands,
        timer: &SharedToolTimer<Self::Timer>,
        services: &Self::Services,
    ) -> Result<ToolRelease, ApplicationToolError>;
    /// Register runtime diagnostics (donor `registerRuntimeDiagnostics`).
    fn register_runtime_diagnostics(
        &self,
        commands: &ToolCommands,
        services: &Self::Services,
        queue: &ToolOperationQueue,
    ) -> Result<ToolRelease, ApplicationToolError>;
    /// Register renderer diagnostics (donor `registerRendererDiagnostics`).
    fn register_renderer_diagnostics(
        &self,
        commands: &ToolCommands,
        services: &Self::Services,
    ) -> Result<ToolRelease, ApplicationToolError>;
}

/// Capture pacing (donor `sourceCaptureFrame` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToolCaptureFrame {
    /// Frame interval in milliseconds.
    pub milliseconds: f64,
    /// Whether this frame is captured.
    pub capture: bool,
}

/// Pace a capture frame (absorbed donor `sourceCaptureFrame`).
pub fn source_capture_frame(
    elapsed: f64,
    fps: f64,
    timescale: f64,
    active: bool,
    force: bool,
) -> Result<ToolCaptureFrame, ApplicationToolError> {
    if !elapsed.is_finite()
        || !fps.is_finite()
        || !timescale.is_finite()
        || elapsed < 0.0
        || fps < 0.0
        || timescale < 0.0
    {
        return Err(ApplicationToolError::CaptureClock("Invalid capture clock".to_string()));
    }
    let fps = fps.trunc();
    if fps == 0.0 || elapsed == 0.0 {
        return Ok(ToolCaptureFrame {
            milliseconds: elapsed,
            capture: false,
        });
    }
    let quantum = (1000.0 / fps).trunc();
    let scaled = (quantum * f64::from(timescale as f32)) as f32;
    Ok(ToolCaptureFrame {
        milliseconds: 1.0_f64.max(f64::from(scaled).trunc()),
        capture: active || force,
    })
}

/// Application diagnostic tools (donor `ApplicationTools`).
pub struct ApplicationTools<S, T, C, R> {
    /// Shared tool timer (donor `timer`).
    pub timer: SharedToolTimer<T>,
    camera: C,
    services: S,
    registrars: R,
    bindings: Vec<(ToolCommands, Option<ToolRelease>)>,
    pending: Rc<RefCell<Vec<QueuedToolOperation>>>,
}

impl<S, T, C, R> ApplicationTools<S, T, C, R>
where
    S: ApplicationToolServices,
    T: ToolTimer,
    C: ToolSplineCamera,
    R: ToolDiagnosticRegistrars<Services = S, Timer = T>,
{
    /// Borrow tools over services, a timer, a camera, and registrars.
    pub fn new(timer: T, camera: C, services: S, registrars: R) -> Self {
        Self {
            timer: Rc::new(RefCell::new(timer)),
            camera,
            services,
            registrars,
            bindings: Vec::new(),
            pending: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Bind tool commands to the current buffers (donor `bind`).
    pub fn bind(&mut self, buffers: &[ToolCommands]) -> Result<(), ApplicationToolError> {
        let mut kept = Vec::with_capacity(self.bindings.len());
        for (owner, release) in self.bindings.drain(..) {
            if buffers.iter().any(|buffer| Rc::ptr_eq(buffer, &owner)) {
                kept.push((owner, release));
            } else if let Some(release) = release {
                release();
            }
        }
        self.bindings = kept;
        for owner in buffers {
            if self.bindings.iter().any(|(bound, _)| Rc::ptr_eq(bound, owner)) {
                continue;
            }
            let mut releases: Vec<ToolRelease> = Vec::new();
            let registered = self.register(owner, &mut releases);
            if let Err(error) = registered {
                for release in releases.into_iter().rev() {
                    release();
                }
                return Err(error);
            }
            self.bindings.push((
                Rc::clone(owner),
                Some(Box::new(move || {
                    for release in releases.into_iter().rev() {
                        release();
                    }
                }) as ToolRelease),
            ));
        }
        Ok(())
    }

    /// Register one buffer, collecting releases for rollback.
    fn register(&mut self, owner: &ToolCommands, releases: &mut Vec<ToolRelease>) -> Result<(), ApplicationToolError> {
        releases.push(self.camera.register(owner)?);
        releases.push(
            self.registrars
                .register_diagnostic_tools(owner, &self.timer, &self.services)?,
        );
        let queue = ToolOperationQueue {
            pending: Rc::clone(&self.pending),
        };
        releases.push(
            self.registrars
                .register_runtime_diagnostics(owner, &self.services, &queue)?,
        );
        releases.push(self.registrars.register_renderer_diagnostics(owner, &self.services)?);
        Ok(())
    }

    /// Finish pending camera work and queued operations (donor `drain`).
    pub fn drain(&mut self) {
        self.camera.drain();
        for operation in self.pending.borrow_mut().drain(..) {
            if let Err(error) = operation() {
                self.services.print(&format!("Diagnostic: {error}\n"), None);
            }
        }
    }

    /// Release bindings and reset the camera for a world change (donor `beforeWorldChange`).
    pub fn before_world_change(&mut self) {
        self.camera.reset();
        for (_, release) in self.bindings.drain(..) {
            if let Some(release) = release {
                release();
            }
        }
    }

    /// Apply the camera override (donor `applyCamera`).
    pub fn apply_camera(&mut self, camera: &SceneCamera) -> SceneCamera {
        self.camera.apply(camera)
    }

    /// Pace a capture frame from console cvars (donor `frameTime`).
    pub fn frame_time(&self, milliseconds: f64, active: bool) -> Result<ToolCaptureFrame, ApplicationToolError> {
        let cvars = self.services.cvars();
        let fps = cvars.map_or(0.0, |cvars| f64::from(cvars.variable_value("cl_avidemo")));
        let timescale = cvars
            .and_then(|cvars| cvars.get("timescale"))
            .map_or(1.0, |found| f64::from(found.numeric_value));
        let force = cvars.is_some_and(|cvars| cvars.variable_value("cl_forceavidemo") != 0.0);
        source_capture_frame(milliseconds, fps, timescale, active, force)
    }

    /// Run an operation inside a named timer scope (donor `measureAsync`).
    pub fn measure<F, V>(&mut self, name: &str, operation: F) -> V
    where
        F: FnOnce() -> V,
    {
        if !self.timer.borrow().enabled() {
            return operation();
        }
        self.timer.borrow_mut().push(name);
        let value = operation();
        self.timer.borrow_mut().pop();
        value
    }

    /// Release tools (donor `close`).
    pub fn close(&mut self) {
        self.before_world_change();
        self.drain();
        self.camera.close();
        self.timer.borrow_mut().set_enabled(false);
        self.timer.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{
        create_mount_id, create_mount_identity, ContentId, ContentMount, LooseMount, MountPlanId, ResolvedMountPlan,
    };
    use qa_content::mounts::{open_mount_plan, OpenMountOptions};
    use qa_core::cmd::Dialect;
    use qa_core::cmd_buffer::{BufferOptions, CommandOrigin};
    use qa_core::identity::IdentityOwner;

    struct StubServices {
        mounts: MountedContent,
        cvars: CvarRegistry,
        printed: RefCell<Vec<String>>,
    }

    impl ApplicationToolServices for StubServices {
        type Renderer = ();
        type Shaders = ();
        type Resources = ();
        fn mounts(&self) -> &MountedContent {
            &self.mounts
        }
        fn milliseconds(&self) -> f64 {
            4.0
        }
        fn cvars(&self) -> Option<&CvarRegistry> {
            Some(&self.cvars)
        }
        fn frame(&self) -> ToolFrameInfo {
            ToolFrameInfo {
                frame: 1,
                milliseconds: 4.0,
                map: "q3dm1".to_string(),
                renderer: "test".to_string(),
                clients: 1,
            }
        }
        fn renderer(&self) -> &() {
            &()
        }
        fn shaders(&self) -> &() {
            &()
        }
        fn resources(&self) -> &() {
            &()
        }
        fn print(&self, text: &str, _source: Option<&CommandContext>) {
            self.printed.borrow_mut().push(text.to_string());
        }
    }

    #[derive(Default)]
    struct StubTimer {
        enabled: bool,
        scopes: Vec<String>,
        resets: u32,
    }

    impl ToolTimer for StubTimer {
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
        }
        fn push(&mut self, name: &str) {
            self.scopes.push(name.to_string());
        }
        fn pop(&mut self) {
            self.scopes.pop();
        }
        fn reset(&mut self) {
            self.resets += 1;
        }
    }

    #[derive(Default)]
    struct StubCamera {
        registrations: u32,
        drains: u32,
        resets: u32,
        closes: u32,
    }

    impl ToolSplineCamera for StubCamera {
        fn register(&mut self, _commands: &ToolCommands) -> Result<ToolRelease, ApplicationToolError> {
            self.registrations += 1;
            Ok(Box::new(|| {}))
        }
        fn drain(&mut self) {
            self.drains += 1;
        }
        fn reset(&mut self) {
            self.resets += 1;
        }
        fn apply(&mut self, camera: &SceneCamera) -> SceneCamera {
            *camera
        }
        fn close(&mut self) {
            self.closes += 1;
        }
    }

    struct StubRegistrars {
        events: Rc<RefCell<Vec<String>>>,
        fail_runtime: bool,
    }

    impl ToolDiagnosticRegistrars for StubRegistrars {
        type Services = StubServices;
        type Timer = StubTimer;
        fn register_diagnostic_tools(
            &self,
            _commands: &ToolCommands,
            _timer: &SharedToolTimer<StubTimer>,
            _services: &StubServices,
        ) -> Result<ToolRelease, ApplicationToolError> {
            self.events.borrow_mut().push("diagnostic".to_string());
            let events = Rc::clone(&self.events);
            Ok(Box::new(move || {
                events.borrow_mut().push("release-diagnostic".to_string())
            }))
        }
        fn register_runtime_diagnostics(
            &self,
            _commands: &ToolCommands,
            _services: &StubServices,
            queue: &ToolOperationQueue,
        ) -> Result<ToolRelease, ApplicationToolError> {
            if self.fail_runtime {
                return Err(ApplicationToolError::Registration("runtime boom".to_string()));
            }
            self.events.borrow_mut().push("runtime".to_string());
            queue.queue(Box::new(|| Err("queued boom".to_string())));
            let events = Rc::clone(&self.events);
            Ok(Box::new(move || {
                events.borrow_mut().push("release-runtime".to_string())
            }))
        }
        fn register_renderer_diagnostics(
            &self,
            _commands: &ToolCommands,
            _services: &StubServices,
        ) -> Result<ToolRelease, ApplicationToolError> {
            self.events.borrow_mut().push("renderer".to_string());
            let events = Rc::clone(&self.events);
            Ok(Box::new(move || {
                events.borrow_mut().push("release-renderer".to_string())
            }))
        }
    }

    fn mounts() -> MountedContent {
        let content = ContentId("tool-test".to_string());
        let mount = LooseMount {
            identity: create_mount_identity(create_mount_id("tools", "test").unwrap(), content, 0).unwrap(),
            root_path: std::env::temp_dir()
                .join("qa-tool-mounts")
                .to_string_lossy()
                .into_owned(),
        };
        open_mount_plan(
            &ResolvedMountPlan {
                id: MountPlanId("mount-plan:tools:test".to_string()),
                mounts: vec![ContentMount::Loose(mount.clone())],
                default_order: vec![mount.identity.id.clone()],
                prefix_orders: Vec::new(),
            },
            OpenMountOptions::default(),
        )
        .unwrap()
    }

    fn services() -> StubServices {
        StubServices {
            mounts: mounts(),
            cvars: CvarRegistry::new(Dialect::Q3),
            printed: RefCell::new(Vec::new()),
        }
    }

    fn buffer() -> ToolCommands {
        let owner = IdentityOwner::create("tools-test").unwrap();
        let context = CommandContext::new(owner.session().clone(), CommandOrigin::LocalConsole);
        Rc::new(RefCell::new(
            CommandBuffer::new(Dialect::Q3, context, BufferOptions::new()).unwrap(),
        ))
    }

    type Harness = (
        ApplicationTools<StubServices, StubTimer, StubCamera, StubRegistrars>,
        Rc<RefCell<Vec<String>>>,
    );

    fn tools() -> Harness {
        let events = Rc::new(RefCell::new(Vec::new()));
        let tools = ApplicationTools::new(
            StubTimer::default(),
            StubCamera::default(),
            services(),
            StubRegistrars {
                events: Rc::clone(&events),
                fail_runtime: false,
            },
        );
        (tools, events)
    }

    #[test]
    fn bind_registers_and_releases_by_identity() {
        let (mut tools, events) = tools();
        let first = buffer();
        let second = buffer();
        tools.bind(std::slice::from_ref(&first)).unwrap();
        tools.bind(&[Rc::clone(&first), Rc::clone(&second)]).unwrap();
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| event.as_str() == "runtime")
                .count(),
            2
        );
        tools.bind(std::slice::from_ref(&second)).unwrap();
        assert!(events.borrow().contains(&"release-runtime".to_string()));
        tools.bind(&[]).unwrap();
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| event.starts_with("release-"))
                .count(),
            6
        );
    }

    #[test]
    fn bind_rolls_back_failed_registration() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let mut tools = ApplicationTools::new(
            StubTimer::default(),
            StubCamera::default(),
            services(),
            StubRegistrars {
                events: Rc::clone(&events),
                fail_runtime: true,
            },
        );
        let error = tools.bind(std::slice::from_ref(&buffer())).unwrap_err();
        assert_eq!(error, ApplicationToolError::Registration("runtime boom".to_string()));
        assert_eq!(events.borrow().as_slice(), ["diagnostic", "release-diagnostic"]);
    }

    #[test]
    fn drain_reports_queued_failures() {
        let (mut tools, _) = tools();
        tools.bind(std::slice::from_ref(&buffer())).unwrap();
        tools.drain();
        assert_eq!(
            tools.services.printed.borrow().as_slice(),
            ["Diagnostic: queued boom\n"]
        );
        tools.drain();
        assert_eq!(tools.services.printed.borrow().len(), 1);
    }

    #[test]
    fn frame_time_paces_capture() {
        let (tools, _) = tools();
        let idle = tools.frame_time(16.0, true).unwrap();
        assert!(!idle.capture);
        assert_eq!(idle.milliseconds, 16.0);
        let mut tools = tools;
        tools.services.cvars.set("cl_avidemo", "30", true).unwrap();
        let paced = tools.frame_time(16.0, false).unwrap();
        assert_eq!(paced.milliseconds, 33.0);
        assert!(!paced.capture);
        let forced = tools.frame_time(16.0, true).unwrap();
        assert!(forced.capture);
        assert!(tools.frame_time(-1.0, false).is_err());
    }

    #[test]
    fn measure_and_close_follow_timer_lifecycle() {
        let (mut tools, _) = tools();
        tools.timer.borrow_mut().enabled = true;
        let value = tools.measure("frame", || 7);
        assert_eq!(value, 7);
        assert!(tools.timer.borrow().scopes.is_empty());
        tools.close();
        assert!(!tools.timer.borrow().enabled);
        assert_eq!(tools.timer.borrow().resets, 1);
        assert_eq!(tools.camera.resets, 1);
        assert_eq!(tools.camera.drains, 1);
        assert_eq!(tools.camera.closes, 1);
    }
}
