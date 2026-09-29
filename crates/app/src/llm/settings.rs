//! LLM settings, credentials, catalogs, and request routing.
//!
//! Donor provenance: `src/llm/settings.ts` (`LlmSettingsService`,
//! `LlmProvider`, `OtherService`, `LlmSettingsSnapshot`). Same four files
//! (`llm.json`, `chatgpt.key`, `other.key`, `other.service`), same atomic
//! `0o600` writes, same validation messages, same catalog lifecycle
//! (loading keeps previous models, errors keep saved models, URL changes
//! invalidate), same request routing with one subscription refresh retry
//! on 401/403.
//!
//! The donor serializes concurrent async callers through a promise queue
//! with shared token refreshes; this synchronous port takes `&mut self`,
//! so the queue, the shared-refresh fan-out, and the abort races collapse
//! into direct calls. Timeouts survive through [`TimeoutFetch`], and
//! cancellation arrives as a [`CancelToken`] checked before each fetch,
//! each catalog commit, and each file write. There is no concurrent
//! sign-in to cancel and no shared refresh promise: every call completes
//! before the next begins.

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::settings::json::{parse_json, stringify, Json};

use super::api::{request_chat_completions, request_openai_responses};
use super::auth::{
    check_abort, login_subscription, now_ms, parse_subscription, random_uuid, record, refresh_subscription,
    SubscriptionAuthOptions, SubscriptionCredential,
};
use super::codex::request_codex;
use super::errors::LlmError;
use super::models::{
    api_model, discover_api_models, discover_codex_models, parse_reasoning_effort, LlmModel, LlmModelCatalog,
};
use super::request::{CancelToken, LlmFetch, LlmRequestInput, TimeoutFetch, TransportRequest};

/// Default request deadline (donor `runRequest` default).
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_millis(120_000);
/// Model discovery deadline cap (donor `refreshModels` cap).
pub const DISCOVERY_TIMEOUT_CAP: Duration = Duration::from_millis(30_000);

/// An LLM provider (donor `LlmProvider`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmProvider {
    /// ChatGPT subscription over Codex Responses.
    ChatGptSubscription,
    /// ChatGPT API key over OpenAI Responses.
    ChatGptApi,
    /// Another OpenAI-compatible chat service.
    OtherApi,
}

impl LlmProvider {
    /// Donor provider name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ChatGptSubscription => "chatgpt-subscription",
            Self::ChatGptApi => "chatgpt-api",
            Self::OtherApi => "other-api",
        }
    }

    /// Parse a donor provider name.
    fn parse(value: &Json) -> Result<Self, LlmError> {
        match value {
            Json::String(name) if name == "chatgpt-subscription" => Ok(Self::ChatGptSubscription),
            Json::String(name) if name == "chatgpt-api" => Ok(Self::ChatGptApi),
            Json::String(name) if name == "other-api" => Ok(Self::OtherApi),
            _ => Err(LlmError::settings("Invalid LLM provider setting.")),
        }
    }
}

/// The only Other-API transport (donor `OtherService` literal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherTransport {
    /// OpenAI chat completions over SSE.
    OpenAiChatCompletions,
}

impl OtherTransport {
    /// Wire name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::OpenAiChatCompletions => "openai-chat-completions",
        }
    }
}

/// Another OpenAI-compatible service (donor `OtherService`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtherService {
    /// Service base URL (no trailing slash).
    pub base_url: String,
    /// Selected model id (empty until chosen).
    pub model: String,
    /// Transport (always chat completions).
    pub transport: OtherTransport,
}

/// Subscription sign-in state (donor `SubscriptionAuthState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionAuthState {
    /// No sign-in running.
    Idle,
    /// Sign-in running.
    Pending,
    /// Last sign-in failed.
    Error {
        /// Failure message.
        message: String,
    },
}

/// One provider's snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSnapshot {
    /// Whether a credential is stored.
    pub configured: bool,
    /// Selected model id.
    pub model: String,
    /// Transport name.
    pub transport: &'static str,
    /// Subscription expiry, when a subscription is stored.
    pub expires_at: Option<i64>,
    /// Other-service base URL, when this is the Other provider.
    pub base_url: Option<String>,
}

/// Per-provider reasoning efforts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEfforts {
    /// Subscription effort.
    pub subscription: Option<String>,
    /// API effort.
    pub api: Option<String>,
    /// Other effort.
    pub other: Option<String>,
}

impl ProviderEfforts {
    /// Effort for one provider.
    #[must_use]
    pub const fn get(&self, provider: LlmProvider) -> Option<&String> {
        match provider {
            LlmProvider::ChatGptSubscription => self.subscription.as_ref(),
            LlmProvider::ChatGptApi => self.api.as_ref(),
            LlmProvider::OtherApi => self.other.as_ref(),
        }
    }

    fn set(&mut self, provider: LlmProvider, effort: Option<String>) {
        match provider {
            LlmProvider::ChatGptSubscription => self.subscription = effort,
            LlmProvider::ChatGptApi => self.api = effort,
            LlmProvider::OtherApi => self.other = effort,
        }
    }
}

/// Per-provider catalogs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogs {
    /// Subscription catalog.
    pub subscription: LlmModelCatalog,
    /// API catalog.
    pub api: LlmModelCatalog,
    /// Other catalog.
    pub other: LlmModelCatalog,
}

impl ProviderCatalogs {
    /// Catalog for one provider.
    #[must_use]
    pub const fn get(&self, provider: LlmProvider) -> &LlmModelCatalog {
        match provider {
            LlmProvider::ChatGptSubscription => &self.subscription,
            LlmProvider::ChatGptApi => &self.api,
            LlmProvider::OtherApi => &self.other,
        }
    }

    fn get_mut(&mut self, provider: LlmProvider) -> &mut LlmModelCatalog {
        match provider {
            LlmProvider::ChatGptSubscription => &mut self.subscription,
            LlmProvider::ChatGptApi => &mut self.api,
            LlmProvider::OtherApi => &mut self.other,
        }
    }
}

/// One unreadable settings file (donor `errors` entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsFileError {
    /// File name.
    pub file: String,
    /// Fixed load message.
    pub message: String,
}

/// Readable settings snapshot (donor `LlmSettingsSnapshot`; never secrets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmSettingsSnapshot {
    /// Selected provider.
    pub provider: LlmProvider,
    /// Selected provider's model.
    pub model: String,
    /// Selected provider's reasoning effort.
    pub reasoning_effort: Option<String>,
    /// Per-provider reasoning efforts.
    pub reasoning_efforts: ProviderEfforts,
    /// Per-provider catalogs.
    pub catalogs: ProviderCatalogs,
    /// Subscription provider state.
    pub subscription: ProviderSnapshot,
    /// API provider state.
    pub api: ProviderSnapshot,
    /// Other provider state.
    pub other: ProviderSnapshot,
    /// Sign-in state.
    pub subscription_auth: SubscriptionAuthState,
    /// Unreadable files.
    pub errors: Vec<SettingsFileError>,
}

/// Which API-key slot to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeySlot {
    /// ChatGPT API key (`chatgpt.key`).
    ChatGptApi,
    /// Other service key (`other.key`).
    OtherApi,
}

/// Request configuration (donor `LlmRequestOptions`).
#[derive(Clone, Default)]
pub struct LlmRequestOptions {
    /// Injectable fetch (no implicit network client exists).
    pub fetch: Option<Arc<dyn LlmFetch>>,
    /// Request deadline.
    pub timeout: Option<Duration>,
}

/// Settings service configuration (donor `LlmSettingsOptions`).
#[derive(Clone, Default)]
pub struct LlmSettingsOptions {
    /// Directory holding the four settings files.
    pub base_directory: PathBuf,
    /// Subscription auth overrides.
    pub auth: SubscriptionAuthOptions,
    /// Request overrides.
    pub request: LlmRequestOptions,
}

#[derive(Debug, Clone)]
struct Preferences {
    provider: LlmProvider,
    subscription_model: String,
    api_model: String,
    efforts: ProviderEfforts,
}

#[derive(Debug, Clone)]
struct ChatgptCredentials {
    api_key: Option<String>,
    subscription: Option<SubscriptionCredential>,
}

fn member<'a>(members: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    members.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

fn parse_document(text: &str) -> Result<Json, LlmError> {
    parse_json(text).map_err(|_| LlmError::settings("Invalid LLM settings file. Check its format before saving."))
}

