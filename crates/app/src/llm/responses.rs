//! OpenAI Responses stream reading for API and subscription transports.
//!
//! Donor provenance: `src/llm/responses.ts` (`readResponses`). Same event
//! handling: refusal/tool failures, `response.output_text.delta` streaming,
//! authoritative `response.completed` text with the donor's consistency
//! check, and label-prefixed fixed errors that never reflect provider text.

use crate::settings::json::{parse_json, Json};

use super::errors::LlmError;
use super::request::{LlmResponse, TransportRequest};
use super::sse::consume_sse;

fn object(value: &Json) -> Result<&Vec<(String, Json)>, LlmError> {
    match value {
        Json::Object(members) => Ok(members),
        _ => Err(LlmError::settings("Invalid LLM response.")),
    }
}

fn member<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

/// Validate one completed output item, returning its text (donor `outputItem`).
fn output_item(value: &Json, label: &str) -> Result<String, LlmError> {
    let mut text = String::new();
    let item = object(value)?;
    let kind = member(item, "type");
    if kind != Some(&Json::String("message".to_string())) && kind != Some(&Json::String("reasoning".to_string())) {
        return Err(LlmError::settings(format!(
            "{label} returned an unsupported output item."
        )));
    }
    if kind == Some(&Json::String("message".to_string())) {
        if let Some(content) = member(item, "content") {
            let Json::Array(parts) = content else {
                return Err(LlmError::settings("Invalid LLM response."));
            };
            for part in parts {
                let entry = object(part)?;
                let entry_type = member(entry, "type");
                if entry_type == Some(&Json::String("refusal".to_string())) {
                    return Err(LlmError::settings(format!("{label} declined the request.")));
                }
                if entry_type != Some(&Json::String("output_text".to_string())) {
                    return Err(LlmError::settings(format!("{label} returned unsupported content.")));
                }
                match member(entry, "text") {
                    Some(Json::String(chunk)) => text.push_str(chunk),
                    _ => return Err(LlmError::settings("Invalid LLM response.")),
                }
            }
        }
    }
    Ok(text)
}

