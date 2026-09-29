//! LLM model catalogs: API listing, Codex listing, reasoning metadata.
//!
//! Donor provenance: `src/llm/models.ts` (`LlmModel`, `parseApiModels`,
//! `parseCodexModels`, `apiModel`, `parseReasoningEffort`,
//! `discoverApiModels`, `discoverCodexModels`). Same reference reasoning
//! table (verified 2026-09-16), same chat-model filter, same Codex
//! priority ordering with the first visible model recommended, same 4 MiB
//! list limit, same fixed errors.

use std::collections::HashMap;

use crate::settings::json::{parse_json, Json};

use super::auth::{record, SubscriptionCredential};
use super::codex;
use super::errors::LlmError;
use super::request::{check_request_abort, fetch_llm_response, CancelToken, FetchInit, FetchMethod, LlmFetch};

/// Maximum accepted model-list size in bytes.
pub const MAX_MODEL_LIST_BYTES: usize = 4_194_304;

/// Where a model's reasoning choices come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningSource {
    /// Listed by the provider itself.
    Provider,
    /// From the verified reference table.
    Reference,
    /// Unknown; the model offers Model default only.
    Unknown,
}

/// One listed model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmModel {
    /// Provider model id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Supported reasoning efforts.
    pub reasoning_efforts: Vec<String>,
    /// Provider default effort, when it names a supported effort.
    pub default_reasoning_effort: Option<String>,
    /// Provenance of the reasoning choices.
    pub reasoning_source: ReasoningSource,
    /// Provider-recommended (first Codex model by priority).
    pub recommended: bool,
}

/// Catalog load state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogStatus {
    /// Never loaded (or invalidated).
    Idle,
    /// Load in flight.
    Loading,
    /// Loaded.
    Ready,
    /// Load failed; `message` explains.
    Error,
}

/// One provider's model catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmModelCatalog {
    /// Load state.
    pub status: CatalogStatus,
    /// Last loaded models (kept across failed reloads).
    pub models: Vec<LlmModel>,
    /// Failure message, when `status` is [`CatalogStatus::Error`].
    pub message: Option<String>,
}

impl LlmModelCatalog {
    /// An empty idle catalog.
    #[must_use]
    pub const fn idle() -> Self {
        Self {
            status: CatalogStatus::Idle,
            models: Vec::new(),
            message: None,
        }
    }
}

fn member<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

/// Validated metadata string (donor `string` in models.ts).
fn metadata(value: &Json) -> Result<String, LlmError> {
    match value {
        Json::String(text)
            if !text.trim().is_empty()
                && text.chars().count() <= 256
                && !text.bytes().any(|byte| byte < 0x20 || byte == 0x7f) =>
        {
            Ok(text.trim().to_string())
        }
        _ => Err(LlmError::settings("The service returned invalid model metadata.")),
    }
}

/// Parse a reasoning effort (`null`/missing becomes `None`).
pub fn parse_reasoning_effort(value: &Json) -> Result<Option<String>, LlmError> {
    match value {
        Json::Null => Ok(None),
        Json::String(_) => {
            let effort = metadata(value).map_err(|_| LlmError::settings("Invalid reasoning effort."))?;
            let mut bytes = effort.bytes();
            let valid = matches!(bytes.next(), Some(b'a'..=b'z'))
                && effort.len() <= 64
                && bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'));
            if valid {
                Ok(Some(effort))
            } else {
                Err(LlmError::settings("Invalid reasoning effort."))
            }
        }
        _ => Err(LlmError::settings("Invalid reasoning effort.")),
    }
}

