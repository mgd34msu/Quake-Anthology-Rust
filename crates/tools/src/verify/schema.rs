//! Verification contracts and parsers.
//!
//! Donor provenance: `verification/schema/contracts.ts` (types) and
//! `verification/schema/parse.ts` (validators). Every contract parses from
//! [`Json`](crate::json::Json) and renders back so runners can round-trip
//! manifests, records, and reports.

use crate::error::ToolsError;
use crate::json::Json;
use crate::time::parse_iso;

/// Verification profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationProfile {
    /// Partial development selection.
    Dev,
    /// Integration selection.
    Integration,
    /// Full selection.
    Full,
    /// Release selection against a compiled executable.
    Release,
}

impl VerificationProfile {
    /// Wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Integration => "integration",
            Self::Full => "full",
            Self::Release => "release",
        }
    }
}

/// Attempt outcome status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    /// Passed.
    Pass,
    /// Failed.
    Fail,
    /// Blocked on missing inputs.
    BlockedMissingInput,
    /// Timed out.
    Timeout,
    /// Not run.
    NotRun,
}

impl VerificationStatus {
    /// Wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::BlockedMissingInput => "BLOCKED_MISSING_INPUT",
            Self::Timeout => "TIMEOUT",
            Self::NotRun => "NOT_RUN",
        }
    }
}

/// Required input kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Source file.
    Source,
    /// Reference data.
    Reference,
    /// Content bytes.
    Content,
    /// Executable.
    Executable,
    /// Schedule.
    Schedule,
}

impl InputKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Reference => "reference",
            Self::Content => "content",
            Self::Executable => "executable",
            Self::Schedule => "schedule",
        }
    }
}

/// Evidence kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceKind {
    /// Tooling evidence.
    Tooling,
    /// Component evidence.
    Component,
    /// Gameplay evidence.
    Gameplay,
    /// Release evidence.
    Release,
}

impl EvidenceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tooling => "tooling",
            Self::Component => "component",
            Self::Gameplay => "gameplay",
            Self::Release => "release",
        }
    }
}

/// Display isolation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// No display.
    Headless,
    /// Offscreen rendering.
    Offscreen,
    /// Private Xvfb display.
    Xvfb,
}

impl DisplayMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Headless => "headless",
            Self::Offscreen => "offscreen",
            Self::Xvfb => "xvfb",
        }
    }
}

/// Oracle provenance kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleKind {
    /// Original source.
    Source,
    /// Reference capture.
    Reference,
    /// Project oracle.
    Project,
}

impl OracleKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Reference => "reference",
            Self::Project => "project",
        }
    }
}

/// A required input file.
#[derive(Debug, Clone, PartialEq)]
pub struct InputRequirement {
    /// Requirement id.
    pub id: String,
    /// Input kind.
    pub kind: InputKind,
    /// Workspace-relative path.
    pub path: String,
    /// Pinned SHA-256, or `None` when unpinned.
    pub sha256: Option<String>,
}

/// Oracle identity for an expected contract.
#[derive(Debug, Clone, PartialEq)]
pub struct Oracle {
    /// Oracle kind.
    pub kind: OracleKind,
    /// Oracle identity.
    pub identity: String,
    /// Pinned oracle bytes, or `None` when unpinned.
    pub sha256: Option<String>,
}

/// A metric tolerance.
#[derive(Debug, Clone, PartialEq)]
pub struct Tolerance {
    /// Metric name.
    pub metric: String,
    /// Absolute tolerance.
    pub absolute: f64,
    /// Relative tolerance.
    pub relative: f64,
    /// Justification.
    pub justification: String,
}

/// An expected contract with its oracle and tolerances.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedContract {
    /// Contract id.
    pub id: String,
    /// Description.
    pub description: String,
    /// Oracle identity.
    pub oracle: Oracle,
    /// Minimum assertion count.
    pub minimum_assertions: i64,
    /// Metric tolerances.
    pub tolerances: Vec<Tolerance>,
}

/// An executable driver contract.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandContract {
    /// Executable with `{placeholder}` variables.
    pub executable: String,
    /// Arguments with `{placeholder}` variables.
    pub args: Vec<String>,
    /// Extra environment (reserved keys rejected).
    pub environment: Vec<(String, String)>,
    /// Timeout in milliseconds.
    pub timeout_ms: i64,
    /// Leased loopback ports.
    pub ports: i64,
    /// Display isolation mode.
    pub display: DisplayMode,
}

/// An expected verification case.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedCase {
    /// Case id.
    pub id: String,
    /// Configuration id.
    pub configuration_id: String,
    /// Suite id.
    pub suite_id: String,
    /// Evidence kind.
    pub evidence_kind: EvidenceKind,
    /// Configuration axes.
    pub configuration: Vec<(String, String)>,
    /// Selecting profiles.
    pub profiles: Vec<VerificationProfile>,
    /// Required inputs.
    pub requirements: Vec<InputRequirement>,
    /// Expected contracts.
    pub contracts: Vec<ExpectedContract>,
    /// Deterministic seed.
    pub seed: i64,
    /// Pinned clock schedule, or `None`.
    pub clock_schedule_sha256: Option<String>,
    /// Pinned network schedule, or `None`.
    pub network_schedule_sha256: Option<String>,
    /// Source paths selecting this case for `--changed`.
    pub source_paths: Vec<String>,
    /// Bound driver, or `None` when unbound.
    pub command: Option<CommandContract>,
}

/// A case manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseManifest {
    /// Manifest id.
    pub id: String,
    /// Required cases.
    pub cases: Vec<ExpectedCase>,
}

/// One composition axis value.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisValue {
    /// Value id.
    pub id: String,
    /// Extra requirements contributed by this value.
    pub requirements: Vec<InputRequirement>,
}

/// One composition axis.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionAxis {
    /// Axis id.
    pub id: String,
    /// Axis values.
    pub values: Vec<AxisValue>,
}

/// One composition suite.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionSuite {
    /// Suite id.
    pub id: String,
    /// Evidence kind.
    pub evidence_kind: EvidenceKind,
    /// Selecting profiles.
    pub profiles: Vec<VerificationProfile>,
    /// Expected contracts.
    pub contracts: Vec<ExpectedContract>,
    /// Bound driver, or `None`.
    pub command: Option<CommandContract>,
}

/// A case composition domain.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionDomain {
    /// Domain id.
    pub id: String,
    /// Composition axes.
    pub axes: Vec<CompositionAxis>,
    /// Shared requirements.
    pub requirements: Vec<InputRequirement>,
    /// Required suites.
    pub suites: Vec<CompositionSuite>,
    /// Deterministic seed.
    pub seed: i64,
    /// Pinned clock schedule, or `None`.
    pub clock_schedule_sha256: Option<String>,
    /// Pinned network schedule, or `None`.
    pub network_schedule_sha256: Option<String>,
    /// Source paths.
    pub source_paths: Vec<String>,
}

