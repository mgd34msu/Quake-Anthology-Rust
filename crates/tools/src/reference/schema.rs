//! Reference schemas (donor `tools/reference/schema.ts`).
//!
//! Typed observations shared by environment inventory and capture tools.
//! Each type renders with donor field order so manifests stay comparable.

use crate::json::Json;

/// Identity of an observed file: path, byte size, and SHA-256 hex.
#[derive(Debug, Clone)]
pub struct FileIdentity {
    /// Observed path.
    pub path: String,
    /// Byte size.
    pub size: u64,
    /// SHA-256 hex digest of the bytes.
    pub sha256: String,
}

impl FileIdentity {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("size".to_owned(), Json::uint(self.size)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
        ])
    }
}

/// How an observed command finished.
#[derive(Debug, Clone)]
pub enum CommandOutcome {
    /// Process exited with a code.
    Exited {
        /// Exit code.
        exit_code: i32,
    },
    /// Timeout fired before the process exited.
    TimedOut {
        /// Timeout in milliseconds.
        timeout_ms: u64,
    },
}

impl CommandOutcome {
    fn to_json(&self) -> Json {
        match self {
            Self::Exited { exit_code } => Json::object(vec![
                ("kind".to_owned(), Json::string("exited")),
                ("exitCode".to_owned(), Json::int(i64::from(*exit_code))),
            ]),
            Self::TimedOut { timeout_ms } => Json::object(vec![
                ("kind".to_owned(), Json::string("timed-out")),
                ("timeoutMs".to_owned(), Json::uint(*timeout_ms)),
            ]),
        }
    }
}

/// A command execution with captured streams and timing.
#[derive(Debug, Clone)]
pub struct CommandObservation {
    /// Argument vector.
    pub command: Vec<String>,
    /// Working directory.
    pub cwd: String,
    /// Environment entries.
    pub environment: Vec<(String, String)>,
    /// ISO-8601 start time.
    pub started_at: String,
    /// Elapsed milliseconds (fractional, like `performance.now`).
    pub duration_ms: f64,
    /// How the command finished.
    pub outcome: CommandOutcome,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl CommandObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            (
                "command".to_owned(),
                Json::array(self.command.iter().map(Json::string).collect()),
            ),
            ("cwd".to_owned(), Json::string(&self.cwd)),
            (
                "environment".to_owned(),
                Json::object(
                    self.environment
                        .iter()
                        .map(|(key, value)| (key.clone(), Json::string(value)))
                        .collect(),
                ),
            ),
            ("startedAt".to_owned(), Json::string(&self.started_at)),
            ("durationMs".to_owned(), Json::float(self.duration_ms)),
            ("outcome".to_owned(), self.outcome.to_json()),
            ("stdout".to_owned(), Json::string(&self.stdout)),
            ("stderr".to_owned(), Json::string(&self.stderr)),
        ])
    }
}

/// Availability of an external tool plus its probe observations.
#[derive(Debug, Clone)]
pub enum ToolObservation {
    /// Tool executable was not found.
    Unavailable {
        /// Tool name.
        name: String,
    },
    /// Tool executable was found and probed.
    Available {
        /// Tool name.
        name: String,
        /// Identity of the executable.
        executable: FileIdentity,
        /// Probe observations.
        observations: Vec<CommandObservation>,
    },
}

impl ToolObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Unavailable { name } => Json::object(vec![
                ("kind".to_owned(), Json::string("unavailable")),
                ("name".to_owned(), Json::string(name)),
            ]),
            Self::Available {
                name,
                executable,
                observations,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("available")),
                ("name".to_owned(), Json::string(name)),
                ("executable".to_owned(), executable.to_json()),
                (
                    "observations".to_owned(),
                    Json::array(observations.iter().map(CommandObservation::to_json).collect()),
                ),
            ]),
        }
    }
}

/// A changed path inside a source tree.
#[derive(Debug, Clone)]
pub enum PathObservation {
    /// Changed file with identity.
    File {
        /// File identity.
        identity: FileIdentity,
    },
    /// Changed symlink.
    Symlink {
        /// Observed path.
        path: String,
        /// Link target.
        target: String,
    },
    /// Deleted path.
    Deleted {
        /// Observed path.
        path: String,
    },
    /// Changed directory.
    Directory {
        /// Observed path.
        path: String,
    },
}