/// Official API reasoning table (donor `apiReasoning`).
const API_REASONING: &[(&str, &[&str])] = &[
    ("gpt-5", &["minimal", "low", "medium", "high"]),
    ("gpt-5-pro", &["high"]),
    ("gpt-5.2-pro", &["medium", "high", "xhigh"]),
    ("gpt-5.4", &["none", "low", "medium", "high", "xhigh"]),
    ("gpt-5.4-pro", &["medium", "high", "xhigh"]),
    ("gpt-5.4-mini", &["none", "low", "medium", "high", "xhigh"]),
    ("gpt-5.4-nano", &["none", "low", "medium", "high", "xhigh"]),
    ("gpt-5.6-sol", &["none", "low", "medium", "high", "xhigh", "max"]),
    ("gpt-5.1", &["none", "low", "medium", "high"]),
    ("gpt-5.2", &["none", "low", "medium", "high", "xhigh"]),
    ("gpt-5.5", &["none", "low", "medium", "high", "xhigh"]),
    ("gpt-5.5-pro", &["medium", "high", "xhigh"]),
    ("gpt-6-astra", &["low", "medium", "high", "xhigh", "max"]),
    ("gpt-5.6-terra", &["none", "low", "medium", "high", "xhigh", "max"]),
    ("gpt-5.6-luna", &["none", "low", "medium", "high", "xhigh", "max"]),
];

/// Strip a dated `-YYYY-MM-DD` snapshot suffix for table lookup.
fn undated(id: &str) -> &str {
    if id.len() > 11 {
        let (base, suffix) = id.split_at(id.len() - 11);
        let bytes = suffix.as_bytes();
        let dated = bytes[0] == b'-'
            && bytes[1..5].iter().all(|byte| byte.is_ascii_digit())
            && bytes[5] == b'-'
            && bytes[6..8].iter().all(|byte| byte.is_ascii_digit())
            && bytes[8] == b'-'
            && bytes[9..11].iter().all(|byte| byte.is_ascii_digit());
        if dated {
            return base;
        }
    }
    id
}

/// Reference metadata for one OpenAI API model id (donor `apiModel`).
pub fn api_model(id: &str) -> LlmModel {
    let efforts = API_REASONING
        .iter()
        .find(|(name, _)| *name == undated(id))
        .map(|(_, efforts)| efforts);
    LlmModel {
        id: id.to_string(),
        name: id.to_string(),
        reasoning_efforts: efforts.map_or_else(Vec::new, |efforts| {
            efforts.iter().map(|effort| (*effort).to_string()).collect()
        }),
        default_reasoning_effort: None,
        reasoning_source: if efforts.is_some() {
            ReasoningSource::Reference
        } else {
            ReasoningSource::Unknown
        },
        recommended: false,
    }
}

/// Whether an OpenAI model id names a non-chat model (donor `NON_CHAT`).
fn is_non_chat(id: &str) -> bool {
    let folded = id.to_ascii_lowercase();
    folded.starts_with("ada")
        || [
            "embedding",
            "whisper",
            "tts",
            "dall-e",
            "davinci",
            "babbage",
            "moderation",
            "text-search",
            "similarity",
            "transcribe",
            "speech",
            "realtime",
            "image",
        ]
        .iter()
        .any(|marker| folded.contains(marker))
}

/// Parse an OpenAI-style `/models` list (donor `parseApiModels`).
///
/// `openai` selects chat filtering plus reference capabilities; other
/// services never inherit OpenAI capability claims.
pub fn parse_api_models(value: &Json, openai: bool) -> Result<Vec<LlmModel>, LlmError> {
    let data = member(
        record(value).map_err(|_| LlmError::settings("The service returned an invalid model list."))?,
        "data",
    );
    let Some(Json::Array(data)) = data else {
        return Err(LlmError::settings("The service returned an invalid model list."));
    };
    let mut models: HashMap<String, LlmModel> = HashMap::new();
    for value in data {
        let id = metadata(
            member(
                record(value).map_err(|_| LlmError::settings("The service returned an invalid model list."))?,
                "id",
            )
            .unwrap_or(&Json::Null),
        )
        .map_err(|_| LlmError::settings("The service returned an invalid model list."))?;
        if openai && is_non_chat(&id) {
            continue;
        }
        models.insert(
            id.clone(),
            if openai {
                api_model(&id)
            } else {
                LlmModel {
                    id: id.clone(),
                    name: id,
                    reasoning_efforts: Vec::new(),
                    default_reasoning_effort: None,
                    reasoning_source: ReasoningSource::Unknown,
                    recommended: false,
                }
            },
        );
    }
    let mut models: Vec<LlmModel> = models.into_values().collect();
    models.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(models)
}

