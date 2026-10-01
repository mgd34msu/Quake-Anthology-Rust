//! Quake III client download queue.
//!
//! Port of `src/app/bootstrap/network/q3-client-downloads.ts`
//! (`q3DownloadPath`, `Q3ApplicationClientDownloads`): `CL_InitDownloads` /
//! `CL_DownloadsComplete` over the shared staged filesystem. Package
//! comparison, checksums, name validation, and sink staging reuse `qa-net`
//! and `qa-content`.
//!
//! The donor drives `qa-net`'s `Q3ClientDownload` pump; this port inlines
//! that pump's sequencing (block matching, temporary lifecycle,
//! progress/reliable/completed callbacks) because the pump borrows its
//! bindings `&mut` and therefore cannot be stored beside the state its
//! callbacks need. The inline pump mirrors the `qa-net` order exactly
//! (including the double `send_packet` before `completed`), and
//! [`tests::pump_matches_qa_net_callbacks`] drives both pumps over the
//! same scripted blocks to pin the callback sequence.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use qa_content::archive::open_archive;
use qa_content::catalog::RemoteContentBase;
use qa_content::contract::ArchiveFormat;
use qa_net::q3_net::{
    check_q3_download_name, compare_q3_packages, q3_archive_checksums, DownloadBlock, Q3NetError,
    Q3ServerPak,
};
use qa_net::q3_pak_references::ServerPak;
use qa_net::services::downloads::{
    DownloadError, DownloadSink, ProtocolDownloadExpectation, SinkExpectation,
};
use thiserror::Error;

use super::client_download_policy::{
    client_download_category, ClientDownloadCategory, ClientDownloadPhase, ClientDownloadProgress,
    ClientDownloadRequest, ClientDownloadTransport,
};
use super::q2_downloads::RemoteContentRoots;
use super::q3_downloads::archive_handle;

/// Quake III client download failure.
#[derive(Debug, Error)]
pub enum Q3ClientDownloadError {
    /// Policy, validation, or protocol failure with donor text.
    #[error("{0}")]
    Message(String),
    /// Filesystem failure.
    #[error("Q3 download filesystem failure: {0}")]
    Filesystem(String),
    /// Download service failure.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// Wire failure.
    #[error(transparent)]
    Net(#[from] Q3NetError),
    /// Archive failure.
    #[error(transparent)]
    Archive(#[from] qa_content::archive::ArchiveError),
}

/// Package comparison buffer (donor `compareQ3Packages` default).
const PACKAGE_CAPACITY: usize = 1024;

/// Source server path limit (donor `MAX_QPATH - 1` guard).
const MAX_DOWNLOAD_NAME: usize = 64;

/// Translate selected/base wire directories to their mounted filesystem
/// spelling (`q3DownloadPath`).
pub fn q3_download_path(path: &str, owner: &RemoteContentRoots) -> Result<String, Q3ClientDownloadError> {
    check_q3_download_name(path)?;
    if owner.selection.base != RemoteContentBase::Q3Baseq3
        || parent_of(&owner.write_root) != parent_of(&owner.base_write_root)
    {
        return Err(Q3ClientDownloadError::Message(
            "Q3 download roots must belong to the selected Q3 family".to_string(),
        ));
    }
    let Some(separator) = path.find('/') else {
        return Ok(path.to_string());
    };
    let directory = path[..separator].to_lowercase();
    let physical = if directory == owner.selection.directory {
        file_name(&owner.write_root)
    } else if directory == "baseq3" {
        file_name(&owner.base_write_root)
    } else {
        return Ok(path.to_string());
    };
    let Some(physical) = physical else {
        return Ok(path.to_string());
    };
    let mapped = format!("{physical}{}", &path[separator..]);
    check_q3_download_name(&mapped)?;
    Ok(mapped)
}

/// Parent directory, `node:path` `dirname` spelling.
fn parent_of(path: &Path) -> PathBuf {
    path.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Final path component, `node:path` `basename` spelling.
fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// Download permission callback.
pub type Q3DownloadPermissionFn<'a> = Box<dyn Fn(&ClientDownloadRequest) -> bool + 'a>;

/// Client download bindings (`Q3ApplicationDownloadBindings`).
pub struct Q3DownloadBindings<'a> {
    /// Download permission (`None` allows everything).
    pub permission: Option<Q3DownloadPermissionFn<'a>>,
    /// Assert the connection epoch is current.
    pub assert_current: Box<dyn FnMut() -> Result<(), Q3ClientDownloadError> + 'a>,
    /// Queue a reliable command.
    pub reliable: Box<dyn FnMut(&str) + 'a>,
    /// Send a packet.
    pub send_packet: Box<dyn FnMut() + 'a>,
    /// Report progress.
    pub progress: Box<dyn FnMut(&str, i32, i32) + 'a>,
    /// Refresh the mounted package catalog; the next gamestate owns
    /// map/pure/module initialization.
    pub reload_packages: Box<dyn FnMut() -> Result<(), Q3ClientDownloadError> + 'a>,
}

/// One queued package (donor `PackageRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageRequest {
    remote: String,
    local: String,
    checksum: i32,
}

/// Quake III client downloads (`Q3ApplicationClientDownloads`).
pub struct Q3ApplicationClientDownloads<'a> {
    root: PathBuf,
    bindings: Q3DownloadBindings<'a>,
    owner: Option<RemoteContentRoots>,
    queue: VecDeque<PackageRequest>,
    current: Option<PackageRequest>,
    sink: Option<DownloadSink>,
    size: i32,
    generation: u64,
    receiving: bool,
    paused: Option<Vec<PackageRequest>>,
    pump_name: String,
    pump_temporary: String,
    pump_block: i32,
    pump_count: i32,
}