impl PathObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::File { identity } => Json::object(vec![
                ("kind".to_owned(), Json::string("file")),
                ("identity".to_owned(), identity.to_json()),
            ]),
            Self::Symlink { path, target } => Json::object(vec![
                ("kind".to_owned(), Json::string("symlink")),
                ("path".to_owned(), Json::string(path)),
                ("target".to_owned(), Json::string(target)),
            ]),
            Self::Deleted { path } => Json::object(vec![
                ("kind".to_owned(), Json::string("deleted")),
                ("path".to_owned(), Json::string(path)),
            ]),
            Self::Directory { path } => Json::object(vec![
                ("kind".to_owned(), Json::string("directory")),
                ("path".to_owned(), Json::string(path)),
            ]),
        }
    }
}

/// Role of an inventoried source tree.
#[derive(Debug, Clone, Copy)]
pub enum SourceRole {
    /// Original upstream source.
    OriginalSource,
    /// TypeScript donor tree.
    TypescriptDonor,
}

impl SourceRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::OriginalSource => "original-source",
            Self::TypescriptDonor => "typescript-donor",
        }
    }
}

/// Cleanliness of an inventoried source tree.
#[derive(Debug, Clone, Copy)]
pub enum SourceState {
    /// Tree matches its recorded head.
    Clean,
    /// Tree has local changes.
    Modified,
}

impl SourceState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Modified => "modified",
        }
    }
}

/// Identity and state of an inventoried source tree.
#[derive(Debug, Clone)]
pub struct SourceIdentity {
    /// Stable source identifier.
    pub source_id: String,
    /// Tree path.
    pub path: String,
    /// Tree role.
    pub role: SourceRole,
    /// Expected HEAD commit.
    pub expected_head: String,
    /// Observed HEAD commit.
    pub head: String,
    /// Observed tree hash.
    pub tree: String,
    /// Cleanliness.
    pub state: SourceState,
    /// Changed paths.
    pub changes: Vec<PathObservation>,
    /// Probe observations.
    pub observations: Vec<CommandObservation>,
}

impl SourceIdentity {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("sourceId".to_owned(), Json::string(&self.source_id)),
            ("path".to_owned(), Json::string(&self.path)),
            ("role".to_owned(), Json::string(self.role.as_str())),
            ("expectedHead".to_owned(), Json::string(&self.expected_head)),
            ("head".to_owned(), Json::string(&self.head)),
            ("tree".to_owned(), Json::string(&self.tree)),
            ("state".to_owned(), Json::string(self.state.as_str())),
            (
                "changes".to_owned(),
                Json::array(self.changes.iter().map(PathObservation::to_json).collect()),
            ),
            (
                "observations".to_owned(),
                Json::array(self.observations.iter().map(CommandObservation::to_json).collect()),
            ),
        ])
    }
}

/// Executable container format.
#[derive(Debug, Clone, Copy)]
pub enum BinaryFormat {
    /// ELF executable or shared object.
    Elf,
    /// PE executable or library.
    Pe,
    /// DOS MZ executable.
    DosMz,
}

impl BinaryFormat {
    fn as_str(self) -> &'static str {
        match self {
            Self::Elf => "elf",
            Self::Pe => "pe",
            Self::DosMz => "dos-mz",
        }
    }
}

/// Assumed purpose of an observed binary.
#[derive(Debug, Clone, Copy)]
pub enum BinaryPurpose {
    /// Retail Windows engine binary.
    RetailWindowsEngine,
    /// Retail DOS engine binary.
    RetailDosEngine,
    /// Donor-built executable.
    TypescriptDonorExecutable,
    /// Binary living in a source tree.
    SourceTreeBinary,
    /// Any other corpus binary.
    OtherCorpusBinary,
}

impl BinaryPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::RetailWindowsEngine => "retail-windows-engine",
            Self::RetailDosEngine => "retail-dos-engine",
            Self::TypescriptDonorExecutable => "typescript-donor-executable",
            Self::SourceTreeBinary => "source-tree-binary",
            Self::OtherCorpusBinary => "other-corpus-binary",
        }
    }
}

