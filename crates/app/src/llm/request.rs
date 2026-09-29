//! Synchronous LLM transport: injectable fetcher, cancellation, timeouts.
//!
//! Donor provenance: `src/llm/request.ts` (`LlmRequestInput`,
//! `TransportRequest`, `LlmFetch`, `checkRequestAbort`, `withRequestAbort`,
//! `fetchLlmResponse`, `responseBody`, `cancelResponseBody`).
//!
//! The donor is async over `fetch`/`ReadableStream`; this port is
//! synchronous over an injectable [`LlmFetch`] that returns complete
//! [`LlmResponse`] bodies, so tests never touch the network. Bodies arrive
//! whole, so there is nothing to cancel mid-read; the only streaming
//! boundary left is the per-event cancellation check in [`super::sse`].
//! `AbortSignal` becomes [`CancelToken`], and the donor's request timeout
//! becomes [`TimeoutFetch`], which abandons (never interrupts) a hung
//! fetch after its deadline.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use super::errors::LlmError;

/// Cooperative cancellation shared between a request and its caller.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// A token that is not cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel the associated request.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Whether the request was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Whether two tokens share one cancellation flag.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.cancelled, &other.cancelled)
    }
}

/// Throw when a request was cancelled (donor `checkRequestAbort`).
pub fn check_request_abort(cancel: &CancelToken) -> Result<(), LlmError> {
    if cancel.is_cancelled() {
        return Err(LlmError::settings("LLM request cancelled."));
    }
    Ok(())
}

/// An HTTP method the LLM transport sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchMethod {
    /// GET (model discovery).
    Get,
    /// POST (completions, responses, token exchange).
    Post,
}