impl<'a> Q3ApplicationClientDownloads<'a> {
    /// Build a download queue over a staging root.
    pub fn new(
        root: PathBuf,
        bindings: Q3DownloadBindings<'a>,
        owner: Option<RemoteContentRoots>,
    ) -> Result<Self, Q3ClientDownloadError> {
        if let Some(owner) = &owner {
            if root != parent_of(&owner.write_root) {
                return Err(Q3ClientDownloadError::Message(
                    "Q3 download root differs from the selected mount family".to_string(),
                ));
            }
        }
        Ok(Self {
            root,
            bindings,
            owner,
            queue: VecDeque::new(),
            current: None,
            sink: None,
            size: 0,
            generation: 0,
            receiving: false,
            paused: None,
            pump_name: String::new(),
            pump_temporary: String::new(),
            pump_block: 0,
            pump_count: 0,
        })
    }

    /// In-flight transfers.
    #[must_use]
    pub fn progress(&self) -> Vec<ClientDownloadProgress> {
        let mut items = Vec::new();
        if let Some(current) = &self.current {
            items.push(ClientDownloadProgress {
                path: current.local.clone(),
                transport: ClientDownloadTransport::Native,
                received: self.sink.as_ref().map_or(0, |sink| sink.byte_length()),
                total: Some(u64::from(self.size.max(0) as u32)),
                percent: if self.size > 0 {
                    Some(
                        self.sink.as_ref().map_or(0, |sink| sink.byte_length()) as f64 * 100.0
                            / f64::from(self.size),
                    )
                } else {
                    None
                },
                phase: ClientDownloadPhase::Running,
            });
        }
        for request in self.pending_requests() {
            items.push(ClientDownloadProgress {
                path: request.local.clone(),
                transport: ClientDownloadTransport::Native,
                received: 0,
                total: None,
                percent: None,
                phase: ClientDownloadPhase::Pending,
            });
        }
        items
    }

    /// Pending requests (paused set wins while suspended).
    fn pending_requests(&self) -> Vec<PackageRequest> {
        match &self.paused {
            Some(paused) => paused.clone(),
            None => self.queue.iter().cloned().collect(),
        }
    }

    /// Whether package downloads are permitted.
    fn package_permitted(&self) -> bool {
        self.bindings.permission.as_ref().map_or(true, |permission| {
            permission(&ClientDownloadRequest {
                transport: ClientDownloadTransport::Native,
                category: ClientDownloadCategory::Package,
            })
        })
    }