fn validate_model(value: &Json) -> Result<String, LlmError> {
    match value {
        Json::String(text) if text.chars().count() <= 256 && !text.bytes().any(|byte| byte < 0x20 || byte == 0x7f) => {
            Ok(text.trim().to_string())
        }
        _ => Err(LlmError::settings("Invalid model name.")),
    }
}

fn validate_api_key(text: &str) -> Result<String, LlmError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || text.chars().count() > 16_384 || trimmed.chars().any(char::is_whitespace) {
        return Err(LlmError::settings("Paste a nonempty API key without whitespace."));
    }
    Ok(trimmed.to_string())
}

fn validate_api_key_json(value: &Json) -> Result<String, LlmError> {
    match value {
        Json::String(text) => validate_api_key(text),
        _ => Err(LlmError::settings("Paste a nonempty API key without whitespace.")),
    }
}

/// Split `scheme://rest`; rejects missing separators and empty hosts.
fn split_url(base: &str) -> Result<(String, String), LlmError> {
    let invalid = || LlmError::settings("Enter a valid Other API base URL.");
    let (scheme, rest) = base.split_once("://").ok_or_else(invalid)?;
    if scheme.is_empty() || rest.is_empty() {
        return Err(invalid());
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return Err(invalid());
    }
    Ok((scheme.to_ascii_lowercase(), rest.to_string()))
}

