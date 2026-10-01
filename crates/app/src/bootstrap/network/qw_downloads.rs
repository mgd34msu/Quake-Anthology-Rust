//! QuakeWorld client download receiver (donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/qw-downloads.ts`).
//!
//! The donor is `async`; this sync port resolves every host call inline.
//! [`QwDownloadReceiver`] also implements [`QwApplicationDownloads`] so the
//! QuakeWorld client can plug it in directly; the trait is infallible, so the
//! trait methods log the donor error through `print` and degrade to the
//! closest queue-advancing verdict (`Skipped` for requests, `Missing` for
//! chunks). Callers that need the exact donor failure use the inherent
//! fallible [`QwDownloadReceiver::request`] and [`QwDownloadReceiver::receive`].

use std::path::Path;

use qa_net::q1_net::QwDownload;
use qa_net::services::downloads::{
    download_path, DownloadError, DownloadSink, ProtocolDownloadExpectation, SinkExpectation,
};
use thiserror::Error;

use super::client_download_policy::{
    ClientDownloadCategory, ClientDownloadPhase, ClientDownloadProgress, ClientDownloadRequest, ClientDownloadTransport,
};
use super::qw_types::{QwApplicationDownloads, QwDownloadCategory, QwDownloadReceive, QwDownloadRequest};

/// Default maximum staged bytes (donor `64 * 1024 * 1024`).
const DEFAULT_MAXIMUM_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum bytes per download block (donor `768`).
const MAX_BLOCK_BYTES: usize = 768;

/// QuakeWorld download failure.
#[derive(Debug, Error)]
pub enum QwDownloadError {
    /// Donor failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Download path validation failure.
    #[error(transparent)]
    Path(#[from] DownloadError),
}

/// Mounted-file lookup (donor `QwDownloadOptions['exists']`).
pub type QwDownloadExists = dyn Fn(&str, QwDownloadCategory) -> bool;

/// Client command sender (donor `QwDownloadOptions['sendCommand']`).
pub type QwDownloadSendCommand = dyn FnMut(&str);

/// Text printer (donor `QwDownloadOptions['print']`).
pub type QwDownloadPrint = dyn FnMut(&str);

/// Cvar/flag reader (donor `QwDownloadOptions` flag closures).
pub type QwDownloadFlag<T> = dyn Fn() -> T;

/// Local permission closure (donor `QwDownloadOptions['permission']`).
pub type QwDownloadPermission = dyn Fn(ClientDownloadRequest) -> bool;

/// QuakeWorld download options (donor `QwDownloadOptions`).
pub struct QwDownloadOptions {
    /// Provisioned writable game directory.
    pub game_root: String,
    /// Shared writable qw directory; skin names keep their `skins/` prefix.
    pub skin_root: String,
    /// Mounted-file lookup supplied by the caller.
    pub exists: Box<QwDownloadExists>,
    /// Send a client string command.
    pub send_command: Box<QwDownloadSendCommand>,
    /// Print text.
    pub print: Box<QwDownloadPrint>,
    /// `noskins` cvar value.
    pub noskins: Box<QwDownloadFlag<i32>>,
    /// Whether a demo is recording.
    pub demo_recording: Box<QwDownloadFlag<bool>>,
    /// Whether a demo is playing back.
    pub demo_playback: Box<QwDownloadFlag<bool>>,
    /// Maximum staged bytes.
    pub maximum_bytes: Option<u64>,
    /// Local permission closure.
    pub permission: Option<Box<QwDownloadPermission>>,
}

/// Paused transfer resumption (donor `QwDownloadReceiver['paused']`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PausedTransfer {
    /// Requested path.
    path: String,
    /// Download category.
    category: QwDownloadCategory,
}

/// In-flight transfer (donor `Transfer`).
struct ActiveTransfer {
    /// Download category.
    category: QwDownloadCategory,
    /// Requested path.
    path: String,
    /// Staging root.
    root: String,
    /// Staged sink, created on the first block.
    sink: Option<DownloadSink>,
    /// Last accepted percent.
    percent: u8,
}

/// QuakeWorld download receiver (donor `QwDownloadReceiver`).
pub struct QwDownloadReceiver {
    options: QwDownloadOptions,
    transfer: Option<ActiveTransfer>,
    generation: u64,
    waiting_block: bool,
    paused: Option<PausedTransfer>,
    resume_requested: bool,
}

impl QwDownloadReceiver {
    /// Build a receiver.
    pub fn new(options: QwDownloadOptions) -> Self {
        Self {
            options,
            transfer: None,
            generation: 0,
            waiting_block: false,
            paused: None,
            resume_requested: false,
        }
    }