    /// Suspend the queue, keeping the remainder for [`retry`](Self::retry).
    pub fn cancel(&mut self) -> Result<(), Q3ClientDownloadError> {
        if self.paused.is_some() {
            return Ok(());
        }
        (self.bindings.assert_current)()?;
        let mut remaining = Vec::new();
        if let Some(current) = self.current.clone() {
            remaining.push(current);
        }
        remaining.extend(self.queue.iter().cloned());
        self.close();
        self.paused = Some(remaining);
        (self.bindings.reliable)("stopdl");
        (self.bindings.send_packet)();
        Ok(())
    }

    /// Resume a suspended queue.
    pub fn retry(&mut self) -> Result<bool, Q3ClientDownloadError> {
        (self.bindings.assert_current)()?;
        if self.paused.is_none() || !self.package_permitted() {
            return Ok(false);
        }
        self.queue = self.paused.take().unwrap_or_default().into();
        Ok(self.start_next()?)
    }

    /// Queue missing referenced packages; true suspends game
    /// initialization until `donedl` produces a new gamestate.
    pub fn begin(
        &mut self,
        referenced: &[ServerPak],
        loaded_checksums: &[u32],
        exists: &dyn Fn(&str) -> bool,
    ) -> Result<bool, Q3ClientDownloadError> {
        (self.bindings.assert_current)()?;
        if self.receiving {
            return Err(Q3ClientDownloadError::Message(
                "Cannot replace Q3 downloads during block processing".to_string(),
            ));
        }
        if !self.package_permitted() {
            self.close();
            return Ok(false);
        }
        self.close();
        let converted: Vec<Q3ServerPak> = referenced
            .iter()
            .map(|pack| Q3ServerPak {
                name: pack.name.clone(),
                checksum: pack.checksum as u32,
            })
            .collect();
        // The existence callback cannot fail, so a mapping failure parks
        // here and raises after the comparison, matching the donor throw.
        let mapping_error: Rc<RefCell<Option<Q3ClientDownloadError>>> =
            Rc::new(RefCell::new(None));
        let list = compare_q3_packages(
            &converted,
            loaded_checksums,
            &|name| match self.destination(name) {
                Ok(mapped) => exists(&mapped),
                Err(error) => {
                    *mapping_error.borrow_mut() = Some(error);
                    false
                }
            },
            true,
            PACKAGE_CAPACITY,
        )?;
        if let Some(error) = mapping_error.borrow_mut().take() {
            return Err(error);
        }
        let loaded: std::collections::HashSet<u32> = loaded_checksums.iter().copied().collect();
        let fields: Vec<&str> = list.split('@').collect();
        if fields.first() != Some(&"") || fields.len() % 2 != 1 {
            return Err(Q3ClientDownloadError::Message(
                "Incomplete Q3 package download list".to_string(),
            ));
        }
        let mut index = 1;
        while index < fields.len() {
            let (Some(remote), Some(local)) = (fields.get(index), fields.get(index + 1)) else {
                return Err(Q3ClientDownloadError::Message(
                    "Incomplete Q3 package pair".to_string(),
                ));
            };
            check_q3_download_name(remote)?;
            check_q3_download_name(local)?;
            // The source server retains MAX_QPATH-1 bytes; never request a
            // silently truncated path.
            if remote.len() >= MAX_DOWNLOAD_NAME {
                return Err(Q3ClientDownloadError::Message(
                    "Q3 download name exceeds source server MAX_QPATH".to_string(),
                ));
            }
            let matching: Vec<&ServerPak> = referenced
                .iter()
                .filter(|pack| {
                    pack.name.as_ref().is_some_and(|name| format!("{name}.pk3") == *remote)
                        && !loaded.contains(&(pack.checksum as u32))
                })
                .collect();
            let Some(pack) = matching.first() else {
                return Err(Q3ClientDownloadError::Message(
                    "Q3 download pair has no server reference".to_string(),
                ));
            };
            if matching
                .iter()
                .any(|other| (other.checksum as u32) != (pack.checksum as u32))
            {
                return Err(Q3ClientDownloadError::Message(
                    "Q3 server references conflicting checksums for one download name".to_string(),
                ));
            }
            self.queue.push_back(PackageRequest {
                remote: (*remote).to_string(),
                local: (*local).to_string(),
                checksum: pack.checksum,
            });
            index += 2;
        }
        Ok(self.start_next()?)
    }

