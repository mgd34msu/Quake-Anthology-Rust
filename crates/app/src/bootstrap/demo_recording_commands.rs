//! Client demo recording commands over retained feeds.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/demo-recording-commands.ts`
//! (`DemoRecordingHost`, `DemoRecordingIntent`, `ClientDemoRecording`).
//! Synchronous port: the host feeds are the local [`DemoRecordingHost`]
//! trait, recordings are shared behind [`SharedRecording`] so feed sinks
//! can fail them, and intents stage through the host.

use qa_core::cmd_buffer::{
    BufferError, CommandBuffer, CommandContext, CommandDocumentation, CommandHandler, CommandOrigin, Invocation,
};
use qa_core::cvar::CvarRegistry;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use thiserror::Error;

use super::demo_recording::{DemoRecording, DemoRecordingError, DemoRecordingPacket, DemoRecordingSeed, RecordingKind};

/// Failure of a demo recording command.
#[derive(Debug, Error)]
pub enum DemoCommandError {
    /// Recording misuse.
    #[error("{0}")]
    Command(String),
    /// Recording failure.
    #[error(transparent)]
    Recording(#[from] DemoRecordingError),
    /// Command buffer failure.
    #[error(transparent)]
    Buffer(#[from] BufferError),
}

/// Shared recording handle: the client and its feed sink fail it together.
pub type SharedRecording = Rc<RefCell<DemoRecording>>;

/// Identity-checked disposer for one sink.
pub type RecordingDetach = Box<dyn FnOnce()>;

/// Feed sink appending packets to one recording.
pub struct RecordingSink {
    /// Append one packet; failures fail the recording.
    pub append: Box<dyn FnMut(&DemoRecordingPacket)>,
}

/// Staged recording intent.
#[derive(Debug, Clone)]
pub enum DemoRecordingIntent {
    /// Record the current session.
    Record {
        /// Demo name, if given.
        name: Option<String>,
        /// Command source.
        source: CommandContext,
    },
    /// Record native Q2 server footage.
    ServerRecord {
        /// Demo name.
        name: String,
        /// Command source.
        source: CommandContext,
    },
    /// Finish server footage.
    ServerStop {
        /// Command source.
        source: CommandContext,
    },
    /// Record a multiview demo.
    MvdRecord {
        /// Demo name.
        name: String,
        /// Command source.
        source: CommandContext,
    },
    /// Reconnect and record the QuakeWorld signon.
    Rerecord {
        /// Demo name.
        name: String,
        /// Command source.
        source: CommandContext,
    },
    /// Finish a multiview demo.
    MvdStop {
        /// Command source.
        source: CommandContext,
    },
    /// Finish the current recording.
    Stop {
        /// Command source.
        source: CommandContext,
    },
}

/// One recording feed: seed plus sink attachment.
pub trait DemoRecordingFeed {
    /// Seed the recording from a command source.
    fn seed(&mut self, source: &CommandContext) -> Result<DemoRecordingSeed, DemoCommandError>;
    /// Attach a sink, returning its disposer.
    fn attach(&mut self, sink: RecordingSink) -> RecordingDetach;
}

/// Recording host: feeds, root, seed, staging, and printing.
pub trait DemoRecordingHost {
    /// Classic Q2 server footage feed, when hosted.
    fn server_feed(&mut self) -> Option<&mut dyn DemoRecordingFeed> {
        let _ = self;
        None
    }

    /// Q2 multiview feed, when hosted.
    fn mvd_feed(&mut self) -> Option<&mut dyn DemoRecordingFeed> {
        let _ = self;
        None
    }

    /// Demo root directory.
    fn root(&self) -> PathBuf;

    /// Seed the session recording from a command source.
    fn seed(&mut self, source: &CommandContext) -> Result<DemoRecordingSeed, DemoCommandError>;

    /// Attach a session sink, returning its disposer.
    fn attach(&mut self, sink: RecordingSink) -> RecordingDetach;

    /// Reconnect to the QuakeWorld server and attach a sink.
    fn reconnect_recording(
        &mut self,
        source: &CommandContext,
        sink: RecordingSink,
    ) -> Result<RecordingDetach, DemoCommandError>;

    /// Print.
    fn print(&mut self, text: &str);

    /// Stage an intent.
    fn stage(&mut self, intent: DemoRecordingIntent);
}

struct ActiveRecording {
    recording: SharedRecording,
    detach: Option<RecordingDetach>,
}

struct ClientInner<H> {
    host: H,
    current: Option<ActiveRecording>,
    current_server: Option<ActiveRecording>,
    busy: bool,
}

/// Lives on the retained client; its feed is detached before source retirement.
pub struct ClientDemoRecording<H> {
    inner: Rc<RefCell<ClientInner<H>>>,
}

impl<H: DemoRecordingHost + 'static> ClientDemoRecording<H> {
    /// Create recording commands over a host.
    pub fn new(host: H) -> Self {
        Self {
            inner: Rc::new(RefCell::new(ClientInner {
                host,
                current: None,
                current_server: None,
                busy: false,
            })),
        }
    }

    /// Current recording path, if any.
    #[must_use]
    pub fn path(&self) -> Option<String> {
        self.inner
            .borrow()
            .current
            .as_ref()
            .map(|current| current.recording.borrow().path().to_string_lossy().into_owned())
    }

    /// Current server footage path, if any.
    #[must_use]
    pub fn server_path(&self) -> Option<String> {
        self.inner
            .borrow()
            .current_server
            .as_ref()
            .map(|current| current.recording.borrow().path().to_string_lossy().into_owned())
    }

    /// Register the recording commands, returning their names for detach.
    pub fn attach(
        &mut self,
        commands: &mut CommandBuffer,
        cvars: &CvarRegistry,
    ) -> Result<RecordingCommands, DemoCommandError> {
        let mut names = Vec::new();
        for name in [
            "record",
            "rerecord",
            "stop",
            "stoprecord",
            "mvdrecord",
            "mvdstop",
            "serverrecord",
            "serverstop",
        ] {
            let inner = Rc::clone(&self.inner);
            let command_name = name.to_owned();
            let handler: CommandHandler = Rc::new(move |invocation: &mut Invocation<'_, '_, '_>| {
                let mut origin = &invocation.source.origin;
                while let CommandOrigin::Script { caller, .. } = origin {
                    origin = caller;
                }
                if matches!(origin, CommandOrigin::RemoteClient { .. }) {
                    inner
                        .borrow_mut()
                        .host
                        .print(&format!("{command_name} is a local recording command.\n"));
                    return;
                }
                if command_name == "record" {
                    if invocation.args().len() > 1 {
                        inner.borrow_mut().host.print("Usage: record [name]\n");
                        return;
                    }
                    let intent = DemoRecordingIntent::Record {
                        name: invocation.args().first().cloned(),
                        source: invocation.source.clone(),
                    };
                    inner.borrow_mut().host.stage(intent);
                } else if command_name == "rerecord" || command_name == "mvdrecord" || command_name == "serverrecord" {
                    let name = invocation.args().first().cloned();
                    match name {
                        Some(filename) if invocation.args().len() == 1 => {
                            let intent = if command_name == "rerecord" {
                                DemoRecordingIntent::Rerecord {
                                    name: filename,
                                    source: invocation.source.clone(),
                                }
                            } else if command_name == "mvdrecord" {
                                DemoRecordingIntent::MvdRecord {
                                    name: filename,
                                    source: invocation.source.clone(),
                                }
                            } else {
                                DemoRecordingIntent::ServerRecord {
                                    name: filename,
                                    source: invocation.source.clone(),
                                }
                            };
                            inner.borrow_mut().host.stage(intent);
                        }
                        _ => {
                            inner
                                .borrow_mut()
                                .host
                                .print(&format!("Usage: {command_name} <name>\n"));
                        }
                    }
                } else if !invocation.args().is_empty() {
                    inner.borrow_mut().host.print(&format!("Usage: {command_name}\n"));
                } else {
                    let intent = if command_name == "serverstop" {
                        DemoRecordingIntent::ServerStop {
                            source: invocation.source.clone(),
                        }
                    } else if command_name == "mvdstop" {
                        DemoRecordingIntent::MvdStop {
                            source: invocation.source.clone(),
                        }
                    } else {
                        DemoRecordingIntent::Stop {
                            source: invocation.source.clone(),
                        }
                    };
                    inner.borrow_mut().host.stage(intent);
                }
            });
            let starts = name == "record" || name == "rerecord" || name == "mvdrecord" || name == "serverrecord";
            let summary = if name == "serverrecord" {
                "Record native Quake II server entity footage."
            } else if name == "mvdrecord" {
                "Record the hosted Quake II world as a native multiview demo."
            } else if name == "rerecord" {
                "Reconnect to the QuakeWorld server and record its signon."
            } else if name == "record" {
                "Record the current session to a demo."
            } else {
                "Finish the current demo recording."
            };
            let usage = if name == "record" {
                "record [name]".to_owned()
            } else if starts {
                format!("{name} <name>")
            } else {
                name.to_owned()
            };
            let example = if starts {
                format!("{name} session1")
            } else {
                name.to_owned()
            };
            let registered = commands.register(
                name,
                Some(handler),
                Some(CommandDocumentation {
                    summary: summary.to_owned(),
                    usage,
                    examples: vec![example],
                    allowed_values: None,
                }),
                cvars,
            )?;
            if registered {
                names.push(name.to_owned());
            } else {
                self.inner.borrow_mut().host.print(&format!(
                    "Recording command {name} is already owned by another command.\n"
                ));
            }
        }
        Ok(RecordingCommands { names })
    }

