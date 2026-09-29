//! API transports: OpenAI chat completions and Responses endpoints.
//!
//! Donor provenance: `src/llm/api.ts` (`requestChatCompletions`,
//! `requestOpenAiResponses`). Same request schemas, same streamed chunk
//! validation (index 0 only, no tool calls, no refusals, `stop` finish,
//! `[DONE]` terminator, nonempty text), and the same fixed errors.

use crate::settings::json::{parse_json, stringify, Json};

use super::auth::record;
use super::errors::LlmError;
use super::request::{fetch_llm_response, FetchInit, FetchMethod, LlmFetch, TransportRequest};
use super::responses::read_responses;
use super::sse::consume_sse;

fn member<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

fn is_nonempty_string(value: &Json) -> bool {
    matches!(value, Json::String(text) if !text.is_empty())
}

/// POST one chat-completions request and stream its text (donor `requestChatCompletions`).
pub fn request_chat_completions(
    input: &mut TransportRequest,
    api_key: &str,
    base_url: &str,
    fetcher: &impl LlmFetch,
) -> Result<String, LlmError> {
    let mut body_members = Vec::new();
    body_members.push(("model".to_string(), Json::String(input.model.to_string())));
    if let Some(effort) = input.reasoning_effort {
        body_members.push(("reasoning_effort".to_string(), Json::String(effort.to_string())));
    }
    body_members.push((
        "messages".to_string(),
        Json::Array(vec![
            Json::Object(vec![
                ("role".to_string(), Json::String("system".to_string())),
                ("content".to_string(), Json::String(input.instructions.to_string())),
            ]),
            Json::Object(vec![
                ("role".to_string(), Json::String("user".to_string())),
                ("content".to_string(), Json::String(input.prompt.to_string())),
            ]),
        ]),
    ));
    body_members.push(("stream".to_string(), Json::Bool(true)));
    let response = fetch_llm_response(
        &format!("{}/chat/completions", base_url.trim_end_matches('/')),
        &FetchInit {
            method: FetchMethod::Post,
            headers: vec![
                ("authorization".to_string(), format!("Bearer {api_key}")),
                ("content-type".to_string(), "application/json".to_string()),
                ("accept".to_string(), "text/event-stream".to_string()),
            ],
            body: Some(stringify(&Json::Object(body_members))),
        },
        fetcher,
        &input.cancel.clone(),
    )?;
    if !response.ok() {
        return Err(LlmError::http(response.status));
    }
    let cancel = input.cancel.clone();
    let mut text = String::new();
    let mut finished = false;
    let mut done = false;
    consume_sse(
        &response,
        &cancel,
        |event| {
            if event.data == "[DONE]" {
                done = true;
                return Ok(true);
            }
            let value =
                parse_json(&event.data).map_err(|_| LlmError::settings("LLM service returned invalid stream data."))?;
            let chunk = record(&value)?;
            if member(chunk, "error").is_some() || event.event == "error" {
                return Err(LlmError::settings("LLM service reported a response error."));
            }
            let Some(Json::Array(choices)) = member(chunk, "choices") else {
                return Err(LlmError::settings("LLM service returned invalid completion data."));
            };
            for item in choices {
                let choice = record(item)?;
                if member(choice, "index") != Some(&Json::Number(0.0)) {
                    continue;
                }
                let delta = record(member(choice, "delta").unwrap_or(&Json::Null))?;
                let content = member(delta, "content");
                let finish = member(choice, "finish_reason");
                if member(delta, "tool_calls").is_some() || member(delta, "function_call").is_some() {
                    return Err(LlmError::settings("LLM service returned an unsupported tool call."));
                }
                if member(delta, "refusal").is_some_and(is_nonempty_string) {
                    return Err(LlmError::settings("LLM service declined the request."));
                }
                if let Some(content) = content {
                    if !matches!(content, Json::Null) {
                        let Json::String(chunk) = content else {
                            return Err(LlmError::settings("LLM service returned invalid completion content."));
                        };
                        if finished {
                            return Err(LlmError::settings("LLM service returned invalid completion content."));
                        }
                        text.push_str(chunk);
                        if !chunk.is_empty() {
                            input.emit(chunk)?;
                        }
                    }
                }
                if let Some(finish) = finish {
                    if !matches!(finish, Json::Null) {
                        if finish != &Json::String("stop".to_string()) {
                            return Err(LlmError::settings(
                                "LLM response was incomplete or declined. Try a shorter request.",
                            ));
                        }
                        finished = true;
                    }
                }
            }
            Ok(false)
        },
        true,
    )?;
    if !done || !finished {
        return Err(LlmError::settings("LLM response ended before completion. Try again."));
    }
    if text.trim().is_empty() {
        return Err(LlmError::settings("LLM service returned an empty response."));
    }
    Ok(text)
}

/// POST one OpenAI Responses request and stream its text (donor `requestOpenAiResponses`).
pub fn request_openai_responses(
    input: &mut TransportRequest,
    api_key: &str,
    fetcher: &impl LlmFetch,
) -> Result<String, LlmError> {
    let mut body_members = vec![
        ("model".to_string(), Json::String(input.model.to_string())),
        ("instructions".to_string(), Json::String(input.instructions.to_string())),
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
        ("store".to_string(), Json::Bool(false)),
        ("stream".to_string(), Json::Bool(true)),
    ];
    if let Some(effort) = input.reasoning_effort {
        body_members.push((
            "reasoning".to_string(),
            Json::Object(vec![("effort".to_string(), Json::String(effort.to_string()))]),
        ));
    }
    let response = fetch_llm_response(
        "https://api.openai.com/v1/responses",
        &FetchInit {
            method: FetchMethod::Post,
            headers: vec![
                ("authorization".to_string(), format!("Bearer {api_key}")),
                ("content-type".to_string(), "application/json".to_string()),
                ("accept".to_string(), "text/event-stream".to_string()),
            ],
            body: Some(stringify(&Json::Object(body_members))),
        },
        fetcher,
        &input.cancel.clone(),
    )?;
    if !response.ok() {
        return Err(LlmError::http(response.status));
    }
    read_responses(input, &response, "OpenAI API", true)
}