    /// Pause the active transfer for a later [`Self::retry`] (`cancel`).
    pub fn cancel(&mut self) {
        if self.paused.is_some() {
            return;
        }
        let paused = self.transfer.as_ref().map(|transfer| PausedTransfer {
            path: transfer.path.clone(),
            category: transfer.category,
        });
        let waiting = self.waiting_block;
        self.close_sink_only();
        self.close_state();
        if paused.is_some() {
            self.paused = paused;
            self.waiting_block = waiting;
        }
    }

    /// Resume a paused transfer (`retry`).
    pub fn retry(&mut self) -> Result<QwDownloadRequest, QwDownloadError> {
        let Some(paused) = self.paused.clone() else {
            return Ok(QwDownloadRequest::Skipped);
        };
        if self.waiting_block {
            self.resume_requested = true;
            return Ok(QwDownloadRequest::Waiting);
        }
        self.paused = None;
        self.resume_requested = false;
        self.request(&paused.path, paused.category)
    }

    /// In-flight transfer progress (`progress`).
    pub fn progress(&self) -> Vec<ClientDownloadProgress> {
        self.transfer.as_ref().map_or_else(Vec::new, |transfer| {
            vec![ClientDownloadProgress {
                path: transfer.path.clone(),
                transport: ClientDownloadTransport::Native,
                received: transfer.sink.as_ref().map_or(0, DownloadSink::byte_length),
                total: None,
                percent: Some(f64::from(transfer.percent)),
                phase: ClientDownloadPhase::Running,
            }]
        })
    }

    /// Request a download (`request`).
    pub fn request(&mut self, path: &str, category: QwDownloadCategory) -> Result<QwDownloadRequest, QwDownloadError> {
        if self.transfer.is_some() {
            return Err(QwDownloadError::Message(
                "QuakeWorld download already active".to_string(),
            ));
        }
        if category == QwDownloadCategory::Model && path.starts_with('*') {
            return Ok(QwDownloadRequest::Skipped);
        }
        download_path(path)?;
        if !valid_download_path(path, category) {
            return Err(QwDownloadError::Message(
                "Invalid QuakeWorld download path or category".to_string(),
            ));
        }
        let generation = self.generation;
        if (self.options.exists)(path, category) {
            return Ok(if generation == self.generation {
                QwDownloadRequest::Available
            } else {
                QwDownloadRequest::Skipped
            });
        }
        if generation != self.generation {
            return Ok(QwDownloadRequest::Skipped);
        }
        let request_category = match category {
            QwDownloadCategory::Skin => ClientDownloadCategory::Player,
            _ if path.starts_with("maps/") => ClientDownloadCategory::Map,
            QwDownloadCategory::Sound => ClientDownloadCategory::Sound,
            QwDownloadCategory::Model => ClientDownloadCategory::Model,
        };
        if self.options.permission.as_ref().is_some_and(|permission| {
            !permission(ClientDownloadRequest {
                transport: ClientDownloadTransport::Native,
                category: request_category,
            })
        }) {
            (self.options.print)(&format!("Skipping QuakeWorld download {path}: local permission\n"));
            return Ok(QwDownloadRequest::Skipped);
        }
        if (self.options.demo_recording)()
            || (self.options.demo_playback)()
            || (category == QwDownloadCategory::Skin && (self.options.noskins)() != 0)
        {
            (self.options.print)(&format!("Skipping QuakeWorld download {path}: demo or skin policy\n"));
            return Ok(QwDownloadRequest::Skipped);
        }
        let root = if category == QwDownloadCategory::Skin {
            self.options.skin_root.clone()
        } else {
            self.options.game_root.clone()
        };
        self.transfer = Some(ActiveTransfer {
            category,
            path: path.to_string(),
            root,
            sink: None,
            percent: 0,
        });
        self.waiting_block = true;
        (self.options.send_command)(&format!("download {path}"));
        Ok(QwDownloadRequest::Waiting)
    }