/// Parse a Codex `/models` list (donor `parseCodexModels`).
pub fn parse_codex_models(value: &Json) -> Result<Vec<LlmModel>, LlmError> {
    let invalid_list = || LlmError::settings("The subscription service returned an invalid model list.");
    let invalid_reasoning = || LlmError::settings("The subscription service returned invalid reasoning choices.");
    let invalid_priority = || LlmError::settings("The subscription service returned invalid model priority metadata.");
    let list = member(record(value).map_err(|_| invalid_list())?, "models");
    let Some(Json::Array(list)) = list else {
        return Err(invalid_list());
    };
    let mut ranked: Vec<(f64, LlmModel)> = Vec::new();
    for value in list {
        let item = record(value).map_err(|_| invalid_list())?;
        if member(item, "visibility") != Some(&Json::String("list".to_string())) {
            continue;
        }
        let id = metadata(member(item, "slug").unwrap_or(&Json::Null)).map_err(|_| invalid_list())?;
        let Some(Json::Array(levels)) = member(item, "supported_reasoning_levels") else {
            return Err(invalid_reasoning());
        };
        let mut efforts = Vec::new();
        for level in levels {
            let level = record(level).map_err(|_| invalid_reasoning())?;
            match parse_reasoning_effort(member(level, "effort").unwrap_or(&Json::Null))? {
                Some(effort) if !efforts.contains(&effort) => efforts.push(effort),
                Some(_) => {}
                None => return Err(invalid_reasoning()),
            }
        }
        let default = parse_reasoning_effort(member(item, "default_reasoning_level").unwrap_or(&Json::Null))?;
        let default = match default {
            Some(effort) if efforts.contains(&effort) => Some(effort),
            _ => None,
        };
        let Some(Json::Number(priority)) = member(item, "priority") else {
            return Err(invalid_priority());
        };
        if !priority.is_finite() {
            return Err(invalid_priority());
        }
        let name = match member(item, "display_name") {
            Some(name @ Json::String(_)) => metadata(name).map_err(|_| invalid_list())?,
            None => id.clone(),
            Some(_) => return Err(invalid_list()),
        };
        ranked.push((
            *priority,
            LlmModel {
                id,
                name,
                reasoning_efforts: efforts,
                default_reasoning_effort: default,
                reasoning_source: ReasoningSource::Provider,
                recommended: member(item, "is_default") == Some(&Json::Bool(true)),
            },
        ));
    }
    ranked.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut ordered: Vec<LlmModel> = Vec::new();
    for (_, model) in ranked {
        if let Some(existing) = ordered.iter_mut().find(|known| known.id == model.id) {
            *existing = model;
        } else {
            ordered.push(model);
        }
    }
    for (index, model) in ordered.iter_mut().enumerate() {
        model.recommended = index == 0;
    }
    Ok(ordered)
}

/// Read one model-list body (donor `modelJson`).
fn model_json(response: super::request::LlmResponse, cancel: &CancelToken) -> Result<Json, LlmError> {
    check_request_abort(cancel)?;
    if !response.ok() {
        return Err(LlmError::http(response.status));
    }
    if response.body.is_empty() {
        return Err(LlmError::settings("The service returned an empty model list response."));
    }
    if response.body.len() > MAX_MODEL_LIST_BYTES {
        return Err(LlmError::settings("The service model list exceeded the size limit."));
    }
    let text = std::str::from_utf8(&response.body)
        .map_err(|_| LlmError::settings("The service returned an invalid model list response."))?;
    parse_json(text).map_err(|_| LlmError::settings("The service returned an invalid model list response."))
}

/// Discover an OpenAI-style `/models` list (donor `discoverApiModels`).
pub fn discover_api_models(
    base_url: &str,
    api_key: &str,
    openai: bool,
    fetcher: &impl LlmFetch,
    cancel: &CancelToken,
) -> Result<Vec<LlmModel>, LlmError> {
    let response = fetch_llm_response(
        &format!("{}/models", base_url.trim_end_matches('/')),
        &FetchInit {
            method: FetchMethod::Get,
            headers: vec![
                ("authorization".to_string(), format!("Bearer {api_key}")),
                ("accept".to_string(), "application/json".to_string()),
            ],
            body: None,
        },
        fetcher,
        cancel,
    )?;
    parse_api_models(&model_json(response, cancel)?, openai)
}