/// A recorded evidence artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    /// Output-relative path.
    pub path: String,
    /// SHA-256 of the recorded bytes.
    pub sha256: String,
    /// Byte length.
    pub bytes: i64,
}

/// A driver checkpoint observation.
#[derive(Debug, Clone, PartialEq)]
pub struct Checkpoint {
    /// Checkpoint id.
    pub id: String,
    /// ISO timestamp.
    pub at: String,
    /// Observations.
    pub observations: Json,
}

/// One assertion observation.
#[derive(Debug, Clone, PartialEq)]
pub struct AssertionObservation {
    /// Assertion id.
    pub id: String,
    /// Contract id.
    pub contract_id: String,
    /// Whether it passed.
    pub passed: bool,
    /// Expected value.
    pub expected: Json,
    /// Actual value.
    pub actual: Json,
}

/// Raw driver output.
#[derive(Debug, Clone, PartialEq)]
pub struct DriverOutput {
    /// Case id.
    pub case_id: String,
    /// Assertions.
    pub assertions: Vec<AssertionObservation>,
    /// Checkpoints.
    pub checkpoints: Vec<Checkpoint>,
    /// Declared artifact paths.
    pub artifact_paths: Vec<String>,
}

/// Fingerprints binding a record to its inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct Fingerprints {
    /// Manifest hash.
    pub manifest: String,
    /// Expected-case hash.
    pub expected_case: String,
    /// Source hash.
    pub source: String,
    /// Snapshot hash.
    pub snapshot: String,
    /// Executable hash.
    pub executable: String,
    /// Runtime executable hash.
    pub runtime_executable: String,
    /// Fixture hash.
    pub fixtures: String,
    /// Environment hash.
    pub environment: String,
    /// Clock schedule hash, or `None`.
    pub clock_schedule: Option<String>,
    /// Network schedule hash, or `None`.
    pub network_schedule: Option<String>,
    /// Seed.
    pub seed: i64,
}

/// Recorded runtime environment.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEnvironment {
    /// Platform.
    pub platform: String,
    /// Architecture.
    pub architecture: String,
    /// OS release.
    pub os_release: String,
    /// Runner runtime version.
    pub bun_version: String,
    /// CPU model.
    pub cpu: String,
    /// Loaded libraries.
    pub libraries: Vec<InputRequirement>,
    /// Environment variables.
    pub variables: Vec<(String, String)>,
}

/// A link to a previous attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptLink {
    /// Run id.
    pub run_id: String,
    /// Attempt id.
    pub attempt_id: String,
    /// Record hash.
    pub record_sha256: String,
}

/// Attempt provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptProvenance {
    /// Run id.
    pub run_id: String,
    /// Attempt id.
    pub attempt_id: String,
    /// Previous attempt, or `None`.
    pub previous_attempt: Option<AttemptLink>,
    /// Reused attempt, or `None`.
    pub reuse: Option<AttemptLink>,
}

/// Attempt outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Passed with exit code zero.
    Pass,
    /// Failed.
    Fail {
        /// Exit code, or `None` when no process ran.
        exit_code: Option<i64>,
        /// Failure reasons.
        reasons: Vec<String>,
    },
    /// Blocked on missing inputs.
    BlockedMissingInput {
        /// Missing inputs.
        missing_inputs: Vec<String>,
    },
    /// Timed out.
    Timeout {
        /// Timeout in milliseconds.
        timeout_ms: i64,
        /// Exit code, or `None`.
        exit_code: Option<i64>,
    },
    /// Not run.
    NotRun {
        /// Reason.
        reason: String,
    },
}

impl Outcome {
    /// Outcome status.
    #[must_use]
    pub fn status(&self) -> VerificationStatus {
        match self {
            Self::Pass => VerificationStatus::Pass,
            Self::Fail { .. } => VerificationStatus::Fail,
            Self::BlockedMissingInput { .. } => VerificationStatus::BlockedMissingInput,
            Self::Timeout { .. } => VerificationStatus::Timeout,
            Self::NotRun { .. } => VerificationStatus::NotRun,
        }
    }
}

/// An execution record.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionRecord {
    /// Case id.
    pub case_id: String,
    /// Configuration id.
    pub configuration_id: String,
    /// Suite id.
    pub suite_id: String,
    /// Evidence kind.
    pub evidence_kind: EvidenceKind,
    /// Recorded contracts.
    pub contracts: Vec<ExpectedContract>,
    /// Provenance.
    pub provenance: AttemptProvenance,
    /// Fingerprints.
    pub fingerprints: Fingerprints,
    /// Recorded inputs.
    pub inputs: Vec<InputRequirement>,
    /// Recorded environment.
    pub environment: RuntimeEnvironment,
    /// Bound driver, or `None`.
    pub command: Option<CommandContract>,
    /// Resolved command.
    pub resolved_command: Vec<String>,
    /// Output root.
    pub output_root: String,
    /// Start timestamp.
    pub started_at: String,
    /// Finish timestamp.
    pub finished_at: String,
    /// Duration in milliseconds.
    pub duration_ms: f64,
    /// Assertion count.
    pub assertion_count: i64,
    /// Assertions.
    pub assertions: Vec<AssertionObservation>,
    /// Checkpoints.
    pub checkpoints: Vec<Checkpoint>,
    /// Artifacts.
    pub artifacts: Vec<Artifact>,
    /// Outcome.
    pub outcome: Outcome,
}

/// Per-status counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StatusCounts {
    /// Pass count.
    pub pass: i64,
    /// Fail count.
    pub fail: i64,
    /// Blocked count.
    pub blocked: i64,
    /// Timeout count.
    pub timeout: i64,
    /// Not-run count.
    pub not_run: i64,
}

impl StatusCounts {
    /// Increment the bucket for `status`.
    pub fn add(&mut self, status: VerificationStatus) {
        match status {
            VerificationStatus::Pass => self.pass += 1,
            VerificationStatus::Fail => self.fail += 1,
            VerificationStatus::BlockedMissingInput => self.blocked += 1,
            VerificationStatus::Timeout => self.timeout += 1,
            VerificationStatus::NotRun => self.not_run += 1,
        }
    }
}

/// An invalid record with reasons.
#[derive(Debug, Clone, PartialEq)]
pub struct InvalidRecord {
    /// Case id.
    pub case_id: String,
    /// Reasons.
    pub reasons: Vec<String>,
}