    /// Receive a download chunk (`receive`).
    pub fn receive(&mut self, result: &QwDownload) -> Result<QwDownloadReceive, QwDownloadError> {
        if (self.options.demo_playback)() {
            self.close();
            return Ok(QwDownloadReceive::Waiting);
        }
        self.waiting_block = false;
        if self.paused.is_some() {
            if self.resume_requested {
                return match self.retry()? {
                    QwDownloadRequest::Waiting => Ok(QwDownloadReceive::Waiting),
                    QwDownloadRequest::Available => Ok(QwDownloadReceive::Complete),
                    QwDownloadRequest::Skipped => Ok(QwDownloadReceive::Missing),
                };
            }
            return Ok(QwDownloadReceive::Waiting);
        }
        let (path, percent) = self
            .transfer
            .as_ref()
            .map(|transfer| (transfer.path.clone(), transfer.percent))
            .ok_or_else(|| QwDownloadError::Message("Unsolicited QuakeWorld download".to_string()))?;
        let QwDownload::Data { percent: next, bytes } = result else {
            (self.options.print)(&format!("QuakeWorld file not found: {path}\n"));
            self.close();
            return Ok(QwDownloadReceive::Missing);
        };
        if *next < percent || *next > 100 || bytes.len() > MAX_BLOCK_BYTES {
            self.close();
            return Err(QwDownloadError::Message(
                "Invalid QuakeWorld download block".to_string(),
            ));
        }
        let receive_category = if path.starts_with("skins/") {
            ClientDownloadCategory::Player
        } else if path.starts_with("sound/") {
            ClientDownloadCategory::Sound
        } else if path.starts_with("maps/") {
            ClientDownloadCategory::Map
        } else {
            ClientDownloadCategory::Model
        };
        if self.options.permission.as_ref().is_some_and(|permission| {
            !permission(ClientDownloadRequest {
                transport: ClientDownloadTransport::Native,
                category: receive_category,
            })
        }) {
            self.close();
            return Ok(QwDownloadReceive::Missing);
        }
        let maximum_bytes = self.options.maximum_bytes.unwrap_or(DEFAULT_MAXIMUM_BYTES);
        let outcome = self.append_block(*next, bytes, maximum_bytes);
        match outcome {
            Ok(verdict) => Ok(verdict),
            Err(error) => {
                self.close();
                (self.options.print)(&format!("QuakeWorld download failed for {path}: {error}\n"));
                Ok(QwDownloadReceive::Missing)
            }
        }
    }

    /// Append a validated block to the staged sink.
    fn append_block(
        &mut self,
        next: u8,
        bytes: &[u8],
        maximum_bytes: u64,
    ) -> Result<QwDownloadReceive, QwDownloadError> {
        let transfer = self
            .transfer
            .as_mut()
            .ok_or_else(|| QwDownloadError::Message("Unsolicited QuakeWorld download".to_string()))?;
        if transfer.sink.is_none() {
            let sink = DownloadSink::create(
                Path::new(&transfer.root),
                &transfer.path,
                SinkExpectation::Protocol(ProtocolDownloadExpectation { maximum_bytes }),
            )
            .map_err(|error| QwDownloadError::Message(error.to_string()))?;
            transfer.sink = Some(sink);
        }
        let sink = transfer.sink.as_mut().expect("sink created above");
        sink.append(bytes)
            .map_err(|error| QwDownloadError::Message(error.to_string()))?;
        transfer.percent = next;
        if next != 100 {
            self.waiting_block = true;
            (self.options.send_command)("nextdl");
            return Ok(QwDownloadReceive::Waiting);
        }
        sink.finish()
            .map_err(|error| QwDownloadError::Message(error.to_string()))?;
        self.transfer = None;
        Ok(QwDownloadReceive::Complete)
    }

    /// Retire the receiver (`close`).
    pub fn close(&mut self) {
        self.close_sink_only();
        self.close_state();
    }

    /// Close the staged sink without touching the generation.
    fn close_sink_only(&mut self) {
        if let Some(transfer) = self.transfer.as_mut() {
            if let Some(sink) = transfer.sink.as_mut() {
                sink.close();
            }
        }
    }

    /// Reset receiver state and advance the generation.
    fn close_state(&mut self) {
        self.paused = None;
        self.resume_requested = false;
        self.waiting_block = false;
        self.generation += 1;
        self.transfer = None;
    }
}

impl QwApplicationDownloads for QwDownloadReceiver {
    fn request(&mut self, path: &str, category: QwDownloadCategory) -> QwDownloadRequest {
        match self.request(path, category) {
            Ok(verdict) => verdict,
            Err(error) => {
                (self.options.print)(&format!("{error}\n"));
                QwDownloadRequest::Skipped
            }
        }
    }

    fn receive(&mut self, result: &QwDownload) -> QwDownloadReceive {
        match self.receive(result) {
            Ok(verdict) => verdict,
            Err(error) => {
                (self.options.print)(&format!("{error}\n"));
                QwDownloadReceive::Missing
            }
        }
    }

    fn close(&mut self) {
        self.close();
    }
}