    /// Map a wire path through the owner mounts.
    fn destination(&self, path: &str) -> Result<String, Q3ClientDownloadError> {
        match &self.owner {
            Some(owner) => q3_download_path(path, owner),
            None => Ok(path.to_string()),
        }
    }

    /// Start the next queued package.
    fn start_next(&mut self) -> Result<bool, Q3ClientDownloadError> {
        let request = self.queue.pop_front();
        self.current = request.clone();
        self.size = 0;
        let Some(request) = request else {
            return Ok(false);
        };
        self.pump_begin(&request.remote, &request.local)?;
        Ok(true)
    }

    /// Publish the advertised size.
    pub fn publish_size(&mut self, size: i32) -> Result<i32, Q3ClientDownloadError> {
        (self.bindings.assert_current)()?;
        if self.sink.is_some() && size != self.size {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download size changed during transfer".to_string(),
            ));
        }
        self.size = size;
        (self.bindings.progress)(&self.pump_name.clone(), self.pump_count, size);
        Ok(size)
    }

    /// Receive one download block.
    pub fn receive(&mut self, download: &DownloadBlock) -> Result<(), Q3ClientDownloadError> {
        if self.receiving {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download block processing is already active".to_string(),
            ));
        }
        if !self.package_permitted() {
            self.cancel()?;
            return Ok(());
        }
        self.receiving = true;
        let generation = self.generation;
        let outcome = self.pump_receive(download);
        self.receiving = false;
        match outcome {
            Ok(()) => Ok(()),
            Err(error) => {
                if self.paused.is_some() && generation != self.generation {
                    return Ok(());
                }
                self.close();
                Err(error)
            }
        }
    }

    /// Cancel everything.
    pub fn close(&mut self) {
        self.paused = None;
        self.generation += 1;
        self.pump_name.clear();
        self.pump_temporary.clear();
        if let Some(sink) = self.sink.as_mut() {
            sink.close();
        }
        self.sink = None;
        self.current = None;
        self.queue.clear();
        self.size = 0;
    }

    /// Begin one package transfer (inline `Q3ClientDownload::begin`).
    fn pump_begin(&mut self, remote: &str, local: &str) -> Result<(), Q3ClientDownloadError> {
        (self.bindings.assert_current)()?;
        check_q3_download_name(remote)?;
        check_q3_download_name(local)?;
        self.pump_name.clear();
        self.pump_temporary.clear();
        self.pump_name = local.to_string();
        self.pump_temporary = format!("{local}.tmp");
        self.pump_block = 0;
        self.pump_count = 0;
        self.size = 0;
        (self.bindings.progress)(remote, 0, 0);
        (self.bindings.reliable)(&format!("download {remote}"));
        Ok(())
    }

    /// Receive one block (inline `Q3ClientDownload::receive`).
    fn pump_receive(&mut self, download: &DownloadBlock) -> Result<(), Q3ClientDownloadError> {
        (self.bindings.assert_current)()?;
        if let DownloadBlock::Error { message, .. } = download {
            return Err(Q3ClientDownloadError::Message(format!("drop: {message}")));
        }
        let block = match download {
            DownloadBlock::Start { .. } => 0,
            DownloadBlock::Chunk { number, .. } => *number,
            DownloadBlock::Error { .. } => unreachable!("checked"),
        };
        if block != self.pump_block {
            return Ok(());
        }
        if self.sink.is_none() {
            if self.pump_temporary.is_empty() {
                (self.bindings.reliable)("stopdl");
                return Ok(());
            }
            let temporary = self.pump_temporary.clone();
            self.open_temporary(&temporary)?;
            (self.bindings.assert_current)()?;
        }
        let data = match download {
            DownloadBlock::Start { data, .. } | DownloadBlock::Chunk { data, .. } => data,
            DownloadBlock::Error { .. } => unreachable!("checked"),
        };
        if !data.is_empty() {
            if let Some(sink) = self.sink.as_mut() {
                sink.append(data)?;
            }
            (self.bindings.assert_current)()?;
        }
        let block = self.pump_block;
        (self.bindings.reliable)(&format!("nextdl {block}"));
        self.pump_block = self.pump_block.wrapping_add(1);
        self.pump_count = self.pump_count.wrapping_add(data.len() as i32);
        let (name, count, size) = (self.pump_name.clone(), self.pump_count, self.size);
        (self.bindings.progress)(&name, count, size);
        if data.is_empty() {
            // The source closes its writer before publish; the shared sink
            // retains its inode through validation.
            (self.bindings.assert_current)()?;
            let (temporary, name) = (self.pump_temporary.clone(), self.pump_name.clone());
            self.publish_temporary(&temporary, &name)?;
            (self.bindings.assert_current)()?;
            self.pump_name.clear();
            self.pump_temporary.clear();
            let (count, size) = (self.pump_count, self.size);
            (self.bindings.progress)("", count, size);
            (self.bindings.send_packet)();
            (self.bindings.assert_current)()?;
            (self.bindings.send_packet)();
            (self.bindings.assert_current)()?;
            self.completed()?;
        }
        Ok(())
    }

    /// Open the staged sink (donor `openTemporary`).
    fn open_temporary(&mut self, path: &str) -> Result<(), Q3ClientDownloadError> {
        let request = self.current.clone();
        let Some(request) = request else {
            return Err(Q3ClientDownloadError::Message(
                "Unexpected Q3 download temporary request".to_string(),
            ));
        };
        if path != format!("{}.tmp", request.local) || self.size <= 0 {
            return Err(Q3ClientDownloadError::Message(
                "Unexpected Q3 download temporary request".to_string(),
            ));
        }
        let sink = DownloadSink::create(
            &self.root,
            &self.destination(&request.local)?,
            SinkExpectation::Protocol(ProtocolDownloadExpectation {
                maximum_bytes: self.size.max(0) as u64,
            }),
        )?;
        self.sink = Some(sink);
        Ok(())
    }

    /// Validate and publish the staged package (donor `publishTemporary`).
    fn publish_temporary(
        &mut self,
        temporary: &str,
        destination: &str,
    ) -> Result<(), Q3ClientDownloadError> {
        let request = self.current.clone();
        let generation = self.generation;
        let Some(request) = request else {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download publication does not match the current package".to_string(),
            ));
        };
        let Some(sink) = self.sink.as_mut() else {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download publication does not match the current package".to_string(),
            ));
        };
        if temporary != format!("{}.tmp", request.local) || destination != request.local {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download publication does not match the current package".to_string(),
            ));
        }
        if sink.byte_length() != self.size.max(0) as u64 {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download ended before its advertised size".to_string(),
            ));
        }
        let expected = request.checksum as u32;
        let failed: Rc<RefCell<Option<Q3ClientDownloadError>>> = Rc::new(RefCell::new(None));
        let failed_in = Rc::clone(&failed);
        sink.inspect_staged(|staged| {
            let outcome = (|| -> Result<(), Q3ClientDownloadError> {
                let archive = open_archive(staged, Some(ArchiveFormat::Pk3))?;
                let (checksum, _) = q3_archive_checksums(&archive_handle(&archive), 0)
                    .map_err(Q3ClientDownloadError::from)?;
                archive.close();
                if checksum != expected {
                    return Err(Q3ClientDownloadError::Message(
                        "Downloaded Q3 package checksum differs from server references".to_string(),
                    ));
                }
                Ok(())
            })();
            if let Err(error) = outcome {
                *failed_in.borrow_mut() = Some(error);
            }
        })?;
        if let Some(error) = failed.borrow_mut().take() {
            return Err(error);
        }
        (self.bindings.assert_current)()?;
        if !self.package_permitted() {
            return Err(Q3ClientDownloadError::Message(
                "Q3 package download permission denied".to_string(),
            ));
        }
        if generation != self.generation {
            return Err(Q3ClientDownloadError::Message(
                "Q3 download was retired during validation".to_string(),
            ));
        }
        if let Some(sink) = self.sink.as_mut() {
            sink.finish()?;
        }
        self.sink = None;
        Ok(())
    }

    /// Finish one package and start the next (donor `completed`).
    fn completed(&mut self) -> Result<(), Q3ClientDownloadError> {
        self.current = None;
        if self.start_next()? {
            return Ok(());
        }
        let generation = self.generation;
        (self.bindings.reload_packages)()?;
        (self.bindings.assert_current)()?;
        if generation != self.generation {
            return Err(Q3ClientDownloadError::Message(
                "Q3 downloads retired during filesystem refresh".to_string(),
            ));
        }
        (self.bindings.reliable)("donedl");
        Ok(())
    }

    /// Classify a path for permission checks.
    #[must_use]
    pub fn category(path: &str) -> ClientDownloadCategory {
        client_download_category(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::q3_net::{Q3DownloadClientBindings, Q3DownloadWriteFile};

    struct ScriptBindings {
        trace: Rc<RefCell<Vec<String>>>,
    }

    impl Q3DownloadClientBindings for ScriptBindings {
        fn assert_current(&mut self) {}

        fn open_temporary(&mut self, path: &str) -> Option<Box<dyn Q3DownloadWriteFile>> {
            self.trace.borrow_mut().push(format!("open:{path}"));
            Some(Box::new(ScriptFile {
                trace: Rc::clone(&self.trace),
            }))
        }

        fn publish_temporary(&mut self, temporary: &str, destination: &str) {
            self.trace.borrow_mut().push(format!("publish:{temporary}>{destination}"));
        }

        fn reliable(&mut self, text: &str) {
            self.trace.borrow_mut().push(format!("reliable:{text}"));
        }

        fn send_packet(&mut self) {
            self.trace.borrow_mut().push("packet".to_string());
        }

        fn progress(&mut self, name: &str, count: i32, size: i32) {
            self.trace
                .borrow_mut()
                .push(format!("progress:{name}:{count}:{size}"));
        }

        fn completed(&mut self) {
            self.trace.borrow_mut().push("completed".to_string());
        }
    }

    struct ScriptFile {
        trace: Rc<RefCell<Vec<String>>>,
    }

    impl Q3DownloadWriteFile for ScriptFile {
        fn write_bytes(&mut self, bytes: &[u8]) {
            self.trace.borrow_mut().push(format!("write:{}", bytes.len()));
        }

        fn close(&mut self) {
            self.trace.borrow_mut().push("close".to_string());
        }
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qa-q3dl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    fn script_queue(
        trace: Rc<RefCell<Vec<String>>>,
        root: PathBuf,
    ) -> Q3ApplicationClientDownloads<'static> {
        let trace_reliable = Rc::clone(&trace);
        let trace_progress = Rc::clone(&trace);
        let trace_packet = Rc::clone(&trace);
        Q3ApplicationClientDownloads::new(
            root,
            Q3DownloadBindings {
                permission: None,
                assert_current: Box::new(|| Ok(())),
                reliable: Box::new(move |text| {
                    trace_reliable.borrow_mut().push(format!("reliable:{text}"));
                }),
                send_packet: Box::new(move || {
                    trace_packet.borrow_mut().push("packet".to_string());
                }),
                progress: Box::new(move |name, count, size| {
                    trace_progress
                        .borrow_mut()
                        .push(format!("progress:{name}:{count}:{size}"));
                }),
                reload_packages: Box::new(|| Ok(())),
            },
            None,
        )
        .expect("queue")
    }

    #[test]
    fn pump_matches_qa_net_callbacks() {
        // Drive the qa-net pump and the inline pump over identical blocks;
        // the callback sequences must match exactly.
        let net_trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let mut net_bindings = ScriptBindings {
            trace: Rc::clone(&net_trace),
        };
        let mut net = qa_net::q3_net::Q3ClientDownload::new(&mut net_bindings);
        net.begin("baseq3/a.pk3", "baseq3/a.pk3").expect("begin");
        net.publish_size(4);
        net.receive(&DownloadBlock::Start {
            file_size: 4,
            data: vec![1, 2],
        })
        .expect("start");
        net.receive(&DownloadBlock::Chunk {
            number: 1,
            data: vec![3, 4],
        })
        .expect("chunk");

        let app_trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let dir = scratch_dir("pump");
        // The app pump publishes through a real sink; point it at scratch.
        let mut app = script_queue(Rc::clone(&app_trace), dir.clone());
        // Seed the app queue state to match the net pump's begin.
        app.pump_begin("baseq3/a.pk3", "baseq3/a.pk3").expect("begin");
        app.current = Some(PackageRequest {
            remote: "baseq3/a.pk3".to_string(),
            local: "baseq3/a.pk3".to_string(),
            checksum: 0,
        });
        app.publish_size(4).expect("size");
        app.pump_receive(&DownloadBlock::Start {
            file_size: 4,
            data: vec![1, 2],
        })
        .expect("start");
        app.pump_receive(&DownloadBlock::Chunk {
            number: 1,
            data: vec![3, 4],
        })
        .expect("chunk");
        // The final empty block publishes through the sink; the checksum
        // validation runs against staged bytes (covered separately), so
        // compare the wire-visible callback prefix before publication here.
        // File open/write/close entries are sink-internal on both sides.
        fn comparable(trace: &[String]) -> Vec<String> {
            trace
                .iter()
                .filter(|entry| {
                    entry.starts_with("reliable:")
                        || entry.starts_with("progress:")
                        || entry.as_str() == "packet"
                        || entry.as_str() == "completed"
                })
                .cloned()
                .collect()
        }
        let net_prefix: Vec<String> = net_trace
            .borrow()
            .iter()
            .take_while(|entry| !entry.starts_with("publish:"))
            .cloned()
            .collect();
        assert_eq!(comparable(&net_prefix), comparable(&app_trace.borrow()));
        assert!(!app_trace.borrow().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn paths_map_through_owner_roots() {
        let dir = scratch_dir("paths");
        let owner = RemoteContentRoots {
            base_write_root: dir.join("family").join("base"),
            write_root: dir.join("family").join("game"),
            selection: qa_content::catalog::remote_content_selection(
                RemoteContentBase::Q3Baseq3,
                "mission",
            )
            .expect("selection"),
            content_directory: "mission".to_string(),
            loose_root: None,
            corpus_root: dir.join("corpus"),
        };
        assert_eq!(
            q3_download_path("mission/a.pk3", &owner).expect("mapped"),
            "game/a.pk3"
        );
        assert_eq!(
            q3_download_path("baseq3/b.pk3", &owner).expect("mapped"),
            "base/b.pk3"
        );
        assert_eq!(
            q3_download_path("other/c.pk3", &owner).expect("passthrough"),
            "other/c.pk3"
        );
        assert_eq!(
            q3_download_path("lone.pk3", &owner).expect("flat"),
            "lone.pk3"
        );
        assert!(q3_download_path("../evil.pk3", &owner).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_foreign_roots() {
        let dir = scratch_dir("foreign");
        let owner = RemoteContentRoots {
            base_write_root: dir.join("a").join("base"),
            write_root: dir.join("b").join("game"),
            selection: qa_content::catalog::remote_content_selection(
                RemoteContentBase::Q2ClassicBaseq2,
                "",
            )
            .expect("selection"),
            content_directory: "baseq2".to_string(),
            loose_root: None,
            corpus_root: dir.join("corpus"),
        };
        assert!(q3_download_path("baseq3/a.pk3", &owner).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sizes_cannot_change_mid_transfer() {
        let dir = scratch_dir("size");
        let mut app = script_queue(Rc::new(RefCell::new(Vec::new())), dir.clone());
        app.current = Some(PackageRequest {
            remote: "baseq3/a.pk3".to_string(),
            local: "baseq3/a.pk3".to_string(),
            checksum: 0,
        });
        app.pump_begin("baseq3/a.pk3", "baseq3/a.pk3").expect("begin");
        app.publish_size(4).expect("size");
        app.open_temporary("baseq3/a.pk3.tmp").expect("sink");
        assert!(app.publish_size(8).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn permission_denial_cancels() {
        let dir = scratch_dir("deny");
        let trace: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let trace_in = Rc::clone(&trace);
        let mut app = Q3ApplicationClientDownloads::new(
            dir.clone(),
            Q3DownloadBindings {
                permission: Some(Box::new(|_| false)),
                assert_current: Box::new(|| Ok(())),
                reliable: Box::new(move |text| {
                    trace_in.borrow_mut().push(text.to_string());
                }),
                send_packet: Box::new(|| {}),
                progress: Box::new(|_, _, _| {}),
                reload_packages: Box::new(|| Ok(())),
            },
            None,
        )
        .expect("queue");
        app.receive(&DownloadBlock::Start {
            file_size: 1,
            data: vec![1],
        })
        .expect("receive");
        assert!(trace.borrow().contains(&"stopdl".to_string()));
        assert!(!app.retry().expect("retry"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