/// Manifest/record reconciliation.
#[derive(Debug, Clone, PartialEq)]
pub struct Reconciliation {
    /// Manifest id.
    pub manifest_id: String,
    /// Manifest hash.
    pub manifest_sha256: String,
    /// Expected case count.
    pub expected: i64,
    /// Record count.
    pub records: i64,
    /// Per-status counts.
    pub counts: StatusCounts,
    /// Missing case ids.
    pub missing_case_ids: Vec<String>,
    /// Unexpected case ids.
    pub unexpected_case_ids: Vec<String>,
    /// Duplicate case ids.
    pub duplicate_case_ids: Vec<String>,
    /// Invalid records.
    pub invalid_records: Vec<InvalidRecord>,
    /// Whether the manifest is complete.
    pub complete: bool,
    /// Whether gameplay evidence is complete.
    pub gameplay_complete: bool,
}

/// Require an object.
pub fn object<'a>(value: &'a Json, label: &str) -> Result<&'a [(String, Json)], ToolsError> {
    value.as_object().ok_or_else(|| ToolsError::parse(format!("{label} must be an object")))
}

/// Require an array.
pub fn list<'a>(value: &'a Json, label: &str) -> Result<&'a [Json], ToolsError> {
    value.as_array().ok_or_else(|| ToolsError::parse(format!("{label} must be an array")))
}

fn member<'a>(members: &'a [(String, Json)], key: &str, label: &str) -> Result<&'a Json, ToolsError> {
    members
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
        .ok_or_else(|| ToolsError::parse(format!("{label} is missing")))
}

/// Require a nonempty string.
pub fn string(value: &Json, label: &str) -> Result<String, ToolsError> {
    match value.as_str() {
        Some(text) if !text.is_empty() => Ok(text.to_owned()),
        _ => Err(ToolsError::parse(format!("{label} must be a nonempty string"))),
    }
}

fn text(value: &Json, label: &str) -> Result<String, ToolsError> {
    value.as_str().map(str::to_owned).ok_or_else(|| ToolsError::parse(format!("{label} must be a string")))
}

fn number(value: &Json, label: &str, minimum: f64) -> Result<f64, ToolsError> {
    match value.as_f64() {
        Some(parsed) if parsed.is_finite() && parsed >= minimum => Ok(parsed),
        _ => Err(ToolsError::parse(format!("{label} must be finite and >= {minimum}"))),
    }
}

fn integer(value: &Json, label: &str, minimum: i64) -> Result<i64, ToolsError> {
    let parsed = number(value, label, minimum as f64)?;
    if parsed.fract() != 0.0 || parsed.abs() > 9_007_199_254_740_991.0 {
        return Err(ToolsError::parse(format!("{label} must be a safe integer")));
    }
    let result = parsed as i64;
    if result < minimum {
        return Err(ToolsError::parse(format!("{label} must be finite and >= {minimum}")));
    }
    Ok(result)
}

/// Require a lowercase SHA-256 digest.
pub fn sha256_hash(value: &Json, label: &str) -> Result<String, ToolsError> {
    let parsed = string(value, label)?;
    if parsed.len() != 64 || !parsed.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()) {
        return Err(ToolsError::parse(format!("{label} must be a lowercase SHA-256")));
    }
    Ok(parsed)
}

fn nullable_hash(value: &Json, label: &str) -> Result<Option<String>, ToolsError> {
    if value.is_null() {
        Ok(None)
    } else {
        sha256_hash(value, label).map(Some)
    }
}

fn version(value: &Json) -> Result<(), ToolsError> {
    match value.as_f64() {
        Some(1.0) => Ok(()),
        _ => Err(ToolsError::parse("Unsupported verification schema version")),
    }
}

fn strings(value: &Json, label: &str) -> Result<Vec<String>, ToolsError> {
    list(value, label)?.iter().map(|item| string(item, label)).collect()
}

fn string_record(value: &Json, label: &str, allow_empty: bool) -> Result<Vec<(String, String)>, ToolsError> {
    let mut pairs = Vec::new();
    for (key, item) in object(value, label)? {
        if key.is_empty() {
            return Err(ToolsError::parse(format!("{label} must be a nonempty string")));
        }
        let text = if allow_empty { text(item, label)? } else { string(item, label)? };
        pairs.push((key.clone(), text));
    }
    Ok(pairs)
}

/// Parse a verification profile.
pub fn profile(value: &Json) -> Result<VerificationProfile, ToolsError> {
    match value.as_str() {
        Some("dev") => Ok(VerificationProfile::Dev),
        Some("integration") => Ok(VerificationProfile::Integration),
        Some("full") => Ok(VerificationProfile::Full),
        Some("release") => Ok(VerificationProfile::Release),
        _ => Err(ToolsError::parse(format!("Unsupported verification profile {}", value.render()))),
    }
}

fn input_kind(value: &Json) -> Result<InputKind, ToolsError> {
    match value.as_str() {
        Some("source") => Ok(InputKind::Source),
        Some("reference") => Ok(InputKind::Reference),
        Some("content") => Ok(InputKind::Content),
        Some("executable") => Ok(InputKind::Executable),
        Some("schedule") => Ok(InputKind::Schedule),
        _ => Err(ToolsError::parse(format!("Unsupported input kind {}", value.render()))),
    }
}

fn evidence_kind(value: &Json) -> Result<EvidenceKind, ToolsError> {
    match value.as_str() {
        Some("tooling") => Ok(EvidenceKind::Tooling),
        Some("component") => Ok(EvidenceKind::Component),
        Some("gameplay") => Ok(EvidenceKind::Gameplay),
        Some("release") => Ok(EvidenceKind::Release),
        _ => Err(ToolsError::parse(format!("Unsupported evidence kind {}", value.render()))),
    }
}

/// Parse an input requirement.
pub fn parse_requirement(value: &Json) -> Result<InputRequirement, ToolsError> {
    let input = object(value, "input requirement")?;
    Ok(InputRequirement {
        id: string(member(input, "id", "input ID")?, "input ID")?,
        kind: input_kind(member(input, "kind", "input kind")?)?,
        path: string(member(input, "path", "input path")?, "input path")?,
        sha256: nullable_hash(member(input, "sha256", "input SHA-256")?, "input SHA-256")?,
    })
}