/// Validate a download path and category (donor inline checks).
fn valid_download_path(path: &str, category: QwDownloadCategory) -> bool {
    if path.is_empty()
        || !path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'.' | b'/' | b'-'))
        || !path.contains('/')
        || path.contains("..")
    {
        return false;
    }
    match category {
        QwDownloadCategory::Skin => path.starts_with("skins/"),
        QwDownloadCategory::Sound => path.starts_with("sound/"),
        QwDownloadCategory::Model => !(path.starts_with("skins/") || path.starts_with("sound/")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    struct Harness {
        commands: Rc<RefCell<Vec<String>>>,
        printed: Rc<RefCell<Vec<String>>>,
    }

    fn fixture(root: &Path, exists: bool, permission: Option<bool>) -> (Harness, QwDownloadOptions) {
        let commands = Rc::new(RefCell::new(Vec::new()));
        let printed = Rc::new(RefCell::new(Vec::new()));
        let send_commands = Rc::clone(&commands);
        let send_printed = Rc::clone(&printed);
        let root = root.to_path_buf();
        (
            Harness { commands, printed },
            QwDownloadOptions {
                game_root: root.join("qw").to_string_lossy().into_owned(),
                skin_root: root.join("skins").to_string_lossy().into_owned(),
                exists: Box::new(move |_, _| exists),
                send_command: Box::new(move |text: &str| {
                    send_commands.borrow_mut().push(text.to_string());
                }),
                print: Box::new(move |text: &str| {
                    send_printed.borrow_mut().push(text.to_string());
                }),
                noskins: Box::new(|| 0),
                demo_recording: Box::new(|| false),
                demo_playback: Box::new(|| false),
                maximum_bytes: None,
                permission: permission.map(|allowed| {
                    let boxed: Box<QwDownloadPermission> = Box::new(move |_| allowed);
                    boxed
                }),
            },
        )
    }

    fn test_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("qa-app-qw-downloads-{name}-{}", std::process::id()))
    }

    #[test]
    fn request_validates_path_and_category() {
        let root = test_root("validate");
        let (_harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        for (path, category) in [
            ("noslash", QwDownloadCategory::Sound),
            ("sound/a b.wav", QwDownloadCategory::Sound),
            ("sound/x.wav", QwDownloadCategory::Skin),
            ("skins/x.pcx", QwDownloadCategory::Sound),
            ("skins/x.pcx", QwDownloadCategory::Model),
            ("sound/x.wav", QwDownloadCategory::Model),
        ] {
            let error = receiver.request(path, category).expect_err("invalid");
            assert_eq!(error.to_string(), "Invalid QuakeWorld download path or category");
        }
        // Escapes fail inside downloadPath, before the category check.
        let error = receiver
            .request("sound/../x.wav", QwDownloadCategory::Sound)
            .expect_err("escape");
        assert_eq!(
            error.to_string(),
            "File path must be relative and stay inside its storage directory"
        );
        assert_eq!(
            receiver.request("*1", QwDownloadCategory::Model).expect("brush"),
            QwDownloadRequest::Skipped
        );
    }

    #[test]
    fn request_serves_existing_and_guards_overlap() {
        let root = test_root("existing");
        let (_harness, options) = fixture(&root, true, None);
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(
            receiver
                .request("sound/x.wav", QwDownloadCategory::Sound)
                .expect("exists"),
            QwDownloadRequest::Available
        );

        let root = test_root("overlap");
        let (harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(
            receiver
                .request("sound/x.wav", QwDownloadCategory::Sound)
                .expect("wait"),
            QwDownloadRequest::Waiting
        );
        assert_eq!(harness.commands.borrow().as_slice(), ["download sound/x.wav"]);
        let error = receiver
            .request("sound/y.wav", QwDownloadCategory::Sound)
            .expect_err("overlap");
        assert_eq!(error.to_string(), "QuakeWorld download already active");
        assert_eq!(receiver.progress().len(), 1);
        let progress = &receiver.progress()[0];
        assert_eq!(progress.path, "sound/x.wav");
        assert_eq!(progress.transport, ClientDownloadTransport::Native);
        assert_eq!(progress.phase, ClientDownloadPhase::Running);
    }

    #[test]
    fn request_applies_permission_and_demo_policy() {
        let root = test_root("permission");
        let (harness, options) = fixture(&root, false, Some(false));
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(
            receiver
                .request("sound/x.wav", QwDownloadCategory::Sound)
                .expect("denied"),
            QwDownloadRequest::Skipped
        );
        assert!(harness.printed.borrow()[0].contains("local permission"));

        let root = test_root("demo");
        let (_harness, mut options) = fixture(&root, false, None);
        options.demo_playback = Box::new(|| true);
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(
            receiver
                .request("sound/x.wav", QwDownloadCategory::Sound)
                .expect("demo"),
            QwDownloadRequest::Skipped
        );
    }

    #[test]
    fn receive_completes_staged_file() {
        let root = test_root("complete");
        let (harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(
            receiver
                .request("sound/x.wav", QwDownloadCategory::Sound)
                .expect("wait"),
            QwDownloadRequest::Waiting
        );
        let first = QwDownload::Data {
            percent: 50,
            bytes: vec![1, 2, 3],
        };
        assert_eq!(receiver.receive(&first).expect("block"), QwDownloadReceive::Waiting);
        assert_eq!(receiver.progress()[0].received, 3);
        assert_eq!(harness.commands.borrow().as_slice(), ["download sound/x.wav", "nextdl"]);
        let last = QwDownload::Data {
            percent: 100,
            bytes: vec![4, 5],
        };
        assert_eq!(receiver.receive(&last).expect("done"), QwDownloadReceive::Complete);
        assert!(receiver.progress().is_empty());
        let staged = root.join("qw").join("sound/x.wav");
        assert_eq!(std::fs::read(&staged).expect("staged"), vec![1, 2, 3, 4, 5]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn receive_rejects_regressed_and_oversize_blocks() {
        let root = test_root("regress");
        let (_harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        receiver
            .request("sound/x.wav", QwDownloadCategory::Sound)
            .expect("wait");
        receiver
            .receive(&QwDownload::Data {
                percent: 50,
                bytes: vec![1],
            })
            .expect("first");
        let error = receiver
            .receive(&QwDownload::Data {
                percent: 40,
                bytes: vec![2],
            })
            .expect_err("regressed");
        assert_eq!(error.to_string(), "Invalid QuakeWorld download block");

        let root = test_root("oversize");
        let (_harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        receiver
            .request("sound/x.wav", QwDownloadCategory::Sound)
            .expect("wait");
        let error = receiver
            .receive(&QwDownload::Data {
                percent: 10,
                bytes: vec![0; 769],
            })
            .expect_err("oversize");
        assert_eq!(error.to_string(), "Invalid QuakeWorld download block");
    }

    #[test]
    fn receive_reports_missing_and_solicits() {
        let root = test_root("missing");
        let (harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        receiver
            .request("sound/x.wav", QwDownloadCategory::Sound)
            .expect("wait");
        assert_eq!(
            receiver.receive(&QwDownload::Missing).expect("missing"),
            QwDownloadReceive::Missing
        );
        assert!(harness.printed.borrow()[0].contains("QuakeWorld file not found"));

        let root = test_root("unsolicited");
        let (_harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        let error = receiver
            .receive(&QwDownload::Data {
                percent: 100,
                bytes: Vec::new(),
            })
            .expect_err("unsolicited");
        assert_eq!(error.to_string(), "Unsolicited QuakeWorld download");
    }

    #[test]
    fn cancel_and_retry_round_trip() {
        let root = test_root("cancel");
        let (_harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(receiver.retry().expect("idle"), QwDownloadRequest::Skipped);
        receiver
            .request("sound/x.wav", QwDownloadCategory::Sound)
            .expect("wait");
        receiver.cancel();
        assert!(receiver.progress().is_empty());
        // Still waiting on a block: retry parks until the chunk arrives.
        assert_eq!(receiver.retry().expect("parked"), QwDownloadRequest::Waiting);
        // A parked chunk reissues the request and stays waiting.
        let verdict = receiver
            .receive(&QwDownload::Data {
                percent: 100,
                bytes: vec![9],
            })
            .expect("resumed");
        assert_eq!(verdict, QwDownloadReceive::Waiting);
        // The parked retry reissued the request; the next chunk completes it.
        let verdict = receiver
            .receive(&QwDownload::Data {
                percent: 100,
                bytes: vec![9],
            })
            .expect("done");
        assert_eq!(verdict, QwDownloadReceive::Complete);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn application_downloads_trait_degrades_with_log() {
        let root = test_root("trait");
        let (harness, options) = fixture(&root, false, None);
        let mut receiver = QwDownloadReceiver::new(options);
        assert_eq!(
            QwApplicationDownloads::receive(
                &mut receiver,
                &QwDownload::Data {
                    percent: 100,
                    bytes: Vec::new()
                }
            ),
            QwDownloadReceive::Missing
        );
        assert!(harness.printed.borrow()[0].contains("Unsolicited QuakeWorld download"));
        assert_eq!(
            QwApplicationDownloads::request(&mut receiver, "noslash", QwDownloadCategory::Sound),
            QwDownloadRequest::Skipped
        );
        QwApplicationDownloads::close(&mut receiver);
    }
}
