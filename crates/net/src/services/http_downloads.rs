//! HTTP download queue ported from `src/network/services/http-downloads.ts`.
//!
//! Q2PRO/Q2 rerelease HTTP queue behavior over the shared staged
//! filesystem. The donor fetches concurrently with `fetch`; this port runs
//! the same redirect, range-probe, ETag, and span protocol synchronously
//! through a blocking [`HttpClient`], so range streams transfer sequentially
//! with identical validation.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::downloads::{download_path, DownloadSink, DownloadSpan, SinkExpectation};
use crate::common::session::ContentDigest;

/// Error for HTTP download failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HttpDownloadError {
    /// Download URL requires HTTP or HTTPS.
    #[error("Download URL requires HTTP or HTTPS")]
    BadScheme,
    /// Invalid download URL.
    #[error("Invalid download URL")]
    BadUrl,
    /// HTTP download redirect limit or missing location.
    #[error("HTTP download redirect limit or missing location")]
    BadRedirect,
    /// HTTP download redirect left advertised origin.
    #[error("HTTP download redirect left advertised origin")]
    RedirectLeftOrigin,
    /// HTTP download status.
    #[error("HTTP download status {0}")]
    BadStatus(u16),
    /// Invalid HTTP byte range response.
    #[error("Invalid HTTP byte range response")]
    BadRange,
    /// HTTP ranged representation changed.
    #[error("HTTP ranged representation changed")]
    RangeChanged,
    /// HTTP range size differs from expected bounds.
    #[error("HTTP range size differs from expected bounds")]
    RangeSize,
    /// HTTP byte range overflow.
    #[error("HTTP byte range overflow")]
    RangeOverflow,
    /// Incomplete HTTP byte range.
    #[error("Incomplete HTTP byte range")]
    IncompleteRange,
    /// Incomplete HTTP response body.
    #[error("Incomplete HTTP response body")]
    IncompleteBody,
    /// HTTP whole-file retry differs from probed size.
    #[error("HTTP whole-file retry differs from probed size")]
    RetrySize,
    /// HTTP download concurrency must be between 1 and 4.
    #[error("HTTP download concurrency must be between 1 and 4")]
    BadConcurrency,
    /// Invalid HTTP range stream count.
    #[error("Invalid HTTP range stream count")]
    BadStreams,
    /// Conflicting HTTP download identity for one destination.
    #[error("Conflicting HTTP download identity for one destination")]
    ConflictingIdentity,
    /// Only a settled HTTP fallback or cancellation can be retried.
    #[error("Only a settled HTTP fallback or cancellation can be retried")]
    NotRetryable,
    /// HTTP download epoch retired.
    #[error("HTTP download epoch retired")]
    EpochRetired,
    /// HTTP download and staged cleanup failed.
    #[error("HTTP download and staged cleanup failed: {0}")]
    CleanupFailed(String),
    /// HTTP ranged staging cleanup failed.
    #[error("HTTP ranged staging cleanup failed")]
    RangedCleanupFailed,
    /// HTTP metadata exceeds its size limit.
    #[error("HTTP metadata exceeds its size limit")]
    MetadataTooLarge,
    /// Invalid HTTP metadata size limit.
    #[error("Invalid HTTP metadata size limit")]
    BadMetadataLimit,
    /// Staged validation failed.
    #[error("HTTP staged validation failed: {0}")]
    ValidationFailed(String),
    /// HTTP server rejected byte ranges.
    #[error("HTTP server rejected byte ranges")]
    UnsupportedRange,
    /// Download failure.
    #[error("{0}")]
    Download(String),
    /// Epoch assertion failed.
    #[error("HTTP download no longer current")]
    StaleEpoch,
}

/// Staged download validator.
pub type HttpDownloadValidator = Box<dyn Fn(&Path) -> Result<(), HttpDownloadError>>;

/// Package refresh callback.
pub type HttpPackageRefresh = Box<dyn FnMut(&str) -> Result<(), HttpDownloadError>>;

/// Progress report callback.
pub type HttpDownloadProgress = Box<dyn FnMut(&str, u64, Option<u64>)>;