/// Quake family of a Steam title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuakeFamily {
    /// Quake.
    Q1,
    /// Quake 2.
    Q2,
    /// Quake 3 Arena.
    Q3,
}

impl QuakeFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::Q1 => "q1",
            Self::Q2 => "q2",
            Self::Q3 => "q3",
        }
    }
}

/// Edition of a Steam title.
#[derive(Debug, Clone, Copy)]
pub enum SteamEdition {
    /// Original classic release.
    Classic,
    /// Rerelease.
    Rerelease,
}

impl SteamEdition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Rerelease => "rerelease",
        }
    }
}

/// Where an observed binary came from.
#[derive(Debug, Clone)]
pub enum BinaryProvenance {
    /// Found in a source tree.
    SourceTree,
    /// Supplied with the corpus.
    SuppliedCorpus,
    /// Found in a Steam installation.
    SteamInstallation {
        /// Installation path.
        installation_path: String,
        /// Quake family.
        family: QuakeFamily,
        /// Title edition.
        edition: SteamEdition,
    },
}

impl BinaryProvenance {
    fn to_json(&self) -> Json {
        match self {
            Self::SourceTree => Json::object(vec![("kind".to_owned(), Json::string("source-tree"))]),
            Self::SuppliedCorpus => Json::object(vec![("kind".to_owned(), Json::string("supplied-corpus"))]),
            Self::SteamInstallation {
                installation_path,
                family,
                edition,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("steam-installation")),
                ("installationPath".to_owned(), Json::string(installation_path)),
                ("family".to_owned(), Json::string(family.as_str())),
                ("edition".to_owned(), Json::string(edition.as_str())),
            ]),
        }
    }
}

/// An observed binary: identity, format, purpose, provenance, and probes.
#[derive(Debug, Clone)]
pub struct BinaryObservation {
    /// File identity.
    pub identity: FileIdentity,
    /// Container format.
    pub format: BinaryFormat,
    /// Assumed purpose.
    pub purpose: BinaryPurpose,
    /// Whether the executable bit is set.
    pub executable_permission: bool,
    /// Provenance.
    pub provenance: BinaryProvenance,
    /// Header probe observation.
    pub header: CommandObservation,
    /// Why the binary was not executed.
    pub execution_reason: String,
}

impl BinaryObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("identity".to_owned(), self.identity.to_json()),
            ("format".to_owned(), Json::string(self.format.as_str())),
            ("purpose".to_owned(), Json::string(self.purpose.as_str())),
            (
                "executablePermission".to_owned(),
                Json::boolean(self.executable_permission),
            ),
            ("provenance".to_owned(), self.provenance.to_json()),
            ("header".to_owned(), self.header.to_json()),
            (
                "execution".to_owned(),
                Json::object(vec![
                    ("kind".to_owned(), Json::string("not-run")),
                    ("reason".to_owned(), Json::string(&self.execution_reason)),
                ]),
            ),
        ])
    }
}

/// Value of an observed file read.
#[derive(Debug, Clone)]
pub enum ReadValue {
    /// Read succeeded.
    Read {
        /// File text.
        text: String,
        /// SHA-256 hex of the text.
        sha256: String,
    },
    /// Read failed.
    Unavailable {
        /// Failure reason.
        reason: String,
    },
}

impl ReadValue {
    fn to_json(&self) -> Json {
        match self {
            Self::Read { text, sha256 } => Json::object(vec![
                ("kind".to_owned(), Json::string("read")),
                ("text".to_owned(), Json::string(text)),
                ("sha256".to_owned(), Json::string(sha256)),
            ]),
            Self::Unavailable { reason } => Json::object(vec![
                ("kind".to_owned(), Json::string("unavailable")),
                ("reason".to_owned(), Json::string(reason)),
            ]),
        }
    }
}

/// An observed file read.
#[derive(Debug, Clone)]
pub struct ReadObservation {
    /// Observed path.
    pub path: String,
    /// Read value.
    pub value: ReadValue,
}

