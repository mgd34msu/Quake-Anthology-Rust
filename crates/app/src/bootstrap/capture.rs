//! Console capture ownership: screenshot readback, config text, command registration.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/capture.ts`
//! (`applicationCaptureRoot`, `ApplicationCaptureServices`,
//! `inputCaptureServices`, `ApplicationCapture`). The port is synchronous:
//! operations run inline, so `drain` has nothing to await; the
//! cancel-before-presentation error survives for reads that reentrantly
//! observe `before_world_change`. Console registration ([`console`](super::console)),
//! archived bindings, and archive commands ([`cvar_archives`](super::cvar_archives))
//! arrive through seams because the donor's registration calls run host-side.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use qa_client::capture::{
    encode_jpeg_image, encode_png_image, encode_tga_image, FrameCapture, FrameReadback, RgbaImage, ScreenshotEncoders,
    ScreenshotFormat,
};
use qa_client::render::types::ImageLevel;
use qa_client::ClientError;
use qa_core::identity::SeatId;
use thiserror::Error;

use super::config_scripts::{console_config_root, seat_console_config};
use crate::settings::config::ConfigStore;

/// Capture failure.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Activate after close.
    #[error("Cannot activate a closed capture owner")]
    Closed,
    /// Client failure.
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Root holding captures (`applicationCaptureRoot`).
#[must_use]
pub fn application_capture_root(user_content_root: Option<&Path>) -> PathBuf {
    console_config_root(user_content_root)
}

/// Command origin chain for configuration text.
#[derive(Debug, Clone)]
pub enum CaptureCommandOrigin {
    /// Script frame wrapping its caller.
    Script {
        /// Calling origin.
        caller: Box<CaptureCommandOrigin>,
    },
    /// Local seat origin.
    LocalSeat {
        /// Origin seat.
        seat: SeatId,
    },
    /// Any other origin.
    Other,
}

/// Seat behind script frames, if the chain ends at a local seat.
fn origin_seat(origin: &CaptureCommandOrigin) -> Option<&SeatId> {
    let mut current = origin;
    while let CaptureCommandOrigin::Script { caller } = current {
        current = caller;
    }
    match current {
        CaptureCommandOrigin::LocalSeat { seat } => Some(seat),
        _ => None,
    }
}

/// Renderer readback (`NativeRenderer.captureNextFrame`, sync).
pub trait CaptureRenderer {
    /// Readback failure.
    type Error;
    /// Capture the next presented frame.
    fn capture_next_frame(&mut self) -> Result<ImageLevel, Self::Error>;
}

struct RendererReadback<R> {
    renderer: Rc<RefCell<R>>,
    reads: Rc<Cell<usize>>,
    cancelled: Rc<Cell<bool>>,
}

impl<R> FrameReadback for RendererReadback<R>
where
    R: CaptureRenderer,
    R::Error: Into<ClientError>,
{
    fn read_rgba(&mut self) -> Result<RgbaImage, ClientError> {
        self.reads.set(self.reads.get() + 1);
        let frame = self.renderer.borrow_mut().capture_next_frame().map_err(Into::into);
        self.reads.set(self.reads.get() - 1);
        if self.cancelled.take() {
            return Err(ClientError::BadUi(
                "Screenshot cancelled before the requested frame was presented".to_string(),
            ));
        }
        let frame = frame?;
        Ok(RgbaImage {
            width: frame.width,
            height: frame.height,
            pixels: frame.pixels,
        })
    }
}

