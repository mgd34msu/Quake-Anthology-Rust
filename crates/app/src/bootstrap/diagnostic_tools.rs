//! Application diagnostic console commands and the capture clock.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/diagnostic-tools.ts`
//! (`registerDiagnosticTools`, `sourceCaptureFrame`,
//! `registerRuntimeDiagnostics`). Synchronous port: the profiler timer,
//! process memory, and frame/host state arrive through caller services, and
//! queued filesystem operations run inline. Handler usage errors print
//! through the invocation printer (handlers cannot fail).

use qa_content::contract::{ContentMount, ResolvedResourceReference};
use qa_content::mounts::{MountError, MountedContent};
use qa_core::cmd_buffer::{BufferError, CommandBuffer, CommandDocumentation, CommandHandler, Invocation};
use qa_core::cvar::CvarRegistry;
use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;

/// Failure to register diagnostic commands.
#[derive(Debug, Error)]
pub enum DiagnosticError {
    /// Command name is taken.
    #[error("{0}")]
    Registered(String),
    /// Command buffer failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
}

/// One profiler stamp.
#[derive(Debug, Clone, PartialEq)]
pub struct TimerStamp {
    /// Stamp time in milliseconds.
    pub milliseconds: f64,
    /// Stamp label.
    pub name: String,
}

/// Profiler timer surface used by the diagnostic commands.
pub trait DiagnosticTimer {
    /// Enable or disable timing.
    fn set_enabled(&mut self, enabled: bool);
    /// Reset timings and stamps.
    fn reset(&mut self);
    /// Format the timing report.
    fn report(&self) -> String;
    /// List stamps.
    fn stamp_list(&self) -> Vec<TimerStamp>;
    /// Record a stamp.
    fn stamp(&mut self, label: &str);
}

/// Shared print sink for diagnostic commands.
type PrintSink = Rc<RefCell<dyn FnMut(&str)>>;

/// Registered diagnostic commands; unregisters on demand.
#[derive(Debug)]
pub struct DiagnosticTools {
    names: Vec<String>,
}

impl DiagnosticTools {
    /// Registered command names.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Unregister every command.
    pub fn unregister(self, commands: &mut CommandBuffer) {
        for name in &self.names {
            commands.unregister(name);
        }
    }
}

fn documentation(summary: &str, usage: &str) -> CommandDocumentation {
    CommandDocumentation {
        summary: summary.to_owned(),
        usage: usage.to_owned(),
        examples: Vec::new(),
        allowed_values: None,
    }
}

fn printer(print: &PrintSink) -> PrintSink {
    Rc::clone(print)
}

/// Registers reports over actual application timing; no fabricated subsystem counters.
pub fn register_diagnostic_tools<T: DiagnosticTimer + 'static>(
    commands: &mut CommandBuffer,
    cvars: &CvarRegistry,
    timer: Rc<RefCell<T>>,
    print: PrintSink,
) -> Result<DiagnosticTools, DiagnosticError> {
    let mut names = Vec::new();
    let mut add = |commands: &mut CommandBuffer,
                   name: &str,
                   usage: &str,
                   handler: CommandHandler|
     -> Result<(), DiagnosticError> {
        if commands.exists(name) {
            return Err(DiagnosticError::Registered(format!(
                "Diagnostic command already registered: {name}"
            )));
        }
        let registered = commands.register(
            name,
            Some(handler),
            Some(documentation("Inspect application profiler timings.", usage)),
            cvars,
        )?;
        if registered {
            names.push(name.to_owned());
        }
        Ok(())
    };
    {
        let timer = Rc::clone(&timer);
        let print = printer(&print);
        add(
            commands,
            "timers",
            "timers [on|off|reset|report|stamps]",
            Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                let action = invocation
                    .args()
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "report".to_owned());
                let mut timer = timer.borrow_mut();
                match action.as_str() {
                    "on" | "off" => timer.set_enabled(action == "on"),
                    "reset" => timer.reset(),
                    "report" => print.borrow_mut()(&timer.report()),
                    "stamps" => {
                        let mut text = String::from("milliseconds\tname\n");
                        for stamp in timer.stamp_list() {
                            text.push_str(&format!("{:.3}\t{}\n", stamp.milliseconds, stamp.name));
                        }
                        print.borrow_mut()(&text);
                    }
                    _ => invocation.print("Usage: timers [on|off|reset|report|stamps]"),
                }
            }),
        )?;
    }
    {
        let timer = Rc::clone(&timer);
        add(
            commands,
            "timerstamp",
            "timerstamp <label>",
            Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                let label = invocation.args().join(" ");
                if label.is_empty() {
                    invocation.print("Usage: timerstamp <label>");
                } else {
                    timer.borrow_mut().stamp(&label);
                }
            }),
        )?;
    }
    Ok(DiagnosticTools { names })
}

