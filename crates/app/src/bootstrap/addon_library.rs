//! Quaddicted add-on library (port of donor
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/addon-library.ts`).
//!
//! [`AddonLibrary`] browses the Quaddicted catalog, installs community Quake
//! add-ons into isolated managed product directories, imports local ZIP
//! packages, and removes installed add-ons. Sync adaptations: catalog fetch,
//! downloads, and installs run inline instead of behind a pending-operation
//! gate (there is no interleaving work to await, so [`AddonLibrary::settle`]
//! is vacuous), downloads pump the shared [`HttpDownloadQueue`] to
//! completion, and cancellation snapshots apply because no other operation
//! can interleave a synchronous install.

use qa_client::ui::library::menu::{LibraryEntry, LibraryMenuService};
use qa_content::archive::{open_archive, ArchiveEntry, EntryRef};
use qa_content::catalog::{
    addon_install_path, managed_addon_hidden, parse_addon_catalog, resolve_addon_packages, AddonMapping, AddonPackage,
    QUADDICTED_CATALOG_URL,
};
use qa_content::contract::ArchiveFormat;
use qa_content::hash::sha256_hex;
use qa_content::mounts::digest_bytes;
use qa_content::value::{parse_save_json, SaveJson};
use qa_net::services::downloads::{DownloadExpectation, SinkExpectation};
use qa_net::services::http_downloads::{
    HttpClient, HttpDownloadError, HttpDownloadKind, HttpDownloadQueue, HttpDownloadRequest, HttpDownloadResult,
    HttpMethod, HttpQueueCallbacks,
};
use qa_platform::files::contained::contained_file_parts;
use qa_platform::files::writable::{UserFileStore, WritableFileMode};
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Staging directory counter (donor `mkdtemp` uniqueness).
static INSTALL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Add-on library failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonLibraryError {
    /// Donor error with its exact message.
    Message(String),
}

impl std::fmt::Display for AddonLibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for AddonLibraryError {}

impl From<std::io::Error> for AddonLibraryError {
    fn from(error: std::io::Error) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<qa_content::catalog::CatalogError> for AddonLibraryError {
    fn from(error: qa_content::catalog::CatalogError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<qa_content::archive::ArchiveError> for AddonLibraryError {
    fn from(error: qa_content::archive::ArchiveError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<qa_content::value::ValueError> for AddonLibraryError {
    fn from(error: qa_content::value::ValueError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<HttpDownloadError> for AddonLibraryError {
    fn from(error: HttpDownloadError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<qa_platform::error::Error> for AddonLibraryError {
    fn from(error: qa_platform::error::Error) -> Self {
        Self::Message(error.to_string())
    }
}

/// Managed content edition (donor `edition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddonEdition {
    /// Classic Quake.
    #[default]
    Classic,
    /// Rerelease Quake.
    Rerelease,
}

/// Add-on library options (donor `AddonLibraryOptions`).
pub struct AddonLibraryOptions<L, C> {
    /// User content root.
    pub root: PathBuf,
    /// Managed content edition (`None` selects classic).
    pub edition: Option<AddonEdition>,
    /// Refresh content after installs change.
    pub changed: C,
    /// Launch an installed product directory and start map.
    pub launch: L,
}

/// Installed managed add-on (donor `InstalledAddon`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledAddon {
    directory: String,
    group: String,
    sha256: String,
    title: String,
    start: Option<String>,
}

/// Shared HTTP client handle for queue ownership.
#[derive(Clone)]
struct SharedClient<H>(Rc<RefCell<H>>);

impl<H: HttpClient> HttpClient for SharedClient<H> {
    fn request(
        &mut self,
        method: HttpMethod,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<qa_net::services::http_downloads::HttpResponse, HttpDownloadError> {
        self.0.borrow_mut().request(method, url, headers)
    }
}

/// Whether a directory name matches the managed `qd_` pattern.
fn is_managed_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() != 44 || !name.starts_with("qd_") || bytes[23] != b'_' {
        return false;
    }
    bytes[3..23]
        .iter()
        .chain(bytes[24..44].iter())
        .all(|byte| byte.is_ascii_hexdigit())
}

/// Whether text is a 64-character lowercase hex digest.
fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether a game directory root is usable (donor `/^[a-zA-Z0-9_+-]+$/`).
fn is_game_root(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-'))
}