/// Download request (`HttpDownloadRequest`).
pub struct HttpDownloadRequest {
    /// Contained destination path.
    pub path: String,
    /// Source URL.
    pub url: String,
    /// Asset or package kind.
    pub kind: HttpDownloadKind,
    /// Size/digest expectation.
    pub expected: SinkExpectation,
    /// Staged validator with an identity tag for conflict checks.
    pub validate: Option<(u64, HttpDownloadValidator)>,
}

/// Request kind: asset or package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpDownloadKind {
    /// Asset file.
    Asset,
    /// Package requiring a refresh after publish.
    Package,
}

/// Download result (`HttpDownloadResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpDownloadResult {
    /// Downloaded with digest.
    Downloaded {
        /// Content digest.
        digest: ContentDigest,
    },
    /// Already resolved.
    Resolved,
    /// Cancelled.
    Cancelled,
    /// Fallback to another source.
    Fallback {
        /// Reason.
        reason: String,
    },
    /// Failed.
    Failed {
        /// Reason.
        reason: String,
    },
}

impl HttpDownloadResult {
    /// Result kind name.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Downloaded { .. } => "downloaded",
            Self::Resolved => "resolved",
            Self::Cancelled => "cancelled",
            Self::Fallback { .. } => "fallback",
            Self::Failed { .. } => "failed",
        }
    }
}

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// GET.
    Get,
    /// HEAD.
    Head,
}

/// Blocking HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Response body.
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Case-insensitive header lookup.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Blocking HTTP client (`fetch` replacement).
pub trait HttpClient {
    /// Perform a request.
    fn request(
        &mut self,
        method: HttpMethod,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HttpResponse, HttpDownloadError>;
}

fn same_expectation(left: &SinkExpectation, right: &SinkExpectation) -> bool {
    match (left, right) {
        (SinkExpectation::Protocol(a), SinkExpectation::Protocol(b)) => a.maximum_bytes == b.maximum_bytes,
        (SinkExpectation::Content(a), SinkExpectation::Content(b)) => {
            a.byte_length == b.byte_length && a.digest == b.digest
        }
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUrl {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

impl ParsedUrl {
    fn origin(&self) -> String {
        format!("{}://{}:{}", self.scheme, self.host, self.port)
    }

    fn href(&self) -> String {
        format!("{}://{}:{}{}", self.scheme, self.host, self.port, self.path)
    }
}

fn parse_url(text: &str) -> Result<ParsedUrl, HttpDownloadError> {
    let (scheme, rest) = text.split_once("://").ok_or(HttpDownloadError::BadUrl)?;
    if scheme != "http" && scheme != "https" {
        return Err(HttpDownloadError::BadUrl);
    }
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], rest[index..].to_owned()),
        None => (rest, "/".to_owned()),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(HttpDownloadError::BadUrl);
    }
    let (host, port) = if let Some(bracket) = authority.strip_prefix('[') {
        let end = bracket.find(']').ok_or(HttpDownloadError::BadUrl)?;
        let host = bracket[..end].to_ascii_lowercase();
        let suffix = &bracket[end + 1..];
        let port = if suffix.is_empty() {
            default_port(scheme)
        } else {
            suffix
                .strip_prefix(':')
                .ok_or(HttpDownloadError::BadUrl)?
                .parse::<u16>()
                .map_err(|_| HttpDownloadError::BadUrl)?
        };
        (format!("[{host}]"), port)
    } else if let Some((host, port_text)) = authority.rsplit_once(':') {
        if host.is_empty() || port_text.is_empty() || !port_text.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(HttpDownloadError::BadUrl);
        }
        (
            host.to_ascii_lowercase(),
            port_text.parse::<u16>().map_err(|_| HttpDownloadError::BadUrl)?,
        )
    } else {
        (authority.to_ascii_lowercase(), default_port(scheme))
    };
    if host.is_empty() {
        return Err(HttpDownloadError::BadUrl);
    }
    Ok(ParsedUrl {
        scheme: scheme.to_owned(),
        host,
        port,
        path,
    })
}

fn default_port(scheme: &str) -> u16 {
    if scheme == "https" {
        443
    } else {
        80
    }
}

fn resolve_location(current: &ParsedUrl, location: &str) -> Result<ParsedUrl, HttpDownloadError> {
    if location.starts_with("http://") || location.starts_with("https://") {
        return parse_url(location);
    }
    if location.starts_with('/') {
        return Ok(ParsedUrl {
            path: location.to_owned(),
            ..current.clone()
        });
    }
    let mut base = current.path.clone();
    if let Some(index) = base.rfind('/') {
        base.truncate(index + 1);
    } else {
        base = "/".to_owned();
    }
    Ok(ParsedUrl {
        path: format!("{base}{location}"),
        ..current.clone()
    })
}