/// Parse an expected contract.
pub fn parse_contract(value: &Json) -> Result<ExpectedContract, ToolsError> {
    let contract = object(value, "expected contract")?;
    let oracle = object(member(contract, "oracle", "oracle")?, "oracle")?;
    let kind = match member(oracle, "kind", "oracle kind")?.as_str() {
        Some("source") => OracleKind::Source,
        Some("reference") => OracleKind::Reference,
        Some("project") => OracleKind::Project,
        _ => return Err(ToolsError::parse("Unknown oracle kind")),
    };
    let mut tolerances = Vec::new();
    for item in list(member(contract, "tolerances", "tolerances")?, "tolerances")? {
        let tolerance = object(item, "tolerance")?;
        tolerances.push(Tolerance {
            metric: string(member(tolerance, "metric", "metric")?, "metric")?,
            absolute: number(member(tolerance, "absolute", "absolute tolerance")?, "absolute tolerance", 0.0)?,
            relative: number(member(tolerance, "relative", "relative tolerance")?, "relative tolerance", 0.0)?,
            justification: string(member(tolerance, "justification", "tolerance justification")?, "tolerance justification")?,
        });
    }
    Ok(ExpectedContract {
        id: string(member(contract, "id", "contract ID")?, "contract ID")?,
        description: string(member(contract, "description", "contract description")?, "contract description")?,
        oracle: Oracle {
            kind,
            identity: string(member(oracle, "identity", "oracle identity")?, "oracle identity")?,
            sha256: nullable_hash(member(oracle, "sha256", "oracle SHA-256")?, "oracle SHA-256")?,
        },
        minimum_assertions: integer(member(contract, "minimumAssertions", "minimum assertions")?, "minimum assertions", 1)?,
        tolerances,
    })
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(ch) if ch.is_ascii_alphabetic() || ch == '_' => {}
        _ => return false,
    }
    if !chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return false;
    }
    if key == "HOME" || key == "TMPDIR" || key == "DISPLAY" || key == "WAYLAND_DISPLAY" {
        return false;
    }
    if key.starts_with("XDG_") || key.starts_with("VERIFY_") {
        return false;
    }
    true
}

/// Parse a command contract (or `None` for null).
pub fn parse_command(value: &Json) -> Result<Option<CommandContract>, ToolsError> {
    if value.is_null() {
        return Ok(None);
    }
    let command = object(value, "command")?;
    let display = match member(command, "display", "display")?.as_str() {
        Some("headless") => DisplayMode::Headless,
        Some("offscreen") => DisplayMode::Offscreen,
        Some("xvfb") => DisplayMode::Xvfb,
        _ => return Err(ToolsError::parse("Unknown display isolation mode")),
    };
    let ports = integer(member(command, "ports", "ports")?, "ports", 0)?;
    if ports > 64 {
        return Err(ToolsError::parse("A case may lease at most 64 ports"));
    }
    let environment = string_record(member(command, "environment", "command environment")?, "command environment", true)?;
    for (key, _) in &environment {
        if !valid_env_key(key) {
            return Err(ToolsError::parse(format!("Reserved or invalid command environment key {key}")));
        }
    }
    let mut args = Vec::new();
    for item in list(member(command, "args", "arguments")?, "arguments")? {
        args.push(text(item, "argument")?);
    }
    Ok(Some(CommandContract {
        executable: string(member(command, "executable", "executable")?, "executable")?,
        args,
        environment,
        timeout_ms: integer(member(command, "timeoutMs", "timeout")?, "timeout", 1)?,
        ports,
        display,
    }))
}

/// Parse an expected case.
pub fn parse_case(value: &Json) -> Result<ExpectedCase, ToolsError> {
    let item = object(value, "expected case")?;
    let contracts: Vec<ExpectedContract> = list(member(item, "contracts", "contracts")?, "contracts")?
        .iter()
        .map(parse_contract)
        .collect::<Result<_, _>>()?;
    if contracts.is_empty() {
        return Err(ToolsError::parse("Expected case must have a contract"));
    }
    let profiles: Vec<VerificationProfile> =
        list(member(item, "profiles", "profiles")?, "profiles")?.iter().map(profile).collect::<Result<_, _>>()?;
    if profiles.is_empty() {
        return Err(ToolsError::parse("Expected case must have a profile"));
    }
    let result = ExpectedCase {
        id: string(member(item, "id", "case ID")?, "case ID")?,
        configuration_id: string(member(item, "configurationId", "configuration ID")?, "configuration ID")?,
        suite_id: string(member(item, "suiteId", "suite ID")?, "suite ID")?,
        evidence_kind: evidence_kind(member(item, "evidenceKind", "evidence kind")?)?,
        configuration: string_record(member(item, "configuration", "configuration")?, "configuration", false)?,
        profiles,
        requirements: list(member(item, "requirements", "requirements")?, "requirements")?
            .iter()
            .map(parse_requirement)
            .collect::<Result<_, _>>()?,
        contracts,
        seed: integer(member(item, "seed", "seed")?, "seed", 0)?,
        clock_schedule_sha256: nullable_hash(member(item, "clockScheduleSha256", "clock schedule hash")?, "clock schedule hash")?,
        network_schedule_sha256: nullable_hash(member(item, "networkScheduleSha256", "network schedule hash")?, "network schedule hash")?,
        source_paths: strings(member(item, "sourcePaths", "source paths")?, "source paths")?,
        command: parse_command(member(item, "command", "command")?)?,
    };
    unique_ids(result.contracts.iter().map(|contract| contract.id.as_str()), "contract")?;
    unique_ids(result.requirements.iter().map(|requirement| requirement.id.as_str()), "requirement")?;
    Ok(result)
}

/// Reject duplicate ids.
pub fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>, label: &str) -> Result<(), ToolsError> {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(ToolsError::parse(format!("Duplicate {label} ID {id}")));
        }
    }
    Ok(())
}

/// Parse a case manifest.
pub fn parse_manifest(value: &Json) -> Result<CaseManifest, ToolsError> {
    let manifest = object(value, "case manifest")?;
    let cases: Vec<ExpectedCase> = list(member(manifest, "cases", "cases")?, "cases")?.iter().map(parse_case).collect::<Result<_, _>>()?;
    if cases.is_empty() {
        return Err(ToolsError::parse("Case manifest must retain at least one required case"));
    }
    unique_ids(cases.iter().map(|case| case.id.as_str()), "case")?;
    version(member(manifest, "schemaVersion", "schema version")?)?;
    Ok(CaseManifest {
        id: string(member(manifest, "id", "manifest ID")?, "manifest ID")?,
        cases,
    })
}

