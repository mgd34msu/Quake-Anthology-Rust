//! Server-sent event parsing for LLM streams.
//!
//! Donor provenance: `src/llm/sse.ts` (`SseEvent`, `consumeSse`). Same
//! line splitting (`\r\n`/`\r`/`\n`), comment lines, `data:`/`event:`
//! fields, blank-line dispatch, 1 MiB size limit, content-type gate, and
//! fixed error texts. The donor reads incrementally from a byte stream;
//! here the body arrives whole from the injectable fetcher, so the parser
//! runs over the complete text with the same dispatch rules.

use super::errors::LlmError;
use super::request::{check_request_abort, CancelToken, LlmResponse};

/// Maximum accepted stream size in bytes (donor `MAX_STREAM_BYTES`).
pub const MAX_STREAM_BYTES: usize = 1_048_576;

/// One parsed SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// Event name (`message` when the stream sets none).
    pub event: String,
    /// Joined `data:` lines.
    pub data: String,
}

/// Classify a `content-type` header the way the donor's `responseFormat` does.
fn content_format(content_type: Option<&str>) -> &'static str {
    let media = content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase());
    match media.as_deref() {
        None | Some("") => "missing content type",
        Some("text/event-stream") => "event stream",
        Some("application/json") => "JSON",
        Some("text/html") => "HTML",
        Some("text/plain") => "plain text",
        Some(_) => "other content type",
    }
}

/// `HTTP {status}, {format}` metadata for stream errors.
fn response_format(response: &LlmResponse) -> String {
    format!(
        "HTTP {}, {}",
        response.status,
        content_format(response.content_type.as_deref())
    )
}

/// Whether the response carries an event stream body.
fn is_event_stream(response: &LlmResponse) -> bool {
    response
        .content_type
        .as_deref()
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|media| media.eq_ignore_ascii_case("text/event-stream"))
}

/// Feed one line into the event state; returns a completed event, if any.
fn feed_line(line: &str, event: &mut String, data: &mut Vec<String>) -> Option<SseEvent> {
    if line.is_empty() {
        if data.is_empty() {
            return None;
        }
        let completed = SseEvent {
            event: std::mem::replace(event, "message".to_string()),
            data: std::mem::take(data).join("\n"),
        };
        return Some(completed);
    }
    if line.starts_with(':') {
        return None;
    }
    let (field, content) = match line.find(':') {
        Some(colon) => (
            &line[..colon],
            line[colon + 1..].strip_prefix(' ').unwrap_or(&line[colon + 1..]),
        ),
        None => (line, ""),
    };
    if field == "data" {
        data.push(content.to_string());
    } else if field == "event" {
        *event = content.to_string();
    }
    None
}

/// Split text into SSE lines on `\r\n`, `\r`, or `\n`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' || bytes[index] == b'\n' {
            lines.push(&text[start..index]);
            if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
                index += 1;
            }
            start = index + 1;
        }
        index += 1;
    }
    lines.push(&text[start..]);
    lines
}