impl ReadObservation {
    /// Read a file, degrading to `unavailable` instead of failing.
    #[must_use]
    pub fn read_file(path: &str) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let sha256 = crate::verify::hash::hash_str(&text);
                Self {
                    path: path.to_owned(),
                    value: ReadValue::Read { text, sha256 },
                }
            }
            Err(error) => Self {
                path: path.to_owned(),
                value: ReadValue::Unavailable {
                    reason: error.to_string(),
                },
            },
        }
    }

    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("value".to_owned(), self.value.to_json()),
        ])
    }
}

/// Availability of a Steam title directory.
#[derive(Debug, Clone)]
pub enum TitleAvailability {
    /// Title directory is present.
    Present,
    /// Title directory is missing or unusable.
    Unavailable {
        /// Reason.
        reason: String,
    },
}

impl TitleAvailability {
    fn to_json(&self) -> Json {
        match self {
            Self::Present => Json::object(vec![("kind".to_owned(), Json::string("present"))]),
            Self::Unavailable { reason } => Json::object(vec![
                ("kind".to_owned(), Json::string("unavailable")),
                ("reason".to_owned(), Json::string(reason)),
            ]),
        }
    }
}

/// An observed Steam title.
#[derive(Debug, Clone)]
pub struct SteamTitleObservation {
    /// Title name.
    pub name: String,
    /// Quake family.
    pub family: QuakeFamily,
    /// Title path.
    pub path: String,
    /// Availability.
    pub availability: TitleAvailability,
}

impl SteamTitleObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("name".to_owned(), Json::string(&self.name)),
            ("family".to_owned(), Json::string(self.family.as_str())),
            ("path".to_owned(), Json::string(&self.path)),
            ("availability".to_owned(), self.availability.to_json()),
        ])
    }
}

/// Observed Steam compatibility runtime (Proton).
#[derive(Debug, Clone)]
pub enum CompatibilityRuntime {
    /// Runtime directory is missing or unusable.
    Unavailable {
        /// Expected path.
        path: String,
        /// Reason.
        reason: String,
    },
    /// Runtime directory was inventoried.
    Present {
        /// Runtime path.
        path: String,
        /// Identities of selected runtime files.
        files: Vec<FileIdentity>,
        /// Runtime version file.
        version: ReadObservation,
        /// `wine --version` probe.
        wine_version: CommandObservation,
        /// Steam runtime version files.
        steam_runtime_versions: Vec<ReadObservation>,
        /// Launch policy statements.
        launch_policy: Vec<String>,
    },
}

impl CompatibilityRuntime {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Unavailable { path, reason } => Json::object(vec![
                ("kind".to_owned(), Json::string("unavailable")),
                ("path".to_owned(), Json::string(path)),
                ("reason".to_owned(), Json::string(reason)),
            ]),
            Self::Present {
                path,
                files,
                version,
                wine_version,
                steam_runtime_versions,
                launch_policy,
            } => Json::object(vec![
                ("kind".to_owned(), Json::string("present")),
                ("path".to_owned(), Json::string(path)),
                (
                    "files".to_owned(),
                    Json::array(files.iter().map(FileIdentity::to_json).collect()),
                ),
                ("version".to_owned(), version.to_json()),
                ("wineVersion".to_owned(), wine_version.to_json()),
                (
                    "steamRuntimeVersions".to_owned(),
                    Json::array(steam_runtime_versions.iter().map(ReadObservation::to_json).collect()),
                ),
                (
                    "launchPolicy".to_owned(),
                    Json::array(launch_policy.iter().map(Json::string).collect()),
                ),
            ]),
        }
    }
}

/// Observed Steam installation state.
#[derive(Debug, Clone)]
pub struct SteamObservation {
    /// Steam `common` directory.
    pub common_path: String,
    /// Observed titles.
    pub titles: Vec<SteamTitleObservation>,
    /// Compatibility runtime state.
    pub compatibility_runtime: CompatibilityRuntime,
    /// Provenance basis statement.
    pub provenance_basis: String,
}

impl SteamObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("commonPath".to_owned(), Json::string(&self.common_path)),
            (
                "titles".to_owned(),
                Json::array(self.titles.iter().map(SteamTitleObservation::to_json).collect()),
            ),
            ("compatibilityRuntime".to_owned(), self.compatibility_runtime.to_json()),
            ("provenanceBasis".to_owned(), Json::string(&self.provenance_basis)),
        ])
    }
}