/// Redirects stay within the advertised origin and never acquire credentials
/// (`downloadResponse`).
fn download_response(
    client: &mut dyn HttpClient,
    url: &ParsedUrl,
    method: HttpMethod,
    headers: &[(String, String)],
) -> Result<(ParsedUrl, HttpResponse), HttpDownloadError> {
    let origin = url.origin();
    let mut current = url.clone();
    for redirects in 0.. {
        if current.scheme != "http" && current.scheme != "https" {
            return Err(HttpDownloadError::BadUrl);
        }
        let response = client.request(method, &current.href(), headers)?;
        if ![301, 302, 303, 307, 308].contains(&response.status) {
            return Ok((current, response));
        }
        let location = response.header("location").map(str::to_owned);
        let Some(location) = location else {
            return Err(HttpDownloadError::BadRedirect);
        };
        if redirects >= 4 {
            return Err(HttpDownloadError::BadRedirect);
        }
        let next = resolve_location(&current, &location)?;
        if next.origin() != origin {
            return Err(HttpDownloadError::RedirectLeftOrigin);
        }
        current = next;
    }
    unreachable!("redirect loop always returns")
}

fn content_length(response: &HttpResponse) -> Option<u64> {
    let value = response.header("content-length")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok()
}

fn identity_encoding(response: &HttpResponse) -> bool {
    response
        .header("content-encoding")
        .is_none_or(|encoding| encoding.eq_ignore_ascii_case("identity"))
}

fn open_response(
    client: &mut dyn HttpClient,
    url: &ParsedUrl,
    identity: Option<&str>,
) -> Result<HttpResponse, HttpDownloadError> {
    let mut headers = Vec::new();
    if let Some(identity) = identity {
        headers.push(("Accept-Encoding".to_owned(), "identity".to_owned()));
        headers.push(("If-Match".to_owned(), identity.to_owned()));
    }
    let (_, response) = download_response(client, url, HttpMethod::Get, &headers)?;
    if response.status != 200 {
        return Err(HttpDownloadError::BadStatus(response.status));
    }
    if let Some(identity) = identity {
        if response.header("etag") != Some(identity) || !identity_encoding(&response) {
            return Err(HttpDownloadError::BadStatus(response.status));
        }
    }
    Ok(response)
}

struct RangeProbe {
    total: u64,
    etag: String,
}

fn valid_etag(etag: &str) -> bool {
    let bytes = etag.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'"' || bytes[bytes.len() - 1] != b'"' {
        return false;
    }
    bytes[1..bytes.len() - 1]
        .iter()
        .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80)
}

const RANGE_THRESHOLD: u64 = 1024 * 1024;

fn probe_ranges(
    client: &mut dyn HttpClient,
    url: &ParsedUrl,
    expected: &SinkExpectation,
) -> Result<Option<RangeProbe>, HttpDownloadError> {
    let limit = match expected {
        SinkExpectation::Protocol(expected) => expected.maximum_bytes,
        SinkExpectation::Content(expected) => expected.byte_length,
    };
    if limit <= RANGE_THRESHOLD {
        return Ok(None);
    }
    let response = match download_response(
        client,
        url,
        HttpMethod::Head,
        &[("Accept-Encoding".to_owned(), "identity".to_owned())],
    ) {
        Ok((_, response)) => response,
        Err(_) => return Ok(None),
    };
    let total = content_length(&response);
    let etag = response.header("etag").map(str::to_owned);
    let (Some(total), Some(etag)) = (total, etag) else {
        return Ok(None);
    };
    if response.status != 200
        || !identity_encoding(&response)
        || total <= RANGE_THRESHOLD
        || !matches!(response.header("accept-ranges"), Some(value) if value.eq_ignore_ascii_case("bytes"))
        || !valid_etag(&etag)
    {
        return Ok(None);
    }
    if total > limit {
        return Err(HttpDownloadError::RangeSize);
    }
    if let SinkExpectation::Content(expected) = expected {
        if total != expected.byte_length {
            return Err(HttpDownloadError::RangeSize);
        }
    }
    Ok(Some(RangeProbe { total, etag }))
}