/// Parse a composition domain.
pub fn parse_domain(value: &Json) -> Result<CompositionDomain, ToolsError> {
    let domain = object(value, "composition domain")?;
    let mut axes = Vec::new();
    for item in list(member(domain, "axes", "axes")?, "axes")? {
        let axis = object(item, "axis")?;
        let mut values = Vec::new();
        for value in list(member(axis, "values", "axis values")?, "axis values")? {
            let entry = object(value, "axis value")?;
            values.push(AxisValue {
                id: string(member(entry, "id", "axis value ID")?, "axis value ID")?,
                requirements: list(member(entry, "requirements", "axis requirements")?, "axis requirements")?
                    .iter()
                    .map(parse_requirement)
                    .collect::<Result<_, _>>()?,
            });
        }
        axes.push(CompositionAxis {
            id: string(member(axis, "id", "axis ID")?, "axis ID")?,
            values,
        });
    }
    let mut suites = Vec::new();
    for item in list(member(domain, "suites", "suites")?, "suites")? {
        let suite = object(item, "suite")?;
        suites.push(CompositionSuite {
            id: string(member(suite, "id", "suite ID")?, "suite ID")?,
            evidence_kind: evidence_kind(member(suite, "evidenceKind", "suite evidence kind")?)?,
            profiles: list(member(suite, "profiles", "suite profiles")?, "suite profiles")?.iter().map(profile).collect::<Result<_, _>>()?,
            contracts: list(member(suite, "contracts", "suite contracts")?, "suite contracts")?
                .iter()
                .map(parse_contract)
                .collect::<Result<_, _>>()?,
            command: parse_command(member(suite, "command", "suite command")?)?,
        });
    }
    let result = CompositionDomain {
        id: string(member(domain, "id", "domain ID")?, "domain ID")?,
        axes,
        requirements: list(member(domain, "requirements", "requirements")?, "requirements")?
            .iter()
            .map(parse_requirement)
            .collect::<Result<_, _>>()?,
        suites,
        seed: integer(member(domain, "seed", "seed")?, "seed", 0)?,
        clock_schedule_sha256: nullable_hash(member(domain, "clockScheduleSha256", "clock schedule hash")?, "clock schedule hash")?,
        network_schedule_sha256: nullable_hash(member(domain, "networkScheduleSha256", "network schedule hash")?, "network schedule hash")?,
        source_paths: strings(member(domain, "sourcePaths", "source paths")?, "source paths")?,
    };
    version(member(domain, "schemaVersion", "schema version")?)?;
    crate::verify::product::validate_domain(&result)?;
    Ok(result)
}

fn check_finite(value: &Json) -> Result<(), ToolsError> {
    match value {
        Json::Number(number) if !number.value().is_finite() => Err(ToolsError::parse("Observation must be finite JSON data")),
        Json::Array(items) => items.iter().try_for_each(check_finite),
        Json::Object(members) => members.iter().try_for_each(|(_, member)| check_finite(member)),
        _ => Ok(()),
    }
}

/// Validate an observation value (finite JSON data).
pub fn json_value(value: &Json) -> Result<Json, ToolsError> {
    check_finite(value)?;
    Ok(value.clone())
}

fn parse_assertion(value: &Json) -> Result<AssertionObservation, ToolsError> {
    let assertion = object(value, "assertion")?;
    let passed = member(assertion, "passed", "assertion passed")?
        .as_bool()
        .ok_or_else(|| ToolsError::parse("Assertion passed must be boolean"))?;
    Ok(AssertionObservation {
        id: string(member(assertion, "id", "assertion ID")?, "assertion ID")?,
        contract_id: string(member(assertion, "contractId", "assertion contract ID")?, "assertion contract ID")?,
        passed,
        expected: json_value(member(assertion, "expected", "assertion expected")?)?,
        actual: json_value(member(assertion, "actual", "assertion actual")?)?,
    })
}

fn timestamp(value: &Json, label: &str) -> Result<String, ToolsError> {
    let parsed = string(value, label)?;
    parse_iso(&parsed).map_err(|_| ToolsError::parse(format!("{label} must be an ISO UTC timestamp")))?;
    Ok(parsed)
}

fn parse_checkpoint(value: &Json) -> Result<Checkpoint, ToolsError> {
    let checkpoint = object(value, "checkpoint")?;
    Ok(Checkpoint {
        id: string(member(checkpoint, "id", "checkpoint ID")?, "checkpoint ID")?,
        at: timestamp(member(checkpoint, "at", "checkpoint timestamp")?, "checkpoint timestamp")?,
        observations: json_value(member(checkpoint, "observations", "checkpoint observations")?)?,
    })
}

/// Parse raw driver output.
pub fn parse_driver_output(value: &Json) -> Result<DriverOutput, ToolsError> {
    let output = object(value, "driver output")?;
    for (key, _) in output {
        if !["schemaVersion", "caseId", "assertions", "checkpoints", "artifactPaths"].contains(&key.as_str()) {
            return Err(ToolsError::parse(format!("Unknown driver output field {key}; status and skips come from the runner")));
        }
    }
    let assertions: Vec<AssertionObservation> = list(member(output, "assertions", "assertions")?, "assertions")?
        .iter()
        .map(parse_assertion)
        .collect::<Result<_, _>>()?;
    let checkpoints: Vec<Checkpoint> = list(member(output, "checkpoints", "checkpoints")?, "checkpoints")?
        .iter()
        .map(parse_checkpoint)
        .collect::<Result<_, _>>()?;
    unique_ids(assertions.iter().map(|assertion| assertion.id.as_str()), "assertion")?;
    unique_ids(checkpoints.iter().map(|checkpoint| checkpoint.id.as_str()), "checkpoint")?;
    version(member(output, "schemaVersion", "schema version")?)?;
    Ok(DriverOutput {
        case_id: string(member(output, "caseId", "driver case ID")?, "driver case ID")?,
        assertions,
        checkpoints,
        artifact_paths: strings(member(output, "artifactPaths", "artifact paths")?, "artifact paths")?,
    })
}

fn parse_artifact(value: &Json) -> Result<Artifact, ToolsError> {
    let artifact = object(value, "artifact")?;
    Ok(Artifact {
        path: string(member(artifact, "path", "artifact path")?, "artifact path")?,
        sha256: sha256_hash(member(artifact, "sha256", "artifact hash")?, "artifact hash")?,
        bytes: integer(member(artifact, "bytes", "artifact bytes")?, "artifact bytes", 0)?,
    })
}

fn parse_fingerprints(value: &Json) -> Result<Fingerprints, ToolsError> {
    let hashes = object(value, "fingerprints")?;
    Ok(Fingerprints {
        manifest: sha256_hash(member(hashes, "manifest", "manifest hash")?, "manifest hash")?,
        expected_case: sha256_hash(member(hashes, "expectedCase", "case hash")?, "case hash")?,
        source: sha256_hash(member(hashes, "source", "source hash")?, "source hash")?,
        snapshot: sha256_hash(member(hashes, "snapshot", "snapshot hash")?, "snapshot hash")?,
        executable: sha256_hash(member(hashes, "executable", "executable hash")?, "executable hash")?,
        runtime_executable: sha256_hash(member(hashes, "runtimeExecutable", "runtime executable hash")?, "runtime executable hash")?,
        fixtures: sha256_hash(member(hashes, "fixtures", "fixture hash")?, "fixture hash")?,
        environment: sha256_hash(member(hashes, "environment", "environment hash")?, "environment hash")?,
        clock_schedule: nullable_hash(member(hashes, "clockSchedule", "clock hash")?, "clock hash")?,
        network_schedule: nullable_hash(member(hashes, "networkSchedule", "network hash")?, "network hash")?,
        seed: integer(member(hashes, "seed", "fingerprint seed")?, "fingerprint seed", 0)?,
    })
}