/// Capture services (donor `ApplicationCaptureServices`, sync).
pub trait ApplicationCaptureServices {
    /// Service failure.
    type Error;
    /// Console-lane configuration-write hook.
    type WriteHook: Clone;
    /// Serialized configuration root for a seat.
    fn config(&self, seat: &SeatId) -> ConfigStore;
    /// Archived binding lines for a seat.
    fn bindings_text(&self, seat: &SeatId) -> Result<Vec<String>, Self::Error>;
    /// Archived command lines for an origin.
    fn archive_text(&mut self, origin: &CaptureCommandOrigin) -> Result<Vec<String>, Self::Error>;
    /// Whether a seat has a console.
    fn has_console(&self, seat: &SeatId) -> bool;
    /// Whether chat bindings apply.
    fn can_chat(&self) -> bool;
    /// Capture root.
    fn root(&self) -> PathBuf;
    /// Current map name.
    fn map_name(&self) -> String;
    /// Serialize an operation after presentation (runs inline in sync hosts).
    fn write(&mut self, operation: Box<dyn FnOnce() -> Result<(), String>>) -> Result<(), String>;
    /// Print a line.
    fn print(&mut self, text: &str);
    /// Optional configuration-write hook.
    fn configuration_write_started(&self) -> Option<Self::WriteHook>;
}

/// Assemble configuration text for an origin.
fn configuration_text<S>(services: &mut S, origin: &CaptureCommandOrigin) -> Result<Option<String>, CaptureError>
where
    S: ApplicationCaptureServices,
    S::Error: Into<CaptureError>,
{
    let Some(seat) = origin_seat(origin) else {
        return Ok(None);
    };
    let bindings = services.bindings_text(seat).map_err(Into::into)?;
    let archived = services.archive_text(origin).map_err(Into::into)?;
    let mut lines = vec!["// Generated by quake-typescript".to_string()];
    lines.extend(bindings);
    lines.extend(archived);
    lines.push(String::new());
    Ok(Some(lines.join("\n")))
}

/// Configuration text lookup for a capture origin.
pub type CaptureConfigurationLookup = dyn Fn(&CaptureCommandOrigin) -> Result<Option<String>, String>;
/// Frame capture lookup for a seat.
pub type SeatCaptureLookup = dyn Fn(&SeatId) -> Option<FrameCapture<'static>>;
/// Queued capture operation.
pub type QueuedCaptureOperation = dyn FnOnce() -> Result<(), String>;

/// Console command bindings (`registerConsoleCommands` input).
pub struct CaptureCommandBindings<W> {
    /// Serialized configuration root for a seat.
    pub config: Box<dyn Fn(&SeatId) -> ConfigStore>,
    /// Configuration text for an origin.
    pub configuration: Box<CaptureConfigurationLookup>,
    /// Whether a seat has a console.
    pub has_console: Box<dyn Fn(&SeatId) -> bool>,
    /// Whether chat bindings apply.
    pub can_chat: Box<dyn Fn() -> bool>,
    /// Frame capture for a seat with a console.
    pub capture: Box<SeatCaptureLookup>,
    /// Current map name.
    pub map_name: Box<dyn Fn() -> String>,
    /// Print a line.
    pub print: Box<dyn Fn(&str)>,
    /// Queue an operation, optionally with readback.
    pub queue: Box<dyn Fn(Box<QueuedCaptureOperation>, bool)>,
    /// Optional configuration-write hook.
    pub configuration_write_started: Option<W>,
}

/// Console command registration (donor `registerConsoleCommands`).
pub trait CaptureCommandRegistrar {
    /// Registration failure.
    type Error;
    /// Console-lane configuration-write hook.
    type WriteHook;
    /// Register capture commands, returning an unregister closure.
    fn register_capture_commands(
        &mut self,
        bindings: CaptureCommandBindings<Self::WriteHook>,
    ) -> Result<Box<dyn FnOnce()>, Self::Error>;
}

/// Engine input surface adapted by [`input_capture_services`].
pub trait CaptureInput {
    /// Input failure.
    type Error;
    /// Archive command lines for an origin (`commands.archiveCommands`).
    fn archive_commands(&mut self, origin: &CaptureCommandOrigin) -> Result<Vec<String>, Self::Error>;
    /// Local seats (`input.locals`).
    fn locals(&self) -> Vec<CaptureInputLocal>;
    /// Whether chat bindings apply (`bindingCapabilities.chat`).
    fn chat_capability(&self) -> bool;
    /// Serialize an operation (`scripts.write`).
    fn write_script(&mut self, operation: Box<dyn FnOnce() -> Result<(), String>>) -> Result<(), String>;
}