fn byte_spans(total: u64, streams: usize) -> Vec<DownloadSpan> {
    let width = total.div_ceil(streams as u64);
    let mut spans = Vec::new();
    let mut start = 0;
    while start < total {
        let end = (start + width).min(total) - 1;
        spans.push(DownloadSpan { start, end });
        start = end + 1;
    }
    spans
}

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let range = value.strip_prefix("bytes ")?;
    let (span, total) = range.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

fn receive_range(
    client: &mut dyn HttpClient,
    url: &ParsedUrl,
    probe: &RangeProbe,
    span: &DownloadSpan,
    sink: &mut DownloadSink,
) -> Result<(), HttpDownloadError> {
    let mut received = 0;
    let span_length = span.end - span.start + 1;
    for attempt in 0u32.. {
        let start = span.start + received;
        let headers = vec![
            ("Range".to_owned(), format!("bytes={start}-{}", span.end)),
            ("If-Range".to_owned(), probe.etag.clone()),
            ("Accept-Encoding".to_owned(), "identity".to_owned()),
        ];
        let (_, response) = download_response(client, url, HttpMethod::Get, &headers)?;
        if let Some(etag) = response.header("etag") {
            if etag != probe.etag {
                return Err(HttpDownloadError::RangeChanged);
            }
        }
        if response.status == 200 || response.status == 416 {
            return Err(HttpDownloadError::UnsupportedRange);
        }
        let attempt_result = (|| -> Result<(), HttpDownloadError> {
            if response.status != 206
                || response.header("etag") != Some(probe.etag.as_str())
                || !identity_encoding(&response)
            {
                return Err(HttpDownloadError::BadRange);
            }
            let content_range = response.header("content-range").unwrap_or("");
            let Some((range_start, range_end, range_total)) = parse_content_range(content_range) else {
                return Err(HttpDownloadError::BadRange);
            };
            if range_start != start || range_end != span.end || range_total != probe.total {
                return Err(HttpDownloadError::BadRange);
            }
            if let Some(length) = content_length(&response) {
                if length != span.end - start + 1 {
                    return Err(HttpDownloadError::BadRange);
                }
            }
            if response.body.len() as u64 > span_length - received {
                return Err(HttpDownloadError::RangeOverflow);
            }
            sink.append_range(span.start + received, &response.body)
                .map_err(|error| HttpDownloadError::Download(error.to_string()))?;
            received += response.body.len() as u64;
            if received != span_length {
                return Err(HttpDownloadError::IncompleteRange);
            }
            Ok(())
        })();
        match attempt_result {
            Ok(()) => return Ok(()),
            Err(error) => {
                if attempt >= 2 || received == span_length {
                    return Err(error);
                }
            }
        }
    }
    Ok(())
}