fn parse_environment(value: &Json) -> Result<RuntimeEnvironment, ToolsError> {
    let environment = object(value, "runtime environment")?;
    Ok(RuntimeEnvironment {
        platform: string(member(environment, "platform", "platform")?, "platform")?,
        architecture: string(member(environment, "architecture", "architecture")?, "architecture")?,
        os_release: string(member(environment, "osRelease", "OS release")?, "OS release")?,
        bun_version: string(member(environment, "bunVersion", "Bun version")?, "Bun version")?,
        cpu: string(member(environment, "cpu", "CPU")?, "CPU")?,
        libraries: list(member(environment, "libraries", "libraries")?, "libraries")?.iter().map(parse_requirement).collect::<Result<_, _>>()?,
        variables: string_record(member(environment, "variables", "environment variables")?, "environment variables", true)?,
    })
}

fn parse_attempt_link(value: &Json) -> Result<Option<AttemptLink>, ToolsError> {
    if value.is_null() {
        return Ok(None);
    }
    let link = object(value, "attempt link")?;
    Ok(Some(AttemptLink {
        run_id: string(member(link, "runId", "previous run ID")?, "previous run ID")?,
        attempt_id: string(member(link, "attemptId", "previous attempt ID")?, "previous attempt ID")?,
        record_sha256: sha256_hash(member(link, "recordSha256", "record hash")?, "record hash")?,
    }))
}

fn parse_outcome(value: &Json) -> Result<Outcome, ToolsError> {
    let outcome = object(value, "outcome")?;
    let status = member(outcome, "status", "outcome status")?;
    match status.as_str() {
        Some("PASS") => {
            if member(outcome, "exitCode", "exit code")?.as_f64() != Some(0.0) {
                return Err(ToolsError::parse("PASS requires exit code zero"));
            }
            Ok(Outcome::Pass)
        }
        Some("FAIL") => {
            let exit_code = member(outcome, "exitCode", "exit code")?;
            Ok(Outcome::Fail {
                exit_code: if exit_code.is_null() {
                    None
                } else {
                    Some(integer(exit_code, "exit code", -2_147_483_648)?)
                },
                reasons: strings(member(outcome, "reasons", "failure reasons")?, "failure reasons")?,
            })
        }
        Some("TIMEOUT") => {
            let exit_code = member(outcome, "exitCode", "exit code")?;
            Ok(Outcome::Timeout {
                timeout_ms: integer(member(outcome, "timeoutMs", "timeout")?, "timeout", 1)?,
                exit_code: if exit_code.is_null() {
                    None
                } else {
                    Some(integer(exit_code, "exit code", -2_147_483_648)?)
                },
            })
        }
        Some("BLOCKED_MISSING_INPUT") => Ok(Outcome::BlockedMissingInput {
            missing_inputs: strings(member(outcome, "missingInputs", "missing inputs")?, "missing inputs")?,
        }),
        Some("NOT_RUN") => Ok(Outcome::NotRun {
            reason: string(member(outcome, "reason", "not-run reason")?, "not-run reason")?,
        }),
        _ => Err(ToolsError::parse(format!("Unknown verification status {}; skips are not accepted", status.render()))),
    }
}

/// Parse an execution record.
pub fn parse_record(value: &Json) -> Result<ExecutionRecord, ToolsError> {
    let record = object(value, "execution record")?;
    let provenance = object(member(record, "provenance", "provenance")?, "provenance")?;
    let resolved: Vec<String> = list(member(record, "resolvedCommand", "resolved command")?, "resolved command")?
        .iter()
        .enumerate()
        .map(|(index, item)| if index == 0 { string(item, "resolved executable") } else { text(item, "resolved argument") })
        .collect::<Result<_, _>>()?;
    let parsed = ExecutionRecord {
        case_id: string(member(record, "caseId", "case ID")?, "case ID")?,
        configuration_id: string(member(record, "configurationId", "configuration ID")?, "configuration ID")?,
        suite_id: string(member(record, "suiteId", "suite ID")?, "suite ID")?,
        evidence_kind: evidence_kind(member(record, "evidenceKind", "record evidence kind")?)?,
        contracts: list(member(record, "contracts", "record contracts")?, "record contracts")?
            .iter()
            .map(parse_contract)
            .collect::<Result<_, _>>()?,
        provenance: AttemptProvenance {
            run_id: string(member(provenance, "runId", "run ID")?, "run ID")?,
            attempt_id: string(member(provenance, "attemptId", "attempt ID")?, "attempt ID")?,
            previous_attempt: parse_attempt_link(member(provenance, "previousAttempt", "previous attempt")?)?,
            reuse: parse_attempt_link(member(provenance, "reuse", "reuse")?)?,
        },
        fingerprints: parse_fingerprints(member(record, "fingerprints", "fingerprints")?)?,
        inputs: list(member(record, "inputs", "record inputs")?, "record inputs")?.iter().map(parse_requirement).collect::<Result<_, _>>()?,
        environment: parse_environment(member(record, "environment", "record environment")?)?,
        command: parse_command(member(record, "command", "record command")?)?,
        resolved_command: resolved,
        output_root: string(member(record, "outputRoot", "output root")?, "output root")?,
        started_at: timestamp(member(record, "startedAt", "start timestamp")?, "start timestamp")?,
        finished_at: timestamp(member(record, "finishedAt", "finish timestamp")?, "finish timestamp")?,
        duration_ms: number(member(record, "durationMs", "duration")?, "duration", 0.0)?,
        assertion_count: integer(member(record, "assertionCount", "assertion count")?, "assertion count", 0)?,
        assertions: list(member(record, "assertions", "record assertions")?, "record assertions")?
            .iter()
            .map(parse_assertion)
            .collect::<Result<_, _>>()?,
        checkpoints: list(member(record, "checkpoints", "record checkpoints")?, "record checkpoints")?
            .iter()
            .map(parse_checkpoint)
            .collect::<Result<_, _>>()?,
        artifacts: list(member(record, "artifacts", "record artifacts")?, "record artifacts")?.iter().map(parse_artifact).collect::<Result<_, _>>()?,
        outcome: parse_outcome(member(record, "outcome", "outcome")?)?,
    };
    version(member(record, "schemaVersion", "schema version")?)?;
    if parsed.outcome.status() == VerificationStatus::Pass
        && (parsed.fingerprints.clock_schedule.is_none() || parsed.fingerprints.network_schedule.is_none())
    {
        return Err(ToolsError::parse("PASS requires pinned clock and network schedules"));
    }
    Ok(parsed)
}

fn opt_hash(value: &Option<String>) -> Json {
    value.as_deref().map_or(Json::Null, Json::string)
}

impl InputRequirement {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("kind".to_owned(), Json::string(self.kind.as_str())),
            ("path".to_owned(), Json::string(&self.path)),
            ("sha256".to_owned(), opt_hash(&self.sha256)),
        ])
    }
}