/// Parse one response body into SSE events (donor `consumeSse`).
///
/// `on_event` returns `true` to stop early (completion was reached). Set
/// `require_content_type` for chat transports; the subscription transport
/// accepts the donor's MIME-agnostic payloads.
pub fn consume_sse(
    response: &LlmResponse,
    cancel: &CancelToken,
    mut on_event: impl FnMut(SseEvent) -> Result<bool, LlmError>,
    require_content_type: bool,
) -> Result<(), LlmError> {
    if response.body.len() > MAX_STREAM_BYTES {
        return Err(LlmError::settings("LLM response exceeded the size limit."));
    }
    let text = std::str::from_utf8(&response.body)
        .map_err(|_| LlmError::settings("Could not read the LLM response stream."))?;
    if require_content_type && !is_event_stream(response) || response.body.is_empty() {
        let empty = if response.body.is_empty() { ", no body" } else { "" };
        return Err(LlmError::settings(format!(
            "LLM service did not return an event stream ({}{empty}).",
            response_format(response)
        )));
    }
    check_request_abort(cancel)?;
    let mut event = "message".to_string();
    let mut data: Vec<String> = Vec::new();
    let mut received = false;
    let mut stopped = false;
    let dispatch = |completed: SseEvent,
                    received: &mut bool,
                    stopped: &mut bool,
                    on_event: &mut dyn FnMut(SseEvent) -> Result<bool, LlmError>|
     -> Result<(), LlmError> {
        check_request_abort(cancel)?;
        *received = true;
        if on_event(completed)? {
            *stopped = true;
        }
        check_request_abort(cancel)?;
        Ok(())
    };
    for line in split_lines(text) {
        if stopped {
            break;
        }
        if let Some(completed) = feed_line(line, &mut event, &mut data) {
            dispatch(completed, &mut received, &mut stopped, &mut on_event)?;
        }
    }
    if !stopped {
        if let Some(completed) = feed_line("", &mut event, &mut data) {
            dispatch(completed, &mut received, &mut stopped, &mut on_event)?;
        }
    }
    if !received {
        return Err(LlmError::settings(format!(
            "LLM service returned no event data ({}).",
            response_format(response)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(body: &str, content_type: Option<&str>) -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: content_type.map(str::to_string),
            body: body.as_bytes().to_vec(),
        }
    }

    fn collect(response: &LlmResponse, require_content_type: bool) -> Result<Vec<SseEvent>, LlmError> {
        let mut events = Vec::new();
        consume_sse(
            response,
            &CancelToken::new(),
            |event| {
                events.push(event);
                Ok(false)
            },
            require_content_type,
        )?;
        Ok(events)
    }

    #[test]
    fn parses_fields_comments_and_crlf() {
        let events = collect(
            &stream(
                ":comment\r\nevent: update\r\ndata: one\r\ndata: two\r\n\r\n",
                Some("text/event-stream; charset=utf-8"),
            ),
            true,
        )
        .unwrap();
        assert_eq!(
            events,
            vec![SseEvent {
                event: "update".to_string(),
                data: "one\ntwo".to_string(),
            }]
        );
    }

    #[test]
    fn lone_cr_and_bare_data_dispatch() {
        let events = collect(&stream("data: a\ndata: b\r\r", Some("text/event-stream")), true).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event, "message");
        assert_eq!(events[0].data, "a\nb");
    }

    #[test]
    fn stop_short_circuits_later_events() {
        let mut seen = 0;
        consume_sse(
            &stream("data: one\n\ndata: two\n\n", Some("text/event-stream")),
            &CancelToken::new(),
            |_| {
                seen += 1;
                Ok(true)
            },
            true,
        )
        .unwrap();
        assert_eq!(seen, 1);
    }

    #[test]
    fn wrong_media_type_reports_format_without_body() {
        let error = collect(
            &stream("{\"a\":1}", Some("application/json; secret=do-not-reflect")),
            true,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "LLM service did not return an event stream (HTTP 200, JSON)."
        );
    }

    #[test]
    fn empty_bodies_report_no_body() {
        let error = collect(&stream("", Some("text/event-stream")), true).unwrap_err();
        assert!(error.to_string().contains(", no body)"), "{error}");
    }

    #[test]
    fn non_event_payloads_report_safe_metadata() {
        for (media, format) in [
            ("application/json", "JSON"),
            ("text/html", "HTML"),
            ("text/plain; private=secret", "plain text"),
            ("application/secret", "other content type"),
        ] {
            let error = collect(&stream("<html>secret</html>", Some(media)), false).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("LLM service returned no event data (HTTP 200, {format}).")
            );
        }
    }

    #[test]
    fn oversized_and_invalid_bytes_fail() {
        let big = LlmResponse {
            status: 200,
            content_type: Some("text/event-stream".to_string()),
            body: vec![b'x'; MAX_STREAM_BYTES + 1],
        };
        assert_eq!(
            collect(&big, true).unwrap_err().to_string(),
            "LLM response exceeded the size limit."
        );
        let invalid = LlmResponse {
            status: 200,
            content_type: Some("text/event-stream".to_string()),
            body: vec![0xff, 0xfe],
        };
        assert_eq!(
            collect(&invalid, true).unwrap_err().to_string(),
            "Could not read the LLM response stream."
        );
    }

    #[test]
    fn cancellation_before_and_inside_dispatch_fails() {
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(collect(&stream("data: x\n\n", Some("text/event-stream")), true).is_ok());
        let mut events = Vec::new();
        let error = consume_sse(
            &stream("data: one\n\ndata: two\n\n", Some("text/event-stream")),
            &cancelled,
            |event| {
                events.push(event);
                Ok(false)
            },
            true,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "LLM request cancelled.");
        assert!(events.is_empty());
    }
}