/// Source-census inputs referenced by the environment capture.
#[derive(Debug, Clone)]
pub struct CensusReference {
    /// Census definition file.
    pub definition: FileIdentity,
    /// Census manifest file, when present.
    pub manifest: Option<FileIdentity>,
}

impl CensusReference {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("definition".to_owned(), self.definition.to_json()),
            (
                "manifest".to_owned(),
                self.manifest.as_ref().map_or(Json::Null, FileIdentity::to_json),
            ),
        ])
    }
}

/// Discovery method and errors for the environment capture.
#[derive(Debug, Clone)]
pub struct Discovery {
    /// Searched roots.
    pub roots: Vec<String>,
    /// Discovery method description.
    pub method: String,
    /// Discovery errors.
    pub errors: Vec<String>,
}

impl Discovery {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            (
                "roots".to_owned(),
                Json::array(self.roots.iter().map(Json::string).collect()),
            ),
            ("method".to_owned(), Json::string(&self.method)),
            (
                "errors".to_owned(),
                Json::array(self.errors.iter().map(Json::string).collect()),
            ),
        ])
    }
}

/// The full reference-environment capture document.
#[derive(Debug, Clone)]
pub struct ReferenceEnvironment {
    /// Capture command.
    pub command: Vec<String>,
    /// ISO-8601 capture time.
    pub captured_at: String,
    /// Executing runtime binary identity.
    pub runtime: FileIdentity,
    /// Operating system platform.
    pub platform: String,
    /// CPU architecture.
    pub architecture: String,
    /// Runtime version string.
    pub runtime_version: String,
    /// Capture program file identities.
    pub capture_program: Vec<FileIdentity>,
    /// Source-census inputs.
    pub source_census: CensusReference,
    /// Inventoried sources.
    pub sources: Vec<SourceIdentity>,
    /// Observed tools.
    pub tools: Vec<ToolObservation>,
    /// Available library files.
    pub available_library_files: Vec<FileIdentity>,
    /// System file reads.
    pub system: Vec<ReadObservation>,
    /// Observed binaries.
    pub binaries: Vec<BinaryObservation>,
    /// Steam state.
    pub steam: SteamObservation,
    /// Discovery record.
    pub discovery: Discovery,
    /// Capture limits.
    pub limits: Vec<String>,
}

impl ReferenceEnvironment {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("capturedAt".to_owned(), Json::string(&self.captured_at)),
            (
                "command".to_owned(),
                Json::array(self.command.iter().map(Json::string).collect()),
            ),
            ("runtime".to_owned(), self.runtime.to_json()),
            ("platform".to_owned(), Json::string(&self.platform)),
            ("architecture".to_owned(), Json::string(&self.architecture)),
            ("bunVersion".to_owned(), Json::string(&self.runtime_version)),
            ("locale".to_owned(), Json::string("C")),
            ("identityHashAlgorithm".to_owned(), Json::string("sha256")),
            (
                "captureProgram".to_owned(),
                Json::array(self.capture_program.iter().map(FileIdentity::to_json).collect()),
            ),
            ("sourceCensus".to_owned(), self.source_census.to_json()),
            (
                "sources".to_owned(),
                Json::array(self.sources.iter().map(SourceIdentity::to_json).collect()),
            ),
            (
                "tools".to_owned(),
                Json::array(self.tools.iter().map(ToolObservation::to_json).collect()),
            ),
            (
                "availableLibraryFiles".to_owned(),
                Json::array(self.available_library_files.iter().map(FileIdentity::to_json).collect()),
            ),
            (
                "system".to_owned(),
                Json::array(self.system.iter().map(ReadObservation::to_json).collect()),
            ),
            (
                "binaries".to_owned(),
                Json::array(self.binaries.iter().map(BinaryObservation::to_json).collect()),
            ),
            ("steam".to_owned(), self.steam.to_json()),
            ("discovery".to_owned(), self.discovery.to_json()),
            (
                "limits".to_owned(),
                Json::array(self.limits.iter().map(Json::string).collect()),
            ),
        ])
    }
}