impl ExpectedContract {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("description".to_owned(), Json::string(&self.description)),
            (
                "oracle".to_owned(),
                Json::object(vec![
                    ("kind".to_owned(), Json::string(self.oracle.kind.as_str())),
                    ("identity".to_owned(), Json::string(&self.oracle.identity)),
                    ("sha256".to_owned(), opt_hash(&self.oracle.sha256)),
                ]),
            ),
            ("minimumAssertions".to_owned(), Json::int(self.minimum_assertions)),
            (
                "tolerances".to_owned(),
                Json::array(
                    self.tolerances
                        .iter()
                        .map(|tolerance| {
                            Json::object(vec![
                                ("metric".to_owned(), Json::string(&tolerance.metric)),
                                ("absolute".to_owned(), Json::float(tolerance.absolute)),
                                ("relative".to_owned(), Json::float(tolerance.relative)),
                                ("justification".to_owned(), Json::string(&tolerance.justification)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
}

impl CommandContract {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("executable".to_owned(), Json::string(&self.executable)),
            ("args".to_owned(), Json::array(self.args.iter().map(Json::string).collect())),
            (
                "environment".to_owned(),
                Json::object(self.environment.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect()),
            ),
            ("timeoutMs".to_owned(), Json::int(self.timeout_ms)),
            ("ports".to_owned(), Json::int(self.ports)),
            ("display".to_owned(), Json::string(self.display.as_str())),
        ])
    }
}

impl ExpectedCase {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("configurationId".to_owned(), Json::string(&self.configuration_id)),
            ("suiteId".to_owned(), Json::string(&self.suite_id)),
            ("evidenceKind".to_owned(), Json::string(self.evidence_kind.as_str())),
            (
                "configuration".to_owned(),
                Json::object(self.configuration.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect()),
            ),
            ("profiles".to_owned(), Json::array(self.profiles.iter().map(|profile| Json::string(profile.as_str())).collect())),
            ("requirements".to_owned(), Json::array(self.requirements.iter().map(InputRequirement::to_json).collect())),
            ("contracts".to_owned(), Json::array(self.contracts.iter().map(ExpectedContract::to_json).collect())),
            ("seed".to_owned(), Json::int(self.seed)),
            ("clockScheduleSha256".to_owned(), opt_hash(&self.clock_schedule_sha256)),
            ("networkScheduleSha256".to_owned(), opt_hash(&self.network_schedule_sha256)),
            ("sourcePaths".to_owned(), Json::array(self.source_paths.iter().map(Json::string).collect())),
            (
                "command".to_owned(),
                self.command.as_ref().map_or(Json::Null, CommandContract::to_json),
            ),
        ])
    }
}

impl CaseManifest {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("id".to_owned(), Json::string(&self.id)),
            ("cases".to_owned(), Json::array(self.cases.iter().map(ExpectedCase::to_json).collect())),
        ])
    }
}

impl AssertionObservation {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("contractId".to_owned(), Json::string(&self.contract_id)),
            ("passed".to_owned(), Json::boolean(self.passed)),
            ("expected".to_owned(), self.expected.clone()),
            ("actual".to_owned(), self.actual.clone()),
        ])
    }
}

impl Checkpoint {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("id".to_owned(), Json::string(&self.id)),
            ("at".to_owned(), Json::string(&self.at)),
            ("observations".to_owned(), self.observations.clone()),
        ])
    }
}

impl Artifact {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("path".to_owned(), Json::string(&self.path)),
            ("sha256".to_owned(), Json::string(&self.sha256)),
            ("bytes".to_owned(), Json::int(self.bytes)),
        ])
    }
}

impl Fingerprints {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("manifest".to_owned(), Json::string(&self.manifest)),
            ("expectedCase".to_owned(), Json::string(&self.expected_case)),
            ("source".to_owned(), Json::string(&self.source)),
            ("snapshot".to_owned(), Json::string(&self.snapshot)),
            ("executable".to_owned(), Json::string(&self.executable)),
            ("runtimeExecutable".to_owned(), Json::string(&self.runtime_executable)),
            ("fixtures".to_owned(), Json::string(&self.fixtures)),
            ("environment".to_owned(), Json::string(&self.environment)),
            ("clockSchedule".to_owned(), opt_hash(&self.clock_schedule)),
            ("networkSchedule".to_owned(), opt_hash(&self.network_schedule)),
            ("seed".to_owned(), Json::int(self.seed)),
        ])
    }
}

impl RuntimeEnvironment {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("platform".to_owned(), Json::string(&self.platform)),
            ("architecture".to_owned(), Json::string(&self.architecture)),
            ("osRelease".to_owned(), Json::string(&self.os_release)),
            ("bunVersion".to_owned(), Json::string(&self.bun_version)),
            ("cpu".to_owned(), Json::string(&self.cpu)),
            ("libraries".to_owned(), Json::array(self.libraries.iter().map(InputRequirement::to_json).collect())),
            (
                "variables".to_owned(),
                Json::object(self.variables.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect()),
            ),
        ])
    }
}

impl AttemptLink {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("runId".to_owned(), Json::string(&self.run_id)),
            ("attemptId".to_owned(), Json::string(&self.attempt_id)),
            ("recordSha256".to_owned(), Json::string(&self.record_sha256)),
        ])
    }
}

impl AttemptProvenance {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("runId".to_owned(), Json::string(&self.run_id)),
            ("attemptId".to_owned(), Json::string(&self.attempt_id)),
            (
                "previousAttempt".to_owned(),
                self.previous_attempt.as_ref().map_or(Json::Null, AttemptLink::to_json),
            ),
            ("reuse".to_owned(), self.reuse.as_ref().map_or(Json::Null, AttemptLink::to_json)),
        ])
    }
}

impl Outcome {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut pairs = vec![("status".to_owned(), Json::string(self.status().as_str()))];
        match self {
            Self::Pass => pairs.push(("exitCode".to_owned(), Json::int(0))),
            Self::Fail { exit_code, reasons } => {
                pairs.push(("exitCode".to_owned(), exit_code.map_or(Json::Null, Json::int)));
                pairs.push(("reasons".to_owned(), Json::array(reasons.iter().map(Json::string).collect())));
            }
            Self::BlockedMissingInput { missing_inputs } => {
                pairs.push(("missingInputs".to_owned(), Json::array(missing_inputs.iter().map(Json::string).collect())));
            }
            Self::Timeout { timeout_ms, exit_code } => {
                pairs.push(("timeoutMs".to_owned(), Json::int(*timeout_ms)));
                pairs.push(("exitCode".to_owned(), exit_code.map_or(Json::Null, Json::int)));
            }
            Self::NotRun { reason } => pairs.push(("reason".to_owned(), Json::string(reason))),
        }
        Json::object(pairs)
    }
}