fn validate_other_service(value: &Json) -> Result<OtherService, LlmError> {
    let item = record(value)
        .map_err(|_| LlmError::settings("Other API requires an OpenAI Chat Completions compatible service."))?;
    if member(item, "transport") != Some(&Json::String("openai-chat-completions".to_string())) {
        return Err(LlmError::settings(
            "Other API requires an OpenAI Chat Completions compatible service.",
        ));
    }
    let base = match member(item, "baseUrl") {
        Some(Json::String(base)) => base.clone(),
        _ => {
            return Err(LlmError::settings(
                "Other API requires an OpenAI Chat Completions compatible service.",
            ));
        }
    };
    let (scheme, rest) = split_url(&base)?;
    let authority = rest.split('/').next().unwrap_or("");
    if authority.contains('@') || rest.contains('?') || rest.contains('#') {
        return Err(LlmError::settings(
            "Use HTTPS or loopback HTTP, without URL credentials, query, or fragment.",
        ));
    }
    let host = if let Some(address) = authority.strip_prefix('[') {
        match address.find(']') {
            Some(end) => format!("[{}]", &address[..end]),
            None => return Err(LlmError::settings("Enter a valid Other API base URL.")),
        }
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    let loopback = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
    if scheme != "https" && !(scheme == "http" && loopback) {
        return Err(LlmError::settings(
            "Use HTTPS or loopback HTTP, without URL credentials, query, or fragment.",
        ));
    }
    Ok(OtherService {
        base_url: base.strip_suffix('/').unwrap_or(&base).to_string(),
        model: validate_model(member(item, "model").unwrap_or(&Json::Null))?,
        transport: OtherTransport::OpenAiChatCompletions,
    })
}

fn parse_chatgpt(text: Option<&str>) -> Result<ChatgptCredentials, LlmError> {
    let Some(text) = text else {
        return Ok(ChatgptCredentials {
            api_key: None,
            subscription: None,
        });
    };
    if !text.trim_start().starts_with('{') {
        return Ok(ChatgptCredentials {
            api_key: Some(validate_api_key(text)?),
            subscription: None,
        });
    }
    let value = parse_document(text)?;
    let item = record(&value)?;
    if member(item, "version") != Some(&Json::Number(1.0)) {
        return Err(LlmError::settings("Unsupported chatgpt.key format."));
    }
    let api_key = match member(item, "apiKey") {
        None | Some(Json::Null) => None,
        Some(key) => Some(validate_api_key_json(key)?),
    };
    let subscription = match member(item, "subscription") {
        None | Some(Json::Null) => None,
        Some(stored) => Some(parse_subscription(stored)?),
    };
    Ok(ChatgptCredentials { api_key, subscription })
}

fn default_preferences() -> Preferences {
    Preferences {
        provider: LlmProvider::ChatGptSubscription,
        subscription_model: String::new(),
        api_model: String::new(),
        efforts: ProviderEfforts {
            subscription: None,
            api: None,
            other: None,
        },
    }
}

fn parse_preferences(text: Option<&str>) -> Result<Preferences, LlmError> {
    let Some(text) = text else {
        return Ok(default_preferences());
    };
    let value = parse_document(text)?;
    let item = record(&value)?;
    if member(item, "version") != Some(&Json::Number(1.0)) {
        return Err(LlmError::settings("Unsupported llm.json format."));
    }
    let models = record(member(item, "models").unwrap_or(&Json::Null))?;
    let efforts = member(item, "reasoningEfforts").unwrap_or(&Json::Null);
    let efforts = if matches!(efforts, Json::Null) {
        Vec::new()
    } else {
        record(efforts)?.clone()
    };
    let effort = |provider: &str| parse_reasoning_effort(member(&efforts, provider).unwrap_or(&Json::Null));
    Ok(Preferences {
        provider: LlmProvider::parse(member(item, "provider").unwrap_or(&Json::Null))?,
        subscription_model: validate_model(member(models, "chatgpt-subscription").unwrap_or(&Json::Null))?,
        api_model: validate_model(member(models, "chatgpt-api").unwrap_or(&Json::Null))?,
        efforts: ProviderEfforts {
            subscription: effort("chatgpt-subscription")?,
            api: effort("chatgpt-api")?,
            other: effort("other-api")?,
        },
    })
}

fn credential_json(credential: &SubscriptionCredential) -> Json {
    Json::Object(vec![
        ("accessToken".to_string(), Json::String(credential.access_token.clone())),
        (
            "refreshToken".to_string(),
            Json::String(credential.refresh_token.clone()),
        ),
        ("tokenType".to_string(), Json::String(credential.token_type.clone())),
        ("expiresAt".to_string(), Json::Number(credential.expires_at as f64)),
        (
            "scopes".to_string(),
            Json::Array(credential.scopes.iter().cloned().map(Json::String).collect()),
        ),
    ])
}

/// Synchronous LLM settings service (donor `LlmSettingsService`).
pub struct LlmSettingsService {
    base: PathBuf,
    auth: SubscriptionAuthOptions,
    request: LlmRequestOptions,
    preferences: Preferences,
    chatgpt: ChatgptCredentials,
    other_key: Option<String>,
    other: OtherService,
    auth_state: SubscriptionAuthState,
    closed: bool,
    errors: Vec<(String, String)>,
    catalogs: ProviderCatalogs,
    generations: HashMap<LlmProvider, u64>,
}

impl LlmSettingsService {
    /// Open the service, recording (never throwing) per-file load failures.
    #[must_use]
    pub fn open(options: &LlmSettingsOptions) -> Self {
        let mut service = Self {
            base: options.base_directory.clone(),
            auth: options.auth.clone(),
            request: options.request.clone(),
            preferences: default_preferences(),
            chatgpt: ChatgptCredentials {
                api_key: None,
                subscription: None,
            },
            other_key: None,
            other: OtherService {
                base_url: String::new(),
                model: String::new(),
                transport: OtherTransport::OpenAiChatCompletions,
            },
            auth_state: SubscriptionAuthState::Idle,
            closed: false,
            errors: Vec::new(),
            catalogs: ProviderCatalogs {
                subscription: LlmModelCatalog {
                    status: super::models::CatalogStatus::Idle,
                    models: Vec::new(),
                    message: None,
                },
                api: LlmModelCatalog {
                    status: super::models::CatalogStatus::Idle,
                    models: Vec::new(),
                    message: None,
                },
                other: LlmModelCatalog {
                    status: super::models::CatalogStatus::Idle,
                    models: Vec::new(),
                    message: None,
                },
            },
            generations: HashMap::new(),
        };
        for file in ["llm.json", "chatgpt.key", "other.key", "other.service"] {
            let loaded: Result<(), LlmError> = (|| {
                let text = service.load(file)?;
                match file {
                    "llm.json" => service.preferences = parse_preferences(text.as_deref())?,
                    "chatgpt.key" => service.chatgpt = parse_chatgpt(text.as_deref())?,
                    "other.key" => service.other_key = text.as_ref().map(|key| validate_api_key(key)).transpose()?,
                    "other.service" => {
                        if let Some(text) = text {
                            service.other = validate_other_service(&parse_document(&text)?)?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            })();
            if loaded.is_err() {
                service.errors_set(
                    file,
                    format!("Could not load {file}. Check its format and permissions."),
                );
            }
        }
        service
    }

    /// Current snapshot (never contains secrets).
    #[must_use]
    pub fn read(&self) -> LlmSettingsSnapshot {
        let subscription = ProviderSnapshot {
            configured: self.chatgpt.subscription.is_some(),
            model: self.preferences.subscription_model.clone(),
            transport: "openai-codex-responses-sse",
            expires_at: self
                .chatgpt
                .subscription
                .as_ref()
                .map(|credential| credential.expires_at),
            base_url: None,
        };
        let api = ProviderSnapshot {
            configured: self.chatgpt.api_key.is_some(),
            model: self.preferences.api_model.clone(),
            transport: "openai-responses-sse",
            expires_at: None,
            base_url: None,
        };
        let other = ProviderSnapshot {
            configured: self.other_key.is_some(),
            model: self.other.model.clone(),
            transport: "openai-chat-completions",
            expires_at: None,
            base_url: Some(self.other.base_url.clone()),
        };
        let (model, reasoning_effort) = match self.preferences.provider {
            LlmProvider::ChatGptSubscription => (
                subscription.model.clone(),
                self.preferences.efforts.subscription.clone(),
            ),
            LlmProvider::ChatGptApi => (api.model.clone(), self.preferences.efforts.api.clone()),
            LlmProvider::OtherApi => (other.model.clone(), self.preferences.efforts.other.clone()),
        };
        LlmSettingsSnapshot {
            provider: self.preferences.provider,
            model,
            reasoning_effort,
            reasoning_efforts: self.preferences.efforts.clone(),
            catalogs: self.catalogs.clone(),
            subscription,
            api,
            other,
            subscription_auth: self.auth_state.clone(),
            errors: self
                .errors
                .iter()
                .map(|(file, message)| SettingsFileError {
                    file: file.clone(),
                    message: message.clone(),
                })
                .collect(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }

    fn load(&self, name: &str) -> Result<Option<String>, LlmError> {
        match std::fs::read_to_string(self.path(name)) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(LlmError::settings(format!("Could not read {name}."))),
        }
    }

    fn write(&mut self, name: &str, text: &str) -> Result<(), LlmError> {
        let target = self.path(name);
        let temporary = target.with_extension(format!("{}.tmp", random_uuid()));
        let failed = || LlmError::settings(format!("Could not save {name}."));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|_| failed())?;
        }
        (|| -> std::io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary)?;
            use std::io::Write;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
            }
            std::fs::rename(&temporary, &target)?;
            Ok(())
        })()
        .map_err(|_| failed())?;
        let _ = std::fs::remove_file(&temporary);
        self.errors_remove(name);
        Ok(())
    }

    fn errors_set(&mut self, file: &str, message: String) {
        if let Some(existing) = self.errors.iter_mut().find(|(name, _)| name == file) {
            existing.1 = message;
        } else {
            self.errors.push((file.to_string(), message));
        }
    }

    fn errors_remove(&mut self, file: &str) {
        self.errors.retain(|(name, _)| name != file);
    }

    fn check_closed(&self) -> Result<(), LlmError> {
        if self.closed {
            return Err(LlmError::settings("LLM settings are closed."));
        }
        Ok(())
    }

    fn check_cancel(cancel: &CancelToken) -> Result<(), LlmError> {
        if cancel.is_cancelled() {
            return Err(LlmError::settings("LLM request cancelled."));
        }
        Ok(())
    }

    fn fetcher(&self, timeout: Duration) -> Result<TimeoutFetch, LlmError> {
        self.check_closed()?;
        match &self.request.fetch {
            Some(fetch) => Ok(TimeoutFetch::new(Arc::clone(fetch), timeout)),
            None => Err(LlmError::settings(
                "Could not reach the LLM service. Check its URL and connection.",
            )),
        }
    }

    fn request_timeout(&self) -> Duration {
        self.request.timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT)
    }

    fn invalidate_catalog(&mut self, provider: LlmProvider) {
        let generation = self.generations.get(&provider).copied().unwrap_or(0) + 1;
        self.generations.insert(provider, generation);
        *self.catalogs.get_mut(provider) = LlmModelCatalog {
            status: super::models::CatalogStatus::Idle,
            models: Vec::new(),
            message: None,
        };
    }

    fn model_metadata(&self, provider: LlmProvider, name: &str) -> Option<LlmModel> {
        if let Some(known) = self.catalogs.get(provider).models.iter().find(|model| model.id == name) {
            return Some(known.clone());
        }
        if provider == LlmProvider::ChatGptApi {
            return Some(api_model(name));
        }
        None
    }

    fn provider_model(&self, preferences: &Preferences, provider: LlmProvider) -> String {
        match provider {
            LlmProvider::ChatGptSubscription => preferences.subscription_model.clone(),
            LlmProvider::ChatGptApi => preferences.api_model.clone(),
            LlmProvider::OtherApi => self.other.model.clone(),
        }
    }

    fn preferences_json(preferences: &Preferences) -> String {
        let effort = |effort: &Option<String>| match effort {
            Some(effort) => Json::String(effort.clone()),
            None => Json::Null,
        };
        stringify(&Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            (
                "provider".to_string(),
                Json::String(preferences.provider.name().to_string()),
            ),
            (
                "models".to_string(),
                Json::Object(vec![
                    (
                        "chatgpt-subscription".to_string(),
                        Json::String(preferences.subscription_model.clone()),
                    ),
                    ("chatgpt-api".to_string(), Json::String(preferences.api_model.clone())),
                ]),
            ),
            (
                "reasoningEfforts".to_string(),
                Json::Object(vec![
                    (
                        "chatgpt-subscription".to_string(),
                        effort(&preferences.efforts.subscription),
                    ),
                    ("chatgpt-api".to_string(), effort(&preferences.efforts.api)),
                    ("other-api".to_string(), effort(&preferences.efforts.other)),
                ]),
            ),
        ])) + "\n"
    }

    fn chatgpt_json(credentials: &ChatgptCredentials) -> String {
        stringify(&Json::Object(vec![
            ("version".to_string(), Json::Number(1.0)),
            (
                "apiKey".to_string(),
                credentials.api_key.clone().map(Json::String).unwrap_or(Json::Null),
            ),
            (
                "subscription".to_string(),
                credentials
                    .subscription
                    .as_ref()
                    .map(credential_json)
                    .unwrap_or(Json::Null),
            ),
        ])) + "\n"
    }

    fn other_service_json(service: &OtherService) -> String {
        stringify(&Json::Object(vec![
            ("baseUrl".to_string(), Json::String(service.base_url.clone())),
            ("model".to_string(), Json::String(service.model.clone())),
            (
                "transport".to_string(),
                Json::String(service.transport.name().to_string()),
            ),
        ])) + "\n"
    }

    /// Select the active provider.
    pub fn select_provider(&mut self, provider: LlmProvider) -> Result<(), LlmError> {
        self.check_closed()?;
        self.auth_state = SubscriptionAuthState::Idle;
        let mut current = parse_preferences(self.load("llm.json")?.as_deref())?;
        current.provider = provider;
        self.write("llm.json", &Self::preferences_json(&current))?;
        self.preferences = current;
        Ok(())
    }

    /// Set one provider's model, resetting unsupported efforts to the default.
    pub fn set_model(&mut self, provider: LlmProvider, value: &str) -> Result<(), LlmError> {
        self.check_closed()?;
        let name = validate_model(&Json::String(value.to_string()))?;
        let current = parse_preferences(self.load("llm.json")?.as_deref())?;
        if self.catalogs.get(provider).status == super::models::CatalogStatus::Ready
            && !self.catalogs.get(provider).models.iter().any(|model| model.id == name)
        {
            return Err(LlmError::settings("Choose a model from the loaded provider list."));
        }
        let metadata = self.model_metadata(provider, &name);
        let effort = current.efforts.get(provider).cloned();
        let next_effort = match (&effort, &metadata) {
            (Some(effort), Some(metadata)) if metadata.reasoning_efforts.contains(effort) => Some(effort.clone()),
            (_, Some(metadata)) => metadata.default_reasoning_effort.clone(),
            (_, None) => None,
        };
        let mut next = current;
        if provider == LlmProvider::ChatGptSubscription {
            next.subscription_model = name.clone();
        } else if provider == LlmProvider::ChatGptApi {
            next.api_model = name.clone();
        }
        next.efforts.set(provider, next_effort);
        if provider == LlmProvider::OtherApi {
            let text = self.load("other.service")?;
            let base = match text {
                Some(text) => validate_other_service(&parse_document(&text)?)?,
                None => self.other.clone(),
            };
            let merged = validate_other_service(&Json::Object(vec![
                ("baseUrl".to_string(), Json::String(base.base_url.clone())),
                ("model".to_string(), Json::String(name)),
                ("transport".to_string(), Json::String(base.transport.name().to_string())),
            ]))?;
            self.write("other.service", &Self::other_service_json(&merged))?;
            self.other = merged;
        }
        self.write("llm.json", &Self::preferences_json(&next))?;
        self.preferences = next;
        Ok(())
    }

    /// Set one provider's reasoning effort (`None` selects Model default).
    pub fn set_reasoning_effort(&mut self, provider: LlmProvider, value: Option<&str>) -> Result<(), LlmError> {
        self.check_closed()?;
        let effort = parse_reasoning_effort(&value.map(str::to_string).map(Json::String).unwrap_or(Json::Null))?;
        let current = parse_preferences(self.load("llm.json")?.as_deref())?;
        let name = self.provider_model(&current, provider);
        if let Some(effort) = &effort {
            let supported = self
                .model_metadata(provider, &name)
                .is_some_and(|metadata| metadata.reasoning_efforts.contains(effort));
            if !supported {
                return Err(LlmError::settings(
                    "Choose a reasoning effort supported by the selected model.",
                ));
            }
        }
        let mut next = current;
        next.efforts.set(provider, effort);
        self.write("llm.json", &Self::preferences_json(&next))?;
        self.preferences = next;
        Ok(())
    }

    /// Save the Other-API service, invalidating its catalog on URL changes.
    pub fn save_other_service(&mut self, value: &OtherService) -> Result<(), LlmError> {
        self.check_closed()?;
        let next = validate_other_service(&Json::Object(vec![
            ("baseUrl".to_string(), Json::String(value.base_url.clone())),
            ("model".to_string(), Json::String(value.model.clone())),
            (
                "transport".to_string(),
                Json::String(value.transport.name().to_string()),
            ),
        ]))?;
        let connection_changed = next.base_url != self.other.base_url;
        let model_changed = next.model != self.other.model;
        if !connection_changed
            && model_changed
            && self.catalogs.get(LlmProvider::OtherApi).status == super::models::CatalogStatus::Ready
            && !self
                .catalogs
                .get(LlmProvider::OtherApi)
                .models
                .iter()
                .any(|model| model.id == next.model)
        {
            return Err(LlmError::settings("Choose a model from the loaded provider list."));
        }
        self.write("other.service", &Self::other_service_json(&next))?;
        self.other = next.clone();
        if connection_changed {
            self.invalidate_catalog(LlmProvider::OtherApi);
        }
        if connection_changed || model_changed {
            let current = parse_preferences(self.load("llm.json")?.as_deref())?;
            let metadata = self.model_metadata(LlmProvider::OtherApi, &next.model);
            let previous = current.efforts.other.clone();
            let effort = match (&previous, &metadata) {
                (Some(previous), Some(metadata)) if metadata.reasoning_efforts.contains(previous) => {
                    Some(previous.clone())
                }
                (_, Some(metadata)) => metadata.default_reasoning_effort.clone(),
                (_, None) => None,
            };
            let mut preferences = current;
            preferences.efforts.other = effort;
            self.write("llm.json", &Self::preferences_json(&preferences))?;
            self.preferences = preferences;
        }
        Ok(())
    }

    /// Save an API key, invalidating the provider catalog.
    pub fn save_api_key(&mut self, slot: ApiKeySlot, value: &str) -> Result<(), LlmError> {
        self.check_closed()?;
        let key = validate_api_key(value)?;
        match slot {
            ApiKeySlot::OtherApi => {
                self.write("other.key", &(key.clone() + "\n"))?;
                self.other_key = Some(key);
                self.invalidate_catalog(LlmProvider::OtherApi);
            }
            ApiKeySlot::ChatGptApi => {
                let mut current = parse_chatgpt(self.load("chatgpt.key")?.as_deref())?;
                current.api_key = Some(key);
                self.write("chatgpt.key", &Self::chatgpt_json(&current))?;
                self.chatgpt = current;
                self.invalidate_catalog(LlmProvider::ChatGptApi);
            }
        }
        Ok(())
    }

    /// Remove one provider's credential, invalidating its catalog.
    pub fn remove_credential(&mut self, provider: LlmProvider) -> Result<(), LlmError> {
        self.check_closed()?;
        if provider == LlmProvider::ChatGptSubscription {
            self.auth_state = SubscriptionAuthState::Idle;
        }
        match provider {
            LlmProvider::OtherApi => {
                std::fs::remove_file(self.path("other.key")).or_else(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        Ok(())
                    } else {
                        Err(LlmError::settings("Could not remove other.key."))
                    }
                })?;
                self.other_key = None;
                self.errors_remove("other.key");
            }
            LlmProvider::ChatGptApi | LlmProvider::ChatGptSubscription => {
                let mut current = parse_chatgpt(self.load("chatgpt.key")?.as_deref())?;
                if provider == LlmProvider::ChatGptApi {
                    current.api_key = None;
                } else {
                    current.subscription = None;
                }
                self.write("chatgpt.key", &Self::chatgpt_json(&current))?;
                self.chatgpt = current;
            }
        }
        self.invalidate_catalog(provider);
        Ok(())
    }

    /// Refresh one provider's catalog, defaulting to the selected provider.
    pub fn refresh_models(
        &mut self,
        provider: Option<LlmProvider>,
        cancel: &CancelToken,
    ) -> Result<Vec<LlmModel>, LlmError> {
        self.check_closed()?;
        Self::check_cancel(cancel)?;
        let provider = provider.unwrap_or(self.preferences.provider);
        let generation = self.generations.get(&provider).copied().unwrap_or(0) + 1;
        self.generations.insert(provider, generation);
        let previous = self.catalogs.get(provider).models.clone();
        *self.catalogs.get_mut(provider) = LlmModelCatalog {
            status: super::models::CatalogStatus::Loading,
            models: previous.clone(),
            message: None,
        };
        let timeout = self.request_timeout().min(DISCOVERY_TIMEOUT_CAP);
        let discovered = self.discover_models(provider, cancel, timeout);
        match discovered {
            Ok(models) => {
                Self::check_cancel(cancel)?;
                if self.generations.get(&provider).copied() != Some(generation) {
                    return Ok(models);
                }
                *self.catalogs.get_mut(provider) = LlmModelCatalog {
                    status: super::models::CatalogStatus::Ready,
                    models: models.clone(),
                    message: None,
                };
                Self::check_cancel(cancel)?;
                if self.generations.get(&provider).copied() != Some(generation) {
                    return Ok(self.catalogs.get(provider).models.clone());
                }
                self.adopt_discovered_defaults(provider, &models, cancel)?;
                Ok(self.catalogs.get(provider).models.clone())
            }
            Err(error) => {
                if self.generations.get(&provider).copied() == Some(generation) {
                    *self.catalogs.get_mut(provider) = if cancel.is_cancelled() || self.closed {
                        LlmModelCatalog {
                            status: super::models::CatalogStatus::Idle,
                            models: previous,
                            message: None,
                        }
                    } else {
                        LlmModelCatalog {
                            status: super::models::CatalogStatus::Error,
                            models: previous,
                            message: Some(error.to_string()),
                        }
                    };
                }
                Err(error)
            }
        }
    }

    fn discover_models(
        &mut self,
        provider: LlmProvider,
        cancel: &CancelToken,
        timeout: Duration,
    ) -> Result<Vec<LlmModel>, LlmError> {
        let fetcher = self.fetcher(timeout)?;
        if provider == LlmProvider::ChatGptSubscription {
            let credential = self.fresh_subscription(cancel, None)?;
            match discover_codex_models(&credential, &fetcher, cancel) {
                Ok(models) => Ok(models),
                Err(error) => match error.status() {
                    Some(401 | 403) => {
                        let refreshed = self.fresh_subscription(cancel, Some(credential.access_token.as_str()))?;
                        discover_codex_models(&refreshed, &fetcher, cancel)
                    }
                    _ => Err(error),
                },
            }
        } else {
            let key = if provider == LlmProvider::ChatGptApi {
                parse_chatgpt(self.load("chatgpt.key")?.as_deref())?.api_key
            } else {
                self.load("other.key")?
            };
            let Some(key) = key else {
                return Err(LlmError::settings("Paste an API key before loading models."));
            };
            let other = if provider == LlmProvider::OtherApi {
                let text = self.load("other.service")?.unwrap_or_else(|| "{}".to_string());
                Some(validate_other_service(&parse_document(&text)?)?)
            } else {
                None
            };
            discover_api_models(
                other
                    .as_ref()
                    .map_or("https://api.openai.com/v1", |service| service.base_url.as_str()),
                &validate_api_key(&key)?,
                provider == LlmProvider::ChatGptApi,
                &fetcher,
                cancel,
            )
        }
    }

    /// Persist a recommended default model and repair the effort after discovery.
    fn adopt_discovered_defaults(
        &mut self,
        provider: LlmProvider,
        models: &[LlmModel],
        cancel: &CancelToken,
    ) -> Result<(), LlmError> {
        let current = parse_preferences(self.load("llm.json")?.as_deref())?;
        let name = self.provider_model(&current, provider);
        let selected = models.iter().find(|model| model.id == name).or_else(|| {
            if name.is_empty() {
                models.iter().find(|model| model.recommended)
            } else {
                None
            }
        });
        let default = if name.is_empty() { selected } else { None };
        let old_effort = current.efforts.get(provider).cloned();
        let effort = default
            .and_then(|model| model.default_reasoning_effort.clone())
            .or_else(|| match &old_effort {
                Some(old) if !selected.is_some_and(|model| model.reasoning_efforts.contains(old)) => None,
                _ => old_effort.clone(),
            });
        if default.is_none() && effort == old_effort {
            return Ok(());
        }
        Self::check_cancel(cancel)?;
        let mut next = current;
        if let Some(default) = default {
            match provider {
                LlmProvider::ChatGptSubscription => next.subscription_model.clone_from(&default.id),
                LlmProvider::ChatGptApi => next.api_model.clone_from(&default.id),
                LlmProvider::OtherApi => {
                    let other = OtherService {
                        base_url: self.other.base_url.clone(),
                        model: default.id.clone(),
                        transport: OtherTransport::OpenAiChatCompletions,
                    };
                    self.write("other.service", &Self::other_service_json(&other))?;
                    self.other = other;
                }
            }
        }
        next.efforts.set(provider, effort);
        self.write("llm.json", &Self::preferences_json(&next))?;
        self.preferences = next;
        Ok(())
    }

    /// Sign in to ChatGPT Subscription over loopback OAuth.
    ///
    /// Cancellation resolves quietly with an idle auth state, matching the
    /// donor's disposed promise; other failures record the error state.
    pub fn sign_in_subscription(&mut self, cancel: &CancelToken) -> Result<(), LlmError> {
        self.check_closed()?;
        Self::check_cancel(cancel).map_err(|_| super::auth::cancelled())?;
        self.auth_state = SubscriptionAuthState::Pending;
        let credential = match login_subscription(&self.auth, cancel) {
            Ok(credential) => credential,
            Err(error) if error == super::auth::cancelled() || cancel.is_cancelled() => {
                self.auth_state = SubscriptionAuthState::Idle;
                return Ok(());
            }
            Err(error) => {
                self.auth_state = SubscriptionAuthState::Error {
                    message: error.to_string(),
                };
                return Err(error);
            }
        };
        Self::check_cancel(cancel).map_err(|_| super::auth::cancelled())?;
        if cancel.is_cancelled() {
            self.auth_state = SubscriptionAuthState::Idle;
            return Ok(());
        }
        let mut current = parse_chatgpt(self.load("chatgpt.key")?.as_deref())?;
        current.subscription = Some(credential);
        self.write("chatgpt.key", &Self::chatgpt_json(&current))?;
        self.chatgpt = current;
        self.invalidate_catalog(LlmProvider::ChatGptSubscription);
        self.auth_state = SubscriptionAuthState::Idle;
        Ok(())
    }

    /// Refresh the subscription credential immediately.
    pub fn refresh_subscription(&mut self, cancel: &CancelToken) -> Result<(), LlmError> {
        self.check_closed()?;
        Self::check_cancel(cancel)?;
        let current = self.load_subscription()?;
        self.fresh_subscription(cancel, Some(current.access_token.as_str()))?;
        Ok(())
    }

    fn load_subscription(&self) -> Result<SubscriptionCredential, LlmError> {
        match parse_chatgpt(self.load("chatgpt.key")?.as_deref())?.subscription {
            Some(credential) => Ok(credential),
            None => Err(LlmError::settings(
                "Sign in to ChatGPT Subscription in LLM options first.",
            )),
        }
    }

    /// A usable subscription credential, refreshing near-expiry or rejected tokens.
    fn fresh_subscription(
        &mut self,
        cancel: &CancelToken,
        rejected_access_token: Option<&str>,
    ) -> Result<SubscriptionCredential, LlmError> {
        Self::check_cancel(cancel)?;
        let credential = self.load_subscription()?;
        match rejected_access_token {
            Some(rejected) if credential.access_token == rejected => {}
            Some(_) => return Ok(credential),
            None if credential.expires_at > now_ms() + 60_000 => return Ok(credential),
            None => {}
        }
        check_abort(cancel)?;
        let before = self.load_subscription()?;
        match rejected_access_token {
            Some(rejected) if before.access_token == rejected => {}
            Some(_) => return Ok(before),
            None if before.expires_at > now_ms() + 60_000 => return Ok(before),
            None => {}
        }
        let refreshed = refresh_subscription(&self.auth, &before, cancel)?;
        Self::check_cancel(cancel)?;
        let latest = parse_chatgpt(self.load("chatgpt.key")?.as_deref())?;
        let Some(current) = latest.subscription else {
            return Err(LlmError::settings(
                "Subscription credential was removed. Sign in again.",
            ));
        };
        if current.refresh_token != before.refresh_token || current.access_token != before.access_token {
            return Ok(current);
        }
        let next = ChatgptCredentials {
            api_key: latest.api_key,
            subscription: Some(refreshed.clone()),
        };
        self.write("chatgpt.key", &Self::chatgpt_json(&next))?;
        self.chatgpt = next;
        Ok(refreshed)
    }

    /// Send one request through the selected provider.
    pub fn request(&mut self, input: LlmRequestInput<'_>, cancel: &CancelToken) -> Result<String, LlmError> {
        self.check_closed()?;
        Self::check_cancel(cancel)?;
        let preferences = parse_preferences(self.load("llm.json")?.as_deref())?;
        if input.prompt.trim().is_empty() {
            return Err(LlmError::settings("Enter a question or command request."));
        }
        let timeout = self.request_timeout();
        let fetcher = self.fetcher(timeout)?;
        let provider = preferences.provider;
        let other = if provider == LlmProvider::OtherApi {
            let text = self.load("other.service")?.unwrap_or_else(|| "{}".to_string());
            Some(validate_other_service(&parse_document(&text)?)?)
        } else {
            None
        };
        let model = match provider {
            LlmProvider::ChatGptSubscription => preferences.subscription_model.clone(),
            LlmProvider::ChatGptApi => preferences.api_model.clone(),
            LlmProvider::OtherApi => other.as_ref().map_or_else(String::new, |service| service.model.clone()),
        };
        if model.is_empty() {
            return Err(LlmError::settings(
                "Choose a Model in LLM options before sending a request.",
            ));
        }
        let selected_effort = preferences.efforts.get(provider).cloned();
        if selected_effort.is_some() && self.model_metadata(provider, &model).is_none() {
            self.refresh_models(Some(provider), cancel)?;
        }
        let metadata = self.model_metadata(provider, &model);
        if let Some(effort) = &selected_effort {
            let supported = metadata
                .as_ref()
                .is_some_and(|metadata| metadata.reasoning_efforts.contains(effort));
            if !supported {
                return Err(LlmError::settings(
                    "The selected reasoning effort is not supported by this model. Choose a supported effort or Model default in LLM options.",
                ));
            }
        }
        let mut request = TransportRequest {
            prompt: input.prompt,
            instructions: input.instructions,
            model: &model,
            reasoning_effort: selected_effort.as_deref(),
            cancel: cancel.clone(),
            on_text: input.on_text,
        };
        if provider == LlmProvider::ChatGptSubscription {
            let credential = self.fresh_subscription(cancel, None)?;
            match request_codex(&mut request, &credential, &fetcher) {
                Ok(answer) => Ok(answer),
                Err(error) => match error.status() {
                    Some(401 | 403) => {
                        let refreshed = self.fresh_subscription(cancel, Some(credential.access_token.as_str()))?;
                        request_codex(&mut request, &refreshed, &fetcher)
                    }
                    _ => Err(error),
                },
            }
        } else {
            let key = if provider == LlmProvider::ChatGptApi {
                parse_chatgpt(self.load("chatgpt.key")?.as_deref())?.api_key
            } else {
                self.load("other.key")?
            };
            let Some(key) = key else {
                return Err(LlmError::settings(
                    "Paste an API key for the selected provider in LLM options first.",
                ));
            };
            if provider == LlmProvider::ChatGptApi {
                return request_openai_responses(&mut request, &validate_api_key(&key)?, &fetcher);
            }
            let Some(other) = other else {
                return Err(LlmError::settings("Configure Other API before sending a request."));
            };
            request_chat_completions(&mut request, &validate_api_key(&key)?, &other.base_url, &fetcher)
        }
    }

    /// Close the service; further mutations and requests fail.
    pub fn close(&mut self) {
        self.closed = true;
        self.auth_state = SubscriptionAuthState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::auth::base64url_encode;
    use super::super::models::CatalogStatus;
    use super::super::request::{FetchInit, FetchMethod, LlmResponse};
    use super::*;

    const CODEX_LIST: &str = "{\"models\":[{\"slug\":\"test-reasoner\",\"display_name\":\"Test Reasoner\",\"visibility\":\"list\",\"priority\":2,\"default_reasoning_level\":\"medium\",\"supported_reasoning_levels\":[{\"effort\":\"low\"},{\"effort\":\"medium\"},{\"effort\":\"xhigh\"}]},{\"slug\":\"test-default\",\"display_name\":\"Test Default\",\"visibility\":\"list\",\"priority\":1,\"default_reasoning_level\":\"low\",\"supported_reasoning_levels\":[{\"effort\":\"low\"},{\"effort\":\"high\"}]}]}";

    fn scratch() -> PathBuf {
        let directory = std::env::temp_dir().join(format!("qa-llm-{}-{}", std::process::id(), random_uuid()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn service(directory: &Path) -> LlmSettingsService {
        LlmSettingsService::open(&LlmSettingsOptions {
            base_directory: directory.to_path_buf(),
            ..LlmSettingsOptions::default()
        })
    }

    fn service_with_fetch(directory: &Path, fetch: Arc<dyn LlmFetch>) -> LlmSettingsService {
        LlmSettingsService::open(&LlmSettingsOptions {
            base_directory: directory.to_path_buf(),
            request: LlmRequestOptions {
                fetch: Some(fetch),
                timeout: None,
            },
            ..LlmSettingsOptions::default()
        })
    }

    fn cleanup(directory: &Path) {
        let _ = std::fs::remove_dir_all(directory);
    }

    fn subscription_token(label: &str) -> String {
        let payload =
            base64url_encode("{\"https://api.openai.com/auth\":{\"chatgpt_account_id\":\"test-account\"}}".as_bytes());
        format!("{label}.{payload}.signature")
    }

    fn seed_subscription(directory: &Path, expires_at: i64) {
        let body = format!(
            "{{\"version\":1,\"apiKey\":\"preserved-api\",\"subscription\":{{\"accessToken\":\"{}\",\"refreshToken\":\"old-refresh\",\"tokenType\":\"Bearer\",\"expiresAt\":{expires_at},\"scopes\":[\"openid\"]}}}}",
            subscription_token("old")
        );
        std::fs::write(directory.join("chatgpt.key"), body).unwrap();
    }

    fn sse(body: &str) -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: Some("text/event-stream".to_string()),
            body: body.as_bytes().to_vec(),
        }
    }

    fn codex_answer() -> LlmResponse {
        sse("data: {\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n")
    }

    fn refreshed_tokens() -> LlmResponse {
        LlmResponse {
            status: 200,
            content_type: Some("application/json".to_string()),
            body: format!(
                "{{\"access_token\":\"{}\",\"refresh_token\":\"new-refresh\",\"token_type\":\"Bearer\",\"expires_in\":3600}}",
                subscription_token("new")
            )
            .into_bytes(),
        }
    }

    #[test]
    fn stores_credentials_models_and_service_without_leaking_secrets() {
        let directory = scratch();
        let mut settings = service(&directory);
        settings.save_api_key(ApiKeySlot::ChatGptApi, " api-test ").unwrap();
        settings.save_api_key(ApiKeySlot::OtherApi, "other-test").unwrap();
        settings
            .save_other_service(&OtherService {
                base_url: "http://127.0.0.1:11434/v1/".to_string(),
                model: "local-model".to_string(),
                transport: OtherTransport::OpenAiChatCompletions,
            })
            .unwrap();
        settings.set_model(LlmProvider::ChatGptApi, "my-model").unwrap();
        settings.select_provider(LlmProvider::OtherApi).unwrap();
        let reopened = service(&directory);
        let snapshot = reopened.read();
        assert_eq!(snapshot.model, "local-model");
        assert!(snapshot.api.configured);
        assert_eq!(snapshot.other.base_url.as_deref(), Some("http://127.0.0.1:11434/v1"));
        assert!(!format!("{snapshot:?}").contains("api-test"));
        #[cfg(unix)]
        for file in ["chatgpt.key", "other.key", "other.service", "llm.json"] {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(directory.join(file)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{file}");
        }
        settings.remove_credential(LlmProvider::ChatGptApi).unwrap();
        assert!(settings.read().other.configured);
        assert!(settings
            .save_other_service(&OtherService {
                base_url: "https://key:secret@example.test/v1".to_string(),
                model: String::new(),
                transport: OtherTransport::OpenAiChatCompletions,
            })
            .is_err());
        assert!(directory.join("llm.json").exists());
        cleanup(&directory);
    }

    #[test]
    fn preserves_malformed_files_and_imports_pasted_keys() {
        let directory = scratch();
        std::fs::write(directory.join("chatgpt.key"), "{\"version\":1,\"secret\":").unwrap();
        let mut reopened = service(&directory);
        assert_eq!(
            reopened
                .read()
                .errors
                .iter()
                .map(|error| error.file.as_str())
                .collect::<Vec<_>>(),
            vec!["chatgpt.key"]
        );
        reopened.select_provider(LlmProvider::OtherApi).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.join("chatgpt.key")).unwrap(),
            "{\"version\":1,\"secret\":"
        );
        drop(reopened);
        std::fs::write(directory.join("chatgpt.key"), "pasted-key\n").unwrap();
        let mut settings = service(&directory);
        settings
            .save_api_key(ApiKeySlot::ChatGptApi, "replacement-key")
            .unwrap();
        assert!(std::fs::read_to_string(directory.join("chatgpt.key"))
            .unwrap()
            .contains("replacement-key"));
        cleanup(&directory);
    }

    #[test]
    fn api_request_posts_responses_schema_and_streams_text() {
        let directory = scratch();
        let fetch: Arc<dyn LlmFetch> = Arc::new(|url: &str, init: &FetchInit| {
            assert_eq!(url, "https://api.openai.com/v1/responses");
            assert_eq!(init.header("authorization"), Some("Bearer api-secret"));
            assert_eq!(init.header("chatgpt-account-id"), None);
            let sent = parse_json(init.body.as_deref().unwrap()).unwrap();
            assert_eq!(sent.get("model"), Some(&Json::String("test-model".to_string())));
            assert_eq!(sent.get("store"), Some(&Json::Bool(false)));
            Ok(sse("data: {\"type\":\"response.output_text.delta\",\"delta\":\"caf\u{e9}\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n"))
        });
        let mut settings = service_with_fetch(&directory, fetch);
        settings.save_api_key(ApiKeySlot::ChatGptApi, "api-secret").unwrap();
        settings.set_model(LlmProvider::ChatGptApi, "test-model").unwrap();
        settings.select_provider(LlmProvider::ChatGptApi).unwrap();
        let mut deltas = Vec::new();
        let answer = settings
            .request(
                LlmRequestInput {
                    prompt: "hello",
                    instructions: "answer",
                    on_text: Some(Box::new(|text: &str| deltas.push(text.to_string()))),
                },
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(answer, "caf\u{e9}");
        assert_eq!(deltas, vec!["caf\u{e9}".to_string()]);
        cleanup(&directory);
    }

    #[test]
    fn blank_model_and_absent_key_fail_before_fetching() {
        let directory = scratch();
        let calls = Arc::new(std::sync::Mutex::new(0));
        let probe = Arc::clone(&calls);
        let fetch: Arc<dyn LlmFetch> = Arc::new(move |_: &str, _: &FetchInit| {
            *probe.lock().unwrap() += 1;
            Ok(codex_answer())
        });
        let mut settings = service_with_fetch(&directory, fetch);
        settings.select_provider(LlmProvider::ChatGptApi).unwrap();
        let input = || LlmRequestInput {
            prompt: "hello",
            instructions: "answer",
            on_text: None,
        };
        assert!(settings
            .request(input(), &CancelToken::new())
            .unwrap_err()
            .to_string()
            .contains("Choose a Model"));
        settings.set_model(LlmProvider::ChatGptApi, "test-model").unwrap();
        assert!(settings
            .request(input(), &CancelToken::new())
            .unwrap_err()
            .to_string()
            .contains("Paste an API key"));
        assert_eq!(*calls.lock().unwrap(), 0);
        cleanup(&directory);
    }

    #[test]
    fn http_errors_are_sanitized_and_never_retried_for_api() {
        let directory = scratch();
        let calls = Arc::new(std::sync::Mutex::new(0));
        let probe = Arc::clone(&calls);
        let fetch: Arc<dyn LlmFetch> = Arc::new(move |_: &str, _: &FetchInit| {
            *probe.lock().unwrap() += 1;
            Ok(LlmResponse {
                status: 401,
                content_type: None,
                body: b"server-secret".to_vec(),
            })
        });
        let mut settings = service_with_fetch(&directory, fetch);
        settings.save_api_key(ApiKeySlot::ChatGptApi, "api-secret").unwrap();
        settings.set_model(LlmProvider::ChatGptApi, "test-model").unwrap();
        settings.select_provider(LlmProvider::ChatGptApi).unwrap();
        let error = settings
            .request(
                LlmRequestInput {
                    prompt: "hello",
                    instructions: "answer",
                    on_text: None,
                },
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("HTTP 401"), "{error}");
        assert!(!error.to_string().contains("secret"));
        assert_eq!(*calls.lock().unwrap(), 1);
        cleanup(&directory);
    }

    #[test]
    fn expiring_subscription_refreshes_once_and_preserves_api_key() {
        let directory = scratch();
        seed_subscription(&directory, now_ms() + 30_000);
        let refreshes = Arc::new(std::sync::Mutex::new(0));
        let probe = Arc::clone(&refreshes);
        let auth = SubscriptionAuthOptions {
            fetch: Some(Arc::new(move |_: &str, init: &FetchInit| {
                *probe.lock().unwrap() += 1;
                assert_eq!(init.method, FetchMethod::Post);
                Ok(refreshed_tokens())
            })),
            ..SubscriptionAuthOptions::default()
        };
        let fetch: Arc<dyn LlmFetch> = Arc::new(|_: &str, init: &FetchInit| {
            assert_eq!(
                init.header("authorization"),
                Some(format!("Bearer {}", subscription_token("new")).as_str())
            );
            Ok(codex_answer())
        });
        let mut settings = LlmSettingsService::open(&LlmSettingsOptions {
            base_directory: directory.clone(),
            auth,
            request: LlmRequestOptions {
                fetch: Some(fetch),
                timeout: None,
            },
        });
        settings
            .set_model(LlmProvider::ChatGptSubscription, "test-model")
            .unwrap();
        let answer = settings
            .request(
                LlmRequestInput {
                    prompt: "hello",
                    instructions: "answer",
                    on_text: None,
                },
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(answer, "answer");
        assert_eq!(*refreshes.lock().unwrap(), 1);
        let text = std::fs::read_to_string(directory.join("chatgpt.key")).unwrap();
        assert!(text.contains("new-refresh"));
        assert!(text.contains("preserved-api"));
        cleanup(&directory);
    }

    #[test]
    fn subscription_retries_auth_status_once_after_refresh() {
        for status in [401u16, 403] {
            let directory = scratch();
            seed_subscription(&directory, now_ms() + 3_600_000);
            let refreshes = Arc::new(std::sync::Mutex::new(0));
            let requests = Arc::new(std::sync::Mutex::new(0));
            let auth = SubscriptionAuthOptions {
                fetch: Some({
                    let refreshes = Arc::clone(&refreshes);
                    Arc::new(move |_: &str, _: &FetchInit| {
                        *refreshes.lock().unwrap() += 1;
                        Ok(refreshed_tokens())
                    })
                }),
                ..SubscriptionAuthOptions::default()
            };
            let sent = Arc::clone(&requests);
            let fetch: Arc<dyn LlmFetch> = Arc::new(move |_: &str, _: &FetchInit| {
                *sent.lock().unwrap() += 1;
                Ok(LlmResponse {
                    status,
                    content_type: None,
                    body: b"secret".to_vec(),
                })
            });
            let mut settings = LlmSettingsService::open(&LlmSettingsOptions {
                base_directory: directory.clone(),
                auth,
                request: LlmRequestOptions {
                    fetch: Some(fetch),
                    timeout: None,
                },
            });
            settings
                .set_model(LlmProvider::ChatGptSubscription, "test-model")
                .unwrap();
            let error = settings
                .request(
                    LlmRequestInput {
                        prompt: "hello",
                        instructions: "answer",
                        on_text: None,
                    },
                    &CancelToken::new(),
                )
                .unwrap_err();
            assert!(error.to_string().contains(&format!("HTTP {status}")), "{error}");
            assert_eq!(*refreshes.lock().unwrap(), 1);
            assert_eq!(*requests.lock().unwrap(), 2);
            cleanup(&directory);
        }
    }

    #[test]
    fn subscription_discovery_defaults_and_effort_lifecycle() {
        let directory = scratch();
        seed_subscription(&directory, now_ms() + 3_600_000);
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
        let probe = Arc::clone(&seen);
        let fetch: Arc<dyn LlmFetch> = Arc::new(move |url: &str, init: &FetchInit| {
            assert_eq!(init.header("chatgpt-account-id"), Some("test-account"));
            if init.method == FetchMethod::Get {
                assert_eq!(
                    url,
                    "https://chatgpt.com/backend-api/codex/models?client_version=0.154.0"
                );
                return Ok(LlmResponse {
                    status: 200,
                    content_type: Some("application/json".to_string()),
                    body: CODEX_LIST.as_bytes().to_vec(),
                });
            }
            probe.lock().unwrap().push(init.body.clone().unwrap_or_default());
            Ok(codex_answer())
        });
        let mut settings = service_with_fetch(&directory, fetch);
        settings.refresh_models(None, &CancelToken::new()).unwrap();
        assert_eq!(settings.read().model, "test-default");
        assert_eq!(settings.read().reasoning_effort.as_deref(), Some("low"));
        settings
            .set_model(LlmProvider::ChatGptSubscription, "test-reasoner")
            .unwrap();
        settings
            .set_reasoning_effort(LlmProvider::ChatGptSubscription, Some("xhigh"))
            .unwrap();
        settings
            .request(
                LlmRequestInput {
                    prompt: "hello",
                    instructions: "commands",
                    on_text: None,
                },
                &CancelToken::new(),
            )
            .unwrap();
        assert!(seen.lock().unwrap()[0].contains("\"effort\":\"xhigh\""));
        settings
            .set_model(LlmProvider::ChatGptSubscription, "missing-model")
            .unwrap_err();
        let reopened = service(&directory);
        assert_eq!(reopened.read().model, "test-reasoner");
        assert_eq!(reopened.read().reasoning_effort.as_deref(), Some("xhigh"));
        cleanup(&directory);
    }

    #[test]
    fn other_service_keeps_catalog_until_url_changes() {
        let directory = scratch();
        let list = "{\"object\":\"list\",\"data\":[{\"id\":\"gpt-4.1\"},{\"id\":\"local\"}]}";
        let fetch: Arc<dyn LlmFetch> = Arc::new(move |url: &str, _: &FetchInit| {
            assert!(url.ends_with("/models"), "{url}");
            Ok(LlmResponse {
                status: 200,
                content_type: Some("application/json".to_string()),
                body: list.as_bytes().to_vec(),
            })
        });
        let mut settings = service_with_fetch(&directory, fetch);
        let base = "http://127.0.0.1:11434/v1";
        settings
            .save_other_service(&OtherService {
                base_url: base.to_string(),
                model: String::new(),
                transport: OtherTransport::OpenAiChatCompletions,
            })
            .unwrap();
        settings.save_api_key(ApiKeySlot::OtherApi, "fake-key").unwrap();
        assert_eq!(
            settings
                .refresh_models(Some(LlmProvider::OtherApi), &CancelToken::new())
                .unwrap()
                .len(),
            2
        );
        settings
            .save_other_service(&OtherService {
                base_url: base.to_string(),
                model: "local".to_string(),
                transport: OtherTransport::OpenAiChatCompletions,
            })
            .unwrap();
        assert_eq!(settings.read().catalogs.other.status, CatalogStatus::Ready);
        settings
            .save_other_service(&OtherService {
                base_url: "http://127.0.0.1:11435/v1".to_string(),
                model: String::new(),
                transport: OtherTransport::OpenAiChatCompletions,
            })
            .unwrap();
        assert_eq!(settings.read().catalogs.other.status, CatalogStatus::Idle);
        assert!(settings.read().catalogs.other.models.is_empty());
        cleanup(&directory);
    }

    #[test]
    fn discovery_errors_keep_saved_models_and_cancelled_loads_stay_idle() {
        let directory = scratch();
        let fetch: Arc<dyn LlmFetch> = Arc::new(|_: &str, _: &FetchInit| {
            std::thread::sleep(Duration::from_millis(200));
            Ok(codex_answer())
        });
        let mut settings = LlmSettingsService::open(&LlmSettingsOptions {
            base_directory: directory.clone(),
            request: LlmRequestOptions {
                fetch: Some(fetch),
                timeout: Some(Duration::from_millis(20)),
            },
            ..LlmSettingsOptions::default()
        });
        settings.save_api_key(ApiKeySlot::ChatGptApi, "fake-key").unwrap();
        settings.set_model(LlmProvider::ChatGptApi, "saved-model").unwrap();
        let error = settings
            .refresh_models(Some(LlmProvider::ChatGptApi), &CancelToken::new())
            .unwrap_err();
        assert!(matches!(error, LlmError::Timeout), "{error}");
        assert_eq!(settings.read().catalogs.api.status, CatalogStatus::Error);
        assert_eq!(settings.read().api.model, "saved-model");
        let cancelled = CancelToken::new();
        cancelled.cancel();
        assert!(settings
            .refresh_models(Some(LlmProvider::ChatGptApi), &cancelled)
            .is_err());
        assert_eq!(settings.read().catalogs.api.status, CatalogStatus::Error);
        cleanup(&directory);
    }

    #[test]
    fn unsupported_persisted_effort_fails_before_paid_requests() {
        let directory = scratch();
        let calls = Arc::new(std::sync::Mutex::new(0));
        let probe = Arc::clone(&calls);
        let fetch: Arc<dyn LlmFetch> = Arc::new(move |_: &str, _: &FetchInit| {
            *probe.lock().unwrap() += 1;
            Ok(codex_answer())
        });
        let mut settings = service_with_fetch(&directory, fetch);
        settings.save_api_key(ApiKeySlot::ChatGptApi, "fixture-key").unwrap();
        settings.set_model(LlmProvider::ChatGptApi, "gpt-5.5-pro").unwrap();
        settings.select_provider(LlmProvider::ChatGptApi).unwrap();
        let text = std::fs::read_to_string(directory.join("llm.json")).unwrap();
        let patched = text.replace("\"chatgpt-api\":null", "\"chatgpt-api\":\"low\"");
        std::fs::write(directory.join("llm.json"), patched).unwrap();
        let error = settings
            .request(
                LlmRequestInput {
                    prompt: "hello",
                    instructions: "answer",
                    on_text: None,
                },
                &CancelToken::new(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("Model default"), "{error}");
        assert_eq!(*calls.lock().unwrap(), 0);
        cleanup(&directory);
    }

    fn settings_callback_url(authorize: &str) -> String {
        let redirect = authorize
            .split("redirect_uri=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .map(crate::llm::auth::form_decode)
            .expect("redirect");
        let state = authorize
            .split("state=")
            .nth(1)
            .and_then(|rest| rest.split('&').next())
            .expect("state")
            .to_string();
        format!("{redirect}?state={state}&code=test-code")
    }

    type BrowserHandles = Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<(u16, String)>>>>;

    fn settings_get(url: &str) -> (u16, String) {
        use std::io::{Read, Write};
        let without_scheme = url.strip_prefix("http://").expect("http");
        let (host, path) = match without_scheme.find('/') {
            Some(index) => (&without_scheme[..index], &without_scheme[index..]),
            None => (without_scheme, "/"),
        };
        let mut stream = std::net::TcpStream::connect(host.replace("localhost", "127.0.0.1")).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        write!(stream, "GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").expect("write");
        let mut body = Vec::new();
        stream.read_to_end(&mut body).expect("read");
        let text = String::from_utf8_lossy(&body).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .unwrap_or(0);
        let payload = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, payload)
    }

    #[test]
    fn sign_in_stores_subscription_without_erasing_api_key() {
        let directory = scratch();
        let browser: BrowserHandles = Arc::new(std::sync::Mutex::new(Vec::new()));
        let driver = Arc::clone(&browser);
        let auth = SubscriptionAuthOptions {
            callback_port: Some(0),
            fetch: Some(Arc::new(|_: &str, _: &FetchInit| {
                Ok(LlmResponse {
                    status: 200,
                    content_type: Some("application/json".to_string()),
                    body: "{\"access_token\":\"test-access\",\"refresh_token\":\"test-refresh\",\"token_type\":\"Bearer\",\"expires_in\":3600,\"scope\":\"openid profile\",\"id_token\":\"must-not-save\"}".as_bytes().to_vec(),
                })
            })),
            open_browser: Some(Arc::new(move |url: &str, _: &CancelToken| {
                let target = settings_callback_url(url);
                driver
                    .lock()
                    .unwrap()
                    .push(std::thread::spawn(move || settings_get(&target)));
                Ok(())
            })),
            ..SubscriptionAuthOptions::default()
        };
        let mut settings = LlmSettingsService::open(&LlmSettingsOptions {
            base_directory: directory.clone(),
            auth,
            ..LlmSettingsOptions::default()
        });
        settings.save_api_key(ApiKeySlot::ChatGptApi, "api-preserved").unwrap();
        settings.sign_in_subscription(&CancelToken::new()).unwrap();
        assert!(settings.read().subscription.configured);
        assert!(settings.read().api.configured);
        let text = std::fs::read_to_string(directory.join("chatgpt.key")).unwrap();
        assert!(text.contains("api-preserved"));
        assert!(!text.contains("must-not-save"));
        assert!(!format!("{:?}", settings.read()).contains("test-access"));
        settings.remove_credential(LlmProvider::ChatGptSubscription).unwrap();
        assert!(settings.read().api.configured);
        for handle in browser.lock().unwrap().drain(..) {
            assert_eq!(handle.join().unwrap().0, 200);
        }
        cleanup(&directory);
    }

    #[test]
    fn close_blocks_further_operations() {
        let directory = scratch();
        let mut settings = service(&directory);
        settings.close();
        assert!(settings.select_provider(LlmProvider::ChatGptApi).is_err());
        assert!(settings
            .request(
                LlmRequestInput {
                    prompt: "hello",
                    instructions: "answer",
                    on_text: None,
                },
                &CancelToken::new(),
            )
            .is_err());
        assert!(settings.read().errors.is_empty());
        cleanup(&directory);
    }
}