/// One engine local seat.
#[derive(Debug, Clone)]
pub struct CaptureInputLocal {
    /// Local seat.
    pub seat: SeatId,
    /// Whether the local has a console.
    pub has_console: bool,
    /// Archived binding lines.
    pub bindings: Vec<String>,
}

/// Capture services adapted from engine input.
pub struct InputCaptureServices<I> {
    input: I,
    root: PathBuf,
    map_name: Box<dyn Fn() -> String>,
    print: Box<dyn FnMut(&str)>,
}

/// Adapt engine locals for the capture console (`inputCaptureServices`).
pub fn input_capture_services<I>(
    input: I,
    root: PathBuf,
    map_name: Box<dyn Fn() -> String>,
    print: Box<dyn FnMut(&str)>,
) -> InputCaptureServices<I> {
    InputCaptureServices {
        input,
        root,
        map_name,
        print,
    }
}

impl<I> ApplicationCaptureServices for InputCaptureServices<I>
where
    I: CaptureInput,
    I::Error: Into<CaptureError>,
{
    type Error = CaptureError;
    type WriteHook = ();

    fn config(&self, seat: &SeatId) -> ConfigStore {
        seat_console_config(&application_capture_root(Some(self.root.as_path())), seat)
    }

    fn bindings_text(&self, seat: &SeatId) -> Result<Vec<String>, CaptureError> {
        Ok(self
            .input
            .locals()
            .into_iter()
            .find(|local| local.seat == *seat)
            .map(|local| local.bindings)
            .unwrap_or_default())
    }

    fn archive_text(&mut self, origin: &CaptureCommandOrigin) -> Result<Vec<String>, CaptureError> {
        self.input.archive_commands(origin).map_err(Into::into)
    }

    fn has_console(&self, seat: &SeatId) -> bool {
        self.input
            .locals()
            .iter()
            .any(|local| local.seat == *seat && local.has_console)
    }

    fn can_chat(&self) -> bool {
        self.input.chat_capability()
    }

    fn root(&self) -> PathBuf {
        self.root.clone()
    }

    fn map_name(&self) -> String {
        (self.map_name)()
    }

    fn write(&mut self, operation: Box<dyn FnOnce() -> Result<(), String>>) -> Result<(), String> {
        self.input.write_script(operation)
    }

    fn print(&mut self, text: &str) {
        (self.print)(text);
    }

    fn configuration_write_started(&self) -> Option<()> {
        None
    }
}

fn frame_encoders() -> ScreenshotEncoders<'static> {
    ScreenshotEncoders {
        tga: &encode_tga_image,
        png: &encode_png_image,
        jpg: &encode_jpeg_image,
    }
}

fn run_queued<S>(
    services: &Rc<RefCell<S>>,
    operations: &Rc<Cell<usize>>,
    closed: &Rc<Cell<bool>>,
    operation: Box<dyn FnOnce() -> Result<(), String>>,
    frame_readback: bool,
) where
    S: ApplicationCaptureServices,
    S::Error: Into<CaptureError>,
{
    if closed.get() {
        services.borrow_mut().print("Console output owner is closed\n");
        return;
    }
    operations.set(operations.get() + 1);
    let result = if frame_readback {
        operation()
    } else {
        services.borrow_mut().write(operation)
    };
    operations.set(operations.get() - 1);
    if let Err(error) = result {
        services
            .borrow_mut()
            .print(&format!("Console output failed: {error}\n"));
    }
}

/// Console capture owner (`ApplicationCapture`).
pub struct ApplicationCapture<S, R> {
    services: Rc<RefCell<S>>,
    renderer: Rc<RefCell<R>>,
    reads: Rc<Cell<usize>>,
    cancelled: Rc<Cell<bool>>,
    operations: Rc<Cell<usize>>,
    closed: Rc<Cell<bool>>,
    unregister: Option<Box<dyn FnOnce()>>,
}