/// Optional dependency metadata fetch (`fetchHttpDownloadMetadata`).
pub fn fetch_http_download_metadata(client: &mut dyn HttpClient, url: &str, maximum_bytes: u64) -> Option<Vec<u8>> {
    let parsed = parse_url(url).ok()?;
    if parsed.scheme != "http" && parsed.scheme != "https" {
        return None;
    }
    let response = open_response(client, &parsed, None).ok()?;
    if response.body.len() as u64 > maximum_bytes {
        return None;
    }
    Some(response.body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryState {
    Pending,
    Running,
    Done,
}

struct Entry {
    path: String,
    url: String,
    kind: HttpDownloadKind,
    expected: SinkExpectation,
    validator_tag: Option<u64>,
    validate: Option<HttpDownloadValidator>,
    state: EntryState,
    received: u64,
    total: Option<u64>,
    result: Option<HttpDownloadResult>,
    cancelled: bool,
}

/// Download progress (`DownloadProgress`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadProgress {
    /// Destination path.
    pub path: String,
    /// Phase.
    pub phase: &'static str,
    /// Received bytes.
    pub received: u64,
    /// Total bytes, if known.
    pub total: Option<u64>,
    /// Result kind, once settled.
    pub result: Option<String>,
}

/// Queue callbacks (`HttpDownloadQueueOptions` without I/O ownership).
pub struct HttpQueueCallbacks {
    /// Assert the connection epoch is current.
    pub assert_current: Box<dyn Fn() -> Result<(), HttpDownloadError>>,
    /// True when the path is already resolved.
    pub resolved: Box<dyn FnMut(&str) -> bool>,
    /// Refresh a published package.
    pub refresh_package: HttpPackageRefresh,
    /// Progress reports.
    pub progress: HttpDownloadProgress,
}

/// HTTP download queue (`HttpDownloadQueue`).
pub struct HttpDownloadQueue {
    root: PathBuf,
    client: Box<dyn HttpClient>,
    callbacks: HttpQueueCallbacks,
    concurrency: usize,
    range_streams: usize,
    entries: HashMap<String, Entry>,
    closed: bool,
    generation: u64,
}

impl HttpDownloadQueue {
    /// Create a queue.
    pub fn new(
        root: PathBuf,
        client: Box<dyn HttpClient>,
        callbacks: HttpQueueCallbacks,
        concurrency: usize,
        range_streams: usize,
    ) -> Result<Self, HttpDownloadError> {
        if !(1..=4).contains(&concurrency) {
            return Err(HttpDownloadError::BadConcurrency);
        }
        Ok(Self {
            root,
            client,
            callbacks,
            concurrency,
            range_streams: range_streams.clamp(1, 8),
            entries: HashMap::new(),
            closed: false,
            generation: 0,
        })
    }

    /// Enqueue a request (`enqueue`).
    pub fn enqueue(&mut self, request: HttpDownloadRequest) -> Result<(), HttpDownloadError> {
        if self.closed {
            self.entries.insert(
                request.path.clone(),
                Entry {
                    path: request.path.clone(),
                    url: request.url.clone(),
                    kind: request.kind,
                    expected: request.expected,
                    validator_tag: None,
                    validate: None,
                    state: EntryState::Done,
                    received: 0,
                    total: None,
                    result: Some(HttpDownloadResult::Cancelled),
                    cancelled: true,
                },
            );
            return Ok(());
        }
        (self.callbacks.assert_current)()?;
        download_path(&request.path).map_err(|error| HttpDownloadError::Download(error.to_string()))?;
        let parsed = parse_url(&request.url)?;
        if parsed.scheme != "http" && parsed.scheme != "https" {
            return Err(HttpDownloadError::BadScheme);
        }
        if let Some(existing) = self.entries.get(&request.path) {
            let (validator_tag, _) = request.validate.as_ref().map(|(tag, _)| (*tag, ())).unzip();
            if existing.url != request.url
                || existing.kind != request.kind
                || !same_expectation(&existing.expected, &request.expected)
                || existing.validator_tag != validator_tag
            {
                return Err(HttpDownloadError::ConflictingIdentity);
            }
            return Ok(());
        }
        let (validator_tag, validate) = match request.validate {
            Some((tag, validate)) => (Some(tag), Some(validate)),
            None => (None, None),
        };
        self.entries.insert(
            request.path.clone(),
            Entry {
                path: request.path.clone(),
                url: request.url,
                kind: request.kind,
                expected: request.expected,
                validator_tag,
                validate,
                state: EntryState::Pending,
                received: 0,
                total: None,
                result: None,
                cancelled: false,
            },
        );
        Ok(())
    }

    /// Entry progress rows (`progress`).
    #[must_use]
    pub fn progress(&self) -> Vec<DownloadProgress> {
        self.entries
            .values()
            .map(|entry| DownloadProgress {
                path: entry.path.clone(),
                phase: match entry.state {
                    EntryState::Pending => "pending",
                    EntryState::Running => "running",
                    EntryState::Done => "done",
                },
                received: entry.received,
                total: entry.total,
                result: entry.result.as_ref().map(HttpDownloadResult::kind).map(str::to_owned),
            })
            .collect()
    }

    /// Result for a path, once settled.
    #[must_use]
    pub fn result(&self, path: &str) -> Option<&HttpDownloadResult> {
        self.entries.get(path).and_then(|entry| entry.result.as_ref())
    }

    /// Stored result kind for a path.
    #[must_use]
    pub fn result_kind(&self, path: &str) -> Option<&str> {
        self.result(path).map(HttpDownloadResult::kind)
    }

    fn retryable(result: Option<&HttpDownloadResult>) -> bool {
        matches!(
            result,
            Some(HttpDownloadResult::Fallback { .. } | HttpDownloadResult::Cancelled)
        )
    }

    /// Retry a settled fallback or cancellation (`retry`).
    ///
    /// Validators do not survive a retry: entries created with a staged
    /// validator cannot be rebuilt identically, so they are not retryable.
    pub fn retry(&mut self, path: &str) -> Result<(), HttpDownloadError> {
        let Some(entry) = self.entries.get(path) else {
            return Err(HttpDownloadError::NotRetryable);
        };
        if entry.state != EntryState::Done || !Self::retryable(entry.result.as_ref()) || entry.validator_tag.is_some() {
            return Err(HttpDownloadError::NotRetryable);
        }
        let rebuilt = HttpDownloadRequest {
            path: entry.path.clone(),
            url: entry.url.clone(),
            kind: entry.kind,
            expected: entry.expected.clone(),
            validate: None,
        };
        self.entries.remove(path);
        self.enqueue(rebuilt)
    }

    /// Cancel one file (`cancelFile`).
    pub fn cancel_file(&mut self, path: &str) {
        if let Some(entry) = self.entries.get_mut(path) {
            if entry.state == EntryState::Pending {
                entry.state = EntryState::Done;
                entry.result = Some(HttpDownloadResult::Cancelled);
            } else if entry.state == EntryState::Running {
                entry.cancelled = true;
            }
        }
    }

    fn report(&mut self, path: &str, received: u64, total: Option<u64>) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.received = received;
            entry.total = total;
        }
        (self.callbacks.progress)(path, received, total);
    }

    /// Run queued work synchronously (`pump` + `transfer`).
    pub fn process(&mut self) {
        if self.closed {
            return;
        }
        if (self.callbacks.assert_current)().is_err() {
            let paths: Vec<String> = self.entries.keys().cloned().collect();
            for path in paths {
                if let Some(entry) = self.entries.get_mut(&path) {
                    if entry.state == EntryState::Pending {
                        entry.state = EntryState::Done;
                        entry.result = Some(HttpDownloadResult::Failed {
                            reason: "epoch retired".to_owned(),
                        });
                    }
                }
            }
            return;
        }
        let pending: Vec<String> = self
            .entries
            .values()
            .filter(|entry| entry.state == EntryState::Pending)
            .map(|entry| entry.path.clone())
            .collect();
        for path in &pending {
            if (self.callbacks.resolved)(path) {
                if let Some(entry) = self.entries.get_mut(path) {
                    entry.state = EntryState::Done;
                    entry.result = Some(HttpDownloadResult::Resolved);
                }
            }
        }
        if (self.callbacks.assert_current)().is_err() {
            return;
        }
        let pending: Vec<String> = self
            .entries
            .values()
            .filter(|entry| entry.state == EntryState::Pending)
            .map(|entry| entry.path.clone())
            .collect();
        if let Some(pack) = pending.iter().find(|path| {
            self.entries
                .get(*path)
                .is_some_and(|entry| entry.kind == HttpDownloadKind::Package)
        }) {
            let pack = pack.clone();
            self.run_entry(&pack);
            return;
        }
        for (started, path) in pending.into_iter().enumerate() {
            if started >= self.concurrency {
                break;
            }
            let failed = self.run_entry(&path);
            if failed {
                self.cancel();
                return;
            }
        }
    }

    fn run_entry(&mut self, path: &str) -> bool {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.state = EntryState::Running;
        }
        let result = self.transfer(path);
        let failed = matches!(result, HttpDownloadResult::Failed { .. });
        if let Some(entry) = self.entries.get_mut(path) {
            entry.state = EntryState::Done;
            entry.result = Some(result);
        }
        failed
    }

    fn transfer(&mut self, path: &str) -> HttpDownloadResult {
        let generation = self.generation;
        let entry = match self.entries.get(path) {
            Some(entry) => entry,
            None => {
                return HttpDownloadResult::Failed {
                    reason: "Unknown HTTP download".to_owned(),
                }
            }
        };
        let url_text = entry.url.clone();
        let expected = entry.expected.clone();
        let kind = entry.kind;
        let has_validator = entry.validate.is_some();
        let check = |queue: &mut Self| -> Result<(), HttpDownloadError> {
            if queue.closed || generation != queue.generation {
                return Err(HttpDownloadError::EpochRetired);
            }
            (queue.callbacks.assert_current)()?;
            if queue.entries.get(path).is_some_and(|entry| entry.cancelled) {
                return Err(HttpDownloadError::EpochRetired);
            }
            Ok(())
        };
        if check(self).is_err() {
            return HttpDownloadResult::Cancelled;
        }
        let parsed = match parse_url(&url_text) {
            Ok(parsed) => parsed,
            Err(error) => {
                return HttpDownloadResult::Failed {
                    reason: error.to_string(),
                }
            }
        };
        let probe = if self.range_streams > 1 {
            match probe_ranges(&mut *self.client, &parsed, &expected) {
                Ok(probe) => probe,
                Err(error) => {
                    return HttpDownloadResult::Fallback {
                        reason: error.to_string(),
                    }
                }
            }
        } else {
            None
        };
        if check(self).is_err() {
            return HttpDownloadResult::Cancelled;
        }
        let mut sink: Option<DownloadSink> = None;
        let mut retry_identity: Option<String> = None;
        if let Some(probe) = &probe {
            let spans = byte_spans(probe.total, self.range_streams);
            let mut ranged = match DownloadSink::create_ranged(&self.root, path, expected.clone(), probe.total, &spans)
            {
                Ok(sink) => sink,
                Err(error) => {
                    return HttpDownloadResult::Failed {
                        reason: error.to_string(),
                    }
                }
            };
            self.report(path, 0, Some(probe.total));
            let mut failed: Option<HttpDownloadError> = None;
            for span in &spans {
                let result = receive_range(&mut *self.client, &parsed, probe, span, &mut ranged);
                let received = ranged.byte_length();
                self.report(path, received, Some(probe.total));
                if check(self).is_err() {
                    let mut ranged = ranged;
                    ranged.close();
                    return HttpDownloadResult::Cancelled;
                }
                if let Err(error) = result {
                    failed = Some(error);
                    break;
                }
            }
            if let Some(error) = failed {
                if matches!(error, HttpDownloadError::UnsupportedRange) {
                    let mut ranged = ranged;
                    ranged.close();
                    if check(self).is_err() {
                        return HttpDownloadResult::Cancelled;
                    }
                    retry_identity = Some(probe.etag.clone());
                } else {
                    let mut ranged = ranged;
                    ranged.close();
                    if self.closed || generation != self.generation {
                        return HttpDownloadResult::Cancelled;
                    }
                    return HttpDownloadResult::Fallback {
                        reason: error.to_string(),
                    };
                }
            } else {
                sink = Some(ranged);
            }
        }
        if sink.is_none() {
            let response = match open_response(&mut *self.client, &parsed, retry_identity.as_deref()) {
                Ok(response) => response,
                Err(error) => {
                    if self.closed || generation != self.generation {
                        return HttpDownloadResult::Cancelled;
                    }
                    return HttpDownloadResult::Fallback {
                        reason: error.to_string(),
                    };
                }
            };
            if check(self).is_err() {
                return HttpDownloadResult::Cancelled;
            }
            let total = content_length(&response);
            let mut whole = match DownloadSink::create(&self.root, path, expected.clone()) {
                Ok(sink) => sink,
                Err(error) => {
                    return HttpDownloadResult::Failed {
                        reason: error.to_string(),
                    }
                }
            };
            self.report(path, 0, total);
            // Stream in 64 KiB slices to mirror chunked appends.
            let mut offset = 0;
            let mut failed: Option<HttpDownloadError> = None;
            while offset < response.body.len() {
                if check(self).is_err() {
                    let mut whole = whole;
                    whole.close();
                    return HttpDownloadResult::Cancelled;
                }
                let end = (offset + 65536).min(response.body.len());
                if let Err(error) = whole.append(&response.body[offset..end]) {
                    failed = Some(HttpDownloadError::Download(error.to_string()));
                    break;
                }
                offset = end;
                let received = whole.byte_length();
                self.report(path, received, total);
            }
            if let Some(error) = failed {
                let mut whole = whole;
                whole.close();
                return HttpDownloadResult::Fallback {
                    reason: error.to_string(),
                };
            }
            if total.is_some_and(|total| whole.byte_length() != total) {
                let mut whole = whole;
                whole.close();
                return HttpDownloadResult::Fallback {
                    reason: HttpDownloadError::IncompleteBody.to_string(),
                };
            }
            if retry_identity.is_some() && probe.as_ref().is_some_and(|probe| whole.byte_length() != probe.total) {
                let mut whole = whole;
                whole.close();
                return HttpDownloadResult::Fallback {
                    reason: HttpDownloadError::RetrySize.to_string(),
                };
            }
            sink = Some(whole);
        }
        if check(self).is_err() {
            if let Some(mut sink) = sink {
                sink.close();
            }
            return HttpDownloadResult::Cancelled;
        }
        let mut sink = match sink {
            Some(sink) => sink,
            None => {
                return HttpDownloadResult::Failed {
                    reason: "HTTP download produced no sink".to_owned(),
                }
            }
        };
        if has_validator {
            let mut outcome = Ok(());
            {
                let validator = self.entries.get(path).and_then(|entry| entry.validate.as_ref());
                let inspected = sink.inspect_staged(|staged| {
                    if let Some(validate) = validator {
                        outcome = validate(staged);
                    }
                });
                if let Err(error) = inspected {
                    return HttpDownloadResult::Failed {
                        reason: error.to_string(),
                    };
                }
            }
            if let Err(error) = outcome {
                sink.close();
                return HttpDownloadResult::Failed {
                    reason: error.to_string(),
                };
            }
            if check(self).is_err() {
                sink.close();
                return HttpDownloadResult::Cancelled;
            }
        }
        match sink.finish() {
            Ok(digest) => {
                if kind == HttpDownloadKind::Package {
                    if let Err(error) = (self.callbacks.refresh_package)(path) {
                        return HttpDownloadResult::Failed {
                            reason: error.to_string(),
                        };
                    }
                    if check(self).is_err() {
                        return HttpDownloadResult::Cancelled;
                    }
                }
                HttpDownloadResult::Downloaded { digest }
            }
            Err(error) => HttpDownloadResult::Failed {
                reason: error.to_string(),
            },
        }
    }

    /// Cancel every entry and retire the epoch (`cancel`).
    pub fn cancel(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.generation += 1;
        for entry in self.entries.values_mut() {
            if entry.state == EntryState::Pending {
                entry.state = EntryState::Done;
                entry.result = Some(HttpDownloadResult::Cancelled);
            } else if entry.state == EntryState::Running {
                entry.cancelled = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeClient {
        body: Vec<u8>,
    }

    impl HttpClient for FakeClient {
        fn request(
            &mut self,
            method: HttpMethod,
            _url: &str,
            _headers: &[(String, String)],
        ) -> Result<HttpResponse, HttpDownloadError> {
            if method == HttpMethod::Head {
                return Ok(HttpResponse {
                    status: 200,
                    headers: vec![
                        ("Content-Length".to_owned(), self.body.len().to_string()),
                        ("Accept-Ranges".to_owned(), "bytes".to_owned()),
                        ("ETag".to_owned(), "\"v1\"".to_owned()),
                    ],
                    body: Vec::new(),
                });
            }
            Ok(HttpResponse {
                status: 200,
                headers: vec![("Content-Length".to_owned(), self.body.len().to_string())],
                body: self.body.clone(),
            })
        }
    }

    #[test]
    fn queue_downloads_whole_files() {
        let root = std::env::temp_dir().join(format!("qa-net-http-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let client = Box::new(FakeClient {
            body: b"hello".to_vec(),
        });
        let callbacks = HttpQueueCallbacks {
            assert_current: Box::new(|| Ok(())),
            resolved: Box::new(|_| false),
            refresh_package: Box::new(|_| Ok(())),
            progress: Box::new(|_, _, _| {}),
        };
        let mut queue = HttpDownloadQueue::new(root.clone(), client, callbacks, 2, 4).unwrap();
        queue
            .enqueue(HttpDownloadRequest {
                path: "a.txt".to_owned(),
                url: "http://example.com/a.txt".to_owned(),
                kind: HttpDownloadKind::Asset,
                expected: SinkExpectation::Protocol(super::super::downloads::ProtocolDownloadExpectation {
                    maximum_bytes: 1024,
                }),
                validate: None,
            })
            .unwrap();
        queue.process();
        assert_eq!(queue.result_kind("a.txt"), Some("downloaded"));
        assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"hello");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn byte_spans_partition() {
        let spans = byte_spans(10, 3);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0], DownloadSpan { start: 0, end: 3 });
        assert_eq!(spans[2].end, 9);
    }
}