/// Capture-clock outcome for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureFrame {
    /// Frame milliseconds.
    pub milliseconds: f64,
    /// Whether to capture this frame.
    pub capture: bool,
}

/// Capture-clock inputs (`CL_Frame`'s `cl_avidemo` clock inputs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureOptions {
    /// Capture frames per second.
    pub fps: f64,
    /// Timescale.
    pub timescale: f64,
    /// Capture active.
    pub active: bool,
    /// Force capture.
    pub force: bool,
}

/// `CL_Frame`'s `cl_avidemo` clock, shared by every renderer and screenshot format.
pub fn source_capture_frame(elapsed: f64, options: &CaptureOptions) -> Result<CaptureFrame, DiagnosticError> {
    if ![elapsed, options.fps, options.timescale]
        .iter()
        .all(|value| value.is_finite())
        || elapsed < 0.0
        || options.fps < 0.0
        || options.timescale < 0.0
    {
        return Err(DiagnosticError::Registered("Invalid capture clock".to_owned()));
    }
    let fps = options.fps.trunc() as i64;
    if fps == 0 || elapsed == 0.0 {
        return Ok(CaptureFrame {
            milliseconds: elapsed,
            capture: false,
        });
    }
    let step = ((1000.0 / fps as f64).trunc() as f32) * (options.timescale as f32);
    Ok(CaptureFrame {
        milliseconds: (step.trunc() as i64).max(1) as f64,
        capture: options.active || options.force,
    })
}

/// Host process memory in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMemory {
    /// Resident set size.
    pub rss: u64,
    /// Total heap.
    pub heap_total: u64,
    /// Used heap.
    pub heap_used: u64,
    /// External allocations.
    pub external: u64,
    /// Array buffers.
    pub array_buffers: u64,
}

/// Current application frame and source state.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeFrameInfo {
    /// Frame number.
    pub frame: u64,
    /// Frame milliseconds.
    pub milliseconds: f64,
    /// Current map.
    pub map: String,
    /// Renderer name.
    pub renderer: String,
    /// Client count.
    pub clients: u32,
}

/// Runtime diagnostic services.
pub struct RuntimeDiagnosticServices {
    /// Active mounts.
    pub mounts: Rc<MountedContent>,
    /// Current frame info.
    pub frame: Rc<dyn Fn() -> RuntimeFrameInfo>,
    /// Host process memory.
    pub memory: Rc<dyn Fn() -> ProcessMemory>,
    /// Print.
    pub print: PrintSink,
}

fn json_escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            _ => out.push(ch),
        }
    }
}

fn frame_json(frame: &RuntimeFrameInfo) -> String {
    let mut map = String::new();
    json_escape(&frame.map, &mut map);
    let mut renderer = String::new();
    json_escape(&frame.renderer, &mut renderer);
    format!(
        "{{\n  \"frame\": {},\n  \"milliseconds\": {},\n  \"map\": \"{map}\",\n  \"renderer\": \"{renderer}\",\n  \"clients\": {}\n}}\n",
        frame.frame, frame.milliseconds, frame.clients,
    )
}