/// Read one Responses event stream to completion (donor `readResponses`).
///
/// `label` prefixes failures (`OpenAI API`, `Subscription`); API
/// transports require the SSE media type while the subscription transport
/// accepts the donor's MIME-agnostic payloads.
pub fn read_responses(
    input: &mut TransportRequest,
    response: &LlmResponse,
    label: &str,
    require_content_type: bool,
) -> Result<String, LlmError> {
    let mut text = String::new();
    let mut completed = false;
    let cancel = input.cancel.clone();
    consume_sse(
        response,
        &cancel,
        |event| {
            if event.data == "[DONE]" {
                return Ok(false);
            }
            let value = parse_json(&event.data).map_err(|_| LlmError::settings("Invalid LLM response."))?;
            let item = object(&value)?;
            let event_type = member(item, "type").and_then(|kind| match kind {
                Json::String(kind) => Some(kind.clone()),
                _ => None,
            });
            let kind = event_type.as_deref().unwrap_or(event.event.as_str());
            if kind == "response.failed" || kind == "response.incomplete" || kind == "error" {
                return Err(LlmError::settings(format!("{label} response failed. Try again.")));
            }
            if kind.starts_with("response.refusal.") {
                return Err(LlmError::settings(format!("{label} declined the request.")));
            }
            if kind.starts_with("response.function_call_arguments.") {
                return Err(LlmError::settings(format!(
                    "{label} returned an unsupported tool call."
                )));
            }
            if kind == "response.output_item.added" || kind == "response.output_item.done" {
                output_item(member(item, "item").unwrap_or(&Json::Null), label)?;
            }
            if (kind == "response.content_part.added" || kind == "response.content_part.done")
                && member(object(member(item, "part").unwrap_or(&Json::Null))?, "type")
                    == Some(&Json::String("refusal".to_string()))
            {
                return Err(LlmError::settings(format!("{label} declined the request.")));
            }
            if kind == "response.output_text.delta" {
                match member(item, "delta") {
                    Some(Json::String(delta)) if !completed => {
                        text.push_str(delta);
                        input.emit(delta)?;
                    }
                    _ => return Err(LlmError::settings("Invalid LLM response.")),
                }
            } else if kind == "response.completed" {
                let result = object(member(item, "response").unwrap_or(&Json::Null))?;
                if member(result, "status") != Some(&Json::String("completed".to_string())) {
                    return Err(LlmError::settings(format!(
                        "{label} response did not complete. Try again."
                    )));
                }
                if let Some(output) = member(result, "output") {
                    let Json::Array(entries) = output else {
                        return Err(LlmError::settings("Invalid LLM response."));
                    };
                    let mut combined = String::new();
                    for entry in entries {
                        combined.push_str(&output_item(entry, label)?);
                    }
                    if !entries.is_empty() && !text.is_empty() && combined != text {
                        return Err(LlmError::settings(format!(
                            "{label} returned inconsistent response text."
                        )));
                    }
                    if text.is_empty() && !combined.is_empty() {
                        input.emit(&combined)?;
                        text = combined;
                    }
                }
                completed = true;
                return Ok(true);
            }
            Ok(false)
        },
        require_content_type,
    )?;
    if !completed {
        return Err(LlmError::settings(format!(
            "{label} response ended before completion. Try again."
        )));
    }
    if text.trim().is_empty() {
        return Err(LlmError::settings(format!("{label} returned no text. Try again.")));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::super::request::CancelToken;
    use super::*;

    fn input<'a>() -> TransportRequest<'a> {
        TransportRequest {
            prompt: "hello",
            instructions: "Reply plainly.",
            model: "test-model",
            reasoning_effort: None,
            cancel: CancelToken::new(),
            on_text: None,
        }
    }

    fn event(value: &str) -> String {
        format!("data: {value}\n\n")
    }

    fn stream(body: &str) -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: Some("text/event-stream".to_string()),
            body: body.as_bytes().to_vec(),
        }
    }

    fn completed(text: &str) -> String {
        event(&format!(
            "{{\"type\":\"response.completed\",\"response\":{{\"status\":\"completed\",\"output\":[{{\"type\":\"message\",\"content\":[{{\"type\":\"output_text\",\"text\":\"{text}\"}}]}}]}}}}"
        ))
    }

    #[test]
    fn streams_deltas_and_accepts_authoritative_completion() {
        let mut deltas = Vec::new();
        let mut request = input();
        request.on_text = Some(Box::new(|text: &str| deltas.push(text.to_string())));
        let body = event("{\"type\":\"response.output_text.delta\",\"delta\":\"caf\u{e9} \u{1f3ae}\"}")
            + &completed("caf\u{e9} \u{1f3ae}");
        let answer = read_responses(&mut request, &stream(&body), "OpenAI API", true).unwrap();
        assert_eq!(answer, "caf\u{e9} \u{1f3ae}");
        drop(request);
        assert_eq!(deltas, vec!["caf\u{e9} \u{1f3ae}".to_string()]);
    }

    #[test]
    fn completion_text_is_authoritative_and_checked() {
        assert_eq!(
            read_responses(
                &mut input(),
                &stream(&completed("echo complete")),
                "Subscription",
                false
            )
            .unwrap(),
            "echo complete"
        );
        let body = event("{\"type\":\"response.output_text.delta\",\"delta\":\"quit\"}") + &completed("echo complete");
        let error = read_responses(&mut input(), &stream(&body), "Subscription", false).unwrap_err();
        assert!(error.to_string().contains("inconsistent"), "{error}");
    }

    #[test]
    fn refuses_tools_refusals_and_failures_without_provider_text() {
        for output in [
            "{\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"name\":\"quit\"}}",
            "{\"type\":\"response.refusal.delta\",\"delta\":\"secret\"}",
            "{\"type\":\"response.function_call_arguments.delta\",\"delta\":\"secret\"}",
        ] {
            let error = read_responses(&mut input(), &stream(&event(output)), "Subscription", false).unwrap_err();
            assert!(!error.to_string().contains("secret"), "{error}");
        }
        let failed = event("{\"type\":\"response.incomplete\",\"response\":{\"status\":\"incomplete\"}}");
        assert_eq!(
            read_responses(&mut input(), &stream(&failed), "Subscription", false)
                .unwrap_err()
                .to_string(),
            "Subscription response failed. Try again."
        );
    }

    #[test]
    fn truncated_and_empty_completions_fail() {
        let truncated = event("{\"type\":\"response.output_text.delta\",\"delta\":\"quit\"}");
        assert_eq!(
            read_responses(&mut input(), &stream(&truncated), "Subscription", false)
                .unwrap_err()
                .to_string(),
            "Subscription response ended before completion. Try again."
        );
        let empty = event("{\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}");
        assert_eq!(
            read_responses(&mut input(), &stream(&empty), "Subscription", false)
                .unwrap_err()
                .to_string(),
            "Subscription returned no text. Try again."
        );
    }
}