impl FetchMethod {
    /// Wire name (`GET`/`POST`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// One injectable fetch call (donor `RequestInit` selection).
#[derive(Debug, Clone)]
pub struct FetchInit {
    /// HTTP method.
    pub method: FetchMethod,
    /// Request headers as `(lowercase-name, value)` pairs.
    pub headers: Vec<(String, String)>,
    /// Request body text, when the call sends one.
    pub body: Option<String>,
}

impl FetchInit {
    /// Look up a header case-insensitively.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A complete injectable fetch result (donor `Response` selection).
#[derive(Debug, Clone)]
pub struct LlmResponse {
    /// HTTP status code.
    pub status: u16,
    /// Raw `content-type` header value, when the service sent one.
    pub content_type: Option<String>,
    /// Complete response body bytes.
    pub body: Vec<u8>,
}

impl LlmResponse {
    /// Whether the status is 2xx.
    #[must_use]
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Body as UTF-8, rejecting invalid bytes like the donor's fatal decoder.
    pub fn text(&self) -> Result<&str, LlmError> {
        std::str::from_utf8(&self.body).map_err(|_| LlmError::settings("Could not read the LLM response stream."))
    }
}

/// Injectable HTTP fetch (donor `LlmFetch`).
///
/// Real hosts implement this over their HTTP stack (owning socket
/// timeouts); tests inject closures or stubs. No implementation in this
/// crate performs network I/O.
pub trait LlmFetch: Send + Sync {
    /// Fetch `url`, returning the complete response or a transport failure.
    fn fetch(&self, url: &str, init: &FetchInit) -> Result<LlmResponse, LlmError>;
}

impl<F> LlmFetch for F
where
    F: Fn(&str, &FetchInit) -> Result<LlmResponse, LlmError> + Send + Sync,
{
    fn fetch(&self, url: &str, init: &FetchInit) -> Result<LlmResponse, LlmError> {
        self(url, init)
    }
}

/// A fetcher that abandons hung calls after `timeout` (donor `runRequest`
/// timer). The abandoned call keeps running detached; only its result is
/// dropped.
pub struct TimeoutFetch {
    inner: Arc<dyn LlmFetch>,
    timeout: Duration,
}

impl TimeoutFetch {
    /// Wrap `inner` with a deadline.
    #[must_use]
    pub fn new(inner: Arc<dyn LlmFetch>, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}

impl LlmFetch for TimeoutFetch {
    fn fetch(&self, url: &str, init: &FetchInit) -> Result<LlmResponse, LlmError> {
        let (sender, receiver) = mpsc::channel();
        let inner = Arc::clone(&self.inner);
        let url = url.to_string();
        let init = init.clone();
        std::thread::spawn(move || {
            let _ = sender.send(inner.fetch(&url, &init));
        });
        match receiver.recv_timeout(self.timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(LlmError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(LlmError::settings(
                "Could not reach the LLM service. Check its URL and connection.",
            )),
        }
    }
}

/// Fetch one LLM response (donor `fetchLlmResponse`).
///
/// Fetcher failures become the donor's unreachable message; timeouts and
/// HTTP errors pass through untouched so callers can retry auth.
pub fn fetch_llm_response(
    url: &str,
    init: &FetchInit,
    fetcher: &impl LlmFetch,
    cancel: &CancelToken,
) -> Result<LlmResponse, LlmError> {
    check_request_abort(cancel)?;
    match fetcher.fetch(url, init) {
        Ok(response) => Ok(response),
        Err(error) => {
            if cancel.is_cancelled() {
                return Err(LlmError::settings("LLM request cancelled."));
            }
            match error {
                LlmError::Timeout | LlmError::Http { .. } => Err(error),
                LlmError::Settings(_) => Err(LlmError::settings(
                    "Could not reach the LLM service. Check its URL and connection.",
                )),
            }
        }
    }
}

/// Streaming text callback, invoked per completed delta.
pub type OnText<'a> = Box<dyn FnMut(&str) + 'a>;

/// Caller-supplied request text (donor `LlmRequestInput`).
pub struct LlmRequestInput<'a> {
    /// User prompt.
    pub prompt: &'a str,
    /// System instructions.
    pub instructions: &'a str,
    /// Streaming text callback, invoked per completed delta.
    pub on_text: Option<OnText<'a>>,
}

/// Provider-bound request (donor `TransportRequest`).
pub struct TransportRequest<'a> {
    /// User prompt.
    pub prompt: &'a str,
    /// System instructions.
    pub instructions: &'a str,
    /// Provider model id.
    pub model: &'a str,
    /// Reasoning effort override, when the model supports one.
    pub reasoning_effort: Option<&'a str>,
    /// Cooperative cancellation.
    pub cancel: CancelToken,
    /// Streaming text callback, invoked per completed delta.
    pub on_text: Option<OnText<'a>>,
}

impl<'a> TransportRequest<'a> {
    /// Emit one completed delta, then honor cancellation inside the callback
    /// (the donor's next read throws after an aborting `onText`).
    pub fn emit(&mut self, text: &str) -> Result<(), LlmError> {
        if let Some(on_text) = self.on_text.as_mut() {
            on_text(text);
        }
        check_request_abort(&self.cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> FetchInit {
        FetchInit {
            method: FetchMethod::Get,
            headers: Vec::new(),
            body: None,
        }
    }

    #[test]
    fn headers_match_case_insensitively() {
        let init = FetchInit {
            headers: vec![("Authorization".to_string(), "Bearer x".to_string())],
            ..init()
        };
        assert_eq!(init.header("authorization"), Some("Bearer x"));
        assert_eq!(init.header("missing"), None);
    }

    #[test]
    fn cancelled_fetch_never_runs() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let error = fetch_llm_response(
            "https://example.test",
            &init(),
            &|_: &str, _: &FetchInit| panic!("must not fetch after cancel"),
            &cancel,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "LLM request cancelled.");
    }

    #[test]
    fn fetcher_failures_are_unreachable_without_leaking_detail() {
        let error = fetch_llm_response(
            "https://example.test",
            &init(),
            &|_: &str, _: &FetchInit| Err(LlmError::settings("socket-secret")),
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Could not reach the LLM service. Check its URL and connection."
        );
    }

    #[test]
    fn timeouts_and_http_errors_pass_through() {
        let timeout = fetch_llm_response(
            "https://example.test",
            &init(),
            &|_: &str, _: &FetchInit| Err(LlmError::Timeout),
            &CancelToken::new(),
        )
        .unwrap_err();
        assert!(matches!(timeout, LlmError::Timeout));
        let http = fetch_llm_response(
            "https://example.test",
            &init(),
            &|_: &str, _: &FetchInit| Err(LlmError::http(401)),
            &CancelToken::new(),
        )
        .unwrap_err();
        assert_eq!(http.status(), Some(401));
    }

    #[test]
    fn timeout_fetch_abandons_hung_calls() {
        let hung: Arc<dyn LlmFetch> = Arc::new(|_: &str, _: &FetchInit| {
            std::thread::sleep(Duration::from_secs(30));
            Ok(LlmResponse {
                status: 200,
                content_type: None,
                body: Vec::new(),
            })
        });
        let fetch = TimeoutFetch::new(hung, Duration::from_millis(20));
        let error = fetch.fetch("https://example.test", &init()).unwrap_err();
        assert!(matches!(error, LlmError::Timeout));
    }

    #[test]
    fn emit_honors_cancellation_inside_the_callback() {
        let cancel = CancelToken::new();
        let probe = cancel.clone();
        let mut request = TransportRequest {
            prompt: "hi",
            instructions: "answer",
            model: "m",
            reasoning_effort: None,
            cancel,
            on_text: Some(Box::new(move |_| probe.cancel())),
        };
        assert_eq!(request.emit("first").unwrap_err().to_string(), "LLM request cancelled.");
    }
}