fn resources_json(resources: &[ResolvedResourceReference]) -> String {
    let mut out = String::from("[\n");
    for (index, resource) in resources.iter().enumerate() {
        let mut id = String::new();
        json_escape(resource.id.as_str(), &mut id);
        let mut path = String::new();
        json_escape(&resource.requested_path, &mut path);
        let mut digest = String::new();
        json_escape(resource.digest.as_str(), &mut digest);
        out.push_str(&format!(
            "  {{\n    \"id\": \"{id}\",\n    \"requestedPath\": \"{path}\",\n    \"digest\": \"{digest}\",\n    \"byteLength\": {}\n  }}{}\n",
            resource.byte_length,
            if index + 1 == resources.len() { "" } else { "," },
        ));
    }
    out.push(']');
    out.push('\n');
    out
}

fn mount_error_text(error: &MountError) -> String {
    error.to_string()
}

/// Host measurements replace allocator-specific C counters; resource queries use the active mount owner.
pub fn register_runtime_diagnostics(
    commands: &mut CommandBuffer,
    cvars: &CvarRegistry,
    services: &RuntimeDiagnosticServices,
) -> Result<DiagnosticTools, DiagnosticError> {
    let mut names = Vec::new();
    let mut add = |commands: &mut CommandBuffer,
                   name: &str,
                   summary: &str,
                   usage: &str,
                   handler: CommandHandler|
     -> Result<(), DiagnosticError> {
        if commands.exists(name) {
            return Err(DiagnosticError::Registered(format!(
                "Diagnostic command already registered: {name}"
            )));
        }
        let registered = commands.register(name, Some(handler), Some(documentation(summary, usage)), cvars)?;
        if registered {
            names.push(name.to_owned());
        }
        Ok(())
    };
    {
        let memory = Rc::clone(&services.memory);
        let print = printer(&services.print);
        add(
            commands,
            "meminfo",
            "Report the actual host process memory in bytes.",
            "meminfo",
            Rc::new(move |_: &mut Invocation<'_, '_, '_>| {
                let memory = memory();
                print.borrow_mut()(&format!(
                    "Host process bytes: rss={} heapTotal={} heapUsed={} external={} arrayBuffers={}\n",
                    memory.rss, memory.heap_total, memory.heap_used, memory.external, memory.array_buffers,
                ));
            }),
        )?;
    }
    {
        let mounts = Rc::clone(&services.mounts);
        let print = printer(&services.print);
        add(
            commands,
            "path",
            "Show active filesystem search order and overrides.",
            "path",
            Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                if let Err(error) = mounts.assert_open() {
                    invocation.print(&mount_error_text(&error));
                    return;
                }
                let describe = |id: &qa_content::contract::MountId| -> Result<String, String> {
                    let mount = mounts
                        .plan
                        .mounts
                        .iter()
                        .find(|item| item.identity().id == *id)
                        .ok_or_else(|| format!("Missing active mount {}", id.as_str()))?;
                    Ok(match mount {
                        ContentMount::Archive(archive) => {
                            format!("{} {}", id.as_str(), archive.archive_path)
                        }
                        ContentMount::Loose(loose) => {
                            format!("{} {}", id.as_str(), loose.root_path)
                        }
                    })
                };
                let mut lines = Vec::new();
                for id in &mounts.plan.default_order {
                    match describe(id) {
                        Ok(line) => lines.push(line),
                        Err(error) => {
                            invocation.print(&error);
                            return;
                        }
                    }
                }
                print.borrow_mut()(&(lines.join("\n") + "\n"));
                for order in &mounts.plan.prefix_orders {
                    let mut prefixed = Vec::new();
                    for id in &order.mounts {
                        match describe(id) {
                            Ok(line) => prefixed.push(line),
                            Err(error) => {
                                invocation.print(&error);
                                return;
                            }
                        }
                    }
                    print.borrow_mut()(&format!("{}:\n{}\n", order.prefix, prefixed.join("\n")));
                }
            }),
        )?;
    }
    {
        let mounts = Rc::clone(&services.mounts);
        let print = printer(&services.print);
        add(
            commands,
            "dir",
            "List files through the active mounted resource owner.",
            "dir [path] [extension]",
            Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                let args = invocation.args().to_vec();
                let path = match args.first().map(String::as_str) {
                    Some(".") => String::new(),
                    Some(path) => path.to_owned(),
                    None => String::new(),
                };
                let extension = args.get(1).cloned().unwrap_or_default();
                match mounts.list_files(&path, &extension) {
                    Ok(files) => print.borrow_mut()(&(files.join("\n") + &format!("\n{} files\n", files.len()))),
                    Err(error) => invocation.print(&mount_error_text(&error)),
                }
            }),
        )?;
    }
    {
        let mounts = Rc::clone(&services.mounts);
        add(
            commands,
            "touchFile",
            "Open a file through the active filesystem and record its actual reference.",
            "touchFile <file>",
            Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                let args = invocation.args().to_vec();
                if args.len() != 1 {
                    invocation.print("Usage: touchFile <file>");
                    return;
                }
                if let Err(error) = mounts.open(&args[0], |_| true) {
                    invocation.print(&mount_error_text(&error));
                }
            }),
        )?;
    }
    {
        let mounts = Rc::clone(&services.mounts);
        let print = printer(&services.print);
        add(
            commands,
            "resourceinfo",
            "Report resources actually opened by the active filesystem.",
            "resourceinfo",
            Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                if let Err(error) = mounts.assert_open() {
                    invocation.print(&mount_error_text(&error));
                    return;
                }
                print.borrow_mut()(&resources_json(&mounts.opened_resources()));
            }),
        )?;
    }
    {
        let frame = Rc::clone(&services.frame);
        let print = printer(&services.print);
        add(
            commands,
            "frameinfo",
            "Report current application frame and source state.",
            "frameinfo",
            Rc::new(move |_: &mut Invocation<'_, '_, '_>| {
                print.borrow_mut()(&frame_json(&frame()));
            }),
        )?;
    }
    Ok(DiagnosticTools { names })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    struct FakeTimer {
        enabled: bool,
        stamps: Vec<TimerStamp>,
    }

    impl DiagnosticTimer for FakeTimer {
        fn set_enabled(&mut self, enabled: bool) {
            self.enabled = enabled;
        }

        fn reset(&mut self) {
            self.stamps.clear();
        }

        fn report(&self) -> String {
            format!("enabled={} stamps={}\n", self.enabled, self.stamps.len())
        }

        fn stamp_list(&self) -> Vec<TimerStamp> {
            self.stamps.clone()
        }

        fn stamp(&mut self, label: &str) {
            self.stamps.push(TimerStamp {
                milliseconds: self.stamps.len() as f64,
                name: label.to_owned(),
            });
        }
    }

    fn buffer() -> (CommandBuffer, CvarRegistry) {
        let context = qa_core::cmd_buffer::CommandContext::new(
            qa_core::identity::IdentityOwner::create("diagnostics")
                .expect("owner")
                .session()
                .clone(),
            qa_core::cmd_buffer::CommandOrigin::LocalConsole,
        );
        let buffer =
            CommandBuffer::new(Dialect::Q3, context, qa_core::cmd_buffer::BufferOptions::default()).expect("buffer");
        (buffer, CvarRegistry::new(Dialect::Q3))
    }

    #[test]
    fn capture_clock_scales_and_gates() {
        let active = CaptureOptions {
            fps: 30.0,
            timescale: 1.0,
            active: true,
            force: false,
        };
        let frame = source_capture_frame(16.0, &active).expect("frame");
        assert_eq!(frame.milliseconds, 33.0);
        assert!(frame.capture);
        let idle = source_capture_frame(
            16.0,
            &CaptureOptions {
                active: false,
                ..active
            },
        )
        .expect("frame");
        assert!(!idle.capture);
        let forced = source_capture_frame(
            16.0,
            &CaptureOptions {
                active: false,
                force: true,
                ..active
            },
        )
        .expect("frame");
        assert!(forced.capture);
        let zero = source_capture_frame(16.0, &CaptureOptions { fps: 0.0, ..active }).expect("frame");
        assert_eq!(zero.milliseconds, 16.0);
        assert!(!zero.capture);
        assert!(source_capture_frame(-1.0, &active).is_err());
        assert!(source_capture_frame(f64::NAN, &active).is_err());
    }

    #[test]
    fn timer_commands_register_report_and_unregister() {
        let (mut commands, cvars) = buffer();
        let timer = Rc::new(RefCell::new(FakeTimer {
            enabled: false,
            stamps: Vec::new(),
        }));
        let printed = Rc::new(RefCell::new(Vec::new()));
        let printed_inner = Rc::clone(&printed);
        let print: PrintSink = Rc::new(RefCell::new(move |text: &str| {
            printed_inner.borrow_mut().push(text.to_owned())
        }));
        let tools = register_diagnostic_tools(&mut commands, &cvars, timer, print).expect("register");
        assert_eq!(tools.names(), &["timers".to_owned(), "timerstamp".to_owned()]);
        assert!(commands.exists("timers"));
        tools.unregister(&mut commands);
        assert!(!commands.exists("timers"));
        assert!(!commands.exists("timerstamp"));
        assert!(printed.borrow().is_empty());
    }

    #[test]
    fn runtime_commands_register_and_report() {
        use qa_content::contract::{ContentId, LooseMount, MountId, MountIdentity, MountPlanId, ResolvedMountPlan};
        use qa_content::mounts::{open_mount_plan, OpenMountOptions};
        let root = std::env::temp_dir().join(format!("qa-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::fs::write(root.join("note.txt"), b"note").expect("write");
        let mount = ContentMount::Loose(LooseMount {
            identity: MountIdentity {
                id: MountId("mount:test:loose".to_owned()),
                content: ContentId("q1:test:base:1".to_owned()),
                generation: 1,
            },
            root_path: root.to_string_lossy().into_owned(),
        });
        let plan = ResolvedMountPlan {
            id: MountPlanId("mount-plan:test:1".to_owned()),
            mounts: vec![mount.clone()],
            default_order: vec![mount.identity().id.clone()],
            prefix_orders: Vec::new(),
        };
        let mounts = Rc::new(
            open_mount_plan(
                &plan,
                OpenMountOptions {
                    pure: None,
                    q3_restriction: None,
                    links: Vec::new(),
                    loose_comparison: None,
                },
            )
            .expect("open"),
        );
        let (mut commands, cvars) = buffer();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let printed_inner = Rc::clone(&printed);
        let services = RuntimeDiagnosticServices {
            mounts,
            frame: Rc::new(|| RuntimeFrameInfo {
                frame: 7,
                milliseconds: 16.0,
                map: "q3dm1".to_owned(),
                renderer: "test".to_owned(),
                clients: 2,
            }),
            memory: Rc::new(|| ProcessMemory {
                rss: 1,
                heap_total: 2,
                heap_used: 3,
                external: 4,
                array_buffers: 5,
            }),
            print: Rc::new(RefCell::new(move |text: &str| {
                printed_inner.borrow_mut().push(text.to_owned());
            })),
        };
        let tools = register_runtime_diagnostics(&mut commands, &cvars, &services).expect("register");
        assert_eq!(tools.names().len(), 6);
        assert!(commands.exists("meminfo"));
        assert!(commands.exists("frameinfo"));
        tools.unregister(&mut commands);
        assert!(!commands.exists("meminfo"));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn duplicate_registration_fails() {
        let (mut commands, cvars) = buffer();
        let timer = Rc::new(RefCell::new(FakeTimer {
            enabled: false,
            stamps: Vec::new(),
        }));
        let print: PrintSink = Rc::new(RefCell::new(|_: &str| {}));
        register_diagnostic_tools(&mut commands, &cvars, Rc::clone(&timer), Rc::clone(&print)).expect("register");
        let err = register_diagnostic_tools(&mut commands, &cvars, timer, print).expect_err("duplicate");
        assert_eq!(err.to_string(), "Diagnostic command already registered: timers");
    }
}