    fn sink(inner: &Rc<RefCell<ClientInner<H>>>, recording: &SharedRecording) -> RecordingSink {
        let owned = Rc::clone(recording);
        let state = Rc::clone(inner);
        RecordingSink {
            append: Box::new(move |packet| {
                let failed = owned.borrow_mut().append(packet).err().map(|error| error.to_string());
                let Some(message) = failed else {
                    return;
                };
                let mut state = state.borrow_mut();
                let current = state.current.as_mut();
                if current.is_none_or(|current| !Rc::ptr_eq(&current.recording, &owned)) {
                    return;
                }
                let mut current = state.current.take().expect("checked above");
                if let Some(detach) = current.detach.take() {
                    detach();
                }
                let abort = owned.borrow_mut().abort().err().map(|error| error.to_string());
                let _ = abort;
                state.host.print(&format!("Demo recording failed: {message}\n"));
            }),
        }
    }

    fn open(
        inner: &Rc<RefCell<ClientInner<H>>>,
        name: Option<&str>,
        seed: DemoRecordingSeed,
    ) -> Result<DemoRecording, DemoCommandError> {
        let root = inner.borrow().host.root();
        if let Some(name) = name {
            return Ok(DemoRecording::open(&root, name, &seed)?);
        }
        for index in 0..10000 {
            let candidate = format!("demo{index:04}");
            match DemoRecording::open(&root, &candidate, &seed) {
                Ok(recording) => return Ok(recording),
                Err(DemoRecordingError::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(DemoCommandError::Command(
            "No unused demo0000–demo9999 recording name remains".to_owned(),
        ))
    }

    fn finish(inner: &Rc<RefCell<ClientInner<H>>>) -> Result<(), DemoCommandError> {
        let current = inner.borrow_mut().current.take();
        let Some(mut current) = current else {
            return Ok(());
        };
        if let Some(detach) = current.detach.take() {
            detach();
        }
        let path = current.recording.borrow().path().to_string_lossy().into_owned();
        current.recording.borrow_mut().stop()?;
        inner.borrow_mut().host.print(&format!("Completed demo {path}\n"));
        Ok(())
    }

    /// Start recording the current session.
    pub fn start(&mut self, name: Option<&str>, source: &CommandContext) -> Result<(), DemoCommandError> {
        let inner = Rc::clone(&self.inner);
        if inner.borrow().busy {
            return Err(DemoCommandError::Command(
                "Recording operation is already in progress".to_owned(),
            ));
        }
        inner.borrow_mut().busy = true;
        let result = (|| {
            let seed = inner.borrow_mut().host.seed(source)?;
            if name.is_none() && seed.identity.kind() != RecordingKind::Q3 {
                return Err(DemoCommandError::Command("Usage: record <name>".to_owned()));
            }
            Self::finish(&inner)?;
            let recording = Self::open(&inner, name, seed)?;
            let path = recording.path().to_string_lossy().into_owned();
            let shared = Rc::new(RefCell::new(recording));
            let detach = inner.borrow_mut().host.attach(Self::sink(&inner, &shared));
            inner.borrow_mut().current = Some(ActiveRecording {
                recording: shared,
                detach: Some(detach),
            });
            inner.borrow_mut().host.print(&format!("Recording {path}\n"));
            Ok(())
        })();
        if result.is_err() {
            let current = inner.borrow_mut().current.take();
            if let Some(mut current) = current {
                if let Some(detach) = current.detach.take() {
                    detach();
                }
                let _ = current.recording.borrow_mut().abort();
            }
        }
        inner.borrow_mut().busy = false;
        result
    }

    /// Start recording a multiview demo.
    pub fn start_mvd(&mut self, name: &str, source: &CommandContext) -> Result<(), DemoCommandError> {
        let inner = Rc::clone(&self.inner);
        if inner.borrow().busy {
            return Err(DemoCommandError::Command(
                "Recording operation is already in progress".to_owned(),
            ));
        }
        if inner.borrow_mut().host.mvd_feed().is_none() {
            return Err(DemoCommandError::Command(
                "mvdrecord requires a hosted Quake II multiview source".to_owned(),
            ));
        }
        inner.borrow_mut().busy = true;
        let result = (|| {
            let seed = inner
                .borrow_mut()
                .host
                .mvd_feed()
                .expect("checked above")
                .seed(source)?;
            if seed.identity.kind() != RecordingKind::Mvd {
                return Err(DemoCommandError::Command(
                    "Multiview source supplied a different recording format".to_owned(),
                ));
            }
            Self::finish(&inner)?;
            let recording = Self::open(&inner, Some(name), seed)?;
            let path = recording.path().to_string_lossy().into_owned();
            let shared = Rc::new(RefCell::new(recording));
            let detach = inner
                .borrow_mut()
                .host
                .mvd_feed()
                .expect("checked above")
                .attach(Self::sink(&inner, &shared));
            inner.borrow_mut().current = Some(ActiveRecording {
                recording: shared,
                detach: Some(detach),
            });
            inner.borrow_mut().host.print(&format!("Recording {path}\n"));
            Ok(())
        })();
        if result.is_err() {
            let current = inner.borrow_mut().current.take();
            if let Some(mut current) = current {
                if let Some(detach) = current.detach.take() {
                    detach();
                }
                let _ = current.recording.borrow_mut().abort();
            }
        }
        inner.borrow_mut().busy = false;
        result
    }

    /// Start recording native server footage.
    pub fn start_server(&mut self, name: &str, source: &CommandContext) -> Result<(), DemoCommandError> {
        let inner = Rc::clone(&self.inner);
        if inner.borrow().busy {
            return Err(DemoCommandError::Command(
                "Recording operation is already in progress".to_owned(),
            ));
        }
        if inner.borrow().current_server.is_some() {
            return Err(DemoCommandError::Command("Already doing a serverrecord".to_owned()));
        }
        if inner.borrow_mut().host.server_feed().is_none() {
            return Err(DemoCommandError::Command(
                "serverrecord requires a hosted classic Quake II source".to_owned(),
            ));
        }
        inner.borrow_mut().busy = true;
        let result = (|| {
            let seed = inner
                .borrow_mut()
                .host
                .server_feed()
                .expect("checked above")
                .seed(source)?;
            if seed.identity.kind() != RecordingKind::Q2Server {
                return Err(DemoCommandError::Command(
                    "Server source supplied a different recording format".to_owned(),
                ));
            }
            let recording = Self::open(&inner, Some(name), seed)?;
            let path = recording.path().to_string_lossy().into_owned();
            let shared = Rc::new(RefCell::new(recording));
            let owned = Rc::clone(&shared);
            let sink = RecordingSink {
                append: Box::new(move |packet| {
                    let _ = owned.borrow_mut().append(packet);
                }),
            };
            let detach = inner
                .borrow_mut()
                .host
                .server_feed()
                .expect("checked above")
                .attach(sink);
            inner.borrow_mut().current_server = Some(ActiveRecording {
                recording: shared,
                detach: Some(detach),
            });
            inner
                .borrow_mut()
                .host
                .print(&format!("Recording server footage {path}\n"));
            Ok(())
        })();
        if result.is_err() {
            let current = inner.borrow_mut().current_server.take();
            if let Some(mut current) = current {
                if let Some(detach) = current.detach.take() {
                    detach();
                }
                let _ = current.recording.borrow_mut().abort();
            }
        }
        inner.borrow_mut().busy = false;
        result
    }

    /// Finish server footage.
    pub fn stop_server(&mut self) -> Result<(), DemoCommandError> {
        let inner = Rc::clone(&self.inner);
        if inner.borrow().busy {
            return Err(DemoCommandError::Command(
                "Recording operation is already in progress".to_owned(),
            ));
        }
        inner.borrow_mut().busy = true;
        let current = inner.borrow_mut().current_server.take();
        let Some(mut current) = current else {
            inner.borrow_mut().host.print("Not doing a serverrecord.\n");
            inner.borrow_mut().busy = false;
            return Ok(());
        };
        if let Some(detach) = current.detach.take() {
            detach();
        }
        let path = current.recording.borrow().path().to_string_lossy().into_owned();
        let result = current.recording.borrow_mut().stop();
        if result.is_ok() {
            inner
                .borrow_mut()
                .host
                .print(&format!("Completed server footage {path}\n"));
        }
        inner.borrow_mut().busy = false;
        Ok(result?)
    }

    /// Finish every recording.
    pub fn stop_all(&mut self) -> Result<(), DemoCommandError> {
        let result = self.stop();
        if self.inner.borrow().current_server.is_some() {
            let server = self.stop_server();
            result.and(server)
        } else {
            result
        }
    }

    /// Reconnect and record the QuakeWorld signon.
    pub fn rerecord(&mut self, name: &str, source: &CommandContext) -> Result<(), DemoCommandError> {
        let inner = Rc::clone(&self.inner);
        if inner.borrow().busy {
            return Err(DemoCommandError::Command(
                "Recording operation is already in progress".to_owned(),
            ));
        }
        inner.borrow_mut().busy = true;
        let result = (|| {
            let seed = inner.borrow_mut().host.seed(source)?;
            if seed.identity.kind() != RecordingKind::Qw {
                return Err(DemoCommandError::Command(
                    "rerecord requires a QuakeWorld server".to_owned(),
                ));
            }
            let root = inner.borrow().host.root();
            Self::finish(&inner)?;
            let recording = DemoRecording::open(
                &root,
                name,
                &DemoRecordingSeed {
                    identity: seed.identity,
                    packets: vec![DemoRecordingPacket::Qw {
                        record: qa_net::demo::QwDemoRecord::Sequences {
                            seconds: 0.0,
                            outgoing: 0,
                            incoming: 0,
                        },
                    }],
                },
            )?;
            let path = recording.path().to_string_lossy().into_owned();
            let shared = Rc::new(RefCell::new(recording));
            let detach = inner
                .borrow_mut()
                .host
                .reconnect_recording(source, Self::sink(&inner, &shared))?;
            inner.borrow_mut().current = Some(ActiveRecording {
                recording: shared,
                detach: Some(detach),
            });
            inner.borrow_mut().host.print(&format!("Recording {path}\n"));
            Ok(())
        })();
        if result.is_err() {
            let current = inner.borrow_mut().current.take();
            if let Some(mut current) = current {
                if let Some(detach) = current.detach.take() {
                    detach();
                }
                let _ = current.recording.borrow_mut().abort();
            }
        }
        inner.borrow_mut().busy = false;
        result
    }

    /// Finish a multiview demo.
    pub fn stop_mvd(&mut self) -> Result<(), DemoCommandError> {
        let recording_mvd = self
            .inner
            .borrow()
            .current
            .as_ref()
            .is_some_and(|current| current.recording.borrow().identity().kind() == RecordingKind::Mvd);
        if !recording_mvd {
            self.inner.borrow_mut().host.print("Not recording a multiview demo.\n");
            return Ok(());
        }
        self.stop()
    }

    /// Finish the current recording.
    pub fn stop(&mut self) -> Result<(), DemoCommandError> {
        let inner = Rc::clone(&self.inner);
        if inner.borrow().busy {
            return Err(DemoCommandError::Command(
                "Recording operation is already in progress".to_owned(),
            ));
        }
        inner.borrow_mut().busy = true;
        let result = Self::finish(&inner);
        inner.borrow_mut().busy = false;
        result
    }
}

/// Registered recording commands; unregisters on demand.
#[derive(Debug)]
pub struct RecordingCommands {
    names: Vec<String>,
}

impl RecordingCommands {
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

#[cfg(test)]
mod tests {
    use super::super::demo_recording::DemoRecordingIdentity;
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    struct FakeHost {
        root: PathBuf,
        printed: Vec<String>,
        staged: Vec<DemoRecordingIntent>,
        sinks: usize,
    }

    impl FakeHost {
        fn seed_packet(&self) -> DemoRecordingPacket {
            DemoRecordingPacket::Q3 {
                sequence: 0,
                message: vec![1, 2, 3],
            }
        }
    }

    impl DemoRecordingHost for FakeHost {
        fn root(&self) -> PathBuf {
            self.root.clone()
        }

        fn seed(&mut self, _source: &CommandContext) -> Result<DemoRecordingSeed, DemoCommandError> {
            Ok(DemoRecordingSeed {
                identity: DemoRecordingIdentity::Q3,
                packets: vec![self.seed_packet()],
            })
        }

        fn attach(&mut self, _sink: RecordingSink) -> RecordingDetach {
            self.sinks += 1;
            Box::new(|| {})
        }

        fn reconnect_recording(
            &mut self,
            _source: &CommandContext,
            _sink: RecordingSink,
        ) -> Result<RecordingDetach, DemoCommandError> {
            Ok(Box::new(|| {}))
        }

        fn print(&mut self, text: &str) {
            self.printed.push(text.to_owned());
        }

        fn stage(&mut self, intent: DemoRecordingIntent) {
            self.staged.push(intent);
        }
    }

    fn context() -> CommandContext {
        CommandContext::new(
            IdentityOwner::create("demos").expect("owner").session().clone(),
            CommandOrigin::LocalConsole,
        )
    }

    fn client(name: &str) -> (ClientDemoRecording<FakeHost>, PathBuf) {
        let root = std::env::temp_dir().join(format!("qa-demo-cmd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let host = FakeHost {
            root: root.clone(),
            printed: Vec::new(),
            staged: Vec::new(),
            sinks: 0,
        };
        (ClientDemoRecording::new(host), root)
    }

    #[test]
    fn start_and_stop_record_named_and_numbered_demos() {
        let (mut client, root) = client("start");
        let source = context();
        assert!(client.path().is_none());
        client.start(Some("session1"), &source).expect("start");
        let path = client.path().expect("path");
        assert!(path.ends_with("session1.dm_68"));
        assert!(client.start(Some("other"), &source).is_ok());
        client.stop().expect("stop");
        assert!(client.path().is_none());
        client.start(None, &source).expect("numbered");
        let numbered = client.path().expect("numbered path");
        assert!(numbered.ends_with("demo0000.dm_68"));
        client.stop_all().expect("stop all");
        assert!(client.path().is_none());
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn missing_feeds_and_wrong_formats_error() {
        let (mut client, root) = client("feeds");
        let source = context();
        let err = client.start_mvd("multi", &source).expect_err("mvd feed");
        assert_eq!(err.to_string(), "mvdrecord requires a hosted Quake II multiview source");
        let err = client.start_server("footage", &source).expect_err("server feed");
        assert_eq!(
            err.to_string(),
            "serverrecord requires a hosted classic Quake II source"
        );
        let err = client.rerecord("qw", &source).expect_err("qw");
        assert_eq!(err.to_string(), "rerecord requires a QuakeWorld server");
        client.stop_mvd().expect("stop mvd");
        std::fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn attaches_and_unregisters_commands() {
        let (mut client, root) = client("attach");
        let context = CommandContext::new(
            IdentityOwner::create("demos").expect("owner").session().clone(),
            CommandOrigin::LocalConsole,
        );
        let mut commands =
            CommandBuffer::new(Dialect::Q3, context, qa_core::cmd_buffer::BufferOptions::default()).expect("buffer");
        let cvars = CvarRegistry::new(Dialect::Q3);
        let registered = client.attach(&mut commands, &cvars).expect("attach");
        assert_eq!(registered.names().len(), 8);
        assert!(commands.exists("mvdrecord"));
        registered.unregister(&mut commands);
        assert!(!commands.exists("mvdrecord"));
        std::fs::remove_dir_all(&root).expect("cleanup");
    }
}