#[cfg(test)]
mod tests {
    use super::super::request::{CancelToken, LlmResponse};
    use super::*;

    fn input<'a>() -> TransportRequest<'a> {
        TransportRequest {
            prompt: "hello",
            instructions: "answer",
            model: "test-model",
            reasoning_effort: None,
            cancel: CancelToken::new(),
            on_text: None,
        }
    }

    fn data(value: &str) -> String {
        format!("data: {value}\r\n\r\n")
    }

    fn completion(text: &str) -> String {
        data(&format!(
            "{{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{text}\"}},\"finish_reason\":null}}]}}"
        )) + &data("{\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}")
            + "data: [DONE]\r\n\r\n"
    }

    fn stream(body: &str) -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: Some("text/event-stream; charset=utf-8".to_string()),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn chat_posts_schema_and_streams_unicode() {
        let mut deltas = Vec::new();
        let mut request = input();
        request.on_text = Some(Box::new(|text: &str| deltas.push(text.to_string())));
        let answer = request_chat_completions(
            &mut request,
            "key",
            "https://example.test/v1/",
            &|url: &str, init: &FetchInit| {
                assert_eq!(url, "https://example.test/v1/chat/completions");
                assert_eq!(init.method, FetchMethod::Post);
                assert_eq!(init.header("authorization"), Some("Bearer key"));
                let body = parse_json(init.body.as_deref().unwrap()).unwrap();
                assert_eq!(body.get("model"), Some(&Json::String("test-model".to_string())));
                assert_eq!(body.get("stream"), Some(&Json::Bool(true)));
                assert!(body.get("reasoning_effort").is_none());
                Ok(stream(&completion("caf\u{e9} \u{1f3ae}")))
            },
        )
        .unwrap();
        assert_eq!(answer, "caf\u{e9} \u{1f3ae}");
        drop(request);
        assert_eq!(deltas, vec!["caf\u{e9} \u{1f3ae}".to_string()]);
    }

    #[test]
    fn chat_rejects_truncated_malformed_incomplete_and_tool_chunks() {
        for body in [
            data("{\"choices\":[{\"index\":0,\"delta\":{\"content\":\"quit\"},\"finish_reason\":null}]}"),
            "data: invalid-json\n\n".to_string(),
            data("{\"choices\":[{\"index\":0,\"delta\":{\"content\":\"quit\"},\"finish_reason\":\"length\"}]}")
                + "data: [DONE]\n\n",
            data("{\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[]},\"finish_reason\":\"tool_calls\"}]}"),
            data("{\"choices\":[{\"index\":0,\"delta\":{\"refusal\":\"no\"},\"finish_reason\":null}]}"),
            data("{\"choices\":[{\"index\":1,\"delta\":{\"content\":\"other\"},\"finish_reason\":null}]}")
                + &data("{\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}")
                + "data: [DONE]\n\n",
        ] {
            let error = request_chat_completions(
                &mut input(),
                "key",
                "https://example.test/v1",
                &|_: &str, _: &FetchInit| Ok(stream(&body)),
            )
            .unwrap_err();
            assert!(matches!(error, LlmError::Settings(_)), "{error}");
        }
    }

    #[test]
    fn chat_skips_other_indices_but_requires_first_index_text() {
        let body = data("{\"choices\":[{\"index\":1,\"delta\":{\"content\":\"ignored\"},\"finish_reason\":\"stop\"}]}")
            + &data("{\"choices\":[{\"index\":0,\"delta\":{\"content\":\"kept\"},\"finish_reason\":null}]}")
            + &data("{\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}")
            + "data: [DONE]\n\n";
        let answer = request_chat_completions(
            &mut input(),
            "key",
            "https://example.test/v1",
            &|_: &str, _: &FetchInit| Ok(stream(&body)),
        )
        .unwrap();
        assert_eq!(answer, "kept");
    }

    #[test]
    fn responses_posts_schema_and_returns_text() {
        let payload = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n".to_string()
            + "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n";
        let answer = request_openai_responses(&mut input(), "key", &|url: &str, init: &FetchInit| {
            assert_eq!(url, "https://api.openai.com/v1/responses");
            let sent = parse_json(init.body.as_deref().unwrap()).unwrap();
            assert_eq!(sent.get("store"), Some(&Json::Bool(false)));
            assert!(sent.get("reasoning").is_none());
            Ok(stream(&payload))
        })
        .unwrap();
        assert_eq!(answer, "ok");
    }

    #[test]
    fn responses_sends_reasoning_effort_when_set() {
        let payload = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\n".to_string()
            + "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n";
        let mut request = input();
        request.reasoning_effort = Some("high");
        request_openai_responses(&mut request, "key", &|_: &str, init: &FetchInit| {
            let sent = parse_json(init.body.as_deref().unwrap()).unwrap();
            assert_eq!(
                sent.get("reasoning").and_then(|reasoning| reasoning.get("effort")),
                Some(&Json::String("high".to_string()))
            );
            Ok(stream(&payload))
        })
        .unwrap();
    }
}
