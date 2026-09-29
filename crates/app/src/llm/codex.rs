//! ChatGPT subscription transport over the Codex Responses endpoint.
//!
//! Donor provenance: `src/llm/codex.ts` (`subscriptionAccountId`,
//! `requestCodex`). Same account-scoped headers, same request schema
//! (model prefix stripped, default instructions, reasoning effort,
//! verbosity, encrypted reasoning include, cache key), same MIME-agnostic
//! Responses reading under the `Subscription` label.

use crate::settings::json::{parse_json, stringify, Json};

use super::auth::{base64url_decode, random_uuid, record, SubscriptionCredential};
use super::errors::LlmError;
use super::request::{fetch_llm_response, FetchInit, FetchMethod, LlmFetch, TransportRequest};
use super::responses::read_responses;

/// Extract the ChatGPT account id from a subscription JWT (donor `subscriptionAccountId`).
pub fn subscription_account_id(token: &str) -> Result<String, LlmError> {
    let invalid = || LlmError::settings("Subscription credential has no valid ChatGPT account ID. Sign in again.");
    let payload = token.split('.').nth(1).ok_or_else(invalid)?;
    if payload.is_empty()
        || !payload
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(invalid());
    }
    let bytes = base64url_decode(payload).map_err(|()| invalid())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let value = parse_json(text).map_err(|_| invalid())?;
    let claims = record(&value).map_err(|_| invalid())?;
    let namespaced = claims
        .iter()
        .find(|(name, _)| name == "https://api.openai.com/auth")
        .map(|(_, auth)| auth)
        .ok_or_else(invalid)?;
    let auth = record(namespaced).map_err(|_| invalid())?;
    match auth.iter().find(|(name, _)| name == "chatgpt_account_id") {
        Some((_, Json::String(id)))
            if !id.trim().is_empty() && !id.bytes().any(|byte| byte <= 0x20 || byte == 0x7f) =>
        {
            Ok(id.clone())
        }
        _ => Err(invalid()),
    }
}

/// POST one subscription request and stream its text (donor `requestCodex`).
pub fn request_codex(
    input: &mut TransportRequest,
    credential: &SubscriptionCredential,
    fetcher: &impl LlmFetch,
) -> Result<String, LlmError> {
    let account = subscription_account_id(&credential.access_token)?;
    let session = random_uuid();
    let model = input
        .model
        .strip_prefix("openai:")
        .or_else(|| input.model.strip_prefix("openai/"))
        .unwrap_or(input.model);
    let instructions = if input.instructions.trim().is_empty() {
        "You are a helpful assistant.".to_string()
    } else {
        input.instructions.trim().to_string()
    };
    let mut body_members = vec![
        ("model".to_string(), Json::String(model.to_string())),
        ("store".to_string(), Json::Bool(false)),
        ("stream".to_string(), Json::Bool(true)),
        ("instructions".to_string(), Json::String(instructions)),
        (
            "input".to_string(),
            Json::Array(vec![Json::Object(vec![
                ("role".to_string(), Json::String("user".to_string())),
                (
                    "content".to_string(),
                    Json::Array(vec![Json::Object(vec![
                        ("type".to_string(), Json::String("input_text".to_string())),
                        ("text".to_string(), Json::String(input.prompt.to_string())),
                    ])]),
                ),
            ])]),
        ),
    ];
    if let Some(effort) = input.reasoning_effort {
        body_members.push((
            "reasoning".to_string(),
            Json::Object(vec![("effort".to_string(), Json::String(effort.to_string()))]),
        ));
    }
    body_members.push((
        "text".to_string(),
        Json::Object(vec![("verbosity".to_string(), Json::String("medium".to_string()))]),
    ));
    body_members.push((
        "include".to_string(),
        Json::Array(vec![Json::String("reasoning.encrypted_content".to_string())]),
    ));
    body_members.push(("prompt_cache_key".to_string(), Json::String(session.clone())));
    let response = fetch_llm_response(
        "https://chatgpt.com/backend-api/codex/responses",
        &FetchInit {
            method: FetchMethod::Post,
            headers: vec![
                (
                    "authorization".to_string(),
                    format!("Bearer {}", credential.access_token),
                ),
                ("chatgpt-account-id".to_string(), account),
                ("originator".to_string(), "pi".to_string()),
                ("OpenAI-Beta".to_string(), "responses=experimental".to_string()),
                ("accept".to_string(), "text/event-stream".to_string()),
                ("content-type".to_string(), "application/json".to_string()),
                (
                    "User-Agent".to_string(),
                    format!("pi ({}; {})", std::env::consts::OS, std::env::consts::ARCH),
                ),
                ("session_id".to_string(), session),
            ],
            body: Some(stringify(&Json::Object(body_members))),
        },
        fetcher,
        &input.cancel.clone(),
    )?;
    if !response.ok() {
        return Err(LlmError::http(response.status));
    }
    read_responses(input, &response, "Subscription", false)
}

#[cfg(test)]
mod tests {
    use super::super::auth::base64url_encode;
    use super::super::request::{CancelToken, LlmResponse};
    use super::*;

    fn credential() -> SubscriptionCredential {
        let payload =
            base64url_encode("{\"https://api.openai.com/auth\":{\"chatgpt_account_id\":\"test-account\"}}".as_bytes());
        SubscriptionCredential {
            access_token: format!("header.{payload}.signature"),
            refresh_token: "test-refresh".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: 1234,
            scopes: Vec::new(),
        }
    }