/// Escape a JSON string.
fn json_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() + 2);
    escaped.push('"');
    for c in text.chars() {
        match c {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if (c as u32) < 0x20 => escaped.push_str(&format!("\\u{:04x}", c as u32)),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

/// Serialize a `.quaddicted.json` record (donor field order).
fn addon_record_json(sha256: &str, title: &str, group: &str, start: Option<&str>) -> String {
    format!(
        "{{\"sha256\":{},\"title\":{},\"group\":{},\"start\":{}}}",
        json_escape(sha256),
        json_escape(title),
        json_escape(group),
        start.map_or_else(|| "null".to_string(), json_escape)
    )
}

/// Whether a rename failure means the destination already exists.
fn is_exists_conflict(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::DirectoryNotEmpty
    ) || matches!(error.raw_os_error(), Some(17) | Some(39) | Some(66))
}

/// Whether a path is a symlink.
fn is_symlink(path: &Path) -> Result<bool, AddonLibraryError> {
    Ok(std::fs::symlink_metadata(path)?.file_type().is_symlink())
}

/// Read a `.quaddicted.json` record (unparseable files are skipped).
fn read_addon_record(path: &Path) -> Option<InstalledAddon> {
    let text = std::fs::read_to_string(path).ok()?;
    let value = parse_save_json(&text).ok()?;
    let sha256 = match value.get("sha256") {
        Some(SaveJson::String(text)) if is_hex64(text) => text.clone(),
        _ => return None,
    };
    let title = match value.get("title") {
        Some(SaveJson::String(text)) => text.clone(),
        _ => return None,
    };
    let group = match value.get("group") {
        Some(SaveJson::String(text)) => text.clone(),
        _ => return None,
    };
    let start = match value.get("start") {
        None | Some(SaveJson::Null) => None,
        Some(SaveJson::String(text)) => Some(text.clone()),
        _ => return None,
    };
    let directory = path.parent()?.file_name()?.to_string_lossy().into_owned();
    Some(InstalledAddon {
        directory,
        group,
        sha256,
        title,
        start,
    })
}

/// Quaddicted add-on library (donor `AddonLibrary`).
pub struct AddonLibrary<H, L, C> {
    options: AddonLibraryOptions<L, C>,
    catalog: Vec<AddonPackage>,
    installed: Vec<InstalledAddon>,
    selected: Option<AddonPackage>,
    selected_local: Option<InstalledAddon>,
    message: String,
    http: Rc<RefCell<H>>,
    closed: bool,
    cancelled: bool,
}

impl<H, L, C> AddonLibrary<H, L, C>
where
    H: HttpClient + 'static,
    L: FnMut(&str, Option<&str>),
    C: FnMut() -> Result<(), AddonLibraryError>,
{
    /// Borrow a library over options and an HTTP client.
    pub fn new(options: AddonLibraryOptions<L, C>, http: H) -> Self {
        Self {
            options,
            catalog: Vec::new(),
            installed: Vec::new(),
            selected: None,
            selected_local: None,
            message: "Quaddicted — community Quake add-ons".to_string(),
            http: Rc::new(RefCell::new(http)),
            closed: false,
            cancelled: false,
        }
    }

    /// Managed content parent (donor `contentParent`).
    fn content_parent(&self) -> String {
        if self.options.edition == Some(AddonEdition::Rerelease) {
            "q1/rerelease".to_string()
        } else {
            "q1".to_string()
        }
    }

    /// Product name for a managed directory (donor `product`).
    fn product(&self, directory: &str) -> String {
        let edition = if self.options.edition == Some(AddonEdition::Rerelease) {
            "rerelease"
        } else {
            "classic"
        };
        format!("q1-{edition}-{directory}")
    }

    /// Managed directory for a package (donor `directory`).
    fn directory_name(item: &AddonPackage) -> String {
        format!("qd_{}_{}", &sha256_hex(item.group.as_bytes())[..20], &item.sha256[..20])
    }

    /// Run an operation, reporting failures on the status line (donor `run`).
    fn run(&mut self, operation: impl FnOnce(&mut Self) -> Result<(), AddonLibraryError>) {
        if self.closed {
            return;
        }
        self.cancelled = false;
        if let Err(error) = operation(self) {
            if !self.closed {
                self.message = error.to_string();
            }
        }
    }

    /// Refresh the catalog (donor `refresh` body).
    fn refresh_catalog(&mut self) -> Result<(), AddonLibraryError> {
        self.read_installed()?;
        self.message = "Reading Quaddicted catalog…".to_string();
        let fetched = self.fetch_catalog();
        match fetched {
            Ok(()) => {
                self.message = format!("Quaddicted — {} community packages", self.catalog.len());
            }
            Err(error) => {
                if self.closed {
                    return Ok(());
                }
                let cached = self.options.root.join(".addons").join("quaddicted.json");
                let fallback = std::fs::read_to_string(&cached)
                    .map_err(|_| error.clone())
                    .and_then(|text| parse_save_json(&text).map_err(|_| error.clone()))
                    .and_then(|data| parse_addon_catalog(&data).map_err(|_| error.clone()));
                let catalog = fallback?;
                self.catalog = catalog;
                self.message = "Quaddicted unavailable — showing cached catalog".to_string();
            }
        }
        Ok(())
    }

    /// Fetch and cache the remote catalog.
    fn fetch_catalog(&mut self) -> Result<(), AddonLibraryError> {
        let response = self
            .http
            .borrow_mut()
            .request(HttpMethod::Get, QUADDICTED_CATALOG_URL, &[])?;
        if !(200..300).contains(&response.status) {
            return Err(AddonLibraryError::Message(format!(
                "Quaddicted returned HTTP {}",
                response.status
            )));
        }
        let text = String::from_utf8(response.body).map_err(|error| AddonLibraryError::Message(error.to_string()))?;
        let data = parse_save_json(&text)?;
        let catalog = parse_addon_catalog(&data)?;
        self.ensure_directory(".addons")?;
        let store = UserFileStore::new(self.options.root.clone());
        let mut cached = store
            .open(".addons/quaddicted.json", WritableFileMode::Write, |_| {})
            .map_err(AddonLibraryError::from)?
            .ok_or_else(|| AddonLibraryError::Message("Cannot save add-on catalog".to_string()))?;
        cached.write(text.as_bytes()).map_err(AddonLibraryError::from)?;
        cached.close();
        self.catalog = catalog;
        Ok(())
    }

    /// Read installed managed add-ons (donor `readInstalled`).
    fn read_installed(&mut self) -> Result<(), AddonLibraryError> {
        let parent = self.content_parent();
        self.ensure_directory(&parent)?;
        let root = self.options.root.join(&parent);
        let mut installed = Vec::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir() || !is_managed_name(&name) {
                continue;
            }
            if managed_addon_hidden(&self.options.root, &format!("{parent}/{name}"))? {
                continue;
            }
            if let Some(item) = read_addon_record(&root.join(&name).join(".quaddicted.json")) {
                installed.push(item);
            }
        }
        self.installed = installed;
        Ok(())
    }

    /// Ensure a contained storage directory (donor `ensureDirectory`).
    fn ensure_directory(&self, name: &str) -> Result<PathBuf, AddonLibraryError> {
        std::fs::create_dir_all(&self.options.root)?;
        if is_symlink(&self.options.root)? {
            return Err(AddonLibraryError::Message(
                "Add-on root must not be a symlink".to_string(),
            ));
        }
        let mut path = self.options.root.clone();
        for part in contained_file_parts(name).map_err(AddonLibraryError::from)? {
            path = path.join(part);
            match std::fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
            if is_symlink(&path)? {
                return Err(AddonLibraryError::Message(
                    "Add-on storage directory must not be a symlink".to_string(),
                ));
            }
        }
        Ok(path)
    }

    /// Create a unique staging directory (donor `mkdtemp`).
    fn stage_directory(&self, cache: &Path) -> Result<PathBuf, AddonLibraryError> {
        for _ in 0..100 {
            let id = INSTALL_COUNTER.fetch_add(1, Ordering::SeqCst);
            let stage = cache.join(format!("install-{}-{id}", std::process::id()));
            match std::fs::create_dir(&stage) {
                Ok(()) => return Ok(stage),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        Err(AddonLibraryError::Message("Cannot stage add-on install".to_string()))
    }

    /// Hide an installed directory (donor `hide`).
    fn hide(&self, directory: &str) -> Result<(), AddonLibraryError> {
        let store = UserFileStore::new(self.options.root.clone());
        let mut file = store
            .open(
                &format!(".addons/removed/{}/{}", self.content_parent(), directory),
                WritableFileMode::Write,
                |_| {},
            )
            .map_err(AddonLibraryError::from)?
            .ok_or_else(|| AddonLibraryError::Message("Cannot update installed add-on catalog".to_string()))?;
        file.close();
        Ok(())
    }

    /// Import a local ZIP package (donor `importLocal`).
    fn import_local(&mut self, path: &str) -> Result<(), AddonLibraryError> {
        let bytes = std::fs::read(path)?;
        let sha256 = sha256_hex(&bytes);
        let archive = open_archive(Path::new(path), None)?;
        let mut game_directory = "id1".to_string();
        if archive.format != ArchiveFormat::Zip {
            archive.close();
            return Err(AddonLibraryError::Message("Select a Quake ZIP package".to_string()));
        }
        {
            let mut roots = HashSet::new();
            for entry in &archive.entries {
                if entry.is_directory() {
                    continue;
                }
                if let Some(root) = entry.path().split('/').next() {
                    roots.insert(root.to_string());
                }
            }
            if roots.len() == 1 {
                let only = roots.into_iter().next().unwrap_or_default();
                if !["maps", "progs", "gfx", "sound", "music", "textures"].contains(&only.as_str())
                    && is_game_root(&only)
                {
                    game_directory = only;
                }
            }
        }
        archive.close();
        let filename = Path::new(path)
            .file_name()
            .map_or_else(|| path.to_string(), |name| name.to_string_lossy().into_owned());
        let title = match filename.len() {
            len if filename.to_lowercase().ends_with(".zip") => filename[..len - 4].to_string(),
            _ => filename.clone(),
        };
        let absolute = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            std::env::current_dir()?.join(path)
        };
        let item = AddonPackage {
            digest: digest_bytes(&bytes),
            sha256: sha256.clone(),
            title,
            filename: filename.clone(),
            group: format!("local:{}", filename.to_lowercase()),
            bytes: bytes.len() as u64,
            url: format!("file://{}", absolute.to_string_lossy()),
            tags: Vec::new(),
            starts: Vec::new(),
            game_directory,
            mappings: vec![AddonMapping {
                from: String::new(),
                to: Some(String::new()),
            }],
            unavailable: None,
        };
        let cache = self.ensure_directory(".addons")?;
        let store = UserFileStore::new(cache);
        let mut file = store
            .open(&format!("{sha256}.zip"), WritableFileMode::Write, |_| {})
            .map_err(AddonLibraryError::from)?
            .ok_or_else(|| AddonLibraryError::Message("Cannot cache local add-on".to_string()))?;
        if file.write(&bytes).map_err(AddonLibraryError::from)? != bytes.len() {
            return Err(AddonLibraryError::Message("Incomplete local add-on copy".to_string()));
        }
        file.close();
        self.install(&item)
    }

    /// Install a package (donor `install`).
    fn install(&mut self, selected: &AddonPackage) -> Result<(), AddonLibraryError> {
        self.read_installed()?;
        let packages = resolve_addon_packages(&self.catalog, selected)?;
        let cache = self.ensure_directory(".addons")?;
        let stage = self.stage_directory(&cache)?;
        let result = self.install_packages(selected, &packages, &cache, &stage);
        let _ = std::fs::remove_dir_all(&stage);
        result
    }

    /// Download and stage packages, then publish the install.
    fn install_packages(
        &mut self,
        selected: &AddonPackage,
        packages: &[AddonPackage],
        cache: &Path,
        stage: &Path,
    ) -> Result<(), AddonLibraryError> {
        let files = UserFileStore::new(stage.to_path_buf());
        let halted = self.closed || self.cancelled;
        let resolved_cache = cache.to_path_buf();
        let progress: Rc<RefCell<(u64, Option<u64>)>> = Rc::new(RefCell::new((0, None)));
        let progress_slot = Rc::clone(&progress);
        let mut queue = HttpDownloadQueue::new(
            cache.to_path_buf(),
            Box::new(SharedClient(Rc::clone(&self.http))),
            HttpQueueCallbacks {
                assert_current: Box::new(move || {
                    if halted {
                        Err(HttpDownloadError::StaleEpoch)
                    } else {
                        Ok(())
                    }
                }),
                resolved: Box::new(move |path: &str| {
                    let Ok(bytes) = std::fs::read(resolved_cache.join(path)) else {
                        return false;
                    };
                    sha256_hex(&bytes) == path.get(..64).unwrap_or_default()
                }),
                refresh_package: Box::new(|_| Ok(())),
                progress: Box::new(move |_, received, total| {
                    *progress_slot.borrow_mut() = (received, total);
                }),
            },
            1,
            1,
        )?;
        for item in packages {
            let name = format!("{}.zip", item.sha256);
            let result = if item.url.starts_with("file:") {
                HttpDownloadResult::Resolved
            } else {
                queue.enqueue(HttpDownloadRequest {
                    path: name.clone(),
                    url: item.url.clone(),
                    kind: HttpDownloadKind::Asset,
                    expected: SinkExpectation::Content(DownloadExpectation {
                        digest: qa_net::common::session::ContentDigest::new(&item.sha256)
                            .map_err(|error| AddonLibraryError::Message(error.to_string()))?,
                        byte_length: item.bytes,
                    }),
                    validate: None,
                })?;
                self.pump_download(&mut queue, &name, &progress, &selected.title)?
            };
            match result {
                HttpDownloadResult::Downloaded { .. } | HttpDownloadResult::Resolved => {}
                HttpDownloadResult::Cancelled => {
                    return Err(AddonLibraryError::Message("Add-on installation cancelled".to_string()));
                }
                HttpDownloadResult::Failed { reason } | HttpDownloadResult::Fallback { reason } => {
                    return Err(AddonLibraryError::Message(reason));
                }
            }
            if self.closed || self.cancelled {
                return Err(AddonLibraryError::Message("Add-on installation cancelled".to_string()));
            }
            let archive = open_archive(&cache.join(&name), None)?;
            for index in 0..archive.entries.len() {
                if self.closed || self.cancelled {
                    archive.close();
                    return Err(AddonLibraryError::Message("Add-on installation cancelled".to_string()));
                }
                let entry: &ArchiveEntry = &archive.entries[index];
                if entry.is_directory() {
                    continue;
                }
                let Some(target) = addon_install_path(item, entry.path())? else {
                    continue;
                };
                let bytes = archive.read_entry(EntryRef::Entry(entry))?;
                let mut file = files
                    .open(&target, WritableFileMode::Write, |_| {})
                    .map_err(AddonLibraryError::from)?
                    .ok_or_else(|| AddonLibraryError::Message(format!("Cannot stage {}", entry.path())))?;
                if file.write(&bytes).map_err(AddonLibraryError::from)? != bytes.len() {
                    archive.close();
                    return Err(AddonLibraryError::Message(format!(
                        "Incomplete add-on write: {}",
                        entry.path()
                    )));
                }
                file.close();
            }
            archive.close();
        }
        let game_root = stage.join(&selected.game_directory);
        let directory = Self::directory_name(selected);
        let destination = self.options.root.join(self.content_parent()).join(directory.clone());
        let start = selected.starts.first().cloned();
        std::fs::write(
            game_root.join(".quaddicted.json"),
            addon_record_json(&selected.sha256, &selected.title, &selected.group, start.as_deref()),
        )?;
        if self.closed || self.cancelled {
            return Err(AddonLibraryError::Message("Add-on installation cancelled".to_string()));
        }
        let previous: Vec<InstalledAddon> = self
            .installed
            .iter()
            .filter(|item| item.group == selected.group && item.directory != directory)
            .cloned()
            .collect();
        match std::fs::rename(&game_root, &destination) {
            Ok(()) => {}
            Err(error) if is_exists_conflict(&error) => {
                let saved = std::fs::read_to_string(destination.join(".quaddicted.json"))?;
                let data = parse_save_json(&saved)?;
                let matches = matches!(data.get("sha256"), Some(SaveJson::String(found)) if found == &selected.sha256);
                if !matches {
                    return Err(AddonLibraryError::Message(
                        "Managed add-on destination conflicts with existing content".to_string(),
                    ));
                }
            }
            Err(error) => return Err(error.into()),
        }
        let _ = std::fs::remove_file(cache.join("removed").join(self.content_parent()).join(&directory));
        for old in &previous {
            self.hide(&old.directory)?;
        }
        if let Err(error) = (self.options.changed)() {
            self.hide(&directory)?;
            for old in &previous {
                let _ = std::fs::remove_file(cache.join("removed").join(self.content_parent()).join(&old.directory));
            }
            return Err(error);
        }
        self.read_installed()?;
        self.message = format!("Installed {} — ready to play", selected.title);
        Ok(())
    }

    /// Pump the download queue until a destination settles.
    fn pump_download(
        &mut self,
        queue: &mut HttpDownloadQueue,
        name: &str,
        progress: &Rc<RefCell<(u64, Option<u64>)>>,
        title: &str,
    ) -> Result<HttpDownloadResult, AddonLibraryError> {
        for _ in 0..16 {
            queue.process();
            let (received, total) = *progress.borrow();
            self.message = format!(
                "Downloading {title}: {received}/{}",
                total.map_or_else(|| "?".to_string(), |total| total.to_string())
            );
            if let Some(result) = queue.result(name) {
                return Ok(result.clone());
            }
        }
        Err(AddonLibraryError::Message("Add-on download stalled".to_string()))
    }

    /// Remove an installed add-on (donor `removeInstalled`).
    fn remove_installed(&mut self, item: &InstalledAddon) -> Result<(), AddonLibraryError> {
        self.hide(&item.directory)?;
        if let Err(error) = (self.options.changed)() {
            let _ = std::fs::remove_file(
                self.options
                    .root
                    .join(".addons")
                    .join("removed")
                    .join(self.content_parent())
                    .join(&item.directory),
            );
            return Err(error);
        }
        self.read_installed()
    }

    /// Launch an installed product.
    fn launch_installed(&mut self, directory: &str, start: Option<&str>) {
        let product = self.product(directory);
        (self.options.launch)(&product, start);
    }

    /// Settle in-flight work (donor `settle`).
    pub fn settle(&mut self) {
        // Synchronous execution never leaves work in flight.
    }

    /// Cancel in-flight work (donor `suspend`).
    pub fn suspend(&mut self) {
        self.cancelled = true;
    }

    /// Close the library (donor `close`).
    pub fn close(&mut self) {
        self.closed = true;
    }
}

impl<H, L, C> LibraryMenuService for AddonLibrary<H, L, C>
where
    H: HttpClient + 'static,
    L: FnMut(&str, Option<&str>),
    C: FnMut() -> Result<(), AddonLibraryError>,
{
    fn scope(&self) -> String {
        if let Some(local) = &self.selected_local {
            return format!("installed:{}", local.directory);
        }
        if let Some(selected) = &self.selected {
            return format!("package:{}", selected.sha256);
        }
        "catalog".to_string()
    }

    fn status(&self) -> String {
        self.message.clone()
    }

    fn entries(&self) -> Vec<LibraryEntry> {
        if let Some(local) = &self.selected_local {
            return vec![
                LibraryEntry {
                    id: "local-play".to_string(),
                    label: "Play".to_string(),
                    detail: Some(local.title.clone()),
                    unavailable: None,
                },
                LibraryEntry {
                    id: "local-remove".to_string(),
                    label: "Remove installed add-on".to_string(),
                    detail: Some(local.title.clone()),
                    unavailable: None,
                },
                LibraryEntry {
                    id: "back".to_string(),
                    label: "Back to add-ons".to_string(),
                    detail: None,
                    unavailable: None,
                },
            ];
        }
        if let Some(selected) = &self.selected {
            let installed = self.installed.iter().find(|item| item.group == selected.group);
            let mut rows = Vec::new();
            if let Some(installed) = installed {
                rows.push(LibraryEntry {
                    id: "play".to_string(),
                    label: "Play".to_string(),
                    detail: Some(installed.title.clone()),
                    unavailable: None,
                });
                rows.push(LibraryEntry {
                    id: "remove".to_string(),
                    label: "Remove installed add-on".to_string(),
                    detail: Some("Only this managed copy".to_string()),
                    unavailable: None,
                });
            }
            if installed.is_none_or(|installed| installed.sha256 != selected.sha256) {
                rows.push(LibraryEntry {
                    id: "install".to_string(),
                    label: if installed.is_none() {
                        "Install".to_string()
                    } else {
                        "Update".to_string()
                    },
                    detail: Some(format!("{} — {} KiB", selected.title, selected.bytes.div_ceil(1024))),
                    unavailable: selected.unavailable.clone(),
                });
            }
            rows.push(LibraryEntry {
                id: "back".to_string(),
                label: "Back to add-ons".to_string(),
                detail: None,
                unavailable: None,
            });
            return rows;
        }
        let mut rows: Vec<LibraryEntry> = self
            .catalog
            .iter()
            .map(|item| LibraryEntry {
                id: item.sha256.clone(),
                label: item.title.clone(),
                detail: Some(if self.installed.iter().any(|saved| saved.group == item.group) {
                    "Installed".to_string()
                } else {
                    "Quaddicted".to_string()
                }),
                unavailable: None,
            })
            .collect();
        for item in &self.installed {
            if !rows.iter().any(|row| row.id == item.sha256) {
                rows.insert(
                    0,
                    LibraryEntry {
                        id: format!("local:{}", item.directory),
                        label: item.title.clone(),
                        detail: Some("Installed — play".to_string()),
                        unavailable: None,
                    },
                );
            }
        }
        rows
    }

    fn refresh(&mut self) {
        self.run(|library| library.refresh_catalog());
    }

    fn activate(&mut self, id: &str) {
        if self.closed {
            return;
        }
        if id == "back" {
            self.selected = None;
            self.selected_local = None;
            return;
        }
        if id == "local-play" {
            if let Some(local) = self.selected_local.clone() {
                self.cancelled = false;
                self.launch_installed(&local.directory, local.start.as_deref());
            }
            return;
        }
        if id == "local-remove" {
            if let Some(local) = self.selected_local.clone() {
                self.run(|library| {
                    library.remove_installed(&local)?;
                    library.selected_local = None;
                    library.message = format!("Removed {}", local.title);
                    Ok(())
                });
            }
            return;
        }
        if let Some(local) = self
            .installed
            .iter()
            .find(|item| format!("local:{}", item.directory) == id)
            .cloned()
        {
            self.selected_local = Some(local);
            return;
        }
        if self.selected.is_none() {
            self.selected = self.catalog.iter().find(|item| item.sha256 == id).cloned();
            return;
        }
        let Some(selected) = self.selected.clone() else {
            return;
        };
        let installed = self.installed.iter().find(|item| item.group == selected.group).cloned();
        if id == "play" {
            if let Some(installed) = installed {
                self.cancelled = false;
                self.launch_installed(&installed.directory, installed.start.as_deref());
            }
        } else if id == "install" {
            self.run(|library| library.install(&selected));
        } else if id == "remove" {
            if let Some(installed) = installed {
                self.run(|library| {
                    library.remove_installed(&installed)?;
                    library.message = format!("Removed {}", installed.title);
                    Ok(())
                });
            }
        }
    }

    fn create_label(&self) -> Option<String> {
        Some("Import Quake ZIP".to_string())
    }

    fn create_submit(&mut self, name: &str) {
        let path = name.to_string();
        self.run(|library| library.import_local(&path));
    }

    fn stop_label(&self) -> Option<String> {
        Some("Cancel download".to_string())
    }

    fn stop_activate(&mut self) {
        self.cancelled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_net::services::http_downloads::HttpResponse;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn crc32_iso(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn zip_store(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut locals = Vec::new();
        for (name, data) in files {
            locals.push(bytes.len() as u32);
            bytes.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&crc32_iso(data).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(data);
        }
        let central_offset = bytes.len() as u32;
        for ((name, data), local) in files.iter().zip(locals.iter()) {
            bytes.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
            bytes.extend_from_slice(&0x031Eu16.to_le_bytes());
            bytes.extend_from_slice(&20u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&crc32_iso(data).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&[0u8; 8]);
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&local.to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
        }
        let central_size = bytes.len() as u32 - central_offset;
        bytes.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }

    fn root(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("qa-addon-{}-{}-{name}", std::process::id(), id));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    struct StubHttp {
        catalog_status: u16,
        catalog_body: Vec<u8>,
        files: std::collections::HashMap<String, Vec<u8>>,
    }

    impl HttpClient for StubHttp {
        fn request(
            &mut self,
            _method: HttpMethod,
            url: &str,
            _headers: &[(String, String)],
        ) -> Result<HttpResponse, HttpDownloadError> {
            if url == QUADDICTED_CATALOG_URL {
                return Ok(HttpResponse {
                    status: self.catalog_status,
                    headers: Vec::new(),
                    body: self.catalog_body.clone(),
                });
            }
            let normalized = url.replace(":443/", "/").replace(":80/", "/");
            match self.files.get(&normalized).or_else(|| self.files.get(url)) {
                Some(body) => Ok(HttpResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: body.clone(),
                }),
                None => Ok(HttpResponse {
                    status: 404,
                    headers: Vec::new(),
                    body: Vec::new(),
                }),
            }
        }
    }

    type LaunchFn = Box<dyn FnMut(&str, Option<&str>)>;
    type ChangedFn = Box<dyn FnMut() -> Result<(), AddonLibraryError>>;
    type TestLibrary = AddonLibrary<StubHttp, LaunchFn, ChangedFn>;
    type LaunchLog = Rc<RefCell<Vec<(String, Option<String>)>>>;
    type Harness = (TestLibrary, LaunchLog, Rc<RefCell<u32>>);

    fn package_url(sha256: &str) -> String {
        format!(
            "https://www.quaddicted.com/files/by-sha256/{}/{sha256}/testquest.zip",
            &sha256[..2]
        )
    }

    fn catalog_json(sha256: &str, bytes: u64) -> Vec<u8> {
        format!(
            r#"[{{"tags":["game=quake","game_mode=singleplayer","filename=testquest.zip","title=Test Quest","release_group=testquest","startmap=start","commandline=-game testquest"],"sha256":"{sha256}","bytes":{bytes},"urls":["{}"],"install":{{"extract":""}}}}]"#,
            package_url(sha256)
        )
        .into_bytes()
    }

    fn quest_zip() -> Vec<u8> {
        zip_store(&[
            ("testquest/maps/start.bsp", b"BSPDATA"),
            ("testquest/readme.txt", b"hi"),
        ])
    }

    fn library(root: PathBuf, http: StubHttp, launches: LaunchLog, changes: Rc<RefCell<u32>>) -> TestLibrary {
        let launch: LaunchFn = Box::new(move |product: &str, map: Option<&str>| {
            launches
                .borrow_mut()
                .push((product.to_string(), map.map(str::to_string)));
        });
        let changed: ChangedFn = Box::new(move || {
            *changes.borrow_mut() += 1;
            Ok(())
        });
        AddonLibrary::new(
            AddonLibraryOptions {
                root,
                edition: None,
                changed,
                launch,
            },
            http,
        )
    }

    fn harness(root: PathBuf, zip: &[u8]) -> Harness {
        let sha256 = sha256_hex(zip);
        let url = package_url(&sha256);
        let mut files = std::collections::HashMap::new();
        files.insert(url, zip.to_vec());
        let http = StubHttp {
            catalog_status: 200,
            catalog_body: catalog_json(&sha256, zip.len() as u64),
            files,
        };
        let launches = Rc::new(RefCell::new(Vec::new()));
        let changes = Rc::new(RefCell::new(0u32));
        (
            library(root, http, Rc::clone(&launches), Rc::clone(&changes)),
            launches,
            changes,
        )
    }

    #[test]
    fn refresh_loads_and_caches_catalog() {
        let zip = quest_zip();
        let (mut library, _, _) = harness(root("refresh"), &zip);
        LibraryMenuService::refresh(&mut library);
        assert_eq!(library.status(), "Quaddicted — 1 community packages");
        let entries = library.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "Test Quest");
        assert_eq!(entries[0].detail.as_deref(), Some("Quaddicted"));
        assert!(library.options.root.join(".addons").join("quaddicted.json").exists());
    }

    #[test]
    fn refresh_falls_back_to_cached_catalog() {
        let zip = quest_zip();
        let root = root("fallback");
        let (mut library, _, _) = harness(root.clone(), &zip);
        LibraryMenuService::refresh(&mut library);
        library.http.borrow_mut().catalog_status = 500;
        library.http.borrow_mut().catalog_body = Vec::new();
        LibraryMenuService::refresh(&mut library);
        assert_eq!(library.status(), "Quaddicted unavailable — showing cached catalog");
        assert_eq!(library.entries().len(), 1);
    }

    #[test]
    fn import_installs_local_zip() {
        let root = root("import");
        let zip_path = root.join("myquest.zip");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&zip_path, zip_store(&[("myquest/maps/start.bsp", b"BSPDATA")])).unwrap();
        let http = StubHttp {
            catalog_status: 200,
            catalog_body: b"[]".to_vec(),
            files: Default::default(),
        };
        let launches = Rc::new(RefCell::new(Vec::new()));
        let changes = Rc::new(RefCell::new(0u32));
        let mut library = library(root.clone(), http, Rc::clone(&launches), Rc::clone(&changes));
        library.create_submit(zip_path.to_string_lossy().as_ref());
        assert!(
            library.status().starts_with("Installed myquest"),
            "{}",
            library.status()
        );
        assert_eq!(*changes.borrow(), 1);
        let entries = library.entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].detail.as_deref(), Some("Installed — play"));
        library.activate(entries[0].id.as_str());
        assert!(library.scope().starts_with("installed:"));
        library.activate("local-play");
        assert_eq!(launches.borrow().len(), 1);
        assert!(launches.borrow()[0].0.starts_with("q1-classic-qd_"));
        library.activate("local-remove");
        assert!(library.status().starts_with("Removed myquest"), "{}", library.status());
        assert_eq!(*changes.borrow(), 2);
        assert!(library.entries().is_empty());
    }

    #[test]
    fn catalog_install_downloads_and_removes() {
        let zip = quest_zip();
        let sha256 = sha256_hex(&zip);
        let (mut library, _, changes) = harness(root("install"), &zip);
        LibraryMenuService::refresh(&mut library);
        library.activate(sha256.as_str());
        assert_eq!(library.scope(), format!("package:{sha256}"));
        let detail = library.entries();
        assert!(detail.iter().any(|row| row.id == "install" && row.label == "Install"));
        library.activate("install");
        assert!(
            library.status().starts_with("Installed Test Quest"),
            "{}",
            library.status()
        );
        assert_eq!(*changes.borrow(), 1);
        let detail = library.entries();
        assert!(detail.iter().any(|row| row.id == "play"));
        assert!(!detail.iter().any(|row| row.id == "install"));
        library.activate("remove");
        assert_eq!(library.status(), "Removed Test Quest");
        let detail = library.entries();
        assert!(detail.iter().any(|row| row.id == "install"));
    }

    #[test]
    fn close_ignores_operations() {
        let zip = quest_zip();
        let (mut library, _, _) = harness(root("close"), &zip);
        library.close();
        LibraryMenuService::refresh(&mut library);
        assert_eq!(library.status(), "Quaddicted — community Quake add-ons");
        library.activate("back");
        library.create_submit("whatever.zip");
        assert_eq!(library.status(), "Quaddicted — community Quake add-ons");
    }
}