impl ExecutionRecord {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("caseId".to_owned(), Json::string(&self.case_id)),
            ("configurationId".to_owned(), Json::string(&self.configuration_id)),
            ("suiteId".to_owned(), Json::string(&self.suite_id)),
            ("evidenceKind".to_owned(), Json::string(self.evidence_kind.as_str())),
            ("contracts".to_owned(), Json::array(self.contracts.iter().map(ExpectedContract::to_json).collect())),
            ("provenance".to_owned(), self.provenance.to_json()),
            ("fingerprints".to_owned(), self.fingerprints.to_json()),
            ("inputs".to_owned(), Json::array(self.inputs.iter().map(InputRequirement::to_json).collect())),
            ("environment".to_owned(), self.environment.to_json()),
            ("command".to_owned(), self.command.as_ref().map_or(Json::Null, CommandContract::to_json)),
            ("resolvedCommand".to_owned(), Json::array(self.resolved_command.iter().map(Json::string).collect())),
            ("outputRoot".to_owned(), Json::string(&self.output_root)),
            ("startedAt".to_owned(), Json::string(&self.started_at)),
            ("finishedAt".to_owned(), Json::string(&self.finished_at)),
            ("durationMs".to_owned(), Json::float(self.duration_ms)),
            ("assertionCount".to_owned(), Json::int(self.assertion_count)),
            ("assertions".to_owned(), Json::array(self.assertions.iter().map(AssertionObservation::to_json).collect())),
            ("checkpoints".to_owned(), Json::array(self.checkpoints.iter().map(Checkpoint::to_json).collect())),
            ("artifacts".to_owned(), Json::array(self.artifacts.iter().map(Artifact::to_json).collect())),
            ("outcome".to_owned(), self.outcome.to_json()),
        ])
    }
}

impl StatusCounts {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("PASS".to_owned(), Json::int(self.pass)),
            ("FAIL".to_owned(), Json::int(self.fail)),
            ("BLOCKED_MISSING_INPUT".to_owned(), Json::int(self.blocked)),
            ("TIMEOUT".to_owned(), Json::int(self.timeout)),
            ("NOT_RUN".to_owned(), Json::int(self.not_run)),
        ])
    }
}

impl Reconciliation {
    /// Render as JSON.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("schemaVersion".to_owned(), Json::int(1)),
            ("manifestId".to_owned(), Json::string(&self.manifest_id)),
            ("manifestSha256".to_owned(), Json::string(&self.manifest_sha256)),
            ("expected".to_owned(), Json::int(self.expected)),
            ("records".to_owned(), Json::int(self.records)),
            ("counts".to_owned(), self.counts.to_json()),
            ("missingCaseIds".to_owned(), Json::array(self.missing_case_ids.iter().map(Json::string).collect())),
            (
                "unexpectedCaseIds".to_owned(),
                Json::array(self.unexpected_case_ids.iter().map(Json::string).collect()),
            ),
            (
                "duplicateCaseIds".to_owned(),
                Json::array(self.duplicate_case_ids.iter().map(Json::string).collect()),
            ),
            (
                "invalidRecords".to_owned(),
                Json::array(
                    self.invalid_records
                        .iter()
                        .map(|record| {
                            Json::object(vec![
                                ("caseId".to_owned(), Json::string(&record.case_id)),
                                ("reasons".to_owned(), Json::array(record.reasons.iter().map(Json::string).collect())),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("complete".to_owned(), Json::boolean(self.complete)),
            ("gameplayComplete".to_owned(), Json::boolean(self.gameplay_complete)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse_json;

    fn requirement() -> Json {
        parse_json("{\"id\":\"r\",\"kind\":\"source\",\"path\":\"a.ts\",\"sha256\":null}").unwrap()
    }

    #[test]
    fn rejects_reserved_command_environment_keys() {
        for key in ["HOME", "TMPDIR", "DISPLAY", "WAYLAND_DISPLAY", "XDG_DATA_HOME", "VERIFY_CASE_ID", "9bad", "has-dash"] {
            let command = parse_json(&format!(
                "{{\"executable\":\"run\",\"args\":[],\"environment\":{{\"{key}\":\"x\"}},\"timeoutMs\":5,\"ports\":0,\"display\":\"headless\"}}"
            ))
            .unwrap();
            assert!(parse_command(&command).is_err(), "{key}");
        }
        let command = parse_json("{\"executable\":\"run\",\"args\":[\"\"],\"environment\":{\"SDL_AUDIODRIVER\":\"dummy\"},\"timeoutMs\":5,\"ports\":0,\"display\":\"headless\"}").unwrap();
        assert!(parse_command(&command).is_ok());
    }

    #[test]
    fn record_round_trip() {
        let record = parse_json(&format!(
            "{{\"schemaVersion\":1,\"caseId\":\"c\",\"configurationId\":\"g\",\"suiteId\":\"s\",\"evidenceKind\":\"tooling\",\
            \"contracts\":[],\"provenance\":{{\"runId\":\"r\",\"attemptId\":\"a\",\"previousAttempt\":null,\"reuse\":null}},\
            \"fingerprints\":{{\"manifest\":\"{}\",\"expectedCase\":\"{}\",\"source\":\"{}\",\"snapshot\":\"{}\",\"executable\":\"{}\",\
            \"runtimeExecutable\":\"{}\",\"fixtures\":\"{}\",\"environment\":\"{}\",\"clockSchedule\":null,\"networkSchedule\":null,\"seed\":0}},\
            \"inputs\":[],\"environment\":{{\"platform\":\"linux\",\"architecture\":\"x64\",\"osRelease\":\"1\",\"bunVersion\":\"1\",\"cpu\":\"c\",\"libraries\":[],\"variables\":{{}}}},\
            \"command\":null,\"resolvedCommand\":[],\"outputRoot\":\"o\",\"startedAt\":\"2024-01-01T00:00:00.000Z\",\
            \"finishedAt\":\"2024-01-01T00:00:00.000Z\",\"durationMs\":0,\"assertionCount\":0,\"assertions\":[],\"checkpoints\":[],\
            \"artifacts\":[],\"outcome\":{{\"status\":\"NOT_RUN\",\"reason\":\"r\"}}}}",
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
            "e".repeat(64),
            "f".repeat(64),
            "0".repeat(64),
            "1".repeat(64),
        ))
        .unwrap();
        let parsed = parse_record(&record).unwrap();
        assert_eq!(parse_record(&parsed.to_json()).unwrap(), parsed);
        let _ = requirement();
    }

    #[test]
    fn driver_output_rejects_unknown_fields() {
        let output = parse_json("{\"schemaVersion\":1,\"caseId\":\"c\",\"assertions\":[],\"checkpoints\":[],\"artifactPaths\":[],\"status\":\"PASS\"}").unwrap();
        assert!(parse_driver_output(&output).is_err());
    }
}