impl<S, R> ApplicationCapture<S, R>
where
    S: ApplicationCaptureServices + 'static,
    S::Error: Into<CaptureError>,
    R: CaptureRenderer + 'static,
    R::Error: Into<ClientError>,
{
    /// Borrow the console capture owner.
    pub fn new(services: S, renderer: R) -> Self {
        Self {
            services: Rc::new(RefCell::new(services)),
            renderer: Rc::new(RefCell::new(renderer)),
            reads: Rc::new(Cell::new(0)),
            cancelled: Rc::new(Cell::new(false)),
            operations: Rc::new(Cell::new(0)),
            closed: Rc::new(Cell::new(false)),
            unregister: None,
        }
    }

    /// Build a frame capture for the current root.
    ///
    /// The donor caches one capture per root; captures hold no mutable
    /// sequencing state (shot indexes are claimed from disk), so rebuilding
    /// per use is behaviorally identical and keeps every binding `'static`.
    fn frame_capture(&self) -> FrameCapture<'static> {
        let readback = RendererReadback {
            renderer: Rc::clone(&self.renderer),
            reads: Rc::clone(&self.reads),
            cancelled: Rc::clone(&self.cancelled),
        };
        FrameCapture::new(self.services.borrow().root(), Box::new(readback), frame_encoders())
    }

    /// Register console commands (`activate`).
    pub fn activate<G>(&mut self, registrar: &mut G) -> Result<(), CaptureError>
    where
        G: CaptureCommandRegistrar,
        G::Error: Into<CaptureError>,
        S::WriteHook: Into<G::WriteHook>,
    {
        if self.closed.get() {
            return Err(CaptureError::Closed);
        }
        if self.unregister.is_some() {
            return Ok(());
        }
        let services = Rc::clone(&self.services);
        let configuration_services = Rc::clone(&self.services);
        let console_services = Rc::clone(&self.services);
        let chat_services = Rc::clone(&self.services);
        let capture_services = Rc::clone(&self.services);
        let capture_renderer = Rc::clone(&self.renderer);
        let capture_reads = Rc::clone(&self.reads);
        let capture_cancelled = Rc::clone(&self.cancelled);
        let map_services = Rc::clone(&self.services);
        let print_services = Rc::clone(&self.services);
        let queue_services = Rc::clone(&self.services);
        let queue_operations = Rc::clone(&self.operations);
        let queue_closed = Rc::clone(&self.closed);
        let hook = self.services.borrow().configuration_write_started().map(Into::into);
        let bindings = CaptureCommandBindings {
            config: Box::new(move |seat| services.borrow().config(seat)),
            configuration: Box::new(move |origin| {
                configuration_text(&mut *configuration_services.borrow_mut(), origin).map_err(|error| error.to_string())
            }),
            has_console: Box::new(move |seat| console_services.borrow().has_console(seat)),
            can_chat: Box::new(move || chat_services.borrow().can_chat()),
            capture: Box::new(move |seat| {
                if !capture_services.borrow().has_console(seat) {
                    return None;
                }
                let readback = RendererReadback {
                    renderer: Rc::clone(&capture_renderer),
                    reads: Rc::clone(&capture_reads),
                    cancelled: Rc::clone(&capture_cancelled),
                };
                Some(FrameCapture::new(
                    capture_services.borrow().root(),
                    Box::new(readback),
                    frame_encoders(),
                ))
            }),
            map_name: Box::new(move || map_services.borrow().map_name()),
            print: Box::new(move |text| print_services.borrow_mut().print(text)),
            queue: Box::new(move |operation, frame_readback| {
                run_queued(
                    &queue_services,
                    &queue_operations,
                    &queue_closed,
                    operation,
                    frame_readback,
                );
            }),
            configuration_write_started: hook,
        };
        self.unregister = Some(registrar.register_capture_commands(bindings).map_err(Into::into)?);
        Ok(())
    }

    /// Queue an operation, optionally with readback.
    pub fn queue(&mut self, operation: Box<dyn FnOnce() -> Result<(), String>>, frame_readback: bool) {
        run_queued(
            &self.services,
            &self.operations,
            &self.closed,
            operation,
            frame_readback,
        );
    }

    /// Capture one TGA frame (`captureFrame`).
    pub fn capture_frame(&mut self) {
        let capture = self.frame_capture();
        self.queue(
            Box::new(move || {
                let mut capture = capture;
                capture
                    .screenshot(ScreenshotFormat::Tga, None, 90)
                    .map_err(|error| error.to_string())?;
                Ok(())
            }),
            true,
        );
    }

    /// Whether a readback is in flight (`pendingReadback`).
    #[must_use]
    pub fn pending_readback(&self) -> bool {
        self.reads.get() != 0
    }

    /// Drain queued operations (`drain`).
    pub fn drain(&self) {
        debug_assert_eq!(self.operations.get(), 0, "sync operations complete inline");
    }

    /// Cancel in-flight readback before a world change (`beforeWorldChange`).
    pub fn before_world_change(&self) {
        if self.reads.get() != 0 {
            self.cancelled.set(true);
        }
        self.drain();
    }

    /// Close the capture owner (`close`).
    pub fn close(&mut self) {
        if self.closed.get() {
            return;
        }
        self.closed.set(true);
        if let Some(unregister) = self.unregister.take() {
            unregister();
        }
        self.before_world_change();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubServices {
        root: PathBuf,
        lines: Rc<RefCell<Vec<String>>>,
        consoles: Vec<SeatId>,
        bindings: Vec<String>,
        archived: Vec<String>,
    }

    impl ApplicationCaptureServices for StubServices {
        type Error = CaptureError;
        type WriteHook = ();

        fn config(&self, seat: &SeatId) -> ConfigStore {
            seat_console_config(&self.root, seat)
        }

        fn bindings_text(&self, _seat: &SeatId) -> Result<Vec<String>, CaptureError> {
            Ok(self.bindings.clone())
        }

        fn archive_text(&mut self, _origin: &CaptureCommandOrigin) -> Result<Vec<String>, CaptureError> {
            Ok(self.archived.clone())
        }

        fn has_console(&self, seat: &SeatId) -> bool {
            self.consoles.contains(seat)
        }

        fn can_chat(&self) -> bool {
            true
        }

        fn root(&self) -> PathBuf {
            self.root.clone()
        }

        fn map_name(&self) -> String {
            "maps/start.bsp".to_string()
        }

        fn write(&mut self, operation: Box<dyn FnOnce() -> Result<(), String>>) -> Result<(), String> {
            operation()
        }

        fn print(&mut self, text: &str) {
            self.lines.borrow_mut().push(text.to_string());
        }

        fn configuration_write_started(&self) -> Option<()> {
            None
        }
    }

    struct StubRenderer {
        frames: usize,
    }

    impl CaptureRenderer for StubRenderer {
        type Error = ClientError;

        fn capture_next_frame(&mut self) -> Result<ImageLevel, ClientError> {
            self.frames += 1;
            Ok(ImageLevel {
                width: 2,
                height: 2,
                pixels: vec![1; 16],
            })
        }
    }

    struct StubRegistrar {
        bound: bool,
        unregistered: Rc<Cell<bool>>,
    }

    impl CaptureCommandRegistrar for StubRegistrar {
        type Error = CaptureError;
        type WriteHook = ();

        fn register_capture_commands(
            &mut self,
            _bindings: CaptureCommandBindings<()>,
        ) -> Result<Box<dyn FnOnce()>, CaptureError> {
            self.bound = true;
            let unregistered = Rc::clone(&self.unregistered);
            Ok(Box::new(move || unregistered.set(true)))
        }
    }

    fn seat() -> SeatId {
        qa_core::identity::IdentityOwner::create("test").unwrap().seat(0)
    }

    #[test]
    fn capture_root_follows_console_root() {
        let root = std::env::temp_dir().join("qa-capture-root");
        assert_eq!(application_capture_root(Some(root.as_path())), root.join("console"));
    }

    #[test]
    fn configuration_text_walks_to_local_seat() {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let mut services = StubServices {
            root: std::env::temp_dir(),
            lines,
            consoles: vec![seat()],
            bindings: vec!["bind a +attack".to_string()],
            archived: vec!["seta volume 1".to_string()],
        };
        let origin = CaptureCommandOrigin::Script {
            caller: Box::new(CaptureCommandOrigin::LocalSeat { seat: seat() }),
        };
        let text = configuration_text(&mut services, &origin).unwrap().unwrap();
        assert_eq!(
            text,
            "// Generated by quake-typescript\nbind a +attack\nseta volume 1\n"
        );
        let none = configuration_text(&mut services, &CaptureCommandOrigin::Other).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn frame_capture_writes_tga_and_reports() {
        let dir: PathBuf = std::env::temp_dir().join(format!("qa-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let services = StubServices {
            root: dir.clone(),
            lines: Rc::clone(&lines),
            consoles: vec![seat()],
            bindings: Vec::new(),
            archived: Vec::new(),
        };
        let mut capture = ApplicationCapture::new(services, StubRenderer { frames: 0 });
        capture
            .activate(&mut StubRegistrar {
                bound: false,
                unregistered: Rc::new(Cell::new(false)),
            })
            .unwrap();
        assert!(!capture.pending_readback());
        capture.capture_frame();
        capture.drain();
        assert!(dir.join("screenshots").join("shot0000.tga").exists());
        assert!(lines.borrow().is_empty());
        capture.close();
        capture.queue(Box::new(|| Ok(())), false);
        assert_eq!(lines.borrow().as_slice(), ["Console output owner is closed\n"]);
    }

    #[test]
    fn failing_operations_print_and_reject_closed_activate() {
        let dir: PathBuf = std::env::temp_dir().join(format!("qa-capture-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let services = StubServices {
            root: dir,
            lines: Rc::clone(&lines),
            consoles: Vec::new(),
            bindings: Vec::new(),
            archived: Vec::new(),
        };
        let mut capture = ApplicationCapture::new(services, StubRenderer { frames: 0 });
        capture.queue(Box::new(|| Err("boom".to_string())), false);
        assert_eq!(lines.borrow().as_slice(), ["Console output failed: boom\n"]);
        capture.close();
        let error = capture
            .activate(&mut StubRegistrar {
                bound: false,
                unregistered: Rc::new(Cell::new(false)),
            })
            .unwrap_err();
        assert!(matches!(error, CaptureError::Closed));
    }

    #[test]
    fn input_services_adapt_locals() {
        struct StubInput {
            locals: Vec<CaptureInputLocal>,
        }

        impl CaptureInput for StubInput {
            type Error = CaptureError;

            fn archive_commands(&mut self, _origin: &CaptureCommandOrigin) -> Result<Vec<String>, CaptureError> {
                Ok(vec!["seta skill 2".to_string()])
            }

            fn locals(&self) -> Vec<CaptureInputLocal> {
                self.locals.clone()
            }

            fn chat_capability(&self) -> bool {
                false
            }

            fn write_script(&mut self, operation: Box<dyn FnOnce() -> Result<(), String>>) -> Result<(), String> {
                operation()
            }
        }

        let printed = Rc::new(RefCell::new(Vec::new()));
        let printed_callback = Rc::clone(&printed);
        let local = seat();
        let mut services = input_capture_services(
            StubInput {
                locals: vec![CaptureInputLocal {
                    seat: local.clone(),
                    has_console: true,
                    bindings: vec!["unbindall".to_string()],
                }],
            },
            PathBuf::from("root"),
            Box::new(|| "maps/e1m1.bsp".to_string()),
            Box::new(move |text| printed_callback.borrow_mut().push(text.to_string())),
        );
        assert!(services.has_console(&local));
        assert!(!services.can_chat());
        assert_eq!(services.map_name(), "maps/e1m1.bsp");
        assert_eq!(services.bindings_text(&local).unwrap(), vec!["unbindall".to_string()]);
        services.print("hi");
        assert_eq!(printed.borrow().as_slice(), ["hi"]);
    }
}