/// Discover the Codex `/models` list (donor `discoverCodexModels`).
pub fn discover_codex_models(
    credential: &SubscriptionCredential,
    fetcher: &impl LlmFetch,
    cancel: &CancelToken,
) -> Result<Vec<LlmModel>, LlmError> {
    let account = codex::subscription_account_id(&credential.access_token)?;
    // Compatibility version matches released Codex 0.154.0.
    let response = fetch_llm_response(
        "https://chatgpt.com/backend-api/codex/models?client_version=0.154.0",
        &FetchInit {
            method: FetchMethod::Get,
            headers: vec![
                (
                    "authorization".to_string(),
                    format!("Bearer {}", credential.access_token),
                ),
                ("chatgpt-account-id".to_string(), account),
                ("originator".to_string(), "pi".to_string()),
                ("accept".to_string(), "application/json".to_string()),
            ],
            body: None,
        },
        fetcher,
        cancel,
    )?;
    parse_codex_models(&model_json(response, cancel)?)
}

#[cfg(test)]
mod tests {
    use super::super::auth::base64url_encode;
    use super::super::request::LlmResponse;
    use super::*;

    const API_LIST: &str = "{\"object\":\"list\",\"data\":[{\"id\":\"gpt-5\",\"object\":\"model\"},{\"id\":\"gpt-4.1\",\"object\":\"model\"},{\"id\":\"text-embedding-test\",\"object\":\"model\"},{\"id\":\"unknown-chat\",\"object\":\"model\"}]}";
    const CODEX_LIST: &str = "{\"models\":[{\"slug\":\"test-hidden\",\"display_name\":\"Hidden\",\"visibility\":\"hide\",\"priority\":0,\"supported_reasoning_levels\":[]},{\"slug\":\"test-reasoner\",\"display_name\":\"Test Reasoner\",\"visibility\":\"list\",\"priority\":2,\"default_reasoning_level\":\"medium\",\"supported_reasoning_levels\":[{\"effort\":\"low\"},{\"effort\":\"medium\"},{\"effort\":\"xhigh\"}]},{\"slug\":\"test-default\",\"display_name\":\"Test Default\",\"visibility\":\"list\",\"priority\":1,\"default_reasoning_level\":\"low\",\"supported_reasoning_levels\":[{\"effort\":\"low\"},{\"effort\":\"high\"}]},{\"slug\":\"test-plain\",\"display_name\":\"Test Plain\",\"visibility\":\"list\",\"priority\":3,\"default_reasoning_level\":null,\"supported_reasoning_levels\":[]}]}";

    fn credential() -> SubscriptionCredential {
        let payload =
            base64url_encode("{\"https://api.openai.com/auth\":{\"chatgpt_account_id\":\"fake-account\"}}".as_bytes());
        SubscriptionCredential {
            access_token: format!("header.{payload}.signature"),
            refresh_token: "fake-refresh".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: i64::MAX,
            scopes: Vec::new(),
        }
    }

    fn json_response(body: &str) -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: Some("application/json".to_string()),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn codex_fields_determine_order_default_and_levels() {
        let entries = parse_codex_models(&parse_json(CODEX_LIST).unwrap()).unwrap();
        assert_eq!(
            entries.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(),
            vec!["test-default", "test-reasoner", "test-plain"]
        );
        assert!(entries[0].recommended);
        assert_eq!(entries[0].default_reasoning_effort.as_deref(), Some("low"));
        assert_eq!(entries[0].reasoning_source, ReasoningSource::Provider);
        assert_eq!(
            entries[1].reasoning_efforts,
            vec!["low".to_string(), "medium".to_string(), "xhigh".to_string()]
        );
        assert!(!entries[1].recommended);
    }