    fn input<'a>() -> TransportRequest<'a> {
        TransportRequest {
            prompt: "hello",
            instructions: "Reply plainly.",
            model: "openai:test-model",
            reasoning_effort: None,
            cancel: CancelToken::new(),
            on_text: None,
        }
    }

    fn event(value: &str) -> String {
        format!("data: {value}\n\n")
    }

    fn stream(body: &str, content_type: Option<&str>) -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: content_type.map(str::to_string),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn posts_account_scoped_schema_and_streams_text() {
        let body = event("{\"type\":\"response.created\"}")
            + &event("{\"type\":\"response.output_text.delta\",\"delta\":\"caf\u{e9} \"}")
            + &event("{\"type\":\"response.output_text.delta\",\"delta\":\"\u{1f3ae}\"}")
            + &event("{\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}");
        let answer = request_codex(&mut input(), &credential(), &|url: &str, init: &FetchInit| {
            assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
            assert_eq!(init.header("chatgpt-account-id"), Some("test-account"));
            assert_eq!(init.header("OpenAI-Beta"), Some("responses=experimental"));
            assert_eq!(init.header("originator"), Some("pi"));
            let sent = parse_json(init.body.as_deref().unwrap()).unwrap();
            assert_eq!(sent.get("model"), Some(&Json::String("test-model".to_string())));
            assert_eq!(sent.get("store"), Some(&Json::Bool(false)));
            assert_eq!(
                sent.get("prompt_cache_key").and_then(|key| match key {
                    Json::String(key) => Some(key.as_str()),
                    _ => None,
                }),
                init.header("session_id")
            );
            Ok(stream(&body, Some("text/event-stream")))
        })
        .unwrap();
        assert_eq!(answer, "caf\u{e9} \u{1f3ae}");
    }

    #[test]
    fn rejects_absent_account_id_before_sending() {
        let mut bad = credential();
        bad.access_token = "secret.invalid.signature".to_string();
        let error = request_codex(&mut input(), &bad, &|_: &str, _: &FetchInit| panic!("must not fetch")).unwrap_err();
        assert!(error.to_string().contains("Sign in again"), "{error}");
    }

    #[test]
    fn rejects_failures_truncation_and_empty_text() {
        for kind in ["response.failed", "response.incomplete", "error"] {
            let body = event(&format!("{{\"type\":\"{kind}\",\"error\":{{\"message\":\"secret\"}}}}"));
            let error = request_codex(&mut input(), &credential(), &|_: &str, _: &FetchInit| {
                Ok(stream(&body, Some("text/event-stream")))
            })
            .unwrap_err();
            assert_eq!(error.to_string(), "Subscription response failed. Try again.");
        }
        let truncated = event("{\"type\":\"response.output_text.delta\",\"delta\":\"quit\"}");
        assert!(
            request_codex(&mut input(), &credential(), &|_: &str, _: &FetchInit| Ok(stream(
                &truncated,
                Some("text/event-stream")
            )))
            .unwrap_err()
            .to_string()
            .contains("before completion")
        );
        let empty = event("{\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}");
        assert_eq!(
            request_codex(&mut input(), &credential(), &|_: &str, _: &FetchInit| Ok(stream(
                &empty,
                Some("text/event-stream")
            )))
            .unwrap_err()
            .to_string(),
            "Subscription returned no text. Try again."
        );
    }

    #[test]
    fn preserves_http_auth_status_and_accepts_any_mime() {
        for status in [401u16, 403] {
            let error = request_codex(&mut input(), &credential(), &|_: &str, _: &FetchInit| {
                Ok(LlmResponse {
                    status,
                    content_type: None,
                    body: b"secret-server-body".to_vec(),
                })
            })
            .unwrap_err();
            assert_eq!(error.status(), Some(status));
            assert!(!error.to_string().contains("secret"));
        }
        let payload = event("{\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}")
            + &event("{\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}");
        for media in [
            None,
            Some("application/octet-stream"),
            Some("text/plain"),
            Some("application/json"),
        ] {
            let answer = request_codex(&mut input(), &credential(), &|_: &str, _: &FetchInit| {
                Ok(stream(&payload, media))
            })
            .unwrap();
            assert_eq!(answer, "answer");
        }
    }

    #[test]
    fn rejects_refusal_and_tool_events() {
        for rejected in [
            "{\"type\":\"response.refusal.delta\",\"delta\":\"x\"}",
            "{\"type\":\"response.function_call_arguments.delta\",\"delta\":\"x\"}",
            "{\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\"}}",
            "{\"type\":\"response.content_part.done\",\"part\":{\"type\":\"refusal\"}}",
        ] {
            let body = event("{\"type\":\"response.output_text.delta\",\"delta\":\"quit\"}")
                + &event(rejected)
                + &event("{\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}");
            let error = request_codex(&mut input(), &credential(), &|_: &str, _: &FetchInit| {
                Ok(stream(&body, Some("text/event-stream")))
            })
            .unwrap_err();
            assert!(
                error.to_string().contains("declined") || error.to_string().contains("unsupported"),
                "{error}"
            );
        }
    }
}