    #[test]
    fn api_capabilities_are_reference_only_and_other_gets_none() {
        let value = parse_json(API_LIST).unwrap();
        let api = parse_api_models(&value, true).unwrap();
        assert!(!api.iter().any(|entry| entry.id == "text-embedding-test"));
        let gpt5 = api.iter().find(|entry| entry.id == "gpt-5").unwrap();
        assert_eq!(gpt5.reasoning_source, ReasoningSource::Reference);
        assert_eq!(
            gpt5.reasoning_efforts,
            vec![
                "minimal".to_string(),
                "low".to_string(),
                "medium".to_string(),
                "high".to_string()
            ]
        );
        assert!(!gpt5.recommended);
        let unknown = api.iter().find(|entry| entry.id == "unknown-chat").unwrap();
        assert!(unknown.reasoning_efforts.is_empty());
        let other = parse_api_models(&value, false).unwrap();
        assert!(other
            .iter()
            .all(|entry| entry.reasoning_source == ReasoningSource::Unknown && entry.reasoning_efforts.is_empty()));
        assert!(other.iter().any(|entry| entry.id == "text-embedding-test"));
    }

    #[test]
    fn reference_table_ignores_lookalikes_but_strips_snapshot_dates() {
        assert_eq!(
            api_model("gpt-6-astra").reasoning_efforts,
            vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
                "xhigh".to_string(),
                "max".to_string()
            ]
        );
        assert_eq!(api_model("gpt-5.5-pro").reasoning_efforts.len(), 3);
        assert!(api_model("gpt-5-2025-08-07")
            .reasoning_efforts
            .contains(&"minimal".to_string()));
        assert_eq!(api_model("gpt-5-future").reasoning_source, ReasoningSource::Unknown);
        assert!(api_model("gpt-5-future").reasoning_efforts.is_empty());
    }

    #[test]
    fn reasoning_effort_validation_matches_donor() {
        assert_eq!(parse_reasoning_effort(&Json::Null).unwrap(), None);
        assert_eq!(
            parse_reasoning_effort(&Json::String("xhigh".to_string())).unwrap(),
            Some("xhigh".to_string())
        );
        for bad in ["High", "9lives", "has space", &"x".repeat(65), ""] {
            assert!(parse_reasoning_effort(&Json::String(bad.to_string())).is_err(), "{bad}");
        }
    }

    #[test]
    fn discovery_uses_documented_urls_and_auth() {
        let models = discover_api_models(
            "https://api.openai.com/v1/",
            "k",
            true,
            &|url: &str, init: &FetchInit| {
                assert_eq!(url, "https://api.openai.com/v1/models");
                assert_eq!(init.method, FetchMethod::Get);
                assert_eq!(init.header("authorization"), Some("Bearer k"));
                Ok(json_response(API_LIST))
            },
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(models.len(), 3);
        let models = discover_codex_models(
            &credential(),
            &|url: &str, init: &FetchInit| {
                assert_eq!(
                    url,
                    "https://chatgpt.com/backend-api/codex/models?client_version=0.154.0"
                );
                assert_eq!(init.header("chatgpt-account-id"), Some("fake-account"));
                Ok(json_response(CODEX_LIST))
            },
            &CancelToken::new(),
        )
        .unwrap();
        assert_eq!(models[0].id, "test-default");
    }

    #[test]
    fn model_bodies_reject_empty_oversized_invalid_and_http_failures() {
        assert_eq!(
            model_json(json_response(""), &CancelToken::new())
                .unwrap_err()
                .to_string(),
            "The service returned an empty model list response."
        );
        let big = LlmResponse {
            status: 200,
            content_type: Some("application/json".to_string()),
            body: vec![b'x'; MAX_MODEL_LIST_BYTES + 1],
        };
        assert_eq!(
            model_json(big, &CancelToken::new()).unwrap_err().to_string(),
            "The service model list exceeded the size limit."
        );
        assert_eq!(
            model_json(json_response("not json"), &CancelToken::new())
                .unwrap_err()
                .to_string(),
            "The service returned an invalid model list response."
        );
        let failed = LlmResponse {
            status: 401,
            content_type: None,
            body: b"secret".to_vec(),
        };
        assert_eq!(model_json(failed, &CancelToken::new()).unwrap_err().status(), Some(401));
    }
}
